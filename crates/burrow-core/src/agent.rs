//! The agent interface (D-018): the tools an AI agent can call to drive Burrow, and the
//! browser-independent answers (listings of spaces and tabs). The app runs the tools that need
//! a live page (reading, screenshots, input) itself.

use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    commands::{Command, Effect, Location},
    model::*,
};

/// One tool: name, what it does, and a JSON Schema for its arguments.
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Value,
}

fn tab_id() -> Value {
    json!({ "type": "string", "description": "A tab id from list_tabs or open_tab." })
}

fn section() -> Value {
    json!({
        "type": "string",
        "enum": ["today", "pinned", "favorites"],
        "description": "today = unpinned tabs; pinned = tabs with a fixed place (can be in folders); favorites = the app tiles at the top."
    })
}

fn object(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "list_spaces",
        description: "List every space (each has its own tabs, folders, favorites and usually its own logins) and which one the user is looking at.",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "get_space",
        description: "The sidebar of one space as a tree: favorites, pinned tabs and folders, and today's tabs.",
        schema: || object(json!({ "space_id": { "type": "string", "description": "Default: the active space." } }), &[]),
    },
    Tool {
        name: "list_tabs",
        description: "List tabs with their ids, titles, URLs, space, section and folder. `loaded` tells whether the tab has a running page.",
        schema: || object(json!({ "space_id": { "type": "string", "description": "Only this space. Default: all spaces." } }), &[]),
    },
    Tool {
        name: "open_tab",
        description: "Open a URL (or search words) in a new tab. By default it opens in the background at the top of Today in the active space and waits for the page to load. Returns the new tab.",
        schema: || {
            object(
                json!({
                    "url": { "type": "string", "description": "A URL, or words to search for." },
                    "space_id": { "type": "string", "description": "Default: the active space." },
                    "section": section(),
                    "folder_id": { "type": "string", "description": "Put it in this pinned folder (implies section=pinned)." },
                    "index": { "type": "integer", "minimum": 0, "description": "Position in the list. Default: top of Today, end of Pinned/Favorites." },
                    "focus": { "type": "boolean", "description": "Show the tab to the user (switches space if needed). Default false." },
                    "wait": { "type": "boolean", "description": "Wait until the page has loaded. Default true." }
                }),
                &["url"],
            )
        },
    },
    Tool {
        name: "close_tab",
        description: "Close a tab. Today tabs go away (they can be reopened from the archive). Pinned tabs and favorites only unload and go back to their home URL, unless delete is true.",
        schema: || {
            object(
                json!({
                    "tab_id": tab_id(),
                    "delete": { "type": "boolean", "description": "Remove a pinned tab or favorite from the sidebar for good. Default false." }
                }),
                &["tab_id"],
            )
        },
    },
    Tool {
        name: "focus_tab",
        description: "Show a tab to the user, switching to its space if needed.",
        schema: || object(json!({ "tab_id": tab_id() }), &["tab_id"]),
    },
    Tool {
        name: "navigate",
        description: "Load a URL (or search words) in an existing tab, or go back/forward/reload. Waits for the page to load.",
        schema: || {
            object(
                json!({
                    "tab_id": tab_id(),
                    "url": { "type": "string", "description": "A URL or search words. Omit when using action." },
                    "action": { "type": "string", "enum": ["back", "forward", "reload", "stop", "home"], "description": "home = a pinned tab's or favorite's home URL." },
                    "wait": { "type": "boolean", "description": "Default true." }
                }),
                &["tab_id"],
            )
        },
    },
    Tool {
        name: "move_tab",
        description: "Move a tab or a folder: reorder it, pin or unpin it, put it in a folder, make it a favorite, or move it to another space. Moving a tab to a space with different logins reloads it there.",
        schema: || {
            object(
                json!({
                    "node_id": { "type": "string", "description": "A tab id or folder id." },
                    "space_id": { "type": "string", "description": "Default: the space it is in now." },
                    "section": section(),
                    "folder_id": { "type": "string", "description": "Into this pinned folder (implies section=pinned)." },
                    "index": { "type": "integer", "minimum": 0, "description": "Position in the target list. Default: the end." }
                }),
                &["node_id"],
            )
        },
    },
    Tool {
        name: "rename_tab",
        description: "Give a tab a custom name in the sidebar, or clear it with null.",
        schema: || object(json!({ "tab_id": tab_id(), "title": { "type": ["string", "null"] } }), &["tab_id", "title"]),
    },
    Tool {
        name: "create_folder",
        description: "Create a folder in a space's pinned section, optionally inside another folder. Returns its id.",
        schema: || {
            object(
                json!({
                    "name": { "type": "string" },
                    "space_id": { "type": "string", "description": "Default: the active space." },
                    "parent_id": { "type": "string", "description": "A folder to nest it in." }
                }),
                &["name"],
            )
        },
    },
    Tool {
        name: "update_folder",
        description: "Rename, collapse/expand, or delete a folder. Deleting keeps its tabs: they move up to where the folder was.",
        schema: || {
            object(
                json!({
                    "folder_id": { "type": "string" },
                    "name": { "type": "string" },
                    "collapsed": { "type": "boolean" },
                    "delete": { "type": "boolean" }
                }),
                &["folder_id"],
            )
        },
    },
    Tool {
        name: "create_space",
        description: "Create a space. By default it gets its own profile (separate cookies and logins); pass share_logins_with to use another space's logins instead. Returns its id. Does not switch to it unless focus is true.",
        schema: || {
            object(
                json!({
                    "name": { "type": "string" },
                    "icon": { "type": "string", "description": "An emoji, or empty for a dot." },
                    "color": { "type": "string", "description": "Sidebar tint as #rrggbb." },
                    "share_logins_with": { "type": "string", "description": "A space id whose profile (logins) to share." },
                    "focus": { "type": "boolean", "description": "Switch to the new space. Default false." }
                }),
                &["name"],
            )
        },
    },
    Tool {
        name: "update_space",
        description: "Rename a space or change its icon or color.",
        schema: || {
            object(
                json!({
                    "space_id": { "type": "string" },
                    "name": { "type": "string" },
                    "icon": { "type": "string" },
                    "color": { "type": "string", "description": "#rrggbb" }
                }),
                &["space_id"],
            )
        },
    },
    Tool {
        name: "delete_space",
        description: "Delete a space and all its tabs and folders. Its logins are kept. The last space can't be deleted.",
        schema: || object(json!({ "space_id": { "type": "string" } }), &["space_id"]),
    },
    Tool {
        name: "switch_space",
        description: "Show a space to the user.",
        schema: || object(json!({ "space_id": { "type": "string" } }), &["space_id"]),
    },
    Tool {
        name: "read_page",
        description: "Read a tab's page, loading it first if needed: its visible text, its HTML, or a numbered list of links, buttons and form fields (with CSS selectors to use with click/type_text). Works on any tab, not just the shown one.",
        schema: || {
            object(
                json!({
                    "tab_id": tab_id(),
                    "format": { "type": "string", "enum": ["text", "html", "elements"], "description": "Default text." },
                    "max_chars": { "type": "integer", "minimum": 100, "description": "Truncate to this many characters. Default 20000." }
                }),
                &["tab_id"],
            )
        },
    },
    Tool {
        name: "screenshot",
        description: "Take a PNG screenshot of any tab (it doesn't have to be the shown one).",
        schema: || {
            object(
                json!({
                    "tab_id": tab_id(),
                    "full_page": { "type": "boolean", "description": "The whole scrollable page instead of the viewport. Default false." }
                }),
                &["tab_id"],
            )
        },
    },
    Tool {
        name: "click",
        description: "Click in a tab, at an element (CSS selector) or at viewport coordinates. Sends a real mouse click.",
        schema: || {
            object(
                json!({
                    "tab_id": tab_id(),
                    "selector": { "type": "string" },
                    "x": { "type": "number" },
                    "y": { "type": "number" }
                }),
                &["tab_id"],
            )
        },
    },
    Tool {
        name: "type_text",
        description: "Type text into a tab: into the element matching selector (focused first), or wherever the focus is. submit presses Enter afterwards.",
        schema: || {
            object(
                json!({
                    "tab_id": tab_id(),
                    "text": { "type": "string" },
                    "selector": { "type": "string" },
                    "clear": { "type": "boolean", "description": "Empty the field first. Default false." },
                    "submit": { "type": "boolean", "description": "Press Enter afterwards. Default false." }
                }),
                &["tab_id", "text"],
            )
        },
    },
    Tool {
        name: "press_key",
        description: "Press a key in a tab: Enter, Tab, Escape, Backspace, Delete, ArrowUp/Down/Left/Right, Home, End, PageUp, PageDown, Space, or a single character.",
        schema: || object(json!({ "tab_id": tab_id(), "key": { "type": "string" } }), &["tab_id", "key"]),
    },
    Tool {
        name: "wait_for",
        description: "Wait until a tab's page shows an element (selector) or some text.",
        schema: || {
            object(
                json!({
                    "tab_id": tab_id(),
                    "selector": { "type": "string" },
                    "text": { "type": "string" },
                    "timeout_ms": { "type": "integer", "minimum": 0, "description": "Default 10000." }
                }),
                &["tab_id"],
            )
        },
    },
    Tool {
        name: "eval_js",
        description: "Run JavaScript in a tab's page and return the result (promises are awaited; the value must be JSON-serializable).",
        schema: || object(json!({ "tab_id": tab_id(), "expression": { "type": "string" } }), &["tab_id", "expression"]),
    },
];

