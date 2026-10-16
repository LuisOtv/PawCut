//! Ícones das ferramentas, desenhados com formas simples (sem arquivos de imagem).

use egui::{Color32, Painter, Pos2, Rect, Stroke, pos2, vec2};

use crate::state::Tool;

fn at(r: Rect, x: f32, y: f32) -> Pos2 {
    pos2(r.left() + r.width() * x, r.top() + r.height() * y)
}

fn dashed(p: &Painter, a: Pos2, b: Pos2, st: Stroke) {
    let v = b - a;
    let n = (v.length() / 3.0).max(1.0) as usize;
    for i in (0..n).step_by(2) {
        let (t0, t1) = (i as f32 / n as f32, ((i + 1) as f32 / n as f32).min(1.0));
        p.line_segment([a + v * t0, a + v * t1], st);
    }
}

fn polyline(p: &Painter, pts: &[Pos2], st: Stroke) {
    for w in pts.windows(2) {
        p.line_segment([w[0], w[1]], st);
    }
}

pub fn paint_tool(p: &Painter, r: Rect, tool: Tool, c: Color32) {
    let st = Stroke::new(1.6, c);
    let thin = Stroke::new(1.2, c);
    match tool {
        Tool::Brush => {
            p.line_segment([at(r, 0.82, 0.12), at(r, 0.42, 0.58)], Stroke::new(2.4, c));
            p.circle_filled(at(r, 0.30, 0.72), r.width() * 0.2, c);
            p.line_segment([at(r, 0.12, 0.92), at(r, 0.34, 0.9)], thin);
        }
        Tool::Eraser => {
            let pts = [
                at(r, 0.12, 0.62),
                at(r, 0.50, 0.14),
                at(r, 0.90, 0.40),
                at(r, 0.52, 0.88),
            ];
            polyline(p, &[pts[0], pts[1], pts[2], pts[3], pts[0]], st);
            p.line_segment([at(r, 0.31, 0.38), at(r, 0.70, 0.64)], thin);
            p.line_segment([at(r, 0.50, 0.94), at(r, 0.90, 0.94)], thin);
        }
        Tool::Smudge => {
            for (i, y) in [0.3, 0.5, 0.7].into_iter().enumerate() {
                p.line_segment(
                    [at(r, 0.12, y), at(r, 0.55 + i as f32 * 0.1, y)],
                    Stroke::new(2.0, c.gamma_multiply(1.0 - i as f32 * 0.25)),
                );
            }
            p.circle_filled(at(r, 0.75, 0.5), r.width() * 0.17, c);
        }
        Tool::Fill => {
            polyline(
                p,
                &[
                    at(r, 0.22, 0.45),
                    at(r, 0.52, 0.12),
                    at(r, 0.82, 0.45),
                    at(r, 0.52, 0.80),
                    at(r, 0.22, 0.45),
                ],
                st,
            );
            p.line_segment([at(r, 0.22, 0.45), at(r, 0.82, 0.45)], thin);
            p.circle_filled(at(r, 0.86, 0.72), r.width() * 0.09, c);
            p.line_segment([at(r, 0.86, 0.56), at(r, 0.86, 0.68)], thin);
        }
        Tool::Gradient => {
            let rr = Rect::from_min_max(at(r, 0.1, 0.22), at(r, 0.9, 0.78));
            for i in 0..8 {
                let t = i as f32 / 7.0;
                let x0 = rr.left() + rr.width() * i as f32 / 8.0;
                p.rect_filled(
                    Rect::from_min_max(
                        pos2(x0, rr.top()),
                        pos2(x0 + rr.width() / 8.0 + 0.5, rr.bottom()),
                    ),
                    0.0,
                    c.gamma_multiply(0.15 + 0.85 * t),
                );
            }
            p.rect_stroke(rr, 1.0, thin, egui::StrokeKind::Outside);
        }
        Tool::Picker => {
            p.line_segment([at(r, 0.78, 0.18), at(r, 0.28, 0.68)], Stroke::new(2.6, c));
            p.circle_stroke(at(r, 0.82, 0.14), r.width() * 0.12, st);
            p.line_segment([at(r, 0.28, 0.68), at(r, 0.14, 0.88)], thin);
        }
        Tool::SelRect => {
            let (a, b, cc, d) = (
                at(r, 0.14, 0.2),
                at(r, 0.86, 0.2),
                at(r, 0.86, 0.8),
                at(r, 0.14, 0.8),
            );
            for (s, e) in [(a, b), (b, cc), (cc, d), (d, a)] {
                dashed(p, s, e, thin);
            }
        }
        Tool::SelEllipse => {
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = i as f32 / 24.0 * std::f32::consts::TAU;
                    at(r, 0.5 + 0.38 * t.cos(), 0.5 + 0.3 * t.sin())
                })
                .collect();
            for (i, w) in pts.windows(2).enumerate() {
                if i % 2 == 0 {
                    p.line_segment([w[0], w[1]], thin);
                }
            }
        }
        Tool::Lasso => {
            let pts = [
                at(r, 0.2, 0.4),
                at(r, 0.4, 0.15),
                at(r, 0.75, 0.25),
                at(r, 0.85, 0.55),
                at(r, 0.6, 0.8),
                at(r, 0.3, 0.7),
                at(r, 0.2, 0.4),
            ];
            for w in pts.windows(2) {
                dashed(p, w[0], w[1], thin);
            }
            p.line_segment([at(r, 0.3, 0.7), at(r, 0.16, 0.92)], thin);
        }
        Tool::Wand => {
            p.line_segment([at(r, 0.16, 0.86), at(r, 0.6, 0.42)], Stroke::new(2.2, c));
            for (dx, dy) in [(0.0, -0.17), (0.17, 0.0), (0.12, -0.12), (-0.12, -0.12)] {
                p.line_segment([at(r, 0.72, 0.3), at(r, 0.72 + dx, 0.3 + dy)], thin);
            }
            p.circle_filled(at(r, 0.72, 0.3), 1.6, c);
        }
        Tool::Move => {
            let ctr = at(r, 0.5, 0.5);
            for (dx, dy) in [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
                let tip = ctr + vec2(dx, dy) * r.width() * 0.42;
                p.line_segment([ctr, tip], st);
                let side = vec2(-dy, dx) * 3.0;
                p.line_segment([tip, tip - vec2(dx, dy) * 4.0 + side], st);
                p.line_segment([tip, tip - vec2(dx, dy) * 4.0 - side], st);
            }
        }
        Tool::Transform => {
            let rr = Rect::from_min_max(at(r, 0.22, 0.3), at(r, 0.78, 0.86));
            p.rect_stroke(rr, 0.0, thin, egui::StrokeKind::Outside);
            for q in [
                rr.left_top(),
                rr.right_top(),
                rr.left_bottom(),
                rr.right_bottom(),
            ] {
                p.rect_filled(Rect::from_center_size(q, vec2(3.5, 3.5)), 0.0, c);
            }
            p.line_segment([rr.center_top(), at(r, 0.5, 0.1)], thin);
            p.circle_filled(at(r, 0.5, 0.1), 2.0, c);
        }
        Tool::Crop => {
            polyline(
                p,
                &[at(r, 0.28, 0.1), at(r, 0.28, 0.72), at(r, 0.9, 0.72)],
                st,
            );
            polyline(
                p,
                &[at(r, 0.1, 0.28), at(r, 0.72, 0.28), at(r, 0.72, 0.9)],
                st,
            );
        }
        Tool::Line => {
            p.line_segment([at(r, 0.14, 0.86), at(r, 0.86, 0.14)], st);
            p.circle_filled(at(r, 0.14, 0.86), 2.0, c);
            p.circle_filled(at(r, 0.86, 0.14), 2.0, c);
        }
        Tool::Rect => {
            p.rect_stroke(
                Rect::from_min_max(at(r, 0.16, 0.24), at(r, 0.84, 0.76)),
                0.0,
                st,
                egui::StrokeKind::Outside,
            );
        }
        Tool::Ellipse => {
            p.add(egui::epaint::EllipseShape::stroke(
                at(r, 0.5, 0.5),
                vec2(r.width() * 0.36, r.height() * 0.28),
                st,
            ));
        }
        Tool::Text => {
            p.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "T",
                paw_ui::theme::text(r.height() * 0.85),
                c,
            );
        }
    }
}
