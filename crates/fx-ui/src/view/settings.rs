//! Page Paramètres (dans sa propre fenêtre, cf. `settings_window`) : général, apparence,
//! dossiers, étiquettes, raccourcis, mises à jour (et notes de version), à propos. Chaque
//! modification est envoyée à la fenêtre principale, qui l'applique et l'enregistre
//! aussitôt.

use eframe::egui::{
    self, Align, Align2, Color32, Frame, Id, Layout, RichText, ScrollArea, Sense, Ui, pos2, vec2,
};
use fx_app::Msg;
use fx_app::upgrade::{Release, Status};
use fx_commands::{CommandId, Group};
use fx_store::{Layout as View, Settings, Tag, ThemeChoice};

use super::Fx;
use crate::theme::{glyph, icon_font, semibold};
use crate::widgets;

const SECTIONS: [(&str, &str); 7] = [
    (glyph::SETTINGS, "Général"),
    (glyph::PALETTE, "Apparence"),
    (glyph::FOLDER, "Dossiers"),
    (glyph::TAG, "Étiquettes"),
    (glyph::KEYBOARD, "Raccourcis"),
    (glyph::SYNC, "Mises à jour"),
    (glyph::INFO, "À propos"),
];

/// Section des mises à jour (et des notes de version).
pub(crate) const UPDATES: usize = 5;

/// Page Paramètres dans la fenêtre courante (fenêtre Paramètres à part entière) :
/// navigation à gauche, section choisie à droite.
pub(crate) fn page(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    egui::Panel::left("settings-nav")
        .exact_size(230.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(pal.base)
                .inner_margin(egui::Margin::symmetric(12, 16)),
        )
        .show(ui, |ui| {
            ui.label(RichText::new("Paramètres").font(semibold(22.0)));
            ui.add_space(16.0);
            for (i, (g, label)) in SECTIONS.iter().enumerate() {
                let (rect, r) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                let active = f.vs.settings_section == i;
                let t = ui
                    .ctx()
                    .animate_bool_with_time(r.id.with("a"), active, 0.15);
                if t > 0.0 {
                    ui.painter()
                        .rect_filled(rect, 6.0, pal.hover.gamma_multiply(1.6 * t));
                    ui.painter().rect_filled(
                        egui::Rect::from_center_size(
                            pos2(rect.left() + 2.0, rect.center().y),
                            vec2(3.0, 16.0 * t),
                        ),
                        1.5,
                        pal.accent,
                    );
                } else {
                    widgets::hover_fill(ui, &r, rect, &pal, 6.0);
                }
                ui.painter().text(
                    pos2(rect.left() + 22.0, rect.center().y),
                    Align2::CENTER_CENTER,
                    *g,
                    icon_font(15.0),
                    if active { pal.accent } else { pal.text2 },
                );
                ui.painter().text(
                    pos2(rect.left() + 44.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    *label,
                    egui::FontId::proportional(14.0),
                    pal.text,
                );
                if r.clicked() {
                    f.vs.settings_section = i;
                    f.vs.settings_notes = false;
                }
            }
        });
    egui::CentralPanel::default()
        .frame(
            Frame::new()
                .fill(pal.layer)
                .inner_margin(egui::Margin::symmetric(28, 20)),
        )
        .show(ui, |ui| {
            let title = if f.vs.settings_section == UPDATES && f.vs.settings_notes {
                "Notes de version"
            } else {
                SECTIONS[f.vs.settings_section].1
            };
            ui.label(RichText::new(title).font(semibold(26.0)));
            ui.add_space(14.0);
            ScrollArea::vertical()
                .id_salt("settings-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width().min(760.0));
                    let mut s = f.state.settings.clone();
                    let before = s.clone();
                    match f.vs.settings_section {
                        0 => general(ui, f, &mut s),
                        1 => appearance(ui, f, &mut s),
                        2 => folders(ui, f, &mut s),
                        3 => tags(ui, f, &mut s),
                        4 => shortcuts(ui, f, &mut s),
                        UPDATES if f.vs.settings_notes => release_notes(ui, f),
                        UPDATES => updates(ui, f, &mut s),
                        _ => about(ui, f),
                    }
                    if s != before {
                        f.send(Msg::UpdateSettings(Box::new(s)));
                    }
                });
        });
}

