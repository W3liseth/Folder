//! Encodage WTF-8 des noms Windows.
//!
//! Un nom de fichier Windows est une suite d'unités UTF-16 *non validée* : elle peut
//! contenir des surrogates isolés. WTF-8 (le même encodage que `OsStr` sous Windows)
//! les représente sans perte ; pour les noms valides, c'est exactement de l'UTF-8.

use std::borrow::Cow;

/// Ajoute l'encodage WTF-8 de `units` à `out`, sans allocation intermédiaire.
pub fn push_utf16(out: &mut Vec<u8>, units: &[u16]) {
    out.reserve(units.len());
    let mut i = 0;
    while i < units.len() {
        let u = units[i];
        if u < 0x80 {
            out.push(u as u8);
            i += 1;
            continue;
        }
        let is_high = (0xD800..0xDC00).contains(&u);
        let cp = match units.get(i + 1) {
            Some(&lo) if is_high && (0xDC00..0xE000).contains(&lo) => {
                i += 2;
                0x10000 + ((u32::from(u) - 0xD800) << 10) + (u32::from(lo) - 0xDC00)
            }
            _ => {
                i += 1;
                u32::from(u)
            }
        };
        push_code_point(out, cp);
    }
}

fn push_code_point(out: &mut Vec<u8>, cp: u32) {
    match cp {
        0..0x80 => out.push(cp as u8),
        0x80..0x800 => out.extend([0xC0 | (cp >> 6) as u8, 0x80 | (cp & 0x3F) as u8]),
        0x800..0x10000 => out.extend([
            0xE0 | (cp >> 12) as u8,
            0x80 | ((cp >> 6) & 0x3F) as u8,
            0x80 | (cp & 0x3F) as u8,
        ]),
        _ => out.extend([
            0xF0 | (cp >> 18) as u8,
            0x80 | ((cp >> 12) & 0x3F) as u8,
            0x80 | ((cp >> 6) & 0x3F) as u8,
            0x80 | (cp & 0x3F) as u8,
        ]),
    }
}

/// Décode du WTF-8 en UTF-16. Les octets mal formés deviennent U+FFFD.
pub fn to_utf16(bytes: &[u8]) -> Vec<u16> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let (cp, n) = decode(&bytes[i..]);
        i += n;
        if cp >= 0x10000 {
            let c = cp - 0x10000;
            out.extend([0xD800 + (c >> 10) as u16, 0xDC00 + (c & 0x3FF) as u16]);
        } else {
            out.push(cp as u16);
        }
    }
    out
}

/// Décode un point de code WTF-8 en tête de `b` ; renvoie (point de code, octets lus).
pub(crate) fn decode(b: &[u8]) -> (u32, usize) {
    const BAD: (u32, usize) = (0xFFFD, 1);
    let cont = |k: usize| {
        b.get(k)
            .filter(|&&x| x & 0xC0 == 0x80)
            .map(|&x| u32::from(x & 0x3F))
    };
    match b[0] {
        x @ 0..0x80 => (u32::from(x), 1),
        x @ 0xC0..0xE0 => cont(1).map_or(BAD, |c1| ((u32::from(x & 0x1F) << 6) | c1, 2)),
        x @ 0xE0..0xF0 => match (cont(1), cont(2)) {
            (Some(c1), Some(c2)) => ((u32::from(x & 0x0F) << 12) | (c1 << 6) | c2, 3),
            _ => BAD,
        },
        x @ 0xF0..0xF8 => match (cont(1), cont(2), cont(3)) {
            (Some(c1), Some(c2), Some(c3)) => {
                ((u32::from(x & 0x07) << 18) | (c1 << 12) | (c2 << 6) | c3, 4)
            }
            _ => BAD,
        },
        _ => BAD,
    }
}

/// Texte affichable ; emprunté (sans allocation) si le nom est de l'UTF-8 valide.
pub fn display(bytes: &[u8]) -> Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

/// Reconstruit le nom exact pour le passer au système.
#[cfg(windows)]
pub fn to_os_string(bytes: &[u8]) -> std::ffi::OsString {
    use std::os::windows::ffi::OsStringExt;
    std::ffi::OsString::from_wide(&to_utf16(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(units: &[u16]) {
        let mut b = Vec::new();
        push_utf16(&mut b, units);
        assert_eq!(to_utf16(&b), units);
    }

    #[test]
    fn ascii_and_bmp_are_utf8() {
        let s = "Résumé – 東京.txt";
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut b = Vec::new();
        push_utf16(&mut b, &units);
        assert_eq!(b, s.as_bytes());
    }

    #[test]
    fn astral_plane() {
        let units: Vec<u16> = "📁 dossier".encode_utf16().collect();
        roundtrip(&units);
    }

    #[test]
    fn lone_surrogates_survive() {
        roundtrip(&[0x61, 0xD800, 0x62, 0xDC00]);
        roundtrip(&[0xDBFF]);
    }

    #[cfg(windows)]
    #[test]
    fn matches_std_os_str_encoding() {
        use std::os::windows::ffi::OsStringExt;
        let units = [0x66, 0xD83D, 0xDCC1, 0xD800, 0x7A];
        let os = std::ffi::OsString::from_wide(&units);
        let mut b = Vec::new();
        push_utf16(&mut b, &units);
        assert_eq!(b, os.as_encoded_bytes());
        assert_eq!(to_os_string(&b), os);
    }
}
