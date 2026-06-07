//! Image preview overlay — decoded via `ratatui-image`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam::channel::Sender;

use crate::primitives::{Buffer, Line, Rect, Style, Widget};
use crate::theme::Theme;
use crate::widgets::modal::{Anchor, Modal, ModalState};

/// State for the image preview overlay.
///
/// Owns the decoded `Protocol` from `ratatui-image`. The image is decoded
/// once when the overlay opens; subsequent renders reuse the cached protocol.
/// Halfblock rendering is used as the default protocol — it works in every
/// terminal, including those without sixel/kitty/iterm2 support.
/// Hard cap on the on-disk size of files the image preview will
/// attempt to decode. Decoding an arbitrary-size file on demand is a
/// peak-memory and latency risk — bounding it here keeps a user
/// fat-fingering a path through the file picker from stalling the
/// editor or thrashing memory.
pub const MAX_IMAGE_PREVIEW_BYTES: u64 = 16 * 1024 * 1024;

/// Lifecycle of an image preview load.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImagePreviewStatus {
    /// No load has been requested (initial / reset state).
    #[default]
    Empty,
    /// A decode is in flight; the render path shows a placeholder.
    Loading,
    /// Decode succeeded; `protocol` holds the encoded image.
    Loaded,
    /// Decode was rejected (size gate, codec error, IO error); `error`
    /// is set.
    Failed,
}

#[derive(Default)]
pub struct ImagePreviewState {
    /// Path of the file currently displayed (shown in the frame title).
    pub path: Option<PathBuf>,
    /// Decoded image protocol — `None` while loading, after failure, or
    /// before any load was requested. Stateful so the encoded image is
    /// rebuilt on terminal resize at the actual popup inner dimensions.
    pub protocol: Option<ratatui_image::protocol::StatefulProtocol>,
    /// Error string from the most recent failed decode, if any.
    pub error: Option<String>,
    /// Monotonic counter incremented on every `begin_load`. Decode
    /// results carry the generation they were dispatched under; results
    /// whose generation no longer matches `self.generation` are stale
    /// (e.g. the user opened another image before the worker finished)
    /// and are dropped on arrival.
    pub generation: u64,
    /// Current lifecycle status — drives the render placeholder.
    pub status: ImagePreviewStatus,
}

impl std::fmt::Debug for ImagePreviewState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImagePreviewState")
            .field("path", &self.path)
            .field("has_protocol", &self.protocol.is_some())
            .field("error", &self.error)
            .field("generation", &self.generation)
            .field("status", &self.status)
            .finish()
    }
}

impl Clone for ImagePreviewState {
    fn clone(&self) -> Self {
        // `Protocol` cannot be cloned because it owns an image-protocol
        // resource. We snapshot path + error and drop the protocol on
        // clone — only `AppState::clone` (test helper) exercises this path
        // and that scope never re-renders the cloned overlay.
        Self {
            path: self.path.clone(),
            protocol: None,
            error: self.error.clone(),
            generation: self.generation,
            status: self.status,
        }
    }
}

