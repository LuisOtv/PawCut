//! Balde de tinta, varinha mágica e degradê.

use crate::blend::{BlendMode, blend_over};
use crate::selection::Selection;
use crate::tile::{TILE, TiledImage, premultiply, unpremultiply};

fn close(a: [u8; 4], b: [u8; 4], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

/// Máscara (0 ou 255) dos pixels parecidos com o da semente. Com `contiguous`, só os que se
/// ligam a ela; senão, a imagem toda. `px` devolve a cor (alfa direto) de cada pixel.
pub fn flood_mask(
    px: &dyn Fn(usize, usize) -> [u8; 4],
    w: usize,
    h: usize,
    seed: (usize, usize),
    tolerance: f32,
    contiguous: bool,
) -> Vec<u8> {
    let mut mask = vec![0u8; w * h];
    if seed.0 >= w || seed.1 >= h {
        return mask;
    }
    let target = px(seed.0, seed.1);
    let tol = (tolerance.clamp(0.0, 1.0) * 255.0).round() as u8;
    if !contiguous {
        for y in 0..h {
            for x in 0..w {
                if close(px(x, y), target, tol) {
                    mask[y * w + x] = 255;
                }
            }
        }
        return mask;
    }
    let mut stack = vec![seed];
    while let Some((x, y)) = stack.pop() {
        if mask[y * w + x] != 0 || !close(px(x, y), target, tol) {
            continue;
        }
        // Anda para os dois lados na linha e empilha as linhas de cima e de baixo.
        let (mut l, mut r) = (x, x);
        while l > 0 && mask[y * w + l - 1] == 0 && close(px(l - 1, y), target, tol) {
            l -= 1;
        }
        while r + 1 < w && mask[y * w + r + 1] == 0 && close(px(r + 1, y), target, tol) {
            r += 1;
        }
        for xx in l..=r {
            mask[y * w + xx] = 255;
            if y > 0 && mask[(y - 1) * w + xx] == 0 {
                stack.push((xx, y - 1));
            }
            if y + 1 < h && mask[(y + 1) * w + xx] == 0 {
                stack.push((xx, y + 1));
            }
        }
    }
    mask
}

pub fn magic_wand(
    px: &dyn Fn(usize, usize) -> [u8; 4],
    w: usize,
    h: usize,
    seed: (usize, usize),
    tolerance: f32,
    contiguous: bool,
) -> Selection {
    Selection::from_mask(w, h, flood_mask(px, w, h, seed, tolerance, contiguous))
}

/// Pinta `color` sobre `pixels` onde `mask` (e a seleção) deixam, com a opacidade dada.
/// Trabalha bloco a bloco, em paralelo, e nem toca nos blocos que a máscara não alcança.
pub fn fill_mask(
    pixels: &mut TiledImage,
    mask: &[u8],
    color: [u8; 3],
    opacity: f32,
    sel: Option<&Selection>,
    alpha_lock: bool,
) {
    let (w, h) = (pixels.width(), pixels.height());
    pixels.par_tiles_mut(|tx, ty, mut tile| {
        let (x0, y0) = (tx * TILE, ty * TILE);
        let (x1, y1) = ((x0 + TILE).min(w), (y0 + TILE).min(h));
        if (y0..y1).all(|y| mask[y * w + x0..y * w + x1].iter().all(|m| *m == 0)) {
            return;
        }
        if alpha_lock && tile.get().is_none() {
            return;
        }
        let t = tile.make();
        for y in y0..y1 {
            for x in x0..x1 {
                let m = mask[y * w + x];
                if m == 0 {
                    continue;
                }
                let cover = m as f32 / 255.0 * sel.map_or(1.0, |s| s.value(x, y) as f32 / 255.0);
                if cover <= 0.0 {
                    continue;
                }
                let i = ((y - y0) * TILE + (x - x0)) * 4;
                let mut d = [t[i], t[i + 1], t[i + 2], t[i + 3]];
                if alpha_lock && d[3] == 0 {
                    continue;
                }
                let a = (cover * 255.0).round() as u8;
                let src = premultiply([color[0], color[1], color[2], a]);
                if alpha_lock {
                    let orig = d[3];
                    blend_over(&mut d, &src, opacity, BlendMode::Normal);
                    let mut st = unpremultiply(d);
                    st[3] = orig;
                    d = premultiply(st);
                } else {
                    blend_over(&mut d, &src, opacity, BlendMode::Normal);
                }
                t[i..i + 4].copy_from_slice(&d);
            }
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientKind {
    Linear,
    Radial,
}

/// Degradê de `a` (cor em `p0`) até `b` (cor em `p1`); cores com alfa direto.
#[allow(clippy::too_many_arguments)]
pub fn gradient(
    pixels: &mut TiledImage,
    p0: (f32, f32),
    p1: (f32, f32),
    kind: GradientKind,
    a: [u8; 4],
    b: [u8; 4],
    opacity: f32,
    sel: Option<&Selection>,
) {
    let (w, h) = (pixels.width(), pixels.height());
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let len2 = (dx * dx + dy * dy).max(1e-6);
    let len = len2.sqrt();
    for y in 0..h {
        for x in 0..w {
            let cover = sel.map_or(1.0, |s| s.value(x, y) as f32 / 255.0);
            if cover <= 0.0 {
                continue;
            }
            let (px, py) = (x as f32 + 0.5 - p0.0, y as f32 + 0.5 - p0.1);
            let t = match kind {
                GradientKind::Linear => (px * dx + py * dy) / len2,
                GradientKind::Radial => (px * px + py * py).sqrt() / len,
            }
            .clamp(0.0, 1.0);
            let mut c = [0u8; 4];
            for i in 0..4 {
                c[i] = (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8;
            }
            c[3] = (c[3] as f32 * cover).round() as u8;
            let mut d = pixels.pixel(x, y);
            blend_over(&mut d, &premultiply(c), opacity, BlendMode::Normal);
            pixels.set_pixel(x, y, d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture() -> (Vec<[u8; 4]>, usize, usize) {
        // 10x10 branco com um quadrado vermelho de 4x4 e um pontinho vermelho solto
        let (w, h) = (10, 10);
        let mut v = vec![[255, 255, 255, 255]; w * h];
        for y in 2..6 {
            for x in 2..6 {
                v[y * w + x] = [255, 0, 0, 255];
            }
        }
        v[9 * w + 9] = [255, 0, 0, 255];
        (v, w, h)
    }

    #[test]
    fn contiguous_vs_global_flood() {
        let (v, w, h) = picture();
        let px = |x: usize, y: usize| v[y * w + x];
        let m = flood_mask(&px, w, h, (3, 3), 0.1, true);
        assert_eq!(m.iter().filter(|v| **v != 0).count(), 16);
        let g = flood_mask(&px, w, h, (3, 3), 0.1, false);
        assert_eq!(g.iter().filter(|v| **v != 0).count(), 17);
        let all = flood_mask(&px, w, h, (0, 0), 1.0, true);
        assert!(all.iter().all(|v| *v != 0));
        assert_eq!(
            magic_wand(&px, w, h, (0, 0), 0.0, true).bounds(),
            Some((0, 0, 10, 10))
        );
    }

    #[test]
    fn fill_respects_selection_and_opacity() {
        let mut img = TiledImage::new(10, 10);
        let mask = vec![255u8; 100];
        let sel = Selection::rect(10, 10, 0, 0, 5, 10);
        fill_mask(&mut img, &mask, [0, 0, 255], 1.0, Some(&sel), false);
        assert_eq!(img.pixel(2, 2), [0, 0, 255, 255]);
        assert_eq!(img.pixel(7, 2), [0; 4]);
        fill_mask(&mut img, &mask, [255, 0, 0], 0.5, Some(&sel), false);
        let p = img.pixel(2, 2);
        assert!((p[0] as i32 - 128).abs() <= 1 && (p[2] as i32 - 128).abs() <= 1 && p[3] == 255);
    }

    #[test]
    fn gradients_interpolate() {
        let mut img = TiledImage::new(100, 10);
        gradient(
            &mut img,
            (0.0, 0.0),
            (100.0, 0.0),
            GradientKind::Linear,
            [0, 0, 0, 255],
            [255, 255, 255, 255],
            1.0,
            None,
        );
        assert!(img.pixel(0, 5)[0] < 5);
        assert!((img.pixel(50, 5)[0] as i32 - 128).abs() <= 3);
        assert!(img.pixel(99, 5)[0] > 250);
        let mut r = TiledImage::new(100, 100);
        gradient(
            &mut r,
            (50.0, 50.0),
            (80.0, 50.0),
            GradientKind::Radial,
            [255, 0, 0, 255],
            [0, 0, 255, 0],
            1.0,
            None,
        );
        assert!(r.pixel(50, 50)[3] > 240);
        assert_eq!(r.pixel(95, 50)[3], 0);
    }

    #[test]
    fn big_fill_runs_in_parallel_and_leaves_untouched_blocks_alone() {
        let (w, h) = (1100usize, 1000usize);
        let mut t = TiledImage::new(w, h);
        t.fill_solid([100, 100, 100, 255]);
        let before = t.clone();
        let mut mask = vec![0u8; w * h];
        for y in 100..300 {
            mask[y * w + 100..y * w + 400].fill(255);
        }
        fill_mask(&mut t, &mask, [200, 0, 0], 0.5, None, false);
        assert_eq!(t.pixel(150, 150), [150, 50, 50, 255]);
        assert_eq!(t.pixel(99, 150), [100, 100, 100, 255]);
        assert_eq!(t.pixel(400, 150), [100, 100, 100, 255]);
        let ptr = |i: &TiledImage, tx, ty| i.tile(tx, ty).unwrap().as_ptr();
        assert_ne!(
            ptr(&t, 2, 2),
            ptr(&before, 2, 2),
            "bloco pintado foi copiado"
        );
        assert_eq!(
            ptr(&t, 15, 14),
            ptr(&before, 15, 14),
            "bloco longe da máscara continua compartilhado"
        );
        // Com a trava de alfa, onde não há nada (bloco inexistente) nada é criado.
        let mut empty = TiledImage::new(w, h);
        fill_mask(&mut empty, &mask, [200, 0, 0], 1.0, None, true);
        assert!(empty.is_empty() && empty.tile(2, 2).is_none());
    }
}
