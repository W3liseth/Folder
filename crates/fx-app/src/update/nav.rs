//! Navigation, onglets, sélection.

use std::path::{Path, PathBuf};
use std::time::Instant;

use fx_core::{CancelToken, DirSnapshot, SortDir, SortKey};
use fx_fs::EnumOptions;
use fx_store::{Layout, Session};

use crate::effect::{Effect, Target};
use crate::history::{History, Location};
use crate::state::{AppState, Column, ListingStatus, Pane, PaneId, PreviewState, path_of};

/// Nombre maximal d'onglets fermés mémorisés.
const MAX_CLOSED_TABS: usize = 20;

pub fn startup(
    state: &mut AppState,
    start: Option<PathBuf>,
    session: Option<Session>,
) -> Vec<Effect> {
    let mut fx = vec![
        Effect::LoadDrives,
        Effect::LoadKnownFolders,
        Effect::LoadRecent,
        Effect::LoadWsl,
        Effect::CheckClipboard,
    ];
    let restore =
        session.filter(|s| state.settings.restore_session && !s.tabs.is_empty() && start.is_none());
    match restore {
        Some(sess) => {
            state.tabs.clear();
            for t in &sess.tabs {
                let loc = t
                    .left
                    .path
                    .clone()
                    .map_or(Location::Home, Location::from_path);
                let mut tab = state.new_tab(loc.clone());
                tab.panes[0].layout = t.left.layout;
                tab.panes[0].history = History::restored(loc, &t.left.back, &t.left.forward);
                if let Some(r) = &t.right {
                    let loc = r.path.clone().map_or(Location::Home, Location::from_path);
                    let mut p = state.new_pane(loc.clone());
                    p.layout = r.layout;
                    p.history = History::restored(loc, &r.back, &r.forward);
                    tab.panes.push(p);
                }
                state.tabs.push(tab);
            }
            state.active_tab = sess.active.min(state.tabs.len() - 1);
            // Charge le contenu de tous les volets (les onglets inactifs aussi : bascule instantanée).
            let panes: Vec<(PaneId, Location)> = state
                .tabs
                .iter()
                .flat_map(|t| &t.panes)
                .map(|p| (p.id, p.location().clone()))
                .collect();
            for (id, loc) in panes {
                fx.extend(load(state, id, loc, false));
            }
        }
        None => {
            let id = state.pane().id;
            let loc = start.map_or(Location::Home, Location::Dir);
            fx.extend(load(state, id, loc, false));
        }
    }
    fx
}

/// Charge `loc` dans le volet `pane_id` en annulant le chargement précédent.
pub fn load(
    state: &mut AppState,
    pane_id: PaneId,
    loc: Location,
    push_history: bool,
) -> Vec<Effect> {
    load_with(state, pane_id, loc, push_history, None)
}

/// Charge l'emplacement sur lequel l'historique vient de se déplacer (Précédent, Suivant) :
/// l'historique désigne déjà `loc`, la navigation est marquée explicitement (animation).
pub fn load_from_history(
    state: &mut AppState,
    pane_id: PaneId,
    loc: Location,
    kind: crate::state::NavKind,
) -> Vec<Effect> {
    load_with(state, pane_id, loc, false, Some(kind))
}

