//! Dessin de la fenêtre. Ne fait que lire l'état et émettre des [`Msg`].
//!
//! Exceptions documentées (appels Win32 obligatoirement sur le thread UI) : style DWM de la
//! fenêtre, options du menu contextuel shell (`IContextMenu`, cf. [`shellmenu`]) et
//! glisser-déposer sortant (`SHDoDragDrop`, modal) ; position du curseur pendant un glisser
//! OLE (cf. [`dnd`]).

mod columns;
mod diagnostics;
mod dialogs;
mod dnd;
mod gitmenu;
mod grid;
mod home;
mod items;
mod list;

pub(crate) use list::type_name;
mod menus;
mod navbar;
mod palette;
mod pane;
mod peer;
mod preview;
mod search;
pub(crate) mod settings;
mod shellmenu;
mod sidebar;
mod status;
mod statuscenter;
mod titlebar;
mod toasts;
mod toolbar;

use std::collections::HashMap;
use std::time::Instant;

use eframe::egui::{
    self, Color32, CursorIcon, Frame, Pos2, ResizeDirection, Theme, ViewportCommand,
};
use fx_app::{AppState, Location, Msg, PaneId};
use fx_commands::{CommandId, Key};
use fx_shell::ShellWorker;
use fx_store::Settings;

use crate::images::ImageCache;
use crate::theme::{self, Palette};

/// Défilement d'un volet (état visuel).
#[derive(Default, Clone)]
pub(crate) struct Scroll {
    pub offset: f32,
    pub view_h: f32,
    pub request: Option<f32>,
    /// Emplacement affiché à l'image précédente : un changement remet le défilement en haut.
    pub shown: Option<Location>,
    pub ensure: Option<usize>,
    /// Largeur utile de la vue (calcul des colonnes de la grille).
    pub width: f32,
}

/// Sélection rectangulaire en cours.
#[derive(Clone, Copy)]
pub(crate) struct Band {
    pub pane: PaneId,
    /// Origine en coordonnées de contenu (y inclut le défilement).
    pub origin: Pos2,
    pub additive: bool,
    pub last: Option<(usize, usize)>,
}

/// Renommage différé après un clic sur l'élément déjà seul sélectionné (comme
/// l'Explorateur) ; annulé par un double-clic, un autre clic, une touche ou la molette.
#[derive(Clone, Copy)]
pub(crate) struct SlowClick {
    pub pane: PaneId,
    pub id: u32,
    pub at: Instant,
}

/// État purement visuel, hors `AppState`.
pub struct ViewState {
    pub(crate) accent: Color32,
    /// Couleur d'accentuation de Windows (quand les réglages n'en imposent pas).
    pub(crate) system_accent: Color32,
    pub(crate) path_edit: Option<String>,
    pub(crate) search: HashMap<PaneId, String>,
    pub(crate) scroll: HashMap<PaneId, Scroll>,
    pub(crate) rename: Option<(u64, String)>,
    pub(crate) band: Option<Band>,
    /// Glisser-déposer de fichiers en cours et cibles de dépôt.
    pub(crate) dnd: dnd::DndState,
    /// Options supplémentaires (menu du shell dessiné par folder).
    pub(crate) shell: shellmenu::ShellMenuState,
    pub(crate) palette_query: String,
    pub(crate) palette_index: usize,
    pub(crate) search_index: usize,
    pub(crate) settings_section: usize,
    /// Paramètres › Mises à jour : notes de version affichées.
    pub(crate) settings_notes: bool,
    pub(crate) capture_hotkey: Option<CommandId>,
    /// Paramètres : capture du raccourci système qui ouvre folder, et refus éventuel.
    pub(crate) capture_open_hotkey: bool,
    pub(crate) open_hotkey_error: Option<String>,
    pub(crate) bulk: dialogs::BulkState,
    pub(crate) typeahead: (String, Option<Instant>),
    pub(crate) fullscreen: bool,
    pub(crate) hwnd: Option<isize>,
    pub(crate) styled_dark: Option<bool>,
    pub(crate) fallbacks: theme::Fallbacks,
    /// Bouton du centre d'état à l'image courante (ancre de son menu).
    pub(crate) status_anchor: Option<egui::Rect>,
    /// Bouton de la branche Git à l'image courante (ancre de son menu) et filtre du menu.
    pub(crate) git_anchor: Option<egui::Rect>,
    pub(crate) git_filter: String,
    pub(crate) git_menu_h: f32,
    /// Emplacement actif à l'image précédente (abandon des icônes en attente au changement).
    pub(crate) last_location: Option<Location>,
    pub(crate) diag: diagnostics::Sample,
    pub(crate) dont_ask_delete: bool,
    pub(crate) slow_click: Option<SlowClick>,
    /// Transition de contenu par volet : `(contenu affiché, début, nature)`.
    pub(crate) nav_anim: HashMap<PaneId, (u64, Instant, fx_app::state::NavKind)>,
    /// Onglet en cours de glisser (réordonnancement ou détachement).
    pub(crate) tab_drag: Option<titlebar::TabDrag>,
    /// Onglets glissés d'une fenêtre folder à une autre.
    pub(crate) peer: peer::PeerState,
    /// Apparition des onglets : premier affichage de chaque onglet (par identifiant).
    pub(crate) tab_born: HashMap<u64, Instant>,
    /// Images dessinées (pas d'animation d'apparition à la toute première).
    pub(crate) frames: u64,
    /// Barre latérale : ligne de l'élément actif à cette image (à l'écran), et la dernière
    /// connue (relative au haut du contenu, qui défile).
    pub(crate) sidebar_active: Option<egui::Rect>,
    pub(crate) sidebar_last: Option<egui::Rect>,
    /// Ctrl + molette accumulée (pavés tactiles : petits pas en points).
    pub(crate) wheel_zoom: f32,
}

