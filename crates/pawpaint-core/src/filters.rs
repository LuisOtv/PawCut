//! Filtros de imagem aplicados numa camada (dentro da seleção, se houver).

use crate::selection::Selection;
use crate::tile::{TiledImage, premultiply, unpremultiply};

#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    /// Ambos de −1 a 1.
    BrightnessContrast {
        brightness: f32,
        contrast: f32,
    },
    /// Matiz em graus (−180 a 180); saturação e luminosidade de −1 a 1.
    HueSaturation {
        hue: f32,
        saturation: f32,
        lightness: f32,
    },
    Invert,
    Desaturate,
    /// Limiar de 0 a 1: abaixo vira preto, acima vira branco.
    Threshold(f32),
    Posterize(u32),
    /// Pretos, brancos (0 a 1) e gama (0,1 a 4).
    Levels {
        black: f32,
        white: f32,
        gamma: f32,
    },
    GaussianBlur(f32),
    /// Máscara de nitidez: quantidade (0 a 3) e raio.
    Sharpen {
        amount: f32,
        radius: f32,
    },
    Pixelate(u32),
    Noise(f32),
    EdgeDetect,
    /// Curva de tons: pontos (entrada, saída) de 0 a 1, em qualquer ordem.
    Curves(Vec<(f32, f32)>),
}

/// Tabela de 256 valores da curva por interpolação cúbica monotônica (não faz ondas).
pub fn curve_lut(points: &[(f32, f32)]) -> [f32; 256] {
    let mut p: Vec<(f32, f32)> = points
        .iter()
        .map(|(x, y)| (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
        .collect();
    p.sort_by(|a, b| a.0.total_cmp(&b.0));
    p.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-4);
    if p.first().is_none_or(|f| f.0 > 0.0) {
        p.insert(0, (0.0, 0.0));
    }
    if p.last().is_none_or(|l| l.0 < 1.0) {
        p.push((1.0, 1.0));
    }
    let n = p.len();
    let secants: Vec<f32> = (0..n - 1)
        .map(|i| (p[i + 1].1 - p[i].1) / (p[i + 1].0 - p[i].0).max(1e-6))
        .collect();
    let mut m = vec![0.0f32; n];
    m[0] = secants[0];
    m[n - 1] = secants[n - 2];
    for i in 1..n - 1 {
        m[i] = if secants[i - 1] * secants[i] <= 0.0 {
            0.0
        } else {
            (secants[i - 1] + secants[i]) / 2.0
        };
    }
    for i in 0..n - 1 {
        if secants[i] == 0.0 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
        } else {
            let (a, b) = (m[i] / secants[i], m[i + 1] / secants[i]);
            let h = (a * a + b * b).sqrt();
            if h > 3.0 {
                m[i] = 3.0 * a / h * secants[i];
                m[i + 1] = 3.0 * b / h * secants[i];
            }
        }
    }
    let mut lut = [0.0f32; 256];
    for (k, out) in lut.iter_mut().enumerate() {
        let x = k as f32 / 255.0;
        let i = p.partition_point(|q| q.0 <= x).saturating_sub(1).min(n - 2);
        let (x0, x1) = (p[i].0, p[i + 1].0);
        let (h, t) = (x1 - x0, ((x - x0) / (x1 - x0).max(1e-6)).clamp(0.0, 1.0));
        let (t2, t3) = (t * t, t * t * t);
        let v = (2.0 * t3 - 3.0 * t2 + 1.0) * p[i].1
            + (t3 - 2.0 * t2 + t) * h * m[i]
            + (-2.0 * t3 + 3.0 * t2) * p[i + 1].1
            + (t3 - t2) * h * m[i + 1];
        *out = v.clamp(0.0, 1.0);
    }
    lut
}

impl Filter {
    /// Quanto o filtro espalha pixels para fora do conteúdo (para ampliar a área processada).
    fn reach(&self) -> usize {
        match self {
            Filter::GaussianBlur(r) => (r * 3.0).ceil() as usize + 1,
            Filter::Sharpen { radius, .. } => (radius * 3.0).ceil() as usize + 1,
            Filter::EdgeDetect => 1,
            _ => 0,
        }
    }
}

type Buf = Vec<[f32; 4]>;

fn to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    if mx == mn {
        return (0.0, 0.0, l);
    }
    let d = mx - mn;
    let s = if l > 0.5 {
        d / (2.0 - mx - mn)
    } else {
        d / (mx + mn)
    };
    let h = if mx == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let hh = h / 360.0;
    (f(hh + 1.0 / 3.0), f(hh), f(hh - 1.0 / 3.0))
}

