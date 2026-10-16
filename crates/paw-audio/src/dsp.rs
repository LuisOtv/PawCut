//! Processamento de áudio: loudness (EBU R128), pico, silêncios, filtros e dinâmica.

/// Filtro biquad (forma direta I).
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    fn run(&mut self, v: f64) -> f64 {
        let out = self.b[0] * v + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [v, self.x[0]];
        self.y = [out, self.y[0]];
        out
    }
}

/// Ponderação K da ITU-R BS.1770 (shelf de agudos + passa-altas) para a taxa `fs`.
fn k_weighting(fs: f64) -> [Biquad; 2] {
    use std::f64::consts::PI;
    let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = (PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        x: [0.0; 2],
        y: [0.0; 2],
    };
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = (PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let high_pass = Biquad {
        b: [1.0, -2.0, 1.0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        x: [0.0; 2],
        y: [0.0; 2],
    };
    [shelf, high_pass]
}

/// Loudness integrado (LUFS, EBU R128) de áudio estéreo intercalado: blocos de 400 ms
/// a cada 100 ms, porta absoluta em −70 LUFS e relativa 10 LU abaixo.
pub fn integrated_lufs(samples: &[f32], sample_rate: u32) -> Option<f32> {
    let frames = samples.len() / 2;
    let mut filters = [
        k_weighting(sample_rate as f64),
        k_weighting(sample_rate as f64),
    ];
    // Energia ponderada de cada janela de 100 ms (somando os dois canais).
    let hop = (sample_rate as usize / 10).max(1);
    let mut hops = Vec::with_capacity(frames / hop + 1);
    let mut acc = 0.0f64;
    for i in 0..frames {
        for (ch, f) in filters.iter_mut().enumerate() {
            let mut v = samples[2 * i + ch] as f64;
            for stage in f.iter_mut() {
                v = stage.run(v);
            }
            acc += v * v;
        }
        if (i + 1) % hop == 0 {
            hops.push(acc / hop as f64);
            acc = 0.0;
        }
    }
    let blocks: Vec<f64> = hops
        .windows(4)
        .map(|w| w.iter().sum::<f64>() / 4.0)
        .collect();
    let loudness = |z: f64| -0.691 + 10.0 * z.log10();
    let above = |gate: f64| -> Vec<f64> {
        blocks
            .iter()
            .copied()
            .filter(|z| *z > 0.0 && loudness(*z) > gate)
            .collect()
    };
    let abs = above(-70.0);
    if abs.is_empty() {
        return None;
    }
    let rel = loudness(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let gated = above(rel);
    let z = gated.iter().sum::<f64>() / gated.len().max(1) as f64;
    Some(loudness(z) as f32)
}

/// Ganho (dB) para chegar em `target` LUFS sem o pico passar de −1 dBFS.
pub fn loudness_gain(lufs: f32, peak: f32, target: f32) -> f32 {
    let want = target - lufs;
    let headroom = if peak > 1e-6 {
        -1.0 - 20.0 * peak.log10()
    } else {
        24.0
    };
    want.min(headroom).clamp(-96.0, 24.0)
}

/// Volume (dB) que leva o pico do clipe a `target_db` (ex.: −1 dBFS).
pub fn normalize_gain(peak: f32, target_db: f32) -> Option<f32> {
    (peak > 1e-6).then(|| (target_db - 20.0 * peak.log10()).clamp(-96.0, 24.0))
}

/// Trechos de silêncio de `samples` (estéreo intercalado) em índices de frame estéreo:
/// abaixo de `threshold_db` por pelo menos `min_len`, encolhidos `pad` em cada ponta.
pub fn silent_runs(
    samples: &[f32],
    sample_rate: u32,
    threshold_db: f32,
    min_len: f64,
    pad: f64,
) -> Vec<(usize, usize)> {
    let window = (sample_rate as usize / 100).max(1);
    let frames = samples.len() / 2;
    let threshold = 10f32.powf(threshold_db / 20.0);
    let mut runs = Vec::new();
    let mut run_start: Option<usize> = None;
    let mut w = 0;
    while w < frames {
        let end = (w + window).min(frames);
        let energy: f32 = (w..end)
            .map(|i| {
                (samples[2 * i] * samples[2 * i] + samples[2 * i + 1] * samples[2 * i + 1]) / 2.0
            })
            .sum();
        let rms = (energy / (end - w) as f32).sqrt();
        match (rms < threshold, run_start) {
            (true, None) => run_start = Some(w),
            (false, Some(s)) => {
                runs.push((s, w));
                run_start = None;
            }
            _ => {}
        }
        w = end;
    }
    if let Some(s) = run_start {
        runs.push((s, frames));
    }
    let min = (min_len * sample_rate as f64) as usize;
    let pad = (pad * sample_rate as f64) as usize;
    runs.into_iter()
        .filter(|(a, b)| b - a >= min)
        .map(|(a, b)| {
            (
                a + if a == 0 { 0 } else { pad },
                b - if b == frames { 0 } else { pad },
            )
        })
        .filter(|(a, b)| a < b)
        .collect()
}

#[cfg(test)]
mod tests {

    fn sine(amp: f32, secs: f32) -> Vec<f32> {
        let sr = 48_000.0;
        (0..(sr * secs) as usize)
            .flat_map(|i| {
                let v = amp * (std::f32::consts::TAU * 1000.0 * i as f32 / sr).sin();
                [v, v]
            })
            .collect()
    }

    #[test]
    fn lufs_of_a_reference_sine() {
        // Seno de 1 kHz a −23 dBFS nos dois canais mede −23 LUFS (EBU R128).
        let amp = 10f32.powf(-23.0 / 20.0);
        let l = integrated_lufs(&sine(amp, 3.0), 48_000).unwrap();
        assert!((l + 23.0).abs() < 0.3, "{l}");
        // Silêncio fica abaixo da porta.
        assert_eq!(integrated_lufs(&vec![0.0; 96_000], 48_000), None);
        // O ganho respeita o pico: −23 → −14 LUFS pediria +9 dB, mas o pico −23 dBFS deixa +22.
        assert!((loudness_gain(-23.0, amp, -14.0) - 9.0).abs() < 1e-4);
        assert!((loudness_gain(-23.0, 0.9, -14.0) - (-1.0 - 20.0 * 0.9f32.log10())).abs() < 1e-4);
    }

    use super::*;

    fn tone(secs: f64, level: f32, sr: u32) -> Vec<f32> {
        (0..(secs * sr as f64) as usize)
            .flat_map(|i| {
                let v = level * (i as f32 * 0.07).sin();
                [v, v]
            })
            .collect()
    }

    #[test]
    fn finds_long_silences_and_keeps_short_pauses() {
        let sr = 48_000;
        let mut s = tone(1.0, 0.5, sr);
        s.extend(tone(1.0, 0.0, sr)); // silêncio de 1 s
        s.extend(tone(0.5, 0.5, sr));
        s.extend(tone(0.2, 0.0, sr)); // pausa curta: fica
        s.extend(tone(0.5, 0.5, sr));
        let runs = silent_runs(&s, sr, -40.0, 0.4, 0.1);
        assert_eq!(runs.len(), 1);
        let (a, b) = runs[0];
        assert!((a as f64 / sr as f64 - 1.1).abs() < 0.02, "início {a}");
        assert!((b as f64 / sr as f64 - 1.9).abs() < 0.02, "fim {b}");
    }

    #[test]
    fn normalize_brings_peak_to_target() {
        let g = normalize_gain(0.25, -1.0).unwrap();
        assert!((g - (-1.0 - 20.0 * 0.25f32.log10())).abs() < 1e-4);
        assert!(normalize_gain(0.0, -1.0).is_none());
    }
}

// ------------------------------------------------------------------------------------------
// Mono (voz): as funções abaixo trabalham numa trilha só, no lugar.

/// Pico absoluto (0..1).
pub fn peak(s: &[f32]) -> f32 {
    s.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// Loudness integrado de uma trilha mono tocada nos dois lados (como fica no vídeo).
pub fn lufs_mono(s: &[f32], sample_rate: u32) -> Option<f32> {
    let stereo: Vec<f32> = s.iter().flat_map(|v| [*v, *v]).collect();
    integrated_lufs(&stereo, sample_rate)
}

/// Silêncios de uma trilha mono (mesma regra de `silent_runs`).
pub fn silent_runs_mono(
    s: &[f32],
    sample_rate: u32,
    threshold_db: f32,
    min_len: f64,
    pad: f64,
) -> Vec<(usize, usize)> {
    let stereo: Vec<f32> = s.iter().flat_map(|v| [*v, *v]).collect();
    silent_runs(&stereo, sample_rate, threshold_db, min_len, pad)
}

pub fn gain(s: &mut [f32], db: f32) {
    let g = 10f32.powf(db / 20.0);
    for v in s {
        *v *= g;
    }
}

/// Fade suave (curva em S) no trecho inteiro: de 0 a 1 (`fade_in`) ou de 1 a 0.
pub fn fade(s: &mut [f32], fade_in: bool) {
    let n = s.len().max(1) as f32;
    for (i, v) in s.iter_mut().enumerate() {
        let t = (i as f32 + 0.5) / n;
        let t = if fade_in { t } else { 1.0 - t };
        *v *= t * t * (3.0 - 2.0 * t);
    }
}

/// Passa-altas de 2ª ordem (Butterworth): tira ronco, vento e batidas no microfone.
pub fn high_pass(s: &mut [f32], sample_rate: u32, hz: f32) {
    use std::f64::consts::PI;
    let w0 = 2.0 * PI * hz as f64 / sample_rate as f64;
    let alpha = w0.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
    let cos = w0.cos();
    let a0 = 1.0 + alpha;
    let mut f = Biquad {
        b: [
            (1.0 + cos) / 2.0 / a0,
            -(1.0 + cos) / a0,
            (1.0 + cos) / 2.0 / a0,
        ],
        a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
        x: [0.0; 2],
        y: [0.0; 2],
    };
    for v in s {
        *v = f.run(*v as f64) as f32;
    }
}

/// Passa-baixas de 2ª ordem (Butterworth), par do `high_pass` para dividir o som em faixas.
fn low_pass(s: &mut [f32], sample_rate: u32, hz: f32) {
    use std::f64::consts::PI;
    let w0 = 2.0 * PI * hz as f64 / sample_rate as f64;
    let alpha = w0.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
    let cos = w0.cos();
    let a0 = 1.0 + alpha;
    let mut f = Biquad {
        b: [
            (1.0 - cos) / 2.0 / a0,
            (1.0 - cos) / a0,
            (1.0 - cos) / 2.0 / a0,
        ],
        a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
        x: [0.0; 2],
        y: [0.0; 2],
    };
    for v in s {
        *v = f.run(*v as f64) as f32;
    }
}

/// De-esser: abaixa só a faixa aguda (a partir de `hz`) quando ela passa de `threshold_db`,
/// tirando o chiado dos "s" e "x" sem mexer no resto da voz. Divide o som com um filtro
/// Linkwitz-Riley (graves + agudos somam de volta exatamente) e só mexe no ganho dos agudos.
pub fn de_ess(s: &mut [f32], sample_rate: u32, hz: f32, threshold_db: f32, reduction_db: f32) {
    let mut high = s.to_vec();
    high_pass(&mut high, sample_rate, hz);
    high_pass(&mut high, sample_rate, hz);
    let mut low = s.to_vec();
    low_pass(&mut low, sample_rate, hz);
    low_pass(&mut low, sample_rate, hz);
    let sr = sample_rate as f32;
    let attack = (-1.0 / (0.001 * sr)).exp();
    let release = (-1.0 / (0.03 * sr)).exp();
    let mut env = 0.0f32;
    for ((v, h), l) in s.iter_mut().zip(&high).zip(&low) {
        let x = h.abs();
        env = if x > env {
            attack * env + (1.0 - attack) * x
        } else {
            release * env + (1.0 - release) * x
        };
        let env_db = 20.0 * env.max(1e-6).log10();
        let cut_db = ((env_db - threshold_db) * 0.8).clamp(0.0, reduction_db);
        *v = l + h * 10f32.powf(-cut_db / 20.0);
    }
}

/// Nível (dB) das partes mais baixas (10% mais silenciosas, em janelas de 50 ms):
/// uma boa estimativa do chiado de fundo.
pub fn noise_floor_db(s: &[f32], sample_rate: u32) -> f32 {
    let win = (sample_rate as usize / 20).max(1);
    let mut rms: Vec<f32> = s
        .chunks(win)
        .filter(|c| c.len() == win)
        .map(|c| (c.iter().map(|v| v * v).sum::<f32>() / win as f32).sqrt())
        .collect();
    if rms.is_empty() {
        return -90.0;
    }
    rms.sort_by(f32::total_cmp);
    let v = rms[rms.len() / 10].max(1e-6);
    20.0 * v.log10()
}

/// Seguidor de envelope (pico com ataque e soltura em ms).
fn envelope(s: &[f32], sample_rate: u32, attack_ms: f32, release_ms: f32) -> Vec<f32> {
    let coef = |ms: f32| (-1.0 / (ms.max(0.1) * 0.001 * sample_rate as f32)).exp();
    let (a, r) = (coef(attack_ms), coef(release_ms));
    let mut env = 0.0f32;
    s.iter()
        .map(|v| {
            let x = v.abs();
            let c = if x > env { a } else { r };
            env = c * env + (1.0 - c) * x;
            env
        })
        .collect()
}

/// Noise gate: abaixa `reduction_db` onde o som fica abaixo de `threshold_db` (pausas com
/// chiado), com transições suaves para não cortar o fim das palavras.
pub fn noise_gate(s: &mut [f32], sample_rate: u32, threshold_db: f32, reduction_db: f32) {
    let env = envelope(s, sample_rate, 2.0, 40.0);
    let thr = 10f32.powf(threshold_db / 20.0);
    let floor = 10f32.powf(-reduction_db.abs() / 20.0);
    let hold = (sample_rate as f32 * 0.08) as usize;
    let smooth = (-1.0 / (0.01 * sample_rate as f32)).exp();
    let (mut g, mut open_for) = (1.0f32, 0usize);
    for (v, e) in s.iter_mut().zip(env) {
        if e >= thr {
            open_for = hold;
        } else {
            open_for = open_for.saturating_sub(1);
        }
        let target = if open_for > 0 { 1.0 } else { floor };
        g = smooth * g + (1.0 - smooth) * target;
        *v *= g;
    }
}

/// Compressor: acima de `threshold_db` o volume sobe só 1/`ratio`; `makeup_db` compensa.
pub fn compress(
    s: &mut [f32],
    sample_rate: u32,
    threshold_db: f32,
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    makeup_db: f32,
) {
    let env = envelope(s, sample_rate, attack_ms, release_ms);
    let makeup = 10f32.powf(makeup_db / 20.0);
    let ratio = ratio.max(1.0);
    for (v, e) in s.iter_mut().zip(env) {
        let level = 20.0 * e.max(1e-9).log10();
        let over = level - threshold_db;
        let reduce = if over > 0.0 { over - over / ratio } else { 0.0 };
        *v *= 10f32.powf(-reduce / 20.0) * makeup;
    }
}

#[cfg(test)]
mod de_ess_tests {
    use super::*;

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
    }

    #[test]
    fn it_tames_sibilance_and_leaves_the_voice_alone() {
        let sr = 48_000;
        let tone = |hz: f32, amp: f32| -> Vec<f32> {
            (0..sr)
                .map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / sr as f32).sin())
                .collect()
        };
        let voice = tone(400.0, 0.2);
        let sss = tone(7000.0, 0.3);
        let mut mix: Vec<f32> = voice.iter().zip(&sss).map(|(a, b)| a + b).collect();
        let before = rms(&mix);
        de_ess(&mut mix, sr, 5000.0, -30.0, 12.0);
        let tail = &mix[sr as usize / 2..];
        // sobrou bem menos energia (o "s" foi domado) e a voz de 400 Hz continua lá
        assert!(rms(tail) < before * 0.7, "{} vs {before}", rms(tail));
        assert!(rms(tail) > 0.12);
        // sem chiado nada muda
        let mut clean = voice.clone();
        de_ess(&mut clean, sr, 5000.0, -30.0, 12.0);
        let (c, v) = (
            rms(&clean[sr as usize / 2..]),
            rms(&voice[sr as usize / 2..]),
        );
        assert!((c - v).abs() / v < 0.02, "{c} vs {v}");
    }
}

#[cfg(test)]
mod mono_tests {
    use super::*;

    const SR: u32 = 48_000;

    fn tone(hz: f32, amp: f32, secs: f32) -> Vec<f32> {
        (0..(SR as f32 * secs) as usize)
            .map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / SR as f32).sin())
            .collect()
    }

    #[test]
    fn high_pass_removes_rumble_and_keeps_voice() {
        let mut low = tone(30.0, 0.5, 1.0);
        high_pass(&mut low, SR, 100.0);
        assert!(peak(&low[SR as usize / 2..]) < 0.1, "30 Hz quase some");
        let mut voice = tone(1000.0, 0.5, 1.0);
        high_pass(&mut voice, SR, 100.0);
        assert!(peak(&voice[SR as usize / 2..]) > 0.48, "1 kHz passa");
    }

    #[test]
    fn gate_quiets_the_pauses_and_compressor_tames_peaks() {
        let mut s = tone(500.0, 0.003, 1.0); // chiado
        s.extend(tone(500.0, 0.5, 1.0)); // fala
        s.extend(tone(500.0, 0.003, 1.0));
        let floor = noise_floor_db(&s, SR);
        assert!((floor + 53.0).abs() < 3.0, "chão {floor}");
        noise_gate(&mut s, SR, floor + 10.0, 30.0);
        assert!(peak(&s[(SR as usize * 5) / 2..]) < 0.0003, "pausa abaixada");
        assert!(
            peak(&s[SR as usize + 9_600..SR as usize * 2 - 9_600]) > 0.49,
            "fala intacta"
        );
        let mut loud = tone(500.0, 1.0, 1.0);
        compress(&mut loud, SR, -20.0, 4.0, 1.0, 100.0, 0.0);
        let p = peak(&loud[SR as usize / 2..]);
        assert!(
            (20.0 * p.log10() + 15.0).abs() < 1.5,
            "0 dB vira ≈ −15 dB: {}",
            20.0 * p.log10()
        );
    }

    #[test]
    fn mono_lufs_counts_both_sides_and_fades_go_to_zero() {
        let amp = 10f32.powf(-23.0 / 20.0);
        let l = lufs_mono(&tone(1000.0, amp, 3.0), SR).unwrap();
        assert!((l + 23.0).abs() < 0.3, "{l}");
        let mut s = vec![1.0f32; 1000];
        fade(&mut s, true);
        assert!(s[0] < 0.01 && s[999] > 0.99);
    }
}

