//! Barre de titre maison avec onglets (comme Files et l'Explorateur de Windows 11).

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Frame, Id, LayerId, Order, Pos2, Rect, Sense, Ui,
    Vec2, ViewportCommand, pos2, vec2,
};
use fx_app::{Location, Msg};
use fx_commands::CommandId;

use super::Fx;
use crate::theme::{glyph, icon_font, semibold};
use crate::widgets;

const HEIGHT: f32 = 40.0;
const BUTTON_W: f32 = 46.0;

/// Onglet en cours de glisser.
#[derive(Clone, Copy)]
pub(crate) struct TabDrag {
    /// Identifiant de l'onglet (son index change quand on le déplace).
    id: u64,
    /// Point de saisie dans l'onglet : il reste sous le pointeur.
    grab: Vec2,
}

pub fn draw(ui: &mut Ui, f: &mut Fx<'_>, maximized: bool) {
    let pal = f.pal;
    egui::Panel::top("titlebar")
        .exact_size(HEIGHT)
        .show_separator_line(false)
        .frame(Frame::new().fill(pal.base))
        .show(ui, |ui| {
            let full = ui.max_rect();
            let update_w = update_button(ui, f, full);
            let buttons_w = BUTTON_W * 3.0 + update_w;
            let tabs_area = Rect::from_min_max(
                pos2(full.left() + 8.0, full.top() + 6.0),
                pos2(full.right() - buttons_w - 46.0, full.bottom()),
            );
            let end_x = tabs(ui, f, tabs_area);

            // Bouton « nouvel onglet ».
            let plus =
                Rect::from_center_size(pos2(end_x + 20.0, full.center().y + 3.0), vec2(32.0, 28.0));
            let r = ui.interact(plus, Id::unique("newtab"), Sense::click());
            widgets::hover_fill(ui, &r, plus, &pal, 4.0);
            ui.painter().text(
                plus.center(),
                Align2::CENTER_CENTER,
                glyph::ADD,
                icon_font(12.0),
                pal.text,
            );
            if r.on_hover_text(format!("Nouvel onglet ({})", f.shortcut(CommandId::NewTab)))
                .clicked()
            {
                f.cmd(CommandId::NewTab);
            }

            // Zone de déplacement de la fenêtre (le reste de la barre).
            let drag = Rect::from_min_max(
                pos2(plus.right() + 4.0, full.top()),
                pos2(full.right() - buttons_w, full.bottom()),
            );
            if drag.width() > 0.0 {
                let r = ui.interact(drag, Id::unique("titledrag"), Sense::click_and_drag());
                if r.double_clicked() {
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                } else if r.drag_started() {
                    ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
                }
            }

            window_buttons(ui, f, full, maximized);
        });
}

