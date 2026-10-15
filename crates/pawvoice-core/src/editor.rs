//! Edição de uma sessão com desfazer/refazer, no estilo do Audacity: as operações agem num
//! intervalo de tempo `[a, b)` das faixas selecionadas.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use thiserror::Error;

use crate::{SILENCE, Segment, Session, SourceId, Track};

#[derive(Debug, Clone, PartialEq, Error)]
pub enum EditError {
    #[error("a faixa não existe")]
    NoTrack,
    #[error("nada selecionado")]
    EmptyRange,
    #[error("a área de transferência está vazia")]
    EmptyClipboard,
}

/// Sessão + fontes de áudio + histórico + área de transferência.
pub struct Editor {
    session: Session,
    sources: BTreeMap<SourceId, Arc<Vec<f32>>>,
    undo: Vec<(Session, &'static str)>,
    redo: Vec<(Session, &'static str)>,
    /// Um trecho por faixa copiada.
    clipboard: Vec<Vec<Segment>>,
    revision: u64,
    saved_revision: u64,
}

const HISTORY: usize = 200;

/// Os trechos que cobrem `[a, b)` da sequência (com silêncio se passar do fim).
pub(crate) fn slice(segs: &[Segment], a: usize, b: usize) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut pos = 0;
    for s in segs {
        let (s0, s1) = (pos, pos + s.len());
        pos = s1;
        let (lo, hi) = (a.max(s0), b.min(s1));
        if lo < hi {
            out.push(Segment {
                source: s.source,
                start: s.start + (lo - s0),
                end: s.start + (hi - s0),
            });
        }
    }
    let have: usize = out.iter().map(Segment::len).sum();
    if b > a && have < b - a {
        out.push(Segment::silence(b - a - have));
    }
    out
}

/// Junta trechos vizinhos que continuam um ao outro e tira o silêncio do fim.
pub(crate) fn merge(segs: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::with_capacity(segs.len());
    for s in segs.into_iter().filter(|s| !s.is_empty()) {
        match out.last_mut() {
            Some(l) if l.source == SILENCE && s.source == SILENCE => l.end += s.len(),
            Some(l) if l.source == s.source && l.end == s.start => l.end = s.end,
            _ => out.push(s),
        }
    }
    while out.last().is_some_and(|s| s.source == SILENCE) {
        out.pop();
    }
    out
}

fn total(segs: &[Segment]) -> usize {
    segs.iter().map(Segment::len).sum()
}

/// Troca `[a, a + len(new))` por `new` (sobrescreve, estendendo com silêncio se preciso).
fn overwrite(segs: &[Segment], a: usize, new: &[Segment]) -> Vec<Segment> {
    let b = a + total(new);
    let mut out = slice(segs, 0, a);
    out.extend_from_slice(new);
    let len = total(segs);
    if len > b {
        out.extend(slice(segs, b, len));
    }
    merge(out)
}

/// Apaga `[a, b)` e puxa o resto para a esquerda.
fn ripple_delete(segs: &[Segment], a: usize, b: usize) -> Vec<Segment> {
    let len = total(segs);
    if a >= len {
        return segs.to_vec();
    }
    let mut out = slice(segs, 0, a);
    out.extend(slice(segs, b.min(len), len));
    merge(out)
}

/// Insere `new` em `at`, empurrando o resto para a direita.
fn insert(segs: &[Segment], at: usize, new: &[Segment]) -> Vec<Segment> {
    let len = total(segs);
    let mut out = slice(segs, 0, at);
    out.extend_from_slice(new);
    if len > at {
        out.extend(slice(segs, at, len));
    }
    merge(out)
}

impl Editor {
    pub fn new(session: Session, sources: BTreeMap<SourceId, Arc<Vec<f32>>>) -> Self {
        Self {
            session,
            sources,
            undo: Vec::new(),
            redo: Vec::new(),
            clipboard: Vec::new(),
            revision: 0,
            saved_revision: 0,
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn sources(&self) -> &BTreeMap<SourceId, Arc<Vec<f32>>> {
        &self.sources
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(|(_, l)| *l)
    }

    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|(_, l)| *l)
    }

    pub fn has_clipboard(&self) -> bool {
        !self.clipboard.is_empty()
    }

    fn checkpoint(&mut self, label: &'static str) {
        self.undo.push((self.session.clone(), label));
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.revision += 1;
    }

    pub fn undo(&mut self) -> bool {
        let Some((prev, label)) = self.undo.pop() else {
            return false;
        };
        self.redo
            .push((std::mem::replace(&mut self.session, prev), label));
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some((next, label)) = self.redo.pop() else {
            return false;
        };
        self.undo
            .push((std::mem::replace(&mut self.session, next), label));
        self.revision += 1;
        true
    }

