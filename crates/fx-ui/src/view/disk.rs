//! Analyse de l'espace (ARCHITECTURE.md §7.13) : en-tête (totaux, pages, actions), puis
//! selon la page : carte proportionnelle (treemap) du dossier regardé au-dessus de sa
//! liste, place par type de fichier, ou suggestions de nettoyage.
//!
//! La carte est une image (pixels physiques) calculée seulement quand l'arbre, le dossier
//! ou la taille changent, comme WinDirStat : à l'affichage, un seul rectangle texturé,
//! alors qu'un maillage de dizaines de milliers de tuiles coûtait 12 à 16 ms de
//! rasterisation CPU à chaque survol en 1080p. Les tuiles de moins de [`MIN_TILE`] points²
//! ne sont pas subdivisées, ce qui borne leur nombre par la surface.

use std::fmt::Write as _;
use std::path::Path;

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, Ui, pos2, vec2,
};
use fx_app::disk::{DiskScan, DiskStatus, DiskTab};
use fx_app::{Location, Msg, Pane, PaneId};
use fx_core::disktree::{DiskTree, NodeKind, flags};
use fx_core::{format, wtf8};
use fx_fs::cleanup::CleanMode;

use super::{Fx, list};
use crate::theme::{glyph, icon_font, semibold};
use crate::widgets;

/// Surface minimale (px²) d'une tuile subdivisée : en dessous, le dossier est une tuile.
const MIN_TILE: f32 = 36.0;
/// Extensions qui ont leur couleur (les plus gourmandes) ; les autres sont grises.
const COLORED_EXTS: usize = 12;

/// Couleurs de la carte, saturées mais lisibles sur les deux thèmes.
const TILE_COLORS: [[u8; 3]; COLORED_EXTS] = [
    [66, 133, 244],
    [234, 67, 53],
    [52, 168, 83],
    [251, 188, 5],
    [0, 172, 193],
    [171, 71, 188],
    [255, 112, 67],
    [124, 179, 66],
    [92, 107, 192],
    [236, 64, 122],
    [38, 166, 154],
    [141, 110, 99],
];

/// Carte calculée d'un volet.
#[derive(Default)]
pub(crate) struct TreemapCache {
    /// (analyse, révision de l'arbre, dossier, taille en pixels physiques, thème sombre)
    key: Option<(u64, u64, u32, [u32; 2], bool)>,
    texture: Option<egui::TextureHandle>,
    texture_bytes: usize,
    /// Taille (points, pixels) pour laquelle tuiles et image ont été calculées : pendant un
    /// redimensionnement, l'image est étirée et les tuiles mises à l'échelle.
    built: egui::Vec2,
    built_px: [u32; 2],
    /// Nouvelle taille en attente et depuis quand (recalcul quand elle ne bouge plus).
    pending: Option<([u32; 2], std::time::Instant)>,
    /// (rectangle relatif au coin de la carte, en points ; nœud)
    tiles: Vec<(Rect, u32)>,
    /// Enfants directs du dossier regardé et leur rectangle (sélection, clic).
    top: Vec<(Rect, u32)>,
    /// Couleur par extension, et ordre des extensions par taille (page Types).
    colors_for: Option<u64>,
    ext_colors: Vec<Color32>,
    ext_order: Vec<u16>,
}

impl TreemapCache {
    pub(crate) fn heap_bytes(&self) -> usize {
        self.texture_bytes
            + (self.tiles.capacity() + self.top.capacity()) * size_of::<(Rect, u32)>()
            + self.ext_colors.capacity() * 4
            + self.ext_order.capacity() * 2
    }
}

fn scan_for<'a>(state: &'a fx_app::AppState, root: &Path) -> Option<&'a DiskScan> {
    fx_app::update::disk::scan_of(state, root)
}

/// Libère la carte (image de quelques Mo) des volets qui ont quitté l'analyse.
pub(crate) fn prune(f: &mut Fx<'_>) {
    if f.vs.treemaps.is_empty() {
        return;
    }
    let state = f.state;
    f.vs.treemaps.retain(|id, _| {
        state
            .find_pane(*id)
            .is_some_and(|p| matches!(p.location(), Location::DiskUsage { .. }))
    });
}

fn size_text(bytes: u64) -> String {
    let mut s = String::new();
    let _ = format::size_human(&mut s, bytes);
    s
}

/// Nombre à une décimale, virgule française (« 12,5 »).
fn dec1(x: f64) -> String {
    format!("{x:.1}").replace('.', ",")
}

fn count_text(n: u64) -> String {
    let mut s = String::new();
    let _ = format::thousands(&mut s, n);
    s
}

