//! Portage fx : rendu partiel et défilement par décalage (ARCHITECTURE.md de fx, §9, pistes 2
//! et 3). Absent de la crate d'origine.
//!
//! Sans cache de primitives, chaque image rasterisait toute la fenêtre. Ici, la liste de
//! primitives de l'image est comparée à celle de l'image précédente (dont les pixels sont
//! encore dans le tampon) et seul ce qui a changé est redessiné :
//!
//! - **Appariement** : chaque primitive porte une empreinte absolue (texture, découpe,
//!   sommets, indices) et une empreinte relative (sommets exprimés par rapport au premier).
//!   Les primitives sont appariées dans l'ordre de dessin (appariement croissant) : soit
//!   identiques, soit translatées verticalement d'un nombre entier de pixels.
//! - **Défilement** : la translation la plus représentée parmi les primitives dont la découpe
//!   n'a pas bougé donne la région de défilement `R` (cette découpe) et le décalage `t`. Les
//!   pixels de `R` sont recopiés de `t` ; la bande découverte est à redessiner.
//! - **Zones endommagées** : primitives sans partenaire (à l'ancienne et à la nouvelle
//!   position), bande découverte et, pour les primitives fixes qui recouvrent `R` (fond de la
//!   carte, ombres de bord du défilement), les zones qui ne sont pas invariantes par
//!   translation verticale (bords des aplats, triangles non unis).
//!
//! Chaque zone endommagée est redessinée en entier (toutes les primitives qui la touchent,
//! découpées au rectangle, du fond vers le dessus) : en dehors des zones endommagées, le
//! contenu du tampon (éventuellement décalé) est celui qu'aurait produit un rendu complet.
//! Hypothèse : seul ce moteur écrit dans le tampon entre deux images.

use alloc::vec::Vec;
use core::mem;

use egui::{Color32, Mesh, Pos2, Rect, TextureId, Vec2, vec2};

use crate::{ColorFieldOrder, color::swizzle_rgba_bgra, render::egui_orient2df};

/// Tuiles de suivi des zones endommagées, en pixels. Peu hautes : les zones typiques sont des
/// bandes horizontales (lignes découvertes, ombres de bord du défilement).
const TILE_W: i32 = 16;
const TILE_H: i32 = 8;
/// Au-delà de cette fraction de tuiles endommagées, un rendu complet coûte autant (et évite
/// la recopie) : on redessine tout.
const FULL_REDRAW_RATIO: f32 = 0.6;
/// Au-delà, les rectangles à redessiner sont regroupés en bandes horizontales (chaque
/// rectangle coûte un parcours des primitives). Les rectangles recopiés ne coûtent que leurs
/// pixels : pas de limite.
const MAX_RECTS: usize = 384;
/// Hauteur (en rangées de tuiles) des bandes présentées : chaque rectangle présenté est un
/// appel `BitBlt`, dont le coût fixe dépasse celui de quelques pixels de trop.
const PRESENT_BAND: i32 = 8;
/// Précision de comparaison des positions (1/256 px, celle du rasteriseur).
const SUBPIX: f32 = 256.0;
/// Partenaires examinés par primitive pour détecter le défilement.
const MAX_CANDIDATES: usize = 64;

