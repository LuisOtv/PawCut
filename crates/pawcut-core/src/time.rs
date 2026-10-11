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
}
