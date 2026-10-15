//! Efeitos de voz aplicados num trecho (cada um vira um passo de desfazer). Trabalham em
//! canais separados; os que mexem no volume geral (loudness, normalizar) usam o mesmo ganho
//! nos dois lados para não desequilibrar o estéreo.

use paw_audio::dsp;

/// Ajuste de compressor: (limiar dB, razão, ataque ms, soltura ms).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Compression {
    Light,
    Medium,
    Strong,
}

impl Compression {
    pub fn label(self) -> &'static str {
        match self {
            Compression::Light => "Leve",
            Compression::Medium => "Médio",
            Compression::Strong => "Forte",
        }
    }

    fn params(self) -> (f32, f32, f32, f32) {
        match self {
            Compression::Light => (-18.0, 2.0, 10.0, 120.0),
            Compression::Medium => (-22.0, 3.0, 5.0, 100.0),
            Compression::Strong => (-26.0, 5.0, 3.0, 80.0),
        }
    }
}

/// Aplica `f` em cada canal.
pub fn each(p: &mut [Vec<f32>], mut f: impl FnMut(&mut [f32])) {
    for ch in p.iter_mut() {
        f(ch);
    }
}

pub fn compress(p: &mut [Vec<f32>], rate: u32, c: Compression) {
    let (thr, ratio, att, rel) = c.params();
    each(p, |s| dsp::compress(s, rate, thr, ratio, att, rel, 0.0));
}

fn peak(p: &[Vec<f32>]) -> f32 {
    p.iter().map(|c| dsp::peak(c)).fold(0.0, f32::max)
}

/// Loudness dos canais juntos (mono conta dos dois lados, como fica no vídeo).
pub fn lufs(p: &[Vec<f32>], rate: u32) -> Option<f32> {
    match p {
        [mono] => dsp::lufs_mono(mono, rate),
        [l, r, ..] => {
            let stereo: Vec<f32> = l.iter().zip(r).flat_map(|(a, b)| [*a, *b]).collect();
            dsp::integrated_lufs(&stereo, rate)
        }
        [] => None,
    }
}

/// Leva o trecho a `target` LUFS sem o pico passar de −1 dBFS. Devolve o loudness de antes.
pub fn loudness(p: &mut [Vec<f32>], rate: u32, target: f32) -> Option<f32> {
    let l = lufs(p, rate)?;
    let g = dsp::loudness_gain(l, peak(p), target);
    each(p, |s| dsp::gain(s, g));
    Some(l)
}

pub fn peak_normalize(p: &mut [Vec<f32>]) {
    if let Some(g) = dsp::normalize_gain(peak(p), -1.0) {
        each(p, |s| dsp::gain(s, g));
    }
}

/// "Melhorar voz" em um clique: passa-altas, noise gate automático, compressor médio,
/// loudness no alvo e limitador em −1 dB.
pub fn enhance_voice(p: &mut [Vec<f32>], rate: u32, target: f32) {
    each(p, |s| {
        dsp::high_pass(s, rate, 80.0);
        let floor = dsp::noise_floor_db(s, rate);
        dsp::noise_gate(s, rate, floor + 8.0, 18.0);
    });
    compress(p, rate, Compression::Medium);
    loudness(p, rate, target);
    each(p, |s| dsp::limit(s, rate, -1.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhance_voice_reaches_the_loudness_target() {
        let rate = 48_000;
        let ch: Vec<f32> = (0..rate * 4)
            .map(|i| {
                let t = i as f32 / rate as f32;
                // "fala" de 1 a 3 s com chiado o tempo todo
                let voice = if (1.0..3.0).contains(&t) {
                    0.1 * (std::f32::consts::TAU * 220.0 * t).sin()
                } else {
                    0.0
                };
                voice + 0.002 * (std::f32::consts::TAU * 3000.0 * t).sin()
            })
            .collect();
        for channels in [1, 2] {
            let mut p = vec![ch.clone(); channels];
            enhance_voice(&mut p, rate as u32, -16.0);
            let l = lufs(&p, rate as u32).unwrap();
            assert!((l + 16.0).abs() < 1.0, "{channels} canal(is): {l}");
            assert!(peak(&p) <= 10f32.powf(-1.0 / 20.0) + 1e-3);
        }
    }
}
