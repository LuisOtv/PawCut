//! Operações de pixel no quadro de saída (sempre opaco).

use image::RgbaImage;
use pawcut_core::{ColorAdjust, Rgba};
use rayon::prelude::*;

use crate::layout::Rect;

/// Pixels cujo centro cai em `[start, start + len)`, limitados a `[0, max)`.
fn span(start: f32, len: f32, max: u32) -> (usize, usize) {
    let a = (start - 0.5).ceil().max(0.0) as usize;
    let b = (start + len - 0.5).ceil().clamp(0.0, max as f32) as usize;
    (a, b.max(a))
}

#[inline]
fn blend(dst: &mut [u8], src: [f32; 3], a: f32) {
    for i in 0..3 {
        dst[i] = (src[i] * a + dst[i] as f32 * (1.0 - a)).round() as u8;
    }
    dst[3] = 255;
}

/// Matiz/Saturação/Luminosidade no jeito do Ctrl+U do Photoshop (RGB em 0..255).
pub(crate) fn adjust_color(rgb: [f32; 3], c: &ColorAdjust) -> [f32; 3] {
    let [r, g, b] = rgb.map(|v| v / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let (mut h, mut s) = (0.0f32, 0.0f32);
    if d > 1e-6 {
        s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } * 60.0;
    }
    h = (h + c.hue).rem_euclid(360.0);
    s = if c.saturation >= 0.0 {
        s + (1.0 - s) * s * c.saturation / 100.0
    } else {
        s * (1.0 + c.saturation / 100.0)
    }
    .clamp(0.0, 1.0);
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let pp = 2.0 * l - q;
    let hue_to = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            pp + (q - pp) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            pp + (q - pp) * (2.0 / 3.0 - t) * 6.0
        } else {
            pp
        }
    };
    let hn = h / 360.0;
    let mut out = if s < 1e-6 {
        [l, l, l]
    } else {
        [hue_to(hn + 1.0 / 3.0), hue_to(hn), hue_to(hn - 1.0 / 3.0)]
    };
    let k = c.lightness / 100.0;
    for v in &mut out {
        *v = if k >= 0.0 {
            *v + (1.0 - *v) * k
        } else {
            *v * (1.0 + k)
        };
    }
    out.map(|v| (v * 255.0).clamp(0.0, 255.0))
}

pub(crate) fn fill_rect(
    canvas: &mut RgbaImage,
    r: Rect,
    color: Rgba,
    opacity: f32,
    adjust: Option<&ColorAdjust>,
) {
    let a = color.a as f32 / 255.0 * opacity;
    let (w, h) = canvas.dimensions();
    let ((x0, x1), (y0, y1)) = (span(r.x0, r.w, w), span(r.y0, r.h, h));
    if a <= 0.0 || x0 == x1 || y0 == y1 {
        return;
    }
    let mut src = [color.r as f32, color.g as f32, color.b as f32];
    if let Some(c) = adjust {
        src = adjust_color(src, c);
    }
    let buf: &mut [u8] = &mut *canvas;
    buf.par_chunks_mut(w as usize * 4)
        .enumerate()
        .filter(|(y, _)| (y0..y1).contains(y))
        .for_each(|(_, row)| {
            for x in x0..x1 {
                blend(&mut row[x * 4..x * 4 + 4], src, a);
            }
        });
}

/// Amostra bilinear com bordas presas; devolve RGBA em 0..255.
fn sample(img: &RgbaImage, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = img.dimensions();
    let u = u.clamp(0.0, (w - 1) as f32);
    let v = v.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (u.floor() as u32, v.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (u - x0 as f32, v - y0 as f32);
    let (p00, p10, p01, p11) = (
        img.get_pixel(x0, y0).0,
        img.get_pixel(x1, y0).0,
        img.get_pixel(x0, y1).0,
        img.get_pixel(x1, y1).0,
    );
    let mut out = [0.0; 4];
    for c in 0..4 {
        let top = p00[c] as f32 * (1.0 - fx) + p10[c] as f32 * fx;
        let bottom = p01[c] as f32 * (1.0 - fx) + p11[c] as f32 * fx;
        out[c] = top * (1.0 - fy) + bottom * fy;
    }
    out
}

/// Como desenhar uma imagem: que parte dela (UV 0..1), espelhada ou não, com ajuste de cor.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrawOpts {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub color: Option<ColorAdjust>,
}

impl Default for DrawOpts {
    fn default() -> Self {
        Self {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            flip_h: false,
            flip_v: false,
            color: None,
        }
    }
}

