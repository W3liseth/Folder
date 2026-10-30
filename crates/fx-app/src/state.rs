use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fx_commands::{CommandContext, KeyMap};
use fx_core::{CancelToken, DirSnapshot, Selection, SortDir, SortKey, SortSpec};
use fx_fs::recycle::Recycled;
use fx_preview::Preview;
use fx_store::{FolderPrefs, Layout, Settings, Tag, TagId};
use fx_win::sys::{Drive, KnownFolder};

use crate::history::{History, Location};
use crate::ops::{OpHistory, Operation, UndoStack};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingStatus {
    Idle,
    Loading {
        started: Instant,
    },
    Ready {
        elapsed: Duration,
    },
    /// Emplacement indisponible (introuvable, accès refusé…) : affiché à la place de la liste.
    Failed {
        message: String,
    },
}

/// Colonne supplémentaire de la vue Colonnes (Miller) : un dossier ouvert à droite.
#[derive(Debug)]
pub struct Column {
    pub path: PathBuf,
    pub listing: Option<DirSnapshot>,
    pub selection: Selection,
    pub(crate) generation: u64,
}

/// Nature de la dernière navigation d'un volet (sens de l'animation de transition).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavKind {
    /// Changement d'emplacement sans sens particulier (barre latérale, onglet restauré…).
    #[default]
    Other,
    /// Ouverture d'un dossier (on « entre »).
    Open,
    Back,
    Forward,
    Up,
}

/// Dernière navigation : `serial` est la génération du chargement correspondant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NavMark {
    pub serial: u64,
    pub kind: NavKind,
}

/// Un volet de navigation.
#[derive(Debug)]
pub struct Pane {
    pub id: PaneId,
    pub history: History,
    pub layout: Layout,
    /// Contenu affiché.
    pub listing: Option<DirSnapshot>,
    /// Contenu en cours de chargement, échangé avec `listing` dès son premier lot : l'ancien
    /// dossier reste visible jusque-là (pas d'écran vide entre deux dossiers).
    pub incoming: Option<DirSnapshot>,
    pub status: ListingStatus,
    pub selection: Selection,
    /// Texte du filtre rapide.
    pub filter: String,
    /// Délai entre la demande de navigation et l'affichage du premier lot.
    pub first_batch_after: Option<Duration>,
    pub columns: Vec<Column>,
    /// Colonne active en vue Colonnes (0 : la liste principale).
    pub active_column: usize,
    /// Branche Git du dossier affiché, s'il est dans un dépôt.
    pub git_branch: Option<String>,
    /// Branche en cours d'activation (`git switch` lancé).
    pub git_switching: Option<String>,
    /// Résumé de la dernière recherche (« 120 résultats dans 34 dossiers »).
    pub search_summary: Option<String>,
    /// Corbeille : élément réel (`$R…`) de chaque entrée affichée (par identifiant).
    pub recycled: HashMap<u32, Recycled>,
    /// Réseau : nom d'analyse shell des appareils affichés (imprimantes, box, médias…).
    pub shell_items: HashMap<u32, String>,
    /// Dernière navigation (animation de transition côté UI).
    pub nav: NavMark,
    /// Analyse de l'espace : page affichée (contenu, types de fichiers, récupérable).
    pub disk_tab: crate::disk::DiskTab,
    pub(crate) cancel: Option<CancelToken>,
}

impl Pane {
    pub(crate) fn new(id: PaneId, location: Location, layout: Layout) -> Self {
        Self {
            id,
            history: History::new(location),
            layout,
            listing: None,
            incoming: None,
            status: ListingStatus::Idle,
            selection: Selection::default(),
            filter: String::new(),
            first_batch_after: None,
            columns: Vec::new(),
            active_column: 0,
            git_branch: None,
            git_switching: None,
            search_summary: None,
            recycled: HashMap::new(),
            shell_items: HashMap::new(),
            nav: NavMark::default(),
            disk_tab: crate::disk::DiskTab::default(),
            cancel: None,
        }
    }

