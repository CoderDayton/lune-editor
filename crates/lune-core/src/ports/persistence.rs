//! Persistence port: replaces sled with a pluggable async-backed store.
//!
//! The in-tree default adapter (not in this file) will be a plain JSON
//! file with debounced writes on the runtime. Sled stays available via a
//! feature flag during migration.

use std::path::PathBuf;
use std::sync::Arc;

use crate::ports::snapshot::Snapshot;

/// Opaque payload — the adapter treats it as bytes. Callers choose the
/// serialization (bincode for recovery, json for workspace state, etc.).
#[derive(Clone, Debug, Default)]
pub struct StoreSnapshot {
    /// Revision bumps every time the store on disk changes.
    pub revision: u64,
    /// Last error, if any. Useful for surfacing "disk full" or similar
    /// without hard-erroring the editor.
    pub last_error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum PersistenceCommand {
    Put { key: String, value: Vec<u8> },
    Delete { key: String },
    Flush,
}

pub trait PersistencePort: Send + Sync + 'static {
    /// Disk-state snapshot (for status bar / error reporting).
    fn snapshot(&self) -> Snapshot<StoreSnapshot>;

    /// Synchronous read of the last value written for `key`. This is the
    /// one read path allowed to touch the underlying store directly
    /// because startup recovery needs it before the UI runs; everything
    /// after startup should prefer cached copies.
    fn get(&self, key: &str) -> Option<Vec<u8>>;

    fn dispatch(&self, cmd: PersistenceCommand);
}

/// In-memory adapter for tests and for the "no workspace open" case.
pub struct MemoryPersistencePort {
    snap: Snapshot<StoreSnapshot>,
    map: std::sync::Mutex<rustc_hash::FxHashMap<String, Vec<u8>>>,
}

impl MemoryPersistencePort {
    pub fn new() -> Self {
        let (_cell, reader) = crate::ports::snapshot::SnapshotCell::new(StoreSnapshot::default());
        Self {
            snap: reader,
            map: std::sync::Mutex::new(rustc_hash::FxHashMap::default()),
        }
    }
}

impl Default for MemoryPersistencePort {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistencePort for MemoryPersistencePort {
    fn snapshot(&self) -> Snapshot<StoreSnapshot> {
        self.snap.clone()
    }
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.map.lock().ok()?.get(key).cloned()
    }
    fn dispatch(&self, cmd: PersistenceCommand) {
        let Ok(mut guard) = self.map.lock() else {
            return;
        };
        match cmd {
            PersistenceCommand::Put { key, value } => {
                guard.insert(key, value);
            }
            PersistenceCommand::Delete { key } => {
                guard.remove(&key);
            }
            PersistenceCommand::Flush => {}
        }
    }
}

pub type SharedPersistencePort = Arc<dyn PersistencePort>;

/// Marker for the file-backed adapter added in the next slice.
pub struct JsonFilePortConfig {
    pub path: PathBuf,
    pub debounce_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_put_then_get() {
        let port = MemoryPersistencePort::new();
        port.dispatch(PersistenceCommand::Put {
            key: "foo".into(),
            value: b"bar".to_vec(),
        });
        assert_eq!(port.get("foo").as_deref(), Some(&b"bar"[..]));
    }

    #[test]
    fn memory_delete() {
        let port = MemoryPersistencePort::new();
        port.dispatch(PersistenceCommand::Put {
            key: "k".into(),
            value: vec![1],
        });
        port.dispatch(PersistenceCommand::Delete { key: "k".into() });
        assert!(port.get("k").is_none());
    }
}
