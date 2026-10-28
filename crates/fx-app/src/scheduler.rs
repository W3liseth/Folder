use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};
use fx_core::{Batch, CancelToken, SortDir, SortKey};
use fx_fs::{EnumStats, Outcome, WatchService};
use fx_shell::{ImageSink, ShellWorker};
use fx_store::{Db, Paths};
use fx_win::fileop::{FileOp, OpEvent};

use crate::effect::{Effect, Persist, Target};
use crate::msg::Msg;
use crate::ops::Job;
use crate::state::RecentItem;

/// Réveille l'UI après l'envoi d'un message (en pratique `egui::Context::request_repaint`).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

type Task = Box<dyn FnOnce() + Send>;

/// Canal de retour vers l'UI.
#[derive(Clone)]
struct Outbox {
    tx: Sender<Msg>,
    waker: Waker,
}

impl Outbox {
    fn send(&self, msg: Msg) {
        // L'UI peut être fermée : l'échec d'envoi n'est pas une erreur.
        if self.tx.send(msg).is_ok() {
            (self.waker)();
        }
    }
}

/// Exécute les [`Effect`] hors du thread UI (ARCHITECTURE.md §6.2) :
/// - un pool d'I/O de taille fixe (énumérations, persistance, aperçus, lecteurs) ;
/// - le thread shell STA de `fx-shell` (actions shell, icônes, miniatures) ;
/// - le thread de surveillance de `fx-fs` ;
/// - un thread temporaire par opération longue (copie, compression…) et par recherche.
pub struct Scheduler {
    io: Sender<Task>,
    shell: ShellWorker,
    watch: WatchService,
    out: Outbox,
    paths: Paths,
    db: Arc<Mutex<Option<Db>>>,
    owner: Arc<AtomicIsize>,
    threads: usize,
}

impl Scheduler {
    pub fn new(
        io_threads: usize,
        tx: Sender<Msg>,
        waker: Waker,
        images: ImageSink,
        paths: Paths,
        db: Option<Db>,
    ) -> std::io::Result<Self> {
        let (io, io_rx) = unbounded::<Task>();
        let io_threads = io_threads.max(1);
        for i in 0..io_threads {
            spawn_worker(format!("fx-io-{i}"), io_rx.clone())?;
        }
        let out = Outbox { tx, waker };
        let shell = ShellWorker::new(images)?;
        let o = out.clone();
        let watch = WatchService::new(move |ev| o.send(Msg::DirChanged(ev)))?;
        Ok(Self {
            io,
            shell,
            watch,
            out,
            paths,
            db: Arc::new(Mutex::new(db)),
            owner: Arc::new(AtomicIsize::new(0)),
            threads: io_threads + 2,
        })
    }

    /// Nombre de threads permanents possédés par le scheduler.
    pub fn thread_count(&self) -> usize {
        self.threads
    }

    /// Thread shell (icônes et miniatures demandées par l'UI).
    pub fn shell(&self) -> &ShellWorker {
        &self.shell
    }

    /// Fenêtre parente des boîtes de dialogue natives (conflits de copie, UAC).
    pub fn set_owner_window(&self, hwnd: isize) {
        self.owner.store(hwnd, Ordering::Relaxed);
    }

    /// Sauvegarde immédiate de la session (à la fermeture uniquement).
    pub fn save_session_now(&self, session: &fx_store::Session) {
        if let Err(e) = fx_store::save_session(&self.paths, session) {
            tracing::warn!(error = %e, "session non sauvegardée");
        }
    }

    fn io(&self, task: impl FnOnce() + Send + 'static) {
        let _ = self.io.send(Box::new(task));
    }

