//! Import from Arc: turns Arc's `StorableSidebar.json` into a Burrow `State`.
//!
//! Arc is a Swift app, and Swift's `Codable` writes keyed enums and dictionaries as flat arrays that
//! alternate key, value (`["unpinned", "<id>", "pinned", "<id>"]`, `[{"default": true}, "<id>"]`).
//! The helpers at the top decode that. Anything we do not recognise is skipped, never an error.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::model::*;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no spaces found in this file")]
    NoSpaces,
}

pub struct ImportedArc {
    pub state: State,
    /// e.g. "3 spaces, 12 folders, 80 pinned tabs, 9 favorites".
    pub summary: String,
}

/// Seconds between the Unix epoch and Apple's reference date (2001-01-01), which Arc's timestamps use.
const APPLE_EPOCH_OFFSET_S: f64 = 978_307_200.0;

/// Folders nest at most this deep; guards against cycles in a damaged file.
const MAX_DEPTH: usize = 32;

pub fn import_arc_sidebar(json: &str) -> Result<ImportedArc, ImportError> {
    let root: Value = serde_json::from_str(json)?;
    let data = Data::collect(&root);
    if data.spaces.is_empty() {
        return Err(ImportError::NoSpaces);
    }

    let mut profiles: Vec<Profile> = Vec::new();
    let mut profile_ids: HashMap<String, Id> = HashMap::new();
    let mut spaces: Vec<Space> = Vec::new();
    let mut folders = 0;
    let mut pinned_tabs = 0;
    let mut favorites_seen: HashSet<String> = HashSet::new();
    let mut favorites_count = 0;

    for (i, arc_space) in data.spaces.iter().enumerate() {
        let profile_name = profile_name(arc_space.get("profile"));
        let profile_id = profile_ids
            .entry(profile_name.clone())
            .or_insert_with(|| {
                let id = new_id();
                profiles.push(Profile {
                    id: id.clone(),
                    name: profile_name.clone(),
                });
                id
            })
            .clone();

        let title = arc_space
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        let name = if title.is_empty() {
            format!("Space {}", i + 1)
        } else {
            title.to_string()
        };
        let color = SPACE_COLORS[i % SPACE_COLORS.len()];
        let mut space = Space::new(&name, &space_icon(arc_space), color, &profile_id);

        let containers = pairs(arc_space.get("containerIDs"));
        let container_of = |label: &str| {
            containers
                .iter()
                .find(|(k, _)| k.as_str() == Some(label))
                .and_then(|(_, v)| v.as_str())
        };
        if let Some(id) = container_of("pinned") {
            space.pinned = data.nodes(id, true, 0, &mut folders, &mut pinned_tabs);
        }
        if let Some(id) = container_of("unpinned") {
            space.today = data.today_tabs(id);
        }
        if let Some(id) = data.top_apps.get(&profile_name) {
            space.favorites = data.favorites(id);
            if favorites_seen.insert(profile_name.clone()) {
                favorites_count += space.favorites.len();
            }
        }
        spaces.push(space);
    }

    let mut state = State {
        version: STATE_VERSION,
        active_space: spaces[0].id.clone(),
        settings: Default::default(),
        archive: Vec::new(),
        site_permissions: Vec::new(),
        routes: Vec::new(),
        zoom: Default::default(),
        boosts: Default::default(),
        profiles,
        spaces,
    };
    state.normalize();
    let summary = format!(
        "{} {}, {} {}, {} pinned {}, {} {}",
        state.spaces.len(),
        plural(state.spaces.len(), "space", "spaces"),
        folders,
        plural(folders, "folder", "folders"),
        pinned_tabs,
        plural(pinned_tabs, "tab", "tabs"),
        favorites_count,
        plural(favorites_count, "favorite", "favorites"),
    );
    Ok(ImportedArc { state, summary })
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

/// Decodes a Swift keyed collection: `[k1, v1, k2, v2, ...]`. Also accepts a plain JSON object.
fn pairs(v: Option<&Value>) -> Vec<(Value, &Value)> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .step_by(2)
            .zip(a.iter().skip(1).step_by(2))
            .map(|(k, v)| (k.clone(), v))
            .collect(),
        Some(Value::Object(o)) => o
            .iter()
            .map(|(k, v)| (Value::String(k.clone()), v))
            .collect(),
        _ => Vec::new(),
    }
}

