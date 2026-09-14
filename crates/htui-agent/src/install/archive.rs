//! The two archive shapes the registry publishes, and the tree one of them becomes (plan MOD-20
//! D10).
//!
//! The only file in the workspace that names `zip`, `tar` or `flate2`. Everything above it speaks
//! in [`ArchiveFormat`] and a directory, so a third shape is a variant here and an arm below, and
//! nowhere else.
//!
//! [`unpack`] is **synchronous** and reads a file rather than a stream, for two reasons that are
//! not negotiable: a zip's directory is at its *end*, so a streamed zip cannot be read at all, and
//! plan D2 says the digest is checked over the whole download before a byte of it is trusted. The
//! caller runs it under `spawn_blocking`, which is also why it takes a token and a counter rather
//! than returning progress — a blocking thread cannot be awaited on and cannot be aborted
//! (blueprint P-3), so it polls the one and publishes the other.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tokio_util::sync::CancellationToken;

use super::InstallError;

/// The unix mode bits that say *this entry is a symlink*.
const S_IFMT: u32 = 0o170000;
/// The value those bits take for a symlink.
const S_IFLNK: u32 = 0o120000;

/// The longest symlink target this installer will read out of a zip entry.
///
/// A zip stores a link's target as the entry's *body*, so reading it is reading an attacker's
/// length: `4096` is `PATH_MAX` on Linux and longer than any filesystem here would store, and a
/// bound is what keeps a 2 GB entry flagged `S_IFLNK` from being read into a `String`.
const MAX_LINK_BYTES: u64 = 4096;

wire_enum!(
    /// The archive shapes this installer unpacks (plan D10).
    ///
    /// Two, because the registry publishes two. A third would be a variant here and an arm in
    /// the unpacker, which is the point of a closed vocabulary: a `.7z` entry is
    /// [`PlanError::Unsupported`](super::PlanError::Unsupported) before consent rather than a
    /// failure after a 600 MB download.
    ArchiveFormat {
        /// A `.zip`, deflate or stored.
        Zip => "zip",
        /// A `.tar.gz` or `.tgz`.
        TarGz => "tar.gz",
    }
);

impl ArchiveFormat {
    /// The format a URL names, from its **path** — query and fragment stripped first, because a
    /// signed CDN link ends in a signature and not in a suffix.
    #[must_use]
    pub fn for_url(url: &str) -> Option<Self> {
        let path = url.split(['?', '#']).next().unwrap_or(url).to_lowercase();
        if path.ends_with(".zip") {
            Some(Self::Zip)
        } else if path.ends_with(".tar.gz") || path.ends_with(".tgz") {
            Some(Self::TarGz)
        } else {
            None
        }
    }
}

/// Writes `archive` out as a tree under `into` — the whole tree, not the `cmd` alone.
///
/// Synchronous on purpose (see the module doc); the caller owns the `spawn_blocking`. Per entry,
/// in this order:
///
/// - `cancel` is read **before** the entry is touched. That is the only way a blocking thread
///   stops: `JoinHandle::abort` does not reach one (blueprint P-3, hazard H-8).
/// - The entry's path goes through `safe_target`, which is this crate's own check and not the
///   archive crate's (hazard H-16).
/// - The path is then walked down from `into` one component at a time by `descend`, which
///   refuses the moment a component that already exists is a symlink. That is what makes the
///   lexical check above enough: nothing is ever created, written or chmod'ed *through* a link an
///   earlier entry of the same archive planted.
/// - A symlink's *target* is checked too, in both shapes, because the two crates do not agree on
///   whether they check it at all.
/// - The unix mode the archive carries is applied where it carries one. The `cmd` gets its
///   executable bit from [`make_executable`] afterwards regardless (hazard H-5).
/// - `written` is bumped by the bytes of the entry, so the progress poller has a number to read
///   while this thread is inside it.
///
/// # Errors
///
/// [`InstallError::Cancelled`] when the token is tripped, [`InstallError::Archive`] for an entry
/// this installer refuses or an archive that will not read, [`InstallError::Io`] for a write that
/// fails.
pub fn unpack(
    archive: &Path,
    into: &Path,
    format: ArchiveFormat,
    cancel: &CancellationToken,
    written: &AtomicU64,
) -> Result<(), InstallError> {
    std::fs::create_dir_all(into).map_err(|error| io("create", into, &error))?;
    let file = std::fs::File::open(archive).map_err(|error| io("open", archive, &error))?;
    let file = std::io::BufReader::new(file);
    match format {
        ArchiveFormat::Zip => unpack_zip(file, into, cancel, written),
        ArchiveFormat::TarGz => unpack_tar_gz(file, into, cancel, written),
    }
}

