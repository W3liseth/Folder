//! Notifications brèves (bas de la fenêtre), avec fondu.

use std::time::Duration;

use eframe::egui::{self, Align2, Area, Frame, Id, Order, RichText, pos2};
use fx_app::Msg;
use fx_app::state::ToastKind;

use super::Fx;
use crate::theme::{glyph, icon_font};

const LIFETIME: Duration = Duration::from_millis(3500);
const FADE: f32 = 0.25;

pub fn draw(ctx: &egui::Context, f: &mut Fx<'_>) {
    if f.state.toasts.is_empty() {
        return;
    }
    let pal = f.pal;
    let screen = ctx.content_rect();
    let mut y = screen.bottom() - 48.0;
    for t in f.state.toasts.iter().rev() {
        let age = t.at.elapsed();
        if age >= LIFETIME {
            f.send(Msg::DismissToast(t.id));
            continue;
        }
        let a = age.as_secs_f32();
        let left = LIFETIME.as_secs_f32() - a;
        let alpha = (a / FADE).min(1.0).min(left / FADE).clamp(0.0, 1.0);
        let slide = (1.0 - (a / FADE).min(1.0)) * 12.0;
        let (g, c) = match t.kind {
            ToastKind::Info => (glyph::INFO, pal.accent),
            ToastKind::Success => (glyph::SUCCESS, pal.success),
            ToastKind::Error => (glyph::ERROR, pal.danger),
        };
        let resp = Area::new(Id::unique(("toast", t.id)))
            .order(Order::Tooltip)
            .fixed_pos(pos2(screen.center().x, y + slide))
            .pivot(Align2::CENTER_BOTTOM)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                Frame::popup(ui.style())
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(g).font(icon_font(15.0)).color(c));
                            ui.label(&t.text);
                        });
                    })
            });
        if resp.response.clicked() {
            f.send(Msg::DismissToast(t.id));
        }
        y -= resp.response.rect.height() + 8.0;
        // Fondu : quelques images supplémentaires, puis plus rien jusqu'à l'expiration.
        if a < FADE || left < FADE {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_secs_f32(left - FADE));
        }
    }
}
