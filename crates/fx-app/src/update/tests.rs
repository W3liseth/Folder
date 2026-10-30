use std::path::{Path, PathBuf};
use std::time::Duration;

use fx_commands::CommandId;
use fx_core::entry::attrs;
use fx_core::{Batch, SortDir, SortKey};
use fx_fs::{Change, DirEvent, EnumStats, Outcome};
use fx_store::{Layout, OpEntry, OpOutcome, Settings};
use fx_win::fileop::{FileOp, OpEvent};
use fx_win::sys::Stat;

use crate::effect::{Effect, Persist};
use crate::history::Location;
use crate::msg::Msg;
use crate::ops::{Job, OpStatus};
use crate::state::{AppState, Dialog, ListingStatus, PaneId, Tags};
use crate::update::update;

fn state() -> AppState {
    AppState::new(
        Settings {
            confirm_delete: true,
            ..Settings::default()
        },
        Tags::default(),
        Vec::new(),
    )
}

fn batch(names: &[(&str, bool)]) -> Batch {
    let mut b = Batch::default();
    for &(n, dir) in names {
        let w: Vec<u16> = n.encode_utf16().collect();
        b.push_utf16(&w, if dir { attrs::DIRECTORY } else { 0 }, 1, 0, 0);
    }
    b
}

fn enum_gen(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|e| match e {
            Effect::Enumerate { generation, .. } => Some(*generation),
            _ => None,
        })
        .unwrap_or_else(|| panic!("pas d'énumération dans {effects:?}"))
}

fn pane(s: &AppState) -> PaneId {
    s.pane().id
}

fn done(s: &mut AppState, generation: u64) {
    let stats = EnumStats {
        entries: 0,
        batches: 0,
        elapsed: Duration::ZERO,
    };
    let p = pane(s);
    update(
        s,
        Msg::ListingFinished {
            pane: p,
            generation,
            result: Ok(Outcome::Completed(stats)),
        },
    );
}

fn feed(s: &mut AppState, generation: u64, names: &[(&str, bool)]) {
    let p = pane(s);
    update(
        s,
        Msg::ListingBatch {
            pane: p,
            generation,
            batch: batch(names),
        },
    );
}

/// Ouvre `dir` et y charge `names`.
fn open(s: &mut AppState, dir: &str, names: &[(&str, bool)]) {
    let g = enum_gen(&update(s, Msg::Navigate(Location::Dir(dir.into()))));
    feed(s, g, names);
    done(s, g);
}

fn names(s: &AppState) -> Vec<String> {
    let l = s.pane().listing.as_ref().expect("listing");
    (0..l.len())
        .map(|i| String::from_utf8_lossy(l.at(i).expect("pos").name).into_owned())
        .collect()
}

fn job(effects: &[Effect]) -> &Job {
    effects
        .iter()
        .find_map(|e| match e {
            Effect::Job { job, .. } => Some(job),
            _ => None,
        })
        .unwrap_or_else(|| panic!("pas de travail dans {effects:?}"))
}

fn op_id(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|e| {
            if let Effect::Job { op, .. } = e {
                Some(*op)
            } else {
                None
            }
        })
        .expect("op")
}

#[test]
fn startup_opens_home_and_loads_sidebar_data() {
    let mut s = state();
    let fx = update(
        &mut s,
        Msg::Startup {
            start: None,
            session: None,
        },
    );
    assert_eq!(s.pane().location(), &Location::Home);
    assert!(fx.iter().any(|e| matches!(e, Effect::LoadDrives)));
    assert!(fx.iter().any(|e| matches!(e, Effect::LoadKnownFolders)));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(Persist::LoadOps))),
        "historique du centre d'état"
    );
}

#[test]
fn status_center_history_loads_from_db_and_reloads_on_open() {
    let mut s = state();
    let entry = |ended, outcome| OpEntry {
        ended,
        title: format!("op {ended}"),
        items: 1,
        outcome,
        duration_ms: 2000,
    };
    // Lancement : les échecs des sessions précédentes ne font pas de pastille.
    update(
        &mut s,
        Msg::OpHistoryLoaded(vec![
            entry(2, OpOutcome::Done),
            entry(1, OpOutcome::Failed("x".into())),
        ]),
    );
    assert_eq!(s.op_history.len(), 2);
    assert!(!s.op_history.has_unseen_failure());
    let first = s.op_history.last_id();
    // Une autre fenêtre a échoué : pastille jusqu'à l'ouverture du centre.
    update(
        &mut s,
        Msg::OpHistoryLoaded(vec![
            entry(3, OpOutcome::Failed("y".into())),
            entry(2, OpOutcome::Done),
            entry(1, OpOutcome::Failed("x".into())),
        ]),
    );
    assert!(s.op_history.has_unseen_failure());
    assert_eq!(
        s.op_history.iter().nth(1).map(|r| r.id),
        first,
        "id conservé"
    );
    let fx = update(&mut s, Msg::Command(CommandId::StatusCenter));
    assert!(matches!(&fx[..], [Effect::Persist(Persist::LoadOps)]));
    assert!(!s.op_history.has_unseen_failure());
    assert!(
        update(&mut s, Msg::Command(CommandId::StatusCenter)).is_empty(),
        "fermeture : rien à lire"
    );
}

#[test]
fn navigate_streams_watches_and_finishes() {
    let mut s = state();
    let fx = update(&mut s, Msg::Navigate(Location::Dir(r"C:\a".into())));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Watch { path, .. } if path == Path::new(r"C:\a")))
    );
    let g = enum_gen(&fx);
    assert!(s.pane().is_loading());
    feed(&mut s, g, &[("x", false), ("d", true)]);
    done(&mut s, g);
    assert_eq!(names(&s), ["d", "x"]);
    assert!(matches!(s.pane().status, ListingStatus::Ready { .. }));
}

#[test]
fn stale_generation_is_ignored_and_previous_is_cancelled() {
    let mut s = state();
    let fx = update(&mut s, Msg::Navigate(Location::Dir(r"C:\a".into())));
    let (g1, c1) = fx
        .iter()
        .find_map(|e| {
            if let Effect::Enumerate {
                generation, cancel, ..
            } = e
            {
                Some((*generation, cancel.clone()))
            } else {
                None
            }
        })
        .expect("enum");
    let g2 = enum_gen(&update(
        &mut s,
        Msg::Navigate(Location::Dir(r"C:\b".into())),
    ));
    assert!(c1.is_cancelled());
    feed(&mut s, g1, &[("stale", false)]);
    assert!(s.pane().listing.is_none());
    feed(&mut s, g2, &[("fresh", false)]);
    assert_eq!(names(&s), ["fresh"]);
}

