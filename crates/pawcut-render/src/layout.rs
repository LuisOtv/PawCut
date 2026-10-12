//! Geometria das camadas e fades: funções puras, em pixels.

use pawcut_core::{Clip, Frame, Transform};

/// Retângulo no quadro de saída (y para baixo).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub w: f32,
    pub h: f32,
}

/// Como uma camada ocupa o quadro antes do transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Fit {
    /// Vídeo: aumenta ou reduz até caber inteiro (como o "Scale to fit" do Resolve).
    Contain,
    /// Imagens e texto: tamanho nativo, reduzido só se não couber.
    Shrink,
    /// Cor sólida: o quadro inteiro.
    Fill,
}

/// Tamanho da camada em pixels do projeto, com escala 1.
pub(crate) fn base_size(fit: Fit, native: (f32, f32), project: (f32, f32)) -> (f32, f32) {
    let (w, h) = (native.0.max(1.0), native.1.max(1.0));
    let contain = (project.0 / w).min(project.1 / h);
    let s = match fit {
        Fit::Contain => contain,
        Fit::Shrink => contain.min(1.0),
        Fit::Fill => return project,
    };
    (w * s, h * s)
}

/// Onde a camada cai no quadro de saída. `x`/`y` do transform são pixels do projeto
/// a partir do centro, com y para cima (como no Resolve).
pub(crate) fn dest_rect(
    base: (f32, f32),
    t: &Transform,
    project: (f32, f32),
    out: (f32, f32),
) -> Rect {
    let k = out.0 / project.0;
    let (w, h) = (base.0 * t.scale * k, base.1 * t.scale * k);
    let cx = (project.0 / 2.0 + t.x) * k;
    let cy = (project.1 / 2.0 - t.y) * k;
    Rect {
        x0: cx - w / 2.0,
        y0: cy - h / 2.0,
        w,
        h,
    }
}

/// Multiplicador de opacidade dos fades: 0 no primeiro e no último frame quando há fade.
pub(crate) fn fade_factor(clip: &Clip, frame: Frame) -> f32 {
    // Dentro de um dissolve o clipe aparece um pouco antes do início / depois do fim.
    let frame = frame.clamp(clip.start, clip.end() - 1);
    let mut a = 1.0f32;
    if clip.fade_in > 0 {
        a = a.min((frame - clip.start) as f32 / clip.fade_in as f32);
    }
    if clip.fade_out > 0 {
        a = a.min((clip.end() - 1 - frame) as f32 / clip.fade_out as f32);
    }
    a.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_sizes() {
        let p = (320.0, 180.0);
        assert_eq!(base_size(Fit::Contain, (64.0, 36.0), p), (320.0, 180.0));
        assert_eq!(base_size(Fit::Contain, (100.0, 100.0), p), (180.0, 180.0));
        assert_eq!(base_size(Fit::Shrink, (32.0, 32.0), p), (32.0, 32.0));
        assert_eq!(base_size(Fit::Shrink, (640.0, 90.0), p), (320.0, 45.0));
        assert_eq!(base_size(Fit::Fill, (1.0, 1.0), p), p);
    }

    #[test]
    fn dest_rect_scales_moves_and_flips_y() {
        let p = (320.0, 180.0);
        let t = Transform {
            x: 10.0,
            y: 20.0,
            scale: 0.5,
            opacity: 1.0,
            ..Default::default()
        };
        let r = dest_rect(p, &t, p, p);
        assert_eq!(
            r,
            Rect {
                x0: 90.0,
                y0: 25.0,
                w: 160.0,
                h: 90.0
            }
        );
        let half = dest_rect(p, &t, p, (160.0, 90.0));
        assert_eq!(
            half,
            Rect {
                x0: 45.0,
                y0: 12.5,
                w: 80.0,
                h: 45.0
            }
        );
    }
}
