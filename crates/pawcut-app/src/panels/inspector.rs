//! Inspector: propriedades dos clipes selecionados, em grade fixa (rótulo · valor · ◆ · ↺).
//! Com vários clipes selecionados, mudar um valor muda em todos (num passo só de desfazer).

use egui::{Color32, DragValue, RichText, Sense, Stroke, vec2};
use pawcut_core::{
    Clip, ClipChange, ClipSource, ColorAdjust, Crop, Edge, Edit, MediaKind, Param, Project, Rgba,
    TextAlign, TrackKind, Transform, format_timecode, parse_timecode,
};

use crate::theme::{self, row, section};
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
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
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
    resp.on_hover_text(if here {
        "Apagar keyframe aqui"
    } else {
        "Criar keyframe no cursor"
    })
}

/// Botão ↺ (voltar ao padrão).
fn reset(ui: &mut egui::Ui, changed_from_default: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    let color = if !changed_from_default {
        theme::BAR_LIGHT
    } else if resp.hovered() {
        Color32::WHITE
    } else {
        theme::TEXT_DIM
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "↺",
        theme::text(14.0),
        color,
    );
    changed_from_default && resp.on_hover_text("Voltar ao padrão").clicked()
}

fn ease_label(e: pawcut_core::Ease) -> &'static str {
    match e {
        pawcut_core::Ease::Linear => "Linear",
        pawcut_core::Ease::Smooth => "Suave (entrada e saída)",
        pawcut_core::Ease::EaseIn => "Suave na entrada (acelera)",
        pawcut_core::Ease::EaseOut => "Suave na saída (desacelera)",
    }
}

/// Escalas que ajustam (cabe inteiro) e preenchem (cobre o quadro) a mídia no projeto.
fn fit_fill_scales(p: &Project, clip: &Clip) -> Option<(f32, f32)> {
    let ClipSource::Media(m) = &clip.source else {
        return None;
    };
    let m = p.media(*m)?;
    let (pw, ph) = (p.settings.width as f32, p.settings.height as f32);
    let (w, h) = (m.info.width.max(1) as f32, m.info.height.max(1) as f32);
    // Mesmo tamanho-base do renderizador: vídeo cabe inteiro; imagem só diminui.
    let contain = (pw / w).min(ph / h);
    let s = if m.info.kind == MediaKind::Image {
        contain.min(1.0)
    } else {
        contain
    };
    let (bw, bh) = (w * s, h * s);
    Some(((pw / bw).min(ph / bh), (pw / bw).max(ph / bh)))
}

fn default_of(param: Param) -> f32 {
    match param {
        Param::Scale | Param::Opacity => 1.0,
        _ => 0.0,
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

/// Uma edição de um parâmetro estático (sem keyframes) num clipe.
fn static_change(clip: &Clip, param: Param, value: f32) -> ClipChange {
    let mut t = clip.transform;
    let mut c = clip.color;
    match param {
        Param::X => t.x = value,
        Param::Y => t.y = value,
        Param::Scale => t.scale = value.max(0.01),
        Param::Opacity => t.opacity = value.clamp(0.0, 1.0),
        Param::VolumeDb => return ClipChange::VolumeDb(value),
        Param::Hue => c.hue = value,
        Param::Saturation => c.saturation = value,
        Param::Lightness => c.lightness = value,
        Param::Rotation => t.rotation = value,
    }
    match param {
        Param::Hue | Param::Saturation | Param::Lightness => ClipChange::Color(c),
        _ => ClipChange::Transform(t),
    }
}

/// Abas do Inspector, como no Resolve (Video / Audio / Color …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorTab {
    Title,
    #[default]
    Video,
    Color,
    Audio,
    Time,
}

impl InspectorTab {
    fn label(self) -> &'static str {
        match self {
            InspectorTab::Title => "Título",
            InspectorTab::Video => "Vídeo",
            InspectorTab::Color => "Cor",
            InspectorTab::Audio => "Áudio",
            InspectorTab::Time => "Tempo",
        }
    }
}

