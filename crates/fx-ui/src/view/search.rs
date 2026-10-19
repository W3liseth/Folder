//! Fenêtre de recherche (bouton Rechercher, Ctrl+F), dans le style de la palette de
//! commandes : saisie, recherche dans le dossier et ses sous-dossiers, et premiers résultats
//! du dossier courant à ouvrir directement (flèches + Entrée).

use eframe::egui::{self, Align2, Area, Frame, Id, Key, Order, Rect, Sense, TextEdit, pos2, vec2};
use fx_app::{Location, Msg};

use super::{Fx, items};
use crate::theme::{glyph, icon_font};
use crate::widgets;

/// Résultats affichés du dossier courant (les premiers trouvés, sans parcourir au-delà).
const MAX_HITS: usize = 8;

pub fn draw(ctx: &egui::Context, f: &mut Fx<'_>) {
    let pane = f.state.pane();
    let pane_id = pane.id;
    if !f.state.show_search {
        // Fermée : la saisie reprend la recherche affichée (ou rien), prête à la réouverture.
        f.vs.search_index = 0;
        let current = match pane.location() {
            Location::Search { query, .. } => query.as_str(),
            _ => "",
        };
        if f.vs.search.get(&pane_id).map(String::as_str) != Some(current) {
            f.vs.search.insert(pane_id, current.to_owned());
        }
        return;
    }
    let pal = f.pal;
    let (scope, root_ok) = match pane.location() {
        Location::Dir(p) | Location::Search { root: p, .. } => (
            p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            true,
        ),
        _ => (String::new(), false),
    };
    let query =
        f.vs.search
            .get(&pane_id)
            .cloned()
            .unwrap_or_default()
            .trim()
            .to_owned();

    // Premiers éléments du dossier dont le nom contient la saisie (arrêt au 8e trouvé).
    let hits: Vec<(usize, u32)> = match (&pane.listing, query.is_empty()) {
        (Some(l), false) => {
            let needle = query.to_lowercase();
            l.visible_ids()
                .iter()
                .enumerate()
                .filter(|(_, id)| {
                    fx_core::wtf8::display(l.view(**id).name)
                        .to_lowercase()
                        .contains(&needle)
                })
                .take(MAX_HITS)
                .map(|(pos, id)| (pos, *id))
                .collect()
        }
        _ => Vec::new(),
    };
    // Entrée 0 : recherche récursive (si possible), puis les résultats.
    let first_hit = usize::from(root_ok && !query.is_empty());
    let count = first_hit + hits.len();
    if f.vs.search_index >= count {
        f.vs.search_index = count.saturating_sub(1);
    }

    let screen = ctx.content_rect();
    let mut run: Option<usize> = None;
    let area = Area::new(Id::unique("search-overlay"))
        .order(Order::Foreground)
        .fixed_pos(pos2(screen.center().x, screen.top() + 90.0))
        .pivot(Align2::CENTER_TOP)
        .show(ctx, |ui| {
            widgets::menu_appear(ui);
            Frame::popup(ui.style()).inner_margin(10).show(ui, |ui| {
                ui.set_width(600.0);
                let text = f.vs.search.entry(pane_id).or_default();
                let id = Id::unique("search-input");
                let hint = if root_ok {
                    format!("Rechercher dans {scope}…")
                } else {
                    "Ouvrez un dossier pour y rechercher".to_owned()
                };
                // Champ : loupe dans la colonne des icônes des lignes, saisie alignée sur leur
                // texte et centrée sur la hauteur (egui place le texte en haut de sa zone).
                let (row, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
                ui.painter().text(
                    pos2(row.left() + 20.0, row.center().y),
                    Align2::CENTER_CENTER,
                    glyph::SEARCH,
                    icon_font(15.0),
                    pal.accent,
                );
                let font = egui::FontId::proportional(14.5);
                let line_h = ui.fonts_mut(|fonts| fonts.row_height(&font));
                let input = Rect::from_min_max(
                    pos2(row.left() + 40.0, row.center().y - line_h / 2.0),
                    pos2(row.right() - 10.0, row.center().y + line_h / 2.0),
                );
                let r = ui.put(
                    input,
                    TextEdit::singleline(text)
                        .id(id)
                        .hint_text(hint)
                        .font(font)
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .desired_width(input.width())
                        .min_size(input.size()),
                );
                r.request_focus();
                if r.changed() {
                    f.vs.search_index = 0;
                }
                ui.painter().hline(
                    row.x_range(),
                    row.bottom() + 0.5,
                    egui::Stroke::new(1.0, pal.stroke),
                );
                ui.input(|i| {
                    if i.key_pressed(Key::ArrowDown) {
                        f.vs.search_index = (f.vs.search_index + 1).min(count.saturating_sub(1));
                    }
                    if i.key_pressed(Key::ArrowUp) {
                        f.vs.search_index = f.vs.search_index.saturating_sub(1);
                    }
                    if i.key_pressed(Key::Enter) && count > 0 {
                        run = Some(f.vs.search_index);
                    }
                });
                if count == 0 {
                    if !query.is_empty() {
                        let (row, _) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                        ui.painter().text(
                            pos2(row.left() + 40.0, row.center().y),
                            Align2::LEFT_CENTER,
                            "Aucun élément de ce dossier ne correspond.",
                            egui::FontId::proportional(13.5),
                            pal.text3,
                        );
                    }
                    return;
                }
                ui.add_space(6.0);
                for i in 0..count {
                    let selected = i == f.vs.search_index;
                    let (rect, r) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                    if selected {
                        ui.painter().rect_filled(rect, 6.0, pal.selection);
                        widgets::accent_pill(
                            ui,
                            pos2(rect.left() + 1.5, rect.center().y),
                            16.0,
                            pal.accent,
                        );
                    } else {
                        widgets::hover_fill(ui, &r, rect, &pal, 6.0);
                    }
                    let icon_at = pos2(rect.left() + 20.0, rect.center().y);
                    if i < first_hit {
                        ui.painter().text(
                            icon_at,
                            Align2::CENTER_CENTER,
                            glyph::SEARCH,
                            icon_font(14.0),
                            pal.text2,
                        );
                        ui.painter().text(
                            pos2(rect.left() + 40.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            format!("Rechercher « {query} » dans {scope} et ses sous-dossiers"),
                            egui::FontId::proportional(13.5),
                            pal.text,
                        );
                    } else if let Some(l) = &pane.listing {
                        let (_, id) = hits[i - first_hit];
                        let e = l.view(id);
                        match items::texture(f, l, &e, 20, false) {
                            Some((tex, _)) => {
                                ui.painter().image(
                                    tex,
                                    Rect::from_center_size(icon_at, vec2(18.0, 18.0)),
                                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                    egui::Color32::WHITE,
                                );
                            }
                            None => {
                                ui.painter().text(
                                    icon_at,
                                    Align2::CENTER_CENTER,
                                    if e.meta.is_dir() {
                                        glyph::FOLDER
                                    } else {
                                        glyph::FILE
                                    },
                                    icon_font(14.0),
                                    pal.text2,
                                );
                            }
                        }
                        ui.painter().text(
                            pos2(rect.left() + 40.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            fx_core::wtf8::display(e.name),
                            egui::FontId::proportional(13.5),
                            pal.text,
                        );
                        ui.painter().text(
                            pos2(rect.right() - 10.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            if e.meta.is_dir() { "Dossier" } else { "Ouvrir" },
                            egui::FontId::proportional(12.0),
                            pal.text3,
                        );
                    }
                    if r.clicked() {
                        run = Some(i);
                    }
                }
            });
        });
    // Clic hors de la fenêtre : fermeture.
    if ctx.input(|i| i.pointer.any_pressed())
        && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
        && !area.response.rect.contains(p)
    {
        f.send(Msg::CloseOverlays);
    }
    if let Some(i) = run {
        f.send(Msg::CloseOverlays);
        if i < first_hit {
            f.send(Msg::Search {
                pane: pane_id,
                query,
            });
        } else if let Some(&(pos, _)) = hits.get(i - first_hit) {
            f.send(Msg::FocusPane(pane_id));
            f.send(Msg::Click {
                pane: pane_id,
                pos,
                ctrl: false,
                shift: false,
            });
            f.send(Msg::Activate { pane: pane_id, pos });
        }
    }
}
