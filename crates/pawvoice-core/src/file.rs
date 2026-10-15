//! Salvar/abrir sessões e exportar áudio.
//!
//! `Nome.pawvoice` é o JSON da sessão; as fontes ficam em `Nome.pawvoice-audio/<id>.wav`.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use thiserror::Error;

use crate::wav::{read_wav_f32, write_wav_f32};
use crate::{Editor, SESSION_VERSION, SILENCE, Session, SourceId};

#[derive(Debug, Error)]
pub enum FileError {
    #[error("erro de arquivo: {0}")]
    Io(#[from] std::io::Error),
    #[error("sessão inválida: {0}")]
    Json(#[from] serde_json::Error),
    #[error("sessão de uma versão mais nova do PawVoice ({0})")]
    TooNew(u32),
    #[error("o ffmpeg falhou: {0}")]
    Ffmpeg(String),
}

pub fn audio_dir(session: &Path) -> PathBuf {
    let mut name = session.file_name().unwrap_or_default().to_os_string();
    name.push("-audio");
    session.with_file_name(name)
}

/// Salva a sessão e as fontes que ela usa (as que já existem no disco não são regravadas;
/// as que não são mais usadas são apagadas).
pub fn save_session(editor: &Editor, path: &Path) -> Result<(), FileError> {
    let dir = audio_dir(path);
    std::fs::create_dir_all(&dir)?;
    let used: std::collections::HashSet<SourceId> = editor
        .session()
        .tracks
        .iter()
        .flat_map(|t| t.segments.iter().map(|s| s.source))
        .filter(|s| *s != SILENCE)
        .collect();
    let rate = editor.session().sample_rate;
    for id in &used {
        let file = dir.join(format!("{id}.wav"));
        if !file.exists()
            && let Some(src) = editor.sources().get(id)
        {
            let tmp = dir.join(format!("{id}.wav.tmp"));
            write_wav_f32(&tmp, &src.data, rate, src.channels as u16)?;
            std::fs::rename(&tmp, &file)?;
        }
    }
    let json = serde_json::to_string_pretty(editor.session())?;
    let tmp = path.with_extension("pawvoice.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    for e in std::fs::read_dir(&dir)?.flatten() {
        let keep = e
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<SourceId>().ok())
            .is_some_and(|id| used.contains(&id));
        if !keep {
            let _ = std::fs::remove_file(e.path());
        }
    }
    Ok(())
}

pub fn load_session(path: &Path) -> Result<Editor, FileError> {
    let session: Session = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    if session.version > SESSION_VERSION {
        return Err(FileError::TooNew(session.version));
    }
    let dir = audio_dir(path);
    let mut sources = BTreeMap::new();
    for id in session
        .tracks
        .iter()
        .flat_map(|t| t.segments.iter().map(|s| s.source))
        .filter(|s| *s != SILENCE)
    {
        if let std::collections::btree_map::Entry::Vacant(e) = sources.entry(id) {
            let (data, _, channels) = read_wav_f32(&dir.join(format!("{id}.wav")))?;
            e.insert(Arc::new(crate::Audio {
                channels: channels as u8,
                data,
            }));
        }
    }
    let mut editor = Editor::new(session, sources);
    editor.mark_saved();
    Ok(editor)
}

/// Exporta áudio intercalado (`channels` canais): `.wav` (24 bits) ou `.mp3` (192 kb/s),
/// pelo ffmpeg.
pub fn export_audio(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
    out: &Path,
) -> Result<(), FileError> {
    let mp3 = out
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"));
    let codec: &[&str] = if mp3 {
        &["-c:a", "libmp3lame", "-b:a", "192k"]
    } else {
        &["-c:a", "pcm_s24le"]
    };
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "f32le", "-ar"])
        .arg(sample_rate.to_string())
        .args(["-ac", &channels.to_string(), "-i", "-"])
        .args(codec)
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                FileError::Ffmpeg("o ffmpeg não foi encontrado".into())
            } else {
                e.into()
            }
        })?;
    {
        let mut stdin = child.stdin.take().expect("stdin configurado");
        let bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
        stdin.write_all(&bytes)?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let _ = std::fs::remove_file(out);
        return Err(FileError::Ffmpeg(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_keep_the_edits_and_clean_unused_audio() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("narracao.pawvoice");
        let mut e = Editor::new(Session::new(48_000), BTreeMap::new());
        let t = e.add_track(
            vec![(0..100).map(|i| i as f32 / 100.0 + 0.01).collect(); 2],
            0,
            None,
        );
        e.process(&[t], 10, 20, "Ganho", |p| {
            p.iter_mut().flatten().for_each(|v| *v *= 2.0)
        })
        .unwrap();
        e.set_script("Olá, pessoal!".into());
        save_session(&e, &path).unwrap();
        let back = load_session(&path).unwrap();
        assert_eq!(back.session(), e.session());
        assert_eq!(back.render(t), e.render(t));
        assert!(!back.is_dirty());
        // Apaga a parte processada: o WAV dela some do disco ao salvar de novo.
        e.delete(&[t], 10, 20).unwrap();
        save_session(&e, &path).unwrap();
        assert_eq!(std::fs::read_dir(audio_dir(&path)).unwrap().count(), 1);
    }

    #[test]
    fn exports_wav_and_mp3() {
        let dir = tempfile::tempdir().unwrap();
        let s: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
        for name in ["a.wav", "a.mp3"] {
            let out = dir.path().join(name);
            export_audio(&s, 48_000, 1, &out).unwrap();
            assert!(std::fs::metadata(&out).unwrap().len() > 1000);
        }
    }
}
