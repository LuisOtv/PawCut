//! Seleção: uma máscara de 0 a 255 do tamanho da tela (bordas suaves são valores no meio).

use std::sync::Arc;

use crate::tile::TILE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelOp {
    Replace,
    Add,
    Subtract,
    Intersect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    w: usize,
    h: usize,
    mask: Arc<Vec<u8>>,
}

impl Selection {
    pub fn empty(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            mask: Arc::new(vec![0; w * h]),
        }
    }

    pub fn all(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            mask: Arc::new(vec![255; w * h]),
        }
    }

    pub fn from_mask(w: usize, h: usize, mask: Vec<u8>) -> Self {
        assert_eq!(mask.len(), w * h);
        Self {
            w,
            h,
            mask: Arc::new(mask),
        }
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    pub fn mask(&self) -> &[u8] {
        &self.mask
    }

    pub fn value(&self, x: usize, y: usize) -> u8 {
        if x < self.w && y < self.h {
            self.mask[y * self.w + x]
        } else {
            0
        }
    }

    pub fn is_empty(&self) -> bool {
        self.mask.iter().all(|v| *v == 0)
    }

    /// Menor retângulo (x0, y0, x1, y1) com algo selecionado.
    pub fn bounds(&self) -> Option<(usize, usize, usize, usize)> {
        let mut b: Option<(usize, usize, usize, usize)> = None;
        for y in 0..self.h {
            let row = &self.mask[y * self.w..(y + 1) * self.w];
            let Some(first) = row.iter().position(|v| *v != 0) else {
                continue;
            };
            let last = row.iter().rposition(|v| *v != 0).unwrap_or(first);
            b = Some(match b {
                None => (first, y, last + 1, y + 1),
                Some((a, bb, c, d)) => (a.min(first), bb.min(y), c.max(last + 1), d.max(y + 1)),
            });
        }
        b
    }

    /// Valores da máscara dentro do bloco (`tx`, `ty`), 64×64.
    pub fn tile_mask(&self, tx: usize, ty: usize) -> Vec<u8> {
        let mut out = vec![0u8; TILE * TILE];
        for y in 0..TILE {
            let py = ty * TILE + y;
            if py >= self.h {
                break;
            }
            for x in 0..TILE {
                let px = tx * TILE + x;
                if px >= self.w {
                    break;
                }
                out[y * TILE + x] = self.mask[py * self.w + px];
            }
        }
        out
    }

    pub fn rect(w: usize, h: usize, x0: i64, y0: i64, x1: i64, y1: i64) -> Self {
        let (xa, xb) = (
            x0.min(x1).max(0) as usize,
            (x0.max(x1)).clamp(0, w as i64) as usize,
        );
        let (ya, yb) = (
            y0.min(y1).max(0) as usize,
            (y0.max(y1)).clamp(0, h as i64) as usize,
        );
        let mut m = vec![0u8; w * h];
        for y in ya..yb {
            m[y * w + xa..y * w + xb].fill(255);
        }
        Self::from_mask(w, h, m)
    }

    /// Elipse com borda suave inscrita no retângulo (x0, y0)–(x1, y1).
    pub fn ellipse(w: usize, h: usize, x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (rx, ry) = (
            ((x1 - x0) / 2.0).abs().max(0.5),
            ((y1 - y0) / 2.0).abs().max(0.5),
        );
        let mut m = vec![0u8; w * h];
        let (ya, yb) = (
            (cy - ry - 1.0).floor().max(0.0) as usize,
            ((cy + ry + 1.0).ceil() as usize).min(h),
        );
        let (xa, xb) = (
            (cx - rx - 1.0).floor().max(0.0) as usize,
            ((cx + rx + 1.0).ceil() as usize).min(w),
        );
        for y in ya..yb {
            for x in xa..xb {
                let (dx, dy) = ((x as f32 + 0.5 - cx) / rx, (y as f32 + 0.5 - cy) / ry);
                let d = (dx * dx + dy * dy).sqrt();
                // distância aproximada até a borda, em pixels
                let edge = (1.0 - d) * rx.min(ry);
                m[y * w + x] = ((edge + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
        Self::from_mask(w, h, m)
    }

    /// Polígono (laço) com borda suave: preenche por linhas, 4 amostras por pixel na vertical.
    pub fn polygon(w: usize, h: usize, pts: &[(f32, f32)]) -> Self {
        let mut acc = vec![0.0f32; w * h];
        if pts.len() >= 3 {
            let (ymin, ymax) = pts
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
            let (ya, yb) = (
                ymin.floor().max(0.0) as usize,
                (ymax.ceil() as usize).min(h),
            );
            for y in ya..yb {
                for sub in 0..4 {
                    let sy = y as f32 + (sub as f32 + 0.5) / 4.0;
                    let mut xs: Vec<f32> = Vec::new();
                    for i in 0..pts.len() {
                        let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                        if (p.1 <= sy) != (q.1 <= sy) {
                            xs.push(p.0 + (sy - p.1) / (q.1 - p.1) * (q.0 - p.0));
                        }
                    }
                    xs.sort_by(|a, b| a.total_cmp(b));
                    for pair in xs.chunks(2) {
                        let [a, b] = pair else { continue };
                        let (a, b) = (a.clamp(0.0, w as f32), b.clamp(0.0, w as f32));
                        if b <= a {
                            continue;
                        }
                        let (xa, xb) = (a.floor() as usize, (b.ceil() as usize).min(w));
                        for x in xa..xb {
                            let cover = (b.min(x as f32 + 1.0) - a.max(x as f32)).clamp(0.0, 1.0);
                            acc[y * w + x] += cover * 0.25;
                        }
                    }
                }
            }
        }
        Self::from_mask(
            w,
            h,
            acc.iter()
                .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect(),
        )
    }

    /// Junta `other` a esta seleção.
    pub fn combine(&self, other: &Selection, op: SelOp) -> Selection {
        let m: Vec<u8> = self
            .mask
            .iter()
            .zip(other.mask.iter())
            .map(|(a, b)| match op {
                SelOp::Replace => *b,
                SelOp::Add => (*a).max(*b),
                SelOp::Subtract => ((*a as u32 * (255 - *b as u32) + 127) / 255) as u8,
                SelOp::Intersect => ((*a as u32 * *b as u32 + 127) / 255) as u8,
            })
            .collect();
        Selection::from_mask(self.w, self.h, m)
    }

    pub fn invert(&self) -> Selection {
        Selection::from_mask(self.w, self.h, self.mask.iter().map(|v| 255 - v).collect())
    }

    /// Suaviza a borda (desfoque de caixa em três passadas).
    pub fn feather(&self, radius: usize) -> Selection {
        if radius == 0 {
            return self.clone();
        }
        let mut a: Vec<f32> = self.mask.iter().map(|v| *v as f32).collect();
        let mut tmp = vec![0.0f32; a.len()];
        let (w, h) = (self.w, self.h);
        for _ in 0..3 {
            for y in 0..h {
                let row = &a[y * w..(y + 1) * w];
                let mut sum: f32 = row[..radius.min(w - 1) + 1].iter().sum();
                for x in 0..w {
                    tmp[y * w + x] = sum / (2 * radius + 1) as f32;
                    if x + radius + 1 < w {
                        sum += row[x + radius + 1];
                    }
                    if x >= radius {
                        sum -= row[x - radius];
                    }
                }
            }
            for x in 0..w {
                let mut sum = 0.0;
                for y in 0..=radius.min(h - 1) {
                    sum += tmp[y * w + x];
                }
                for y in 0..h {
                    a[y * w + x] = sum / (2 * radius + 1) as f32;
                    if y + radius + 1 < h {
                        sum += tmp[(y + radius + 1) * w + x];
                    }
                    if y >= radius {
                        sum -= tmp[(y - radius) * w + x];
                    }
                }
            }
        }
        Selection::from_mask(
            w,
            h,
            a.iter()
                .map(|v| v.round().clamp(0.0, 255.0) as u8)
                .collect(),
        )
    }

    /// Cresce (`px` > 0) ou encolhe (`px` < 0) a seleção, em quadrado.
    pub fn grow(&self, px: i32) -> Selection {
        let r = px.unsigned_abs() as usize;
        if r == 0 {
            return self.clone();
        }
        let grow = px > 0;
        let (w, h) = (self.w, self.h);
        let pass = |src: &[u8], horizontal: bool| -> Vec<u8> {
            let mut out = src.to_vec();
            for y in 0..h {
                for x in 0..w {
                    let mut v = src[y * w + x];
                    for d in 1..=r {
                        let (a, b) = if horizontal {
                            (
                                x.checked_sub(d).map(|nx| (nx, y)),
                                (x + d < w).then_some((x + d, y)),
                            )
                        } else {
                            (
                                y.checked_sub(d).map(|ny| (x, ny)),
                                (y + d < h).then_some((x, y + d)),
                            )
                        };
                        for (nx, ny) in [a, b].into_iter().flatten() {
                            let o = src[ny * w + nx];
                            v = if grow { v.max(o) } else { v.min(o) };
                        }
                        // fora da tela conta como vazio ao encolher
                        if !grow && (a.is_none() || b.is_none()) {
                            v = 0;
                        }
                    }
                    out[y * w + x] = v;
                }
            }
            out
        };
        let m = pass(&pass(&self.mask, true), false);
        Selection::from_mask(w, h, m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_bounds_and_ops() {
        let a = Selection::rect(100, 100, 10, 10, 50, 50);
        assert_eq!(a.bounds(), Some((10, 10, 50, 50)));
        assert_eq!(a.value(20, 20), 255);
        assert_eq!(a.value(5, 5), 0);
        let b = Selection::rect(100, 100, 30, 30, 70, 70);
        assert_eq!(a.combine(&b, SelOp::Add).bounds(), Some((10, 10, 70, 70)));
        assert_eq!(
            a.combine(&b, SelOp::Intersect).bounds(),
            Some((30, 30, 50, 50))
        );
        let sub = a.combine(&b, SelOp::Subtract);
        assert_eq!(sub.value(40, 40), 0);
        assert_eq!(sub.value(15, 15), 255);
        assert_eq!(a.invert().value(20, 20), 0);
        assert_eq!(a.invert().value(0, 0), 255);
        assert!(Selection::empty(10, 10).is_empty());
    }

    #[test]
    fn ellipse_and_polygon_cover_the_right_area() {
        let e = Selection::ellipse(100, 100, 10.0, 10.0, 90.0, 90.0);
        assert_eq!(e.value(50, 50), 255);
        assert_eq!(e.value(12, 12), 0);
        assert!(e.value(50, 11) > 0);
        let tri = Selection::polygon(100, 100, &[(10.0, 10.0), (90.0, 10.0), (10.0, 90.0)]);
        assert_eq!(tri.value(20, 20), 255);
        assert_eq!(tri.value(80, 80), 0);
        let area: u32 = tri.mask().iter().map(|v| *v as u32).sum::<u32>() / 255;
        assert!((area as i32 - 3200).abs() < 120, "{area}");
    }

    #[test]
    fn feather_and_grow() {
        let a = Selection::rect(60, 60, 20, 20, 40, 40);
        let f = a.feather(4);
        assert!(f.value(20, 30) > 0 && f.value(20, 30) < 255);
        assert!(f.value(30, 30) > 200);
        let g = a.grow(3);
        assert_eq!(g.bounds(), Some((17, 17, 43, 43)));
        let s = a.grow(-3);
        assert_eq!(s.bounds(), Some((23, 23, 37, 37)));
    }
}