pub fn draw(ui: &mut Ui, f: &mut Fx<'_>, pane: &Pane) {
    let Location::DiskUsage { root, dir } = pane.location() else {
        return;
    };
    // L'état vit plus longtemps que `f` : l'analyse reste lisible pendant qu'on émet.
    let state: &fx_app::AppState = f.state;
    let Some(scan) = scan_for(state, root) else {
        ui.centered_and_justified(|ui| ui.spinner());
        return;
    };
    header(ui, f, pane, scan, dir);
    match (&scan.status, &scan.tree) {
        (DiskStatus::Scanning, _) => scanning(ui, f, scan),
        (DiskStatus::Cancelled, None) => {
            super::pane::placeholder(
                ui,
                f,
                glyph::PIE,
                "Analyse interrompue",
                "Relancez-la pour voir ce qui occupe la place.",
            );
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                if ui.button("Relancer l'analyse").clicked() {
                    f.send(Msg::DiskRescan(scan.root.clone()));
                }
            });
        }
        (_, Some(tree)) => match pane.disk_tab {
            DiskTab::Content => content(ui, f, pane, scan, tree, dir),
            DiskTab::Types => types(ui, f, pane.id, scan, tree),
            DiskTab::Cleanup => cleanup(ui, f, pane.id, scan, root),
        },
        (_, None) => {
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
        }
    }
}

/// En dessous, l'en-tête passe sur deux lignes et les libellés raccourcissent.
const NARROW: f32 = 760.0;

fn header(ui: &mut Ui, f: &mut Fx<'_>, pane: &Pane, scan: &DiskScan, dir: &Path) {
    let pal = f.pal;
    let narrow = ui.available_width() < NARROW;
    let summary = match (&scan.status, &scan.tree) {
        (DiskStatus::Scanning, _) => "en cours…".to_owned(),
        (DiskStatus::Done { elapsed }, Some(t)) => {
            let mut s = format!(
                "{} · {} fichiers · analysé en {} s",
                size_text(t.size(DiskTree::root())),
                count_text(u64::from(t.files(DiskTree::root()))),
                dec1(elapsed.as_secs_f64())
            );
            if t.denied() > 0 {
                let _ = write!(
                    s,
                    " · {} dossiers illisibles",
                    count_text(u64::from(t.denied()))
                );
            }
            if scan.refreshing > 0 {
                s.push_str(" · mise à jour…");
            }
            s
        }
        _ => String::new(),
    };
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        // Boutons d'abord (à droite), puis titre et résumé dans la place qui reste,
        // tronqués plutôt que de passer sous les boutons.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(10.0);
            let close = if narrow { "Fermer" } else { "Fermer l'analyse" };
            if ui.button(close).clicked() {
                f.send(Msg::CloseDiskUsage(pane.id));
            }
            if scan.is_scanning() {
                if ui.button("Arrêter").clicked() {
                    f.send(Msg::DiskCancel(scan.root.clone()));
                }
            } else if ui
                .button("Relancer")
                .on_hover_text("Relire tout le contenu analysé")
                .clicked()
            {
                f.send(Msg::DiskRescan(scan.root.clone()));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(
                    RichText::new(glyph::PIE)
                        .font(icon_font(14.0))
                        .color(pal.accent),
                );
                ui.add_space(2.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("Analyse de {}", scan.root.display()))
                            .font(semibold(14.0))
                            .color(pal.text),
                    )
                    .truncate(),
                );
                if !narrow {
                    ui.add(egui::Label::new(RichText::new(&summary).color(pal.text2)).truncate());
                }
            });
        });
    });
    if narrow && !summary.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(36.0);
            ui.add(egui::Label::new(RichText::new(&summary).color(pal.text2)).truncate());
        });
    }
    if let Some(tree) = &scan.tree {
        ui.add_space(4.0);
        volume_bar(ui, f, scan, tree);
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(14.0);
            let cleanup: u64 = scan.suggestions.iter().map(|s| s.bytes).sum();
            let tabs = [
                (DiskTab::Content, "Contenu".to_owned()),
                (
                    DiskTab::Types,
                    if narrow { "Types" } else { "Types de fichiers" }.to_owned(),
                ),
                (
                    DiskTab::Cleanup,
                    if cleanup > 0 {
                        format!("Récupérable · {}", size_text(cleanup))
                    } else {
                        "Récupérable".to_owned()
                    },
                ),
            ];
            for (tab, label) in tabs {
                let on = pane.disk_tab == tab;
                let text = RichText::new(label).color(if on { pal.accent } else { pal.text2 });
                let r = ui.add(egui::Button::new(text).selected(on).frame(on));
                if r.clicked() && !on {
                    f.send(Msg::DiskTab { pane: pane.id, tab });
                }
            }
            if pane.disk_tab == DiskTab::Content
                && !narrow
                && let Some(n) = tree.find(dir)
            {
                ui.add_space(12.0);
                let total = tree.size(DiskTree::root()).max(1);
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "Ce dossier : {} ({} % de l'analyse)",
                            size_text(tree.size(n)),
                            dec1(tree.size(n) as f64 * 100.0 / total as f64)
                        ))
                        .color(pal.text3),
                    )
                    .truncate(),
                );
            }
        });
    }
    ui.add_space(6.0);
}

