//! Arborescence de la barre latérale : sous-dossiers des épingles, des lecteurs et des
//! distributions WSL, déroulés à la demande.
//!
//! Seuls les dossiers ouverts sont gardés en mémoire : refermer un dossier libère tout son
//! sous-arbre. Les noms d'un dossier tiennent dans une seule [`NameArena`] (pas d'objet par
//! sous-dossier) et sont lus sur un thread d'I/O (`Effect::ListSubdirs`).

use std::path::{Component, Path, PathBuf};

use fx_core::{CancelToken, NameArena, NameSpan, natural_cmp};

use crate::effect::Effect;

/// Dossiers ouverts au plus en même temps (toutes racines confondues).
pub const MAX_OPEN: usize = 256;
/// Sous-dossiers gardés par dossier ouvert (les suivants ne sont pas affichés).
pub const MAX_CHILDREN: usize = 20_000;

/// Refus d'ouvrir un dossier : [`MAX_OPEN`] dossiers déjà ouverts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooManyOpen;

/// Section de la barre latérale qui porte une racine : un même dossier épinglé et lecteur
/// se déroule indépendamment dans chaque section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TreeSection {
    Pinned,
    Drives,
    Wsl,
}

/// Sous-dossiers d'un dossier, triés dans l'ordre naturel.
#[derive(Debug, Default, Clone)]
pub struct SubDirs {
    names: NameArena,
    entries: Vec<(NameSpan, u32)>,
    /// Sous-dossiers non gardés au-delà de [`MAX_CHILDREN`].
    pub truncated: usize,
}

impl SubDirs {
    /// Ajoute un sous-dossier (nom WTF-8 et attributs Win32).
    pub fn push(&mut self, name: &[u8], attributes: u32) {
        if self.entries.len() >= MAX_CHILDREN {
            self.truncated += 1;
            return;
        }
        let span = self.names.push_bytes(name);
        self.entries.push((span, attributes));
    }

    /// Tri naturel, comme la liste (`StrCmpLogicalW`), puis mémoire rendue.
    pub fn finish(&mut self) {
        let names = &self.names;
        self.entries
            .sort_by(|a, b| natural_cmp(names.get(a.0), names.get(b.0)));
        self.entries.shrink_to_fit();
        self.names.shrink_to_fit();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Nom WTF-8 du `i`-ième sous-dossier.
    pub fn name(&self, i: usize) -> &[u8] {
        self.names.get(self.entries[i].0)
    }

    pub fn attributes(&self, i: usize) -> u32 {
        self.entries[i].1
    }

    /// Position de `name` (ordre naturel, donc recherche dichotomique).
    pub fn find(&self, name: &[u8]) -> Option<usize> {
        self.entries
            .binary_search_by(|e| natural_cmp(self.names.get(e.0), name))
            .ok()
    }

    pub fn heap_bytes(&self) -> usize {
        self.names.capacity() + self.entries.capacity() * size_of::<(NameSpan, u32)>()
    }
}

/// Dossier ouvert de l'arborescence.
#[derive(Debug)]
pub struct TreeNode {
    pub path: PathBuf,
    /// Contenu connu (vide tant que la première lecture n'est pas arrivée).
    pub children: SubDirs,
    /// Lecture en cours (première ou relecture : l'ancien contenu reste affiché).
    pub loading: bool,
    /// Au moins une lecture a abouti (sinon : chargement ou erreur).
    pub loaded: bool,
    pub error: Option<String>,
    /// Sous-dossiers ouverts, par position croissante dans `children`.
    pub open: Vec<(u32, TreeNode)>,
    generation: u64,
    cancel: CancelToken,
    /// Changement signalé pendant une lecture : relire à son arrivée.
    dirty: bool,
}

impl TreeNode {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            children: SubDirs::default(),
            loading: false,
            loaded: false,
            error: None,
            open: Vec::new(),
            generation: 0,
            cancel: CancelToken::new(),
            dirty: false,
        }
    }

    /// Sous-dossier ouvert à la position `i`.
    pub fn open_child(&self, i: usize) -> Option<&TreeNode> {
        self.open
            .binary_search_by_key(&(i as u32), |(k, _)| *k)
            .ok()
            .map(|k| &self.open[k].1)
    }

    /// Chemin du `i`-ième sous-dossier.
    pub fn child_path(&self, i: usize) -> PathBuf {
        self.path
            .join(fx_core::wtf8::to_os_string(self.children.name(i)))
    }

    fn cancel_all(&self) {
        self.cancel.cancel();
        for (_, c) in &self.open {
            c.cancel_all();
        }
    }

    fn count(&self) -> usize {
        1 + self.open.iter().map(|(_, c)| c.count()).sum::<usize>()
    }

    fn heap_bytes(&self) -> usize {
        self.children.heap_bytes()
            + self.open.capacity() * size_of::<(u32, TreeNode)>()
            + self.open.iter().map(|(_, c)| c.heap_bytes()).sum::<usize>()
    }

    fn find_generation(&mut self, generation: u64) -> Option<&mut TreeNode> {
        if self.generation == generation && self.loading {
            return Some(self);
        }
        self.open
            .iter_mut()
            .find_map(|(_, c)| c.find_generation(generation))
    }

    fn visit_mut(&mut self, f: &mut impl FnMut(&mut TreeNode)) {
        f(self);
        for (_, c) in &mut self.open {
            c.visit_mut(f);
        }
    }
}