    pub fn location(&self) -> &Location {
        self.history.current()
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.status, ListingStatus::Loading { .. })
    }

    /// Dossier dans lequel créer, coller, ouvrir un terminal… (le plus profond en vue Colonnes).
    pub fn working_dir(&self) -> Option<&Path> {
        if self.layout == Layout::Columns
            && self.active_column > 0
            && let Some(c) = self.columns.get(self.active_column - 1)
        {
            return Some(&c.path);
        }
        match self.location() {
            Location::Dir(p) => Some(p),
            _ => None,
        }
    }

    /// Liste et sélection actives (la colonne active en vue Colonnes).
    pub fn active_listing(&self) -> Option<(&DirSnapshot, &Selection)> {
        if self.layout == Layout::Columns && self.active_column > 0 {
            let c = self.columns.get(self.active_column - 1)?;
            return Some((c.listing.as_ref()?, &c.selection));
        }
        Some((self.listing.as_ref()?, &self.selection))
    }

    /// Corbeille : éléments sélectionnés, dans l'ordre d'affichage.
    pub fn selected_recycled(&self) -> Vec<Recycled> {
        let Some(l) = &self.listing else {
            return Vec::new();
        };
        l.visible_ids()
            .iter()
            .filter(|&&id| self.selection.contains(id))
            .filter_map(|id| self.recycled.get(id).cloned())
            .collect()
    }

    /// Chemins sélectionnés, dans l'ordre d'affichage.
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        let Some((l, sel)) = self.active_listing() else {
            return Vec::new();
        };
        l.visible_ids()
            .iter()
            .filter(|&&id| sel.contains(id))
            .map(|&id| path_of(l, id))
            .collect()
    }

    /// Le seul élément sélectionné, si c'est un dossier.
    pub fn selected_dir(&self) -> Option<PathBuf> {
        let (l, sel) = self.active_listing()?;
        if sel.count() != 1 {
            return None;
        }
        let id = *l.visible_ids().iter().find(|&&id| sel.contains(id))?;
        l.view(id).meta.is_dir().then(|| path_of(l, id))
    }
}

/// Chemin complet d'une entrée d'un instantané.
pub fn path_of(listing: &DirSnapshot, id: u32) -> PathBuf {
    let v = listing.view(id);
    listing.dir_of(id).join(fx_core::wtf8::to_os_string(v.name))
}

#[derive(Debug)]
pub struct Tab {
    pub id: u64,
    pub panes: Vec<Pane>,
    pub active: usize,
}

impl Tab {
    pub fn active_pane(&self) -> &Pane {
        &self.panes[self.active.min(self.panes.len() - 1)]
    }

    pub fn title(&self) -> String {
        self.active_pane().location().title()
    }
}

/// Étiquettes et leurs associations, gardées en mémoire (la base est écrite en arrière-plan).
#[derive(Debug, Default)]
pub struct Tags {
    pub list: Vec<Tag>,
    by_path: HashMap<String, (PathBuf, Vec<TagId>)>,
}

impl Tags {
    pub fn new(list: Vec<Tag>, tagged: Vec<(PathBuf, TagId)>) -> Self {
        let mut by_path: HashMap<String, (PathBuf, Vec<TagId>)> = HashMap::new();
        for (p, t) in tagged {
            by_path
                .entry(key(&p))
                .or_insert_with(|| (p.clone(), Vec::new()))
                .1
                .push(t);
        }
        Self { list, by_path }
    }

    pub fn of(&self, path: &Path) -> &[TagId] {
        self.by_path
            .get(&key(path))
            .map_or(&[], |(_, v)| v.as_slice())
    }

    pub fn get(&self, id: TagId) -> Option<&Tag> {
        self.list.iter().find(|t| t.id == id)
    }

    pub fn paths_with(&self, id: TagId) -> Vec<PathBuf> {
        self.by_path
            .values()
            .filter(|(_, t)| t.contains(&id))
            .map(|(p, _)| p.clone())
            .collect()
    }

