//! MOD-132 plan P6, P8, P9: collect. A target clone's run branch comes back to this box as a
//! bundle, and a local branch moves to its tip, in one stateless call.
//!
//! On the target, through the [`TargetChannel`]: `rev-parse` of the run branch (its tip, 40
//! lowercase hex), then `git bundle create - refs/heads/<run> ^<base>` into a quarantine file on
//! this box (`create_new`, `0600`, at most [`CollectRequest::cap`] bytes). A tip at the base is
//! [`CollectOutcome::Nothing`] and makes no bundle.
//!
//! On this box, before any object reaches the repository: the header ([`read_header`]) carries
//! exactly one ref, the run branch at the tip just read; every prerequisite is a commit this box
//! holds; `git bundle verify` passes; `git index-pack --strict --fix-thin` takes the pack (the
//! fsck `git fetch` skips on a bundle, probe B5) into an object quarantine
//! ([`ObjectQuarantine`], R1b M4) that reads the repository's objects as an alternate;
//! `rev-list --objects` finds the tip connected; and the tip is a commit.
//!
//! The decision, read through the quarantine: the local branch must not be checked out anywhere;
//! it is created, moved forward, or (rewritten) forced. A rewrite, or a changed path that runs on
//! this box ([`HOST_EXECUTING_PATHS`], plus the caller's), needs a [`Consent`] naming this tip,
//! the local tip it was given over and this path list, else the outcome is
//! [`CollectOutcome::NeedsConsent`] and nothing moves. Only a move takes the quarantined pack into
//! the repository, and then the branch moves by compare-and-swap; a branch moved under it is
//! [`CollectRefusal::LocalMoved`].
//!
//! The bundle file and the quarantine are removed on every exit, a dropped future included.
//! Nothing writes a temp ref any more: objects stay out of the repository until the move, so
//! there is nothing to hold through the decision; [`sweep_temp_refs`] removes those an earlier
//! build left. Waiting, retrying and asking the person are M2's (PRD D5): every fact a waiting
//! row needs is in the outcome.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::isolate::IsolateError;
use crate::isolate::git::{
    self, Cli, ObjectQuarantine, TRANSFER_TIMEOUT, VERB_TIMEOUT, blocking, with_retry,
};
use crate::target::bundle::{BundleRefusal, is_full_hash, read_header};
use crate::target::seed::{bad_run_branch, bad_target_dir, target_git_failed};
use crate::target::util::{Quarantine, escape_controls, git_in, last_stderr_line};
use htui_agent::target_channel::{
    ChannelError, ExecOutput, ExecSpec, StdinFrom, StdoutTo, TargetChannel,
};

/// The default most bytes a collected bundle may hold: 2 GiB.
pub const COLLECT_CAP: u64 = 2 << 30;

/// Where the collect temp refs of earlier builds live (plan P8); [`sweep_temp_refs`] removes
/// them. A collect no longer writes one (R1b M4).
pub const TEMP_REF_PREFIX: &str = "refs/htui-collect/";

/// The entry [`CollectOutcome::NeedsConsent`] lists when the changed paths overflowed the diff
/// cap: the paths past it are unknown, so consent is asked as if one of them ran on this box.
pub const UNLISTED_PATHS: &str = "(more changed paths than the diff could list)";

/// The repo-relative paths whose change can run code on this box once the branch is checked out
/// or worked in here (plan P9, PRD OQ-1). Matched case-insensitively beyond ASCII and with each
/// segment's trailing `.` and spaces dropped (R1b L1), since a case-insensitive filesystem reads
/// `.Claude/` and `.huſky/` as `.claude/` and `.husky/`, and NTFS reads `CLAUDE.md.` as
/// `CLAUDE.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathRules {
    /// Directories at the root, each ending in `/`: anything under one, and the entry itself.
    pub prefixes: &'static [&'static str],
    /// Directory names, each ending in `/`, matched at any depth: anything under one, and the
    /// entry itself.
    pub directories: &'static [&'static str],
    /// Files at the root, matched exactly.
    pub root_files: &'static [&'static str],
    /// File names matched in any directory.
    pub basenames: &'static [&'static str],
}