/// The tool list in MCP's `tools/list` shape.
pub fn tools_json() -> Value {
    Value::Array(
        TOOLS
            .iter()
            .map(|t| json!({ "name": t.name, "description": t.description, "inputSchema": (t.schema)() }))
            .collect(),
    )
}

/// Tools that look inside or act on a page. The owner can turn these off; tab and space tools
/// always work (D-019).
pub const PAGE_TOOLS: &[&str] = &["read_page", "screenshot", "click", "type_text", "press_key", "wait_for", "eval_js"];

pub fn is_page_tool(name: &str) -> bool {
    PAGE_TOOLS.contains(&name)
}

pub fn tool(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

/// A tab as an agent sees it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TabInfo {
    pub id: Id,
    pub title: String,
    pub url: String,
    pub space_id: Id,
    pub space: String,
    pub section: Section,
    /// The folder names from the top of Pinned down to the tab's folder.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_url: Option<String>,
    /// The tab the user sees (the active tab of the active space).
    pub shown: bool,
}

/// A tab, its section and the folders (id, name) above it.
type Placed = (Tab, Section, Vec<(Id, String)>);

/// Every tab in sidebar order, optionally only those of one space.
pub fn list_tabs(state: &State, space_id: Option<&str>) -> Vec<TabInfo> {
    fn walk(nodes: &[Node], path: &mut Vec<(Id, String)>, out: &mut Vec<Placed>) {
        for n in nodes {
            match n {
                Node::Tab(t) => out.push((t.clone(), Section::Pinned, path.clone())),
                Node::Folder(f) => {
                    path.push((f.id.clone(), f.name.clone()));
                    walk(&f.children, path, out);
                    path.pop();
                }
            }
        }
    }
    let shown = state.active_tab().map(|t| t.id.clone());
    let mut out = vec![];
    for space in state.spaces.iter().filter(|s| space_id.is_none_or(|id| s.id == id)) {
        let mut tabs: Vec<Placed> =
            space.favorites.iter().map(|t| (t.clone(), Section::Favorites, vec![])).collect();
        walk(&space.pinned, &mut vec![], &mut tabs);
        tabs.extend(space.today.iter().map(|t| (t.clone(), Section::Today, vec![])));
        out.extend(tabs.into_iter().map(|(t, section, path)| TabInfo {
            title: t.display_title().to_string(),
            shown: shown.as_deref() == Some(&t.id),
            id: t.id,
            url: t.url,
            space_id: space.id.clone(),
            space: space.name.clone(),
            section,
            folder_id: path.last().map(|(id, _)| id.clone()),
            folders: path.into_iter().map(|(_, name)| name).collect(),
            home_url: t.home_url,
        }));
    }
    out
}

