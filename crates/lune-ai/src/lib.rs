//! # lune-ai
//!
//! AI integration layer for Lune Editor.
//!
//! Manages the embedded AI client (Claude Code) via PTY:
//! - **`AiSession`**: PTY lifecycle management
//! - **Context provider**: Feeds editor state to the AI
//! - **Live Mode**: Streams AI-generated file changes back into the editor
