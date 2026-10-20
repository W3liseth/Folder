//! Opérations sur les fichiers, renommage, dialogues, réglages, étiquettes.

use std::path::{Path, PathBuf};

use fx_store::{Settings, TagId};
use fx_win::fileop::FileOp;

use crate::effect::{Effect, Persist};
use crate::history::Location;
use crate::ops::{Job, Operation};
use crate::state::{AppState, Dialog, PaneId, Renaming, ToastKind, path_of};
use crate::update::nav;

/// Caractères interdits dans un nom de fichier Windows.
const INVALID: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// Lance un travail long et l'ajoute au centre d'opérations.
pub(crate) fn start_job(state: &mut AppState, job: Job, record_undo: bool) -> Effect {
    let id = state.next_id();
    let op = Operation::new(id, job.clone(), record_undo);
    let cancel = op.cancel.clone();
    state.ops.push(op);
    Effect::Job {
        op: id,
        job,
        cancel,
    }
}

pub fn begin_rename(state: &mut AppState) -> Vec<Effect> {
    let pane = state.pane();
    let Some((l, sel)) = pane.active_listing() else {
        return vec![];
    };
    let Some(&id) = l.visible_ids().iter().find(|&&id| sel.contains(id)) else {
        return vec![];
    };
    let pane_id = pane.id;
    let serial = state.next_id();
    state.renaming = Some(Renaming {
        pane: pane_id,
        id,
        serial,
    });
    vec![]
}

pub fn commit_rename(state: &mut AppState, pane: PaneId, id: u32, name: String) -> Vec<Effect> {
    state.renaming = None;
    let name = name.trim().trim_end_matches('.').to_owned();
    let Some(p) = state.find_pane(pane) else {
        return vec![];
    };
    let Some((l, _)) = p.active_listing() else {
        return vec![];
    };
    let path = path_of(l, id);
    let old = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.is_empty() || name == old {
        return vec![];
    }
    if name.contains(INVALID) {
        state.toast(
            "Un nom de fichier ne peut pas contenir \\ / : * ? \" < > |",
            ToastKind::Error,
        );
        return vec![];
    }
    if let Some(parent) = path.parent() {
        state.pending_select = vec![parent.join(&name)];
    }
    vec![start_job(
        state,
        Job::File(FileOp::Rename {
            item: path,
            new_name: name,
        }),
        true,
    )]
}

