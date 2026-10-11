//! Leitura de mídia para o PawCut: metadados, frames de vídeo, imagens e áudio.

mod error;
mod probe;
mod video;

pub use error::MediaError;
pub use image::RgbaImage;
pub use probe::probe;
pub use video::VideoSource;

use std::sync::Once;

/// Inicializa o ffmpeg uma vez só e silencia os logs dele.
pub(crate) fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = ffmpeg_next::init();
        ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Fatal);
    });
}
