//! Bouton « Analyser des lecteurs » (barre de navigation, à côté de la palette) et son menu :
//! cases à cocher des lecteurs, chacun analysé dans son onglet, ou le dossier affiché.

use std::path::PathBuf;

use eframe::egui::{
    self, Align, Align2, Area, FontId, Frame, Id, Layout, Order, Rect, RichText, Sense, Ui, pos2,
    vec2,
};
use fx_app::Msg;
use fx_commands::CommandId;
use fx_win::sys::DriveKind;

use super::Fx;
use super::sidebar::{drive_label, human};
use crate::theme::{glyph, icon_font, semibold};
use crate::widgets;

/// Bouton de la barre de navigation (allumé tant que le menu est ouvert).
pub fn button(ui: &mut Ui, f: &mut Fx<'_>, size: egui::Vec2) {
    let pal = f.pal;
    let cmd = CommandId::AnalyzeDrives;
    let open = f.state.show_disk_picker;
    let r = widgets::icon_button(
        ui,
        &pal,
        glyph::PIE,
        "Analyser l'espace des lecteurs",
        true,
        size,
    );
    f.vs.disk_anchor = Some(r.rect);
    if open {
        ui.painter()
            .rect_filled(r.rect.shrink(1.0), 6.0, pal.selection);
        ui.painter().text(
            r.rect.center(),
            Align2::CENTER_CENTER,
            glyph::PIE,
            icon_font(14.0),
            pal.accent,
        );
    }
    if r.clicked() {
        f.cmd(cmd);
    }
}

