//! Browsing history, kept in its own file (`history.json`) so `state.json` stays small (D-023).

use std::{fs, io, path::Path};

use serde::{Deserialize, Serialize};

/// The most entries kept; the least recently visited go first.
pub const HISTORY_LIMIT: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Visit {
    pub url: String,
    #[serde(default)]
    pub title: String,
    /// Milliseconds since the Unix epoch.
    pub last_ms: u64,
    #[serde(default = "one")]
    pub count: u32,
}

fn one() -> u32 {
    1
}

/// One visit per URL, most recent first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    pub visits: Vec<Visit>,
}

/// Pages worth remembering: web pages, not blank pages, Burrow's own UI or data URLs.
pub fn is_recordable(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://")) && url.len() <= 4096
}

impl History {
    /// Records a visit to `url` now.
    pub fn record(&mut self, url: &str, title: &str, now_ms: u64) {
        if !is_recordable(url) {
            return;
        }
        let mut visit = match self.visits.iter().position(|v| v.url == url) {
            Some(i) => {
                let mut v = self.visits.remove(i);
                v.count = v.count.saturating_add(1);
                v
            }
            None => Visit { url: url.into(), title: String::new(), last_ms: 0, count: 1 },
        };
        visit.last_ms = now_ms;
        if !title.is_empty() {
            visit.title = title.into();
        }
        self.visits.insert(0, visit);
        self.visits.truncate(HISTORY_LIMIT);
    }

    /// A page's title arrived after its visit was recorded.
    pub fn set_title(&mut self, url: &str, title: &str) {
        if let Some(v) = self.visits.iter_mut().take(50).find(|v| v.url == url) {
            v.title = title.into();
        }
    }

    pub fn remove(&mut self, url: &str) {
        self.visits.retain(|v| v.url != url);
    }

    /// Visits whose title or URL contains every word of `query`, best first: often and recently
    /// visited pages rank higher, and so do matches at the start of the host. An empty query
    /// lists the most recent visits.
    pub fn search(&self, query: &str, limit: usize, now_ms: u64) -> Vec<&Visit> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return self.visits.iter().take(limit).collect();
        }
        let mut hits: Vec<(f64, &Visit)> = self
            .visits
            .iter()
            .filter_map(|v| {
                let title = v.title.to_lowercase();
                let url = v.url.to_lowercase();
                if !words.iter().all(|w| title.contains(w.as_str()) || url.contains(w.as_str())) {
                    return None;
                }
                let host = crate::links::host_of(&v.url);
                let days = now_ms.saturating_sub(v.last_ms) as f64 / 86_400_000.0;
                let mut score = (1.0 + v.count as f64).ln() * 2.0 - days.min(365.0) / 30.0;
                if host.starts_with(words[0].as_str()) {
                    score += 6.0;
                }
                if url.split_once("://").is_some_and(|(_, r)| r.trim_start_matches("www.").trim_end_matches('/') == host) {
                    score += 1.0; // a site's front page
                }
                Some((score, v))
            })
            .collect();
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        hits.into_iter().take(limit).map(|(_, v)| v).collect()
    }
}

/// Loads history; a missing or unreadable file is an empty history.
pub fn load(path: &Path) -> History {
    fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Writes history atomically, like `store::save`.
pub fn save(path: &Path, history: &History) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string(history).map_err(io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400_000;

    #[test]
    fn records_one_entry_per_url_most_recent_first() {
        let mut h = History::default();
        h.record("https://a.example/", "A", 1);
        h.record("https://b.example/", "", 2);
        h.record("https://a.example/", "", 3);
        h.record("about:blank", "", 4);
        h.record("file:///C:/ui/sidebar.html", "", 5);
        assert_eq!(h.visits.len(), 2);
        assert_eq!(h.visits[0].url, "https://a.example/");
        assert_eq!(h.visits[0].count, 2);
        assert_eq!(h.visits[0].title, "A", "an empty title keeps the old one");
        h.set_title("https://b.example/", "B");
        assert_eq!(h.visits[1].title, "B");
    }

    #[test]
    fn search_needs_every_word_and_ranks_frequent_recent_pages() {
        let mut h = History::default();
        let now = 400 * DAY;
        h.record("https://docs.rs/cef/latest/cef/", "cef - Rust", now - 300 * DAY);
        h.record("https://github.com/tauri-apps/cef-rs", "cef-rs: Rust bindings", now - DAY);
        for _ in 0..5 {
            h.record("https://github.com/", "GitHub", now - 2 * DAY);
        }
        let urls = |q: &str| h.search(q, 10, now).into_iter().map(|v| v.url.as_str()).collect::<Vec<_>>();
        assert_eq!(urls("rust cef"), vec!["https://github.com/tauri-apps/cef-rs", "https://docs.rs/cef/latest/cef/"]);
        assert_eq!(urls("git")[0], "https://github.com/");
        assert!(urls("nothing-like-this").is_empty());
        assert_eq!(urls("").len(), 3);
    }

    #[test]
    fn keeps_at_most_the_limit() {
        let mut h = History::default();
        for i in 0..HISTORY_LIMIT + 5 {
            h.record(&format!("https://e.example/{i}"), "", i as u64);
        }
        assert_eq!(h.visits.len(), HISTORY_LIMIT);
        assert_eq!(h.visits.last().unwrap().url, "https://e.example/5");
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("burrow-history-{}", crate::model::new_id()));
        let path = dir.join("history.json");
        let mut h = History::default();
        h.record("https://a.example/", "A", 1);
        save(&path, &h).unwrap();
        assert_eq!(load(&path), h);
        assert_eq!(load(&dir.join("missing.json")), History::default());
        let _ = fs::remove_dir_all(dir);
    }
}
