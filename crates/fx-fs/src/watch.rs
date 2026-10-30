//! Service de surveillance : **un seul thread** pour tous les dossiers affichés.
//!
//! Les notifications sont regroupées (au plus toutes les 50 ms), dédupliquées par nom, puis
//! complétées par les attributs actuels avant d'être remises à l'appelant.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use fx_win::sys::Stat;
use fx_win::watch::{Action, DirWatch, Port, Wait};

/// Délai maximal entre une notification et sa remise.
const FLUSH_AFTER: Duration = Duration::from_millis(50);
/// Clé réservée au réveil du thread pour traiter les commandes.
const WAKE: usize = 0;

/// Un changement dans un dossier surveillé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Entrée créée ou modifiée (attributs actuels).
    Upsert {
        name: Vec<u16>,
        stat: Stat,
    },
    Remove {
        name: Vec<u16>,
    },
    /// Trop de changements (tampon débordé) ou erreur : recharger le dossier.
    Rescan,
}

/// Lot de changements pour une surveillance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEvent {
    pub key: u64,
    pub changes: Vec<Change>,
}

enum Cmd {
    Watch { key: u64, path: PathBuf },
    Unwatch { key: u64 },
}

pub struct WatchService {
    cmds: Sender<Cmd>,
    port: Arc<Port>,
}

impl WatchService {
    /// Démarre le thread de surveillance ; `deliver` est appelé depuis ce thread.
    pub fn new(deliver: impl Fn(DirEvent) + Send + 'static) -> std::io::Result<Self> {
        let port = Arc::new(Port::new().map_err(std::io::Error::other)?);
        let (tx, rx) = unbounded();
        let p = port.clone();
        std::thread::Builder::new()
            .name("fx-watch".into())
            .stack_size(256 * 1024)
            .spawn(move || run(&p, &rx, &deliver))?;
        Ok(Self { cmds: tx, port })
    }

    /// Surveille `path` sous la clé `key` (> 0), en remplaçant une éventuelle surveillance
    /// existante pour cette clé.
    pub fn watch(&self, key: u64, path: &Path) {
        let _ = self.cmds.send(Cmd::Watch {
            key,
            path: path.to_path_buf(),
        });
        self.port.wake(WAKE);
    }

    pub fn unwatch(&self, key: u64) {
        let _ = self.cmds.send(Cmd::Unwatch { key });
        self.port.wake(WAKE);
    }
}

struct Entry {
    watch: DirWatch,
    path: PathBuf,
    pending: Vec<(Action, Vec<u16>)>,
    since: Option<Instant>,
    rescan: bool,
}

