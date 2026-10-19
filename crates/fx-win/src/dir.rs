use std::ops::ControlFlow;
use std::path::Path;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, FILETIME, HANDLE};
use windows::Win32::Storage::FileSystem::{
    FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindNextFileW, WIN32_FIND_DATAW,
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