/// Objects in an alternating `id, object, id, object` list. The objects carry their own ids.
fn objects(v: Option<&Value>) -> Vec<&Value> {
    match v {
        Some(Value::Array(a)) => a.iter().filter(|x| x.is_object()).collect(),
        Some(Value::Object(o)) => o.values().filter(|x| x.is_object()).collect(),
        _ => Vec::new(),
    }
}

/// `{"default": ..}` is "Default"; `{"custom": {"_0": {"directoryBasename": ..}}}` is that name.
fn profile_name(profile: Option<&Value>) -> String {
    profile
        .and_then(|p| p.get("custom"))
        .and_then(|c| c.get("_0").unwrap_or(c).get("directoryBasename"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("Default")
        .to_string()
}

fn space_icon(space: &Value) -> String {
    let icon_type = space.get("customInfo").and_then(|c| c.get("iconType"));
    let emoji_v2 = icon_type
        .and_then(|t| t.get("emoji_v2"))
        .and_then(Value::as_str);
    if let Some(s) = emoji_v2.filter(|s| !s.is_empty()) {
        return s.to_string();
    }
    icon_type
        .and_then(|t| t.get("emoji"))
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .and_then(char::from_u32)
        .filter(|c| !c.is_control())
        .map(String::from)
        .unwrap_or_default()
}

/// The parts of the sidebar we read, with ids resolved.
struct Data<'a> {
    spaces: Vec<&'a Value>,
    items: HashMap<&'a str, &'a Value>,
    /// Profile name -> id of the favorites container.
    top_apps: HashMap<String, &'a str>,
}

impl<'a> Data<'a> {
    fn collect(root: &'a Value) -> Self {
        let containers = root
            .get("sidebar")
            .and_then(|s| s.get("containers"))
            .or(Some(root));
        let list: Vec<&Value> = match containers {
            Some(Value::Array(a)) => a.iter().collect(),
            Some(v) => vec![v],
            None => Vec::new(),
        };
        let is_data = |v: &Value| v.get("spaces").is_some() || v.get("items").is_some();
        // The real data follows a `global` key; the other entries describe windows (`standalone`,
        // `littleBrowser`) and hold only scratch containers. Without a `global` key, use everything.
        let mut chosen: Vec<&Value> = Vec::new();
        for w in list.windows(2) {
            if w[0].get("global").is_some() && is_data(w[1]) {
                chosen.push(w[1]);
            }
        }
        if chosen.is_empty() {
            chosen = list.into_iter().filter(|v| is_data(v)).collect();
        }

        let mut data = Data {
            spaces: Vec::new(),
            items: HashMap::new(),
            top_apps: HashMap::new(),
        };
        for c in chosen {
            data.spaces.extend(
                objects(c.get("spaces"))
                    .into_iter()
                    .filter(|s| s.is_object()),
            );
            for item in objects(c.get("items")) {
                if let Some(id) = item.get("id").and_then(Value::as_str) {
                    data.items.insert(id, item);
                }
            }
            for (profile, id) in pairs(c.get("topAppsContainerIDs")) {
                if let Some(id) = id.as_str() {
                    data.top_apps
                        .entry(profile_name(Some(&profile)))
                        .or_insert(id);
                }
            }
        }
        data
    }

    fn item(&self, id: &str) -> Option<&'a Value> {
        self.items.get(id).copied()
    }

    fn children(&self, id: &str) -> Vec<&'a Value> {
        let Some(item) = self.item(id) else {
            return Vec::new();
        };
        item.get("childrenIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|c| self.item(c))
            .collect()
    }

    /// Pinned content of a container: tabs and (nested) folders, in Arc's order.
    fn nodes(
        &self,
        container: &str,
        pinned: bool,
        depth: usize,
        folders: &mut usize,
        tabs: &mut usize,
    ) -> Vec<Node> {
        if depth > MAX_DEPTH {
            return Vec::new();
        }
        let mut out = Vec::new();
        for item in self.children(container) {
            let data = item.get("data");
            if data.and_then(|d| d.get("tab")).is_some() {
                if let Some(tab) = tab_from(item, pinned) {
                    *tabs += 1;
                    out.push(Node::Tab(tab));
                }
            } else if let Some(list) = data.and_then(|d| d.get("list")) {
                // Live folders (e.g. GitHub) are filled in by Arc from a remote source.
                if list.get("automaticLiveFolderData").is_some() {
                    continue;
                }
                let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
                *folders += 1;
                let children = self.nodes(id, pinned, depth + 1, folders, tabs);
                out.push(Node::Folder(Folder {
                    id: new_id(),
                    name: item
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    collapsed: false,
                    children,
                }));
            }
        }
        out
    }

    /// Today has only tabs: folders are flattened away.
    fn today_tabs(&self, container: &str) -> Vec<Tab> {
        let (mut folders, mut tabs) = (0, 0);
        let mut out = Vec::new();
        fn flatten(nodes: Vec<Node>, out: &mut Vec<Tab>) {
            for n in nodes {
                match n {
                    Node::Tab(t) => out.push(t),
                    Node::Folder(f) => flatten(f.children, out),
                }
            }
        }
        flatten(
            self.nodes(container, false, 0, &mut folders, &mut tabs),
            &mut out,
        );
        out
    }

    fn favorites(&self, container: &str) -> Vec<Tab> {
        self.children(container)
            .into_iter()
            .filter_map(|i| tab_from(i, true))
            .collect()
    }
}

