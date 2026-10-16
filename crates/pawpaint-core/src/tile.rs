//! Imagem em blocos de 64×64 pixels (RGBA, alfa pré-multiplicado). Blocos vazios não ocupam
//! memória e copiar a imagem só copia ponteiros: o desfazer guarda o documento inteiro de graça,
//! e pintar copia só os blocos tocados.

use std::sync::Arc;

use image::RgbaImage;

pub const TILE: usize = 64;
pub const TILE_BYTES: usize = TILE * TILE * 4;

#[derive(Clone, Debug)]
pub struct TiledImage {
    w: usize,
    h: usize,
    tx: usize,
    ty: usize,
    tiles: Vec<Option<Arc<Vec<u8>>>>,
}

impl PartialEq for TiledImage {
    fn eq(&self, other: &Self) -> bool {
        self.w == other.w
            && self.h == other.h
            && self
                .tiles
                .iter()
                .zip(&other.tiles)
                .all(|(a, b)| match (a, b) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
                    (Some(t), None) | (None, Some(t)) => t.iter().all(|v| *v == 0),
                })
    }
}

impl TiledImage {
    pub fn new(w: usize, h: usize) -> Self {
        let (tx, ty) = (w.div_ceil(TILE), h.div_ceil(TILE));
        Self {
            w,
            h,
            tx,
            ty,
            tiles: vec![None; tx * ty],
        }
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    /// Quantidade de blocos na horizontal e na vertical.
    pub fn tile_dims(&self) -> (usize, usize) {
        (self.tx, self.ty)
    }

    pub fn tile(&self, tx: usize, ty: usize) -> Option<&[u8]> {
        self.tiles
            .get(ty * self.tx + tx)?
            .as_deref()
            .map(Vec::as_slice)
    }

    /// Bloco para escrever (cria se estiver vazio e copia se estiver compartilhado).
    pub fn tile_mut(&mut self, tx: usize, ty: usize) -> &mut [u8] {
        let i = ty * self.tx + tx;
        let t = self.tiles[i].get_or_insert_with(|| Arc::new(vec![0; TILE_BYTES]));
        Arc::make_mut(t).as_mut_slice()
    }

    pub fn clear_tile(&mut self, tx: usize, ty: usize) {
        self.tiles[ty * self.tx + tx] = None;
    }

    pub fn clear(&mut self) {
        self.tiles.iter_mut().for_each(|t| *t = None);
    }

    pub fn is_empty(&self) -> bool {
        self.tiles
            .iter()
            .flatten()
            .all(|t| t.iter().all(|v| *v == 0))
    }

    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        if x >= self.w || y >= self.h {
            return [0; 4];
        }
        match self.tile(x / TILE, y / TILE) {
            Some(t) => {
                let i = ((y % TILE) * TILE + x % TILE) * 4;
                [t[i], t[i + 1], t[i + 2], t[i + 3]]
            }
            None => [0; 4],
        }
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, p: [u8; 4]) {
        if x >= self.w || y >= self.h {
            return;
        }
        let i = ((y % TILE) * TILE + x % TILE) * 4;
        // Escrever transparente num bloco que não existe não precisa criar o bloco.
        if p == [0; 4] && self.tile(x / TILE, y / TILE).is_none() {
            return;
        }
        let t = self.tile_mut(x / TILE, y / TILE);
        t[i..i + 4].copy_from_slice(&p);
    }

    /// De uma imagem comum (alfa direto) para blocos pré-multiplicados.
    pub fn from_rgba(img: &RgbaImage) -> Self {
        let mut out = Self::new(img.width() as usize, img.height() as usize);
        for (x, y, p) in img.enumerate_pixels() {
            let a = p.0[3] as u32;
            if a == 0 {
                continue;
            }
            let m = |c: u8| ((c as u32 * a + 127) / 255) as u8;
            out.set_pixel(
                x as usize,
                y as usize,
                [m(p.0[0]), m(p.0[1]), m(p.0[2]), a as u8],
            );
        }
        out
    }

