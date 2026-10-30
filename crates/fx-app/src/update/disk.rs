//! Analyse de l'espace : ouverture, résultats des workers, nettoyage, actualisation après
//! les opérations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use fx_core::disktree::NodeKind;
use fx_core::{Batch, CancelToken, DiskTree};
use fx_fs::cleanup::{CleanMode, Suggestion};
use fx_fs::diskscan::{Refreshed, ScanProgress};

use crate::disk::{DiskScan, DiskStatus, DiskTab, MAX_SCANS, is_drive_root, is_under};
use crate::effect::Effect;
use crate::history::Location;
use crate::state::{AppState, Dialog, PaneId, ToastKind};
use crate::update::nav;

fn same_path(a: &Path, b: &Path) -> bool {
    is_under(a, b) && is_under(b, a)
}

/// Seuil des fichiers gardés un par un : d'après l'espace occupé du lecteur quand `root`
/// en est la racine.
fn keep_from(state: &AppState, root: &Path) -> u64 {
    let used = is_drive_root(root)
        .then(|| {
            state
                .drives
                .iter()
                .find(|d| same_path(&d.root, root))
                .map(|d| d.total.saturating_sub(d.free))
        })
        .flatten();
    fx_fs::diskscan::keep_files_from(used)
}

/// Analyse non annulée dont la racine contient `path` (la plus profonde).
fn covering(state: &AppState, path: &Path) -> Option<usize> {
    state
        .disk_scans
        .iter()
        .enumerate()
        .filter(|(_, s)| s.status != DiskStatus::Cancelled && is_under(path, &s.root))
        .max_by_key(|(_, s)| s.root.components().count())
        .map(|(i, _)| i)
}

fn by_id(state: &mut AppState, scan: u64) -> Option<&mut DiskScan> {
    state.disk_scans.iter_mut().find(|s| s.id == scan)
}

/// Analyse de `root` (pour la vue).
pub fn scan_of<'a>(state: &'a AppState, root: &Path) -> Option<&'a DiskScan> {
    state.disk_scans.iter().find(|s| same_path(&s.root, root))
}

/// Ouvre l'analyse de `path` dans le volet actif.
pub fn analyze(state: &mut AppState, path: PathBuf) -> Vec<Effect> {
    let root =
        covering(state, &path).map_or_else(|| path.clone(), |i| state.disk_scans[i].root.clone());
    let pane = state.pane().id;
    let mut fx = nav::load(state, pane, Location::DiskUsage { root, dir: path }, true);
    if let Some(p) = state.find_pane_mut(pane) {
        p.disk_tab = DiskTab::Content;
    }
    fx.push(Effect::LoadDrives);
    fx
}

/// Analyse plusieurs lecteurs : le premier dans le volet actif, chacun des suivants dans
/// un nouvel onglet (à la fin) ; l'onglet courant reste affiché. Les analyses tournent en
/// parallèle (un coordinateur et 4 lecteurs chacune, le temps de l'analyse).
pub fn analyze_many(state: &mut AppState, roots: Vec<PathBuf>) -> Vec<Effect> {
    state.show_disk_picker = false;
    let mut roots = roots.into_iter();
    let Some(first) = roots.next() else {
        return vec![];
    };
    let mut fx = analyze(state, first);
    let current = state.active_tab;
    for root in roots {
        fx.extend(nav::open_tab(
            state,
            Location::DiskUsage {
                root: root.clone(),
                dir: root,
            },
        ));
    }
    state.active_tab = current;
    fx
}

/// Navigation vers un dossier depuis une analyse : on reste dans l'analyse tant qu'il est
/// sous sa racine (dossier ouvert, parent, fil d'Ariane, chemin saisi).
pub(crate) fn redirect(state: &AppState, pane: PaneId, loc: Location) -> Location {
    let Location::Dir(dir) = &loc else { return loc };
    match state.find_pane(pane).map(crate::Pane::location) {
        Some(Location::DiskUsage { root, .. }) if is_under(dir, root) => Location::DiskUsage {
            root: root.clone(),
            dir: dir.clone(),
        },
        _ => loc,
    }
}

/// S'assure qu'une analyse de `root` existe (relancée si elle avait été annulée).
pub(crate) fn ensure_scan(state: &mut AppState, root: &Path) -> Vec<Effect> {
    if scan_of(state, root).is_some_and(|s| s.status != DiskStatus::Cancelled) {
        return vec![];
    }
    state.disk_scans.retain(|s| !same_path(&s.root, root));
    start_scan(state, root.to_path_buf())
}

fn start_scan(state: &mut AppState, root: PathBuf) -> Vec<Effect> {
    let id = state.next_id();
    let cancel = CancelToken::new();
    let keep_from = keep_from(state, &root);
    state.disk_scans.push(DiskScan {
        id,
        root: root.clone(),
        started: Instant::now(),
        status: DiskStatus::Scanning,
        progress: ScanProgress::default(),
        tree: None,
        suggestions: Vec::new(),
        refreshing: 0,
        cancel: cancel.clone(),
    });
    evict(state);
    vec![Effect::ScanDisk {
        scan: id,
        root,
        keep_from,
        cancel,
    }]
}

