//! Títulos de aba que cabem: quando o docker fica estreito demais para o nome, ele vira
//! "Fer…" e depois só "…", em vez de aparecer uma barra de rolagem na faixa de abas.

use std::collections::HashMap;
use std::hash::Hash;

use egui_dock::DockState;

/// Largura média de um caractere do corpo do texto (12,5 pt) com folga.
const CHAR_W: f32 = 6.5;
/// Margens da aba (e o espaço do "×" quando fecha).
const CHROME: f32 = 18.0;
const CLOSE_W: f32 = 18.0;

pub struct TabFit<K> {
    /// Largura que cada docker teve no último frame (a do grupo de abas inteiro).
    width: HashMap<K, f32>,
    /// Abas do grupo de cada docker: quantas são e quantos caracteres os títulos somam.
    mates: HashMap<K, (usize, usize)>,
    /// As outras abas do mesmo grupo (só a aba visível desenha, mas todas dividem a largura).
    group: HashMap<K, Vec<K>>,
}

impl<K> Default for TabFit<K> {
    fn default() -> Self {
        Self {
            width: HashMap::new(),
            mates: HashMap::new(),
            group: HashMap::new(),
        }
    }
}

impl<K: Hash + Eq + Copy> TabFit<K> {
    /// Conta quantas abas dividem cada grupo; chame antes de desenhar o `DockArea`.
    pub fn observe(&mut self, dock: &DockState<K>, name: impl Fn(K) -> &'static str) {
        let mut per_leaf: HashMap<_, Vec<K>> = HashMap::new();
        for (path, tab) in dock.iter_all_tabs() {
            per_leaf
                .entry((path.surface, path.node))
                .or_default()
                .push(*tab);
        }
        self.mates.clear();
        self.group.clear();
        for tabs in per_leaf.values() {
            let chars: usize = tabs.iter().map(|t| name(*t).chars().count()).sum();
            for t in tabs {
                self.mates.insert(*t, (tabs.len(), chars));
                self.group.insert(*t, tabs.clone());
            }
        }
    }

    /// Guarda a largura do docker (chame no começo do `ui` da aba).
    pub fn record(&mut self, tab: K, width: f32) {
        self.width.insert(tab, width);
        if let Some(group) = self.group.get(&tab) {
            for t in group {
                self.width.insert(*t, width);
            }
        }
    }

    /// O título que cabe: inteiro, cortado com "…" ou só "…".
    pub fn title(&self, tab: K, full: &str, closeable: bool) -> String {
        let Some(&w) = self.width.get(&tab) else {
            return full.to_string();
        };
        let (n, total) = self
            .mates
            .get(&tab)
            .copied()
            .unwrap_or((1, full.chars().count()));
        let chrome = CHROME + if closeable { CLOSE_W } else { 0.0 };
        // O que sobra depois das margens é dividido na proporção do tamanho de cada título.
        let share = full.chars().count() as f32 / total.max(1) as f32;
        fit(full, (w - n as f32 * chrome).max(0.0) * share)
    }
}

/// Corta `text` para caber em `room` pixels.
pub fn fit(text: &str, room: f32) -> String {
    let chars: Vec<char> = text.chars().collect();
    let max = (room / CHAR_W).floor().max(0.0) as usize;
    if chars.len() <= max {
        text.to_string()
    } else if max >= 3 {
        let mut s: String = chars[..max - 1].iter().collect();
        s.push('…');
        s
    } else {
        "…".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_whole_then_truncates_then_ellipsis() {
        assert_eq!(fit("Ferramentas", 200.0), "Ferramentas");
        assert_eq!(fit("Ferramentas", 40.0), "Ferra…");
        assert_eq!(fit("Ferramentas", 10.0), "…");
        assert_eq!(fit("Ferramentas", 0.0), "…");
    }
}