impl ImagePreviewState {
    /// Reset to the `Loading` state for `path`, bump the generation,
    /// and return the new generation so the caller can hand it to the
    /// worker that will produce the result. Subsequent stale results
    /// (different generation or different path) are dropped by
    /// [`apply_result`].
    pub fn begin_load(&mut self, path: &Path) -> u64 {
        self.path = Some(path.to_path_buf());
        self.protocol = None;
        self.error = None;
        self.status = ImagePreviewStatus::Loading;
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    /// Apply a decode result that arrived from the worker thread.
    /// No-op when the result is stale, so an abandoned decode can never
    /// clobber the user's current preview.
    pub fn apply_result(&mut self, result: ImageDecodeResult) {
        if result.generation != self.generation {
            return;
        }
        if self.path.as_deref() != Some(result.path.as_path()) {
            return;
        }
        match result.outcome {
            Ok(protocol) => {
                self.protocol = Some(protocol);
                self.error = None;
                self.status = ImagePreviewStatus::Loaded;
            }
            Err(e) => {
                self.protocol = None;
                self.error = Some(e);
                self.status = ImagePreviewStatus::Failed;
            }
        }
    }
}

/// Result posted back to the event loop from an image decode worker.
pub struct ImageDecodeResult {
    /// Generation handed to the worker at dispatch time. Used to drop
    /// stale results when the user navigates to a different preview
    /// before the previous decode finishes.
    pub generation: u64,
    /// Path that was decoded (for path-based stale-result checks too).
    pub path: PathBuf,
    /// Decoded stateful protocol on success; a human-readable error
    /// string on failure (size-gate rejection, IO error, codec error).
    pub outcome: Result<ratatui_image::protocol::StatefulProtocol, String>,
}

impl std::fmt::Debug for ImageDecodeResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageDecodeResult")
            .field("generation", &self.generation)
            .field("path", &self.path)
            .field("ok", &self.outcome.is_ok())
            .finish()
    }
}

/// Dispatch handle for off-thread image decoding.
///
/// Cloned freely; each call to `spawn` launches a one-shot worker that
/// posts its result back on the bound channel and sets the wake flag
/// (so `PollImageDecode` fires immediately on the next event-loop tick).
#[derive(Clone)]
pub struct ImageDecoder {
    tx: Sender<ImageDecodeResult>,
    wake: Arc<AtomicBool>,
}

impl ImageDecoder {
    pub const fn new(tx: Sender<ImageDecodeResult>, wake: Arc<AtomicBool>) -> Self {
        Self { tx, wake }
    }

    /// Spawn a worker that decodes `path` at `generation` and posts the
    /// outcome on the channel. Threads are detached — the worker is a
    /// one-shot and never long-lives the request.
    pub fn spawn(&self, generation: u64, path: PathBuf) {
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        let worker_path = path.clone();
        let spawn_res = std::thread::Builder::new()
            .name("lune-image-decode".to_string())
            .spawn(move || {
                // Isolate decoder panics: `panic = "abort"` would
                // otherwise let a malformed image kill the editor.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    decode_image_blocking(&worker_path)
                }))
                .unwrap_or_else(|_| Err("image decoder panicked".to_string()));
                let _ = tx.send(ImageDecodeResult {
                    generation,
                    path: worker_path,
                    outcome,
                });
                wake.store(true, Ordering::Release);
            });
        if let Err(e) = spawn_res {
            // Spawn refused (e.g. OS thread cap). Synthesize a failure
            // so the overlay leaves Loading and the user sees an error.
            let _ = self.tx.send(ImageDecodeResult {
                generation,
                path,
                outcome: Err(format!("spawn decode worker: {e}")),
            });
            self.wake.store(true, Ordering::Release);
        }
    }
}

