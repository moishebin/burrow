//! Turning what the user typed into a URL to load.

/// Returns a URL for `input`: the input itself if it looks like an address, otherwise a search.
pub fn resolve(input: &str, search_url: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return String::new();
    }
    let has_scheme = s
        .split_once(':')
        .is_some_and(|(scheme, rest)| {
            !scheme.is_empty()
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
                && (rest.starts_with("//")
                    || matches!(scheme, "about" | "data" | "file" | "chrome" | "view-source" | "mailto"))
        });
    if has_scheme {
        return s.to_string();
    }
    if !s.contains(char::is_whitespace) && looks_like_host(s) {
        let host = s.split(['/', '?', '#']).next().unwrap_or(s);
        let scheme = if host.starts_with("localhost") || host.starts_with("127.") { "http" } else { "https" };
        return format!("{scheme}://{s}");
    }
    search_url.replace("{}", &encode_query(s))
}

fn looks_like_host(s: &str) -> bool {
    let host = s.split(['/', '?', '#']).next().unwrap_or(s);
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let name = host.split(':').next().unwrap_or(host);
    if name == "localhost" {
        return true;
    }
    let labels: Vec<&str> = name.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && l.chars().all(|c| c.is_alphanumeric() || c == '-'))
        && labels.last().is_some_and(|tld| tld.chars().any(|c| c.is_alphabetic()) || labels.len() == 4)
}

pub fn encode_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::resolve;
    const S: &str = "https://search.example/?q={}";

    #[test]
    fn urls_pass_through_or_get_https() {
        assert_eq!(resolve("https://a.com/x", S), "https://a.com/x");
        assert_eq!(resolve("github.com/foo", S), "https://github.com/foo");
        assert_eq!(resolve("localhost:3000", S), "http://localhost:3000");
        assert_eq!(resolve("192.168.1.1", S), "https://192.168.1.1");
        assert_eq!(resolve("about:blank", S), "about:blank");
    }

    #[test]
    fn everything_else_is_a_search() {
        assert_eq!(resolve("rust cef", S), "https://search.example/?q=rust+cef");
        assert_eq!(resolve("hello", S), "https://search.example/?q=hello");
        assert_eq!(resolve("c++ & rust", S), "https://search.example/?q=c%2B%2B+%26+rust");
        assert_eq!(resolve("v1.2", S), "https://search.example/?q=v1.2");
    }
}
