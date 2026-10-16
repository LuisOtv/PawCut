//! A tela: abas dos documentos, desenho da imagem e o que cada ferramenta faz com o mouse.

use egui::{
    Align2, Color32, CursorIcon, Key, Mesh, Pos2, Rect, Sense, Shape, Stroke, TextureHandle,
    TextureOptions, Vec2, pos2, vec2,
};
use paw_ui::theme;
use pawpaint_core::composite::composite_tile;
use pawpaint_core::fill::GradientKind;
use pawpaint_core::selection::{SelOp, Selection};
use pawpaint_core::tile::TILE;
use pawpaint_core::{Dirty, Editor};

use crate::state::{DocView, FloatState, Interaction, State, Tool};

const HANDLE: f32 = 6.0;

// ---------------------------------------------------------------- textura

fn tile_image(tile: &[u8], tw: usize, th: usize) -> egui::ColorImage {
    let mut px = Vec::with_capacity(tw * th * 4);
    for y in 0..th {
        px.extend_from_slice(&tile[y * TILE * 4..y * TILE * 4 + tw * 4]);
    }
    egui::ColorImage::from_rgba_premultiplied([tw, th], &px)
}

/// Põe na textura o que mudou no documento desde o último quadro.
pub fn sync_texture(ctx: &egui::Context, d: &mut DocView) {
    let (w, h) = (d.ed.doc().width, d.ed.doc().height);
    let dirty = d.ed.take_dirty();
    let need_full = d.tex.is_none() || d.tex_size != (w, h) || dirty == Dirty::All;
    if need_full {
        let mut img = egui::ColorImage::filled([w, h], Color32::TRANSPARENT);
        let live = d.ed.live();
        for ty in 0..h.div_ceil(TILE) {
            for tx in 0..w.div_ceil(TILE) {
                let t = composite_tile(d.ed.doc(), tx, ty, live.as_ref());
                let (tw, th) = (TILE.min(w - tx * TILE), TILE.min(h - ty * TILE));
                let ti = tile_image(&t, tw, th);
                for y in 0..th {
                    let dst = (ty * TILE + y) * w + tx * TILE;
                    img.pixels[dst..dst + tw].copy_from_slice(&ti.pixels[y * tw..(y + 1) * tw]);
                }
            }
        }
        match &mut d.tex {
            Some(t) if d.tex_size == (w, h) => t.set(img, TextureOptions::NEAREST),
            _ => {
                d.tex =
                    Some(ctx.load_texture(format!("canvas{}", d.uid), img, TextureOptions::NEAREST))
            }
        }
        d.tex_size = (w, h);
    } else if let Dirty::Tiles(set) = dirty {
        let live = d.ed.live();
        let Some(tex) = d.tex.as_mut() else { return };
        for (tx, ty) in set {
            if tx * TILE >= w || ty * TILE >= h {
                continue;
            }
            let t = composite_tile(d.ed.doc(), tx, ty, live.as_ref());
            let (tw, th) = (TILE.min(w - tx * TILE), TILE.min(h - ty * TILE));
            tex.set_partial(
                [tx * TILE, ty * TILE],
                tile_image(&t, tw, th),
                TextureOptions::NEAREST,
            );
        }
    }
}

fn checker(ctx: &egui::Context) -> TextureHandle {
    let id = egui::Id::new("paw-checker");
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return t;
    }
    let mut img = egui::ColorImage::filled([16, 16], Color32::from_gray(200));
    for y in 0..16 {
        for x in 0..16 {
            if (x / 8 + y / 8) % 2 == 1 {
                img.pixels[y * 16 + x] = Color32::from_gray(150);
            }
        }
    }
    let opts = TextureOptions {
        wrap_mode: egui::TextureWrapMode::Repeat,
        ..TextureOptions::NEAREST
    };
    let t = ctx.load_texture("paw-checker", img, opts);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

/// Contorno da seleção como textura: pontos pretos e brancos alternados na borda.
fn selection_overlay(ctx: &egui::Context, d: &mut DocView) {
    let rev = d.ed.revision();
    if d.sel_overlay.as_ref().is_some_and(|(r, _, _)| *r == rev) {
        return;
    }
    d.sel_overlay = None;
    let Some(sel) = &d.ed.doc().selection else {
        return;
    };
    let Some((x0, y0, x1, y1)) = sel.bounds() else {
        return;
    };
    let (w, h) = (x1 - x0 + 2, y1 - y0 + 2);
    let mut img = egui::ColorImage::filled([w, h], Color32::TRANSPARENT);
    let inside = |x: i64, y: i64| x >= 0 && y >= 0 && sel.value(x as usize, y as usize) >= 128;
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x0 as i64 + x as i64 - 1, y0 as i64 + y as i64 - 1);
            if !inside(px, py) {
                continue;
            }
            if !(inside(px - 1, py)
                && inside(px + 1, py)
                && inside(px, py - 1)
                && inside(px, py + 1))
            {
                img.pixels[y * w + x] = if (px + py).div_euclid(3) % 2 == 0 {
                    Color32::BLACK
                } else {
                    Color32::WHITE
                };
            }
        }
    }
    let t = ctx.load_texture(format!("sel{}", d.uid), img, TextureOptions::NEAREST);
    d.sel_overlay = Some((rev, t, (x0.saturating_sub(1), y0.saturating_sub(1))));
}

