//! Petits utilitaires internes : chaînes larges et PIDL.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILFree, SHParseDisplayName};
use windows::core::{PCWSTR, PWSTR};

use crate::error::{Result, ResultExt};

/// Chemin en UTF-16 terminé par NUL, **sans** préfixe `\\?\` (les API shell ne le
/// comprennent pas toujours).
pub(crate) fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

pub(crate) fn wide_str(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Copie une chaîne allouée par COM puis la libère.
///
/// # Safety
/// `p` doit être nul ou une chaîne terminée par NUL allouée par `CoTaskMemAlloc`.
pub(crate) unsafe fn take_co_string(p: PWSTR) -> Option<PathBuf> {
    if p.is_null() {
        return None;
    }
    // SAFETY: garanti par l'appelant.
    let s = unsafe { p.as_wide() };
    let out = PathBuf::from(OsString::from_wide(s));
    // SAFETY: mémoire allouée par COM, libérée une seule fois.
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    Some(out)
}

/// PIDL absolu possédé, libéré au `Drop`.
pub(crate) struct Pidl(pub(crate) *mut ITEMIDLIST);

impl Pidl {
    pub(crate) fn parse(path: &Path) -> Result<Self> {
        let w = wide(path);
        let mut p = std::ptr::null_mut();
        // SAFETY: chaîne terminée par NUL ; `p` reçoit un PIDL alloué par le shell.
        unsafe { SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut p, 0, None) }
            .op("SHParseDisplayName")?;
        Ok(Self(p))
    }

    pub(crate) fn as_ptr(&self) -> *const ITEMIDLIST {
        self.0
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        // SAFETY: PIDL alloué par le shell, libéré une seule fois.
        unsafe { ILFree(Some(self.0)) };
    }
}
