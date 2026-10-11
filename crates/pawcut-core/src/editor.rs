use crate::edit::{self, Edit, EditError, Outcome};
use crate::model::Project;

const HISTORY_LIMIT: usize = 200;

/// Dono do projeto aberto. Toda alteração passa por `apply`, que guarda um retrato para desfazer.
#[derive(Debug, Clone)]
pub struct Editor {
    project: Project,
    undo: Vec<Project>,
    redo: Vec<Project>,
}

impl Editor {
    pub fn new(project: Project) -> Self {
        Self {
            project,
            undo: Vec::new(),
            redo: Vec::new(),
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
        Ok(outcome)
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(&mut self.project, prev));
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(&mut self.project, next));
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
