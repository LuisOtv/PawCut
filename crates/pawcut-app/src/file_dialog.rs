//! Diálogos de arquivo fora da thread da interface: o `rfd` bloqueia enquanto a janela de
//! escolha está aberta, e na thread da interface isso faz o sistema achar que o app travou.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use pawcut_core::MediaId;

use crate::workspace::Pending;

/// Para que serve o diálogo aberto (o que fazer com a resposta).
#[derive(Debug, Clone)]
pub enum Purpose {
    Import,
    Open,
    /// "Salvar como"; depois de salvar, continua com `then` (ex.: abrir outro projeto).
    Save {
        then: Option<Pending>,
    },
    Relink(MediaId),
    ExportTo,
    /// Pasta onde procurar as mídias offline pelo nome.
    RelinkFolder,
    ImportSubtitles,
    ExportSubtitles,
}

pub struct OpenDialog {
    pub purpose: Purpose,
    rx: Receiver<Vec<PathBuf>>,
}

impl OpenDialog {
    /// Abre o diálogo numa thread; `run` devolve os arquivos escolhidos (vazio = cancelado).
    pub fn spawn(
        purpose: Purpose,
        run: impl FnOnce() -> Vec<PathBuf> + Send + 'static,
        repaint: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("pawcut-file-dialog".into())
            .spawn(move || {
                let _ = tx.send(run());
                repaint();
            })
            .expect("não foi possível abrir o diálogo de arquivo");
        Self { purpose, rx }
    }

    /// Resposta, se o usuário já escolheu (ou cancelou).
    pub fn poll(&self) -> Option<Vec<PathBuf>> {
        self.rx.try_recv().ok()
    }
}
