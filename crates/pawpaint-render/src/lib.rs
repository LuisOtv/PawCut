//! PawPaint: desenha o documento numa imagem (camadas, texto, formas e efeitos) e exporta.

mod effects;
mod text;

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use image::{Rgba as Px, RgbaImage};
use pawpaint_core::{Background, Doc, Kind, Layer, LayerId, ShapeKind, ShapeStyle};

use effects::{adjust_pixels, apply_effects};
use text::TextRasterizer;

pub use text::font_families;

/// Camada já girada, escalada e com efeitos. `ox`/`oy` é o canto de cima à esquerda em relação
/// ao centro da camada: assim mover a camada não precisa redesenhá-la.
struct Placed {
    ox: i32,
    oy: i32,
    img: RgbaImage,
}

pub struct Renderer {
    text: TextRasterizer,
    placed: HashMap<LayerId, (u64, Option<Arc<Placed>>)>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Maior lado (em pixels) de uma camada depois de escalada; acima disso ela é ignorada.
const MAX_LAYER: f32 = 8192.0;

impl Renderer {
    pub fn new() -> Self {
        Self {
            text: TextRasterizer::new(),
            placed: HashMap::new(),
        }
    }

    /// Desenha o documento inteiro na resolução dele.
    pub fn render(&mut self, doc: &Doc, assets: &BTreeMap<String, Arc<RgbaImage>>) -> RgbaImage {
        let mut canvas = background(doc);
        let live: Vec<LayerId> = doc.layers.iter().map(|l| l.id).collect();
        self.placed.retain(|id, _| live.contains(id));
        for layer in doc.layers.iter().filter(|l| l.visible && l.opacity > 0.0) {
            let Some(p) = self.placed_layer(layer, assets) else {
                continue;
            };
            let (x0, y0) = (layer.x.round() as i32 + p.ox, layer.y.round() as i32 + p.oy);
            composite(&mut canvas, &p.img, x0, y0, layer.opacity.clamp(0.0, 1.0));
        }
        canvas
    }

    /// A camada visível e destravada mais por cima que tem tinta no ponto (`x`, `y`) da tela.
    pub fn layer_at(
        &mut self,
        doc: &Doc,
        assets: &BTreeMap<String, Arc<RgbaImage>>,
        x: f32,
        y: f32,
    ) -> Option<LayerId> {
        for layer in doc.layers.iter().rev() {
            if !layer.visible || layer.locked {
                continue;
            }
            let Some(p) = self.placed_layer(layer, assets) else {
                continue;
            };
            let lx = x.floor() as i32 - (layer.x.round() as i32 + p.ox);
            let ly = y.floor() as i32 - (layer.y.round() as i32 + p.oy);
            if lx >= 0
                && ly >= 0
                && (lx as u32) < p.img.width()
                && (ly as u32) < p.img.height()
                && p.img.get_pixel(lx as u32, ly as u32).0[3] > 24
            {
                return Some(layer.id);
            }
        }
        None
    }

    /// Largura e altura do conteúdo da camada na tela (sem girar nem contar efeitos).
    pub fn extent(
        &mut self,
        layer: &Layer,
        assets: &BTreeMap<String, Arc<RgbaImage>>,
    ) -> Option<(f32, f32)> {
        let k = layer.scale.clamp(0.01, 16.0);
        match &layer.kind {
            Kind::Image { asset } => {
                let i = assets.get(asset)?;
                Some((i.width() as f32 * k, i.height() as f32 * k))
            }
            Kind::Text(style) => {
                let i = self.text.rasterize(style, k);
                Some((i.width() as f32, i.height() as f32))
            }
            Kind::Shape(s) => Some((s.w * k, s.h * k)),
        }
    }

