//! Janelas: Deliver (exportar), Preferences (atalhos), novo projeto, confirmações e recuperação.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use egui::{RichText, Window};
use pawcut_core::{FrameRate, ProjectSettings, format_timecode};
use pawcut_export::{AudioFormat, EncoderSpeed, ExportSettings, VideoCodec};

use crate::config;
use crate::engine::ExportJob;
use crate::keymap::{Action, Keymap};
use crate::workspace::{Pending, Workspace};

pub struct DeliverDialog {
    pub output: String,
    pub opts: DeliverOptions,
    /// Preset rápido que gerou as opções atuais (`None` = personalizado).
    pub preset: Option<usize>,
    pub only_in_out: bool,
    pub job: Option<ExportJob>,
    pub message: Option<Result<String, String>>,
}

/// Contêiner do arquivo de saída.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mp4,
    Mov,
    Mkv,
    Mp3,
    Wav,
}

impl Format {
    pub const ALL: [Format; 5] = [
        Format::Mp4,
        Format::Mov,
        Format::Mkv,
        Format::Mp3,
        Format::Wav,
    ];

    pub fn ext(self) -> &'static str {
        match self {
            Format::Mp4 => "mp4",
            Format::Mov => "mov",
            Format::Mkv => "mkv",
            Format::Mp3 => "mp3",
            Format::Wav => "wav",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Format::Mp4 => "MP4",
            Format::Mov => "QuickTime (MOV)",
            Format::Mkv => "Matroska (MKV)",
            Format::Mp3 => "MP3 (só áudio)",
            Format::Wav => "WAV (só áudio)",
        }
    }

    fn audio_only(self) -> Option<AudioFormat> {
        match self {
            Format::Mp3 => Some(AudioFormat::Mp3),
            Format::Wav => Some(AudioFormat::Wav),
            _ => None,
        }
    }

    fn codecs(self) -> &'static [VideoCodec] {
        match self {
            Format::Mp4 => &[VideoCodec::H264, VideoCodec::H265],
            Format::Mov | Format::Mkv => &[VideoCodec::H264, VideoCodec::H265, VideoCodec::ProRes],
            Format::Mp3 | Format::Wav => &[],
        }
    }
}

fn codec_label(c: VideoCodec) -> &'static str {
    match c {
        VideoCodec::H264 => "H.264 (compatível com tudo)",
        VideoCodec::H265 => "H.265 / HEVC (arquivo menor)",
        VideoCodec::ProRes => "Apple ProRes 422 HQ (para editar)",
    }
}

fn speed_label(s: EncoderSpeed) -> &'static str {
    match s {
        EncoderSpeed::Ultrafast => "Muito rápida (arquivo maior)",
        EncoderSpeed::Veryfast => "Rápida",
        EncoderSpeed::Fast => "Um pouco rápida",
        EncoderSpeed::Medium => "Equilibrada",
        EncoderSpeed::Slow => "Lenta (arquivo menor)",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Project,
    /// Lado menor em pixels, mantendo a proporção do projeto.
    Short(u32),
    Fixed(u32, u32),
    Custom,
}

pub const RESOLUTIONS: [(&str, Resolution); 10] = [
    ("Do projeto", Resolution::Project),
    ("2160p (4K UHD)", Resolution::Short(2160)),
    ("1440p (2K)", Resolution::Short(1440)),
    ("1080p (Full HD)", Resolution::Short(1080)),
    ("720p (HD)", Resolution::Short(720)),
    ("480p", Resolution::Short(480)),
    ("1080×1920 — vertical 9:16", Resolution::Fixed(1080, 1920)),
    ("1080×1350 — retrato 4:5", Resolution::Fixed(1080, 1350)),
    ("1080×1080 — quadrado", Resolution::Fixed(1080, 1080)),
    ("Personalizada", Resolution::Custom),
];

pub const FRAME_RATES: [(&str, Option<FrameRate>); 9] = [
    ("Do projeto", None),
    ("23,976", Some(FrameRate::new(24000, 1001))),
    ("24", Some(FrameRate::new(24, 1))),
    ("25", Some(FrameRate::new(25, 1))),
    ("29,97", Some(FrameRate::new(30000, 1001))),
    ("30", Some(FrameRate::new(30, 1))),
    ("50", Some(FrameRate::new(50, 1))),
    ("59,94", Some(FrameRate::new(60000, 1001))),
    ("60", Some(FrameRate::new(60, 1))),
];

/// Tudo que o usuário escolhe na janela de exportação (menos o arquivo e o trecho).
#[derive(Debug, Clone, PartialEq)]
pub struct DeliverOptions {
    pub format: Format,
    pub codec: VideoCodec,
    /// Índice em `RESOLUTIONS`.
    pub resolution: usize,
    pub custom: (u32, u32),
    /// Índice em `FRAME_RATES`.
    pub fps: usize,
    pub use_bitrate: bool,
    pub crf: u8,
    pub bitrate_mbps: f32,
    pub speed: EncoderSpeed,
    pub audio: bool,
    pub audio_kbps: u32,
}

