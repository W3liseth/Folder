use crate::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use egui::{
    Context, CursorGrab, IconData, Pos2, SystemTheme, Vec2, ViewportBuilder, ViewportCommand,
    WindowLevel, X11WindowType,
};
use softbuffer::SoftBufferError;
use std::boxed::Box;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::mem;
use std::num::NonZeroU32;
use std::ops::Deref;
use std::rc::Rc;
use std::string::String;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::vec::Vec;
use winit::application::ApplicationHandler;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::window::{CursorGrabMode, Fullscreen, Icon, Theme, Window, WindowButtons, WindowId};

/// Errors that can occur when using the egui software backend with winit.
#[derive(Debug)]
pub enum SoftwareBackendAppError {
    /// A softbuffer error has occurred.
    /// The softbuffer crate is used to manage the pixel buffer of the window.
    SoftBuffer {
        soft_buffer_error: Box<dyn Error>,
        function: &'static str,
    },

    /// Some event loop error has occurred.
    EventLoop(Box<dyn Error>),

    /// The event loop has errored in addition to an error from the software renderer
    SuppressedEventLoop {
        event_loop_error: Box<dyn Error>,
        suppressed: Box<SoftwareBackendAppError>,
    },

    /// Error when calling winit create_window
    CreateWindowOs(Box<dyn Error>),
}

impl Display for SoftwareBackendAppError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            SoftwareBackendAppError::SoftBuffer { function, .. } => {
                f.write_str("error calling ")?;
                f.write_str(function)
            }
            SoftwareBackendAppError::EventLoop(_) => f.write_str("winit event loop has errored"),
            SoftwareBackendAppError::SuppressedEventLoop { .. } => {
                f.write_str("software renderer and winit event loop have both errored")
            }
            SoftwareBackendAppError::CreateWindowOs(_) => {
                f.write_str("os error calling winit::create_window")
            }
        }
    }
}

impl Error for SoftwareBackendAppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            SoftwareBackendAppError::SuppressedEventLoop { suppressed, .. } => {
                Some(suppressed as &dyn Error)
            }
            _ => None,
        }
    }
}

impl SoftwareBackendAppError {
    fn soft_buffer(
        function: &'static str,
    ) -> impl FnOnce(SoftBufferError) -> SoftwareBackendAppError {
        move |error| Self::SoftBuffer {
            soft_buffer_error: Box::new(error),
            function,
        }
    }
}

// Doing what clippy suggests would make it impossible for the compiler to optimize the layout of this enum, causing
// state machine transitions to be more expensive!
#[allow(clippy::large_enum_variant)]
/// Winit App State machine, that handles all states of our application.
enum WinitAppStateMachine<EguiApp: App, EguiAppFactory: FnMut(Context) -> EguiApp> {
    /// The app has died, either without or with some sort of error.
    Dead(Option<SoftwareBackendAppError>),

    /// The app is configured, but the window has not been created yet.
    /// Transitions into WindowInitialized (resume).
    Configured(ConfiguredAppState<EguiApp, EguiAppFactory>),

    /// The window has been initialized.
    /// Transitions into Dead (error) or Running (resume).
    WindowInitialized(WindowInitializedAppState<EguiApp, EguiAppFactory>),

    /// The app is running.
    /// Transitions into Dead (quit/error) or WindowInitialized (suspend)
    Running(RunningEguiAppState<EguiApp, EguiAppFactory>),
}

struct ConfiguredAppState<EguiApp: App, EguiAppFactory: FnMut(Context) -> EguiApp> {
    /////////// DANGER ZONE DO NOT CHANGE THE ORDER OF THOSE FIELDS ////////////////////
    // WAYLAND BUG: The wayland clipboard blows up with a segmentation fault if
    // If the fields are dropped in the wrong order. Other platforms are not affected by drop order.
    // Fields of a struct are dropped in declaration order. https://doc.rust-lang.org/reference/destructors.html
    egui_context: Context,
    softbuffer_context: softbuffer::Context<OwnedDisplayHandle>,
    /////////////////// END OF DANGER ZONE//////////////////////////////////////
    config: SoftwareBackendAppConfiguration,
    software_backend: SoftwareBackend,
    renderer: EguiSoftwareRender,
    egui_app_factory: EguiAppFactory,
}

struct WindowInitializedAppState<EguiApp: App, EguiAppFactory: FnMut(Context) -> EguiApp> {
    /////////// DANGER ZONE DO NOT CHANGE THE ORDER OF THOSE FIELDS ////////////////////
    // WAYLAND BUG: The wayland clipboard blows up with a segmentation fault if
    // If the fields are dropped in the wrong order. Other platforms are not affected by drop order.
    // Fields of a struct are dropped in declaration order. https://doc.rust-lang.org/reference/destructors.html
    egui_context: Context,
    softbuffer_context: softbuffer::Context<OwnedDisplayHandle>,
    window: Rc<Window>,
    /////////////////// END OF DANGER ZONE//////////////////////////////////////
    config: SoftwareBackendAppConfiguration,
    software_backend: SoftwareBackend,
    renderer: EguiSoftwareRender,
    egui_app_factory: EguiAppFactory,
}

