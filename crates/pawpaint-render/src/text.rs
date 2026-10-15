//! Texto com cosmic-text: só os glifos (contorno e sombra são efeitos da camada).

use std::collections::HashMap;
use std::sync::Arc;

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight,
};
use image::RgbaImage;
use pawpaint_core::{TextAlign, TextStyle};

const CACHE_LIMIT: usize = 48;

pub(crate) struct TextRasterizer {
    fonts: FontSystem,
    glyphs: SwashCache,
    cache: HashMap<String, Arc<RgbaImage>>,
}

/// Nomes das fontes instaladas, em ordem.
pub fn font_families() -> Vec<String> {
    let fonts = FontSystem::new();
    let mut names: Vec<String> = fonts
        .db()
        .faces()
        .flat_map(|f| f.families.iter().map(|(n, _)| n.clone()))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

fn family(name: &str) -> Family<'_> {
    match name {
        "sans-serif" => Family::SansSerif,
        "serif" => Family::Serif,
        "monospace" => Family::Monospace,
        other => Family::Name(other),
    }
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
        let mut attrs = Attrs::new().family(family(&style.font));
        if style.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        if style.italic {
            attrs = attrs.style(Style::Italic);
        }
        let align = match style.align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        };
        let line = size * 1.2 * style.line_spacing.clamp(0.5, 3.0);
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, line));
        // Mede a linha mais larga e alinha as outras dentro dela.
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
        let pad = (size * 0.1).ceil() as usize + 2;
        let (w, h) = (text_w as usize + 2 * pad, text_h as usize + 2 * pad);
        let mut img = RgbaImage::new(w as u32, h as u32);
        let col = style.color;
        buffer.draw(
            &mut self.fonts,
            &mut self.glyphs,
            Color::rgba(255, 255, 255, 255),
            |x, y, gw, gh, c| {
                let cov = c.a() as f32 / 255.0 * col.a as f32 / 255.0;
                for py in y.max(0)..(y + gh as i32) {
                    for px in x.max(0)..(x + gw as i32) {
                        let (mx, my) = (px as usize + pad, py as usize + pad);
                        if mx < w && my < h {
                            let d = img.get_pixel_mut(mx as u32, my as u32);
                            let da = d.0[3] as f32 / 255.0;
                            let out = cov + da * (1.0 - cov);
                            d.0 = [col.r, col.g, col.b, (out * 255.0).round() as u8];
                        }
                    }
                }
            },
        );
        img
    }
}