/// Open `path`, decode it, and build a `StatefulProtocol` ready for
/// the `StatefulImage` widget. The widget re-encodes on every render
/// where the area changes — so the popup reflows on terminal resize
/// instead of being letterboxed into a fixed pre-encoded rectangle.
///
/// Pure CPU/IO; safe to call from any thread. Rejects files larger
/// than [`MAX_IMAGE_PREVIEW_BYTES`] before opening the decoder so a
/// gigabyte PNG can't allocate hundreds of MB of pixel buffers.
fn decode_image_blocking(path: &Path) -> Result<ratatui_image::protocol::StatefulProtocol, String> {
    use ratatui_image::picker::Picker;
    use std::io::Read;

    // Reject symlinks outright: prevents a workspace link to /proc/*,
    // /sys/*, or a device node from bypassing the size gate via
    // `metadata.len() == 0` while still streaming arbitrary bytes.
    let link_meta = std::fs::symlink_metadata(path).map_err(|e| format!("stat: {e}"))?;
    if link_meta.file_type().is_symlink() {
        return Err(format!("refusing to follow symlink: {}", path.display()));
    }
    if !link_meta.is_file() {
        return Err(format!("not a regular file: {}", path.display()));
    }
    if link_meta.len() > MAX_IMAGE_PREVIEW_BYTES {
        return Err(format!(
            "image too large ({} bytes; max {} bytes)",
            link_meta.len(),
            MAX_IMAGE_PREVIEW_BYTES
        ));
    }

    // Read into memory through a hard byte cap so pseudo-files where
    // `metadata.len() == 0` (e.g. /proc/*) still can't feed unbounded
    // bytes into the decoder.
    let file = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let cap = usize::try_from(link_meta.len()).unwrap_or(0);
    let mut buf = Vec::with_capacity(cap);
    let limit = MAX_IMAGE_PREVIEW_BYTES + 1;
    file.take(limit)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read: {e}"))?;
    if buf.len() as u64 > MAX_IMAGE_PREVIEW_BYTES {
        return Err(format!(
            "image too large (>{MAX_IMAGE_PREVIEW_BYTES} bytes)"
        ));
    }

    let mut reader = image::ImageReader::new(std::io::Cursor::new(buf))
        .with_guessed_format()
        .map_err(|e| format!("format: {e}"))?;
    // Cap decode work: the file-size limit above bounds *compressed* input,
    // but a small highly-compressed file (e.g. a solid-color PNG) can still
    // expand into a huge pixel buffer. These limits bound the decoded
    // allocation so a hostile image can't exhaust memory.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let dyn_img = reader.decode().map_err(|e| format!("decode: {e}"))?;

    // Halfblocks is universally supported. A future enhancement can
    // query stdio at startup for sixel/kitty/iterm2 and pass the
    // detected font size into `Picker::new`.
    let picker = Picker::halfblocks();
    Ok(picker.new_resize_protocol(dyn_img))
}

pub(crate) fn render_image_preview(
    area: Rect,
    buf: &mut Buffer,
    state: &mut ImagePreviewState,
    theme: &Theme,
) {
    let w = area.width.saturating_mul(8) / 10;
    let h = area.height.saturating_mul(8) / 10;
    if w < 20 || h < 5 {
        return;
    }
    let title_text = state.path.as_ref().map_or_else(
        || " Image Preview ".to_string(),
        |p| format!(" {} ", p.display()),
    );
    let mut modal = ModalState::new();
    modal.open();
    Modal::new(theme)
        .title(&title_text)
        .title_style(Style::new().fg(theme.fg).bold())
        .border_style(Style::new().fg(theme.fg_muted))
        .size_cells(w, h)
        .anchor(Anchor::Center)
        .render(area, buf, &mut modal, |_, _| {});
    let Some(inner) = modal.inner_area() else {
        return;
    };

    if let Some(err) = &state.error {
        let msg = format!("Failed to render image: {err}");
        Line::from(msg).style(Style::new().fg(theme.fg)).render(
            Rect::new(inner.x, inner.y + inner.height / 2, inner.width, 1),
            buf,
        );
        return;
    }

    if let Some(protocol) = state.protocol.as_mut() {
        // Stateful widget — `resize_encode_render` re-encodes whenever
        // the area changes (e.g. terminal resize), so the image always
        // fits the current popup inner rect. Halfblocks encoding is
        // cheap; the threaded `ThreadProtocol` path (recommended for
        // sixel/kitty in the upstream docs) isn't needed here.
        use ratatui_core::widgets::StatefulWidget;
        use ratatui_image::StatefulImage;
        use ratatui_image::protocol::StatefulProtocol;
        StatefulImage::<StatefulProtocol>::default().render(inner, buf, protocol);
    } else {
        Line::from("Decoding…")
            .style(Style::new().fg(theme.fg_muted))
            .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);
    }
}
