//! Legendas SRT (e WebVTT na leitura): tempos em frames do projeto.

use crate::time::{Frame, FrameRate};

/// Uma legenda: aparece em `[start, end)` com `text` (pode ter várias linhas).
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub start: Frame,
    pub end: Frame,
    pub text: String,
}

/// `HH:MM:SS,mmm` (ou `.mmm`, ou `MM:SS.mmm` do WebVTT) em segundos.
fn parse_time(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m, sec] => (
            h.parse::<f64>().ok()?,
            m.parse::<f64>().ok()?,
            sec.parse::<f64>().ok()?,
        ),
        [m, sec] => (0.0, m.parse::<f64>().ok()?, sec.parse::<f64>().ok()?),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// Lê um SRT ou WebVTT. Blocos que não entendeu são pulados; erro só se nada for lido.
pub fn parse_subtitles(text: &str, fps: FrameRate) -> Result<Vec<Cue>, String> {
    let text = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut cues = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        let Some(i) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let mut times = lines[i].split("-->");
        let (Some(a), Some(b)) = (times.next(), times.next()) else {
            continue;
        };
        // No WebVTT pode vir "00:01.000 --> 00:02.000 align:start": só o primeiro pedaço.
        let b = b.split_whitespace().next().unwrap_or("");
        let (Some(a), Some(b)) = (parse_time(a), parse_time(b)) else {
            continue;
        };
        let body = lines[i + 1..].join("\n");
        if body.trim().is_empty() || b <= a {
            continue;
        }
        cues.push(Cue {
            start: fps.seconds_to_frames(a),
            end: fps.seconds_to_frames(b).max(fps.seconds_to_frames(a) + 1),
            text: body,
        });
    }
    if cues.is_empty() {
        return Err("nenhuma legenda encontrada (o arquivo é SRT ou VTT?)".into());
    }
    cues.sort_by_key(|c| c.start);
    Ok(cues)
}

fn srt_time(frame: Frame, fps: FrameRate) -> String {
    let ms = (fps.frames_to_seconds(frame.max(0)) * 1000.0).round() as i64;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// Escreve as legendas como SRT.
pub fn write_srt(cues: &[Cue], fps: FrameRate) -> String {
    let mut out = String::new();
    for (i, c) in cues.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            srt_time(c.start, fps),
            srt_time(c.end, fps),
            c.text.trim()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS: FrameRate = FrameRate::new(30, 1);

    #[test]
    fn reads_srt_and_writes_it_back() {
        let srt = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,500\r\nOlá\r\nmundo\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\nTchau\r\n";
        let cues = parse_subtitles(srt, FPS).unwrap();
        assert_eq!(
            cues,
            vec![
                Cue {
                    start: 30,
                    end: 75,
                    text: "Olá\nmundo".into()
                },
                Cue {
                    start: 90,
                    end: 120,
                    text: "Tchau".into()
                },
            ]
        );
        let back = write_srt(&cues, FPS);
        assert!(back.starts_with("1\n00:00:01,000 --> 00:00:02,500\nOlá\nmundo\n\n2\n"));
        assert_eq!(parse_subtitles(&back, FPS).unwrap(), cues);
    }

    #[test]
    fn reads_webvtt_and_skips_garbage() {
        let vtt = "WEBVTT\n\nNOTE comentário\n\n00:01.000 --> 00:02.000 align:start\nOi\n\nlixo --> lixo\nx\n";
        let cues = parse_subtitles(vtt, FPS).unwrap();
        assert_eq!(
            cues,
            vec![Cue {
                start: 30,
                end: 60,
                text: "Oi".into()
            }]
        );
        assert!(parse_subtitles("nada aqui", FPS).is_err());
    }
}
