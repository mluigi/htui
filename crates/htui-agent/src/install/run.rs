//! The pipeline: one call from a consented plan to a probed row (plan MOD-20 D16, T6).
//!
//! Everything the other modules of `install/` do, in the one order that makes the guarantees hold.
//! Two of those orderings are the whole design and neither is negotiable:
//!
//! - **Nothing the glob can resolve exists until the promote.** The download and the unpack land
//!   under `.staging/`, which is a sibling of `<id>/` and outside every seed pattern's walk
//!   (hazard H-3), so every `?` before step 5 leaves a box that resolves exactly what it resolved
//!   before — and this module removes the residue anyway rather than waiting an hour for the next
//!   sweep (hazard H-6).
//! - **The probe decides, and this module never does.** `R-AGT-6`: the status on the outcome is
//!   read off the row [`probe_agent`] returned. A pipeline that fetched, verified, unpacked and
//!   promoted perfectly into a tree that will not handshake ends `failed`, which is the PRD's
//!   "an installer that succeeds into a broken tree" metric made structural.
//!
//! Wait-free by construction: nothing here holds a `std::sync::Mutex` across an `.await`, and the
//! only state shared with another thread is the [`AtomicU64`] the blocking unpack bumps.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime};

use htui_core::model::AgentBox;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

use super::plan::cmd_relative;
use super::{
    Consent, InstallError, InstallJob, InstallOutcome, InstallPhase, InstallProgress,
    InstallRecord, Installer, Layout, Manifest, Throttle, archive, fetch, layout,
};
use crate::launch::AgentLaunch;
use crate::probe::{
    ProbeOutcome, ProbeSnapshot, ProbeStatus, probe_agent, resolve_tool, version_key,
};

