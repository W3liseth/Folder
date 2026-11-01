//! folder, gestionnaire de fichiers par défaut : les dossiers et lecteurs ouverts par une
//! autre application (« Afficher dans le dossier » d'un navigateur, raccourci, double-clic
//! sur le Bureau) s'ouvrent dans folder plutôt que dans l'Explorateur.
//!
//! Le shell lance l'action par défaut de la classe `Directory` (ou `Drive`) : on pose la
//! nôtre dans la partie utilisateur du registre (`HKCU\Software\Classes`), sans droits
//! d'administrateur, qui prime sur celle de la machine. La classe `Folder` n'est pas
//! touchée : elle couvre aussi les dossiers virtuels (Panneau de configuration, Ce PC,
//! Corbeille) que folder ne sait pas tous afficher. Retirer nos clés rend aussitôt la main à
//! l'Explorateur ; une action par défaut posée avant nous par une autre application est
//! retenue et remise en place.

use std::path::Path;

use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, RegCloseKey, RegDeleteKeyValueW, RegDeleteTreeW,
};
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use windows::core::PCWSTR;

use crate::error::{Error, Result};
use crate::shortcuts::{create_key, read_string, set_string};
use crate::util::wide_str;

/// Classes du registre de l'utilisateur.
const CLASSES: &str = r"Software\Classes";
/// Classes dont on remplace l'action par défaut.
const TYPES: [&str; 2] = ["Directory", "Drive"];
/// Notre action (verbe) sous `<classe>\shell`.
const VERB: &str = "openinfolder";
/// Valeur de notre verbe qui retient l'action par défaut d'avant (autre application).
const PREVIOUS: &str = "folderPrevious";
/// Argument de `folder.exe` lancé par le shell : le dossier s'ouvre dans la fenêtre folder
/// active, sinon dans une nouvelle (cf. `app/folder`).
pub const OPEN_ARG: &str = "--open-folder";

/// folder devient (Some) ou cesse d'être (None) le gestionnaire de fichiers par défaut.
pub fn set_default(exe: Option<&Path>) -> Result<()> {
    let done = match exe {
        Some(exe) => TYPES.iter().try_for_each(|t| register(CLASSES, t, exe)),
        None => TYPES.iter().try_for_each(|t| unregister(CLASSES, t)),
    };
    // Le shell relit les associations (sans quoi l'Explorateur garde l'ancienne un moment).
    // SAFETY: notification sans paramètre.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    done
}

/// folder est-il l'action par défaut des dossiers et des lecteurs ?
pub fn is_default() -> bool {
    TYPES
        .iter()
        .all(|t| read_string(&shell_key(CLASSES, t), "").as_deref() == Some(VERB))
}

fn shell_key(root: &str, class: &str) -> String {
    format!(r"{root}\{class}\shell")
}

fn register(root: &str, class: &str, exe: &Path) -> Result<()> {
    let shell = shell_key(root, class);
    let verb = format!(r"{shell}\{VERB}");
    let before = read_string(&shell, "").filter(|v| !v.is_empty() && v != VERB);
    let exe_s = exe.display();
    with_key(&verb, |k| {
        set_string(k, "", "Ouvrir dans folder")?;
        set_string(k, "Icon", &format!("\"{exe_s}\",0"))?;
        match &before {
            Some(prev) => set_string(k, PREVIOUS, prev),
            None => Ok(()),
        }
    })?;
    with_key(&format!(r"{verb}\command"), |k| {
        set_string(k, "", &format!("\"{exe_s}\" {OPEN_ARG} \"%1\""))
    })?;
    // En dernier : l'action par défaut ne désigne jamais une commande incomplète.
    with_key(&shell, |k| set_string(k, "", VERB))
}

fn unregister(root: &str, class: &str) -> Result<()> {
    let shell = shell_key(root, class);
    let verb = format!(r"{shell}\{VERB}");
    let previous = read_string(&verb, PREVIOUS);
    // D'abord l'action par défaut : le shell ne lance plus jamais une commande retirée.
    if read_string(&shell, "").as_deref() == Some(VERB) {
        match previous {
            Some(prev) => with_key(&shell, |k| set_string(k, "", &prev))?,
            None => delete_value(&shell, "")?,
        }
    }
    delete_tree(&verb)
}

fn with_key(
    path: &str,
    f: impl FnOnce(windows::Win32::System::Registry::HKEY) -> Result<()>,
) -> Result<()> {
    let k = create_key(path)?;
    let r = f(k);
    // SAFETY: clé ouverte par create_key.
    let _ = unsafe { RegCloseKey(k) };
    r
}

/// Retire une valeur ; absente : pas une erreur.
fn delete_value(key: &str, name: &str) -> Result<()> {
    let (k, n) = (wide_str(key), wide_str(name));
    // SAFETY: chaînes terminées par NUL ; valeur de la partie utilisateur.
    let r =
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr())) };
    if r.is_err() && r != ERROR_FILE_NOT_FOUND {
        return Err(Error::new("RegDeleteKeyValueW", r.into()));
    }
    Ok(())
}

/// Retire une clé et son contenu ; absente : pas une erreur.
fn delete_tree(key: &str) -> Result<()> {
    let k = wide_str(key);
    // SAFETY: chaîne terminée par NUL ; sous-arbre de la partie utilisateur.
    let r = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr())) };
    if r.is_err() && r != ERROR_FILE_NOT_FOUND {
        return Err(Error::new("RegDeleteTreeW", r.into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Écrit dans une clé de test de la partie utilisateur, jamais dans les vraies classes.
    #[test]
    fn register_and_restore_previous_default() {
        let root = r"Software\folder-tests\default-fm\Classes";
        let exe = Path::new(r"C:\Program Files\folder\folder.exe");
        let shell = shell_key(root, "Directory");
        // Action par défaut d'une autre application, posée avant nous.
        with_key(&shell, |k| set_string(k, "", "autre")).expect("préparation");

        register(root, "Directory", exe).expect("inscription");
        assert_eq!(read_string(&shell, "").as_deref(), Some(VERB));
        assert_eq!(
            read_string(&format!(r"{shell}\{VERB}\command"), "").as_deref(),
            Some(r#""C:\Program Files\folder\folder.exe" --open-folder "%1""#)
        );
        // Réinscription (chemin de l'exécutable changé) : l'action d'avant reste retenue.
        register(root, "Directory", exe).expect("réinscription");

        unregister(root, "Directory").expect("retrait");
        assert_eq!(read_string(&shell, "").as_deref(), Some("autre"));
        assert_eq!(read_string(&format!(r"{shell}\{VERB}"), ""), None);

        // Sans action d'avant : la valeur par défaut disparaît.
        delete_value(&shell, "").expect("nettoyage");
        register(root, "Drive", exe).expect("inscription lecteurs");
        unregister(root, "Drive").expect("retrait lecteurs");
        assert_eq!(read_string(&shell_key(root, "Drive"), ""), None);
        // Déjà retiré : pas une erreur.
        unregister(root, "Drive").expect("retrait répété");

        delete_tree(r"Software\folder-tests\default-fm").expect("nettoyage");
    }
}
