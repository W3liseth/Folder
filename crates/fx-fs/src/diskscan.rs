//! Analyse de l'espace : parcours récursif parallèle d'un dossier ou d'un lecteur, qui
//! construit un [`DiskTree`] avec la taille sur disque de chaque élément.
//!
//! Le thread appelant coordonne : il seul écrit l'arbre (identifiants attribués sans
//! verrou) ; au plus [`MAX_THREADS`] threads temporaires lisent les dossiers et lui
//! renvoient leur contenu. Les points d'analyse (jonctions, liens, points de montage) ne
//! sont pas suivis, comme l'Explorateur : un autre volume monté n'est pas compté deux fois.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::RecvTimeoutError;
use fx_core::disktree::{self, DirListing, ListedDir, ListedFile, MAX_NODES, NodeKind};
use fx_core::entry::attrs;
use fx_core::{CancelToken, DiskTree, wtf8};
use fx_win::dir::DirBuffer;

/// Threads de lecture d'une analyse (temporaires : ils se terminent avec elle).
pub const MAX_THREADS: usize = 4;
/// Vérification d'annulation pendant la lecture d'un très grand dossier.
const CHECK_EVERY: usize = 4096;
/// Fichiers gardés un par un au-delà desquels le seuil passe à [`RAISED_KEEP`] pour la
/// suite de l'analyse : borne la mémoire d'un très grand dossier (profil, partage) dont la
/// taille n'est pas connue d'avance.
const FILE_NODE_BUDGET: usize = 100_000;
const RAISED_KEEP: u64 = 1 << 20;

