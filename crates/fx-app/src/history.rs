use std::path::{Path, PathBuf};

/// Profondeur maximale conservée dans chaque sens.
const MAX_DEPTH: usize = 100;

/// Nom d'analyse shell de la Corbeille (icône, session).
pub const RECYCLE_BIN_ID: &str = "::{645FF040-5081-101B-9F08-00AA002F954E}";
/// Nom d'analyse shell du dossier Réseau (énumération, icône, session).
pub const NETWORK_ID: &str = "::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}";
/// Page d'accueil dans l'historique sauvegardé (pas un chemin réel).
pub const HOME_ID: &str = "folder:home";
/// Paramètres dans un onglet (session, historique ; pas un chemin réel).
pub const SETTINGS_ID: &str = "folder:settings";
/// Historique de navigation dans un onglet (session, historique ; pas un chemin réel).
pub const HISTORY_ID: &str = "folder:history";
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
    /// Analyse de l'espace de `root`, ouverte sur son dossier `dir`.
    DiskUsage {
        root: PathBuf,
        dir: PathBuf,
    },
    /// Paramètres de folder.
    Settings,
    /// Historique de navigation (dossiers visités, toutes fenêtres confondues).
    History,
}

impl Location {
    /// Dossier réel affiché, s'il y en a un.
    pub fn dir(&self) -> Option<&Path> {
        match self {
            Self::Dir(p) => Some(p),
            Self::Search { root, .. } => Some(root),
            Self::DiskUsage { dir, .. } => Some(dir),
            Self::Home
            | Self::Tag(_)
            | Self::RecycleBin
            | Self::Network(_)
            | Self::Settings
            | Self::History => None,
        }
    }

    /// Emplacement désigné par un chemin saisi ou sauvegardé : identifiants shell de la
    /// Corbeille et du Réseau, ordinateur du réseau seul (`\\SERVEUR`), sinon un dossier.
    pub fn from_path(path: PathBuf) -> Self {
        let s = path.to_string_lossy();
        if s == HOME_ID {
            return Self::Home;
        }
        if s == SETTINGS_ID {
            return Self::Settings;
        }
        if s == HISTORY_ID {
            return Self::History;
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
            Self::Settings => Some(PathBuf::from(SETTINGS_ID)),
            Self::History => Some(PathBuf::from(HISTORY_ID)),
            Self::Home | Self::Search { .. } | Self::Tag(_) | Self::DiskUsage { .. } => None,
        }
    }

