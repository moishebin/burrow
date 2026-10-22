//! Where a link goes: Air Traffic Control rules (D-021) and Peek (D-022).

use crate::model::*;

/// The host of a URL, lowercased and without `www.`. Empty for URLs without one.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or("", |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host.starts_with('[') {
        host.split_inclusive(']').next().unwrap_or(host)
    } else {
        host.split(':').next().unwrap_or(host)
    };
    let host = host.to_ascii_lowercase();
    host.strip_prefix("www.").map(String::from).unwrap_or(host)
}

/// Second-level labels under which country domains register names (`example.co.uk`).
const SECOND_LEVEL: &[&str] = &["co", "com", "org", "net", "gov", "ac", "edu", "ne", "or", "go"];

/// The registrable part of a host ("site"), roughly: `mail.google.com` → `google.com`,
/// `news.bbc.co.uk` → `bbc.co.uk`. IP addresses and single labels are their own site.
pub fn site_of(host: &str) -> &str {
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return host;
    }
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();
    let keep = if n >= 3 && labels[n - 1].len() == 2 && SECOND_LEVEL.contains(&labels[n - 2]) { 3 } else { 2 };
    if n <= keep {
        return host;
    }
    let skip: usize = labels[..n - keep].iter().map(|l| l.len() + 1).sum();
    &host[skip..]
}

/// Whether two URLs belong to the same site (same registrable domain).
pub fn same_site(a: &str, b: &str) -> bool {
    let (ha, hb) = (host_of(a), host_of(b));
    !ha.is_empty() && site_of(&ha) == site_of(&hb)
}

/// Normalizes what the user typed as a rule: lowercase, no scheme, no `www.`, no trailing `/`.
pub fn clean_pattern(pattern: &str) -> String {
    let p = pattern.trim().to_ascii_lowercase();
    let p = p.split_once("://").map_or(p.as_str(), |(_, r)| r);
    let p = p.strip_prefix("www.").unwrap_or(p);
    p.trim_end_matches('/').to_string()
}

impl Route {
    /// A pattern without `/` is a domain and also matches its subdomains. With a `/` it matches
    /// addresses that start with it (scheme and `www.` ignored).
    pub fn matches(&self, url: &str) -> bool {
        let pattern = clean_pattern(&self.pattern);
        if pattern.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
            return false;
        }
        let host = host_of(url);
        if !pattern.contains('/') {
            return host == pattern || host.ends_with(&format!(".{pattern}"));
        }
        let (p_host, p_path) = pattern.split_once('/').unwrap_or((&pattern, ""));
        let rest = url.split_once("://").map_or("", |(_, r)| r);
        let path = rest.find(['/', '?', '#']).map_or("", |i| &rest[i..]).to_ascii_lowercase();
        let path = path.strip_prefix('/').unwrap_or(&path);
        host == p_host && path.starts_with(p_path) && {
            // `github.com/org` must not match `github.com/organizer`.
            let next = path[p_path.len()..].chars().next();
            p_path.is_empty() || p_path.ends_with(['/', '?', '=', '&']) || matches!(next, None | Some('/' | '?' | '#'))
        }
    }
}

/// Sign-in pages stay in the tab that opened them, so the site's redirect back lands there.
pub fn looks_like_sign_in(url: &str) -> bool {
    let host = host_of(url);
    let first = host.split('.').next().unwrap_or("");
    let rest = url.split_once("://").map_or("", |(_, r)| r);
    let path = rest.find(['/', '?']).map_or("", |i| &rest[i..]).to_ascii_lowercase();
    ["login", "signin", "sso", "auth", "accounts", "account", "id", "idp", "oauth"].contains(&first)
        || ["/oauth", "/login", "/signin", "/sign-in", "/sso", "/saml", "/authorize", "/auth/"]
            .iter()
            .any(|p| path.starts_with(p) || path.contains(&format!("{p}/")) || path.contains(&format!("{p}?")))
}