/// Ligne de réglage façon Windows 11 : titre, description, contrôle à droite.
fn setting(ui: &mut Ui, f: &Fx<'_>, title: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
    let pal = f.pal;
    Frame::new()
        .fill(pal.layer)
        .stroke(egui::Stroke::new(1.0, pal.stroke))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(title);
                    if !desc.is_empty() {
                        ui.label(RichText::new(desc).size(12.0).color(pal.text2));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), control);
            });
        });
    ui.add_space(4.0);
}

fn toggle(ui: &mut Ui, on: &mut bool) {
    let (rect, r) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::click());
    if r.clicked() {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool_with_time(r.id, *on, 0.12);
    let v = ui.visuals();
    let accent = v.hyperlink_color;
    let bg = if *on { accent } else { Color32::TRANSPARENT };
    ui.painter()
        .rect_filled(rect, 10.0, bg.gamma_multiply(t.max(0.0)));
    ui.painter().rect_stroke(
        rect,
        10.0,
        egui::Stroke::new(
            1.0,
            if *on {
                accent
            } else {
                v.widgets.noninteractive.fg_stroke.color
            },
        ),
        egui::StrokeKind::Inside,
    );
    let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, t);
    let knob = if *on {
        Color32::WHITE
    } else {
        v.widgets.noninteractive.fg_stroke.color
    };
    ui.painter().circle_filled(
        pos2(x, rect.center().y),
        if r.hovered() { 6.0 } else { 5.0 },
        knob,
    );
}

fn general(ui: &mut Ui, f: &mut Fx<'_>, s: &mut Settings) {
    setting(
        ui,
        f,
        "Restaurer les onglets au démarrage",
        "Rouvre les onglets de la dernière session.",
        |ui| toggle(ui, &mut s.restore_session),
    );
    setting(
        ui,
        f,
        "Confirmer les suppressions",
        "Demande confirmation avant d'envoyer à la corbeille.",
        |ui| toggle(ui, &mut s.confirm_delete),
    );
    setting(
        ui,
        f,
        "Ouvrir d'un simple clic",
        "Ouvre fichiers et dossiers d'un clic, comme un lien.",
        |ui| toggle(ui, &mut s.single_click),
    );
}

fn appearance(ui: &mut Ui, f: &mut Fx<'_>, s: &mut Settings) {
    let pal = f.pal;
    setting(
        ui,
        f,
        "Thème",
        "Clair, sombre ou selon le réglage de Windows.",
        |ui| {
            egui::ComboBox::from_id_salt("theme")
                .selected_text(match s.theme {
                    ThemeChoice::System => "Système",
                    ThemeChoice::Light => "Clair",
                    ThemeChoice::Dark => "Sombre",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.theme, ThemeChoice::System, "Système");
                    ui.selectable_value(&mut s.theme, ThemeChoice::Light, "Clair");
                    ui.selectable_value(&mut s.theme, ThemeChoice::Dark, "Sombre");
                });
        },
    );
    accent_setting(ui, f, s);
    setting(ui, f, "Barre latérale", "", |ui| {
        toggle(ui, &mut s.show_sidebar)
    });
    setting(
        ui,
        f,
        "Épaisseur des jauges des lecteurs",
        "Espace utilisé, sous chaque lecteur de la barre latérale.",
        |ui| {
            widgets::slider(ui, &pal, &mut s.drive_bar, 2.0..=10.0, 1.0, " px");
        },
    );
    setting(ui, f, "Volet d'aperçu", "", |ui| {
        toggle(ui, &mut s.show_preview)
    });
}

