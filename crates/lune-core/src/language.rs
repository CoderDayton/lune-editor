//! Language detection and registry.
//!
//! Maps file extensions and shebangs to [`LanguageId`] values, enabling
//! downstream crates to select the appropriate highlighter or grammar.

use std::collections::HashMap;
use std::path::Path;

// ── Language ID ────────────────────────────────────────────────────────

/// Opaque identifier for a programming language.
///
/// Uses a `&'static str` internally so it can be cheaply copied and compared.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LanguageId(pub &'static str);

impl LanguageId {
    /// Create a new `LanguageId` from a static string.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The language name as a string slice.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for LanguageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

// ── Well-known languages ──────────────────────────────────────────────

/// Well-known language constants.
pub mod lang {
    use super::LanguageId;

    pub const RUST: LanguageId = LanguageId::new("Rust");
    pub const PYTHON: LanguageId = LanguageId::new("Python");
    pub const JAVASCRIPT: LanguageId = LanguageId::new("JavaScript");
    pub const TYPESCRIPT: LanguageId = LanguageId::new("TypeScript");
    pub const TSX: LanguageId = LanguageId::new("TSX");
    pub const JSX: LanguageId = LanguageId::new("JSX");
    pub const JSON: LanguageId = LanguageId::new("JSON");
    pub const TOML: LanguageId = LanguageId::new("TOML");
    pub const YAML: LanguageId = LanguageId::new("YAML");
    pub const MARKDOWN: LanguageId = LanguageId::new("Markdown");
    pub const C: LanguageId = LanguageId::new("C");
    pub const CPP: LanguageId = LanguageId::new("C++");
    pub const GO: LanguageId = LanguageId::new("Go");
    pub const HTML: LanguageId = LanguageId::new("HTML");
    pub const CSS: LanguageId = LanguageId::new("CSS");
    pub const SHELL: LanguageId = LanguageId::new("Shell");
    pub const LUA: LanguageId = LanguageId::new("Lua");
    pub const RUBY: LanguageId = LanguageId::new("Ruby");
    pub const JAVA: LanguageId = LanguageId::new("Java");
    pub const PLAIN_TEXT: LanguageId = LanguageId::new("Plain Text");
}

// ── Language Registry ─────────────────────────────────────────────────

/// Maps file extensions and shebangs to `LanguageId`.
pub struct LanguageRegistry {
    /// Extension (without dot) to language ID.
    extension_map: HashMap<&'static str, LanguageId>,
    /// Shebang interpreter name to language ID.
    shebang_map: HashMap<&'static str, LanguageId>,
}

impl LanguageRegistry {
    /// Build the default registry with common language mappings.
    #[must_use]
    pub fn new() -> Self {
        let mut ext = HashMap::new();
        let mut shebang = HashMap::new();

        // Rust
        ext.insert("rs", lang::RUST);

        // Python
        ext.insert("py", lang::PYTHON);
        ext.insert("pyi", lang::PYTHON);
        ext.insert("pyw", lang::PYTHON);
        shebang.insert("python", lang::PYTHON);
        shebang.insert("python3", lang::PYTHON);

        // JavaScript / TypeScript
        ext.insert("js", lang::JAVASCRIPT);
        ext.insert("mjs", lang::JAVASCRIPT);
        ext.insert("cjs", lang::JAVASCRIPT);
        ext.insert("jsx", lang::JSX);
        ext.insert("ts", lang::TYPESCRIPT);
        ext.insert("mts", lang::TYPESCRIPT);
        ext.insert("cts", lang::TYPESCRIPT);
        ext.insert("tsx", lang::TSX);
        shebang.insert("node", lang::JAVASCRIPT);

        // Data formats
        ext.insert("json", lang::JSON);
        ext.insert("jsonc", lang::JSON);
        ext.insert("toml", lang::TOML);
        ext.insert("yaml", lang::YAML);
        ext.insert("yml", lang::YAML);

        // Markdown
        ext.insert("md", lang::MARKDOWN);
        ext.insert("mdx", lang::MARKDOWN);
        ext.insert("markdown", lang::MARKDOWN);

        // C / C++
        ext.insert("c", lang::C);
        ext.insert("h", lang::C);
        ext.insert("cpp", lang::CPP);
        ext.insert("cxx", lang::CPP);
        ext.insert("cc", lang::CPP);
        ext.insert("hpp", lang::CPP);
        ext.insert("hxx", lang::CPP);

        // Go
        ext.insert("go", lang::GO);

        // Web
        ext.insert("html", lang::HTML);
        ext.insert("htm", lang::HTML);
        ext.insert("css", lang::CSS);
        ext.insert("scss", lang::CSS);

        // Shell
        ext.insert("sh", lang::SHELL);
        ext.insert("bash", lang::SHELL);
        ext.insert("zsh", lang::SHELL);
        ext.insert("fish", lang::SHELL);
        shebang.insert("bash", lang::SHELL);
        shebang.insert("sh", lang::SHELL);
        shebang.insert("zsh", lang::SHELL);

        // Lua
        ext.insert("lua", lang::LUA);
        shebang.insert("lua", lang::LUA);

        // Ruby
        ext.insert("rb", lang::RUBY);
        shebang.insert("ruby", lang::RUBY);

        // Java
        ext.insert("java", lang::JAVA);

        // Plain text
        ext.insert("txt", lang::PLAIN_TEXT);

        Self {
            extension_map: ext,
            shebang_map: shebang,
        }
    }

