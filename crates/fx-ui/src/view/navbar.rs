//! Barre de navigation : historique, omnibar (fil d'Ariane / saisie du chemin), recherche.

use std::path::{Component, Path, PathBuf};

use eframe::egui::{
    self, Align2, Frame, Id, Key, Margin, Rect, Sense, Stroke, TextEdit, Ui, pos2, vec2,
};
use fx_app::{Location, Msg};
use fx_commands::CommandId;

use super::Fx;
use crate::theme::{glyph, icon_font};
use crate::widgets;

pub fn draw(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    egui::Panel::top("navbar")
        .exact_size(46.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(pal.layer).inner_margin(Margin {
            left: 8,
            right: 10,
            top: 6,
            bottom: 4,
        }))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let ctx = f.state.command_context();
                let size = vec2(36.0, 34.0);
                for (cmd, g) in [
                    (CommandId::Back, glyph::BACK),
                    (CommandId::Forward, glyph::FORWARD),
                    (CommandId::Up, glyph::UP),
                    (CommandId::Refresh, glyph::REFRESH),
                ] {
                    let tip = format!("{} ({})", cmd.label(), f.shortcut(cmd));
                    let r = widgets::icon_button(ui, &pal, g, &tip, cmd.is_enabled(&ctx), size);
                    if r.clicked() {
                        f.cmd(cmd);
                    }
                    if cmd == CommandId::Back {
                        history_menu(&r, f);
                    }
                }
                ui.add_space(6.0);
                // Rechercher, palette de commandes, analyse de l'espace et centre d'état :
                // boutons ouvrant chacun leur fenêtre.
                let omni_w = ui.available_width() - 4.0 * size.x - 10.0;
                omnibar(ui, f, omni_w);
                ui.add_space(6.0);
                for (cmd, open) in [
                    (CommandId::Search, f.state.show_search),
                    (CommandId::CommandPalette, f.state.show_palette),
                ] {
                    let tip = format!("{} ({})", cmd.label(), f.shortcut(cmd));
                    let r = widgets::icon_button(
                        ui,
                        &pal,
                        cmd.glyph(),
                        &tip,
                        cmd.is_enabled(&ctx),
                        size,
                    );
                    if open {
                        ui.painter()
                            .rect_filled(r.rect.shrink(1.0), 6.0, pal.selection);
                        ui.painter().text(
                            r.rect.center(),
                            Align2::CENTER_CENTER,
                            cmd.glyph(),
                            icon_font(14.0),
                            pal.accent,
                        );
                    }
                    if r.clicked() {
                        f.cmd(cmd);
                    }
                }
                super::diskpicker::button(ui, f, size);
                super::statuscenter::button(ui, f, size);
            });
        });
}

/// Clic droit sur « Précédent » : historique récent.
fn history_menu(r: &egui::Response, f: &mut Fx<'_>) {
    let entries: Vec<Location> = f
        .state
        .pane()
        .history
        .back_entries()
        .take(12)
        .cloned()
        .collect();
    if entries.is_empty() {
        return;
    }
    r.context_menu(|ui| {
        for loc in entries {
            let label = match &loc {
                Location::Dir(p) => p.display().to_string(),
                other => other.title(),
            };
            if ui.button(label).clicked() {
                f.send(Msg::Navigate(loc));
                ui.close();
            }
        }
    });
}

fn omnibar(ui: &mut Ui, f: &mut Fx<'_>, width: f32) {
    let pal = f.pal;
    let (rect, bg) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
    let editing = f.vs.path_edit.is_some();
    // Extrémités en demi-cercle (pilule).
    let radius = rect.height() / 2.0;
    // Halo de saisie : apparaît en 200 ms (courbe douce), s'élargit puis se pose ; un fin
    // contour d'accent le borde.
    let focus_t = ui
        .ctx()
        .animate_bool_with_time(bg.id.with("f"), editing, 0.2);
    let e = widgets::ease_out(focus_t);
    if e > 0.0 {
        widgets::glow(
            ui,
            rect,
            radius,
            pal.accent.gamma_multiply(0.38 * e),
            4.0 + 12.0 * e,
        );
    }
    ui.painter()
        .rect_filled(rect, radius, if editing { pal.card } else { pal.layer });
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.5, pal.stroke_strong),
        egui::StrokeKind::Inside,
    );
    if e > 0.0 {
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(2.0, pal.accent.gamma_multiply(e)),
            egui::StrokeKind::Inside,
        );
    }
    if editing {
        edit_path(ui, f, rect);
        return;
    }
    if bg.clicked() {
        f.vs.path_edit = Some(match f.state.pane().location() {
            Location::Dir(p) => p.display().to_string(),
            Location::Search { root, .. } => root.display().to_string(),
            _ => String::new(),
        });
    }
    let inner = rect.shrink2(vec2(12.0, 0.0));
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    ui.set_clip_rect(inner);
    ui.spacing_mut().item_spacing.x = 0.0;
    match f.state.pane().location().clone() {
        Location::Home => crumb_label(&mut ui, f, glyph::HOME, "Accueil"),
        Location::Tag(t) => {
            let name = f
                .state
                .tags
                .get(t)
                .map_or_else(|| "Étiquette".into(), |t| t.name.clone());
            crumb_label(&mut ui, f, glyph::TAG, &name);
        }
        Location::Search { root, query } => {
            crumbs(&mut ui, f, &root, inner.width() - 200.0);
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!("›  Résultats pour « {query} »")).color(pal.text2),
            );
        }
        Location::Dir(p) | Location::DiskUsage { dir: p, .. } => {
            crumbs(&mut ui, f, &p, inner.width());
        }
        Location::RecycleBin => crumb_shell(&mut ui, f, &Location::RecycleBin, glyph::RECYCLE),
        Location::Network(None) => {
            crumb_shell(&mut ui, f, &Location::Network(None), glyph::NETWORK);
        }
        Location::Network(Some(server)) => {
            let net = Location::Network(None);
            shell_slot(&mut ui, f, &net, glyph::NETWORK);
            ui.add_space(4.0);
            if segment_button(&mut ui, f, "Réseau", false).clicked() {
                f.send(Msg::Navigate(net));
            }
            chevron(&mut ui, f);
            segment_button(&mut ui, f, server.trim_start_matches('\\'), true);
        }
    }
}