/// Plan P9's list, widened by the maintainer's decision of 2026-11-16 (R1b M6).
///
/// `.cargo/` counts at any depth, and `rust-toolchain`, `rust-toolchain.toml`, `package.json`
/// and `.envrc` as names in any directory: each runs code as soon as a tool is used in that
/// directory (cargo's config can name a runner or a wrapper, a toolchain file installs and runs
/// a toolchain, `package.json` names install scripts, direnv loads `.envrc`), whatever the
/// person meant to do there. `Cargo.toml` and the lockfiles stay off the list: what they make a
/// build run is the person's own step, like any change to the source they choose to build.
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
        ".devcontainer/",
    ],
    directories: &[".cargo/"],
    root_files: &[
        ".mcp.json",
        ".pre-commit-config.yaml",
        "Makefile",
        "justfile",
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
        "rust-toolchain",
        "rust-toolchain.toml",
        "package.json",
        ".envrc",
    ],
};

/// The entries of `changed` that [`HOST_EXECUTING_PATHS`] or `extra` name, sorted and
/// deduplicated. An `extra` entry ending in `/` is a prefix (a directory at the root, or deeper),
/// any other is an exact path (plan P9: M2 reads them from
/// `project.settings.collect_confirm_paths`).
#[must_use]
pub fn host_executing(changed: &[String], extra: &[String]) -> Vec<String> {
    let rules = Rules::new(extra);
    let mut listed: Vec<String> = changed
        .iter()
        .filter(|path| rules.lists(path))
        .cloned()
        .collect();
    listed.sort();
    listed.dedup();
    listed
}

/// [`HOST_EXECUTING_PATHS`] and the caller's entries, folded once per [`host_executing`] call.
struct Rules {
    prefixes: Vec<String>,
    directories: Vec<String>,
    files: Vec<String>,
    basenames: Vec<String>,
}

impl Rules {
    fn new(extra: &[String]) -> Self {
        let rules = &HOST_EXECUTING_PATHS;
        let folded = |entries: &[&str]| entries.iter().map(|entry| fold(entry)).collect();
        let (extra_prefixes, extra_files): (Vec<&String>, Vec<&String>) =
            extra.iter().partition(|entry| entry.ends_with('/'));
        let mut prefixes: Vec<String> = folded(rules.prefixes);
        prefixes.extend(extra_prefixes.into_iter().map(|entry| fold(entry)));
        let mut files: Vec<String> = folded(rules.root_files);
        files.extend(extra_files.into_iter().map(|entry| fold(entry)));
        Self {
            prefixes,
            directories: folded(rules.directories),
            files,
            basenames: folded(rules.basenames),
        }
    }

    /// Whether `path` is one [`host_executing`] lists.
    fn lists(&self, path: &str) -> bool {
        let path = fold(path);
        let basename = path.rsplit('/').next().unwrap_or(&path);
        self.prefixes.iter().any(|prefix| under(&path, prefix))
            || self.directories.iter().any(|dir| anywhere(&path, dir))
            || self.files.contains(&path)
            || self.basenames.iter().any(|name| name == basename)
    }
}

