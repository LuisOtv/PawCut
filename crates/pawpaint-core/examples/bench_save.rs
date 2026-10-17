//! Mede quanto o salvamento (e o autosave) demora num documento grande com várias camadas.
//! Uso: cargo run --release -p pawpaint-core --example bench_save

use image::RgbaImage;
use pawpaint_core::{Editor, file};
use std::time::Instant;

fn main() {
    let (w, h) = (4000u32, 3000u32);
    // Degradê com um pouco de ruído, para o PNG trabalhar como numa foto.
    let mut seed = 12345u32;
    let img = RgbaImage::from_fn(w, h, |x, y| {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let n = (seed >> 28) as u8;
        image::Rgba([
            (x * 255 / w) as u8 ^ n,
            (y * 255 / h) as u8,
            ((x + y) % 256) as u8 ^ n,
            255,
        ])
    });
    let mut ed = Editor::from_image(&img, "Fundo");
    for k in 0..4 {
        ed.import_layer(&img, &format!("Camada {k}"));
    }
    let dir = std::env::temp_dir().join("pawpaint-bench");
    std::fs::create_dir_all(&dir).unwrap();
    for (nome, speed) in [
        ("melhor", file::SaveSpeed::Best),
        ("rápido (autosave)", file::SaveSpeed::Fast),
    ] {
        let t = Instant::now();
        file::save_doc(ed.doc(), &dir.join("grande.pawpaint"), speed).unwrap();
        println!(
            "salvar {w}x{h}, 5 camadas, {nome}: {:.0} ms ({} MB no disco)",
            t.elapsed().as_secs_f64() * 1000.0,
            std::fs::metadata(dir.join("grande.pawpaint"))
                .unwrap()
                .len()
                / 1_000_000
        );
    }
    let t = Instant::now();
    let doc = ed.doc().clone();
    println!(
        "copiar o documento (como o desfazer faz): {:.3} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    drop(doc);
}
