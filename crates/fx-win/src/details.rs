//! Propriétés détaillées d'un élément (onglet « Détails ») : la liste complète que
//! l'Explorateur affiche pour son type (`System.PropList.FullDetails`), groupée par section
//! (Description, Origine, Image, Audio, Fichier…), valeurs mises en forme par Windows.

use std::path::Path;

use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Storage::EnhancedStorage::PKEY_PropList_FullDetails;
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Variant::VT_EMPTY;
use windows::Win32::UI::Shell::PropertiesSystem::{
    IPropertyDescription, IPropertyDescriptionList, PDFF_DEFAULT, PSFormatForDisplayAlloc,
};
use windows::Win32::UI::Shell::{IShellItem2, SHCreateItemFromParsingName};
use windows::core::PCWSTR;

use crate::error::{Result, ResultExt};
use crate::util::{take_co_string, wide};

/// Une ligne de l'onglet Détails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    /// Titre de section (« Description », « Image »…).
    Group(String),
    Value {
        name: String,
        value: String,
    },
}

/// Détails de `path`, dans l'ordre de l'Explorateur. Sections vides retirées.
///
/// Peut solliciter des gestionnaires de propriétés tiers : depuis un thread STA de travail.
pub fn read(path: &Path) -> Result<Vec<Detail>> {
    let w = wide(path);
    // SAFETY: chaîne terminée par NUL valide pendant l'appel.
    let item: IShellItem2 = unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None) }
        .op("SHCreateItemFromParsingName")?;
    // SAFETY: clé documentée ; l'élément renvoie la liste propre à son type.
    let list: IPropertyDescriptionList =
        unsafe { item.GetPropertyDescriptionList(&PKEY_PropList_FullDetails) }
            .op("GetPropertyDescriptionList")?;
    // SAFETY: interface valide.
    let count = unsafe { list.GetCount() }.op("GetCount")?;
    let mut out: Vec<Detail> = Vec::new();
    for i in 0..count {
        // SAFETY: index dans les bornes.
        let Ok(desc) = (unsafe { list.GetAt::<IPropertyDescription>(i) }) else {
            continue;
        };
        let mut key = PROPERTYKEY::default();
        // SAFETY: `key` locale.
        if unsafe { desc.GetPropertyKey(&mut key) }.is_err() {
            continue;
        }
        // SAFETY: chaînes allouées par COM, libérées par `take_co_string`.
        let canonical = unsafe { desc.GetCanonicalName().ok().and_then(|p| take_co_string(p)) }
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        // SAFETY: idem, nom affiché alloué par COM.
        let name = unsafe { desc.GetDisplayName().ok().and_then(|p| take_co_string(p)) }
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if canonical.starts_with("System.PropGroup.") {
            if !name.is_empty() {
                out.push(Detail::Group(name));
            }
            continue;
        }
        if name.is_empty() {
            continue;
        }
        // SAFETY: clé valide ; la valeur est libérée par PropVariantClear.
        let Ok(mut value) = (unsafe { item.GetProperty(&key) }) else {
            continue;
        };
        // SAFETY: lecture du type d'une PROPVARIANT initialisée par GetProperty.
        let empty = unsafe { value.Anonymous.Anonymous.vt } == VT_EMPTY;
        let text = if empty {
            None
        } else {
            // SAFETY: clé et valeur valides ; chaîne allouée par COM.
            unsafe { PSFormatForDisplayAlloc(&key, &value, PDFF_DEFAULT) }
                .ok()
                .and_then(|p| unsafe { take_co_string(p) })
                .map(|s| s.to_string_lossy().into_owned())
        };
        // SAFETY: valeur obtenue de GetProperty, libérée une seule fois.
        let _ = unsafe { PropVariantClear(&raw mut value) };
        if let Some(value) = text.filter(|t| !t.trim().is_empty()) {
            out.push(Detail::Value { name, value });
        }
    }
    // Sections sans aucune valeur : retirées.
    let mut cleaned: Vec<Detail> = Vec::with_capacity(out.len());
    for d in out {
        if matches!(d, Detail::Group(_)) && matches!(cleaned.last(), Some(Detail::Group(_))) {
            cleaned.pop();
        }
        cleaned.push(d);
    }
    if matches!(cleaned.last(), Some(Detail::Group(_))) {
        cleaned.pop();
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notepad_has_file_details() {
        let _com = crate::com::init_sta();
        let windir = std::env::var_os("SystemRoot").expect("SystemRoot");
        let d = read(&Path::new(&windir).join("notepad.exe")).expect("détails");
        assert!(d.iter().any(|x| matches!(x, Detail::Value { .. })), "{d:?}");
    }
}