    pub fn count(&self, id: TagId) -> usize {
        self.by_path
            .values()
            .filter(|(_, t)| t.contains(&id))
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    pub(crate) fn set(&mut self, path: &Path, ids: Vec<TagId>) {
        if ids.is_empty() {
            self.by_path.remove(&key(path));
        } else {
            self.by_path.insert(key(path), (path.to_path_buf(), ids));
        }
    }

    pub(crate) fn remove_tag(&mut self, id: TagId) {
        self.list.retain(|t| t.id != id);
        for (_, v) in self.by_path.values_mut() {
            v.retain(|&t| t != id);
        }
        self.by_path.retain(|_, (_, v)| !v.is_empty());
    }

    /// Suit un renommage ou un déplacement (l'élément et tout son contenu).
    pub(crate) fn move_path(&mut self, from: &Path, to: &Path) {
        let f = key(from);
        let prefix = format!("{f}\\");
        let moved: Vec<String> = self
            .by_path
            .keys()
            .filter(|k| **k == f || k.starts_with(&prefix))
            .cloned()
            .collect();
        for k in moved {
            if let Some((p, v)) = self.by_path.remove(&k) {
                let rest = p
                    .strip_prefix(from)
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                let np = if rest.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(rest)
                };
                self.by_path.insert(key(&np), (np, v));
            }
        }
    }
}