#[test]
fn activate_dir_navigates_and_file_opens() {
    let mut s = state();
    open(&mut s, r"C:\a", &[("sub", true), ("f.txt", false)]);
    let p = pane(&s);
    let fx = update(&mut s, Msg::Activate { pane: p, pos: 1 });
    assert!(matches!(&fx[..], [Effect::Open(f)] if f == Path::new(r"C:\a\f.txt")));
    let fx = update(&mut s, Msg::Activate { pane: p, pos: 0 });
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Enumerate { path, .. } if path == Path::new(r"C:\a\sub")))
    );
    update(&mut s, Msg::Command(CommandId::Back));
    assert_eq!(s.pane().location(), &Location::Dir(r"C:\a".into()));
    update(&mut s, Msg::Command(CommandId::Up));
    assert_eq!(s.pane().location(), &Location::Dir(r"C:\".into()));
}

#[test]
fn click_ctrl_shift_selection_and_select_all() {
    let mut s = state();
    open(
        &mut s,
        r"C:\a",
        &[("a", false), ("b", false), ("c", false), ("d", false)],
    );
    let p = pane(&s);
    update(
        &mut s,
        Msg::Click {
            pane: p,
            pos: 1,
            ctrl: false,
            shift: false,
        },
    );
    update(
        &mut s,
        Msg::Click {
            pane: p,
            pos: 3,
            ctrl: false,
            shift: true,
        },
    );
    assert_eq!(s.pane().selection.count(), 3);
    update(
        &mut s,
        Msg::Click {
            pane: p,
            pos: 2,
            ctrl: true,
            shift: false,
        },
    );
    assert_eq!(s.pane().selection.count(), 2);
    update(&mut s, Msg::Command(CommandId::SelectAll));
    assert_eq!(s.pane().selection.count(), 4);
    update(&mut s, Msg::Command(CommandId::InvertSelection));
    assert_eq!(s.pane().selection.count(), 0);
}

#[test]
fn sort_toggles_and_is_remembered_per_folder() {
    let mut s = state();
    open(&mut s, r"C:\a", &[("a", false)]);
    update(&mut s, Msg::SortBy(SortKey::Name));
    assert_eq!(
        s.pane().listing.as_ref().expect("l").sort().dir,
        SortDir::Desc
    );
    let fx = update(&mut s, Msg::SortBy(SortKey::Size));
    assert!(fx.iter().any(|e| matches!(e, Effect::Persist(_))));
    let spec = s.pane().listing.as_ref().expect("l").sort();
    assert_eq!((spec.key, spec.dir), (SortKey::Size, SortDir::Desc));
    // Revenir dans le dossier réapplique le tri mémorisé.
    open(&mut s, r"C:\b", &[]);
    open(&mut s, r"C:\a", &[("a", false)]);
    assert_eq!(
        s.pane().listing.as_ref().expect("l").sort().key,
        SortKey::Size
    );
}

#[test]
fn failure_shows_unavailable_location() {
    let mut s = state();
    let g = enum_gen(&update(
        &mut s,
        Msg::Navigate(Location::Dir(r"C:\nope".into())),
    ));
    let p = pane(&s);
    update(
        &mut s,
        Msg::ListingFinished {
            pane: p,
            generation: g,
            result: Err("introuvable".into()),
        },
    );
    assert!(
        matches!(&s.pane().status, ListingStatus::Failed { message } if message == "introuvable")
    );
}

#[test]
fn tabs_open_close_and_reopen() {
    let mut s = state();
    open(&mut s, r"C:\a", &[]);
    update(&mut s, Msg::Command(CommandId::NewTab));
    assert_eq!((s.tabs.len(), s.active_tab), (2, 1));
    assert_eq!(s.pane().location(), &Location::Home);
    update(&mut s, Msg::Command(CommandId::PreviousTab));
    assert_eq!(s.active_tab, 0);
    update(&mut s, Msg::Command(CommandId::CloseTab));
    assert_eq!(s.tabs.len(), 1);
    update(&mut s, Msg::Command(CommandId::ReopenClosedTab));
    assert_eq!(s.tabs.len(), 2);
    assert_eq!(s.pane().location(), &Location::Dir(r"C:\a".into()));
}

#[test]
fn detached_tab_moves_to_a_new_window() {
    let mut s = state();
    open(&mut s, r"C:\a", &[]);
    // Seul onglet : il reste (la fenêtre serait vide).
    assert!(
        update(
            &mut s,
            Msg::DetachTab {
                index: 0,
                rect: None
            }
        )
        .is_empty()
    );
    assert_eq!(s.tabs.len(), 1);

    update(&mut s, Msg::Command(CommandId::NewTab));
    let rect = Some([10, 20, 1210, 800]);
    let fx = update(&mut s, Msg::DetachTab { index: 0, rect });
    assert_eq!((s.tabs.len(), s.active_tab), (1, 0));
    assert_eq!(s.pane().location(), &Location::Home);
    assert!(fx.iter().any(|e| matches!(e,
        Effect::NewWindow { start: Some(p), rect: r } if p == &PathBuf::from(r"C:\a") && *r == rect)));
    // Rien à rouvrir : l'onglet n'a pas été fermé, il a changé de fenêtre.
    assert!(s.closed_tabs.is_empty());
    // Nouvelle fenêtre depuis l'accueil : l'accueil, pas la session restaurée.
    let fx = update(&mut s, Msg::Command(CommandId::NewWindow));
    assert!(fx.iter().any(|e| matches!(e,
        Effect::NewWindow { start: Some(p), rect: None } if Location::from_path(p.clone()) == Location::Home)));
}

#[test]
fn tab_moves_between_windows() {
    // Fenêtre source : l'onglet part, sans être proposé à la réouverture.
    let mut s = state();
    open(&mut s, r"C:\a", &[]);
    update(&mut s, Msg::Command(CommandId::NewTab));
    update(&mut s, Msg::RemoveTab(0));
    assert_eq!((s.tabs.len(), s.active_tab), (1, 0));
    assert_eq!(s.pane().location(), &Location::Home);
    assert!(s.closed_tabs.is_empty());
    // Le dernier onglet reste (la fenêtre se ferme d'elle-même).
    update(&mut s, Msg::RemoveTab(0));
    assert_eq!(s.tabs.len(), 1);

    // Fenêtre cible : nouvel onglet inséré, activé, chargé, avec son historique.
    let mut t = state();
    open(&mut t, r"C:\b", &[]);
    let fx = update(
        &mut t,
        Msg::AdoptTab {
            path: r"C:\a\sub".into(),
            back: vec!["folder:home".into(), r"C:\a".into()],
            forward: vec![],
            index: 0,
        },
    );
    assert_eq!((t.tabs.len(), t.active_tab), (2, 0));
    assert_eq!(t.pane().location(), &Location::Dir(r"C:\a\sub".into()));
    assert!(t.pane().history.can_go_back());
    assert!(fx.iter().any(|e| matches!(e, Effect::Enumerate { .. })));
    // Index trop grand : ajouté à la fin.
    update(
        &mut t,
        Msg::AdoptTab {
            path: "folder:home".into(),
            back: vec![],
            forward: vec![],
            index: 99,
        },
    );
    assert_eq!((t.tabs.len(), t.active_tab), (3, 2));
    assert_eq!(t.pane().location(), &Location::Home);
}

#[test]
fn system_shortcuts_follow_settings() {
    let mut s = state();
    let mut new = s.settings.clone();
    new.keybindings.clear();
    // Sans changement de ces réglages : rien à appliquer.
    let fx = update(&mut s, Msg::UpdateSettings(Box::new(new.clone())));
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::SystemShortcuts { .. }))
    );
    new.win_e_opens_folder = true;
    let fx = update(&mut s, Msg::UpdateSettings(Box::new(new.clone())));
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::SystemShortcuts {
            win_e: true,
            open_hotkey: None
        }
    )));
    new.win_e_opens_folder = false;
    new.open_hotkey = fx_commands::HotKey::parse("Ctrl+Alt+F");
    let fx = update(&mut s, Msg::UpdateSettings(Box::new(new)));
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::SystemShortcuts {
            win_e: false,
            open_hotkey: Some(_)
        }
    )));
}

#[test]
fn startup_ensures_win_e_helper_only_when_enabled() {
    let has = |fx: &[Effect]| fx.iter().any(|e| matches!(e, Effect::EnsureWinE));
    let mut s = state();
    assert!(!has(&update(
        &mut s,
        Msg::Startup {
            start: None,
            session: None
        }
    )));
    let mut s = state();
    s.settings.win_e_opens_folder = true;
    assert!(has(&update(
        &mut s,
        Msg::Startup {
            start: None,
            session: None
        }
    )));
}

