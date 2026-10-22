//! Menu contextuel du shell (`IContextMenu`) et glisser-déposer sortant.
//!
//! Comme l'Explorateur de Windows 11, folder affiche d'abord son propre menu ; « Afficher plus
//! d'options » (ou Maj + clic droit) y ajoute les entrées du menu shell complet, extensions
//! tierces comprises. Ce menu n'est pas affiché par Windows : [`build`] le construit puis le
//! lit (textes, raccourcis, icônes, états, sous-menus, y compris ceux que les extensions ne
//! remplissent qu'à l'ouverture), folder le dessine, et [`invoke`] exécute l'entrée choisie.
//! Construction et exécution sur le thread UI : les extensions comptent sur la boucle de
//! messages de la fenêtre propriétaire.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::Com::{IBindCtx, IDataObject};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, IDropSource,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CMF_EXPLORE, CMF_EXTENDEDVERBS, CMF_NORMAL, CMIC_MASK_CONTROL_DOWN, CMIC_MASK_PTINVOKE,
    CMIC_MASK_SHIFT_DOWN, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, GCS_VERBW, IContextMenu,
    IContextMenu2, IContextMenu3, ILFindLastID, IShellFolder, SHBindToObject, SHBindToParent,
    SHCreateDataObject, SHDoDragDrop,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetMenuItemCount, GetMenuItemInfoW, HMENU, MENUITEMINFOW,
    MFS_CHECKED, MFS_DEFAULT, MFS_DISABLED, MFT_OWNERDRAW, MFT_SEPARATOR, MIIM_BITMAP, MIIM_FTYPE,
    MIIM_ID, MIIM_STATE, MIIM_STRING, MIIM_SUBMENU, SW_SHOWNORMAL, WM_INITMENUPOPUP,
};
use windows::core::{Interface, PCSTR, PCWSTR, PSTR, PWSTR};

use crate::error::{Error, Result, ResultExt};
use crate::image::Rgba;
use crate::util::{Pidl, wide};

const CMIC_MASK_UNICODE: u32 = 0x4000;
const FIRST_CMD: u32 = 1;
const LAST_CMD: u32 = 0x7FFF;
/// Profondeur maximale des sous-menus lus (les extensions n'en imbriquent guère plus).
const MAX_DEPTH: u32 = 4;

/// Entrée du menu shell, telle que l'Explorateur l'afficherait.
#[derive(Debug, Clone)]
pub enum MenuItem {
    Separator,
    Command {
        /// Identifiant à passer à [`invoke`].
        id: u32,
        label: String,
        /// Raccourci affiché à droite (texte après la tabulation), souvent vide.
        shortcut: String,
        /// Verbe canonique (« open », « delete », « properties »…), s'il est déclaré.
        verb: Option<String>,
        enabled: bool,
        checked: bool,
        /// Commande par défaut (en gras, exécutée par un double-clic).
        default: bool,
        icon: Option<Rgba>,
    },
    Submenu {
        label: String,
        enabled: bool,
        icon: Option<Rgba>,
        items: Vec<MenuItem>,
    },
}

/// Résultat d'une entrée choisie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuOutcome {
    /// Commande exécutée par le shell ; `verb` canonique s'il est connu (« delete », « copy »…).
    Invoked { verb: Option<String> },
    /// « Renommer » : folder gère le renommage sur place.
    Rename,
}

/// Menu construit, gardé jusqu'à l'exécution d'une entrée (ou au prochain menu) : l'objet
/// des extensions, le menu Win32 qu'elles ont rempli et le dossier courant de l'exécution.
struct Live {
    menu: IContextMenu,
    _hmenu: MenuGuard,
    dir: Option<PathBuf>,
}

thread_local! {
    static LIVE: RefCell<Option<Live>> = const { RefCell::new(None) };
}

/// Construit et lit le menu shell des `items` (tous dans le même dossier) ; `extended` :
/// entrées étendues (comme Maj + clic droit dans l'Explorateur). Bloquant (extensions
/// tierces) : depuis le thread UI, qui exécutera ensuite l'entrée choisie.
pub fn build(hwnd: isize, items: &[PathBuf], extended: bool) -> Result<Vec<MenuItem>> {
    let Some(first) = items.first() else {
        return Ok(Vec::new());
    };
    let pidls = items
        .iter()
        .map(|p| Pidl::parse(p))
        .collect::<Result<Vec<_>>>()?;
    let mut last = std::ptr::null_mut();
    // SAFETY: PIDL absolu valide ; `last` pointe dans ce PIDL (pas d'allocation).
    let parent: IShellFolder =
        unsafe { SHBindToParent(pidls[0].as_ptr(), Some(&mut last)) }.op("SHBindToParent")?;
    let children: Vec<*const ITEMIDLIST> = pidls
        .iter()
        // SAFETY: PIDL valides ; le dernier identifiant vit aussi longtemps que `pidls`.
        .map(|p| unsafe { ILFindLastID(p.as_ptr()) }.cast_const())
        .collect();
    // SAFETY: PIDL enfants relatifs au même parent, valides pendant l'appel.
    let menu: IContextMenu = unsafe { parent.GetUIObjectOf(HWND(hwnd as *mut _), &children, None) }
        .op("GetUIObjectOf")?;
    let dir = first.parent().map(Path::to_path_buf);
    read(menu, extended, dir)
}

