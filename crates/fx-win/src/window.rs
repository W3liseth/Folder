//! Fenêtre principale : recherche du HWND, placement (écran, taille, état agrandi), cadre DWM
//! (coins arrondis, ombre, mode sombre), position du curseur.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromRect};
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GetCursorPos, GetWindowPlacement, GetWindowTextW, IsZoomed,
    SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED, SW_SHOWNORMAL, SetWindowPlacement, WINDOWPLACEMENT,
    WPF_RESTORETOMAXIMIZED,
};
use windows::core::BOOL;

/// Fenêtre principale du thread courant, repérée par son titre (elle peut être encore cachée :
/// folder la crée invisible pour l'afficher directement à sa place), en `isize`.
pub fn main_window(title: &str) -> Option<isize> {
    let mut search = Search {
        title: title.encode_utf16().collect(),
        found: 0,
    };
    // SAFETY: le rappel ne vit que pendant l'appel et n'accède à `search` que via LPARAM.
    unsafe {
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(enum_proc),
            LPARAM((&raw mut search) as isize),
        );
    }
    (search.found != 0).then_some(search.found)
}

struct Search {
    title: Vec<u16>,
    found: isize,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: LPARAM pointe vers la `Search` de `main_window`, vivante pendant l'énumération.
    let search = unsafe { &mut *(lparam.0 as *mut Search) };
    let mut buf = [0u16; 64];
    // SAFETY: HWND fourni par le système, tampon local.
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if usize::try_from(n).is_ok_and(|n| buf[..n] == search.title[..]) {
        search.found = hwnd.0 as isize;
        return false.into();
    }
    true.into()
}

/// Placement d'une fenêtre en pixels physiques : rectangle de la fenêtre restaurée
/// (coordonnées de l'espace de travail, comme `GetWindowPlacement`) et état agrandi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub maximized: bool,
}

/// Placement actuel de la fenêtre (même cachée ou réduite : on retient alors l'état qu'elle
/// retrouvera, agrandie ou non).
pub fn placement(hwnd: isize) -> Option<Placement> {
    let hwnd = HWND(hwnd as *mut _);
    let mut wp = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: structure initialisée avec sa taille.
    unsafe { GetWindowPlacement(hwnd, &mut wp) }.ok()?;
    // SAFETY: HWND valide (vient d'être interrogé).
    let zoomed = unsafe { IsZoomed(hwnd) }.as_bool();
    let minimized_from_max =
        wp.showCmd == SW_SHOWMINIMIZED.0 as u32 && wp.flags.contains(WPF_RESTORETOMAXIMIZED);
    let r = wp.rcNormalPosition;
    Some(Placement {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
        maximized: zoomed || minimized_from_max,
    })
}

/// Place la fenêtre (écran, taille, état agrandi) et l'affiche directement ainsi, en une
/// seule opération (comme l'Explorateur), sans passer par la taille par défaut. Renvoie faux,
/// sans rien faire, si le rectangle n'est plus sur un écran branché.
pub fn show_with_placement(hwnd: isize, p: &Placement) -> bool {
    let rect = RECT {
        left: p.left,
        top: p.top,
        right: p.right,
        bottom: p.bottom,
    };
    // SAFETY: rectangle local ; un HMONITOR nul signifie « sur aucun écran ».
    if unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) }.is_invalid() {
        return false;
    }
    let show = if p.maximized {
        SW_SHOWMAXIMIZED
    } else {
        SW_SHOWNORMAL
    };
    let wp = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        showCmd: show.0 as u32,
        ptMinPosition: POINT { x: -1, y: -1 },
        ptMaxPosition: POINT { x: -1, y: -1 },
        rcNormalPosition: rect,
        ..Default::default()
    };
    // SAFETY: structure complète, HWND de la fenêtre principale.
    unsafe { SetWindowPlacement(HWND(hwnd as *mut _), &wp) }.is_ok()
}