/// Menu déroulant, ancré sous le bouton.
pub fn draw(ctx: &egui::Context, f: &mut Fx<'_>) {
    if !f.state.show_disk_picker {
        return;
    }
    let pal = f.pal;
    let screen = ctx.content_rect();
    let anchor = f.vs.disk_anchor.unwrap_or_else(|| {
        Rect::from_min_size(
            pos2(screen.right() - 96.0, screen.top() + 40.0),
            vec2(36.0, 34.0),
        )
    });
    // Lecteurs lisibles (un lecteur optique vide a une capacité nulle).
    let drives: Vec<fx_win::sys::Drive> = f
        .state
        .drives
        .iter()
        .filter(|d| d.total > 0)
        .cloned()
        .collect();
    // Première ouverture : le lecteur système est proposé.
    if !f.vs.disk_pick_init {
        f.vs.disk_pick_init = true;
        let sys = std::env::var_os("SystemDrive").map(|d| d.to_string_lossy().to_uppercase());
        f.vs.disk_pick = drives
            .iter()
            .filter(|d| {
                sys.as_deref()
                    .is_some_and(|s| d.root.to_string_lossy().to_uppercase().starts_with(s))
            })
            .map(|d| d.root.clone())
            .collect();
    }
    f.vs.disk_pick
        .retain(|p| drives.iter().any(|d| &d.root == p));
    let width = (screen.width() - 32.0).clamp(300.0, 440.0);
    let area = Area::new(Id::unique("disk-picker"))
        .order(Order::Foreground)
        .fixed_pos(pos2(anchor.right(), anchor.bottom() + 6.0))
        .pivot(Align2::RIGHT_TOP)
        .show(ctx, |ui| {
            widgets::menu_appear(ui);
            Frame::popup(ui.style()).inner_margin(14).show(ui, |ui| {
                ui.set_width(width);
                ui.label(RichText::new("Analyser l'espace").font(semibold(15.0)));
                ui.label(
                    RichText::new("Cochez les lecteurs : chacun s'ouvre dans son onglet.")
                        .size(12.0)
                        .color(pal.text2),
                );
                ui.add_space(8.0);
                if drives.is_empty() {
                    ui.label(RichText::new("Aucun lecteur trouvé.").color(pal.text3));
                }
                for d in &drives {
                    drive_row(ui, f, d);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let all = !drives.is_empty() && f.vs.disk_pick.len() == drives.len();
                    let label = if all {
                        "Tout désélectionner"
                    } else {
                        "Tout sélectionner"
                    };
                    if ui.button(label).clicked() {
                        f.vs.disk_pick = if all {
                            Vec::new()
                        } else {
                            drives.iter().map(|d| d.root.clone()).collect()
                        };
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let n = f.vs.disk_pick.len();
                        let text = match n {
                            0 | 1 => "Analyser".to_owned(),
                            n => format!("Analyser ({n})"),
                        };
                        let go = ui.add_enabled(
                            n > 0,
                            egui::Button::new(
                                RichText::new(format!("  {text}  ")).color(pal.on_accent),
                            )
                            .fill(pal.accent)
                            .min_size(vec2(110.0, 30.0)),
                        );
                        if go.clicked() || (n > 0 && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        {
                            // Dans l'ordre des lecteurs, pas dans celui des clics.
                            let roots: Vec<PathBuf> = drives
                                .iter()
                                .filter(|d| f.vs.disk_pick.contains(&d.root))
                                .map(|d| d.root.clone())
                                .collect();
                            f.send(Msg::AnalyzeDrives(roots));
                        }
                    });
                });
                // Le dossier affiché, s'il y en a un (hors analyse).
                let here = match f.state.pane().location() {
                    fx_app::Location::Dir(d) if d.parent().is_some() => Some(d.clone()),
                    _ => None,
                };
                if let Some(dir) = here {
                    ui.add_space(6.0);
                    ui.separator();
                    let r = ui.add(
                        egui::Button::new(widgets::menu_entry(
                            glyph::FOLDER,
                            &format!("Analyser le dossier affiché ({})", dir.display()),
                            &pal,
                        ))
                        .truncate(),
                    );
                    if r.clicked() {
                        f.send(Msg::CloseOverlays);
                        f.send(Msg::AnalyzeSpace(dir));
                    }
                }
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

/// Ligne d'un lecteur : case, icône, nom, jauge d'occupation, place libre ; « analysé » si
/// une analyse est déjà ouverte (elle sera reprise, pas relancée).
fn drive_row(ui: &mut Ui, f: &mut Fx<'_>, d: &fx_win::sys::Drive) {
    let pal = f.pal;
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    let on = f.vs.disk_pick.contains(&d.root);
    if r.hovered() {
        ui.painter().rect_filled(rect, 6.0, pal.hover);
    }
    if r.clicked() {
        if on {
            f.vs.disk_pick.retain(|p| p != &d.root);
        } else {
            f.vs.disk_pick.push(d.root.clone());
        }
    }
    let state = if on {
        widgets::Check::On
    } else {
        widgets::Check::Off
    };
    widgets::check_box(
        ui,
        pos2(rect.left() + 16.0, rect.center().y),
        state,
        r.hovered(),
        &pal,
    );
    let g = match d.kind {
        DriveKind::Network => glyph::NETWORK,
        DriveKind::Removable => glyph::USB,
        DriveKind::Optical => glyph::DISC,
        _ => glyph::DRIVE,
    };
    let p = ui.painter();
    p.text(
        pos2(rect.left() + 42.0, rect.center().y),
        Align2::CENTER_CENTER,
        g,
        icon_font(16.0),
        pal.text2,
    );
    let x = rect.left() + 60.0;
    let right = rect.right() - 8.0;
    p.with_clip_rect(Rect::from_x_y_ranges(x..=(right - 90.0), rect.y_range()))
        .text(
            pos2(x, rect.top() + 14.0),
            Align2::LEFT_CENTER,
            drive_label(d),
            FontId::proportional(13.0),
            pal.text,
        );
    if fx_app::update::disk::scan_of(f.state, &d.root).is_some() {
        p.text(
            pos2(right, rect.top() + 14.0),
            Align2::RIGHT_CENTER,
            "déjà analysé",
            FontId::proportional(11.5),
            pal.accent,
        );
    }
    let used = 1.0 - d.free as f32 / d.total as f32;
    let bar = Rect::from_min_size(
        pos2(x, rect.top() + 26.0),
        vec2((right - x - 150.0).max(40.0), 5.0),
    );
    let color = if used > 0.9 { pal.danger } else { pal.accent };
    widgets::gradient_bar(ui, bar, used, color, pal.stroke_strong);
    ui.painter().text(
        pos2(right, bar.center().y),
        Align2::RIGHT_CENTER,
        format!("{} libres sur {}", human(d.free), human(d.total)),
        FontId::proportional(11.5),
        pal.text3,
    );
}
