//! Inspector: propriedades do clipe selecionado, com keyframes (◆) nos parâmetros animáveis.

use egui::{Color32, DragValue, RichText, Sense, Stroke, vec2};
use pawcut_core::{
    Clip, ClipChange, ClipSource, ColorAdjust, Crop, Edit, MediaKind, Param, Project, Rgba,
    TextAlign, TrackKind, Transform,
};

use crate::theme;
use crate::workspace::Workspace;

fn to_color32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn to_rgba(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Rgba::new(r, g, b, a)
}

fn color_button(ui: &mut egui::Ui, c: &mut Rgba) -> bool {
    let mut c32 = to_color32(*c);
    let changed = egui::color_picker::color_edit_button_srgba(
        ui,
        &mut c32,
        egui::color_picker::Alpha::OnlyBlend,
    )
    .changed();
    *c = to_rgba(c32);
    changed
}

/// Losango de keyframe: cheio se há keyframe no cursor, vazado se o parâmetro é animado.
fn diamond(ui: &mut egui::Ui, animated: bool, here: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
    let c = rect.center();
    let r = 5.0;
    let pts = vec![
        c + vec2(0.0, -r),
        c + vec2(r, 0.0),
        c + vec2(0.0, r),
        c + vec2(-r, 0.0),
    ];
    let color = if here || animated {
        theme::ACCENT
    } else if resp.hovered() {
        Color32::WHITE
    } else {
        theme::TEXT_DIM
    };
    if here {
        ui.painter()
            .add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    } else {
        ui.painter()
            .add(egui::Shape::closed_line(pts, Stroke::new(1.3, color)));
    }
    let tip = if here {
        "Apagar keyframe aqui"
    } else {
        "Criar keyframe no cursor"
    };
    resp.on_hover_text(tip)
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::BAR);
    ui.painter().text(
        rect.left_center() + vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        title,
        theme::text(12.0),
        Color32::WHITE,
    );
    ui.add_space(2.0);
    egui::Grid::new(title)
        .num_columns(3)
        .spacing(vec2(8.0, 5.0))
        .min_col_width(70.0)
        .show(ui, |ui| add(ui));
}

impl Workspace {
    /// Uma linha "nome · valor · ◆". Com keyframes, editar grava um keyframe no cursor.
    #[allow(clippy::too_many_arguments)]
    fn param_row(
        &mut self,
        ui: &mut egui::Ui,
        clip: &Clip,
        param: Param,
        label: &str,
        mul: f32,
        range: std::ops::RangeInclusive<f32>,
        suffix: &str,
    ) {
        let frame = self.st.playhead.clamp(clip.start, clip.end() - 1);
        let at = clip.source_time_at(frame);
        let keys = clip.keyframes.get(&param).cloned().unwrap_or_default();
        let animated = !keys.is_empty();
        let here = keys.iter().any(|k| (k.at - at).abs() < 0.5);
        let mut v = clip.param(param, frame) * mul;
        ui.label(label);
        let changed = ui
            .add(
                DragValue::new(&mut v)
                    .range(range)
                    .speed(0.5 * mul.max(0.2))
                    .suffix(suffix),
            )
            .changed();
        if diamond(ui, animated, here).clicked() {
            let value = if here {
                None
            } else {
                Some(clip.param(param, frame))
            };
            self.st.apply(Edit::SetKeyframe {
                clip: clip.id,
                param,
                at,
                value,
            });
        }
        ui.end_row();
        if !changed {
            return;
        }
        let value = v / mul;
        let key = format!("insp-{:?}-{param:?}", clip.id);
        if animated {
            self.st.apply_coalescing(
                Edit::SetKeyframe {
                    clip: clip.id,
                    param,
                    at,
                    value: Some(value),
                },
                &key,
            );
            return;
        }
        let mut t = clip.transform;
        let mut c = clip.color;
        let change = match param {
            Param::X => {
                t.x = value;
                ClipChange::Transform(t)
            }
            Param::Y => {
                t.y = value;
                ClipChange::Transform(t)
            }
            Param::Scale => {
                t.scale = value.max(0.01);
                ClipChange::Transform(t)
            }
            Param::Opacity => {
                t.opacity = value.clamp(0.0, 1.0);
                ClipChange::Transform(t)
            }
            Param::VolumeDb => ClipChange::VolumeDb(value),
            Param::Hue => {
                c.hue = value;
                ClipChange::Color(c)
            }
            Param::Saturation => {
                c.saturation = value;
                ClipChange::Color(c)
            }
            Param::Lightness => {
                c.lightness = value;
                ClipChange::Color(c)
            }
        };
        self.st.apply_coalescing(
            Edit::SetClip {
                clip: clip.id,
                change,
            },
            &key,
        );
    }

