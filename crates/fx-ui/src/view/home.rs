//! Page d'accueil (comme celle de Files) : accès rapide, lecteurs, étiquettes, récents.

use eframe::egui::{self, Align2, FontId, Id, Rect, RichText, ScrollArea, Sense, Ui, pos2, vec2};
use fx_app::{Location, Msg, PaneId};
use fx_win::sys::DriveKind;

use super::Fx;
use super::sidebar::{display_name, drive_label, human};
use crate::theme::{glyph, semibold};
use crate::widgets;

pub fn draw(ui: &mut Ui, f: &mut Fx<'_>, pane: PaneId) {
    let pal = f.pal;
    ScrollArea::vertical()
        .id_salt(("home", pane.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(18.0);
            let margin = 24.0;
            let inner_w = ui.available_width() - 2.0 * margin;
            ui.horizontal(|ui| {
                ui.add_space(margin);
                ui.vertical(|ui| {
                    ui.set_width(inner_w);
                    ui.label(
                        RichText::new("Accueil")
                            .font(semibold(26.0))
                            .color(pal.text),
                    );
                    ui.add_space(14.0);

                    widgets::section(ui, &pal, "Accès rapide");
                    quick_access(ui, f, inner_w);
                    ui.add_space(20.0);

                    widgets::section(ui, &pal, "Lecteurs");
                    drives(ui, f, inner_w);
                    ui.add_space(20.0);

                    if f.state.settings.show_tags && !f.state.tags.list.is_empty() {
                        widgets::section(ui, &pal, "Étiquettes");
                        tags(ui, f);
                        ui.add_space(20.0);
                    }

                    widgets::section(ui, &pal, "Fichiers récents");
                    recent(ui, f, inner_w);
                    ui.add_space(24.0);
                });
            });
        });
}

/// Écart entre les tuiles.
const GAP: f32 = 12.0;

/// Grille fluide : autant de tuiles d'au moins `min` points que la largeur en permet,
/// élargies (jusqu'à `max`) pour remplir la ligne au lieu de laisser un vide à droite.
fn fluid(width: f32, min: f32, max: f32) -> (usize, f32) {
    let per_row = ((width + GAP) / (min + GAP)).floor().max(1.0);
    let w = ((width - GAP * (per_row - 1.0)) / per_row).clamp(min.min(width), max);
    (per_row as usize, w)
}

/// État vide d'une section : carte discrète avec pictogramme, titre et explication.
fn empty_state(ui: &mut Ui, f: &Fx<'_>, width: f32, g: &str, title: &str, hint: &str) {
    let pal = f.pal;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 72.0), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, pal.hover);
    ui.painter().rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(1.0, pal.stroke),
        egui::StrokeKind::Inside,
    );
    let c = pos2(rect.left() + 36.0, rect.center().y);
    ui.painter()
        .circle_filled(c, 18.0, pal.accent.gamma_multiply(0.14));
    ui.painter().text(
        c,
        Align2::CENTER_CENTER,
        g,
        crate::theme::icon_font(16.0),
        pal.accent,
    );
    let x = rect.left() + 68.0;
    ui.painter().text(
        pos2(x, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        title,
        semibold(13.5),
        pal.text,
    );
    ui.painter().text(
        pos2(x, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        hint,
        FontId::proportional(12.0),
        pal.text2,
    );
}

fn card(ui: &mut Ui, f: &Fx<'_>, size: egui::Vec2, id: Id) -> (Rect, egui::Response) {
    let pal = f.pal;
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    let _ = id;
    let t = ui
        .ctx()
        .animate_bool_with_time(r.id.with("h"), r.hovered(), 0.1);
    ui.painter().rect_filled(rect, 8.0, pal.card);
    ui.painter().rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(
            1.0,
            if t > 0.0 {
                pal.stroke_strong
            } else {
                pal.stroke
            },
        ),
        egui::StrokeKind::Inside,
    );
    if t > 0.0 {
        ui.painter()
            .rect_filled(rect, 8.0, pal.hover.gamma_multiply(t));
    }
    (rect, r)
}

