use serde::{Deserialize, Serialize};

/// Posição ou duração na timeline, em frames do projeto.
pub type Frame = i64;

/// Taxa de quadros racional (ex.: 30/1, 30000/1001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    pub const fn new(num: u32, den: u32) -> Self {
        Self { num, den }
    }

    pub fn as_f64(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    /// Frames por segundo arredondado, usado no timecode (non-drop-frame).
    pub fn nominal(self) -> i64 {
        (self.as_f64().round() as i64).max(1)
    }

    pub fn frames_to_seconds(self, frames: Frame) -> f64 {
        frames as f64 / self.as_f64()
    }

    pub fn seconds_to_frames(self, seconds: f64) -> Frame {
        (seconds * self.as_f64()).round() as Frame
    }

    /// Primeira amostra de áudio que pertence ao frame `frame`.
    pub fn frame_to_sample(self, frame: Frame, sample_rate: u32) -> i64 {
        (i128::from(frame) * i128::from(sample_rate) * i128::from(self.den) / i128::from(self.num))
            as i64
    }
}

/// Formata como `HH:MM:SS:FF`, igual à régua do Resolve.
pub fn format_timecode(frame: Frame, rate: FrameRate) -> String {
    let fps = rate.nominal();
    let sign = if frame < 0 { "-" } else { "" };
    let f = frame.abs();
    let secs = f / fps;
    format!(
        "{sign}{:02}:{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60,
        f % fps
    )
}

/// Lê um tempo digitado: `01:00:05:12` (ou só as partes da direita, `5:12`), dígitos colados
/// como no Resolve (`512` = 5 s 12 fr), `150f` (frames) e relativo a `current` com `+` / `-`.
pub fn parse_timecode(input: &str, rate: FrameRate, current: Frame) -> Option<Frame> {
    let s: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let (sign, body) = match s.chars().next()? {
        '+' => (1, &s[1..]),
        '-' => (-1, &s[1..]),
        _ => (0, s.as_str()),
    };
    if body.is_empty() {
        return None;
    }
    let fps = rate.nominal();
    let value = if let Some(frames) = body.strip_suffix(['f', 'F']) {
        frames.parse::<Frame>().ok()?
    } else {
        let parts: Vec<i64> = if body.contains([':', ';', '.']) {
            body.split([':', ';', '.'])
                .map(|p| {
                    if p.is_empty() {
                        Some(0)
                    } else {
                        p.parse::<i64>().ok()
                    }
                })
                .collect::<Option<_>>()?
        } else {
            if !body.chars().all(|c| c.is_ascii_digit()) || body.len() > 8 {
                return None;
            }
            // Dígitos colados: de dois em dois a partir da direita (FF, SS, MM, HH).
            let padded = format!("{body:0>8}");
            (0..4)
                .map(|i| padded[i * 2..i * 2 + 2].parse::<i64>().unwrap_or(0))
                .collect()
        };
        if parts.len() > 4 {
            return None;
        }
        let mut units = [0i64; 4]; // HH MM SS FF
        for (slot, v) in units.iter_mut().rev().zip(parts.iter().rev()) {
            *slot = *v;
        }
        ((units[0] * 60 + units[1]) * 60 + units[2]) * fps + units[3]
    };
    let out = match sign {
        1 => current + value,
        -1 => current - value,
        _ => value,
    };
    Some(out.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS30: FrameRate = FrameRate::new(30, 1);
    const NTSC: FrameRate = FrameRate::new(30000, 1001);

    #[test]
    fn timecode_formats_hours_minutes_seconds_frames() {
        assert_eq!(format_timecode(0, FPS30), "00:00:00:00");
        assert_eq!(format_timecode(29, FPS30), "00:00:00:29");
        assert_eq!(format_timecode(30, FPS30), "00:00:01:00");
        assert_eq!(format_timecode(30 * 3661 + 5, FPS30), "01:01:01:05");
    }

    #[test]
    fn timecode_uses_nominal_rate_for_fractional_fps() {
        assert_eq!(format_timecode(30, NTSC), "00:00:01:00");
    }

    #[test]
    fn negative_timecode_has_sign() {
        assert_eq!(format_timecode(-31, FPS30), "-00:00:01:01");
    }

    #[test]
    fn converts_between_frames_seconds_and_samples() {
        assert_eq!(FPS30.seconds_to_frames(2.0), 60);
        assert!((FPS30.frames_to_seconds(45) - 1.5).abs() < 1e-9);
        assert_eq!(FPS30.frame_to_sample(30, 48_000), 48_000);
        assert_eq!(NTSC.frame_to_sample(30, 48_000), 48_048);
    }

    #[test]
    fn parses_typed_timecodes() {
        let r = FPS30;
        assert_eq!(parse_timecode("01:00:05:12", r, 0), Some(30 * 3605 + 12));
        assert_eq!(parse_timecode("5:12", r, 0), Some(5 * 30 + 12));
        assert_eq!(parse_timecode("512", r, 0), Some(5 * 30 + 12));
        assert_eq!(parse_timecode("10000", r, 0), Some(60 * 30));
        assert_eq!(parse_timecode("150f", r, 0), Some(150));
        assert_eq!(parse_timecode("+30", r, 100), Some(130));
        assert_eq!(parse_timecode("-1:00", r, 100), Some(70));
        assert_eq!(parse_timecode("-1:00", r, 10), Some(0));
        assert_eq!(parse_timecode("abc", r, 0), None);
        assert_eq!(parse_timecode("", r, 0), None);
    }
}
