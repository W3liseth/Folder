//! Volet d'aperçu : image, texte et code colorés, Markdown, dossier, propriétés.

use std::fmt::Write as _;

use eframe::egui::{
    self, Align2, Color32, FontId, Frame, Margin, Rect, RichText, ScrollArea, Ui, pos2,
    text::LayoutJob, vec2,
};
use fx_preview::{Content, MdBlock, MdSpan, TokenKind, highlight};
use fx_shell::{ImageKey, image_request};

use super::Fx;
use crate::theme::{glyph, icon_font, semibold};
use crate::widgets;

/// Lignes de code affichées au plus (le reste est tronqué).
const MAX_LINES: usize = 1500;

pub fn draw(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    let width = f.state.settings.preview_width;
    let resp = egui::Panel::right("preview")
        .resizable(true)
        .default_size(width)
        .size_range(240.0..=720.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(pal.base).inner_margin(Margin {
            left: 0,
            right: 8,
            top: 4,
            bottom: 4,
        }))
        .show(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            ui.painter().rect_filled(rect, 8.0, pal.layer);
            ui.painter().rect_stroke(
                rect,
                8.0,
                egui::Stroke::new(1.0, pal.stroke),
                egui::StrokeKind::Inside,
            );
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(14.0)));
            content(&mut child, f);
        });
    let w = resp.response.rect.width();
    if (w - width).abs() > 1.0 && !ui.ctx().input(|i| i.pointer.any_down()) {
        f.send(super::settings_with(f.state, |s| s.preview_width = w));
    }
}

fn content(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    let Some(st) = f.state.preview.clone() else {
        super::pane::placeholder(
            ui,
            f,
            glyph::PREVIEW,
            "Aperçu",
            "Sélectionnez un élément pour l'afficher ici.",
        );
        return;
    };
    let path = st.path.clone();
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    f.vs.fallbacks.scan(&name);
    let is_dir = st
        .preview
        .as_ref()
        .is_some_and(|p| p.attributes & 0x10 != 0);
    ScrollArea::vertical()
        .id_salt("preview")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // En-tête : grande icône (ou miniature), nom, type.
            let w = ui.available_width();
            let is_image = matches!(
                st.preview.as_ref().map(|p| &p.content),
                Some(Content::Image)
            );
            let big = if is_image { (w - 8.0).min(420.0) } else { 72.0 };
            let (rect, _) = ui.allocate_exact_size(vec2(w, big + 8.0), egui::Sense::hover());
            let ppp = ui.ctx().pixels_per_point();
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let dir = path.parent().unwrap_or(&path).to_path_buf();
            let key = if is_image {
                let p = st.preview.as_ref();
                ImageKey::Thumb {
                    path: path.clone(),
                    size: (big * ppp) as u16,
                    modified: p.map_or(0, |p| p.modified),
                    len: p.map_or(0, |p| p.size),
                }
            } else {
                image_request(&dir, &name, &ext, is_dir, (64.0 * ppp) as u16).0
            };
            match f.images.get(f.shell, &key, &path) {
                Some(tex) => {
                    let ts = tex.size_vec2() / ppp;
                    let scale = (big / ts.x.max(1.0))
                        .min(big / ts.y.max(1.0))
                        .min(if is_image { 1.0 } else { 72.0 / ts.x.max(1.0) });
                    let r = Rect::from_center_size(rect.center(), ts * scale);
                    ui.painter().image(
                        tex.id(),
                        r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    let (g, c) = if is_dir {
                        (glyph::FOLDER, pal.folder)
                    } else {
                        (glyph::FILE, pal.text3)
                    };
                    ui.painter()
                        .text(rect.center(), Align2::CENTER_CENTER, g, icon_font(56.0), c);
                }
            }
            ui.add_space(6.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(&name).font(semibold(16.0)).color(pal.text));
                let kind = super::list::type_name(is_dir, ext.as_bytes());
                ui.label(RichText::new(kind).color(pal.text2));
            });
            ui.add_space(10.0);

            let Some(p) = &st.preview else {
                ui.vertical_centered(|ui| ui.spinner());
                return;
            };
            match &p.content {
                Content::Text {
                    text,
                    truncated,
                    lang,
                } => code(ui, f, text, *lang, *truncated),
                Content::Markdown(blocks) => markdown(ui, f, blocks),
                Content::Folder {
                    files,
                    dirs,
                    truncated,
                } => {
                    let more = if *truncated { "+" } else { "" };
                    widgets::section(ui, &pal, "Contenu");
                    ui.label(format!("{files}{more} fichier(s), {dirs}{more} dossier(s)"));
                }
                Content::Image | Content::None => {}
            }
            ui.add_space(10.0);
            properties(ui, f, p);
        });
}