/// Couleur d'accentuation : celle de Windows, une couleur proposée ou une couleur libre.
fn accent_setting(ui: &mut Ui, f: &mut Fx<'_>, s: &mut Settings) {
    let pal = f.pal;
    Frame::new()
        .fill(pal.layer)
        .stroke(egui::Stroke::new(1.0, pal.stroke))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label("Couleur d'accentuation");
                    ui.label(
                        RichText::new("Boutons, sélection, indicateurs.")
                            .size(12.0)
                            .color(pal.text2),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut system = s.accent.is_none();
                    if ui.checkbox(&mut system, "Couleur de Windows").changed() {
                        s.accent = if system {
                            None
                        } else {
                            let c = f.vs.system_accent;
                            Some([c.r(), c.g(), c.b()])
                        };
                    }
                });
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for c in crate::theme::ACCENTS {
                    let (rect, r) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
                    let color = egui::Color32::from_rgb(c[0], c[1], c[2]);
                    ui.painter().rect_filled(rect.shrink(3.0), 6.0, color);
                    if s.accent == Some(c) {
                        ui.painter().rect_stroke(
                            rect,
                            8.0,
                            egui::Stroke::new(2.0, pal.text),
                            egui::StrokeKind::Inside,
                        );
                    } else if r.hovered() {
                        ui.painter().rect_stroke(
                            rect,
                            8.0,
                            egui::Stroke::new(1.0, pal.text2),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if r.clicked() {
                        s.accent = Some(c);
                    }
                }
            });
            // Couleur libre : sélecteur complet (teinte, saturation et luminosité, valeurs
            // RVB / TSV / hexadécimale).
            ui.add_space(10.0);
            ui.label(
                RichText::new("Couleur personnalisée")
                    .size(12.0)
                    .color(pal.text2),
            );
            ui.add_space(4.0);
            // Couleur réglée telle quelle : `pal.accent` est éclaircie en thème sombre, le
            // sélecteur perdrait sa position à chaque image.
            let raw = crate::theme::accent_of(s, f.vs.system_accent);
            if let Some(c) = widgets::color_picker(ui, &pal, raw) {
                s.accent = Some([c.r(), c.g(), c.b()]);
            }
        });
    ui.add_space(4.0);
}

fn folders(ui: &mut Ui, f: &mut Fx<'_>, s: &mut Settings) {
    let pal = f.pal;
    setting(ui, f, "Afficher les éléments masqués", "", |ui| {
        toggle(ui, &mut s.show_hidden)
    });
    setting(ui, f, "Afficher les extensions de fichiers", "", |ui| {
        toggle(ui, &mut s.show_extensions)
    });
    setting(
        ui,
        f,
        "Dossiers en premier",
        "Les dossiers précèdent les fichiers, quel que soit le tri.",
        |ui| toggle(ui, &mut s.folders_first),
    );
    setting(
        ui,
        f,
        "Miniatures",
        "Affiche l'aperçu des images et vidéos en grandes icônes.",
        |ui| toggle(ui, &mut s.show_thumbnails),
    );
    setting(
        ui,
        f,
        "Disposition par défaut",
        "Pour les dossiers sans préférence mémorisée.",
        |ui| {
            egui::ComboBox::from_id_salt("layout")
                .selected_text(s.default_layout.label())
                .show_ui(ui, |ui| {
                    for l in [View::Details, View::List, View::Grid, View::Columns] {
                        ui.selectable_value(&mut s.default_layout, l, l.label());
                    }
                });
        },
    );
    setting(ui, f, "Taille des grandes icônes", "", |ui| {
        let mut size = f32::from(s.grid_size);
        if widgets::slider(ui, &pal, &mut size, 48.0..=256.0, 8.0, " px").changed() {
            s.grid_size = size as u16;
        }
    });
}

