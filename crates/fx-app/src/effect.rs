use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use fx_core::CancelToken;
use fx_fs::EnumOptions;
use fx_store::{FolderPrefs, OpEntry, Session, Settings, Tag, TagId};

use crate::ops::Job;
use crate::state::PaneId;

/// Où ranger les lots d'une énumération.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Pane,
    Column,
}

/// Écritures persistantes, exécutées hors du thread UI.
#[derive(Debug, Clone, PartialEq)]
pub enum Persist {
    Settings(Box<Settings>),
    Session(Session),
    Tags {
        path: PathBuf,
        tags: Vec<TagId>,
    },
    Tag(Tag),
    DeleteTag(TagId),
    MovePath {
        from: PathBuf,
        to: PathBuf,
    },
    FolderPrefs {
        path: PathBuf,
        prefs: FolderPrefs,
    },
    /// Historique des opérations (centre d'état). Chacune répond par
    /// `Msg::OpHistoryLoaded` avec l'historique de la base après l'écriture, envoyé sous le
    /// verrou de la base : les réponses arrivent dans l'ordre des écritures, même venant de
    /// plusieurs threads d'I/O.
    AddOp(OpEntry),
    RemoveOp {
        ended: i64,
        title: String,
    },
    ClearOps,
    /// Simple lecture (démarrage, ouverture du centre d'état : les autres fenêtres écrivent
    /// aussi dans la base).
    LoadOps,
}

/// Travail demandé par `update`, exécuté hors du thread UI par le `Scheduler`.
#[derive(Debug)]
pub enum Effect {
    Enumerate {
        pane: PaneId,
        generation: u64,
        path: PathBuf,
        options: EnumOptions,
        cancel: CancelToken,
        target: Target,
    },
    Search {
        pane: PaneId,
        generation: u64,
        root: PathBuf,
        query: String,
        show_hidden: bool,
        cancel: CancelToken,
    },
    /// Construit une liste virtuelle à partir de chemins (fichiers étiquetés).
    ListPaths {
        pane: PaneId,
        generation: u64,
        paths: Vec<PathBuf>,
    },
    Watch {
        key: u64,
        path: PathBuf,
    },
    Unwatch {
        key: u64,
    },
    Open(PathBuf),
    /// Image ou vidéo : ouverte avec ses voisins du dossier, dans l'ordre affiché, pour que
    /// l'application (Photos…) passe au fichier suivant avec les flèches.
    OpenMedia {
        path: PathBuf,
        sort: fx_core::SortSpec,
    },
    /// Ouvre un élément du shell par son nom d'analyse (appareil du réseau).
    OpenShellItem(String),
    OpenWith(PathBuf),
    /// Fenêtre Propriétés de Windows.
    Properties(PathBuf),
    /// Fenêtre Propriétés de folder (processus `--properties`) ; `recycled` : chemin
    /// d'origine et date de suppression d'un élément de la corbeille.
    PropertiesWindow {
        paths: Vec<PathBuf>,
        recycled: Option<(PathBuf, i64)>,
    },
    LoadRecycleBin {
        pane: PaneId,
        generation: u64,
    },
    /// Énumère un dossier du shell (Réseau, ordinateur) sur un thread STA temporaire.
    LoadNetwork {
        pane: PaneId,
        generation: u64,
        parsing: String,
    },
    LoadWsl,
    /// Sous-dossiers de `path` pour l'arborescence de la barre latérale (cf. `tree`) ;
    /// répond par `Msg::SubdirsLoaded`.
    ListSubdirs {
        generation: u64,
        path: PathBuf,
        show_hidden: bool,
        cancel: CancelToken,
    },
    /// Fenêtre Paramètres (processus `--settings`), ou ramenée au premier plan si ouverte ;
    /// `notes` : ouverte sur les notes de version.
    SettingsWindow {
        notes: bool,
    },
    RunAsAdmin(PathBuf),
    OpenTerminal(PathBuf),
    /// Saisie de la barre d'adresse à résoudre hors du thread UI (cf. `address`) : dossier
    /// (navigation du volet `pane`), fichier à ouvrir ou programme à lancer dans `cwd`.
    RunAddress {
        pane: PaneId,
        text: String,
        cwd: Option<PathBuf>,
    },
    Job {
        op: u64,
        job: Job,
        cancel: Arc<AtomicBool>,
    },
    SetClipboard {
        paths: Vec<PathBuf>,
        cut: bool,
    },
    CopyText(String),
    CheckClipboard,
    ReadClipboard {
        dest: PathBuf,
    },
    LoadDrives,
    LoadKnownFolders,
    LoadRecent,
    LoadPreview(PathBuf),
    DetectGit {
        pane: PaneId,
        dir: PathBuf,
    },
    /// Branches du dépôt Git contenant `dir` (menu de la barre d'état).
    ListGitBranches {
        pane: PaneId,
        dir: PathBuf,
    },
    /// Active une branche (`git switch`) dans le dépôt contenant `dir`.
    SwitchGitBranch {
        pane: PaneId,
        dir: PathBuf,
        branch: String,
    },
    /// Relit l'état de chemins touchés par une opération terminée (cf. `fx_fs::restat`).
    Restat {
        paths: Vec<PathBuf>,
    },
    Persist(Persist),
    /// Nouvelle fenêtre (processus `folder.exe`) ouverte sur `start` (cf.
    /// [`crate::Location::window_arg`]) ; `rect` : placement imposé en pixels physiques
    /// (gauche, haut, droite, bas), sinon celui de la dernière session.
    NewWindow {
        start: Option<PathBuf>,
        rect: Option<[i32; 4]>,
    },
    EmptyRecycleBin,
    /// Analyse de l'espace de `root` sur des threads temporaires (cf. `fx_fs::diskscan`) ;
    /// répond par `Msg::DiskProgress` puis `Msg::DiskScanned`.
    ScanDisk {
        scan: u64,
        root: PathBuf,
        keep_from: u64,
        cancel: CancelToken,
    },
    /// Relit un dossier analysé après une opération (`known` : sous-dossiers déjà dans
    /// l'arbre, non relus).
    RefreshDisk {
        scan: u64,
        dir: PathBuf,
        known: Vec<Vec<u8>>,
        keep_from: u64,
    },
    /// Liste le contenu direct de `dirs` pour un nettoyage (répond par `Msg::CleanListed`).
    ListContents {
        title: &'static str,
        dirs: Vec<PathBuf>,
        bytes: u64,
        permanent: bool,
    },
    /// Raccourcis système (cf. `fx_win::shortcuts`) : Windows + E ouvre folder (`win_e`),
    /// sinon la touche `open_hotkey` (si elle est valide) ouvre folder.
    SystemShortcuts {
        win_e: bool,
        open_hotkey: Option<fx_commands::HotKey>,
    },
    /// Au démarrage, Windows + E pris par folder : relance le processus résident s'il ne
    /// tourne plus (arrêté, ou exécutable remplacé par une mise à jour).
    EnsureWinE,
    /// Recherche de mise à jour (cf. `upgrade::check`) ; `delay` : avant une vérification
    /// en ligne (automatique : après le démarrage, pour ne pas le ralentir).
    CheckUpdate {
        force: bool,
        delay: std::time::Duration,
    },
    /// Lance l'installeur prêt d'après le fichier partagé (cf. `upgrade::cached` : il a pu
    /// être téléchargé par une autre fenêtre ou par les Paramètres), ou signale qu'il n'y
    /// en a pas.
    InstallUpdate,
}