/// A consented [`InstallPlan`](super::InstallPlan) made real, in nine steps.
///
/// 0. [`Layout::sweep`] clears what an aborted run left behind (blueprint P-10), and a
///    [`layout::nonce`] names everything this run puts in `.staging/`.
/// 1. [`fetch::download`] streams the archive into `.staging/<id>-<v>-<n>.archive`, hashing it as
///    it goes and reporting `Downloading` frames through the [`Throttle`].
/// 2. `Verifying`: [`fetch::verify_digest`] against the digest the consent pane already showed.
///    A published digest that does not match is [`InstallError::DigestMismatch`] with **nothing**
///    unpacked; none published records the computed one as unverified (plan D2).
/// 3. `Unpacking`: [`archive::unpack`] under `spawn_blocking`, its byte counter polled every
///    `progress_every` while the handle is awaited — the only way to report on work a
///    [`JoinHandle::abort`](tokio::task::JoinHandle::abort) cannot even stop (blueprint P-3). The
///    `cmd` must be in the tree, and it is made executable **in staging** so the promoted tree is
///    never unrunnable for an instant (hazard H-5).
/// 4. [`Layout::set_aside`] takes a same-version tree out of the way of the rename.
/// 5. [`Layout::promote`] — one rename, and the install is committed.
/// 6. The post-promote check (hazard H-4): [`resolve_tool`] over the row's own glob must answer a
///    path that canonicalises to the file that was written. Otherwise
///    [`InstallError::NotWhereTheRowLooks`] naming both, and the rollback of step 8(b).
/// 7. `Probing`: [`probe_agent`]. The status comes off the row it returns and from nowhere else.
/// 8. `ready`/`unauthenticated` → [`Layout::retain_only`], the manifest, and
///    [`InstallOutcome::Installed`]. `missing`/`failed` **with** a previous version → the promoted
///    tree removed, the set-aside one restored, the row probed again so it describes what is left,
///    and [`InstallOutcome::Failed`] with `restored: Some(_)`. **Without** one → the tree stays and
///    `restored` is `None`: "the adapter is fine and a sibling CLI is missing" is `missing` too,
///    and rolling back a finished download for a `PATH` problem would punish the wrong thing.
///    A [`ProbeOutcome::Kept`] counts as "the probe found nothing" and takes the same split.
/// 9. Cancellation is observed by [`fetch::download`]'s `select!`, by [`archive::unpack`] before
///    every entry, and once between each numbered step. Before step 5 it removes the staging
///    entries and answers [`InstallError::Cancelled`]; **after** the promote it is ignored — a
///    promoted tree is committed, and the re-probe that certifies it is seconds, not minutes.
///
/// # Errors
///
/// Every variant of [`InstallError`]. What is **not** an error: a probe that says no. That is an
/// `Ok(InstallOutcome::Failed)`, because it is an answer about this box and the caller has a row
/// to write either way.
pub async fn install(
    installer: &Installer,
    job: InstallJob<'_>,
    progress: &mut (dyn FnMut(InstallProgress) + Send),
    cancel: &CancellationToken,
) -> Result<InstallOutcome, InstallError> {
    let plan = job.plan;
    let id = plan.registry_id.as_str();
    let version = plan.version.as_str();
    let layout = Layout::new(plan.root.clone());
    let staging = Staging::new(&layout, id, version);

    // Steps 0–5. Every way out of this call but `Ok` happened before the rename, so the staging
    // entries are this run's own residue and go with it (hazard H-6). A set-aside that the promote
    // never consumed is put back by `staged` itself, which is closer to the failure than a sweep
    // an hour later.
    let staged = match staged(installer, &job, &layout, &staging, progress, cancel).await {
        Ok(staged) => staged,
        Err(error) => {
            staging.discard().await;
            return Err(error);
        }
    };

    // Step 6. From here `cancel` is ignored: the rename already happened, and a box that has the
    // tree is better served by the truth about it than by a rollback nobody asked for.
    if let Err(error) = row_looks_here(&job, &staged).await {
        roll_back(&layout, &job, &staged).await;
        return Err(error);
    }

    // Step 7. The one place a status is decided, and it is not decided here.
    //
    // Sent without a throttle because there is nothing to throttle: this is the only frame of its
    // phase, and a phase change is admitted whatever the clock says. The throttle that governed
    // the download and the unpack was `staged`'s and went out of scope with it.
    progress(InstallProgress {
        phase: InstallPhase::Probing,
        done: 0,
        total: None,
    });
    let probe = probe_agent(job.agent, job.box_id, job.existing, job.ctx, job.tier2).await;
    let (status, stderr_tail) = verdict(&probe);

    // Step 8.
    if matches!(status, ProbeStatus::Ready | ProbeStatus::Unauthenticated) {
        let row = match probe {
            ProbeOutcome::Row(row) => row,
            // Unreachable by construction: `Kept` is only returned when the probe resolved
            // nothing, and neither `ready` nor `unauthenticated` is reachable without a
            // resolution. Handled rather than asserted because a panic in an install task would
            // take the frame the user is waiting on with it.
            ProbeOutcome::Kept { reason } => {
                return Ok(failed_without_rollback(
                    &staged,
                    version,
                    status,
                    Some(vec![reason.to_owned()]),
                    ProbeOutcome::Kept { reason },
                ));
            }
        };
        return Ok(installed(&layout, &job, &staged, row, status).await);
    }

    if !staged.had_previous {
        // Plan D16(c). The row already describes the tree that is still there, so there is nothing
        // to probe a second time.
        return Ok(failed_without_rollback(
            &staged,
            version,
            status,
            stderr_tail,
            probe,
        ));
    }

    // Plan D16(b): put the box back, then ask it again — the second answer is the one the caller
    // writes, because the first describes a directory that no longer exists.
    roll_back(&layout, &job, &staged).await;
    let restored = newest_version(
        &layout
            .existing_versions(job.plan.registry_id.as_str())
            .await,
    );
    let probe = probe_agent(job.agent, job.box_id, job.existing, job.ctx, job.tier2).await;
    Ok(InstallOutcome::Failed {
        record: staged.record,
        version: job.plan.version.clone(),
        status,
        stderr_tail,
        restored,
        probe,
    })
}

/// The three names one run owns under `.staging/`, composed once from one nonce.
///
/// The nonce travels with the paths rather than being recovered from one of them later: it is what
/// keeps two installs — or an install and the remains of an earlier one — from ever naming the
/// same entry (hazard H-6), and a run that reconstructed it by string-splitting its own directory
/// name would be one rename away from silently sharing another run's.
struct Staging {
    nonce: String,
    archive: PathBuf,
    tree: PathBuf,
}

impl Staging {
    /// The archive, the tree and the nonce that names them both.
    fn new(layout: &Layout, id: &str, version: &str) -> Self {
        let nonce = layout::nonce();
        Self {
            archive: layout.staging_archive(id, version, &nonce),
            tree: layout.staging_tree(id, version, &nonce),
            nonce,
        }
    }

