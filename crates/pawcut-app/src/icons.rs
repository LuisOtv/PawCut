//! Ícones vetoriais desenhados com o painter (sem depender de emoji na fonte).

use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, pos2, vec2};

use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Select,
    Blade,
    Magnet,
    Link,
    Lock,
    Unlock,
    Monitor,
    Speaker,
    Play,
    Pause,
    Stop,
    PrevFrame,
    NextFrame,
    ToStart,
    ToEnd,
    Reverse,
    Import,
    Plus,
    Minus,
    Fit,
    MediaPool,
    Effects,
    Inspector,
    DeliverPage,
    Insert,
    Overwrite,
    Append,
    Skim,
    Trim,
    Loop,
}

fn p(r: Rect, x: f32, y: f32) -> Pos2 {
    pos2(r.left() + r.width() * x, r.top() + r.height() * y)
}

/// Desenha o ícone dentro de `r` (quadrado) na cor `c`.
pub fn paint(painter: &Painter, r: Rect, icon: Icon, c: Color32) {
    let s = Stroke::new((r.width() / 11.0).max(1.2), c);
    let poly = |pts: Vec<Pos2>| Shape::convex_polygon(pts, c, Stroke::NONE);
    match icon {
        Icon::Loop => {
            // Seta em volta: um retângulo arredondado aberto com a ponta da seta.
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = std::f32::consts::TAU * (0.08 + 0.84 * i as f32 / 24.0);
                    p(r, 0.5 + 0.36 * a.cos(), 0.5 + 0.3 * a.sin())
                })
                .collect();
            let tip = *pts.last().expect("pontos");
            painter.add(Shape::line(pts, s));
            painter.add(poly(vec![
                tip + vec2(r.width() * 0.16, 0.0),
                tip + vec2(-r.width() * 0.04, -r.height() * 0.16),
                tip + vec2(-r.width() * 0.04, r.height() * 0.16),
            ]));
        }
        Icon::Trim => {
            // ][ : duas bordas se encontrando no corte.
            for (x, dir) in [(0.42, -1.0), (0.58, 1.0)] {
                painter.line_segment([p(r, x, 0.15), p(r, x, 0.85)], s);
                painter.line_segment([p(r, x, 0.15), p(r, x + 0.18 * dir, 0.15)], s);
                painter.line_segment([p(r, x, 0.85), p(r, x + 0.18 * dir, 0.85)], s);
            }
        }
        Icon::Skim => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.12, 0.28), p(r, 0.88, 0.78)),
                1.0,
                s,
                egui::StrokeKind::Middle,
            );
            painter.line_segment([p(r, 0.55, 0.08), p(r, 0.55, 0.95)], s);
            painter.add(poly(vec![
                p(r, 0.42, 0.08),
                p(r, 0.68, 0.08),
                p(r, 0.55, 0.22),
            ]));
        }
        Icon::Select => {
            painter.add(poly(vec![
                p(r, 0.25, 0.1),
                p(r, 0.25, 0.85),
                p(r, 0.45, 0.66),
                p(r, 0.75, 0.66),
            ]));
        }
        Icon::Blade => {
            painter.add(poly(vec![
                p(r, 0.55, 0.08),
                p(r, 0.78, 0.08),
                p(r, 0.62, 0.55),
                p(r, 0.42, 0.55),
            ]));
            painter.line_segment([p(r, 0.52, 0.55), p(r, 0.36, 0.92)], s);
        }
        Icon::Magnet => {
            let pts: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI * (i as f32 / 12.0);
                    p(r, 0.5 - 0.28 * a.cos(), 0.55 + 0.3 * a.sin())
                })
                .collect();
            painter.add(Shape::line(pts, Stroke::new(s.width * 1.6, c)));
            painter.line_segment(
                [p(r, 0.22, 0.55), p(r, 0.22, 0.15)],
                Stroke::new(s.width * 1.6, c),
            );
            painter.line_segment(
                [p(r, 0.78, 0.55), p(r, 0.78, 0.15)],
                Stroke::new(s.width * 1.6, c),
            );
        }
        Icon::Link => {
            let a = Rect::from_min_max(p(r, 0.08, 0.32), p(r, 0.58, 0.68));
            let b = Rect::from_min_max(p(r, 0.42, 0.32), p(r, 0.92, 0.68));
            painter.rect_stroke(a, r.height() * 0.18, s, egui::StrokeKind::Middle);
            painter.rect_stroke(b, r.height() * 0.18, s, egui::StrokeKind::Middle);
        }
        Icon::Lock | Icon::Unlock => {
            painter.rect_filled(Rect::from_min_max(p(r, 0.2, 0.45), p(r, 0.8, 0.9)), 2.0, c);
            let lift = if icon == Icon::Unlock { -0.12 } else { 0.0 };
            let pts: Vec<Pos2> = (0..=10)
                .map(|i| {
                    let a = std::f32::consts::PI * (i as f32 / 10.0);
                    p(
                        r,
                        0.5 - 0.2 * a.cos() + if icon == Icon::Unlock { 0.12 } else { 0.0 },
                        0.45 + lift - 0.3 * a.sin(),
                    )
                })
                .collect();
            painter.add(Shape::line(pts, s));
        }
        Icon::Monitor => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.1, 0.18), p(r, 0.9, 0.7)),
                1.5,
                s,
                egui::StrokeKind::Middle,
            );
            painter.line_segment([p(r, 0.5, 0.7), p(r, 0.5, 0.85)], s);
            painter.line_segment([p(r, 0.3, 0.86), p(r, 0.7, 0.86)], s);
        }
        Icon::Speaker => {
            painter.add(poly(vec![
                p(r, 0.12, 0.38),
                p(r, 0.3, 0.38),
                p(r, 0.52, 0.18),
                p(r, 0.52, 0.82),
                p(r, 0.3, 0.62),
                p(r, 0.12, 0.62),
            ]));
            painter.line_segment([p(r, 0.68, 0.32), p(r, 0.68, 0.68)], s);
            painter.line_segment([p(r, 0.84, 0.2), p(r, 0.84, 0.8)], s);
        }
        Icon::Play => {
            painter.add(poly(vec![
                p(r, 0.28, 0.15),
                p(r, 0.85, 0.5),
                p(r, 0.28, 0.85),
            ]));
        }
        Icon::Reverse => {
            painter.add(poly(vec![
                p(r, 0.72, 0.15),
                p(r, 0.72, 0.85),
                p(r, 0.15, 0.5),
            ]));
        }
        Icon::Pause => {
            painter.rect_filled(
                Rect::from_min_max(p(r, 0.24, 0.15), p(r, 0.42, 0.85)),
                1.0,
                c,
            );
            painter.rect_filled(
                Rect::from_min_max(p(r, 0.58, 0.15), p(r, 0.76, 0.85)),
                1.0,
                c,
            );
        }
        Icon::Stop => {
            painter.rect_filled(
                Rect::from_min_max(p(r, 0.22, 0.22), p(r, 0.78, 0.78)),
                1.0,
                c,
            );
        }
        Icon::PrevFrame => {
            painter.add(poly(vec![p(r, 0.7, 0.2), p(r, 0.7, 0.8), p(r, 0.3, 0.5)]));
            painter.rect_filled(Rect::from_min_max(p(r, 0.18, 0.2), p(r, 0.28, 0.8)), 0.0, c);
        }
        Icon::NextFrame => {
            painter.add(poly(vec![p(r, 0.3, 0.2), p(r, 0.7, 0.5), p(r, 0.3, 0.8)]));
            painter.rect_filled(Rect::from_min_max(p(r, 0.72, 0.2), p(r, 0.82, 0.8)), 0.0, c);
        }
        Icon::ToStart => {
            painter.rect_filled(Rect::from_min_max(p(r, 0.12, 0.2), p(r, 0.22, 0.8)), 0.0, c);
            painter.add(poly(vec![
                p(r, 0.55, 0.2),
                p(r, 0.55, 0.8),
                p(r, 0.22, 0.5),
            ]));
            painter.add(poly(vec![
                p(r, 0.88, 0.2),
                p(r, 0.88, 0.8),
                p(r, 0.55, 0.5),
            ]));
        }
        Icon::ToEnd => {
            painter.rect_filled(Rect::from_min_max(p(r, 0.78, 0.2), p(r, 0.88, 0.8)), 0.0, c);
            painter.add(poly(vec![
                p(r, 0.12, 0.2),
                p(r, 0.45, 0.5),
                p(r, 0.12, 0.8),
            ]));
            painter.add(poly(vec![
                p(r, 0.45, 0.2),
                p(r, 0.78, 0.5),
                p(r, 0.45, 0.8),
            ]));
        }
        Icon::Import => {
            painter.line_segment([p(r, 0.5, 0.1), p(r, 0.5, 0.6)], s);
            painter.add(poly(vec![
                p(r, 0.3, 0.45),
                p(r, 0.7, 0.45),
                p(r, 0.5, 0.68),
            ]));
            painter.add(Shape::line(
                vec![
                    p(r, 0.12, 0.6),
                    p(r, 0.12, 0.88),
                    p(r, 0.88, 0.88),
                    p(r, 0.88, 0.6),
                ],
                s,
            ));
        }
        Icon::Plus => {
            painter.line_segment([p(r, 0.2, 0.5), p(r, 0.8, 0.5)], s);
            painter.line_segment([p(r, 0.5, 0.2), p(r, 0.5, 0.8)], s);
        }
        Icon::Minus => {
            painter.line_segment([p(r, 0.2, 0.5), p(r, 0.8, 0.5)], s);
        }
        Icon::Fit => {
            painter.add(Shape::line(
                vec![
                    p(r, 0.3, 0.2),
                    p(r, 0.15, 0.2),
                    p(r, 0.15, 0.8),
                    p(r, 0.3, 0.8),
                ],
                s,
            ));
            painter.add(Shape::line(
                vec![
                    p(r, 0.7, 0.2),
                    p(r, 0.85, 0.2),
                    p(r, 0.85, 0.8),
                    p(r, 0.7, 0.8),
                ],
                s,
            ));
            painter.line_segment([p(r, 0.32, 0.5), p(r, 0.68, 0.5)], s);
        }
        Icon::MediaPool => {
            for (x, y) in [(0.12, 0.15), (0.54, 0.15), (0.12, 0.55), (0.54, 0.55)] {
                painter.rect_filled(
                    Rect::from_min_size(p(r, x, y), vec2(r.width() * 0.34, r.height() * 0.3)),
                    1.0,
                    c,
                );
            }
        }
        Icon::Effects => {
            let center = r.center();
            let pts: Vec<Pos2> = (0..10)
                .map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / 10.0 - std::f32::consts::FRAC_PI_2;
                    let rad = if i % 2 == 0 {
                        r.width() * 0.42
                    } else {
                        r.width() * 0.18
                    };
                    center + vec2(a.cos(), a.sin()) * rad
                })
                .collect();
            painter.add(Shape::closed_line(pts, s));
        }
        Icon::Inspector => {
            for (y, x) in [(0.25, 0.35), (0.5, 0.65), (0.75, 0.45)] {
                painter.line_segment([p(r, 0.1, y), p(r, 0.9, y)], s);
                painter.circle_filled(p(r, x, y), r.width() * 0.1, c);
            }
        }
        Icon::DeliverPage => {
            painter.add(poly(vec![
                p(r, 0.5, 0.08),
                p(r, 0.75, 0.4),
                p(r, 0.25, 0.4),
            ]));
            painter.rect_filled(Rect::from_min_max(p(r, 0.4, 0.4), p(r, 0.6, 0.66)), 0.0, c);
            painter.add(Shape::line(
                vec![
                    p(r, 0.12, 0.62),
                    p(r, 0.12, 0.9),
                    p(r, 0.88, 0.9),
                    p(r, 0.88, 0.62),
                ],
                s,
            ));
        }
        Icon::Insert => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.08, 0.3), p(r, 0.92, 0.7)),
                1.0,
                s,
                egui::StrokeKind::Middle,
            );
            painter.add(poly(vec![
                p(r, 0.35, 0.05),
                p(r, 0.65, 0.05),
                p(r, 0.5, 0.3),
            ]));
            painter.line_segment([p(r, 0.5, 0.3), p(r, 0.5, 0.7)], s);
        }
        Icon::Overwrite => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.08, 0.3), p(r, 0.92, 0.7)),
                1.0,
                s,
                egui::StrokeKind::Middle,
            );
            painter.rect_filled(Rect::from_min_max(p(r, 0.32, 0.3), p(r, 0.68, 0.7)), 0.0, c);
        }
        Icon::Append => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.08, 0.3), p(r, 0.62, 0.7)),
                1.0,
                s,
                egui::StrokeKind::Middle,
            );
            painter.rect_filled(Rect::from_min_max(p(r, 0.66, 0.3), p(r, 0.92, 0.7)), 0.0, c);
        }
    }
}

/// Botão só com ícone. `on`: estado ligado (ícone e fundo em destaque).
pub fn button(ui: &mut Ui, icon: Icon, size: f32, on: bool, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size + 8.0, size + 6.0), Sense::click());
    let bg = if on {
        Some(theme::ACCENT.gamma_multiply(0.25))
    } else if resp.hovered() {
        Some(theme::BAR_LIGHT)
    } else {
        None
    };
    if let Some(bg) = bg {
        ui.painter().rect_filled(rect, 3.0, bg);
    }
    let color = if on {
        theme::ACCENT
    } else if resp.hovered() {
        Color32::WHITE
    } else {
        theme::TEXT
    };
    paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), vec2(size, size)),
        icon,
        color,
    );
    if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    }
}
