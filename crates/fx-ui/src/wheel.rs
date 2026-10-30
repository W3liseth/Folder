//! Défilement à la molette : accélération selon la cadence des crans, glissé continu.
//!
//! egui avance chaque cran d'une distance fixe, amortie séparément : un cran isolé part d'un
//! coup puis freine, et la vitesse ne dépend pas de l'allure de la molette. Ici :
//!
//! - **Accélération** : la distance d'un cran croît avec la cadence récente des crans
//!   (moyenne à décroissance exponentielle, constante [`TAU`]). Molette tournée lentement :
//!   [`BASE`] points par cran, précis ; tournée vite : jusqu'à [`MAX_GAIN`] fois plus.
//! - **Glissé** : le contenu rejoint sa cible par un ressort à amortissement critique. La
//!   vitesse reste continue quand un cran arrive en plein mouvement (pas d'à-coup), sans
//!   dépassement ; le mouvement s'arrête de lui-même (~0,25 s après le dernier cran).
//!
//! Appliqué en début d'image, avant toute zone de défilement : le résultat remplace
//! `smooth_scroll_delta`, que lisent les `ScrollArea`. Les molettes haute résolution et les
//! pavés tactiles (fractions de cran) ne sont pas accélérés : leur vitesse suit déjà le geste.
//!
//! **Barres de défilement** : visibles pendant le défilement (molette, touches de
//! navigation), elles s'effacent [`BAR_HOLD`] secondes après le dernier mouvement, même
//! pointeur au-dessus de la liste. Survolées ou saisies, egui les montre toujours.

use std::time::Duration;

use crate::egui::{Context, Event, Id, InputState, Key, MouseWheelUnit, TouchPhase, Vec2, vec2};

/// Distance d'un cran isolé (points) : deux lignes de la vue Détails.
pub const BASE: f32 = 60.0;
/// Constante de temps de la cadence des crans (s).
const TAU: f64 = 0.1;
/// Cadence (crans récents pondérés) en dessous de laquelle rien n'est accéléré : ~6 crans/s.
const CALM: f32 = 1.2;
/// Accélération par cran récent au-delà de [`CALM`].
const GAIN: f32 = 1.6;
/// Plafond de l'accélération.
pub const MAX_GAIN: f32 = 7.0;
/// Raideur du ressort (1/s) : 90 % de la distance en ~0,13 s.
const OMEGA: f32 = 30.0;
/// Écart (points) et vitesse (points/s) en dessous desquels le glissé se termine.
const SETTLE_DIST: f32 = 0.5;
const SETTLE_VEL: f32 = 20.0;
/// Barres de défilement : durée d'affichage après le dernier défilement (s), puis fondu (s).
pub const BAR_HOLD: f64 = 2.0;
const BAR_FADE: f32 = 0.15;
/// Opacités des barres au-dessus de la zone, non survolées (valeurs d'egui).
const BAR_HANDLE: f32 = 0.6;
const BAR_BACKGROUND: f32 = 0.4;

/// État du défilement à la molette d'une fenêtre.
#[derive(Debug, Clone, Default)]
pub struct WheelGlide {
    /// Crans récents, pondérés par leur âge (décroissance exponentielle).
    cadence: f32,
    last_notch: f64,
    /// Position moins cible, par axe (convention de `smooth_scroll_delta`).
    error: Vec2,
    velocity: Vec2,
    /// Déplacement donné à l'image en cours (repris tel quel par une passe supplémentaire).
    out: Vec2,
    /// Dernier défilement (temps egui) : les barres restent visibles [`BAR_HOLD`] secondes.
    last_activity: Option<f64>,
}

