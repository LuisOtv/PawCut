//! Mede a gravação: quanto demora para o primeiro áudio chegar e se o fim é preservado.
//! Uso: cargo run -p paw-audio --example rec_latency

use std::time::{Duration, Instant};

fn main() {
    let rate = 48_000;
    let mut rec = paw_audio::io::Recorder::new(rate);
    let t0 = Instant::now();
    rec.start(None);
    while rec.is_empty() && t0.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(1));
    }
    let first = t0.elapsed();
    std::thread::sleep(Duration::from_millis(2000) - first.min(Duration::from_millis(2000)));
    let wall = t0.elapsed();
    let samples = rec.stop();
    if let Some(e) = rec.take_error() {
        println!("erro: {e}");
    }
    println!(
        "primeiro áudio em {:.0} ms; gravou {:.3} s em {:.3} s de relógio",
        first.as_secs_f64() * 1000.0,
        samples.len() as f64 / rate as f64,
        wall.as_secs_f64()
    );
}