/// Construit et lit le menu shell du **fond** du dossier `dir` (Nouveau, Coller…).
pub fn build_background(hwnd: isize, dir: &Path, extended: bool) -> Result<Vec<MenuItem>> {
    let pidl = Pidl::parse(dir)?;
    // SAFETY: PIDL absolu valide ; liaison au dossier lui-même (pas de parent).
    let folder: IShellFolder =
        unsafe { SHBindToObject(None::<&IShellFolder>, pidl.as_ptr(), None::<&IBindCtx>) }
            .op("SHBindToObject")?;
    // SAFETY: dossier valide ; le menu de fond est obtenu via la vue du dossier.
    let menu: IContextMenu =
        unsafe { folder.CreateViewObject(HWND(hwnd as *mut _)) }.op("CreateViewObject")?;
    read(menu, extended, Some(dir.to_path_buf()))
}

/// Oublie le menu construit (menu refermé sans choix) : libère les extensions.
pub fn release() {
    LIVE.with(|l| *l.borrow_mut() = None);
}

fn read(menu: IContextMenu, extended: bool, dir: Option<PathBuf>) -> Result<Vec<MenuItem>> {
    // SAFETY: création d'un menu vide, détruit par `MenuGuard`.
    let hmenu = unsafe { CreatePopupMenu() }.op("CreatePopupMenu")?;
    let guard = MenuGuard(hmenu);
    let mut flags = CMF_NORMAL | CMF_EXPLORE;
    if extended {
        flags |= CMF_EXTENDEDVERBS;
    }
    // SAFETY: menu et interface valides.
    unsafe { menu.QueryContextMenu(hmenu, 0, FIRST_CMD, LAST_CMD, flags) }
        .ok()
        .op("QueryContextMenu")?;
    let items = read_items(&menu, hmenu, 0);
    LIVE.with(|l| {
        *l.borrow_mut() = Some(Live {
            menu,
            _hmenu: guard,
            dir,
        });
    });
    Ok(items)
}

/// Entrées du menu `hmenu`. Les sous-menus sont d'abord « ouverts » pour l'extension
/// (`WM_INITMENUPOPUP`), qui ne remplit les siens (Envoyer vers, Ouvrir avec…) qu'à ce moment.
fn read_items(menu: &IContextMenu, hmenu: HMENU, depth: u32) -> Vec<MenuItem> {
    let mut out = Vec::new();
    // SAFETY: menu valide (rempli par QueryContextMenu).
    let count = unsafe { GetMenuItemCount(Some(hmenu)) };
    for pos in 0..u32::try_from(count).unwrap_or(0) {
        let mut info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_SUBMENU | MIIM_STRING | MIIM_BITMAP,
            ..Default::default()
        };
        // SAFETY: structure initialisée avec sa taille ; sans tampon, `cch` reçoit la longueur.
        if unsafe { GetMenuItemInfoW(hmenu, pos, true, &mut info) }.is_err() {
            continue;
        }
        if info.fType.0 & MFT_SEPARATOR.0 != 0 {
            out.push(MenuItem::Separator);
            continue;
        }
        // Entrée dessinée par l'extension (pas de texte lisible) : ignorée.
        if info.fType.0 & MFT_OWNERDRAW.0 != 0 || info.cch == 0 {
            continue;
        }
        // Le second appel (texte seul) peut remettre à zéro les autres champs : on les garde.
        let (sub, id, state, hbm) = (info.hSubMenu, info.wID, info.fState.0, info.hbmpItem);
        let mut text = vec![0u16; info.cch as usize + 1];
        info.fMask = MIIM_STRING;
        info.dwTypeData = PWSTR(text.as_mut_ptr());
        info.cch += 1;
        // SAFETY: tampon de `cch` unités, vivant pendant l'appel.
        if unsafe { GetMenuItemInfoW(hmenu, pos, true, &mut info) }.is_err() {
            continue;
        }
        let n = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        let raw = String::from_utf16_lossy(&text[..n]);
        let (label, shortcut) = match raw.split_once('\t') {
            Some((l, s)) => (strip_mnemonic(l), s.trim().to_owned()),
            None => (strip_mnemonic(&raw), String::new()),
        };
        if label.is_empty() {
            continue;
        }
        let enabled = state & MFS_DISABLED.0 == 0;
        let icon = bitmap(hbm);
        if !sub.is_invalid() {
            if depth >= MAX_DEPTH {
                continue;
            }
            init_popup(menu, sub, pos);
            let items = read_items(menu, sub, depth + 1);
            out.push(MenuItem::Submenu {
                label,
                enabled,
                icon,
                items,
            });
            continue;
        }
        if !(FIRST_CMD..=LAST_CMD).contains(&id) {
            continue;
        }
        let id = id - FIRST_CMD;
        out.push(MenuItem::Command {
            id,
            label,
            shortcut,
            verb: verb_of(menu, id as usize),
            enabled,
            checked: state & MFS_CHECKED.0 != 0,
            default: state & MFS_DEFAULT.0 != 0,
            icon,
        });
    }
    out
}