/// Icône shell d'un emplacement (Corbeille, Réseau…) dans le fil d'Ariane.
fn shell_slot(ui: &mut Ui, f: &mut Fx<'_>, loc: &Location, fallback: &str) {
    let (slot, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    if let Some(src) = loc.icon_source() {
        super::items::shell_icon(ui, f, &src, slot.center(), 16.0, fallback, f.pal.text2);
    }
}

/// Fil d'Ariane d'un emplacement sans chemin : son icône Windows et son nom.
fn crumb_shell(ui: &mut Ui, f: &mut Fx<'_>, loc: &Location, fallback: &str) {
    shell_slot(ui, f, loc, fallback);
    ui.add_space(8.0);
    ui.label(egui::RichText::new(loc.title()).color(f.pal.text));
}

fn crumb_label(ui: &mut Ui, f: &Fx<'_>, g: &str, text: &str) {
    ui.label(
        egui::RichText::new(g)
            .font(icon_font(14.0))
            .color(f.pal.text2),
    );
    ui.add_space(8.0);
    ui.label(egui::RichText::new(text).color(f.pal.text));
}

/// Segments du fil d'Ariane : (libellé, chemin).
fn segments(path: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut acc = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Prefix(p) => {
                acc.push(p.as_os_str());
                acc.push("\\");
                out.push((p.as_os_str().to_string_lossy().into_owned(), acc.clone()));
            }
            Component::RootDir => {}
            Component::Normal(n) => {
                acc.push(n);
                out.push((n.to_string_lossy().into_owned(), acc.clone()));
            }
            _ => {}
        }
    }
    out
}

fn crumbs(ui: &mut Ui, f: &mut Fx<'_>, path: &Path, max_w: f32) {
    let pal = f.pal;
    let segs = segments(path);
    // Repli des premiers segments si le chemin est trop long.
    let font = egui::FontId::proportional(13.5);
    let widths: Vec<f32> = segs
        .iter()
        .map(|(s, _)| {
            ui.painter()
                .layout_no_wrap(s.clone(), font.clone(), pal.text)
                .size()
                .x
                + 34.0
        })
        .collect();
    let mut start = 0;
    while start + 1 < segs.len() && widths[start..].iter().sum::<f32>() + 30.0 > max_w {
        start += 1;
    }
    let icon = if path.parent().is_none() {
        glyph::DRIVE
    } else {
        glyph::FOLDER
    };
    // Icône du dossier courant telle que Windows la définit (dossiers connus, lecteurs…).
    let (slot, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    let color = if icon == glyph::FOLDER {
        pal.folder
    } else {
        pal.text2
    };
    super::items::shell_icon(ui, f, path, slot.center(), 16.0, icon, color);
    ui.add_space(4.0);
    if start > 0 {
        let r = segment_button(ui, f, "…", false);
        r.context_menu(|ui| {
            for (name, p) in &segs[..start] {
                if ui.button(name).clicked() {
                    f.send(Msg::Navigate(Location::Dir(p.clone())));
                    ui.close();
                }
            }
        });
        if r.clicked() {
            f.send(Msg::Navigate(Location::Dir(segs[start - 1].1.clone())));
        }
        chevron(ui, f);
    }
    let last = segs.len().saturating_sub(1);
    for (i, (name, p)) in segs.iter().enumerate().skip(start) {
        f.vs.fallbacks.scan(name);
        let r = segment_button(ui, f, name, i == last);
        if r.clicked() {
            f.send(Msg::Navigate(Location::Dir(p.clone())));
        }
        if r.middle_clicked() {
            f.send(Msg::OpenInNewTab(Location::Dir(p.clone())));
        }
        // Dépôt sur un segment : copie ou déplacement vers ce dossier.
        super::dnd::zone(f, r.rect, super::dnd::Target::Dir(p.clone()));
        r.context_menu(|ui| {
            if ui.button("Ouvrir dans un nouvel onglet").clicked() {
                f.send(Msg::OpenInNewTab(Location::Dir(p.clone())));
                ui.close();
            }
            if ui.button("Copier le chemin").clicked() {
                ui.ctx().copy_text(p.display().to_string());
                ui.close();
            }
            if ui.button("Épingler à l'accès rapide").clicked() {
                f.send(Msg::Pin(p.clone()));
                ui.close();
            }
        });
        if i != last {
            chevron(ui, f);
        }
    }
}

fn segment_button(ui: &mut Ui, f: &Fx<'_>, text: &str, last: bool) -> egui::Response {
    let pal = f.pal;
    let font = egui::FontId::proportional(13.5);
    let g = ui.painter().layout_no_wrap(
        text.to_owned(),
        font,
        if last { pal.text } else { pal.text2 },
    );
    let (rect, r) = ui.allocate_exact_size(vec2(g.size().x + 14.0, 28.0), Sense::click());
    widgets::hover_fill(ui, &r, rect, &pal, 4.0);
    ui.painter().galley(
        pos2(rect.left() + 7.0, rect.center().y - g.size().y / 2.0),
        g,
        pal.text,
    );
    r
}

fn chevron(ui: &mut Ui, f: &Fx<'_>) {
    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 28.0), Sense::hover());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph::CHEVRON_RIGHT,
        icon_font(9.0),
        f.pal.text3,
    );
}