/// Fileira de abas com a mesma largura; devolve a aba clicada.
fn tab_bar(
    ui: &mut egui::Ui,
    tabs: &[InspectorTab],
    current: InspectorTab,
) -> Option<InspectorTab> {
    let rect = egui::Rect::from_min_size(
        ui.cursor().min,
        egui::vec2(ui.available_width(), theme::TOOLBAR_H),
    );
    ui.painter().rect_filled(rect, 0.0, theme::BG_DARK);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, theme::BORDER),
    );
    let w = rect.width() / tabs.len().max(1) as f32;
    let mut clicked = None;
    for (i, t) in tabs.iter().enumerate() {
        let r = egui::Rect::from_min_size(
            rect.min + egui::vec2(i as f32 * w, 0.0),
            egui::vec2(w, rect.height()),
        );
        let resp = ui.interact(r, ui.id().with(("insp-tab", i)), egui::Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t.label()));
        let on = *t == current;
        if on {
            ui.painter().rect_filled(r, 0.0, theme::BAR);
            ui.painter().line_segment(
                [
                    r.left_bottom() + egui::vec2(6.0, -1.0),
                    r.right_bottom() + egui::vec2(-6.0, -1.0),
                ],
                egui::Stroke::new(2.0, theme::ACCENT),
            );
        }
        ui.painter().text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            t.label(),
            theme::text(12.0),
            if on || resp.hovered() {
                egui::Color32::WHITE
            } else {
                theme::TEXT_DIM
            },
        );
        if resp.clicked() {
            clicked = Some(*t);
        }
    }
    ui.advance_cursor_after_rect(rect);
    clicked
}

impl Workspace {
    /// Linha de um parâmetro animável, aplicada a todos os `clips` (o primeiro manda no valor mostrado).
    #[allow(clippy::too_many_arguments)]
    fn param_row(
        &mut self,
        ui: &mut egui::Ui,
        clips: &[Clip],
        param: Param,
        label: &str,
        mul: f32,
        range: std::ops::RangeInclusive<f32>,
        suffix: &str,
    ) {
        let first = &clips[0];
        let ph = self.st.playhead;
        let frame_of = move |c: &Clip| ph.clamp(c.start, c.end() - 1);
        let frame = frame_of(first);
        let at = first.source_time_at(frame);
        let keys = first.keyframes.get(&param).cloned().unwrap_or_default();
        let animated = !keys.is_empty();
        let here = keys.iter().any(|k| (k.at - at).abs() < 0.5);
        let current = first.param(param, frame);
        let mixed = clips
            .iter()
            .any(|c| (c.param(param, frame_of(c)) - current).abs() > 1e-4);
        let mut v = current * mul;
        let mut changed = false;
        let mut toggle_key = false;
        let mut do_reset = false;
        let mut pick_ease: Option<pawcut_core::Ease> = None;
        let ease_here = keys
            .iter()
            .find(|k| (k.at - at).abs() < 0.5)
            .map(|k| k.ease);
        row(
            ui,
            label,
            |ui| {
                changed = ui
                    .add(
                        DragValue::new(&mut v)
                            .range(range)
                            .speed(0.5 * mul.max(0.2))
                            .suffix(suffix),
                    )
                    .changed();
                if mixed {
                    ui.label(RichText::new("vários").small().color(theme::TEXT_DIM));
                }
            },
            |ui| {
                let d = diamond(ui, animated, here);
                toggle_key = d.clicked();
                // Botão direito no ◆: curva de chegada deste keyframe.
                if let Some(current) = ease_here {
                    d.context_menu(|ui| {
                        ui.label(RichText::new("Curva até este keyframe").weak());
                        for e in pawcut_core::Ease::ALL {
                            if ui.radio(current == e, ease_label(e)).clicked() {
                                pick_ease = Some(e);
                                ui.close();
                            }
                        }
                    });
                }
                do_reset = reset(ui, animated || (current - default_of(param)).abs() > 1e-4);
            },
        );
        let key = format!("insp-{param:?}");
        if let Some(ease) = pick_ease {
            for c in clips {
                let at = c.source_time_at(frame_of(c));
                self.st.apply_coalescing(
                    Edit::SetKeyframeEase {
                        clip: c.id,
                        param,
                        at,
                        ease,
                    },
                    &format!("{key}-ease"),
                );
            }
            self.st.editor.end_coalescing();
        }
        if toggle_key {
            for c in clips {
                let at = c.source_time_at(frame_of(c));
                let value = if here {
                    None
                } else {
                    Some(c.param(param, frame_of(c)))
                };
                self.st.apply_coalescing(
                    Edit::SetKeyframe {
                        clip: c.id,
                        param,
                        at,
                        value,
                    },
                    &format!("{key}-key"),
                );
            }
            self.st.editor.end_coalescing();
        }
        if do_reset {
            for c in clips {
                self.st.apply_coalescing(
                    Edit::ClearKeyframes { clip: c.id, param },
                    &format!("{key}-reset"),
                );
                self.st.apply_coalescing(
                    Edit::SetClip {
                        clip: c.id,
                        change: static_change(c, param, default_of(param)),
                    },
                    &format!("{key}-reset"),
                );
            }
            self.st.editor.end_coalescing();
        }
        if !changed {
            return;
        }
        let value = v / mul;
        for c in clips {
            let edit = if c.keyframes.get(&param).is_some_and(|k| !k.is_empty()) {
                Edit::SetKeyframe {
                    clip: c.id,
                    param,
                    at: c.source_time_at(frame_of(c)),
                    value: Some(value),
                }
            } else {
                Edit::SetClip {
                    clip: c.id,
                    change: static_change(c, param, value),
                }
            };
            self.st.apply_coalescing(edit, &key);
        }
    }

