//! Vigia o microfone enquanto grava: segura o pico no medidor, acende o "estourou" e avisa
//! quando a entrada fica muda (microfone errado ou mutado) antes de perder a gravação toda.

/// A partir daqui (−0,1 dBFS) conta como estouro.
pub const CLIP: f32 = 0.989;
/// Abaixo disso (−60 dBFS) a entrada está muda.
const QUIET: f32 = 0.001;
/// Quanto tempo de silêncio total até avisar.
const QUIET_SECS: f64 = 3.0;
/// Quanto tempo o pico fica parado antes de cair.
const HOLD_SECS: f64 = 1.5;

#[derive(Debug, Default)]
pub struct InputWatch {
    clipped: [bool; 2],
    hold: [(f32, f64); 2],
    quiet_since: Option<f64>,
    warned_quiet: bool,
}

impl InputWatch {
    /// Começo de uma gravação: zera o estouro e o aviso de silêncio.
    pub fn start_take(&mut self) {
        *self = Self::default();
    }

    /// Passa os níveis atuais (0..1 por lado). Devolve `true` uma vez por gravação quando a
    /// entrada ficou muda por tempo demais.
    pub fn feed(&mut self, levels: [f32; 2], now: f64, recording: bool) -> bool {
        for (c, &l) in levels.iter().enumerate() {
            if l >= CLIP {
                self.clipped[c] = true;
            }
            let (peak, at) = self.hold[c];
            if l >= peak || now - at > HOLD_SECS {
                self.hold[c] = (l, now);
            }
        }
        if !recording {
            self.quiet_since = None;
            return false;
        }
        if levels.iter().any(|&l| l >= QUIET) {
            self.quiet_since = None;
            return false;
        }
        let since = *self.quiet_since.get_or_insert(now);
        if !self.warned_quiet && now - since >= QUIET_SECS {
            self.warned_quiet = true;
            return true;
        }
        false
    }

    pub fn clipped(&self) -> [bool; 2] {
        self.clipped
    }

    pub fn clear_clip(&mut self) {
        self.clipped = [false; 2];
    }

    /// Pico segurado de cada lado.
    pub fn hold(&self) -> [f32; 2] {
        [self.hold[0].0, self.hold[1].0]
    }
}

/// Quantas amostras de cada canal passaram do limite de estouro.
pub fn clipped_samples(planar: &[Vec<f32>]) -> usize {
    planar
        .iter()
        .map(|c| c.iter().filter(|v| v.abs() >= CLIP).count())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_once_after_three_quiet_seconds_while_recording() {
        let mut w = InputWatch::default();
        let mut warnings = 0;
        for i in 0..600 {
            if w.feed([0.0, 0.0], i as f64 / 60.0, true) {
                warnings += 1;
                assert!(i as f64 / 60.0 >= 3.0);
            }
        }
        assert_eq!(warnings, 1);
    }

    #[test]
    fn speech_resets_the_quiet_timer_and_monitoring_never_warns() {
        let mut w = InputWatch::default();
        for i in 0..600 {
            let t = i as f64 / 60.0;
            // fala a cada 2 s
            let l = if i % 120 == 0 { 0.2 } else { 0.0 };
            assert!(!w.feed([l, l], t, true));
            assert!(!w.feed([0.0, 0.0], t, false));
        }
    }

    #[test]
    fn clip_latches_until_cleared_and_new_take() {
        let mut w = InputWatch::default();
        w.feed([0.5, 1.0], 0.0, true);
        w.feed([0.1, 0.1], 1.0, true);
        assert_eq!(w.clipped(), [false, true]);
        w.clear_clip();
        assert_eq!(w.clipped(), [false, false]);
        w.feed([1.0, 0.0], 2.0, true);
        w.start_take();
        assert_eq!(w.clipped(), [false, false]);
    }

    #[test]
    fn peak_holds_then_falls() {
        let mut w = InputWatch::default();
        w.feed([0.8, 0.8], 0.0, false);
        w.feed([0.1, 0.1], 1.0, false);
        assert_eq!(w.hold(), [0.8, 0.8]);
        w.feed([0.1, 0.1], 2.0, false);
        assert_eq!(w.hold(), [0.1, 0.1]);
    }

    #[test]
    fn counts_clipped_samples() {
        assert_eq!(clipped_samples(&[vec![0.0, 1.0, -0.995], vec![0.5]]), 2);
    }
}