    /// Esquece fontes que nem a sessão, nem o histórico, nem a área de transferência usam.
    fn prune(&mut self) {
        let mut used: HashSet<SourceId> = HashSet::new();
        for s in std::iter::once(&self.session)
            .chain(self.undo.iter().map(|(s, _)| s))
            .chain(self.redo.iter().map(|(s, _)| s))
        {
            for t in &s.tracks {
                used.extend(t.segments.iter().map(|g| g.source));
            }
        }
        used.extend(self.clipboard.iter().flatten().map(|g| g.source));
        self.sources.retain(|id, _| used.contains(id));
    }

    fn track_mut(&mut self, id: u64) -> Result<&mut Track, EditError> {
        self.session
            .tracks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or(EditError::NoTrack)
    }

    fn check(&self, tracks: &[u64], a: usize, b: usize) -> Result<(), EditError> {
        if tracks.is_empty() || a >= b {
            return Err(EditError::EmptyRange);
        }
        for t in tracks {
            self.session.track(*t).ok_or(EditError::NoTrack)?;
        }
        Ok(())
    }

    /// Aplica `f` nos trechos de cada faixa (um passo de desfazer).
    fn edit_tracks(
        &mut self,
        tracks: &[u64],
        label: &'static str,
        mut f: impl FnMut(&[Segment]) -> Vec<Segment>,
    ) -> Result<(), EditError> {
        self.checkpoint(label);
        for id in tracks {
            let t = self.track_mut(*id)?;
            t.segments = merge(f(&t.segments));
        }
        self.prune();
        Ok(())
    }

    pub fn set_script(&mut self, text: String) {
        // O roteiro não entra no desfazer (é texto livre, editado o tempo todo).
        if self.session.script != text {
            self.session.script = text;
            self.revision += 1;
        }
    }

    fn add_source(&mut self, samples: Vec<f32>) -> Segment {
        let source = self.session.fresh();
        let len = samples.len();
        self.sources.insert(source, Arc::new(samples));
        Segment {
            source,
            start: 0,
            end: len,
        }
    }

    /// Faixa nova com `samples` começando em `at`. Devolve o id.
    pub fn add_track(&mut self, samples: Vec<f32>, at: usize, name: Option<String>) -> u64 {
        self.checkpoint("Nova faixa");
        let seg = self.add_source(samples);
        let id = self.session.fresh();
        let n = self.session.tracks.len() + 1;
        self.session.tracks.push(Track::new(
            id,
            name.unwrap_or_else(|| format!("Áudio {n}")),
            merge(vec![Segment::silence(at), seg]),
        ));
        id
    }

    /// Grava `samples` por cima da faixa a partir de `at`.
    pub fn write_at(&mut self, track: u64, at: usize, samples: Vec<f32>) -> Result<(), EditError> {
        self.session.track(track).ok_or(EditError::NoTrack)?;
        self.checkpoint("Gravar");
        let seg = self.add_source(samples);
        let t = self.track_mut(track)?;
        t.segments = overwrite(&t.segments, at, &[seg]);
        Ok(())
    }

    pub fn remove_track(&mut self, id: u64) -> Result<(), EditError> {
        self.session.track(id).ok_or(EditError::NoTrack)?;
        self.checkpoint("Apagar faixa");
        self.session.tracks.retain(|t| t.id != id);
        self.prune();
        Ok(())
    }

    pub fn rename_track(&mut self, id: u64, name: String) -> Result<(), EditError> {
        if self.session.track(id).ok_or(EditError::NoTrack)?.name == name {
            return Ok(());
        }
        self.checkpoint("Renomear faixa");
        self.track_mut(id)?.name = name;
        Ok(())
    }

    /// Ganho, pan, mudo e solo (fora do desfazer, como mexer num mixer).
    pub fn set_mix(&mut self, id: u64, gain_db: f32, pan: f32, mute: bool, solo: bool) {
        if let Ok(t) = self.track_mut(id) {
            let changed = (t.gain_db, t.pan, t.mute, t.solo) != (gain_db, pan, mute, solo);
            t.gain_db = gain_db.clamp(-36.0, 36.0);
            t.pan = pan.clamp(-1.0, 1.0);
            t.mute = mute;
            t.solo = solo;
            if changed {
                self.revision += 1;
            }
        }
    }