// ---------------------------------------------------------------- pedaço solto

fn start_float(ctx: &egui::Context, d: &mut DocView, quick: bool) -> bool {
    let id = d.active;
    let Ok(f) = d.ed.begin_float(id) else {
        return false;
    };
    let (x0, y0, x1, y1) = f.bounds;
    let img = f.pixels.region_to_rgba(x0, y0, x1, y1);
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [img.width() as usize, img.height() as usize],
        img.as_raw(),
    );
    let tex = ctx.load_texture(format!("float{}", d.uid), color, TextureOptions::LINEAR);
    let center = ((x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0);
    d.float = Some(FloatState {
        f,
        tex,
        center,
        sx: 1.0,
        sy: 1.0,
        rotation: 0.0,
        dx: 0.0,
        dy: 0.0,
        quick,
    });
    true
}

pub fn commit_float(d: &mut DocView) {
    if let Some(fs) = d.float.take() {
        d.ed.finish_float(&fs.f, &fs.matrix());
    }
}

pub fn abort_float(d: &mut DocView) {
    if let Some(fs) = d.float.take() {
        d.ed.abort_float(&fs.f);
    }
}

/// Os cantos do pedaço solto, transformados, em coordenadas da imagem.
fn float_corners(fs: &FloatState) -> [Pos2; 4] {
    let (x0, y0, x1, y1) = fs.f.bounds;
    let m = fs.matrix();
    [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| {
        let (a, b) = m.apply(x as f32, y as f32);
        pos2(a, b)
    })
}

// ---------------------------------------------------------------- painel

