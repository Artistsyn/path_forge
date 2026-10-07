//! Undo history for the studio: edits are grouped into steps that match what a person did. A
//! whole drag, or a number typed into a field, is one step, because edits are only committed once
//! the pointer is up and no text field has focus. Kept apart from the UI so it can be tested.

use serde_json::Value;

pub struct History {
    undo: Vec<Value>,
    redo: Vec<Value>,
    /// The document as of the last step.
    committed: Value,
    cap: usize,
}

impl History {
    pub fn new(doc: &Value, cap: usize) -> History { History { undo: Vec::new(), redo: Vec::new(), committed: doc.clone(), cap: cap.max(1) } }

    fn push(&mut self, v: Value) {
        self.undo.push(v);
        if self.undo.len() > self.cap { self.undo.remove(0); }
        self.redo.clear();
    }

    /// The document was replaced whole (a preset, a file, a remix); `undoable` makes that a step.
    pub fn replaced(&mut self, old: Value, doc: &Value, undoable: bool) {
        if undoable && old != *doc { self.push(old); }
        self.committed = doc.clone();
    }

    /// Make the edits since the last step into one step, unless the user is mid-gesture (`busy`:
    /// a pointer button down or a text field focused). Returns whether a step was made.
    pub fn commit(&mut self, doc: &Value, busy: bool) -> bool {
        if *doc == self.committed || busy { return false; }
        let prev = std::mem::replace(&mut self.committed, doc.clone());
        self.push(prev);
        true
    }

    /// Step back, first closing any edits in progress into a step of their own.
    pub fn undo(&mut self, doc: &mut Value) -> bool {
        self.commit(doc, false);
        let Some(v) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(doc, v));
        self.committed = doc.clone();
        true
    }

    pub fn redo(&mut self, doc: &mut Value) -> bool {
        let Some(v) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(doc, v));
        self.committed = doc.clone();
        true
    }

    pub fn can_undo(&self, doc: &Value) -> bool { !self.undo.is_empty() || *doc != self.committed }
    pub fn can_redo(&self) -> bool { !self.redo.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_drag_is_one_step_and_undo_redo_walk_the_steps() {
        let mut doc = json!({"horizon": 0.3});
        let mut h = History::new(&doc, 200);
        assert!(!h.can_undo(&doc));
        // A drag: forty intermediate values while the pointer is down, then let go.
        for i in 0..40 { doc["horizon"] = json!(0.3 + i as f64 * 0.005); assert!(!h.commit(&doc, true)); }
        assert!(h.commit(&doc, false));
        doc["horizon"] = json!(0.5);
        assert!(h.commit(&doc, false));
        assert!(h.undo(&mut doc));
        assert_eq!(doc["horizon"], json!(0.3 + 39.0 * 0.005), "back to the end of the drag");
        assert!(h.undo(&mut doc));
        assert_eq!(doc["horizon"], json!(0.3), "the whole drag undone in one step");
        assert!(!h.undo(&mut doc));
        assert!(h.redo(&mut doc) && h.redo(&mut doc));
        assert_eq!(doc["horizon"], json!(0.5));
        assert!(!h.can_redo());
    }

    #[test]
    fn a_new_edit_drops_the_redo_steps_and_undo_closes_edits_in_progress() {
        let mut doc = json!({"a": 1});
        let mut h = History::new(&doc, 200);
        doc["a"] = json!(2); h.commit(&doc, false);
        h.undo(&mut doc);
        assert!(h.can_redo());
        doc["a"] = json!(3); h.commit(&doc, false);
        assert!(!h.can_redo(), "redo is gone once you edit after undoing");
        // Typing into a field (focused, so not yet a step), then undo: the typing is undone.
        doc["a"] = json!(4);
        assert!(h.can_undo(&doc));
        h.undo(&mut doc);
        assert_eq!(doc["a"], json!(3));
    }

    #[test]
    fn history_is_bounded_and_whole_replacements_can_be_undone() {
        let mut doc = json!(0);
        let mut h = History::new(&doc, 5);
        for i in 1..=12 { doc = json!(i); h.commit(&doc, false); }
        let mut n = 0;
        while h.undo(&mut doc) { n += 1; }
        assert_eq!(n, 5);
        assert_eq!(doc, json!(7));
        let old = std::mem::replace(&mut doc, json!("preset"));
        h.replaced(old, &doc, true);
        assert!(h.undo(&mut doc));
        assert_eq!(doc, json!(7));
        let steps = h.undo.len();
        let old = std::mem::replace(&mut doc, json!("opened"));
        h.replaced(old, &doc, false);
        assert_eq!(h.undo.len(), steps, "a replacement that is not undoable adds no step");
        assert!(!h.commit(&doc, false), "and leaves nothing pending");
    }
}
