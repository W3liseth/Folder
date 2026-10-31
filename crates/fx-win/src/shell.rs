//! Actions shell simples : ouvrir, ouvrir avec, propriétés, terminal, administrateur,
//! corbeille.

use std::ffi::OsStr;
use std::path::Path;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    ASSOCF_NONE, ASSOCSTR_FRIENDLYAPPNAME, AssocQueryStringW, OAIF_ALLOW_REGISTRATION, OAIF_EXEC,
    OPENASINFO, SEE_MASK_FLAG_NO_UI, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW,
    SHERB_NOCONFIRMATION, SHEmptyRecycleBinW, SHOP_FILEPATH, SHObjectProperties, SHOpenWithDialog,
    ShellExecuteExW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, w};

use crate::error::{Error, Result, ResultExt};
use crate::util::{Pidl, wide, wide_str};

fn owner(hwnd: Option<isize>) -> Option<HWND> {
    hwnd.map(|h| HWND(h as *mut _))
}

fn execute(
    hwnd: Option<isize>,
    verb: PCWSTR,
    file: &[u16],
    params: Option<&[u16]>,
    dir: Option<&[u16]>,
) -> Result<()> {
    // SAFETY: chaînes terminées par NUL valides pendant l'appel.
    let h = unsafe {
        ShellExecuteW(
            owner(hwnd),
            verb,
            PCWSTR(file.as_ptr()),
            params.map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
            dir.map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
            SW_SHOWNORMAL,
        )
    };
    // Convention historique : une valeur <= 32 est un code d'erreur.
    if h.0 as isize <= 32 {
        return Err(Error::new(
            "ShellExecuteW",
            windows::core::Error::from_thread(),
        ));
    }
    Ok(())
}

/// Dossier de travail du programme lancé : celui du fichier, comme l'Explorateur. Sans lui,
/// ShellExecute prend le dossier courant de folder (`C:\Program Files\folder`) et un script
/// (.bat, .ps1, .py…) ne trouve pas les fichiers qui l'entourent.
fn working_dir(path: &Path) -> Option<&Path> {
    path.parent().filter(|d| !d.as_os_str().is_empty())
}

/// Ouvre un fichier avec son application par défaut (verbe « open »).
///
/// Bloquant tant que le shell résout l'association : depuis un thread STA de travail.
pub fn open(path: &Path) -> Result<()> {
    // Pas de forme \\?\ ici : de nombreuses applications ne la comprennent pas.
    let dir = working_dir(path).map(wide);
    execute(None, w!("open"), &wide(path), None, dir.as_deref())
}