pub fn canvas_panel(st: &mut State, ui: &mut egui::Ui) {
    doc_tabs(st, ui);
    let area = ui.available_rect_before_wrap();
    ui.painter()
        .rect_filled(area, 0.0, Color32::from_rgb(0x2b, 0x2b, 0x2f));
    if st.docs.is_empty() {
        ui.painter().text(
            area.center(),
            Align2::CENTER_CENTER,
            "Ctrl+N cria uma imagem · Ctrl+O abre · ou arraste um arquivo para cá",
            theme::text(14.0),
            theme::TEXT_DIM,
        );
        return;
    }
    let ctx = ui.ctx().clone();
    let cur = st.cur;
    if std::mem::take(&mut st.tool_changed)
        && st.tool == Tool::Transform
        && st.docs[cur].float.is_none()
        && !start_float(&ctx, &mut st.docs[cur], false)
    {
        st.warn("Nada para transformar nesta camada");
    }
    sync_texture(&ctx, &mut st.docs[cur]);
    selection_overlay(&ctx, &mut st.docs[cur]);
    let (cw, ch) = (
        st.docs[cur].ed.doc().width as f32,
        st.docs[cur].ed.doc().height as f32,
    );
    if st.docs[cur].fit {
        st.docs[cur].zoom = ((area.width() - 40.0) / cw)
            .min((area.height() - 40.0) / ch)
            .clamp(0.02, 1.0)
            .max(0.02);
    }
    let resp = ui.interact(
        area,
        ui.id().with(("canvas", st.docs[cur].uid)),
        Sense::click_and_drag(),
    );
    // Roda do mouse: zoom em volta do cursor.
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y + (i.zoom_delta() - 1.0) * 300.0);
        if scroll.abs() > 0.1 {
            let d = &mut st.docs[cur];
            let old = d.zoom;
            let new = (old * (1.0 + scroll * 0.0025)).clamp(0.02, 32.0);
            if let Some(p) = resp.hover_pos() {
                let center = area.center() + d.pan;
                let before = (p - center) / old;
                d.pan += before * old - before * new;
            }
            d.zoom = new;
            d.fit = false;
        }
    }
    let d = &st.docs[cur];
    let size = vec2(cw, ch) * d.zoom;
    let origin = area.center() + d.pan - size / 2.0;
    let canvas = Rect::from_min_size(origin, size);
    let zoom = d.zoom;
    let to_doc = move |p: Pos2| ((p - origin) / zoom).to_pos2();
    let to_screen = move |x: f32, y: f32| origin + vec2(x, y) * zoom;
    let clip = ui.painter().with_clip_rect(area);

    // ---- desenho
    let checker_tex = checker(&ctx);
    clip.image(
        checker_tex.id(),
        canvas,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(cw / 16.0, ch / 16.0)),
        Color32::WHITE,
    );
    if let Some(t) = &d.tex {
        clip.image(
            t.id(),
            canvas,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    if let Some(fs) = &d.float {
        let (x0, y0, x1, y1) = fs.f.bounds;
        let _ = (x0, y0, x1, y1);
        let c = float_corners(fs);
        let mut mesh = Mesh::with_texture(fs.tex.id());
        let uvs = [
            pos2(0.0, 0.0),
            pos2(1.0, 0.0),
            pos2(1.0, 1.0),
            pos2(0.0, 1.0),
        ];
        for (p, uv) in c.iter().zip(uvs) {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: to_screen(p.x, p.y),
                uv,
                color: Color32::WHITE,
            });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        clip.add(Shape::mesh(mesh));
    } else if let Some((_, tex, (sx, sy))) = &d.sel_overlay {
        let r = Rect::from_min_size(to_screen(*sx as f32, *sy as f32), tex.size_vec2() * zoom);
        clip.image(
            tex.id(),
            r,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    clip.rect_stroke(
        canvas,
        0.0,
        Stroke::new(1.0, Color32::BLACK),
        egui::StrokeKind::Outside,
    );

    // Eixos do espelho.
    if st.mirror.0 {
        let x = canvas.center().x;
        clip.line_segment(
            [pos2(x, canvas.top()), pos2(x, canvas.bottom())],
            Stroke::new(1.0, Color32::from_rgb(255, 80, 200)),
        );
    }
    if st.mirror.1 {
        let y = canvas.center().y;
        clip.line_segment(
            [pos2(canvas.left(), y), pos2(canvas.right(), y)],
            Stroke::new(1.0, Color32::from_rgb(255, 80, 200)),
        );
    }
    // Corte em andamento: escurece o que sai.
    if let Some((x0, y0, x1, y1)) = d.crop {
        let r = Rect::from_two_pos(to_screen(x0, y0), to_screen(x1, y1));
        let dim = Color32::from_black_alpha(140);
        clip.rect_filled(
            Rect::from_min_max(canvas.min, pos2(canvas.right(), r.top())),
            0.0,
            dim,
        );
        clip.rect_filled(
            Rect::from_min_max(pos2(canvas.left(), r.bottom()), canvas.max),
            0.0,
            dim,
        );
        clip.rect_filled(
            Rect::from_min_max(pos2(canvas.left(), r.top()), pos2(r.left(), r.bottom())),
            0.0,
            dim,
        );
        clip.rect_filled(
            Rect::from_min_max(pos2(r.right(), r.top()), pos2(canvas.right(), r.bottom())),
            0.0,
            dim,
        );
        clip.rect_stroke(
            r,
            0.0,
            Stroke::new(1.5, Color32::WHITE),
            egui::StrokeKind::Outside,
        );
        clip.text(
            r.left_top() + vec2(4.0, -14.0),
            Align2::LEFT_TOP,
            format!(
                "{} × {}  (Enter corta, Esc cancela)",
                (x1 - x0).abs().round(),
                (y1 - y0).abs().round()
            ),
            theme::text(11.0),
            Color32::WHITE,
        );
    }

    // Caixa e alças do pedaço solto (só no Transformar).
    let mut handles: Option<(Pos2, [Pos2; 4], Pos2)> = None;
    if let Some(fs) = &d.float
        && !fs.quick
    {
        let c = float_corners(fs).map(|p| to_screen(p.x, p.y));
        let st_ = Stroke::new(1.5, theme::ACCENT);
        for i in 0..4 {
            clip.line_segment([c[i], c[(i + 1) % 4]], st_);
        }
        let top_mid = c[0].lerp(c[1], 0.5);
        let out = (top_mid - c[0].lerp(c[2], 0.5)).normalized();
        let rot = top_mid + out * 28.0;
        clip.line_segment([top_mid, rot], st_);
        for p in c {
            clip.rect_filled(
                Rect::from_center_size(p, Vec2::splat(HANDLE * 2.0)),
                2.0,
                Color32::WHITE,
            );
            clip.rect_stroke(
                Rect::from_center_size(p, Vec2::splat(HANDLE * 2.0)),
                2.0,
                st_,
                egui::StrokeKind::Inside,
            );
        }
        clip.circle_filled(rot, HANDLE, Color32::WHITE);
        clip.circle_stroke(rot, HANDLE, st_);
        let center = c[0].lerp(c[2], 0.5);
        handles = Some((center, c, rot));
    }

    // Prévias das ferramentas em andamento.
    match &st.interaction {
        Some(Interaction::Select {
            tool,
            start,
            cur,
            pts,
            ..
        }) => {
            let st_a = Stroke::new(1.0, Color32::WHITE);
            let st_b = Stroke::new(1.0, Color32::BLACK);
            match tool {
                Tool::SelRect => {
                    let r =
                        Rect::from_two_pos(to_screen(start.0, start.1), to_screen(cur.0, cur.1));
                    clip.rect_stroke(r, 0.0, st_b, egui::StrokeKind::Middle);
                    clip.rect_stroke(r.shrink(1.0), 0.0, st_a, egui::StrokeKind::Middle);
                }
                Tool::SelEllipse => {
                    let r =
                        Rect::from_two_pos(to_screen(start.0, start.1), to_screen(cur.0, cur.1));
                    clip.add(egui::epaint::EllipseShape::stroke(
                        r.center(),
                        r.size() / 2.0,
                        st_b,
                    ));
                    clip.add(egui::epaint::EllipseShape::stroke(
                        r.center(),
                        (r.size() / 2.0 - vec2(1.0, 1.0)).max(Vec2::ZERO),
                        st_a,
                    ));
                }
                _ => {
                    let sp: Vec<Pos2> = pts.iter().map(|p| to_screen(p.0, p.1)).collect();
                    clip.add(Shape::line(sp.clone(), st_b));
                    if let (Some(a), Some(b)) = (sp.first(), sp.last()) {
                        clip.line_segment(
                            [*a, *b],
                            Stroke::new(1.0, Color32::from_white_alpha(120)),
                        );
                    }
                }
            }
        }
        Some(Interaction::Shape {
            tool,
            start,
            cur,
            shift,
        }) => {
            let cur = if *shift && *tool != Tool::Line {
                square_to(*start, *cur)
            } else {
                *cur
            };
            let st_ = Stroke::new(1.5, Color32::from_rgb(255, 80, 200));
            match tool {
                Tool::Line => {
                    clip.line_segment([to_screen(start.0, start.1), to_screen(cur.0, cur.1)], st_);
                }
                Tool::Rect => {
                    clip.rect_stroke(
                        Rect::from_two_pos(to_screen(start.0, start.1), to_screen(cur.0, cur.1)),
                        0.0,
                        st_,
                        egui::StrokeKind::Middle,
                    );
                }
                _ => {
                    let r =
                        Rect::from_two_pos(to_screen(start.0, start.1), to_screen(cur.0, cur.1));
                    clip.add(egui::epaint::EllipseShape::stroke(
                        r.center(),
                        r.size() / 2.0,
                        st_,
                    ));
                }
            }
        }
        Some(Interaction::Gradient { start, cur }) => {
            let (a, b) = (to_screen(start.0, start.1), to_screen(cur.0, cur.1));
            clip.line_segment([a, b], Stroke::new(2.0, Color32::BLACK));
            clip.line_segment([a, b], Stroke::new(1.0, Color32::WHITE));
            clip.circle_filled(a, 4.0, Color32::WHITE);
            clip.circle_filled(b, 4.0, Color32::BLACK);
        }
        _ => {}
    }

    // ---- entrada
    let pos = ui.input(|i| i.pointer.interact_pos()).or(resp.hover_pos());
    let (pressed, down, released, mods, middle, space) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            i.modifiers,
            i.pointer.middle_down(),
            i.key_down(Key::Space),
        )
    });
    let doc_pos = pos.map(to_doc);
    st.docs[cur].cursor = doc_pos.map(|p| (p.x, p.y));
    let over = resp.hovered() && st.text_input.is_none();

    if st.interaction.is_none() && ((pressed && over) || (middle && resp.hovered())) {
        if let Some(p) = pos {
            start_interaction(st, &ctx, p, to_doc(p), mods, middle || space, handles);
        }
    } else if st.interaction.is_some() {
        if let Some(p) = pos {
            update_interaction(st, to_doc(p), p, mods, handles, resp.drag_delta(), &ctx);
        }
        if released || (!down && !middle) {
            let p = pos.map_or((0.0, 0.0), |p| {
                let q = to_doc(p);
                (q.x, q.y)
            });
            finish_interaction(st, p, mods);
        }
    }

    // Cursor: círculo do pincel nas ferramentas de pintura.
    if over && st.interaction.is_none() || matches!(st.interaction, Some(Interaction::Stroke)) {
        if let (Some(p), true) = (pos, st.tool.paints() || st.tool == Tool::Eraser) {
            if resp.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::None);
                let r = (st.brush.size * zoom / 2.0).max(1.5);
                clip.circle_stroke(p, r, Stroke::new(1.0, Color32::BLACK));
                clip.circle_stroke(p, r + 1.0, Stroke::new(1.0, Color32::WHITE));
                clip.circle_filled(p, 1.0, Color32::WHITE);
            }
        } else if resp.hovered() {
            let over_handle = handles.zip(pos).is_some_and(|((_, c, rot), p)| {
                p.distance(rot) <= HANDLE + 4.0 || c.iter().any(|q| p.distance(*q) <= HANDLE + 4.0)
            });
            ui.ctx().set_cursor_icon(match (st.tool, over_handle) {
                (_, true) => CursorIcon::Crosshair,
                (
                    Tool::Picker
                    | Tool::Fill
                    | Tool::Wand
                    | Tool::Gradient
                    | Tool::Crop
                    | Tool::SelRect
                    | Tool::SelEllipse
                    | Tool::Lasso,
                    _,
                ) => CursorIcon::Crosshair,
                (Tool::Move, _) => CursorIcon::Grab,
                (Tool::Text, _) => CursorIcon::Text,
                _ => CursorIcon::Default,
            });
        }
    }
    st.docs[cur].view_origin = origin;
    text_popup(st, ui, &to_screen);
    if space && resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
}

