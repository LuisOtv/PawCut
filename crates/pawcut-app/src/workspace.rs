//! Estado da interface (tudo menos o layout do dock) e o que acontece a cada frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Event, Key, TextureHandle, TextureOptions};
use pawcut_core::{Frame, MediaId, MediaKind, Project, ProjectSettings};

use crate::config;
use crate::engine::{AudioPlayer, PreviewWorker, ViewSlot, Waveforms};
use crate::keymap::{Action, KeyChord, Keymap};
use crate::panels::dialogs::{DeliverDialog, NewProjectDialog};
use crate::panels::timeline::TimelineView;
use crate::state::{AppState, Monitor, PlaceMode, Tool};

const AUTOSAVE_EVERY: Duration = Duration::from_secs(300);
const MEDIA_EXTENSIONS: [&str; 18] = [
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "mts", "mp3", "wav", "flac", "ogg", "m4a", "aac",
    "png", "jpg", "jpeg", "webp", "bmp",
];

#[derive(Debug, Clone, Copy)]
pub enum Playback {
    Stopped,
    /// `audio`: o relógio vem da caixa de som (1x para frente); senão é o relógio do sistema.
    Timeline {
        rate: f64,
        origin: Frame,
        since: Instant,
        audio: bool,
    },
    Source {
        rate: f64,
        origin: Frame,
        since: Instant,
    },
}

impl Playback {
    pub fn is_playing(&self) -> bool {
        !matches!(self, Playback::Stopped)
    }

    fn rate(&self) -> f64 {
        match self {
            Playback::Stopped => 0.0,
            Playback::Timeline { rate, .. } | Playback::Source { rate, .. } => *rate,
        }
    }
}

/// Ação que espera o usuário decidir sobre alterações não salvas.
#[derive(Debug, Clone)]
pub enum Pending {
    New(ProjectSettings),
    Open(PathBuf),
    Quit,
}

pub struct Workspace {
    pub st: AppState,
    pub keymap: Keymap,
    pub ctx: egui::Context,
    pub preview: PreviewWorker,
    pub audio: AudioPlayer,
    pub waveforms: Waveforms,
    pub tex_timeline: Option<TextureHandle>,
    pub tex_source: Option<TextureHandle>,
    pub thumbs: HashMap<MediaId, Option<TextureHandle>>,
    pub requested_timeline: Option<(u64, Frame, (u32, u32))>,
    pub requested_source: Option<(MediaId, Frame, (u32, u32))>,
    pub project_arc: Arc<Project>,
    pub project_rev: u64,
    pub tl: TimelineView,
    pub tl_width: f32,
    pub playback: Playback,
    pub deliver: Option<DeliverDialog>,
    pub prefs_open: bool,
    pub prefs_note: Option<String>,
    pub capturing: Option<Action>,
    pub new_project: Option<NewProjectDialog>,
    pub confirm: Option<Pending>,
    pub recovery: Vec<PathBuf>,
    pub allow_close: bool,
    last_autosave: Instant,
    last_offline_check: Instant,
}

impl Workspace {
    pub fn new(ctx: &egui::Context, st: AppState) -> Self {
        let c = ctx.clone();
        let preview = PreviewWorker::spawn((1920, 1080), 48_000, move || c.request_repaint());
        let project_arc = Arc::new(st.project().clone());
        let project_rev = st.editor.revision();
        Self {
            keymap: Keymap::load(&config::keymap_path()),
            ctx: ctx.clone(),
            preview,
            audio: AudioPlayer::spawn(48_000),
            waveforms: Waveforms::new(),
            tex_timeline: None,
            tex_source: None,
            thumbs: HashMap::new(),
            requested_timeline: None,
            requested_source: None,
            project_arc,
            project_rev,
            tl: TimelineView::default(),
            tl_width: 1200.0,
            playback: Playback::Stopped,
            deliver: None,
            prefs_open: false,
            prefs_note: None,
            capturing: None,
            new_project: None,
            confirm: None,
            recovery: config::leftover_autosaves(),
            allow_close: false,
            last_autosave: Instant::now(),
            last_offline_check: Instant::now(),
            st,
        }
    }

    pub fn refresh_project_arc(&mut self) {
        if self.st.editor.revision() != self.project_rev {
            self.project_arc = Arc::new(self.st.project().clone());
            self.project_rev = self.st.editor.revision();
            if matches!(self.playback, Playback::Timeline { audio: true, .. }) {
                self.audio.update_project(self.project_arc.clone());
            }
        }
    }

