//! Glisser-déposer de fichiers et de dossiers.
//!
//! Dans la fenêtre, folder gère lui-même le glisser : cible surlignée au survol, aperçu de
//! l'opération sous le pointeur, dépôt au relâchement. Le glisser-déposer OLE
//! (`SHDoDragDrop`, boucle modale qui fige l'interface jusqu'au dépôt) ne prend le relais que
//! lorsque le pointeur quitte la fenêtre : Explorateur, bureau, autre application. Les dépôts
//! venus de l'extérieur, ou d'un glisser OLE revenu dans la fenêtre, arrivent par
//! `dropped_files` ; pendant un glisser OLE la fenêtre ne reçoit aucun message de souris, la
//! position du curseur est donc lue à Windows.
//!
//! Les cibles (« zones ») sont enregistrées par les vues pendant le dessin, seulement quand un
//! glisser est en cours ; la dernière enregistrée qui contient le pointeur l'emporte (un
//! dossier de la liste plutôt que le fond de son volet).

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Id, LayerId, Modifiers, Order, Pos2, Rect, Stroke,
    StrokeKind, pos2, vec2,
};
use fx_app::Msg;

use super::Fx;
use crate::theme::{Palette, glyph, icon_font};

/// Cible d'un dépôt.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Dir(PathBuf),
    RecycleBin,
}

struct Zone {
    rect: Rect,
    target: Target,
    /// Fond d'un volet : surlignage en retrait, sur tout le volet.
    pane: bool,
}

/// Glisser interne en cours : éléments d'un volet.
struct ItemDrag {
    paths: Vec<PathBuf>,
    /// L'élément saisi est un dossier (icône de l'aperçu).
    dir: bool,
}

#[derive(Default)]
pub(crate) struct DndState {
    drag: Option<ItemDrag>,
    /// Zones de l'image en cours et de la précédente (un relâchement se résout en début
    /// d'image, sur ce qui était affiché).
    zones: Vec<Zone>,
    prev: Vec<Zone>,
    /// Un glisser est en cours (interne ou venu de l'extérieur) : les vues enregistrent
    /// leurs zones. Sinon rien n'est collecté.
    active: bool,
    /// Glisser OLE à lancer en fin d'image : le pointeur a quitté la fenêtre.
    ole: Option<Vec<PathBuf>>,
}

/// Commence un glisser interne de `paths` (premier mouvement d'un élément saisi).
pub(crate) fn start(f: &mut Fx<'_>, paths: Vec<PathBuf>, dir: bool) {
    if !paths.is_empty() {
        f.vs.dnd.drag = Some(ItemDrag { paths, dir });
        f.vs.dnd.active = true;
    }
}

/// Enregistre une cible de dépôt pour cette image (sans effet hors glisser).
pub(crate) fn zone(f: &mut Fx<'_>, rect: Rect, target: Target) {
    if f.vs.dnd.active {
        f.vs.dnd.zones.push(Zone {
            rect,
            target,
            pane: false,
        });
    }
}

/// Fond d'un volet : dépôt dans son dossier.
pub(crate) fn pane_zone(f: &mut Fx<'_>, rect: Rect, dir: &Path) {
    if f.vs.dnd.active {
        f.vs.dnd.zones.push(Zone {
            rect,
            target: Target::Dir(dir.to_path_buf()),
            pane: true,
        });
    }
}

/// Début d'image : dépôt ou abandon d'un glisser interne, relais OLE si le pointeur est sorti.
pub(crate) fn begin(ctx: &egui::Context, f: &mut Fx<'_>) {
    let dnd = &mut f.vs.dnd;
    std::mem::swap(&mut dnd.zones, &mut dnd.prev);
    dnd.zones.clear();
    if dnd.drag.is_some() {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, egui::Key::Escape)) {
            dnd.drag = None;
        } else if !ctx.input(|i| i.pointer.primary_down()) {
            // Relâché dans la fenêtre : dépôt sur la cible surlignée à l'image précédente.
            let drag = dnd.drag.take();
            let (pos, mods) = ctx.input(|i| (i.pointer.latest_pos(), i.modifiers));
            if let (Some(drag), Some(pos)) = (drag, pos)
                && let Some(z) = zone_at(&dnd.prev, pos)
                && useful(&z.target, &drag.paths, copy_choice(mods))
            {
                let msg = drop_msg(z.target.clone(), drag.paths, mods);
                f.send(msg);
            }
        } else if outside(ctx) {
            dnd.ole = dnd.drag.take().map(|d| d.paths);
        }
    }
    let (hovering, dropped) = ctx.input(|i| {
        (
            !i.raw.hovered_files.is_empty(),
            !i.raw.dropped_files.is_empty(),
        )
    });
    if hovering {
        // Glisser OLE au-dessus de la fenêtre : aucun message de souris, on suit le curseur
        // en redessinant (le temps du survol seulement).
        ctx.request_repaint_after(Duration::from_millis(30));
    }
    f.vs.dnd.active = f.vs.dnd.drag.is_some() || hovering || dropped;
}

