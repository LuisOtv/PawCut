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
