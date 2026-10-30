//! Vue Grandes icônes : tuiles virtualisées avec miniatures shell.

use eframe::egui::{self, Align2, Color32, FontId, Id, Rect, ScrollArea, Sense, Ui, pos2, vec2};
use fx_app::{Pane, PaneId};
use fx_core::EntryFlags;

use super::{Fx, items, list};
use crate::theme::{glyph, icon_font};

fn tile(f: &Fx<'_>) -> (f32, f32, f32) {
    let icon = f32::from(f.state.settings.grid_size);
    (icon + 36.0, icon + 54.0, icon)
}

/// Nombre de tuiles par ligne (navigation au clavier).
pub(crate) fn columns_for(f: &Fx<'_>, pane: PaneId) -> usize {
    let (tw, _, _) = tile(f);
    let w = f.vs.scroll.get(&pane).map_or(800.0, |s| s.width);
    ((w - 16.0) / tw).floor().max(1.0) as usize
}

pub fn draw(ui: &mut Ui, f: &mut Fx<'_>, pane: &Pane) {
    let Some(listing) = &pane.listing else { return };
    let pal = f.pal;
    let (tw, th, icon) = tile(f);
    list::select_all_strip(ui, f, pane);
    let n = listing.len();
    let per_row = ((ui.available_width() - 16.0) / tw).floor().max(1.0) as usize;
    let rows = n.div_ceil(per_row);

    let sc = f.vs.scroll.entry(pane.id).or_default();
    if sc.shown.as_ref() != Some(pane.location()) {
        sc.shown = Some(pane.location().clone());
        sc.request = Some(0.0);
    }
    if let Some(pos) = sc.ensure.take() {
        let top = (pos / per_row) as f32 * th;
        if top < sc.offset {
            sc.request = Some(top);
        } else if top + th > sc.offset + sc.view_h {
            sc.request = Some(top + th - sc.view_h);
        }
    }
    let mut area = ScrollArea::vertical()
        .id_salt(("grid", pane.id.0))
        .auto_shrink([false, false]);
    if let Some(off) = sc.request.take() {
        area = area.vertical_scroll_offset(off);
    }
    let out = area.show_viewport(ui, |ui, viewport| {
        ui.set_height((rows as f32 * th + 40.0).max(viewport.height()));
        let origin = ui.max_rect().top();
        let left = ui.max_rect().left() + 8.0;
        let width = ui.max_rect().width();
        let vis = Rect::from_min_size(
            pos2(ui.max_rect().left(), origin + viewport.top()),
            vec2(width, viewport.height()),
        );
        let bg = ui.interact(vis, Id::unique(("bg", pane.id.0)), Sense::click_and_drag());
        list::background(ui, f, pane.id, &bg);

        let first_row = (viewport.top() / th).floor().max(0.0) as usize;
        let last_row = ((viewport.bottom() / th).ceil() as usize).min(rows);
        for r in first_row..last_row {
            for c in 0..per_row {
                let pos = r * per_row + c;
                let Some(e) = listing.at(pos) else { break };
                let rect = Rect::from_min_size(
                    pos2(left + c as f32 * tw, origin + r as f32 * th),
                    vec2(tw - 6.0, th - 6.0),
                );
                let resp = ui.interact(
                    rect,
                    Id::unique(("tile", pane.id.0, e.id)),
                    Sense::click_and_drag(),
                );
                let selected = pane.selection.contains(e.id);
                let hover =
                    ui.ctx()
                        .animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.08);
                // Sélection en fondu (100 ms).
                let sel = ui
                    .ctx()
                    .animate_bool_with_time(resp.id.with("s"), selected, 0.1);
                if sel > 0.0 {
                    let fill = if resp.hovered() {
                        pal.selection_hover
                    } else {
                        pal.selection
                    };
                    crate::widgets::glow(
                        ui,
                        rect,
                        10.0,
                        pal.accent.gamma_multiply(0.25 * sel),
                        12.0,
                    );
                    ui.painter()
                        .rect_filled(rect, 8.0, fill.gamma_multiply(sel));
                    ui.painter().rect_stroke(
                        rect,
                        8.0,
                        egui::Stroke::new(1.0, pal.accent.gamma_multiply(0.7 * sel)),
                        egui::StrokeKind::Inside,
                    );
                    crate::widgets::neon_line(
                        ui,
                        (rect.left() + 10.0)..=(rect.right() - 10.0),
                        rect.top(),
                        1.5,
                        pal.accent.gamma_multiply(sel),
                    );
                }
                if !selected && hover > 0.0 {
                    ui.painter()
                        .rect_filled(rect, 8.0, pal.hover.gamma_multiply(hover));
                }
                let dim = e.meta.flags.contains(EntryFlags::HIDDEN);
                let tint = if dim {
                    Color32::WHITE.gamma_multiply(0.55)
                } else {
                    Color32::WHITE
                };
                let icon_box = Rect::from_center_size(
                    pos2(rect.center().x, rect.top() + 8.0 + icon / 2.0),
                    vec2(icon, icon),
                );
                let size_px = (icon * ui.ctx().pixels_per_point()).round() as u16;
                match items::texture(f, listing, &e, size_px.max(32), true) {
                    Some((tex, px)) => {
                        // Conserve les proportions de la miniature dans la case.
                        let ts = px / ui.ctx().pixels_per_point();
                        let scale = (icon / ts.x.max(1.0)).min(icon / ts.y.max(1.0)).min(1.0);
                        let fit = Rect::from_center_size(icon_box.center(), ts * scale);
                        ui.painter().image(
                            tex,
                            fit,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            tint,
                        );
                        items::sync_overlay(ui, f, listing, &e, icon_box);
                    }
                    None => {
                        let (g, col) = if e.meta.is_dir() {
                            (glyph::FOLDER, pal.folder)
                        } else {
                            (glyph::FILE, pal.text3)
                        };
                        ui.painter().text(
                            icon_box.center(),
                            Align2::CENTER_CENTER,
                            g,
                            icon_font(icon * 0.7),
                            col,
                        );
                    }
                }
                let name = items::display_name(f, listing, &e);
                f.vs.fallbacks.scan(&name);
                let label = Rect::from_min_max(
                    pos2(rect.left() + 4.0, icon_box.bottom() + 4.0),
                    pos2(rect.right() - 4.0, rect.bottom() - 2.0),
                );
                if !items::rename_editor(
                    ui,
                    f,
                    // Hauteur d'une ligne de 12,5 points : centre du champ = centre de la
                    // première ligne du nom.
                    Rect::from_min_size(label.min, vec2(label.width(), 12.5 * 1.25)),
                    FontId::proportional(12.5),
                    pane.id,
                    e.id,
                    &fx_core::wtf8::display(e.name),
                    true,
                ) {
                    let mut job = egui::text::LayoutJob::simple(
                        name,
                        FontId::proportional(12.5),
                        if dim { pal.text2 } else { pal.text },
                        label.width(),
                    );
                    job.halign = egui::Align::Center;
                    job.wrap.max_rows = 2;
                    // Coupure entre les mots, comme l'Explorateur (egui coupe quand même un
                    // mot trop long pour la tuile, de préférence après un tiret ou un point).
                    job.wrap.break_anywhere = false;
                    let galley = ui.painter().layout_job(job);
                    ui.painter()
                        .galley(pos2(label.center().x, label.top()), galley, pal.text);
                }
                // Case de sélection sur le coin de l'icône, au survol de la tuile.
                let check_center = pos2(
                    icon_box.left() + 9.0_f32.min(icon / 2.0),
                    icon_box.top() + 9.0_f32.min(icon / 2.0),
                );
                let check = ui.interact(
                    Rect::from_center_size(check_center, vec2(20.0, 20.0)),
                    resp.id.with("check"),
                    Sense::click(),
                );
                if resp.contains_pointer() || check.hovered() {
                    let state = if selected {
                        crate::widgets::Check::On
                    } else {
                        crate::widgets::Check::Off
                    };
                    crate::widgets::check_box(ui, check_center, state, check.hovered(), &pal);
                }
                if check.clicked() {
                    items::toggle_check(f, pane.id, pos);
                }
                items::interact(ui, f, pane.id, listing, pos, &e, &resp, selected, None);
            }
        }
        list::band_with(ui, f, pane.id, &bg, origin, viewport, |band| {
            let mut out = Vec::new();
            let r0 = (band.top() / th).floor().max(0.0) as usize;
            let r1 = ((band.bottom() / th).ceil() as usize).min(rows);
            for r in r0..r1 {
                for c in 0..per_row {
                    let x0 = 8.0 + c as f32 * tw;
                    let cell = Rect::from_min_size(
                        pos2(left - 8.0 + x0, r as f32 * th),
                        vec2(tw - 6.0, th - 6.0),
                    );
                    if cell.intersects(band) && r * per_row + c < n {
                        out.push(r * per_row + c);
                    }
                }
            }
            out
        });
    });
    let sc = f.vs.scroll.entry(pane.id).or_default();
    sc.offset = out.state.clamped_offset().y;
    sc.view_h = out.inner_rect.height();
    sc.width = out.inner_rect.width();
}
