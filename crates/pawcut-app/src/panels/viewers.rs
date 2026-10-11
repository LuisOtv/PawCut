//! Source Viewer e Timeline Viewer.

use egui::{Color32, Rect, RichText, Sense, Vec2};
use pawcut_core::{ClipChange, ClipSource, Frame, MediaKind, Transform, format_timecode};

use crate::engine::{ViewRequest, ViewSlot};
use crate::state::{Monitor, PlaceMode};
use crate::workspace::{Playback, Workspace};

/// Retângulo com a proporção `aspect` centralizado em `area`.
fn fit_rect(area: Rect, aspect: f32) -> Rect {
    let w = area.width().min(area.height() * aspect).max(1.0);
    Rect::from_center_size(area.center(), Vec2::new(w, w / aspect))
}

impl Workspace {
    fn monitor_frame(&self, ui: &egui::Ui, active: bool, rect: Rect) {
        let color = if active {
            Color32::from_rgb(220, 140, 40)
        } else {
            Color32::from_gray(50)
        };
        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
    }

    pub fn timeline_viewer_ui(&mut self, ui: &mut egui::Ui) {
        let project = self.project_arc.clone();
        let s = &project.settings;
        let aspect = s.width as f32 / s.height as f32;
        let transport_h = 30.0;
        let area = ui.available_rect_before_wrap();
        let image_area =
            Rect::from_min_max(area.min, egui::pos2(area.max.x, area.max.y - transport_h));
        let pic = fit_rect(image_area.shrink(4.0), aspect);
        let resp = ui.allocate_rect(image_area, Sense::click_and_drag());
        ui.painter()
            .rect_filled(image_area, 0.0, Color32::from_gray(12));
        if let Some(tex) = &self.tex_timeline {
            ui.painter().image(
                tex.id(),
                pic,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        self.monitor_frame(ui, self.st.active_monitor == Monitor::Timeline, image_area);

        // Pede o frame no tamanho em que ele aparece (menor enquanto toca).
        let ppp = ui.ctx().pixels_per_point();
        let mut w = (pic.width() * ppp) as u32;
        if self.playback.is_playing() {
            w = w.min(s.width / 2);
        }
        let w = w.clamp(16, s.width);
        let h = ((w as f32 / aspect).round() as u32).max(2);
        let key = (self.st.editor.revision(), self.st.playhead, (w, h));
        if self.requested_timeline != Some(key) {
            self.requested_timeline = Some(key);
            self.preview.request(
                ViewSlot::Timeline,
                ViewRequest::Timeline {
                    project: project.clone(),
                    frame: self.st.playhead,
                },
                (w, h),
            );
        }

        if resp.clicked() || resp.drag_started() {
            self.st.active_monitor = Monitor::Timeline;
        }
        // Arrastar no viewer move o clipe visual selecionado (posição do Inspector).
        if resp.dragged()
            && let Some(clip) = self.visual_selection(self.st.playhead)
        {
            let k = s.width as f32 / pic.width();
            let d = resp.drag_delta() * k;
            if d != Vec2::ZERO
                && let Some(c) = project.clip(clip)
            {
                let t = Transform {
                    x: c.transform.x + d.x,
                    y: c.transform.y - d.y,
                    ..c.transform
                };
                self.st.apply_coalescing(
                    pawcut_core::Edit::SetClip {
                        clip,
                        change: ClipChange::Transform(t),
                    },
                    &format!("viewer-{clip:?}"),
                );
            }
        }
        if self.visual_selection(self.st.playhead).is_some() {
            resp.on_hover_cursor(egui::CursorIcon::Move);
        }

        ui.scope_builder(
            egui::UiBuilder::new().max_rect(Rect::from_min_max(
                egui::pos2(area.min.x, area.max.y - transport_h),
                area.max,
            )),
            |ui| {
                ui.horizontal_centered(|ui| {
                    self.transport_buttons(ui, Monitor::Timeline);
                    ui.separator();
                    ui.label(
                        RichText::new(format_timecode(self.st.playhead, s.fps))
                            .monospace()
                            .size(15.0),
                    );
                    ui.label(
                        RichText::new(format!("/ {}", format_timecode(project.duration(), s.fps)))
                            .monospace()
                            .weak(),
                    );
                    if let (Some(i), Some(o)) = (self.st.mark_in, self.st.mark_out) {
                        ui.label(
                            RichText::new(format!(
                                "In {} Out {}",
                                format_timecode(i, s.fps),
                                format_timecode(o, s.fps)
                            ))
                            .small()
                            .weak(),
                        );
                    }
                });
            },
        );
    }

    /// Clipe de vídeo selecionado visível no frame (o de cima), se houver.
    fn visual_selection(&self, frame: Frame) -> Option<pawcut_core::ClipId> {
        let p = &self.project_arc;
        p.video_tracks
            .iter()
            .rev()
            .filter_map(|t| t.clip_at(frame))
            .find(|c| self.st.selection.contains(&c.id))
            .map(|c| c.id)
    }

    pub fn source_viewer_ui(&mut self, ui: &mut egui::Ui) {
        let project = self.project_arc.clone();
        let fps = project.settings.fps;
        let Some(media) = self
            .st
            .source
            .media
            .and_then(|id| project.media(id))
            .cloned()
        else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Clique numa mídia do Media Pool para ver aqui").weak());
            });
            return;
        };
        let len = self.st.source_length().unwrap_or(1).max(1);
        let aspect = if media.info.height > 0 {
            media.info.width as f32 / media.info.height as f32
        } else {
            16.0 / 9.0
        };
        let bottom_h = 58.0;
        let area = ui.available_rect_before_wrap();
        let image_area =
            Rect::from_min_max(area.min, egui::pos2(area.max.x, area.max.y - bottom_h));
        let resp = ui.allocate_rect(image_area, Sense::click());
        ui.painter()
            .rect_filled(image_area, 0.0, Color32::from_gray(12));
        if media.info.kind == MediaKind::Audio {
            ui.painter().text(
                image_area.center(),
                egui::Align2::CENTER_CENTER,
                "🎵 Áudio",
                egui::FontId::proportional(22.0),
                Color32::GRAY,
            );
        } else if let Some(tex) = &self.tex_source {
            let pic = fit_rect(image_area.shrink(4.0), aspect);
            ui.painter().image(
                tex.id(),
                pic,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            let ppp = ui.ctx().pixels_per_point();
            let w = ((pic.width() * ppp) as u32).clamp(16, project.settings.width);
            let h = ((w as f32 / aspect) as u32).max(2);
            let key = (media.id, self.st.source.playhead, (w, h));
            if self.requested_source != Some(key) {
                self.requested_source = Some(key);
                let seconds = self.st.source.playhead as f64 / fps.as_f64();
                self.preview.request(
                    ViewSlot::Source,
                    ViewRequest::Source {
                        path: media.info.path.clone(),
                        kind: media.info.kind,
                        seconds,
                    },
                    (w, h),
                );
            }
        } else {
            let seconds = self.st.source.playhead as f64 / fps.as_f64();
            self.requested_source = Some((media.id, self.st.source.playhead, (640, 360)));
            self.preview.request(
                ViewSlot::Source,
                ViewRequest::Source {
                    path: media.info.path.clone(),
                    kind: media.info.kind,
                    seconds,
                },
                (640, 360),
            );
        }
        self.monitor_frame(ui, self.st.active_monitor == Monitor::Source, image_area);
        if resp.clicked() {
            self.st.active_monitor = Monitor::Source;
        }

        let bottom = Rect::from_min_max(egui::pos2(area.min.x, area.max.y - bottom_h), area.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(bottom), |ui| {
            // Barra de scrub com as marcas In/Out.
            let (bar, bresp) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), 16.0),
                Sense::click_and_drag(),
            );
            let painter = ui.painter();
            painter.rect_filled(bar, 2.0, Color32::from_gray(35));
            let x_of = |f: Frame| bar.left() + bar.width() * (f as f32 / len as f32);
            let (mi, mo) = (
                self.st.source.mark_in.unwrap_or(0),
                self.st.source.mark_out.unwrap_or(len),
            );
            if self.st.source.mark_in.is_some() || self.st.source.mark_out.is_some() {
                painter.rect_filled(
                    Rect::from_min_max(
                        egui::pos2(x_of(mi), bar.top()),
                        egui::pos2(x_of(mo), bar.bottom()),
                    ),
                    0.0,
                    Color32::from_rgba_unmultiplied(220, 140, 40, 70),
                );
            }
            let px = x_of(self.st.source.playhead);
            painter.line_segment(
                [egui::pos2(px, bar.top()), egui::pos2(px, bar.bottom())],
                egui::Stroke::new(2.0, Color32::from_rgb(230, 60, 60)),
            );
            if (bresp.clicked() || bresp.dragged())
                && let Some(pos) = bresp.interact_pointer_pos()
            {
                let f = ((pos.x - bar.left()) / bar.width() * len as f32) as Frame;
                self.st.source.playhead = f.clamp(0, len - 1);
                self.st.active_monitor = Monitor::Source;
            }
            ui.horizontal(|ui| {
                self.transport_buttons(ui, Monitor::Source);
                ui.label(RichText::new(format_timecode(self.st.source.playhead, fps)).monospace());
                ui.separator();
                if ui
                    .small_button("In")
                    .on_hover_text("Marcar entrada (I)")
                    .clicked()
                {
                    self.st.active_monitor = Monitor::Source;
                    self.st.mark(true);
                }
                if ui
                    .small_button("Out")
                    .on_hover_text("Marcar saída (O)")
                    .clicked()
                {
                    self.st.active_monitor = Monitor::Source;
                    self.st.mark(false);
                }
                ui.separator();
                if ui.small_button("Insert").on_hover_text("F9").clicked() {
                    self.st.place_from_source(PlaceMode::Insert);
                }
                if ui.small_button("Overwrite").on_hover_text("F10").clicked() {
                    self.st.place_from_source(PlaceMode::Overwrite);
                }
                if ui
                    .small_button("Append")
                    .on_hover_text("Shift+F12")
                    .clicked()
                {
                    self.st.place_from_source(PlaceMode::Append);
                }
            });
        });
        let _ = ClipSource::SolidColor;
    }

    fn transport_buttons(&mut self, ui: &mut egui::Ui, monitor: Monitor) {
        let playing = self.playback.is_playing() && self.st.active_monitor == monitor;
        if ui
            .small_button("⏮")
            .on_hover_text("Início (Home)")
            .clicked()
        {
            self.st.active_monitor = monitor;
            self.go_to_start();
        }
        if ui
            .small_button("⏴")
            .on_hover_text("Voltar um frame (←)")
            .clicked()
        {
            self.st.active_monitor = monitor;
            self.stop();
            self.st.step(-1);
        }
        if ui
            .small_button(if playing { "⏸" } else { "▶" })
            .on_hover_text("Tocar/pausar (Espaço)")
            .clicked()
        {
            self.st.active_monitor = monitor;
            self.toggle_play();
        }
        if ui
            .small_button("⏵")
            .on_hover_text("Avançar um frame (→)")
            .clicked()
        {
            self.st.active_monitor = monitor;
            self.stop();
            self.st.step(1);
        }
        if ui.small_button("⏭").on_hover_text("Fim (End)").clicked() {
            self.st.active_monitor = monitor;
            self.go_to_end();
        }
        let _ = Playback::Stopped;
    }
}
