//! Rasterização de títulos com cosmic-text: negrito, alinhamento, contorno, sombra e caixa.

use std::collections::HashMap;
use std::sync::Arc;

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight,
};
use image::RgbaImage;
use pawcut_core::{Rgba, TextAlign, TextStyle};

/// Quantos textos rasterizados ficam em cache (mudam pouco de um frame para o outro).
const CACHE_LIMIT: usize = 64;

pub(crate) struct TextRasterizer {
    fonts: FontSystem,
    glyphs: SwashCache,
    cache: HashMap<String, Arc<RgbaImage>>,
}

/// Cobertura (0..1) por pixel.
struct Mask {
    w: usize,
    h: usize,
    a: Vec<f32>,
}

impl Mask {
    fn get(&self, x: i64, y: i64) -> f32 {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            0.0
        } else {
            self.a[y as usize * self.w + x as usize]
        }
    }
}

/// Pinta `color` com cobertura `cov` sobre o pixel (alfa "por cima").
fn over(dst: &mut image::Rgba<u8>, color: Rgba, cov: f32) {
    let a = color.a as f32 / 255.0 * cov.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let da = dst.0[3] as f32 / 255.0;
    let out_a = a + da * (1.0 - a);
    let mix = |s: u8, d: u8| ((s as f32 * a + d as f32 * da * (1.0 - a)) / out_a).round() as u8;
    dst.0 = [
        mix(color.r, dst.0[0]),
        mix(color.g, dst.0[1]),
        mix(color.b, dst.0[2]),
        (out_a * 255.0).round() as u8,
    ];
}

impl TextRasterizer {
    pub(crate) fn new() -> Self {
        Self {
            fonts: FontSystem::new(),
            glyphs: SwashCache::new(),
            cache: HashMap::new(),
        }
    }

    /// Texto em RGBA transparente, com a fonte em `style.size * k` pixels.
    pub(crate) fn rasterize(&mut self, style: &TextStyle, k: f32) -> Arc<RgbaImage> {
        let key = format!("{style:?}|{k}");
        if let Some(img) = self.cache.get(&key) {
            return img.clone();
        }
        let img = Arc::new(self.render(style, k));
        if self.cache.len() >= CACHE_LIMIT {
            self.cache.clear();
        }
        self.cache.insert(key, img.clone());
        img
    }

    fn render(&mut self, style: &TextStyle, k: f32) -> RgbaImage {
        let size = (style.size * k).max(1.0);
        let mut attrs = Attrs::new().family(Family::Name(&style.font));
        if style.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        let align = match style.align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        };
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.2));
        // Primeiro mede sem largura; depois alinha as linhas dentro da maior.
        buffer.set_size(None, None);
        buffer.set_text(&style.text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.fonts, false);
        let text_w = buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0f32, f32::max)
            .ceil()
            .max(1.0);
        buffer.set_size(Some(text_w), None);
        buffer.set_text(&style.text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.fonts, false);
        let text_h = buffer
            .layout_runs()
            .map(|r| r.line_top + r.line_height)
            .fold(0.0f32, f32::max)
            .ceil()
            .max(1.0);

        let outline = (style.outline * k).max(0.0);
        let shadow = if style.shadow {
            (size * 0.06).max(1.0)
        } else {
            0.0
        };
        let pad_box = if style.background { size * 0.3 } else { 0.0 };
        let pad = (outline.ceil() + shadow.ceil() + pad_box.ceil() + 1.0) as usize;
        let (tw, th) = (text_w as usize, text_h as usize);
        let (w, h) = (tw + 2 * pad, th + 2 * pad);

        // Cobertura dos glifos.
        let mut mask = Mask {
            w,
            h,
            a: vec![0.0; w * h],
        };
        buffer.draw(
            &mut self.fonts,
            &mut self.glyphs,
            Color::rgba(255, 255, 255, 255),
            |x, y, gw, gh, color| {
                let cov = color.a() as f32 / 255.0;
                for py in y.max(0)..(y + gh as i32) {
                    for px in x.max(0)..(x + gw as i32) {
                        let (mx, my) = (px as usize + pad, py as usize + pad);
                        if mx < w && my < h {
                            let v = &mut mask.a[my * w + mx];
                            *v = (*v + cov * (1.0 - *v)).min(1.0);
                        }
                    }
                }
            },
        );

        let mut img = RgbaImage::new(w as u32, h as u32);
        if style.background {
            let (x0, y0) = (pad - pad_box as usize, pad - pad_box as usize);
            let (x1, y1) = (
                (pad + tw + pad_box as usize).min(w),
                (pad + th + pad_box as usize).min(h),
            );
            for y in y0..y1 {
                for x in x0..x1 {
                    over(
                        img.get_pixel_mut(x as u32, y as u32),
                        style.background_color,
                        1.0,
                    );
                }
            }
        }
        if shadow > 0.0 {
            let off = shadow.round() as i64;
            let color = Rgba::new(0, 0, 0, 170);
            for y in 0..h {
                for x in 0..w {
                    // Sombra levemente borrada (média 3x3) e deslocada.
                    let mut sum = 0.0;
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            sum += mask.get(x as i64 - off + dx, y as i64 - off + dy);
                        }
                    }
                    over(img.get_pixel_mut(x as u32, y as u32), color, sum / 9.0);
                }
            }
        }
        if outline > 0.0 {
            // Contorno: dilatação da cobertura num círculo de raio `outline`.
            let r = outline.ceil() as i64;
            let offsets: Vec<(i64, i64, f32)> = (-r..=r)
                .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
                .filter_map(|(dx, dy)| {
                    let d = ((dx * dx + dy * dy) as f32).sqrt();
                    (d <= outline + 0.5).then_some((dx, dy, (outline + 0.5 - d).min(1.0)))
                })
                .collect();
            for y in 0..h {
                for x in 0..w {
                    let cov = offsets.iter().fold(0.0f32, |m, (dx, dy, wgt)| {
                        m.max(mask.get(x as i64 + dx, y as i64 + dy) * wgt)
                    });
                    over(
                        img.get_pixel_mut(x as u32, y as u32),
                        style.outline_color,
                        cov,
                    );
                }
            }
        }
        for y in 0..h {
            for x in 0..w {
                over(
                    img.get_pixel_mut(x as u32, y as u32),
                    style.color,
                    mask.a[y * w + x],
                );
            }
        }
        img
    }
}
