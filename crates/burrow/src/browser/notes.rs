//! Web notifications (D-042): pages hand them to Burrow (`burrow_core::notify`), Burrow shows
//! them as system notifications, and a click brings Burrow forward on the tab that sent it.

use std::{collections::VecDeque, sync::OnceLock};

use burrow_core::{links, notify::Note, Command, Id};
use cef::*;
use serde::Serialize;
use serde_json::{json, Value};

use super::{cef, js_string, later, now_ms, tools::Done, Burrow};

static SECRET: OnceLock<String> = OnceLock::new();
const KEEP: usize = 50;

fn secret() -> &'static str {
    SECRET.get_or_init(|| burrow_core::model::new_id().replace('-', ""))
}

/// The script every page runs as it starts loading.
pub fn page_script() -> &'static str {
    static SCRIPT: OnceLock<String> = OnceLock::new();
    SCRIPT.get_or_init(|| burrow_core::notify::page_script(secret()))
}

/// A notification in a page's console message.
pub fn parse_note(message: &str) -> Option<Note> {
    burrow_core::notify::parse(message, secret())
}

/// One shown notification, for agents.
#[derive(Clone, Serialize)]
pub(super) struct Shown {
    #[serde(skip)]
    id: String,
    tab_id: Id,
    title: String,
    body: String,
    site: String,
    at_ms: u64,
}

#[derive(Default)]
pub(super) struct Notes {
    recent: VecDeque<Shown>,
}

impl Burrow {
    pub fn on_web_notification(&mut self, browser_id: i32, note: Note) {
        if !self.state.settings.notifications {
            return;
        }
        let Some(tab) = self.tab_of_browser(browser_id) else { return };
        let site = links::host_of(&note.origin);
        self.notes.recent.push_front(Shown { id: note.id.clone(), tab_id: tab.clone(), title: note.title.clone(), body: note.body.clone(), site: site.clone(), at_ms: now_ms() });
        self.notes.recent.truncate(KEEP);
        let id = note.id.clone();
        crate::notify::show(
            &note.title,
            &note.body,
            &site,
            Box::new(move || {
                let (tab, id) = (tab.clone(), id.clone());
                later(move |b| b.on_notification_clicked(&tab, &id));
            }),
        );
    }

    /// The user clicked a notification: show its tab, in front, and tell the page.
    pub(super) fn on_notification_clicked(&mut self, tab: &str, id: &str) {
        if self.state.tab(tab).is_none() {
            return;
        }
        if self.shown.as_deref() != Some(tab) {
            self.command(Command::SelectTab { tab_id: tab.to_string() });
        }
        self.bring_to_front();
        if id.is_empty() {
            return;
        }
        let Some(browser) = self.tabs.get(tab).and_then(|t| t.view.browser()) else { return };
        let js = format!("window.__burrowNote && window.__burrowNote.click({})", js_string(id));
        // It may have come from a frame; ids are unique, so asking every frame is safe.
        let mut ids = CefStringList::new();
        browser.frame_identifiers(Some(&mut ids));
        for fid in ids {
            if let Some(frame) = browser.frame_by_identifier(Some(&cef(&fid))) {
                frame.execute_java_script(Some(&cef(&js)), Some(&CefString::from(&frame.url())), 0);
            }
        }
    }

    /// The `notifications` agent tool.
    pub(super) fn agent_notifications(&mut self, args: &Value, done: Done) {
        match args.get("action").and_then(Value::as_str).unwrap_or("list") {
            "list" => {}
            // What clicking it does.
            "open" => {
                let i = args.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let Some(n) = self.notes.recent.get(i).cloned() else { return done(self, Err(format!("no notification {i}"))) };
                self.on_notification_clicked(&n.tab_id, &n.id);
                return done(self, Ok(json!({ "opened": n })));
            }
            a @ ("enable" | "disable") => {
                self.state.settings.notifications = a == "enable";
                self.changed();
            }
            a => return done(self, Err(format!("unknown action `{a}` (use list, open, enable or disable)"))),
        }
        let recent: Vec<&Shown> = self.notes.recent.iter().collect();
        done(self, Ok(json!({ "enabled": self.state.settings.notifications, "recent": recent })))
    }
}