    pub fn dispatch(&self, effect: Effect) {
        let out = self.out.clone();
        match effect {
            Effect::Enumerate {
                pane,
                generation,
                path,
                options,
                cancel,
                target,
            } => self.io(move || {
                let result = fx_fs::enumerate(&path, &options, &cancel, |batch| {
                    out.send(match target {
                        Target::Pane => Msg::ListingBatch {
                            pane,
                            generation,
                            batch,
                        },
                        Target::Column => Msg::ColumnBatch {
                            pane,
                            generation,
                            batch,
                        },
                    });
                })
                .map_err(|e| e.message().trim().to_owned());
                out.send(match target {
                    Target::Pane => Msg::ListingFinished {
                        pane,
                        generation,
                        result,
                    },
                    Target::Column => Msg::ColumnFinished { pane, generation },
                });
            }),
            Effect::Search {
                pane,
                generation,
                root,
                query,
                show_hidden,
                cancel,
            } => {
                spawn_temp("fx-search", move || {
                    let stats = fx_fs::search(&root, &query, show_hidden, &cancel, |batch| {
                        out.send(Msg::ListingBatch {
                            pane,
                            generation,
                            batch,
                        });
                    });
                    out.send(Msg::SearchFinished {
                        pane,
                        generation,
                        stats,
                    });
                });
            }
            Effect::ListPaths {
                pane,
                generation,
                paths,
            } => self.io(move || {
                let started = std::time::Instant::now();
                let mut by_dir: Vec<(PathBuf, Batch)> = Vec::new();
                for p in &paths {
                    let (Some(dir), Some(name), Some(st)) =
                        (p.parent(), p.file_name(), fx_win::sys::stat(p))
                    else {
                        continue;
                    };
                    let wide: Vec<u16> = name.to_string_lossy().encode_utf16().collect();
                    let slot = match by_dir.iter().position(|(d, _)| d == dir) {
                        Some(i) => i,
                        None => {
                            by_dir.push((dir.to_path_buf(), Batch::default()));
                            by_dir.len() - 1
                        }
                    };
                    by_dir[slot].1.push_utf16(
                        &wide,
                        st.attributes,
                        st.size,
                        st.created,
                        st.modified,
                    );
                }
                let entries = by_dir.iter().map(|(_, b)| b.len() as u32).sum();
                for (dir, mut batch) in by_dir {
                    batch.dir = Some(dir);
                    out.send(Msg::ListingBatch {
                        pane,
                        generation,
                        batch,
                    });
                }
                let stats = EnumStats {
                    entries,
                    batches: 1,
                    elapsed: started.elapsed(),
                };
                out.send(Msg::ListingFinished {
                    pane,
                    generation,
                    result: Ok(Outcome::Completed(stats)),
                });
            }),
            Effect::Watch { key, path } => self.watch.watch(key, &path),
            Effect::Unwatch { key } => self.watch.unwatch(key),
            Effect::Open(path) => self.shell.spawn(move || {
                if let Err(e) = fx_win::shell::open(&path) {
                    out.send(Msg::Error(format!(
                        "Impossible d'ouvrir « {} » : {}",
                        display(&path),
                        e.message().trim()
                    )));
                }
            }),
            // Processus auxiliaire (cf. `fx_win::launch`) : il reste en vie tant que la
            // fenêtre ouverte existe. Un thread temporaire guette seulement un échec rapide
            // (rien de lancé), pour se replier sur ShellExecute.
            Effect::OpenMedia { path, sort } => {
                let spawned = std::thread::Builder::new()
                    .name("folder-launch".into())
                    .spawn(move || {
                        let key = match sort.key {
                            SortKey::Name => "name",
                            SortKey::Modified => "modified",
                            SortKey::Type => "type",
                            SortKey::Size => "size",
                        };
                        let order = if sort.dir == SortDir::Asc {
                            "asc"
                        } else {
                            "desc"
                        };
                        let child = std::env::current_exe().and_then(|exe| {
                            std::process::Command::new(exe)
                                .args(["--open-media", key, order, "--"])
                                .arg(&path)
                                .spawn()
                        });
                        if let Ok(mut child) = child {
                            let start = std::time::Instant::now();
                            let failed = loop {
                                match child.try_wait() {
                                    Ok(Some(status)) => break !status.success(),
                                    Ok(None) if start.elapsed().as_secs() < 20 => {
                                        std::thread::sleep(std::time::Duration::from_millis(50));
                                    }
                                    _ => break false,
                                }
                            };
                            if !failed {
                                return;
                            }
                        }
                        let _com = fx_win::com::init_sta();
                        if let Err(e) = fx_win::shell::open(&path) {
                            out.send(Msg::Error(format!(
                                "Impossible d'ouvrir « {} » : {}",
                                display(&path),
                                e.message().trim()
                            )));
                        }
                    });
                if let Err(e) = spawned {
                    tracing::warn!(error = %e, "thread de lancement");
                }
            }
            Effect::OpenShellItem(parsing) => self.shell.spawn(move || {
                if let Err(e) = fx_win::shell::open_parsing(Path::new(&parsing)) {
                    out.send(Msg::Error(format!(
                        "Impossible d'ouvrir cet appareil : {}",
                        e.message().trim()
                    )));
                }
            }),
            Effect::OpenWith(path) => {
                let owner = self.owner.load(Ordering::Relaxed);
                // Boîte modale : sur un thread temporaire pour ne pas bloquer les icônes.
                spawn_temp("fx-openwith", move || {
                    let _com = fx_win::com::init_sta();
                    let _ = fx_win::shell::open_with((owner != 0).then_some(owner), &path);
                });
            }
            Effect::Properties(path) => {
                let owner = self.owner.load(Ordering::Relaxed);
                self.shell.spawn(move || {
                    if let Err(e) = fx_win::shell::properties((owner != 0).then_some(owner), &path)
                    {
                        out.send(Msg::Error(format!(
                            "Propriétés indisponibles : {}",
                            e.message().trim()
                        )));
                    }
                });
            }
            Effect::RunAsAdmin(path) => {
                let owner = self.owner.load(Ordering::Relaxed);
                self.shell.spawn(move || {
                    let _ = fx_win::shell::run_as_admin((owner != 0).then_some(owner), &path);
                });
            }
            Effect::OpenTerminal(dir) => self.shell.spawn(move || {
                if let Err(e) = fx_win::shell::open_terminal(&dir) {
                    out.send(Msg::Error(format!(
                        "Terminal indisponible : {}",
                        e.message().trim()
                    )));
                }
            }),
            Effect::Job { op, job, cancel } => {
                let owner = self.owner.load(Ordering::Relaxed);
                spawn_temp("fx-op", move || run_job(op, &job, cancel, owner, &out));
            }
            Effect::SetClipboard { paths, cut } => self.shell.spawn(move || {
                if let Err(e) = fx_win::clipboard::set_files(&paths, cut) {
                    out.send(Msg::Error(format!(
                        "Presse-papiers indisponible : {}",
                        e.message().trim()
                    )));
                }
            }),
            Effect::CopyText(text) => self.shell.spawn(move || {
                let _ = fx_win::clipboard::set_text(&text);
            }),
            Effect::CheckClipboard => self
                .shell
                .spawn(move || out.send(Msg::ClipboardState(fx_win::clipboard::has_files()))),
            Effect::ReadClipboard { dest } => self.shell.spawn(move || {
                let (files, cut) = fx_win::clipboard::get_files().unwrap_or_default();
                out.send(Msg::ClipboardFiles { dest, files, cut });
            }),
            Effect::LoadDrives => {
                self.io(move || out.send(Msg::DrivesLoaded(fx_win::sys::drives())))
            }
            Effect::LoadKnownFolders => self.shell.spawn(move || {
                let known = fx_win::sys::KnownFolder::ALL
                    .iter()
                    .filter_map(|&k| fx_win::sys::known_folder(k).map(|p| (k, p)))
                    .collect();
                out.send(Msg::KnownLoaded(known));
            }),
            Effect::LoadRecent => self.io(move || out.send(Msg::RecentLoaded(recent_items()))),
            Effect::LoadPreview(path) => {
                self.io(move || out.send(Msg::PreviewLoaded(fx_preview::build(&path))))
            }
            Effect::DetectGit { pane, dir } => self.io(move || {
                let branch = git_branch(&dir);
                out.send(Msg::GitBranch { pane, dir, branch });
            }),
            Effect::Persist(p) => {
                let (paths, db) = (self.paths.clone(), self.db.clone());
                self.io(move || persist(&paths, &db, p, &out));
            }
            Effect::NewWindow { start, rect } => {
                if let Ok(exe) = std::env::current_exe() {
                    let mut cmd = std::process::Command::new(exe);
                    if let Some([l, t, r, b]) = rect {
                        cmd.arg("--rect").arg(format!("{l},{t},{r},{b}"));
                    }
                    if let Some(d) = start {
                        cmd.arg(d);
                    }
                    if cmd.spawn().is_err() {
                        out.send(Msg::Error(
                            "Impossible d'ouvrir une nouvelle fenêtre".into(),
                        ));
                    }
                }
            }
            Effect::SystemShortcuts { win_e, open_hotkey } => {
                self.shell.spawn(move || {
                    let Ok(exe) = std::env::current_exe() else {
                        return;
                    };
                    // Windows + E ouvre déjà folder : la touche d'ouverture est inutile.
                    let hotkey = if win_e {
                        None
                    } else {
                        open_hotkey.as_ref().and_then(crate::system::launch_hotkey)
                    };
                    let done = fx_win::shortcuts::set_win_e(win_e.then_some(exe.as_path()))
                        .and_then(|()| fx_win::shortcuts::set_launch_hotkey(&exe, hotkey));
                    if let Err(e) = done {
                        out.send(Msg::Error(format!(
                            "Raccourcis système : {}",
                            e.message().trim()
                        )));
                    }
                });
            }
            Effect::EnsureWinE => self.io(move || {
                if fx_win::win_e::is_running() {
                    return;
                }
                let Ok(exe) = std::env::current_exe() else {
                    return;
                };
                if let Err(e) = fx_win::shortcuts::set_win_e(Some(&exe)) {
                    out.send(Msg::Error(format!(
                        "Raccourcis système : {}",
                        e.message().trim()
                    )));
                }
            }),
            Effect::EmptyRecycleBin => {
                let owner = self.owner.load(Ordering::Relaxed);
                self.shell.spawn(move || {
                    if let Err(e) = fx_win::shell::empty_recycle_bin((owner != 0).then_some(owner))
                    {
                        out.send(Msg::Error(format!("Corbeille : {}", e.message().trim())));
                    }
                    out.send(Msg::RecycleBinChanged);
                });
            }
            Effect::LoadRecycleBin { pane, generation } => self.io(move || {
                out.send(Msg::RecycleBinLoaded {
                    pane,
                    generation,
                    items: fx_fs::recycle::list(),
                });
            }),
            Effect::LoadNetwork {
                pane,
                generation,
                parsing,
            } => {
                // Découverte réseau parfois longue : thread STA dédié, jamais le worker shell
                // (icônes) ni le pool I/O.
                // Les appareils arrivent en quelques secondes, la fin de la découverte peut
                // prendre 30 s : chacun est envoyé dès qu'il est trouvé.
                spawn_temp("fx-network", move || {
                    let _com = fx_win::com::init_sta();
                    let o = out.clone();
                    let result = fx_win::shellfolder::for_each_child(&parsing, |c| {
                        o.send(Msg::NetworkLoaded {
                            pane,
                            generation,
                            result: Ok(vec![c]),
                            done: false,
                        });
                    })
                    .map(|()| Vec::new())
                    .map_err(|e| format!("Réseau indisponible : {}", e.message().trim()));
                    out.send(Msg::NetworkLoaded {
                        pane,
                        generation,
                        result,
                        done: true,
                    });
                });
            }
            Effect::LoadWsl => self.io(move || {
                out.send(Msg::WslLoaded(fx_win::sys::wsl_distributions()));
            }),
            Effect::SettingsWindow { notes } => {
                let owner = self.owner.load(Ordering::Relaxed);
                self.io(move || {
                    if fx_win::window::focus_title(crate::SETTINGS_TITLE) {
                        return;
                    }
                    let mut cmd = match std::env::current_exe() {
                        Ok(exe) => std::process::Command::new(exe),
                        Err(e) => {
                            out.send(Msg::Error(format!("Paramètres indisponibles : {e}")));
                            return;
                        }
                    };
                    cmd.arg("--settings");
                    if notes {
                        cmd.arg("--notes");
                    }
                    if owner != 0 {
                        cmd.args(["--owner", &owner.to_string()]);
                    }
                    if let Err(e) = cmd.spawn() {
                        out.send(Msg::Error(format!("Paramètres indisponibles : {e}")));
                    }
                });
            }
            Effect::CheckUpdate { force, delay } => {
                // Thread temporaire : le processus de vérification peut télécharger
                // l'installeur (plusieurs secondes), sans occuper le pool I/O.
                let paths = self.paths.clone();
                spawn_temp("fx-update", move || {
                    let (status, fresh) = crate::upgrade::check(&paths, force, delay);
                    out.send(Msg::UpdateChecked {
                        status,
                        fresh,
                        force,
                    });
                });
            }
            Effect::InstallUpdate => {
                let paths = self.paths.clone();
                self.io(move || match crate::upgrade::install(&paths) {
                    Ok(()) => out.send(Msg::UpdateLaunched),
                    Err(e) => out.send(Msg::Error(format!("Mise à jour impossible : {e}"))),
                });
            }
            Effect::PropertiesWindow { paths, recycled } => {
                let owner = self.owner.load(Ordering::Relaxed);
                self.io(move || {
                    if let Err(e) = properties_window(&paths, recycled.as_ref(), owner) {
                        out.send(Msg::Error(format!("Propriétés indisponibles : {e}")));
                    }
                });
            }
        }
    }
}

