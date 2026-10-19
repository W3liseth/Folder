//! Opérations sur fichiers via `IFileOperation` (comme Files et l'Explorateur) :
//! corbeille, élévation UAC, boîtes de conflit natives, compatibilité totale avec le shell.
//! Notre `IFileOperationProgressSink` remonte la progression au centre d'état.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{E_ABORT, HWND};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::UI::Shell::{
    FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR, FOF_SILENT,
    FOFX_RECYCLEONDELETE, FileOperation, IFileOperation, IFileOperationProgressSink,
    IFileOperationProgressSink_Impl, IShellItem, IShellItemArray, SHCreateItemFromParsingName,
    SHCreateShellItemArrayFromIDLists, SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, PCWSTR, Ref, implement};

use crate::error::{Result, ResultExt};
use crate::util::{Pidl, take_co_string, wide, wide_str};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOp {
    Copy {
        sources: Vec<PathBuf>,
        dest: PathBuf,
    },
    Move {
        sources: Vec<PathBuf>,
        dest: PathBuf,
    },
    /// `permanent: false` envoie à la corbeille.
    Delete {
        items: Vec<PathBuf>,
        permanent: bool,
    },
    Rename {
        item: PathBuf,
        new_name: String,
    },
    /// Déplacements avec nouveau nom : `(source, dossier de destination, nom)` (restauration
    /// depuis la corbeille : `$R…` vers son dossier et son nom d'origine).
    MoveRenamed {
        items: Vec<(PathBuf, PathBuf, String)>,
    },
    /// Plusieurs renommages en une seule opération (renommage en masse, annulation).
    RenameMany {
        items: Vec<(PathBuf, String)>,
    },
    NewFolder {
        parent: PathBuf,
        name: String,
    },
    NewFile {
        parent: PathBuf,
        name: String,
    },
}

/// Événements remontés pendant l'opération (sur le thread de l'opération).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpEvent {
    /// Unités de travail effectuées / totales (unités choisies par le shell).
    Progress { done: u32, total: u32 },
    /// Élément en cours de traitement.
    Item(String),
    /// Élément créé : `(source, nouveau chemin)` ; `source` absent pour un nouvel élément.
    Created {
        source: Option<PathBuf>,
        created: PathBuf,
    },
    /// Élément renommé ou déplacé : utile pour l'annulation.
    Moved { from: PathBuf, to: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpOutcome {
    /// L'utilisateur (ou l'annulation) a interrompu au moins une partie de l'opération.
    pub aborted: bool,
}

/// Exécute `op`. Bloquant : à appeler depuis un thread STA dédié (`com::init_sta`).
/// `owner` : fenêtre parente des boîtes de dialogue natives (conflits, UAC, erreurs).
pub fn perform(
    op: &FileOp,
    owner: Option<isize>,
    cancel: Arc<AtomicBool>,
    on_event: impl FnMut(OpEvent) + 'static,
) -> Result<OpOutcome> {
    // SAFETY: CLSID de la classe FileOperation du shell ; appel sur un thread COM initialisé.
    let ifo: IFileOperation =
        unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }.op("CoCreateInstance")?;

    let mut flags: FILEOPERATION_FLAGS = FOF_SILENT | FOF_NOCONFIRMMKDIR;
    if let FileOp::Delete { permanent, .. } = op {
        // La confirmation de suppression est faite par fx (réglage utilisateur).
        flags |= FOF_NOCONFIRMATION;
        if !permanent {
            flags |= FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE;
        }
    } else {
        flags |= FOF_ALLOWUNDO;
    }
    // SAFETY: interface valide.
    unsafe { ifo.SetOperationFlags(flags) }.op("SetOperationFlags")?;
    if let Some(h) = owner {
        // SAFETY: HWND fourni par l'appelant (fenêtre de fx) ; seulement mémorisé par le shell.
        unsafe { ifo.SetOwnerWindow(HWND(h as *mut _)) }.op("SetOwnerWindow")?;
    }

    let sink: IFileOperationProgressSink = Sink {
        on_event: RefCell::new(Box::new(on_event)),
        cancel,
    }
    .into();
    // SAFETY: le sink reste vivant jusqu'à Unadvise.
    let cookie = unsafe { ifo.Advise(&sink) }.op("Advise")?;

    let queued = queue(&ifo, op);
    let run = queued.and_then(|()| {
        // SAFETY: opérations mises en file ci-dessus.
        unsafe { ifo.PerformOperations() }.op("PerformOperations")
    });
    // SAFETY: cookie issu d'Advise.
    let _ = unsafe { ifo.Unadvise(cookie) };
    // SAFETY: interface valide.
    let aborted = unsafe { ifo.GetAnyOperationsAborted() }
        .map(|b| b.as_bool())
        .unwrap_or(false);
    match run {
        Ok(()) => Ok(OpOutcome { aborted }),
        // Annulation : pas une erreur pour l'appelant.
        Err(e) if e.hresult() == E_ABORT.0 => Ok(OpOutcome { aborted: true }),
        Err(e) => Err(e),
    }
}