/// Dessine les onglets ; renvoie l'abscisse de fin.
///
/// Glisser d'un onglet : il reste attaché au pointeur au point où il a été saisi. Dans la
/// barre, il glisse horizontalement et les autres s'écartent pour lui faire place (ordre
/// appliqué au relâchement) ; au-delà, il flotte sous le pointeur. Relâché sur la barre
/// d'onglets d'une autre fenêtre folder, il la rejoint (cf. `peer`) : elle le montre et lui
/// fait place exactement de la même façon, puis il glisse jusqu'à sa place. Ailleurs, il part
/// dans une nouvelle fenêtre. Seul onglet de sa fenêtre : c'est la fenêtre entière qui suit
/// le pointeur (comme Chrome), masquée au-dessus de la barre d'une autre fenêtre et fermée
/// si elle la rejoint.
/// Suivi maison plutôt que `egui::DragAndDrop`, qui impose le curseur « saisie » (flèche
/// multidirectionnelle sous Windows).
fn tabs(ui: &mut Ui, f: &mut Fx<'_>, area: Rect) -> f32 {
    let pal = f.pal;
    let ctx = ui.ctx().clone();
    let bar = ui.max_rect();
    let count = f.state.tabs.len();
    // Onglet d'une autre fenêtre survolant la barre (ou qui vient d'y être lâché) : traité
    // comme un des nôtres qu'on glisserait.
    let guest = super::peer::incoming(&ctx, f);
    let n = (count + usize::from(guest.is_some())).max(1);
    let w = (area.width() / n as f32).clamp(110.0, 230.0);
    // Onglets disparus : on oublie leur date d'apparition.
    let alive: Vec<u64> = f.state.tabs.iter().map(|t| t.id).collect();
    f.vs.tab_born.retain(|id, _| alive.contains(id));

    // Glisser en cours : index actuel de l'onglet saisi (retrouvé par son identifiant).
    let drag =
        f.vs.tab_drag
            .and_then(|d| alive.iter().position(|&id| id == d.id).map(|i| (i, d)));
    if drag.is_none() {
        f.vs.tab_drag = None;
    }
    let pointer = ctx.input(|i| i.pointer.latest_pos());
    let detachable = count > 1;
    let moving_window = drag.is_some() && !detachable;
    let floating = drag.is_some()
        && detachable
        && !pointer.is_some_and(|p| bar.expand2(vec2(0.0, 24.0)).contains(p));
    // Onglet (ou fenêtre) au-dessus de la barre d'une autre fenêtre : elle est prévenue.
    let peer_target = (floating || moving_window)
        .then(|| super::peer::target_at_cursor(f, HEIGHT))
        .flatten();
    if let Some((from, d)) = drag {
        super::peer::hover(f, peer_target, from, d.grab);
        if peer_target.is_some() {
            // Pointeur immobile : on continue de donner des nouvelles (sinon l'aperçu expire).
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
        if moving_window {
            follow_pointer(&ctx, f, area, d.grab, peer_target.is_some());
        }
    }
    // Onglet saisi dans la barre (le nôtre ou celui d'une autre fenêtre) : abscisse sous le
    // pointeur au point de saisie, bornée à la barre, et place visée (la plus proche).
    let x_max = area.left() + w * (n - 1) as f32;
    let held_x = |p: Pos2, grab: Vec2| (p.x - grab.x).clamp(area.left(), x_max);
    let slot_of = |x: f32| (((x - area.left()) / w).round().max(0.0) as usize).min(n - 1);
    let (drag_x, target) = match (drag, pointer) {
        (Some((from, d)), Some(p)) => {
            let x = held_x(p, d.grab);
            (x, if floating { from } else { slot_of(x) })
        }
        (Some((from, _)), None) => (area.left() + w * from as f32, from),
        _ => (0.0, 0),
    };
    let guest_x = guest.as_ref().map(|g| held_x(g.pointer, g.grab));

    // Onglet adopté (lâché ici par une autre fenêtre, inséré depuis) : il glisse depuis
    // l'endroit du dépôt au lieu d'apparaître comme un nouvel onglet.
    let adopted = f.vs.peer.adopted.filter(|a| count > a.tabs);
    if adopted.is_some() {
        f.vs.peer.adopted = None;
    }
    // Apparition (160 ms) : un nouvel onglet s'élargit et apparaît en fondu.
    let first_frame = f.vs.frames == 0;
    let grows: Vec<f32> = alive
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            let at_rest = first_frame || adopted.is_some_and(|a| a.index == i);
            let born = *f.vs.tab_born.entry(id).or_insert_with(|| {
                if at_rest {
                    std::time::Instant::now() - std::time::Duration::from_secs(1)
                } else {
                    std::time::Instant::now()
                }
            });
            crate::widgets::ease_out(born.elapsed().as_secs_f32() / 0.16)
        })
        .collect();
    if grows.iter().any(|&g| g < 1.0) {
        ctx.request_repaint();
    }
    // Places : ordre affiché (l'onglet saisi à sa place visée, ou retiré s'il flotte ; place
    // de l'onglet entrant).
    let mut order: Vec<usize> = (0..count)
        .filter(|&i| drag.map(|d| d.0) != Some(i))
        .collect();
    if let Some((from, _)) = drag
        && !floating
    {
        order.insert(target.min(order.len()), from);
    }
    let guest_slot = guest_x.map(|gx| slot_of(gx).min(order.len()));
    let mut slots = vec![area.left(); count];
    let mut x = area.left();
    for (k, &i) in order.iter().enumerate() {
        if guest_slot == Some(k) {
            x += w;
        }
        slots[i] = x;
        x += w * grows[i];
    }
    if guest_slot == Some(order.len()) {
        x += w;
    }

    for (i, tab) in f.state.tabs.iter().enumerate() {
        let id = Id::unique(("tab", tab.id));
        let dragged = drag.is_some_and(|(d, _)| d == i);
        let grow = grows[i];
        // Les autres glissent vers leur place (ouverture, fermeture, déplacement) ; l'onglet
        // saisi suit le pointeur sans délai, puis rejoint sa place au relâchement.
        if let Some(a) = adopted.filter(|a| a.index == i) {
            ctx.animate_value_with_time(id.with("x"), a.x, 0.0);
        }
        let tx = if dragged {
            ctx.animate_value_with_time(id.with("x"), drag_x, 0.0)
        } else {
            ctx.animate_value_with_time(id.with("x"), slots[i], 0.16)
        };
        let rect = Rect::from_min_size(
            pos2(tx, area.top()),
            vec2((w - 2.0) * grow.max(0.05), area.height()),
        );
        let active = i == f.state.active_tab || dragged;
        let mut tab_ui = ui.new_child(egui::UiBuilder::new().max_rect(area));
        if grow < 1.0 {
            tab_ui.set_opacity(grow);
        }
        let resp = tab_ui.interact(rect, id, Sense::click_and_drag());
        if resp.drag_started_by(egui::PointerButton::Primary) {
            let origin = ctx.input(|i| i.pointer.press_origin()).unwrap_or(rect.min);
            f.vs.tab_drag = Some(TabDrag {
                id: tab.id,
                grab: origin - rect.min,
            });
        }
        // L'onglet saisi est dessiné au-dessus de tout (il peut quitter la barre de titre).
        let mut float_ui;
        let (ui, rect) = if dragged {
            float_ui = float_layer(ui, &ctx, id);
            let rect = match (floating, pointer, drag) {
                (true, Some(p), Some((_, d))) => Rect::from_min_size(p - d.grab, rect.size()),
                _ => rect,
            };
            (&mut float_ui, rect)
        } else {
            (&mut tab_ui, rect)
        };
        let hovered = !dragged
            && drag.is_none()
            && guest.is_none()
            && (resp.hovered() || resp.contains_pointer());
        let t = ctx.animate_bool_with_time(id.with("h"), hovered, 0.1);
        let radius = if dragged && floating {
            CornerRadius::same(8)
        } else {
            TAB_RADIUS
        };
        if dragged && floating {
            ui.painter().rect_filled(
                rect.translate(vec2(0.0, 3.0)).expand(1.0),
                radius,
                Color32::from_black_alpha(60),
            );
        }
        if active {
            active_background(ui, &pal, rect, radius);
        } else if t > 0.0 {
            ui.painter().rect_filled(
                rect.shrink2(vec2(0.0, 3.0)).translate(vec2(0.0, -1.0)),
                4.0,
                pal.hover.gamma_multiply(t),
            );
        } else if let Some(pos) = order.iter().position(|&j| j == i)
            && guest_slot != Some(pos + 1)
            && order
                .get(pos + 1)
                .is_some_and(|&next| next != f.state.active_tab && drag.map(|d| d.0) != Some(next))
        {
            // Séparateur discret entre onglets inactifs.
            ui.painter().vline(
                rect.right() + 1.0,
                rect.y_range().shrink(9.0),
                egui::Stroke::new(1.0, pal.stroke_strong),
            );
        }

        let loc = tab.active_pane().location();
        tab_face(ui, f, rect, loc, &tab.title(), active);
        if dragged && floating && detachable {
            // Ce que fera le relâchement (s'il ne rejoint pas une autre fenêtre).
            super::dnd::ghost(
                &ctx,
                &pal,
                pos2(rect.left() + 6.0, rect.bottom() + 8.0),
                glyph::OPEN,
                pal.text2,
                "Nouvelle fenêtre",
            );
        }
        // Fondu du texte sous le bouton de fermeture.
        let close =
            Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(22.0, 22.0));
        if !dragged && drag.is_none() && guest.is_none() && (active || hovered) {
            let cr = ui.interact(close, id.with("close"), Sense::click());
            widgets::hover_fill(ui, &cr, close, &pal, 4.0);
            ui.painter().text(
                close.center(),
                Align2::CENTER_CENTER,
                glyph::CLOSE,
                icon_font(9.0),
                pal.text2,
            );
            if cr.clicked() {
                f.send(Msg::CloseTab(i));
            }
        }
        // Fichiers glissés sur un onglet : dépôt dans le dossier qu'il affiche.
        if let Location::Dir(p) = loc {
            super::dnd::zone(f, rect, super::dnd::Target::Dir(p.clone()));
        }

        if resp.clicked() {
            f.send(Msg::SelectTab(i));
        }
        if resp.middle_clicked() {
            f.send(Msg::CloseTab(i));
        }
        resp.context_menu(|ui| {
            ui.set_min_width(220.0);
            if ui.button("Nouvel onglet").clicked() {
                f.cmd(CommandId::NewTab);
                ui.close();
            }
            if ui.button("Dupliquer l'onglet").clicked() {
                f.send(Msg::SelectTab(i));
                f.cmd(CommandId::DuplicateTab);
                ui.close();
            }
            if ui
                .add_enabled(
                    !f.state.closed_tabs.is_empty(),
                    egui::Button::new("Rouvrir l'onglet fermé"),
                )
                .clicked()
            {
                f.cmd(CommandId::ReopenClosedTab);
                ui.close();
            }
            ui.separator();
            if ui.button("Fermer l'onglet").clicked() {
                f.send(Msg::CloseTab(i));
                ui.close();
            }
            if ui
                .add_enabled(
                    f.state.tabs.len() > 1,
                    egui::Button::new("Fermer les autres onglets"),
                )
                .clicked()
            {
                for j in (0..f.state.tabs.len()).rev().filter(|&j| j != i) {
                    f.send(Msg::CloseTab(j));
                }
                ui.close();
            }
        });
        if drag.is_none() {
            resp.on_hover_text(match tab.active_pane().location() {
                Location::Dir(p) => p.display().to_string(),
                other => other.title(),
            });
        }
    }

    // Onglet d'une autre fenêtre : dessiné comme un des nôtres qu'on glisse, puis inséré à
    // sa place s'il a été lâché (il en repart en glissant, cf. `adopted`).
    if let (Some(g), Some(gx)) = (guest, guest_x) {
        let id = Id::unique("tab-guest");
        let rect = Rect::from_min_size(pos2(gx, area.top()), vec2(w - 2.0, area.height()));
        let mut float_ui = float_layer(ui, &ctx, id);
        active_background(&float_ui, &pal, rect, TAB_RADIUS);
        let title = g.location.title();
        tab_face(&mut float_ui, f, rect, &g.location, &title, true);
        super::peer::adopt(f, guest_slot.unwrap_or(count), gx);
        ctx.request_repaint();
    }

    // Fin du glisser : autre fenêtre rejointe, nouvel ordre, ou nouvelle fenêtre si l'onglet
    // flottait hors de la barre.
    if let Some((from, d)) = drag {
        let (released, escape) =
            ctx.input(|i| (!i.pointer.primary_down(), i.key_pressed(egui::Key::Escape)));
        if escape {
            f.vs.tab_drag = None;
            super::peer::end(f);
            super::peer::uncloak(f);
        } else if released {
            f.vs.tab_drag = None;
            if let Some(t) = peer_target
                && super::peer::send_tab(f, t, from, d.grab)
            {
                if detachable {
                    f.send(Msg::RemoveTab(from));
                } else {
                    // Fenêtre fondue dans l'autre (encore masquée : elle disparaît sans
                    // réapparaître).
                    f.vs.peer.merged_away = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            } else if moving_window {
                // Fenêtre lâchée ailleurs : elle reste où on l'a amenée.
                super::peer::end(f);
                super::peer::uncloak(f);
            } else if floating {
                super::peer::end(f);
                let rect = detach_rect(&ctx, f, area, d.grab);
                f.send(Msg::DetachTab { index: from, rect });
            } else {
                super::peer::end(f);
                if target != from {
                    f.send(Msg::MoveTab { from, to: target });
                }
            }
        }
    }
    x
}

