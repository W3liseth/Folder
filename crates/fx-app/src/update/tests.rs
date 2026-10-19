use std::path::{Path, PathBuf};
use std::time::Duration;

use fx_commands::CommandId;
use fx_core::entry::attrs;
use fx_core::{Batch, SortDir, SortKey};
use fx_fs::{Change, DirEvent, EnumStats, Outcome};
use fx_store::{Layout, Settings};
use fx_win::fileop::{FileOp, OpEvent};
use fx_win::sys::Stat;

use crate::effect::Effect;
use crate::history::Location;
use crate::msg::Msg;
use crate::ops::Job;
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
