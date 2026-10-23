//! Interface egui de fx.
//!
//! Règle : ce module **lit** l'état et **émet** des [`Msg`] ; il ne fait ni I/O ni logique
//! métier. Le rendu est réactif : aucune image n'est produite sans événement.
//!
//! Deux moteurs de rendu sont disponibles derrière le même cœur ([`Core`]) :
//! - [`Renderer::Cpu`] (défaut) : rasterisation logicielle sans cache (egui_software_backend
//!   vendu + softbuffer), aucun pilote GPU chargé dans le processus ;
//! - [`Renderer::Glow`] / `Wgpu` : eframe sur OpenGL ou DirectX 12.

mod bench;
mod images;
mod keys;
pub mod offscreen;
pub mod theme;
mod view;
mod widgets;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
pub use eframe::egui;
use eframe::egui::{Context, ViewportBuilder};
use fx_app::{AppState, Msg, Scheduler, Tags, update};
use fx_store::Paths;

pub use bench::BenchMode;

/// Workers d'I/O : l'énumération est limitée par le disque, pas par le CPU.
const IO_THREADS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    Cpu,
    Glow,
    #[cfg(feature = "wgpu")]
    Wgpu,
}

impl Renderer {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cpu" => Some(Self::Cpu),
            "glow" => Some(Self::Glow),
            #[cfg(feature = "wgpu")]
            "wgpu" => Some(Self::Wgpu),
            _ => None,
        }
    }

    /// `FOLDER_RENDERER`, sinon le rendu CPU (décision de la phase 0, ARCHITECTURE.md §3.1).
    pub fn from_env() -> Self {
        std::env::var("FOLDER_RENDERER")
            .ok()
            .and_then(|s| Self::parse(&s))
            .unwrap_or(Self::Cpu)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Glow => "glow",
            #[cfg(feature = "wgpu")]
            Self::Wgpu => "wgpu",
        }
    }
}

pub struct Options {
    /// Dossier ouvert au démarrage (sinon : session restaurée ou accueil).
    pub start_dir: Option<PathBuf>,
    /// Mode mesure : écrit des lignes `FX_BENCH` sur stdout (cf. `tools/memprobe`).
    pub bench: BenchMode,
    /// Instant le plus proche possible du lancement du processus.
    pub process_start: Instant,
    pub renderer: Renderer,
    /// Taille intérieure visée en pixels physiques (mesures), sinon 1200×780 points.
    pub window_px: Option<[u32; 2]>,
    /// Placement imposé (gauche, haut, droite, bas en pixels physiques) : onglet détaché.
    pub rect: Option<[i32; 4]>,
}

pub type Error = Box<dyn std::error::Error>;

pub mod props_window;
pub mod settings_window;

/// `folder.exe --restore-explorer` (désinstallation) : Windows + E rendu à l'Explorateur et
/// raccourci clavier d'ouverture retiré (cf. `fx_win::shortcuts`).
pub fn restore_explorer() {
    let _com = fx_win::com::init_sta();
    if let Ok(exe) = std::env::current_exe() {
        fx_win::shortcuts::restore(&exe);
    }
}

pub use fx_win::shortcuts::WIN_E_ARG;

