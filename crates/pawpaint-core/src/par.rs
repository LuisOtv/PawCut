//! Divide o trabalho de uma imagem grande entre os núcleos, sem dependências: cada thread recebe
//! um pedaço próprio do vetor (linhas inteiras), então não há trava nem `unsafe`.

/// Abaixo disso o custo de criar threads supera o ganho.
const MIN_PARALLEL: usize = 200_000;

pub fn workers_for(len: usize) -> usize {
    if len < MIN_PARALLEL {
        1
    } else {
        std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .clamp(1, 8)
    }
}

/// Roda `f(primeira_linha, pedaço)` em vários pedaços de `data`, cortados em múltiplos de
/// `row_len` elementos (use 1 para trabalhar pixel a pixel). `f` pode ler dados compartilhados.
pub fn rows<T: Send>(data: &mut [T], row_len: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    rows_with(workers_for(data.len()), data, row_len, f);
}

/// Como `rows`, mas com o número de threads dado (quando cada elemento é bem mais pesado que um
/// número, como um bloco de 64×64 pixels).
pub fn rows_with<T: Send>(
    workers: usize,
    data: &mut [T],
    row_len: usize,
    f: impl Fn(usize, &mut [T]) + Sync,
) {
    let row_len = row_len.max(1);
    let n = data.len() / row_len;
    if workers <= 1 || n < 2 {
        f(0, data);
        return;
    }
    let per = n.div_ceil(workers);
    std::thread::scope(|scope| {
        for (i, chunk) in data.chunks_mut(per * row_len).enumerate() {
            let f = &f;
            scope.spawn(move || f(i * per, chunk));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_element_is_visited_once_with_the_right_row_offset() {
        for (w, h) in [(10, 7), (1000, 300), (3, 100_000)] {
            let mut v = vec![0usize; w * h];
            rows(&mut v, w, |row0, chunk| {
                for (i, row) in chunk.chunks_mut(w).enumerate() {
                    for x in row.iter_mut() {
                        *x += row0 + i + 1;
                    }
                }
            });
            for y in 0..h {
                assert_eq!(v[y * w], y + 1, "{w}x{h} linha {y}");
                assert_eq!(v[y * w + w - 1], y + 1);
            }
        }
        // Pixel a pixel (row_len 1): o deslocamento é o índice do primeiro elemento.
        let mut v = vec![0usize; 500_000];
        rows(&mut v, 1, |first, chunk| {
            for (i, x) in chunk.iter_mut().enumerate() {
                *x = first + i;
            }
        });
        assert!(v.iter().enumerate().all(|(i, x)| i == *x));
    }
}
