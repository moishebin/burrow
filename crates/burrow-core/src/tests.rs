use crate::*;

fn loc(section: Section) -> Location {
    Location {
        space_id: None,
        section,
        parent: None,
        index: None,
    }
}

fn open(state: &mut State, url: &str) -> Id {
    state.apply(Command::NewTab { url: url.into() }).unwrap();
    state.active_tab().unwrap().id.clone()
}

#[test]
fn new_tab_goes_to_top_of_today_and_becomes_active() {
    let mut s = State::default();
    let a = open(&mut s, "https://a.example");
    let b = open(&mut s, "https://b.example");
    let today: Vec<_> = s
        .active_space()
        .today
        .iter()
        .map(|t| t.id.clone())
        .collect();
    assert_eq!(today, vec![b.clone(), a]);
    assert_eq!(s.active_tab().unwrap().id, b);
}

#[test]
fn pinning_sets_home_and_closing_returns_home_without_removing() {
    let mut s = State::default();
    let t = open(&mut s, "https://mail.example");
    s.apply(Command::MoveNode {
        node_id: t.clone(),
        to: loc(Section::Pinned),
    })
    .unwrap();
    assert_eq!(
        s.tab(&t).unwrap().home_url.as_deref(),
        Some("https://mail.example")
    );

    // Browsing away inside the pinned tab keeps its place and its home.
    s.tab_mut(&t).unwrap().url = "https://mail.example/inbox/123".into();
    let fx = s.apply(Command::CloseTab { tab_id: t.clone() }).unwrap();
    assert_eq!(fx, vec![Effect::CloseBrowser { tab_id: t.clone() }]);
    let tab = s.tab(&t).unwrap();
    assert_eq!(tab.url, "https://mail.example");
    assert_eq!(s.active_space().section_of(&t), Some(Section::Pinned));
    assert_eq!(s.active_space().active_tab, None);
}

#[test]
fn closing_today_tab_removes_it_and_selects_neighbour() {
    let mut s = State::default();
    let a = open(&mut s, "https://a");
    let b = open(&mut s, "https://b");
    let c = open(&mut s, "https://c"); // today: c, b, a
    s.apply(Command::SelectTab { tab_id: b.clone() }).unwrap();
    s.apply(Command::CloseTab { tab_id: b.clone() }).unwrap();
    assert!(s.tab(&b).is_none());
    assert_eq!(s.active_tab().unwrap().id, a);
    assert_eq!(s.active_space().today.len(), 2);
    let _ = c;
}

#[test]
fn reset_tab_navigates_home() {
    let mut s = State::default();
    let t = open(&mut s, "https://home");
    s.apply(Command::AddFavorite { tab_id: t.clone() }).unwrap();
    s.tab_mut(&t).unwrap().url = "https://home/elsewhere".into();
    let fx = s.apply(Command::ResetTab { tab_id: t.clone() }).unwrap();
    assert_eq!(
        fx,
        vec![Effect::Navigate {
            tab_id: t.clone(),
            url: "https://home".into()
        }]
    );
    assert_eq!(s.active_space().favorites[0].id, t);
}

#[test]
fn folders_nest_and_tabs_move_into_them() {
    let mut s = State::default();
    s.apply(Command::NewFolder {
        space_id: None,
        parent: None,
        name: "Work".into(),
    })
    .unwrap();
    let work = s.active_space().pinned[0].id().clone();
    s.apply(Command::NewFolder {
        space_id: None,
        parent: Some(work.clone()),
        name: "PRs".into(),
    })
    .unwrap();
    let t = open(&mut s, "https://github.com");
    s.apply(Command::MoveNode {
        node_id: t.clone(),
        to: Location {
            space_id: None,
            section: Section::Pinned,
            parent: Some(work.clone()),
            index: Some(0),
        },
    })
    .unwrap();
    let Node::Folder(f) = &s.active_space().pinned[0] else {
        panic!()
    };
    assert_eq!(f.children.len(), 2);
    assert_eq!(f.children[0].id(), &t);
    assert!(s.active_space().today.is_empty());
}

