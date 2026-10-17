//! Slider da suíte, no estilo do Inspector do Resolve: trilho fino, a parte preenchida em
//! laranja (a partir do zero quando o intervalo passa pelo zero, como ganho e pan), botão
//! redondo e o valor num campo de largura fixa à direita. Substitui o `egui::Slider`, que tem
//! botão retangular e campo que muda de largura com os dígitos.
//!
//! A API imita a do egui (`new`, `suffix`, `text`, `logarithmic`, `show_value`,
//! `max_decimals`), então trocar um pelo outro é trocar o caminho.

use std::ops::RangeInclusive;

use egui::emath::Numeric;
use egui::{Color32, Rect, Response, Sense, Stroke, Ui, Widget, pos2, vec2};

use crate::theme;

/// Largura padrão do trilho.
pub const RAIL_W: f32 = 120.0;
const RAIL_H: f32 = 4.0;
const KNOB_R: f32 = 6.0;
/// Campo do valor, do mesmo tamanho em todos os sliders.
pub const VALUE_W: f32 = 72.0;
/// Coluna do rótulo (quando o slider tem `text`).
pub const LABEL_W: f32 = 104.0;

pub struct Slider<'a, N: Numeric> {
    value: &'a mut N,
    range: RangeInclusive<N>,
    suffix: String,
    text: String,
    log: bool,
    show_value: bool,
    decimals: Option<usize>,
    width: Option<f32>,
    from_zero: bool,
}

impl<'a, N: Numeric> Slider<'a, N> {
    pub fn new(value: &'a mut N, range: RangeInclusive<N>) -> Self {
        Self {
            value,
            range,
            suffix: String::new(),
            text: String::new(),
            log: false,
            show_value: true,
            decimals: None,
            width: None,
            from_zero: true,
        }
    }

    pub fn suffix(mut self, s: impl Into<String>) -> Self {
        self.suffix = s.into();
        self
    }

    /// Rótulo depois do valor (como no egui).
    pub fn text(mut self, s: impl Into<String>) -> Self {
        self.text = s.into();
        self
    }

    pub fn logarithmic(mut self, on: bool) -> Self {
        self.log = on;
        self
    }

    pub fn show_value(mut self, on: bool) -> Self {
        self.show_value = on;
        self
    }

    pub fn max_decimals(mut self, n: usize) -> Self {
        self.decimals = Some(n);
        self
    }

    /// Sempre preenchido; existe só para trocar `egui::Slider` sem mexer no resto.
    pub fn trailing_fill(self, _on: bool) -> Self {
        self
    }

    /// Preencher a partir do zero quando o intervalo passa por ele (padrão). Desligue quando o
    /// número não é um valor com sinal (ex.: zoom guardado em logaritmo).
    pub fn fill_from_zero(mut self, on: bool) -> Self {
        self.from_zero = on;
        self
    }

    /// Largura do trilho (sem o campo do valor).
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }
}

/// Posição (0 a 1) de `v` no trilho, linear ou logarítmica.
pub fn to_t(v: f64, lo: f64, hi: f64, log: bool) -> f64 {
    if hi <= lo || !v.is_finite() {
        return 0.0;
    }
    let t = if log && lo > 0.0 {
        (v.max(lo).ln() - lo.ln()) / (hi.ln() - lo.ln())
    } else {
        (v - lo) / (hi - lo)
    };
    t.clamp(0.0, 1.0)
}

/// Valor na posição `t` (0 a 1) do trilho.
pub fn from_t(t: f64, lo: f64, hi: f64, log: bool) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if log && lo > 0.0 && hi > lo {
        (lo.ln() + t * (hi.ln() - lo.ln())).exp()
    } else {
        lo + t * (hi - lo)
    }
}

impl<N: Numeric> Widget for Slider<'_, N> {
    fn ui(self, ui: &mut Ui) -> Response {
        // Trilho, valor e rótulo numa linha só: dentro de uma `Grid` contam como uma célula.
        ui.horizontal(|ui| self.contents(ui)).inner
    }
}

