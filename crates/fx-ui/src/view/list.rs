//! Vues Détails et Liste : lignes virtualisées, en-têtes triables et redimensionnables,
//! sélection rectangulaire.

use std::fmt::Write as _;

use eframe::egui::{
    self, Align2, Color32, FontId, Id, Pos2, Rect, Response, ScrollArea, Sense, Stroke, Ui, pos2,
    vec2,
};
use fx_app::update::sort_indicator;
use fx_app::{Msg, Pane, PaneId};
use fx_core::{DirSnapshot, EntryFlags, SortDir, SortKey, format};

use super::{Band, Fx, items, menus};
use crate::theme::{glyph, icon_font};

const ROW: f32 = 30.0;
const COMPACT_ROW: f32 = 26.0;
const HEADER: f32 = 34.0;
const PAD: f32 = 10.0;
/// Début du nom, après l'icône, depuis le bord de la colonne Nom (en-tête compris).
const NAME_X: f32 = 30.0;

struct Cols {
    name: (f32, f32),
    modified: (f32, f32),
    kind: (f32, f32),
    size: (f32, f32),
}

/// Largeurs [Nom, Date, Type, Taille] ; Nom à 0 : automatique. Pendant un redimensionnement,
/// les largeurs en cours vivent dans la mémoire d'egui (sauvegardées au relâchement).
fn widths(ui: &Ui, f: &Fx<'_>) -> [f32; 4] {
    let s = &f.state.settings;
    let saved = [s.col_name, s.col_modified, s.col_type, s.col_size];
    match ui
        .ctx()
        .data(|d| d.get_temp::<[f32; 4]>(Id::unique("colw")))
    {
        Some(t) if t == saved => {
            ui.ctx()
                .data_mut(|d| d.remove::<[f32; 4]>(Id::unique("colw")));
            saved
        }
        Some(t) => t,
        None => saved,
    }
}

/// Largeur minimale de chaque colonne, et de la colonne Nom quand elle est automatique.
const MIN_COL: f32 = 60.0;
const MIN_NAME: f32 = 160.0;

/// Colonnes ancrées à gauche, comme l'Explorateur : chaque séparateur ne redimensionne que
/// la colonne à sa gauche, les suivantes se décalent. Nom automatique : la place restante
/// (au moins `MIN_NAME`). Trop étroit : les dernières colonnes sont coupées au bord droit
/// (jamais masquées pendant un redimensionnement, qui ne se bloque donc pas).
fn layout(rect: Rect, w: [f32; 4], compact: bool) -> Cols {
    let right = rect.right() - PAD;
    let left = rect.left() + PAD;
    if compact {
        return Cols {
            name: (left, right),
            modified: (right, right),
            kind: (right, right),
            size: (right, right),
        };
    }
    let others = w[1] + w[2] + w[3];
    let avail = right - left;
    let name_w = if w[0] > 0.0 {
        w[0]
    } else {
        (avail - others).max(MIN_NAME)
    };
    let clip = |x0: f32, x1: f32| (x0.min(right), x1.min(right));
    let name = (left, left + name_w);
    let modified = clip(name.1, name.1 + w[1]);
    let kind = clip(name.1 + w[1], name.1 + w[1] + w[2]);
    let size = clip(name.1 + w[1] + w[2], name.1 + others);
    Cols {
        name: clip(name.0, name.1),
        modified,
        kind,
        size,
    }
}

fn visible(c: (f32, f32)) -> bool {
    c.1 - c.0 > 24.0
}

