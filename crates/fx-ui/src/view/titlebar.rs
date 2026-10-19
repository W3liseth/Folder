//! Barre de titre maison avec onglets (comme Files et l'Explorateur de Windows 11).

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Frame, Id, Rect, Sense, Ui, ViewportCommand, pos2,
    vec2,
};
use fx_app::{Location, Msg};
use fx_commands::CommandId;

use super::Fx;
use crate::theme::{glyph, icon_font, semibold};
use crate::widgets;

const HEIGHT: f32 = 40.0;
const BUTTON_W: f32 = 46.0;

/// Charge utile du glisser-déposer d'onglet.
#[derive(Clone, Copy)]
struct TabDrag(usize);

pub fn draw(ui: &mut Ui, f: &mut Fx<'_>, maximized: bool) {
    let pal = f.pal;
    egui::Panel::top("titlebar")
        .exact_size(HEIGHT)
        .show_separator_line(false)
        .frame(Frame::new().fill(pal.base))
        .show(ui, |ui| {
            let full = ui.max_rect();
            let buttons_w = BUTTON_W * 3.0;
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
fn tabs(ui: &mut Ui, f: &mut Fx<'_>, area: Rect) -> f32 {
    let pal = f.pal;
    let n = f.state.tabs.len().max(1);
    let w = (area.width() / n as f32).clamp(110.0, 230.0);
    let mut x = area.left();
    let dragging = egui::DragAndDrop::has_payload_of_type::<TabDrag>(ui.ctx());
    // Onglets disparus : on oublie leur date d'apparition.
    let alive: Vec<u64> = f.state.tabs.iter().map(|t| t.id).collect();
    f.vs.tab_born.retain(|id, _| alive.contains(id));
    for (i, tab) in f.state.tabs.iter().enumerate() {
        let id = Id::unique(("tab", tab.id));
        // Nouvel onglet : il s'élargit et apparaît en fondu (160 ms) ; les autres glissent
        // vers leur nouvelle place (ouverture, fermeture, déplacement).
        let first_frame = f.vs.frames == 0;
        let born = *f.vs.tab_born.entry(tab.id).or_insert_with(|| {
            if first_frame {
                std::time::Instant::now() - std::time::Duration::from_secs(1)
            } else {
                std::time::Instant::now()
            }
        });
        let grow = crate::widgets::ease_out(born.elapsed().as_secs_f32() / 0.16);
        if grow < 1.0 {
            ui.ctx().request_repaint();
        }
        let tx = ui.ctx().animate_value_with_time(id.with("x"), x, 0.16);
        let rect = Rect::from_min_size(
            pos2(tx, area.top()),
            vec2((w - 2.0) * grow.max(0.05), area.height()),
        );
        x += w * grow;
        let active = i == f.state.active_tab;
        let mut tab_ui = ui.new_child(egui::UiBuilder::new().max_rect(area));
        if grow < 1.0 {
            tab_ui.set_opacity(grow);
        }
        let ui = &mut tab_ui;
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        let hovered = resp.hovered() || resp.contains_pointer();
        let t = ui.ctx().animate_bool_with_time(id.with("h"), hovered, 0.1);
        let radius = CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        };
        if active {
            ui.painter().rect_filled(rect, radius, pal.layer);
            widgets::neon_line(
                ui,
                (rect.left() + 10.0)..=(rect.right() - 10.0),
                rect.top(),
                2.0,
                pal.accent,
            );
        } else if t > 0.0 {
            ui.painter().rect_filled(
                rect.shrink2(vec2(0.0, 3.0)).translate(vec2(0.0, -1.0)),
                6.0,
                pal.hover.gamma_multiply(t),
            );
        } else if i + 1 != f.state.active_tab && i + 1 < f.state.tabs.len() {
            // Séparateur discret entre onglets inactifs.
            ui.painter().vline(
                rect.right() + 1.0,
                rect.y_range().shrink(9.0),
                egui::Stroke::new(1.0, pal.stroke_strong),
            );
        }

        let loc = tab.active_pane().location();
        let g = match loc {
            Location::Home => glyph::HOME,
            Location::Search { .. } => glyph::SEARCH,
            Location::Tag(_) => glyph::TAG,
            Location::Dir(p) if p.parent().is_none() => glyph::DRIVE,
            Location::Dir(_) => glyph::FOLDER,
            Location::RecycleBin => glyph::RECYCLE,
            Location::Network(_) => glyph::NETWORK,
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
        let title = tab.title();
        f.vs.fallbacks.scan(&title);
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
        // Fondu du texte sous le bouton de fermeture.
        let close =
            Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(22.0, 22.0));
        if active || hovered {
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

        if resp.clicked() {
            f.send(Msg::SelectTab(i));
        }
        if resp.middle_clicked() {
            f.send(Msg::CloseTab(i));
        }
        if resp.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), TabDrag(i));
        }
        if dragging && resp.contains_pointer() {
            ui.painter().rect_stroke(
                rect,
                radius,
                egui::Stroke::new(1.0, pal.accent),
                egui::StrokeKind::Inside,
            );
            if let Some(TabDrag(from)) = resp.dnd_release_payload::<TabDrag>().map(|p| *p) {
                f.send(Msg::MoveTab { from, to: i });
            }
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
        resp.on_hover_text(match tab.active_pane().location() {
            Location::Dir(p) => p.display().to_string(),
            other => other.title(),
        });
    }
    x
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
