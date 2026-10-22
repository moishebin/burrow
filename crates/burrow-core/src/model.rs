//! The persistent sidebar model: profiles, spaces, favorites, pinned folders/tabs and today's tabs.
//!
//! Nothing here knows about CEF. The app layer owns the actual browsers and keys them by `Tab::id`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::extensions::Extension;

pub type Id = String;

pub fn new_id() -> Id {
    uuid::Uuid::new_v4().to_string()
}

pub const STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub profiles: Vec<Profile>,
    pub spaces: Vec<Space>,
    pub active_space: Id,
    #[serde(default)]
    pub settings: Settings,
    /// Recently closed Today tabs, newest first (Arc's archive). Capped at `ARCHIVE_LIMIT`.
    #[serde(default)]
    pub archive: Vec<ArchivedTab>,
    /// Remembered answers to site permission prompts.
    #[serde(default)]
    pub site_permissions: Vec<SitePermission>,
    /// Air Traffic Control: links that match a rule open in the rule's space (D-021).
    #[serde(default)]
    pub routes: Vec<Route>,
    /// Remembered page zoom per host (Chromium zoom levels; 0 = 100%).
    #[serde(default)]
    pub zoom: BTreeMap<String, f64>,
    /// Boosts: the user's own CSS for a host, added to every page of it.
    #[serde(default)]
    pub boosts: BTreeMap<String, String>,
    /// Installed Chrome extensions (D-032).
    #[serde(default)]
    pub extensions: Vec<Extension>,
}

/// One Air Traffic Control rule (D-021).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub id: Id,
    /// A domain (`github.com`, also matches its subdomains) or, with a `/`, the start of an
    /// address without its scheme (`github.com/my-org`).
    pub pattern: String,
    pub space_id: Id,
}

pub const ARCHIVE_LIMIT: usize = 300;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchivedTab {
    pub url: String,
    #[serde(default)]
    pub title: String,
    pub space_id: Id,
    pub closed_ms: u64,
}

/// "This site may / may not use X" for one profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SitePermission {
    pub profile_id: Id,
    /// Scheme + host + port, e.g. `https://meet.google.com`.
    pub origin: String,
    /// e.g. `camera`, `microphone`, `notifications`.
    pub permission: String,
    pub allow: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Search URL with `{}` where the query goes.
    pub search_url: String,
    /// Today tabs not used for this many hours are archived. 0 = never.
    #[serde(default = "default_archive_hours")]
    pub archive_after_hours: u32,
    /// Use the Windows title bar instead of Burrow's frameless, Arc-style window. Needs a restart.
    #[serde(default)]
    pub system_title_bar: bool,
    /// AI agents may read and use pages (screenshots, clicks, typing, scripts). Managing tabs and
    /// spaces is always allowed (D-019).
    #[serde(default = "yes")]
    pub agent_page_access: bool,
    /// Links that would open a new tab open in a Peek over the page instead (D-022).
    #[serde(default = "yes")]
    pub peek_new_tabs: bool,
    /// In pinned tabs and favorites, links to another site open in a Peek, so the tab stays on
    /// its own site (Arc's behaviour, D-022).
    #[serde(default = "yes")]
    pub peek_from_pinned: bool,
    /// The sidebar's width in DIPs, set by dragging its right edge (D-033).
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: u32,
    /// Leaving a tab whose video plays with sound floats the video in picture in picture (D-036).
    #[serde(default = "yes")]
    pub auto_pip: bool,
}

pub const SIDEBAR_WIDTH_DEFAULT: u32 = 264;
pub const SIDEBAR_WIDTH_MIN: u32 = 200;
pub const SIDEBAR_WIDTH_MAX: u32 = 600;

fn default_sidebar_width() -> u32 {
    SIDEBAR_WIDTH_DEFAULT
}

/// A sidebar width the window can show: between the limits, whatever was asked for.
pub fn clamp_sidebar_width(width: i64) -> u32 {
    width.clamp(SIDEBAR_WIDTH_MIN as i64, SIDEBAR_WIDTH_MAX as i64) as u32
}

fn yes() -> bool {
    true
}

