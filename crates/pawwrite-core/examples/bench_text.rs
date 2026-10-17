//! Mede o que o PawWrite refaz a cada tecla num roteiro grande.
//! Uso: cargo run --release -p pawwrite-core --example bench_text -- arquivo.md

use pawwrite_core as core;
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("passe um arquivo .md");
    let text = std::fs::read_to_string(path).expect("ler o arquivo");
    println!(
        "{} bytes, {} palavras",
        text.len(),
        core::count_words(&text)
    );
    let time = |name: &str, f: &dyn Fn()| {
        let t = Instant::now();
        for _ in 0..5 {
            f();
        }
        println!(
            "{name:<34} {:>8.2} ms",
            t.elapsed().as_secs_f64() * 1000.0 / 5.0
        );
    };
    time("analyze (estrutura e tempos)", &|| {
        let _ = core::analyze(&text);
    });
    time("spans (cores do editor)", &|| {
        let _ = core::spans(&text);
    });
    time("clone do texto", &|| {
        let _ = text.clone();
    });
    time("comparar texto salvo (dirty)", &|| {
        let other = text.clone();
        let _ = text == other;
    });
    time("achar a posição do cursor no fim", &|| {
        let n = text.chars().count();
        let _ = text.char_indices().nth(n.saturating_sub(10));
    });
    // Roteiro de ~1 h de fala (uns 70 KB): trocar uma palavra comum.
    let one_hour: String = text.chars().take(70_000).collect();
    let t = Instant::now();
    let (_, n) = core::replace_all(&one_hour, "e", "ee");
    println!(
        "{:<34} {:>8.2} ms ({n} trocas em {} bytes)",
        "trocar 'e' (roteiro de 1 h)",
        t.elapsed().as_secs_f64() * 1000.0,
        one_hour.len()
    );
    time("buscar uma palavra rara", &|| {
        let _ = core::find_next(&text, "zzzz", 0);
    });
}