/// Clé d'association : chemin en minuscules (Windows est insensible à la casse).
fn key(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentItem {
    pub name: String,
    /// Raccourci `.lnk` du dossier Récents (ouvert tel quel par le shell).
    pub path: PathBuf,
    pub modified: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

/// Menu des branches du dépôt affiché par un volet.
#[derive(Debug, Clone)]
pub struct GitMenu {
    pub pane: PaneId,
    pub dir: PathBuf,
    /// `None` : lecture en cours.
    pub branches: Option<Result<crate::git::GitBranches, String>>,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    pub kind: ToastKind,
    pub at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    ConfirmDelete {
        items: Vec<PathBuf>,
        permanent: bool,
    },
    ConfirmEmptyRecycleBin,
    /// Suppression définitive d'éléments de la corbeille.
    ConfirmPurge {
        items: Vec<Recycled>,
    },
    BulkRename {
        items: Vec<PathBuf>,
    },
    /// Nettoyage proposé par l'analyse de l'espace : `bytes` libérés (estimation de
    /// l'analyse).
    ConfirmClean {
        title: &'static str,
        items: Vec<PathBuf>,
        bytes: u64,
        permanent: bool,
    },
    Error {
        title: String,
        message: String,
    },
    About,
}

/// Renommage sur place en cours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Renaming {
    pub pane: PaneId,
    pub id: u32,
    /// Change à chaque nouveau renommage : l'UI réinitialise son champ de saisie.
    pub serial: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewState {
    pub path: PathBuf,
    /// `None` pendant le chargement.
    pub preview: Option<Preview>,
}

/// Demandes ponctuelles à l'UI (focus d'un champ, menu shell…), consommées par celle-ci.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UiRequests {
    pub focus_path: bool,
    pub shell_menu: bool,
    pub toggle_fullscreen: bool,
    /// Fermer la fenêtre (installeur de mise à jour lancé).
    pub close_window: bool,
}

pub struct AppState {
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    pub closed_tabs: Vec<Location>,
    pub settings: Settings,
    pub keymap: KeyMap,
    pub drives: Vec<Drive>,
    pub known: Vec<(KnownFolder, PathBuf)>,
    pub recent: Vec<RecentItem>,
    /// Distributions WSL installées.
    pub wsl: Vec<String>,
    /// Analyses de l'espace (en cours ou terminées), bornées (cf. `disk::MAX_SCANS`).
    pub disk_scans: Vec<crate::disk::DiskScan>,
    /// Dernière relecture de l'espace des lecteurs demandée pendant une opération.
    pub(crate) drives_requested: Option<Instant>,
    pub tags: Tags,
    pub folder_prefs: HashMap<String, FolderPrefs>,
    /// Opérations en cours (centre d'état).
    pub ops: Vec<Operation>,
    /// Opérations terminées (centre d'état), effaçable.
    pub op_history: OpHistory,
    pub undo: UndoStack,
    pub clipboard_has_files: bool,
    pub toasts: Vec<Toast>,
    pub dialog: Option<Dialog>,
    pub renaming: Option<Renaming>,
    pub preview: Option<PreviewState>,
    pub show_palette: bool,
    /// Fenêtre de recherche (bouton Rechercher, Ctrl+F).
    pub show_search: bool,
    /// Centre d'état (bouton à côté de Rechercher et de la palette).
    pub show_status_center: bool,
    /// Choix des lecteurs à analyser (bouton à côté de la palette).
    pub show_disk_picker: bool,
    pub show_diagnostics: bool,
    /// Menu des branches Git (bouton de la barre d'état).
    pub git_menu: Option<GitMenu>,
    /// Mise à jour : dernière vérification connue, installeur prêt.
    pub update: crate::upgrade::Status,
    pub requests: UiRequests,
    pub(crate) next_id: u64,
    /// Chemin d'un élément tout juste créé, à renommer dès qu'il apparaît dans la liste.
    pub(crate) pending_rename: Option<PathBuf>,
    /// Éléments à sélectionner dès qu'ils apparaissent (après collage, déplacement…).
    pub(crate) pending_select: Vec<PathBuf>,
}

impl AppState {
    /// État initial : un onglet sur l'accueil (la session est restaurée par `Msg::Restore`).
    pub fn new(settings: Settings, tags: Tags, folder_prefs: Vec<(PathBuf, FolderPrefs)>) -> Self {
        let keymap = KeyMap::new(&settings.keybindings);
        let mut s = Self {
            tabs: Vec::new(),
            active_tab: 0,
            closed_tabs: Vec::new(),
            keymap,
            drives: Vec::new(),
            known: Vec::new(),
            recent: Vec::new(),
            wsl: Vec::new(),
            disk_scans: Vec::new(),
            drives_requested: None,
            tags,
            folder_prefs: folder_prefs
                .into_iter()
                .map(|(p, f)| (p.to_string_lossy().to_lowercase(), f))
                .collect(),
            ops: Vec::new(),
            op_history: OpHistory::default(),
            undo: UndoStack::default(),
            clipboard_has_files: false,
            toasts: Vec::new(),
            dialog: None,
            renaming: None,
            preview: None,
            show_palette: false,
            show_search: false,
            show_status_center: false,
            show_disk_picker: false,
            show_diagnostics: false,
            git_menu: None,
            update: crate::upgrade::Status::Unknown,
            requests: UiRequests::default(),
            next_id: 1,
            pending_rename: None,
            pending_select: Vec::new(),

            settings,
        };
        let tab = s.new_tab(Location::Home);
        s.tabs.push(tab);
        s
    }

    pub(crate) fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(crate) fn new_tab(&mut self, loc: Location) -> Tab {
        let pane = self.new_pane(loc);
        Tab {
            id: self.next_id(),
            panes: vec![pane],
            active: 0,
        }
    }

    pub(crate) fn new_pane(&mut self, loc: Location) -> Pane {
        let layout = self.settings.default_layout;
        Pane::new(PaneId(self.next_id()), loc, layout)
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active_tab.min(self.tabs.len() - 1)]
    }

    pub(crate) fn tab_mut(&mut self) -> &mut Tab {
        let i = self.active_tab.min(self.tabs.len() - 1);
        &mut self.tabs[i]
    }

    /// Volet actif de l'onglet actif.
    pub fn pane(&self) -> &Pane {
        self.tab().active_pane()
    }

    pub(crate) fn pane_mut(&mut self) -> &mut Pane {
        let tab = self.tab_mut();
        let i = tab.active.min(tab.panes.len() - 1);
        &mut tab.panes[i]
    }

    pub fn find_pane(&self, id: PaneId) -> Option<&Pane> {
        self.tabs.iter().flat_map(|t| &t.panes).find(|p| p.id == id)
    }

    pub(crate) fn find_pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.tabs
            .iter_mut()
            .flat_map(|t| &mut t.panes)
            .find(|p| p.id == id)
    }

    /// Tri d'un dossier : préférences mémorisées pour ce dossier, sinon tri par nom.
    pub fn sort_for(&self, loc: &Location) -> SortSpec {
        // Analyse de l'espace : du plus gros au plus petit, dossiers et fichiers mêlés.
        if let Location::DiskUsage { .. } = loc {
            return SortSpec {
                key: SortKey::Size,
                dir: SortDir::Desc,
                folders_first: false,
            };
        }
        let base = SortSpec {
            folders_first: self.settings.folders_first,
            ..SortSpec::default()
        };
        match loc
            .dir()
            .and_then(|d| self.folder_prefs.get(&d.to_string_lossy().to_lowercase()))
        {
            Some(p) => SortSpec {
                key: sort_key(p.sort_key),
                dir: if p.sort_desc {
                    SortDir::Desc
                } else {
                    SortDir::Asc
                },
                ..base
            },
            None => base,
        }
    }

    /// Contexte d'activation des commandes pour l'UI (barre d'outils, menus, palette).
    pub fn command_context(&self) -> CommandContext {
        let pane = self.pane();
        let selected = pane.selected_paths();
        let single_file = selected.len() == 1
            && pane.listing.as_ref().is_some_and(|l| {
                l.visible_ids()
                    .iter()
                    .find(|&&id| pane.selection.contains(id))
                    .is_some_and(|&id| !l.view(id).meta.is_dir())
            });
        CommandContext {
            selection: selected.len(),
            has_location: pane.working_dir().is_some(),
            has_listing: pane.active_listing().is_some(),
            can_back: pane.history.can_go_back(),
            can_forward: pane.history.can_go_forward(),
            can_up: matches!(
                pane.location(),
                Location::Dir(_)
                    | Location::Search { .. }
                    | Location::Network(Some(_))
                    | Location::DiskUsage { .. }
            ),
            clipboard_has_files: self.clipboard_has_files,
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
            tabs: self.tabs.len(),
            closed_tabs: self.closed_tabs.len(),
            dual_pane: self.tab().panes.len() > 1,
            single_file,
            zip_selected: selected
                .iter()
                .any(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"))),
            in_recycle_bin: matches!(pane.location(), Location::RecycleBin),
            update_ready: self.update.ready_version().is_some(),
            has_op_history: !self.op_history.is_empty(),
            git_repo: pane.git_branch.is_some(),
        }
    }

    /// Nom d'analyse shell d'un appareil du réseau affiché dans une liste (`generation`).
    pub fn shell_item(&self, generation: u64, id: u32) -> Option<&str> {
        self.tabs
            .iter()
            .flat_map(|t| &t.panes)
            .filter(|p| {
                p.listing
                    .as_ref()
                    .is_some_and(|l| l.generation() == generation)
            })
            .find_map(|p| p.shell_items.get(&id))
            .map(String::as_str)
    }

    pub fn running_ops(&self) -> usize {
        self.ops.iter().filter(|o| o.is_running()).count()
    }

    /// Session à sauvegarder (onglets et volets).
    pub fn session(&self) -> fx_store::Session {
        let pane_session = |p: &Pane| {
            let (back, forward) = p.history.saved();
            fx_store::PaneSession {
                path: p.location().session_path(),
                layout: p.layout,
                back,
                forward,
            }
        };
        fx_store::Session {
            tabs: self
                .tabs
                .iter()
                .map(|t| fx_store::TabSession {
                    left: pane_session(&t.panes[0]),
                    right: t.panes.get(1).map(pane_session),
                })
                .collect(),
            active: self.active_tab,
            // Le placement de la fenêtre est un état de la vue : `fx-ui` le complète.
            placement: None,
        }
    }

    pub(crate) fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        let id = self.next_id();
        self.toasts.push(Toast {
            id,
            text: text.into(),
            kind,
            at: Instant::now(),
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }
}

pub(crate) fn sort_key(k: u8) -> SortKey {
    match k {
        1 => SortKey::Modified,
        2 => SortKey::Type,
        3 => SortKey::Size,
        _ => SortKey::Name,
    }
}

pub(crate) fn sort_key_id(k: SortKey) -> u8 {
    match k {
        SortKey::Name => 0,
        SortKey::Modified => 1,
        SortKey::Type => 2,
        SortKey::Size => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_follow_moves_case_insensitively() {
        let mut t = Tags::new(
            Vec::new(),
            vec![
                (PathBuf::from(r"C:\Docs\a.txt"), 1),
                (PathBuf::from(r"C:\Docs"), 2),
            ],
        );
        assert_eq!(t.of(Path::new(r"c:\docs\A.TXT")), &[1]);
        t.move_path(Path::new(r"C:\Docs"), Path::new(r"D:\Archive"));
        assert_eq!(t.of(Path::new(r"D:\Archive\a.txt")), &[1]);
        assert_eq!(t.of(Path::new(r"D:\Archive")), &[2]);
        assert!(t.of(Path::new(r"C:\Docs\a.txt")).is_empty());
        assert_eq!(t.paths_with(1), vec![PathBuf::from(r"D:\Archive\a.txt")]);
    }
}