#[test]
fn drop_on_recycle_bin_asks_then_deletes() {
    let mut s = state();
    let items = vec![PathBuf::from(r"C:\a\x.txt")];
    s.settings.confirm_delete = true;
    assert!(update(&mut s, Msg::DropToRecycleBin(items.clone())).is_empty());
    assert!(matches!(&s.dialog,
        Some(Dialog::ConfirmDelete { items: i, permanent: false }) if i == &items));
    s.dialog = None;
    s.settings.confirm_delete = false;
    let fx = update(&mut s, Msg::DropToRecycleBin(items.clone()));
    assert!(matches!(job(&fx),
        Job::File(FileOp::Delete { items: i, permanent: false }) if i == &items));
}

#[test]
fn dual_pane_toggle_and_open_in_other_pane() {
    let mut s = state();
    open(&mut s, r"C:\a", &[("sub", true)]);
    update(&mut s, Msg::Command(CommandId::ToggleDualPane));
    assert_eq!(s.tab().panes.len(), 2);
    assert_eq!(s.tab().active, 1);
    update(&mut s, Msg::Command(CommandId::FocusOtherPane));
    assert_eq!(s.tab().active, 0);
    let p = pane(&s);
    update(
        &mut s,
        Msg::Click {
            pane: p,
            pos: 0,
            ctrl: false,
            shift: false,
        },
    );
    let fx = update(&mut s, Msg::Command(CommandId::OpenInOtherPane));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Enumerate { path, .. } if path == Path::new(r"C:\a\sub")))
    );
    let fx = update(&mut s, Msg::Command(CommandId::ToggleDualPane));
    assert!(matches!(&fx[..], [Effect::Unwatch { .. }]));
}

#[test]
fn new_folder_gets_unique_name_then_inline_rename() {
    let mut s = state();
    open(&mut s, r"C:\a", &[("Nouveau dossier", true)]);
    let fx = update(&mut s, Msg::Command(CommandId::NewFolder));
    let Job::File(FileOp::NewFolder { parent, name }) = job(&fx) else {
        panic!("{fx:?}")
    };
    assert_eq!(
        (parent.as_path(), name.as_str()),
        (Path::new(r"C:\a"), "Nouveau dossier (2)")
    );
    let stat = Stat {
        attributes: attrs::DIRECTORY,
        size: 0,
        created: 0,
        modified: 0,
    };
    let key = pane(&s).0;
    update(
        &mut s,
        Msg::DirChanged(DirEvent {
            key,
            changes: vec![Change::Upsert {
                name: "Nouveau dossier (2)".encode_utf16().collect(),
                stat,
            }],
        }),
    );
    let r = s.renaming.expect("renommage");
    assert_eq!(s.pane().selection.count(), 1);
    let fx = update(
        &mut s,
        Msg::CommitRename {
            pane: r.pane,
            id: r.id,
            name: "Projets".into(),
        },
    );
    assert!(
        matches!(job(&fx), Job::File(FileOp::Rename { new_name, .. }) if new_name == "Projets")
    );
    assert!(s.renaming.is_none());
}

#[test]
fn rename_rejects_invalid_names() {
    let mut s = state();
    open(&mut s, r"C:\a", &[("f.txt", false)]);
    let p = pane(&s);
    update(
        &mut s,
        Msg::Click {
            pane: p,
            pos: 0,
            ctrl: false,
            shift: false,
        },
    );
    update(&mut s, Msg::Command(CommandId::Rename));
    let r = s.renaming.expect("renaming");
    let fx = update(
        &mut s,
        Msg::CommitRename {
            pane: r.pane,
            id: r.id,
            name: "a:b".into(),
        },
    );
    assert!(fx.is_empty());
    assert_eq!(s.toasts.len(), 1);
}

#[test]
fn delete_asks_confirmation_then_runs() {
    let mut s = state();
    open(&mut s, r"C:\a", &[("f.txt", false)]);
    let p = pane(&s);
    update(
        &mut s,
        Msg::Click {
            pane: p,
            pos: 0,
            ctrl: false,
            shift: false,
        },
    );
    assert!(update(&mut s, Msg::Command(CommandId::Delete)).is_empty());
    assert!(matches!(
        s.dialog,
        Some(Dialog::ConfirmDelete {
            permanent: false,
            ..
        })
    ));
    let fx = update(&mut s, Msg::ConfirmDialog);
    assert!(
        matches!(job(&fx), Job::File(FileOp::Delete { items, permanent: false }) if items == &[PathBuf::from(r"C:\a\f.txt")])
    );
    assert_eq!(s.running_ops(), 1);
}

#[test]
fn finished_copy_can_be_undone_and_redone() {
    let mut s = state();
    open(&mut s, r"C:\a", &[]);
    let fx = update(
        &mut s,
        Msg::DropPaths {
            dest: r"C:\a".into(),
            paths: vec![r"D:\x.txt".into()],
            copy: None,
        },
    );
    assert!(
        matches!(job(&fx), Job::File(FileOp::Copy { .. })),
        "autre volume : copie"
    );
    let op = op_id(&fx);
    update(
        &mut s,
        Msg::OpEvent {
            op,
            event: OpEvent::Created {
                source: Some(r"D:\x.txt".into()),
                created: r"C:\a\x.txt".into(),
            },
        },
    );
    update(
        &mut s,
        Msg::OpFinished {
            op,
            result: Ok(false),
        },
    );
    assert!(s.undo.can_undo());
    let fx = update(&mut s, Msg::Command(CommandId::Undo));
    assert!(
        matches!(job(&fx), Job::File(FileOp::Delete { items, permanent: false }) if items == &[PathBuf::from(r"C:\a\x.txt")])
    );
    let fx = update(&mut s, Msg::Command(CommandId::Redo));
    assert!(matches!(job(&fx), Job::File(FileOp::Copy { .. })));
}

#[test]
fn finished_ops_go_to_status_center_history_until_cleared() {
    let mut s = state();
    open(&mut s, r"C:\a", &[]);
    let start = |s: &mut AppState, name: &str| {
        op_id(&update(
            s,
            Msg::DropPaths {
                dest: r"C:\a".into(),
                paths: vec![PathBuf::from(format!(r"D:\{name}"))],
                copy: None,
            },
        ))
    };
    let (a, b) = (start(&mut s, "x.txt"), start(&mut s, "y.txt"));
    assert!(!s.command_context().has_op_history);
    let finish = |s: &mut AppState, op, result| {
        let saved: Vec<OpEntry> = update(s, Msg::OpFinished { op, result })
            .into_iter()
            .filter_map(|e| match e {
                Effect::Persist(Persist::AddOp(e)) => Some(e),
                _ => None,
            })
            .collect();
        assert_eq!(saved.len(), 1, "enregistrée dans la base");
        saved.into_iter().next().expect("entrée")
    };
    let saved_a = finish(&mut s, a, Ok(false));
    let saved_b = finish(&mut s, b, Err("accès refusé".into()));
    assert_eq!(saved_b.outcome, OpOutcome::Failed("accès refusé".into()));
    assert!(saved_b.ended > saved_a.ended, "(fin, titre) unique");
    assert!(s.ops.is_empty(), "plus rien en cours");
    assert!(s.op_history.has_unseen_failure(), "pastille d'échec");
    let hist: Vec<_> = s
        .op_history
        .iter()
        .map(|r| (r.id, r.status.clone()))
        .collect();
    assert_eq!(
        hist,
        vec![
            (b, OpStatus::Failed("accès refusé".into())),
            (a, OpStatus::Done)
        ],
        "la plus récente en premier"
    );

    let fx = update(&mut s, Msg::DismissOp(b));
    assert!(
        matches!(&fx[..], [Effect::Persist(Persist::RemoveOp { ended, title })] if *ended == saved_b.ended && *title == saved_b.title)
    );
    assert_eq!(s.op_history.last_id(), Some(a));
    assert!(s.command_context().has_op_history);
    assert!(CommandId::ClearOperationHistory.is_enabled(&s.command_context()));
    let fx = update(&mut s, Msg::Command(CommandId::ClearOperationHistory));
    assert!(matches!(&fx[..], [Effect::Persist(Persist::ClearOps)]));
    assert!(s.op_history.is_empty());
    assert!(!CommandId::ClearOperationHistory.is_enabled(&s.command_context()));
}

