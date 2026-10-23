//! Finding a site's icon for tabs that haven't loaded yet, and naming the icon cache (D-025).

use crate::links;

/// The key a site's icon is saved under: its host. None for anything but http(s) URLs.
pub fn icon_key(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        return None;
    }
    let host = links::host_of(url);
    (!host.is_empty()).then_some(host)
}

/// The cache file name for an icon key, e.g. `mail.google.com.png`.
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
    fn icons_are_kept_per_host() {
        assert_eq!(icon_key("https://mail.google.com/mail/u/0/#inbox").unwrap(), "mail.google.com");
        assert_eq!(icon_key("https://www.GitHub.com/a/b").unwrap(), "github.com");
        assert_eq!(icon_key("burrow://ui/x"), None);
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
