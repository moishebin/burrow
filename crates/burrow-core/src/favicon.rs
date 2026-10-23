//! Finding a site's icon for tabs that haven't loaded yet, and naming the icon cache (D-025).

use crate::links;

/// Hosts that serve several apps, each with its own icon, told apart by the first part of the
/// path: Gmail and Google Chat on mail.google.com, Docs, Sheets, Slides and Forms on
/// docs.google.com. Their icons are kept per app (D-049).
pub const SPLIT_HOSTS: &[&str] = &["mail.google.com", "docs.google.com"];

/// The key a page's icon is saved under: its host, plus the first part of the path on
/// `SPLIT_HOSTS` (`docs.google.com/spreadsheets`). None for anything but http(s) URLs.
pub fn icon_key(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        return None;
    }
    let host = links::host_of(url);
    if host.is_empty() {
        return None;
    }
    if !SPLIT_HOSTS.contains(&host.as_str()) {
        return Some(host);
    }
    let rest = url.split_once("://").map_or("", |(_, r)| r);
    let path = rest.find('/').map_or("", |i| &rest[i + 1..]);
    let first = path.split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
    Some(if first.is_empty() { host } else { format!("{host}/{first}") })
}

/// Whether a URL looks like a sign-in page (`accounts.google.com`, `login.microsoftonline.com`,
/// `…/signin`). An icon lookup that ends on one would save the sign-in page's icon (D-052).
pub fn is_sign_in(url: &str) -> bool {
    let host = links::host_of(url);
    let first = host.split('.').next().unwrap_or("");
    if matches!(first, "accounts" | "login" | "signin" | "auth" | "sso" | "id") {
        return true;
    }
    let rest = url.split_once("://").map_or("", |(_, r)| r);
    let path = rest.find('/').map_or("", |i| &rest[i..]).split(['?', '#']).next().unwrap_or("");
    let path = path.to_ascii_lowercase();
    ["login", "signin", "sign-in", "sign_in", "oauth", "authorize", "sso"].iter().any(|w| path.contains(w))
}

/// The key a cache file holds, from its name without `.png` (`file_name` turns `/` into `_`).
pub fn key_of_file(stem: &str) -> String {
    for host in SPLIT_HOSTS {
        if let Some(app) = stem.strip_prefix(host).and_then(|r| r.strip_prefix('_')) {
            return format!("{host}/{app}");
        }
    }
    stem.to_string()
}

/// The cache file name for an icon key, e.g. `mail.google.com.png`, `docs.google.com_spreadsheets.png`.
pub fn file_name(host: &str) -> String {
    let safe: String = host
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c.to_ascii_lowercase() } else { '_' })
        .collect();
    format!("{safe}.png")
}

/// Resolves `href` against the page `base` URL.
pub fn resolve(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("data:image/") {
        return Some(href.to_string());
    }
    let (scheme, rest) = base.split_once("://")?;
    if let Some(r) = href.strip_prefix("//") {
        return Some(format!("{scheme}://{r}"));
    }
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let origin = format!("{scheme}://{}", &rest[..host_end]);
    if href.starts_with('/') {
        return Some(format!("{origin}{href}"));
    }
    let path = &rest[host_end..];
    let path = path.split(['?', '#']).next().unwrap_or("");
    let dir = path.rfind('/').map_or("/", |i| &path[..=i]);
    Some(format!("{origin}{dir}{href}"))
}

