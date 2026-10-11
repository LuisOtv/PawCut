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
    /// Qualidade (CRF): menor é melhor (18 é visualmente sem perda).
    pub crf: u8,
    /// Só um trecho `[início, fim)` da timeline (In/Out); `None` = tudo.
    pub range: Option<(Frame, Frame)>,
    /// Tamanho da saída; `None` = resolução do projeto.
    pub size: Option<(u32, u32)>,
    pub codec: VideoCodec,
    /// Exportar só o áudio.
    pub audio_only: Option<AudioFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoCodec {
    #[default]
    H264,
    /// HEVC: arquivos menores, mais lento para exportar.
    H265,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Mp3,
    Wav,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            output: PathBuf::new(),
            crf: 18,
            range: None,
            size: None,
            codec: VideoCodec::H264,
            audio_only: None,
        }
    }
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
    let (from, to) = settings.range.unwrap_or((0, project.duration()));
    let (from, to) = (from.max(0), to.min(project.duration()));
    let total = to - from;
    if total <= 0 {
        return Err(ExportError::Empty);
    }
    // Arquivo sumido no áudio também para a exportação (senão sairia mudo sem avisar).
    if let Some((frame, path)) = first_missing_media(project) {
        return Err(ExportError::Offline { frame, path });
    }
    let work = tempfile::tempdir()?;
    let audio = work.path().join("audio.f32");
    write_audio(project, renderer, from, total, &audio, cancel)?;

    if let Some(format) = settings.audio_only {
        let mut child = spawn_audio_ffmpeg(project, &audio, format, &settings.output)?;
        let mut stderr = child.stderr.take().expect("stderr configurado");
        let log = thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });
        drop(child.stdin.take());
        let result = finish(child, Ok(()), log, &settings.output);
        if result.is_ok() {
            on_progress(Progress { done: total, total });
        }
        return result;
    }

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
        (from, total),
        output_size(project, settings),
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
    from: Frame,
    total: Frame,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<(), ExportError> {
    let s = &project.settings;
    let first = s.fps.frame_to_sample(from, s.sample_rate);
    let samples = s.fps.frame_to_sample(from + total, s.sample_rate) - first;
    let mut out = BufWriter::new(File::create(path)?);
    let mut pos = 0i64;
    while pos < samples {
        if cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        let n = AUDIO_CHUNK.min((samples - pos) as usize);
        for v in renderer.mix_audio(project, first + pos, n) {
            out.write_all(&v.to_le_bytes())?;
        }
        pos += n as i64;
    }
    out.flush()?;
    Ok(())
}

/// Tamanho da saída (par, como o H.264/H.265 em 4:2:0 exige).
fn output_size(project: &Project, settings: &ExportSettings) -> (u32, u32) {
    let (w, h) = settings
        .size
        .unwrap_or((project.settings.width, project.settings.height));
    (w.max(2).div_ceil(2) * 2, h.max(2).div_ceil(2) * 2)
}

fn spawn_ffmpeg(
    project: &Project,
    audio: &Path,
    settings: &ExportSettings,
) -> Result<Child, ExportError> {
    let s = &project.settings;
    let (w, h) = output_size(project, settings);
    let crf = settings.crf.to_string();
    let codec: &[&str] = match settings.codec {
        VideoCodec::H264 => &["-c:v", "libx264", "-preset", "medium", "-crf", &crf],
        // A tag hvc1 faz o arquivo abrir no QuickTime/iPhone.
        VideoCodec::H265 => &[
            "-c:v",
            "libx265",
            "-preset",
            "medium",
            "-crf",
            &crf,
            "-tag:v",
            "hvc1",
            "-x265-params",
            "log-level=error",
        ],
    };
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{w}x{h}")])
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
        .args(codec)
        .args([
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
        ])
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

fn spawn_audio_ffmpeg(
    project: &Project,
    audio: &Path,
    format: AudioFormat,
    output: &Path,
) -> Result<Child, ExportError> {
    let s = &project.settings;
    let codec: &[&str] = match format {
        AudioFormat::Mp3 => &["-c:a", "libmp3lame", "-b:a", "256k"],
        AudioFormat::Wav => &["-c:a", "pcm_s16le"],
    };
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y"])
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
        .args(codec)
        .arg(output)
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
    (from, total): (Frame, Frame),
    (w, h): (u32, u32),
    stdin: &mut ChildStdin,
    cancel: &AtomicBool,
    on_progress: &mut impl FnMut(Progress),
) -> Result<(), ExportError> {
    for i in 0..total {
        if cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        let frame = from + i;
        let rendered = renderer.render_frame(project, frame, w, h);
        if let Some(&clip) = rendered.offline.first() {
            return Err(ExportError::Offline {
                frame,
                path: media_path(project, clip),
            });
        }
        stdin.write_all(rendered.image.as_raw())?;
        on_progress(Progress { done: i + 1, total });
    }
    Ok(())
}

/// Salva o frame `frame` da timeline como PNG, no tamanho do projeto.
pub fn export_frame(
    project: &Project,
    renderer: &mut Renderer,
    frame: Frame,
    output: &Path,
) -> Result<(), ExportError> {
    let s = &project.settings;
    let rendered = renderer.render_frame(project, frame, s.width, s.height);
    rendered
        .image
        .save(output)
        .map_err(|e| ExportError::Io(std::io::Error::other(e.to_string())))
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

/// Primeiro clipe (em ordem de tempo) cujo arquivo não existe.
fn first_missing_media(project: &Project) -> Option<(Frame, PathBuf)> {
    project
        .all_tracks()
        .flat_map(|t| t.clips.iter())
        .filter_map(|c| match &c.source {
            ClipSource::Media(id) => project
                .media(*id)
                .filter(|m| !m.info.path.exists())
                .map(|m| (c.start, m.info.path.clone())),
            _ => None,
        })
        .min_by_key(|(start, _)| *start)
}
