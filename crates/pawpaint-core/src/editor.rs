use std::collections::BTreeMap;
use std::sync::Arc;

use image::RgbaImage;

use crate::model::*;

/// Quantos passos de desfazer ficam guardados.
const HISTORY: usize = 100;
/// Lado maior (em pixels) das imagens importadas: fotos de câmera não precisam de mais.
pub const MAX_IMPORT: u32 = 2560;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveTo {
    Up,
    Down,
    Top,
    Bottom,
}

/// O documento, as imagens dele e o histórico de desfazer.
pub struct Editor {
    doc: Doc,
    assets: BTreeMap<String, Arc<RgbaImage>>,
    undo: Vec<(Doc, &'static str)>,
    redo: Vec<(Doc, &'static str)>,
    /// Chave da edição contínua em andamento (arrastar, digitar): só a primeira vira passo.
    coalesce: Option<String>,
    revision: u64,
    saved_revision: u64,
}

impl Editor {
    pub fn new(width: u32, height: u32) -> Self {
        Self::from_parts(Doc::new(width, height), BTreeMap::new())
    }

    pub fn from_parts(doc: Doc, assets: BTreeMap<String, Arc<RgbaImage>>) -> Self {
        Self {
            doc,
            assets,
            undo: Vec::new(),
            redo: Vec::new(),
            coalesce: None,
            revision: 0,
            saved_revision: 0,
        }
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    pub fn assets(&self) -> &BTreeMap<String, Arc<RgbaImage>> {
        &self.assets
    }

    /// Sobe a cada mudança (para saber quando redesenhar).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
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

    /// Fim de uma edição contínua (soltou o mouse, saiu do campo de texto).
    pub fn end_change(&mut self) {
        self.coalesce = None;
    }

    pub fn undo(&mut self) -> bool {
        let Some((doc, label)) = self.undo.pop() else {
            return false;
        };
        self.redo
            .push((std::mem::replace(&mut self.doc, doc), label));
        self.coalesce = None;
        self.revision += 1;
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
        true
    }

    /// Mexe numa camada. Repetir a mesma `key` em seguida (arrastar um controle) conta como
    /// um passo só de desfazer.
    pub fn edit(
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
        if let Some(l) = self.doc.layers.iter_mut().find(|l| l.id == id) {
            f(l);
        }
    }

    pub fn set_background(&mut self, bg: Background) {
        if self.doc.background == bg {
            return;
        }
        if self.coalesce.as_deref() != Some("background") {
            self.checkpoint("Fundo");
            self.coalesce = Some("background".into());
        } else {
            self.revision += 1;
        }
        self.doc.background = bg;
    }

    /// Muda o tamanho da tela (as camadas ficam onde estão).
    pub fn set_canvas(&mut self, width: u32, height: u32) {
        let (w, h) = (width.clamp(64, 8192), height.clamp(64, 8192));
        if (w, h) != (self.doc.width, self.doc.height) {
            self.checkpoint("Tamanho da tela");
            self.doc.width = w;
            self.doc.height = h;
        }
    }

    fn push(&mut self, layer: Layer) -> LayerId {
        let id = layer.id;
        self.doc.layers.push(layer);
        id
    }

    fn center(&self) -> (f32, f32) {
        (self.doc.width as f32 / 2.0, self.doc.height as f32 / 2.0)
    }

    /// Camada de imagem nova, encaixada em até 60% da tela e centralizada.
    pub fn add_image(&mut self, name: &str, mut img: RgbaImage) -> LayerId {
        self.checkpoint("Nova imagem");
        let longest = img.width().max(img.height());
        if longest > MAX_IMPORT {
            let k = MAX_IMPORT as f32 / longest as f32;
            img = image::imageops::resize(
                &img,
                ((img.width() as f32 * k) as u32).max(1),
                ((img.height() as f32 * k) as u32).max(1),
                image::imageops::FilterType::Lanczos3,
            );
        }
        let fit = (self.doc.width as f32 * 0.6 / img.width() as f32)
            .min(self.doc.height as f32 * 0.6 / img.height() as f32)
            .min(1.0);
        let id = self.doc.fresh();
        let asset = format!("img-{id}.png");
        self.assets.insert(asset.clone(), Arc::new(img));
        let (cx, cy) = self.center();
        let mut layer = Layer::new(id, name, Kind::Image { asset }, cx, cy);
        layer.scale = fit;
        self.push(layer)
    }

    pub fn add_text(&mut self, text: &str) -> LayerId {
        self.checkpoint("Novo texto");
        let id = self.doc.fresh();
        let (cx, cy) = self.center();
        let style = TextStyle {
            text: text.to_string(),
            ..Default::default()
        };
        let mut layer = Layer::new(id, "Texto", Kind::Text(style), cx, cy);
        // Padrão de thumbnail: contorno preto grosso.
        layer.outline = Some(Outline {
            width: 10.0,
            color: Rgba::BLACK,
        });
        layer.shadow = Some(Shadow {
            dx: 6.0,
            dy: 8.0,
            blur: 8.0,
            color: Rgba::new(0, 0, 0, 160),
        });
        self.push(layer)
    }

    pub fn add_shape(&mut self, kind: ShapeKind) -> LayerId {
        self.checkpoint("Nova forma");
        let id = self.doc.fresh();
        let (cx, cy) = self.center();
        let name = match kind {
            ShapeKind::Rect => "Retângulo",
            ShapeKind::Ellipse => "Elipse",
        };
        self.push(Layer::new(
            id,
            name,
            Kind::Shape(ShapeStyle::new(kind)),
            cx,
            cy,
        ))
    }

    pub fn delete(&mut self, id: LayerId) {
        if self.doc.layer(id).is_none() {
            return;
        }
        self.checkpoint("Apagar camada");
        self.doc.layers.retain(|l| l.id != id);
        self.prune();
    }

    pub fn duplicate(&mut self, id: LayerId) -> Option<LayerId> {
        let src = self.doc.layer(id)?.clone();
        self.checkpoint("Duplicar camada");
        let mut copy = src.clone();
        copy.id = self.doc.fresh();
        copy.name = format!("{} cópia", src.name);
        copy.x += 30.0;
        copy.y += 30.0;
        let at = self.doc.layers.iter().position(|l| l.id == id)? + 1;
        let new_id = copy.id;
        self.doc.layers.insert(at, copy);
        Some(new_id)
    }

    pub fn reorder(&mut self, id: LayerId, to: MoveTo) {
        let Some(i) = self.doc.layers.iter().position(|l| l.id == id) else {
            return;
        };
        let last = self.doc.layers.len() - 1;
        let j = match to {
            MoveTo::Up => (i + 1).min(last),
            MoveTo::Down => i.saturating_sub(1),
            MoveTo::Top => last,
            MoveTo::Bottom => 0,
        };
        if i == j {
            return;
        }
        self.checkpoint("Ordem das camadas");
        let l = self.doc.layers.remove(i);
        self.doc.layers.insert(j, l);
    }

    /// Esquece imagens que nenhuma camada (nem o histórico) usa mais.
    fn prune(&mut self) {
        let used: std::collections::HashSet<&str> = std::iter::once(&self.doc)
            .chain(self.undo.iter().map(|(d, _)| d))
            .chain(self.redo.iter().map(|(d, _)| d))
            .flat_map(|d| d.layers.iter())
            .filter_map(|l| match &l.kind {
                Kind::Image { asset } => Some(asset.as_str()),
                _ => None,
            })
            .collect();
        let keep: Vec<String> = self
            .assets
            .keys()
            .filter(|k| used.contains(k.as_str()))
            .cloned()
            .collect();
        self.assets.retain(|k, _| keep.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed() -> Editor {
        Editor::new(THUMB_W, THUMB_H)
    }

    #[test]
    fn layers_are_added_on_top_and_reordered() {
        let mut e = ed();
        let t = e.add_text("OI");
        let s = e.add_shape(ShapeKind::Rect);
        let order = |e: &Editor| e.doc().layers.iter().map(|l| l.id).collect::<Vec<_>>();
        assert_eq!(order(&e), vec![t, s]);
        e.reorder(s, MoveTo::Down);
        assert_eq!(order(&e), vec![s, t]);
        e.reorder(s, MoveTo::Top);
        assert_eq!(order(&e), vec![t, s]);
        e.undo();
        assert_eq!(order(&e), vec![s, t]);
        e.redo();
        assert_eq!(order(&e), vec![t, s]);
    }

    #[test]
    fn dragging_is_one_undo_step() {
        let mut e = ed();
        let t = e.add_text("OI");
        e.end_change();
        for x in [10.0, 20.0, 30.0] {
            e.edit(t, "move", "Mover", |l| l.x = x);
        }
        assert_eq!(e.doc().layer(t).unwrap().x, 30.0);
        e.undo();
        assert_eq!(e.doc().layer(t).unwrap().x, THUMB_W as f32 / 2.0);
        // outro gesto depois de soltar o mouse é outro passo
        e.redo();
        e.end_change();
        e.edit(t, "move", "Mover", |l| l.x = 99.0);
        e.undo();
        assert_eq!(e.doc().layer(t).unwrap().x, 30.0);
    }

    #[test]
    fn images_fit_the_canvas_and_unused_assets_are_dropped() {
        let mut e = ed();
        let id = e.add_image("foto", RgbaImage::new(4000, 3000));
        let l = e.doc().layer(id).unwrap().clone();
        let Kind::Image { asset } = &l.kind else {
            panic!()
        };
        assert!(e.assets()[asset].width() <= MAX_IMPORT);
        let drawn_h = e.assets()[asset].height() as f32 * l.scale;
        assert!(drawn_h <= THUMB_H as f32 * 0.6 + 1.0);
        e.delete(id);
        // ainda dá para desfazer, então a imagem fica
        assert_eq!(e.assets().len(), 1);
        e.undo();
        assert!(e.doc().layer(id).is_some());
    }

    #[test]
    fn duplicate_and_dirty_tracking() {
        let mut e = ed();
        assert!(!e.is_dirty());
        let t = e.add_text("A");
        assert!(e.is_dirty());
        e.mark_saved();
        assert!(!e.is_dirty());
        let c = e.duplicate(t).unwrap();
        assert_eq!(e.doc().layers.len(), 2);
        assert_ne!(c, t);
        assert!(e.is_dirty());
        e.set_canvas(1080, 1920);
        assert_eq!((e.doc().width, e.doc().height), (1080, 1920));
    }
}
