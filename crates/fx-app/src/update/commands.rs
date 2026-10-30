//! Exécution des commandes du registre (`fx-commands`).

use std::path::{Path, PathBuf};

use fx_commands::CommandId;
use fx_core::{SortDir, SortKey, SortSpec};
use fx_store::{Layout, ThemeChoice};

use crate::effect::{Effect, Persist};
use crate::history::Location;
use crate::msg::Msg;
use crate::ops::Job;
use crate::state::{AppState, Dialog, ToastKind};
use crate::update::results::{first_target, paths_text};
use crate::update::{apply_sort, edit, nav, persist_settings, update};

pub fn run(state: &mut AppState, id: CommandId) -> Vec<Effect> {
    let ctx = state.command_context();
    if !id.is_enabled(&ctx) {
        return vec![];
    }
    let pane_id = state.pane().id;
    use CommandId::*;
    match id {
        Back => {
            let target = state.pane_mut().history.go_back().cloned();
            target.map_or_else(Vec::new, |loc| {
                nav::load_from_history(state, pane_id, loc, crate::state::NavKind::Back)
            })
        }
        Forward => {
            let target = state.pane_mut().history.go_forward().cloned();
            target.map_or_else(Vec::new, |loc| {
                nav::load_from_history(state, pane_id, loc, crate::state::NavKind::Forward)
            })
        }
        Up => {
            let target = match state.pane().location() {
                Location::Search { root, .. } => Some(Location::Dir(root.clone())),
                // Analyse : le parent reste dans l'analyse s'il est sous sa racine.
                Location::Dir(d) | Location::DiskUsage { dir: d, .. } => Some(
                    d.parent()
                        .map_or(Location::Home, |p| Location::from_path(p.to_path_buf())),
                ),
                Location::Network(Some(_)) => Some(Location::Network(None)),
                _ => None,
            };
            let fx = target.map_or_else(Vec::new, |loc| nav::load(state, pane_id, loc, true));
            mark_nav(state, crate::state::NavKind::Up);
            fx
        }
        Refresh => {
            let loc = state.pane().location().clone();
            let mut fx = nav::load(state, pane_id, loc, false);
            fx.push(Effect::LoadDrives);
            state.requests.refresh_images = true;
            // L'aperçu garde son contenu jusqu'à la relecture (pas de clignotement).
            if let Some(p) = &state.preview {
                fx.push(Effect::LoadPreview(p.path.clone()));
            }
            fx
        }
        Home => nav::load(state, pane_id, Location::Home, true),
        EditPath => {
            state.requests.focus_path = true;
            vec![]
        }
        // Recherche, palette et centre d'état : fenêtres flottantes exclusives.
        Search => {
            state.show_search = !state.show_search;
            state.show_palette = false;
            state.git_menu = None;
            state.show_status_center = false;
            state.show_disk_picker = false;
            vec![]
        }
        CommandPalette => {
            state.show_palette = !state.show_palette;
            state.show_search = false;
            state.git_menu = None;
            state.show_status_center = false;
            state.show_disk_picker = false;
            vec![]
        }
        StatusCenter => {
            state.show_status_center = !state.show_status_center;
            state.show_search = false;
            state.show_palette = false;
            state.git_menu = None;
            state.show_disk_picker = false;
            if state.show_status_center {
                state.op_history.mark_seen();
                // Les autres fenêtres ont pu compléter ou effacer l'historique.
                vec![Effect::Persist(Persist::LoadOps)]
            } else {
                vec![]
            }
        }
        GitBranches => {
            if state.git_menu.take().is_some() {
                return vec![];
            }
            let pane = state.pane();
            let (Some(_), Some(dir)) = (&pane.git_branch, pane.location().dir()) else {
                return vec![];
            };
            let (pane, dir) = (pane.id, dir.to_path_buf());
            state.show_palette = false;
            state.show_search = false;
            state.show_status_center = false;
            state.show_disk_picker = false;
            state.git_menu = Some(crate::state::GitMenu {
                pane,
                dir: dir.clone(),
                branches: None,
            });
            vec![Effect::ListGitBranches { pane, dir }]
        }
        ClearOperationHistory => {
            state.op_history.clear();
            vec![Effect::Persist(Persist::ClearOps)]
        }

        NewTab => nav::open_tab(state, Location::Home),
        DuplicateTab => {
            let loc = state.pane().location().clone();
            nav::open_tab(state, loc)
        }
        CloseTab => {
            let i = state.active_tab;
            nav::close_tab(state, i)
        }
        NextTab => {
            state.active_tab = (state.active_tab + 1) % state.tabs.len();
            vec![]
        }
        PreviousTab => {
            state.active_tab = (state.active_tab + state.tabs.len() - 1) % state.tabs.len();
            vec![]
        }
        ReopenClosedTab => match state.closed_tabs.pop() {
            Some(loc) => nav::open_tab(state, loc),
            None => vec![],
        },
        NewWindow => vec![Effect::NewWindow {
            start: Some(state.pane().location().window_arg()),
            rect: None,
        }],
        ToggleDualPane => {
            let loc = state.pane().location().clone();
            if state.tab().panes.len() > 1 {
                let tab = state.tab_mut();
                let removed = tab.panes.remove(1);
                tab.active = 0;
                if let Some(c) = &removed.cancel {
                    c.cancel();
                }
                vec![Effect::Unwatch { key: removed.id.0 }]
            } else {
                let pane = state.new_pane(Location::Home);
                let id = pane.id;
                let tab = state.tab_mut();
                tab.panes.push(pane);
                tab.active = 1;
                nav::load(state, id, loc, false)
            }
        }
        FocusOtherPane => {
            let tab = state.tab_mut();
            tab.active = (tab.active + 1) % tab.panes.len();
            nav::after_selection(state)
        }

        Open => {
            let pane = state.pane();
            let Some((l, sel)) = pane.active_listing() else {
                return vec![];
            };
            let picked: Vec<(PathBuf, bool)> = l
                .visible_ids()
                .iter()
                .filter(|&&id| sel.contains(id))
                .map(|&id| (crate::state::path_of(l, id), l.view(id).meta.is_dir()))
                .collect();
            let sort = l.sort();
            let mut fx = Vec::new();
            let mut dirs = picked.iter().filter(|(_, d)| *d).map(|(p, _)| p.clone());
            if let Some(first) = dirs.next() {
                fx.extend(nav::load(state, pane_id, Location::Dir(first), true));
            }
            for d in dirs {
                fx.extend(nav::open_tab(state, Location::Dir(d)));
            }
            fx.extend(
                picked
                    .into_iter()
                    .filter(|(_, d)| !d)
                    .map(|(p, _)| nav::open_file(p, sort)),
            );
            fx
        }
        OpenInNewTab => {
            let targets: Vec<PathBuf> = state.pane().selected_paths();
            let mut fx = Vec::new();
            for p in targets {
                fx.extend(nav::open_tab(state, Location::Dir(p)));
            }
            fx
        }
        OpenInOtherPane => {
            let Some(target) = state.pane().selected_paths().into_iter().next() else {
                return vec![];
            };
            let tab = state.tab();
            let other = tab.panes[(tab.active + 1) % tab.panes.len()].id;
            nav::load(state, other, Location::Dir(target), true)
        }
        OpenWith => first_target(state)
            .map(Effect::OpenWith)
            .into_iter()
            .collect(),
        RunAsAdmin => first_target(state)
            .map(Effect::RunAsAdmin)
            .into_iter()
            .collect(),
        Properties => {
            let p = state.pane();
            if matches!(p.location(), Location::RecycleBin) {
                let items = p.selected_recycled();
                let paths = items.iter().map(|r| r.item.clone()).collect();
                let single = (items.len() == 1).then(|| items[0].clone());
                return edit::open_properties(state, paths, single);
            }
            let mut paths = p.selected_paths();
            if paths.is_empty()
                && let Some(d) = p.working_dir()
            {
                paths.push(d.to_path_buf());
            }
            edit::open_properties(state, paths, None)
        }
        Restore => edit::restore_recycled(state, false),
        OpenTerminal => {
            let sel = state.pane().selected_paths();
            let dir = match sel.as_slice() {
                [one]
                    if state.pane().active_listing().is_some_and(|(l, s)| {
                        l.visible_ids()
                            .iter()
                            .any(|&id| s.contains(id) && l.view(id).meta.is_dir())
                    }) =>
                {
                    Some(one.clone())
                }
                _ => state.pane().working_dir().map(Path::to_path_buf),
            };
            dir.map(Effect::OpenTerminal).into_iter().collect()
        }
        Cut | Copy => {
            let paths = state.pane().selected_paths();
            let n = paths.len();
            state.clipboard_has_files = true;
            let verb = if id == Cut { "coupé" } else { "copié" };
            state.toast(
                format!(
                    "{n} élément{} {verb}{}",
                    if n > 1 { "s" } else { "" },
                    if n > 1 { "s" } else { "" }
                ),
                ToastKind::Info,
            );
            vec![Effect::SetClipboard {
                paths,
                cut: id == Cut,
            }]
        }
        Paste => state
            .pane()
            .working_dir()
            .map(|d| Effect::ReadClipboard {
                dest: d.to_path_buf(),
            })
            .into_iter()
            .collect(),
        CopyPath => match paths_text(state) {
            Some(t) => {
                state.toast("Chemin copié", ToastKind::Info);
                vec![Effect::CopyText(t)]
            }
            None => vec![],
        },
        Rename => edit::begin_rename(state),
        BulkRename => {
            let items = state.pane().selected_paths();
            state.dialog = Some(Dialog::BulkRename { items });
            vec![]
        }
        Delete => edit::delete(state, false),
        DeletePermanently => edit::delete(state, true),
        NewFolder => edit::new_item(state, true),
        NewFile => edit::new_item(state, false),
        PinToSidebar => {
            let mut targets = state.pane().selected_paths();
            if targets.is_empty() {
                targets.extend(state.pane().working_dir().map(Path::to_path_buf));
            }
            let mut fx = Vec::new();
            for t in targets {
                fx.extend(update(state, Msg::Pin(t)));
            }
            fx
        }
        CompressToZip => {
            let sources = state.pane().selected_paths();
            let Some(dir) = state.pane().working_dir().map(Path::to_path_buf) else {
                return vec![];
            };
            let base = match sources.as_slice() {
                [one] => one
                    .file_stem()
                    .map_or_else(|| "Archive".into(), |s| s.to_string_lossy().into_owned()),
                _ => "Archive".into(),
            };
            let name = edit::unique_name(state, &base, ".zip");
            let dest = dir.join(&name);
            state.pending_select = vec![dest.clone()];
            vec![edit::start_job(
                state,
                Job::Compress { sources, dest },
                true,
            )]
        }
        ExtractHere => {
            let archives: Vec<PathBuf> = state
                .pane()
                .selected_paths()
                .into_iter()
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")))
                .collect();
            let mut fx = Vec::new();
            for archive in archives {
                let Some(parent) = archive.parent() else {
                    continue;
                };
                let stem = archive
                    .file_stem()
                    .map_or_else(|| "Archive".into(), |s| s.to_string_lossy().into_owned());
                let dest = parent.join(edit::unique_name(state, &stem, ""));
                state.pending_select.push(dest.clone());
                fx.push(edit::start_job(
                    state,
                    Job::Extract { archive, dest },
                    false,
                ));
            }
            fx
        }
        EmptyRecycleBin => {
            state.dialog = Some(Dialog::ConfirmEmptyRecycleBin);
            vec![]
        }
        // Choix des lecteurs (bouton à côté de la palette) : exclusif des autres fenêtres
        // flottantes.
        AnalyzeDrives => {
            state.show_disk_picker = !state.show_disk_picker;
            state.show_search = false;
            state.show_palette = false;
            state.show_status_center = false;
            state.git_menu = None;
            if state.show_disk_picker {
                // Espace libre à jour dans la liste.
                vec![Effect::LoadDrives]
            } else {
                vec![]
            }
        }
        // Dossier sélectionné, sinon dossier affiché, sinon lecteur système.
        AnalyzeSpace => {
            let p = state.pane();
            let target = p
                .selected_dir()
                .or_else(|| p.location().dir().map(Path::to_path_buf))
                .or_else(|| {
                    let sys = std::env::var_os("SystemDrive").map(|mut d| {
                        d.push("\\");
                        PathBuf::from(d)
                    });
                    sys.or_else(|| state.drives.first().map(|d| d.root.clone()))
                });
            target.map_or_else(Vec::new, |t| super::disk::analyze(state, t))
        }
        Undo => match state.undo.pop_undo() {
            Some(e) => {
                state.toast(format!("Annulation : {}", e.label), ToastKind::Info);
                e.backward
                    .into_iter()
                    .map(|job| edit::start_job(state, job, false))
                    .collect()
            }
            None => vec![],
        },
        Redo => match state.undo.pop_redo() {
            Some(e) => vec![edit::start_job(state, e.forward, true)],
            None => vec![],
        },
        ShowMoreOptions => {
            state.requests.shell_menu = true;
            vec![]
        }

        SelectAll => {
            let n = state.pane().active_listing().map_or(0, |(l, _)| l.len());
            let positions: Vec<usize> = (0..n).collect();
            nav::select_positions(state, pane_id, &positions, false);
            nav::after_selection(state)
        }
        InvertSelection => {
            let p = state.pane();
            let Some((l, sel)) = p.active_listing() else {
                return vec![];
            };
            let positions: Vec<usize> = (0..l.len())
                .filter(|&i| l.at(i).is_some_and(|e| !sel.contains(e.id)))
                .collect();
            nav::select_positions(state, pane_id, &positions, false);
            nav::after_selection(state)
        }
        ClearSelection => {
            nav::select_positions(state, pane_id, &[], false);
            state.renaming = None;
            nav::after_selection(state)
        }

        LayoutDetails => update(state, Msg::SetLayout(Layout::Details)),
        LayoutList => update(state, Msg::SetLayout(Layout::List)),
        LayoutGrid => update(state, Msg::SetLayout(Layout::Grid)),
        LayoutColumns => update(state, Msg::SetLayout(Layout::Columns)),
        ZoomIn | ZoomOut => zoom(state, id == ZoomIn),
        SortByName => update(state, Msg::SortBy(SortKey::Name)),
        SortByModified => update(state, Msg::SortBy(SortKey::Modified)),
        SortByType => update(state, Msg::SortBy(SortKey::Type)),
        SortBySize => update(state, Msg::SortBy(SortKey::Size)),
        ToggleSortDirection => {
            let cur = current_sort(state);
            let dir = if cur.dir == SortDir::Asc {
                SortDir::Desc
            } else {
                SortDir::Asc
            };
            apply_sort(state, SortSpec { dir, ..cur })
        }
        ToggleFoldersFirst => {
            let mut s = state.settings.clone();
            s.folders_first = !s.folders_first;
            edit::update_settings(state, s)
        }
        ToggleHidden => {
            let mut s = state.settings.clone();
            s.show_hidden = !s.show_hidden;
            let on = s.show_hidden;
            let fx = edit::update_settings(state, s);
            state.toast(
                if on {
                    "Éléments masqués affichés"
                } else {
                    "Éléments masqués cachés"
                },
                ToastKind::Info,
            );
            fx
        }
        ToggleExtensions => {
            state.settings.show_extensions = !state.settings.show_extensions;
            vec![persist_settings(state)]
        }
        ToggleSidebar => {
            state.settings.show_sidebar = !state.settings.show_sidebar;
            vec![persist_settings(state)]
        }
        TogglePreview => {
            let mut s = state.settings.clone();
            s.show_preview = !s.show_preview;
            edit::update_settings(state, s)
        }

        Settings => vec![Effect::SettingsWindow { notes: false }],
        ReleaseNotes => vec![Effect::SettingsWindow { notes: true }],
        CheckForUpdates => super::upgrade::check(state, true),
        InstallUpdate => super::upgrade::install(state),
        ToggleTheme => {
            state.settings.theme = match state.settings.theme {
                ThemeChoice::Dark => ThemeChoice::Light,
                ThemeChoice::Light => ThemeChoice::Dark,
                // Le thème système n'est pas connu ici : l'UI résout « Système » en clair/sombre
                // avant d'envoyer la commande ; par défaut on passe en sombre.
                ThemeChoice::System => ThemeChoice::Dark,
            };
            vec![persist_settings(state)]
        }
        FullScreen => {
            state.requests.toggle_fullscreen = true;
            vec![]
        }
        Diagnostics => {
            state.show_diagnostics = !state.show_diagnostics;
            vec![]
        }
    }
}

