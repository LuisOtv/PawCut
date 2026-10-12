//! Leitura de mídia para o PawCut: metadados, frames de vídeo, imagens e áudio.

mod audio;
mod error;
mod library;
mod orient;
mod probe;
mod proxy;
mod video;

pub use audio::{Pcm, decode_audio, load_audio, peaks};
pub use error::MediaError;
pub use image::RgbaImage;
pub use library::{MediaLibrary, default_cache_dir, load_image, thumbnail};
pub use probe::probe;
pub use proxy::{has_proxy, make_proxy, proxy_path};
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
