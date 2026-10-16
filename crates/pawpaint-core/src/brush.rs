//! Motor de pincel: carimbos (dabs) espaçados ao longo do traço, acumulados num buffer do traço
//! (o "fluxo" soma, a "opacidade" limita o traço inteiro) e só depois aplicados na camada.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::blend::{BlendMode, blend_over};
use crate::selection::Selection;
use crate::tile::{TILE, TILE_BYTES, TiledImage, premultiply, unpremultiply};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Tip {
    #[default]
    Round,
    Square,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Brush {
    pub name: String,
    /// Diâmetro em pixels.
    pub size: f32,
    /// 0 = bordas totalmente macias, 1 = bordas duras.
    pub hardness: f32,
    /// Teto de opacidade do traço inteiro.
    pub opacity: f32,
    /// Quanto cada carimbo deposita (soma quando carimbos se cruzam).
    pub flow: f32,
    /// Distância entre carimbos, em fração do tamanho.
    pub spacing: f32,
    pub angle: f32,
    /// 1 = redondo, menor = achatado.
    pub roundness: f32,
    pub tip: Tip,
    /// Força da textura de grão (0 = liso).
    pub grain: f32,
    /// Quanto os carimbos se espalham em volta do traço (fração do tamanho).
    pub scatter: f32,
    /// Estabilizador: 0 = segue o mouse, perto de 1 = suaviza muito.
    pub smoothing: f32,
}

impl Brush {
    pub fn basic() -> Self {
        Self {
            name: "Básico".into(),
            size: 24.0,
            hardness: 0.85,
            opacity: 1.0,
            flow: 1.0,
            spacing: 0.1,
            angle: 0.0,
            roundness: 1.0,
            tip: Tip::Round,
            grain: 0.0,
            scatter: 0.0,
            smoothing: 0.0,
        }
    }

    /// Os pincéis que vêm prontos.
    pub fn presets() -> Vec<Brush> {
        let b = Brush::basic();
        vec![
            b.clone(),
            Brush {
                name: "Aerógrafo".into(),
                hardness: 0.0,
                flow: 0.25,
                spacing: 0.08,
                size: 60.0,
                ..b.clone()
            },
            Brush {
                name: "Lápis".into(),
                size: 4.0,
                hardness: 1.0,
                grain: 0.55,
                spacing: 0.06,
                ..b.clone()
            },
            Brush {
                name: "Tinta".into(),
                size: 10.0,
                hardness: 1.0,
                spacing: 0.04,
                smoothing: 0.5,
                ..b.clone()
            },
            Brush {
                name: "Marcador".into(),
                size: 30.0,
                hardness: 1.0,
                tip: Tip::Square,
                angle: 45.0,
                roundness: 0.4,
                opacity: 0.8,
                spacing: 0.05,
                ..b.clone()
            },
            Brush {
                name: "Giz".into(),
                size: 36.0,
                hardness: 0.6,
                grain: 0.85,
                flow: 0.6,
                spacing: 0.12,
                scatter: 0.04,
                ..b.clone()
            },
            Brush {
                name: "Aquarela macia".into(),
                size: 80.0,
                hardness: 0.2,
                opacity: 0.45,
                flow: 0.5,
                spacing: 0.1,
                ..b
            },
        ]
    }
}

/// Cobertura do traço em andamento, em blocos (RGBA pré-multiplicado).
#[derive(Debug, Clone, Default)]
pub struct StrokeBuffer {
    tiles: HashMap<(usize, usize), Vec<u8>>,
    dirty: HashSet<(usize, usize)>,
}

impl StrokeBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.tiles.clear();
        self.dirty.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn tile(&self, tx: usize, ty: usize) -> Option<&[u8]> {
        self.tiles.get(&(tx, ty)).map(Vec::as_slice)
    }

    pub fn tile_keys(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.tiles.keys().copied()
    }

    /// Blocos mexidos desde a última chamada.
    pub fn take_dirty(&mut self) -> Vec<(usize, usize)> {
        self.dirty.drain().collect()
    }

    fn tile_mut(&mut self, tx: usize, ty: usize) -> &mut Vec<u8> {
        self.dirty.insert((tx, ty));
        self.tiles
            .entry((tx, ty))
            .or_insert_with(|| vec![0; TILE_BYTES])
    }
}

fn hash_noise(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0xffff) as f32 / 65535.0
}

