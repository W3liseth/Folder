//! Barre latérale : accueil, accès rapide (épingles, corbeille), lecteurs, réseau, WSL,
//! étiquettes.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Frame, Id, Margin, Rect, ScrollArea, Sense, Ui, pos2, vec2};
use fx_app::{Location, Msg};
use fx_commands::CommandId;
use fx_win::sys::{DriveKind, KnownFolder};

use super::Fx;
use crate::theme::{glyph, icon_font, semibold};
use crate::widgets;

const ROW: f32 = 32.0;

/// Icône d'une entrée : image shell (si chargée) sinon glyphe.
enum Icon<'a> {
    Glyph(&'a str, egui::Color32),
    Shell(PathBuf, &'a str),
}

pub fn draw(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    let width = f.state.settings.sidebar_width;
    let resp = egui::Panel::left("sidebar")
        .resizable(true)
        .default_size(width)
        .size_range(160.0..=420.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(pal.side).inner_margin(Margin {
            left: 6,
            right: 6,
            top: 6,
            bottom: 6,
        }))
        .show(ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| content(ui, f));
        });
    let w = resp.response.rect.width();
    if (w - width).abs() > 1.0 && !ui.ctx().input(|i| i.pointer.any_down()) {
        f.send(super::settings_with(f.state, |s| s.sidebar_width = w));
    }
}

fn content(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    ui.spacing_mut().item_spacing.y = 1.0;
    // Surlignage de l'élément actif : réservé sous les lignes, dessiné à la fin quand sa
    // position est connue, et animé d'un élément à l'autre (il glisse).
    let highlight = ui.painter().add(egui::Shape::Noop);
    f.vs.sidebar_active = None;
    let current = f.state.pane().location().clone();

    if row(
        ui,
        f,
        Icon::Glyph(glyph::HOME, pal.text2),
        "Accueil",
        current == Location::Home,
        None,
    )
    .clicked()
    {
        f.send(Msg::Navigate(Location::Home));
    }

    widgets::section(ui, &pal, "Accès rapide");
    let pins = f.state.settings.pinned.clone();
    for (i, p) in pins.iter().enumerate() {
        let name = display_name(p);
        let known = f
            .state
            .known
            .iter()
            .find(|(_, kp)| kp == p)
            .map(|(k, _)| *k);
        let icon = match known {
            // Icône définie par Windows (Bureau, Téléchargements…), glyphe en attendant.
            Some(k) => Icon::Shell(p.clone(), known_glyph(k)),
            None => Icon::Shell(p.clone(), glyph::FOLDER),
        };
        let active = matches!(&current, Location::Dir(d) if d == p);
        let r = row(ui, f, icon, &name, active, Some(p));
        if r.clicked() {
            f.send(Msg::Navigate(Location::Dir(p.clone())));
        }
        if r.middle_clicked() {
            f.send(Msg::OpenInNewTab(Location::Dir(p.clone())));
        }
        r.context_menu(|ui| {
            widgets::menu_appear(ui);
            ui.set_min_width(220.0);
            if ui.button("Ouvrir dans un nouvel onglet").clicked() {
                f.send(Msg::OpenInNewTab(Location::Dir(p.clone())));
                ui.close();
            }
            if ui.add_enabled(i > 0, egui::Button::new("Monter")).clicked() {
                f.send(Msg::MovePin { from: i, to: i - 1 });
                ui.close();
            }
            if ui
                .add_enabled(i + 1 < pins.len(), egui::Button::new("Descendre"))
                .clicked()
            {
                f.send(Msg::MovePin { from: i, to: i + 1 });
                ui.close();
            }
            ui.separator();
            if ui.button("Désépingler").clicked() {
                f.send(Msg::Unpin(p.clone()));
                ui.close();
            }
            if ui.button("Copier le chemin").clicked() {
                ui.ctx().copy_text(p.display().to_string());
                ui.close();
            }
            if ui.button("Propriétés").clicked() {
                f.send(Msg::Properties(p.clone()));
                ui.close();
            }
        });
        r.on_hover_text(p.display().to_string());
    }
    if pins.is_empty() {
        ui.label(
            egui::RichText::new("Épinglez des dossiers depuis leur menu contextuel.")
                .small()
                .color(pal.text3),
        );
    }
    recycle_bin(ui, f, &current);

    widgets::section(ui, &pal, "Lecteurs");
    for d in &f.state.drives.clone() {
        let label = drive_label(d);
        let active = matches!(&current, Location::Dir(p) if p == &d.root);
        let g = match d.kind {
            DriveKind::Network => glyph::NETWORK,
            DriveKind::Removable => glyph::USB,
            DriveKind::Optical => glyph::DISC,
            _ => glyph::DRIVE,
        };
        let thick = if d.total > 0 {
            f.state.settings.drive_bar.clamp(1.0, 12.0)
        } else {
            0.0
        };
        let r = row_with_gauge(
            ui,
            f,
            Icon::Shell(d.root.clone(), g),
            &label,
            active,
            Some(&d.root),
            thick,
        );
        if d.total > 0 {
            let used = 1.0 - d.free as f32 / d.total as f32;
            let bar = Rect::from_min_size(
                pos2(r.rect.left() + 38.0, r.rect.top() + GAUGE_TOP),
                vec2(r.rect.width() - 50.0, thick),
            );
            let color = if used > 0.9 { pal.danger } else { pal.accent };
            widgets::gradient_bar(ui, bar, used, color, pal.stroke_strong);
        }
        if r.clicked() {
            f.send(Msg::Navigate(Location::Dir(d.root.clone())));
        }
        if r.middle_clicked() {
            f.send(Msg::OpenInNewTab(Location::Dir(d.root.clone())));
        }
        r.on_hover_text(format!("{} libres sur {}", human(d.free), human(d.total)));
    }

    network_and_wsl(ui, f, &current);

    if f.state.settings.show_tags && !f.state.tags.list.is_empty() {
        widgets::section(ui, &pal, "Étiquettes");
        for t in &f.state.tags.list.clone() {
            let active = current == Location::Tag(t.id);
            let r = row(ui, f, Icon::Glyph("", pal.text2), &t.name, active, None);
            widgets::tag_icon(
                ui,
                pos2(r.rect.left() + 18.0, r.rect.center().y),
                15.0,
                t.color,
            );
            let n = f.state.tags.count(t.id);
            if n > 0 {
                ui.painter().text(
                    pos2(r.rect.right() - 10.0, r.rect.center().y),
                    Align2::RIGHT_CENTER,
                    n.to_string(),
                    egui::FontId::proportional(11.5),
                    pal.text3,
                );
            }
            if r.clicked() {
                f.send(Msg::Navigate(Location::Tag(t.id)));
            }
        }
    }
    paint_highlight(ui, f, highlight);
}

