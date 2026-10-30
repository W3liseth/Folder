//! Résultats des workers : lots d'énumération, changements surveillés, recherche,
//! progression des opérations.

use std::path::{Path, PathBuf};
use std::time::Instant;

use fx_core::entry::attrs;
use fx_core::{Batch, wtf8};
use fx_fs::{Change, DirEvent, Outcome, SearchStats};
use fx_win::fileop::OpEvent;
use fx_win::sys::KnownFolder;

use crate::effect::{Effect, Persist};
use crate::history::Location;
use crate::ops::{OpRecord, OpStatus, UndoEntry};
use crate::state::{AppState, ListingStatus, Pane, PaneId, Renaming, ToastKind};
use crate::update::{nav, persist_settings};

fn promote_incoming(p: &mut Pane) {
    let Some(next) = p.incoming.take() else {
        return;
    };
    let same_dir = p
        .listing
        .as_ref()
        .is_some_and(|l| l.location() == next.location() && !l.is_virtual());
    let keep = same_dir.then(|| p.selected_paths());
    p.listing = Some(next);
    p.selection.clear();
    // Actualisation : on conserve la sélection (par nom) au fil des lots.
    if let Some(paths) = keep.filter(|k| !k.is_empty()) {
        p.selection.anchor = None;
        reselect(p, &paths);
    }
    if let ListingStatus::Loading { started } = p.status {
        p.first_batch_after = Some(started.elapsed());
    }
}

/// Sélectionne les `paths` présents dans la liste.
fn reselect(p: &mut Pane, paths: &[PathBuf]) -> bool {
    let Some(l) = &mut p.listing else {
        return false;
    };
    let mut any = false;
    for path in paths {
        if !path
            .parent()
            .is_some_and(|d| d.as_os_str().eq_ignore_ascii_case(l.location().as_os_str()))
        {
            continue;
        }
        let Some(n) = path.file_name() else { continue };
        if let Some(id) = l.find(n.as_encoded_bytes()) {
            p.selection.set(id, true);
            p.selection.anchor.get_or_insert(id);
            any = true;
        }
    }
    any
}

pub fn listing_batch(
    state: &mut AppState,
    pane: PaneId,
    generation: u64,
    batch: Batch,
) -> Vec<Effect> {
    let Some(p) = state.find_pane_mut(pane) else {
        return vec![];
    };
    if p.incoming
        .as_ref()
        .is_some_and(|s| s.generation() == generation)
    {
        let keep: Vec<PathBuf> = if p.listing.as_ref().map(|l| l.location())
            == p.incoming.as_ref().map(|l| l.location())
        {
            p.selected_paths()
        } else {
            Vec::new()
        };
        promote_incoming(p);
        if let Some(l) = &mut p.listing {
            l.append(batch);
        }
        if !keep.is_empty() {
            reselect(p, &keep);
        }
    } else if let Some(l) = p.listing.as_mut().filter(|l| l.generation() == generation) {
        l.append(batch);
    } else {
        return vec![];
    }
    apply_pending(state, pane);
    vec![]
}

pub fn listing_finished(
    state: &mut AppState,
    pane: PaneId,
    generation: u64,
    result: Result<Outcome, String>,
) -> Vec<Effect> {
    let Some(p) = state.find_pane_mut(pane) else {
        return vec![];
    };
    let current =
        |s: &Option<fx_core::DirSnapshot>| s.as_ref().is_some_and(|s| s.generation() == generation);
    if !(current(&p.incoming) || current(&p.listing)) {
        return vec![];
    }
    match result {
        Ok(Outcome::Completed(stats)) => {
            if current(&p.incoming) {
                promote_incoming(p); // dossier vide : aucun lot reçu
            }
            if let Some(l) = p.listing.as_mut() {
                l.finish();
            }
            p.status = ListingStatus::Ready {
                elapsed: stats.elapsed,
            };
        }
        Ok(Outcome::Cancelled) => {}
        Err(message) => {
            p.incoming = None;
            p.listing = None;
            p.selection.clear();
            p.status = ListingStatus::Failed { message };
        }
    }
    p.cancel = None;
    apply_pending(state, pane);
    nav::after_selection(state)
}

