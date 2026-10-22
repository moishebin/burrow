//! Agent tools that need the app (D-018): opening and loading tabs, and reading, screenshotting
//! and driving pages through each tab's own DevTools protocol, so they work on any tab, shown or
//! not, without a debugging port. Tools that only change the model run in `burrow_core::agent`.

use std::time::{Duration, Instant};

use burrow_core::{agent, links, omnibox, Command, Id};
use cef::*;
use serde_json::{json, Value};

use super::Burrow;
use crate::{agent::Reply, clients, task};

/// Called with a tool's (or one step's) answer.
pub type Done = Box<dyn FnOnce(&mut Burrow, Result<Value, String>) + Send>;
/// The next step of a tool, run on the UI thread.
type Step = Box<dyn FnOnce(&mut Burrow) + Send>;
type Check = Box<dyn FnMut(&mut Burrow) -> bool + Send>;
/// Called with whether a wait succeeded before its deadline.
type Settled = Box<dyn FnOnce(&mut Burrow, bool) + Send>;

const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const DEVTOOLS_TIMEOUT_MS: i64 = 30_000;
/// How long the sidebar shows "agent working" after a call.
const AGENT_BADGE_MS: i64 = 4_000;


fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn need<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    str_arg(args, key).ok_or_else(|| format!("missing argument `{key}`"))
}

fn flag(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(default)
}

/// A string as a JavaScript literal.
fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

impl Burrow {
    /// Runs one agent tool and answers `reply`, now or once the page has answered.
    pub fn agent_call(&mut self, tool: &str, args: Value, reply: Reply) {
        if self.closing {
            return reply.send(Err("Burrow is closing".into()));
        }
        if agent::is_page_tool(tool) && !self.state.settings.agent_page_access {
            return reply.send(Err(
                "The user turned off agent access to pages (+ menu in Burrow). Tab and space tools still work.".into(),
            ));
        }
        self.agent_seen();
        if let Some(result) = agent::run_model_tool(&mut self.state, tool, &args) {
            let result = result.map(|o| {
                for e in o.effects {
                    self.effect(e);
                }
                o.result
            });
            if !tool.starts_with("list_") && !tool.starts_with("get_") {
                self.sync_view();
                self.changed();
            }
            return reply.send(result);
        }
        let done: Done = Box::new(move |_, r| reply.send(r));
        if tool == "open_tab" {
            return self.agent_open_tab(&args, done);
        }
        if tool == "extensions" {
            return self.agent_extensions(&args, done);
        }
        if tool == "picture_in_picture" {
            return self.agent_pip(&args, done);
        }
        if tool == "update" {
            return self.agent_update(&args, done);
        }
        if let Some(result) = self.app_tool(tool, &args) {
            return done(self, result);
        }
        let tab = match need(&args, "tab_id") {
            Ok(t) if self.state.tab(t).is_some() => t.to_string(),
            Ok(t) => return done(self, Err(format!("no tab {t}"))),
            Err(e) => return done(self, Err(e)),
        };
        match tool {
            "navigate" => self.agent_navigate(tab, &args, done),
            "read_page" => {
                let max = args.get("max_chars").and_then(Value::as_u64).unwrap_or(20_000).max(100);
                let expr = match str_arg(&args, "format").unwrap_or("text") {
                    "text" => page_js("document.body ? document.body.innerText : ''", max),
                    "html" => page_js("document.documentElement.outerHTML", max),
                    "elements" => ELEMENTS_JS.to_string(),
                    f => return done(self, Err(format!("unknown format `{f}` (use text, html or elements)"))),
                };
                self.when_page_ready(tab.clone(), Box::new(move |b| b.eval(&tab, expr, done)));
            }
            "eval_js" => {
                let expr = match need(&args, "expression") {
                    Ok(e) => e.to_string(),
                    Err(e) => return done(self, Err(e)),
                };
                let done: Done = Box::new(move |b, r| done(b, r.map(|value| json!({ "value": value }))));
                self.when_page_ready(tab.clone(), Box::new(move |b| b.eval(&tab, expr, done)));
            }
            "screenshot" => {
                let full = flag(&args, "full_page", false);
                self.when_page_ready(tab.clone(), Box::new(move |b| b.screenshot(tab, full, done)));
            }
            "click" => {
                let target = match (str_arg(&args, "selector"), args["x"].as_f64(), args["y"].as_f64()) {
                    (Some(sel), _, _) => Err(sel.to_string()),
                    (None, Some(x), Some(y)) => Ok((x, y)),
                    _ => return done(self, Err("give a selector, or x and y".into())),
                };
                self.when_page_ready(tab.clone(), Box::new(move |b| b.click(tab, target, done)));
            }
            "type_text" => {
                let text = match need(&args, "text") {
                    Ok(t) => t.to_string(),
                    Err(e) => return done(self, Err(e)),
                };
                let selector = str_arg(&args, "selector").map(String::from);
                let (clear, submit) = (flag(&args, "clear", false), flag(&args, "submit", false));
                self.when_page_ready(tab.clone(), Box::new(move |b| b.type_text(tab, selector, text, clear, submit, done)));
            }
            "press_key" => {
                let events = match need(&args, "key").map(key_events) {
                    Ok(Some(e)) => e,
                    Ok(None) => return done(self, Err(format!("unknown key `{}`", args["key"]))),
                    Err(e) => return done(self, Err(e)),
                };
                let mut calls = vec![focus_emulation()];
                calls.extend(events.into_iter().map(|e| ("Input.dispatchKeyEvent".to_string(), e)));
                let done: Done = Box::new(|b, r| done(b, r.map(|_| json!({ "pressed": true }))));
                self.when_page_ready(tab.clone(), Box::new(move |b| b.devtools_seq(tab, calls, done)));
            }
            "wait_for" => {
                let check = match (str_arg(&args, "selector"), str_arg(&args, "text")) {
                    (Some(sel), _) => format!("!!document.querySelector({})", js(sel)),
                    (None, Some(text)) => format!("!!document.body && document.body.innerText.includes({})", js(text)),
                    _ => return done(self, Err("give a selector or text".into())),
                };
                let timeout = args.get("timeout_ms").and_then(Value::as_u64).unwrap_or(10_000);
                let deadline = Instant::now() + Duration::from_millis(timeout);
                self.when_page_ready(tab.clone(), Box::new(move |b| b.wait_for(tab, check, deadline, done)));
            }
            _ => done(self, Err(format!("unknown tool {tool}"))),
        }
    }