/// Lecteur analysé : place analysée, place non lue (système, illisible), place libre.
fn volume_bar(ui: &mut Ui, f: &Fx<'_>, scan: &DiskScan, tree: &DiskTree) {
    let pal = f.pal;
    let Some(drive) = f.state.drives.iter().find(|d| {
        scan.root.parent().is_none()
            && d.root
                .as_os_str()
                .eq_ignore_ascii_case(scan.root.as_os_str())
    }) else {
        return;
    };
    if drive.total == 0 {
        return;
    }
    let used = drive.total.saturating_sub(drive.free);
    let seen = tree.size(DiskTree::root()).min(used);
    let other = used - seen;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let bar = Rect::from_min_size(
        pos2(rect.left() + 14.0, rect.top() + 2.0),
        vec2((rect.width() - 28.0).min(520.0), 8.0),
    );
    let total = drive.total as f32;
    let x = |b: u64| bar.left() + bar.width() * (b as f32 / total);
    let p = ui.painter();
    p.rect_filled(bar, 4.0, pal.hover);
    p.rect_filled(
        Rect::from_x_y_ranges(bar.left()..=x(seen), bar.y_range()),
        4.0,
        pal.accent,
    );
    p.rect_filled(
        Rect::from_x_y_ranges(x(seen)..=x(used), bar.y_range()),
        0.0,
        pal.text3.gamma_multiply(0.7),
    );
    let legend = if rect.width() < NARROW {
        format!(
            "{} analysés · {} libres sur {}",
            size_text(seen),
            size_text(drive.free),
            size_text(drive.total)
        )
    } else {
        format!(
            "{} analysés · {} non lus (système, accès refusé) · {} libres sur {}",
            size_text(seen),
            size_text(other),
            size_text(drive.free),
            size_text(drive.total)
        )
    };
    p.text(
        pos2(bar.left(), bar.bottom() + 9.0),
        Align2::LEFT_CENTER,
        legend,
        FontId::proportional(11.5),
        pal.text3,
    );
}

fn scanning(ui: &mut Ui, f: &mut Fx<'_>, scan: &DiskScan) {
    let pal = f.pal;
    let p = &scan.progress;
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.spinner();
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("{} lus", size_text(p.bytes)))
                .font(semibold(22.0))
                .color(pal.text),
        );
        ui.label(
            RichText::new(format!(
                "{} fichiers · {} dossiers · {:.0} s",
                count_text(p.files),
                count_text(p.dirs),
                scan.started.elapsed().as_secs_f32()
            ))
            .color(pal.text2),
        );
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(
                RichText::new(p.current.display().to_string())
                    .size(11.5)
                    .color(pal.text3),
            )
            .truncate(),
        );
    });
    ui.add_space(16.0);
    // Premier niveau au fil de l'analyse.
    let max = p.top.first().map_or(1, |t| t.bytes.max(1));
    let w = (ui.available_width() - 28.0).min(640.0);
    for t in p.top.iter().take(14) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        let left = rect.center().x - w / 2.0;
        let name = if t.name.is_empty() {
            "Fichiers".to_owned()
        } else {
            t.name.clone()
        };
        let painter = ui.painter();
        painter.text(
            pos2(left, rect.center().y),
            Align2::LEFT_CENTER,
            if t.dir { glyph::FOLDER } else { glyph::FILE },
            icon_font(13.0),
            if t.dir { pal.folder } else { pal.text3 },
        );
        painter
            .with_clip_rect(Rect::from_x_y_ranges(
                left..=(left + w * 0.38),
                rect.y_range(),
            ))
            .text(
                pos2(left + 22.0, rect.center().y),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(12.5),
                pal.text,
            );
        let bar = Rect::from_min_size(
            pos2(left + w * 0.4, rect.center().y - 4.0),
            vec2(w * 0.45, 8.0),
        );
        widgets::gradient_bar(ui, bar, t.bytes as f32 / max as f32, pal.accent, pal.hover);
        ui.painter().text(
            pos2(left + w, rect.center().y),
            Align2::RIGHT_CENTER,
            size_text(t.bytes),
            FontId::proportional(12.0),
            pal.text2,
        );
    }
}

/// Couleurs par extension (les plus gourmandes d'abord) : une fois par analyse.
fn ensure_colors(cache: &mut TreemapCache, scan: &DiskScan, tree: &DiskTree) {
    if cache.colors_for == Some(scan.id) {
        return;
    }
    let exts = tree.exts();
    let mut order: Vec<u16> = (1..exts.len() as u16).collect();
    order.sort_by(|&a, &b| exts[b as usize].bytes.cmp(&exts[a as usize].bytes));
    let mut colors = vec![Color32::from_gray(128); exts.len()];
    for (i, &e) in order.iter().take(COLORED_EXTS).enumerate() {
        let [r, g, b] = TILE_COLORS[i];
        colors[e as usize] = Color32::from_rgb(r, g, b);
    }
    // Sans extension : en tête du classement s'il le mérite (indice 0).
    let mut with_none = order;
    let pos = with_none
        .iter()
        .position(|&e| exts[e as usize].bytes < exts[0].bytes)
        .unwrap_or(with_none.len());
    if exts[0].files > 0 {
        with_none.insert(pos, 0);
    }
    cache.ext_colors = colors;
    cache.ext_order = with_none;
    cache.colors_for = Some(scan.id);
}

