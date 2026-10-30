use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::entry::{Batch, EntryFlags, EntryMeta, ExtId};
use crate::names::NameArena;
use crate::sort::{SortItem, SortSpec};
use crate::wtf8;

/// Compteurs agrégés d'un dossier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirStats {
    pub files: u32,
    pub dirs: u32,
    pub file_bytes: u64,
}

/// Une entrée vue à travers l'ordre d'affichage.
#[derive(Debug, Clone, Copy)]
pub struct EntryView<'a> {
    /// Identifiant stable (indice dans `entries`), indépendant du tri.
    pub id: u32,
    pub meta: &'a EntryMeta,
    pub name: &'a [u8],
}

/// Contenu d'un dossier, en structure de tableaux.
///
/// Les entrées ne bougent jamais (leur indice sert d'identifiant stable pour la
/// sélection) ; seul `order` est trié. Une entrée supprimée sort de `order` mais garde sa
/// place dans `entries` jusqu'au prochain rechargement.
#[derive(Debug)]
pub struct DirSnapshot {
    location: PathBuf,
    generation: u64,
    names: NameArena,
    entries: Vec<EntryMeta>,
    order: Vec<u32>,
    /// Sous-ensemble de `order` correspondant au filtre rapide, s'il y en a un.
    filtered: Option<Vec<u32>>,
    filter: Option<String>,
    exts: ExtTable,
    sort: SortSpec,
    stats: DirStats,
    complete: bool,
    /// Listes virtuelles (recherche) : dossier de chaque entrée, indexé dans `dirs`.
    dirs: Vec<PathBuf>,
    entry_dir: Vec<u32>,
    /// Index nom → identifiant, construit à la première notification de changement.
    index: Option<HashMap<u64, u32>>,
}

impl DirSnapshot {
    pub fn new(location: PathBuf, generation: u64, sort: SortSpec) -> Self {
        Self {
            location,
            generation,
            names: NameArena::default(),
            entries: Vec::new(),
            order: Vec::new(),
            filtered: None,
            filter: None,
            exts: ExtTable::default(),
            sort,
            stats: DirStats::default(),
            complete: false,
            dirs: Vec::new(),
            entry_dir: Vec::new(),
            index: None,
        }
    }

    /// Ajoute un lot. Le lot est trié puis fusionné en O(n + m) : l'affichage reste trié
    /// pendant tout le chargement sans re-trier l'ensemble à chaque lot.
    pub fn append(&mut self, mut batch: Batch) {
        let delta = self.names.append(&batch.names);
        let first = self.entries.len() as u32;
        let dir_slot = batch.dir.take().map(|d| {
            self.dirs.push(d);
            (self.dirs.len() - 1) as u32
        });
        self.entries.reserve(batch.entries.len());
        for mut e in batch.entries.drain(..) {
            e.name = e.name.shifted(delta);
            self.account(&mut e, true);
            let id = self.entries.len() as u32;
            self.entries.push(e);
            if let Some(slot) = dir_slot {
                self.entry_dir.resize(id as usize, 0);
                self.entry_dir.push(slot);
            }
            if let Some(index) = &mut self.index {
                index.insert(name_key(self.names.get(e.name)), id);
            }
        }
        let mut fresh: Vec<u32> = (first..self.entries.len() as u32).collect();
        fresh.sort_unstable_by(|&a, &b| self.cmp_ids(a, b));
        self.merge_into_order(fresh);
        self.refilter();
    }

    /// Met à jour les statistiques et l'extension d'une entrée qui entre (`add`) ou sort.
    fn account(&mut self, e: &mut EntryMeta, add: bool) {
        if e.is_dir() {
            self.stats.dirs = if add {
                self.stats.dirs + 1
            } else {
                self.stats.dirs.saturating_sub(1)
            };
        } else {
            if add {
                e.ext = self.exts.intern(extension(self.names.get(e.name)));
                self.stats.files += 1;
                self.stats.file_bytes += e.size;
            } else {
                self.stats.files = self.stats.files.saturating_sub(1);
                self.stats.file_bytes = self.stats.file_bytes.saturating_sub(e.size);
            }
        }
    }

