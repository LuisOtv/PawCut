//! Mede quanto cada efeito demora em 10 minutos de áudio mono (para achar o que trava a tela).
//! Uso: cargo run --release -p paw-audio --example bench_dsp

use paw_audio::dsp;
use std::time::Instant;

fn main() {
    let rate = 48_000u32;
    let secs = 600;
    let tone = |i: usize| {
        let t = i as f32 / rate as f32;
        0.1 * (std::f32::consts::TAU * 220.0 * t).sin() * (0.5 + 0.5 * (t * 0.7).sin())
            + 0.003 * (std::f32::consts::TAU * 3000.0 * t).sin()
    };
    let base: Vec<f32> = (0..rate as usize * secs).map(tone).collect();
    let time = |name: &str, f: &dyn Fn(&mut [f32])| {
        let mut s = base.clone();
        let t = Instant::now();
        f(&mut s);
        println!("{name:<28} {:>7.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    };
    time("ganho", &|s| dsp::gain(s, 3.0));
    time("passa-altas", &|s| dsp::high_pass(s, rate, 80.0));
    time("noise gate", &|s| dsp::noise_gate(s, rate, -50.0, 18.0));
    time("compressor", &|s| {
        dsp::compress(s, rate, -22.0, 3.0, 5.0, 100.0, 0.0)
    });
    time("limitador", &|s| dsp::limit(s, rate, -1.0));
    time("graves e agudos", &|s| dsp::bass_treble(s, rate, 3.0, 3.0));
    time("de-esser", &|s| dsp::de_ess(s, rate, 5500.0, -30.0, 9.0));
    let t = Instant::now();
    let _ = dsp::integrated_lufs(
        &base.iter().flat_map(|v| [*v, *v]).collect::<Vec<_>>(),
        rate,
    );
    println!(
        "{:<28} {:>7.0} ms",
        "loudness (LUFS)",
        t.elapsed().as_secs_f64() * 1000.0
    );
    let t = Instant::now();
    let _ = dsp::silent_runs_mono(&base, rate, -40.0, 0.5, 0.06);
    println!(
        "{:<28} {:>7.0} ms",
        "achar silêncios",
        t.elapsed().as_secs_f64() * 1000.0
    );
    let t = Instant::now();
    let prof = dsp::noise_profile(&base[..rate as usize]).expect("perfil");
    println!(
        "{:<28} {:>7.0} ms",
        "perfil de ruído (1 s)",
        t.elapsed().as_secs_f64() * 1000.0
    );
    time("redução de ruído", &|s| {
        dsp::reduce_noise(s, &prof, 12.0, 6.0)
    });
}
