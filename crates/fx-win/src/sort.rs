use std::cmp::Ordering;

use windows::Win32::UI::Shell::StrCmpLogicalW;
use windows::core::PCWSTR;

/// Comparaison « logique » de l'Explorateur (`StrCmpLogicalW`).
///
/// Sert de référence pour valider le tri naturel pur Rust de `fx-core` ; trop coûteuse
/// (conversion UTF-16 + appel système) pour trier des centaines de milliers d'entrées.
pub fn explorer_cmp(a: &str, b: &str) -> Ordering {
    let a: Vec<u16> = a.encode_utf16().chain([0]).collect();
    let b: Vec<u16> = b.encode_utf16().chain([0]).collect();
    // SAFETY: deux chaînes UTF-16 terminées par NUL, valides pendant l'appel.
    let r = unsafe { StrCmpLogicalW(PCWSTR(a.as_ptr()), PCWSTR(b.as_ptr())) };
    r.cmp(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_numbers() {
        assert_eq!(explorer_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(explorer_cmp("File", "file"), Ordering::Equal);
    }
}
