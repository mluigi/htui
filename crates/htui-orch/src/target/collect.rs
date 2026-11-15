//! MOD-132 plan P6, P8, P9: collect. A target clone's run branch comes back to this box as a
//! bundle, and a local branch moves to its tip, in one stateless call.
//!
//! On the target, through the [`TargetChannel`]: `rev-parse` of the run branch (its tip, 40
//! lowercase hex), then `git bundle create - refs/heads/<run> ^<base>` into a quarantine file on
//! this box (`create_new`, `0600`, at most [`CollectRequest::cap`] bytes). A tip at the base is
//! [`CollectOutcome::Nothing`] and makes no bundle.
//!
//! On this box, before any ref is written: the header ([`read_header`]) carries exactly one ref,
//! the run branch at the tip just read; every prerequisite is a commit this box holds; `git bundle
//! verify` passes; `git index-pack --strict --fix-thin` takes the pack (the fsck `git fetch`
//! skips on a bundle, probe B5); and `rev-list --objects` finds the tip connected. Then the temp
//! ref ([`temp_ref`]) holds the tip through the decision.
//!
//! The decision: the local branch must not be checked out anywhere; it is created, moved forward,
//! or (rewritten) forced. A rewrite, or a changed path that runs on this box
//! ([`HOST_EXECUTING_PATHS`], plus the caller's), needs a [`Consent`] naming this tip and this
//! path list, else the outcome is [`CollectOutcome::NeedsConsent`] and nothing moves. The move is
//! a compare-and-swap; a branch moved under it is [`CollectRefusal::LocalMoved`].
//!
//! The temp ref and the quarantine file are removed on every exit. Waiting, retrying and asking
//! the person are M2's (PRD D5): every fact a waiting row needs is in the outcome.

use std::path::{Path, PathBuf};
use std::time::Duration;

use htui_agent::target_channel::{
    ChannelError, ExecOutput, ExecSpec, StdinFrom, StdoutTo, TargetChannel,
};
use htui_core::model::{RepoId, RunId};

use crate::isolate::IsolateError;
use crate::isolate::git::{self, Cli, TRANSFER_TIMEOUT, VERB_TIMEOUT, blocking, with_retry};
use crate::target::bundle::{BundleRefusal, is_full_hash, read_header};
use crate::target::seed::{bad_run_branch, bad_target_dir, target_git_failed};

/// The default most bytes a collected bundle may hold: 2 GiB.
pub const COLLECT_CAP: u64 = 2 << 30;

/// Where every collect temp ref lives (plan P8); [`collect`] refuses a temp ref outside it.
pub const TEMP_REF_PREFIX: &str = "refs/htui-collect/";

/// The entry [`CollectOutcome::NeedsConsent`] lists when the changed paths overflowed the diff
/// cap: the paths past it are unknown, so consent is asked as if one of them ran on this box.
pub const UNLISTED_PATHS: &str = "(more changed paths than the diff could list)";

/// The repo-relative paths whose change can run code on this box once the branch is checked out
/// or worked in here (plan P9, PRD OQ-1). Matched ASCII-case-insensitively, since a
/// case-insensitive filesystem reads `.Claude/` as `.claude/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathRules {
    /// Directories at the root, each ending in `/`: anything under one, and the entry itself.
    pub prefixes: &'static [&'static str],
    /// Files at the root, matched exactly.
    pub root_files: &'static [&'static str],
    /// File names matched in any directory.
    pub basenames: &'static [&'static str],
}

/// Plan P9's list.
pub const HOST_EXECUTING_PATHS: PathRules = PathRules {
    prefixes: &[
        ".claude/",
        ".clinerules/",
        ".cursor/",
        ".gemini/",
        ".codex/",
        ".agent/",
        ".agents/",
        ".vscode/",
        ".idea/",
        ".githooks/",
        ".husky/",
        ".cargo/",
        ".devcontainer/",
    ],
    root_files: &[
        ".mcp.json",
        ".envrc",
        ".pre-commit-config.yaml",
        "rust-toolchain",
        "rust-toolchain.toml",
        "Makefile",
        "justfile",
        "package.json",
        ".npmrc",
        ".gitmodules",
        "compose.yaml",
        "docker-compose.yml",
    ],
    basenames: &[
        "CLAUDE.md",
        "CLAUDE.local.md",
        "AGENTS.md",
        "GEMINI.md",
        ".gitattributes",
        "build.rs",
    ],
};