impl Default for DeliverOptions {
    fn default() -> Self {
        Self {
            format: Format::Mp4,
            codec: VideoCodec::H264,
            resolution: 0,
            custom: (1920, 1080),
            fps: 0,
            use_bitrate: false,
            crf: 18,
            bitrate_mbps: 16.0,
            speed: EncoderSpeed::Medium,
            audio: true,
            audio_kbps: 192,
        }
    }
}

impl DeliverOptions {
    /// Tamanho de saída para um projeto `w`×`h`.
    pub fn size(&self, w: u32, h: u32) -> (u32, u32) {
        match RESOLUTIONS[self.resolution].1 {
            Resolution::Project => (w, h),
            Resolution::Short(t) => deliver_size(w, h, Some(t)),
            Resolution::Fixed(a, b) => (a, b),
            Resolution::Custom => (self.custom.0.max(16), self.custom.1.max(16)),
        }
    }

    pub fn export_settings(
        &self,
        project: &pawcut_core::Project,
        output: PathBuf,
        range: Option<(pawcut_core::Frame, pawcut_core::Frame)>,
    ) -> ExportSettings {
        let s = &project.settings;
        let size = self.size(s.width, s.height);
        ExportSettings {
            output,
            crf: self.crf,
            range,
            size: (size != (s.width, s.height)).then_some(size),
            codec: self.codec,
            audio_only: self.format.audio_only(),
            fps: FRAME_RATES[self.fps].1,
            bitrate_kbps: self
                .use_bitrate
                .then_some((self.bitrate_mbps * 1000.0).round() as u32),
            speed: self.speed,
            audio_kbps: self.audio.then_some(self.audio_kbps),
        }
    }
}

/// Presets rápidos (preenchem as opções; dá para ajustar depois).
pub const QUICK_PRESETS: [&str; 7] = [
    "YouTube 1080p",
    "YouTube 4K",
    "Reels / TikTok / Shorts",
    "Rápido 720p",
    "Master ProRes",
    "Só áudio MP3",
    "Só áudio WAV",
];

