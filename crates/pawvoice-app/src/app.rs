//! A janela do PawVoice: tomadas à esquerda, forma de onda no centro, roteiro/teleprompter
//! à direita e o transporte (gravar, tocar) embaixo.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use egui::{
    Align2, Color32, CursorIcon, Key, Modifiers, Rect, RichText, Sense, Stroke, pos2, vec2,
};
use paw_audio::dsp;
use paw_audio::io::{Player, Recorder};
use paw_ui::icons::{self, Icon};
use paw_ui::theme;
use paw_ui::toasts::Toasts;
use pawvoice_core::{Editor, Segment, Session, format_time};

use crate::effects::{self, Compression};
use crate::wave;

pub const RATE: u32 = 48_000;

/// Resultado de importar um arquivo: (nome, amostras) ou o erro.
type Imported = Result<(String, Vec<f32>), String>;
const RULER_H: f32 = 24.0;

/// Para que serve a janela de arquivos aberta.
enum Purpose {
    Open,
    SaveAs,
    Import,
    Export(u64),
    ExportAll,
    Script,
}

/// Janelinhas de parâmetros e confirmações.
enum Dialog {
    Gain(f32),
    Gate { threshold: f32, reduction: f32 },
    Compressor(Compression),
    Silences { threshold: f32, min: f32 },
    ConfirmClose,
    Recover(Vec<PathBuf>),
}

/// Áudio renderizado da tomada aberta (refeito quando os trechos mudam).
struct Cache {
    take: u64,
    segments: Vec<Segment>,
    samples: Arc<Vec<f32>>,
    peaks: Vec<(f32, f32)>,
}

pub struct VoiceApp {
    pub ed: Editor,
    pub path: Option<PathBuf>,
    pub selected: Option<u64>,
    /// Cursor (amostra) e seleção `[a, b)` na tomada aberta.
    pub cursor: usize,
    pub sel: Option<(usize, usize)>,
    /// Primeira amostra visível e amostras por pixel.
    pub view_start: f64,
    pub spp: f64,
    cache: Option<Cache>,
    player: Player,
    /// Onde a reprodução começou (para voltar o cursor ao parar, como no Audacity).
    play_from: Option<usize>,
    recorder: Recorder,
    rec_since: Option<Instant>,
    rec_spill: Option<PathBuf>,
    rec_peaks: Vec<(f32, f32)>,
    pub toasts: Toasts,
    dialog: Option<Dialog>,
    file: Option<(Purpose, Receiver<Vec<PathBuf>>)>,
    imports: Option<Receiver<Imported>>,
    pub teleprompter: bool,
    pub wpm: f32,
    prompt_height: f32,
    pub lufs_target: f32,
    renaming: Option<(u64, String)>,
    drag_anchor: Option<usize>,
    allow_close: bool,
    wave_width: f32,
    last_title: String,
    /// Testes visuais: `PAWVOICE_SCREENSHOT=arquivo.png` salva a janela e fecha.
    screenshot: Option<PathBuf>,
    frames: u64,
}

fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pawvoice")
}

/// Gravações em andamento ficam aqui até virarem tomada (para recuperar se o app fechar).
fn spill_dir() -> PathBuf {
    data_dir().join("gravando")
}

fn leftover_recordings() -> Vec<PathBuf> {
    std::fs::read_dir(spill_dir())
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "f32"))
                .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > 0))
                .collect()
        })
        .unwrap_or_default()
}

fn read_raw_f32(path: &Path) -> std::io::Result<Vec<f32>> {
    let bytes = std::fs::read(path)?;
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}

/// Lê um arquivo de áudio qualquer (pelo ffmpeg) em mono na taxa da sessão.
fn decode_mono(path: &Path) -> Result<Vec<f32>, String> {
    let pcm = paw_media::load_audio(path, RATE, 1.0, None).map_err(|e| e.to_string())?;
    Ok((0..pcm.frames())
        .map(|i| {
            let [l, r] = pcm.frame(i as i64);
            (l + r) * 0.5
        })
        .collect())
}

impl VoiceApp {
    pub fn new(ctx: &egui::Context, args: Vec<PathBuf>) -> Self {
        theme::apply(ctx);
        let mut app = Self::empty();
        app.screenshot = std::env::var_os("PAWVOICE_SCREENSHOT").map(PathBuf::from);
        app.toasts.bottom_margin = 48.0;
        let leftovers = leftover_recordings();
        if !leftovers.is_empty() {
            app.dialog = Some(Dialog::Recover(leftovers));
        }
        for a in args {
            app.open_any(&a);
        }
        app
    }

