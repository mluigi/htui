//! MOD-132 plan P5: prepare's clone. The target's repository is seeded from a bundle made on
//! this box, so the target needs no credentials and no network (PRD D4).
//!
//! On this box, `git bundle create <scratch>/seed-<uuid>.bundle <base_ref>` writes the bundle
//! (the quarantine file) and its header names the base. On the target, through the
//! [`TargetChannel`], in order: `git init`, `git index-pack --stdin --fix-thin` fed the bundle's
//! pack, `update-ref` of the run branch at the base, `checkout` of it, `remote add origin` when
//! a URL is given, `remote.origin.pushurl` set to [`PUSH_REFUSED`], and last
//! `htui.seededBase`, so a clone that carries the marker is a complete one.
//!
//! Before any of that the request is checked (a `remote_url` carrying a credential is
//! [`PrepareRefusal::CredentialInUrl`]) and the target directory is probed: one whose
//! `htui.seededBase` is this base and that has the run branch is reused, one with this base but
//! not the run branch is [`PrepareRefusal::NoRunBranch`], one with another base is
//! [`PrepareRefusal::SeededOther`], and any other non-empty directory is
//! [`PrepareRefusal::Occupied`]. The quarantine file, private to this user, is removed on every
//! exit.
//! Registering the clone in the store is M2's; making step worktrees in it is MOD-44's (D3).

use std::path::{Path, PathBuf};
use std::time::Duration;

use htui_agent::target_channel::{
    ChannelError, ExecOutput, ExecSpec, StdinFrom, StdoutTo, TargetChannel,
};

use crate::isolate::IsolateError;
use crate::isolate::git::{Cli, TRANSFER_TIMEOUT, VERB_TIMEOUT};
use crate::target::bundle::{BundleRefusal, read_header};
use crate::target::util::{Quarantine, escape_controls, git_in, last_stderr_line};

/// The `remote.origin.pushurl` every seeded clone carries: not a repository anywhere, so a
/// `git push origin` from the target fails instead of reaching the project's remote.
pub const PUSH_REFUSED: &str = "no-push-from-htui-target";

/// The target-side config key naming the base a clone was seeded from; written last.
const SEEDED_BASE_KEY: &str = "htui.seededBase";

/// What [`seed`] is asked to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedRequest {
    /// This box's checkout, the bundle's source.
    pub repo: PathBuf,
    /// The full ref the clone starts from (`refs/heads/…`); a ref name, never a hash (B3).
    pub base_ref: String,
    /// The clone's directory on the target; a relative one is relative to the target's working
    /// directory (the stand-in's home).
    pub target_dir: String,
    /// The branch the clone is left on, at the base: a short name (`htui/run/…`).
    pub run_branch: String,
    /// `origin`'s fetch URL, if the clone should have one. It must carry no credential: no
    /// password in any scheme's userinfo, and no userinfo at all for http(s) (a token).
    pub remote_url: Option<String>,
    /// An absolute directory on this box the bundle is written to and removed from.
    pub scratch: PathBuf,
}

/// What [`seed`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seeded {
    /// The commit the clone was seeded at, 40 lowercase hex.
    pub base: String,
    /// The directory already held a clone seeded at this base, and was left as it was.
    pub reused: bool,
}

