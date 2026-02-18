//! PTY handle for spawning and managing child processes.
//!
//! Wraps `portable-pty` to provide a clean API for:
//! - Spawning a command in a pseudo-terminal
//! - Reading stdout (via a reader handle)
//! - Writing to stdin
//! - Resizing the terminal
//! - Killing the process

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

/// Terminal dimensions (rows x cols).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSize {
    /// Number of rows (lines).
    pub rows: u16,
    /// Number of columns (characters per line).
    pub cols: u16,
}

impl TermSize {
    /// Create a new terminal size.
    #[must_use]
    pub const fn new(rows: u16, cols: u16) -> Self {
        Self { rows, cols }
    }

    /// Convert to a `portable_pty::PtySize`.
    #[must_use]
    pub const fn to_pty_size(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl Default for TermSize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

/// A handle to a PTY-backed child process.
///
/// The reader and writer are separated so the reader can be moved to a
/// background thread while the writer stays in the main thread for input
/// forwarding.
pub struct PtyHandle {
    /// The child process.
    child: Box<dyn Child + Send>,
    /// The master side of the PTY (for resize).
    master: Box<dyn MasterPty + Send>,
    /// Writer for sending bytes to the child's stdin.
    writer: Box<dyn Write + Send>,
    /// Current terminal size.
    size: TermSize,
}

impl PtyHandle {
    /// Spawn a new command in a PTY.
    ///
    /// Returns the `PtyHandle` and a boxed reader for the child's stdout.
    /// The reader should be moved to a background thread.
    ///
    /// # Arguments
    /// - `command`: The executable path or name.
    /// - `args`: Command-line arguments.
    /// - `cwd`: Optional working directory.
    /// - `env`: Additional environment variables.
    /// - `size`: Initial terminal dimensions.
    ///
    /// # Errors
    /// Returns an error if the PTY cannot be opened or the command fails to spawn.
    pub fn spawn(
        command: &str,
        args: &[&str],
        cwd: Option<&Path>,
        env: &HashMap<String, String>,
        size: TermSize,
    ) -> anyhow::Result<(Self, Box<dyn Read + Send>)> {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(size.to_pty_size())
            .map_err(|e| anyhow::anyhow!("Failed to open PTY: {e}"))?;

        let mut cmd = CommandBuilder::new(command);
        for arg in args {
            cmd.arg(arg);
        }
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| anyhow::anyhow!("Failed to spawn command '{command}': {e}"))?;

        // Drop slave — we only need the master side now.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| anyhow::anyhow!("Failed to clone PTY reader: {e}"))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| anyhow::anyhow!("Failed to take PTY writer: {e}"))?;

        let handle = Self {
            child,
            master: pair.master,
            writer,
            size,
        };

        Ok((handle, reader))
    }

    /// Write bytes to the child's stdin.
    ///
    /// # Errors
    /// Returns an error if the write fails.
    pub fn write_all(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.writer
            .write_all(data)
            .map_err(|e| anyhow::anyhow!("PTY write failed: {e}"))?;
        self.writer
            .flush()
            .map_err(|e| anyhow::anyhow!("PTY flush failed: {e}"))?;
        Ok(())
    }

    /// Resize the PTY terminal.
    ///
    /// # Errors
    /// Returns an error if the resize fails.
    pub fn resize(&mut self, size: TermSize) -> anyhow::Result<()> {
        self.master
            .resize(size.to_pty_size())
            .map_err(|e| anyhow::anyhow!("PTY resize failed: {e}"))?;
        self.size = size;
        Ok(())
    }

    /// Check if the child process is still alive.
    #[must_use]
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Wait for the child to exit and return its exit code.
    ///
    /// # Errors
    /// Returns an error if waiting fails.
    pub fn wait(&mut self) -> anyhow::Result<Option<u32>> {
        match self.child.try_wait() {
            Ok(Some(status)) => Ok(Some(status.exit_code())),
            Ok(None) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("Failed to wait on child: {e}")),
        }
    }

    /// Kill the child process.
    ///
    /// # Errors
    /// Returns an error if the kill signal cannot be sent.
    pub fn kill(&mut self) -> anyhow::Result<()> {
        self.child
            .kill()
            .map_err(|e| anyhow::anyhow!("Failed to kill child: {e}"))
    }

    /// Get the current terminal size.
    #[must_use]
    pub const fn size(&self) -> TermSize {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_size_default() {
        let size = TermSize::default();
        assert_eq!(size.rows, 24);
        assert_eq!(size.cols, 80);
    }

    #[test]
    fn term_size_to_pty_size() {
        let size = TermSize::new(30, 100);
        let pty = size.to_pty_size();
        assert_eq!(pty.rows, 30);
        assert_eq!(pty.cols, 100);
        assert_eq!(pty.pixel_width, 0);
        assert_eq!(pty.pixel_height, 0);
    }

    #[test]
    fn spawn_echo_and_read_output() {
        let (mut handle, mut reader) = PtyHandle::spawn(
            "/bin/sh",
            &["-c", "echo hello"],
            None,
            &HashMap::new(),
            TermSize::new(24, 80),
        )
        .expect("spawn failed");

        // Read output (with timeout via limited buffer).
        let mut output = vec![0u8; 4096];
        // Give the process a moment to produce output.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let n = reader.read(&mut output).unwrap_or(0);
        let text = String::from_utf8_lossy(&output[..n]);
        assert!(
            text.contains("hello"),
            "Expected 'hello' in output: {text:?}"
        );

        // Process should exit quickly.
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!handle.is_alive());
    }

    #[test]
    fn spawn_cat_write_and_read() {
        let (mut handle, mut reader) = PtyHandle::spawn(
            "/bin/cat",
            &[],
            None,
            &HashMap::new(),
            TermSize::new(24, 80),
        )
        .expect("spawn failed");

        assert!(handle.is_alive());

        // Write to stdin.
        handle.write_all(b"test input\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));

        let mut output = vec![0u8; 4096];
        let n = reader.read(&mut output).unwrap_or(0);
        let text = String::from_utf8_lossy(&output[..n]);
        assert!(
            text.contains("test input"),
            "Expected 'test input' in output: {text:?}"
        );

        handle.kill().unwrap();
    }

    #[test]
    fn spawn_with_env() {
        let mut env = HashMap::new();
        env.insert("LUNE_TEST_VAR".to_string(), "42".to_string());

        let (_handle, mut reader) = PtyHandle::spawn(
            "/bin/sh",
            &["-c", "echo $LUNE_TEST_VAR"],
            None,
            &env,
            TermSize::default(),
        )
        .expect("spawn failed");

        std::thread::sleep(std::time::Duration::from_millis(200));
        let mut output = vec![0u8; 4096];
        let n = reader.read(&mut output).unwrap_or(0);
        let text = String::from_utf8_lossy(&output[..n]);
        assert!(text.contains("42"), "Expected '42' in output: {text:?}");
    }

    #[test]
    fn spawn_invalid_command_fails() {
        let result = PtyHandle::spawn(
            "/nonexistent/binary",
            &[],
            None,
            &HashMap::new(),
            TermSize::default(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn resize_succeeds() {
        let (mut handle, _reader) = PtyHandle::spawn(
            "/bin/cat",
            &[],
            None,
            &HashMap::new(),
            TermSize::new(24, 80),
        )
        .expect("spawn failed");

        let new_size = TermSize::new(40, 120);
        handle.resize(new_size).unwrap();
        assert_eq!(handle.size(), new_size);

        handle.kill().unwrap();
    }
}