fn default_archive_hours() -> u32 {
    12
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            search_url: "https://www.google.com/search?q={}".into(),
            archive_after_hours: default_archive_hours(),
            system_title_bar: false,
            agent_page_access: true,
            peek_new_tabs: true,
            peek_from_pinned: true,
            sidebar_width: SIDEBAR_WIDTH_DEFAULT,
            auto_pip: true,
        }
    }
}

/// A browser profile: its own cookies, storage and logins (D-004).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: Id,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Space {
    pub id: Id,
    pub name: String,
    /// An emoji, or empty for the default dot.
    #[serde(default)]
    pub icon: String,
    /// Sidebar tint, as a CSS hex color.
    pub color: String,
    pub profile_id: Id,
    /// The app tiles at the top of the sidebar. Per space, unlike Arc (D-005).
    #[serde(default)]
    pub favorites: Vec<Tab>,
    /// Pinned tabs and folders. Their place never changes unless the user moves them.
    #[serde(default)]
    pub pinned: Vec<Node>,
    /// Unpinned tabs, newest first.
    #[serde(default)]
    pub today: Vec<Tab>,
    #[serde(default)]
    pub active_tab: Option<Id>,
    /// Two tabs shown side by side while either of them is the active tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<Split>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Split {
    pub left: Id,
    pub right: Id,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Node {
    Folder(Folder),
    Tab(Tab),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: Id,
    /// Where the tab is now.
    pub url: String,
    /// The page title, as last reported by the browser.
    #[serde(default)]
    pub title: String,
    /// A name the user gave the tab; shown instead of `title`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    /// For pinned tabs and favorites: the URL the tab belongs to. Closing the tab returns it here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_url: Option<String>,
    /// Milliseconds since the Unix epoch.
    #[serde(default)]
    pub last_active_ms: u64,
}

impl Tab {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            url: url.into(),
            title: String::new(),
            custom_title: None,
            favicon: None,
            home_url: None,
            last_active_ms: 0,
        }
    }

    pub fn display_title(&self) -> &str {
        match &self.custom_title {
            Some(t) if !t.is_empty() => t,
            _ if !self.title.is_empty() => &self.title,
            _ => &self.url,
        }
    }
}

impl Node {
    pub fn id(&self) -> &Id {
        match self {
            Node::Folder(f) => &f.id,
            Node::Tab(t) => &t.id,
        }
    }
}

/// Which list of a space something lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Favorites,
    Pinned,
    Today,
}

/// Default sidebar tints for new spaces, cycled in order.
pub const SPACE_COLORS: &[&str] = &[
    "#c9c2f0", "#f3c4c4", "#bfe3d0", "#f6dcb0", "#bcd6f2", "#e8c6ea", "#d6d6d6",
];

impl Default for State {
    fn default() -> Self {
        let profile = Profile {
            id: new_id(),
            name: "Default".into(),
        };
        let space = Space::new("Personal", "", SPACE_COLORS[0], &profile.id);
        Self {
            version: STATE_VERSION,
            active_space: space.id.clone(),
            settings: Settings::default(),
            archive: Vec::new(),
            site_permissions: Vec::new(),
            routes: Vec::new(),
            zoom: BTreeMap::new(),
            boosts: BTreeMap::new(),
            extensions: Vec::new(),
            profiles: vec![profile],
            spaces: vec![space],
        }
    }
}

