//! `renderbench --disk <dossier> [--frames 300] [--tab contenu|types|recuperable]
//! [--snapshot capture.png] [--size 1366x768]` : analyse réellement `<dossier>` (comme
//! « Analyser l'espace »),
//! puis mesure le rendu de la vue d'analyse (carte proportionnelle et liste) pendant que le
//! pointeur parcourt la carte (survol, info-bulle), en rendu complet et partiel. Avec
//! `--snapshot`, enregistre la première image mesurée en 1920×1080 (PNG sans compression).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use fx_app::disk::DiskTab;
use fx_app::{AppState, Effect, Msg, update};
use fx_ui::egui::{self, Event, Pos2, RawInput, Rect, ViewportId, pos2, vec2};

use crate::{ms, p50_p95};

const WARMUP: usize = 10;

struct Args {
    root: PathBuf,
    frames: usize,
    tab: DiskTab,
    snapshot: Option<PathBuf>,
    /// `--size LxH` : une seule taille (capture comprise) au lieu de 1080p et 1440p.
    size: Option<(usize, usize)>,
    /// `--menu` : menu « Analyser des lecteurs » ouvert (capture).
    menu: bool,
}

pub fn main() -> Result<()> {
    let args = parse_args()?;
    let t = Instant::now();
    let state = load(&args.root, args.tab, false)?;
    let scan = &state.disk_scans[0];
    let tree = scan.tree.as_ref().context("analyse vide")?;
    println!(
        "analyse de {} : {} nœuds, {:.1} Mo d'arbre, {} suggestions, {:.1} s",
        args.root.display(),
        tree.len(),
        tree.heap_bytes() as f64 / 1_048_576.0,
        scan.suggestions.len(),
        t.elapsed().as_secs_f64()
    );
    drop(state);
    println!(
        "{:<11} {:>7} | {:>13} | {:>13} | {:>13} | {:>13}",
        "résolution", "mode", "egui p50/p95", "tess p50/p95", "raster p50/p95", "total p50/p95"
    );
    let sizes = match args.size {
        Some(s) => vec![s],
        None => vec![(1920, 1080), (2560, 1440)],
    };
    let first = sizes[0];
    for (w, h) in sizes {
        for partial in [false, true] {
            let snapshot = ((w, h) == first && !partial)
                .then_some(args.snapshot.as_deref())
                .flatten();
            let state = load(&args.root, args.tab, args.menu)?;
            let r = run(state, w, h, partial, args.frames, snapshot)?;
            println!(
                "{:<11} {:>7} | {:>5.2} / {:>5.2} | {:>5.2} / {:>5.2} | {:>5.2} / {:>5.2} | {:>5.2} / {:>5.2}",
                format!("{w}x{h}"),
                if partial { "partiel" } else { "complet" },
                r[0][0],
                r[0][1],
                r[1][0],
                r[1][1],
                r[2][0],
                r[2][1],
                r[3][0],
                r[3][1],
            );
        }
    }
    Ok(())
}