/// Surlignage de l'élément actif, qui glisse vers sa nouvelle position (180 ms) et s'efface
/// quand aucun élément de la barre ne correspond à l'emplacement affiché.
fn paint_highlight(ui: &Ui, f: &mut Fx<'_>, slot: egui::layers::ShapeIdx) {
    let pal = f.pal;
    let ctx = ui.ctx().clone();
    let id = Id::unique("sidebar-highlight");
    let shown = f.vs.sidebar_active.is_some();
    let alpha = ctx.animate_bool_with_time(id.with("a"), shown, 0.15);
    let Some(target) = f.vs.sidebar_active.or(f.vs.sidebar_last) else {
        return;
    };
    f.vs.sidebar_last = Some(target);
    let y = ctx.animate_value_with_time(id.with("y"), target.top(), 0.18);
    // Les lignes des lecteurs sont plus hautes (jauge) : la hauteur glisse aussi.
    let h = ctx.animate_value_with_time(id.with("h"), target.height(), 0.18);
    if alpha <= 0.0 {
        return;
    }
    let rect = Rect::from_min_size(pos2(target.left(), y), vec2(target.width(), h));
    let pill = Rect::from_center_size(pos2(rect.left() + 2.0, rect.center().y), vec2(3.0, 16.0));
    // Fond, halo de la pastille, dégradé d'accent qui s'éteint vers la droite, pastille.
    let glow = egui::Shadow {
        offset: [0, 0],
        blur: 10,
        spread: 0,
        color: pal.accent.gamma_multiply(0.6 * alpha),
    }
    .as_shape(pill, egui::CornerRadius::same(2));
    let mut fade = egui::Mesh::default();
    let (a, b) = (
        pal.accent.gamma_multiply(0.26 * alpha),
        egui::Color32::TRANSPARENT,
    );
    let band = Rect::from_min_max(
        pos2(rect.left() + 6.0, rect.top()),
        pos2(rect.left() + rect.width() * 0.6, rect.bottom()),
    );
    fade.colored_vertex(band.left_top(), a);
    fade.colored_vertex(band.right_top(), b);
    fade.colored_vertex(band.left_bottom(), a);
    fade.colored_vertex(band.right_bottom(), b);
    fade.add_triangle(0, 1, 2);
    fade.add_triangle(1, 3, 2);
    ui.painter().set(
        slot,
        egui::Shape::Vec(vec![
            egui::Shape::rect_filled(rect, 6.0, pal.hover.gamma_multiply(1.4 * alpha)),
            egui::Shape::mesh(fade),
            glow.into(),
            egui::Shape::rect_filled(pill, 1.5, pal.accent.gamma_multiply(alpha)),
        ]),
    );
}