fn tile_color(cache: &TreemapCache, tree: &DiskTree, node: u32, dark: bool) -> Color32 {
    match tree.kind(node) {
        NodeKind::File => cache
            .ext_colors
            .get(tree.ext(node) as usize)
            .copied()
            .unwrap_or(Color32::GRAY),
        NodeKind::SmallFiles => {
            if dark {
                Color32::from_gray(92)
            } else {
                Color32::from_gray(170)
            }
        }
        NodeKind::Dir => {
            if dark {
                Color32::from_rgb(78, 86, 102)
            } else {
                Color32::from_rgb(150, 160, 178)
            }
        }
    }
}

/// Disposition « squarified » (Bruls, Huizing, van Wijk) des enfants de `node` dans `rect`,
/// récursive tant que les tuiles restent assez grandes.
fn layout(
    tree: &DiskTree,
    node: u32,
    rect: Rect,
    tiles: &mut Vec<(Rect, u32)>,
    top: &mut Vec<(Rect, u32)>,
) {
    let mut stack = vec![(node, rect, true)];
    let mut row: Vec<(u32, f32)> = Vec::new();
    while let Some((n, r, first)) = stack.pop() {
        let kids: Vec<(u32, f32)> = {
            let total: u64 = tree.children(n).map(|c| tree.size(c)).sum();
            if total == 0 || r.area() < MIN_TILE || tree.kind(n) != NodeKind::Dir {
                if !first || tree.kind(n) != NodeKind::Dir {
                    tiles.push((r, n));
                }
                continue;
            }
            let scale = r.area() / total as f32;
            tree.children(n)
                .filter(|&c| tree.size(c) > 0)
                .map(|c| (c, tree.size(c) as f32 * scale))
                .collect()
        };
        let mut free = r;
        let mut i = 0;
        while i < kids.len() {
            let short = free.width().min(free.height()).max(1.0);
            row.clear();
            row.push(kids[i]);
            let mut sum = kids[i].1;
            let mut worst = worst_ratio(kids[i].1, kids[i].1, sum, short);
            let mut j = i + 1;
            while j < kids.len() {
                let s2 = sum + kids[j].1;
                let w2 = worst_ratio(kids[i].1, kids[j].1, s2, short);
                if w2 > worst {
                    break;
                }
                row.push(kids[j]);
                sum = s2;
                worst = w2;
                j += 1;
            }
            let thick = (sum / short).min(free.width().max(free.height()));
            let horizontal = free.width() >= free.height();
            let mut at = if horizontal { free.top() } else { free.left() };
            for &(c, a) in &row {
                let len = a / thick;
                let cr = if horizontal {
                    Rect::from_min_size(pos2(free.left(), at), vec2(thick, len))
                } else {
                    Rect::from_min_size(pos2(at, free.top()), vec2(len, thick))
                };
                at += len;
                if first {
                    top.push((cr, c));
                }
                // Liseré d'un pixel entre les dossiers assez grands : la hiérarchie se lit.
                let inner =
                    if tree.kind(c) == NodeKind::Dir && cr.width() > 8.0 && cr.height() > 8.0 {
                        cr.shrink(1.0)
                    } else {
                        cr
                    };
                stack.push((c, inner, false));
            }
            if horizontal {
                free.min.x += thick;
            } else {
                free.min.y += thick;
            }
            i = j;
        }
    }
}

fn worst_ratio(max: f32, min: f32, sum: f32, short: f32) -> f32 {
    let s2 = sum * sum;
    let w2 = short * short;
    (w2 * max / s2).max(s2 / (w2 * min.max(f32::EPSILON)))
}

