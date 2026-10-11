use crate::edit::{self, Edit, EditError, Outcome};
use crate::model::Project;

const HISTORY_LIMIT: usize = 200;

/// Dono do projeto aberto. Toda alteração passa por `apply`, que guarda um retrato para desfazer.
#[derive(Debug, Clone)]
pub struct Editor {
    project: Project,
    undo: Vec<Project>,
    redo: Vec<Project>,
    /// Chave do grupo de `apply_coalescing` em andamento.
    coalesce: Option<String>,
    revision: u64,
}

impl Editor {
    pub fn new(project: Project) -> Self {
        Self {
            project,
            undo: Vec::new(),
            redo: Vec::new(),
            coalesce: None,
            revision: 0,
        }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Aplica a edição numa cópia. Em caso de erro ou edição vazia, o projeto e o histórico ficam intactos.
    pub fn apply(&mut self, edit: Edit) -> Result<Outcome, EditError> {
        let mut next = self.project.clone();
        let outcome = edit::apply(&mut next, edit)?;
        edit::normalize_links(&mut next);
        if next == self.project {
            return Err(EditError::NoChange);
        }
        self.undo.push(std::mem::replace(&mut self.project, next));
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.coalesce = None;
        self.revision += 1;
        Ok(outcome)
    }

    /// Como `apply`, mas edições seguidas com a mesma `key` viram um passo só de desfazer
    /// (ex.: arrastar um valor no Inspector). Qualquer outra edição encerra o grupo.
    pub fn apply_coalescing(&mut self, edit: Edit, key: &str) -> Result<Outcome, EditError> {
        if self.coalesce.as_deref() != Some(key) || self.undo.is_empty() {
            let outcome = self.apply(edit)?;
            self.coalesce = Some(key.to_string());
            return Ok(outcome);
        }
        let mut next = self.project.clone();
        let outcome = edit::apply(&mut next, edit)?;
        edit::normalize_links(&mut next);
        if next == self.project {
            return Err(EditError::NoChange);
        }
        self.project = next;
        self.redo.clear();
        self.revision += 1;
        Ok(outcome)
    }

    /// Muda a cada alteração (inclusive desfazer/refazer): serve para saber se há algo não salvo.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(&mut self.project, prev));
                self.coalesce = None;
                self.revision += 1;
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(&mut self.project, next));
                self.coalesce = None;
                self.revision += 1;
                true
            }
            None => false,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