/// Fin d'image : surlignage de la cible, aperçu, dépôts venus de l'extérieur, relais OLE.
pub(crate) fn finish(ctx: &egui::Context, f: &mut Fx<'_>) {
    let mods = ctx.input(|i| i.modifiers);
    let copy = copy_choice(mods);
    if let Some(drag) = &f.vs.dnd.drag {
        let pos = ctx.input(|i| i.pointer.latest_pos());
        let target = pos
            .and_then(|p| zone_at(&f.vs.dnd.zones, p))
            .filter(|z| useful(&z.target, &drag.paths, copy));
        if let Some(z) = target {
            highlight(ctx, &f.pal, z);
        }
        if let Some(p) = pos {
            let (g, gc) = if drag.dir {
                (glyph::FOLDER, f.pal.folder)
            } else {
                (glyph::FILE, f.pal.text2)
            };
            let text = match target.map(|z| &z.target) {
                Some(Target::Dir(dest)) => {
                    let copies =
                        copy.unwrap_or_else(|| fx_app::update::drop_copies(&drag.paths, dest));
                    let verb = if copies {
                        "Copier vers"
                    } else {
                        "Déplacer vers"
                    };
                    format!("{verb} {}", super::sidebar::display_name(dest))
                }
                Some(Target::RecycleBin) => "Mettre à la corbeille".to_owned(),
                None => match drag.paths.as_slice() {
                    [one] => super::sidebar::display_name(one),
                    many => format!("{} éléments", many.len()),
                },
            };
            ghost(ctx, &f.pal, p + vec2(14.0, 16.0), g, gc, &text);
        }
        ctx.set_cursor_icon(match (target, copy) {
            (None, _) => CursorIcon::NoDrop,
            (Some(_), Some(true)) => CursorIcon::Copy,
            _ => CursorIcon::Default,
        });
    }
    external(ctx, f, copy);
    if let Some(paths) = f.vs.dnd.ole.take()
        && let Some(hwnd) = f.vs.hwnd
    {
        // Modal jusqu'au dépôt ; un dépôt revenu dans folder arrive par `dropped_files`.
        if let Err(e) = fx_win::ctxmenu::drag_out(hwnd, &paths) {
            f.send(Msg::Error(format!(
                "Glisser-déposer impossible : {}",
                e.message().trim()
            )));
        }
    }
}

