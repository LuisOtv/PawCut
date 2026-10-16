//! Mover, girar, escalar e espelhar camadas (amostragem bilinear em alfa pré-multiplicado).

use crate::tile::TiledImage;

/// `x' = a·x + c·y + tx`, `y' = b·x + d·y + ty`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Affine {
    pub fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// Escala (`sx`, `sy`) e gira `deg` graus (sentido horário) em volta de `center`, depois
    /// soma (`dx`, `dy`).
    pub fn around(center: (f32, f32), sx: f32, sy: f32, deg: f32, dx: f32, dy: f32) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        let (a, b, cc, d) = (c * sx, s * sx, -s * sy, c * sy);
        Self {
            a,
            b,
            c: cc,
            d,
            tx: center.0 - (a * center.0 + cc * center.1) + dx,
            ty: center.1 - (b * center.0 + d * center.1) + dy,
        }
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }

    pub fn inverse(&self) -> Option<Affine> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-9 {
            return None;
        }
        let (ia, ib, ic, id) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Affine {
            a: ia,
            b: ib,
            c: ic,
            d: id,
            tx: -(ia * self.tx + ic * self.ty),
            ty: -(ib * self.tx + id * self.ty),
        })
    }
}

fn sample(src: &TiledImage, x: f32, y: f32) -> [f32; 4] {
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
        let (px, py) = (x0 + dx, y0 + dy);
        if px < 0 || py < 0 || w <= 0.0 {
            continue;
        }
        let p = src.pixel(px as usize, py as usize);
        for i in 0..4 {
            acc[i] += p[i] as f32 * w;
        }
    }
    acc
}

/// Desenha `src` transformada por `m` numa imagem nova de `w`×`h` (o resto fica transparente).
pub fn transform(src: &TiledImage, m: &Affine, w: usize, h: usize) -> TiledImage {
    let mut out = TiledImage::new(w, h);
    let (Some(inv), Some((x0, y0, x1, y1))) = (m.inverse(), src.content_bounds()) else {
        return out;
    };
    // Caixa do resultado: os cantos do conteúdo transformados.
    let corners = [
        m.apply(x0 as f32, y0 as f32),
        m.apply(x1 as f32, y0 as f32),
        m.apply(x0 as f32, y1 as f32),
        m.apply(x1 as f32, y1 as f32),
    ];
    let (mut bx0, mut by0, mut bx1, mut by1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in corners {
        bx0 = bx0.min(x);
        by0 = by0.min(y);
        bx1 = bx1.max(x);
        by1 = by1.max(y);
    }
    let (xa, xb) = (
        (bx0.floor() - 1.0).max(0.0) as usize,
        ((bx1.ceil() + 1.0) as usize).min(w),
    );
    let (ya, yb) = (
        (by0.floor() - 1.0).max(0.0) as usize,
        ((by1.ceil() + 1.0) as usize).min(h),
    );
    for y in ya..yb {
        for x in xa..xb {
            let (sx, sy) = inv.apply(x as f32 + 0.5, y as f32 + 0.5);
            let p = sample(src, sx - 0.5, sy - 0.5);
            if p[3] <= 0.0 {
                continue;
            }
            out.set_pixel(
                x,
                y,
                [
                    p[0].round().min(255.0) as u8,
                    p[1].round().min(255.0) as u8,
                    p[2].round().min(255.0) as u8,
                    p[3].round().min(255.0) as u8,
                ],
            );
        }
    }
    out
}

pub fn flip_horizontal(src: &TiledImage) -> TiledImage {
    let (w, h) = (src.width(), src.height());
    let mut out = TiledImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = src.pixel(x, y);
            if p[3] != 0 {
                out.set_pixel(w - 1 - x, y, p);
            }
        }
    }
    out
}

pub fn flip_vertical(src: &TiledImage) -> TiledImage {
    let (w, h) = (src.width(), src.height());
    let mut out = TiledImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = src.pixel(x, y);
            if p[3] != 0 {
                out.set_pixel(x, h - 1 - y, p);
            }
        }
    }
    out
}