/// Avancement d'une analyse en cours.
#[derive(Debug, Clone, Default)]
pub struct ScanProgress {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub denied: u32,
    /// Dernier dossier lu.
    pub current: PathBuf,
    /// Premier niveau sous la racine, du plus gros au plus petit (30 au plus).
    pub top: Vec<TopItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopItem {
    pub name: String,
    pub bytes: u64,
    pub dir: bool,
}

/// Seuil de fichier gardé un par un : de l'ordre d'un 200 000ᵉ de l'espace occupé (pour
/// qu'un lecteur entier tienne en mémoire), entre 4 Kio et 1 Mio ; 64 Kio quand le volume
/// à lire est inconnu (dossier).
pub fn keep_files_from(used_bytes: Option<u64>) -> u64 {
    match used_bytes {
        Some(u) => (u / 200_000).clamp(4 << 10, 1 << 20),
        None => 64 << 10,
    }
}

/// Lit le contenu direct de `dir` (taille sur disque). Un dossier illisible donne une
/// liste vide marquée `denied`.
///
/// Seuls les noms des dossiers et des fichiers gardés entrent dans la liste (et donc dans
/// l'arbre) : `scratch` reçoit les autres, le temps de lire leur extension.
pub fn list_dir(
    dir: &Path,
    keep_from: u64,
    buf: &mut DirBuffer,
    scratch: &mut Vec<u8>,
    cancel: &CancelToken,
) -> DirListing {
    let mut l = DirListing::default();
    let mut seen = 0usize;
    let r = fx_win::dir::for_each_sized_entry(dir, buf, |e| {
        seen += 1;
        if seen.is_multiple_of(CHECK_EVERY) && cancel.is_cancelled() {
            return ControlFlow::Break(());
        }
        if e.attributes & attrs::DIRECTORY != 0 {
            if e.attributes & attrs::REPARSE_POINT == 0 {
                let name = l.names.push_utf16(e.name);
                l.dirs.push(ListedDir {
                    name,
                    modified: e.modified,
                    attributes: e.attributes,
                });
            }
            return ControlFlow::Continue(());
        }
        scratch.clear();
        wtf8::push_utf16(scratch, e.name);
        if scratch.eq_ignore_ascii_case(b"CACHEDIR.TAG") {
            l.cache_tag = true;
        }
        let ext = disktree::extension_of(scratch);
        l.count_ext(&ext, e.allocated);
        if e.allocated >= keep_from {
            let name = l.names.push_bytes(scratch);
            l.files.push(ListedFile {
                name,
                size: e.allocated,
                modified: e.modified,
                attributes: e.attributes,
            });
        } else {
            l.small_files += 1;
            l.small_bytes += e.allocated;
        }
        ControlFlow::Continue(())
    });
    if r.is_err() {
        l.denied = true;
    }
    l
}

struct Work {
    node: u32,
    /// Contenu regroupé dans `node` (plafond de nœuds atteint).
    folded: bool,
    path: PathBuf,
}

#[derive(Default)]
struct Queue {
    items: Vec<Work>,
    closed: bool,
}

/// Analyse `root`. `progress` est appelé toutes les `every` ; `None` si annulé.
pub fn scan(
    root: &Path,
    keep_from: u64,
    threads: usize,
    cancel: &CancelToken,
    every: Duration,
    mut progress: impl FnMut(ScanProgress),
) -> Option<DiskTree> {
    let mut tree = DiskTree::new(root.to_path_buf(), keep_from);
    // Seuil courant, relevé en cours de route si l'arbre grossit trop (lu par les workers).
    let keep = AtomicU64::new(keep_from);
    let mut kept_files = 0usize;
    let queue = (Mutex::new(Queue::default()), Condvar::new());
    let (tx, rx) = crossbeam_channel::unbounded::<(Work, DirListing)>();
    let threads = threads.clamp(1, MAX_THREADS);
    let finished = std::thread::scope(|s| {
        for i in 0..threads {
            let tx = tx.clone();
            let (queue, keep) = (&queue, &keep);
            let spawned = std::thread::Builder::new()
                .name(format!("fx-scan-{i}"))
                .stack_size(256 * 1024)
                .spawn_scoped(s, move || worker(queue, &tx, keep, cancel));
            if spawned.is_err() && i == 0 {
                return false;
            }
        }
        drop(tx);
        let push = |items: Vec<Work>| {
            let (lock, cv) = &queue;
            let mut q = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            q.items.extend(items);
            cv.notify_all();
        };
        let close = || {
            let (lock, cv) = &queue;
            lock.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .closed = true;
            cv.notify_all();
        };
        push(vec![Work {
            node: DiskTree::root(),
            folded: false,
            path: root.to_path_buf(),
        }]);
        let mut pending = 1usize;
        let mut last = Instant::now();
        let mut current = root.to_path_buf();
        loop {
            match rx.recv_timeout(every.min(Duration::from_millis(100))) {
                Ok((work, listing)) => {
                    pending -= 1;
                    kept_files += listing.files.len();
                    if kept_files > FILE_NODE_BUDGET && keep.load(Ordering::Relaxed) < RAISED_KEEP {
                        keep.store(RAISED_KEEP, Ordering::Relaxed);
                        tree.set_keep_files_from(RAISED_KEEP);
                    }
                    let next = integrate(&mut tree, work, &listing);
                    pending += next.len();
                    if let Some(w) = next.last() {
                        current.clone_from(&w.path);
                    }
                    if !next.is_empty() {
                        push(next);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if cancel.is_cancelled() {
                close();
                return false;
            }
            if pending == 0 {
                close();
                return true;
            }
            if last.elapsed() >= every {
                last = Instant::now();
                progress(progress_of(&tree, &current));
            }
        }
        close();
        pending == 0
    });
    if !finished || cancel.is_cancelled() {
        return None;
    }
    tree.finish();
    Some(tree)
}

/// Range un dossier lu dans l'arbre ; renvoie ses sous-dossiers à lire.
fn integrate(tree: &mut DiskTree, work: Work, listing: &DirListing) -> Vec<Work> {
    let fold = work.folded || tree.len() + listing.dirs.len() + listing.files.len() + 2 > MAX_NODES;
    let sub = |d: &ListedDir| {
        work.path
            .join(wtf8::to_os_string(listing.names.get(d.name)))
    };
    if fold {
        tree.insert_folded(work.node, listing);
        return listing
            .dirs
            .iter()
            .map(|d| Work {
                node: work.node,
                folded: true,
                path: sub(d),
            })
            .collect();
    }
    let ids = tree.insert(work.node, listing);
    listing
        .dirs
        .iter()
        .zip(ids)
        .map(|(d, node)| Work {
            node,
            folded: false,
            path: sub(d),
        })
        .collect()
}

fn worker(
    queue: &(Mutex<Queue>, Condvar),
    tx: &crossbeam_channel::Sender<(Work, DirListing)>,
    keep: &AtomicU64,
    cancel: &CancelToken,
) {
    let mut buf = DirBuffer::default();
    let mut scratch = Vec::with_capacity(512);
    loop {
        let work = {
            let (lock, cv) = queue;
            let mut q = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if q.closed {
                    return;
                }
                // Dernier entré, premier lu : parcours en profondeur, file courte.
                if let Some(w) = q.items.pop() {
                    break w;
                }
                q = cv
                    .wait(q)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        if cancel.is_cancelled() {
            return;
        }
        let listing = list_dir(
            &work.path,
            keep.load(Ordering::Relaxed),
            &mut buf,
            &mut scratch,
            cancel,
        );
        if tx.send((work, listing)).is_err() {
            return;
        }
    }
}

fn progress_of(tree: &DiskTree, current: &Path) -> ScanProgress {
    let root = DiskTree::root();
    let mut top: Vec<TopItem> = tree
        .children(root)
        .map(|c| TopItem {
            name: match tree.kind(c) {
                NodeKind::SmallFiles => String::new(),
                _ => wtf8::display(tree.name(c)).into_owned(),
            },
            bytes: tree.size(c),
            dir: tree.kind(c) == NodeKind::Dir,
        })
        .collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.bytes));
    top.truncate(30);
    ScanProgress {
        files: u64::from(tree.files(root)),
        dirs: tree.dirs_read(),
        bytes: tree.size(root),
        denied: tree.denied(),
        current: current.to_path_buf(),
        top,
    }
}

/// Contenu direct relu d'un dossier déjà analysé, et ses nouveaux sous-dossiers lus en
/// entier (cf. [`DiskTree::refresh`]).
#[derive(Debug)]
pub struct Refreshed {
    pub listing: DirListing,
    pub fresh: Vec<(Vec<u8>, DiskTree)>,
}

/// Relit `dir` après une opération : `known` donne les sous-dossiers déjà dans l'arbre
/// (non relus). `None` si annulé.
pub fn refresh(
    dir: &Path,
    known: &[Vec<u8>],
    keep_from: u64,
    cancel: &CancelToken,
) -> Option<Refreshed> {
    let mut buf = DirBuffer::default();
    let listing = list_dir(dir, keep_from, &mut buf, &mut Vec::new(), cancel);
    let mut fresh = Vec::new();
    for d in &listing.dirs {
        let name = listing.names.get(d.name);
        if known.iter().any(|k| disktree::same_name(k, name)) {
            continue;
        }
        let path = dir.join(wtf8::to_os_string(name));
        let sub = scan(&path, keep_from, 2, cancel, Duration::MAX, |_| {})?;
        fresh.push((name.to_vec(), sub));
    }
    (!cancel.is_cancelled()).then_some(Refreshed { listing, fresh })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn temp_tree(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("fx-diskscan-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a").join("b")).expect("mkdir");
        std::fs::create_dir_all(root.join("vide")).expect("mkdir");
        std::fs::write(root.join("gros.bin"), vec![1u8; 300_000]).expect("w");
        std::fs::write(root.join("a").join("moyen.iso"), vec![1u8; 200_000]).expect("w");
        for i in 0..20 {
            std::fs::write(
                root.join("a").join("b").join(format!("p{i}.txt")),
                vec![1u8; 5000],
            )
            .expect("w");
        }
        root
    }

    #[test]
    fn scan_counts_every_file_with_its_size_on_disk() {
        let root = temp_tree("scan");
        let mut calls = 0;
        let tree = scan(
            &root,
            64 << 10,
            3,
            &CancelToken::new(),
            Duration::ZERO,
            |_| calls += 1,
        )
        .expect("scan");
        assert_eq!(tree.files(0), 22);
        // Taille sur disque : au moins le contenu, arrondie aux clusters.
        assert!(
            tree.size(0) >= 300_000 + 200_000 + 20 * 5000,
            "{}",
            tree.size(0)
        );
        let big = tree.find(&root.join("gros.bin")).expect("gros");
        assert_eq!(tree.kind(big), NodeKind::File);
        let b = tree.find(&root.join("a").join("b")).expect("b");
        // Petits fichiers regroupés dans un seul nœud.
        assert_eq!(tree.children(b).count(), 1);
        assert_eq!(tree.files(b), 20);
        assert!(tree.find(&root.join("vide")).is_some());
        let first = tree.children(0).next().expect("enfant");
        assert!(tree.size(first) >= tree.children(0).map(|c| tree.size(c)).max().unwrap_or(0));
        assert!(calls > 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cancelled_scan_returns_nothing() {
        let root = temp_tree("cancel");
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(scan(&root, 1, 2, &cancel, Duration::MAX, |_| {}).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refresh_reads_only_new_subdirs() {
        let root = temp_tree("refresh");
        let mut tree = scan(
            &root,
            64 << 10,
            2,
            &CancelToken::new(),
            Duration::MAX,
            |_| {},
        )
        .expect("scan");
        std::fs::remove_file(root.join("gros.bin")).expect("rm");
        std::fs::create_dir_all(root.join("neuf")).expect("mkdir");
        std::fs::write(root.join("neuf").join("n.bin"), vec![1u8; 100_000]).expect("w");
        let known = tree.subdir_names(0);
        let r = refresh(&root, &known, 64 << 10, &CancelToken::new()).expect("refresh");
        assert_eq!(r.fresh.len(), 1);
        tree.refresh(0, &r.listing, &r.fresh);
        assert!(tree.find(&root.join("gros.bin")).is_none());
        assert!(tree.find(&root.join("neuf").join("n.bin")).is_some());
        assert_eq!(tree.files(0), 22);
        let _ = std::fs::remove_dir_all(&root);
    }
}