fn properties(ui: &mut Ui, f: &mut Fx<'_>, p: &fx_preview::Preview) {
    let pal = f.pal;
    widgets::section(ui, &pal, "Propriétés");
    let date = |t: i64| {
        fx_win::time::filetime_to_local(t).map_or_else(String::new, |d| {
            format!(
                "{:02}/{:02}/{} {:02}:{:02}",
                d.day, d.month, d.year, d.hour, d.minute
            )
        })
    };
    let mut attrs = String::new();
    for (bit, label) in [
        (0x1, "Lecture seule"),
        (0x2, "Masqué"),
        (0x4, "Système"),
        (0x20, "Archive"),
        (0x400, "Lien"),
        (0x800, "Compressé"),
        (0x4000, "Chiffré"),
    ] {
        if p.attributes & bit != 0 {
            if !attrs.is_empty() {
                attrs.push_str(", ");
            }
            attrs.push_str(label);
        }
    }
    egui::Grid::new("props")
        .num_columns(2)
        .spacing(vec2(12.0, 6.0))
        .show(ui, |ui| {
            let mut kv = |k: &str, v: String| {
                ui.label(RichText::new(k).color(pal.text2));
                ui.add(egui::Label::new(v).selectable(true));
                ui.end_row();
            };
            if p.attributes & 0x10 == 0 {
                let mut s = super::sidebar::human(p.size);
                let _ = write!(s, " ({} octets)", p.size);
                kv("Taille", s);
            }
            kv("Modifié", date(p.modified));
            kv("Créé", date(p.created));
            if !attrs.is_empty() {
                kv("Attributs", attrs);
            }
            kv(
                "Emplacement",
                p.path
                    .parent()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default(),
            );
        });
    let tags = f.state.tags.of(&p.path).to_vec();
    if !tags.is_empty() {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for t in tags.iter().filter_map(|id| f.state.tags.get(*id)) {
                let c = Color32::from_rgb(t.color[0], t.color[1], t.color[2]);
                ui.label(RichText::new(format!("● {}", t.name)).color(c));
            }
        });
    }
}

fn code(ui: &mut Ui, f: &Fx<'_>, text: &str, lang: fx_preview::Lang, truncated: bool) {
    let pal = f.pal;
    let mut end = text.len();
    let mut lines_cut = false;
    if let Some((i, _)) = text.match_indices('\n').nth(MAX_LINES) {
        end = i;
        lines_cut = true;
    }
    let text = &text[..end];
    let colors = |k: TokenKind| -> Color32 {
        match (k, pal.dark) {
            (TokenKind::Comment, true) => Color32::from_rgb(106, 153, 85),
            (TokenKind::Comment, false) => Color32::from_rgb(0, 128, 0),
            (TokenKind::String, true) => Color32::from_rgb(206, 145, 120),
            (TokenKind::String, false) => Color32::from_rgb(163, 21, 21),
            (TokenKind::Number, true) => Color32::from_rgb(181, 206, 168),
            (TokenKind::Number, false) => Color32::from_rgb(9, 134, 88),
            (TokenKind::Keyword, true) => Color32::from_rgb(86, 156, 214),
            (TokenKind::Keyword, false) => Color32::from_rgb(0, 0, 255),
            (TokenKind::Tag, true) => Color32::from_rgb(78, 201, 176),
            (TokenKind::Tag, false) => Color32::from_rgb(128, 0, 0),
        }
    };
    let font = FontId::monospace(12.5);
    let mut job = LayoutJob::default();
    let mut at = 0;
    for t in highlight(text, lang) {
        if t.range.start > at {
            job.append(
                &text[at..t.range.start],
                0.0,
                egui::TextFormat::simple(font.clone(), pal.text),
            );
        }
        job.append(
            &text[t.range.clone()],
            0.0,
            egui::TextFormat::simple(font.clone(), colors(t.kind)),
        );
        at = t.range.end;
    }
    if at < text.len() {
        job.append(&text[at..], 0.0, egui::TextFormat::simple(font, pal.text));
    }
    job.wrap.max_width = ui.available_width() - 16.0;
    Frame::new()
        .fill(pal.card)
        .corner_radius(4)
        .inner_margin(8)
        .stroke(egui::Stroke::new(1.0, pal.stroke))
        .show(ui, |ui| {
            ui.add(egui::Label::new(job).selectable(true));
        });
    if truncated || lines_cut {
        ui.label(RichText::new("Aperçu tronqué").small().color(pal.text3));
    }
}