pub fn draw(ui: &mut Ui, f: &mut Fx<'_>, pane: &Pane, compact: bool) {
    let Some(listing) = &pane.listing else { return };
    let w = widths(ui, f);
    let full = ui.available_rect_before_wrap();
    let cols = layout(full, w, compact);
    if !compact {
        header(ui, f, pane, &cols, w);
    } else {
        select_all_strip(ui, f, pane);
    }
    let row_h = if compact { COMPACT_ROW } else { ROW };
    let rows = listing.len();

    // Défilement : retour en haut à chaque nouvel emplacement, élément rendu visible au clavier.
    let sc = f.vs.scroll.entry(pane.id).or_default();
    if sc.shown.as_ref() != Some(pane.location()) {
        sc.shown = Some(pane.location().clone());
        sc.request = Some(0.0);
    }
    if let Some(pos) = sc.ensure.take() {
        let top = pos as f32 * row_h;
        if top < sc.offset {
            sc.request = Some(top);
        } else if top + row_h > sc.offset + sc.view_h {
            sc.request = Some(top + row_h - sc.view_h);
        }
    }
    let mut area = ScrollArea::vertical()
        .id_salt(("list", pane.id.0))
        .auto_shrink([false, false]);
    if let Some(off) = sc.request.take() {
        area = area.vertical_scroll_offset(off);
    }

    let out = area.show_viewport(ui, |ui, viewport| {
        let content_h = (rows as f32 * row_h + 40.0).max(viewport.height());
        ui.set_height(content_h);
        let origin = ui.max_rect().top();
        let left = ui.max_rect().left();
        let width = ui.max_rect().width();
        let vis = Rect::from_min_size(
            pos2(left, origin + viewport.top()),
            vec2(width, viewport.height()),
        );
        let bg = ui.interact(vis, Id::unique(("bg", pane.id.0)), Sense::click_and_drag());
        background(ui, f, pane.id, &bg);

        let first = (viewport.top() / row_h).floor().max(0.0) as usize;
        let last = ((viewport.bottom() / row_h).ceil() as usize).min(rows);
        let mut fmt = String::new();
        for pos in first..last {
            let Some(e) = listing.at(pos) else { break };
            let rect =
                Rect::from_min_size(pos2(left, origin + pos as f32 * row_h), vec2(width, row_h));
            let resp = ui.interact(
                rect,
                Id::unique(("row", pane.id.0, e.id)),
                Sense::click_and_drag(),
            );
            let selected = pane.selection.contains(e.id);
            row(
                ui, f, pane, listing, pos, &e, rect, &cols, selected, &resp, compact, &mut fmt,
            );
            items::interact(ui, f, pane.id, listing, pos, &e, &resp, selected, None);
        }
        band(ui, f, pane.id, &bg, origin, viewport, |y0, y1| {
            let a = (y0 / row_h).floor().max(0.0) as usize;
            let b = ((y1 / row_h).ceil() as usize).min(rows);
            (a, b)
        });
    });
    let sc = f.vs.scroll.entry(pane.id).or_default();
    sc.offset = out.state.clamped_offset().y;
    sc.view_h = out.inner_rect.height();
    sc.width = out.inner_rect.width();
}

