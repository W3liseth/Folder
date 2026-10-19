//! Petits widgets au style Fluent partagés par les vues.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    pos2, vec2,
};

use crate::theme::{Palette, icon_font};

/// Fond de survol animé (fondu bref) pour un élément interactif.
pub fn hover_fill(ui: &Ui, resp: &Response, rect: Rect, pal: &Palette, radius: f32) {
    let t = ui.ctx().animate_bool_with_time(
        resp.id.with("hover"),
        resp.hovered() || resp.has_focus(),
        0.08,
    );
    if resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, radius, pal.pressed);
    } else if t > 0.0 {
        ui.painter()
            .rect_filled(rect, radius, pal.hover.gamma_multiply(t));
    }
}

/// Bouton icône plat (barres d'outils, navigation).
pub fn icon_button(
    ui: &mut Ui,
    pal: &Palette,
    glyph: &str,
    tip: &str,
    enabled: bool,
    size: egui::Vec2,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if ui.is_rect_visible(rect) {
        if enabled {
            hover_fill(ui, &resp, rect, pal, 4.0);
        }
        let color = if enabled { pal.text } else { pal.text3 };
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            icon_font(15.0),
            color,
        );
    }
    let resp = if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    };
    if enabled {
        resp
    } else {
        resp.on_hover_cursor(egui::CursorIcon::Default)
    }
}

/// Bouton de barre de commandes : glyphe + libellé (style Files / Explorateur).
pub fn tool_button(
    ui: &mut Ui,
    pal: &Palette,
    glyph: &str,
    glyph_color: Option<Color32>,
    label: &str,
    enabled: bool,
) -> Response {
    let font = FontId::proportional(13.5);
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        font,
        if enabled { pal.text } else { pal.text3 },
    );
    let w = 10.0
        + 16.0
        + if label.is_empty() {
            10.0
        } else {
            8.0 + galley.size().x + 12.0
        };
    let (rect, resp) = ui.allocate_exact_size(
        vec2(w, 32.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if ui.is_rect_visible(rect) {
        if enabled {
            hover_fill(ui, &resp, rect, pal, 4.0);
        }
        let gc = if enabled {
            glyph_color.unwrap_or(pal.text)
        } else {
            pal.text3
        };
        let gx = rect.left() + 10.0 + 8.0;
        ui.painter().text(
            pos2(gx, rect.center().y),
            Align2::CENTER_CENTER,
            glyph,
            icon_font(15.0),
            gc,
        );
        if !label.is_empty() {
            ui.painter().galley(
                pos2(gx + 16.0, rect.center().y - galley.size().y / 2.0),
                galley,
                pal.text,
            );
        }
    }
    resp
}

/// Bouton de commande avec chevron (menu déroulant).
pub fn menu_label(ui: &mut Ui, pal: &Palette, glyph: &str, label: &str) -> Response {
    let resp = tool_button(ui, pal, glyph, None, &format!("{label}  "), true);
    let r = resp.rect;
    ui.painter().text(
        pos2(r.right() - 12.0, r.center().y + 1.0),
        Align2::CENTER_CENTER,
        crate::theme::glyph::CHEVRON_DOWN,
        icon_font(8.0),
        pal.text2,
    );
    resp
}

/// Bouton d'accent (action principale) : fond d'accent avec reflet, halo au survol,
/// chevron s'il ouvre un menu.
pub fn accent_button(ui: &mut Ui, pal: &Palette, glyph: &str, label: &str, menu: bool) -> Response {
    let font = FontId::proportional(13.5);
    let text_w = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x;
    let w = 14.0 + 16.0 + 8.0 + text_w + if menu { 22.0 } else { 14.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.12);
    let fill = crate::theme::mix(pal.accent, Color32::WHITE, 0.10 * hover);
    let fill = if resp.is_pointer_button_down_on() {
        crate::theme::mix(pal.accent, Color32::BLACK, 0.12)
    } else {
        fill
    };
    if hover > 0.0 {
        glow(ui, rect, 8.0, pal.accent.gamma_multiply(0.45 * hover), 14.0);
    }
    ui.painter().rect_filled(rect, 8.0, fill);
    // Reflet vertical sur la partie droite du bouton (bords droits : sans les arrondis).
    vgradient(
        ui,
        rect.shrink2(vec2(8.0, 1.0)),
        Color32::from_white_alpha(34),
        Color32::TRANSPARENT,
    );
    let on = if pal.dark {
        pal.on_accent
    } else {
        Color32::WHITE
    };
    ui.painter().text(
        pos2(rect.left() + 14.0 + 8.0, rect.center().y),
        Align2::CENTER_CENTER,
        glyph,
        icon_font(13.0),
        on,
    );
    ui.painter().text(
        pos2(rect.left() + 14.0 + 16.0 + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        on,
    );
    if menu {
        ui.painter().text(
            pos2(rect.right() - 13.0, rect.center().y + 1.0),
            Align2::CENTER_CENTER,
            crate::theme::glyph::CHEVRON_DOWN,
            icon_font(8.0),
            on,
        );
    }
    resp
}

/// Séparateur vertical de barre d'outils.
pub fn vsep(ui: &mut Ui, pal: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 20.0), Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        Stroke::new(1.0, pal.stroke_strong),
    );
}

