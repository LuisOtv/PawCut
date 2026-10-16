//! Modos de mistura de camadas (os mesmos do Krita/Photoshop), com alfa pré-multiplicado.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Add,
}

impl BlendMode {
    pub const ALL: [BlendMode; 13] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Add,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiplicar",
            BlendMode::Screen => "Tela",
            BlendMode::Overlay => "Sobrepor",
            BlendMode::Darken => "Escurecer",
            BlendMode::Lighten => "Clarear",
            BlendMode::ColorDodge => "Subexposição de cor",
            BlendMode::ColorBurn => "Superexposição de cor",
            BlendMode::HardLight => "Luz direta",
            BlendMode::SoftLight => "Luz suave",
            BlendMode::Difference => "Diferença",
            BlendMode::Exclusion => "Exclusão",
            BlendMode::Add => "Adicionar",
        }
    }

    /// Mistura um canal (valores de 0 a 1, cor direta): `b` embaixo, `s` em cima.
    fn channel(self, b: f32, s: f32) -> f32 {
        match self {
            BlendMode::Normal => s,
            BlendMode::Multiply => b * s,
            BlendMode::Screen => b + s - b * s,
            BlendMode::Overlay => BlendMode::HardLight.channel(s, b),
            BlendMode::Darken => b.min(s),
            BlendMode::Lighten => b.max(s),
            BlendMode::ColorDodge => {
                if b == 0.0 {
                    0.0
                } else if s >= 1.0 {
                    1.0
                } else {
                    (b / (1.0 - s)).min(1.0)
                }
            }
            BlendMode::ColorBurn => {
                if b >= 1.0 {
                    1.0
                } else if s <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - b) / s).min(1.0)
                }
            }
            BlendMode::HardLight => {
                if s <= 0.5 {
                    b * 2.0 * s
                } else {
                    let s2 = 2.0 * s - 1.0;
                    b + s2 - b * s2
                }
            }
            BlendMode::SoftLight => {
                if s <= 0.5 {
                    b - (1.0 - 2.0 * s) * b * (1.0 - b)
                } else {
                    let d = if b <= 0.25 {
                        ((16.0 * b - 12.0) * b + 4.0) * b
                    } else {
                        b.sqrt()
                    };
                    b + (2.0 * s - 1.0) * (d - b)
                }
            }
            BlendMode::Difference => (b - s).abs(),
            BlendMode::Exclusion => b + s - 2.0 * b * s,
            BlendMode::Add => (b + s).min(1.0),
        }
    }
}

/// Põe o pixel `src` (pré-multiplicado) sobre `dst` com a opacidade e o modo dados.
pub fn blend_over(dst: &mut [u8], src: &[u8], opacity: f32, mode: BlendMode) {
    let sa = src[3] as f32 / 255.0 * opacity;
    if sa <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    if mode == BlendMode::Normal {
        let k = 1.0 - sa;
        let s = src[3] as f32; // só para evitar divisão; abaixo usamos opacity no rgb
        let _ = s;
        for c in 0..3 {
            dst[c] = (src[c] as f32 * opacity + dst[c] as f32 * k)
                .round()
                .min(255.0) as u8;
        }
        dst[3] = (sa * 255.0 + dst[3] as f32 * k).round().min(255.0) as u8;
        return;
    }
    let out_a = sa + da * (1.0 - sa);
    let src_a = src[3] as f32 / 255.0;
    for c in 0..3 {
        let cs = if src_a > 0.0 {
            (src[c] as f32 / 255.0 / src_a).min(1.0)
        } else {
            0.0
        };
        let cb = if da > 0.0 {
            (dst[c] as f32 / 255.0 / da).min(1.0)
        } else {
            0.0
        };
        let b = mode.channel(cb, cs);
        let co = cs * sa * (1.0 - da) + cb * da * (1.0 - sa) + sa * da * b;
        dst[c] = (co * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_over_transparent_and_opaque() {
        let mut d = [0u8; 4];
        blend_over(&mut d, &[100, 50, 25, 255], 1.0, BlendMode::Normal);
        assert_eq!(d, [100, 50, 25, 255]);
        let mut d = [200, 200, 200, 255];
        blend_over(&mut d, &[0, 0, 0, 255], 0.5, BlendMode::Normal);
        assert_eq!(d, [100, 100, 100, 255]);
    }

    #[test]
    fn multiply_screen_and_difference_on_opaque() {
        let mut d = [255, 128, 0, 255];
        blend_over(&mut d, &[128, 128, 128, 255], 1.0, BlendMode::Multiply);
        assert_eq!(d, [128, 64, 0, 255]);
        let mut d = [0, 0, 0, 255];
        blend_over(&mut d, &[100, 100, 100, 255], 1.0, BlendMode::Screen);
        assert_eq!(d, [100, 100, 100, 255]);
        let mut d = [200, 50, 0, 255];
        blend_over(&mut d, &[50, 200, 0, 255], 1.0, BlendMode::Difference);
        assert_eq!(d, [150, 150, 0, 255]);
    }

    #[test]
    fn blend_modes_over_transparent_keep_the_source() {
        for m in BlendMode::ALL {
            let mut d = [0u8; 4];
            blend_over(&mut d, &[90, 60, 30, 255], 1.0, m);
            assert_eq!(d, [90, 60, 30, 255], "{m:?}");
        }
    }
}