#[test]
fn status_center_excludes_search_and_palette() {
    let mut s = state();
    update(&mut s, Msg::Command(CommandId::Search));
    update(&mut s, Msg::Command(CommandId::StatusCenter));
    assert!(s.show_status_center && !s.show_search && !s.show_palette);
    update(&mut s, Msg::Command(CommandId::CommandPalette));
    assert!(!s.show_status_center && s.show_palette);
    update(&mut s, Msg::Command(CommandId::StatusCenter));
    update(&mut s, Msg::CloseOverlays);
    assert!(!s.show_status_center);
}

#[test]
fn drop_on_same_volume_moves_and_ignores_noops() {
    let mut s = state();
    open(&mut s, r"C:\a", &[]);
    let fx = update(
        &mut s,
        Msg::DropPaths {
            dest: r"C:\a".into(),
            paths: vec![r"C:\b\y.txt".into(), r"C:\a\z.txt".into()],
            copy: None,
        },
    );
    assert!(
        matches!(job(&fx), Job::File(FileOp::Move { sources, .. }) if sources == &[PathBuf::from(r"C:\b\y.txt")])
    );
    let fx = update(
        &mut s,
        Msg::DropPaths {
            dest: r"C:\a\sub".into(),
            paths: vec![r"C:\a".into()],
            copy: None,
        },
    );
    assert!(fx.is_empty(), "un dossier ne se déplace pas dans lui-même");
}

#[test]
fn search_builds_virtual_listing() {
    let mut s = state();
    open(&mut s, r"C:\root", &[]);
    let p = pane(&s);
    let fx = update(
        &mut s,
        Msg::Search {
            pane: p,
            query: "rap".into(),
        },
    );
    let generation = fx
        .iter()
        .find_map(|e| {
            if let Effect::Search { generation, .. } = e {
                Some(*generation)
            } else {
                None
            }
        })
        .expect("search");
    let mut b = batch(&[("rapport.pdf", false)]);
    b.dir = Some(PathBuf::from(r"C:\root\docs"));
    update(
        &mut s,
        Msg::ListingBatch {
            pane: p,
            generation,
            batch: b,
        },
    );
    update(
        &mut s,
        Msg::SearchFinished {
            pane: p,
            generation,
            stats: Some(fx_fs::SearchStats {
                matches: 1,
                dirs_scanned: 3,
                elapsed_ms: 1,
            }),
        },
    );
    let l = s.pane().listing.as_ref().expect("l");
    assert!(l.is_virtual());
    assert_eq!(
        crate::state::path_of(l, l.at(0).expect("0").id),
        PathBuf::from(r"C:\root\docs\rapport.pdf")
    );
    assert_eq!(
        s.pane().search_summary.as_deref(),
        Some("1 résultat dans 3 dossiers")
    );
    update(&mut s, Msg::Command(CommandId::Up));
    assert_eq!(s.pane().location(), &Location::Dir(r"C:\root".into()));
}

#[test]
fn watcher_changes_update_listing() {
    let mut s = state();
    open(&mut s, r"C:\a", &[("a.txt", false), ("b.txt", false)]);
    let p = pane(&s);
    update(
        &mut s,
        Msg::Click {
            pane: p,
            pos: 1,
            ctrl: false,
            shift: false,
        },
    );
    let key = p.0;
    update(
        &mut s,
        Msg::DirChanged(DirEvent {
            key,
            changes: vec![Change::Remove {
                name: "b.txt".encode_utf16().collect(),
            }],
        }),
    );
    assert_eq!(names(&s), ["a.txt"]);
    assert_eq!(s.pane().selection.count(), 0);
    let fx = update(
        &mut s,
        Msg::DirChanged(DirEvent {
            key,
            changes: vec![Change::Rescan],
        }),
    );
    assert!(fx.iter().any(|e| matches!(e, Effect::Enumerate { .. })));
}

#[test]
fn refresh_rereads_images_and_rewritten_preview() {
    let mut s = state();
    s.settings.show_preview = true;
    open(&mut s, r"C:\a", &[("a.png", false), ("b.png", false)]);
    let p = pane(&s);
    let fx = update(
        &mut s,
        Msg::Click {
            pane: p,
            pos: 0,
            ctrl: false,
            shift: false,
        },
    );
    let shown = PathBuf::from(r"C:\a\a.png");
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::LoadPreview(x) if *x == shown))
    );
    update(
        &mut s,
        Msg::PreviewLoaded(Some(fx_preview::Preview {
            path: shown.clone(),
            size: 10,
            created: 1,
            modified: 1,
            attributes: 0,
            content: fx_preview::Content::Image,
        })),
    );
    // Actualiser : liste relue, images et aperçu aussi.
    let fx = update(&mut s, Msg::Command(CommandId::Refresh));
    assert!(fx.iter().any(|e| matches!(e, Effect::Enumerate { .. })));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::LoadPreview(x) if *x == shown))
    );
    assert!(s.requests.refresh_images);
    update(&mut s, Msg::ConsumeRequests);
    assert!(!s.requests.refresh_images);

    // Fichier affiché réenregistré : l'aperçu est relu ; un autre fichier ne le relit pas.
    let rewritten = |name: &str, modified| {
        Msg::DirChanged(DirEvent {
            key: p.0,
            changes: vec![Change::Upsert {
                name: name.encode_utf16().collect(),
                stat: Stat {
                    attributes: 0,
                    size: 10,
                    created: 1,
                    modified,
                },
            }],
        })
    };
    let fx = update(&mut s, rewritten("b.png", 2));
    assert!(!fx.iter().any(|e| matches!(e, Effect::LoadPreview(_))));
    let fx = update(&mut s, rewritten("a.png", 1));
    assert!(!fx.iter().any(|e| matches!(e, Effect::LoadPreview(_))));
    let fx = update(&mut s, rewritten("a.png", 2));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::LoadPreview(x) if *x == shown))
    );
}

#[test]
fn tags_and_tag_location() {
    let mut s = state();
    open(&mut s, r"C:\a", &[]);
    update(
        &mut s,
        Msg::SetTag {
            paths: vec![r"C:\a\f.txt".into()],
            tag: 1,
            on: true,
        },
    );
    assert_eq!(s.tags.of(Path::new(r"C:\a\f.txt")), &[1]);
    let fx = update(&mut s, Msg::Navigate(Location::Tag(1)));
    assert!(fx.iter().any(
        |e| matches!(e, Effect::ListPaths { paths, .. } if paths == &[PathBuf::from(r"C:\a\f.txt")])
    ));
}