fn square_to(start: (f32, f32), cur: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (cur.0 - start.0, cur.1 - start.1);
    let m = dx.abs().max(dy.abs());
    (start.0 + m * dx.signum(), start.1 + m * dy.signum())
}

fn doc_tabs(st: &mut State, ui: &mut egui::Ui) {
    let mut close = None;
    let mut select = None;
    egui::Frame::new()
        .fill(theme::BAR)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, d) in st.docs.iter().enumerate() {
                    let selected = i == st.cur;
                    if ui.selectable_label(selected, d.title()).clicked() {
                        select = Some(i);
                    }
                    if ui
                        .add(egui::Button::new("×").frame(false).small())
                        .on_hover_text("Fechar o documento")
                        .clicked()
                    {
                        close = Some(i);
                    }
                    ui.add_space(6.0);
                }
            });
        });
    if let Some(i) = select {
        switch_doc(st, i);
    }
    if let Some(i) = close {
        crate::app::request_close_doc(st, i);
    }
}

pub fn switch_doc(st: &mut State, i: usize) {
    if i != st.cur
        && let Some(d) = st.docs.get_mut(st.cur)
        && d.float.is_some()
    {
        commit_float(d);
    }
    st.cur = i.min(st.docs.len().saturating_sub(1));
    st.interaction = None;
}

