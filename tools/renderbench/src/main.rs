//! Banc de rendu hors écran : la vraie vue fx (barre, liste Détails, barre d'état) sur un
//! dossier réel, en défilement continu, à plusieurs résolutions.
//!
//! ```text
//! renderbench --tree bench-data/100k [--frames 300]
//! renderbench --tree bench-data/100k --verify [--frames 600]
//! cargo run --release -p renderbench --features rayon -- --tree bench-data/100k
//! ```
//!
//! Pour chaque image : passe egui (`run_ui`) et tessellation — communes aux moteurs CPU et
//! GPU — puis rasterisation CPU. La présentation à l'écran n'est pas incluse (voir
//! `memprobe --scroll` pour la mesure en fenêtre).
//!
//! Trois modes de rasterisation : avec cache de primitives, complet (sans cache, toute la
//! fenêtre à chaque image, comme fx avant le rendu partiel) et partiel (`render_damaged` :
//! seules les zones modifiées sont rasterisées, le contenu qui défile est recopié). Le mode
//! avec cache efface le tampon à chaque image (il compose son canevas par-dessus) ; les deux
//! autres non, comme fx.
//!
//! `--disk <dossier>` : vue « Analyser l'espace » (cf. `disk.rs`).
//!
//! `--verify` : défilements variés (bas, haut, petits pas, arrêts), survol, clics, Ctrl+A et
//! menu contextuel simulés ; chaque image du mode partiel est comparée pixel à pixel à un
//! rendu complet. Code de sortie 1 au moindre écart.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use fx_app::{AppState, Effect, Msg, update};
use fx_ui::egui::{
    self, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, ViewportId, pos2, vec2,
};

const WARMUP: usize = 30;
const STEP_POINTS: f32 = 84.0;

struct Res {
    w: usize,
    h: usize,
    ppp: f32,
}

const RESOLUTIONS: [Res; 4] = [
    Res {
        w: 1920,
        h: 1080,
        ppp: 1.0,
    },
    Res {
        w: 2560,
        h: 1440,
        ppp: 1.0,
    },
    Res {
        w: 3840,
        h: 2160,
        ppp: 1.5,
    },
    Res {
        w: 3840,
        h: 2160,
        ppp: 1.0,
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Cache,
    Full,
    Partial,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Self::Cache => "cache",
            Self::Full => "complet",
            Self::Partial => "partiel",
        }
    }
}

mod disk;

fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--disk") {
        return disk::main();
    }
    let (tree, frames, verify) = parse_args()?;
    if verify {
        return verify_all(&tree, frames);
    }
    let rayon = cfg!(feature = "rayon");
    println!(
        "renderbench : {} — {frames} images par configuration, rayon {}",
        tree.display(),
        if rayon { "activé" } else { "désactivé" }
    );
    println!(
        "{:<11} {:>4} {:>7} | {:>13} | {:>13} | {:>13} | {:>13} | {:>8}",
        "résolution",
        "ppp",
        "mode",
        "egui p50/p95",
        "tess p50/p95",
        "raster p50/p95",
        "total p50/p95",
        "Δ mém"
    );
    for res in &RESOLUTIONS {
        for mode in [Mode::Cache, Mode::Full, Mode::Partial] {
            let r = run(&tree, res, mode, frames)?;
            println!(
                "{:<11} {:>4.1} {:>7} | {:>5.2} / {:>5.2} | {:>5.2} / {:>5.2} | {:>5.2} / {:>5.2} | {:>5.2} / {:>5.2} | {:>5.1} Mo",
                format!("{}x{}", res.w, res.h),
                res.ppp,
                mode.label(),
                r.egui[0],
                r.egui[1],
                r.tess[0],
                r.tess[1],
                r.raster[0],
                r.raster[1],
                r.total[0],
                r.total[1],
                r.mem_delta_mb,
            );
        }
    }
    Ok(())
}