/// Nom libre dans `listing` : « base », « base (2) »…
pub(crate) fn unique_name(state: &AppState, base: &str, ext: &str) -> String {
    let existing: std::collections::HashSet<String> = state
        .pane()
        .active_listing()
        .map(|(l, _)| {
            l.visible_ids()
                .iter()
                .map(|&id| fx_core::wtf8::display(l.view(id).name).to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let candidate = |n: u32| {
        if n == 1 {
            format!("{base}{ext}")
        } else {
            format!("{base} ({n}){ext}")
        }
    };
    (1..)
        .map(candidate)
        .find(|c| !existing.contains(&c.to_lowercase()))
        .unwrap_or_else(|| candidate(1))
}

pub fn new_item(state: &mut AppState, folder: bool) -> Vec<Effect> {
    let Some(dir) = state.pane().working_dir().map(Path::to_path_buf) else {
        return vec![];
    };
    let name = if folder {
        unique_name(state, "Nouveau dossier", "")
    } else {
        unique_name(state, "Nouveau document texte", ".txt")
    };
    state.pending_rename = Some(dir.join(&name));
    state.pending_select = vec![dir.join(&name)];
    let op = if folder {
        FileOp::NewFolder { parent: dir, name }
    } else {
        FileOp::NewFile { parent: dir, name }
    };
    vec![start_job(state, Job::File(op), true)]
}

pub fn delete(state: &mut AppState, permanent: bool) -> Vec<Effect> {
    // Corbeille : supprimer = supprimer définitivement, toujours après confirmation.
    if matches!(state.pane().location(), Location::RecycleBin) {
        let items = state.pane().selected_recycled();
        if !items.is_empty() {
            state.dialog = Some(Dialog::ConfirmPurge { items });
        }
        return vec![];
    }
    let items = state.pane().selected_paths();
    delete_items(state, items, permanent)
}

/// Supprime `items` (corbeille ou définitivement), après confirmation si les réglages la
/// demandent (toujours pour une suppression définitive).
pub fn delete_items(state: &mut AppState, items: Vec<PathBuf>, permanent: bool) -> Vec<Effect> {
    if items.is_empty() {
        return vec![];
    }
    if state.settings.confirm_delete || permanent {
        state.dialog = Some(Dialog::ConfirmDelete { items, permanent });
        return vec![];
    }
    vec![start_job(
        state,
        Job::File(FileOp::Delete { items, permanent }),
        false,
    )]
}

pub fn confirm_dialog(state: &mut AppState) -> Vec<Effect> {
    match state.dialog.take() {
        Some(Dialog::ConfirmDelete { items, permanent }) => {
            vec![start_job(
                state,
                Job::File(FileOp::Delete { items, permanent }),
                false,
            )]
        }
        Some(Dialog::ConfirmEmptyRecycleBin) => vec![Effect::EmptyRecycleBin],
        Some(Dialog::ConfirmPurge { items }) => {
            vec![start_job(state, Job::PurgeRecycled(items), false)]
        }
        _ => vec![],
    }
}

/// Corbeille : restaure la sélection, ou tous les éléments.
pub fn restore_recycled(state: &mut AppState, all: bool) -> Vec<Effect> {
    let p = state.pane();
    if !matches!(p.location(), Location::RecycleBin) {
        return vec![];
    }
    let items = if all {
        p.recycled.values().cloned().collect()
    } else {
        p.selected_recycled()
    };
    if items.is_empty() {
        return vec![];
    }
    vec![start_job(state, Job::RestoreRecycled(items), false)]
}

/// Ouvre la fenêtre Propriétés de folder (fenêtre séparée).
pub fn open_properties(
    _state: &mut AppState,
    paths: Vec<PathBuf>,
    recycled: Option<fx_fs::recycle::Recycled>,
) -> Vec<Effect> {
    if paths.is_empty() {
        return vec![];
    }
    vec![Effect::PropertiesWindow {
        paths,
        recycled: recycled.map(|r| (r.original, r.deleted)),
    }]
}

pub fn bulk_rename(state: &mut AppState, items: Vec<(PathBuf, String)>) -> Vec<Effect> {
    state.dialog = None;
    let items: Vec<(PathBuf, String)> = items
        .into_iter()
        .filter(|(p, n)| {
            !n.is_empty()
                && !n.contains(INVALID)
                && p.file_name().is_some_and(|o| o.to_string_lossy() != *n)
        })
        .collect();
    if items.is_empty() {
        return vec![];
    }
    state.pending_select = items
        .iter()
        .filter_map(|(p, n)| p.parent().map(|d| d.join(n)))
        .collect();
    vec![start_job(
        state,
        Job::File(FileOp::RenameMany { items }),
        true,
    )]
}

/// Même volume (même lettre de lecteur ou même partage) : déplacement par défaut.
fn same_volume(a: &Path, b: &Path) -> bool {
    let root = |p: &Path| {
        p.components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
    };
    root(a) == root(b)
}

/// Un dépôt sans touche de modification copie-t-il (plutôt que déplacer) ? Comme
/// l'Explorateur : déplacement si tout vient du même volume que `dest`, copie sinon.
pub fn drop_copies(sources: &[PathBuf], dest: &Path) -> bool {
    !sources.iter().all(|s| same_volume(s, dest))
}

pub fn drop_paths(
    state: &mut AppState,
    dest: PathBuf,
    paths: Vec<PathBuf>,
    copy: Option<bool>,
) -> Vec<Effect> {
    let sources: Vec<PathBuf> = paths
        .into_iter()
        .filter(|p| p != &dest && !dest.starts_with(p))
        .filter(|p| copy == Some(true) || !nav::is_child_of(p, &dest))
        .collect();
    if sources.is_empty() {
        return vec![];
    }
    let copy = copy.unwrap_or_else(|| drop_copies(&sources, &dest));
    state.pending_select = sources
        .iter()
        .filter_map(|s| s.file_name().map(|n| dest.join(n)))
        .collect();
    let op = if copy {
        FileOp::Copy { sources, dest }
    } else {
        FileOp::Move { sources, dest }
    };
    vec![start_job(state, Job::File(op), true)]
}

pub fn paste(state: &mut AppState, dest: PathBuf, files: Vec<PathBuf>, cut: bool) -> Vec<Effect> {
    if files.is_empty() {
        state.toast(
            "Le presse-papiers ne contient pas de fichiers",
            ToastKind::Info,
        );
        return vec![];
    }
    let fx = drop_paths(state, dest, files, Some(!cut));
    if cut {
        // Comme l'Explorateur : un « couper » ne se colle qu'une fois.
        state.clipboard_has_files = false;
    }
    fx
}

pub fn update_settings(state: &mut AppState, new: Settings) -> Vec<Effect> {
    let old = std::mem::replace(&mut state.settings, new);
    let s = &state.settings;
    let mut fx = vec![crate::update::persist_settings(state)];
    if old.keybindings != s.keybindings {
        state.keymap = fx_commands::KeyMap::new(&s.keybindings);
    }
    if old.show_preview && !s.show_preview {
        state.preview = None;
    }
    if old.show_hidden != s.show_hidden || old.folders_first != s.folders_first {
        // Recharge tous les dossiers affichés.
        let panes: Vec<_> = state
            .tabs
            .iter()
            .flat_map(|t| &t.panes)
            .filter(|p| matches!(p.location(), Location::Dir(_)))
            .map(|p| (p.id, p.location().clone()))
            .collect();
        for (id, loc) in panes {
            fx.extend(nav::load(state, id, loc, false));
        }
    }
    if !old.show_preview && state.settings.show_preview {
        fx.extend(nav::after_selection(state));
    }
    fx
}

pub fn set_tag(state: &mut AppState, paths: Vec<PathBuf>, tag: TagId, on: bool) -> Vec<Effect> {
    let mut fx = Vec::new();
    for path in paths {
        let mut ids = state.tags.of(&path).to_vec();
        if on && !ids.contains(&tag) {
            ids.push(tag);
        } else if !on {
            ids.retain(|&t| t != tag);
        }
        state.tags.set(&path, ids.clone());
        fx.push(Effect::Persist(Persist::Tags { path, tags: ids }));
    }
    fx
}
