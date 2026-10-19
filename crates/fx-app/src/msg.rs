use std::path::PathBuf;

use fx_commands::CommandId;
use fx_core::{Batch, SortKey};
use fx_fs::recycle::Recycled;
use fx_fs::{DirEvent, Outcome, SearchStats};
use fx_preview::Preview;
use fx_store::{Layout, Settings, Tag, TagId};
use fx_win::fileop::OpEvent;
use fx_win::shellfolder::ShellChild;
use fx_win::sys::{Drive, KnownFolder};

use crate::history::Location;
use crate::state::{PaneId, RecentItem};

/// Tout ce qui peut modifier l'état : entrées utilisateur et résultats des workers.
#[derive(Debug)]
pub enum Msg {
    /// Commande du registre (raccourci, barre d'outils, menu, palette).
    Command(CommandId),
    /// Démarrage : ouvre `start` s'il est fourni, sinon restaure `session`.
    Startup {
        start: Option<PathBuf>,
        session: Option<fx_store::Session>,
    },

    // --- Navigation et onglets ---
    Navigate(Location),
    NavigatePane(PaneId, Location),
    OpenInNewTab(Location),
    SelectTab(usize),
    CloseTab(usize),
    MoveTab {
        from: usize,
        to: usize,
    },
    FocusPane(PaneId),
    /// Ouvre un fichier avec son application par défaut (ou navigue si c'est un dossier).
    OpenPath(PathBuf),
    /// Fenêtre Propriétés de folder pour un chemin.
    Properties(PathBuf),
    /// Fenêtre Propriétés de Windows (onglets avancés : sécurité, versions…).
    NativeProperties(PathBuf),
    /// Corbeille : restaure la sélection, ou tout (`all`).
    RestoreRecycled {
        all: bool,
    },
    /// Colle le presse-papiers dans un dossier précis.
    PasteInto(PathBuf),

    // --- Liste (positions d'affichage, après filtre) ---
    Click {
        pane: PaneId,
        pos: usize,
        ctrl: bool,
        shift: bool,
    },
    /// Double-clic ou Entrée.
    Activate {
        pane: PaneId,
        pos: usize,
    },
    /// Sélection rectangulaire ou saisie au clavier : remplace (ou étend) la sélection.
    SelectPositions {
        pane: PaneId,
        positions: Vec<usize>,
        extend: bool,
    },
    /// Clic dans une colonne de la vue Colonnes (`column` 0 = la liste principale).
    ColumnClick {
        pane: PaneId,
        column: usize,
        pos: usize,
    },
    SetFilter {
        pane: PaneId,
        text: String,
    },
    Search {
        pane: PaneId,
        query: String,
    },
    SetLayout(Layout),
    SortBy(SortKey),
    SetGridSize(u16),

    // --- Édition ---
    BeginRename,
    CommitRename {
        pane: PaneId,
        id: u32,
        name: String,
    },
    CancelRename,
    /// Dépôt de fichiers dans `dest` ; `copy` : `Some(true)` copier, `Some(false)` déplacer,
    /// `None` : comme l'Explorateur (déplacer sur le même volume, copier sinon).
    DropPaths {
        dest: PathBuf,
        paths: Vec<PathBuf>,
        copy: Option<bool>,
    },
    ConfirmDialog,
    CancelDialog,
    ShowAbout,
    BulkRenameApply(Vec<(PathBuf, String)>),
    CancelOp(u64),
    DismissOp(u64),
    ClearFinishedOps,
    DismissToast(u64),
    /// Exécute une commande de menu shell déjà invoquée (signalement : actualiser…).
    ShellInvoked {
        verb: Option<String>,
    },

    // --- Réglages et données ---
    UpdateSettings(Box<Settings>),
    SetTag {
        paths: Vec<PathBuf>,
        tag: TagId,
        on: bool,
    },
    SaveTag(Tag),
    DeleteTag(TagId),
    Pin(PathBuf),
    Unpin(PathBuf),
    MovePin {
        from: usize,
        to: usize,
    },
    TogglePalette,
    CloseOverlays,
    /// L'UI a traité les demandes ponctuelles (`AppState::requests`).
    ConsumeRequests,

    // --- Résultats des workers ---
    ListingBatch {
        pane: PaneId,
        generation: u64,
        batch: Batch,
    },
    ListingFinished {
        pane: PaneId,
        generation: u64,
        result: Result<Outcome, String>,
    },
    ColumnBatch {
        pane: PaneId,
        generation: u64,
        batch: Batch,
    },
    ColumnFinished {
        pane: PaneId,
        generation: u64,
    },
    DirChanged(DirEvent),
    SearchFinished {
        pane: PaneId,
        generation: u64,
        stats: Option<SearchStats>,
    },
    OpEvent {
        op: u64,
        event: OpEvent,
    },
    OpFinished {
        op: u64,
        result: Result<bool, String>,
    },
    ClipboardState(bool),
    ClipboardFiles {
        dest: PathBuf,
        files: Vec<PathBuf>,
        cut: bool,
    },
    DrivesLoaded(Vec<Drive>),
    KnownLoaded(Vec<(KnownFolder, PathBuf)>),
    RecentLoaded(Vec<RecentItem>),
    PreviewLoaded(Option<Preview>),
    GitBranch {
        pane: PaneId,
        dir: PathBuf,
        branch: Option<String>,
    },
    TagSaved(Tag),
    RecycleBinLoaded {
        pane: PaneId,
        generation: u64,
        items: Vec<Recycled>,
    },
    /// La corbeille a changé hors des opérations suivies (vidée) : recharger.
    RecycleBinChanged,
    /// Éléments du réseau trouvés (au fil de la découverte) ; `done` : découverte terminée.
    NetworkLoaded {
        pane: PaneId,
        generation: u64,
        result: Result<Vec<ShellChild>, String>,
        done: bool,
    },
    WslLoaded(Vec<String>),
    Error(String),
}