/// Titre de section (barre latérale, accueil).
pub fn section(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(caps_job(text, 10.5, pal.text3));
    });
    ui.add_space(3.0);
}

/// Petites capitales espacées (titres de section, en-têtes de colonnes).
pub fn caps_job(text: &str, size: f32, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let mut fmt = egui::TextFormat::simple(crate::theme::semibold(size), color);
    fmt.extra_letter_spacing = size * 0.09;
    job.append(&text.to_uppercase(), 0.0, fmt);
    job
}

/// Dessine des petites capitales espacées ; renvoie leur rectangle.
pub fn caps(
    ui: &Ui,
    pos: egui::Pos2,
    anchor: Align2,
    text: &str,
    size: f32,
    color: Color32,
) -> Rect {
    let galley = ui.painter().layout_job(caps_job(text, size, color));
    let rect = anchor.anchor_size(pos, galley.size());
    ui.painter().galley(rect.min, galley, color);
    rect
}

/// Dégradé horizontal sur un rectangle aux bords droits (les bords obliques ou arrondis
/// d'un maillage ne sont pas lissés : on réserve les dégradés aux formes droites).
pub fn hgradient(ui: &Ui, rect: Rect, left: Color32, right: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    ui.painter().add(mesh);
}

/// Dégradé vertical (haut → bas) sur un rectangle aux bords droits.
pub fn vgradient(ui: &Ui, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    ui.painter().add(mesh);
}

/// Liseré « néon » : ligne d'accent qui s'estompe vers les deux extrémités.
pub fn neon_line(ui: &Ui, x: std::ops::RangeInclusive<f32>, y: f32, height: f32, accent: Color32) {
    let (x0, x1) = (*x.start(), *x.end());
    let mid = (x0 + x1) / 2.0;
    let rect = |a: f32, b: f32| Rect::from_min_max(egui::pos2(a, y), egui::pos2(b, y + height));
    let clear = Color32::TRANSPARENT;
    hgradient(ui, rect(x0, mid), clear, accent);
    hgradient(ui, rect(mid, x1), accent, clear);
}

/// Halo doux autour d'un rectangle (ombre colorée), sous le contenu déjà réservé.
pub fn glow(ui: &Ui, rect: Rect, radius: f32, color: Color32, blur: f32) {
    let shadow = egui::Shadow {
        offset: [0, 0],
        blur: blur as u8,
        spread: 0,
        color,
    };
    ui.painter()
        .add(shadow.as_shape(rect, CornerRadius::same(radius as u8)));
}

/// Pastille d'accent verticale (élément actif) avec un léger halo.
pub fn accent_pill(ui: &Ui, center: egui::Pos2, height: f32, accent: Color32) {
    let pill = Rect::from_center_size(center, vec2(3.0, height));
    glow(ui, pill, 1.5, accent.gamma_multiply(0.55), 8.0);
    ui.painter().rect_filled(pill, 1.5, accent);
}

