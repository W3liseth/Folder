//! Messages entre fenêtres folder de processus différents (fenêtre Paramètres → fenêtre
//! principale) par `WM_COPYDATA` : pas de thread ni de canal nommé à ouvrir, le message
//! arrive dans la boucle de la fenêtre principale.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_COPYDATA};

/// Signature des messages folder (« fold »), pour ignorer tout autre `WM_COPYDATA`.
const MAGIC: usize = 0x666F_6C64;
const SUBCLASS_ID: usize = 0x666F_6C65;

type Sink = Box<dyn Fn(Vec<u8>)>;

/// Envoie `data` à la fenêtre `hwnd` (bloquant jusqu'à son traitement) ; vrai si accepté.
pub fn send(hwnd: isize, data: &[u8]) -> bool {
    let cds = COPYDATASTRUCT {
        dwData: MAGIC,
        cbData: data.len() as u32,
        lpData: data.as_ptr() as *mut _,
    };
    // SAFETY: la structure et les données vivent pendant l'appel synchrone.
    let r = unsafe {
        SendMessageW(
            HWND(hwnd as *mut _),
            WM_COPYDATA,
            Some(WPARAM(0)),
            Some(LPARAM((&raw const cds) as isize)),
        )
    };
    r.0 != 0
}

/// Reçoit les messages folder adressés à `hwnd` (fenêtre du thread courant) : `sink` est
/// appelé sur ce thread, dans la boucle de messages. Une seule fois par fenêtre.
pub fn listen(hwnd: isize, sink: impl Fn(Vec<u8>) + 'static) -> bool {
    let boxed: Box<Sink> = Box::new(Box::new(sink));
    let data = Box::into_raw(boxed) as usize;
    // SAFETY: fenêtre du thread courant ; `data` (jamais libéré : vit autant que la fenêtre,
    // c'est-à-dire autant que le processus) est relu par `proc`.
    unsafe { SetWindowSubclass(HWND(hwnd as *mut _), Some(proc), SUBCLASS_ID, data) }.as_bool()
}

unsafe extern "system" fn proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    if msg == WM_COPYDATA && lparam.0 != 0 {
        // SAFETY: pour WM_COPYDATA, LPARAM désigne une COPYDATASTRUCT valide pendant l'appel.
        let cds = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
        if cds.dwData == MAGIC {
            let bytes = if cds.lpData.is_null() || cds.cbData == 0 {
                Vec::new()
            } else {
                // SAFETY: tampon de `cbData` octets fourni par le système pendant l'appel.
                unsafe { std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize) }
                    .to_vec()
            };
            // SAFETY: `data` est le `Sink` installé par `listen`, jamais libéré.
            let sink = unsafe { &*(data as *const Sink) };
            sink(bytes);
            return LRESULT(1);
        }
    }
    // SAFETY: transmission au traitement par défaut de la chaîne de sous-classes.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