#[test]
fn layout_change_is_remembered_and_columns_open() {
    let mut s = state();
    open(&mut s, r"C:\a", &[("sub", true), ("f", false)]);
    update(&mut s, Msg::SetLayout(Layout::Columns));
    let p = pane(&s);
    let fx = update(
        &mut s,
        Msg::ColumnClick {
            pane: p,
            column: 0,
            pos: 0,
        },
    );
    assert!(fx.iter().any(|e| matches!(e, Effect::Enumerate { path, target: crate::effect::Target::Column, .. } if path == Path::new(r"C:\a\sub"))));
    assert_eq!(s.pane().columns.len(), 1);
    assert_eq!(s.pane().working_dir(), Some(Path::new(r"C:\a")));
    open(&mut s, r"C:\b", &[]);
    assert_eq!(s.pane().layout, Layout::Details);
    open(&mut s, r"C:\a", &[]);
    assert_eq!(s.pane().layout, Layout::Columns);
}

#[test]
fn session_roundtrip_restores_tabs() {
    let mut s = state();
    open(&mut s, r"C:\a", &[]);
    update(&mut s, Msg::Command(CommandId::ToggleDualPane));
    let session = s.session();
    let mut t = state();
    let fx = update(
        &mut t,
        Msg::Startup {
            start: None,
            session: Some(session),
        },
    );
    assert_eq!(t.tab().panes.len(), 2);
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::Enumerate { .. }))
            .count(),
        2
    );
}

#[test]
fn zoom_follows_explorer_ladder() {
    let mut s = state();
    update(&mut s, Msg::SetLayout(Layout::Details));
    update(&mut s, Msg::Command(CommandId::ZoomOut));
    assert_eq!(s.pane().layout, Layout::Details, "déjà au plus petit");
    update(&mut s, Msg::Command(CommandId::ZoomIn));
    assert_eq!(s.pane().layout, Layout::List);
    update(&mut s, Msg::Command(CommandId::ZoomIn));
    assert_eq!((s.pane().layout, s.settings.grid_size), (Layout::Grid, 48));
    for _ in 0..10 {
        update(&mut s, Msg::Command(CommandId::ZoomIn));
    }
    assert_eq!(s.settings.grid_size, 256);
    for _ in 0..6 {
        update(&mut s, Msg::Command(CommandId::ZoomOut));
    }
    assert_eq!(s.pane().layout, Layout::List);
}

#[test]
fn shell_locations_from_paths() {
    use crate::history::{NETWORK_ID, RECYCLE_BIN_ID};
    assert_eq!(
        Location::from_path(PathBuf::from(RECYCLE_BIN_ID)),
        Location::RecycleBin
    );
    assert_eq!(
        Location::from_path(PathBuf::from("shell:RecycleBinFolder")),
        Location::RecycleBin
    );
    assert_eq!(
        Location::from_path(PathBuf::from(NETWORK_ID)),
        Location::Network(None)
    );
    assert_eq!(
        Location::from_path(PathBuf::from(r"\\NAS\")),
        Location::Network(Some(r"\\NAS".into()))
    );
    assert_eq!(
        Location::from_path(PathBuf::from(r"\\NAS\photos")),
        Location::Dir(PathBuf::from(r"\\NAS\photos"))
    );
    assert_eq!(
        Location::from_path(PathBuf::from(r"C:\Users")),
        Location::Dir(PathBuf::from(r"C:\Users"))
    );
    // Aller-retour par la session.
    for loc in [
        Location::RecycleBin,
        Location::Network(None),
        Location::Network(Some(r"\\NAS".into())),
    ] {
        assert_eq!(
            Location::from_path(loc.session_path().expect("chemin")),
            loc
        );
    }
}

#[test]
fn back_and_forward_mark_a_new_navigation() {
    use crate::state::NavKind;
    let mut s = state();
    update(&mut s, Msg::Navigate(Location::Dir(r"C:\a".into())));
    update(&mut s, Msg::Navigate(Location::Dir(r"C:\b".into())));
    let before = s.pane().nav.serial;
    let fx = update(&mut s, Msg::Command(CommandId::Back));
    // Nouvelle génération chargée et marquée « Précédent » : l'interface anime l'arrivée.
    assert_eq!(s.pane().nav.kind, NavKind::Back);
    assert_eq!(s.pane().nav.serial, enum_gen(&fx));
    assert_ne!(s.pane().nav.serial, before);
    let fx = update(&mut s, Msg::Command(CommandId::Forward));
    assert_eq!(s.pane().nav.kind, NavKind::Forward);
    assert_eq!(s.pane().nav.serial, enum_gen(&fx));
    assert_eq!(s.pane().location(), &Location::Dir(r"C:\b".into()));
}

#[test]
fn history_survives_restart() {
    let mut s = state();
    update(&mut s, Msg::Navigate(Location::Dir(r"C:\a".into())));
    update(&mut s, Msg::Navigate(Location::Dir(r"C:\a\b".into())));
    update(
        &mut s,
        Msg::Navigate(Location::Search {
            root: r"C:\a\b".into(),
            query: "x".into(),
        }),
    );
    update(&mut s, Msg::Navigate(Location::Dir(r"C:\c".into())));
    update(&mut s, Msg::Command(CommandId::Back));
    let session = s.session();
    let pane = &session.tabs[0].left;
    // Précédent : accueil, a, b (la recherche est omise) ; Suivant : c.
    assert_eq!(
        pane.back,
        [
            PathBuf::from(crate::history::HOME_ID),
            PathBuf::from(r"C:\a"),
            PathBuf::from(r"C:\a\b"),
        ]
    );
    assert_eq!(pane.forward, [PathBuf::from(r"C:\c")]);

    let mut r = AppState::new(
        Settings {
            restore_session: true,
            ..Settings::default()
        },
        Tags::default(),
        Vec::new(),
    );
    update(
        &mut r,
        Msg::Startup {
            start: None,
            session: Some(session),
        },
    );
    assert!(r.pane().history.can_go_back() && r.pane().history.can_go_forward());
    update(&mut r, Msg::Command(CommandId::Back));
    assert_eq!(r.pane().location(), &Location::Dir(r"C:\a\b".into()));
    update(&mut r, Msg::Command(CommandId::Back));
    update(&mut r, Msg::Command(CommandId::Back));
    assert_eq!(r.pane().location(), &Location::Home);
}

fn recycled(original: &str, item: &str) -> fx_fs::recycle::Recycled {
    fx_fs::recycle::Recycled {
        item: PathBuf::from(item),
        info: PathBuf::from(item.replace("$R", "$I")),
        original: PathBuf::from(original),
        size: 10,
        deleted: 7,
        attributes: 0x20,
    }
}

#[test]
fn recycle_bin_lists_original_names_and_restores() {
    let mut s = state();
    let fx = update(&mut s, Msg::Navigate(Location::RecycleBin));
    let generation = fx
        .iter()
        .find_map(|e| match e {
            Effect::LoadRecycleBin { generation, .. } => Some(*generation),
            _ => None,
        })
        .expect("chargement de la corbeille");
    let p = pane(&s);
    let items = vec![
        recycled(r"C:\docs\rapport.txt", r"C:\$Recycle.Bin\S-1\$RAAA.txt"),
        recycled(r"D:\photos\vacances.jpg", r"D:\$Recycle.Bin\S-1\$RBBB.jpg"),
        recycled(r"C:\docs\notes.md", r"C:\$Recycle.Bin\S-1\$RCCC.md"),
    ];
    update(
        &mut s,
        Msg::RecycleBinLoaded {
            pane: p,
            generation,
            items,
        },
    );
    let pane_ref = s.pane();
    let l = pane_ref.listing.as_ref().expect("liste");
    assert_eq!(l.len(), 3);
    // Noms et dossiers d'origine affichés ; l'élément réel est retenu à part.
    let mut shown: Vec<PathBuf> = (0..3)
        .map(|i| crate::state::path_of(l, l.at(i).expect("entrée").id))
        .collect();
    shown.sort();
    assert_eq!(
        shown,
        vec![
            PathBuf::from(r"C:\docs\notes.md"),
            PathBuf::from(r"C:\docs\rapport.txt"),
            PathBuf::from(r"D:\photos\vacances.jpg"),
        ]
    );
    assert_eq!(pane_ref.recycled.len(), 3);

    // Restaurer la sélection : le travail porte sur l'élément réel.
    let pos = (0..3)
        .find(|&i| fx_core::wtf8::display(l.at(i).expect("entrée").name) == "vacances.jpg")
        .expect("position");
    update(
        &mut s,
        Msg::Click {
            pane: p,
            pos,
            ctrl: false,
            shift: false,
        },
    );
    let fx = update(&mut s, Msg::Command(CommandId::Restore));
    assert!(
        matches!(job(&fx), Job::RestoreRecycled(v) if v.len() == 1 && v[0].item == Path::new(r"D:\$Recycle.Bin\S-1\$RBBB.jpg"))
    );

    // Supprimer dans la corbeille : toujours définitif, après confirmation.
    let fx = update(&mut s, Msg::Command(CommandId::Delete));
    assert!(fx.is_empty());
    assert!(matches!(&s.dialog, Some(Dialog::ConfirmPurge { items }) if items.len() == 1));
    let fx = update(&mut s, Msg::ConfirmDialog);
    assert!(matches!(job(&fx), Job::PurgeRecycled(v) if v.len() == 1));
}

#[test]
fn properties_open_a_separate_window() {
    let mut s = state();
    let fx = update(&mut s, Msg::Properties(PathBuf::from(r"C:\Windows")));
    assert!(matches!(
        fx.as_slice(),
        [Effect::PropertiesWindow { paths, recycled: None }] if paths == &[PathBuf::from(r"C:\Windows")]
    ));
    assert!(
        s.dialog.is_none(),
        "plus de fenêtre modale dans la fenêtre principale"
    );
}

#[test]
fn update_check_install_and_close() {
    use crate::upgrade::Status;
    let mut s = state();
    let ready = Status::Ready {
        version: "9.0.0".into(),
        installer: PathBuf::from(r"C:\u\folder-setup-9.0.0-x64.exe"),
    };
    assert!(!s.command_context().update_ready);

    // Vérification automatique : différée, silencieuse.
    let fx = update(&mut s, Msg::CheckUpdate { force: false });
    assert!(matches!(
        fx[..],
        [Effect::CheckUpdate { force: false, delay }] if delay > Duration::ZERO
    ));
    assert_eq!(s.update, Status::Unknown);
    assert!(s.toasts.is_empty());

    // Version téléchargée : annoncée une fois, installable.
    let checked = |s: &mut AppState, status: &Status, fresh, force| {
        update(
            s,
            Msg::UpdateChecked {
                status: status.clone(),
                fresh,
                force,
            },
        )
    };
    checked(&mut s, &ready, true, false);
    assert_eq!(s.update, ready);
    assert_eq!(s.toasts.len(), 1);
    checked(&mut s, &ready, true, false);
    assert_eq!(s.toasts.len(), 1, "même version : pas de nouvelle annonce");
    assert!(s.command_context().update_ready);

    // Échec d'une vérification automatique : la mise à jour prête reste.
    checked(&mut s, &Status::Failed("délai dépassé".into()), true, false);
    assert_eq!(s.update, ready);

    // Demande explicite : annoncée, sans perdre l'installeur pendant la recherche.
    let fx = update(&mut s, Msg::Command(CommandId::CheckForUpdates));
    assert!(matches!(
        fx[..],
        [Effect::CheckUpdate {
            force: true,
            delay: Duration::ZERO
        }]
    ));
    assert_eq!(s.update, ready);
    checked(&mut s, &Status::Failed("délai dépassé".into()), true, true);
    assert!(matches!(s.update, Status::Failed(_)));
    checked(&mut s, &ready, false, false);

    let fx = update(&mut s, Msg::Command(CommandId::InstallUpdate));
    assert!(matches!(fx[..], [Effect::InstallUpdate]));
    assert!(!s.requests.close_window);
    update(&mut s, Msg::UpdateLaunched);
    assert!(s.requests.close_window);
}

#[test]
fn enabling_auto_update_checks_and_notes_open_settings() {
    let mut s = state();
    let mut off = s.settings.clone();
    off.auto_update = false;
    update(&mut s, Msg::UpdateSettings(Box::new(off.clone())));
    let mut on = off;
    on.auto_update = true;
    let fx = update(&mut s, Msg::UpdateSettings(Box::new(on)));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::CheckUpdate { force: false, .. }))
    );
    assert!(matches!(
        update(&mut s, Msg::Command(CommandId::ReleaseNotes))[..],
        [Effect::SettingsWindow { notes: true }]
    ));
}

