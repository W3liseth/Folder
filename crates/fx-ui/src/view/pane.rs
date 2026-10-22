//! Volets : cartes arrondies (une ou deux côte à côte), contenu selon l'emplacement et la
//! disposition.

use std::time::Instant;

use eframe::egui::{self, Align, Layout, Rect, RichText, Ui, UiBuilder, pos2, vec2};
use fx_app::{AppState, ListingStatus, Location, Msg, Pane, PaneId};
use fx_store::Layout as View;

use super::{Fx, columns, grid, home, list};
use crate::theme::{glyph, icon_font};
use crate::widgets;

pub fn draw_tab(ui: &mut Ui, f: &mut Fx<'_>) {
    let state: &AppState = f.state;
    let tab = state.tab();
    let full = ui.available_rect_before_wrap().shrink2(vec2(0.0, 0.0));
    let n = tab.panes.len();
    let gap = 8.0;
    let w = (full.width() - gap * (n as f32 - 1.0)) / n as f32;
    for (i, pane) in tab.panes.iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(full.left() + i as f32 * (w + gap), full.top() + 4.0),
            vec2(w, full.height() - 8.0),
        );
        let pal = f.pal;
        // Carte en relief : ombre douce, fond, contour, reflet clair sur l'arête haute et
        // liseré d'accent (le volet actif en double volet).
        widgets::glow(
            ui,
            rect.translate(vec2(0.0, 2.0)),
            10.0,
            egui::Color32::from_black_alpha(if pal.dark { 70 } else { 22 }),
            14.0,
        );
        ui.painter().rect_filled(rect, 10.0, pal.layer);
        ui.painter().rect_stroke(
            rect,
            10.0,
            egui::Stroke::new(1.0, pal.stroke),
            egui::StrokeKind::Inside,
        );
        let highlight = if pal.dark {
            egui::Color32::from_white_alpha(18)
        } else {
            egui::Color32::from_white_alpha(200)
        };
        widgets::neon_line(
            ui,
            (rect.left() + 12.0)..=(rect.right() - 12.0),
            rect.top() + 0.5,
            1.0,
            highlight,
        );
        if n == 1 || i == tab.active {
            widgets::neon_line(
                ui,
                (rect.left() + rect.width() * 0.2)..=(rect.right() - rect.width() * 0.2),
                rect.top(),
                1.5,
                pal.accent.gamma_multiply(0.75),
            );
        }
        if n > 1 && i == tab.active {
            widgets::focus_ring(ui, rect, &pal);
        }
        // Un appui n'importe où dans un volet inactif lui donne le focus.
        if n > 1
            && i != tab.active
            && ui.input(|inp| {
                inp.pointer.primary_pressed()
                    && inp.pointer.interact_pos().is_some_and(|p| rect.contains(p))
            })
        {
            f.send(Msg::FocusPane(pane.id));
        }
        // Transition à chaque nouvel emplacement : glissement dans le sens de la navigation
        // (entrer, suivant : depuis la droite ; précédent, parent : depuis la gauche) et fondu.
        let (offset, opacity) = transition(ui, f, pane);
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect.shrink(1.0).translate(offset))
                .id_salt(("pane", pane.id.0))
                .layout(Layout::top_down(Align::Min)),
        );
        child.set_clip_rect(rect.shrink(1.0));
        if opacity < 1.0 {
            child.set_opacity(opacity);
        }
        draw_pane(&mut child, f, pane);
    }
}

/// Décalage et opacité du contenu d'un volet pendant la transition (180 ms) qui suit une
/// navigation. Le contenu est repéré par la génération de sa liste (ou de la navigation pour
/// l'accueil) : une actualisation ne déclenche rien.
fn transition(ui: &Ui, f: &mut Fx<'_>, pane: &Pane) -> (egui::Vec2, f32) {
    use fx_app::state::NavKind;
    const DURATION: f32 = 0.18;
    let content = match pane.location() {
        Location::Home => pane.nav.serial,
        _ => pane
            .listing
            .as_ref()
            .map_or(0, fx_core::DirSnapshot::generation),
    };
    let first_frame = f.vs.frames == 0;
    let entry = f.vs.nav_anim.entry(pane.id).or_insert((
        content,
        Instant::now() - std::time::Duration::from_secs(1),
        NavKind::Other,
    ));
    if entry.0 != content {
        entry.0 = content;
        if content == pane.nav.serial && !first_frame {
            entry.1 = Instant::now();
            entry.2 = pane.nav.kind;
        }
    }
    let t = entry.1.elapsed().as_secs_f32() / DURATION;
    if t >= 1.0 {
        return (egui::Vec2::ZERO, 1.0);
    }
    ui.ctx().request_repaint();
    let e = widgets::ease_out(t);
    let shift = (1.0 - e) * 28.0;
    let offset = match entry.2 {
        NavKind::Open | NavKind::Forward => vec2(shift, 0.0),
        NavKind::Back | NavKind::Up => vec2(-shift, 0.0),
        NavKind::Other => vec2(0.0, (1.0 - e) * 10.0),
    };
    (offset, e)
}