/// Cadre Windows 11 pour une fenêtre sans décorations natives : coins arrondis, ombre DWM,
/// barre de titre système en mode sombre si demandé.
pub fn style_frame(hwnd: isize, dark: bool) {
    let hwnd = HWND(hwnd as *mut _);
    let pref = DWMWCP_ROUND;
    let dark = BOOL::from(dark);
    let margins = MARGINS {
        cxLeftWidth: 1,
        cxRightWidth: 1,
        cyTopHeight: 1,
        cyBottomHeight: 1,
    };
    // SAFETY: attributs DWM documentés, pointeurs vers des valeurs locales de la bonne taille.
    // Les échecs (Windows 10 sans coins arrondis…) sont sans conséquence.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const pref).cast(),
            4,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark).cast(),
            4,
        );
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
}

/// Barre de titre native d'une fenêtre secondaire en mode sombre (ou clair).
pub fn set_dark_title(hwnd: isize, dark: bool) {
    let dark = BOOL::from(dark);
    // SAFETY: attribut DWM documenté, valeur locale de la bonne taille.
    let _ = unsafe {
        DwmSetWindowAttribute(
            HWND(hwnd as *mut _),
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark).cast(),
            4,
        )
    };
}

/// Ramène au premier plan la fenêtre de titre `title` (de n'importe quel processus) ;
/// faux si elle n'existe pas.
pub fn focus_title(title: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    let w: Vec<u16> = title.encode_utf16().chain([0]).collect();
    // SAFETY: titre terminé par NUL ; HWND renvoyé par le système.
    unsafe {
        let Ok(hwnd) = FindWindowW(
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR(w.as_ptr()),
        ) else {
            return false;
        };
        if hwnd.is_invalid() {
            return false;
        }
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
    true
}

/// Rectangle (pixels physiques) d'une fenêtre, s'il existe encore.
pub fn rect_of(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    let mut r = RECT::default();
    // SAFETY: `r` locale ; un HWND invalide fait simplement échouer l'appel.
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(HWND(hwnd as *mut _), &mut r) }
        .ok()?;
    Some((r.left, r.top, r.right, r.bottom))
}

/// Attend la fin du processus propriétaire de la fenêtre `hwnd` (fenêtres secondaires qui
/// se ferment avec la fenêtre principale, même si celle-ci plante). Bloquant : depuis un
/// thread dédié. Retourne aussitôt si la fenêtre n'existe plus.
pub fn wait_owner_exit(hwnd: isize) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    let mut pid = 0;
    // SAFETY: `pid` locale ; un HWND invalide donne 0.
    unsafe { GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid)) };
    if pid == 0 {
        return;
    }
    // SAFETY: droit de synchronisation seul ; la poignée est refermée ci-dessous.
    let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
        return;
    };
    // SAFETY: poignée valide ouverte ci-dessus.
    unsafe { WaitForSingleObject(process, INFINITE) };
    // SAFETY: poignée ouverte ci-dessus, fermée une seule fois.
    let _ = unsafe { CloseHandle(process) };
}

/// Icône de l'application (ressource 1 de l'exécutable, cf. `app/folder/folder.rc`) pour la
/// barre de titre, la barre des tâches et Alt+Tab, aux tailles du DPI de la fenêtre (winit
/// n'en pose aucune).
pub fn set_app_icon(hwnd: isize) {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::{
        ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON, SM_CXSMICON,
        SendMessageW, WM_SETICON,
    };
    let hwnd = HWND(hwnd as *mut _);
    // SAFETY: module de l'exécutable courant ; HWND éventuellement invalide (DPI 0 → 96).
    let (module, dpi) = unsafe { (GetModuleHandleW(None), GetDpiForWindow(hwnd)) };
    let Ok(module) = module else { return };
    let dpi = if dpi == 0 { 96 } else { dpi };
    for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
        // SAFETY: lecture d'une métrique système.
        let size = unsafe { GetSystemMetricsForDpi(metric, dpi) };
        // SAFETY: MAKEINTRESOURCE(1) dans le module courant ; l'icône vit autant que la
        // fenêtre (jamais détruite : deux icônes par fenêtre et par processus).
        let icon = unsafe {
            LoadImageW(
                Some(module.into()),
                // MAKEINTRESOURCE(1) : identifiant entier à la place du nom.
                windows::core::PCWSTR(std::ptr::without_provenance::<u16>(1)),
                IMAGE_ICON,
                size,
                size,
                LR_DEFAULTCOLOR,
            )
        };
        if let Ok(icon) = icon {
            // SAFETY: message standard ; l'icône reste valide (voir ci-dessus).
            unsafe {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(windows::Win32::Foundation::WPARAM(kind as usize)),
                    Some(windows::Win32::Foundation::LPARAM(icon.0 as isize)),
                )
            };
        }
    }
}