/// Desfoque de caixa em três passadas por canal (parece gaussiano), em alfa pré-multiplicado.
fn blur(buf: &mut Buf, w: usize, h: usize, sigma: f32) {
    let r = (sigma * 1.2).round().max(1.0) as usize;
    let mut premult: Buf = buf
        .iter()
        .map(|p| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]])
        .collect();
    let mut tmp = premult.clone();
    for _ in 0..3 {
        for y in 0..h {
            for c in 0..4 {
                let mut sum = 0.0;
                for x in 0..=r.min(w - 1) {
                    sum += premult[y * w + x][c];
                }
                for x in 0..w {
                    tmp[y * w + x][c] = sum / (2 * r + 1) as f32;
                    if x + r + 1 < w {
                        sum += premult[y * w + x + r + 1][c];
                    }
                    if x >= r {
                        sum -= premult[y * w + x - r][c];
                    }
                }
            }
        }
        for x in 0..w {
            for c in 0..4 {
                let mut sum = 0.0;
                for y in 0..=r.min(h - 1) {
                    sum += tmp[y * w + x][c];
                }
                for y in 0..h {
                    premult[y * w + x][c] = sum / (2 * r + 1) as f32;
                    if y + r + 1 < h {
                        sum += tmp[(y + r + 1) * w + x][c];
                    }
                    if y >= r {
                        sum -= tmp[(y - r) * w + x][c];
                    }
                }
            }
        }
    }
    for (o, p) in buf.iter_mut().zip(&premult) {
        let a = p[3];
        *o = if a > 1e-4 {
            [p[0] / a, p[1] / a, p[2] / a, a]
        } else {
            [0.0; 4]
        };
    }
}

fn run(f: &Filter, buf: &mut Buf, w: usize, h: usize) {
    match f {
        Filter::BrightnessContrast {
            brightness,
            contrast,
        } => {
            let c = contrast.clamp(-0.99, 0.99);
            let k = (1.0 + c) / (1.0 - c);
            for p in buf.iter_mut() {
                for v in &mut p[..3] {
                    *v = ((*v - 0.5) * k + 0.5 + brightness * 0.5).clamp(0.0, 1.0);
                }
            }
        }
        Filter::HueSaturation {
            hue,
            saturation,
            lightness,
        } => {
            for p in buf.iter_mut() {
                let (hh, s, l) = to_hsl(p[0], p[1], p[2]);
                let s2 = if *saturation >= 0.0 {
                    s + (1.0 - s) * saturation
                } else {
                    s * (1.0 + saturation)
                };
                let l2 = if *lightness >= 0.0 {
                    l + (1.0 - l) * lightness
                } else {
                    l * (1.0 + lightness)
                };
                let (r, g, b) = from_hsl(
                    (hh + hue).rem_euclid(360.0),
                    s2.clamp(0.0, 1.0),
                    l2.clamp(0.0, 1.0),
                );
                p[..3].copy_from_slice(&[r, g, b]);
            }
        }
        Filter::Invert => {
            for p in buf.iter_mut() {
                for v in &mut p[..3] {
                    *v = 1.0 - *v;
                }
            }
        }
        Filter::Desaturate => {
            for p in buf.iter_mut() {
                let l = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
                p[..3].copy_from_slice(&[l, l, l]);
            }
        }
        Filter::Threshold(t) => {
            for p in buf.iter_mut() {
                let l = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
                let v = if l >= *t { 1.0 } else { 0.0 };
                p[..3].copy_from_slice(&[v, v, v]);
            }
        }
        Filter::Posterize(n) => {
            let n = (*n).clamp(2, 64) as f32 - 1.0;
            for p in buf.iter_mut() {
                for v in &mut p[..3] {
                    *v = (*v * n).round() / n;
                }
            }
        }
        Filter::Levels {
            black,
            white,
            gamma,
        } => {
            let span = (white - black).max(1e-3);
            for p in buf.iter_mut() {
                for v in &mut p[..3] {
                    *v = ((*v - black) / span)
                        .clamp(0.0, 1.0)
                        .powf(1.0 / gamma.clamp(0.1, 4.0));
                }
            }
        }
        Filter::Curves(points) => {
            let lut = curve_lut(points);
            for p in buf.iter_mut() {
                for v in &mut p[..3] {
                    *v = lut[(*v * 255.0).round().clamp(0.0, 255.0) as usize];
                }
            }
        }
        Filter::GaussianBlur(r) => blur(buf, w, h, r.max(0.1)),
        Filter::Sharpen { amount, radius } => {
            let mut soft = buf.clone();
            blur(&mut soft, w, h, radius.max(0.3));
            for (p, s) in buf.iter_mut().zip(&soft) {
                for c in 0..3 {
                    p[c] = (p[c] + (p[c] - s[c]) * amount).clamp(0.0, 1.0);
                }
            }
        }
        Filter::Pixelate(size) => {
            let s = (*size).max(2) as usize;
            for by in (0..h).step_by(s) {
                for bx in (0..w).step_by(s) {
                    let mut acc = [0.0f32; 4];
                    let mut n = 0.0;
                    let (ye, xe) = ((by + s).min(h), (bx + s).min(w));
                    for y in by..ye {
                        for x in bx..xe {
                            let p = buf[y * w + x];
                            acc[0] += p[0] * p[3];
                            acc[1] += p[1] * p[3];
                            acc[2] += p[2] * p[3];
                            acc[3] += p[3];
                            n += 1.0;
                        }
                    }
                    let a = acc[3];
                    let px = if a > 0.0 {
                        [acc[0] / a, acc[1] / a, acc[2] / a, a / n]
                    } else {
                        [0.0; 4]
                    };
                    for y in by..ye {
                        for x in bx..xe {
                            buf[y * w + x] = px;
                        }
                    }
                }
            }
        }
        Filter::Noise(amount) => {
            let mut seed = 12345u32;
            for p in buf.iter_mut() {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let n = ((seed >> 8) as f32 / 16_777_216.0 - 0.5) * amount;
                for v in &mut p[..3] {
                    *v = (*v + n).clamp(0.0, 1.0);
                }
            }
        }
        Filter::EdgeDetect => {
            let src = buf.clone();
            let lum = |x: usize, y: usize| {
                let p = src[y.min(h - 1) * w + x.min(w - 1)];
                (0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]) * p[3]
            };
            for y in 0..h {
                for x in 0..w {
                    let (xm, xp, ym, yp) = (x.saturating_sub(1), x + 1, y.saturating_sub(1), y + 1);
                    let gx = -lum(xm, ym) - 2.0 * lum(xm, y) - lum(xm, yp)
                        + lum(xp, ym)
                        + 2.0 * lum(xp, y)
                        + lum(xp, yp);
                    let gy = -lum(xm, ym) - 2.0 * lum(x, ym) - lum(xp, ym)
                        + lum(xm, yp)
                        + 2.0 * lum(x, yp)
                        + lum(xp, yp);
                    let g = (gx * gx + gy * gy).sqrt().clamp(0.0, 1.0);
                    buf[y * w + x] = [g, g, g, src[y * w + x][3].max(g)];
                }
            }
        }
    }
}