    pub fn empty() -> Self {
        Self {
            ed: Editor::new(Session::new(RATE), Default::default()),
            path: None,
            selected: None,
            cursor: 0,
            sel: None,
            view_start: 0.0,
            spp: 400.0,
            cache: None,
            player: Player::spawn(RATE),
            play_from: None,
            recorder: Recorder::new(RATE),
            rec_since: None,
            rec_spill: None,
            rec_peaks: Vec::new(),
            toasts: Toasts::default(),
            dialog: None,
            file: None,
            imports: None,
            teleprompter: false,
            wpm: 150.0,
            prompt_height: 0.0,
            lufs_target: -16.0,
            renaming: None,
            drag_anchor: None,
            allow_close: false,
            wave_width: 1000.0,
            last_title: String::new(),
            screenshot: None,
            frames: 0,
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.toasts.push(text, false);
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.toasts.push(text, true);
    }

    // ---------------------------------------------------------------- tomada aberta

    fn take_len(&self) -> usize {
        self.selected
            .and_then(|id| self.ed.session().take(id))
            .map_or(0, |t| t.len())
    }

    /// Áudio da tomada aberta (renderiza de novo só se os trechos mudaram).
    fn current(&mut self) -> Option<Arc<Vec<f32>>> {
        let id = self.selected?;
        let take = self.ed.session().take(id)?;
        let fresh = self
            .cache
            .as_ref()
            .is_some_and(|c| c.take == id && c.segments == take.segments);
        if !fresh {
            let segments = take.segments.clone();
            let samples = self.ed.render(id);
            let peaks = wave::peaks(&samples);
            self.cache = Some(Cache {
                take: id,
                segments,
                samples: Arc::new(samples),
                peaks,
            });
        }
        self.cache.as_ref().map(|c| c.samples.clone())
    }

    pub fn select_take(&mut self, id: u64) {
        if self.selected != Some(id) {
            self.stop();
            self.selected = Some(id);
            self.cursor = 0;
            self.sel = None;
            self.fit();
        }
    }

    pub fn fit(&mut self) {
        let len = self.take_len().max(RATE as usize);
        self.spp = (len as f64 / (self.wave_width as f64 - 20.0).max(100.0)).max(1.0);
        self.view_start = 0.0;
    }

    fn zoom(&mut self, factor: f64, anchor_px: f32) {
        let at = self.view_start + anchor_px as f64 * self.spp;
        self.spp = (self.spp / factor).clamp(1.0, 200_000.0);
        self.view_start = (at - anchor_px as f64 * self.spp).max(0.0);
    }

    /// Trecho em que os efeitos agem: a seleção ou a tomada inteira.
    fn target(&self) -> Option<(u64, usize, usize)> {
        let id = self.selected?;
        let len = self.take_len();
        let (a, b) = self.sel.unwrap_or((0, len));
        (b > a).then_some((id, a, b))
    }

    pub fn apply_effect(&mut self, label: &'static str, f: impl FnOnce(&mut Vec<f32>)) {
        self.stop();
        match self.target() {
            Some((id, a, b)) => {
                if let Err(e) = self.ed.process(id, a, b, label, f) {
                    self.warn(e.to_string());
                } else {
                    self.say(format!(
                        "{label}: {}",
                        if self.sel.is_some() {
                            "seleção"
                        } else {
                            "tomada inteira"
                        }
                    ));
                }
            }
            None => self.warn("Grave ou importe uma tomada primeiro"),
        }
    }

    pub fn delete_selection(&mut self) {
        let (Some(id), Some((a, b))) = (self.selected, self.sel) else {
            return;
        };
        self.stop();
        if self.ed.delete(id, a, b).is_ok() {
            self.cursor = a;
            self.sel = None;
        }
    }

    pub fn remove_silences(&mut self, threshold: f32, min: f32) {
        let Some(id) = self.selected else { return };
        self.stop();
        let Some(samples) = self.current() else {
            return;
        };
        let runs = dsp::silent_runs_mono(&samples, RATE, threshold, min as f64, 0.12);
        match self.ed.delete_many(id, &runs) {
            Ok(removed) => {
                self.sel = None;
                self.cursor = 0;
                self.say(format!(
                    "{} pausa(s) removida(s), {:.1} s a menos",
                    runs.len(),
                    removed as f32 / RATE as f32
                ));
            }
            Err(_) => self.say("Nenhuma pausa longa encontrada"),
        }
    }

    // ---------------------------------------------------------------- tocar e gravar

    pub fn toggle_play(&mut self) {
        if self.player.is_playing() {
            self.stop();
            return;
        }
        let Some(samples) = self.current() else {
            return;
        };
        let len = samples.len();
        let (from, to) = match self.sel {
            Some((a, b)) => (a, b),
            None => (if self.cursor >= len { 0 } else { self.cursor }, len),
        };
        if from < to {
            self.play_from = Some(from);
            self.player.play(samples, from, to);
        }
    }

    pub fn stop(&mut self) {
        if self.player.is_playing() {
            self.player.stop();
            if self.sel.is_none() {
                self.cursor = self.player.position();
            }
        }
        self.play_from = None;
    }

    pub fn toggle_record(&mut self) {
        if self.recorder.is_recording() {
            self.finish_recording();
        } else {
            self.stop();
            let dir = spill_dir();
            let _ = std::fs::create_dir_all(&dir);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            let spill = dir.join(format!("{stamp}.f32"));
            self.recorder.start(Some(spill.clone()));
            self.rec_spill = Some(spill);
            self.rec_since = Some(Instant::now());
            self.rec_peaks.clear();
        }
    }

    fn finish_recording(&mut self) {
        let samples = self.recorder.stop();
        self.rec_since = None;
        if samples.len() < RATE as usize / 5 {
            self.warn("Gravação curta demais (nada foi guardado)");
        } else {
            let secs = samples.len() as f32 / RATE as f32;
            let id = self.ed.add_take(samples, None);
            self.selected = None;
            self.select_take(id);
            self.say(format!("Tomada gravada ({secs:.1} s)"));
        }
        if let Some(p) = self.rec_spill.take() {
            let _ = std::fs::remove_file(p);
        }
    }

    // ---------------------------------------------------------------- arquivos

    fn ask(&mut self, purpose: Purpose, run: impl FnOnce() -> Vec<PathBuf> + Send + 'static) {
        if self.file.is_some() {
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(run());
        });
        self.file = Some((purpose, rx));
    }

