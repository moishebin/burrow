//! Chrome extensions (D-032): installing from the Chrome Web Store or from disk, loading them
//! at startup, and showing an extension's popup in a small card over the page.
//!
//! Chromium loads extensions only at startup (`--load-extension`), so installing, removing,
//! turning on or off needs a restart, which Burrow offers. Extensions run in every space's
//! profile, each with its own storage, so an extension's logins are per space like the sites'.
//!
//! Alloy-style tabs (D-010) aren't Chrome tabs: `chrome.tabs.query` finds none and
//! `chrome.tabs.create` fails. A popup gets a small script first that answers "the active tab"
//! with the tab the user is on and opens new tabs through Burrow.

use std::sync::{
    atomic::{AtomicBool, AtomicI32, Ordering},
    Arc, Mutex, OnceLock,
};

use burrow_core::{
    extensions::{self as ext, Extension, InstallFrom, Source},
    store, State,
};
use cef::*;
use serde::Serialize;
use serde_json::{json, Value};

use super::{cef, later, tools::Done, Burrow};
use crate::{agent::Reply, clients, paths, task, views};

/// The popup's browser id, `i32::MAX` while it isn't known yet, or -1 with no popup. Read by
/// CEF callbacks that must answer at once (keys, console messages).
static POPUP_BROWSER: AtomicI32 = AtomicI32::new(-1);
/// The extensions Chromium was told to load at startup, as (id, version).
static LOADED: OnceLock<Vec<(String, String)>> = OnceLock::new();
static RESTART: AtomicBool = AtomicBool::new(false);
/// The helper extension's secret for this run (`ext::helper_files`).
static HELPER_SECRET: OnceLock<String> = OnceLock::new();

/// A popup's page logs this, then JSON, to ask Burrow for something (`chrome.tabs.create`).
pub const POPUP_PREFIX: &str = "__burrow_ext__";
/// Chrome's limits for a popup's size.
const POPUP_MIN: Size = Size { width: 25, height: 25 };
const POPUP_MAX: Size = Size { width: 800, height: 600 };
const POPUP_START: Size = Size { width: 360, height: 480 };
const POPUP_MARGIN: i32 = 8;
/// A `.crx` larger than this is refused.
const MAX_PACKAGE_BYTES: usize = 200 * 1024 * 1024;

pub fn is_popup_browser(browser_id: i32) -> bool {
    browser_id >= 0 && POPUP_BROWSER.load(Ordering::Relaxed) == browser_id
}

/// Burrow should start again after it has shut down (to load changed extensions).
pub fn restart_requested() -> bool {
    RESTART.load(Ordering::Relaxed)
}

/// `--load-extension`'s value: the folders of the enabled extensions. Called once, in the
/// browser process, before Chromium starts. Also deletes folders nothing uses any more.
pub fn load_extension_switch() -> Option<String> {
    let state = store::load(&paths::state_file()).unwrap_or_default();
    let root = paths::extensions_dir();
    ext::clean_up(&root, &state.extensions);
    let enabled: Vec<&Extension> = state.extensions.iter().filter(|e| e.enabled && root.join(&e.dir).is_dir()).collect();
    let _ = LOADED.set(enabled.iter().map(|e| (e.id.clone(), e.version.clone())).collect());
    let mut dirs: Vec<String> = enabled.iter().map(|e| root.join(&e.dir).to_string_lossy().into_owned()).collect();
    if dirs.is_empty() {
        return None;
    }
    // Burrow's script in each one's background (D-041).
    let hook = crate::ext_bridge::hook();
    for e in &enabled {
        if let Err(err) = ext::add_bridge(&root.join(&e.dir), &hook) {
            eprintln!("Burrow: could not add the bridge to {}: {err}", e.name);
        }
    }
    // Burrow's helper, so popups can find the tab the user is on. New secret every run.
    let secret = burrow_core::model::new_id().replace('-', "");
    let helper = paths::data_dir().join("extension-helper");
    let written = std::fs::create_dir_all(&helper)
        .and_then(|_| ext::helper_files(&secret).into_iter().try_for_each(|(name, body)| std::fs::write(helper.join(name), body)));
    match written {
        Ok(()) => {
            let _ = HELPER_SECRET.set(secret);
            dirs.push(helper.to_string_lossy().into_owned());
        }
        Err(e) => eprintln!("Burrow: could not write the extension helper: {e}"),
    }
    Some(dirs.join(","))
}