pub fn list_spaces(state: &State) -> Value {
    let spaces: Vec<Value> = state
        .spaces
        .iter()
        .map(|s| {
            let shares = state.spaces.iter().filter(|o| o.profile_id == s.profile_id && o.id != s.id).map(|o| o.id.clone());
            json!({
                "id": s.id,
                "name": s.name,
                "icon": s.icon,
                "color": s.color,
                "active": s.id == state.active_space,
                "profile_id": s.profile_id,
                "shares_logins_with": shares.collect::<Vec<_>>(),
                "favorites": s.favorites.len(),
                "pinned": s.tabs().len() - s.favorites.len() - s.today.len(),
                "today": s.today.len(),
            })
        })
        .collect();
    json!({ "spaces": spaces, "active_space": state.active_space })
}

/// One space's sidebar as a tree of folders and tabs.
pub fn space_tree(space: &Space) -> Value {
    fn tab(t: &Tab) -> Value {
        json!({ "id": t.id, "title": t.display_title(), "url": t.url, "home_url": t.home_url })
    }
    fn nodes(ns: &[Node]) -> Vec<Value> {
        ns.iter()
            .map(|n| match n {
                Node::Tab(t) => tab(t),
                Node::Folder(f) => json!({ "folder_id": f.id, "name": f.name, "collapsed": f.collapsed, "children": nodes(&f.children) }),
            })
            .collect()
    }
    json!({
        "id": space.id,
        "name": space.name,
        "favorites": space.favorites.iter().map(tab).collect::<Vec<_>>(),
        "pinned": nodes(&space.pinned),
        "today": space.today.iter().map(tab).collect::<Vec<_>>(),
        "active_tab": space.active_tab,
    })
}