fn tags(ui: &mut Ui, f: &mut Fx<'_>, s: &mut Settings) {
    let pal = f.pal;
    setting(
        ui,
        f,
        "Activer les étiquettes",
        "Section de la barre latérale et de l'accueil, menu contextuel, pastilles après les noms.",
        |ui| toggle(ui, &mut s.show_tags),
    );
    if !s.show_tags {
        return;
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new("Les étiquettes s'appliquent depuis le menu contextuel des fichiers.")
            .color(pal.text2),
    );
    ui.add_space(8.0);
    // Une carte par étiquette, sur deux lignes : nom (et suppression), puis couleurs. Les
    // couleurs passent à la ligne si la fenêtre est étroite : rien ne se chevauche.
    for t in f.state.tags.list.clone() {
        Frame::new()
            .fill(pal.layer)
            .stroke(egui::Stroke::new(1.0, pal.stroke))
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (slot, _) = ui.allocate_exact_size(vec2(24.0, 28.0), Sense::hover());
                    widgets::tag_icon(ui, slot.center(), 17.0, t.color);
                    let id = Id::unique(("tagname", t.id));
                    let mut name = ui
                        .data(|d| d.get_temp::<String>(id))
                        .unwrap_or_else(|| t.name.clone());
                    let width = (ui.available_width() - 190.0).clamp(120.0, 320.0);
                    let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(width));
                    if r.changed() {
                        ui.data_mut(|d| d.insert_temp(id, name.clone()));
                    }
                    if r.lost_focus() && !name.trim().is_empty() && name != t.name {
                        f.send(Msg::SaveTag(Tag {
                            name: name.trim().to_owned(),
                            ..t.clone()
                        }));
                        ui.data_mut(|d| d.remove::<String>(id));
                    }
                    let n = f.state.tags.count(t.id);
                    ui.label(
                        RichText::new(format!("{n} élément{}", if n > 1 { "s" } else { "" }))
                            .size(12.5)
                            .color(pal.text3),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::icon_button(
                            ui,
                            &pal,
                            glyph::DELETE,
                            "Supprimer l'étiquette",
                            true,
                            vec2(30.0, 28.0),
                        )
                        .clicked()
                        {
                            f.send(Msg::DeleteTag(t.id));
                        }
                    });
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    ui.add_space(30.0);
                    for c in widgets::TAG_COLORS {
                        let (rect, r) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                        let fill = egui::Color32::from_rgb(c[0], c[1], c[2]);
                        ui.painter().circle_filled(rect.center(), 8.0, fill);
                        if c == t.color {
                            ui.painter().circle_stroke(
                                rect.center(),
                                10.5,
                                egui::Stroke::new(2.0, pal.text),
                            );
                        } else if r.hovered() {
                            ui.painter().circle_stroke(
                                rect.center(),
                                10.5,
                                egui::Stroke::new(1.0, pal.text2),
                            );
                        }
                        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            f.send(Msg::SaveTag(Tag {
                                color: c,
                                ..t.clone()
                            }));
                        }
                    }
                    ui.add_space(4.0);
                    let mut color = t.color;
                    if ui
                        .color_edit_button_srgb(&mut color)
                        .on_hover_text("Autre couleur…")
                        .changed()
                    {
                        f.send(Msg::SaveTag(Tag { color, ..t.clone() }));
                    }
                });
            });
        ui.add_space(6.0);
    }
    ui.add_space(6.0);
    if ui.button(format!("{}  Nouvelle étiquette", '+')).clicked() {
        let n = f.state.tags.list.len();
        f.send(Msg::SaveTag(Tag {
            id: 0,
            name: format!("Étiquette {}", n + 1),
            color: widgets::TAG_COLORS[n % widgets::TAG_COLORS.len()],
        }));
    }
}