/// A tab's id for the extension APIs, from a console message of the helper extension.
pub fn helper_tab_id(message: &str) -> Option<i32> {
    ext::helper_tab_id(message, HELPER_SECRET.get()?)
}

fn loaded() -> &'static [(String, String)] {
    LOADED.get().map_or(&[], |v| v.as_slice())
}

/// Whether Chromium is running this extension now (it was on, at this version, at startup).
fn is_running(e: &Extension) -> bool {
    loaded().iter().any(|(id, v)| *id == e.id && *v == e.version)
}

/// What changed since startup takes a restart to apply.
fn restart_needed(state: &State) -> bool {
    let mut now = state.enabled_extensions();
    let mut then = loaded().to_vec();
    now.sort();
    then.sort();
    now != then
}

pub(super) struct Popup {
    overlay: OverlayController,
    view: BrowserView,
    browser_id: Option<i32>,
    devtools: Option<Registration>,
    ext_id: String,
    url: String,
    /// The tab the popup acts on (`chrome.tabs.query({active: true})`), as a CEF browser id.
    tab: Option<i32>,
    size: Size,
}

/// An extension's badge, as its background set it (D-041): for every tab, and per tab by its
/// extension tab id.
#[derive(Default)]
pub(super) struct Badge {
    text: String,
    color: Option<String>,
    tabs: std::collections::HashMap<i32, String>,
    tab_colors: std::collections::HashMap<i32, String>,
}

#[derive(Serialize, Clone)]
pub(super) struct BadgeView {
    text: String,
    color: Option<String>,
}

/// What the sidebar shows about extensions.
#[derive(Serialize)]
pub(super) struct ExtensionsView<'a> {
    /// Badges for the shown tab, by extension id.
    badges: std::collections::HashMap<&'a str, BadgeView>,
    /// Ids of the extensions running now.
    running: Vec<&'a str>,
    restart_needed: bool,
    /// What is being installed, by name or link.
    installing: Vec<&'a str>,
    /// The extension whose popup is open.
    popup: Option<&'a str>,
}

pub(super) struct Install {
    label: String,
    done: Option<Done>,
}

