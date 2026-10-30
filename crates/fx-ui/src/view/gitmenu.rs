//! Branche Git dans la barre d'état : bouton (logo et branche active) et menu des branches,
//! qui en active une autre (`git switch`, cf. `fx_app::git`).

use eframe::egui::{
    self, Align, Align2, Area, Color32, FontId, Frame, Id, Layout, Order, Rect, RichText, Sense,
    Shape, Stroke, TextEdit, Ui, pos2, vec2,
};
use fx_app::Msg;
use fx_app::git::GitBranches;
use fx_commands::CommandId;

use super::Fx;
use crate::theme::{Palette, glyph, icon_font};
use crate::widgets;

/// Orange du logo Git.
const GIT_ORANGE: Color32 = Color32::from_rgb(0xF0, 0x50, 0x32);
/// Au-delà, un champ de filtre est proposé en tête du menu.
const FILTER_FROM: usize = 8;

/// Bouton de la barre d'état (dans une mise en page de droite à gauche).
pub fn button(ui: &mut Ui, f: &mut Fx<'_>, branch: &str) {
    let pal = f.pal;
    let pane = f.state.pane();
    let switching = pane.git_switching.clone();
    let label = switching.as_deref().unwrap_or(branch);
    let font = FontId::proportional(12.0);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, pal.text2);
    let text_w = galley.size().x.min(220.0);
    let size = vec2(8.0 + 14.0 + 6.0 + text_w + 6.0 + 10.0 + 8.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let open = f.state.git_menu.is_some();
    if open {
        ui.painter()
            .rect_filled(rect, 4.0, pal.hover.gamma_multiply(1.5));
    } else {
        widgets::hover_fill(ui, &resp, rect, &pal, 4.0);
    }
    let logo_c = pos2(rect.left() + 8.0 + 7.0, rect.center().y);
    if switching.is_some() {
        let r = Rect::from_center_size(logo_c, vec2(12.0, 12.0));
        egui::Spinner::new().size(12.0).paint_at(ui, r);
    } else {
        git_logo(ui, logo_c, 14.0);
    }
    let text_pos = pos2(
        logo_c.x + 7.0 + 6.0,
        rect.center().y - galley.size().y / 2.0,
    );
    let clip = Rect::from_min_size(text_pos, vec2(text_w, galley.size().y));
    ui.painter()
        .with_clip_rect(clip)
        .galley(text_pos, galley, pal.text2);
    ui.painter().text(
        pos2(rect.right() - 8.0 - 5.0, rect.center().y),
        Align2::CENTER_CENTER,
        if open { "\u{E70E}" } else { "\u{E70D}" },
        icon_font(9.0),
        pal.text3,
    );
    f.vs.git_anchor = Some(rect);
    let tip = match &switching {
        Some(b) => format!("Activation de la branche « {b} »…"),
        None => format!(
            "Branche Git « {branch} » — {} ({})",
            CommandId::GitBranches.label(),
            f.shortcut(CommandId::GitBranches)
        ),
    };
    if resp.on_hover_text(tip).clicked() && switching.is_none() {
        f.cmd(CommandId::GitBranches);
    }
}

/// Logo Git (losange orange et branche blanche) centré sur `c`, de côté `size`.
fn git_logo(ui: &Ui, c: egui::Pos2, size: f32) {
    let r = size / 2.0;
    let p = ui.painter();
    p.add(Shape::convex_polygon(
        vec![
            pos2(c.x, c.y - r),
            pos2(c.x + r, c.y),
            pos2(c.x, c.y + r),
            pos2(c.x - r, c.y),
        ],
        GIT_ORANGE,
        Stroke::NONE,
    ));
    // Tronc venu du bord supérieur gauche jusqu'au nœud du bas, branche vers la droite.
    let at = |x: f32, y: f32| pos2(c.x + x * r, c.y + y * r);
    let (node, bottom, right) = (at(-0.05, 0.0), at(-0.05, 0.5), at(0.42, -0.02));
    let line = Stroke::new((size * 0.11).max(1.2), Color32::WHITE);
    p.line_segment([at(-0.5, -0.45), node], line);
    p.line_segment([node, bottom], line);
    p.line_segment([node, right], line);
    let dot = (size * 0.12).max(1.4);
    p.circle_filled(bottom, dot, Color32::WHITE);
    p.circle_filled(right, dot, Color32::WHITE);
}

