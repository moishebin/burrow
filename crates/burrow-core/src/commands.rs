//! Everything the UI can ask the model to do, and what the app must then do to browsers.

use serde::{Deserialize, Serialize};

use crate::model::*;

/// A request from the sidebar (sent as JSON, tagged by `cmd`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Open a new tab at the top of Today in the active space and show it.
    NewTab {
        url: String,
    },
    SelectTab {
        tab_id: Id,
    },
    /// Today tabs are removed. Pinned tabs and favorites are unloaded and return home.
    CloseTab {
        tab_id: Id,
    },
    /// Removes a tab for good, wherever it is (pinned tabs and favorites included).
    DeleteTab {
        tab_id: Id,
    },
    /// Navigate a pinned tab or favorite back to its home URL.
    ResetTab {
        tab_id: Id,
    },
    RenameTab {
        tab_id: Id,
        title: Option<String>,
    },
    /// Moves a tab or folder. Moving into Pinned/Favorites pins it; into Today unpins it.
    MoveNode {
        node_id: Id,
        to: Location,
    },
    /// Pin the tab as a favorite (end of the grid).
    AddFavorite {
        tab_id: Id,
    },
    NewFolder {
        space_id: Option<Id>,
        parent: Option<Id>,
        name: String,
    },
    RenameFolder {
        folder_id: Id,
        name: String,
    },
    ToggleFolder {
        folder_id: Id,
    },
    /// Deletes the folder. Its children move up into the folder's parent.
    DeleteFolder {
        folder_id: Id,
    },
    SwitchSpace {
        space_id: Id,
    },
    /// `profile_id: None` creates a fresh profile (separate logins).
    NewSpace {
        name: String,
        icon: String,
        profile_id: Option<Id>,
    },
    UpdateSpace {
        space_id: Id,
        name: Option<String>,
        icon: Option<String>,
        color: Option<String>,
    },
    DeleteSpace {
        space_id: Id,
    },
    /// Close every Today tab in the active space (Arc's "clear").
    ClearToday,
    /// Reopen the most recently closed Today tab (Ctrl+Shift+T), in the space it came from.
    ReopenClosed,
    /// Reopen one entry of the archive (0 = newest), in the space it came from.
    ReopenArchived { index: usize },
}

/// Where to put a node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    /// Defaults to the space the node is in now.
    #[serde(default)]
    pub space_id: Option<Id>,
    pub section: Section,
    /// A folder id. Only meaningful in `Pinned`.
    #[serde(default)]
    pub parent: Option<Id>,
    /// Position in the target list. `None` appends.
    #[serde(default)]
    pub index: Option<usize>,
}

/// Something the app must do to its browsers after a command.
/// Which tab is visible is not an effect: the app always shows `State::active_tab()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum Effect {
    /// Destroy this tab's browser, if one exists.
    CloseBrowser { tab_id: Id },
    /// Load this URL in the tab's existing browser, if one exists.
    Navigate { tab_id: Id, url: String },
    /// The tab moved to another space. If it moved to a different profile its browser
    /// must be recreated, so the app just closes it.
    Rehome { tab_id: Id, profile_changed: bool },
    /// A new profile exists; the app may prepare its request context.
    ProfileCreated { profile_id: Id },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CommandError {
    #[error("no tab {0}")]
    NoTab(Id),
    #[error("no folder {0}")]
    NoFolder(Id),
    #[error("no space {0}")]
    NoSpace(Id),
    #[error("cannot move a folder into {0:?}")]
    FolderOutsidePinned(Section),
    #[error("cannot move a folder into itself")]
    FolderIntoItself,
    #[error("cannot delete the last space")]
    LastSpace,
}