/// Desenha (a região `opts` de) `src` esticada em `r`, respeitando o alfa e a `opacity`.
pub(crate) fn draw_image(
    canvas: &mut RgbaImage,
    src: &RgbaImage,
    r: Rect,
    opacity: f32,
    opts: &DrawOpts,
) {
    let (w, h) = canvas.dimensions();
    let ((x0, x1), (y0, y1)) = (span(r.x0, r.w, w), span(r.y0, r.h, h));
    if opacity <= 0.0 || x0 == x1 || y0 == y1 || src.width() == 0 || src.height() == 0 {
        return;
    }
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let buf: &mut [u8] = &mut *canvas;
    buf.par_chunks_mut(w as usize * 4)
        .enumerate()
        .filter(|(y, _)| (y0..y1).contains(y))
        .for_each(|(y, row)| {
            let mut fy = (y as f32 + 0.5 - r.y0) / r.h;
            if opts.flip_v {
                fy = 1.0 - fy;
            }
            let v = (opts.v0 + (opts.v1 - opts.v0) * fy) * sh - 0.5;
            for x in x0..x1 {
                let mut fx = (x as f32 + 0.5 - r.x0) / r.w;
                if opts.flip_h {
                    fx = 1.0 - fx;
                }
                let u = (opts.u0 + (opts.u1 - opts.u0) * fx) * sw - 0.5;
                let p = sample(src, u, v);
                let mut rgb = [p[0], p[1], p[2]];
                if let Some(c) = &opts.color {
                    rgb = adjust_color(rgb, c);
                }
                blend(&mut row[x * 4..x * 4 + 4], rgb, p[3] / 255.0 * opacity);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn black(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]))
    }

    #[test]
    fn fill_covers_pixels_whose_center_is_inside() {
        let mut c = black(4, 4);
        fill_rect(
            &mut c,
            Rect {
                x0: 1.0,
                y0: 1.0,
                w: 2.0,
                h: 2.0,
            },
            Rgba::new(255, 0, 0, 255),
            1.0,
            None,
        );
        assert_eq!(c.get_pixel(1, 1).0, [255, 0, 0, 255]);
        assert_eq!(c.get_pixel(2, 2).0, [255, 0, 0, 255]);
        assert_eq!(c.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(c.get_pixel(3, 3).0, [0, 0, 0, 255]);
    }

    #[test]
    fn half_opacity_blends() {
        let mut c = black(2, 2);
        fill_rect(
            &mut c,
            Rect {
                x0: 0.0,
                y0: 0.0,
                w: 2.0,
                h: 2.0,
            },
            Rgba::new(255, 255, 255, 255),
            0.5,
            None,
        );
        assert_eq!(c.get_pixel(0, 0).0, [128, 128, 128, 255]);
    }

    #[test]
    fn rect_outside_canvas_is_ignored() {
        let mut c = black(2, 2);
        fill_rect(
            &mut c,
            Rect {
                x0: -10.0,
                y0: 5.0,
                w: 3.0,
                h: 3.0,
            },
            Rgba::new(255, 0, 0, 255),
            1.0,
            None,
        );
        draw_image(
            &mut c,
            &black(1, 1),
            Rect {
                x0: 9.0,
                y0: 9.0,
                w: 1.0,
                h: 1.0,
            },
            1.0,
            &DrawOpts::default(),
        );
        assert_eq!(c, black(2, 2));
    }

    #[test]
    fn image_is_stretched_and_alpha_respected() {
        let mut src = RgbaImage::new(2, 1);
        src.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        src.put_pixel(1, 0, image::Rgba([0, 255, 0, 0]));
        let mut c = black(8, 2);
        draw_image(
            &mut c,
            &src,
            Rect {
                x0: 0.0,
                y0: 0.0,
                w: 8.0,
                h: 2.0,
            },
            1.0,
            &DrawOpts::default(),
        );
        assert_eq!(c.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(c.get_pixel(7, 1).0, [0, 0, 0, 255]);
    }

    #[test]
    fn hue_saturation_lightness_like_photoshop() {
        let red = [255.0, 0.0, 0.0];
        let shifted = adjust_color(
            red,
            &ColorAdjust {
                hue: 120.0,
                ..Default::default()
            },
        );
        assert!(
            shifted[1] > 250.0 && shifted[0] < 5.0,
            "vermelho + 120° vira verde: {shifted:?}"
        );
        let gray = adjust_color(
            red,
            &ColorAdjust {
                saturation: -100.0,
                ..Default::default()
            },
        );
        assert!((gray[0] - gray[1]).abs() < 1.0 && (gray[1] - gray[2]).abs() < 1.0);
        let white = adjust_color(
            red,
            &ColorAdjust {
                lightness: 100.0,
                ..Default::default()
            },
        );
        assert_eq!(white, [255.0, 255.0, 255.0]);
        assert_eq!(
            adjust_color([10.0, 20.0, 30.0], &ColorAdjust::default()).map(|v| v.round()),
            [10.0, 20.0, 30.0]
        );
    }

    #[test]
    fn crop_region_and_flip() {
        // 8 px: metade esquerda vermelha, direita azul.
        let mut src = RgbaImage::new(8, 1);
        for x in 0..8 {
            let c = if x < 4 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            };
            src.put_pixel(x, 0, image::Rgba(c));
        }
        let r = Rect {
            x0: 0.0,
            y0: 0.0,
            w: 4.0,
            h: 1.0,
        };
        let mut c = black(4, 1);
        draw_image(
            &mut c,
            &src,
            r,
            1.0,
            &DrawOpts {
                u0: 0.5,
                ..DrawOpts::default()
            },
        );
        assert_eq!(
            c.get_pixel(0, 0).0,
            [0, 0, 255, 255],
            "só a metade direita (azul)"
        );
        let mut c = black(4, 1);
        draw_image(
            &mut c,
            &src,
            r,
            1.0,
            &DrawOpts {
                flip_h: true,
                ..DrawOpts::default()
            },
        );
        assert_eq!(
            c.get_pixel(0, 0).0,
            [0, 0, 255, 255],
            "espelhado: azul à esquerda"
        );
        assert_eq!(c.get_pixel(3, 0).0, [255, 0, 0, 255]);
    }
}