fn shortcuts(ui: &mut Ui, f: &mut Fx<'_>, s: &mut Settings) {
    system_shortcuts(ui, f, s);
    let pal = f.pal;
    // Capture de la prochaine combinaison pour la commande en cours d'édition.
    if let Some(cmd) = f.vs.capture_hotkey {
        let pressed = crate::keys::pressed(ui.ctx());
        if let Some(hk) = pressed
            .into_iter()
            .find(|h| h.key != fx_commands::Key::Escape || h.ctrl || h.alt || h.shift)
        {
            s.keybindings.insert(cmd, vec![hk]);
            f.vs.capture_hotkey = None;
        } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            f.vs.capture_hotkey = None;
        }
    }
    ui.label(
        RichText::new(
            "Cliquez sur un raccourci pour le modifier, puis appuyez sur la nouvelle combinaison.",
        )
        .color(pal.text2),
    );
    ui.add_space(8.0);
    let mut group = None;
    for &id in CommandId::ALL {
        if group != Some(id.group()) {
            group = Some(id.group());
            widgets::section(ui, &pal, group_label(id.group()));
        }
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(id.glyph())
                    .font(icon_font(13.0))
                    .color(pal.text2),
            );
            ui.label(id.label());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let custom = s.keybindings.contains_key(&id);
                if custom
                    && ui
                        .small_button("Rétablir")
                        .on_hover_text("Raccourci par défaut")
                        .clicked()
                {
                    s.keybindings.remove(&id);
                }
                let capturing = f.vs.capture_hotkey == Some(id);
                let text = if capturing {
                    "Appuyez sur une touche…".to_owned()
                } else {
                    let keys = s
                        .keybindings
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| id.default_hotkeys());
                    if keys.is_empty() {
                        "—".to_owned()
                    } else {
                        keys.iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("   ")
                    }
                };
                let b = egui::Button::new(RichText::new(text).monospace().size(12.0))
                    .min_size(vec2(160.0, 26.0));
                if ui.add(b).clicked() {
                    f.vs.capture_hotkey = Some(id);
                }
            });
        });
    }
}

/// Raccourcis qui agissent hors de folder, même fermé (cf. `fx_win::shortcuts`).
fn system_shortcuts(ui: &mut Ui, f: &mut Fx<'_>, s: &mut Settings) {
    let pal = f.pal;
    widgets::section(ui, &pal, "Système");
    setting(
        ui,
        f,
        "Windows + E ouvre folder",
        "Override le raccourci Windows + E pour ouvrir folder au lieu de l'Explorateur Windows.",
        |ui| toggle(ui, &mut s.win_e_opens_folder),
    );
    // Capture de la combinaison : Windows n'accepte qu'une lettre, un chiffre ou F1–F12 avec
    // au moins deux modificateurs.
    if f.vs.capture_open_hotkey {
        let pressed = crate::keys::pressed(ui.ctx());
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            f.vs.capture_open_hotkey = false;
            f.vs.open_hotkey_error = None;
        } else if let Some(hk) = pressed.into_iter().next() {
            if fx_app::system::launch_hotkey(&hk).is_some() {
                s.open_hotkey = Some(hk);
                f.vs.capture_open_hotkey = false;
                f.vs.open_hotkey_error = None;
            } else {
                f.vs.open_hotkey_error = Some(format!(
                    "{hk} n'est pas accepté par Windows : utilisez au moins deux touches parmi \
                     Ctrl, Alt et Maj, avec une lettre, un chiffre ou F1 à F12."
                ));
            }
        }
    }
    let active = !s.win_e_opens_folder;
    let capturing = f.vs.capture_open_hotkey;
    let desc = if active {
        "Ouvre folder de n'importe où, même fermé (par exemple Ctrl + Alt + F)."
    } else {
        "Inactif : Windows + E ouvre déjà folder."
    };
    let mut start_capture = false;
    setting(ui, f, "Raccourci pour ouvrir folder", desc, |ui| {
        ui.add_enabled_ui(active, |ui| {
            let text = if capturing {
                "Appuyez sur la combinaison…".to_owned()
            } else {
                s.open_hotkey
                    .map_or_else(|| "Aucun".to_owned(), |h| h.to_string())
            };
            let b = egui::Button::new(RichText::new(text).monospace().size(12.0))
                .min_size(vec2(160.0, 26.0));
            if ui.add(b).clicked() {
                start_capture = true;
            }
            if s.open_hotkey.is_some() && !capturing && ui.small_button("Effacer").clicked() {
                s.open_hotkey = None;
            }
        });
    });
    if start_capture {
        f.vs.capture_open_hotkey = true;
        f.vs.open_hotkey_error = None;
    }
    if let Some(err) = &f.vs.open_hotkey_error {
        ui.label(RichText::new(err).size(12.0).color(pal.danger));
        ui.add_space(4.0);
    }
    ui.add_space(8.0);
}

