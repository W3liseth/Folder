//! Arbre d'occupation du disque (analyse de l'espace), en structure de tableaux.
//!
//! Un nœud par dossier, par fichier volumineux (au moins `keep_files_from` octets sur le
//! disque) et un nœud « petits fichiers » par dossier qui en a : un lecteur système compte
//! 1 à 2 millions de fichiers, dont l'immense majorité ne ferait pas un pixel dans une
//! carte proportionnelle. La taille d'un dossier est celle de tout son contenu.
//!
//! Les nœuds ne bougent jamais (leur indice sert d'identifiant) : un nœud retiré est
//! seulement détaché de son parent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::entry::attrs;
use crate::names::{NameArena, NameSpan};
use crate::snapshot::extension;
use crate::wtf8;

/// Absence de nœud (fin de liste, racine sans parent).
pub const NONE: u32 = u32::MAX;
/// Au-delà, le contenu des nouveaux dossiers est regroupé dans leur parent (les totaux
/// restent exacts) : ~70 Mo pour l'arbre, borne de mémoire d'une analyse.
pub const MAX_NODES: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeKind {
    Dir,
    File,
    /// Fichiers du dossier parent trop petits pour être gardés un par un.
    SmallFiles,
}

/// Indicateurs d'un nœud.
pub mod flags {
    /// Dossier illisible (accès refusé…) : sa taille est inconnue.
    pub const DENIED: u8 = 1;
    /// Dossier marqué `CACHEDIR.TAG` (sortie de compilation, cache régénérable).
    pub const CACHE_TAG: u8 = 1 << 1;
    pub const REMOVED: u8 = 1 << 2;
    pub const HIDDEN: u8 = 1 << 3;
    pub const SYSTEM: u8 = 1 << 4;
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Node {
    name: NameSpan,
    /// Octets sur le disque (contenu entier pour un dossier).
    size: u64,
    /// FILETIME UTC brut.
    modified: i64,
    parent: u32,
    first_child: u32,
    next_sibling: u32,
    /// Fichiers contenus (1 pour un fichier).
    files: u32,
    /// Indice dans la table des extensions (0 : aucune).
    ext: u16,
    kind: NodeKind,
    flags: u8,
}

const _: () = assert!(size_of::<Node>() == 48, "budget : 48 octets par nœud");

/// Un dossier lu par un worker, avant insertion dans l'arbre.
#[derive(Debug, Default)]
pub struct DirListing {
    pub names: NameArena,
    pub dirs: Vec<ListedDir>,
    /// Fichiers gardés un par un (au moins `keep_files_from` octets).
    pub files: Vec<ListedFile>,
    pub small_files: u32,
    pub small_bytes: u64,
    /// Extensions de tous les fichiers (minuscules) : fichiers, octets.
    pub exts: Vec<(Box<[u8]>, u32, u64)>,
    pub denied: bool,
    pub cache_tag: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ListedDir {
    pub name: NameSpan,
    pub modified: i64,
    pub attributes: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ListedFile {
    pub name: NameSpan,
    pub size: u64,
    pub modified: i64,
    pub attributes: u32,
}

impl DirListing {
    /// Compte un fichier dans les extensions.
    pub fn count_ext(&mut self, ext: &[u8], bytes: u64) {
        match self.exts.iter_mut().find(|(e, ..)| &**e == ext) {
            Some((_, n, b)) => {
                *n += 1;
                *b += bytes;
            }
            None => self.exts.push((ext.into(), 1, bytes)),
        }
    }

    pub fn file_bytes(&self) -> u64 {
        self.small_bytes + self.files.iter().map(|f| f.size).sum::<u64>()
    }

    pub fn file_count(&self) -> u32 {
        self.small_files + self.files.len() as u32
    }
}

/// Place occupée par une extension (toute l'analyse).
#[derive(Debug, Clone)]
pub struct ExtStat {
    /// Minuscules, sans le point ; vide : sans extension.
    pub name: Box<[u8]>,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug)]
pub struct DiskTree {
    root: PathBuf,
    names: NameArena,
    nodes: Vec<Node>,
    ext_ids: HashMap<Box<[u8]>, u16>,
    exts: Vec<ExtStat>,
    keep_files_from: u64,
    dirs_read: u64,
    denied: u32,
    truncated: bool,
    revision: u64,
}

impl DiskTree {
    pub fn new(root: PathBuf, keep_files_from: u64) -> Self {
        let mut names = NameArena::default();
        let name = names.push_bytes(b"");
        Self {
            root,
            names,
            nodes: vec![Node {
                name,
                size: 0,
                modified: 0,
                parent: NONE,
                first_child: NONE,
                next_sibling: NONE,
                files: 0,
                ext: 0,
                kind: NodeKind::Dir,
                flags: 0,
            }],
            ext_ids: HashMap::new(),
            exts: vec![ExtStat {
                name: Box::default(),
                files: 0,
                bytes: 0,
            }],
            keep_files_from,
            dirs_read: 0,
            denied: 0,
            truncated: false,
            revision: 0,
        }
    }