/// The `.zip` arm.
fn unpack_zip(
    file: std::io::BufReader<std::fs::File>,
    into: &Path,
    cancel: &CancellationToken,
    written: &AtomicU64,
) -> Result<(), InstallError> {
    let mut archive = zip::ZipArchive::new(file).map_err(|error| refuse(&error.to_string()))?;
    for index in 0..archive.len() {
        if cancel.is_cancelled() {
            return Err(InstallError::Cancelled);
        }
        let mut entry = archive
            .by_index(index)
            .map_err(|error| refuse(&error.to_string()))?;
        let name = entry.name().to_owned();
        let target = safe_target(into, &name)?;

        if entry.is_dir() {
            descend(into, &target, &name, Last::Create)?;
            continue;
        }
        let mode = entry.unix_mode();
        if mode.is_some_and(|mode| mode & S_IFMT == S_IFLNK) {
            not_the_tree_itself(into, &target, &name)?;
            let mut link = String::new();
            entry
                .by_ref()
                .take(MAX_LINK_BYTES + 1)
                .read_to_string(&mut link)
                .map_err(|error| refuse(&format!("`{name}` is an unreadable symlink: {error}")))?;
            if link.len() as u64 > MAX_LINK_BYTES {
                return Err(refuse(&format!(
                    "`{name}` is a symlink whose target is longer than {MAX_LINK_BYTES} bytes; no \
                     filesystem this installer writes to would store it"
                )));
            }
            write_link(into, &target, Path::new(&link), &name)?;
            written.fetch_add(link.len() as u64, Ordering::Relaxed);
            continue;
        }
        not_the_tree_itself(into, &target, &name)?;
        descend(into, &target, &name, Last::Leave)?;
        let mut out = std::fs::File::create(&target).map_err(|e| io("create", &target, &e))?;
        let copied =
            std::io::copy(&mut entry, &mut out).map_err(|error| io("write", &target, &error))?;
        written.fetch_add(copied, Ordering::Relaxed);
        apply_mode(&target, mode).map_err(|error| io("set the mode of", &target, &error))?;
    }
    Ok(())
}

/// The `.tar.gz` arm.
fn unpack_tar_gz(
    file: std::io::BufReader<std::fs::File>,
    into: &Path,
    cancel: &CancellationToken,
    written: &AtomicU64,
) -> Result<(), InstallError> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let entries = archive
        .entries()
        .map_err(|error| refuse(&format!("the tar cannot be read: {error}")))?;
    for entry in entries {
        if cancel.is_cancelled() {
            return Err(InstallError::Cancelled);
        }
        let mut entry =
            entry.map_err(|error| refuse(&format!("a tar entry is damaged: {error}")))?;
        let kind = entry.header().entry_type();
        // `git archive` puts one of these at the head of every tarball it writes. It carries
        // repository metadata and no file, `tar`'s own `unpack` skips it, and refusing it below as
        // "a kind this installer does not write" would reject an ordinary release tarball.
        if kind.is_pax_global_extensions() {
            continue;
        }
        let mode = entry.header().mode().ok();
        let name = entry
            .path()
            .map_err(|error| refuse(&format!("a tar entry has no usable path: {error}")))?
            .to_string_lossy()
            .into_owned();
        let target = safe_target(into, &name)?;

        if kind.is_dir() {
            descend(into, &target, &name, Last::Create)?;
            continue;
        }
        if kind.is_symlink() {
            not_the_tree_itself(into, &target, &name)?;
            let link = entry
                .link_name()
                .map_err(|error| refuse(&format!("`{name}` has no usable target: {error}")))?
                .ok_or_else(|| refuse(&format!("`{name}` is a symlink to nothing")))?;
            write_link(into, &target, &link, &name)?;
            continue;
        }
        if !kind.is_file() {
            return Err(refuse(&format!(
                "`{name}` is an entry of a kind this installer does not write ({kind:?}); an \
                 adapter that needs one is a registry entry to fix, not an archive to trust"
            )));
        }
        not_the_tree_itself(into, &target, &name)?;
        descend(into, &target, &name, Last::Leave)?;
        let mut out = std::fs::File::create(&target).map_err(|e| io("create", &target, &e))?;
        let copied =
            std::io::copy(&mut entry, &mut out).map_err(|error| io("write", &target, &error))?;
        written.fetch_add(copied, Ordering::Relaxed);
        apply_mode(&target, mode).map_err(|error| io("set the mode of", &target, &error))?;
    }
    Ok(())
}

