//! Onglet Historique (`Location::History`) : dossiers visités, regroupés par jour, du plus
//! récent au plus ancien. Un clic ouvre le dossier dans le volet (clic du milieu : nouvel
//! onglet) ; cases à cocher, Ctrl + clic et Maj + clic sélectionnent ; Suppr, le bouton de
//! l'en-tête ou la croix d'une ligne retirent de l'historique.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, FontId, Frame, Rect, RichText, ScrollArea, Sense, Ui};
use eframe::egui::{pos2, vec2};
use fx_app::{Location, Msg, PaneId};
use fx_commands::CommandId;
use fx_win::time::LocalTime;

use super::Fx;
use crate::theme::{glyph, icon_font, semibold};
use crate::widgets::{self, Check};

const ROW_H: f32 = 36.0;
/// Largeur maximale de la liste (au-delà, les chemins s'étirent sans rien apprendre).
const MAX_W: f32 = 980.0;

/// État visuel de l'onglet Historique (un seul par fenêtre : il ne s'ouvre qu'une fois).
#[derive(Default)]
pub(crate) struct HistoryView {
    filter: String,
    /// Dossiers cochés, tels qu'enregistrés dans l'historique.
    selected: Vec<PathBuf>,
    /// Dernière ligne cochée ou décochée (Maj + clic), index dans `rows`.
    anchor: Option<usize>,
    /// Lignes affichées (jours et dossiers), refaites seulement quand l'historique, le
    /// filtre ou le jour changent : rien n'est alloué par image.
    rows: Vec<Row>,
    built_revision: Option<u64>,
    built_filter: String,
    built_day: Option<(u16, u8, u8)>,
}

enum Row {
    Day(String),
    Visit {
        path: PathBuf,
        name: String,
        full: String,
        time: String,
    },
}

impl HistoryView {
    fn rebuild(&mut self, visits: &fx_app::history::Visits) {
        let today = fx_win::time::filetime_to_local(fx_win::time::now_filetime());
        let day = today.map(|t| (t.year, t.month, t.day));
        if self.built_revision == Some(visits.revision())
            && self.built_filter == self.filter
            && self.built_day == day
        {
            return;
        }
        self.built_revision = Some(visits.revision());
        self.built_filter.clone_from(&self.filter);
        self.built_day = day;
        self.rows.clear();
        self.anchor = None;
        let needle = self.filter.trim().to_lowercase();
        let mut last = None;
        for v in visits.iter() {
            let full = v.path.display().to_string();
            if !needle.is_empty() && !full.to_lowercase().contains(&needle) {
                continue;
            }
            let t = fx_win::time::filetime_to_local(v.at);
            let d = t.map(|t| (t.year, t.month, t.day));
            if self.rows.is_empty() || d != last {
                self.rows.push(Row::Day(day_label(t, today)));
                last = d;
            }
            self.rows.push(Row::Visit {
                name: v
                    .path
                    .file_name()
                    .map_or_else(|| full.clone(), |n| n.to_string_lossy().into_owned()),
                time: t.map_or_else(String::new, |t| format!("{:02}:{:02}", t.hour, t.minute)),
                path: v.path.clone(),
                full,
            });
        }
        // Dossiers retirés entre-temps (ici ou dans une autre fenêtre) : plus cochés.
        self.selected
            .retain(|p| visits.iter().any(|v| same_path(&v.path, p)));
    }

    fn is_selected(&self, path: &Path) -> bool {
        self.selected.iter().any(|p| same_path(p, path))
    }

    fn toggle(&mut self, path: &Path) {
        if self.is_selected(path) {
            self.selected.retain(|p| !same_path(p, path));
        } else {
            self.selected.push(path.to_path_buf());
        }
    }
}

/// Dossiers de ces lignes.
fn shown(rows: &[Row]) -> impl Iterator<Item = &PathBuf> {
    rows.iter().filter_map(|r| match r {
        Row::Visit { path, .. } => Some(path),
        Row::Day(_) => None,
    })
}

/// Même dossier (casse ASCII ignorée, comme l'historique).
fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
}

/// Ce qu'une ligne demande à l'image courante.
enum Action {
    Open(PathBuf),
    OpenInNewTab(PathBuf),
    Forget(Vec<PathBuf>),
}