fn header(ui: &mut Ui, f: &mut Fx<'_>, pane: &Pane, cols: &Cols, w: [f32; 4]) {
    let pal = f.pal;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER), Sense::hover());
    ui.painter().hline(
        rect.x_range().shrink(8.0),
        rect.bottom() - 0.5,
        Stroke::new(1.0, pal.stroke),
    );
    // Corbeille : date de suppression et emplacement d'origine à la place de la date de
    // modification et du type, comme l'Explorateur.
    let recycle = matches!(pane.location(), fx_app::Location::RecycleBin);
    let (date_label, kind_label) = if recycle {
        ("Supprimé le", "Emplacement d'origine")
    } else {
        ("Modifié le", "Type")
    };
    let entries = [
        ("Nom", SortKey::Name, cols.name, false, 0),
        (date_label, SortKey::Modified, cols.modified, false, 1),
        (kind_label, SortKey::Type, cols.kind, false, 2),
        ("Taille", SortKey::Size, cols.size, true, 3),
    ];
    for (label, key, (x0, x1), right, _) in entries {
        if !visible((x0, x1)) {
            continue;
        }
        let cell = Rect::from_x_y_ranges(x0..=x1, rect.y_range().shrink(3.0));
        let resp = ui.interact(cell, Id::unique(("hdr", pane.id.0, label)), Sense::click());
        crate::widgets::hover_fill(ui, &resp, cell, &pal, 4.0);
        if resp.clicked() {
            f.send(Msg::FocusPane(pane.id));
            f.send(Msg::SortBy(key));
        }
        let font = FontId::proportional(12.5);
        let (anchor, x) = if right {
            (Align2::RIGHT_CENTER, x1 - 10.0)
        } else if key == SortKey::Name {
            // Aligné sur les noms ; la case « tout sélectionner » est au-dessus des icônes.
            (Align2::LEFT_CENTER, x0 + NAME_X)
        } else {
            (Align2::LEFT_CENTER, x0 + 8.0)
        };
        let _ = font;
        let tr = crate::widgets::caps(ui, pos2(x, cell.center().y), anchor, label, 10.5, pal.text3);
        if let Some(dir) = sort_indicator(pane, key) {
            let g = if dir == SortDir::Asc {
                glyph::CHEVRON_UP
            } else {
                glyph::CHEVRON_DOWN
            };
            let gx = if right {
                tr.left() - 12.0
            } else {
                tr.right() + 8.0
            };
            ui.painter().text(
                pos2(gx, cell.center().y + 1.0),
                Align2::CENTER_CENTER,
                g,
                icon_font(8.0),
                pal.text2,
            );
        }
    }
    // Séparateurs, créés après tous les en-têtes : sinon l'en-tête suivant recouvre leur
    // moitié droite et un glissement commencé juste à droite du bord trie au lieu de
    // redimensionner.
    for (_, _, (x0, x1), _, wi) in entries {
        if !visible((x0, x1)) {
            continue;
        }
        let edge = x1.min(rect.right() - PAD - 2.0);
        let grip =
            Rect::from_center_size(pos2(edge, rect.center().y), vec2(12.0, rect.height() - 6.0));
        let g = ui.interact(
            grip,
            Id::unique(("grip", pane.id.0, wi)),
            Sense::click_and_drag(),
        );
        if g.hovered() || g.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            let color = if g.dragged() {
                pal.accent
            } else {
                pal.stroke_strong
            };
            ui.painter().vline(
                edge,
                rect.y_range().shrink(7.0),
                Stroke::new(if g.dragged() { 2.0 } else { 1.0 }, color),
            );
        }
        let origin_id = Id::unique(("colw-origin", pane.id.0));
        if g.drag_started()
            && let Some(px) = g.interact_pointer_pos()
        {
            // Nom automatique : figé à sa largeur actuelle dès qu'on touche aux colonnes.
            let mut nw = w;
            nw[0] = cols.name.1 - cols.name.0;
            ui.ctx().data_mut(|d| {
                d.insert_temp(Id::unique("colw"), nw);
                d.insert_temp(origin_id, (px.x, nw));
            });
        }
        // Largeur suivant la position du curseur (et non des déplacements cumulés) : le bord
        // reste sous le curseur, même après avoir atteint une largeur minimale.
        if g.dragged()
            && let (Some(px), Some((start_x, start))) = (
                g.interact_pointer_pos(),
                ui.ctx().data(|d| d.get_temp::<(f32, [f32; 4])>(origin_id)),
            )
        {
            let min = if wi == 0 { MIN_NAME } else { MIN_COL };
            let mut nw = start;
            nw[wi] = (start[wi] + px.x - start_x).clamp(min, 1600.0);
            ui.ctx().data_mut(|d| d.insert_temp(Id::unique("colw"), nw));
        }
        if g.drag_stopped() {
            let nw = widths(ui, f);
            f.send(super::settings_with(f.state, |s| {
                s.col_name = nw[0];
                s.col_modified = nw[1];
                s.col_type = nw[2];
                s.col_size = nw[3];
            }));
            // Les largeurs provisoires restent jusqu'à ce que les réglages les reprennent
            // (cf. `widths`) : pas de retour en arrière d'une image au relâchement.
            ui.ctx()
                .data_mut(|d| d.remove::<(f32, [f32; 4])>(origin_id));
        }
        // Double-clic sur le séparateur de Nom : de nouveau automatique.
        if wi == 0 && g.double_clicked() {
            f.send(super::settings_with(f.state, |s| s.col_name = 0.0));
            ui.ctx()
                .data_mut(|d| d.remove::<[f32; 4]>(Id::unique("colw")));
        }
    }
    let center = pos2(cols.name.0 + 14.0, rect.center().y);
    select_all_box(ui, f, pane, center);
}

