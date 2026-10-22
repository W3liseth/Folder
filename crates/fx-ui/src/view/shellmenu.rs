//! Options supplémentaires : le menu contextuel du shell (Ouvrir avec, Envoyer vers,
//! extensions tierces…) dessiné par folder, avec les mêmes entrées, icônes et sous-menus que
//! le menu de Windows (cf. `fx_win::ctxmenu`).
//!
//! - « Afficher plus d'options » ouvre, à l'emplacement du menu de folder, un menu folder qui
//!   contient toutes ces entrées ;
//! - Maj + clic droit : le menu de folder les reprend directement à sa suite (entrées
//!   étendues comprises), sans celles qu'il propose déjà (Couper, Copier, Supprimer…).

use std::path::PathBuf;

use eframe::egui::{
    self, Align, Id, LayerId, Layout, Order, Pos2, Rect, TextureHandle, Ui, pos2, vec2,
};
use fx_app::{Location, Msg};
use fx_win::ctxmenu::{MenuItem, MenuOutcome, Modifiers};

use super::Fx;
use crate::theme::{SEMIBOLD, glyph};
use crate::widgets;

/// Ce que vise le menu.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Items(Vec<PathBuf>),
    /// Fond du dossier (Nouveau, Coller…).
    Background(PathBuf),
}

/// Entrée prête à dessiner (icônes en textures).
enum Item {
    Separator,
    Command {
        id: u32,
        label: String,
        shortcut: String,
        verb: Option<String>,
        enabled: bool,
        checked: bool,
        default: bool,
        icon: Option<TextureHandle>,
    },
    Submenu {
        label: String,
        enabled: bool,
        icon: Option<TextureHandle>,
        items: Vec<Item>,
    },
}

/// Verbes que le menu de folder propose déjà : écartés quand les deux menus sont réunis.
const FOLDER_VERBS: [&str; 15] = [
    "open",
    "explore",
    "opennewwindow",
    "opennewtab",
    "opennewprocess",
    "cut",
    "copy",
    "paste",
    "delete",
    "rename",
    "properties",
    "copyaspath",
    "pintohome",
    "pintohomefile",
    "undo",
];

#[derive(Default)]
pub(crate) struct ShellMenuState {
    /// Menu lu : cible, entrées étendues ou non, entrées.
    built: Option<(Target, bool, Vec<Item>)>,
    /// Menu « plus d'options » ouvert : position (points) et image d'ouverture (il n'est
    /// dessiné qu'à partir de la suivante, sans quoi le clic qui l'ouvre le refermerait).
    popup: Option<(Pos2, u64)>,
    /// Entrée choisie, exécutée en fin d'image (après le dessin des menus).
    chosen: Option<(u32, Option<String>)>,
    /// Dernier clic droit : position (le menu de folder s'y ouvre) et touche Maj.
    pub anchor: Pos2,
    pub shift: bool,
}

/// Clic droit sur un élément ou sur le fond : retient où et comment le menu s'ouvre.
pub(crate) fn secondary_click(ui: &Ui, f: &mut Fx<'_>) {
    let (pos, shift) = ui.input(|i| (i.pointer.interact_pos(), i.modifiers.shift));
    f.vs.shell.anchor = pos.unwrap_or_default();
    f.vs.shell.shift = shift;
}

/// « Afficher plus d'options » : ouvre le menu des options du shell à l'emplacement du menu
/// de folder (ou en `at` pour le clavier).
pub(crate) fn open_more(ctx: &egui::Context, f: &mut Fx<'_>, target: Target, at: Option<Pos2>) {
    let extended = ctx.input(|i| i.modifiers.shift);
    if ensure(ctx, f, &target, extended) {
        let at = at.unwrap_or(f.vs.shell.anchor);
        f.vs.shell.popup = Some((at, f.vs.frames));
    }
}

/// Maj + clic droit : entrées du shell (étendues), à la suite du menu de folder, sans celles
/// qu'il propose déjà.
pub(crate) fn inline(ui: &mut Ui, f: &mut Fx<'_>, target: Target) {
    if !ensure(ui.ctx(), f, &target, true) {
        return;
    }
    let Some((_, _, items)) = f.vs.shell.built.take() else {
        return;
    };
    let shown = prune(&items, true);
    if !shown.is_empty() {
        ui.separator();
        if let Some(choice) = render(ui, f, &items, true) {
            f.vs.shell.chosen = Some(choice);
            ui.close();
        }
    }
    f.vs.shell.built = Some((target, true, items));
}