struct RunningEguiAppState<EguiApp: App, EguiAppFactory: FnMut(Context) -> EguiApp> {
    /////////// DANGER ZONE DO NOT CHANGE THE ORDER OF THOSE FIELDS ////////////////////
    // WAYLAND BUG: The wayland clipboard blows up with a segmentation fault if
    // If the fields are dropped in the wrong order. Other platforms are not affected by drop order.
    // Fields of a struct are dropped in declaration order. https://doc.rust-lang.org/reference/destructors.html
    egui_context: Context,
    surface: softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>,
    egui_winit: egui_winit::State,
    window: Rc<Window>,
    /////////////////// END OF DANGER ZONE//////////////////////////////////////
    config: SoftwareBackendAppConfiguration,
    software_backend: SoftwareBackend,
    renderer: EguiSoftwareRender,
    egui_app_factory: EguiAppFactory,
    softbuffer_context: softbuffer::Context<OwnedDisplayHandle>,
    egui_app: EguiApp,
    fullscreen: bool,
    visible: bool,
    input_events: Vec<egui::Event>,
    /// Portage fx : la prochaine image est redessinée et présentée entièrement (la fenêtre a
    /// pu perdre son contenu : redimensionnement, restauration, changement d'échelle).
    full_redraw: bool,
    /// Portage fx : la prochaine image est présentée entièrement (sans être redessinée). GDI
    /// ne recopie pas les parties de la fenêtre hors de l'écran : après un déplacement, elles
    /// peuvent réapparaître périmées alors que le tampon, lui, est à jour.
    full_present: bool,
    /// Portage fx : rectangles passés à `present_with_damage` (réutilisés).
    damage_rects: Vec<softbuffer::Rect>,
    /// Portage fx : image demandée pour plus tard (`request_repaint_after`). L'original
    /// redessinait tout de suite, ce qui faisait tourner la boucle à vide jusqu'à l'échéance.
    next_repaint: Option<Instant>,
    /// Portage fx : début de la dernière image et intervalle de rafraîchissement de l'écran,
    /// pour cadencer les images demandées par egui (animations) sans synchronisation verticale.
    last_frame: Option<Instant>,
    frame_interval: Duration,
    /// Portage fx : `App::on_exit` déjà appelé (fermeture native ou `ViewportCommand::Close`).
    exited: bool,
}

impl<EguiApp: App, EguiAppFactory: FnMut(Context) -> EguiApp> Default
    for WinitAppStateMachine<EguiApp, EguiAppFactory>
{
    fn default() -> Self {
        Self::Dead(None)
    }
}

impl<EguiApp: App, EguiAppFactory: FnMut(Context) -> EguiApp>
    ConfiguredAppState<EguiApp, EguiAppFactory>
{
    pub(crate) fn create_window(
        mut self,
        elwt: &ActiveEventLoop,
    ) -> Result<WindowInitializedAppState<EguiApp, EguiAppFactory>, SoftwareBackendAppError> {
        // !BUG IN WAYLAND!
        // If resizable is false during window creation you can never make the window resizable again.
        // We always force None before we call into egui_winit and set it on the window object later.
        let resizable = self
            .config
            .viewport_builder
            .resizable
            .take()
            .unwrap_or(true);

        let window =
            egui_winit::create_window(&self.egui_context, elwt, &self.config.viewport_builder);

        self.config.viewport_builder.resizable = Some(resizable);

        let window = window
            .map_err(|ose| SoftwareBackendAppError::CreateWindowOs(Box::new(ose)))
            .map(Rc::new)?;

        window.set_resizable(resizable);

        Ok(WindowInitializedAppState {
            config: self.config,
            software_backend: self.software_backend,
            renderer: self.renderer,
            egui_context: self.egui_context,
            egui_app_factory: self.egui_app_factory,
            softbuffer_context: self.softbuffer_context,
            window,
        })
    }
}

impl<EguiApp: App, EguiAppFactory: FnMut(Context) -> EguiApp>
    WindowInitializedAppState<EguiApp, EguiAppFactory>
{
    pub(crate) fn create_surface(
        mut self,
    ) -> Result<RunningEguiAppState<EguiApp, EguiAppFactory>, SoftwareBackendAppError> {
        let surface = softbuffer::Surface::new(&self.softbuffer_context, self.window.clone())
            .map_err(SoftwareBackendAppError::soft_buffer(
                "softbuffer::Surface::new",
            ))?;

        let mut egui_winit = egui_winit::State::new(
            self.egui_context.clone(),
            egui::ViewportId::ROOT,
            &self.window,
            Some(self.window.scale_factor() as f32),
            None,
            None,
        );
        // Portage fx : cf. `set_clipboard_shortcuts`.
        egui_winit.set_clipboard_shortcuts(
            CLIPBOARD_SHORTCUTS.load(core::sync::atomic::Ordering::Relaxed),
        );

        let egui_app = (self.egui_app_factory)(self.egui_context.clone());
        let fullscreen = self.config.viewport_builder.fullscreen.unwrap_or_default();
        let visible = self.config.viewport_builder.visible.unwrap_or(true);

        Ok(RunningEguiAppState {
            config: self.config,
            renderer: self.renderer,
            egui_context: self.egui_context,
            egui_app_factory: self.egui_app_factory,
            softbuffer_context: self.softbuffer_context,
            window: self.window,
            surface,
            egui_winit,
            egui_app,
            fullscreen,
            visible,
            input_events: Vec::new(),
            full_redraw: true,
            full_present: true,
            damage_rects: Vec::new(),
            next_repaint: None,
            last_frame: None,
            frame_interval: DEFAULT_FRAME_INTERVAL,
            exited: false,
            software_backend: self.software_backend,
        })
    }
}

impl<EguiApp: App, EguiAppFactory: FnMut(Context) -> EguiApp>
    WinitAppStateMachine<EguiApp, EguiAppFactory>
{
    /// Create a new application.
    pub(crate) fn new(
        config: SoftwareBackendAppConfiguration,
        renderer: EguiSoftwareRender,
        softbuffer_context: softbuffer::Context<OwnedDisplayHandle>,
        egui_app_factory: EguiAppFactory,
        egui_context: Context,
    ) -> Self {
        Self::Configured(ConfiguredAppState {
            config,
            software_backend: SoftwareBackend {
                capture_frame_time: false,
                last_frame_time: None,
            },
            renderer,
            softbuffer_context,
            egui_context,
            egui_app_factory,
        })
    }
}

enum UserEvent {
    RequestRepaint {
        /// What to repaint.
        viewport_id: egui::ViewportId,

        /// When to repaint.
        when: Instant,

        /// What the cumulative pass number was when the repaint was _requested_.
        cumulative_pass_nr: u64,
    },
}