    /// Detect language from a file path (extension-based).
    #[must_use]
    pub fn detect_from_path(&self, path: &Path) -> Option<LanguageId> {
        let ext = path.extension()?.to_str()?;
        self.extension_map.get(ext).copied()
    }

    /// Detect language from the first line of content (shebang-based).
    ///
    /// Parses `#!/usr/bin/env <interpreter>` and `#!/path/to/<interpreter>`.
    #[must_use]
    pub fn detect_from_shebang(&self, first_line: &str) -> Option<LanguageId> {
        let line = first_line.trim();
        if !line.starts_with("#!") {
            return None;
        }
        let after_hash_bang = line[2..].trim();

        // Handle `#!/usr/bin/env <interpreter>` or `#!/usr/bin/env -S <interpreter>`
        let interpreter = if after_hash_bang.contains("env") {
            after_hash_bang
                .split_whitespace()
                .find(|s| !s.starts_with('-') && !s.contains("env") && !s.contains('/'))
        } else {
            // Direct path: `#!/usr/bin/python3`
            after_hash_bang
                .split_whitespace()
                .next()
                .and_then(|path_str| path_str.rsplit('/').next())
        };

        let interp = interpreter?;

        // Strip version suffixes: `python3.11` -> `python3` -> `python`
        let base = interp
            .split('.')
            .next()
            .unwrap_or(interp)
            .trim_end_matches(|c: char| c.is_ascii_digit());

        // Try full name first (e.g., "python3"), then stripped (e.g., "python").
        self.shebang_map
            .get(interp.split('.').next().unwrap_or(interp))
            .or_else(|| self.shebang_map.get(base))
            .copied()
    }

    /// Detect language by trying path first, then shebang from content.
    #[must_use]
    pub fn detect(&self, path: &Path, first_line: Option<&str>) -> Option<LanguageId> {
        self.detect_from_path(path)
            .or_else(|| first_line.and_then(|line| self.detect_from_shebang(line)))
    }

    /// Get the extension map (for testing or introspection).
    #[must_use]
    pub fn extension_count(&self) -> usize {
        self.extension_map.len()
    }

    /// All known language IDs (deduplicated).
    #[must_use]
    pub fn known_languages(&self) -> Vec<LanguageId> {
        let mut langs: Vec<LanguageId> = self.extension_map.values().copied().collect();
        langs.sort_by_key(|l| l.0);
        langs.dedup();
        langs
    }
}

impl Default for LanguageRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn reg() -> LanguageRegistry {
        LanguageRegistry::new()
    }

    // ── Extension detection ───────────────────────────────────────────

    #[test]
    fn detect_rust_from_extension() {
        let r = reg();
        assert_eq!(
            r.detect_from_path(&PathBuf::from("main.rs")),
            Some(lang::RUST)
        );
    }

