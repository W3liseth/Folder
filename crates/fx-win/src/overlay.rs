//! Icônes superposées du shell (« overlays ») : état de synchronisation que les fournisseurs
//! cloud (Google Drive, OneDrive, Dropbox…) et les fichiers hors connexion font dessiner par
//! l'Explorateur sur l'icône d'un fichier. Les superpositions propres au shell (flèche des
//! raccourcis, partage, fichier lent) sont écartées : elles ne disent rien de la
//! synchronisation et folder ne les a jamais affichées.
//!
//! Seuls les chemins chez un fournisseur cloud sont interrogés (cf. [`in_cloud`]) : la
//! première interrogation charge dans le processus les extensions de tous les fournisseurs
//! installés (Google Drive y crée 4 threads), ce qu'il faut éviter dans les autres dossiers.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{
    IDO_SHGIOI_DEFAULT, IDO_SHGIOI_LINK, IDO_SHGIOI_SHARE, IDO_SHGIOI_SLOWFILE, IShellIconOverlay,
    SHBindToParent, SHGetIconOverlayIndexW, SHGetImageList,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};
use windows::core::PCWSTR;

use crate::error::{Error, Result, ResultExt};
use crate::image::Rgba;
use crate::util::Pidl;

/// Index (dans la liste d'images système) de l'icône superposée de `path`, s'il en a une
/// d'un fournisseur.
///
/// Bloquant (interroge chaque gestionnaire de superposition installé, parfois par IPC) :
/// depuis le thread STA du shell.
pub fn overlay_of(path: &Path) -> Result<Option<i32>> {
    if !in_cloud(path) {
        return Ok(None);
    }
    let pidl = Pidl::parse(path)?;
    let mut child = std::ptr::null_mut();
    // SAFETY: PIDL absolu valide ; `child` pointe dans `pidl`, qui vit pendant l'appel.
    let folder: IShellIconOverlay =
        unsafe { SHBindToParent(pidl.as_ptr(), Some(&mut child)) }.op("SHBindToParent")?;
    // OI_DEFAULT en entrée (pas d'appel asynchrone) ; inchangé (ou ≤ 0) s'il n'y en a pas
    // (S_FALSE, que `windows` ne distingue pas de S_OK).
    let mut index = -1i32;
    // SAFETY: `child` est le dernier identifiant de `pidl`, encore vivant.
    let _ = unsafe { folder.GetOverlayIconIndex(child, &mut index) };
    Ok((index > 0 && !shell_owned(index)).then_some(index))
}

/// Racines cloud relues au plus à cet intervalle (fournisseur installé ou déconnecté).
const ROOTS_TTL: Duration = Duration::from_secs(30);

/// Le chemin est-il chez un fournisseur cloud : sous une racine de synchronisation déclarée
/// à Windows (OneDrive, Dropbox, iCloud, Box… : `SyncRootManager`, et les dossiers OneDrive
/// des variables d'environnement), ou sur le lecteur virtuel de Google Drive (volume nommé
/// « Google Drive », sans racine déclarée) ?
pub fn in_cloud(path: &Path) -> bool {
    static ROOTS: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);
    let Ok(mut cache) = ROOTS.lock() else {
        return false;
    };
    if cache
        .as_ref()
        .is_none_or(|(at, _)| at.elapsed() >= ROOTS_TTL)
    {
        *cache = Some((Instant::now(), cloud_roots()));
    }
    let mut p = path.to_string_lossy().to_lowercase();
    if !p.ends_with('\\') {
        p.push('\\');
    }
    cache
        .as_ref()
        .is_some_and(|(_, roots)| roots.iter().any(|r| p.starts_with(r.as_str())))
}

/// Racines cloud en minuscules, terminées par `\`.
fn cloud_roots() -> Vec<String> {
    let mut roots = sync_roots();
    for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
        roots.extend(std::env::var(var).ok());
    }
    roots.extend(
        crate::sys::drives()
            .into_iter()
            .filter(|d| d.label.eq_ignore_ascii_case("Google Drive"))
            .map(|d| d.root.to_string_lossy().into_owned()),
    );
    roots
        .into_iter()
        .filter(|r| !r.is_empty())
        .map(|r| {
            let mut r = r.to_lowercase();
            if !r.ends_with('\\') {
                r.push('\\');
            }
            r
        })
        .collect()
}

