//! Analyse de l'espace (ARCHITECTURE.md §7.13) : état des analyses ouvertes.
//!
//! Une analyse appartient à la fenêtre, pas au volet : tous les volets qui regardent un
//! dossier sous sa racine la partagent, et Précédent/Suivant y reviennent sans relire le
//! disque. Leur nombre est borné ([`MAX_SCANS`]) : un lecteur système occupe 20 à 40 Mo.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fx_core::{CancelToken, DiskTree};
use fx_fs::cleanup::Suggestion;
use fx_fs::diskscan::ScanProgress;

/// Analyses gardées : au-delà, la plus ancienne qu'aucun volet n'affiche est libérée.
pub const MAX_SCANS: usize = 2;

/// Page de la vue d'analyse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiskTab {
    #[default]
    Content,
    Types,
    Cleanup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskStatus {
    Scanning,
    Done { elapsed: Duration },
    Cancelled,
}

#[derive(Debug)]
pub struct DiskScan {
    pub id: u64,
    pub root: PathBuf,
    pub started: Instant,
    pub status: DiskStatus,
    /// Dernier avancement reçu (pendant l'analyse).
    pub progress: ScanProgress,
    pub tree: Option<DiskTree>,
    pub suggestions: Vec<Suggestion>,
    /// Actualisations de dossiers en cours (après une opération).
    pub refreshing: u32,
    pub(crate) cancel: CancelToken,
}

impl DiskScan {
    pub fn is_scanning(&self) -> bool {
        self.status == DiskStatus::Scanning
    }

    /// Mémoire occupée (diagnostics F12).
    pub fn heap_bytes(&self) -> usize {
        self.tree.as_ref().map_or(0, DiskTree::heap_bytes)
            + self
                .suggestions
                .iter()
                .map(|s| {
                    s.paths
                        .iter()
                        .map(|p| p.as_os_str().len() + 32)
                        .sum::<usize>()
                })
                .sum::<usize>()
    }

    /// Recalcule les suggestions d'après l'arbre (éléments supprimés retirés, tailles à
    /// jour) ; celles devenues négligeables disparaissent.
    pub(crate) fn update_suggestions(&mut self) {
        let Some(tree) = &self.tree else { return };
        for s in &mut self.suggestions {
            s.paths.retain(|p| tree.find(p).is_some());
            s.bytes = s
                .paths
                .iter()
                .filter_map(|p| tree.find(p))
                .map(|n| tree.size(n))
                .sum();
        }
        self.suggestions.retain(|s| s.bytes >= 1 << 20);
        self.suggestions.sort_by_key(|s| std::cmp::Reverse(s.bytes));
    }
}

/// `path` est `root` ou se trouve dessous (sans tenir compte de la casse).
pub fn is_under(path: &Path, root: &Path) -> bool {
    let mut p = path.components();
    root.components().all(|r| {
        p.next().is_some_and(|c| {
            c.as_os_str()
                .as_encoded_bytes()
                .eq_ignore_ascii_case(r.as_os_str().as_encoded_bytes())
        })
    })
}

/// Racine de lecteur (`C:\`) ?
pub fn is_drive_root(path: &Path) -> bool {
    path.parent().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_ignores_case_and_needs_whole_components() {
        assert!(is_under(Path::new(r"C:\Users\Moi"), Path::new(r"c:\users")));
        assert!(is_under(Path::new(r"C:\Users"), Path::new(r"C:\Users")));
        assert!(!is_under(Path::new(r"C:\Users2"), Path::new(r"C:\Users")));
        assert!(!is_under(Path::new(r"C:\"), Path::new(r"C:\Users")));
        assert!(is_drive_root(Path::new(r"C:\")));
        assert!(!is_drive_root(Path::new(r"C:\Users")));
    }
}