/// A tab item -> `Tab`. `None` when it has no saved URL.
fn tab_from(item: &Value, pinned: bool) -> Option<Tab> {
    let data = item.get("data")?.get("tab")?;
    let url = data.get("savedURL")?.as_str().filter(|u| !u.is_empty())?;
    let mut tab = Tab::new(url);
    tab.title = data
        .get("savedTitle")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    tab.custom_title = item
        .get("title")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty() && *t != tab.title)
        .map(String::from);
    if pinned {
        tab.home_url = Some(url.to_string());
    }
    if let Some(t) = data.get("timeLastActiveAt").and_then(Value::as_f64) {
        tab.last_active_ms = ((t + APPLE_EPOCH_OFFSET_S) * 1000.0).max(0.0) as u64;
    }
    Some(tab)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tab(id: &str, parent: &str, url: &str, title: &str) -> Value {
        json!({"id": id, "parentID": parent, "childrenIds": [], "title": null,
               "data": {"tab": {"savedURL": url, "savedTitle": title, "timeLastActiveAt": 0.0}}})
    }
    fn list(id: &str, parent: &str, name: &str, kids: &[&str]) -> Value {
        json!({"id": id, "parentID": parent, "childrenIds": kids, "title": name, "data": {"list": {}}})
    }
    fn container(id: &str, kind: Value, kids: &[&str]) -> Value {
        json!({"id": id, "parentID": null, "childrenIds": kids,
               "title": null, "data": {"itemContainer": {"containerType": kind}}})
    }
    /// Flattens `[(id, obj), ...]` into Swift's alternating encoding.
    fn alt(entries: Vec<(String, Value)>) -> Value {
        Value::Array(
            entries
                .into_iter()
                .flat_map(|(k, v)| [Value::String(k), v])
                .collect(),
        )
    }
    fn with_id(v: Value) -> (String, Value) {
        (v["id"].as_str().unwrap().to_string(), v)
    }

    /// Two spaces on the default profile (emoji "🏠", and one untitled), one on "Profile 1".
    fn fixture() -> String {
        let items = vec![
            // Space A pinned: tab, folder f1 { tab, folder f2 { tab } }
            container("a-pin", json!({"spaceItems": {"_0": "sa"}}), &["pa", "f1"]),
            tab("pa", "a-pin", "https://example.com/a", "A"),
            list("f1", "a-pin", "Work", &["pb", "f2"]),
            tab("pb", "f1", "https://example.com/b", "B"),
            list("f2", "f1", "Deep", &["pc"]),
            tab("pc", "f2", "https://example.com/c", "C"),
            // Space A today: a tab, a folder with a tab, and a tab without a URL
            container(
                "a-today",
                json!({"spaceItems": {"_0": "sa"}}),
                &["t1", "tf", "t0"],
            ),
            tab("t1", "a-today", "https://example.com/t1", "T1"),
            list("tf", "a-today", "Loose", &["t2"]),
            tab("t2", "tf", "https://example.com/t2", "T2"),
            json!({"id": "t0", "parentID": "a-today", "childrenIds": [], "title": "x",
                   "data": {"tab": {"savedURL": null, "savedTitle": "no url"}}}),
            // Space B (same profile) has nothing of its own
            container("b-pin", json!({"spaceItems": {"_0": "sb"}}), &[]),
            container("b-today", json!({"spaceItems": {"_0": "sb"}}), &[]),
            // Space C (custom profile): a renamed pinned tab
            container("c-pin", json!({"spaceItems": {"_0": "sc"}}), &["pr"]),
            json!({"id": "pr", "parentID": "c-pin", "childrenIds": [], "title": "My name",
                   "data": {"tab": {"savedURL": "https://example.com/r", "savedTitle": "R"}}}),
            container("c-today", json!({"spaceItems": {"_0": "sc"}}), &[]),
            // Favorites per profile
            container("fav-default", json!({"topApps": {"_0": {}}}), &["fa", "fb"]),
            tab("fa", "fav-default", "https://example.com/fa", "FA"),
            tab("fb", "fav-default", "https://example.com/fb", "FB"),
            container("fav-p1", json!({"topApps": {"_0": {}}}), &["fc"]),
            tab("fc", "fav-p1", "https://example.com/fc", "FC"),
        ];
        let spaces = vec![
            (
                "sa".to_string(),
                json!({"id": "sa", "title": "Home",
                "customInfo": {"iconType": {"emoji_v2": "🏠"}},
                "containerIDs": ["unpinned", "a-today", "pinned", "a-pin"],
                "profile": {"default": true}}),
            ),
            (
                "sb".to_string(),
                json!({"id": "sb", "title": null,
                "customInfo": {"iconType": {"icon": "star"}},
                "containerIDs": ["pinned", "b-pin", "unpinned", "b-today"],
                "profile": {"default": true}}),
            ),
            (
                "sc".to_string(),
                json!({"id": "sc", "title": "Other",
                "customInfo": {"iconType": {}},
                "containerIDs": ["unpinned", "c-today", "pinned", "c-pin"],
                "profile": {"custom": {"_0": {"directoryBasename": "Profile 1"}}}}),
            ),
        ];
        let top = json!([{"default": true}, "fav-default",
                         {"custom": {"_0": {"directoryBasename": "Profile 1"}}}, "fav-p1"]);
        json!({"sidebar": {"containers": [
            {"standalone": {"_0": "w"}},
            {"items": ["junk", {"id": "junk"}], "spaces": [], "topAppsContainerIDs": []},
            {"global": {}},
            {"spaces": alt(spaces), "topAppsContainerIDs": top,
             "items": alt(items.into_iter().map(with_id).collect())},
        ]}, "version": 1})
        .to_string()
    }

    fn pinned_names(nodes: &[Node]) -> Vec<String> {
        nodes
            .iter()
            .map(|n| match n {
                Node::Tab(t) => t.title.clone(),
                Node::Folder(f) => format!("{}[{}]", f.name, pinned_names(&f.children).join(",")),
            })
            .collect()
    }

    #[test]
    fn folders_nest_two_levels_in_order() {
        let arc = import_arc_sidebar(&fixture()).unwrap();
        let a = &arc.state.spaces[0];
        assert_eq!(pinned_names(&a.pinned), ["A", "Work[B,Deep[C]]"]);
    }

    #[test]
    fn pinned_tabs_keep_home_and_today_does_not() {
        let arc = import_arc_sidebar(&fixture()).unwrap();
        let a = &arc.state.spaces[0];
        let pinned: Vec<&Tab> = a
            .tabs()
            .into_iter()
            .filter(|t| a.section_of(&t.id) == Some(Section::Pinned))
            .collect();
        assert_eq!(pinned.len(), 3);
        for t in pinned {
            assert_eq!(t.home_url.as_deref(), Some(t.url.as_str()));
        }
        // Folders are flattened, URL-less tabs are skipped.
        let today: Vec<&str> = a.today.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(today, ["T1", "T2"]);
        assert!(a.today.iter().all(|t| t.home_url.is_none()));
    }

    #[test]
    fn favorites_are_copied_to_spaces_sharing_a_profile() {
        let arc = import_arc_sidebar(&fixture()).unwrap();
        let s = &arc.state.spaces;
        let urls = |sp: &Space| {
            sp.favorites
                .iter()
                .map(|t| t.url.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            urls(&s[0]),
            ["https://example.com/fa", "https://example.com/fb"]
        );
        assert_eq!(urls(&s[0]), urls(&s[1]));
        assert_eq!(urls(&s[2]), ["https://example.com/fc"]);
        assert!(
            s[0].favorites
                .iter()
                .all(|t| t.home_url.as_deref() == Some(t.url.as_str()))
        );
        // Copies are independent tabs.
        assert_ne!(s[0].favorites[0].id, s[1].favorites[0].id);
    }

    #[test]
    fn two_profiles_are_shared_like_arcs() {
        let arc = import_arc_sidebar(&fixture()).unwrap();
        let st = &arc.state;
        let names: Vec<&str> = st.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Default", "Profile 1"]);
        assert_eq!(st.spaces[0].profile_id, st.spaces[1].profile_id);
        assert_ne!(st.spaces[0].profile_id, st.spaces[2].profile_id);
        assert_eq!(st.active_space, st.spaces[0].id);
    }

    #[test]
    fn space_names_icons_and_colors() {
        let arc = import_arc_sidebar(&fixture()).unwrap();
        let s = &arc.state.spaces;
        assert_eq!((s[0].name.as_str(), s[0].icon.as_str()), ("Home", "🏠"));
        assert_eq!((s[1].name.as_str(), s[1].icon.as_str()), ("Space 2", ""));
        assert_eq!(s[2].color, SPACE_COLORS[2]);
    }

    #[test]
    fn alternating_arrays_decode_and_titles_map() {
        let arc = import_arc_sidebar(&fixture()).unwrap();
        let c = &arc.state.spaces[2];
        let Node::Tab(t) = &c.pinned[0] else {
            panic!("expected a tab")
        };
        assert_eq!(t.title, "R");
        assert_eq!(t.custom_title.as_deref(), Some("My name"));
        assert_eq!(
            arc.summary,
            "3 spaces, 2 folders, 4 pinned tabs, 3 favorites"
        );
    }

    #[test]
    fn live_folders_and_unknown_items_are_skipped() {
        let json = json!({"sidebar": {"containers": [{"global": {}}, {
        "spaces": ["s", {"id": "s", "containerIDs": ["pinned", "p"], "profile": {"default": true}}],
        "topAppsContainerIDs": [],
        "items": [
            "p", container("p", json!({"spaceItems": {"_0": "s"}}), &["live", "weird", "ok"]),
            "live", {"id": "live", "childrenIds": [], "title": "PRs",
                     "data": {"list": {"automaticLiveFolderData": {}}}},
            "weird", {"id": "weird", "childrenIds": [], "data": {"somethingNew": {}}},
            "ok", tab("ok", "p", "https://example.com/ok", "OK"),
        ]}]}})
        .to_string();
        let arc = import_arc_sidebar(&json).unwrap();
        assert_eq!(pinned_names(&arc.state.spaces[0].pinned), ["OK"]);
    }

    #[test]
    fn no_spaces_is_an_error() {
        assert!(matches!(
            import_arc_sidebar("{}"),
            Err(ImportError::NoSpaces)
        ));
        assert!(matches!(
            import_arc_sidebar("nope"),
            Err(ImportError::Json(_))
        ));
    }

    /// Runs on a real Arc file and prints only counts:
    /// `ARC_SIDEBAR_JSON=/path/to/StorableSidebar.json cargo test -p burrow-core -- --ignored --nocapture real_arc`
    #[test]
    #[ignore]
    fn real_arc_file_summary() {
        let path = std::env::var("ARC_SIDEBAR_JSON").expect("set ARC_SIDEBAR_JSON");
        let json = std::fs::read_to_string(path).unwrap();
        let arc = import_arc_sidebar(&json).unwrap();
        println!("{}", arc.summary);
    }
}