/// Rectangle de pixels `[x0, x1) × [y0, y1)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DamageRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl DamageRect {
    pub const EMPTY: Self = Self {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
    };

    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    pub fn width(&self) -> i32 {
        (self.x1 - self.x0).max(0)
    }

    pub fn height(&self) -> i32 {
        (self.y1 - self.y0).max(0)
    }

    pub fn area(&self) -> u64 {
        self.width() as u64 * self.height() as u64
    }

    pub(crate) fn intersect(&self, o: &Self) -> Self {
        Self {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        }
    }

    pub(crate) fn intersects(&self, o: &Self) -> bool {
        !self.intersect(o).is_empty()
    }

    /// Vrai si `o` est entièrement dans `self` (un rectangle vide est contenu partout).
    pub(crate) fn contains(&self, o: &Self) -> bool {
        o.is_empty() || (o.x0 >= self.x0 && o.y0 >= self.y0 && o.x1 <= self.x1 && o.y1 <= self.y1)
    }

    pub(crate) fn hull(&self, o: &Self) -> Self {
        Self {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    pub(crate) fn translate_y(&self, t: i32) -> Self {
        Self {
            y0: self.y0 + t,
            y1: self.y1 + t,
            ..*self
        }
    }

    /// Plus petit rectangle de pixels contenant tous les pixels dont le centre peut être
    /// couvert par la boîte `[min, max]`, avec une marge d'un pixel.
    pub(crate) fn around(min: Vec2, max: Vec2) -> Self {
        Self {
            x0: sat_i32(min.x.floor()) - 1,
            y0: sat_i32(min.y.floor()) - 1,
            x1: sat_i32(max.x.ceil()) + 1,
            y1: sat_i32(max.y.ceil()) + 1,
        }
    }

    pub(crate) fn to_rect(self) -> Rect {
        Rect::from_min_max(
            Pos2::new(self.x0 as f32, self.y0 as f32),
            Pos2::new(self.x1 as f32, self.y1 as f32),
        )
    }
}

fn sat_i32(v: f32) -> i32 {
    // `as` sature (et NaN donne 0) : pas de débordement possible.
    (v as i64).clamp(i32::MIN as i64 / 2, i32::MAX as i64 / 2) as i32
}

/// Résumé de la dernière image rendue par [`crate::EguiSoftwareRender::render_damaged`].
#[derive(Clone, Copy, Debug, Default)]
pub struct DamageReport {
    /// Image redessinée entièrement.
    pub full: bool,
    /// Région décalée et décalage vertical (pixels) appliqué à son contenu.
    pub scroll: Option<(DamageRect, i32)>,
    /// Pixels rasterisés (somme des rectangles redessinés).
    pub redrawn_px: u64,
    /// Pixels recopiés par le décalage.
    pub copied_px: u64,
}

/// Une primitive de l'image, en pixels.
#[derive(Clone, Copy)]
pub(crate) struct PrimInfo {
    /// Découpe en pixels (flottants), telle que passée au rasteriseur.
    pub clip: Rect,
    /// Découpe en pixels entiers, comme `draw_egui_mesh` la calcule.
    pub iclip: DamageRect,
    /// Pixels que la primitive peut toucher : boîte du maillage ∩ découpe.
    pub cov: DamageRect,
    /// Le maillage tient entièrement dans la découpe : elle ne change rien au rendu.
    pub unclipped: bool,
    pub texture: TextureId,
    pub h_abs: u64,
    pub h_rel: u64,
    /// Premier sommet, en 1/256 de pixel.
    pub qref: [i32; 2],
    /// Indice du maillage préparé dans `Frame::meshes`.
    pub mesh: u32,
    pub low_precision: bool,
    /// Boîte des coordonnées de texture (UV normalisées).
    pub uv: Rect,
}

#[derive(Default)]
struct Frame {
    prims: Vec<PrimInfo>,
    /// Maillages en pixels, réutilisés d'une image à l'autre (pas d'allocation en régime
    /// établi).
    meshes: Vec<Mesh>,
    width: usize,
    height: usize,
    ppp: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Match {
    None,
    Same,
    Moved,
}

#[derive(Clone, Copy)]
struct Edge {
    group: u16,
    a: u32,
    b: u32,
    /// Sommet opposé (pour savoir de quel côté de l'arête est le triangle).
    c: u32,
}

#[derive(Clone, Copy)]
struct Group {
    color: Color32,
    uv: [u32; 2],
    first: u32,
    last: u32,
    valid: bool,
}

/// État du rendu incrémental : image précédente et tampons réutilisés.
#[derive(Default)]
pub(crate) struct Incremental {
    cur: Frame,
    prev: Frame,
    prev_valid: bool,
    abs_index: Vec<(u64, u32)>,
    rel_index: Vec<(u64, u32)>,
    matches: Vec<Match>,
    /// Appariement de chaque ancienne primitive.
    old_matches: Vec<Match>,
    votes: Vec<(i32, DamageRect, u64)>,
    /// Zones de textures réécrites pour cette image (UV normalisées).
    dirty_textures: Vec<(TextureId, Rect)>,
    tiles: Vec<u8>,
    /// Tuiles à recopier lors d'un décalage (contenu mobile, avant ou après).
    copy_tiles: Vec<u8>,
    tiles_w: i32,
    tiles_h: i32,
    tri_group: Vec<u16>,
    groups: Vec<Group>,
    edges: Vec<Edge>,
    open: Vec<(i32, i32, i32)>,
    runs: Vec<(i32, i32)>,
    /// Rectangles à redessiner (disjoints).
    pub rects: Vec<DamageRect>,
    /// Rectangles recopiés par le décalage (dans l'ordre où les recopier), disjoints des
    /// rectangles redessinés.
    pub copy_rects: Vec<DamageRect>,
    /// Rectangles à présenter (redessinés et recopiés).
    pub present: Vec<DamageRect>,
    pub report: DamageReport,
}

/// Résultat de la planification d'une image.
pub(crate) enum Plan {
    Full,
    Partial { scroll: Option<(DamageRect, i32)> },
}

const NO_GROUP: u16 = u16::MAX;

impl Incremental {
    pub fn prims(&self) -> &[PrimInfo] {
        &self.cur.prims
    }

    pub fn mesh(&self, p: &PrimInfo) -> &Mesh {
        &self.cur.meshes[p.mesh as usize]
    }

    /// Oublie l'image précédente : la prochaine sera redessinée entièrement.
    pub fn invalidate(&mut self) {
        self.prev_valid = false;
    }

    /// Prépare les primitives de l'image (maillages en pixels, empreintes).
    pub fn prepare(
        &mut self,
        paint_jobs: &[egui::ClippedPrimitive],
        pixels_per_point: f32,
        width: usize,
        height: usize,
        order: ColorFieldOrder,
    ) {
        let f = &mut self.cur;
        f.prims.clear();
        f.width = width;
        f.height = height;
        f.ppp = pixels_per_point;
        let (w, h) = (width as i64, height as i64);
        let mut used = 0usize;
        for egui::ClippedPrimitive {
            clip_rect,
            primitive,
        } in paint_jobs
        {
            let egui::epaint::Primitive::Mesh(mesh) = primitive else {
                continue;
            };
            if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                continue;
            }
            let clip = Rect {
                min: clip_rect.min * pixels_per_point,
                max: clip_rect.max * pixels_per_point,
            };
            // Mêmes bornes que `draw_egui_mesh`.
            let iclip = DamageRect {
                x0: (clip.min.x as i64).clamp(0, w) as i32,
                y0: (clip.min.y as i64).clamp(0, h) as i32,
                x1: (clip.max.x.ceil() as i64).clamp(0, w) as i32,
                y1: (clip.max.y.ceil() as i64).clamp(0, h) as i32,
            };
            if iclip.is_empty() {
                continue;
            }
            if used == f.meshes.len() {
                f.meshes.push(Mesh::default());
            }
            let px = &mut f.meshes[used];
            let (min, max, uv) = prepare_px_mesh_into(px, mesh, pixels_per_point, order);
            let size = max - min;
            if size.x > 8192.0 || size.y > 8192.0 {
                // Comme `render_direct` : maillage aberrant ignoré.
                continue;
            }
            let bounds = DamageRect::around(min, max);
            let cov = bounds.intersect(&iclip);
            if cov.is_empty() {
                continue;
            }
            let (h_abs, h_rel, qref) = fingerprint(px, &iclip);
            f.prims.push(PrimInfo {
                clip,
                iclip,
                cov,
                unclipped: iclip.contains(&bounds),
                texture: mesh.texture_id,
                h_abs,
                h_rel,
                qref,
                mesh: used as u32,
                low_precision: size.x > 4096.0 || size.y > 4096.0,
                uv,
            });
            used += 1;
        }
    }

    /// Décide comment rendre l'image préparée et calcule les zones endommagées.
    /// `dirty_textures` : zones de textures réécrites depuis l'image précédente (UV
    /// normalisées, marge de filtrage comprise) ; les primitives qui les échantillonnent sont
    /// redessinées.
    pub fn plan(
        &mut self,
        reuse_previous: bool,
        dirty_textures: impl Iterator<Item = (TextureId, Rect)>,
    ) -> Plan {
        let same_target = self.prev.width == self.cur.width
            && self.prev.height == self.cur.height
            && self.prev.ppp == self.cur.ppp;
        if !(reuse_previous && self.prev_valid && same_target) {
            return Plan::Full;
        }

        self.dirty_textures.clear();
        self.dirty_textures.extend(dirty_textures);

        self.tiles_w = (self.cur.width as i32 + TILE_W - 1) / TILE_W;
        self.tiles_h = (self.cur.height as i32 + TILE_H - 1) / TILE_H;
        self.tiles.clear();
        self.tiles.resize((self.tiles_w * self.tiles_h) as usize, 0);

        self.build_indices();
        let mut scroll = self.detect_scroll();
        self.match_prims(scroll);
        if let Some((r, _)) = scroll {
            // Décalage peu crédible (un seul élément a bougé, par exemple le fond de survol
            // passé d'une ligne à une autre) : la plupart du contenu de la région est resté
            // en place. Recopier coûterait plus que redessiner ce qui a changé.
            let (mut moved, mut still) = (0u64, 0u64);
            for (p, m) in self.cur.prims.iter().zip(&self.matches) {
                match m {
                    Match::Moved => moved += p.cov.area(),
                    Match::Same if r.contains(&p.cov) => still += p.cov.area(),
                    _ => {}
                }
            }
            if moved < still {
                scroll = None;
                self.match_prims(None);
            }
        }

        let screen = self.screen();
        if let Some((r, t)) = scroll {
            // Bande découverte par le décalage.
            let v = r.intersect(&r.translate_y(t));
            let exposed = if t < 0 {
                DamageRect { y0: v.y1, ..r }
            } else {
                DamageRect { y1: v.y0, ..r }
            };
            self.mark(exposed);
        }
        self.damage_unmatched(scroll);
        if let Some((r, t)) = scroll {
            for j in 0..self.cur.prims.len() {
                let p = self.cur.prims[j];
                if self.matches[j] == Match::Same && p.cov.intersects(&r) {
                    self.damage_static(j, r, t);
                }
            }
        }

        let dirty = self.tiles.iter().filter(|&&d| d != 0).count();
        if dirty as f32 > FULL_REDRAW_RATIO * self.tiles.len() as f32 {
            return Plan::Full;
        }
        let (tw, th) = (self.tiles_w, self.tiles_h);
        extract_rects(
            &self.tiles,
            tw,
            th,
            screen,
            &mut self.open,
            &mut self.runs,
            &mut self.rects,
        );
        self.copy_rects.clear();
        if let Some((r, t)) = scroll {
            self.plan_copy(r, t);
            extract_rects(
                &self.copy_tiles,
                tw,
                th,
                screen,
                &mut self.open,
                &mut self.runs,
                &mut self.copy_rects,
            );
            let v = r.intersect(&r.translate_y(t));
            for c in &mut self.copy_rects {
                *c = c.intersect(&v);
            }
            self.copy_rects.retain(|c| !c.is_empty());
            // Ordre de recopie : on lit à `y - t` ; ces lignes ne doivent pas avoir été
            // réécrites. Contenu qui monte (t < 0) : de haut en bas, sinon de bas en haut.
            if t < 0 {
                self.copy_rects.sort_unstable_by_key(|c| c.y0);
            } else {
                self.copy_rects.sort_unstable_by_key(|c| -c.y1);
            }
        }
        if self.rects.len() > MAX_RECTS {
            band_rects(&self.tiles, None, tw, th, screen, &mut self.rects);
        }
        let copy = scroll.map(|_| self.copy_tiles.as_slice());
        band_rects(&self.tiles, copy, tw, th, screen, &mut self.present);
        Plan::Partial { scroll }
    }

    /// Pixels à recopier. En dehors des zones endommagées, un pixel `p` de la zone décalée
    /// qui n'est couvert, ni en `p` ni en `p - t` dans l'image précédente, par une primitive
    /// mobile ou disparue, ne voit que des primitives fixes, invariantes par translation hors
    /// des zones endommagées : son ancienne valeur est déjà la bonne, inutile de le recopier
    /// (le fond uni entre les colonnes d'une liste, par exemple).
    fn plan_copy(&mut self, r: DamageRect, t: i32) {
        self.copy_tiles.clear();
        self.copy_tiles.resize(self.tiles.len(), 0);
        let v = r.intersect(&r.translate_y(t));
        let mut copy = TileMarker {
            tiles: &mut self.copy_tiles,
            w: self.tiles_w,
            h: self.tiles_h,
        };
        for (i, o) in self.prev.prims.iter().enumerate() {
            if self.old_matches[i] == Match::Same {
                continue;
            }
            let inside = o.cov.intersect(&r);
            if inside.is_empty() {
                continue;
            }
            copy.mark(inside.intersect(&v));
            copy.mark(inside.translate_y(t).intersect(&v));
        }
        // Les tuiles endommagées seront redessinées : inutile d'y recopier.
        for (c, d) in self.copy_tiles.iter_mut().zip(&self.tiles) {
            *c &= !*d;
        }
    }

    /// Après le rendu : l'image courante devient la référence de la suivante.
    pub fn finish(&mut self, plan: &Plan) {
        let screen = self.screen();
        match plan {
            Plan::Full => {
                self.rects.clear();
                self.rects.push(screen);
                self.copy_rects.clear();
                self.present.clear();
                self.present.push(screen);
                self.report = DamageReport {
                    full: true,
                    scroll: None,
                    redrawn_px: screen.area(),
                    copied_px: 0,
                };
            }
            Plan::Partial { scroll } => {
                self.report = DamageReport {
                    full: false,
                    scroll: *scroll,
                    redrawn_px: self.rects.iter().map(DamageRect::area).sum(),
                    copied_px: self.copy_rects.iter().map(DamageRect::area).sum(),
                };
            }
        }
        mem::swap(&mut self.cur, &mut self.prev);
        self.prev_valid = true;
    }

    fn screen(&self) -> DamageRect {
        DamageRect::new(0, 0, self.cur.width as i32, self.cur.height as i32)
    }

    fn build_indices(&mut self) {
        self.abs_index.clear();
        self.rel_index.clear();
        for (i, p) in self.prev.prims.iter().enumerate() {
            self.abs_index.push((p.h_abs, i as u32));
            self.rel_index.push((p.h_rel, i as u32));
        }
        // Tri par (empreinte, indice) : dans une plage d'empreinte, le premier indice
        // admissible est le plus petit.
        self.abs_index.sort_unstable();
        self.rel_index.sort_unstable();
    }

    /// Translation verticale dominante parmi les primitives dont la découpe est restée en
    /// place (contenu d'une zone de défilement), et cette découpe.
    fn detect_scroll(&mut self) -> Option<(DamageRect, i32)> {
        // Translation entière verticale de `o` vers `n`, découpe fixe ou translatée avec le
        // contenu.
        let moved_by = |o: &PrimInfo, n: &PrimInfo| -> Option<i32> {
            let dy = n.qref[1] - o.qref[1];
            if n.qref[0] != o.qref[0] || dy == 0 || dy % SUBPIX as i32 != 0 {
                return None;
            }
            let t = dy / SUBPIX as i32;
            // Découpe fixe, translatée avec le contenu, ou sans effet (les découpes par
            // cellule, calculées en points, peuvent s'arrondir différemment d'une image à
            // l'autre à l'échelle 150 %).
            let clip_ok = o.iclip == n.iclip
                || o.iclip.translate_y(t) == n.iclip
                || (o.unclipped && n.unclipped);
            clip_ok.then_some(t)
        };

        // 1. Le décalage. Des primitives identiques (icônes d'un même type, d'une ligne à
        //    l'autre) ont plusieurs partenaires possibles : chacune répartit son poids (son
        //    aire) entre eux, les primitives sans ambiguïté (noms) l'emportent. Une primitive
        //    restée en place (même empreinte absolue) ne vote pas : sinon, quand rien ne
        //    défile, ces icônes identiques suggéreraient un décalage d'une ligne.
        self.votes.clear();
        for n in &self.cur.prims {
            let start = self.abs_index.partition_point(|e| e.0 < n.h_abs);
            if self.abs_index.get(start).is_some_and(|e| e.0 == n.h_abs) {
                continue;
            }
            let k = candidates(&self.rel_index, n.h_rel)
                .filter(|&oi| moved_by(&self.prev.prims[oi], n).is_some())
                .count() as u64;
            if k == 0 {
                continue;
            }
            let weight = n.cov.area() * 64 / k;
            for oi in candidates(&self.rel_index, n.h_rel) {
                let Some(t) = moved_by(&self.prev.prims[oi], n) else {
                    continue;
                };
                match self.votes.iter_mut().find(|v| v.0 == t) {
                    Some(v) => v.2 += weight,
                    None => self.votes.push((t, DamageRect::EMPTY, weight)),
                }
            }
        }
        let t = self.votes.iter().max_by_key(|v| v.2)?.0;

        // 2. La région : la découpe restée en place la plus couverte par du contenu décalé
        //    de `t` (la zone visible d'une liste qui défile).
        self.votes.clear();
        for n in &self.cur.prims {
            for oi in candidates(&self.rel_index, n.h_rel) {
                let o = &self.prev.prims[oi];
                if o.iclip == n.iclip && moved_by(o, n) == Some(t) {
                    match self.votes.iter_mut().find(|v| v.1 == n.iclip) {
                        Some(v) => v.2 += n.cov.area(),
                        None => self.votes.push((t, n.iclip, n.cov.area())),
                    }
                    break;
                }
            }
        }
        let r = self.votes.iter().max_by_key(|v| v.2)?.1;
        (t.abs() < r.height()).then_some((r, t))
    }

    /// Appariement croissant (dans l'ordre de dessin) des primitives des deux images.
    fn match_prims(&mut self, scroll: Option<(DamageRect, i32)>) {
        let n_new = self.cur.prims.len();
        self.matches.clear();
        self.matches.resize(n_new, Match::None);
        self.old_matches.clear();
        self.old_matches.resize(self.prev.prims.len(), Match::None);

        let mut last: i64 = -1;
        for j in 0..n_new {
            let n = self.cur.prims[j];
            if samples_dirty(&self.dirty_textures, &n) {
                continue;
            }
            let mut best: Option<(u32, Match)> = None;

            let start = self.abs_index.partition_point(|e| e.0 < n.h_abs);
            for &(h, oi) in &self.abs_index[start..] {
                if h != n.h_abs {
                    break;
                }
                if (oi as i64) > last {
                    best = Some((oi, Match::Same));
                    break;
                }
            }

            if let Some((r, t)) = scroll
                && r.contains(&n.cov)
            {
                let start = self.rel_index.partition_point(|e| e.0 < n.h_rel);
                for &(h, oi) in &self.rel_index[start..] {
                    if h != n.h_rel || best.is_some_and(|b| oi >= b.0) {
                        break;
                    }
                    if (oi as i64) <= last {
                        continue;
                    }
                    let o = &self.prev.prims[oi as usize];
                    let moved =
                        o.qref[0] == n.qref[0] && o.qref[1] + t * SUBPIX as i32 == n.qref[1];
                    // Même découpe (qui contient alors R), découpe translatée avec le contenu,
                    // ou découpes sans effet sur le rendu.
                    let clip_ok = (o.iclip == n.iclip && n.iclip.contains(&r))
                        || o.iclip.translate_y(t) == n.iclip
                        || (o.unclipped && n.unclipped);
                    if moved && clip_ok && r.contains(&o.cov) {
                        best = Some((oi, Match::Moved));
                        break;
                    }
                }
            }

            if let Some((oi, kind)) = best {
                let o = &self.prev.prims[oi as usize];
                if samples_dirty(&self.dirty_textures, o) {
                    continue;
                }
                self.matches[j] = kind;
                self.old_matches[oi as usize] = kind;
                last = oi as i64;
            }
        }
    }

    /// Primitives sans partenaire : leurs triangles, dans l'image précédente (là où ils
    /// apparaissent après décalage) et dans la nouvelle.
    fn damage_unmatched(&mut self, scroll: Option<(DamageRect, i32)>) {
        let mut tiles = TileMarker {
            tiles: &mut self.tiles,
            w: self.tiles_w,
            h: self.tiles_h,
        };
        for (j, p) in self.cur.prims.iter().enumerate() {
            if self.matches[j] != Match::None {
                continue;
            }
            for_each_tri_rect(&self.cur.meshes[p.mesh as usize], |b| {
                tiles.mark(b.intersect(&p.iclip));
            });
        }
        let v = scroll.map(|(r, t)| (r, t, r.intersect(&r.translate_y(t))));
        for (i, p) in self.prev.prims.iter().enumerate() {
            if self.old_matches[i] != Match::None {
                continue;
            }
            for_each_tri_rect(&self.prev.meshes[p.mesh as usize], |b| {
                let b = b.intersect(&p.iclip);
                match v {
                    Some((r, t, v)) => {
                        if !r.contains(&b) {
                            tiles.mark(b);
                        }
                        let inside = b.intersect(&r);
                        if !inside.is_empty() {
                            tiles.mark(inside.translate_y(t).intersect(&v));
                        }
                    }
                    None => tiles.mark(b),
                }
            });
        }
    }

    /// Primitive identique dans les deux images qui recouvre la région décalée : son rendu en
    /// `p` (nouvelle image) doit égaler son rendu en `p - t` (pixels recopiés). C'est le cas à
    /// l'intérieur d'un aplat uni ; on endommage les bords d'aplats et les triangles non unis.
    fn damage_static(&mut self, j: usize, r: DamageRect, t: i32) {
        let p = self.cur.prims[j];
        let mesh = &self.cur.meshes[p.mesh as usize];
        let v = r.intersect(&r.translate_y(t));
        let reach = DamageRect {
            x0: r.x0 - 2,
            y0: r.y0 - 2 - t.abs(),
            x1: r.x1 + 2,
            y1: r.y1 + 2 + t.abs(),
        }
        .to_rect();
        let mut tiles = TileMarker {
            tiles: &mut self.tiles,
            w: self.tiles_w,
            h: self.tiles_h,
        };

        // 1. Triangles proches de R, groupés par couleur unie.
        let n_tris = mesh.indices.len() / 3;
        self.tri_group.clear();
        self.tri_group.resize(n_tris, NO_GROUP);
        self.groups.clear();
        let mut relevant = false;
        for k in 0..n_tris {
            let [a, b, c] = tri(mesh, k);
            let (min, max) = tri_bounds(a.pos, b.pos, c.pos);
            if max.x < reach.min.x
                || min.x > reach.max.x
                || max.y < reach.min.y
                || min.y > reach.max.y
            {
                continue;
            }
            relevant = true;
            if a.color != b.color || a.color != c.color || a.uv != b.uv || a.uv != c.uv {
                // Dégradé, texte, image : non invariant.
                self.tri_group[k] = NO_GROUP - 1;
                continue;
            }
            let uv = [a.uv.x.to_bits(), a.uv.y.to_bits()];
            let g = match self
                .groups
                .iter()
                .position(|g| g.color == a.color && g.uv == uv)
            {
                Some(g) => g,
                None if self.groups.len() < 64 => {
                    self.groups.push(Group {
                        color: a.color,
                        uv,
                        first: k as u32,
                        last: k as u32,
                        valid: true,
                    });
                    self.groups.len() - 1
                }
                None => {
                    self.tri_group[k] = NO_GROUP - 1;
                    continue;
                }
            };
            self.groups[g].last = k as u32;
            self.tri_group[k] = g as u16;
        }
        if !relevant {
            return;
        }
        // Deux couleurs entrelacées dans l'ordre de dessin : l'ordre des mélanges pourrait
        // différer entre `p` et `p - t`. On renonce à l'invariance pour ces groupes.
        for g in 0..self.groups.len() {
            for h in g + 1..self.groups.len() {
                let (a, b) = (self.groups[g], self.groups[h]);
                if a.first <= b.last && b.first <= a.last {
                    self.groups[g].valid = false;
                    self.groups[h].valid = false;
                }
            }
        }

        // 2. Triangles non unis : endommagés là où ils sont, et là où ils apparaissent après
        //    recopie.
        self.edges.clear();
        for k in 0..n_tris {
            let g = self.tri_group[k];
            if g == NO_GROUP {
                continue;
            }
            let [a, b, c] = tri(mesh, k);
            if g == NO_GROUP - 1 || !self.groups[g as usize].valid {
                let (min, max) = tri_bounds(a.pos, b.pos, c.pos);
                let bb = DamageRect::around(min, max).intersect(&p.iclip);
                tiles.mark(bb.intersect(&v));
                tiles.mark(bb.intersect(&r).translate_y(t).intersect(&v));
                continue;
            }
            let ids = tri_indices(mesh, k);
            for e in 0..3 {
                let (i0, i1, i2) = (ids[e], ids[(e + 1) % 3], ids[(e + 2) % 3]);
                self.edges.push(Edge {
                    group: g,
                    a: i0.min(i1),
                    b: i0.max(i1),
                    c: i2,
                });
            }
        }

        // 3. Aplats unis : seuls leurs bords (arêtes non partagées par deux triangles de part
        //    et d'autre) changent sous une translation verticale.
        self.edges.sort_unstable_by_key(|e| (e.group, e.a, e.b));
        let mut i = 0;
        while i < self.edges.len() {
            let e = self.edges[i];
            let mut n = 1;
            while i + n < self.edges.len() {
                let f = self.edges[i + n];
                if (f.group, f.a, f.b) != (e.group, e.a, e.b) {
                    break;
                }
                n += 1;
            }
            let interior = n == 2 && {
                let f = self.edges[i + 1];
                let (pa, pb) = (
                    mesh.vertices[e.a as usize].pos,
                    mesh.vertices[e.b as usize].pos,
                );
                let s1 = egui_orient2df(&pa, &pb, &mesh.vertices[e.c as usize].pos);
                let s2 = egui_orient2df(&pa, &pb, &mesh.vertices[f.c as usize].pos);
                (s1 > 0.0 && s2 < 0.0) || (s1 < 0.0 && s2 > 0.0)
            };
            if !interior {
                for k in i..i + n {
                    let ed = self.edges[k];
                    let pa = mesh.vertices[ed.a as usize].pos;
                    let pb = mesh.vertices[ed.b as usize].pos;
                    edge_sweep(pa, pb, t, |d| tiles.mark(d.intersect(&v)));
                }
            }
            i += n;
        }
    }

    fn mark(&mut self, rect: DamageRect) {
        TileMarker {
            tiles: &mut self.tiles,
            w: self.tiles_w,
            h: self.tiles_h,
        }
        .mark(rect);
    }
}

/// La primitive échantillonne-t-elle une zone de texture réécrite ?
fn samples_dirty(dirty: &[(TextureId, Rect)], p: &PrimInfo) -> bool {
    dirty
        .iter()
        .any(|(id, r)| *id == p.texture && r.intersects(p.uv))
}

/// Anciennes primitives de même empreinte relative que `h` (au plus `MAX_CANDIDATES`).
fn candidates(rel_index: &[(u64, u32)], h: u64) -> impl Iterator<Item = usize> + '_ {
    let start = rel_index.partition_point(|e| e.0 < h);
    rel_index[start..]
        .iter()
        .take_while(move |e| e.0 == h)
        .take(MAX_CANDIDATES)
        .map(|e| e.1 as usize)
}

