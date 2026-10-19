use std::cmp::Ordering;

use crate::entry::EntryMeta;
use crate::wtf8;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortKey {
    #[default]
    Name,
    Modified,
    Size,
    Type,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortSpec {
    pub key: SortKey,
    pub dir: SortDir,
    /// Dossiers avant fichiers, quel que soit le sens (comportement de l'Explorateur).
    pub folders_first: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            dir: SortDir::Asc,
            folders_first: true,
        }
    }
}

/// Une entrée vue par le comparateur.
#[derive(Clone, Copy)]
pub struct SortItem<'a> {
    pub meta: &'a EntryMeta,
    pub name: &'a [u8],
    pub ext: &'a [u8],
}

impl SortSpec {
    pub fn compare(&self, a: SortItem<'_>, b: SortItem<'_>) -> Ordering {
        if self.folders_first {
            let by_kind = b.meta.is_dir().cmp(&a.meta.is_dir());
            if by_kind != Ordering::Equal {
                return by_kind;
            }
        }
        let ord = match self.key {
            SortKey::Name => Ordering::Equal,
            SortKey::Modified => a.meta.modified.cmp(&b.meta.modified),
            SortKey::Size => a.meta.size.cmp(&b.meta.size),
            SortKey::Type => a.ext.cmp(b.ext),
        }
        .then_with(|| natural_cmp(a.name, b.name));
        match self.dir {
            SortDir::Asc => ord,
            SortDir::Desc => ord.reverse(),
        }
    }
}

/// Tri « naturel » proche de l'Explorateur (`StrCmpLogicalW`) :
/// insensible à la casse, suites de chiffres comparées comme des nombres,
/// espaces < ponctuation < chiffres < lettres.
///
/// Comme `CompareStringEx`, la comparaison se fait par niveaux : d'abord sans accents
/// (`Ébauche` < `file`), puis avec (`resume` < `résumé`), enfin octets bruts pour un ordre
/// total. L'ordre fin entre signes de ponctuation diffère encore (Windows ignore par exemple
/// les tirets) ; la concordance est vérifiée sur des noms alphanumériques par un test de `fx-fs`.
pub fn natural_cmp(a: &[u8], b: &[u8]) -> Ordering {
    natural_cmp_level(a, b, Level::Base)
        .then_with(|| natural_cmp_level(a, b, Level::Accents))
        .then_with(|| a.cmp(b))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    /// Lettres de base, sans casse ni accents.
    Base,
    /// Sans casse, accents distingués.
    Accents,
}

fn natural_cmp_level(a: &[u8], b: &[u8], level: Level) -> Ordering {
    let (mut i, mut j) = (0, 0);
    loop {
        match (i < a.len(), j < b.len()) {
            (false, false) => return Ordering::Equal,
            (false, true) => return Ordering::Less,
            (true, false) => return Ordering::Greater,
            (true, true) => {}
        }
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let ea = digit_run_end(a, i);
            let eb = digit_run_end(b, j);
            let ra = trim_leading_zeros(&a[i..ea]);
            let rb = trim_leading_zeros(&b[j..eb]);
            // À valeur égale, l'Explorateur place d'emblée la suite la plus longue (plus de
            // zéros en tête) en premier : `a01b` < `a1a` (vérifié contre StrCmpLogicalW).
            let ord = ra
                .len()
                .cmp(&rb.len())
                .then_with(|| ra.cmp(rb))
                .then_with(|| (eb - j).cmp(&(ea - i)));
            if ord != Ordering::Equal {
                return ord;
            }
            i = ea;
            j = eb;
            continue;
        }
        let (ca, na) = next_char(&a[i..]);
        let (cb, nb) = next_char(&b[j..]);
        if ca != cb {
            let ord = rank(ca)
                .cmp(&rank(cb))
                .then_with(|| fold(ca, level).cmp(&fold(cb, level)));
            if ord != Ordering::Equal {
                return ord;
            }
        }
        i += na;
        j += nb;
    }
}

