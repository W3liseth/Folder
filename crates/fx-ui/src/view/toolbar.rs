//! Barre de commandes (style Explorateur de Windows 11 / Files) : chaque outil a son icône
//! et son libellé ; ceux qui ne tiennent pas dans la largeur passent dans le menu « … ».

use eframe::egui::{self, Frame, Margin, Popup, Ui};
use fx_app::Msg;
use fx_commands::CommandId;
use fx_core::{SortDir, SortKey};
use fx_store::Layout;

use super::Fx;
use crate::theme::glyph;
use crate::widgets;

/// Outil de la barre : une commande, ou un menu déroulant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Cmd(CommandId),
    Sort,
    View,
    Tags,
}

use Tool::Cmd;

/// Outils après « Nouveau », par groupes séparés d'un trait, dans l'ordre d'affichage.
const GROUPS: &[&[Tool]] = &[
    &[
        Cmd(CommandId::Cut),
        Cmd(CommandId::Copy),
        Cmd(CommandId::Paste),
        Cmd(CommandId::Rename),
        Cmd(CommandId::Delete),
    ],
    &[Tool::Sort, Tool::View],
    &[Tool::Tags],
    &[Cmd(CommandId::Undo), Cmd(CommandId::Redo)],
    &[
        Cmd(CommandId::SelectAll),
        Cmd(CommandId::InvertSelection),
        Cmd(CommandId::ClearSelection),
    ],
    &[
        Cmd(CommandId::CopyPath),
        Cmd(CommandId::BulkRename),
        Cmd(CommandId::PinToSidebar),
        Cmd(CommandId::CompressToZip),
        Cmd(CommandId::ExtractHere),
    ],
    &[
        Cmd(CommandId::OpenTerminal),
        Cmd(CommandId::NewWindow),
        Cmd(CommandId::Properties),
    ],
];
/// Séparateur (9 px) et bouton « … » (36 px), espacement compris.
const SEP_W: f32 = 11.0;
const MORE_W: f32 = 38.0;
const FONT: f32 = 13.5;

pub fn draw(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    egui::Panel::top("toolbar")
        .exact_size(44.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(pal.layer).inner_margin(Margin {
            left: 8,
            right: 10,
            top: 2,
            bottom: 6,
        }))
        .show(ui, |ui| {
            let bottom = ui.max_rect().bottom() + 6.0;
            ui.painter().hline(
                ui.max_rect().x_range(),
                bottom - 0.5,
                egui::Stroke::new(1.0, pal.stroke),
            );
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let ctx = f.state.command_context();

                if ctx.in_recycle_bin {
                    recycle_bin_buttons(ui, f, &ctx);
                } else {
                    new_button(ui, f);
                }
                // Corbeille : ses propres boutons remplacent le premier groupe ; pas
                // d'étiquettes sur des éléments supprimés.
                let tags = f.state.settings.show_tags && !ctx.in_recycle_bin;
                let groups: Vec<Vec<Tool>> = GROUPS
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !(ctx.in_recycle_bin && *i == 0))
                    .map(|(_, g)| {
                        g.iter()
                            .copied()
                            .filter(|t| tags || *t != Tool::Tags)
                            .collect::<Vec<_>>()
                    })
                    .filter(|g| !g.is_empty())
                    .collect();
                let widths: Vec<Vec<f32>> = groups
                    .iter()
                    .map(|g| g.iter().map(|&t| tool_width(ui, t)).collect())
                    .collect();
                let right_w = button_width(ui, "Aperçu") + button_width(ui, "Double volet") + 16.0;
                let shown = fit(&widths, ui.available_width() - right_w);

                let mut overflow: Vec<&[Tool]> = Vec::new();
                for (g, &n) in groups.iter().zip(&shown) {
                    if n > 0 {
                        widgets::vsep(ui, &pal);
                        for &t in &g[..n] {
                            tool(ui, f, t);
                        }
                    }
                    if n < g.len() {
                        overflow.push(&g[n..]);
                    }
                }
                if !overflow.is_empty() {
                    widgets::vsep(ui, &pal);
                    let more = widgets::tool_button(ui, &pal, glyph::MORE, None, "", true)
                        .on_hover_text("Plus d'options");
                    Popup::menu(&more).show(|ui| more_menu(ui, f, &overflow));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let preview = f.state.settings.show_preview;
                    let r = widgets::tool_button(
                        ui,
                        &pal,
                        glyph::PREVIEW,
                        preview.then_some(pal.accent),
                        "Aperçu",
                        true,
                    )
                    .on_hover_text(format!(
                        "Volet d'aperçu ({})",
                        f.shortcut(CommandId::TogglePreview)
                    ));
                    if r.clicked() {
                        f.cmd(CommandId::TogglePreview);
                    }
                    let dual = f.state.tab().panes.len() > 1;
                    let r = widgets::tool_button(
                        ui,
                        &pal,
                        glyph::DUAL,
                        dual.then_some(pal.accent),
                        "Double volet",
                        true,
                    )
                    .on_hover_text(format!(
                        "Double volet ({})",
                        f.shortcut(CommandId::ToggleDualPane)
                    ));
                    if r.clicked() {
                        f.cmd(CommandId::ToggleDualPane);
                    }
                });
            });
        });
}

