//! Lock-free snapshot cell: async producer, sync consumer.
//!
//! The UI thread never awaits, never blocks. A background tokio task owns
//! the adapter, publishes immutable state via [`SnapshotCell::publish`], and
//! the UI reads the latest `Arc<T>` with [`Snapshot::load`] on every frame.
//!
//! This is the foundation of the port contract: reads are O(1) atomic loads,
//! writes are fire-and-forget, and there is no shared mutable state.

use std::sync::Arc;

use arc_swap::ArcSwap;

/// Producer side. Held by the adapter task; publishes new snapshots.
pub struct SnapshotCell<T> {
    inner: Arc<ArcSwap<T>>,
}

impl<T> SnapshotCell<T> {
    pub fn new(initial: T) -> (Self, Snapshot<T>) {
        let inner = Arc::new(ArcSwap::from_pointee(initial));
        (
            Self {
                inner: inner.clone(),
            },
            Snapshot { inner },
        )
    }

    /// Replace the current snapshot. O(1), non-blocking.
    pub fn publish(&self, value: T) {
        self.inner.store(Arc::new(value));
    }

    /// Read-modify-write helper. Clones the current value, applies `f`,
    /// publishes the result. Intended for adapter-internal use only.
    pub fn update<F>(&self, f: F)
    where
        T: Clone,
        F: FnOnce(&mut T),
    {
        let mut next = (**self.inner.load()).clone();
        f(&mut next);
        self.publish(next);
    }

    /// Hand out another reader. Cheap (Arc clone).
    pub fn reader(&self) -> Snapshot<T> {
        Snapshot {
            inner: self.inner.clone(),
        }
    }
}

/// Consumer side. Cloneable, `Send + Sync`. Safe to hand to the UI thread.
pub struct Snapshot<T> {
    inner: Arc<ArcSwap<T>>,
}

impl<T> Clone for Snapshot<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Snapshot<T> {
    /// Load the latest published value. Lock-free, never blocks.
    pub fn load(&self) -> Arc<T> {
        arc_swap::Guard::into_inner(self.inner.load())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_then_load_returns_latest() {
        let (cell, reader) = SnapshotCell::new(0u32);
        assert_eq!(*reader.load(), 0);
        cell.publish(42);
        assert_eq!(*reader.load(), 42);
    }

    #[test]
    fn multiple_readers_see_same_value() {
        let (cell, r1) = SnapshotCell::new("hello".to_string());
        let r2 = cell.reader();
        cell.publish("world".to_string());
        assert_eq!(*r1.load(), "world");
        assert_eq!(*r2.load(), "world");
    }

    #[test]
    fn update_applies_function() {
        let (cell, reader) = SnapshotCell::new(vec![1, 2]);
        cell.update(|v| v.push(3));
        assert_eq!(*reader.load(), vec![1, 2, 3]);
    }
}