#[test]
fn folder_cannot_move_into_its_own_descendant() {
    let mut s = State::default();
    s.apply(Command::NewFolder {
        space_id: None,
        parent: None,
        name: "A".into(),
    })
    .unwrap();
    let a = s.active_space().pinned[0].id().clone();
    s.apply(Command::NewFolder {
        space_id: None,
        parent: Some(a.clone()),
        name: "B".into(),
    })
    .unwrap();
    let Node::Folder(fa) = &s.active_space().pinned[0] else {
        panic!()
    };
    let b = fa.children[0].id().clone();
    let err = s
        .apply(Command::MoveNode {
            node_id: a,
            to: Location {
                space_id: None,
                section: Section::Pinned,
                parent: Some(b),
                index: None,
            },
        })
        .unwrap_err();
    assert_eq!(err, CommandError::FolderIntoItself);
    assert_eq!(s.active_space().pinned.len(), 1);
}

#[test]
fn deleting_folder_keeps_children_in_place() {
    let mut s = State::default();
    let x = open(&mut s, "https://x");
    s.apply(Command::MoveNode {
        node_id: x.clone(),
        to: loc(Section::Pinned),
    })
    .unwrap();
    s.apply(Command::NewFolder {
        space_id: None,
        parent: None,
        name: "F".into(),
    })
    .unwrap();
    let f = s.active_space().pinned[1].id().clone();
    let y = open(&mut s, "https://y");
    s.apply(Command::MoveNode {
        node_id: y.clone(),
        to: Location {
            space_id: None,
            section: Section::Pinned,
            parent: Some(f.clone()),
            index: None,
        },
    })
    .unwrap();
    s.apply(Command::DeleteFolder { folder_id: f }).unwrap();
    let ids: Vec<_> = s
        .active_space()
        .pinned
        .iter()
        .map(|n| n.id().clone())
        .collect();
    assert_eq!(ids, vec![x, y]);
}

#[test]
fn new_space_gets_its_own_profile_and_becomes_active() {
    let mut s = State::default();
    let first_profile = s.active_space().profile_id.clone();
    let fx = s
        .apply(Command::NewSpace {
            name: "Work".into(),
            icon: "💼".into(),
            profile_id: None,
        })
        .unwrap();
    assert_eq!(s.spaces.len(), 2);
    assert_eq!(s.active_space().name, "Work");
    assert_ne!(s.active_space().profile_id, first_profile);
    assert!(matches!(fx[0], Effect::ProfileCreated { .. }));

    // Sharing a profile is allowed.
    s.apply(Command::NewSpace {
        name: "Side".into(),
        icon: String::new(),
        profile_id: Some(first_profile.clone()),
    })
    .unwrap();
    assert_eq!(s.active_space().profile_id, first_profile);
}

#[test]
fn moving_tab_to_space_with_other_profile_reports_it() {
    let mut s = State::default();
    let personal = s.active_space.clone();
    let t = open(&mut s, "https://a");
    s.apply(Command::NewSpace {
        name: "Work".into(),
        icon: String::new(),
        profile_id: None,
    })
    .unwrap();
    let work = s.active_space.clone();
    s.apply(Command::SwitchSpace {
        space_id: personal.clone(),
    })
    .unwrap();
    let fx = s
        .apply(Command::MoveNode {
            node_id: t.clone(),
            to: Location {
                space_id: Some(work.clone()),
                section: Section::Today,
                parent: None,
                index: None,
            },
        })
        .unwrap();
    assert_eq!(
        fx,
        vec![Effect::Rehome {
            tab_id: t.clone(),
            profile_changed: true
        }]
    );
    assert_eq!(s.space_of(&t).unwrap().id, work);
    assert_eq!(s.space(&personal).unwrap().active_tab, None);
}

#[test]
fn cannot_delete_last_space() {
    let mut s = State::default();
    let id = s.active_space.clone();
    assert_eq!(
        s.apply(Command::DeleteSpace { space_id: id }),
        Err(CommandError::LastSpace)
    );
}

