//! Whole-document commands used at serialization boundaries.

use super::EditCommand;
use crate::doc::Document;
use crate::work_mode::WorkMode;

/// Replace the open document as one Setup-mode undo step.
pub struct ReplaceDocument {
    other: Document,
}

impl ReplaceDocument {
    pub fn new(other: Document) -> Self {
        Self { other }
    }
}

impl EditCommand for ReplaceDocument {
    fn apply(&mut self, doc: &mut Document) {
        std::mem::swap(doc, &mut self.other);
    }

    fn revert(&mut self, doc: &mut Document) {
        std::mem::swap(doc, &mut self.other);
    }

    fn label(&self) -> &str {
        "Import Project"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Edit;

    #[test]
    fn replacing_a_project_is_one_undo_step() {
        let mut imported = Document::new();
        imported.meta.name = "imported".into();

        let mut edit = Edit::default();
        edit.dispatch(Box::new(ReplaceDocument::new(imported)))
            .expect("setup mode");
        assert_eq!(edit.doc.meta.name, "imported");

        assert!(edit.undo());
        assert_eq!(edit.doc.meta.name, "untitled");
        assert!(!edit.undo(), "the whole import was one command");

        assert!(edit.redo());
        assert_eq!(edit.doc.meta.name, "imported");
    }
}