/// Coins d'un onglet posé dans la barre (arrondis en haut seulement).
const TAB_RADIUS: CornerRadius = CornerRadius {
    nw: 8,
    ne: 8,
    sw: 0,
    se: 0,
};

/// Calque au-dessus de tout pour un onglet qu'on glisse (il peut quitter la barre de titre).
fn float_layer(ui: &mut Ui, ctx: &egui::Context, id: Id) -> Ui {
    let mut layer = ui.new_child(
        egui::UiBuilder::new()
            .layer_id(LayerId::new(Order::Foreground, id.with("float")))
            .max_rect(ctx.content_rect()),
    );
    layer.set_clip_rect(ctx.content_rect());
    layer
}

/// Fond de l'onglet actif (ou saisi) : surface du contenu et liseré d'accent. Posé dans la
/// barre (coins bas droits), il se prolonge par deux congés concaves dans la barre de
/// navigation, de même couleur : l'onglet et la page ne font qu'une surface, comme dans Files
/// et Edge.
fn active_background(ui: &Ui, pal: &crate::theme::Palette, rect: Rect, radius: CornerRadius) {
    ui.painter().rect_filled(rect, radius, pal.layer);
    if radius.sw == 0 {
        for side in [-1.0, 1.0] {
            ui.painter().add(flare(rect, side, pal.layer));
        }
    }
    widgets::neon_line(
        ui,
        (rect.left() + 10.0)..=(rect.right() - 10.0),
        rect.top(),
        2.0,
        pal.accent,
    );
}

