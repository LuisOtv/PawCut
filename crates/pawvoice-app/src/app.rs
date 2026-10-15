//! A janela do PawVoice, no jeito do Audacity: barra de transporte e medidores em cima,
//! régua, faixas empilhadas (painel de controle à esquerda, forma de onda à direita),
//! barra de seleção e de status embaixo. O roteiro/teleprompter fica num painel opcional.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, UiBuilder, pos2, vec2,
};
use paw_audio::dsp;
use paw_audio::io::{Player, Recorder};
use paw_ui::icons::{self, Icon};
use paw_ui::theme;
use paw_ui::toasts::Toasts;
use pawvoice_core::{Editor, Segment, Session, format_time, parse_time};

use crate::effects::{self, Compression};
use crate::wave;

pub const RATE: u32 = 48_000;
const RULER_H: f32 = 28.0;
/// Largura do painel de controle da faixa (à esquerda de cada faixa).
const TCP_W: f32 = 176.0;
const TRACK_H: f32 = 132.0;
const TRACK_GAP: f32 = 4.0;
/// Faixa de título do clipe (arrastar por ela move o clipe).
const CLIP_HEADER: f32 = 16.0;

/// Cores no estilo do tema escuro do Audacity.
const WAVE_BLUE: Color32 = Color32::from_rgb(0x68, 0x88, 0xf0);
const WAVE_SEL: Color32 = Color32::from_rgb(0xc0, 0xcc, 0xff);
const CLIP_BG: Color32 = Color32::from_rgb(0x2a, 0x2f, 0x45);
const CLIP_HEAD: Color32 = Color32::from_rgb(0x3a, 0x44, 0x6e);
const TRACK_BG: Color32 = Color32::from_rgb(0x1c, 0x1c, 0x20);

/// Amostras de uma faixa e os picos para desenhar.
type TrackAudio = (Arc<Vec<f32>>, Vec<(f32, f32)>);

/// Resultado de importar um arquivo: (nome, amostras) ou o erro.
type Imported = Result<(String, Vec<f32>), String>;