    pub fn inspector_ui(&mut self, ui: &mut egui::Ui) {
        let project = self.project_arc.clone();
        let Some(clip) = self
            .st
            .selection
            .first()
            .and_then(|c| project.clip(*c))
            .cloned()
        else {
            theme::header_strip(ui, 30.0, |ui| {
                ui.label(RichText::new("Inspector").strong());
            });
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Selecione um clipe na timeline").weak())
            });
            return;
        };
        let Some((track_id, index)) = project.locate(clip.id) else {
            return;
        };
        let kind = project.track(track_id).map_or(TrackKind::Video, |t| t.kind);
        let fps = project.settings.fps;
        let name = clip_title(&project, &clip);
        theme::header_strip(ui, 30.0, |ui| {
            ui.label(RichText::new(&name).strong());
        });
        ui.label(
            RichText::new(format!(
                "  {} · início {} · duração {}",
                project.track_label(track_id).unwrap_or_default(),
                pawcut_core::format_timecode(clip.start, fps),
                pawcut_core::format_timecode(clip.duration, fps)
            ))
            .weak()
            .small(),
        );
        let id = clip.id;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().interact_size.x = 64.0;
                match &clip.source {
                    ClipSource::Text(style) => self.text_section(ui, id, style),
                    ClipSource::SolidColor(color) => section(ui, "Gerador", |ui| {
                        ui.label("Cor");
                        let mut c = *color;
                        if color_button(ui, &mut c) {
                            self.st.apply_coalescing(
                                Edit::SetClip {
                                    clip: id,
                                    change: ClipChange::Source(ClipSource::SolidColor(c)),
                                },
                                &format!("insp-{id:?}-solid"),
                            );
                        }
                        ui.end_row();
                    }),
                    ClipSource::Media(_) => {}
                }
                if kind == TrackKind::Video {
                    section(ui, "Transform", |ui| {
                        self.param_row(
                            ui,
                            &clip,
                            Param::X,
                            "Posição X",
                            1.0,
                            -10000.0..=10000.0,
                            "",
                        );
                        self.param_row(
                            ui,
                            &clip,
                            Param::Y,
                            "Posição Y",
                            1.0,
                            -10000.0..=10000.0,
                            "",
                        );
                        self.param_row(ui, &clip, Param::Scale, "Zoom", 100.0, 1.0..=1000.0, " %");
                        self.param_row(
                            ui,
                            &clip,
                            Param::Opacity,
                            "Opacidade",
                            100.0,
                            0.0..=100.0,
                            " %",
                        );
                        ui.label("Espelhar");
                        ui.horizontal(|ui| {
                            let mut t = clip.transform;
                            let h = ui.toggle_value(&mut t.flip_h, "↔ Horizontal").changed();
                            let v = ui.toggle_value(&mut t.flip_v, "↕ Vertical").changed();
                            if h || v {
                                self.st.apply(Edit::SetClip {
                                    clip: id,
                                    change: ClipChange::Transform(t),
                                });
                            }
                        });
                        ui.end_row();
                        if ui.small_button("Resetar transform").clicked() {
                            for p in [Param::X, Param::Y, Param::Scale, Param::Opacity] {
                                self.st.apply(Edit::ClearKeyframes { clip: id, param: p });
                            }
                            self.st.apply(Edit::SetClip {
                                clip: id,
                                change: ClipChange::Transform(Transform::default()),
                            });
                        }
                        ui.end_row();
                    });
                    section(ui, "Recorte (crop)", |ui| {
                        let mut k = clip.transform.crop;
                        let mut changed = false;
                        for (label, v) in [
                            ("Esquerda", &mut k.left),
                            ("Direita", &mut k.right),
                            ("Topo", &mut k.top),
                            ("Base", &mut k.bottom),
                        ] {
                            ui.label(label);
                            let mut pct = *v * 100.0;
                            if ui
                                .add(
                                    DragValue::new(&mut pct)
                                        .range(0.0..=90.0)
                                        .speed(0.3)
                                        .suffix(" %"),
                                )
                                .changed()
                            {
                                *v = pct / 100.0;
                                changed = true;
                            }
                            ui.end_row();
                        }
                        if changed && k.left + k.right < 0.95 && k.top + k.bottom < 0.95 {
                            let t = Transform {
                                crop: k,
                                ..clip.transform
                            };
                            self.st.apply_coalescing(
                                Edit::SetClip {
                                    clip: id,
                                    change: ClipChange::Transform(t),
                                },
                                &format!("insp-{id:?}-crop"),
                            );
                        }
                        if ui.small_button("Sem recorte").clicked() {
                            let t = Transform {
                                crop: Crop::default(),
                                ..clip.transform
                            };
                            self.st.apply(Edit::SetClip {
                                clip: id,
                                change: ClipChange::Transform(t),
                            });
                        }
                        ui.end_row();
                    });
                    section(ui, "Cor — Matiz/Saturação/Luminosidade (Ctrl+U)", |ui| {
                        self.param_row(ui, &clip, Param::Hue, "Matiz", 1.0, -180.0..=180.0, "°");
                        self.param_row(
                            ui,
                            &clip,
                            Param::Saturation,
                            "Saturação",
                            1.0,
                            -100.0..=100.0,
                            "",
                        );
                        self.param_row(
                            ui,
                            &clip,
                            Param::Lightness,
                            "Luminosidade",
                            1.0,
                            -100.0..=100.0,
                            "",
                        );
                        if ui.small_button("Resetar cor").clicked() {
                            for p in [Param::Hue, Param::Saturation, Param::Lightness] {
                                self.st.apply(Edit::ClearKeyframes { clip: id, param: p });
                            }
                            self.st.apply(Edit::SetClip {
                                clip: id,
                                change: ClipChange::Color(ColorAdjust::default()),
                            });
                        }
                        ui.end_row();
                    });
                }
                section(ui, "Tempo", |ui| {
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
                                &format!("insp-{id:?}-speed"),
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
                            &format!("insp-{id:?}-fi"),
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
                            &format!("insp-{id:?}-fo"),
                        );
                    }
                    ui.end_row();
                    let track = project.track(track_id);
                    let joined =
                        index > 0 && track.is_some_and(|t| t.clips[index - 1].end() == clip.start);
                    if joined {
                        ui.label("Dissolve");
                        let mut d = clip.dissolve_in;
                        if ui
                            .add(
                                DragValue::new(&mut d)
                                    .range(0..=clip.duration)
                                    .suffix(" fr"),
                            )
                            .on_hover_text("Cross dissolve com o clipe anterior (Ctrl+T)")
                            .changed()
                        {
                            self.st.apply_coalescing(
                                Edit::SetDissolve {
                                    clip: id,
                                    frames: d,
                                },
                                &format!("insp-{id:?}-dissolve"),
                            );
                        }
                        ui.end_row();
                    }
                });
                if kind == TrackKind::Audio {
                    section(ui, "Áudio", |ui| {
                        self.param_row(
                            ui,
                            &clip,
                            Param::VolumeDb,
                            "Volume",
                            1.0,
                            -60.0..=24.0,
                            " dB",
                        );
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

    fn text_section(
        &mut self,
        ui: &mut egui::Ui,
        id: pawcut_core::ClipId,
        style: &pawcut_core::TextStyle,
    ) {
        let mut s = style.clone();
        let mut changed = false;
        section(ui, "Título", |ui| {
            ui.label("Texto");
            changed |= ui
                .add(
                    egui::TextEdit::multiline(&mut s.text)
                        .desired_rows(2)
                        .desired_width(180.0),
                )
                .changed();
            ui.end_row();
            ui.label("Fonte");
            changed |= ui
                .add(egui::TextEdit::singleline(&mut s.font).desired_width(120.0))
                .changed();
            ui.end_row();
            ui.label("Tamanho");
            changed |= ui
                .add(DragValue::new(&mut s.size).range(4.0..=600.0).speed(0.5))
                .changed();
            ui.end_row();
            ui.label("Cor");
            changed |= color_button(ui, &mut s.color);
            ui.end_row();
            ui.label("Estilo");
            ui.horizontal(|ui| {
                changed |= ui
                    .toggle_value(&mut s.bold, RichText::new("N").strong())
                    .on_hover_text("Negrito")
                    .changed();
                for (a, label) in [
                    (TextAlign::Left, "⬅"),
                    (TextAlign::Center, "↔"),
                    (TextAlign::Right, "➡"),
                ] {
                    changed |= ui.selectable_value(&mut s.align, a, label).changed();
                }
            });
            ui.end_row();
            ui.label("Contorno");
            ui.horizontal(|ui| {
                changed |= ui
                    .add(
                        DragValue::new(&mut s.outline)
                            .range(0.0..=30.0)
                            .speed(0.2)
                            .suffix(" px"),
                    )
                    .changed();
                changed |= color_button(ui, &mut s.outline_color);
            });
            ui.end_row();
            ui.label("Sombra");
            changed |= ui.checkbox(&mut s.shadow, "").changed();
            ui.end_row();
            ui.label("Caixa");
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut s.background, "").changed();
                changed |= color_button(ui, &mut s.background_color);
            });
            ui.end_row();
        });
        if changed {
            self.st.apply_coalescing(
                Edit::SetClip {
                    clip: id,
                    change: ClipChange::Source(ClipSource::Text(s)),
                },
                &format!("insp-{id:?}-text"),
            );
        }
    }
}

fn clip_title(p: &Project, clip: &Clip) -> String {
    match &clip.source {
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
        ClipSource::Text(_) => "Texto".into(),
    }
}
