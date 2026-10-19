//! Génère des arborescences de test reproductibles pour les benchmarks.
//!
//! ```text
//! gen-tree <SORTIE> [--sizes 1k,100k] [--force]
//! ```
//!
//! Chaque jeu contient des noms réalistes (nombres pour le tri naturel, accents, CJK, emoji),
//! ~2 % de dossiers, quelques fichiers non vides et, dans `long/`, un chemin > 260 caractères.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};

const MARKER: &str = ".fx-gen-tree-complete";
const THREADS: usize = 4;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mut out: Option<PathBuf> = None;
    let mut sizes = vec![1_000usize, 100_000];
    let mut force = false;
    while let Some(a) = args.next() {
        match a.to_str() {
            Some("--force") => force = true,
            Some("--sizes") => {
                let v = args
                    .next()
                    .context("--sizes attend une liste, ex. 1k,100k")?;
                sizes = v
                    .to_string_lossy()
                    .split(',')
                    .map(parse_size)
                    .collect::<Result<_>>()?;
            }
            Some(s) if s.starts_with("--") => bail!("option inconnue : {s}"),
            _ => out = Some(a.into()),
        }
    }
    let out = out.context("usage : gen-tree <SORTIE> [--sizes 1k,100k] [--force]")?;

    for n in sizes {
        let dir = out.join(label(n));
        if dir.join(MARKER).exists() && !force {
            println!("{} : déjà généré (--force pour régénérer)", dir.display());
            continue;
        }
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("suppression de {}", dir.display()))?;
        }
        let t = Instant::now();
        generate(&dir, n)?;
        println!(
            "{} : {n} entrées en {:.1} s",
            dir.display(),
            t.elapsed().as_secs_f64()
        );
    }
    Ok(())
}

fn parse_size(s: &str) -> Result<usize> {
    let s = s.trim().to_ascii_lowercase();
    let (num, mult) = match s.strip_suffix('k') {
        Some(n) => (n, 1_000),
        None => match s.strip_suffix('m') {
            Some(n) => (n, 1_000_000),
            None => (s.as_str(), 1),
        },
    };
    Ok(num
        .parse::<usize>()
        .with_context(|| format!("taille invalide : {s}"))?
        * mult)
}

fn label(n: usize) -> String {
    match n {
        n if n >= 1_000_000 && n % 1_000_000 == 0 => format!("{}m", n / 1_000_000),
        n if n >= 1_000 && n % 1_000 == 0 => format!("{}k", n / 1_000),
        n => n.to_string(),
    }
}

/// Nom déterministe de l'entrée `i`.
fn name(i: usize) -> String {
    const EXT: [&str; 8] = ["pdf", "docx", "jpg", "txt", "rs", "png", "xlsx", "mp4"];
    let ext = EXT[i % EXT.len()];
    match i % 20 {
        0 => format!("Résumé {i:06}.{ext}"),
        1 => format!("東京 写真 {i}.{ext}"),
        2 => format!("📁 archive {i}.{ext}"),
        3 => format!("IMG_{i:06}.{ext}"),
        4 => format!("{i}"),
        _ => format!("Rapport {} - v{} final ({i}).{ext}", i % 977, i % 13),
    }
}

fn generate(dir: &Path, n: usize) -> Result<()> {
    fs::create_dir_all(dir)?;
    std::thread::scope(|s| -> Result<()> {
        let handles: Vec<_> = (0..THREADS)
            .map(|t| {
                s.spawn(move || -> Result<()> {
                    for i in (t..n).step_by(THREADS) {
                        let p = dir.join(name(i));
                        if i % 50 == 7 {
                            fs::create_dir(&p).with_context(|| p.display().to_string())?;
                        } else {
                            let f = File::create(&p).with_context(|| p.display().to_string())?;
                            // 1 % de fichiers non vides (taille seulement, sans écrire de données).
                            if i % 100 == 3 {
                                f.set_len((i as u64 * 7919) % (256 * 1024))?;
                            }
                        }
                    }
                    Ok(())
                })
            })
            .collect();
        for h in handles {
            h.join()
                .map_err(|_| anyhow::anyhow!("thread de génération en panique"))??;
        }
        Ok(())
    })?;

    // Chemin long (> 260 caractères) : la std utilise automatiquement la forme \\?\.
    let mut long = dir.join("long");
    for i in 0..12 {
        long.push(format!("segment-de-chemin-assez-long-{i:02}"));
    }
    fs::create_dir_all(&long)?;
    File::create(long.join("fichier au bout du chemin long.txt"))?;

    File::create(dir.join(MARKER))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_labels() {
        assert_eq!(parse_size("100k").expect("size"), 100_000);
        assert_eq!(parse_size("1M").expect("size"), 1_000_000);
        assert_eq!(label(100_000), "100k");
        assert_eq!(label(1_000_000), "1m");
    }

    #[test]
    fn names_are_unique() {
        let set: std::collections::HashSet<_> = (0..200_000).map(name).collect();
        assert_eq!(set.len(), 200_000);
    }
}
