//! JSON-file [`PersistencePort`] adapter.
//!
//! Replaces the sled-backed `StateDb` for simple key/value workspace state
//! with a single JSON file. Values are stored as base64 strings (the port
//! payload is opaque `Vec<u8>`, so we can't just write raw bytes inside a
//! JSON object and round-trip cleanly).
//!
//! # Semantics
//!
//! - Reads: O(1) hash lookup against an in-memory mirror.
//! - Writes: update the mirror immediately, schedule a flush on the
//!   adapter's runtime task. Multiple Puts within `debounce_ms` are
//!   coalesced into one atomic file write.
//! - Atomic write: write to `<path>.tmp`, then rename over `<path>`.
//! - Single-process: the adapter does **not** take a file lock. If the
//!   user opens two instances on the same workspace, the last writer
//!   wins. This is the same behaviour sled offered once you stripped the
//!   lock contention — just without the "disabled workspace state"
//!   failure mode.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use rustc_hash::FxHashMap;
use tokio::sync::Notify;

use crate::ports::persistence::{
    JsonFilePortConfig, PersistenceCommand, PersistencePort, SharedPersistencePort, StoreSnapshot,
};
use crate::ports::runtime::RuntimeHandle;
use crate::ports::snapshot::{Snapshot, SnapshotCell};

type Map = FxHashMap<String, Vec<u8>>;

struct Inner {
    path: PathBuf,
    mirror: Mutex<Map>,
    cell: SnapshotCell<StoreSnapshot>,
    snap_reader: Snapshot<StoreSnapshot>,
    dirty: Mutex<bool>,
    notify: Notify,
    debounce: Duration,
}

pub struct JsonFilePersistencePort {
    inner: Arc<Inner>,
}

impl JsonFilePersistencePort {
    /// Spawn the adapter. Loads the existing file (if any) synchronously
    /// on the calling thread, then hands off background flush work to
    /// the runtime.
    pub fn spawn(rt: &RuntimeHandle, cfg: JsonFilePortConfig) -> Self {
        let map = load_from_disk(&cfg.path).unwrap_or_default();
        let (cell, snap_reader) = SnapshotCell::new(StoreSnapshot {
            revision: 1,
            last_error: None,
        });

        let inner = Arc::new(Inner {
            path: cfg.path,
            mirror: Mutex::new(map),
            cell,
            snap_reader,
            dirty: Mutex::new(false),
            notify: Notify::new(),
            debounce: Duration::from_millis(cfg.debounce_ms.max(1)),
        });

        let worker = inner.clone();
        rt.spawn(async move {
            flush_loop(worker).await;
        });

        Self { inner }
    }

    /// Build and register under an `Arc<dyn PersistencePort>` for storage
    /// on `AppState`.
    pub fn shared(rt: &RuntimeHandle, cfg: JsonFilePortConfig) -> SharedPersistencePort {
        Arc::new(Self::spawn(rt, cfg))
    }
}

impl PersistencePort for JsonFilePersistencePort {
    fn snapshot(&self) -> Snapshot<StoreSnapshot> {
        self.inner.snap_reader.clone()
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.inner.mirror.lock().ok()?.get(key).cloned()
    }

    fn dispatch(&self, cmd: PersistenceCommand) {
        let changed = {
            let Ok(mut map) = self.inner.mirror.lock() else {
                return;
            };
            match cmd {
                PersistenceCommand::Put { key, value } => {
                    let prev = map.insert(key, value);
                    // Only treat as dirty if the value actually changed.
                    // (Not enforced here — caller might re-Put the same
                    // bytes — but the no-op flush is cheap.)
                    let _ = prev;
                    true
                }
                PersistenceCommand::Delete { key } => map.remove(&key).is_some(),
                PersistenceCommand::Flush => true,
            }
        };

        if changed {
            if let Ok(mut dirty) = self.inner.dirty.lock() {
                *dirty = true;
            }
            self.inner.notify.notify_one();
        }
    }
}

// ── background flush loop ──────────────────────────────────────────