/// Dossiers des racines de synchronisation des fournisseurs (Cloud Files API), tous comptes
/// confondus : `SyncRootManager\<id>\UserSyncRoots`, une valeur par compte.
fn sync_roots() -> Vec<String> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegEnumKeyExW, RegEnumValueW,
        RegOpenKeyExW,
    };
    use windows::core::{PWSTR, w};
    let mut base = HKEY::default();
    // SAFETY: clé racine prédéfinie, sous-clé littérale, handle fermé plus bas.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager"),
            None,
            KEY_READ,
            &mut base,
        )
    };
    if opened.is_err() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for index in 0.. {
        let mut sub = [0u16; 256];
        let mut len = sub.len() as u32;
        // SAFETY: tampon local et sa taille en caractères.
        let r = unsafe {
            RegEnumKeyExW(
                base,
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
        let mut name: Vec<u16> = sub[..len as usize].to_vec();
        name.extend(r"\UserSyncRoots".encode_utf16());
        name.push(0);
        let mut users = HKEY::default();
        // SAFETY: sous-clé terminée par NUL, handle fermé plus bas.
        if unsafe { RegOpenKeyExW(base, PCWSTR(name.as_ptr()), None, KEY_READ, &mut users) }
            .is_err()
        {
            continue;
        }
        for v in 0.. {
            let mut value = [0u16; 256];
            let mut value_len = value.len() as u32;
            let mut data = [0u16; 1024];
            let mut data_len = (data.len() * 2) as u32;
            // SAFETY: tampons locaux et leurs tailles (en octets pour la donnée).
            let r = unsafe {
                RegEnumValueW(
                    users,
                    v,
                    Some(PWSTR(value.as_mut_ptr())),
                    &mut value_len,
                    None,
                    None,
                    Some(data.as_mut_ptr().cast()),
                    Some(&mut data_len),
                )
            };
            if r.is_err() {
                break;
            }
            let units = &data[..(data_len as usize / 2).min(data.len())];
            let n = units.iter().position(|&c| c == 0).unwrap_or(units.len());
            out.push(String::from_utf16_lossy(&units[..n]));
        }
        // SAFETY: handle ouvert ci-dessus.
        let _ = unsafe { RegCloseKey(users) };
    }
    // SAFETY: handle ouvert ci-dessus.
    let _ = unsafe { RegCloseKey(base) };
    out
}

/// Superposition fournie par le shell lui-même (raccourci, partage, fichier lent, défaut).
fn shell_owned(index: i32) -> bool {
    // Constantes de types différents selon la version de `windows` : ramenées en i32.
    [
        IDO_SHGIOI_LINK as i32,
        IDO_SHGIOI_SHARE as i32,
        IDO_SHGIOI_SLOWFILE as i32,
        IDO_SHGIOI_DEFAULT as i32,
    ]
    .into_iter()
    // SAFETY: chemin nul : identifiant de superposition prédéfini.
    .any(|id| unsafe { SHGetIconOverlayIndexW(PCWSTR::null(), id) } == index)
}

/// Image de la superposition `index`, à la taille de liste système la plus proche de `size`
/// pixels (16, 32, 48 ou 256). Elle couvre toute l'icône ; le badge est dans un coin.
pub fn overlay_image(index: i32, size: u32) -> Result<Rgba> {
    // SHIL_SMALL (1), SHIL_LARGE (0), SHIL_EXTRALARGE (2), SHIL_JUMBO (4).
    let list = match size {
        0..=16 => 1,
        17..=32 => 0,
        33..=48 => 2,
        _ => 4,
    };
    // SAFETY: identifiant de liste documenté.
    let images: IImageList = unsafe { SHGetImageList(list) }.op("SHGetImageList")?;
    // SAFETY: index de la liste système ; l'icône renvoyée nous appartient.
    let icon = unsafe { images.GetIcon(index, ILD_TRANSPARENT.0) }.op("IImageList::GetIcon")?;
    let mut info = ICONINFO::default();
    // SAFETY: icône valide ; les deux bitmaps renvoyés nous appartiennent.
    let got = unsafe { GetIconInfo(icon, &mut info) };
    // SAFETY: icône créée par GetIcon, détruite une seule fois.
    let _ = unsafe { DestroyIcon(icon) };
    got.op("GetIconInfo")?;
    let _mask = crate::image::OwnedBitmap(info.hbmMask);
    let color = crate::image::OwnedBitmap(info.hbmColor);
    if color.0.is_invalid() {
        return Err(Error::new(
            "GetIconInfo",
            windows::core::Error::from_thread(),
        ));
    }
    let mut img = crate::image::read_bitmap(color.0)?;
    // Couleurs d'icône non prémultipliées ; `Rgba` l'est.
    for p in img.pixels.as_chunks_mut::<4>().0 {
        let a = u16::from(p[3]);
        for c in &mut p[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_folders_are_not_cloud() {
        let windows = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        assert!(!in_cloud(Path::new(&windows)));
        assert!(!in_cloud(&std::env::temp_dir()));
        // Pas d'interrogation (ni d'extension chargée) hors du cloud.
        assert_eq!(overlay_of(Path::new(&windows)).ok().flatten(), None);
    }

    /// Mesure manuelle (lecteurs cloud de la machine) :
    /// `cargo test -p fx-win overlay::tests::cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn cost() {
        let _com = crate::com::init_sta();
        // Racine de G:, son premier sous-dossier, et un dossier système (aucun nom affiché).
        let sub = std::fs::read_dir(r"G:\")
            .ok()
            .and_then(|rd| rd.flatten().find(|e| e.path().is_dir()))
            .map(|e| e.path());
        let dirs = [
            Some(std::path::PathBuf::from(r"G:\")),
            sub,
            Some(r"C:\Windows\System32".into()),
        ];
        for dir in dirs.into_iter().flatten() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            let paths: Vec<_> = rd.flatten().take(60).map(|e| e.path()).collect();
            for pass in ["à froid", "à chaud"] {
                let t = std::time::Instant::now();
                let mut found = std::collections::BTreeMap::new();
                for p in &paths {
                    if let Ok(Some(i)) = overlay_of(p) {
                        *found.entry(i).or_insert(0) += 1;
                    }
                }
                let per = t.elapsed().as_secs_f64() * 1000.0 / paths.len().max(1) as f64;
                println!(
                    "{} : {} éléments {pass}, {per:.2} ms chacun, superpositions {found:?}",
                    if dir.starts_with("G:") {
                        "G:"
                    } else {
                        "système"
                    },
                    paths.len()
                );
            }
        }
        for size in [16, 32, 48, 256] {
            if let Ok(img) = overlay_image(383, size) {
                println!("  taille {size} : {}×{}", img.width, img.height);
            }
        }
    }
}
