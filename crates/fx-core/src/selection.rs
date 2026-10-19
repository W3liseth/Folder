/// Sélection par identifiant d'entrée (stable au tri), en bitset : test O(1) par ligne
/// affichée, 12,5 Ko pour 100 000 entrées.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    bits: Vec<u64>,
    count: u32,
    /// Dernier élément cliqué : origine des sélections par plage (Maj+clic).
    pub anchor: Option<u32>,
}

impl Selection {
    pub fn clear(&mut self) {
        self.bits.clear();
        self.count = 0;
        self.anchor = None;
    }

    pub fn contains(&self, id: u32) -> bool {
        self.bits
            .get(id as usize / 64)
            .is_some_and(|w| w & (1 << (id % 64)) != 0)
    }

    pub fn set(&mut self, id: u32, on: bool) {
        let word = id as usize / 64;
        if word >= self.bits.len() {
            if !on {
                return;
            }
            self.bits.resize(word + 1, 0);
        }
        let mask = 1u64 << (id % 64);
        let was = self.bits[word] & mask != 0;
        if was != on {
            self.bits[word] ^= mask;
            if on {
                self.count += 1;
            } else {
                self.count -= 1;
            }
        }
    }

    pub fn toggle(&mut self, id: u32) {
        self.set(id, !self.contains(id));
        self.anchor = Some(id);
    }

    pub fn select_only(&mut self, id: u32) {
        self.clear();
        self.set(id, true);
        self.anchor = Some(id);
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.bits.iter().enumerate().flat_map(|(w, &bits)| {
            (0..64)
                .filter(move |b| bits & (1 << b) != 0)
                .map(move |b| (w * 64 + b) as u32)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_toggle_count() {
        let mut s = Selection::default();
        s.set(3, true);
        s.set(130, true);
        s.set(3, true);
        assert_eq!(s.count(), 2);
        s.toggle(130);
        assert!(!s.contains(130));
        assert_eq!(s.iter().collect::<Vec<_>>(), [3]);
        s.select_only(7);
        assert_eq!((s.count(), s.anchor), (1, Some(7)));
    }
}