pub fn column_batch(
    state: &mut AppState,
    pane: PaneId,
    generation: u64,
    batch: Batch,
) -> Vec<Effect> {
    if let Some(p) = state.find_pane_mut(pane)
        && let Some(c) = p.columns.iter_mut().find(|c| c.generation == generation)
        && let Some(l) = &mut c.listing
    {
        l.append(batch);
    }
    vec![]
}

pub fn column_finished(state: &mut AppState, pane: PaneId, generation: u64) -> Vec<Effect> {
    if let Some(p) = state.find_pane_mut(pane)
        && let Some(c) = p.columns.iter_mut().find(|c| c.generation == generation)
        && let Some(l) = &mut c.listing
    {
        l.finish();
    }
    vec![]
}

pub fn dir_changed(state: &mut AppState, ev: DirEvent) -> Vec<Effect> {
    let pane = PaneId(ev.key);
    if ev.changes.contains(&Change::Rescan) {
        let Some(Location::Dir(dir)) = state.find_pane(pane).map(|p| p.location().clone()) else {
            return vec![];
        };
        return nav::load(state, pane, Location::Dir(dir), false);
    }
    apply_changes(state, pane, &ev.changes);
    vec![]
}

/// État relu après une opération : appliqué à chaque volet qui affiche `dir`.
pub fn paths_changed(state: &mut AppState, dir: &Path, changes: &[Change]) -> Vec<Effect> {
    let panes: Vec<PaneId> = state
        .tabs
        .iter()
        .flat_map(|t| &t.panes)
        .filter(|p| matches!(p.location(), Location::Dir(d) if same_dir(d, dir)))
        .map(|p| p.id)
        .collect();
    for pane in panes {
        apply_changes(state, pane, changes);
    }
    vec![]
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
}

/// Applique des changements à la liste du volet `pane` (s'il affiche toujours ce dossier).
fn apply_changes(state: &mut AppState, pane: PaneId, changes: &[Change]) {
    let show_hidden = state.settings.show_hidden;
    let Some(p) = state.find_pane_mut(pane) else {
        return;
    };
    let Location::Dir(dir) = p.location().clone() else {
        return;
    };
    let Some(l) = p.listing.as_mut().filter(|l| l.location() == dir.as_path()) else {
        return;
    };
    for c in changes {
        match c {
            Change::Upsert { name, stat } => {
                if !show_hidden && stat.attributes & attrs::HIDDEN != 0 {
                    let mut b = Vec::new();
                    wtf8::push_utf16(&mut b, name);
                    if let Some(id) = l.remove(&b) {
                        p.selection.set(id, false);
                    }
                } else {
                    l.upsert(
                        name,
                        stat.attributes,
                        stat.size,
                        stat.created,
                        stat.modified,
                    );
                }
            }
            Change::Remove { name } => {
                let mut b = Vec::new();
                wtf8::push_utf16(&mut b, name);
                if let Some(id) = l.remove(&b) {
                    p.selection.set(id, false);
                }
            }
            Change::Rescan => {}
        }
    }
    apply_pending(state, pane);
}

/// Sélection et renommage en attente (éléments créés, collés, renommés) dès qu'ils
/// apparaissent dans la liste du volet.
fn apply_pending(state: &mut AppState, pane: PaneId) {
    if state.pending_select.is_empty() && state.pending_rename.is_none() {
        return;
    }
    let pending = std::mem::take(&mut state.pending_select);
    let rename = state.pending_rename.clone();
    let serial = state.next_id;
    let Some(p) = state.find_pane_mut(pane) else {
        state.pending_select = pending;
        return;
    };
    let Some(l) = &p.listing else {
        state.pending_select = pending;
        return;
    };
    let dir = l.location().to_path_buf();
    let (mine, others): (Vec<PathBuf>, Vec<PathBuf>) = pending
        .into_iter()
        .partition(|path| nav::is_child_of(path, &dir));
    if mine.is_empty() {
        state.pending_select = others;
        return;
    }
    let prev = std::mem::take(&mut p.selection);
    if !reselect(p, &mine) {
        p.selection = prev;
    }
    let mut still: Vec<PathBuf> = Vec::new();
    if let Some(l) = &mut p.listing {
        for path in &mine {
            if path
                .file_name()
                .is_none_or(|n| l.find(n.as_encoded_bytes()).is_none())
            {
                still.push(path.clone());
            }
        }
    }
    let mut renaming = None;
    if let Some(r) = rename
        && nav::is_child_of(&r, &dir)
        && let Some(l) = &mut p.listing
        && let Some(id) = r.file_name().and_then(|n| l.find(n.as_encoded_bytes()))
    {
        renaming = Some(Renaming { pane, id, serial });
    }
    still.extend(others);
    state.pending_select = still;
    if let Some(r) = renaming {
        state.pending_rename = None;
        state.renaming = Some(r);
        state.next_id += 1;
    }
}

