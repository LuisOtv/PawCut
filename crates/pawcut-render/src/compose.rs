//! Operações de pixel no quadro de saída (sempre opaco).

use image::RgbaImage;
use pawcut_core::Rgba;
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

pub(crate) fn fill_rect(canvas: &mut RgbaImage, r: Rect, color: Rgba, opacity: f32) {
    let a = color.a as f32 / 255.0 * opacity;
    let (w, h) = canvas.dimensions();
    let ((x0, x1), (y0, y1)) = (span(r.x0, r.w, w), span(r.y0, r.h, h));
    if a <= 0.0 || x0 == x1 || y0 == y1 {
        return;
    }
    let src = [color.r as f32, color.g as f32, color.b as f32];
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

/// Desenha `src` esticada em `r`, respeitando o alfa da imagem e a `opacity`.
pub(crate) fn draw_image(canvas: &mut RgbaImage, src: &RgbaImage, r: Rect, opacity: f32) {
    let (w, h) = canvas.dimensions();
    let ((x0, x1), (y0, y1)) = (span(r.x0, r.w, w), span(r.y0, r.h, h));
    if opacity <= 0.0 || x0 == x1 || y0 == y1 || src.width() == 0 || src.height() == 0 {
        return;
    }
    let (sx, sy) = (src.width() as f32 / r.w, src.height() as f32 / r.h);
    let buf: &mut [u8] = &mut *canvas;
    buf.par_chunks_mut(w as usize * 4)
        .enumerate()
        .filter(|(y, _)| (y0..y1).contains(y))
        .for_each(|(y, row)| {
            let v = (y as f32 + 0.5 - r.y0) * sy - 0.5;
            for x in x0..x1 {
                let u = (x as f32 + 0.5 - r.x0) * sx - 0.5;
                let p = sample(src, u, v);
                blend(
                    &mut row[x * 4..x * 4 + 4],
                    [p[0], p[1], p[2]],
                    p[3] / 255.0 * opacity,
                );
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
        );
        assert_eq!(c.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(c.get_pixel(7, 1).0, [0, 0, 0, 255]);
    }
}
