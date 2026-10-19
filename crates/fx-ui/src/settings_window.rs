//! Fenêtre Paramètres : une vraie fenêtre Windows (barre de titre native, déplaçable,
//! redimensionnable), dans un processus `folder.exe --settings` lancé par la fenêtre
//! principale (le moteur CPU ne gère qu'une fenêtre par processus).
//!
//! Chaque modification est appliquée ici (affichage) et envoyée aussitôt à la fenêtre
//! principale par `WM_COPYDATA` (`fx_win::ipc`, `fx_store::remote`) : c'est elle qui
//! l'applique et l'enregistre, seule à écrire les réglages et la base des étiquettes.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Context, Ui, ViewportBuilder, ViewportCommand};
use fx_app::{AppState, Msg, Tags};
use fx_store::remote::Remote;
use fx_store::{Db, Paths};

use crate::Error;
use crate::images::ImageCache;
use crate::theme::{self, Palette};
use crate::view::{Fx, ViewState};

/// Titre de la fenêtre (sert à la retrouver pour la ramener au premier plan).
pub const TITLE: &str = fx_app::SETTINGS_TITLE;

struct SettingsApp {
    state: AppState,
    vs: ViewState,
    images: ImageCache,
    shell: fx_shell::ShellWorker,
    paths: Paths,
    owner: Option<isize>,
    system_accent: Color32,
    shown: bool,
    hwnd: Option<isize>,
    dark: Option<bool>,
    /// Relecture des étiquettes après une création (l'identifiant vient de la base).
    reload_tags_at: Option<Instant>,
}

/// Ouvre la fenêtre Paramètres et rend la main à sa fermeture.
pub fn run(owner: Option<isize>) -> Result<(), Error> {
    let viewport = ViewportBuilder::default()
        .with_title(TITLE)
        .with_app_id("folder-settings")
        .with_inner_size([940.0, 660.0])
        .with_min_inner_size([680.0, 460.0])
        .with_visible(false);
    let config = egui_software_backend::SoftwareBackendAppConfiguration::new()
        .viewport_builder(viewport)
        .caching(false);
    egui_software_backend::set_clear_each_frame(false);
    egui_software_backend::set_frame_pacing(true);
    let mut init_error = None;
    let result =
        egui_software_backend::run_app_with_software_backend(config, |ctx| match SettingsApp::new(
            &ctx, owner,
        ) {
            Ok(app) => Shell(Some(app)),
            Err(e) => {
                init_error = Some(e);
                ctx.send_viewport_cmd(ViewportCommand::Close);
                Shell(None)
            }
        });
    if let Some(e) = init_error {
        return Err(e.into());
    }
    result.map_err(|e| e.to_string().into())
}

/// `None` si l'initialisation a échoué (la fenêtre se ferme aussitôt).
struct Shell(Option<SettingsApp>);

impl egui_software_backend::App for Shell {
    fn ui(&mut self, ui: &mut Ui, _backend: &mut egui_software_backend::SoftwareBackend) {
        if let Some(app) = &mut self.0 {
            app.ui(ui);
        }
    }
}

impl SettingsApp {
    fn new(ctx: &Context, owner: Option<isize>) -> std::io::Result<Self> {
        let paths = Paths::standard();
        let settings = fx_store::load_settings(&paths);
        let system_accent = theme::system_accent();
        let accent = theme::accent_of(&settings, system_accent);
        theme::install(ctx, accent);
        let tags = load_tags(&paths);
        let state = AppState::new(settings, tags, Vec::new());
        // Le cache d'images et le worker shell ne servent pas ici, mais la page en a besoin.
        let (_, rx) = crossbeam_channel::unbounded();
        let shell = fx_shell::ShellWorker::new(std::sync::Arc::new(|_, _| {}))?;
        // La fenêtre principale fermée (ou plantée), les Paramètres se ferment avec elle.
        if let Some(owner) = owner {
            let ctx = ctx.clone();
            let _ = std::thread::Builder::new()
                .name("folder-owner".into())
                .spawn(move || {
                    fx_win::window::wait_owner_exit(owner);
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                    ctx.request_repaint();
                });
        }
        let mut vs = ViewState::new(accent);
        vs.system_accent = system_accent;
        Ok(Self {
            state,
            vs,
            images: ImageCache::new(rx),
            shell,
            paths,
            owner,
            system_accent,
            shown: false,
            hwnd: None,
            dark: None,
            reload_tags_at: None,
        })
    }

    fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let accent = theme::accent_of(&self.state.settings, self.system_accent);
        if accent != self.vs.accent {
            self.vs.accent = accent;
            theme::set_accent(&ctx, accent);
        }
        theme::apply(&ctx, self.state.settings.theme, accent);
        let pal = Palette::new(ctx.theme() == egui::Theme::Dark, accent);
        if self.reload_tags_at.is_some_and(|t| t <= Instant::now()) {
            self.reload_tags_at = None;
            self.state.tags = load_tags(&self.paths);
        } else if let Some(t) = self.reload_tags_at {
            ctx.request_repaint_after(t.saturating_duration_since(Instant::now()));
        }

        let mut f = Fx {
            state: &self.state,
            vs: &mut self.vs,
            images: &mut self.images,
            shell: &self.shell,
            pal,
            msgs: Vec::new(),
            threads: 0,
        };
        theme::apply_scroll_speed(&ctx, self.state.settings.scroll_lines);
        crate::view::settings::page(ui, &mut f);
        let msgs = std::mem::take(&mut f.msgs);
        for m in msgs {
            self.handle(&ctx, m);
        }
        self.show(&ctx, pal.dark);
    }

    /// Applique ici et transmet à la fenêtre principale.
    fn handle(&mut self, ctx: &Context, msg: Msg) {
        let remote = match &msg {
            Msg::UpdateSettings(s) => Some(Remote::Settings(s.clone())),
            Msg::SaveTag(t) => {
                self.reload_tags_at = Some(Instant::now() + Duration::from_millis(400));
                Some(Remote::SaveTag(t.clone()))
            }
            Msg::DeleteTag(id) => Some(Remote::DeleteTag(*id)),
            Msg::CloseOverlays => {
                ctx.send_viewport_cmd(ViewportCommand::Close);
                None
            }
            _ => None,
        };
        let Some(remote) = remote else { return };
        let delivered = self
            .owner
            .is_some_and(|h| fx_win::ipc::send(h, &remote.encode()));
        // Fenêtre principale fermée entre-temps : les réglages sont enregistrés ici.
        if !delivered && let Remote::Settings(s) = &remote {
            let _ = fx_store::save_settings(&self.paths, s);
        }
        // Les effets (écritures) reviennent à la fenêtre principale : ignorés ici.
        let _ = fx_app::update(&mut self.state, msg);
    }

    /// Premier affichage centré sur la fenêtre principale ; barre de titre au thème.
    fn show(&mut self, ctx: &Context, dark: bool) {
        if let Some(h) = self.hwnd
            && self.dark != Some(dark)
        {
            fx_win::window::set_dark_title(h, dark);
            self.dark = Some(dark);
        }
        if self.shown {
            return;
        }
        let Some(hwnd) = fx_win::window::main_window(TITLE) else {
            return;
        };
        self.shown = true;
        self.hwnd = Some(hwnd);
        fx_win::window::set_app_icon(hwnd);
        fx_win::window::set_dark_title(hwnd, dark);
        self.dark = Some(dark);
        if let (Some((ol, ot, or, ob)), Some((l, t, r, b))) = (
            self.owner.and_then(fx_win::window::rect_of),
            fx_win::window::rect_of(hwnd),
        ) {
            let (w, h) = (r - l, b - t);
            let left = (ol + or) / 2 - w / 2;
            let top = ((ot + ob) / 2 - h / 2).max(ot);
            fx_win::window::show_with_placement(
                hwnd,
                &fx_win::window::Placement {
                    left,
                    top,
                    right: left + w,
                    bottom: top + h,
                    maximized: false,
                },
            );
        }
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }
}

fn load_tags(paths: &Paths) -> Tags {
    Db::open(paths).map_or_else(
        |_| Tags::default(),
        |db| {
            Tags::new(
                db.tags().unwrap_or_default(),
                db.tagged().unwrap_or_default(),
            )
        },
    )
}