/// Tuiles marquées → rectangles disjoints (plages par rangée, fusionnées verticalement quand
/// elles sont identiques).
fn extract_rects(
    tiles: &[u8],
    tw: i32,
    th: i32,
    screen: DamageRect,
    open: &mut Vec<(i32, i32, i32)>,
    runs: &mut Vec<(i32, i32)>,
    rects: &mut Vec<DamageRect>,
) {
    rects.clear();
    open.clear();
    let close = |rects: &mut Vec<DamageRect>, (x0, x1, y0): (i32, i32, i32), y1: i32| {
        rects.push(
            DamageRect::new(x0 * TILE_W, y0 * TILE_H, x1 * TILE_W, y1 * TILE_H).intersect(&screen),
        );
    };
    for ty in 0..=th {
        runs.clear();
        if ty < th {
            let row = &tiles[(ty * tw) as usize..((ty + 1) * tw) as usize];
            let mut tx = 0;
            while tx < tw {
                if row[tx as usize] == 0 {
                    tx += 1;
                    continue;
                }
                let start = tx;
                while tx < tw && row[tx as usize] != 0 {
                    tx += 1;
                }
                runs.push((start, tx));
            }
        }
        // Ferme les rectangles qui ne continuent pas, prolonge les autres.
        let mut k = 0;
        while k < open.len() {
            let o = open[k];
            if let Some(pos) = runs.iter().position(|r| *r == (o.0, o.1)) {
                runs.swap_remove(pos);
                k += 1;
            } else {
                close(rects, o, ty);
                open.swap_remove(k);
            }
        }
        for &(x0, x1) in runs.iter() {
            open.push((x0, x1, ty));
        }
    }
}

