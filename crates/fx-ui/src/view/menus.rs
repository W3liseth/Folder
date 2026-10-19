//! Menus contextuels de fx (le menu shell complet est accessible par
//! « Afficher plus d'options », comme sous Windows 11).

use std::path::PathBuf;

use eframe::egui::{self, Ui};
use fx_app::Msg;
use fx_commands::CommandId;

use super::toolbar::{menu_cmd, sort_menu, view_menu};
use super::{Fx, ShellMenu};
use crate::theme::glyph;
use crate::widgets::menu_entry;

pub fn item_menu(ui: &mut Ui, f: &mut Fx<'_>, paths: &[PathBuf], is_dir: bool) {
    crate::widgets::menu_appear(ui);
    ui.set_min_width(260.0);
    if matches!(f.state.pane().location(), fx_app::Location::RecycleBin) {
        menu_cmd(ui, f, CommandId::Restore, "");
        menu_cmd(ui, f, CommandId::Delete, "Supprimer définitivement");
        ui.separator();
        menu_cmd(ui, f, CommandId::Properties, "");
        return;
    }
    let first = paths.first().cloned();
    let single = paths.len() == 1;
    let mut open = menu_entry(CommandId::Open.glyph(), "Ouvrir", &f.pal);
    if let Some(s) = open.sections.last_mut() {
        s.format.font_id =
            egui::FontId::new(14.0, egui::FontFamily::Name(crate::theme::SEMIBOLD.into()));
    }
    if ui
        .add(egui::Button::new(open).shortcut_text(f.shortcut(CommandId::Open)))
        .clicked()
    {
        f.cmd(CommandId::Open);
        ui.close();
    }
    if is_dir {
        menu_cmd(ui, f, CommandId::OpenInNewTab, "");
        if f.state.tab().panes.len() > 1 {
            menu_cmd(ui, f, CommandId::OpenInOtherPane, "");
        }
    } else if single {
        menu_cmd(ui, f, CommandId::OpenWith, "");
    }
    ui.separator();
    for cmd in [CommandId::Cut, CommandId::Copy] {
        menu_cmd(ui, f, cmd, "");
    }
    if is_dir
        && single
        && f.state.clipboard_has_files
        && ui
            .button(menu_entry(glyph::PASTE, "Coller dans ce dossier", &f.pal))
            .clicked()
    {
        if let Some(p) = &first {
            f.send(Msg::PasteInto(p.clone()));
        }
        ui.close();
    }
    ui.separator();
    for cmd in [
        CommandId::CopyPath,
        CommandId::Rename,
        CommandId::BulkRename,
        CommandId::Delete,
    ] {
        if cmd == CommandId::BulkRename && single {
            continue;
        }
        menu_cmd(ui, f, cmd, "");
    }
    ui.separator();
    tags_menu(ui, f, paths);
    if is_dir {
        menu_cmd(ui, f, CommandId::PinToSidebar, "");
    }
    menu_cmd(ui, f, CommandId::CompressToZip, "");
    if paths
        .iter()
        .any(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")))
    {
        menu_cmd(ui, f, CommandId::ExtractHere, "");
    }
    ui.separator();
    if is_dir && single {
        menu_cmd(ui, f, CommandId::OpenTerminal, "");
    }
    let runnable = |p: &PathBuf| {
        p.extension().is_some_and(|e| {
            ["exe", "bat", "cmd", "msi", "ps1", "com"]
                .iter()
                .any(|x| e.eq_ignore_ascii_case(x))
        })
    };
    if !is_dir && single && first.as_ref().is_some_and(runnable) {
        menu_cmd(ui, f, CommandId::RunAsAdmin, "");
    }
    menu_cmd(ui, f, CommandId::Properties, "");
    ui.separator();
    if ui
        .add(
            egui::Button::new(menu_entry(
                CommandId::ShowMoreOptions.glyph(),
                "Afficher plus d'options",
                &f.pal,
            ))
            .shortcut_text(f.shortcut(CommandId::ShowMoreOptions)),
        )
        .clicked()
    {
        let at = fx_win::window::cursor_pos();
        let extended = ui.input(|i| i.modifiers.shift);
        f.vs.shell_menu = Some((ShellMenu::Items(paths.to_vec()), at, extended));
        ui.close();
    }
}

fn tags_menu(ui: &mut Ui, f: &mut Fx<'_>, paths: &[PathBuf]) {
    if !f.state.settings.show_tags || f.state.tags.list.is_empty() {
        return;
    }
    ui.menu_button(menu_entry(glyph::TAG, "Étiquettes", &f.pal), |ui| {
        crate::widgets::menu_appear(ui);
        ui.set_min_width(200.0);
        tag_list(ui, f, paths);
    });
}

/// Étiquettes cochables pour des chemins (coche si tous l'ont) ; menu contextuel et barre
/// d'outils.
pub(crate) fn tag_list(ui: &mut Ui, f: &mut Fx<'_>, paths: &[PathBuf]) {
    let tags = f.state.tags.list.clone();
    for t in tags {
        let on = !paths.is_empty() && paths.iter().all(|p| f.state.tags.of(p).contains(&t.id));
        let color = egui::Color32::from_rgb(t.color[0], t.color[1], t.color[2]);
        // Même icône que la barre latérale, dans la couleur de l'étiquette ; coche à droite.
        let job = crate::widgets::menu_entry_colored(glyph::TAG, color, &t.name, &f.pal);
        let check = if on {
            egui::RichText::new(glyph::CHECK)
                .font(crate::theme::icon_font(12.0))
                .color(f.pal.accent)
        } else {
            egui::RichText::new(" ")
        };
        if ui
            .add_enabled(
                !paths.is_empty(),
                egui::Button::new(job).shortcut_text(check),
            )
            .clicked()
        {
            f.send(Msg::SetTag {
                paths: paths.to_vec(),
                tag: t.id,
                on: !on,
            });
            ui.close();
        }
    }
}

/// Menu du fond d'un dossier.
pub fn background_menu(ui: &mut Ui, f: &mut Fx<'_>) {
    crate::widgets::menu_appear(ui);
    ui.set_min_width(260.0);
    if matches!(f.state.pane().location(), fx_app::Location::RecycleBin) {
        menu_cmd(ui, f, CommandId::EmptyRecycleBin, "");
        if ui
            .button(menu_entry(
                glyph::UNDELETE,
                "Restaurer tous les éléments",
                &f.pal,
            ))
            .clicked()
        {
            f.send(Msg::RestoreRecycled { all: true });
            ui.close();
        }
        ui.separator();
        let (view, sort) = (
            menu_entry(glyph::VIEW, "Affichage", &f.pal),
            menu_entry(glyph::SORT, "Trier par", &f.pal),
        );
        ui.menu_button(view, |ui| view_menu(ui, f));
        ui.menu_button(sort, |ui| sort_menu(ui, f));
        menu_cmd(ui, f, CommandId::Refresh, "");
        return;
    }
    let (view, sort) = (
        menu_entry(glyph::VIEW, "Affichage", &f.pal),
        menu_entry(glyph::SORT, "Trier par", &f.pal),
    );
    ui.menu_button(view, |ui| view_menu(ui, f));
    ui.menu_button(sort, |ui| sort_menu(ui, f));
    menu_cmd(ui, f, CommandId::Refresh, "");
    ui.separator();
    menu_cmd(ui, f, CommandId::Paste, "");
    menu_cmd(ui, f, CommandId::Undo, "");
    ui.separator();
    ui.menu_button(menu_entry(glyph::NEW, "Nouveau", &f.pal), |ui| {
        menu_cmd(ui, f, CommandId::NewFolder, "Dossier");
        menu_cmd(ui, f, CommandId::NewFile, "Document texte");
    });
    ui.separator();
    menu_cmd(ui, f, CommandId::SelectAll, "");
    menu_cmd(ui, f, CommandId::OpenTerminal, "");
    menu_cmd(ui, f, CommandId::PinToSidebar, "Épingler ce dossier");
    menu_cmd(ui, f, CommandId::Properties, "");
    ui.separator();
    if ui
        .button(menu_entry(
            CommandId::ShowMoreOptions.glyph(),
            "Afficher plus d'options",
            &f.pal,
        ))
        .clicked()
    {
        if let Some(dir) = f.state.pane().working_dir() {
            let at = fx_win::window::cursor_pos();
            f.vs.shell_menu = Some((
                ShellMenu::Background(dir.to_path_buf()),
                at,
                ui.input(|i| i.modifiers.shift),
            ));
        }
        ui.close();
    }
}
