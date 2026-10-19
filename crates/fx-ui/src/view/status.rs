//! Barre d'état et centre d'opérations (copies, déplacements… en cours).

use eframe::egui::{
    self, Align, Align2, Area, Frame, Id, Layout, Margin, Order, RichText, Ui, pos2, vec2,
};
use fx_app::ops::OpStatus;
use fx_app::{Location, Msg};
use fx_commands::CommandId;
use fx_store::Layout as View;

use super::Fx;
use crate::theme::{glyph, icon_font};
use crate::widgets;

pub fn draw_bar(ui: &mut Ui, f: &mut Fx<'_>) {
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
                if let Some(b) = &pane.git_branch {
                    ui.label(
                        RichText::new(glyph::BRANCH)
                            .font(icon_font(11.0))
                            .color(pal.text2),
                    );
                    ui.label(small(b.clone()));
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
                    ops_indicator(ui, f);
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

fn ops_indicator(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    let running: Vec<&fx_app::ops::Operation> =
        f.state.ops.iter().filter(|o| o.is_running()).collect();
    let finished = f.state.ops.len() - running.len();
    if running.is_empty() && finished == 0 {
        return;
    }
    let label = match running.as_slice() {
        [] => format!("{finished} terminée{}", if finished > 1 { "s" } else { "" }),
        [one] => match one.fraction() {
            Some(p) => format!("{}  {:.0} %", short(&one.title), p * 100.0),
            None => short(&one.title),
        },
        many => format!("{} opérations en cours", many.len()),
    };
    let r = ui.add(
        egui::Button::new(
            RichText::new(label)
                .size(12.0)
                .color(if running.is_empty() {
                    pal.text2
                } else {
                    pal.accent
                }),
        )
        .frame(false),
    );
    if !running.is_empty() {
        let frac = running.iter().filter_map(|o| o.fraction()).sum::<f32>() / running.len() as f32;
        let bar = egui::Rect::from_min_size(
            pos2(r.rect.left() + 4.0, r.rect.bottom() - 3.0),
            vec2(r.rect.width() - 8.0, 2.0),
        );
        widgets::progress(
            ui,
            bar,
            (frac > 0.0).then_some(frac),
            pal.accent,
            pal.stroke,
        );
    }
    if r.clicked() {
        f.vs.status_center = !f.vs.status_center;
    }
}

fn short(s: &str) -> String {
    if s.chars().count() > 38 {
        format!("{}…", s.chars().take(37).collect::<String>())
    } else {
        s.to_owned()
    }
}

/// Centre d'opérations (panneau flottant en bas à droite).
pub fn draw_center(ctx: &egui::Context, f: &mut Fx<'_>) {
    // Ouvre automatiquement le centre quand une opération longue démarre.
    let long_running = f
        .state
        .ops
        .iter()
        .any(|o| o.is_running() && o.started.elapsed().as_millis() > 1500);
    if long_running
        && !f.vs.status_center
        && !ctx.data(|d| d.get_temp::<bool>(Id::unique("sc-auto")).unwrap_or(false))
    {
        f.vs.status_center = true;
        ctx.data_mut(|d| d.insert_temp(Id::unique("sc-auto"), true));
    }
    if f.state.running_ops() == 0 {
        ctx.data_mut(|d| d.remove::<bool>(Id::unique("sc-auto")));
    }
    if !f.vs.status_center || f.state.ops.is_empty() {
        return;
    }
    let pal = f.pal;
    let screen = ctx.content_rect();
    Area::new(Id::unique("status-center"))
        .order(Order::Foreground)
        .fixed_pos(pos2(screen.right() - 420.0, screen.bottom() - 44.0))
        .pivot(Align2::LEFT_BOTTOM)
        .show(ctx, |ui| {
            Frame::popup(ui.style()).inner_margin(14).show(ui, |ui| {
                ui.set_width(380.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Opérations").font(crate::theme::semibold(15.0)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::icon_button(
                            ui,
                            &pal,
                            glyph::CLOSE,
                            "Fermer",
                            true,
                            vec2(26.0, 24.0),
                        )
                        .clicked()
                        {
                            f.vs.status_center = false;
                        }
                        if f.state.ops.iter().any(|o| !o.is_running())
                            && ui.small_button("Effacer les terminées").clicked()
                        {
                            f.send(Msg::ClearFinishedOps);
                        }
                    });
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for op in f.state.ops.iter().rev() {
                            Frame::new()
                                .fill(pal.layer)
                                .corner_radius(6)
                                .inner_margin(10)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        let (g, c) = match &op.status {
                                            OpStatus::Running => (glyph::COPY, pal.accent),
                                            OpStatus::Done => (glyph::SUCCESS, pal.success),
                                            OpStatus::Cancelled => (glyph::CANCEL, pal.text3),
                                            OpStatus::Failed(_) => (glyph::ERROR, pal.danger),
                                        };
                                        ui.label(RichText::new(g).font(icon_font(14.0)).color(c));
                                        ui.label(RichText::new(&op.title).strong());
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if op.is_running() {
                                                    if widgets::icon_button(
                                                        ui,
                                                        &pal,
                                                        glyph::CANCEL,
                                                        "Annuler",
                                                        true,
                                                        vec2(24.0, 22.0),
                                                    )
                                                    .clicked()
                                                    {
                                                        f.send(Msg::CancelOp(op.id));
                                                    }
                                                } else if widgets::icon_button(
                                                    ui,
                                                    &pal,
                                                    glyph::CLOSE,
                                                    "Retirer",
                                                    true,
                                                    vec2(24.0, 22.0),
                                                )
                                                .clicked()
                                                {
                                                    f.send(Msg::DismissOp(op.id));
                                                }
                                            },
                                        );
                                    });
                                    if op.is_running() {
                                        let (rect, _) = ui.allocate_exact_size(
                                            vec2(ui.available_width(), 4.0),
                                            egui::Sense::hover(),
                                        );
                                        widgets::progress(
                                            ui,
                                            rect,
                                            op.fraction(),
                                            pal.accent,
                                            pal.stroke_strong,
                                        );
                                        let elapsed = op.started.elapsed().as_secs_f32();
                                        let mut detail = op.current.clone();
                                        if let Some(p) =
                                            op.fraction().filter(|p| *p > 0.02 && elapsed > 1.0)
                                        {
                                            let remain = elapsed / p - elapsed;
                                            detail =
                                                format!("{detail}  ·  environ {}", eta(remain));
                                        }
                                        ui.label(RichText::new(detail).size(11.5).color(pal.text2));
                                    } else if let OpStatus::Failed(e) = &op.status {
                                        ui.label(RichText::new(e).size(11.5).color(pal.danger));
                                    }
                                });
                            ui.add_space(6.0);
                        }
                    });
            });
        });
}

fn eta(secs: f32) -> String {
    match secs as u64 {
        s if s < 60 => format!("{s} s restantes"),
        s if s < 3600 => format!("{} min restantes", s / 60 + 1),
        s => format!("{} h {:02} restantes", s / 3600, (s % 3600) / 60),
    }
}

pub(crate) fn thousands(n: u64) -> String {
    let mut s = String::new();
    let _ = fx_core::format::thousands(&mut s, n);
    s
}
