use std::os::windows::ffi::OsStrExt;
use std::path::Path;

const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;

/// Convertit un chemin en UTF-16 terminé par NUL, en forme étendue `\\?\` pour lever la
/// limite de 260 caractères.
///
/// Le chemin est d'abord rendu absolu (`GetFullPathNameW`, qui résout `.` et `..`), car
/// la forme `\\?\` désactive toute normalisation côté Windows.
pub fn to_wide_extended(path: &Path) -> Vec<u16> {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let wide: Vec<u16> = abs
        .as_os_str()
        .encode_wide()
        .map(|c| if c == SLASH { BACKSLASH } else { c })
        .collect();
    let mut out = Vec::with_capacity(wide.len() + 9);
    if starts_with(&wide, r"\\?\") || starts_with(&wide, r"\\.\") {
        out.extend_from_slice(&wide);
    } else if starts_with(&wide, r"\\") {
        // \\serveur\partage -> \\?\UNC\serveur\partage
        out.extend(r"\\?\UNC".encode_utf16());
        out.extend_from_slice(&wide[1..]);
    } else {
        out.extend(r"\\?\".encode_utf16());
        out.extend_from_slice(&wide);
    }
    out.push(0);
    out
}

fn starts_with(wide: &[u16], prefix: &str) -> bool {
    let mut it = wide.iter();
    prefix.encode_utf16().all(|p| it.next() == Some(&p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(w: &[u16]) -> String {
        String::from_utf16_lossy(&w[..w.len() - 1])
    }

    #[test]
    fn drive_path() {
        assert_eq!(
            s(&to_wide_extended(Path::new(r"C:\Users\..\Windows"))),
            r"\\?\C:\Windows"
        );
    }

    #[test]
    fn forward_slashes() {
        assert_eq!(s(&to_wide_extended(Path::new("C:/a/b"))), r"\\?\C:\a\b");
    }

    #[test]
    fn unc_path() {
        assert_eq!(
            s(&to_wide_extended(Path::new(r"\\srv\share\x"))),
            r"\\?\UNC\srv\share\x"
        );
    }

    #[test]
    fn already_extended() {
        assert_eq!(s(&to_wide_extended(Path::new(r"\\?\C:\x"))), r"\\?\C:\x");
    }

    #[test]
    fn nul_terminated() {
        assert_eq!(to_wide_extended(Path::new(r"C:\")).last(), Some(&0));
    }
}