/// `path` as a case-insensitive filesystem compares it: each `/`-separated segment without its
/// trailing `.` and spaces (NTFS drops them), upper- then lower-cased, so `ſ` (U+017F) folds to
/// `s` and `K` (U+212A) to `k` as they do on such a filesystem. Folding can only add matches.
fn fold(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            segment
                .trim_end_matches(['.', ' '])
                .to_uppercase()
                .to_lowercase()
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether the folded `path` is the directory `prefix` (which ends in `/`) or lies under it.
fn under(path: &str, prefix: &str) -> bool {
    let dir = &prefix[..prefix.len() - 1];
    (!dir.is_empty() && path == dir) || path.starts_with(prefix)
}

/// Whether the folded `path` is the directory `dir` (which ends in `/`), or lies under it, at
/// any depth.
fn anywhere(path: &str, dir: &str) -> bool {
    under(path, dir)
        || path
            .match_indices('/')
            .any(|(at, _)| under(&path[at + 1..], dir))
}

/// `path` as a consent prompt shows it: every control character and line separator written as
/// `\u{..}` (R1b L2). [`CollectOutcome::NeedsConsent`] keeps the raw names, which a [`Consent`]
/// is compared with; M2 renders each through this.
#[must_use]
pub fn display_path(path: &str) -> String {
    escape_controls(path)
}

/// Deletes every ref under [`TEMP_REF_PREFIX`] in `repo`, as earlier builds of collect wrote
/// them (M2 calls it at startup), and returns how many it deleted.
///
/// # Errors
/// [`IsolateError::Git`] when the repository or its refs cannot be read, or a delete fails.
pub fn sweep_temp_refs(repo: &Path) -> Result<usize, IsolateError> {
    let names = {
        let opened = git::open(repo)?;
        let unreadable = |err: &dyn std::fmt::Display| {
            IsolateError::Git(format!("cannot read the refs of {}: {err}", repo.display()))
        };
        let refs = opened.references().map_err(|err| unreadable(&err))?;
        let mut names = Vec::new();
        for reference in refs.all().map_err(|err| unreadable(&err))? {
            let name = reference
                .map_err(|err| unreadable(&err))?
                .name()
                .as_bstr()
                .to_string();
            if name.starts_with(TEMP_REF_PREFIX) {
                names.push(name);
            }
        }
        names
    };
    let mut swept = 0;
    for name in names {
        if git::delete_ref(repo, &name)? {
            swept += 1;
        }
    }
    Ok(swept)
}

/// The person's yes, as a waiting row stores it: valid for one tip, one local tip and one path
/// list only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consent {
    /// The tip consented to.
    pub tip: String,
    /// The local branch's tip when the person was asked: [`CollectOutcome::NeedsConsent`]'s
    /// `rewritten_from` for a rewrite, the local tip when only paths were listed, `None` when the
    /// branch did not exist (R1b M3). A branch that moved since asks again.
    pub from: Option<String>,
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
    /// More host-executing paths: a prefix ends in `/`, anything else is exact.
    pub extra_confirm_paths: Vec<String>,
    /// An absolute directory on this box the bundle and the object quarantine are written to and
    /// removed from.
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
    /// Nothing moved: the person must consent to this tip (and to `config_paths`), over the local
    /// branch as it is now.
    NeedsConsent {
        /// The tip to consent to.
        tip: String,
        /// The local branch's tip the move would drop, when it is a rewrite.
        rewritten_from: Option<String>,
        /// The changed host-executing paths, raw and sorted; empty when only the rewrite needs
        /// consent. A prompt shows each through [`display_path`].
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
    /// The tip is not a commit (a tree, a tag or a blob): no branch may point at it.
    #[error("{}", not_a_commit(.tip))]
    NotACommit {
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

/// [`CollectRefusal::NotACommit`].
#[must_use]
pub fn not_a_commit(tip: &str) -> String {
    format!("the collected tip {tip} is not a commit, so no branch is moved to it")
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

/// The scratch directory is relative.
#[must_use]
pub fn scratch_not_absolute(scratch: &Path) -> String {
    format!(
        "the collect scratch {} must be an absolute path",
        scratch.display()
    )
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
    let id = uuid::Uuid::now_v7();
    // Held across every await: a refusal, a panic, or a future dropped mid-way removes both.
    let scratch = Quarantine::with_dir(
        req.scratch.join(format!("collect-{id}.bundle")),
        req.scratch.join(format!("collect-{id}-objects")),
    );
    Collect {
        cli,
        channel,
        req,
        bundle: scratch.path(),
        objects: scratch.dir().expect("a collect scratch has a directory"),
    }
    .run(before_cas)
    .await
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

/// One collect in flight.
struct Collect<'a> {
    cli: &'a Cli,
    channel: &'a dyn TargetChannel,
    req: &'a CollectRequest,
    /// The bundle file in the scratch.
    bundle: &'a Path,
    /// The object quarantine directory in the scratch, made by [`Collect::check`].
    objects: &'a Path,
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
        self.make_bundle(&full_run).await?;
        let quarantine = self.check(&full_run, &tip).await?;
        self.decide_and_move(tip, &quarantine, before_cas).await
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

    /// Step 3: the bundle, into the scratch file.
    async fn make_bundle(&self, full_run: &str) -> Result<(), CollectRefusal> {
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
                    path: self.bundle.to_path_buf(),
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

    /// Step 4: every check on this box, before any object reaches the repository. The pack is
    /// indexed into the object quarantine this returns, and stays there unless the branch moves.
    async fn check(&self, full_run: &str, tip: &str) -> Result<ObjectQuarantine, CollectRefusal> {
        let req = self.req;
        let header = read_header(self.bundle)?;
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

        match self.cli.bundle_verify(&req.repo, self.bundle).await {
            Err(IsolateError::Refused(sentence)) => {
                return Err(CollectRefusal::VerifyFailed(sentence));
            }
            other => other?,
        }

        let quarantine = {
            let (repo, dir) = (req.repo.clone(), self.objects.to_path_buf());
            blocking(move || {
                private_dir(&dir)?;
                private_dir(&dir.join("pack"))?;
                Ok(ObjectQuarantine {
                    dir,
                    objects: git::objects_dir(&repo)?,
                })
            })
            .await?
        };
        let cli = self.cli.in_quarantine(&quarantine);
        match cli
            .with_transfer_budget()
            .index_pack(&req.repo, self.bundle, header.pack_offset, req.cap)
            .await
        {
            Err(IsolateError::Refused(sentence)) => {
                return Err(CollectRefusal::FsckFailed(sentence));
            }
            other => other?,
        }
        if !cli.connected(&req.repo, tip, &header.prerequisites).await? {
            return Err(CollectRefusal::NotConnected {
                tip: tip.to_owned(),
            });
        }
        if cli.object_kind(&req.repo, tip).await?.as_deref() != Some("commit") {
            return Err(CollectRefusal::NotACommit {
                tip: tip.to_owned(),
            });
        }
        Ok(quarantine)
    }

    /// Step 6: the decision, read through the quarantine; then, for a move, the pack migrates
    /// into the repository and the branch moves by compare-and-swap.
    async fn decide_and_move(
        &self,
        tip: String,
        quarantine: &ObjectQuarantine,
        before_cas: &(dyn Fn() + Send + Sync),
    ) -> Result<CollectOutcome, CollectRefusal> {
        let req = self.req;
        let cli = self.cli.in_quarantine(quarantine);
        let branch = req.local_branch.clone();
        let full_local = format!("refs/heads/{branch}");

        let (local, at) = {
            let (repo, full_local) = (req.repo.clone(), full_local.clone());
            blocking(move || {
                let local = git::ref_target(&repo, &full_local)?;
                let at = git::checked_out_at(&repo, &full_local)?;
                Ok((local, at))
            })
            .await?
        };
        if let Some(at) = at {
            return Err(CollectRefusal::CheckedOut { branch, at });
        }
        let rewritten = match &local {
            Some(old) => !cli.is_ancestor_of(&req.repo, old, &tip).await?,
            None => false,
        };

        let old = local.clone().unwrap_or_else(|| req.base.clone());
        let mut changed = cli.name_only(&req.repo, &old, &tip).await?;
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
                    && consent.from == local
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
        {
            let quarantine = quarantine.clone();
            blocking(move || git::migrate_quarantine(&quarantine)).await?;
        }
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
        CollectRefusal::TargetGit {
            verb,
            dir: self.req.target_dir.clone(),
            stderr: last_stderr_line(out),
        }
    }
}

/// Makes the directory `dir`, private to this user (`0700`) where the mode exists; it must not
/// exist yet.
fn private_dir(dir: &Path) -> Result<(), IsolateError> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir).map_err(|err| {
        IsolateError::Git(format!(
            "cannot make the collect quarantine {}: {err}",
            dir.display()
        ))
    })
}
