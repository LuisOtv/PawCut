//! Mede a mixagem ao vivo numa sessão grande (30 min, 4 faixas, muitos cortes).
//! Uso: cargo run --release -p pawvoice-core --example bench_mix

use pawvoice_core::{Editor, Session};
use std::collections::BTreeMap;
use std::time::Instant;

fn main() {
    let rate = 48_000usize;
    let minutes = 30;
    let mut ed = Editor::new(Session::new(rate as u32), BTreeMap::new());
    let audio: Vec<f32> = (0..rate * 60 * minutes)
        .map(|i| 0.2 * (i as f32 * 0.03).sin())
        .collect();
    let mut ids = Vec::new();
    for _ in 0..4 {
        ids.push(ed.add_track(vec![audio.clone()], 0, None));
    }
    // Muitos cortes: apaga um pedacinho a cada 2 s em cada faixa (~900 trechos por faixa).
    for id in &ids {
        for k in (1..minutes * 30).rev() {
            let at = k * 2 * rate;
            ed.delete(&[*id], at, at + rate / 20).unwrap();
        }
    }
    let t = Instant::now();
    let snap = ed.mixer();
    println!(
        "foto da sessão (mixer): {:.2} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    let len = ed.session().len();
    let t = Instant::now();
    let mut worst = 0.0f64;
    let mut n = 0;
    let mut at = 0;
    while at + 480 < len {
        let t0 = Instant::now();
        let _ = snap.mix(at, at + 480);
        worst = worst.max(t0.elapsed().as_secs_f64() * 1000.0);
        at += 480 * 500; // amostra espalhada pela sessão
        n += 1;
    }
    println!(
        "bloco de 10 ms: média {:.3} ms, pior {:.3} ms ({n} blocos, {} faixas)",
        t.elapsed().as_secs_f64() * 1000.0 / n as f64,
        worst,
        ids.len()
    );
    let t = Instant::now();
    let _ = ed.mix(0, len.min(rate * 60 * 5));
    println!(
        "mixagem inteira de 5 min (o jeito antigo, na thread da tela): {:.0} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
}
