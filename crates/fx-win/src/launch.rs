//! Ouverture d'une image ou d'une vidéo « comme depuis l'Explorateur » : l'application ouverte
//! (Photos…) passe au fichier précédent ou suivant avec les flèches, **dans l'ordre affiché**.
//!
//! Photos lit cet ordre dans la vue de dossier qui l'a lancé (service `SID_SFolderView` du
//! site du verbe) et y accède ensuite pendant toute la navigation. La requête WinRT
//! `LauncherOptions.NeighboringFilesQuery` ne fonctionne pas depuis une application non
//! empaquetée, et `ShellExecute` ne transmet aucun voisin. On héberge donc une vraie vue du
//! shell (`IExplorerBrowser`, fenêtre invisible), triée comme folder, on y sélectionne le
//! fichier et on invoque son verbe par défaut depuis cette vue ; puis on la garde en vie tant
//! que la fenêtre ouverte existe. Le tout dans un processus auxiliaire
//! (`folder.exe --open-media`, cf. `fx_ui::open_media`) : la fenêtre principale ne charge pas
//! la vue du shell et peut se fermer sans couper Photos.

use std::path::Path;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, PROPERTYKEY, RECT};
use windows::Win32::Storage::EnhancedStorage::{
    PKEY_DateModified, PKEY_ItemNameDisplay, PKEY_ItemTypeText, PKEY_Size,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::{
    EBO_NOWRAPPERWINDOW, ExplorerBrowser, FOLDERSETTINGS, FVM_DETAILS, FWF_NOBROWSERVIEWSTATE,
    FWF_NOWEBVIEW, IExplorerBrowser, IFolderView2, ILFindLastID, IShellView, SBSP_ABSOLUTE,
    SORT_ASCENDING, SORT_DESCENDING, SORTCOLUMN, SVSI_DESELECTOTHERS, SVSI_FOCUSED, SVSI_SELECT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, CreateWindowExW, DestroyWindow, DispatchMessageW,
    GetForegroundWindow, GetWindowThreadProcessId, IsWindow, MSG, MsgWaitForMultipleObjects,
    PM_REMOVE, PeekMessageW, QS_ALLINPUT, TranslateMessage, WINDOW_EX_STYLE, WS_OVERLAPPED,
};
use windows::core::{PCSTR, w};

use crate::error::{Error, Result, ResultExt};
use crate::util::Pidl;

/// Colonne de tri de la vue (celle de folder).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Name,
    Modified,
    Type,
    Size,
}

impl SortBy {
    fn key(self) -> PROPERTYKEY {
        match self {
            Self::Name => PKEY_ItemNameDisplay,
            Self::Modified => PKEY_DateModified,
            Self::Type => PKEY_ItemTypeText,
            Self::Size => PKEY_Size,
        }
    }
}

/// Délai maximal pour que la vue énumère le dossier et que l'application apparaisse.
const STARTUP: Duration = Duration::from_secs(15);
/// Durée de vie maximale du processus auxiliaire (filet de sécurité).
const MAX_LIFETIME: Duration = Duration::from_secs(12 * 3600);

/// Ouvre `path` avec son application par défaut depuis une vue triée par `sort`, puis sert
/// cette vue jusqu'à la fermeture de la fenêtre de l'application. Bloquant (des secondes à
/// des heures) : depuis le thread STA du processus `--open-media`. Une erreur signifie que
/// rien n'a été lancé (l'appelant se replie sur `ShellExecute`).
pub fn open_in_sorted_view(path: &Path, sort: SortBy, ascending: bool) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| Error::new("open_in_sorted_view", windows::core::Error::empty()))?;
    let dir_pidl = Pidl::parse(dir)?;
    let file_pidl = Pidl::parse(path)?;
    // SAFETY: fenêtre hôte jamais affichée, détruite en fin de fonction sur ce thread.
    let host = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("folder"),
            WS_OVERLAPPED,
            0,
            0,
            640,
            480,
            None,
            None,
            None,
            None,
        )
    }
    .op("CreateWindowExW")?;
    let result = serve(host, &dir_pidl, &file_pidl, sort, ascending);
    // SAFETY: fenêtre créée ci-dessus.
    let _ = unsafe { DestroyWindow(host) };
    result
}

fn serve(host: HWND, dir: &Pidl, file: &Pidl, sort: SortBy, ascending: bool) -> Result<()> {
    // SAFETY: création d'un objet COM in-process sur ce thread STA.
    let browser: IExplorerBrowser =
        unsafe { CoCreateInstance(&ExplorerBrowser, None, CLSCTX_INPROC_SERVER) }
            .op("CoCreateInstance(ExplorerBrowser)")?;
    let rect = RECT {
        left: 0,
        top: 0,
        right: 640,
        bottom: 480,
    };
    // Sac de propriétés à part et pas d'enregistrement de l'état : le tri posé ici ne doit
    // pas modifier l'affichage de ce dossier dans l'Explorateur.
    let settings = FOLDERSETTINGS {
        ViewMode: FVM_DETAILS.0 as u32,
        fFlags: (FWF_NOWEBVIEW.0 | FWF_NOBROWSERVIEWSTATE.0) as u32,
    };
    // SAFETY: interface valide, fenêtre hôte vivante, structures locales.
    unsafe {
        browser
            .SetPropertyBag(w!("folder.OpenMedia"))
            .op("SetPropertyBag")?;
        browser
            .Initialize(host, &rect, Some(&settings))
            .op("IExplorerBrowser::Initialize")?;
        browser.SetOptions(EBO_NOWRAPPERWINDOW).op("SetOptions")?;
        browser
            .BrowseToIDList(dir.as_ptr(), SBSP_ABSOLUTE)
            .op("BrowseToIDList")?;
    }
    let launched = select_and_invoke(&browser, file, sort, ascending);
    if launched.is_ok() {
        wait_for_viewer();
    }
    // SAFETY: interface valide ; Destroy libère la vue.
    let _ = unsafe { browser.Destroy() };
    launched
}

