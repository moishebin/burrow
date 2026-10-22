//! Sleeping tabs (D-040): a tab out of sight longer than its sleep time has its page unloaded to
//! free memory, and loads again when shown. Before that, the page and each of its frames are
//! asked whether they are in use (sound or video playing, picture in picture, typed text); if so
//! the tab stays awake for another round. Tabs using the camera or microphone never sleep.

use serde_json::json;

use super::{now_ms, task, with, Burrow};

const TICK_MS: i64 = 60_000;

/// "pip", "media", "typing", or "" when the frame can sleep. Runs in the page and in each frame
/// from another site (frames from the same site are searched from their parent).
const PROBE: &str = r#"(() => {
  if (document.pictureInPictureElement || window.documentPictureInPicture?.window) return "pip";
  const docs = [document];
  for (const f of document.querySelectorAll("iframe")) {
    try { if (f.contentDocument) docs.push(f.contentDocument); } catch { /* another site: probed on its own */ }
  }
  const text = /^(text|search|email|url|tel|number)$/;
  for (const d of docs) {
    for (const m of d.querySelectorAll("video, audio")) {
      if (!m.paused && !m.ended && !m.muted && m.volume > 0) return "media";
    }
    for (const e of d.querySelectorAll("textarea, input")) {
      if ((e.tagName === "TEXTAREA" || text.test(e.type)) && e.value && e.value !== e.defaultValue) return "typing";
    }
    // Rich editors have no default to compare with: the cursor left in one that holds text.
    const a = d.activeElement;
    if (a && a.isContentEditable && a.innerText.trim()) return "typing";
  }
  return "";
})()"#;

/// Called with whether the page or one of its frames is in use.
type Answer = Box<dyn FnOnce(&mut Burrow, bool) + Send>;

impl Burrow {
    /// Checks for tabs due to sleep now and every minute.
    pub(super) fn sleep_tick(&mut self) {
        let now = now_ms();
        // Tabs on screen, in a call, or waiting for an answer to a permission prompt are in use.
        let in_use: Vec<String> =
            self.visible.iter().cloned().chain(self.permissions.iter().map(|p| p.tab_id.clone())).collect();
        for (id, t) in self.tabs.iter_mut() {
            if t.capturing || in_use.contains(id) {
                t.seen_ms = now;
            }
        }
        let due = self.state.tabs_to_sleep(self.tabs.iter().map(|(id, t)| (id.as_str(), t.seen_ms)), now);
        for id in due {
            self.try_sleep(id);
        }
        task::post_ui_delayed(TICK_MS, || {
            with(|b| b.sleep_tick());
        });
    }

    /// A page started or stopped using the camera or microphone.
    pub fn on_media_access(&mut self, browser_id: i32, on: bool) {
        let Some(id) = self.tab_of_browser(browser_id) else { return };
        if let Some(t) = self.tabs.get_mut(&id) {
            t.capturing = on;
            t.seen_ms = now_ms();
        }
    }

    /// A frame from another site got (or lost) its own DevTools session.
    pub fn on_frame_session(&mut self, browser_id: i32, session: String, attached: bool) {
        let Some(id) = self.tab_of_browser(browser_id) else { return };
        let Some(frames) = self.tabs.get_mut(&id).and_then(|t| t.frames.as_mut()) else { return };
        frames.retain(|s| *s != session);
        if attached {
            frames.push(session);
        }
    }

    fn try_sleep(&mut self, id: String) {
        let Some(seen) = self.tabs.get(&id).map(|t| t.seen_ms) else { return };
        let tab = id.clone();
        let answer: Answer = Box::new(move |b, busy| {
            // Shown (or put back in use) while the page was answering: stay awake.
            if b.visible.contains(&id) || b.tabs.get(&id).is_none_or(|t| t.seen_ms != seen) {
                return;
            }
            if !busy {
                b.unload(&id);
                b.push_state();
            } else if let Some(t) = b.tabs.get_mut(&id) {
                t.seen_ms = now_ms();
            }
        });
        let t = tab.clone();
        self.watch_frames(&t, Box::new(move |b| b.probe(tab, None, answer)));
    }

    /// Makes DevTools attach to the page's frames from other sites, once per page; each reports
    /// through `on_frame_session` before this call's answer arrives.
    fn watch_frames(&mut self, tab: &str, then: Box<dyn FnOnce(&mut Burrow) + Send>) {
        let Some(live) = self.tabs.get_mut(tab) else { return };
        if live.frames.is_some() {
            return then(self);
        }
        live.frames = Some(vec![]);
        let params = json!({ "autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true });
        self.devtools(tab, "Target.setAutoAttach", params, Box::new(move |b, _| then(b)));
    }

    /// Asks the page (`frame` = None) or one frame session, then the next frame, until one is
    /// busy or all have answered.
    fn probe(&mut self, tab: String, frame: Option<usize>, answer: Answer) {
        let session = match frame {
            None => None,
            Some(i) => match self.tabs.get(&tab).and_then(|t| t.frames.as_ref()).and_then(|f| f.get(i)) {
                Some(s) => Some(s.clone()),
                None => return answer(self, false), // every frame answered: not in use
            },
        };
        let next = frame.map_or(0, |i| i + 1);
        let params = json!({ "expression": PROBE, "returnByValue": true });
        let t = tab.clone();
        self.devtools_in(
            &t,
            session.as_deref(),
            "Runtime.evaluate",
            params,
            Box::new(move |b, result| {
                // A page or frame that doesn't answer isn't counted as in use.
                let busy = result.ok().is_some_and(|v| v["result"]["value"].as_str().is_some_and(|s| !s.is_empty()));
                if busy {
                    answer(b, true)
                } else {
                    b.probe(tab, Some(next), answer)
                }
            }),
        );
    }
}
