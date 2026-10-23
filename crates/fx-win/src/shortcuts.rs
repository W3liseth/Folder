//! Raccourcis système de folder, qui marchent même folder fermé :
//!
//! - **Windows + E** : le processus résident `folder.exe --win-e` (cf. [`crate::win_e`]),
//!   lancé à l'ouverture de session par la clé `Run` de l'utilisateur. Chaque appui ouvre une
//!   nouvelle fenêtre. Retirer la valeur et arrêter le processus rend la main à l'Explorateur.
//!   (Remplacer la commande `opennewwindow` de l'Explorateur dans le registre n'a plus d'effet
//!   depuis Windows 11 26H2.)
//! - **Raccourci d'ouverture** : la « touche de raccourci » d'un raccourci du menu Démarrer,
//!   que l'Explorateur surveille lui-même (Ctrl + Alt + lettre, etc.).

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegGetValueW, RegSetValueExW,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Programs, IShellLinkW, KF_FLAG_DEFAULT, SHCNE_UPDATEITEM, SHCNF_PATHW, SHChangeNotify,
    SHGetKnownFolderPath, ShellLink,
};
use windows::core::{Interface, PCWSTR};

use crate::error::{Error, Result, ResultExt};
use crate::util::{wide, wide_str};

/// Programmes lancés à l'ouverture de session de l'utilisateur.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Valeur de `RUN_KEY` qui lance le processus résident de Windows + E.
const RUN_VALUE: &str = "folder (Windows + E)";
/// Argument du processus résident (cf. `app/folder`).
pub const WIN_E_ARG: &str = "--win-e";
/// Description du raccourci que folder crée lui-même dans le menu Démarrer (pour ne
/// supprimer que le sien).
const OWN_SHORTCUT: &str = "folder (raccourci clavier)";

/// Windows + E ouvre `exe` (Some : démarrage à l'ouverture de session, processus résident
/// lancé s'il ne tourne pas) ou de nouveau l'Explorateur (None : démarrage retiré, processus
/// arrêté).
pub fn set_win_e(exe: Option<&Path>) -> Result<()> {
    match exe {
        Some(exe) => {
            set_run_value(RUN_KEY, RUN_VALUE, Some(exe))?;
            if !crate::win_e::is_running() {
                let mut cmd = std::process::Command::new(exe);
                cmd.arg(WIN_E_ARG);
                // Processus de longue durée : il ne doit verrouiller aucun dossier de l'utilisateur.
                if let Some(dir) = exe.parent() {
                    cmd.current_dir(dir);
                }
                cmd.spawn()
                    .map_err(|e| Error::new("CreateProcessW", e.into()))?;
            }
            Ok(())
        }
        None => {
            let removed = set_run_value(RUN_KEY, RUN_VALUE, None);
            crate::win_e::stop();
            removed
        }
    }
}

/// Le processus résident doit-il tourner (démarrage à l'ouverture de session inscrit) ?
pub fn win_e_enabled() -> bool {
    read_string(RUN_KEY, RUN_VALUE).is_some()
}

fn set_run_value(key: &str, name: &str, exe: Option<&Path>) -> Result<()> {
    match exe {
        Some(exe) => {
            let k = create_key(key)?;
            let set = set_string(k, name, &format!("\"{}\" {WIN_E_ARG}", exe.display()));
            // SAFETY: clé ouverte par create_key.
            let _ = unsafe { RegCloseKey(k) };
            set
        }
        None => {
            let (k, n) = (wide_str(key), wide_str(name));
            // SAFETY: chaînes terminées par NUL ; valeur de la partie utilisateur.
            let r = unsafe {
                RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr()))
            };
            if r.is_err() && r != ERROR_FILE_NOT_FOUND {
                return Err(Error::new("RegDeleteKeyValueW", r.into()));
            }
            Ok(())
        }
    }
}