/// The icon URLs a page's HTML names, best first, ending with `/favicon.ico`. Big icons win
/// (`apple-touch-icon`, `sizes="192x192"`), so they stay sharp on high-DPI screens.
pub fn candidates(html: &str, page_url: &str) -> Vec<String> {
    let mut found: Vec<(i32, String)> = vec![];
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<link") {
        let start = at + i;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        let tag = &html[start..end];
        at = end;
        let rel = attr(tag, "rel").unwrap_or_default().to_ascii_lowercase();
        if !rel.split_whitespace().any(|r| r == "icon" || r == "apple-touch-icon" || r == "apple-touch-icon-precomposed") {
            continue;
        }
        let Some(url) = attr(tag, "href").and_then(|h| resolve(page_url, &decode_entities(&h))) else { continue };
        let size = attr(tag, "sizes")
            .and_then(|s| s.split(['x', 'X']).next().and_then(|n| n.trim().parse::<i32>().ok()))
            .unwrap_or(if rel.contains("apple") { 180 } else { 16 });
        // Prefer something around 32–192px; SVGs scale, very large PNGs are wasteful.
        let score = if url.to_ascii_lowercase().ends_with(".svg") { 150 } else if size > 256 { 100 } else { size };
        found.push((score, url));
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    let mut out: Vec<String> = vec![];
    for (_, u) in found {
        if !out.contains(&u) {
            out.push(u);
        }
    }
    if let Some(ico) = resolve(page_url, "/favicon.ico")
        && !out.contains(&ico)
    {
        out.push(ico);
    }
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let pos = from + i;
        from = pos + name.len();
        let before_ok = pos > 0 && lower.as_bytes()[pos - 1].is_ascii_whitespace();
        let rest = lower[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value_start = tag.len() - rest.len() + 1;
        let v = tag[value_start..].trim_start();
        return Some(match v.chars().next() {
            Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or("").to_string(),
            _ => v.split(char::is_whitespace).next().unwrap_or("").to_string(),
        });
    }
    None
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&").replace("&#x2F;", "/").replace("&#47;", "/").replace("&quot;", "\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("Mail.Google.com"), "mail.google.com.png");
        assert_eq!(file_name("../evil"), ".._evil.png");
    }

    #[test]
    fn google_apps_get_their_own_icons() {
        assert_eq!(icon_key("https://mail.google.com/mail/u/0/#inbox").unwrap(), "mail.google.com/mail");
        assert_eq!(icon_key("https://mail.google.com/chat/u/0/?x=1#chat/home").unwrap(), "mail.google.com/chat");
        assert_eq!(icon_key("https://docs.google.com/spreadsheets/d/1Vk/edit").unwrap(), "docs.google.com/spreadsheets");
        assert_eq!(icon_key("https://docs.google.com/").unwrap(), "docs.google.com");
        assert_eq!(icon_key("https://www.GitHub.com/a/b").unwrap(), "github.com");
        assert_eq!(icon_key("burrow://ui/x"), None);
        assert_eq!(file_name("docs.google.com/spreadsheets"), "docs.google.com_spreadsheets.png");
        assert_eq!(key_of_file("docs.google.com_spreadsheets"), "docs.google.com/spreadsheets");
        assert_eq!(key_of_file("mail.google.com"), "mail.google.com");
        assert_eq!(key_of_file("app.slack.com"), "app.slack.com");
    }

    #[test]
    fn sign_in_pages_are_recognized() {
        assert!(is_sign_in("https://accounts.google.com/v3/signin/identifier?continue=x"));
        assert!(is_sign_in("https://login.microsoftonline.com/organizations/oauth2/v2.0/authorize?x"));
        assert!(is_sign_in("https://app.slack.com/workspace-signin?redir=x"));
        assert!(is_sign_in("https://github.com/login"));
        assert!(!is_sign_in("https://chat.google.com/u/0/app/home"));
        assert!(!is_sign_in("https://mail.google.com/mail/u/0/?flowName=GlifWebSignIn#inbox"));
        assert!(!is_sign_in("https://workspace.google.com/intl/en-US/gmail/"));
    }

    #[test]
    fn relative_links_resolve() {
        let base = "https://app.example.com/team/inbox?x=1";
        assert_eq!(resolve(base, "/i.png").unwrap(), "https://app.example.com/i.png");
        assert_eq!(resolve(base, "i.png").unwrap(), "https://app.example.com/team/i.png");
        assert_eq!(resolve(base, "//cdn.example.com/i.png").unwrap(), "https://cdn.example.com/i.png");
        assert_eq!(resolve("https://a.example", "i.png").unwrap(), "https://a.example/i.png");
        assert_eq!(resolve(base, "https://x.example/i.ico").unwrap(), "https://x.example/i.ico");
    }

    #[test]
    fn picks_big_icons_first_and_falls_back_to_favicon_ico() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="/s.css">
            <LINK REL="shortcut icon" href="/small.ico">
            <link href='/icons/192.png' rel='icon' sizes='192x192'>
            <link rel="apple-touch-icon" href="https://cdn.example/touch.png">
            <link rel=icon href=/plain.png>
            </head></html>"#;
        assert_eq!(
            candidates(html, "https://trello.com/b/1"),
            vec![
                "https://trello.com/icons/192.png",
                "https://cdn.example/touch.png",
                "https://trello.com/small.ico",
                "https://trello.com/plain.png",
                "https://trello.com/favicon.ico",
            ]
        );
        assert_eq!(candidates("<p>no icons</p>", "https://a.example/x"), vec!["https://a.example/favicon.ico"]);
    }
}
