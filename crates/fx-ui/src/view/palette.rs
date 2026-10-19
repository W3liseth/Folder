//! Palette de commandes (Ctrl+Maj+P) : recherche floue dans toutes les commandes, ou chemin.

use std::path::PathBuf;

use eframe::egui::{
    self, Align2, Area, Frame, Id, Key, Order, RichText, Sense, TextEdit, pos2, vec2,
};
use fx_app::Msg;
use fx_commands::{CommandId, fuzzy_score};

use super::Fx;
use crate::theme::{glyph, icon_font};

enum Entry {
    Command(CommandId),
    Path(PathBuf),
}

pub fn draw(ctx: &egui::Context, f: &mut Fx<'_>) {
    if !f.state.show_palette {
        if !f.vs.palette_query.is_empty() {
            f.vs.palette_query.clear();
            f.vs.palette_index = 0;
        }
        return;
    }
    let pal = f.pal;
    let cctx = f.state.command_context();
    let q = f.vs.palette_query.trim().to_owned();
    let mut entries: Vec<(i32, Entry)> = Vec::new();
    if q.contains(":\\") || q.starts_with("\\\\") || q.starts_with('%') {
        entries.push((i32::MAX, Entry::Path(PathBuf::from(&q))));
    }
    for &id in CommandId::ALL {
        if id == CommandId::CommandPalette {
            continue;
        }
        let score = if q.is_empty() {
            Some(0)
        } else {
            fuzzy_score(id.label(), &q)
        };
        if let Some(s) = score {
            entries.push((
                s + if id.is_enabled(&cctx) { 50 } else { 0 },
                Entry::Command(id),
            ));
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.0));
    entries.truncate(12);
    if f.vs.palette_index >= entries.len() {
        f.vs.palette_index = entries.len().saturating_sub(1);
    }

    let screen = ctx.content_rect();
    let mut run: Option<usize> = None;
    let area = Area::new(Id::unique("palette"))
        .order(Order::Foreground)
        .fixed_pos(pos2(screen.center().x, screen.top() + 90.0))
        .pivot(Align2::CENTER_TOP)
        .show(ctx, |ui| {
            Frame::popup(ui.style()).inner_margin(10).show(ui, |ui| {
                ui.set_width(600.0);
                let id = Id::unique("palette-input");
                let r = ui.add(
                    TextEdit::singleline(&mut f.vs.palette_query)
                        .id(id)
                        .hint_text("Rechercher une commande ou saisir un chemin…")
                        .desired_width(f32::INFINITY)
                        .margin(vec2(10.0, 8.0)),
                );
                r.request_focus();
                if r.changed() {
                    f.vs.palette_index = 0;
                }
                ui.input(|i| {
                    if i.key_pressed(Key::ArrowDown) {
                        f.vs.palette_index =
                            (f.vs.palette_index + 1).min(entries.len().saturating_sub(1));
                    }
                    if i.key_pressed(Key::ArrowUp) {
                        f.vs.palette_index = f.vs.palette_index.saturating_sub(1);
                    }
                    if i.key_pressed(Key::Enter) {
                        run = Some(f.vs.palette_index);
                    }
                });
                ui.add_space(6.0);
                for (i, (_, e)) in entries.iter().enumerate() {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                    let selected = i == f.vs.palette_index;
                    if selected {
                        ui.painter().rect_filled(rect, 4.0, pal.selection);
                        ui.painter().rect_filled(
                            egui::Rect::from_center_size(
                                pos2(rect.left() + 1.5, rect.center().y),
                                vec2(3.0, 16.0),
                            ),
                            1.5,
                            pal.accent,
                        );
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 4.0, pal.hover);
                    }
                    let (g, label, right, enabled) = match e {
                        Entry::Command(c) => (
                            c.glyph(),
                            c.label().to_owned(),
                            f.shortcut(*c),
                            c.is_enabled(&cctx),
                        ),
                        Entry::Path(p) => (
                            glyph::FOLDER,
                            format!("Ouvrir « {} »", p.display()),
                            String::new(),
                            true,
                        ),
                    };
                    let color = if enabled { pal.text } else { pal.text3 };
                    ui.painter().text(
                        pos2(rect.left() + 20.0, rect.center().y),
                        Align2::CENTER_CENTER,
                        g,
                        icon_font(14.0),
                        color,
                    );
                    ui.painter().text(
                        pos2(rect.left() + 42.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(14.0),
                        color,
                    );
                    if let Entry::Command(c) = e {
                        ui.painter().text(
                            pos2(rect.right() - 120.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            c.group().label(),
                            egui::FontId::proportional(11.5),
                            pal.text3,
                        );
                    }
                    ui.painter().text(
                        pos2(rect.right() - 10.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        right,
                        egui::FontId::proportional(12.0),
                        pal.text2,
                    );
                    if resp.clicked() {
                        run = Some(i);
                    }
                    if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                        f.vs.palette_index = i;
                    }
                }
                if entries.is_empty() {
                    ui.label(RichText::new("Aucune commande").color(pal.text3));
                }
            });
        });
    let palette_rect = area.response.rect;
    if let Some(i) = run
        && let Some((_, e)) = entries.into_iter().nth(i)
    {
        f.send(Msg::TogglePalette);
        match e {
            Entry::Command(c) => f.cmd(c),
            Entry::Path(p) => f.send(Msg::OpenPath(p)),
        }
    }
    // Clic hors de la palette : fermeture.
    if ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| !palette_rect.contains(p))
    }) {
        f.send(Msg::TogglePalette);
    }
}
