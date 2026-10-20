use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use fx_core::CancelToken;
use fx_fs::EnumOptions;
use fx_store::{FolderPrefs, Session, Settings, Tag, TagId};

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
    Tags { path: PathBuf, tags: Vec<TagId> },
    Tag(Tag),
    DeleteTag(TagId),
    MovePath { from: PathBuf, to: PathBuf },
    FolderPrefs { path: PathBuf, prefs: FolderPrefs },
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
    /// Fenêtre Paramètres (processus `--settings`), ou ramenée au premier plan si ouverte.
    SettingsWindow,
    RunAsAdmin(PathBuf),
    OpenTerminal(PathBuf),
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
    Persist(Persist),
    /// Nouvelle fenêtre (processus `folder.exe`) ouverte sur `start` (cf.
    /// [`crate::Location::window_arg`]) ; `rect` : placement imposé en pixels physiques
    /// (gauche, haut, droite, bas), sinon celui de la dernière session.
    NewWindow {
        start: Option<PathBuf>,
        rect: Option<[i32; 4]>,
    },
    EmptyRecycleBin,
}
