//! Shared tokio runtime handle for all port adapters.
//!
//! One multi-threaded runtime is spawned at startup. Adapters `spawn` their
//! worker tasks on it; the UI thread keeps its own sync event loop and never
//! touches the runtime except to submit commands.

use std::time::Duration;

use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinHandle;

/// Owns the tokio runtime.
///
/// Dropping a bare `Runtime` waits indefinitely for any `spawn_blocking`
/// task that is parked inside a blocking call (e.g. `mpsc::blocking_recv`).
/// Adapters commonly do exactly that. To keep tests and editor shutdown
/// snappy, we wrap the runtime so `Drop` invokes a bounded shutdown.
pub struct PortRuntime {
    rt: Option<Runtime>,
}

/// Upper bound on how long `Drop` waits for blocking workers before
/// forcibly terminating them. 250 ms is generous for git2/file-io
/// adapters and still feels instant to the user.
const SHUTDOWN_BUDGET: Duration = Duration::from_millis(250);

impl Drop for PortRuntime {
    fn drop(&mut self) {
        if let Some(rt) = self.rt.take() {
            rt.shutdown_timeout(SHUTDOWN_BUDGET);
        }
    }
}

impl PortRuntime {
    /// Build a multi-threaded runtime sized for I/O-bound adapter work.
    ///
    /// Four worker threads is plenty: git walks, file watching, AI streams,
    /// and persistence writes are all mostly blocking on syscalls or the
    /// network. Raise if profiling shows saturation.
    pub fn new() -> std::io::Result<Self> {
        let rt = Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("lune-port")
            .enable_io()
            .enable_time()
            .build()?;
        Ok(Self { rt: Some(rt) })
    }

    pub fn handle(&self) -> RuntimeHandle {
        RuntimeHandle {
            handle: self.rt.as_ref().expect("runtime live").handle().clone(),
        }
    }
}

/// Cheap clone. Given to each adapter constructor.
#[derive(Clone)]
pub struct RuntimeHandle {
    handle: tokio::runtime::Handle,
}

impl RuntimeHandle {
    pub fn spawn<F>(&self, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.handle.spawn(fut)
    }

    pub fn spawn_blocking<F, R>(&self, f: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        self.handle.spawn_blocking(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_spawns_task() {
        let rt = PortRuntime::new().unwrap();
        let handle = rt.handle();
        let (tx, rx) = std::sync::mpsc::channel();
        handle.spawn(async move {
            tx.send(7).unwrap();
        });
        assert_eq!(rx.recv().unwrap(), 7);
    }
}