async fn flush_loop(inner: Arc<Inner>) {
    loop {
        inner.notify.notified().await;
        // Coalesce bursts: wait the debounce window before actually writing.
        tokio::time::sleep(inner.debounce).await;

        let snap = {
            let Ok(mut dirty) = inner.dirty.lock() else {
                break;
            };
            if !*dirty {
                continue;
            }
            *dirty = false;
            let Ok(map) = inner.mirror.lock() else {
                break;
            };
            map.clone()
        };

        let path = inner.path.clone();
        let result = tokio::task::spawn_blocking(move || write_atomic(&path, &snap)).await;
        match result {
            Ok(Ok(())) => {
                let prev = inner.snap_reader.load();
                inner.cell.publish(StoreSnapshot {
                    revision: prev.revision.wrapping_add(1),
                    last_error: None,
                });
            }
            Ok(Err(e)) => {
                log::warn!("json-persistence write failed: {e}");
                let prev = inner.snap_reader.load();
                inner.cell.publish(StoreSnapshot {
                    revision: prev.revision,
                    last_error: Some(e.to_string()),
                });
            }
            Err(join) => {
                log::warn!("json-persistence flush task panicked: {join}");
            }
        }
    }
}

// ── disk I/O (all on spawn_blocking) ───────────────────────────────

fn load_from_disk(path: &std::path::Path) -> Option<Map> {
    let raw = std::fs::read(path).ok()?;
    let parsed: FxHashMap<String, String> = serde_json::from_slice(&raw).ok()?;
    let engine = base64::engine::general_purpose::STANDARD;
    let mut out = FxHashMap::default();
    for (k, v) in parsed {
        let Ok(bytes) = engine.decode(v.as_bytes()) else {
            continue;
        };
        out.insert(k, bytes);
    }
    Some(out)
}

fn write_atomic(path: &std::path::Path, map: &Map) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let engine = base64::engine::general_purpose::STANDARD;
    let encoded: FxHashMap<&str, String> = map
        .iter()
        .map(|(k, v)| (k.as_str(), engine.encode(v)))
        .collect();
    let bytes = serde_json::to_vec(&encoded).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::runtime::PortRuntime;
    use std::time::Instant;

    fn wait_for_rev(reader: &Snapshot<StoreSnapshot>, min_rev: u64) {
        let start = Instant::now();
        while reader.load().revision < min_rev {
            assert!(
                start.elapsed() <= Duration::from_secs(3),
                "persistence revision never reached {min_rev}; last={}",
                reader.load().revision
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn round_trip_via_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        let rt = PortRuntime::new().unwrap();
        let port = JsonFilePersistencePort::spawn(
            &rt.handle(),
            JsonFilePortConfig {
                path: path.clone(),
                debounce_ms: 10,
            },
        );

        port.dispatch(PersistenceCommand::Put {
            key: "alpha".into(),
            value: b"one".to_vec(),
        });
        port.dispatch(PersistenceCommand::Put {
            key: "beta".into(),
            value: vec![0, 1, 2, 3],
        });
        port.dispatch(PersistenceCommand::Flush);
        wait_for_rev(&port.snapshot(), 2);

        // Reopen: a fresh adapter on the same path should see the values.
        let rt2 = PortRuntime::new().unwrap();
        let port2 = JsonFilePersistencePort::spawn(
            &rt2.handle(),
            JsonFilePortConfig {
                path,
                debounce_ms: 10,
            },
        );
        assert_eq!(port2.get("alpha").as_deref(), Some(&b"one"[..]));
        assert_eq!(port2.get("beta").as_deref(), Some(&[0u8, 1, 2, 3][..]));
    }

    #[test]
    fn delete_removes_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        let rt = PortRuntime::new().unwrap();
        let port = JsonFilePersistencePort::spawn(
            &rt.handle(),
            JsonFilePortConfig {
                path: path.clone(),
                debounce_ms: 10,
            },
        );

        port.dispatch(PersistenceCommand::Put {
            key: "x".into(),
            value: b"y".to_vec(),
        });
        port.dispatch(PersistenceCommand::Delete { key: "x".into() });
        port.dispatch(PersistenceCommand::Flush);
        wait_for_rev(&port.snapshot(), 2);

        let rt2 = PortRuntime::new().unwrap();
        let port2 = JsonFilePersistencePort::spawn(
            &rt2.handle(),
            JsonFilePortConfig {
                path,
                debounce_ms: 10,
            },
        );
        assert!(port2.get("x").is_none());
    }
}
