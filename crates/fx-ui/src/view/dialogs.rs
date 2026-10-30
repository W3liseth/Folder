//! Boîtes de dialogue modales : suppression, corbeille, renommage en masse, erreur, à propos.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Id, Key, Layout, Modal, RichText, Ui, vec2};
use fx_app::Msg;
use fx_app::state::Dialog;

use super::Fx;
use crate::theme::{glyph, icon_font, semibold};

/// État du renommage en masse (saisie en cours).
pub struct BulkState {
    pub pattern_mode: bool,
    pub find: String,
    pub replace: String,
    pub pattern: String,
    pub start: u32,
    pub ignore_case: bool,
}

impl Default for BulkState {
    fn default() -> Self {
        Self {
            pattern_mode: false,
            find: String::new(),
            replace: String::new(),
            pattern: "{nom} ({n})".into(),
            start: 1,
            ignore_case: true,
        }
    }
}

pub fn draw(ctx: &egui::Context, f: &mut Fx<'_>) {
    let Some(dialog) = f.state.dialog.clone() else {
        return;
    };
    let modal = Modal::new(Id::unique("dialog"))
        .backdrop_color(f.pal.scrim)
        .show(ctx, |ui| {
            ui.set_width(match dialog {
                Dialog::BulkRename { .. } => 640.0,
                _ => 420.0,
            });
            match &dialog {
                Dialog::ConfirmDelete { items, permanent } => {
                    confirm_delete(ui, f, items, *permanent)
                }
                Dialog::ConfirmEmptyRecycleBin => confirm(
                    ui,
                    f,
                    glyph::RECYCLE,
                    "Vider la corbeille ?",
                    "Tous les éléments de la corbeille seront supprimés définitivement.",
                    "Vider",
                ),
                Dialog::BulkRename { items } => bulk_rename(ui, f, items),
                Dialog::ConfirmClean {
                    title,
                    items,
                    bytes,
                    permanent,
                } => confirm_clean(ui, f, title, items, *bytes, *permanent),
                Dialog::Error { title, message } => {
                    header(ui, f, glyph::ERROR, title);
                    ui.label(message);
                    ui.add_space(12.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("  OK  ").clicked() {
                            f.send(Msg::CancelDialog);
                        }
                    });
                }
                Dialog::About => about(ui, f),
                Dialog::ConfirmPurge { items } => {
                    let title = match items.as_slice() {
                        [one] => format!("Supprimer définitivement « {} » ?", name(&one.original)),
                        many => format!("Supprimer définitivement ces {} éléments ?", many.len()),
                    };
                    confirm(
                        ui,
                        f,
                        glyph::DELETE,
                        &title,
                        "Ils seront retirés de la corbeille : cette action est définitive.",
                        "Supprimer",
                    );
                }
            }
        });
    if modal.should_close() {
        f.send(Msg::CancelDialog);
    }
}

fn header(ui: &mut Ui, f: &Fx<'_>, g: &str, title: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(g).font(icon_font(20.0)).color(f.pal.accent));
        ui.add_space(4.0);
        ui.label(RichText::new(title).font(semibold(18.0)));
    });
    ui.add_space(10.0);
}

/// Bouton principal (couleur d'accent ou de danger).
fn primary(ui: &mut Ui, f: &Fx<'_>, label: &str, danger: bool) -> egui::Response {
    let fill = if danger { f.pal.danger } else { f.pal.accent };
    ui.add(
        egui::Button::new(RichText::new(format!("  {label}  ")).color(egui::Color32::WHITE))
            .fill(fill)
            .min_size(vec2(100.0, 32.0)),
    )
}

fn confirm(ui: &mut Ui, f: &mut Fx<'_>, g: &str, title: &str, text: &str, action: &str) {
    header(ui, f, g, title);
    ui.label(RichText::new(text).color(f.pal.text2));
    ui.add_space(16.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if ui
            .add(egui::Button::new("Annuler").min_size(vec2(100.0, 32.0)))
            .clicked()
        {
            f.send(Msg::CancelDialog);
        }
        let r = primary(ui, f, action, true);
        if r.clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
            f.send(Msg::ConfirmDialog);
        }
    });
}