#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    f: &mut Fx<'_>,
    pane: &Pane,
    l: &DirSnapshot,
    pos: usize,
    e: &fx_core::EntryView<'_>,
    rect: Rect,
    cols: &Cols,
    selected: bool,
    resp: &Response,
    compact: bool,
    fmt: &mut String,
) {
    let pal = f.pal;
    if !ui.is_rect_visible(rect) {
        return;
    }
    let bg = rect.shrink2(vec2(6.0, 1.0));
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.08);
    // Sélection en fondu (100 ms) ; la pastille d'accent grandit avec elle.
    let sel = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("s"), selected, 0.1);
    if sel > 0.0 {
        let fill = if resp.hovered() {
            pal.selection_hover
        } else {
            pal.selection
        };
        ui.painter().rect_filled(bg, 6.0, fill.gamma_multiply(sel));
        // Halo d'accent qui part de la pastille et s'éteint vers la droite.
        let glow = Rect::from_min_max(
            pos2(bg.left() + 6.0, bg.top()),
            pos2(bg.left() + (bg.width() * 0.45).max(60.0), bg.bottom()),
        );
        crate::widgets::hgradient(
            ui,
            glow,
            pal.accent.gamma_multiply(0.34 * sel),
            egui::Color32::TRANSPARENT,
        );
        crate::widgets::accent_pill(
            ui,
            pos2(bg.left() + 1.5, bg.center().y),
            bg.height() * 0.5 * sel,
            pal.accent,
        );
    }
    if !selected && hover > 0.0 {
        ui.painter()
            .rect_filled(bg, 6.0, pal.hover.gamma_multiply(hover));
    }

    let y = rect.center().y;
    let dim = e.meta.flags.contains(EntryFlags::HIDDEN);
    let fade = |c: Color32| if dim { c.gamma_multiply(0.55) } else { c };
    let icon_size = if compact { 16.0 } else { 18.0 };
    let icon_rect = Rect::from_center_size(pos2(cols.name.0 + 14.0, y), vec2(icon_size, icon_size));
    // Case de sélection posée sur l'icône, au survol de la ligne : un clic dessus ajoute ou
    // retire l'élément de la sélection.
    let check = ui.interact(icon_rect.expand(2.0), resp.id.with("check"), Sense::click());
    if check.clicked() {
        items::toggle_check(f, pane.id, pos);
    }
    let texture = items::texture(f, l, e, 20, false);
    if resp.contains_pointer() || check.hovered() {
        let state = if selected {
            crate::widgets::Check::On
        } else {
            crate::widgets::Check::Off
        };
        crate::widgets::check_box(ui, icon_rect.center(), state, check.hovered(), &pal);
    } else {
        match texture {
            Some((tex, _)) => {
                ui.painter().image(
                    tex,
                    icon_rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    fade(Color32::WHITE),
                );
            }
            None => {
                let (g, c) = if e.meta.is_dir() {
                    (glyph::FOLDER, pal.folder)
                } else {
                    (glyph::FILE, pal.text3)
                };
                ui.painter().text(
                    icon_rect.center(),
                    Align2::CENTER_CENTER,
                    g,
                    icon_font(15.0),
                    fade(c),
                );
            }
        }
    }

    let name = items::display_name(f, l, e);
    f.vs.fallbacks.scan(&name);
    let name_rect =
        Rect::from_x_y_ranges((cols.name.0 + NAME_X)..=(cols.name.1 - 8.0), rect.y_range());
    let name_font = FontId::proportional(13.5);
    // Pendant le renommage, le champ remplace le nom ; les autres colonnes restent visibles.
    let renaming = items::rename_editor(
        ui,
        f,
        name_rect,
        name_font.clone(),
        pane.id,
        e.id,
        &fx_core::wtf8::display(e.name),
        false,
    );
    if !renaming {
        let galley = ui.painter().layout_no_wrap(name, name_font, fade(pal.text));
        let text_w = galley.size().x;
        ui.painter().with_clip_rect(name_rect).galley(
            pos2(name_rect.left(), y - galley.size().y / 2.0),
            galley,
            pal.text,
        );
        // Pastilles d'étiquettes après le nom.
        let path = fx_app::state::path_of(l, e.id);
        let tags: &[fx_store::TagId] = if f.state.settings.show_tags {
            f.state.tags.of(&path)
        } else {
            &[]
        };
        let mut tx = (name_rect.left() + text_w + 10.0).min(name_rect.right() - 6.0);
        for t in tags.iter().filter_map(|id| f.state.tags.get(*id)) {
            crate::widgets::tag_dot(ui, pos2(tx, y), 4.0, t.color);
            tx += 10.0;
        }
    }
    if compact {
        return;
    }

    let small = FontId::proportional(12.5);
    let clip = |x0: f32, x1: f32| {
        ui.painter()
            .with_clip_rect(Rect::from_x_y_ranges(x0..=(x1 - 6.0), rect.y_range()))
    };
    if visible(cols.modified) {
        fmt.clear();
        if let Some(t) = fx_win::time::filetime_to_local(e.meta.modified) {
            let _ = write!(
                fmt,
                "{:02}/{:02}/{} {:02}:{:02}",
                t.day, t.month, t.year, t.hour, t.minute
            );
        }
        clip(cols.modified.0, cols.modified.1).text(
            pos2(cols.modified.0 + 8.0, y),
            Align2::LEFT_CENTER,
            fmt.as_str(),
            small.clone(),
            fade(pal.text2),
        );
    }
    if visible(cols.kind) {
        let kind = if matches!(pane.location(), fx_app::Location::RecycleBin) {
            l.dir_of(e.id).display().to_string()
        } else if let fx_app::Location::Network(server) = pane.location() {
            match (server, e.meta.is_dir()) {
                (None, true) => "Ordinateur".to_owned(),
                (None, false) => "Appareil".to_owned(),
                (Some(_), _) => "Dossier partagé".to_owned(),
            }
        } else {
            type_name(e.meta.is_dir(), l.ext_of(e.meta))
        };
        clip(cols.kind.0, cols.kind.1).text(
            pos2(cols.kind.0 + 8.0, y),
            Align2::LEFT_CENTER,
            kind,
            small.clone(),
            fade(pal.text2),
        );
    }
    // Appareils du réseau : pas de taille.
    let sized = !matches!(pane.location(), fx_app::Location::Network(_));
    if !e.meta.is_dir() && sized && visible(cols.size) {
        fmt.clear();
        // Taille lisible (Ko, Mo, Go…), comme la barre d'état.
        let _ = format::size_human(fmt, e.meta.size);
        ui.painter().text(
            pos2(cols.size.1 - 10.0, y),
            Align2::RIGHT_CENTER,
            fmt.as_str(),
            small,
            fade(pal.text2),
        );
    }
    if l.is_virtual() {
        // Résultats de recherche : dossier d'origine en info-bulle.
        resp.clone()
            .on_hover_text(l.dir_of(e.id).display().to_string());
    }
}

