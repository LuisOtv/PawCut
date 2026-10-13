//! Tocar e gravar pelo PulseAudio, em mono, cada um na sua thread.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;

const CHUNK: usize = 1024;

fn spec(rate: u32) -> Spec {
    Spec {
        format: Format::F32le,
        channels: 1,
        rate,
    }
}

struct PlayState {
    buffer: Option<Arc<Vec<f32>>>,
    start: usize,
    end: usize,
    generation: u64,
}

/// Toca um trecho de uma trilha mono. A posição tocada é o relógio da interface.
pub struct Player {
    state: Arc<Mutex<PlayState>>,
    playing: Arc<AtomicBool>,
    position: Arc<AtomicUsize>,
    pub has_device: Arc<AtomicBool>,
}

impl Player {
    pub fn spawn(rate: u32) -> Self {
        let state = Arc::new(Mutex::new(PlayState {
            buffer: None,
            start: 0,
            end: 0,
            generation: 0,
        }));
        let playing = Arc::new(AtomicBool::new(false));
        let position = Arc::new(AtomicUsize::new(0));
        let has_device = Arc::new(AtomicBool::new(false));
        let (st, pl, pos, dev) = (
            state.clone(),
            playing.clone(),
            position.clone(),
            has_device.clone(),
        );
        thread::Builder::new()
            .name("paw-play".into())
            .spawn(move || {
                let device = Simple::new(
                    None,
                    "Paw",
                    Direction::Playback,
                    None,
                    "Reprodução",
                    &spec(rate),
                    None,
                    None,
                )
                .ok();
                dev.store(device.is_some(), Ordering::SeqCst);
                let chunk_time = Duration::from_secs_f64(CHUNK as f64 / rate as f64);
                let mut current = u64::MAX;
                let mut cursor = 0usize;
                loop {
                    if !pl.load(Ordering::SeqCst) {
                        if current != u64::MAX {
                            if let Some(d) = &device {
                                let _ = d.flush();
                            }
                            current = u64::MAX;
                        }
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    let (buf, start, end, generation) = {
                        let s = st.lock().expect("estado do player");
                        (s.buffer.clone(), s.start, s.end, s.generation)
                    };
                    let Some(buf) = buf else {
                        pl.store(false, Ordering::SeqCst);
                        continue;
                    };
                    if generation != current {
                        if let Some(d) = &device {
                            let _ = d.flush();
                        }
                        current = generation;
                        cursor = start;
                    }
                    let end = end.min(buf.len());
                    if cursor >= end {
                        // Espera o que está no buffer da placa terminar de tocar.
                        if let Some(d) = &device {
                            let _ = d.drain();
                        }
                        pos.store(end, Ordering::SeqCst);
                        pl.store(false, Ordering::SeqCst);
                        continue;
                    }
                    let to = (cursor + CHUNK).min(end);
                    let bytes: Vec<u8> = buf[cursor..to]
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect();
                    cursor = to;
                    match &device {
                        Some(d) => {
                            let _ = d.write(&bytes);
                            let latency = d
                                .get_latency()
                                .map(|l| (l.0 as u128 * rate as u128 / 1_000_000) as usize)
                                .unwrap_or(0);
                            pos.store(cursor.saturating_sub(latency).max(start), Ordering::SeqCst);
                        }
                        None => {
                            thread::sleep(chunk_time);
                            pos.store(cursor, Ordering::SeqCst);
                        }
                    }
                }
            })
            .expect("não foi possível criar a thread de reprodução");
        Self {
            state,
            playing,
            position,
            has_device,
        }
    }

    /// Toca `buffer[from..to]`.
    pub fn play(&self, buffer: Arc<Vec<f32>>, from: usize, to: usize) {
        let mut s = self.state.lock().expect("estado do player");
        s.buffer = Some(buffer);
        s.start = from;
        s.end = to;
        s.generation += 1;
        self.position.store(from, Ordering::SeqCst);
        self.playing.store(true, Ordering::SeqCst);
    }

    pub fn stop(&self) {
        self.playing.store(false, Ordering::SeqCst);
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::SeqCst)
    }