fn read_string(key: &str, name: &str) -> Option<String> {
    let (k, n) = (wide_str(key), wide_str(name));
    let mut buf = [0u16; 1024];
    let mut bytes = (buf.len() * 2) as u32;
    // SAFETY: chaînes terminées par NUL, tampon local et sa taille en octets.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(k.as_ptr()),
            PCWSTR(n.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    }
    .ok()
    .ok()?;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn create_key(path: &str) -> Result<HKEY> {
    let w = wide_str(path);
    let mut key = HKEY::default();
    // SAFETY: chaîne terminée par NUL ; `key` reçoit une clé que l'appelant ferme.
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(w.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            None,
            &mut key,
            None,
        )
    }
    .ok()
    .op("RegCreateKeyExW")?;
    Ok(key)
}

fn set_string(key: HKEY, name: &str, value: &str) -> Result<()> {
    let name = wide_str(name);
    let data: Vec<u8> = value
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    // SAFETY: clé ouverte en écriture, nom terminé par NUL, données REG_SZ terminées par NUL.
    unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&data)) }
        .ok()
        .op("RegSetValueExW")
}

/// Touche de raccourci d'un raccourci Windows : code de touche virtuelle (lettre, chiffre,
/// F1–F12) et au moins deux modificateurs (contrainte de l'Explorateur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchHotkey {
    pub vk: u8,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl LaunchHotkey {
    /// Valeur de `IShellLink::SetHotkey` : touche dans l'octet bas, HOTKEYF_* dans le haut.
    fn word(self) -> u16 {
        let mods = u16::from(self.shift) | u16::from(self.ctrl) << 1 | u16::from(self.alt) << 2;
        mods << 8 | u16::from(self.vk)
    }
}

/// Raccourci « folder » du menu Démarrer de l'utilisateur (celui de l'installeur, ou celui
/// que folder crée).
pub fn start_menu_shortcut() -> Result<PathBuf> {
    // SAFETY: dossier connu ; la chaîne renvoyée (allouée par COM) est libérée ci-dessous.
    let p = unsafe { SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None) }
        .op("SHGetKnownFolderPath")?;
    // SAFETY: chaîne COM terminée par NUL renvoyée par SHGetKnownFolderPath.
    let dir = unsafe { crate::util::take_co_string(p) }
        .ok_or_else(|| Error::new("SHGetKnownFolderPath", windows::core::Error::from_thread()))?;
    Ok(dir.join("folder.lnk"))
}

/// Pose (Some) ou retire (None) la touche de raccourci qui ouvre `exe`. Sans raccourci
/// existant, folder en crée un dans le menu Démarrer ; il le supprime quand la touche est
/// retirée. Depuis un thread COM.
pub fn set_launch_hotkey(exe: &Path, hotkey: Option<LaunchHotkey>) -> Result<()> {
    let lnk = start_menu_shortcut()?;
    set_launch_hotkey_at(&lnk, exe, hotkey)
}

fn set_launch_hotkey_at(lnk: &Path, exe: &Path, hotkey: Option<LaunchHotkey>) -> Result<()> {
    // SAFETY: classe CLSID_ShellLink, objet en processus.
    let link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.op("ShellLink")?;
    let file: IPersistFile = link.cast().op("IPersistFile")?;
    let path = wide(lnk);
    let exists = lnk.exists();
    if exists {
        // SAFETY: chemin terminé par NUL valide pendant l'appel.
        unsafe { file.Load(PCWSTR(path.as_ptr()), STGM_READ) }.op("IPersistFile::Load")?;
    }
    let own = exists && description(&link).as_deref() == Some(OWN_SHORTCUT);
    match hotkey {
        None if !exists => return Ok(()),
        // Le nôtre, sans raison d'être : supprimé.
        None if own => {
            std::fs::remove_file(lnk).map_err(|e| Error::new("remove_file", e.into()))?;
            notify(&path);
            return Ok(());
        }
        _ => {}
    }
    if !exists {
        let exe_w = wide(exe);
        let desc = wide_str(OWN_SHORTCUT);
        // SAFETY: chaînes terminées par NUL valides pendant les appels.
        unsafe {
            link.SetPath(PCWSTR(exe_w.as_ptr())).op("SetPath")?;
            link.SetDescription(PCWSTR(desc.as_ptr()))
                .op("SetDescription")?;
        }
    }
    let word = hotkey.map_or(0, LaunchHotkey::word);
    // SAFETY: objet valide ; 0 retire la touche.
    unsafe { link.SetHotkey(word) }.op("SetHotkey")?;
    // SAFETY: chemin terminé par NUL ; enregistre (crée au besoin) le fichier.
    unsafe { file.Save(PCWSTR(path.as_ptr()), true) }.op("IPersistFile::Save")?;
    notify(&path);
    Ok(())
}