/// Processus résident `folder.exe --win-e` (cf. `fx_win::win_e`) : chaque Windows + E ouvre
/// une nouvelle fenêtre folder sur l'accueil, jusqu'à ce que l'option soit désactivée. Sans
/// fenêtre ni egui : quelques Mo. Se termine aussitôt si l'option est désactivée (lancement
/// systématique après l'installation) ou s'il tourne déjà.
pub fn run_win_e() -> bool {
    if !fx_win::shortcuts::win_e_enabled() {
        return true;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let dir = exe.parent().map(std::path::Path::to_path_buf);
    fx_win::win_e::run(|| {
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("folder:home");
        // Pas le dossier courant du processus résident : il ne doit verrouiller aucun dossier.
        if let Some(d) = &dir {
            cmd.current_dir(d);
        }
        if let Ok(child) = cmd.spawn() {
            fx_win::win_e::allow_foreground(child.id());
        }
    })
    .is_ok()
}

/// `folder.exe --stop-win-e` (mise à jour) : arrête le processus résident sans toucher à
/// l'option, pour que l'installeur puisse remplacer l'exécutable.
pub fn stop_win_e() {
    fx_win::win_e::stop();
}

/// Processus auxiliaire `folder.exe --open-media name|modified|type|size asc|desc -- FICHIER` :
/// ouvre une image ou une vidéo depuis une vue du shell triée comme folder, pour que
/// l'application (Photos) la parcoure dans cet ordre, et la sert jusqu'à la fermeture de sa
/// fenêtre (cf. `fx_win::launch`). `false` (aussitôt) si rien n'a pu être lancé.
pub fn open_media(args: &[std::ffi::OsString]) -> bool {
    use fx_win::launch::SortBy;
    let [key, order, sep, path] = args else {
        return false;
    };
    let sort = match key.to_str() {
        Some("name") => SortBy::Name,
        Some("modified") => SortBy::Modified,
        Some("type") => SortBy::Type,
        Some("size") => SortBy::Size,
        _ => return false,
    };
    if sep != "--" {
        return false;
    }
    let _com = fx_win::com::init_sta();
    let res =
        fx_win::launch::open_in_sorted_view(std::path::Path::new(path), sort, order != "desc");
    if let Err(e) = &res {
        tracing::warn!(error = %e, "ouverture depuis une vue triée");
    }
    res.is_ok()
}

/// Titre de la fenêtre principale (sert aussi à retrouver son HWND tant qu'elle est cachée).
pub(crate) const WINDOW_TITLE: &str = "folder";

/// Lance la fenêtre principale ; rend la main à la fermeture.
pub fn run(opts: Options) -> Result<(), Error> {
    let viewport = ViewportBuilder::default()
        .with_title(WINDOW_TITLE)
        .with_app_id("folder")
        .with_inner_size([1200.0, 780.0])
        .with_min_inner_size([640.0, 420.0])
        // Barre de titre maison avec onglets (comme Files) ; le cadre DWM reste natif.
        .with_decorations(false)
        .with_transparent(false)
        // Créée cachée : la première image la place (écran, taille, état agrandi de la
        // dernière fermeture) et l'affiche directement ainsi (cf. `Core::show_window`).
        .with_visible(false);
    match opts.renderer {
        Renderer::Cpu => run_cpu(viewport, opts),
        _ => run_gpu(viewport, opts),
    }
}

// ---------------------------------------------------------------------------------------
// Cœur commun aux moteurs de rendu

struct Core {
    state: AppState,
    scheduler: Scheduler,
    rx: Receiver<Msg>,
    images: images::ImageCache,
    view: view::ViewState,
    bench: Option<bench::Probe>,
    owner_set: bool,
    last_session_save: Instant,
    /// Messages de la fenêtre Paramètres (`WM_COPYDATA`, reçus sur le thread UI).
    remote: (
        crossbeam_channel::Sender<Vec<u8>>,
        crossbeam_channel::Receiver<Vec<u8>>,
    ),
    listening: bool,
    /// Placement de la fenêtre à la dernière fermeture (repli si la fenêtre a disparu).
    placement: Option<fx_store::WindowPlacement>,
    /// La fenêtre, créée cachée, a été placée et affichée.
    shown: bool,
}

impl Core {
    fn new(ctx: &Context, opts: &Options) -> std::io::Result<Self> {
        let paths = Paths::standard();
        let settings = fx_store::load_settings(&paths);
        let session = fx_store::load_session(&paths);
        let db = fx_store::Db::open(&paths)
            .map_err(|e| tracing::warn!(error = %e, "base folder.db indisponible"))
            .ok();
        let tags = db.as_ref().map_or_else(Tags::default, |db| {
            Tags::new(
                db.tags().unwrap_or_default(),
                db.tagged().unwrap_or_default(),
            )
        });
        let prefs = db
            .as_ref()
            .and_then(|db| db.all_folder_prefs().ok())
            .unwrap_or_default();

        let accent = theme::system_accent();
        theme::install(ctx, accent);
        ctx.options_mut(|o| {
            // Ctrl + Plus/Moins et Ctrl + molette changent la taille des icônes (commandes
            // ZoomIn/ZoomOut), pas l'échelle de toute l'interface.
            o.zoom_with_keyboard = false;
            // Même délai de double-clic que Windows : le renommage par clic lent en dépend.
            o.input_options.max_double_click_delay =
                fx_win::sys::double_click_time().clamp(0.2, 2.0);
        });
        let (tx, rx) = crossbeam_channel::unbounded();
        let (img_tx, img_rx) = crossbeam_channel::unbounded();
        let waker_ctx = ctx.clone();
        let img_ctx = ctx.clone();
        let scheduler = Scheduler::new(
            IO_THREADS,
            tx,
            Arc::new(move || waker_ctx.request_repaint()),
            Arc::new(move |key, img| {
                if img_tx.send((key, img)).is_ok() {
                    img_ctx.request_repaint();
                }
            }),
            paths,
            db,
        )?;
        let bench = (opts.bench != BenchMode::Off).then(|| {
            bench::Probe::new(
                opts.bench,
                opts.process_start,
                opts.renderer,
                opts.window_px,
            )
        });
        let mut core = Self {
            state: AppState::new(settings, tags, prefs),
            scheduler,
            rx,
            images: images::ImageCache::new(img_rx),
            view: view::ViewState::new(accent),
            bench,
            owner_set: false,
            last_session_save: Instant::now(),
            // En mode mesure, taille imposée : pas de placement restauré.
            placement: opts
                .rect
                .map(|[left, top, right, bottom]| fx_store::WindowPlacement {
                    left,
                    top,
                    right,
                    bottom,
                    maximized: false,
                })
                .or_else(|| session.as_ref().and_then(|s| s.placement))
                .filter(|_| opts.bench == BenchMode::Off && opts.window_px.is_none()),
            shown: false,
            remote: crossbeam_channel::unbounded(),
            listening: false,
        };
        // En mode mesure, pas de restauration de session : on ouvre le dossier demandé.
        let session = if opts.bench == BenchMode::Off {
            session
        } else {
            None
        };
        core.apply(Msg::Startup {
            start: opts.start_dir.clone(),
            session,
        });
        Ok(core)
    }

    fn apply(&mut self, msg: Msg) {
        for effect in update(&mut self.state, msg) {
            self.scheduler.dispatch(effect);
        }
    }

    /// Applique les résultats des workers arrivés depuis l'image précédente.
    fn pump(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            self.apply(msg);
        }
        while let Ok(bytes) = self.remote.1.try_recv() {
            if let Some(msg) = self.remote_msg(&bytes) {
                self.apply(msg);
            }
        }
        if let Some(msg) = self.bench.as_mut().and_then(|b| b.on_state(&self.state)) {
            self.apply(msg);
        }
    }

    /// `last_frame` : durée de l'image précédente selon le moteur (mesures uniquement).
    fn ui(&mut self, ui: &mut egui::Ui, last_frame: Option<Duration>) {
        if let Some(b) = &mut self.bench {
            b.on_frame();
        }
        let msgs = view::draw(
            ui,
            &self.state,
            &mut self.view,
            &mut self.images,
            self.scheduler.shell(),
            self.scheduler.thread_count(),
        );
        // Les messages de la vue modifient l'état après l'image : en redessiner une pour les
        // montrer (sinon un changement sans événement d'entrée, comme le renommage différé,
        // n'apparaît qu'au prochain mouvement de souris).
        if !msgs.is_empty() {
            ui.ctx().request_repaint();
        }
        for m in msgs {
            self.apply(m);
        }
        self.show_window(ui.ctx());
        self.listen(ui.ctx());
        if !self.owner_set
            && let Some(h) = self.view.hwnd()
        {
            self.scheduler.set_owner_window(h);
            self.owner_set = true;
        }
        // Sauvegarde périodique de la session (en arrière-plan), en plus de celle de la fermeture.
        if self.last_session_save.elapsed() > Duration::from_secs(30) {
            self.last_session_save = Instant::now();
            self.scheduler
                .dispatch(fx_app::Effect::Persist(fx_app::effect::Persist::Session(
                    self.session(),
                )));
        }
        if let Some(b) = &mut self.bench {
            b.after_frame(ui.ctx(), &self.state, &mut self.view, last_frame);
        }
    }

    /// Session à sauvegarder : onglets (état applicatif) et placement de la fenêtre (vue).
    fn session(&self) -> fx_store::Session {
        let current = self
            .view
            .hwnd()
            .and_then(fx_win::window::placement)
            .map(|p| fx_store::WindowPlacement {
                left: p.left,
                top: p.top,
                right: p.right,
                bottom: p.bottom,
                maximized: p.maximized,
            });
        fx_store::Session {
            placement: current.or(self.placement),
            ..self.state.session()
        }
    }

    /// Affiche la fenêtre (créée cachée) dès que son HWND est connu : directement à sa place
    /// de la dernière fermeture si cet écran existe encore, sinon à la taille par défaut.
    fn show_window(&mut self, ctx: &egui::Context) {
        let Some(hwnd) = self.view.hwnd().filter(|_| !self.shown) else {
            return;
        };
        self.shown = true;
        let restored = self.placement.is_some_and(|p| {
            fx_win::window::show_with_placement(
                hwnd,
                &fx_win::window::Placement {
                    left: p.left,
                    top: p.top,
                    right: p.right,
                    bottom: p.bottom,
                    maximized: p.maximized,
                },
            )
        });
        // Toujours signaler l'état réel à winit : il garde sa propre copie (visible,
        // agrandie) et la réapplique à chaque changement d'état. Sans cela, le premier clic
        // sur Réduire ou Restaurer recachait la fenêtre, que winit croyait encore invisible.
        // La fenêtre est déjà affichée à sa place : ces commandes ne la déplacent pas.
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        if restored && self.placement.is_some_and(|p| p.maximized) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
    }

    /// Message de la fenêtre Paramètres. Les dimensions propres à cette fenêtre (barre
    /// latérale, colonnes, épingles) ne sont pas modifiables là-bas : on garde les nôtres.
    fn remote_msg(&mut self, bytes: &[u8]) -> Option<Msg> {
        // Onglet glissé depuis une autre fenêtre : traité par la vue (aperçu, insertion).
        let msg = self
            .view
            .receive_peer(fx_store::remote::Remote::decode(bytes)?)?;
        Some(match msg {
            fx_store::remote::Remote::Settings(mut s) => {
                let cur = &self.state.settings;
                s.sidebar_width = cur.sidebar_width;
                s.preview_width = cur.preview_width;
                s.col_name = cur.col_name;
                s.col_modified = cur.col_modified;
                s.col_type = cur.col_type;
                s.col_size = cur.col_size;
                s.pinned.clone_from(&cur.pinned);
                s.pinned_initialized = cur.pinned_initialized;
                Msg::UpdateSettings(s)
            }
            fx_store::remote::Remote::SaveTag(t) => Msg::SaveTag(t),
            fx_store::remote::Remote::DeleteTag(id) => Msg::DeleteTag(id),
            fx_store::remote::Remote::TabHover { .. }
            | fx_store::remote::Remote::TabHoverEnd
            | fx_store::remote::Remote::TabDrop { .. } => return None,
        })
    }

    /// Écoute la fenêtre Paramètres dès que la fenêtre principale existe.
    fn listen(&mut self, ctx: &egui::Context) {
        if self.listening {
            return;
        }
        let Some(hwnd) = self.view.hwnd() else { return };
        self.listening = true;
        let tx = self.remote.0.clone();
        let ctx = ctx.clone();
        fx_win::ipc::listen(hwnd, move |bytes| {
            let _ = tx.send(bytes);
            ctx.request_repaint();
        });
    }

    fn on_exit(&mut self) {
        // Fenêtre fondue dans une autre : sa session (vide de sens) ne remplace pas la leur.
        if self.bench.is_none() && !self.view.merged_away() {
            self.scheduler.save_session_now(&self.session());
        }
    }
}

