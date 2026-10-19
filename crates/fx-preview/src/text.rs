//! Lecture de texte avec détection d'encodage (BOM UTF-8/UTF-16, UTF-8, sinon Windows-1252).

use std::io::Read;
use std::path::Path;

use crate::MAX_TEXT_BYTES;

/// Texte décodé et indicateur de troncature ; `None` si le fichier semble binaire.
pub fn read(path: &Path) -> Option<(String, bool)> {
    let f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::with_capacity(16 * 1024);
    f.take(MAX_TEXT_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .ok()?;
    let truncated = buf.len() > MAX_TEXT_BYTES;
    buf.truncate(MAX_TEXT_BYTES);
    decode(&buf).map(|t| (t, truncated))
}

pub fn decode(b: &[u8]) -> Option<String> {
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Some(String::from_utf8_lossy(rest).into_owned());
    }
    if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return Some(String::from_utf16_lossy(&units));
    }
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return Some(String::from_utf16_lossy(&units));
    }
    // Un octet nul dans les 8 premiers Kio : binaire.
    if b.iter().take(8192).any(|&c| c == 0) {
        return None;
    }
    match std::str::from_utf8(b) {
        Ok(s) => Some(s.to_owned()),
        // Coupure au milieu d'un caractère en fin de tampon : on garde la partie valide.
        Err(e) if e.error_len().is_none() => {
            Some(String::from_utf8_lossy(&b[..e.valid_up_to()]).into_owned())
        }
        Err(_) => Some(b.iter().map(|&c| cp1252(c)).collect()),
    }
}

/// Windows-1252 : identique à Latin-1 sauf 0x80–0x9F.
fn cp1252(c: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    match c {
        0x80..=0x9F => HIGH[usize::from(c - 0x80)],
        _ => char::from(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings() {
        assert_eq!(decode(b"\xEF\xBB\xBFh\xC3\xA9").as_deref(), Some("hé"));
        assert_eq!(
            decode(&[0xFF, 0xFE, b'h', 0, 0xE9, 0]).as_deref(),
            Some("hé")
        );
        assert_eq!(decode(b"caf\xE9 \x80").as_deref(), Some("café €"));
        assert_eq!(decode(b"a\0b"), None);
        assert_eq!(decode("été".as_bytes()).as_deref(), Some("été"));
    }
}