fn display(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Exécute un travail long (thread temporaire STA) et rapporte progression et résultat.
fn run_job(
    op: u64,
    job: &Job,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    owner: isize,
    out: &Outbox,
) {
    let _com = fx_win::com::init_sta();
    let result = match job {
        Job::File(fop) => file_job(op, fop, cancel, owner, out),
        Job::Compress { sources, dest } => {
            archive_job(op, &cancel, out, dest, |token, progress| {
                fx_fs::archive::compress(sources, dest, token, progress)
            })
        }
        Job::Extract { archive, dest } => archive_job(op, &cancel, out, dest, |token, progress| {
            fx_fs::archive::extract(archive, dest, token, progress)
        }),
        Job::RestoreRecycled(items) => {
            let mut moves = Vec::new();
            for r in items {
                let (Some(dir), Some(name)) = (r.original.parent(), r.original.file_name()) else {
                    continue;
                };
                // Le dossier d'origine a pu disparaître depuis : on le recrée.
                let _ = std::fs::create_dir_all(dir);
                moves.push((
                    r.item.clone(),
                    dir.to_path_buf(),
                    name.to_string_lossy().into_owned(),
                ));
            }
            let result = file_job(
                op,
                &FileOp::MoveRenamed { items: moves },
                cancel,
                owner,
                out,
            );
            forget_recycled(items);
            result
        }
        Job::PurgeRecycled(items) => {
            let delete = FileOp::Delete {
                items: items.iter().map(|r| r.item.clone()).collect(),
                permanent: true,
            };
            let result = file_job(op, &delete, cancel, owner, out);
            forget_recycled(items);
            result
        }
    };
    out.send(Msg::OpFinished { op, result });
}

/// Lance la fenêtre Propriétés : `folder.exe --properties …` (cf. `fx_ui::props_window`).
fn properties_window(
    paths: &[PathBuf],
    recycled: Option<&(PathBuf, i64)>,
    owner: isize,
) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg("--properties");
    if owner != 0 {
        cmd.args(["--owner", &owner.to_string()]);
    }
    if let Some((original, deleted)) = recycled {
        cmd.arg("--recycled").arg(original).arg(deleted.to_string());
    }
    cmd.arg("--").args(paths);
    cmd.spawn().map(|_| ())
}