/// Position du curseur en coordonnées écran (pixels physiques).
pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT::default();
    // SAFETY: `p` est une POINT valide.
    let _ = unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

/// Position du curseur dans la zone client de `hwnd` (pixels physiques, éventuellement hors
/// de la fenêtre). Sert pendant un glisser-déposer OLE, où la fenêtre ne reçoit aucun
/// message de souris.
pub fn client_cursor_pos(hwnd: isize) -> Option<(i32, i32)> {
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    let mut p = POINT::default();
    // SAFETY: `p` est une POINT valide ; un HWND invalide fait échouer la conversion.
    unsafe {
        GetCursorPos(&mut p).ok()?;
        ScreenToClient(HWND(hwnd as *mut _), &mut p).ok().ok()?;
    }
    Some((p.x, p.y))
}

/// Termine, pour la fenêtre, un glisser commencé chez elle et achevé par une boucle modale
/// (`SHDoDragDrop`) : celle-ci consomme le relâchement du bouton, que la fenêtre croirait
/// donc toujours enfoncé (plus aucun survol jusqu'au clic suivant). On lui envoie la
/// position actuelle du curseur puis le relâchement du bouton gauche.
pub fn end_mouse_drag(hwnd: isize) {
    use windows::Win32::Foundation::WPARAM;
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_LBUTTONUP, WM_MOUSEMOVE};
    let (x, y) = client_cursor_pos(hwnd).unwrap_or((-1, -1));
    // MAKELPARAM : coordonnées signées sur 16 bits (négatives hors de la fenêtre).
    let lparam = LPARAM(((y as u16 as u32) << 16 | x as u16 as u32) as isize);
    let hwnd = Some(HWND(hwnd as *mut _));
    // SAFETY: messages postés (asynchrones) à notre propre fenêtre, sans pointeur.
    unsafe {
        let _ = PostMessageW(hwnd, WM_MOUSEMOVE, WPARAM(0), lparam);
        let _ = PostMessageW(hwnd, WM_LBUTTONUP, WPARAM(0), lparam);
    }
}

/// Autre fenêtre principale de folder (autre processus) visible au point écran (`x`, `y`) :
/// la plus haute des fenêtres qui le couvrent, `own` exclue (elle peut être sous le pointeur
/// pendant qu'on la déplace), si elle a le titre et la classe de `own`.
pub fn peer_at(own: isize, x: i32, y: i32) -> Option<isize> {
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
    let mut find = Find {
        own: HWND(own as *mut _),
        pt: POINT { x, y },
        found: None,
    };
    // SAFETY: le rappel ne vit que pendant l'appel et n'accède à `find` que via LPARAM.
    unsafe {
        let _ = EnumWindows(Some(topmost_at), LPARAM((&raw mut find) as isize));
    }
    let top = find.found?;
    (text_of(top, false) == text_of(find.own, false)
        && text_of(top, true) == text_of(find.own, true))
    .then_some(top.0 as isize)
}

struct Find {
    own: HWND,
    pt: POINT,
    found: Option<HWND>,
}

/// `EnumWindows` parcourt les fenêtres de haut en bas : la première visible qui couvre le
/// point est celle qu'on voit. Ignorées : fenêtres cachées, réduites, masquées par DWM
/// (autres bureaux virtuels, applications suspendues) ou transparentes aux clics.
unsafe extern "system" fn topmost_at(hwnd: HWND, lparam: LPARAM) -> BOOL {
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindowVisible, WS_EX_TRANSPARENT,
    };
    // SAFETY: LPARAM pointe vers la `Find` de `peer_at`, vivante pendant l'énumération.
    let find = unsafe { &mut *(lparam.0 as *mut Find) };
    if hwnd == find.own {
        return true.into();
    }
    let mut r = RECT::default();
    let mut cloaked = 0u32;
    // SAFETY: HWND fourni par le système ; sorties locales de la bonne taille.
    let visible = unsafe {
        IsWindowVisible(hwnd).as_bool()
            && !IsIconic(hwnd).as_bool()
            && GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TRANSPARENT.0 as isize == 0
            && DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, (&raw mut cloaked).cast(), 4).is_ok()
            && cloaked == 0
            && GetWindowRect(hwnd, &mut r).is_ok()
    };
    let p = find.pt;
    if visible && p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom {
        find.found = Some(hwnd);
        return false.into();
    }
    true.into()
}