    pub const fn root() -> u32 {
        0
    }

    pub fn root_path(&self) -> &Path {
        &self.root
    }

    /// Seuil à partir duquel un fichier a son propre nœud (le dernier appliqué : il peut
    /// être relevé en cours d'analyse).
    pub fn keep_files_from(&self) -> u64 {
        self.keep_files_from
    }

    pub fn set_keep_files_from(&mut self, bytes: u64) {
        self.keep_files_from = bytes;
    }

    /// Change à chaque modification (caches de l'UI).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Nombre de nœuds (retirés compris).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1 && self.nodes[0].first_child == NONE
    }

    pub fn dirs_read(&self) -> u64 {
        self.dirs_read
    }

    /// Dossiers illisibles rencontrés.
    pub fn denied(&self) -> u32 {
        self.denied
    }

    /// Plafond de nœuds atteint : une partie des dossiers est regroupée dans son parent.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    pub fn heap_bytes(&self) -> usize {
        self.nodes.capacity() * size_of::<Node>()
            + self.names.capacity()
            + self
                .exts
                .iter()
                .map(|e| e.name.len() + size_of::<ExtStat>() + 24)
                .sum::<usize>()
    }

    // --- Lecture -------------------------------------------------------------------

    pub fn name(&self, node: u32) -> &[u8] {
        self.names.get(self.nodes[node as usize].name)
    }

    pub fn size(&self, node: u32) -> u64 {
        self.nodes[node as usize].size
    }

    pub fn files(&self, node: u32) -> u32 {
        self.nodes[node as usize].files
    }

    pub fn kind(&self, node: u32) -> NodeKind {
        self.nodes[node as usize].kind
    }

    pub fn flags(&self, node: u32) -> u8 {
        self.nodes[node as usize].flags
    }

    pub fn modified(&self, node: u32) -> i64 {
        self.nodes[node as usize].modified
    }

    pub fn parent(&self, node: u32) -> Option<u32> {
        let p = self.nodes[node as usize].parent;
        (p != NONE).then_some(p)
    }

    /// Indice d'extension (cf. [`DiskTree::exts`]) ; 0 : aucune, ou pas un fichier.
    pub fn ext(&self, node: u32) -> u16 {
        self.nodes[node as usize].ext
    }

    /// Extensions (l'indice 0 regroupe les fichiers sans extension), état à la fin de
    /// l'analyse : les suppressions ultérieures n'y sont pas déduites.
    pub fn exts(&self) -> &[ExtStat] {
        &self.exts
    }

