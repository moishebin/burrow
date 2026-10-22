//! Picture in picture (D-036), as in Arc: leaving a tab whose video is playing with sound floats
//! the video in Chromium's own picture-in-picture window, and coming back to the tab puts it
//! back. "Back to tab" in that window shows the tab. The tab menu, Ctrl+Shift+P and agents can
//! also float a video by hand.
//!
//! Everything goes through the tab's DevTools: `Runtime.evaluate` with a user gesture (pages may
//! only enter picture in picture after one), and a `__burrowPip` binding through which the page
//! reports entering and leaving it.

use burrow_core::{Command, Id};
use serde_json::{json, Value};

use super::{tools::Done, Burrow};

pub const BINDING: &str = "__burrowPip";

/// Floats the page's best video and reports back. `auto` (leaving the tab) only takes a video
/// that is playing with sound, so muted background loops never pop out; by hand, any playing
/// video, else any loaded one. The largest wins.
fn start_js(auto: bool) -> String {
    format!(
        r#"(async (auto) => {{
  if (!window.__burrowPipHooked) {{
    window.__burrowPipHooked = true;
    const say = (m) => {{ try {{ {BINDING}(m); }} catch (e) {{}} }};
    document.addEventListener("enterpictureinpicture", () => say("enter"), true);
    // Closing the floating window pauses the video; "back to tab" leaves it playing. A live
    // stream (a call) can't pause, so there the two can't be told apart: just "closed".
    document.addEventListener("leavepictureinpicture", (e) => {{
      const v = e.target;
      setTimeout(() => say(v.paused || v.ended || v.srcObject ? "closed" : "back"), 100);
    }}, true);
  }}
  if (document.pictureInPictureElement) return "open";
  if (!document.pictureInPictureEnabled) return "unsupported";
  const all = [...document.querySelectorAll("video")].filter((v) => v.readyState >= 1 && v.videoWidth > 0 && !v.disablePictureInPicture);
  const playing = all.filter((v) => !v.paused && !v.ended);
  const pool = auto ? playing.filter((v) => !v.muted && v.volume > 0) : (playing.length ? playing : all);
  const area = (v) => v.clientWidth * v.clientHeight;
  const v = pool.sort((a, b) => area(b) - area(a))[0];
  if (!v) return "none";
  await v.requestPictureInPicture();
  return "open";
}})({auto})"#
    )
}

const STOP_JS: &str = "document.pictureInPictureElement ? document.exitPictureInPicture().then(() => 'closed') : 'closed'";

impl Burrow {
    /// The tabs in `left` stopped being shown: float a playing video, if Auto Picture in Picture
    /// is on. Never while a page is fullscreen (that hides the split partner, not the user).
    pub(super) fn auto_pip(&mut self, left: Vec<Id>) {
        if !self.state.settings.auto_pip || self.page_fullscreen.is_some() || self.closing {
            return;
        }
        for tab in left {
            if self.tabs.get(&tab).is_some_and(|t| t.loads > 0) {
                self.start_pip(&tab, true, Box::new(|_, _| {}));
            }
        }
    }

    /// Floats the tab's video. Answers "open", "none" (no video to float) or "unsupported".
    pub(super) fn start_pip(&mut self, tab: &str, auto: bool, done: Done) {
        let t = tab.to_string();
        self.devtools(
            tab,
            "Runtime.addBinding",
            json!({ "name": BINDING }),
            Box::new(move |b, r| {
                if let Err(e) = r {
                    return done(b, Err(e));
                }
                let tab = t.clone();
                b.eval(
                    &t,
                    start_js(auto),
                    Box::new(move |b, r| {
                        if r.as_ref().is_ok_and(|v| v == "open") {
                            b.pip_tab = Some(tab);
                            b.push_state();
                        }
                        done(b, r)
                    }),
                );
            }),
        );
    }

