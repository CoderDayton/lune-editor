//! File-backed key/value store used by [`crate::state_db::StateDb`].
//!
//! # Format
//!
//! Single JSON file per store:
//!
//! ```json
//! { "hex-encoded-key": "base64-bytes", ... }
//! ```
//!
//! Keys are arbitrary byte slices, so we hex-encode them for JSON
//! compatibility. Values are arbitrary byte slices (bincode blobs from
//! the caller), so we base64-encode them.
//!
//! # Semantics
//!
//! - Reads are O(1) hash lookups against an in-memory mirror.
//! - Writes update the mirror immediately and schedule an atomic flush.
//! - `flush` blocks until the file is on disk (tmp + rename).
//! - No file locks — multiple processes writing the same store
//!   last-writer-wins. See `state_db.rs` for the rationale.

use std::path::{Path, PathBuf};

use base64::Engine;
use rustc_hash::FxHashMap;

pub(crate) struct KvStore {
    path: PathBuf,
    map: FxHashMap<Vec<u8>, Vec<u8>>,
    dirty: bool,
}

impl KvStore {
    /// Open or create the store at `path`. Missing / malformed files
    /// yield an empty in-memory store; the next [`flush`](Self::flush)
    /// will write a valid file.
    pub fn open(path: PathBuf) -> Self {
        let map = load(&path).unwrap_or_default();
        Self {
            path,
            map,
            dirty: false,
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.map.get(key).map(Vec::as_slice)
    }

    /// In-memory write. No disk I/O — call [`flush`](Self::flush) to
    /// persist. `Drop` also flushes so callers who forget see a clean
    /// final write on graceful shutdown (data during a crash between
    /// `put` and `flush` is lost, same as sled's pre-`flush` behaviour).
    pub fn put(&mut self, key: &[u8], value: Vec<u8>) {
        if self.map.get(key).map(Vec::as_slice) == Some(value.as_slice()) {
            return; // No-op write; don't mark dirty.
        }
        self.map.insert(key.to_vec(), value);
        self.dirty = true;
    }

    /// In-memory delete. See [`put`](Self::put) for durability semantics.
    pub fn remove(&mut self, key: &[u8]) {
        if self.map.remove(key).is_some() {
            self.dirty = true;
        }
    }

    /// Atomically write all pending changes to disk. No-op if nothing
    /// has changed since the last flush.
    pub fn flush(&mut self) -> anyhow::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        write_atomic(&self.path, &self.map)?;
        self.dirty = false;
        Ok(())
    }

    /// Whether there are unflushed in-memory writes.
    #[cfg(test)]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }
}

/// On drop, flush any pending writes so callers who forget explicit
/// `flush()` still get their last changes on disk during graceful
/// shutdown. Errors are logged rather than propagated — a failing drop
/// cannot return.
impl Drop for KvStore {
    fn drop(&mut self) {
        if self.dirty {
            if let Err(e) = self.flush() {
                log::warn!(
                    "kv-store: final flush failed for {}: {e}",
                    self.path.display()
                );
            }
        }
    }
}

fn load(path: &Path) -> Option<FxHashMap<Vec<u8>, Vec<u8>>> {
    let raw = std::fs::read(path).ok()?;
    let parsed: FxHashMap<String, String> = serde_json::from_slice(&raw).ok()?;
    let engine = base64::engine::general_purpose::STANDARD;
    let mut out = FxHashMap::default();
    for (k_hex, v_b64) in parsed {
        let Ok(key) = decode_hex(&k_hex) else {
            continue;
        };
        let Ok(value) = engine.decode(v_b64.as_bytes()) else {
            continue;
        };
        out.insert(key, value);
    }
    Some(out)
}

fn write_atomic(path: &Path, map: &FxHashMap<Vec<u8>, Vec<u8>>) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let engine = base64::engine::general_purpose::STANDARD;
    let encoded: FxHashMap<String, String> = map
        .iter()
        .map(|(k, v)| (encode_hex(k), engine.encode(v)))
        .collect();
    let bytes = serde_json::to_vec(&encoded)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn decode_hex(s: &str) -> anyhow::Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        anyhow::bail!("odd-length hex");
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks_exact(2) {
        let hi = from_hex_digit(chunk[0])?;
        let lo = from_hex_digit(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn from_hex_digit(b: u8) -> anyhow::Result<u8> {
    Ok(match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => anyhow::bail!("invalid hex digit: {b:#x}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_put_get() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = KvStore::open(dir.path().join("kv.json"));
        s.put(b"foo", b"bar".to_vec());
        assert_eq!(s.get(b"foo"), Some(&b"bar"[..]));
    }

    #[test]
    fn reopen_sees_prior_writes_via_drop_flush() {
        // Relies on `Drop` flushing dirty state. No explicit `flush()`.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kv.json");
        {
            let mut s = KvStore::open(path.clone());
            s.put(b"a", vec![1, 2, 3]);
            s.put(b"b", vec![4, 5, 6]);
        }
        let s2 = KvStore::open(path);
        assert_eq!(s2.get(b"a"), Some(&[1u8, 2, 3][..]));
        assert_eq!(s2.get(b"b"), Some(&[4u8, 5, 6][..]));
    }

    #[test]
    fn remove_erases_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = KvStore::open(dir.path().join("kv.json"));
        s.put(b"x", b"y".to_vec());
        s.remove(b"x");
        assert_eq!(s.get(b"x"), None);
    }

    #[test]
    fn put_is_in_memory_until_flush() {
        // Prove that `put` alone does NOT touch disk — the file is only
        // written when `flush()` (or `Drop`) runs.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kv.json");
        let mut s = KvStore::open(path.clone());
        s.put(b"k", vec![42]);
        assert!(s.is_dirty());
        assert!(!path.exists(), "put should not have written yet");
        s.flush().unwrap();
        assert!(!s.is_dirty());
        assert!(path.exists());
    }

    #[test]
    fn identical_put_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = KvStore::open(dir.path().join("kv.json"));
        s.put(b"k", vec![1, 2, 3]);
        s.flush().unwrap();
        s.put(b"k", vec![1, 2, 3]); // same bytes
        assert!(!s.is_dirty(), "identical write should not mark dirty");
    }

    #[test]
    fn hex_round_trip_handles_arbitrary_bytes() {
        for bytes in [&b""[..], &[0, 1, 255, 127, 42][..], &b"workspace:state"[..]] {
            let hex = encode_hex(bytes);
            let decoded = decode_hex(&hex).unwrap();
            assert_eq!(decoded, bytes);
        }
    }
}
