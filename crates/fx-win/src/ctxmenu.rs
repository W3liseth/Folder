//! Menu contextuel shell natif (`IContextMenu`) et glisser-déposer sortant.
//!
//! Comme l'Explorateur de Windows 11, fx affiche d'abord son propre menu ; l'entrée
//! « Afficher plus d'options » ouvre ce menu shell complet (extensions tierces comprises).

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{IBindCtx, IDataObject};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, IDropSource,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CMF_EXPLORE, CMF_EXTENDEDVERBS, CMF_NORMAL, CMIC_MASK_CONTROL_DOWN, CMIC_MASK_PTINVOKE,
    CMIC_MASK_SHIFT_DOWN, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, DefSubclassProc, GCS_VERBW,
    IContextMenu, IContextMenu2, IContextMenu3, ILFindLastID, IShellFolder, RemoveWindowSubclass,
    SHBindToObject, SHBindToParent, SHCreateDataObject, SHDoDragDrop, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
    WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};
use windows::core::{Interface, PCSTR, PCWSTR, PSTR};

use crate::error::{Error, Result, ResultExt};
use crate::util::{Pidl, wide};

const CMIC_MASK_UNICODE: u32 = 0x4000;
const FIRST_CMD: u32 = 1;
const LAST_CMD: u32 = 0x7FFF;
const SUBCLASS_ID: usize = 0xF0F0;

/// Résultat d'un menu shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuOutcome {
    Dismissed,
    /// Commande exécutée par le shell ; `verb` canonique s'il est connu (« delete », « copy »…).
    Invoked {
        verb: Option<String>,
    },
    /// L'utilisateur a choisi « Renommer » : fx gère le renommage sur place.
    Rename,
}

/// Affiche le menu shell des `items` (tous dans le même dossier) à la position écran `at`.
/// Bloquant (boucle modale du menu) : à appeler depuis le thread UI.
pub fn show(
    hwnd: isize,
    items: &[PathBuf],
    at: (i32, i32),
    extended: bool,
    mods: Modifiers,
) -> Result<MenuOutcome> {
    let Some(first) = items.first() else {
        return Ok(MenuOutcome::Dismissed);
    };
    let hwnd = HWND(hwnd as *mut _);
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
    let menu: IContextMenu =
        unsafe { parent.GetUIObjectOf(hwnd, &children, None) }.op("GetUIObjectOf")?;
    let dir = first.parent().map(Path::to_path_buf);
    run_menu(hwnd, &menu, at, extended, mods, dir.as_deref())
}

/// Affiche le menu shell du **fond** du dossier `dir` (Nouveau, Coller, Affichage…).
pub fn show_background(
    hwnd: isize,
    dir: &Path,
    at: (i32, i32),
    extended: bool,
    mods: Modifiers,
) -> Result<MenuOutcome> {
    let hwnd = HWND(hwnd as *mut _);
    let pidl = Pidl::parse(dir)?;
    // SAFETY: PIDL absolu valide ; liaison au dossier lui-même (pas de parent).
    let folder: IShellFolder =
        unsafe { SHBindToObject(None::<&IShellFolder>, pidl.as_ptr(), None::<&IBindCtx>) }
            .op("SHBindToObject")?;
    // SAFETY: dossier valide ; le menu de fond est obtenu via la vue du dossier.
    let menu: IContextMenu = unsafe { folder.CreateViewObject(hwnd) }.op("CreateViewObject")?;
    run_menu(hwnd, &menu, at, extended, mods, Some(dir))
}

/// Touches enfoncées au moment de l'invocation (transmises aux extensions).
#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
}

fn run_menu(
    hwnd: HWND,
    menu: &IContextMenu,
    at: (i32, i32),
    extended: bool,
    mods: Modifiers,
    dir: Option<&Path>,
) -> Result<MenuOutcome> {
    // SAFETY: création d'un menu vide, détruit par `MenuGuard`.
    let hmenu = unsafe { CreatePopupMenu() }.op("CreatePopupMenu")?;
    let _guard = MenuGuard(hmenu);
    let mut flags = CMF_NORMAL | CMF_EXPLORE;
    if extended {
        flags |= CMF_EXTENDEDVERBS;
    }
    // SAFETY: menu et interface valides.
    let hr = unsafe { menu.QueryContextMenu(hmenu, 0, FIRST_CMD, LAST_CMD, flags) };
    hr.ok().op("QueryContextMenu")?;

    // Les sous-menus dynamiques (Envoyer vers, Ouvrir avec) sont dessinés par l'extension :
    // on lui transmet les messages de menu reçus par notre fenêtre.
    ACTIVE.with(|a| *a.borrow_mut() = Some(menu.clone()));
    // SAFETY: sous-classement retiré ci-dessous, sur le même thread.
    let _ = unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0) };
    // SAFETY: menu valide ; TPM_RETURNCMD renvoie l'identifiant choisi (0 si annulé).
    let cmd = unsafe {
        TrackPopupMenuEx(
            hmenu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            at.0,
            at.1,
            hwnd,
            None,
        )
    }
    .0 as u32;
    // SAFETY: retire le sous-classement posé ci-dessus.
    let _ = unsafe { RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID) };
    ACTIVE.with(|a| *a.borrow_mut() = None);

    if cmd < FIRST_CMD {
        return Ok(MenuOutcome::Dismissed);
    }
    let offset = (cmd - FIRST_CMD) as usize;
    let verb = verb_of(menu, offset);
    if verb.as_deref() == Some("rename") {
        return Ok(MenuOutcome::Rename);
    }

    let dir_w = dir.map(wide);
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
        hwnd,
        // MAKEINTRESOURCE : l'identifiant relatif passé à la place d'une chaîne.
        lpVerb: PCSTR(offset as *const u8),
        lpVerbW: PCWSTR(offset as *const u16),
        lpDirectoryW: dir_w
            .as_ref()
            .map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ptInvoke: POINT { x: at.0, y: at.1 },
        ..Default::default()
    };
    // SAFETY: structure étendue complète dont cbSize indique la taille ; chaînes vivantes.
    unsafe { menu.InvokeCommand((&raw const info).cast::<CMINVOKECOMMANDINFO>()) }
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

thread_local! {
    static ACTIVE: RefCell<Option<IContextMenu>> = const { RefCell::new(None) };
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if matches!(
        msg,
        WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
    ) {
        let handled = ACTIVE.with(|a| {
            let a = a.borrow();
            let menu = a.as_ref()?;
            if let Ok(m3) = menu.cast::<IContextMenu3>() {
                let mut res = LRESULT(0);
                // SAFETY: message de menu transmis tel quel à l'extension propriétaire.
                unsafe { m3.HandleMenuMsg2(msg, wparam, lparam, Some(&mut res)) }.ok()?;
                return Some(res);
            }
            let m2 = menu.cast::<IContextMenu2>().ok()?;
            // SAFETY: idem.
            unsafe { m2.HandleMenuMsg(msg, wparam, lparam) }.ok()?;
            Some(LRESULT(if msg == WM_INITMENUPOPUP { 0 } else { 1 }))
        });
        if let Some(r) = handled {
            return r;
        }
    }
    // SAFETY: chaînage standard vers la procédure suivante.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

struct MenuGuard(windows::Win32::UI::WindowsAndMessaging::HMENU);

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
    .op("SHDoDragDrop")?;
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
