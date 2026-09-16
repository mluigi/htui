//! The per-box root guard (MOD-15 milestone 3, D8; PRD D11): a workspace root or a repo checkout
//! is stored canonical or refused, and a refusal never names a link's target (`R-BOX-4`; the
//! message rule of `htui-agent/src/excerpt.rs:419-430`).

use std::path::{Path, PathBuf};

/// Why a typed path was not stored. Each variant carries the path **as typed**, never what a link
/// points at: a refusal is shown on the status line, and naming the target would leak a path the
/// user did not write.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RootRefusal {
    /// The path is not absolute, so it names nothing on its own.
    #[error("`{}` is not an absolute path", .0.display())]
    Relative(PathBuf),
    /// Nothing is there, or the guard could not look.
    #[error("`{}` does not exist on this box", .0.display())]
    Missing(PathBuf),
    /// A symlink whose target cannot be resolved.
    #[error("`{}` is a link to nothing", .0.display())]
    Dangling(PathBuf),
    /// Something is there, and it is not a directory.
    #[error("`{}` is not a directory", .0.display())]
    NotADirectory(PathBuf),
}

/// The canonical directory `path` names, or why it is refused.
///
/// Checked in a fixed order — relative, missing, dangling, not a directory — so one typed path has
/// exactly one refusal. A link **to** a directory passes and is stored as the directory it
/// resolves to, which is deliberate: `/tmp` is a symlink on more than one box and refusing the
/// root itself would refuse a legitimate checkout (`htui-agent/src/excerpt.rs:396-398`).
/// `canonicalize` runs on plain directories too, so one directory has one stored string.
///
/// Sync and `std::fs` only: the caller (`htui`'s store worker) wraps it in `spawn_blocking`, and
/// this crate's tokio is `macros, rt` (`Cargo.toml`). On Windows `canonicalize` returns a `\\?\`
/// prefix and it is stored as returned; comparing those strings is MOD-16's.
///
/// # Errors
/// [`RootRefusal`], which renders as the sentence the user sees.
pub fn canonical_root(path: &Path) -> Result<PathBuf, RootRefusal> {
    let typed = || path.to_path_buf();
    if !path.is_absolute() {
        return Err(RootRefusal::Relative(typed()));
    }
    // `symlink_metadata` does not follow the last component, so a dangling link is a row that
    // exists here and fails to canonicalize below.
    let meta = std::fs::symlink_metadata(path).map_err(|_| RootRefusal::Missing(typed()))?;
    let canonical = match std::fs::canonicalize(path) {
        Ok(canonical) => canonical,
        Err(_) if meta.is_symlink() => return Err(RootRefusal::Dangling(typed())),
        // A non-link whose `canonicalize` fails — EACCES on a parent, a vanished component —
        // is reported as missing rather than gaining a fifth variant: from where the user sits
        // the path is not reachable, and the guard may not say why in more detail than that.
        Err(_) => return Err(RootRefusal::Missing(typed())),
    };
    let is_dir = std::fs::metadata(&canonical).is_ok_and(|meta| meta.is_dir());
    if !is_dir {
        return Err(RootRefusal::NotADirectory(typed()));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::{RootRefusal, canonical_root};
    use std::path::{Path, PathBuf};

    /// The order is fixed: a relative path is refused before anything is stat'ed, and the four
    /// sentences are the ones the status line shows.
    #[test]
    fn a_relative_path_is_refused_first() {
        let typed = Path::new("x/y");
        assert_eq!(
            canonical_root(typed),
            Err(RootRefusal::Relative(typed.to_path_buf()))
        );

        let p = PathBuf::from("/x");
        assert_eq!(
            RootRefusal::Relative(p.clone()).to_string(),
            "`/x` is not an absolute path"
        );
        assert_eq!(
            RootRefusal::Missing(p.clone()).to_string(),
            "`/x` does not exist on this box"
        );
        assert_eq!(
            RootRefusal::Dangling(p.clone()).to_string(),
            "`/x` is a link to nothing"
        );
        assert_eq!(
            RootRefusal::NotADirectory(p).to_string(),
            "`/x` is not a directory"
        );
    }

    #[test]
    fn a_missing_path_is_missing() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        let typed = dir.path().join("gone");
        assert_eq!(
            canonical_root(&typed),
            Err(RootRefusal::Missing(typed.clone()))
        );
    }

    #[test]
    fn a_directory_is_stored_canonical() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).expect("the nested directory is created");
        let typed = dir.path().join("a").join(".").join("b");

        let stored = canonical_root(&typed).expect("a real directory is stored");
        assert_eq!(
            stored,
            nested
                .canonicalize()
                .expect("the nested directory resolves")
        );
    }

    #[test]
    fn a_file_is_not_a_directory() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        let file = dir.path().join("root.txt");
        std::fs::write(&file, b"not a directory").expect("the file is written");
        assert_eq!(
            canonical_root(&file),
            Err(RootRefusal::NotADirectory(file.clone()))
        );
    }

    /// A root that is a link to a real directory is stored as the directory (PRD D11): refusing it
    /// would refuse every checkout under a `/tmp` that is itself a link.
    #[cfg(unix)]
    #[test]
    fn a_link_to_a_directory_is_stored_canonical() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        let real = dir.path().join("real");
        std::fs::create_dir(&real).expect("the target directory is created");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("the link is created");

        let stored = canonical_root(&link).expect("a link to a directory is stored");
        assert_eq!(stored, real.canonicalize().expect("the target resolves"));
    }

    /// The refusal names what was typed and never where the link pointed.
    #[cfg(unix)]
    #[test]
    fn a_dangling_link_is_refused_without_naming_its_target() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        let target = dir.path().join("secret-target-name");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("the link is created");

        let refusal = canonical_root(&link).expect_err("a dangling link is refused");
        assert_eq!(refusal, RootRefusal::Dangling(link.clone()));

        let sentence = refusal.to_string();
        assert!(
            sentence.contains(&link.display().to_string()),
            "the refusal names the typed path: {sentence}"
        );
        assert!(
            !sentence.contains("secret-target-name"),
            "the refusal never names the target: {sentence}"
        );
    }
}
