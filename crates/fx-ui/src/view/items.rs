//! Comportement commun aux éléments des vues (Détails, Liste, Grille, Colonnes) :
//! icônes, clics, glisser-déposer, dépôt sur un dossier, renommage sur place.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Id, Key, Rect, Response, TextEdit, TextureId, Ui};
use fx_app::{Msg, PaneId};
use fx_core::{DirSnapshot, EntryView};
use fx_shell::{ImageKey, THUMBNAIL_EXTS, image_request};

use super::{Fx, menus};

/// Icône (ou miniature) d'une entrée, si déjà chargée ; la demande sinon.
/// Renvoie l'identifiant de texture et sa taille en pixels.
pub fn texture(
    f: &mut Fx<'_>,
    l: &DirSnapshot,
    e: &EntryView<'_>,
    size: u16,
    thumbnail: bool,
) -> Option<(TextureId, egui::Vec2)> {
    let name = fx_core::wtf8::display(e.name);
    let ext_bytes = l.ext_of(e.meta);
    let ext = std::str::from_utf8(ext_bytes).unwrap_or("");
    let dir = l.dir_of(e.id);
    if thumbnail
        && !e.meta.is_dir()
        && f.state.settings.show_thumbnails
        && THUMBNAIL_EXTS.contains(&ext)
    {
        let path = dir.join(name.as_ref());
        let key = ImageKey::Thumb {
            path: path.clone(),
            size,
            modified: e.meta.modified,
            len: e.meta.size,
        };
        if !f.images.failed(&key)
            && let Some(t) = f.images.get(f.shell, &key, &path)
        {
            return Some((t.id(), t.size_vec2()));
        }
        // En attendant la miniature : l'icône du type.
    }
    // Appareil du réseau : icône du shell par son nom d'analyse.
    if let Some(parsing) = f.state.shell_item(l.generation(), e.id) {
        let source = PathBuf::from(parsing);
        let key = ImageKey::Path {
            path: source.clone(),
            size,
        };
        return f
            .images
            .get(f.shell, &key, &source)
            .map(|t| (t.id(), t.size_vec2()));
    }
    // Dossier personnalisé (Bureau, Téléchargements, desktop.ini…) : Windows le marque
    // en lecture seule ou système ; son icône lui est propre, comme dans l'Explorateur.
    let custom_dir = e.meta.is_dir()
        && e.meta.attributes & (fx_core::entry::attrs::READONLY | fx_core::entry::attrs::SYSTEM)
            != 0;
    let (key, source) = if custom_dir {
        let path = dir.join(name.as_ref());
        (
            ImageKey::Path {
                path: path.clone(),
                size,
            },
            path,
        )
    } else {
        image_request(dir, &name, ext, e.meta.is_dir(), size)
    };
    f.images
        .get(f.shell, &key, &source)
        .map(|t| (t.id(), t.size_vec2()))
}

