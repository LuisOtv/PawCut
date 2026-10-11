//! Exportação (Deliver): renderiza a timeline frame a frame e manda para o ffmpeg gerar o MP4.

use std::fs::{self, File};
use std::io::{BufWriter, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use pawcut_core::{ClipId, ClipSource, Frame, Project};
use pawcut_render::Renderer;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ExportSettings {
    pub output: PathBuf,
    /// Qualidade do H.264: menor é melhor (18 é visualmente sem perda).
    pub crf: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: Frame,
    pub total: Frame,
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("a timeline está vazia")]
    Empty,
    #[error("mídia offline no frame {frame}: {}", .path.display())]
    Offline { frame: Frame, path: PathBuf },
    #[error("o ffmpeg não foi encontrado (instale com `sudo apt install ffmpeg`)")]
    FfmpegMissing,
    #[error("o ffmpeg falhou: {log}")]
    Ffmpeg { log: String },
    #[error("erro de arquivo: {0}")]
    Io(#[from] std::io::Error),
    #[error("exportação cancelada")]
    Cancelled,
}

/// Amostras de áudio mixadas por vez ao gerar a trilha.
const AUDIO_CHUNK: usize = 48_000;

/// Exporta a timeline inteira para `settings.output` (MP4 H.264 + AAC).
/// `cancel` pode ser ligado de outra thread; o arquivo parcial é apagado em qualquer falha.
pub fn export(
    project: &Project,
    renderer: &mut Renderer,
    settings: &ExportSettings,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(Progress),
) -> Result<(), ExportError> {
    let total = project.duration();
    if total == 0 {
        return Err(ExportError::Empty);
    }
    let work = tempfile::tempdir()?;
    let audio = work.path().join("audio.f32");
    write_audio(project, renderer, total, &audio, cancel)?;

    let mut child = spawn_ffmpeg(project, &audio, settings)?;
    let mut stderr = child.stderr.take().expect("stderr configurado");
    let log = thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let mut stdin = child.stdin.take().expect("stdin configurado");
    let fed = feed_frames(
        project,
        renderer,
        total,
        &mut stdin,
        cancel,
        &mut on_progress,
    );
    drop(stdin);
    finish(child, fed, log, &settings.output)
}

fn finish(
    mut child: Child,
    fed: Result<(), ExportError>,
    log: thread::JoinHandle<String>,
    output: &Path,
) -> Result<(), ExportError> {
    if fed.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let log = log.join().unwrap_or_default();
    let tail = |log: String| {
        log.lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    };
    let result = match fed {
        Err(ExportError::Io(e)) if e.kind() == ErrorKind::BrokenPipe => {
            Err(ExportError::Ffmpeg { log: tail(log) })
        }
        Err(e) => Err(e),
        Ok(()) if !status.success() => Err(ExportError::Ffmpeg { log: tail(log) }),
        Ok(()) => Ok(()),
    };
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result
}

fn write_audio(
    project: &Project,
    renderer: &mut Renderer,
    total: Frame,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<(), ExportError> {
    let s = &project.settings;
    let samples = s.fps.frame_to_sample(total, s.sample_rate);
    let mut out = BufWriter::new(File::create(path)?);
    let mut pos = 0i64;
    while pos < samples {
        if cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        let n = AUDIO_CHUNK.min((samples - pos) as usize);
        for v in renderer.mix_audio(project, pos, n) {
            out.write_all(&v.to_le_bytes())?;
        }
        pos += n as i64;
    }
    out.flush()?;
    Ok(())
}

fn spawn_ffmpeg(
    project: &Project,
    audio: &Path,
    settings: &ExportSettings,
) -> Result<Child, ExportError> {
    let s = &project.settings;
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{}x{}", s.width, s.height)])
        .args(["-framerate", &format!("{}/{}", s.fps.num, s.fps.den)])
        .args(["-i", "-"])
        .args([
            "-f",
            "f32le",
            "-ar",
            &s.sample_rate.to_string(),
            "-ac",
            "2",
            "-i",
        ])
        .arg(audio)
        .args(["-map", "0:v:0", "-map", "1:a:0"])
        // H.264 em yuv420p exige largura e altura pares.
        .args(["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2"])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            &settings.crf.to_string(),
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"])
        .arg(&settings.output)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    cmd.spawn().map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            ExportError::FfmpegMissing
        } else {
            e.into()
        }
    })
}

fn feed_frames(
    project: &Project,
    renderer: &mut Renderer,
    total: Frame,
    stdin: &mut ChildStdin,
    cancel: &AtomicBool,
    on_progress: &mut impl FnMut(Progress),
) -> Result<(), ExportError> {
    let s = &project.settings;
    for frame in 0..total {
        if cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        let rendered = renderer.render_frame(project, frame, s.width, s.height);
        if let Some(&clip) = rendered.offline.first() {
            return Err(ExportError::Offline {
                frame,
                path: media_path(project, clip),
            });
        }
        stdin.write_all(rendered.image.as_raw())?;
        on_progress(Progress {
            done: frame + 1,
            total,
        });
    }
    Ok(())
}

fn media_path(project: &Project, clip: ClipId) -> PathBuf {
    match project.clip(clip).map(|c| &c.source) {
        Some(ClipSource::Media(id)) => project
            .media(*id)
            .map(|m| m.info.path.clone())
            .unwrap_or_default(),
        _ => PathBuf::new(),
    }
}
