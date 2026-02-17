use std::path::PathBuf;

use anyhow::Result;

fn main() -> Result<()> {
    // Parse CLI args: optional file/directory path(s).
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut state = lune_ui::app::AppState::new();

    if args.is_empty() {
        // No arguments: auto-open CWD as workspace.
        if let Ok(cwd) = std::env::current_dir() {
            match state.open_workspace(&cwd) {
                Ok(()) => {}
                Err(e) => eprintln!("Warning: could not open workspace {}: {e}", cwd.display()),
            }
        }
    } else {
        // Open any files/directories passed as arguments.
        for arg in &args {
            let path = PathBuf::from(arg);
            if path.is_dir() {
                // Open as workspace.
                match state.open_workspace(&path) {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("Warning: could not open workspace {}: {e}", path.display());
                    }
                }
            } else if path.exists() {
                match state.open_file(&path) {
                    Ok(_id) => {}
                    Err(e) => eprintln!("Warning: could not open {}: {e}", path.display()),
                }
            } else {
                eprintln!("Warning: path not found: {}", path.display());
            }
        }
    }

    // Run the TUI event loop.
    lune_ui::app::run(&mut state)?;

    Ok(())
}