/// Para que serve a janela de arquivos aberta.
enum Purpose {
    Open,
    SaveAs,
    Import,
    Export { selection: bool },
    ExportTracks,
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

/// O que um arrasto no painel das faixas está fazendo.
#[derive(Clone, Copy)]
enum Drag {
    /// Seleção de tempo; `row` é a faixa onde começou (para selecionar faixas na vertical).
    Select { anchor: usize, row: usize },
    /// Mover o clipe `[a, b)` da faixa `track`, pego `grab` amostras depois do início.
    Clip {
        track: u64,
        a: usize,
        b: usize,
        grab: usize,
    },
}

/// Onde a gravação em andamento vai cair.
#[derive(Clone, Copy)]
struct RecTarget {
    /// `Some`: grava por cima desta faixa; `None`: cria uma faixa nova.
    track: Option<u64>,
    at: usize,
}

/// Áudio e picos de uma faixa (refeitos quando os trechos mudam).
struct TrackCache {
    segments: Vec<Segment>,
    samples: Arc<Vec<f32>>,
    peaks: Vec<(f32, f32)>,
}

pub struct VoiceApp {
    pub ed: Editor,
    pub path: Option<PathBuf>,
    /// Faixas selecionadas (as edições e efeitos agem nelas).
    pub selected: Vec<u64>,
    /// Cursor (amostra) e seleção de tempo `[a, b)`.
    pub cursor: usize,
    pub sel: Option<(usize, usize)>,
    /// Primeira amostra visível, amostras por pixel e rolagem vertical das faixas.
    pub view_start: f64,
    pub spp: f64,
    v_scroll: f32,
    cache: HashMap<u64, TrackCache>,
    player: Player,
    /// Mixagem tocando e em que amostra da timeline ela começa.
    play_buf: Option<Arc<Vec<f32>>>,
    play_offset: usize,
    pub looping: bool,
    recorder: Recorder,
    rec: Option<RecTarget>,
    rec_since: Option<Instant>,
    rec_spill: Option<PathBuf>,
    rec_peaks: Vec<(f32, f32)>,
    /// Toca as outras faixas enquanto grava (como no Audacity).
    pub overdub: bool,
    pub toasts: Toasts,
    dialog: Option<Dialog>,
    file: Option<(Purpose, Receiver<Vec<PathBuf>>)>,
    imports: Vec<Receiver<Imported>>,
    pub show_script: bool,
    pub teleprompter: bool,
    pub wpm: f32,
    prompt_height: f32,
    pub lufs_target: f32,
    renaming: Option<(u64, String)>,
    drag: Option<Drag>,
    allow_close: bool,
    wave_width: f32,
    last_title: String,
    /// Textos dos campos de início/fim da barra de seleção enquanto são editados.
    sel_edit: [Option<String>; 2],
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

/// Gravações em andamento ficam aqui até virarem faixa (para recuperar se o app fechar).
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

/// Lê um arquivo de áudio qualquer (pelo ffmpeg) em mono na taxa do projeto.
fn decode_mono(path: &Path) -> Result<Vec<f32>, String> {
    let pcm = paw_media::load_audio(path, RATE, 1.0, None).map_err(|e| e.to_string())?;
    Ok((0..pcm.frames())
        .map(|i| {
            let [l, r] = pcm.frame(i as i64);
            (l + r) * 0.5
        })
        .collect())
}

/// Raiz: fala baixa continua visível (escala parecida com dB).
fn shape(v: f32) -> f32 {
    v.clamp(-1.0, 1.0).abs().sqrt().copysign(v)
}

impl VoiceApp {
    pub fn new(ctx: &egui::Context, args: Vec<PathBuf>) -> Self {
        theme::apply(ctx);
        let mut app = Self::empty();
        app.screenshot = std::env::var_os("PAWVOICE_SCREENSHOT").map(PathBuf::from);
        app.toasts.bottom_margin = 64.0;
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
            selected: Vec::new(),
            cursor: 0,
            sel: None,
            view_start: 0.0,
            spp: 400.0,
            v_scroll: 0.0,
            cache: HashMap::new(),
            player: Player::spawn(RATE, 2),
            play_buf: None,
            play_offset: 0,
            looping: false,
            recorder: Recorder::new(RATE),
            rec: None,
            rec_since: None,
            rec_spill: None,
            rec_peaks: Vec::new(),
            overdub: true,
            toasts: Toasts::default(),
            dialog: None,
            file: None,
            imports: Vec::new(),
            show_script: false,
            teleprompter: false,
            wpm: 150.0,
            prompt_height: 0.0,
            lufs_target: -16.0,
            renaming: None,
            drag: None,
            allow_close: false,
            wave_width: 1000.0,
            last_title: String::new(),
            sel_edit: [None, None],
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

    // ---------------------------------------------------------------- seleção e alvo

    fn track_ids(&self) -> Vec<u64> {
        self.ed.session().tracks.iter().map(|t| t.id).collect()
    }

    fn sel_tracks(&self) -> Vec<u64> {
        if self.selected.is_empty() {
            self.track_ids()
        } else {
            self.selected.clone()
        }
    }

    /// Faixas e intervalo onde aplicar efeito. Sem nada selecionado, vale tudo
    /// (como o "selecionar tudo se nada estiver selecionado" do Audacity).
    pub fn target(&self) -> Option<(Vec<u64>, usize, usize)> {
        let tracks = self.sel_tracks();
        let (a, b) = self.sel.unwrap_or((0, self.ed.session().len()));
        (!tracks.is_empty() && b > a).then_some((tracks, a, b))
    }

    /// Só com seleção de tempo de verdade (para apagar, recortar…).
    fn time_target(&self) -> Option<(Vec<u64>, usize, usize)> {
        let (a, b) = self.sel?;
        let tracks = self.sel_tracks();
        (!tracks.is_empty() && b > a).then_some((tracks, a, b))
    }

    pub fn select_all(&mut self) {
        self.selected = self.track_ids();
        let len = self.ed.session().len();
        self.sel = (len > 0).then_some((0, len));
    }

    fn select_none(&mut self) {
        self.selected.clear();
        self.sel = None;
    }

    // ---------------------------------------------------------------- edições

    pub fn apply_effect(&mut self, label: &'static str, f: impl FnMut(&mut Vec<f32>)) {
        self.stop();
        match self.target() {
            Some((tracks, a, b)) => {
                if let Err(e) = self.ed.process(&tracks, a, b, label, f) {
                    self.warn(e.to_string());
                }
            }
            None => self.warn("Grave ou importe áudio primeiro"),
        }
    }

    pub fn delete_selection(&mut self) {
        let Some((tracks, a, b)) = self.time_target() else {
            return;
        };
        self.stop();
        if self.ed.delete(&tracks, a, b).is_ok() {
            self.cursor = a;
            self.sel = None;
        }
    }

    pub fn cut(&mut self) {
        if let Some((tracks, a, b)) = self.time_target() {
            self.stop();
            if self.ed.cut(&tracks, a, b).is_ok() {
                self.cursor = a;
                self.sel = None;
            }
        }
    }

    pub fn copy(&mut self) {
        if let Some((tracks, a, b)) = self.time_target() {
            let _ = self.ed.copy(&tracks, a, b);
        }
    }

    pub fn paste(&mut self) {
        self.stop();
        let at = self.sel.map_or(self.cursor, |s| s.0);
        let first = self.selected.first().copied();
        match self.ed.paste(first, at) {
            Ok((targets, len)) => {
                self.selected = targets;
                self.sel = Some((at, at + len));
            }
            Err(e) => self.warn(e.to_string()),
        }
    }

    fn silence(&mut self) {
        if let Some((tracks, a, b)) = self.time_target() {
            self.stop();
            let _ = self.ed.silence(&tracks, a, b);
        }
    }

    fn trim(&mut self) {
        if let Some((tracks, a, b)) = self.time_target() {
            self.stop();
            let _ = self.ed.trim(&tracks, a, b);
        }
    }

    fn duplicate(&mut self) {
        if let Some((tracks, a, b)) = self.time_target() {
            self.stop();
            if let Ok(new) = self.ed.duplicate(&tracks, a, b) {
                self.selected = new;
            }
        }
    }

    fn split_new(&mut self) {
        if let Some((tracks, a, b)) = self.time_target() {
            self.stop();
            if let Ok(new) = self.ed.split_new(&tracks, a, b) {
                self.selected = new;
            }
        }
    }

    fn undo(&mut self) {
        self.stop();
        self.ed.undo();
        self.prune_selection();
    }

    fn redo(&mut self) {
        self.stop();
        self.ed.redo();
        self.prune_selection();
    }

    fn prune_selection(&mut self) {
        let ids = self.track_ids();
        self.selected.retain(|t| ids.contains(t));
    }

    pub fn remove_silences(&mut self, threshold: f32, min: f32) {
        self.stop();
        let mut total = 0;
        for id in self.sel_tracks() {
            let samples = self.ed.render(id);
            let runs = dsp::silent_runs_mono(&samples, RATE, threshold, min as f64, 0.12);
            if let Ok(removed) = self.ed.delete_many(id, &runs) {
                total += removed;
            }
        }
        self.sel = None;
        if total > 0 {
            self.say(format!(
                "{:.1} s de pausas removidos",
                total as f32 / RATE as f32
            ));
        } else {
            self.say("Nenhuma pausa longa encontrada");
        }
    }

    // ---------------------------------------------------------------- zoom

    pub fn fit(&mut self) {
        let len = self.ed.session().len().max(RATE as usize * 5);
        self.spp = (len as f64 / (self.wave_width as f64 - 40.0).max(100.0)).max(1.0);
        self.view_start = 0.0;
    }

    fn zoom(&mut self, factor: f64, anchor_px: f32) {
        let at = self.view_start + anchor_px as f64 * self.spp;
        self.spp = (self.spp / factor).clamp(1.0, 200_000.0);
        self.view_start = (at - anchor_px as f64 * self.spp).max(0.0);
    }

    fn zoom_selection(&mut self) {
        if let Some((a, b)) = self.sel {
            self.spp = ((b - a) as f64 / (self.wave_width as f64 - 40.0).max(100.0)).max(1.0);
            self.view_start = a as f64;
        }
    }

    // ---------------------------------------------------------------- tocar e gravar

    pub fn is_playing(&self) -> bool {
        self.player.is_playing()
    }

    /// Posição do áudio na timeline (tocando, é o que está saindo da caixa de som).
    pub fn position(&self) -> usize {
        if self.recorder.is_recording()
            && let Some(r) = self.rec
        {
            return r.at + self.recorder.len();
        }
        if self.is_playing() {
            self.play_offset + self.player.position()
        } else {
            self.cursor
        }
    }

    /// Toca de `from` até `to`.
    fn play_range(&mut self, from: usize, to: usize) {
        if from >= to {
            return;
        }
        let mix = Arc::new(self.ed.mix(from, to));
        self.play_offset = from;
        self.play_buf = Some(mix.clone());
        self.player.play(mix, 0, to - from);
    }

    /// Espaço: toca a seleção (ou do cursor até o fim) / para.
    pub fn toggle_play(&mut self) {
        if self.recorder.is_recording() {
            self.finish_recording();
            return;
        }
        if self.is_playing() {
            self.stop();
            return;
        }
        let len = self.ed.session().len();
        let (from, to) = match self.sel {
            Some((a, b)) => (a, b),
            None => (if self.cursor >= len { 0 } else { self.cursor }, len),
        };
        self.play_range(from, to);
    }

    /// Para (como o Audacity: o cursor volta para onde a reprodução começou).
    pub fn stop(&mut self) {
        if self.recorder.is_recording() {
            self.finish_recording();
        }
        self.player.stop();
        self.play_buf = None;
    }

    /// P / Pausa: para e deixa o cursor onde estava tocando.
    fn pause(&mut self) {
        if self.is_playing() {
            let at = self.position();
            self.stop();
            self.cursor = at;
            self.sel = None;
        }
    }

    /// R: grava na faixa selecionada a partir do cursor, se ali estiver livre; senão (ou com
    /// `new_track`, Shift+R) numa faixa nova.
    pub fn start_recording(&mut self, new_track: bool) {
        if self.recorder.is_recording() {
            return;
        }
        self.player.stop();
        let at = self.sel.map_or(self.cursor, |s| s.0);
        let track = (!new_track)
            .then(|| self.selected.first().copied())
            .flatten()
            .filter(|id| self.ed.session().track(*id).is_some_and(|t| t.len() <= at));
        self.rec = Some(RecTarget { track, at });
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
        self.sel = None;
        // Overdub: as outras faixas tocam enquanto grava.
        let len = self.ed.session().len();
        if self.overdub && at < len {
            let mix = Arc::new(self.ed.mix(at, len));
            self.play_offset = at;
            self.player.play(mix.clone(), 0, len - at);
            self.play_buf = Some(mix);
        }
    }

    fn finish_recording(&mut self) {
        let samples = self.recorder.stop();
        self.player.stop();
        self.play_buf = None;
        self.rec_since = None;
        let target = self.rec.take();
        if samples.len() < RATE as usize / 10 {
            self.warn("Gravação curta demais (nada foi guardado)");
        } else if let Some(t) = target {
            let len = samples.len();
            let into = t
                .track
                .filter(|id| self.ed.write_at(*id, t.at, samples.clone()).is_ok());
            let id = into.unwrap_or_else(|| self.ed.add_track(samples, t.at, None));
            self.selected = vec![id];
            self.cursor = t.at + len;
        }
        if let Some(p) = self.rec_spill.take() {
            let _ = std::fs::remove_file(p);
        }
    }

    /// Mantém o que está tocando/gravando visível; no fim da reprodução, repete se em loop.
    fn follow(&mut self) {
        let at = self.position() as f64;
        let right = self.view_start + (self.wave_width as f64 - 30.0) * self.spp;
        if (self.is_playing() || self.recorder.is_recording()) && at > right {
            self.view_start = at - 30.0 * self.spp;
        }
        if !self.is_playing() && self.play_buf.is_some() && !self.recorder.is_recording() {
            self.play_buf = None;
            if self.looping {
                let len = self.ed.session().len();
                let (a, b) = self.sel.unwrap_or((self.cursor.min(len), len));
                self.play_range(a, b);
            }
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
            Purpose::Export { selection } => self.export_mix(&first, selection),
            Purpose::ExportTracks => {
                let tracks: Vec<(u64, String)> = self
                    .ed
                    .session()
                    .tracks
                    .iter()
                    .map(|t| (t.id, t.name.clone()))
                    .collect();
                for (id, name) in &tracks {
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
                    let path = first.join(format!("{}.wav", safe.trim()));
                    let samples = self.ed.render(*id);
                    if let Err(e) = pawvoice_core::export_audio(&samples, RATE, 1, &path) {
                        self.warn(e.to_string());
                    }
                }
                self.say(format!("{} faixa(s) exportada(s)", tracks.len()));
            }
            Purpose::Script => match std::fs::read_to_string(&first) {
                Ok(text) => {
                    self.ed.set_script(text);
                    self.show_script = true;
                    self.say("Roteiro carregado");
                }
                Err(e) => self.warn(e.to_string()),
            },
        }
    }

    /// Abre o que vier: projeto `.pawvoice`, roteiro `.txt/.md` ou áudio (vira faixa).
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
                    self.cache.clear();
                    self.select_none();
                    self.cursor = 0;
                    self.fit();
                }
                Err(e) => self.warn(format!("Não deu para abrir: {e}")),
            },
            "txt" | "md" => {
                if let Ok(text) = std::fs::read_to_string(path) {
                    self.ed.set_script(text);
                    self.show_script = true;
                    self.say("Roteiro carregado");
                }
            }
            _ => self.import(vec![path.to_path_buf()]),
        }
    }

    /// Importa áudios em segundo plano (cada arquivo vira uma faixa no começo).
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
        self.imports.push(rx);
    }

    fn poll_imports(&mut self) {
        let mut done = Vec::new();
        // Um canal por importação; some quando a thread termina e não sobra nada.
        self.imports.retain(|rx| {
            loop {
                match rx.try_recv() {
                    Ok(r) => done.push(r),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break true,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break false,
                }
            }
        });
        for r in done {
            match r {
                Ok((name, samples)) if !samples.is_empty() => {
                    let empty = self.ed.session().is_empty();
                    let id = self.ed.add_track(samples, 0, Some(name));
                    self.selected = vec![id];
                    if empty {
                        self.fit();
                    }
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
                .set_title("Salvar projeto")
                .add_filter("Projeto PawVoice", &["pawvoice"])
                .set_file_name("narracao.pawvoice")
                .save_file()
                .into_iter()
                .collect()
        });
    }

    fn save_to(&mut self, path: PathBuf) {
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
                self.say("Projeto salvo");
            }
            Err(e) => self.warn(format!("Não deu para salvar: {e}")),
        }
    }

    /// Exporta a mixagem (estéreo): tudo ou só a seleção.
    fn export_mix(&mut self, path: &Path, selection: bool) {
        let mut path = path.to_path_buf();
        if path.extension().is_none() {
            path.set_extension("wav");
        }
        let (a, b) = if selection {
            self.sel.unwrap_or((0, self.ed.session().len()))
        } else {
            (0, self.ed.session().len())
        };
        if b <= a {
            self.warn("Nada para exportar");
            return;
        }
        let mix = self.ed.mix(a, b);
        match pawvoice_core::export_audio(&mix, RATE, 2, &path) {
            Ok(()) => self.say(format!("Exportado: {}", path.display())),
            Err(e) => self.warn(e.to_string()),
        }
    }

    fn export_dialog(&mut self, selection: bool) {
        if self.ed.session().is_empty() {
            self.warn("Nada para exportar");
            return;
        }
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("audio".into(), |s| s.to_string_lossy().into_owned());
        self.ask(Purpose::Export { selection }, move || {
            rfd::FileDialog::new()
                .set_title("Exportar áudio")
                .add_filter("WAV", &["wav"])
                .add_filter("MP3", &["mp3"])
                .set_file_name(format!("{name}.wav"))
                .save_file()
                .into_iter()
                .collect()
        });
    }

    fn open_dialog(&mut self) {
        self.ask(Purpose::Open, || {
            rfd::FileDialog::new()
                .set_title("Abrir")
                .add_filter("Projeto PawVoice", &["pawvoice"])
                .add_filter(
                    "Áudio",
                    &["wav", "mp3", "flac", "ogg", "m4a", "aac", "opus"],
                )
                .pick_file()
                .into_iter()
                .collect()
        });
    }

    fn import_dialog(&mut self) {
        self.ask(Purpose::Import, || {
            rfd::FileDialog::new()
                .set_title("Importar áudio")
                .add_filter(
                    "Áudio e vídeo",
                    &[
                        "wav", "mp3", "flac", "ogg", "m4a", "aac", "opus", "mp4", "mkv", "mov",
                    ],
                )
                .pick_files()
                .unwrap_or_default()
        });
    }

    // ---------------------------------------------------------------- teclado

    /// Atalhos do Audacity.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.dialog.is_some() {
            return;
        }
        let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        let ctrl = Modifiers::COMMAND;
        let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        if pressed(Modifiers::SHIFT, Key::Space) {
            self.looping = true;
            self.toggle_play();
        } else if pressed(Modifiers::NONE, Key::Space) {
            self.toggle_play();
        }
        if pressed(Modifiers::NONE, Key::X) {
            // X: toca/para deixando o cursor onde parou.
            if self.is_playing() {
                self.pause();
            } else {
                self.toggle_play();
            }
        }
        if pressed(Modifiers::NONE, Key::P) {
            self.pause();
        }
        if pressed(Modifiers::SHIFT, Key::R) {
            self.start_recording(true);
        } else if pressed(Modifiers::NONE, Key::R) {
            if self.recorder.is_recording() {
                self.finish_recording();
            } else {
                self.start_recording(false);
            }
        }
        if pressed(Modifiers::NONE, Key::Delete) || pressed(Modifiers::NONE, Key::Backspace) {
            self.delete_selection();
        }
        if pressed(ctrl_shift, Key::Z) || pressed(ctrl, Key::Y) {
            self.redo();
        } else if pressed(ctrl, Key::Z) {
            self.undo();
        }
        if pressed(ctrl, Key::X) {
            self.cut();
        }
        if pressed(ctrl, Key::C) {
            self.copy();
        }
        if pressed(ctrl, Key::V) {
            self.paste();
        }
        if pressed(ctrl, Key::L) {
            self.silence();
        }
        if pressed(ctrl, Key::T) {
            self.trim();
        }
        if pressed(ctrl, Key::D) {
            self.duplicate();
        }
        if pressed(Modifiers::COMMAND | Modifiers::ALT, Key::I) {
            self.split_new();
        }
        if pressed(ctrl_shift, Key::A) || pressed(Modifiers::NONE, Key::Escape) {
            self.select_none();
        } else if pressed(ctrl, Key::A) {
            self.select_all();
        }
        if pressed(ctrl_shift, Key::S) {
            self.save_as();
        } else if pressed(ctrl, Key::S) {
            self.save();
        }
        if pressed(ctrl, Key::O) {
            self.open_dialog();
        }
        if pressed(ctrl_shift, Key::I) {
            self.import_dialog();
        }
        if pressed(ctrl_shift, Key::E) {
            self.export_dialog(false);
        } else if pressed(ctrl, Key::E) {
            self.zoom_selection();
        }
        if pressed(ctrl, Key::Num1) {
            let w = self.wave_width / 2.0;
            self.zoom(2.0, w);
        }
        if pressed(ctrl, Key::Num3) {
            let w = self.wave_width / 2.0;
            self.zoom(0.5, w);
        }
        if pressed(ctrl, Key::F) {
            self.fit();
        }
        if pressed(Modifiers::NONE, Key::Home) || pressed(Modifiers::NONE, Key::J) {
            self.stop();
            self.cursor = 0;
            self.sel = None;
            self.view_start = 0.0;
        }
        if pressed(Modifiers::NONE, Key::End) || pressed(Modifiers::NONE, Key::K) {
            self.stop();
            self.cursor = self.ed.session().len();
            self.sel = None;
        }
        let step = (self.spp * 10.0).max(RATE as f64 / 100.0) as usize;
        if pressed(Modifiers::NONE, Key::ArrowLeft) {
            self.cursor = self.sel.map_or(self.cursor, |s| s.0).saturating_sub(step);
            self.sel = None;
        }
        if pressed(Modifiers::NONE, Key::ArrowRight) {
            self.cursor = self.sel.map_or(self.cursor, |s| s.1) + step;
            self.sel = None;
        }
    }

    // ---------------------------------------------------------------- menus

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, label: &str, keys: &str, enabled: bool| {
            ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys))
                .clicked()
        };
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Arquivo", |ui| {
                if item(ui, "Novo", "", true) {
                    ui.close();
                    if self.ed.is_dirty() {
                        self.warn("Salve o projeto atual antes (Ctrl+S)");
                    } else {
                        self.stop();
                        self.ed = Editor::new(Session::new(RATE), Default::default());
                        self.path = None;
                        self.cache.clear();
                        self.select_none();
                        self.cursor = 0;
                    }
                }
                if item(ui, "Abrir…", "Ctrl+O", true) {
                    ui.close();
                    self.open_dialog();
                }
                ui.separator();
                if item(ui, "Salvar projeto", "Ctrl+S", true) {
                    ui.close();
                    self.save();
                }
                if item(ui, "Salvar projeto como…", "Ctrl+Shift+S", true) {
                    ui.close();
                    self.save_as();
                }
                ui.separator();
                let has = !self.ed.session().is_empty();
                if item(ui, "Exportar áudio…", "Ctrl+Shift+E", has) {
                    ui.close();
                    self.export_dialog(false);
                }
                if item(ui, "Exportar seleção…", "", self.sel.is_some()) {
                    ui.close();
                    self.export_dialog(true);
                }
                if item(ui, "Exportar faixas separadas (pasta)…", "", has) {
                    ui.close();
                    self.ask(Purpose::ExportTracks, || {
                        rfd::FileDialog::new()
                            .set_title("Pasta para as faixas")
                            .pick_folder()
                            .into_iter()
                            .collect()
                    });
                }
                ui.separator();
                if item(ui, "Importar áudio…", "Ctrl+Shift+I", true) {
                    ui.close();
                    self.import_dialog();
                }
                if item(ui, "Abrir roteiro (.txt/.md)…", "", true) {
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
                if item(ui, "Sair", "", true) {
                    ui.close();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Editar", |ui| {
                let undo = self
                    .ed
                    .undo_label()
                    .map_or("Desfazer".to_string(), |l| format!("Desfazer {l}"));
                if item(ui, &undo, "Ctrl+Z", self.ed.undo_label().is_some()) {
                    ui.close();
                    self.undo();
                }
                let redo = self
                    .ed
                    .redo_label()
                    .map_or("Refazer".to_string(), |l| format!("Refazer {l}"));
                if item(ui, &redo, "Ctrl+Y", self.ed.redo_label().is_some()) {
                    ui.close();
                    self.redo();
                }
                ui.separator();
                let has_sel = self.sel.is_some();
                if item(ui, "Recortar", "Ctrl+X", has_sel) {
                    ui.close();
                    self.cut();
                }
                if item(ui, "Apagar", "Delete", has_sel) {
                    ui.close();
                    self.delete_selection();
                }
                if item(ui, "Copiar", "Ctrl+C", has_sel) {
                    ui.close();
                    self.copy();
                }
                if item(ui, "Colar", "Ctrl+V", self.ed.has_clipboard()) {
                    ui.close();
                    self.paste();
                }
                if item(ui, "Duplicar", "Ctrl+D", has_sel) {
                    ui.close();
                    self.duplicate();
                }
                ui.separator();
                ui.menu_button("Remover especial", |ui| {
                    if item(ui, "Silenciar áudio", "Ctrl+L", has_sel) {
                        ui.close();
                        self.silence();
                    }
                    if item(ui, "Aparar áudio", "Ctrl+T", has_sel) {
                        ui.close();
                        self.trim();
                    }
                    if item(ui, "Separar em nova faixa", "Ctrl+Alt+I", has_sel) {
                        ui.close();
                        self.split_new();
                    }
                });
            });
            ui.menu_button("Selecionar", |ui| {
                if item(ui, "Tudo", "Ctrl+A", true) {
                    ui.close();
                    self.select_all();
                }
                if item(ui, "Nenhum", "Ctrl+Shift+A", true) {
                    ui.close();
                    self.select_none();
                }
                if item(ui, "Do cursor até o fim", "", true) {
                    ui.close();
                    let len = self.ed.session().len();
                    if self.cursor < len {
                        self.sel = Some((self.cursor, len));
                    }
                }
                if item(ui, "Do início até o cursor", "", self.cursor > 0) {
                    ui.close();
                    self.sel = Some((0, self.cursor));
                }
            });
            ui.menu_button("Ver", |ui| {
                if item(ui, "Mais zoom", "Ctrl+1", true) {
                    ui.close();
                    let w = self.wave_width / 2.0;
                    self.zoom(2.0, w);
                }
                if item(ui, "Menos zoom", "Ctrl+3", true) {
                    ui.close();
                    let w = self.wave_width / 2.0;
                    self.zoom(0.5, w);
                }
                if item(ui, "Zoom na seleção", "Ctrl+E", self.sel.is_some()) {
                    ui.close();
                    self.zoom_selection();
                }
                if item(ui, "Caber no projeto", "Ctrl+F", true) {
                    ui.close();
                    self.fit();
                }
                ui.separator();
                ui.checkbox(&mut self.show_script, "Roteiro e teleprompter");
            });
            ui.menu_button("Transporte", |ui| {
                if item(ui, "Tocar / parar", "Espaço", true) {
                    ui.close();
                    self.toggle_play();
                }
                if item(ui, "Pausar", "P", true) {
                    ui.close();
                    self.pause();
                }
                if item(ui, "Gravar", "R", true) {
                    ui.close();
                    self.start_recording(false);
                }
                if item(ui, "Gravar em nova faixa", "Shift+R", true) {
                    ui.close();
                    self.start_recording(true);
                }
                ui.separator();
                ui.checkbox(&mut self.looping, "Repetir (loop)");
                ui.checkbox(&mut self.overdub, "Tocar as outras faixas enquanto grava");
            });
            ui.menu_button("Faixas", |ui| {
                if item(ui, "Nova faixa", "", true) {
                    ui.close();
                    let id = self.ed.add_track(Vec::new(), 0, None);
                    self.selected = vec![id];
                }
                if item(
                    ui,
                    "Apagar faixas selecionadas",
                    "",
                    !self.selected.is_empty(),
                ) {
                    ui.close();
                    self.stop();
                    for id in self.selected.clone() {
                        let _ = self.ed.remove_track(id);
                    }
                    self.selected.clear();
                }
            });
            self.effects_menu(ui);
        });
    }

    fn effects_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Efeitos", |ui| {
            ui.label(
                RichText::new(if self.sel.is_some() {
                    "Aplica na seleção"
                } else {
                    "Nada selecionado: aplica em tudo"
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
                    self.apply_effect("Normalizar loudness", |s| {
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
            if ui.button("Normalizar (pico −1 dB)").clicked() {
                ui.close();
                self.apply_effect("Normalizar", |s| effects::peak_normalize(s));
            }
            if ui.button("Amplificar…").clicked() {
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
            if ui.button("Filtro passa-altas 80 Hz (tira ronco)").clicked() {
                ui.close();
                self.apply_effect("Passa-altas", |s| dsp::high_pass(s, RATE, 80.0));
            }
            if ui.button("Noise gate (chiado nas pausas)…").clicked() {
                ui.close();
                let floor = self
                    .target()
                    .and_then(|(tracks, a, b)| {
                        tracks
                            .first()
                            .map(|t| dsp::noise_floor_db(&self.ed.render_range(*t, a, b), RATE))
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
            if ui.button("Truncar silêncio…").clicked() {
                ui.close();
                self.dialog = Some(Dialog::Silences {
                    threshold: -40.0,
                    min: 0.6,
                });
            }
        });
    }

    // ---------------------------------------------------------------- barra de ferramentas

    /// Botão redondo grande do transporte (como os do Audacity).
    fn big_button(ui: &mut egui::Ui, icon: Icon, tip: &str, on: bool, color: Color32) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
        let bg = if on {
            theme::ACCENT.gamma_multiply(0.35)
        } else if resp.hovered() {
            theme::BAR_LIGHT.gamma_multiply(1.5)
        } else {
            theme::BAR_LIGHT
        };
        ui.painter().circle_filled(rect.center(), 19.0, bg);
        ui.painter()
            .circle_stroke(rect.center(), 19.0, Stroke::new(1.0, theme::BORDER));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), vec2(18.0, 18.0)),
            icon,
            color,
        );
        resp.on_hover_text(tip).clicked()
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let playing = self.is_playing() && !self.recorder.is_recording();
        let recording = self.recorder.is_recording();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add_space(4.0);
            if Self::big_button(ui, Icon::Pause, "Pausar (P)", false, theme::TEXT) {
                self.pause();
            }
            if Self::big_button(
                ui,
                Icon::Play,
                "Tocar (Espaço) · Shift+Espaço repete",
                playing,
                Color32::from_rgb(110, 210, 120),
            ) {
                self.toggle_play();
            }
            if Self::big_button(ui, Icon::Stop, "Parar", false, theme::TEXT) {
                self.stop();
            }
            if Self::big_button(ui, Icon::ToStart, "Ir ao início (Home)", false, theme::TEXT) {
                self.stop();
                self.cursor = 0;
                self.sel = None;
                self.view_start = 0.0;
            }
            if Self::big_button(ui, Icon::ToEnd, "Ir ao fim (End)", false, theme::TEXT) {
                self.stop();
                self.cursor = self.ed.session().len();
                self.sel = None;
            }
            if Self::big_button(
                ui,
                Icon::Record,
                "Gravar (R) · Shift+R grava em faixa nova",
                recording,
                Color32::from_rgb(230, 60, 60),
            ) {
                if recording {
                    self.finish_recording();
                } else {
                    self.start_recording(false);
                }
            }
            if Self::big_button(ui, Icon::Loop, "Repetir (loop)", self.looping, theme::TEXT) {
                self.looping = !self.looping;
            }
            ui.add_space(10.0);
            let sep = |ui: &mut egui::Ui| {
                let (r, _) = ui.allocate_exact_size(vec2(1.0, 30.0), Sense::hover());
                ui.painter().rect_filled(r, 0.0, theme::BORDER);
            };
            sep(ui);
            let small = |ui: &mut egui::Ui, text: &str, tip: &str, enabled: bool| {
                ui.add_enabled(enabled, egui::Button::new(text).min_size(vec2(30.0, 26.0)))
                    .on_hover_text(tip)
                    .clicked()
            };
            let has_sel = self.sel.is_some();
            if small(ui, "Recortar", "Recortar (Ctrl+X)", has_sel) {
                self.cut();
            }
            if small(ui, "Copiar", "Copiar (Ctrl+C)", has_sel) {
                self.copy();
            }
            if small(ui, "Colar", "Colar (Ctrl+V)", self.ed.has_clipboard()) {
                self.paste();
            }
            if small(ui, "Aparar", "Aparar: fica só a seleção (Ctrl+T)", has_sel) {
                self.trim();
            }
            if small(ui, "Silenciar", "Silenciar a seleção (Ctrl+L)", has_sel) {
                self.silence();
            }
            sep(ui);
            if small(
                ui,
                "Desfazer",
                "Desfazer (Ctrl+Z)",
                self.ed.undo_label().is_some(),
            ) {
                self.undo();
            }
            if small(
                ui,
                "Refazer",
                "Refazer (Ctrl+Y)",
                self.ed.redo_label().is_some(),
            ) {
                self.redo();
            }
            sep(ui);
            if icons::button(ui, Icon::Plus, 14.0, false, "Mais zoom (Ctrl+1)").clicked() {
                let w = self.wave_width / 2.0;
                self.zoom(2.0, w);
            }
            if icons::button(ui, Icon::Minus, 14.0, false, "Menos zoom (Ctrl+3)").clicked() {
                let w = self.wave_width / 2.0;
                self.zoom(0.5, w);
            }
            if icons::button(ui, Icon::Fit, 14.0, false, "Caber no projeto (Ctrl+F)").clicked() {
                self.fit();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                self.meters(ui);
            });
        });
    }

    /// Medidores de gravação (vermelho) e reprodução (verde), com a escala em dB.
    fn meters(&mut self, ui: &mut egui::Ui) {
        let play_level = self.play_level();
        let (rect, _) = ui.allocate_exact_size(vec2(330.0, 44.0), Sense::hover());
        let p = ui.painter();
        let rows = [
            ("Mic", self.recorder.level(), Color32::from_rgb(220, 70, 70)),
            ("Saída", play_level, Color32::from_rgb(80, 200, 110)),
        ];
        let bar_left = rect.left() + 40.0;
        for (i, (label, level, color)) in rows.iter().enumerate() {
            let y = rect.top() + 6.0 + i as f32 * 16.0;
            let bar = Rect::from_min_max(pos2(bar_left, y), pos2(rect.right(), y + 11.0));
            p.text(
                pos2(rect.left(), bar.center().y),
                Align2::LEFT_CENTER,
                *label,
                theme::text(10.5),
                theme::TEXT_DIM,
            );
            p.rect_filled(bar, 2.0, theme::BG_DARK);
            let db = 20.0 * level.max(1e-5).log10();
            let frac = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
            let c = if db > -3.0 {
                Color32::from_rgb(235, 60, 50)
            } else if db > -12.0 {
                Color32::from_rgb(235, 200, 70)
            } else {
                *color
            };
            p.rect_filled(
                Rect::from_min_size(bar.min, vec2(bar.width() * frac, bar.height())),
                2.0,
                c,
            );
        }
        // Escala: −60 … 0 dB.
        let w = rect.right() - bar_left;
        for db in [-60, -48, -36, -24, -12, -6, 0] {
            let x = bar_left + w * (db as f32 + 60.0) / 60.0;
            p.text(
                pos2(x, rect.bottom() - 2.0),
                Align2::CENTER_BOTTOM,
                db.to_string(),
                theme::text(8.5),
                theme::TEXT_DIM,
            );
        }
    }

    /// Pico da saída perto da posição que está tocando.
    fn play_level(&self) -> f32 {
        let (Some(buf), true) = (&self.play_buf, self.is_playing()) else {
            return 0.0;
        };
        let at = self.player.position() * 2;
        let w = (RATE as usize / 20) * 2;
        buf.get(at.min(buf.len())..(at + w).min(buf.len()))
            .map_or(0.0, |s| s.iter().fold(0.0f32, |m, v| m.max(v.abs())))
    }

    // ---------------------------------------------------------------- faixas

    /// Áudio e picos da faixa (refaz só se os trechos mudaram).
    fn track_data(&mut self, id: u64) -> Option<TrackAudio> {
        let t = self.ed.session().track(id)?;
        let fresh = self
            .cache
            .get(&id)
            .is_some_and(|c| c.segments == t.segments);
        if !fresh {
            let segments = t.segments.clone();
            let samples = self.ed.render(id);
            let peaks = wave::peaks(&samples);
            self.cache.insert(
                id,
                TrackCache {
                    segments,
                    samples: Arc::new(samples),
                    peaks,
                },
            );
        }
        self.cache
            .get(&id)
            .map(|c| (c.samples.clone(), c.peaks.clone()))
    }

    fn tracks_panel(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        self.wave_width = area.width() - TCP_W;
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, theme::BG_DARK);
        let wave_left = area.left() + TCP_W;
        let x_of = |s: f64, me: &Self| wave_left + ((s - me.view_start) / me.spp) as f32;
        let s_at = |x: f32, me: &Self| (me.view_start + (x - wave_left) as f64 * me.spp).max(0.0);

        // ---- régua (clique toca dali, como a "Timeline" do Audacity)
        let ruler = Rect::from_min_max(
            pos2(wave_left, area.top()),
            pos2(area.right(), area.top() + RULER_H),
        );
        painter.rect_filled(
            Rect::from_min_max(area.min, pos2(area.right(), ruler.bottom())),
            0.0,
            theme::BAR,
        );
        let secs_per_px = self.spp / RATE as f64;
        let step = [
            0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0,
        ]
        .into_iter()
        .find(|s| s / secs_per_px >= 80.0)
        .unwrap_or(600.0);
        let mut t = (self.view_start / RATE as f64 / step).floor() * step;
        let rp = painter.with_clip_rect(ruler);
        while x_of(t * RATE as f64, self) < area.right() {
            let x = x_of(t * RATE as f64, self);
            rp.line_segment(
                [pos2(x, ruler.bottom() - 8.0), pos2(x, ruler.bottom())],
                Stroke::new(1.0, theme::TEXT_DIM),
            );
            for k in 1..5 {
                let xm = x_of((t + step * k as f64 / 5.0) * RATE as f64, self);
                rp.line_segment(
                    [pos2(xm, ruler.bottom() - 4.0), pos2(xm, ruler.bottom())],
                    Stroke::new(1.0, Color32::from_gray(70)),
                );
            }
            rp.text(
                pos2(x + 3.0, ruler.top() + 4.0),
                Align2::LEFT_TOP,
                format_time((t * RATE as f64) as usize, RATE),
                theme::mono(10.0),
                theme::TEXT,
            );
            t += step;
        }
        if let Some((a, b)) = self.sel {
            rp.rect_filled(
                Rect::from_min_max(
                    pos2(x_of(a as f64, self), ruler.bottom() - 5.0),
                    pos2(x_of(b as f64, self), ruler.bottom()),
                ),
                0.0,
                theme::ACCENT.gamma_multiply(0.7),
            );
        }
        let ruler_resp = ui
            .interact(ruler, ui.id().with("ruler"), Sense::click())
            .on_hover_text("Clique para tocar a partir daqui");
        if ruler_resp.clicked()
            && let Some(p) = ruler_resp.interact_pointer_pos()
        {
            let at = s_at(p.x, self) as usize;
            self.stop();
            self.cursor = at;
            self.sel = None;
            let len = self.ed.session().len();
            self.play_range(at, len);
        }

        // ---- faixas
        let lanes = Rect::from_min_max(pos2(area.left(), ruler.bottom()), area.max);
        type Row = (u64, String, f32, f32, bool, bool);
        let tracks: Vec<Row> = self
            .ed
            .session()
            .tracks
            .iter()
            .map(|t| (t.id, t.name.clone(), t.gain_db, t.pan, t.mute, t.solo))
            .collect();
        let rec_now = self.recorder.is_recording().then_some(self.rec).flatten();
        let new_rec_lane = rec_now.is_some_and(|r| r.track.is_none());
        let rows = tracks.len() + usize::from(new_rec_lane);
        let content_h = rows as f32 * (TRACK_H + TRACK_GAP);
        // Roda: rola as faixas; Shift+roda: anda no tempo; Ctrl+roda: zoom.
        let pointer = ui.input(|i| i.pointer.hover_pos());
        if pointer.is_some_and(|p| lanes.contains(p)) {
            let (zoom, scroll, shift) =
                ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.modifiers.shift));
            if (zoom - 1.0).abs() > 1e-3 {
                let px = pointer.map_or(self.wave_width / 2.0, |p| p.x - wave_left);
                self.zoom(zoom as f64, px);
            } else if scroll != egui::Vec2::ZERO {
                if shift || scroll.x != 0.0 {
                    let d = if scroll.x != 0.0 { scroll.x } else { scroll.y };
                    self.view_start = (self.view_start - d as f64 * self.spp).max(0.0);
                } else {
                    self.v_scroll = (self.v_scroll - scroll.y)
                        .clamp(0.0, (content_h - lanes.height() + 40.0).max(0.0));
                }
            }
        }
        let row_rect = |i: usize, me: &Self| {
            let top = lanes.top() + i as f32 * (TRACK_H + TRACK_GAP) - me.v_scroll;
            Rect::from_min_size(pos2(area.left(), top), vec2(area.width(), TRACK_H))
        };
        let n_tracks = tracks.len();
        let row_at = |y: f32, me: &Self| -> Option<usize> {
            let i = ((y - lanes.top() + me.v_scroll) / (TRACK_H + TRACK_GAP)).floor();
            (i >= 0.0 && (i as usize) < n_tracks).then_some(i as usize)
        };
        let lp = painter.with_clip_rect(lanes);

        // Interação na área das formas de onda (selecionar, mover clipe).
        let wave_area = Rect::from_min_max(pos2(wave_left, lanes.top()), lanes.max);
        let resp = ui.interact(wave_area, ui.id().with("waves"), Sense::click_and_drag());
        let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        // Clipe sob o ponteiro: (faixa, início, fim, no cabeçalho?).
        let clip_under = |p: Pos2, me: &Self| -> Option<(u64, usize, usize, bool)> {
            let i = row_at(p.y, me)?;
            let id = tracks[i].0;
            let s = s_at(p.x, me) as usize;
            let r = row_rect(i, me);
            me.ed
                .session()
                .track(id)?
                .clips()
                .into_iter()
                .find(|(a, b)| s >= *a && s < *b)
                .map(|(a, b)| (id, a, b, p.y < r.top() + CLIP_HEADER + 1.0))
        };
        if let Some(p) = resp.hover_pos()
            && self.drag.is_none()
        {
            ui.ctx().set_cursor_icon(match clip_under(p, self) {
                Some((_, _, _, true)) => CursorIcon::Grab,
                _ => CursorIcon::Text,
            });
        }
        if resp.drag_started()
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
        {
            self.stop();
            self.drag = Some(match clip_under(p, self) {
                Some((id, a, b, true)) => Drag::Clip {
                    track: id,
                    a,
                    b,
                    grab: (s_at(p.x, self) as usize).saturating_sub(a),
                },
                _ => Drag::Select {
                    anchor: s_at(p.x, self) as usize,
                    row: row_at(p.y, self).unwrap_or(0),
                },
            });
        }
        let mut move_ghost: Option<(usize, usize, usize)> = None;
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            match self.drag {
                Some(Drag::Select { anchor, row }) => {
                    let at = s_at(p.x, self) as usize;
                    self.sel = (at != anchor).then(|| (anchor.min(at), anchor.max(at)));
                    self.cursor = anchor.min(at);
                    // Arrastar na vertical seleciona as faixas entre a de começo e a atual.
                    if n_tracks > 0 {
                        let now = row_at(p.y, self).unwrap_or(row);
                        let (r0, r1) = (row.min(now), row.max(now).min(n_tracks - 1));
                        self.selected = tracks[r0.min(r1)..=r1].iter().map(|t| t.0).collect();
                    }
                }
                Some(Drag::Clip { a, b, grab, track }) => {
                    let start = (s_at(p.x, self) as usize).saturating_sub(grab);
                    let row = row_at(p.y, self)
                        .unwrap_or_else(|| tracks.iter().position(|t| t.0 == track).unwrap_or(0));
                    move_ghost = Some((row, start, b - a));
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
                None => {}
            }
        }
        if resp.drag_stopped() {
            if let (Some(Drag::Clip { track, a, b, grab }), Some(p)) =
                (self.drag, ui.input(|i| i.pointer.latest_pos()))
            {
                let start = (s_at(p.x, self) as usize).saturating_sub(grab);
                let to = row_at(p.y, self).map_or(track, |r| tracks[r].0);
                if (to, start) != (track, a) && self.ed.move_clip(track, a, b, to, start).is_ok() {
                    self.selected = vec![to];
                    self.sel = Some((start, start + (b - a)));
                }
            }
            self.drag = None;
        }
        if resp.double_clicked()
            && let Some(p) = resp.interact_pointer_pos()
            && let Some((id, a, b, _)) = clip_under(p, self)
        {
            // Duplo clique seleciona o clipe inteiro.
            self.selected = vec![id];
            self.sel = Some((a, b));
        } else if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.stop();
            let at = s_at(p.x, self) as usize;
            match row_at(p.y, self) {
                Some(r) => {
                    let id = tracks[r].0;
                    if shift {
                        // Shift+clique estende a seleção de tempo.
                        let a = self.sel.map_or(self.cursor, |s| s.0);
                        self.sel = Some((a.min(at), a.max(at)));
                        if !self.selected.contains(&id) {
                            self.selected.push(id);
                        }
                    } else if cmd {
                        if !self.selected.contains(&id) {
                            self.selected.push(id);
                        }
                    } else {
                        self.selected = vec![id];
                        self.sel = None;
                        self.cursor = at;
                    }
                }
                None => {
                    self.select_none();
                    self.cursor = at;
                }
            }
        }

        // Desenho de cada faixa.
        for (i, (id, name, gain, pan, mute, solo)) in tracks.iter().enumerate() {
            let r = row_rect(i, self);
            if r.bottom() < lanes.top() || r.top() > lanes.bottom() {
                continue;
            }
            let selected = self.selected.contains(id);
            let wave = Rect::from_min_max(pos2(wave_left, r.top()), r.max);
            lp.rect_filled(
                wave,
                0.0,
                if selected {
                    TRACK_BG.gamma_multiply(1.25)
                } else {
                    TRACK_BG
                },
            );
            let Some((samples, peaks)) = self.track_data(*id) else {
                continue;
            };
            let wp = lp.with_clip_rect(wave.intersect(lanes));
            // Seleção de tempo nesta faixa.
            if selected && let Some((a, b)) = self.sel {
                wp.rect_filled(
                    Rect::from_min_max(
                        pos2(x_of(a as f64, self), wave.top()),
                        pos2(x_of(b as f64, self), wave.bottom()),
                    ),
                    0.0,
                    Color32::from_white_alpha(28),
                );
            }
            let clips = self
                .ed
                .session()
                .track(*id)
                .map(|t| t.clips())
                .unwrap_or_default();
            for (ci, (a, b)) in clips.iter().enumerate() {
                let (x0, x1) = (x_of(*a as f64, self), x_of(*b as f64, self));
                if x1 < wave.left() || x0 > wave.right() {
                    continue;
                }
                let body =
                    Rect::from_min_max(pos2(x0, wave.top() + 1.0), pos2(x1, wave.bottom() - 1.0));
                wp.rect_filled(body, 4.0, CLIP_BG);
                let head =
                    Rect::from_min_max(body.min, pos2(body.right(), body.top() + CLIP_HEADER));
                wp.rect_filled(
                    head,
                    CornerRadius {
                        nw: 4,
                        ne: 4,
                        sw: 0,
                        se: 0,
                    },
                    CLIP_HEAD,
                );
                wp.with_clip_rect(head.intersect(wave)).text(
                    head.left_center() + vec2(6.0, 0.0),
                    Align2::LEFT_CENTER,
                    format!("{name} #{}", ci + 1),
                    theme::text(10.5),
                    Color32::from_gray(220),
                );
                // Forma de onda.
                let inner = Rect::from_min_max(pos2(body.left(), head.bottom()), body.max);
                let mid = inner.center().y;
                let half = inner.height() / 2.0 - 4.0;
                wp.line_segment(
                    [pos2(inner.left(), mid), pos2(inner.right(), mid)],
                    Stroke::new(1.0, WAVE_BLUE.gamma_multiply(0.4)),
                );
                let mut x = x0.max(wave.left());
                while x < x1.min(wave.right()) {
                    let sa = s_at(x, self) as usize;
                    let sb = (s_at(x + 1.0, self) as usize).max(sa + 1);
                    let (lo, hi) = wave::range_minmax(&samples, &peaks, sa, sb);
                    let in_sel = selected && self.sel.is_some_and(|(a, b)| sa >= a && sa < b);
                    wp.line_segment(
                        [
                            pos2(x, mid - shape(hi) * half),
                            pos2(x, mid - shape(lo) * half + 0.5),
                        ],
                        Stroke::new(1.0, if in_sel { WAVE_SEL } else { WAVE_BLUE }),
                    );
                    x += 1.0;
                }
                wp.rect_stroke(
                    body,
                    4.0,
                    Stroke::new(1.0, theme::BORDER),
                    StrokeKind::Inside,
                );
            }
            // Gravando por cima desta faixa: o áudio novo aparece ao vivo.
            if let Some(rt) = rec_now.filter(|rt| rt.track == Some(*id)) {
                self.paint_recording(&wp, wave, rt.at, wave_left);
            }
            // Painel de controle da faixa.
            self.track_controls(
                ui,
                r,
                lanes,
                *id,
                name,
                (*gain, *pan, *mute, *solo),
                selected,
            );
        }
        // Faixa nova sendo gravada.
        if new_rec_lane && let Some(rt) = rec_now {
            let r = row_rect(tracks.len(), self);
            let wave = Rect::from_min_max(pos2(wave_left, r.top()), r.max);
            lp.rect_filled(wave, 0.0, TRACK_BG.gamma_multiply(1.25));
            lp.rect_filled(
                Rect::from_min_max(r.min, pos2(wave_left - 1.0, r.bottom())),
                0.0,
                theme::BAR,
            );
            lp.text(
                r.left_top() + vec2(10.0, 10.0),
                Align2::LEFT_TOP,
                "Gravando…",
                theme::text(13.0),
                Color32::from_rgb(240, 100, 90),
            );
            let wp = lp.with_clip_rect(wave.intersect(lanes));
            self.paint_recording(&wp, wave, rt.at, wave_left);
        }
        // Fantasma do clipe sendo arrastado.
        if let Some((row, start, len)) = move_ghost {
            let r = row_rect(row, self);
            let g = Rect::from_min_max(
                pos2(x_of(start as f64, self), r.top() + 1.0),
                pos2(x_of((start + len) as f64, self), r.bottom() - 1.0),
            );
            lp.rect_filled(g, 4.0, theme::ACCENT.gamma_multiply(0.2));
            lp.rect_stroke(g, 4.0, Stroke::new(1.5, theme::ACCENT), StrokeKind::Inside);
        }
        if tracks.is_empty() && !new_rec_lane {
            lp.text(
                wave_area.center(),
                Align2::CENTER_CENTER,
                "Aperte R (ou o botão vermelho) para gravar, ou arraste um áudio para cá",
                theme::text(15.0),
                theme::TEXT_DIM,
            );
        }
        // Cursor / posição.
        let head = self.position();
        let hx = x_of(head as f64, self);
        if hx >= wave_left && hx <= area.right() {
            let color = if self.recorder.is_recording() {
                Color32::from_rgb(240, 80, 70)
            } else if self.is_playing() {
                Color32::from_rgb(110, 210, 120)
            } else {
                Color32::from_gray(230)
            };
            painter.line_segment(
                [pos2(hx, ruler.top()), pos2(hx, area.bottom())],
                Stroke::new(1.0, color),
            );
            if self.is_playing() || self.recorder.is_recording() {
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        pos2(hx - 6.0, ruler.top()),
                        pos2(hx + 6.0, ruler.top()),
                        pos2(hx, ruler.top() + 9.0),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
        }
        ui.advance_cursor_after_rect(area);
    }

    /// Forma de onda ao vivo do que está sendo gravado, começando em `at`.
    fn paint_recording(&mut self, painter: &egui::Painter, wave: Rect, at: usize, wave_left: f32) {
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
        let x_of = |s: f64| wave_left + ((s - self.view_start) / self.spp) as f32;
        let len = self.rec_peaks.len() * wave::BUCKET;
        let body = Rect::from_min_max(
            pos2(x_of(at as f64), wave.top() + 1.0),
            pos2(x_of((at + len) as f64), wave.bottom() - 1.0),
        );
        painter.rect_filled(body, 4.0, Color32::from_rgb(0x45, 0x26, 0x2a));
        let mid = body.center().y;
        let half = body.height() / 2.0 - 4.0;
        let per_px = (self.spp / wave::BUCKET as f64).max(1.0) as usize;
        let n = self.rec_peaks.len();
        let mut x = body.left().max(wave.left());
        while x < body.right().min(wave.right()) {
            let s = (((x - body.left()) as f64 * self.spp) as usize / wave::BUCKET).min(n);
            let (lo, hi) = self.rec_peaks[s..(s + per_px).min(n)]
                .iter()
                .fold((0.0f32, 0.0f32), |(l, h), (a, b)| (l.min(*a), h.max(*b)));
            painter.line_segment(
                [
                    pos2(x, mid - shape(hi) * half),
                    pos2(x, mid - shape(lo) * half + 0.5),
                ],
                Stroke::new(1.0, Color32::from_rgb(240, 110, 100)),
            );
            x += 1.0;
        }
        painter.ctx().request_repaint();
    }

    /// Painel de controle da faixa (nome, Mudo/Solo, ganho e pan), como o do Audacity.
    #[allow(clippy::too_many_arguments)]
    fn track_controls(
        &mut self,
        ui: &mut egui::Ui,
        r: Rect,
        lanes: Rect,
        id: u64,
        name: &str,
        mix: (f32, f32, bool, bool),
        selected: bool,
    ) {
        let tcp = Rect::from_min_max(r.min, pos2(r.left() + TCP_W - 1.0, r.bottom()));
        let visible = tcp.intersect(lanes);
        if visible.height() < 4.0 {
            return;
        }
        // Fundo (clique seleciona a faixa; Ctrl/Shift soma ou tira).
        let bg = ui.interact(visible, ui.id().with(("tcp", id)), Sense::click());
        if bg.clicked() {
            if ui.input(|i| i.modifiers.command || i.modifiers.shift) {
                if let Some(pos) = self.selected.iter().position(|t| *t == id) {
                    self.selected.remove(pos);
                } else {
                    self.selected.push(id);
                }
            } else {
                self.selected = vec![id];
            }
        }
        let painter = ui.painter_at(visible);
        painter.rect_filled(
            tcp,
            0.0,
            if selected {
                Color32::from_rgb(0x34, 0x34, 0x3c)
            } else {
                theme::BAR
            },
        );
        if selected {
            painter.rect_filled(
                Rect::from_min_size(tcp.min, vec2(3.0, tcp.height())),
                0.0,
                theme::ACCENT,
            );
        }
        let (gain, pan, mute, solo) = mix;
        let (mut g, mut p, mut m, mut s) = mix;
        let mut remove = false;
        ui.scope_builder(UiBuilder::new().max_rect(tcp.shrink(6.0)), |ui| {
            ui.set_clip_rect(visible);
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::new("×").small().frame(false))
                    .on_hover_text("Apagar a faixa")
                    .clicked()
                {
                    remove = true;
                }
                if let Some((rid, text)) = &mut self.renaming
                    && *rid == id
                {
                    let rr = ui.add(egui::TextEdit::singleline(text).desired_width(130.0));
                    rr.request_focus();
                    if rr.lost_focus() {
                        let (rid, text) = self.renaming.take().expect("renomeando");
                        let _ = self.ed.rename_track(rid, text.trim().to_string());
                    }
                } else {
                    let nr = ui
                        .add(
                            egui::Label::new(RichText::new(name).strong().color(Color32::WHITE))
                                .truncate()
                                .sense(Sense::click()),
                        )
                        .on_hover_text("Duplo clique para renomear");
                    if nr.double_clicked() {
                        self.renaming = Some((id, name.to_string()));
                    }
                }
            });
            ui.horizontal(|ui| {
                let toggle = |ui: &mut egui::Ui, text: &str, on: bool, color: Color32| {
                    ui.add(
                        egui::Button::new(RichText::new(text).color(if on {
                            Color32::BLACK
                        } else {
                            theme::TEXT
                        }))
                        .fill(if on { color } else { theme::BAR_LIGHT })
                        .min_size(vec2(76.0, 22.0)),
                    )
                    .clicked()
                };
                if toggle(ui, "Mudo", m, Color32::from_rgb(230, 80, 70)) {
                    m = !m;
                }
                if toggle(ui, "Solo", s, Color32::from_rgb(235, 200, 70)) {
                    s = !s;
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Ganho").small().color(theme::TEXT_DIM));
                let r = ui
                    .add(egui::Slider::new(&mut g, -36.0..=36.0).show_value(false))
                    .on_hover_text(format!("{g:+.1} dB (duplo clique zera)"));
                if r.double_clicked() {
                    g = 0.0;
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Pan   ").small().color(theme::TEXT_DIM));
                let side = if p.abs() < 0.01 {
                    "centro".to_string()
                } else if p < 0.0 {
                    format!("{:.0}% esquerda", -p * 100.0)
                } else {
                    format!("{:.0}% direita", p * 100.0)
                };
                let r = ui
                    .add(egui::Slider::new(&mut p, -1.0..=1.0).show_value(false))
                    .on_hover_text(format!("{side} (duplo clique centraliza)"));
                if r.double_clicked() {
                    p = 0.0;
                }
            });
        });
        if (g, p, m, s) != (gain, pan, mute, solo) {
            self.ed.set_mix(id, g, p, m, s);
        }
        if remove {
            self.stop();
            let _ = self.ed.remove_track(id);
            self.selected.retain(|t| *t != id);
        }
    }

    // ---------------------------------------------------------------- embaixo

    /// Barra de seleção do Audacity: taxa, início/fim editáveis e a posição do áudio.
    fn selection_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Taxa do projeto")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            ui.label(RichText::new(format!("{RATE} Hz")).font(theme::mono(12.0)));
            ui.add_space(18.0);
            ui.label(
                RichText::new("Início e fim da seleção")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            let (a, b) = self.sel.unwrap_or((self.cursor, self.cursor));
            for (k, value) in [a, b].into_iter().enumerate() {
                let mut t = self.sel_edit[k]
                    .clone()
                    .unwrap_or_else(|| format_time(value, RATE));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut t)
                        .desired_width(96.0)
                        .font(theme::mono(13.0)),
                );
                if r.has_focus() {
                    self.sel_edit[k] = Some(t.clone());
                }
                if r.lost_focus() {
                    if let Some(v) = parse_time(&t, RATE) {
                        let (mut na, mut nb) = (a, b);
                        if k == 0 {
                            na = v;
                        } else {
                            nb = v;
                        }
                        if nb > na {
                            self.sel = Some((na, nb));
                        } else {
                            self.sel = None;
                            self.cursor = na;
                        }
                    }
                    self.sel_edit[k] = None;
                }
            }
            ui.add_space(18.0);
            ui.label(
                RichText::new("Posição do áudio")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            ui.label(
                RichText::new(format_time(self.position(), RATE))
                    .font(theme::mono(18.0))
                    .color(theme::TIMECODE),
            );
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            let state = if self.recorder.is_recording() {
                "Gravando."
            } else if self.is_playing() {
                "Tocando."
            } else {
                "Parado."
            };
            ui.label(RichText::new(state).small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                if !self
                    .player
                    .has_device
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    ui.label(
                        RichText::new("sem saída de áudio")
                            .small()
                            .color(Color32::from_rgb(240, 150, 90)),
                    );
                }
                ui.label(
                    RichText::new(format!(
                        "{} faixa(s) · {}",
                        self.ed.session().tracks.len(),
                        format_time(self.ed.session().len(), RATE)
                    ))
                    .small()
                    .color(theme::TEXT_DIM),
                );
            });
        });
    }

    fn script_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(theme::PAD);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Roteiro").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("×")
                    .on_hover_text("Esconder (Ver → Roteiro)")
                    .clicked()
                {
                    self.show_script = false;
                }
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
            let mut area = egui::ScrollArea::vertical().auto_shrink(false);
            if let Some(elapsed) = self.rec_since.map(|t| t.elapsed().as_secs_f32())
                && secs > 0.0
            {
                area = area.vertical_scroll_offset((elapsed / secs * self.prompt_height).max(0.0));
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

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let mut apply = false;
        let mut close = false;
        let title = match dialog {
            Dialog::Gain(_) => "Amplificar",
            Dialog::Gate { .. } => "Noise gate",
            Dialog::Compressor(_) => "Compressor",
            Dialog::Silences { .. } => "Truncar silêncio",
            Dialog::ConfirmClose => "Sair sem salvar?",
            Dialog::Recover(_) => "Recuperação automática",
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                match dialog {
                    Dialog::Gain(db) => {
                        ui.add(
                            egui::Slider::new(db, -24.0..=24.0)
                                .text("amplificação")
                                .suffix(" dB"),
                        );
                    }
                    Dialog::Gate {
                        threshold,
                        reduction,
                    } => {
                        ui.label(
                            RichText::new(
                                "Abaixa o som onde ele fica abaixo do limiar (as pausas).",
                            )
                            .weak(),
                        );
                        ui.add(
                            egui::Slider::new(threshold, -80.0..=-10.0)
                                .text("limiar")
                                .suffix(" dB"),
                        );
                        ui.add(
                            egui::Slider::new(reduction, 3.0..=40.0)
                                .text("redução")
                                .suffix(" dB"),
                        );
                    }
                    Dialog::Compressor(c) => {
                        ui.label(RichText::new("Deixa a voz mais uniforme.").weak());
                        ui.horizontal(|ui| {
                            for k in [Compression::Light, Compression::Medium, Compression::Strong]
                            {
                                ui.selectable_value(c, k, k.label());
                            }
                        });
                    }
                    Dialog::Silences { threshold, min } => {
                        ui.label(
                            RichText::new("Corta as pausas longas (sobra um respiro de 0,12 s).")
                                .weak(),
                        );
                        ui.add(
                            egui::Slider::new(threshold, -70.0..=-20.0)
                                .text("abaixo de")
                                .suffix(" dB"),
                        );
                        ui.add(
                            egui::Slider::new(min, 0.2..=3.0)
                                .text("por pelo menos")
                                .suffix(" s"),
                        );
                    }
                    Dialog::ConfirmClose => {
                        ui.label("O projeto tem mudanças que não foram salvas.");
                    }
                    Dialog::Recover(files) => {
                        ui.label(format!(
                            "{} gravação(ões) não terminou(aram) direito da última vez.",
                            files.len()
                        ));
                        ui.label(RichText::new("Dá para recuperar como faixas.").weak());
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
            Some(Dialog::Gain(db)) => self.apply_effect("Amplificar", |s| dsp::gain(s, db)),
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
                    if let Ok(s) = read_raw_f32(&f)
                        && !s.is_empty()
                    {
                        let id = self.ed.add_track(s, 0, Some("Recuperada".into()));
                        self.selected = vec![id];
                    }
                    let _ = std::fs::remove_file(&f);
                }
                self.fit();
                self.say("Gravação recuperada. Salve o projeto (Ctrl+S).");
            }
            None => {}
        }
    }

    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll_file();
        self.poll_imports();
        if let Some(e) = self.recorder.take_error() {
            self.rec = None;
            self.rec_since = None;
            self.warn(e);
        }
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
        self.follow();
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
        egui::Panel::top("toolbar")
            .exact_size(56.0)
            .frame(bar(theme::BAR))
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status")
            .exact_size(22.0)
            .frame(bar(theme::BG_DARK))
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("selection")
            .exact_size(40.0)
            .frame(bar(theme::BAR))
            .show(ui, |ui| self.selection_bar(ui));
        if self.show_script {
            egui::Panel::right("script")
                .default_size(340.0)
                .resizable(true)
                .frame(
                    egui::Frame::new()
                        .fill(theme::BG)
                        .inner_margin(egui::Margin::same(8)),
                )
                .show(ui, |ui| self.script_panel(ui));
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.tracks_panel(ui));
        self.dialogs(&ctx);
        self.toasts.tick(&ctx);
        self.toasts.ui(&ctx);
        if self.is_playing() || self.recorder.is_recording() {
            ctx.request_repaint();
        }
    }

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

    fn app_with(samples: Vec<f32>) -> (VoiceApp, u64) {
        let mut app = VoiceApp::empty();
        let id = app.ed.add_track(samples, 0, None);
        app.selected = vec![id];
        (app, id)
    }

    #[test]
    fn effects_act_on_the_selection_or_everything() {
        let (mut app, id) = app_with(vec![0.1; 1000]);
        app.sel = Some((100, 200));
        app.apply_effect("Ganho", |s| dsp::gain(s, 20.0));
        let out = app.ed.render(id);
        assert!((out[150] - 1.0).abs() < 1e-4 && (out[50] - 0.1).abs() < 1e-6);
        app.select_none();
        app.apply_effect("Ganho", |s| dsp::gain(s, -20.0));
        assert!((app.ed.render(id)[50] - 0.01).abs() < 1e-6);
        app.selected = vec![id];
        app.sel = Some((0, 500));
        app.delete_selection();
        assert_eq!(app.ed.render(id).len(), 500);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn cut_and_paste_like_audacity() {
        let (mut app, id) = app_with((1..=100).map(|i| i as f32).collect());
        app.sel = Some((10, 20));
        app.cut();
        assert_eq!(app.ed.render(id).len(), 90);
        assert_eq!((app.cursor, app.sel), (10, None));
        app.cursor = 0;
        app.paste();
        assert_eq!(app.ed.render(id)[0..2], [11.0, 12.0]);
        assert_eq!(app.sel, Some((0, 10)), "o que foi colado fica selecionado");
    }

    #[test]
    fn remove_silences_cuts_long_pauses() {
        let tone: Vec<f32> = (0..RATE).map(|i| 0.3 * (i as f32 * 0.05).sin()).collect();
        let mut s = tone.clone();
        s.extend(vec![0.0; RATE as usize * 2]);
        s.extend(tone);
        let (mut app, id) = app_with(s);
        app.remove_silences(-40.0, 0.6);
        let secs = app.ed.render(id).len() as f32 / RATE as f32;
        assert!((secs - 2.24).abs() < 0.1, "{secs}");
    }
}