fn file_job(
    op: u64,
    fop: &FileOp,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    owner: isize,
    out: &Outbox,
) -> Result<bool, String> {
    let o = out.clone();
    fx_win::fileop::perform(fop, (owner != 0).then_some(owner), cancel, move |event| {
        o.send(Msg::OpEvent { op, event });
    })
    .map(|r| r.aborted)
    .map_err(|e| e.message().trim().to_owned())
}

/// Efface les métadonnées (`$I…`) des éléments qui ont quitté la corbeille (restaurés ou
/// supprimés) ; celles des éléments restés (conflit refusé, annulation) sont conservées.
fn forget_recycled(items: &[fx_fs::recycle::Recycled]) {
    for r in items {
        if fx_win::sys::stat(&r.item).is_none() {
            let _ = std::fs::remove_file(&r.info);
        }
    }
}

fn archive_job(
    op: u64,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
    out: &Outbox,
    created: &Path,
    run: impl FnOnce(&CancelToken, fx_fs::archive::Progress<'_>) -> std::io::Result<()>,
) -> Result<bool, String> {
    // Relais de l'indicateur d'annulation partagé vers un jeton fx-core.
    let token = CancelToken::new();
    let mut last = std::time::Instant::now();
    let mut progress = |done: u64, total: u64, item: &str| {
        if cancel.load(Ordering::Relaxed) {
            token.cancel();
        }
        if last.elapsed().as_millis() >= 50 || done == total {
            last = std::time::Instant::now();
            let scale = |v: u64| (v / 1024).min(u64::from(u32::MAX)) as u32;
            out.send(Msg::OpEvent {
                op,
                event: OpEvent::Progress {
                    done: scale(done),
                    total: scale(total).max(1),
                },
            });
            if !item.is_empty() {
                out.send(Msg::OpEvent {
                    op,
                    event: OpEvent::Item(item.to_owned()),
                });
            }
        }
    };
    match run(&token, &mut progress) {
        Ok(()) => {
            out.send(Msg::OpEvent {
                op,
                event: OpEvent::Created {
                    source: None,
                    created: created.to_path_buf(),
                },
            });
            Ok(false)
        }
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => Ok(true),
        Err(e) => Err(e.to_string()),
    }
}

/// Écritures persistantes.
fn persist(paths: &Paths, db: &Mutex<Option<Db>>, p: Persist, out: &Outbox) {
    let res: Result<(), String> = match p {
        Persist::Settings(s) => fx_store::save_settings(paths, &s).map_err(|e| e.to_string()),
        Persist::Session(s) => fx_store::save_session(paths, &s).map_err(|e| e.to_string()),
        other => {
            let Ok(guard) = db.lock() else { return };
            let Some(db) = guard.as_ref() else { return };
            let send_ops =
                |db: &Db, out: &Outbox| db.op_history().map(|h| out.send(Msg::OpHistoryLoaded(h)));
            match other {
                Persist::Tags { path, tags } => db.set_tags(&path, &tags),
                Persist::Tag(tag) => db
                    .save_tag(&tag)
                    .map(|id| out.send(Msg::TagSaved(fx_store::Tag { id, ..tag }))),
                Persist::DeleteTag(id) => db.delete_tag(id),
                Persist::MovePath { from, to } => db.move_path(&from, &to),
                Persist::FolderPrefs { path, prefs } => db.set_folder_prefs(&path, prefs),
                // Réponse envoyée sous le verrou : dans l'ordre des écritures.
                Persist::AddOp(e) => db.add_op(&e).and_then(|()| send_ops(db, out)),
                Persist::RemoveOp { ended, title } => {
                    db.remove_op(ended, &title).and_then(|()| send_ops(db, out))
                }
                Persist::ClearOps => db.clear_ops().and_then(|()| send_ops(db, out)),
                Persist::LoadOps => send_ops(db, out),
                Persist::Settings(_) | Persist::Session(_) => Ok(()),
            }
            .map_err(|e| e.to_string())
        }
    };
    if let Err(e) = res {
        tracing::warn!(error = %e, "écriture persistante échouée");
    }
}

/// Fichiers récents (dossier Récents de Windows), du plus récent au plus ancien.
fn recent_items() -> Vec<RecentItem> {
    let Some(dir) = fx_win::sys::known_folder(fx_win::sys::KnownFolder::Recent) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    let _ = fx_win::dir::for_each_entry(&dir, |e| {
        let name = String::from_utf16_lossy(e.name);
        if let Some(stem) = name.strip_suffix(".lnk")
            && e.attributes & 0x10 == 0
        {
            items.push(RecentItem {
                name: stem.to_owned(),
                path: dir.join(&name),
                modified: e.modified,
            });
        }
        std::ops::ControlFlow::Continue(())
    });
    items.sort_by_key(|i| std::cmp::Reverse(i.modified));
    items.truncate(24);
    items
}

/// Branche Git du dépôt contenant `dir` (lecture de `.git/HEAD`, sans dépendance Git).
fn git_branch(dir: &Path) -> Option<String> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        let git = d.join(".git");
        let head = if git.is_dir() {
            std::fs::read_to_string(git.join("HEAD")).ok()
        } else if git.is_file() {
            // Sous-module ou worktree : « gitdir: <chemin> ».
            let link = std::fs::read_to_string(&git).ok()?;
            let target = link.strip_prefix("gitdir:")?.trim();
            let target = if Path::new(target).is_absolute() {
                PathBuf::from(target)
            } else {
                d.join(target)
            };
            std::fs::read_to_string(target.join("HEAD")).ok()
        } else {
            None
        };
        if let Some(h) = head {
            let h = h.trim();
            return Some(match h.strip_prefix("ref: refs/heads/") {
                Some(b) => b.to_owned(),
                None => h.chars().take(7).collect(),
            });
        }
        cur = d.parent();
    }
    None
}