// ---------------------------------------------------------------------------------------
// Moteur CPU

/// `None` si l'initialisation a échoué : la fenêtre se ferme aussitôt.
struct CpuApp(Option<Core>);

impl egui_software_backend::App for CpuApp {
    fn ui(&mut self, ui: &mut egui::Ui, backend: &mut egui_software_backend::SoftwareBackend) {
        let Some(core) = &mut self.0 else { return };
        if core.bench.is_some() && !backend.is_capture_frame_time() {
            backend.set_capture_frame_time(true);
        }
        core.pump();
        core.ui(ui, backend.last_frame_time());
    }

    fn on_exit(&mut self, _ctx: &Context) {
        if let Some(core) = &mut self.0 {
            core.on_exit();
        }
    }
}

fn run_cpu(viewport: ViewportBuilder, opts: Options) -> Result<(), Error> {
    // Sans cache de primitives : mesuré plus rapide en défilement (la zone de liste change à
    // chaque image) et ~3x moins de mémoire (pas de canevas intermédiaire plein écran).
    // Cf. ARCHITECTURE.md §3.4 et `tools/renderbench`.
    let config = egui_software_backend::SoftwareBackendAppConfiguration::new()
        .viewport_builder(viewport)
        .caching(false);
    // Les panneaux opaques couvrent toute la fenêtre : inutile d'effacer le tampon à chaque
    // image (piste n° 1 de l'optimisation du rendu CPU, ARCHITECTURE.md §9).
    egui_software_backend::set_clear_each_frame(false);
    // Rendu partiel et défilement par décalage (pistes 2 et 3) : actifs par défaut ;
    // `FOLDER_CPU_FULL_REDRAW=1` revient au rendu complet de chaque image (diagnostic, mesures A/B).
    egui_software_backend::set_partial_redraw(std::env::var_os("FOLDER_CPU_FULL_REDRAW").is_none());
    // Animations cadencées sur la fréquence de l'écran (pas de synchronisation verticale en
    // rendu CPU) ; le banc de défilement mesure au contraire le débit sans plafond.
    egui_software_backend::set_frame_pacing(opts.bench != BenchMode::Scroll);
    // Ctrl+C/X/V comme touches ordinaires : copier, couper, coller des fichiers (sinon
    // egui-winit les change en événements de texte, et Ctrl+V disparaît quand le
    // presse-papiers ne contient que des fichiers). Les champs de texte les retrouvent par
    // `keys::text_clipboard`.
    egui_software_backend::set_clipboard_shortcuts(false);
    let mut init_error = None;
    let result =
        egui_software_backend::run_app_with_software_backend(config, |ctx| {
            match Core::new(&ctx, &opts) {
                Ok(core) => CpuApp(Some(core)),
                Err(e) => {
                    init_error = Some(e);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    CpuApp(None)
                }
            }
        });
    if let Some(e) = init_error {
        return Err(e.into());
    }
    result.map_err(Into::into)
}