/// Nettoyage proposé par l'analyse de l'espace : place libérée et éléments concernés.
fn confirm_clean(
    ui: &mut Ui,
    f: &mut Fx<'_>,
    title: &str,
    items: &[PathBuf],
    bytes: u64,
    permanent: bool,
) {
    let mut size = String::new();
    let _ = fx_core::format::size_human(&mut size, bytes);
    header(ui, f, glyph::BROOM, &format!("Nettoyer « {title} » ?"));
    let what = match items.len() {
        1 => "1 élément".to_owned(),
        n => format!("{n} éléments"),
    };
    let text = if permanent {
        format!(
            "{what}, environ {size}. Suppression définitive : ils ne passeront pas par la \
             corbeille. Les éléments en cours d'utilisation seront ignorés."
        )
    } else {
        format!(
            "{what}, environ {size}, mis à la corbeille : la place n'est libérée qu'une fois \
             la corbeille vidée."
        )
    };
    ui.label(RichText::new(text).color(f.pal.text2));
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .max_height(140.0)
        .show(ui, |ui| {
            for p in items.iter().take(100) {
                ui.label(
                    RichText::new(p.display().to_string())
                        .size(12.0)
                        .color(f.pal.text2),
                );
            }
            if items.len() > 100 {
                ui.label(
                    RichText::new(format!("… et {} autres", items.len() - 100))
                        .size(12.0)
                        .color(f.pal.text3),
                );
            }
        });
    ui.add_space(14.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if ui
            .add(egui::Button::new("Annuler").min_size(vec2(100.0, 32.0)))
            .clicked()
        {
            f.send(Msg::CancelDialog);
        }
        let action = if permanent {
            "Supprimer"
        } else {
            "Mettre à la corbeille"
        };
        if primary(ui, f, action, true).clicked() {
            f.send(Msg::ConfirmDialog);
        }
    });
}

fn confirm_delete(ui: &mut Ui, f: &mut Fx<'_>, items: &[PathBuf], permanent: bool) {
    let what = match items {
        [one] => format!("« {} »", name(one)),
        many => format!("ces {} éléments", many.len()),
    };
    let title = if permanent {
        format!("Supprimer définitivement {what} ?")
    } else {
        format!("Supprimer {what} ?")
    };
    let text = if permanent {
        "Cette action est définitive : les éléments ne passeront pas par la corbeille."
    } else if matches!(
        f.state.pane().location(),
        fx_app::Location::DiskUsage { .. }
    ) {
        // Dans l'analyse de l'espace, on supprime pour faire de la place : la corbeille la
        // garde occupée.
        "Les éléments seront placés dans la corbeille : la place n'est libérée qu'une fois \
         la corbeille vidée (Maj+Suppr supprime définitivement)."
    } else {
        "Les éléments seront placés dans la corbeille."
    };
    header(ui, f, glyph::DELETE, &title);
    ui.label(RichText::new(text).color(f.pal.text2));
    if items.len() > 1 {
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .max_height(120.0)
            .show(ui, |ui| {
                for p in items.iter().take(50) {
                    ui.label(RichText::new(name(p)).size(12.5).color(f.pal.text2));
                }
            });
    }
    if !permanent {
        ui.add_space(8.0);
        ui.checkbox(
            &mut f.vs.dont_ask_delete,
            "Ne plus demander de confirmation",
        );
    }
    ui.add_space(14.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if ui
            .add(egui::Button::new("Annuler").min_size(vec2(100.0, 32.0)))
            .clicked()
        {
            f.send(Msg::CancelDialog);
        }
        let r = primary(ui, f, "Supprimer", true);
        if r.clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
            if f.vs.dont_ask_delete && !permanent {
                f.send(super::settings_with(f.state, |s| s.confirm_delete = false));
                f.vs.dont_ask_delete = false;
            }
            f.send(Msg::ConfirmDialog);
        }
    });
}