    fn placed_layer(
        &mut self,
        layer: &Layer,
        assets: &BTreeMap<String, Arc<RgbaImage>>,
    ) -> Option<Arc<Placed>> {
        // Tudo que muda o desenho, menos posição, opacidade e nome.
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!(
            "{:?}|{}|{}|{}|{:?}|{:?}|{:?}",
            layer.kind,
            layer.scale,
            layer.rotation,
            layer.flip_h,
            layer.outline,
            layer.shadow,
            layer.adjust
        )
        .hash(&mut h);
        let key = h.finish();
        if let Some((k, p)) = self.placed.get(&layer.id)
            && *k == key
        {
            return p.clone();
        }
        let p = self.build(layer, assets).map(Arc::new);
        self.placed.insert(layer.id, (key, p.clone()));
        p
    }

    fn build(
        &mut self,
        layer: &Layer,
        assets: &BTreeMap<String, Arc<RgbaImage>>,
    ) -> Option<Placed> {
        let scale = layer.scale.clamp(0.01, 16.0);
        // Imagens escalam na hora de girar; texto e formas já saem no tamanho final.
        let (mut src, mut k): (RgbaImage, f32) = match &layer.kind {
            Kind::Image { asset } => ((**assets.get(asset)?).clone(), scale),
            Kind::Text(style) => ((*self.text.rasterize(style, scale)).clone(), 1.0),
            Kind::Shape(s) => (shape_image(s, scale), 1.0),
        };
        if src.width() == 0 || src.height() == 0 {
            return None;
        }
        adjust_pixels(&mut src, &layer.adjust);
        // Reduzir muito de uma vez com amostragem simples serrilha: encolhe antes.
        if k < 0.75 {
            let (w, h) = (
                ((src.width() as f32 * k).round() as u32).max(1),
                ((src.height() as f32 * k).round() as u32).max(1),
            );
            src = image::imageops::resize(&src, w, h, image::imageops::FilterType::Triangle);
            k = 1.0;
        }
        let pad = effects::padding(layer);
        let (sw, sh) = (src.width() as f32 * k, src.height() as f32 * k);
        let rad = layer.rotation.to_radians();
        let (c, s) = (rad.cos(), rad.sin());
        let bw = sw * c.abs() + sh * s.abs();
        let bh = sw * s.abs() + sh * c.abs();
        let (dw, dh) = (bw.ceil() + 2.0 * pad, bh.ceil() + 2.0 * pad);
        if dw > MAX_LAYER || dh > MAX_LAYER {
            return None;
        }
        let (dw, dh) = (dw as u32, dh as u32);
        let mut out = RgbaImage::new(dw, dh);
        let (scx, scy) = (src.width() as f32 / 2.0, src.height() as f32 / 2.0);
        let (dcx, dcy) = (dw as f32 / 2.0, dh as f32 / 2.0);
        let plain = layer.rotation == 0.0 && k == 1.0 && !layer.flip_h;
        for y in 0..dh {
            for x in 0..dw {
                let (rx, ry) = (x as f32 + 0.5 - dcx, y as f32 + 0.5 - dcy);
                // Gira de volta (a camada gira no sentido horário na tela).
                let (mut u, v) = (rx * c + ry * s, -rx * s + ry * c);
                u /= k;
                let v = v / k;
                let u = if layer.flip_h { -u } else { u };
                let (u, v) = (u + scx, v + scy);
                let px = if plain {
                    let (ix, iy) = (u.floor() as i64, v.floor() as i64);
                    src_at(&src, ix, iy)
                } else {
                    bilinear(&src, u - 0.5, v - 0.5)
                };
                out.put_pixel(x, y, Px(px));
            }
        }
        let out = apply_effects(out, layer);
        Some(Placed {
            ox: -(dw as i32) / 2,
            oy: -(dh as i32) / 2,
            img: out,
        })
    }
}

fn src_at(img: &RgbaImage, x: i64, y: i64) -> [u8; 4] {
    if x < 0 || y < 0 || x >= img.width() as i64 || y >= img.height() as i64 {
        [0; 4]
    } else {
        img.get_pixel(x as u32, y as u32).0
    }
}

/// Amostra com interpolação bilinear em alfa pré-multiplicado (sem halo escuro nas bordas).
fn bilinear(img: &RgbaImage, x: f32, y: f32) -> [u8; 4] {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let mut acc = [0.0f32; 4];
    for (dx, dy, w) in [
        (0, 0, (1.0 - fx) * (1.0 - fy)),
        (1, 0, fx * (1.0 - fy)),
        (0, 1, (1.0 - fx) * fy),
        (1, 1, fx * fy),
    ] {
        if w <= 0.0 {
            continue;
        }
        let p = src_at(img, x0 + dx, y0 + dy);
        let a = p[3] as f32 / 255.0;
        acc[0] += p[0] as f32 * a * w;
        acc[1] += p[1] as f32 * a * w;
        acc[2] += p[2] as f32 * a * w;
        acc[3] += a * w;
    }
    if acc[3] <= 0.0 {
        return [0; 4];
    }
    [
        (acc[0] / acc[3]).round().clamp(0.0, 255.0) as u8,
        (acc[1] / acc[3]).round().clamp(0.0, 255.0) as u8,
        (acc[2] / acc[3]).round().clamp(0.0, 255.0) as u8,
        (acc[3] * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// Forma com bordas suaves, já no tamanho final (`k` = escala da camada).
fn shape_image(s: &ShapeStyle, k: f32) -> RgbaImage {
    let (w, h) = ((s.w * k).max(1.0), (s.h * k).max(1.0));
    let (iw, ih) = (w.ceil() as u32 + 2, h.ceil() as u32 + 2);
    let mut img = RgbaImage::new(iw, ih);
    let (cx, cy) = (iw as f32 / 2.0, ih as f32 / 2.0);
    let (hw, hh) = (w / 2.0, h / 2.0);
    let radius = (s.radius * k).clamp(0.0, hw.min(hh));
    let stroke = (s.stroke_width * k).max(0.0);
    for y in 0..ih {
        for x in 0..iw {
            let (px, py) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            // Distância assinada até a borda da forma (negativa dentro).
            let d = match s.kind {
                ShapeKind::Rect => {
                    let (qx, qy) = (px.abs() - (hw - radius), py.abs() - (hh - radius));
                    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0)
                        - radius
                }
                ShapeKind::Ellipse => {
                    // Aproximação boa para elipses: distância normalizada.
                    let n = ((px / hw).powi(2) + (py / hh).powi(2)).sqrt();
                    (n - 1.0) * hw.min(hh)
                }
            };
            let cover = (0.5 - d).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            // Borda por dentro: mistura o traço com o preenchimento.
            let in_stroke = if stroke > 0.0 {
                (d + stroke + 0.5).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (f, st) = (s.fill, s.stroke);
            let mix = |a: u8, b: u8| (a as f32 * (1.0 - in_stroke) + b as f32 * in_stroke) as u8;
            let alpha = (f.a as f32 * (1.0 - in_stroke) + st.a as f32 * in_stroke) * cover;
            img.put_pixel(
                x,
                y,
                Px([mix(f.r, st.r), mix(f.g, st.g), mix(f.b, st.b), alpha as u8]),
            );
        }
    }
    img
}

fn background(doc: &Doc) -> RgbaImage {
    let (w, h) = (doc.width, doc.height);
    match &doc.background {
        Background::Solid(c) => RgbaImage::from_pixel(w, h, Px([c.r, c.g, c.b, 255])),
        Background::Gradient { from, to, angle } => {
            let a = angle.to_radians();
            let (dx, dy) = (a.sin(), a.cos());
            let extent = (w as f32 * dx.abs() + h as f32 * dy.abs()).max(1.0);
            let mut img = RgbaImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let t = (((x as f32 + 0.5 - w as f32 / 2.0) * dx
                        + (y as f32 + 0.5 - h as f32 / 2.0) * dy)
                        / extent
                        + 0.5)
                        .clamp(0.0, 1.0);
                    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
                    img.put_pixel(
                        x,
                        y,
                        Px([mix(from.r, to.r), mix(from.g, to.g), mix(from.b, to.b), 255]),
                    );
                }
            }
            img
        }
    }
}

/// Põe `layer` sobre `canvas` com o canto em (`x0`, `y0`) e a opacidade dada.
fn composite(canvas: &mut RgbaImage, layer: &RgbaImage, x0: i32, y0: i32, opacity: f32) {
    let (cw, ch) = (canvas.width() as i32, canvas.height() as i32);
    for ly in 0..layer.height() as i32 {
        let cy = y0 + ly;
        if cy < 0 || cy >= ch {
            continue;
        }
        for lx in 0..layer.width() as i32 {
            let cx = x0 + lx;
            if cx < 0 || cx >= cw {
                continue;
            }
            let s = layer.get_pixel(lx as u32, ly as u32).0;
            if s[3] == 0 {
                continue;
            }
            let a = s[3] as f32 / 255.0 * opacity;
            let d = canvas.get_pixel_mut(cx as u32, cy as u32);
            let d_a = d.0[3] as f32 / 255.0;
            let out_a = a + d_a * (1.0 - a);
            for (dst, src) in d.0.iter_mut().zip(s).take(3) {
                let v = (src as f32 * a + *dst as f32 * d_a * (1.0 - a)) / out_a;
                *dst = v.round().clamp(0.0, 255.0) as u8;
            }
            d.0[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// JPEG com a qualidade dada (a imagem vai sobre fundo preto se tiver transparência).
pub fn encode_jpeg(img: &RgbaImage, quality: u8) -> Vec<u8> {
    let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    let _ = rgb.write_with_encoder(enc);
    out
}

/// O JPEG de maior qualidade que cabe em `max_bytes` (o YouTube aceita até 2 MB). Devolve os
/// bytes e a qualidade usada; se nem a qualidade 30 cabe, devolve essa mesma.
pub fn jpeg_under(img: &RgbaImage, max_bytes: usize) -> (Vec<u8>, u8) {
    let mut best = (encode_jpeg(img, 30), 30);
    // Busca binária: o tamanho cresce com a qualidade.
    let (mut lo, mut hi) = (30u8, 98u8);
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let bytes = encode_jpeg(img, mid);
        if bytes.len() <= max_bytes {
            best = (bytes, mid);
            lo = mid + 1;
        } else {
            if mid == 0 {
                break;
            }
            hi = mid - 1;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use pawpaint_core::{Editor, Rgba, ShapeKind};

    fn px(img: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
        img.get_pixel(x, y).0
    }

    fn render(e: &Editor) -> RgbaImage {
        Renderer::new().render(e.doc(), e.assets())
    }

    #[test]
    fn solid_background_and_a_centered_shape() {
        let mut e = Editor::new(200, 100);
        e.set_background(Background::Solid(Rgba::rgb(10, 20, 30)));
        let id = e.add_shape(ShapeKind::Rect);
        e.edit(id, "t", "t", |l| {
            if let Kind::Shape(s) = &mut l.kind {
                s.w = 40.0;
                s.h = 20.0;
                s.fill = Rgba::rgb(255, 0, 0);
            }
        });
        let img = render(&e);
        assert_eq!(px(&img, 5, 5), [10, 20, 30, 255]);
        assert_eq!(px(&img, 100, 50), [255, 0, 0, 255]);
        assert_eq!(px(&img, 100 + 19, 50), [255, 0, 0, 255]);
        assert_eq!(px(&img, 100 + 25, 50), [10, 20, 30, 255]);
    }

    #[test]
    fn moving_a_layer_does_not_redraw_it_and_hidden_layers_vanish() {
        let mut e = Editor::new(200, 100);
        e.set_background(Background::Solid(Rgba::BLACK));
        let id = e.add_shape(ShapeKind::Ellipse);
        e.edit(id, "s", "s", |l| {
            if let Kind::Shape(s) = &mut l.kind {
                s.w = 40.0;
                s.h = 40.0;
            }
        });
        let mut r = Renderer::new();
        let a = r.render(e.doc(), e.assets());
        let before = Arc::as_ptr(r.placed[&id].1.as_ref().unwrap());
        e.edit(id, "m", "m", |l| l.x = 60.0);
        let b = r.render(e.doc(), e.assets());
        let after = Arc::as_ptr(r.placed[&id].1.as_ref().unwrap());
        assert_eq!(before, after, "mover não devia redesenhar a camada");
        assert!(a != b, "a posição devia mudar a imagem");
        e.edit(id, "v", "v", |l| l.visible = false);
        assert_eq!(px(&r.render(e.doc(), e.assets()), 60, 50), [0, 0, 0, 255]);
    }

    #[test]
    fn rotation_and_opacity() {
        let mut e = Editor::new(100, 100);
        e.set_background(Background::Solid(Rgba::BLACK));
        let id = e.add_shape(ShapeKind::Rect);
        e.edit(id, "s", "s", |l| {
            if let Kind::Shape(s) = &mut l.kind {
                s.w = 80.0;
                s.h = 10.0;
                s.fill = Rgba::WHITE;
            }
            l.rotation = 90.0;
            l.opacity = 0.5;
        });
        let img = render(&e);
        // virou uma barra em pé, pela metade da opacidade
        assert!(px(&img, 50, 20)[0] > 100 && px(&img, 50, 20)[0] < 160);
        assert_eq!(px(&img, 20, 50), [0, 0, 0, 255]);
    }

    #[test]
    fn outline_grows_around_the_shape_and_shadow_goes_under() {
        let mut e = Editor::new(200, 200);
        e.set_background(Background::Solid(Rgba::rgb(0, 0, 255)));
        let id = e.add_shape(ShapeKind::Rect);
        e.edit(id, "s", "s", |l| {
            if let Kind::Shape(s) = &mut l.kind {
                s.w = 40.0;
                s.h = 40.0;
                s.fill = Rgba::rgb(255, 255, 255);
            }
            l.outline = Some(pawpaint_core::Outline {
                width: 10.0,
                color: Rgba::rgb(255, 0, 0),
            });
        });
        let img = render(&e);
        assert_eq!(px(&img, 100, 100), [255, 255, 255, 255]);
        // 5 px fora da forma: contorno vermelho
        assert_eq!(px(&img, 100 + 25, 100), [255, 0, 0, 255]);
        // bem longe: fundo
        assert_eq!(px(&img, 100 + 40, 100), [0, 0, 255, 255]);
    }

    #[test]
    fn chroma_key_removes_the_background_color() {
        let mut e = Editor::new(100, 100);
        e.set_background(Background::Solid(Rgba::BLACK));
        let mut img = RgbaImage::from_pixel(10, 10, Px([0, 255, 0, 255]));
        img.put_pixel(5, 5, Px([200, 50, 50, 255]));
        let id = e.add_image("tela verde", img);
        e.edit(id, "c", "c", |l| {
            l.scale = 1.0;
            l.adjust.chroma = Some(pawpaint_core::Chroma {
                color: Rgba::rgb(0, 255, 0),
                tolerance: 0.2,
                softness: 0.0,
            });
        });
        let out = render(&e);
        // o canto verde sumiu (fundo preto aparece); o pixel vermelho ficou
        assert_eq!(px(&out, 50 - 5, 50 - 5), [0, 0, 0, 255]);
        assert_eq!(px(&out, 50, 50), [200, 50, 50, 255]);
    }

    #[test]
    fn text_draws_something_and_jpeg_fits_the_limit() {
        let mut e = Editor::new(640, 360);
        e.set_background(Background::Solid(Rgba::rgb(20, 20, 20)));
        e.add_text("OLÁ");
        let img = render(&e);
        let lit = img.pixels().filter(|p| p.0[0] > 200).count();
        assert!(lit > 500, "o texto devia ter pintado pixels claros: {lit}");
        let (bytes, q) = jpeg_under(&img, 20_000);
        assert!(
            bytes.len() <= 20_000 && q >= 30,
            "{} bytes, q{q}",
            bytes.len()
        );
        assert!(encode_jpeg(&img, 95).len() > bytes.len() || q >= 95);
    }
}
