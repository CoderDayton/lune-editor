//! File system watcher — monitors a directory for changes.
//!
//! Uses the [`notify`] crate to watch a workspace directory for
//! file changes, with debouncing to coalesce rapid events
//! (e.g., during `git checkout` or AI bulk edits).
//!
//! Events are converted to [`WatchEvent`] and sent through a
//! [`crossbeam::channel`] for consumption by the event loop.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam::channel::{self, Receiver, Sender};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// Event emitted by the file watcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    /// A file or directory was created.
    Created(PathBuf),
    /// A file was modified.
    Modified(PathBuf),
    /// A file or directory was deleted.
    Deleted(PathBuf),
    /// A file or directory was renamed.
    Renamed {
        /// Old path.
        from: PathBuf,
        /// New path.
        to: PathBuf,
    },
}

/// Paths to ignore (always filtered out).
const IGNORED_DIRS: &[&str] = &[".git", "target", "node_modules", ".venv", "__pycache__"];

/// Watches a directory tree for file system changes.
pub struct FileWatcher {
    /// The underlying notify watcher. Dropping this stops watching.
    _watcher: RecommendedWatcher,
    /// Receive end of the event channel.
    rx: Receiver<WatchEvent>,
}

impl FileWatcher {
    /// Start watching a directory for changes.
    ///
    /// Events are debounced by `debounce` duration to coalesce rapid changes.
    /// The default recommended debounce is 200ms.
    ///
    /// # Errors
    /// Returns an error if the watcher cannot be created or the path
    /// cannot be watched.
    pub fn new(root: &Path, debounce: Duration) -> Result<Self> {
        let (tx, rx) = channel::unbounded();
        let pending = Arc::new(Mutex::new(PendingEvents::new()));
        let pending_clone = Arc::clone(&pending);
        let root_owned = root.to_path_buf();

        // Spawn a debounce flush thread.
        let debounce_dur = debounce;
        std::thread::Builder::new()
            .name("lune-watcher-debounce".into())
            .spawn(move || debounce_loop(&pending_clone, &tx, debounce_dur))
            .context("failed to spawn watcher debounce thread")?;

        let pending_for_handler = Arc::clone(&pending);

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                for path in &event.paths {
                    if should_ignore(path, &root_owned) {
                        return;
                    }
                }
                if let Ok(mut lock) = pending_for_handler.lock() {
                    lock.add_event(&event);
                }
            }
        })
        .context("failed to create file watcher")?;

        watcher
            .watch(root, RecursiveMode::Recursive)
            .with_context(|| format!("failed to watch directory: {}", root.display()))?;

        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }

    /// Try to receive pending events without blocking.
    ///
    /// Returns all currently available events (may be empty).
    pub fn try_recv_all(&self) -> Vec<WatchEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Receive the next event, blocking until one is available.
    ///
    /// # Errors
    /// Returns an error if the channel is disconnected.
    pub fn recv(&self) -> Result<WatchEvent> {
        self.rx.recv().context("watcher channel disconnected")
    }

    /// Get a clone of the receiver for integration with select! or
    /// other multi-channel patterns.
    #[must_use]
    pub const fn receiver(&self) -> &Receiver<WatchEvent> {
        &self.rx
    }
}

/// Check whether a path should be ignored.
fn should_ignore(path: &Path, root: &Path) -> bool {
    // Check each component of the relative path.
    if let Ok(relative) = path.strip_prefix(root) {
        for component in relative.components() {
            let name = component.as_os_str().to_string_lossy();
            if IGNORED_DIRS.contains(&name.as_ref()) {
                return true;
            }
        }
    }
    false
}

/// Accumulated events waiting to be flushed.
struct PendingEvents {
    created: HashSet<PathBuf>,
    modified: HashSet<PathBuf>,
    deleted: HashSet<PathBuf>,
    renamed: Vec<(PathBuf, PathBuf)>,
    dirty: bool,
}

impl PendingEvents {
    fn new() -> Self {
        Self {
            created: HashSet::new(),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            renamed: Vec::new(),
            dirty: false,
        }
    }

    fn add_event(&mut self, event: &notify::Event) {
        use notify::EventKind;
        self.dirty = true;

        match event.kind {
            EventKind::Create(_) => {
                for path in &event.paths {
                    self.created.insert(path.clone());
                    // If it was previously marked as deleted, remove from deleted.
                    self.deleted.remove(path);
                }
            }
            EventKind::Modify(_) => {
                for path in &event.paths {
                    // Only add to modified if not already in created.
                    if !self.created.contains(path) {
                        self.modified.insert(path.clone());
                    }
                }
            }
            EventKind::Remove(_) => {
                for path in &event.paths {
                    self.deleted.insert(path.clone());
                    // If it was created in this batch, remove from created.
                    self.created.remove(path);
                    self.modified.remove(path);
                }
            }
            EventKind::Any | EventKind::Access(_) | EventKind::Other => {}
        }

        // Handle rename: notify sends it as a pair of paths in one event.
        if matches!(
            event.kind,
            notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
        ) && event.paths.len() == 2
        {
            let from = event.paths[0].clone();
            let to = event.paths[1].clone();
            self.modified.remove(&from);
            self.modified.remove(&to);
            self.created.remove(&to);
            self.deleted.remove(&from);
            self.renamed.push((from, to));
        }
    }