/// Congé concave au pied de l'onglet, à gauche (`side` = -1) ou à droite (1) : coin compris
/// entre le bord de l'onglet, le haut de la barre de navigation et un quart de cercle. La
/// forme est étoilée depuis le coin de l'onglet, premier point du chemin : le remplissage en
/// éventail d'epaint (prévu pour les polygones convexes) reste exact. Les bords droits
/// (pied et flanc) sont rentrés d'un demi-pixel : l'anticrénelage d'epaint déborde d'un
/// demi-pixel hors du polygone et ajouterait sinon un liseré plus clair sur la barre de
/// navigation et sur l'onglet, déjà de cette couleur.
fn flare(tab: Rect, side: f32, fill: Color32) -> egui::Shape {
    const R: f32 = 8.0;
    const STEPS: usize = 8;
    let x = if side < 0.0 { tab.left() } else { tab.right() } + side * 0.5;
    let b = tab.bottom() - 0.5;
    let center = pos2(x + side * R, b - R);
    let mut points = Vec::with_capacity(STEPS + 2);
    points.push(pos2(x, b));
    // Du pied (sur la barre de navigation) jusqu'au bord de l'onglet.
    for k in 0..=STEPS {
        let a = std::f32::consts::FRAC_PI_2 * k as f32 / STEPS as f32;
        points.push(pos2(center.x - side * R * a.sin(), center.y + R * a.cos()));
    }
    egui::Shape::Path(egui::epaint::PathShape::convex_polygon(
        points,
        fill,
        egui::Stroke::NONE,
    ))
}

