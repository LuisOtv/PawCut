//! Texto rasterizado (cosmic-text) para a ferramenta de texto.

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight,
};
use image::RgbaImage;

pub struct TextRaster {
    fonts: FontSystem,
    glyphs: SwashCache,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextOpts {
    pub text: String,
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: [u8; 3],
    pub align: u8, // 0 esquerda, 1 centro, 2 direita
}

impl Default for TextOpts {
    fn default() -> Self {
        Self {
            text: String::new(),
            font: "sans-serif".into(),
            size: 48.0,
            bold: false,
            italic: false,
            color: [0, 0, 0],
            align: 0,
        }
    }
}

impl Default for TextRaster {
    fn default() -> Self {
        Self::new()
    }
}

fn family(name: &str) -> Family<'_> {
    match name {
        "sans-serif" => Family::SansSerif,
        "serif" => Family::Serif,
        "monospace" => Family::Monospace,
        other => Family::Name(other),
    }
}

impl TextRaster {
    pub fn new() -> Self {
        Self {
            fonts: FontSystem::new(),
            glyphs: SwashCache::new(),
        }
    }

    /// Nomes das fontes instaladas.
    pub fn families(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .fonts
            .db()
            .faces()
            .flat_map(|f| f.families.iter().map(|(n, _)| n.clone()))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    }

    /// Imagem do texto (alfa direto) com margem de 2 px; vazia se não houver texto.
    pub fn render(&mut self, o: &TextOpts) -> RgbaImage {
        if o.text.trim().is_empty() {
            return RgbaImage::new(1, 1);
        }
        let size = o.size.max(4.0);
        let mut attrs = Attrs::new().family(family(&o.font));
        if o.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        if o.italic {
            attrs = attrs.style(Style::Italic);
        }
        let align = match o.align {
            1 => Align::Center,
            2 => Align::Right,
            _ => Align::Left,
        };
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.2));
        buffer.set_size(None, None);
        buffer.set_text(&o.text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.fonts, false);
        let text_w = buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0f32, f32::max)
            .ceil()
            .max(1.0);
        buffer.set_size(Some(text_w), None);
        buffer.set_text(&o.text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.fonts, false);
        let text_h = buffer
            .layout_runs()
            .map(|r| r.line_top + r.line_height)
            .fold(0.0f32, f32::max)
            .ceil()
            .max(1.0);
        let pad = 2usize;
        let (w, h) = (text_w as usize + 2 * pad, text_h as usize + 2 * pad);
        let mut img = RgbaImage::new(w as u32, h as u32);
        let col = o.color;
        buffer.draw(
            &mut self.fonts,
            &mut self.glyphs,
            Color::rgba(255, 255, 255, 255),
            |x, y, gw, gh, c| {
                let cov = c.a() as f32 / 255.0;
                for py in y.max(0)..(y + gh as i32) {
                    for px in x.max(0)..(x + gw as i32) {
                        let (mx, my) = (px as usize + pad, py as usize + pad);
                        if mx < w && my < h {
                            let d = img.get_pixel_mut(mx as u32, my as u32);
                            let da = d.0[3] as f32 / 255.0;
                            let out = cov + da * (1.0 - cov);
                            d.0 = [col[0], col[1], col[2], (out * 255.0).round() as u8];
                        }
                    }
                }
            },
        );
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_visible_pixels_in_the_chosen_color() {
        let mut r = TextRaster::new();
        let img = r.render(&TextOpts {
            text: "Oi".into(),
            size: 60.0,
            color: [255, 0, 0],
            ..Default::default()
        });
        assert!(img.width() > 20 && img.height() > 20);
        let lit: Vec<_> = img.pixels().filter(|p| p.0[3] > 200).collect();
        assert!(lit.len() > 100);
        assert!(lit.iter().all(|p| p.0[..3] == [255, 0, 0]));
        assert_eq!(r.render(&TextOpts::default()).width(), 1);
    }
}
