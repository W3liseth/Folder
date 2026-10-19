//! Vue Colonnes (Miller) : chaque dossier sélectionné s'ouvre dans une colonne à droite.

use eframe::egui::{self, Align2, Color32, FontId, Id, Rect, ScrollArea, Sense, Ui, pos2, vec2};
use fx_app::Pane;
use fx_core::{DirSnapshot, Selection};

use super::{Fx, items};
use crate::theme::{glyph, icon_font};

const COL_W: f32 = 260.0;
const ROW: f32 = 28.0;

pub fn draw(ui: &mut Ui, f: &mut Fx<'_>, pane: &Pane) {
    let pal = f.pal;
    let mut lists: Vec<(&DirSnapshot, &Selection)> = Vec::new();
    if let Some(l) = &pane.listing {
        lists.push((l, &pane.selection));
    }
    for c in &pane.columns {
        if let Some(l) = &c.listing {
            lists.push((l, &c.selection));
        }
    }
    let n = lists.len();
    let scroll_right = n > f
        .vs
        .scroll
        .get(&pane.id)
        .map_or(0, |s| s.ensure.unwrap_or(0));
    let mut area = ScrollArea::horizontal()
        .id_salt(("cols", pane.id.0))
        .auto_shrink([false, false]);
    if scroll_right {
        area = area.horizontal_scroll_offset(f32::MAX);
        f.vs.scroll.entry(pane.id).or_default().ensure = Some(n);
    }
    area.show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let h = ui.available_height();
            for (c, (l, sel)) in lists.into_iter().enumerate() {
                let (col_rect, _) = ui.allocate_exact_size(vec2(COL_W, h), Sense::hover());
                ui.painter().vline(
                    col_rect.right(),
                    col_rect.y_range(),
                    egui::Stroke::new(1.0, pal.stroke),
                );
                let active = pane.active_column == c;
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(col_rect.shrink2(vec2(0.0, 4.0)))
                        .id_salt(("col", c)),
                );
                ScrollArea::vertical()
                    .id_salt(("colv", pane.id.0, c))
                    .auto_shrink([false, false])
                    .show_viewport(&mut child, |ui, viewport| {
                        let rows = l.len();
                        ui.set_height((rows as f32 * ROW).max(viewport.height()));
                        let origin = ui.max_rect().top();
                        let left = ui.max_rect().left();
                        let width = ui.max_rect().width();
                        let vis = Rect::from_min_size(
                            pos2(left, origin + viewport.top()),
                            vec2(width, viewport.height()),
                        );
                        let bg =
                            ui.interact(vis, Id::unique(("colbg", pane.id.0, c)), Sense::click());
                        if bg.clicked() {
                            f.send(fx_app::Msg::FocusPane(pane.id));
                        }
                        bg.context_menu(|ui| super::menus::background_menu(ui, f));
                        let first = (viewport.top() / ROW).floor().max(0.0) as usize;
                        let last = ((viewport.bottom() / ROW).ceil() as usize).min(rows);
                        for pos in first..last {
                            let Some(e) = l.at(pos) else { break };
                            let rect = Rect::from_min_size(
                                pos2(left, origin + pos as f32 * ROW),
                                vec2(width, ROW),
                            );
                            let resp = ui.interact(
                                rect,
                                Id::unique(("colrow", pane.id.0, c, e.id)),
                                Sense::click_and_drag(),
                            );
                            let selected = sel.contains(e.id);
                            let bgr = rect.shrink2(vec2(6.0, 1.0));
                            if selected {
                                ui.painter().rect_filled(
                                    bgr,
                                    4.0,
                                    if active {
                                        pal.selection
                                    } else {
                                        pal.hover.gamma_multiply(1.8)
                                    },
                                );
                            } else {
                                crate::widgets::hover_fill(ui, &resp, bgr, &pal, 4.0);
                            }
                            let icon_rect = Rect::from_center_size(
                                pos2(rect.left() + 22.0, rect.center().y),
                                vec2(16.0, 16.0),
                            );
                            match items::texture(f, l, &e, 20, false) {
                                Some((tex, _)) => {
                                    ui.painter().image(
                                        tex,
                                        icon_rect,
                                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                        Color32::WHITE,
                                    );
                                }
                                None => {
                                    let (g, col) = if e.meta.is_dir() {
                                        (glyph::FOLDER, pal.folder)
                                    } else {
                                        (glyph::FILE, pal.text3)
                                    };
                                    ui.painter().text(
                                        icon_rect.center(),
                                        Align2::CENTER_CENTER,
                                        g,
                                        icon_font(14.0),
                                        col,
                                    );
                                }
                            }
                            let name = items::display_name(f, l, &e);
                            f.vs.fallbacks.scan(&name);
                            let text_rect = Rect::from_x_y_ranges(
                                (rect.left() + 38.0)..=(rect.right() - 24.0),
                                rect.y_range(),
                            );
                            if !items::rename_editor(
                                ui,
                                f,
                                text_rect,
                                FontId::proportional(13.0),
                                pane.id,
                                e.id,
                                &fx_core::wtf8::display(e.name),
                                false,
                            ) {
                                ui.painter().with_clip_rect(text_rect).text(
                                    pos2(text_rect.left(), rect.center().y),
                                    Align2::LEFT_CENTER,
                                    name,
                                    FontId::proportional(13.0),
                                    pal.text,
                                );
                            }
                            if e.meta.is_dir() {
                                ui.painter().text(
                                    pos2(rect.right() - 16.0, rect.center().y),
                                    Align2::CENTER_CENTER,
                                    glyph::CHEVRON_RIGHT,
                                    icon_font(9.0),
                                    pal.text3,
                                );
                            }
                            items::interact(ui, f, pane.id, l, pos, &e, &resp, selected, Some(c));
                        }
                    });
            }
        });
    });
}
