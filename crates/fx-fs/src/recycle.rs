//! Corbeille : lecture directe de `X:\$Recycle.Bin\<SID>\` sur chaque lecteur. Chaque élément
//! supprimé y laisse deux entrées : `$R…` (l'élément lui-même) et `$I…` (chemin d'origine,
//! taille, date de suppression). Aucun appel au shell : rapide, sans thread STA.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use fx_win::sys::DriveKind;

/// Élément de la corbeille.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recycled {
    /// L'élément lui-même (`$R…`).
    pub item: PathBuf,
    /// Ses métadonnées (`$I…`), à supprimer quand l'élément quitte la corbeille.
    pub info: PathBuf,
    /// Chemin complet d'origine (où le restaurer).
    pub original: PathBuf,
    /// Taille d'origine (contenu compris pour un dossier).
    pub size: u64,
    /// Date de suppression (FILETIME UTC brut).
    pub deleted: i64,
    pub attributes: u32,
}

/// Éléments de la corbeille de tous les lecteurs fixes et amovibles accessibles à
/// l'utilisateur (les dossiers des autres comptes sont illisibles et ignorés).
pub fn list() -> Vec<Recycled> {
    let mut out = Vec::new();
    for drive in fx_win::sys::drives() {
        if matches!(drive.kind, DriveKind::Fixed | DriveKind::Removable) {
            list_bin(&drive.root.join("$Recycle.Bin"), &mut out);
        }
    }
    out
}

fn list_bin(bin: &Path, out: &mut Vec<Recycled>) {
    let Ok(accounts) = std::fs::read_dir(bin) else {
        return;
    };
    for account in accounts.flatten() {
        let dir = account.path();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(suffix) = name.to_str().and_then(|n| n.strip_prefix("$I")) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            let Some(info) = parse_info(&bytes) else {
                continue;
            };
            let item = dir.join(format!("$R{suffix}"));
            // `$I` orphelin (élément déjà purgé) : ignoré.
            let Some(st) = fx_win::sys::stat(&item) else {
                continue;
            };
            out.push(Recycled {
                item,
                info: entry.path(),
                original: info.original,
                size: info.size,
                deleted: info.deleted,
                attributes: st.attributes,
            });
        }
    }
}

/// Contenu d'un fichier `$I…`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoFile {
    pub size: u64,
    pub deleted: i64,
    pub original: PathBuf,
}

/// Décode un fichier `$I…` : version (1 : Vista à 8.1, chemin de 260 caractères ;
/// 2 : Windows 10+, chemin de longueur variable), taille, date, chemin d'origine.
pub fn parse_info(b: &[u8]) -> Option<InfoFile> {
    let u64_at = |o: usize| {
        b.get(o..o + 8)
            .map(|s| u64::from_le_bytes(s.try_into().unwrap_or_default()))
    };
    let version = u64_at(0)?;
    let size = u64_at(8)?;
    let deleted = u64_at(16)? as i64;
    let raw = match version {
        1 => b.get(24..24 + 520)?,
        2 => {
            let chars = u32::from_le_bytes(b.get(24..28)?.try_into().ok()?) as usize;
            b.get(28..28 + chars * 2)?
        }
        _ => return None,
    };
    let units: Vec<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|&c| c != 0)
        .collect();
    if units.is_empty() {
        return None;
    }
    Some(InfoFile {
        size,
        deleted,
        original: PathBuf::from(OsString::from_wide(&units)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v2(size: u64, time: i64, path: &str) -> Vec<u8> {
        let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
        let mut b = Vec::new();
        b.extend(2u64.to_le_bytes());
        b.extend(size.to_le_bytes());
        b.extend(time.to_le_bytes());
        b.extend((wide.len() as u32).to_le_bytes());
        for c in wide {
            b.extend(c.to_le_bytes());
        }
        b
    }

    #[test]
    fn parses_version_2() {
        let i = parse_info(&v2(
            2_156_008,
            133_000_000_000_000_000,
            r"C:\Users\moi\Images\photo.png",
        ))
        .expect("v2");
        assert_eq!(i.size, 2_156_008);
        assert_eq!(i.deleted, 133_000_000_000_000_000);
        assert_eq!(i.original, PathBuf::from(r"C:\Users\moi\Images\photo.png"));
    }

    #[test]
    fn parses_version_1_and_rejects_garbage() {
        let mut b = Vec::new();
        b.extend(1u64.to_le_bytes());
        b.extend(10u64.to_le_bytes());
        b.extend(5i64.to_le_bytes());
        let mut path = [0u16; 260];
        for (d, s) in path.iter_mut().zip(r"D:\a.txt".encode_utf16()) {
            *d = s;
        }
        for c in path {
            b.extend(c.to_le_bytes());
        }
        assert_eq!(
            parse_info(&b).map(|i| i.original),
            Some(PathBuf::from(r"D:\a.txt"))
        );
        assert_eq!(parse_info(&[1, 2, 3]), None);
        assert_eq!(parse_info(&v2(1, 1, "")), None);
    }

    #[test]
    fn listing_does_not_fail() {
        // Contenu réel de la corbeille : seulement cohérent, pas prévisible.
        for r in list() {
            assert!(
                r.item
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("$R"))
            );
            assert!(r.original.is_absolute(), "{r:?}");
        }
    }
}
