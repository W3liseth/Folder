//! Centre d'état : bouton de la barre de navigation (à côté de Rechercher et de la palette)
//! et son menu déroulant — opérations en cours (progression, annulation) puis historique des
//! opérations terminées, effaçable.

use std::f32::consts::TAU;
use std::time::Duration;

use eframe::egui::{
    self, Align, Align2, Area, Frame, Id, Layout, Order, Rect, RichText, Shape, Stroke, Ui, pos2,
    vec2,
};
use fx_app::ops::{OpRecord, OpStatus, Operation};
use fx_app::{Msg, state::AppState};
use fx_commands::CommandId;

use super::Fx;
use crate::theme::{Palette, glyph, icon_font};
use crate::widgets;

/// Bouton du centre d'état. Pendant une opération, un anneau autour du glyphe montre la
/// progression moyenne ; un point rouge signale un échec pas encore vu.
pub fn button(ui: &mut Ui, f: &mut Fx<'_>, size: egui::Vec2) {
    let pal = f.pal;
    let cmd = CommandId::StatusCenter;
    let open = f.state.show_status_center;
    let running = f.state.ops.len();
    let tip = match running {
        0 => cmd.label().to_owned(),
        1 => format!("{} — 1 opération en cours", cmd.label()),
        n => format!("{} — {n} opérations en cours", cmd.label()),
    };
    let r = widgets::icon_button(ui, &pal, cmd.glyph(), &tip, true, size);
    f.vs.status_anchor = Some(r.rect);
    if open {
        ui.painter()
            .rect_filled(r.rect.shrink(1.0), 6.0, pal.selection);
        ui.painter().text(
            r.rect.center(),
            Align2::CENTER_CENTER,
            cmd.glyph(),
            icon_font(14.0),
            pal.accent,
        );
    }
    if running > 0 {
        ring(ui, &pal, r.rect.center(), 12.5, progress(f.state));
    } else if f.state.op_history.has_unseen_failure() {
        let c = r.rect.center() + vec2(7.0, -7.0);
        ui.painter().circle_filled(c, 4.0, pal.danger);
        ui.painter()
            .circle_stroke(c, 4.0, Stroke::new(1.5, pal.layer));
    }
    if r.clicked() {
        f.cmd(cmd);
    }
}

/// Progression moyenne des opérations en cours dont le total est connu.
fn progress(state: &AppState) -> Option<f32> {
    let (sum, n) = state
        .ops
        .iter()
        .filter_map(Operation::fraction)
        .fold((0.0, 0u32), |(s, n), p| (s + p, n + 1));
    (n > 0).then(|| sum / n as f32)
}

/// Anneau de progression (piste + arc depuis midi). Sans total connu, la piste seule, en
/// accent : pas d'animation continue (repaint réactif).
fn ring(ui: &Ui, pal: &Palette, c: egui::Pos2, radius: f32, frac: Option<f32>) {
    let p = ui.painter();
    let Some(frac) = frac else {
        p.circle_stroke(c, radius, Stroke::new(2.0, pal.accent.gamma_multiply(0.45)));
        return;
    };
    p.circle_stroke(c, radius, Stroke::new(2.0, pal.stroke_strong));
    let frac = frac.clamp(0.04, 1.0);
    let n = ((40.0 * frac) as usize).max(3);
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let a = -TAU / 4.0 + TAU * frac * i as f32 / n as f32;
            c + radius * vec2(a.cos(), a.sin())
        })
        .collect();
    p.add(Shape::line(pts, Stroke::new(2.0, pal.accent)));
}

/// Menu déroulant, ancré sous le bouton.
pub fn draw(ctx: &egui::Context, f: &mut Fx<'_>) {
    if !f.state.show_status_center {
        return;
    }
    let pal = f.pal;
    let screen = ctx.content_rect();
    let anchor = f.vs.status_anchor.unwrap_or_else(|| {
        Rect::from_min_size(
            pos2(screen.right() - 48.0, screen.top() + 40.0),
            vec2(36.0, 34.0),
        )
    });
    // Grand panneau (titres de chemins longs, historique) borné par la fenêtre.
    let width = (screen.width() - 32.0).clamp(320.0, 540.0);
    let max_h = (screen.bottom() - anchor.bottom() - 80.0).clamp(200.0, 760.0);
    let area = Area::new(Id::unique("status-center"))
        .order(Order::Foreground)
        .fixed_pos(pos2(anchor.right(), anchor.bottom() + 6.0))
        .pivot(Align2::RIGHT_TOP)
        .show(ctx, |ui| {
            widgets::menu_appear(ui);
            Frame::popup(ui.style()).inner_margin(14).show(ui, |ui| {
                ui.set_width(width);
                header(ui, f);
                ui.add_space(8.0);
                let running = &f.state.ops;
                let history = &f.state.op_history;
                if running.is_empty() && history.is_empty() {
                    ui.add_space(18.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(glyph::HISTORY)
                                .font(icon_font(26.0))
                                .color(pal.text3),
                        );
                        ui.add_space(6.0);
                        ui.label(RichText::new("Aucune opération").color(pal.text2));
                        ui.label(
                            RichText::new(
                                "Les copies, déplacements et suppressions apparaîtront ici.",
                            )
                            .size(12.0)
                            .color(pal.text3),
                        );
                    });
                    ui.add_space(18.0);
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(max_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if !running.is_empty() {
                            section(ui, &pal, "En cours");
                            for op in running.iter().rev() {
                                running_card(ui, f, op);
                                ui.add_space(6.0);
                            }
                        }
                        if !history.is_empty() {
                            if !running.is_empty() {
                                ui.add_space(6.0);
                            }
                            section(ui, &pal, "Historique");
                            let today =
                                fx_win::time::filetime_to_local(fx_win::time::now_filetime());
                            for r in history.iter() {
                                history_row(ui, f, r, today);
                            }
                        }
                    });
            });
        });
    // Clic hors du menu (et hors du bouton, qui le referme lui-même) : fermeture.
    let rect = area.response.rect;
    if ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| !rect.contains(p) && !anchor.contains(p))
    }) {
        f.send(Msg::CloseOverlays);
    }
}