fn queue(ifo: &IFileOperation, op: &FileOp) -> Result<()> {
    match op {
        FileOp::Copy { sources, dest } => {
            let items = item_array(sources)?;
            let dest = shell_item(dest)?;
            // SAFETY: tableau et dossier de destination valides.
            unsafe { ifo.CopyItems(&items, &dest) }.op("CopyItems")
        }
        FileOp::Move { sources, dest } => {
            let items = item_array(sources)?;
            let dest = shell_item(dest)?;
            // SAFETY: idem.
            unsafe { ifo.MoveItems(&items, &dest) }.op("MoveItems")
        }
        FileOp::Delete { items, .. } => {
            let items = item_array(items)?;
            // SAFETY: tableau valide.
            unsafe { ifo.DeleteItems(&items) }.op("DeleteItems")
        }
        FileOp::Rename { item, new_name } => {
            let it = shell_item(item)?;
            let name = wide_str(new_name);
            // SAFETY: élément valide, nom terminé par NUL vivant pendant l'appel.
            unsafe { ifo.RenameItem(&it, PCWSTR(name.as_ptr()), None) }.op("RenameItem")
        }
        FileOp::RenameMany { items } => {
            for (item, new_name) in items {
                let it = shell_item(item)?;
                let name = wide_str(new_name);
                // SAFETY: élément valide, nom terminé par NUL vivant pendant l'appel (le shell
                // copie la chaîne à la mise en file).
                unsafe { ifo.RenameItem(&it, PCWSTR(name.as_ptr()), None) }.op("RenameItem")?;
            }
            Ok(())
        }
        FileOp::MoveRenamed { items } => {
            for (item, dest, new_name) in items {
                let it = shell_item(item)?;
                let dest = shell_item(dest)?;
                let name = wide_str(new_name);
                // SAFETY: éléments valides, nom terminé par NUL copié à la mise en file.
                unsafe { ifo.MoveItem(&it, &dest, PCWSTR(name.as_ptr()), None) }.op("MoveItem")?;
            }
            Ok(())
        }
        FileOp::NewFolder { parent, name } | FileOp::NewFile { parent, name } => {
            let folder = shell_item(parent)?;
            let attrs = if matches!(op, FileOp::NewFolder { .. }) {
                FILE_ATTRIBUTE_DIRECTORY
            } else {
                FILE_ATTRIBUTE_NORMAL
            };
            let name = wide_str(name);
            // SAFETY: dossier valide, nom terminé par NUL ; pas de modèle.
            unsafe {
                ifo.NewItem(
                    &folder,
                    attrs.0,
                    PCWSTR(name.as_ptr()),
                    PCWSTR::null(),
                    None,
                )
            }
            .op("NewItem")
        }
    }
}

fn shell_item(path: &Path) -> Result<IShellItem> {
    let w = wide(path);
    // SAFETY: chaîne terminée par NUL valide pendant l'appel.
    unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None) }
        .op("SHCreateItemFromParsingName")
}

fn item_array(paths: &[PathBuf]) -> Result<IShellItemArray> {
    let pidls = paths
        .iter()
        .map(|p| Pidl::parse(p))
        .collect::<Result<Vec<_>>>()?;
    let raw: Vec<_> = pidls.iter().map(Pidl::as_ptr).collect();
    // SAFETY: PIDL absolus valides pendant l'appel (le tableau en garde une copie).
    unsafe { SHCreateShellItemArrayFromIDLists(&raw) }.op("SHCreateShellItemArrayFromIDLists")
}

fn path_of(item: &IShellItem) -> Option<PathBuf> {
    // SAFETY: élément valide ; la chaîne renvoyée est libérée par `take_co_string`.
    unsafe {
        item.GetDisplayName(SIGDN_FILESYSPATH)
            .ok()
            .and_then(|p| take_co_string(p))
    }
}

#[implement(IFileOperationProgressSink)]
struct Sink {
    on_event: RefCell<Box<dyn FnMut(OpEvent)>>,
    cancel: Arc<AtomicBool>,
}

impl Sink {
    fn emit(&self, e: OpEvent) {
        if let Ok(mut f) = self.on_event.try_borrow_mut() {
            f(e);
        }
    }