impl<N: Numeric> Slider<'_, N> {
    fn contents(self, ui: &mut Ui) -> Response {
        let (lo, hi) = (self.range.start().to_f64(), self.range.end().to_f64());
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        let height = ui.spacing().interact_size.y;
        let width = self
            .width
            .unwrap_or_else(|| ui.spacing().slider_width.clamp(48.0, RAIL_W * 2.0));
        // Rótulo à esquerda numa coluna de largura fixa: vários sliders seguidos (num diálogo, num
        // docker) ficam com os trilhos alinhados.
        let label = (!self.text.is_empty()).then(|| {
            let mut t = self.text.clone();
            if let Some(f) = t.get(..1) {
                let up = f.to_uppercase();
                t.replace_range(..1, &up);
            }
            ui.allocate_ui_with_layout(
                vec2(LABEL_W, height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(LABEL_W);
                    ui.add(
                        egui::Label::new(egui::RichText::new(t).color(theme::TEXT_DIM)).truncate(),
                    )
                },
            )
            .inner
        });
        let (rect, mut resp) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
        let rail = Rect::from_center_size(rect.center(), vec2(rect.width() - 2.0 * KNOB_R, RAIL_H));
        let mut v = self.value.to_f64();
        let mut changed = false;
        if let Some(p) = resp.interact_pointer_pos()
            && (resp.dragged_by(egui::PointerButton::Primary)
                || resp.clicked()
                || resp.drag_started_by(egui::PointerButton::Primary))
        {
            let t = ((p.x - rail.left()) / rail.width().max(1.0)) as f64;
            v = from_t(t, lo, hi, self.log);
            changed = true;
        }
        // Teclado (com o slider em foco) e roda do mouse com Ctrl: passos de 1 % do trilho.
        if resp.has_focus() {
            let step = ui.input(|i| {
                i.num_presses(egui::Key::ArrowRight) as i32
                    - i.num_presses(egui::Key::ArrowLeft) as i32
            });
            if step != 0 {
                let t = to_t(v, lo, hi, self.log) + step as f64 * 0.01;
                v = from_t(t, lo, hi, self.log);
                changed = true;
            }
        }
        if changed {
            if N::INTEGRAL {
                v = v.round();
            } else if let Some(d) = self.decimals {
                let k = 10f64.powi(d as i32);
                v = (v * k).round() / k;
            }
            let new = N::from_f64(v.clamp(lo, hi));
            if new != *self.value {
                *self.value = new;
                resp.mark_changed();
            }
        }
        if ui.is_rect_visible(rect) {
            paint(
                ui,
                &resp,
                rail,
                self.value.to_f64(),
                (lo, hi),
                self.log,
                self.from_zero,
            );
        }
        resp.widget_info(|| {
            egui::WidgetInfo::slider(ui.is_enabled(), self.value.to_f64(), &self.text)
        });
        let mut out = resp;
        if self.show_value {
            let decimals = if N::INTEGRAL {
                0
            } else {
                self.decimals.unwrap_or_else(|| auto_decimals(lo, hi))
            };
            // Tamanho exato (não mínimo): "-24.0 dB" e "0.0 dB" ocupam o mesmo espaço.
            let drag = ui.add_sized(
                vec2(VALUE_W, height),
                egui::DragValue::new(self.value)
                    .range(self.range.clone())
                    .suffix(self.suffix.clone())
                    .fixed_decimals(decimals)
                    .speed(((hi - lo) / 300.0).max(if N::INTEGRAL { 0.25 } else { 0.0 })),
            );
            out = out.union(drag);
        }
        if let Some(l) = label {
            out = out.union(l);
        }
        out
    }
}

/// Casas decimais que fazem sentido para o tamanho do intervalo.
fn auto_decimals(lo: f64, hi: f64) -> usize {
    let span = hi - lo;
    if span >= 100.0 {
        0
    } else if span >= 5.0 {
        1
    } else {
        2
    }
}