struct Report {
    egui: [f64; 2],
    tess: [f64; 2],
    raster: [f64; 2],
    total: [f64; 2],
    mem_delta_mb: f64,
}

fn raw_input(res: &Res, frame: usize) -> RawInput {
    let screen = Rect::from_min_size(
        Pos2::ZERO,
        vec2(res.w as f32 / res.ppp, res.h as f32 / res.ppp),
    );
    let mut raw = RawInput {
        screen_rect: Some(screen),
        time: Some(frame as f64 / 120.0),
        ..Default::default()
    };
    raw.viewports
        .entry(ViewportId::ROOT)
        .or_default()
        .native_pixels_per_point = Some(res.ppp);
    raw
}

fn run(tree: &Path, res: &Res, mode: Mode, frames: usize) -> Result<Report> {
    let before = private_mb();
    let ctx = egui::Context::default();
    let mut harness = fx_ui::offscreen::Harness::new(&ctx, load(tree)?);
    let mut renderer =
        EguiSoftwareRender::new(ColorFieldOrder::Bgra).with_caching(mode == Mode::Cache);
    let mut buffer = vec![[0u8; 4]; res.w * res.h];

    let (mut e, mut t, mut r, mut tot) = (vec![], vec![], vec![], vec![]);
    let mut peak_mem = before;
    for i in 0..WARMUP + frames {
        harness.scroll(STEP_POINTS);
        let raw = raw_input(res, i);

        let t0 = Instant::now();
        let mut out = ctx.run_ui(raw, |ui| harness.ui(ui));
        let t1 = Instant::now();
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        let t2 = Instant::now();
        if mode == Mode::Cache {
            buffer.fill([0; 4]);
        }
        let mut target = BufferMutRef::new(&mut buffer, res.w, res.h);
        if mode == Mode::Partial {
            renderer.render_damaged(
                &mut target,
                &prims,
                &out.textures_delta,
                out.pixels_per_point,
                true,
                false,
            );
        } else {
            renderer.render(
                &mut target,
                &prims,
                &out.textures_delta,
                out.pixels_per_point,
            );
        }
        let t3 = Instant::now();
        // Appliqué par le moteur ; egui exige qu'on le vide (assertion en debug).
        out.textures_delta.clear();

        if i >= WARMUP {
            e.push(ms(t1 - t0));
            t.push(ms(t2 - t1));
            r.push(ms(t3 - t2));
            tot.push(ms(t3 - t0));
        }
        if i == WARMUP + frames - 1 {
            peak_mem = private_mb();
        }
    }
    Ok(Report {
        egui: p50_p95(&mut e),
        tess: p50_p95(&mut t),
        raster: p50_p95(&mut r),
        total: p50_p95(&mut tot),
        mem_delta_mb: peak_mem - before,
    })
}

/// Rendu partiel comparé image par image à un rendu complet, à plusieurs résolutions et
/// échelles (dont des échelles fractionnaires).
fn verify_all(tree: &Path, frames: usize) -> Result<()> {
    let extra = Res {
        w: 1600,
        h: 900,
        ppp: 1.25,
    };
    let mut ok = true;
    for res in RESOLUTIONS[..3].iter().chain([&extra]) {
        ok &= verify(tree, res, frames)?;
    }
    if !ok {
        bail!("le rendu partiel diffère du rendu complet");
    }
    println!(
        "rendu partiel identique au rendu complet (à {TOLERANCE} niveaux près) sur toutes les images"
    );
    Ok(())
}

/// Écart toléré par canal : un rectangle redessiné découpe les primitives texturées ailleurs
/// qu'un rendu complet, et l'interpolation des UV (pas à pas depuis le bord découpé) peut
/// différer d'un niveau. Un défaut réel (contenu périmé, mauvais décalage) donne des écarts
/// bien plus grands.
const TOLERANCE: u8 = 2;