    fn poll_file(&mut self) {
        let Some(paths) = self.file.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) else {
            return;
        };
        let (purpose, _) = self.file.take().expect("janela aberta");
        let Some(first) = paths.first().cloned() else {
            return;
        };
        match purpose {
            Purpose::Open => self.open_any(&first),
            Purpose::SaveAs => {
                let mut p = first;
                if p.extension().is_none_or(|e| e != "pawvoice") {
                    p.set_extension("pawvoice");
                }
                self.save_to(p);
            }
            Purpose::Import => self.import(paths),
            Purpose::Export(id) => self.export_take(id, &first),
            Purpose::ExportAll => {
                let ids: Vec<(u64, String)> = self
                    .ed
                    .session()
                    .takes
                    .iter()
                    .map(|t| (t.id, t.name.clone()))
                    .collect();
                for (id, name) in &ids {
                    let safe: String = name
                        .chars()
                        .map(|c| {
                            if c.is_alphanumeric() || c == ' ' || c == '-' {
                                c
                            } else {
                                '_'
                            }
                        })
                        .collect();
                    self.export_take(*id, &first.join(format!("{}.wav", safe.trim())));
                }
            }
            Purpose::Script => match std::fs::read_to_string(&first) {
                Ok(text) => {
                    self.ed.set_script(text);
                    self.say("Roteiro carregado");
                }
                Err(e) => self.warn(e.to_string()),
            },
        }
    }

    /// Abre o que vier: sessão `.pawvoice`, roteiro `.txt/.md` ou áudio (vira tomada).
    pub fn open_any(&mut self, path: &Path) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "pawvoice" => match pawvoice_core::load_session(path) {
                Ok(ed) => {
                    self.stop();
                    self.ed = ed;
                    self.path = Some(path.to_path_buf());
                    self.cache = None;
                    self.selected = None;
                    if let Some(id) = self.ed.session().takes.first().map(|t| t.id) {
                        self.select_take(id);
                    }
                }
                Err(e) => self.warn(format!("Não deu para abrir: {e}")),
            },
            "txt" | "md" => {
                if let Ok(text) = std::fs::read_to_string(path) {
                    self.ed.set_script(text);
                    self.say("Roteiro carregado");
                }
            }
            _ => self.import(vec![path.to_path_buf()]),
        }
    }

    /// Importa áudios em segundo plano (cada arquivo vira uma tomada).
    fn import(&mut self, paths: Vec<PathBuf>) {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for p in paths {
                let name = p
                    .file_stem()
                    .map_or("Áudio".into(), |s| s.to_string_lossy().into_owned());
                let _ = tx.send(decode_mono(&p).map(|s| (name, s)));
            }
        });
        self.imports = Some(rx);
        self.say("Importando…");
    }

    fn poll_imports(&mut self) {
        let Some(rx) = &self.imports else { return };
        let mut done = Vec::new();
        while let Ok(r) = rx.try_recv() {
            done.push(r);
        }
        for r in done {
            match r {
                Ok((name, samples)) if !samples.is_empty() => {
                    let id = self.ed.add_take(samples, Some(name));
                    self.selected = None;
                    self.select_take(id);
                }
                Ok(_) => self.warn("O arquivo não tem áudio"),
                Err(e) => self.warn(format!("Não deu para importar: {e}")),
            }
        }
    }

    pub fn save(&mut self) {
        match self.path.clone() {
            Some(p) => self.save_to(p),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) {
        self.ask(Purpose::SaveAs, || {
            rfd::FileDialog::new()
                .set_title("Salvar sessão")
                .add_filter("Sessão PawVoice", &["pawvoice"])
                .set_file_name("narracao.pawvoice")
                .save_file()
                .into_iter()
                .collect()
        });
    }

    fn save_to(&mut self, path: PathBuf) {
        // Backup da versão anterior, como no PawCut.
        let backups = data_dir().join("backups");
        let _ = paw_ui::backups::backup_existing(
            &path,
            &paw_ui::backups::backups_dir_in(&backups, &path),
            paw_ui::backups::BACKUPS_KEPT,
        );
        match pawvoice_core::save_session(&self.ed, &path) {
            Ok(()) => {
                self.ed.mark_saved();
                self.path = Some(path);
                self.say("Sessão salva");
                if self.allow_close {
                    self.allow_close = true;
                }
            }
            Err(e) => self.warn(format!("Não deu para salvar: {e}")),
        }
    }

    fn export_take(&mut self, id: u64, path: &Path) {
        let mut path = path.to_path_buf();
        if path.extension().is_none() {
            path.set_extension("wav");
        }
        let samples = self.ed.render(id);
        match pawvoice_core::export_audio(&samples, RATE, &path) {
            Ok(()) => self.say(format!("Exportado: {}", path.display())),
            Err(e) => self.warn(e.to_string()),
        }
    }

    fn export_dialog(&mut self) {
        let Some(id) = self.selected else {
            self.warn("Nenhuma tomada aberta");
            return;
        };
        let name = self
            .ed
            .session()
            .take(id)
            .map_or("tomada".into(), |t| t.name.clone());
        self.ask(Purpose::Export(id), move || {
            rfd::FileDialog::new()
                .set_title("Exportar tomada")
                .add_filter("WAV", &["wav"])
                .add_filter("MP3", &["mp3"])
                .set_file_name(format!("{name}.wav"))
                .save_file()
                .into_iter()
                .collect()
        });
    }

    // ---------------------------------------------------------------- teclado

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.dialog.is_some() {
            return;
        }
        let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        let (ctrl, shift) = (Modifiers::COMMAND, Modifiers::COMMAND | Modifiers::SHIFT);
        if pressed(Modifiers::NONE, Key::Space) {
            self.toggle_play();
        }
        if pressed(Modifiers::NONE, Key::R) {
            self.toggle_record();
        }
        if pressed(Modifiers::NONE, Key::Delete) || pressed(Modifiers::NONE, Key::Backspace) {
            self.delete_selection();
        }
        if pressed(shift, Key::Z) || pressed(ctrl, Key::Y) {
            self.stop();
            self.ed.redo();
        } else if pressed(ctrl, Key::Z) {
            self.stop();
            self.ed.undo();
        }
        if pressed(ctrl, Key::A) {
            let len = self.take_len();
            self.sel = (len > 0).then_some((0, len));
        }
        if pressed(Modifiers::NONE, Key::Escape) {
            self.sel = None;
        }
        if pressed(shift, Key::S) {
            self.save_as();
        } else if pressed(ctrl, Key::S) {
            self.save();
        }
        if pressed(ctrl, Key::O) {
            self.open_dialog();
        }
        if pressed(ctrl, Key::E) {
            self.export_dialog();
        }
        if pressed(ctrl, Key::L)
            && let (Some(id), Some((a, b))) = (self.selected, self.sel)
        {
            self.stop();
            let _ = self.ed.silence(id, a, b);
        }
        if pressed(ctrl, Key::T)
            && let (Some(id), Some((a, b))) = (self.selected, self.sel)
        {
            self.stop();
            if self.ed.crop(id, a, b).is_ok() {
                self.sel = None;
                self.cursor = 0;
                self.fit();
            }
        }
        if pressed(ctrl, Key::I)
            && let Some(id) = self.selected
        {
            self.stop();
            if let Ok(new) = self.ed.split_take(id, self.cursor) {
                self.selected = None;
                self.select_take(new);
            }
        }
        if pressed(ctrl, Key::F) {
            self.fit();
        }
        if pressed(ctrl, Key::Equals) || pressed(ctrl, Key::Plus) {
            let w = self.wave_width / 2.0;
            self.zoom(1.5, w);
        }
        if pressed(ctrl, Key::Minus) {
            let w = self.wave_width / 2.0;
            self.zoom(1.0 / 1.5, w);
        }
        if pressed(Modifiers::NONE, Key::Home) {
            self.cursor = 0;
            self.view_start = 0.0;
        }
        if pressed(Modifiers::NONE, Key::End) {
            self.cursor = self.take_len();
        }
        let step = |up: bool, s: &mut Self| {
            let takes = &s.ed.session().takes;
            let i = s
                .selected
                .and_then(|id| takes.iter().position(|t| t.id == id));
            let next = match (i, up) {
                (Some(i), true) if i > 0 => Some(takes[i - 1].id),
                (Some(i), false) => takes.get(i + 1).map(|t| t.id),
                (None, _) => takes.first().map(|t| t.id),
                _ => None,
            };
            if let Some(n) = next {
                s.select_take(n);
            }
        };
        if pressed(Modifiers::NONE, Key::ArrowUp) {
            step(true, self);
        }
        if pressed(Modifiers::NONE, Key::ArrowDown) {
            step(false, self);
        }
    }

    fn open_dialog(&mut self) {
        self.ask(Purpose::Open, || {
            rfd::FileDialog::new()
                .set_title("Abrir")
                .add_filter("Sessão PawVoice", &["pawvoice"])
                .add_filter(
                    "Áudio",
                    &["wav", "mp3", "flac", "ogg", "m4a", "aac", "opus"],
                )
                .pick_file()
                .into_iter()
                .collect()
        });
    }

    // ---------------------------------------------------------------- interface

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Arquivo", |ui| {
                if ui.button("Nova sessão").clicked() {
                    ui.close();
                    if self.ed.is_dirty() {
                        self.warn("Salve a sessão atual antes (Ctrl+S)");
                    } else {
                        self.stop();
                        *self = Self {
                            player: std::mem::replace(&mut self.player, Player::spawn(RATE)),
                            ..Self::empty()
                        };
                    }
                }
                if ui
                    .add(egui::Button::new("Abrir…").shortcut_text("Ctrl+O"))
                    .clicked()
                {
                    ui.close();
                    self.open_dialog();
                }
                if ui
                    .add(egui::Button::new("Salvar").shortcut_text("Ctrl+S"))
                    .clicked()
                {
                    ui.close();
                    self.save();
                }
                if ui
                    .add(egui::Button::new("Salvar como…").shortcut_text("Ctrl+Shift+S"))
                    .clicked()
                {
                    ui.close();
                    self.save_as();
                }
                ui.separator();
                if ui.button("Importar áudio…").clicked() {
                    ui.close();
                    self.ask(Purpose::Import, || {
                        rfd::FileDialog::new()
                            .set_title("Importar áudio")
                            .add_filter(
                                "Áudio e vídeo",
                                &[
                                    "wav", "mp3", "flac", "ogg", "m4a", "aac", "opus", "mp4",
                                    "mkv", "mov",
                                ],
                            )
                            .pick_files()
                            .unwrap_or_default()
                    });
                }
                if ui.button("Abrir roteiro (.txt/.md)…").clicked() {
                    ui.close();
                    self.ask(Purpose::Script, || {
                        rfd::FileDialog::new()
                            .add_filter("Texto", &["txt", "md"])
                            .pick_file()
                            .into_iter()
                            .collect()
                    });
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("Exportar tomada (WAV/MP3)…").shortcut_text("Ctrl+E"))
                    .clicked()
                {
                    ui.close();
                    self.export_dialog();
                }
                if ui.button("Exportar todas as tomadas (pasta)…").clicked() {
                    ui.close();
                    self.ask(Purpose::ExportAll, || {
                        rfd::FileDialog::new()
                            .set_title("Pasta para as tomadas")
                            .pick_folder()
                            .into_iter()
                            .collect()
                    });
                }
                ui.separator();
                if ui.button("Sair").clicked() {
                    ui.close();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Editar", |ui| {
                let undo = self
                    .ed
                    .undo_label()
                    .map_or("Desfazer".to_string(), |l| format!("Desfazer: {l}"));
                if ui
                    .add_enabled(
                        self.ed.undo_label().is_some(),
                        egui::Button::new(undo).shortcut_text("Ctrl+Z"),
                    )
                    .clicked()
                {
                    ui.close();
                    self.stop();
                    self.ed.undo();
                }
                let redo = self
                    .ed
                    .redo_label()
                    .map_or("Refazer".to_string(), |l| format!("Refazer: {l}"));
                if ui
                    .add_enabled(
                        self.ed.redo_label().is_some(),
                        egui::Button::new(redo).shortcut_text("Ctrl+Shift+Z"),
                    )
                    .clicked()
                {
                    ui.close();
                    self.stop();
                    self.ed.redo();
                }
                ui.separator();
                let has_sel = self.sel.is_some();
                if ui
                    .add_enabled(
                        has_sel,
                        egui::Button::new("Apagar seleção").shortcut_text("Delete"),
                    )
                    .clicked()
                {
                    ui.close();
                    self.delete_selection();
                }
                if ui
                    .add_enabled(
                        has_sel,
                        egui::Button::new("Silenciar seleção").shortcut_text("Ctrl+L"),
                    )
                    .clicked()
                {
                    ui.close();
                    if let (Some(id), Some((a, b))) = (self.selected, self.sel) {
                        let _ = self.ed.silence(id, a, b);
                    }
                }
                if ui
                    .add_enabled(
                        has_sel,
                        egui::Button::new("Manter só a seleção").shortcut_text("Ctrl+T"),
                    )
                    .clicked()
                {
                    ui.close();
                    if let (Some(id), Some((a, b))) = (self.selected, self.sel)
                        && self.ed.crop(id, a, b).is_ok()
                    {
                        self.sel = None;
                        self.cursor = 0;
                        self.fit();
                    }
                }
                if ui
                    .add(egui::Button::new("Dividir tomada no cursor").shortcut_text("Ctrl+I"))
                    .clicked()
                {
                    ui.close();
                    if let Some(id) = self.selected
                        && let Ok(new) = self.ed.split_take(id, self.cursor)
                    {
                        self.selected = None;
                        self.select_take(new);
                    }
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("Selecionar tudo").shortcut_text("Ctrl+A"))
                    .clicked()
                {
                    ui.close();
                    let len = self.take_len();
                    self.sel = (len > 0).then_some((0, len));
                }
            });
            ui.menu_button("Efeitos", |ui| {
                ui.label(
                    RichText::new(if self.sel.is_some() {
                        "Aplica na seleção"
                    } else {
                        "Aplica na tomada inteira"
                    })
                    .weak()
                    .small(),
                );
                if ui
                    .button(RichText::new("Melhorar voz (1 clique)").strong())
                    .on_hover_text(
                        "Tira o ronco, abaixa o chiado nas pausas, comprime e ajusta o loudness",
                    )
                    .clicked()
                {
                    ui.close();
                    let t = self.lufs_target;
                    self.apply_effect("Melhorar voz", |s| effects::enhance_voice(s, RATE, t));
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Normalizar loudness").clicked() {
                        ui.close();
                        let t = self.lufs_target;
                        self.apply_effect("Loudness", |s| {
                            effects::loudness(s, RATE, t);
                        });
                    }
                    ui.add(
                        egui::DragValue::new(&mut self.lufs_target)
                            .range(-30.0..=-6.0)
                            .speed(0.2)
                            .suffix(" LUFS"),
                    )
                    .on_hover_text("−14 YouTube · −16 narração/podcast · −23 TV");
                });
                if ui.button("Normalizar pico (−1 dB)").clicked() {
                    ui.close();
                    self.apply_effect("Normalizar pico", |s| effects::peak_normalize(s));
                }
                if ui.button("Ganho…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Gain(3.0));
                }
                ui.separator();
                let has_sel = self.sel.is_some();
                if ui
                    .add_enabled(has_sel, egui::Button::new("Fade in"))
                    .on_disabled_hover_text("Selecione o trecho do fade")
                    .clicked()
                {
                    ui.close();
                    self.apply_effect("Fade in", |s| dsp::fade(s, true));
                }
                if ui
                    .add_enabled(has_sel, egui::Button::new("Fade out"))
                    .on_disabled_hover_text("Selecione o trecho do fade")
                    .clicked()
                {
                    ui.close();
                    self.apply_effect("Fade out", |s| dsp::fade(s, false));
                }
                ui.separator();
                if ui.button("Passa-altas 80 Hz (tira ronco)").clicked() {
                    ui.close();
                    self.apply_effect("Passa-altas", |s| dsp::high_pass(s, RATE, 80.0));
                }
                if ui
                    .button("Reduzir chiado nas pausas (noise gate)…")
                    .clicked()
                {
                    ui.close();
                    let floor = self
                        .target()
                        .map(|(id, a, b)| {
                            dsp::noise_floor_db(&self.ed.render_range(id, a, b), RATE)
                        })
                        .unwrap_or(-60.0);
                    self.dialog = Some(Dialog::Gate {
                        threshold: (floor + 8.0).clamp(-80.0, -10.0),
                        reduction: 18.0,
                    });
                }
                if ui.button("Compressor…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Compressor(Compression::Medium));
                }
                ui.separator();
                if ui.button("Remover silêncios…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Silences {
                        threshold: -40.0,
                        min: 0.6,
                    });
                }
            });
        });
    }

    fn takes_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(theme::PAD);
        let recording = self.recorder.is_recording();
        let (label, fill, icon) = if recording {
            (
                "Parar gravação (R)",
                Color32::from_rgb(170, 40, 40),
                Icon::Stop,
            )
        } else {
            ("Gravar (R)", Color32::from_rgb(200, 50, 50), Icon::Record)
        };
        let w = ui.available_width() - theme::PAD;
        let r = ui.add_sized(
            [w, 36.0],
            egui::Button::new(
                RichText::new(label)
                    .strong()
                    .size(14.0)
                    .color(Color32::WHITE),
            )
            .fill(fill),
        );
        let galley_w = 14.0 * label.chars().count() as f32 * 0.5;
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                pos2(r.rect.center().x - galley_w / 2.0 - 16.0, r.rect.center().y),
                vec2(16.0, 16.0),
            ),
            icon,
            Color32::WHITE,
        );
        if r.clicked() {
            self.toggle_record();
        }
        ui.add_space(theme::PAD);
        ui.label(RichText::new("Tomadas").color(theme::TEXT_DIM).small());
        let takes: Vec<(u64, String, usize)> = self
            .ed
            .session()
            .takes
            .iter()
            .map(|t| (t.id, t.name.clone(), t.len()))
            .collect();
        if takes.is_empty() {
            ui.add_space(20.0);
            ui.label(
                RichText::new(
                    "Nenhuma tomada ainda.\nAperte R para gravar,\nou arraste um áudio para cá.",
                )
                .color(theme::TEXT_DIM),
            );
        }
        let mut action: Option<(u64, &str)> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (id, name, len) in takes {
                if let Some((rid, text)) = &mut self.renaming
                    && *rid == id
                {
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                    r.request_focus();
                    if r.lost_focus() {
                        let (rid, text) = self.renaming.take().expect("renomeando");
                        let _ = self.ed.rename_take(rid, text.trim().to_string());
                    }
                    continue;
                }
                let on = self.selected == Some(id);
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                if on || resp.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        4.0,
                        if on {
                            theme::ACCENT.gamma_multiply(0.25)
                        } else {
                            theme::BAR_LIGHT
                        },
                    );
                }
                ui.painter().text(
                    rect.left_top() + vec2(10.0, 6.0),
                    Align2::LEFT_TOP,
                    &name,
                    theme::text(13.0),
                    Color32::WHITE,
                );
                ui.painter().text(
                    rect.left_bottom() + vec2(10.0, -6.0),
                    Align2::LEFT_BOTTOM,
                    format_time(len, RATE),
                    theme::mono(10.5),
                    theme::TEXT_DIM,
                );
                if resp.clicked() {
                    action = Some((id, "open"));
                }
                if resp.double_clicked() {
                    action = Some((id, "rename"));
                }
                resp.context_menu(|ui| {
                    for (label, a) in [
                        ("Renomear", "rename"),
                        ("Duplicar", "dup"),
                        ("Exportar…", "export"),
                        ("Apagar", "delete"),
                    ] {
                        if ui.button(label).clicked() {
                            action = Some((id, a));
                            ui.close();
                        }
                    }
                });
            }
        });
        match action {
            Some((id, "open")) => self.select_take(id),
            Some((id, "rename")) => {
                let name = self
                    .ed
                    .session()
                    .take(id)
                    .map(|t| t.name.clone())
                    .unwrap_or_default();
                self.renaming = Some((id, name));
            }
            Some((id, "dup")) => {
                if let Ok(new) = self.ed.duplicate_take(id) {
                    self.select_take(new);
                }
            }
            Some((id, "export")) => {
                self.selected = Some(id);
                self.export_dialog();
            }
            Some((id, "delete")) => {
                self.stop();
                let _ = self.ed.remove_take(id);
                if self.selected == Some(id) {
                    self.selected = None;
                    if let Some(first) = self.ed.session().takes.first().map(|t| t.id) {
                        self.select_take(first);
                    }
                }
            }
            _ => {}
        }
    }

    fn wave_panel(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        self.wave_width = area.width();
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, theme::BG_DARK);
        if self.recorder.is_recording() {
            self.recording_view(ui, area);
            ui.advance_cursor_after_rect(area);
            return;
        }
        let Some(samples) = self.current() else {
            painter.text(
                area.center(),
                Align2::CENTER_CENTER,
                "Aperte R para gravar a primeira tomada",
                theme::text(16.0),
                theme::TEXT_DIM,
            );
            ui.advance_cursor_after_rect(area);
            return;
        };
        let peaks = self
            .cache
            .as_ref()
            .map(|c| c.peaks.clone())
            .unwrap_or_default();
        let len = samples.len();
        let ruler = Rect::from_min_size(area.min, vec2(area.width(), RULER_H));
        let wave = Rect::from_min_max(pos2(area.left(), ruler.bottom()), area.max);
        let x_of = |s: f64, me: &Self| wave.left() + ((s - me.view_start) / me.spp) as f32;
        let s_at = |x: f32, me: &Self| (me.view_start + (x - wave.left()) as f64 * me.spp).max(0.0);

        // Rolagem e zoom.
        let resp = ui.interact(area, ui.id().with("wave"), Sense::click_and_drag());
        if resp.hovered() {
            let (zoom, scroll, pointer) =
                ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
            if (zoom - 1.0).abs() > 1e-3 {
                let px = pointer.map_or(wave.width() / 2.0, |p| p.x - wave.left());
                self.zoom(zoom as f64, px);
            } else if scroll != egui::Vec2::ZERO {
                self.view_start =
                    (self.view_start - (scroll.x + scroll.y) as f64 * self.spp).max(0.0);
            }
        }
        // Seleção: arrastar; clique põe o cursor; Shift+clique estende.
        if resp.drag_started()
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
        {
            self.stop();
            self.drag_anchor = Some((s_at(p.x, self) as usize).min(len));
        }
        if resp.dragged()
            && let (Some(anchor), Some(p)) = (self.drag_anchor, resp.interact_pointer_pos())
        {
            let at = (s_at(p.x, self) as usize).min(len);
            self.sel = (at != anchor).then(|| (anchor.min(at), anchor.max(at)));
            self.cursor = anchor.min(at);
        }
        if resp.drag_stopped() {
            self.drag_anchor = None;
        }
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.stop();
            let at = (s_at(p.x, self) as usize).min(len);
            if ui.input(|i| i.modifiers.shift) {
                let a = self.cursor;
                self.sel = Some((a.min(at), a.max(at)));
            } else {
                self.sel = None;
                self.cursor = at;
            }
        }

        // Régua.
        painter.rect_filled(ruler, 0.0, theme::BAR);
        let secs_per_px = self.spp / RATE as f64;
        let step = [
            0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0,
        ]
        .into_iter()
        .find(|s| s / secs_per_px >= 90.0)
        .unwrap_or(600.0);
        let first = (self.view_start / RATE as f64 / step).floor() * step;
        let mut t = first;
        while x_of(t * RATE as f64, self) < area.right() {
            let x = x_of(t * RATE as f64, self);
            painter.line_segment(
                [pos2(x, ruler.bottom() - 7.0), pos2(x, ruler.bottom())],
                Stroke::new(1.0, theme::TEXT_DIM),
            );
            painter.text(
                pos2(x + 4.0, ruler.top() + 4.0),
                Align2::LEFT_TOP,
                format_time((t * RATE as f64) as usize, RATE),
                theme::mono(10.0),
                theme::TEXT_DIM,
            );
            t += step;
        }

        // Seleção.
        if let Some((a, b)) = self.sel {
            let r = Rect::from_min_max(
                pos2(x_of(a as f64, self).max(wave.left()), wave.top()),
                pos2(x_of(b as f64, self).min(wave.right()), wave.bottom()),
            );
            painter.rect_filled(r, 0.0, theme::ACCENT.gamma_multiply(0.22));
        }
        // Forma de onda.
        let mid = wave.center().y;
        let half = wave.height() / 2.0 - 8.0;
        painter.line_segment(
            [pos2(wave.left(), mid), pos2(wave.right(), mid)],
            Stroke::new(1.0, theme::BAR_LIGHT),
        );
        let end_x = x_of(len as f64, self).min(wave.right());
        let mut x = wave.left();
        while x < end_x {
            let a = s_at(x, self) as usize;
            let b = (s_at(x + 1.0, self) as usize).max(a + 1);
            let (lo, hi) = wave::range_minmax(&samples, &peaks, a, b);
            // Raiz: fala baixa continua visível (escala parecida com dB, como no PawCut).
            let shape = |v: f32| v.clamp(-1.0, 1.0).abs().sqrt().copysign(v);
            let (lo, hi) = (shape(lo), shape(hi));
            let inside = self.sel.is_some_and(|(sa, sb)| a >= sa && a < sb);
            painter.line_segment(
                [pos2(x, mid - hi * half), pos2(x, mid - lo * half + 0.5)],
                Stroke::new(
                    1.0,
                    if inside {
                        Color32::from_rgb(240, 200, 170)
                    } else {
                        theme::WAVE
                    },
                ),
            );
            x += 1.0;
        }
        // Linha do mouse e cursor.
        if let Some(p) = resp.hover_pos() {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
            painter.line_segment(
                [pos2(p.x, ruler.top()), pos2(p.x, wave.bottom())],
                Stroke::new(1.0, Color32::from_white_alpha(60)),
            );
            painter.text(
                pos2(p.x + 4.0, ruler.bottom() + 4.0),
                Align2::LEFT_TOP,
                format_time(s_at(p.x, self) as usize, RATE),
                theme::mono(10.5),
                Color32::WHITE,
            );
        }
        let head = if self.player.is_playing() {
            self.player.position()
        } else {
            self.cursor
        };
        // Durante a reprodução, a tela acompanha o cursor.
        if self.player.is_playing() && x_of(head as f64, self) > wave.right() - 20.0 {
            self.view_start = head as f64;
        }
        let hx = x_of(head as f64, self);
        if hx >= wave.left() && hx <= wave.right() {
            painter.line_segment(
                [pos2(hx, ruler.top()), pos2(hx, wave.bottom())],
                Stroke::new(1.5, theme::PLAYHEAD),
            );
        }
        ui.advance_cursor_after_rect(area);
    }

    /// Enquanto grava: os últimos 20 s rolando, com o tempo grande.
    fn recording_view(&mut self, ui: &mut egui::Ui, area: Rect) {
        let painter = ui.painter_at(area);
        // Atualiza os picos só com o que chegou de novo.
        let seen = self.rec_peaks.len() * wave::BUCKET;
        let new: Vec<(f32, f32)> = self.recorder.with_buffer(|b| {
            if b.len() > seen {
                let whole = (b.len() - seen) / wave::BUCKET * wave::BUCKET;
                wave::peaks(&b[seen..seen + whole])
            } else {
                Vec::new()
            }
        });
        self.rec_peaks.extend(new);
        let total = self.recorder.len();
        let window = 20 * RATE as usize / wave::BUCKET;
        let start = self.rec_peaks.len().saturating_sub(window);
        let mid = area.center().y;
        let half = area.height() / 2.0 - 40.0;
        let px_per = area.width() / window as f32;
        let shape = |v: f32| v.clamp(-1.0, 1.0).abs().sqrt().copysign(v);
        for (i, (lo, hi)) in self.rec_peaks[start..].iter().enumerate() {
            let (lo, hi) = (&shape(*lo), &shape(*hi));
            let x = area.left() + i as f32 * px_per;
            painter.line_segment(
                [pos2(x, mid - hi * half), pos2(x, mid - lo * half + 0.5)],
                Stroke::new(px_per.max(1.0), Color32::from_rgb(240, 110, 100)),
            );
        }
        painter.text(
            area.center_top() + vec2(0.0, 18.0),
            Align2::CENTER_TOP,
            format!("GRAVANDO  {}", format_time(total, RATE)),
            theme::mono(22.0),
            Color32::from_rgb(240, 90, 80),
        );
        painter.text(
            area.center_bottom() - vec2(0.0, 14.0),
            Align2::CENTER_BOTTOM,
            "R para parar · a gravação fica salva no disco enquanto grava",
            theme::text(12.0),
            theme::TEXT_DIM,
        );
        ui.ctx().request_repaint();
    }

    fn script_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(theme::PAD);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Roteiro").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut self.teleprompter, "Teleprompter")
                    .on_hover_text("Letra grande que rola sozinha enquanto você grava");
            });
        });
        let script = self.ed.session().script.clone();
        let words = script.split_whitespace().count();
        let secs = words as f32 / self.wpm.max(1.0) * 60.0;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{words} palavras · ~{}:{:02}",
                    secs as u32 / 60,
                    secs as u32 % 60
                ))
                .weak()
                .small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.wpm)
                        .range(80.0..=240.0)
                        .speed(1.0)
                        .suffix(" ppm"),
                )
                .on_hover_text("Palavras por minuto (velocidade da leitura)");
            });
        });
        ui.separator();
        if self.teleprompter {
            let auto = self.rec_since.map(|t| t.elapsed().as_secs_f32());
            let mut area = egui::ScrollArea::vertical().auto_shrink(false);
            if let Some(elapsed) = auto
                && secs > 0.0
            {
                // Rola no ritmo da fala; começa com a primeira linha no meio da tela.
                let offset = (elapsed / secs * self.prompt_height).max(0.0);
                area = area.vertical_scroll_offset(offset);
                ui.ctx().request_repaint();
            }
            let out = area.show(ui, |ui| {
                ui.add_space(ui.available_height() * 0.3);
                ui.label(RichText::new(&script).size(28.0).color(Color32::WHITE));
                ui.add_space(300.0);
            });
            self.prompt_height = (out.content_size.y - out.inner_rect.height()).max(1.0);
        } else {
            let mut text = script;
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    let r = ui.add_sized(
                        ui.available_size(),
                        egui::TextEdit::multiline(&mut text).hint_text(
                            "Cole ou escreva o roteiro aqui (ou Arquivo → Abrir roteiro)…",
                        ),
                    );
                    if r.changed() {
                        self.ed.set_script(text.clone());
                    }
                });
        }
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let playing = self.player.is_playing();
        let recording = self.recorder.is_recording();
        ui.horizontal_centered(|ui| {
            ui.add_space(theme::PAD);
            if icons::button(ui, Icon::ToStart, 16.0, false, "Início (Home)").clicked() {
                self.stop();
                self.cursor = 0;
                self.view_start = 0.0;
            }
            if icons::button(
                ui,
                if playing { Icon::Pause } else { Icon::Play },
                18.0,
                playing,
                "Tocar / pausar (Espaço)",
            )
            .clicked()
            {
                self.toggle_play();
            }
            if icons::button(ui, Icon::Stop, 16.0, false, "Parar").clicked() {
                self.stop();
            }
            if icons::button(ui, Icon::Record, 18.0, recording, "Gravar / parar (R)").clicked() {
                self.toggle_record();
            }
            ui.add_space(12.0);
            let head = if playing {
                self.player.position()
            } else {
                self.cursor
            };
            ui.label(
                RichText::new(format!(
                    "{}  /  {}",
                    format_time(head, RATE),
                    format_time(self.take_len(), RATE)
                ))
                .font(theme::mono(15.0))
                .color(theme::TIMECODE),
            );
            if let Some((a, b)) = self.sel {
                ui.add_space(12.0);
                ui.label(
                    RichText::new(format!("seleção {}", format_time(b - a, RATE)))
                        .color(theme::TEXT_DIM),
                );
            }
        });
        // Medidor do microfone à direita.
        let meter = Rect::from_min_size(
            pos2(rect.right() - 230.0, rect.center().y - 6.0),
            vec2(200.0, 12.0),
        );
        let p = ui.painter();
        p.text(
            meter.left_center() - vec2(8.0, 0.0),
            Align2::RIGHT_CENTER,
            "mic",
            theme::text(11.0),
            theme::TEXT_DIM,
        );
        p.rect_filled(meter, 2.0, theme::BG_DARK);
        let level = self.recorder.level();
        let db = 20.0 * level.max(1e-5).log10();
        let frac = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
        let color = if db > -3.0 {
            Color32::from_rgb(235, 70, 60)
        } else if db > -12.0 {
            Color32::from_rgb(235, 200, 70)
        } else {
            Color32::from_rgb(80, 200, 110)
        };
        p.rect_filled(
            Rect::from_min_size(meter.min, vec2(meter.width() * frac, meter.height())),
            2.0,
            color,
        );
        if !self
            .player
            .has_device
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            p.text(
                meter.left_center() - vec2(50.0, 0.0),
                Align2::RIGHT_CENTER,
                "sem saída de áudio",
                theme::text(11.0),
                Color32::from_rgb(240, 150, 90),
            );
        }
        if recording || playing {
            ui.ctx().request_repaint();
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let mut apply = false;
        let mut close = false;
        let title = match dialog {
            Dialog::Gain(_) => "Ganho",
            Dialog::Gate { .. } => "Reduzir chiado (noise gate)",
            Dialog::Compressor(_) => "Compressor",
            Dialog::Silences { .. } => "Remover silêncios",
            Dialog::ConfirmClose => "Sair sem salvar?",
            Dialog::Recover(_) => "Gravação interrompida",
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                match dialog {
                    Dialog::Gain(db) => {
                        ui.add(egui::Slider::new(db, -24.0..=24.0).suffix(" dB"));
                    }
                    Dialog::Gate {
                        threshold,
                        reduction,
                    } => {
                        ui.label(
                            RichText::new("Abaixa o som onde ele fica abaixo do limiar (as pausas).")
                                .weak(),
                        );
                        ui.add(egui::Slider::new(threshold, -80.0..=-10.0).text("limiar").suffix(" dB"));
                        ui.add(egui::Slider::new(reduction, 3.0..=40.0).text("redução").suffix(" dB"));
                    }
                    Dialog::Compressor(c) => {
                        ui.label(RichText::new("Deixa a voz mais uniforme.").weak());
                        ui.horizontal(|ui| {
                            for k in [Compression::Light, Compression::Medium, Compression::Strong] {
                                ui.selectable_value(c, k, k.label());
                            }
                        });
                    }
                    Dialog::Silences { threshold, min } => {
                        ui.label(
                            RichText::new("Corta as pausas longas da tomada inteira (sobra um respiro de 0,12 s).")
                                .weak(),
                        );
                        ui.add(egui::Slider::new(threshold, -70.0..=-20.0).text("abaixo de").suffix(" dB"));
                        ui.add(egui::Slider::new(min, 0.2..=3.0).text("por pelo menos").suffix(" s"));
                    }
                    Dialog::ConfirmClose => {
                        ui.label("A sessão tem mudanças que não foram salvas.");
                    }
                    Dialog::Recover(files) => {
                        ui.label(format!(
                            "{} gravação(ões) não terminou(aram) direito da última vez.",
                            files.len()
                        ));
                        ui.label(RichText::new("Dá para recuperar como tomadas.").weak());
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let (ok, cancel) = match dialog {
                        Dialog::ConfirmClose => ("Sair sem salvar", "Cancelar"),
                        Dialog::Recover(_) => ("Recuperar", "Descartar"),
                        _ => ("Aplicar", "Cancelar"),
                    };
                    if ui.button(RichText::new(ok).strong()).clicked() {
                        apply = true;
                    }
                    if ui.button(cancel).clicked() {
                        close = true;
                    }
                });
            });
        if close {
            if let Some(Dialog::Recover(files)) = self.dialog.take() {
                for f in files {
                    let _ = std::fs::remove_file(f);
                }
            }
            return;
        }
        if !apply {
            return;
        }
        match self.dialog.take() {
            Some(Dialog::Gain(db)) => self.apply_effect("Ganho", |s| dsp::gain(s, db)),
            Some(Dialog::Gate {
                threshold,
                reduction,
            }) => self.apply_effect("Noise gate", |s| {
                dsp::noise_gate(s, RATE, threshold, reduction)
            }),
            Some(Dialog::Compressor(c)) => {
                self.apply_effect("Compressor", |s| effects::compress(s, RATE, c))
            }
            Some(Dialog::Silences { threshold, min }) => self.remove_silences(threshold, min),
            Some(Dialog::ConfirmClose) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(Dialog::Recover(files)) => {
                for f in files {
                    match read_raw_f32(&f) {
                        Ok(s) if !s.is_empty() => {
                            let id = self.ed.add_take(s, Some("Recuperada".into()));
                            self.select_take(id);
                            let _ = std::fs::remove_file(&f);
                        }
                        _ => {
                            let _ = std::fs::remove_file(&f);
                        }
                    }
                }
                self.say("Gravação recuperada. Salve a sessão (Ctrl+S).");
            }
            None => {}
        }
    }

    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll_file();
        self.poll_imports();
        if let Some(e) = self.recorder.take_error() {
            self.rec_since = None;
            self.warn(e);
        }
        // Soltar arquivos na janela.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        for p in dropped {
            self.open_any(&p);
        }
        self.shortcuts(&ctx);
        // Fechar com mudanças não salvas pergunta antes; gravando, guarda a tomada.
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.recorder.is_recording() {
                self.finish_recording();
            }
            if self.ed.is_dirty() && !self.allow_close {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.dialog = Some(Dialog::ConfirmClose);
            }
        }
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Sem título".into(), |s| s.to_string_lossy().into_owned());
        let title = format!(
            "{}{name} — PawVoice",
            if self.ed.is_dirty() { "• " } else { "" }
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        let bar = |fill| {
            egui::Frame::new()
                .fill(fill)
                .inner_margin(egui::Margin::symmetric(6, 0))
        };
        egui::Panel::top("menu")
            .frame(bar(theme::BG_DARK))
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("transport")
            .exact_size(48.0)
            .frame(bar(theme::BAR))
            .show(ui, |ui| self.transport(ui));
        egui::Panel::left("takes")
            .default_size(220.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::same(8)),
            )
            .show(ui, |ui| self.takes_panel(ui));
        egui::Panel::right("script")
            .default_size(340.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::same(8)),
            )
            .show(ui, |ui| self.script_panel(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.wave_panel(ui));
        self.dialogs(&ctx);
        self.toasts.tick(&ctx);
        self.toasts.ui(&ctx);
    }
}