/// Fond de liste : clic (désélection), menu du fond.
pub(crate) fn background(ui: &Ui, f: &mut Fx<'_>, pane: PaneId, bg: &Response) {
    if bg.clicked() || bg.secondary_clicked() {
        f.send(Msg::FocusPane(pane));
        if !ui.input(|i| i.modifiers.command) {
            f.send(Msg::SelectPositions {
                pane,
                positions: Vec::new(),
                extend: false,
            });
        }
    }
    if let Some(fx_app::Location::Dir(dir)) = f.state.find_pane(pane).map(fx_app::Pane::location) {
        super::dnd::pane_zone(f, bg.rect, dir);
    }
    bg.context_menu(|ui| menus::background_menu(ui, f));
}

/// Sélection rectangulaire. `range(y0, y1)` : positions (début, fin exclue) couvertes par
/// l'intervalle vertical (coordonnées de contenu) — pour la grille, l'appelant filtre ensuite.
pub(crate) fn band(
    ui: &Ui,
    f: &mut Fx<'_>,
    pane: PaneId,
    bg: &Response,
    origin: f32,
    viewport: Rect,
    range: impl Fn(f32, f32) -> (usize, usize),
) {
    band_with(ui, f, pane, bg, origin, viewport, |r| {
        let (a, b) = range(r.top(), r.bottom());
        (a..b).collect()
    });
}