fn spans_job(spans: &[MdSpan], size: f32, f: &Fx<'_>, wrap: f32) -> LayoutJob {
    let pal = f.pal;
    let mut job = LayoutJob::default();
    for s in spans {
        let font = if s.code {
            FontId::monospace(size - 1.0)
        } else if s.bold {
            semibold(size)
        } else {
            FontId::proportional(size)
        };
        let mut fmt = egui::TextFormat::simple(font, if s.link { pal.accent } else { pal.text });
        fmt.italics = s.italic;
        if s.code {
            fmt.background = pal.card;
        }
        if s.link {
            fmt.underline = egui::Stroke::new(1.0, pal.accent);
        }
        job.append(&s.text, 0.0, fmt);
    }
    job.wrap.max_width = wrap;
    job
}

fn markdown(ui: &mut Ui, f: &Fx<'_>, blocks: &[MdBlock]) {
    let pal = f.pal;
    let w = ui.available_width() - 8.0;
    for b in blocks.iter().take(400) {
        match b {
            MdBlock::Heading(level, spans) => {
                ui.add_space(6.0);
                let size = match level {
                    1 => 22.0,
                    2 => 18.0,
                    3 => 16.0,
                    _ => 14.5,
                };
                let mut bold: Vec<MdSpan> = spans.clone();
                for s in &mut bold {
                    s.bold = true;
                }
                ui.add(egui::Label::new(spans_job(&bold, size, f, w)).selectable(true));
            }
            MdBlock::Paragraph(spans) => {
                ui.add(egui::Label::new(spans_job(spans, 13.5, f, w)).selectable(true));
            }
            MdBlock::Item {
                depth,
                number,
                spans,
            } => {
                ui.horizontal_top(|ui| {
                    ui.add_space(8.0 + f32::from(*depth) * 16.0);
                    ui.label(
                        RichText::new(number.map_or_else(|| "•".to_owned(), |n| format!("{n}.")))
                            .color(pal.text2),
                    );
                    ui.add(egui::Label::new(spans_job(spans, 13.5, f, w - 40.0)).selectable(true));
                });
            }
            MdBlock::Quote(spans) => {
                Frame::new()
                    .inner_margin(Margin {
                        left: 10,
                        right: 4,
                        top: 2,
                        bottom: 2,
                    })
                    .show(ui, |ui| {
                        let r = ui.available_rect_before_wrap();
                        ui.painter().vline(
                            r.left() - 8.0,
                            r.y_range(),
                            egui::Stroke::new(3.0, pal.stroke_strong),
                        );
                        ui.add(
                            egui::Label::new(spans_job(spans, 13.5, f, w - 20.0)).selectable(true),
                        );
                    });
            }
            MdBlock::Code { text, lang } => {
                code(ui, f, text, fx_preview::Lang::from_ext(lang), false)
            }
            MdBlock::Rule => {
                ui.separator();
            }
        }
        ui.add_space(4.0);
    }
}
