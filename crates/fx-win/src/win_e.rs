//! Processus résident `folder.exe --win-e` : prend Windows + E à l'Explorateur.
//!
//! Depuis Windows 11 26H2, l'Explorateur ignore la commande `opennewwindow` remplacée dans
//! le registre (méthode historique) : seul un crochet clavier bas niveau (comme PowerToys)
//! intercepte la combinaison de façon fiable. Le crochet tourne dans un petit processus à
//! part, sans fenêtre, lancé à l'ouverture de session ; folder fermé, Windows + E ouvre
//! quand même folder. Arrêter ce processus rend aussitôt la touche à l'Explorateur.
//!
//! Limite (UIPI) : quand une fenêtre élevée (administrateur) a le focus, le crochet ne voit
//! pas les touches et Windows + E ouvre l'Explorateur.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, GetCurrentThreadId, INFINITE,
    MUTEX_MODIFY_STATE, OpenEventW, OpenMutexW, ReleaseMutex, ResetEvent,
    SYNCHRONIZATION_SYNCHRONIZE, SetEvent, WaitForSingleObject,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CallNextHookEx, DispatchMessageW, HC_ACTION, KBDLLHOOKSTRUCT, MSG,
    MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, PostThreadMessageW, QS_ALLINPUT,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_QUIT,
    WM_SYSKEYDOWN,
};
use windows::core::PCWSTR;

use crate::error::{Result, ResultExt};
use crate::util::wide_str;

/// Une seule instance par session (espace de noms `Local`).
const MUTEX: &str = r"Local\folder-win-e";
/// Signalé par [`stop`] : le processus résident se termine.
const QUIT_EVENT: &str = r"Local\folder-win-e-quit";
/// Message posté par le crochet à son propre thread : l'ouverture se fait hors du crochet,
/// qui doit rendre la main en quelques millisecondes (sinon Windows le retire).
const WM_PRESS: u32 = WM_APP + 1;
/// Touche virtuelle non attribuée, envoyée entre l'appui et le relâchement de Windows pour
/// que celui-ci n'ouvre pas le menu Démarrer (même procédé qu'AutoHotkey).
const MASK_KEY: u16 = 0xE8;

static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);
/// E avalé (appui vu) : la répétition et le relâchement le sont aussi.
static SWALLOWED: AtomicBool = AtomicBool::new(false);

/// Fait tourner le crochet jusqu'à [`stop`] ; `on_press` est appelé à chaque Windows + E.
/// `Ok(false)` aussitôt si une autre instance tourne déjà.
pub fn run(mut on_press: impl FnMut()) -> Result<bool> {
    let name = wide_str(MUTEX);
    // SAFETY: nom terminé par NUL ; le mutex est pris tant que le processus tourne.
    let mutex = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) }.op("CreateMutexW")?;
    // SAFETY: lu juste après CreateMutexW, qui le pose quand le mutex existait déjà.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // SAFETY: handle obtenu ci-dessus.
        let _ = unsafe { CloseHandle(mutex) };
        return Ok(false);
    }
    let result = hook_loop(&mut on_press);
    // SAFETY: mutex pris par ce thread à sa création.
    unsafe {
        let _ = ReleaseMutex(mutex);
        let _ = CloseHandle(mutex);
    }
    result.map(|()| true)
}

fn hook_loop(on_press: &mut dyn FnMut()) -> Result<()> {
    let name = wide_str(QUIT_EVENT);
    // SAFETY: nom terminé par NUL ; événement à réinitialisation manuelle.
    let quit =
        unsafe { CreateEventW(None, true, false, PCWSTR(name.as_ptr())) }.op("CreateEventW")?;
    // SAFETY: handle valide ; un arrêt demandé avant ce démarrage ne compte pas.
    let _ = unsafe { ResetEvent(quit) };
    // SAFETY: appel sans argument.
    HOOK_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
    // SAFETY: module de l'exécutable ; `keyboard_hook` vit autant que le processus.
    let hook = unsafe {
        let module = GetModuleHandleW(PCWSTR::null()).op("GetModuleHandleW")?;
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(module.into()), 0)
    }
    .op("SetWindowsHookExW")?;
    let mut msg = MSG::default();
    'run: loop {
        // Le crochet est appelé pendant cette attente (messages envoyés au thread).
        // SAFETY: un handle valide ; attente sans fin, réveillée par les messages.
        let r = unsafe { MsgWaitForMultipleObjects(Some(&[quit]), false, INFINITE, QS_ALLINPUT) };
        if r == WAIT_OBJECT_0 {
            break;
        }
        // SAFETY: `msg` local ; messages de toutes les fenêtres du thread (il n'y en a pas).
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            match msg.message {
                WM_QUIT => break 'run,
                WM_PRESS => on_press(),
                // SAFETY: message tout juste retiré de la file.
                _ => unsafe {
                    DispatchMessageW(&msg);
                },
            }
        }
    }
    // SAFETY: crochet et handle créés ci-dessus.
    unsafe {
        let _ = UnhookWindowsHookEx(hook);
        let _ = CloseHandle(quit);
    }
    Ok(())
}