impl ViewState {
    pub fn new(accent: Color32) -> Self {
        Self {
            accent,
            system_accent: accent,
            path_edit: None,
            search: HashMap::new(),
            scroll: HashMap::new(),
            rename: None,
            band: None,
            dnd: dnd::DndState::default(),
            shell: shellmenu::ShellMenuState::default(),
            palette_query: String::new(),
            palette_index: 0,
            search_index: 0,
            settings_section: 0,
            settings_notes: false,
            capture_hotkey: None,
            capture_open_hotkey: false,
            open_hotkey_error: None,
            bulk: dialogs::BulkState::default(),
            typeahead: (String::new(), None),
            fullscreen: false,
            hwnd: None,
            styled_dark: None,
            fallbacks: theme::Fallbacks::default(),
            status_anchor: None,
            git_anchor: None,
            git_filter: String::new(),
            git_menu_h: 0.0,
            last_location: None,
            diag: diagnostics::Sample::default(),
            dont_ask_delete: false,
            slow_click: None,
            nav_anim: HashMap::new(),
            tab_drag: None,
            peer: peer::PeerState::default(),
            tab_born: HashMap::new(),
            frames: 0,
            sidebar_active: None,
            sidebar_last: None,
            wheel_zoom: 0.0,
        }
    }

    /// Message d'une autre fenêtre folder : ceux qui concernent les onglets (survol, dépôt)
    /// sont gardés par la vue, les autres rendus à l'appelant.
    pub fn receive_peer(
        &mut self,
        msg: fx_store::remote::Remote,
    ) -> Option<fx_store::remote::Remote> {
        match msg {
            m @ (fx_store::remote::Remote::TabHover { .. }
            | fx_store::remote::Remote::TabHoverEnd
            | fx_store::remote::Remote::TabDrop { .. }) => {
                peer::receive(&mut self.peer, m);
                None
            }
            other => Some(other),
        }
    }

    /// Cette fenêtre a cédé son unique onglet à une autre : ne pas enregistrer sa session.
    pub fn merged_away(&self) -> bool {
        self.peer.merged_away
    }

    /// Fenêtre principale (pour les boîtes de dialogue natives).
    pub fn hwnd(&self) -> Option<isize> {
        self.hwnd
    }

    /// Défilement piloté par les bancs de mesure (vue Détails : lignes de 30 points) ;
    /// revient en haut en fin de liste.
    pub(crate) fn bench_scroll(&mut self, pane: PaneId, step: f32, rows: usize) {
        let sc = self.scroll.entry(pane).or_default();
        let max = (rows as f32 * 30.0 - sc.view_h).max(0.0);
        let next = sc.offset + step;
        sc.request = Some(if next > max { 0.0 } else { next });
    }
}

/// Données d'une image, passées à toutes les vues.
pub(crate) struct Fx<'a> {
    pub state: &'a AppState,
    pub vs: &'a mut ViewState,
    pub images: &'a mut ImageCache,
    pub shell: &'a ShellWorker,
    pub pal: Palette,
    pub msgs: Vec<Msg>,
    pub threads: usize,
}