    /// Tudo que roda antes de desenhar: resultados do preview, reprodução, atalhos, arquivos soltos.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.refresh_project_arc();
        while let Ok(r) = self.preview.results.try_recv() {
            let size = [r.image.width() as usize, r.image.height() as usize];
            let img = egui::ColorImage::from_rgba_unmultiplied(size, r.image.as_raw());
            let slot = match r.slot {
                ViewSlot::Timeline => &mut self.tex_timeline,
                ViewSlot::Source => &mut self.tex_source,
            };
            match slot {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => {
                    *slot =
                        Some(ctx.load_texture(format!("{:?}", r.slot), img, TextureOptions::LINEAR))
                }
            }
        }
        self.tick();
        self.handle_keys(ctx);
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.open_paths(dropped);
        }
        if self.last_autosave.elapsed() > AUTOSAVE_EVERY {
            self.autosave();
        }
        if self.last_offline_check.elapsed() > Duration::from_secs(3) {
            self.last_offline_check = Instant::now();
            let before = self.st.offline.clone();
            self.st.refresh_offline();
            for id in before.symmetric_difference(&self.st.offline) {
                self.thumbs.remove(id);
            }
        }
        if self.playback.is_playing() {
            ctx.request_repaint();
        }
    }

    // ---------------- reprodução ----------------

    fn tick(&mut self) {
        let fps = self.project_arc.settings.fps;
        let sr = self.project_arc.settings.sample_rate;
        match self.playback {
            Playback::Stopped => {}
            Playback::Timeline {
                rate,
                origin,
                since,
                audio,
            } => {
                let total = self.project_arc.duration();
                let frame = if audio {
                    let s = self.audio.position() as i128;
                    (s * fps.num as i128 / (fps.den as i128 * sr as i128)) as Frame
                } else {
                    origin + (since.elapsed().as_secs_f64() * fps.as_f64() * rate) as Frame
                };
                if rate > 0.0 && frame >= total {
                    self.st.playhead = total;
                    self.stop();
                } else if rate < 0.0 && frame <= 0 {
                    self.st.playhead = 0;
                    self.stop();
                } else {
                    self.st.playhead = frame;
                }
            }
            Playback::Source {
                rate,
                origin,
                since,
            } => {
                let len = self.st.source_length().unwrap_or(0);
                let frame = origin + (since.elapsed().as_secs_f64() * fps.as_f64() * rate) as Frame;
                if frame >= len - 1 || frame <= 0 {
                    self.st.source.playhead = frame.clamp(0, (len - 1).max(0));
                    self.stop();
                } else {
                    self.st.source.playhead = frame;
                }
            }
        }
    }

    pub fn play(&mut self, rate: f64) {
        let now = Instant::now();
        match self.st.active_monitor {
            Monitor::Timeline => {
                self.refresh_project_arc();
                let total = self.project_arc.duration();
                if total == 0 {
                    return;
                }
                if rate > 0.0 && self.st.playhead >= total {
                    self.st.playhead = 0;
                }
                let audio = (rate - 1.0).abs() < 1e-9;
                if audio {
                    let s = &self.project_arc.settings;
                    self.audio.play(
                        self.project_arc.clone(),
                        s.fps.frame_to_sample(self.st.playhead, s.sample_rate),
                    );
                } else {
                    self.audio.stop();
                }
                self.playback = Playback::Timeline {
                    rate,
                    origin: self.st.playhead,
                    since: now,
                    audio,
                };
            }
            Monitor::Source => {
                if self.st.source.media.is_none() {
                    return;
                }
                self.audio.stop();
                self.playback = Playback::Source {
                    rate,
                    origin: self.st.source.playhead,
                    since: now,
                };
            }
        }
    }

    pub fn stop(&mut self) {
        self.audio.stop();
        self.playback = Playback::Stopped;
    }

    pub fn toggle_play(&mut self) {
        if self.playback.is_playing() {
            self.stop()
        } else {
            self.play(1.0)
        }
    }

    /// L: toca para frente; apertar de novo acelera (2x, 4x, 8x).
    fn shuttle(&mut self, forward: bool) {
        let rate = self.playback.rate();
        let next = match (forward, rate) {
            (true, r) if r > 0.0 => (r * 2.0).min(8.0),
            (false, r) if r < 0.0 => (r * 2.0).max(-8.0),
            (true, _) => 1.0,
            (false, _) => -1.0,
        };
        self.play(next);
    }

    pub fn go_to_start(&mut self) {
        self.stop();
        match self.st.active_monitor {
            Monitor::Timeline => self.st.playhead = 0,
            Monitor::Source => self.st.source.playhead = 0,
        }
    }

    pub fn go_to_end(&mut self) {
        self.stop();
        match self.st.active_monitor {
            Monitor::Timeline => self.st.playhead = self.project_arc.duration(),
            Monitor::Source => {
                self.st.source.playhead = (self.st.source_length().unwrap_or(1) - 1).max(0)
            }
        }
    }

    // ---------------- teclado ----------------

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());
        let wants_text = ctx.text_edit_focused();
        let mut handled = Vec::new();
        for (idx, e) in events.iter().enumerate() {
            let Event::Key {
                key,
                pressed: true,
                modifiers,
                repeat,
                ..
            } = e
            else {
                continue;
            };
            if let Some(action) = self.capturing {
                if *key == Key::Escape {
                    self.capturing = None;
                } else if !matches!(key, Key::Escape) {
                    let chord = KeyChord::from_event(*key, *modifiers);
                    let stolen = self.keymap.conflict(&chord, action);
                    self.keymap.set(action, chord);
                    self.save_keymap();
                    self.prefs_note = Some(match stolen {
                        Some(other) => format!(
                            "{} saiu de “{}” e agora é de “{}”",
                            chord.display(),
                            other.label(),
                            action.label()
                        ),
                        None => format!("{} → {}", chord.display(), action.label()),
                    });
                    self.capturing = None;
                }
                handled.push(idx);
                continue;
            }
            if wants_text || self.confirm.is_some() {
                continue;
            }
            let chord = KeyChord::from_event(*key, *modifiers);
            if let Some(action) = self.keymap.action_for(&chord) {
                let repeatable = matches!(
                    action,
                    Action::StepBack
                        | Action::StepForward
                        | Action::Undo
                        | Action::Redo
                        | Action::ZoomIn
                        | Action::ZoomOut
                );
                if !*repeat || repeatable {
                    self.run_action(action);
                }
                handled.push(idx);
            }
        }
        if !handled.is_empty() {
            ctx.input_mut(|i| {
                let mut k = 0;
                i.events.retain(|_| {
                    let keep = !handled.contains(&k);
                    k += 1;
                    keep
                });
            });
        }
    }

    pub fn run_action(&mut self, action: Action) {
        let st = &mut self.st;
        match action {
            Action::PlayPause => self.toggle_play(),
            Action::PlayForward => self.shuttle(true),
            Action::PlayReverse => self.shuttle(false),
            Action::Stop => self.stop(),
            Action::StepBack => {
                self.stop();
                self.st.step(-1);
            }
            Action::StepForward => {
                self.stop();
                self.st.step(1);
            }
            Action::PrevEdit => {
                self.stop();
                self.st.go_to_edit(false);
            }
            Action::NextEdit => {
                self.stop();
                self.st.go_to_edit(true);
            }
            Action::GoToStart => self.go_to_start(),
            Action::GoToEnd => self.go_to_end(),
            Action::MarkIn => st.mark(true),
            Action::MarkOut => st.mark(false),
            Action::ClearMarks => st.clear_marks(),
            Action::SelectionTool => st.tool = Tool::Selection,
            Action::BladeTool => st.tool = Tool::Blade,
            Action::ToggleSnapping => {
                st.snapping = !st.snapping;
                let on = st.snapping;
                st.say(if on {
                    "Snapping ligado"
                } else {
                    "Snapping desligado"
                });
            }
            Action::ToggleLinkedSelection => {
                st.linked_selection = !st.linked_selection;
                let on = st.linked_selection;
                st.say(if on {
                    "Seleção ligada ativada"
                } else {
                    "Seleção ligada desativada"
                });
            }
            Action::BladeAtPlayhead => st.blade_at_playhead(),
            Action::Delete => st.delete_selection(false),
            Action::RippleDelete => st.delete_selection(true),
            Action::InsertEdit => st.place_from_source(PlaceMode::Insert),
            Action::OverwriteEdit => st.place_from_source(PlaceMode::Overwrite),
            Action::AppendEdit => st.place_from_source(PlaceMode::Append),
            Action::LinkUnlink => st.toggle_link(),
            Action::SelectAll => st.select_all(),
            Action::Deselect => st.selection.clear(),
            Action::Undo => st.undo(),
            Action::Redo => st.redo(),
            Action::NewProject => self.new_project = Some(NewProjectDialog::default()),
            Action::Open => self.open_dialog(),
            Action::Save => {
                self.save(false);
            }
            Action::SaveAs => {
                self.save(true);
            }
            Action::Import => self.import_dialog(),
            Action::Deliver => self.open_deliver(),
            Action::ZoomIn => self.zoom(1.4),
            Action::ZoomOut => self.zoom(1.0 / 1.4),
            Action::ZoomToFit => {
                let w = self.tl_width;
                self.zoom_to_fit(w);
            }
        }
    }

    // ---------------- mídia ----------------

    pub fn thumbnail(
        &mut self,
        id: MediaId,
        path: &Path,
        kind: MediaKind,
    ) -> Option<TextureHandle> {
        if kind == MediaKind::Audio || self.st.offline.contains(&id) {
            return None;
        }
        self.thumbs
            .entry(id)
            .or_insert_with(|| {
                let img = pawcut_media::thumbnail(path, (160, 90)).ok()?;
                let size = [img.width() as usize, img.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                Some(
                    self.ctx
                        .load_texture(format!("thumb-{id:?}"), color, TextureOptions::LINEAR),
                )
            })
            .clone()
    }

    pub fn forget_media(&mut self, id: MediaId) {
        self.thumbs.remove(&id);
        self.requested_source = None;
        self.requested_timeline = None;
    }

    pub fn import_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("Importar mídia")
            .add_filter("Vídeo, áudio e imagem", &MEDIA_EXTENSIONS)
            .pick_files()
        {
            self.st.import(&paths);
        }
    }

    /// Arquivos soltos na janela ou passados na linha de comando.
    pub fn open_paths(&mut self, paths: Vec<PathBuf>) {
        let (projects, media): (Vec<_>, Vec<_>) = paths
            .into_iter()
            .partition(|p| p.extension().is_some_and(|e| e == "pawcut"));
        if let Some(p) = projects.into_iter().next() {
            self.request(Pending::Open(p));
        }
        if !media.is_empty() {
            self.st.import(&media);
        }
    }

    // ---------------- arquivo ----------------

    pub fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Abrir projeto")
            .add_filter("Projeto PawCut", &["pawcut"])
            .pick_file()
        {
            self.request(Pending::Open(path));
        }
    }

    /// Faz a ação, perguntando antes se há alterações não salvas.
    pub fn request(&mut self, pending: Pending) {
        if self.st.is_dirty() {
            self.confirm = Some(pending);
        } else {
            let ctx = self.ctx.clone();
            self.run_pending(pending, &ctx);
        }
    }

    pub fn run_pending(&mut self, pending: Pending, ctx: &egui::Context) {
        match pending {
            Pending::New(settings) => {
                self.discard_autosave();
                self.replace_state(AppState::new(Project::new(settings)));
            }
            Pending::Open(path) => match AppState::open(&path) {
                Ok(st) => {
                    self.discard_autosave();
                    self.replace_state(st);
                    let w = self.tl_width;
                    self.zoom_to_fit(w);
                }
                Err(e) => self.st.say(e.to_string()),
            },
            Pending::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    pub fn replace_state(&mut self, st: AppState) {
        self.stop();
        self.st = st;
        self.thumbs.clear();
        self.tex_source = None;
        self.requested_source = None;
        self.requested_timeline = None;
        self.project_rev = u64::MAX;
        self.refresh_project_arc();
    }

    /// Salva (ou "Salvar como"). Devolve se salvou.
    pub fn save(&mut self, save_as: bool) -> bool {
        let path = match (&self.st.path, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let name = self
                    .st
                    .path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map_or("projeto.pawcut".into(), |n| {
                        n.to_string_lossy().into_owned()
                    });
                let Some(mut p) = rfd::FileDialog::new()
                    .set_title("Salvar projeto")
                    .add_filter("Projeto PawCut", &["pawcut"])
                    .set_file_name(name)
                    .save_file()
                else {
                    return false;
                };
                if p.extension().is_none_or(|e| e != "pawcut") {
                    p.set_extension("pawcut");
                }
                p
            }
        };
        match self.st.save_to(&path) {
            Ok(()) => {
                self.discard_autosave();
                self.st.say(format!("Salvo em {}", path.display()));
                true
            }
            Err(e) => {
                self.st.say(e.to_string());
                false
            }
        }
    }

    fn autosave_file(&self) -> PathBuf {
        let stem = self
            .st
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("sem-titulo".into(), |s| s.to_string_lossy().into_owned());
        config::autosave_dir().join(format!("{stem}-autosave.pawcut"))
    }

    fn autosave(&mut self) {
        self.last_autosave = Instant::now();
        if !self.st.is_dirty() {
            return;
        }
        let path = self.autosave_file();
        let result = std::fs::create_dir_all(config::autosave_dir())
            .map_err(|e| e.to_string())
            .and_then(|_| {
                pawcut_core::save_project(self.st.project(), &path).map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            self.st.say(format!("Autosave falhou: {e}"));
        }
    }

    pub fn discard_autosave(&mut self) {
        let _ = std::fs::remove_file(self.autosave_file());
    }
}