    /// Enfants, du plus gros au plus petit une fois l'analyse terminée.
    pub fn children(&self, node: u32) -> Children<'_> {
        Children {
            tree: self,
            next: self.nodes[node as usize].first_child,
        }
    }

    pub fn is_alive(&self, node: u32) -> bool {
        (node as usize) < self.nodes.len()
            && self.nodes[node as usize].flags & flags::REMOVED == 0
            && (node == 0 || self.nodes[node as usize].parent != NONE)
    }

    pub fn path(&self, node: u32) -> PathBuf {
        let mut chain = Vec::new();
        let mut n = node;
        while n != 0 && n != NONE {
            chain.push(n);
            n = self.nodes[n as usize].parent;
        }
        let mut p = self.root.clone();
        for n in chain.into_iter().rev() {
            p.push(wtf8::to_os_string(self.name(n)));
        }
        p
    }

    /// Enfant nommé (sans tenir compte de la casse, comme NTFS) ; les « petits fichiers »
    /// n'ont pas de nom.
    pub fn child_by_name(&self, node: u32, name: &[u8]) -> Option<u32> {
        self.children(node)
            .find(|&c| self.kind(c) != NodeKind::SmallFiles && same_name(self.name(c), name))
    }

    /// Nœud d'un chemin situé sous la racine.
    pub fn find(&self, path: &Path) -> Option<u32> {
        let mut rest = path.components();
        for want in self.root.components() {
            let got = rest.next()?;
            if !same_name(
                want.as_os_str().as_encoded_bytes(),
                got.as_os_str().as_encoded_bytes(),
            ) {
                return None;
            }
        }
        let mut node = 0;
        for c in rest {
            node = self.child_by_name(node, c.as_os_str().as_encoded_bytes())?;
        }
        Some(node)
    }

    /// Noms des sous-dossiers (actualisation d'un dossier : seuls les nouveaux sont lus).
    pub fn subdir_names(&self, node: u32) -> Vec<Vec<u8>> {
        self.children(node)
            .filter(|&c| self.kind(c) == NodeKind::Dir)
            .map(|c| self.name(c).to_vec())
            .collect()
    }

    // --- Construction --------------------------------------------------------------

    /// Ajoute le contenu lu d'un dossier ; renvoie les identifiants de ses sous-dossiers,
    /// dans l'ordre de `listing.dirs`.
    pub fn insert(&mut self, node: u32, listing: &DirListing) -> std::ops::Range<u32> {
        let delta = self.names.append(&listing.names);
        let first = self.nodes.len() as u32;
        for d in &listing.dirs {
            self.push_child(
                node,
                Node {
                    name: d.name.shifted(delta),
                    size: 0,
                    modified: d.modified,
                    parent: node,
                    first_child: NONE,
                    next_sibling: NONE,
                    files: 0,
                    ext: 0,
                    kind: NodeKind::Dir,
                    flags: attr_flags(d.attributes),
                },
            );
        }
        let dirs = first..self.nodes.len() as u32;
        self.add_files(node, listing, delta);
        for (ext, files, bytes) in &listing.exts {
            let id = self.intern(ext);
            let e = &mut self.exts[id as usize];
            e.files += u64::from(*files);
            e.bytes += bytes;
        }
        self.dirs_read += 1;
        self.revision += 1;
        dirs
    }

    /// Ajoute tout le contenu lu d'un dossier aux « petits fichiers » de `node`, sans créer
    /// de nœud pour ses sous-dossiers (plafond atteint ; ils sont lus et comptés dans `node`).
    pub fn insert_folded(&mut self, node: u32, listing: &DirListing) {
        self.add_small(node, listing.file_count(), listing.file_bytes());
        if listing.denied {
            self.denied += 1;
        }
        for (ext, files, bytes) in &listing.exts {
            let id = self.intern(ext);
            let e = &mut self.exts[id as usize];
            e.files += u64::from(*files);
            e.bytes += bytes;
        }
        self.truncated = true;
        self.dirs_read += 1;
        self.revision += 1;
    }

    /// Fichiers, petits fichiers et indicateurs d'un dossier lu.
    fn add_files(&mut self, node: u32, listing: &DirListing, delta: u32) {
        for f in &listing.files {
            let name = f.name.shifted(delta);
            let ext = self.intern(&lower(extension(self.names.get(name))));
            self.push_child(
                node,
                Node {
                    name,
                    size: f.size,
                    modified: f.modified,
                    parent: node,
                    first_child: NONE,
                    next_sibling: NONE,
                    files: 1,
                    ext,
                    kind: NodeKind::File,
                    flags: attr_flags(f.attributes),
                },
            );
            self.grow(node, f.size, 1);
        }
        self.add_small(node, listing.small_files, listing.small_bytes);
        let n = &mut self.nodes[node as usize];
        if listing.denied {
            n.flags |= flags::DENIED;
            self.denied += 1;
        }
        if listing.cache_tag {
            n.flags |= flags::CACHE_TAG;
        }
    }

    fn add_small(&mut self, node: u32, files: u32, bytes: u64) {
        if files == 0 {
            return;
        }
        let existing = self
            .children(node)
            .find(|&c| self.kind(c) == NodeKind::SmallFiles);
        let id = match existing {
            Some(id) => id,
            None => {
                let name = self.names.push_bytes(b"");
                self.push_child(
                    node,
                    Node {
                        name,
                        size: 0,
                        modified: 0,
                        parent: node,
                        first_child: NONE,
                        next_sibling: NONE,
                        files: 0,
                        ext: 0,
                        kind: NodeKind::SmallFiles,
                        flags: 0,
                    },
                )
            }
        };
        let n = &mut self.nodes[id as usize];
        n.size += bytes;
        n.files += files;
        self.grow(node, bytes, files);
    }

    fn push_child(&mut self, parent: u32, mut n: Node) -> u32 {
        let id = self.nodes.len() as u32;
        n.parent = parent;
        n.next_sibling = self.nodes[parent as usize].first_child;
        self.nodes.push(n);
        self.nodes[parent as usize].first_child = id;
        id
    }

    fn intern(&mut self, ext: &[u8]) -> u16 {
        if ext.is_empty() {
            return 0;
        }
        if let Some(&id) = self.ext_ids.get(ext) {
            return id;
        }
        if self.exts.len() > usize::from(u16::MAX) {
            return 0;
        }
        let id = self.exts.len() as u16;
        self.exts.push(ExtStat {
            name: ext.into(),
            files: 0,
            bytes: 0,
        });
        self.ext_ids.insert(ext.into(), id);
        id
    }

    /// Ajoute `bytes` et `files` à `node` et à tous ses parents.
    fn grow(&mut self, mut node: u32, bytes: u64, files: u32) {
        while node != NONE {
            let n = &mut self.nodes[node as usize];
            n.size += bytes;
            n.files += files;
            node = n.parent;
        }
    }

    fn shrink(&mut self, mut node: u32, bytes: u64, files: u32) {
        while node != NONE {
            let n = &mut self.nodes[node as usize];
            n.size = n.size.saturating_sub(bytes);
            n.files = n.files.saturating_sub(files);
            node = n.parent;
        }
    }

    /// Trie les enfants de chaque dossier, du plus gros au plus petit, et rend la place
    /// réservée en trop par les tableaux (fin d'analyse).
    pub fn finish(&mut self) {
        self.nodes.shrink_to_fit();
        self.names.shrink_to_fit();
        let mut scratch = Vec::new();
        for id in 0..self.nodes.len() as u32 {
            if self.nodes[id as usize].first_child != NONE {
                self.sort_children(id, &mut scratch);
            }
        }
        self.revision += 1;
    }

    fn sort_children(&mut self, node: u32, scratch: &mut Vec<u32>) {
        scratch.clear();
        scratch.extend(self.children(node));
        scratch.sort_by(|&a, &b| {
            self.size(b)
                .cmp(&self.size(a))
                .then_with(|| crate::natural_cmp(self.name(a), self.name(b)))
        });
        let mut next = NONE;
        for &c in scratch.iter().rev() {
            self.nodes[c as usize].next_sibling = next;
            next = c;
        }
        self.nodes[node as usize].first_child = next;
    }

    // --- Modifications après l'analyse -----------------------------------------------

    /// Détache un nœud (supprimé, déplacé) et retire sa taille de ses parents.
    pub fn remove(&mut self, node: u32) {
        let Some(parent) = self.parent(node) else {
            return;
        };
        let mut prev = NONE;
        let mut c = self.nodes[parent as usize].first_child;
        while c != NONE && c != node {
            prev = c;
            c = self.nodes[c as usize].next_sibling;
        }
        if c == NONE {
            return;
        }
        let next = self.nodes[node as usize].next_sibling;
        if prev == NONE {
            self.nodes[parent as usize].first_child = next;
        } else {
            self.nodes[prev as usize].next_sibling = next;
        }
        let (size, files) = (self.size(node), self.files(node));
        let n = &mut self.nodes[node as usize];
        n.parent = NONE;
        n.next_sibling = NONE;
        n.flags |= flags::REMOVED;
        self.shrink(parent, size, files);
        self.revision += 1;
    }

    /// Remplace le contenu direct d'un dossier relu : fichiers et petits fichiers
    /// recomptés, sous-dossiers disparus retirés, nouveaux sous-dossiers (`fresh`, lus en
    /// entier) greffés. Les sous-dossiers déjà connus gardent leur contenu.
    pub fn refresh(&mut self, node: u32, listing: &DirListing, fresh: &[(Vec<u8>, DiskTree)]) {
        let stale: Vec<u32> = self
            .children(node)
            .filter(|&c| match self.kind(c) {
                NodeKind::Dir => !listing
                    .dirs
                    .iter()
                    .any(|d| same_name(listing.names.get(d.name), self.name(c))),
                NodeKind::File | NodeKind::SmallFiles => true,
            })
            .collect();
        for c in stale {
            self.remove(c);
        }
        let n = &mut self.nodes[node as usize];
        n.flags &= !(flags::DENIED | flags::CACHE_TAG);
        let delta = self.names.append(&listing.names);
        self.add_files(node, listing, delta);
        for (name, sub) in fresh {
            if self.child_by_name(node, name).is_none() {
                self.graft(node, name, sub);
            }
        }
        // Les tailles ont changé jusqu'à la racine : l'ordre des frères aussi.
        let mut scratch = Vec::new();
        let mut n = node;
        while n != NONE {
            self.sort_children(n, &mut scratch);
            n = self.nodes[n as usize].parent;
        }
        self.revision += 1;
    }

    /// Copie l'arbre `sub` sous `parent`, sous le nom `name`.
    fn graft(&mut self, parent: u32, name: &[u8], sub: &DiskTree) {
        let root = sub.nodes[0];
        let span = self.names.push_bytes(name);
        let top = self.push_child(
            parent,
            Node {
                name: span,
                first_child: NONE,
                ..root
            },
        );
        self.grow(parent, root.size, root.files);
        // (nœud de `sub`, nœud copié) dont il reste à copier les enfants. L'ordre des
        // enfants de `sub` (trié) est conservé : on les ajoute du dernier au premier.
        let mut stack = vec![(0u32, top)];
        let mut kids = Vec::new();
        while let Some((from, to)) = stack.pop() {
            kids.clear();
            kids.extend(sub.children(from));
            for &c in kids.iter().rev() {
                let n = sub.nodes[c as usize];
                let span = self.names.push_bytes(sub.name(c));
                let ext = if n.kind == NodeKind::File {
                    self.intern(&sub.exts[n.ext as usize].name)
                } else {
                    0
                };
                let id = self.push_child(
                    to,
                    Node {
                        name: span,
                        first_child: NONE,
                        ext,
                        ..n
                    },
                );
                stack.push((c, id));
            }
        }
        self.denied += sub.denied;
        self.truncated |= sub.truncated;
    }
}