// ---------------------------------------------------------------- ferramentas

fn sel_op(st: &State, mods: egui::Modifiers) -> SelOp {
    match (mods.shift, mods.alt) {
        (true, true) => SelOp::Intersect,
        (true, false) => SelOp::Add,
        (false, true) => SelOp::Subtract,
        _ => st.opts.sel_op,
    }
}

fn start_interaction(
    st: &mut State,
    ctx: &egui::Context,
    screen: Pos2,
    p: Pos2,
    mods: egui::Modifiers,
    pan: bool,
    handles: Option<(Pos2, [Pos2; 4], Pos2)>,
) {
    let cur = st.cur;
    let pt = (p.x, p.y);
    if pan {
        st.interaction = Some(Interaction::Pan);
        return;
    }
    // Ctrl+clique com ferramentas de pintura pega a cor (como no Krita).
    if mods.command
        && matches!(
            st.tool,
            Tool::Brush
                | Tool::Eraser
                | Tool::Fill
                | Tool::Gradient
                | Tool::Line
                | Tool::Rect
                | Tool::Ellipse
        )
    {
        st.interaction = Some(Interaction::Picker);
        pick_color(st, pt, mods.shift);
        return;
    }
    let active = st.docs[cur].active;
    let (w, h) = (
        st.docs[cur].ed.doc().width as f32,
        st.docs[cur].ed.doc().height as f32,
    );
    let inside = pt.0 >= 0.0 && pt.1 >= 0.0 && pt.0 < w && pt.1 < h;
    // Transformar: alças primeiro.
    if st.tool == Tool::Transform
        && let Some(fs) = &st.docs[cur].float
        && let Some((center, corners, rot)) = handles
    {
        if screen.distance(rot) <= HANDLE + 4.0 {
            st.interaction = Some(Interaction::FloatRotate);
            return;
        }
        if corners.iter().any(|c| screen.distance(*c) <= HANDLE + 4.0) {
            st.interaction = Some(Interaction::FloatScale {
                start_dist: screen.distance(center).max(1.0),
                sx: fs.sx,
                sy: fs.sy,
            });
            return;
        }
        st.interaction = Some(Interaction::FloatMove {
            grab: pt,
            orig: (fs.dx, fs.dy),
        });
        return;
    }
    match st.tool {
        Tool::Brush | Tool::Eraser => {
            let erase = st.tool == Tool::Eraser;
            let mode = if erase {
                pawpaint_core::BlendMode::Normal
            } else {
                st.blend
            };
            let (brush, color) = (st.brush.clone(), st.fg);
            st.docs[cur].ed.set_mirror(st.mirror.0, st.mirror.1);
            match st.docs[cur]
                .ed
                .begin_stroke(active, brush, color, erase, mode)
            {
                Ok(()) => {
                    st.docs[cur].ed.stroke_point(pt.0, pt.1);
                    st.interaction = Some(Interaction::Stroke);
                }
                Err(e) => st.warn(e.to_string()),
            }
        }
        Tool::Fill => {
            if inside {
                let (tol, contig, merged, op, c) = (
                    st.opts.fill_tolerance,
                    st.opts.contiguous,
                    st.opts.sample_merged,
                    st.opts.fill_opacity,
                    st.fg,
                );
                if let Err(e) = st.docs[cur].ed.bucket_fill(
                    active,
                    (pt.0 as usize, pt.1 as usize),
                    c,
                    tol,
                    contig,
                    op,
                    merged,
                ) {
                    st.warn(e.to_string());
                }
            }
        }
        Tool::Gradient => st.interaction = Some(Interaction::Gradient { start: pt, cur: pt }),
        Tool::Picker => {
            st.interaction = Some(Interaction::Picker);
            pick_color(st, pt, mods.shift);
        }
        Tool::SelRect | Tool::SelEllipse | Tool::Lasso => {
            let op = sel_op(st, mods);
            st.interaction = Some(Interaction::Select {
                tool: st.tool,
                start: pt,
                cur: pt,
                pts: vec![pt],
                op,
            });
        }
        Tool::Wand => {
            if inside {
                let op = sel_op(st, mods);
                let (tol, contig, merged) = (
                    st.opts.wand_tolerance,
                    st.opts.contiguous,
                    st.opts.sample_merged,
                );
                st.docs[cur].ed.magic_wand(
                    active,
                    (pt.0 as usize, pt.1 as usize),
                    tol,
                    contig,
                    op,
                    merged,
                );
            }
        }
        Tool::Move => {
            if st.docs[cur].float.is_none() && !start_float(ctx, &mut st.docs[cur], true) {
                st.warn("Nada para mover nesta camada");
                return;
            }
            let fs = st.docs[cur].float.as_ref().expect("solto");
            st.interaction = Some(Interaction::FloatMove {
                grab: pt,
                orig: (fs.dx, fs.dy),
            });
        }
        Tool::Transform => {
            if st.docs[cur].float.is_none() && !start_float(ctx, &mut st.docs[cur], false) {
                st.warn("Nada para transformar nesta camada");
            }
        }
        Tool::Crop => {
            st.docs[cur].crop = Some((pt.0, pt.1, pt.0, pt.1));
            st.interaction = Some(Interaction::Crop { start: pt });
        }
        Tool::Line | Tool::Rect | Tool::Ellipse => {
            st.interaction = Some(Interaction::Shape {
                tool: st.tool,
                start: pt,
                cur: pt,
                shift: mods.shift,
            });
        }
        Tool::Text => {
            if let Some((tp, text)) = st.text_input.take() {
                commit_text(st, tp, &text);
            }
            if inside {
                st.text_input = Some((pt, String::new()));
            }
        }
    }
}

