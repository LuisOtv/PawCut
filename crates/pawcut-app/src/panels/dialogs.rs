//! Janelas: Deliver (exportar), Preferences (atalhos), novo projeto, confirmações e recuperação.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use egui::{RichText, Window};
use pawcut_core::{FrameRate, ProjectSettings, format_timecode};
use pawcut_export::ExportSettings;

use crate::config;
use crate::engine::ExportJob;
use crate::keymap::{Action, Keymap};
use crate::workspace::{Pending, Workspace};

pub struct DeliverDialog {
    pub output: String,
    pub crf: u8,
    pub job: Option<ExportJob>,
    pub message: Option<Result<String, String>>,
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
                crf: 18,
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
    }

    fn deliver_ui(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.deliver else { return };
        let mut open = true;
        let project = self.project_arc.clone();
        let s = &project.settings;
        let mut start = false;
        Window::new("Deliver — Exportar")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("MP4 · H.264 + AAC").strong());
                ui.label(format!(
                    "{}×{} · {} fps · duração {}",
                    s.width,
                    s.height,
                    s.fps.as_f64(),
                    format_timecode(project.duration(), s.fps)
                ));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Arquivo");
                    ui.add(egui::TextEdit::singleline(&mut d.output).desired_width(300.0));
                    if ui.button("…").clicked()
                        && let Some(p) = rfd::FileDialog::new()
                            .add_filter("MP4", &["mp4"])
                            .set_file_name("video.mp4")
                            .save_file()
                    {
                        d.output = p.display().to_string();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Qualidade");
                    ui.add(egui::Slider::new(&mut d.crf, 14..=32).text("CRF (menor = melhor)"));
                });
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
                        let ok = project.duration() > 0 && !d.output.trim().is_empty();
                        if ui
                            .add_enabled(
                                ok,
                                egui::Button::new(RichText::new("⏺ Exportar").strong()),
                            )
                            .clicked()
                        {
                            start = true;
                        }
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
        if start {
            let mut output = PathBuf::from(d.output.trim());
            if output.extension().is_none() {
                output.set_extension("mp4");
            }
            d.output = output.display().to_string();
            d.message = None;
            let c = ctx.clone();
            d.job = Some(ExportJob::start(
                (*project).clone(),
                ExportSettings { output, crf: d.crf },
                move || c.request_repaint(),
            ));
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
                if self.save(false) {
                    self.run_pending(pending, ctx);
                }
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
