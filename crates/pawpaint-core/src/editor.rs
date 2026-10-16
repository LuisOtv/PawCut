//! O documento com histórico de desfazer e todas as operações de edição.

use std::collections::HashSet;

use image::RgbaImage;

use crate::blend::BlendMode;
use crate::brush::{Brush, Smudger, Stroker, apply_mask_stroke, commit_stroke};
use crate::composite::{self, Live};
use crate::doc::{Doc, Layer, LayerId};
use crate::fill::{self, GradientKind};
use crate::filters::{self, Filter};
use crate::selection::{SelOp, Selection};
use crate::tile::{TILE, TiledImage, premultiply, unpremultiply};
use crate::transform::{self, Affine};

const HISTORY: usize = 60;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EditError {
    #[error("a camada não existe")]
    NoLayer,
    #[error("a camada está travada")]
    Locked,
    #[error("isto é uma pasta: escolha uma camada de dentro")]
    IsGroup,
    #[error("não há nada selecionado")]
    NoSelection,
    #[error("não há camada para unir")]
    NothingBelow,
    #[error("não há nada para mover")]
    NothingToMove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveTo {
    Up,
    Down,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    White,
    Transparent,
    Color([u8; 3]),
}

/// O que mudou desde a última leitura (para redesenhar só o necessário).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Dirty {
    #[default]
    Nothing,
    Tiles(HashSet<(usize, usize)>),
    All,
}

/// Pedaço copiado: pixels (alfa direto) e onde estavam.
#[derive(Debug, Clone)]
pub struct Clip {
    pub image: RgbaImage,
    pub x: i64,
    pub y: i64,
}

/// Parte da camada que está sendo movida ou transformada (a camada fica só com o resto até
/// `finish_float`, e a interface desenha o pedaço solto por cima).
pub struct Floating {
    pub layer: LayerId,
    pub pixels: TiledImage,
    rest: TiledImage,
    original: TiledImage,
    pub bounds: (usize, usize, usize, usize),
    selection: Option<Selection>,
}

struct ActiveSmudge {
    layer: LayerId,
    smudger: Smudger,
}

struct ActiveStroke {
    on_mask: bool,
    layer: LayerId,
    stroker: Stroker,
    opacity: f32,
    mode: BlendMode,
}

