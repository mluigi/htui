//! MOD-132 plan P7: the header of a git bundle, parsed here rather than trusted to `git`.
//!
//! A bundle is a header of text lines, an empty line, then a pack. The header names the refs the
//! bundle carries and the commits it needs (its prerequisites); collect reads both before any
//! byte of the pack reaches `git index-pack`, and only `pack_offset..` is ever handed to it.
//!
//! Accepted: `# v2 git bundle` and `# v3 git bundle`; in v3 only the `@object-format=sha1`
//! capability. `sha256` is refused by name, because the workspace builds `gix` with its `sha1`
//! feature alone (root `Cargo.toml`), so no repository this box opens is sha256; `@filter=` (a
//! partial bundle) is refused by name, and so is every other capability. Every hash is 40
//! lowercase hex digits and every ref name passes `gix::validate::reference::name`. The header,
//! its empty line included, is at most [`HEADER_CAP`] bytes.

use std::io::Read as _;
use std::path::{Path, PathBuf};

/// The longest header [`parse_header`] reads, its terminating empty line included: 1 MiB.
pub const HEADER_CAP: usize = 1024 * 1024;

/// One bundle's header, validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleHeader {
    /// `2` or `3`, from the signature line.
    pub version: u8,
    /// The commits the receiving repository must already hold, as lowercase hex, in file order;
    /// a prerequisite's comment is dropped.
    pub prerequisites: Vec<String>,
    /// `(hash, full ref name)` for every ref the bundle carries, in file order.
    pub refs: Vec<(String, String)>,
    /// The byte after the header's empty line: where the pack starts.
    pub pack_offset: u64,
}

/// Why a header was refused: one variant per class, each with its own sentence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BundleRefusal {
    /// The first line is not a `# v<n> git bundle` signature.
    #[error("{}", not_a_bundle())]
    NotABundle,
    /// The signature names a version other than 2 or 3; the field is the `v<n>` it named.
    #[error("{}", unknown_version(.0))]
    UnknownVersion(String),
    /// A v3 capability other than `object-format` or `filter`; the field is the line after `@`.
    #[error("{}", unknown_capability(.0))]
    Capability(String),
    /// A v3 `@filter=` capability: a partial bundle. The field is the filter.
    #[error("{}", partial_bundle(.0))]
    Filter(String),
    /// A line that is not a capability, a prerequisite or a ref line where it stands, holds a
    /// carriage return, or is not UTF-8. The field is the 1-based line number.
    #[error("{}", malformed_line(*.0))]
    MalformedLine(usize),
    /// A hash that is not 40 lowercase hex digits.
    #[error("{}", bad_hash(.0))]
    BadHash(String),
    /// A ref name `gix::validate::reference::name` refuses.
    #[error("{}", bad_ref_name(.0))]
    BadRefName(String),
    /// No empty line within the first [`HEADER_CAP`] bytes.
    #[error("{}", header_too_long())]
    HeaderTooLong,
    /// The bytes ended before the header's empty line.
    #[error("{}", no_end_of_header())]
    NoEndOfHeader,
    /// A v3 `@object-format=sha256` bundle.
    #[error("{}", sha256_bundle())]
    Sha256,
    /// [`read_header`] could not read the file.
    #[error("{}", unreadable(.path, .message))]
    Unreadable {
        /// The bundle file.
        path: PathBuf,
        /// The I/O error, as text.
        message: String,
    },
}

// One named function per sentence (plan P11), so a rewording is a one-line change here.

/// [`BundleRefusal::NotABundle`].
#[must_use]
pub fn not_a_bundle() -> String {
    "not a git bundle: the first line is not `# v2 git bundle` or `# v3 git bundle`".to_owned()
}

/// [`BundleRefusal::UnknownVersion`].
#[must_use]
pub fn unknown_version(version: &str) -> String {
    format!("git bundle version {version} is not supported: only v2 and v3 are")
}

/// [`BundleRefusal::Capability`].
#[must_use]
pub fn unknown_capability(capability: &str) -> String {
    format!("git bundle capability @{capability} is not supported")
}

/// [`BundleRefusal::Filter`].
#[must_use]
pub fn partial_bundle(filter: &str) -> String {
    format!("a partial git bundle (@filter={filter}) is not accepted")
}

/// [`BundleRefusal::MalformedLine`].
#[must_use]
pub fn malformed_line(line: usize) -> String {
    format!("git bundle header line {line} is malformed")
}

/// [`BundleRefusal::BadHash`].
#[must_use]
pub fn bad_hash(hash: &str) -> String {
    format!("git bundle header names {hash}, which is not a 40-digit lowercase sha1 hash")
}

/// [`BundleRefusal::BadRefName`].
#[must_use]
pub fn bad_ref_name(name: &str) -> String {
    format!("git bundle header names the ref {name}, which is not a valid ref name")
}

/// [`BundleRefusal::HeaderTooLong`].
#[must_use]
pub fn header_too_long() -> String {
    format!("git bundle header is longer than {} KiB", HEADER_CAP / 1024)
}

/// [`BundleRefusal::NoEndOfHeader`].
#[must_use]
pub fn no_end_of_header() -> String {
    "git bundle ends before the end of its header".to_owned()
}

