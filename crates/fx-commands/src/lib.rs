//! Registre des commandes de fx (inspiré du `CommandManager` + `CommandCodes` de Files).
//!
//! Un seul registre alimente barre d'outils, menus, raccourcis clavier, palette de commandes
//! et éditeur de raccourcis. Ce crate ne décrit que les **métadonnées** ; l'exécution vit dans
//! `fx-app::update` (`Msg::Command`), qui seul peut modifier l'état.

mod hotkey;

use std::collections::HashMap;

pub use hotkey::{HotKey, Key};
use serde::{Deserialize, Serialize};

/// Contexte d'activation des commandes (dérivé de l'état par `fx-app`).
#[derive(Debug, Clone, Copy, Default)]
pub struct CommandContext {
    pub selection: usize,
    pub has_location: bool,
    pub can_back: bool,
    pub can_forward: bool,
    pub can_up: bool,
    pub clipboard_has_files: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub tabs: usize,
    pub closed_tabs: usize,
    pub dual_pane: bool,
    /// Sélection d'un seul fichier (pas dossier).
    pub single_file: bool,
    /// Sélection d'une archive .zip.
    pub zip_selected: bool,
    pub in_recycle_bin: bool,
    /// Une mise à jour est téléchargée et prête à être installée.
    pub update_ready: bool,
    /// Le centre d'état a des opérations terminées.
    pub has_op_history: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Navigation,
    Tabs,
    Files,
    Selection,
    View,
    Application,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Self::Navigation => "Navigation",
            Self::Tabs => "Onglets et volets",
            Self::Files => "Fichiers",
            Self::Selection => "Sélection",
            Self::View => "Affichage",
            Self::Application => "Application",
        }
    }
}

macro_rules! commands {
    ($( $id:ident => $group:ident, $label:literal, $glyph:literal, [$($hk:literal),*] ;)*) => {
        /// Identifiant stable d'une commande (sérialisé dans les raccourcis personnalisés).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum CommandId { $($id),* }

        impl CommandId {
            pub const ALL: &'static [CommandId] = &[$(CommandId::$id),*];

            pub fn label(self) -> &'static str {
                match self { $(CommandId::$id => $label),* }
            }

            /// Glyphe Segoe Fluent Icons (chaîne vide : aucun).
            pub fn glyph(self) -> &'static str {
                match self { $(CommandId::$id => $glyph),* }
            }

            pub fn group(self) -> Group {
                match self { $(CommandId::$id => Group::$group),* }
            }

            pub fn default_hotkeys(self) -> Vec<HotKey> {
                match self { $(CommandId::$id => vec![$(HotKey::parse($hk).expect($hk)),*]),* }
            }
        }
    };
}