fn load_with(
    state: &mut AppState,
    pane_id: PaneId,
    loc: Location,
    push_history: bool,
    moved: Option<crate::state::NavKind>,
) -> Vec<Effect> {
    let generation = state.next_id();
    let sort = state.sort_for(&loc);
    let prefs_layout = loc
        .dir()
        .filter(|_| matches!(loc, Location::Dir(_)))
        .and_then(|d| state.folder_prefs.get(&d.to_string_lossy().to_lowercase()))
        .map(|p| p.layout);
    let default_layout = state.settings.default_layout;
    let show_hidden = state.settings.show_hidden;
    let tagged = match &loc {
        Location::Tag(t) => state.tags.paths_with(*t),
        _ => Vec::new(),
    };
    if state.renaming.is_some_and(|r| r.pane == pane_id) {
        state.renaming = None;
    }
    let Some(pane) = state.find_pane_mut(pane_id) else {
        return vec![];
    };
    if let Some(c) = pane.cancel.take() {
        c.cancel();
    }
    // Précédent/Suivant : l'historique a déjà changé d'emplacement (`moved`).
    let changed_dir = moved.is_some() || pane.location() != &loc;
    if push_history {
        pane.history.visit(loc.clone());
    } else {
        pane.history.replace(loc.clone());
    }
    if changed_dir {
        pane.nav = crate::state::NavMark {
            serial: generation,
            kind: match moved {
                Some(kind) => kind,
                None if push_history => crate::state::NavKind::Open,
                None => crate::state::NavKind::Other,
            },
        };
        pane.filter.clear();
        pane.columns.clear();
        pane.active_column = 0;
        pane.git_branch = None;
        if let Location::Dir(_) = loc {
            pane.layout = prefs_layout.unwrap_or(default_layout);
        }
    }
    pane.search_summary = None;
    pane.first_batch_after = None;
    pane.recycled.clear();
    pane.shell_items.clear();
    let key = pane.id.0;
    let cancel = CancelToken::new();
    let mut fx = Vec::new();
    match &loc {
        Location::Home => {
            pane.listing = None;
            pane.incoming = None;
            pane.selection.clear();
            pane.status = ListingStatus::Idle;
            fx.push(Effect::Unwatch { key });
            fx.push(Effect::LoadDrives);
            fx.push(Effect::LoadRecent);
        }
        Location::Dir(path) => {
            pane.incoming = Some(DirSnapshot::new(path.clone(), generation, sort));
            pane.status = ListingStatus::Loading {
                started: Instant::now(),
            };
            pane.cancel = Some(cancel.clone());
            fx.push(Effect::Enumerate {
                pane: pane_id,
                generation,
                path: path.clone(),
                options: EnumOptions { show_hidden },
                cancel,
                target: Target::Pane,
            });
            fx.push(Effect::Watch {
                key,
                path: path.clone(),
            });
            fx.push(Effect::DetectGit {
                pane: pane_id,
                dir: path.clone(),
            });
        }
        Location::Search { root, query } => {
            let mut snap = DirSnapshot::new(root.clone(), generation, sort);
            snap.set_filter(None);
            pane.incoming = Some(snap);
            pane.status = ListingStatus::Loading {
                started: Instant::now(),
            };
            pane.cancel = Some(cancel.clone());
            pane.layout = Layout::Details;
            fx.push(Effect::Unwatch { key });
            fx.push(Effect::Search {
                pane: pane_id,
                generation,
                root: root.clone(),
                query: query.clone(),
                show_hidden,
                cancel,
            });
        }
        Location::RecycleBin => {
            pane.incoming = Some(DirSnapshot::new(PathBuf::new(), generation, sort));
            pane.status = ListingStatus::Loading {
                started: Instant::now(),
            };
            pane.layout = Layout::Details;
            fx.push(Effect::Unwatch { key });
            fx.push(Effect::LoadRecycleBin {
                pane: pane_id,
                generation,
            });
        }
        Location::Network(server) => {
            // Découverte parfois longue (plusieurs secondes) : pas d'ancienne liste affichée
            // sous le titre « Réseau » en attendant.
            pane.listing = None;
            pane.selection.clear();
            pane.incoming = Some(DirSnapshot::new(PathBuf::new(), generation, sort));
            pane.status = ListingStatus::Loading {
                started: Instant::now(),
            };
            fx.push(Effect::Unwatch { key });
            fx.push(Effect::LoadNetwork {
                pane: pane_id,
                generation,
                parsing: server
                    .clone()
                    .unwrap_or_else(|| crate::history::NETWORK_ID.to_owned()),
            });
        }
        Location::Tag(_) => {
            pane.incoming = Some(DirSnapshot::new(PathBuf::new(), generation, sort));
            pane.status = ListingStatus::Loading {
                started: Instant::now(),
            };
            pane.layout = Layout::Details;
            fx.push(Effect::Unwatch { key });
            fx.push(Effect::ListPaths {
                pane: pane_id,
                generation,
                paths: tagged,
            });
        }
    }
    fx
}

pub fn open_tab(state: &mut AppState, loc: Location) -> Vec<Effect> {
    let tab = state.new_tab(Location::Home);
    let pane = tab.panes[0].id;
    state.tabs.push(tab);
    state.active_tab = state.tabs.len() - 1;
    load(state, pane, loc, false)
}

pub fn close_tab(state: &mut AppState, i: usize) -> Vec<Effect> {
    if i >= state.tabs.len() {
        return vec![];
    }
    if state.tabs.len() == 1 {
        // Le dernier onglet revient à l'accueil plutôt que de fermer la fenêtre.
        let id = state.pane().id;
        return load(state, id, Location::Home, true);
    }
    let tab = state.tabs.remove(i);
    let mut fx = Vec::new();
    for p in &tab.panes {
        if let Some(c) = &p.cancel {
            c.cancel();
        }
        fx.push(Effect::Unwatch { key: p.id.0 });
    }
    state.closed_tabs.push(tab.active_pane().location().clone());
    if state.closed_tabs.len() > MAX_CLOSED_TABS {
        state.closed_tabs.remove(0);
    }
    if state.active_tab >= i && state.active_tab > 0 {
        state.active_tab -= 1;
    }
    fx
}