    fn flush(&mut self) -> Vec<WatchEvent> {
        if !self.dirty {
            return Vec::new();
        }

        let mut events = Vec::new();

        for path in self.created.drain() {
            events.push(WatchEvent::Created(path));
        }
        for path in self.modified.drain() {
            events.push(WatchEvent::Modified(path));
        }
        for path in self.deleted.drain() {
            events.push(WatchEvent::Deleted(path));
        }
        for (from, to) in self.renamed.drain(..) {
            events.push(WatchEvent::Renamed { from, to });
        }

        self.dirty = false;
        events
    }
}

/// Background loop that periodically flushes pending events.
fn debounce_loop(pending: &Arc<Mutex<PendingEvents>>, tx: &Sender<WatchEvent>, interval: Duration) {
    loop {
        std::thread::sleep(interval);
        let events = {
            let Ok(mut lock) = pending.lock() else {
                break;
            };
            lock.flush()
        };
        for event in events {
            if tx.send(event).is_err() {
                // Receiver dropped — shut down.
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use notify::event::EventAttributes;

    use super::*;

    #[test]
    fn should_ignore_git_dir() {
        let root = PathBuf::from("/workspace");
        assert!(should_ignore(
            Path::new("/workspace/.git/objects/abc"),
            &root
        ));
        assert!(should_ignore(
            Path::new("/workspace/target/debug/build"),
            &root
        ));
        assert!(should_ignore(
            Path::new("/workspace/node_modules/foo"),
            &root
        ));
        assert!(!should_ignore(Path::new("/workspace/src/main.rs"), &root));
    }

    #[test]
    fn pending_events_create_then_delete_cancels() {
        let mut pending = PendingEvents::new();

        let event_create = notify::Event {
            kind: notify::EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/workspace/test.txt")],
            attrs: EventAttributes::default(),
        };
        pending.add_event(&event_create);

        let event_delete = notify::Event {
            kind: notify::EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![PathBuf::from("/workspace/test.txt")],
            attrs: EventAttributes::default(),
        };
        pending.add_event(&event_delete);

        let events = pending.flush();
        // Created then deleted in same batch — only deleted remains.
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], WatchEvent::Deleted(p) if p == Path::new("/workspace/test.txt"))
        );
    }

    #[test]
    fn pending_events_modify_coalesced() {
        let mut pending = PendingEvents::new();

        for _ in 0..5 {
            let event = notify::Event {
                kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                    notify::event::DataChange::Content,
                )),
                paths: vec![PathBuf::from("/workspace/file.rs")],
                attrs: EventAttributes::default(),
            };
            pending.add_event(&event);
        }

        let events = pending.flush();
        // 5 modifications coalesced into 1.
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], WatchEvent::Modified(p) if p == Path::new("/workspace/file.rs"))
        );
    }

    #[test]
    fn pending_events_flush_clears() {
        let mut pending = PendingEvents::new();

        let event = notify::Event {
            kind: notify::EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/workspace/new.txt")],
            attrs: EventAttributes::default(),
        };
        pending.add_event(&event);

        let events = pending.flush();
        assert_eq!(events.len(), 1);

        // Second flush should be empty.
        let events2 = pending.flush();
        assert!(events2.is_empty());
    }

    #[test]
    fn watcher_create_and_receive() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let watcher = FileWatcher::new(tmp.path(), Duration::from_millis(50))
            .expect("failed to create watcher");

        // Create a file to trigger an event.
        std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();

        // Wait for debounce + some margin.
        std::thread::sleep(Duration::from_millis(200));

        let events = watcher.try_recv_all();
        // We should get at least one event (created or modified).
        assert!(!events.is_empty(), "expected at least one event, got none");
    }

    #[test]
    fn watcher_ignores_git_changes() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");

        // Create a .git directory.
        std::fs::create_dir_all(tmp.path().join(".git/objects")).unwrap();

        let watcher = FileWatcher::new(tmp.path(), Duration::from_millis(50))
            .expect("failed to create watcher");

        // Write inside .git — should be ignored.
        std::fs::write(tmp.path().join(".git/objects/abc"), "data").unwrap();

        // Write a normal file — should be reported.
        std::fs::write(tmp.path().join("normal.txt"), "visible").unwrap();

        std::thread::sleep(Duration::from_millis(200));

        let events = watcher.try_recv_all();
        // All events should be for normal.txt, none for .git/*.
        for event in &events {
            match event {
                WatchEvent::Created(p) | WatchEvent::Modified(p) => {
                    assert!(
                        !p.to_string_lossy().contains(".git"),
                        "should not get events for .git paths, got: {p:?}"
                    );
                }
                _ => {}
            }
        }
    }
}