fn pick_color(st: &mut State, pt: (f32, f32), to_bg: bool) {
    let Some(d) = st.doc() else { return };
    if pt.0 < 0.0 || pt.1 < 0.0 {
        return;
    }
    let c =
        d.ed.pick_color(pt.0 as usize, pt.1 as usize, true, Some(d.active));
    if c[3] == 0 {
        return;
    }
    let rgb = [c[0], c[1], c[2]];
    if to_bg {
        st.bg = rgb;
    } else {
        st.fg = rgb;
        st.hsv = crate::state::rgb_to_hsv(rgb);
    }
}

fn update_interaction(
    st: &mut State,
    p: Pos2,
    screen: Pos2,
    mods: egui::Modifiers,
    handles: Option<(Pos2, [Pos2; 4], Pos2)>,
    delta: Vec2,
    ctx: &egui::Context,
) {
    let cur = st.cur;
    let pt = (p.x, p.y);
    let Some(inter) = st.interaction.as_mut() else {
        return;
    };
    match inter {
        Interaction::Stroke => st.docs[cur].ed.stroke_point(pt.0, pt.1),
        Interaction::Pan => {
            let d = &mut st.docs[cur];
            d.pan += ctx.input(|i| i.pointer.delta());
            d.fit = false;
            let _ = delta;
        }
        Interaction::Picker => {
            let to_bg = mods.shift;
            pick_color(st, pt, to_bg);
        }
        Interaction::Gradient { cur: c, .. } => *c = pt,
        Interaction::Select {
            cur: c, pts, tool, ..
        } => {
            *c = pt;
            if *tool == Tool::Lasso
                && pts
                    .last()
                    .is_none_or(|l| (l.0 - pt.0).hypot(l.1 - pt.1) > 1.5)
            {
                pts.push(pt);
            }
        }
        Interaction::Shape { cur: c, shift, .. } => {
            *c = pt;
            *shift = mods.shift;
        }
        Interaction::Crop { start } => {
            let s = *start;
            st.docs[cur].crop = Some((s.0, s.1, pt.0, pt.1));
        }
        Interaction::FloatMove { grab, orig } => {
            let (g, o) = (*grab, *orig);
            if let Some(fs) = &mut st.docs[cur].float {
                fs.dx = (o.0 + pt.0 - g.0).round();
                fs.dy = (o.1 + pt.1 - g.1).round();
            }
        }
        Interaction::FloatScale { start_dist, sx, sy } => {
            let (sd, osx, osy) = (*start_dist, *sx, *sy);
            if let (Some(fs), Some((center, _, _))) = (&mut st.docs[cur].float, handles) {
                let k = (screen.distance(center) / sd).max(0.02);
                fs.sx = (osx * k).clamp(0.02, 40.0);
                fs.sy = (osy * k).clamp(0.02, 40.0);
                if mods.shift {
                    // Shift: só proporcional não faz diferença (já é), então Alt estica.
                }
            }
        }
        Interaction::FloatRotate => {
            if let (Some(fs), Some((center, _, _))) = (&mut st.docs[cur].float, handles) {
                let v = screen - center;
                let mut deg = v.y.atan2(v.x).to_degrees() + 90.0;
                if mods.shift {
                    deg = (deg / 15.0).round() * 15.0;
                }
                fs.rotation = deg;
            }
        }
    }
}