fn quick_access(ui: &mut Ui, f: &mut Fx<'_>, width: f32) {
    let pal = f.pal;
    let pins = f.state.settings.pinned.clone();
    let (per_row, w) = fluid(width, 150.0, 210.0);
    let h = 120.0;
    for chunk in pins.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for p in chunk {
                let (rect, r) = card(ui, f, vec2(w, h), Id::unique(("qa", p)));
                super::items::shell_icon(
                    ui,
                    f,
                    p,
                    pos2(rect.center().x, rect.top() + 42.0),
                    48.0,
                    glyph::FOLDER,
                    pal.folder,
                );
                let name = display_name(p);
                f.vs.fallbacks.scan(&name);
                let tr = Rect::from_min_max(
                    pos2(rect.left() + 10.0, rect.top() + 76.0),
                    pos2(rect.right() - 10.0, rect.bottom() - 6.0),
                );
                ui.painter().with_clip_rect(tr).text(
                    pos2(tr.center().x, tr.top() + 8.0),
                    Align2::CENTER_CENTER,
                    name,
                    FontId::proportional(13.5),
                    pal.text,
                );
                let parent = p
                    .parent()
                    .map(|x| x.display().to_string())
                    .unwrap_or_default();
                ui.painter().with_clip_rect(tr).text(
                    pos2(tr.center().x, tr.top() + 27.0),
                    Align2::CENTER_CENTER,
                    parent,
                    FontId::proportional(11.0),
                    pal.text3,
                );
                if r.clicked() {
                    f.send(Msg::Navigate(Location::Dir(p.clone())));
                }
                if r.middle_clicked() {
                    f.send(Msg::OpenInNewTab(Location::Dir(p.clone())));
                }
                r.context_menu(|ui| {
                    if ui.button("Ouvrir dans un nouvel onglet").clicked() {
                        f.send(Msg::OpenInNewTab(Location::Dir(p.clone())));
                        ui.close();
                    }
                    if ui.button("Désépingler").clicked() {
                        f.send(Msg::Unpin(p.clone()));
                        ui.close();
                    }
                    if ui.button("Propriétés").clicked() {
                        f.send(Msg::Properties(p.clone()));
                        ui.close();
                    }
                });
            }
        });
        ui.add_space(GAP);
    }
    if pins.is_empty() {
        empty_state(
            ui,
            f,
            width,
            glyph::PIN,
            "Aucun dossier épinglé",
            "Clic droit sur un dossier › Épingler à l'accès rapide.",
        );
    }
}

fn drives(ui: &mut Ui, f: &mut Fx<'_>, width: f32) {
    let pal = f.pal;
    let drives = f.state.drives.clone();
    let (per_row, w) = fluid(width, 280.0, 420.0);
    let h = 82.0;
    if drives.is_empty() {
        ui.spinner();
    }
    for chunk in drives.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for d in chunk {
                let (rect, r) = card(ui, f, vec2(w, h), Id::unique(("drv", d.letter)));
                let g = match d.kind {
                    DriveKind::Network => glyph::NETWORK,
                    DriveKind::Removable => glyph::USB,
                    DriveKind::Optical => glyph::DISC,
                    _ => glyph::DRIVE,
                };
                super::items::shell_icon(
                    ui,
                    f,
                    &d.root,
                    pos2(rect.left() + 34.0, rect.center().y),
                    40.0,
                    g,
                    pal.text2,
                );
                let x = rect.left() + 66.0;
                ui.painter().text(
                    pos2(x, rect.top() + 20.0),
                    Align2::LEFT_CENTER,
                    drive_label(d),
                    semibold(13.5),
                    pal.text,
                );
                if d.total > 0 {
                    let used = 1.0 - d.free as f32 / d.total as f32;
                    let bar = Rect::from_min_size(
                        pos2(x, rect.top() + 38.0),
                        vec2(rect.right() - x - 16.0, 6.0),
                    );
                    let color = if used > 0.9 { pal.danger } else { pal.accent };
                    widgets::gradient_bar(ui, bar, used, color, pal.stroke_strong);
                    ui.painter().text(
                        pos2(x, rect.top() + 60.0),
                        Align2::LEFT_CENTER,
                        format!("{} libres sur {}", human(d.free), human(d.total)),
                        FontId::proportional(11.5),
                        pal.text2,
                    );
                }
                if r.clicked() {
                    f.send(Msg::Navigate(Location::Dir(d.root.clone())));
                }
                if r.middle_clicked() {
                    f.send(Msg::OpenInNewTab(Location::Dir(d.root.clone())));
                }
                r.context_menu(|ui| {
                    if ui.button("Ouvrir dans un nouvel onglet").clicked() {
                        f.send(Msg::OpenInNewTab(Location::Dir(d.root.clone())));
                        ui.close();
                    }
                    if ui.button("Analyser l'espace").clicked() {
                        f.send(Msg::AnalyzeSpace(d.root.clone()));
                        ui.close();
                    }
                    if ui.button("Propriétés").clicked() {
                        f.send(Msg::Properties(d.root.clone()));
                        ui.close();
                    }
                });
            }
        });
        ui.add_space(GAP);
    }
}