    /// Removes this run's entries, whatever state they reached (hazard H-6).
    ///
    /// Failures are ignored: this runs on a path that already has an error to report, and a
    /// staging entry nobody could delete is what the next install's sweep is for.
    async fn discard(&self) {
        let _ = tokio::fs::remove_file(&self.archive).await;
        let _ = tokio::fs::remove_dir_all(&self.tree).await;
    }
}

/// What steps 0–5 produced: a promoted directory and everything the rest of the pipeline needs to
/// describe it or undo it.
struct Staged {
    /// `<root>/<id>/<version>/`, as it now exists.
    dir: PathBuf,
    /// The `cmd` inside it, relative — what the glob has to resolve and what is made executable.
    cmd: PathBuf,
    /// What the download and the verify decided (plan D2, D17).
    record: InstallRecord,
    /// This version was installed before and hashes differently now.
    digest_changed: bool,
    /// The same-version tree that was moved out of the way, if there was one.
    previous: Option<PathBuf>,
    /// Whether anything was installed under this id **before** the promote — the (b)/(c) split of
    /// plan D16, decided while it can still be decided honestly.
    had_previous: bool,
}

/// Steps 0 to 5: sweep, download, verify, unpack, set aside, promote.
///
/// Every failure it returns happened before the rename, so the caller's cleanup of the staging
/// entries is always correct. The one thing it undoes itself is the set-aside, because a promote
/// that failed after it would otherwise leave the box with no version at all until the next
/// install's sweep noticed.
async fn staged(
    installer: &Installer,
    job: &InstallJob<'_>,
    layout: &Layout,
    staging: &Staging,
    progress: &mut (dyn FnMut(InstallProgress) + Send),
    cancel: &CancellationToken,
) -> Result<Staged, InstallError> {
    let config = installer.config();
    let plan = job.plan;
    let id = plan.registry_id.as_str();
    let version = plan.version.as_str();
    let mut throttle = Throttle::new(config.progress_every);

    // 0. The residue of a run that was aborted rather than cancelled (blueprint P-10). A failure
    // here is not skippable: carrying on would promote over something a sweep was about to move.
    layout
        .sweep(config.staging_max_age, SystemTime::now())
        .await?;
    stop_if_cancelled(cancel)?;

    // 1.
    let downloaded = fetch::download(
        installer,
        plan,
        &staging.archive,
        &mut throttle,
        progress,
        cancel,
    )
    .await?;
    stop_if_cancelled(cancel)?;

    // 2. Between the last byte and the first entry, which is the only place a digest check is
    // worth anything (plan D2).
    emit(
        &mut throttle,
        progress,
        InstallProgress {
            phase: InstallPhase::Verifying,
            done: 0,
            total: None,
        },
    );
    let published = fetch::verify_digest(plan.sha256.as_deref(), &downloaded.sha256)?;
    let digest_changed = plan
        .recorded
        .as_ref()
        .is_some_and(|record| record.sha256 != downloaded.sha256);
    let record = InstallRecord {
        sha256: downloaded.sha256.clone(),
        published,
        archive: plan.archive_url.clone(),
        platform: plan.platform.clone(),
        installed_at: job.ctx.now,
    };
    stop_if_cancelled(cancel)?;

    // 3. On a blocking thread, because a zip's directory is at its end and the whole file has to
    // be read; polled from here, because that thread cannot be awaited on and cannot be aborted.
    let cmd = cmd_relative(&plan.cmd).map_err(|error| InstallError::Archive {
        message: error.to_string(),
    })?;
    unpack_polling(
        installer,
        &downloaded.path,
        &staging.tree,
        &cmd,
        plan.format,
        progress,
        &mut throttle,
        cancel,
    )
    .await?;
    // The archive has done its work and is the largest thing in staging; keeping it until the
    // sweep would mean the peak disk of an install outliving the install by an hour.
    if let Err(error) = tokio::fs::remove_file(&downloaded.path).await {
        warn!(
            archive = %downloaded.path.display(),
            %error,
            "the downloaded archive could not be removed; the next sweep will collect it"
        );
    }
    stop_if_cancelled(cancel)?;

    // 4. Only ever the *same* version: a different one is a sibling the retention deals with once
    // the probe has spoken, not something to move now.
    let previous = layout.set_aside(id, version, &staging.nonce).await?;
    // Decided here, while `<root>/<id>/<version>/` is guaranteed absent: everything still listed
    // is a version this install did not write, which is exactly what plan D16(b) means by "a
    // previous version existed".
    let had_previous = previous.is_some() || !layout.existing_versions(id).await.is_empty();
    if let Err(error) = stop_if_cancelled(cancel) {
        restore(layout, id, version, previous.as_deref()).await;
        return Err(error);
    }

    // 5.
    let dir = match layout.promote(&staging.tree, id, version).await {
        Ok(dir) => dir,
        Err(error) => {
            restore(layout, id, version, previous.as_deref()).await;
            return Err(error);
        }
    };
    Ok(Staged {
        dir,
        cmd,
        record,
        digest_changed,
        previous,
        had_previous,
    })
}