/// What a model tool did: its answer, and what the app must do to its browsers.
pub struct Outcome {
    pub result: Value,
    pub effects: Vec<Effect>,
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn need<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    str_arg(args, key).ok_or_else(|| format!("missing argument `{key}`"))
}

fn bool_arg(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

fn parse_section(s: &str) -> Result<Section, String> {
    serde_json::from_value(Value::String(s.into())).map_err(|_| format!("unknown section `{s}` (use today, pinned or favorites)"))
}

/// Where `section`/`folder_id`/`space_id`/`index` say to put something. `current` is the section
/// used when neither `section` nor `folder_id` is given.
pub fn location(args: &Value, current: Option<Section>) -> Result<Option<Location>, String> {
    let folder = str_arg(args, "folder_id").map(String::from);
    let section = match (str_arg(args, "section"), &folder) {
        (Some(s), _) => Some(parse_section(s)?),
        (None, Some(_)) => Some(Section::Pinned),
        (None, None) => current,
    };
    let index = args.get("index").and_then(Value::as_u64).map(|i| i as usize);
    let space_id = str_arg(args, "space_id").map(String::from);
    Ok(match section {
        Some(section) => Some(Location { space_id, section, parent: folder, index }),
        None if space_id.is_some() => Some(Location { space_id, section: Section::Today, parent: None, index: Some(0) }),
        None => None,
    })
}

/// Runs a tool that only needs the model. `None` means the tool needs the app (a live page).
pub fn run_model_tool(state: &mut State, name: &str, args: &Value) -> Option<Result<Outcome, String>> {
    let done = |result: Value, effects: Vec<Effect>| Ok(Outcome { result, effects });
    let apply = |state: &mut State, cmd: Command| state.apply(cmd).map_err(|e| e.to_string());
    let run = || -> Result<Outcome, String> {
        match name {
            "list_spaces" => done(list_spaces(state), vec![]),
            "get_space" => {
                let id = str_arg(args, "space_id").map(String::from).unwrap_or_else(|| state.active_space.clone());
                let space = state.space(&id).ok_or_else(|| format!("no space {id}"))?;
                done(space_tree(space), vec![])
            }
            "list_tabs" => done(json!(list_tabs(state, str_arg(args, "space_id"))), vec![]),
            "close_tab" => {
                let tab_id = need(args, "tab_id")?.to_string();
                let cmd = if bool_arg(args, "delete") == Some(true) {
                    Command::DeleteTab { tab_id }
                } else {
                    Command::CloseTab { tab_id }
                };
                done(json!({ "closed": true }), apply(state, cmd)?)
            }
            "focus_tab" => {
                let fx = apply(state, Command::SelectTab { tab_id: need(args, "tab_id")?.into() })?;
                done(json!({ "shown": true }), fx)
            }
            "move_tab" => {
                let node_id = need(args, "node_id")?.to_string();
                let current = state.space_of(&node_id).and_then(|s| s.section_of(&node_id));
                let to = location(args, current)?.ok_or_else(|| format!("no tab or folder {node_id}"))?;
                let fx = apply(state, Command::MoveNode { node_id: node_id.clone(), to })?;
                let section = state.space_of(&node_id).and_then(|s| s.section_of(&node_id));
                done(json!({ "moved": true, "space_id": state.space_of(&node_id).map(|s| &s.id), "section": section }), fx)
            }
            "rename_tab" => {
                let title = str_arg(args, "title").map(String::from);
                let fx = apply(state, Command::RenameTab { tab_id: need(args, "tab_id")?.into(), title })?;
                done(json!({ "renamed": true }), fx)
            }
            "create_folder" => {
                let space_id = str_arg(args, "space_id").map(String::from).unwrap_or_else(|| state.active_space.clone());
                let parent = str_arg(args, "parent_id").map(String::from);
                let before = folder_ids(state);
                let fx = apply(state, Command::NewFolder { space_id: Some(space_id), parent, name: need(args, "name")?.into() })?;
                let id = folder_ids(state).into_iter().find(|f| !before.contains(f));
                done(json!({ "folder_id": id }), fx)
            }
            "update_folder" => {
                let folder_id = need(args, "folder_id")?.to_string();
                let mut fx = vec![];
                if let Some(name) = str_arg(args, "name") {
                    fx.extend(apply(state, Command::RenameFolder { folder_id: folder_id.clone(), name: name.into() })?);
                }
                if let Some(c) = bool_arg(args, "collapsed") {
                    let now = state.spaces.iter().find_map(|s| find_folder(&s.pinned, &folder_id)).map(|f| f.collapsed);
                    if now.is_some_and(|now| now != c) {
                        fx.extend(apply(state, Command::ToggleFolder { folder_id: folder_id.clone() })?);
                    }
                }
                if bool_arg(args, "delete") == Some(true) {
                    fx.extend(apply(state, Command::DeleteFolder { folder_id: folder_id.clone() })?);
                    return done(json!({ "deleted": true }), fx);
                }
                if !state.spaces.iter().any(|s| find_folder(&s.pinned, &folder_id).is_some()) {
                    return Err(format!("no folder {folder_id}"));
                }
                done(json!({ "updated": true }), fx)
            }
            "create_space" => {
                let profile_id = match str_arg(args, "share_logins_with") {
                    Some(id) => Some(state.space(id).ok_or_else(|| format!("no space {id}"))?.profile_id.clone()),
                    None => None,
                };
                let previous = state.active_space.clone();
                let name = need(args, "name")?.to_string();
                let icon = str_arg(args, "icon").unwrap_or_default().to_string();
                let mut fx = apply(state, Command::NewSpace { name, icon, profile_id })?;
                let id = state.spaces.last().map(|s| s.id.clone()).unwrap_or_default();
                if let Some(color) = str_arg(args, "color") {
                    let cmd = Command::UpdateSpace { space_id: id.clone(), name: None, icon: None, color: Some(color.into()) };
                    fx.extend(apply(state, cmd)?);
                }
                if bool_arg(args, "focus") != Some(true) {
                    state.active_space = previous;
                }
                done(json!({ "space_id": id }), fx)
            }
            "update_space" => {
                let cmd = Command::UpdateSpace {
                    space_id: need(args, "space_id")?.into(),
                    name: str_arg(args, "name").map(String::from),
                    icon: str_arg(args, "icon").map(String::from),
                    color: str_arg(args, "color").map(String::from),
                };
                done(json!({ "updated": true }), apply(state, cmd)?)
            }
            "delete_space" => done(json!({ "deleted": true }), apply(state, Command::DeleteSpace { space_id: need(args, "space_id")?.into() })?),
            "switch_space" => done(json!({ "shown": true }), apply(state, Command::SwitchSpace { space_id: need(args, "space_id")?.into() })?),
            _ => unreachable!(),
        }
    };
    const MODEL_TOOLS: &[&str] = &[
        "list_spaces", "get_space", "list_tabs", "close_tab", "focus_tab", "move_tab", "rename_tab", "create_folder",
        "update_folder", "create_space", "update_space", "delete_space", "switch_space",
    ];
    MODEL_TOOLS.contains(&name).then(run)
}

fn folder_ids(state: &State) -> Vec<Id> {
    fn walk(nodes: &[Node], out: &mut Vec<Id>) {
        for n in nodes {
            if let Node::Folder(f) = n {
                out.push(f.id.clone());
                walk(&f.children, out);
            }
        }
    }
    let mut out = vec![];
    for s in &state.spaces {
        walk(&s.pinned, &mut out);
    }
    out
}

fn find_folder<'a>(nodes: &'a [Node], id: &str) -> Option<&'a Folder> {
    nodes.iter().find_map(|n| match n {
        Node::Folder(f) if f.id == id => Some(f),
        Node::Folder(f) => find_folder(&f.children, id),
        Node::Tab(_) => None,
    })
}