fn group_label(g: Group) -> &'static str {
    g.label()
}

/// État de la mise à jour, recherche à la demande, réglage automatique, notes de version.
fn updates(ui: &mut Ui, f: &mut Fx<'_>, s: &mut Settings) {
    let pal = f.pal;
    // Page ouverte : un résultat ancien est revérifié plutôt qu'affiché comme « à jour ».
    if !f.vs.update_refreshed {
        f.vs.update_refreshed = true;
        if s.auto_update && f.state.update.needs_refresh(fx_app::upgrade::PAGE_MAX_AGE) {
            f.send(Msg::CheckUpdate { force: true });
        }
    }
    let status = &f.state.update;
    let (glyph_, color, line) = match status {
        Status::Ready { version, .. } => (
            glyph::DOWNLOAD,
            pal.accent,
            format!("folder {version} est téléchargé et prêt à être installé."),
        ),
        Status::UpToDate { latest, checked } => (
            glyph::SUCCESS,
            pal.success,
            format!(
                "folder est à jour : {latest} est la dernière version publiée (vérifié {}).",
                since(*checked)
            ),
        ),
        Status::Checking => (glyph::SYNC, pal.text2, "Recherche en cours…".to_owned()),
        Status::Failed(e) => (
            glyph::WARNING,
            pal.warning,
            format!("La dernière recherche a échoué : {e}."),
        ),
        Status::NotInstalled => (
            glyph::INFO,
            pal.text2,
            "Indisponible : ce folder n'a pas été lancé depuis son installation (version de \
             développement)."
                .to_owned(),
        ),
        Status::Unknown => (
            glyph::SYNC,
            pal.text2,
            "Aucune recherche récente.".to_owned(),
        ),
    };
    Frame::new()
        .fill(pal.layer)
        .stroke(egui::Stroke::new(1.0, pal.stroke))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph_).font(icon_font(22.0)).color(color));
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(format!("folder {}", env!("CARGO_PKG_VERSION")))
                            .font(semibold(15.0)),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(line).size(12.0).color(pal.text2)).wrap(),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| match status {
                    Status::Ready { .. } => {
                        if widgets::accent_button(
                            ui,
                            &pal,
                            glyph::DOWNLOAD,
                            "Installer et redémarrer",
                            false,
                        )
                        .on_hover_text(
                            "Ferme folder, installe la mise à jour, puis rouvre vos onglets",
                        )
                        .clicked()
                        {
                            f.send(Msg::InstallUpdate);
                        }
                    }
                    Status::Checking => {
                        ui.add(egui::Spinner::new().size(18.0));
                    }
                    Status::NotInstalled => {}
                    _ => {
                        if ui
                            .add(
                                egui::Button::new("Rechercher les mises à jour")
                                    .min_size(vec2(0.0, 30.0)),
                            )
                            .clicked()
                        {
                            f.send(Msg::CheckUpdate { force: true });
                        }
                    }
                });
            });
        });
    ui.add_space(4.0);
    setting(
        ui,
        f,
        "Rechercher automatiquement les mises à jour",
        "Au plus une fois par heure, sur GitHub ; l'installation reste à votre demande.",
        |ui| toggle(ui, &mut s.auto_update),
    );
    let mut open_notes = false;
    setting(
        ui,
        f,
        "Notes de version",
        "Les changements de chaque version de folder.",
        |ui| {
            open_notes = ui
                .add(
                    egui::Button::new(widgets::menu_entry(glyph::NOTES, "Notes de version", &pal))
                        .min_size(vec2(0.0, 30.0)),
                )
                .clicked();
        },
    );
    if open_notes {
        f.vs.settings_notes = true;
    }
}

