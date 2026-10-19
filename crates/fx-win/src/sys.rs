//! Informations système : attributs d'un fichier, lecteurs, dossiers connus.

use std::path::{Path, PathBuf};

use windows::Win32::Storage::FileSystem::{
    FILE_FLAGS_AND_ATTRIBUTES, GetDiskFreeSpaceExW, GetDriveTypeW, GetFileAttributesExW,
    GetFileExInfoStandard, GetLogicalDrives, GetVolumeInformationW, SetFileAttributesW,
    WIN32_FILE_ATTRIBUTE_DATA,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW,
    RegOpenKeyExW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures,
    FOLDERID_Profile, FOLDERID_Recent, FOLDERID_Videos, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::core::{GUID, PCWSTR, PWSTR, w};

use crate::error::{Result, ResultExt};
use crate::util::{take_co_string, wide};

/// Attributs d'une entrée, pour mettre à jour un instantané après une notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub attributes: u32,
    pub size: u64,
    pub created: i64,
    pub modified: i64,
}

pub fn stat(path: &Path) -> Option<Stat> {
    let w = crate::path::to_wide_extended(path);
    let mut d = WIN32_FILE_ATTRIBUTE_DATA::default();
    // SAFETY: chemin terminé par NUL ; `d` a le type attendu pour GetFileExInfoStandard.
    unsafe {
        GetFileAttributesExW(
            PCWSTR(w.as_ptr()),
            GetFileExInfoStandard,
            (&raw mut d).cast(),
        )
    }
    .ok()?;
    let ft = |f: windows::Win32::Foundation::FILETIME| {
        ((u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)) as i64
    };
    Some(Stat {
        attributes: d.dwFileAttributes,
        size: (u64::from(d.nFileSizeHigh) << 32) | u64::from(d.nFileSizeLow),
        created: ft(d.ftCreationTime),
        modified: ft(d.ftLastWriteTime),
    })
}

/// Remplace les attributs d'un fichier ou dossier (lecture seule, caché…).
pub fn set_attributes(path: &Path, attributes: u32) -> Result<()> {
    let w = crate::path::to_wide_extended(path);
    // SAFETY: chemin terminé par NUL valide pendant l'appel.
    unsafe { SetFileAttributesW(PCWSTR(w.as_ptr()), FILE_FLAGS_AND_ATTRIBUTES(attributes)) }
        .op("SetFileAttributesW")
}