/// `into.join(entry)`, or a refusal (hazard H-16).
///
/// The check is this crate's own, for both shapes, because `zip`'s `enclosed_name` and `tar`'s
/// `unpack_in` are two different rules and an installer cannot afford the case where only one of
/// them covers something. Refused: an absolute path, a drive letter, and any `..` component.
///
/// An entry whose remainder is empty answers `into` itself rather than a refusal, because that is
/// what `./` means and `tar -C dir -czf x.tgz .` — the most common way a tarball is made — writes
/// it as the first entry. As a *directory* it names the tree that already exists and there is
/// nothing to do; as a file or a symlink it is nonsense, which is why the two arms that write one
/// call [`not_the_tree_itself`] and the directory arm does not.
///
/// The split is on **both** separators, like `plan::cmd_relative`'s and for the same reason: a
/// backslash is an ordinary character in a unix path, so an entry named `sub\..\..\etc` would pass
/// a `Path::components` check on Linux and escape when the same tree is read on Windows.
fn safe_target(into: &Path, entry: &str) -> Result<PathBuf, InstallError> {
    let refuse_because = |why: &str| refuse(&format!("`{entry}` {why}"));
    if entry.starts_with('/') || entry.starts_with('\\') {
        return Err(refuse_because("is an absolute path"));
    }
    if entry.chars().nth(1) == Some(':') {
        return Err(refuse_because("names a volume rather than an entry"));
    }
    let mut relative = PathBuf::new();
    for part in entry.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                return Err(refuse_because(
                    "climbs out of the tree it is being written into",
                ));
            }
            name => relative.push(name),
        }
    }
    let target = into.join(relative);
    // Belt and braces: the loop above already makes this true, and it is cheap to be sure of the
    // one property the whole function exists for.
    if !target.starts_with(into) {
        return Err(refuse_because(
            "resolves outside the tree it is being written into",
        ));
    }
    Ok(target)
}

/// Writes one symlink, having checked where it points.
///
/// The target is resolved **lexically** against the link's own directory — never through the
/// filesystem, which at this point holds a half-written tree — and refused if it leaves `into`.
/// An absolute target is refused outright: an archive that ships a link to `/etc/passwd` is not
/// an adapter.
fn write_link(into: &Path, link_at: &Path, target: &Path, entry: &str) -> Result<(), InstallError> {
    if !link_stays_inside(into, link_at, target) {
        return Err(refuse(&format!(
            "`{entry}` is a symlink to `{}`, which leaves the tree",
            target.display()
        )));
    }
    descend(into, link_at, entry, Last::Leave)?;
    create_link(link_at, target).map_err(|error| io("link", link_at, &error))
}

/// Whether `target`, read from a symlink at `link_at`, stays under `into`.
fn link_stays_inside(into: &Path, link_at: &Path, target: &Path) -> bool {
    if target.is_absolute() {
        return false;
    }
    let Some(Ok(base)) = link_at.parent().map(|parent| parent.strip_prefix(into)) else {
        return false;
    };
    let mut depth = base.components().count();
    for part in target.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => match depth.checked_sub(1) {
                Some(less) => depth = less,
                None => return false,
            },
            Component::Normal(_) => depth += 1,
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

/// What `descend` does with the last component of the path it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Last {
    /// A directory entry: create it like every component above it.
    Create,
    /// A file or a symlink: the caller creates it, so this only checks that nothing is standing
    /// there in the shape of a link.
    Leave,
}