pub struct Children<'a> {
    tree: &'a DiskTree,
    next: u32,
}

impl Iterator for Children<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if self.next == NONE {
            return None;
        }
        let id = self.next;
        self.next = self.tree.nodes[id as usize].next_sibling;
        Some(id)
    }
}

fn attr_flags(a: u32) -> u8 {
    let mut f = 0;
    if a & attrs::HIDDEN != 0 {
        f |= flags::HIDDEN;
    }
    if a & attrs::SYSTEM != 0 {
        f |= flags::SYSTEM;
    }
    f
}

/// Extension en minuscules ASCII (clé de la table).
pub fn lower(ext: &[u8]) -> Vec<u8> {
    ext.to_ascii_lowercase()
}

/// Extension d'un nom (WTF-8), en minuscules ; vide s'il n'en a pas.
pub fn extension_of(name: &[u8]) -> Vec<u8> {
    lower(extension(name))
}

/// Comparaison de noms sans tenir compte de la casse (ASCII, puis Unicode si besoin).
pub fn same_name(a: &[u8], b: &[u8]) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    if a.is_ascii() && b.is_ascii() {
        return false;
    }
    wtf8::display(a).to_lowercase() == wtf8::display(b).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(dirs: &[&str], files: &[(&str, u64)], small: (u32, u64)) -> DirListing {
        let mut l = DirListing::default();
        for d in dirs {
            let name = l.names.push_bytes(d.as_bytes());
            l.dirs.push(ListedDir {
                name,
                modified: 0,
                attributes: attrs::DIRECTORY,
            });
        }
        for (f, size) in files {
            let name = l.names.push_bytes(f.as_bytes());
            l.files.push(ListedFile {
                name,
                size: *size,
                modified: 0,
                attributes: 0,
            });
            l.count_ext(&extension_of(f.as_bytes()), *size);
        }
        l.small_files = small.0;
        l.small_bytes = small.1;
        if small.0 > 0 {
            l.count_ext(b"txt", small.1);
        }
        l
    }

    /// C:\r : a\ (gros.iso 5000, b\ (x.bin 3000)), film.mkv 2000, 10 petits (100).
    fn sample() -> DiskTree {
        let mut t = DiskTree::new(PathBuf::from(r"C:\r"), 1000);
        let top = t.insert(0, &listing(&["a"], &[("film.mkv", 2000)], (10, 100)));
        let a = top.start;
        let sub = t.insert(a, &listing(&["b"], &[("gros.ISO", 5000)], (0, 0)));
        t.insert(sub.start, &listing(&[], &[("x.bin", 3000)], (0, 0)));
        t.finish();
        t
    }

    #[test]
    fn sizes_add_up_to_the_root_and_children_are_sorted() {
        let t = sample();
        assert_eq!(t.size(0), 10_100);
        assert_eq!(t.files(0), 13);
        let kids: Vec<u64> = t.children(0).map(|c| t.size(c)).collect();
        assert_eq!(kids, vec![8000, 2000, 100]);
        let a = t.find(Path::new(r"c:\R\A")).expect("a");
        assert_eq!(t.name(a), b"a");
        assert_eq!(t.path(a), PathBuf::from(r"C:\r\a"));
        let iso = t.find(Path::new(r"C:\r\a\GROS.iso")).expect("iso");
        assert_eq!(&*t.exts()[t.ext(iso) as usize].name, b"iso");
        assert_eq!(t.find(Path::new(r"D:\r\a")), None);
        assert_eq!(t.find(Path::new(r"C:\r\zz")), None);
    }

    #[test]
    fn extensions_cover_every_file() {
        let t = sample();
        let bytes = |e: &[u8]| {
            t.exts()
                .iter()
                .find(|x| &*x.name == e)
                .map(|x| (x.files, x.bytes))
        };
        assert_eq!(bytes(b"iso"), Some((1, 5000)));
        assert_eq!(bytes(b"txt"), Some((1, 100)));
        assert_eq!(bytes(b"mkv"), Some((1, 2000)));
    }

    #[test]
    fn remove_subtracts_from_every_parent() {
        let mut t = sample();
        let b = t.find(Path::new(r"C:\r\a\b")).expect("b");
        let rev = t.revision();
        t.remove(b);
        assert!(t.revision() > rev);
        assert!(!t.is_alive(b));
        assert_eq!(t.size(0), 7100);
        assert_eq!(t.files(0), 12);
        assert_eq!(t.find(Path::new(r"C:\r\a\b")), None);
        assert_eq!(t.size(t.find(Path::new(r"C:\r\a")).expect("a")), 5000);
    }

    #[test]
    fn refresh_recounts_files_and_grafts_new_dirs() {
        let mut t = sample();
        // a\ : gros.iso supprimé, b\ toujours là, nouveau dossier c\ (y.bin 4000).
        let a = t.find(Path::new(r"C:\r\a")).expect("a");
        let mut c = DiskTree::new(PathBuf::from(r"C:\r\a\c"), 1000);
        c.insert(0, &listing(&[], &[("y.bin", 4000)], (2, 20)));
        c.finish();
        t.refresh(
            a,
            &listing(&["b", "c"], &[], (1, 10)),
            &[(b"c".to_vec(), c)],
        );
        assert_eq!(t.size(a), 3000 + 4020 + 10);
        assert_eq!(t.size(0), 7030 + 2100);
        let c = t.find(Path::new(r"C:\r\a\c")).expect("c");
        assert_eq!(t.files(c), 3);
        assert!(t.find(Path::new(r"C:\r\a\c\y.bin")).is_some());
        assert!(t.find(Path::new(r"C:\r\a\gros.iso")).is_none());
        // Ordre par taille après l'actualisation : c (4020), b (3000), petits (10).
        let kids: Vec<u64> = t.children(a).map(|n| t.size(n)).collect();
        assert_eq!(kids, vec![4020, 3000, 10]);
    }

    #[test]
    fn folded_content_keeps_totals_exact() {
        let mut t = DiskTree::new(PathBuf::from(r"C:\r"), 1000);
        t.insert_folded(0, &listing(&["a", "b"], &[("x.bin", 5000)], (3, 30)));
        assert!(t.is_truncated());
        assert_eq!(t.size(0), 5030);
        assert_eq!(t.files(0), 4);
        assert_eq!(t.children(0).count(), 1);
    }
}
