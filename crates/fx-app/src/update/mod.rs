//! `update` : applique un message à l'état. **Pur** : aucune I/O ni appel système, < 1 ms.
//! Tout travail coûteux est renvoyé sous forme d'[`Effect`].

mod commands;
mod edit;
mod nav;
mod results;

use fx_core::{SortDir, SortKey, SortSpec};
use fx_store::FolderPrefs;

use crate::effect::{Effect, Persist};
use crate::history::Location;
use crate::msg::Msg;
use crate::state::{AppState, sort_key_id};

pub use edit::drop_copies;
pub use nav::sort_indicator;

pub fn update(state: &mut AppState, msg: Msg) -> Vec<Effect> {
    match msg {
        Msg::Command(id) => commands::run(state, id),
        Msg::Startup { start, session } => nav::startup(state, start, session),

        Msg::Navigate(loc) => {
            let id = state.pane().id;
            nav::load(state, id, loc, true)
        }
        Msg::NavigatePane(pane, loc) => nav::load(state, pane, loc, true),
        Msg::OpenInNewTab(loc) => nav::open_tab(state, loc),
        Msg::SelectTab(i) => {
            if i < state.tabs.len() {
                state.active_tab = i;
                state.renaming = None;
            }
            nav::after_selection(state)
        }
        Msg::CloseTab(i) => nav::close_tab(state, i),
        Msg::DetachTab { index, rect } => nav::detach_tab(state, index, rect),
        Msg::RemoveTab(i) => nav::remove_moved_tab(state, i),
        Msg::AdoptTab {
            path,
            back,
            forward,
            index,
        } => nav::adopt_tab(state, path, &back, &forward, index),
        Msg::MoveTab { from, to } => {
            if from < state.tabs.len() && to < state.tabs.len() && from != to {
                let t = state.tabs.remove(from);
                state.tabs.insert(to, t);
                state.active_tab = to;
            }
            vec![]
        }
        Msg::FocusPane(id) => {
            let tab = state.tab_mut();
            if let Some(i) = tab.panes.iter().position(|p| p.id == id) {
                tab.active = i;
            }
            nav::after_selection(state)
        }
        Msg::OpenPath(path) => nav::open_path(state, path),
        Msg::Properties(path) => edit::open_properties(state, vec![path], None),
        Msg::NativeProperties(path) => vec![Effect::Properties(path)],
        Msg::RestoreRecycled { all } => edit::restore_recycled(state, all),
        Msg::PasteInto(dest) => vec![Effect::ReadClipboard { dest }],

        Msg::Click {
            pane,
            pos,
            ctrl,
            shift,
        } => {
            nav::click(state, pane, pos, ctrl, shift);
            nav::after_selection(state)
        }
        Msg::Activate { pane, pos } => nav::activate(state, pane, pos),
        Msg::SelectPositions {
            pane,
            positions,
            extend,
        } => {
            nav::select_positions(state, pane, &positions, extend);
            nav::after_selection(state)
        }
        Msg::ColumnClick { pane, column, pos } => nav::column_click(state, pane, column, pos),
        Msg::SetFilter { pane, text } => {
            if let Some(p) = state.find_pane_mut(pane) {
                if let Some(l) = &mut p.listing {
                    l.set_filter(Some(&text));
                }
                p.filter = text;
            }
            vec![]
        }
        Msg::Search { pane, query } => nav::search(state, pane, query),
        Msg::SetLayout(layout) => {
            let p = state.pane_mut();
            p.layout = layout;
            p.columns.clear();
            p.active_column = 0;
            remember_folder_prefs(state)
        }
        Msg::SortBy(key) => {
            let cur = state
                .pane()
                .listing
                .as_ref()
                .map_or_else(|| state.sort_for(state.pane().location()), |l| l.sort());
            let dir = match (cur.key == key, cur.dir) {
                (true, SortDir::Asc) => SortDir::Desc,
                (true, SortDir::Desc) => SortDir::Asc,
                _ if matches!(key, SortKey::Modified | SortKey::Size) => SortDir::Desc,
                _ => SortDir::Asc,
            };
            apply_sort(state, SortSpec { key, dir, ..cur })
        }
        Msg::SetGridSize(size) => {
            state.settings.grid_size = size.clamp(48, 256);
            vec![persist_settings(state)]
        }

        Msg::BeginRename => edit::begin_rename(state),
        Msg::CommitRename { pane, id, name } => edit::commit_rename(state, pane, id, name),
        Msg::CancelRename => {
            state.renaming = None;
            vec![]
        }
        Msg::DropPaths { dest, paths, copy } => edit::drop_paths(state, dest, paths, copy),
        Msg::DropToRecycleBin(items) => edit::delete_items(state, items, false),
        Msg::ConfirmDialog => edit::confirm_dialog(state),
        Msg::CancelDialog => {
            state.dialog = None;
            vec![]
        }
        Msg::ShowAbout => {
            state.dialog = Some(crate::state::Dialog::About);
            vec![]
        }
        Msg::BulkRenameApply(items) => edit::bulk_rename(state, items),
        Msg::CancelOp(id) => {
            if let Some(op) = state.ops.iter().find(|o| o.id == id) {
                op.request_cancel();
            }
            vec![]
        }
        Msg::DismissOp(id) => {
            state.ops.retain(|o| o.id != id || o.is_running());
            vec![]
        }
        Msg::ClearFinishedOps => {
            state.ops.retain(crate::ops::Operation::is_running);
            vec![]
        }
        Msg::DismissToast(id) => {
            state.toasts.retain(|t| t.id != id);
            vec![]
        }
        Msg::ShellInvoked { verb } => {
            // Les commandes shell agissent hors de fx : la surveillance met la liste à jour ;
            // le presse-papiers a pu changer.
            let _ = verb;
            vec![Effect::CheckClipboard]
        }

        Msg::UpdateSettings(s) => edit::update_settings(state, *s),
        Msg::SetTag { paths, tag, on } => edit::set_tag(state, paths, tag, on),
        Msg::SaveTag(tag) => {
            if tag.id != 0
                && let Some(t) = state.tags.list.iter_mut().find(|t| t.id == tag.id)
            {
                *t = tag.clone();
            }
            vec![Effect::Persist(Persist::Tag(tag))]
        }
        Msg::DeleteTag(id) => {
            state.tags.remove_tag(id);
            if matches!(state.pane().location(), Location::Tag(t) if *t == id) {
                let pane = state.pane().id;
                let mut fx = nav::load(state, pane, Location::Home, true);
                fx.push(Effect::Persist(Persist::DeleteTag(id)));
                return fx;
            }
            vec![Effect::Persist(Persist::DeleteTag(id))]
        }
        Msg::Pin(path) => {
            if !state.settings.pinned.iter().any(|p| p == &path) {
                state.settings.pinned.push(path);
            }
            vec![persist_settings(state)]
        }
        Msg::Unpin(path) => {
            state.settings.pinned.retain(|p| p != &path);
            vec![persist_settings(state)]
        }
        Msg::MovePin { from, to } => {
            let pins = &mut state.settings.pinned;
            if from < pins.len() && to < pins.len() {
                let p = pins.remove(from);
                pins.insert(to, p);
            }
            vec![persist_settings(state)]
        }
        Msg::TogglePalette => {
            state.show_palette = !state.show_palette;
            vec![]
        }
        Msg::ConsumeRequests => {
            state.requests = crate::state::UiRequests::default();
            vec![]
        }
        Msg::CloseOverlays => {
            state.show_palette = false;
            state.show_search = false;
            vec![]
        }

        Msg::ListingBatch {
            pane,
            generation,
            batch,
        } => results::listing_batch(state, pane, generation, batch),
        Msg::ListingFinished {
            pane,
            generation,
            result,
        } => results::listing_finished(state, pane, generation, result),
        Msg::ColumnBatch {
            pane,
            generation,
            batch,
        } => results::column_batch(state, pane, generation, batch),
        Msg::ColumnFinished { pane, generation } => {
            results::column_finished(state, pane, generation)
        }
        Msg::DirChanged(ev) => results::dir_changed(state, ev),
        Msg::SearchFinished {
            pane,
            generation,
            stats,
        } => results::search_finished(state, pane, generation, stats),
        Msg::OpEvent { op, event } => results::op_event(state, op, event),
        Msg::OpFinished { op, result } => results::op_finished(state, op, result),
        Msg::ClipboardState(has) => {
            state.clipboard_has_files = has;
            vec![]
        }
        Msg::ClipboardFiles { dest, files, cut } => edit::paste(state, dest, files, cut),
        Msg::DrivesLoaded(d) => {
            state.drives = d;
            vec![]
        }
        Msg::KnownLoaded(k) => results::known_loaded(state, k),
        Msg::RecentLoaded(r) => {
            state.recent = r;
            vec![]
        }
        Msg::PreviewLoaded(p) => {
            if let (Some(st), Some(p)) = (&mut state.preview, p)
                && st.path == p.path
            {
                st.preview = Some(p);
            }
            vec![]
        }
        Msg::GitBranch { pane, dir, branch } => {
            if let Some(p) = state.find_pane_mut(pane)
                && p.location().dir() == Some(dir.as_path())
            {
                p.git_branch = branch;
            }
            vec![]
        }
        Msg::TagSaved(tag) => {
            match state.tags.list.iter_mut().find(|t| t.id == tag.id) {
                Some(t) => *t = tag,
                None => state.tags.list.push(tag),
            }
            vec![]
        }
        Msg::RecycleBinLoaded {
            pane,
            generation,
            items,
        } => results::recycle_loaded(state, pane, generation, items),
        Msg::RecycleBinChanged => results::reload_recycle_bins(state),
        Msg::NetworkLoaded {
            pane,
            generation,
            result,
            done,
        } => results::network_loaded(state, pane, generation, result, done),
        Msg::WslLoaded(w) => {
            state.wsl = w;
            vec![]
        }
        Msg::Error(message) => {
            state.toast(message, crate::state::ToastKind::Error);
            vec![]
        }
    }
}

