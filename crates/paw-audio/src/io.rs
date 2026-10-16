//! Tocar e gravar pelo PulseAudio, cada um na sua thread.
//!
//! Os buffers são pedidos pequenos (10 ms na gravação, 50 ms na reprodução): o padrão do
//! PulseAudio chega a 2 s, o que atrasava o começo da gravação e perdia o fim ao parar.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use libpulse_binding::def::BufferAttr;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;

use crate::mix::{BufferSource, MixSource, crossfade};

/// Quadros por pedaço lido/escrito.
const CHUNK: usize = 480;

fn spec(rate: u32, channels: u8) -> Spec {
    Spec {
        format: Format::F32le,
        channels,
        rate,
    }
}

/// Bytes de `ms` milissegundos.
fn bytes_for(ms: u32, rate: u32, channels: u8) -> u32 {
    rate * ms / 1000 * 4 * channels as u32
}

struct PlayState {
    source: Option<Arc<dyn MixSource>>,
    start: usize,
    end: usize,
    generation: u64,
}

/// Toca um trecho `[from, to)` pedindo o som aos poucos a uma `MixSource` (um buffer pronto ou
/// uma mixagem ao vivo). As posições são em quadros. Trocar a fonte durante o play
/// (`update_live`) não reinicia nada: o som antigo passa para o novo em um crossfade curto.
pub struct Player {
    channels: usize,
    state: Arc<Mutex<PlayState>>,
    playing: Arc<AtomicBool>,
    position: Arc<AtomicUsize>,
    /// Pico recente de cada canal (bits de f32), para o medidor de saída.
    levels: Arc<[AtomicU32; 2]>,
    /// Desligado quando o `Player` é descartado: a thread de reprodução termina e solta a saída.
    alive: Arc<AtomicBool>,
    pub has_device: Arc<AtomicBool>,
}

impl Player {
    pub fn spawn(rate: u32, channels: u8) -> Self {
        Self::spawn_on(rate, channels, None)
    }

