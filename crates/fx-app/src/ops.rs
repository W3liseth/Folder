//! Centre d'opérations (copies, déplacements, suppressions… en cours) et historique
//! d'annulation.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use fx_win::fileop::FileOp;

/// Travail long exécuté sur un thread dédié.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    File(FileOp),
    Compress {
        sources: Vec<PathBuf>,
        dest: PathBuf,
    },
    Extract {
        archive: PathBuf,
        dest: PathBuf,
    },
    /// Remet des éléments de la corbeille à leur emplacement d'origine.
    RestoreRecycled(Vec<fx_fs::recycle::Recycled>),
    /// Supprime définitivement des éléments de la corbeille.
    PurgeRecycled(Vec<fx_fs::recycle::Recycled>),
}

impl Job {
    /// Titre affiché dans le centre d'opérations.
    pub fn title(&self) -> String {
        let n = |v: &[PathBuf]| match v {
            [one] => format!("« {} »", name(one)),
            v => format!("{} éléments", v.len()),
        };
        match self {
            Job::File(FileOp::Copy { sources, dest }) => {
                format!("Copie de {} vers {}", n(sources), name(dest))
            }
            Job::File(FileOp::Move { sources, dest }) => {
                format!("Déplacement de {} vers {}", n(sources), name(dest))
            }
            Job::File(FileOp::Delete {
                items,
                permanent: true,
            }) => format!("Suppression de {}", n(items)),
            Job::File(FileOp::Delete {
                items,
                permanent: false,
            }) => format!("Mise à la corbeille de {}", n(items)),
            Job::File(FileOp::Rename { item, new_name }) => {
                format!("Renommage de « {} » en « {new_name} »", name(item))
            }
            Job::File(FileOp::RenameMany { items }) => {
                format!("Renommage de {} éléments", items.len())
            }
            Job::File(FileOp::NewFolder { name: n, .. }) => format!("Création du dossier « {n} »"),
            Job::File(FileOp::NewFile { name: n, .. }) => format!("Création du fichier « {n} »"),
            Job::Compress { sources, dest } => {
                format!("Compression de {} dans « {} »", n(sources), name(dest))
            }
            Job::Extract { archive, .. } => format!("Extraction de « {} »", name(archive)),
            Job::RestoreRecycled(items) => match items.as_slice() {
                [one] => format!("Restauration de « {} »", name(&one.original)),
                v => format!("Restauration de {} éléments", v.len()),
            },
            Job::PurgeRecycled(items) => match items.as_slice() {
                [one] => format!("Suppression définitive de « {} »", name(&one.original)),
                v => format!("Suppression définitive de {} éléments", v.len()),
            },
            Job::File(FileOp::MoveRenamed { items }) => {
                format!("Déplacement de {} éléments", items.len())
            }
        }
    }

    pub fn item_count(&self) -> usize {
        match self {
            Job::File(FileOp::Copy { sources, .. } | FileOp::Move { sources, .. }) => sources.len(),
            Job::File(FileOp::Delete { items, .. }) => items.len(),
            Job::File(FileOp::RenameMany { items }) => items.len(),
            Job::Compress { sources, .. } => sources.len(),
            Job::RestoreRecycled(items) | Job::PurgeRecycled(items) => items.len(),
            _ => 1,
        }
    }

    /// Dossier où l'opération crée des éléments (pour les sélectionner ensuite).
    pub fn destination(&self) -> Option<&Path> {
        match self {
            Job::File(FileOp::Copy { dest, .. } | FileOp::Move { dest, .. }) => Some(dest),
            Job::Extract { dest, .. } => Some(dest),
            Job::Compress { dest, .. } => dest.parent(),
            _ => None,
        }
    }
}

pub(crate) fn name(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpStatus {
    Running,
    Done,
    Cancelled,
    Failed(String),
}

#[derive(Debug)]
pub struct Operation {
    pub id: u64,
    pub title: String,
    pub items: usize,
    pub done: u32,
    pub total: u32,
    pub current: String,
    pub status: OpStatus,
    pub started: Instant,
    pub finished: Option<Instant>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) job: Job,
    /// Éléments créés (copie, création) et déplacés (déplacement, renommage), pour l'annulation.
    pub(crate) created: Vec<PathBuf>,
    pub(crate) moved: Vec<(PathBuf, PathBuf)>,
    /// Faux pour les opérations lancées par Annuler (elles ne s'annulent pas elles-mêmes).
    pub(crate) record_undo: bool,
}

impl Operation {
    pub(crate) fn new(id: u64, job: Job, record_undo: bool) -> Self {
        Self {
            id,
            title: job.title(),
            items: job.item_count(),
            done: 0,
            total: 0,
            current: String::new(),
            status: OpStatus::Running,
            started: Instant::now(),
            finished: None,
            cancel: Arc::new(AtomicBool::new(false)),
            job,
            created: Vec::new(),
            moved: Vec::new(),
            record_undo,
        }
    }

    pub fn is_running(&self) -> bool {
        self.status == OpStatus::Running
    }