impl WheelGlide {
    /// Remplace le défilement lissé d'egui pour cette image ; à appeler avant les zones de
    /// défilement. Demande une image tant que le glissé n'est pas fini. Règle aussi la
    /// visibilité des barres de défilement.
    pub fn frame(&mut self, ctx: &Context) {
        // Passe supplémentaire (image refaite) : mêmes entrées, même déplacement.
        if ctx.current_pass_index() > 0 {
            let out = self.out;
            ctx.input_mut(|i| i.smooth_scroll_delta = out);
            return;
        }
        let (moving, bar_left) = ctx.input_mut(|i| {
            let direct = self.take_events(i);
            // Après une pause, `stable_dt` est la durée d'image prévue, pas le temps écoulé.
            let dt = i.stable_dt.clamp(0.0, 1.0 / 30.0);
            self.out = self.step(dt) + direct;
            i.smooth_scroll_delta = self.out;
            if self.is_moving() || navigates(i) {
                self.last_activity = Some(i.time);
            }
            (self.is_moving(), self.bar_left(i.time))
        });
        if moving {
            ctx.request_repaint();
        } else if bar_left > 0.0 {
            // Une seule image différée, pour lancer le fondu.
            ctx.request_repaint_after(Duration::from_secs_f64(bar_left));
        }
        let shown =
            ctx.animate_bool_with_time(Id::unique("fx-scroll-bars"), bar_left > 0.0, BAR_FADE);
        set_bar_opacity(ctx, shown);
    }

    /// Temps restant (s) avant l'effacement des barres ; 0 : effacées.
    fn bar_left(&self, now: f64) -> f64 {
        self.last_activity
            .map_or(0.0, |t| (BAR_HOLD - (now - t)).max(0.0))
    }

    /// Lit les événements de molette de l'image ; renvoie les déplacements en points
    /// (appareils déjà fluides), appliqués tels quels.
    fn take_events(&mut self, i: &InputState) -> Vec2 {
        let mut direct = Vec2::ZERO;
        for e in &i.raw.events {
            let Event::MouseWheel {
                unit,
                delta,
                phase: TouchPhase::Move,
                modifiers,
                ..
            } = e
            else {
                continue;
            };
            // Ctrl + molette : zoom (egui et `view::wheel_zoom`).
            if modifiers.command {
                continue;
            }
            self.last_activity = Some(i.time);
            // Mêmes conventions qu'egui : Maj horizontal, Alt vertical.
            let delta = match (modifiers.shift, modifiers.alt) {
                (true, false) => vec2(delta.x + delta.y, 0.0),
                (false, true) => vec2(0.0, delta.x + delta.y),
                _ => *delta,
            };
            match unit {
                MouseWheelUnit::Point => direct += delta,
                MouseWheelUnit::Line => self.notch(i.time, delta),
                MouseWheelUnit::Page => self.push(delta * i.viewport_rect().height()),
            }
        }
        direct
    }

    /// Un événement de molette en crans (`lines`, winit : un cran = une ligne).
    fn notch(&mut self, time: f64, lines: Vec2) {
        let count = lines.x.abs() + lines.y.abs();
        // Crans entiers (molette crantée) : accélérés ; fractions (molette libre, pavé
        // tactile) : la vitesse suit déjà le geste.
        let gain = if count >= 0.99 {
            if self.reverses(lines) {
                self.cadence = 0.0;
            }
            let decay = (-(time - self.last_notch).max(0.0) / TAU).exp() as f32;
            self.cadence = self.cadence * decay + count;
            self.last_notch = time;
            (1.0 + GAIN * (self.cadence - CALM)).clamp(1.0, MAX_GAIN)
        } else {
            1.0
        };
        self.push(lines * BASE * gain);
    }

    /// Ajoute une distance à parcourir. Un changement de sens arrête net le glissé en cours.
    fn push(&mut self, delta: Vec2) {
        if self.reverses(delta) {
            self.error = Vec2::ZERO;
            self.velocity = Vec2::ZERO;
        }
        self.error -= delta;
    }

    /// `delta` va à l'opposé du mouvement en cours (reste à parcourir ou vitesse) ?
    fn reverses(&self, delta: Vec2) -> bool {
        let ahead = -self.error;
        delta.dot(ahead) < 0.0 || delta.dot(self.velocity) < 0.0
    }