fn run(port: &Port, cmds: &Receiver<Cmd>, deliver: &dyn Fn(DirEvent)) {
    let mut watches: HashMap<u64, Entry> = HashMap::new();
    loop {
        let timeout = watches
            .values()
            .filter_map(|e| e.since)
            .map(|t| FLUSH_AFTER.saturating_sub(t.elapsed()).as_millis() as u32)
            .min()
            .unwrap_or(u32::MAX);
        match port.wait(timeout) {
            Wait::Woken { key: WAKE } => loop {
                match cmds.try_recv() {
                    Ok(Cmd::Watch { key, path }) => {
                        watches.remove(&key);
                        match DirWatch::open(&path, port, key as usize) {
                            Ok(watch) => {
                                watches.insert(
                                    key,
                                    Entry {
                                        watch,
                                        path,
                                        pending: Vec::new(),
                                        since: None,
                                        rescan: false,
                                    },
                                );
                            }
                            Err(e) => {
                                tracing::debug!(path = %path.display(), error = %e, "surveillance impossible")
                            }
                        }
                    }
                    Ok(Cmd::Unwatch { key }) => {
                        watches.remove(&key);
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    // Service détruit : fin du thread.
                    Err(crossbeam_channel::TryRecvError::Disconnected) => return,
                }
            },
            Wait::Ready { key, bytes } => {
                // Paquets d'E/S annulées (surveillance retirée) : clé inconnue, ignorée.
                if let Some(e) = watches.get_mut(&(key as u64)) {
                    if bytes == 0 {
                        e.rescan = true;
                    } else {
                        e.pending.extend(e.watch.events(bytes));
                    }
                    e.since.get_or_insert_with(Instant::now);
                    if e.watch.arm().is_err() {
                        e.rescan = true;
                    }
                }
            }
            Wait::Woken { .. } | Wait::Timeout => {}
        }
        for (&key, e) in &mut watches {
            if e.since.is_some_and(|t| t.elapsed() >= FLUSH_AFTER) {
                e.since = None;
                let changes = coalesce(
                    &e.path,
                    std::mem::take(&mut e.pending),
                    std::mem::take(&mut e.rescan),
                );
                if !changes.is_empty() {
                    deliver(DirEvent { key, changes });
                }
            }
        }
    }
}

/// État actuel de `paths`, regroupés par dossier parent.
///
/// Sert à la fin d'une opération lancée par folder : la liste suit le résultat même sans
/// notification du système de fichiers (certains lecteurs virtuels, comme Google Drive, n'en
/// envoient pas). Appel bloquant (un `stat` par chemin) : hors du thread UI.
pub fn restat(paths: &[PathBuf]) -> Vec<(PathBuf, Vec<Change>)> {
    use std::os::windows::ffi::OsStrExt;
    type Names = Vec<(Action, Vec<u16>)>;
    let mut groups: Vec<(PathBuf, Names)> = Vec::new();
    for p in paths {
        let (Some(dir), Some(name)) = (p.parent(), p.file_name()) else {
            continue;
        };
        let i = match groups.iter().position(|(d, _)| d == dir) {
            Some(i) => i,
            None => {
                groups.push((dir.to_path_buf(), Vec::new()));
                groups.len() - 1
            }
        };
        groups[i]
            .1
            .push((Action::Modified, name.encode_wide().collect()));
    }
    groups
        .into_iter()
        .map(|(dir, events)| {
            let changes = coalesce(&dir, events, false);
            (dir, changes)
        })
        .collect()
}

/// Réduit une rafale de notifications à un changement final par nom.
fn coalesce(dir: &Path, events: Vec<(Action, Vec<u16>)>, rescan: bool) -> Vec<Change> {
    if rescan {
        return vec![Change::Rescan];
    }
    // Dernier état connu par nom (insensible à la casse ASCII), dans l'ordre d'apparition.
    let mut order: Vec<Vec<u16>> = Vec::new();
    let mut last: HashMap<Vec<u16>, (Vec<u16>, bool)> = HashMap::new();
    for (action, name) in events {
        let key: Vec<u16> = name
            .iter()
            .map(|&c| if (65..=90).contains(&c) { c + 32 } else { c })
            .collect();
        let exists = !matches!(action, Action::Removed | Action::RenamedFrom);
        if !last.contains_key(&key) {
            order.push(key.clone());
        }
        last.insert(key, (name, exists));
    }
    order
        .into_iter()
        .filter_map(|k| last.remove(&k))
        .map(|(name, exists)| {
            if !exists {
                return Change::Remove { name };
            }
            let path = dir.join(std::ffi::OsString::from(String::from_utf16_lossy(&name)));
            match fx_win::sys::stat(&path) {
                Some(stat) => Change::Upsert { name, stat },
                // Disparu entre la notification et la lecture des attributs.
                None => Change::Remove { name },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivers_coalesced_changes() {
        let dir = std::env::temp_dir().join(format!("fx-watchsvc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let (tx, rx) = unbounded();
        let svc = WatchService::new(move |e| {
            let _ = tx.send(e);
        })
        .expect("service");
        svc.watch(3, &dir);
        std::thread::sleep(Duration::from_millis(100));
        std::fs::write(dir.join("new.txt"), b"abc").expect("write");
        std::fs::write(dir.join("gone.txt"), b"x").expect("write");
        std::fs::remove_file(dir.join("gone.txt")).expect("rm");

        let mut changes = Vec::new();
        while let Ok(e) = rx.recv_timeout(Duration::from_secs(2)) {
            assert_eq!(e.key, 3);
            changes.extend(e.changes);
            if changes.iter().any(|c| matches!(c, Change::Upsert { name, .. } if name == &"new.txt".encode_utf16().collect::<Vec<_>>())) {
                break;
            }
        }
        let new = changes.iter().rev().find_map(|c| match c {
            Change::Upsert { name, stat } if String::from_utf16_lossy(name) == "new.txt" => {
                Some(*stat)
            }
            _ => None,
        });
        assert_eq!(new.map(|s| s.size), Some(3), "{changes:?}");
        assert!(!changes.iter().any(|c| matches!(c, Change::Upsert { name, .. } if String::from_utf16_lossy(name) == "gone.txt")));
        svc.unwatch(3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restat_reports_current_state_by_parent() {
        let dir = std::env::temp_dir().join(format!("fx-restat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).expect("mkdir");
        std::fs::write(dir.join("new.txt"), b"abcd").expect("write");
        let got = restat(&[
            dir.join("old.txt"),
            dir.join("new.txt"),
            dir.join("sub"),
            dir.join("sub").join("gone"),
        ]);
        let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0].0, dir);
        let c = &got[0].1;
        assert_eq!(c[0], Change::Remove { name: w("old.txt") });
        assert!(
            matches!(&c[1], Change::Upsert { name, stat } if *name == w("new.txt") && stat.size == 4)
        );
        assert!(
            matches!(&c[2], Change::Upsert { name, stat } if *name == w("sub") && stat.attributes & 0x10 != 0)
        );
        assert_eq!(got[1].1, vec![Change::Remove { name: w("gone") }]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
