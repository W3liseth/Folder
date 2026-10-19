//! Aperçus (volet d'aperçu), construits hors du thread UI et seulement quand le volet est
//! ouvert (ARCHITECTURE.md §7.9) : texte et code (coloration légère), Markdown, image
//! (miniature shell demandée par l'UI), dossier. Aucune WebView.

mod highlight;
mod markdown;
mod text;

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

pub use highlight::{Lang, Token, TokenKind, highlight};
pub use markdown::{MdBlock, MdSpan, parse_markdown};

/// Octets lus au plus pour un aperçu texte.
pub const MAX_TEXT_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Text {
        text: String,
        truncated: bool,
        lang: Lang,
    },
    Markdown(Vec<MdBlock>),
    /// Image : l'UI affiche une grande miniature shell.
    Image,
    Folder {
        files: u32,
        dirs: u32,
        truncated: bool,
    },
    /// Type sans aperçu (binaire, inconnu) : seules les propriétés sont affichées.
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub path: PathBuf,
    pub size: u64,
    pub created: i64,
    pub modified: i64,
    pub attributes: u32,
    pub content: Content,
}

const IMAGE_EXTS: [&str; 14] = [
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "heif", "tif", "tiff", "avif", "ico",
    "svg", "jxl",
];

/// Construit l'aperçu de `path`. Bloquant (lecture disque) : sur un worker.
pub fn build(path: &Path) -> Option<Preview> {
    let stat = fx_win::sys::stat(path)?;
    let is_dir = stat.attributes & 0x10 != 0;
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let content = if is_dir {
        folder_summary(path)
    } else if IMAGE_EXTS.contains(&ext.as_str()) {
        Content::Image
    } else if ext == "md" || ext == "markdown" {
        match text::read(path) {
            Some((t, _)) => Content::Markdown(parse_markdown(&t)),
            None => Content::None,
        }
    } else {
        match text::read(path) {
            Some((text, truncated)) => Content::Text {
                text,
                truncated,
                lang: Lang::from_ext(&ext),
            },
            None => Content::None,
        }
    };
    Some(Preview {
        path: path.to_path_buf(),
        size: stat.size,
        created: stat.created,
        modified: stat.modified,
        attributes: stat.attributes,
        content,
    })
}

fn folder_summary(path: &Path) -> Content {
    let (mut files, mut dirs, mut n) = (0u32, 0u32, 0u32);
    let truncated = std::cell::Cell::new(false);
    let _ = fx_win::dir::for_each_entry(path, |e| {
        if e.attributes & 0x10 != 0 {
            dirs += 1;
        } else {
            files += 1;
        }
        n += 1;
        if n >= 100_000 {
            truncated.set(true);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    Content::Folder {
        files,
        dirs,
        truncated: truncated.get(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_by_type() {
        let dir = std::env::temp_dir().join(format!("fx-preview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).expect("mkdir");
        std::fs::write(dir.join("a.rs"), "fn main() {}\n").expect("w");
        std::fs::write(dir.join("doc.md"), "# Titre\n\nTexte **gras**.").expect("w");
        std::fs::write(dir.join("bin.dat"), [0u8, 1, 2, 0, 255]).expect("w");
        std::fs::write(dir.join("p.png"), [0u8]).expect("w");

        let p = build(&dir.join("a.rs")).expect("rs");
        assert!(matches!(
            p.content,
            Content::Text {
                lang: Lang::Rust,
                ..
            }
        ));
        assert!(
            matches!(build(&dir.join("doc.md")).expect("md").content, Content::Markdown(ref b) if b.len() == 2)
        );
        assert_eq!(
            build(&dir.join("bin.dat")).expect("bin").content,
            Content::None
        );
        assert_eq!(
            build(&dir.join("p.png")).expect("png").content,
            Content::Image
        );
        assert!(matches!(
            build(&dir).expect("dir").content,
            Content::Folder {
                files: 4,
                dirs: 1,
                ..
            }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