/// Crochet clavier bas niveau : avale Windows + E (sans Ctrl, Alt ni Maj).
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: pour HC_ACTION, lparam pointe sur un KBDLLHOOKSTRUCT valide pendant l'appel.
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Touches injectées comprises (outils de remappage) ; celle qui masque le menu
        // Démarrer n'est pas E.
        if kb.vkCode == u32::from(b'E') {
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if down && SWALLOWED.load(Ordering::Relaxed) {
                return LRESULT(1); // répétition automatique : une seule fenêtre par appui
            }
            if down && win_alone() {
                SWALLOWED.store(true, Ordering::Relaxed);
                mask_start_menu();
                // SAFETY: thread de ce crochet ; message sans données.
                let _ = unsafe {
                    PostThreadMessageW(
                        HOOK_THREAD.load(Ordering::Relaxed),
                        WM_PRESS,
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
                return LRESULT(1);
            }
            if !down && SWALLOWED.swap(false, Ordering::Relaxed) {
                return LRESULT(1);
            }
        }
    }
    // SAFETY: arguments reçus, transmis tels quels au crochet suivant.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Windows enfoncée, sans autre modificateur.
fn win_alone() -> bool {
    // SAFETY: lecture de l'état asynchrone du clavier.
    let held = |vk: VIRTUAL_KEY| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0;
    (held(VK_LWIN) || held(VK_RWIN)) && !held(VK_CONTROL) && !held(VK_MENU) && !held(VK_SHIFT)
}

fn mask_start_menu() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(MASK_KEY),
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    // SAFETY: tableau local de structures INPUT et leur taille.
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

/// Autorise le processus `pid` (la fenêtre folder qui vient d'être lancée) à passer au
/// premier plan : le processus résident y a droit, il vient d'envoyer une touche.
pub fn allow_foreground(pid: u32) {
    // SAFETY: simple autorisation, sans effet si elle est refusée.
    let _ = unsafe { AllowSetForegroundWindow(pid) };
}

/// Le processus résident tourne-t-il dans cette session ?
pub fn is_running() -> bool {
    match open_mutex() {
        Some(m) => {
            // SAFETY: handle obtenu par OpenMutexW.
            let _ = unsafe { CloseHandle(m) };
            true
        }
        None => false,
    }
}

fn open_mutex() -> Option<HANDLE> {
    let name = wide_str(MUTEX);
    // SAFETY: nom terminé par NUL ; droits d'attente et de libération.
    unsafe {
        OpenMutexW(
            SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE,
            false,
            PCWSTR(name.as_ptr()),
        )
    }
    .ok()
}

/// Arrête le processus résident s'il tourne, et attend sa fin (2 s au plus) : Windows + E
/// revient à l'Explorateur au retour.
pub fn stop() {
    let Some(mutex) = open_mutex() else {
        return;
    };
    let name = wide_str(QUIT_EVENT);
    // SAFETY: nom terminé par NUL ; droit de signaler seulement.
    if let Ok(quit) = unsafe { OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(name.as_ptr())) } {
        // SAFETY: handle obtenu ci-dessus.
        unsafe {
            let _ = SetEvent(quit);
            let _ = CloseHandle(quit);
        }
    }
    // Rendu (ou abandonné) quand le processus se termine.
    // SAFETY: handle valide, attente bornée.
    unsafe {
        let _ = WaitForSingleObject(mutex, 2000);
        let _ = ReleaseMutex(mutex);
        let _ = CloseHandle(mutex);
    }
}