    /// Argument de `folder.exe` qui ouvre une fenêtre sur cet emplacement (relu par
    /// [`Location::from_path`]) : une recherche rouvre son dossier, une étiquette l'accueil.
    pub fn window_arg(&self) -> PathBuf {
        match self {
            Self::Search { root, .. } => root.clone(),
            Self::DiskUsage { dir, .. } => dir.clone(),
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
            Self::Settings => "Paramètres".into(),
            Self::History => "Historique".into(),
            Self::Dir(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string()),
            Self::Search { query, .. } => format!("Recherche « {query} »"),
            Self::Tag(_) => "Étiquette".into(),
            Self::RecycleBin => "Corbeille".into(),
            Self::Network(None) => "Réseau".into(),
            Self::Network(Some(s)) => s.trim_start_matches('\\').to_owned(),
            Self::DiskUsage { dir, .. } => format!(
                "Espace · {}",
                dir.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| dir.display().to_string())
            ),
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
    /// chaque côté ; recherches, étiquettes et analyses, non restaurables, sont omises.
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

/// Historique de navigation : dossiers visités, du plus récent au plus ancien, un seul par
/// dossier (casse ASCII ignorée, comme `COLLATE NOCASE` dans la base), au plus
/// [`fx_store::MAX_VISITS`]. Relu de la base à l'ouverture de l'onglet Historique (les autres
/// fenêtres y écrivent aussi), puis tenu à jour à chaque visite.
#[derive(Debug, Default)]
pub struct Visits {
    entries: Vec<fx_store::Visit>,
    /// Change à chaque modification : l'UI refait alors sa liste filtrée.
    revision: u64,
}

impl Visits {
    pub fn iter(&self) -> impl Iterator<Item = &fx_store::Visit> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Mémoire occupée (diagnostics F12).
    pub fn heap_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<fx_store::Visit>()
            + self
                .entries
                .iter()
                .map(|v| v.path.as_os_str().len())
                .sum::<usize>()
    }

    /// Visite de `path` à `at` : le dossier passe en tête.
    pub(crate) fn visit(&mut self, path: &Path, at: i64) {
        self.entries.retain(|v| !same_path(&v.path, path));
        self.entries.insert(
            0,
            fx_store::Visit {
                path: path.to_path_buf(),
                at,
            },
        );
        self.entries.truncate(fx_store::MAX_VISITS);
        self.revision += 1;
    }

    /// Retire ces dossiers ; renvoie ceux qui y étaient.
    pub(crate) fn remove(&mut self, paths: &[PathBuf]) -> Vec<PathBuf> {
        let mut removed = Vec::new();
        self.entries.retain(|v| {
            let hit = paths.iter().any(|p| same_path(&v.path, p));
            if hit {
                removed.push(v.path.clone());
            }
            !hit
        });
        if !removed.is_empty() {
            self.revision += 1;
        }
        removed
    }

    pub(crate) fn clear(&mut self) {
        self.entries = Vec::new();
        self.revision += 1;
    }

    /// Liste relue de la base. Les visites plus récentes que la plus récente de la base
    /// restent en tête : leur écriture, sur un autre thread d'I/O, a pu passer après la
    /// lecture.
    pub(crate) fn replace(&mut self, loaded: Vec<fx_store::Visit>) {
        let newest = loaded.first().map_or(i64::MIN, |v| v.at);
        let mut entries: Vec<fx_store::Visit> = self
            .entries
            .iter()
            .filter(|v| v.at > newest)
            .cloned()
            .collect();
        let older: Vec<fx_store::Visit> = loaded
            .into_iter()
            .filter(|v| !entries.iter().any(|e| same_path(&e.path, &v.path)))
            .collect();
        entries.extend(older);
        entries.truncate(fx_store::MAX_VISITS);
        self.entries = entries;
        self.revision += 1;
    }
}

/// Même dossier pour l'historique de navigation (casse ASCII ignorée, comme la base).
fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
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
    fn visits_move_to_front_and_merge_with_the_db() {
        let p = PathBuf::from;
        let at = |v: &Visits| {
            v.iter()
                .map(|v| (v.path.display().to_string(), v.at))
                .collect::<Vec<_>>()
        };
        let mut v = Visits::default();
        v.visit(&p(r"C:\A"), 1);
        v.visit(&p(r"C:\B"), 2);
        v.visit(&p(r"c:\a"), 3);
        assert_eq!(at(&v), [(r"c:\a".into(), 3), (r"C:\B".into(), 2)]);
        // Base relue sans la visite 3 (écriture pas encore faite) : elle reste en tête.
        let r = v.revision();
        v.replace(vec![
            fx_store::Visit {
                path: p(r"C:\Z"),
                at: 2,
            },
            fx_store::Visit {
                path: p(r"C:\A"),
                at: 1,
            },
        ]);
        assert!(v.revision() > r);
        assert_eq!(at(&v), [(r"c:\a".into(), 3), (r"C:\Z".into(), 2)]);
        assert_eq!(
            v.remove(&[p(r"C:\A"), p(r"C:\nope")]),
            [p(r"c:\a")],
            "chemin retiré tel qu'enregistré"
        );
        assert_eq!(v.len(), 1);
        v.clear();
        assert!(v.is_empty());
    }

    #[test]
    fn history_location_round_trips() {
        assert_eq!(
            Location::from_path(Location::History.session_path().expect("path")),
            Location::History
        );
        assert_eq!(Location::History.dir(), None);
    }

    #[test]
    fn titles() {
        assert_eq!(d(r"C:\Users\moi").title(), "moi");
        assert_eq!(d(r"C:\").title(), r"C:\");
        assert_eq!(Location::Home.title(), "Accueil");
    }
}
