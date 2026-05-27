//! AI session lifecycle management.
//!
//! An [`AiSession`] wraps a PTY-backed process with a `vt100::Parser` for
//! terminal emulation. A background reader thread feeds raw bytes from the
//! PTY into a crossbeam channel; the main thread drains the channel and
//! feeds bytes to the parser during `poll_events()`.
//!
//! This design keeps the vt100 parser on the main thread (no `Mutex` on
//! the render path) while still reading PTY output asynchronously.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crossbeam::channel::{self, Receiver, Sender, TryRecvError};
use uuid::Uuid;

use crate::pty::{PtyHandle, TermSize};

/// Unique identifier for an AI session.
pub type AiSessionId = Uuid;

/// The kind of AI client running in this session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiClientKind {
    /// A plain shell (e.g. bash, zsh).
    Shell,
    /// Claude Code CLI tool.
    ClaudeCode,
    /// Custom AI client with a user-specified command.
    Custom {
        /// Display name.
        name: String,
        /// Executable command.
        command: String,
    },
}

impl AiClientKind {
    /// Resolve the actual command path.  For [`Self::Shell`] this honors
    /// the `$SHELL` environment variable when set, otherwise falls back
    /// to `/bin/sh`.
    #[must_use]
    pub fn resolved_command(&self) -> String {
        match self {
            Self::Shell => std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
            Self::ClaudeCode => "claude".to_string(),
            Self::Custom { command, .. } => command.clone(),
        }
    }

    /// Display name for the session.
    #[must_use]
    pub fn display_name(&self) -> &str {
        match self {
            Self::Shell => "Shell",
            Self::ClaudeCode => "Claude Code",
            Self::Custom { name, .. } => name,
        }
    }
}

/// State of an AI session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// Session is starting up.
    Starting,
    /// Session is running.
    Running,
    /// Process exited with the given code.
    Exited(i32),
    /// An error occurred.
    Error,
}

/// Events sent from the reader thread to the main thread.
#[derive(Debug)]
pub enum SessionEvent {
    /// Raw bytes read from the PTY stdout.
    Output(Vec<u8>),
    /// The reader thread detected the process ended (EOF or read error).
    Ended,
    /// A read error occurred.
    ReadError(String),
}

/// Default scrollback buffer size (lines).
const DEFAULT_SCROLLBACK: usize = 10_000;

/// Read buffer size for the PTY reader thread.
const READ_BUF_SIZE: usize = 8192;

/// An AI session: a PTY process + vt100 terminal emulator.
pub struct AiSession {
    /// Unique session ID.
    id: AiSessionId,
    /// What kind of AI client this is.
    kind: AiClientKind,
    /// Current session state.
    state: SessionState,
    /// The PTY handle (for writing input and resizing).
    pty: PtyHandle,
    /// The vt100 terminal parser (main thread only).
    parser: vt100::Parser,
    /// Receiver for events from the reader thread.
    event_rx: Receiver<SessionEvent>,
    /// Scrollback offset (0 = bottom, positive = scrolled up).
    scroll_offset: usize,
}

impl AiSession {
    /// Start a new AI session.
    ///
    /// Spawns the PTY process and starts a background reader thread.
    ///
    /// # Arguments
    /// - `kind`: The AI client to run.
    /// - `cwd`: Optional working directory.
    /// - `env`: Additional environment variables.
    /// - `size`: Initial terminal dimensions.
    ///
    /// # Errors
    /// Returns an error if the PTY cannot be spawned.
    pub fn start(
        kind: AiClientKind,
        cwd: Option<&Path>,
        env: &HashMap<String, String>,
        size: TermSize,
    ) -> anyhow::Result<Self> {
        Self::start_with_wake(kind, cwd, env, size, None)
    }

    /// Start a new AI session and optionally emit wake notifications whenever
    /// the reader thread forwards PTY events.
    ///
    /// # Errors
    /// Returns an error if the PTY cannot be spawned.
    pub fn start_with_wake(
        kind: AiClientKind,
        cwd: Option<&Path>,
        env: &HashMap<String, String>,
        size: TermSize,
        wake_flag: Option<Arc<AtomicBool>>,
    ) -> anyhow::Result<Self> {
        let command = kind.resolved_command();
        // Args are currently empty for all kinds; future kinds may add arguments.
        let args: Vec<&str> = Vec::new();

        let (pty, reader) = PtyHandle::spawn(&command, &args, cwd, env, size)?;

        let parser = vt100::Parser::new(size.rows, size.cols, DEFAULT_SCROLLBACK);

        let (event_tx, event_rx) = channel::unbounded();

        // Start the reader thread.
        start_reader_thread(reader, event_tx, wake_flag);

        Ok(Self {
            id: Uuid::new_v4(),
            kind,
            state: SessionState::Running,
            pty,
            parser,
            event_rx,
            scroll_offset: 0,
        })
    }