/// Barre de progression arrondie ; `None` : indéterminée (animée).
pub fn progress(ui: &Ui, rect: Rect, fraction: Option<f32>, color: Color32, track: Color32) {
    let p = ui.painter();
    let r = CornerRadius::same((rect.height() / 2.0) as u8);
    p.rect_filled(rect, r, track);
    match fraction {
        Some(f) => {
            let mut fill = rect;
            fill.set_width((rect.width() * f.clamp(0.0, 1.0)).max(rect.height()));
            p.rect_filled(fill, r, color);
            // Reflet : dégradé clair sur la partie centrale (bords droits, sans les arrondis).
            let inner = fill.shrink2(vec2(rect.height() / 2.0, 0.0));
            if inner.width() > 1.0 {
                hgradient(
                    ui,
                    inner,
                    Color32::TRANSPARENT,
                    Color32::from_white_alpha(40),
                );
            }
        }
        None => {
            let t = (ui.input(|i| i.time) as f32 * 0.8).fract();
            let w = rect.width() * 0.3;
            let x = rect.left() - w + (rect.width() + w) * t;
            let seg = Rect::from_x_y_ranges(
                x.max(rect.left())..=(x + w).min(rect.right()),
                rect.y_range(),
            );
            if seg.width() > 0.0 {
                p.rect_filled(seg, r, color);
            }
            ui.ctx().request_repaint();
        }
    }
}

/// Jauge en dégradé (espace utilisé des lecteurs) : le dégradé couvre toute la piste, la partie
/// remplie s'arrête sur la couleur atteinte. Extrémités arrondies peintes en couleur unie, maillage
/// seulement entre elles (bords droits) ; reflet sur la moitié haute des jauges épaisses.
pub fn gradient_bar(ui: &Ui, rect: Rect, fraction: f32, color: Color32, track: Color32) {
    let p = ui.painter();
    let h = rect.height();
    let r = CornerRadius::same((h / 2.0).round() as u8);
    p.rect_filled(rect, r, track);
    let (from, to) = crate::theme::gradient_pair(color);
    let w = (rect.width() * fraction.clamp(0.0, 1.0)).max(h);
    let end = crate::theme::mix(from, to, (w / rect.width()).min(1.0));
    let cap = |x: f32, c: Color32| {
        p.rect_filled(
            Rect::from_min_size(egui::pos2(x, rect.top()), vec2(h, h)),
            r,
            c,
        );
    };
    cap(rect.left(), from);
    cap(rect.left() + w - h, end);
    let mid = Rect::from_x_y_ranges(
        rect.left() + h / 2.0..=rect.left() + w - h / 2.0,
        rect.y_range(),
    );
    if mid.width() > 0.5 {
        hgradient(ui, mid, from, end);
        if h >= 4.0 {
            let top = Rect::from_x_y_ranges(mid.x_range(), rect.top()..=rect.center().y);
            vgradient(ui, top, Color32::from_white_alpha(45), Color32::TRANSPARENT);
        }
    }
}

/// Curseur de réglage opaque (style Windows 11) : piste pleine, partie active en dégradé
/// d'accent, poignée pleine dont le point d'accent grossit au survol, valeur affichée à droite.
/// Glisser, cliquer, molette ou flèches (au focus) ; la valeur est arrondie au pas.
pub fn slider(
    ui: &mut Ui,
    pal: &Palette,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    step: f32,
    suffix: &str,
) -> Response {
    let (lo, hi) = (*range.start(), *range.end());
    let (rect, mut resp) = ui.allocate_exact_size(vec2(220.0, 28.0), Sense::click_and_drag());
    let track = Rect::from_min_max(
        pos2(rect.left() + 9.0, rect.center().y - 2.0),
        pos2(rect.right() - 62.0, rect.center().y + 2.0),
    );
    let snap = |v: f32| ((v - lo) / step).round().mul_add(step, lo).clamp(lo, hi);
    let old = *value;
    if let Some(p) = resp.interact_pointer_pos() {
        let t = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0);
        *value = snap(egui::lerp(lo..=hi, t));
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
        if scroll != 0.0 {
            *value = snap(*value + step * scroll.signum());
        }
    }
    if resp.has_focus() {
        ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowUp) {
                *value = snap(*value + step);
            }
            if i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowDown) {
                *value = snap(*value - step);
            }
        });
    }
    if *value != old {
        resp.mark_changed();
    }
    let t = ((*value - lo) / (hi - lo)).clamp(0.0, 1.0);
    gradient_bar(ui, track, t, pal.accent, pal.stroke_strong);
    let knob = pos2(egui::lerp(track.x_range(), t), track.center().y);
    let grow =
        ui.ctx()
            .animate_bool_with_time(resp.id.with("k"), resp.hovered() || resp.dragged(), 0.1);
    let p = ui.painter();
    p.circle_filled(knob + vec2(0.0, 1.0), 10.0, Color32::from_black_alpha(50));
    p.circle_filled(knob, 10.0, pal.card);
    p.circle_stroke(knob, 10.0, Stroke::new(1.0, pal.stroke_strong));
    let inner = if resp.is_pointer_button_down_on() {
        4.5
    } else {
        5.0 + 1.5 * grow
    };
    p.circle_filled(knob, inner, pal.accent);
    let text = if (step - step.round()).abs() < f32::EPSILON {
        format!("{value:.0}{suffix}")
    } else {
        format!("{value:.1}{suffix}")
    };
    p.text(
        pos2(rect.right(), rect.center().y),
        Align2::RIGHT_CENTER,
        text,
        FontId::proportional(13.0),
        pal.text,
    );
    resp
}

