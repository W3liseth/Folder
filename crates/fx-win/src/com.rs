use std::marker::PhantomData;

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};

/// Initialise COM en STA pour le thread courant ; désinitialise au `Drop`.
///
/// Requis par tout thread qui touche au shell (icônes, menus contextuels, `IFileOperation`).
/// Non `Send` : un appartement COM est lié à son thread.
pub struct ComApartment {
    must_uninit: bool,
    _not_send: PhantomData<*const ()>,
}

/// Entre dans un appartement STA. Si le thread est déjà en MTA, on n'y touche pas.
pub fn init_sta() -> ComApartment {
    // SAFETY: pas de pointeur réservé ; l'appel est valide sur n'importe quel thread.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    // S_OK et S_FALSE exigent tous deux un CoUninitialize ; RPC_E_CHANGED_MODE non.
    ComApartment {
        must_uninit: hr.is_ok() && hr != RPC_E_CHANGED_MODE,
        _not_send: PhantomData,
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.must_uninit {
            // SAFETY: équilibre le CoInitializeEx réussi de `init_sta`, sur le même thread
            // (garanti par `!Send`).
            unsafe { CoUninitialize() };
        }
    }
}