    pub fn set_sort(&mut self, sort: SortSpec) {
        if sort == self.sort {
            return;
        }
        self.sort = sort;
        let mut order = std::mem::take(&mut self.order);
        order.sort_unstable_by(|&a, &b| self.cmp_ids(a, b));
        self.order = order;
        self.refilter();
    }

    fn merge_into_order(&mut self, fresh: Vec<u32>) {
        if self.order.is_empty() {
            self.order = fresh;
            return;
        }
        let old = std::mem::take(&mut self.order);
        let mut merged = Vec::with_capacity(old.len() + fresh.len());
        let (mut i, mut j) = (0, 0);
        while i < old.len() && j < fresh.len() {
            if self.cmp_ids(fresh[j], old[i]).is_lt() {
                merged.push(fresh[j]);
                j += 1;
            } else {
                merged.push(old[i]);
                i += 1;
            }
        }
        merged.extend_from_slice(&old[i..]);
        merged.extend_from_slice(&fresh[j..]);
        self.order = merged;
    }

    fn cmp_ids(&self, a: u32, b: u32) -> std::cmp::Ordering {
        self.sort.compare(self.sort_item(a), self.sort_item(b))
    }

    fn sort_item(&self, id: u32) -> SortItem<'_> {
        let meta = &self.entries[id as usize];
        SortItem {
            meta,
            name: self.names.get(meta.name),
            ext: self.exts.get(meta.ext),
        }
    }

    // --- Changements incrémentaux (surveillance) ------------------------------------

    /// Identifiant d'une entrée par son nom (insensible à la casse ASCII).
    pub fn find(&mut self, name: &[u8]) -> Option<u32> {
        if self.index.is_none() {
            let mut index = HashMap::with_capacity(self.order.len());
            for &id in &self.order {
                index.insert(name_key(self.names.get(self.entries[id as usize].name)), id);
            }
            self.index = Some(index);
        }
        let id = *self.index.as_ref()?.get(&name_key(name))?;
        // Collision de hachage improbable : on vérifie le nom.
        let stored = self.names.get(self.entries[id as usize].name);
        stored.eq_ignore_ascii_case(name).then_some(id)
    }

    /// Retire une entrée ; renvoie son identifiant.
    pub fn remove(&mut self, name: &[u8]) -> Option<u32> {
        let id = self.find(name)?;
        if let Some(pos) = self.order.iter().position(|&x| x == id) {
            self.order.remove(pos);
        }
        if let Some(index) = &mut self.index {
            index.remove(&name_key(name));
        }
        let mut e = self.entries[id as usize];
        self.account(&mut e, false);
        self.refilter();
        Some(id)
    }

    /// Ajoute ou met à jour une entrée (nom UTF-16 tel que notifié), à sa place dans le tri.
    pub fn upsert(
        &mut self,
        name: &[u16],
        attributes: u32,
        size: u64,
        created: i64,
        modified: i64,
    ) -> u32 {
        let mut bytes = Vec::with_capacity(name.len());
        wtf8::push_utf16(&mut bytes, name);
        let existing = self.find(&bytes);
        let id = match existing {
            Some(id) => {
                let cur = self.entries[id as usize];
                let same_name = self.names.get(cur.name) == bytes.as_slice();
                // Changement déjà appliqué (surveillance puis relecture après une opération) :
                // rien à faire, ni décalage O(n) de l'ordre.
                if same_name
                    && cur.attributes == attributes
                    && cur.size == size
                    && cur.created == created
                    && cur.modified == modified
                {
                    return id;
                }
                if let Some(pos) = self.order.iter().position(|&x| x == id) {
                    self.order.remove(pos);
                }
                let mut old = cur;
                self.account(&mut old, false);
                // Renommage qui ne change que la casse : le nom affiché suit.
                if !same_name {
                    self.entries[id as usize].name = self.names.push_bytes(&bytes);
                }
                let e = &mut self.entries[id as usize];
                e.attributes = attributes;
                e.size = size;
                e.created = created;
                e.modified = modified;
                e.flags = EntryFlags::from_attributes(attributes);
                id
            }
            None => {
                let span = self.names.push_bytes(&bytes);
                let id = self.entries.len() as u32;
                self.entries.push(EntryMeta {
                    name: span,
                    size,
                    modified,
                    created,
                    attributes,
                    ext: ExtId::NONE,
                    flags: EntryFlags::from_attributes(attributes),
                });
                if let Some(index) = &mut self.index {
                    index.insert(name_key(&bytes), id);
                }
                id
            }
        };
        let mut e = self.entries[id as usize];
        self.account(&mut e, true);
        self.entries[id as usize] = e;
        let pos = self.order.partition_point(|&x| self.cmp_ids(x, id).is_lt());
        self.order.insert(pos, id);
        self.refilter();
        id
    }

    // --- Filtre rapide --------------------------------------------------------------

    /// Filtre « contient » insensible à la casse ; `None` ou vide : tout afficher.
    pub fn set_filter(&mut self, filter: Option<&str>) {
        let f = filter
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(str::to_lowercase);
        if f == self.filter {
            return;
        }
        self.filter = f;
        self.refilter();
    }

    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    fn refilter(&mut self) {
        self.filtered = self.filter.as_ref().map(|f| {
            self.order
                .iter()
                .copied()
                .filter(|&id| {
                    let n = self.names.get(self.entries[id as usize].name);
                    wtf8::display(n).to_lowercase().contains(f.as_str())
                })
                .collect()
        });
    }

    fn visible(&self) -> &[u32] {
        self.filtered.as_deref().unwrap_or(&self.order)
    }

    // --- Lecture ----------------------------------------------------------------------

    /// Entrée à la position d'affichage `pos` (après filtre).
    pub fn at(&self, pos: usize) -> Option<EntryView<'_>> {
        let id = *self.visible().get(pos)?;
        Some(self.view(id))
    }

    pub fn view(&self, id: u32) -> EntryView<'_> {
        let meta = &self.entries[id as usize];
        EntryView {
            id,
            meta,
            name: self.names.get(meta.name),
        }
    }

    /// Position d'affichage d'un identifiant. O(n) : réservé aux actions ponctuelles.
    pub fn position_of(&self, id: u32) -> Option<usize> {
        self.visible().iter().position(|&x| x == id)
    }

    /// Identifiants visibles, dans l'ordre d'affichage.
    pub fn visible_ids(&self) -> &[u32] {
        self.visible()
    }

    /// Dossier contenant l'entrée `id` (différent de `location` dans une liste virtuelle).
    pub fn dir_of(&self, id: u32) -> &Path {
        match self.entry_dir.get(id as usize) {
            Some(&slot) if !self.dirs.is_empty() => &self.dirs[slot as usize],
            _ => &self.location,
        }
    }

    pub fn is_virtual(&self) -> bool {
        !self.dirs.is_empty()
    }

    pub fn ext_of(&self, meta: &EntryMeta) -> &[u8] {
        self.exts.get(meta.ext)
    }

    /// Nombre d'entrées visibles (après filtre).
    pub fn len(&self) -> usize {
        self.visible().len()
    }

    pub fn is_empty(&self) -> bool {
        self.visible().is_empty()
    }

    /// Nombre total d'entrées, filtre ignoré.
    pub fn total(&self) -> usize {
        self.order.len()
    }

    pub fn location(&self) -> &Path {
        &self.location
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn sort(&self) -> SortSpec {
        self.sort
    }

    pub fn stats(&self) -> DirStats {
        self.stats
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Fin du chargement : libère la capacité excédentaire des tampons.
    pub fn finish(&mut self) {
        self.complete = true;
        self.entries.shrink_to_fit();
        self.order.shrink_to_fit();
    }

    /// Octets occupés par cet instantané (pour l'overlay et les tests de budget).
    pub fn heap_bytes(&self) -> usize {
        self.names.capacity()
            + self.entries.capacity() * size_of::<EntryMeta>()
            + self.order.capacity() * size_of::<u32>()
            + self.filtered.as_ref().map_or(0, |f| f.capacity() * 4)
            + self.entry_dir.capacity() * 4
            + self.index.as_ref().map_or(0, |i| i.capacity() * 16)
            + self.exts.heap_bytes()
    }
}

/// Clé d'index : FNV-1a sur le nom en minuscules ASCII.
fn name_key(name: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in name {
        h ^= u64::from(b.to_ascii_lowercase());
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Extension (sans le point) d'un nom de fichier, en minuscules ASCII via l'interneur.
fn extension(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|&c| c == b'.') {
        Some(dot) if dot + 1 < name.len() && name.len() - dot <= 17 => &name[dot + 1..],
        _ => b"",
    }
}

/// Interneur d'extensions : `.TXT` et `.txt` partagent le même `ExtId`.
#[derive(Debug, Default)]
struct ExtTable {
    ids: HashMap<Box<[u8]>, ExtId>,
    names: Vec<Box<[u8]>>,
}

impl ExtTable {
    fn intern(&mut self, ext: &[u8]) -> ExtId {
        if ext.is_empty() {
            return ExtId::NONE;
        }
        let mut buf = [0u8; 16];
        let key = &mut buf[..ext.len()];
        key.copy_from_slice(ext);
        key.make_ascii_lowercase();
        if let Some(&id) = self.ids.get(&*key) {
            return id;
        }
        if self.names.len() >= usize::from(u16::MAX) - 1 {
            return ExtId::NONE;
        }
        let id = ExtId(self.names.len() as u16 + 1);
        self.names.push(key.into());
        self.ids.insert(key.into(), id);
        id
    }

    fn get(&self, id: ExtId) -> &[u8] {
        match id.0 {
            0 => b"",
            n => &self.names[usize::from(n) - 1],
        }
    }

    fn heap_bytes(&self) -> usize {
        self.names.iter().map(|n| n.len() * 2 + 48).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::attrs;
    use crate::sort::{SortDir, SortKey};

    fn batch(names: &[(&str, bool, u64)]) -> Batch {
        let mut b = Batch::default();
        for &(n, dir, size) in names {
            let wide: Vec<u16> = n.encode_utf16().collect();
            b.push_utf16(&wide, if dir { attrs::DIRECTORY } else { 0 }, size, 0, 0);
        }
        b
    }

    fn names(s: &DirSnapshot) -> Vec<String> {
        (0..s.len())
            .map(|i| String::from_utf8_lossy(s.at(i).expect("pos").name).into_owned())
            .collect()
    }

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn batches_stay_sorted_with_folders_first() {
        let mut s = DirSnapshot::new("C:\\x".into(), 1, SortSpec::default());
        s.append(batch(&[
            ("b10.txt", false, 1),
            ("dir", true, 0),
            ("b2.txt", false, 2),
        ]));
        s.append(batch(&[
            ("a.TXT", false, 3),
            ("b1.txt", false, 4),
            ("adir", true, 0),
        ]));
        assert_eq!(
            names(&s),
            ["adir", "dir", "a.TXT", "b1.txt", "b2.txt", "b10.txt"]
        );
        assert_eq!(
            s.stats(),
            DirStats {
                files: 4,
                dirs: 2,
                file_bytes: 10
            }
        );
    }

    #[test]
    fn extensions_are_interned_case_insensitively() {
        let mut s = DirSnapshot::new("C:\\x".into(), 1, SortSpec::default());
        s.append(batch(&[
            ("a.TXT", false, 0),
            ("b.txt", false, 0),
            ("c", false, 0),
        ]));
        let ext = |i| s.ext_of(s.at(i).expect("pos").meta).to_vec();
        assert_eq!(ext(0), b"txt");
        assert_eq!(s.at(0).expect("a").meta.ext, s.at(1).expect("b").meta.ext);
        assert_eq!(ext(2), b"");
    }

    #[test]
    fn resort_by_size_desc_keeps_ids_stable() {
        let mut s = DirSnapshot::new("C:\\x".into(), 1, SortSpec::default());
        s.append(batch(&[("small", false, 1), ("big", false, 100)]));
        let big_id = s.at(0).expect("big").id;
        s.set_sort(SortSpec {
            key: SortKey::Size,
            dir: SortDir::Desc,
            folders_first: true,
        });
        assert_eq!(names(&s), ["big", "small"]);
        assert_eq!(s.at(0).expect("big").id, big_id);
        assert_eq!(s.position_of(big_id), Some(0));
    }

    #[test]
    fn incremental_upsert_and_remove() {
        let mut s = DirSnapshot::new("C:\\x".into(), 1, SortSpec::default());
        s.append(batch(&[("b.txt", false, 1), ("d.txt", false, 1)]));
        s.upsert(&w("c.txt"), 0, 5, 0, 0);
        assert_eq!(names(&s), ["b.txt", "c.txt", "d.txt"]);
        let id = s.upsert(&w("C.TXT"), 0, 7, 0, 0);
        assert_eq!(s.total(), 3, "mise à jour, pas d'ajout");
        assert_eq!(s.view(id).meta.size, 7);
        assert_eq!(s.stats().file_bytes, 9);
        assert_eq!(
            names(&s),
            ["b.txt", "C.TXT", "d.txt"],
            "casse du nom notifié"
        );
        assert_eq!(s.upsert(&w("C.TXT"), 0, 7, 0, 0), id, "déjà à jour");
        s.upsert(&w("a"), attrs::DIRECTORY, 0, 0, 0);
        assert_eq!(names(&s), ["a", "b.txt", "C.TXT", "d.txt"]);
        assert!(s.remove(b"b.TXT").is_some());
        assert_eq!(names(&s), ["a", "C.TXT", "d.txt"]);
        assert_eq!(
            s.stats(),
            DirStats {
                files: 2,
                dirs: 1,
                file_bytes: 8
            }
        );
        assert!(s.remove(b"zzz").is_none());
    }

    #[test]
    fn quick_filter_follows_changes() {
        let mut s = DirSnapshot::new("C:\\x".into(), 1, SortSpec::default());
        s.append(batch(&[
            ("Rapport.pdf", false, 1),
            ("photo.jpg", false, 1),
            ("RAPPEL.txt", false, 1),
        ]));
        s.set_filter(Some("rap"));
        assert_eq!(names(&s), ["RAPPEL.txt", "Rapport.pdf"]);
        assert_eq!(s.total(), 3);
        s.upsert(&w("rapide.md"), 0, 1, 0, 0);
        assert_eq!(s.len(), 3);
        s.set_filter(None);
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn virtual_listing_keeps_dirs() {
        let mut s = DirSnapshot::new("C:\\search".into(), 1, SortSpec::default());
        let mut b = batch(&[("x.txt", false, 1)]);
        b.dir = Some(PathBuf::from("C:\\a"));
        s.append(b);
        let mut b = batch(&[("y.txt", false, 1)]);
        b.dir = Some(PathBuf::from("C:\\b"));
        s.append(b);
        assert!(s.is_virtual());
        let y = s.at(1).expect("y");
        assert_eq!(s.dir_of(y.id), Path::new("C:\\b"));
    }
}