/// Ouvre un chemin : navigation pour un dossier, application par défaut pour un fichier.
pub fn open_path(state: &mut AppState, path: PathBuf) -> Vec<Effect> {
    // Corbeille, Réseau, ordinateur du réseau : emplacements affichés dans fx.
    let loc = Location::from_path(path.clone());
    if !matches!(loc, Location::Dir(_)) {
        let id = state.pane().id;
        return load(state, id, loc, true);
    }
    let is_dir_hint = path.extension().is_none() && !path.to_string_lossy().ends_with(".lnk");
    if is_dir_hint && path.is_absolute() {
        let id = state.pane().id;
        // Le dossier est vérifié par l'énumération (emplacement indisponible sinon).
        return load(state, id, Location::Dir(path), true);
    }
    vec![Effect::Open(path)]
}

/// Liste et sélection ciblées par un clic en colonne `column` (0 : liste principale).
fn column_parts(pane: &mut Pane, column: usize) -> Option<(&DirSnapshot, &mut fx_core::Selection)> {
    if column == 0 {
        Some((pane.listing.as_ref()?, &mut pane.selection))
    } else {
        let c = pane.columns.get_mut(column - 1)?;
        Some((c.listing.as_ref()?, &mut c.selection))
    }
}

pub fn click(state: &mut AppState, pane_id: PaneId, pos: usize, ctrl: bool, shift: bool) {
    let Some(p) = state.find_pane_mut(pane_id) else {
        return;
    };
    let column = if p.layout == Layout::Columns {
        p.active_column
    } else {
        0
    };
    let Some((l, sel)) = column_parts(p, column) else {
        return;
    };
    let Some(e) = l.at(pos) else {
        sel.clear();
        return;
    };
    if shift {
        let anchor = sel.anchor;
        let anchor_pos = anchor.and_then(|a| l.position_of(a)).unwrap_or(pos);
        if !ctrl {
            sel.clear();
        }
        for i in anchor_pos.min(pos)..=anchor_pos.max(pos) {
            if let Some(x) = l.at(i) {
                sel.set(x.id, true);
            }
        }
        sel.anchor = anchor.or(Some(e.id));
    } else if ctrl {
        sel.toggle(e.id);
    } else {
        sel.select_only(e.id);
    }
}

pub fn select_positions(state: &mut AppState, pane_id: PaneId, positions: &[usize], extend: bool) {
    let Some(p) = state.find_pane_mut(pane_id) else {
        return;
    };
    let column = if p.layout == Layout::Columns {
        p.active_column
    } else {
        0
    };
    let Some((l, sel)) = column_parts(p, column) else {
        return;
    };
    if !extend {
        sel.clear();
    }
    let mut last = None;
    for &pos in positions {
        if let Some(e) = l.at(pos) {
            sel.set(e.id, true);
            last = Some(e.id);
        }
    }
    if let Some(id) = last.filter(|_| sel.anchor.is_none() || !extend) {
        sel.anchor = Some(id);
    }
}

pub fn activate(state: &mut AppState, pane_id: PaneId, pos: usize) -> Vec<Effect> {
    let Some(p) = state.find_pane(pane_id) else {
        return vec![];
    };
    let column = if p.layout == Layout::Columns {
        p.active_column
    } else {
        0
    };
    let listing = if column == 0 {
        p.listing.as_ref()
    } else {
        p.columns.get(column - 1).and_then(|c| c.listing.as_ref())
    };
    let Some(l) = listing else { return vec![] };
    let Some(e) = l.at(pos) else { return vec![] };
    let path = path_of(l, e.id);
    match p.location() {
        // Ordinateur du réseau : ses partages.
        Location::Network(None) => {
            if let Some(parsing) = p.shell_items.get(&e.id) {
                return vec![Effect::OpenShellItem(parsing.clone())];
            }
            let server = path.to_string_lossy().into_owned();
            return load(state, pane_id, Location::Network(Some(server)), true);
        }
        // Corbeille : on parcourt un dossier supprimé ; un fichier montre ses propriétés
        // (comme l'Explorateur).
        Location::RecycleBin => {
            let Some(r) = p.recycled.get(&e.id).cloned() else {
                return vec![];
            };
            return if e.meta.is_dir() {
                load(state, pane_id, Location::Dir(r.item), true)
            } else {
                super::edit::open_properties(state, vec![r.item.clone()], Some(r))
            };
        }
        _ => {}
    }
    if e.meta.is_dir() {
        load(state, pane_id, Location::Dir(path), true)
    } else {
        vec![open_file(path, l.sort())]
    }
}