/// How a link asked to be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// `target=_blank` or `window.open`: opens in a Peek when that is on.
    NewTab,
    /// The user asked for a tab ("Open Link in New Tab", Shift+click): never a Peek.
    ForegroundTab,
    /// Ctrl/middle click, "Open Link in Background Tab": a tab for later.
    BackgroundTab,
    /// From another program (Burrow as the default browser).
    External,
}

/// Where to open a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// A Today tab in this space. `routed` = an Air Traffic Control rule chose the space.
    Tab { space_id: Id, foreground: bool, routed: bool },
    /// A Peek over the current page, in the source tab's profile.
    Peek,
}

impl State {
    /// The space an Air Traffic Control rule sends `url` to. Longer (more specific) patterns win.
    pub fn route_for(&self, url: &str) -> Option<&Route> {
        self.routes
            .iter()
            .filter(|r| r.matches(url) && self.space(&r.space_id).is_some())
            .max_by_key(|r| clean_pattern(&r.pattern).len())
    }

    /// Decides where a link opened from `source_tab` (if any) goes.
    pub fn link_destination(&self, source_tab: Option<&str>, url: &str, kind: LinkKind) -> Destination {
        let from_space = source_tab
            .and_then(|t| self.space_of(t))
            .map_or_else(|| self.active_space.clone(), |s| s.id.clone());
        if let Some(route) = self.route_for(url)
            && (route.space_id != from_space || kind == LinkKind::External)
        {
            let foreground = kind != LinkKind::BackgroundTab;
            return Destination::Tab { space_id: route.space_id.clone(), foreground, routed: true };
        }
        match kind {
            LinkKind::NewTab if self.settings.peek_new_tabs && source_tab.is_some() => Destination::Peek,
            LinkKind::BackgroundTab => Destination::Tab { space_id: from_space, foreground: false, routed: false },
            _ => Destination::Tab { space_id: from_space, foreground: true, routed: false },
        }
    }