/// Lance `program` avec `args` dans le dossier `dir`, comme la barre d'adresse de
/// l'Explorateur : chemin complet, ou nom que le shell résout lui-même (App Paths, URL,
/// protocole). Sans boîte d'erreur du shell : l'appelant affiche la sienne.
///
/// Bloquant tant que le shell résout la cible : depuis un thread STA de travail.
pub fn run(program: &OsStr, args: &str, dir: Option<&Path>) -> Result<()> {
    let file = wide(Path::new(program));
    let params = (!args.is_empty()).then(|| wide_str(args));
    let dir = dir.map(wide);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("open"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: params
            .as_ref()
            .map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
        lpDirectory: dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: structure initialisée avec sa taille ; chaînes terminées par NUL vivantes
    // jusqu'à la fin de l'appel.
    unsafe { ShellExecuteExW(&mut info) }.op("ShellExecuteExW")
}

/// Ouvre un élément de l'espace de noms du shell désigné par son nom d'analyse
/// (appareil du réseau, `::{CLSID}`…) : action par défaut, comme un double-clic dans
/// l'Explorateur. Bloquant : depuis un thread STA de travail.
pub fn open_parsing(parsing: &Path) -> Result<()> {
    let pidl = Pidl::parse(parsing)?;
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST,
        lpIDList: pidl.as_ptr() as *mut _,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: structure initialisée avec sa taille ; le PIDL vit jusqu'à la fin de l'appel.
    unsafe { ShellExecuteExW(&mut info) }.op("ShellExecuteExW")
}

/// Nom de l'application qui ouvre les fichiers d'extension `ext` (« Bloc-notes »…), si une
/// association existe.
pub fn default_app_name(ext: &str) -> Option<String> {
    let ext = wide_str(&format!(".{}", ext.trim_start_matches('.')));
    let mut buf = [0u16; 260];
    let mut len = buf.len() as u32;
    // SAFETY: extension terminée par NUL, tampon local et sa taille.
    let hr = unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_FRIENDLYAPPNAME,
            PCWSTR(ext.as_ptr()),
            PCWSTR::null(),
            Some(windows::core::PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
    };
    if hr.is_err() {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    let name = String::from_utf16_lossy(&buf[..n]);
    (!name.is_empty()).then_some(name)
}

/// Fenêtre Propriétés de Windows ouverte sur un onglet précis (« Sécurité », « Partage »…).
pub fn properties_page(hwnd: Option<isize>, path: &Path, page: &str) -> Result<()> {
    let w = wide(path);
    let p = wide_str(page);
    // SAFETY: chaînes terminées par NUL valides pendant l'appel.
    let ok = unsafe {
        SHObjectProperties(
            owner(hwnd),
            SHOP_FILEPATH,
            PCWSTR(w.as_ptr()),
            PCWSTR(p.as_ptr()),
        )
    };
    if ok.as_bool() {
        Ok(())
    } else {
        Err(Error::new(
            "SHObjectProperties",
            windows::core::Error::from_thread(),
        ))
    }
}

/// Exécute en tant qu'administrateur (invite UAC).
pub fn run_as_admin(hwnd: Option<isize>, path: &Path) -> Result<()> {
    let dir = working_dir(path).map(wide);
    execute(hwnd, w!("runas"), &wide(path), None, dir.as_deref())
}

/// Ouvre un terminal dans `dir` : Windows Terminal s'il est installé, sinon l'invite de commandes.
pub fn open_terminal(dir: &Path) -> Result<()> {
    let d = wide(dir);
    let params = wide_str(&format!("-d \"{}\"", dir.display()));
    execute(
        None,
        w!("open"),
        &wide_str("wt.exe"),
        Some(&params),
        Some(&d),
    )
    .or_else(|_| execute(None, w!("open"), &wide_str("cmd.exe"), None, Some(&d)))
}

/// Boîte « Ouvrir avec » native.
pub fn open_with(hwnd: Option<isize>, path: &Path) -> Result<()> {
    let w = wide(path);
    let info = OPENASINFO {
        pcszFile: PCWSTR(w.as_ptr()),
        oaifInFlags: OAIF_EXEC | OAIF_ALLOW_REGISTRATION,
        ..Default::default()
    };
    // SAFETY: structure valide, chaîne vivante pendant l'appel (boîte modale).
    unsafe { SHOpenWithDialog(owner(hwnd), &info) }.op("SHOpenWithDialog")
}

/// Fenêtre Propriétés native (non modale).
pub fn properties(hwnd: Option<isize>, path: &Path) -> Result<()> {
    let w = wide(path);
    // SAFETY: chemin terminé par NUL ; pas de page imposée.
    let ok = unsafe {
        SHObjectProperties(
            owner(hwnd),
            SHOP_FILEPATH,
            PCWSTR(w.as_ptr()),
            PCWSTR::null(),
        )
    };
    if ok.as_bool() {
        Ok(())
    } else {
        Err(Error::new(
            "SHObjectProperties",
            windows::core::Error::from_thread(),
        ))
    }
}

/// Vide la corbeille de tous les lecteurs (confirmation faite par fx).
pub fn empty_recycle_bin(hwnd: Option<isize>) -> Result<()> {
    // SAFETY: racine nulle = tous les lecteurs.
    unsafe { SHEmptyRecycleBinW(owner(hwnd), PCWSTR::null(), SHERB_NOCONFIRMATION) }
        .op("SHEmptyRecycleBinW")
}

#[cfg(test)]
mod tests {
    use super::working_dir;
    use std::path::Path;

    #[test]
    fn working_dir_is_the_file_folder() {
        assert_eq!(
            working_dir(Path::new(r"D:\projets\outils\build.bat")),
            Some(Path::new(r"D:\projets\outils"))
        );
        assert_eq!(
            working_dir(Path::new(r"C:\script.ps1")),
            Some(Path::new(r"C:\"))
        );
        // Ni racine ni nom seul : on laisse ShellExecute décider.
        assert_eq!(working_dir(Path::new(r"C:\")), None);
        assert_eq!(working_dir(Path::new("script.bat")), None);
    }
}
