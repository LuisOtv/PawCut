//! Arquivos de mídia sintéticos para os testes, gerados com o `ffmpeg` da máquina.

use std::path::{Path, PathBuf};
use std::process::Command;

fn ffmpeg(args: &[&str]) {
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(args)
        .status()
        .expect("o ffmpeg precisa estar instalado para os testes");
    assert!(status.success(), "ffmpeg falhou: {args:?}");
}

/// Vídeo 64x36 a 30 fps em que o frame N tem cinza `expected_gray(N)`.
/// Keyframe a cada 10 frames. Com `with_audio`, leva um áudio AAC constante em 0.5.
pub fn gray_ramp_video(dir: &Path, secs: u32, with_audio: bool) -> PathBuf {
    let name = if with_audio { "ramp_av.mp4" } else { "ramp.mp4" };
    let out = dir.join(name);
    let video = format!(
        "nullsrc=s=64x36:r=30:d={secs},format=yuv420p,geq=lum='16+7*mod(N\\,30)':cb=128:cr=128"
    );
    let audio = format!("aevalsrc=0.5|0.5:s=48000:d={secs}");
    let mut args = vec!["-f", "lavfi", "-i", &video];
    if with_audio {
        args.extend(["-f", "lavfi", "-i", &audio, "-c:a", "aac", "-b:a", "128k"]);
    }
    let out_s = out.to_str().expect("caminho utf-8");
    args.extend(["-c:v", "libx264", "-qp", "0", "-g", "10", "-pix_fmt", "yuv420p", out_s]);
    ffmpeg(&args);
    out
}

/// Cinza (0..255) esperado no frame N do `gray_ramp_video`.
pub fn expected_gray(frame: i64) -> u8 {
    ((7 * frame.rem_euclid(30)) as f64 * 255.0 / 219.0).round().min(255.0) as u8
}

/// Vídeo de cor sólida (`color` no formato do ffmpeg, ex.: "0x3366CC").
pub fn solid_video(dir: &Path, name: &str, color: &str, w: u32, h: u32, secs: u32) -> PathBuf {
    let out = dir.join(name);
    let src = format!("color=c={color}:s={w}x{h}:r=30:d={secs}");
    ffmpeg(&["-f", "lavfi", "-i", &src, "-c:v", "libx264", "-pix_fmt", "yuv420p", out.to_str().unwrap()]);
    out
}

/// WAV estéreo 48 kHz float com valor constante `level`.
pub fn dc_audio(dir: &Path, name: &str, level: f32, secs: f32) -> PathBuf {
    let out = dir.join(name);
    let src = format!("aevalsrc={level}|{level}:s=48000:d={secs}");
    ffmpeg(&["-f", "lavfi", "-i", &src, "-c:a", "pcm_f32le", out.to_str().unwrap()]);
    out
}

/// PNG opaco de cor sólida.
pub fn png(dir: &Path, name: &str, color: &str, w: u32, h: u32) -> PathBuf {
    let out = dir.join(name);
    let src = format!("color=c={color}:s={w}x{h}");
    ffmpeg(&["-f", "lavfi", "-i", &src, "-frames:v", "1", out.to_str().unwrap()]);
    out
}

/// MP3 com capa de álbum (stream de vídeo marcado como attached_pic).
pub fn mp3_with_cover(dir: &Path) -> PathBuf {
    let out = dir.join("musica.mp3");
    ffmpeg(&[
        "-f", "lavfi", "-i", "sine=frequency=440:duration=1",
        "-f", "lavfi", "-i", "color=c=red:s=16x16",
        "-map", "0:a", "-map", "1:v", "-frames:v", "1",
        "-c:a", "libmp3lame", "-c:v", "mjpeg", "-disposition:v:0", "attached_pic",
        "-id3v2_version", "3", out.to_str().unwrap(),
    ]);
    out
}

/// Saída do `ffprobe -show_streams -show_format` em JSON.
pub fn ffprobe(path: &Path) -> serde_json::Value {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-show_format", "-of", "json"])
        .arg(path)
        .output()
        .expect("ffprobe instalado");
    assert!(out.status.success(), "ffprobe falhou em {}", path.display());
    serde_json::from_slice(&out.stdout).expect("json do ffprobe")
}

/// Um frame do arquivo em `seconds`, como RGBA `w`x`h`.
pub fn grab_frame(path: &Path, seconds: f64, w: u32, h: u32) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &seconds.to_string(), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-"])
        .output()
        .expect("ffmpeg instalado");
    assert!(out.status.success(), "ffmpeg não extraiu o frame");
    out.stdout
}

/// Pixel RGBA (x, y) de um buffer `w` de largura.
pub fn pixel(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}