/// [`BundleRefusal::Sha256`].
#[must_use]
pub fn sha256_bundle() -> String {
    "a sha256 git bundle is not accepted: every repository on this box is sha1".to_owned()
}

/// [`BundleRefusal::Unreadable`].
#[must_use]
pub fn unreadable(path: &Path, message: &str) -> String {
    format!("cannot read the git bundle {}: {message}", path.display())
}

/// Parses the header at the start of `bytes`; anything past its empty line is not looked at.
///
/// # Errors
/// The [`BundleRefusal`] for the first thing wrong with it, in file order.
pub fn parse_header(bytes: &[u8]) -> Result<BundleHeader, BundleRefusal> {
    let window = &bytes[..bytes.len().min(HEADER_CAP)];
    let ran_out = || {
        if bytes.len() >= HEADER_CAP {
            BundleRefusal::HeaderTooLong
        } else {
            BundleRefusal::NoEndOfHeader
        }
    };

    let Some(end) = window.iter().position(|&byte| byte == b'\n') else {
        return Err(if window.starts_with(b"# v") {
            ran_out()
        } else {
            BundleRefusal::NotABundle
        });
    };
    let version = signature(&window[..end])?;
    let mut header = BundleHeader {
        version,
        prerequisites: Vec::new(),
        refs: Vec::new(),
        pack_offset: 0,
    };

    let mut start = end + 1;
    let mut number = 1;
    let mut capabilities_open = version == 3;
    loop {
        number += 1;
        let Some(len) = window[start..].iter().position(|&byte| byte == b'\n') else {
            return Err(ran_out());
        };
        let raw = &window[start..start + len];
        start += len + 1;
        if raw.is_empty() {
            break;
        }
        let line = std::str::from_utf8(raw)
            .ok()
            .filter(|line| !line.contains('\r'))
            .ok_or(BundleRefusal::MalformedLine(number))?;

        if let Some(capability) = line.strip_prefix('@') {
            if !capabilities_open {
                return Err(BundleRefusal::MalformedLine(number));
            }
            check_capability(capability)?;
            continue;
        }
        capabilities_open = false;

        if let Some(rest) = line.strip_prefix('-') {
            let hash = rest.split_once(' ').map_or(rest, |(hash, _comment)| hash);
            header.prerequisites.push(check_hash(hash)?);
        } else {
            let (hash, name) = line
                .split_once(' ')
                .ok_or(BundleRefusal::MalformedLine(number))?;
            let hash = check_hash(hash)?;
            gix::validate::reference::name(name.as_bytes().into())
                .map_err(|_| BundleRefusal::BadRefName(name.to_owned()))?;
            header.refs.push((hash, name.to_owned()));
        }
    }
    header.pack_offset = u64::try_from(start).map_err(|_| BundleRefusal::HeaderTooLong)?;
    Ok(header)
}

/// Reads at most [`HEADER_CAP`] bytes of the bundle at `path` and parses its header.
///
/// # Errors
/// [`BundleRefusal::Unreadable`] when the file cannot be opened or read; otherwise
/// [`parse_header`]'s.
pub fn read_header(path: &Path) -> Result<BundleHeader, BundleRefusal> {
    let unreadable = |err: std::io::Error| BundleRefusal::Unreadable {
        path: path.to_path_buf(),
        message: err.to_string(),
    };
    let file = std::fs::File::open(path).map_err(unreadable)?;
    let mut bytes = Vec::new();
    file.take(HEADER_CAP as u64)
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    parse_header(&bytes)
}

/// The version a signature line names.
fn signature(line: &[u8]) -> Result<u8, BundleRefusal> {
    match line {
        b"# v2 git bundle" => Ok(2),
        b"# v3 git bundle" => Ok(3),
        _ if line.contains(&b'\r') => Err(BundleRefusal::MalformedLine(1)),
        _ => {
            let version = line
                .strip_prefix(b"# ")
                .and_then(|rest| rest.strip_suffix(b" git bundle"))
                .filter(|version| version.starts_with(b"v"))
                .ok_or(BundleRefusal::NotABundle)?;
            Err(BundleRefusal::UnknownVersion(
                String::from_utf8_lossy(version).into_owned(),
            ))
        }
    }
}

/// One v3 capability, the text after its `@`.
fn check_capability(capability: &str) -> Result<(), BundleRefusal> {
    match capability.split_once('=') {
        Some(("object-format", "sha1")) => Ok(()),
        Some(("object-format", "sha256")) => Err(BundleRefusal::Sha256),
        Some(("filter", filter)) => Err(BundleRefusal::Filter(filter.to_owned())),
        _ => Err(BundleRefusal::Capability(capability.to_owned())),
    }
}

/// Whether `text` is a full sha1 hash as P7 accepts one: 40 lowercase hex digits.
#[must_use]
pub fn is_full_hash(text: &str) -> bool {
    text.len() == 40
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `hash` when [`is_full_hash`] accepts it.
fn check_hash(hash: &str) -> Result<String, BundleRefusal> {
    if is_full_hash(hash) {
        Ok(hash.to_owned())
    } else {
        Err(BundleRefusal::BadHash(hash.to_owned()))
    }
}
