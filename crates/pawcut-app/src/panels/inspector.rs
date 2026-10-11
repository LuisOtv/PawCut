//! Inspector: propriedades do clipe selecionado.

use egui::{Color32, DragValue, RichText};
use pawcut_core::{ClipChange, ClipSource, Edit, MediaKind, Rgba, TrackKind, Transform};

use crate::workspace::Workspace;

fn to_color32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn to_rgba(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Rgba::new(r, g, b, a)
}

impl Workspace {
    pub fn inspector_ui(&mut self, ui: &mut egui::Ui) {
        let project = self.project_arc.clone();
        let Some(clip) = self
            .st
            .selection
            .first()
            .and_then(|c| project.clip(*c))
            .cloned()
        else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Selecione um clipe na timeline").weak())
            });
            return;
        };
        let Some((track_id, _)) = project.locate(clip.id) else {
            return;
        };
        let kind = project.track(track_id).map_or(TrackKind::Video, |t| t.kind);
        let fps = project.settings.fps;
        let id = clip.id;
        let key = |field: &str| format!("insp-{id:?}-{field}");

        let name = match &clip.source {
            ClipSource::Media(m) => project
                .media(*m)
                .and_then(|m| {
                    m.info
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "?".into()),
            ClipSource::SolidColor(_) => "Cor sólida".into(),
            ClipSource::Text(_) => "Texto".into(),
        };
        ui.label(RichText::new(name).strong().size(15.0));
        ui.label(
            RichText::new(format!(
                "{} · início {} · duração {}",
                project.track_label(track_id).unwrap_or_default(),
                pawcut_core::format_timecode(clip.start, fps),
                pawcut_core::format_timecode(clip.duration, fps)
            ))
            .weak()
            .small(),
        );
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                // ----- gerador -----
                match &clip.source {
                    ClipSource::Text(style) => {
                        egui::CollapsingHeader::new("Título")
                            .default_open(true)
                            .show(ui, |ui| {
                                let mut s = style.clone();
                                let mut changed = ui
                                    .add(egui::TextEdit::multiline(&mut s.text).desired_rows(2))
                                    .changed();
                                egui::Grid::new("text-grid").num_columns(2).show(ui, |ui| {
                                    ui.label("Tamanho");
                                    changed |= ui
                                        .add(
                                            DragValue::new(&mut s.size)
                                                .range(4.0..=600.0)
                                                .speed(0.5),
                                        )
                                        .changed();
                                    ui.end_row();
                                    ui.label("Fonte");
                                    changed |= ui.text_edit_singleline(&mut s.font).changed();
                                    ui.end_row();
                                    ui.label("Cor");
                                    let mut c = to_color32(s.color);
                                    changed |= egui::color_picker::color_edit_button_srgba(
                                        ui,
                                        &mut c,
                                        egui::color_picker::Alpha::OnlyBlend,
                                    )
                                    .changed();
                                    s.color = to_rgba(c);
                                    ui.end_row();
                                });
                                if changed {
                                    self.st.apply_coalescing(
                                        Edit::SetClip {
                                            clip: id,
                                            change: ClipChange::Source(ClipSource::Text(s)),
                                        },
                                        &key("text"),
                                    );
                                }
                            });
                    }
                    ClipSource::SolidColor(color) => {
                        egui::CollapsingHeader::new("Gerador")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Cor");
                                    let mut c = to_color32(*color);
                                    if egui::color_picker::color_edit_button_srgba(
                                        ui,
                                        &mut c,
                                        egui::color_picker::Alpha::OnlyBlend,
                                    )
                                    .changed()
                                    {
                                        self.st.apply_coalescing(
                                            Edit::SetClip {
                                                clip: id,
                                                change: ClipChange::Source(ClipSource::SolidColor(
                                                    to_rgba(c),
                                                )),
                                            },
                                            &key("color"),
                                        );
                                    }
                                });
                            });
                    }
                    ClipSource::Media(_) => {}
                }

                // ----- vídeo -----
                if kind == TrackKind::Video {
                    egui::CollapsingHeader::new("Transform")
                        .default_open(true)
                        .show(ui, |ui| {
                            let mut t: Transform = clip.transform;
                            let mut scale = t.scale * 100.0;
                            let mut opacity = t.opacity * 100.0;
                            let mut changed = false;
                            egui::Grid::new("transform-grid")
                                .num_columns(2)
                                .show(ui, |ui| {
                                    ui.label("Posição X");
                                    changed |=
                                        ui.add(DragValue::new(&mut t.x).speed(1.0)).changed();
                                    ui.end_row();
                                    ui.label("Posição Y");
                                    changed |=
                                        ui.add(DragValue::new(&mut t.y).speed(1.0)).changed();
                                    ui.end_row();
                                    ui.label("Zoom");
                                    changed |= ui
                                        .add(
                                            DragValue::new(&mut scale)
                                                .range(1.0..=1000.0)
                                                .speed(0.5)
                                                .suffix(" %"),
                                        )
                                        .changed();
                                    ui.end_row();
                                    ui.label("Opacidade");
                                    changed |= ui
                                        .add(
                                            DragValue::new(&mut opacity)
                                                .range(0.0..=100.0)
                                                .speed(0.5)
                                                .suffix(" %"),
                                        )
                                        .changed();
                                    ui.end_row();
                                });
                            if ui.small_button("Resetar transform").clicked() {
                                t = Transform::default();
                                scale = 100.0;
                                opacity = 100.0;
                                changed = true;
                            }
                            if changed {
                                t.scale = (scale / 100.0).max(0.01);
                                t.opacity = (opacity / 100.0).clamp(0.0, 1.0);
                                self.st.apply_coalescing(
                                    Edit::SetClip {
                                        clip: id,
                                        change: ClipChange::Transform(t),
                                    },
                                    &key("transform"),
                                );
                            }
                        });
                }

                // ----- tempo -----
                egui::CollapsingHeader::new("Tempo")
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::Grid::new("time-grid").num_columns(2).show(ui, |ui| {
                            let still = match &clip.source {
                                ClipSource::Media(m) => project
                                    .media(*m)
                                    .is_some_and(|m| m.info.kind == MediaKind::Image),
                                _ => true,
                            };
                            if !still {
                                ui.label("Velocidade");
                                let mut pct = clip.speed * 100.0;
                                if ui
                                    .add(
                                        DragValue::new(&mut pct)
                                            .range(10.0..=1000.0)
                                            .speed(1.0)
                                            .suffix(" %"),
                                    )
                                    .changed()
                                {
                                    let clips = self.st.expanded(&[id]);
                                    self.st.apply_coalescing(
                                        Edit::SetSpeed {
                                            clips,
                                            speed: pct / 100.0,
                                        },
                                        &key("speed"),
                                    );
                                }
                                ui.end_row();
                            }
                            ui.label("Fade in");
                            let mut fi = clip.fade_in;
                            if ui
                                .add(
                                    DragValue::new(&mut fi)
                                        .range(0..=clip.duration)
                                        .suffix(" fr"),
                                )
                                .changed()
                            {
                                self.st.apply_coalescing(
                                    Edit::SetClip {
                                        clip: id,
                                        change: ClipChange::FadeIn(fi),
                                    },
                                    &key("fadein"),
                                );
                            }
                            ui.end_row();
                            ui.label("Fade out");
                            let mut fo = clip.fade_out;
                            if ui
                                .add(
                                    DragValue::new(&mut fo)
                                        .range(0..=clip.duration)
                                        .suffix(" fr"),
                                )
                                .changed()
                            {
                                self.st.apply_coalescing(
                                    Edit::SetClip {
                                        clip: id,
                                        change: ClipChange::FadeOut(fo),
                                    },
                                    &key("fadeout"),
                                );
                            }
                            ui.end_row();
                        });
                    });

                // ----- áudio -----
                if kind == TrackKind::Audio {
                    egui::CollapsingHeader::new("Áudio")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Volume");
                                let mut db = clip.volume_db;
                                if ui
                                    .add(
                                        egui::Slider::new(&mut db, -60.0..=12.0)
                                            .suffix(" dB")
                                            .step_by(0.1),
                                    )
                                    .changed()
                                {
                                    self.st.apply_coalescing(
                                        Edit::SetClip {
                                            clip: id,
                                            change: ClipChange::VolumeDb(db),
                                        },
                                        &key("volume"),
                                    );
                                }
                            });
                        });
                }

                if clip.link.is_some() {
                    ui.add_space(6.0);
                    if ui.button("Separar áudio e vídeo").clicked() {
                        self.st.apply(Edit::Unlink {
                            clips: project.expand_linked(&[id]),
                        });
                    }
                }
            });
    }
}
