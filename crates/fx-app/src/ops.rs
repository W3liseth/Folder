//! Centre d'état (copies, déplacements, suppressions… en cours, et historique des
//! opérations terminées) et historique d'annulation.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use fx_store::{OpEntry, OpOutcome};
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

    /// Chemins dont l'opération a pu changer l'état (sources, destinations, éléments créés),
    /// relus à sa fin pour mettre les listes à jour sans dépendre de la surveillance.
    pub(crate) fn touched(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = Vec::new();
        let renamed = |item: &Path, name: &str| item.parent().map(|d| d.join(name));
        match &self.job {
            Job::File(FileOp::Copy { sources, dest }) => {
                v.extend(
                    sources
                        .iter()
                        .filter_map(|s| s.file_name().map(|n| dest.join(n))),
                );
            }
            Job::File(FileOp::Move { sources, dest }) => {
                v.extend(sources.iter().cloned());
                v.extend(
                    sources
                        .iter()
                        .filter_map(|s| s.file_name().map(|n| dest.join(n))),
                );
            }
            Job::File(FileOp::Delete { items, .. }) => v.extend(items.iter().cloned()),
            Job::File(FileOp::Rename { item, new_name }) => {
                v.push(item.clone());
                v.extend(renamed(item, new_name));
            }
            Job::File(FileOp::RenameMany { items }) => {
                for (item, name) in items {
                    v.push(item.clone());
                    v.extend(renamed(item, name));
                }
            }
            Job::File(FileOp::MoveRenamed { items }) => {
                for (item, dir, name) in items {
                    v.push(item.clone());
                    v.push(dir.join(name));
                }
            }
            Job::File(FileOp::NewFolder { parent, name } | FileOp::NewFile { parent, name }) => {
                v.push(parent.join(name));
            }
            Job::Compress { dest, .. } | Job::Extract { dest, .. } => v.push(dest.clone()),
            Job::RestoreRecycled(items) => v.extend(items.iter().map(|r| r.original.clone())),
            Job::PurgeRecycled(_) => {}
        }
        for (from, to) in &self.moved {
            v.push(from.clone());
            v.push(to.clone());
        }
        v.extend(self.created.iter().cloned());
        v
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

/// Opération terminée, dans l'historique du centre d'état. Ne garde que ce qui s'affiche :
/// les chemins créés ou déplacés (jusqu'à des milliers) restent dans l'entrée d'annulation.
/// L'historique est enregistré dans `folder.db` (partagé par toutes les fenêtres) ; `id` est
/// propre à ce processus, la base identifie une opération par `(ended, title)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpRecord {
    pub id: u64,
    pub title: String,
    pub items: usize,
    pub status: OpStatus,
    pub duration: Duration,
    /// Fin de l'opération (FILETIME UTC), pour afficher l'heure.
    pub ended: i64,
}

impl OpRecord {
    pub(crate) fn of(op: &Operation, ended: i64) -> Self {
        Self {
            id: op.id,
            title: op.title.clone(),
            items: op.items,
            status: op.status.clone(),
            duration: op.started.elapsed(),
            ended,
        }
    }

    pub(crate) fn entry(&self) -> OpEntry {
        OpEntry {
            ended: self.ended,
            title: self.title.clone(),
            items: u32::try_from(self.items).unwrap_or(u32::MAX),
            outcome: match &self.status {
                OpStatus::Failed(e) => OpOutcome::Failed(e.clone()),
                OpStatus::Cancelled => OpOutcome::Cancelled,
                OpStatus::Done | OpStatus::Running => OpOutcome::Done,
            },
            duration_ms: u64::try_from(self.duration.as_millis()).unwrap_or(u64::MAX),
        }
    }

    fn from_entry(id: u64, e: OpEntry) -> Self {
        Self {
            id,
            title: e.title,
            items: e.items as usize,
            status: match e.outcome {
                OpOutcome::Done => OpStatus::Done,
                OpOutcome::Cancelled => OpStatus::Cancelled,
                OpOutcome::Failed(err) => OpStatus::Failed(err),
            },
            duration: Duration::from_millis(e.duration_ms),
            ended: e.ended,
        }
    }
}

/// Historique des opérations terminées (borné, la plus récente en dernier).
#[derive(Debug, Default)]
pub struct OpHistory {
    entries: VecDeque<OpRecord>,
    /// Fin de la dernière opération vue dans le centre d'état (pastille d'échec).
    seen: i64,
    /// Premier chargement depuis la base reçu (ce qui précède ce lancement est déjà vu).
    loaded: bool,
}

pub const MAX_HISTORY: usize = fx_store::MAX_OP_HISTORY;

impl OpHistory {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Du plus récent au plus ancien.
    pub fn iter(&self) -> impl Iterator<Item = &OpRecord> {
        self.entries.iter().rev()
    }

