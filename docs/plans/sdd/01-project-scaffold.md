# 01 — Project Scaffold

> **Phase:** 1 (Foundation)
> **Estimated effort:** 1 session (~2 hours)
> **Prerequisites:** None — this is the first step.

## Goal

Convert the single-crate `lune-editor` into a Cargo workspace with four sub-crates, declare all dependencies, configure lints/clippy, and set up a minimal CI pipeline so that `cargo build && cargo test && cargo clippy` all pass from day one.

---

## Types & Structures

No application types yet — this plan is purely structural.

### Workspace Layout

```
lune-editor/
├── Cargo.toml                 # [workspace] root
├── crates/
│   ├── lune-core/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── lune-ui/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── lune-ai/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   └── lune-git/
│       ├── Cargo.toml
│       └── src/lib.rs
├── src/
│   └── main.rs                # binary: imports and wires crates
├── tests/                     # integration tests (later)
├── .github/
│   └── workflows/
│       └── ci.yml
└── docs/
```

---

## Implementation Steps

### Step 1: Convert to workspace

1. Rewrite root `Cargo.toml` to declare a `[workspace]` with members:
   ```toml
   [workspace]
   members = ["crates/*"]
   resolver = "2"

   [workspace.package]
   version = "0.1.0"
   edition = "2024"
   license = "MIT"
   rust-version = "1.85"

   [workspace.dependencies]
   # shared dependency versions declared here
   ratatui = "0.29"
   crossterm = "0.28"
   rat-salsa = "0.29"
   rat-widget = "0.29"
   rat-event = "0.29"
   tachyonfx = "0.7"
   ropey = "1"
   uuid = { version = "1", features = ["v4"] }
   similar = "2"
   notify = "7"
   git2 = "0.19"
   portable-pty = "0.8"
   serde = { version = "1", features = ["derive"] }
   toml = "0.8"
   anyhow = "1"
   thiserror = "2"
   tracing = "0.1"
   tracing-subscriber = "0.3"
   ```

2. The binary crate stays at root level:
   ```toml
   [package]
   name = "lune-editor"
   version.workspace = true
   edition.workspace = true

   [[bin]]
   name = "lune"
   path = "src/main.rs"

   [dependencies]
   lune-core = { path = "crates/lune-core" }
   lune-ui = { path = "crates/lune-ui" }
   lune-ai = { path = "crates/lune-ai" }
   lune-git = { path = "crates/lune-git" }
   anyhow.workspace = true
   tracing.workspace = true
   tracing-subscriber.workspace = true
   ```

### Step 2: Create sub-crates

For each crate (`lune-core`, `lune-ui`, `lune-ai`, `lune-git`):

1. Create `crates/<name>/Cargo.toml` with `version.workspace = true`, `edition.workspace = true`.
2. Create `crates/<name>/src/lib.rs` with a placeholder module doc comment and a `pub fn init() {}` stub.
3. Declare only the dependencies each crate actually needs:

| Crate | Dependencies |
|-------|-------------|
| `lune-core` | `ropey`, `uuid`, `similar`, `anyhow`, `thiserror`, `serde` |
| `lune-ui` | `lune-core`, `ratatui`, `crossterm`, `rat-salsa`, `rat-widget`, `rat-event`, `tachyonfx`, `anyhow`, `tracing` |
| `lune-ai` | `lune-core`, `portable-pty`, `anyhow`, `thiserror`, `tracing` |
| `lune-git` | `lune-core`, `git2`, `anyhow`, `thiserror`, `tracing` |

### Step 3: Update main.rs

Replace hello-world with a minimal entry point:

```rust
use anyhow::Result;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("Lune Editor starting");

    // Will be replaced by rat-salsa event loop in 03-event-system
    println!("Lune Editor v{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
```

### Step 4: Configure lints

Add to workspace `Cargo.toml`:

```toml
[workspace.lints.rust]
unsafe_code = "deny"

[workspace.lints.clippy]
all = "warn"
pedantic = "warn"
nursery = "warn"
```

Each sub-crate inherits: `[lints] workspace = true`.

### Step 5: Update .gitignore

Extend `.gitignore` with standard Rust project ignores:

```
/target
*.swp
*.swo
.DS_Store
.env
```

### Step 6: CI pipeline

Create `.github/workflows/ci.yml`:

```yaml
name: CI
on: [push, pull_request]
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
      - run: cargo build --release
```

### Step 7: Verify

```bash
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace
cargo build --release
```

All must pass cleanly.

---

## Acceptance Criteria

- [ ] `cargo build --workspace` compiles with zero warnings
- [ ] `cargo clippy --workspace --all-targets` passes with no warnings
- [ ] `cargo test --workspace` passes (trivially — no tests yet, but no compilation errors)
- [ ] Running `cargo run` prints version info
- [ ] All 4 sub-crates exist with correct dependency declarations
- [ ] CI workflow file exists and would pass on push

---

## Risks

| Risk | Mitigation |
|------|-----------|
| `rat-salsa`/`rat-widget` version incompatibility with latest `ratatui` | Pin compatible versions; check docs.rs for version matrices before declaring |
| Edition 2024 not supported by all deps | Fall back to `edition = "2021"` if any crate fails |
| `portable-pty` may not compile on all targets | Gate behind feature flag or cfg; test on Linux first |
