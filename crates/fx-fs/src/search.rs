//! Recherche récursive par nom, diffusée par lots (un lot par dossier contenant des
//! correspondances), annulable.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::Instant;

use fx_core::entry::attrs;
use fx_core::{Batch, CancelToken};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchStats {
    pub matches: u32,
    pub dirs_scanned: u32,
    pub elapsed_ms: u64,
}

/// Limite de résultats : au-delà, l'utilisateur doit affiner sa requête.
pub const MAX_RESULTS: u32 = 50_000;

/// Parcourt `root` en profondeur et émet les entrées dont le nom contient `query`
/// (insensible à la casse). Les points d'analyse (jonctions, liens) ne sont pas suivis.
pub fn search(
    root: &Path,
    query: &str,
    show_hidden: bool,
    cancel: &CancelToken,
    mut emit: impl FnMut(Batch),
) -> Option<SearchStats> {
    let start = Instant::now();
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    if needle.is_empty() {
        return None;
    }
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    let (mut matches, mut dirs_scanned) = (0u32, 0u32);
    while let Some(dir) = stack.pop() {
        if cancel.is_cancelled() {
            return None;
        }
        dirs_scanned += 1;
        let mut batch = Batch::default();
        let mut subdirs = Vec::new();
        let _ = fx_win::dir::for_each_entry(&dir, |e| {
            if !show_hidden && e.attributes & attrs::HIDDEN != 0 {
                return ControlFlow::Continue(());
            }
            let name = String::from_utf16_lossy(e.name);
            let is_dir = e.attributes & attrs::DIRECTORY != 0;
            if is_dir && e.attributes & attrs::REPARSE_POINT == 0 {
                subdirs.push(dir.join(&name));
            }
            if contains_ci(&name, &needle) {
                batch.push_utf16(e.name, e.attributes, e.size, e.created, e.modified);
            }
            ControlFlow::Continue(())
        });
        if !batch.is_empty() {
            matches += batch.len() as u32;
            batch.dir = Some(dir);
            emit(batch);
            if matches >= MAX_RESULTS {
                break;
            }
        }
        // Ordre de parcours stable : on dépile dans l'ordre alphabétique.
        subdirs.sort_by(|a, b| b.cmp(a)); // ordre inverse : on dépile
        stack.extend(subdirs);
    }
    Some(SearchStats {
        matches,
        dirs_scanned,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

fn contains_ci(hay: &str, needle: &[char]) -> bool {
    let hay: Vec<char> = hay.chars().flat_map(char::to_lowercase).collect();
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nested_matches() {
        let root = std::env::temp_dir().join(format!("fx-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a").join("b")).expect("mkdir");
        std::fs::write(root.join("Rapport.txt"), b"").expect("w");
        std::fs::write(root.join("a").join("b").join("vieux RAPPORT.pdf"), b"").expect("w");
        std::fs::write(root.join("a").join("autre.txt"), b"").expect("w");
        let mut found = Vec::new();
        let stats = search(&root, "rapport", true, &CancelToken::new(), |b| {
            found.push((b.dir.clone().expect("dir"), b.len()));
        })
        .expect("stats");
        assert_eq!(stats.matches, 2);
        assert!(found.iter().any(|(d, _)| d.ends_with("b")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