impl Burrow {
    pub(super) fn extensions_view(&self) -> ExtensionsView<'_> {
        ExtensionsView {
            badges: self
                .ext_badges
                .keys()
                .filter(|id| self.state.extension(id).is_some_and(|e| e.enabled && is_running(e)))
                .filter_map(|id| Some((id.as_str(), self.badge_for(id)?)))
                .collect(),
            running: self.state.extensions.iter().filter(|e| is_running(e)).map(|e| e.id.as_str()).collect(),
            restart_needed: restart_needed(&self.state),
            installing: self.ext_installs.values().map(|i| i.label.as_str()).collect(),
            popup: self.ext_popup.as_ref().map(|p| p.ext_id.as_str()),
        }
    }

    /// An extension's badge for the tab the user is on; None when it shows nothing.
    pub(super) fn badge_for(&self, id: &str) -> Option<BadgeView> {
        let b = self.ext_badges.get(id)?;
        let tab = self.shown.as_ref().and_then(|t| self.tabs.get(t)).and_then(|t| t.ext_tab);
        let text = tab.and_then(|t| b.tabs.get(&t)).unwrap_or(&b.text).clone();
        let color = tab.and_then(|t| b.tab_colors.get(&t)).or(b.color.as_ref()).cloned();
        (!text.is_empty()).then_some(BadgeView { text, color })
    }

    // ---- The bridge (D-041) --------------------------------------------------------------------

    /// A request from Burrow's script in an extension's background (`ext_bridge`).
    pub fn extension_bridge(&mut self, ext_id: &str, msg: &Value, reply: Reply) {
        if self.state.extension(ext_id).is_none() {
            return reply.send(Err("unknown extension".into()));
        }
        let tab = msg["tab"].as_i64().map(|t| t as i32);
        let result = match msg["what"].as_str().unwrap_or_default() {
            "badge" => {
                let b = self.ext_badges.entry(ext_id.to_string()).or_default();
                if let Some(text) = msg.get("text") {
                    // Chrome's badges hold a few characters; anything else is cut.
                    let text: String = text.as_str().unwrap_or_default().chars().take(6).collect();
                    match tab {
                        Some(t) => b.tabs.insert(t, text),
                        None => Some(std::mem::replace(&mut b.text, text)),
                    };
                }
                if let Some(color) = msg["color"].as_str().filter(|c| c.len() < 40 && !c.contains([';', '{', '}'])) {
                    match tab {
                        Some(t) => b.tab_colors.insert(t, color.to_string()),
                        None => b.color.replace(color.to_string()),
                    };
                }
                self.push_state();
                Ok(Value::Null)
            }
            "open" => {
                let url = msg["url"].as_str().unwrap_or_default();
                // Relative addresses are pages of the extension itself; an empty one is a new tab.
                let url = if url.is_empty() {
                    "about:blank".to_string()
                } else if url.contains("://") || url.starts_with("about:") || url.starts_with("data:") {
                    url.to_string()
                } else {
                    format!("chrome-extension://{ext_id}/{}", url.trim_start_matches('/'))
                };
                if url.starts_with("javascript:") || url.starts_with("file:") {
                    Err("not allowed".to_string())
                } else {
                    let space = self.state.active_space.clone();
                    self.open_in_space(url, &space, msg["active"].as_bool() != Some(false));
                    Ok(Value::Null)
                }
            }
            "tabs" => return self.answer_bridge_tabs(reply, 0),
            w => Err(format!("unknown request `{w}`")),
        };
        reply.send(result);
    }

    /// The tab the user is on and the space's tabs, by extension tab id. Right after a start the
    /// pages haven't told their ids yet (the helper's content script does), so wait a little.
    fn answer_bridge_tabs(&mut self, reply: Reply, tries: u32) {
        let shown = self.shown.as_ref().and_then(|t| self.tabs.get(t));
        let active = shown.and_then(|t| t.ext_tab);
        if active.is_none() && shown.is_some() && tries < 30 {
            return task::post_ui_delayed(100, move || {
                super::with(|b| b.answer_bridge_tabs(reply, tries + 1));
            });
        }
        reply.send(Ok(json!({ "active": active, "all": self.profile_tab_ids(active) })));
    }

    pub fn set_extension_pinned(&mut self, id: &str, pinned: bool) -> bool {
        if !self.state.set_extension_pinned(id, pinned) {
            return false;
        }
        self.changed();
        true
    }

    // ---- Installing ---------------------------------------------------------------------------

    /// Downloads an extension from the Chrome Web Store (a store link or an id) and installs it.
    pub fn install_from_web_store(&mut self, input: &str, done: Option<Done>) {
        let Some(id) = ext::web_store_id(input) else {
            return self.install_failed("That isn't a Chrome Web Store link or extension id.".into(), done);
        };
        let label = self.state.extension(&id).map_or_else(|| "Chrome Web Store extension".to_string(), |e| e.name.clone());
        let key = self.next_install;
        self.next_install += 1;
        self.ext_installs.insert(key, Install { label, done });
        let started = (|| {
            let mut request = request_create()?;
            request.set_url(Some(&cef(&ext::web_store_download_url(&id))));
            request.set_method(Some(&cef("GET")));
            let mut client = CrxDownload::new(key, Arc::new(Mutex::new(Vec::new())));
            urlrequest_create(Some(&mut request), Some(&mut client), None)
        })();
        if started.is_none() {
            self.on_installed(key, Err("The download couldn't start.".into()));
        }
        self.push_state();
    }

    fn on_package_downloaded(&mut self, key: u64, result: Result<Vec<u8>, String>) {
        match result {
            Ok(bytes) => {
                std::thread::spawn(move || {
                    let r = ext::install(InstallFrom::Package(&bytes), &paths::extensions_dir(), Source::WebStore);
                    later(move |b| b.on_installed(key, r.map_err(|e| e.to_string())));
                });
            }
            Err(e) => self.on_installed(key, Err(e)),
        }
    }

    /// Installs an unpacked extension's folder, or a `.crx`/`.zip` file.
    pub fn install_from_path(&mut self, path: String, done: Option<Done>) {
        let key = self.next_install;
        self.next_install += 1;
        let label = std::path::Path::new(&path).file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned());
        self.ext_installs.insert(key, Install { label, done });
        self.push_state();
        std::thread::spawn(move || {
            let p = std::path::Path::new(&path);
            let root = paths::extensions_dir();
            let r = if p.is_dir() {
                ext::install(InstallFrom::Folder(p), &root, Source::File).map_err(|e| e.to_string())
            } else {
                std::fs::read(p)
                    .map_err(|e| format!("{path}: {e}"))
                    .and_then(|bytes| ext::install(InstallFrom::Package(&bytes), &root, Source::File).map_err(|e| e.to_string()))
            };
            later(move |b| b.on_installed(key, r));
        });
    }

    fn on_installed(&mut self, key: u64, result: Result<Extension, String>) {
        let done = self.ext_installs.remove(&key).and_then(|i| i.done);
        match result {
            Ok(e) => {
                let updated = self.state.extension(&e.id).map(|old| old.version.clone());
                let msg = match &updated {
                    Some(v) if *v == e.version => format!("{} is up to date.", e.name),
                    Some(_) => format!("Updated {} to {}. Restart Burrow to use it.", e.name, e.version),
                    None => format!("Installed {}. Restart Burrow to start it.", e.name),
                };
                let mut info = json!({ "installed": e.id, "name": e.name, "version": e.version });
                self.state.add_extension(e);
                info["restart_needed"] = json!(restart_needed(&self.state));
                self.toast = Some(msg);
                self.changed();
                if let Some(done) = done {
                    done(self, Ok(info));
                }
            }
            Err(e) => self.install_failed(format!("Couldn't install the extension: {e}"), done),
        }
    }

    fn install_failed(&mut self, msg: String, done: Option<Done>) {
        self.toast = Some(msg.clone());
        self.push_state();
        if let Some(done) = done {
            done(self, Err(msg));
        }
    }

    /// Asks the user for an extension folder, or a `.crx`/`.zip` file.
    pub fn choose_extension_file(&mut self, folder: bool) {
        let Some(host) = self.sidebar.browser().and_then(|b| b.host()) else { return };
        let mut filters = CefStringList::new();
        if !folder {
            filters.append(".crx");
            filters.append(".zip");
        }
        let (mode, title) = if folder {
            (FileDialogMode::OPEN_FOLDER, "Choose an unpacked extension's folder")
        } else {
            (FileDialogMode::OPEN, "Choose an extension file (.crx or .zip)")
        };
        let mut callback = ChosenFile::new();
        host.run_file_dialog(mode, Some(&cef(title)), None, Some(&mut filters), Some(&mut callback));
    }

    /// Turns an extension on or off (after a restart). False if there is no such extension.
    pub fn set_extension_enabled(&mut self, id: &str, enabled: bool) -> bool {
        if !self.state.set_extension_enabled(id, enabled) {
            return false;
        }
        if !enabled && self.ext_popup.as_ref().is_some_and(|p| p.ext_id == id) {
            self.close_extension_popup();
        }
        self.changed();
        true
    }

    /// Removes an extension; its files go at the next start, when Chromium no longer has them.
    pub fn remove_extension(&mut self, id: &str) -> Option<Extension> {
        if self.ext_popup.as_ref().is_some_and(|p| p.ext_id == id) {
            self.close_extension_popup();
        }
        let e = self.state.remove_extension(id)?;
        self.ext_badges.remove(id);
        self.toast = Some(format!("Removed {}.", e.name));
        self.changed();
        Some(e)
    }

    /// Updates every Web Store extension (or just `only`) to the store's newest version.
    pub fn update_extensions(&mut self, only: Option<&str>) -> usize {
        let ids: Vec<String> = self
            .state
            .extensions
            .iter()
            .filter(|e| e.source == Source::WebStore && only.is_none_or(|o| o == e.id))
            .map(|e| e.id.clone())
            .collect();
        for id in &ids {
            self.install_from_web_store(id, None);
        }
        ids.len()
    }

    /// Closes the window and starts Burrow again once it has shut down (`main`).
    pub fn restart(&mut self) {
        RESTART.store(true, Ordering::Relaxed);
        task::post_ui(|| {
            if let Some(w) = super::with(|b| b.window.clone()).flatten() {
                w.close();
            }
        });
    }

    // ---- Popup ----------------------------------------------------------------------------------

    /// Shows an extension's popup over the page, in the active space's profile. With no popup,
    /// opens its options page as a tab.
    pub fn open_extension_popup(&mut self, id: &str) -> Result<(), String> {
        let Some(e) = self.state.extension(id).cloned() else { return Err(format!("no extension {id}")) };
        if !e.enabled {
            return Err(format!("{} is turned off.", e.name));
        }
        if !is_running(&e) {
            return Err(format!("Restart Burrow to start {}.", e.name));
        }
        let page = match (&e.popup, &e.options) {
            (Some(p), _) => p.clone(),
            (None, Some(o)) => {
                let url = e.url(o);
                let space = self.state.active_space.clone();
                self.open_in_space(url, &space, true);
                return Ok(());
            }
            (None, None) => return Err(format!("{} has no window of its own; it works on the pages you visit.", e.name)),
        };
        self.close_extension_popup();
        self.close_peek();
        let profile_id = self.state.active_space().profile_id.clone();
        let Some(profile) = self.profile(&profile_id) else { return Err("the space's profile isn't available".into()) };
        if !profile.ready {
            return Err("The space is still starting; try again in a moment.".into());
        }
        let mut context = Some(profile.context.clone());
        let tab = self.shown.as_ref().and_then(|t| self.tabs.get(t)).and_then(|t| t.ext_tab);
        let mut client = clients::ContentClient::new();
        let mut delegate = views::TabViewDelegate::new(false);
        // Blank first: the script that fixes `chrome.tabs` must be in place before the page runs.
        let view = browser_view_create(
            Some(&mut client),
            Some(&cef("about:blank")),
            Some(&BrowserSettings { background_color: 0xFFFFFFFF, ..Default::default() }),
            None,
            context.as_mut(),
            Some(&mut delegate),
        )
        .ok_or("the popup couldn't be created")?;
        let window = self.window.clone().ok_or("no window")?;
        let frame = panel_create(None).ok_or("no panel")?;
        frame.set_to_fill_layout();
        frame.set_insets(Some(&Insets { top: 1, left: 1, bottom: 1, right: 1 }));
        frame.add_child_view(Some(&mut View::from(&view)));
        let overlay = window.add_overlay_view(Some(&mut View::from(&frame)), DockingMode::CUSTOM, 1).ok_or("no overlay")?;
        // CEF resets the colors of views it puts in an overlay (D-022).
        frame.set_background_color(0xFFB9B4CC);
        POPUP_BROWSER.store(i32::MAX, Ordering::Relaxed);
        self.ext_popup = Some(Popup {
            overlay,
            view,
            browser_id: None,
            devtools: None,
            ext_id: e.id.clone(),
            url: e.url(&page),
            tab,
            size: POPUP_START,
        });
        self.layout_extension_popup();
        if let Some(p) = &self.ext_popup {
            p.overlay.set_visible(1);
        }
        self.popup_wait_for_browser(0);
        self.push_state();
        Ok(())
    }

    /// The popup's browser exists a moment after its view: then set it up and load the page.
    fn popup_wait_for_browser(&mut self, tries: u32) {
        let Some(p) = &mut self.ext_popup else { return };
        let Some(browser) = p.view.browser() else {
            if tries < 100 {
                task::post_ui_delayed(20, move || {
                    super::with(|b| b.popup_wait_for_browser(tries + 1));
                });
            }
            return;
        };
        let Some(host) = browser.host() else { return };
        let id = browser.identifier();
        p.browser_id = Some(id);
        POPUP_BROWSER.store(id, Ordering::Relaxed);
        host.set_auto_resize_enabled(1, Some(&POPUP_MIN), Some(&POPUP_MAX));
        p.devtools = host.add_dev_tools_message_observer(Some(&mut clients::AgentDevTools::new()));
        let tab = p.tab;
        let shim = popup_shim(tab, &self.profile_tab_ids(tab));
        let load: Done = Box::new(move |b, r| {
            if let Err(e) = r {
                eprintln!("Burrow: extension popup script: {e}");
            }
            let Some(p) = b.ext_popup.as_ref().filter(|p| p.browser_id == Some(id)) else { return };
            if let Some(frame) = p.view.browser().and_then(|br| br.main_frame()) {
                frame.load_url(Some(&cef(&p.url)));
            }
        });
        // New-document scripts only run once the Page domain is on.
        self.send_devtools(
            &host.clone(),
            "Page.enable",
            json!({}),
            Box::new(move |b, _| b.send_devtools(&host, "Page.addScriptToEvaluateOnNewDocument", json!({ "source": shim }), load)),
        );
    }

    /// The live tabs of the active space's profile (what `chrome.tabs.query({})` should list),
    /// by their extension tab ids, `tab` first.
    fn profile_tab_ids(&self, tab: Option<i32>) -> Vec<i32> {
        let profile = self.state.active_space().profile_id.clone();
        let mut ids: Vec<i32> = self
            .tabs
            .iter()
            .filter(|(id, _)| self.state.profile_of_tab(id) == Some(&profile))
            .filter_map(|(_, t)| t.ext_tab)
            .collect();
        ids.sort_by_key(|i| Some(*i) != tab);
        ids
    }

    /// Places the popup at the top left of the content area, at the size its page wants.
    pub(super) fn layout_extension_popup(&self) {
        let Some(p) = &self.ext_popup else { return };
        let area = View::from(&self.content).bounds();
        let width = (p.size.width + 2).min(area.width - 2 * POPUP_MARGIN).max(1);
        let height = (p.size.height + 2).min(area.height - 2 * POPUP_MARGIN).max(1);
        p.overlay.set_bounds(Some(&Rect { x: area.x + POPUP_MARGIN, y: area.y + POPUP_MARGIN, width, height }));
    }

    pub fn on_extension_popup_resized(&mut self, browser_id: i32, size: Size) {
        let Some(p) = self.ext_popup.as_mut().filter(|p| p.browser_id == Some(browser_id)) else { return };
        p.size = size;
        self.layout_extension_popup();
    }

    pub fn close_extension_popup(&mut self) {
        let Some(p) = self.ext_popup.take() else { return };
        POPUP_BROWSER.store(-1, Ordering::Relaxed);
        if let Some(host) = p.view.browser().and_then(|b| b.host()) {
            // The window waits for it to finish closing, like a Peek's.
            self.dying_peeks.extend(p.browser_id);
            host.close_browser(1);
        }
        drop(p.devtools);
        p.overlay.destroy();
        self.focus_content();
        self.push_state();
    }

    /// The popup's page closed itself (`window.close()`), or its browser went away.
    pub(super) fn on_extension_popup_closed(&mut self) {
        self.close_extension_popup();
    }

    /// The helper extension told us a tab's id for the extension APIs.
    pub fn on_extension_tab_id(&mut self, browser_id: i32, ext_tab: i32) {
        let Some(id) = self.tab_of_browser(browser_id) else { return };
        if let Some(t) = self.tabs.get_mut(&id) {
            t.ext_tab = Some(ext_tab);
        }
    }

    pub(super) fn is_extension_popup(&self, browser_id: i32) -> bool {
        self.ext_popup.as_ref().is_some_and(|p| p.browser_id == Some(browser_id))
    }

    /// A request from the popup's page (see `popup_shim`).
    pub fn on_extension_popup_message(&mut self, browser_id: i32, json: &str) {
        let Some(p) = self.ext_popup.as_ref().filter(|p| p.browser_id == Some(browser_id)) else { return };
        let ext_id = p.ext_id.clone();
        let Ok(msg) = serde_json::from_str::<Value>(json) else { return };
        let space = self.state.active_space.clone();
        let url = if msg["options"].as_bool() == Some(true) {
            self.state.extension(&ext_id).and_then(|e| e.options.as_ref().map(|o| e.url(o)))
        } else {
            msg["open"].as_str().filter(|u| !u.is_empty()).map(|u| {
                // Relative addresses are pages of the extension itself.
                if u.contains("://") || u.starts_with("about:") { u.to_string() } else { format!("chrome-extension://{ext_id}/{}", u.trim_start_matches('/')) }
            })
        };
        let Some(url) = url else { return };
        let foreground = msg["active"].as_bool() != Some(false);
        if foreground {
            self.close_extension_popup();
        }
        self.open_in_space(url, &space, foreground);
    }

    // ---- Agents ---------------------------------------------------------------------------------

    /// The `extensions` agent tool: list, install, update, enable, disable, remove, open, restart.
    pub(super) fn agent_extensions(&mut self, args: &Value, done: Done) {
        let arg = |k: &str| args.get(k).and_then(Value::as_str);
        let list = |b: &Burrow| {
            let items: Vec<Value> = b
                .state
                .extensions
                .iter()
                .map(|e| {
                    json!({
                        "id": e.id, "name": e.name, "version": e.version, "description": e.description,
                        "enabled": e.enabled, "running": is_running(e), "source": e.source,
                        "has_popup": e.popup.is_some(), "options_url": e.options.as_ref().map(|o| e.url(o)),
                        "pinned": e.pinned, "badge": b.badge_for(&e.id).map(|v| v.text),
                    })
                })
                .collect();
            json!({ "extensions": items, "restart_needed": restart_needed(&b.state) })
        };
        let which = |b: &Burrow| -> Result<Extension, String> {
            let q = arg("extension").ok_or("missing argument `extension` (an id or a name)")?;
            b.state.find_extension(q).cloned().ok_or_else(|| format!("no extension `{q}`"))
        };
        let result = match arg("action").unwrap_or("list") {
            "list" => Ok(list(self)),
            "install" => {
                return match (arg("web_store"), arg("path")) {
                    (Some(s), _) => self.install_from_web_store(s, Some(done)),
                    (None, Some(p)) => self.install_from_path(p.to_string(), Some(done)),
                    _ => done(self, Err("give `web_store` (a Chrome Web Store link or id) or `path` (a folder, .crx or .zip)".into())),
                };
            }
            "update" => {
                let only = match arg("extension") {
                    Some(_) => match which(self) {
                        Ok(e) => Some(e.id),
                        Err(e) => return done(self, Err(e)),
                    },
                    None => None,
                };
                let n = self.update_extensions(only.as_deref());
                Ok(json!({ "checking": n, "note": "Updates download in the background; list again to see new versions." }))
            }
            a @ ("enable" | "disable") => which(self).map(|e| {
                self.set_extension_enabled(&e.id, a == "enable");
                json!({ "id": e.id, "enabled": a == "enable", "restart_needed": restart_needed(&self.state) })
            }),
            a @ ("pin" | "unpin") => which(self).map(|e| {
                self.set_extension_pinned(&e.id, a == "pin");
                json!({ "id": e.id, "pinned": a == "pin" })
            }),
            "remove" => which(self).map(|e| {
                self.remove_extension(&e.id);
                json!({ "removed": e.id, "restart_needed": restart_needed(&self.state) })
            }),
            "open" => which(self).and_then(|e| {
                self.open_extension_popup(&e.id)?;
                Ok(json!({ "open": e.id, "popup": self.ext_popup.is_some() }))
            }),
            "close" => {
                self.close_extension_popup();
                Ok(json!({ "open": false }))
            }
            "restart" => {
                // Answer first: the server stops with the app.
                done(self, Ok(json!({ "restarting": true })));
                return self.restart();
            }
            a => Err(format!("unknown action `{a}` (use list, install, update, enable, disable, pin, unpin, remove, open, close or restart)")),
        };
        done(self, result);
    }
}