    /// Como `spawn`, mas tocando na saída `device` (nome do PulseAudio; `None` = padrão).
    pub fn spawn_on(rate: u32, channels: u8, device: Option<String>) -> Self {
        let state = Arc::new(Mutex::new(PlayState {
            source: None,
            start: 0,
            end: 0,
            generation: 0,
        }));
        let playing = Arc::new(AtomicBool::new(false));
        let position = Arc::new(AtomicUsize::new(0));
        let has_device = Arc::new(AtomicBool::new(false));
        let levels = Arc::new([AtomicU32::new(0), AtomicU32::new(0)]);
        let alive = Arc::new(AtomicBool::new(true));
        let (st, pl, pos, dev, lv, al) = (
            state.clone(),
            playing.clone(),
            position.clone(),
            has_device.clone(),
            levels.clone(),
            alive.clone(),
        );
        let ch = channels as usize;
        thread::Builder::new()
            .name("paw-play".into())
            .spawn(move || {
                let attr = BufferAttr {
                    maxlength: u32::MAX,
                    tlength: bytes_for(60, rate, channels),
                    prebuf: u32::MAX,
                    minreq: u32::MAX,
                    fragsize: u32::MAX,
                };
                let device = Simple::new(
                    None,
                    "Paw",
                    Direction::Playback,
                    device.as_deref(),
                    "Reprodução",
                    &spec(rate, channels),
                    None,
                    Some(&attr),
                )
                .ok();
                dev.store(device.is_some(), Ordering::SeqCst);
                let chunk_time = Duration::from_secs_f64(CHUNK as f64 / rate as f64);
                let mut current = u64::MAX;
                let mut cursor = 0usize;
                // Fonte que gerou o último bloco: se mudou sem reinício, o bloco novo entra em crossfade.
                let mut previous: Option<Arc<dyn MixSource>> = None;
                while al.load(Ordering::SeqCst) {
                    if !pl.load(Ordering::SeqCst) {
                        if current != u64::MAX {
                            if let Some(d) = &device {
                                let _ = d.flush();
                            }
                            current = u64::MAX;
                        }
                        previous = None;
                        for l in lv.iter() {
                            l.store(0f32.to_bits(), Ordering::Relaxed);
                        }
                        thread::sleep(Duration::from_millis(3));
                        continue;
                    }
                    let (source, start, end, generation) = {
                        let s = st.lock().expect("estado do player");
                        (s.source.clone(), s.start, s.end, s.generation)
                    };
                    let Some(source) = source else {
                        pl.store(false, Ordering::SeqCst);
                        continue;
                    };
                    let restarted = generation != current;
                    if restarted {
                        if let Some(d) = &device {
                            let _ = d.flush();
                        }
                        current = generation;
                        cursor = start;
                    }
                    if cursor >= end {
                        // Espera o que está no buffer da placa terminar de tocar.
                        if let Some(d) = &device {
                            let _ = d.drain();
                        }
                        // Se outro play começou durante a espera, ele manda: não o apague.
                        if st.lock().expect("estado do player").generation == current {
                            pos.store(end, Ordering::SeqCst);
                            pl.store(false, Ordering::SeqCst);
                        }
                        continue;
                    }
                    let to = (cursor + CHUNK).min(end);
                    let n = to - cursor;
                    let mut samples = source.mix(cursor, n);
                    if !restarted
                        && let Some(old) = previous.as_ref().filter(|p| !Arc::ptr_eq(p, &source))
                    {
                        let before = old.mix(cursor, n);
                        crossfade(&before, &mut samples, ch);
                    }
                    previous = Some(source);
                    for (c, l) in lv.iter().enumerate() {
                        let peak = samples
                            .iter()
                            .skip(c.min(ch - 1))
                            .step_by(ch)
                            .fold(0.0f32, |m, v| m.max(v.abs()));
                        let old = f32::from_bits(l.load(Ordering::Relaxed));
                        l.store(peak.max(old * 0.85).to_bits(), Ordering::Relaxed);
                    }
                    cursor = to;
                    let bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
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
            channels: ch,
            state,
            playing,
            position,
            levels,
            alive,
            has_device,
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Toca os quadros `[from, to)` de `buffer` (intercalado).
    pub fn play(&self, buffer: Arc<Vec<f32>>, from: usize, to: usize) {
        let source = Arc::new(BufferSource {
            buffer,
            channels: self.channels,
        });
        self.play_live(source, from, to);
    }

    /// Toca os quadros `[from, to)` pedindo o som a `source` (posições da timeline).
    pub fn play_live(&self, source: Arc<dyn MixSource>, from: usize, to: usize) {
        let mut s = self.state.lock().expect("estado do player");
        s.source = Some(source);
        s.start = from;
        s.end = to;
        s.generation += 1;
        self.position.store(from, Ordering::SeqCst);
        self.playing.store(true, Ordering::SeqCst);
    }

    /// Troca a fonte no meio do play (edição): continua do mesmo ponto, sem esvaziar a fila da
    /// placa nem repetir trechos.
    pub fn update_live(&self, source: Arc<dyn MixSource>) {
        self.state.lock().expect("estado do player").source = Some(source);
    }

    pub fn stop(&self) {
        self.playing.store(false, Ordering::SeqCst);
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::SeqCst)
    }

    /// Quadro saindo da caixa de som agora.
    pub fn position(&self) -> usize {
        self.position.load(Ordering::SeqCst)
    }

    /// Pico recente (0..1) da saída, de cada lado; zera quando para.
    pub fn levels(&self) -> [f32; 2] {
        [
            f32::from_bits(self.levels[0].load(Ordering::Relaxed)),
            f32::from_bits(self.levels[1].load(Ordering::Relaxed)),
        ]
    }

    /// Geração atual do play (cada `play` soma um; `update_live` não mexe). Para testes.
    pub fn generation(&self) -> u64 {
        self.state.lock().expect("estado do player").generation
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Trocar a saída cria outro `Player`; sem isto a thread antiga ficava acordando de 3 em
        // 3 ms para sempre e segurando a conexão com o som.
        self.alive.store(false, Ordering::SeqCst);
    }
}

/// Grava do microfone padrão em mono. Enquanto grava, também despeja num arquivo (`spill`)
/// para não perder nada se o programa fechar no meio.
pub struct Recorder {
    rate: u32,
    channels: u8,
    device: Option<String>,
    running: Arc<AtomicBool>,
    /// Só monitorando (medidor), sem guardar o áudio.
    monitor: Arc<AtomicBool>,
    buffer: Arc<Mutex<Vec<f32>>>,
    levels: Arc<[AtomicU32; 2]>,
    error: Arc<Mutex<Option<String>>>,
    handle: Option<JoinHandle<()>>,
}

impl Recorder {
    pub fn new(rate: u32) -> Self {
        Self {
            rate,
            channels: 1,
            device: None,
            running: Arc::new(AtomicBool::new(false)),
            monitor: Arc::new(AtomicBool::new(false)),
            buffer: Arc::new(Mutex::new(Vec::new())),
            levels: Arc::new([AtomicU32::new(0), AtomicU32::new(0)]),
            error: Arc::new(Mutex::new(None)),
            handle: None,
        }
    }

    /// Canais (1 ou 2) e microfone (`None` = padrão) das próximas gravações.
    pub fn configure(&mut self, channels: u8, device: Option<String>) {
        let changed = (channels.clamp(1, 2), &device) != (self.channels, &self.device);
        if changed {
            let monitoring = self.is_monitoring();
            self.stop_stream();
            self.channels = channels.clamp(1, 2);
            self.device = device;
            if monitoring {
                self.start_monitor();
            }
        }
    }

    pub fn channels(&self) -> u8 {
        self.channels
    }

    /// Gravando de verdade (não só monitorando).
    pub fn is_recording(&self) -> bool {
        self.running.load(Ordering::SeqCst) && !self.monitor.load(Ordering::SeqCst)
    }

    pub fn is_monitoring(&self) -> bool {
        self.running.load(Ordering::SeqCst) && self.monitor.load(Ordering::SeqCst)
    }

    /// Liga o medidor do microfone sem gravar (o "monitorar" do Audacity).
    pub fn start_monitor(&mut self) {
        if self.running.load(Ordering::SeqCst) {
            return;
        }
        self.monitor.store(true, Ordering::SeqCst);
        self.spawn(None);
    }

    pub fn stop_monitor(&mut self) {
        if self.is_monitoring() {
            self.stop_stream();
        }
    }

    fn stop_stream(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        for l in self.levels.iter() {
            l.store(0f32.to_bits(), Ordering::Relaxed);
        }
    }

    /// Começa a gravar. `spill`: arquivo f32 cru onde cada pedaço também é gravado.
    /// Se estava monitorando, a gravação continua do mesmo fluxo (sem atraso).
    pub fn start(&mut self, spill: Option<PathBuf>) {
        if self.is_recording() {
            return;
        }
        if self.is_monitoring() {
            self.stop_stream();
        }
        self.monitor.store(false, Ordering::SeqCst);
        self.spawn(spill);
    }

    fn spawn(&mut self, spill: Option<PathBuf>) {
        self.buffer.lock().expect("buffer").clear();
        *self.error.lock().expect("erro") = None;
        self.running.store(true, Ordering::SeqCst);
        let (run, buf, lvl, err, rate, mon) = (
            self.running.clone(),
            self.buffer.clone(),
            self.levels.clone(),
            self.error.clone(),
            self.rate,
            self.monitor.clone(),
        );
        let (channels, dev) = (self.channels, self.device.clone());
        self.handle = Some(
            thread::Builder::new()
                .name("paw-record".into())
                .spawn(move || {
                    // Fragmentos de 10 ms: o áudio chega quase na hora (o padrão é até 2 s).
                    let attr = BufferAttr {
                        maxlength: u32::MAX,
                        tlength: u32::MAX,
                        prebuf: u32::MAX,
                        minreq: u32::MAX,
                        fragsize: bytes_for(10, rate, channels),
                    };
                    let device = match Simple::new(
                        None,
                        "Paw",
                        Direction::Record,
                        dev.as_deref(),
                        "Gravação",
                        &spec(rate, channels),
                        None,
                        Some(&attr),
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
                    let mut take = |bytes: &[u8]| {
                        let samples: Vec<f32> = bytes
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|b| f32::from_le_bytes(*b))
                            .collect();
                        let ch = channels as usize;
                        for (c, l) in lvl.iter().enumerate() {
                            let peak = samples
                                .iter()
                                .skip(c.min(ch - 1))
                                .step_by(ch)
                                .fold(0.0f32, |m, v| m.max(v.abs()));
                            let old = f32::from_bits(l.load(Ordering::Relaxed));
                            l.store(peak.max(old * 0.85).to_bits(), Ordering::Relaxed);
                        }
                        if mon.load(Ordering::SeqCst) {
                            return;
                        }
                        if let Some(f) = file.as_mut() {
                            let _ = f.write_all(bytes);
                        }
                        buf.lock().expect("buffer").extend_from_slice(&samples);
                    };
                    let mut bytes = vec![0u8; CHUNK * 4 * channels as usize];
                    while run.load(Ordering::SeqCst) {
                        if let Err(e) = device.read(&mut bytes) {
                            *err.lock().expect("erro") = Some(format!("Erro ao gravar: {e}"));
                            break;
                        }
                        take(&bytes);
                    }
                    // Ao parar, lê o que já foi captado e ainda estava no buffer: senão o
                    // fim da gravação some.
                    let pending = device
                        .get_latency()
                        .map(|l| (l.0 as u128 * rate as u128 / 1_000_000) as usize)
                        .unwrap_or(0)
                        .min(rate as usize / 2);
                    if pending > 0 && !mon.load(Ordering::SeqCst) {
                        let mut rest = vec![0u8; pending * 4 * channels as usize];
                        if device.read(&mut rest).is_ok() {
                            take(&rest);
                        }
                    }
                    if let Some(mut f) = file {
                        let _ = f.flush();
                    }
                    run.store(false, Ordering::SeqCst);
                })
                .expect("não foi possível criar a thread de gravação"),
        );
    }

    /// Para e devolve o que foi gravado, um vetor por canal.
    pub fn stop(&mut self) -> Vec<Vec<f32>> {
        self.stop_stream();
        let data = std::mem::take(&mut *self.buffer.lock().expect("buffer"));
        let ch = self.channels.max(1) as usize;
        (0..ch)
            .map(|c| data.iter().skip(c).step_by(ch).copied().collect())
            .collect()
    }

    /// Quantos quadros já foram gravados.
    pub fn len(&self) -> usize {
        self.buffer.lock().map_or(0, |b| b.len()) / self.channels.max(1) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Lê o que já foi gravado, intercalado (para desenhar a forma de onda ao vivo).
    pub fn with_buffer<R>(&self, f: impl FnOnce(&[f32]) -> R) -> R {
        let b = self.buffer.lock().expect("buffer");
        f(&b)
    }

    /// Pico recente do microfone (0..1) de cada lado, para o medidor (mono: os dois iguais).
    pub fn levels(&self) -> [f32; 2] {
        [
            f32::from_bits(self.levels[0].load(Ordering::Relaxed)),
            f32::from_bits(self.levels[1].load(Ordering::Relaxed)),
        ]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// Fonte que só conta quantas vezes foi chamada.
    struct Counting(Arc<AtomicUsize>);

    impl MixSource for Counting {
        fn mix(&self, _from: usize, frames: usize) -> Vec<f32> {
            self.0.fetch_add(1, Ordering::SeqCst);
            vec![0.0; frames * 2]
        }
    }

    fn wait_until(what: &str, cond: impl Fn() -> bool) {
        let start = std::time::Instant::now();
        while !cond() {
            assert!(start.elapsed() < Duration::from_secs(5), "{what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn swapping_the_live_source_keeps_playing_without_restarting() {
        let player = Player::spawn(48_000, 2);
        let (a, b) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        player.play_live(Arc::new(Counting(a.clone())), 1_000, 10_000_000);
        wait_until("a fonte antiga nunca tocou", || {
            a.load(Ordering::SeqCst) > 3
        });
        let (generation, at) = (player.generation(), player.position());
        // Vários ajustes seguidos (arrastar um slider) não podem reiniciar a reprodução.
        for _ in 0..5 {
            player.update_live(Arc::new(Counting(b.clone())));
        }
        wait_until("a fonte nova nunca tocou", || b.load(Ordering::SeqCst) > 3);
        assert_eq!(player.generation(), generation, "não pode reiniciar");
        assert!(player.position() >= at, "a posição não pode voltar");
        let before = a.load(Ordering::SeqCst);
        thread::sleep(Duration::from_millis(60));
        assert!(
            a.load(Ordering::SeqCst) <= before + 1,
            "a fonte antiga continuou tocando"
        );
        player.stop();
    }

    #[test]
    fn dropping_the_player_ends_its_thread() {
        let player = Player::spawn(48_000, 2);
        let state = Arc::downgrade(&player.state);
        drop(player);
        wait_until("a thread de reprodução não terminou", || {
            state.upgrade().is_none()
        });
    }

    #[test]
    fn a_play_stops_by_itself_at_the_end_of_the_range() {
        let player = Player::spawn(48_000, 2);
        let n = Arc::new(AtomicUsize::new(0));
        player.play_live(Arc::new(Counting(n.clone())), 0, 4_800);
        wait_until("o play não terminou sozinho", || !player.is_playing());
        assert_eq!(player.position(), 4_800);
        // Tocar de novo depois do fim funciona.
        player.play_live(Arc::new(Counting(n)), 0, 4_800);
        wait_until("o segundo play não terminou", || !player.is_playing());
    }
}