pub(crate) fn band_with(
    ui: &Ui,
    f: &mut Fx<'_>,
    pane: PaneId,
    bg: &Response,
    origin: f32,
    viewport: Rect,
    hits: impl Fn(Rect) -> Vec<usize>,
) {
    let pal = f.pal;
    let Some(pointer) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    let to_content = |p: Pos2| pos2(p.x, p.y - origin);
    if bg.drag_started() {
        f.vs.band = Some(Band {
            pane,
            origin: to_content(pointer),
            additive: ui.input(|i| i.modifiers.command),
            last: None,
        });
        f.send(Msg::FocusPane(pane));
    }
    let Some(mut b) = f.vs.band.filter(|b| b.pane == pane) else {
        return;
    };
    if bg.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
        f.vs.band = None;
        return;
    }
    let cur = to_content(pointer);
    let r = Rect::from_two_pos(b.origin, cur);
    let screen = r.translate(vec2(0.0, origin));
    ui.painter()
        .rect_filled(screen, 2.0, pal.accent.gamma_multiply(0.15));
    ui.painter().rect_stroke(
        screen,
        2.0,
        Stroke::new(1.0, pal.accent.gamma_multiply(0.8)),
        egui::StrokeKind::Inside,
    );
    let positions = hits(r);
    let key = (
        positions.first().copied().unwrap_or(usize::MAX),
        positions.len(),
    );
    if b.last != Some(key) {
        b.last = Some(key);
        f.send(Msg::SelectPositions {
            pane,
            positions,
            extend: b.additive,
        });
    }
    f.vs.band = Some(b);
    // Défilement automatique près des bords.
    let sc = f.vs.scroll.entry(pane).or_default();
    let top = origin + viewport.top();
    if pointer.y < top + 10.0 {
        sc.request = Some((sc.offset - 18.0).max(0.0));
        ui.ctx().request_repaint();
    } else if pointer.y > top + viewport.height() - 10.0 {
        sc.request = Some(sc.offset + 18.0);
        ui.ctx().request_repaint();
    }
}

/// Nom de type lisible, comme la colonne « Type » de l'Explorateur.
pub(crate) fn type_name(is_dir: bool, ext: &[u8]) -> String {
    if is_dir {
        return "Dossier de fichiers".into();
    }
    let e = std::str::from_utf8(ext).unwrap_or("");
    let known = match e {
        "" => return "Fichier".into(),
        "txt" => "Document texte",
        "log" => "Fichier journal",
        "pdf" => "Document PDF",
        "doc" | "docx" => "Document Microsoft Word",
        "xls" | "xlsx" => "Feuille de calcul Excel",
        "ppt" | "pptx" => "Présentation PowerPoint",
        "odt" => "Texte OpenDocument",
        "rtf" => "Document RTF",
        "md" | "markdown" => "Fichier Markdown",
        "csv" => "Fichier CSV",
        "json" => "Fichier JSON",
        "xml" => "Document XML",
        "toml" => "Fichier TOML",
        "yaml" | "yml" => "Fichier YAML",
        "ini" | "cfg" | "conf" => "Paramètres de configuration",
        "html" | "htm" => "Page web HTML",
        "css" => "Feuille de style CSS",
        "js" | "mjs" => "Fichier JavaScript",
        "ts" => "Fichier TypeScript",
        "rs" => "Fichier source Rust",
        "py" => "Fichier Python",
        "c" | "h" => "Fichier source C",
        "cpp" | "hpp" | "cc" => "Fichier source C++",
        "cs" => "Fichier source C#",
        "java" => "Fichier source Java",
        "go" => "Fichier source Go",
        "ps1" => "Script PowerShell",
        "bat" | "cmd" => "Fichier de commandes Windows",
        "sh" => "Script shell",
        "jpg" | "jpeg" => "Image JPEG",
        "png" => "Image PNG",
        "gif" => "Image GIF",
        "bmp" => "Image BMP",
        "webp" => "Image WebP",
        "heic" => "Image HEIC",
        "svg" => "Image SVG",
        "ico" => "Icône",
        "psd" => "Image Photoshop",
        "mp3" => "Fichier audio MP3",
        "wav" => "Fichier audio WAV",
        "flac" => "Fichier audio FLAC",
        "m4a" => "Fichier audio MPEG-4",
        "mp4" => "Vidéo MP4",
        "mkv" => "Vidéo Matroska",
        "mov" => "Vidéo QuickTime",
        "avi" => "Vidéo AVI",
        "webm" => "Vidéo WebM",
        "zip" => "Archive ZIP",
        "7z" => "Archive 7-Zip",
        "rar" => "Archive RAR",
        "tar" | "gz" | "tgz" => "Archive compressée",
        "iso" => "Image disque",
        "exe" => "Application",
        "msi" => "Package Windows Installer",
        "dll" => "Extension d'application",
        "lnk" => "Raccourci",
        "url" => "Raccourci Internet",
        "ttf" | "otf" => "Police",
        "db" | "sqlite" => "Base de données",
        _ => return format!("Fichier {}", e.to_ascii_uppercase()),
    };
    known.into()
}