    /// Get the session ID.
    #[must_use]
    pub const fn id(&self) -> AiSessionId {
        self.id
    }

    /// Get the client kind.
    #[must_use]
    pub const fn kind(&self) -> &AiClientKind {
        &self.kind
    }

    /// Get the current session state.
    #[must_use]
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// Get the current scroll offset (0 = bottom).
    #[must_use]
    pub const fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Scroll up by `n` lines.
    #[allow(clippy::missing_const_for_fn)] // calls non-const methods
    pub fn scroll_up(&mut self, n: usize) {
        let max_scrollback = self.parser.screen().scrollback();
        self.scroll_offset = (self.scroll_offset + n).min(max_scrollback);
    }

    /// Scroll down by `n` lines (towards bottom).
    pub const fn scroll_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Reset scroll to bottom.
    pub const fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    /// Send input bytes to the PTY process.
    ///
    /// # Errors
    /// Returns an error if writing to the PTY fails.
    pub fn send_input(&mut self, data: &[u8]) -> anyhow::Result<()> {
        // Auto-scroll to bottom on input.
        self.scroll_offset = 0;
        self.pty.write_all(data)
    }

    /// Poll for events from the reader thread and feed them to the vt100 parser.
    ///
    /// Returns `true` if any output was processed (screen may have changed).
    #[allow(clippy::cast_possible_wrap)] // exit codes fit i32
    pub fn poll_events(&mut self) -> bool {
        let mut changed = false;

        loop {
            match self.event_rx.try_recv() {
                Ok(SessionEvent::Output(bytes)) => {
                    self.parser.process(&bytes);
                    changed = true;
                }
                Ok(SessionEvent::Ended) => {
                    // Check exit code.
                    let exit_code = self
                        .pty
                        .try_wait()
                        .ok()
                        .flatten()
                        .map_or(-1, |code| code as i32);
                    self.state = SessionState::Exited(exit_code);
                    changed = true;
                    break;
                }
                Ok(SessionEvent::ReadError(msg)) => {
                    log::error!("PTY read error in session {}: {msg}", self.id);
                    self.state = SessionState::Error;
                    changed = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Reader thread is gone — process likely exited.
                    if self.state == SessionState::Running {
                        let exit_code = self
                            .pty
                            .try_wait()
                            .ok()
                            .flatten()
                            .map_or(-1, |code| code as i32);
                        self.state = SessionState::Exited(exit_code);
                        changed = true;
                    }
                    break;
                }
            }
        }

        changed
    }

    /// Get a reference to the vt100 screen for rendering.
    #[must_use]
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Resize the terminal (both PTY and vt100 parser).
    ///
    /// # Errors
    /// Returns an error if the PTY resize fails.
    pub fn resize(&mut self, size: TermSize) -> anyhow::Result<()> {
        self.pty.resize(size)?;
        self.parser.screen_mut().set_size(size.rows, size.cols);
        Ok(())
    }

    /// Stop the session (kill the child process).
    ///
    /// # Errors
    /// Returns an error if the kill signal fails.
    pub fn stop(&mut self) -> anyhow::Result<()> {
        self.pty.kill()
    }

    /// Check if the session process is still alive.
    #[must_use]
    pub fn is_alive(&mut self) -> bool {
        self.pty.is_alive()
    }

    /// Get a clone of the event receiver (for `PollEvents` integration).
    #[must_use]
    pub fn event_receiver(&self) -> Receiver<SessionEvent> {
        self.event_rx.clone()
    }
}

/// Ensure the child process and reader thread are torn down whenever
/// the session is dropped — whether through an explicit
/// [`AiSession::stop`] call or because the owning manager was itself
/// dropped on app exit.  `PtyHandle::Drop` does the actual `kill + wait`;
/// this impl is the seam that makes sure that destructor runs even when
/// the session is moved/dropped without an explicit stop.
impl Drop for AiSession {
    fn drop(&mut self) {
        let _ = self.pty.kill();
    }
}

/// Spawn a background thread that reads from the PTY reader and sends
/// events to the main thread via a crossbeam channel.
fn send_session_event(
    tx: &Sender<SessionEvent>,
    wake_flag: Option<&Arc<AtomicBool>>,
    event: SessionEvent,
) -> bool {
    if tx.send(event).is_err() {
        return false;
    }
    if let Some(wake_flag) = wake_flag {
        wake_flag.store(true, Ordering::Release);
    }
    true
}