pub(crate) fn page(ui: &mut Ui, f: &mut Fx<'_>, pane: PaneId) {
    let pal = f.pal;
    f.vs.history.rebuild(&f.state.visits);
    // Lignes sorties de l'état le temps de l'image : elles sont lues pendant que `f` sert
    // à tout le reste (icônes, messages), sans copie.
    let rows = std::mem::take(&mut f.vs.history.rows);
    let active = f.state.pane().id == pane;
    let mut action = None;
    Frame::new()
        .inner_margin(egui::Margin::symmetric(28, 20))
        .show(ui, |ui| {
            // Colonne centrée dans le volet ; la barre de défilement reste au bord du volet.
            let avail = ui.available_width();
            let width = avail.min(MAX_W);
            let side = ((avail - width) / 2.0).floor();
            ui.horizontal(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Historique").font(semibold(26.0)));
                        let n = f.state.visits.len();
                        if n > 0 {
                            ui.add_space(10.0);
                            let s = if n > 1 { "s" } else { "" };
                            ui.label(RichText::new(format!("{n} dossier{s}")).color(pal.text2));
                        }
                    });
                    ui.add_space(12.0);
                    header(ui, f, &rows, &mut action);
                });
            });
            ui.add_space(8.0);
            if f.state.visits.is_empty() {
                super::pane::placeholder(
                    ui,
                    f,
                    glyph::HISTORY,
                    "Aucun dossier dans l'historique",
                    "Les dossiers que vous ouvrez apparaîtront ici.",
                );
                return;
            }
            if rows.is_empty() {
                super::pane::placeholder(
                    ui,
                    f,
                    glyph::SEARCH,
                    "Aucun résultat",
                    "Aucun dossier de l'historique ne correspond à la recherche.",
                );
                return;
            }
            ScrollArea::vertical()
                .id_salt(("history-scroll", pane.0))
                .auto_shrink([false, false])
                .show_rows(ui, ROW_H, rows.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for i in range {
                        let (line, lr) = ui
                            .allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
                        // Seule la colonne centrée réagit : un clic dans la marge n'ouvre rien.
                        let rect = Rect::from_min_size(
                            pos2(line.left() + side, line.top()),
                            vec2(width, ROW_H),
                        );
                        let r = ui.interact(rect, lr.id.with("visit"), Sense::click());
                        if let Some(a) = row(ui, f, &rows, i, rect, &r) {
                            action = Some(a);
                        }
                        if r.clicked() && !active {
                            f.send(Msg::FocusPane(pane));
                        }
                    }
                });
        });
    // Clavier : seulement dans le volet actif, hors saisie (filtre).
    if active && !ui.ctx().egui_wants_keyboard_input() {
        let (delete, escape, all) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Delete),
                i.key_pressed(egui::Key::Escape),
                i.modifiers.command && i.key_pressed(egui::Key::A),
            )
        });
        let h = &mut f.vs.history;
        if delete && !h.selected.is_empty() {
            action = Some(Action::Forget(std::mem::take(&mut h.selected)));
        }
        if escape {
            h.selected.clear();
        }
        if all {
            h.selected = shown(&rows).cloned().collect();
        }
    }
    f.vs.history.rows = rows;
    match action {
        Some(Action::Open(p)) => f.send(Msg::NavigatePane(pane, Location::Dir(p))),
        Some(Action::OpenInNewTab(p)) => f.send(Msg::OpenInNewTab(Location::Dir(p))),
        Some(Action::Forget(paths)) => {
            let h = &mut f.vs.history;
            h.selected
                .retain(|s| !paths.iter().any(|p| same_path(p, s)));
            f.send(Msg::ForgetVisits(paths));
        }
        None => {}
    }
}

/// Case « tout sélectionner », recherche, actions sur la sélection et « Effacer
/// l'historique ».
fn header(ui: &mut Ui, f: &mut Fx<'_>, rows: &[Row], action: &mut Option<Action>) {
    let pal = f.pal;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        // Tout sélectionner (parmi les dossiers affichés).
        let h = &f.vs.history;
        let count = shown(rows).count();
        let checked = shown(rows).filter(|p| h.is_selected(p)).count();
        let state = match checked {
            0 => Check::Off,
            n if n == count => Check::On,
            _ => Check::Mixed,
        };
        let (rect, r) = ui.allocate_exact_size(vec2(36.0, 30.0), Sense::click());
        if count > 0 {
            widgets::check_box(ui, rect.center(), state, r.hovered(), &pal);
            let tip = if state == Check::On {
                "Tout désélectionner"
            } else {
                "Tout sélectionner"
            };
            if r.on_hover_text(tip).clicked() {
                let h = &mut f.vs.history;
                if state == Check::On {
                    h.selected.clear();
                } else {
                    h.selected = shown(rows).cloned().collect();
                }
            }
        }
        ui.add(
            egui::TextEdit::singleline(&mut f.vs.history.filter)
                .hint_text("Rechercher dans l'historique")
                .desired_width(280.0),
        );
        let n = f.vs.history.selected.len();
        if n > 0 {
            ui.add_space(8.0);
            let s = if n > 1 { "s" } else { "" };
            ui.label(RichText::new(format!("{n} sélectionné{s}")).color(pal.text2));
            let r = widgets::tool_button(
                ui,
                &pal,
                glyph::DELETE,
                None,
                "Supprimer de l'historique",
                true,
            )
            .on_hover_text("Retirer les dossiers sélectionnés de l'historique (Suppr)");
            if r.clicked() {
                *action = Some(Action::Forget(f.vs.history.selected.clone()));
            }
            let r = widgets::tool_button(ui, &pal, glyph::CANCEL, None, "Désélectionner", true);
            if r.clicked() {
                f.vs.history.selected.clear();
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let enabled = f.enabled(CommandId::ClearHistory);
            let r = widgets::tool_button(
                ui,
                &pal,
                glyph::BROOM,
                None,
                "Effacer l'historique",
                enabled,
            )
            .on_hover_text("Retirer tous les dossiers de l'historique de navigation");
            if enabled && r.clicked() {
                f.vs.history.selected.clear();
                f.cmd(CommandId::ClearHistory);
            }
        });
    });
}

