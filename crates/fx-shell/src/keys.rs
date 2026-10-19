use std::path::{Path, PathBuf};

use crate::ImageKind;

/// Clé de cache d'une image shell.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImageKey {
    /// Icône partagée par tous les fichiers d'une extension (minuscules).
    Ext { ext: String, size: u16 },
    /// Icône générique de dossier.
    Folder { size: u16 },
    /// Icône propre à un fichier ou dossier (.exe, .lnk, racines de lecteurs, dossiers
    /// spéciaux…).
    Path { path: PathBuf, size: u16 },
    /// Miniature d'un fichier.
    Thumb { path: PathBuf, size: u16 },
}

impl ImageKey {
    pub fn kind(&self) -> ImageKind {
        match self {
            Self::Thumb { .. } => ImageKind::Thumbnail,
            _ => ImageKind::Icon,
        }
    }

    pub fn size(&self) -> u16 {
        match self {
            Self::Ext { size, .. }
            | Self::Folder { size }
            | Self::Path { size, .. }
            | Self::Thumb { size, .. } => *size,
        }
    }
}

/// Extensions dont chaque fichier a sa propre icône.
const PER_FILE: [&str; 11] = [
    "exe",
    "lnk",
    "ico",
    "url",
    "cur",
    "ani",
    "msc",
    "appref-ms",
    "scr",
    "cpl",
    "website",
];

/// Extensions pour lesquelles une miniature est pertinente.
pub const THUMBNAIL_EXTS: [&str; 22] = [
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "heif", "tif", "tiff", "avif", "jxl",
    "svg", "ico", "mp4", "mkv", "mov", "avi", "wmv", "webm", "pdf", "psd",
];

/// Clé d'icône et fichier source pour l'entrée `name` du dossier `dir`.
pub fn image_request(
    dir: &Path,
    name: &str,
    ext: &str,
    is_dir: bool,
    size: u16,
) -> (ImageKey, PathBuf) {
    let path = dir.join(name);
    let key = if is_dir {
        ImageKey::Folder { size }
    } else if ext.is_empty() || PER_FILE.contains(&ext) {
        ImageKey::Path {
            path: path.clone(),
            size,
        }
    } else {
        ImageKey::Ext {
            ext: ext.to_owned(),
            size,
        }
    };
    (key, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_share_by_extension() {
        let (a, _) = image_request(Path::new(r"C:\x"), "a.txt", "txt", false, 16);
        let (b, _) = image_request(Path::new(r"C:\y"), "b.txt", "txt", false, 16);
        assert_eq!(a, b);
        let (e1, _) = image_request(Path::new(r"C:\x"), "a.exe", "exe", false, 16);
        let (e2, _) = image_request(Path::new(r"C:\x"), "b.exe", "exe", false, 16);
        assert_ne!(e1, e2);
        assert_eq!(
            image_request(Path::new(r"C:\x"), "d", "", true, 16).0,
            ImageKey::Folder { size: 16 }
        );
    }
}