/// Menu « plus d'options », s'il est ouvert.
pub(crate) fn show(ctx: &egui::Context, f: &mut Fx<'_>) {
    let Some((at, since)) = f.vs.shell.popup else {
        return;
    };
    if f.vs.frames <= since {
        ctx.request_repaint();
        return;
    }
    let Some((target, extended, items)) = f.vs.shell.built.take() else {
        f.vs.shell.popup = None;
        return;
    };
    let id = Id::unique("fx-shell-menu");
    let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let mut choice = None;
    egui::Popup::new(
        id,
        ctx.clone(),
        egui::PopupAnchor::Position(at),
        LayerId::new(Order::Foreground, id),
    )
    .kind(egui::PopupKind::Menu)
    .layout(Layout::top_down_justified(Align::Min))
    .style(egui::containers::menu::menu_style)
    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
    .open_bool(&mut open)
    .show(|ui| {
        widgets::menu_appear(ui);
        ui.set_min_width(260.0);
        if items.is_empty() {
            ui.label("Aucune option supplémentaire");
        }
        choice = widgets::menu_scroll(ui, |ui| render(ui, f, &items, false));
    });
    f.vs.shell.built = Some((target, extended, items));
    if choice.is_some() {
        f.vs.shell.chosen = choice;
        open = false;
    }
    if !open {
        f.vs.shell.popup = None;
    }
}

/// Fin d'image : exécute l'entrée choisie ; oublie le menu (et libère les extensions) quand
/// plus aucun menu ne l'affiche.
pub(crate) fn finish(ctx: &egui::Context, f: &mut Fx<'_>) {
    if let Some((id, verb)) = f.vs.shell.chosen.take() {
        run(ctx, f, id, verb);
    } else if f.vs.shell.built.is_some()
        && f.vs.shell.popup.is_none()
        && !egui::Popup::is_any_open(ctx)
    {
        f.vs.shell.built = None;
        fx_win::ctxmenu::release();
    }
}

/// Construit (si besoin) le menu de `target` ; faux s'il est indisponible.
fn ensure(ctx: &egui::Context, f: &mut Fx<'_>, target: &Target, extended: bool) -> bool {
    if f.vs
        .shell
        .built
        .as_ref()
        .is_some_and(|(t, e, _)| t == target && *e == extended)
    {
        return true;
    }
    let Some(hwnd) = f.vs.hwnd else {
        return false;
    };
    let read = match target {
        Target::Items(paths) => fx_win::ctxmenu::build(hwnd, paths, extended),
        Target::Background(dir) => fx_win::ctxmenu::build_background(hwnd, dir, extended),
    };
    match read {
        Ok(items) => {
            let items = convert(ctx, items);
            f.vs.shell.built = Some((target.clone(), extended, items));
            true
        }
        Err(e) => {
            f.vs.shell.built = None;
            f.send(Msg::Error(format!(
                "Options supplémentaires indisponibles : {}",
                e.message().trim()
            )));
            false
        }
    }
}

fn convert(ctx: &egui::Context, items: Vec<MenuItem>) -> Vec<Item> {
    let tex = |icon: Option<fx_win::image::Rgba>, ctx: &egui::Context| {
        icon.filter(|i| i.width > 0 && i.height > 0).map(|i| {
            let size = [i.width as usize, i.height as usize];
            ctx.load_texture(
                "shell-menu-icon",
                egui::ColorImage::from_rgba_premultiplied(size, &i.pixels),
                egui::TextureOptions::LINEAR,
            )
        })
    };
    items
        .into_iter()
        .map(|it| match it {
            MenuItem::Separator => Item::Separator,
            MenuItem::Command {
                id,
                label,
                shortcut,
                verb,
                enabled,
                checked,
                default,
                icon,
            } => Item::Command {
                id,
                label,
                shortcut,
                verb,
                enabled,
                checked,
                default,
                icon: tex(icon, ctx),
            },
            MenuItem::Submenu {
                label,
                enabled,
                icon,
                items,
            } => Item::Submenu {
                label,
                enabled,
                icon: tex(icon, ctx),
                items: convert(ctx, items),
            },
        })
        .collect()
}

/// Entrées à montrer (index dans `items`) : sans les doublons du menu de folder si
/// `dedupe`, sans sous-menu vide ni séparateur en tête, en fin ou doublé.
fn prune(items: &[Item], dedupe: bool) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let keep = match it {
            Item::Separator => out
                .last()
                .is_some_and(|&j| !matches!(items[j], Item::Separator)),
            Item::Command { verb, .. } => {
                !dedupe || verb.as_deref().is_none_or(|v| !FOLDER_VERBS.contains(&v))
            }
            Item::Submenu { items, .. } => !prune(items, false).is_empty(),
        };
        if keep {
            out.push(i);
        }
    }
    while out
        .last()
        .is_some_and(|&j| matches!(items[j], Item::Separator))
    {
        out.pop();
    }
    out
}