/// Glisser venu d'ailleurs (Explorateur, ou glisser OLE de folder revenu dans la fenêtre) :
/// surlignage pendant le survol, dépôt sur la cible sous le curseur.
fn external(ctx: &egui::Context, f: &mut Fx<'_>, copy: Option<bool>) {
    let (hovered, dropped): (Vec<PathBuf>, Vec<PathBuf>) = ctx.input(|i| {
        (
            i.raw
                .hovered_files
                .iter()
                .filter_map(|h| h.path.clone())
                .collect(),
            i.raw
                .dropped_files
                .iter()
                .map(|d| d.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect(),
        )
    });
    if hovered.is_empty() && dropped.is_empty() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let pos =
        f.vs.hwnd
            .and_then(fx_win::window::client_cursor_pos)
            .map(|(x, y)| pos2(x as f32 / ppp, y as f32 / ppp));
    let at = pos.and_then(|p| zone_at(&f.vs.dnd.zones, p));
    if dropped.is_empty() {
        if let Some(z) = at.filter(|z| useful(&z.target, &hovered, copy)) {
            highlight(ctx, &f.pal, z);
        }
        return;
    }
    // Hors de toute cible (barre d'outils, barre d'état…) : dossier du volet actif.
    let target = match at {
        Some(z) => z.target.clone(),
        None => match f.state.pane().working_dir() {
            Some(d) => Target::Dir(d.to_path_buf()),
            None => return,
        },
    };
    let mods = ctx.input(|i| i.modifiers);
    f.send(drop_msg(target, dropped, mods));
}

/// Touches de modification, comme l'Explorateur : Ctrl copie, Maj déplace, sinon selon le
/// volume (cf. [`fx_app::update::drop_copies`]).
fn copy_choice(mods: Modifiers) -> Option<bool> {
    if mods.ctrl {
        Some(true)
    } else if mods.shift {
        Some(false)
    } else {
        None
    }
}

fn drop_msg(target: Target, paths: Vec<PathBuf>, mods: Modifiers) -> Msg {
    match target {
        Target::Dir(dest) => Msg::DropPaths {
            dest,
            paths,
            copy: copy_choice(mods),
        },
        Target::RecycleBin => Msg::DropToRecycleBin(paths),
    }
}

fn zone_at(zones: &[Zone], pos: Pos2) -> Option<&Zone> {
    zones.iter().rev().find(|z| z.rect.contains(pos))
}

/// Le dépôt ferait-il quelque chose ? Pas dans un élément déplacé ni dans l'un de ses
/// sous-dossiers, ni un déplacement vers le dossier d'où il vient (cf. `edit::drop_paths`).
fn useful(target: &Target, paths: &[PathBuf], copy: Option<bool>) -> bool {
    match target {
        Target::RecycleBin => !paths.is_empty(),
        Target::Dir(dest) => paths.iter().any(|p| {
            p != dest
                && !dest.starts_with(p)
                && (copy == Some(true) || p.parent() != Some(dest.as_path()))
        }),
    }
}

/// Le pointeur a quitté la fenêtre (le bouton toujours enfoncé).
fn outside(ctx: &egui::Context) -> bool {
    let inner = ctx.content_rect().shrink(2.0);
    ctx.input(|i| i.pointer.latest_pos())
        .is_none_or(|p| !inner.contains(p))
}

fn highlight(ctx: &egui::Context, pal: &Palette, z: &Zone) {
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("fx-dnd-target")));
    let (rect, radius) = if z.pane {
        (z.rect.shrink(3.0), 8.0)
    } else {
        (z.rect, 4.0)
    };
    painter.rect_filled(rect, radius, pal.accent.gamma_multiply(0.12));
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.5, pal.accent),
        StrokeKind::Inside,
    );
}

/// Aperçu d'un glisser près du pointeur : pastille avec icône et texte, coin haut gauche en
/// `at` (ramenée dans la fenêtre près des bords).
pub(crate) fn ghost(
    ctx: &egui::Context,
    pal: &Palette,
    at: Pos2,
    g: &str,
    glyph_color: Color32,
    text: &str,
) {
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::unique("fx-dnd-ghost")));
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(13.0), pal.text);
    let size = vec2(galley.size().x + 44.0, 30.0);
    let mut rect = Rect::from_min_size(at, size);
    // Reste dans la fenêtre près du bord droit ou bas.
    let screen = ctx.content_rect();
    if rect.right() > screen.right() - 4.0 {
        rect = rect.translate(vec2(screen.right() - 4.0 - rect.right(), 0.0));
    }
    if rect.bottom() > screen.bottom() - 4.0 {
        rect = rect.translate(vec2(0.0, -(size.y + 24.0)));
    }
    painter.rect_filled(
        rect.translate(vec2(0.0, 2.0)),
        8.0,
        Color32::from_black_alpha(40),
    );
    painter.rect_filled(rect, 8.0, pal.layer);
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, pal.accent.gamma_multiply(0.7)),
        StrokeKind::Inside,
    );
    painter.text(
        pos2(rect.left() + 18.0, rect.center().y),
        Align2::CENTER_CENTER,
        g,
        icon_font(14.0),
        glyph_color,
    );
    painter.galley(
        pos2(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0),
        galley,
        pal.text,
    );
}