/// Why [`seed`] made no clone.
#[derive(Debug, thiserror::Error)]
pub enum PrepareRefusal {
    /// The channel could not run a command; its own sentence.
    #[error(transparent)]
    Channel(#[from] ChannelError),
    /// A verb on this box failed, or the request was refused before anything ran.
    #[error(transparent)]
    Git(#[from] IsolateError),
    /// The bundle's header was refused.
    #[error(transparent)]
    Bundle(#[from] BundleRefusal),
    /// The target directory holds something that is not a clone this seed made.
    #[error("{}", occupied(.dir))]
    Occupied {
        /// The target directory.
        dir: String,
    },
    /// The target directory holds a clone seeded at this base without the run branch: it is
    /// refused, never seeded again over.
    #[error("{}", no_run_branch(.dir, .branch))]
    NoRunBranch {
        /// The target directory.
        dir: String,
        /// The run branch it lacks.
        branch: String,
    },
    /// The `remote_url` carries a password or a token; the sentence names its host only.
    #[error("{}", credential_in_url(.host))]
    CredentialInUrl {
        /// The URL's host (and port), never its userinfo.
        host: String,
    },
    /// The target directory holds a clone seeded at another base.
    #[error("{}", seeded_other(.dir, .base))]
    SeededOther {
        /// The target directory.
        dir: String,
        /// The base it was seeded at.
        base: String,
    },
    /// A `git` verb on the target exited non-zero.
    #[error("{}", target_git_failed(.verb, .dir, .stderr))]
    TargetGit {
        /// The verb (`"index-pack"`).
        verb: &'static str,
        /// The target directory.
        dir: String,
        /// Its stderr's last line, or the exit status when it wrote none.
        stderr: String,
    },
}

// One named function per sentence (plan P11).

/// [`PrepareRefusal::Occupied`].
#[must_use]
pub fn occupied(dir: &str) -> String {
    format!("the target directory {dir} is not empty and holds no clone htui seeded")
}

/// [`PrepareRefusal::SeededOther`].
#[must_use]
pub fn seeded_other(dir: &str, base: &str) -> String {
    format!("the target directory {dir} holds a clone seeded at {base}, not at this base")
}

/// [`PrepareRefusal::NoRunBranch`].
#[must_use]
pub fn no_run_branch(dir: &str, branch: &str) -> String {
    format!(
        "the target directory {dir} holds a clone seeded at this base but no branch {branch}: \
         clear it to seed it again"
    )
}

/// [`PrepareRefusal::CredentialInUrl`].
#[must_use]
pub fn credential_in_url(host: &str) -> String {
    format!(
        "the remote URL for {host} carries a credential; give the target a URL without one \
         (it never pushes, and fetches need none)"
    )
}

/// [`PrepareRefusal::TargetGit`]; `stderr` is the target's, so its control characters are
/// escaped (R1b L2).
#[must_use]
pub fn target_git_failed(verb: &str, dir: &str, stderr: &str) -> String {
    format!(
        "git {verb} in {dir} on the target failed: {}",
        escape_controls(stderr)
    )
}

/// The run branch is not a branch name `git` and this module can pass as one argument.
#[must_use]
pub fn bad_run_branch(dir: &str, branch: &str) -> String {
    format!("the run branch {branch:?} for {dir} is not a valid branch name")
}

/// The target directory is empty or starts with `-`.
#[must_use]
pub fn bad_target_dir(dir: &str) -> String {
    format!("the target directory {dir:?} must be non-empty and must not start with `-`")
}

/// The seed bundle's header does not name exactly `base_ref`, with no prerequisite.
#[must_use]
pub fn unexpected_seed_bundle(repo: &Path, base_ref: &str) -> String {
    format!(
        "the seed bundle of {base_ref} made in {} does not carry exactly that ref with no \
         prerequisite",
        repo.display()
    )
}

/// The host of a `remote_url` that carries a credential, or `None` for one that carries none:
/// a password in any scheme's userinfo (`scheme://user:pass@host`), or any userinfo in an
/// http(s) URL (`https://token@host`). An ssh user without a password (`ssh://git@host/path`,
/// `git@host:path`) is no credential.
#[must_use]
pub fn credential_host(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let (userinfo, host) = authority.rsplit_once('@')?;
    let http = scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
    (userinfo.contains(':') || (http && !userinfo.is_empty())).then(|| host.to_owned())
}

/// Seeds `req.target_dir` on the target behind `channel` from `req.base_ref` of `req.repo`
/// (plan P5).
///
/// A directory a seed was interrupted in (no `htui.seededBase` marker yet) is
/// [`PrepareRefusal::Occupied`] from then on, until the caller clears it (M2's cleanup): a seed
/// never writes over a directory it cannot prove it made.
///
/// # Errors
/// [`PrepareRefusal::Git`] for a bad request or a failing verb on this box,
/// [`PrepareRefusal::CredentialInUrl`] for a `remote_url` with a credential,
/// [`PrepareRefusal::Bundle`] for a header the parser refuses, [`PrepareRefusal::Channel`] when
/// the target cannot run a command, [`PrepareRefusal::Occupied`],
/// [`PrepareRefusal::NoRunBranch`] and [`PrepareRefusal::SeededOther`] for a directory already
/// in use, and [`PrepareRefusal::TargetGit`] when a target verb exits non-zero.
pub async fn seed(
    cli: &Cli,
    channel: &dyn TargetChannel,
    req: &SeedRequest,
) -> Result<Seeded, PrepareRefusal> {
    let dir = req.target_dir.as_str();
    if dir.is_empty() || dir.starts_with('-') {
        return Err(IsolateError::Refused(bad_target_dir(dir)).into());
    }
    let full_branch = format!("refs/heads/{}", req.run_branch);
    if req.run_branch.is_empty()
        || req.run_branch.starts_with('-')
        || req.run_branch.starts_with("refs/")
        || gix::validate::reference::name(full_branch.as_bytes().into()).is_err()
    {
        return Err(IsolateError::Refused(bad_run_branch(dir, &req.run_branch)).into());
    }
    if let Some(host) = req.remote_url.as_deref().and_then(credential_host) {
        return Err(PrepareRefusal::CredentialInUrl { host });
    }

    let quarantine = Quarantine::file(
        req.scratch
            .join(format!("seed-{}.bundle", uuid::Uuid::now_v7())),
    );
    cli.with_transfer_budget()
        .bundle_create(&req.repo, &req.base_ref, quarantine.path())
        .await?;
    private_file(quarantine.path())?;
    let header = read_header(quarantine.path())?;
    let base = match (header.prerequisites.as_slice(), header.refs.as_slice()) {
        ([], [(hash, name)]) if *name == req.base_ref => hash.clone(),
        _ => {
            return Err(IsolateError::Git(unexpected_seed_bundle(&req.repo, &req.base_ref)).into());
        }
    };

    let target = Target { channel, dir };
    if let Some(reused) = target.probe(&base, &full_branch).await? {
        return Ok(reused);
    }

    target
        .git_ok("init", &["init", "--quiet", "--", dir])
        .await?;
    target
        .run(
            "index-pack",
            git_in(dir, &["index-pack", "--stdin", "--fix-thin"]),
            StdinFrom::File {
                path: quarantine.path().to_path_buf(),
                offset: header.pack_offset,
            },
            TRANSFER_TIMEOUT,
        )
        .await?;
    target
        .git_in_ok("update-ref", &["update-ref", &full_branch, &base])
        .await?;
    target
        .git_in_ok("checkout", &["checkout", "--quiet", &req.run_branch, "--"])
        .await?;
    if let Some(url) = &req.remote_url {
        target
            .git_in_ok("remote add", &["remote", "add", "--", "origin", url])
            .await?;
    }
    target
        .git_in_ok("config", &["config", "remote.origin.pushurl", PUSH_REFUSED])
        .await?;
    target
        .git_in_ok("config", &["config", SEEDED_BASE_KEY, &base])
        .await?;
    drop(quarantine);
    Ok(Seeded {
        base,
        reused: false,
    })
}

/// Makes the seed bundle readable by this user only (`0600`) before the channel reads it.
fn private_file(path: &Path) -> Result<(), IsolateError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|err| {
            IsolateError::Git(format!(
                "cannot make the seed bundle {} private: {err}",
                path.display()
            ))
        })?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// The target directory behind its channel.
struct Target<'a> {
    channel: &'a dyn TargetChannel,
    dir: &'a str,
}

impl Target<'_> {
    /// Runs `argv` with stdout as a tail.
    async fn exec(
        &self,
        verb: &'static str,
        argv: Vec<String>,
        stdin: StdinFrom,
        timeout: Duration,
    ) -> Result<ExecOutput, ChannelError> {
        let spec = ExecSpec {
            verb,
            argv,
            stdin,
            stdout: StdoutTo::Tail,
            timeout,
        };
        self.channel.exec(&spec).await
    }

