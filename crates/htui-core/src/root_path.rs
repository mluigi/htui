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
    /// The directory exists and could not be read (MOD-49, blueprint D3). Only [`list_dirs`]
    /// answers it: [`canonical_root`] never reads a directory's entries.
    #[error("`{}` cannot be read on this box", .0.display())]
    Unreadable(PathBuf),
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

/// One entry of a [`DirListing`] (MOD-49 P4): a directory, or a link that resolves to one.
///
/// Carries the entry's **name** only — never a path and never what a link points at (`R-BOX-4`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// The file name, UTF-8 (a name that isn't is skipped).
    pub name: String,
    /// Whether the entry is a symlink that resolves to a directory; the picker marks it `@`.
    pub is_link: bool,
}

/// What [`list_dirs`] found in one directory (MOD-49 P3/P4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirListing {
    /// The directory listed, **exactly as given**: never canonicalised, so it never names a link's
    /// target, and the picker can match a reply to the path it asked for (P9).
    pub path: String,
    /// The directories, in byte order of their names, at most `cap` of them.
    pub entries: Vec<DirEntry>,
    /// How many more qualifying entries the cap left out (`+N more`; nothing is cut silently).
    pub more: usize,
}

/// The subdirectories of `path`, or why it cannot be listed (MOD-49 P4).
///
/// Checked in [`canonical_root`]'s order — relative, missing, dangling, not a directory — then
/// [`RootRefusal::Unreadable`] when `read_dir` itself fails (blueprint D3). Kept: directories, and
/// links that resolve to a directory (`is_link`). Dropped: files, dangling links, links to files,
/// names that aren't UTF-8, entries `read_dir` yields as errors, and `.`-prefixed names unless
/// `show_hidden` (blueprint D6). Every qualifying name is read and sorted before `cap` applies, so
/// which entries are shown never depends on the order the filesystem returned them (blueprint D5).
///
/// Sync and `std::fs` only, like [`canonical_root`]: `htui`'s store worker wraps it in
/// `spawn_blocking`.
///
/// # Errors
/// [`RootRefusal`], carrying `path` as given.
pub fn list_dirs(path: &Path, show_hidden: bool, cap: usize) -> Result<DirListing, RootRefusal> {
    let typed = || path.to_path_buf();
    if !path.is_absolute() {
        return Err(RootRefusal::Relative(typed()));
    }
    let link = std::fs::symlink_metadata(path).map_err(|_| RootRefusal::Missing(typed()))?;
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(RootRefusal::NotADirectory(typed())),
        Err(_) if link.is_symlink() => return Err(RootRefusal::Dangling(typed())),
        Err(_) => return Err(RootRefusal::Missing(typed())),
    }
    let read = std::fs::read_dir(path).map_err(|_| RootRefusal::Unreadable(typed()))?;

    let mut entries: Vec<DirEntry> = read
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !show_hidden && name.starts_with('.') {
                return None;
            }
            let kind = entry.file_type().ok()?;
            // Only a link is followed, and only to ask whether it ends at a directory: what it
            // points at is never kept (`R-BOX-4`).
            let is_link = if kind.is_dir() {
                false
            } else if kind.is_symlink()
                && std::fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir())
            {
                true
            } else {
                return None;
            };
            Some(DirEntry { name, is_link })
        })
        .collect();
    entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    let more = entries.len().saturating_sub(cap);
    entries.truncate(cap);
    Ok(DirListing {
        path: path.to_string_lossy().into_owned(),
        entries,
        more,
    })
}

#[cfg(test)]
mod tests {
    use super::{DirEntry, DirListing, RootRefusal, canonical_root, list_dirs};
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

    /// The names of a listing, in order.
    fn names(listing: &DirListing) -> Vec<&str> {
        listing
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

    /// Directories in `dir`, one per name.
    fn mkdirs(dir: &Path, names: &[&str]) {
        for name in names {
            std::fs::create_dir(dir.join(name)).expect("the directory is created");
        }
    }

    /// MOD-49 P4: directories only, in byte order (`B` before `a`), files dropped.
    #[test]
    fn a_listing_holds_directories_only_in_byte_order() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        mkdirs(dir.path(), &["b", "a", "B"]);
        std::fs::write(dir.path().join("f.txt"), b"a file").expect("the file is written");

        let listing = list_dirs(dir.path(), false, 10).expect("a directory is listed");
        assert_eq!(names(&listing), ["B", "a", "b"]);
        assert_eq!(listing.more, 0);
        assert!(listing.entries.iter().all(|entry| !entry.is_link));
    }

    /// MOD-49 P4: `.`-entries are listed only when asked (blueprint D1).
    #[test]
    fn hidden_entries_are_dropped_unless_asked() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        mkdirs(dir.path(), &[".git", "src"]);

