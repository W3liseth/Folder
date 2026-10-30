//! Barre d'état (le centre d'état est dans la barre de navigation, cf. `statuscenter`).

use eframe::egui::{self, Align, Align2, Frame, Layout, Margin, RichText, Ui, vec2};
use fx_app::Location;
use fx_commands::CommandId;
use fx_store::Layout as View;

use super::Fx;
use crate::theme::{glyph, icon_font};
use crate::widgets;

pub fn draw_bar(ui: &mut Ui, f: &mut Fx<'_>) {
    // Ancre du menu Git : recalculée à chaque image (aucune si le bouton est absent).
    f.vs.git_anchor = None;
    let pal = f.pal;
    egui::Panel::bottom("status")
        .exact_size(30.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(pal.base).inner_margin(Margin {
            left: 6,
            right: 10,
            top: 0,
            bottom: 0,
        }))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let cmd = CommandId::Settings;
                let tip = format!("{} ({})", cmd.label(), f.shortcut(cmd));
                if widgets::icon_button(ui, &pal, cmd.glyph(), &tip, true, vec2(28.0, 24.0))
                    .clicked()
                {
                    f.cmd(cmd);
                }
                widgets::vsep(ui, &pal);
                let pane = f.state.pane();
                let small = |t: String| RichText::new(t).size(12.0).color(pal.text2);
                if let Some(l) = &pane.listing
                    && !matches!(pane.location(), Location::Home)
                {
                    let n = l.len();
                    ui.label(small(format!(
                        "{} élément{}",
                        thousands(n as u64),
                        if n > 1 { "s" } else { "" }
                    )));
                    let sel = pane.selected_paths();
                    if !sel.is_empty() {
                        let bytes: u64 = l
                            .visible_ids()
                            .iter()
                            .filter(|&&id| pane.selection.contains(id))
                            .map(|&id| l.view(id).meta)
                            .filter(|m| !m.is_dir())
                            .map(|m| m.size)
                            .sum();
                        let size = if bytes > 0 {
                            format!("  ({})", super::sidebar::human(bytes))
                        } else {
                            String::new()
                        };
                        ui.label(small(format!(
                            "·  {} sélectionné{}{size}",
                            sel.len(),
                            if sel.len() > 1 { "s" } else { "" }
                        )));
                    }
                    if pane.is_loading() {
                        ui.add(egui::Spinner::new().size(12.0));
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (layout, g, cmd) in [
                        (View::Grid, glyph::GRID, CommandId::LayoutGrid),
                        (View::Details, glyph::DETAILS, CommandId::LayoutDetails),
                    ] {
                        let on = pane.layout == layout;
                        let r =
                            widgets::icon_button(ui, &pal, g, cmd.label(), true, vec2(28.0, 24.0));
                        if on {
                            ui.painter().rect_filled(
                                r.rect.shrink(3.0),
                                4.0,
                                pal.hover.gamma_multiply(1.5),
                            );
                            ui.painter().text(
                                r.rect.center(),
                                Align2::CENTER_CENTER,
                                g,
                                icon_font(13.0),
                                pal.accent,
                            );
                        }
                        if r.clicked() {
                            f.cmd(cmd);
                        }
                    }
                    ui.add_space(8.0);
                    // Dépôt Git : branche active, menu pour en changer.
                    if let Some(b) = f.state.pane().git_branch.clone() {
                        widgets::vsep(ui, &pal);
                        ui.add_space(2.0);
                        super::gitmenu::button(ui, f, &b);
                        ui.add_space(4.0);
                    }
                    if f.state.settings.show_hidden {
                        ui.label(
                            RichText::new(glyph::HIDDEN)
                                .font(icon_font(12.0))
                                .color(pal.text3),
                        )
                        .on_hover_text("Éléments masqués affichés");
                    }
                });
            });
        });
}

pub(crate) fn thousands(n: u64) -> String {
    let mut s = String::new();
    let _ = fx_core::format::thousands(&mut s, n);
    s
}