/// The script a popup's page runs before its own. `tab` is the tab the user is on, `all` the
/// open tabs of the space's profile, by their extension tab ids.
fn popup_shim(tab: Option<i32>, all: &[i32]) -> String {
    let tab = tab.map_or("null".to_string(), |t| t.to_string());
    let all = serde_json::to_string(all).unwrap_or_else(|_| "[]".into());
    format!(
        r#"(() => {{
  if (!globalThis.chrome || !chrome.tabs || location.protocol !== "chrome-extension:") return;
  const ACTIVE = {tab}, ALL = {all};
  const send = (m) => console.log({prefix} + JSON.stringify(m));
  const tabs = chrome.tabs;
  const query = tabs.query.bind(tabs);
  const filters = ["url", "title", "status", "audible", "muted", "pinned", "index", "groupId", "discarded", "windowType"];
  const answer = (p, cb) => {{ if (typeof cb === "function") {{ p.then(cb); return; }} return p; }};
  tabs.query = (q, cb) => answer(query(q || {{}}).then(async (found) => {{
    q = q || {{}};
    if (found.length) return found;
    const wantsActive = q.active === true || q.highlighted === true || q.currentWindow === true || q.lastFocusedWindow === true;
    const ids = wantsActive && q.active !== false ? (ACTIVE === null ? [] : [ACTIVE])
      : filters.some((k) => q[k] !== undefined) ? [] : ALL;
    const list = (await Promise.all(ids.map((id) => tabs.get(id).catch(() => null)))).filter(Boolean);
    return list.map((t) => Object.assign(t, {{ active: t.id === ACTIVE, highlighted: t.id === ACTIVE, selected: t.id === ACTIVE }}));
  }}), cb);
  // A popup isn't a tab (CEF gives every browser a tab id; Chrome's popups have none).
  tabs.getCurrent = (cb) => answer(Promise.resolve(undefined), cb);
  tabs.create = (props, cb) => {{
    props = props || {{}};
    send({{ open: props.url || "", active: props.active !== false }});
    return answer(Promise.resolve({{ id: -1, url: props.url || "", active: props.active !== false }}), cb);
  }};
  if (chrome.runtime && chrome.runtime.openOptionsPage) {{
    chrome.runtime.openOptionsPage = (cb) => (send({{ options: true }}), answer(Promise.resolve(), cb));
  }}
}})();"#,
        prefix = serde_json::to_string(POPUP_PREFIX).unwrap_or_default(),
    )
}

