//! Avisos que aparecem no canto de baixo e somem sozinhos (erros ficam mais tempo).

use std::time::{Duration, Instant};

use egui::Color32;

use crate::theme;

#[derive(Default)]
pub struct Toasts {
    items: Vec<(String, bool, Instant)>,
}

impl Toasts {
    /// Mostra `text` (erro em vermelho). O mesmo texto não aparece duas vezes.
    pub fn push(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        self.items.retain(|t| t.0 != text);
        self.items.push((text, error, Instant::now()));
        if self.items.len() > 4 {
            self.items.drain(..self.items.len() - 4);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Tira os vencidos (4 s; erros 8 s) e pede repaint enquanto houver algum.
    pub fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.items.retain(|(_, error, born)| {
            now.duration_since(*born) < Duration::from_secs(if *error { 8 } else { 4 })
        });
        if !self.items.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Desenha por cima de tudo; clicar num aviso fecha.
    pub fn ui(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        let mut y = screen.bottom() - 16.0;
        let mut close = None;
        for (i, (text, error, born)) in self.items.iter().enumerate().rev() {
            let age = born.elapsed().as_secs_f32();
            let life = if *error { 8.0 } else { 4.0 };
            let alpha = ((life - age) / 0.4).clamp(0.0, 1.0) * (age / 0.15).clamp(0.0, 1.0);
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new(("toast", i)),
            ));
            let galley = painter.layout(text.clone(), theme::text(12.5), Color32::WHITE, 360.0);
            let size = galley.size() + egui::vec2(28.0, 16.0);
            let rect = egui::Rect::from_min_size(
                egui::pos2(screen.right() - size.x - 16.0, y - size.y),
                size,
            );
            let bg = if *error {
                Color32::from_rgb(110, 30, 34)
            } else {
                theme::BAR_LIGHT
            };
            painter.rect_filled(rect, 6.0, bg.gamma_multiply(alpha));
            painter.rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(4.0, rect.height())),
                2.0,
                (if *error {
                    Color32::from_rgb(240, 90, 80)
                } else {
                    theme::ACCENT
                })
                .gamma_multiply(alpha),
            );
            painter.galley(
                rect.min + egui::vec2(16.0, 8.0),
                galley,
                Color32::WHITE.gamma_multiply(alpha),
            );
            if ctx.input(|inp| {
                inp.pointer.primary_clicked()
                    && inp.pointer.interact_pos().is_some_and(|p| rect.contains(p))
            }) {
                close = Some(i);
            }
            y = rect.top() - 8.0;
        }
        if let Some(i) = close {
            self.items.remove(i);
        }
    }
}