/// Image de la carte : chaque tuile en relief « coussin » (plus claire en haut à gauche,
/// plus sombre en bas à droite), séparée de ses voisines par une ligne de fond.
fn build_image(
    cache: &TreemapCache,
    tree: &DiskTree,
    dark: bool,
    [w, h]: [u32; 2],
    ppp: f32,
) -> egui::ColorImage {
    // Une colonne et une ligne de plus que la carte affichée : le moteur CPU ne recopie
    // l'image telle quelle (sans échantillonnage) que si elle déborde de la zone dessinée.
    let (vw, vh) = (w as usize, h as usize);
    let (w, h) = (vw + 1, vh + 1);
    let bg = if dark {
        Color32::from_gray(24)
    } else {
        Color32::from_gray(225)
    };
    let mut px = vec![bg; w * h];
    for &(r, n) in &cache.tiles {
        let x0 = ((r.min.x * ppp).round() as usize).min(vw);
        let y0 = ((r.min.y * ppp).round() as usize).min(vh);
        let mut x1 = ((r.max.x * ppp).round() as usize).min(vw);
        let mut y1 = ((r.max.y * ppp).round() as usize).min(vh);
        if x1 >= x0 + 3 {
            x1 -= 1;
        }
        if y1 >= y0 + 3 {
            y1 -= 1;
        }
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let c = tile_color(cache, tree, n, dark);
        let light = lerp(c, Color32::WHITE, 0.3);
        let shade = lerp(c, Color32::BLACK, 0.35);
        let (fw, fh) = ((x1 - x0) as f32, (y1 - y0) as f32);
        for y in y0..y1 {
            let fy = (y - y0) as f32 / fh;
            let row = &mut px[y * w + x0..y * w + x1];
            for (i, p) in row.iter_mut().enumerate() {
                let t = (i as f32 / fw + fy) * 0.5;
                *p = lerp(light, shade, t);
            }
        }
    }
    egui::ColorImage::new([w, h], px)
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Part de la hauteur donnée à la carte par défaut (réglable avec le séparateur).
const DEFAULT_SPLIT: f32 = 0.45;

fn content(ui: &mut Ui, f: &mut Fx<'_>, pane: &Pane, scan: &DiskScan, tree: &DiskTree, dir: &Path) {
    let Some(node) = tree.find(dir) else {
        super::pane::placeholder(
            ui,
            f,
            glyph::FOLDER,
            "Dossier absent de l'analyse",
            "Il a été supprimé ou déplacé depuis. Relancez l'analyse pour le retrouver.",
        );
        return;
    };
    let avail = ui.available_rect_before_wrap();
    // La carte suit la hauteur du volet ; la liste garde au moins l'en-tête et trois
    // lignes.
    let split = if f.vs.disk_split > 0.0 {
        f.vs.disk_split
    } else {
        DEFAULT_SPLIT
    };
    let max_h = (avail.height() - 150.0).max(48.0);
    let h = (avail.height() * split).clamp(48.0_f32.min(max_h), max_h);
    // Carte calée sur les pixels physiques : son image est recopiée sans rééchantillonnage.
    let ppp = ui.ctx().pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let map = Rect::from_min_max(
        pos2(snap(avail.left() + 12.0), snap(avail.top())),
        pos2(snap(avail.right() - 12.0), snap(avail.top() + h)),
    );
    ui.allocate_rect(
        Rect::from_min_size(avail.min, vec2(avail.width(), h + 10.0)),
        Sense::hover(),
    );
    treemap(ui, f, pane, scan, tree, node, map);
    // Séparateur : glisser pour partager la hauteur entre carte et liste ; double-clic :
    // retour au partage par défaut.
    let grip = Rect::from_min_max(
        pos2(map.left(), map.bottom() + 1.0),
        pos2(map.right(), map.bottom() + 9.0),
    );
    let g = ui.interact(
        grip,
        egui::Id::unique(("disk-split", pane.id.0)),
        Sense::click_and_drag(),
    );
    if g.hovered() || g.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        ui.painter().rect_filled(
            Rect::from_center_size(grip.center(), vec2(48.0, 3.0)),
            1.5,
            f.pal.text3,
        );
    }
    if g.dragged()
        && let Some(p) = ui.ctx().pointer_interact_pos()
        && avail.height() > 0.0
    {
        f.vs.disk_split = ((p.y - avail.top()) / avail.height()).clamp(0.12, 0.85);
    }
    if g.double_clicked() {
        f.vs.disk_split = 0.0;
    }
    if pane.listing.is_some() {
        list::draw(ui, f, pane, false);
    } else {
        ui.centered_and_justified(|ui| ui.spinner());
    }
}

