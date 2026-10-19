//! Sondes de performance (`fx --bench-nav` / `fx --bench-scroll`), lues par `tools/memprobe`.
//!
//! Écrivent des lignes `FX_BENCH clé=valeur …` sur la sortie standard.

use std::io::Write;
use std::time::{Duration, Instant};

use eframe::egui::{Context, ViewportCommand, vec2};
use fx_app::{AppState, ListingStatus, Msg};
use fx_commands::CommandId;

use crate::Renderer;
use crate::view::ViewState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchMode {
    Off,
    /// Chargement à froid puis actualisation à chaud (mémoire, threads, latences).
    Nav,
    /// Défilement continu de la liste : distribution des temps d'image.
    Scroll,
}

/// Images ignorées avant la mesure (atlas de police, caches).
const WARMUP_FRAMES: usize = 30;
const MEASURED_FRAMES: usize = 600;
/// 3 lignes par image : un défilement rapide à la molette.
const SCROLL_STEP_POINTS: f32 = 84.0;

pub struct Probe {
    mode: BenchMode,
    process_start: Instant,
    renderer: Renderer,
    window_px: Option<[u32; 2]>,
    first_frame: Option<Duration>,
    /// Premier chargement (à froid, pendant l'initialisation de la fenêtre).
    cold: Option<Cold>,
    scroll: Option<ScrollRun>,
    done: bool,
}

struct Cold {
    generation: u64,
    listing: Duration,
    ready_since_start: Duration,
}

#[derive(Default)]
struct ScrollRun {
    sized: bool,
    settle_frames: u32,
    frames: usize,
    last_at: Option<Instant>,
    frame_ms: Vec<f64>,
    interval_ms: Vec<f64>,
}

impl Probe {
    pub fn new(
        mode: BenchMode,
        process_start: Instant,
        renderer: Renderer,
        window_px: Option<[u32; 2]>,
    ) -> Self {
        Self {
            mode,
            process_start,
            renderer,
            window_px,
            first_frame: None,
            cold: None,
            scroll: None,
            done: false,
        }
    }

    pub fn on_frame(&mut self) {
        if self.first_frame.is_none() {
            let t = self.process_start.elapsed();
            self.first_frame = Some(t);
            emit(&format!(
                "FX_BENCH first_frame_ms={:.1} renderer={}",
                ms(t),
                self.renderer.name()
            ));
        }
    }

    /// Après le chargement à froid : en mode `Nav`, demande une actualisation pour mesurer une
    /// navigation « à chaud » (budget du premier lot) ; en mode `Scroll`, lance le défilement.
    pub fn on_state(&mut self, state: &AppState) -> Option<Msg> {
        if self.done || self.first_frame.is_none() {
            return None;
        }
        let pane = state.pane();
        let ListingStatus::Ready { elapsed } = pane.status else {
            return None;
        };
        let listing = pane.listing.as_ref()?;
        let Some(cold) = &self.cold else {
            self.cold = Some(Cold {
                generation: listing.generation(),
                listing: elapsed,
                ready_since_start: self.process_start.elapsed(),
            });
            return match self.mode {
                BenchMode::Nav => Some(Msg::Command(CommandId::Refresh)),
                BenchMode::Scroll => {
                    self.scroll = Some(ScrollRun::default());
                    None
                }
                BenchMode::Off => None,
            };
        };
        if self.mode != BenchMode::Nav || listing.generation() == cold.generation {
            return None;
        }
        self.done = true;
        emit(&format!(
            "FX_BENCH ready entries={} first_batch_ms={:.2} listing_ms={:.1} cold_listing_ms={:.1} since_start_ms={:.1} snapshot_kb={} private_mb={:.1}",
            listing.len(),
            pane.first_batch_after.map_or(f64::NAN, ms),
            ms(elapsed),
            ms(cold.listing),
            ms(cold.ready_since_start),
            listing.heap_bytes() / 1024,
            private_mb(),
        ));
        None
    }

    /// Pilote le défilement automatique et collecte les temps d'image.
    pub fn after_frame(
        &mut self,
        ctx: &Context,
        state: &AppState,
        vs: &mut ViewState,
        last_frame: Option<Duration>,
    ) {
        if self.done {
            return;
        }
        let Some(run) = &mut self.scroll else { return };
        ctx.request_repaint();

        let ppp = ctx.pixels_per_point();
        if !run.sized {
            if let Some([w, h]) = self.window_px {
                let size = ctx.content_rect().size() * ppp;
                if (size.x - w as f32).abs() > 1.0 || (size.y - h as f32).abs() > 1.0 {
                    if run.settle_frames.is_multiple_of(30) {
                        ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(
                            w as f32 / ppp,
                            h as f32 / ppp,
                        )));
                    }
                    run.settle_frames += 1;
                    // Fenêtre plus grande que l'écran : on mesure à la taille obtenue.
                    if run.settle_frames < 120 {
                        return;
                    }
                }
            }
            run.sized = true;
        }

        let now = Instant::now();
        if run.frames > WARMUP_FRAMES {
            if let Some(t) = last_frame {
                run.frame_ms.push(ms(t));
            }
            if let Some(prev) = run.last_at {
                run.interval_ms.push(ms(now - prev));
            }
        }
        run.last_at = Some(now);
        run.frames += 1;
        let pane = state.pane();
        let rows = pane.listing.as_ref().map_or(0, |l| l.len());
        vs.bench_scroll(pane.id, SCROLL_STEP_POINTS, rows);

        if run.frames > WARMUP_FRAMES + MEASURED_FRAMES {
            self.done = true;
            let size = ctx.content_rect().size() * ppp;
            let threads = fx_win::process::thread_count(std::process::id()).unwrap_or(0);
            let (f, i) = (stats(&mut run.frame_ms), stats(&mut run.interval_ms));
            emit(&format!(
                "FX_BENCH scroll renderer={} width={:.0} height={:.0} ppp={ppp:.2} rows={rows} frames={} \
                 frame_p50_ms={:.2} frame_p95_ms={:.2} frame_p99_ms={:.2} frame_max_ms={:.2} \
                 interval_p50_ms={:.2} interval_p95_ms={:.2} interval_max_ms={:.2} private_mb={:.1} threads={threads}",
                self.renderer.name(),
                size.x,
                size.y,
                run.frame_ms.len(),
                f[0],
                f[1],
                f[2],
                f[3],
                i[0],
                i[1],
                i[3],
                private_mb(),
            ));
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}

/// [p50, p95, p99, max] ; NaN si vide.
fn stats(v: &mut [f64]) -> [f64; 4] {
    if v.is_empty() {
        return [f64::NAN; 4];
    }
    v.sort_by(f64::total_cmp);
    let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    [q(0.50), q(0.95), q(0.99), v[v.len() - 1]]
}

fn private_mb() -> f64 {
    fx_win::process::current_memory().map_or(f64::NAN, |m| m.private_bytes as f64 / 1_048_576.0)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn emit(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}
