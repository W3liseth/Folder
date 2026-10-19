//! Mesure fx face aux budgets d'ARCHITECTURE.md §2.
//!
//! ```text
//! memprobe --exe target/release/folder.exe --tree bench-data/100k
//!          [--renderer cpu|glow|wgpu] [--runs 3] [--settle-ms 1500] [--max-private-mb N]
//!          [--max-threads 12] [--max-first-frame-ms 150]
//! memprobe --scroll --tree bench-data/100k [--renderer cpu] [--size 1920x1080]
//!          [--max-frame-p95-ms 8]
//! ```
//!
//! Mode par défaut : lance `fx --bench-nav <tree>` (chargement à froid puis actualisation à
//! chaud), attend la ligne `FX_BENCH ready`, laisse le processus se
//! stabiliser, relève mémoire privée / working set / threads, puis le termine.
//!
//! Mode `--scroll` : lance `fx --bench-scroll <tree>`, qui fait défiler la liste en continu et
//! rapporte la distribution des temps d'image (p50/p95/p99/max) avant de se fermer.
//! Budget mémoire par défaut : 30 Mo sous 10 000 entrées, 60 Mo au-delà.
//! Code de sortie 1 si un budget est dépassé (sur la médiane des exécutions).

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

struct Config {
    exe: PathBuf,
    tree: PathBuf,
    runs: usize,
    settle: Duration,
    max_private_mb: Option<f64>,
    max_threads: u32,
    max_first_frame_ms: f64,
    renderer: Option<String>,
    size: Option<String>,
    scroll: bool,
    max_frame_p95_ms: f64,
}

#[derive(Debug, Clone)]
struct Run {
    first_frame_ms: f64,
    first_batch_ms: f64,
    listing_ms: f64,
    cold_listing_ms: f64,
    ready_wall_ms: f64,
    entries: u64,
    snapshot_kb: u64,
    private_mb: f64,
    working_set_mb: f64,
    threads: u32,
}

fn main() -> Result<()> {
    let cfg = parse_args()?;
    if cfg.scroll {
        return scroll(&cfg);
    }
    let mut runs = Vec::new();
    for i in 0..cfg.runs {
        let r = run_once(&cfg).with_context(|| format!("exécution {}", i + 1))?;
        println!(
            "run {}: première image {:.1} ms | premier lot (à chaud) {:.2} ms | énumération {:.1} ms (à froid {:.1}) | {} entrées | instantané {} Ko | privé {:.1} Mo | WS {:.1} Mo | {} threads",
            i + 1,
            r.first_frame_ms,
            r.first_batch_ms,
            r.listing_ms,
            r.cold_listing_ms,
            r.entries,
            r.snapshot_kb,
            r.private_mb,
            r.working_set_mb,
            r.threads
        );
        runs.push(r);
    }
    let m = median(&runs);
    let max_private = cfg
        .max_private_mb
        .unwrap_or(if m.entries < 10_000 { 30.0 } else { 60.0 });

    println!("\nmédiane sur {} exécution(s) :", runs.len());
    let mut ok = true;
    ok &= check("mémoire privée (Mo)", m.private_mb, max_private);
    ok &= check("threads", f64::from(m.threads), f64::from(cfg.max_threads));
    ok &= check(
        "première image (ms)",
        m.first_frame_ms,
        cfg.max_first_frame_ms,
    );
    ok &= check("premier lot à chaud (ms)", m.first_batch_ms, 16.0);
    if m.entries >= 100_000 {
        ok &= check("énumération 100k (ms)", m.listing_ms, 300.0);
    }
    println!(
        "  (info) working set {:.1} Mo, instantané {} Ko pour {} entrées",
        m.working_set_mb, m.snapshot_kb, m.entries
    );
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

fn check(label: &str, value: f64, max: f64) -> bool {
    let pass = value <= max;
    println!(
        "  {} {label:<24} {value:>8.1}  (budget ≤ {max})",
        if pass { "OK  " } else { "ÉCHEC" }
    );
    pass
}

type Lines = mpsc::Receiver<(String, Instant)>;

/// Lance fx en mode mesure ; les lignes de sa sortie standard arrivent sur le canal.
fn spawn(cfg: &Config, mode: &str) -> Result<(Child, Lines)> {
    let mut cmd = Command::new(&cfg.exe);
    cmd.arg(mode).arg(&cfg.tree);
    if let Some(r) = &cfg.renderer {
        cmd.args(["--renderer", r]);
    }
    if let Some(s) = &cfg.size {
        cmd.args(["--size", s]);
    }
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("lancement de {}", cfg.exe.display()))?;
    let stdout = child.stdout.take().context("stdout")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send((line, Instant::now())).is_err() {
                break;
            }
        }
    });
    Ok((child, rx))
}

/// Accumule les champs `clé=valeur` des lignes FX_BENCH jusqu'à celle qui commence par `last`.
fn collect(
    child: &mut Child,
    rx: &Lines,
    last: &str,
    timeout: Duration,
) -> Result<(HashMap<String, String>, Instant)> {
    let mut fields = HashMap::new();
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok((line, at)) = rx.recv_timeout(left) else {
            let _ = child.kill();
            bail!(
                "pas de ligne « FX_BENCH {last} » sous {} s",
                timeout.as_secs()
            );
        };
        let Some(rest) = line.strip_prefix("FX_BENCH ") else {
            continue;
        };
        for kv in rest.split_whitespace() {
            if let Some((k, v)) = kv.split_once('=') {
                fields.insert(k.to_owned(), v.to_owned());
            }
        }
        if rest.starts_with(last) {
            return Ok((fields, at));
        }
    }
}