fn finish_interaction(st: &mut State, pt: (f32, f32), _mods: egui::Modifiers) {
    let cur = st.cur;
    let Some(inter) = st.interaction.take() else {
        return;
    };
    let (w, h) = (st.docs[cur].ed.doc().width, st.docs[cur].ed.doc().height);
    let active = st.docs[cur].active;
    match inter {
        Interaction::Stroke => st.docs[cur].ed.end_stroke(pt.0, pt.1),
        Interaction::Gradient { start, cur: end } => {
            if (start.0 - end.0).hypot(start.1 - end.1) < 2.0 {
                return;
            }
            let (fg, bg) = (st.fg, st.bg);
            let a = [fg[0], fg[1], fg[2], 255];
            let b = if st.opts.gradient_transparent {
                [fg[0], fg[1], fg[2], 0]
            } else {
                [bg[0], bg[1], bg[2], 255]
            };
            let (kind, op) = (st.opts.gradient_kind, st.opts.gradient_opacity);
            if let Err(e) = st.docs[cur].ed.gradient(active, start, end, kind, a, b, op) {
                st.warn(e.to_string());
            }
        }
        Interaction::Select {
            tool,
            start,
            cur: end,
            pts,
            op,
        } => {
            let sel = match tool {
                Tool::SelRect => Selection::rect(
                    w,
                    h,
                    start.0.round() as i64,
                    start.1.round() as i64,
                    end.0.round() as i64,
                    end.1.round() as i64,
                ),
                Tool::SelEllipse => Selection::ellipse(w, h, start.0, start.1, end.0, end.1),
                _ => Selection::polygon(w, h, &pts),
            };
            let feather = st.opts.feather;
            let sel = if feather > 0 {
                sel.feather(feather)
            } else {
                sel
            };
            if sel.is_empty() {
                if op == SelOp::Replace {
                    st.docs[cur].ed.deselect();
                }
            } else {
                st.docs[cur].ed.combine_selection(sel, op, "Seleção");
            }
        }
        Interaction::Shape {
            tool,
            start,
            cur: end,
            shift,
        } => {
            let end = if shift && tool != Tool::Line {
                square_to(start, end)
            } else {
                end
            };
            commit_shape(st, tool, start, end);
        }
        Interaction::FloatMove { .. }
        | Interaction::FloatScale { .. }
        | Interaction::FloatRotate => {
            if st.docs[cur].float.as_ref().is_some_and(|f| f.quick) {
                commit_float(&mut st.docs[cur]);
            }
        }
        Interaction::Pan | Interaction::Picker | Interaction::Crop { .. } => {}
    }
}