/// Quem desenha um traço: guarda a posição suavizada e a distância desde o último carimbo.
pub struct Stroker {
    pub brush: Brush,
    pub color: [u8; 3],
    pub erase: bool,
    pub buf: StrokeBuffer,
    w: usize,
    h: usize,
    smoothed: Option<(f32, f32)>,
    last: Option<(f32, f32)>,
    carry: f32,
    seed: u32,
}

impl Stroker {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            brush: Brush::basic(),
            color: [0, 0, 0],
            erase: false,
            buf: StrokeBuffer::new(),
            w,
            h,
            smoothed: None,
            last: None,
            carry: 0.0,
            seed: 1,
        }
    }

    pub fn begin(&mut self, brush: Brush, color: [u8; 3], erase: bool) {
        self.brush = brush;
        self.color = color;
        self.erase = erase;
        self.buf.clear();
        self.smoothed = None;
        self.last = None;
        self.carry = 0.0;
    }

    /// Próximo ponto do mouse. Põe os carimbos entre o ponto anterior e este.
    pub fn add_point(&mut self, x: f32, y: f32) {
        let target = (x, y);
        let pos = match self.smoothed {
            None => target,
            Some(p) => {
                let k = 1.0 - 0.92 * self.brush.smoothing.clamp(0.0, 1.0);
                (p.0 + (target.0 - p.0) * k, p.1 + (target.1 - p.1) * k)
            }
        };
        self.smoothed = Some(pos);
        self.move_to(pos);
    }

    /// Fim do traço: alcança o último ponto do mouse (o suavizador fica um pouco atrás).
    pub fn finish(&mut self, x: f32, y: f32) {
        if self.smoothed.is_some() {
            let saved = self.brush.smoothing;
            self.brush.smoothing = 0.0;
            self.add_point(x, y);
            self.brush.smoothing = saved;
        }
    }

    fn move_to(&mut self, pos: (f32, f32)) {
        let step = (self.brush.spacing * self.brush.size).max(1.0);
        let Some(last) = self.last else {
            self.stamp(pos.0, pos.1);
            self.last = Some(pos);
            self.carry = 0.0;
            return;
        };
        let (dx, dy) = (pos.0 - last.0, pos.1 - last.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 0.0 {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        // `last` é o ponto anterior do caminho e `carry` a distância andada desde o último
        // carimbo até ele: continua de onde parou.
        let mut travelled = 0.0f32;
        let mut remaining = len;
        loop {
            let need = step - self.carry;
            if remaining < need {
                self.carry += remaining;
                break;
            }
            travelled += need;
            remaining -= need;
            self.stamp(last.0 + ux * travelled, last.1 + uy * travelled);
            self.carry = 0.0;
        }
        self.last = Some(pos);
    }

    fn stamp(&mut self, cx: f32, cy: f32) {
        let b = self.brush.clone();
        let (mut cx, mut cy) = (cx, cy);
        if b.scatter > 0.0 {
            self.seed = self
                .seed
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let a = (self.seed >> 8) as f32 / 16_777_216.0 * std::f32::consts::TAU;
            self.seed = self
                .seed
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let r = (self.seed >> 8) as f32 / 16_777_216.0 * b.scatter * b.size;
            cx += a.cos() * r;
            cy += a.sin() * r;
        }
        let r = (b.size / 2.0).max(0.5);
        let (rx, ry) = (r, (r * b.roundness.clamp(0.05, 1.0)).max(0.5));
        let (sin, cos) = b.angle.to_radians().sin_cos();
        let reach = r + 1.5;
        let (x0, x1) = (
            (cx - reach).floor().max(0.0) as i64,
            ((cx + reach).ceil() as i64).min(self.w as i64),
        );
        let (y0, y1) = (
            (cy - reach).floor().max(0.0) as i64,
            ((cy + reach).ceil() as i64).min(self.h as i64),
        );
        let hard = b.hardness.clamp(0.0, 0.999);
        for py in y0..y1 {
            for px in x0..x1 {
                let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
                let (u, v) = ((dx * cos + dy * sin) / rx, (-dx * sin + dy * cos) / ry);
                let d = match b.tip {
                    Tip::Round => (u * u + v * v).sqrt(),
                    Tip::Square => u.abs().max(v.abs()),
                };
                if d >= 1.0 + 1.5 / rx.min(ry) {
                    continue;
                }
                let falloff = if d <= hard {
                    1.0
                } else {
                    let t = ((1.0 - d) / (1.0 - hard)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                };
                // Borda de 1 pixel suave mesmo nos pincéis duros.
                let aa = (((1.0 - d) * rx.min(ry)) + 0.5).clamp(0.0, 1.0);
                let mut a = falloff * aa;
                if b.grain > 0.0 {
                    let n = hash_noise(px as i32, py as i32);
                    a *= 1.0 - b.grain * (1.0 - n);
                }
                a *= b.flow;
                if a <= 0.0 {
                    continue;
                }
                self.deposit(px as usize, py as usize, a);
            }
        }
    }

    fn deposit(&mut self, x: usize, y: usize, a: f32) {
        let color = self.color;
        let erase = self.erase;
        let t = self.buf.tile_mut(x / TILE, y / TILE);
        let i = ((y % TILE) * TILE + x % TILE) * 4;
        let da = t[i + 3] as f32 / 255.0;
        let k = 1.0 - a;
        if erase {
            t[i + 3] = ((a + da * k) * 255.0).round().min(255.0) as u8;
        } else {
            for c in 0..3 {
                t[i + c] = (color[c] as f32 * a + t[i + c] as f32 * k)
                    .round()
                    .min(255.0) as u8;
            }
            t[i + 3] = ((a + da * k) * 255.0).round().min(255.0) as u8;
        }
    }

    /// Traço reto de `a` até `b` (linhas e contornos de formas).
    pub fn line(&mut self, a: (f32, f32), b: (f32, f32)) {
        let saved = self.brush.smoothing;
        self.brush.smoothing = 0.0;
        self.add_point(a.0, a.1);
        self.add_point(b.0, b.1);
        self.brush.smoothing = saved;
    }
}

/// Aplica o traço (já com opacidade) sobre um bloco da camada.
#[allow(clippy::too_many_arguments)]
pub fn apply_stroke_tile(
    dst: &mut [u8],
    scratch: &[u8],
    opacity: f32,
    erase: bool,
    mode: BlendMode,
    sel: Option<&[u8]>,
    alpha_lock: bool,
) {
    for p in 0..TILE * TILE {
        let i = p * 4;
        let s = &scratch[i..i + 4];
        if s[3] == 0 {
            continue;
        }
        let m = sel.map_or(1.0, |m| m[p] as f32 / 255.0);
        if m <= 0.0 {
            continue;
        }
        let d = &mut dst[i..i + 4];
        if erase {
            if alpha_lock {
                continue;
            }
            let k = 1.0 - s[3] as f32 / 255.0 * opacity * m;
            for v in d.iter_mut() {
                *v = (*v as f32 * k).round() as u8;
            }
            continue;
        }
        if alpha_lock {
            let orig_a = d[3];
            if orig_a == 0 {
                continue;
            }
            let mut tmp = [d[0], d[1], d[2], orig_a];
            blend_over(&mut tmp, s, opacity * m, mode);
            // Só a cor muda: o alfa volta ao que era.
            let mut st = unpremultiply(tmp);
            st[3] = orig_a;
            d.copy_from_slice(&premultiply(st));
        } else {
            blend_over(d, s, opacity * m, mode);
        }
    }
}

/// Aplica o traço inteiro na camada, respeitando seleção e trava de alfa.
pub fn commit_stroke(
    pixels: &mut TiledImage,
    buf: &StrokeBuffer,
    opacity: f32,
    erase: bool,
    mode: BlendMode,
    sel: Option<&Selection>,
    alpha_lock: bool,
) {
    for (tx, ty) in buf.tile_keys().collect::<Vec<_>>() {
        let Some(scratch) = buf.tile(tx, ty) else {
            continue;
        };
        if tx >= pixels.tile_dims().0 || ty >= pixels.tile_dims().1 {
            continue;
        }
        let mask = sel.map(|s| s.tile_mask(tx, ty));
        let dst = pixels.tile_mut(tx, ty);
        apply_stroke_tile(
            dst,
            scratch,
            opacity,
            erase,
            mode,
            mask.as_deref(),
            alpha_lock,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(brush: Brush, pts: &[(f32, f32)]) -> (StrokeBuffer, TiledImage) {
        let mut s = Stroker::new(200, 100);
        s.begin(brush, [255, 0, 0], false);
        for p in pts {
            s.add_point(p.0, p.1);
        }
        let last = pts[pts.len() - 1];
        s.finish(last.0, last.1);
        let mut img = TiledImage::new(200, 100);
        commit_stroke(&mut img, &s.buf, 1.0, false, BlendMode::Normal, None, false);
        (s.buf, img)
    }

    #[test]
    fn a_hard_dab_paints_a_disc() {
        let b = Brush {
            size: 20.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        let (_, img) = stroke(b, &[(50.0, 50.0)]);
        assert_eq!(img.pixel(50, 50), [255, 0, 0, 255]);
        assert_eq!(img.pixel(58, 50), [255, 0, 0, 255]);
        assert_eq!(img.pixel(65, 50), [0; 4]);
        assert_eq!(img.pixel(50, 65), [0; 4]);
    }

    #[test]
    fn a_stroke_is_continuous_between_points() {
        let b = Brush {
            size: 8.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        let (_, img) = stroke(b, &[(20.0, 50.0), (150.0, 50.0)]);
        for x in 22..148 {
            assert_eq!(img.pixel(x, 50)[3], 255, "furo em x={x}");
        }
        assert_eq!(img.pixel(100, 60), [0; 4]);
    }

    #[test]
    fn soft_brush_fades_out_and_flow_builds_up() {
        let soft = Brush {
            size: 40.0,
            hardness: 0.0,
            ..Brush::basic()
        };
        let (_, img) = stroke(soft, &[(100.0, 50.0)]);
        let (c, e) = (img.pixel(100, 50)[3], img.pixel(100 + 15, 50)[3]);
        assert!(c > 240 && e < c && e > 0, "{c} {e}");
        let weak = Brush {
            size: 30.0,
            hardness: 1.0,
            flow: 0.3,
            ..Brush::basic()
        };
        let (_, one) = stroke(weak.clone(), &[(100.0, 50.0)]);
        let (_, many) = stroke(
            weak,
            &[(100.0, 50.0), (101.0, 50.0), (100.0, 51.0), (99.0, 50.0)],
        );
        assert!(many.pixel(100, 50)[3] > one.pixel(100, 50)[3]);
    }

    #[test]
    fn opacity_caps_the_whole_stroke_and_erase_removes() {
        let b = Brush {
            size: 20.0,
            hardness: 1.0,
            flow: 1.0,
            ..Brush::basic()
        };
        let mut s = Stroker::new(100, 100);
        s.begin(b.clone(), [0, 255, 0], false);
        s.add_point(30.0, 30.0);
        s.add_point(60.0, 30.0);
        s.add_point(30.0, 32.0); // volta por cima
        let mut img = TiledImage::new(100, 100);
        commit_stroke(&mut img, &s.buf, 0.5, false, BlendMode::Normal, None, false);
        let a = img.pixel(40, 30)[3];
        assert!((126..=129).contains(&a), "alfa {a}");
        // borracha
        let mut e = Stroker::new(100, 100);
        e.begin(b, [0; 3], true);
        e.add_point(40.0, 30.0);
        commit_stroke(&mut img, &e.buf, 1.0, true, BlendMode::Normal, None, false);
        assert_eq!(img.pixel(40, 30)[3], 0);
        assert!(img.pixel(55, 30)[3] > 100);
    }

    #[test]
    fn selection_and_alpha_lock_limit_the_paint() {
        let b = Brush {
            size: 60.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        let mut s = Stroker::new(100, 100);
        s.begin(b, [255, 0, 0], false);
        s.add_point(50.0, 50.0);
        let sel = Selection::rect(100, 100, 0, 0, 50, 100);
        let mut img = TiledImage::new(100, 100);
        commit_stroke(
            &mut img,
            &s.buf,
            1.0,
            false,
            BlendMode::Normal,
            Some(&sel),
            false,
        );
        assert_eq!(img.pixel(40, 50)[3], 255);
        assert_eq!(img.pixel(60, 50)[3], 0);
        // trava de alfa: só pinta onde já há tinta
        let mut base = TiledImage::new(100, 100);
        base.set_pixel(50, 50, [0, 0, 255, 255]);
        commit_stroke(&mut base, &s.buf, 1.0, false, BlendMode::Normal, None, true);
        assert_eq!(base.pixel(50, 50), [255, 0, 0, 255]);
        assert_eq!(base.pixel(51, 50), [0; 4]);
    }

    #[test]
    fn grain_and_square_tip() {
        let b = Brush {
            size: 30.0,
            hardness: 1.0,
            grain: 0.9,
            ..Brush::basic()
        };
        let (_, img) = stroke(b, &[(60.0, 50.0)]);
        let alphas: Vec<u8> = (50..70).map(|x| img.pixel(x, 50)[3]).collect();
        assert!(alphas.iter().any(|a| *a < 200) && alphas.iter().any(|a| *a > 200));
        let sq = Brush {
            size: 30.0,
            hardness: 1.0,
            tip: Tip::Square,
            ..Brush::basic()
        };
        let (_, img) = stroke(sq, &[(60.0, 50.0)]);
        assert_eq!(img.pixel(60 + 13, 50 - 13)[3], 255);
    }
}