fn description(link: &IShellLinkW) -> Option<String> {
    let mut buf = [0u16; 256];
    // SAFETY: tampon local et sa taille.
    unsafe { link.GetDescription(&mut buf) }.ok()?;
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buf[..n]))
}

/// Prévient l'Explorateur qu'un raccourci du menu Démarrer a changé (touches surveillées).
fn notify(path: &[u16]) {
    // SAFETY: chemin terminé par NUL valide pendant l'appel.
    unsafe {
        SHChangeNotify(
            SHCNE_UPDATEITEM,
            SHCNF_PATHW,
            Some(path.as_ptr().cast()),
            None,
        );
    }
}

/// Désinstallation (`folder.exe --restore-explorer`) : Windows + E rendu à l'Explorateur et
/// touche de raccourci retirée.
pub fn restore(exe: &Path) {
    let _ = set_win_e(None);
    let _ = set_launch_hotkey(exe, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Écrit dans une clé de test de la partie utilisateur, jamais dans la vraie clé `Run`.
    #[test]
    fn run_value_roundtrip() {
        use windows::Win32::System::Registry::RegDeleteTreeW;
        let key = r"Software\folder-tests\Run";
        let exe = Path::new(r"C:\Program Files\folder\folder.exe");
        set_run_value(key, RUN_VALUE, Some(exe)).expect("écriture");
        assert_eq!(
            read_string(key, RUN_VALUE).as_deref(),
            Some(r#""C:\Program Files\folder\folder.exe" --win-e"#)
        );
        set_run_value(key, RUN_VALUE, None).expect("retrait");
        assert_eq!(read_string(key, RUN_VALUE), None);
        // Déjà retirée : pas une erreur.
        set_run_value(key, RUN_VALUE, None).expect("retrait répété");
        let w = wide_str(r"Software\folder-tests");
        // SAFETY: sous-arbre de test.
        let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(w.as_ptr())) };
    }

    #[test]
    fn hotkey_word() {
        let hk = LaunchHotkey {
            vk: b'F',
            ctrl: true,
            alt: true,
            shift: false,
        };
        assert_eq!(hk.word(), 0x0646);
    }

    #[test]
    fn shortcut_in_temp_dir() {
        let _com = crate::com::init_sta();
        let dir = std::env::temp_dir().join("folder-shortcut-test");
        let _ = std::fs::create_dir_all(&dir);
        let lnk = dir.join("folder.lnk");
        let _ = std::fs::remove_file(&lnk);
        let exe = std::env::current_exe().expect("exe");
        let hk = LaunchHotkey {
            vk: b'F',
            ctrl: true,
            alt: true,
            shift: false,
        };
        set_launch_hotkey_at(&lnk, &exe, Some(hk)).expect("création");
        assert!(lnk.exists());
        // Notre raccourci, sans touche : supprimé.
        set_launch_hotkey_at(&lnk, &exe, None).expect("retrait");
        assert!(!lnk.exists());
        let _ = std::fs::remove_dir(&dir);
    }
}