pub struct Editor {
    doc: Doc,
    undo: Vec<(Doc, &'static str)>,
    redo: Vec<(Doc, &'static str)>,
    coalesce: Option<String>,
    revision: u64,
    saved_revision: u64,
    dirty: Dirty,
    stroke: Option<ActiveStroke>,
    smudge: Option<ActiveSmudge>,
    mirror: (bool, bool),
    paint_mask: bool,
    pixel_rev: u64,
}

impl Editor {
    pub fn new(width: usize, height: usize, bg: Background) -> Self {
        let mut doc = Doc::new(width, height);
        let id = doc.fresh();
        let mut layer = Layer::new(id, "Fundo", width, height);
        let color = match bg {
            Background::White => Some([255, 255, 255]),
            Background::Color(c) => Some(c),
            Background::Transparent => None,
        };
        if let Some(c) = color {
            layer.pixels.fill_solid([c[0], c[1], c[2], 255]);
        } else {
            layer.name = "Camada 1".into();
        }
        doc.layers.push(layer);
        Self::from_doc(doc)
    }

    pub fn from_doc(doc: Doc) -> Self {
        Self {
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
            coalesce: None,
            revision: 0,
            saved_revision: 0,
            dirty: Dirty::All,
            stroke: None,
            smudge: None,
            mirror: (false, false),
            paint_mask: false,
            pixel_rev: 0,
        }
    }

    /// Documento novo com uma imagem como única camada.
    pub fn from_image(img: &RgbaImage, name: &str) -> Self {
        let mut doc = Doc::new(img.width() as usize, img.height() as usize);
        let id = doc.fresh();
        let mut l = Layer::new(id, name, doc.width, doc.height);
        l.pixels = TiledImage::from_rgba(img);
        doc.layers.push(l);
        Self::from_doc(doc)
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Sobe quando os pixels mudam (inclusive ao fim de um traço), para miniaturas.
    pub fn pixel_rev(&self) -> u64 {
        self.revision + self.pixel_rev
    }

    pub fn set_mirror(&mut self, horizontal_axis: bool, vertical_axis: bool) {
        self.mirror = (horizontal_axis, vertical_axis);
    }

    /// O próximo traço pinta na máscara da camada (se ela tiver uma).
    pub fn set_paint_mask(&mut self, on: bool) {
        self.paint_mask = on;
    }

    pub fn mirror(&self) -> (bool, bool) {
        self.mirror
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    pub fn take_dirty(&mut self) -> Dirty {
        std::mem::take(&mut self.dirty)
    }

    fn dirty_all(&mut self) {
        self.dirty = Dirty::All;
    }

    fn dirty_tiles(&mut self, tiles: impl IntoIterator<Item = (usize, usize)>) {
        match &mut self.dirty {
            Dirty::All => {}
            Dirty::Tiles(set) => set.extend(tiles),
            Dirty::Nothing => self.dirty = Dirty::Tiles(tiles.into_iter().collect()),
        }
    }

    // ------------------------------------------------------------ histórico

    pub fn history(&self) -> (Vec<&'static str>, Vec<&'static str>) {
        (
            self.undo.iter().map(|(_, l)| *l).collect(),
            self.redo.iter().rev().map(|(_, l)| *l).collect(),
        )
    }

    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(|(_, l)| *l)
    }

    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|(_, l)| *l)
    }

    fn checkpoint(&mut self, label: &'static str) {
        self.undo.push((self.doc.clone(), label));
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.coalesce = None;
        self.revision += 1;
    }

    pub fn end_change(&mut self) {
        self.coalesce = None;
    }

    pub fn undo(&mut self) -> bool {
        self.stroke = None;
        let Some((doc, label)) = self.undo.pop() else {
            return false;
        };
        self.redo
            .push((std::mem::replace(&mut self.doc, doc), label));
        self.coalesce = None;
        self.revision += 1;
        self.dirty_all();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some((doc, label)) = self.redo.pop() else {
            return false;
        };
        self.undo
            .push((std::mem::replace(&mut self.doc, doc), label));
        self.coalesce = None;
        self.revision += 1;
        self.dirty_all();
        true
    }

    /// Volta (ou avança) até ficar com `steps` passos de desfazer.
    pub fn jump_to(&mut self, steps: usize) {
        while self.undo.len() > steps && self.undo() {}
        while self.undo.len() < steps && self.redo() {}
    }

    fn layer_checked(&self, id: LayerId) -> Result<&Layer, EditError> {
        let l = self.doc.layer(id).ok_or(EditError::NoLayer)?;
        if l.is_group {
            return Err(EditError::IsGroup);
        }
        if l.locked {
            return Err(EditError::Locked);
        }
        Ok(l)
    }

    // ------------------------------------------------------------ camadas

    /// Camada vazia nova logo acima de `above` (ou no topo). Devolve o id.
    pub fn add_layer(&mut self, name: Option<String>, above: Option<LayerId>) -> LayerId {
        self.checkpoint("Nova camada");
        let id = self.doc.fresh();
        let name = name.unwrap_or_else(|| format!("Camada {}", self.doc.layers.len() + 1));
        let mut layer = Layer::new(id, name, self.doc.width, self.doc.height);
        let at = above
            .and_then(|a| self.doc.index_of(a))
            .map_or(self.doc.layers.len(), |i| i + 1);
        // Fica na mesma pasta da camada de referência (ou dentro dela, se for uma pasta).
        if let Some(r) = above.and_then(|a| self.doc.layer(a)) {
            layer.parent = if r.is_group { Some(r.id) } else { r.parent };
        }
        self.doc.layers.insert(at, layer);
        self.dirty_all();
        id
    }

    pub fn duplicate_layer(&mut self, id: LayerId) -> Result<LayerId, EditError> {
        let src = self.doc.layer(id).ok_or(EditError::NoLayer)?.clone();
        self.checkpoint("Duplicar camada");
        let mut copy = src;
        copy.id = self.doc.fresh();
        copy.name = format!("{} cópia", copy.name);
        let at = self
            .doc
            .index_of(id)
            .map_or(self.doc.layers.len(), |i| i + 1);
        let new_id = copy.id;
        let is_group = copy.is_group;
        self.doc.layers.insert(at, copy);
        // Uma pasta leva as camadas de dentro (e as pastas dentro dela) junto.
        if is_group {
            self.copy_children(id, new_id, at + 1);
        }
        self.dirty_all();
        Ok(new_id)
    }

    fn copy_children(&mut self, from: LayerId, to: LayerId, mut at: usize) -> usize {
        let kids: Vec<Layer> = self
            .doc
            .layers
            .iter()
            .filter(|l| l.parent == Some(from))
            .cloned()
            .collect();
        for mut k in kids {
            let old = k.id;
            k.id = self.doc.fresh();
            k.parent = Some(to);
            let (new, grp) = (k.id, k.is_group);
            self.doc.layers.insert(at, k);
            at += 1;
            if grp {
                at = self.copy_children(old, new, at);
            }
        }
        at
    }

    pub fn delete_layer(&mut self, id: LayerId) -> Result<(), EditError> {
        let parent = self.doc.layer(id).ok_or(EditError::NoLayer)?.parent;
        self.checkpoint("Apagar camada");
        // O que estava dentro de uma pasta apagada sobe para onde a pasta estava.
        for l in self.doc.layers.iter_mut().filter(|l| l.parent == Some(id)) {
            l.parent = parent;
        }
        self.doc.layers.retain(|l| l.id != id);
        self.dirty_all();
        Ok(())
    }

    pub fn move_layer(&mut self, id: LayerId, to: MoveTo) {
        let Some(i) = self.doc.index_of(id) else {
            return;
        };
        // Só conta quem está na mesma pasta: subir/descer troca com a vizinha de verdade.
        let parent = self.doc.layers[i].parent;
        let siblings: Vec<usize> = (0..self.doc.layers.len())
            .filter(|k| self.doc.layers[*k].parent == parent)
            .collect();
        let pos = siblings.iter().position(|k| *k == i).unwrap_or(0);
        let target_pos = match to {
            MoveTo::Up => (pos + 1).min(siblings.len() - 1),
            MoveTo::Down => pos.saturating_sub(1),
            MoveTo::Top => siblings.len() - 1,
            MoveTo::Bottom => 0,
        };
        if target_pos == pos {
            return;
        }
        let j = siblings[target_pos];
        self.checkpoint("Mover camada");
        let l = self.doc.layers.remove(i);
        self.doc.layers.insert(j, l);
        self.dirty_all();
    }

    /// Mexe nas propriedades da camada (nome, opacidade, modo…). Repetir a mesma `key` em seguida
    /// (arrastar um controle) vira um passo só.
    pub fn edit_layer(
        &mut self,
        id: LayerId,
        key: &str,
        label: &'static str,
        f: impl FnOnce(&mut Layer),
    ) {
        if self.doc.layer(id).is_none() {
            return;
        }
        let key = format!("{id}:{key}");
        if self.coalesce.as_deref() != Some(&key) {
            self.checkpoint(label);
            self.coalesce = Some(key);
        } else {
            self.revision += 1;
        }
        if let Some(l) = self.doc.layer_mut(id) {
            f(l);
        }
        self.dirty_all();
    }

    /// Une a camada com a de baixo (na opacidade e no modo dela).
    pub fn merge_down(&mut self, id: LayerId) -> Result<LayerId, EditError> {
        let i = self.doc.index_of(id).ok_or(EditError::NoLayer)?;
        if i == 0 {
            return Err(EditError::NothingBelow);
        }
        let below = &self.doc.layers[i - 1];
        let this = &self.doc.layers[i];
        if this.is_group || below.is_group || this.parent != below.parent {
            return Err(EditError::NothingBelow);
        }
        self.checkpoint("Unir com a de baixo");
        let top = self.doc.layers.remove(i);
        let below_id = self.doc.layers[i - 1].id;
        let (w, h) = (self.doc.width, self.doc.height);
        let below = &mut self.doc.layers[i - 1];
        for ty in 0..h.div_ceil(TILE) {
            for tx in 0..w.div_ceil(TILE) {
                let Some(src) = top.pixels.tile(tx, ty) else {
                    continue;
                };
                // A máscara da camada de cima vale na união.
                let masked: Vec<u8>;
                let src: &[u8] = match top.mask.as_ref().and_then(|m| m.tile(tx, ty)) {
                    Some(m) => {
                        masked = (0..TILE * TILE * 4)
                            .map(|i| {
                                ((src[i] as u32 * (255 - m[i / 4 * 4 + 3] as u32) + 127) / 255)
                                    as u8
                            })
                            .collect();
                        &masked
                    }
                    None => src,
                };
                let dst = below.pixels.tile_mut(tx, ty);
                for p in 0..TILE * TILE {
                    let k = p * 4;
                    if src[k + 3] != 0 {
                        crate::blend::blend_over(
                            &mut dst[k..k + 4],
                            &src[k..k + 4],
                            top.opacity,
                            top.mode,
                        );
                    }
                }
            }
        }
        self.dirty_all();
        Ok(below_id)
    }

    /// Junta tudo numa camada só.
    pub fn flatten(&mut self) {
        if self.doc.layers.len() <= 1 {
            return;
        }
        self.checkpoint("Achatar imagem");
        let img = composite::flatten(&self.doc);
        let id = self.doc.fresh();
        let mut l = Layer::new(id, "Imagem achatada", self.doc.width, self.doc.height);
        l.pixels = TiledImage::from_rgba(&img);
        self.doc.layers = vec![l];
        self.dirty_all();
    }

    /// Pasta nova (acima de `above`), vazia.
    pub fn add_group(&mut self, above: Option<LayerId>) -> LayerId {
        self.checkpoint("Nova pasta");
        let id = self.doc.fresh();
        let n = self.doc.layers.iter().filter(|l| l.is_group).count() + 1;
        let mut g = Layer::new(id, format!("Pasta {n}"), self.doc.width, self.doc.height);
        g.is_group = true;
        let (parent, at) = match above.and_then(|a| self.doc.index_of(a)) {
            Some(i) => (self.doc.layers[i].parent, i + 1),
            None => (None, self.doc.layers.len()),
        };
        g.parent = parent;
        self.doc.layers.insert(at, g);
        self.dirty_all();
        id
    }

    /// Põe a camada dentro da pasta `group` (ou fora de qualquer pasta com `None`). Pasta dentro
    /// dela mesma não vale.
    pub fn set_parent(&mut self, id: LayerId, group: Option<LayerId>) -> Result<(), EditError> {
        self.doc.layer(id).ok_or(EditError::NoLayer)?;
        if let Some(g) = group {
            let mut cur = Some(g);
            while let Some(c) = cur {
                if c == id {
                    return Err(EditError::NoLayer);
                }
                cur = self.doc.layer(c).and_then(|l| l.parent);
            }
            if !self.doc.layer(g).is_some_and(|l| l.is_group) {
                return Err(EditError::NoLayer);
            }
        }
        if self.doc.layer(id).is_some_and(|l| l.parent == group) {
            return Ok(());
        }
        self.checkpoint("Mover para pasta");
        self.doc.layer_mut(id).expect("existe").parent = group;
        self.dirty_all();
        Ok(())
    }

    /// Põe uma máscara (vazia: tudo visível) na camada.
    pub fn add_mask(&mut self, id: LayerId) -> Result<(), EditError> {
        let l = self.doc.layer(id).ok_or(EditError::NoLayer)?;
        if l.mask.is_some() {
            return Ok(());
        }
        self.checkpoint("Nova máscara");
        let (w, h) = (self.doc.width, self.doc.height);
        self.doc.layer_mut(id).expect("existe").mask = Some(TiledImage::new(w, h));
        self.dirty_all();
        Ok(())
    }

    /// Tira a máscara; com `apply`, o que estava escondido some de vez dos pixels.
    pub fn remove_mask(&mut self, id: LayerId, apply: bool) -> Result<(), EditError> {
        self.doc
            .layer(id)
            .and_then(|l| l.mask.as_ref())
            .ok_or(EditError::NoLayer)?;
        self.checkpoint(if apply {
            "Aplicar máscara"
        } else {
            "Apagar máscara"
        });
        let l = self.doc.layer_mut(id).expect("existe");
        if let Some(mask) = l.mask.take()
            && apply
        {
            for ty in 0..l.pixels.tile_dims().1 {
                for tx in 0..l.pixels.tile_dims().0 {
                    let Some(m) = mask.tile(tx, ty) else { continue };
                    let dst = l.pixels.tile_mut(tx, ty);
                    for p in 0..TILE * TILE {
                        let k = 255 - m[p * 4 + 3] as u32;
                        for c in 0..4 {
                            dst[p * 4 + c] = ((dst[p * 4 + c] as u32 * k + 127) / 255) as u8;
                        }
                    }
                }
            }
        }
        self.dirty_all();
        Ok(())
    }

    pub fn clear_layer(&mut self, id: LayerId) -> Result<(), EditError> {
        let l = self.layer_checked(id)?;
        if let Some(sel) = self.doc.selection.clone() {
            let mut px = l.pixels.clone();
            erase_with_mask(&mut px, &sel);
            self.checkpoint("Limpar seleção");
            self.doc.layer_mut(id).expect("existe").pixels = px;
        } else {
            self.checkpoint("Limpar camada");
            self.doc.layer_mut(id).expect("existe").pixels.clear();
        }
        self.dirty_all();
        Ok(())
    }

    // ------------------------------------------------------------ pincel

    /// Começa um traço na camada. Um passo de desfazer cobre o traço inteiro.
    pub fn begin_stroke(
        &mut self,
        layer: LayerId,
        brush: Brush,
        color: [u8; 3],
        erase: bool,
        mode: BlendMode,
    ) -> Result<(), EditError> {
        self.layer_checked(layer)?;
        self.checkpoint(if erase { "Borracha" } else { "Pincel" });
        let mut stroker = Stroker::new(self.doc.width, self.doc.height);
        let opacity = brush.opacity;
        stroker.begin(brush, color, erase);
        stroker.mirror = self.mirror;
        let on_mask = self.paint_mask && self.doc.layer(layer).is_some_and(|l| l.mask.is_some());
        self.stroke = Some(ActiveStroke {
            on_mask,
            layer,
            stroker,
            opacity,
            mode,
        });
        Ok(())
    }

    pub fn stroke_active(&self) -> bool {
        self.stroke.is_some() || self.smudge.is_some()
    }

    /// Começa um borrão (arrastar cor) na camada.
    pub fn begin_smudge(
        &mut self,
        layer: LayerId,
        brush: Brush,
        x: f32,
        y: f32,
    ) -> Result<(), EditError> {
        let l = self.layer_checked(layer)?;
        let mut smudger = Smudger::new(brush);
        smudger.begin(&l.pixels, x, y);
        self.checkpoint("Borrão");
        self.smudge = Some(ActiveSmudge { layer, smudger });
        Ok(())
    }

    pub fn smudge_point(&mut self, x: f32, y: f32) {
        let Some(s) = &mut self.smudge else { return };
        let sel = self.doc.selection.clone();
        let Some(l) = self.doc.layer_mut(s.layer) else {
            return;
        };
        let tiles = s.smudger.add_point(&mut l.pixels, sel.as_ref(), x, y);
        self.pixel_rev += 1;
        self.dirty_tiles(tiles);
    }

    pub fn end_smudge(&mut self) {
        self.smudge = None;
    }

    pub fn stroke_point(&mut self, x: f32, y: f32) {
        if let Some(s) = &mut self.stroke {
            s.stroker.add_point(x, y);
            let tiles = s.stroker.buf.take_dirty();
            self.dirty_tiles(tiles);
        }
    }

    /// Traço reto (linhas e contornos de formas) dentro de um traço já começado.
    pub fn stroke_line(&mut self, a: (f32, f32), b: (f32, f32)) {
        if let Some(s) = &mut self.stroke {
            s.stroker.line(a, b);
            let tiles = s.stroker.buf.take_dirty();
            self.dirty_tiles(tiles);
        }
    }

    pub fn end_stroke(&mut self, x: f32, y: f32) {
        let Some(mut s) = self.stroke.take() else {
            return;
        };
        s.stroker.finish(x, y);
        let (selection, alpha_lock) = (
            self.doc.selection.clone(),
            self.doc.layer(s.layer).is_some_and(|l| l.alpha_lock),
        );
        if let Some(layer) = self.doc.layer_mut(s.layer) {
            if s.on_mask
                && let Some(mask) = layer.mask.as_mut()
            {
                for (tx, ty) in s.stroker.buf.tile_keys().collect::<Vec<_>>() {
                    let Some(scratch) = s.stroker.buf.tile(tx, ty) else {
                        continue;
                    };
                    let sel = selection.as_ref().map(|m| m.tile_mask(tx, ty));
                    apply_mask_stroke(
                        mask.tile_mut(tx, ty),
                        scratch,
                        s.opacity,
                        s.stroker.erase,
                        sel.as_deref(),
                    );
                }
            } else {
                commit_stroke(
                    &mut layer.pixels,
                    &s.stroker.buf,
                    s.opacity,
                    s.stroker.erase,
                    s.mode,
                    selection.as_ref(),
                    alpha_lock,
                );
            }
        }
        let tiles: Vec<_> = s.stroker.buf.tile_keys().collect();
        self.dirty_tiles(tiles);
        self.pixel_rev += 1;
    }

    /// O traço em andamento, para mostrar na tela antes de aplicar.
    pub fn live(&self) -> Option<Live<'_>> {
        self.stroke.as_ref().map(|s| Live {
            layer: s.layer,
            buf: &s.stroker.buf,
            opacity: s.opacity,
            erase: s.stroker.erase,
            mode: s.mode,
            on_mask: s.on_mask,
        })
    }

