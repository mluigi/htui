//! Private per-process directories, for a file another local account must not read (MOD-79 D2).
//!
//! One helper, two callers: `htui-mcp`'s socket directory (`htui-mcp-<pid>-<8 hex>`) and the CLI
//! driver's MCP config (`htui-cli-<pid>-<8 hex>`, [`crate::cli::McpConfigFile`]). Both live under
//! the same base, so whatever isolation lets the relay child reach the socket also lets the CLI read
//! its config.

use std::path::PathBuf;

#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};

/// Creates `<base>/<prefix>-<pid>-<8 hex>` and returns its path. The caller owns the directory and
/// removes it; after a success this function never does.
///
/// On Unix, `<base>` is `$XDG_RUNTIME_DIR` when that is set to an existing absolute directory, else
/// [`std::env::temp_dir`]. The directory is created with mode `0700` and its mode is read back. A
/// directory that grants anything to group or others is removed and refused.
///
/// Elsewhere (Windows), `<base>` is [`std::env::temp_dir`] (`%LOCALAPPDATA%\Temp` for a user), and
/// the directory inherits that directory's ACL: the account, SYSTEM and Administrators (MOD-79,
/// option A). Nothing checks it at run time; that is MOD-16's.
///
/// # Errors
///
/// The `create` error (the name exists, the base is not writable), the metadata read's, or, on Unix
/// only, `"<dir> is not private (mode <octal>)"`.
pub fn create(prefix: &str) -> std::io::Result<PathBuf> {
    // `new_v4`, not `now_v7`: a v7's leading hex is its millisecond, so one pid making two
    // directories in the same millisecond would collide (blueprint H-2).
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let dir = base().join(format!("{prefix}-{}-{}", std::process::id(), &suffix[..8]));
    // `channel.rs`'s body, moved verbatim (MOD-79 D2): create with `0700`, then read the mode back,
    // because a filesystem may ignore the requested bits and the check is what the caller relies on.
    #[cfg(unix)]
    {
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let mode = std::fs::metadata(&dir)?.permissions().mode();
        if mode & 0o077 != 0 {
            let _ = std::fs::remove_dir(&dir);
            return Err(std::io::Error::other(format!(
                "{} is not private (mode {mode:o})",
                dir.display()
            )));
        }
    }
    // Option A: the per-user temp directory's inherited ACL is the whole of the privacy, and no
    // `unsafe` DACL call is reachable under the workspace's `forbid` (MOD-79 plan).
    #[cfg(not(unix))]
    std::fs::create_dir(&dir)?;
    Ok(dir)
}

/// `$XDG_RUNTIME_DIR` through [`base_from`]: the env read is the only part a test cannot drive
/// (`set_var` is `unsafe`, and the workspace forbids `unsafe`; blueprint H-7).
#[cfg(unix)]
fn base() -> PathBuf {
    base_from(std::env::var_os("XDG_RUNTIME_DIR"))
}

/// `xdg` when it is an absolute path to an existing directory, else the temp directory. Moved
/// verbatim from `htui-mcp`'s `channel.rs` (MOD-11 H-12's socket base), with the env read lifted
/// out.
#[cfg(unix)]
fn base_from(xdg: Option<OsString>) -> PathBuf {
    xdg.map(PathBuf::from)
        .filter(|dir| dir.is_absolute() && dir.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

/// The temp directory, which on Windows is per user (MOD-79 option A).
#[cfg(not(unix))]
fn base() -> PathBuf {
    std::env::temp_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Removes a directory a test created, so a run leaves nothing behind.
    struct Cleanup(PathBuf);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn a_private_dir_is_named_after_its_prefix_and_pid() {
        let dir = Cleanup(create("mod79-test").expect("the directory is created"));
        let name = dir
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a UTF-8 name");
        let stem = format!("mod79-test-{}-", std::process::id());
        let suffix = name
            .strip_prefix(&stem)
            .expect("the prefix and the pid lead the name");
        assert_eq!(suffix.len(), 8, "{name}");
        assert!(
            suffix
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "eight lowercase hex digits: {name}"
        );
        assert_eq!(dir.0.parent(), Some(base().as_path()));
        assert!(dir.0.is_dir());
    }

    #[test]
    fn two_private_dirs_never_share_a_name() {
        let one = Cleanup(create("mod79-test").expect("the first directory"));
        let two = Cleanup(create("mod79-test").expect("the second directory"));
        assert_ne!(one.0, two.0);
    }

    #[cfg(unix)]
    #[test]
    fn a_private_dir_has_mode_0700() {
        let dir = Cleanup(create("mod79-test").expect("the directory is created"));
        let mode = std::fs::metadata(&dir.0)
            .expect("its metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700, "{mode:o}");
    }

    #[cfg(unix)]
    #[test]
    fn the_base_is_xdg_runtime_dir_only_when_absolute_and_a_directory() {
        let temp = std::env::temp_dir();
        assert_eq!(base_from(None), temp);
        assert_eq!(base_from(Some("relative".into())), temp);
        assert_eq!(base_from(Some("/nonexistent-mod79".into())), temp);
        let runtime = tempfile::tempdir().expect("a stand-in runtime directory");
        assert_eq!(
            base_from(Some(runtime.path().as_os_str().to_owned())),
            runtime.path()
        );
    }
}
