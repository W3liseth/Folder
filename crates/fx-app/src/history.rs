use std::path::{Path, PathBuf};

/// Profondeur maximale conservée dans chaque sens.
const MAX_DEPTH: usize = 100;

/// Nom d'analyse shell de la Corbeille (icône, session).
pub const RECYCLE_BIN_ID: &str = "::{645FF040-5081-101B-9F08-00AA002F954E}";
/// Nom d'analyse shell du dossier Réseau (énumération, icône, session).
pub const NETWORK_ID: &str = "::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}";
/// Page d'accueil dans l'historique sauvegardé (pas un chemin réel).
pub const HOME_ID: &str = "folder:home";
/// Entrées d'historique sauvegardées par sens et par volet.
const SAVED_DEPTH: usize = 30;

/// Ce qu'affiche un volet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// Page d'accueil (accès rapide, lecteurs, étiquettes, récents).
    Home,
    Dir(PathBuf),
    /// Résultats d'une recherche récursive sous `root`.
    Search {
        root: PathBuf,
        query: String,
    },
    /// Fichiers portant une étiquette.
    Tag(fx_store::TagId),
    /// Corbeille (tous les lecteurs), affichée dans fx.
    RecycleBin,
    /// Réseau : ordinateurs (`None`) ou partages d'un ordinateur (`Some(r"\\SERVEUR")`).
    Network(Option<String>),
}

impl Location {
    /// Dossier réel affiché, s'il y en a un.
    pub fn dir(&self) -> Option<&Path> {
        match self {
            Self::Dir(p) => Some(p),
            Self::Search { root, .. } => Some(root),
            Self::Home | Self::Tag(_) | Self::RecycleBin | Self::Network(_) => None,
        }
    }

    /// Emplacement désigné par un chemin saisi ou sauvegardé : identifiants shell de la
    /// Corbeille et du Réseau, ordinateur du réseau seul (`\\SERVEUR`), sinon un dossier.
    pub fn from_path(path: PathBuf) -> Self {
        let s = path.to_string_lossy();
        if s == HOME_ID {
            return Self::Home;
        }
        if s.eq_ignore_ascii_case(RECYCLE_BIN_ID)
            || s.eq_ignore_ascii_case("shell:RecycleBinFolder")
        {
            return Self::RecycleBin;
        }
        if s.eq_ignore_ascii_case(NETWORK_ID) || s.eq_ignore_ascii_case("shell:NetworkPlacesFolder")
        {
            return Self::Network(None);
        }
        if let Some(server) = s.strip_prefix(r"\\")
            && !server.trim_end_matches('\\').is_empty()
            && !server.trim_end_matches('\\').contains('\\')
        {
            return Self::Network(Some(format!(r"\\{}", server.trim_end_matches('\\'))));
        }
        Self::Dir(path)
    }

    /// Chemin sauvegardé dans la session (relu par [`Location::from_path`]).
    pub fn session_path(&self) -> Option<PathBuf> {
        match self {
            Self::Dir(d) => Some(d.clone()),
            Self::RecycleBin => Some(PathBuf::from(RECYCLE_BIN_ID)),
            Self::Network(None) => Some(PathBuf::from(NETWORK_ID)),
            Self::Network(Some(s)) => Some(PathBuf::from(s)),
            Self::Home | Self::Search { .. } | Self::Tag(_) => None,
        }
    }

    /// Argument de `folder.exe` qui ouvre une fenêtre sur cet emplacement (relu par
    /// [`Location::from_path`]) : une recherche rouvre son dossier, une étiquette l'accueil.
    pub fn window_arg(&self) -> PathBuf {
        match self {
            Self::Search { root, .. } => root.clone(),
            _ => self
                .history_path()
                .unwrap_or_else(|| PathBuf::from(HOME_ID)),
        }
    }

    /// Entrée d'historique sauvegardée : comme [`Location::session_path`], accueil compris.
    fn history_path(&self) -> Option<PathBuf> {
        match self {
            Self::Home => Some(PathBuf::from(HOME_ID)),
            _ => self.session_path(),
        }
    }

    /// Source de l'icône shell propre à l'emplacement (dossier, Corbeille, Réseau…).
    pub fn icon_source(&self) -> Option<PathBuf> {
        match self {
            Self::Dir(_) | Self::RecycleBin | Self::Network(_) => self.session_path(),
            _ => None,
        }
    }

