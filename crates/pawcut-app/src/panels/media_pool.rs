//! Media Pool (grade de miniaturas) e a Toolbox de geradores (aba Effects).

use egui::{Color32, Rect, Sense, Stroke, Vec2, pos2, vec2};
use pawcut_core::{MediaKind, format_timecode};

use super::DragPayload;
use crate::icons::{self, Icon};
use crate::state::Generator;
use crate::theme;
use crate::workspace::Workspace;

const THUMB: Vec2 = Vec2::new(124.0, 70.0);
const TILE: Vec2 = Vec2::new(132.0, 100.0);

impl Workspace {
    pub fn media_pool_ui(&mut self, ui: &mut egui::Ui) {
        let project = self.project_arc.clone();
        theme::header_strip(ui, 30.0, |ui| {
            if ui
                .add(egui::Button::new("Importar mídia…"))
                .on_hover_text("Ctrl+I — ou arraste arquivos para a janela")
                .clicked()
            {
                self.import_dialog();
            }
            ui.label(
                egui::RichText::new(format!("{} itens", project.media.len()))
                    .color(theme::TEXT_DIM)
                    .small(),
            );
        });
        let area = ui.available_rect_before_wrap();
        ui.painter().rect_filled(area, 0.0, theme::BG);
        if project.media.is_empty() {
            let c = area.center();
            icons::paint(
                ui.painter(),
                Rect::from_center_size(c - vec2(0.0, 22.0), vec2(34.0, 34.0)),
                Icon::Import,
                theme::TEXT_DIM,
            );
            ui.painter().text(
                c + vec2(0.0, 12.0),
                egui::Align2::CENTER_CENTER,
                "Arraste vídeos, músicas e imagens para cá",
                theme::text(13.0),
                theme::TEXT_DIM,
            );
            ui.painter().text(
                c + vec2(0.0, 30.0),
                egui::Align2::CENTER_CENTER,
                "ou use Ctrl+I",
                theme::text(11.0),
                theme::TEXT_DIM,
            );
            ui.advance_cursor_after_rect(area);
            return;
        }
        let mut open_source = None;
        let mut relink = None;
        let mut remove = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    ui.add_space(4.0);
                    for m in &project.media {
                        let offline = self.st.offline.contains(&m.id);
                        let selected = self.st.source.media == Some(m.id);
                        let (rect, resp) = ui.allocate_exact_size(TILE, Sense::click_and_drag());
                        let painter = ui.painter();
                        if selected {
                            painter.rect_filled(rect, 4.0, theme::ACCENT.gamma_multiply(0.18));
                        } else if resp.hovered() {
                            painter.rect_filled(rect, 4.0, theme::BAR_LIGHT);
                        }
                        let thumb = Rect::from_min_size(
                            rect.min + vec2((TILE.x - THUMB.x) / 2.0, 4.0),
                            THUMB,
                        );
                        painter.rect_filled(thumb, 2.0, Color32::BLACK);
                        let tex = self.thumbnail(m.id, &m.info.path, m.info.kind);
                        let painter = ui.painter();
                        match tex {
                            Some(tex) => {
                                let size = tex.size_vec2();
                                let scale = (THUMB.x / size.x).min(THUMB.y / size.y);
                                let r = Rect::from_center_size(thumb.center(), size * scale);
                                painter.image(
                                    tex.id(),
                                    r,
                                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            }
                            None => {
                                let (icon, fill) = match m.info.kind {
                                    MediaKind::Audio => (Icon::Speaker, theme::CLIP_AUDIO_LABEL),
                                    _ => (Icon::MediaPool, theme::CLIP_VIDEO_LABEL),
                                };
                                painter.rect_filled(
                                    thumb,
                                    2.0,
                                    if offline { theme::OFFLINE } else { fill },
                                );
                                icons::paint(
                                    painter,
                                    Rect::from_center_size(thumb.center(), vec2(26.0, 26.0)),
                                    icon,
                                    Color32::from_white_alpha(200),
                                );
                            }
                        }
                        if offline {
                            painter.rect_filled(thumb, 2.0, theme::OFFLINE.gamma_multiply(0.75));
                            painter.text(
                                thumb.center(),
                                egui::Align2::CENTER_CENTER,
                                "MEDIA OFFLINE",
                                theme::text(11.0),
                                Color32::WHITE,
                            );
                        }
                        if selected {
                            painter.rect_stroke(
                                thumb,
                                2.0,
                                Stroke::new(2.0, theme::ACCENT),
                                egui::StrokeKind::Outside,
                            );
                        }
                        let fps = project.settings.fps;
                        if let (Some(d), false) = (m.info.duration, m.info.kind == MediaKind::Image)
                        {
                            let tc = format_timecode(d, fps);
                            let galley =
                                painter.layout_no_wrap(tc, theme::mono(10.0), Color32::WHITE);
                            let at = thumb.right_bottom() - galley.size() - vec2(4.0, 3.0);
                            painter.rect_filled(
                                Rect::from_min_size(
                                    at - vec2(2.0, 1.0),
                                    galley.size() + vec2(4.0, 2.0),
                                ),
                                2.0,
                                Color32::from_black_alpha(170),
                            );
                            painter.galley(at, galley, Color32::WHITE);
                        }
                        let name = m
                            .info
                            .path
                            .file_name()
                            .map_or("?".into(), |n| n.to_string_lossy().into_owned());
                        let text_rect = Rect::from_min_max(
                            pos2(rect.left() + 4.0, thumb.bottom() + 4.0),
                            pos2(rect.right() - 4.0, rect.bottom()),
                        );
                        let galley =
                            painter.layout(name, theme::text(11.5), theme::TEXT, text_rect.width());
                        painter.with_clip_rect(text_rect).galley(
                            text_rect.min,
                            galley,
                            theme::TEXT,
                        );

                        resp.dnd_set_drag_payload(DragPayload::Media(m.id));
                        if resp.clicked() || resp.double_clicked() {
                            open_source = Some(m.id);
                        }
                        let id = m.id;
                        resp.context_menu(|ui| {
                            if ui.button("Abrir no Source Viewer").clicked() {
                                open_source = Some(id);
                                ui.close();
                            }
                            if ui.button("Reconectar arquivo (Relink)…").clicked() {
                                relink = Some(id);
                                ui.close();
                            }
                            if ui.button("Remover do Media Pool").clicked() {
                                remove = Some(id);
                                ui.close();
                            }
                        });
                        let detail = match m.info.kind {
                            MediaKind::Image => {
                                format!("Imagem {}×{}", m.info.width, m.info.height)
                            }
                            MediaKind::Audio => "Áudio".to_string(),
                            MediaKind::Video { has_audio } => {
                                format!(
                                    "{}×{}{}",
                                    m.info.width,
                                    m.info.height,
                                    if has_audio { " · com áudio" } else { "" }
                                )
                            }
                        };
                        resp.on_hover_text(format!("{}\n{detail}", m.info.path.display()));
                    }
                });
            });
        if let Some(id) = open_source {
            self.st.open_in_source(id);
        }
        if let Some(id) = relink
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Onde está esse arquivo agora?")
                .pick_file()
        {
            self.forget_media(id);
            self.st.relink(id, &path);
            self.forget_media(id);
        }
        if let Some(id) = remove {
            let removed = self.st.apply(pawcut_core::Edit::RemoveMedia(id)).is_some();
            if removed && self.st.source.media == Some(id) {
                self.st.source = Default::default();
            }
        }
    }

    pub fn effects_ui(&mut self, ui: &mut egui::Ui) {
        theme::header_strip(ui, 30.0, |ui| {
            ui.label(egui::RichText::new("Toolbox").strong());
        });
        ui.add_space(6.0);
        ui.indent("toolbox", |ui| {
            ui.label(
                egui::RichText::new("Títulos")
                    .color(theme::TEXT_DIM)
                    .small(),
            );
            self.toolbox_item(ui, "Texto", theme::CLIP_TITLE, Generator::Text);
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Geradores")
                    .color(theme::TEXT_DIM)
                    .small(),
            );
            self.toolbox_item(
                ui,
                "Cor sólida",
                theme::CLIP_GENERATOR,
                Generator::SolidColor,
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Arraste para uma faixa de vídeo, ou dê dois cliques.")
                    .color(theme::TEXT_DIM)
                    .small(),
            );
        });
    }

    fn toolbox_item(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        color: Color32,
        generator: Generator,
    ) {
        let (rect, resp) = ui.allocate_exact_size(
            vec2(ui.available_width() - 8.0, 26.0),
            Sense::click_and_drag(),
        );
        let fill = if resp.hovered() {
            theme::BAR_LIGHT
        } else {
            theme::BAR
        };
        ui.painter().rect_filled(rect, 3.0, fill);
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(6.0, 7.0), vec2(18.0, 12.0)),
            2.0,
            color,
        );
        ui.painter().text(
            rect.left_center() + vec2(32.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            theme::text(12.5),
            theme::TEXT,
        );
        resp.dnd_set_drag_payload(DragPayload::Generator(generator));
        if resp.double_clicked() {
            let track = self.project_arc.video_tracks.len();
            self.st.drop_generator(generator, track, self.st.playhead);
        }
    }
}