fn treemap(
    ui: &mut Ui,
    f: &mut Fx<'_>,
    pane: &Pane,
    scan: &DiskScan,
    tree: &DiskTree,
    node: u32,
    rect: Rect,
) {
    let pal = f.pal;
    let ppp = ui.ctx().pixels_per_point();
    let size_px = [
        (rect.width() * ppp).round().max(1.0) as u32,
        (rect.height() * ppp).round().max(1.0) as u32,
    ];
    let key = (scan.id, tree.revision(), node, size_px, pal.dark);
    // Une carte par volet ; on n'en garde que pour quelques volets.
    if f.vs.treemaps.len() > 4 && !f.vs.treemaps.contains_key(&pane.id) {
        f.vs.treemaps.clear();
    }
    let cache = f.vs.treemaps.entry(pane.id).or_default();
    ensure_colors(cache, scan, tree);
    // Seule la taille a changé (fenêtre redimensionnée, séparateur) : l'image actuelle est
    // étirée et le calcul attend que la taille ne bouge plus depuis 120 ms, plutôt que de
    // refaire disposition et image à chaque image du redimensionnement.
    let only_size = cache.texture.is_some()
        && cache
            .key
            .is_some_and(|k| (k.0, k.1, k.2, k.4) == (key.0, key.1, key.2, key.4));
    let mut rebuild = cache.key != Some(key);
    if rebuild && only_size {
        const SETTLE: std::time::Duration = std::time::Duration::from_millis(120);
        match cache.pending {
            Some((s, t)) if s == size_px && t.elapsed() >= SETTLE => {}
            Some((s, t)) if s == size_px => {
                rebuild = false;
                ui.ctx()
                    .request_repaint_after(SETTLE.saturating_sub(t.elapsed()));
            }
            _ => {
                rebuild = false;
                cache.pending = Some((size_px, std::time::Instant::now()));
                ui.ctx().request_repaint_after(SETTLE);
            }
        }
    }
    if rebuild {
        cache.key = Some(key);
        cache.pending = None;
        cache.built = rect.size();
        cache.built_px = size_px;
        cache.tiles.clear();
        cache.top.clear();
        let local = Rect::from_min_size(egui::Pos2::ZERO, rect.size());
        let (mut tiles, mut top) = (
            std::mem::take(&mut cache.tiles),
            std::mem::take(&mut cache.top),
        );
        layout(tree, node, local, &mut tiles, &mut top);
        cache.tiles = tiles;
        cache.top = top;
        let image = build_image(cache, tree, pal.dark, size_px, ppp);
        cache.texture_bytes = image.pixels.len() * 4;
        // Nouvelle texture plutôt que `set` : une texture de taille différente serait
        // étirée tant que le moteur ne l'a pas remplacée.
        cache.texture = Some(ui.ctx().load_texture(
            format!("treemap-{}", pane.id.0),
            image,
            egui::TextureOptions::NEAREST,
        ));
    }
    let painter = ui.painter_at(rect);
    if cache.tiles.is_empty() {
        // Le fond n'est peint que sans image : celle-ci le contient déjà (une seule passe
        // sur la surface de la carte avec le moteur CPU).
        painter.rect_filled(rect, 4.0, pal.hover);
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Dossier vide",
            FontId::proportional(13.0),
            pal.text3,
        );
        return;
    }
    // Tuiles calculées pour `built` : mises à l'échelle de la carte affichée.
    let scale = rect.size() / cache.built.max(vec2(1.0, 1.0));
    let place = |r: Rect| {
        Rect::from_min_max(
            rect.min + r.min.to_vec2() * scale,
            rect.min + r.max.to_vec2() * scale,
        )
    };
    if let Some(t) = &cache.texture {
        let [bw, bh] = cache.built_px;
        let uv = Rect::from_min_max(
            pos2(0.0, 0.0),
            pos2(bw as f32 / (bw + 1) as f32, bh as f32 / (bh + 1) as f32),
        );
        let mut mesh = egui::Mesh::with_texture(t.id());
        mesh.add_rect_with_uv(rect, uv, Color32::WHITE);
        // Triangle vide après le rectangle : le moteur CPU ne reconnaît un rectangle (et sa
        // recopie directe) que s'il reste des indices après lui ; une image seule (6 indices)
        // serait rasterisée en deux triangles échantillonnés, ~7 ms en 1080p.
        mesh.add_triangle(0, 0, 0);
        painter.add(egui::Shape::mesh(mesh));
    }

    let resp = ui.interact(
        rect,
        egui::Id::unique(("treemap", pane.id.0)),
        Sense::click(),
    );
    // Sélection de la liste (64 éléments au plus) : contour des tuiles correspondantes.
    if let Some(l) = &pane.listing
        && (1..=64).contains(&pane.selection.count())
    {
        for id in l
            .visible_ids()
            .iter()
            .copied()
            .filter(|&id| pane.selection.contains(id))
        {
            let name = l.view(id).name;
            let hit = cache.top.iter().find(|&&(_, c)| {
                tree.kind(c) != NodeKind::SmallFiles
                    && fx_core::disktree::same_name(tree.name(c), name)
            });
            if let Some(&(r, _)) = hit {
                let r = place(r);
                painter.rect_stroke(
                    r,
                    0.0,
                    Stroke::new(2.0, Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
                painter.rect_stroke(
                    r.shrink(2.0),
                    0.0,
                    Stroke::new(1.0, Color32::BLACK),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
    let Some(pos) = resp.hover_pos() else { return };
    let local = ((pos - rect.min) / scale).to_pos2();
    let Some(&(tr, tile)) = cache.tiles.iter().rev().find(|(r, _)| r.contains(local)) else {
        return;
    };
    painter.rect_stroke(
        place(tr),
        0.0,
        Stroke::new(1.5, Color32::WHITE),
        egui::StrokeKind::Inside,
    );
    // Enfant direct du dossier regardé qui contient la tuile.
    let mut top = tile;
    while let Some(p) = tree.parent(top) {
        if p == node {
            break;
        }
        top = p;
    }
    let total = tree.size(node).max(1);
    let label = match tree.kind(tile) {
        NodeKind::SmallFiles => {
            let mut s = String::new();
            let _ = write!(
                s,
                "{} petits fichiers (regroupés) dans {}",
                count_text(u64::from(tree.files(tile))),
                tree.parent(tile)
                    .map_or_else(String::new, |p| tree.path(p).display().to_string())
            );
            s
        }
        _ => tree.path(tile).display().to_string(),
    };
    let detail = format!(
        "{} · {} % de ce dossier{}",
        size_text(tree.size(tile)),
        dec1(tree.size(tile) as f64 * 100.0 / total as f64),
        if tree.flags(tile) & flags::DENIED != 0 {
            " · accès refusé"
        } else {
            ""
        }
    );
    resp.clone().on_hover_ui_at_pointer(|ui| {
        ui.label(RichText::new(label).color(pal.text));
        ui.label(RichText::new(detail).color(pal.text2));
        ui.label(
            RichText::new("Clic : sélectionner · double-clic : ouvrir")
                .size(11.0)
                .color(pal.text3),
        );
    });
    let Location::DiskUsage { root, .. } = pane.location() else {
        return;
    };
    if resp.double_clicked() {
        let target = if tree.kind(tile) == NodeKind::Dir {
            Some(tile)
        } else {
            tree.parent(tile)
        };
        if let Some(t) = target.filter(|&t| t != node) {
            f.send(Msg::NavigatePane(
                pane.id,
                Location::DiskUsage {
                    root: root.clone(),
                    dir: tree.path(t),
                },
            ));
        }
    } else if resp.clicked() {
        select_child(
            f,
            pane,
            tree.name(top),
            tree.kind(top),
            ui.input(|i| i.modifiers.command),
        );
    }
}

fn l_find(l: &fx_core::DirSnapshot, name: &[u8]) -> Option<u32> {
    // `find` construit son index au premier appel : on évite de muter l'instantané à
    // l'affichage, en comparant sur les entrées visibles (au plus quelques dizaines de
    // tuiles de premier niveau par image).
    l.visible_ids()
        .iter()
        .copied()
        .find(|&id| fx_core::disktree::same_name(l.view(id).name, name))
}

fn select_child(f: &mut Fx<'_>, pane: &Pane, name: &[u8], kind: NodeKind, extend: bool) {
    if kind == NodeKind::SmallFiles {
        return;
    }
    let Some(l) = &pane.listing else { return };
    let Some(id) = l_find(l, name) else { return };
    let Some(pos) = l.position_of(id) else { return };
    f.send(Msg::FocusPane(pane.id));
    f.send(Msg::SelectPositions {
        pane: pane.id,
        positions: vec![pos],
        extend,
    });
    f.vs.scroll.entry(pane.id).or_default().ensure = Some(pos);
}

fn types(ui: &mut Ui, f: &mut Fx<'_>, pane: PaneId, scan: &DiskScan, tree: &DiskTree) {
    let pal = f.pal;
    let cache = f.vs.treemaps.entry(pane).or_default();
    ensure_colors(cache, scan, tree);
    let (order, colors) = (&cache.ext_order, &cache.ext_colors);
    let exts = tree.exts();
    let total: u64 = exts.iter().map(|e| e.bytes).sum::<u64>().max(1);
    let max = order.first().map_or(1, |&e| exts[e as usize].bytes.max(1));
    ui.add_space(4.0);
    let row_h = 28.0;
    egui::ScrollArea::vertical()
        .id_salt(("disk-types", pane.0))
        .auto_shrink([false, false])
        .show_rows(ui, row_h, order.len().min(1000), |ui, range| {
            for &e in &order[range] {
                let x = &exts[e as usize];
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                if resp.hovered() {
                    ui.painter()
                        .rect_filled(rect.shrink2(vec2(8.0, 1.0)), 6.0, pal.hover);
                }
                let y = rect.center().y;
                let left = rect.left() + 18.0;
                let right = rect.right() - 18.0;
                let w = right - left;
                // Colonnes facultatives selon la largeur : description, puis nombre de
                // fichiers ; la barre prend la place qui reste.
                let show_desc = w >= 620.0;
                let show_count = w >= 760.0;
                let size_w = 130.0;
                let count_w = if show_count { 110.0 } else { 0.0 };
                let bar_x0 = left
                    + if show_desc {
                        (w * 0.40).max(260.0)
                    } else {
                        120.0
                    };
                let bar_x1 = (right - size_w - count_w - 16.0).max(bar_x0 + 8.0);
                let swatch = Rect::from_center_size(pos2(left + 6.0, y), vec2(12.0, 12.0));
                ui.painter().rect_filled(
                    swatch,
                    3.0,
                    colors.get(e as usize).copied().unwrap_or(Color32::GRAY),
                );
                let ext = wtf8::display(&x.name);
                let name = if ext.is_empty() {
                    "Sans extension".to_owned()
                } else {
                    format!(".{ext}")
                };
                let painter = ui.painter();
                painter
                    .with_clip_rect(Rect::from_x_y_ranges(
                        left..=(left + if show_desc { 104.0 } else { 112.0 }),
                        rect.y_range(),
                    ))
                    .text(
                        pos2(left + 22.0, y),
                        Align2::LEFT_CENTER,
                        name,
                        semibold(12.5),
                        pal.text,
                    );
                if show_desc {
                    painter
                        .with_clip_rect(Rect::from_x_y_ranges(
                            (left + 110.0)..=(bar_x0 - 12.0),
                            rect.y_range(),
                        ))
                        .text(
                            pos2(left + 110.0, y),
                            Align2::LEFT_CENTER,
                            super::type_name(false, &x.name),
                            FontId::proportional(12.0),
                            pal.text2,
                        );
                }
                let bar = Rect::from_x_y_ranges(bar_x0..=bar_x1, (y - 4.0)..=(y + 4.0));
                widgets::gradient_bar(ui, bar, x.bytes as f32 / max as f32, pal.accent, pal.hover);
                let painter = ui.painter();
                if show_count {
                    painter.text(
                        pos2(right - size_w - 8.0, y),
                        Align2::RIGHT_CENTER,
                        format!("{} fichiers", count_text(x.files)),
                        FontId::proportional(12.0),
                        pal.text3,
                    );
                }
                painter.text(
                    pos2(right, y),
                    Align2::RIGHT_CENTER,
                    format!(
                        "{}  ·  {} %",
                        size_text(x.bytes),
                        dec1(x.bytes as f64 * 100.0 / total as f64)
                    ),
                    FontId::proportional(12.0),
                    pal.text2,
                );
            }
        });
}

fn cleanup(ui: &mut Ui, f: &mut Fx<'_>, pane: PaneId, scan: &DiskScan, root: &Path) {
    let pal = f.pal;
    if scan.suggestions.is_empty() {
        super::pane::placeholder(
            ui,
            f,
            glyph::SUCCESS,
            "Rien à signaler",
            "Aucun cache, fichier temporaire ou gros fichier ancien de plus de 10 Mo n'a été trouvé.",
        );
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt(("disk-cleanup", root.as_os_str().len()))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            for (i, s) in scan.suggestions.iter().enumerate() {
                // Carte empilée verticalement : la page défile de haut en bas.
                {
                    let w = ui.available_width() - 28.0;
                    egui::Frame::new()
                        .fill(pal.card)
                        .stroke(Stroke::new(1.0, pal.stroke))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .outer_margin(egui::Margin {
                            left: 14,
                            right: 14,
                            top: 0,
                            bottom: 0,
                        })
                        .show(ui, |ui| {
                            ui.set_width(w - 28.0);
                            ui.horizontal(|ui| {
                                let g = match s.mode {
                                    CleanMode::RecycleBin => glyph::RECYCLE,
                                    _ if s.permanent => glyph::BROOM,
                                    _ => glyph::WARNING,
                                };
                                ui.label(
                                    RichText::new(g)
                                        .font(icon_font(15.0))
                                        .color(if s.permanent { pal.accent } else { pal.warning }),
                                );
                                ui.label(
                                    RichText::new(s.title).font(semibold(14.0)).color(pal.text),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(size_text(s.bytes))
                                            .font(semibold(15.0))
                                            .color(pal.text),
                                    );
                                });
                            });
                            ui.label(RichText::new(s.detail).color(pal.text2));
                            ui.add_space(2.0);
                            for p in s.paths.iter().take(3) {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(p.display().to_string())
                                            .size(11.5)
                                            .color(pal.text3),
                                    )
                                    .truncate(),
                                );
                            }
                            if s.paths.len() > 3 {
                                ui.label(
                                    RichText::new(format!("… et {} autres", s.paths.len() - 3))
                                        .size(11.5)
                                        .color(pal.text3),
                                );
                            }
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                let action = match s.mode {
                                    CleanMode::RecycleBin => "Vider la corbeille",
                                    _ if s.permanent => "Nettoyer",
                                    _ => "Mettre à la corbeille…",
                                };
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(action).color(Color32::WHITE),
                                        )
                                        .fill(if s.permanent { pal.accent } else { pal.danger }),
                                    )
                                    .clicked()
                                {
                                    f.send(Msg::DiskClean {
                                        root: root.to_path_buf(),
                                        index: i,
                                    });
                                }
                                if let Some(first) = s.paths.first()
                                    && s.mode != CleanMode::RecycleBin
                                    && ui.button("Afficher").clicked()
                                {
                                    let dir = if s.mode == CleanMode::Contents {
                                        first.clone()
                                    } else {
                                        first
                                            .parent()
                                            .map_or_else(|| first.clone(), Path::to_path_buf)
                                    };
                                    f.send(Msg::NavigatePane(
                                        pane,
                                        Location::DiskUsage {
                                            root: root.to_path_buf(),
                                            dir,
                                        },
                                    ));
                                    f.send(Msg::DiskTab {
                                        pane,
                                        tab: DiskTab::Content,
                                    });
                                }
                            });
                        });
                }
                ui.add_space(8.0);
            }
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(
                    RichText::new(
                        "Estimations de l'analyse. Rien n'est supprimé sans confirmation.",
                    )
                    .size(11.5)
                    .color(pal.text3),
                );
            });
            ui.add_space(12.0);
        });
}