    #[test]
    fn detect_python_from_extension() {
        let r = reg();
        assert_eq!(
            r.detect_from_path(&PathBuf::from("script.py")),
            Some(lang::PYTHON)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("types.pyi")),
            Some(lang::PYTHON)
        );
    }

    #[test]
    fn detect_javascript_variants() {
        let r = reg();
        assert_eq!(
            r.detect_from_path(&PathBuf::from("app.js")),
            Some(lang::JAVASCRIPT)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("app.mjs")),
            Some(lang::JAVASCRIPT)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("app.jsx")),
            Some(lang::JSX)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("app.ts")),
            Some(lang::TYPESCRIPT)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("app.tsx")),
            Some(lang::TSX)
        );
    }

    #[test]
    fn detect_c_cpp() {
        let r = reg();
        assert_eq!(r.detect_from_path(&PathBuf::from("main.c")), Some(lang::C));
        assert_eq!(
            r.detect_from_path(&PathBuf::from("header.h")),
            Some(lang::C)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("main.cpp")),
            Some(lang::CPP)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("main.hpp")),
            Some(lang::CPP)
        );
    }

    #[test]
    fn detect_data_formats() {
        let r = reg();
        assert_eq!(
            r.detect_from_path(&PathBuf::from("config.json")),
            Some(lang::JSON)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("Cargo.toml")),
            Some(lang::TOML)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("config.yaml")),
            Some(lang::YAML)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("config.yml")),
            Some(lang::YAML)
        );
    }

    #[test]
    fn detect_markdown() {
        let r = reg();
        assert_eq!(
            r.detect_from_path(&PathBuf::from("README.md")),
            Some(lang::MARKDOWN)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("doc.mdx")),
            Some(lang::MARKDOWN)
        );
    }

    #[test]
    fn detect_shell() {
        let r = reg();
        assert_eq!(
            r.detect_from_path(&PathBuf::from("script.sh")),
            Some(lang::SHELL)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("script.bash")),
            Some(lang::SHELL)
        );
    }

    #[test]
    fn detect_web() {
        let r = reg();
        assert_eq!(
            r.detect_from_path(&PathBuf::from("index.html")),
            Some(lang::HTML)
        );
        assert_eq!(
            r.detect_from_path(&PathBuf::from("style.css")),
            Some(lang::CSS)
        );
    }

    #[test]
    fn unknown_extension_returns_none() {
        let r = reg();
        assert_eq!(r.detect_from_path(&PathBuf::from("file.xyz")), None);
        assert_eq!(r.detect_from_path(&PathBuf::from("Makefile")), None);
    }

    // ── Shebang detection ─────────────────────────────────────────────

    #[test]
    fn shebang_env_python() {
        let r = reg();
        assert_eq!(
            r.detect_from_shebang("#!/usr/bin/env python3"),
            Some(lang::PYTHON)
        );
        assert_eq!(
            r.detect_from_shebang("#!/usr/bin/env python"),
            Some(lang::PYTHON)
        );
    }

    #[test]
    fn shebang_direct_path() {
        let r = reg();
        assert_eq!(
            r.detect_from_shebang("#!/usr/bin/python3"),
            Some(lang::PYTHON)
        );
        assert_eq!(r.detect_from_shebang("#!/bin/bash"), Some(lang::SHELL));
        assert_eq!(r.detect_from_shebang("#!/bin/sh"), Some(lang::SHELL));
    }

    #[test]
    fn shebang_env_with_flags() {
        let r = reg();
        assert_eq!(
            r.detect_from_shebang("#!/usr/bin/env -S python3"),
            Some(lang::PYTHON)
        );
    }

    #[test]
    fn shebang_node() {
        let r = reg();
        assert_eq!(
            r.detect_from_shebang("#!/usr/bin/env node"),
            Some(lang::JAVASCRIPT)
        );
    }

    #[test]
    fn shebang_ruby() {
        let r = reg();
        assert_eq!(
            r.detect_from_shebang("#!/usr/bin/env ruby"),
            Some(lang::RUBY)
        );
    }

    #[test]
    fn no_shebang_returns_none() {
        let r = reg();
        assert_eq!(r.detect_from_shebang("fn main() {"), None);
        assert_eq!(r.detect_from_shebang(""), None);
    }

    // ── Combined detection ────────────────────────────────────────────

    #[test]
    fn detect_prefers_extension_over_shebang() {
        let r = reg();
        // .py file with bash shebang => should still be Python (extension wins).
        assert_eq!(
            r.detect(&PathBuf::from("script.py"), Some("#!/bin/bash")),
            Some(lang::PYTHON)
        );
    }

    #[test]
    fn detect_falls_back_to_shebang() {
        let r = reg();
        // No extension recognized, but shebang says python.
        assert_eq!(
            r.detect(&PathBuf::from("myscript"), Some("#!/usr/bin/env python3")),
            Some(lang::PYTHON)
        );
    }

    #[test]
    fn detect_no_info_returns_none() {
        let r = reg();
        assert_eq!(r.detect(&PathBuf::from("Makefile"), None), None);
    }

    #[test]
    fn known_languages_not_empty() {
        let r = reg();
        let langs = r.known_languages();
        assert!(!langs.is_empty());
        // Should contain at least Rust and Python.
        assert!(langs.contains(&lang::RUST));
        assert!(langs.contains(&lang::PYTHON));
    }
}