/// Tuiles marquées (dans `tiles` ou `extra`) → peu de rectangles : par bande de
/// `PRESENT_BAND` rangées, l'enveloppe des tuiles marquées ; bandes voisines de même largeur
/// fusionnées.
fn band_rects(
    tiles: &[u8],
    extra: Option<&[u8]>,
    tw: i32,
    th: i32,
    screen: DamageRect,
    out: &mut Vec<DamageRect>,
) {
    out.clear();
    let mut ty = 0;
    while ty < th {
        let end = (ty + PRESENT_BAND).min(th);
        let mut hull: Option<DamageRect> = None;
        for row in ty..end {
            for tx in 0..tw {
                let k = (row * tw + tx) as usize;
                if tiles[k] != 0 || extra.is_some_and(|e| e[k] != 0) {
                    let t = DamageRect::new(tx, row, tx + 1, row + 1);
                    hull = Some(hull.map_or(t, |h| h.hull(&t)));
                }
            }
        }
        if let Some(h) = hull {
            let r = DamageRect::new(h.x0 * TILE_W, h.y0 * TILE_H, h.x1 * TILE_W, h.y1 * TILE_H)
                .intersect(&screen);
            match out.last_mut() {
                Some(last) if last.x0 == r.x0 && last.x1 == r.x1 && last.y1 == r.y0 => {
                    last.y1 = r.y1;
                }
                _ => out.push(r),
            }
        }
        ty = end;
    }
}

