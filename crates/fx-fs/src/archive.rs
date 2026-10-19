//! Archives .zip : compression et extraction, annulables, avec progression en octets.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use fx_core::CancelToken;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// Progression : octets traités, total, élément courant.
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64, &str);

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "annulé")
}

/// Élément à compresser : chemin, nom dans l'archive, dossier ?
type Item = (PathBuf, String, bool);

/// Fichiers à compresser et taille totale, dossiers parcourus récursivement.
fn collect(sources: &[PathBuf]) -> io::Result<(Vec<Item>, u64)> {
    let mut out = Vec::new();
    let mut total = 0;
    let mut stack: Vec<(PathBuf, String)> = sources
        .iter()
        .filter_map(|s| {
            s.file_name()
                .map(|n| (s.clone(), n.to_string_lossy().into_owned()))
        })
        .collect();
    while let Some((path, name)) = stack.pop() {
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            out.push((path.clone(), format!("{name}/"), true));
            for e in std::fs::read_dir(&path)? {
                let e = e?;
                stack.push((
                    e.path(),
                    format!("{name}/{}", e.file_name().to_string_lossy()),
                ));
            }
        } else {
            total += meta.len();
            out.push((path, name, false));
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok((out, total))
}

/// Compresse `sources` dans `dest` (créé ; supprimé en cas d'échec ou d'annulation).
pub fn compress(
    sources: &[PathBuf],
    dest: &Path,
    cancel: &CancelToken,
    progress: Progress<'_>,
) -> io::Result<()> {
    let result = (|| {
        let (items, total) = collect(sources)?;
        let mut zip = ZipWriter::new(BufWriter::new(File::create_new(dest)?));
        let opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .large_file(total > u32::MAX as u64);
        let mut done = 0u64;
        let mut buf = vec![0u8; 256 * 1024];
        for (path, name, is_dir) in items {
            if cancel.is_cancelled() {
                return Err(cancelled());
            }
            progress(done, total, &name);
            if is_dir {
                zip.add_directory(name.trim_end_matches('/'), opts)?;
                continue;
            }
            zip.start_file(name.as_str(), opts)?;
            let mut f = BufReader::new(File::open(&path)?);
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                zip.write_all(&buf[..n])?;
                done += n as u64;
                if cancel.is_cancelled() {
                    return Err(cancelled());
                }
            }
            progress(done, total, &name);
        }
        zip.finish()?.flush()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

/// Extrait `archive` dans `dest` (créé). Les chemins qui sortiraient de `dest` sont refusés.
pub fn extract(
    archive: &Path,
    dest: &Path,
    cancel: &CancelToken,
    progress: Progress<'_>,
) -> io::Result<()> {
    let mut zip =
        ZipArchive::new(BufReader::new(File::open(archive)?)).map_err(io::Error::other)?;
    let total: u64 = (0..zip.len())
        .filter_map(|i| zip.by_index_raw(i).ok().map(|f| f.size()))
        .sum();
    std::fs::create_dir_all(dest)?;
    let mut done = 0u64;
    let mut buf = vec![0u8; 256 * 1024];
    for i in 0..zip.len() {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let mut file = zip.by_index(i).map_err(io::Error::other)?;
        // `enclosed_name` rejette « ../ » et les chemins absolus (« zip slip »).
        let Some(rel) = file.enclosed_name() else {
            continue;
        };
        let out = dest.join(&rel);
        let name = rel.to_string_lossy().into_owned();
        progress(done, total, &name);
        if file.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut w = BufWriter::new(File::create(&out)?);
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n])?;
            done += n as u64;
            if cancel.is_cancelled() {
                return Err(cancelled());
            }
        }
        w.flush()?;
    }
    progress(total, total, "");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_then_extract_roundtrip() {
        let base = std::env::temp_dir().join(format!("fx-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("src").join("sub")).expect("mkdir");
        std::fs::write(base.join("src").join("a.txt"), b"bonjour").expect("w");
        std::fs::write(
            base.join("src").join("sub").join("b.bin"),
            vec![7u8; 100_000],
        )
        .expect("w");
        let zip = base.join("out.zip");
        let mut last = (0, 0);
        compress(
            &[base.join("src")],
            &zip,
            &CancelToken::new(),
            &mut |d, t, _| last = (d, t),
        )
        .expect("compress");
        assert_eq!(last.0, last.1);
        assert!(std::fs::metadata(&zip).expect("zip").len() < 100_000);

        extract(
            &zip,
            &base.join("x"),
            &CancelToken::new(),
            &mut |_, _, _| {},
        )
        .expect("extract");
        assert_eq!(
            std::fs::read(base.join("x").join("src").join("a.txt")).expect("a"),
            b"bonjour"
        );
        assert_eq!(
            std::fs::read(base.join("x").join("src").join("sub").join("b.bin"))
                .expect("b")
                .len(),
            100_000
        );

        let c = CancelToken::new();
        c.cancel();
        assert!(
            compress(
                &[base.join("src")],
                &base.join("c.zip"),
                &c,
                &mut |_, _, _| {}
            )
            .is_err()
        );
        assert!(!base.join("c.zip").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