/// Changements de chaque version (CHANGELOG.md embarqué), du plus récent au plus ancien.
fn release_notes(ui: &mut Ui, f: &mut Fx<'_>) {
    static NOTES: std::sync::OnceLock<Vec<Release>> = std::sync::OnceLock::new();
    let pal = f.pal;
    if ui
        .add(egui::Button::new(widgets::menu_entry(glyph::BACK, "Mises à jour", &pal)).frame(false))
        .clicked()
    {
        f.vs.settings_notes = false;
    }
    ui.add_space(8.0);
    let current = env!("CARGO_PKG_VERSION");
    for r in NOTES.get_or_init(fx_app::upgrade::notes) {
        Frame::new()
            .fill(pal.layer)
            .stroke(egui::Stroke::new(1.0, pal.stroke))
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Version {}", r.version)).font(semibold(16.0)));
                    ui.label(RichText::new(long_date(r.date)).size(12.5).color(pal.text3));
                    if r.version == current {
                        ui.add_space(4.0);
                        Frame::new()
                            .fill(pal.accent.gamma_multiply(0.18))
                            .corner_radius(9)
                            .inner_margin(egui::Margin::symmetric(8, 2))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new("Version installée")
                                        .size(11.5)
                                        .color(pal.accent),
                                );
                            });
                    }
                });
                ui.add_space(6.0);
                for item in &r.items {
                    ui.horizontal_top(|ui| {
                        ui.add_space(4.0);
                        ui.label(RichText::new("•").color(pal.accent));
                        ui.add(egui::Label::new(item.as_str()).wrap());
                    });
                    ui.add_space(2.0);
                }
            });
        ui.add_space(8.0);
    }
}

/// « 2026-10-28 » → « 28 octobre 2026 » (date des notes de version).
fn long_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];
    let mut it = iso.split('-').map(|p| p.parse::<usize>().ok());
    match (it.next(), it.next(), it.next()) {
        (Some(Some(y)), Some(Some(m)), Some(Some(d))) if (1..=12).contains(&m) => {
            let day = if d == 1 {
                "1er".to_owned()
            } else {
                d.to_string()
            };
            format!("{day} {} {y}", MONTHS[m - 1])
        }
        _ => iso.to_owned(),
    }
}

/// Temps écoulé depuis `secs` (secondes depuis 1970), en clair.
fn since(secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    match (now - secs).max(0) {
        0..60 => "à l'instant".to_owned(),
        s @ 60..3600 => format!("il y a {} min", s / 60),
        s @ 3600..86_400 => format!("il y a {} h", s / 3600),
        s if s < 2 * 86_400 => "hier".to_owned(),
        s => format!("il y a {} jours", s / 86_400),
    }
}

fn about(ui: &mut Ui, f: &mut Fx<'_>) {
    let pal = f.pal;
    ui.label(RichText::new(format!("folder {}", env!("CARGO_PKG_VERSION"))).font(semibold(16.0)));
    ui.label(RichText::new("Gestionnaire de fichiers natif pour Windows, écrit en Rust : rendu CPU, aucune WebView, aucun moteur JavaScript.").color(pal.text2));
    ui.add_space(10.0);
    ui.label(
        RichText::new("Mémoire, threads et caches de la fenêtre principale : diagnostics (F12).")
            .color(pal.text2),
    );
    ui.add_space(10.0);
    ui.label(RichText::new("Interface inspirée de Files (Files Community) et de l'Explorateur de Windows 11. Rasterisation CPU dérivée de egui_software_backend (MIT/Apache-2.0).").size(12.0).color(pal.text3));
}