/// Sélecteur de couleur complet et compact : carré saturation × luminosité, barre de teinte,
/// aperçu et saisie hexadécimale. Retourne la couleur choisie quand elle change. La teinte est
/// mémorisée entre deux images (elle se perd sur un gris ou un noir).
pub fn color_picker(ui: &mut Ui, pal: &Palette, color: Color32) -> Option<Color32> {
    use egui::ecolor::HsvaGamma;
    let pure = |h: f32| {
        Color32::from(HsvaGamma {
            h,
            s: 1.0,
            v: 1.0,
            a: 1.0,
        })
    };
    let id = ui.scope_id().with("color-picker");
    let mut hsv = ui
        .data(|d| d.get_temp::<HsvaGamma>(id))
        .filter(|h| Color32::from(*h) == color)
        .unwrap_or_else(|| HsvaGamma::from(color));
    let before = hsv;
    let mut typed = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            // Carré : saturation de gauche à droite, luminosité de bas en haut. Maillage de
            // 16 × 16 cases (les couleurs sont interpolées entre les sommets).
            let (rect, r) = ui.allocate_exact_size(vec2(280.0, 150.0), Sense::click_and_drag());
            if let Some(p) = r.interact_pointer_pos() {
                hsv.s = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                hsv.v = (1.0 - (p.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            }
            const N: u32 = 16;
            let mut mesh = egui::Mesh::default();
            for j in 0..=N {
                for i in 0..=N {
                    let (x, y) = (i as f32 / N as f32, j as f32 / N as f32);
                    mesh.colored_vertex(
                        rect.lerp_inside(vec2(x, y)),
                        Color32::from(HsvaGamma {
                            h: hsv.h,
                            s: x,
                            v: 1.0 - y,
                            a: 1.0,
                        }),
                    );
                }
            }
            for j in 0..N {
                for i in 0..N {
                    let k = j * (N + 1) + i;
                    mesh.add_triangle(k, k + 1, k + N + 1);
                    mesh.add_triangle(k + 1, k + N + 2, k + N + 1);
                }
            }
            ui.painter().add(mesh);
            ui.painter().rect_stroke(
                rect,
                0.0,
                Stroke::new(1.0, pal.stroke_strong),
                StrokeKind::Outside,
            );
            let at = rect.lerp_inside(vec2(hsv.s, 1.0 - hsv.v));
            ui.painter()
                .circle_stroke(at, 7.0, Stroke::new(3.0, Color32::from_black_alpha(90)));
            ui.painter()
                .circle_stroke(at, 7.0, Stroke::new(1.5, Color32::WHITE));

            // Barre de teinte : 12 dégradés entre teintes pures.
            let (bar, r) = ui.allocate_exact_size(vec2(280.0, 12.0), Sense::click_and_drag());
            if let Some(p) = r.interact_pointer_pos() {
                hsv.h = ((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
            }
            const STOPS: usize = 12;
            for k in 0..STOPS {
                let (a, b) = (k as f32 / STOPS as f32, (k + 1) as f32 / STOPS as f32);
                let seg = Rect::from_x_y_ranges(
                    egui::lerp(bar.x_range(), a)..=egui::lerp(bar.x_range(), b),
                    bar.y_range(),
                );
                hgradient(ui, seg, pure(a), pure(b));
            }
            let knob = Rect::from_center_size(
                pos2(egui::lerp(bar.x_range(), hsv.h), bar.center().y),
                vec2(8.0, bar.height() + 8.0),
            );
            ui.painter().rect_filled(knob, 4.0, pure(hsv.h));
            ui.painter().rect_stroke(
                knob,
                4.0,
                Stroke::new(2.0, Color32::WHITE),
                StrokeKind::Outside,
            );
        });
        ui.add_space(14.0);
        ui.vertical(|ui| {
            let c = Color32::from(hsv);
            let (sw, _) = ui.allocate_exact_size(vec2(110.0, 54.0), Sense::hover());
            ui.painter().rect_filled(sw, 8.0, c);
            ui.painter().rect_stroke(
                sw,
                8.0,
                Stroke::new(1.0, pal.stroke_strong),
                StrokeKind::Inside,
            );
            ui.add_space(8.0);
            // Saisie hexadécimale : le texte tapé est conservé tant que le champ a le focus,
            // appliqué dès qu'il forme une couleur valide.
            let hex_id = id.with("hex");
            let current = format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
            let mut text = ui
                .data(|d| d.get_temp::<String>(hex_id))
                .unwrap_or_else(|| current.clone());
            let te = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(110.0 - 8.0)
                    .font(FontId::monospace(13.0))
                    .char_limit(7),
            );
            if te.changed() {
                typed = parse_hex(&text);
            }
            if !te.has_focus() {
                text = current;
            }
            ui.data_mut(|d| d.insert_temp(hex_id, text));
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("R {}  V {}  B {}", c.r(), c.g(), c.b()))
                    .size(12.0)
                    .color(pal.text2),
            );
        });
    });
    if let Some(c) = typed {
        hsv = HsvaGamma::from(c);
    }
    ui.data_mut(|d| d.insert_temp(id, hsv));
    (typed.is_some() || hsv != before).then(|| Color32::from(hsv))
}