/// Corbeille, dans l'accès rapide comme dans l'Explorateur (icône de Windows, vide ou pleine).
fn recycle_bin(ui: &mut Ui, f: &mut Fx<'_>, current: &Location) {
    let icon = Icon::Shell(
        PathBuf::from(fx_app::history::RECYCLE_BIN_ID),
        glyph::RECYCLE,
    );
    let r = row(
        ui,
        f,
        icon,
        "Corbeille",
        *current == Location::RecycleBin,
        None,
    );
    // Dépôt sur la Corbeille : envoi à la corbeille, comme dans l'Explorateur.
    super::dnd::zone(f, r.rect, super::dnd::Target::RecycleBin);
    if r.clicked() {
        f.send(Msg::Navigate(Location::RecycleBin));
    }
    if r.middle_clicked() {
        f.send(Msg::OpenInNewTab(Location::RecycleBin));
    }
    r.context_menu(|ui| {
        widgets::menu_appear(ui);
        ui.set_min_width(220.0);
        if ui
            .button(widgets::menu_entry(glyph::OPEN, "Ouvrir", &f.pal))
            .clicked()
        {
            f.send(Msg::Navigate(Location::RecycleBin));
            ui.close();
        }
        if ui
            .button(widgets::menu_entry(
                glyph::ADD,
                "Ouvrir dans un nouvel onglet",
                &f.pal,
            ))
            .clicked()
        {
            f.send(Msg::OpenInNewTab(Location::RecycleBin));
            ui.close();
        }
        ui.separator();
        if ui
            .button(widgets::menu_entry(
                glyph::RECYCLE,
                "Vider la corbeille",
                &f.pal,
            ))
            .clicked()
        {
            f.cmd(CommandId::EmptyRecycleBin);
            ui.close();
        }
    });
}

/// Réseau et distributions WSL.
fn network_and_wsl(ui: &mut Ui, f: &mut Fx<'_>, current: &Location) {
    let pal = f.pal;
    widgets::section(ui, &pal, "Réseau");
    let icon = Icon::Shell(PathBuf::from(fx_app::history::NETWORK_ID), glyph::NETWORK);
    let active = matches!(current, Location::Network(_));
    let r = row(ui, f, icon, "Réseau", active, None);
    if r.clicked() {
        f.send(Msg::Navigate(Location::Network(None)));
    }
    if r.middle_clicked() {
        f.send(Msg::OpenInNewTab(Location::Network(None)));
    }

    if f.state.wsl.is_empty() {
        return;
    }
    widgets::section(ui, &pal, "WSL");
    for name in f.state.wsl.clone() {
        let path = PathBuf::from(format!(r"\\wsl.localhost\{name}"));
        let active = matches!(current, Location::Dir(d) if d.starts_with(&path));
        // Icône « Linux » de Windows (dossier shell de WSL) : n'éveille pas la machine
        // virtuelle, contrairement à l'icône du dossier de la distribution.
        let icon = Icon::Shell(PathBuf::from(LINUX_ID), glyph::LINUX);
        let r = row(ui, f, icon, &name, active, Some(&path));
        if r.clicked() {
            f.send(Msg::Navigate(Location::Dir(path.clone())));
        }
        if r.middle_clicked() {
            f.send(Msg::OpenInNewTab(Location::Dir(path.clone())));
        }
        r.on_hover_text(path.display().to_string());
    }
}