/// Signale à l'extension l'ouverture du sous-menu `sub` (position `pos`).
fn init_popup(menu: &IContextMenu, sub: HMENU, pos: u32) {
    let (wparam, lparam) = (WPARAM(sub.0 as usize), LPARAM(pos as isize));
    if let Ok(m3) = menu.cast::<IContextMenu3>() {
        // SAFETY: message de menu destiné à l'extension propriétaire du menu.
        let _ = unsafe { m3.HandleMenuMsg2(WM_INITMENUPOPUP, wparam, lparam, None) };
    } else if let Ok(m2) = menu.cast::<IContextMenu2>() {
        // SAFETY: idem.
        let _ = unsafe { m2.HandleMenuMsg(WM_INITMENUPOPUP, wparam, lparam) };
    }
}

/// Icône d'une entrée (bitmap 32 bits du menu, qui en reste propriétaire). Les bitmaps
/// système (`HBMMENU_*`, petites valeurs) et dessinés à la demande (-1) n'en ont pas.
fn bitmap(hbm: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<Rgba> {
    let v = hbm.0 as isize;
    if (-1..=11).contains(&v) {
        return None;
    }
    crate::image::read_bitmap(hbm).ok()
}

/// Libellé sans marque de raccourci clavier (`&Ouvrir` → `Ouvrir`, `&&` → `&`).
fn strip_mnemonic(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                out.push('&');
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out.trim().to_owned()
}

/// Touches enfoncées au moment de l'invocation (transmises aux extensions).
#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
}

/// Exécute l'entrée `id` du dernier menu construit sur ce thread ; `at` : position écran du
/// menu (certaines extensions y ouvrent leur fenêtre). Le menu est ensuite libéré.
pub fn invoke(hwnd: isize, id: u32, at: (i32, i32), mods: Modifiers) -> Result<MenuOutcome> {
    let Some(live) = LIVE.with(|l| l.borrow_mut().take()) else {
        return Err(Error::new(
            "InvokeCommand",
            windows::core::Error::from_thread(),
        ));
    };
    let verb = verb_of(&live.menu, id as usize);
    if verb.as_deref() == Some("rename") {
        return Ok(MenuOutcome::Rename);
    }
    let dir_w = live.dir.as_deref().map(wide);
    let mut mask = CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE;
    if mods.shift {
        mask |= CMIC_MASK_SHIFT_DOWN;
    }
    if mods.ctrl {
        mask |= CMIC_MASK_CONTROL_DOWN;
    }
    let info = CMINVOKECOMMANDINFOEX {
        cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: mask,
        hwnd: HWND(hwnd as *mut _),
        // MAKEINTRESOURCE : l'identifiant relatif passé à la place d'une chaîne.
        lpVerb: PCSTR(id as usize as *const u8),
        lpVerbW: PCWSTR(id as usize as *const u16),
        lpDirectoryW: dir_w
            .as_ref()
            .map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ptInvoke: POINT { x: at.0, y: at.1 },
        ..Default::default()
    };
    // SAFETY: structure étendue complète dont cbSize indique la taille ; chaînes vivantes.
    unsafe {
        live.menu
            .InvokeCommand((&raw const info).cast::<CMINVOKECOMMANDINFO>())
    }
    .op("InvokeCommand")?;
    Ok(MenuOutcome::Invoked { verb })
}