    /// Renvoyer `E_ABORT` depuis un rappel interrompt l'opération.
    fn check(&self) -> windows::core::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(E_ABORT.into())
        } else {
            Ok(())
        }
    }

    fn item(&self, item: &Ref<IShellItem>) -> windows::core::Result<()> {
        if let Some(p) = item.as_ref().and_then(path_of) {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.emit(OpEvent::Item(name));
        }
        self.check()
    }

    fn post(&self, hr: HRESULT, src: &Ref<IShellItem>, created: &Ref<IShellItem>, moved: bool) {
        if hr.is_err() {
            return;
        }
        let (Some(src), Some(dst)) = (
            src.as_ref().and_then(path_of),
            created.as_ref().and_then(path_of),
        ) else {
            return;
        };
        self.emit(if moved {
            OpEvent::Moved { from: src, to: dst }
        } else {
            OpEvent::Created {
                source: Some(src),
                created: dst,
            }
        });
    }
}

#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for Sink_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, _hr: HRESULT) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreRenameItem(
        &self,
        _f: u32,
        item: Ref<IShellItem>,
        _n: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.item(&item)
    }
    fn PostRenameItem(
        &self,
        _f: u32,
        item: Ref<IShellItem>,
        _n: &PCWSTR,
        hr: HRESULT,
        created: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.post(hr, &item, &created, true);
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _f: u32,
        item: Ref<IShellItem>,
        _d: Ref<IShellItem>,
        _n: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.item(&item)
    }
    fn PostMoveItem(
        &self,
        _f: u32,
        item: Ref<IShellItem>,
        _d: Ref<IShellItem>,
        _n: &PCWSTR,
        hr: HRESULT,
        created: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.post(hr, &item, &created, true);
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _f: u32,
        item: Ref<IShellItem>,
        _d: Ref<IShellItem>,
        _n: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.item(&item)
    }
    fn PostCopyItem(
        &self,
        _f: u32,
        item: Ref<IShellItem>,
        _d: Ref<IShellItem>,
        _n: &PCWSTR,
        hr: HRESULT,
        created: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.post(hr, &item, &created, false);
        Ok(())
    }
    fn PreDeleteItem(&self, _f: u32, item: Ref<IShellItem>) -> windows::core::Result<()> {
        self.item(&item)
    }
    fn PostDeleteItem(
        &self,
        _f: u32,
        _item: Ref<IShellItem>,
        _hr: HRESULT,
        _created: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreNewItem(&self, _f: u32, _d: Ref<IShellItem>, _n: &PCWSTR) -> windows::core::Result<()> {
        self.check()
    }
    fn PostNewItem(
        &self,
        _f: u32,
        _d: Ref<IShellItem>,
        _n: &PCWSTR,
        _t: &PCWSTR,
        _a: u32,
        hr: HRESULT,
        item: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        if hr.is_ok()
            && let Some(p) = item.as_ref().and_then(path_of)
        {
            self.emit(OpEvent::Created {
                source: None,
                created: p,
            });
        }
        Ok(())
    }
    fn UpdateProgress(&self, total: u32, done: u32) -> windows::core::Result<()> {
        self.emit(OpEvent::Progress { done, total });
        self.check()
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn copy_rename_delete_roundtrip() {
        let _com = crate::com::init_sta();
        let base = std::env::temp_dir().join(format!("fx-fileop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("dst")).expect("mkdir");
        std::fs::write(base.join("a.txt"), b"hello").expect("write");

        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let op = FileOp::Copy {
            sources: vec![base.join("a.txt")],
            dest: base.join("dst"),
        };
        let out = perform(&op, None, cancel.clone(), move |e| {
            ev.lock().expect("lock").push(e)
        })
        .expect("copy");
        assert!(!out.aborted);
        assert!(base.join("dst").join("a.txt").exists());
        assert!(
            events
                .lock()
                .expect("lock")
                .iter()
                .any(|e| matches!(e, OpEvent::Created { .. }))
        );

        let op = FileOp::Rename {
            item: base.join("dst").join("a.txt"),
            new_name: "b.txt".into(),
        };
        perform(&op, None, cancel.clone(), |_| {}).expect("rename");
        assert!(base.join("dst").join("b.txt").exists());

        let op = FileOp::NewFolder {
            parent: base.clone(),
            name: "Nouveau".into(),
        };
        perform(&op, None, cancel.clone(), |_| {}).expect("new folder");
        assert!(base.join("Nouveau").is_dir());

        let op = FileOp::Delete {
            items: vec![base.join("dst"), base.join("Nouveau")],
            permanent: true,
        };
        perform(&op, None, cancel, |_| {}).expect("delete");
        assert!(!base.join("dst").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
