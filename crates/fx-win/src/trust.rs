//! Signature Authenticode d'un fichier (`WinVerifyTrust`) : la mise à jour n'installe un
//! installeur que s'il est signé par le même éditeur que le folder en cours d'exécution.

use std::path::Path;

use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::Security::Cryptography::{CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW};
use windows::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
    WTD_STATEACTION_VERIFY, WTD_UI_NONE, WTHelperGetProvSignerFromChain,
    WTHelperProvDataFromStateData, WinVerifyTrust,
};
use windows::core::PCWSTR;

use crate::util::wide;

/// Nom de l'éditeur (sujet du certificat) si `path` porte une signature Authenticode
/// valide, `None` sinon (non signé, signature invalide ou chaîne non approuvée).
/// Sans vérification de révocation en ligne : aucun accès réseau implicite.
pub fn signer(path: &Path) -> Option<String> {
    let w = wide(path);
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(w.as_ptr()),
        hFile: HANDLE::default(),
        pgKnownSubject: std::ptr::null_mut(),
    };
    let mut data = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: `data` et `file` (et la chaîne `w`) vivent jusqu'à la fermeture ci-dessous ;
    // aucune interface (WTD_UI_NONE).
    let status = unsafe { WinVerifyTrust(HWND::default(), &mut action, (&raw mut data).cast()) };
    let name = (status == 0)
        .then(|| signer_name(data.hWVTStateData))
        .flatten();
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: libère l'état ouvert par l'appel précédent (mêmes structures).
    let _ = unsafe { WinVerifyTrust(HWND::default(), &mut action, (&raw mut data).cast()) };
    name
}

/// Sujet du certificat du premier signataire, lu dans l'état d'une vérification réussie.
fn signer_name(state: HANDLE) -> Option<String> {
    // SAFETY: `state` vient d'une vérification réussie, pas encore fermée.
    let prov = unsafe { WTHelperProvDataFromStateData(state) };
    if prov.is_null() {
        return None;
    }
    // SAFETY: données du fournisseur valides tant que l'état est ouvert.
    let sgnr = unsafe { WTHelperGetProvSignerFromChain(prov, 0, false, 0) };
    // SAFETY: pointeurs vérifiés non nuls avant chaque déréférencement ; chaîne d'au moins
    // un certificat (le signataire) quand `csCertChain` > 0.
    let cert = unsafe {
        if sgnr.is_null() || (*sgnr).csCertChain == 0 || (*sgnr).pasCertChain.is_null() {
            return None;
        }
        (*(*sgnr).pasCertChain).pCert
    };
    if cert.is_null() {
        return None;
    }
    let mut buf = [0u16; 256];
    // SAFETY: certificat valide ; `buf` accessible en écriture.
    let n =
        unsafe { CertGetNameStringW(cert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, Some(&mut buf)) }
            as usize;
    // `n` compte le NUL final ; 1 : nom vide.
    (n > 1).then(|| String::from_utf16_lossy(&buf[..n - 1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_binary_is_signed() {
        // notepad.exe est signé par catalogue, pas dans le fichier : on prend un exécutable
        // signé directement (présent sur tout Windows 10/11).
        let windir = std::env::var_os("WINDIR").map_or_else(
            || std::path::PathBuf::from(r"C:\Windows"),
            std::path::PathBuf::from,
        );
        let signed = windir.join("explorer.exe");
        if signed.exists() {
            assert!(
                signer(&signed).is_some_and(|s| s.contains("Microsoft")),
                "explorer.exe devrait être signé par Microsoft"
            );
        }
        let dir = std::env::temp_dir().join(format!("fx-trust-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let unsigned = dir.join("unsigned.exe");
        std::fs::write(&unsigned, b"MZ pas un vrai programme").expect("write");
        assert_eq!(signer(&unsigned), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