    // ------------------------------------------------------------ preenchimento

    /// Cor (alfa direto) no ponto, da camada ou da imagem toda.
    pub fn pick_color(&self, x: usize, y: usize, merged: bool, layer: Option<LayerId>) -> [u8; 4] {
        if x >= self.doc.width || y >= self.doc.height {
            return [0; 4];
        }
        if merged || layer.is_none() {
            let t = composite::composite_tile(&self.doc, x / TILE, y / TILE, None);
            let i = ((y % TILE) * TILE + x % TILE) * 4;
            unpremultiply([t[i], t[i + 1], t[i + 2], t[i + 3]])
        } else {
            layer
                .and_then(|id| self.doc.layer(id))
                .map_or([0; 4], |l| unpremultiply(l.pixels.pixel(x, y)))
        }
    }

    /// Balde de tinta: pinta a região parecida com a do ponto.
    #[allow(clippy::too_many_arguments)]
    pub fn bucket_fill(
        &mut self,
        id: LayerId,
        seed: (usize, usize),
        color: [u8; 3],
        tolerance: f32,
        contiguous: bool,
        opacity: f32,
        sample_merged: bool,
    ) -> Result<(), EditError> {
        let layer = self.layer_checked(id)?;
        let (w, h) = (self.doc.width, self.doc.height);
        let mask = if sample_merged {
            let flat = composite::flatten(&self.doc);
            fill::flood_mask(
                &|x, y| flat.get_pixel(x as u32, y as u32).0,
                w,
                h,
                seed,
                tolerance,
                contiguous,
            )
        } else {
            let px = &layer.pixels;
            fill::flood_mask(
                &|x, y| unpremultiply(px.pixel(x, y)),
                w,
                h,
                seed,
                tolerance,
                contiguous,
            )
        };
        let (sel, lock) = (self.doc.selection.clone(), layer.alpha_lock);
        self.checkpoint("Balde de tinta");
        let l = self.doc.layer_mut(id).expect("existe");
        fill::fill_mask(&mut l.pixels, &mask, color, opacity, sel.as_ref(), lock);
        self.dirty_all();
        Ok(())
    }