/// État de synchronisation (Google Drive, OneDrive, fichiers hors connexion…) dessiné sur
/// l'icône `rect` de l'entrée, comme dans l'Explorateur : l'icône superposée que le
/// fournisseur fait afficher, son badge dans un coin.
pub fn sync_overlay(ui: &Ui, f: &mut Fx<'_>, l: &DirSnapshot, e: &EntryView<'_>, rect: Rect) {
    // Appareils du réseau : pas de fichier à interroger.
    if f.state.shell_item(l.generation(), e.id).is_some() {
        return;
    }
    let ctx = ui.ctx().clone();
    let px = (rect.width() * ctx.pixels_per_point()).round() as u16;
    let path = || l.dir_of(e.id).join(fx_core::wtf8::display(e.name).as_ref());
    if let Some(t) = f
        .images
        .overlay(&ctx, f.shell, (l.generation(), e.id), px.max(16), path)
    {
        ui.painter().image(
            t.id(),
            rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
}

/// Icône shell propre à `path` (icône de dossier définie par Windows, lecteur…), dessinée
/// en `size` points autour de `center` ; glyphe `fallback` tant qu'elle n'est pas chargée.
#[allow(clippy::too_many_arguments)]
pub fn shell_icon(
    ui: &Ui,
    f: &mut Fx<'_>,
    path: &Path,
    center: egui::Pos2,
    size: f32,
    fallback: &str,
    color: egui::Color32,
) {
    let px = (size * ui.ctx().pixels_per_point()).round() as u16;
    let key = ImageKey::Path {
        path: path.to_path_buf(),
        size: px.max(16),
    };
    match f.images.get(f.shell, &key, path) {
        Some(tex) => {
            ui.painter().image(
                tex.id(),
                Rect::from_center_size(center, egui::vec2(size, size)),
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            ui.painter().text(
                center,
                egui::Align2::CENTER_CENTER,
                fallback,
                crate::theme::icon_font(size * 0.85),
                color,
            );
        }
    }
}

/// Nom affiché (sans extension si le réglage le demande).
pub fn display_name(f: &Fx<'_>, l: &DirSnapshot, e: &EntryView<'_>) -> String {
    let name = fx_core::wtf8::display(e.name).into_owned();
    if f.state.settings.show_extensions || e.meta.is_dir() {
        return name;
    }
    let ext = l.ext_of(e.meta);
    if !ext.is_empty() && name.len() > ext.len() + 1 {
        name[..name.len() - ext.len() - 1].to_owned()
    } else {
        name
    }
}

/// Clic sur la case de sélection d'un élément : ajoute ou retire de la sélection (comme
/// Ctrl + clic), sans ouvrir ni renommer.
pub fn toggle_check(f: &mut Fx<'_>, pane: PaneId, pos: usize) {
    f.vs.slow_click = None;
    f.send(Msg::FocusPane(pane));
    f.send(Msg::Click {
        pane,
        pos,
        ctrl: true,
        shift: false,
    });
}

/// Interactions d'un élément. `column` : colonne de la vue Colonnes (sinon `None`).
#[allow(clippy::too_many_arguments)]
pub fn interact(
    ui: &Ui,
    f: &mut Fx<'_>,
    pane: PaneId,
    l: &DirSnapshot,
    pos: usize,
    e: &EntryView<'_>,
    resp: &Response,
    selected: bool,
    column: Option<usize>,
) {
    let path = fx_app::state::path_of(l, e.id);
    let is_dir = e.meta.is_dir();
    let mods = ui.input(|i| i.modifiers);
    let click = |f: &mut Fx<'_>| {
        f.send(Msg::FocusPane(pane));
        match column {
            Some(c) => f.send(Msg::ColumnClick {
                pane,
                column: c,
                pos,
            }),
            None => f.send(Msg::Click {
                pane,
                pos,
                ctrl: mods.command,
                shift: mods.shift,
            }),
        }
    };

    // Sélection dès l'appui, comme l'Explorateur : on voit ce qui va être glissé. Seule
    // exception, un élément déjà sélectionné (sans Maj) : le relâchement décide, pour qu'un
    // glisser emporte toute la sélection et que Ctrl + clic ne la retire qu'au relâchement.
    if resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()) {
        let now = !selected || mods.shift;
        f.vs.pressed = now.then_some((pane, e.id));
        if now {
            click(f);
        }
    }
    if resp.drag_started() {
        f.vs.pressed = None; // pas de clic au relâchement d'un glisser
    }
    if resp.clicked() {
        // Sélectionné à l'appui : il ne l'était pas avant ce clic.
        let was_selected = f.vs.pressed.take() != Some((pane, e.id));
        if was_selected {
            click(f);
        }
        let selected = selected && was_selected;
        // Ouverture d'un simple clic (réglage).
        if f.state.settings.single_click && !mods.command && !mods.shift && column.is_none() {
            f.send(Msg::Activate { pane, pos });
        }
        // Clic lent sur l'élément déjà seul sélectionné : renommage une fois le délai de
        // double-clic écoulé sans second clic (comme l'Explorateur).
        let alone = f
            .state
            .find_pane(pane)
            .and_then(fx_app::Pane::active_listing)
            .is_some_and(|(_, sel)| sel.count() == 1);
        f.vs.slow_click = (selected
            && alone
            && !resp.double_clicked()
            && !mods.any()
            && !f.state.settings.single_click)
            .then(|| {
                let delay = ui.ctx().options(|o| o.input_options.max_double_click_delay);
                // Repaint réactif : sans ce réveil, aucune image ne viendrait déclencher
                // le renommage une fois le délai écoulé.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(delay));
                super::SlowClick {
                    pane,
                    id: e.id,
                    at: std::time::Instant::now() + std::time::Duration::from_secs_f64(delay),
                }
            });
    }
    if resp.double_clicked() && !f.state.settings.single_click {
        f.send(Msg::Activate { pane, pos });
    }
    if resp.middle_clicked() && is_dir {
        f.send(Msg::OpenInNewTab(fx_app::Location::Dir(path.clone())));
    }
    if resp.secondary_clicked() {
        super::shellmenu::secondary_click(ui, f);
    }
    if resp.secondary_clicked() && !selected {
        f.send(Msg::FocusPane(pane));
        match column {
            Some(c) => f.send(Msg::ColumnClick {
                pane,
                column: c,
                pos,
            }),
            None => f.send(Msg::Click {
                pane,
                pos,
                ctrl: false,
                shift: false,
            }),
        }
    }
    // Glisser vers un dossier, la barre latérale, un onglet… puis, hors de la fenêtre, vers
    // l'Explorateur ou le bureau (cf. `dnd`). Seulement depuis de vrais dossiers : les
    // éléments de la Corbeille et du Réseau ne sont pas des fichiers déplaçables.
    let real_files = f.state.find_pane(pane).is_some_and(|p| {
        matches!(
            p.location(),
            fx_app::Location::Dir(_) | fx_app::Location::Search { .. } | fx_app::Location::Tag(_)
        )
    });
    if resp.drag_started_by(egui::PointerButton::Primary) && real_files && f.vs.band.is_none() {
        let paths = if selected {
            f.state
                .find_pane(pane)
                .map(fx_app::Pane::selected_paths)
                .unwrap_or_default()
        } else {
            vec![path.clone()]
        };
        super::dnd::start(f, paths, is_dir);
    }
    if is_dir {
        super::dnd::zone(f, resp.rect, super::dnd::Target::Dir(path.clone()));
    }
    let paths_for_menu = |f: &Fx<'_>| -> Vec<PathBuf> {
        if selected {
            f.state
                .find_pane(pane)
                .map(fx_app::Pane::selected_paths)
                .unwrap_or_default()
        } else {
            vec![path.clone()]
        }
    };
    let paths = paths_for_menu(f);
    resp.context_menu(|ui| menus::item_menu(ui, f, &paths, is_dir));
}