    /// A link click inside a pinned tab or favorite that leaves its site opens in a Peek
    /// instead, so the tab keeps its place on its own site (D-022).
    pub fn peeks_navigation(&self, tab_id: &str, from_url: &str, to_url: &str) -> bool {
        if !self.settings.peek_from_pinned || !(to_url.starts_with("http://") || to_url.starts_with("https://")) {
            return false;
        }
        let Some(space) = self.space_of(tab_id) else { return false };
        let pinned = matches!(space.section_of(tab_id), Some(Section::Pinned | Section::Favorites));
        let home = space.tab(tab_id).and_then(|t| t.home_url.as_deref()).unwrap_or(from_url);
        pinned && !same_site(from_url, to_url) && !same_site(home, to_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, Location};

    fn route(pattern: &str, space_id: &str) -> Route {
        Route { id: new_id(), pattern: pattern.into(), space_id: space_id.into() }
    }

    #[test]
    fn hosts_and_sites() {
        assert_eq!(host_of("https://www.GitHub.com:443/x?y"), "github.com");
        assert_eq!(host_of("https://user@mail.google.com/"), "mail.google.com");
        assert_eq!(host_of("about:blank"), "");
        assert_eq!(site_of("mail.google.com"), "google.com");
        assert_eq!(site_of("news.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(site_of("google.com"), "google.com");
        assert_eq!(site_of("192.168.1.1"), "192.168.1.1");
        assert!(same_site("https://mail.google.com/x", "https://accounts.google.com/y"));
        assert!(!same_site("https://github.com", "https://gitlab.com"));
        assert!(!same_site("about:blank", "about:blank"));
    }

    #[test]
    fn sign_in_pages_are_recognized() {
        assert!(looks_like_sign_in("https://login.microsoftonline.com/common/oauth2"));
        assert!(looks_like_sign_in("https://accounts.google.com/o/oauth2/auth"));
        assert!(looks_like_sign_in("https://github.com/login/oauth/authorize?x=1"));
        assert!(looks_like_sign_in("https://example.okta.com/sso/saml"));
        assert!(!looks_like_sign_in("https://en.wikipedia.org/wiki/Login"));
        assert!(!looks_like_sign_in("https://news.example.com/authors/jane"));
    }

    #[test]
    fn domain_rules_match_subdomains_only() {
        let r = route("Figma.com", "s");
        assert!(r.matches("https://figma.com/file/1"));
        assert!(r.matches("https://www.figma.com/"));
        assert!(r.matches("https://help.figma.com/"));
        assert!(!r.matches("https://notfigma.com/"));
        assert!(!r.matches("https://figma.com.evil.example/"));
        assert!(!r.matches("file:///figma.com"));
    }

    #[test]
    fn path_rules_match_whole_segments() {
        let r = route("https://github.com/my-org/", "s");
        assert!(r.matches("https://github.com/my-org"));
        assert!(r.matches("https://github.com/My-Org/repo/pulls"));
        assert!(!r.matches("https://github.com/my-organizer"));
        assert!(!r.matches("https://gist.github.com/my-org"));
    }

    fn two_spaces() -> (State, Id, Id) {
        let mut s = State::default();
        let home = s.active_space.clone();
        s.apply(Command::NewSpace { name: "Work".into(), icon: String::new(), profile_id: None }).unwrap();
        let work = s.active_space.clone();
        s.active_space = home.clone();
        (s, home, work)
    }

    #[test]
    fn rules_send_links_to_their_space() {
        let (mut s, home, work) = two_spaces();
        s.routes.push(route("github.com", &work));
        s.routes.push(route("github.com/personal", &home));
        let src = s.open_tab("https://news.example".into(), None, true).unwrap().0;
        assert_eq!(
            s.link_destination(Some(&src), "https://github.com/acme", LinkKind::NewTab),
            Destination::Tab { space_id: work.clone(), foreground: true, routed: true }
        );
        // The more specific rule wins, and a link to the space it is already in is not routed.
        assert_eq!(s.link_destination(Some(&src), "https://github.com/personal/x", LinkKind::NewTab), Destination::Peek);
        assert_eq!(
            s.link_destination(None, "https://github.com/acme", LinkKind::BackgroundTab),
            Destination::Tab { space_id: work, foreground: false, routed: true }
        );
    }

    #[test]
    fn new_tab_links_peek_unless_turned_off() {
        let (mut s, home, _) = two_spaces();
        let src = s.open_tab("https://news.example".into(), None, true).unwrap().0;
        assert_eq!(s.link_destination(Some(&src), "https://a.example", LinkKind::NewTab), Destination::Peek);
        assert_eq!(
            s.link_destination(Some(&src), "https://a.example", LinkKind::BackgroundTab),
            Destination::Tab { space_id: home.clone(), foreground: false, routed: false }
        );
        assert_eq!(
            s.link_destination(None, "https://a.example", LinkKind::External),
            Destination::Tab { space_id: home.clone(), foreground: true, routed: false }
        );
        s.settings.peek_new_tabs = false;
        assert_eq!(
            s.link_destination(Some(&src), "https://a.example", LinkKind::NewTab),
            Destination::Tab { space_id: home, foreground: true, routed: false }
        );
    }

    #[test]
    fn pinned_tabs_peek_links_to_other_sites() {
        let mut s = State::default();
        let t = s.open_tab("https://mail.google.com/inbox".into(), None, true).unwrap().0;
        let today = |s: &State| s.peeks_navigation(&t, "https://mail.google.com/inbox", "https://example.com/");
        assert!(!today(&s), "Today tabs navigate normally");
        let to = Location { space_id: None, section: Section::Pinned, parent: None, index: None };
        s.apply(Command::MoveNode { node_id: t.clone(), to }).unwrap();
        assert!(today(&s));
        assert!(!s.peeks_navigation(&t, "https://mail.google.com/inbox", "https://accounts.google.com/x"));
        assert!(!s.peeks_navigation(&t, "https://mail.google.com/inbox", "mailto:a@b.c"));
        s.settings.peek_from_pinned = false;
        assert!(!today(&s));
    }
}
