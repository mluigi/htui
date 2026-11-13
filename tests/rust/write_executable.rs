//! Writing a fixture executable without opening the `ETXTBSY` window (TOOL-16).
//!
//! One source, compiled into each crate's `it` binary that needs it through
//! `#[path = "../../../../tests/rust/write_executable.rs"] mod write_executable;` in its
//! `tests/it/main.rs`: a helper shared by test binaries of three crates, with no feature and no
//! library API.
//!
//! The window: a test that writes a script and then executes it races every `fork` the other
//! tests of the same process make. A child forked while this process holds the write handle
//! inherits it, and `execve` of the script answers `ETXTBSY` ("Text file busy") until that child
//! execs or exits. Since TOOL-15 a crate's integration tests share one process, and since TOOL-16
//! the gate runs them at default threads, so the other test can be any test of the crate. Here a
//! `/bin/sh` child writes the file: the only process that ever holds it open for writing is that
//! child, and it has exited before this returns.
#![cfg(unix)]
#![allow(dead_code)] // each including binary uses what it needs

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// Writes `contents` at `path` with mode 0755, creating the parent directory.
///
/// # Panics
/// When the directory cannot be made or `/bin/sh` cannot write the file.
pub fn write_executable(path: &Path, contents: impl AsRef<[u8]>) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create the executable's directory");
    }
    let mut child = Command::new("/bin/sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .expect("run /bin/sh to write the executable");
    child
        .stdin
        .take()
        .expect("a piped stdin")
        .write_all(contents.as_ref())
        .expect("hand the contents to /bin/sh");
    let status = child.wait().expect("wait for /bin/sh");
    assert!(
        status.success(),
        "/bin/sh could not write {}: {status}",
        path.display()
    );
}