/// Opens the way to `target` without ever passing **through** a symlink, creating the directories
/// that are missing as it goes.
///
/// This is the guard [`link_stays_inside`] cannot be. That check is purely lexical — it counts a
/// `Normal` component as one level down — so it cannot know that a `Normal` component is itself a
/// link an earlier entry of the same archive planted. An archive carrying `d/a -> ..`, then
/// `b -> d/a/..`, then a plain file `b/pwned` passes it twice and still writes outside the tree,
/// because `create_dir_all`, `File::create` and `set_permissions` all follow links.
///
/// So every component that already exists is inspected with `symlink_metadata` and a link is
/// refused outright. Two alternatives were on the table and this one is stricter than both:
///
/// - `canonicalize` the parent after a `create_dir_all` and compare it with `canonicalize(into)`
///   — what the `tar` crate does. It refuses the write, but `create_dir_all` has already followed
///   the link by then, so the escape still leaves directories outside the tree behind it.
/// - Defer every symlink entry until the regular ones are written. That closes the same-archive
///   case, but only that one, and it reorders an archive's own entries.
///
/// The cost is that an archive which writes *through* a link it also ships — `lib -> lib64` and
/// then `lib/x` — is refused rather than resolved. No published adapter does that, tar itself
/// stores such a tree as `lib64/x`, and refusing it is the side of the trade an installer belongs
/// on.
fn descend(into: &Path, target: &Path, entry: &str, last: Last) -> Result<(), InstallError> {
    let Ok(relative) = target.strip_prefix(into) else {
        // Unreachable: `safe_target` builds every target as `into.join(..)`.
        return Err(refuse(&format!(
            "`{entry}` resolves outside the tree it is being written into"
        )));
    };
    let parts: Vec<_> = relative.components().collect();
    let mut at = into.to_path_buf();
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else {
            // Unreachable for the same reason: `safe_target` drops `.` and refuses `..`.
            return Err(refuse(&format!(
                "`{entry}` names a path component this installer will not follow"
            )));
        };
        at.push(name);
        let is_last = index + 1 == parts.len();
        match std::fs::symlink_metadata(&at) {
            Ok(found) if found.is_symlink() => {
                return Err(refuse(&format!(
                    "`{entry}` would be written through `{}`, which is a symlink; an archive that \
                     needs one to reach its own files is not an adapter",
                    at.display()
                )));
            }
            // Whatever is already at the far end is the caller's to write over or trip on:
            // `File::create` truncates a file, `symlink` refuses one.
            Ok(_) if is_last && last == Last::Leave => {}
            // A directory that is already there — this entry's own, or an earlier one's.
            Ok(found) if found.is_dir() => {}
            // Anything else standing where a directory has to be. `create_dir_all` used to report
            // this as an `Io` error one component further down; it is the archive that is wrong.
            Ok(_) => {
                return Err(refuse(&format!(
                    "`{entry}` needs `{}` to be a directory and the archive already wrote \
                     something else there",
                    at.display()
                )));
            }
            // Absent. The last component of a file or a symlink is the caller's to create; every
            // other one is a directory this entry needs under it.
            Err(_) if is_last && last == Last::Leave => {}
            Err(_) => std::fs::create_dir(&at).map_err(|error| io("create", &at, &error))?,
        }
    }
    Ok(())
}

/// Refuses an entry that names `into` itself where a file or a symlink is expected.
///
/// `./` is an ordinary first entry of a tarball and means the tree, which the directory arm writes
/// by doing nothing. A *file* by that name would have the unpacker create or chmod the staging
/// tree as though it were a file, so it is refused here instead.
fn not_the_tree_itself(into: &Path, target: &Path, entry: &str) -> Result<(), InstallError> {
    if target == into {
        return Err(refuse(&format!(
            "`{entry}` names the tree itself rather than something in it"
        )));
    }
    Ok(())
}

/// Sets the executable bit on the registry's `cmd`, whatever the archive said (hazard H-5).
///
/// `launch::spawn` resolves through `which` even for an absolute path, and `which` refuses a file
/// nobody may execute — so an adapter whose tarball carries plain `0644` would probe `failed` with
/// "is not executable". This is the README's `chmod +x`, made structural, and the caller runs it
/// **in staging, before the promote**, so a promoted tree is never non-executable for an instant.
///
/// The bit is derived from the read bits rather than set to a flat `0o111`: whoever may read the
/// file may run it, and nobody else gains anything.
///
/// # Errors
///
/// The `io::Error` of the metadata read or the permissions write.
#[cfg(unix)]
pub fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    let mode = permissions.mode();
    permissions.set_mode(mode | ((mode & 0o444) >> 2));
    std::fs::set_permissions(path, permissions)
}

/// Nothing to do: Windows decides what may run from the file's extension and its ACL, neither of
/// which an archive's unix mode says anything about (plan D21).
///
/// # Errors
///
/// Never; the signature matches the unix half so the caller has no `cfg` of its own.
#[cfg(not(unix))]
pub fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Applies the archive's own mode to a written file.
#[cfg(unix)]
fn apply_mode(path: &Path, mode: Option<u32>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let Some(mode) = mode else {
        return Ok(());
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o777))
}

/// A unix mode says nothing a Windows filesystem can store, so it is recorded by being ignored
/// (plan D21).
#[cfg(not(unix))]
fn apply_mode(_path: &Path, _mode: Option<u32>) -> std::io::Result<()> {
    Ok(())
}

/// One symlink, where the platform has them.
#[cfg(unix)]
fn create_link(link_at: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link_at)
}

/// Windows symlinks need a privilege an ordinary user does not have, so the link is written as
/// what it is: a small file naming its target. An adapter that depends on one is broken on
/// Windows either way, and this leaves the evidence for MOD-16 rather than failing the install.
#[cfg(not(unix))]
fn create_link(link_at: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::write(link_at, target.to_string_lossy().as_bytes())
}

/// An archive this installer will not unpack.
fn refuse(message: &str) -> InstallError {
    InstallError::Archive {
        message: message.to_owned(),
    }
}

/// One filesystem failure, named by what was being done and to what.
fn io(what: &str, path: &Path, error: &std::io::Error) -> InstallError {
    InstallError::Io {
        what: format!("{what} {}", path.display()),
        message: error.to_string(),
    }
}