    /// Runs `argv` and refuses with [`PrepareRefusal::TargetGit`] unless it exits 0.
    async fn run(
        &self,
        verb: &'static str,
        argv: Vec<String>,
        stdin: StdinFrom,
        timeout: Duration,
    ) -> Result<ExecOutput, PrepareRefusal> {
        let out = self.exec(verb, argv, stdin, timeout).await?;
        if out.ok() {
            return Ok(out);
        }
        Err(PrepareRefusal::TargetGit {
            verb,
            dir: self.dir.to_owned(),
            stderr: last_stderr_line(&out),
        })
    }

    /// `git <args>…` (no `-C`) under [`VERB_TIMEOUT`], exit 0 required.
    async fn git_ok(&self, verb: &'static str, args: &[&str]) -> Result<(), PrepareRefusal> {
        let argv = std::iter::once("git")
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect();
        self.run(verb, argv, StdinFrom::Null, VERB_TIMEOUT)
            .await
            .map(drop)
    }

    /// `git -C <dir> <args>…` under [`VERB_TIMEOUT`], exit 0 required.
    async fn git_in_ok(&self, verb: &'static str, args: &[&str]) -> Result<(), PrepareRefusal> {
        self.run(verb, git_in(self.dir, args), StdinFrom::Null, VERB_TIMEOUT)
            .await
            .map(drop)
    }

