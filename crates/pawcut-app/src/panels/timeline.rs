//! A timeline: régua, cabeçalhos das faixas, clipes e as interações de edição (visual do Resolve).

use egui::{Align2, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use pawcut_core::{
    Clip, ClipChange, ClipId, ClipSource, Edge, Edit, Frame, MediaKind, Project, TrackFlags,
    TrackKind, format_timecode,
};

use super::DragPayload;
use crate::icons::{self, Icon};
use crate::state::{Monitor, PlaceMode, Tool};
use crate::theme;
use crate::workspace::Workspace;

const HEADER_W: f32 = 196.0;
const RULER_H: f32 = 30.0;
const TRACK_H: f32 = 50.0;
const ZONE_H: f32 = 22.0;
const LABEL_H: f32 = 15.0;
const EDGE_GRAB: f32 = 6.0;
const SNAP_PX: f32 = 8.0;

/// Uma linha da timeline. `index == len` é a zona vazia que cria uma faixa nova ao soltar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub kind: TrackKind,
    pub index: usize,
}

#[derive(Debug, Clone)]
enum TlDrag {
    Scrub,
    /// Alça de fade no canto de cima do clipe.
    Fade {
        clip: ClipId,
        edge: Edge,
    },
    Move {
        anchor: ClipId,
        clips: Vec<ClipId>,
        grab: Frame,
        origin: Row,
    },
    Trim {
        anchor: ClipId,
        clips: Vec<ClipId>,
        edge: Edge,
        grab: Frame,
    },
}

pub struct TimelineView {
    /// Pixels por frame.
    pub ppf: f32,
    /// Primeiro frame visível.
    pub scroll: f32,
    /// Rolagem vertical das faixas, em pixels.
    pub v_scroll: f32,
    drag: Option<TlDrag>,
    menu_clip: Option<ClipId>,
}

impl Default for TimelineView {
    fn default() -> Self {
        Self {
            ppf: 3.0,
            scroll: 0.0,
            v_scroll: 0.0,
            drag: None,
            menu_clip: None,
        }
    }
}

struct Layout {
    rows: Vec<(Row, Rect)>,
    lanes: Rect,
    left: f32,
    /// Altura total das linhas (para limitar a rolagem vertical).
    content_h: f32,
}

impl Layout {
    fn new(p: &Project, area: Rect, v_scroll: f32) -> Self {
        let mut rows = Vec::new();
        let top = area.top() + RULER_H - v_scroll;
        let mut y = top;
        let lane =
            |y: f32, h: f32| Rect::from_min_max(pos2(area.left(), y), pos2(area.right(), y + h));
        let vlen = p.video_tracks.len();
        rows.push((
            Row {
                kind: TrackKind::Video,
                index: vlen,
            },
            lane(y, ZONE_H),
        ));
        y += ZONE_H;
        for i in (0..vlen).rev() {
            rows.push((
                Row {
                    kind: TrackKind::Video,
                    index: i,
                },
                lane(y, TRACK_H),
            ));
            y += TRACK_H;
        }
        y += 6.0;
        let alen = p.audio_tracks.len();
        for i in 0..alen {
            rows.push((
                Row {
                    kind: TrackKind::Audio,
                    index: i,
                },
                lane(y, TRACK_H),
            ));
            y += TRACK_H;
        }
        rows.push((
            Row {
                kind: TrackKind::Audio,
                index: alen,
            },
            lane(y, ZONE_H),
        ));
        y += ZONE_H;
        let lanes =
            Rect::from_min_max(pos2(area.left() + HEADER_W, area.top() + RULER_H), area.max);
        Self {
            rows,
            lanes,
            left: area.left() + HEADER_W,
            content_h: y - top,
        }
    }

    fn row_at(&self, y: f32) -> Option<Row> {
        self.rows
            .iter()
            .find(|(_, r)| y >= r.top() && y < r.bottom())
            .map(|(row, _)| *row)
    }

    fn rect_of(&self, row: Row) -> Option<Rect> {
        self.rows
            .iter()
            .find(|(r, _)| *r == row)
            .map(|(_, rect)| *rect)
    }
}

fn row_of(p: &Project, clip: ClipId) -> Option<Row> {
    let (tid, _) = p.locate(clip)?;
    let t = p.track(tid)?;
    let index = p.tracks(t.kind).iter().position(|x| x.id == tid)?;
    Some(Row {
        kind: t.kind,
        index,
    })
}