    /// Amostra saindo da caixa de som agora.
    pub fn position(&self) -> usize {
        self.position.load(Ordering::SeqCst)
    }
}

/// Grava do microfone padrão em mono. Enquanto grava, também despeja num arquivo (`spill`)
/// para não perder nada se o programa fechar no meio.
pub struct Recorder {
    rate: u32,
    running: Arc<AtomicBool>,
    buffer: Arc<Mutex<Vec<f32>>>,
    level: Arc<AtomicU32>,
    error: Arc<Mutex<Option<String>>>,
    handle: Option<JoinHandle<()>>,
}

impl Recorder {
    pub fn new(rate: u32) -> Self {
        Self {
            rate,
            running: Arc::new(AtomicBool::new(false)),
            buffer: Arc::new(Mutex::new(Vec::new())),
            level: Arc::new(AtomicU32::new(0)),
            error: Arc::new(Mutex::new(None)),
            handle: None,
        }
    }

    pub fn is_recording(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Começa a gravar. `spill`: arquivo f32 cru onde cada pedaço também é gravado.
    pub fn start(&mut self, spill: Option<PathBuf>) {
        if self.is_recording() {
            return;
        }
        self.buffer.lock().expect("buffer").clear();
        *self.error.lock().expect("erro") = None;
        self.running.store(true, Ordering::SeqCst);
        let (run, buf, lvl, err, rate) = (
            self.running.clone(),
            self.buffer.clone(),
            self.level.clone(),
            self.error.clone(),
            self.rate,
        );
        self.handle = Some(
            thread::Builder::new()
                .name("paw-record".into())
                .spawn(move || {
                    let device = match Simple::new(
                        None,
                        "Paw",
                        Direction::Record,
                        None,
                        "Gravação",
                        &spec(rate),
                        None,
                        None,
                    ) {
                        Ok(d) => d,
                        Err(e) => {
                            *err.lock().expect("erro") =
                                Some(format!("Não foi possível abrir o microfone: {e}"));
                            run.store(false, Ordering::SeqCst);
                            return;
                        }
                    };
                    let mut file = spill.and_then(|p| File::create(p).ok()).map(BufWriter::new);
                    let mut bytes = vec![0u8; CHUNK * 4];
                    while run.load(Ordering::SeqCst) {
                        if let Err(e) = device.read(&mut bytes) {
                            *err.lock().expect("erro") = Some(format!("Erro ao gravar: {e}"));
                            break;
                        }
                        let samples: Vec<f32> = bytes
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|b| f32::from_le_bytes(*b))
                            .collect();
                        let peak = samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                        let old = f32::from_bits(lvl.load(Ordering::Relaxed));
                        lvl.store(peak.max(old * 0.8).to_bits(), Ordering::Relaxed);
                        if let Some(f) = file.as_mut() {
                            let _ = f.write_all(&bytes);
                        }
                        buf.lock().expect("buffer").extend_from_slice(&samples);
                    }
                    if let Some(mut f) = file {
                        let _ = f.flush();
                    }
                    run.store(false, Ordering::SeqCst);
                })
                .expect("não foi possível criar a thread de gravação"),
        );
    }

    /// Para e devolve tudo que foi gravado.
    pub fn stop(&mut self) -> Vec<f32> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.level.store(0f32.to_bits(), Ordering::Relaxed);
        std::mem::take(&mut *self.buffer.lock().expect("buffer"))
    }

    /// Quantas amostras já foram gravadas.
    pub fn len(&self) -> usize {
        self.buffer.lock().map_or(0, |b| b.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Lê o que já foi gravado (para desenhar a forma de onda ao vivo).
    pub fn with_buffer<R>(&self, f: impl FnOnce(&[f32]) -> R) -> R {
        let b = self.buffer.lock().expect("buffer");
        f(&b)
    }

    /// Pico recente do microfone (0..1), para o medidor.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// Erro da última gravação (microfone não abriu etc.).
    pub fn take_error(&self) -> Option<String> {
        self.error.lock().ok()?.take()
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}