#[test]
fn finished_rename_rereads_items_without_watch_notification() {
    // Lecteurs virtuels (Google Drive…) : aucune notification ; la fin de l'opération suffit.
    let mut s = state();
    open(&mut s, r"G:\a", &[("old.txt", false), ("z.txt", false)]);
    let p = pane(&s);
    let id = s
        .find_pane_mut(p)
        .and_then(|p| p.listing.as_mut())
        .and_then(|l| l.find(b"old.txt"))
        .expect("id");
    let p = pane(&s);
    let fx = update(
        &mut s,
        Msg::CommitRename {
            pane: p,
            id,
            name: "new.txt".into(),
        },
    );
    let op = op_id(&fx);
    let fx = update(
        &mut s,
        Msg::OpFinished {
            op,
            result: Ok(false),
        },
    );
    let paths = fx
        .iter()
        .find_map(|e| match e {
            Effect::Restat { paths } => Some(paths.clone()),
            _ => None,
        })
        .expect("relecture");
    assert_eq!(
        paths,
        [
            PathBuf::from(r"G:\a\old.txt"),
            PathBuf::from(r"G:\a\new.txt")
        ]
    );
    let stat = Stat {
        attributes: 0,
        size: 1,
        created: 0,
        modified: 0,
    };
    update(
        &mut s,
        Msg::PathsChanged {
            dir: r"g:\A".into(),
            changes: vec![
                Change::Remove {
                    name: "old.txt".encode_utf16().collect(),
                },
                Change::Upsert {
                    name: "new.txt".encode_utf16().collect(),
                    stat,
                },
            ],
        },
    );
    assert_eq!(names(&s), ["new.txt", "z.txt"]);
    assert_eq!(s.pane().selected_paths(), [PathBuf::from(r"G:\a\new.txt")]);
}

#[test]
fn finished_extraction_rereads_only_items_of_shown_folders() {
    let mut s = state();
    open(&mut s, r"C:\a", &[("x.zip", false)]);
    let p = pane(&s);
    let id = s
        .find_pane_mut(p)
        .and_then(|p| p.listing.as_mut())
        .and_then(|l| l.find(b"x.zip"))
        .expect("id");
    let p = pane(&s);
    s.find_pane_mut(p).expect("pane").selection.set(id, true);
    let fx = update(&mut s, Msg::Command(CommandId::ExtractHere));
    assert!(matches!(job(&fx), Job::Extract { dest, .. } if dest == Path::new(r"C:\a\x")));
    let op = op_id(&fx);
    let fx = update(
        &mut s,
        Msg::OpFinished {
            op,
            result: Ok(false),
        },
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Restat { paths } if paths == &[PathBuf::from(r"C:\a\x")]))
    );
    // Dossier qui n'est plus affiché : rien à relire.
    open(&mut s, r"C:\b", &[]);
    let fx = update(&mut s, Msg::Command(CommandId::NewFolder));
    let op = op_id(&fx);
    open(&mut s, r"C:\c", &[]);
    let fx = update(
        &mut s,
        Msg::OpFinished {
            op,
            result: Ok(false),
        },
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::Restat { .. })));
}