// ------------------------------------------------------------------------------------------
// Redução de ruído (como a do Audacity): pega o "perfil" do ruído num trecho só de ruído e
// abaixa, em cada faixa de frequência, o que não passa daquele nível.

const NR_SIZE: usize = 2048;
const NR_HOP: usize = NR_SIZE / 4;

fn hann() -> Vec<f32> {
    (0..NR_SIZE)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / NR_SIZE as f32).cos())
        .collect()
}

/// Potência média de cada faixa de frequência do ruído.
#[derive(Debug, Clone, PartialEq)]
pub struct NoiseProfile {
    pub power: Vec<f32>,
}

/// Mede o ruído de um trecho (o ideal é ~1 s só de ruído, sem fala).
pub fn noise_profile(s: &[f32]) -> Option<NoiseProfile> {
    use rustfft::{FftPlanner, num_complex::Complex};
    if s.len() < NR_SIZE {
        return None;
    }
    let fft = FftPlanner::<f32>::new().plan_fft_forward(NR_SIZE);
    let win = hann();
    let mut power = vec![0.0f32; NR_SIZE / 2 + 1];
    let mut frames = 0;
    let mut buf = vec![Complex::new(0.0, 0.0); NR_SIZE];
    let mut pos = 0;
    while pos + NR_SIZE <= s.len() {
        for i in 0..NR_SIZE {
            buf[i] = Complex::new(s[pos + i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        for (k, p) in power.iter_mut().enumerate() {
            *p += buf[k].norm_sqr();
        }
        frames += 1;
        pos += NR_HOP;
    }
    for p in &mut power {
        *p /= frames as f32;
    }
    Some(NoiseProfile { power })
}

/// Reduz o ruído: em cada faixa de frequência abaixo de `sensitivity_db` acima do perfil,
/// abaixa `reduction_db`. As decisões são suavizadas entre frequências vizinhas e no tempo
/// para não sobrar o "som de robô".
pub fn reduce_noise(s: &mut [f32], profile: &NoiseProfile, reduction_db: f32, sensitivity_db: f32) {
    use rustfft::{FftPlanner, num_complex::Complex};
    let bins = NR_SIZE / 2 + 1;
    if profile.power.len() != bins || s.is_empty() {
        return;
    }
    let mut planner = FftPlanner::<f32>::new();
    let fwd = planner.plan_fft_forward(NR_SIZE);
    let inv = planner.plan_fft_inverse(NR_SIZE);
    let win = hann();
    let floor = 10f32.powf(-reduction_db.abs() / 20.0);
    let thr: Vec<f32> = profile
        .power
        .iter()
        .map(|p| p * 10f32.powf(sensitivity_db / 10.0))
        .collect();
    // Bordas com zeros para a janela cobrir o começo e o fim.
    let pad = NR_SIZE;
    let mut input = vec![0.0f32; s.len() + 2 * pad];
    input[pad..pad + s.len()].copy_from_slice(s);
    let mut out = vec![0.0f32; input.len()];
    let mut buf = vec![Complex::new(0.0, 0.0); NR_SIZE];
    let mut prev_gain = vec![1.0f32; bins];
    let mut pos = 0;
    while pos + NR_SIZE <= input.len() {
        for i in 0..NR_SIZE {
            buf[i] = Complex::new(input[pos + i] * win[i], 0.0);
        }
        fwd.process(&mut buf);
        let mut gain: Vec<f32> = (0..bins)
            .map(|k| {
                if buf[k].norm_sqr() > thr[k] {
                    1.0
                } else {
                    floor
                }
            })
            .collect();
        // Suaviza entre frequências vizinhas (±3 faixas).
        let raw = gain.clone();
        for (k, g) in gain.iter_mut().enumerate() {
            let (a, b) = (k.saturating_sub(3), (k + 4).min(bins));
            *g = raw[a..b].iter().sum::<f32>() / (b - a) as f32;
        }
        // No tempo: abre na hora, fecha devagar (evita picotar o fim das palavras).
        for (g, p) in gain.iter_mut().zip(prev_gain.iter_mut()) {
            *g = g.max(*p * 0.7).max(floor);
            *p = *g;
        }
        for k in 0..bins {
            buf[k] *= gain[k];
            if k > 0 && k < NR_SIZE / 2 {
                buf[NR_SIZE - k] *= gain[k];
            }
        }
        inv.process(&mut buf);
        for i in 0..NR_SIZE {
            out[pos + i] += buf[i].re / NR_SIZE as f32 * win[i];
        }
        pos += NR_HOP;
    }
    // Janela de Hann na análise e na síntese com sobreposição de 75%: soma 1,5.
    let n = s.len();
    for (v, o) in s.iter_mut().zip(&out[pad..pad + n]) {
        *v = o / 1.5;
    }
}

/// Graves e agudos (shelf em 250 Hz e 4 kHz), em dB.
pub fn bass_treble(s: &mut [f32], sample_rate: u32, bass_db: f32, treble_db: f32) {
    use std::f64::consts::PI;
    let shelf = |hz: f64, db: f32, low: bool| {
        let a = 10f64.powf(db as f64 / 40.0);
        let w0 = 2.0 * PI * hz / sample_rate as f64;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / 2.0 * std::f64::consts::SQRT_2;
        let sq = 2.0 * a.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = if low {
            (
                a * ((a + 1.0) - (a - 1.0) * cos + sq),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - sq),
                (a + 1.0) + (a - 1.0) * cos + sq,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - sq,
            )
        } else {
            (
                a * ((a + 1.0) + (a - 1.0) * cos + sq),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - sq),
                (a + 1.0) - (a - 1.0) * cos + sq,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - sq,
            )
        };
        Biquad {
            b: [b0 / a0, b1 / a0, b2 / a0],
            a: [a1 / a0, a2 / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    };
    let (mut lo, mut hi) = (shelf(250.0, bass_db, true), shelf(4000.0, treble_db, false));
    for v in s {
        *v = hi.run(lo.run(*v as f64)) as f32;
    }
}

/// Limitador: nada passa de `ceiling_db` (ganho cai na hora e volta devagar).
pub fn limit(s: &mut [f32], sample_rate: u32, ceiling_db: f32) {
    let ceiling = 10f32.powf(ceiling_db / 20.0);
    let release = (-1.0 / (0.08 * sample_rate as f32)).exp();
    let mut g = 1.0f32;
    for v in s {
        // O ganho volta devagar para 1, mas nunca deixa esta amostra passar do teto.
        let need = if v.abs() > ceiling {
            ceiling / v.abs()
        } else {
            1.0
        };
        g = (release * g + (1.0 - release)).min(need);
        *v *= g;
    }
}

#[cfg(test)]
mod nr_tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// Ruído branco determinístico.
    fn noise(n: usize, amp: f32) -> Vec<f32> {
        let mut x = 12345u32;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
    }

    #[test]
    fn noise_reduction_lowers_the_hiss_and_keeps_the_voice() {
        let n = SR as usize * 2;
        let hiss = noise(n, 0.02);
        let profile = noise_profile(&hiss[..SR as usize / 2]).unwrap();
        // Primeiro segundo: só chiado. Segundo: tom + chiado.
        let mut s: Vec<f32> = (0..n)
            .map(|i| {
                let tone = if i >= SR as usize {
                    0.3 * (std::f32::consts::TAU * 440.0 * i as f32 / SR).sin()
                } else {
                    0.0
                };
                tone + hiss[i]
            })
            .collect();
        let before = rms(&s[2000..SR as usize - 2000]);
        reduce_noise(&mut s, &profile, 24.0, 6.0);
        let after = rms(&s[2000..SR as usize - 2000]);
        assert!(
            20.0 * (after / before).log10() < -15.0,
            "chiado: {before} → {after}"
        );
        let voice = rms(&s[SR as usize + 4000..n - 4000]);
        assert!(
            (voice - 0.3 / 2f32.sqrt()).abs() < 0.03,
            "voz preservada: {voice}"
        );
    }

    #[test]
    fn limiter_and_tone_controls() {
        let mut s: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.05).sin()).collect();
        limit(&mut s, 48_000, -6.0);
        assert!(peak(&s) <= 10f32.powf(-6.0 / 20.0) + 1e-4);
        let mut low: Vec<f32> = (0..48_000)
            .map(|i| 0.1 * (std::f32::consts::TAU * 60.0 * i as f32 / SR).sin())
            .collect();
        bass_treble(&mut low, 48_000, 12.0, 0.0);
        assert!(peak(&low[24_000..]) > 0.3, "graves +12 dB");
    }
}