/// Step 3: the unpack, with the byte counter read every `progress_every` while it runs.
///
/// `timeout(progress_every, &mut handle)` in a loop rather than a second task and a channel: the
/// counter is one `AtomicU64` and the poller is this future, so there is no place for a lock to be
/// held across an `.await` and nothing to shut down afterwards.
#[expect(
    clippy::too_many_arguments,
    reason = "every argument is a distinct injected seam — the client's config, the two paths, the \
              format, the sink, the throttle and the token — and bundling them into a struct \
              would name the same six values twice for one call site"
)]
async fn unpack_polling(
    installer: &Installer,
    from: &Path,
    into: &Path,
    cmd: &Path,
    format: super::ArchiveFormat,
    progress: &mut (dyn FnMut(InstallProgress) + Send),
    throttle: &mut Throttle,
    cancel: &CancellationToken,
) -> Result<(), InstallError> {
    let written = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&written);
    let (archive_path, tree, cmd_at) = (from.to_path_buf(), into.to_path_buf(), into.join(cmd));
    let token = cancel.clone();
    let mut handle = tokio::task::spawn_blocking(move || {
        archive::unpack(&archive_path, &tree, format, &token, &counter)?;
        if !cmd_at.exists() {
            return Err(InstallError::Archive {
                message: format!(
                    "the archive does not carry `{}`, which this registry entry names as its \
                     command; nothing under it could ever be launched",
                    cmd_at
                        .strip_prefix(&tree)
                        .unwrap_or(&cmd_at)
                        .to_string_lossy()
                ),
            });
        }
        // In staging and before the promote (hazard H-5): `launch::spawn` goes through `which`
        // even for an absolute path, and `which` refuses a file nobody may execute.
        archive::make_executable(&cmd_at).map_err(|error| InstallError::Io {
            what: format!("make {} executable", cmd_at.display()),
            message: error.to_string(),
        })
    });

    let joined = loop {
        match tokio::time::timeout(installer.config().progress_every, &mut handle).await {
            Ok(joined) => break joined,
            Err(_elapsed) => emit(
                throttle,
                progress,
                InstallProgress {
                    phase: InstallPhase::Unpacking,
                    done: written.load(Ordering::Relaxed),
                    total: None,
                },
            ),
        }
    };
    let done = written.load(Ordering::Relaxed);
    match joined {
        Ok(result) => {
            result?;
            // `total == done` is what makes the throttle let this one through whatever the clock
            // says, so the cell never ends the phase mid-count.
            emit(
                throttle,
                progress,
                InstallProgress {
                    phase: InstallPhase::Unpacking,
                    done,
                    total: Some(done),
                },
            );
            Ok(())
        }
        Err(error) => Err(InstallError::Io {
            what: format!("unpack {}", from.display()),
            message: error.to_string(),
        }),
    }
}

/// Step 6: does the row's own glob resolve what was just written (plan D5, hazard H-4)?
///
/// [`resolve_tool`] and not [`probe_tools`](crate::probe::probe_tools), deliberately: the question
/// is whether the **glob** sees the file, not what the box would run. Going through the override
/// tier would let a `HTUI_TOOL_<NAME>` the user happens to have set answer for a seed pattern that
/// points at the wrong directory, and the drift this check exists to catch would ship (hazard
/// H-11).
///
/// Both sides go through `canonicalize` because on Windows one of them arrives with a `\\?\`
/// prefix and the other does not (plan D21), and a string comparison would refuse every correct
/// install on that platform.
async fn row_looks_here(job: &InstallJob<'_>, staged: &Staged) -> Result<(), InstallError> {
    let promoted = staged.dir.join(&staged.cmd);
    let resolved = match tool_probe(job) {
        Some(probe) => match resolve_tool(&probe, &job.ctx.env).await {
            Ok(found) => found.map(|found| found.path),
            // A resolver that could not run has not shown that the glob misses the file, but it
            // has not shown that it finds it either, and this check only ever answers yes on
            // proof.
            Err(error) => {
                warn!(%error, "the post-promote check could not run the row's own resolver");
                None
            }
        },
        None => None,
    };
    if same_file(resolved.as_deref(), &promoted).await {
        return Ok(());
    }
    Err(InstallError::NotWhereTheRowLooks { promoted, resolved })
}

