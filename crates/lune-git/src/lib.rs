//! # lune-git
//!
//! Git integration layer for Lune Editor.
//!
//! Wraps libgit2 to provide:
//! - **[`GitService`]**: Repository discovery, status queries, staging, commits
//! - **[`GutterMarks`]**: Line-level change indicators (added/modified/deleted)
//! - **Diff**: Unified diff generation for the panel ([`FileDiff`], [`DiffHunk`])

pub mod diff;
pub mod gutter;
pub mod service;
pub mod staging;

pub use diff::{DiffHunk, DiffLine, DiffLineKind, FileDiff};
pub use gutter::{GutterMark, GutterMarks};
pub use service::{GitFileStatus, GitService, GitStatus};
