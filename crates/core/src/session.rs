use crate::Result;

/// Undo steps kept; older snapshots are dropped.
const MAX_HISTORY: usize = 100;
/// Byte budget for all undo + redo snapshots (512 MiB); oldest undo steps go first.
const MAX_HISTORY_BYTES: usize = 512 * 1024 * 1024;

/// One open document with byte-snapshot undo/redo.
#[derive(Debug, Clone)]
pub struct Session {
    /// Display name (file name).
    pub name: String,
    /// Where it was loaded from / last saved to (native only).
    pub path: Option<std::path::PathBuf>,
    bytes: Vec<u8>,
    undo: Vec<(String, Vec<u8>)>,
    redo: Vec<(String, Vec<u8>)>,
    dirty: bool,
}

impl Session {
    pub fn new(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            path: None,
            bytes,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
        }
    }

    /// Current document bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Run `edit` on the current bytes; on success push history under `label`, clear redo, mark dirty.
    /// On error nothing changes. History keeps the last 100 steps, within a 512 MiB budget
    /// (at least one undo step is always kept).
    pub fn apply(
        &mut self,
        label: &str,
        edit: impl FnOnce(&[u8]) -> Result<Vec<u8>>,
    ) -> Result<()> {
        let new_bytes = edit(&self.bytes)?;
        let old_bytes = std::mem::replace(&mut self.bytes, new_bytes);
        self.undo.push((label.to_owned(), old_bytes));
        self.redo.clear();
        self.trim_history(MAX_HISTORY_BYTES);
        self.dirty = true;
        Ok(())
    }

    /// Like [`Self::apply`] but folds the change into the latest undo step (no new step),
    /// so a multi-part edit undoes in one go. Behaves like `apply` when history is empty.
    pub fn amend(
        &mut self,
        label: &str,
        edit: impl FnOnce(&[u8]) -> Result<Vec<u8>>,
    ) -> Result<()> {
        if self.undo.is_empty() {
            return self.apply(label, edit);
        }
        self.bytes = edit(&self.bytes)?;
        self.redo.clear();
        self.dirty = true;
        Ok(())
    }

    /// Label of the step `undo` would revert.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|(label, _)| label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|(label, _)| label.as_str())
    }

    /// Returns false when nothing to undo.
    pub fn undo(&mut self) -> bool {
        Self::step(
            &mut self.bytes,
            &mut self.undo,
            &mut self.redo,
            &mut self.dirty,
        )
    }

    pub fn redo(&mut self) -> bool {
        Self::step(
            &mut self.bytes,
            &mut self.redo,
            &mut self.undo,
            &mut self.dirty,
        )
    }

    /// Call after a successful save. History is kept (undo still works after save).
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// Drop the oldest undo steps beyond [`MAX_HISTORY`] or while undo + redo snapshots
    /// exceed `max_bytes`. The newest undo step is always kept.
    fn trim_history(&mut self, max_bytes: usize) {
        let snapshot_bytes = |stack: &[(String, Vec<u8>)]| -> usize {
            stack.iter().map(|(_, bytes)| bytes.len()).sum()
        };
        let mut total = snapshot_bytes(&self.undo) + snapshot_bytes(&self.redo);
        let over_count = self.undo.len().saturating_sub(MAX_HISTORY);
        let mut evict = 0;
        while evict + 1 < self.undo.len() && (evict < over_count || total > max_bytes) {
            total -= self.undo[evict].1.len();
            evict += 1;
        }
        self.undo.drain(..evict);
    }

    /// Move one snapshot from `from` to the current bytes, pushing the current bytes onto `to`.
    fn step(
        bytes: &mut Vec<u8>,
        from: &mut Vec<(String, Vec<u8>)>,
        to: &mut Vec<(String, Vec<u8>)>,
        dirty: &mut bool,
    ) -> bool {
        let Some((label, snapshot)) = from.pop() else {
            return false;
        };
        let current = std::mem::replace(bytes, snapshot);
        to.push((label, current));
        *dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit_to(len: usize) -> impl FnOnce(&[u8]) -> Result<Vec<u8>> {
        move |_| Ok(vec![0; len])
    }

    #[test]
    fn byte_budget_evicts_oldest_but_keeps_one_step() {
        let mut session = Session::new("doc", vec![0; 10]);
        for (i, label) in ["a", "b", "c"].into_iter().enumerate() {
            session.apply(label, edit_to(20 + i)).unwrap();
        }
        // Snapshots: a=10, b=20, c=21 bytes; a 45-byte budget drops only "a".
        session.trim_history(45);
        assert_eq!(session.undo.len(), 2);
        assert_eq!(session.undo[0].0, "b");
        // A tiny budget still keeps the newest step.
        session.trim_history(1);
        assert_eq!(session.undo_label(), Some("c"));
        assert!(session.undo());
        assert!(!session.undo());
    }
}