fn clip_name(p: &Project, c: &Clip) -> String {
    match &c.source {
        ClipSource::Media(m) => p
            .media(*m)
            .and_then(|m| {
                m.info
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "?".into()),
        ClipSource::SolidColor(_) => "Cor sólida".into(),
        ClipSource::Text(t) => format!("Texto — {}", t.text.lines().next().unwrap_or("")),
    }
}

/// Cor do corpo e da faixa de título do clipe.
fn clip_colors(p: &Project, c: &Clip, kind: TrackKind) -> (Color32, Color32) {
    let body = match (&c.source, kind) {
        (_, TrackKind::Audio) => return (theme::CLIP_AUDIO, theme::CLIP_AUDIO_LABEL),
        (ClipSource::Media(m), _)
            if p.media(*m).is_some_and(|m| m.info.kind == MediaKind::Image) =>
        {
            theme::CLIP_IMAGE
        }
        (ClipSource::Media(_), _) => return (theme::CLIP_VIDEO, theme::CLIP_VIDEO_LABEL),
        (ClipSource::Text(_), _) => theme::CLIP_TITLE,
        (ClipSource::SolidColor(_), _) => theme::CLIP_GENERATOR,
    };
    (body, body.gamma_multiply(0.72))
}

/// Passo da régua (em frames) para que as marcas fiquem a pelo menos `min_px` de distância.
fn ruler_step(ppf: f32, fps: i64, min_px: f32) -> Frame {
    let candidates = [
        1,
        2,
        5,
        10,
        fps / 2,
        fps,
        2 * fps,
        5 * fps,
        10 * fps,
        15 * fps,
        30 * fps,
        60 * fps,
        120 * fps,
        300 * fps,
        600 * fps,
    ];
    candidates
        .into_iter()
        .filter(|c| *c > 0)
        .find(|c| *c as f32 * ppf >= min_px)
        .unwrap_or(600 * fps)
}

impl Workspace {
    fn x_of(&self, frame: f32, left: f32) -> f32 {
        left + (frame - self.tl.scroll) * self.tl.ppf
    }

    fn frame_at(&self, x: f32, left: f32) -> f32 {
        self.tl.scroll + (x - left) / self.tl.ppf
    }

    pub fn zoom_to_fit(&mut self, width: f32) {
        let total = self.project_arc.duration().max(30) as f32;
        self.tl.ppf = ((width - HEADER_W - 40.0) / total).clamp(0.02, 40.0);
        self.tl.scroll = 0.0;
    }

    pub fn zoom(&mut self, factor: f32) {
        let center = self.st.playhead as f32;
        let before = (center - self.tl.scroll) * self.tl.ppf;
        self.tl.ppf = (self.tl.ppf * factor).clamp(0.02, 40.0);
        self.tl.scroll = (center - before / self.tl.ppf).max(0.0);
    }

    pub fn timeline_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.project_arc.clone();
        let fps = p.settings.fps;
        self.timeline_toolbar(ui);
        let area = ui.available_rect_before_wrap();
        self.tl_width = area.width();
        self.tl_area = area;
        let visible_h = area.height() - RULER_H;
        let content_h = Layout::new(&p, area, 0.0).content_h;
        self.tl.v_scroll = self
            .tl
            .v_scroll
            .clamp(0.0, (content_h - visible_h).max(0.0));
        let layout = Layout::new(&p, area, self.tl.v_scroll);
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, theme::BG_DARK);

        // Mantém o cursor visível enquanto toca.
        if self.playback.is_playing() {
            let px = self.x_of(self.st.playhead as f32, layout.left);
            if px > area.right() - 20.0 || px < layout.left {
                self.tl.scroll = (self.st.playhead as f32 - 10.0 / self.tl.ppf).max(0.0);
            }
        }

        // ---------- rolagem e zoom ----------
        // Ctrl+roda: zoom. Roda sobre os clipes: anda no tempo. Roda sobre os cabeçalhos ou
        // Shift+roda: sobe/desce as faixas.
        if ui.rect_contains_pointer(area) {
            let (zoom, scroll, shift, pointer) = ui.input(|i| {
                (
                    i.zoom_delta(),
                    i.smooth_scroll_delta,
                    i.modifiers.shift,
                    i.pointer.hover_pos(),
                )
            });
            let over_headers = pointer.is_some_and(|p| p.x < layout.left);
            if (zoom - 1.0).abs() > 1e-3 {
                let px = pointer.map_or(layout.left, |p| p.x);
                let anchor = self.frame_at(px, layout.left);
                self.tl.ppf = (self.tl.ppf * zoom).clamp(0.02, 40.0);
                self.tl.scroll = (anchor - (px - layout.left) / self.tl.ppf).max(0.0);
            } else if scroll != Vec2::ZERO {
                if over_headers || shift {
                    let dy = if scroll.y != 0.0 { scroll.y } else { scroll.x };
                    self.tl.v_scroll =
                        (self.tl.v_scroll - dy).clamp(0.0, (content_h - visible_h).max(0.0));
                } else {
                    self.tl.scroll =
                        (self.tl.scroll - (scroll.x + scroll.y) / self.tl.ppf).max(0.0);
                }
            }
        }

        // ---------- fundo das faixas ----------
        let rows_painter = painter.with_clip_rect(Rect::from_min_max(
            pos2(area.left(), layout.lanes.top()),
            area.max,
        ));
        for (row, rect) in &layout.rows {
            let exists = row.index < p.tracks(row.kind).len();
            let fill = match (exists, row.kind) {
                (false, _) => theme::BG_DARK,
                (true, TrackKind::Video) => Color32::from_rgb(0x1d, 0x1d, 0x21),
                (true, TrackKind::Audio) => Color32::from_rgb(0x1b, 0x1e, 0x1d),
            };
            rows_painter.rect_filled(*rect, 0.0, fill);
            rows_painter.line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                Stroke::new(1.0, theme::BORDER),
            );
        }

        // ---------- interações na área das faixas ----------
        let lanes_resp = ui.interact(layout.lanes, ui.id().with("lanes"), Sense::click_and_drag());
        let tolerance = (SNAP_PX / self.tl.ppf).ceil() as Frame;
        let pointer = lanes_resp.interact_pointer_pos().or(lanes_resp.hover_pos());
        let hovered_clip = pointer.and_then(|pos| self.hit_clip(&p, &layout, pos));

        let fade_hit = pointer.and_then(|pos| self.hit_fade(&p, &layout, pos));
        if fade_hit.is_some() && self.tl.drag.is_none() && self.st.tool == Tool::Selection {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        } else if let Some((_, edge)) = hovered_clip.filter(|_| self.tl.drag.is_none()) {
            let icon = match (self.st.tool, edge) {
                (Tool::Blade, _) => CursorIcon::Crosshair,
                (_, Some(_)) => CursorIcon::ResizeHorizontal,
                _ => CursorIcon::Grab,
            };
            ui.ctx().set_cursor_icon(icon);
        }

        // O drag só começa depois que o ponteiro anda alguns pixels: a pegada é onde o botão desceu.
        if lanes_resp.drag_started()
            && let Some(pos) = ui
                .input(|i| i.pointer.press_origin())
                .or(lanes_resp.interact_pointer_pos())
        {
            let grab = self.frame_at(pos.x, layout.left).round() as Frame;
            let fade = if self.st.tool == Tool::Selection {
                self.hit_fade(&p, &layout, pos)
            } else {
                None
            };
            if let Some((clip, edge)) = fade {
                self.st.select(clip, false);
                self.tl.drag = Some(TlDrag::Fade { clip, edge });
            } else {
                match (self.st.tool, self.hit_clip(&p, &layout, pos)) {
                    (Tool::Selection, Some((clip, edge))) => {
                        if !self.st.selection.contains(&clip) {
                            self.st.select(clip, false);
                        }
                        match edge {
                            Some(edge) => {
                                let clips = self.st.expanded(&[clip]);
                                self.tl.drag = Some(TlDrag::Trim {
                                    anchor: clip,
                                    clips,
                                    edge,
                                    grab,
                                });
                            }
                            None => {
                                let clips = self.st.expanded(&self.st.selection.clone());
                                if let Some(origin) = row_of(&p, clip) {
                                    self.tl.drag = Some(TlDrag::Move {
                                        anchor: clip,
                                        clips,
                                        grab,
                                        origin,
                                    });
                                }
                            }
                        }
                    }
                    (_, None) => {
                        self.stop();
                        self.tl.drag = Some(TlDrag::Scrub);
                    }
                    _ => {}
                }
            }
        }

        let mut snap_line: Option<Frame> = None;
        let mut ghosts: Vec<(Rect, bool)> = Vec::new();
        if let Some(pos) = lanes_resp
            .interact_pointer_pos()
            .filter(|_| lanes_resp.dragged())
        {
            let now = self.frame_at(pos.x, layout.left).round() as Frame;
            match self.tl.drag.clone() {
                Some(TlDrag::Scrub) => self.st.set_playhead(now),
                Some(TlDrag::Fade { clip, edge }) => {
                    if let Some(c) = p.clip(clip) {
                        let frames = match edge {
                            Edge::Start => now - c.start,
                            Edge::End => c.end() - now,
                        }
                        .clamp(0, c.duration);
                        let change = if edge == Edge::Start {
                            ClipChange::FadeIn(frames)
                        } else {
                            ClipChange::FadeOut(frames)
                        };
                        self.st.apply_coalescing(
                            Edit::SetClip { clip, change },
                            &format!("fade-drag-{clip:?}"),
                        );
                    }
                }
                Some(TlDrag::Move {
                    anchor,
                    clips,
                    grab,
                    origin,
                }) => {
                    let (delta, snapped) =
                        self.move_delta(&p, anchor, &clips, now - grab, tolerance);
                    snap_line = snapped;
                    let rows = layout
                        .row_at(pos.y)
                        .filter(|r| r.kind == origin.kind)
                        .map_or(0, |r| r.index as i64 - origin.index as i64);
                    ghosts = self.move_ghosts(&p, &layout, &clips, origin.kind, delta, rows);
                }
                Some(TlDrag::Trim {
                    anchor,
                    clips,
                    edge,
                    grab,
                }) => {
                    if let Some(c) = p.clip(anchor) {
                        let edge_frame = if edge == Edge::Start {
                            c.start
                        } else {
                            c.end()
                        };
                        let target = self.st.snap(edge_frame + (now - grab), &clips, tolerance);
                        if target != edge_frame + (now - grab) {
                            snap_line = Some(target);
                        }
                        ghosts.extend(
                            self.trim_ghost(&p, &layout, anchor, edge, target)
                                .map(|r| (r, false)),
                        );
                    }
                }
                None => {}
            }
        }

        if lanes_resp.drag_stopped() {
            let pos = lanes_resp
                .interact_pointer_pos()
                .or(ui.input(|i| i.pointer.latest_pos()));
            if let (Some(drag), Some(pos)) = (self.tl.drag.take(), pos) {
                let now = self.frame_at(pos.x, layout.left).round() as Frame;
                match drag {
                    TlDrag::Scrub | TlDrag::Fade { .. } => {}
                    TlDrag::Move {
                        anchor,
                        clips,
                        grab,
                        origin,
                    } => {
                        let (delta, _) = self.move_delta(&p, anchor, &clips, now - grab, tolerance);
                        let row_delta = layout
                            .row_at(pos.y)
                            .filter(|r| r.kind == origin.kind)
                            .map_or(0, |r| r.index as i32 - origin.index as i32);
                        // Cria a faixa de destino se o clipe foi solto na zona vazia.
                        if row_delta > 0 {
                            let max = clips
                                .iter()
                                .filter_map(|c| row_of(&p, *c))
                                .filter(|r| r.kind == origin.kind)
                                .map(|r| r.index)
                                .max()
                                .unwrap_or(0);
                            self.st.ensure_track(origin.kind, max + row_delta as usize);
                        }
                        let (vtd, atd) = if origin.kind == TrackKind::Video {
                            (row_delta, 0)
                        } else {
                            (0, row_delta)
                        };
                        self.st.apply(Edit::Move {
                            clips,
                            delta,
                            video_track_delta: vtd,
                            audio_track_delta: atd,
                        });
                    }
                    TlDrag::Trim {
                        anchor,
                        clips,
                        edge,
                        grab,
                    } => {
                        if let Some(c) = p.clip(anchor) {
                            let edge_frame = if edge == Edge::Start {
                                c.start
                            } else {
                                c.end()
                            };
                            let target = self.st.snap(edge_frame + (now - grab), &clips, tolerance);
                            self.st.apply(Edit::Trim {
                                clips,
                                edge,
                                delta: target - edge_frame,
                            });
                        }
                    }
                }
            }
            self.tl.drag = None;
        }

        if lanes_resp.double_clicked()
            && let Some(pos) = lanes_resp.interact_pointer_pos()
            && let Some((clip, _)) = self.hit_clip(&p, &layout, pos)
        {
            self.match_frame(&p, clip, self.frame_at(pos.x, layout.left).round() as Frame);
        } else if lanes_resp.clicked()
            && let Some(pos) = lanes_resp.interact_pointer_pos()
        {
            let additive = ui.input(|i| i.modifiers.shift || i.modifiers.ctrl);
            match (self.st.tool, self.hit_clip(&p, &layout, pos)) {
                (Tool::Blade, Some((clip, _))) => {
                    let raw = self.frame_at(pos.x, layout.left).round() as Frame;
                    let at = self.st.snap(raw, &[], tolerance);
                    self.st.blade_clip(clip, at);
                }
                (Tool::Selection, Some((clip, _))) => self.st.select(clip, additive),
                (_, None) => {
                    self.stop();
                    self.st.selection.clear();
                    let f = self.frame_at(pos.x, layout.left).round() as Frame;
                    self.st.set_playhead(f);
                }
            }
            self.st.active_monitor = Monitor::Timeline;
        }

        if lanes_resp.secondary_clicked() {
            self.tl.menu_clip = pointer
                .and_then(|pos| self.hit_clip(&p, &layout, pos))
                .map(|(c, _)| c);
            if let Some(c) = self.tl.menu_clip
                && !self.st.selection.contains(&c)
            {
                self.st.select(c, false);
            }
        }
        lanes_resp.context_menu(|ui| self.clip_menu(ui));

        // ---------- soltar mídia / geradores ----------
        if let Some(payload) = lanes_resp.dnd_hover_payload::<DragPayload>()
            && let Some(pos) = ui.input(|i| i.pointer.hover_pos())
            && let Some(row) = layout.row_at(pos.y)
            && let Some(rect) = layout.rect_of(row)
        {
            let at = self.st.snap(
                self.frame_at(pos.x, layout.left).round().max(0.0) as Frame,
                &[],
                tolerance,
            );
            let len = self.payload_len(&p, *payload);
            let x0 = self.x_of(at as f32, layout.left);
            ghosts.push((
                Rect::from_min_max(
                    pos2(x0, rect.top() + 2.0),
                    pos2(x0 + len as f32 * self.tl.ppf, rect.bottom() - 2.0),
                ),
                true,
            ));
        }
        if let Some(payload) = lanes_resp.dnd_release_payload::<DragPayload>()
            && let Some(pos) = ui.input(|i| i.pointer.latest_pos())
            && let Some(row) = layout.row_at(pos.y)
        {
            let at = self.st.snap(
                self.frame_at(pos.x, layout.left).round().max(0.0) as Frame,
                &[],
                tolerance,
            );
            match *payload {
                DragPayload::Media(m) => self.st.drop_media(m, row.kind, row.index, at),
                DragPayload::Generator(g) if row.kind == TrackKind::Video => {
                    self.st.drop_generator(g, row.index, at)
                }
                DragPayload::Generator(_) => {
                    self.st.say("Títulos e geradores vão nas faixas de vídeo")
                }
                DragPayload::Dissolve => {
                    self.st.set_playhead(at);
                    self.st.add_dissolve();
                }
            }
        }

        // ---------- clipes ----------
        let p = self.project_arc_after_edits(&p);
        let lanes_painter = painter.with_clip_rect(layout.lanes);
        let hovered_id = hovered_clip
            .map(|(c, _)| c)
            .filter(|_| self.tl.drag.is_none());
        for (row, rect) in &layout.rows {
            let Some(track) = p.tracks(row.kind).get(row.index) else {
                continue;
            };
            for clip in &track.clips {
                let x0 = self.x_of(clip.start as f32, layout.left);
                let x1 = self.x_of(clip.end() as f32, layout.left);
                if x1 < layout.left || x0 > area.right() {
                    continue;
                }
                let r = Rect::from_min_max(
                    pos2(x0, rect.top() + 1.0),
                    pos2(x1.max(x0 + 1.0), rect.bottom() - 1.0),
                );
                self.paint_clip(
                    &lanes_painter,
                    &p,
                    clip,
                    row.kind,
                    track.enabled && !track.muted,
                    r,
                    Some(clip.id) == hovered_id,
                );
            }
        }
        for (g, is_drop) in ghosts {
            lanes_painter.rect_filled(
                g,
                2.0,
                theme::ACCENT.gamma_multiply(if is_drop { 0.25 } else { 0.18 }),
            );
            lanes_painter.rect_stroke(g, 2.0, Stroke::new(1.5, theme::ACCENT), StrokeKind::Inside);
        }

        // ---------- régua ----------
        let ruler = Rect::from_min_max(
            pos2(layout.left, area.top()),
            pos2(area.right(), area.top() + RULER_H),
        );
        painter.rect_filled(ruler, 0.0, theme::BAR);
        painter.line_segment(
            [ruler.left_bottom(), ruler.right_bottom()],
            Stroke::new(1.0, theme::BORDER),
        );
        let step = ruler_step(self.tl.ppf, fps.nominal(), 90.0);
        let first = (self.tl.scroll as Frame / step) * step;
        let ruler_painter = painter.with_clip_rect(ruler);
        let mut f = first;
        while self.x_of(f as f32, layout.left) < area.right() {
            let x = self.x_of(f as f32, layout.left);
            ruler_painter.line_segment(
                [pos2(x, ruler.bottom() - 10.0), pos2(x, ruler.bottom())],
                Stroke::new(1.0, theme::TEXT_DIM),
            );
            ruler_painter.text(
                pos2(x + 4.0, ruler.top() + 4.0),
                Align2::LEFT_TOP,
                format_timecode(f, fps),
                theme::mono(10.0),
                theme::TEXT_DIM,
            );
            let minor = step / 5;
            if minor > 0 {
                for k in 1..5 {
                    let xm = self.x_of((f + minor * k) as f32, layout.left);
                    ruler_painter.line_segment(
                        [pos2(xm, ruler.bottom() - 4.0), pos2(xm, ruler.bottom())],
                        Stroke::new(1.0, Color32::from_gray(70)),
                    );
                }
            }
            f += step;
        }
        if let (Some(i), Some(o)) = (self.st.mark_in, self.st.mark_out) {
            let r = Rect::from_min_max(
                pos2(self.x_of(i as f32, layout.left), ruler.bottom() - 6.0),
                pos2(self.x_of(o as f32, layout.left), ruler.bottom()),
            );
            ruler_painter.rect_filled(r, 0.0, theme::ACCENT.gamma_multiply(0.6));
        }
        // Marcadores (bandeirinhas) na régua.
        let mut marker_hover = None;
        for m in &p.markers {
            let x = self.x_of(m.frame as f32, layout.left);
            if x < layout.left - 8.0 || x > area.right() + 8.0 {
                continue;
            }
            let c = Color32::from_rgb(m.color.r, m.color.g, m.color.b);
            let top = ruler.top() + 15.0;
            ruler_painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(x - 5.0, top),
                    pos2(x + 5.0, top),
                    pos2(x + 5.0, top + 8.0),
                    pos2(x, top + 13.0),
                    pos2(x - 5.0, top + 8.0),
                ],
                c,
                Stroke::new(1.0, Color32::BLACK),
            ));
            if let Some(h) = ui.input(|i| i.pointer.hover_pos())
                && ruler.contains(h)
                && (h.x - x).abs() <= 6.0
            {
                marker_hover = Some(m.clone());
            }
        }
        let ruler_resp = ui.interact(ruler, ui.id().with("ruler"), Sense::click_and_drag());
        if let Some(m) = &marker_hover {
            let tip = if m.note.is_empty() {
                "Marcador (duplo clique para editar)".to_string()
            } else {
                m.note.clone()
            };
            ruler_resp.clone().on_hover_text_at_pointer(tip);
            if ruler_resp.double_clicked() {
                self.editing_marker = Some(m.frame);
            }
        }
        if (ruler_resp.clicked() || ruler_resp.dragged())
            && let Some(pos) = ruler_resp.interact_pointer_pos()
        {
            self.stop();
            self.st.active_monitor = Monitor::Timeline;
            self.st
                .set_playhead(self.frame_at(pos.x, layout.left).round() as Frame);
        }

        // ---------- linha do mouse, snapping e cursor ----------
        let hover_x = ui.input(|i| i.pointer.hover_pos()).filter(|pos| {
            pos.x >= layout.left
                && pos.x <= area.right()
                && pos.y >= area.top()
                && pos.y <= area.bottom()
        });
        if let Some(pos) = hover_x
            && self.tl.drag.is_none()
        {
            let frame = self.frame_at(pos.x, layout.left).round().max(0.0);
            let x = self.x_of(frame, layout.left);
            painter.line_segment(
                [pos2(x, ruler.top()), pos2(x, area.bottom())],
                Stroke::new(1.0, Color32::from_white_alpha(70)),
            );
            let label = format_timecode(frame as Frame, fps);
            let galley = painter.layout_no_wrap(label, theme::mono(10.5), Color32::WHITE);
            let bubble = Rect::from_min_size(
                pos2(x + 4.0, ruler.bottom() - galley.size().y - 4.0),
                galley.size() + vec2(6.0, 2.0),
            );
            painter.rect_filled(bubble, 2.0, Color32::from_rgb(0x45, 0x45, 0x4c));
            painter.galley(bubble.min + vec2(3.0, 1.0), galley, Color32::WHITE);
        }
        if let Some(sf) = snap_line {
            let x = self.x_of(sf as f32, layout.left);
            lanes_painter.line_segment(
                [pos2(x, layout.lanes.top()), pos2(x, area.bottom())],
                Stroke::new(1.0, Color32::from_rgb(250, 220, 80)),
            );
        }
        let px = self.x_of(self.st.playhead as f32, layout.left);
        if px >= layout.left && px <= area.right() {
            painter.line_segment(
                [pos2(px, ruler.top()), pos2(px, area.bottom())],
                Stroke::new(1.5, theme::PLAYHEAD),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(px - 7.0, ruler.top()),
                    pos2(px + 7.0, ruler.top()),
                    pos2(px + 7.0, ruler.top() + 8.0),
                    pos2(px, ruler.top() + 15.0),
                    pos2(px - 7.0, ruler.top() + 8.0),
                ],
                theme::PLAYHEAD,
                Stroke::NONE,
            ));
        }

        // ---------- cabeçalhos ----------
        self.track_headers(ui, &painter, &p, &layout, area);
        // Timecode grande no canto, como no Resolve.
        let tc = Rect::from_min_size(area.min, vec2(HEADER_W, RULER_H));
        painter.rect_filled(tc, 0.0, theme::BG_DARK);
        painter.text(
            tc.left_center() + vec2(10.0, 0.0),
            Align2::LEFT_CENTER,
            format_timecode(self.st.playhead, fps),
            theme::mono(17.0),
            theme::TIMECODE,
        );
        ui.advance_cursor_after_rect(area);
    }

    /// Match frame (F no Resolve): abre a mídia do clipe no Source Viewer no frame equivalente.
    fn match_frame(&mut self, p: &Project, clip: ClipId, at: Frame) {
        let Some(c) = p.clip(clip) else { return };
        let ClipSource::Media(m) = &c.source else {
            return;
        };
        let source_frame = c.source_time_at(at.clamp(c.start, c.end() - 1)).floor() as Frame;
        self.st.open_in_source(*m);
        self.st.source.playhead = source_frame.max(0);
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_clip(
        &mut self,
        painter: &egui::Painter,
        p: &Project,
        clip: &Clip,
        kind: TrackKind,
        active: bool,
        r: Rect,
        hovered: bool,
    ) {
        let offline = matches!(clip.source, ClipSource::Media(m) if self.st.offline.contains(&m));
        let (mut body, mut label_bg) = if offline {
            (theme::OFFLINE, theme::OFFLINE.gamma_multiply(0.7))
        } else {
            clip_colors(p, clip, kind)
        };
        if !active {
            body = body.gamma_multiply(0.4);
            label_bg = label_bg.gamma_multiply(0.4);
        }
        if hovered {
            body = body.gamma_multiply(1.15);
        }
        painter.rect_filled(r, 2.0, body);
        let label = Rect::from_min_max(r.min, pos2(r.right(), r.top() + LABEL_H));
        painter.rect_filled(
            label,
            egui::CornerRadius {
                nw: 2,
                ne: 2,
                sw: 0,
                se: 0,
            },
            label_bg,
        );
        let content = Rect::from_min_max(pos2(r.left(), label.bottom()), r.max);

        // Miniatura no começo dos clipes de vídeo/imagem (como a tira de frames do Resolve).
        if kind == TrackKind::Video
            && let ClipSource::Media(m) = &clip.source
            && let Some(media) = p.media(*m)
            && content.width() > 30.0
            && let Some(tex) = self.thumbnail(*m, &media.info.path, media.info.kind)
        {
            let size = tex.size_vec2();
            let h = content.height() - 2.0;
            let w = (h * size.x / size.y).min(content.width() - 2.0);
            let thumb = Rect::from_min_size(content.min + vec2(1.0, 1.0), vec2(w, h));
            painter
                .with_clip_rect(content.intersect(painter.clip_rect()))
                .image(
                    tex.id(),
                    thumb,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(w / (h * size.x / size.y), 1.0)),
                    if active {
                        Color32::WHITE
                    } else {
                        Color32::from_gray(110)
                    },
                );
        }
        if kind == TrackKind::Audio {
            self.paint_waveform(painter, p, clip, content);
        }
        self.paint_fades(painter, clip, r);
        if let Some((a, _)) = clip.dissolve_window() {
            // Caixa da transição centrada no corte, como no Resolve.
            let x0 = r.left() + (a - clip.start) as f32 * self.tl.ppf;
            let x1 = x0 + clip.dissolve_in as f32 * self.tl.ppf;
            let bx = Rect::from_min_max(pos2(x0, r.top() + LABEL_H), pos2(x1, r.bottom()));
            painter.rect_filled(bx, 2.0, Color32::from_white_alpha(45));
            painter.rect_stroke(
                bx,
                2.0,
                Stroke::new(1.0, Color32::from_white_alpha(150)),
                StrokeKind::Inside,
            );
            painter.line_segment(
                [bx.left_bottom(), bx.right_top()],
                Stroke::new(1.0, Color32::from_white_alpha(150)),
            );
        }
        let selected_now = self.st.selection.contains(&clip.id);
        if hovered || selected_now {
            // Alças de fade nos cantos de cima.
            for x in [
                r.left() + clip.fade_in as f32 * self.tl.ppf,
                r.right() - clip.fade_out as f32 * self.tl.ppf,
            ] {
                let hdl = Rect::from_center_size(
                    pos2(x.clamp(r.left() + 4.0, r.right() - 4.0), r.top() + 4.0),
                    vec2(8.0, 8.0),
                );
                painter.rect_filled(hdl, 1.0, Color32::WHITE);
                painter.rect_stroke(
                    hdl,
                    1.0,
                    Stroke::new(1.0, Color32::BLACK),
                    StrokeKind::Inside,
                );
            }
        }
        if selected_now {
            // Losangos dos keyframes (no tempo da fonte → posição na timeline).
            for keys in clip.keyframes.values() {
                for k in keys {
                    let f = clip.start as f64 + (k.at - clip.source_start) / clip.speed;
                    let x = r.left() + (f as f32 - clip.start as f32) * self.tl.ppf;
                    if x < r.left() || x > r.right() {
                        continue;
                    }
                    let c = pos2(x, r.bottom() - 6.0);
                    let pts = vec![
                        c + vec2(0.0, -4.5),
                        c + vec2(4.5, 0.0),
                        c + vec2(0.0, 4.5),
                        c + vec2(-4.5, 0.0),
                    ];
                    painter.add(egui::Shape::convex_polygon(
                        pts,
                        theme::ACCENT,
                        Stroke::new(1.0, Color32::BLACK),
                    ));
                }
            }
        }

        let selected = self.st.selection.contains(&clip.id);
        let stroke = if selected {
            Stroke::new(2.0, theme::ACCENT)
        } else {
            Stroke::new(1.0, Color32::from_black_alpha(160))
        };
        painter.rect_stroke(r, 2.0, stroke, StrokeKind::Inside);
        let name = if offline {
            format!("MEDIA OFFLINE — {}", clip_name(p, clip))
        } else {
            clip_name(p, clip)
        };
        let text_painter =
            painter.with_clip_rect(label.shrink2(vec2(3.0, 0.0)).intersect(painter.clip_rect()));
        text_painter.text(
            label.left_center() + vec2(5.0, 0.0),
            Align2::LEFT_CENTER,
            name,
            theme::text(11.0),
            Color32::from_gray(240),
        );
        if clip.speed != 1.0 {
            text_painter.text(
                label.right_center() - vec2(5.0, 0.0),
                Align2::RIGHT_CENTER,
                format!("{:.0}%", clip.speed * 100.0),
                theme::text(10.5),
                Color32::from_rgb(255, 210, 120),
            );
        }
    }

    fn track_headers(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        p: &Project,
        layout: &Layout,
        area: Rect,
    ) {
        let header = Rect::from_min_max(area.min, pos2(layout.left, area.bottom()));
        painter.rect_filled(header, 0.0, theme::BAR);
        painter.line_segment(
            [header.right_top(), header.right_bottom()],
            Stroke::new(1.0, theme::BORDER),
        );
        let below = Rect::from_min_max(
            pos2(area.left(), layout.lanes.top()),
            pos2(layout.left, area.bottom()),
        );
        let painter = &painter.with_clip_rect(below);

        for (row, rect) in &layout.rows {
            if rect.bottom() <= below.top() + 4.0 || rect.top() >= below.bottom() - 4.0 {
                continue;
            }
            let h = Rect::from_min_max(rect.min, pos2(layout.left - 1.0, rect.max.y));
            let Some(track) = p.tracks(row.kind).get(row.index).cloned() else {
                let label = if row.kind == TrackKind::Video {
                    "+ faixa de vídeo"
                } else {
                    "+ faixa de áudio"
                };
                let resp = ui.put(
                    h.shrink2(vec2(8.0, 1.0)),
                    egui::Button::new(egui::RichText::new(label).small().color(theme::TEXT_DIM))
                        .frame(false),
                );
                if resp.clicked() {
                    self.st.apply(Edit::AddTrack(row.kind));
                }
                continue;
            };
            painter.rect_filled(h, 0.0, Color32::from_rgb(0x25, 0x25, 0x29));
            painter.line_segment(
                [h.left_bottom(), h.right_bottom()],
                Stroke::new(1.0, theme::BORDER),
            );
            let label = p.track_label(track.id).unwrap_or_default();
            let (badge_color, name) = match row.kind {
                TrackKind::Video => (theme::CLIP_VIDEO_LABEL, format!("Vídeo {}", row.index + 1)),
                TrackKind::Audio => (theme::CLIP_AUDIO_LABEL, format!("Áudio {}", row.index + 1)),
            };
            let badge = Rect::from_min_size(h.min + vec2(8.0, 7.0), vec2(30.0, 18.0));
            painter.rect_filled(badge, 3.0, badge_color);
            painter.text(
                badge.center(),
                Align2::CENTER_CENTER,
                &label,
                theme::text(12.0),
                Color32::WHITE,
            );
            painter.text(
                pos2(badge.right() + 8.0, badge.center().y),
                Align2::LEFT_CENTER,
                &name,
                theme::text(12.0),
                theme::TEXT,
            );
            let count = track.clips.len();
            painter.text(
                pos2(h.right() - 8.0, badge.center().y),
                Align2::RIGHT_CENTER,
                format!("{count} clipe{}", if count == 1 { "" } else { "s" }),
                theme::text(10.5),
                theme::TEXT_DIM,
            );

            let mut flags = TrackFlags {
                locked: track.locked,
                enabled: track.enabled,
                muted: track.muted,
                solo: track.solo,
            };
            let mut changed = false;
            let buttons = Rect::from_min_max(
                pos2(h.left() + 6.0, h.bottom() - 25.0),
                pos2(h.right() - 6.0, h.bottom() - 2.0),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(buttons)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.set_clip_rect(below);
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let lock_icon = if track.locked {
                        Icon::Lock
                    } else {
                        Icon::Unlock
                    };
                    if icons::button(ui, lock_icon, 13.0, track.locked, "Travar faixa").clicked() {
                        flags.locked = !flags.locked;
                        changed = true;
                    }
                    match row.kind {
                        TrackKind::Video => {
                            if icons::button(
                                ui,
                                Icon::Monitor,
                                13.0,
                                !track.enabled,
                                if track.enabled {
                                    "Desligar faixa"
                                } else {
                                    "Ligar faixa"
                                },
                            )
                            .clicked()
                            {
                                flags.enabled = !flags.enabled;
                                changed = true;
                            }
                        }
                        TrackKind::Audio => {
                            let toggle = |ui: &mut egui::Ui,
                                          text: &str,
                                          on: bool,
                                          color: Color32,
                                          tip: &str| {
                                let b = egui::Button::new(
                                    egui::RichText::new(text).size(11.0).color(if on {
                                        Color32::BLACK
                                    } else {
                                        theme::TEXT
                                    }),
                                )
                                .fill(if on { color } else { theme::BAR_LIGHT })
                                .min_size(vec2(20.0, 18.0));
                                ui.add(b).on_hover_text(tip).clicked()
                            };
                            if toggle(ui, "M", track.muted, Color32::from_rgb(230, 80, 70), "Mudo")
                            {
                                flags.muted = !flags.muted;
                                changed = true;
                            }
                            if toggle(ui, "S", track.solo, Color32::from_rgb(235, 200, 70), "Solo")
                            {
                                flags.solo = !flags.solo;
                                changed = true;
                            }
                        }
                    }
                },
            );
            if changed {
                self.st.apply(Edit::SetTrackFlags {
                    track: track.id,
                    flags,
                });
            }
            let hresp = ui.interact(
                Rect::from_min_size(h.min, vec2(h.width(), 30.0)).intersect(below),
                ui.id().with(("hdr", track.id)),
                Sense::click(),
            );
            hresp.context_menu(|ui| {
                if ui.button(format!("Apagar faixa {label}")).clicked() {
                    self.st.apply(Edit::RemoveTrack(track.id));
                    ui.close();
                }
            });
        }
    }

    /// O projeto mais novo (se uma edição aconteceu neste frame) para desenhar sem atraso.
    fn project_arc_after_edits(&mut self, p: &std::sync::Arc<Project>) -> std::sync::Arc<Project> {
        if self.st.editor.revision() != self.project_rev {
            self.refresh_project_arc();
        }
        if self.st.editor.revision() == self.project_rev {
            self.project_arc.clone()
        } else {
            p.clone()
        }
    }

    fn timeline_toolbar(&mut self, ui: &mut egui::Ui) {
        theme::header_strip(ui, 32.0, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icons::button(
                ui,
                Icon::Select,
                15.0,
                self.st.tool == Tool::Selection,
                "Ferramenta de seleção (A)",
            )
            .clicked()
            {
                self.st.tool = Tool::Selection;
            }
            if icons::button(
                ui,
                Icon::Blade,
                15.0,
                self.st.tool == Tool::Blade,
                "Lâmina (B)",
            )
            .clicked()
            {
                self.st.tool = Tool::Blade;
            }
            ui.add_space(10.0);
            if icons::button(ui, Icon::Insert, 15.0, false, "Insert do Source (F9)").clicked() {
                self.st.place_from_source(PlaceMode::Insert);
            }
            if icons::button(
                ui,
                Icon::Overwrite,
                15.0,
                false,
                "Overwrite do Source (F10)",
            )
            .clicked()
            {
                self.st.place_from_source(PlaceMode::Overwrite);
            }
            if icons::button(
                ui,
                Icon::Append,
                15.0,
                false,
                "Append do Source (Shift+F12)",
            )
            .clicked()
            {
                self.st.place_from_source(PlaceMode::Append);
            }
            ui.add_space(10.0);
            if icons::button(ui, Icon::Magnet, 15.0, self.st.snapping, "Snapping (N)").clicked() {
                self.st.snapping = !self.st.snapping;
            }
            if icons::button(
                ui,
                Icon::Link,
                15.0,
                self.st.linked_selection,
                "Seleção ligada (Ctrl+Shift+L)",
            )
            .clicked()
            {
                self.st.linked_selection = !self.st.linked_selection;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if icons::button(ui, Icon::Fit, 15.0, false, "Zoom para caber (Shift+Z)").clicked()
                {
                    let w = self.tl_width;
                    self.zoom_to_fit(w);
                }
                if icons::button(ui, Icon::Plus, 13.0, false, "Zoom + (Ctrl+=)").clicked() {
                    self.zoom(1.4);
                }
                let mut z = self.tl.ppf.ln();
                let slider = egui::Slider::new(&mut z, 0.02f32.ln()..=40f32.ln()).show_value(false);
                if ui.add_sized([110.0, 16.0], slider).changed() {
                    let factor = z.exp() / self.tl.ppf;
                    self.zoom(factor);
                }
                if icons::button(ui, Icon::Minus, 13.0, false, "Zoom − (Ctrl+-)").clicked() {
                    self.zoom(1.0 / 1.4);
                }
            });
        });
    }

    fn clip_menu(&mut self, ui: &mut egui::Ui) {
        if self.st.selection.is_empty() {
            ui.label("Nenhum clipe");
            return;
        }
        if ui.button("Cortar no cursor").clicked() {
            self.st.blade_at_playhead();
            ui.close();
        }
        if ui.button("Apagar").clicked() {
            self.st.delete_selection(false);
            ui.close();
        }
        if ui.button("Apagar com ripple").clicked() {
            self.st.delete_selection(true);
            ui.close();
        }
        if ui.button("Ligar / separar áudio e vídeo").clicked() {
            self.st.toggle_link();
            ui.close();
        }
        if let Some(c) = self.tl.menu_clip
            && ui.button("Abrir no Source Viewer (match frame)").clicked()
        {
            let p = self.project_arc.clone();
            let at = self.st.playhead;
            self.match_frame(&p, c, at);
            ui.close();
        }
    }

    /// Alça de fade sob o ponteiro (cantos de cima do clipe).
    fn hit_fade(&self, p: &Project, layout: &Layout, pos: Pos2) -> Option<(ClipId, Edge)> {
        let (clip, _) = self.hit_clip(p, layout, pos)?;
        let rect = layout.rect_of(layout.row_at(pos.y)?)?;
        if pos.y > rect.top() + LABEL_H + 2.0 {
            return None;
        }
        let c = p.clip(clip)?;
        let x0 = self.x_of(c.start as f32, layout.left);
        let x1 = self.x_of(c.end() as f32, layout.left);
        let xin = (x0 + c.fade_in as f32 * self.tl.ppf).clamp(x0 + 4.0, x1 - 4.0);
        let xout = (x1 - c.fade_out as f32 * self.tl.ppf).clamp(x0 + 4.0, x1 - 4.0);
        if (pos.x - xin).abs() <= 6.0 {
            Some((clip, Edge::Start))
        } else if (pos.x - xout).abs() <= 6.0 {
            Some((clip, Edge::End))
        } else {
            None
        }
    }

    /// Clipe sob o ponteiro e, se for perto da borda, qual borda.
    fn hit_clip(&self, p: &Project, layout: &Layout, pos: Pos2) -> Option<(ClipId, Option<Edge>)> {
        if !layout.lanes.contains(pos) {
            return None;
        }
        let row = layout.row_at(pos.y)?;
        let track = p.tracks(row.kind).get(row.index)?;
        let f = self.frame_at(pos.x, layout.left);
        let clip = track
            .clips
            .iter()
            .find(|c| f >= c.start as f32 && f < c.end() as f32)?;
        let (x0, x1) = (
            self.x_of(clip.start as f32, layout.left),
            self.x_of(clip.end() as f32, layout.left),
        );
        let grab = EDGE_GRAB.min((x1 - x0) / 3.0);
        let edge = if pos.x - x0 <= grab {
            Some(Edge::Start)
        } else if x1 - pos.x <= grab {
            Some(Edge::End)
        } else {
            None
        };
        Some((clip.id, edge))
    }

    /// Delta de um arrasto, encaixando o início ou o fim do clipe âncora.
    fn move_delta(
        &self,
        p: &Project,
        anchor: ClipId,
        clips: &[ClipId],
        raw: Frame,
        tolerance: Frame,
    ) -> (Frame, Option<Frame>) {
        let Some(c) = p.clip(anchor) else {
            return (raw, None);
        };
        let min_start = clips
            .iter()
            .filter_map(|id| p.clip(*id))
            .map(|c| c.start)
            .min()
            .unwrap_or(0);
        let raw = raw.max(-min_start);
        let s = self.st.snap(c.start + raw, clips, tolerance) - c.start;
        let e = self.st.snap(c.end() + raw, clips, tolerance) - c.end();
        let (delta, at) = if (s - raw).abs() <= (e - raw).abs() {
            (s, c.start + s)
        } else {
            (e, c.end() + e)
        };
        let snapped = (delta != raw).then_some(at);
        (delta.max(-min_start), snapped)
    }

    fn move_ghosts(
        &self,
        p: &Project,
        layout: &Layout,
        clips: &[ClipId],
        kind: TrackKind,
        delta: Frame,
        rows: i64,
    ) -> Vec<(Rect, bool)> {
        let mut out = Vec::new();
        for id in clips {
            let (Some(c), Some(row)) = (p.clip(*id), row_of(p, *id)) else {
                continue;
            };
            let index = if row.kind == kind {
                (row.index as i64 + rows).max(0) as usize
            } else {
                row.index
            };
            let target = layout
                .rect_of(Row {
                    kind: row.kind,
                    index,
                })
                .or_else(|| {
                    layout.rect_of(Row {
                        kind: row.kind,
                        index: p.tracks(row.kind).len(),
                    })
                });
            let Some(rect) = target else { continue };
            let x0 = self.x_of((c.start + delta) as f32, layout.left);
            let x1 = self.x_of((c.end() + delta) as f32, layout.left);
            out.push((
                Rect::from_min_max(pos2(x0, rect.top() + 2.0), pos2(x1, rect.bottom() - 2.0)),
                false,
            ));
        }
        out
    }

    fn trim_ghost(
        &self,
        p: &Project,
        layout: &Layout,
        anchor: ClipId,
        edge: Edge,
        target: Frame,
    ) -> Option<Rect> {
        let (c, row) = (p.clip(anchor)?, row_of(p, anchor)?);
        let rect = layout.rect_of(row)?;
        let (s, e) = if edge == Edge::Start {
            (target, c.end())
        } else {
            (c.start, target)
        };
        Some(Rect::from_min_max(
            pos2(self.x_of(s as f32, layout.left), rect.top() + 2.0),
            pos2(self.x_of(e as f32, layout.left), rect.bottom() - 2.0),
        ))
    }

    fn paint_fades(&self, painter: &egui::Painter, clip: &Clip, r: Rect) {
        let shade = Color32::from_black_alpha(110);
        let line = Stroke::new(1.0, Color32::from_white_alpha(120));
        if clip.fade_in > 0 {
            let x = (r.left() + clip.fade_in as f32 * self.tl.ppf).min(r.right());
            painter.add(egui::Shape::convex_polygon(
                vec![r.left_top(), r.left_bottom(), pos2(x, r.top())],
                shade,
                Stroke::NONE,
            ));
            painter.line_segment([r.left_bottom(), pos2(x, r.top())], line);
        }
        if clip.fade_out > 0 {
            let x = (r.right() - clip.fade_out as f32 * self.tl.ppf).max(r.left());
            painter.add(egui::Shape::convex_polygon(
                vec![r.right_top(), pos2(x, r.top()), r.right_bottom()],
                shade,
                Stroke::NONE,
            ));
            painter.line_segment([pos2(x, r.top()), r.right_bottom()], line);
        }
    }

    fn paint_waveform(&self, painter: &egui::Painter, p: &Project, clip: &Clip, r: Rect) {
        let ClipSource::Media(m) = &clip.source else {
            return;
        };
        let Some(media) = p.media(*m) else { return };
        let ctx = self.ctx.clone();
        let Some(peaks) = self
            .waveforms
            .get(&media.info.path, move || ctx.request_repaint())
        else {
            return;
        };
        let fps = p.settings.fps.as_f64();
        let gain = 10f32.powf(clip.volume_db / 20.0);
        let mid = r.center().y;
        let half = r.height() / 2.0 - 2.0;
        let x_start = r.left().max(painter.clip_rect().left());
        let x_end = r.right().min(painter.clip_rect().right());
        let mut x = x_start;
        while x < x_end {
            let local = ((x - r.left()) / self.tl.ppf) as f64;
            let seconds = (clip.source_start + local * clip.speed) / fps;
            let i = (seconds * crate::engine::PEAKS_PER_SECOND) as usize;
            if let Some((lo, hi)) = peaks.get(i) {
                // Raiz quadrada: áudio baixo continua visível, como numa escala em dB.
                let shape = |v: f32| (v * gain).clamp(-1.0, 1.0).abs().sqrt().copysign(v);
                let (lo, hi) = (shape(*lo), shape(*hi));
                painter.line_segment(
                    [pos2(x, mid - hi * half), pos2(x, mid - lo * half)],
                    Stroke::new(1.0, theme::WAVE),
                );
            }
            x += 1.0;
        }
    }

    /// Ponto na linha da faixa (`kind`, `index`) no `frame`, na última timeline desenhada.
    #[cfg(test)]
    pub fn tl_point(&self, kind: TrackKind, index: usize, frame: f32) -> Pos2 {
        let layout = Layout::new(&self.project_arc, self.tl_area, self.tl.v_scroll);
        let r = layout.rect_of(Row { kind, index }).expect("linha existe");
        pos2(self.x_of(frame, layout.left), r.center().y)
    }

    /// Centro do botão de travar no cabeçalho da faixa (para os testes).
    #[cfg(test)]
    pub fn tl_lock_button(&self, kind: TrackKind, index: usize) -> Pos2 {
        let layout = Layout::new(&self.project_arc, self.tl_area, self.tl.v_scroll);
        let r = layout.rect_of(Row { kind, index }).expect("linha existe");
        pos2(r.left() + 6.0 + 10.5, r.bottom() - 13.5)
    }

    fn payload_len(&self, p: &Project, payload: DragPayload) -> Frame {
        let still = crate::state::default_still_frames(p);
        match payload {
            DragPayload::Media(m) => p.media(m).and_then(|m| m.info.duration).unwrap_or(still),
            DragPayload::Generator(_) => still,
            DragPayload::Dissolve => p.settings.fps.nominal(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruler_step_keeps_labels_apart() {
        assert_eq!(ruler_step(10.0, 30, 70.0), 10);
        assert_eq!(ruler_step(1.0, 30, 70.0), 150);
        assert_eq!(ruler_step(0.05, 30, 70.0), 1800);
    }
}