/// Ligne `i` : en-tête de jour, ou dossier (case, icône, nom, chemin, heure, croix).
fn row(
    ui: &mut Ui,
    f: &mut Fx<'_>,
    rows: &[Row],
    i: usize,
    rect: Rect,
    r: &egui::Response,
) -> Option<Action> {
    let pal = f.pal;
    let Row::Visit {
        path,
        name,
        full,
        time,
    } = &rows[i]
    else {
        if let Row::Day(label) = &rows[i] {
            ui.painter().text(
                pos2(rect.left() + 8.0, rect.bottom() - 9.0),
                Align2::LEFT_BOTTOM,
                label,
                semibold(14.0),
                pal.text,
            );
        }
        return None;
    };
    let selected = f.vs.history.is_selected(path);
    let hovered = r.hovered() || r.contains_pointer();
    let bg = rect.shrink2(vec2(0.0, 1.0));
    if selected {
        ui.painter()
            .rect_filled(bg, 4.0, pal.accent.gamma_multiply(0.18));
    } else {
        widgets::hover_fill(ui, r, bg, &pal, 4.0);
    }

    // Case à cocher.
    let check = Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(28.0, 28.0));
    let cr = ui.interact(check, r.id.with("check"), Sense::click());
    if selected || hovered || cr.hovered() || !f.vs.history.selected.is_empty() {
        let state = if selected { Check::On } else { Check::Off };
        widgets::check_box(ui, check.center(), state, cr.hovered(), &pal);
    }
    super::items::shell_icon(
        ui,
        f,
        path,
        pos2(rect.left() + 48.0, rect.center().y),
        16.0,
        glyph::FOLDER,
        pal.folder,
    );

    // Croix (au survol) et heure, à droite.
    let close =
        Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(26.0, 26.0));
    let xr = ui.interact(close, r.id.with("forget"), Sense::click());
    if hovered || xr.hovered() {
        widgets::hover_fill(ui, &xr, close, &pal, 4.0);
        ui.painter().text(
            close.center(),
            Align2::CENTER_CENTER,
            glyph::CLOSE,
            icon_font(10.0),
            pal.text2,
        );
    }
    let time_rect = ui.painter().text(
        pos2(rect.right() - 40.0, rect.center().y),
        Align2::RIGHT_CENTER,
        time,
        FontId::proportional(12.5),
        pal.text2,
    );

    // Nom (au plus 45 % de la ligne) puis chemin complet, coupés avant l'heure.
    f.vs.fallbacks.scan(name);
    f.vs.fallbacks.scan(full);
    let text_left = rect.left() + 66.0;
    let text_right = time_rect.left() - 16.0;
    let name_clip = Rect::from_x_y_ranges(
        text_left..=(text_left + rect.width() * 0.45).min(text_right),
        rect.y_range(),
    );
    let name_rect = ui.painter().with_clip_rect(name_clip).text(
        pos2(text_left, rect.center().y),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(14.0),
        pal.text,
    );
    let path_left = name_rect.right().min(name_clip.right()) + 12.0;
    // Racine d'un lecteur : le nom est déjà le chemin complet.
    if path_left < text_right && name != full {
        let path_clip = Rect::from_x_y_ranges(path_left..=text_right, rect.y_range());
        ui.painter().with_clip_rect(path_clip).text(
            pos2(path_left, rect.center().y),
            Align2::LEFT_CENTER,
            full,
            FontId::proportional(12.5),
            pal.text2,
        );
    }

    if xr.on_hover_text("Retirer de l'historique").clicked() {
        return Some(Action::Forget(vec![path.clone()]));
    }
    let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
    let h = &mut f.vs.history;
    if cr.clicked() || (r.clicked() && (ctrl || !h.selected.is_empty()) && !shift) {
        // Case, Ctrl + clic, ou clic pendant une sélection : coche ou décoche.
        h.toggle(path);
        h.anchor = Some(i);
    } else if r.clicked() && shift {
        let from = h.anchor.unwrap_or(i);
        let (a, b) = (from.min(i), from.max(i));
        for p in shown(&rows[a..=b.min(rows.len() - 1)]) {
            if !h.is_selected(p) {
                h.selected.push(p.clone());
            }
        }
        h.anchor = Some(i);
    } else if r.clicked() || r.double_clicked() {
        return Some(Action::Open(path.clone()));
    }
    if r.middle_clicked() {
        return Some(Action::OpenInNewTab(path.clone()));
    }
    let mut out = None;
    r.clone().on_hover_text(full).context_menu(|ui| {
        widgets::menu_appear(ui);
        ui.set_min_width(240.0);
        if ui
            .button(widgets::menu_entry(glyph::OPEN, "Ouvrir", &pal))
            .clicked()
        {
            out = Some(Action::Open(path.clone()));
            ui.close();
        }
        if ui
            .button(widgets::menu_entry(
                CommandId::OpenInNewTab.glyph(),
                CommandId::OpenInNewTab.label(),
                &pal,
            ))
            .clicked()
        {
            out = Some(Action::OpenInNewTab(path.clone()));
            ui.close();
        }
        ui.separator();
        // Clic droit sur un dossier coché : toute la sélection ; sinon ce dossier.
        let h = &f.vs.history;
        let targets = if selected {
            h.selected.clone()
        } else {
            vec![path.clone()]
        };
        let label = if targets.len() > 1 {
            format!("Supprimer {} dossiers de l'historique", targets.len())
        } else {
            "Supprimer de l'historique".to_owned()
        };
        if ui
            .button(widgets::menu_entry(glyph::DELETE, &label, &pal))
            .clicked()
        {
            out = Some(Action::Forget(targets));
            ui.close();
        }
    });
    out
}

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
const WEEKDAYS: [&str; 7] = [
    "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
];

