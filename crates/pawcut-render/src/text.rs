//! Rasterização de títulos com cosmic-text.

use std::collections::HashMap;
use std::sync::Arc;

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use image::RgbaImage;
use pawcut_core::TextStyle;

/// Quantos textos rasterizados ficam em cache (mudam pouco de um frame para o outro).
const CACHE_LIMIT: usize = 64;

pub(crate) struct TextRasterizer {
    fonts: FontSystem,
    glyphs: SwashCache,
    cache: HashMap<String, Arc<RgbaImage>>,
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
        let size = (style.size * k).max(1.0);
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.2));
        buffer.set_size(None, None);
        let attrs = Attrs::new().family(Family::Name(&style.font));
        buffer.set_text(&style.text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);

        let (mut w, mut h) = (0.0f32, 0.0f32);
        for run in buffer.layout_runs() {
            w = w.max(run.line_w);
            h = h.max(run.line_top + run.line_height);
        }
        let (iw, ih) = ((w.ceil() as u32).max(1), (h.ceil() as u32).max(1));
        let mut img = RgbaImage::new(iw, ih);
        let c = style.color;
        buffer.draw(
            &mut self.fonts,
            &mut self.glyphs,
            Color::rgba(c.r, c.g, c.b, c.a),
            |x, y, gw, gh, color| {
                let a = color.a() as f32 / 255.0;
                if a <= 0.0 {
                    return;
                }
                for py in y.max(0)..(y + gh as i32).min(ih as i32) {
                    for px in x.max(0)..(x + gw as i32).min(iw as i32) {
                        let dst = img.get_pixel_mut(px as u32, py as u32);
                        let da = dst.0[3] as f32 / 255.0;
                        let out_a = a + da * (1.0 - a);
                        let mix = |s: u8, d: u8| {
                            ((s as f32 * a + d as f32 * da * (1.0 - a)) / out_a).round() as u8
                        };
                        dst.0 = [
                            mix(color.r(), dst.0[0]),
                            mix(color.g(), dst.0[1]),
                            mix(color.b(), dst.0[2]),
                            (out_a * 255.0).round() as u8,
                        ];
                    }
                }
            },
        );
        let img = Arc::new(img);
        if self.cache.len() >= CACHE_LIMIT {
            self.cache.clear();
        }
        self.cache.insert(key, img.clone());
        img
    }
}