pub fn search_finished(
    state: &mut AppState,
    pane: PaneId,
    generation: u64,
    stats: Option<SearchStats>,
) -> Vec<Effect> {
    let Some(p) = state.find_pane_mut(pane) else {
        return vec![];
    };
    let matches_gen =
        |s: &Option<fx_core::DirSnapshot>| s.as_ref().is_some_and(|s| s.generation() == generation);
    if !(matches_gen(&p.incoming) || matches_gen(&p.listing)) {
        return vec![];
    }
    if matches_gen(&p.incoming) {
        promote_incoming(p);
    }
    if let Some(l) = &mut p.listing {
        l.finish();
    }
    let elapsed = match p.status {
        ListingStatus::Loading { started } => started.elapsed(),
        _ => std::time::Duration::ZERO,
    };
    p.status = ListingStatus::Ready { elapsed };
    p.search_summary = stats.map(|s| {
        let more = if s.matches >= fx_fs::search::MAX_RESULTS {
            " (limite atteinte)"
        } else {
            ""
        };
        format!(
            "{} résultat{} dans {} dossier{}{more}",
            s.matches,
            plural(s.matches),
            s.dirs_scanned,
            plural(s.dirs_scanned)
        )
    });
    p.cancel = None;
    vec![]
}

fn plural(n: u32) -> &'static str {
    if n > 1 { "s" } else { "" }
}

pub fn op_event(state: &mut AppState, op: u64, event: OpEvent) -> Vec<Effect> {
    let mut fx = Vec::new();
    let Some(o) = state.ops.iter_mut().find(|o| o.id == op) else {
        return fx;
    };
    match event {
        OpEvent::Progress { done, total } => {
            o.done = done;
            o.total = total;
        }
        OpEvent::Item(name) => o.current = name,
        OpEvent::Created { created, .. } => o.created.push(created),
        OpEvent::Moved { from, to } => {
            o.moved.push((from.clone(), to.clone()));
            state.tags.move_path(&from, &to);
            // Les épingles suivent aussi les dossiers renommés ou déplacés.
            let mut pins_changed = false;
            for pin in &mut state.settings.pinned {
                if pin == &from {
                    *pin = to.clone();
                    pins_changed = true;
                }
            }
            if pins_changed {
                fx.push(persist_settings(state));
            }
            fx.push(Effect::Persist(Persist::MovePath { from, to }));
        }
    }
    fx
}

/// Corbeille lue : instantané virtuel groupé par dossier d'origine (le chemin affiché est
/// celui d'origine ; l'élément réel `$R…` est retenu par identifiant).
pub fn recycle_loaded(
    state: &mut AppState,
    pane: PaneId,
    generation: u64,
    mut items: Vec<fx_fs::recycle::Recycled>,
) -> Vec<Effect> {
    let Some(p) = state.find_pane_mut(pane) else {
        return vec![];
    };
    let Some(mut snap) = p.incoming.take_if(|s| s.generation() == generation) else {
        return vec![];
    };
    items.sort_by(|a, b| a.original.parent().cmp(&b.original.parent()));
    let mut recycled = std::collections::HashMap::with_capacity(items.len());
    for group in items.chunk_by(|a, b| a.original.parent() == b.original.parent()) {
        let Some(dir) = group[0].original.parent() else {
            continue;
        };
        let mut batch = Batch {
            dir: Some(dir.to_path_buf()),
            ..Batch::default()
        };
        let first = snap.total() as u32;
        let mut n = 0u32;
        for r in group {
            let Some(name) = r.original.file_name() else {
                continue;
            };
            let wide: Vec<u16> = name.to_string_lossy().encode_utf16().collect();
            // « Modifié le » affiche la date de suppression dans la corbeille.
            batch.push_utf16(&wide, r.attributes, r.size, r.deleted, r.deleted);
            recycled.insert(first + n, r.clone());
            n += 1;
        }
        snap.append(batch);
    }
    snap.finish();
    let elapsed = match p.status {
        ListingStatus::Loading { started } => started.elapsed(),
        _ => std::time::Duration::ZERO,
    };
    p.listing = Some(snap);
    p.recycled = recycled;
    p.selection.clear();
    p.status = ListingStatus::Ready { elapsed };
    vec![]
}

