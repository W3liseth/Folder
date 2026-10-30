use std::ops::ControlFlow;
use std::path::Path;
use std::time::{Duration, Instant};

use fx_core::entry::attrs;
use fx_core::{Batch, CancelToken};

/// Premier lot : petit et rapide, pour afficher quelque chose en moins d'une image.
const FIRST_BATCH: usize = 256;
const FIRST_BATCH_MAX_DELAY: Duration = Duration::from_millis(8);
/// Lots suivants : taille doublée à chaque envoi, pour limiter le nombre de fusions côté UI.
const MAX_BATCH: usize = 16_384;
const BATCH_MAX_DELAY: Duration = Duration::from_millis(50);
/// Fréquence (en entrées) des vérifications d'horloge et d'annulation.
const CHECK_EVERY: usize = 64;

#[derive(Debug, Clone, Copy, Default)]
pub struct EnumOptions {
    pub show_hidden: bool,
    /// Taille sur disque (`AllocationSize`) à la place de la taille du contenu (analyse de
    /// l'espace) : lue par `GetFileInformationByHandleEx`.
    pub allocated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumStats {
    pub entries: u32,
    pub batches: u32,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Completed(EnumStats),
    Cancelled,
}

/// Énumère `dir` et envoie les entrées par lots à `emit`, au fil de l'eau.
///
/// Les lots grossissent (256, 512, … 16 384) et partent au plus tard après 8 ms (premier)
/// ou 50 ms (suivants). L'annulation est vérifiée toutes les 64 entrées.
pub fn enumerate(
    dir: &Path,
    opts: &EnumOptions,
    cancel: &CancelToken,
    mut emit: impl FnMut(Batch),
) -> Result<Outcome, fx_win::Error> {
    let start = Instant::now();
    let mut target = FIRST_BATCH;
    let mut max_delay = FIRST_BATCH_MAX_DELAY;
    let mut batch = Batch::with_capacity(target);
    let mut last_flush = start;
    let (mut total, mut batches, mut seen) = (0u32, 0u32, 0usize);
    let mut cancelled = false;

    let mut on_entry = |name: &[u16], attributes: u32, size: u64, created: i64, modified: i64| {
        seen += 1;
        if !opts.show_hidden && attributes & attrs::HIDDEN != 0 {
            return ControlFlow::Continue(());
        }
        batch.push_utf16(name, attributes, size, created, modified);

        let full = batch.len() >= target;
        if full || seen % CHECK_EVERY == 0 {
            if cancel.is_cancelled() {
                cancelled = true;
                return ControlFlow::Break(());
            }
            if full || (!batch.is_empty() && last_flush.elapsed() >= max_delay) {
                total += batch.len() as u32;
                batches += 1;
                target = (target * 2).min(MAX_BATCH);
                max_delay = BATCH_MAX_DELAY;
                emit(std::mem::replace(&mut batch, Batch::with_capacity(target)));
                last_flush = Instant::now();
            }
        }
        ControlFlow::Continue(())
    };
    if opts.allocated {
        let mut buf = fx_win::dir::DirBuffer::default();
        fx_win::dir::for_each_sized_entry(dir, &mut buf, |e| {
            on_entry(e.name, e.attributes, e.allocated, e.created, e.modified)
        })?;
    } else {
        fx_win::dir::for_each_entry(dir, |e| {
            on_entry(e.name, e.attributes, e.size, e.created, e.modified)
        })?;
    }

    if cancelled || cancel.is_cancelled() {
        return Ok(Outcome::Cancelled);
    }
    if !batch.is_empty() {
        total += batch.len() as u32;
        batches += 1;
        emit(batch);
    }
    Ok(Outcome::Completed(EnumStats {
        entries: total,
        batches,
        elapsed: start.elapsed(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("fx-fs-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).expect("mkdir");
            Self(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn streams_every_entry_in_growing_batches() {
        let dir = TempDir::new("stream");
        for i in 0..1_000 {
            fs::write(dir.0.join(format!("f{i}.txt")), b"x").expect("write");
        }
        fs::create_dir(dir.0.join("sub")).expect("mkdir");

        let mut sizes = Vec::new();
        let out = enumerate(&dir.0, &EnumOptions::default(), &CancelToken::new(), |b| {
            sizes.push(b.len())
        })
        .expect("enumerate");
        let Outcome::Completed(stats) = out else {
            panic!("cancelled")
        };
        assert_eq!(stats.entries, 1_001);
        assert_eq!(sizes.iter().sum::<usize>(), 1_001);
        assert!(sizes[0] <= FIRST_BATCH);
        assert_eq!(stats.batches as usize, sizes.len());
    }

    #[test]
    fn cancelled_before_start_emits_nothing() {
        let dir = TempDir::new("cancel");
        for i in 0..200 {
            fs::write(dir.0.join(format!("f{i}")), b"").expect("write");
        }
        let cancel = CancelToken::new();
        cancel.cancel();
        let mut emitted = 0;
        let out = enumerate(&dir.0, &EnumOptions::default(), &cancel, |_| emitted += 1)
            .expect("enumerate");
        assert_eq!(out, Outcome::Cancelled);
        assert_eq!(emitted, 0);
    }

    #[test]
    fn missing_dir_errors() {
        let err = enumerate(
            Path::new(r"C:\fx-nope-123"),
            &EnumOptions::default(),
            &CancelToken::new(),
            |_| {},
        )
        .expect_err("missing");
        assert!(err.is_not_found());
    }

    /// Le tri naturel pur Rust doit suivre `StrCmpLogicalW` sur les noms alphanumériques.
    #[test]
    fn natural_sort_matches_explorer_on_alphanumeric_names() {
        let names = [
            "a", "A1", "a01", "a2", "a10", "a100", "ab", "Ab2", "b", "file 2", "file 10", "file1",
            "x9y", "x10y", "x9z", "IMG 0001", "IMG 0010", "img 0002", "z", "Zz", "10", "9", "1a",
            "Résumé", "resume", "Ébauche", "eau", "x01", "x1", "a001", "a01b", "a1a", "010",
        ];
        for a in names {
            for b in names {
                let explorer = fx_win::sort::explorer_cmp(a, b);
                if explorer.is_ne() {
                    assert_eq!(
                        fx_core::natural_cmp(a.as_bytes(), b.as_bytes()),
                        explorer,
                        "{a:?} vs {b:?}"
                    );
                }
            }
        }
    }
}