/// Gira 90° (a imagem nova tem largura e altura trocadas).
pub fn rotate90(src: &TiledImage, clockwise: bool) -> TiledImage {
    let (w, h) = (src.width(), src.height());
    let mut out = TiledImage::new(h, w);
    for y in 0..h {
        for x in 0..w {
            let p = src.pixel(x, y);
            if p[3] == 0 {
                continue;
            }
            let (nx, ny) = if clockwise {
                (h - 1 - y, x)
            } else {
                (y, w - 1 - x)
            };
            out.set_pixel(nx, ny, p);
        }
    }
    out
}

/// Muda o tamanho da imagem (filtro Lanczos, bom para reduzir e ampliar).
pub fn scale_image(src: &TiledImage, new_w: usize, new_h: usize) -> TiledImage {
    let rgba = src.to_rgba();
    // Alfa direto: as bordas transparentes não escurecem porque a cor de pixels sem alfa é preta
    // só quando não há nada; aceitável para imagens do dia a dia.
    let scaled = image::imageops::resize(
        &rgba,
        new_w as u32,
        new_h as u32,
        image::imageops::FilterType::Lanczos3,
    );
    TiledImage::from_rgba(&scaled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dot() -> TiledImage {
        let mut t = TiledImage::new(100, 100);
        for y in 10..20 {
            for x in 10..20 {
                t.set_pixel(x, y, [255, 0, 0, 255]);
            }
        }
        t
    }

    #[test]
    fn translation_and_scale() {
        let moved = transform(
            &dot(),
            &Affine::around((0.0, 0.0), 1.0, 1.0, 0.0, 30.0, 5.0),
            100,
            100,
        );
        assert_eq!(moved.content_bounds(), Some((40, 15, 50, 25)));
        let big = transform(
            &dot(),
            &Affine::around((15.0, 15.0), 2.0, 2.0, 0.0, 0.0, 0.0),
            100,
            100,
        );
        let (x0, y0, x1, y1) = big.content_bounds().unwrap();
        assert!(
            (19..=23).contains(&(x1 - x0)) && x0 >= 3 && y0 >= 3 && y1 <= 27,
            "{x0} {y0} {x1} {y1}"
        );
    }

    #[test]
    fn rotating_90_degrees_around_the_center() {
        let mut t = TiledImage::new(100, 100);
        for x in 40..60 {
            t.set_pixel(x, 50, [0, 255, 0, 255]);
            t.set_pixel(x, 51, [0, 255, 0, 255]);
        }
        let r = transform(
            &t,
            &Affine::around((50.0, 50.5), 1.0, 1.0, 90.0, 0.0, 0.0),
            100,
            100,
        );
        let (x0, y0, x1, y1) = r.content_bounds().unwrap();
        assert!(y1 - y0 >= 19 && x1 - x0 <= 4, "{x0} {y0} {x1} {y1}");
    }

    #[test]
    fn flips_and_quarter_turns_keep_pixels() {
        let mut t = TiledImage::new(30, 20);
        t.set_pixel(1, 2, [9, 9, 9, 255]);
        assert_eq!(flip_horizontal(&t).pixel(28, 2), [9, 9, 9, 255]);
        assert_eq!(flip_vertical(&t).pixel(1, 17), [9, 9, 9, 255]);
        let cw = rotate90(&t, true);
        assert_eq!((cw.width(), cw.height()), (20, 30));
        assert_eq!(cw.pixel(17, 1), [9, 9, 9, 255]);
        let ccw = rotate90(&t, false);
        assert_eq!(ccw.pixel(2, 28), [9, 9, 9, 255]);
        let s = scale_image(&t, 60, 40);
        assert_eq!((s.width(), s.height()), (60, 40));
    }

    #[test]
    fn inverse_round_trips() {
        let m = Affine::around((20.0, 30.0), 1.5, 0.5, 33.0, 4.0, -7.0);
        let inv = m.inverse().unwrap();
        let (x, y) = m.apply(11.0, 22.0);
        let (bx, by) = inv.apply(x, y);
        assert!((bx - 11.0).abs() < 1e-3 && (by - 22.0).abs() < 1e-3);
    }
}