struct TileMarker<'a> {
    tiles: &'a mut [u8],
    w: i32,
    h: i32,
}

impl TileMarker<'_> {
    fn mark(&mut self, r: DamageRect) {
        if r.is_empty() {
            return;
        }
        let tx0 = (r.x0.max(0) / TILE_W).min(self.w);
        let ty0 = (r.y0.max(0) / TILE_H).min(self.h);
        let tx1 = ((r.x1.max(0) + TILE_W - 1) / TILE_W).min(self.w);
        let ty1 = ((r.y1.max(0) + TILE_H - 1) / TILE_H).min(self.h);
        for ty in ty0..ty1 {
            let row = (ty * self.w) as usize;
            self.tiles[row + tx0 as usize..row + tx1 as usize].fill(1);
        }
    }
}

/// Zone où l'appartenance d'un pixel à l'aplat peut différer entre `p` et `p - t` à cause
/// de l'arête `a-b`. Une arête verticale ne compte qu'à ses extrémités.
fn edge_sweep(a: Pos2, b: Pos2, t: i32, mut out: impl FnMut(DamageRect)) {
    let t = t as f32;
    if a.x == b.x {
        for p in [a, b] {
            out(DamageRect::around(
                vec2(p.x, p.y.min(p.y + t)),
                vec2(p.x, p.y.max(p.y + t)),
            ));
        }
    } else {
        let min = vec2(a.x.min(b.x), a.y.min(b.y));
        let max = vec2(a.x.max(b.x), a.y.max(b.y));
        out(DamageRect::around(
            vec2(min.x, min.y.min(min.y + t)),
            vec2(max.x, max.y.max(max.y + t)),
        ));
    }
}