fn digit_run_end(s: &[u8], start: usize) -> usize {
    start + s[start..].iter().take_while(|c| c.is_ascii_digit()).count()
}

fn trim_leading_zeros(run: &[u8]) -> &[u8] {
    let zeros = run.iter().take_while(|&&c| c == b'0').count();
    &run[zeros.min(run.len().saturating_sub(1))..]
}

#[inline]
fn next_char(s: &[u8]) -> (char, usize) {
    if s[0] < 0x80 {
        return (s[0] as char, 1);
    }
    let (cp, n) = wtf8::decode(s);
    (char::from_u32(cp).unwrap_or(char::REPLACEMENT_CHARACTER), n)
}

#[inline]
fn rank(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_ascii_digit() {
        2
    } else if c.is_alphanumeric() {
        3
    } else {
        1
    }
}

#[inline]
fn fold(c: char, level: Level) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    if level == Level::Base
        && let Some(base) = base_letter(c)
    {
        return base;
    }
    c.to_lowercase().next().unwrap_or(c)
}

/// Lettre de base des lettres latines accentuées (Latin-1 et Latin étendu A).
fn base_letter(c: char) -> Option<char> {
    // Une lettre ASCII minuscule par point de code ; `.` = pas de lettre de base.
    const LATIN1: &[u8; 64] = b"aaaaaaaceeeeiiiidnooooo.ouuuuy.saaaaaaaceeeeiiiidnooooo.ouuuuy.y";
    const LATIN_EXT_A: &[u8; 128] = b"aaaaaaccccccccddddeeeeeeeeeegggggggghhhhiiiiiiiiiijjjjkkkllllllllllnnnnnnnnnoooooooorrrrrrssssssssttttttuuuuuuuuuuuuwwyyyzzzzzzs";
    let cp = c as u32;
    let b = match cp {
        0xC0..=0xFF => LATIN1[(cp - 0xC0) as usize],
        0x100..=0x17F => LATIN_EXT_A[(cp - 0x100) as usize],
        _ => return None,
    };
    (b != b'.').then_some(b as char)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut v: Vec<&str> = names.to_vec();
        v.sort_by(|a, b| natural_cmp(a.as_bytes(), b.as_bytes()));
        v.into_iter().map(String::from).collect()
    }

    #[test]
    fn numbers_are_numeric() {
        assert_eq!(
            sorted(&["file10", "file2", "file1"]),
            ["file1", "file2", "file10"]
        );
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(sorted(&["b", "A", "a", "B"]), ["A", "a", "B", "b"]);
    }

    #[test]
    fn punctuation_before_digits_before_letters() {
        assert_eq!(sorted(&["a", "1", "_x", " y"]), [" y", "_x", "1", "a"]);
    }

    #[test]
    fn accents_are_secondary() {
        assert_eq!(
            sorted(&["file", "Ébauche", "résumé", "resume"]),
            ["Ébauche", "file", "resume", "résumé"]
        );
        assert_eq!(base_letter('Ž'), Some('z'));
        assert_eq!(base_letter('×'), None);
    }

    #[test]
    fn more_leading_zeros_first() {
        assert_eq!(natural_cmp(b"x007", b"x7"), Ordering::Less);
        assert_eq!(natural_cmp(b"a01b", b"a1a"), Ordering::Less);
        assert_eq!(natural_cmp(b"010", b"10"), Ordering::Less);
    }

    #[test]
    fn total_order_on_case_variants() {
        assert_ne!(natural_cmp(b"File", b"file"), Ordering::Equal);
    }

    #[test]
    fn folders_first_survives_desc() {
        let dir = EntryMeta {
            flags: crate::EntryFlags::DIR,
            ..Default::default()
        };
        let file = EntryMeta::default();
        let spec = SortSpec {
            dir: SortDir::Desc,
            ..Default::default()
        };
        let d = SortItem {
            meta: &dir,
            name: b"z",
            ext: b"",
        };
        let f = SortItem {
            meta: &file,
            name: b"a",
            ext: b"",
        };
        assert_eq!(spec.compare(d, f), Ordering::Less);
    }
}
