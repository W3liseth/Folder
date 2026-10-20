//! Onglet glissé d'une fenêtre folder à une autre (chaque fenêtre est un processus).
//!
//! Pendant le glisser, la fenêtre d'origine garde la souris (capture) : l'autre ne reçoit
//! aucun événement. La source lui envoie donc, par `WM_COPYDATA` (`fx_win::ipc`, format
//! `fx_store::remote`), la position du pointeur et le point de saisie de l'onglet tant qu'il
//! survole sa barre d'onglets, puis l'onglet au relâchement (emplacement et historique). La
//! cible montre l'onglet sous le pointeur exactement comme un des siens qu'on glisse (les
//! autres s'écartent), puis l'adopte : il glisse de l'endroit du dépôt jusqu'à sa place. La
//! source le retire, ou se ferme s'il était seul.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Pos2, Vec2, pos2, vec2};
use fx_app::{Location, Msg};
use fx_store::remote::Remote;

use super::Fx;

/// Survol sans nouvelles au-delà duquel l'aperçu disparaît (source fermée, plantée…).
const HOVER_TIMEOUT: Duration = Duration::from_millis(400);

/// Côté cible : onglet d'une autre fenêtre au-dessus de la barre.
struct Hover {
    /// Pointeur en pixels écran, point de saisie en points.
    at: [i32; 2],
    grab: [i32; 2],
    location: Location,
    since: Instant,
}

/// Côté cible : onglet lâché sur la barre, en attente d'insertion.
struct Drop {
    at: [i32; 2],
    grab: [i32; 2],
    path: PathBuf,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
}

/// Onglet entrant, en coordonnées de la fenêtre : pointeur, point de saisie, emplacement.
pub(crate) struct Guest {
    pub pointer: Pos2,
    pub grab: Vec2,
    pub location: Location,
}

/// Onglet adopté : son index et l'abscisse où il a été lâché, d'où il glisse jusqu'à sa
/// place (sans l'apparition d'un nouvel onglet) ; `tabs` : nombre d'onglets avant lui.
#[derive(Clone, Copy)]
pub(crate) struct Adopted {
    pub index: usize,
    pub x: f32,
    pub tabs: usize,
}

/// État des transferts d'onglets de la fenêtre.
#[derive(Default)]
pub(crate) struct PeerState {
    hover: Option<Hover>,
    drop: Option<Drop>,
    pub adopted: Option<Adopted>,
    /// Côté source : fenêtre dont la barre est survolée (pour lui signaler la fin).
    target: Option<isize>,
    /// L'unique onglet est parti dans une autre fenêtre : celle-ci se ferme sans enregistrer
    /// sa session (elle remplacerait celle des fenêtres qui restent).
    pub merged_away: bool,
    /// Côté source : fenêtre masquée (DWM) le temps de survoler la barre d'une autre.
    cloaked: bool,
}

/// Côté source : masque ou réaffiche la fenêtre qu'on déplace avec son unique onglet.
pub(crate) fn set_cloaked(f: &mut Fx<'_>, hwnd: isize, cloaked: bool) {
    if f.vs.peer.cloaked != cloaked {
        fx_win::window::set_cloaked(hwnd, cloaked);
        f.vs.peer.cloaked = cloaked;
    }
}

/// Côté source : réaffiche la fenêtre si elle était masquée (glisser terminé ici).
pub(crate) fn uncloak(f: &mut Fx<'_>) {
    if let Some(hwnd) = f.vs.hwnd {
        set_cloaked(f, hwnd, false);
    }
}

/// Message d'une autre fenêtre concernant les onglets (les autres sont ignorés).
pub(crate) fn receive(peer: &mut PeerState, msg: Remote) {
    match msg {
        Remote::TabHover { at, grab, path } => {
            peer.hover = Some(Hover {
                at,
                grab,
                location: Location::from_path(path),
                since: Instant::now(),
            });
        }
        Remote::TabHoverEnd => peer.hover = None,
        Remote::TabDrop {
            at,
            grab,
            path,
            back,
            forward,
        } => {
            peer.hover = None;
            peer.drop = Some(Drop {
                at,
                grab,
                path,
                back,
                forward,
            });
        }
        _ => {}
    }
}

/// Point écran (pixels) en coordonnées de la fenêtre (points).
fn to_local(ctx: &egui::Context, f: &Fx<'_>, [x, y]: [i32; 2]) -> Option<Pos2> {
    let (left, top, ..) = f.vs.hwnd.and_then(fx_win::window::rect_of)?;
    let ppp = ctx.pixels_per_point();
    Some(pos2((x - left) as f32 / ppp, (y - top) as f32 / ppp))
}