fn spawn_worker(name: String, rx: Receiver<Task>) -> std::io::Result<()> {
    thread::Builder::new()
        .name(name)
        // Pile réduite : les workers n'ont pas de récursion profonde.
        .stack_size(512 * 1024)
        .spawn(move || {
            for task in rx {
                // Un panic dans une tâche ne doit pas faire perdre un thread du pool.
                if catch_unwind(AssertUnwindSafe(task)).is_err() {
                    tracing::error!(thread = ?thread::current().name(), "tâche en panique");
                }
            }
        })?;
    Ok(())
}

/// Thread temporaire pour un travail long (opération, recherche, boîte modale).
fn spawn_temp(name: &str, f: impl FnOnce() + Send + 'static) {
    let r = thread::Builder::new()
        .name(name.into())
        .stack_size(1024 * 1024)
        .spawn(move || {
            if catch_unwind(AssertUnwindSafe(f)).is_err() {
                tracing::error!("travail en panique");
            }
        });
    if let Err(e) = r {
        tracing::error!(error = %e, "thread impossible à créer");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_branch_from_head() {
        let d = std::env::temp_dir().join(format!("fx-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".git")).expect("mkdir");
        std::fs::create_dir_all(d.join("src").join("deep")).expect("mkdir");
        std::fs::write(d.join(".git").join("HEAD"), "ref: refs/heads/feature/x\n").expect("w");
        assert_eq!(
            git_branch(&d.join("src").join("deep")).as_deref(),
            Some("feature/x")
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Deux fenêtres (deux processus) sur la même base : l'historique des opérations est
    /// partagé, chaque écriture répond par l'historique à jour.
    #[test]
    fn op_history_is_shared_through_the_db() {
        let d = std::env::temp_dir().join(format!("fx-ops-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let paths = Paths::in_dir(&d);
        let window = || {
            let (tx, rx) = crossbeam_channel::unbounded();
            let db = Db::open(&paths).expect("db");
            let s = Scheduler::new(
                1,
                tx,
                Arc::new(|| {}),
                Arc::new(|_, _| {}),
                paths.clone(),
                Some(db),
            )
            .expect("scheduler");
            (s, rx)
        };
        let loaded = |rx: &crossbeam_channel::Receiver<Msg>| loop {
            match rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(Msg::OpHistoryLoaded(h)) => break h,
                Ok(_) => {}
                Err(e) => panic!("pas de réponse : {e}"),
            }
        };
        let (a, rx_a) = window();
        let (b, rx_b) = window();
        let entry = fx_store::OpEntry {
            ended: 42,
            title: "Copie de « x.txt » vers Docs".into(),
            items: 1,
            outcome: fx_store::OpOutcome::Done,
            duration_ms: 1200,
        };
        a.dispatch(Effect::Persist(Persist::AddOp(entry.clone())));
        assert_eq!(loaded(&rx_a), vec![entry.clone()]);
        b.dispatch(Effect::Persist(Persist::LoadOps));
        assert_eq!(loaded(&rx_b), vec![entry], "visible depuis l'autre fenêtre");
        b.dispatch(Effect::Persist(Persist::ClearOps));
        assert!(loaded(&rx_b).is_empty());
        a.dispatch(Effect::Persist(Persist::LoadOps));
        assert!(loaded(&rx_a).is_empty(), "effacé pour toutes les fenêtres");
        drop((a, b));
        let _ = std::fs::remove_dir_all(&d);
    }
}