fn current_sort(state: &AppState) -> SortSpec {
    let p = state.pane();
    p.listing
        .as_ref()
        .map_or_else(|| state.sort_for(p.location()), |l| l.sort())
}

/// Tailles d'icônes de la vue Grille, de la plus petite à la plus grande.
const GRID_SIZES: [u16; 6] = [48, 64, 96, 128, 192, 256];

/// Échelle de l'Explorateur (Ctrl + molette, Ctrl + Plus/Moins) : Détails < Liste < icônes
/// de 48 à 256 points. La vue Colonnes se place au niveau de Détails.
fn zoom(state: &mut AppState, bigger: bool) -> Vec<Effect> {
    let rung = match state.pane().layout {
        Layout::Details | Layout::Columns => 0,
        Layout::List => 1,
        Layout::Grid => {
            let cur = state.settings.grid_size;
            2 + GRID_SIZES
                .iter()
                .position(|&s| s >= cur)
                .unwrap_or(GRID_SIZES.len() - 1)
        }
    };
    let next = if bigger {
        (rung + 1).min(GRID_SIZES.len() + 1)
    } else {
        rung.saturating_sub(1)
    };
    match next {
        _ if next == rung => vec![],
        0 => update(state, Msg::SetLayout(Layout::Details)),
        1 => update(state, Msg::SetLayout(Layout::List)),
        n => {
            let mut fx = update(state, Msg::SetGridSize(GRID_SIZES[n - 2]));
            if state.pane().layout != Layout::Grid {
                fx.extend(update(state, Msg::SetLayout(Layout::Grid)));
            }
            fx
        }
    }
}

/// Précise le sens de la navigation qui vient d'avoir lieu dans le volet actif (animation).
fn mark_nav(state: &mut AppState, kind: crate::state::NavKind) {
    state.pane_mut().nav.kind = kind;
}