commands! {
    // Navigation
    Back => Navigation, "Précédent", "\u{E72B}", ["Alt+Left", "BrowserBack"];
    Forward => Navigation, "Suivant", "\u{E72A}", ["Alt+Right", "BrowserForward"];
    Up => Navigation, "Dossier parent", "\u{E74A}", ["Alt+Up"];
    Refresh => Navigation, "Actualiser", "\u{E72C}", ["F5", "Ctrl+R"];
    Home => Navigation, "Accueil", "\u{E80F}", ["Alt+Home"];
    EditPath => Navigation, "Modifier le chemin", "\u{E70F}", ["Ctrl+L", "Alt+D", "F4"];
    Search => Navigation, "Rechercher", "\u{E721}", ["Ctrl+F", "F3"];
    CommandPalette => Navigation, "Palette de commandes", "\u{E945}", ["Ctrl+Shift+P"];
    // Onglets et volets
    NewTab => Tabs, "Nouvel onglet", "\u{E710}", ["Ctrl+T"];
    DuplicateTab => Tabs, "Dupliquer l'onglet", "\u{E8C8}", [];
    CloseTab => Tabs, "Fermer l'onglet", "\u{E711}", ["Ctrl+W", "Ctrl+F4"];
    NextTab => Tabs, "Onglet suivant", "\u{E76C}", ["Ctrl+Tab", "Ctrl+PageDown"];
    PreviousTab => Tabs, "Onglet précédent", "\u{E76B}", ["Ctrl+Shift+Tab", "Ctrl+PageUp"];
    ReopenClosedTab => Tabs, "Rouvrir l'onglet fermé", "\u{E7A7}", ["Ctrl+Shift+T"];
    NewWindow => Tabs, "Nouvelle fenêtre", "\u{E737}", ["Ctrl+N"];
    ToggleDualPane => Tabs, "Double volet", "\u{E89F}", ["Ctrl+Shift+S"];
    FocusOtherPane => Tabs, "Volet suivant", "\u{E8AB}", ["Ctrl+Shift+Right"];
    // Fichiers
    Open => Files, "Ouvrir", "\u{E8E5}", ["Enter"];
    OpenInNewTab => Files, "Ouvrir dans un nouvel onglet", "\u{E8A7}", ["Ctrl+Enter"];
    OpenInOtherPane => Files, "Ouvrir dans l'autre volet", "\u{E89F}", [];
    OpenWith => Files, "Ouvrir avec…", "\u{E7AC}", [];
    OpenTerminal => Files, "Ouvrir dans le terminal", "\u{E756}", ["Ctrl+`"];
    RunAsAdmin => Files, "Exécuter en tant qu'administrateur", "\u{E7EF}", [];
    Cut => Files, "Couper", "\u{E8C6}", ["Ctrl+X"];
    Copy => Files, "Copier", "\u{E8C8}", ["Ctrl+C"];
    Paste => Files, "Coller", "\u{E77F}", ["Ctrl+V"];
    CopyPath => Files, "Copier le chemin", "\u{E62F}", ["Ctrl+Shift+C"];
    Rename => Files, "Renommer", "\u{E8AC}", ["F2"];
    BulkRename => Files, "Renommer en masse…", "\u{E8AC}", ["Ctrl+Shift+F2"];
    Delete => Files, "Supprimer", "\u{E74D}", ["Delete", "Ctrl+D"];
    DeletePermanently => Files, "Supprimer définitivement", "\u{E74D}", ["Shift+Delete"];
    NewFolder => Files, "Nouveau dossier", "\u{E8F4}", ["Ctrl+Shift+N"];
    NewFile => Files, "Nouveau fichier texte", "\u{E8A5}", ["Ctrl+Alt+N"];
    Properties => Files, "Propriétés", "\u{E946}", ["Alt+Enter"];
    PinToSidebar => Files, "Épingler à l'accès rapide", "\u{E718}", [];
    CompressToZip => Files, "Compresser en .zip", "\u{F012}", [];
    ExtractHere => Files, "Extraire ici", "\u{E8B7}", [];
    EmptyRecycleBin => Files, "Vider la corbeille", "\u{E74D}", [];
    Undo => Files, "Annuler", "\u{E7A7}", ["Ctrl+Z"];
    Redo => Files, "Rétablir", "\u{E7A6}", ["Ctrl+Y", "Ctrl+Shift+Z"];
    Restore => Files, "Restaurer", "\u{E777}", [];
    ShowMoreOptions => Files, "Afficher plus d'options", "\u{E712}", ["Shift+F10"];
    // Sélection
    SelectAll => Selection, "Tout sélectionner", "\u{E8B3}", ["Ctrl+A"];
    InvertSelection => Selection, "Inverser la sélection", "\u{E8B3}", [];
    ClearSelection => Selection, "Désélectionner", "\u{E8E6}", ["Escape"];
    // Affichage
    LayoutDetails => View, "Détails", "\u{E8EF}", ["Ctrl+Shift+1"];
    LayoutList => View, "Liste", "\u{E8FD}", ["Ctrl+Shift+2"];
    LayoutGrid => View, "Grandes icônes", "\u{E8A9}", ["Ctrl+Shift+3"];
    LayoutColumns => View, "Colonnes", "\u{E80A}", ["Ctrl+Shift+4"];
    ZoomIn => View, "Agrandir les icônes", "\u{E8A3}", ["Ctrl+Plus"];
    ZoomOut => View, "Réduire les icônes", "\u{E71F}", ["Ctrl+Minus"];
    SortByName => View, "Trier par nom", "\u{E8CB}", [];
    SortByModified => View, "Trier par date", "\u{E8CB}", [];
    SortByType => View, "Trier par type", "\u{E8CB}", [];
    SortBySize => View, "Trier par taille", "\u{E8CB}", [];
    ToggleSortDirection => View, "Inverser le tri", "\u{E8CB}", [];
    ToggleFoldersFirst => View, "Dossiers en premier", "\u{E8B7}", [];
    ToggleHidden => View, "Éléments masqués", "\u{ED1A}", ["Ctrl+H"];
    ToggleExtensions => View, "Extensions de fichiers", "\u{E8A5}", [];
    ToggleSidebar => View, "Barre latérale", "\u{E700}", ["Ctrl+B"];
    TogglePreview => View, "Volet d'aperçu", "\u{E8A1}", ["Alt+P"];
    // Application
    Settings => Application, "Paramètres", "\u{E713}", ["Ctrl+Comma"];
    ToggleTheme => Application, "Basculer clair / sombre", "\u{E793}", [];
    FullScreen => Application, "Plein écran", "\u{E740}", ["F11"];
    Diagnostics => Application, "Diagnostics", "\u{E9D9}", ["F12"];
    StatusCenter => Application, "Centre d'état", "\u{E81C}", [];
    ClearOperationHistory => Application, "Effacer l'historique des opérations", "\u{E74D}", [];
    CheckForUpdates => Application, "Rechercher les mises à jour", "\u{E895}", [];
    InstallUpdate => Application, "Installer la mise à jour", "\u{E896}", [];
    ReleaseNotes => Application, "Notes de version", "\u{E7BC}", [];
}