    /// Tools that need the app but no page: Boosts, history, the Peek, zoom and sound. `None`
    /// means `tool` isn't one of them.
    fn app_tool(&mut self, tool: &str, args: &Value) -> Option<Result<Value, String>> {
        let tab = || -> Result<String, String> {
            let t = need(args, "tab_id")?;
            if self.state.tab(t).is_none() {
                return Err(format!("no tab {t}"));
            }
            Ok(t.to_string())
        };
        Some(match tool {
            "boost" => (|| {
                let host = match (str_arg(args, "tab_id"), str_arg(args, "host")) {
                    (Some(t), _) => {
                        let url = self.state.tab(t).map(|t| t.url.clone()).ok_or_else(|| format!("no tab {t}"))?;
                        Some(links::host_of(&url)).filter(|h| !h.is_empty()).ok_or("that tab isn't on a website")?
                    }
                    (None, Some(h)) => links::host_of(&format!("https://{}", h.trim_start_matches("https://").trim_start_matches("http://"))),
                    (None, None) => String::new(),
                };
                match str_arg(args, "action").unwrap_or("list") {
                    "list" => Ok(json!({ "boosts": self.state.boosts })),
                    _ if host.is_empty() => Err("give host or tab_id".into()),
                    "get" => Ok(json!({ "host": host, "css": self.state.boosts.get(&host) })),
                    "set" => {
                        let css = need(args, "css")?.to_string();
                        let host = self.set_boost(&host, &css).ok_or("not a site")?;
                        Ok(json!({ "host": host, "set": true }))
                    }
                    "remove" => {
                        self.set_boost(&host, "");
                        Ok(json!({ "host": host, "removed": true }))
                    }
                    a => Err(format!("unknown action `{a}` (use list, get, set or remove)")),
                }
            })(),
            "search_history" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 500) as usize;
                let hits = self.history.search(str_arg(args, "query").unwrap_or(""), limit, super::now_ms());
                Ok(json!({ "pages": hits }))
            }
            "delete_history" => {
                if flag(args, "all", false) {
                    self.history = Default::default();
                } else {
                    match need(args, "url") {
                        Ok(u) => self.history.remove(u),
                        Err(_) => return Some(Err("give url, or all=true".into())),
                    }
                }
                self.schedule_history_save();
                Ok(json!({ "deleted": true }))
            }
            "peek" => match str_arg(args, "action").unwrap_or("status") {
                "status" => Ok(json!({ "open": self.peek.is_some(), "url": self.peek.as_ref().map(|p| &p.url) })),
                "open" => match (self.shown.clone(), need(args, "url")) {
                    (None, _) => Err("a Peek opens over the shown tab, and no tab is shown".into()),
                    (_, Err(e)) => Err(e),
                    (Some(shown), Ok(u)) => {
                        let url = omnibox::resolve(u, &self.state.settings.search_url);
                        self.open_peek(url.clone(), Some(shown));
                        Ok(json!({ "open": self.peek.is_some(), "url": url }))
                    }
                },
                "close" => {
                    self.close_peek();
                    Ok(json!({ "open": false }))
                }
                "expand" if self.peek.is_none() => Err("no Peek is open".into()),
                "expand" => {
                    self.expand_peek();
                    Ok(json!({ "tab_id": self.shown }))
                }
                a => Err(format!("unknown action `{a}` (use status, open, close or expand)")),
            },
            "set_zoom" => tab().and_then(|t| {
                let percent = args.get("percent").and_then(Value::as_f64).ok_or("missing argument `percent`")?;
                if !self.set_zoom_percent(&t, percent) {
                    return Err("the tab isn't loaded; focus it or navigate it first".into());
                }
                Ok(json!({ "zoom": percent.clamp(25.0, 500.0) }))
            }),
            "sidebar" => {
                if let Some(w) = args.get("width").and_then(Value::as_i64) {
                    self.set_sidebar_width(w);
                    self.changed();
                }
                if let Some(v) = args.get("visible").and_then(Value::as_bool)
                    && v != self.sidebar_visible
                {
                    self.toggle_sidebar();
                }
                Ok(json!({ "width": self.state.settings.sidebar_width, "visible": self.sidebar_visible }))
            }
            "window" => {
                let action = str_arg(args, "action").unwrap_or("status");
                let Some(w) = self.window.clone() else { return Some(Err("the window isn't open yet".into())) };
                match action {
                    "status" => {}
                    "minimize" => w.minimize(),
                    "maximize" => w.maximize(),
                    "restore" => w.restore(),
                    "fullscreen" | "exit_fullscreen" => {
                        let on = action == "fullscreen";
                        if on != self.fullscreen {
                            self.shortcut(super::Shortcut::Fullscreen);
                        }
                    }
                    "show_buttons" => {
                        if !self.reveal_window_buttons() {
                            return Some(Err("the window uses the system title bar, which has its own buttons".into()));
                        }
                    }
                    a => return Some(Err(format!("unknown action `{a}`"))),
                }
                Ok(json!({
                    "maximized": w.is_maximized() != 0,
                    "minimized": w.is_minimized() != 0,
                    "fullscreen": self.fullscreen,
                }))
            }
            "mute_tab" => tab().and_then(|t| {
                let muted = args.get("muted").and_then(Value::as_bool).ok_or("missing argument `muted`")?;
                if !self.set_muted(&t, muted) {
                    return Err("the tab isn't loaded, so it has no sound".into());
                }
                Ok(json!({ "muted": muted }))
            }),
            _ => return None,
        })
    }

    /// Shows "agent working" in the sidebar for a few seconds after each call.
    fn agent_seen(&mut self) {
        self.agent_generation += 1;
        let generation = self.agent_generation;
        if !self.agent_active {
            self.agent_active = true;
            self.push_state();
        }
        task::post_ui_delayed(AGENT_BADGE_MS, move || {
            super::with(|b| {
                if b.agent_generation == generation {
                    b.agent_active = false;
                    b.push_state();
                }
            });
        });
    }

    /// A tab as the agent sees it, with whether its page is running.
    fn tab_json(&self, tab_id: &str) -> Value {
        let info = agent::list_tabs(&self.state, None).into_iter().find(|t| t.id == tab_id);
        let mut v = serde_json::to_value(info).unwrap_or(Value::Null);
        if let Some(o) = v.as_object_mut() {
            let live = self.tabs.get(tab_id);
            o.insert("loaded".into(), json!(live.is_some()));
            o.insert("loading".into(), json!(live.is_some_and(|t| t.nav.loading)));
        }
        v
    }

    fn agent_open_tab(&mut self, args: &Value, done: Done) {
        let url = match need(args, "url") {
            Ok(u) => omnibox::resolve(u, &self.state.settings.search_url),
            Err(e) => return done(self, Err(e)),
        };
        let to = match agent::location(args, None) {
            Ok(to) => to,
            Err(e) => return done(self, Err(e)),
        };
        match self.state.open_tab(url, to, flag(args, "focus", false)) {
            Err(e) => done(self, Err(e.to_string())),
            Ok((id, effects)) => {
                for e in effects {
                    self.effect(e);
                }
                // Background tabs load right away, so the agent can read them.
                self.ensure_live(&id);
                self.sync_view();
                self.changed();
                if flag(args, "wait", true) {
                    self.wait_load(id, 0, done);
                } else {
                    let info = self.tab_json(&id);
                    done(self, Ok(info));
                }
            }
        }
    }

    fn agent_navigate(&mut self, tab: Id, args: &Value, done: Done) {
        let loads = self.tabs.get(&tab).map_or(0, |t| t.loads);
        let browser = self.tabs.get(&tab).and_then(|t| t.view.browser());
        let action = str_arg(args, "action");
        match (str_arg(args, "url"), action) {
            (Some(input), _) => {
                let url = omnibox::resolve(input, &self.state.settings.search_url);
                if let Some(t) = self.state.tab_mut(&tab) {
                    t.url = url.clone();
                }
                match browser.as_ref().and_then(|b| b.main_frame()) {
                    Some(frame) => frame.load_url(Some(&super::cef(&url))),
                    None => self.ensure_live(&tab),
                }
            }
            (None, Some("home")) => {
                if let Ok(effects) = self.state.apply(Command::ResetTab { tab_id: tab.clone() }) {
                    for e in effects {
                        self.effect(e);
                    }
                }
                self.ensure_live(&tab);
            }
            (None, Some(a @ ("back" | "forward" | "reload" | "stop"))) => match &browser {
                Some(b) => match a {
                    "back" => b.go_back(),
                    "forward" => b.go_forward(),
                    "reload" => b.reload(),
                    _ => b.stop_load(),
                },
                None if a == "reload" => self.ensure_live(&tab),
                None => return done(self, Err("the tab isn't loaded".into())),
            },
            (None, Some(a)) => return done(self, Err(format!("unknown action `{a}`"))),
            (None, None) => return done(self, Err("give a url or an action".into())),
        }
        self.changed();
        if flag(args, "wait", true) && action != Some("stop") {
            self.wait_load(tab, loads, done);
        } else {
            let info = self.tab_json(&tab);
            done(self, Ok(info));
        }
    }

    /// Answers with the tab once more than `after` page loads have finished (or on timeout).
    fn wait_load(&mut self, tab: Id, after: u32, done: Done) {
        let t = tab.clone();
        let check = Box::new(move |b: &mut Burrow| b.tabs.get(&t).is_some_and(|l| l.loads > after && !l.nav.loading));
        self.poll(
            Instant::now() + LOAD_TIMEOUT,
            check,
            Box::new(move |b, loaded| {
                let mut info = b.tab_json(&tab);
                if !loaded {
                    info["note"] = json!("still loading after 30 s");
                }
                done(b, Ok(info))
            }),
        );
    }

    /// Loads the tab if needed and waits for its first page, then runs `then`.
    fn when_page_ready(&mut self, tab: Id, then: Step) {
        self.ensure_live(&tab);
        let t = tab.clone();
        let check = Box::new(move |b: &mut Burrow| b.tabs.get(&t).is_some_and(|l| l.loads > 0 && l.view.browser().is_some()));
        self.poll(Instant::now() + LOAD_TIMEOUT, check, Box::new(move |b, _| b.emulate_viewport(tab, then)));
    }

    /// A hidden tab's view has no size and doesn't paint. Giving its page the content area's size
    /// through DevTools makes it lay out and render (screenshots, coordinates) as if shown, while
    /// the user keeps seeing their own tab.
    fn emulate_viewport(&mut self, tab: Id, then: Step) {
        if self.visible.contains(&tab) || !self.tabs.contains_key(&tab) {
            return then(self);
        }
        let size = View::from(&self.content).bounds();
        let (width, height) = if size.width > 0 && size.height > 0 { (size.width, size.height) } else { (1280, 800) };
        if let Some(t) = self.tabs.get_mut(&tab) {
            t.emulated = true;
        }
        let params = json!({ "width": width, "height": height, "deviceScaleFactor": 0, "mobile": false });
        let t = tab.clone();
        self.devtools(&t, "Emulation.setDeviceMetricsOverride", params, Box::new(move |b, _| then(b)));
    }

    /// The tab is shown again: its real view size applies.
    pub(super) fn end_emulation(&mut self, tab: &str) {
        if self.tabs.get_mut(tab).is_some_and(|t| std::mem::take(&mut t.emulated)) {
            self.devtools(tab, "Emulation.clearDeviceMetricsOverride", json!({}), Box::new(|_, _| {}));
        }
    }

    /// Calls `done(true)` as soon as `check` passes, or `done(false)` at the deadline.
    fn poll(
        &mut self,
        deadline: Instant,
        mut check: Check,
        done: Settled,
    ) {
        if check(self) {
            return done(self, true);
        }
        if Instant::now() >= deadline {
            return done(self, false);
        }
        task::post_ui_delayed(100, move || {
            super::with(move |b| b.poll(deadline, check, done));
        });
    }

    // ---- DevTools ---------------------------------------------------------------------------------

    /// Sends one DevTools protocol call to a tab's page.
    pub(super) fn devtools(&mut self, tab: &str, method: &str, params: Value, done: Done) {
        let Some(live) = self.tabs.get_mut(tab) else {
            return done(self, Err("the tab isn't loaded".into()));
        };
        let Some(host) = live.view.browser().and_then(|b| b.host()) else {
            return done(self, Err("the tab's page hasn't started".into()));
        };
        if live.devtools.is_none() {
            live.devtools = host.add_dev_tools_message_observer(Some(&mut clients::AgentDevTools::new()));
        }
        self.send_devtools(&host, method, params, done);
    }

    /// Sends one DevTools call to any browser that has an `AgentDevTools` observer.
    pub(super) fn send_devtools(&mut self, host: &BrowserHost, method: &str, params: Value, done: Done) {
        let id = self.next_devtools_id;
        self.next_devtools_id += 1;
        let message = json!({ "id": id, "method": method, "params": params }).to_string();
        if host.send_dev_tools_message(Some(message.as_bytes())) == 0 {
            return done(self, Err(format!("{method} could not be sent")));
        }
        self.devtools_pending.insert(id, done);
        let method = method.to_string();
        task::post_ui_delayed(DEVTOOLS_TIMEOUT_MS, move || {
            super::with(|b| {
                if let Some(done) = b.devtools_pending.remove(&id) {
                    done(b, Err(format!("{method} timed out")));
                }
            });
        });
    }

    pub fn on_devtools_result(&mut self, id: i32, success: bool, json: &str) {
        let Some(done) = self.devtools_pending.remove(&id) else { return };
        let value: Value = serde_json::from_str(json).unwrap_or(Value::Null);
        if success {
            done(self, Ok(value))
        } else {
            done(self, Err(value["message"].as_str().unwrap_or(json).to_string()))
        }
    }

    /// Runs DevTools calls one after another; answers with the last result.
    fn devtools_seq(&mut self, tab: Id, mut calls: Vec<(String, Value)>, done: Done) {
        if calls.is_empty() {
            return done(self, Ok(Value::Null));
        }
        let (method, params) = calls.remove(0);
        let t = tab.clone();
        self.devtools(
            &t,
            &method,
            params,
            Box::new(move |b, r| match r {
                Ok(v) if calls.is_empty() => done(b, Ok(v)),
                Ok(_) => b.devtools_seq(tab, calls, done),
                Err(e) => done(b, Err(e)),
            }),
        );
    }

    /// Evaluates JavaScript in the page; answers with its (JSON) value or the thrown error.
    pub(super) fn eval(&mut self, tab: &str, expression: String, done: Done) {
        let params = json!({ "expression": expression, "returnByValue": true, "awaitPromise": true, "userGesture": true });
        self.devtools(
            tab,
            "Runtime.evaluate",
            params,
            Box::new(move |b, r| {
                let r = r.and_then(|v| match v.get("exceptionDetails") {
                    // The description includes a stack trace; its first line is the message.
                    Some(ex) => Err(ex["exception"]["description"].as_str().or(ex["text"].as_str()).unwrap_or("script error")
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_string()),
                    None => Ok(v["result"]["value"].clone()),
                });
                done(b, r)
            }),
        );
    }

    fn screenshot(&mut self, tab: Id, full_page: bool, done: Done) {
        let image: Done = Box::new(|b, r| done(b, r.map(|v| json!({ "type": "image", "mimeType": "image/png", "data": v["data"] }))));
        if !full_page {
            return self.devtools(&tab, "Page.captureScreenshot", json!({ "format": "png" }), image);
        }
        let t = tab.clone();
        self.devtools(
            &t,
            "Page.getLayoutMetrics",
            json!({}),
            Box::new(move |b, r| {
                let size = match r {
                    Ok(v) => v["cssContentSize"].clone(),
                    Err(e) => return image(b, Err(e)),
                };
                let clip = json!({
                    "x": 0, "y": 0, "scale": 1,
                    "width": size["width"].as_f64().unwrap_or(1280.0),
                    "height": size["height"].as_f64().unwrap_or(800.0).min(16_384.0),
                });
                let params = json!({ "format": "png", "captureBeyondViewport": true, "clip": clip });
                b.devtools(&tab, "Page.captureScreenshot", params, image);
            }),
        );
    }

    /// Clicks at a point, or at the middle of the element `target` selects (scrolled into view).
    fn click(&mut self, tab: Id, target: Result<(f64, f64), String>, done: Done) {
        let (x, y) = match target {
            Ok(point) => point,
            Err(selector) => {
                let expr = format!(
                    "(() => {{ const el = document.querySelector({}); if (!el) throw new Error('no element matches the selector'); \
                     el.scrollIntoView({{ block: 'center', inline: 'center' }}); const r = el.getBoundingClientRect(); \
                     return [r.left + r.width / 2, r.top + r.height / 2]; }})()",
                    js(&selector)
                );
                let t = tab.clone();
                return self.eval(
                    &t,
                    expr,
                    Box::new(move |b, r| match r {
                        Ok(v) => {
                            let point = (v[0].as_f64().unwrap_or(0.0), v[1].as_f64().unwrap_or(0.0));
                            b.click(tab, Ok(point), done)
                        }
                        Err(e) => done(b, Err(e)),
                    }),
                );
            }
        };
        let mouse = |kind: &str| {
            let mut e = json!({ "type": kind, "x": x, "y": y });
            if kind != "mouseMoved" {
                e["button"] = json!("left");
                e["clickCount"] = json!(1);
                e["buttons"] = json!(if kind == "mousePressed" { 1 } else { 0 });
            }
            ("Input.dispatchMouseEvent".to_string(), e)
        };
        let calls = vec![focus_emulation(), mouse("mouseMoved"), mouse("mousePressed"), mouse("mouseReleased")];
        self.devtools_seq(tab, calls, Box::new(move |b, r| done(b, r.map(|_| json!({ "clicked": true, "x": x, "y": y })))));
    }

    fn type_text(&mut self, tab: Id, selector: Option<String>, text: String, clear: bool, submit: bool, done: Done) {
        let mut calls = vec![focus_emulation()];
        if let Some(sel) = &selector {
            let expr = format!(
                "(() => {{ const el = document.querySelector({}); if (!el) throw new Error('no element matches the selector'); \
                 el.scrollIntoView({{ block: 'center' }}); el.focus(); \
                 if ({clear}) {{ if ('value' in el) {{ el.value = ''; el.dispatchEvent(new Event('input', {{ bubbles: true }})); }} \
                 else if (el.isContentEditable) document.execCommand('selectAll'); }} return true; }})()",
                js(sel)
            );
            calls.push(("Runtime.evaluate".into(), json!({ "expression": expr, "returnByValue": true, "userGesture": true })));
        }
        calls.push(("Input.insertText".into(), json!({ "text": text })));
        if submit {
            calls.extend(key_events("Enter").unwrap_or_default().into_iter().map(|e| ("Input.dispatchKeyEvent".to_string(), e)));
        }
        self.devtools_seq(
            tab,
            calls,
            Box::new(move |b, r| done(b, r.map(|_| json!({ "typed": true, "submitted": submit })))),
        );
    }

    fn wait_for(&mut self, tab: Id, check: String, deadline: Instant, done: Done) {
        let t = tab.clone();
        self.eval(
            &t,
            check.clone(),
            Box::new(move |b, r| {
                if r.as_ref().is_ok_and(|v| v == &json!(true)) {
                    return done(b, Ok(json!({ "found": true })));
                }
                if Instant::now() >= deadline {
                    return done(b, Ok(json!({ "found": false })));
                }
                // Errors here usually mean the page is navigating; try again.
                task::post_ui_delayed(250, move || {
                    super::with(move |b| {
                        if b.tabs.contains_key(&tab) {
                            b.wait_for(tab, check, deadline, done)
                        } else {
                            done(b, Err("the tab was closed".into()))
                        }
                    });
                });
            }),
        );
    }
}