fn tri_indices(mesh: &Mesh, k: usize) -> [u32; 3] {
    [
        mesh.indices[3 * k],
        mesh.indices[3 * k + 1],
        mesh.indices[3 * k + 2],
    ]
}

fn tri(mesh: &Mesh, k: usize) -> [egui::epaint::Vertex; 3] {
    let i = tri_indices(mesh, k);
    [
        mesh.vertices[i[0] as usize],
        mesh.vertices[i[1] as usize],
        mesh.vertices[i[2] as usize],
    ]
}

fn tri_bounds(a: Pos2, b: Pos2, c: Pos2) -> (Vec2, Vec2) {
    (
        vec2(a.x.min(b.x).min(c.x), a.y.min(b.y).min(c.y)),
        vec2(a.x.max(b.x).max(c.x), a.y.max(b.y).max(c.y)),
    )
}

fn for_each_tri_rect(mesh: &Mesh, mut f: impl FnMut(DamageRect)) {
    for k in 0..mesh.indices.len() / 3 {
        let [a, b, c] = tri(mesh, k);
        let (min, max) = tri_bounds(a.pos, b.pos, c.pos);
        f(DamageRect::around(min, max));
    }
}

/// Comme `EguiSoftwareRender::prepare_px_mesh`, dans un maillage réutilisé. Renvoie la boîte
/// englobante en pixels et celle des UV.
fn prepare_px_mesh_into(
    out: &mut Mesh,
    mesh: &Mesh,
    pixels_per_point: f32,
    order: ColorFieldOrder,
) -> (Vec2, Vec2, Rect) {
    out.texture_id = mesh.texture_id;
    out.vertices.clear();
    out.indices.clear();
    out.indices.extend_from_slice(&mesh.indices);
    let mut min = Vec2::splat(f32::MAX);
    let mut max = Vec2::splat(-f32::MAX);
    let mut uv = Rect::NOTHING;
    out.vertices.extend(mesh.vertices.iter().map(|v| {
        let mut v = *v;
        v.pos = (v.pos.to_vec2() * pixels_per_point).to_pos2();
        if let ColorFieldOrder::Bgra = order {
            let d = swizzle_rgba_bgra(v.color.to_array());
            v.color = Color32::from_rgba_premultiplied(d[0], d[1], d[2], d[3]);
        }
        min = min.min(v.pos.to_vec2());
        max = max.max(v.pos.to_vec2());
        uv.extend_with(v.uv);
        v
    }));
    for i in (0..out.indices.len()).step_by(3) {
        let v0 = out.vertices[out.indices[i] as usize].pos;
        let v1 = out.vertices[out.indices[i + 1] as usize].pos;
        let v2 = out.vertices[out.indices[i + 2] as usize].pos;
        if egui_orient2df(&v0, &v1, &v2) < 0.0 {
            out.indices.swap(i + 1, i + 2);
        }
    }
    (min, max, uv)
}