/// Pas de défilement (points) : bas, haut, petits pas, pas fractionnaires, arrêts.
const VERIFY_STEPS: [f32; 16] = [
    84.0, 84.0, 30.0, -45.0, 0.0, 3.0, -84.0, 120.0, 1.0, 0.0, 0.0, -2.0, 60.0, 17.5, -30.0, 0.0,
];

fn verify(tree: &Path, res: &Res, frames: usize) -> Result<bool> {
    let ctx = egui::Context::default();
    let mut harness = fx_ui::offscreen::Harness::new(&ctx, load(tree)?);
    let mut partial = EguiSoftwareRender::new(ColorFieldOrder::Bgra).with_caching(false);
    let mut full = EguiSoftwareRender::new(ColorFieldOrder::Bgra).with_caching(false);
    let mut a = vec![[0u8; 4]; res.w * res.h];
    let mut b = vec![[0u8; 4]; res.w * res.h];
    let size = vec2(res.w as f32 / res.ppp, res.h as f32 / res.ppp);

    let (mut bad_frames, mut scrolled, mut partial_frames, mut redrawn) = (0, 0, 0, 0u64);
    let mut max_dev = 0u8;
    for i in 0..frames {
        let step = VERIFY_STEPS[i % VERIFY_STEPS.len()];
        harness.scroll(step);
        let mut raw = raw_input(res, i);
        raw.events = input_script(i, size);
        let mut out = ctx.run_ui(raw, |ui| harness.ui_interactive(ui));
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);

        partial.render_damaged(
            &mut BufferMutRef::new(&mut a, res.w, res.h),
            &prims,
            &out.textures_delta,
            out.pixels_per_point,
            true,
            false,
        );
        full.render(
            &mut BufferMutRef::new(&mut b, res.w, res.h),
            &prims,
            &out.textures_delta,
            out.pixels_per_point,
        );
        out.textures_delta.clear();
        let report = partial.last_damage();
        if !report.full {
            partial_frames += 1;
            redrawn += report.redrawn_px;
        }
        if report.scroll.is_some() {
            scrolled += 1;
        }
        let mut n = 0usize;
        let mut worst = 0u8;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for (k, (pa, pb)) in a.iter().zip(&b).enumerate() {
            let d = (0..4).map(|c| pa[c].abs_diff(pb[c])).max().unwrap_or(0);
            worst = worst.max(d);
            if d > TOLERANCE {
                let (x, y) = (k % res.w, k / res.w);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                n += 1;
            }
        }
        max_dev = max_dev.max(worst);
        if n > 0 {
            bad_frames += 1;
            if bad_frames <= 5 {
                println!(
                    "  image {i} : {n} pixels différents dans [{x0},{y0}]-[{x1},{y1}] (pas {step}, plein {}, décalage {:?})",
                    report.full, report.scroll
                );
            }
            // Repart d'une image juste pour isoler les écarts suivants.
            a.copy_from_slice(&b);
        }
    }
    let px = (res.w * res.h) as f64;
    let mean = if partial_frames == 0 {
        0.0
    } else {
        redrawn as f64 / f64::from(partial_frames) / px * 100.0
    };
    println!(
        "{}x{} @{} : {frames} images, {partial_frames} partielles ({scrolled} avec décalage), {mean:.1} % des pixels rasterisés en moyenne sur les partielles, écart max {max_dev}, {bad_frames} image(s) fausse(s)",
        res.w, res.h, res.ppp,
    );
    Ok(bad_frames == 0)
}