fn header(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Centre d'état").font(crate::theme::semibold(15.0)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let cmd = CommandId::ClearOperationHistory;
            let enabled = !f.state.op_history.is_empty();
            let text = RichText::new("Effacer l'historique")
                .size(12.5)
                .color(if enabled { pal.accent } else { pal.text3 });
            let r = ui
                .add_enabled(enabled, egui::Button::new(text).frame(false))
                .on_hover_text("Retirer toutes les opérations terminées");
            if r.clicked() {
                f.cmd(cmd);
            }
        });
    });
}

fn section(ui: &mut Ui, pal: &Palette, title: &str) {
    ui.label(RichText::new(title).size(12.0).color(pal.text2));
    ui.add_space(4.0);
}

fn running_card(ui: &mut Ui, f: &mut Fx<'_>, op: &Operation) {
    let pal = f.pal;
    Frame::new()
        .fill(pal.layer)
        .corner_radius(6)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(glyph::COPY)
                        .font(icon_font(14.0))
                        .color(pal.accent),
                );
                ui.add(egui::Label::new(RichText::new(&op.title).strong()).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(
                        ui,
                        &pal,
                        glyph::CANCEL,
                        "Annuler l'opération",
                        true,
                        vec2(24.0, 22.0),
                    )
                    .clicked()
                    {
                        f.send(Msg::CancelOp(op.id));
                    }
                });
            });
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 4.0), egui::Sense::hover());
            widgets::progress(ui, rect, op.fraction(), pal.accent, pal.stroke_strong);
            let elapsed = op.started.elapsed().as_secs_f32();
            let mut detail = op.current.clone();
            if let Some(p) = op.fraction() {
                detail = format!("{:.0} %  ·  {detail}", p * 100.0);
                if p > 0.02 && elapsed > 1.0 {
                    detail = format!("{detail}  ·  environ {}", eta(elapsed / p - elapsed));
                }
            }
            ui.add(egui::Label::new(RichText::new(detail).size(11.5).color(pal.text2)).truncate());
        });
}

fn history_row(ui: &mut Ui, f: &mut Fx<'_>, r: &OpRecord, today: Option<fx_win::time::LocalTime>) {
    let pal = f.pal;
    let (g, c, status) = match &r.status {
        OpStatus::Done => (glyph::SUCCESS, pal.success, "Terminé"),
        OpStatus::Cancelled => (glyph::CANCEL, pal.text3, "Annulé"),
        OpStatus::Failed(_) => (glyph::ERROR, pal.danger, "Échec"),
        OpStatus::Running => (glyph::COPY, pal.accent, "En cours"),
    };
    Frame::new()
        .inner_margin(egui::Margin::symmetric(6, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(g).font(icon_font(14.0)).color(c));
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width() - 30.0);
                    ui.add(egui::Label::new(RichText::new(&r.title)).truncate());
                    let mut meta = format!("{status}  ·  {}", when(r.ended, today));
                    if r.duration >= Duration::from_secs(1) {
                        meta = format!("{meta}  ·  {}", duration(r.duration));
                    }
                    ui.label(RichText::new(meta).size(11.5).color(pal.text2));
                    if let OpStatus::Failed(e) = &r.status {
                        ui.label(RichText::new(e).size(11.5).color(pal.danger));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    if widgets::icon_button(
                        ui,
                        &pal,
                        glyph::CANCEL,
                        "Retirer de l'historique",
                        true,
                        vec2(24.0, 22.0),
                    )
                    .clicked()
                    {
                        f.send(Msg::DismissOp(r.id));
                    }
                });
            });
        });
}

/// Heure de fin : « 14:32 » aujourd'hui, « 27/10 14:32 » sinon.
fn when(ended: i64, today: Option<fx_win::time::LocalTime>) -> String {
    let Some(t) = fx_win::time::filetime_to_local(ended) else {
        return String::new();
    };
    let same_day = today.is_some_and(|d| (d.year, d.month, d.day) == (t.year, t.month, t.day));
    if same_day {
        format!("{:02}:{:02}", t.hour, t.minute)
    } else {
        format!("{:02}/{:02} {:02}:{:02}", t.day, t.month, t.hour, t.minute)
    }
}

fn duration(d: Duration) -> String {
    match d.as_secs() {
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => format!("{} min {:02} s", s / 60, s % 60),
        s => format!("{} h {:02} min", s / 3600, (s % 3600) / 60),
    }
}

fn eta(secs: f32) -> String {
    match secs as u64 {
        s if s < 60 => format!("{s} s restantes"),
        s if s < 3600 => format!("{} min restantes", s / 60 + 1),
        s => format!("{} h {:02} restantes", s / 3600, (s % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration(Duration::from_secs(5)), "5 s");
        assert_eq!(duration(Duration::from_secs(125)), "2 min 05 s");
        assert_eq!(duration(Duration::from_secs(3720)), "1 h 02 min");
    }
}