    /// Identifiant de l'opération terminée la plus récente.
    pub fn last_id(&self) -> Option<u64> {
        self.entries.back().map(|r| r.id)
    }

    /// Un échec terminé depuis la dernière ouverture du centre d'état.
    pub fn has_unseen_failure(&self) -> bool {
        self.iter()
            .take_while(|r| r.ended > self.seen)
            .any(|r| matches!(r.status, OpStatus::Failed(_)))
    }

    pub(crate) fn mark_seen(&mut self) {
        self.seen = self.seen.max(self.entries.back().map_or(0, |r| r.ended));
    }

    /// Fin à enregistrer pour une opération qui se termine à `now` : strictement après la
    /// précédente, pour que `(ended, title)` reste unique.
    pub(crate) fn next_end(&self, now: i64) -> i64 {
        self.entries.back().map_or(now, |r| now.max(r.ended + 1))
    }

    pub(crate) fn push(&mut self, r: OpRecord) {
        if self.entries.len() == MAX_HISTORY {
            self.entries.pop_front();
        }
        self.entries.push_back(r);
    }

    pub(crate) fn remove(&mut self, id: u64) -> Option<OpRecord> {
        let i = self.entries.iter().position(|r| r.id == id)?;
        self.entries.remove(i)
    }

    pub(crate) fn clear(&mut self) {
        self.entries = VecDeque::new();
    }

    /// Remplace l'historique par celui de la base (`entries` : du plus récent au plus
    /// ancien). Les opérations déjà affichées gardent leur identifiant.
    pub(crate) fn replace(&mut self, entries: Vec<OpEntry>, mut next_id: impl FnMut() -> u64) {
        let old = std::mem::take(&mut self.entries);
        self.entries = entries
            .into_iter()
            .rev()
            .map(|e| {
                let id = old
                    .iter()
                    .find(|r| r.ended == e.ended && r.title == e.title)
                    .map_or_else(&mut next_id, |r| r.id);
                OpRecord::from_entry(id, e)
            })
            .collect();
        if !self.loaded {
            self.loaded = true;
            self.mark_seen();
        }
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
    fn history_is_bounded_and_newest_first() {
        let mut h = OpHistory::default();
        for id in 0..(MAX_HISTORY as u64 + 5) {
            h.push(OpRecord {
                id,
                title: String::new(),
                items: 1,
                status: OpStatus::Done,
                duration: Duration::ZERO,
                ended: 0,
            });
        }
        assert_eq!(h.len(), MAX_HISTORY);
        assert_eq!(h.iter().next().map(|r| r.id), Some(MAX_HISTORY as u64 + 4));
        assert_eq!(h.iter().last().map(|r| r.id), Some(5));
        h.remove(10);
        assert_eq!(h.len(), MAX_HISTORY - 1);
        h.clear();
        assert!(h.is_empty() && h.last_id().is_none());
    }

    #[test]
    fn reload_keeps_ids_and_tracks_unseen_failures() {
        let rec = |id, ended, status| OpRecord {
            id,
            title: format!("op {ended}"),
            items: 1,
            status,
            duration: Duration::from_millis(1500),
            ended,
        };
        let mut h = OpHistory::default();
        let old_failure = rec(0, 5, OpStatus::Failed("x".into()));
        let done = rec(0, 10, OpStatus::Done);
        let mut ids = 100..;
        // Premier chargement : ce qui précède ce lancement est déjà vu.
        h.replace(vec![done.entry(), old_failure.entry()], || {
            ids.next().expect("id")
        });
        assert_eq!(h.iter().map(|r| r.id).collect::<Vec<_>>(), [101, 100]);
        assert_eq!(
            h.iter().next(),
            Some(&OpRecord {
                id: 101,
                ..done.clone()
            })
        );
        assert!(!h.has_unseen_failure());
        assert_eq!(h.next_end(3), 11, "toujours après la précédente");

        h.push(rec(7, 20, OpStatus::Failed("Accès refusé.".into())));
        assert!(h.has_unseen_failure());
        // Rechargement (une autre fenêtre a ajouté une opération) : identifiants conservés.
        let other = rec(0, 30, OpStatus::Done);
        let entries: Vec<OpEntry> = std::iter::once(other.entry())
            .chain(h.iter().map(OpRecord::entry))
            .collect();
        h.replace(entries, || ids.next().expect("id"));
        assert_eq!(
            h.iter().map(|r| r.id).collect::<Vec<_>>(),
            [102, 7, 101, 100]
        );
        assert!(
            h.has_unseen_failure(),
            "un rechargement ne vaut pas lecture"
        );
        h.mark_seen();
        assert!(!h.has_unseen_failure());
        assert_eq!(h.remove(7).map(|r| r.ended), Some(20));
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