/// « #RRGGBB » (dièse facultatif) ; `None` si incomplet ou invalide.
fn parse_hex(text: &str) -> Option<Color32> {
    let t = text.trim().trim_start_matches('#');
    if t.len() != 6 || !t.is_ascii() {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&t[i..i + 2], 16).ok();
    Some(Color32::from_rgb(c(0)?, c(2)?, c(4)?))
}

/// Puce colorée (étiquettes).
/// Icône d'étiquette (même glyphe que le menu contextuel), dans la couleur de l'étiquette.
pub fn tag_icon(ui: &Ui, center: egui::Pos2, size: f32, color: [u8; 3]) {
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        crate::theme::glyph::TAG,
        icon_font(size),
        Color32::from_rgb(color[0], color[1], color[2]),
    );
}

/// Couleurs proposées pour les étiquettes (celles de Files et de l'Explorateur).
pub const TAG_COLORS: [[u8; 3]; 10] = [
    [232, 17, 35],
    [247, 99, 12],
    [255, 185, 0],
    [16, 137, 62],
    [0, 153, 188],
    [0, 120, 212],
    [136, 23, 152],
    [227, 0, 140],
    [122, 117, 116],
    [134, 100, 71],
];

pub fn tag_dot(ui: &Ui, center: egui::Pos2, radius: f32, color: [u8; 3]) {
    ui.painter().circle_filled(
        center,
        radius,
        Color32::from_rgb(color[0], color[1], color[2]),
    );
}

/// Contour de focus (volet actif en double volet).
pub fn focus_ring(ui: &Ui, rect: Rect, pal: &Palette) {
    ui.painter().rect_stroke(
        rect,
        8.0,
        Stroke::new(1.5, pal.accent.gamma_multiply(0.8)),
        StrokeKind::Inside,
    );
}

