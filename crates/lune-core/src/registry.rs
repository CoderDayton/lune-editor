//! Buffer registry — manages all open buffers.
//!
//! The registry ensures that each file path maps to at most one buffer,
//! preventing duplicate opens.

use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::buffer::{BufferId, TextBuffer};

/// Manages all open text buffers.
#[derive(Debug, Default)]
pub struct BufferRegistry {
    /// Buffers indexed by their unique ID.
    buffers: FxHashMap<BufferId, TextBuffer>,
    /// Reverse lookup: file path → buffer ID.
    path_index: FxHashMap<PathBuf, BufferId>,
}

impl BufferRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a file. If a buffer for this path already exists, return its ID.
    /// Otherwise, read the file and create a new buffer.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read.
    pub fn open_file(&mut self, path: &Path) -> Result<BufferId> {
        let canonical = std::fs::canonicalize(path)?;

        if let Some(&id) = self.path_index.get(&canonical) {
            return Ok(id);
        }

        let buf = TextBuffer::from_file(&canonical)?;
        let id = buf.id;
        self.path_index.insert(canonical, id);
        self.buffers.insert(id, buf);
        Ok(id)
    }

    /// Create a new scratch (untitled) buffer.
    pub fn new_scratch(&mut self) -> BufferId {
        let buf = TextBuffer::new();
        let id = buf.id;
        self.buffers.insert(id, buf);
        id
    }

    /// Close a buffer, removing it from the registry.
    ///
    /// Returns `true` if the buffer existed and was removed.
    pub fn close(&mut self, id: BufferId) -> bool {
        if let Some(buf) = self.buffers.remove(&id) {
            if let Some(ref path) = buf.file_path {
                self.path_index.remove(path);
            }
            true
        } else {
            false
        }
    }

    /// Get an immutable reference to a buffer by ID.
    #[must_use]
    pub fn get(&self, id: BufferId) -> Option<&TextBuffer> {
        self.buffers.get(&id)
    }

    /// Get a mutable reference to a buffer by ID.
    pub fn get_mut(&mut self, id: BufferId) -> Option<&mut TextBuffer> {
        self.buffers.get_mut(&id)
    }

    /// Look up a buffer by its file path.
    #[must_use]
    pub fn by_path(&self, path: &Path) -> Option<BufferId> {
        self.path_index.get(path).copied()
    }

    /// Number of open buffers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffers.len()
    }

    /// Whether there are any open buffers.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }

    /// Iterate over all buffer IDs.
    pub fn ids(&self) -> impl Iterator<Item = BufferId> + '_ {
        self.buffers.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_scratch() {
        let mut reg = BufferRegistry::new();
        let id = reg.new_scratch();
        assert!(reg.get(id).is_some());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn close_removes_buffer() {
        let mut reg = BufferRegistry::new();
        let id = reg.new_scratch();
        assert!(reg.close(id));
        assert!(reg.get(id).is_none());
        assert!(reg.is_empty());
    }

    #[test]
    fn close_nonexistent_returns_false() {
        let mut reg = BufferRegistry::new();
        assert!(!reg.close(BufferId::new()));
    }

    #[test]
    fn open_same_file_returns_same_id() {
        let dir = std::env::temp_dir().join("lune_test_registry");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.txt");
        std::fs::write(&path, "hello").unwrap();

        let mut reg = BufferRegistry::new();
        let id1 = reg.open_file(&path).unwrap();
        let id2 = reg.open_file(&path).unwrap();
        assert_eq!(id1, id2);
        assert_eq!(reg.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_file_and_lookup_by_path() {
        let dir = std::env::temp_dir().join("lune_test_registry_lookup");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("lookup.txt");
        std::fs::write(&path, "content").unwrap();

        let mut reg = BufferRegistry::new();
        let id = reg.open_file(&path).unwrap();

        let canonical = std::fs::canonicalize(&path).unwrap();
        assert_eq!(reg.by_path(&canonical), Some(id));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scratch_has_no_path() {
        let mut reg = BufferRegistry::new();
        let id = reg.new_scratch();
        let buf = reg.get(id).unwrap();
        assert!(buf.file_path.is_none());
    }

    #[test]
    fn get_mut_allows_editing() {
        let mut reg = BufferRegistry::new();
        let id = reg.new_scratch();
        let buf = reg.get_mut(id).unwrap();
        buf.insert(crate::position::Position::new(0, 0), "hello");
        assert_eq!(reg.get(id).unwrap().text(), "hello");
    }
}