impl Space {
    pub fn new(name: &str, icon: &str, color: &str, profile_id: &str) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            icon: icon.into(),
            color: color.into(),
            profile_id: profile_id.into(),
            favorites: Vec::new(),
            pinned: Vec::new(),
            today: Vec::new(),
            active_tab: None,
            split: None,
        }
    }

    /// Every tab in the space, in sidebar order.
    pub fn tabs(&self) -> Vec<&Tab> {
        fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Tab>) {
            for n in nodes {
                match n {
                    Node::Tab(t) => out.push(t),
                    Node::Folder(f) => walk(&f.children, out),
                }
            }
        }
        let mut out: Vec<&Tab> = self.favorites.iter().collect();
        walk(&self.pinned, &mut out);
        out.extend(self.today.iter());
        out
    }

    pub fn tab(&self, id: &str) -> Option<&Tab> {
        self.tabs().into_iter().find(|t| t.id == id)
    }

    pub fn tab_mut(&mut self, id: &str) -> Option<&mut Tab> {
        fn walk<'a>(nodes: &'a mut [Node], id: &str) -> Option<&'a mut Tab> {
            for n in nodes {
                match n {
                    Node::Tab(t) if t.id == id => return Some(t),
                    Node::Folder(f) => {
                        if let Some(t) = walk(&mut f.children, id) {
                            return Some(t);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        if let Some(t) = self.favorites.iter_mut().find(|t| t.id == id) {
            return Some(t);
        }
        if let Some(t) = walk(&mut self.pinned, id) {
            return Some(t);
        }
        self.today.iter_mut().find(|t| t.id == id)
    }

    pub fn folder_mut(&mut self, id: &str) -> Option<&mut Folder> {
        find_folder_mut(&mut self.pinned, id)
    }

    /// The section a node (tab or folder) lives in.
    pub fn section_of(&self, id: &str) -> Option<Section> {
        if self.favorites.iter().any(|t| t.id == id) {
            Some(Section::Favorites)
        } else if contains_node(&self.pinned, id) {
            Some(Section::Pinned)
        } else if self.today.iter().any(|t| t.id == id) {
            Some(Section::Today)
        } else {
            None
        }
    }
}

pub(crate) fn find_folder_mut<'a>(nodes: &'a mut [Node], id: &str) -> Option<&'a mut Folder> {
    for n in nodes {
        if let Node::Folder(f) = n {
            if f.id == id {
                return Some(f);
            }
            if let Some(found) = find_folder_mut(&mut f.children, id) {
                return Some(found);
            }
        }
    }
    None
}

pub(crate) fn contains_node(nodes: &[Node], id: &str) -> bool {
    nodes.iter().any(|n| match n {
        Node::Tab(t) => t.id == id,
        Node::Folder(f) => f.id == id || contains_node(&f.children, id),
    })
}

/// Removes a node from a tree, wherever it is nested.
pub(crate) fn remove_node(nodes: &mut Vec<Node>, id: &str) -> Option<Node> {
    if let Some(i) = nodes.iter().position(|n| n.id() == id) {
        return Some(nodes.remove(i));
    }
    for n in nodes.iter_mut() {
        if let Node::Folder(f) = n
            && let Some(found) = remove_node(&mut f.children, id) {
                return Some(found);
            }
    }
    None
}

impl State {
    pub fn space(&self, id: &str) -> Option<&Space> {
        self.spaces.iter().find(|s| s.id == id)
    }

    pub fn space_mut(&mut self, id: &str) -> Option<&mut Space> {
        self.spaces.iter_mut().find(|s| s.id == id)
    }

    pub fn active_space(&self) -> &Space {
        self.space(&self.active_space)
            .or_else(|| self.spaces.first())
            .expect("state always has at least one space")
    }

    pub fn active_space_mut(&mut self) -> &mut Space {
        let id = self.active_space.clone();
        let idx = self.spaces.iter().position(|s| s.id == id).unwrap_or(0);
        &mut self.spaces[idx]
    }

    /// The tab showing in the content area, if any.
    pub fn active_tab(&self) -> Option<&Tab> {
        let space = self.active_space();
        space.active_tab.as_deref().and_then(|id| space.tab(id))
    }

    /// The tabs the content area shows, left to right: the active tab, or both halves of its
    /// split view.
    pub fn visible_tabs(&self) -> Vec<Id> {
        let space = self.active_space();
        let Some(active) = self.active_tab().map(|t| t.id.clone()) else { return vec![] };
        match &space.split {
            Some(s) if s.left == active || s.right == active => vec![s.left.clone(), s.right.clone()],
            _ => vec![active],
        }
    }

    /// The space that owns a tab or folder.
    pub fn space_of(&self, node_id: &str) -> Option<&Space> {
        self.spaces.iter().find(|s| s.section_of(node_id).is_some())
    }

    pub fn tab(&self, id: &str) -> Option<&Tab> {
        self.spaces.iter().find_map(|s| s.tab(id))
    }