    /// Avance le ressort de `dt` secondes ; renvoie le déplacement de l'image.
    fn step(&mut self, dt: f32) -> Vec2 {
        if !self.is_moving() {
            return Vec2::ZERO;
        }
        // Solution exacte du ressort à amortissement critique : indépendante de la cadence
        // d'affichage, sans dépassement de la cible.
        let decay = (-OMEGA * dt).exp();
        let mut moved = Vec2::ZERO;
        for d in 0..2 {
            let (e, v) = (self.error[d], self.velocity[d]);
            let k = v + OMEGA * e;
            let e2 = (e + k * dt) * decay;
            let v2 = (v - OMEGA * k * dt) * decay;
            if e2.abs() < SETTLE_DIST && v2.abs() < SETTLE_VEL {
                moved[d] = -e;
                self.error[d] = 0.0;
                self.velocity[d] = 0.0;
            } else {
                moved[d] = e2 - e;
                self.error[d] = e2;
                self.velocity[d] = v2;
            }
        }
        moved
    }

    fn is_moving(&self) -> bool {
        self.error != Vec2::ZERO || self.velocity != Vec2::ZERO
    }
}

/// Touches qui font défiler une liste.
fn navigates(i: &InputState) -> bool {
    i.events.iter().any(|e| {
        matches!(
            e,
            Event::Key {
                key: Key::ArrowUp
                    | Key::ArrowDown
                    | Key::ArrowLeft
                    | Key::ArrowRight
                    | Key::PageUp
                    | Key::PageDown
                    | Key::Home
                    | Key::End,
                pressed: true,
                ..
            }
        )
    })
}