/// « Nouveau » : dossier ou document texte.
fn new_button(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    let new = widgets::accent_button(ui, &pal, glyph::NEW, "Nouveau", true);
    ui.add_space(4.0);
    Popup::menu(&new).show(|ui| {
        widgets::menu_appear(ui);
        ui.set_min_width(220.0);
        menu_cmd(ui, f, CommandId::NewFolder, "Dossier");
        menu_cmd(ui, f, CommandId::NewFile, "Document texte");
    });
}

/// Libellé dans la barre (plus court que celui des menus quand il le faut).
fn tool_label(t: Tool) -> &'static str {
    match t {
        Tool::Sort => "Trier",
        Tool::View => "Afficher",
        Tool::Tags => "Étiquette",
        Cmd(CommandId::CopyPath) => "Copier le chemin",
        Cmd(CommandId::BulkRename) => "Renommer en masse",
        Cmd(CommandId::PinToSidebar) => "Épingler",
        Cmd(CommandId::CompressToZip) => "Compresser",
        Cmd(CommandId::OpenTerminal) => "Terminal",
        Cmd(CommandId::InvertSelection) => "Inverser",
        Cmd(c) => c.label(),
    }
}

fn tool_glyph(t: Tool) -> &'static str {
    match t {
        Tool::Sort => glyph::SORT,
        Tool::View => glyph::VIEW,
        Tool::Tags => glyph::TAG,
        Cmd(c) => c.glyph(),
    }
}

/// Largeur d'un bouton avec libellé (cf. [`widgets::tool_button`]), espacement compris.
fn button_width(ui: &Ui, label: &str) -> f32 {
    let text = ui
        .painter()
        .layout_no_wrap(
            label.to_owned(),
            egui::FontId::proportional(FONT),
            egui::Color32::WHITE,
        )
        .size()
        .x;
    10.0 + 16.0 + 8.0 + text + 12.0 + 2.0
}

fn tool_width(ui: &Ui, t: Tool) -> f32 {
    match t {
        // Les menus déroulants ajoutent la place du chevron.
        Tool::Sort | Tool::View | Tool::Tags => button_width(ui, &format!("{}  ", tool_label(t))),
        Cmd(_) => button_width(ui, tool_label(t)),
    }
}

/// Nombre d'outils affichés par groupe pour une largeur disponible : groupes entiers tant
/// qu'ils tiennent, le premier qui ne tient pas est tronqué, les suivants passent dans « … »
/// (dont la place n'est réservée que s'il sert).
fn fit(widths: &[Vec<f32>], avail: f32) -> Vec<usize> {
    let total: f32 = widths.iter().map(|g| SEP_W + g.iter().sum::<f32>()).sum();
    let mut budget = if total <= avail {
        avail
    } else {
        avail - SEP_W - MORE_W
    };
    let mut full = true;
    widths
        .iter()
        .map(|g| {
            if !full {
                return 0;
            }
            budget -= SEP_W;
            let mut n = 0;
            for &w in g {
                if w > budget {
                    full = false;
                    break;
                }
                budget -= w;
                n += 1;
            }
            n
        })
        .collect()
}