/// Dossiers déroulés de la barre latérale.
#[derive(Debug, Default)]
pub struct FolderTree {
    roots: Vec<(TreeSection, TreeNode)>,
    next_generation: u64,
}

impl FolderTree {
    /// Racine `path` de `section`, si elle est ouverte.
    pub fn root(&self, section: TreeSection, path: &Path) -> Option<&TreeNode> {
        self.roots
            .iter()
            .find(|(s, n)| *s == section && n.path == path)
            .map(|(_, n)| n)
    }

    /// Nombre de dossiers ouverts (borné par [`MAX_OPEN`]).
    pub fn len(&self) -> usize {
        self.roots.iter().map(|(_, n)| n.count()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    pub fn heap_bytes(&self) -> usize {
        self.roots.capacity() * size_of::<(TreeSection, TreeNode)>()
            + self
                .roots
                .iter()
                .map(|(_, n)| n.heap_bytes())
                .sum::<usize>()
    }

    /// Ouvre ou referme `path`, racine `root` de `section` ou l'un de ses descendants
    /// (dont chaque parent est ouvert). Renvoie la lecture à lancer, s'il y en a une.
    pub fn toggle(
        &mut self,
        section: TreeSection,
        root: &Path,
        path: &Path,
        show_hidden: bool,
    ) -> Result<Option<Effect>, TooManyOpen> {
        let full = self.len() >= MAX_OPEN;
        let Some(ri) = self
            .roots
            .iter()
            .position(|(s, n)| *s == section && n.path == root)
        else {
            if path != root {
                return Ok(None);
            }
            if full {
                return Err(TooManyOpen);
            }
            self.roots
                .push((section, TreeNode::new(root.to_path_buf())));
            let node = &mut self.roots.last_mut().expect("racine ajoutée").1;
            return Ok(Some(request(&mut self.next_generation, node, show_hidden)));
        };
        if path == root {
            let (_, node) = self.roots.remove(ri);
            node.cancel_all();
            return Ok(None);
        }
        let Ok(rel) = path.strip_prefix(root) else {
            return Ok(None);
        };
        let names: Vec<&[u8]> = rel
            .components()
            .filter_map(|c| match c {
                Component::Normal(n) => Some(n.as_encoded_bytes()),
                _ => None,
            })
            .collect();
        let Some((last, parents)) = names.split_last() else {
            return Ok(None);
        };
        let mut node = &mut self.roots[ri].1;
        for name in parents {
            let Some(i) = node.children.find(name) else {
                return Ok(None);
            };
            let Ok(k) = node.open.binary_search_by_key(&(i as u32), |(k, _)| *k) else {
                return Ok(None);
            };
            node = &mut node.open[k].1;
        }
        let Some(i) = node.children.find(last) else {
            return Ok(None);
        };
        match node.open.binary_search_by_key(&(i as u32), |(k, _)| *k) {
            Ok(k) => {
                let (_, child) = node.open.remove(k);
                child.cancel_all();
                Ok(None)
            }
            Err(_) if full => Err(TooManyOpen),
            Err(k) => {
                let child_path = node.child_path(i);
                node.open.insert(k, (i as u32, TreeNode::new(child_path)));
                let child = &mut node.open[k].1;
                Ok(Some(request(&mut self.next_generation, child, show_hidden)))
            }
        }
    }

    /// Résultat d'une lecture : remplace le contenu du dossier, garde ouverts les
    /// sous-dossiers toujours présents, relit si un changement est arrivé entre-temps.
    pub fn loaded(
        &mut self,
        generation: u64,
        result: Result<SubDirs, String>,
        show_hidden: bool,
    ) -> Option<Effect> {
        let node = self
            .roots
            .iter_mut()
            .find_map(|(_, n)| n.find_generation(generation))?;
        node.loading = false;
        match result {
            Ok(children) => {
                let old = std::mem::replace(&mut node.children, children);
                let open = std::mem::take(&mut node.open);
                for (i, child) in open {
                    match node.children.find(old.name(i as usize)) {
                        Some(j) => node.open.push((j as u32, child)),
                        None => child.cancel_all(),
                    }
                }
                node.open.sort_by_key(|(k, _)| *k);
                node.loaded = true;
                node.error = None;
            }
            Err(e) => {
                for (_, c) in std::mem::take(&mut node.open) {
                    c.cancel_all();
                }
                node.children = SubDirs::default();
                node.error = Some(e);
            }
        }
        if std::mem::take(&mut node.dirty) {
            return Some(request(&mut self.next_generation, node, show_hidden));
        }
        None
    }

    /// Les sous-dossiers de `dir` ont changé : relit les dossiers ouverts qui le montrent.
    pub fn changed(&mut self, dir: &Path, show_hidden: bool) -> Vec<Effect> {
        let mut fx = Vec::new();
        let next = &mut self.next_generation;
        for (_, root) in &mut self.roots {
            root.visit_mut(&mut |n| {
                if same_dir(&n.path, dir) {
                    if n.loading {
                        n.dirty = true;
                    } else {
                        fx.push(request(next, n, show_hidden));
                    }
                }
            });
        }
        fx
    }

    /// Le dossier ouvert `dir` contient-il déjà ce sous-dossier ?
    pub fn lists(&self, dir: &Path, name: &[u8]) -> bool {
        fn walk(n: &TreeNode, dir: &Path, name: &[u8]) -> bool {
            (same_dir(&n.path, dir) && n.children.find(name).is_some())
                || n.open.iter().any(|(_, c)| walk(c, dir, name))
        }
        self.roots.iter().any(|(_, n)| walk(n, dir, name))
    }

    /// Le dossier `dir` est-il ouvert dans l'arborescence ?
    pub fn is_open(&self, dir: &Path) -> bool {
        fn walk(n: &TreeNode, dir: &Path) -> bool {
            same_dir(&n.path, dir) || n.open.iter().any(|(_, c)| walk(c, dir))
        }
        self.roots.iter().any(|(_, n)| walk(n, dir))
    }

    /// Relit tous les dossiers ouverts (Actualiser, éléments masqués affichés ou non).
    pub fn reload_all(&mut self, show_hidden: bool) -> Vec<Effect> {
        let mut fx = Vec::new();
        let next = &mut self.next_generation;
        for (_, root) in &mut self.roots {
            root.visit_mut(&mut |n| {
                if n.loading {
                    n.dirty = true;
                } else {
                    fx.push(request(next, n, show_hidden));
                }
            });
        }
        fx
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
}

fn request(next: &mut u64, node: &mut TreeNode, show_hidden: bool) -> Effect {
    *next += 1;
    node.generation = *next;
    node.loading = true;
    node.cancel = CancelToken::new();
    Effect::ListSubdirs {
        generation: node.generation,
        path: node.path.clone(),
        show_hidden,
        cancel: node.cancel.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subdirs(names: &[&str]) -> SubDirs {
        let mut s = SubDirs::default();
        for n in names {
            s.push(n.as_bytes(), 0x10);
        }
        s.finish();
        s
    }

    fn generation(fx: Option<Effect>) -> u64 {
        match fx {
            Some(Effect::ListSubdirs { generation, .. }) => generation,
            other => panic!("lecture attendue, obtenu {other:?}"),
        }
    }

    #[test]
    fn subdirs_sorted_naturally() {
        let s = subdirs(&["b10", "a", "b2"]);
        let names: Vec<_> = (0..s.len()).map(|i| s.name(i)).collect();
        assert_eq!(names, [&b"a"[..], b"b2", b"b10"]);
        assert_eq!(s.find(b"b10"), Some(2));
        assert_eq!(s.find(b"zz"), None);
    }

    #[test]
    fn expand_nested_and_collapse_frees_subtree() {
        let mut t = FolderTree::default();
        let root = Path::new(r"C:\");
        let g = generation(
            t.toggle(TreeSection::Drives, root, root, false)
                .expect("attendu"),
        );
        assert!(t.root(TreeSection::Drives, root).expect("attendu").loading);
        assert!(t.root(TreeSection::Pinned, root).is_none());
        assert!(
            t.loaded(g, Ok(subdirs(&["Users", "Windows"])), false)
                .is_none()
        );
        let users = Path::new(r"C:\Users");
        let g = generation(
            t.toggle(TreeSection::Drives, root, users, false)
                .expect("attendu"),
        );
        assert!(t.loaded(g, Ok(subdirs(&["Public"])), false).is_none());
        let public = Path::new(r"C:\Users\Public");
        t.toggle(TreeSection::Drives, root, public, false)
            .expect("attendu");
        assert_eq!(t.len(), 3);
        let node = t.root(TreeSection::Drives, root).expect("attendu");
        assert_eq!(node.open_child(1).map(|n| n.path.as_path()), None);
        assert_eq!(node.open_child(0).expect("attendu").path, users);
        // Refermer Users libère aussi Public.
        assert!(
            t.toggle(TreeSection::Drives, root, users, false)
                .expect("attendu")
                .is_none()
        );
        assert_eq!(t.len(), 1);
        t.toggle(TreeSection::Drives, root, root, false)
            .expect("attendu");
        assert!(t.is_empty());
    }

    #[test]
    fn reload_keeps_open_children_by_name() {
        let mut t = FolderTree::default();
        let root = Path::new(r"D:\");
        let g = generation(
            t.toggle(TreeSection::Drives, root, root, false)
                .expect("attendu"),
        );
        t.loaded(g, Ok(subdirs(&["b", "c"])), false);
        let c = Path::new(r"D:\c");
        let gc = generation(
            t.toggle(TreeSection::Drives, root, c, false)
                .expect("attendu"),
        );
        t.loaded(gc, Ok(subdirs(&[])), false);
        // Nouveau dossier « a » : « c » passe de la position 1 à 2 et reste ouvert.
        let fx = t.changed(Path::new(r"d:\"), false);
        assert_eq!(fx.len(), 1);
        let g = generation(fx.into_iter().next());
        t.loaded(g, Ok(subdirs(&["a", "b", "c"])), false);
        let node = t.root(TreeSection::Drives, root).expect("attendu");
        assert_eq!(node.open_child(2).expect("attendu").path, c);
        // « c » supprimé : il est refermé.
        let g = generation(t.changed(root, false).into_iter().next());
        t.loaded(g, Ok(subdirs(&["a"])), false);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn change_during_load_rereads_and_stale_results_ignored() {
        let mut t = FolderTree::default();
        let root = Path::new(r"E:\");
        let g = generation(
            t.toggle(TreeSection::Wsl, root, root, false)
                .expect("attendu"),
        );
        assert!(t.changed(root, false).is_empty());
        let again = generation(t.loaded(g, Ok(subdirs(&["x"])), false));
        assert_ne!(again, g);
        // Ancienne génération : ignorée.
        assert!(t.loaded(g, Ok(subdirs(&[])), false).is_none());
        assert!(t.lists(root, b"x"));
        // Refermé avant la réponse : ignorée aussi.
        t.toggle(TreeSection::Wsl, root, root, false)
            .expect("attendu");
        assert!(t.loaded(again, Ok(subdirs(&["y"])), false).is_none());
    }

    #[test]
    fn open_count_is_bounded() {
        let mut t = FolderTree::default();
        for i in 0..MAX_OPEN {
            let p = PathBuf::from(format!(r"C:\p{i}"));
            assert!(t.toggle(TreeSection::Pinned, &p, &p, false).is_ok());
        }
        let p = Path::new(r"C:\trop");
        assert!(t.toggle(TreeSection::Pinned, p, p, false).is_err());
    }
}
