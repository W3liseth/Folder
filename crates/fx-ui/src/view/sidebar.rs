//! Barre latérale : accueil, accès rapide (épingles, corbeille), lecteurs, réseau, WSL,
//! étiquettes. Sections repliables ; épingles, lecteurs et distributions WSL se déroulent
//! en arborescence de dossiers (cf. `fx_app::tree`).

use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align2, Color32, Frame, Id, Margin, Pos2, Rect, ScrollArea, Sense, Stroke, Ui, pos2, vec2,
};
use fx_app::tree::{TreeNode, TreeSection};
use fx_app::{Location, Msg};
use fx_commands::CommandId;
use fx_core::entry::attrs;
use fx_store::SidebarSection;
use fx_win::sys::{DriveKind, KnownFolder};

use super::Fx;
use crate::theme::{glyph, icon_font, semibold};
use crate::widgets;

/// Hauteur d'une ligne de premier niveau.
const ROW: f32 = 32.0;
/// Hauteur d'un sous-dossier de l'arborescence (plus dense, comme l'Explorateur).
const SUB_ROW: f32 = 28.0;
/// Décalage par niveau de l'arborescence.
const INDENT: f32 = 14.0;
/// Positions dans une ligne de premier niveau : flèche, centre de l'icône, début du texte.
const CHEVRON_X: f32 = 9.0;
const ICON_X: f32 = 27.0;
const TEXT_X: f32 = 45.0;

/// Icône d'une entrée : image shell (si chargée) sinon glyphe.
enum Icon<'a> {
    Glyph(&'a str, egui::Color32),
    Shell(PathBuf, &'a str),
    /// Sous-dossier : icône de dossier commune, ou la sienne s'il est personnalisé
    /// (`desktop.ini`, que Windows signale par l'attribut lecture seule ou système).
    Folder(PathBuf, bool),
}

/// Une ligne de la barre latérale.
struct Row<'a> {
    id: Id,
    icon: Icon<'a>,
    label: &'a str,
    active: bool,
    /// Dossier cible d'un dépôt de fichiers.
    drop: Option<&'a Path>,
    /// Épaisseur de la jauge d'espace (lecteurs), 0 sans jauge.
    gauge: f32,
    depth: usize,
    /// Flèche pour dérouler : `Some(ouvert)`.
    expander: Option<bool>,
}

impl<'a> Row<'a> {
    fn new(id: Id, icon: Icon<'a>, label: &'a str, active: bool) -> Self {
        Self {
            id,
            icon,
            label,
            active,
            drop: None,
            gauge: 0.0,
            depth: 0,
            expander: None,
        }
    }
}

/// Réponse d'une ligne ; `toggled` : clic sur sa flèche (ne pas naviguer).
struct RowResponse {
    r: egui::Response,
    toggled: bool,
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
    // position est connue, et animé d'un élément à l'autre (il glisse). Haut du contenu
    // (il défile) : le surlignage est placé et animé par rapport à lui.
    let highlight = ui.painter().add(egui::Shape::Noop);
    let origin = ui.cursor().top();
    f.vs.sidebar_active = None;
    let current = f.state.pane().location().clone();

    let home = Row::new(
        Id::unique("side-home"),
        Icon::Glyph(glyph::HOME, pal.text2),
        "Accueil",
        current == Location::Home,
    );
    if draw_row(ui, f, home).r.clicked() {
        f.send(Msg::Navigate(Location::Home));
    }

    if section(ui, f, SidebarSection::QuickAccess, "Accès rapide") {
        quick_access(ui, f, &current);
    }
    if section(ui, f, SidebarSection::Drives, "Lecteurs") {
        drives(ui, f, &current);
    }
    network_and_wsl(ui, f, &current);

