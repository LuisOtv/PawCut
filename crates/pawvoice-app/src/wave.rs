//! Forma de onda: picos pré-calculados para desenhar rápido em qualquer zoom.

/// Amostras por balde de pico.
pub const BUCKET: usize = 256;

/// (mín, máx) de cada balde de `BUCKET` amostras.
pub fn peaks(s: &[f32]) -> Vec<(f32, f32)> {
    s.chunks(BUCKET)
        .map(|c| {
            c.iter()
                .fold((0.0f32, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)))
        })
        .collect()
}

/// (mín, máx) do intervalo de amostras `[a, b)`, usando os baldes quando o intervalo é grande.
pub fn range_minmax(samples: &[f32], peaks: &[(f32, f32)], a: usize, b: usize) -> (f32, f32) {
    let b = b.min(samples.len());
    if a >= b {
        return (0.0, 0.0);
    }
    if b - a >= BUCKET * 2 {
        let (pa, pb) = (a / BUCKET, (b / BUCKET).min(peaks.len()));
        peaks[pa..pb.max(pa + 1).min(peaks.len())]
            .iter()
            .fold((0.0, 0.0), |(lo, hi), (l, h)| {
                (f32::min(lo, *l), f32::max(hi, *h))
            })
    } else {
        samples[a..b]
            .iter()
            .fold((0.0f32, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minmax_matches_brute_force() {
        let s: Vec<f32> = (0..5000)
            .map(|i| ((i * 37) % 101) as f32 / 50.0 - 1.0)
            .collect();
        let p = peaks(&s);
        for (a, b) in [(0, 10), (100, 3000), (4000, 5000), (1, 2)] {
            let brute = s[a..b]
                .iter()
                .fold((0.0f32, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            let fast = range_minmax(&s, &p, a, b);
            assert!(fast.0 <= brute.0 + 1e-6 && fast.1 >= brute.1 - 1e-6);
        }
    }
}