    /// Aplica `f(clip)` a cada clipe selecionado, num passo só de desfazer.
    fn apply_all(&mut self, clips: &[Clip], key: &str, f: impl Fn(&Clip) -> Option<Edit>) {
        for c in clips {
            if let Some(e) = f(c) {
                self.st.apply_coalescing(e, key);
            }
        }
    }

    pub fn inspector_ui(&mut self, ui: &mut egui::Ui) {
        let project = self.project_arc.clone();
        let selected: Vec<Clip> = self
            .st
            .selection
            .iter()
            .filter_map(|c| project.clip(*c))
            .cloned()
            .collect();
        let Some(first) = selected.first().cloned() else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Selecione um clipe na timeline").color(theme::TEXT_DIM))
            });
            return;
        };
        let Some((track_id, index)) = project.locate(first.id) else {
            return;
        };
        let kind = project.track(track_id).map_or(TrackKind::Video, |t| t.kind);
        // Só os clipes do mesmo tipo de faixa entram na edição em grupo.
        let clips: Vec<Clip> = selected
            .iter()
            .filter(|c| {
                project
                    .locate(c.id)
                    .and_then(|(t, _)| project.track(t))
                    .is_some_and(|t| t.kind == kind)
            })
            .cloned()
            .collect();
        let fps = project.settings.fps;
        theme::toolbar(ui, |ui| {
            ui.label(RichText::new(clip_title(&project, &first)).strong());
            if clips.len() > 1 {
                ui.label(
                    RichText::new(format!("+ {} clipes", clips.len() - 1)).color(theme::ACCENT),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(project.track_label(track_id).unwrap_or_default())
                        .color(theme::TEXT_DIM)
                        .small(),
                );
            });
        });
        // Áudio de um clipe de vídeo: os clipes de áudio ligados a ele (como no Resolve).
        let aclips: Vec<Clip> = if kind == TrackKind::Audio {
            clips.clone()
        } else {
            project
                .expand_linked(&clips.iter().map(|c| c.id).collect::<Vec<_>>())
                .into_iter()
                .filter(|c| {
                    project
                        .locate(*c)
                        .and_then(|(t, _)| project.track(t))
                        .is_some_and(|t| t.kind == TrackKind::Audio)
                })
                .filter_map(|c| project.clip(c).cloned())
                .collect()
        };
        let mut tabs = Vec::new();
        if matches!(first.source, ClipSource::Text(_)) && clips.len() == 1 {
            tabs.push(InspectorTab::Title);
        }
        if kind == TrackKind::Video {
            tabs.extend([InspectorTab::Video, InspectorTab::Color]);
        }
        if !aclips.is_empty() {
            tabs.push(InspectorTab::Audio);
        }
        tabs.push(InspectorTab::Time);
        let tab = if tabs.contains(&self.inspector_tab) {
            self.inspector_tab
        } else {
            tabs[0]
        };
        if let Some(t) = tab_bar(ui, &tabs, tab) {
            self.inspector_tab = t;
        }
        let id = first.id;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add_space(4.0);
                match &first.source {
                    ClipSource::Text(style) if clips.len() == 1 => {
                        if tab == InspectorTab::Title {
                            self.text_section(ui, id, style)
                        }
                    }
                    ClipSource::SolidColor(color) if tab == InspectorTab::Video => {
                        let mut c = *color;
                        section(ui, "Gerador", |ui| {
                            row(
                                ui,
                                "Cor",
                                |ui| {
                                    if color_button(ui, &mut c) {
                                        self.apply_all(&clips, "insp-solid", |cl| {
                                            matches!(cl.source, ClipSource::SolidColor(_)).then(
                                                || Edit::SetClip {
                                                    clip: cl.id,
                                                    change: ClipChange::Source(
                                                        ClipSource::SolidColor(c),
                                                    ),
                                                },
                                            )
                                        });
                                    }
                                },
                                |_| {},
                            );
                        });
                    }
                    _ => {}
                }
                if kind == TrackKind::Video && tab == InspectorTab::Video {
                    section(ui, "Transform", |ui| {
                        self.param_row(
                            ui,
                            &clips,
                            Param::X,
                            "Posição X",
                            1.0,
                            -10000.0..=10000.0,
                            "",
                        );
                        self.param_row(
                            ui,
                            &clips,
                            Param::Y,
                            "Posição Y",
                            1.0,
                            -10000.0..=10000.0,
                            "",
                        );
                        self.param_row(ui, &clips, Param::Scale, "Zoom", 100.0, 1.0..=1000.0, " %");
                        self.param_row(
                            ui,
                            &clips,
                            Param::Opacity,
                            "Opacidade",
                            100.0,
                            0.0..=100.0,
                            " %",
                        );
                        self.param_row(
                            ui,
                            &clips,
                            Param::Rotation,
                            "Rotação",
                            1.0,
                            -3600.0..=3600.0,
                            "°",
                        );
                        if fit_fill_scales(&project, &first).is_some() {
                            row(
                                ui,
                                "Enquadrar",
                                |ui| {
                                    let fit = ui
                                        .button("Ajustar")
                                        .on_hover_text("Imagem inteira no quadro, centralizada");
                                    let fill = ui
                                        .button("Preencher")
                                        .on_hover_text("Cobre o quadro todo (corta as sobras)");
                                    let pick = if fit.clicked() {
                                        Some(false)
                                    } else if fill.clicked() {
                                        Some(true)
                                    } else {
                                        None
                                    };
                                    if let Some(fill) = pick {
                                        let p = project.clone();
                                        self.apply_all(&clips, "insp-fit", |c| {
                                            let (fit, cover) = fit_fill_scales(&p, c)?;
                                            Some(Edit::SetClip {
                                                clip: c.id,
                                                change: ClipChange::Transform(Transform {
                                                    x: 0.0,
                                                    y: 0.0,
                                                    scale: if fill { cover } else { fit },
                                                    ..c.transform
                                                }),
                                            })
                                        });
                                        self.st.editor.end_coalescing();
                                    }
                                },
                                |_| {},
                            );
                        }
                        let mut t = first.transform;
                        row(
                            ui,
                            "Espelhar",
                            |ui| {
                                let h = ui.toggle_value(&mut t.flip_h, "↔ Horiz.").changed();
                                let v = ui.toggle_value(&mut t.flip_v, "↕ Vert.").changed();
                                if h || v {
                                    self.apply_all(&clips, "insp-flip", |c| {
                                        Some(Edit::SetClip {
                                            clip: c.id,
                                            change: ClipChange::Transform(Transform {
                                                flip_h: t.flip_h,
                                                flip_v: t.flip_v,
                                                ..c.transform
                                            }),
                                        })
                                    });
                                    self.st.editor.end_coalescing();
                                }
                            },
                            |_| {},
                        );
                    });
                    section(ui, "Recorte", |ui| {
                        let k = first.transform.crop;
                        type Side = (&'static str, f32, fn(&mut Crop) -> &mut f32);
                        let sides: [Side; 4] = [
                            ("Esquerda", k.left, |c| &mut c.left),
                            ("Direita", k.right, |c| &mut c.right),
                            ("Topo", k.top, |c| &mut c.top),
                            ("Base", k.bottom, |c| &mut c.bottom),
                        ];
                        for (label, value, field) in sides {
                            let mut pct = value * 100.0;
                            let mut changed = false;
                            let mut clear = false;
                            row(
                                ui,
                                label,
                                |ui| {
                                    changed = ui
                                        .add(
                                            DragValue::new(&mut pct)
                                                .range(0.0..=90.0)
                                                .speed(0.3)
                                                .suffix(" %"),
                                        )
                                        .changed()
                                },
                                |ui| {
                                    ui.add_space(22.0);
                                    clear = reset(ui, value > 0.0);
                                },
                            );
                            if changed || clear {
                                let v = if clear { 0.0 } else { pct / 100.0 };
                                self.apply_all(&clips, &format!("insp-crop-{label}"), |c| {
                                    let mut crop = c.transform.crop;
                                    *field(&mut crop) = v;
                                    (crop.left + crop.right < 0.95 && crop.top + crop.bottom < 0.95)
                                        .then_some(Edit::SetClip {
                                            clip: c.id,
                                            change: ClipChange::Transform(Transform {
                                                crop,
                                                ..c.transform
                                            }),
                                        })
                                });
                            }
                        }
                    });
                }
                if kind == TrackKind::Video && tab == InspectorTab::Color {
                    section(ui, "Cor  (Matiz / Saturação / Luminosidade)", |ui| {
                        self.param_row(ui, &clips, Param::Hue, "Matiz", 1.0, -180.0..=180.0, "°");
                        self.param_row(
                            ui,
                            &clips,
                            Param::Saturation,
                            "Saturação",
                            1.0,
                            -100.0..=100.0,
                            "",
                        );
                        self.param_row(
                            ui,
                            &clips,
                            Param::Lightness,
                            "Luminosidade",
                            1.0,
                            -100.0..=100.0,
                            "",
                        );
                        if first.color != ColorAdjust::default()
                            && ui.small_button("Resetar cor").clicked()
                        {
                            self.apply_all(&clips, "insp-color-reset", |c| {
                                Some(Edit::SetClip {
                                    clip: c.id,
                                    change: ClipChange::Color(ColorAdjust::default()),
                                })
                            });
                        }
                    });
                }
                if tab == InspectorTab::Time {
                    section(ui, "Tempo", |ui| {
                        let still = match &first.source {
                            ClipSource::Media(m) => project
                                .media(*m)
                                .is_some_and(|m| m.info.kind == MediaKind::Image),
                            _ => true,
                        };
                        // Duração exata, digitada em timecode (aplica um trim na borda final).
                        let dur_id = ui.id().with(("dur", id));
                        let mut text = ui
                            .data_mut(|d| d.get_temp::<String>(dur_id))
                            .unwrap_or_else(|| format_timecode(first.duration, fps));
                        let mut commit = false;
                        row(
                            ui,
                            "Duração",
                            |ui| {
                                let resp = ui.add(
                                    egui::TextEdit::singleline(&mut text)
                                        .desired_width(theme::VALUE_W + 20.0)
                                        .font(egui::TextStyle::Monospace),
                                );
                                if resp.changed() {
                                    ui.data_mut(|d| d.insert_temp(dur_id, text.clone()));
                                }
                                if resp.lost_focus() {
                                    commit = ui.input(|i| i.key_pressed(egui::Key::Enter));
                                    ui.data_mut(|d| d.remove::<String>(dur_id));
                                }
                            },
                            |_| {},
                        );
                        if commit && let Some(d) = parse_timecode(&text, fps, first.duration) {
                            let delta = d.max(1) - first.duration;
                            self.st.apply(Edit::Trim {
                                clips: self.st.expanded(&[id]),
                                edge: Edge::End,
                                delta,
                            });
                        }
                        if !still && kind == TrackKind::Video {
                            let frozen = first.freeze.is_some();
                            let mut toggle = false;
                            row(
                                ui,
                                "Congelar",
                                |ui| {
                                    toggle = ui
                                        .selectable_label(
                                            frozen,
                                            if frozen {
                                                "Congelado (clique para soltar)"
                                            } else {
                                                "Congelar no cursor"
                                            },
                                        )
                                        .on_hover_text(
                                            "Freeze frame: o clipe inteiro mostra o frame do cursor",
                                        )
                                        .clicked();
                                },
                                |_| {},
                            );
                            if toggle {
                                self.st.freeze_frame();
                            }
                        }
                        if !still {
                            let mut pct = first.speed * 100.0;
                            let (mut set_speed, mut reset_speed) = (None, false);
                            row(
                                ui,
                                "Velocidade",
                                |ui| {
                                    if ui
                                        .add(
                                            DragValue::new(&mut pct)
                                                .range(10.0..=1000.0)
                                                .speed(1.0)
                                                .suffix(" %"),
                                        )
                                        .changed()
                                    {
                                        set_speed = Some(pct / 100.0);
                                    }
                                },
                                |ui| {
                                    ui.add_space(22.0);
                                    reset_speed = reset(ui, first.speed != 1.0);
                                },
                            );
                            let speed = if reset_speed { Some(1.0) } else { set_speed };
                            if let Some(speed) = speed {
                                let all: Vec<_> = clips
                                    .iter()
                                    .flat_map(|c| self.st.expanded(&[c.id]))
                                    .collect();
                                self.st.apply_coalescing(
                                    Edit::SetSpeed { clips: all, speed },
                                    "insp-speed",
                                );
                            }
                        }
                        for (label, value, is_in) in [
                            ("Fade in", first.fade_in, true),
                            ("Fade out", first.fade_out, false),
                        ] {
                            let mut f = value;
                            row(
                                ui,
                                label,
                                |ui| {
                                    if ui
                                        .add(
                                            DragValue::new(&mut f)
                                                .range(0..=first.duration)
                                                .suffix(" fr"),
                                        )
                                        .changed()
                                    {
                                        self.apply_all(&clips, &format!("insp-{label}"), |c| {
                                            Some(Edit::SetClip {
                                                clip: c.id,
                                                change: if is_in {
                                                    ClipChange::FadeIn(f)
                                                } else {
                                                    ClipChange::FadeOut(f)
                                                },
                                            })
                                        });
                                    }
                                },
                                |_| {},
                            );
                        }
                        let joined = index > 0
                            && project
                                .track(track_id)
                                .is_some_and(|t| t.clips[index - 1].end() == first.start);
                        if joined {
                            let mut d = first.dissolve_in;
                            row(
                                ui,
                                "Dissolve",
                                |ui| {
                                    if ui
                                        .add(
                                            DragValue::new(&mut d)
                                                .range(0..=first.duration)
                                                .suffix(" fr"),
                                        )
                                        .on_hover_text(
                                            "Cross dissolve com o clipe anterior (Ctrl+T)",
                                        )
                                        .changed()
                                    {
                                        self.st.apply_coalescing(
                                            Edit::SetDissolve {
                                                clip: id,
                                                frames: d,
                                            },
                                            "insp-dissolve",
                                        );
                                    }
                                },
                                |_| {},
                            );
                        }
                    });
                }
                if tab == InspectorTab::Audio && !aclips.is_empty() {
                    section(ui, "Áudio", |ui| {
                        self.param_row(
                            ui,
                            &aclips,
                            Param::VolumeDb,
                            "Volume",
                            1.0,
                            -60.0..=24.0,
                            " dB",
                        );
                        row(
                            ui,
                            "",
                            |ui| {
                                if ui.button("Normalizar (−1 dB)").clicked() {
                                    self.st.normalize_selection();
                                }
                            },
                            |_| {},
                        );
                    });
                    section(ui, "Remover silêncios", |ui| {
                        row(
                            ui,
                            "Limiar",
                            |ui| {
                                ui.add(
                                    DragValue::new(&mut self.silence_threshold)
                                        .range(-70.0..=-10.0)
                                        .suffix(" dB"),
                                );
                            },
                            |_| {},
                        );
                        row(
                            ui,
                            "Mínimo",
                            |ui| {
                                ui.add(
                                    DragValue::new(&mut self.silence_min)
                                        .range(0.1..=5.0)
                                        .speed(0.02)
                                        .suffix(" s"),
                                );
                            },
                            |_| {},
                        );
                        row(
                            ui,
                            "",
                            |ui| {
                                if ui
                                    .button("Remover silêncios")
                                    .on_hover_text(
                                        "Corta as pausas e fecha os buracos (vídeo ligado junto)",
                                    )
                                    .clicked()
                                {
                                    let (t, m) = (self.silence_threshold, self.silence_min);
                                    self.st.remove_silences(t, m);
                                }
                            },
                            |_| {},
                        );
                    });
                }
                if first.link.is_some() {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(theme::LABEL_W + theme::PAD);
                        if ui.button("Separar áudio e vídeo").clicked() {
                            self.st.apply(Edit::Unlink {
                                clips: project.expand_linked(&[id]),
                            });
                        }
                    });
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
            ui.horizontal(|ui| {
                ui.add_space(theme::PAD);
                changed |= ui
                    .add(
                        egui::TextEdit::multiline(&mut s.text)
                            .desired_rows(2)
                            .desired_width(ui.available_width() - theme::PAD),
                    )
                    .changed();
            });
            ui.add_space(4.0);
            row(
                ui,
                "Fonte",
                |ui| {
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut s.font).desired_width(140.0))
                        .changed()
                },
                |_| {},
            );
            row(
                ui,
                "Tamanho",
                |ui| {
                    changed |= ui
                        .add(DragValue::new(&mut s.size).range(4.0..=600.0).speed(0.5))
                        .changed()
                },
                |_| {},
            );
            row(
                ui,
                "Cor",
                |ui| changed |= color_button(ui, &mut s.color),
                |_| {},
            );
            row(
                ui,
                "Estilo",
                |ui| {
                    changed |= ui
                        .toggle_value(&mut s.bold, RichText::new(" N ").strong())
                        .on_hover_text("Negrito")
                        .changed();
                    for (a, label, tip) in [
                        (TextAlign::Left, "⬅", "Alinhar à esquerda"),
                        (TextAlign::Center, "↔", "Centralizar"),
                        (TextAlign::Right, "➡", "Alinhar à direita"),
                    ] {
                        changed |= ui
                            .selectable_value(&mut s.align, a, label)
                            .on_hover_text(tip)
                            .changed();
                    }
                },
                |_| {},
            );
            row(
                ui,
                "Contorno",
                |ui| {
                    changed |= ui
                        .add(
                            DragValue::new(&mut s.outline)
                                .range(0.0..=30.0)
                                .speed(0.2)
                                .suffix(" px"),
                        )
                        .changed();
                    changed |= color_button(ui, &mut s.outline_color);
                },
                |_| {},
            );
            row(
                ui,
                "Sombra",
                |ui| changed |= ui.checkbox(&mut s.shadow, "").changed(),
                |_| {},
            );
            row(
                ui,
                "Caixa",
                |ui| {
                    changed |= ui.checkbox(&mut s.background, "").changed();
                    changed |= color_button(ui, &mut s.background_color);
                },
                |_| {},
            );
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