impl Fx<'_> {
    pub fn send(&mut self, m: Msg) {
        self.msgs.push(m);
    }

    pub fn cmd(&mut self, c: CommandId) {
        self.msgs.push(Msg::Command(c));
    }

    pub fn enabled(&self, c: CommandId) -> bool {
        c.is_enabled(&self.state.command_context())
    }

    /// Raccourci affichable d'une commande.
    pub fn shortcut(&self, c: CommandId) -> String {
        self.state.keymap.shortcut_text(c).unwrap_or_default()
    }
}

pub fn draw(
    ui: &mut egui::Ui,
    state: &AppState,
    vs: &mut ViewState,
    images: &mut ImageCache,
    shell: &ShellWorker,
    threads: usize,
) -> Vec<Msg> {
    let ctx = ui.ctx().clone();
    let accent = theme::accent_of(&state.settings, vs.system_accent);
    if accent != vs.accent {
        vs.accent = accent;
        theme::set_accent(&ctx, accent);
    }
    theme::apply(&ctx, state.settings.theme, vs.accent);
    theme::apply_scroll_speed(&ctx, state.settings.scroll_lines);
    let pal = Palette::new(ctx.theme() == Theme::Dark, vs.accent);
    images.pump(&ctx);
    vs.fallbacks.apply(&ctx);
    style_window(vs, pal.dark);

    let loc = state.pane().location();
    if vs.last_location.as_ref() != Some(loc) {
        vs.last_location = Some(loc.clone());
        images.forget_pending(shell);
    }
    let mut f = Fx {
        state,
        vs,
        images,
        shell,
        pal,
        msgs: Vec::new(),
        threads,
    };
    requests(&ctx, &mut f);
    crate::keys::text_clipboard(&ctx);
    // Avant le clavier : Échap abandonne d'abord un glisser en cours.
    dnd::begin(&ctx, &mut f);
    keyboard(&ctx, &mut f);
    slow_click(&ctx, &mut f);
    wheel_zoom(&ctx, &mut f);
    mouse_nav(&ctx, &mut f);

    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    if !f.vs.fullscreen {
        titlebar::draw(ui, &mut f, maximized);
    }
    navbar::draw(ui, &mut f);
    toolbar::draw(ui, &mut f);
    status::draw_bar(ui, &mut f);
    if state.settings.show_sidebar {
        sidebar::draw(ui, &mut f);
    }
    if state.settings.show_preview {
        preview::draw(ui, &mut f);
    }
    egui::CentralPanel::default()
        .frame(Frame::new().fill(pal.base).inner_margin(egui::Margin {
            left: if state.settings.show_sidebar { 0 } else { 8 },
            right: 8,
            top: 0,
            bottom: 0,
        }))
        .show(ui, |ui| pane::draw_tab(ui, &mut f));

    statuscenter::draw(&ctx, &mut f);
    gitmenu::draw(&ctx, &mut f);
    toasts::draw(&ctx, &mut f);
    dialogs::draw(&ctx, &mut f);
    palette::draw(&ctx, &mut f);
    search::draw(&ctx, &mut f);
    if state.show_diagnostics {
        diagnostics::draw(&ctx, &mut f);
    }
    dnd::finish(&ctx, &mut f);
    if !maximized && !f.vs.fullscreen {
        resize_edges(&ctx);
    }
    shellmenu::show(&ctx, &mut f);
    shellmenu::finish(&ctx, &mut f);
    f.vs.frames += 1;
    f.msgs
}

/// Cadre DWM (coins arrondis, ombre, mode sombre) appliqué dès que la fenêtre existe.
fn style_window(vs: &mut ViewState, dark: bool) {
    if vs.hwnd.is_none() {
        vs.hwnd = fx_win::window::main_window(crate::WINDOW_TITLE);
        fx_win::window::init_ole();
        if let Some(h) = vs.hwnd {
            fx_win::window::set_app_icon(h);
        }
    }
    if let Some(h) = vs.hwnd
        && vs.styled_dark != Some(dark)
    {
        fx_win::window::style_frame(h, dark);
        vs.styled_dark = Some(dark);
    }
}

