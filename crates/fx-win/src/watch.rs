//! Surveillance de dossiers : `ReadDirectoryChangesW` asynchrone sur un port de complétion
//! unique (un seul thread pour toutes les surveillances, cf. ARCHITECTURE.md §7.2).

use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    ReadDirectoryChangesW,
};
use windows::Win32::System::IO::{
    CancelIoEx, CreateIoCompletionPort, GetOverlappedResult, GetQueuedCompletionStatus, OVERLAPPED,
    PostQueuedCompletionStatus,
};
use windows::Win32::System::Threading::CreateEventW;
use windows::core::PCWSTR;

use crate::error::{Result, ResultExt};

/// Taille du tampon de notifications (64 Kio, maximum conseillé pour les partages réseau).
const BUF_WORDS: usize = 16 * 1024;

/// Port de complétion partagé par toutes les surveillances.
pub struct Port(HANDLE);

// SAFETY: un handle de port de complétion est utilisable depuis n'importe quel thread.
unsafe impl Send for Port {}
// SAFETY: idem, les appels sur le port sont thread-safe.
unsafe impl Sync for Port {}

/// Résultat d'une attente sur le port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// Notifications disponibles pour la surveillance `key` (`bytes == 0` : débordement).
    Ready {
        key: usize,
        bytes: u32,
    },
    /// Réveil explicite (`Port::wake`) ou opération annulée.
    Woken {
        key: usize,
    },
    Timeout,
}

impl Port {
    pub fn new() -> Result<Self> {
        // SAFETY: création d'un port sans fichier associé.
        let h = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, 1) }
            .op("CreateIoCompletionPort")?;
        Ok(Self(h))
    }

    pub fn wait(&self, timeout_ms: u32) -> Wait {
        let (mut bytes, mut key, mut ov) = (0u32, 0usize, std::ptr::null_mut());
        // SAFETY: pointeurs vers des variables locales ; on ne déréférence jamais `ov`.
        let r =
            unsafe { GetQueuedCompletionStatus(self.0, &mut bytes, &mut key, &mut ov, timeout_ms) };
        match r {
            Ok(()) if !ov.is_null() => Wait::Ready { key, bytes },
            Ok(()) => Wait::Woken { key },
            Err(e)
                if ov.is_null()
                    && e.code() == windows::core::HRESULT::from_win32(WAIT_TIMEOUT.0) =>
            {
                Wait::Timeout
            }
            Err(_) => Wait::Woken { key },
        }
    }

    /// Réveille le thread en attente (commande à traiter).
    pub fn wake(&self, key: usize) {
        // SAFETY: paquet sans OVERLAPPED.
        let _ = unsafe { PostQueuedCompletionStatus(self.0, 0, key, None) };
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        // SAFETY: handle possédé.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Nature d'une notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Added,
    Removed,
    Modified,
    RenamedFrom,
    RenamedTo,
}

/// Surveillance d'un dossier (non récursive).
///
/// Le tampon et l'`OVERLAPPED` sont en tas et vivent jusqu'à ce que l'E/S en cours soit
/// annulée **et terminée** (`Drop` attend la fin) : le noyau n'écrit jamais en mémoire libérée.
pub struct DirWatch {
    handle: HANDLE,
    /// Événement de l'`OVERLAPPED` : permet au `Drop` d'attendre la fin de l'E/S annulée
    /// (le handle de dossier n'est pas signalé de façon fiable une fois associé au port).
    event: HANDLE,
    ov: Box<OVERLAPPED>,
    buf: Box<[u32]>,
}

// SAFETY: la surveillance n'est manipulée que par un thread à la fois (déplacée vers le thread
// de surveillance) ; le noyau n'accède au tampon qu'à travers l'E/S armée.
unsafe impl Send for DirWatch {}

impl DirWatch {
    pub fn open(path: &Path, port: &Port, key: usize) -> Result<Self> {
        let w = crate::path::to_wide_extended(path);
        // SAFETY: chemin terminé par NUL ; handle fermé au Drop.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(w.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .op("CreateFileW")?;
        // SAFETY: événement à réinitialisation manuelle, fermé au Drop.
        let event = match unsafe { CreateEventW(None, true, false, PCWSTR::null()) } {
            Ok(e) => e,
            Err(e) => {
                // SAFETY: handle ouvert ci-dessus, pas encore possédé par une structure.
                let _ = unsafe { CloseHandle(handle) };
                return Err(crate::Error::new("CreateEventW", e));
            }
        };
        let mut watch = Self {
            handle,
            event,
            ov: Box::default(),
            buf: vec![0u32; BUF_WORDS].into_boxed_slice(),
        };
        // SAFETY: associe le handle au port existant avec la clé de l'appelant.
        unsafe { CreateIoCompletionPort(handle, Some(port.0), key, 0) }
            .op("CreateIoCompletionPort")?;
        watch.arm()?;
        Ok(watch)
    }