    pub fn tab_mut(&mut self, id: &str) -> Option<&mut Tab> {
        self.spaces.iter_mut().find_map(|s| s.tab_mut(id))
    }

    /// Which profile a tab's browser must be created in.
    pub fn profile_of_tab(&self, tab_id: &str) -> Option<&Id> {
        self.space_of(tab_id).map(|s| &s.profile_id)
    }

    /// Adds another state's profiles and spaces (e.g. from an Arc import) after our own.
    /// Returns the number of spaces added.
    pub fn absorb(&mut self, other: State) -> usize {
        // A fresh, untouched default space is just a placeholder; drop it.
        if self.spaces.len() == 1 && self.spaces[0].tabs().is_empty() && self.spaces[0].pinned.is_empty() {
            let placeholder = self.spaces.remove(0);
            if self.profiles.len() == 1 && self.profiles[0].id == placeholder.profile_id {
                self.profiles.clear();
            }
        }
        let n = other.spaces.len();
        self.profiles.extend(other.profiles);
        if let Some(first) = other.spaces.first() {
            self.active_space = first.id.clone();
        }
        self.spaces.extend(other.spaces);
        self.normalize();
        n
    }

    /// Records a closed Today tab so it can be reopened.
    pub(crate) fn archive_tab(&mut self, tab: &Tab, space_id: &str, now_ms: u64) {
        if tab.url.is_empty() || tab.url == "about:blank" {
            return;
        }
        self.archive.insert(
            0,
            ArchivedTab {
                url: tab.url.clone(),
                title: tab.display_title().to_string(),
                space_id: space_id.to_string(),
                closed_ms: now_ms,
            },
        );
        self.archive.truncate(ARCHIVE_LIMIT);
    }

    /// The remembered answer for every permission in `permissions`: `Some(true)` only if all are
    /// allowed, `Some(false)` if any is blocked, `None` if the user must be asked.
    pub fn permission_decision(&self, profile_id: &str, origin: &str, permissions: &[&str]) -> Option<bool> {
        let mut all_allowed = true;
        for p in permissions {
            match self
                .site_permissions
                .iter()
                .find(|x| x.profile_id == profile_id && x.origin == origin && x.permission == *p)
            {
                Some(x) if !x.allow => return Some(false),
                Some(_) => {}
                None => all_allowed = false,
            }
        }
        all_allowed.then_some(true)
    }

    pub fn remember_permission(&mut self, profile_id: &str, origin: &str, permissions: &[&str], allow: bool) {
        for p in permissions {
            self.site_permissions
                .retain(|x| !(x.profile_id == profile_id && x.origin == origin && x.permission == *p));
            self.site_permissions.push(SitePermission {
                profile_id: profile_id.into(),
                origin: origin.into(),
                permission: (*p).into(),
                allow,
            });
        }
    }

    /// Repairs references that may have gone stale (e.g. after a hand-edited or imported file).
    pub fn normalize(&mut self) {
        if self.profiles.is_empty() {
            self.profiles.push(Profile {
                id: new_id(),
                name: "Default".into(),
            });
        }
        if self.spaces.is_empty() {
            let p = self.profiles[0].id.clone();
            self.spaces
                .push(Space::new("Personal", "", SPACE_COLORS[0], &p));
        }
        let default_profile = self.profiles[0].id.clone();
        for s in &mut self.spaces {
            if !self.profiles.iter().any(|p| p.id == s.profile_id) {
                s.profile_id = default_profile.clone();
            }
            if let Some(a) = &s.active_tab
                && s.tab(a).is_none() {
                    s.active_tab = None;
                }
            if let Some(split) = &s.split
                && (split.left == split.right || s.tab(&split.left).is_none() || s.tab(&split.right).is_none()) {
                    s.split = None;
                }
        }
        let spaces: Vec<Id> = self.spaces.iter().map(|s| s.id.clone()).collect();
        self.routes.retain(|r| spaces.contains(&r.space_id));
        if self.space(&self.active_space).is_none() {
            self.active_space = self.spaces[0].id.clone();
        }
        self.version = STATE_VERSION;
    }
}