/// Demandes ponctuelles de l'état (focus, plein écran, menu shell).
fn requests(ctx: &egui::Context, f: &mut Fx<'_>) {
    let r = f.state.requests;
    if r == fx_app::state::UiRequests::default() {
        return;
    }
    if r.toggle_fullscreen {
        f.vs.fullscreen = !f.vs.fullscreen;
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(f.vs.fullscreen));
    }
    // Installeur de mise à jour lancé : fermeture normale (session enregistrée), avant que
    // l'installeur n'ait à fermer folder lui-même.
    if r.close_window {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    if r.shell_menu {
        let sel = f.state.pane().selected_paths();
        let target = if sel.is_empty() {
            f.state
                .pane()
                .working_dir()
                .map(|d| shellmenu::Target::Background(d.to_path_buf()))
        } else {
            Some(shellmenu::Target::Items(sel))
        };
        if let Some(t) = target {
            // Au clavier : sous le pointeur s'il est dans la fenêtre, sinon au centre.
            let at = ctx
                .pointer_latest_pos()
                .unwrap_or_else(|| ctx.content_rect().center());
            shellmenu::open_more(ctx, f, t, Some(at));
        }
    }
    if r.focus_path {
        f.vs.path_edit = Some(match f.state.pane().location() {
            Location::Dir(p) => p.display().to_string(),
            Location::Search { root, .. } => root.display().to_string(),
            _ => String::new(),
        });
    }
    f.send(Msg::ConsumeRequests);
}

/// Raccourcis clavier et saisie rapide (aller au premier élément commençant par les
/// lettres tapées, comme l'Explorateur).
fn keyboard(ctx: &egui::Context, f: &mut Fx<'_>) {
    if f.vs.capture_hotkey.is_some() {
        return; // l'éditeur de raccourcis capture la prochaine touche
    }
    let typing = ctx.egui_wants_keyboard_input();
    let overlays = f.state.dialog.is_some()
        || f.state.show_palette
        || f.state.show_search
        || f.state.git_menu.is_some();
    for hk in crate::keys::pressed(ctx) {
        if typing && crate::keys::conflicts_with_typing(&hk) {
            continue;
        }
        if hk.key == Key::Escape {
            if f.state.show_palette
                || f.state.show_search
                || f.state.show_status_center
                || f.state.git_menu.is_some()
            {
                f.send(Msg::CloseOverlays);
                continue;
            }
            if f.vs.path_edit.is_some() || f.state.renaming.is_some() || typing {
                continue;
            }
        }
        if overlays && !matches!(hk.key, Key::F(_)) && hk.key != Key::Escape {
            continue;
        }
        if let Some(cmd) = f.state.keymap.command_for(&hk) {
            f.cmd(cmd);
        } else if !typing {
            arrows(f, &hk);
        }
    }
    if !typing && !overlays {
        let text: String = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| {
                    if let egui::Event::Text(t) = e {
                        Some(t.clone())
                    } else {
                        None
                    }
                })
                .collect()
        });
        if !text.is_empty() && !text.chars().all(char::is_control) {
            typeahead(f, &text);
        }
    }
}

/// Déclenche le renommage différé quand son délai est écoulé.
fn slow_click(ctx: &egui::Context, f: &mut Fx<'_>) {
    let Some(sc) = f.vs.slow_click else { return };
    let interrupted = ctx.input(|i| {
        i.pointer.any_pressed()
            || i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key { pressed: true, .. } | egui::Event::MouseWheel { .. }
                )
            })
    });
    let still_selected = f
        .state
        .find_pane(sc.pane)
        .and_then(fx_app::Pane::active_listing)
        .is_some_and(|(_, sel)| sel.count() == 1 && sel.contains(sc.id));
    if interrupted || !still_selected || f.state.renaming.is_some() {
        f.vs.slow_click = None;
        return;
    }
    let now = Instant::now();
    if now >= sc.at {
        f.vs.slow_click = None;
        f.send(Msg::FocusPane(sc.pane));
        f.send(Msg::BeginRename);
    } else {
        ctx.request_repaint_after(sc.at - now);
    }
}

/// Boutons latéraux de la souris (4 : précédent, 5 : suivant), comme les navigateurs et
/// l'Explorateur.
fn mouse_nav(ctx: &egui::Context, f: &mut Fx<'_>) {
    let (back, forward) = ctx.input(|i| {
        (
            i.pointer.button_pressed(egui::PointerButton::Extra1),
            i.pointer.button_pressed(egui::PointerButton::Extra2),
        )
    });
    if back {
        f.cmd(CommandId::Back);
    }
    if forward {
        f.cmd(CommandId::Forward);
    }
}