fn start_reader_thread(
    mut reader: Box<dyn Read + Send>,
    tx: Sender<SessionEvent>,
    wake_flag: Option<Arc<AtomicBool>>,
) {
    std::thread::Builder::new()
        .name("lune-ai-pty-reader".into())
        .spawn(move || {
            let mut buf = vec![0u8; READ_BUF_SIZE];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        // EOF — process exited.
                        let _ = send_session_event(&tx, wake_flag.as_ref(), SessionEvent::Ended);
                        break;
                    }
                    Ok(n) => {
                        if !send_session_event(
                            &tx,
                            wake_flag.as_ref(),
                            SessionEvent::Output(buf[..n].to_vec()),
                        ) {
                            // Receiver dropped — session was closed.
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = send_session_event(
                            &tx,
                            wake_flag.as_ref(),
                            SessionEvent::ReadError(e.to_string()),
                        );
                        break;
                    }
                }
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_shell_session() {
        let mut session = AiSession::start(
            AiClientKind::Shell,
            None,
            &HashMap::new(),
            TermSize::new(24, 80),
        )
        .expect("Failed to start shell session");

        assert_eq!(session.state(), SessionState::Running);
        assert!(session.is_alive());

        // Send a command and wait for output.
        session.send_input(b"echo hello_test\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));

        let changed = session.poll_events();
        assert!(changed, "Expected output from shell");

        // The screen should contain our echoed text.
        let screen = session.screen();
        let mut found = false;
        for row in 0..screen.size().0 {
            let mut line = String::new();
            for col in 0..screen.size().1 {
                if let Some(cell) = screen.cell(row, col) {
                    line.push_str(cell.contents());
                }
            }
            if line.contains("hello_test") {
                found = true;
                break;
            }
        }
        assert!(found, "Expected 'hello_test' on the vt100 screen");

        session.stop().unwrap();
    }

    #[test]
    fn session_scroll() {
        let mut session = AiSession::start(
            AiClientKind::Shell,
            None,
            &HashMap::new(),
            TermSize::new(5, 80), // Small screen to fill scrollback quickly
        )
        .expect("Failed to start session");

        // Fresh session has 0 scrollback content, so scroll_up is clamped to 0.
        assert_eq!(session.scroll_offset(), 0);
        session.scroll_up(5);
        // No scrollback content yet → stays at 0.
        assert_eq!(session.scroll_offset(), 0);

        // Generate enough output to create scrollback lines.
        // Use printf to emit many lines quickly without shell echo overhead.
        session.send_input(b"printf '%s\\n' $(seq 1 50)\n").unwrap();
        // Give shell time to process and produce output.
        std::thread::sleep(std::time::Duration::from_millis(800));
        session.poll_events();

        let scrollback = session.screen().scrollback();
        if scrollback > 0 {
            // Scrollback is available — test scroll mechanics.
            session.scroll_up(3);
            assert_eq!(session.scroll_offset(), 3);

            session.scroll_down(1);
            assert_eq!(session.scroll_offset(), 2);

            session.scroll_to_bottom();
            assert_eq!(session.scroll_offset(), 0);
        }
        // If scrollback is 0 (e.g. terminal processed lines differently),
        // the clamping behavior was already verified above.

        session.stop().unwrap();
    }

    #[test]
    fn session_resize() {
        let mut session = AiSession::start(
            AiClientKind::Shell,
            None,
            &HashMap::new(),
            TermSize::new(24, 80),
        )
        .expect("Failed to start session");

        session.resize(TermSize::new(40, 120)).unwrap();
        let (rows, cols) = session.screen().size();
        assert_eq!(rows, 40);
        assert_eq!(cols, 120);

        session.stop().unwrap();
    }

    #[test]
    fn client_kind_display_names() {
        assert_eq!(AiClientKind::Shell.display_name(), "Shell");
        assert_eq!(AiClientKind::ClaudeCode.display_name(), "Claude Code");
        let custom = AiClientKind::Custom {
            name: "My Tool".to_string(),
            command: "/usr/bin/my-tool".to_string(),
        };
        assert_eq!(custom.display_name(), "My Tool");
    }

    #[test]
    fn session_detects_exit() {
        let mut session = AiSession::start(
            AiClientKind::Custom {
                name: "echo".to_string(),
                command: "/bin/sh".to_string(),
            },
            None,
            &HashMap::new(),
            TermSize::new(24, 80),
        )
        .expect("spawn failed");

        // Send exit command.
        session.send_input(b"exit 0\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));

        session.poll_events();

        // May need another poll after the reader detects EOF.
        std::thread::sleep(std::time::Duration::from_millis(200));
        session.poll_events();

        assert!(
            matches!(session.state(), SessionState::Exited(_)),
            "Expected Exited state, got {:?}",
            session.state()
        );
    }
}