    /// Delete: apaga e puxa o resto (em cada faixa).
    pub fn delete(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<(), EditError> {
        self.check(tracks, a, b)?;
        self.edit_tracks(tracks, "Apagar", |s| ripple_delete(s, a, b))
    }

    /// Apaga vários trechos de uma faixa num passo (ex.: silêncios). Devolve quanto saiu.
    pub fn delete_many(
        &mut self,
        track: u64,
        ranges: &[(usize, usize)],
    ) -> Result<usize, EditError> {
        let mut ranges: Vec<(usize, usize)> =
            ranges.iter().copied().filter(|(a, b)| a < b).collect();
        if ranges.is_empty() {
            return Err(EditError::EmptyRange);
        }
        ranges.sort();
        let mut removed = 0;
        self.edit_tracks(&[track], "Remover silêncios", |s| {
            let mut cur = s.to_vec();
            // Do fim para o começo: os índices do que falta não mudam.
            for (a, b) in ranges.iter().rev() {
                let before = total(&cur);
                cur = ripple_delete(&cur, *a, *b);
                removed += before - total(&cur);
            }
            cur
        })?;
        Ok(removed)
    }

    /// Silenciar (Ctrl+L).
    pub fn silence(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<(), EditError> {
        self.check(tracks, a, b)?;
        self.edit_tracks(tracks, "Silenciar", |s| {
            overwrite(s, a, &[Segment::silence(b - a)])
        })
    }

    /// Aparar (Ctrl+T): tudo fora de `[a, b)` vira silêncio; o que fica não muda de lugar.
    pub fn trim(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<(), EditError> {
        self.check(tracks, a, b)?;
        self.edit_tracks(tracks, "Aparar", |s| {
            let mut out = vec![Segment::silence(a)];
            out.extend(slice(s, a, b));
            out
        })
    }

    pub fn copy(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<(), EditError> {
        self.check(tracks, a, b)?;
        self.clipboard = tracks
            .iter()
            .filter_map(|id| self.session.track(*id))
            .map(|t| slice(&t.segments, a, b))
            .collect();
        Ok(())
    }

    pub fn cut(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<(), EditError> {
        self.copy(tracks, a, b)?;
        self.check(tracks, a, b)?;
        self.edit_tracks(tracks, "Recortar", |s| ripple_delete(s, a, b))
    }

    /// Cola em `at` (inserindo) a partir de `first`; faltando faixas, cria novas.
    /// Devolve as faixas que receberam e o tamanho colado.
    pub fn paste(&mut self, first: Option<u64>, at: usize) -> Result<(Vec<u64>, usize), EditError> {
        if self.clipboard.is_empty() {
            return Err(EditError::EmptyClipboard);
        }
        let clip = self.clipboard.clone();
        let len = clip.iter().map(|c| total(c)).max().unwrap_or(0);
        self.checkpoint("Colar");
        let start = first
            .and_then(|f| self.session.tracks.iter().position(|t| t.id == f))
            .unwrap_or(self.session.tracks.len());
        let mut targets = Vec::new();
        for (i, segs) in clip.iter().enumerate() {
            let id = match self.session.tracks.get(start + i) {
                Some(t) => t.id,
                None => {
                    let id = self.session.fresh();
                    let n = self.session.tracks.len() + 1;
                    self.session
                        .tracks
                        .push(Track::new(id, format!("Áudio {n}"), Vec::new()));
                    id
                }
            };
            let t = self.track_mut(id)?;
            t.segments = insert(&t.segments, at, segs);
            targets.push(id);
        }
        Ok((targets, len))
    }

    /// Duplicar (Ctrl+D): a seleção vai para faixas novas, no mesmo tempo.
    pub fn duplicate(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<Vec<u64>, EditError> {
        self.check(tracks, a, b)?;
        self.checkpoint("Duplicar");
        let mut new = Vec::new();
        for id in tracks {
            let Some(t) = self.session.track(*id) else {
                continue;
            };
            let mut segs = vec![Segment::silence(a)];
            segs.extend(slice(&t.segments, a, b));
            let name = format!("{} (cópia)", t.name);
            let nid = self.session.fresh();
            self.session.tracks.push(Track::new(nid, name, merge(segs)));
            new.push(nid);
        }
        Ok(new)
    }

    /// Separar em faixa nova: tira a seleção (deixa silêncio) e põe numa faixa nova.
    pub fn split_new(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<Vec<u64>, EditError> {
        self.check(tracks, a, b)?;
        self.checkpoint("Separar em faixa nova");
        let mut new = Vec::new();
        for id in tracks {
            let Some(t) = self.session.track(*id) else {
                continue;
            };
            let mut segs = vec![Segment::silence(a)];
            segs.extend(slice(&t.segments, a, b));
            let name = t.name.clone();
            let nid = self.session.fresh();
            self.session.tracks.push(Track::new(nid, name, merge(segs)));
            let t = self.track_mut(*id)?;
            t.segments = overwrite(&t.segments, a, &[Segment::silence(b - a)]);
            new.push(nid);
        }
        Ok(new)
    }

    /// Arrasta o clipe `[a, b)` da faixa `from` para `at` na faixa `to` (sobrescrevendo lá).
    pub fn move_clip(
        &mut self,
        from: u64,
        a: usize,
        b: usize,
        to: u64,
        at: usize,
    ) -> Result<(), EditError> {
        self.check(&[from, to], a, b)?;
        let piece = slice(
            &self.session.track(from).ok_or(EditError::NoTrack)?.segments,
            a,
            b,
        );
        self.checkpoint("Mover clipe");
        let src = self.track_mut(from)?;
        src.segments = merge(overwrite(&src.segments, a, &[Segment::silence(b - a)]));
        let dst = self.track_mut(to)?;
        dst.segments = overwrite(&dst.segments, at, &piece);
        Ok(())
    }

    /// Efeito em `[a, b)` de cada faixa: o trecho é renderizado, processado e vira uma fonte
    /// nova (do mesmo tamanho).
    pub fn process(
        &mut self,
        tracks: &[u64],
        a: usize,
        b: usize,
        label: &'static str,
        mut f: impl FnMut(&mut Vec<f32>),
    ) -> Result<(), EditError> {
        self.check(tracks, a, b)?;
        let mut pieces = Vec::new();
        for id in tracks {
            let t = self.session.track(*id).ok_or(EditError::NoTrack)?;
            let end = b.min(t.len());
            if a >= end {
                continue;
            }
            let mut samples = self.render_segments(&slice(&t.segments, a, end));
            f(&mut samples);
            samples.resize(end - a, 0.0);
            pieces.push((*id, end, samples));
        }
        if pieces.is_empty() {
            return Err(EditError::EmptyRange);
        }
        self.checkpoint(label);
        for (id, _end, samples) in pieces {
            let seg = self.add_source(samples);
            let t = self.track_mut(id)?;
            t.segments = overwrite(&t.segments, a, &[seg]);
        }
        self.prune();
        Ok(())
    }

    fn render_segments(&self, segs: &[Segment]) -> Vec<f32> {
        let mut out = Vec::with_capacity(total(segs));
        for s in segs {
            match self.sources.get(&s.source) {
                Some(src) if s.source != SILENCE => {
                    let (lo, hi) = (s.start.min(src.len()), s.end.min(src.len()));
                    out.extend_from_slice(&src[lo..hi]);
                    out.resize(out.len() + (s.len() - (hi - lo)), 0.0);
                }
                _ => out.resize(out.len() + s.len(), 0.0),
            }
        }
        out
    }

    /// Áudio de uma faixa (sem ganho/pan).
    pub fn render(&self, id: u64) -> Vec<f32> {
        self.session
            .track(id)
            .map(|t| self.render_segments(&t.segments))
            .unwrap_or_default()
    }

    pub fn render_range(&self, id: u64, a: usize, b: usize) -> Vec<f32> {
        match self.session.track(id) {
            Some(t) if a < b => self.render_segments(&slice(&t.segments, a, b)),
            _ => Vec::new(),
        }
    }

    /// Mixagem estéreo intercalada de `[a, b)`: respeita mudo, solo, ganho e pan
    /// (pan por balanço: o lado oposto abaixa, o centro fica igual).
    pub fn mix(&self, a: usize, b: usize) -> Vec<f32> {
        let n = b.saturating_sub(a);
        let mut out = vec![0.0f32; n * 2];
        let any_solo = self.session.tracks.iter().any(|t| t.solo);
        for t in &self.session.tracks {
            if t.mute || (any_solo && !t.solo) {
                continue;
            }
            let g = 10f32.powf(t.gain_db / 20.0);
            let (gl, gr) = (g * (1.0 - t.pan).min(1.0), g * (1.0 + t.pan).min(1.0));
            for (i, v) in self.render_range(t.id, a, b).iter().enumerate() {
                out[2 * i] += v * gl;
                out[2 * i + 1] += v * gr;
            }
        }
        for v in &mut out {
            *v = v.clamp(-1.0, 1.0);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Vec<f32> {
        (1..=n).map(|i| i as f32).collect()
    }

    fn editor_with(n: usize) -> (Editor, u64) {
        let mut e = Editor::new(Session::new(48_000), BTreeMap::new());
        let id = e.add_track(ramp(n), 0, None);
        (e, id)
    }

    #[test]
    fn delete_silence_trim_and_undo() {
        let (mut e, t) = editor_with(10);
        e.delete(&[t], 2, 5).unwrap();
        assert_eq!(e.render(t), vec![1., 2., 6., 7., 8., 9., 10.]);
        e.silence(&[t], 0, 2).unwrap();
        assert_eq!(e.render(t), vec![0., 0., 6., 7., 8., 9., 10.]);
        e.trim(&[t], 3, 5).unwrap();
        assert_eq!(e.render(t), vec![0., 0., 0., 7., 8.]);
        assert_eq!(e.undo_label(), Some("Aparar"));
        e.undo();
        e.undo();
        e.undo();
        assert_eq!(e.render(t), ramp(10));
        e.redo();
        assert_eq!(e.render(t).len(), 7);
        assert!(e.delete(&[t], 5, 5).is_err());
    }

    #[test]
    fn recording_into_a_new_track_or_over_an_old_one() {
        let (mut e, t) = editor_with(4);
        let late = e.add_track(vec![9.0, 9.0], 3, None);
        assert_eq!(e.render(late), vec![0., 0., 0., 9., 9.]);
        assert_eq!(e.session().track(late).unwrap().clips(), vec![(3, 5)]);
        e.write_at(t, 2, vec![7.0, 7.0, 7.0]).unwrap();
        assert_eq!(e.render(t), vec![1., 2., 7., 7., 7.]);
        assert_eq!(e.session().len(), 5);
    }

    #[test]
    fn cut_copy_paste_duplicate_and_split() {
        let (mut e, t) = editor_with(6);
        e.copy(&[t], 1, 3).unwrap();
        e.paste(Some(t), 0).unwrap();
        assert_eq!(e.render(t), vec![2., 3., 1., 2., 3., 4., 5., 6.]);
        e.cut(&[t], 0, 2).unwrap();
        assert_eq!(e.render(t), ramp(6));
        // Colar com mais faixas copiadas do que existem cria faixas.
        let (targets, len) = e.paste(None, 2).unwrap();
        assert_eq!((targets.len(), len), (1, 2));
        assert_eq!(e.render(targets[0]), vec![0., 0., 2., 3.]);
        let dup = e.duplicate(&[t], 4, 6).unwrap();
        assert_eq!(e.render(dup[0]), vec![0., 0., 0., 0., 5., 6.]);
        let moved = e.split_new(&[t], 0, 2).unwrap();
        assert_eq!(e.render(t), vec![0., 0., 3., 4., 5., 6.]);
        assert_eq!(e.render(moved[0]), vec![1., 2.]);
    }

    #[test]
    fn moving_a_clip_in_time_and_between_tracks() {
        let (mut e, t) = editor_with(4);
        e.move_clip(t, 0, 4, t, 3).unwrap();
        assert_eq!(e.render(t), vec![0., 0., 0., 1., 2., 3., 4.]);
        let other = e.add_track(vec![5.0], 0, None);
        e.move_clip(t, 3, 7, other, 2).unwrap();
        assert!(e.render(t).is_empty());
        assert_eq!(e.render(other), vec![5., 0., 1., 2., 3., 4.]);
    }

    #[test]
    fn process_mix_and_prune() {
        let (mut e, t) = editor_with(6);
        e.process(&[t], 2, 4, "Ganho", |s| {
            s.iter_mut().for_each(|v| *v *= 10.0)
        })
        .unwrap();
        assert_eq!(e.render(t), vec![1., 2., 30., 40., 5., 6.]);
        let u = e.add_track(vec![0.25; 3], 0, None);
        e.set_mix(u, 0.0, 1.0, false, false);
        let m = e.mix(0, 2);
        // faixa 1 (1,2) no centro + faixa 2 (0.25) só na direita; mixagem limitada a ±1.
        assert_eq!(m, vec![1.0, 1.0, 1.0, 1.0]);
        e.set_mix(t, 0.0, 0.0, true, false);
        assert_eq!(e.mix(0, 1), vec![0.0, 0.25]);
        e.set_mix(u, 0.0, 1.0, false, true);
        e.set_mix(t, 0.0, 0.0, false, false);
        assert_eq!(
            e.mix(0, 1),
            vec![0.0, 0.25],
            "solo só toca a faixa com solo"
        );
        assert!(e.delete_many(t, &[(0, 1), (4, 5)]).unwrap() == 2);
        assert_eq!(e.render(t), vec![2., 30., 40., 6.]);
    }
}