/// The entries of `changed` that [`HOST_EXECUTING_PATHS`] or `extra` name, sorted and
/// deduplicated. An `extra` entry ending in `/` is a prefix (a directory at the root, or deeper),
/// any other is an exact path (plan P9: M2 reads them from
/// `project.settings.collect_confirm_paths`).
#[must_use]
pub fn host_executing(changed: &[String], extra: &[String]) -> Vec<String> {
    let mut listed: Vec<String> = changed
        .iter()
        .filter(|path| runs_here(path, extra))
        .cloned()
        .collect();
    listed.sort();
    listed.dedup();
    listed
}

/// Whether `path` is one [`host_executing`] lists.
fn runs_here(path: &str, extra: &[String]) -> bool {
    let rules = &HOST_EXECUTING_PATHS;
    let basename = path.rsplit('/').next().unwrap_or(path);
    let (extra_prefixes, extra_files): (Vec<&str>, Vec<&str>) = extra
        .iter()
        .map(String::as_str)
        .partition(|entry| entry.ends_with('/'));
    rules
        .prefixes
        .iter()
        .copied()
        .chain(extra_prefixes)
        .any(|prefix| under(path, prefix))
        || rules
            .root_files
            .iter()
            .copied()
            .chain(extra_files)
            .any(|file| path.eq_ignore_ascii_case(file))
        || rules
            .basenames
            .iter()
            .any(|name| basename.eq_ignore_ascii_case(name))
}

/// Whether `path` is the directory `prefix` (which ends in `/`) or lies under it.
fn under(path: &str, prefix: &str) -> bool {
    let dir = &prefix[..prefix.len() - 1];
    (!dir.is_empty() && path.eq_ignore_ascii_case(dir))
        || path
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// Plan P8: the temp ref of one run's collect of one repo, `refs/htui-collect/<run>/<repo>`. A
/// run may span repos and an item may have several runs; uuids need no ref-name escaping.
#[must_use]
pub fn temp_ref(run: RunId, repo: RepoId) -> String {
    format!("{TEMP_REF_PREFIX}{run}/{repo}")
}

/// The person's yes, as a waiting row stores it: valid for one tip and one path list only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consent {
    /// The tip consented to.
    pub tip: String,
    /// The local branch may be rewritten; without it a rewrite is never let through.
    pub force: bool,
    /// The host-executing paths consented to, as [`CollectOutcome::NeedsConsent`] listed them.
    pub config_paths: Vec<String>,
}

/// One collect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectRequest {
    /// This box's checkout, where the objects and the local branch land.
    pub repo: PathBuf,
    /// The clone's directory on the target, as seeded.
    pub target_dir: String,
    /// The target's run branch, a short name (`htui/run/…`).
    pub run_branch: String,
    /// The commit the clone was seeded at (40 lowercase hex): the bundle's lower bound.
    pub base: String,
    /// The branch in [`repo`](Self::repo) that moves to the tip, a short name.
    pub local_branch: String,
    /// The temp ref, from [`temp_ref`]; it must start with [`TEMP_REF_PREFIX`].
    pub temp_ref: String,
    /// More host-executing paths: a prefix ends in `/`, anything else is exact.
    pub extra_confirm_paths: Vec<String>,
    /// An absolute directory on this box the bundle is written to and removed from.
    pub scratch: PathBuf,
    /// The most bytes the bundle may hold; [`COLLECT_CAP`] by default.
    pub cap: u64,
    /// The person's consent, if a previous call asked for it.
    pub consent: Option<Consent>,
}

/// What [`collect`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectOutcome {
    /// The run branch is still at the base: nothing to collect, nothing written.
    Nothing,
    /// The local branch did not exist and now points at the tip.
    Created {
        /// The collected tip.
        tip: String,
    },
    /// The local branch moved forward to the tip.
    Moved {
        /// Where it was.
        from: String,
        /// The collected tip.
        tip: String,
    },
    /// The local branch was rewritten to the tip, with consent.
    Forced {
        /// Where it was; no longer reachable from it.
        from: String,
        /// The collected tip.
        tip: String,
    },
    /// Nothing moved: the person must consent to this tip (and to `config_paths`).
    NeedsConsent {
        /// The tip to consent to.
        tip: String,
        /// The local branch's tip the move would drop, when it is a rewrite.
        rewritten_from: Option<String>,
        /// The changed host-executing paths, sorted; empty when only the rewrite needs consent.
        config_paths: Vec<String>,
    },
}