impl<EguiApp: App, InitSurface: FnMut(Context) -> EguiApp> ApplicationHandler<UserEvent>
    for WinitAppStateMachine<EguiApp, InitSurface>
{
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if el.exiting() {
            return;
        }

        match mem::take(self) {
            Self::Configured(state) => match state.create_window(el) {
                Ok(ss) => {
                    *self = Self::WindowInitialized(ss);
                    self.resumed(el);
                }
                Err(e) => {
                    *self = Self::Dead(Some(e));
                    el.exit();
                }
            },
            Self::WindowInitialized(state) => match state.create_surface() {
                Ok(mut ss) => {
                    // Portage fx : une fenêtre créée cachée ne reçoit pas de WM_PAINT ; sa
                    // première image est produite ici, pour que l'application puisse la placer
                    // puis l'afficher directement dans son état final.
                    if !ss.window.is_visible().unwrap_or(true) {
                        let window_id = ss.window.id();
                        if let Err(e) = ss.handle_event(
                            Event::WindowEvent {
                                window_id,
                                event: WindowEvent::RedrawRequested,
                            },
                            el,
                        ) {
                            *self = Self::Dead(Some(e));
                            el.exit();
                            return;
                        }
                    }
                    *self = Self::Running(ss);
                }
                Err(e) => {
                    *self = Self::Dead(Some(e));
                    el.exit();
                }
            },
            Self::Running(state) => {
                *self = Self::Running(state);
                self.suspended(el);
                self.resumed(el);
            }
            Self::Dead(err) => {
                *self = Self::Dead(err);
                el.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        if event_loop.exiting() {
            return;
        }
        if let Self::Running(state) = self {
            match event {
                UserEvent::RequestRepaint {
                    viewport_id,
                    when,
                    cumulative_pass_nr,
                } => {
                    let current_pass_nr = state.egui_context.cumulative_pass_nr_for(viewport_id);
                    if current_pass_nr == cumulative_pass_nr
                        || current_pass_nr == cumulative_pass_nr + 1
                    {
                        // Portage fx : n'avancer l'image que si l'échéance est atteinte ;
                        // sinon la retenir (cf. `next_repaint` et `handle_event`).
                        // Cadence : pas plus d'une image par rafraîchissement de l'écran.
                        let when = match state.last_frame {
                            Some(last)
                                if FRAME_PACING.load(core::sync::atomic::Ordering::Relaxed) =>
                            {
                                when.max(last + state.frame_interval)
                            }
                            _ => when,
                        };
                        if when <= Instant::now() {
                            state.request_redraw();
                        } else {
                            state.next_repaint =
                                Some(state.next_repaint.map_or(when, |t| t.min(when)));
                            event_loop.set_control_flow(ControlFlow::WaitUntil(when));
                        }
                    }
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if event_loop.exiting() {
            return;
        }

        match self {
            Self::Configured(_) | Self::WindowInitialized(_) => {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Self::Running(state) => {
                if let Err(e) =
                    state.handle_event(Event::WindowEvent { window_id, event }, event_loop)
                {
                    *self = Self::Dead(Some(e));
                    event_loop.exit();
                }
            }
            Self::Dead(_) => {
                event_loop.exit();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if event_loop.exiting() {
            return;
        }

        match self {
            Self::Configured(_) | Self::WindowInitialized(_) => {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Self::Running(state) => {
                if let Err(e) = state.handle_event(Event::AboutToWait, event_loop) {
                    *self = Self::Dead(Some(e));
                    event_loop.exit();
                }
            }
            Self::Dead(_) => {
                event_loop.exit();
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // Portage fx : sortie par `ViewportCommand::Close` (cf. `finish`).
        if let Self::Running(state) = self {
            state.finish();
        }
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        if event_loop.exiting() {
            return;
        }

        match mem::take(self) {
            Self::Dead(e) => {
                *self = Self::Dead(e);
                event_loop.exit();
            }
            Self::Configured(state) => {
                *self = Self::Configured(state);
            }
            Self::WindowInitialized(window) => {
                *self = Self::WindowInitialized(window);
            }
            Self::Running(state) => {
                *self = Self::WindowInitialized(state.suspend());
            }
        }
    }
}

impl<EguiApp: App, EguiAppFactory: FnMut(Context) -> EguiApp>
    RunningEguiAppState<EguiApp, EguiAppFactory>
{
    pub(crate) fn suspend(self) -> WindowInitializedAppState<EguiApp, EguiAppFactory> {
        WindowInitializedAppState {
            config: self.config,
            software_backend: self.software_backend,
            renderer: self.renderer,
            egui_context: self.egui_context,
            egui_app_factory: self.egui_app_factory,
            softbuffer_context: self.softbuffer_context,
            window: self.window,
        }
    }
    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// Portage fx : prévient l'application une seule fois, quelle que soit la façon dont la
    /// fenêtre se ferme. L'original ne le faisait pas pour `ViewportCommand::Close` (bouton de
    /// fermeture d'une barre de titre maison) : l'application quittait sans rien sauvegarder.
    fn finish(&mut self) {
        if !self.exited {
            self.exited = true;
            self.egui_app.on_exit(&self.egui_context);
        }
    }

    pub(crate) fn handle_event(
        &mut self,
        event: Event<()>,
        elwt: &ActiveEventLoop,
    ) -> Result<(), SoftwareBackendAppError> {
        let start = if self.software_backend.capture_frame_time {
            Some(Instant::now())
        } else {
            None
        };

        // Portage fx : réveil à l'échéance d'une image différée, sinon attente pure.
        match self.next_repaint {
            Some(t) if t <= Instant::now() => {
                self.next_repaint = None;
                self.window.request_redraw();
                elwt.set_control_flow(ControlFlow::Wait);
            }
            Some(t) => elwt.set_control_flow(ControlFlow::WaitUntil(t)),
            None => elwt.set_control_flow(ControlFlow::Wait),
        }

        let Event::WindowEvent {
            window_id,
            event: window_event,
        } = event
        else {
            return Ok(());
        };

        if window_id != self.window.id() {
            return Ok(());
        }

        match window_event {
            WindowEvent::RedrawRequested => {
                // Portage fx : cette image honore toute demande en attente ; egui
                // redemandera pendant l'image ce dont il a encore besoin.
                self.next_repaint = None;
                self.last_frame = Some(Instant::now());
                let size = self.window.inner_size();
                let width = NonZeroU32::new(size.width).unwrap_or(ONE_PIXEL);
                let height = NonZeroU32::new(size.height).unwrap_or(ONE_PIXEL);

                self.surface.resize(width, height).map_err(
                    SoftwareBackendAppError::soft_buffer("softbuffer::Surface::resize"),
                )?;

                let mut raw_input = self.egui_winit.take_egui_input(self.window.deref());
                // Portage fx : état de la fenêtre (agrandie, focus…) pour la barre de titre maison.
                let viewport_id = raw_input.viewport_id;
                egui_winit::update_viewport_info(
                    raw_input.viewports.entry(viewport_id).or_default(),
                    &self.egui_context,
                    self.window.deref(),
                    false,
                );

                raw_input
                    .events
                    .extend_from_slice(self.input_events.as_slice());
                self.input_events.clear();

                let mut full_output = self.egui_context.run_ui(raw_input, |ui| {
                    self.egui_app.ui(ui, &mut self.software_backend);

                    self.egui_context.viewport(|r| {
                        let mut die = false;
                        for command in &r.commands {
                            match command {
                                ViewportCommand::Close => {
                                    die = true;
                                }
                                ViewportCommand::CancelClose => {
                                    die = false;
                                }
                                ViewportCommand::Title(title) => self.window.set_title(title),
                                ViewportCommand::Transparent(trans) => {
                                    self.window.set_transparent(*trans)
                                }
                                ViewportCommand::Visible(true) => {
                                    self.visible = true;
                                    self.window.set_visible(true);
                                    if self.fullscreen {
                                        self.window
                                            .set_fullscreen(Some(Fullscreen::Borderless(None)));
                                    } else {
                                        self.window.set_fullscreen(None);
                                    }
                                }
                                ViewportCommand::Visible(false) => {
                                    self.visible = false;
                                    // Needed because otherwise fullscreen mode never works again.
                                    self.window.set_fullscreen(None);

                                    // Needed because otherwise cursor grab needs to be manually set to none before it
                                    // can be enabled again
                                    _ = self.window.set_cursor_grab(CursorGrabMode::None);

                                    self.window.set_visible(false)
                                }
                                ViewportCommand::OuterPosition(state) => {
                                    self.window.set_outer_position(
                                        winit::dpi::LogicalPosition::new(state.x, state.y),
                                    );
                                }
                                ViewportCommand::InnerSize(state) => {
                                    _ = self.window.request_inner_size(
                                        winit::dpi::LogicalSize::new(state.x, state.y),
                                    );
                                }
                                ViewportCommand::MinInnerSize(state) => {
                                    self.window.set_min_inner_size(Some(
                                        winit::dpi::LogicalSize::new(state.x, state.y),
                                    ));
                                }
                                ViewportCommand::MaxInnerSize(state) => {
                                    self.window.set_max_inner_size(Some(
                                        winit::dpi::LogicalSize::new(state.x, state.y),
                                    ));
                                }
                                ViewportCommand::ResizeIncrements(None) => {
                                    self.window
                                        .set_resize_increments::<winit::dpi::LogicalSize<f32>>(
                                            None,
                                        );
                                }
                                ViewportCommand::ResizeIncrements(Some(state)) => {
                                    let size = winit::dpi::LogicalSize::new(state.x, state.y);
                                    self.window.set_resize_increments(Some(size));
                                }
                                ViewportCommand::Resizable(state) => {
                                    self.window.set_resizable(*state);
                                }
                                ViewportCommand::EnableButtons {
                                    close,
                                    maximize,
                                    minimized,
                                } => {
                                    let mut button_state = WindowButtons::empty();
                                    if *close {
                                        button_state.set(WindowButtons::CLOSE, true);
                                    }
                                    if *maximize {
                                        button_state.set(WindowButtons::MAXIMIZE, true);
                                    }
                                    if *minimized {
                                        button_state.set(WindowButtons::MINIMIZE, true);
                                    }

                                    self.window.set_enabled_buttons(button_state);
                                }
                                ViewportCommand::Minimized(state) => {
                                    self.window.set_minimized(*state);
                                }
                                ViewportCommand::Maximized(state) => {
                                    self.window.set_maximized(*state);
                                }
                                ViewportCommand::Fullscreen(false) => {
                                    self.fullscreen = false;
                                    if self.visible {
                                        self.window.set_fullscreen(None)
                                    }
                                }
                                ViewportCommand::Fullscreen(true) => {
                                    self.fullscreen = true;
                                    if self.visible {
                                        self.window
                                            .set_fullscreen(Some(Fullscreen::Borderless(None)))
                                    }
                                }
                                ViewportCommand::Decorations(dec) => {
                                    self.window.set_decorations(*dec)
                                }
                                ViewportCommand::WindowLevel(WindowLevel::Normal) => {
                                    self.window
                                        .set_window_level(winit::window::WindowLevel::Normal);
                                }
                                ViewportCommand::WindowLevel(WindowLevel::AlwaysOnBottom) => {
                                    self.window.set_window_level(
                                        winit::window::WindowLevel::AlwaysOnBottom,
                                    );
                                }
                                ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop) => {
                                    self.window
                                        .set_window_level(winit::window::WindowLevel::AlwaysOnTop);
                                }
                                ViewportCommand::Icon(ico) => {
                                    self.window.set_window_icon(
                                        ico.as_ref()
                                            .map(|i| {
                                                Icon::from_rgba(i.rgba.clone(), i.width, i.height)
                                                    .ok()
                                            })
                                            .unwrap_or(None),
                                    );
                                }
                                ViewportCommand::Focus => self.window.focus_window(),
                                ViewportCommand::SetTheme(SystemTheme::Dark) => {
                                    self.window.set_theme(Some(Theme::Dark))
                                }
                                ViewportCommand::SetTheme(SystemTheme::Light) => {
                                    self.window.set_theme(Some(Theme::Light))
                                }
                                ViewportCommand::SetTheme(SystemTheme::SystemDefault) => {
                                    // Winit has no default...
                                    // we will just use light as that is the default
                                    // of most operating systems.
                                    self.window.set_theme(Some(Theme::Light))
                                }

                                //Wtf, I don't think DXGI, or reading the X11 backbuffer cares about this.
                                ViewportCommand::ContentProtected(x) => {
                                    self.window.set_content_protected(*x);
                                }

                                ViewportCommand::CursorGrab(CursorGrab::Confined) => {
                                    _ = self.window.set_cursor_grab(CursorGrabMode::Confined);
                                }
                                ViewportCommand::CursorGrab(CursorGrab::Locked) => {
                                    _ = self.window.set_cursor_grab(CursorGrabMode::Locked);
                                }
                                ViewportCommand::CursorGrab(CursorGrab::None) => {
                                    _ = self.window.set_cursor_grab(CursorGrabMode::None);
                                }
                                ViewportCommand::CursorVisible(visible) => {
                                    self.window.set_cursor_visible(*visible);
                                }
                                ViewportCommand::RequestCut => {
                                    self.input_events.push(egui::Event::Cut);
                                }
                                ViewportCommand::RequestCopy => {
                                    self.input_events.push(egui::Event::Copy);
                                }
                                ViewportCommand::RequestPaste => {
                                    if let Some(content) = self.egui_winit.clipboard_text() {
                                        self.input_events.push(egui::Event::Paste(content));
                                    }
                                }
                                ViewportCommand::MousePassthrough(_) => {
                                    //UNSUPPORTED
                                }
                                ViewportCommand::Screenshot(_) => {
                                    //UNSUPPORTED (YET)
                                }
                                // Portage fx : fenêtres sans décorations (barre de titre maison).
                                ViewportCommand::BeginResize(dir) => {
                                    use winit::window::ResizeDirection as R;
                                    let d = match dir {
                                        egui::viewport::ResizeDirection::North => R::North,
                                        egui::viewport::ResizeDirection::South => R::South,
                                        egui::viewport::ResizeDirection::East => R::East,
                                        egui::viewport::ResizeDirection::West => R::West,
                                        egui::viewport::ResizeDirection::NorthEast => R::NorthEast,
                                        egui::viewport::ResizeDirection::SouthEast => R::SouthEast,
                                        egui::viewport::ResizeDirection::NorthWest => R::NorthWest,
                                        egui::viewport::ResizeDirection::SouthWest => R::SouthWest,
                                    };
                                    _ = self.window.drag_resize_window(d);
                                }
                                ViewportCommand::IMERect(rect) => {
                                    let ppp = self.egui_context.pixels_per_point();
                                    self.window.set_ime_cursor_area(
                                        winit::dpi::PhysicalPosition::new(
                                            rect.min.x * ppp,
                                            rect.min.y * ppp,
                                        ),
                                        winit::dpi::PhysicalSize::new(
                                            rect.width() * ppp,
                                            rect.height() * ppp,
                                        ),
                                    );
                                }
                                ViewportCommand::IMEAllowed(allowed) => {
                                    self.window.set_ime_allowed(*allowed);
                                }
                                ViewportCommand::IMEPurpose(_) => {
                                    //UNSUPPORTED
                                }
                                ViewportCommand::CursorPosition(_) => {
                                    //UNSUPPORTED
                                }
                                ViewportCommand::RequestUserAttention(_) => {
                                    //UNSUPPORTED
                                }
                                ViewportCommand::StartDrag => {
                                    _ = self.window.drag_window();
                                }
                                // Ajoutés par egui 0.37 (portage fx).
                                ViewportCommand::SetMonitor(_)
                                | ViewportCommand::RequestPaintWhileHidden => {
                                    //UNSUPPORTED
                                }
                            }

                            if die {
                                self.window.set_visible(false); //I wish eframe did this.
                                elwt.exit();
                            }
                        }
                    });
                });

                //Makes the clipboard work.
                self.egui_winit
                    .handle_platform_output(self.window.deref(), full_output.platform_output);

                let clipped_primitives = self
                    .egui_context
                    .tessellate(full_output.shapes, full_output.pixels_per_point);

                let mut buffer =
                    self.surface
                        .buffer_mut()
                        .map_err(SoftwareBackendAppError::soft_buffer(
                            "softbuffer::Surface::buffer_mut",
                        ))?;
                // Portage fx : effacement optionnel (inutile si l'UI couvre toute la fenêtre).
                let clear = CLEAR_EACH_FRAME.load(core::sync::atomic::Ordering::Relaxed);

                if self.config.caching
                    || !PARTIAL_REDRAW.load(core::sync::atomic::Ordering::Relaxed)
                {
                    // Portage fx : l'image précédente du rendu partiel n'est plus à jour.
                    self.renderer.invalidate();
                    if clear {
                        buffer.fill(0); // CLEAR
                    }

                    let buffer_ref = &mut BufferMutRef::new(
                        bytemuck::cast_slice_mut(&mut buffer),
                        width.get() as usize,
                        height.get() as usize,
                    );

                    self.renderer.render(
                        buffer_ref,
                        &clipped_primitives,
                        &full_output.textures_delta,
                        full_output.pixels_per_point,
                    );

                    buffer
                        .present()
                        .map_err(SoftwareBackendAppError::soft_buffer(
                            "softbuffer::Buffer::present",
                        ))?;
                } else {
                    // Portage fx : rendu partiel. Le tampon de softbuffer garde l'image
                    // précédente (`age() == 1`) : on ne rasterise et ne présente que ce qui a
                    // changé, et on recopie ce qui a défilé (cf. damage.rs).
                    let reuse = buffer.age() == 1 && !mem::take(&mut self.full_redraw);
                    let buffer_ref = &mut BufferMutRef::new(
                        bytemuck::cast_slice_mut(&mut buffer),
                        width.get() as usize,
                        height.get() as usize,
                    );
                    let damage = self.renderer.render_damaged(
                        buffer_ref,
                        &clipped_primitives,
                        &full_output.textures_delta,
                        full_output.pixels_per_point,
                        reuse,
                        clear,
                    );
                    self.damage_rects.clear();
                    self.damage_rects.extend(damage.iter().filter_map(|r| {
                        Some(softbuffer::Rect {
                            x: u32::try_from(r.x0).ok()?,
                            y: u32::try_from(r.y0).ok()?,
                            width: NonZeroU32::new(u32::try_from(r.width()).ok()?)?,
                            height: NonZeroU32::new(u32::try_from(r.height()).ok()?)?,
                        })
                    }));
                    let full_present = mem::take(&mut self.full_present);
                    let presented = if full_present || self.renderer.last_damage().full {
                        buffer.present()
                    } else {
                        // Même vide : valide la fenêtre (sinon Windows renverrait WM_PAINT).
                        buffer.present_with_damage(&self.damage_rects)
                    };
                    presented.map_err(SoftwareBackendAppError::soft_buffer(
                        "softbuffer::Buffer::present",
                    ))?;
                }

                // Portage fx (egui 0.37) : un `TexturesDelta` non vidé déclenche une assertion
                // en debug ; il vient d'être appliqué par le moteur.
                full_output.textures_delta.clear();

                self.software_backend.last_frame_time = start.map(|a| a.elapsed());
            }

            WindowEvent::CloseRequested => {
                self.finish();
                elwt.exit();
            }
            _ => {
                // Portage fx : contenu de la fenêtre peut-être perdu, tout représenter.
                if matches!(
                    window_event,
                    WindowEvent::Resized(_)
                        | WindowEvent::ScaleFactorChanged { .. }
                        | WindowEvent::Occluded(false)
                        | WindowEvent::ThemeChanged(_)
                ) {
                    self.full_redraw = true;
                }
                // Portage fx : fréquence de l'écran qui affiche la fenêtre (cadence).
                if matches!(
                    window_event,
                    WindowEvent::Moved(_)
                        | WindowEvent::Resized(_)
                        | WindowEvent::ScaleFactorChanged { .. }
                ) {
                    self.frame_interval = self
                        .window
                        .current_monitor()
                        .and_then(|m| m.refresh_rate_millihertz())
                        .filter(|&mhz| mhz >= 24_000)
                        .map_or(DEFAULT_FRAME_INTERVAL, |mhz| {
                            Duration::from_secs_f64(1000.0 / f64::from(mhz))
                        });
                }
                if let WindowEvent::Moved(_) = window_event {
                    self.full_present = true;
                    self.window.request_redraw();
                }
                let response = self
                    .egui_winit
                    .on_window_event(self.window.deref(), &window_event);

                if response.repaint {
                    // Redraw when egui says it's necessary (e.g., mouse move, key press):
                    self.window.request_redraw();
                }
            }
        };

        Ok(())
    }
}

/// This struct contains statistics as well as possible interactions with the software renderer.
///
/// # Example
/// ```rust
///  use egui_software_backend::{App, SoftwareBackend};
///
/// struct MyApp {
///
/// }
///
/// impl App for MyApp {
///     fn ui(&mut self, ui: &mut egui::Ui, backend: &mut SoftwareBackend) {
///         backend.set_capture_frame_time(true);
///
///
///        egui::CentralPanel::default().show_inside(ui, |ui| {
///        ui.label(format!(
///           "Frame Time {}ms",
///            backend.last_frame_time().unwrap_or_default().as_millis()
///         ));
///     });
///    }
/// }
///
/// ```
pub struct SoftwareBackend {
    capture_frame_time: bool,
    last_frame_time: Option<Duration>,
}

impl SoftwareBackend {
    /// Returns true if the frame time for the next frame is captured.
    pub fn is_capture_frame_time(&self) -> bool {
        self.capture_frame_time
    }

    /// Enables or disables capturing the frame time.
    /// Note that once this is called, the value persists until this function is called again.
    /// Calling this with true will not affect the current frame, so once this is called with true,
    /// you will need to wait for 2 more frames until you get a value.
    pub fn set_capture_frame_time(&mut self, capture: bool) {
        self.capture_frame_time = capture;
    }

    /// Returns the rendering duration of the last frame if this information is available.
    /// Returns none otherwise. Note that this information is only captured is `set_capture_frame_time`
    /// is called with true.
    pub fn last_frame_time(&self) -> Option<Duration> {
        self.last_frame_time
    }
}

/// Implement this trait to write apps using the optional winit backend similarly to eframe's App.
pub trait App {
    fn ui(&mut self, ui: &mut egui::Ui, software_backend: &mut SoftwareBackend);

    fn on_exit(&mut self, _ctx: &Context) {}
}

/// Used to initialize the software render backend app, renderer, and winit configuration.
#[derive(Debug, Clone)]
pub struct SoftwareBackendAppConfiguration {
    /// The underlying egui viewport builder that is used to create the window with winit.
    pub viewport_builder: ViewportBuilder,

    /// If true: rasterized ClippedPrimitives are cached and rendered to an intermediate tiled canvas. That canvas is
    /// then rendered over the frame buffer. If false ClippedPrimitives are rendered directly to the frame buffer.
    /// Rendering without caching is much slower and primarily intended for testing.
    ///
    /// Default is true!
    pub allow_raster_opt: bool,

    /// If true: attempts to optimize by converting suitable triangle pairs into rectangles for faster rendering.
    ///   Things *should* look the same with this set to `true` while rendering faster.
    ///
    /// Default is true!
    pub convert_tris_to_rects: bool,

    /// If true: rasterized ClippedPrimitives are cached and rendered to an intermediate tiled canvas. That canvas is
    /// then rendered over the frame buffer. If false ClippedPrimitives are rendered directly to the frame buffer.
    /// Rendering without caching is much slower and primarily intended for testing.
    ///
    /// Default is true!
    pub caching: bool,
}

impl SoftwareBackendAppConfiguration {
    /// Creates a new SoftwareBackendAppConfiguration using the default configuration.
    pub const fn new() -> Self {
        //The constructor is not const.
        let vp = ViewportBuilder {
            override_redirect: None,
            title: None,
            app_id: None,
            position: None,
            //CGA
            inner_size: Some(Vec2::new(320f32, 200f32)),
            min_inner_size: None,
            max_inner_size: None,
            clamp_size_to_monitor_size: None,
            fullscreen: None,
            maximized: None,
            resizable: None,
            transparent: None,
            decorations: None,
            icon: None,
            active: None,
            visible: None,
            fullsize_content_view: None,
            movable_by_window_background: None,
            title_shown: None,
            titlebar_buttons_shown: None,
            titlebar_shown: None,
            has_shadow: None,
            drag_and_drop: None,
            taskbar: None,
            close_button: None,
            minimize_button: None,
            maximize_button: None,
            window_level: None,
            mouse_passthrough: None,
            window_type: None,
            monitor: None,
        };

        Self {
            viewport_builder: vp,

            allow_raster_opt: true,
            convert_tris_to_rects: true,
            caching: true,
        }
    }

    /// This sets the egui viewport builder to the given builder. This replaces most settings.
    pub fn viewport_builder(mut self, viewport_builder: ViewportBuilder) -> Self {
        self.viewport_builder = viewport_builder;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn title(mut self, title: Option<String>) -> Self {
        self.viewport_builder.title = title;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn app_id(mut self, app_id: Option<String>) -> Self {
        self.viewport_builder.app_id = app_id;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn position(mut self, position: Option<Pos2>) -> Self {
        self.viewport_builder.position = position;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn inner_size(mut self, inner_size: Option<Vec2>) -> Self {
        self.viewport_builder.inner_size = inner_size;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn min_inner_size(mut self, min_inner_size: Option<Vec2>) -> Self {
        self.viewport_builder.min_inner_size = min_inner_size;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn max_inner_size(mut self, max_inner_size: Option<Vec2>) -> Self {
        self.viewport_builder.max_inner_size = max_inner_size;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn clamp_size_to_monitor_size(
        mut self,
        clamp_size_to_monitor_size: Option<bool>,
    ) -> Self {
        self.viewport_builder.clamp_size_to_monitor_size = clamp_size_to_monitor_size;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn fullscreen(mut self, fullscreen: Option<bool>) -> Self {
        self.viewport_builder.fullscreen = fullscreen;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn maximized(mut self, maximized: Option<bool>) -> Self {
        self.viewport_builder.maximized = maximized;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn resizable(mut self, resizable: Option<bool>) -> Self {
        self.viewport_builder.resizable = resizable;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn transparent(mut self, transparent: Option<bool>) -> Self {
        self.viewport_builder.transparent = transparent;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn decorations(mut self, decorations: Option<bool>) -> Self {
        self.viewport_builder.decorations = decorations;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn icon(mut self, icon: Option<Arc<IconData>>) -> Self {
        self.viewport_builder.icon = icon;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn active(mut self, active: Option<bool>) -> Self {
        self.viewport_builder.active = active;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn visible(mut self, visible: Option<bool>) -> Self {
        self.viewport_builder.visible = visible;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn fullsize_content_view(mut self, fullsize_content_view: Option<bool>) -> Self {
        self.viewport_builder.fullsize_content_view = fullsize_content_view;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn movable_by_window_background(
        mut self,
        movable_by_window_background: Option<bool>,
    ) -> Self {
        self.viewport_builder.movable_by_window_background = movable_by_window_background;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn title_shown(mut self, title_shown: Option<bool>) -> Self {
        self.viewport_builder.title_shown = title_shown;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn titlebar_buttons_shown(mut self, titlebar_buttons_shown: Option<bool>) -> Self {
        self.viewport_builder.titlebar_buttons_shown = titlebar_buttons_shown;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn titlebar_shown(mut self, titlebar_shown: Option<bool>) -> Self {
        self.viewport_builder.titlebar_shown = titlebar_shown;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn has_shadow(mut self, has_shadow: Option<bool>) -> Self {
        self.viewport_builder.has_shadow = has_shadow;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn drag_and_drop(mut self, drag_and_drop: Option<bool>) -> Self {
        self.viewport_builder.has_shadow = drag_and_drop;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub fn taskbar(mut self, taskbar: Option<bool>) -> Self {
        self.viewport_builder.has_shadow = taskbar;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn close_button(mut self, close_button: Option<bool>) -> Self {
        self.viewport_builder.close_button = close_button;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn minimize_button(mut self, minimize_button: Option<bool>) -> Self {
        self.viewport_builder.minimize_button = minimize_button;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn maximize_button(mut self, maximize_button: Option<bool>) -> Self {
        self.viewport_builder.maximize_button = maximize_button;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn window_level(mut self, window_level: Option<WindowLevel>) -> Self {
        self.viewport_builder.window_level = window_level;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn mouse_passthrough(mut self, mouse_passthrough: Option<bool>) -> Self {
        self.viewport_builder.mouse_passthrough = mouse_passthrough;
        self
    }

    /// See egui::viewport::ViewportBuilder. This is a convenience function that sets the field.
    pub const fn window_type(mut self, window_type: Option<X11WindowType>) -> Self {
        self.viewport_builder.window_type = window_type;
        self
    }

    /// If false: Rasterize everything with triangles, always calculate vertex colors, uvs, use bilinear
    ///   everywhere, etc... Things *should* look the same with this set to `true` while rendering faster.
    ///
    /// Default is true!
    pub const fn allow_raster_opt(mut self, allow_raster_opt: bool) -> Self {
        self.allow_raster_opt = allow_raster_opt;
        self
    }

    /// If true: attempts to optimize by converting suitable triangle pairs into rectangles for faster rendering.
    ///   Things *should* look the same with this set to `true` while rendering faster.
    ///
    /// Default is true!
    pub const fn convert_tris_to_rects(mut self, convert_tris_to_rects: bool) -> Self {
        self.convert_tris_to_rects = convert_tris_to_rects;
        self
    }

    /// If true: rasterized ClippedPrimitives are cached and rendered to an intermediate tiled canvas. That canvas is
    /// then rendered over the frame buffer. If false ClippedPrimitives are rendered directly to the frame buffer.
    /// Rendering without caching is much slower and primarily intended for testing.
    ///
    /// Default is true!
    pub const fn caching(mut self, caching: bool) -> Self {
        self.caching = caching;
        self
    }
}

impl Default for SoftwareBackendAppConfiguration {
    fn default() -> Self {
        Self::new()
    }
}

const ONE_PIXEL: NonZeroU32 = NonZeroU32::new(1).unwrap();

static CLEAR_EACH_FRAME: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// Portage fx : désactive l'effacement du tampon avant chaque image. À n'utiliser que si
/// l'application peint toute la fenêtre (panneaux opaques) ; économise un remplissage
/// plein écran par image.
pub fn set_clear_each_frame(clear: bool) {
    CLEAR_EACH_FRAME.store(clear, core::sync::atomic::Ordering::Relaxed);
}

/// Portage fx : intervalle d'images par défaut (60 Hz) tant que l'écran n'est pas connu.
const DEFAULT_FRAME_INTERVAL: Duration = Duration::from_micros(16_667);

static FRAME_PACING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Portage fx : cadence les images demandées par egui (animations, `request_repaint`) sur la
/// fréquence de l'écran. Sans synchronisation verticale, une animation se jouerait sinon à
/// des centaines d'images par seconde. Les entrées (souris, clavier) restent immédiates.
/// À désactiver pour mesurer le temps d'image en défilement continu.
pub fn set_frame_pacing(on: bool) {
    FRAME_PACING.store(on, core::sync::atomic::Ordering::Relaxed);
}

static PARTIAL_REDRAW: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// Portage fx : active (défaut) ou désactive le rendu partiel du mode sans cache (ne
/// rasteriser et présenter que les zones modifiées, recopier ce qui défile). Désactivé, chaque
/// image est rasterisée et présentée entièrement, comme dans la crate d'origine.
pub fn set_partial_redraw(partial: bool) {
    PARTIAL_REDRAW.store(partial, core::sync::atomic::Ordering::Relaxed);
}

static CLIPBOARD_SHORTCUTS: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(true);

/// Portage fx : traduire (défaut) ou non Ctrl+C/X/V, Ctrl+Inser, Maj+Inser et Maj+Suppr en
/// événements `Copy`/`Cut`/`Paste` (cf. `egui_winit::State::set_clipboard_shortcuts`).
/// Désactivé, ces touches arrivent comme des `Event::Key` ordinaires : l'application les lie
/// à ses propres commandes (copier des fichiers…) et rend elle-même ces événements aux champs
/// de texte. À appeler avant `run_app_with_software_backend`.
pub fn set_clipboard_shortcuts(enabled: bool) {
    CLIPBOARD_SHORTCUTS.store(enabled, core::sync::atomic::Ordering::Relaxed);
}

/// Starts the app.
pub fn run_app_with_software_backend<T: App>(
    settings: SoftwareBackendAppConfiguration,
    egui_app_factory: impl FnMut(Context) -> T,
) -> Result<(), SoftwareBackendAppError> {
    let egui_software_render = EguiSoftwareRender::new(ColorFieldOrder::Bgra)
        .with_allow_raster_opt(settings.allow_raster_opt)
        .with_convert_tris_to_rects(settings.convert_tris_to_rects)
        .with_caching(settings.caching);

    let event_loop: EventLoop<UserEvent> = EventLoop::with_user_event()
        .build()
        .map_err(|e| SoftwareBackendAppError::EventLoop(Box::new(e)))?;

    let softbuffer_context = softbuffer::Context::new(event_loop.owned_display_handle()).map_err(
        SoftwareBackendAppError::soft_buffer("softbuffer::Context::new"),
    )?;

    let egui_ctx = Context::default();

    let event_loop_proxy = event_loop.create_proxy();
    egui_ctx.set_request_repaint_callback(move |info| {
        let when = Instant::now() + info.delay;
        let cumulative_pass_nr = info.current_cumulative_pass_nr;
        event_loop_proxy
            .send_event(UserEvent::RequestRepaint {
                viewport_id: info.viewport_id,
                when,
                cumulative_pass_nr,
            })
            .ok();
    });

    let mut app = WinitAppStateMachine::new(
        settings.clone(),
        egui_software_render,
        softbuffer_context,
        egui_app_factory,
        egui_ctx,
    );

    if let Err(event_loop_error) = event_loop.run_app(&mut app) {
        if let WinitAppStateMachine::Dead(Some(app_err)) = app {
            return Err(SoftwareBackendAppError::SuppressedEventLoop {
                event_loop_error: Box::new(event_loop_error),
                suppressed: Box::new(app_err),
            });
        }

        return Err(SoftwareBackendAppError::EventLoop(Box::new(
            event_loop_error,
        )));
    }

    if let WinitAppStateMachine::Dead(Some(app_err)) = app {
        return Err(app_err);
    }

    Ok(())
}
