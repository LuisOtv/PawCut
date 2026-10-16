//! PawWrite: o que um roteiro de vídeo é. O texto é Markdown simples com três regras:
//! `# Título` abre uma seção, `[entre colchetes]` é nota de edição (B-roll, texto na tela) e
//! `// linha` é comentário. Só o resto é fala, e é ele que conta para o tempo do vídeo.

use std::ops::Range;

/// Palavras por minuto de uma narração tranquila (o padrão do app).
pub const DEFAULT_WPM: f32 = 150.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Linha de título (`#`, `##`…), com o nível.
    Heading(u8),
    /// `[nota]` no meio da linha.
    Cue,
    /// Linha que começa com `//`.
    Comment,
    /// Fala.
    Spoken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: Kind,
}

/// Divide o texto em trechos contíguos (cobrem tudo, inclusive as quebras de linha), para
/// pintar o editor e para contar só a fala.
pub fn spans(text: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut push = |start: usize, end: usize, kind: Kind| {
        if end <= start {
            return;
        }
        match out.last_mut() {
            Some(last) if last.kind == kind && last.range.end == start => last.range.end = end,
            _ => out.push(Span {
                range: start..end,
                kind,
            }),
        }
    };
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let end = pos + line.len();
        let body = line.trim_end_matches(['\n', '\r']);
        let trimmed = body.trim_start();
        if trimmed.starts_with("//") {
            push(pos, end, Kind::Comment);
        } else if let Some(level) = heading_level(trimmed) {
            push(pos, end, Kind::Heading(level));
        } else {
            let mut at = 0;
            while let Some(open) = body[at..].find('[') {
                let open = at + open;
                let Some(close) = body[open..].find(']') else {
                    break;
                };
                let close = open + close + 1;
                push(pos + at, pos + open, Kind::Spoken);
                push(pos + open, pos + close, Kind::Cue);
                at = close;
            }
            push(pos + at, end, Kind::Spoken);
        }
        pos = end;
    }
    out
}

/// `# x` → 1, `## x` → 2… (precisa de espaço depois dos `#`, como no Markdown).
fn heading_level(line: &str) -> Option<u8> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    let rest = &line[hashes..];
    ((1..=6).contains(&hashes) && (rest.is_empty() || rest.starts_with(' ')))
        .then_some(hashes as u8)
}

/// Palavra = pedaço separado por espaço que tem alguma letra ou número ("—" sozinho não conta).
pub fn count_words(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count()
}

/// Uma seção do roteiro: do título até o próximo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub level: u8,
    /// Posição (em bytes) do começo do título no texto.
    pub start: usize,
    /// Palavras faladas na seção.
    pub words: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Analysis {
    /// A abertura antes do primeiro título aparece como seção sem título.
    pub sections: Vec<Section>,
    pub words: usize,
    /// Palavras dentro de `[notas]`: não entram no tempo.
    pub cue_words: usize,
}

pub fn analyze(text: &str) -> Analysis {
    let mut a = Analysis::default();
    let mut current = Section {
        title: String::new(),
        level: 0,
        start: 0,
        words: 0,
    };
    for span in spans(text) {
        let s = &text[span.range.clone()];
        match span.kind {
            Kind::Heading(level) => {
                if current.level > 0 || current.words > 0 {
                    a.sections.push(current);
                }
                current = Section {
                    title: s.trim().trim_start_matches('#').trim().to_string(),
                    level,
                    start: span.range.start,
                    words: 0,
                };
            }
            Kind::Spoken => {
                let n = count_words(s);
                current.words += n;
                a.words += n;
            }
            Kind::Cue => a.cue_words += count_words(s),
            Kind::Comment => {}
        }
    }
    if current.level > 0 || current.words > 0 {
        a.sections.push(current);
    }
    a
}

