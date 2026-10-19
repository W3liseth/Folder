use crate::wtf8;

/// Position d'un nom dans une [`NameArena`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct NameSpan {
    offset: u32,
    len: u32,
}

/// Tous les noms d'un dossier, concaténés en WTF-8 dans un seul tampon.
///
/// Évite une allocation par fichier : 100 000 noms = une seule `Vec<u8>`.
#[derive(Debug, Default, Clone)]
pub struct NameArena {
    bytes: Vec<u8>,
}

impl NameArena {
    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(bytes),
        }
    }

    pub fn push_utf16(&mut self, units: &[u16]) -> NameSpan {
        let offset = self.offset();
        wtf8::push_utf16(&mut self.bytes, units);
        NameSpan {
            offset,
            len: self.offset() - offset,
        }
    }

    pub fn push_bytes(&mut self, name: &[u8]) -> NameSpan {
        let offset = self.offset();
        self.bytes.extend_from_slice(name);
        NameSpan {
            offset,
            len: name.len() as u32,
        }
    }

    pub fn get(&self, span: NameSpan) -> &[u8] {
        &self.bytes[span.offset as usize..(span.offset + span.len) as usize]
    }

    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    pub fn capacity(&self) -> usize {
        self.bytes.capacity()
    }

    /// Concatène `other` à la fin ; renvoie le décalage à ajouter à ses `NameSpan`.
    pub fn append(&mut self, other: &NameArena) -> u32 {
        let delta = self.offset();
        self.bytes.extend_from_slice(&other.bytes);
        delta
    }

    fn offset(&self) -> u32 {
        u32::try_from(self.bytes.len()).expect("arène de noms > 4 Gio")
    }
}

impl NameSpan {
    #[must_use]
    pub fn shifted(self, delta: u32) -> Self {
        Self {
            offset: self.offset + delta,
            len: self.len,
        }
    }

    pub fn len(self) -> usize {
        self.len as usize
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_get() {
        let mut a = NameArena::default();
        let x = a.push_bytes(b"alpha");
        let y = a.push_utf16(&"bêta".encode_utf16().collect::<Vec<_>>());
        assert_eq!(a.get(x), b"alpha");
        assert_eq!(a.get(y), "bêta".as_bytes());
    }

    #[test]
    fn append_shifts_spans() {
        let mut a = NameArena::default();
        a.push_bytes(b"one");
        let mut b = NameArena::default();
        let s = b.push_bytes(b"two");
        let delta = a.append(&b);
        assert_eq!(a.get(s.shifted(delta)), b"two");
    }
}
