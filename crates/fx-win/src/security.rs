//! Sécurité d'un fichier ou dossier (onglet « Sécurité ») : propriétaire et entrées
//! d'autorisation, regroupées par compte comme dans l'Explorateur.

use std::path::Path;

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, LookupAccountSidW,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SID_NAME_USE,
};
use windows::core::{PCWSTR, PWSTR};

use crate::error::{Error, Result};

/// Droits usuels (colonnes de l'onglet Sécurité de l'Explorateur).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rights {
    pub full: bool,
    pub modify: bool,
    pub read_execute: bool,
    pub read: bool,
    pub write: bool,
}

impl Rights {
    /// Droits contenus dans un masque d'accès (droits génériques compris).
    pub fn from_mask(mask: u32) -> Self {
        // Droits génériques (ACE héritables) traduits en droits fichiers.
        const GENERIC_ALL: u32 = 0x1000_0000;
        const GENERIC_EXECUTE: u32 = 0x2000_0000;
        const GENERIC_WRITE: u32 = 0x4000_0000;
        const GENERIC_READ: u32 = 0x8000_0000;
        const FULL: u32 = 0x001F_01FF;
        const MODIFY: u32 = 0x0013_01BF;
        const READ_EXECUTE: u32 = 0x0012_00A9;
        const READ: u32 = 0x0012_0089;
        const WRITE: u32 = 0x0010_0116;
        let mut m = mask & 0x00FF_FFFF;
        if mask & GENERIC_ALL != 0 {
            m |= FULL;
        }
        if mask & GENERIC_READ != 0 {
            m |= READ;
        }
        if mask & GENERIC_WRITE != 0 {
            m |= WRITE;
        }
        if mask & GENERIC_EXECUTE != 0 {
            m |= READ_EXECUTE;
        }
        let has = |bits: u32| m & bits == bits;
        Self {
            full: has(FULL),
            modify: has(MODIFY),
            read_execute: has(READ_EXECUTE),
            read: has(READ),
            write: has(WRITE),
        }
    }

    pub fn any(self) -> bool {
        self.full || self.modify || self.read_execute || self.read || self.write
    }
}

/// Autorisations d'un compte (toutes ses entrées réunies).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// `DOMAINE\Nom`, ou le SID textuel si le compte est inconnu.
    pub account: String,
    pub allow: Rights,
    pub deny: Rights,
    /// Au moins une entrée héritée du dossier parent.
    pub inherited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Security {
    pub owner: Option<String>,
    pub entries: Vec<Entry>,
}

/// Lit le propriétaire et la liste d'accès de `path`.
pub fn read(path: &Path) -> Result<Security> {
    let w = crate::path::to_wide_extended(path);
    let mut owner = PSID::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: chemin terminé par NUL ; les pointeurs reçus désignent `sd`, libéré plus bas.
    let r = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(w.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            &mut sd,
        )
    };
    if r.is_err() {
        return Err(Error::new(
            "GetNamedSecurityInfoW",
            windows::core::Error::from_hresult(r.to_hresult()),
        ));
    }
    let out = Security {
        owner: (!owner.is_invalid()).then(|| account_name(owner)),
        // SAFETY: liste d'accès dans le descripteur `sd`, vivant jusqu'au LocalFree.
        entries: unsafe { entries(dacl) },
    };
    // SAFETY: descripteur alloué par GetNamedSecurityInfoW.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sd.0)));
    }
    Ok(out)
}

/// # Safety
/// `dacl` est nul ou une liste d'accès valide.
unsafe fn entries(dacl: *mut ACL) -> Vec<Entry> {
    const ALLOWED: u8 = 0;
    const DENIED: u8 = 1;
    const INHERIT_ONLY: u8 = 0x08;
    const INHERITED: u8 = 0x10;
    let mut out: Vec<Entry> = Vec::new();
    if dacl.is_null() {
        return out;
    }
    let mut info = ACL_SIZE_INFORMATION::default();
    // SAFETY: liste valide, structure de la taille annoncée.
    if unsafe {
        GetAclInformation(
            dacl,
            (&raw mut info).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    }
    .is_err()
    {
        return out;
    }
    for i in 0..info.AceCount {
        let mut ace = std::ptr::null_mut();
        // SAFETY: index dans les bornes ; `ace` pointe dans la liste.
        if unsafe { GetAce(dacl, i, &mut ace) }.is_err() || ace.is_null() {
            continue;
        }
        // SAFETY: toute entrée commence par un ACE_HEADER.
        let header = unsafe { *ace.cast::<ACE_HEADER>() };
        if !matches!(header.AceType, ALLOWED | DENIED) || header.AceFlags & INHERIT_ONLY != 0 {
            continue;
        }
        // Entrées d'autorisation et de refus : même disposition (en-tête, masque, SID).
        let allowed = ace.cast::<ACCESS_ALLOWED_ACE>();
        // SAFETY: type vérifié ci-dessus ; le SID suit le masque dans l'entrée.
        let (mask, sid) = unsafe { ((*allowed).Mask, PSID((&raw mut (*allowed).SidStart).cast())) };
        let account = account_name(sid);
        let rights = Rights::from_mask(mask);
        let inherited = header.AceFlags & INHERITED != 0;
        let entry = match out.iter_mut().find(|e| e.account == account) {
            Some(e) => e,
            None => {
                out.push(Entry {
                    account,
                    allow: Rights::default(),
                    deny: Rights::default(),
                    inherited,
                });
                out.last_mut().expect("entrée ajoutée")
            }
        };
        let target = if header.AceType == ALLOWED {
            &mut entry.allow
        } else {
            &mut entry.deny
        };
        target.full |= rights.full;
        target.modify |= rights.modify;
        target.read_execute |= rights.read_execute;
        target.read |= rights.read;
        target.write |= rights.write;
        entry.inherited |= inherited;
    }
    out
}

/// `DOMAINE\Nom` d'un SID (ou le nom seul pour les comptes intégrés sans domaine).
fn account_name(sid: PSID) -> String {
    let mut name = [0u16; 256];
    let mut domain = [0u16; 256];
    let (mut n, mut d) = (name.len() as u32, domain.len() as u32);
    let mut kind = SID_NAME_USE::default();
    // SAFETY: SID valide (fourni par le descripteur) ; tampons locaux et leurs tailles.
    let ok = unsafe {
        LookupAccountSidW(
            PCWSTR::null(),
            sid,
            Some(PWSTR(name.as_mut_ptr())),
            &mut n,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut d,
            &mut kind,
        )
    }
    .is_ok();
    if !ok {
        return "Compte inconnu".to_owned();
    }
    let name = String::from_utf16_lossy(&name[..n as usize]);
    let domain = String::from_utf16_lossy(&domain[..d as usize]);
    if domain.is_empty() {
        name
    } else {
        format!("{domain}\\{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_windows_dir_security() {
        let windir = std::env::var_os("SystemRoot").expect("SystemRoot");
        let s = read(Path::new(&windir)).expect("sécurité");
        assert!(s.owner.is_some());
        assert!(!s.entries.is_empty());
        // Le système a le contrôle total sur %SystemRoot%.
        assert!(
            s.entries.iter().any(|e| e.allow.full || e.allow.modify),
            "{s:?}"
        );
    }

    #[test]
    fn masks_map_to_explorer_rights() {
        let full = Rights::from_mask(0x001F_01FF);
        assert!(full.full && full.modify && full.read && full.write);
        let read = Rights::from_mask(0x0012_00A9);
        assert!(read.read_execute && read.read && !read.write && !read.full);
        assert!(Rights::from_mask(0x1000_0000).full);
    }
}