pub fn quick_preset(i: usize) -> DeliverOptions {
    let base = DeliverOptions::default();
    match i {
        0 => DeliverOptions {
            resolution: 3,
            ..base
        },
        1 => DeliverOptions {
            codec: VideoCodec::H265,
            resolution: 1,
            crf: 20,
            ..base
        },
        2 => DeliverOptions {
            resolution: 6,
            fps: 5,
            crf: 20,
            ..base
        },
        3 => DeliverOptions {
            resolution: 4,
            crf: 23,
            speed: EncoderSpeed::Veryfast,
            audio_kbps: 128,
            ..base
        },
        4 => DeliverOptions {
            format: Format::Mov,
            codec: VideoCodec::ProRes,
            ..base
        },
        5 => DeliverOptions {
            format: Format::Mp3,
            audio_kbps: 256,
            ..base
        },
        _ => DeliverOptions {
            format: Format::Wav,
            ..base
        },
    }
}

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
                opts: quick_preset(0),
                preset: Some(0),
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
        let before = d.opts.clone();
        let mut picked_preset = None;
        Window::new("Deliver — Exportar")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .show(ctx, |ui| {
                // Presets rápidos, como a fileira de presets do Render Settings do Resolve.
                ui.label(RichText::new("Presets").strong());
                ui.horizontal_wrapped(|ui| {
                    for (i, name) in QUICK_PRESETS.iter().enumerate() {
                        if ui.selectable_label(d.preset == Some(i), *name).clicked() {
                            picked_preset = Some(i);
                        }
                    }
                    ui.label(
                        RichText::new(if d.preset.is_none() {
                            "· personalizado"
                        } else {
                            ""
                        })
                        .weak(),
                    );
                });
                ui.separator();

                let label = |ui: &mut egui::Ui, text: &str| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(text).color(crate::theme::TEXT_DIM));
                    });
                };
                egui::Grid::new("deliver-file")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .min_col_width(110.0)
                    .show(ui, |ui| {
                        label(ui, "Arquivo");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut d.output).desired_width(340.0));
                            if ui.button("Procurar…").clicked() {
                                pick_output = true;
                            }
                        });
                        ui.end_row();
                        label(ui, "Formato");
                        egui::ComboBox::from_id_salt("deliver-format")
                            .width(220.0)
                            .selected_text(d.opts.format.label())
                            .show_ui(ui, |ui| {
                                for f in Format::ALL {
                                    ui.selectable_value(&mut d.opts.format, f, f.label());
                                }
                            });
                        ui.end_row();
                        label(ui, "Trecho");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut d.only_in_out, false, "Timeline inteira");
                            ui.add_enabled_ui(marks.is_some(), |ui| {
                                ui.radio_value(&mut d.only_in_out, true, "Só In/Out")
                                    .on_disabled_hover_text("Marque In (I) e Out (O) na timeline");
                            });
                        });
                        ui.end_row();
                    });

                let audio_only = d.opts.format.audio_only().is_some();
                if !audio_only {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Vídeo").strong());
                    egui::Grid::new("deliver-video")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .min_col_width(110.0)
                        .show(ui, |ui| {
                            label(ui, "Codec");
                            egui::ComboBox::from_id_salt("deliver-codec")
                                .width(260.0)
                                .selected_text(codec_label(d.opts.codec))
                                .show_ui(ui, |ui| {
                                    for c in d.opts.format.codecs() {
                                        ui.selectable_value(&mut d.opts.codec, *c, codec_label(*c));
                                    }
                                });
                            ui.end_row();
                            label(ui, "Resolução");
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("deliver-res")
                                    .width(220.0)
                                    .selected_text(RESOLUTIONS[d.opts.resolution].0)
                                    .show_ui(ui, |ui| {
                                        for (i, (name, _)) in RESOLUTIONS.iter().enumerate() {
                                            ui.selectable_value(&mut d.opts.resolution, i, *name);
                                        }
                                    });
                                if RESOLUTIONS[d.opts.resolution].1 == Resolution::Custom {
                                    ui.add(
                                        egui::DragValue::new(&mut d.opts.custom.0)
                                            .range(16..=8192)
                                            .suffix(" L"),
                                    );
                                    ui.label("×");
                                    ui.add(
                                        egui::DragValue::new(&mut d.opts.custom.1)
                                            .range(16..=8192)
                                            .suffix(" A"),
                                    );
                                } else {
                                    let (w, h) = d.opts.size(s.width, s.height);
                                    ui.label(RichText::new(format!("{w}×{h}")).weak());
                                }
                            });
                            ui.end_row();
                            label(ui, "Frame rate");
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("deliver-fps")
                                    .width(120.0)
                                    .selected_text(FRAME_RATES[d.opts.fps].0)
                                    .show_ui(ui, |ui| {
                                        for (i, (name, _)) in FRAME_RATES.iter().enumerate() {
                                            ui.selectable_value(&mut d.opts.fps, i, *name);
                                        }
                                    });
                                ui.label(RichText::new("fps").weak());
                                if d.opts.fps == 0 {
                                    ui.label(
                                        RichText::new(format!("({})", fps_text(s.fps))).weak(),
                                    );
                                }
                            });
                            ui.end_row();
                            if d.opts.codec != VideoCodec::ProRes {
                                label(ui, "Qualidade");
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut d.opts.use_bitrate, false, "Constante");
                                    ui.radio_value(&mut d.opts.use_bitrate, true, "Bitrate");
                                });
                                ui.end_row();
                                ui.label("");
                                if d.opts.use_bitrate {
                                    ui.add(
                                        egui::Slider::new(&mut d.opts.bitrate_mbps, 1.0..=100.0)
                                            .logarithmic(true)
                                            .suffix(" Mb/s"),
                                    );
                                } else {
                                    ui.add(
                                        egui::Slider::new(&mut d.opts.crf, 14..=32)
                                            .text("CRF (menor = melhor)"),
                                    );
                                }
                                ui.end_row();
                                label(ui, "Codificação");
                                egui::ComboBox::from_id_salt("deliver-speed")
                                    .width(220.0)
                                    .selected_text(speed_label(d.opts.speed))
                                    .show_ui(ui, |ui| {
                                        for sp in EncoderSpeed::ALL {
                                            ui.selectable_value(
                                                &mut d.opts.speed,
                                                sp,
                                                speed_label(sp),
                                            );
                                        }
                                    });
                                ui.end_row();
                            }
                        });
                }

                ui.add_space(6.0);
                ui.label(RichText::new("Áudio").strong());
                egui::Grid::new("deliver-audio")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .min_col_width(110.0)
                    .show(ui, |ui| {
                        if !audio_only {
                            label(ui, "Exportar áudio");
                            ui.checkbox(&mut d.opts.audio, "");
                            ui.end_row();
                        }
                        let lossy = d.opts.format != Format::Wav
                            && !(d.opts.codec == VideoCodec::ProRes && !audio_only);
                        if (d.opts.audio || audio_only) && lossy {
                            label(ui, "Bitrate");
                            egui::ComboBox::from_id_salt("deliver-abr")
                                .width(120.0)
                                .selected_text(format!("{} kb/s", d.opts.audio_kbps))
                                .show_ui(ui, |ui| {
                                    for k in [128, 192, 256, 320] {
                                        ui.selectable_value(
                                            &mut d.opts.audio_kbps,
                                            k,
                                            format!("{k} kb/s"),
                                        );
                                    }
                                });
                            ui.end_row();
                        }
                        label(ui, "Amostragem");
                        ui.label(RichText::new(format!("{} Hz · estéreo", s.sample_rate)).weak());
                        ui.end_row();
                    });

                // Resumo do que vai sair.
                let (from, to) = if d.only_in_out {
                    marks.unwrap_or((0, project.duration()))
                } else {
                    (0, project.duration())
                };
                let secs = s.fps.frames_to_seconds(to - from);
                let summary = if audio_only {
                    format!(
                        "{} · duração {}",
                        d.opts.format.label(),
                        format_timecode(to - from, s.fps)
                    )
                } else {
                    let (w, h) = d.opts.size(s.width, s.height);
                    let fps = FRAME_RATES[d.opts.fps].1.unwrap_or(s.fps);
                    let mut t = format!(
                        "{w}×{h} · {} fps · duração {}",
                        fps_text(fps),
                        format_timecode(to - from, s.fps)
                    );
                    if d.opts.use_bitrate && d.opts.codec != VideoCodec::ProRes {
                        let kbps = d.opts.bitrate_mbps as f64 * 1000.0
                            + if d.opts.audio {
                                d.opts.audio_kbps as f64
                            } else {
                                0.0
                            };
                        t.push_str(&format!(" · ~{:.0} MB", kbps * secs / 8000.0));
                    }
                    t
                };
                ui.add_space(6.0);
                ui.separator();
                ui.label(RichText::new(summary).weak());
                ui.add_space(4.0);
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
                                    egui::Button::new(RichText::new("Exportar").strong())
                                        .min_size(egui::vec2(110.0, 26.0)),
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
        if let Some(i) = picked_preset {
            d.opts = quick_preset(i);
        }
        // Codec que não cabe no formato escolhido volta para o primeiro que cabe.
        if let Some(first) = d.opts.format.codecs().first()
            && !d.opts.format.codecs().contains(&d.opts.codec)
        {
            d.opts.codec = *first;
        }
        if d.opts != before {
            d.preset = picked_preset;
            if d.opts.format != before.format {
                // Troca a extensão do arquivo junto com o formato.
                let mut out = PathBuf::from(d.output.trim());
                out.set_extension(d.opts.format.ext());
                d.output = out.display().to_string();
            }
        }
        if pick_output {
            let ext = d.opts.format.ext();
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
            let mut output = PathBuf::from(d.output.trim());
            if output.extension().is_none() {
                output.set_extension(d.opts.format.ext());
            }
            d.output = output.display().to_string();
            d.message = None;
            let range = if d.only_in_out { marks } else { None };
            let settings = d.opts.export_settings(&project, output, range);
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

/// "29,97" / "30" — fps com vírgula, sem casas à toa.
fn fps_text(f: FrameRate) -> String {
    let v = f.as_f64();
    if (v - v.round()).abs() < 1e-6 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.3}").trim_end_matches('0').replace('.', ",")
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

#[cfg(test)]
mod tests {
    use super::*;
    use pawcut_core::{Project, ProjectSettings};

    #[test]
    fn deliver_options_build_export_settings() {
        let project = Project::new(ProjectSettings::default());
        let (w, h) = (project.settings.width, project.settings.height);
        let reels = quick_preset(2).export_settings(&project, "a.mp4".into(), None);
        assert_eq!(reels.size, Some((1080, 1920)));
        assert_eq!(reels.fps, Some(FrameRate::new(30, 1)));
        let same = DeliverOptions::default().export_settings(&project, "a.mp4".into(), None);
        assert_eq!(same.size, None, "resolução do projeto não reescala");
        let opts = DeliverOptions {
            use_bitrate: true,
            bitrate_mbps: 8.0,
            audio: false,
            resolution: 9,
            custom: (640, 360),
            ..DeliverOptions::default()
        };
        let s = opts.export_settings(&project, "a.mkv".into(), Some((10, 20)));
        assert_eq!(s.bitrate_kbps, Some(8000));
        assert_eq!(s.audio_kbps, None);
        assert_eq!(s.size, Some((640, 360)));
        assert_eq!(s.range, Some((10, 20)));
        assert_eq!(
            quick_preset(5)
                .export_settings(&project, "a.mp3".into(), None)
                .audio_only,
            Some(AudioFormat::Mp3)
        );
        assert_eq!(DeliverOptions::default().size(w, h), (w, h));
        assert_eq!(fps_text(FrameRate::new(30000, 1001)), "29,97");
        assert_eq!(fps_text(FrameRate::new(24000, 1001)), "23,976");
    }
}