/// Opacité des barres de défilement au-dessus de leur zone (`shown` dans [0, 1]). Hors de la
/// zone elles sont déjà invisibles ; survolées, egui les affiche d'après `interact_*`.
fn set_bar_opacity(ctx: &Context, shown: f32) {
    let (handle, background) = (BAR_HANDLE * shown, BAR_BACKGROUND * shown);
    let current = ctx.global_style().spacing.scroll;
    if current.active_handle_opacity != handle || current.active_background_opacity != background {
        ctx.all_styles_mut(|s| {
            s.spacing.scroll.active_handle_opacity = handle;
            s.spacing.scroll.active_background_opacity = background;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egui;

    const DT: f32 = 1.0 / 60.0;

    /// Crans vers le bas espacés de `interval` secondes, image par image (60 Hz) ;
    /// renvoie le déplacement de chaque image.
    fn run(notches: usize, interval: f64, tail: f64) -> Vec<f32> {
        let mut g = WheelGlide::default();
        let mut frames = Vec::new();
        let mut next = 0.0;
        let mut sent = 0;
        let end = interval * notches as f64 + tail;
        let mut t = 0.0;
        while t < end {
            while sent < notches && next <= t {
                g.notch(t, vec2(0.0, -1.0));
                sent += 1;
                next += interval;
            }
            frames.push(g.step(DT).y);
            t += f64::from(DT);
        }
        frames
    }

    fn total(frames: &[f32]) -> f32 {
        -frames.iter().sum::<f32>()
    }

    #[test]
    fn slow_notches_move_the_base_distance() {
        let frames = run(5, 0.4, 0.5);
        assert!(
            (total(&frames) - 5.0 * BASE).abs() < 0.01,
            "{}",
            total(&frames)
        );
    }

    #[test]
    fn fast_notches_accelerate_progressively() {
        // Même nombre de crans, cadences croissantes : distance croissante.
        let per_notch = |interval| total(&run(10, interval, 1.0)) / 10.0;
        let (calm, brisk, fast, flick) = (
            per_notch(0.3),
            per_notch(0.08),
            per_notch(0.04),
            per_notch(0.015),
        );
        assert!((calm - BASE).abs() < 0.01, "{calm}");
        assert!(
            calm < brisk && brisk < fast && fast < flick,
            "{calm} {brisk} {fast} {flick}"
        );
        assert!(fast > 2.5 * BASE, "{fast}");
        assert!(flick <= MAX_GAIN * BASE + 0.01, "{flick}");
    }

    #[test]
    fn glide_has_no_jolt_and_stops_by_itself() {
        let frames = run(1, 1.0, 0.0);
        // Pas de saut au premier cran : le ressort part de la vitesse actuelle (nulle).
        assert!(-frames[0] < 0.15 * BASE, "{}", frames[0]);
        // Mouvement dans un seul sens, puis arrêt complet en moins de 0,3 s.
        assert!(frames.iter().all(|&d| d <= 0.0));
        let moving = frames.iter().filter(|&&d| d != 0.0).count();
        assert!((moving as f32) * DT < 0.3, "{moving} images");
    }

    #[test]
    fn continuous_wheel_keeps_a_smooth_speed() {
        // 20 crans/s : la vitesse d'une image à l'autre ne varie que progressivement.
        let frames = run(40, 0.05, 0.0);
        let steady = &frames[30..frames.len() - 5];
        for w in steady.windows(2) {
            let (a, b) = (-w[0], -w[1]);
            assert!((a - b).abs() < 0.25 * a.max(b), "{a} → {b}");
        }
    }

    #[test]
    fn reversing_stops_at_once() {
        let mut g = WheelGlide::default();
        for k in 0..5 {
            g.notch(f64::from(k) * 0.02, vec2(0.0, -1.0));
            g.step(DT);
        }
        g.notch(0.12, vec2(0.0, 1.0));
        // Plus rien vers le bas : on repart vers le haut, sans accélération héritée.
        let mut up = 0.0;
        for _ in 0..60 {
            let d = g.step(DT).y;
            assert!(d >= 0.0, "{d}");
            up += d;
        }
        assert!((up - BASE).abs() < 0.01, "{up}");
    }

    #[test]
    fn fractional_steps_are_not_accelerated() {
        let mut g = WheelGlide::default();
        let mut moved = 0.0;
        for k in 0..20 {
            g.notch(f64::from(k) * 0.008, vec2(0.0, -0.1));
            moved -= g.step(0.008).y;
        }
        for _ in 0..100 {
            moved -= g.step(DT).y;
        }
        assert!((moved - 2.0 * BASE).abs() < 0.01, "{moved}");
    }

    #[test]
    fn replaces_egui_scroll_delta_except_zoom() {
        let wheel = |modifiers| Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers,
        };
        let ctx = Context::default();
        let mut g = WheelGlide::default();
        let mut frame = |time: f64, events: Vec<Event>| {
            let mut seen = Vec2::ZERO;
            let input = egui::RawInput {
                time: Some(time),
                events,
                ..Default::default()
            };
            let output = ctx.run_ui(input, |ui| {
                g.frame(ui.ctx());
                seen = ui.input(|i| i.smooth_scroll_delta);
            });
            output.drop_without_applying_deltas();
            seen
        };
        let mut moved = frame(0.0, vec![wheel(egui::Modifiers::NONE)]).y;
        for k in 1..60 {
            moved += frame(f64::from(k) / 60.0, Vec::new()).y;
        }
        assert!((moved + BASE).abs() < 0.01, "{moved}");
        // Ctrl + molette : rien pour le défilement.
        let zoom = frame(2.0, vec![wheel(egui::Modifiers::COMMAND)]);
        assert_eq!(zoom, Vec2::ZERO);
    }

    #[test]
    fn scroll_bars_fade_after_scrolling_stops() {
        let ctx = Context::default();
        let mut g = WheelGlide::default();
        let mut frame = |time: f64, events: Vec<Event>| {
            let input = egui::RawInput {
                time: Some(time),
                events,
                ..Default::default()
            };
            let output = ctx.run_ui(input, |ui| g.frame(ui.ctx()));
            output.drop_without_applying_deltas();
            ctx.global_style().spacing.scroll.active_handle_opacity
        };
        // Rien n'a défilé : barres effacées.
        assert_eq!(frame(0.0, Vec::new()), 0.0);
        let notch = Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        };
        frame(1.0, vec![notch]);
        let mut t = 1.0;
        let mut opacity = 0.0;
        // Visibles pendant BAR_HOLD secondes après le dernier mouvement…
        while t < 1.0 + BAR_HOLD {
            t += 1.0 / 60.0;
            opacity = frame(t, Vec::new());
            if t > 1.3 {
                assert!((opacity - BAR_HANDLE).abs() < 1e-3, "{t} : {opacity}");
            }
        }
        assert!(opacity > 0.0);
        // … puis effacées en un bref fondu.
        for _ in 0..30 {
            t += 1.0 / 60.0;
            opacity = frame(t, Vec::new());
        }
        assert_eq!(opacity, 0.0);
    }
}