fn tags(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for t in f.state.tags.list.clone() {
            let n = f.state.tags.count(t.id);
            let label = format!("{}   {n}", t.name);
            let galley =
                ui.painter()
                    .layout_no_wrap(label.clone(), FontId::proportional(13.0), pal.text);
            let (rect, r) =
                ui.allocate_exact_size(vec2(galley.size().x + 40.0, 32.0), Sense::click());
            let hover = ui
                .ctx()
                .animate_bool_with_time(r.id.with("h"), r.hovered(), 0.1);
            ui.painter().rect_filled(rect, 16.0, pal.card);
            ui.painter().rect_stroke(
                rect,
                16.0,
                egui::Stroke::new(1.0, pal.stroke_strong),
                egui::StrokeKind::Inside,
            );
            if hover > 0.0 {
                ui.painter()
                    .rect_filled(rect, 16.0, pal.hover.gamma_multiply(hover));
            }
            widgets::tag_icon(ui, pos2(rect.left() + 17.0, rect.center().y), 14.0, t.color);
            ui.painter().galley(
                pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0),
                galley,
                pal.text,
            );
            if r.clicked() {
                f.send(Msg::Navigate(Location::Tag(t.id)));
            }
        }
    });
}

fn recent(ui: &mut Ui, f: &mut Fx<'_>, width: f32) {
    let pal = f.pal;
    let items = f.state.recent.clone();
    if items.is_empty() {
        empty_state(
            ui,
            f,
            width,
            glyph::HISTORY,
            "Aucun fichier récent",
            "Les fichiers que vous ouvrez dans Windows apparaîtront ici.",
        );
        return;
    }
    let frame_rect = ui.available_rect_before_wrap();
    let total_h = items.len() as f32 * 34.0 + 8.0;
    let bg = Rect::from_min_size(frame_rect.min, vec2(width, total_h));
    ui.painter().rect_filled(bg, 8.0, pal.card);
    ui.painter().rect_stroke(
        bg,
        8.0,
        egui::Stroke::new(1.0, pal.stroke),
        egui::StrokeKind::Inside,
    );
    ui.add_space(4.0);
    let now = fx_win::time::now_filetime();
    for it in items {
        let (rect, r) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
        crate::widgets::hover_fill(ui, &r, rect.shrink2(vec2(4.0, 1.0)), &pal, 4.0);
        super::items::shell_icon(
            ui,
            f,
            &it.path,
            pos2(rect.left() + 22.0, rect.center().y),
            18.0,
            glyph::FILE,
            pal.text3,
        );
        f.vs.fallbacks.scan(&it.name);
        let tr = Rect::from_x_y_ranges(
            (rect.left() + 42.0)..=(rect.right() - 160.0),
            rect.y_range(),
        );
        ui.painter().with_clip_rect(tr).text(
            pos2(tr.left(), rect.center().y),
            Align2::LEFT_CENTER,
            &it.name,
            FontId::proportional(13.5),
            pal.text,
        );
        ui.painter().text(
            pos2(rect.right() - 16.0, rect.center().y),
            Align2::RIGHT_CENTER,
            relative_time(now, it.modified),
            FontId::proportional(12.0),
            pal.text2,
        );
        if r.clicked() {
            f.send(Msg::OpenPath(it.path.clone()));
        }
    }
    ui.add_space(4.0);
}

/// « à l'instant », « il y a 5 min », « hier »… à partir de FILETIME.
pub(crate) fn relative_time(now: i64, t: i64) -> String {
    let secs = (now - t) / 10_000_000;
    match secs {
        s if s < 60 => "à l'instant".into(),
        s if s < 3600 => format!("il y a {} min", s / 60),
        s if s < 86_400 => format!("il y a {} h", s / 3600),
        s if s < 2 * 86_400 => "hier".into(),
        s if s < 30 * 86_400 => format!("il y a {} jours", s / 86_400),
        _ => fx_win::time::filetime_to_local(t).map_or_else(String::new, |d| {
            format!("{:02}/{:02}/{}", d.day, d.month, d.year)
        }),
    }
}