/// Extensions dont l'application par défaut sait parcourir le dossier (Photos, Lecteur
/// multimédia) quand on lui donne les fichiers voisins.
const MEDIA: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp", "dib", "tif", "tiff", "webp", "heic",
    "heif", "avif", "jxr", "wdp", "ico", "arw", "cr2", "cr3", "nef", "dng", "orf", "rw2", "raf",
    "mp4", "m4v", "mov", "wmv", "avi", "mkv", "webm", "3gp",
];

/// Ouvrir un fichier : avec ses voisins (ordre affiché) pour les images et vidéos.
pub(crate) fn open_file(path: PathBuf, sort: fx_core::SortSpec) -> Effect {
    let media = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| MEDIA.iter().any(|m| m.eq_ignore_ascii_case(e)));
    if media {
        Effect::OpenMedia { path, sort }
    } else {
        Effect::Open(path)
    }
}

/// Clic dans la vue Colonnes : sélectionne l'élément et, pour un dossier, ouvre une colonne.
pub fn column_click(
    state: &mut AppState,
    pane_id: PaneId,
    column: usize,
    pos: usize,
) -> Vec<Effect> {
    let generation = state.next_id();
    let show_hidden = state.settings.show_hidden;
    let Some(p) = state.find_pane_mut(pane_id) else {
        return vec![];
    };
    p.active_column = column;
    let Some((l, sel)) = column_parts(p, column) else {
        return vec![];
    };
    let Some(e) = l.at(pos) else { return vec![] };
    sel.select_only(e.id);
    let is_dir = e.meta.is_dir();
    let path = path_of(l, e.id);
    let sort = l.sort();
    p.columns.truncate(column);
    let mut fx = Vec::new();
    if is_dir {
        if let Some(c) = &p.cancel {
            c.cancel();
        }
        let cancel = CancelToken::new();
        p.cancel = Some(cancel.clone());
        p.columns.push(Column {
            path: path.clone(),
            listing: Some(DirSnapshot::new(path.clone(), generation, sort)),
            selection: fx_core::Selection::default(),
            generation,
        });
        fx.push(Effect::Enumerate {
            pane: pane_id,
            generation,
            path,
            options: EnumOptions { show_hidden },
            cancel,
            target: Target::Column,
        });
    }
    fx.extend(after_selection(state));
    fx
}

pub fn search(state: &mut AppState, pane_id: PaneId, query: String) -> Vec<Effect> {
    let query = query.trim().to_owned();
    let Some(p) = state.find_pane(pane_id) else {
        return vec![];
    };
    let root = match p.location() {
        Location::Dir(d) => d.clone(),
        Location::Search { root, .. } => root.clone(),
        _ => return vec![],
    };
    if query.is_empty() {
        return load(state, pane_id, Location::Dir(root), true);
    }
    // Une recherche affinée remplace la précédente dans l'historique.
    let refine = matches!(p.location(), Location::Search { .. });
    load(state, pane_id, Location::Search { root, query }, !refine)
}

/// Après un changement de sélection : met à jour le volet d'aperçu s'il est ouvert.
pub fn after_selection(state: &mut AppState) -> Vec<Effect> {
    if !state.settings.show_preview {
        return vec![];
    }
    let target = state.pane().selected_paths().into_iter().next();
    match target {
        Some(path) if state.preview.as_ref().is_none_or(|p| p.path != path) => {
            state.preview = Some(PreviewState {
                path: path.clone(),
                preview: None,
            });
            vec![Effect::LoadPreview(path)]
        }
        Some(_) => vec![],
        None => {
            state.preview = None;
            vec![]
        }
    }
}

/// Indicateur de tri d'une colonne pour l'en-tête du volet.
pub fn sort_indicator(pane: &Pane, key: SortKey) -> Option<SortDir> {
    let s = pane.listing.as_ref()?.sort();
    (s.key == key).then_some(s.dir)
}

/// Le chemin `p` est-il dans le dossier `dir` (directement) ?
pub(crate) fn is_child_of(p: &Path, dir: &Path) -> bool {
    p.parent()
        .is_some_and(|parent| parent.as_os_str().eq_ignore_ascii_case(dir.as_os_str()))
}