impl CommandId {
    /// La commande est-elle applicable dans ce contexte ?
    pub fn is_enabled(self, c: &CommandContext) -> bool {
        use CommandId::*;
        match self {
            Back => c.can_back,
            Forward => c.can_forward,
            Up => c.can_up,
            Refresh | Search | NewFolder | NewFile | OpenTerminal | SelectAll | InvertSelection => {
                c.has_location
            }
            Paste => c.has_location && c.clipboard_has_files,
            Restore => c.in_recycle_bin && c.selection > 0,
            // Corbeille : seuls restaurer, supprimer définitivement et propriétés s'appliquent.
            Open | OpenInNewTab | Cut | Copy | CopyPath | PinToSidebar | CompressToZip
            | ShowMoreOptions | Rename | BulkRename | OpenWith | RunAsAdmin | ExtractHere
                if c.in_recycle_bin =>
            {
                false
            }
            Open | OpenInNewTab | Cut | Copy | CopyPath | Delete | DeletePermanently
            | Properties | PinToSidebar | CompressToZip | ShowMoreOptions => c.selection > 0,
            OpenInOtherPane => c.selection > 0 && c.dual_pane,
            Rename => c.selection == 1,
            BulkRename => c.selection > 1,
            OpenWith | RunAsAdmin => c.single_file,
            ExtractHere => c.zip_selected,
            ClearSelection => c.selection > 0,
            CloseTab => c.tabs > 1,
            NextTab | PreviousTab => c.tabs > 1,
            ReopenClosedTab => c.closed_tabs > 0,
            FocusOtherPane => c.dual_pane,
            Undo => c.can_undo,
            Redo => c.can_redo,
            EmptyRecycleBin => true,
            InstallUpdate => c.update_ready,
            ClearOperationHistory => c.has_op_history,
            _ => true,
        }
    }
}

/// Raccourcis effectifs : défauts + personnalisations.
#[derive(Debug, Clone)]
pub struct KeyMap {
    by_key: HashMap<HotKey, CommandId>,
    by_command: HashMap<CommandId, Vec<HotKey>>,
}