#[test]
fn git_menu_lists_branches_and_switches() {
    let mut s = state();
    open(&mut s, r"C:\repo", &[("a.txt", false)]);
    let p = pane(&s);
    assert!(!s.command_context().git_repo);
    assert!(update(&mut s, Msg::Command(CommandId::GitBranches)).is_empty());
    update(
        &mut s,
        Msg::GitBranch {
            pane: p,
            dir: r"C:\repo".into(),
            branch: Some("main".into()),
        },
    );
    assert!(s.command_context().git_repo);
    let fx = update(&mut s, Msg::Command(CommandId::GitBranches));
    assert!(
        matches!(&fx[..], [Effect::ListGitBranches { dir, .. }] if dir == Path::new(r"C:\repo"))
    );
    let branches = crate::git::GitBranches {
        current: Some("dev".into()),
        local: vec!["dev".into(), "main".into()],
        remote: vec!["origin/fix".into()],
    };
    update(
        &mut s,
        Msg::GitBranchesLoaded {
            pane: p,
            dir: r"C:\repo".into(),
            result: Ok(branches.clone()),
        },
    );
    let menu = s.git_menu.as_ref().expect("menu");
    assert_eq!(menu.branches, Some(Ok(branches)));
    assert_eq!(
        s.pane().git_branch.as_deref(),
        Some("dev"),
        "branche changée hors de folder"
    );
    // Branche déjà active : rien à faire.
    assert!(update(&mut s, Msg::SwitchGitBranch("dev".into())).is_empty());
    assert!(s.git_menu.is_none());
    update(&mut s, Msg::Command(CommandId::GitBranches));
    let fx = update(&mut s, Msg::SwitchGitBranch("fix".into()));
    assert!(matches!(&fx[..], [Effect::SwitchGitBranch { branch, .. }] if branch == "fix"));
    assert_eq!(s.pane().git_switching.as_deref(), Some("fix"));
    let fx = update(
        &mut s,
        Msg::GitSwitched {
            pane: p,
            dir: r"C:\repo".into(),
            branch: "fix".into(),
            result: Ok(()),
        },
    );
    assert!(s.pane().git_switching.is_none());
    assert_eq!(s.pane().git_branch.as_deref(), Some("fix"));
    enum_gen(&fx);
    // Échec (modifications locales) : message, branche inchangée.
    update(&mut s, Msg::Command(CommandId::GitBranches));
    update(&mut s, Msg::SwitchGitBranch("main".into()));
    update(
        &mut s,
        Msg::GitSwitched {
            pane: p,
            dir: r"C:\repo".into(),
            branch: "main".into(),
            result: Err("conflit".into()),
        },
    );
    assert_eq!(s.pane().git_branch.as_deref(), Some("fix"));
    assert!(s.toasts.last().is_some_and(|t| t.text.contains("conflit")));
    update(&mut s, Msg::Command(CommandId::GitBranches));
    update(&mut s, Msg::CloseOverlays);
    assert!(s.git_menu.is_none());
}

// --- Analyse de l'espace ---------------------------------------------------------------

fn disk_listing(dirs: &[&str], files: &[(&str, u64)]) -> fx_core::DirListing {
    use fx_core::disktree::{ListedDir, ListedFile};
    let mut l = fx_core::DirListing::default();
    for d in dirs {
        let name = l.names.push_bytes(d.as_bytes());
        l.dirs.push(ListedDir {
            name,
            modified: 0,
            attributes: attrs::DIRECTORY,
        });
    }
    for (f, size) in files {
        let name = l.names.push_bytes(f.as_bytes());
        l.files.push(ListedFile {
            name,
            size: *size,
            modified: 0,
            attributes: 0,
        });
    }
    l
}

/// C:\r : a\ (gros.iso 5000), f.bin 1000.
fn disk_tree() -> fx_core::DiskTree {
    let mut t = fx_core::DiskTree::new(PathBuf::from(r"C:\r"), 100);
    let a = t.insert(0, &disk_listing(&["a"], &[("f.bin", 1000)])).start;
    t.insert(a, &disk_listing(&[], &[("gros.iso", 5000)]));
    t.finish();
    t
}

fn scan_id(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|e| match e {
            Effect::ScanDisk { scan, .. } => Some(*scan),
            _ => None,
        })
        .unwrap_or_else(|| panic!("pas d'analyse dans {effects:?}"))
}

/// Analyse de C:\r terminée, liste du dossier racine chargée.
fn analyzed(s: &mut AppState, suggestions: Vec<fx_fs::cleanup::Suggestion>) {
    let fx = update(s, Msg::AnalyzeSpace(PathBuf::from(r"C:\r")));
    let scan = scan_id(&fx);
    let fx = update(
        s,
        Msg::DiskScanned {
            scan,
            result: Some(Box::new((disk_tree(), suggestions))),
        },
    );
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Enumerate {
            options: fx_fs::EnumOptions {
                show_hidden: true,
                allocated: true
            },
            ..
        }
    )));
    let g = enum_gen(&fx);
    feed(s, g, &[("f.bin", false), ("a", true)]);
    done(s, g);
}