/// Un outil de la barre.
fn tool(ui: &mut Ui, f: &mut Fx<'_>, t: Tool) {
    let pal = f.pal;
    let label = tool_label(t);
    match t {
        Tool::Sort => {
            let r = widgets::menu_label(ui, &pal, glyph::SORT, label);
            Popup::menu(&r).show(|ui| sort_menu(ui, f));
        }
        Tool::View => {
            let r = widgets::menu_label(ui, &pal, glyph::VIEW, label);
            Popup::menu(&r).show(|ui| view_menu(ui, f));
        }
        Tool::Tags => {
            let r = widgets::menu_label(ui, &pal, glyph::TAG, label)
                .on_hover_text("Ajouter ou retirer une étiquette de la sélection");
            Popup::menu(&r).show(|ui| {
                widgets::menu_appear(ui);
                tag_entries(ui, f);
            });
        }
        Cmd(cmd) => {
            let name = match (cmd, f.state.undo.undo_label(), f.state.undo.redo_label()) {
                (CommandId::Undo, Some(l), _) => format!("Annuler : {l}"),
                (CommandId::Redo, _, Some(l)) => format!("Rétablir : {l}"),
                _ => cmd.label().to_owned(),
            };
            let shortcut = f.shortcut(cmd);
            let tip = if shortcut.is_empty() {
                name
            } else {
                format!("{name} ({shortcut})")
            };
            let r = widgets::tool_button(ui, &pal, tool_glyph(t), None, label, f.enabled(cmd))
                .on_hover_text(tip);
            if r.clicked() {
                f.cmd(cmd);
            }
        }
    }
}

/// Étiquettes de la sélection : cocher pour ajouter, décocher pour retirer.
fn tag_entries(ui: &mut Ui, f: &mut Fx<'_>) {
    ui.set_min_width(220.0);
    let paths = f.state.pane().selected_paths();
    if f.state.tags.list.is_empty() {
        ui.label(egui::RichText::new("Aucune étiquette pour l'instant.").color(f.pal.text2));
        if ui
            .button(widgets::menu_entry(
                glyph::SETTINGS,
                "Créer des étiquettes…",
                &f.pal,
            ))
            .clicked()
        {
            f.cmd(CommandId::Settings);
            ui.close();
        }
        return;
    }
    if paths.is_empty() {
        ui.label(egui::RichText::new("Sélectionnez des éléments à étiqueter.").color(f.pal.text2));
        ui.separator();
    }
    super::menus::tag_list(ui, f, &paths);
}

/// Corbeille : vider, tout restaurer, restaurer la sélection, supprimer définitivement.
fn recycle_bin_buttons(ui: &mut Ui, f: &mut Fx<'_>, ctx: &fx_commands::CommandContext) {
    let pal = f.pal;
    let empty = f
        .state
        .pane()
        .listing
        .as_ref()
        .is_none_or(|l| l.total() == 0);
    if widgets::tool_button(ui, &pal, glyph::RECYCLE, None, "Vider la corbeille", !empty).clicked()
    {
        f.cmd(CommandId::EmptyRecycleBin);
    }
    widgets::vsep(ui, &pal);
    if widgets::tool_button(
        ui,
        &pal,
        glyph::UNDELETE,
        None,
        "Restaurer tous les éléments",
        !empty,
    )
    .clicked()
    {
        f.send(Msg::RestoreRecycled { all: true });
    }
    if widgets::tool_button(
        ui,
        &pal,
        glyph::UNDELETE,
        None,
        "Restaurer la sélection",
        CommandId::Restore.is_enabled(ctx),
    )
    .clicked()
    {
        f.cmd(CommandId::Restore);
    }
    let r = widgets::tool_button(
        ui,
        &pal,
        glyph::DELETE,
        None,
        "",
        CommandId::Delete.is_enabled(ctx),
    )
    .on_hover_text("Supprimer définitivement (Suppr)");
    if r.clicked() {
        f.cmd(CommandId::Delete);
    }
    widgets::vsep(ui, &pal);
}

/// Entrée de menu pour une commande (libellé, raccourci, état).
pub(crate) fn menu_cmd(ui: &mut Ui, f: &mut Fx<'_>, cmd: CommandId, label: &str) {
    let enabled = f.enabled(cmd);
    let text = if label.is_empty() { cmd.label() } else { label };
    let shortcut = f.shortcut(cmd);
    let button =
        egui::Button::new(widgets::menu_entry(cmd.glyph(), text, &f.pal)).shortcut_text(shortcut);
    if ui.add_enabled(enabled, button).clicked() {
        f.cmd(cmd);
        ui.close();
    }
}

fn check(ui: &mut Ui, f: &mut Fx<'_>, on: bool, label: &str, cmd: CommandId) {
    let button =
        egui::Button::new(widgets::check_label(on, label, &f.pal)).shortcut_text(f.shortcut(cmd));
    if ui.add(button).clicked() {
        f.cmd(cmd);
        ui.close();
    }
}