/// Ctrl + molette : taille des éléments (Détails < Liste < icônes), comme l'Explorateur.
fn wheel_zoom(ctx: &egui::Context, f: &mut Fx<'_>) {
    const NOTCH: f32 = 50.0;
    let delta: f32 = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } if modifiers.command => Some(match unit {
                    egui::MouseWheelUnit::Point => delta.y,
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => delta.y * NOTCH,
                }),
                _ => None,
            })
            .sum()
    });
    let overlays = f.state.dialog.is_some() || f.state.show_palette || f.state.show_search;
    if delta == 0.0 || overlays || f.state.pane().listing.is_none() {
        return;
    }
    // Changement de sens : on repart de zéro pour réagir dès le premier cran.
    if f.vs.wheel_zoom * delta < 0.0 {
        f.vs.wheel_zoom = 0.0;
    }
    f.vs.wheel_zoom += delta;
    while f.vs.wheel_zoom.abs() >= NOTCH {
        let bigger = f.vs.wheel_zoom > 0.0;
        f.vs.wheel_zoom -= NOTCH.copysign(f.vs.wheel_zoom);
        f.cmd(if bigger {
            CommandId::ZoomIn
        } else {
            CommandId::ZoomOut
        });
    }
}

/// Flèches, Début/Fin, Pg préc./suiv. dans la liste.
fn arrows(f: &mut Fx<'_>, hk: &fx_commands::HotKey) {
    let pane = f.state.pane();
    let Some((l, sel)) = pane.active_listing() else {
        return;
    };
    if l.is_empty() {
        return;
    }
    let cur = sel.anchor.and_then(|a| l.position_of(a));
    let last = l.len() - 1;
    let per_row = if pane.layout == fx_store::Layout::Grid {
        grid::columns_for(f, pane.id)
    } else {
        1
    };
    let page = (f.vs.scroll.get(&pane.id).map_or(400.0, |s| s.view_h) / 30.0).max(1.0) as usize;
    let target = match hk.key {
        Key::Down => Some(cur.map_or(0, |p| (p + per_row).min(last))),
        Key::Up => Some(cur.map_or(0, |p| p.saturating_sub(per_row))),
        Key::Right if per_row > 1 => Some(cur.map_or(0, |p| (p + 1).min(last))),
        Key::Left if per_row > 1 => Some(cur.map_or(0, |p| p.saturating_sub(1))),
        Key::Home => Some(0),
        Key::End => Some(last),
        Key::PageDown => Some(cur.map_or(0, |p| (p + page).min(last))),
        Key::PageUp => Some(cur.map_or(0, |p| p.saturating_sub(page))),
        _ => None,
    };
    if let Some(pos) = target {
        let id = pane.id;
        f.send(Msg::Click {
            pane: id,
            pos,
            ctrl: hk.ctrl,
            shift: hk.shift,
        });
        f.vs.scroll.entry(id).or_default().ensure = Some(pos);
    }
}

fn typeahead(f: &mut Fx<'_>, text: &str) {
    let now = Instant::now();
    let (buf, at) = &mut f.vs.typeahead;
    if at.is_none_or(|t| now.duration_since(t).as_millis() > 900) {
        buf.clear();
    }
    *at = Some(now);
    buf.push_str(&text.to_lowercase());
    let query = buf.clone();
    let pane = f.state.pane();
    let Some((l, _)) = pane.active_listing() else {
        return;
    };
    let found = (0..l.len()).find(|&i| {
        l.at(i).is_some_and(|e| {
            fx_core::wtf8::display(e.name)
                .to_lowercase()
                .starts_with(&query)
        })
    });
    if let Some(pos) = found {
        let id = pane.id;
        f.send(Msg::Click {
            pane: id,
            pos,
            ctrl: false,
            shift: false,
        });
        f.vs.scroll.entry(id).or_default().ensure = Some(pos);
    }
}

/// Bords de redimensionnement de la fenêtre sans décorations.
fn resize_edges(ctx: &egui::Context) {
    let Some(pos) = ctx.pointer_hover_pos() else {
        return;
    };
    let r = ctx.content_rect();
    const E: f32 = 5.0;
    let (l, rt, t, b) = (
        pos.x - r.left() < E,
        r.right() - pos.x < E,
        pos.y - r.top() < E,
        r.bottom() - pos.y < E,
    );
    let dir = match (l, rt, t, b) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::West,
        (_, true, ..) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return,
    };
    ctx.set_cursor_icon(match dir {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}

/// Ouvre le menu shell demandé pendant l'image (modal, sur le thread UI).
/// Copie de travail des réglages pour une modification.
pub(crate) fn settings_with(state: &AppState, change: impl FnOnce(&mut Settings)) -> Msg {
    let mut s = state.settings.clone();
    change(&mut s);
    Msg::UpdateSettings(Box::new(s))
}