pub type CommandResult = Result<Vec<Effect>, CommandError>;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl State {
    pub fn apply(&mut self, cmd: Command) -> CommandResult {
        match cmd {
            Command::NewTab { url } => {
                let mut tab = Tab::new(url);
                tab.last_active_ms = now_ms();
                let space = self.active_space_mut();
                space.active_tab = Some(tab.id.clone());
                space.today.insert(0, tab);
                Ok(vec![])
            }
            Command::SelectTab { tab_id } => {
                let space_id = self
                    .space_of(&tab_id)
                    .ok_or_else(|| CommandError::NoTab(tab_id.clone()))?
                    .id
                    .clone();
                self.active_space = space_id;
                let now = now_ms();
                let space = self.active_space_mut();
                // The tab we leave was in use until now; stamp it so auto-archive counts from here.
                if let Some(prev) = space.active_tab.clone() {
                    if let Some(t) = space.tab_mut(&prev) {
                        t.last_active_ms = now;
                    }
                }
                space.active_tab = Some(tab_id.clone());
                if let Some(t) = space.tab_mut(&tab_id) {
                    t.last_active_ms = now;
                }
                Ok(vec![])
            }
            Command::CloseTab { tab_id } => self.close_tab(&tab_id),
            Command::DeleteTab { tab_id } => {
                let space = self.space_of(&tab_id).filter(|s| s.tab(&tab_id).is_some());
                match space.and_then(|s| s.section_of(&tab_id)) {
                    None => Err(CommandError::NoTab(tab_id)),
                    Some(Section::Today) => self.close_tab(&tab_id),
                    Some(_) => {
                        // Unpinning first makes the close remove it instead of sending it home.
                        let to = Location { space_id: None, section: Section::Today, parent: None, index: Some(0) };
                        self.move_node(&tab_id, to)?;
                        self.close_tab(&tab_id)
                    }
                }
            }
            Command::ResetTab { tab_id } => {
                let tab = self
                    .tab_mut(&tab_id)
                    .ok_or_else(|| CommandError::NoTab(tab_id.clone()))?;
                match tab.home_url.clone() {
                    Some(home) if home != tab.url => {
                        tab.url = home.clone();
                        Ok(vec![Effect::Navigate { tab_id, url: home }])
                    }
                    _ => Ok(vec![]),
                }
            }
            Command::RenameTab { tab_id, title } => {
                let tab = self
                    .tab_mut(&tab_id)
                    .ok_or_else(|| CommandError::NoTab(tab_id.clone()))?;
                tab.custom_title = title.filter(|t| !t.trim().is_empty());
                Ok(vec![])
            }
            Command::MoveNode { node_id, to } => self.move_node(&node_id, to),
            Command::AddFavorite { tab_id } => self.move_node(
                &tab_id,
                Location {
                    space_id: None,
                    section: Section::Favorites,
                    parent: None,
                    index: None,
                },
            ),
            Command::NewFolder {
                space_id,
                parent,
                name,
            } => {
                let space_id = space_id.unwrap_or_else(|| self.active_space.clone());
                let space = self
                    .space_mut(&space_id)
                    .ok_or(CommandError::NoSpace(space_id))?;
                let folder = Node::Folder(Folder {
                    id: new_id(),
                    name,
                    collapsed: false,
                    children: Vec::new(),
                });
                match parent {
                    Some(pid) => space
                        .folder_mut(&pid)
                        .ok_or(CommandError::NoFolder(pid))?
                        .children
                        .push(folder),
                    None => space.pinned.push(folder),
                }
                Ok(vec![])
            }
            Command::RenameFolder { folder_id, name } => {
                self.folder_mut(&folder_id)?.name = name;
                Ok(vec![])
            }
            Command::ToggleFolder { folder_id } => {
                let f = self.folder_mut(&folder_id)?;
                f.collapsed = !f.collapsed;
                Ok(vec![])
            }
            Command::DeleteFolder { folder_id } => {
                for space in &mut self.spaces {
                    if let Some(parent) = parent_list_mut(&mut space.pinned, &folder_id) {
                        let i = parent.iter().position(|n| n.id() == &folder_id).unwrap();
                        let Node::Folder(f) = parent.remove(i) else {
                            unreachable!()
                        };
                        for (k, child) in f.children.into_iter().enumerate() {
                            parent.insert(i + k, child);
                        }
                        return Ok(vec![]);
                    }
                }
                Err(CommandError::NoFolder(folder_id))
            }
            Command::SwitchSpace { space_id } => {
                if self.space(&space_id).is_none() {
                    return Err(CommandError::NoSpace(space_id));
                }
                self.active_space = space_id;
                Ok(vec![])
            }
            Command::NewSpace {
                name,
                icon,
                profile_id,
            } => {
                let mut effects = vec![];
                let profile_id =
                    match profile_id.filter(|p| self.profiles.iter().any(|x| &x.id == p)) {
                        Some(p) => p,
                        None => {
                            let p = Profile {
                                id: new_id(),
                                name: name.clone(),
                            };
                            effects.push(Effect::ProfileCreated {
                                profile_id: p.id.clone(),
                            });
                            let id = p.id.clone();
                            self.profiles.push(p);
                            id
                        }
                    };
                let color = SPACE_COLORS[self.spaces.len() % SPACE_COLORS.len()];
                let space = Space::new(&name, &icon, color, &profile_id);
                self.active_space = space.id.clone();
                self.spaces.push(space);
                Ok(effects)
            }
            Command::UpdateSpace {
                space_id,
                name,
                icon,
                color,
            } => {
                let s = self
                    .space_mut(&space_id)
                    .ok_or(CommandError::NoSpace(space_id))?;
                if let Some(n) = name {
                    s.name = n;
                }
                if let Some(i) = icon {
                    s.icon = i;
                }
                if let Some(c) = color {
                    s.color = c;
                }
                Ok(vec![])
            }
            Command::DeleteSpace { space_id } => {
                if self.spaces.len() <= 1 {
                    return Err(CommandError::LastSpace);
                }
                let i = self
                    .spaces
                    .iter()
                    .position(|s| s.id == space_id)
                    .ok_or_else(|| CommandError::NoSpace(space_id.clone()))?;
                let space = self.spaces.remove(i);
                if self.active_space == space_id {
                    self.active_space = self.spaces[i.saturating_sub(1)].id.clone();
                }
                // Profiles are kept even when unused, so their logins survive (cheap, explicit).
                Ok(space
                    .tabs()
                    .into_iter()
                    .map(|t| Effect::CloseBrowser {
                        tab_id: t.id.clone(),
                    })
                    .collect())
            }
            Command::ClearToday => {
                let now = now_ms();
                let space_id = self.active_space.clone();
                let space = self.active_space_mut();
                let drained: Vec<Tab> = std::mem::take(&mut space.today);
                if let Some(a) = &space.active_tab
                    && space.tab(a).is_none() {
                        space.active_tab = None;
                    }
                for t in drained.iter().rev() {
                    self.archive_tab(t, &space_id, now);
                }
                Ok(drained
                    .into_iter()
                    .map(|t| Effect::CloseBrowser { tab_id: t.id })
                    .collect())
            }
            Command::ReopenClosed => self.apply(Command::ReopenArchived { index: 0 }),
            Command::ReopenArchived { index } => {
                if index >= self.archive.len() {
                    return Ok(vec![]);
                }
                let entry = self.archive.remove(index);
                if self.space(&entry.space_id).is_some() {
                    self.active_space = entry.space_id;
                }
                self.apply(Command::NewTab { url: entry.url })
            }
        }
    }

    /// Opens a tab at `to` (default: top of Today in the active space) and returns its id.
    /// Unlike `NewTab`, the tab only becomes the one shown when `focus` is set.
    pub fn open_tab(&mut self, url: String, to: Option<Location>, focus: bool) -> Result<(Id, Vec<Effect>), CommandError> {
        let space_id = to.as_ref().and_then(|l| l.space_id.clone()).unwrap_or_else(|| self.active_space.clone());
        let mut tab = Tab::new(url);
        tab.last_active_ms = now_ms();
        let id = tab.id.clone();
        self.space_mut(&space_id)
            .ok_or_else(|| CommandError::NoSpace(space_id.clone()))?
            .today
            .insert(0, tab);
        let mut effects = vec![];
        if let Some(to) = to.filter(|l| l.section != Section::Today || l.index.unwrap_or(0) != 0) {
            match self.move_node(&id, Location { space_id: Some(space_id.clone()), ..to }) {
                Ok(fx) => effects = fx,
                Err(e) => {
                    let space = self.space_mut(&space_id).unwrap();
                    space.today.retain(|t| t.id != id);
                    return Err(e);
                }
            }
        }
        if focus {
            self.apply(Command::SelectTab { tab_id: id.clone() })?;
        }
        Ok((id, effects))
    }

    /// Archives Today tabs that haven't been used for `settings.archive_after_hours`, except
    /// each space's active tab. Returns the browsers to close.
    pub fn archive_stale(&mut self, now_ms: u64) -> Vec<Effect> {
        let hours = self.settings.archive_after_hours;
        if hours == 0 {
            return vec![];
        }
        let cutoff = now_ms.saturating_sub(hours as u64 * 3_600_000);
        let mut stale: Vec<(Tab, Id)> = vec![];
        for space in &mut self.spaces {
            let active = space.active_tab.clone();
            let (old, keep): (Vec<Tab>, Vec<Tab>) = space
                .today
                .drain(..)
                .partition(|t| t.last_active_ms < cutoff && active.as_deref() != Some(&t.id));
            space.today = keep;
            stale.extend(old.into_iter().map(|t| (t, space.id.clone())));
        }
        let mut effects = vec![];
        for (t, space_id) in stale {
            self.archive_tab(&t, &space_id, now_ms);
            effects.push(Effect::CloseBrowser { tab_id: t.id });
        }
        effects
    }

    fn folder_mut(&mut self, id: &str) -> Result<&mut Folder, CommandError> {
        self.spaces
            .iter_mut()
            .find_map(|s| s.folder_mut(id))
            .ok_or_else(|| CommandError::NoFolder(id.into()))
    }

    fn close_tab(&mut self, tab_id: &str) -> CommandResult {
        let space_id = self
            .space_of(tab_id)
            .ok_or_else(|| CommandError::NoTab(tab_id.into()))?
            .id
            .clone();
        let space = self.space_mut(&space_id).unwrap();
        let section = space.section_of(tab_id).unwrap();
        let effects = vec![Effect::CloseBrowser {
            tab_id: tab_id.into(),
        }];

        // The tab to show next, Arc-style: the neighbour in Today, else nothing.
        let today_pos = space.today.iter().position(|t| t.id == tab_id);
        if section == Section::Today {
            let tab = space.today.remove(today_pos.unwrap());
            let now = now_ms();
            // Re-borrow after archiving, which needs the whole state.
            self.archive_tab(&tab, &space_id, now);
        } else {
            let tab = space.tab_mut(tab_id).unwrap();
            if let Some(home) = tab.home_url.clone() {
                tab.url = home;
            }
        }

        let space = self.space_mut(&space_id).unwrap();
        if space.active_tab.as_deref() == Some(tab_id) {
            space.active_tab = match today_pos {
                Some(p) if !space.today.is_empty() => {
                    Some(space.today[p.min(space.today.len() - 1)].id.clone())
                }
                None => space.today.first().map(|t| t.id.clone()),
                _ => None,
            };
            let now = now_ms();
            if let Some(next) = space.active_tab.clone() {
                if let Some(t) = space.tab_mut(&next) {
                    t.last_active_ms = now;
                }
            }
        }
        Ok(effects)
    }

    fn move_node(&mut self, node_id: &str, to: Location) -> CommandResult {
        let from_space = self
            .space_of(node_id)
            .ok_or_else(|| CommandError::NoTab(node_id.into()))?
            .id
            .clone();
        let to_space = to.space_id.clone().unwrap_or_else(|| from_space.clone());
        let target = self
            .space(&to_space)
            .ok_or_else(|| CommandError::NoSpace(to_space.clone()))?;
        if let Some(p) = &to.parent
            && to.section == Section::Pinned && !contains_folder(&target.pinned, p) {
                return Err(CommandError::NoFolder(p.clone()));
            }

        let source = self.space(&from_space).unwrap();
        let section = source.section_of(node_id).unwrap();
        let is_folder = section == Section::Pinned
            && matches!(find_node(&source.pinned, node_id), Some(Node::Folder(_)));
        if is_folder {
            if to.section != Section::Pinned {
                return Err(CommandError::FolderOutsidePinned(to.section));
            }
            if let (Some(p), Some(Node::Folder(f))) =
                (&to.parent, find_node(&source.pinned, node_id))
                && (&f.id == p || contains_node(&f.children, p)) {
                    return Err(CommandError::FolderIntoItself);
                }
        }

        // Take the node out of its current place.
        let source = self.space_mut(&from_space).unwrap();
        let mut node = match section {
            Section::Favorites => {
                let i = source
                    .favorites
                    .iter()
                    .position(|t| t.id == node_id)
                    .unwrap();
                Node::Tab(source.favorites.remove(i))
            }
            Section::Pinned => remove_node(&mut source.pinned, node_id).unwrap(),
            Section::Today => {
                let i = source.today.iter().position(|t| t.id == node_id).unwrap();
                Node::Tab(source.today.remove(i))
            }
        };

        // Pinning gives a tab a home; unpinning takes it away.
        if let Node::Tab(t) = &mut node {
            match to.section {
                Section::Favorites | Section::Pinned => {
                    if t.home_url.is_none() {
                        t.home_url = Some(t.url.clone());
                    }
                }
                Section::Today => t.home_url = None,
            }
        }

        let target = self.space_mut(&to_space).unwrap();
        match to.section {
            Section::Favorites | Section::Today => {
                let Node::Tab(t) = node else { unreachable!() };
                let list = if to.section == Section::Favorites {
                    &mut target.favorites
                } else {
                    &mut target.today
                };
                let i = to.index.unwrap_or(list.len()).min(list.len());
                list.insert(i, t);
            }
            Section::Pinned => {
                let list = match &to.parent {
                    Some(p) => &mut target.folder_mut(p).unwrap().children,
                    None => &mut target.pinned,
                };
                let i = to.index.unwrap_or(list.len()).min(list.len());
                list.insert(i, node);
            }
        }

        // Moving to another space takes the active tab along if it was inside what moved.
        let source = self.space_mut(&from_space).unwrap();
        if let Some(a) = source.active_tab.clone() {
            if source.tab(&a).is_none() {
                source.active_tab = None;
            }
        }

        let mut effects = vec![];
        if from_space != to_space {
            let profile_changed = self.space(&from_space).unwrap().profile_id
                != self.space(&to_space).unwrap().profile_id;
            let moved = self.space(&to_space).unwrap();
            let ids: Vec<Id> = if is_folder {
                let mut v = vec![];
                if let Some(Node::Folder(f)) = find_node(&moved.pinned, node_id) {
                    collect_tab_ids(&f.children, &mut v);
                }
                v
            } else {
                vec![node_id.to_string()]
            };
            effects.extend(ids.into_iter().map(|tab_id| Effect::Rehome {
                tab_id,
                profile_changed,
            }));
        }
        Ok(effects)
    }
}

fn find_node<'a>(nodes: &'a [Node], id: &str) -> Option<&'a Node> {
    for n in nodes {
        if n.id() == id {
            return Some(n);
        }
        if let Node::Folder(f) = n
            && let Some(found) = find_node(&f.children, id) {
                return Some(found);
            }
    }
    None
}

fn contains_folder(nodes: &[Node], id: &str) -> bool {
    matches!(find_node(nodes, id), Some(Node::Folder(_)))
}

fn collect_tab_ids(nodes: &[Node], out: &mut Vec<Id>) {
    for n in nodes {
        match n {
            Node::Tab(t) => out.push(t.id.clone()),
            Node::Folder(f) => collect_tab_ids(&f.children, out),
        }
    }
}

/// The list that directly contains `id`.
fn parent_list_mut<'a>(nodes: &'a mut Vec<Node>, id: &str) -> Option<&'a mut Vec<Node>> {
    if nodes.iter().any(|n| n.id() == id) {
        return Some(nodes);
    }
    for n in nodes.iter_mut() {
        if let Node::Folder(f) = n
            && let Some(found) = parent_list_mut(&mut f.children, id) {
                return Some(found);
            }
    }
    None
}