pub(crate) fn persist_settings(state: &AppState) -> Effect {
    Effect::Persist(Persist::Settings(Box::new(state.settings.clone())))
}

/// Applique un tri au volet actif et le mémorise pour le dossier courant.
pub(crate) fn apply_sort(state: &mut AppState, spec: SortSpec) -> Vec<Effect> {
    let p = state.pane_mut();
    for l in [&mut p.listing, &mut p.incoming].into_iter().flatten() {
        l.set_sort(spec);
    }
    for c in &mut p.columns {
        if let Some(l) = &mut c.listing {
            l.set_sort(spec);
        }
    }
    remember_folder_prefs(state)
}

/// Mémorise disposition et tri du dossier courant.
fn remember_folder_prefs(state: &mut AppState) -> Vec<Effect> {
    let p = state.pane();
    let Location::Dir(dir) = p.location() else {
        return vec![];
    };
    let sort = p
        .listing
        .as_ref()
        .map_or_else(|| state.sort_for(p.location()), |l| l.sort());
    let prefs = FolderPrefs {
        layout: p.layout,
        sort_key: sort_key_id(sort.key),
        sort_desc: sort.dir == SortDir::Desc,
    };
    let dir = dir.clone();
    state
        .folder_prefs
        .insert(dir.to_string_lossy().to_lowercase(), prefs);
    vec![Effect::Persist(Persist::FolderPrefs { path: dir, prefs })]
}

#[cfg(test)]
mod tests;