#[inline(always)]
fn quantize(v: f32) -> i32 {
    (v * SUBPIX).round() as i32
}

/// Empreintes (absolue, relative au premier sommet) et premier sommet quantifié.
fn fingerprint(mesh: &Mesh, iclip: &DamageRect) -> (u64, u64, [i32; 2]) {
    let tex = match mesh.texture_id {
        TextureId::Managed(id) => id,
        TextureId::User(id) => id ^ 0x8000_0000_0000_0000,
    };
    let mut abs = Hasher::new(tex);
    let mut rel = Hasher::new(tex);
    abs.add(iclip.x0 as u32, iclip.y0 as u32);
    abs.add(iclip.x1 as u32, iclip.y1 as u32);
    let r = mesh.vertices[0].pos;
    let qref = [quantize(r.x), quantize(r.y)];
    for v in &mesh.vertices {
        let (qx, qy) = (quantize(v.pos.x), quantize(v.pos.y));
        abs.add(qx as u32, qy as u32);
        rel.add((qx - qref[0]) as u32, (qy - qref[1]) as u32);
        let uv = (v.uv.x.to_bits(), v.uv.y.to_bits());
        let color = u32::from_le_bytes(v.color.to_array());
        abs.add(uv.0, uv.1);
        rel.add(uv.0, uv.1);
        abs.add(color, 0);
        rel.add(color, 0);
    }
    let n = mesh.vertices.len() as u32;
    abs.add(n, mesh.indices.len() as u32);
    rel.add(n, mesh.indices.len() as u32);
    for pair in mesh.indices.chunks(2) {
        let (a, b) = (pair[0], pair.get(1).copied().unwrap_or(u32::MAX));
        abs.add(a, b);
        rel.add(a, b);
    }
    (abs.finish(), rel.finish(), qref)
}

