//! MOD-41 plan D6, D8: the library links no terminal crate. The worker runs on a headless box,
//! so `ratatui` and `crossterm` never reach its normal dependency tree.

use std::process::Command;

/// `cargo tree -p htui-worker -e normal` names neither `ratatui` nor `crossterm`, and does name
/// `htui-orch`, so the tree was really read.
#[test]
fn the_library_links_no_terminal_crate() {
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "htui-worker",
            "-e",
            "normal",
            "--prefix",
            "none",
            "--all-features",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let terminal: Vec<&str> = stdout
        .lines()
        .filter(|line| line.starts_with("ratatui") || line.starts_with("crossterm"))
        .collect();
    assert!(
        terminal.is_empty(),
        "the library links a terminal crate: {terminal:?}"
    );
    assert!(
        stdout.lines().any(|line| line.starts_with("htui-orch")),
        "the tree names htui-orch, so it was read:\n{stdout}"
    );
}