        let hidden = list_dirs(dir.path(), false, 10).expect("a directory is listed");
        assert_eq!(names(&hidden), ["src"]);
        let shown = list_dirs(dir.path(), true, 10).expect("a directory is listed");
        assert_eq!(names(&shown), [".git", "src"]);
    }

    /// MOD-49 P4, blueprint D5: the cap keeps the first names in byte order and counts the rest.
    #[test]
    fn the_cap_keeps_the_first_entries_and_counts_the_rest() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        mkdirs(dir.path(), &["e", "c", "a", "d", "b"]);

        let listing = list_dirs(dir.path(), false, 3).expect("a directory is listed");
        assert_eq!(names(&listing), ["a", "b", "c"]);
        assert_eq!(listing.more, 2);
    }

    /// A listing refuses in the guard's order and with its sentences, about the path as given.
    #[test]
    fn a_listing_refuses_as_the_guard_does() {
        let relative = Path::new("x/y");
        assert_eq!(
            list_dirs(relative, false, 10),
            Err(RootRefusal::Relative(relative.to_path_buf()))
        );

        let dir = tempfile::tempdir().expect("a throwaway directory");
        let missing = dir.path().join("gone");
        assert_eq!(
            list_dirs(&missing, false, 10),
            Err(RootRefusal::Missing(missing.clone()))
        );

        let file = dir.path().join("root.txt");
        std::fs::write(&file, b"not a directory").expect("the file is written");
        assert_eq!(
            list_dirs(&file, false, 10),
            Err(RootRefusal::NotADirectory(file.clone()))
        );
    }

    /// MOD-49 P5, blueprint D4: `path` is echoed byte for byte, never normalised.
    #[test]
    fn the_listing_echoes_the_path_as_given() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        mkdirs(dir.path(), &["a"]);
        let typed = format!("{}/./a/", dir.path().display());

        let listing = list_dirs(Path::new(&typed), false, 10).expect("a directory is listed");
        assert_eq!(listing.path, typed);
    }

    /// `R-BOX-4`: a link to a directory is listed under its own name, marked, and nothing in the
    /// listing names what it points at.
    #[cfg(unix)]
    #[test]
    fn a_link_to_a_directory_is_listed_and_marked_without_its_target() {
        let outer = tempfile::tempdir().expect("a throwaway directory");
        let target = outer.path().join("secret-target-name");
        std::fs::create_dir(&target).expect("the target directory is created");
        let dir = outer.path().join("listed");
        std::fs::create_dir(&dir).expect("the listed directory is created");
        std::os::unix::fs::symlink(&target, dir.join("shared")).expect("the link is created");

        let listing = list_dirs(&dir, false, 10).expect("a directory is listed");
        assert_eq!(
            listing.entries,
            [DirEntry {
                name: "shared".to_owned(),
                is_link: true
            }]
        );
        let debug = format!("{listing:?}");
        assert!(
            !debug.contains("secret-target-name"),
            "a listing never names a link's target: {debug}"
        );

        // The target itself is listed only under its own name, one level up.
        let up = list_dirs(outer.path(), false, 10).expect("a directory is listed");
        assert_eq!(names(&up), ["listed", "secret-target-name"]);
    }

    /// MOD-49 P4: a dangling link and a link to a file are not directories, so neither is listed.
    #[cfg(unix)]
    #[test]
    fn a_dangling_link_and_a_link_to_a_file_are_dropped() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        mkdirs(dir.path(), &["kept"]);
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"a file").expect("the file is written");
        std::os::unix::fs::symlink(&file, dir.path().join("to-file")).expect("the link is created");
        std::os::unix::fs::symlink(dir.path().join("nowhere"), dir.path().join("dangling"))
            .expect("the link is created");

        let listing = list_dirs(dir.path(), false, 10).expect("a directory is listed");
        assert_eq!(names(&listing), ["kept"]);
    }

    /// Listing a dangling link itself is the guard's `Dangling`, and the sentence keeps the target
    /// out (`R-BOX-4`).
    #[cfg(unix)]
    #[test]
    fn a_dangling_link_cannot_be_listed() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path().join("secret-target-name"), &link)
            .expect("the link is created");

        let refusal = list_dirs(&link, false, 10).expect_err("a dangling link is refused");
        assert_eq!(refusal, RootRefusal::Dangling(link.clone()));
        let sentence = refusal.to_string();
        assert!(
            !sentence.contains("secret-target-name"),
            "the refusal never names the target: {sentence}"
        );
    }

    /// A name that isn't UTF-8 can't be stored, so it is skipped and the listing still succeeds.
    #[cfg(unix)]
    #[test]
    fn a_name_that_is_not_utf8_is_skipped() {
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().expect("a throwaway directory");
        mkdirs(dir.path(), &["plain"]);
        let odd = dir.path().join(std::ffi::OsStr::from_bytes(b"not\xffutf8"));
        std::fs::create_dir(&odd).expect("the non-UTF-8 directory is created");

        let listing = list_dirs(dir.path(), false, 10).expect("a directory is listed");
        assert_eq!(names(&listing), ["plain"]);
    }

    /// Blueprint D3: a directory `read_dir` can't open is `Unreadable`, not `Missing`. Root reads
    /// a mode-`000` directory anyway, so the test returns early when it runs as root.
    #[cfg(unix)]
    #[test]
    fn a_directory_that_cannot_be_read_is_refused_as_unreadable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("a throwaway directory");
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).expect("the directory is created");
        let mode = |bits| {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(bits))
                .expect("the mode is set");
        };
        mode(0o000);
        if std::fs::read_dir(&locked).is_ok() {
            mode(0o755);
            return;
        }

        let refusal = list_dirs(&locked, false, 10);
        mode(0o755);
        let refusal = refusal.expect_err("an unreadable directory is refused");
        assert_eq!(refusal, RootRefusal::Unreadable(locked.clone()));
        assert_eq!(
            refusal.to_string(),
            format!("`{}` cannot be read on this box", locked.display())
        );
    }
}