/// Libère les analyses en trop, en commençant par la plus ancienne qu'aucun volet n'affiche.
fn evict(state: &mut AppState) {
    while state.disk_scans.len() > MAX_SCANS {
        let shown: Vec<PathBuf> = state
            .tabs
            .iter()
            .flat_map(|t| &t.panes)
            .filter_map(|p| match p.location() {
                Location::DiskUsage { root, .. } => Some(root.clone()),
                _ => None,
            })
            .collect();
        let last = state.disk_scans.len() - 1;
        let Some(i) = state.disk_scans[..last]
            .iter()
            .position(|s| !shown.iter().any(|r| same_path(r, &s.root)))
        else {
            break;
        };
        state.disk_scans.remove(i).cancel.cancel();
    }
}

/// Relit la liste des volets qui affichent l'analyse de `root`.
fn reload_panes(state: &mut AppState, root: &Path) -> Vec<Effect> {
    let panes: Vec<(PaneId, Location)> = state
        .tabs
        .iter()
        .flat_map(|t| &t.panes)
        .filter(
            |p| matches!(p.location(), Location::DiskUsage { root: r, .. } if same_path(r, root)),
        )
        .map(|p| (p.id, p.location().clone()))
        .collect();
    let mut fx = Vec::new();
    for (id, loc) in panes {
        fx.extend(nav::load(state, id, loc, false));
    }
    fx
}

pub fn progress(state: &mut AppState, scan: u64, progress: ScanProgress) -> Vec<Effect> {
    if let Some(s) = by_id(state, scan).filter(|s| s.is_scanning()) {
        s.progress = progress;
    }
    vec![]
}

pub fn scanned(
    state: &mut AppState,
    scan: u64,
    result: Option<Box<(DiskTree, Vec<Suggestion>)>>,
) -> Vec<Effect> {
    let Some(s) = by_id(state, scan) else {
        return vec![];
    };
    match result {
        Some(r) => {
            let (tree, suggestions) = *r;
            s.tree = Some(tree);
            s.suggestions = suggestions;
            s.status = DiskStatus::Done {
                elapsed: s.started.elapsed(),
            };
            s.progress = ScanProgress::default();
        }
        None => {
            s.status = DiskStatus::Cancelled;
            return vec![];
        }
    }
    let root = s.root.clone();
    reload_panes(state, &root)
}

pub fn set_tab(state: &mut AppState, pane: PaneId, tab: DiskTab) -> Vec<Effect> {
    if let Some(p) = state.find_pane_mut(pane) {
        p.disk_tab = tab;
    }
    vec![]
}

pub fn rescan(state: &mut AppState, root: &Path) -> Vec<Effect> {
    let Some(i) = state
        .disk_scans
        .iter()
        .position(|s| same_path(&s.root, root))
    else {
        return vec![];
    };
    let old = state.disk_scans.remove(i);
    old.cancel.cancel();
    let mut fx = start_scan(state, old.root.clone());
    fx.extend(reload_panes(state, &old.root));
    fx.push(Effect::LoadDrives);
    fx
}

pub fn cancel(state: &mut AppState, root: &Path) -> Vec<Effect> {
    if let Some(s) = state
        .disk_scans
        .iter_mut()
        .find(|s| same_path(&s.root, root) && s.is_scanning())
    {
        s.cancel.cancel();
        s.status = DiskStatus::Cancelled;
    }
    vec![]
}

/// Quitte l'analyse : le dossier regardé s'ouvre normalement.
pub fn close(state: &mut AppState, pane: PaneId) -> Vec<Effect> {
    let Some(Location::DiskUsage { dir, .. }) = state.find_pane(pane).map(|p| p.location().clone())
    else {
        return vec![];
    };
    nav::load_exact(state, pane, Location::Dir(dir), true)
}

pub fn clean(state: &mut AppState, root: &Path, index: usize) -> Vec<Effect> {
    let Some(s) = scan_of(state, root)
        .and_then(|s| s.suggestions.get(index))
        .cloned()
    else {
        return vec![];
    };
    match s.mode {
        CleanMode::RecycleBin => {
            state.dialog = Some(Dialog::ConfirmEmptyRecycleBin);
            vec![]
        }
        CleanMode::Items => {
            state.dialog = Some(Dialog::ConfirmClean {
                title: s.title,
                items: s.paths,
                bytes: s.bytes,
                permanent: s.permanent,
            });
            vec![]
        }
        CleanMode::Contents => vec![Effect::ListContents {
            title: s.title,
            dirs: s.paths,
            bytes: s.bytes,
            permanent: s.permanent,
        }],
    }
}