    /// Região `[x0, x1) × [y0, y1)` como imagem comum (alfa direto).
    pub fn region_to_rgba(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> RgbaImage {
        let (x1, y1) = (x1.min(self.w), y1.min(self.h));
        let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        let mut img = RgbaImage::new(w as u32, h as u32);
        for y in 0..h {
            for x in 0..w {
                let p = self.pixel(x0 + x, y0 + y);
                img.put_pixel(x as u32, y as u32, image::Rgba(unpremultiply(p)));
            }
        }
        img
    }

    pub fn to_rgba(&self) -> RgbaImage {
        self.region_to_rgba(0, 0, self.w, self.h)
    }

    /// Escreve `img` (alfa direto) com o canto em (`x0`, `y0`), trocando o que havia.
    pub fn write_rgba(&mut self, img: &RgbaImage, x0: i64, y0: i64) {
        for (x, y, p) in img.enumerate_pixels() {
            let (px, py) = (x0 + x as i64, y0 + y as i64);
            if px < 0 || py < 0 {
                continue;
            }
            let a = p.0[3] as u32;
            let m = |c: u8| ((c as u32 * a + 127) / 255) as u8;
            self.set_pixel(
                px as usize,
                py as usize,
                [m(p.0[0]), m(p.0[1]), m(p.0[2]), a as u8],
            );
        }
    }

    /// Menor retângulo (x0, y0, x1, y1) que contém todos os pixels não transparentes.
    pub fn content_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        let mut b: Option<(usize, usize, usize, usize)> = None;
        for ty in 0..self.ty {
            for tx in 0..self.tx {
                let Some(t) = self.tile(tx, ty) else { continue };
                for y in 0..TILE {
                    for x in 0..TILE {
                        if t[(y * TILE + x) * 4 + 3] != 0 {
                            let (px, py) = (tx * TILE + x, ty * TILE + y);
                            if px >= self.w || py >= self.h {
                                continue;
                            }
                            b = Some(match b {
                                None => (px, py, px + 1, py + 1),
                                Some((a, bb, c, d)) => {
                                    (a.min(px), bb.min(py), c.max(px + 1), d.max(py + 1))
                                }
                            });
                        }
                    }
                }
            }
        }
        b
    }

    /// Copia para uma tela de outro tamanho, deslocando o conteúdo em (`dx`, `dy`).
    pub fn resized(&self, w: usize, h: usize, dx: i64, dy: i64) -> Self {
        let mut out = Self::new(w, h);
        for ty in 0..self.ty {
            for tx in 0..self.tx {
                let Some(t) = self.tile(tx, ty) else { continue };
                for y in 0..TILE {
                    for x in 0..TILE {
                        let i = (y * TILE + x) * 4;
                        if t[i + 3] == 0 {
                            continue;
                        }
                        let (px, py) = ((tx * TILE + x) as i64 + dx, (ty * TILE + y) as i64 + dy);
                        if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                            out.set_pixel(
                                px as usize,
                                py as usize,
                                [t[i], t[i + 1], t[i + 2], t[i + 3]],
                            );
                        }
                    }
                }
            }
        }
        out
    }
}

pub fn unpremultiply(p: [u8; 4]) -> [u8; 4] {
    if p[3] == 0 || p[3] == 255 {
        return if p[3] == 0 { [0; 4] } else { p };
    }
    let a = p[3] as u32;
    let u = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
    [u(p[0]), u(p[1]), u(p[2]), p[3]]
}

pub fn premultiply(p: [u8; 4]) -> [u8; 4] {
    let a = p[3] as u32;
    let m = |c: u8| ((c as u32 * a + 127) / 255) as u8;
    [m(p[0]), m(p[1]), m(p[2]), p[3]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_tiles_and_copy_on_write() {
        let mut a = TiledImage::new(200, 100);
        assert!(a.is_empty());
        assert!(a.tile(0, 0).is_none());
        a.set_pixel(70, 5, [10, 20, 30, 255]);
        assert!(a.tile(1, 0).is_some() && a.tile(0, 0).is_none());
        let snapshot = a.clone();
        a.set_pixel(70, 5, [1, 2, 3, 255]);
        assert_eq!(snapshot.pixel(70, 5), [10, 20, 30, 255]);
        assert_eq!(a.pixel(70, 5), [1, 2, 3, 255]);
        assert_eq!(a.pixel(500, 5), [0; 4]);
        // escrever transparente em bloco vazio não aloca
        a.set_pixel(190, 90, [0; 4]);
        assert!(a.tile(2, 1).is_none());
    }

    #[test]
    fn rgba_round_trip_and_bounds() {
        let mut img = RgbaImage::new(130, 70);
        img.put_pixel(100, 60, image::Rgba([200, 100, 50, 128]));
        img.put_pixel(3, 4, image::Rgba([255, 0, 0, 255]));
        let t = TiledImage::from_rgba(&img);
        assert_eq!(t.content_bounds(), Some((3, 4, 101, 61)));
        let back = t.to_rgba();
        let p = back.get_pixel(100, 60).0;
        assert!((p[0] as i32 - 200).abs() <= 2 && p[3] == 128);
        assert_eq!(back.get_pixel(3, 4).0, [255, 0, 0, 255]);
    }

    #[test]
    fn resized_moves_content() {
        let mut t = TiledImage::new(100, 100);
        t.set_pixel(10, 10, [9, 9, 9, 255]);
        let r = t.resized(50, 50, -5, 20);
        assert_eq!(r.pixel(5, 30), [9, 9, 9, 255]);
        assert_eq!(r.content_bounds(), Some((5, 30, 6, 31)));
    }
}
