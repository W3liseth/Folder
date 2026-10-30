//! Panneau de diagnostics (F12) : mémoire, threads, caches, latences.

use std::time::{Duration, Instant};

use eframe::egui::{self, pos2};

use super::Fx;

pub struct Sample {
    sampled: Option<Instant>,
    private_mb: f64,
    working_set_mb: f64,
    threads: u32,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            sampled: None,
            private_mb: 0.0,
            working_set_mb: 0.0,
            threads: 0,
        }
    }
}

pub fn draw(ctx: &egui::Context, f: &mut Fx<'_>) {
    let d = &mut f.vs.diag;
    if d.sampled
        .is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
    {
        d.sampled = Some(Instant::now());
        if let Ok(m) = fx_win::process::current_memory() {
            d.private_mb = m.private_bytes as f64 / 1_048_576.0;
            d.working_set_mb = m.working_set as f64 / 1_048_576.0;
        }
        d.threads = fx_win::process::thread_count(std::process::id()).unwrap_or(0);
    }
    // Rafraîchissement lent tant que le panneau est ouvert, rien sinon.
    ctx.request_repaint_after(Duration::from_secs(1));
    let (private_mb, ws, threads) = (d.private_mb, d.working_set_mb, d.threads);
    let pane = f.state.pane();
    let listing = pane.listing.as_ref();
    let (icons, thumbs) = f.images.bytes();
    let mut open = true;
    egui::Window::new("Diagnostics")
        .open(&mut open)
        .default_pos(pos2(60.0, 160.0))
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("diag").num_columns(2).show(ui, |ui| {
                let mut kv = |k: &str, v: String| {
                    ui.label(k);
                    ui.monospace(v);
                    ui.end_row();
                };
                kv("Mémoire privée", format!("{private_mb:.1} Mo"));
                kv("Working set", format!("{ws:.1} Mo"));
                kv("Threads (processus)", threads.to_string());
                kv("Threads (workers)", f.threads.to_string());
                kv("Onglets", f.state.tabs.len().to_string());
                kv(
                    "Entrées",
                    listing.map_or(0, fx_core::DirSnapshot::total).to_string(),
                );
                kv(
                    "Instantané",
                    format!(
                        "{} Ko",
                        listing.map_or(0, fx_core::DirSnapshot::heap_bytes) / 1024
                    ),
                );
                kv(
                    "Images en cache",
                    format!(
                        "{} ({} Ko icônes, {} Ko miniatures) ; {} états de synchronisation",
                        f.images.len(),
                        icons / 1024,
                        thumbs / 1024,
                        f.images.overlays()
                    ),
                );
                kv(
                    "Analyses de l'espace",
                    format!(
                        "{} / {} ({} Ko) ; cartes {} Ko",
                        f.state.disk_scans.len(),
                        fx_app::disk::MAX_SCANS,
                        f.state
                            .disk_scans
                            .iter()
                            .map(fx_app::disk::DiskScan::heap_bytes)
                            .sum::<usize>()
                            / 1024,
                        f.vs.treemaps
                            .values()
                            .map(super::disk::TreemapCache::heap_bytes)
                            .sum::<usize>()
                            / 1024
                    ),
                );
                kv(
                    "Premier lot",
                    pane.first_batch_after.map_or("—".into(), |t| {
                        format!("{:.1} ms", t.as_secs_f64() * 1000.0)
                    }),
                );
                kv(
                    "Opérations",
                    format!(
                        "{} en cours ; historique {} / {}",
                        f.state.ops.len(),
                        f.state.op_history.len(),
                        fx_app::ops::MAX_HISTORY
                    ),
                );
            });
        });
    if !open {
        f.cmd(fx_commands::CommandId::Diagnostics);
    }
}