fn commit_shape(st: &mut State, tool: Tool, a: (f32, f32), b: (f32, f32)) {
    let cur = st.cur;
    let active = st.docs[cur].active;
    let (w, h) = (st.docs[cur].ed.doc().width, st.docs[cur].ed.doc().height);
    if (a.0 - b.0).hypot(a.1 - b.1) < 1.0 {
        return;
    }
    let points: Vec<(f32, f32)> = match tool {
        Tool::Line => vec![a, b],
        Tool::Rect => vec![a, (b.0, a.1), b, (a.0, b.1), a],
        _ => {
            let (cx, cy, rx, ry) = (
                (a.0 + b.0) / 2.0,
                (a.1 + b.1) / 2.0,
                (b.0 - a.0).abs() / 2.0,
                (b.1 - a.1).abs() / 2.0,
            );
            let n = ((rx.max(ry) * std::f32::consts::TAU / 3.0) as usize).clamp(24, 720);
            (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32 * std::f32::consts::TAU;
                    (cx + rx * t.cos(), cy + ry * t.sin())
                })
                .collect()
        }
    };
    if st.opts.shape_fill && tool != Tool::Line {
        let poly = Selection::polygon(w, h, &points);
        let bg = st.bg;
        if let Err(e) = st.docs[cur]
            .ed
            .fill_shape(active, &poly, bg, "Forma preenchida")
        {
            st.warn(e.to_string());
            return;
        }
    }
    let (brush, color, mode) = (st.brush.clone(), st.fg, st.blend);
    st.docs[cur].ed.set_mirror(st.mirror.0, st.mirror.1);
    match st.docs[cur]
        .ed
        .begin_stroke(active, brush, color, false, mode)
    {
        Ok(()) => {
            for pair in points.windows(2) {
                st.docs[cur].ed.stroke_line(pair[0], pair[1]);
            }
            let last = points[points.len() - 1];
            st.docs[cur].ed.end_stroke(last.0, last.1);
        }
        Err(e) => st.warn(e.to_string()),
    }
}

fn commit_text(st: &mut State, at: (f32, f32), text: &str) {
    if text.trim().is_empty() {
        return;
    }
    let cur = st.cur;
    let mut o = st.opts.text.clone();
    o.text = text.to_string();
    o.color = st.fg;
    let img = st.text_raster.render(&o);
    let active = st.docs[cur].active;
    if let Err(e) = st.docs[cur].ed.stamp(
        active,
        &img,
        at.0.round() as i64,
        at.1.round() as i64,
        "Texto",
    ) {
        st.warn(e.to_string());
    }
}

fn text_popup(st: &mut State, ui: &mut egui::Ui, to_screen: &dyn Fn(f32, f32) -> Pos2) {
    let Some(((x, y), mut text)) = st.text_input.take() else {
        return;
    };
    let pos = to_screen(x, y);
    let mut commit = false;
    let mut cancel = false;
    egui::Area::new(egui::Id::new("paw-text-input"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                let r = ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_width(220.0)
                        .desired_rows(2)
                        .hint_text("Digite o texto (Ctrl+Enter aplica, Esc cancela)"),
                );
                r.request_focus();
                if ui.input(|i| i.key_pressed(Key::Enter) && i.modifiers.command) {
                    commit = true;
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    cancel = true;
                }
            });
        });
    if commit {
        commit_text(st, (x, y), &text);
    } else if !cancel {
        st.text_input = Some(((x, y), text));
    }
}

/// Enter/Esc: aplica ou cancela o corte e a transformação em andamento.
pub fn confirm(st: &mut State, apply: bool) {
    let cur = st.cur;
    let Some(d) = st.docs.get_mut(cur) else {
        return;
    };
    if d.float.is_some() {
        if apply {
            commit_float(d);
        } else {
            abort_float(d);
        }
    }
    if let Some((x0, y0, x1, y1)) = d.crop.take()
        && apply
    {
        let (a, b) = (
            (x0.min(x1).max(0.0)) as usize,
            (y0.min(y1).max(0.0)) as usize,
        );
        let (c, e) = (x0.max(x1).max(0.0) as usize, y0.max(y1).max(0.0) as usize);
        d.ed.crop(a, b, c, e);
        d.fit = true;
    }
}

#[allow(dead_code)]
fn unused(_: &Editor, _: GradientKind) {}
