//! Edição de uma sessão com desfazer/refazer.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use thiserror::Error;

use crate::{SILENCE, Segment, Session, SourceId, Take};

#[derive(Debug, Clone, PartialEq, Error)]
pub enum EditError {
    #[error("a tomada não existe")]
    NoTake,
    #[error("trecho vazio")]
    EmptyRange,
}

/// Sessão + fontes de áudio + histórico.
pub struct Editor {
    session: Session,
    sources: BTreeMap<SourceId, Arc<Vec<f32>>>,
    undo: Vec<(Session, &'static str)>,
    redo: Vec<(Session, &'static str)>,
    revision: u64,
    saved_revision: u64,
}

const HISTORY: usize = 200;

impl Editor {
    pub fn new(session: Session, sources: BTreeMap<SourceId, Arc<Vec<f32>>>) -> Self {
        Self {
            session,
            sources,
            undo: Vec::new(),
            redo: Vec::new(),
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

    /// Guarda o estado atual no histórico antes de uma mudança.
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
        self.prune();
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

    /// Esquece fontes que nem a sessão nem o histórico usam mais.
    fn prune(&mut self) {
        let mut used: HashSet<SourceId> = HashSet::new();
        for s in std::iter::once(&self.session)
            .chain(self.undo.iter().map(|(s, _)| s))
            .chain(self.redo.iter().map(|(s, _)| s))
        {
            for t in &s.takes {
                used.extend(t.segments.iter().map(|g| g.source));
            }
        }
        self.sources.retain(|id, _| used.contains(id));
    }

    fn take_mut(&mut self, id: u64) -> Result<&mut Take, EditError> {
        self.session
            .takes
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or(EditError::NoTake)
    }

    pub fn set_script(&mut self, text: String) {
        // O roteiro não entra no desfazer (é texto livre, editado o tempo todo).
        if self.session.script != text {
            self.session.script = text;
            self.revision += 1;
        }
    }

    /// Cria uma tomada nova com `samples`. Devolve o id.
    pub fn add_take(&mut self, samples: Vec<f32>, name: Option<String>) -> u64 {
        self.checkpoint("Nova tomada");
        let source = self.session.fresh();
        let len = samples.len();
        self.sources.insert(source, Arc::new(samples));
        let id = self.session.fresh();
        let n = self.session.takes.len() + 1;
        self.session.takes.push(Take {
            id,
            name: name.unwrap_or_else(|| format!("Tomada {n}")),
            segments: vec![Segment {
                source,
                start: 0,
                end: len,
            }],
        });
        id
    }

    pub fn remove_take(&mut self, id: u64) -> Result<(), EditError> {
        self.session.take(id).ok_or(EditError::NoTake)?;
        self.checkpoint("Apagar tomada");
        self.session.takes.retain(|t| t.id != id);
        self.prune();
        Ok(())
    }

    pub fn rename_take(&mut self, id: u64, name: String) -> Result<(), EditError> {
        if self.session.take(id).ok_or(EditError::NoTake)?.name == name {
            return Ok(());
        }
        self.checkpoint("Renomear tomada");
        self.take_mut(id)?.name = name;
        Ok(())
    }

    pub fn duplicate_take(&mut self, id: u64) -> Result<u64, EditError> {
        let t = self.session.take(id).ok_or(EditError::NoTake)?.clone();
        self.checkpoint("Duplicar tomada");
        let new = self.session.fresh();
        let pos = self
            .session
            .takes
            .iter()
            .position(|x| x.id == id)
            .unwrap_or(0);
        self.session.takes.insert(
            pos + 1,
            Take {
                id: new,
                name: format!("{} (cópia)", t.name),
                segments: t.segments,
            },
        );
        Ok(new)
    }

    /// Divide a tomada em duas no ponto `at`.
    pub fn split_take(&mut self, id: u64, at: usize) -> Result<u64, EditError> {
        let t = self.session.take(id).ok_or(EditError::NoTake)?.clone();
        if at == 0 || at >= t.len() {
            return Err(EditError::EmptyRange);
        }
        self.checkpoint("Dividir tomada");
        let (left, right) = (slice(&t.segments, 0, at), slice(&t.segments, at, t.len()));
        let new = self.session.fresh();
        let pos = self
            .session
            .takes
            .iter()
            .position(|x| x.id == id)
            .unwrap_or(0);
        self.session.takes[pos].segments = left;
        self.session.takes.insert(
            pos + 1,
            Take {
                id: new,
                name: format!("{} (2)", t.name),
                segments: right,
            },
        );
        Ok(new)
    }

    fn range(
        &self,
        id: u64,
        a: usize,
        b: usize,
    ) -> Result<(usize, usize, Vec<Segment>), EditError> {
        let t = self.session.take(id).ok_or(EditError::NoTake)?;
        let (a, b) = (a.min(t.len()), b.min(t.len()));
        if a >= b {
            return Err(EditError::EmptyRange);
        }
        Ok((a, b, t.segments.clone()))
    }

    /// Apaga `[a, b)` e junta o resto.
    pub fn delete(&mut self, id: u64, a: usize, b: usize) -> Result<(), EditError> {
        let (a, b, segs) = self.range(id, a, b)?;
        self.checkpoint("Apagar trecho");
        let len: usize = segs.iter().map(Segment::len).sum();
        let mut out = slice(&segs, 0, a);
        out.extend(slice(&segs, b, len));
        self.take_mut(id)?.segments = merge(out);
        self.prune();
        Ok(())
    }

    /// Apaga vários trechos de uma vez (ex.: silêncios), num passo de desfazer.
    pub fn delete_many(&mut self, id: u64, ranges: &[(usize, usize)]) -> Result<usize, EditError> {
        let t = self.session.take(id).ok_or(EditError::NoTake)?;
        let len = t.len();
        let mut ranges: Vec<(usize, usize)> = ranges
            .iter()
            .map(|(a, b)| (*a.min(&len), *b.min(&len)))
            .filter(|(a, b)| a < b)
            .collect();
        if ranges.is_empty() {
            return Err(EditError::EmptyRange);
        }
        ranges.sort();
        let segs = t.segments.clone();
        self.checkpoint("Remover silêncios");
        let mut out = Vec::new();
        let mut cur = 0;
        let mut removed = 0;
        for (a, b) in ranges {
            let a = a.max(cur);
            if a < b {
                out.extend(slice(&segs, cur, a));
                removed += b - a;
                cur = b;
            }
        }
        out.extend(slice(&segs, cur, len));
        self.take_mut(id)?.segments = merge(out);
        self.prune();
        Ok(removed)
    }

    /// Troca `[a, b)` por silêncio (mesma duração).
    pub fn silence(&mut self, id: u64, a: usize, b: usize) -> Result<(), EditError> {
        let (a, b, segs) = self.range(id, a, b)?;
        self.checkpoint("Silenciar");
        let len: usize = segs.iter().map(Segment::len).sum();
        let mut out = slice(&segs, 0, a);
        out.push(Segment {
            source: SILENCE,
            start: 0,
            end: b - a,
        });
        out.extend(slice(&segs, b, len));
        self.take_mut(id)?.segments = merge(out);
        self.prune();
        Ok(())
    }

    /// Fica só com `[a, b)`.
    pub fn crop(&mut self, id: u64, a: usize, b: usize) -> Result<(), EditError> {
        let (a, b, segs) = self.range(id, a, b)?;
        self.checkpoint("Manter só a seleção");
        self.take_mut(id)?.segments = slice(&segs, a, b);
        self.prune();
        Ok(())
    }

    /// Aplica um efeito em `[a, b)`: o trecho é renderizado, processado e vira uma fonte nova.
    pub fn process(
        &mut self,
        id: u64,
        a: usize,
        b: usize,
        label: &'static str,
        f: impl FnOnce(&mut Vec<f32>),
    ) -> Result<(), EditError> {
        let (a, b, segs) = self.range(id, a, b)?;
        let mut samples = self.render_segments(&slice(&segs, a, b));
        f(&mut samples);
        self.checkpoint(label);
        let source = self.session.fresh();
        let n = samples.len();
        self.sources.insert(source, Arc::new(samples));
        let len: usize = segs.iter().map(Segment::len).sum();
        let mut out = slice(&segs, 0, a);
        out.push(Segment {
            source,
            start: 0,
            end: n,
        });
        out.extend(slice(&segs, b, len));
        self.take_mut(id)?.segments = merge(out);
        self.prune();
        Ok(())
    }

    fn render_segments(&self, segs: &[Segment]) -> Vec<f32> {
        let mut out = Vec::with_capacity(segs.iter().map(Segment::len).sum());
        for s in segs {
            match self.sources.get(&s.source) {
                Some(src) if s.source != SILENCE => {
                    out.extend_from_slice(&src[s.start.min(src.len())..s.end.min(src.len())]);
                    out.resize(
                        out.len() + s.end.saturating_sub(src.len().max(s.start)),
                        0.0,
                    );
                }
                _ => out.resize(out.len() + s.len(), 0.0),
            }
        }
        out
    }

    /// Áudio da tomada inteira.
    pub fn render(&self, id: u64) -> Vec<f32> {
        self.session
            .take(id)
            .map(|t| self.render_segments(&t.segments))
            .unwrap_or_default()
    }

    /// Áudio de `[a, b)` da tomada.
    pub fn render_range(&self, id: u64, a: usize, b: usize) -> Vec<f32> {
        match self.session.take(id) {
            Some(t) if a < b => self.render_segments(&slice(&t.segments, a, b.min(t.len()))),
            _ => Vec::new(),
        }
    }
}

/// Os trechos que cobrem `[a, b)` da sequência.
fn slice(segs: &[Segment], a: usize, b: usize) -> Vec<Segment> {
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
    out
}

/// Junta trechos vizinhos da mesma fonte que continuam um ao outro.
fn merge(segs: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::with_capacity(segs.len());
    for s in segs.into_iter().filter(|s| !s.is_empty()) {
        match out.last_mut() {
            Some(l) if l.source == s.source && l.end == s.start => l.end = s.end,
            Some(l) if l.source == SILENCE && s.source == SILENCE => l.end += s.len(),
            _ => out.push(s),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32).collect()
    }

    fn editor_with(n: usize) -> (Editor, u64) {
        let mut e = Editor::new(Session::new(48_000), BTreeMap::new());
        let id = e.add_take(ramp(n), None);
        (e, id)
    }

    #[test]
    fn delete_silence_crop_and_undo() {
        let (mut e, t) = editor_with(10);
        e.delete(t, 2, 5).unwrap();
        assert_eq!(e.render(t), vec![0., 1., 5., 6., 7., 8., 9.]);
        e.silence(t, 0, 2).unwrap();
        assert_eq!(e.render(t), vec![0., 0., 5., 6., 7., 8., 9.]);
        e.crop(t, 2, 4).unwrap();
        assert_eq!(e.render(t), vec![5., 6.]);
        assert_eq!(e.undo_label(), Some("Manter só a seleção"));
        e.undo();
        e.undo();
        e.undo();
        assert_eq!(e.render(t), ramp(10));
        e.redo();
        assert_eq!(e.render(t).len(), 7);
        assert!(e.delete(t, 5, 5).is_err());
    }

    #[test]
    fn process_replaces_only_the_range_and_prunes_sources() {
        let (mut e, t) = editor_with(6);
        e.process(t, 2, 4, "Ganho", |s| s.iter_mut().for_each(|v| *v *= 10.0))
            .unwrap();
        assert_eq!(e.render(t), vec![0., 1., 20., 30., 4., 5.]);
        assert_eq!(e.sources().len(), 2);
        e.undo();
        e.remove_take(t).ok();
        // A tomada apagada continua no histórico: as fontes ficam até não haver como voltar.
        assert!(!e.sources().is_empty());
    }

    #[test]
    fn delete_many_split_and_duplicate() {
        let (mut e, t) = editor_with(10);
        assert_eq!(e.delete_many(t, &[(1, 3), (6, 8), (2, 4)]).unwrap(), 5);
        assert_eq!(e.render(t), vec![0., 4., 5., 8., 9.]);
        let second = e.split_take(t, 2).unwrap();
        assert_eq!(e.render(t), vec![0., 4.]);
        assert_eq!(e.render(second), vec![5., 8., 9.]);
        let copy = e.duplicate_take(second).unwrap();
        assert_eq!(e.render(copy), e.render(second));
        assert_eq!(e.session().takes.len(), 3);
        assert_eq!(e.render_range(t, 1, 2), vec![4.]);
    }
}