fn draw_pane(ui: &mut Ui, f: &mut Fx<'_>, pane: &Pane) {
    let pal = f.pal;
    if let Location::Search { .. } = pane.location() {
        search_header(ui, f, pane);
    }
    match (&pane.status, pane.location()) {
        (_, Location::Home) => home::draw(ui, f, pane.id),
        (ListingStatus::Failed { message }, _) => {
            placeholder(ui, f, glyph::ERROR, "Emplacement indisponible", message)
        }
        _ => {
            let Some(listing) = &pane.listing else {
                ui.centered_and_justified(|ui| ui.spinner());
                return;
            };
            // Ancienne liste vide pendant un chargement (découverte réseau…) : on attend.
            if listing.is_empty() && pane.is_loading() {
                ui.centered_and_justified(|ui| ui.spinner());
                return;
            }
            if listing.is_empty() && listing.is_complete() {
                let (title, detail) = match pane.location() {
                    Location::Search { query, .. } => (
                        "Aucun résultat",
                        format!("Aucun élément ne correspond à « {query} »."),
                    ),
                    Location::Tag(_) => (
                        "Aucun fichier",
                        "Aucun fichier ne porte cette étiquette.".to_owned(),
                    ),
                    Location::RecycleBin => (
                        "La corbeille est vide",
                        "Les éléments supprimés apparaîtront ici.".to_owned(),
                    ),
                    Location::Network(None) => (
                        "Aucun ordinateur trouvé",
                        "Vérifiez que la découverte du réseau est activée dans les paramètres de \
                         partage de Windows."
                            .to_owned(),
                    ),
                    Location::Network(Some(_)) => (
                        "Aucun dossier partagé",
                        "Cet ordinateur ne partage aucun dossier accessible.".to_owned(),
                    ),
                    _ => (
                        "Ce dossier est vide",
                        "Glissez des fichiers ici ou créez un nouvel élément.".to_owned(),
                    ),
                };
                empty_area(ui, f, pane.id);
                placeholder(ui, f, glyph::FOLDER, title, &detail);
                return;
            }
            match pane.layout {
                View::Details => list::draw(ui, f, pane, false),
                View::List => list::draw(ui, f, pane, true),
                View::Grid => grid::draw(ui, f, pane),
                View::Columns => columns::draw(ui, f, pane),
            }
        }
    }
    let _ = pal;
}

fn search_header(ui: &mut Ui, f: &mut Fx<'_>, pane: &Pane) {
    let pal = f.pal;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(
            RichText::new(glyph::SEARCH)
                .font(icon_font(13.0))
                .color(pal.accent),
        );
        let summary = match (&pane.search_summary, pane.is_loading()) {
            (_, true) => "Recherche en cours…".to_owned(),
            (Some(s), _) => s.clone(),
            (None, _) => String::new(),
        };
        ui.label(RichText::new(summary).color(pal.text2));
        if pane.is_loading() {
            ui.spinner();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(10.0);
            if ui.button("Fermer la recherche").clicked() {
                f.send(Msg::Search {
                    pane: pane.id,
                    query: String::new(),
                });
            }
        });
    });
    ui.add_space(4.0);
}

/// Zone vide interactive (menu du fond, dépôt) pour un dossier vide.
fn empty_area(ui: &mut Ui, f: &mut Fx<'_>, pane: PaneId) {
    let rect = ui.available_rect_before_wrap();
    let r = ui.interact(
        rect,
        egui::Id::unique(("empty", pane.0)),
        egui::Sense::click(),
    );
    if r.secondary_clicked() {
        super::shellmenu::secondary_click(ui, f);
    }
    if r.clicked() || r.secondary_clicked() {
        f.send(Msg::FocusPane(pane));
    }
    if let Some(Location::Dir(dir)) = f.state.find_pane(pane).map(Pane::location) {
        super::dnd::pane_zone(f, rect, dir);
    }
    r.context_menu(|ui| super::menus::background_menu(ui, f));
}

pub(crate) fn placeholder(ui: &mut Ui, f: &Fx<'_>, g: &str, title: &str, detail: &str) {
    let pal = f.pal;
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space((ui.available_height() * 0.28).max(20.0));
        ui.label(RichText::new(g).font(icon_font(44.0)).color(pal.text3));
        ui.add_space(10.0);
        ui.label(
            RichText::new(title)
                .font(crate::theme::semibold(18.0))
                .color(pal.text),
        );
        ui.add_space(2.0);
        if !detail.is_empty() {
            ui.label(RichText::new(detail).color(pal.text2));
        }
    });
}
