//! Junta as camadas (modos de mistura, opacidade, traço em andamento) num bloco ou na imagem.

use image::RgbaImage;

use crate::blend::{BlendMode, blend_over};
use crate::brush::{StrokeBuffer, apply_mask_stroke, apply_stroke_tile};
use crate::doc::{Doc, LayerId};
use crate::tile::{TILE, TILE_BYTES, unpremultiply};

/// Traço que ainda não foi aplicado, mostrado por cima da camada onde está sendo pintado.
pub struct Live<'a> {
    pub layer: LayerId,
    pub buf: &'a StrokeBuffer,
    pub opacity: f32,
    pub erase: bool,
    pub mode: BlendMode,
    /// O traço mexe na máscara da camada (`erase` quer dizer revelar).
    pub on_mask: bool,
}

/// O bloco (`tx`, `ty`) com todas as camadas visíveis, sobre transparente (pré-multiplicado).
pub fn composite_tile(doc: &Doc, tx: usize, ty: usize, live: Option<&Live>) -> Vec<u8> {
    composite_level(doc, None, tx, ty, live)
}

/// As camadas (e pastas, recursivamente) que estão dentro de `parent`.
fn composite_level(
    doc: &Doc,
    parent: Option<LayerId>,
    tx: usize,
    ty: usize,
    live: Option<&Live>,
) -> Vec<u8> {
    let mut out = vec![0u8; TILE_BYTES];
    for layer in doc
        .layers
        .iter()
        .filter(|l| l.visible && l.parent == parent)
    {
        if layer.is_group {
            let inner = composite_level(doc, Some(layer.id), tx, ty, live);
            for p in 0..TILE * TILE {
                let i = p * 4;
                if inner[i + 3] != 0 {
                    blend_over(
                        &mut out[i..i + 4],
                        &inner[i..i + 4],
                        layer.opacity,
                        layer.mode,
                    );
                }
            }
            continue;
        }
        let base = layer.pixels.tile(tx, ty);
        let stroke = live
            .filter(|l| l.layer == layer.id)
            .and_then(|l| l.buf.tile(tx, ty).map(|s| (l, s)));
        let (px_stroke, mask_stroke) = match stroke {
            Some((l, s)) if l.on_mask => (None, Some((l, s))),
            other => (other, None),
        };
        let merged: Vec<u8>;
        let src: &[u8] = match (base, px_stroke) {
            (None, None) => continue,
            (Some(b), None) => b,
            (b, Some((l, s))) => {
                let mut t = b.map_or_else(|| vec![0u8; TILE_BYTES], <[u8]>::to_vec);
                let mask = doc.selection.as_ref().map(|s| s.tile_mask(tx, ty));
                apply_stroke_tile(
                    &mut t,
                    s,
                    l.opacity,
                    l.erase,
                    l.mode,
                    mask.as_deref(),
                    layer.alpha_lock,
                );
                merged = t;
                &merged
            }
        };
        // Máscara da camada (com o traço de máscara em andamento, se houver).
        let mask_tile: Option<Vec<u8>> = match (
            layer.mask.as_ref().and_then(|m| m.tile(tx, ty)),
            mask_stroke,
        ) {
            (m, Some((l, s))) => {
                let mut t = m.map_or_else(|| vec![0u8; TILE_BYTES], <[u8]>::to_vec);
                let sel = doc.selection.as_ref().map(|s| s.tile_mask(tx, ty));
                apply_mask_stroke(&mut t, s, l.opacity, l.erase, sel.as_deref());
                Some(t)
            }
            (Some(m), None) => Some(m.to_vec()),
            (None, None) => None,
        };
        for p in 0..TILE * TILE {
            let i = p * 4;
            if src[i + 3] == 0 {
                continue;
            }
            if let Some(m) = &mask_tile {
                let hide = m[i + 3];
                if hide == 255 {
                    continue;
                }
                if hide > 0 {
                    let k = 255 - hide as u32;
                    let px: [u8; 4] =
                        std::array::from_fn(|c| ((src[i + c] as u32 * k + 127) / 255) as u8);
                    blend_over(&mut out[i..i + 4], &px, layer.opacity, layer.mode);
                    continue;
                }
            }
            blend_over(
                &mut out[i..i + 4],
                &src[i..i + 4],
                layer.opacity,
                layer.mode,
            );
        }
    }
    out
}

/// A imagem final (alfa direto) com todas as camadas visíveis.
pub fn flatten(doc: &Doc) -> RgbaImage {
    let mut img = RgbaImage::new(doc.width as u32, doc.height as u32);
    let tx = doc.width.div_ceil(TILE);
    let ty = doc.height.div_ceil(TILE);
    for by in 0..ty {
        for bx in 0..tx {
            let t = composite_tile(doc, bx, by, None);
            for y in 0..TILE {
                let py = by * TILE + y;
                if py >= doc.height {
                    break;
                }
                for x in 0..TILE {
                    let px = bx * TILE + x;
                    if px >= doc.width {
                        break;
                    }
                    let i = (y * TILE + x) * 4;
                    img.put_pixel(
                        px as u32,
                        py as u32,
                        image::Rgba(unpremultiply([t[i], t[i + 1], t[i + 2], t[i + 3]])),
                    );
                }
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Layer;

    fn doc_with_two() -> Doc {
        let mut d = Doc::new(100, 80);
        let mut a = Layer::new(1, "fundo", 100, 80);
        let mut b = Layer::new(2, "topo", 100, 80);
        for x in 0..100 {
            for y in 0..80 {
                a.pixels.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        b.pixels.set_pixel(70, 70, [100, 0, 0, 255]);
        d.layers = vec![a, b];
        d
    }

    #[test]
    fn layers_stack_with_opacity_modes_and_visibility() {
        let mut d = doc_with_two();
        let img = flatten(&d);
        assert_eq!(img.get_pixel(70, 70).0, [100, 0, 0, 255]);
        assert_eq!(img.get_pixel(5, 5).0, [255, 255, 255, 255]);
        d.layers[1].opacity = 0.5;
        assert_eq!(flatten(&d).get_pixel(70, 70).0, [178, 128, 128, 255]);
        d.layers[1].opacity = 1.0;
        d.layers[1].mode = BlendMode::Multiply;
        assert_eq!(flatten(&d).get_pixel(70, 70).0, [100, 0, 0, 255]);
        d.layers[1].visible = false;
        assert_eq!(flatten(&d).get_pixel(70, 70).0, [255, 255, 255, 255]);
    }

    #[test]
    fn live_stroke_shows_before_it_is_committed() {
        let d = doc_with_two();
        let mut buf = StrokeBuffer::new();
        let mut s = crate::brush::Stroker::new(100, 80);
        s.begin(
            crate::brush::Brush {
                size: 20.0,
                hardness: 1.0,
                ..crate::brush::Brush::basic()
            },
            [0, 0, 255],
            false,
        );
        s.add_point(20.0, 20.0);
        buf.clear();
        let live = Live {
            on_mask: false,
            layer: 2,
            buf: &s.buf,
            opacity: 1.0,
            erase: false,
            mode: BlendMode::Normal,
        };
        let t = composite_tile(&d, 0, 0, Some(&live));
        let i = (20 * TILE + 20) * 4;
        assert_eq!(&t[i..i + 4], &[0, 0, 255, 255]);
        let plain = composite_tile(&d, 0, 0, None);
        assert_eq!(&plain[i..i + 4], &[255, 255, 255, 255]);
    }
}