/// Recharge les volets qui affichent la corbeille.
pub fn reload_recycle_bins(state: &mut AppState) -> Vec<Effect> {
    let panes: Vec<PaneId> = state
        .tabs
        .iter()
        .flat_map(|t| &t.panes)
        .filter(|p| matches!(p.location(), Location::RecycleBin))
        .map(|p| p.id)
        .collect();
    panes
        .into_iter()
        .flat_map(|id| nav::load(state, id, Location::RecycleBin, false))
        .collect()
}

/// Réseau énuméré (au fil de la découverte) : ordinateurs et appareils (racine) ou partages
/// d'un ordinateur.
pub fn network_loaded(
    state: &mut AppState,
    pane: PaneId,
    generation: u64,
    result: Result<Vec<fx_win::shellfolder::ShellChild>, String>,
    done: bool,
) -> Vec<Effect> {
    let Some(p) = state.find_pane_mut(pane) else {
        return vec![];
    };
    // Premier envoi : la liste en cours de chargement devient la liste affichée.
    if let Some(next) = p.incoming.take_if(|s| s.generation() == generation) {
        p.listing = Some(next);
        p.selection.clear();
    }
    if !p
        .listing
        .as_ref()
        .is_some_and(|l| l.generation() == generation)
    {
        return vec![];
    }
    let children = match result {
        Ok(c) => c,
        Err(message) => {
            p.listing = None;
            p.status = ListingStatus::Failed { message };
            return vec![];
        }
    };
    let server = match p.location() {
        Location::Network(s) => s.clone(),
        _ => return vec![],
    };
    let Some(snap) = p.listing.as_mut() else {
        return vec![];
    };
    // Racine : ordinateurs (`\\NOM`, navigables vers leurs partages) puis autres appareils
    // (imprimantes, box, serveurs multimédias…), ouverts par le shell. Sous un ordinateur :
    // ses partages (dossiers du système de fichiers). Attribut système : chacun reçoit son
    // icône propre.
    let flags = attrs::DIRECTORY | attrs::SYSTEM;
    match &server {
        None => {
            let mut computers = Batch {
                dir: Some(PathBuf::from(r"\\")),
                ..Batch::default()
            };
            let mut devices = Vec::new();
            for c in &children {
                match c.parsing.strip_prefix(r"\\") {
                    Some(n) if !n.is_empty() && !n.contains('\\') => {
                        let wide: Vec<u16> = n.encode_utf16().collect();
                        computers.push_utf16(&wide, flags, 0, 0, 0);
                    }
                    _ => devices.push(c),
                }
            }
            snap.append(computers);
            let first = snap.total() as u32;
            let mut batch = Batch {
                dir: Some(PathBuf::from(crate::history::NETWORK_ID)),
                ..Batch::default()
            };
            for (i, c) in devices.iter().enumerate() {
                let wide: Vec<u16> = c.name.encode_utf16().collect();
                batch.push_utf16(&wide, attrs::SYSTEM, 0, 0, 0);
                p.shell_items.insert(first + i as u32, c.parsing.clone());
            }
            snap.append(batch);
        }
        Some(s) => {
            let mut batch = Batch {
                dir: Some(PathBuf::from(s)),
                ..Batch::default()
            };
            for c in children.iter().filter(|c| c.folder && c.filesystem) {
                if let Some(n) = c.parsing.rsplit('\\').next() {
                    let wide: Vec<u16> = n.encode_utf16().collect();
                    batch.push_utf16(&wide, flags, 0, 0, 0);
                }
            }
            snap.append(batch);
        }
    }
    if done {
        snap.finish();
        if let ListingStatus::Loading { started } = p.status {
            p.status = ListingStatus::Ready {
                elapsed: started.elapsed(),
            };
        }
    }
    vec![]
}

