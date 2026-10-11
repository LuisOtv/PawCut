//! Janelas: Deliver (exportar), Preferences (atalhos), novo projeto, confirmações e recuperação.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use egui::{RichText, Window};
use pawcut_core::{FrameRate, ProjectSettings, format_timecode};
use pawcut_export::{AudioFormat, ExportSettings, VideoCodec};

use crate::config;
use crate::engine::ExportJob;
use crate::keymap::{Action, Keymap};
use crate::workspace::{Pending, Workspace};

pub struct DeliverDialog {
    pub output: String,
    pub crf: u8,
    pub preset: usize,
    pub only_in_out: bool,
    pub job: Option<ExportJob>,
    pub message: Option<Result<String, String>>,
}

/// Presets de exportação: nome, altura de saída (`None` = do projeto), codec, só áudio, extensão.
/// Nome, altura de saída, codec, só áudio, extensão.
type DeliverPreset = (
    &'static str,
    Option<u32>,
    VideoCodec,
    Option<AudioFormat>,
    &'static str,
);

pub const DELIVER_PRESETS: [DeliverPreset; 6] = [
    (
        "Resolução do projeto (H.264)",
        None,
        VideoCodec::H264,
        None,
        "mp4",
    ),
    (
        "YouTube 1080p (H.264)",
        Some(1080),
        VideoCodec::H264,
        None,
        "mp4",
    ),
    (
        "YouTube 4K (H.265)",
        Some(2160),
        VideoCodec::H265,
        None,
        "mp4",
    ),
    (
        "Rápido 720p (H.264)",
        Some(720),
        VideoCodec::H264,
        None,
        "mp4",
    ),
    (
        "Só áudio — MP3",
        None,
        VideoCodec::H264,
        Some(AudioFormat::Mp3),
        "mp3",
    ),
    (
        "Só áudio — WAV",
        None,
        VideoCodec::H264,
        Some(AudioFormat::Wav),
        "wav",
    ),
];

pub struct NewProjectDialog {
    pub preset: usize,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

pub const PRESETS: [(&str, u32, u32, u32); 6] = [
    ("1080p · 30 fps", 1920, 1080, 30),
    ("1080p · 60 fps", 1920, 1080, 60),
    ("720p · 30 fps", 1280, 720, 30),
    ("4K UHD · 30 fps", 3840, 2160, 30),
    ("Vertical 1080×1920 · 30 fps (Reels/Shorts)", 1080, 1920, 30),
    ("Quadrado 1080×1080 · 30 fps", 1080, 1080, 30),
];

impl Default for NewProjectDialog {
    fn default() -> Self {
        Self {
            preset: 0,
            width: 1920,
            height: 1080,
            fps: 30,
        }
    }
}

impl Workspace {
    pub fn open_deliver(&mut self) {
        if self.deliver.is_none() {
            let stem = self
                .st
                .path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map_or("video".into(), |s| s.to_string_lossy().into_owned());
            let dir = self
                .st
                .path
                .as_ref()
                .and_then(|p| p.parent().map(PathBuf::from))
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Vídeos")))
                .unwrap_or_default();
            let dir = if dir.exists() {
                dir
            } else {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default()
            };
            self.deliver = Some(DeliverDialog {
                output: dir.join(format!("{stem}.mp4")).display().to_string(),
                crf: 18,
                preset: 0,
                only_in_out: false,
                job: None,
                message: None,
            });
        }
    }

    pub fn dialogs_ui(&mut self, ctx: &egui::Context) {
        self.deliver_ui(ctx);
        self.prefs_ui(ctx);
        self.new_project_ui(ctx);
        self.confirm_ui(ctx);
        self.recovery_ui(ctx);
        self.export_quit_ui(ctx);
        self.marker_ui(ctx);
        self.project_settings_ui(ctx);
    }