fn verb_of(menu: &IContextMenu, offset: usize) -> Option<String> {
    let mut buf = [0u16; 128];
    // SAFETY: GCS_VERBW écrit au plus `cchmax` unités UTF-16 dans `buf`.
    let ok = unsafe {
        menu.GetCommandString(
            offset,
            GCS_VERBW,
            None,
            PSTR(buf.as_mut_ptr().cast()),
            buf.len() as u32,
        )
    };
    ok.ok()?;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    (len > 0).then(|| String::from_utf16_lossy(&buf[..len]).to_ascii_lowercase())
}

struct MenuGuard(HMENU);

impl Drop for MenuGuard {
    fn drop(&mut self) {
        // SAFETY: menu créé par CreatePopupMenu, détruit une seule fois.
        let _ = unsafe { DestroyMenu(self.0) };
    }
}

/// Effet du glisser-déposer sortant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropResult {
    None,
    Copy,
    Move,
    Link,
}

/// Lance un glisser-déposer shell des `items` (même dossier), avec image de glissement native.
/// Bloquant jusqu'au dépôt : à appeler depuis le thread UI (OLE initialisé).
pub fn drag_out(hwnd: isize, items: &[PathBuf]) -> Result<DropResult> {
    if items.is_empty() {
        return Ok(DropResult::None);
    }
    let pidls = items
        .iter()
        .map(|p| Pidl::parse(p))
        .collect::<Result<Vec<_>>>()?;
    let parent_path = items[0]
        .parent()
        .ok_or_else(|| Error::new("drag_out", windows::core::Error::from_thread()))?;
    let parent = Pidl::parse(parent_path)?;
    // SAFETY: PIDL valides ; derniers identifiants relatifs au parent commun.
    let children: Vec<*const ITEMIDLIST> = pidls
        .iter()
        .map(|p| unsafe { ILFindLastID(p.as_ptr()) }.cast_const())
        .collect();
    // SAFETY: PIDL parent et enfants valides pendant l'appel (copiés par l'objet).
    let data: IDataObject =
        unsafe { SHCreateDataObject(Some(parent.as_ptr()), Some(&children), None::<&IDataObject>) }
            .op("SHCreateDataObject")?;
    let allowed = DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0 | DROPEFFECT_LINK.0);
    // SAFETY: source de glissement par défaut du shell (pdsrc nul) ; boucle modale.
    let effect = unsafe {
        SHDoDragDrop(
            Some(HWND(hwnd as *mut _)),
            &data,
            None::<&IDropSource>,
            allowed,
        )
    }
    .op("SHDoDragDrop");
    crate::window::end_mouse_drag(hwnd);
    let effect = effect?;
    Ok(if effect.0 & DROPEFFECT_MOVE.0 != 0 {
        DropResult::Move
    } else if effect.0 & DROPEFFECT_COPY.0 != 0 {
        DropResult::Copy
    } else if effect.0 & DROPEFFECT_LINK.0 != 0 {
        DropResult::Link
    } else {
        DropResult::None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnemonics() {
        assert_eq!(strip_mnemonic("&Ouvrir"), "Ouvrir");
        assert_eq!(strip_mnemonic("Copier && coller"), "Copier & coller");
        assert_eq!(strip_mnemonic(" Envoyer v&ers "), "Envoyer vers");
    }

    /// Lecture du menu shell d'un fichier temporaire (extensions de la machine) :
    /// `cargo test -p fx-win ctxmenu::tests::read_menu -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn read_menu() {
        let _com = crate::com::init_sta();
        crate::window::init_ole();
        let file = std::env::temp_dir().join("folder-menu-test.txt");
        std::fs::write(&file, "x").expect("fichier");
        for extended in [false, true] {
            let t = std::time::Instant::now();
            let items = build(0, std::slice::from_ref(&file), extended).expect("menu");
            println!("--- étendu={extended} en {:?}", t.elapsed());
            print(&items, 0);
            release();
        }
        let _ = std::fs::remove_file(&file);
    }

    fn print(items: &[MenuItem], depth: usize) {
        for it in items {
            let pad = "  ".repeat(depth);
            match it {
                MenuItem::Separator => println!("{pad}----"),
                MenuItem::Command {
                    label,
                    shortcut,
                    verb,
                    enabled,
                    default,
                    icon,
                    ..
                } => println!(
                    "{pad}{label} [{shortcut}] verbe={verb:?} actif={enabled} défaut={default} icône={}",
                    icon.as_ref()
                        .map_or("-".into(), |i| format!("{}×{}", i.width, i.height))
                ),
                MenuItem::Submenu { label, items, .. } => {
                    println!("{pad}{label} >");
                    print(items, depth + 1);
                }
            }
        }
    }
}