/// Souris et clavier simulés : survol qui balaie la liste, clics de sélection, Ctrl+A, menu
/// contextuel ouvert pendant le défilement puis fermé.
fn input_script(i: usize, size: egui::Vec2) -> Vec<Event> {
    let mut ev = Vec::new();
    let phase = (i % 240) as f32 / 240.0;
    let p = pos2(
        size.x * (0.25 + 0.5 * phase),
        size.y * (0.2 + 0.7 * ((i % 37) as f32 / 37.0)),
    );
    if i % 90 < 80 {
        ev.push(Event::PointerMoved(p));
    } else {
        ev.push(Event::PointerGone);
    }
    let click = |ev: &mut Vec<Event>, button, pressed| {
        ev.push(Event::PointerButton {
            pos: p,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        });
    };
    match i % 120 {
        10 | 55 => click(&mut ev, PointerButton::Primary, true),
        11 | 56 => click(&mut ev, PointerButton::Primary, false),
        70 => click(&mut ev, PointerButton::Secondary, true),
        71 => click(&mut ev, PointerButton::Secondary, false),
        100 => ev.push(key(Key::Escape, Modifiers::NONE)),
        _ => {}
    }
    if i % 300 == 150 {
        ev.push(key(Key::A, Modifiers::COMMAND));
    }
    ev
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// Charge le dossier de façon synchrone, par les mêmes messages que l'application.
fn load(tree: &Path) -> Result<AppState> {
    let mut state = AppState::new(
        fx_store::Settings::default(),
        fx_app::Tags::default(),
        Vec::new(),
    );
    for effect in update(
        &mut state,
        Msg::Navigate(fx_app::Location::Dir(tree.to_path_buf())),
    ) {
        let Effect::Enumerate {
            pane,
            generation,
            path,
            options,
            cancel,
            ..
        } = effect
        else {
            continue;
        };
        let mut batches = Vec::new();
        let result = fx_fs::enumerate(&path, &options, &cancel, |b| batches.push(b))
            .map_err(|e| e.message().trim().to_owned());
        for batch in batches {
            update(
                &mut state,
                Msg::ListingBatch {
                    pane,
                    generation,
                    batch,
                },
            );
        }
        update(
            &mut state,
            Msg::ListingFinished {
                pane,
                generation,
                result,
            },
        );
    }
    let rows = state.pane().listing.as_ref().map_or(0, |l| l.len());
    if rows == 0 {
        bail!("{} : aucun élément chargé", tree.display());
    }
    Ok(state)
}

fn p50_p95(v: &mut [f64]) -> [f64; 2] {
    v.sort_by(f64::total_cmp);
    let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    [q(0.50), q(0.95)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn private_mb() -> f64 {
    fx_win::process::current_memory().map_or(f64::NAN, |m| m.private_bytes as f64 / 1_048_576.0)
}

fn parse_args() -> Result<(PathBuf, usize, bool)> {
    let mut tree = None;
    let mut frames = 300;
    let mut verify = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--tree" => {
                tree = Some(PathBuf::from(
                    it.next().context("--tree attend un dossier")?,
                ))
            }
            "--frames" => frames = it.next().context("--frames attend un nombre")?.parse()?,
            "--verify" => verify = true,
            other => bail!("option inconnue : {other}"),
        }
    }
    let tree = tree.context("--tree <dossier> est requis")?;
    Ok((std::path::absolute(tree)?, frames, verify))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le rendu partiel (zones modifiées, défilement par recopie) donne la même image qu'un
    /// rendu complet, à chaque image d'un scénario de défilement, survol, sélection et menu.
    #[test]
    fn partial_render_matches_full_render() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("fx-renderbench-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("dossier"))?;
        for i in 0..1500 {
            let name = format!("Fichier {i:04} — élément ({}).txt", i * 37 % 1000);
            std::fs::write(dir.join(name), vec![b'x'; i % 4096])?;
        }
        // Sans optimisations (`cargo test` par défaut), egui et le rasteriseur sont ~30 fois
        // plus lents : scénario raccourci (défilements, survol, clics et menu contextuel
        // restent couverts).
        let (w, h, frames) = if cfg!(debug_assertions) {
            (800, 520, [80, 40])
        } else {
            (1280, 800, [160, 160])
        };
        let mut ok = true;
        for (ppp, frames) in [1.0, 1.25].into_iter().zip(frames) {
            ok &= verify(&dir, &Res { w, h, ppp }, frames)?;
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ok, "le rendu partiel diffère du rendu complet");
        Ok(())
    }
}