/// Déplace la fenêtre (coin haut gauche en pixels écran), sans la redimensionner ni changer
/// son rang ou son activation.
pub fn move_to(hwnd: isize, x: i32, y: i32) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    };
    // SAFETY: HWND de notre fenêtre ; un échec est sans conséquence.
    let _ = unsafe {
        SetWindowPos(
            HWND(hwnd as *mut _),
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}

/// Masque (ou réaffiche) la fenêtre à l'écran par DWM : elle reste « visible » pour Windows
/// et winit, garde la capture de la souris et son contenu (rien à redessiner ensuite).
pub fn set_cloaked(hwnd: isize, cloaked: bool) {
    use windows::Win32::Graphics::Dwm::DWMWA_CLOAK;
    let value = BOOL::from(cloaked);
    // SAFETY: attribut DWM documenté, valeur locale de la bonne taille.
    let _ = unsafe {
        DwmSetWindowAttribute(
            HWND(hwnd as *mut _),
            DWMWA_CLOAK,
            (&raw const value).cast(),
            4,
        )
    };
}

/// Une fenêtre de premier niveau titrée `title` appartient-elle à un processus lancé depuis
/// `exe` ? (Le titre seul ne suffit pas : l'Explorateur titre ses fenêtres du nom du dossier.)
pub fn exists_for_exe(title: &str, exe: &std::path::Path) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
    let mut find = ExeFind {
        title: title.encode_utf16().collect(),
        exe: exe.as_os_str().to_ascii_lowercase(),
        found: false,
    };
    // SAFETY: le rappel ne vit que pendant l'appel et n'accède à `find` que via LPARAM.
    unsafe {
        let _ = EnumWindows(Some(exe_window), LPARAM((&raw mut find) as isize));
    }
    find.found
}

struct ExeFind {
    title: Vec<u16>,
    exe: std::ffi::OsString,
    found: bool,
}

unsafe extern "system" fn exe_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    // SAFETY: LPARAM pointe vers la `ExeFind` de `exists_for_exe`, vivante pendant l'énumération.
    let find = unsafe { &mut *(lparam.0 as *mut ExeFind) };
    if text_of(hwnd, false) != find.title {
        return true.into();
    }
    let mut pid = 0u32;
    // SAFETY: HWND fourni par le système, sortie locale.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    let same = crate::process::image_path(pid)
        .is_some_and(|p| p.as_os_str().to_ascii_lowercase() == find.exe);
    if same {
        find.found = true;
        return false.into();
    }
    true.into()
}

/// Titre (ou nom de classe si `class`) d'une fenêtre, tronqué à 128 caractères.
fn text_of(hwnd: HWND, class: bool) -> Vec<u16> {
    use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
    let mut buf = [0u16; 128];
    // SAFETY: HWND quelconque (échec → 0), tampon local.
    let n = unsafe {
        if class {
            GetClassNameW(hwnd, &mut buf)
        } else {
            GetWindowTextW(hwnd, &mut buf)
        }
    };
    buf[..usize::try_from(n).unwrap_or(0)].to_vec()
}

/// Facteur d'échelle d'une fenêtre (1.0 à 96 ppp).
pub fn scale_of(hwnd: isize) -> f32 {
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    // SAFETY: lecture ; un HWND invalide donne 0.
    let dpi = unsafe { GetDpiForWindow(HWND(hwnd as *mut _)) };
    if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
}

/// Ramène une fenêtre au premier plan (restaurée si réduite). Réussit depuis le processus
/// qui a le premier plan, celui où l'utilisateur vient d'agir.
pub fn bring_to_front(hwnd: isize) {
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    let hwnd = HWND(hwnd as *mut _);
    // SAFETY: HWND quelconque ; les échecs sont sans conséquence.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

/// OLE sur le thread UI (glisser-déposer, presse-papiers OLE). Idempotent.
pub fn init_ole() {
    // SAFETY: pas de pointeur réservé ; S_FALSE si déjà initialisé (winit le fait aussi).
    let _ = unsafe { OleInitialize(None) };
}
