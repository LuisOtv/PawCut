//! Ajustes de cor, contorno de adesivo e sombra.

use image::{Rgba as Px, RgbaImage};
use pawpaint_core::{Adjust, Layer};

/// Espaço extra em volta da camada para o contorno e a sombra caberem.
pub(crate) fn padding(layer: &Layer) -> f32 {
    let outline = layer.outline.map_or(0.0, |o| o.width.max(0.0));
    let shadow = layer
        .shadow
        .map_or(0.0, |s| s.dx.abs().max(s.dy.abs()) + s.blur.max(0.0) * 2.0);
    (outline + shadow).ceil() + 2.0
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-4)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Brilho, contraste, saturação e remoção de cor (em ordem).
pub(crate) fn adjust_pixels(img: &mut RgbaImage, adj: &Adjust) {
    if adj.is_neutral() {
        return;
    }
    let contrast = adj.contrast.clamp(-0.95, 0.95);
    let factor = (1.0 + contrast) / (1.0 - contrast);
    for p in img.pixels_mut() {
        let mut rgb = [
            p.0[0] as f32 / 255.0,
            p.0[1] as f32 / 255.0,
            p.0[2] as f32 / 255.0,
        ];
        let mut alpha = p.0[3] as f32 / 255.0;
        if let Some(ch) = adj.chroma {
            let key = [
                ch.color.r as f32 / 255.0,
                ch.color.g as f32 / 255.0,
                ch.color.b as f32 / 255.0,
            ];
            let d =
                ((rgb[0] - key[0]).powi(2) + (rgb[1] - key[1]).powi(2) + (rgb[2] - key[2]).powi(2))
                    .sqrt()
                    / 3f32.sqrt();
            let t = ch.tolerance.clamp(0.0, 1.0) * 0.5;
            alpha *= smoothstep(t, t + ch.softness.clamp(0.0, 1.0) * 0.5 + 1e-3, d);
        }
        let lum = 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2];
        for c in rgb.iter_mut() {
            *c = lum + (*c - lum) * (1.0 + adj.saturation.clamp(-1.0, 1.0));
            *c = (*c - 0.5) * factor + 0.5 + adj.brightness.clamp(-1.0, 1.0) * 0.5;
        }
        p.0 = [
            (rgb[0] * 255.0).round().clamp(0.0, 255.0) as u8,
            (rgb[1] * 255.0).round().clamp(0.0, 255.0) as u8,
            (rgb[2] * 255.0).round().clamp(0.0, 255.0) as u8,
            (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
        ];
    }
}

/// Distância (em pixels) de cada ponto até o ponto "dentro" mais perto (0 dentro).
fn distance_to_inside(inside: &[bool], w: usize, h: usize) -> Vec<f32> {
    const INF: f32 = 1e12;
    let mut d: Vec<f32> = inside.iter().map(|&i| if i { 0.0 } else { INF }).collect();
    // Transformada de distância euclidiana (Felzenszwalb), uma dimensão por vez.
    let edt_1d = |f: &[f32], out: &mut [f32]| {
        let n = f.len();
        let mut v = vec![0usize; n];
        let mut z = vec![0.0f32; n + 1];
        let mut k = 0usize;
        z[0] = -INF;
        z[1] = INF;
        for q in 1..n {
            loop {
                let p = v[k];
                let s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32))
                    / (2.0 * q as f32 - 2.0 * p as f32);
                if s <= z[k] && k > 0 {
                    k -= 1;
                } else {
                    k += 1;
                    v[k] = q;
                    z[k] = s;
                    z[k + 1] = INF;
                    break;
                }
                if k == 0 && s <= z[0] {
                    v[0] = q;
                    z[1] = INF;
                    break;
                }
            }
        }
        k = 0;
        for (q, o) in out.iter_mut().enumerate() {
            while z[k + 1] < q as f32 {
                k += 1;
            }
            let p = v[k];
            *o = (q as f32 - p as f32).powi(2) + f[p];
        }
    };
    let mut col = vec![0.0f32; h];
    let mut res = vec![0.0f32; h.max(w)];
    for x in 0..w {
        for y in 0..h {
            col[y] = d[y * w + x];
        }
        edt_1d(&col, &mut res[..h]);
        for y in 0..h {
            d[y * w + x] = res[y];
        }
    }
    let mut row = vec![0.0f32; w];
    for y in 0..h {
        row.copy_from_slice(&d[y * w..(y + 1) * w]);
        edt_1d(&row, &mut res[..w]);
        d[y * w..(y + 1) * w].copy_from_slice(&res[..w]);
    }
    d.iter().map(|v| v.sqrt()).collect()
}