/// The row's probe for the tool its `install` block names, or `None` when the document no longer
/// declares one — which the pre-flight refuses as `NoGlobTool`, and which a hand-edited row could
/// still produce between the consent and the confirm.
fn tool_probe(job: &InstallJob<'_>) -> Option<crate::launch::ToolProbe> {
    let launch: AgentLaunch = serde_json::from_value(job.agent.launch.clone()).ok()?;
    launch.discovery?.tools.get(&job.plan.tool).cloned()
}

/// Whether two paths name the same file once both are canonicalised. An unreadable path is not the
/// same file as anything.
async fn same_file(left: Option<&Path>, right: &Path) -> bool {
    let Some(left) = left else {
        return false;
    };
    let (Ok(left), Ok(right)) = (
        tokio::fs::canonicalize(left).await,
        tokio::fs::canonicalize(right).await,
    ) else {
        return false;
    };
    left == right
}

/// Step 8(a): the retention and the manifest, in that order and only now.
///
/// Neither failure undoes the install. A version directory that will not delete is disk to
/// reclaim, and a manifest that will not write costs one repeated consent question — measured
/// against telling a user that a tree their box is happily running failed to install, both are the
/// cheap side.
async fn installed(
    layout: &Layout,
    job: &InstallJob<'_>,
    staged: &Staged,
    row: AgentBox,
    status: ProbeStatus,
) -> InstallOutcome {
    let plan = job.plan;
    let id = plan.registry_id.as_str();
    let removed_versions = match layout.retain_only(id, &plan.version).await {
        Ok(removed) => removed,
        Err(error) => {
            warn!(%error, id, "a replaced version could not be removed; it is disk, not an install");
            Vec::new()
        }
    };
    // `retain_only` walks `<root>/<id>/` and the set-aside copy is not there — it is under
    // `.staging/`, which is the whole point of the suffix. Removing it here rather than leaving it
    // to the sweep is not only about the duplicate gigabytes: a `.previous` outlives the retention
    // that deleted `<id>/<version>/`, and the next sweep, finding its target absent, would
    // *restore* it at any age (`Layout::sweep`'s first rule) and silently undo plan D16(a).
    if let Some(previous) = &staged.previous
        && let Err(error) = tokio::fs::remove_dir_all(previous).await
    {
        warn!(
            previous = %previous.display(),
            %error,
            "the set-aside copy of this version could not be removed; it is disk, not an install"
        );
    }

    let path = layout.manifest(id);
    let mut file = Manifest::load(&path).await;
    file.installs
        .insert(plan.version.clone(), staged.record.clone());
    if plan.consent.is_none() {
        // `y` accepted the licence and the install in one keystroke (plan D13); this is the record
        // that gives that keystroke its meaning (plan D17).
        file.consent = Some(Consent {
            license: plan.license.clone(),
            license_url: plan.license_url.clone(),
            accepted_at: job.ctx.now,
            version: plan.version.clone(),
        });
    }
    if let Err(error) = file.store(&path).await {
        warn!(
            manifest = %path.display(),
            %error,
            "the install manifest could not be written; consent will be asked again"
        );
    }

    InstallOutcome::Installed {
        record: staged.record.clone(),
        version: plan.version.clone(),
        dir: staged.dir.clone(),
        row,
        status,
        removed_versions,
        digest_changed: staged.digest_changed,
    }
}

/// Plan D16(c): the promoted tree stays and the probe's own row describes it.
fn failed_without_rollback(
    staged: &Staged,
    version: &str,
    status: ProbeStatus,
    stderr_tail: Option<Vec<String>>,
    probe: ProbeOutcome,
) -> InstallOutcome {
    InstallOutcome::Failed {
        record: staged.record.clone(),
        version: version.to_owned(),
        status,
        stderr_tail,
        restored: None,
        probe,
    }
}

