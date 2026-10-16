//! Mixagem ao vivo: quem toca pede o som aos poucos, e uma edição feita no meio do play troca a
//! fonte sem reiniciar nada.

use std::sync::Arc;

/// De onde o player tira o som, bloco a bloco. As posições são em quadros da timeline.
pub trait MixSource: Send + Sync {
    /// `frames` quadros a partir de `from`, intercalados nos canais do player.
    fn mix(&self, from: usize, frames: usize) -> Vec<f32>;
}

/// Fonte que toca um buffer já pronto (intercalado, `channels` canais).
pub struct BufferSource {
    pub buffer: Arc<Vec<f32>>,
    pub channels: usize,
}

impl MixSource for BufferSource {
    fn mix(&self, from: usize, frames: usize) -> Vec<f32> {
        let ch = self.channels.max(1);
        let total = self.buffer.len() / ch;
        let mut out = vec![0.0; frames * ch];
        if from < total {
            let n = frames.min(total - from);
            out[..n * ch].copy_from_slice(&self.buffer[from * ch..(from + n) * ch]);
        }
        out
    }
}

/// Mistura linear de `before` (some) com `after` (aparece) ao longo do trecho, in loco em
/// `after`. Os dois são intercalados em `channels` canais e têm o mesmo tamanho. Serve para
/// passar do som antigo para o novo quando algo muda no meio da reprodução, sem dar um degrau
/// (estalo).
pub fn crossfade(before: &[f32], after: &mut [f32], channels: usize) {
    let ch = channels.max(1);
    let frames = after.len() / ch;
    for (i, (b, a)) in before.chunks(ch).zip(after.chunks_mut(ch)).enumerate() {
        let t = (i as f32 + 0.5) / frames as f32;
        for (b, a) in b.iter().zip(a.iter_mut()) {
            *a = *b * (1.0 - t) + *a * t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossfade_goes_from_old_to_new() {
        let before = vec![1.0f32; 8];
        let mut after = vec![0.0f32; 8];
        crossfade(&before, &mut after, 2);
        assert!(after[0] > 0.8 && after[7] < 0.2);
        assert!(after.windows(2).all(|w| w[0] >= w[1]));
    }

    #[test]
    fn buffer_source_pads_with_silence_past_the_end() {
        let src = BufferSource {
            buffer: Arc::new(vec![1.0, 2.0, 3.0, 4.0]),
            channels: 2,
        };
        assert_eq!(src.mix(1, 3), vec![3.0, 4.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(src.mix(5, 1), vec![0.0, 0.0]);
    }
}
