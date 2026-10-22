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