/// The rollback of plan D16(b), also used by the post-promote check: the promoted tree goes, and
/// the tree it displaced comes back.
///
/// Errors are logged and not returned. This runs on a path that is already reporting a failure,
/// and replacing the probe's own explanation with a filesystem message would hide the thing the
/// user needs to read; what is left behind is `.staging/` residue the next sweep restores anyway
/// (hazard H-7).
///
/// The removal is retried on [`promote`](Layout::promote)'s own backoff, because it is the same
/// refusal seen from the other side: on Windows a `.exe` the post-promote check has just probed is
/// exactly what `remove_dir_all` comes back `PermissionDenied` on (plan D21). Getting this wrong
/// is worse than a leaked directory — a `.previous` whose version directory is *occupied* is not
/// restored by the next sweep, it is **deleted** by it once it is an hour old, and this function's
/// caller has already told the user that version is what the box was left with. So the final
/// failure is an `error!` that names the directory to remove and the copy to rename back.
async fn roll_back(layout: &Layout, job: &InstallJob<'_>, staged: &Staged) {
    let id = job.plan.registry_id.as_str();
    let version = job.plan.version.as_str();
    if let Err(error) = remove_promoted(layout, id, version).await {
        error!(
            %error,
            dir = %staged.dir.display(),
            previous = ?staged.previous,
            "the promoted tree could not be removed, so the version set aside for it cannot be put \
             back; remove that directory by hand and rename the `.previous` copy into its place \
             before the next sweep collects it"
        );
        return;
    }
    restore(layout, id, version, staged.previous.as_deref()).await;
}

/// [`Layout::remove_version`] on [`promote`](Layout::promote)'s attempts and backoff.
async fn remove_promoted(layout: &Layout, id: &str, version: &str) -> Result<(), InstallError> {
    let mut attempt = 1;
    let mut backoff = layout::PROMOTE_BACKOFF;
    loop {
        match layout.remove_version(id, version).await {
            Ok(()) => return Ok(()),
            Err(error) if attempt < layout::PROMOTE_ATTEMPTS => {
                warn!(
                    attempt,
                    %error,
                    "the promoted tree would not be removed; something may still be holding it"
                );
                tokio::time::sleep(backoff).await;
                backoff *= 2;
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Puts a set-aside version back, if there was one.
async fn restore(layout: &Layout, id: &str, version: &str, previous: Option<&Path>) {
    let Some(previous) = previous else {
        return;
    };
    if let Err(error) = layout.restore(previous, id, version).await {
        warn!(
            %error,
            previous = %previous.display(),
            "the previous version could not be put back; the next install's sweep will restore it"
        );
    }
}

/// The status and the failure text of one probe answer.
///
/// A [`ProbeOutcome::Kept`] has no row and therefore no snapshot, and plan D16 reads it as "the
/// probe found nothing" — `missing`, which is what sends it down the (b)/(c) split. A row whose
/// snapshot will not parse is read the same way: a column nobody can read is not a certificate.
fn verdict(probe: &ProbeOutcome) -> (ProbeStatus, Option<Vec<String>>) {
    match probe {
        ProbeOutcome::Row(row) => match ProbeSnapshot::from_row(row) {
            Some(snapshot) => (snapshot.status, snapshot.stderr_tail),
            None => (ProbeStatus::Missing, None),
        },
        ProbeOutcome::Kept { reason } => (ProbeStatus::Missing, Some(vec![(*reason).to_owned()])),
    }
}

/// The version a box is left resolving, by plan D14's own rule: a parsable version beats an
/// unparsable one, and among parsable ones the highest wins.
///
/// Keyed the same way `probe::newest` keys a glob capture, so what this reports and what the row's
/// glob will actually resolve after the rollback are the same directory rather than two guesses.
fn newest_version(versions: &[String]) -> Option<String> {
    versions
        .iter()
        .max_by_key(|name| (version_key(name), (*name).clone()))
        .cloned()
}

/// Step 9's check between two numbered steps.
fn stop_if_cancelled(cancel: &CancellationToken) -> Result<(), InstallError> {
    if cancel.is_cancelled() {
        return Err(InstallError::Cancelled);
    }
    Ok(())
}

/// One frame through the throttle.
fn emit(
    throttle: &mut Throttle,
    progress: &mut dyn FnMut(InstallProgress),
    frame: InstallProgress,
) {
    if throttle.admit(frame, Instant::now()) {
        progress(frame);
    }
}