fn grab_of([gx, gy]: [i32; 2]) -> Vec2 {
    vec2(gx as f32, gy as f32)
}

/// Côté cible : onglet survolant la barre (ou qui vient d'y être lâché), tant que la source
/// donne des nouvelles.
pub(crate) fn incoming(ctx: &egui::Context, f: &mut Fx<'_>) -> Option<Guest> {
    if let Some(d) = &f.vs.peer.drop {
        let (at, grab, location) = (d.at, d.grab, Location::from_path(d.path.clone()));
        return match to_local(ctx, f, at) {
            Some(pointer) => Some(Guest {
                pointer,
                grab: grab_of(grab),
                location,
            }),
            None => {
                f.vs.peer.drop = None;
                None
            }
        };
    }
    let h = f.vs.peer.hover.as_ref()?;
    let left = HOVER_TIMEOUT.saturating_sub(h.since.elapsed());
    if left.is_zero() {
        f.vs.peer.hover = None;
        return None;
    }
    ctx.request_repaint_after(left);
    let (at, grab, location) = (h.at, h.grab, h.location.clone());
    Some(Guest {
        pointer: to_local(ctx, f, at)?,
        grab: grab_of(grab),
        location,
    })
}

/// Côté cible : insère l'onglet lâché en `index` ; il glissera depuis l'abscisse `x`.
/// Sans effet pendant un simple survol.
pub(crate) fn adopt(f: &mut Fx<'_>, index: usize, x: f32) {
    if let Some(d) = f.vs.peer.drop.take() {
        f.vs.peer.adopted = Some(Adopted {
            index,
            x,
            tabs: f.state.tabs.len(),
        });
        f.send(Msg::AdoptTab {
            path: d.path,
            back: d.back,
            forward: d.forward,
            index,
        });
    }
}

/// Côté source : autre fenêtre folder dont la barre d'onglets (`bar_height` points) est sous
/// le curseur, et la position du curseur (pixels écran).
pub(crate) fn target_at_cursor(f: &Fx<'_>, bar_height: f32) -> Option<(isize, i32, i32)> {
    let own = f.vs.hwnd?;
    let (x, y) = fx_win::window::cursor_pos();
    let peer = fx_win::window::peer_at(own, x, y)?;
    let (_, top, ..) = fx_win::window::rect_of(peer)?;
    ((y - top) as f32 <= bar_height * fx_win::window::scale_of(peer)).then_some((peer, x, y))
}

/// Côté source : signale le survol de l'onglet `index` saisi en `grab` à `target` (et la fin
/// du survol à la fenêtre précédente).
pub(crate) fn hover(f: &mut Fx<'_>, target: Option<(isize, i32, i32)>, index: usize, grab: Vec2) {
    let now = target.map(|t| t.0);
    if let Some(old) = f.vs.peer.target.filter(|&o| Some(o) != now) {
        fx_win::ipc::send(old, &Remote::TabHoverEnd.encode());
    }
    f.vs.peer.target = now;
    if let (Some((hwnd, x, y)), Some(tab)) = (target, f.state.tabs.get(index)) {
        let msg = Remote::TabHover {
            at: [x, y],
            grab: [grab.x.round() as i32, grab.y.round() as i32],
            path: tab.active_pane().location().window_arg(),
        };
        fx_win::ipc::send(hwnd, &msg.encode());
    }
}

/// Côté source : glisser terminé sans dépôt chez une autre fenêtre.
pub(crate) fn end(f: &mut Fx<'_>) {
    if let Some(old) = f.vs.peer.target.take() {
        fx_win::ipc::send(old, &Remote::TabHoverEnd.encode());
    }
}

/// Côté source : envoie l'onglet `index` saisi en `grab` à `target` et amène celle-ci au
/// premier plan ; vrai si la fenêtre l'a reçu.
pub(crate) fn send_tab(
    f: &mut Fx<'_>,
    (target, x, y): (isize, i32, i32),
    index: usize,
    grab: Vec2,
) -> bool {
    f.vs.peer.target = None;
    let Some(tab) = f.state.tabs.get(index) else {
        return false;
    };
    let pane = tab.active_pane();
    let (back, forward) = pane.history.saved();
    let msg = Remote::TabDrop {
        at: [x, y],
        grab: [grab.x.round() as i32, grab.y.round() as i32],
        path: pane.location().window_arg(),
        back,
        forward,
    };
    let sent = fx_win::ipc::send(target, &msg.encode());
    if sent {
        fx_win::window::bring_to_front(target);
    }
    sent
}
