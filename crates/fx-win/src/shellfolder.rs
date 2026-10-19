//! Énumération d'un dossier de l'espace de noms du shell (Réseau, un ordinateur du réseau…),
//! pour les emplacements qui ne sont pas des dossiers du système de fichiers.

use windows::Win32::System::SystemServices::{SFGAO_FILESYSTEM, SFGAO_FOLDER};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, IEnumShellItems, IShellItem, SHCreateItemFromParsingName,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
};
use windows::core::PCWSTR;

use crate::error::{Result, ResultExt};
use crate::util::{take_co_string, wide_str};

/// Élément d'un dossier du shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellChild {
    /// Nom affiché.
    pub name: String,
    /// Nom d'analyse absolu (`\\SERVEUR`, `\\SERVEUR\partage`, `::{CLSID}`…).
    pub parsing: String,
    pub folder: bool,
    /// Dossier ou fichier du système de fichiers (navigable comme un dossier ordinaire).
    pub filesystem: bool,
}

/// Enfants du dossier shell `parsing` (`shell:NetworkPlacesFolder`, `\\SERVEUR`…).
///
/// Peut être long (découverte réseau) : depuis un thread STA dédié, jamais le thread UI.
pub fn children(parsing: &str) -> Result<Vec<ShellChild>> {
    let mut out = Vec::new();
    for_each_child(parsing, |c| out.push(c))?;
    Ok(out)
}

/// Comme [`children`], mais remet chaque élément dès qu'il est trouvé : la découverte réseau
/// peut durer des dizaines de secondes, les premiers appareils arrivent bien avant la fin.
pub fn for_each_child(parsing: &str, mut on_item: impl FnMut(ShellChild)) -> Result<()> {
    let w = wide_str(parsing);
    // SAFETY: chaîne terminée par NUL valide pendant l'appel.
    let parent: IShellItem = unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None) }
        .op("SHCreateItemFromParsingName")?;
    // SAFETY: GUID de gestionnaire documenté ; pas de contexte de liaison.
    let items: IEnumShellItems =
        unsafe { parent.BindToHandler(None, &BHID_EnumItems) }.op("BindToHandler")?;
    loop {
        let mut slot = [None];
        let mut fetched = 0u32;
        // SAFETY: tableau d'une case et compteur locaux.
        let hr = unsafe { items.Next(&mut slot, Some(&mut fetched)) };
        let Some(item) = slot[0].take().filter(|_| hr.is_ok() && fetched == 1) else {
            break;
        };
        // SAFETY: élément valide ; chaînes allouées par COM, libérées par `take_co_string`.
        let (name, parsing) = unsafe {
            (
                item.GetDisplayName(SIGDN_NORMALDISPLAY)
                    .ok()
                    .and_then(|p| take_co_string(p)),
                item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)
                    .ok()
                    .and_then(|p| take_co_string(p)),
            )
        };
        let (Some(name), Some(parsing)) = (name, parsing) else {
            continue;
        };
        // SAFETY: élément valide.
        let attrs =
            unsafe { item.GetAttributes(SFGAO_FOLDER | SFGAO_FILESYSTEM) }.unwrap_or_default();
        on_item(ShellChild {
            name: name.to_string_lossy().into_owned(),
            parsing: parsing.to_string_lossy().into_owned(),
            folder: attrs.contains(SFGAO_FOLDER),
            filesystem: attrs.contains(SFGAO_FILESYSTEM),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_this_pc() {
        let _com = crate::com::init_sta();
        // « Ce PC » : contient au moins le lecteur système.
        let items = children("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}").expect("Ce PC");
        assert!(
            items.iter().any(|c| c.parsing.eq_ignore_ascii_case("C:\\")),
            "{items:?}"
        );
    }

    #[test]
    #[ignore = "dépend du réseau local"]
    fn network_timing() {
        let _com = crate::com::init_sta();
        let t = std::time::Instant::now();
        for_each_child("::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}", |c| {
            eprintln!("réseau : {} après {:?}", c.name, t.elapsed());
        })
        .expect("réseau");
        eprintln!("réseau : fin après {:?}", t.elapsed());
    }
}
