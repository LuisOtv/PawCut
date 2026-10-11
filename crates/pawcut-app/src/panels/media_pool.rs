//! Media Pool e a Toolbox de geradores (aba Effects).

use egui::{Color32, RichText, Sense, Vec2};
use pawcut_core::{MediaKind, format_timecode};

use super::DragPayload;
use crate::state::Generator;
use crate::workspace::Workspace;

const THUMB: Vec2 = Vec2::new(80.0, 45.0);

impl Workspace {
    pub fn media_pool_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("📥 Importar mídia…").clicked() {
                self.import_dialog();
            }
            ui.label(
                RichText::new("ou arraste arquivos para a janela")
                    .weak()
                    .small(),
            );
        });
        ui.separator();
        let project = self.project_arc.clone();
        if project.media.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Nenhuma mídia importada").weak())
            });
            return;
        }
        let mut open_source = None;
        let mut relink = None;
        let mut remove = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for m in &project.media {
                    let offline = self.st.offline.contains(&m.id);
                    let selected = self.st.source.media == Some(m.id);
                    let (rect, resp) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), THUMB.y + 8.0),
                        Sense::click_and_drag(),
                    );
                    if selected || resp.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            3.0,
                            ui.visuals().widgets.hovered.weak_bg_fill,
                        );
                    }
                    let thumb_rect =
                        egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 4.0), THUMB);
                    match self.thumbnail(m.id, &m.info.path, m.info.kind) {
                        Some(tex) => {
                            let size = tex.size_vec2();
                            let scale = (THUMB.x / size.x).min(THUMB.y / size.y);
                            let r = egui::Rect::from_center_size(thumb_rect.center(), size * scale);
                            ui.painter().rect_filled(thumb_rect, 2.0, Color32::BLACK);
                            ui.painter().image(
                                tex.id(),
                                r,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                Color32::WHITE,
                            );
                        }
                        None => {
                            let (icon, color) = match m.info.kind {
                                MediaKind::Audio => ("🎵", Color32::from_rgb(60, 120, 80)),
                                _ => ("🎞", Color32::from_rgb(60, 70, 100)),
                            };
                            ui.painter().rect_filled(
                                thumb_rect,
                                2.0,
                                if offline {
                                    Color32::from_rgb(120, 20, 30)
                                } else {
                                    color
                                },
                            );
                            ui.painter().text(
                                thumb_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                icon,
                                egui::FontId::proportional(20.0),
                                Color32::WHITE,
                            );
                        }
                    }
                    let name = m
                        .info
                        .path
                        .file_name()
                        .map_or("?".into(), |n| n.to_string_lossy().into_owned());
                    let fps = project.settings.fps;
                    let detail = match (m.info.kind, m.info.duration) {
                        (MediaKind::Image, _) => {
                            format!("Imagem {}×{}", m.info.width, m.info.height)
                        }
                        (MediaKind::Audio, Some(d)) => {
                            format!("Áudio · {}", format_timecode(d, fps))
                        }
                        (MediaKind::Video { .. }, Some(d)) => {
                            format!(
                                "{}×{} · {}",
                                m.info.width,
                                m.info.height,
                                format_timecode(d, fps)
                            )
                        }
                        _ => String::new(),
                    };
                    let text_pos = egui::pos2(thumb_rect.right() + 8.0, rect.top() + 8.0);
                    let color = if offline {
                        Color32::from_rgb(230, 80, 80)
                    } else {
                        ui.visuals().text_color()
                    };
                    ui.painter().text(
                        text_pos,
                        egui::Align2::LEFT_TOP,
                        &name,
                        egui::FontId::proportional(13.0),
                        color,
                    );
                    let detail = if offline {
                        "MEDIA OFFLINE".to_string()
                    } else {
                        detail
                    };
                    ui.painter().text(
                        text_pos + egui::vec2(0.0, 18.0),
                        egui::Align2::LEFT_TOP,
                        detail,
                        egui::FontId::proportional(11.0),
                        ui.visuals().weak_text_color(),
                    );
                    resp.dnd_set_drag_payload(DragPayload::Media(m.id));
                    if resp.double_clicked() || (resp.clicked() && m.info.kind != MediaKind::Audio)
                    {
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
                    resp.on_hover_text(m.info.path.display().to_string());
                }
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
        ui.label(RichText::new("Toolbox").strong());
        ui.add_space(4.0);
        ui.label(RichText::new("Títulos").weak());
        self.toolbox_item(ui, "T   Texto", Generator::Text);
        ui.add_space(6.0);
        ui.label(RichText::new("Geradores").weak());
        self.toolbox_item(ui, "■   Cor sólida", Generator::SolidColor);
        ui.add_space(10.0);
        ui.label(RichText::new("Arraste para a timeline.").weak().small());
    }

    fn toolbox_item(&mut self, ui: &mut egui::Ui, label: &str, generator: Generator) {
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), 26.0),
            Sense::click_and_drag(),
        );
        let fill = if resp.hovered() {
            ui.visuals().widgets.hovered.bg_fill
        } else {
            ui.visuals().widgets.inactive.bg_fill
        };
        ui.painter().rect_filled(rect, 3.0, fill);
        ui.painter().text(
            rect.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            ui.visuals().text_color(),
        );
        resp.dnd_set_drag_payload(DragPayload::Generator(generator));
        if resp.double_clicked() {
            let track = self.project_arc.video_tracks.len();
            self.st.drop_generator(generator, track, self.st.playhead);
        }
    }
}