/// Courbe des animations : départ vif, arrivée douce.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Apparition d'un menu (contextuel ou déroulant) : léger zoom et fondu en 140 ms, depuis
/// son coin supérieur gauche. À appeler en tête du contenu du menu.
pub fn menu_appear(ui: &mut Ui) {
    const DURATION: f64 = 0.14;
    let ctx = ui.ctx().clone();
    let layer = ui.layer_id();
    let id = egui::Id::unique(("menu-appear", layer.id));
    let frame = ctx.cumulative_frame_nr();
    let now = ctx.input(|i| i.time);
    // Un menu non dessiné à l'image précédente vient de s'ouvrir : l'animation repart.
    let start = match ctx.data(|d| d.get_temp::<(f64, u64)>(id)) {
        Some((start, last)) if last + 1 >= frame => start,
        _ => now,
    };
    ctx.data_mut(|d| d.insert_temp(id, (start, frame)));
    let t = ((now - start) / DURATION) as f32;
    let e = ease_out(t);
    if t < 1.0 {
        ctx.request_repaint();
    }
    let pivot = ui.max_rect().min.to_vec2();
    let transform = egui::emath::TSTransform::from_translation(pivot + vec2(0.0, -6.0 * (1.0 - e)))
        * egui::emath::TSTransform::from_scaling(0.95 + 0.05 * e)
        * egui::emath::TSTransform::from_translation(-pivot);
    ctx.set_transform_layer(layer, transform);
    ui.set_opacity(e);
}

/// État d'une case à cocher d'élément (sélection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Off,
    On,
    /// Sélection partielle (case « tout sélectionner »).
    Mixed,
}

/// Case à cocher de sélection (style Fluent, 16 points) centrée sur `center`. `hot` : survolée.
pub fn check_box(ui: &Ui, center: egui::Pos2, state: Check, hot: bool, pal: &Palette) {
    let r = Rect::from_center_size(center, vec2(16.0, 16.0));
    let p = ui.painter();
    match state {
        Check::Off => {
            p.rect_filled(r, 4.0, if hot { pal.hover } else { pal.card });
            p.rect_stroke(
                r,
                4.0,
                Stroke::new(1.0, if hot { pal.text } else { pal.text2 }),
                StrokeKind::Inside,
            );
        }
        Check::On | Check::Mixed => {
            p.rect_filled(r, 4.0, pal.accent);
            let g = if state == Check::On {
                crate::theme::glyph::CHECK
            } else {
                "\u{E738}"
            };
            p.text(
                center,
                Align2::CENTER_CENTER,
                g,
                icon_font(10.0),
                Color32::WHITE,
            );
        }
    }
}

/// Libellé d'entrée de menu façon Windows 11 : une colonne d'icône (vide si `glyph` est
/// vide) puis le texte, pour que le texte ne colle pas au bord gauche du menu.
pub fn menu_entry(glyph: &str, text: &str, pal: &Palette) -> egui::text::LayoutJob {
    menu_entry_colored(glyph, pal.text, text, pal)
}

/// [`menu_entry`] avec une couleur d'icône choisie (coche, pastille d'étiquette…).
pub fn menu_entry_colored(
    glyph: &str,
    glyph_color: Color32,
    text: &str,
    pal: &Palette,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    // Les glyphes de Segoe Fluent Icons ont tous la même chasse : une coche transparente
    // réserve exactement la place d'une icône.
    let (g, color) = if glyph.is_empty() {
        (crate::theme::glyph::CHECK, Color32::TRANSPARENT)
    } else {
        (glyph, glyph_color)
    };
    let mut fmt = egui::TextFormat::simple(icon_font(14.0), color);
    fmt.valign = egui::Align::Center;
    job.append(g, 6.0, fmt);
    job.append(
        text,
        12.0,
        egui::TextFormat::simple(FontId::proportional(14.0), pal.text),
    );
    job
}

/// Entrée de menu cochable : la coche occupe la colonne d'icône.
pub fn check_label(on: bool, text: &str, pal: &Palette) -> egui::text::LayoutJob {
    let g = if on { crate::theme::glyph::CHECK } else { "" };
    menu_entry_colored(g, pal.accent, text, pal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex("#0078D4"), Some(Color32::from_rgb(0, 120, 212)));
        assert_eq!(parse_hex("ff8000"), Some(Color32::from_rgb(255, 128, 0)));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#12345G"), None);
        assert_eq!(parse_hex("#éé1234"), None);
    }
}