/// Hachage 64 bits non cryptographique (multiplication-décalage, finaliseur de MurmurHash3).
struct Hasher(u64);

impl Hasher {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x243F_6A88_85A3_08D3)
    }

    #[inline(always)]
    fn add(&mut self, a: u32, b: u32) {
        let w = (a as u64) | ((b as u64) << 32);
        self.0 = (self.0 ^ w).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 ^= self.0 >> 29;
    }

    fn finish(&self) -> u64 {
        let mut h = self.0;
        h ^= h >> 33;
        h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        h ^= h >> 33;
        h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
        h ^ (h >> 33)
    }
}

/// Recopie de `t` pixels vers le bas (vers le haut si `t < 0`) le contenu des rectangles,
/// dans l'ordre donné, en lisant chaque ligne `y` depuis la ligne `y - t`.
pub(crate) fn shift_rects(data: &mut [[u8; 4]], width: usize, rects: &[DamageRect], t: i32) {
    for r in rects {
        let (x0, x1) = (r.x0 as usize, r.x1 as usize);
        let mut copy = |y: i32| {
            let dst = y as usize * width;
            let src = (y - t) as usize * width;
            data.copy_within(src + x0..src + x1, dst + x0);
        };
        if t < 0 {
            // Le contenu monte : on lit plus bas qu'on n'écrit, de haut en bas.
            for y in r.y0..r.y1 {
                copy(y);
            }
        } else {
            for y in (r.y0..r.y1).rev() {
                copy(y);
            }
        }
    }
}

/// Efface (noir transparent) un rectangle du tampon.
pub(crate) fn clear_rect(data: &mut [[u8; 4]], width: usize, r: DamageRect) {
    for y in r.y0..r.y1 {
        let row = y as usize * width;
        data[row + r.x0 as usize..row + r.x1 as usize].fill([0; 4]);
    }
}

pub(crate) fn clip_to(clip: Rect, r: DamageRect) -> Rect {
    clip.intersect(r.to_rect())
}