/// Jours depuis le 1er janvier 1970 (calendrier grégorien proleptique, algorithme de
/// H. Hinnant).
fn civil_days(y: u16, m: u8, d: u8) -> i64 {
    let (m, d) = (i64::from(m), i64::from(d));
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// « Aujourd'hui », « Hier », « Lundi 3 novembre » (cette année) ou « 3 novembre 2025 ».
fn day_label(t: Option<LocalTime>, today: Option<LocalTime>) -> String {
    let Some(t) = t else {
        return "Date inconnue".into();
    };
    let days = civil_days(t.year, t.month, t.day);
    if let Some(n) = today {
        match civil_days(n.year, n.month, n.day) - days {
            0 => return "Aujourd'hui".into(),
            1 => return "Hier".into(),
            _ => {}
        }
    }
    let month = MONTHS[usize::from(t.month.clamp(1, 12)) - 1];
    if today.is_some_and(|n| n.year == t.year) {
        // 1970-01-01 était un jeudi.
        let wd = WEEKDAYS[(days + 3).rem_euclid(7) as usize];
        let mut wd = wd.to_owned();
        wd[..1].make_ascii_uppercase();
        format!("{wd} {} {month}", t.day)
    } else {
        format!("{} {month} {}", t.day, t.year)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lt(year: u16, month: u8, day: u8) -> Option<LocalTime> {
        Some(LocalTime {
            year,
            month,
            day,
            hour: 10,
            minute: 0,
        })
    }

    #[test]
    fn day_labels() {
        let today = lt(2026, 11, 8);
        assert_eq!(day_label(lt(2026, 11, 8), today), "Aujourd'hui");
        assert_eq!(day_label(lt(2026, 11, 7), today), "Hier");
        assert_eq!(day_label(lt(2026, 11, 2), today), "Lundi 2 novembre");
        assert_eq!(day_label(lt(2026, 1, 1), lt(2026, 1, 2)), "Hier");
        assert_eq!(day_label(lt(2025, 12, 31), lt(2026, 1, 1)), "Hier");
        assert_eq!(day_label(lt(2024, 2, 29), today), "29 février 2024");
        assert_eq!(day_label(None, today), "Date inconnue");
    }

    #[test]
    fn civil_days_matches_known_dates() {
        assert_eq!(civil_days(1970, 1, 1), 0);
        assert_eq!(civil_days(2000, 3, 1), 11_017);
        assert_eq!(civil_days(2026, 11, 8) - civil_days(2026, 11, 1), 7);
    }
}