/// État de la case « tout sélectionner » du volet.
fn select_all_state(pane: &Pane) -> crate::widgets::Check {
    let total = pane.listing.as_ref().map_or(0, DirSnapshot::len);
    let n = pane.selection.count() as usize;
    match n {
        0 => crate::widgets::Check::Off,
        n if n >= total && total > 0 => crate::widgets::Check::On,
        _ => crate::widgets::Check::Mixed,
    }
}

/// Case « tout sélectionner » : coche tout, ou décoche tout si tout est déjà sélectionné.
pub(crate) fn select_all_box(ui: &Ui, f: &mut Fx<'_>, pane: &Pane, center: Pos2) -> Response {
    let state = select_all_state(pane);
    let r = ui
        .interact(
            Rect::from_center_size(center, vec2(20.0, 20.0)),
            Id::unique(("select-all", pane.id.0)),
            Sense::click(),
        )
        .on_hover_text(if state == crate::widgets::Check::On {
            "Tout désélectionner"
        } else {
            "Tout sélectionner"
        });
    crate::widgets::check_box(ui, center, state, r.hovered(), &f.pal);
    if r.clicked() {
        f.send(Msg::FocusPane(pane.id));
        if state == crate::widgets::Check::On {
            f.send(Msg::SelectPositions {
                pane: pane.id,
                positions: Vec::new(),
                extend: false,
            });
        } else {
            f.cmd(fx_commands::CommandId::SelectAll);
        }
    }
    r
}

/// Bandeau « tout sélectionner » des vues sans en-tête (Liste, Grandes icônes).
pub(crate) fn select_all_strip(ui: &mut Ui, f: &mut Fx<'_>, pane: &Pane) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    // Au-dessus des icônes de la vue Liste, libellé aligné sur les noms.
    let center = pos2(rect.left() + PAD + 14.0, rect.center().y);
    let r = select_all_box(ui, f, pane, center);
    let label = ui.interact(
        Rect::from_min_max(
            pos2(center.x + 14.0, rect.top()),
            pos2(center.x + 140.0, rect.bottom()),
        ),
        Id::unique(("select-all-label", pane.id.0)),
        Sense::click(),
    );
    let color = if r.hovered() || label.hovered() {
        f.pal.text
    } else {
        f.pal.text2
    };
    ui.painter().text(
        pos2(center.x + 16.0, center.y),
        Align2::LEFT_CENTER,
        "Tout sélectionner",
        FontId::proportional(12.5),
        color,
    );
    if label.clicked() {
        f.send(Msg::FocusPane(pane.id));
        f.cmd(fx_commands::CommandId::SelectAll);
    }
}