/// Dossier shell « Linux » (racine des distributions WSL dans l'Explorateur).
const LINUX_ID: &str = "::{B2B4A4D1-2754-4140-A2EB-9A76D9D7CDC6}";

/// Une ligne de la barre latérale ; `drop` : dossier cible d'un dépôt de fichiers.
fn row(
    ui: &mut Ui,
    f: &mut Fx<'_>,
    icon: Icon<'_>,
    label: &str,
    active: bool,
    drop: Option<&Path>,
) -> egui::Response {
    row_with_gauge(ui, f, icon, label, active, drop, 0.0)
}

/// Haut de la jauge d'un lecteur dans sa ligne (cf. [`row_with_gauge`]).
const GAUGE_TOP: f32 = 24.0;

/// Ligne de la barre latérale ; `gauge` > 0 : place pour une jauge de cette épaisseur sous le
/// libellé (lecteurs). La ligne grandit avec la jauge et libellé + jauge restent centrés,
/// pour que surlignage, survol et pastille couvrent l'ensemble.
fn row_with_gauge(
    ui: &mut Ui,
    f: &mut Fx<'_>,
    icon: Icon<'_>,
    label: &str,
    active: bool,
    drop: Option<&Path>,
    gauge: f32,
) -> egui::Response {
    let pal = f.pal;
    let (height, content_y) = if gauge > 0.0 {
        ((GAUGE_TOP + gauge + 5.0).max(ROW), GAUGE_TOP - 11.0)
    } else {
        (ROW, ROW / 2.0)
    };
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let content_y = rect.top() + content_y;
    if !ui.is_rect_visible(rect) {
        return r;
    }
    if let Some(d) = drop {
        super::dnd::zone(f, rect, super::dnd::Target::Dir(d.to_path_buf()));
    }
    if active {
        f.vs.sidebar_active = Some(rect);
    } else {
        widgets::hover_fill(ui, &r, rect, &pal, 4.0);
    }
    let icon_center = pos2(rect.left() + 18.0, content_y);
    match icon {
        Icon::Glyph(g, c) if !g.is_empty() => {
            ui.painter()
                .text(icon_center, Align2::CENTER_CENTER, g, icon_font(15.0), c);
        }
        Icon::Glyph(..) => {}
        Icon::Shell(path, fallback) => {
            let c = if fallback == glyph::FOLDER {
                pal.folder
            } else {
                pal.text2
            };
            super::items::shell_icon(ui, f, &path, icon_center, 16.0, fallback, c);
        }
    }
    f.vs.fallbacks.scan(label);
    let text_rect = Rect::from_min_max(
        pos2(rect.left() + 36.0, rect.top()),
        pos2(rect.right() - 4.0, rect.bottom()),
    );
    let font = if active {
        semibold(13.5)
    } else {
        egui::FontId::proportional(13.5)
    };
    ui.painter().with_clip_rect(text_rect).text(
        pos2(text_rect.left(), content_y - 1.0),
        Align2::LEFT_CENTER,
        label,
        font,
        pal.text,
    );
    r
}

fn known_glyph(k: KnownFolder) -> &'static str {
    match k {
        KnownFolder::Home => glyph::PERSON,
        KnownFolder::Desktop => glyph::DESKTOP,
        KnownFolder::Downloads => glyph::DOWNLOAD,
        KnownFolder::Documents => glyph::DOCUMENTS,
        KnownFolder::Pictures => glyph::PICTURES,
        KnownFolder::Music => glyph::MUSIC,
        KnownFolder::Videos => glyph::VIDEOS,
        KnownFolder::Recent => glyph::CLOCK,
    }
}

pub(crate) fn display_name(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

pub(crate) fn drive_label(d: &fx_win::sys::Drive) -> String {
    let base = if d.label.is_empty() {
        match d.kind {
            DriveKind::Fixed => "Disque local".to_owned(),
            DriveKind::Removable => "Lecteur amovible".to_owned(),
            DriveKind::Network => "Lecteur réseau".to_owned(),
            DriveKind::Optical => "Lecteur optique".to_owned(),
            _ => "Lecteur".to_owned(),
        }
    } else {
        d.label.clone()
    };
    format!("{base} ({}:)", d.letter)
}

/// Taille lisible (« 12,4 Go »).
pub(crate) fn human(bytes: u64) -> String {
    let mut s = String::new();
    let _ = fx_core::format::size_human(&mut s, bytes);
    s
}