/// Why [`collect`] moved nothing.
#[derive(Debug, thiserror::Error)]
pub enum CollectRefusal {
    /// The channel could not run a command; its own sentence.
    #[error(transparent)]
    Channel(#[from] ChannelError),
    /// A verb on this box failed, or the request was refused before anything ran.
    #[error(transparent)]
    Git(#[from] IsolateError),
    /// The bundle's header was refused.
    #[error(transparent)]
    Bundle(#[from] BundleRefusal),
    /// The target clone has no such run branch.
    #[error("{}", no_branch(.dir, .branch))]
    NoBranch {
        /// The target directory.
        dir: String,
        /// The run branch.
        branch: String,
    },
    /// The target answered something other than a full hash for the run branch's tip.
    #[error("{}", tip_not_a_hash(.got))]
    NotAHash {
        /// What it answered, cut to 80 characters.
        got: String,
    },
    /// The bundle does not carry exactly the run branch at the tip read before it was made.
    #[error("{}", changed(.expected, .got))]
    Changed {
        /// `<tip> refs/heads/<run>`.
        expected: String,
        /// The bundle's refs, `<hash> <name>` joined with `, `.
        got: String,
    },
    /// A prerequisite this box does not hold as a commit.
    #[error("{}", missing_prerequisite(.hash))]
    MissingPrerequisite {
        /// The prerequisite.
        hash: String,
    },
    /// `git bundle verify` refused the bundle; its sentence.
    #[error("{0}")]
    VerifyFailed(String),
    /// `git index-pack --strict` refused the pack; its sentence.
    #[error("{0}")]
    FsckFailed(String),
    /// The tip's history is not complete on this box.
    #[error("{}", not_connected(.tip))]
    NotConnected {
        /// The tip.
        tip: String,
    },
    /// The local branch is checked out (or mid-rebase or -bisect) in a tree.
    #[error("{}", checked_out(.branch, .at))]
    CheckedOut {
        /// The local branch.
        branch: String,
        /// The tree holding it.
        at: PathBuf,
    },
    /// The local branch moved between the decision and the compare-and-swap.
    #[error("{}", local_moved(.branch))]
    LocalMoved {
        /// The local branch.
        branch: String,
    },
    /// A `git` verb on the target exited non-zero.
    #[error("{}", target_git_failed(.verb, .dir, .stderr))]
    TargetGit {
        /// The verb (`"bundle create"`).
        verb: &'static str,
        /// The target directory.
        dir: String,
        /// Its stderr's last line, or the exit status when it wrote none.
        stderr: String,
    },
}

// One named function per sentence (plan P11).

/// [`CollectRefusal::NoBranch`].
#[must_use]
pub fn no_branch(dir: &str, branch: &str) -> String {
    format!("the target clone {dir} has no branch {branch} to collect")
}

/// [`CollectRefusal::NotAHash`].
#[must_use]
pub fn tip_not_a_hash(got: &str) -> String {
    format!("the target answered {got:?} for the run branch's tip, not a full lowercase hash")
}

/// [`CollectRefusal::Changed`].
#[must_use]
pub fn changed(expected: &str, got: &str) -> String {
    format!(
        "the collected bundle carries {got}, not exactly {expected}: the target changed under \
         the collect"
    )
}

/// [`CollectRefusal::MissingPrerequisite`].
#[must_use]
pub fn missing_prerequisite(hash: &str) -> String {
    format!("the collected bundle needs the commit {hash}, which this box does not hold")
}

/// [`CollectRefusal::NotConnected`].
#[must_use]
pub fn not_connected(tip: &str) -> String {
    format!("the collected tip {tip} is missing part of its history on this box")
}

/// [`CollectRefusal::CheckedOut`].
#[must_use]
pub fn checked_out(branch: &str, at: &Path) -> String {
    format!(
        "the branch {branch} is checked out in {}: collect does not move it",
        at.display()
    )
}

/// [`CollectRefusal::LocalMoved`].
#[must_use]
pub fn local_moved(branch: &str) -> String {
    format!("the branch {branch} moved while it was being collected into; collect again")
}

/// The local branch is not a branch name `git` and this module can pass as one argument.
#[must_use]
pub fn bad_local_branch(repo: &Path, branch: &str) -> String {
    format!(
        "the local branch {branch:?} in {} is not a valid branch name",
        repo.display()
    )
}

/// The temp ref is not a valid ref under [`TEMP_REF_PREFIX`].
#[must_use]
pub fn bad_temp_ref(repo: &Path, name: &str) -> String {
    format!(
        "the collect temp ref {name:?} in {} is not a valid ref under {TEMP_REF_PREFIX}",
        repo.display()
    )
}

/// The scratch directory is relative.
#[must_use]
pub fn scratch_not_absolute(scratch: &Path) -> String {
    format!(
        "the collect scratch {} must be an absolute path",
        scratch.display()
    )
}

/// The quarantine file: removed on drop, so on every exit.
struct Quarantine(PathBuf);

impl Drop for Quarantine {
    fn drop(&mut self) {
        // NotFound is the normal case after a refusal before the bundle was written.
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Collects `req.run_branch` of the clone behind `channel` into `req.local_branch` of `req.repo`
/// (plan P6).
///
/// # Errors
/// [`CollectRefusal::Git`] for a bad request or a failing verb on this box,
/// [`CollectRefusal::Channel`] when the target cannot run a command (or the bundle passes the
/// cap), [`CollectRefusal::TargetGit`] when a target verb exits non-zero, and one variant per
/// refused check: see [`CollectRefusal`].
pub async fn collect(
    cli: &Cli,
    channel: &dyn TargetChannel,
    req: &CollectRequest,
) -> Result<CollectOutcome, CollectRefusal> {
    collect_with(cli, channel, req, &|| {}).await
}

/// [`collect`] with `before_cas` run between the decision and the compare-and-swap: the test seam
/// for a local branch that moves under the collect.
///
/// # Errors
/// As [`collect`].
#[cfg(feature = "test-support")]
pub async fn collect_racing(
    cli: &Cli,
    channel: &dyn TargetChannel,
    req: &CollectRequest,
    before_cas: &(dyn Fn() + Send + Sync),
) -> Result<CollectOutcome, CollectRefusal> {
    collect_with(cli, channel, req, before_cas).await
}

async fn collect_with(
    cli: &Cli,
    channel: &dyn TargetChannel,
    req: &CollectRequest,
    before_cas: &(dyn Fn() + Send + Sync),
) -> Result<CollectOutcome, CollectRefusal> {
    validate(req)?;
    let quarantine = Quarantine(
        req.scratch
            .join(format!("collect-{}.bundle", uuid::Uuid::now_v7())),
    );
    let outcome = Collect {
        cli,
        channel,
        req,
        quarantine: &quarantine.0,
    }
    .run(before_cas)
    .await;
    let cleared = delete_temp_ref(&req.repo, &req.temp_ref).await;
    drop(quarantine);
    if let Err(err) = cleared {
        // The next collect deletes a leftover before it writes; the outcome already happened.
        tracing::warn!(
            temp_ref = %req.temp_ref,
            repo = %req.repo.display(),
            %err,
            "the collect temp ref could not be removed"
        );
    }
    outcome
}

/// Refuses a request before anything runs.
fn validate(req: &CollectRequest) -> Result<(), IsolateError> {
    let dir = req.target_dir.as_str();
    if dir.is_empty() || dir.starts_with('-') {
        return Err(IsolateError::Refused(bad_target_dir(dir)));
    }
    if !valid_branch(&req.run_branch) {
        return Err(IsolateError::Refused(bad_run_branch(dir, &req.run_branch)));
    }
    if !valid_branch(&req.local_branch) {
        return Err(IsolateError::Refused(bad_local_branch(
            &req.repo,
            &req.local_branch,
        )));
    }
    if !is_full_hash(&req.base) {
        return Err(IsolateError::Refused(git::not_a_hash(&req.repo, &req.base)));
    }
    if !req.temp_ref.starts_with(TEMP_REF_PREFIX)
        || gix::validate::reference::name(req.temp_ref.as_bytes().into()).is_err()
    {
        return Err(IsolateError::Refused(bad_temp_ref(
            &req.repo,
            &req.temp_ref,
        )));
    }
    if req.scratch.is_relative() {
        return Err(IsolateError::Refused(scratch_not_absolute(&req.scratch)));
    }
    Ok(())
}

/// A short branch name that is a valid `refs/heads/<name>` and cannot be read as an option.
fn valid_branch(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.starts_with("refs/")
        && gix::validate::reference::name(format!("refs/heads/{name}").as_bytes().into()).is_ok()
}

/// Deletes `full_ref` in `repo` if it is there.
async fn delete_temp_ref(repo: &Path, full_ref: &str) -> Result<(), IsolateError> {
    with_retry("delete-ref", || {
        let (repo, full_ref) = (repo.to_path_buf(), full_ref.to_owned());
        blocking(move || git::delete_ref(&repo, &full_ref))
    })
    .await
    .map(drop)
}

/// One collect in flight.
struct Collect<'a> {
    cli: &'a Cli,
    channel: &'a dyn TargetChannel,
    req: &'a CollectRequest,
    quarantine: &'a Path,
}

impl Collect<'_> {
    async fn run(
        &self,
        before_cas: &(dyn Fn() + Send + Sync),
    ) -> Result<CollectOutcome, CollectRefusal> {
        let req = self.req;
        let full_run = format!("refs/heads/{}", req.run_branch);

        let tip = self.target_tip(&full_run).await?;
        if tip == req.base {
            return Ok(CollectOutcome::Nothing);
        }
        self.bundle(&full_run).await?;
        self.check(&full_run, &tip).await?;

        delete_temp_ref(&req.repo, &req.temp_ref).await?;
        with_retry("create temp ref", || {
            let (repo, name, tip) = (req.repo.clone(), req.temp_ref.clone(), tip.clone());
            blocking(move || git::create_ref(&repo, &name, &tip))
        })
        .await?;

        self.decide_and_move(tip, before_cas).await
    }

    /// Step 1: the run branch's tip on the target.
    async fn target_tip(&self, full_run: &str) -> Result<String, CollectRefusal> {
        let dir = &self.req.target_dir;
        let out = self
            .exec(
                "rev-parse",
                git_in(
                    dir,
                    &[
                        "rev-parse",
                        "--verify",
                        "--quiet",
                        &format!("{full_run}^{{commit}}"),
                    ],
                ),
                StdoutTo::Tail,
                VERB_TIMEOUT,
            )
            .await?;
        match out.code {
            Some(0) => {}
            Some(1) if out.stdout.trim().is_empty() => {
                return Err(CollectRefusal::NoBranch {
                    dir: dir.clone(),
                    branch: self.req.run_branch.clone(),
                });
            }
            _ => return Err(self.target_git("rev-parse", &out)),
        }
        let tip = out.stdout.trim();
        if !is_full_hash(tip) {
            return Err(CollectRefusal::NotAHash {
                got: tip.chars().take(80).collect(),
            });
        }
        Ok(tip.to_owned())
    }

    /// Step 3: the bundle, into the quarantine file.
    async fn bundle(&self, full_run: &str) -> Result<(), CollectRefusal> {
        let out = self
            .exec(
                "bundle create",
                git_in(
                    &self.req.target_dir,
                    &[
                        "bundle",
                        "create",
                        "-",
                        full_run,
                        &format!("^{}", self.req.base),
                    ],
                ),
                StdoutTo::File {
                    path: self.quarantine.to_path_buf(),
                    cap: self.req.cap,
                },
                TRANSFER_TIMEOUT,
            )
            .await?;
        if out.ok() {
            Ok(())
        } else {
            Err(self.target_git("bundle create", &out))
        }
    }

    /// Step 4: every check on this box, before any ref is written.
    async fn check(&self, full_run: &str, tip: &str) -> Result<(), CollectRefusal> {
        let req = self.req;
        let header = read_header(self.quarantine)?;
        if !matches!(header.refs.as_slice(), [(hash, name)] if hash == tip && name == full_run) {
            let got = if header.refs.is_empty() {
                "no ref".to_owned()
            } else {
                header
                    .refs
                    .iter()
                    .map(|(hash, name)| format!("{hash} {name}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            return Err(CollectRefusal::Changed {
                expected: format!("{tip} {full_run}"),
                got,
            });
        }

        let missing = {
            let (repo, prerequisites) = (req.repo.clone(), header.prerequisites.clone());
            blocking(move || {
                for hash in prerequisites {
                    if !git::has_commit(&repo, &hash)? {
                        return Ok(Some(hash));
                    }
                }
                Ok(None)
            })
            .await?
        };
        if let Some(hash) = missing {
            return Err(CollectRefusal::MissingPrerequisite { hash });
        }

        match self.cli.bundle_verify(&req.repo, self.quarantine).await {
            Err(IsolateError::Refused(sentence)) => {
                return Err(CollectRefusal::VerifyFailed(sentence));
            }
            other => other?,
        }
        match self
            .cli
            .with_transfer_budget()
            .index_pack(&req.repo, self.quarantine, header.pack_offset, true)
            .await
        {
            Err(IsolateError::Refused(sentence)) => {
                return Err(CollectRefusal::FsckFailed(sentence));
            }
            other => other?,
        }
        if !self
            .cli
            .connected(&req.repo, tip, &header.prerequisites)
            .await?
        {
            return Err(CollectRefusal::NotConnected {
                tip: tip.to_owned(),
            });
        }
        Ok(())
    }

    /// Step 6: the decision, and the compare-and-swap.
    async fn decide_and_move(
        &self,
        tip: String,
        before_cas: &(dyn Fn() + Send + Sync),
    ) -> Result<CollectOutcome, CollectRefusal> {
        let req = self.req;
        let branch = req.local_branch.clone();
        let full_local = format!("refs/heads/{branch}");

        let (local, at, rewritten) = {
            let (repo, full_local, tip) = (req.repo.clone(), full_local.clone(), tip.clone());
            blocking(move || {
                let local = git::ref_target(&repo, &full_local)?;
                let at = git::checked_out_at(&repo, &full_local)?;
                let rewritten = match &local {
                    Some(old) => !git::is_ancestor(&repo, old, &tip)?,
                    None => false,
                };
                Ok((local, at, rewritten))
            })
            .await?
        };
        if let Some(at) = at {
            return Err(CollectRefusal::CheckedOut { branch, at });
        }

        let old = local.clone().unwrap_or_else(|| req.base.clone());
        let mut changed = self.cli.name_only(&req.repo, &old, &tip).await?;
        // A non-UTF-8 name is matched on its lossy decoding: a skill or a `build.rs` under one
        // must not slip past consent (P9).
        changed.paths.append(&mut changed.unreadable);
        let mut config_paths = host_executing(&changed.paths, &req.extra_confirm_paths);
        if changed.truncated {
            config_paths.push(UNLISTED_PATHS.to_owned());
            config_paths.sort();
        }
        let rewritten_from = local.clone().filter(|_| rewritten);

        if rewritten_from.is_some() || !config_paths.is_empty() {
            let consented = req.consent.as_ref().is_some_and(|consent| {
                let mut named = consent.config_paths.clone();
                named.sort();
                named.dedup();
                consent.tip == tip
                    && named == config_paths
                    && (rewritten_from.is_none() || consent.force)
            });
            if !consented {
                return Ok(CollectOutcome::NeedsConsent {
                    tip,
                    rewritten_from,
                    config_paths,
                });
            }
        }

        before_cas();
        let swapped = {
            let (repo, branch, local, tip) =
                (req.repo.clone(), branch.clone(), local.clone(), tip.clone());
            with_retry("collect branch", move || {
                let (repo, branch, local, tip) =
                    (repo.clone(), branch.clone(), local.clone(), tip.clone());
                blocking(move || match &local {
                    None => git::create_branch(&repo, &branch, &tip),
                    Some(from) => git::move_branch(&repo, &branch, from, &tip),
                })
            })
            .await
        };
        if let Err(err) = swapped {
            let now = {
                let (repo, full_local) = (req.repo.clone(), full_local.clone());
                blocking(move || git::ref_target(&repo, &full_local)).await?
            };
            return Err(if now == local {
                err.into()
            } else {
                CollectRefusal::LocalMoved { branch }
            });
        }

        Ok(match local {
            None => CollectOutcome::Created { tip },
            Some(from) if rewritten => CollectOutcome::Forced { from, tip },
            Some(from) => CollectOutcome::Moved { from, tip },
        })
    }

    /// Runs `argv` on the target with a null stdin.
    async fn exec(
        &self,
        verb: &'static str,
        argv: Vec<String>,
        stdout: StdoutTo,
        timeout: Duration,
    ) -> Result<ExecOutput, ChannelError> {
        let spec = ExecSpec {
            verb,
            argv,
            stdin: StdinFrom::Null,
            stdout,
            timeout,
        };
        self.channel.exec(&spec).await
    }

    /// [`CollectRefusal::TargetGit`] over `out`'s last stderr line.
    fn target_git(&self, verb: &'static str, out: &ExecOutput) -> CollectRefusal {
        let stderr = out
            .stderr
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map_or_else(
                || match out.code {
                    Some(code) => format!("exit status {code} with no stderr"),
                    None => "killed by a signal".to_owned(),
                },
                str::to_owned,
            );
        CollectRefusal::TargetGit {
            verb,
            dir: self.req.target_dir.clone(),
            stderr,
        }
    }
}

/// `git -C <dir> <args>…` as an argv.
fn git_in(dir: &str, args: &[&str]) -> Vec<String> {
    ["git", "-C", dir]
        .iter()
        .chain(args)
        .map(|arg| (*arg).to_owned())
        .collect()
}