    fn project_settings_ui(&mut self, ctx: &egui::Context) {
        let Some((mut w, mut h)) = self.project_settings else {
            return;
        };
        let fps = self.project_arc.settings.fps;
        let mut open = true;
        let mut apply = false;
        Window::new("Configurações do projeto")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Resolução da timeline");
                ui.horizontal_wrapped(|ui| {
                    for (name, pw, ph) in [
                        ("1080p", 1920, 1080),
                        ("4K", 3840, 2160),
                        ("720p", 1280, 720),
                        ("Vertical 9:16", 1080, 1920),
                        ("Quadrado", 1080, 1080),
                        ("4:5 (feed)", 1080, 1350),
                    ] {
                        if ui.selectable_label((w, h) == (pw, ph), name).clicked() {
                            (w, h) = (pw, ph);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut w).range(16..=8192));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut h).range(16..=8192));
                });
                ui.label(
                    RichText::new(format!("FPS: {} (fixo depois de criado)", fps.as_f64()))
                        .weak()
                        .small(),
                );
                ui.add_space(6.0);
                if ui.button("Aplicar").clicked() {
                    apply = true;
                }
            });
        self.project_settings = Some((w, h));
        if apply {
            self.st.apply(pawcut_core::Edit::SetResolution {
                width: w,
                height: h,
            });
            self.project_settings = None;
            self.requested_timeline = None;
        }
        if !open {
            self.project_settings = None;
        }
    }

    fn marker_ui(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.editing_marker else {
            return;
        };
        let Some(mut marker) = self
            .project_arc
            .markers
            .iter()
            .find(|m| m.frame == frame)
            .cloned()
        else {
            self.editing_marker = None;
            return;
        };
        let fps = self.project_arc.settings.fps;
        let mut open = true;
        let mut changed = false;
        let mut delete = false;
        Window::new(format!("Marcador {}", format_timecode(frame, fps)))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Nota");
                changed |= ui
                    .add(
                        egui::TextEdit::multiline(&mut marker.note)
                            .desired_rows(3)
                            .desired_width(260.0),
                    )
                    .changed();
                ui.horizontal(|ui| {
                    for (r, g, b) in [
                        (70, 140, 230),
                        (90, 200, 120),
                        (240, 200, 70),
                        (240, 120, 50),
                        (220, 70, 70),
                        (180, 110, 220),
                    ] {
                        let c = pawcut_core::Rgba::new(r, g, b, 255);
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                        ui.painter()
                            .rect_filled(rect, 3.0, egui::Color32::from_rgb(r, g, b));
                        if marker.color == c {
                            ui.painter().rect_stroke(
                                rect,
                                3.0,
                                egui::Stroke::new(2.0, egui::Color32::WHITE),
                                egui::StrokeKind::Outside,
                            );
                        }
                        if resp.clicked() {
                            marker.color = c;
                            changed = true;
                        }
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Apagar marcador").clicked() {
                        delete = true;
                    }
                });
            });
        if delete {
            self.st.apply(pawcut_core::Edit::SetMarker {
                frame,
                marker: None,
            });
            self.editing_marker = None;
        } else if changed {
            self.st.apply_coalescing(
                pawcut_core::Edit::SetMarker {
                    frame,
                    marker: Some(marker),
                },
                &format!("marker-{frame}"),
            );
        }
        if !open {
            self.editing_marker = None;
        }
    }

    fn deliver_ui(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.deliver else { return };
        let mut open = true;
        let project = self.project_arc.clone();
        let s = &project.settings;
        let marks = match (self.st.mark_in, self.st.mark_out) {
            (Some(a), Some(b)) if b > a => Some((a, b)),
            _ => None,
        };
        let mut start = false;
        let mut pick_output = false;
        let mut still = false;
        Window::new("Deliver — Exportar")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                egui::Grid::new("deliver-grid")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Formato");
                        let before = d.preset;
                        egui::ComboBox::from_id_salt("deliver-preset")
                            .width(260.0)
                            .selected_text(DELIVER_PRESETS[d.preset].0)
                            .show_ui(ui, |ui| {
                                for (i, p) in DELIVER_PRESETS.iter().enumerate() {
                                    ui.selectable_value(&mut d.preset, i, p.0);
                                }
                            });
                        if d.preset != before {
                            // Troca a extensão do arquivo junto com o formato.
                            let mut out = PathBuf::from(d.output.trim());
                            out.set_extension(DELIVER_PRESETS[d.preset].4);
                            d.output = out.display().to_string();
                        }
                        ui.end_row();
                        ui.label("Trecho");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut d.only_in_out, false, "Timeline inteira");
                            ui.add_enabled_ui(marks.is_some(), |ui| {
                                ui.radio_value(&mut d.only_in_out, true, "Só In/Out")
                                    .on_disabled_hover_text("Marque In (I) e Out (O) na timeline");
                            });
                        });
                        ui.end_row();
                        ui.label("Arquivo");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut d.output).desired_width(300.0));
                            if ui.button("…").clicked() {
                                pick_output = true;
                            }
                        });
                        ui.end_row();
                        if DELIVER_PRESETS[d.preset].3.is_none() {
                            ui.label("Qualidade");
                            ui.add(
                                egui::Slider::new(&mut d.crf, 14..=32).text("CRF (menor = melhor)"),
                            );
                            ui.end_row();
                        }
                    });
                let (from, to) = if d.only_in_out {
                    marks.unwrap_or((0, project.duration()))
                } else {
                    (0, project.duration())
                };
                let (w, h) = deliver_size(s.width, s.height, DELIVER_PRESETS[d.preset].1);
                ui.label(
                    RichText::new(format!(
                        "{w}×{h} · {} fps · duração {}",
                        s.fps.as_f64(),
                        format_timecode(to - from, s.fps)
                    ))
                    .weak(),
                );
                ui.add_space(8.0);
                match &d.job {
                    Some(job) => {
                        let done = job.done.load(Ordering::Relaxed) as f32;
                        let frac = if job.total > 0 {
                            done / job.total as f32
                        } else {
                            0.0
                        };
                        ui.add(
                            egui::ProgressBar::new(frac)
                                .show_percentage()
                                .text(format!("{} / {} frames", done as i64, job.total)),
                        );
                        if ui.button("Cancelar").clicked() {
                            job.cancel.store(true, Ordering::Relaxed);
                        }
                        if let Some(result) = job.finished() {
                            d.message = Some(result.map(|_| format!("Pronto: {}", d.output)));
                            d.job = None;
                        }
                    }
                    None => {
                        ui.horizontal(|ui| {
                            let ok = to > from && !d.output.trim().is_empty();
                            if ui
                                .add_enabled(
                                    ok,
                                    egui::Button::new(RichText::new("Exportar").strong()),
                                )
                                .clicked()
                            {
                                start = true;
                            }
                            if ui.button("Salvar frame atual como PNG").clicked() {
                                still = true;
                            }
                        });
                        if project.duration() == 0 {
                            ui.label(RichText::new("A timeline está vazia.").weak());
                        }
                    }
                }
                match &d.message {
                    Some(Ok(m)) => {
                        ui.colored_label(egui::Color32::from_rgb(120, 220, 120), m);
                        if ui.button("Abrir pasta").clicked()
                            && let Some(dir) = PathBuf::from(&d.output).parent()
                        {
                            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
                        }
                    }
                    Some(Err(m)) => {
                        ui.colored_label(egui::Color32::from_rgb(240, 110, 110), m);
                    }
                    None => {}
                }
            });
        if pick_output {
            let ext = DELIVER_PRESETS[d.preset].4;
            self.ask_file(crate::file_dialog::Purpose::ExportTo, move || {
                rfd::FileDialog::new()
                    .add_filter(ext.to_uppercase(), &[ext])
                    .set_file_name(format!("video.{ext}"))
                    .save_file()
                    .into_iter()
                    .collect()
            });
            return;
        }
        if still {
            let mut out = PathBuf::from(d.output.trim());
            out.set_extension("png");
            let frame = self.st.playhead.min((project.duration() - 1).max(0));
            let s = &project.settings;
            let mut renderer = pawcut_render::Renderer::new(pawcut_media::MediaLibrary::new(
                (s.width, s.height),
                s.sample_rate,
                None,
            ));
            d.message = Some(
                match pawcut_export::export_frame(&project, &mut renderer, frame, &out) {
                    Ok(()) => Ok(format!("Frame salvo: {}", out.display())),
                    Err(e) => Err(e.to_string()),
                },
            );
        }
        if start {
            let preset = DELIVER_PRESETS[d.preset];
            let mut output = PathBuf::from(d.output.trim());
            if output.extension().is_none() {
                output.set_extension(preset.4);
            }
            d.output = output.display().to_string();
            d.message = None;
            let range = if d.only_in_out { marks } else { None };
            let size = preset
                .1
                .map(|_| deliver_size(project.settings.width, project.settings.height, preset.1));
            let settings = ExportSettings {
                output,
                crf: d.crf,
                range,
                size,
                codec: preset.2,
                audio_only: preset.3,
            };
            let c = ctx.clone();
            d.job = Some(ExportJob::start((*project).clone(), settings, move || {
                c.request_repaint()
            }));
        }
        if !open {
            if let Some(job) = d.job.as_ref() {
                job.cancel.store(true, Ordering::Relaxed);
            }
            self.deliver = None;
        }
    }

    fn prefs_ui(&mut self, ctx: &egui::Context) {
        if !self.prefs_open {
            return;
        }
        let mut open = true;
        Window::new("Preferences — Teclado")
            .open(&mut open)
            .default_size([520.0, 520.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Clique em “Mudar” e aperte a nova combinação. Esc cancela.")
                            .weak(),
                    );
                    if ui.button("Restaurar padrões (Resolve)").clicked() {
                        self.keymap = Keymap::defaults();
                        self.save_keymap();
                        self.prefs_note = Some("Atalhos restaurados".into());
                    }
                });
                if let Some(note) = &self.prefs_note {
                    ui.colored_label(egui::Color32::from_rgb(240, 190, 90), note);
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        egui::Grid::new("keys")
                            .striped(true)
                            .num_columns(3)
                            .show(ui, |ui| {
                                for action in Action::ALL {
                                    ui.label(action.label());
                                    let text = if self.capturing == Some(action) {
                                        "aperte as teclas…".to_string()
                                    } else {
                                        let chords: Vec<String> = self
                                            .keymap
                                            .chords(action)
                                            .iter()
                                            .map(|c| c.display())
                                            .collect();
                                        if chords.is_empty() {
                                            "—".into()
                                        } else {
                                            chords.join("  ·  ")
                                        }
                                    };
                                    ui.label(RichText::new(text).monospace());
                                    ui.horizontal(|ui| {
                                        if ui.small_button("Mudar").clicked() {
                                            self.capturing = Some(action);
                                            self.prefs_note = None;
                                        }
                                        if ui.small_button("Limpar").clicked() {
                                            self.keymap.clear(action);
                                            self.save_keymap();
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                    });
            });
        if !open {
            self.prefs_open = false;
            self.capturing = None;
        }
    }

    pub fn save_keymap(&mut self) {
        if let Err(e) = self.keymap.save(&config::keymap_path()) {
            self.st
                .say(format!("Não foi possível salvar os atalhos: {e}"));
        }
    }

    fn new_project_ui(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.new_project else {
            return;
        };
        let mut open = true;
        let mut create = None;
        Window::new("Novo projeto")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::ComboBox::from_label("Formato")
                    .selected_text(PRESETS[d.preset].0)
                    .show_ui(ui, |ui| {
                        for (i, (name, w, h, fps)) in PRESETS.iter().enumerate() {
                            if ui.selectable_value(&mut d.preset, i, *name).clicked() {
                                (d.width, d.height, d.fps) = (*w, *h, *fps);
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Resolução");
                    ui.add(egui::DragValue::new(&mut d.width).range(16..=8192));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut d.height).range(16..=8192));
                    ui.label("FPS");
                    ui.add(egui::DragValue::new(&mut d.fps).range(1..=120));
                });
                ui.label(
                    RichText::new("Resolução e FPS não mudam depois de criado.")
                        .weak()
                        .small(),
                );
                if ui.button("Criar").clicked() {
                    create = Some(ProjectSettings {
                        width: d.width,
                        height: d.height,
                        fps: FrameRate::new(d.fps, 1),
                        ..ProjectSettings::default()
                    });
                }
            });
        if let Some(settings) = create {
            self.new_project = None;
            self.request(Pending::New(settings));
        } else if !open {
            self.new_project = None;
        }
    }

    fn confirm_ui(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.confirm.clone() else {
            return;
        };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("confirm-unsaved")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("Alterações não salvas");
            ui.label("O projeto tem alterações que ainda não foram salvas.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Salvar").clicked() {
                    choice = Some(0);
                }
                if ui.button("Descartar").clicked() {
                    choice = Some(1);
                }
                if ui.button("Cancelar").clicked() {
                    choice = Some(2);
                }
            });
        });
        match choice {
            Some(0) => {
                self.confirm = None;
                // Salva e depois continua (na hora, ou quando o diálogo de "Salvar como" responder).
                self.save_then(false, Some(pending));
            }
            Some(1) => {
                self.confirm = None;
                self.discard_autosave();
                self.run_pending(pending, ctx);
            }
            Some(_) => self.confirm = None,
            None => {}
        }
    }

    fn recovery_ui(&mut self, ctx: &egui::Context) {
        if self.recovery.is_empty() {
            return;
        }
        enum Choice {
            Recover(usize),
            Delete(usize),
            Later,
        }
        let mut choice = None;
        egui::Modal::new(egui::Id::new("recovery")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading("Recuperar projeto?");
            ui.label("O PawCut não fechou direito da última vez. Achei estes autosaves:");
            ui.add_space(4.0);
            for (i, p) in self.recovery.iter().enumerate() {
                let when = std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .map_or(String::new(), |e| format!(" — há {} min", e.as_secs() / 60));
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{}{when}",
                        p.file_name().map_or("?".into(), |n| n.to_string_lossy())
                    ));
                    if ui.button("Recuperar").clicked() {
                        choice = Some(Choice::Recover(i));
                    }
                    if ui.small_button("Apagar").clicked() {
                        choice = Some(Choice::Delete(i));
                    }
                });
            }
            ui.add_space(6.0);
            if ui.button("Agora não (perguntar de novo depois)").clicked() {
                choice = Some(Choice::Later);
            }
        });
        match choice {
            Some(Choice::Recover(i)) => {
                let path = self.recovery[i].clone();
                match crate::state::AppState::open(&path) {
                    Ok(mut st) => {
                        st.path = None;
                        st.saved_revision = u64::MAX;
                        self.replace_state(st);
                        // O próprio arquivo recuperado vira o autosave desta sessão:
                        // só some quando o projeto for salvo de verdade.
                        self.autosave_path = path;
                        self.recovery.clear();
                        self.st.say("Projeto recuperado — salve com Ctrl+Shift+S");
                    }
                    Err(e) => self.st.say(format!("Não deu para recuperar: {e}")),
                }
            }
            Some(Choice::Delete(i)) => {
                let path = self.recovery.remove(i);
                let _ = std::fs::remove_file(path);
            }
            Some(Choice::Later) => self.recovery.clear(),
            None => {}
        }
    }

    fn export_quit_ui(&mut self, ctx: &egui::Context) {
        if !self.export_quit_prompt {
            return;
        }
        egui::Modal::new(egui::Id::new("export-quit")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Exportação em andamento");
            ui.label("Fechar agora cancela a exportação (o arquivo incompleto é apagado).");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancelar exportação e sair").clicked() {
                    if let Some(job) = self.deliver.as_ref().and_then(|d| d.job.as_ref()) {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    self.quit_after_export = true;
                    self.export_quit_prompt = false;
                }
                if ui.button("Continuar exportando").clicked() {
                    self.export_quit_prompt = false;
                }
            });
        });
    }
}

/// Tamanho de saída: a altura do preset no formato do projeto (vertical usa a altura como lado menor).
pub fn deliver_size(w: u32, h: u32, target: Option<u32>) -> (u32, u32) {
    let Some(t) = target else { return (w, h) };
    let short = w.min(h) as f32;
    let k = t as f32 / short;
    let even = |v: f32| (v.round() as u32).div_ceil(2) * 2;
    (even(w as f32 * k), even(h as f32 * k))
}