/// Trie la vue, sélectionne le fichier une fois énuméré et invoque son verbe par défaut.
fn select_and_invoke(
    browser: &IExplorerBrowser,
    file: &Pidl,
    sort: SortBy,
    ascending: bool,
) -> Result<()> {
    let column = SORTCOLUMN {
        propkey: sort.key(),
        direction: if ascending {
            SORT_ASCENDING
        } else {
            SORT_DESCENDING
        },
    };
    // SAFETY: PIDL absolu valide ; le dernier identifiant vit aussi longtemps que `file`.
    let child = unsafe { ILFindLastID(file.as_ptr()) };
    let start = Instant::now();
    let mut sorted = false;
    // L'énumération de la vue est asynchrone : on réessaie en traitant les messages.
    loop {
        pump(Duration::from_millis(30));
        // SAFETY: interface valide ; échoue tant que la navigation n'est pas terminée.
        let views = unsafe {
            browser
                .GetCurrentView::<IFolderView2>()
                .and_then(|v| Ok((v.clone(), browser.GetCurrentView::<IShellView>()?)))
        };
        if let Ok((view, shell_view)) = views {
            if !sorted {
                // SAFETY: tableau d'une colonne valide pendant l'appel.
                sorted = unsafe { view.SetSortColumns(&[column]) }.is_ok();
            }
            let flags = (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_FOCUSED.0) as u32;
            // SAFETY: PIDL enfant relatif au dossier affiché.
            let _ = unsafe { shell_view.SelectItem(child, flags) };
            // SAFETY: interface valide.
            if sorted && unsafe { view.GetSelectedItem(0) }.is_ok() {
                // Le premier plan revient à l'application ouverte, comme depuis l'Explorateur.
                // SAFETY: pas de pointeur ; autorise tout processus à passer au premier plan.
                let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
                // SAFETY: verbe nul = verbe par défaut de la sélection.
                return unsafe { view.InvokeVerbOnSelection(PCSTR::null()) }
                    .op("InvokeVerbOnSelection");
            }
        }
        if start.elapsed() > STARTUP {
            return Err(Error::new(
                "open_in_sorted_view",
                windows::Win32::Foundation::E_ABORT.into(),
            ));
        }
    }
}

/// Garde la vue en vie tant que la fenêtre de l'application ouverte existe : la première
/// fenêtre d'un autre processus à passer au premier plan après le lancement.
fn wait_for_viewer() {
    // SAFETY: lecture de la fenêtre au premier plan et de son processus.
    let ours = unsafe { windows::Win32::System::Threading::GetCurrentProcessId() };
    // SAFETY: lecture seule de la fenêtre au premier plan.
    let before = unsafe { GetForegroundWindow() };
    let start = Instant::now();
    let viewer = loop {
        pump(Duration::from_millis(50));
        // SAFETY: idem.
        let fg = unsafe { GetForegroundWindow() };
        let mut pid = 0;
        // SAFETY: `pid` reçoit l'identifiant du processus de la fenêtre.
        unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
        if !fg.is_invalid() && fg != before && pid != ours {
            break Some(fg);
        }
        if start.elapsed() > STARTUP {
            break None;
        }
    };
    let Some(viewer) = viewer else {
        // Application lancée sans prendre le premier plan : la vue reste disponible une
        // minute (elle sert surtout au démarrage de l'application).
        pump(Duration::from_secs(60));
        return;
    };
    // SAFETY: IsWindow accepte n'importe quelle valeur.
    while unsafe { IsWindow(Some(viewer)) }.as_bool() && start.elapsed() < MAX_LIFETIME {
        pump(Duration::from_secs(1));
    }
}

/// Traite les messages du thread (appels COM entrants de l'application ouverte) pendant
/// `d` au plus, en dormant entre deux arrivées.
fn pump(d: Duration) {
    let end = Instant::now() + d;
    loop {
        let mut msg = MSG::default();
        // SAFETY: boucle de messages standard du thread STA.
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            // SAFETY: message reçu ci-dessus.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        // SAFETY: aucune poignée ; réveil à l'arrivée d'un message ou au délai.
        unsafe { MsgWaitForMultipleObjects(None, false, left.as_millis() as u32, QS_ALLINPUT) };
    }
}

#[cfg(test)]
mod tests {
    /// Manuel : `FX_LAUNCH_TEST=<image>` puis `cargo test -p fx-win launch -- --ignored`
    /// (ouvre l'image triée par nom décroissant ; se termine à la fermeture de la fenêtre).
    #[test]
    #[ignore = "ouvre une application"]
    fn open_sorted() {
        let Some(p) = std::env::var_os("FX_LAUNCH_TEST") else {
            return;
        };
        let _com = crate::com::init_sta();
        let r = super::open_in_sorted_view(std::path::Path::new(&p), super::SortBy::Name, false);
        assert!(r.is_ok(), "{r:?}");
    }
}