    /// Puts the video back into its page.
    pub(super) fn stop_pip(&mut self, tab: &str, done: Done) {
        if self.pip_tab.as_deref() == Some(tab) {
            self.pip_tab = None;
            self.push_state();
        }
        if !self.tabs.contains_key(tab) {
            return done(self, Ok(json!("closed")));
        }
        self.eval(tab, STOP_JS.to_string(), done);
    }

    /// The tab menu and Ctrl+Shift+P.
    pub(super) fn toggle_pip(&mut self, tab: &str) {
        if self.pip_tab.as_deref() == Some(tab) {
            return self.stop_pip(tab, Box::new(|_, _| {}));
        }
        self.start_pip(
            tab,
            false,
            Box::new(move |b, r| {
                let msg = match r.as_ref().map(|v| v.as_str().unwrap_or_default()) {
                    Ok("open") => return,
                    Ok("none") => "There's no video on this page to show in picture in picture.".to_string(),
                    Ok("unsupported") => "This page doesn't allow picture in picture.".to_string(),
                    Ok(other) => other.to_string(),
                    Err(e) => format!("Picture in picture didn't start: {e}"),
                };
                b.toast = Some(msg);
                b.push_state();
            }),
        );
    }

    /// The page entered or left picture in picture (from the `__burrowPip` binding).
    pub fn on_pip_event(&mut self, browser_id: i32, event: &str) {
        let Some(tab) = self.tab_of_browser(browser_id) else { return };
        match event {
            // Also when the page's own button started it. Pages can call the binding, so only
            // believe it if the page really is in picture in picture.
            "enter" => self.eval(
                &tab.clone(),
                "!!document.pictureInPictureElement".into(),
                Box::new(move |b, r| {
                    if r.is_ok_and(|v| v == Value::Bool(true)) {
                        b.pip_tab = Some(tab);
                        b.push_state();
                    }
                }),
            ),
            // "Back to tab": show the tab, in its space, with the window in front.
            "back" if self.pip_tab.as_deref() == Some(&*tab) => {
                self.pip_tab = None;
                if self.shown.as_deref() != Some(&*tab) {
                    self.command(Command::SelectTab { tab_id: tab });
                }
                self.bring_to_front();
                self.push_state();
            }
            "back" | "closed" if self.pip_tab.as_deref() == Some(&*tab) => {
                self.pip_tab = None;
                self.push_state();
            }
            _ => {}
        }
    }

    /// The `picture_in_picture` agent tool.
    pub(super) fn agent_pip(&mut self, args: &Value, done: Done) {
        if let Some(on) = args.get("auto").and_then(Value::as_bool) {
            self.state.settings.auto_pip = on;
            self.changed();
        }
        let action = args.get("action").and_then(Value::as_str).unwrap_or("status");
        let status = |b: &Burrow| json!({ "tab_id": b.pip_tab, "auto": b.state.settings.auto_pip });
        if action == "status" {
            return done(self, Ok(status(self)));
        }
        let Some(tab) = args.get("tab_id").and_then(Value::as_str).map(String::from).or_else(|| self.pip_tab.clone()) else {
            return done(self, Err("missing argument `tab_id`".into()));
        };
        if self.state.tab(&tab).is_none() {
            return done(self, Err(format!("no tab {tab}")));
        }
        if !self.tabs.contains_key(&tab) {
            return done(self, Err("the tab isn't loaded, so it has no video".into()));
        }
        match action {
            "open" => self.start_pip(
                &tab,
                false,
                Box::new(move |b, r| {
                    let r = r.and_then(|v| match v.as_str() {
                        Some("open") => Ok(status(b)),
                        Some("none") => Err("no video on that page".into()),
                        Some("unsupported") => Err("the page doesn't allow picture in picture".into()),
                        _ => Err(format!("unexpected answer {v}")),
                    });
                    done(b, r)
                }),
            ),
            "close" => self.stop_pip(&tab, Box::new(move |b, r| done(b, r.map(|_| status(b))))),
            a => done(self, Err(format!("unknown action `{a}` (use status, open or close)"))),
        }
    }
}