    if f.state.settings.show_tags
        && !f.state.tags.list.is_empty()
        && section(ui, f, SidebarSection::Tags, "Étiquettes")
    {
        for t in &f.state.tags.list.clone() {
            let active = current == Location::Tag(t.id);
            let row = Row::new(
                Id::unique(("side-tag", t.id)),
                Icon::Glyph("", pal.text2),
                &t.name,
                active,
            );
            let r = draw_row(ui, f, row).r;
            widgets::tag_icon(
                ui,
                pos2(r.rect.left() + ICON_X, r.rect.center().y),
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
    paint_highlight(ui, f, highlight, origin);
}

/// En-tête de section, cliquable pour replier ou déplier la section (flèche à droite) ;
/// renvoie `true` si la section est dépliée.
fn section(ui: &mut Ui, f: &mut Fx<'_>, s: SidebarSection, text: &str) -> bool {
    let pal = f.pal;
    let open = !f.state.settings.sidebar_collapsed.contains(&s);
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let r = ui.interact(rect, Id::unique(("side-section", s)), Sense::click());
    if ui.is_rect_visible(rect) {
        widgets::hover_fill(ui, &r, rect, &pal, 4.0);
        ui.painter().text(
            pos2(rect.left() + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            semibold(12.5),
            pal.text2,
        );
        let t = ui
            .ctx()
            .animate_bool_with_time(r.id.with("chevron"), open, 0.12);
        let color = if r.hovered() { pal.text2 } else { pal.text3 };
        chevron(ui, pos2(rect.right() - 12.0, rect.center().y), t, color);
    }
    let r = r.on_hover_text(if open {
        "Replier la section"
    } else {
        "Déplier la section"
    });
    if r.clicked() {
        f.send(super::settings_with(f.state, |st| {
            if open {
                st.sidebar_collapsed.push(s);
            } else {
                st.sidebar_collapsed.retain(|x| *x != s);
            }
        }));
    }
    ui.add_space(2.0);
    open
}

/// Flèche d'arborescence : `t` = 0 vers la droite (replié), 1 vers le bas (déplié).
fn chevron(ui: &Ui, center: Pos2, t: f32, color: Color32) {
    let (sin, cos) = (t * std::f32::consts::FRAC_PI_2).sin_cos();
    let rot = |x: f32, y: f32| center + vec2(x * cos - y * sin, x * sin + y * cos);
    ui.painter().add(egui::Shape::line(
        vec![rot(-1.75, -3.5), rot(1.75, 0.0), rot(-1.75, 3.5)],
        Stroke::new(1.3, color),
    ));
}

/// État de la flèche d'un dossier : `None` s'il est ouvert et n'a aucun sous-dossier.
fn expander(node: Option<&TreeNode>) -> Option<bool> {
    match node {
        None => Some(false),
        Some(n) if n.loaded && n.error.is_none() && n.children.is_empty() => None,
        Some(_) => Some(true),
    }
}

fn quick_access(ui: &mut Ui, f: &mut Fx<'_>, current: &Location) {
    let state = f.state;
    let pins = &state.settings.pinned;
    for (i, p) in pins.iter().enumerate() {
        let name = display_name(p);
        let known = state.known.iter().find(|(_, kp)| kp == p).map(|(k, _)| *k);
        let icon = match known {
            // Icône définie par Windows (Bureau, Téléchargements…), glyphe en attendant.
            Some(k) => Icon::Shell(p.clone(), known_glyph(k)),
            None => Icon::Shell(p.clone(), glyph::FOLDER),
        };
        let active = matches!(current, Location::Dir(d) if d == p);
        let node = state.tree.root(TreeSection::Pinned, p);
        let mut row = Row::new(Id::unique(("side-pin", p)), icon, &name, active);
        row.drop = Some(p);
        row.expander = expander(node);
        let out = draw_row(ui, f, row);
        let r = out.r;
        if out.toggled {
            f.send(Msg::ToggleTree {
                section: TreeSection::Pinned,
                root: p.clone(),
                path: p.clone(),
            });
        } else if r.clicked() {
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
        if let Some(node) = node {
            subtree(ui, f, TreeSection::Pinned, p, node, 1, current);
        }
    }
    if pins.is_empty() {
        ui.add(
            egui::Label::new(
                egui::RichText::new("Épinglez des dossiers depuis leur menu contextuel.")
                    .small()
                    .color(f.pal.text3),
            )
            .selectable(false),
        );
    }
    recycle_bin(ui, f, current);
}

fn drives(ui: &mut Ui, f: &mut Fx<'_>, current: &Location) {
    let state = f.state;
    let pal = f.pal;
    for d in &state.drives {
        let label = drive_label(d);
        let active = matches!(current, Location::Dir(p) if p == &d.root);
        let g = match d.kind {
            DriveKind::Network => glyph::NETWORK,
            DriveKind::Removable => glyph::USB,
            DriveKind::Optical => glyph::DISC,
            _ => glyph::DRIVE,
        };
        let thick = if d.total > 0 {
            state.settings.drive_bar.clamp(1.0, 12.0)
        } else {
            0.0
        };
        let node = state.tree.root(TreeSection::Drives, &d.root);
        let mut row = Row::new(
            Id::unique(("side-drive", &d.root)),
            Icon::Shell(d.root.clone(), g),
            &label,
            active,
        );
        row.drop = Some(&d.root);
        row.gauge = thick;
        row.expander = expander(node);
        let out = draw_row(ui, f, row);
        let r = out.r;
        if d.total > 0 && ui.is_rect_visible(r.rect) {
            let used = 1.0 - d.free as f32 / d.total as f32;
            let bar = Rect::from_min_size(
                pos2(r.rect.left() + TEXT_X, r.rect.top() + GAUGE_TOP),
                vec2(r.rect.width() - TEXT_X - 12.0, thick),
            );
            let color = if used > 0.9 { pal.danger } else { pal.accent };
            widgets::gradient_bar(ui, bar, used, color, pal.stroke_strong);
        }
        if out.toggled {
            f.send(Msg::ToggleTree {
                section: TreeSection::Drives,
                root: d.root.clone(),
                path: d.root.clone(),
            });
        } else if r.clicked() {
            f.send(Msg::Navigate(Location::Dir(d.root.clone())));
        }
        if r.middle_clicked() {
            f.send(Msg::OpenInNewTab(Location::Dir(d.root.clone())));
        }
        r.context_menu(|ui| {
            widgets::menu_appear(ui);
            ui.set_min_width(220.0);
            if ui
                .button(widgets::menu_entry(
                    glyph::ADD,
                    "Ouvrir dans un nouvel onglet",
                    &f.pal,
                ))
                .clicked()
            {
                f.send(Msg::OpenInNewTab(Location::Dir(d.root.clone())));
                ui.close();
            }
            if ui
                .button(widgets::menu_entry(glyph::PIE, "Analyser l'espace", &f.pal))
                .clicked()
            {
                f.send(Msg::AnalyzeSpace(d.root.clone()));
                ui.close();
            }
            ui.separator();
            if ui
                .button(widgets::menu_entry(glyph::INFO, "Propriétés", &f.pal))
                .clicked()
            {
                f.send(Msg::Properties(d.root.clone()));
                ui.close();
            }
        });
        r.on_hover_text(format!("{} libres sur {}", human(d.free), human(d.total)));
        if let Some(node) = node {
            subtree(ui, f, TreeSection::Drives, &d.root, node, 1, current);
        }
    }
}

/// Sous-dossiers d'un dossier ouvert, au niveau `depth`.
///
/// Un dossier peut compter des dizaines de milliers de sous-dossiers : entre deux
/// sous-dossiers ouverts, les lignes ont toutes la même hauteur, seules celles qui sont
/// visibles sont construites et le reste n'est que de l'espace réservé.
fn subtree(
    ui: &mut Ui,
    f: &mut Fx<'_>,
    section: TreeSection,
    root: &Path,
    node: &TreeNode,
    depth: usize,
    current: &Location,
) {
    if let Some(e) = &node.error {
        info_row(ui, f, depth, "Contenu inaccessible").on_hover_text(e);
        return;
    }
    // Sous-dossier affiché dans le volet : seul candidat au surlignage, retrouvé par
    // dichotomie (les noms sont triés).
    let active = match current {
        Location::Dir(d) if d.parent() == Some(node.path.as_path()) => d
            .file_name()
            .and_then(|n| node.children.find(n.as_encoded_bytes())),
        _ => None,
    };
    let step = SUB_ROW + ui.spacing().item_spacing.y;
    let n = node.children.len();
    let mut i = 0;
    let mut open = node.open.iter().peekable();
    while i < n {
        let stop = open.peek().map_or(n, |(k, _)| *k as usize).min(n);
        // Lignes fermées [i, stop) : seules les visibles sont construites.
        let top = ui.cursor().top();
        let clip = ui.clip_rect();
        let len = stop - i;
        let first = (((clip.top() - top) / step).floor().max(0.0) as usize).min(len);
        let last = ((((clip.bottom() - top) / step).ceil().max(0.0) as usize) + 1)
            .min(len)
            .max(first);
        // Ligne active hors de la vue : sa place suffit au surlignage (il glisse vers elle
        // et reste collé à sa ligne pendant le défilement).
        if let Some(a) = active.filter(|a| (i..stop).contains(a))
            && !(i + first..i + last).contains(&a)
        {
            f.vs.sidebar_active = Some(Rect::from_min_size(
                pos2(ui.max_rect().left(), top + (a - i) as f32 * step),
                vec2(ui.available_width(), SUB_ROW),
            ));
        }
        ui.add_space(first as f32 * step);
        for k in i + first..i + last {
            child_row(
                ui,
                f,
                section,
                root,
                node,
                k,
                None,
                depth,
                active == Some(k),
            );
        }
        ui.add_space((len - last) as f32 * step);
        let Some((k, child)) = open.next() else {
            break;
        };
        let k = *k as usize;
        child_row(
            ui,
            f,
            section,
            root,
            node,
            k,
            Some(child),
            depth,
            active == Some(k),
        );
        subtree(ui, f, section, root, child, depth + 1, current);
        i = k + 1;
    }
    if node.children.truncated > 0 {
        let more = node.children.truncated;
        info_row(
            ui,
            f,
            depth,
            &format!(
                "… et {} autre{} dossier{}",
                super::status::thousands(more as u64),
                if more > 1 { "s" } else { "" },
                if more > 1 { "s" } else { "" }
            ),
        );
    }
}

/// Ligne du `i`-ième sous-dossier de `node` ; `open` : ce sous-dossier, s'il est ouvert.
#[allow(clippy::too_many_arguments)]
fn child_row(
    ui: &mut Ui,
    f: &mut Fx<'_>,
    section: TreeSection,
    root: &Path,
    node: &TreeNode,
    i: usize,
    open: Option<&TreeNode>,
    depth: usize,
    active: bool,
) {
    let label = fx_core::wtf8::display(node.children.name(i));
    let path = node.child_path(i);
    let custom = node.children.attributes(i) & (attrs::READONLY | attrs::SYSTEM) != 0;
    let mut row = Row::new(
        Id::unique(("side-tree", section, &path)),
        Icon::Folder(path.clone(), custom),
        &label,
        active,
    );
    row.drop = Some(&path);
    row.depth = depth;
    row.expander = expander(open);
    let out = draw_row(ui, f, row);
    let r = out.r;
    if out.toggled {
        f.send(Msg::ToggleTree {
            section,
            root: root.to_path_buf(),
            path: path.clone(),
        });
    } else if r.clicked() {
        f.send(Msg::Navigate(Location::Dir(path.clone())));
    }
    if r.middle_clicked() {
        f.send(Msg::OpenInNewTab(Location::Dir(path.clone())));
    }
    r.context_menu(|ui| {
        widgets::menu_appear(ui);
        ui.set_min_width(220.0);
        if ui
            .button(widgets::menu_entry(
                glyph::ADD,
                "Ouvrir dans un nouvel onglet",
                &f.pal,
            ))
            .clicked()
        {
            f.send(Msg::OpenInNewTab(Location::Dir(path.clone())));
            ui.close();
        }
        if ui.button("Épingler à l'accès rapide").clicked() {
            f.send(Msg::Pin(path.clone()));
            ui.close();
        }
        if ui.button("Copier le chemin").clicked() {
            ui.ctx().copy_text(path.display().to_string());
            ui.close();
        }
        ui.separator();
        if ui
            .button(widgets::menu_entry(glyph::INFO, "Propriétés", &f.pal))
            .clicked()
        {
            f.send(Msg::Properties(path.clone()));
            ui.close();
        }
    });
}

/// Ligne d'information grisée dans l'arborescence (erreur, liste tronquée).
fn info_row(ui: &mut Ui, f: &Fx<'_>, depth: usize, text: &str) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), SUB_ROW), Sense::hover());
    if ui.is_rect_visible(rect) {
        let x = rect.left() + TEXT_X + depth as f32 * INDENT;
        ui.painter().with_clip_rect(rect).text(
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            egui::FontId::proportional(12.0),
            f.pal.text3,
        );
    }
    r
}

/// Surlignage de l'élément actif, qui glisse vers sa nouvelle position (180 ms) et s'efface
/// quand aucun élément de la barre ne correspond à l'emplacement affiché.
///
/// Positions relatives au haut du contenu (`origin`, qui suit le défilement) : en
/// défilant, le surlignage reste collé à sa ligne au lieu de la rattraper en 180 ms, et
/// une ligne sortie de la vue puis revenue le retrouve en place.
fn paint_highlight(ui: &Ui, f: &mut Fx<'_>, slot: egui::layers::ShapeIdx, origin: f32) {
    let pal = f.pal;
    let ctx = ui.ctx().clone();
    let id = Id::unique("sidebar-highlight");
    let shown = f.vs.sidebar_active.is_some();
    let alpha = ctx.animate_bool_with_time(id.with("a"), shown, 0.15);
    let active = f.vs.sidebar_active.map(|r| r.translate(vec2(0.0, -origin)));
    let Some(target) = active.or(f.vs.sidebar_last) else {
        return;
    };
    f.vs.sidebar_last = Some(target);
    let target = target.translate(vec2(0.0, origin));
    let y = origin + ctx.animate_value_with_time(id.with("y"), target.top() - origin, 0.18);
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
            egui::Shape::rect_filled(rect, 4.0, pal.hover.gamma_multiply(1.4 * alpha)),
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
    let row = Row::new(
        Id::unique("side-recycle"),
        icon,
        "Corbeille",
        *current == Location::RecycleBin,
    );
    let r = draw_row(ui, f, row).r;
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
    if section(ui, f, SidebarSection::Network, "Réseau") {
        let icon = Icon::Shell(PathBuf::from(fx_app::history::NETWORK_ID), glyph::NETWORK);
        let active = matches!(current, Location::Network(_));
        let row = Row::new(Id::unique("side-network"), icon, "Réseau", active);
        let r = draw_row(ui, f, row).r;
        if r.clicked() {
            f.send(Msg::Navigate(Location::Network(None)));
        }
        if r.middle_clicked() {
            f.send(Msg::OpenInNewTab(Location::Network(None)));
        }
    }

    let state = f.state;
    if state.wsl.is_empty() || !section(ui, f, SidebarSection::Wsl, "WSL") {
        return;
    }
    for name in &state.wsl {
        let path = PathBuf::from(format!(r"\\wsl.localhost\{name}"));
        let node = state.tree.root(TreeSection::Wsl, &path);
        // Distribution surlignée partout à l'intérieur, sauf quand l'arborescence ouverte
        // montre le dossier affiché (son sous-dossier est surligné à la place).
        let active = matches!(current, Location::Dir(d) if d.starts_with(&path));
        // Icône « Linux » de Windows (dossier shell de WSL) : n'éveille pas la machine
        // virtuelle, contrairement à l'icône du dossier de la distribution.
        let icon = Icon::Shell(PathBuf::from(LINUX_ID), glyph::LINUX);
        let mut row = Row::new(Id::unique(("side-wsl", name)), icon, name, active);
        row.drop = Some(&path);
        row.expander = expander(node);
        let out = draw_row(ui, f, row);
        let r = out.r;
        if out.toggled {
            f.send(Msg::ToggleTree {
                section: TreeSection::Wsl,
                root: path.clone(),
                path: path.clone(),
            });
        } else if r.clicked() {
            f.send(Msg::Navigate(Location::Dir(path.clone())));
        }
        if r.middle_clicked() {
            f.send(Msg::OpenInNewTab(Location::Dir(path.clone())));
        }
        r.on_hover_text(path.display().to_string());
        if let Some(node) = node {
            subtree(ui, f, TreeSection::Wsl, &path, node, 1, current);
        }
    }
}

/// Dossier shell « Linux » (racine des distributions WSL dans l'Explorateur).
const LINUX_ID: &str = "::{B2B4A4D1-2754-4140-A2EB-9A76D9D7CDC6}";

/// Haut de la jauge d'un lecteur dans sa ligne (cf. [`draw_row`]).
const GAUGE_TOP: f32 = 24.0;

/// Dessine une ligne. Avec une jauge, la ligne grandit et libellé + jauge restent centrés,
/// pour que surlignage, survol et pastille couvrent l'ensemble. Un clic sur la flèche (ou
/// juste autour) déroule le dossier au lieu d'y naviguer.
fn draw_row(ui: &mut Ui, f: &mut Fx<'_>, row: Row<'_>) -> RowResponse {
    let pal = f.pal;
    let (height, content_y) = if row.gauge > 0.0 {
        ((GAUGE_TOP + row.gauge + 5.0).max(ROW), GAUGE_TOP - 11.0)
    } else if row.depth > 0 {
        (SUB_ROW, SUB_ROW / 2.0)
    } else {
        (ROW, ROW / 2.0)
    };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    // Identifiant fixe (et non celui de la position) : les lignes de l'arborescence hors de
    // la vue ne sont pas construites, une ligne garde ainsi survol et menu en défilant.
    let r = ui.interact(rect, row.id, Sense::click());
    let content_y = rect.top() + content_y;
    let indent = row.depth as f32 * INDENT;
    let hit = Rect::from_min_max(
        pos2(rect.left() + indent, rect.top()),
        pos2(rect.left() + indent + CHEVRON_X * 2.0, rect.bottom()),
    );
    let on_chevron = row.expander.is_some()
        && r.interact_pointer_pos()
            .or_else(|| r.hover_pos())
            .is_some_and(|p| hit.contains(p));
    let toggled = on_chevron && r.clicked();
    // Même hors de la vue : le surlignage reste ainsi en place pendant le défilement.
    if row.active {
        f.vs.sidebar_active = Some(rect);
    }
    if !ui.is_rect_visible(rect) {
        return RowResponse { r, toggled };
    }
    if let Some(d) = row.drop {
        super::dnd::zone(f, rect, super::dnd::Target::Dir(d.to_path_buf()));
    }
    if !row.active {
        widgets::hover_fill(ui, &r, rect, &pal, 4.0);
    }
    if let Some(open) = row.expander {
        let t = ui
            .ctx()
            .animate_bool_with_time(row.id.with("chevron"), open, 0.12);
        let color = if on_chevron { pal.text } else { pal.text3 };
        chevron(
            ui,
            pos2(rect.left() + indent + CHEVRON_X, content_y),
            t,
            color,
        );
    }
    let icon_center = pos2(rect.left() + indent + ICON_X, content_y);
    match row.icon {
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
        Icon::Folder(path, true) => {
            super::items::shell_icon(ui, f, &path, icon_center, 16.0, glyph::FOLDER, pal.folder);
        }
        Icon::Folder(path, false) => folder_icon(ui, f, &path, icon_center, 16.0),
    }
    f.vs.fallbacks.scan(row.label);
    let text_rect = Rect::from_min_max(
        pos2(rect.left() + indent + TEXT_X, rect.top()),
        pos2(rect.right() - 4.0, rect.bottom()),
    );
    let font = if row.active {
        semibold(13.5)
    } else {
        egui::FontId::proportional(13.5)
    };
    ui.painter().with_clip_rect(text_rect).text(
        pos2(text_rect.left(), content_y - 1.0),
        Align2::LEFT_CENTER,
        row.label,
        font,
        pal.text,
    );
    RowResponse { r, toggled }
}

/// Icône de dossier commune (une seule image pour tous les dossiers ordinaires, comme dans
/// la liste) ; glyphe en attendant.
fn folder_icon(ui: &Ui, f: &mut Fx<'_>, path: &Path, center: Pos2, size: f32) {
    let px = (size * ui.ctx().pixels_per_point()).round() as u16;
    let key = fx_shell::ImageKey::Folder { size: px.max(16) };
    match f.images.get(f.shell, &key, path) {
        Some(tex) => {
            ui.painter().image(
                tex.id(),
                Rect::from_center_size(center, vec2(size, size)),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            ui.painter().text(
                center,
                Align2::CENTER_CENTER,
                glyph::FOLDER,
                icon_font(size * 0.85),
                f.pal.folder,
            );
        }
    }
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
