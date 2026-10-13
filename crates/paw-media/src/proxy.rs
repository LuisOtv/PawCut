//! Mídia otimizada (proxy): uma cópia leve do vídeo só para o preview ficar liso.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use crate::{MediaError, default_cache_dir};

/// Onde fica (ou ficaria) o proxy de `path`. Muda se o arquivo original mudar.
pub fn proxy_path(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let mut h = DefaultHasher::new();
    (path, meta.len(), mtime).hash(&mut h);
    Some(
        default_cache_dir()?
            .join("proxies")
            .join(format!("{:016x}.mp4", h.finish())),
    )
}

pub fn has_proxy(path: &Path) -> bool {
    proxy_path(path).is_some_and(|p| p.exists())
}

/// Gera o proxy (540p, H.264 rápido de decodificar, sem áudio). Bloqueia: rode numa thread.
pub fn make_proxy(path: &Path) -> Result<PathBuf, MediaError> {
    let out = proxy_path(path).ok_or_else(|| MediaError::Unsupported {
        path: path.to_path_buf(),
    })?;
    let io = |source| MediaError::Cache {
        path: out.clone(),
        source,
    };
    std::fs::create_dir_all(out.parent().expect("pasta de proxies")).map_err(io)?;
    let tmp = out.with_extension("tmp.mp4");
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-vf",
            "scale=-2:'min(540,ih)'",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "23",
            "-g",
            "15",
            "-tune",
            "fastdecode",
            "-an",
        ])
        .arg(&tmp)
        .status()
        .map_err(io)?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(MediaError::Unsupported {
            path: path.to_path_buf(),
        });
    }
    std::fs::rename(&tmp, &out).map_err(io)?;
    Ok(out)
}