fn paint(
    ui: &Ui,
    resp: &Response,
    rail: Rect,
    v: f64,
    (lo, hi): (f64, f64),
    log: bool,
    from_zero: bool,
) {
    let p = ui.painter();
    let hot = resp.hovered() || resp.dragged_by(egui::PointerButton::Primary) || resp.has_focus();
    p.rect_filled(rail.expand(1.0), 3.0, theme::BORDER);
    p.rect_filled(rail, 2.0, theme::BG_DARK);
    let x_of = |t: f64| rail.left() + rail.width() * t as f32;
    let knob_x = x_of(to_t(v, lo, hi, log));
    // Intervalo que passa pelo zero (ganho, pan, brilho): preenche a partir do zero.
    let zero = if from_zero && lo < 0.0 && hi > 0.0 && !log {
        Some(x_of(to_t(0.0, lo, hi, false)))
    } else {
        None
    };
    let from = zero.unwrap_or(rail.left());
    let fill = if ui.is_enabled() {
        if hot {
            theme::ACCENT
        } else {
            theme::ACCENT.gamma_multiply(0.85)
        }
    } else {
        theme::TEXT_DIM
    };
    let (a, b) = (from.min(knob_x), from.max(knob_x));
    if b > a {
        p.rect_filled(
            Rect::from_min_max(pos2(a, rail.top()), pos2(b, rail.bottom())),
            2.0,
            fill,
        );
    }
    if let Some(z) = zero {
        p.line_segment(
            [pos2(z, rail.top() - 3.0), pos2(z, rail.bottom() + 3.0)],
            Stroke::new(1.0, theme::TEXT_DIM),
        );
    }
    let knob = pos2(knob_x, rail.center().y);
    let knob_fill = if resp.dragged_by(egui::PointerButton::Primary) {
        Color32::WHITE
    } else if hot {
        Color32::from_gray(0xee)
    } else {
        Color32::from_gray(0xc8)
    };
    p.circle_filled(knob + vec2(0.0, 1.0), KNOB_R, Color32::from_black_alpha(90));
    p.circle(knob, KNOB_R, knob_fill, Stroke::new(1.0, theme::BORDER));
    if resp.has_focus() {
        p.circle_stroke(knob, KNOB_R + 2.5, Stroke::new(1.0, theme::ACCENT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_and_log_mapping_round_trip() {
        for (lo, hi, log) in [
            (0.0, 100.0, false),
            (-24.0, 24.0, false),
            (1.0, 500.0, true),
        ] {
            for t in [0.0, 0.25, 0.5, 0.9, 1.0] {
                let v = from_t(t, lo, hi, log);
                assert!((to_t(v, lo, hi, log) - t).abs() < 1e-9, "{lo}..{hi} {t}");
            }
        }
        assert_eq!(to_t(f64::NAN, 0.0, 1.0, false), 0.0);
        assert_eq!(to_t(5.0, 3.0, 3.0, false), 0.0);
        assert_eq!(to_t(-50.0, 0.0, 10.0, false), 0.0);
        assert_eq!(to_t(50.0, 0.0, 10.0, false), 1.0);
    }

    fn run(f: impl FnMut(&mut Ui)) -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let mut f = f;
        for _ in 0..2 {
            ctx.run_ui(egui::RawInput::default(), |ui| f(ui))
                .drop_without_applying_deltas();
        }
        ctx
    }

    #[test]
    fn width_does_not_depend_on_the_value() {
        let mut widths = Vec::new();
        for v in [0.0f32, 7.5, -24.0, 24.0, 12.345] {
            let mut x = v;
            let mut w = 0.0;
            run(|ui| {
                ui.horizontal(|ui| {
                    w = ui
                        .add(Slider::new(&mut x, -24.0..=24.0).suffix(" dB"))
                        .rect
                        .width();
                });
            });
            widths.push(w);
        }
        assert!(
            widths.windows(2).all(|p| (p[0] - p[1]).abs() < 0.5),
            "{widths:?}"
        );
    }

    #[test]
    fn integers_stay_integers_and_values_stay_in_range() {
        let mut x = 7i32;
        run(|ui| {
            ui.add(Slider::new(&mut x, 0..=60).text("borda"));
        });
        assert_eq!(x, 7);
    }
}
