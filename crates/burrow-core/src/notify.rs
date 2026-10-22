//! Web notifications (D-042). In Burrow's Alloy-style tabs Chromium accepts `new Notification(…)`
//! and says it showed it, but nothing reaches the screen. Burrow gives each page, as it starts
//! loading, a `Notification` of its own that hands notifications to Burrow (a console message
//! with a secret made fresh each run, so pages can't fake one), and Burrow shows them as system
//! notifications. Permission stays Chromium's (and Burrow's prompts): nothing is sent unless the
//! page's real `Notification.permission` is "granted".

use serde::{Deserialize, Serialize};

/// What the page script logs, before `<secret>:` and the notification as JSON.
pub const PREFIX: &str = "__burrow_note__";

/// A notification a page asked to show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// The page's handle for it, to fire its `click` event; empty for ones shown through a
    /// service worker registration.
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub tag: String,
    /// The page's origin, shown as where it came from.
    #[serde(default)]
    pub origin: String,
}

const MAX_TEXT: usize = 300;

/// The notification in a console message, if it carries `secret`. Long texts are cut.
pub fn parse(message: &str, secret: &str) -> Option<Note> {
    let json = message.strip_prefix(PREFIX)?.strip_prefix(secret)?.strip_prefix(':')?;
    let mut n: Note = serde_json::from_str(json).ok()?;
    for s in [&mut n.title, &mut n.body, &mut n.tag, &mut n.origin, &mut n.id] {
        if s.chars().count() > MAX_TEXT {
            *s = s.chars().take(MAX_TEXT).collect::<String>() + "…";
        }
    }
    (!n.title.is_empty() || !n.body.is_empty()).then_some(n)
}

/// The script each page (and frame) runs as it starts loading.
pub fn page_script(secret: &str) -> String {
    let prefix = serde_json::to_string(&format!("{PREFIX}{secret}:")).unwrap_or_default();
    format!(
        r#"(() => {{
  const N = window.Notification;
  if (!N || N.__burrow) return;
  const log = console.log.bind(console), PREFIX = {prefix};
  const live = new Map();
  const send = (n, id) => {{
    log(PREFIX + JSON.stringify({{ id, title: n.title, body: n.body, tag: n.tag, origin: location.origin }}));
  }};
  class Notification extends EventTarget {{
    constructor(title, options) {{
      if (arguments.length === 0) throw new TypeError("Failed to construct 'Notification': 1 argument required, but only 0 present.");
      super();
      options = options || {{}};
      this.title = String(title);
      this.body = options.body == null ? "" : String(options.body);
      this.tag = options.tag == null ? "" : String(options.tag);
      this.icon = options.icon == null ? "" : String(options.icon);
      this.data = options.data === undefined ? null : options.data;
      this.silent = !!options.silent;
      this.requireInteraction = !!options.requireInteraction;
      this.onclick = this.onshow = this.onerror = this.onclose = null;
      const id = Math.random().toString(36).slice(2) + Date.now().toString(36);
      Object.defineProperty(this, "__id", {{ value: id }});
      if (N.permission !== "granted") {{ setTimeout(() => this.__fire("error")); return; }}
      live.set(id, this);
      send(this, id);
      setTimeout(() => this.__fire("show"));
    }}
    __fire(type) {{
      const e = new Event(type);
      this.dispatchEvent(e);
      const h = this["on" + type];
      if (typeof h === "function") h.call(this, e);
    }}
    close() {{ if (live.delete(this.__id)) this.__fire("close"); }}
    static get permission() {{ return N.permission; }}
    static requestPermission(cb) {{ return N.requestPermission(cb); }}
    static get maxActions() {{ return N.maxActions || 0; }}
  }}
  Object.defineProperty(Notification, "__burrow", {{ value: true }});
  Object.defineProperty(window, "Notification", {{ value: Notification, writable: true, configurable: true }});
  // Burrow calls this when its notification is clicked.
  Object.defineProperty(window, "__burrowNote", {{
    value: Object.freeze({{ click(id) {{ const n = live.get(id); if (n) n.__fire("click"); }} }}),
  }});
  // Many sites show notifications through their service worker from the page.
  const R = window.ServiceWorkerRegistration;
  if (R && R.prototype.showNotification) {{
    R.prototype.showNotification = function (title, options) {{
      options = options || {{}};
      if (N.permission !== "granted") return Promise.reject(new TypeError("No notification permission has been granted for this origin."));
      send({{ title: String(title), body: options.body == null ? "" : String(options.body), tag: options.tag == null ? "" : String(options.tag) }}, "");
      return Promise.resolve();
    }};
  }}
}})();"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_messages_with_the_secret_count() {
        let msg = format!(r#"{PREFIX}s3:{{"id":"a1","title":"Hi","body":"there","origin":"https://mail.example"}}"#);
        let n = parse(&msg, "s3").unwrap();
        assert_eq!((n.id.as_str(), n.title.as_str(), n.body.as_str()), ("a1", "Hi", "there"));
        assert_eq!(parse(&msg, "other"), None);
        assert_eq!(parse(r#"{"title":"x"}"#, "s3"), None);
        assert_eq!(parse(&format!(r#"{PREFIX}s3:{{"title":""}}"#), "s3"), None, "nothing to show");
        let long = format!(r#"{PREFIX}s3:{{"title":"{}"}}"#, "x".repeat(1000));
        assert!(parse(&long, "s3").unwrap().title.chars().count() <= MAX_TEXT + 1);
    }

    #[test]
    fn the_page_script_carries_the_secret() {
        let js = page_script("s3");
        assert!(js.contains(&format!(r#"PREFIX = "{PREFIX}s3:""#)));
        assert!(js.contains("N.permission !== \"granted\""));
    }
}