// ---------------------------------------------------------------------------------------
// Moteurs GPU (eframe)

struct GpuApp(Core);

impl eframe::App for GpuApp {
    fn logic(&mut self, _ctx: &Context, _frame: &mut eframe::Frame) {
        self.0.pump();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let last = frame.info().cpu_usage.map(Duration::from_secs_f32);
        self.0.ui(ui, last);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.0.on_exit();
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }
}

fn run_gpu(viewport: ViewportBuilder, opts: Options) -> Result<(), Error> {
    let renderer = match opts.renderer {
        #[cfg(feature = "wgpu")]
        Renderer::Wgpu => eframe::Renderer::Wgpu,
        _ => eframe::Renderer::Glow,
    };
    let mut native = eframe::NativeOptions {
        viewport,
        renderer,
        // Pas de MSAA, ni profondeur, ni stencil : egui n'en a pas besoin (mémoire GPU en moins).
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        system_font_fallback: false,
        persist_window: false,
        ..Default::default()
    };
    // Mesure de défilement : ne pas plafonner à la fréquence de l'écran.
    native.glow_options.vsync = opts.bench != BenchMode::Scroll;
    eframe::run_native(
        "folder",
        native,
        Box::new(move |cc| Ok(Box::new(GpuApp(Core::new(&cc.egui_ctx, &opts)?)))),
    )
    .map_err(|e| e.to_string().into())
}