/// Distributions WSL de l'utilisateur, lues dans le registre (sans démarrer WSL, contrairement
/// à l'énumération de `\\wsl.localhost`). Vide si WSL n'est pas installé.
pub fn wsl_distributions() -> Vec<String> {
    let mut key = HKEY::default();
    // SAFETY: clé racine prédéfinie, sous-clé littérale, `key` reçoit un handle fermé plus bas.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Lxss"),
            None,
            KEY_READ,
            &mut key,
        )
    };
    if opened.is_err() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for index in 0.. {
        let mut sub = [0u16; 128];
        let mut len = sub.len() as u32;
        // SAFETY: tampon local et sa taille en caractères.
        let r = unsafe {
            RegEnumKeyExW(
                key,
                index,
                Some(PWSTR(sub.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if r.is_err() {
            break;
        }
        let mut value = [0u16; 128];
        let mut bytes = (value.len() * 2) as u32;
        // SAFETY: sous-clé terminée par NUL (écrite par RegEnumKeyExW), tampon et taille locaux.
        let r = unsafe {
            RegGetValueW(
                key,
                PCWSTR(sub.as_ptr()),
                w!("DistributionName"),
                RRF_RT_REG_SZ,
                None,
                Some(value.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        if r.is_ok() {
            let n = value.iter().position(|&c| c == 0).unwrap_or(0);
            let name = String::from_utf16_lossy(&value[..n]);
            if !name.is_empty() {
                out.push(name);
            }
        }
    }
    // SAFETY: handle ouvert ci-dessus.
    let _ = unsafe { RegCloseKey(key) };
    out.sort();
    out
}

/// Partage réseau de cet ordinateur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub name: String,
    pub path: std::path::PathBuf,
}

/// Dossiers partagés par cet ordinateur, lus dans la configuration du service Serveur
/// (`LanmanServer\Shares`, lisible sans élévation contrairement à `NetShareEnum` niveau 2).
/// Les partages administratifs (`C$`, `ADMIN$`…) n'y figurent pas.
pub fn shares() -> Vec<Share> {
    let mut key = HKEY::default();
    // SAFETY: clé racine prédéfinie, sous-clé littérale, handle fermé plus bas.
    let opened = unsafe {
        RegOpenKeyExW(
            windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
            w!(r"SYSTEM\CurrentControlSet\Services\LanmanServer\Shares"),
            None,
            KEY_READ,
            &mut key,
        )
    };
    if opened.is_err() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for index in 0.. {
        let mut name = [0u16; 256];
        let mut name_len = name.len() as u32;
        let mut data = vec![0u8; 4096];
        let mut data_len = data.len() as u32;
        // SAFETY: tampons locaux et leurs tailles.
        let r = unsafe {
            windows::Win32::System::Registry::RegEnumValueW(
                key,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                None,
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
        if r.is_err() {
            break;
        }
        // Valeur multichaîne : « Path=C:\… » parmi d'autres lignes.
        let units: Vec<u16> = data[..data_len as usize]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let text = String::from_utf16_lossy(&units);
        if let Some(path) = text.split('\0').find_map(|l| l.strip_prefix("Path=")) {
            out.push(Share {
                name: String::from_utf16_lossy(&name[..name_len as usize]),
                path: std::path::PathBuf::from(path),
            });
        }
    }
    // SAFETY: handle ouvert ci-dessus.
    let _ = unsafe { RegCloseKey(key) };
    out
}

/// Nom de cet ordinateur (chemins réseau `\\NOM\partage`).
pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    Fixed,
    Removable,
    Network,
    Optical,
    Ram,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    /// Racine, ex. `C:\`.
    pub root: PathBuf,
    pub letter: char,
    pub label: String,
    pub kind: DriveKind,
    pub total: u64,
    pub free: u64,
}

/// Lecteurs montés. Peut être lent (lecteurs réseau, optiques) : hors thread UI.
pub fn drives() -> Vec<Drive> {
    // SAFETY: pas de précondition.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| {
            let letter = char::from(b'A' + i);
            let root = PathBuf::from(format!("{letter}:\\"));
            let w = wide(&root);
            // SAFETY: racine terminée par NUL.
            let kind = match unsafe { GetDriveTypeW(PCWSTR(w.as_ptr())) } {
                2 => DriveKind::Removable,
                3 => DriveKind::Fixed,
                4 => DriveKind::Network,
                5 => DriveKind::Optical,
                6 => DriveKind::Ram,
                _ => DriveKind::Unknown,
            };
            let mut label = [0u16; 261];
            // SAFETY: tampon de 261 unités pour le nom de volume.
            let has_label = unsafe {
                GetVolumeInformationW(PCWSTR(w.as_ptr()), Some(&mut label), None, None, None, None)
            }
            .is_ok();
            let label = if has_label {
                let n = label.iter().position(|&c| c == 0).unwrap_or(0);
                String::from_utf16_lossy(&label[..n])
            } else {
                String::new()
            };
            let (mut free, mut total) = (0u64, 0u64);
            // SAFETY: pointeurs vers des u64 locaux.
            let _ = unsafe {
                GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut free), Some(&mut total), None)
            };
            Drive {
                root,
                letter,
                label,
                kind,
                total,
                free,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownFolder {
    Home,
    Desktop,
    Downloads,
    Documents,
    Pictures,
    Music,
    Videos,
    Recent,
}

impl KnownFolder {
    pub const ALL: [Self; 8] = [
        Self::Home,
        Self::Desktop,
        Self::Downloads,
        Self::Documents,
        Self::Pictures,
        Self::Music,
        Self::Videos,
        Self::Recent,
    ];

    fn id(self) -> GUID {
        match self {
            Self::Home => FOLDERID_Profile,
            Self::Desktop => FOLDERID_Desktop,
            Self::Downloads => FOLDERID_Downloads,
            Self::Documents => FOLDERID_Documents,
            Self::Pictures => FOLDERID_Pictures,
            Self::Music => FOLDERID_Music,
            Self::Videos => FOLDERID_Videos,
            Self::Recent => FOLDERID_Recent,
        }
    }
}

/// Chemin d'un dossier connu (redirections OneDrive comprises).
pub fn known_folder(f: KnownFolder) -> Option<PathBuf> {
    // SAFETY: GUID valide ; chaîne libérée par `take_co_string`.
    unsafe {
        SHGetKnownFolderPath(&f.id(), KF_FLAG_DEFAULT, None)
            .ok()
            .and_then(|p| take_co_string(p))
    }
}

/// Délai maximal entre les deux clics d'un double-clic (réglage de la souris), en secondes.
pub fn double_click_time() -> f64 {
    // SAFETY: lecture d'un réglage système, sans précondition.
    f64::from(unsafe { GetDoubleClickTime() }) / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_drive_and_known_folders() {
        let d = drives();
        let sys = d.iter().find(|d| d.letter == 'C').expect("C:");
        assert_eq!(sys.kind, DriveKind::Fixed);
        assert!(sys.total > sys.free && sys.total > 0);
        assert!(known_folder(KnownFolder::Home).is_some_and(|p| p.is_dir()));
        // WSL facultatif : la lecture du registre ne doit simplement pas échouer.
        let _ = wsl_distributions();
        let windir = std::env::var_os("SystemRoot").expect("SystemRoot");
        let s = stat(Path::new(&windir)).expect("stat");
        assert!(s.attributes & 0x10 != 0);
    }
}