fn edit_path(ui: &mut Ui, f: &mut Fx<'_>, rect: Rect) {
    let Some(mut text) = f.vs.path_edit.take() else {
        return;
    };
    let id = Id::unique("omnibar-edit");
    // Champ exactement à la hauteur d'une ligne, centré dans la barre : egui place le texte en
    // haut de la zone qu'on lui donne, il faut donc la dimensionner nous-mêmes.
    let font = egui::FontId::proportional(13.5);
    let line_h = ui
        .painter()
        .layout_no_wrap("Ag".to_owned(), font.clone(), f.pal.text)
        .size()
        .y;
    let inner = Rect::from_min_max(
        pos2(rect.left() + 18.0, rect.center().y - line_h / 2.0),
        pos2(rect.right() - 18.0, rect.center().y + line_h / 2.0),
    );
    let resp = ui.put(
        inner,
        TextEdit::singleline(&mut text)
            .id(id)
            .font(font)
            .text_color(f.pal.text)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .align(egui::Align2::LEFT_TOP)
            .hint_text("Saisissez un chemin, ex. C:\\Users")
            .desired_width(inner.width())
            .min_size(inner.size()),
    );
    if !resp.has_focus() && !resp.lost_focus() {
        resp.request_focus();
        // Tout sélectionner à l'ouverture, comme l'Explorateur.
        if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            )));
            st.store(ui.ctx(), id);
        }
    }
    if resp.lost_focus() {
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            let t = text.trim().trim_matches('"').to_owned();
            if !t.is_empty() {
                let p = if t.len() == 2 && t.ends_with(':') {
                    PathBuf::from(format!("{t}\\"))
                } else {
                    expand(&t)
                };
                f.send(Msg::OpenPath(p));
            }
        }
        return; // fin de l'édition (Entrée, Échap ou clic ailleurs)
    }
    f.vs.path_edit = Some(text);
}

/// Variables d'environnement (`%USERPROFILE%`) et `~`.
fn expand(t: &str) -> PathBuf {
    let mut s = t.to_owned();
    if let Some(rest) = s.strip_prefix('~')
        && let Some(home) = std::env::var_os("USERPROFILE")
    {
        s = format!("{}{rest}", home.to_string_lossy());
    }
    while let Some(start) = s.find('%') {
        let Some(len) = s[start + 1..].find('%') else {
            break;
        };
        let name = &s[start + 1..start + 1 + len];
        let Some(val) = std::env::var_os(name) else {
            break;
        };
        s = format!(
            "{}{}{}",
            &s[..start],
            val.to_string_lossy(),
            &s[start + 2 + len..]
        );
    }
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_of_paths() {
        let s = segments(Path::new(r"C:\Users\moi"));
        assert_eq!(
            s.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["C:", "Users", "moi"]
        );
        assert_eq!(s[0].1, PathBuf::from(r"C:\"));
        assert_eq!(s[2].1, PathBuf::from(r"C:\Users\moi"));
    }

    #[test]
    fn expands_env_vars() {
        let home = std::env::var("USERPROFILE").expect("USERPROFILE");
        assert_eq!(
            expand("%USERPROFILE%\\x"),
            PathBuf::from(format!("{home}\\x"))
        );
        assert_eq!(expand("~\\y"), PathBuf::from(format!("{home}\\y")));
    }
}
