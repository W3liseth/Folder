use std::collections::HashMap;
use std::path::PathBuf;

use fx_commands::{CommandId, HotKey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

/// Disposition d'un volet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
pub enum Layout {
    #[default]
    Details,
    List,
    Grid,
    Columns,
}

impl Layout {
    pub fn label(self) -> &'static str {
        match self {
            Self::Details => "Détails",
            Self::List => "Liste",
            Self::Grid => "Grandes icônes",
            Self::Columns => "Colonnes",
        }
    }
}

/// Réglages utilisateur. Tout champ absent du fichier prend sa valeur par défaut, ce qui
/// rend les ajouts de réglages compatibles avec les anciens fichiers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub show_hidden: bool,
    pub show_extensions: bool,
    /// Étiquettes : section de la barre latérale et de l'accueil, menu contextuel, pastilles.
    pub show_tags: bool,
    /// Couleur d'accentuation ; `None` : celle de Windows.
    pub accent: Option<[u8; 3]>,
    /// Épaisseur des barres d'espace disque de la barre latérale (points).
    pub drive_bar: f32,
    /// Lignes parcourues par cran de molette (3, comme le réglage par défaut de Windows).
    pub scroll_lines: f32,
    pub folders_first: bool,
    pub confirm_delete: bool,
    pub show_thumbnails: bool,
    pub default_layout: Layout,
    /// Taille des icônes en vue Grandes icônes (points).
    pub grid_size: u16,
    pub show_sidebar: bool,
    pub sidebar_width: f32,
    pub show_preview: bool,
    pub preview_width: f32,
    pub restore_session: bool,
    /// Ouvre les dossiers d'un simple clic (comme le Web).
    pub single_click: bool,
    /// Dossiers épinglés (accès rapide). Vide au premier lancement : rempli avec les dossiers
    /// connus par `fx-app`.
    pub pinned: Vec<PathBuf>,
    pub pinned_initialized: bool,
    /// Raccourcis personnalisés (remplacent les défauts de la commande).
    pub keybindings: HashMap<CommandId, Vec<HotKey>>,
    /// Largeur de la colonne Nom ; 0 : automatique (occupe la place restante).
    pub col_name: f32,
    pub col_modified: f32,
    pub col_type: f32,
    pub col_size: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            show_hidden: false,
            show_extensions: true,
            show_tags: true,
            accent: None,
            drive_bar: 3.0,
            scroll_lines: 3.0,
            folders_first: true,
            confirm_delete: true,
            show_thumbnails: true,
            default_layout: Layout::Details,
            grid_size: 96,
            show_sidebar: true,
            sidebar_width: 230.0,
            show_preview: false,
            preview_width: 320.0,
            restore_session: true,
            single_click: false,
            pinned: Vec::new(),
            pinned_initialized: false,
            keybindings: HashMap::new(),
            col_name: 0.0,
            col_modified: 150.0,
            col_type: 150.0,
            col_size: 100.0,
        }
    }
}

/// Onglets ouverts, restaurés au démarrage.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub tabs: Vec<TabSession>,
    pub active: usize,
    /// Écran, taille et état de la fenêtre à la fermeture.
    pub placement: Option<WindowPlacement>,
}

/// Placement de la fenêtre en pixels physiques, tel que Windows le donne
/// (`GetWindowPlacement`) : rectangle de la fenêtre restaurée, même quand elle est agrandie,
/// et état agrandi. Les pixels physiques désignent sans ambiguïté l'écran, quelle que soit
/// sa mise à l'échelle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub maximized: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TabSession {
    pub left: PaneSession,
    pub right: Option<PaneSession>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneSession {
    /// `None` : page d'accueil.
    pub path: Option<PathBuf>,
    pub layout: Layout,
    /// Historique Précédent (du plus ancien au plus récent) et Suivant (du plus lointain au
    /// plus proche), dans le format de `path` ; l'accueil y a son propre identifiant.
    pub back: Vec<PathBuf>,
    pub forward: Vec<PathBuf>,
}
