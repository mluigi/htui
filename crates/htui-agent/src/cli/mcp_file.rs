//! A CLI session's `--mcp-config` file (MOD-79): the JSON that used to ride on the argv, where only
//! the session's own account can read it.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::driver::McpServerSpec;

/// The directory's name prefix: `htui-cli-<pid>-<8 hex>`.
const DIR_PREFIX: &str = "htui-cli";
/// The file's name inside it.
const FILE_NAME: &str = "mcp.json";

/// [`super::mcp_config`]'s JSON in `<dir>/mcp.json`, with `<dir>` a fresh
/// [`crate::private_dir::create`] directory. On Unix the file has mode `0600`.
///
/// The session owns it for its whole life (MOD-79 D3): [`super::open_session`] moves it into the
/// session task, so every exit drops it, the abort of a session that never sent `system/init`
/// included. `Drop` removes the file and then the directory, best effort. `Debug` prints the
/// path, never the contents. Move-only on purpose: a copy would outlive the session.
pub struct McpConfigFile {
    /// `<dir>/mcp.json`: what `--mcp-config=` names.
    path: PathBuf,
    /// The private directory around it, removed after the file.
    dir: PathBuf,
}

impl McpConfigFile {
    /// Writes `servers`' config. `Ok(None)` for an empty slice, with no directory created.
    ///
    /// # Errors
    ///
    /// [`crate::private_dir::create`]'s, a path that is not UTF-8 (the argv is `String`s), or the
    /// file's create/write error. Every error removes what was already created.
    pub fn write(servers: &[McpServerSpec]) -> std::io::Result<Option<Self>> {
        let Some(json) = super::mcp_config(servers) else {
            return Ok(None);
        };
        let dir = crate::private_dir::create(DIR_PREFIX)?;
        // The guard exists before anything else can fail, so every `?` below removes the
        // directory and whatever part of the file was written (blueprint H-10).
        let file = Self {
            path: dir.join(FILE_NAME),
            dir,
        };
        // `channel.rs`'s socket-path check: `argv` is `String`s, and a lossy path would name a
        // file that does not exist.
        if file.path.to_str().is_none() {
            return Err(std::io::Error::other("the MCP config path is not UTF-8"));
        }
        // `create_new`: a fresh directory holds no file, so one already there is never ours to
        // reuse. Synchronous `std::fs`, as `channel.rs`'s `bind`: a few hundred bytes, and `Drop`
        // is synchronous anyway (blueprint G-4).
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        options.open(&file.path)?.write_all(json.as_bytes())?;
        Ok(Some(file))
    }

    /// The file, for `--mcp-config=`.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for McpConfigFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

/// Written by hand rather than derived, so "the path, never the contents" is visible here
/// (blueprint H-6): the file holds the session's `HTUI_MCP_TOKEN`.
impl core::fmt::Debug for McpConfigFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("McpConfigFile")
            .field("path", &self.path)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// One server whose `env` carries a token a test can look for.
    fn server() -> McpServerSpec {
        McpServerSpec {
            name: "htui".to_owned(),
            command: "/abs/htui".to_owned(),
            args: vec!["mcp".to_owned()],
            env: BTreeMap::from([(
                "HTUI_MCP_TOKEN".to_owned(),
                "planted-mod79-token".to_owned(),
            )]),
        }
    }

    fn written() -> McpConfigFile {
        McpConfigFile::write(&[server()])
            .expect("the config is written")
            .expect("one server writes a file")
    }

    fn dir_of(file: &McpConfigFile) -> PathBuf {
        file.path()
            .parent()
            .expect("the file has a directory")
            .to_path_buf()
    }

    #[test]
    fn no_servers_write_no_file() {
        assert!(
            McpConfigFile::write(&[])
                .expect("nothing to write")
                .is_none()
        );
    }

    #[test]
    fn the_file_holds_the_clis_config_json() {
        let file = written();
        let text = std::fs::read_to_string(file.path()).expect("the file is readable");
        assert_eq!(Some(text), super::super::mcp_config(&[server()]));
        assert_eq!(file.path().file_name(), Some("mcp.json".as_ref()));
        let dir = dir_of(&file);
        let name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a UTF-8 name");
        let stem = format!("htui-cli-{}-", std::process::id());
        assert!(name.starts_with(&stem), "{name}");
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_0600_in_a_0700_directory() {
        use std::os::unix::fs::PermissionsExt as _;

        let file = written();
        let mode = |path: &Path| {
            std::fs::metadata(path)
                .expect("its metadata")
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(file.path()), 0o600);
        assert_eq!(mode(&dir_of(&file)), 0o700);
    }

    #[test]
    fn dropping_the_guard_removes_the_file_and_its_directory() {
        let file = written();
        let (path, dir) = (file.path().to_path_buf(), dir_of(&file));
        assert!(path.is_file());
        drop(file);
        assert!(!path.exists(), "{}", path.display());
        assert!(!dir.exists(), "{}", dir.display());
    }

    #[test]
    fn debug_prints_the_path_and_never_the_contents() {
        let file = written();
        let debug = format!("{file:?}");
        assert!(debug.contains(&format!("{:?}", file.path())), "{debug}");
        assert!(!debug.contains("planted-mod79-token"), "{debug}");
        assert!(!debug.contains("mcpServers"), "{debug}");
    }
}