/// Menu des branches, ouvert au-dessus du bouton.
pub fn draw(ctx: &egui::Context, f: &mut Fx<'_>) {
    let Some(menu) = f.state.git_menu.clone() else {
        f.vs.git_filter.clear();
        f.vs.git_menu_h = 0.0;
        return;
    };
    let pal = f.pal;
    let screen = ctx.content_rect();
    let anchor = f.vs.git_anchor.unwrap_or_else(|| {
        Rect::from_min_size(
            pos2(screen.right() - 140.0, screen.bottom() - 30.0),
            vec2(120.0, 24.0),
        )
    });
    let max_h = (anchor.top() - screen.top() - 120.0).clamp(140.0, 420.0);
    let current = f.state.pane().git_branch.clone();
    let mut chosen: Option<String> = None;
    // Ouvert vers le haut : placé d'après sa hauteur à l'image précédente (un ancrage par le
    // bas limiterait la liste à la hauteur du premier affichage, « Lecture des branches… »).
    let width = 300.0;
    let last_h = if f.vs.git_menu_h > 0.0 {
        f.vs.git_menu_h
    } else {
        80.0
    };
    let top = (anchor.top() - 6.0 - last_h).max(screen.top() + 8.0);
    let left = (anchor.right() - width - 16.0).max(screen.left() + 8.0);
    let area = Area::new(Id::unique("git-menu"))
        .order(Order::Foreground)
        .fixed_pos(pos2(left, top))
        .pivot(Align2::LEFT_TOP)
        .show(ctx, |ui| {
            widgets::menu_appear(ui);
            Frame::popup(ui.style()).inner_margin(8).show(ui, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Branches").font(crate::theme::semibold(14.0)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(name) = menu.dir.file_name() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(name.to_string_lossy())
                                        .size(11.5)
                                        .color(pal.text3),
                                )
                                .truncate(),
                            );
                        }
                    });
                });
                ui.add_space(4.0);
                match &menu.branches {
                    None => {
                        ui.horizontal(|ui| {
                            ui.add_space(6.0);
                            ui.add(egui::Spinner::new().size(14.0));
                            ui.label(RichText::new("Lecture des branches…").color(pal.text2));
                        });
                        ui.add_space(4.0);
                    }
                    Some(Err(e)) => {
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(6.0);
                            ui.label(RichText::new(e).size(12.5).color(pal.danger));
                        });
                        ui.add_space(4.0);
                    }
                    Some(Ok(b)) => {
                        chosen = branch_list(ui, f, b, current.as_deref(), max_h);
                    }
                }
            });
        });
    let h = area.response.rect.height();
    if (h - f.vs.git_menu_h).abs() > 0.5 {
        f.vs.git_menu_h = h;
        ctx.request_repaint();
    }
    if let Some(b) = chosen {
        f.send(Msg::SwitchGitBranch(b));
    }
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

/// Liste filtrable ; renvoie le nom à passer à `git switch` pour la branche cliquée.
fn branch_list(
    ui: &mut Ui,
    f: &mut Fx<'_>,
    b: &GitBranches,
    current: Option<&str>,
    max_h: f32,
) -> Option<String> {
    let pal = f.pal;
    let mut chosen = None;
    if b.local.len() + b.remote.len() >= FILTER_FROM {
        let r = ui.add(
            TextEdit::singleline(&mut f.vs.git_filter)
                .hint_text("Filtrer les branches")
                .desired_width(f32::INFINITY)
                .margin(vec2(8.0, 6.0)),
        );
        if !r.has_focus() && !ui.ctx().memory(|m| m.focused().is_some()) {
            r.request_focus();
        }
        ui.add_space(4.0);
    }
    let filter = f.vs.git_filter.trim().to_lowercase();
    let keep = |n: &str| filter.is_empty() || n.to_lowercase().contains(&filter);
    let local: Vec<&String> = b.local.iter().filter(|n| keep(n)).collect();
    let remote: Vec<&String> = b.remote.iter().filter(|n| keep(n)).collect();
    // Entrée dans le filtre : la première branche trouvée.
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    if enter && !filter.is_empty() {
        if let Some(n) = local.first() {
            return Some((*n).clone());
        }
        if let Some(n) = remote.first() {
            return Some(GitBranches::remote_target(n).to_owned());
        }
    }
    // Hauteur voulue imposée : une zone flottante reprend sa taille de l'image précédente
    // (« Lecture des branches… ») et la liste y resterait à l'étroit.
    let sections =
        usize::from(!remote.is_empty()) + usize::from(!b.remote.is_empty() && !local.is_empty());
    let rows = (local.len() + remote.len()).max(1);
    let gap = ui.spacing().item_spacing.y;
    let want = rows as f32 * (30.0 + gap) + sections as f32 * (24.0 + 2.0 * gap);
    egui::ScrollArea::vertical()
        .max_height(max_h)
        .min_scrolled_height(want.min(max_h))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if local.is_empty() && remote.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Aucune branche").color(pal.text3));
                });
            }
            if !b.remote.is_empty() && !local.is_empty() {
                section(ui, &pal, "Locales");
            }
            for n in local {
                let on = current == Some(n.as_str());
                if row(ui, &pal, n, on, false).clicked() && !on {
                    chosen = Some(n.clone());
                }
            }
            if !remote.is_empty() {
                section(ui, &pal, "Distantes");
                for n in remote {
                    if row(ui, &pal, n, false, true)
                        .on_hover_text("Crée la branche locale qui suit cette branche")
                        .clicked()
                    {
                        chosen = Some(GitBranches::remote_target(n).to_owned());
                    }
                }
            }
        });
    chosen
}

fn section(ui: &mut Ui, pal: &Palette, title: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(RichText::new(title).size(11.5).color(pal.text3));
    });
    ui.add_space(2.0);
}

fn row(ui: &mut Ui, pal: &Palette, name: &str, current: bool, remote: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    widgets::hover_fill(ui, &resp, rect, pal, 4.0);
    let p = ui.painter();
    if current {
        p.text(
            pos2(rect.left() + 16.0, rect.center().y),
            Align2::CENTER_CENTER,
            glyph::CHECK,
            icon_font(12.0),
            pal.accent,
        );
    } else if remote {
        p.text(
            pos2(rect.left() + 16.0, rect.center().y),
            Align2::CENTER_CENTER,
            "\u{E753}",
            icon_font(12.0),
            pal.text3,
        );
    }
    let font = if current {
        crate::theme::semibold(13.5)
    } else {
        FontId::proportional(13.5)
    };
    let color = if current { pal.text } else { pal.text2 };
    let text_rect = Rect::from_min_max(
        pos2(rect.left() + 32.0, rect.top()),
        pos2(rect.right() - 6.0, rect.bottom()),
    );
    let galley = p.layout_no_wrap(name.to_owned(), font, color);
    let pos = pos2(text_rect.left(), rect.center().y - galley.size().y / 2.0);
    p.with_clip_rect(text_rect).galley(pos, galley, color);
    if current {
        resp.on_hover_text("Branche active")
    } else {
        resp
    }
}