    /// Pinta a seleção (ou a camada toda) com a cor.
    pub fn fill_selection(&mut self, id: LayerId, color: [u8; 3]) -> Result<(), EditError> {
        let layer = self.layer_checked(id)?;
        let (w, h) = (self.doc.width, self.doc.height);
        let lock = layer.alpha_lock;
        let mask = self
            .doc
            .selection
            .as_ref()
            .map_or_else(|| vec![255u8; w * h], |s| s.mask().to_vec());
        self.checkpoint("Preencher");
        let l = self.doc.layer_mut(id).expect("existe");
        fill::fill_mask(&mut l.pixels, &mask, color, 1.0, None, lock);
        self.dirty_all();
        Ok(())
    }

    /// Pinta o que `shape` cobre (formas preenchidas), respeitando a seleção.
    pub fn fill_shape(
        &mut self,
        id: LayerId,
        shape: &Selection,
        color: [u8; 3],
        label: &'static str,
    ) -> Result<(), EditError> {
        let layer = self.layer_checked(id)?;
        let (sel, lock) = (self.doc.selection.clone(), layer.alpha_lock);
        self.checkpoint(label);
        let l = self.doc.layer_mut(id).expect("existe");
        fill::fill_mask(&mut l.pixels, shape.mask(), color, 1.0, sel.as_ref(), lock);
        self.dirty_all();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn gradient(
        &mut self,
        id: LayerId,
        p0: (f32, f32),
        p1: (f32, f32),
        kind: GradientKind,
        a: [u8; 4],
        b: [u8; 4],
        opacity: f32,
    ) -> Result<(), EditError> {
        self.layer_checked(id)?;
        let sel = self.doc.selection.clone();
        self.checkpoint("Degradê");
        let l = self.doc.layer_mut(id).expect("existe");
        fill::gradient(&mut l.pixels, p0, p1, kind, a, b, opacity, sel.as_ref());
        self.dirty_all();
        Ok(())
    }

    pub fn apply_filter(
        &mut self,
        id: LayerId,
        filter: &Filter,
        label: &'static str,
    ) -> Result<(), EditError> {
        let layer = self.layer_checked(id)?;
        let (sel, lock) = (self.doc.selection.clone(), layer.alpha_lock);
        self.checkpoint(label);
        let l = self.doc.layer_mut(id).expect("existe");
        filters::apply(&mut l.pixels, filter, sel.as_ref(), lock);
        self.dirty_all();
        Ok(())
    }

    // ------------------------------------------------------------ seleção

    pub fn set_selection(&mut self, sel: Option<Selection>, label: &'static str) {
        let sel = sel.filter(|s| !s.is_empty());
        if self.doc.selection == sel {
            return;
        }
        self.checkpoint(label);
        self.doc.selection = sel;
        self.dirty_all();
    }

    /// Junta `new` à seleção atual conforme `op`.
    pub fn combine_selection(&mut self, new: Selection, op: SelOp, label: &'static str) {
        let merged = match (&self.doc.selection, op) {
            (Some(cur), op) if op != SelOp::Replace => cur.combine(&new, op),
            (None, SelOp::Subtract) => {
                Selection::all(self.doc.width, self.doc.height).combine(&new, op)
            }
            (None, SelOp::Intersect) => Selection::empty(self.doc.width, self.doc.height),
            _ => new,
        };
        self.set_selection(Some(merged), label);
    }

    pub fn select_all(&mut self) {
        self.set_selection(
            Some(Selection::all(self.doc.width, self.doc.height)),
            "Selecionar tudo",
        );
    }

    pub fn deselect(&mut self) {
        self.set_selection(None, "Desfazer seleção");
    }

    pub fn invert_selection(&mut self) {
        let inv = match &self.doc.selection {
            Some(s) => s.invert(),
            None => Selection::all(self.doc.width, self.doc.height),
        };
        self.set_selection(Some(inv), "Inverter seleção");
    }

    pub fn feather_selection(&mut self, radius: usize) {
        if let Some(s) = self.doc.selection.clone() {
            self.set_selection(Some(s.feather(radius)), "Suavizar seleção");
        }
    }

    pub fn grow_selection(&mut self, px: i32) {
        if let Some(s) = self.doc.selection.clone() {
            self.set_selection(
                Some(s.grow(px)),
                if px >= 0 {
                    "Expandir seleção"
                } else {
                    "Contrair seleção"
                },
            );
        }
    }

    /// Varinha mágica na camada (ou na imagem toda).
    pub fn magic_wand(
        &mut self,
        id: LayerId,
        seed: (usize, usize),
        tolerance: f32,
        contiguous: bool,
        op: SelOp,
        merged: bool,
    ) {
        let (w, h) = (self.doc.width, self.doc.height);
        let sel = if merged {
            let flat = composite::flatten(&self.doc);
            fill::magic_wand(
                &|x, y| flat.get_pixel(x as u32, y as u32).0,
                w,
                h,
                seed,
                tolerance,
                contiguous,
            )
        } else if let Some(l) = self.doc.layer(id) {
            let px = &l.pixels;
            fill::magic_wand(
                &|x, y| unpremultiply(px.pixel(x, y)),
                w,
                h,
                seed,
                tolerance,
                contiguous,
            )
        } else {
            return;
        };
        self.combine_selection(sel, op, "Varinha mágica");
    }

    // ------------------------------------------------------------ copiar e colar

    /// Copia a seleção (ou a camada toda) como imagem.
    pub fn copy(&self, id: LayerId, merged: bool) -> Option<Clip> {
        let (x0, y0, x1, y1) = match &self.doc.selection {
            Some(s) => s.bounds()?,
            None => (0, 0, self.doc.width, self.doc.height),
        };
        let mut img = if merged {
            let flat = composite::flatten(&self.doc);
            image::imageops::crop_imm(
                &flat,
                x0 as u32,
                y0 as u32,
                (x1 - x0) as u32,
                (y1 - y0) as u32,
            )
            .to_image()
        } else {
            self.doc.layer(id)?.pixels.region_to_rgba(x0, y0, x1, y1)
        };
        if let Some(s) = &self.doc.selection {
            for (x, y, p) in img.enumerate_pixels_mut() {
                let m = s.value(x0 + x as usize, y0 + y as usize) as u32;
                p.0[3] = (p.0[3] as u32 * m / 255) as u8;
            }
        }
        Some(Clip {
            image: img,
            x: x0 as i64,
            y: y0 as i64,
        })
    }

    pub fn cut(&mut self, id: LayerId) -> Result<Option<Clip>, EditError> {
        self.layer_checked(id)?;
        let clip = self.copy(id, false);
        if clip.is_some() {
            self.clear_layer(id)?;
        }
        Ok(clip)
    }

    /// Cola como camada nova. `at` = canto de cima à esquerda (o lugar original se vazio).
    pub fn paste(&mut self, clip: &Clip, at: Option<(i64, i64)>) -> LayerId {
        self.checkpoint("Colar");
        let id = self.doc.fresh();
        let mut l = Layer::new(id, "Colado", self.doc.width, self.doc.height);
        let (x, y) = at.unwrap_or((clip.x, clip.y));
        l.pixels.write_rgba(&clip.image, x, y);
        self.doc.layers.push(l);
        self.dirty_all();
        id
    }

    /// Importa uma imagem como camada nova (centralizada se couber, senão no canto).
    pub fn import_layer(&mut self, img: &RgbaImage, name: &str) -> LayerId {
        let x = (self.doc.width as i64 - img.width() as i64) / 2;
        let y = (self.doc.height as i64 - img.height() as i64) / 2;
        let id = self.paste(
            &Clip {
                image: img.clone(),
                x,
                y,
            },
            Some((x.max(0), y.max(0))),
        );
        if let Some(l) = self.doc.layer_mut(id) {
            l.name = name.to_string();
        }
        id
    }

    // ------------------------------------------------------------ transformar

    /// Transforma a seleção (ou a camada toda) com `m`. Com seleção, só o que está dentro dela
    /// se move (e a própria seleção acompanha).
    pub fn transform(&mut self, id: LayerId, m: &Affine) -> Result<(), EditError> {
        let layer = self.layer_checked(id)?;
        let (w, h) = (self.doc.width, self.doc.height);
        let new_pixels;
        let mut new_sel = None;
        if let Some(sel) = self.doc.selection.clone() {
            let mut floating = layer.pixels.clone();
            keep_with_mask(&mut floating, &sel);
            let mut rest = layer.pixels.clone();
            erase_with_mask(&mut rest, &sel);
            let moved = transform::transform(&floating, m, w, h);
            let mut out = rest;
            for ty in 0..h.div_ceil(TILE) {
                for tx in 0..w.div_ceil(TILE) {
                    let Some(src) = moved.tile(tx, ty) else {
                        continue;
                    };
                    let dst = out.tile_mut(tx, ty);
                    for p in 0..TILE * TILE {
                        let k = p * 4;
                        if src[k + 3] != 0 {
                            crate::blend::blend_over(
                                &mut dst[k..k + 4],
                                &src[k..k + 4],
                                1.0,
                                BlendMode::Normal,
                            );
                        }
                    }
                }
            }
            new_pixels = out;
            let mut mask_img = TiledImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let v = sel.value(x, y);
                    if v != 0 {
                        mask_img.set_pixel(x, y, [v, v, v, v]);
                    }
                }
            }
            let tm = transform::transform(&mask_img, m, w, h);
            let mut mm = vec![0u8; w * h];
            for y in 0..h {
                for x in 0..w {
                    mm[y * w + x] = tm.pixel(x, y)[3];
                }
            }
            new_sel = Some(Selection::from_mask(w, h, mm));
        } else {
            new_pixels = transform::transform(&layer.pixels, m, w, h);
        }
        self.checkpoint("Transformar");
        self.doc.layer_mut(id).expect("existe").pixels = new_pixels;
        if let Some(s) = new_sel {
            self.doc.selection = Some(s).filter(|s| !s.is_empty());
        }
        self.dirty_all();
        Ok(())
    }

    /// Solta a seleção (ou a camada toda) para mover/transformar. Um passo de desfazer cobre
    /// tudo até `finish_float` (ou `abort_float`).
    pub fn begin_float(&mut self, id: LayerId) -> Result<Floating, EditError> {
        let layer = self.layer_checked(id)?;
        let original = layer.pixels.clone();
        let sel = self.doc.selection.clone();
        let (pixels, rest) = match &sel {
            Some(s) => {
                let mut f = original.clone();
                keep_with_mask(&mut f, s);
                let mut r = original.clone();
                erase_with_mask(&mut r, s);
                (f, r)
            }
            None => (
                original.clone(),
                TiledImage::new(self.doc.width, self.doc.height),
            ),
        };
        let bounds = pixels.content_bounds().ok_or(EditError::NothingToMove)?;
        self.checkpoint("Transformar");
        self.doc.layer_mut(id).expect("existe").pixels = rest.clone();
        self.dirty_all();
        Ok(Floating {
            layer: id,
            pixels,
            rest,
            original,
            bounds,
            selection: sel,
        })
    }

    /// Põe o pedaço solto de volta, transformado por `m` (e leva a seleção junto).
    pub fn finish_float(&mut self, f: &Floating, m: &Affine) {
        let (w, h) = (self.doc.width, self.doc.height);
        let moved = transform::transform(&f.pixels, m, w, h);
        let mut out = f.rest.clone();
        for ty in 0..h.div_ceil(TILE) {
            for tx in 0..w.div_ceil(TILE) {
                let Some(src) = moved.tile(tx, ty) else {
                    continue;
                };
                let dst = out.tile_mut(tx, ty);
                for p in 0..TILE * TILE {
                    let k = p * 4;
                    if src[k + 3] != 0 {
                        crate::blend::blend_over(
                            &mut dst[k..k + 4],
                            &src[k..k + 4],
                            1.0,
                            BlendMode::Normal,
                        );
                    }
                }
            }
        }
        if let Some(l) = self.doc.layer_mut(f.layer) {
            l.pixels = out;
        }
        if let Some(sel) = &f.selection {
            let mut mask_img = TiledImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let v = sel.value(x, y);
                    if v != 0 {
                        mask_img.set_pixel(x, y, [v, v, v, v]);
                    }
                }
            }
            let tm = transform::transform(&mask_img, m, w, h);
            let mut mm = vec![0u8; w * h];
            for y in 0..h {
                for x in 0..w {
                    mm[y * w + x] = tm.pixel(x, y)[3];
                }
            }
            self.doc.selection = Some(Selection::from_mask(w, h, mm)).filter(|s| !s.is_empty());
        }
        self.dirty_all();
    }

    /// Desiste: a camada volta a ser como era e o passo de desfazer some.
    pub fn abort_float(&mut self, f: &Floating) {
        if let Some(l) = self.doc.layer_mut(f.layer) {
            l.pixels = f.original.clone();
        }
        self.undo.pop();
        self.dirty_all();
    }

    /// Recomeça o traço atual (para pré-visualizar formas que mudam enquanto se arrasta).
    pub fn stroke_restart(&mut self) {
        if let Some(s) = &mut self.stroke {
            let tiles: Vec<_> = s.stroker.buf.tile_keys().collect();
            let (brush, color, erase) = (s.stroker.brush.clone(), s.stroker.color, s.stroker.erase);
            s.stroker.begin(brush, color, erase);
            self.dirty_tiles(tiles);
        }
    }

    // ------------------------------------------------------------ imagem toda

    fn map_layers(
        &mut self,
        label: &'static str,
        w: usize,
        h: usize,
        f: impl Fn(&TiledImage) -> TiledImage,
    ) {
        self.checkpoint(label);
        for l in &mut self.doc.layers {
            l.pixels = f(&l.pixels);
        }
        self.doc.width = w;
        self.doc.height = h;
        self.doc.selection = None;
        self.dirty_all();
    }

    pub fn crop(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) {
        let (x1, y1) = (x1.min(self.doc.width), y1.min(self.doc.height));
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        self.map_layers("Cortar tela", w, h, |p| {
            p.resized(w, h, -(x0 as i64), -(y0 as i64))
        });
    }

    /// Corta a tela para caber só no que tem tinta (em qualquer camada visível).
    pub fn crop_to_content(&mut self) -> bool {
        let mut b: Option<(usize, usize, usize, usize)> = None;
        for l in self.doc.layers.iter().filter(|l| l.visible) {
            if let Some((a, bb, c, d)) = l.pixels.content_bounds() {
                b = Some(match b {
                    None => (a, bb, c, d),
                    Some((x0, y0, x1, y1)) => (x0.min(a), y0.min(bb), x1.max(c), y1.max(d)),
                });
            }
        }
        match b {
            Some((x0, y0, x1, y1)) if (x1 - x0, y1 - y0) != (self.doc.width, self.doc.height) => {
                self.crop(x0, y0, x1, y1);
                true
            }
            _ => false,
        }
    }

    /// Muda o tamanho da tela sem esticar a imagem; (`dx`, `dy`) é onde a imagem antiga fica.
    pub fn resize_canvas(&mut self, w: usize, h: usize, dx: i64, dy: i64) {
        let (w, h) = (w.clamp(1, 16384), h.clamp(1, 16384));
        self.map_layers("Tamanho da tela", w, h, |p| p.resized(w, h, dx, dy));
    }

    pub fn scale_image(&mut self, w: usize, h: usize) {
        let (w, h) = (w.clamp(1, 16384), h.clamp(1, 16384));
        self.map_layers("Escalar imagem", w, h, |p| transform::scale_image(p, w, h));
    }

    pub fn rotate_image(&mut self, clockwise: bool) {
        let (w, h) = (self.doc.height, self.doc.width);
        self.map_layers("Girar imagem", w, h, |p| transform::rotate90(p, clockwise));
    }

    pub fn flip_image(&mut self, horizontal: bool) {
        let (w, h) = (self.doc.width, self.doc.height);
        self.map_layers("Espelhar imagem", w, h, |p| {
            if horizontal {
                transform::flip_horizontal(p)
            } else {
                transform::flip_vertical(p)
            }
        });
    }

    pub fn flip_layer(&mut self, id: LayerId, horizontal: bool) -> Result<(), EditError> {
        let l = self.layer_checked(id)?;
        let px = if horizontal {
            transform::flip_horizontal(&l.pixels)
        } else {
            transform::flip_vertical(&l.pixels)
        };
        self.checkpoint("Espelhar camada");
        self.doc.layer_mut(id).expect("existe").pixels = px;
        self.dirty_all();
        Ok(())
    }

    /// Põe uma imagem (texto, por exemplo) na camada, misturando por cima.
    pub fn stamp(
        &mut self,
        id: LayerId,
        img: &RgbaImage,
        x: i64,
        y: i64,
        label: &'static str,
    ) -> Result<(), EditError> {
        self.layer_checked(id)?;
        let sel = self.doc.selection.clone();
        self.checkpoint(label);
        let l = self.doc.layer_mut(id).expect("existe");
        for (ix, iy, p) in img.enumerate_pixels() {
            let (px, py) = (x + ix as i64, y + iy as i64);
            if px < 0 || py < 0 || p.0[3] == 0 {
                continue;
            }
            let (px, py) = (px as usize, py as usize);
            let m = sel.as_ref().map_or(1.0, |s| s.value(px, py) as f32 / 255.0);
            if m <= 0.0 {
                continue;
            }
            let mut d = l.pixels.pixel(px, py);
            crate::blend::blend_over(&mut d, &premultiply(p.0), m, BlendMode::Normal);
            l.pixels.set_pixel(px, py, d);
        }
        self.dirty_all();
        Ok(())
    }
}