    /// What the directory already holds: `Some` for a clone seeded at `base` that has the run
    /// branch `full_branch` (reused), `None` for an absent or empty directory, a refusal for
    /// anything else.
    ///
    /// The marker and the branch are read with `--git-dir=<dir>/.git` (and the marker with
    /// `--local`), not `-C <dir>`, so neither a repository above the directory (a home kept in
    /// git) nor the target's global config can answer for them. Emptiness is `ls -A`: exit 0 with nothing listed is an empty directory,
    /// any listing is occupied (a plain file lists itself), and a non-zero exit is taken as
    /// absent, for `git init` to create, or to fail on with its own sentence.
    async fn probe(&self, base: &str, full_branch: &str) -> Result<Option<Seeded>, PrepareRefusal> {
        let marker = self
            .exec(
                "config",
                vec![
                    "git".to_owned(),
                    format!("--git-dir={}/.git", self.dir),
                    "config".to_owned(),
                    "--local".to_owned(),
                    "--get".to_owned(),
                    SEEDED_BASE_KEY.to_owned(),
                ],
                StdinFrom::Null,
                VERB_TIMEOUT,
            )
            .await?;
        match marker.code {
            Some(0) => {
                let seeded = marker.stdout.trim();
                if seeded != base {
                    return Err(PrepareRefusal::SeededOther {
                        dir: self.dir.to_owned(),
                        base: seeded.to_owned(),
                    });
                }
                return if self.has_ref(full_branch).await? {
                    Ok(Some(Seeded {
                        base: base.to_owned(),
                        reused: true,
                    }))
                } else {
                    Err(PrepareRefusal::NoRunBranch {
                        dir: self.dir.to_owned(),
                        branch: full_branch
                            .strip_prefix("refs/heads/")
                            .unwrap_or(full_branch)
                            .to_owned(),
                    })
                };
            }
            // A repository without the marker: somebody else's, or a seed that never finished.
            Some(1) => {
                return Err(PrepareRefusal::Occupied {
                    dir: self.dir.to_owned(),
                });
            }
            _ => {}
        }
        let listing = self
            .exec(
                "ls",
                vec![
                    "ls".to_owned(),
                    "-A".to_owned(),
                    "--".to_owned(),
                    self.dir.to_owned(),
                ],
                StdinFrom::Null,
                VERB_TIMEOUT,
            )
            .await?;
        if listing.ok() && listing.stdout_bytes > 0 {
            return Err(PrepareRefusal::Occupied {
                dir: self.dir.to_owned(),
            });
        }
        Ok(None)
    }

    /// Whether the clone holds `full_ref`: `rev-parse --verify --quiet` exits 0. Exit 1 is no;
    /// any other exit is [`PrepareRefusal::TargetGit`].
    async fn has_ref(&self, full_ref: &str) -> Result<bool, PrepareRefusal> {
        let out = self
            .exec(
                "rev-parse",
                vec![
                    "git".to_owned(),
                    format!("--git-dir={}/.git", self.dir),
                    "rev-parse".to_owned(),
                    "--verify".to_owned(),
                    "--quiet".to_owned(),
                    format!("{full_ref}^{{commit}}"),
                ],
                StdinFrom::Null,
                VERB_TIMEOUT,
            )
            .await?;
        match out.code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(PrepareRefusal::TargetGit {
                verb: "rev-parse",
                dir: self.dir.to_owned(),
                stderr: last_stderr_line(&out),
            }),
        }
    }
}