    /// (Ré)arme la lecture asynchrone des changements.
    pub fn arm(&mut self) -> Result<()> {
        let filter: FILE_NOTIFY_CHANGE = FILE_NOTIFY_CHANGE_FILE_NAME
            | FILE_NOTIFY_CHANGE_DIR_NAME
            | FILE_NOTIFY_CHANGE_ATTRIBUTES
            | FILE_NOTIFY_CHANGE_SIZE
            | FILE_NOTIFY_CHANGE_LAST_WRITE
            | FILE_NOTIFY_CHANGE_CREATION;
        *self.ov = OVERLAPPED {
            hEvent: self.event,
            ..Default::default()
        };
        // SAFETY: tampon et OVERLAPPED en tas, possédés par `self`, vivants jusqu'au Drop qui
        // attend la fin de l'E/S ; tampon aligné sur 4 octets.
        unsafe {
            ReadDirectoryChangesW(
                self.handle,
                self.buf.as_mut_ptr().cast(),
                (self.buf.len() * 4) as u32,
                false,
                filter,
                None,
                Some(&raw mut *self.ov),
                None,
            )
        }
        .op("ReadDirectoryChangesW")
    }

    /// Décode les `bytes` premiers octets du tampon (après une complétion `Ready`).
    pub fn events(&self, bytes: u32) -> Vec<(Action, Vec<u16>)> {
        let bytes = (bytes as usize).min(self.buf.len() * 4);
        let raw: Vec<u8> = self
            .buf
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .take(bytes)
            .collect();
        let u32_at = |o: usize| {
            raw.get(o..o + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let mut out = Vec::new();
        let mut off = 0usize;
        while let (Some(next), Some(action), Some(len)) =
            (u32_at(off), u32_at(off + 4), u32_at(off + 8))
        {
            let start = off + 12;
            let Some(name) = raw.get(start..start + len as usize) else {
                break;
            };
            let name: Vec<u16> = name
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let action = match action {
                1 => Some(Action::Added),
                2 => Some(Action::Removed),
                3 => Some(Action::Modified),
                4 => Some(Action::RenamedFrom),
                5 => Some(Action::RenamedTo),
                _ => None,
            };
            if let Some(a) = action {
                out.push((a, name));
            }
            if next == 0 {
                break;
            }
            off += next as usize;
        }
        out
    }
}

impl Drop for DirWatch {
    fn drop(&mut self) {
        let mut n = 0;
        // SAFETY: annule l'E/S en cours puis attend sa fin avant de libérer tampon et
        // OVERLAPPED ; enfin ferme le handle possédé.
        unsafe {
            let _ = CancelIoEx(self.handle, Some(&raw const *self.ov));
            let _ = GetOverlappedResult(self.handle, &raw const *self.ov, &mut n, true);
            let _ = CloseHandle(self.handle);
            let _ = CloseHandle(self.event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_create_and_rename() {
        let dir = std::env::temp_dir().join(format!("fx-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let port = Port::new().expect("port");
        let mut w = DirWatch::open(&dir, &port, 7).expect("watch");
        std::fs::write(dir.join("a.txt"), b"x").expect("write");
        std::fs::rename(dir.join("a.txt"), dir.join("b.txt")).expect("rename");

        let mut seen = Vec::new();
        for _ in 0..20 {
            match port.wait(500) {
                Wait::Ready { key: 7, bytes } => {
                    seen.extend(
                        w.events(bytes)
                            .into_iter()
                            .map(|(a, n)| (a, String::from_utf16_lossy(&n))),
                    );
                    w.arm().expect("rearm");
                    if seen.iter().any(|(a, _)| *a == Action::RenamedTo) {
                        break;
                    }
                }
                Wait::Timeout => break,
                _ => {}
            }
        }
        assert!(seen.contains(&(Action::Added, "a.txt".into())), "{seen:?}");
        assert!(
            seen.contains(&(Action::RenamedTo, "b.txt".into())),
            "{seen:?}"
        );
        drop(w);
        port.wake(1);
        // Le paquet de l'E/S annulée (clé 7) peut précéder le réveil : on l'ignore.
        let mut woke = false;
        for _ in 0..3 {
            if port.wait(200) == (Wait::Woken { key: 1 }) {
                woke = true;
                break;
            }
        }
        assert!(woke);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