/// Apaga da camada o que a seleção cobre (nas bordas suaves, só em parte).
fn erase_with_mask(px: &mut TiledImage, sel: &Selection) {
    if let Some((x0, y0, x1, y1)) = sel.bounds() {
        for y in y0..y1 {
            for x in x0..x1 {
                let m = sel.value(x, y) as u32;
                if m == 0 {
                    continue;
                }
                let p = px.pixel(x, y);
                if p[3] == 0 {
                    continue;
                }
                let k = 255 - m;
                px.set_pixel(
                    x,
                    y,
                    [0, 1, 2, 3].map(|i| ((p[i] as u32 * k + 127) / 255) as u8),
                );
            }
        }
    }
}

/// Deixa na imagem só o que a seleção cobre.
fn keep_with_mask(px: &mut TiledImage, sel: &Selection) {
    let (w, h) = (px.width(), px.height());
    for y in 0..h {
        for x in 0..w {
            let p = px.pixel(x, y);
            if p[3] == 0 {
                continue;
            }
            let m = sel.value(x, y) as u32;
            px.set_pixel(
                x,
                y,
                [0, 1, 2, 3].map(|i| ((p[i] as u32 * m + 127) / 255) as u8),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed() -> Editor {
        Editor::new(100, 80, Background::White)
    }

    fn top(e: &Editor) -> LayerId {
        e.doc().layers.last().unwrap().id
    }

    #[test]
    fn new_documents_start_with_a_white_or_transparent_layer() {
        let e = ed();
        assert_eq!(e.doc().layers.len(), 1);
        assert_eq!(e.doc().layers[0].pixels.pixel(5, 5), [255, 255, 255, 255]);
        let t = Editor::new(10, 10, Background::Transparent);
        assert!(t.doc().layers[0].pixels.is_empty());
        let c = Editor::new(10, 10, Background::Color([1, 2, 3]));
        assert_eq!(c.doc().layers[0].pixels.pixel(0, 0), [1, 2, 3, 255]);
    }

    #[test]
    fn a_stroke_is_one_undo_step_and_shows_live() {
        let mut e = ed();
        let id = e.add_layer(None, None);
        let b = Brush {
            size: 10.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        e.begin_stroke(id, b, [255, 0, 0], false, BlendMode::Normal)
            .unwrap();
        e.stroke_point(20.0, 20.0);
        e.stroke_point(60.0, 20.0);
        assert!(e.live().is_some());
        let t = composite::composite_tile(e.doc(), 0, 0, e.live().as_ref());
        assert_eq!(&t[(20 * TILE + 40) * 4..][..4], &[255, 0, 0, 255]);
        e.end_stroke(60.0, 20.0);
        assert!(e.live().is_none());
        assert_eq!(
            e.doc().layer(id).unwrap().pixels.pixel(40, 20),
            [255, 0, 0, 255]
        );
        assert_eq!(e.undo_label(), Some("Pincel"));
        e.undo();
        assert_eq!(e.doc().layer(id).unwrap().pixels.pixel(40, 20), [0; 4]);
        e.redo();
        assert_eq!(
            e.doc().layer(id).unwrap().pixels.pixel(40, 20),
            [255, 0, 0, 255]
        );
        assert!(matches!(e.take_dirty(), Dirty::All));
    }

    #[test]
    fn locked_layers_refuse_edits() {
        let mut e = ed();
        let id = e.add_layer(None, None);
        e.edit_layer(id, "lock", "Travar", |l| l.locked = true);
        assert_eq!(e.fill_selection(id, [0, 0, 0]), Err(EditError::Locked));
        assert_eq!(
            e.apply_filter(id, &Filter::Invert, "x"),
            Err(EditError::Locked)
        );
        assert_eq!(
            e.begin_stroke(id, Brush::basic(), [0; 3], false, BlendMode::Normal),
            Err(EditError::Locked)
        );
    }

    #[test]
    fn selection_limits_fill_clear_and_copy_paste() {
        let mut e = ed();
        let id = top(&e);
        e.combine_selection(
            Selection::rect(100, 80, 10, 10, 30, 30),
            SelOp::Replace,
            "Seleção",
        );
        e.fill_selection(id, [0, 0, 255]).unwrap();
        assert_eq!(e.doc().layers[0].pixels.pixel(15, 15), [0, 0, 255, 255]);
        assert_eq!(e.doc().layers[0].pixels.pixel(50, 15), [255, 255, 255, 255]);
        let clip = e.copy(id, false).unwrap();
        assert_eq!(
            (clip.image.width(), clip.image.height(), clip.x, clip.y),
            (20, 20, 10, 10)
        );
        let pasted = e.paste(&clip, Some((60, 40)));
        assert_eq!(
            e.doc().layer(pasted).unwrap().pixels.pixel(65, 45),
            [0, 0, 255, 255]
        );
        e.clear_layer(id).unwrap();
        assert_eq!(e.doc().layer(id).unwrap().pixels.pixel(15, 15), [0; 4]);
        assert_eq!(
            e.doc().layer(id).unwrap().pixels.pixel(50, 15),
            [255, 255, 255, 255]
        );
        e.deselect();
        assert!(e.doc().selection.is_none());
        e.invert_selection();
        assert_eq!(e.doc().selection.as_ref().unwrap().value(5, 5), 255);
    }

    #[test]
    fn bucket_wand_and_merge_down() {
        let mut e = ed();
        let bg = top(&e);
        let line = e.add_layer(None, None);
        for y in 0..80 {
            e.doc
                .layer_mut(line)
                .unwrap()
                .pixels
                .set_pixel(50, y, [0, 0, 0, 255]);
        }
        e.bucket_fill(line, (10, 10), [255, 0, 0], 0.1, true, 1.0, true)
            .unwrap();
        // com "amostrar tudo", o traço preto fecha a região esquerda
        let px = &e.doc().layer(line).unwrap().pixels;
        assert_eq!(px.pixel(10, 10), [255, 0, 0, 255]);
        assert_eq!(px.pixel(70, 10), [0; 4]);
        e.magic_wand(bg, (5, 5), 0.0, true, SelOp::Replace, false);
        assert_eq!(
            e.doc().selection.as_ref().unwrap().bounds(),
            Some((0, 0, 100, 80))
        );
        let merged = e.merge_down(line).unwrap();
        assert_eq!(merged, bg);
        assert_eq!(e.doc().layers.len(), 1);
        assert_eq!(e.doc().layers[0].pixels.pixel(10, 10), [255, 0, 0, 255]);
        assert_eq!(e.merge_down(bg), Err(EditError::NothingBelow));
    }

    #[test]
    fn transform_moves_only_the_selection() {
        let mut e = Editor::new(100, 80, Background::Transparent);
        let id = top(&e);
        e.doc
            .layer_mut(id)
            .unwrap()
            .pixels
            .set_pixel(10, 10, [255, 0, 0, 255]);
        e.doc
            .layer_mut(id)
            .unwrap()
            .pixels
            .set_pixel(70, 60, [0, 255, 0, 255]);
        e.combine_selection(
            Selection::rect(100, 80, 0, 0, 30, 30),
            SelOp::Replace,
            "Seleção",
        );
        e.transform(id, &Affine::around((0.0, 0.0), 1.0, 1.0, 0.0, 20.0, 5.0))
            .unwrap();
        let px = &e.doc().layer(id).unwrap().pixels;
        assert_eq!(px.pixel(10, 10), [0; 4]);
        assert!(px.pixel(30, 15)[0] > 200);
        assert_eq!(px.pixel(70, 60), [0, 255, 0, 255]);
        assert!(e.doc().selection.as_ref().unwrap().value(35, 20) > 200);
    }

    #[test]
    fn groups_composite_together_and_move_with_their_children() {
        let mut e = Editor::new(100, 80, Background::Color([255, 255, 255]));
        let red = e.add_layer(None, None);
        e.fill_selection(red, [255, 0, 0]).unwrap();
        let g = e.add_group(Some(red));
        e.set_parent(red, Some(g)).unwrap();
        // a pasta inteira some/aparece e muda de opacidade junto
        assert_eq!(
            composite::flatten(e.doc()).get_pixel(5, 5).0,
            [255, 0, 0, 255]
        );
        e.edit_layer(g, "o", "o", |l| l.opacity = 0.5);
        assert_eq!(
            composite::flatten(e.doc()).get_pixel(5, 5).0,
            [255, 128, 128, 255]
        );
        e.edit_layer(g, "v", "v", |l| l.visible = false);
        assert_eq!(
            composite::flatten(e.doc()).get_pixel(5, 5).0,
            [255, 255, 255, 255]
        );
        e.edit_layer(g, "v2", "v", |l| l.visible = true);
        // pasta dentro dela mesma não vale; unir pasta não vale
        assert!(e.set_parent(g, Some(g)).is_err());
        assert!(e.set_parent(red, Some(red)).is_err());
        assert!(e.merge_down(g).is_err());
        // duplicar leva os filhos
        let g2 = e.duplicate_layer(g).unwrap();
        assert_eq!(
            e.doc()
                .layers
                .iter()
                .filter(|l| l.parent == Some(g2))
                .count(),
            1
        );
        assert_eq!(e.doc().layers.len(), 5);
        // apagar a pasta solta os filhos
        e.delete_layer(g).unwrap();
        assert_eq!(e.doc().layer(red).unwrap().parent, None);
        e.undo();
        assert_eq!(e.doc().layer(red).unwrap().parent, Some(g));
    }

    #[test]
    fn merge_down_respects_the_mask_of_the_top_layer() {
        let mut e = Editor::new(100, 80, Background::Color([0, 0, 255]));
        let top_id = e.add_layer(None, None);
        e.fill_selection(top_id, [255, 0, 0]).unwrap();
        e.add_mask(top_id).unwrap();
        e.set_paint_mask(true);
        e.begin_stroke(
            top_id,
            Brush {
                size: 30.0,
                hardness: 1.0,
                ..Brush::basic()
            },
            [0; 3],
            false,
            BlendMode::Normal,
        )
        .unwrap();
        e.stroke_point(50.0, 40.0);
        e.end_stroke(50.0, 40.0);
        let merged = e.merge_down(top_id).unwrap();
        let px = &e.doc().layer(merged).unwrap().pixels;
        assert_eq!(px.pixel(50, 40), [0, 0, 255, 255]);
        assert_eq!(px.pixel(5, 5), [255, 0, 0, 255]);
    }

    #[test]
    fn masks_hide_reveal_and_apply() {
        let mut e = ed();
        let id = top(&e);
        e.add_mask(id).unwrap();
        e.set_paint_mask(true);
        let b = Brush {
            size: 30.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        e.begin_stroke(id, b.clone(), [0; 3], false, BlendMode::Normal)
            .unwrap();
        e.stroke_point(50.0, 40.0);
        // ao vivo, antes de soltar, já aparece escondido
        let t = composite::composite_tile(e.doc(), 0, 0, e.live().as_ref());
        assert_eq!(t[(40 * TILE + 50) * 4 + 3], 0);
        e.end_stroke(50.0, 40.0);
        assert_eq!(composite::flatten(e.doc()).get_pixel(50, 40).0[3], 0);
        assert_eq!(
            composite::flatten(e.doc()).get_pixel(5, 5).0,
            [255, 255, 255, 255]
        );
        // os pixels continuam lá embaixo
        assert_eq!(
            e.doc().layer(id).unwrap().pixels.pixel(50, 40),
            [255, 255, 255, 255]
        );
        // revelar de novo
        e.begin_stroke(id, b, [0; 3], true, BlendMode::Normal)
            .unwrap();
        e.stroke_point(50.0, 40.0);
        e.end_stroke(50.0, 40.0);
        assert_eq!(composite::flatten(e.doc()).get_pixel(50, 40).0[3], 255);
        // aplicar: esconde, aplica e some a máscara
        e.begin_stroke(
            id,
            Brush {
                size: 30.0,
                hardness: 1.0,
                ..Brush::basic()
            },
            [0; 3],
            false,
            BlendMode::Normal,
        )
        .unwrap();
        e.stroke_point(50.0, 40.0);
        e.end_stroke(50.0, 40.0);
        e.remove_mask(id, true).unwrap();
        assert!(e.doc().layer(id).unwrap().mask.is_none());
        assert_eq!(e.doc().layer(id).unwrap().pixels.pixel(50, 40)[3], 0);
        assert_eq!(e.doc().layer(id).unwrap().pixels.pixel(5, 5)[3], 255);
        e.undo();
        assert!(e.doc().layer(id).unwrap().mask.is_some());
    }

    #[test]
    fn mirrored_strokes_and_crop_to_content() {
        let mut e = Editor::new(100, 80, Background::Transparent);
        let id = e.doc().layers[0].id;
        e.set_mirror(true, false);
        let rev = e.pixel_rev();
        e.begin_stroke(
            id,
            Brush {
                size: 6.0,
                hardness: 1.0,
                ..Brush::basic()
            },
            [0, 0, 0],
            false,
            BlendMode::Normal,
        )
        .unwrap();
        e.stroke_point(20.0, 40.0);
        e.end_stroke(20.0, 40.0);
        assert!(e.pixel_rev() > rev);
        let px = &e.doc().layer(id).unwrap().pixels;
        assert_eq!(px.pixel(80, 40)[3], 255);
        assert!(e.crop_to_content());
        assert_eq!((e.doc().width, e.doc().height), (66, 6));
        assert!(!e.crop_to_content());
    }

    #[test]
    fn floating_move_commit_and_abort() {
        let mut e = Editor::new(100, 80, Background::Transparent);
        let id = top(&e);
        e.doc
            .layer_mut(id)
            .unwrap()
            .pixels
            .set_pixel(10, 10, [255, 0, 0, 255]);
        let f = e.begin_float(id).unwrap();
        assert_eq!(f.bounds, (10, 10, 11, 11));
        assert_eq!(e.doc().layer(id).unwrap().pixels.pixel(10, 10), [0; 4]);
        e.finish_float(&f, &Affine::around((0.0, 0.0), 1.0, 1.0, 0.0, 30.0, 20.0));
        assert_eq!(
            e.doc().layer(id).unwrap().pixels.pixel(40, 30),
            [255, 0, 0, 255]
        );
        e.undo();
        assert_eq!(
            e.doc().layer(id).unwrap().pixels.pixel(10, 10),
            [255, 0, 0, 255]
        );
        let f = e.begin_float(id).unwrap();
        let steps = e.history().0.len();
        e.abort_float(&f);
        assert_eq!(e.history().0.len(), steps - 1);
        assert_eq!(
            e.doc().layer(id).unwrap().pixels.pixel(10, 10),
            [255, 0, 0, 255]
        );
        let empty = e.add_layer(None, None);
        assert!(matches!(
            e.begin_float(empty),
            Err(EditError::NothingToMove)
        ));
    }

    #[test]
    fn canvas_operations_apply_to_every_layer() {
        let mut e = ed();
        e.add_layer(None, None);
        e.crop(10, 10, 60, 50);
        assert_eq!((e.doc().width, e.doc().height), (50, 40));
        assert!(e.doc().layers.iter().all(|l| l.pixels.width() == 50));
        e.rotate_image(true);
        assert_eq!((e.doc().width, e.doc().height), (40, 50));
        e.resize_canvas(80, 80, 20, 10);
        assert_eq!(e.doc().layers[0].pixels.pixel(25, 15), [255, 255, 255, 255]);
        e.scale_image(160, 160);
        assert_eq!(e.doc().layers[0].pixels.width(), 160);
        e.flatten();
        assert_eq!(e.doc().layers.len(), 1);
        e.undo();
        assert_eq!(e.doc().layers.len(), 2);
        let (undo, _) = e.history();
        assert!(!undo.is_empty());
    }

    #[test]
    fn layer_properties_coalesce_and_reorder() {
        let mut e = ed();
        let id = e.add_layer(None, None);
        e.end_change();
        for o in [0.9, 0.8, 0.7] {
            e.edit_layer(id, "opacity", "Opacidade", |l| l.opacity = o);
        }
        assert_eq!(e.doc().layer(id).unwrap().opacity, 0.7);
        e.undo();
        assert_eq!(e.doc().layer(id).unwrap().opacity, 1.0);
        e.move_layer(id, MoveTo::Bottom);
        assert_eq!(e.doc().layers[0].id, id);
        let dup = e.duplicate_layer(id).unwrap();
        assert_eq!(e.doc().layers[1].id, dup);
        e.jump_to(0);
        assert_eq!(e.doc().layers.len(), 1);
    }
}

#[cfg(test)]
mod perf_probe {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore]
    fn big_canvas_timings() {
        let t = Instant::now();
        let mut e = Editor::new(4096, 4096, Background::White);
        println!("new 4096²: {:?}", t.elapsed());
        let id = e.add_layer(None, None);
        let t = Instant::now();
        let b = Brush {
            size: 60.0,
            hardness: 0.5,
            ..Brush::basic()
        };
        e.begin_stroke(id, b, [10, 20, 30], false, BlendMode::Normal)
            .unwrap();
        for i in 0..200 {
            e.stroke_point(
                100.0 + i as f32 * 10.0,
                2000.0 + (i as f32 * 0.2).sin() * 300.0,
            );
        }
        e.end_stroke(2100.0, 2000.0);
        println!("stroke of 200 points: {:?}", t.elapsed());
        let t = Instant::now();
        let mut n = 0;
        for ty in 0..64 {
            for tx in 0..64 {
                n += composite::composite_tile(e.doc(), tx, ty, None).len();
            }
        }
        println!("full composite 4096² ({n} bytes): {:?}", t.elapsed());
        let t = Instant::now();
        e.checkpoint("x");
        println!("checkpoint: {:?}", t.elapsed());
        let t = Instant::now();
        e.apply_filter(id, &Filter::GaussianBlur(8.0), "b").unwrap();
        println!("blur: {:?}", t.elapsed());
    }
}