    /// Titre court (onglet).
    pub fn title(&self) -> String {
        match self {
            Self::Home => "Accueil".into(),
            Self::Dir(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string()),
            Self::Search { query, .. } => format!("Recherche « {query} »"),
            Self::Tag(_) => "Étiquette".into(),
            Self::RecycleBin => "Corbeille".into(),
            Self::Network(None) => "Réseau".into(),
            Self::Network(Some(s)) => s.trim_start_matches('\\').to_owned(),
        }
    }
}

/// Historique Précédent/Suivant d'un volet.
#[derive(Debug)]
pub struct History {
    back: Vec<Location>,
    forward: Vec<Location>,
    current: Location,
}

impl Default for History {
    fn default() -> Self {
        Self {
            back: Vec::new(),
            forward: Vec::new(),
            current: Location::Home,
        }
    }
}

impl History {
    pub fn new(start: Location) -> Self {
        Self {
            current: start,
            ..Self::default()
        }
    }

    pub fn current(&self) -> &Location {
        &self.current
    }

    /// Nouvelle destination : vide l'historique « suivant ». Sans effet si identique.
    pub fn visit(&mut self, loc: Location) -> bool {
        if self.current == loc {
            return false;
        }
        let prev = std::mem::replace(&mut self.current, loc);
        push_bounded(&mut self.back, prev);
        self.forward.clear();
        true
    }

    /// Remplace la destination courante sans toucher à l'historique (recherche affinée).
    pub fn replace(&mut self, loc: Location) {
        self.current = loc;
    }

    pub fn go_back(&mut self) -> Option<&Location> {
        let target = self.back.pop()?;
        let cur = std::mem::replace(&mut self.current, target);
        push_bounded(&mut self.forward, cur);
        Some(&self.current)
    }

    pub fn go_forward(&mut self) -> Option<&Location> {
        let target = self.forward.pop()?;
        let cur = std::mem::replace(&mut self.current, target);
        push_bounded(&mut self.back, cur);
        Some(&self.current)
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Entrées précédentes, de la plus récente à la plus ancienne (menu de l'historique).
    pub fn back_entries(&self) -> impl Iterator<Item = &Location> {
        self.back.iter().rev()
    }

    /// Historique à sauvegarder (Précédent, Suivant) : les 30 entrées les plus proches de
    /// chaque côté ; recherches et étiquettes, non restaurables, sont omises.
    pub fn saved(&self) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let keep = |stack: &[Location]| {
            let mut v: Vec<PathBuf> = stack
                .iter()
                .rev()
                .filter_map(Location::history_path)
                .take(SAVED_DEPTH)
                .collect();
            v.reverse();
            v
        };
        (keep(&self.back), keep(&self.forward))
    }

    /// Historique relu d'une session (cf. [`History::saved`]).
    pub fn restored(current: Location, back: &[PathBuf], forward: &[PathBuf]) -> Self {
        let read = |v: &[PathBuf]| {
            v.iter()
                .map(|p| Location::from_path(p.clone()))
                .collect::<Vec<_>>()
        };
        Self {
            back: read(back),
            forward: read(forward),
            current,
        }
    }
}

fn push_bounded(stack: &mut Vec<Location>, l: Location) {
    if stack.len() == MAX_DEPTH {
        stack.remove(0);
    }
    stack.push(l);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Location {
        Location::Dir(PathBuf::from(s))
    }

    #[test]
    fn back_forward_and_branching() {
        let mut h = History::new(d("a"));
        h.visit(d("b"));
        h.visit(d("c"));
        assert_eq!(h.go_back(), Some(&d("b")));
        assert_eq!(h.go_back(), Some(&d("a")));
        assert_eq!(h.go_back(), None);
        assert_eq!(h.go_forward(), Some(&d("b")));
        h.visit(d("d"));
        assert!(!h.can_go_forward());
        assert_eq!(h.go_back(), Some(&d("b")));
    }

    #[test]
    fn revisiting_current_is_noop_and_depth_is_bounded() {
        let mut h = History::default();
        for i in 0..250 {
            h.visit(d(&i.to_string()));
            assert!(!h.visit(d(&i.to_string())));
        }
        assert_eq!(h.back.len(), MAX_DEPTH);
    }

    #[test]
    fn titles() {
        assert_eq!(d(r"C:\Users\moi").title(), "moi");
        assert_eq!(d(r"C:\").title(), r"C:\");
        assert_eq!(Location::Home.title(), "Accueil");
    }
}
