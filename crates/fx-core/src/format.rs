//! Formats d'affichage. Écrivent dans un `fmt::Write` fourni pour que l'UI réutilise
//! ses tampons au lieu d'allouer une `String` par ligne et par image.

use std::fmt::{self, Write};

/// Taille façon colonne « Taille » de l'Explorateur : kilo-octets arrondis au supérieur,
/// séparateur de milliers (`12 345 Ko`).
pub fn size_kb(out: &mut impl Write, bytes: u64) -> fmt::Result {
    let kb = bytes.div_ceil(1024);
    thousands(out, kb)?;
    out.write_str(" Ko")
}

/// Taille lisible pour la barre d'état (`12,4 Mo`).
pub fn size_human(out: &mut impl Write, bytes: u64) -> fmt::Result {
    const UNITS: [&str; 5] = ["octets", "Ko", "Mo", "Go", "To"];
    if bytes < 1024 {
        return write!(out, "{bytes} {}", UNITS[0]);
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    let tenths = (v * 10.0).round() as u64;
    // « 1 Go » plutôt que « 1,0 Go » ; une décimale en dessous de 100.
    if tenths >= 1000 {
        write!(out, "{} {}", (tenths + 5) / 10, UNITS[unit])
    } else if tenths.is_multiple_of(10) {
        write!(out, "{} {}", tenths / 10, UNITS[unit])
    } else {
        write!(out, "{},{} {}", tenths / 10, tenths % 10, UNITS[unit])
    }
}

/// Entier avec espace insécable fine comme séparateur de milliers.
pub fn thousands(out: &mut impl Write, n: u64) -> fmt::Result {
    let s = n.to_string();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.write_char('\u{202F}')?;
        }
        out.write_char(c)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(g: impl Fn(&mut String) -> fmt::Result) -> String {
        let mut s = String::new();
        g(&mut s).expect("fmt");
        s
    }

    #[test]
    fn kb_rounds_up() {
        assert_eq!(f(|s| size_kb(s, 0)), "0 Ko");
        assert_eq!(f(|s| size_kb(s, 1)), "1 Ko");
        assert_eq!(f(|s| size_kb(s, 12_345 * 1024)), "12\u{202F}345 Ko");
    }

    #[test]
    fn human() {
        assert_eq!(f(|s| size_human(s, 512)), "512 octets");
        assert_eq!(f(|s| size_human(s, 1536)), "1,5 Ko");
        assert_eq!(f(|s| size_human(s, 300 * 1024 * 1024)), "300 Mo");
        assert_eq!(f(|s| size_human(s, 1024 * 1024 * 1024)), "1 Go");
        assert_eq!(f(|s| size_human(s, 1_000_000 * 1024)), "977 Mo");
    }
}
