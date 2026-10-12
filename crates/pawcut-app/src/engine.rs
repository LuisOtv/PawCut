//! Trabalho fora da thread da interface: render do preview, áudio da reprodução,
//! waveforms e exportação.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use pawcut_core::{Frame, MediaKind, Project};
use pawcut_export::{ExportError, ExportSettings, export};
use pawcut_media::{MediaLibrary, RgbaImage, default_cache_dir, load_audio, peaks};
use pawcut_render::Renderer;

/// O que mostrar num viewer.
#[derive(Clone)]
pub enum ViewRequest {
    Timeline {
        project: Arc<Project>,
        frame: Frame,
    },
    Source {
        path: PathBuf,
        kind: MediaKind,
        seconds: f64,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ViewSlot {
    Source,
    Timeline,
}

struct Job {
    slot: ViewSlot,
    request: ViewRequest,
    size: (u32, u32),
}

enum Msg {
    Render(Job),
    /// O arquivo mudou (voltou, sumiu, foi reconectado): esquecer o que está em cache.
    Forget(PathBuf),
}

pub struct Rendered {
    pub slot: ViewSlot,
    pub image: RgbaImage,
}

/// Thread que renderiza os viewers. Pedidos acumulados são descartados: só o mais novo de cada
/// viewer é atendido.
pub struct PreviewWorker {
    tx: Sender<Msg>,
    pub results: Receiver<Rendered>,
}

impl PreviewWorker {
    pub fn spawn(fit: (u32, u32), sample_rate: u32, repaint: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = channel::<Msg>();
        let (out_tx, results) = channel();
        thread::Builder::new()
            .name("pawcut-preview".into())
            .spawn(move || {
                let mut renderer = {
                    let mut lib = MediaLibrary::new(fit, sample_rate, default_cache_dir());
                    // O preview usa a mídia otimizada (proxy) quando existir; a exportação nunca.
                    lib.set_use_proxies(true);
                    Renderer::new(lib)
                };
                while let Ok(first) = rx.recv() {
                    let mut latest: HashMap<ViewSlot, Job> = HashMap::new();
                    for msg in std::iter::once(first).chain(rx.try_iter()) {
                        match msg {
                            Msg::Render(job) => {
                                latest.insert(job.slot, job);
                            }
                            Msg::Forget(path) => renderer.media().forget(&path),
                        }
                    }
                    for (_, job) in latest {
                        let image = render(&mut renderer, &job);
                        if out_tx
                            .send(Rendered {
                                slot: job.slot,
                                image,
                            })
                            .is_err()
                        {
                            return;
                        }
                        repaint();
                    }
                }
            })
            .expect("não foi possível criar a thread de preview");
        Self { tx, results }
    }

    pub fn request(&self, slot: ViewSlot, request: ViewRequest, size: (u32, u32)) {
        let _ = self.tx.send(Msg::Render(Job {
            slot,
            request,
            size,
        }));
    }

    pub fn forget(&self, path: &Path) {
        let _ = self.tx.send(Msg::Forget(path.to_path_buf()));
    }
}

fn render(renderer: &mut Renderer, job: &Job) -> RgbaImage {
    match &job.request {
        ViewRequest::Timeline { project, frame } => {
            renderer
                .render_frame(project, *frame, job.size.0.max(2), job.size.1.max(2))
                .image
        }
        ViewRequest::Source {
            path,
            kind,
            seconds,
        } => {
            let media = renderer.media();
            let img = match kind {
                MediaKind::Image => media.image(path).ok(),
                MediaKind::Video { .. } => media.video_frame(path, *seconds).ok(),
                MediaKind::Audio => None,
            };
            img.map(|i| i.as_ref().clone())
                .unwrap_or_else(|| RgbaImage::from_pixel(16, 9, image::Rgba([25, 25, 28, 255])))
        }
    }
}

// ---------------------------------------------------------------------------------------------

struct AudioState {
    project: Option<Arc<Project>>,
    start_sample: i64,
    generation: u64,
    /// Projeto cujo áudio deve ser decodificado com antecedência (antes do play).
    warm: Option<Arc<Project>>,
    forget: Vec<PathBuf>,
}

/// Reprodução de áudio pelo PulseAudio. A posição tocada é o relógio mestre do preview.
pub struct AudioPlayer {
    /// Pico recente de cada canal (bits de f32), para os medidores.
    levels: Arc<[AtomicU32; 2]>,
    state: Arc<Mutex<AudioState>>,
    playing: Arc<AtomicBool>,
    position: Arc<AtomicI64>,
    generation: Arc<AtomicU64>,
    /// Amostra em que a reprodução para sozinha (scrub); `i64::MAX` = não para.
    stop_at: Arc<AtomicI64>,
    pub has_device: Arc<AtomicBool>,
}

const AUDIO_CHUNK: usize = 1024;

impl AudioPlayer {
    pub fn spawn(sample_rate: u32) -> Self {
        let state = Arc::new(Mutex::new(AudioState {
            project: None,
            start_sample: 0,
            generation: 0,
            warm: None,
            forget: Vec::new(),
        }));
        let playing = Arc::new(AtomicBool::new(false));
        let position = Arc::new(AtomicI64::new(0));
        let generation = Arc::new(AtomicU64::new(0));
        let has_device = Arc::new(AtomicBool::new(false));
        let levels = Arc::new([AtomicU32::new(0), AtomicU32::new(0)]);
        let lv = levels.clone();
        let stop_at = Arc::new(AtomicI64::new(i64::MAX));
        let sa = stop_at.clone();
        let (st, pl, pos, dev) = (
            state.clone(),
            playing.clone(),
            position.clone(),
            has_device.clone(),
        );
        thread::Builder::new()
            .name("pawcut-audio".into())
            .spawn(move || audio_loop(sample_rate, st, pl, pos, dev, lv, sa))
            .expect("não foi possível criar a thread de áudio");
        Self {
            levels,
            state,
            playing,
            position,
            generation,
            stop_at,
            has_device,
        }
    }

    /// Toca só `len` amostras a partir de `start` (scrub com som).
    pub fn scrub(&self, project: Arc<Project>, start: i64, len: i64) {
        self.play(project, start);
        self.stop_at.store(start + len, Ordering::SeqCst);
    }

    /// Começa a tocar `project` a partir da amostra `start`.
    pub fn play(&self, project: Arc<Project>, start: i64) {
        let g = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut s = self.state.lock().expect("estado de áudio");
        s.project = Some(project);
        s.start_sample = start;
        s.generation = g;
        self.stop_at.store(i64::MAX, Ordering::SeqCst);
        self.position.store(start, Ordering::SeqCst);
        self.playing.store(true, Ordering::SeqCst);
    }

    /// Troca o projeto sem reiniciar (edição durante a reprodução).
    pub fn update_project(&self, project: Arc<Project>) {
        if let Ok(mut s) = self.state.lock() {
            let resume = self.position.load(Ordering::SeqCst);
            s.project = Some(project);
            s.start_sample = resume;
            s.generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        }
    }

    /// Decodifica em segundo plano o áudio que o projeto usa, para o play começar na hora.
    pub fn prepare(&self, project: Arc<Project>) {
        if let Ok(mut s) = self.state.lock() {
            s.warm = Some(project);
        }
    }

    pub fn forget(&self, path: &Path) {
        if let Ok(mut s) = self.state.lock() {
            s.forget.push(path.to_path_buf());
        }
    }

    /// Pico recente (0..1) de L e R; zera quando para de tocar.
    pub fn levels(&self) -> [f32; 2] {
        [
            f32::from_bits(self.levels[0].load(Ordering::Relaxed)),
            f32::from_bits(self.levels[1].load(Ordering::Relaxed)),
        ]
    }

    pub fn stop(&self) {
        self.playing.store(false, Ordering::SeqCst);
    }

    /// Amostra que está saindo da caixa de som agora.
    pub fn position(&self) -> i64 {
        self.position.load(Ordering::SeqCst)
    }
}

fn audio_loop(
    sample_rate: u32,
    state: Arc<Mutex<AudioState>>,
    playing: Arc<AtomicBool>,
    position: Arc<AtomicI64>,
    has_device: Arc<AtomicBool>,
    levels: Arc<[AtomicU32; 2]>,
    stop_at: Arc<AtomicI64>,
) {
    use libpulse_binding::sample::{Format, Spec};
    use libpulse_binding::stream::Direction;
    use libpulse_simple_binding::Simple;

    let spec = Spec {
        format: Format::F32le,
        channels: 2,
        rate: sample_rate,
    };
    let device = Simple::new(
        None,
        "PawCut",
        Direction::Playback,
        None,
        "Reprodução",
        &spec,
        None,
        None,
    )
    .ok();
    has_device.store(device.is_some(), Ordering::SeqCst);
    let mut renderer = Renderer::new(MediaLibrary::new((2, 2), sample_rate, default_cache_dir()));
    let mut current_gen = u64::MAX;
    let mut pos = 0i64;
    let chunk_time = Duration::from_secs_f64(AUDIO_CHUNK as f64 / sample_rate as f64);
    loop {
        let (forget, warm) = {
            let mut s = state.lock().expect("estado de áudio");
            let warm = if playing.load(Ordering::SeqCst) {
                None
            } else {
                s.warm.take()
            };
            (std::mem::take(&mut s.forget), warm)
        };
        for path in forget {
            renderer.media().forget(&path);
        }
        if let Some(project) = warm {
            for clip in project.audio_tracks.iter().flat_map(|t| t.clips.iter()) {
                if let pawcut_core::ClipSource::Media(id) = &clip.source
                    && let Some(m) = project.media(*id)
                {
                    let _ = renderer.media().audio(&m.info.path, clip.speed);
                }
            }
        }
        if !playing.load(Ordering::SeqCst) {
            for l in levels.iter() {
                l.store(0f32.to_bits(), Ordering::Relaxed);
            }
            if current_gen != u64::MAX {
                if let Some(d) = &device {
                    let _ = d.flush();
                }
                current_gen = u64::MAX;
            }
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        let (project, start, generation) = {
            let s = state.lock().expect("estado de áudio");
            (s.project.clone(), s.start_sample, s.generation)
        };
        let Some(project) = project else {
            thread::sleep(Duration::from_millis(5));
            continue;
        };
        if generation != current_gen {
            if let Some(d) = &device {
                let _ = d.flush();
            }
            current_gen = generation;
            pos = start;
        }
        let samples = renderer.mix_audio(&project, pos, AUDIO_CHUNK);
        for (ch, l) in levels.iter().enumerate() {
            let peak = samples
                .iter()
                .skip(ch)
                .step_by(2)
                .fold(0.0f32, |m, v| m.max(v.abs()));
            // Sobe na hora, desce devagar (como um medidor de verdade).
            let old = f32::from_bits(l.load(Ordering::Relaxed));
            l.store(peak.max(old * 0.85).to_bits(), Ordering::Relaxed);
        }
        pos += AUDIO_CHUNK as i64;
        if pos >= stop_at.load(Ordering::SeqCst) {
            playing.store(false, Ordering::SeqCst);
        }
        match &device {
            Some(d) => {
                let bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
                if d.write(&bytes).is_err() {
                    thread::sleep(chunk_time);
                }
                let latency = d
                    .get_latency()
                    .map(|l| l.0 as i64 * sample_rate as i64 / 1_000_000)
                    .unwrap_or(0);
                position.store((pos - latency).max(start), Ordering::SeqCst);
            }
            None => {
                // Sem caixa de som: mantém o relógio andando no tempo real.
                thread::sleep(chunk_time);
                position.store(pos, Ordering::SeqCst);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------

/// Miniaturas de vídeo em tempos específicos (tira de frames nos clipes), em segundo plano.
/// O tempo é arredondado para meio segundo para reaproveitar o cache.
pub struct FrameThumbs {
    tx: Sender<(PathBuf, i64)>,
    rx: Receiver<((PathBuf, i64), Option<RgbaImage>)>,
    pending: std::collections::HashSet<(PathBuf, i64)>,
    done: Vec<((PathBuf, i64), Option<RgbaImage>)>,
}

impl FrameThumbs {
    pub fn spawn(repaint: impl Fn() + Send + 'static) -> Self {
        let (tx, jobs) = channel::<(PathBuf, i64)>();
        let (out, rx) = channel();
        thread::Builder::new()
            .name("pawcut-thumbs".into())
            .spawn(move || {
                let mut lib = MediaLibrary::new((192, 108), 48_000, default_cache_dir());
                lib.set_use_proxies(true);
                while let Ok(key) = jobs.recv() {
                    let img = lib
                        .video_frame(&key.0, key.1 as f64 / 2.0)
                        .ok()
                        .map(|i| i.as_ref().clone());
                    if out.send((key, img)).is_err() {
                        break;
                    }
                    repaint();
                }
            })
            .expect("não foi possível criar a thread de miniaturas");
        Self {
            tx,
            rx,
            pending: Default::default(),
            done: Vec::new(),
        }
    }

    pub fn key(path: &Path, seconds: f64) -> (PathBuf, i64) {
        (path.to_path_buf(), (seconds * 2.0).round().max(0.0) as i64)
    }

    /// Pede a miniatura (uma vez só).
    pub fn request(&mut self, key: (PathBuf, i64)) {
        if self.pending.insert(key.clone()) {
            let _ = self.tx.send(key);
        }
    }

    /// Resultados que chegaram desde a última chamada.
    pub fn take_done(&mut self) -> Vec<((PathBuf, i64), Option<RgbaImage>)> {
        while let Ok(r) = self.rx.try_recv() {
            self.done.push(r);
        }
        std::mem::take(&mut self.done)
    }

    pub fn forget(&mut self, path: &Path) {
        self.pending.retain(|(p, _)| p != path);
    }
}

/// Picos de áudio (100 por segundo) de cada arquivo, calculados em segundo plano.
/// Picos (mín, máx) de um arquivo.
pub type Peaks = Arc<Vec<(f32, f32)>>;

pub struct Waveforms {
    /// `None` enquanto calcula.
    ready: Arc<Mutex<HashMap<PathBuf, Option<Peaks>>>>,
}

pub const PEAKS_PER_SECOND: f64 = 100.0;

impl Waveforms {
    pub fn new() -> Self {
        Self {
            ready: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Picos do arquivo (na velocidade 1x) se já estiverem prontos; senão começa a calcular.
    pub fn get(
        &self,
        path: &Path,
        repaint: impl Fn() + Send + 'static,
    ) -> Option<Arc<Vec<(f32, f32)>>> {
        let mut map = self.ready.lock().ok()?;
        if let Some(entry) = map.get(path) {
            return entry.clone();
        }
        map.insert(path.to_path_buf(), None);
        let ready = self.ready.clone();
        let path = path.to_path_buf();
        thread::spawn(move || {
            if let Ok(pcm) = load_audio(&path, 48_000, 1.0, default_cache_dir().as_deref()) {
                let buckets = (pcm.frames() as f64 / 48_000.0 * PEAKS_PER_SECOND).ceil() as usize;
                let p = Arc::new(peaks(&pcm, buckets.max(1)));
                if let Ok(mut m) = ready.lock() {
                    m.insert(path, Some(p));
                }
                repaint();
            }
        });
        None
    }
}

// ---------------------------------------------------------------------------------------------

/// Exportação em andamento.
pub struct ExportJob {
    pub done: Arc<AtomicI64>,
    pub total: Frame,
    pub cancel: Arc<AtomicBool>,
    pub result: Arc<Mutex<Option<Result<(), String>>>>,
}

impl ExportJob {
    pub fn start(
        project: Project,
        settings: ExportSettings,
        repaint: impl Fn() + Send + 'static,
    ) -> Self {
        let done = Arc::new(AtomicI64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let result = Arc::new(Mutex::new(None));
        let total = pawcut_export::frame_count(&project, &settings);
        let (d, c, r) = (done.clone(), cancel.clone(), result.clone());
        thread::Builder::new()
            .name("pawcut-export".into())
            .spawn(move || {
                let s = &project.settings;
                let mut renderer = Renderer::new(MediaLibrary::new(
                    (s.width, s.height),
                    s.sample_rate,
                    default_cache_dir(),
                ));
                let out = export(&project, &mut renderer, &settings, &c, |p| {
                    d.store(p.done, Ordering::Relaxed);
                    if p.done % 15 == 0 {
                        repaint();
                    }
                });
                let msg = match out {
                    Ok(()) => Ok(()),
                    Err(ExportError::Cancelled) => Err("Exportação cancelada".to_string()),
                    Err(e) => Err(e.to_string()),
                };
                if let Ok(mut slot) = r.lock() {
                    *slot = Some(msg);
                }
                repaint();
            })
            .expect("não foi possível criar a thread de exportação");
        Self {
            done,
            total,
            cancel,
            result,
        }
    }

    pub fn finished(&self) -> Option<Result<(), String>> {
        self.result.lock().ok()?.clone()
    }
}