/// Icône et titre d'un onglet.
fn tab_face(ui: &mut Ui, f: &mut Fx<'_>, rect: Rect, loc: &Location, title: &str, active: bool) {
    let pal = f.pal;
    let g = match loc {
        Location::Home => glyph::HOME,
        Location::Search { .. } => glyph::SEARCH,
        Location::Tag(_) => glyph::TAG,
        Location::Dir(p) if p.parent().is_none() => glyph::DRIVE,
        Location::Dir(_) => glyph::FOLDER,
        Location::RecycleBin => glyph::RECYCLE,
        Location::Network(_) => glyph::NETWORK,
        Location::DiskUsage { .. } => glyph::PIE,
    };
    let gc = if matches!(loc, Location::Dir(p) if p.parent().is_some()) {
        pal.folder
    } else {
        pal.text2
    };
    let icon_at = pos2(rect.left() + 18.0, rect.center().y);
    match loc.icon_source() {
        Some(p) => super::items::shell_icon(ui, f, &p, icon_at, 16.0, g, gc),
        None => {
            ui.painter()
                .text(icon_at, Align2::CENTER_CENTER, g, icon_font(14.0), gc);
        }
    }
    f.vs.fallbacks.scan(title);
    let font = if active {
        semibold(13.0)
    } else {
        FontId::proportional(13.0)
    };
    let text_rect = Rect::from_min_max(
        pos2(rect.left() + 32.0, rect.top()),
        pos2(rect.right() - 28.0, rect.bottom()),
    );
    ui.painter().with_clip_rect(text_rect).text(
        pos2(text_rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        title,
        font,
        if active { pal.text } else { pal.text2 },
    );
}

/// Coin haut gauche (pixels écran) d'une fenêtre dont le premier onglet aurait sous le
/// curseur le point où l'onglet a été saisi.
fn origin_under_cursor(ctx: &egui::Context, area: Rect, grab: Vec2) -> (i32, i32) {
    let (cx, cy) = fx_win::window::cursor_pos();
    let ppp = ctx.pixels_per_point();
    let origin = ctx.content_rect().min;
    (
        cx - ((area.left() - origin.x + grab.x) * ppp).round() as i32,
        cy - ((area.top() - origin.y + grab.y) * ppp).round() as i32,
    )
}

/// Placement de la fenêtre d'un onglet détaché : même taille que celle-ci, son premier
/// onglet sous le pointeur au point où l'onglet a été saisi.
fn detach_rect(ctx: &egui::Context, f: &Fx<'_>, area: Rect, grab: Vec2) -> Option<[i32; 4]> {
    let p = f.vs.hwnd.and_then(fx_win::window::placement)?;
    let (left, top) = origin_under_cursor(ctx, area, grab);
    Some([
        left,
        top,
        left + (p.right - p.left),
        top + (p.bottom - p.top),
    ])
}

/// Glisser du seul onglet : la fenêtre suit le pointeur (restaurée d'abord si elle est
/// agrandie), masquée tant qu'elle survole la barre d'onglets d'une autre fenêtre. Le
/// pointeur ne bouge plus dans la fenêtre qui se déplace avec lui : winit n'envoie alors
/// aucun événement, d'où une image demandée à chaque fois, le temps du glisser.
fn follow_pointer(ctx: &egui::Context, f: &mut Fx<'_>, area: Rect, grab: Vec2, over_peer: bool) {
    let Some(hwnd) = f.vs.hwnd else { return };
    super::peer::set_cloaked(f, hwnd, over_peer);
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
    } else {
        let (x, y) = origin_under_cursor(ctx, area, grab);
        fx_win::window::move_to(hwnd, x, y);
    }
    ctx.request_repaint();
}