#[test]
fn commands_parse_from_ui_json() {
    let c: Command = serde_json::from_str(
        r#"{"cmd":"move_node","node_id":"n1","to":{"section":"pinned","parent":"f1","index":2}}"#,
    )
    .unwrap();
    assert_eq!(
        c,
        Command::MoveNode {
            node_id: "n1".into(),
            to: Location {
                space_id: None,
                section: Section::Pinned,
                parent: Some("f1".into()),
                index: Some(2)
            },
        }
    );
}

#[test]
fn state_round_trips_through_disk() {
    let dir = std::env::temp_dir().join(format!("burrow-test-{}", model::new_id()));
    let path = dir.join("state.json");
    let mut s = State::default();
    let t = open(&mut s, "https://a");
    s.apply(Command::AddFavorite { tab_id: t }).unwrap();
    store::save(&path, &s).unwrap();
    assert_eq!(store::load(&path).unwrap(), s);

    std::fs::write(&path, "{ not json").unwrap();
    let fresh = store::load(&path).unwrap();
    assert_eq!(fresh.spaces.len(), 1);
    assert!(
        std::fs::read_dir(&dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("corrupt"))
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn closed_today_tabs_can_be_reopened_in_their_space() {
    let mut s = State::default();
    let home = s.active_space.clone();
    let t = open(&mut s, "https://closed.example");
    s.apply(Command::CloseTab { tab_id: t }).unwrap();
    assert_eq!(s.archive[0].url, "https://closed.example");

    s.apply(Command::NewSpace { name: "Other".into(), icon: String::new(), profile_id: None }).unwrap();
    s.apply(Command::ReopenClosed).unwrap();
    assert_eq!(s.active_space, home);
    assert_eq!(s.active_tab().unwrap().url, "https://closed.example");
    assert!(s.archive.is_empty());
}

#[test]
fn stale_today_tabs_are_archived_but_not_the_active_one() {
    let mut s = State::default();
    let old = open(&mut s, "https://old.example");
    let active = open(&mut s, "https://active.example");
    let now = 100 * 3_600_000;
    for t in &mut s.active_space_mut().today {
        t.last_active_ms = 0;
    }
    let pinned = open(&mut s, "https://pinned.example");
    s.apply(Command::MoveNode { node_id: pinned.clone(), to: loc(Section::Pinned) }).unwrap();
    s.apply(Command::SelectTab { tab_id: active.clone() }).unwrap();
    s.tab_mut(&active).unwrap().last_active_ms = 0;

    let fx = s.archive_stale(now);
    assert_eq!(fx, vec![Effect::CloseBrowser { tab_id: old }]);
    let today: Vec<_> = s.active_space().today.iter().map(|t| t.id.clone()).collect();
    assert_eq!(today, vec![active]);
    assert!(s.tab(&pinned).is_some(), "pinned tabs are never archived");

    s.settings.archive_after_hours = 0;
    assert!(s.archive_stale(now * 2).is_empty());
}

#[test]
fn permission_memory_is_per_profile_and_origin() {
    let mut s = State::default();
    let p = s.profiles[0].id.clone();
    let meet = "https://meet.example";
    assert_eq!(s.permission_decision(&p, meet, &["camera", "microphone"]), None);
    s.remember_permission(&p, meet, &["camera"], true);
    assert_eq!(s.permission_decision(&p, meet, &["camera", "microphone"]), None);
    s.remember_permission(&p, meet, &["microphone"], true);
    assert_eq!(s.permission_decision(&p, meet, &["camera", "microphone"]), Some(true));
    s.remember_permission(&p, meet, &["camera"], false);
    assert_eq!(s.permission_decision(&p, meet, &["camera", "microphone"]), Some(false));
    assert_eq!(s.permission_decision("other-profile", meet, &["camera"]), None);
}

#[test]
fn importing_replaces_an_untouched_default_space() {
    let mut s = State::default();
    let mut other = State::default();
    other.spaces[0].name = "Imported".into();
    assert_eq!(s.absorb(other), 1);
    assert_eq!(s.spaces.len(), 1);
    assert_eq!(s.profiles.len(), 1);
    assert_eq!(s.active_space().name, "Imported");
}

#[test]
fn leaving_a_tab_stamps_it_so_it_is_not_archived_right_away() {
    let mut s = State::default();
    let a = open(&mut s, "https://a.example");
    let b = open(&mut s, "https://b.example");
    s.tab_mut(&a).unwrap().last_active_ms = 0;
    s.apply(Command::SelectTab { tab_id: a.clone() }).unwrap();
    s.tab_mut(&a).unwrap().last_active_ms = 0; // long ago…
    s.apply(Command::SelectTab { tab_id: b }).unwrap(); // …but in use until now
    assert!(s.tab(&a).unwrap().last_active_ms > 0);
}

#[test]
fn moving_a_folder_with_the_active_tab_away_clears_active() {
    let mut s = State::default();
    let home = s.active_space.clone();
    s.apply(Command::NewFolder { space_id: None, parent: None, name: "F".into() }).unwrap();
    let f = s.active_space().pinned[0].id().clone();
    let t = open(&mut s, "https://x");
    s.apply(Command::MoveNode {
        node_id: t.clone(),
        to: Location { space_id: None, section: Section::Pinned, parent: Some(f.clone()), index: None },
    })
    .unwrap();
    s.apply(Command::SelectTab { tab_id: t }).unwrap();
    s.apply(Command::NewSpace { name: "W".into(), icon: String::new(), profile_id: None }).unwrap();
    let w = s.active_space.clone();
    s.apply(Command::MoveNode {
        node_id: f,
        to: Location { space_id: Some(w), section: Section::Pinned, parent: None, index: None },
    })
    .unwrap();
    assert_eq!(s.space(&home).unwrap().active_tab, None);
}

#[test]
fn reopen_archived_by_index() {
    let mut s = State::default();
    let a = open(&mut s, "https://a");
    let b = open(&mut s, "https://b");
    s.apply(Command::CloseTab { tab_id: a }).unwrap();
    s.apply(Command::CloseTab { tab_id: b }).unwrap(); // archive: b, a
    s.apply(Command::ReopenArchived { index: 1 }).unwrap();
    assert_eq!(s.active_tab().unwrap().url, "https://a");
    assert_eq!(s.archive.len(), 1);
    assert_eq!(s.archive[0].url, "https://b");
}

// ---- Agent tools (D-018) ------------------------------------------------------------------------

fn tool(s: &mut State, name: &str, args: serde_json::Value) -> serde_json::Value {
    agent::run_model_tool(s, name, &args).expect("model tool").unwrap_or_else(|e| panic!("{name}: {e}")).result
}

#[test]
fn open_tab_in_background_keeps_the_shown_tab() {
    let mut s = State::default();
    let a = open(&mut s, "https://a");
    let (b, _) = s.open_tab("https://b".into(), None, false).unwrap();
    assert_eq!(s.active_tab().unwrap().id, a);
    assert_eq!(s.active_space().today[0].id, b);
    let (c, _) = s.open_tab("https://c".into(), None, true).unwrap();
    assert_eq!(s.active_tab().unwrap().id, c);
}

#[test]
fn open_tab_into_a_folder_of_another_space() {
    let mut s = State::default();
    let other = tool(&mut s, "create_space", serde_json::json!({ "name": "Work" }))["space_id"].as_str().unwrap().to_string();
    assert_ne!(s.active_space, other, "creating a space doesn't switch to it");
    let f = tool(&mut s, "create_folder", serde_json::json!({ "name": "Docs", "space_id": other }))["folder_id"]
        .as_str()
        .unwrap()
        .to_string();
    let to = agent::location(&serde_json::json!({ "space_id": other, "folder_id": f }), None).unwrap();
    let (t, _) = s.open_tab("https://docs".into(), to, false).unwrap();
    let info = agent::list_tabs(&s, Some(&other));
    assert_eq!(info.len(), 1);
    assert_eq!(info[0].id, t);
    assert_eq!(info[0].section, Section::Pinned);
    assert_eq!(info[0].folders, vec!["Docs".to_string()]);
    assert_eq!(info[0].home_url.as_deref(), Some("https://docs"));
}

#[test]
fn open_tab_into_missing_folder_leaves_nothing_behind() {
    let mut s = State::default();
    let to = agent::location(&serde_json::json!({ "folder_id": "nope" }), None).unwrap();
    assert!(s.open_tab("https://x".into(), to, false).is_err());
    assert!(s.active_space().today.is_empty());
}

#[test]
fn delete_tab_removes_pinned_tabs_for_good() {
    let mut s = State::default();
    let t = open(&mut s, "https://mail");
    s.apply(Command::MoveNode { node_id: t.clone(), to: loc(Section::Pinned) }).unwrap();
    tool(&mut s, "close_tab", serde_json::json!({ "tab_id": t }));
    assert!(s.tab(&t).is_some(), "plain close only unloads a pinned tab");
    let fx = agent::run_model_tool(&mut s, "close_tab", &serde_json::json!({ "tab_id": t, "delete": true }))
        .unwrap()
        .unwrap()
        .effects;
    assert_eq!(fx, vec![Effect::CloseBrowser { tab_id: t.clone() }]);
    assert!(s.tab(&t).is_none());
}

#[test]
fn move_tab_keeps_its_section_unless_told() {
    let mut s = State::default();
    let a = open(&mut s, "https://a");
    let b = open(&mut s, "https://b"); // today: b, a
    tool(&mut s, "move_tab", serde_json::json!({ "node_id": a, "index": 0 }));
    assert_eq!(s.active_space().today[0].id, a);
    tool(&mut s, "move_tab", serde_json::json!({ "node_id": b, "section": "favorites" }));
    assert_eq!(s.active_space().favorites[0].id, b);
}

#[test]
fn update_folder_collapses_renames_and_deletes() {
    let mut s = State::default();
    let f = tool(&mut s, "create_folder", serde_json::json!({ "name": "A" }))["folder_id"].as_str().unwrap().to_string();
    tool(&mut s, "update_folder", serde_json::json!({ "folder_id": f, "name": "B", "collapsed": true }));
    tool(&mut s, "update_folder", serde_json::json!({ "folder_id": f, "collapsed": true }));
    let Node::Folder(folder) = &s.active_space().pinned[0] else { panic!() };
    assert_eq!((folder.name.as_str(), folder.collapsed), ("B", true));
    tool(&mut s, "update_folder", serde_json::json!({ "folder_id": f, "delete": true }));
    assert!(s.active_space().pinned.is_empty());
    assert!(agent::run_model_tool(&mut s, "update_folder", &serde_json::json!({ "folder_id": f })).unwrap().is_err());
}

#[test]
fn create_space_can_share_logins() {
    let mut s = State::default();
    let first = s.active_space.clone();
    let id = tool(&mut s, "create_space", serde_json::json!({ "name": "Side", "share_logins_with": first, "color": "#bcd6f2", "focus": true }))
        ["space_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(s.active_space, id);
    assert_eq!(s.profiles.len(), 1);
    assert_eq!(s.space(&id).unwrap().color, "#bcd6f2");
    let spaces = tool(&mut s, "list_spaces", serde_json::json!({}));
    assert_eq!(spaces["spaces"][0]["shares_logins_with"][0], id);
}

#[test]
fn every_tool_has_an_object_schema() {
    for t in agent::TOOLS {
        assert_eq!((t.schema)()["type"], "object", "{}", t.name);
    }
    assert!(agent::run_model_tool(&mut State::default(), "screenshot", &serde_json::json!({})).is_none());
}

#[test]
fn page_tools_are_known_tools_and_tab_tools_are_not_page_tools() {
    for name in agent::PAGE_TOOLS {
        assert!(agent::tool(name).is_some(), "{name}");
    }
    for name in ["open_tab", "close_tab", "navigate", "create_space", "list_tabs"] {
        assert!(!agent::is_page_tool(name), "{name}");
    }
}

#[test]
fn split_view_shows_both_tabs_while_either_is_active() {
    let mut s = State::default();
    let a = open(&mut s, "https://a.example");
    let b = open(&mut s, "https://b.example");
    let c = open(&mut s, "https://c.example");
    s.apply(Command::SelectTab { tab_id: a.clone() }).unwrap();
    s.apply(Command::SplitView { tab_id: b.clone() }).unwrap();
    assert_eq!(s.visible_tabs(), vec![a.clone(), b.clone()]);
    s.apply(Command::SelectTab { tab_id: b.clone() }).unwrap();
    assert_eq!(s.visible_tabs(), vec![a.clone(), b.clone()], "the halves keep their sides");
    s.apply(Command::SelectTab { tab_id: c.clone() }).unwrap();
    assert_eq!(s.visible_tabs(), vec![c.clone()]);
    assert_eq!(s.apply(Command::SplitView { tab_id: c.clone() }), Err(CommandError::BadSplit));

    // The divider moves within limits.
    s.apply(Command::SetSplitRatio { space_id: None, ratio: 0.7 }).unwrap();
    assert_eq!(s.active_space().split.as_ref().unwrap().ratio, 0.7);
    s.apply(Command::SetSplitRatio { space_id: None, ratio: 0.01 }).unwrap();
    assert_eq!(s.active_space().split.as_ref().unwrap().ratio, crate::model::SPLIT_RATIO_MIN);

    // Closing one half shows the other and ends the split.
    s.apply(Command::SelectTab { tab_id: a.clone() }).unwrap();
    s.apply(Command::CloseTab { tab_id: b.clone() }).unwrap();
    assert_eq!(s.visible_tabs(), vec![a.clone()]);
    assert_eq!(s.active_space().split, None);
}

#[test]
fn split_ends_when_a_half_leaves_the_space_or_is_cleared() {
    let mut s = State::default();
    let a = open(&mut s, "https://a.example");
    let b = open(&mut s, "https://b.example");
    s.apply(Command::SplitView { tab_id: a.clone() }).unwrap();
    let home = s.active_space.clone();
    s.apply(Command::NewSpace { name: "W".into(), icon: String::new(), profile_id: None }).unwrap();
    let work = s.active_space.clone();
    s.active_space = home.clone();
    let to = Location { space_id: Some(work), section: Section::Today, parent: None, index: Some(0) };
    s.apply(Command::MoveNode { node_id: a.clone(), to }).unwrap();
    assert_eq!(s.space(&home).unwrap().split, None);

    let c = open(&mut s, "https://c.example");
    s.apply(Command::SplitView { tab_id: b.clone() }).unwrap();
    assert!(s.active_space().split.is_some());
    s.apply(Command::ClearToday).unwrap();
    assert_eq!(s.active_space().split, None);
    let _ = c;
}

#[test]
fn auto_archive_keeps_both_halves_of_a_shown_split() {
    let mut s = State::default();
    let a = open(&mut s, "https://a.example");
    let b = open(&mut s, "https://b.example");
    s.apply(Command::SplitView { tab_id: a.clone() }).unwrap();
    for t in &mut s.active_space_mut().today {
        t.last_active_ms = 0;
    }
    assert!(s.archive_stale(100 * 3_600_000).is_empty());
    assert!(s.tab(&a).is_some() && s.tab(&b).is_some());
}

#[test]
fn duplicate_opens_a_copy_below_and_shows_it() {
    let mut s = State::default();
    let a = open(&mut s, "https://a.example/page");
    let _b = open(&mut s, "https://b.example"); // today: b, a
    s.apply(Command::DuplicateTab { tab_id: a.clone() }).unwrap();
    let today = &s.active_space().today;
    assert_eq!(today.len(), 3);
    assert_eq!(today[1].id, a);
    assert_eq!(today[2].url, "https://a.example/page");
    assert_eq!(s.active_tab().unwrap().id, today[2].id);
}

#[test]
fn routes_are_cleaned_replaced_and_removed_with_their_space() {
    let mut s = State::default();
    let home = s.active_space.clone();
    s.apply(Command::NewSpace { name: "W".into(), icon: String::new(), profile_id: None }).unwrap();
    let work = s.active_space.clone();
    s.apply(Command::AddRoute { pattern: " https://www.Figma.com/ ".into(), space_id: work.clone() }).unwrap();
    assert_eq!(s.routes[0].pattern, "figma.com");
    s.apply(Command::AddRoute { pattern: "figma.com".into(), space_id: home.clone() }).unwrap();
    assert_eq!(s.routes.len(), 1, "the same pattern replaces the old rule");
    assert_eq!(s.routes[0].space_id, home);
    assert_eq!(s.apply(Command::AddRoute { pattern: "  ".into(), space_id: work.clone() }), Err(CommandError::BadPattern));
    assert!(s.apply(Command::AddRoute { pattern: "x.com".into(), space_id: "nope".into() }).is_err());

    s.apply(Command::AddRoute { pattern: "linear.app".into(), space_id: work.clone() }).unwrap();
    s.apply(Command::DeleteSpace { space_id: work }).unwrap();
    assert_eq!(s.routes.len(), 1);
    let id = s.routes[0].id.clone();
    s.apply(Command::RemoveRoute { route_id: id }).unwrap();
    assert!(s.routes.is_empty());
}

#[test]
fn old_state_files_load_with_new_fields_defaulted() {
    let mut v = serde_json::to_value(State::default()).unwrap();
    let o = v.as_object_mut().unwrap();
    o.remove("routes");
    o.remove("zoom");
    o.remove("boosts");
    o["settings"].as_object_mut().unwrap().remove("peek_new_tabs");
    let s: State = serde_json::from_value(v).unwrap();
    assert!(s.routes.is_empty() && s.zoom.is_empty());
    assert!(s.settings.peek_new_tabs && s.settings.peek_from_pinned);
}

#[test]
fn agents_can_manage_air_traffic_control() {
    use crate::agent::run_model_tool;
    let mut s = State::default();
    let space = s.active_space.clone();
    let add = serde_json::json!({ "action": "add", "pattern": "figma.com", "space_id": space });
    let out = run_model_tool(&mut s, "air_traffic_control", &add).unwrap().unwrap();
    let id = out.result["rules"][0]["route_id"].as_str().unwrap().to_string();
    assert_eq!(out.result["rules"][0]["pattern"], "figma.com");
    let bad = serde_json::json!({ "action": "add", "pattern": "", "space_id": space });
    assert!(run_model_tool(&mut s, "air_traffic_control", &bad).unwrap().is_err());
    let rm = serde_json::json!({ "action": "remove", "route_id": id });
    let out = run_model_tool(&mut s, "air_traffic_control", &rm).unwrap().unwrap();
    assert_eq!(out.result["rules"].as_array().unwrap().len(), 0);
}

#[test]
fn agent_split_and_duplicate_leave_the_users_view_alone() {
    use crate::agent::run_model_tool;
    use serde_json::json;
    let mut s = State::default();
    let a = open(&mut s, "https://a.example");
    let b = open(&mut s, "https://b.example");
    let c = open(&mut s, "https://c.example"); // shown
    let out = run_model_tool(&mut s, "split_view", &json!({ "tab_id": a, "other_tab_id": b })).unwrap().unwrap();
    assert_eq!(out.result["split"], true);
    assert_eq!(s.active_tab().unwrap().id, c, "the user still sees c");
    assert_eq!(s.visible_tabs(), vec![c.clone()]);
    s.apply(Command::SelectTab { tab_id: b.clone() }).unwrap();
    assert_eq!(s.visible_tabs(), vec![a.clone(), b.clone()]);
    assert!(run_model_tool(&mut s, "split_view", &json!({ "tab_id": a, "other_tab_id": a })).unwrap().is_err());
    run_model_tool(&mut s, "split_view", &json!({ "action": "close", "tab_id": a })).unwrap().unwrap();
    assert_eq!(s.active_space().split, None);

    let out = run_model_tool(&mut s, "duplicate_tab", &json!({ "tab_id": c })).unwrap().unwrap();
    let copy = out.result["tab_id"].as_str().unwrap().to_string();
    assert_ne!(copy, c);
    assert_eq!(s.tab(&copy).unwrap().url, "https://c.example");
    assert_eq!(s.active_tab().unwrap().id, b, "a background copy doesn't change the shown tab");

    let out = run_model_tool(&mut s, "link_settings", &json!({ "peek_new_tabs": false })).unwrap().unwrap();
    assert_eq!(out.result, json!({ "peek_new_tabs": false, "peek_from_pinned": true }));
}