/// Champ de renommage sur place. `text_rect` : zone où le nom est dessiné d'habitude (sa
/// hauteur est celle d'une ligne de texte), `font` : police du nom. Le texte saisi se place
/// exactement sur le nom affiché ; le cadre déborde autour. Renvoie vrai s'il a été dessiné.
#[allow(clippy::too_many_arguments)]
pub fn rename_editor(
    ui: &mut Ui,
    f: &mut Fx<'_>,
    text_rect: Rect,
    font: egui::FontId,
    pane: PaneId,
    id: u32,
    current: &str,
    align_center: bool,
) -> bool {
    /// Marges intérieures du champ (le contour d'un point est compris dans `PAD_X`).
    const PAD_X: f32 = 6.0;
    const PAD_Y: f32 = 3.0;
    let Some(r) = f.state.renaming.filter(|r| r.pane == pane && r.id == id) else {
        return false;
    };
    let fresh =
        f.vs.rename
            .as_ref()
            .is_none_or(|(serial, _)| *serial != r.serial);
    if fresh {
        f.vs.rename = Some((r.serial, current.to_owned()));
    }
    let Some((_, text)) = &mut f.vs.rename else {
        return false;
    };
    // Largeur ajustée au nom saisi (comme l'Explorateur), dans la limite de la colonne.
    let galley = ui
        .painter()
        .layout_no_wrap(format!("{text}W"), font.clone(), f.pal.text);
    let line_h = galley.size().y;
    let max_w = text_rect.width() + if align_center { 0.0 } else { PAD_X };
    let w = (galley.size().x + 2.0 * PAD_X).clamp(80.0_f32.min(max_w), max_w);
    let cy = text_rect.center().y;
    let left = if align_center {
        text_rect.center().x - w / 2.0
    } else {
        text_rect.left() - PAD_X
    };
    let frame = Rect::from_min_size(
        egui::pos2(left, cy - line_h / 2.0 - PAD_Y),
        egui::vec2(w, line_h + 2.0 * PAD_Y),
    );
    let edit_id = Id::unique(("rename", r.serial));
    ui.painter().rect_filled(frame, 4.0, f.pal.card);
    ui.painter().rect_stroke(
        frame,
        4.0,
        egui::Stroke::new(1.0, f.pal.accent),
        egui::StrokeKind::Inside,
    );
    let edit = TextEdit::singleline(text)
        .id(edit_id)
        .font(font)
        .text_color(f.pal.text)
        // egui 0.37 : la marge intérieure appartient au cadre (`margin` seul est ignoré
        // avec un cadre vide).
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(PAD_X as i8, PAD_Y as i8)))
        .desired_width(w - 2.0 * PAD_X)
        .min_size(frame.size())
        .align(if align_center {
            egui::Align2::CENTER_TOP
        } else {
            egui::Align2::LEFT_TOP
        });
    let resp = ui.put(frame, edit);
    if fresh {
        resp.request_focus();
        // Sélectionne le nom sans l'extension, comme l'Explorateur.
        let stem_len = Path::new(current)
            .file_stem()
            .map_or(0, |s| s.to_string_lossy().chars().count());
        let end = if current.contains('.') && stem_len > 0 {
            stem_len
        } else {
            current.chars().count()
        };
        if let Some(mut st) = TextEdit::load_state(ui.ctx(), edit_id) {
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(end),
            )));
            st.store(ui.ctx(), edit_id);
        }
    }
    if resp.lost_focus() && !fresh {
        let cancel = ui.input(|i| i.key_pressed(Key::Escape));
        let name = f.vs.rename.take().map(|(_, t)| t).unwrap_or_default();
        if cancel {
            f.send(Msg::CancelRename);
        } else {
            f.send(Msg::CommitRename { pane, id, name });
        }
    }
    true
}