/// « Mettre à jour », à gauche des boutons de la fenêtre, quand une nouvelle version est
/// téléchargée (cf. `fx_app::upgrade`) ; renvoie la largeur occupée (0 sans mise à jour).
fn update_button(ui: &mut Ui, f: &mut Fx<'_>, full: Rect) -> f32 {
    let Some(version) = f.state.update.ready_version() else {
        return 0.0;
    };
    let pal = f.pal;
    let label = "Mettre à jour";
    let font = FontId::proportional(12.5);
    let text_w = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x;
    let w = 12.0 + 14.0 + 6.0 + text_w + 12.0;
    // Centré comme les glyphes des boutons de la fenêtre voisins (hauts de HEIGHT - 4),
    // pas comme les onglets, décalés vers le bas.
    let rect = Rect::from_center_size(
        pos2(
            full.right() - BUTTON_W * 3.0 - 8.0 - w / 2.0,
            full.top() + (HEIGHT - 4.0) / 2.0,
        ),
        vec2(w, 26.0),
    );
    let r = ui.interact(rect, Id::unique("update-ready"), Sense::click());
    let hover = ui
        .ctx()
        .animate_bool_with_time(r.id.with("h"), r.hovered(), 0.12);
    let fill = if r.is_pointer_button_down_on() {
        crate::theme::mix(pal.accent, Color32::BLACK, 0.12)
    } else {
        crate::theme::mix(pal.accent, Color32::WHITE, 0.10 * hover)
    };
    ui.painter().rect_filled(rect, 13.0, fill);
    let on = if pal.dark {
        pal.on_accent
    } else {
        Color32::WHITE
    };
    ui.painter().text(
        pos2(rect.left() + 12.0 + 7.0, rect.center().y),
        Align2::CENTER_CENTER,
        glyph::DOWNLOAD,
        icon_font(12.0),
        on,
    );
    ui.painter().text(
        pos2(rect.left() + 12.0 + 14.0 + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        on,
    );
    if r.on_hover_text(format!(
        "folder {version} est prêt : l'installer et redémarrer folder (onglets conservés)"
    ))
    .clicked()
    {
        f.send(Msg::InstallUpdate);
    }
    w + 8.0
}

fn window_buttons(ui: &mut Ui, f: &mut Fx<'_>, full: Rect, maximized: bool) {
    let pal = f.pal;
    let ctx = ui.ctx().clone();
    let items = [
        (glyph::MINIMIZE, "Réduire", 0),
        (
            if maximized {
                glyph::RESTORE
            } else {
                glyph::MAXIMIZE
            },
            if maximized { "Restaurer" } else { "Agrandir" },
            1,
        ),
        (glyph::CLOSE, "Fermer", 2),
    ];
    for (k, (g, tip, which)) in items.into_iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(full.right() - BUTTON_W * (3 - k) as f32, full.top()),
            vec2(BUTTON_W, HEIGHT - 4.0),
        );
        let r = ui.interact(rect, Id::unique(("winbtn", which)), Sense::click());
        let close = which == 2;
        let t = ctx.animate_bool_with_time(r.id.with("h"), r.hovered(), 0.08);
        let mut fg = pal.text;
        if t > 0.0 {
            let bg = if close {
                Color32::from_rgb(196, 43, 28)
            } else {
                pal.hover
            };
            ui.painter().rect_filled(
                rect,
                if k == 2 {
                    CornerRadius {
                        ne: 8,
                        ..CornerRadius::ZERO
                    }
                } else {
                    CornerRadius::ZERO
                },
                bg.gamma_multiply(t),
            );
            if close {
                fg = Color32::WHITE;
            }
        }
        ui.painter()
            .text(rect.center(), Align2::CENTER_CENTER, g, icon_font(10.0), fg);
        if r.on_hover_text(tip).clicked() {
            match which {
                0 => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                1 => ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized)),
                _ => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }
}