#[test]
fn analyze_space_scans_then_lists_with_folder_sizes() {
    let mut s = state();
    let fx = update(&mut s, Msg::AnalyzeSpace(PathBuf::from(r"C:\r")));
    scan_id(&fx);
    // Pendant l'analyse : pas encore de liste.
    assert!(!fx.iter().any(|e| matches!(e, Effect::Enumerate { .. })));
    assert_eq!(s.pane().status, ListingStatus::Idle);
    s.disk_scans.clear();

    analyzed(&mut s, Vec::new());
    // Dossiers à la taille de leur contenu, du plus gros au plus petit.
    assert_eq!(names(&s), vec!["a", "f.bin"]);
    let l = s.pane().listing.as_ref().expect("liste");
    assert_eq!(l.at(0).expect("a").meta.size, 5000);
    assert_eq!(s.pane().layout, Layout::Details);

    // Ouvrir un sous-dossier reste dans l'analyse, sans la relancer.
    let fx = update(
        &mut s,
        Msg::Navigate(Location::Dir(PathBuf::from(r"C:\r\a"))),
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::ScanDisk { .. })));
    assert_eq!(
        s.pane().location(),
        &Location::DiskUsage {
            root: PathBuf::from(r"C:\r"),
            dir: PathBuf::from(r"C:\r\a")
        }
    );
    update(&mut s, Msg::Command(CommandId::Up));
    assert!(
        matches!(s.pane().location(), Location::DiskUsage { dir, .. } if dir == Path::new(r"C:\r"))
    );
    // Au-dessus de la racine, on quitte l'analyse.
    update(&mut s, Msg::Command(CommandId::Up));
    assert_eq!(s.pane().location(), &Location::Dir(PathBuf::from(r"C:\")));
    update(&mut s, Msg::Command(CommandId::Back));
    let p = pane(&s);
    update(&mut s, Msg::CloseDiskUsage(p));
    assert_eq!(s.pane().location(), &Location::Dir(PathBuf::from(r"C:\r")));
    // L'analyse reste disponible pour y revenir.
    assert_eq!(s.disk_scans.len(), 1);
}

#[test]
fn analyze_space_command_targets_the_selected_folder() {
    let mut s = state();
    open(&mut s, r"C:\x", &[("dossier", true), ("f.txt", false)]);
    let p = pane(&s);
    update(
        &mut s,
        Msg::SelectPositions {
            pane: p,
            positions: vec![0],
            extend: false,
        },
    );
    let fx = update(&mut s, Msg::Command(CommandId::AnalyzeSpace));
    assert!(
        fx.iter().any(
            |e| matches!(e, Effect::ScanDisk { root, .. } if root == Path::new(r"C:\x\dossier"))
        )
    );
}

#[test]
fn cleanup_asks_then_deletes_and_refreshes_the_analysis() {
    use fx_fs::cleanup::{CleanKind, CleanMode, Suggestion};
    let mut s = state();
    let sugg = Suggestion {
        kind: CleanKind::NodeModules,
        title: "Dépendances",
        detail: "",
        paths: vec![PathBuf::from(r"C:\r\a")],
        bytes: 5000,
        mode: CleanMode::Items,
        permanent: true,
    };
    analyzed(&mut s, vec![sugg]);
    let root = PathBuf::from(r"C:\r");
    assert!(
        update(
            &mut s,
            Msg::DiskClean {
                root: root.clone(),
                index: 0
            }
        )
        .is_empty()
    );
    assert!(
        matches!(&s.dialog, Some(Dialog::ConfirmClean { items, permanent: true, .. }) if items == &[PathBuf::from(r"C:\r\a")])
    );
    let fx = update(&mut s, Msg::ConfirmDialog);
    assert!(
        matches!(job(&fx), Job::File(FileOp::Delete { items, permanent: true }) if items == &[PathBuf::from(r"C:\r\a")])
    );
    let op = op_id(&fx);
    // Fin de la suppression : le dossier parent est relu dans l'analyse.
    let fx = update(
        &mut s,
        Msg::OpFinished {
            op,
            result: Ok(false),
        },
    );
    let (scan, known) = fx
        .iter()
        .find_map(|e| match e {
            Effect::RefreshDisk {
                scan, dir, known, ..
            } if dir == &root => Some((*scan, known.clone())),
            _ => None,
        })
        .expect("actualisation");
    assert_eq!(known, vec![b"a".to_vec()]);
    let refreshed = fx_fs::diskscan::Refreshed {
        listing: disk_listing(&[], &[("f.bin", 1000)]),
        fresh: Vec::new(),
    };
    let fx = update(
        &mut s,
        Msg::DiskRefreshed {
            scan,
            dir: root.clone(),
            result: Some(Box::new(refreshed)),
        },
    );
    // Liste relue, arbre et suggestions à jour.
    enum_gen(&fx);
    let d = &s.disk_scans[0];
    let tree = d.tree.as_ref().expect("arbre");
    assert_eq!(tree.size(0), 1000);
    assert!(tree.find(Path::new(r"C:\r\a")).is_none());
    assert!(d.suggestions.is_empty());
}

#[test]
fn cleaning_folder_contents_lists_them_first() {
    use fx_fs::cleanup::{CleanKind, CleanMode, Suggestion};
    let mut s = state();
    let sugg = Suggestion {
        kind: CleanKind::Temp,
        title: "Fichiers temporaires",
        detail: "",
        paths: vec![PathBuf::from(r"C:\r\a")],
        bytes: 5000,
        mode: CleanMode::Contents,
        permanent: true,
    };
    analyzed(&mut s, vec![sugg]);
    let fx = update(
        &mut s,
        Msg::DiskClean {
            root: PathBuf::from(r"C:\r"),
            index: 0,
        },
    );
    assert!(
        matches!(&fx[..], [Effect::ListContents { dirs, permanent: true, .. }] if dirs == &[PathBuf::from(r"C:\r\a")])
    );
    update(
        &mut s,
        Msg::CleanListed {
            title: "Fichiers temporaires",
            items: Vec::new(),
            bytes: 0,
            permanent: true,
        },
    );
    assert!(s.dialog.is_none(), "rien à supprimer : pas de confirmation");
    update(
        &mut s,
        Msg::CleanListed {
            title: "Fichiers temporaires",
            items: vec![PathBuf::from(r"C:\r\a\gros.iso")],
            bytes: 5000,
            permanent: true,
        },
    );
    assert!(matches!(
        &s.dialog,
        Some(Dialog::ConfirmClean { bytes: 5000, .. })
    ));
}

#[test]
fn analyzing_several_drives_opens_one_tab_each() {
    let mut s = state();
    update(&mut s, Msg::Command(CommandId::AnalyzeDrives));
    assert!(s.show_disk_picker);
    // Exclusif des autres fenêtres flottantes.
    update(&mut s, Msg::Command(CommandId::CommandPalette));
    assert!(!s.show_disk_picker && s.show_palette);
    update(&mut s, Msg::Command(CommandId::AnalyzeDrives));
    assert!(s.show_disk_picker && !s.show_palette);

    let fx = update(
        &mut s,
        Msg::AnalyzeDrives(vec![PathBuf::from(r"C:\"), PathBuf::from(r"D:\")]),
    );
    assert!(!s.show_disk_picker);
    let roots: Vec<&Path> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::ScanDisk { root, .. } => Some(root.as_path()),
            _ => None,
        })
        .collect();
    assert_eq!(roots, vec![Path::new(r"C:\"), Path::new(r"D:\")]);
    assert_eq!(s.tabs.len(), 2);
    // L'onglet courant (premier lecteur) reste affiché.
    assert_eq!(s.active_tab, 0);
    assert!(
        matches!(s.pane().location(), Location::DiskUsage { root, .. } if root == Path::new(r"C:\"))
    );
    assert!(matches!(
        s.tabs[1].active_pane().location(),
        Location::DiskUsage { root, .. } if root == Path::new(r"D:\")
    ));
    assert_eq!(s.disk_scans.len(), 2);
}

#[test]
fn drive_space_is_reloaded_during_and_after_operations() {
    let mut s = state();
    open(&mut s, r"C:\a", &[("gros.iso", false)]);
    let p = pane(&s);
    update(
        &mut s,
        Msg::SelectPositions {
            pane: p,
            positions: vec![0],
            extend: false,
        },
    );
    update(&mut s, Msg::Command(CommandId::DeletePermanently));
    let fx = update(&mut s, Msg::ConfirmDialog);
    let op = op_id(&fx);
    let loads = |fx: &[Effect]| {
        fx.iter()
            .filter(|e| matches!(e, Effect::LoadDrives))
            .count()
    };
    let progress = |s: &mut AppState, done| {
        update(
            s,
            Msg::OpEvent {
                op,
                event: OpEvent::Progress { done, total: 10 },
            },
        )
    };
    // Pendant l'opération : une relecture, puis plus rien avant une seconde.
    assert_eq!(loads(&progress(&mut s, 1)), 1);
    assert_eq!(loads(&progress(&mut s, 2)), 0);
    // À la fin : toujours.
    let fx = update(
        &mut s,
        Msg::OpFinished {
            op,
            result: Ok(false),
        },
    );
    assert_eq!(loads(&fx), 1);
    // Corbeille vidée : la place revient aux lecteurs.
    assert_eq!(loads(&update(&mut s, Msg::RecycleBinChanged)), 1);
}

#[test]
fn select_all_works_in_the_disk_analysis() {
    let mut s = state();
    analyzed(&mut s, Vec::new());
    assert!(CommandId::SelectAll.is_enabled(&s.command_context()));
    update(&mut s, Msg::Command(CommandId::SelectAll));
    assert_eq!(s.pane().selection.count(), 2);
    assert_eq!(s.pane().selected_paths().len(), 2);
    update(&mut s, Msg::Command(CommandId::InvertSelection));
    assert_eq!(s.pane().selection.count(), 0);
}