pub(crate) fn sort_menu(ui: &mut Ui, f: &mut Fx<'_>) {
    widgets::menu_appear(ui);
    ui.set_min_width(220.0);
    let cur = f.state.pane().listing.as_ref().map(|l| l.sort());
    for (key, label) in [
        (SortKey::Name, "Nom"),
        (SortKey::Modified, "Date de modification"),
        (SortKey::Type, "Type"),
        (SortKey::Size, "Taille"),
    ] {
        let on = cur.is_some_and(|s| s.key == key);
        if ui.button(widgets::check_label(on, label, &f.pal)).clicked() {
            if !on {
                f.send(Msg::SortBy(key));
            }
            ui.close();
        }
    }
    ui.separator();
    let desc = cur.is_some_and(|s| s.dir == SortDir::Desc);
    check(ui, f, !desc, "Croissant", CommandId::ToggleSortDirection);
    check(ui, f, desc, "Décroissant", CommandId::ToggleSortDirection);
    ui.separator();
    check(
        ui,
        f,
        f.state.settings.folders_first,
        "Dossiers en premier",
        CommandId::ToggleFoldersFirst,
    );
}

pub(crate) fn view_menu(ui: &mut Ui, f: &mut Fx<'_>) {
    widgets::menu_appear(ui);
    ui.set_min_width(240.0);
    let cur = f.state.pane().layout;
    for (layout, cmd) in [
        (Layout::Details, CommandId::LayoutDetails),
        (Layout::List, CommandId::LayoutList),
        (Layout::Grid, CommandId::LayoutGrid),
        (Layout::Columns, CommandId::LayoutColumns),
    ] {
        check(ui, f, cur == layout, layout.label(), cmd);
    }
    ui.separator();
    menu_cmd(ui, f, CommandId::ZoomIn, "");
    menu_cmd(ui, f, CommandId::ZoomOut, "");
    ui.separator();
    let s = &f.state.settings;
    let (hidden, ext, side, prev) = (
        s.show_hidden,
        s.show_extensions,
        s.show_sidebar,
        s.show_preview,
    );
    check(ui, f, hidden, "Éléments masqués", CommandId::ToggleHidden);
    check(
        ui,
        f,
        ext,
        "Extensions de fichiers",
        CommandId::ToggleExtensions,
    );
    check(ui, f, side, "Barre latérale", CommandId::ToggleSidebar);
    check(ui, f, prev, "Volet d'aperçu", CommandId::TogglePreview);
}

fn more_menu(ui: &mut Ui, f: &mut Fx<'_>, groups: &[&[Tool]]) {
    widgets::menu_appear(ui);
    ui.set_min_width(260.0);
    for (i, group) in groups.iter().enumerate() {
        if i > 0 {
            ui.separator();
        }
        for &t in *group {
            let pal = f.pal;
            match t {
                Cmd(cmd) => menu_cmd(ui, f, cmd, ""),
                Tool::Sort => {
                    ui.menu_button(widgets::menu_entry(glyph::SORT, "Trier", &pal), |ui| {
                        sort_menu(ui, f);
                    });
                }
                Tool::View => {
                    ui.menu_button(widgets::menu_entry(glyph::VIEW, "Afficher", &pal), |ui| {
                        view_menu(ui, f);
                    });
                }
                Tool::Tags => {
                    ui.menu_button(widgets::menu_entry(glyph::TAG, "Étiquette", &pal), |ui| {
                        widgets::menu_appear(ui);
                        tag_entries(ui, f);
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_fit_or_overflow() {
        let widths = vec![vec![50.0; 5], vec![60.0, 60.0], vec![40.0; 3]];
        let total: f32 = 3.0 * SEP_W + 250.0 + 120.0 + 120.0;
        // Tout tient : rien dans « … ».
        assert_eq!(fit(&widths, total), [5, 2, 3]);
        // Rien ne tient.
        assert_eq!(fit(&widths, 0.0), [0, 0, 0]);
        // Premier groupe entier et un outil du second ; la place de « … » est réservée.
        let avail = SEP_W + MORE_W + SEP_W + 250.0 + SEP_W + 60.0 + 1.0;
        assert_eq!(fit(&widths, avail), [5, 1, 0]);
        // Un groupe suivant plus étroit ne passe pas devant un groupe tronqué.
        let avail = SEP_W + MORE_W + SEP_W + 250.0 + SEP_W + 50.0;
        assert_eq!(fit(&widths, avail), [5, 0, 0]);
    }
}
