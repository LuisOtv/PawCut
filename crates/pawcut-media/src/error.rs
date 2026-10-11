use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("não foi possível ler {}: {source}", .path.display())]
    Ffmpeg { path: PathBuf, source: ffmpeg_next::Error },
    #[error("{} não tem vídeo nem áudio que o PawCut entenda", .path.display())]
    Unsupported { path: PathBuf },
    #[error("não foi possível ler a imagem {}: {source}", .path.display())]
    Image { path: PathBuf, source: image::ImageError },
    #[error("erro no cache de áudio {}: {source}", .path.display())]
    Cache { path: PathBuf, source: std::io::Error },
}

impl MediaError {
    pub(crate) fn ffmpeg(path: &Path, source: ffmpeg_next::Error) -> Self {
        Self::Ffmpeg { path: path.to_path_buf(), source }
    }
}
