//! Trabalho fora da thread da interface: render do preview, áudio da reprodução,
//! waveforms e exportação.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
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

pub struct Rendered {
    pub slot: ViewSlot,
    pub image: RgbaImage,
}

/// Thread que renderiza os viewers. Pedidos acumulados são descartados: só o mais novo de cada
/// viewer é atendido.
pub struct PreviewWorker {
    tx: Sender<Job>,
    pub results: Receiver<Rendered>,
}

impl PreviewWorker {
    pub fn spawn(fit: (u32, u32), sample_rate: u32, repaint: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = channel::<Job>();
        let (out_tx, results) = channel();
        thread::Builder::new()
            .name("pawcut-preview".into())
            .spawn(move || {
                let mut renderer =
                    Renderer::new(MediaLibrary::new(fit, sample_rate, default_cache_dir()));
                while let Ok(first) = rx.recv() {
                    let mut latest: HashMap<ViewSlot, Job> = HashMap::new();
                    latest.insert(first.slot, first);
                    while let Ok(more) = rx.try_recv() {
                        latest.insert(more.slot, more);
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
        let _ = self.tx.send(Job {
            slot,
            request,
            size,
        });
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
}

/// Reprodução de áudio pelo PulseAudio. A posição tocada é o relógio mestre do preview.
pub struct AudioPlayer {
    state: Arc<Mutex<AudioState>>,
    playing: Arc<AtomicBool>,
    position: Arc<AtomicI64>,
    generation: Arc<AtomicU64>,
    pub has_device: Arc<AtomicBool>,
}

const AUDIO_CHUNK: usize = 1024;

impl AudioPlayer {
    pub fn spawn(sample_rate: u32) -> Self {
        let state = Arc::new(Mutex::new(AudioState {
            project: None,
            start_sample: 0,
            generation: 0,
        }));
        let playing = Arc::new(AtomicBool::new(false));
        let position = Arc::new(AtomicI64::new(0));
        let generation = Arc::new(AtomicU64::new(0));
        let has_device = Arc::new(AtomicBool::new(false));
        let (st, pl, pos, dev) = (
            state.clone(),
            playing.clone(),
            position.clone(),
            has_device.clone(),
        );
        thread::Builder::new()
            .name("pawcut-audio".into())
            .spawn(move || audio_loop(sample_rate, st, pl, pos, dev))
            .expect("não foi possível criar a thread de áudio");
        Self {
            state,
            playing,
            position,
            generation,
            has_device,
        }
    }

    /// Começa a tocar `project` a partir da amostra `start`.
    pub fn play(&self, project: Arc<Project>, start: i64) {
        let g = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut s = self.state.lock().expect("estado de áudio");
        s.project = Some(project);
        s.start_sample = start;
        s.generation = g;
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
        if !playing.load(Ordering::SeqCst) {
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
        pos += AUDIO_CHUNK as i64;
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
        let total = project.duration();
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