fn scroll(cfg: &Config) -> Result<()> {
    let (mut child, rx) = spawn(cfg, "--bench-scroll")?;
    let (f, _) = collect(&mut child, &rx, "scroll", Duration::from_secs(180))?;
    let _ = child.wait();
    let get = |k: &str| f.get(k).map_or("?", String::as_str);
    let num = |k: &str| get(k).parse::<f64>().unwrap_or(f64::NAN);
    println!(
        "défilement {} {}x{} (ppp {}) sur {} lignes, {} images mesurées",
        get("renderer"),
        get("width"),
        get("height"),
        get("ppp"),
        get("rows"),
        get("frames")
    );
    println!(
        "  temps d'image  p50 {:.2} ms | p95 {:.2} ms | p99 {:.2} ms | max {:.2} ms",
        num("frame_p50_ms"),
        num("frame_p95_ms"),
        num("frame_p99_ms"),
        num("frame_max_ms")
    );
    println!(
        "  intervalle     p50 {:.2} ms | p95 {:.2} ms | max {:.2} ms",
        num("interval_p50_ms"),
        num("interval_p95_ms"),
        num("interval_max_ms")
    );
    println!(
        "  mémoire privée {:.1} Mo, {} threads (pendant le défilement)",
        num("private_mb"),
        get("threads")
    );
    if !check(
        "temps d'image p95 (ms)",
        num("frame_p95_ms"),
        cfg.max_frame_p95_ms,
    ) {
        std::process::exit(1);
    }
    Ok(())
}

fn run_once(cfg: &Config) -> Result<Run> {
    let start = Instant::now();
    let (mut child, rx) = spawn(cfg, "--bench-nav")?;
    let pid = child.id();
    let (fields, ready) = collect(&mut child, &rx, "ready", Duration::from_secs(60))?;
    let ready_at = Some(ready);

    std::thread::sleep(cfg.settle);
    let mem = fx_win::process::memory_of(pid).context("mémoire du processus")?;
    let threads = fx_win::process::thread_count(pid).context("threads du processus")?;
    let _ = child.kill();
    let _ = child.wait();

    let num = |k: &str| -> Result<f64> {
        fields
            .get(k)
            .with_context(|| format!("champ {k} absent"))?
            .parse::<f64>()
            .with_context(|| format!("champ {k}"))
    };
    Ok(Run {
        first_frame_ms: num("first_frame_ms")?,
        first_batch_ms: num("first_batch_ms")?,
        listing_ms: num("listing_ms")?,
        cold_listing_ms: num("cold_listing_ms")?,
        ready_wall_ms: ready_at.map_or(f64::NAN, |t| (t - start).as_secs_f64() * 1000.0),
        entries: num("entries")? as u64,
        snapshot_kb: num("snapshot_kb")? as u64,
        private_mb: mem.private_bytes as f64 / 1_048_576.0,
        working_set_mb: mem.working_set as f64 / 1_048_576.0,
        threads,
    })
}

fn median(runs: &[Run]) -> Run {
    let pick = |f: fn(&Run) -> f64| {
        let mut v: Vec<f64> = runs.iter().map(f).collect();
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    Run {
        first_frame_ms: pick(|r| r.first_frame_ms),
        first_batch_ms: pick(|r| r.first_batch_ms),
        listing_ms: pick(|r| r.listing_ms),
        cold_listing_ms: pick(|r| r.cold_listing_ms),
        ready_wall_ms: pick(|r| r.ready_wall_ms),
        entries: pick(|r| r.entries as f64) as u64,
        snapshot_kb: pick(|r| r.snapshot_kb as f64) as u64,
        private_mb: pick(|r| r.private_mb),
        working_set_mb: pick(|r| r.working_set_mb),
        threads: pick(|r| f64::from(r.threads)) as u32,
    }
}

fn parse_args() -> Result<Config> {
    let mut cfg = Config {
        exe: PathBuf::from(r"target\release\folder.exe"),
        tree: PathBuf::new(),
        runs: 3,
        settle: Duration::from_millis(1500),
        max_private_mb: None,
        max_threads: 12,
        max_first_frame_ms: 150.0,
        renderer: None,
        size: None,
        scroll: false,
        max_frame_p95_ms: 8.0,
    };
    let mut it = std::env::args_os().skip(1);
    while let Some(a) = it.next() {
        let flag = a.to_string_lossy().into_owned();
        let mut val = || {
            it.next()
                .with_context(|| format!("{flag} attend une valeur"))
        };
        match flag.as_str() {
            "--exe" => cfg.exe = val()?.into(),
            "--tree" => cfg.tree = val()?.into(),
            "--runs" => cfg.runs = val()?.to_string_lossy().parse::<usize>()?.max(1),
            "--settle-ms" => cfg.settle = Duration::from_millis(val()?.to_string_lossy().parse()?),
            "--max-private-mb" => cfg.max_private_mb = Some(val()?.to_string_lossy().parse()?),
            "--max-threads" => cfg.max_threads = val()?.to_string_lossy().parse()?,
            "--max-first-frame-ms" => cfg.max_first_frame_ms = val()?.to_string_lossy().parse()?,
            "--max-frame-p95-ms" => cfg.max_frame_p95_ms = val()?.to_string_lossy().parse()?,
            "--renderer" => cfg.renderer = Some(val()?.to_string_lossy().into_owned()),
            "--size" => cfg.size = Some(val()?.to_string_lossy().into_owned()),
            "--scroll" => cfg.scroll = true,
            other => bail!("option inconnue : {other}"),
        }
    }
    if cfg.tree.as_os_str().is_empty() {
        bail!("--tree <dossier> est requis (générez-le avec gen-tree)");
    }
    cfg.tree = std::path::absolute(&cfg.tree)?;
    Ok(cfg)
}