/// Dessine les entrées ; renvoie celle qui a été choisie (identifiant, verbe).
fn render(
    ui: &mut Ui,
    f: &mut Fx<'_>,
    items: &[Item],
    dedupe: bool,
) -> Option<(u32, Option<String>)> {
    let pal = f.pal;
    let mut choice = None;
    for i in prune(items, dedupe) {
        match &items[i] {
            Item::Separator => {
                ui.separator();
            }
            Item::Command {
                id,
                label,
                shortcut,
                verb,
                enabled,
                checked,
                default,
                icon,
            } => {
                let glyph = if *checked && icon.is_none() {
                    glyph::CHECK
                } else {
                    ""
                };
                let mut job = widgets::menu_entry_colored(glyph, pal.accent, label, &pal);
                if *default && let Some(s) = job.sections.last_mut() {
                    s.format.font_id =
                        egui::FontId::new(14.0, egui::FontFamily::Name(SEMIBOLD.into()));
                }
                let resp = ui.add_enabled(
                    *enabled,
                    egui::Button::new(job).shortcut_text(shortcut.as_str()),
                );
                paint_icon(ui, icon.as_ref(), resp.rect, *enabled);
                if resp.clicked() {
                    choice = Some((*id, verb.clone()));
                }
            }
            Item::Submenu {
                label,
                enabled,
                icon,
                items,
            } => {
                let job = widgets::menu_entry("", label, &pal);
                let r = ui.add_enabled_ui(*enabled, |ui| {
                    ui.menu_button(job, |ui| {
                        widgets::menu_appear(ui);
                        ui.set_min_width(220.0);
                        render(ui, f, items, false)
                    })
                });
                let resp = &r.inner.response;
                paint_icon(ui, icon.as_ref(), resp.rect, *enabled);
                if let Some(Some(c)) = r.inner.inner {
                    choice = Some(c);
                }
            }
        }
    }
    choice
}

/// Icône d'une entrée, posée dans la colonne d'icône réservée par `menu_entry`.
fn paint_icon(ui: &Ui, icon: Option<&TextureHandle>, rect: Rect, enabled: bool) {
    let Some(tex) = icon else { return };
    let x = rect.left() + ui.spacing().button_padding.x + 6.0 + 7.0;
    let at = Rect::from_center_size(pos2(x, rect.center().y), vec2(16.0, 16.0));
    let tint = if enabled {
        egui::Color32::WHITE
    } else {
        egui::Color32::WHITE.gamma_multiply(0.4)
    };
    ui.painter().image(
        tex.id(),
        at,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        tint,
    );
}

/// Exécute l'entrée `id`. Ouvrir un dossier reste dans folder (le shell ouvrirait
/// l'Explorateur) ; Renommer passe par le renommage sur place.
fn run(ctx: &egui::Context, f: &mut Fx<'_>, id: u32, verb: Option<String>) {
    let single_dir = match f.vs.shell.built.as_ref().map(|b| &b.0) {
        Some(Target::Items(paths)) if paths.len() == 1 && paths[0].is_dir() => {
            Some(paths[0].clone())
        }
        _ => None,
    };
    if let (Some(dir), Some("open" | "explore")) = (&single_dir, verb.as_deref()) {
        f.send(Msg::Navigate(Location::Dir(dir.clone())));
        fx_win::ctxmenu::release();
        f.vs.shell.built = None;
        return;
    }
    let Some(hwnd) = f.vs.hwnd else { return };
    let mods = ctx.input(|i| Modifiers {
        shift: i.modifiers.shift,
        ctrl: i.modifiers.ctrl,
    });
    // L'exécution consomme le menu (cf. `fx_win::ctxmenu::invoke`).
    f.vs.shell.built = None;
    match fx_win::ctxmenu::invoke(hwnd, id, fx_win::window::cursor_pos(), mods) {
        Ok(MenuOutcome::Rename) => f.send(Msg::BeginRename),
        Ok(MenuOutcome::Invoked { verb }) => f.send(Msg::ShellInvoked { verb }),
        Err(e) => f.send(Msg::Error(format!(
            "Commande indisponible : {}",
            e.message().trim()
        ))),
    }
}