/// Só a fala: sem títulos, notas e comentários, com as linhas em branco repetidas juntas.
/// É o que vai para o teleprompter do PawVoice.
pub fn spoken(text: &str) -> String {
    let mut out = String::new();
    for span in spans(text) {
        if span.kind == Kind::Spoken {
            out.push_str(&text[span.range]);
        }
    }
    let mut lines: Vec<String> = Vec::new();
    let mut blank = true;
    for line in out.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            if !blank {
                lines.push(String::new());
            }
            blank = true;
        } else {
            lines.push(line);
            blank = false;
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

/// Intervalo (em bytes) do bloco da seção `i`: do título até o próximo título do mesmo nível
/// ou de nível mais alto (as subseções vêm junto).
fn block_range(a: &Analysis, i: usize, len: usize) -> Option<Range<usize>> {
    let s = a.sections.get(i)?;
    if s.level == 0 {
        return None;
    }
    let end = a.sections[i + 1..]
        .iter()
        .find(|n| n.level <= s.level)
        .map_or(len, |n| n.start);
    Some(s.start..end)
}

/// Troca a seção `i` de lugar com a vizinha de cima (`up`) ou de baixo, levando as subseções.
/// Devolve o texto novo e a posição (em bytes) onde a seção ficou.
pub fn move_section(text: &str, i: usize, up: bool) -> Option<(String, usize)> {
    let a = analyze(text);
    let me = block_range(&a, i, text.len())?;
    let level = a.sections[i].level;
    let (first, second) = if up {
        let prev = (0..i)
            .rev()
            .take_while(|k| a.sections[*k].level >= level || a.sections[*k].level == 0)
            .find(|k| a.sections[*k].level == level)?;
        (block_range(&a, prev, text.len())?, me)
    } else {
        let next = a.sections[i + 1..]
            .iter()
            .position(|n| n.level <= level)
            .map(|p| p + i + 1)
            .filter(|k| a.sections[*k].level == level)?;
        (me, block_range(&a, next, text.len())?)
    };
    let fix = |r: Range<usize>| {
        let t = &text[r];
        if t.ends_with('\n') {
            t.to_string()
        } else {
            format!("{t}\n")
        }
    };
    let (x, y) = (fix(first.clone()), fix(second.clone()));
    let mut out = String::with_capacity(text.len() + 2);
    out.push_str(&text[..first.start]);
    out.push_str(&y);
    let landed = if up {
        first.start
    } else {
        first.start + y.len()
    };
    out.push_str(&x);
    out.push_str(&text[second.end..]);
    Some((out, landed))
}

/// Tempo de fala de `words` palavras a `wpm` palavras por minuto, em segundos.
pub fn seconds(words: usize, wpm: f32) -> f32 {
    words as f32 / wpm.max(1.0) * 60.0
}

/// "45 s", "3 min 20 s", "1 h 05 min".
pub fn format_duration(secs: f32) -> String {
    let total = secs.round().max(0.0) as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h} h {m:02} min")
    } else if m > 0 {
        format!("{m} min {s:02} s")
    } else {
        format!("{s} s")
    }
}

/// Posição (em bytes) da próxima ocorrência de `needle` a partir de `from`, sem diferenciar
/// maiúsculas de minúsculas. Dá a volta ao chegar no fim.
pub fn find_next(text: &str, needle: &str, from: usize) -> Option<Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    let (hay, pat) = (text.to_lowercase(), needle.to_lowercase());
    // `to_lowercase` pode mudar o tamanho de algumas letras; sem isso os índices batem.
    if hay.len() != text.len() {
        return text
            .match_indices(needle)
            .find(|(i, _)| *i >= from)
            .or_else(|| text.match_indices(needle).next())
            .map(|(i, m)| i..i + m.len());
    }
    let from = from.min(hay.len());
    let from = (from..=hay.len())
        .find(|i| hay.is_char_boundary(*i))
        .unwrap_or(hay.len());
    hay[from..]
        .find(&pat)
        .map(|i| i + from)
        .or_else(|| hay.find(&pat))
        .map(|i| i..i + pat.len())
}

