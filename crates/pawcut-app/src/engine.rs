//! Trabalho fora da thread da interface: render do preview, áudio da reprodução,
//! waveforms e exportação.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use paw_media::{MediaLibrary, RgbaImage, default_cache_dir, load_audio, peaks};
use pawcut_core::{Frame, MediaKind, Project};
use pawcut_export::{ExportError, ExportSettings, Stage, export};
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

    /// Troca o projeto sem reiniciar (edição durante a reprodução). Não mexe na geração: o laço
    /// de áudio continua do mesmo ponto, sem esvaziar o buffer da placa (isso repetia trechos e
    /// picotava o som a cada ajuste de volume) e faz um crossfade curto entre os dois projetos.
    pub fn update_project(&self, project: Arc<Project>) {
        if let Ok(mut s) = self.state.lock() {
            s.warm = Some(project.clone());
            s.project = Some(project);
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

/// Decodifica o áudio dos clipes numa thread à parte. O laço de áudio nunca decodifica nada:
/// um clipe novo (colado, trocado de velocidade) fica mudo por um instante até chegar, em vez
/// de travar o som de todos os outros.
struct AudioLoader {
    tx: Sender<(PathBuf, f64)>,
    rx: Receiver<(PathBuf, f64, Option<Arc<paw_media::Pcm>>)>,
    /// Pedidos em andamento ou que falharam (esses só voltam a ser tentados depois de `forget`).
    asked: std::collections::HashSet<(PathBuf, u64)>,
}

impl AudioLoader {
    fn spawn(sample_rate: u32) -> Self {
        let (tx, jobs) = channel::<(PathBuf, f64)>();
        let (out, rx) = channel();
        thread::Builder::new()
            .name("pawcut-audio-load".into())
            .spawn(move || {
                let cache = default_cache_dir();
                while let Ok((path, speed)) = jobs.recv() {
                    let pcm = load_audio(&path, sample_rate, speed, cache.as_deref())
                        .ok()
                        .map(Arc::new);
                    if out.send((path, speed, pcm)).is_err() {
                        break;
                    }
                }
            })
            .expect("não foi possível criar a thread de decodificação de áudio");
        Self {
            tx,
            rx,
            asked: Default::default(),
        }
    }

    /// Pede o áudio de tudo que o projeto usa e ainda não está na memória.
    fn request_missing(&mut self, project: &Project, media: &mut MediaLibrary) {
        for clip in project.audio_tracks.iter().flat_map(|t| t.clips.iter()) {
            let pawcut_core::ClipSource::Media(id) = &clip.source else {
                continue;
            };
            let Some(m) = project.media(*id) else {
                continue;
            };
            let key = (m.info.path.clone(), clip.speed.to_bits());
            if !self.asked.contains(&key)
                && media.resident_audio(&m.info.path, clip.speed).is_none()
            {
                self.asked.insert(key);
                let _ = self.tx.send((m.info.path.clone(), clip.speed));
            }
        }
    }

    /// Entrega ao player o que ficou pronto.
    fn deliver(&mut self, media: &mut MediaLibrary) {
        while let Ok((path, speed, pcm)) = self.rx.try_recv() {
            if let Some(pcm) = pcm {
                media.insert_audio(&path, speed, pcm);
                self.asked.remove(&(path, speed.to_bits()));
            }
        }
    }

    fn forget(&mut self, path: &Path) {
        self.asked.retain(|(p, _)| p != path);
    }
}

/// Mistura linear de `before` (some) com `after` (aparece) ao longo do trecho, in loco em `after`.
/// Os dois são estéreo intercalado e têm o mesmo tamanho.
fn crossfade(before: &[f32], after: &mut [f32]) {
    let frames = after.len() / 2;
    for (i, (b, a)) in before.chunks(2).zip(after.chunks_mut(2)).enumerate() {
        let t = (i as f32 + 0.5) / frames as f32;
        for (b, a) in b.iter().zip(a.iter_mut()) {
            *a = *b * (1.0 - t) + *a * t;
        }
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
    use libpulse_binding::def::BufferAttr;
    use libpulse_binding::sample::{Format, Spec};
    use libpulse_binding::stream::Direction;
    use libpulse_simple_binding::Simple;

    let spec = Spec {
        format: Format::F32le,
        channels: 2,
        rate: sample_rate,
    };
    // Buffer curto (~100 ms): uma edição feita durante o play é ouvida quase na hora, sem
    // precisar esvaziar a fila da placa. O padrão do PulseAudio guarda ~250 ms.
    let attr = BufferAttr {
        maxlength: u32::MAX,
        tlength: (sample_rate as u64 * 100 / 1000) as u32 * 2 * 4,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize: u32::MAX,
    };
    let device = Simple::new(
        None,
        "PawCut",
        Direction::Playback,
        None,
        "Reprodução",
        &spec,
        None,
        Some(&attr),
    )
    .ok();
    has_device.store(device.is_some(), Ordering::SeqCst);
    let mut renderer = Renderer::new(MediaLibrary::new((2, 2), sample_rate, default_cache_dir()));
    let mut loader = AudioLoader::spawn(sample_rate);
    let mut last_check = std::time::Instant::now();
    let mut current_gen = u64::MAX;
    // Projeto que gerou o último trecho: se mudou sem reinício, o trecho novo entra em crossfade.
    let mut previous: Option<Arc<Project>> = None;
    let mut pos = 0i64;
    let chunk_time = Duration::from_secs_f64(AUDIO_CHUNK as f64 / sample_rate as f64);
    loop {
        let (forget, warm) = {
            let mut s = state.lock().expect("estado de áudio");
            (std::mem::take(&mut s.forget), s.warm.take())
        };
        for path in forget {
            renderer.media().forget(&path);
            loader.forget(&path);
        }
        if let Some(project) = warm {
            loader.request_missing(&project, renderer.media());
        }
        loader.deliver(renderer.media());
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
            previous = None;
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
        let restarted = generation != current_gen;
        if restarted {
            if let Some(d) = &device {
                let _ = d.flush();
            }
            current_gen = generation;
            pos = start;
            // Play sem preparo (o projeto mudou e o play veio logo em seguida): pede já o que falta.
            loader.request_missing(&project, renderer.media());
        }
        // De vez em quando confere se algo saiu da memória (orçamento de áudio) e pede de novo.
        if last_check.elapsed() > Duration::from_secs(1) {
            last_check = std::time::Instant::now();
            loader.request_missing(&project, renderer.media());
        }
        let mut samples = renderer.mix_audio_resident(&project, pos, AUDIO_CHUNK);
        if !restarted && let Some(old) = previous.as_ref().filter(|p| !Arc::ptr_eq(p, &project)) {
            // Edição durante o play (volume, corte, mudo...): passa do som antigo para o novo
            // ao longo do trecho, em vez de dar um degrau (estalo).
            let before = renderer.mix_audio_resident(old, pos, AUDIO_CHUNK);
            crossfade(&before, &mut samples);
        }
        previous = Some(project);
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
    /// Ligado enquanto o áudio está sendo mixado (antes dos frames).
    pub mixing_audio: Arc<AtomicBool>,
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
        let mixing_audio = Arc::new(AtomicBool::new(false));
        let (d, c, r, m) = (
            done.clone(),
            cancel.clone(),
            result.clone(),
            mixing_audio.clone(),
        );
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
                    m.store(p.stage == Stage::Audio, Ordering::Relaxed);
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
            mixing_audio,
            result,
        }
    }

    pub fn finished(&self) -> Option<Result<(), String>> {
        self.result.lock().ok()?.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pawcut_core::ProjectSettings;

    #[test]
    fn crossfade_goes_from_old_to_new() {
        let before = vec![1.0f32; 8];
        let mut after = vec![0.0f32; 8];
        crossfade(&before, &mut after);
        // Começa quase todo no som antigo, termina quase todo no novo, sem degrau.
        assert!(after[0] > 0.8 && after[7] < 0.2);
        assert!(after.windows(2).all(|w| w[0] >= w[1]));
    }

    #[test]
    fn a_new_clip_is_silent_until_the_loader_delivers_it_and_never_blocks_the_mix() {
        let dir = tempfile::tempdir().unwrap();
        let wav = paw_testkit::dc_audio(dir.path(), "a.wav", 0.5, 1.0);
        let settings = ProjectSettings::default();
        let mut st = crate::state::AppState::new(Project::new(settings));
        let info = paw_media::probe(&wav, ProjectSettings::default().fps).unwrap();
        let m = st
            .apply(pawcut_core::Edit::AddMedia(info))
            .unwrap()
            .media
            .unwrap();
        st.drop_media(m, pawcut_core::TrackKind::Audio, 0, 0);
        let project = st.project().clone();
        assert!(!project.audio_tracks[0].clips.is_empty());
        let mut renderer = Renderer::new(MediaLibrary::new((2, 2), 48_000, None));
        let mut loader = AudioLoader::spawn(48_000);
        loader.request_missing(&project, renderer.media());
        let silent = renderer.mix_audio_resident(&project, 1000, 64);
        assert!(silent.iter().all(|v| *v == 0.0), "ainda não chegou");
        let start = std::time::Instant::now();
        loop {
            loader.deliver(renderer.media());
            let out = renderer.mix_audio_resident(&project, 1000, 64);
            if out.iter().any(|v| *v != 0.0) {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "nunca chegou");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn editing_during_play_does_not_restart_the_audio() {
        let p = Arc::new(Project::new(ProjectSettings::default()));
        let player = AudioPlayer::spawn(48_000);
        player.play(p.clone(), 4_800);
        let g = player.generation.load(Ordering::SeqCst);
        // Arrastar o volume manda um projeto novo por frame; nenhum pode reiniciar a reprodução.
        for _ in 0..10 {
            player.update_project(Arc::new(Project::new(ProjectSettings::default())));
        }
        assert_eq!(player.generation.load(Ordering::SeqCst), g);
        let s = player.state.lock().unwrap();
        assert_eq!(s.generation, g);
        assert_eq!(s.start_sample, 4_800);
    }
}