/// Pages in background tabs think they have focus, so focus-dependent input works.
fn focus_emulation() -> (String, Value) {
    ("Emulation.setFocusEmulationEnabled".into(), json!({ "enabled": true }))
}

/// JavaScript returning the page's title, URL and `content`, cut to `max` characters.
fn page_js(content: &str, max: u64) -> String {
    format!(
        "(() => {{ const c = {content}; return {{ title: document.title, url: location.href, length: c.length, \
         truncated: c.length > {max}, content: c.slice(0, {max}) }}; }})()"
    )
}

/// DevTools key events for a key name (Enter, ArrowDown, …) or a single character.
fn key_events(name: &str) -> Option<Vec<Value>> {
    let named: &[(&str, &str, i32, &str)] = &[
        ("Enter", "Enter", 13, "\r"),
        ("Tab", "Tab", 9, ""),
        ("Escape", "Escape", 27, ""),
        ("Backspace", "Backspace", 8, ""),
        ("Delete", "Delete", 46, ""),
        ("ArrowUp", "ArrowUp", 38, ""),
        ("ArrowDown", "ArrowDown", 40, ""),
        ("ArrowLeft", "ArrowLeft", 37, ""),
        ("ArrowRight", "ArrowRight", 39, ""),
        ("Home", "Home", 36, ""),
        ("End", "End", 35, ""),
        ("PageUp", "PageUp", 33, ""),
        ("PageDown", "PageDown", 34, ""),
        ("Space", "Space", 32, " "),
    ];
    let (key, code, vk, text) = match named.iter().find(|(n, ..)| n.eq_ignore_ascii_case(name)) {
        Some(&(n, code, vk, text)) => (if n == "Space" { " " } else { n }.to_string(), code.to_string(), vk, text.to_string()),
        None => {
            let mut chars = name.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else { return None };
            let upper = c.to_ascii_uppercase();
            let (code, vk) = match c {
                'a'..='z' | 'A'..='Z' => (format!("Key{upper}"), upper as i32),
                '0'..='9' => (format!("Digit{c}"), c as i32),
                _ => (String::new(), 0),
            };
            (c.to_string(), code, vk, c.to_string())
        }
    };
    let down = if text.is_empty() {
        json!({ "type": "rawKeyDown", "key": key, "code": code, "windowsVirtualKeyCode": vk })
    } else {
        json!({ "type": "keyDown", "key": key, "code": code, "windowsVirtualKeyCode": vk, "text": text, "unmodifiedText": text })
    };
    Some(vec![down, json!({ "type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk })])
}

/// Lists the page's visible links, buttons and fields, each with a CSS selector and its center.
/// Never includes what's typed in password fields.
const ELEMENTS_JS: &str = r#"(() => {
  const unique = (s) => { try { return document.querySelectorAll(s).length === 1; } catch { return false; } };
  const selectorOf = (el) => {
    const parts = [];
    for (let e = el; e && e.nodeType === 1 && e !== document.documentElement; e = e.parentElement) {
      if (e.id && unique('#' + CSS.escape(e.id))) { parts.unshift('#' + CSS.escape(e.id)); break; }
      let i = 1;
      for (let s = e.previousElementSibling; s; s = s.previousElementSibling) if (s.tagName === e.tagName) i++;
      parts.unshift(e.tagName.toLowerCase() + ':nth-of-type(' + i + ')');
    }
    return parts.join(' > ');
  };
  const query = 'a[href], button, input:not([type=hidden]), select, textarea, summary, [role=button], [role=link], ' +
    '[role=checkbox], [role=tab], [role=menuitem], [role=option], [contenteditable=""], [contenteditable=true], [onclick]';
  const out = [];
  for (const el of document.querySelectorAll(query)) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) continue;
    const st = getComputedStyle(el);
    if (st.visibility === 'hidden' || st.display === 'none') continue;
    const field = el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT';
    const value = field && el.type !== 'password' ? el.value : '';
    const label = (field ? '' : el.innerText) || el.getAttribute('aria-label') || el.getAttribute('title') ||
      el.getAttribute('placeholder') || el.getAttribute('alt') || (el.labels && el.labels[0] && el.labels[0].innerText) || '';
    const o = {
      i: out.length, tag: el.tagName.toLowerCase(), text: label.trim().replace(/\s+/g, ' ').slice(0, 100),
      selector: selectorOf(el), x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2),
      in_view: r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth,
    };
    if (value) o.value = String(value).slice(0, 100);
    if (el.type && field) o.type = el.type;
    if (el.name) o.name = el.name;
    if (el.href) o.href = el.href;
    if (el.getAttribute('role')) o.role = el.getAttribute('role');
    if (el.disabled) o.disabled = true;
    if (el.checked) o.checked = true;
    out.push(o);
    if (out.length >= 300) break;
  }
  return { title: document.title, url: location.href, viewport: { width: innerWidth, height: innerHeight }, elements: out };
})()"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        let enter = key_events("enter").unwrap();
        assert_eq!(enter[0]["type"], "keyDown");
        assert_eq!(enter[0]["text"], "\r");
        assert_eq!(key_events("Tab").unwrap()[0]["type"], "rawKeyDown");
        assert_eq!(key_events("a").unwrap()[0]["code"], "KeyA");
        assert_eq!(key_events("a").unwrap()[0]["windowsVirtualKeyCode"], 65);
        assert!(key_events("Hyper").is_none());
    }

    #[test]
    fn js_strings_are_quoted() {
        assert_eq!(js("a'b\"c</script>"), r#""a'b\"c</script>""#);
    }
}