wrap_urlrequest_client! {
    struct CrxDownload {
        key: u64,
        body: Arc<Mutex<Vec<u8>>>,
    }
    impl UrlrequestClient {
        fn on_download_data(&self, _request: Option<&mut Urlrequest>, data: *const u8, data_length: usize) {
            if data.is_null() {
                return;
            }
            if let Ok(mut body) = self.body.lock()
                && body.len() <= MAX_PACKAGE_BYTES
            {
                // Safe: CEF hands us `data_length` readable bytes for the duration of the call.
                body.extend_from_slice(unsafe { std::slice::from_raw_parts(data, data_length) });
            }
        }

        fn on_request_complete(&self, request: Option<&mut Urlrequest>) {
            let key = self.key;
            let status = request.as_ref().map(|r| r.request_status());
            let http = request.as_ref().and_then(|r| r.response()).map_or(0, |r| r.status());
            let body = self.body.lock().map(|mut b| std::mem::take(&mut *b)).unwrap_or_default();
            let result = if status != Some(UrlrequestStatus::SUCCESS) {
                Err("the Chrome Web Store couldn't be reached".to_string())
            } else if http == 204 || http == 404 || body.is_empty() {
                Err("the Chrome Web Store has no such extension for this browser".to_string())
            } else if http != 200 {
                Err(format!("the Chrome Web Store answered {http}"))
            } else if body.len() > MAX_PACKAGE_BYTES {
                Err("the package is too large".to_string())
            } else {
                Ok(body)
            };
            later(move |b| b.on_package_downloaded(key, result));
        }
    }
}

wrap_run_file_dialog_callback! {
    struct ChosenFile {}
    impl RunFileDialogCallback {
        fn on_file_dialog_dismissed(&self, file_paths: Option<&mut CefStringList>) {
            let Some(path) = file_paths.and_then(|l| l.clone().into_iter().next()) else { return };
            later(move |b| b.install_from_path(path, None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shim_names_the_tabs() {
        let js = popup_shim(Some(7), &[7, 9]);
        assert!(js.contains("const ACTIVE = 7, ALL = [7,9];"));
        assert!(js.contains("\"__burrow_ext__\""));
        assert!(popup_shim(None, &[]).contains("const ACTIVE = null, ALL = [];"));
    }
}
