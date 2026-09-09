//! The archive body: streamed to disk, hashed as it goes, cancellable between any two chunks
//! (plan MOD-20 D2, D9, D16).
//!
//! This is the first byte of an adapter anything in `htui` requests, and it is requested only
//! after the user has said `y` to a plan that named its size and its digest (plan D13). What
//! arrives goes straight into `.staging/`, where no glob walks (hazard H-3), and the digest is
//! computed **as the bytes pass** rather than by reading the file back — a second read of 682 MB
//! to learn something the first read already knew.

use std::path::{Path, PathBuf};
use std::time::Instant;

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use super::{InstallError, InstallPhase, InstallPlan, InstallProgress, Installer, Throttle};

/// What one completed download left on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    /// The staging file the body was written to.
    pub path: PathBuf,
    /// Lowercase hex sha256 of everything that was written — the spelling
    /// `htui_store::identity`'s digest uses, and the one the manifest records.
    pub sha256: String,
    /// How many bytes that was.
    pub bytes: u64,
}

/// Streams `plan.archive_url` into `into`, hashing every chunk.
///
/// Progress is a [`InstallProgress`] per admitted frame, `total` being the `content-length` the
/// pre-flight already showed the user, so the cell and the consent pane cannot disagree about the
/// denominator. The [`Throttle`] decides what reaches the sink (plan D18: at most one frame per
/// 250 ms), and the final frame is sent whatever it decides — a cell left reading `97%` because
/// the last frame fell inside the window is a bug the user reports.
///
/// Cancellation is checked before every chunk **and** raced against the read, so `x` on a stalled
/// body takes effect at once rather than at the next byte. Every way out but success removes the
/// partial file (hazard H-6): a 400 MB `.archive` waiting an hour for the sweep is exactly what
/// pressing `x` is meant to avoid.
///
/// # Errors
///
/// [`InstallError::Cancelled`] when the token is tripped, [`InstallError::Network`] — carrying
/// plan D20's derived manual steps — when the transport fails or a chunk does not arrive inside
/// the read timeout, [`InstallError::Io`] when the file cannot be written.
pub async fn download(
    installer: &Installer,
    plan: &InstallPlan,
    into: &Path,
    throttle: &mut Throttle,
    progress: &mut (dyn FnMut(InstallProgress) + Send),
    cancel: &CancellationToken,
) -> Result<Downloaded, InstallError> {
    let config = installer.config();
    if let Some(parent) = into.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| io("create", parent, &error))?;
    }
    let (info, mut body) = installer
        .http()
        .stream(&plan.archive_url)
        .await
        .map_err(|error| network(plan, &config.registry_base, error.message))?;

    // The pre-flight's number first: it is what the user consented to. The body's own header is
    // the fallback for the case where the `HEAD` was refused and the `GET` was not.
    let total = plan.content_length.or(info.content_length);
    let frame = |done: u64| InstallProgress {
        phase: InstallPhase::Downloading,
        done,
        total,
    };

    let mut file = tokio::fs::File::create(into)
        .await
        .map_err(|error| io("create", into, &error))?;
    let mut hasher = Sha256::new();
    let mut done = 0_u64;
    let mut sent = None;
    if throttle.admit(frame(0), Instant::now()) {
        progress(frame(0));
        sent = Some(0);
    }

    // Every way out of the loop is a value and not a `return`, so that the handle below can be
    // dropped **before** `abandon` unlinks the file it belongs to: on Windows `remove_file` fails
    // for as long as any handle to the file is open, and the partial download hazard H-6 is about
    // would survive on the one platform that cannot afford it.
    let failure = loop {
        if cancel.is_cancelled() {
            break Some(InstallError::Cancelled);
        }
        let next = tokio::select! {
            biased;
            () = cancel.cancelled() => break Some(InstallError::Cancelled),
            chunk = body.chunk() => chunk,
        };
        let chunk = match next {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break None,
            Err(error) => break Some(network(plan, &config.registry_base, error.message)),
        };
        hasher.update(&chunk);
        if let Err(error) = file.write_all(&chunk).await {
            break Some(io("write", into, &error));
        }
        done += chunk.len() as u64;
        if throttle.admit(frame(done), Instant::now()) {
            progress(frame(done));
            sent = Some(done);
        }
    };
    let failure = match failure {
        Some(error) => Some(error),
        None => file
            .sync_all()
            .await
            .err()
            .map(|error| io("flush", into, &error)),
    };
    drop(file);
    if let Some(error) = failure {
        return abandon(into, error).await;
    }
    if sent != Some(done) {
        progress(frame(done));
    }

    Ok(Downloaded {
        path: into.to_path_buf(),
        sha256: hex(&hasher.finalize()),
        bytes: done,
    })
}

/// Plan D2's verification, as one decision: whether the digest that will be recorded was
/// *published* or merely *computed*.
///
/// `None` published is `Ok(false)` and not an error — eight of the registry's forty entries
/// publish nothing, and refusing them would refuse the adapter this item exists to install. What
/// `htui` owes those entries is honesty, which is the `published` flag the manifest keeps and the
/// sentence the consent pane already showed.
///
/// The comparison is case-insensitive on trimmed text: a registry that writes its hex in capitals
/// has not published a different digest.
///
/// # Errors
///
/// [`InstallError::DigestMismatch`], naming both digests. The caller has not unpacked anything at
/// this point and must not: verifying after unpacking would mean deleting an attacker's files
/// rather than never writing them.
pub fn verify_digest(published: Option<&str>, computed: &str) -> Result<bool, InstallError> {
    match published {
        None => Ok(false),
        Some(expected) if expected.trim().eq_ignore_ascii_case(computed) => Ok(true),
        Some(expected) => Err(InstallError::DigestMismatch {
            expected: expected.to_owned(),
            computed: computed.to_owned(),
        }),
    }
}

/// Removes the partial file and answers the error that made it partial.
///
/// Every caller drops its write handle first — see the loop in [`download`].
async fn abandon<T>(path: &Path, error: InstallError) -> Result<T, InstallError> {
    let _ = tokio::fs::remove_file(path).await;
    Err(error)
}

/// A transport failure, with plan D20's steps for doing it by hand attached.
fn network(plan: &InstallPlan, registry_base: &str, message: String) -> InstallError {
    InstallError::Network {
        message,
        manual: Box::new(plan.manual_steps(registry_base)),
    }
}

/// Lowercase hex, the spelling the manifest and `htui_store::identity` both use.
fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// One filesystem failure, named by what was being done and to what.
fn io(what: &str, path: &Path, error: &std::io::Error) -> InstallError {
    InstallError::Io {
        what: format!("{what} {}", path.display()),
        message: error.to_string(),
    }
}