fn name(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Nouveau nom d'un élément selon les réglages du renommage en masse.
fn new_name(b: &BulkState, original: &str, index: usize, width: usize) -> String {
    if b.pattern_mode {
        let p = Path::new(original);
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        let n = format!("{:0width$}", b.start as usize + index);
        let mut out = b
            .pattern
            .replace("{nom}", &stem)
            .replace("{n}", &n)
            .replace("{ext}", &ext);
        if !b.pattern.contains("{ext}") && !ext.is_empty() {
            out = format!("{out}.{ext}");
        }
        out
    } else if b.find.is_empty() {
        original.to_owned()
    } else if b.ignore_case {
        replace_ci(original, &b.find, &b.replace)
    } else {
        original.replace(&b.find, &b.replace)
    }
}

fn replace_ci(s: &str, find: &str, rep: &str) -> String {
    let lower = s.to_lowercase();
    let f = find.to_lowercase();
    if lower.len() != s.len() {
        return s.replace(find, rep); // casse non triviale : remplacement exact
    }
    let mut out = String::new();
    let mut i = 0;
    while let Some(pos) = lower[i..].find(&f) {
        out.push_str(&s[i..i + pos]);
        out.push_str(rep);
        i += pos + f.len();
    }
    out.push_str(&s[i..]);
    out
}

fn bulk_rename(ui: &mut Ui, f: &mut Fx<'_>, items: &[PathBuf]) {
    header(
        ui,
        f,
        glyph::RENAME,
        &format!("Renommer {} éléments", items.len()),
    );
    let b = &mut f.vs.bulk;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut b.pattern_mode, false, "Rechercher et remplacer");
        ui.selectable_value(&mut b.pattern_mode, true, "Motif");
    });
    ui.add_space(8.0);
    egui::Grid::new("bulk")
        .num_columns(2)
        .spacing(vec2(10.0, 8.0))
        .show(ui, |ui| {
            if b.pattern_mode {
                ui.label("Motif");
                ui.add(egui::TextEdit::singleline(&mut b.pattern).desired_width(400.0));
                ui.end_row();
                ui.label("Premier numéro");
                ui.add(egui::DragValue::new(&mut b.start).range(0..=999_999));
                ui.end_row();
                ui.label("");
                ui.label(
                    RichText::new("{nom} : nom d'origine · {n} : numéro · {ext} : extension")
                        .size(12.0)
                        .color(f.pal.text3),
                );
                ui.end_row();
            } else {
                ui.label("Rechercher");
                ui.add(egui::TextEdit::singleline(&mut b.find).desired_width(400.0));
                ui.end_row();
                ui.label("Remplacer par");
                ui.add(egui::TextEdit::singleline(&mut b.replace).desired_width(400.0));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut b.ignore_case, "Ignorer la casse");
                ui.end_row();
            }
        });
    ui.add_space(10.0);
    let width = (items.len() + f.vs.bulk.start as usize).to_string().len();
    let preview: Vec<(PathBuf, String, String)> = items
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let old = name(p);
            let new = new_name(&f.vs.bulk, &old, i, width);
            (p.clone(), old, new)
        })
        .collect();
    egui::Frame::new()
        .fill(f.pal.layer)
        .corner_radius(4)
        .inner_margin(8)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    egui::Grid::new("bulk-preview")
                        .num_columns(3)
                        .striped(true)
                        .spacing(vec2(12.0, 4.0))
                        .show(ui, |ui| {
                            for (_, old, new) in preview.iter().take(300) {
                                ui.label(RichText::new(old).color(f.pal.text2));
                                ui.label(RichText::new("→").color(f.pal.text3));
                                let changed = old != new;
                                ui.label(if changed {
                                    RichText::new(new).color(f.pal.accent)
                                } else {
                                    RichText::new(new).color(f.pal.text3)
                                });
                                ui.end_row();
                            }
                        });
                });
        });
    let changes = preview.iter().filter(|(_, o, n)| o != n).count();
    ui.add_space(12.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if ui
            .add(egui::Button::new("Annuler").min_size(vec2(100.0, 32.0)))
            .clicked()
        {
            f.send(Msg::CancelDialog);
        }
        let r = ui.add_enabled(
            changes > 0,
            egui::Button::new(
                RichText::new(format!("  Renommer ({changes})  ")).color(egui::Color32::WHITE),
            )
            .fill(f.pal.accent)
            .min_size(vec2(120.0, 32.0)),
        );
        if r.clicked() {
            let items = preview
                .into_iter()
                .filter(|(_, o, n)| o != n)
                .map(|(p, _, n)| (p, n))
                .collect();
            f.send(Msg::BulkRenameApply(items));
        }
    });
}

fn about(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(glyph::FOLDER)
                .font(icon_font(48.0))
                .color(pal.folder),
        );
        ui.label(RichText::new("folder").font(semibold(26.0)));
        ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(pal.text2));
        ui.add_space(8.0);
        ui.label("Gestionnaire de fichiers natif pour Windows, écrit en Rust.");
        ui.label(
            RichText::new("Interface inspirée de Files et de l'Explorateur de Windows 11.")
                .color(pal.text2),
        );
        ui.add_space(8.0);
        if let Ok(m) = fx_win::process::current_memory() {
            ui.label(
                RichText::new(format!(
                    "Mémoire privée : {:.1} Mo",
                    m.private_bytes as f64 / 1_048_576.0
                ))
                .size(12.0)
                .color(pal.text3),
            );
        }
        ui.add_space(10.0);
        if ui.button("  Fermer  ").clicked() {
            f.send(Msg::CancelDialog);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_names() {
        let mut b = BulkState {
            find: "IMG".into(),
            replace: "Photo".into(),
            ..BulkState::default()
        };
        assert_eq!(new_name(&b, "img_001.jpg", 0, 1), "Photo_001.jpg");
        b.pattern_mode = true;
        b.pattern = "Vacances {n}".into();
        assert_eq!(new_name(&b, "x.jpg", 4, 3), "Vacances 005.jpg");
        b.pattern = "{nom}-copie.{ext}".into();
        assert_eq!(new_name(&b, "a.txt", 0, 1), "a-copie.txt");
    }
}