/// Desfoque por três passadas de média (parece gaussiano), só no alfa.
fn blur(a: &mut [f32], w: usize, h: usize, radius: f32) {
    let r = radius.round() as usize;
    if r == 0 {
        return;
    }
    let mut tmp = vec![0.0f32; a.len()];
    for _ in 0..3 {
        // horizontal
        for y in 0..h {
            let row = &a[y * w..(y + 1) * w];
            let mut sum: f32 = row[..r.min(w - 1) + 1].iter().sum();
            for x in 0..w {
                tmp[y * w + x] = sum / (2 * r + 1) as f32;
                let add = x + r + 1;
                if add < w {
                    sum += row[add];
                }
                if x >= r {
                    sum -= row[x - r];
                }
            }
        }
        // vertical
        for x in 0..w {
            let mut sum = 0.0;
            for y in 0..=r.min(h - 1) {
                sum += tmp[y * w + x];
            }
            for y in 0..h {
                a[y * w + x] = sum / (2 * r + 1) as f32;
                let add = y + r + 1;
                if add < h {
                    sum += tmp[add * w + x];
                }
                if y >= r {
                    sum -= tmp[(y - r) * w + x];
                }
            }
        }
    }
}

/// Soma de `color` (com `cov` de cobertura) por baixo do que já está em `px`.
fn under(px: &mut Px<u8>, color: [u8; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    let da = px.0[3] as f32 / 255.0;
    // `px` está por cima de `color`.
    let out_a = da + a * (1.0 - da);
    for (dst, src) in px.0.iter_mut().zip(color) {
        let v = (*dst as f32 * da + src as f32 * a * (1.0 - da)) / out_a;
        *dst = v.round().clamp(0.0, 255.0) as u8;
    }
    px.0[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Contorno e sombra por baixo do desenho da camada.
pub(crate) fn apply_effects(mut img: RgbaImage, layer: &Layer) -> RgbaImage {
    if layer.outline.is_none() && layer.shadow.is_none() {
        return img;
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    // Silhueta (alfa) do desenho, que vira a do desenho + contorno.
    let mut sil: Vec<f32> = img.pixels().map(|p| p.0[3] as f32 / 255.0).collect();
    let mut ring: Option<Vec<f32>> = None;
    if let Some(o) = layer.outline.filter(|o| o.width > 0.0) {
        let inside: Vec<bool> = sil.iter().map(|a| *a >= 0.5).collect();
        let dist = distance_to_inside(&inside, w, h);
        let cov: Vec<f32> = dist
            .iter()
            .map(|d| (o.width + 0.5 - d).clamp(0.0, 1.0) * o.color.a as f32 / 255.0)
            .collect();
        for (s, c) in sil.iter_mut().zip(&cov) {
            *s = s.max(*c);
        }
        ring = Some(cov);
    }
    // Por cima de tudo vem o desenho; embaixo dele o contorno; embaixo de ambos a sombra.
    if let (Some(cov), Some(o)) = (&ring, layer.outline) {
        for (i, p) in img.pixels_mut().enumerate() {
            under(p, [o.color.r, o.color.g, o.color.b], cov[i]);
        }
    }
    if let Some(s) = layer.shadow {
        let mut a = vec![0.0f32; w * h];
        let (dx, dy) = (s.dx.round() as i64, s.dy.round() as i64);
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let (sx, sy) = (x - dx, y - dy);
                if sx >= 0 && sy >= 0 && sx < w as i64 && sy < h as i64 {
                    a[y as usize * w + x as usize] = sil[sy as usize * w + sx as usize];
                }
            }
        }
        blur(&mut a, w, h, s.blur);
        for (i, p) in img.pixels_mut().enumerate() {
            under(
                p,
                [s.color.r, s.color.g, s.color.b],
                a[i] * s.color.a as f32 / 255.0,
            );
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_transform_is_euclidean() {
        let (w, h) = (9, 9);
        let mut inside = vec![false; w * h];
        inside[4 * w + 4] = true;
        let d = distance_to_inside(&inside, w, h);
        assert_eq!(d[4 * w + 4], 0.0);
        assert!((d[4 * w + 7] - 3.0).abs() < 1e-4);
        assert!((d[7 * w + 7] - (18.0f32).sqrt()).abs() < 1e-3);
    }

    #[test]
    fn blur_keeps_total_energy_away_from_edges() {
        let (w, h) = (31, 31);
        let mut a = vec![0.0f32; w * h];
        a[15 * w + 15] = 1.0;
        blur(&mut a, w, h, 3.0);
        let total: f32 = a.iter().sum();
        assert!((total - 1.0).abs() < 1e-3, "{total}");
        assert!(a[15 * w + 15] < 0.2);
    }
}