/// Troca todas as ocorrências (sem diferenciar maiúsculas). Devolve o texto novo e quantas trocas.
pub fn replace_all(text: &str, needle: &str, with: &str) -> (String, usize) {
    if needle.is_empty() {
        return (text.to_string(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    let mut n = 0;
    while let Some(r) = find_next(text, needle, at) {
        if r.start < at {
            break; // deu a volta
        }
        out.push_str(&text[at..r.start]);
        out.push_str(with);
        at = r.end;
        n += 1;
    }
    out.push_str(&text[at..]);
    (out, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "Olá, pessoal!\n\n# Introdução\nHoje eu vou mostrar [B-roll: mesa] três truques.\n// lembrar de pedir like\n\n## Truque 1\nAbra o programa — fácil.\n[tela: menu Arquivo]\n\n# Final\nTchau!\n";

    #[test]
    fn spans_cover_everything_and_classify() {
        let sp = spans(SCRIPT);
        assert_eq!(sp.first().unwrap().range.start, 0);
        assert_eq!(sp.last().unwrap().range.end, SCRIPT.len());
        for w in sp.windows(2) {
            assert_eq!(w[0].range.end, w[1].range.start);
        }
        let kinds: Vec<Kind> = sp.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&Kind::Heading(1)));
        assert!(kinds.contains(&Kind::Heading(2)));
        assert!(kinds.contains(&Kind::Cue));
        assert!(kinds.contains(&Kind::Comment));
        let cue = sp.iter().find(|s| s.kind == Kind::Cue).unwrap();
        assert_eq!(&SCRIPT[cue.range.clone()], "[B-roll: mesa]");
    }

    #[test]
    fn analysis_counts_only_speech_per_section() {
        let a = analyze(SCRIPT);
        let titles: Vec<(&str, u8, usize)> = a
            .sections
            .iter()
            .map(|s| (s.title.as_str(), s.level, s.words))
            .collect();
        assert_eq!(
            titles,
            vec![
                ("", 0, 2),
                ("Introdução", 1, 6),
                ("Truque 1", 2, 4),
                ("Final", 1, 1)
            ]
        );
        assert_eq!(a.words, 13);
        assert_eq!(a.cue_words, 5);
        assert!(SCRIPT[a.sections[1].start..].starts_with("# Introdução"));
    }

    #[test]
    fn unclosed_bracket_and_hashtags_stay_spoken() {
        let a = analyze("Isso é [um erro de digitação\n#hashtag não é título\n");
        assert_eq!(a.words, 10);
        assert_eq!(a.cue_words, 0);
        assert!(a.sections.iter().all(|s| s.level == 0));
    }

    #[test]
    fn spoken_text_is_clean() {
        assert_eq!(
            spoken(SCRIPT),
            "Olá, pessoal!\n\nHoje eu vou mostrar três truques.\n\nAbra o programa — fácil.\n\nTchau!"
        );
        assert_eq!(spoken(""), "");
    }

    #[test]
    fn sections_move_with_their_subsections() {
        let t = "# A\num\n## A1\ndois\n# B\ntres\n# C\nquatro\n";
        let (down, at) = move_section(t, 0, false).unwrap();
        assert_eq!(down, "# B\ntres\n# A\num\n## A1\ndois\n# C\nquatro\n");
        assert!(down[at..].starts_with("# A"));
        let (up, _) = move_section(t, 3, true).unwrap();
        assert_eq!(up, "# A\num\n## A1\ndois\n# C\nquatro\n# B\ntres\n");
        assert!(move_section(t, 0, true).is_none());
        assert!(move_section(t, 3, false).is_none());
        assert!(
            move_section(t, 1, false).is_none(),
            "A1 não tem irmã abaixo"
        );
    }

    #[test]
    fn timing() {
        assert_eq!(seconds(150, 150.0), 60.0);
        assert_eq!(format_duration(45.0), "45 s");
        assert_eq!(format_duration(200.0), "3 min 20 s");
        assert_eq!(format_duration(3900.0), "1 h 05 min");
        assert_eq!(count_words("— ... oi, mundo!"), 2);
    }

    #[test]
    fn find_and_replace_ignore_case_and_wrap() {
        let t = "Gato, gato e GATO.";
        assert_eq!(find_next(t, "gato", 0), Some(0..4));
        assert_eq!(find_next(t, "gato", 1), Some(6..10));
        assert_eq!(find_next(t, "gato", 11), Some(13..17));
        assert_eq!(find_next(t, "gato", 17), Some(0..4));
        assert_eq!(find_next(t, "cão", 0), None);
        let (out, n) = replace_all(t, "gato", "cão");
        assert_eq!((out.as_str(), n), ("cão, cão e cão.", 3));
        assert_eq!(replace_all("áé", "é", "e").0, "áe");
    }
}