/// Aplica `filter` em `pixels`. Só muda o que a seleção deixa; com `alpha_lock` o alfa não muda.
pub fn apply(pixels: &mut TiledImage, filter: &Filter, sel: Option<&Selection>, alpha_lock: bool) {
    let (cw, ch) = (pixels.width(), pixels.height());
    let base = match sel {
        Some(s) => s.bounds(),
        None => pixels.content_bounds(),
    };
    let Some((x0, y0, x1, y1)) = base else { return };
    let reach = if alpha_lock { 0 } else { filter.reach() };
    let (x0, y0) = (x0.saturating_sub(reach), y0.saturating_sub(reach));
    let (x1, y1) = ((x1 + reach).min(cw), (y1 + reach).min(ch));
    let (w, h) = (x1 - x0, y1 - y0);
    if w == 0 || h == 0 {
        return;
    }
    let mut buf: Buf = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let p = unpremultiply(pixels.pixel(x0 + x, y0 + y));
            buf.push([
                p[0] as f32 / 255.0,
                p[1] as f32 / 255.0,
                p[2] as f32 / 255.0,
                p[3] as f32 / 255.0,
            ]);
        }
    }
    let original = buf.clone();
    run(filter, &mut buf, w, h);
    for y in 0..h {
        for x in 0..w {
            let m = sel.map_or(1.0, |s| s.value(x0 + x, y0 + y) as f32 / 255.0);
            let (o, n) = (original[y * w + x], buf[y * w + x]);
            if alpha_lock && o[3] == 0.0 {
                continue;
            }
            let mut out = [0.0f32; 4];
            for c in 0..4 {
                out[c] = o[c] + (n[c] - o[c]) * m;
            }
            if alpha_lock {
                out[3] = o[3];
            }
            let px = [
                (out[0] * 255.0).round().clamp(0.0, 255.0) as u8,
                (out[1] * 255.0).round().clamp(0.0, 255.0) as u8,
                (out[2] * 255.0).round().clamp(0.0, 255.0) as u8,
                (out[3] * 255.0).round().clamp(0.0, 255.0) as u8,
            ];
            pixels.set_pixel(x0 + x, y0 + y, premultiply(px));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(color: [u8; 4]) -> TiledImage {
        let mut t = TiledImage::new(40, 30);
        for y in 0..30 {
            for x in 0..40 {
                t.set_pixel(x, y, color);
            }
        }
        t
    }

    #[test]
    fn invert_desaturate_threshold() {
        let mut t = flat([200, 100, 50, 255]);
        apply(&mut t, &Filter::Invert, None, false);
        assert_eq!(t.pixel(5, 5), [55, 155, 205, 255]);
        let mut t = flat([200, 100, 50, 255]);
        apply(&mut t, &Filter::Desaturate, None, false);
        let p = t.pixel(5, 5);
        assert_eq!(p[0], p[1]);
        assert_eq!(p[1], p[2]);
        apply(&mut t, &Filter::Threshold(0.9), None, false);
        assert_eq!(t.pixel(5, 5), [0, 0, 0, 255]);
    }

    #[test]
    fn brightness_hue_and_levels() {
        let mut t = flat([100, 100, 100, 255]);
        apply(
            &mut t,
            &Filter::BrightnessContrast {
                brightness: 0.4,
                contrast: 0.0,
            },
            None,
            false,
        );
        assert!(t.pixel(1, 1)[0] > 150);
        let mut t = flat([255, 0, 0, 255]);
        apply(
            &mut t,
            &Filter::HueSaturation {
                hue: 120.0,
                saturation: 0.0,
                lightness: 0.0,
            },
            None,
            false,
        );
        let p = t.pixel(1, 1);
        assert!(p[1] > 240 && p[0] < 15 && p[2] < 15, "{p:?}");
        let mut t = flat([128, 128, 128, 255]);
        apply(
            &mut t,
            &Filter::Levels {
                black: 0.5,
                white: 1.0,
                gamma: 1.0,
            },
            None,
            false,
        );
        assert!(t.pixel(1, 1)[0] < 5);
    }

    #[test]
    fn curves_follow_the_points_and_stay_monotone() {
        let id = curve_lut(&[]);
        assert!((id[128] - 128.0 / 255.0).abs() < 0.01);
        let lut = curve_lut(&[(0.5, 0.75)]);
        assert!((lut[128] - 0.75).abs() < 0.02);
        assert!(lut.windows(2).all(|w| w[1] >= w[0] - 1e-6));
        assert_eq!((lut[0], lut[255]), (0.0, 1.0));
        let inv = curve_lut(&[(0.0, 1.0), (1.0, 0.0)]);
        assert!(inv[0] > 0.99 && inv[255] < 0.01);
        let mut t = flat([128, 128, 128, 255]);
        apply(&mut t, &Filter::Curves(vec![(0.5, 0.75)]), None, false);
        assert!((t.pixel(3, 3)[0] as i32 - 191).abs() <= 6);
    }

    #[test]
    fn blur_spreads_and_selection_limits_filters() {
        let mut t = TiledImage::new(60, 60);
        for y in 20..40 {
            for x in 20..40 {
                t.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        apply(&mut t, &Filter::GaussianBlur(3.0), None, false);
        let (inside, edge_out, far) = (t.pixel(30, 30)[3], t.pixel(18, 30)[3], t.pixel(2, 30)[3]);
        assert!(
            inside > 240 && edge_out > 0 && edge_out < inside && far == 0,
            "{inside} {edge_out} {far}"
        );
        assert!(t.pixel(20, 30)[3] < 255, "a borda de dentro devia amaciar");
        let mut t = flat([200, 100, 50, 255]);
        let sel = Selection::rect(40, 30, 0, 0, 20, 30);
        apply(&mut t, &Filter::Invert, Some(&sel), false);
        assert_eq!(t.pixel(5, 5), [55, 155, 205, 255]);
        assert_eq!(t.pixel(30, 5), [200, 100, 50, 255]);
    }

    #[test]
    fn alpha_lock_keeps_the_shape() {
        let mut t = TiledImage::new(20, 20);
        t.set_pixel(10, 10, [200, 100, 50, 255]);
        apply(&mut t, &Filter::GaussianBlur(3.0), None, true);
        assert_eq!(t.pixel(10, 10)[3], 255);
        assert_eq!(t.pixel(11, 10), [0; 4]);
    }

    #[test]
    fn pixelate_and_edges_run() {
        let mut t = flat([10, 20, 30, 255]);
        t.set_pixel(3, 3, [255, 255, 255, 255]);
        apply(&mut t, &Filter::Pixelate(8), None, false);
        assert_eq!(t.pixel(0, 0), t.pixel(7, 7));
        let mut t = TiledImage::new(30, 30);
        for y in 0..30 {
            for x in 15..30 {
                t.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        apply(&mut t, &Filter::EdgeDetect, None, false);
        assert!(t.pixel(15, 15)[0] > t.pixel(5, 15)[0]);
    }
}