impl KeyMap {
    /// `overrides` : commandes dont l'utilisateur a remplacé les raccourcis.
    pub fn new(overrides: &HashMap<CommandId, Vec<HotKey>>) -> Self {
        let mut by_key = HashMap::new();
        let mut by_command = HashMap::new();
        for &id in CommandId::ALL {
            let keys = overrides
                .get(&id)
                .cloned()
                .unwrap_or_else(|| id.default_hotkeys());
            for k in &keys {
                by_key.insert(*k, id);
            }
            by_command.insert(id, keys);
        }
        Self { by_key, by_command }
    }

    pub fn command_for(&self, key: &HotKey) -> Option<CommandId> {
        self.by_key.get(key).copied()
    }

    pub fn hotkeys(&self, id: CommandId) -> &[HotKey] {
        self.by_command.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Premier raccourci, formaté pour l'affichage (« Ctrl+C »).
    pub fn shortcut_text(&self, id: CommandId) -> Option<String> {
        self.hotkeys(id).first().map(ToString::to_string)
    }
}

impl Default for KeyMap {
    fn default() -> Self {
        Self::new(&HashMap::new())
    }
}

/// Recherche floue pour la palette de commandes : toutes les lettres de la requête, dans
/// l'ordre, avec bonus pour les débuts de mots. `None` si pas de correspondance.
pub fn fuzzy_score(label: &str, query: &str) -> Option<i32> {
    let label: Vec<char> = label.to_lowercase().chars().collect();
    let mut score = 0;
    let mut pos = 0;
    let mut prev_match = false;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = label[pos..].iter().position(|&c| c == q)?;
        let at = pos + found;
        score += 10 - found.min(9) as i32;
        if at == 0 || label[at - 1] == ' ' {
            score += 15;
        }
        if prev_match && found == 0 {
            score += 5;
        }
        prev_match = true;
        pos = at + 1;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hotkeys_are_unique() {
        let mut seen = HashMap::new();
        for &id in CommandId::ALL {
            for k in id.default_hotkeys() {
                if let Some(other) = seen.insert(k, id) {
                    panic!("{k} affecté à {other:?} et {id:?}");
                }
            }
        }
    }

    #[test]
    fn keymap_resolves_and_overrides() {
        let km = KeyMap::default();
        assert_eq!(
            km.command_for(&HotKey::parse("Ctrl+C").expect("hk")),
            Some(CommandId::Copy)
        );
        assert_eq!(
            km.shortcut_text(CommandId::Paste).as_deref(),
            Some("Ctrl+V")
        );
        let mut o = HashMap::new();
        o.insert(
            CommandId::Copy,
            vec![HotKey::parse("Ctrl+Shift+K").expect("hk")],
        );
        let km = KeyMap::new(&o);
        assert_eq!(km.command_for(&HotKey::parse("Ctrl+C").expect("hk")), None);
        assert_eq!(
            km.command_for(&HotKey::parse("Ctrl+Shift+K").expect("hk")),
            Some(CommandId::Copy)
        );
    }

    #[test]
    fn enabled_depends_on_context() {
        let c = CommandContext {
            selection: 0,
            has_location: true,
            ..Default::default()
        };
        assert!(!CommandId::Copy.is_enabled(&c));
        assert!(CommandId::NewFolder.is_enabled(&c));
        let c = CommandContext { selection: 2, ..c };
        assert!(!CommandId::Rename.is_enabled(&c));
        assert!(CommandId::BulkRename.is_enabled(&c));
    }

    #[test]
    fn fuzzy_prefers_word_starts() {
        let a = fuzzy_score("Nouveau dossier", "nd").expect("match");
        let b = fuzzy_score("Inverser la sélection", "nd");
        assert!(b.is_none_or(|b| a > b));
        assert!(fuzzy_score("Copier", "xyz").is_none());
    }
}