/// Analyse puis liste du dossier, de façon synchrone, par les messages de l'application.
fn load(root: &Path, tab: DiskTab, menu: bool) -> Result<AppState> {
    let mut state = AppState::new(
        fx_store::Settings::default(),
        fx_app::Tags::default(),
        Vec::new(),
    );
    state.drives = fx_win::sys::drives();
    let mut queue: VecDeque<Effect> =
        update(&mut state, Msg::AnalyzeSpace(root.to_path_buf())).into();
    while let Some(effect) = queue.pop_front() {
        match effect {
            Effect::ScanDisk {
                scan,
                root,
                keep_from,
                cancel,
            } => {
                let tree = fx_fs::diskscan::scan(
                    &root,
                    keep_from,
                    fx_fs::diskscan::MAX_THREADS,
                    &cancel,
                    Duration::MAX,
                    |_| {},
                );
                let result = tree.map(|t| {
                    let s = fx_fs::cleanup::suggest(&t, &fx_fs::cleanup::Places::current());
                    Box::new((t, s))
                });
                queue.extend(update(&mut state, Msg::DiskScanned { scan, result }));
            }
            Effect::Enumerate {
                pane,
                generation,
                path,
                options,
                cancel,
                ..
            } => {
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
            _ => {}
        }
    }
    if state
        .disk_scans
        .first()
        .and_then(|s| s.tree.as_ref())
        .is_none()
    {
        bail!("{} : analyse impossible", root.display());
    }
    let pane = state.pane().id;
    update(&mut state, Msg::DiskTab { pane, tab });
    if menu {
        update(
            &mut state,
            Msg::Command(fx_commands::CommandId::AnalyzeDrives),
        );
    }
    Ok(state)
}

fn run(
    state: AppState,
    w: usize,
    h: usize,
    partial: bool,
    frames: usize,
    snapshot: Option<&Path>,
) -> Result<[[f64; 2]; 4]> {
    let ctx = egui::Context::default();
    let mut harness = fx_ui::offscreen::Harness::new(&ctx, state);
    let mut renderer = EguiSoftwareRender::new(ColorFieldOrder::Bgra).with_caching(false);
    let mut buffer = vec![[0u8; 4]; w * h];
    let (mut e, mut t, mut r, mut tot) = (vec![], vec![], vec![], vec![]);
    for i in 0..WARMUP + frames {
        // Le pointeur balaie la carte (haut du volet) en zigzag.
        let k = i.saturating_sub(WARMUP) as f32;
        let pointer = pos2(
            320.0 + (k * 23.0) % (w as f32 - 400.0),
            230.0 + (k * 7.0) % 260.0,
        );
        let mut raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32))),
            time: Some(i as f64 / 120.0),
            ..Default::default()
        };
        raw.viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(1.0);
        if i >= WARMUP {
            raw.events.push(Event::PointerMoved(pointer));
        }
        let t0 = Instant::now();
        let mut out = ctx.run_ui(raw, |ui| harness.ui(ui));
        let t1 = Instant::now();
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        let t2 = Instant::now();
        let mut target = BufferMutRef::new(&mut buffer, w, h);
        if partial {
            renderer.render_damaged(&mut target, &prims, &out.textures_delta, 1.0, true, false);
        } else {
            renderer.render(&mut target, &prims, &out.textures_delta, 1.0);
        }
        let t3 = Instant::now();
        out.textures_delta.clear();
        if i == WARMUP
            && let Some(p) = snapshot
        {
            write_png(p, w, h, &buffer)?;
            println!("capture : {}", p.display());
        }
        if i >= WARMUP {
            e.push(ms(t1 - t0));
            t.push(ms(t2 - t1));
            r.push(ms(t3 - t2));
            tot.push(ms(t3 - t0));
        }
    }
    Ok([
        p50_p95(&mut e),
        p50_p95(&mut t),
        p50_p95(&mut r),
        p50_p95(&mut tot),
    ])
}

fn parse_args() -> Result<Args> {
    let mut a = Args {
        root: PathBuf::new(),
        frames: 300,
        tab: DiskTab::Content,
        snapshot: None,
        size: None,
        menu: false,
    };
    let mut root = None;
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--disk" => {
                root = Some(PathBuf::from(
                    it.next().context("--disk attend un dossier")?,
                ))
            }
            "--frames" => a.frames = it.next().context("--frames attend un nombre")?.parse()?,
            "--snapshot" => {
                a.snapshot = Some(it.next().context("--snapshot attend un fichier")?.into())
            }
            "--menu" => a.menu = true,
            "--size" => {
                let s = it.next().context("--size attend LxH (ex. 1366x768)")?;
                let (w, h) = s.split_once('x').context("--size attend LxH")?;
                a.size = Some((w.parse()?, h.parse()?));
            }
            "--tab" => {
                a.tab = match it.next().context("--tab attend une page")?.as_str() {
                    "contenu" => DiskTab::Content,
                    "types" => DiskTab::Types,
                    "recuperable" => DiskTab::Cleanup,
                    other => bail!("page inconnue : {other} (contenu, types, recuperable)"),
                }
            }
            other => bail!("option inconnue : {other}"),
        }
    }
    a.root = std::path::absolute(root.context("--disk <dossier> est requis")?)?;
    Ok(a)
}

/// PNG RGBA sans compression (blocs « stored ») : de quoi regarder une capture sans
/// dépendance.
fn write_png(path: &Path, w: usize, h: usize, bgra: &[[u8; 4]]) -> Result<()> {
    let mut raw = Vec::with_capacity(h * (w * 4 + 1));
    for row in bgra.chunks(w) {
        raw.push(0);
        for &[b, g, r, _] in row {
            raw.extend_from_slice(&[r, g, b, 255]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65_535).peekable();
    while let Some(c) = chunks.next() {
        z.push(u8::from(chunks.peek().is_none()));
        let n = c.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    for (kind, data) in [(&b"IHDR"[..], &ihdr[..]), (b"IDAT", &z), (b"IEND", &[])] {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let crc = crc32(&png[start..]);
        png.extend_from_slice(&crc.to_be_bytes());
    }
    std::fs::write(path, png).with_context(|| path.display().to_string())
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &x in data {
        crc ^= u32::from(x);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