pub fn clean_listed(
    state: &mut AppState,
    title: &'static str,
    items: Vec<PathBuf>,
    bytes: u64,
    permanent: bool,
) -> Vec<Effect> {
    if items.is_empty() {
        state.toast(format!("{title} : rien à supprimer"), ToastKind::Info);
        return vec![];
    }
    state.dialog = Some(Dialog::ConfirmClean {
        title,
        items,
        bytes,
        permanent,
    });
    vec![]
}

/// Vue d'analyse : les dossiers d'un lot prennent la taille de tout leur contenu.
pub(crate) fn patch_batch(state: &AppState, pane: PaneId, batch: &mut Batch) {
    let Some(Location::DiskUsage { root, dir }) = state.find_pane(pane).map(crate::Pane::location)
    else {
        return;
    };
    let Some(tree) = scan_of(state, root).and_then(|s| s.tree.as_ref()) else {
        return;
    };
    let Some(node) = tree.find(dir) else { return };
    // Index des sous-dossiers (un dossier peut en compter des dizaines de milliers).
    let sizes: HashMap<Vec<u8>, u64> = tree
        .children(node)
        .filter(|&c| tree.kind(c) == NodeKind::Dir)
        .map(|c| (tree.name(c).to_ascii_lowercase(), tree.size(c)))
        .collect();
    for e in batch.entries.iter_mut().filter(|e| e.is_dir()) {
        let name = batch.names.get(e.name).to_ascii_lowercase();
        e.size = sizes.get(&name).copied().unwrap_or(0);
    }
}

/// Dossiers de la corbeille du lecteur de `root` (un par compte), s'ils sont analysés.
fn recycle_dirs(tree: &DiskTree, root: &Path) -> Vec<PathBuf> {
    let Some(drive) = root.components().next() else {
        return Vec::new();
    };
    let mut bin = drive.as_os_str().to_os_string();
    bin.push(r"\$Recycle.Bin");
    let bin = PathBuf::from(bin);
    tree.find(&bin).map_or_else(Vec::new, |n| {
        tree.children(n)
            .filter(|&c| tree.kind(c) == NodeKind::Dir)
            .map(|c| tree.path(c))
            .collect()
    })
}

fn refresh_effects(s: &mut DiskScan, dirs: Vec<PathBuf>) -> Vec<Effect> {
    let Some(tree) = &s.tree else { return vec![] };
    let mut fx = Vec::new();
    for dir in dirs {
        let Some(node) = tree.find(&dir).filter(|&n| tree.kind(n) == NodeKind::Dir) else {
            continue;
        };
        fx.push(Effect::RefreshDisk {
            scan: s.id,
            known: tree.subdir_names(node),
            keep_from: tree.keep_files_from(),
            dir,
        });
    }
    s.refreshing += fx.len() as u32;
    fx
}

/// Après une opération : relecture des dossiers analysés qu'elle a touchés (et de la
/// corbeille, qui a pu recevoir les éléments supprimés).
pub(crate) fn after_op(state: &mut AppState, touched: &[PathBuf]) -> Vec<Effect> {
    let mut fx = Vec::new();
    for s in &mut state.disk_scans {
        let Some(tree) = &s.tree else { continue };
        let mut dirs: Vec<PathBuf> = Vec::new();
        for p in touched.iter().filter(|p| is_under(p, &s.root)) {
            if let Some(parent) = p.parent().filter(|d| is_under(d, &s.root))
                && !dirs.iter().any(|d| same_path(d, parent))
            {
                dirs.push(parent.to_path_buf());
            }
        }
        if dirs.is_empty() {
            continue;
        }
        for d in recycle_dirs(tree, &s.root) {
            if !dirs.iter().any(|x| same_path(x, &d)) {
                dirs.push(d);
            }
        }
        fx.extend(refresh_effects(s, dirs));
    }
    fx
}

/// La corbeille a été vidée : ses dossiers sont relus dans chaque analyse.
pub(crate) fn recycle_changed(state: &mut AppState) -> Vec<Effect> {
    let mut fx = Vec::new();
    for s in &mut state.disk_scans {
        let Some(tree) = &s.tree else { continue };
        let dirs = recycle_dirs(tree, &s.root);
        fx.extend(refresh_effects(s, dirs));
    }
    fx
}

pub fn refreshed(
    state: &mut AppState,
    scan: u64,
    dir: &Path,
    result: Option<Box<Refreshed>>,
) -> Vec<Effect> {
    let Some(s) = by_id(state, scan) else {
        return vec![];
    };
    s.refreshing = s.refreshing.saturating_sub(1);
    if let (Some(r), Some(tree)) = (result, &mut s.tree)
        && let Some(node) = tree.find(dir)
    {
        tree.refresh(node, &r.listing, &r.fresh);
        s.update_suggestions();
    }
    if s.refreshing > 0 {
        return vec![];
    }
    let root = s.root.clone();
    let mut fx = reload_panes(state, &root);
    fx.push(Effect::LoadDrives);
    fx
}
