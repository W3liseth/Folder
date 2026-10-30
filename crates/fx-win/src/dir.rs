use std::ops::ControlFlow;
use std::path::Path;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, FILETIME, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FULL_DIR_INFO, FILE_LIST_DIRECTORY,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FIND_FIRST_EX_LARGE_FETCH,
    FileFullDirectoryInfo, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindNextFileW, GetFileInformationByHandleEx, OPEN_EXISTING, WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

use crate::error::{Error, Result, ResultExt};

/// Une entrée de dossier telle que renvoyée par le système, empruntée au tampon interne.
///
/// Aucune allocation : `name` pointe dans la `WIN32_FIND_DATAW` réutilisée.
#[derive(Debug)]
pub struct RawEntry<'a> {
    /// Nom UTF-16 (non terminé), jamais `.` ni `..`.
    pub name: &'a [u16],
    /// `FILE_ATTRIBUTE_*`.
    pub attributes: u32,
    pub size: u64,
    /// FILETIME UTC (intervalles de 100 ns depuis 1601).
    pub created: i64,
    pub modified: i64,
}

/// Énumère les entrées directes de `dir`.
///
/// Même stratégie que Files (observée par rétro-ingénierie) : `FindExInfoBasic` (pas de nom
/// 8.3) + `FIND_FIRST_EX_LARGE_FETCH` (tampon noyau plus grand, moins d'appels système).
/// Le rappel peut interrompre l'énumération avec `ControlFlow::Break`.
pub fn for_each_entry(
    dir: &Path,
    mut f: impl FnMut(&RawEntry<'_>) -> ControlFlow<()>,
) -> Result<()> {
    let mut pattern = crate::path::to_wide_extended(dir);
    pattern.pop(); // NUL
    if pattern.last() != Some(&u16::from(b'\\')) {
        pattern.push(u16::from(b'\\'));
    }
    pattern.extend([u16::from(b'*'), 0]);

    let mut data = WIN32_FIND_DATAW::default();
    // SAFETY: `pattern` est terminé par NUL et vit pendant l'appel ; `data` est une
    // WIN32_FIND_DATAW valide, ce qu'attend FindExInfoBasic.
    let first = unsafe {
        FindFirstFileExW(
            PCWSTR(pattern.as_ptr()),
            FindExInfoBasic,
            (&raw mut data).cast(),
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let handle = match first {
        Ok(h) => FindHandle(h),
        // Racine de lecteur vide : aucune entrée, pas même `.`.
        Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => return Ok(()),
        Err(e) => return Err(Error::new("FindFirstFileExW", e)),
    };

    loop {
        let len = data
            .cFileName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(data.cFileName.len());
        let name = &data.cFileName[..len];
        if !is_dot_or_dotdot(name) {
            let entry = RawEntry {
                name,
                attributes: data.dwFileAttributes,
                size: (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
                created: filetime(data.ftCreationTime),
                modified: filetime(data.ftLastWriteTime),
            };
            if f(&entry).is_break() {
                return Ok(());
            }
        }
        // SAFETY: handle valide (fermé seulement au Drop) ; `data` est une WIN32_FIND_DATAW.
        match unsafe { FindNextFileW(handle.0, &raw mut data) } {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_NO_MORE_FILES.to_hresult() => return Ok(()),
            Err(e) => return Err(e).op("FindNextFileW"),
        }
    }
}

/// Une entrée avec la place qu'elle occupe sur le disque (analyse de l'espace).
#[derive(Debug)]
pub struct SizedEntry<'a> {
    /// Nom UTF-16 (non terminé), jamais `.` ni `..`.
    pub name: &'a [u16],
    pub attributes: u32,
    /// Taille du contenu (`EndOfFile`).
    pub size: u64,
    /// Place réservée sur le volume (`AllocationSize`) : 0 pour un fichier cloud non
    /// téléchargé ou un petit fichier logé dans la MFT, taille compressée pour un fichier
    /// compressé. C'est ce qu'on libère en le supprimant.
    pub allocated: u64,
    pub created: i64,
    pub modified: i64,
}

/// Tampon réutilisé par [`for_each_sized_entry`] : 64 Ko alignés sur 8 octets, un appel
/// système renvoie des centaines d'entrées.
pub struct DirBuffer(Vec<u64>);

impl Default for DirBuffer {
    fn default() -> Self {
        Self(vec![0; 64 * 1024 / 8])
    }
}

/// Énumère les entrées directes de `dir` avec leur taille sur disque :
/// `GetFileInformationByHandleEx(FileFullDirectoryInfo)` (que `FindFirstFileExW` ne donne
/// pas), dans le tampon fourni.
pub fn for_each_sized_entry(
    dir: &Path,
    buf: &mut DirBuffer,
    mut f: impl FnMut(&SizedEntry<'_>) -> ControlFlow<()>,
) -> Result<()> {
    let w = crate::path::to_wide_extended(dir);
    // SAFETY: chemin terminé par NUL ; le handle est fermé par `DirHandle`.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .op("CreateFileW")?;
    let handle = DirHandle(handle);
    let bytes = buf.0.len() * 8;
    loop {
        // SAFETY: handle de dossier ouvert ci-dessus ; le tampon fait `bytes` octets et
        // reste emprunté pendant l'appel.
        let read = unsafe {
            GetFileInformationByHandleEx(
                handle.0,
                FileFullDirectoryInfo,
                buf.0.as_mut_ptr().cast(),
                bytes as u32,
            )
        };
        match read {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_NO_MORE_FILES.to_hresult() => return Ok(()),
            Err(e) => return Err(Error::new("GetFileInformationByHandleEx", e)),
        }
        let base = buf.0.as_ptr().cast::<u8>();
        let mut offset = 0usize;
        loop {
            // Les enregistrements sont alignés sur 8 octets dans un tampon aligné sur 8 : la
            // lecture de l'en-tête est alignée, le nom suit dans le même enregistrement.
            if offset + size_of::<FILE_FULL_DIR_INFO>() > bytes {
                break;
            }
            // SAFETY: `offset` désigne un enregistrement écrit par le noyau dans le tampon
            // (premier, ou atteint par `NextEntryOffset`), aligné et complet.
            let info = unsafe { &*base.add(offset).cast::<FILE_FULL_DIR_INFO>() };
            let name_len = (info.FileNameLength / 2) as usize;
            let name_at = offset + std::mem::offset_of!(FILE_FULL_DIR_INFO, FileName);
            if name_at + name_len * 2 > bytes {
                break;
            }
            // SAFETY: le nom (`FileNameLength` octets) suit l'en-tête dans l'enregistrement,
            // à l'intérieur du tampon (vérifié ci-dessus) ; UTF-16 aligné sur 2.
            let name =
                unsafe { std::slice::from_raw_parts(base.add(name_at).cast::<u16>(), name_len) };
            if !is_dot_or_dotdot(name) {
                let entry = SizedEntry {
                    name,
                    attributes: info.FileAttributes,
                    size: info.EndOfFile.max(0) as u64,
                    allocated: info.AllocationSize.max(0) as u64,
                    created: info.CreationTime,
                    modified: info.LastWriteTime,
                };
                if f(&entry).is_break() {
                    return Ok(());
                }
            }
            if info.NextEntryOffset == 0 {
                break;
            }
            offset += info.NextEntryOffset as usize;
        }
    }
}

struct DirHandle(HANDLE);

impl Drop for DirHandle {
    fn drop(&mut self) {
        // SAFETY: handle issu d'un CreateFileW réussi, fermé une seule fois.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn is_dot_or_dotdot(name: &[u16]) -> bool {
    matches!(name, [0x2E] | [0x2E, 0x2E])
}

fn filetime(ft: FILETIME) -> i64 {
    ((u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)) as i64
}

struct FindHandle(HANDLE);

impl Drop for FindHandle {
    fn drop(&mut self) {
        // SAFETY: handle issu d'un FindFirstFileExW réussi, fermé une seule fois.
        let _ = unsafe { FindClose(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windir() -> std::path::PathBuf {
        std::env::var_os("SystemRoot").expect("SystemRoot").into()
    }

    #[test]
    fn lists_windows_dir_without_dots() {
        let mut n = 0;
        let mut saw_system32 = false;
        for_each_entry(&windir(), |e| {
            assert!(!is_dot_or_dotdot(e.name));
            saw_system32 |= String::from_utf16_lossy(e.name).eq_ignore_ascii_case("System32");
            n += 1;
            ControlFlow::Continue(())
        })
        .expect("enumerate");
        assert!(n > 10);
        assert!(saw_system32);
    }

    #[test]
    fn sized_entries_match_find_first_file() {
        let mut plain = Vec::new();
        for_each_entry(&windir(), |e| {
            plain.push((String::from_utf16_lossy(e.name), e.size));
            ControlFlow::Continue(())
        })
        .expect("enumerate");
        let mut sized = Vec::new();
        let mut buf = DirBuffer::default();
        for_each_sized_entry(&windir(), &mut buf, |e| {
            assert!(!is_dot_or_dotdot(e.name));
            sized.push((String::from_utf16_lossy(e.name), e.size));
            ControlFlow::Continue(())
        })
        .expect("enumerate sized");
        plain.sort();
        sized.sort();
        assert_eq!(plain, sized);
        let err = for_each_sized_entry(Path::new(r"C:\fx-does-not-exist-42"), &mut buf, |_| {
            ControlFlow::Continue(())
        })
        .expect_err("should fail");
        assert!(err.is_not_found(), "{err}");
    }

    #[test]
    fn missing_dir_is_not_found() {
        let err = for_each_entry(Path::new(r"C:\fx-does-not-exist-42"), |_| {
            ControlFlow::Continue(())
        })
        .expect_err("should fail");
        assert!(err.is_not_found(), "{err}");
    }

    #[test]
    fn break_stops_early() {
        let mut n = 0;
        for_each_entry(&windir(), |_| {
            n += 1;
            ControlFlow::Break(())
        })
        .expect("enumerate");
        assert_eq!(n, 1);
    }
}