pub fn op_finished(state: &mut AppState, op: u64, result: Result<bool, String>) -> Vec<Effect> {
    let Some(o) = state.ops.iter_mut().find(|o| o.id == op) else {
        return vec![];
    };
    o.finished = Some(Instant::now());
    o.status = match &result {
        Ok(false) => OpStatus::Done,
        Ok(true) => OpStatus::Cancelled,
        Err(e) => OpStatus::Failed(e.clone()),
    };
    if o.total > 0 {
        o.done = o.total;
    }
    let undo = (o.status == OpStatus::Done && o.record_undo)
        .then(|| {
            o.inverse().map(|backward| UndoEntry {
                label: o.title.clone(),
                forward: o.job.clone(),
                backward,
            })
        })
        .flatten();
    let (title, status, items) = (o.title.clone(), o.status.clone(), o.items);
    let quick = o.started.elapsed().as_millis() < 1500;
    if let Some(e) = undo {
        state.undo.push(e);
    }
    match status {
        OpStatus::Failed(e) => state.toast(format!("{title} : échec — {e}"), ToastKind::Error),
        OpStatus::Cancelled => state.toast(format!("{title} : annulé"), ToastKind::Info),
        // Opérations brèves sur un seul élément : pas de notification (le résultat est visible).
        OpStatus::Done if items > 1 || !quick => {
            state.toast(format!("{title} : terminé"), ToastKind::Success)
        }
        _ => {}
    }
    // L'opération passe dans l'historique du centre d'état ; ses listes de chemins (déjà
    // copiées dans l'entrée d'annulation) sont libérées.
    let mut fx = Vec::new();
    if let Some(i) = state.ops.iter().position(|o| o.id == op) {
        let o = state.ops.remove(i);
        fx.extend(restat_effect(state, o.touched()));
        let ended = state.op_history.next_end(fx_win::time::now_filetime());
        let record = OpRecord::of(&o, ended);
        fx.push(Effect::Persist(Persist::AddOp(record.entry())));
        state.op_history.push(record);
        if state.show_status_center {
            state.op_history.mark_seen();
        }
    }
    // Toute opération peut avoir changé la corbeille (mise à la corbeille, restauration…).
    fx.extend(reload_recycle_bins(state));
    fx.push(Effect::CheckClipboard);
    fx
}

/// Relecture des chemins touchés situés dans un dossier affiché (les autres n'ont pas de
/// liste à mettre à jour). La surveillance fait souvent déjà ce travail : le refaire ne coûte
/// presque rien (`DirSnapshot::upsert` ignore un état inchangé).
fn restat_effect(state: &AppState, touched: Vec<PathBuf>) -> Option<Effect> {
    let shown: Vec<&Path> = state
        .tabs
        .iter()
        .flat_map(|t| &t.panes)
        .filter_map(|p| match p.location() {
            Location::Dir(d) => Some(d.as_path()),
            _ => None,
        })
        .collect();
    let mut paths: Vec<PathBuf> = touched
        .into_iter()
        .filter(|p| {
            p.parent()
                .is_some_and(|d| shown.iter().any(|s| same_dir(s, d)))
        })
        .collect();
    paths.dedup();
    (!paths.is_empty()).then_some(Effect::Restat { paths })
}

/// Dossiers connus chargés ; au premier lancement, ils deviennent les épingles par défaut.
pub fn known_loaded(state: &mut AppState, known: Vec<(KnownFolder, PathBuf)>) -> Vec<Effect> {
    state.known = known;
    if state.settings.pinned_initialized {
        return vec![];
    }
    state.settings.pinned = state
        .known
        .iter()
        .filter(|(k, _)| !matches!(k, KnownFolder::Home | KnownFolder::Recent))
        .map(|(_, p)| p.clone())
        .collect();
    state.settings.pinned_initialized = true;
    vec![persist_settings(state)]
}

/// Copie le chemin des sélections ou du dossier courant (texte, une ligne par chemin).
pub(crate) fn paths_text(state: &AppState) -> Option<String> {
    let p = state.pane();
    let sel = p.selected_paths();
    let paths: Vec<PathBuf> = if sel.is_empty() {
        p.working_dir().map(Path::to_path_buf).into_iter().collect()
    } else {
        sel
    };
    (!paths.is_empty()).then(|| {
        paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\r\n")
    })
}

/// Premier élément sélectionné (ou dossier courant) pour Propriétés.
pub(crate) fn first_target(state: &AppState) -> Option<PathBuf> {
    let p = state.pane();
    p.selected_paths()
        .into_iter()
        .next()
        .or_else(|| p.working_dir().map(Path::to_path_buf))
}