    /// Progression 0..=1, ou `None` si inconnue.
    pub fn fraction(&self) -> Option<f32> {
        (self.total > 0).then(|| (self.done as f32 / self.total as f32).clamp(0.0, 1.0))
    }

    pub(crate) fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Opérations inverses, si l'opération est annulable.
    pub(crate) fn inverse(&self) -> Option<Vec<Job>> {
        let jobs: Vec<Job> = match &self.job {
            Job::File(FileOp::Copy { .. } | FileOp::NewFolder { .. } | FileOp::NewFile { .. })
            | Job::Compress { .. } => {
                if self.created.is_empty() {
                    return None;
                }
                vec![Job::File(FileOp::Delete {
                    items: self.created.clone(),
                    permanent: false,
                })]
            }
            Job::File(FileOp::Move { .. } | FileOp::Rename { .. } | FileOp::RenameMany { .. }) => {
                if self.moved.is_empty() {
                    return None;
                }
                let mut renames = Vec::new();
                let mut moves: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
                for (from, to) in &self.moved {
                    if from.parent() == to.parent() {
                        renames.push((to.clone(), name(from)));
                    } else if let Some(dest) = from.parent() {
                        match moves.iter_mut().find(|(d, _)| d == dest) {
                            Some((_, v)) => v.push(to.clone()),
                            None => moves.push((dest.to_path_buf(), vec![to.clone()])),
                        }
                    }
                }
                let mut jobs: Vec<Job> = moves
                    .into_iter()
                    .map(|(dest, sources)| Job::File(FileOp::Move { sources, dest }))
                    .collect();
                if !renames.is_empty() {
                    jobs.push(Job::File(FileOp::RenameMany { items: renames }));
                }
                jobs
            }
            // La restauration depuis la corbeille et les suppressions définitives ne sont pas
            // annulables par fx.
            // La corbeille n'a pas d'annulation : restaurer / supprimer définitivement.
            Job::File(FileOp::Delete { .. } | FileOp::MoveRenamed { .. })
            | Job::Extract { .. }
            | Job::RestoreRecycled(_)
            | Job::PurgeRecycled(_) => return None,
        };
        Some(jobs)
    }
}

#[derive(Debug, Clone)]
pub struct UndoEntry {
    pub label: String,
    pub(crate) forward: Job,
    pub(crate) backward: Vec<Job>,
}

/// Historique Annuler / Rétablir (borné).
#[derive(Debug, Default)]
pub struct UndoStack {
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
}

const MAX_UNDO: usize = 50;

impl UndoStack {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    pub(crate) fn push(&mut self, e: UndoEntry) {
        if self.undo.len() == MAX_UNDO {
            self.undo.remove(0);
        }
        self.undo.push(e);
        self.redo.clear();
    }

    pub(crate) fn pop_undo(&mut self) -> Option<UndoEntry> {
        let e = self.undo.pop()?;
        self.redo.push(e.clone());
        Some(e)
    }

    pub(crate) fn pop_redo(&mut self) -> Option<UndoEntry> {
        self.redo.pop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_of_move_and_rename() {
        let mut op = Operation::new(
            1,
            Job::File(FileOp::Move {
                sources: vec![],
                dest: PathBuf::from(r"D:\b"),
            }),
            true,
        );
        op.moved
            .push((PathBuf::from(r"C:\a\x.txt"), PathBuf::from(r"D:\b\x.txt")));
        op.moved.push((
            PathBuf::from(r"D:\b\old.txt"),
            PathBuf::from(r"D:\b\new.txt"),
        ));
        let inv = op.inverse().expect("inverse");
        assert_eq!(
            inv,
            vec![
                Job::File(FileOp::Move {
                    sources: vec![PathBuf::from(r"D:\b\x.txt")],
                    dest: PathBuf::from(r"C:\a")
                }),
                Job::File(FileOp::RenameMany {
                    items: vec![(PathBuf::from(r"D:\b\new.txt"), "old.txt".into())]
                }),
            ]
        );
    }

    #[test]
    fn copy_undo_recycles_created_and_titles() {
        let job = Job::File(FileOp::Copy {
            sources: vec![PathBuf::from(r"C:\a.txt")],
            dest: PathBuf::from(r"D:\Docs"),
        });
        assert_eq!(job.title(), "Copie de « a.txt » vers Docs");
        let mut op = Operation::new(1, job, true);
        assert!(op.inverse().is_none());
        op.created.push(PathBuf::from(r"D:\Docs\a.txt"));
        assert_eq!(
            op.inverse(),
            Some(vec![Job::File(FileOp::Delete {
                items: vec![PathBuf::from(r"D:\Docs\a.txt")],
                permanent: false
            })])
        );
    }

    #[test]
    fn undo_stack_moves_entries_to_redo() {
        let mut s = UndoStack::default();
        let e = UndoEntry {
            label: "x".into(),
            forward: Job::File(FileOp::NewFolder {
                parent: PathBuf::new(),
                name: "n".into(),
            }),
            backward: vec![],
        };
        s.push(e);
        assert!(s.can_undo() && !s.can_redo());
        s.pop_undo();
        assert!(!s.can_undo() && s.can_redo());
        assert!(s.pop_redo().is_some());
    }
}
