//! Taille et contenu d'une sélection (fenêtre Propriétés) : parcours récursif annulable, avec
//! résultats partiels réguliers pour les gros dossiers.

use std::ops::ControlFlow;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fx_core::CancelToken;
use fx_core::entry::attrs;

/// Totaux d'une sélection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub size: u64,
    pub files: u64,
    pub folders: u64,
}

/// Mesure `paths` (fichiers et dossiers, sous-dossiers compris ; les liens et jonctions ne
/// sont pas suivis, comme l'Explorateur). `progress` reçoit des totaux partiels toutes les
/// `every`. Renvoie `None` si annulé.
pub fn measure(
    paths: &[PathBuf],
    cancel: &CancelToken,
    every: Duration,
    mut progress: impl FnMut(Totals),
) -> Option<Totals> {
    let mut t = Totals::default();
    let mut stack: Vec<PathBuf> = Vec::new();
    for p in paths {
        let Some(st) = fx_win::sys::stat(p) else {
            continue;
        };
        if st.attributes & attrs::DIRECTORY != 0 {
            t.folders += 1;
            if st.attributes & attrs::REPARSE_POINT == 0 {
                stack.push(p.clone());
            }
        } else {
            t.files += 1;
            t.size += st.size;
        }
    }
    let mut last = Instant::now();
    while let Some(dir) = stack.pop() {
        if cancel.is_cancelled() {
            return None;
        }
        let _ = fx_win::dir::for_each_entry(&dir, |e| {
            if e.attributes & attrs::DIRECTORY != 0 {
                t.folders += 1;
                if e.attributes & attrs::REPARSE_POINT == 0 {
                    stack.push(dir.join(std::ffi::OsString::from_wide(e.name)));
                }
            } else {
                t.files += 1;
                t.size += e.size;
            }
            ControlFlow::Continue(())
        });
        if last.elapsed() >= every {
            last = Instant::now();
            progress(t);
        }
    }
    Some(t)
}

/// Raccourci pour un seul chemin.
pub fn measure_one(path: &Path, cancel: &CancelToken) -> Option<Totals> {
    measure(&[path.to_path_buf()], cancel, Duration::MAX, |_| {})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_nested_files_and_folders() {
        let root = std::env::temp_dir().join(format!("fx-measure-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a").join("b")).expect("mkdir");
        std::fs::write(root.join("x.txt"), [0u8; 10]).expect("write");
        std::fs::write(root.join("a").join("y.txt"), [0u8; 20]).expect("write");
        std::fs::write(root.join("a").join("b").join("z.txt"), [0u8; 30]).expect("write");
        let t = measure_one(&root, &CancelToken::new()).expect("mesure");
        // Le dossier mesuré compte lui-même, comme deux sous-dossiers.
        assert_eq!(
            t,
            Totals {
                size: 60,
                files: 3,
                folders: 3
            }
        );
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(measure_one(&root, &cancel), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