impl VoiceApp {
    fn screenshot_hook(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else {
            return;
        };
        self.frames += 1;
        if self.frames == 40 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let bytes: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
            if let Some(buf) =
                image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, bytes)
            {
                let _ = buf.save(&path);
            }
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

impl eframe::App for VoiceApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
        let ctx = ui.ctx().clone();
        self.screenshot_hook(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_take(samples: Vec<f32>) -> VoiceApp {
        let mut app = VoiceApp::empty();
        let id = app.ed.add_take(samples, None);
        app.select_take(id);
        app
    }

    #[test]
    fn effects_act_on_the_selection_or_the_whole_take() {
        let mut app = app_with_take(vec![0.1; 1000]);
        app.sel = Some((100, 200));
        app.apply_effect("Ganho", |s| dsp::gain(s, 20.0));
        let id = app.selected.unwrap();
        let out = app.ed.render(id);
        assert!((out[150] - 1.0).abs() < 1e-4 && (out[50] - 0.1).abs() < 1e-6);
        app.sel = None;
        app.apply_effect("Ganho", |s| dsp::gain(s, -20.0));
        assert!((app.ed.render(id)[50] - 0.01).abs() < 1e-6);
        app.sel = Some((0, 500));
        app.delete_selection();
        assert_eq!(app.ed.render(id).len(), 500);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn remove_silences_cuts_long_pauses() {
        let tone: Vec<f32> = (0..RATE).map(|i| 0.3 * (i as f32 * 0.05).sin()).collect();
        let mut s = tone.clone();
        s.extend(vec![0.0; RATE as usize * 2]);
        s.extend(tone);
        let mut app = app_with_take(s);
        app.remove_silences(-40.0, 0.6);
        let len = app.ed.render(app.selected.unwrap()).len();
        let secs = len as f32 / RATE as f32;
        assert!((secs - 2.24).abs() < 0.1, "{secs}");
    }
}
