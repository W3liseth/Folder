//! Services shell sur **un seul** thread STA :
//! - tâches urgentes (ouvrir, propriétés…) traitées en premier, dans l'ordre ;
//! - images (icônes, miniatures) traitées en **LIFO** : lors d'un défilement, les éléments
//!   visibles en dernier sont servis d'abord ; la file est bornée (les plus anciennes
//!   demandes sont abandonnées).

mod keys;

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

pub use fx_win::image::{ImageKind, Rgba};
pub use keys::{ImageKey, THUMBNAIL_EXTS, image_request};

/// Nombre maximal de demandes d'images en attente.
const MAX_PENDING_IMAGES: usize = 512;

type Job = Box<dyn FnOnce() + Send>;

/// Demande d'image.
#[derive(Debug, Clone)]
pub struct ImageRequest {
    pub key: ImageKey,
    /// Fichier source (un fichier quelconque de ce type pour une icône par extension).
    pub path: PathBuf,
    pub size: u32,
    pub kind: ImageKind,
}

/// Remise d'une image (ou de son échec), depuis le thread shell.
pub type ImageSink = Arc<dyn Fn(ImageKey, Option<Rgba>) + Send + Sync>;

struct Queues {
    urgent: VecDeque<Job>,
    images: Vec<ImageRequest>,
    queued: HashSet<ImageKey>,
}

struct Shared {
    q: Mutex<Queues>,
    cv: Condvar,
}

/// Poignée clonable vers le thread shell.
#[derive(Clone)]
pub struct ShellWorker {
    shared: Arc<Shared>,
}

impl ShellWorker {
    pub fn new(sink: ImageSink) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            q: Mutex::new(Queues {
                urgent: VecDeque::new(),
                images: Vec::new(),
                queued: HashSet::new(),
            }),
            cv: Condvar::new(),
        });
        let s = shared.clone();
        std::thread::Builder::new()
            .name("fx-shell".into())
            .stack_size(1024 * 1024)
            .spawn(move || run(&s, &sink))?;
        Ok(Self { shared })
    }

    /// Tâche shell prioritaire (exécutée sur le thread STA).
    pub fn spawn(&self, job: impl FnOnce() + Send + 'static) {
        if let Ok(mut q) = self.shared.q.lock() {
            q.urgent.push_back(Box::new(job));
            self.shared.cv.notify_one();
        }
    }

    /// Demande une image ; ignorée si la même est déjà en attente.
    pub fn request_image(&self, req: ImageRequest) {
        let Ok(mut q) = self.shared.q.lock() else {
            return;
        };
        if !q.queued.insert(req.key.clone()) {
            // Déjà en file : on la remonte en tête (dernière demandée = prioritaire).
            if let Some(pos) = q.images.iter().position(|r| r.key == req.key) {
                let r = q.images.remove(pos);
                q.images.push(r);
            }
            return;
        }
        q.images.push(req);
        if q.images.len() > MAX_PENDING_IMAGES {
            let dropped = q.images.remove(0);
            q.queued.remove(&dropped.key);
        }
        self.shared.cv.notify_one();
    }

    /// Abandonne les images en attente (changement de dossier).
    pub fn clear_images(&self) {
        if let Ok(mut q) = self.shared.q.lock() {
            q.images.clear();
            q.queued.clear();
        }
    }
}

/// Boucle du thread shell ; il vit aussi longtemps que le processus.
fn run(shared: &Shared, sink: &ImageSink) {
    let _com = fx_win::com::init_sta();
    loop {
        let task = {
            let Ok(mut q) = shared.q.lock() else { return };
            loop {
                if let Some(job) = q.urgent.pop_front() {
                    break Task::Job(job);
                }
                if let Some(req) = q.images.pop() {
                    q.queued.remove(&req.key);
                    break Task::Image(req);
                }
                q = match shared.cv.wait(q) {
                    Ok(q) => q,
                    Err(_) => return,
                };
            }
        };
        match task {
            Task::Job(job) => {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
                    tracing::error!("tâche shell en panique");
                }
            }
            Task::Image(req) => {
                let img = fx_win::image::shell_image(&req.path, req.size, req.kind).ok();
                sink(req.key, img);
            }
        }
    }
}

enum Task {
    Job(Job),
    Image(ImageRequest),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn urgent_jobs_and_icons() {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let w = ShellWorker::new(Arc::new(move |k, img: Option<Rgba>| {
            let _ = tx.lock().expect("lock").send((k, img.map(|i| i.width)));
        }))
        .expect("worker");
        let (jtx, jrx) = mpsc::channel();
        w.spawn(move || {
            let _ = jtx.send(42);
        });
        assert_eq!(jrx.recv_timeout(Duration::from_secs(5)).expect("job"), 42);

        let windir = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"));
        let (key, path) = image_request(&windir, "notepad.exe", "exe", false, 32);
        w.request_image(ImageRequest {
            key: key.clone(),
            path,
            size: 32,
            kind: ImageKind::Icon,
        });
        let (k, width) = rx.recv_timeout(Duration::from_secs(10)).expect("image");
        assert_eq!(k, key);
        assert!(width.is_some());
    }
}
