//! Presse-papiers de fichiers, interopérable avec l'Explorateur : `CF_HDROP` +
//! `Preferred DropEffect` (copier ou couper).

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::{CFSTR_PREFERREDDROPEFFECT, DROPFILES, DragQueryFileW, HDROP};

use crate::error::{Error, Result, ResultExt};

const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_MOVE: u32 = 2;

/// Place des fichiers dans le presse-papiers. `cut` : l'application qui colle les déplacera.
pub fn set_files(paths: &[PathBuf], cut: bool) -> Result<()> {
    // Liste UTF-16 : chemins séparés par NUL, terminée par un double NUL.
    let mut list: Vec<u16> = Vec::new();
    for p in paths {
        list.extend(p.as_os_str().encode_wide());
        list.push(0);
    }
    list.push(0);

    let header = size_of::<DROPFILES>();
    let drop = global_with(header + list.len() * 2, |dst| {
        let df = DROPFILES {
            pFiles: header as u32,
            fWide: true.into(),
            ..Default::default()
        };
        // SAFETY: `dst` couvre `header + list.len()*2` octets, alloués ci-dessus.
        unsafe {
            std::ptr::write_unaligned(dst.cast::<DROPFILES>(), df);
            std::ptr::copy_nonoverlapping(
                list.as_ptr().cast::<u8>(),
                dst.add(header),
                list.len() * 2,
            );
        }
    })?;
    let effect = if cut {
        DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    };
    let fx = global_with(4, |dst| {
        // SAFETY: 4 octets alloués.
        unsafe { std::ptr::write_unaligned(dst.cast::<u32>(), effect) };
    })?;

    let _open = Open::acquire()?;
    // SAFETY: presse-papiers ouvert par ce thread.
    unsafe { EmptyClipboard() }.op("EmptyClipboard")?;
    for (format, mem) in [
        (u32::from(CF_HDROP.0), drop),
        (preferred_effect_format(), fx),
    ] {
        // SAFETY: en cas de succès, le système devient propriétaire de `mem`.
        if let Err(e) = unsafe { SetClipboardData(format, Some(HANDLE(mem.0))) } {
            // SAFETY: échec : la mémoire nous appartient toujours.
            let _ = unsafe { GlobalFree(Some(mem)) };
            return Err(Error::new("SetClipboardData", e));
        }
    }
    Ok(())
}

/// Place du texte (Unicode) dans le presse-papiers.
pub fn set_text(text: &str) -> Result<()> {
    let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let mem = global_with(units.len() * 2, |dst| {
        // SAFETY: bloc de `units.len()*2` octets.
        unsafe { std::ptr::copy_nonoverlapping(units.as_ptr().cast::<u8>(), dst, units.len() * 2) };
    })?;
    let _open = Open::acquire()?;
    // SAFETY: presse-papiers ouvert par ce thread.
    unsafe { EmptyClipboard() }.op("EmptyClipboard")?;
    const CF_UNICODETEXT: u32 = 13;
    // SAFETY: en cas de succès, le système devient propriétaire de `mem`.
    if let Err(e) = unsafe { SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0))) } {
        // SAFETY: échec : la mémoire nous appartient toujours.
        let _ = unsafe { GlobalFree(Some(mem)) };
        return Err(Error::new("SetClipboardData", e));
    }
    Ok(())
}

/// Fichiers du presse-papiers et indicateur « couper ».
pub fn get_files() -> Option<(Vec<PathBuf>, bool)> {
    if !has_files() {
        return None;
    }
    let _open = Open::acquire().ok()?;
    // SAFETY: presse-papiers ouvert ; le handle reste au système.
    let h = unsafe { GetClipboardData(u32::from(CF_HDROP.0)) }.ok()?;
    let hdrop = HDROP(h.0);
    // SAFETY: HDROP valide tant que le presse-papiers est ouvert.
    let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
    let mut files = Vec::with_capacity(count as usize);
    for i in 0..count {
        // SAFETY: idem ; la première requête donne la longueur sans le NUL.
        let len = unsafe { DragQueryFileW(hdrop, i, None) } as usize;
        let mut buf = vec![0u16; len + 1];
        // SAFETY: tampon de len+1 unités.
        let n = unsafe { DragQueryFileW(hdrop, i, Some(&mut buf)) } as usize;
        files.push(PathBuf::from(OsString::from_wide(&buf[..n])));
    }
    let cut = read_u32(preferred_effect_format()).is_some_and(|e| e & DROPEFFECT_MOVE != 0);
    Some((files, cut))
}

/// Vrai si le presse-papiers contient des fichiers (pour activer « Coller »).
pub fn has_files() -> bool {
    // SAFETY: pas de précondition.
    unsafe { IsClipboardFormatAvailable(u32::from(CF_HDROP.0)) }.is_ok()
}

fn preferred_effect_format() -> u32 {
    // SAFETY: littéral terminé par NUL.
    unsafe { RegisterClipboardFormatW(CFSTR_PREFERREDDROPEFFECT) }
}

fn read_u32(format: u32) -> Option<u32> {
    // SAFETY: appelé presse-papiers ouvert (cf. `get_files`).
    let h = unsafe { GetClipboardData(format) }.ok()?;
    let mem = HGLOBAL(h.0);
    // SAFETY: bloc global d'au moins 4 octets (format DWORD), déverrouillé aussitôt.
    unsafe {
        let p = GlobalLock(mem);
        if p.is_null() {
            return None;
        }
        let v = std::ptr::read_unaligned(p.cast::<u32>());
        let _ = GlobalUnlock(mem);
        Some(v)
    }
}

fn global_with(size: usize, fill: impl FnOnce(*mut u8)) -> Result<HGLOBAL> {
    // SAFETY: allocation déplaçable de `size` octets.
    let mem = unsafe { GlobalAlloc(GMEM_MOVEABLE, size) }.op("GlobalAlloc")?;
    // SAFETY: bloc fraîchement alloué, verrouillé le temps de l'écriture.
    unsafe {
        let p = GlobalLock(mem);
        if p.is_null() {
            let _ = GlobalFree(Some(mem));
            return Err(Error::new(
                "GlobalLock",
                windows::core::Error::from_thread(),
            ));
        }
        std::ptr::write_bytes(p.cast::<u8>(), 0, size);
        fill(p.cast());
        let _ = GlobalUnlock(mem);
    }
    Ok(mem)
}

/// Presse-papiers ouvert, refermé au `Drop`. Réessaie brièvement si une autre application
/// le détient.
struct Open;

impl Open {
    fn acquire() -> Result<Self> {
        let mut last = None;
        for _ in 0..10 {
            // SAFETY: pas de fenêtre propriétaire : associé à la tâche courante.
            match unsafe { OpenClipboard(None) } {
                Ok(()) => return Ok(Self),
                Err(e) => last = Some(e),
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(Error::new(
            "OpenClipboard",
            last.unwrap_or_else(windows::core::Error::from_thread),
        ))
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: ouvert par `acquire` sur ce thread.
        let _ = unsafe { CloseClipboard() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_cut() {
        let p = std::env::temp_dir().join("fx-clip-test.txt");
        set_files(std::slice::from_ref(&p), true).expect("set");
        let (files, cut) = get_files().expect("get");
        assert_eq!(files, vec![p]);
        assert!(cut);
    }
}
