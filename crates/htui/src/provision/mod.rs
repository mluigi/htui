//! `htui provision <destination>` (MOD-45): installs this htui build as the `htui-worker` system
//! service on a Linux host over the user's own `ssh`, with the DSN encrypted on that host by
//! `systemd-creds` and nowhere else (`R-STO-1`, PRD D1–D3).
//!
//! Four short sessions: the preflight (read-only), prepare (unprivileged: the log directory and the
//! binary), install (privileged: the credential, the unit, the start) and verify (wait for the
//! service and its `box.toml`). Then, from this machine, the box is checked in Postgres against a
//! baseline taken once the install has returned, and its executor is set to `worker` (D313). Each
//! session is bounded ([`Timings`]), and remote text is sanitised before it is printed.
//!
//! Every remote script is a constant in [`script`]. Values reach a script only as quoted
//! positional arguments. The DSN and the sudo password travel only on the ssh child's stdin:
//! never in an `argv`, the environment, a log or a file on either side.
//!
//! A refusal (exit 2) is anything decided before the first remote write. A failure (exit 1) is
//! anything after it. [`remote::SshRemote`] is the only place `htui` spawns `ssh`.

pub mod plan;
pub mod preflight;
pub mod remote;
pub mod script;
pub mod secret_prompt;
pub mod verify;

use std::io::Write;
use std::time::Duration;

use htui_core::model::BoxId;
use htui_store::dsn::DsnHost;
use htui_store::{Dsn, DsnError, identity, secret};
use sha2::Digest as _;
use zeroize::Zeroizing;

use crate::cli::ProvisionArgs;
use plan::{Decision, LocalFacts, SudoMode};
use preflight::{Facts, FactsError};
use remote::{Remote, RemoteOutput};
use secret_prompt::{PasswordPrompt, PromptError};
use verify::Verifier;

/// How `htui provision` ends (D292): `main` maps it to the exit code.
#[derive(Debug)]
pub enum ProvisionExit {
    /// Exit 2: refused before the first remote write (local checks, preflight, an aborted prompt).
    Refused(String),
    /// Exit 1: failed after it (an upload that does not run, sudo refused inside INSTALL, a
    /// service that never came up, ssh dropping mid-way).
    Failed(String),
}

impl ProvisionExit {
    /// 2 for a refusal, 1 for a failure.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Refused(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl core::fmt::Display for ProvisionExit {
    /// The sentence, and nothing else.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused(sentence) | Self::Failed(sentence) => f.write_str(sentence),
        }
    }
}

impl std::error::Error for ProvisionExit {}

/// The local build being shipped (D293). `Debug` prints its length, arch and hash, never the bytes.
#[derive(Clone)]
pub struct Payload {
    bytes: Vec<u8>,
    arch: String,
    sha: String,
}

impl core::fmt::Debug for Payload {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Payload")
            .field("len", &self.bytes.len())
            .field("arch", &self.arch)
            .field("sha", &self.sha)
            .finish()
    }
}

impl Payload {
    /// `bytes` with their sha256 computed here (tests build a stub binary this way).
    #[must_use]
    pub fn new(bytes: Vec<u8>, arch: &str) -> Self {
        let digest = sha2::Sha256::digest(&bytes);
        let sha = format!("{digest:x}");
        Self {
            bytes,
            arch: arch.to_owned(),
            sha,
        }
    }

    /// This process's own binary, `/proc/self/exe` (E-20), and `std::env::consts::ARCH`.
    ///
    /// # Errors
    ///
    /// The read's error.
    pub fn this_binary() -> std::io::Result<Self> {
        let bytes = std::fs::read("/proc/self/exe")?;
        Ok(Self::new(bytes, std::env::consts::ARCH))
    }

    /// The bytes PREPARE streams.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// `std::env::consts::ARCH` at build time, or the test's value.
    #[must_use]
    pub fn arch(&self) -> &str {
        &self.arch
    }

    /// Lowercase hex sha256 of [`Payload::bytes`].
    #[must_use]
    pub fn sha(&self) -> &str {
        &self.sha
    }
}

/// The waits (D305; E-10) and the bound on each ssh session (review finding 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timings {
    /// VERIFY's checks inside its one session.
    pub verify_tries: u32,
    /// Seconds between them.
    pub verify_pause_secs: u32,
    /// The local `box_seen` poll.
    pub poll: verify::Poll,
    /// The preflight session's bound; past it the run is refused (nothing was written).
    pub preflight_limit: Duration,
    /// PREPARE's bound without an upload.
    pub prepare_limit: Duration,
    /// Added to [`Timings::prepare_limit`] for each MiB uploaded.
    pub upload_per_mib: Duration,
    /// INSTALL's bound.
    pub install_limit: Duration,
    /// VERIFY's bound beyond its own `verify_tries` × `verify_pause_secs`.
    pub verify_slack: Duration,
}

impl Timings {
    /// 30 checks 2 s apart (58 s of waiting); a poll every 2 s for one box heartbeat plus 30 s
    /// (90 s), because the baseline is taken after INSTALL and a fast new worker may already be in
    /// it (review finding 3). Sessions: 120 s for the preflight, 120 s plus 2 s per uploaded MiB
    /// for PREPARE, 300 s for INSTALL, VERIFY's own wait plus 120 s.
    pub const PRODUCTION: Self = Self {
        verify_tries: 30,
        verify_pause_secs: 2,
        poll: verify::Poll {
            interval: Duration::from_secs(2),
            deadline: htui_store::connect::BOX_HEARTBEAT.saturating_add(Duration::from_secs(30)),
        },
        preflight_limit: Duration::from_secs(120),
        prepare_limit: Duration::from_secs(120),
        upload_per_mib: Duration::from_secs(2),
        install_limit: Duration::from_secs(300),
        verify_slack: Duration::from_secs(120),
    };

    /// How long VERIFY waits at most, in seconds: the pauses between its checks (review finding
    /// 13), `(tries - 1) × pause`.
    #[must_use]
    pub fn verify_wait_secs(&self) -> u64 {
        u64::from(self.verify_tries.saturating_sub(1)) * u64::from(self.verify_pause_secs)
    }

    /// VERIFY's session bound: its own wait plus [`Timings::verify_slack`].
    #[must_use]
    pub fn verify_limit(&self) -> Duration {
        let wait = u64::from(self.verify_tries) * u64::from(self.verify_pause_secs);
        Duration::from_secs(wait).saturating_add(self.verify_slack)
    }
}

/// Everything `run_with` needs (D309). `Debug` redacts the DSN and summarises the payload.
pub struct Ctx<'a, R> {
    /// The ssh destination, as typed.
    pub destination: &'a str,
    /// The root prefix every script gets as `$1`: `""` in production (D309).
    pub root: &'a str,
    /// `--replace-credential`.
    pub replace_credential: bool,
    /// The transport.
    pub remote: &'a R,
    /// The binary to ship.
    pub payload: Payload,
    /// The DSN to encrypt on the host; the only copy `run_with` holds.
    pub dsn: Zeroizing<String>,
    /// Reads the sudo password with no echo (D302).
    pub prompter: &'a dyn PasswordPrompt,
    /// Baseline, `box_seen`, `set_executor` (D305, D313).
    pub verifier: &'a dyn Verifier,
    /// See [`Timings`].
    pub timings: Timings,
    /// The success line.
    pub out: &'a mut dyn Write,
    /// Progress, warnings, prompts, the hint.
    pub err: &'a mut dyn Write,
}

impl<R> core::fmt::Debug for Ctx<'_, R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ctx")
            .field("destination", &self.destination)
            .field("root", &self.root)
            .field("replace_credential", &self.replace_credential)
            .field("payload", &self.payload)
            .field("dsn", &"<redacted>")
            .field("prompter", &self.prompter)
            .field("verifier", &self.verifier)
            .field("timings", &self.timings)
            .finish_non_exhaustive()
    }
}

/// What a successful run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// D300 step 3: nothing was installed. The box was still looked up and its executor set when
    /// it could be (E-25).
    AlreadyProvisioned,
    /// Installed and started.
    Provisioned {
        /// From the remote `box.toml`.
        box_id: BoxId,
        /// From the remote `box.toml`.
        hostname: String,
        /// `box_seen` answered within the poll.
        verified: bool,
        /// `set_executor` wrote (or found) `worker`.
        executor_set: bool,
    },
}

/// `htui provision`: builds the production [`Ctx`] and runs [`run_with`] (D309; E-12).
///
/// # Errors
///
/// [`ProvisionExit`].
pub async fn run(args: ProvisionArgs) -> Result<(), ProvisionExit> {
    let dest = args.destination.as_str();
    if !cfg!(target_os = "linux") {
        return Err(refused(
            dest,
            "provisioning ships this htui binary, which is not a Linux build",
        ));
    }
    plan::validate_destination(dest).map_err(|sentence| refused(dest, &sentence))?;
    let dsn = read_dsn(args.dsn_stdin, dest)?;
    let payload = Payload::this_binary()
        .map_err(|err| refused(dest, &format!("this htui binary could not be read: {err}")))?;
    let root = identity::config_root().map_err(|err| refused(dest, &err.to_string()))?;
    let remote = remote::SshRemote::new(dest);
    let verifier = verify::PgVerifier::new(root);
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut out = stdout.lock();
    let mut err = stderr.lock();
    run_with(Ctx {
        destination: dest,
        root: "",
        replace_credential: args.replace_credential,
        remote: &remote,
        payload,
        dsn,
        prompter: &secret_prompt::TtyPrompt,
        verifier: &verifier,
        timings: Timings::PRODUCTION,
        out: &mut out,
        err: &mut err,
    })
    .await
    .map(|_| ())
}

/// The whole provisioning, over injected parts (D309).
///
/// # Errors
///
/// [`ProvisionExit`]: `Refused` before the first remote write, `Failed` after it.
pub async fn run_with<R: Remote>(ctx: Ctx<'_, R>) -> Result<Outcome, ProvisionExit> {
    let Ctx {
        destination: dest,
        root,
        replace_credential,
        remote,
        payload,
        dsn,
        prompter,
        verifier,
        timings,
        out,
        err,
    } = ctx;

    // 1–2: local checks, no remote call.
    plan::validate_destination(dest).map_err(|sentence| refused(dest, &sentence))?;
    check_dsn(dest, &dsn)?;

    // 3–4: the preflight.
    note(err, dest, "preflight");
    let answer = bounded(
        remote,
        &format!("the preflight on {dest}"),
        timings.preflight_limit,
        &script::remote_command(script::PREFLIGHT, &[root]),
        b"",
    )
    .await
    .map_err(|reason| refused(dest, &reason))?;
    let facts = match Facts::parse(&answer.stdout_text()) {
        Ok(facts) => facts,
        Err(FactsError::NoPreflight) if answer.code != Some(0) => {
            return Err(refused(
                dest,
                &with_detail(
                    &format!("ssh to {dest} failed (exit {})", exit_text(answer.code)),
                    &answer.last_stderr_line(),
                ),
            ));
        }
        Err(error) => {
            return Err(refused(
                dest,
                &with_detail(&error.to_string(), &answer.last_stderr_line()),
            ));
        }
    };

    // 5–6: the values that go into the unit, then D300.
    plan::validate_account("user", &facts.user)
        .and_then(|()| plan::validate_account("group", &facts.group))
        .and_then(|()| plan::validate_home(&facts.home))
        .map_err(|sentence| refused(dest, &sentence))?;
    let local = LocalFacts {
        arch: payload.arch().to_owned(),
        sha: payload.sha().to_owned(),
    };
    let (upload, sudo) = match plan::decide(&facts, &local, replace_credential) {
        Decision::Refuse(sentence) => return Err(refused(dest, &sentence)),
        Decision::AlreadyProvisioned => {
            let finish = Finish {
                dest,
                root,
                remote,
                verifier,
                timings,
                out,
                err,
            };
            return Ok(finish.already_provisioned(&facts, &dsn).await);
        }
        Decision::Steps { upload, sudo } => (upload, sudo),
    };

    // 7: the sudo password, before anything is written. The prompt goes where the answer is read
    // from, the terminal, when the prompter has one (review finding 10).
    let password = match sudo {
        SudoMode::NoPassword => None,
        SudoMode::Password => {
            let mut terminal = prompter.prompt_writer();
            let shown: &mut dyn Write = match terminal.as_mut() {
                Some(terminal) => terminal,
                None => &mut *err,
            };
            let _ = write!(shown, "[sudo] password for {} on {dest}: ", facts.user);
            let _ = shown.flush();
            let answer = prompter.ask().await;
            let _ = writeln!(shown);
            let _ = shown.flush();
            drop(terminal);
            match answer {
                Ok(password) => Some(password),
                Err(PromptError::Aborted) => {
                    return Err(refused(
                        dest,
                        "the sudo password prompt was cancelled; nothing was written",
                    ));
                }
                Err(PromptError::NoTerminal) => {
                    return Err(refused(
                        dest,
                        "sudo needs a password and there is no terminal; configure NOPASSWD or \
                         run interactively",
                    ));
                }
            }
        }
    };

    // 8: PREPARE, the first remote write. From here every exit is a failure.
    let mib = payload.bytes().len().div_ceil(1024 * 1024);
    if upload {
        note(err, dest, &format!("uploading htui ({mib} MiB)"));
    }
    let send = if upload { "1" } else { "0" };
    let prepare_limit = if upload {
        let per = timings
            .upload_per_mib
            .saturating_mul(u32::try_from(mib).unwrap_or(u32::MAX));
        timings.prepare_limit.saturating_add(per)
    } else {
        timings.prepare_limit
    };
    let prepared = bounded(
        remote,
        &format!("preparing {dest}"),
        prepare_limit,
        &script::remote_command(script::PREPARE, &[root, &facts.home, send, payload.sha()]),
        if upload { payload.bytes() } else { b"" },
    )
    .await
    .map_err(|reason| failed(dest, &reason))?;
    match prepared.code {
        Some(0) => {}
        Some(3) => {
            return Err(failed(
                dest,
                &format!(
                    "the htui binary does not run on {dest}: {}",
                    prepared.last_stderr_line()
                ),
            ));
        }
        Some(6) => {
            return Err(failed(
                dest,
                &format!(
                    "the upload to {dest} was corrupted (its sha256 is not this build's); \
                     nothing was replaced"
                ),
            ));
        }
        code => {
            return Err(failed(
                dest,
                &with_detail(
                    &format!("preparing {dest} failed (exit {})", exit_text(code)),
                    &prepared.stderr_tail(5),
                ),
            ));
        }
    }

    // 9: INSTALL, under sudo.
    note(err, dest, "installing the service");
    let mode = match sudo {
        SudoMode::Password => "password",
        SudoMode::NoPassword => "nopasswd",
    };
    let replace = if replace_credential { "1" } else { "0" };
    let stdin = install_stdin(password.as_ref().map(|p| p.as_str()), &dsn);
    drop(password);
    let installed = bounded(
        remote,
        &format!("installing the service on {dest}"),
        timings.install_limit,
        &script::remote_command(
            script::INSTALL,
            &[
                root,
                &facts.user,
                &facts.home,
                replace,
                mode,
                script::INSTALL_ROOT,
            ],
        ),
        &stdin,
    )
    .await;
    drop(stdin);
    let installed = installed.map_err(|reason| failed(dest, &reason))?;
    if installed.code != Some(0) {
        let root_started = installed
            .stdout_text()
            .lines()
            .any(|line| line.trim_end() == script::ROOT_MARKER);
        if !root_started && matches!(installed.code, Some(255) | None) {
            return Err(failed(
                dest,
                &with_detail(
                    &format!(
                        "the ssh session to {dest} dropped before the privileged part (exit {}); \
                         nothing privileged was written",
                        exit_text(installed.code)
                    ),
                    &installed.last_stderr_line(),
                ),
            ));
        }
        if installed.code == Some(4) || !root_started {
            let refusal = match sudo {
                SudoMode::Password => {
                    "sudo refused the password (or needs a tty or a second factor)"
                }
                SudoMode::NoPassword => {
                    "sudo refused to run the installer as root (or needs a tty)"
                }
            };
            let said = installed.last_stderr_line();
            let said = if said.is_empty() {
                String::new()
            } else {
                format!(" ({said})")
            };
            return Err(failed(
                dest,
                &format!("{refusal} on {dest}{said}; nothing privileged was written"),
            ));
        }
        return Err(failed(
            dest,
            &with_detail(
                &format!(
                    "installing the service on {dest} failed (exit {})",
                    exit_text(installed.code)
                ),
                &installed.stderr_tail(5),
            ),
        ));
    }

    // 10: the baseline, once INSTALL has started or restarted the service, so no heartbeat of an
    // old worker counts (review finding 3). It never fails the run; its reason is kept for 12.
    let baseline = verifier.baseline(&dsn).await;

    // 11: VERIFY, inside one session.
    note(err, dest, "waiting for the worker");
    let checked = verify_session(remote, dest, root, &facts.home, timings)
        .await
        .map_err(|reason| failed(dest, &reason))?;
    match checked.code {
        Some(0) => {}
        Some(5) => {
            let stdout = checked.stdout_text();
            let state = stdout
                .lines()
                .find_map(|line| line.trim_end().strip_prefix("htui.active="))
                .unwrap_or("unknown");
            let stderr = String::from_utf8_lossy(&checked.stderr);
            return Err(failed(
                dest,
                &format!(
                    "the htui-worker service on {dest} did not start with a box.toml within {} s \
                     (systemctl says {state}); if the worker cannot use its DSN, fix it and \
                     re-run with --replace-credential; its last journal and log lines follow:\n{}",
                    timings.verify_wait_secs(),
                    stderr.trim_end()
                ),
            ));
        }
        code => {
            return Err(failed(
                dest,
                &with_detail(
                    &format!(
                        "waiting for the worker on {dest} failed (exit {})",
                        exit_text(code)
                    ),
                    &checked.stderr_tail(5),
                ),
            ));
        }
    }
    let identity = read_box(&checked).map_err(|reason| {
        failed(
            dest,
            &format!("the worker on {dest} wrote a box.toml htui cannot read: {reason}"),
        )
    })?;
    let id = identity.box_id;

    // 12: Postgres, from here; a warning at worst.
    let (verified, executor_set) = match baseline {
        Ok(baseline) => match verifier.box_seen(id, &baseline, timings.poll).await {
            Ok(()) => {
                if baseline.contains_key(&id) {
                    // Review finding 3: no new row, only a later last_seen_at, which any htui on
                    // the host with the same box.toml (a TUI over ssh) also moves.
                    let _ = writeln!(
                        err,
                        "note: box {id} was registered before this install, so the check-in is \
                         best-effort: an htui TUI on {dest} with the same box.toml would also \
                         count"
                    );
                }
                (true, set_executor(verifier, id, dest, err).await.is_some())
            }
            Err(reason) => {
                not_verified(
                    err,
                    id,
                    &format!(
                        "{reason}; see journalctl -u htui-worker and \
                         ~/.local/state/htui/worker.log on {dest}"
                    ),
                );
                (false, false)
            }
        },
        Err(reason) => {
            not_verified(err, id, &reason);
            (false, false)
        }
    };

    // 13: the result, then the way in for an agent login (OQ-1 (a); E-26).
    let _ = writeln!(
        out,
        "box {id} ({}) provisioned on {dest}",
        remote::sanitise(&identity.hostname)
    );
    let _ = writeln!(
        err,
        "to log an agent in on {dest}: ssh -t {dest} '~/.local/bin/htui' --dsn-stdin, paste the \
         DSN, then Settings › Agents"
    );
    Ok(Outcome::Provisioned {
        box_id: id,
        hostname: identity.hostname,
        verified,
        executor_set,
    })
}

/// The parts the already-provisioned path still needs (E-25).
struct Finish<'a, 'w, R> {
    dest: &'a str,
    root: &'a str,
    remote: &'a R,
    verifier: &'a dyn Verifier,
    timings: Timings,
    out: &'w mut dyn Write,
    err: &'w mut dyn Write,
}

impl<R: Remote> Finish<'_, '_, R> {
    /// E-25: nothing is installed, but a run that failed while waiting for the worker leaves the
    /// service active and lands here, so the box is still read (VERIFY, unprivileged) and its
    /// executor set to `worker`. `box_seen` is not asked: the box may have been registered long
    /// ago. Every problem on this path is a warning, and the exit stays 0.
    async fn already_provisioned(self, facts: &Facts, dsn: &str) -> Outcome {
        let Self {
            dest,
            root,
            remote,
            verifier,
            timings,
            out,
            err,
        } = self;
        note(
            err,
            dest,
            "already provisioned with this build; reading its box",
        );
        let _ = writeln!(
            err,
            "(--replace-credential re-encrypts the DSN and restarts the service)"
        );
        let identity = match verify_session(remote, dest, root, &facts.home, timings).await {
            Err(reason) => Err(reason),
            Ok(checked) if checked.code == Some(0) => read_box(&checked),
            Ok(checked) => Err(with_detail(
                &format!("the check exited {}", exit_text(checked.code)),
                &checked.last_stderr_line(),
            )),
        };
        let identity = match identity {
            Ok(identity) => identity,
            Err(reason) => {
                let _ = writeln!(
                    err,
                    "warning: the box on {dest} could not be read: {}; if its executor is not \
                     worker yet, set it in Settings › Boxes",
                    remote::sanitise(&reason)
                );
                let _ = writeln!(
                    out,
                    "{dest} is already provisioned with this build; nothing was changed"
                );
                return Outcome::AlreadyProvisioned;
            }
        };
        let id = identity.box_id;
        let changed = match verifier.baseline(dsn).await {
            Ok(_) => set_executor(verifier, id, dest, err).await == Some(true),
            Err(reason) => {
                not_verified(err, id, &reason);
                false
            }
        };
        let suffix = if changed {
            "; executor set to worker"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "{dest} is already provisioned with this build; box {id} ({}){suffix}",
            remote::sanitise(&identity.hostname)
        );
        Outcome::AlreadyProvisioned
    }
}

/// VERIFY with `Ctx.timings` (E-10), bounded by [`Timings::verify_limit`].
async fn verify_session<R: Remote>(
    remote: &R,
    dest: &str,
    root: &str,
    home: &str,
    timings: Timings,
) -> Result<RemoteOutput, String> {
    let tries = timings.verify_tries.to_string();
    let pause = timings.verify_pause_secs.to_string();
    bounded(
        remote,
        &format!("waiting for the worker on {dest}"),
        timings.verify_limit(),
        &script::remote_command(script::VERIFY, &[root, home, &tries, &pause]),
        b"",
    )
    .await
}

/// One session, given at most `limit` (review finding 2). On the timeout the session's future is
/// dropped, and with it the local `ssh`, which `run_piped`'s `kill_on_drop` kills. `Err` is the
/// sentence: `cannot run ssh: …`, or `<what> did not finish within …`.
async fn bounded<R: Remote>(
    remote: &R,
    what: &str,
    limit: Duration,
    command: &str,
    stdin: &[u8],
) -> Result<RemoteOutput, String> {
    match tokio::time::timeout(limit, remote.run(command, stdin)).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(format!("cannot run ssh: {e}")),
        Err(_) => Err(format!(
            "{what} did not finish within {}; its ssh session was closed",
            duration_text(limit)
        )),
    }
}

/// `120 s`, or `50 ms` under a second.
fn duration_text(limit: Duration) -> String {
    if limit.as_secs() > 0 {
        format!("{} s", limit.as_secs())
    } else {
        format!("{} ms", limit.as_millis())
    }
}

/// The `box.toml` VERIFY printed between its markers.
fn read_box(checked: &RemoteOutput) -> Result<identity::Identity, String> {
    let stdout = checked.stdout_text();
    let text = box_text(&stdout).ok_or_else(|| "no box.toml in its answer".to_owned())?;
    identity::parse_box_toml(&text).map_err(|err| err.to_string())
}

/// D313 through the verifier: `Some(wrote)` on success, after a progress line that says whether it
/// wrote or found `worker`; `None` after the warning.
async fn set_executor(
    verifier: &dyn Verifier,
    id: BoxId,
    dest: &str,
    err: &mut dyn Write,
) -> Option<bool> {
    match verifier.set_executor(id).await {
        Ok(wrote) => {
            let what = if wrote {
                "executor set to worker"
            } else {
                "executor already worker"
            };
            note(err, dest, what);
            Some(wrote)
        }
        Err(reason) => {
            let _ = writeln!(
                err,
                "warning: box {id}'s executor is not set to worker: {reason}; set it in \
                 Settings › Boxes"
            );
            None
        }
    }
}

/// D305's warning, and D313's hint to set the executor by hand.
fn not_verified(err: &mut dyn Write, id: BoxId, reason: &str) {
    let _ = writeln!(
        err,
        "warning: service active, box {id}; not verified in Postgres from here: {reason}"
    );
    let _ = writeln!(err, "set box {id}'s executor to worker in Settings › Boxes");
}

/// Step 2: the DSN must name a host the remote host can reach (D306).
fn check_dsn(dest: &str, dsn: &str) -> Result<(), ProvisionExit> {
    let parsed = Dsn::parse(dsn).map_err(|error| match error {
        DsnError::NoHost => refused(
            dest,
            &format!("the DSN names no host; pass one {dest} can reach with --dsn-stdin"),
        ),
        other => refused(dest, &format!("the DSN cannot be used: {other}")),
    })?;
    match parsed.host_class() {
        DsnHost::Remote => Ok(()),
        DsnHost::Loopback => Err(refused(
            dest,
            &format!(
                "the DSN's host is loopback, which would mean {dest} itself; pass the address \
                 {dest} reaches Postgres at with --dsn-stdin"
            ),
        )),
        DsnHost::Socket => Err(refused(
            dest,
            &format!(
                "the DSN names a Unix socket, which would be a socket on {dest}; pass a TCP \
                 address with --dsn-stdin"
            ),
        )),
    }
}

/// INSTALL's stdin: the password line (when sudo wants one), then the DSN line. Sized up front, so
/// no reallocation leaves a copy behind.
fn install_stdin(password: Option<&str>, dsn: &str) -> Zeroizing<Vec<u8>> {
    let size = password.map_or(0, |p| p.len() + 1) + dsn.len() + 1;
    let mut stdin = Zeroizing::new(Vec::with_capacity(size));
    if let Some(password) = password {
        stdin.extend_from_slice(password.as_bytes());
        stdin.push(b'\n');
    }
    stdin.extend_from_slice(dsn.as_bytes());
    stdin.push(b'\n');
    stdin
}

/// The lines between [`script::BOX_BEGIN`] and [`script::BOX_END`], `\r` stripped.
fn box_text(stdout: &str) -> Option<String> {
    let mut lines = stdout.lines().map(|line| line.trim_end_matches('\r'));
    lines.find(|line| *line == script::BOX_BEGIN)?;
    let mut text = String::new();
    for line in lines {
        if line == script::BOX_END {
            return Some(text);
        }
        text.push_str(line);
        text.push('\n');
    }
    None
}

/// The DSN `run` ships: `--dsn-stdin`'s line, or the keyring's, wrapped at once.
fn read_dsn(dsn_stdin: bool, dest: &str) -> Result<Zeroizing<String>, ProvisionExit> {
    if dsn_stdin {
        let stdin = std::io::stdin();
        if std::io::IsTerminal::is_terminal(&stdin) {
            eprintln!("paste the DSN {dest} should use and press Enter (it will be visible):");
        }
        return match secret::read_dsn_line(&mut stdin.lock()) {
            Ok(Some(dsn)) => Ok(dsn),
            Ok(None) => Err(refused(dest, "no DSN on stdin; nothing was sent")),
            Err(err) => Err(refused(
                dest,
                &format!("the DSN could not be read from stdin: {err}"),
            )),
        };
    }
    match secret::get_dsn().map(|dsn| dsn.map(Zeroizing::new)) {
        Ok(Some(dsn)) => Ok(dsn),
        Ok(None) => Err(refused(
            dest,
            "no Postgres DSN in the OS keyring; run `htui --set-dsn` or pass --dsn-stdin",
        )),
        Err(err) => Err(refused(
            dest,
            &format!("the OS keyring could not be read ({err}); pass the DSN with --dsn-stdin"),
        )),
    }
}

/// `not provisioning {dest}: {sentence}`, sanitised: it may quote the remote host (review finding
/// 9).
fn refused(dest: &str, sentence: &str) -> ProvisionExit {
    ProvisionExit::Refused(remote::sanitise(&format!(
        "not provisioning {dest}: {sentence}"
    )))
}

/// `provisioning {dest} failed: {sentence}`, sanitised like [`refused`].
fn failed(dest: &str, sentence: &str) -> ProvisionExit {
    ProvisionExit::Failed(remote::sanitise(&format!(
        "provisioning {dest} failed: {sentence}"
    )))
}

/// A progress line (D307); a write error is ignored.
fn note(err: &mut dyn Write, dest: &str, what: &str) {
    let _ = writeln!(err, "provisioning {dest}: {what}");
}

/// `sentence: detail`, or the sentence alone when there is no detail; the detail, remote text, is
/// sanitised (review finding 9).
fn with_detail(sentence: &str, detail: &str) -> String {
    if detail.is_empty() {
        sentence.to_owned()
    } else {
        format!("{sentence}: {}", remote::sanitise(detail))
    }
}

/// An exit code as a sentence shows it; a signal has none.
fn exit_text(code: Option<i32>) -> String {
    code.map_or_else(|| "by a signal".to_owned(), |code| code.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use futures::future::BoxFuture;

    use super::*;
    use verify::{Baseline, Poll};

    const DSN: &str = "postgres://u:SENTINEL-DSN-PW@db.example:5432/htui";
    const PASSWORD: &str = "SENTINEL-SUDO-PW";
    const DEST: &str = "alice@box1";
    const BOX: &str = "0190a5c4-1c3e-7000-8000-00000000b0c5";
    const HINT: &str = "to log an agent in on alice@box1: ssh -t alice@box1 '~/.local/bin/htui' \
                        --dsn-stdin, paste the DSN, then Settings › Agents\n";
    const TIMINGS: Timings = Timings {
        verify_tries: 1,
        verify_pause_secs: 0,
        poll: Poll {
            interval: Duration::from_millis(10),
            deadline: Duration::from_millis(50),
        },
        preflight_limit: Duration::from_secs(10),
        prepare_limit: Duration::from_secs(10),
        upload_per_mib: Duration::from_secs(1),
        install_limit: Duration::from_secs(10),
        verify_slack: Duration::from_secs(10),
    };

    /// Every bound 50 ms, for the sessions that never end.
    const SHORT: Timings = Timings {
        preflight_limit: Duration::from_millis(50),
        prepare_limit: Duration::from_millis(50),
        upload_per_mib: Duration::ZERO,
        install_limit: Duration::from_millis(50),
        verify_slack: Duration::from_millis(50),
        ..TIMINGS
    };

    /// Answers each session from a script and records what it was asked.
    #[derive(Debug, Default)]
    struct ScriptedRemote {
        answers: Mutex<VecDeque<io::Result<RemoteOutput>>>,
        calls: Mutex<Vec<(String, Vec<u8>)>>,
        /// The session (by index) that never ends.
        hang_at: Option<usize>,
        /// Shared with a [`FakeVerifier`] to see the two in one order.
        timeline: Arc<Mutex<Vec<String>>>,
    }

    impl ScriptedRemote {
        fn new(answers: Vec<io::Result<RemoteOutput>>) -> Self {
            Self {
                answers: Mutex::new(answers.into()),
                ..Self::default()
            }
        }

        fn calls(&self) -> Vec<(String, Vec<u8>)> {
            self.calls.lock().expect("calls").clone()
        }

        fn sessions(&self) -> Vec<&'static str> {
            self.calls()
                .iter()
                .map(|(command, _)| session(command))
                .collect()
        }

        /// The stdin of the first call to `name`.
        fn stdin_of(&self, name: &str) -> Vec<u8> {
            self.calls()
                .into_iter()
                .find(|(command, _)| session(command) == name)
                .map(|(_, stdin)| stdin)
                .expect("the session ran")
        }

        fn command_of(&self, name: &str) -> String {
            self.calls()
                .into_iter()
                .find(|(command, _)| session(command) == name)
                .map(|(command, _)| command)
                .expect("the session ran")
        }
    }

    impl Remote for ScriptedRemote {
        async fn run(&self, command: &str, stdin: &[u8]) -> io::Result<RemoteOutput> {
            let index = {
                let mut calls = self.calls.lock().expect("calls");
                calls.push((command.to_owned(), stdin.to_vec()));
                calls.len() - 1
            };
            self.timeline
                .lock()
                .expect("timeline")
                .push(session(command).to_owned());
            if self.hang_at == Some(index) {
                std::future::pending::<()>().await;
            }
            self.answers
                .lock()
                .expect("answers")
                .pop_front()
                .unwrap_or_else(|| Err(io::Error::other("no scripted answer")))
        }
    }

    /// Which script a command runs.
    fn session(command: &str) -> &'static str {
        [
            ("PREFLIGHT", script::PREFLIGHT),
            ("PREPARE", script::PREPARE),
            ("INSTALL", script::INSTALL),
            ("VERIFY", script::VERIFY),
        ]
        .into_iter()
        .find(|(_, text)| command.starts_with(&format!("sh -c '{text}'")))
        .map_or("?", |(name, _)| name)
    }

    #[derive(Debug)]
    struct FakeVerifier {
        baseline: Result<Baseline, String>,
        seen: Result<(), String>,
        executor: Result<bool, String>,
        log: Mutex<Vec<String>>,
        /// See [`ScriptedRemote::timeline`].
        timeline: Arc<Mutex<Vec<String>>>,
    }

    impl FakeVerifier {
        fn new() -> Self {
            Self {
                baseline: Ok(Baseline::new()),
                seen: Ok(()),
                executor: Ok(true),
                log: Mutex::default(),
                timeline: Arc::default(),
            }
        }

        fn log(&self) -> Vec<String> {
            self.log.lock().expect("log").clone()
        }

        fn note(&self, call: String) {
            self.timeline.lock().expect("timeline").push(call.clone());
            self.log.lock().expect("log").push(call);
        }
    }

    impl Verifier for FakeVerifier {
        fn baseline<'a>(&'a self, dsn: &'a str) -> BoxFuture<'a, Result<Baseline, String>> {
            assert_eq!(dsn, DSN, "the baseline connects with the DSN being shipped");
            self.note("baseline".to_owned());
            Box::pin(futures::future::ready(self.baseline.clone()))
        }

        fn box_seen<'a>(
            &'a self,
            id: BoxId,
            _baseline: &'a Baseline,
            _poll: Poll,
        ) -> BoxFuture<'a, Result<(), String>> {
            self.note(format!("box_seen({id})"));
            Box::pin(futures::future::ready(self.seen.clone()))
        }

        fn set_executor(&self, id: BoxId) -> BoxFuture<'_, Result<bool, String>> {
            self.note(format!("set_executor({id})"));
            Box::pin(futures::future::ready(self.executor.clone()))
        }
    }

    /// A `Write` into a shared buffer: the fake terminal of [`FakePrompt::terminal`].
    #[derive(Debug, Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("buffer").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Shared {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().expect("buffer").clone()).expect("utf-8")
        }
    }

    #[derive(Debug)]
    struct FakePrompt {
        answer: Result<&'static str, PromptError>,
        count: AtomicUsize,
        /// `Some`: the prompt's own terminal (review finding 10).
        terminal: Option<Shared>,
    }

    impl FakePrompt {
        fn new(answer: Result<&'static str, PromptError>) -> Self {
            Self {
                answer,
                count: AtomicUsize::new(0),
                terminal: None,
            }
        }

        fn count(&self) -> usize {
            self.count.load(Ordering::SeqCst)
        }
    }

    impl PasswordPrompt for FakePrompt {
        fn ask(&self) -> BoxFuture<'_, Result<Zeroizing<String>, PromptError>> {
            self.count.fetch_add(1, Ordering::SeqCst);
            let answer = self.answer.map(|p| Zeroizing::new(p.to_owned()));
            Box::pin(futures::future::ready(answer))
        }

        fn prompt_writer(&self) -> Option<Box<dyn Write + Send>> {
            self.terminal
                .clone()
                .map(|t| Box::new(t) as Box<dyn Write + Send>)
        }
    }

    fn payload() -> Payload {
        Payload::new(b"#!/bin/sh\necho htui 0.1.0\n".to_vec(), "x86_64")
    }

    /// A fresh password host, with `overrides` replacing keys.
    fn preflight(overrides: &[(&str, &str)]) -> io::Result<RemoteOutput> {
        let defaults = [
            ("os", "Linux"),
            ("arch", "x86_64"),
            ("systemd", "255"),
            ("creds", "yes"),
            ("user", "alice"),
            ("group", "alice"),
            ("home", "/home/alice"),
            ("bin_sha", "none"),
            ("unit", "no"),
            ("active", "inactive"),
            ("sudo", "password"),
            ("printf", "builtin"),
        ];
        let mut text = "Welcome to box1\n".to_owned();
        for (key, value) in defaults {
            let value = overrides
                .iter()
                .find(|(k, _)| *k == key)
                .map_or(value, |(_, v)| v);
            text.push_str(&format!("htui.{key}={value}\n"));
        }
        ok(&text)
    }

    fn ok(stdout: &str) -> io::Result<RemoteOutput> {
        exit(0, stdout, "")
    }

    fn exit(code: i32, stdout: &str, stderr: &str) -> io::Result<RemoteOutput> {
        Ok(RemoteOutput {
            code: Some(code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        })
    }

    fn verified() -> io::Result<RemoteOutput> {
        ok(&format!(
            "htui.active=active\n{}\nbox_id = \"{BOX}\"\nhostname = \"box1\"\n\n{}\n",
            script::BOX_BEGIN,
            script::BOX_END
        ))
    }

    fn box_id() -> BoxId {
        BoxId::from_uuid(uuid::Uuid::parse_str(BOX).expect("a uuid"))
    }

    struct Ran {
        result: Result<Outcome, ProvisionExit>,
        out: String,
        err: String,
    }

    async fn drive(
        dest: &str,
        dsn: &str,
        replace_credential: bool,
        remote: &ScriptedRemote,
        prompter: &FakePrompt,
        verifier: &FakeVerifier,
    ) -> Ran {
        drive_with(
            TIMINGS,
            dest,
            dsn,
            replace_credential,
            remote,
            prompter,
            verifier,
        )
        .await
    }

    async fn drive_with(
        timings: Timings,
        dest: &str,
        dsn: &str,
        replace_credential: bool,
        remote: &ScriptedRemote,
        prompter: &FakePrompt,
        verifier: &FakeVerifier,
    ) -> Ran {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let result = run_with(Ctx {
            destination: dest,
            root: "",
            replace_credential,
            remote,
            payload: payload(),
            dsn: Zeroizing::new(dsn.to_owned()),
            prompter,
            verifier,
            timings,
            out: &mut out,
            err: &mut err,
        })
        .await;
        let ran = Ran {
            result,
            out: String::from_utf8(out).expect("utf-8"),
            err: String::from_utf8(err).expect("utf-8"),
        };
        // House rule: no sentinel reaches an output, a command line or an exit sentence.
        assert!(!ran.out.contains("SENTINEL"), "stdout holds a sentinel");
        assert!(!ran.err.contains("SENTINEL"), "stderr holds a sentinel");
        for (command, _) in remote.calls() {
            assert!(!command.contains("SENTINEL"), "a command holds a sentinel");
        }
        if let Err(exit) = &ran.result {
            assert!(
                !exit.to_string().contains("SENTINEL"),
                "the exit sentence holds a sentinel"
            );
        }
        ran
    }

    /// The fresh password host every failure case starts from, with nopasswd unless asked.
    fn fresh(nopasswd: bool, rest: Vec<io::Result<RemoteOutput>>) -> ScriptedRemote {
        let sudo = if nopasswd { "nopasswd" } else { "password" };
        let mut answers = vec![preflight(&[("sudo", sudo)])];
        answers.extend(rest);
        ScriptedRemote::new(answers)
    }

    fn expect_exit(ran: &Ran, code: u8) -> String {
        match &ran.result {
            Err(exit) => {
                assert_eq!(exit.code(), code, "{exit}");
                exit.to_string()
            }
            Ok(outcome) => panic!("expected exit {code}, got {outcome:?}"),
        }
    }

    #[test]
    fn provision_exit_codes() {
        let refused = ProvisionExit::Refused("no".to_owned());
        let failed = ProvisionExit::Failed("lost".to_owned());
        assert_eq!(refused.code(), 2);
        assert_eq!(failed.code(), 1);
        assert_eq!(refused.to_string(), "no");
        assert_eq!(failed.to_string(), "lost");
    }

    #[test]
    fn payload_hashes_and_debug_prints_no_bytes() {
        let payload = Payload::new(b"abc".to_vec(), "x86_64");
        assert_eq!(
            payload.sha(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(payload.arch(), "x86_64");
        assert_eq!(payload.bytes(), b"abc");
        let shown = format!("{payload:?}");
        assert!(
            shown.contains("len: 3") && !shown.contains("97, 98"),
            "{shown}"
        );
    }

    #[test]
    fn ctx_debug_redacts_the_dsn() {
        let remote = ScriptedRemote::default();
        let verifier = FakeVerifier::new();
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let ctx = Ctx {
            destination: DEST,
            root: "",
            replace_credential: false,
            remote: &remote,
            payload: payload(),
            dsn: Zeroizing::new(DSN.to_owned()),
            prompter: &prompt,
            verifier: &verifier,
            timings: TIMINGS,
            out: &mut out,
            err: &mut err,
        };
        let shown = format!("{ctx:?}");
        assert!(
            !shown.contains("SENTINEL-DSN-PW"),
            "Ctx's Debug shows the DSN"
        );
        assert!(shown.contains("<redacted>"));
    }

    #[test]
    fn install_stdin_and_box_text() {
        assert_eq!(
            install_stdin(Some("pw"), "dsn").as_slice(),
            b"pw\ndsn\n".as_slice()
        );
        assert_eq!(install_stdin(None, "dsn").as_slice(), b"dsn\n".as_slice());
        assert_eq!(
            box_text("noise\r\nhtui.box.begin\r\na = 1\r\nhtui.box.end\r\n").as_deref(),
            Some("a = 1\n")
        );
        assert_eq!(box_text("htui.box.begin\na = 1\n"), None, "no end marker");
        assert_eq!(box_text("a = 1\n"), None);
    }

    #[tokio::test]
    async fn a_fresh_password_host_runs_four_sessions_with_their_stdin() {
        let remote = ScriptedRemote::new(vec![
            preflight(&[]),
            ok(""),
            ok("htui.root=start\n"),
            verified(),
        ]);
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;

        assert_eq!(
            ran.result.expect("provisioned"),
            Outcome::Provisioned {
                box_id: box_id(),
                hostname: "box1".to_owned(),
                verified: true,
                executor_set: true,
            }
        );
        assert_eq!(
            remote.sessions(),
            ["PREFLIGHT", "PREPARE", "INSTALL", "VERIFY"]
        );
        let stdins: Vec<Vec<u8>> = remote.calls().into_iter().map(|(_, s)| s).collect();
        assert_eq!(
            stdins,
            [
                Vec::new(),
                payload().bytes().to_vec(),
                format!("{PASSWORD}\n{DSN}\n").into_bytes(),
                Vec::new(),
            ]
        );
        assert!(
            remote
                .command_of("PREPARE")
                .ends_with(&format!(" /home/alice 1 {}", payload().sha()))
        );
        let install = remote.command_of("INSTALL");
        assert!(install.contains(" password '"), "{install}");
        assert!(install.contains(script::INSTALL_ROOT));
        assert_eq!(ran.out, format!("box {BOX} (box1) provisioned on {DEST}\n"));
        assert_eq!(
            verifier.log(),
            [
                "baseline".to_owned(),
                format!("box_seen({BOX})"),
                format!("set_executor({BOX})"),
            ]
        );
        assert_eq!(prompt.count(), 1);
        assert!(
            ran.err
                .contains("[sudo] password for alice on alice@box1: \n")
        );
        assert!(
            ran.err
                .contains("provisioning alice@box1: uploading htui (1 MiB)\n")
        );
        assert!(
            ran.err
                .contains("provisioning alice@box1: executor set to worker\n")
        );
        assert!(ran.err.ends_with(HINT), "{}", ran.err);
    }

    #[tokio::test]
    async fn a_nopasswd_host_sends_only_the_dsn_and_never_prompts() {
        let remote = fresh(true, vec![ok(""), ok("htui.root=start\n"), verified()]);
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert!(ran.result.is_ok());
        assert_eq!(remote.stdin_of("INSTALL"), format!("{DSN}\n").into_bytes());
        assert!(remote.command_of("INSTALL").contains(" nopasswd '"));
        assert_eq!(prompt.count(), 0);
        assert!(!ran.err.contains("[sudo]"));
    }

    #[tokio::test]
    async fn matching_hashes_skip_the_upload() {
        let sha = payload().sha().to_owned();
        let remote = ScriptedRemote::new(vec![
            preflight(&[("bin_sha", &sha), ("sudo", "nopasswd")]),
            ok(""),
            ok("htui.root=start\n"),
            verified(),
        ]);
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert!(ran.result.is_ok());
        assert!(remote.stdin_of("PREPARE").is_empty());
        assert!(
            remote
                .command_of("PREPARE")
                .ends_with(&format!(" /home/alice 0 {sha}"))
        );
        assert!(!ran.err.contains("uploading"));
    }

    /// E-25: an already-provisioned host gets no PREPARE and no INSTALL, but its box is read and
    /// its executor set.
    #[tokio::test]
    async fn already_provisioned_runs_only_the_preflight_and_verify() {
        let sha = payload().sha().to_owned();
        let provisioned = || {
            preflight(&[
                ("bin_sha", sha.as_str()),
                ("unit", "yes"),
                ("active", "active"),
            ])
        };
        let remote = ScriptedRemote::new(vec![provisioned(), verified()]);
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert_eq!(ran.result.expect("ok"), Outcome::AlreadyProvisioned);
        assert_eq!(remote.sessions(), ["PREFLIGHT", "VERIFY"]);
        assert!(remote.calls().iter().all(|(_, stdin)| stdin.is_empty()));
        assert_eq!(
            verifier.log(),
            ["baseline".to_owned(), format!("set_executor({BOX})")]
        );
        assert_eq!(prompt.count(), 0);
        assert_eq!(
            ran.out,
            format!(
                "{DEST} is already provisioned with this build; box {BOX} (box1); executor set \
                 to worker\n"
            )
        );
        // Review finding 14: the progress line, then the hint.
        let progress = ran
            .err
            .find("provisioning alice@box1: already provisioned with this build; reading its box")
            .expect("the progress line");
        let hint = ran
            .err
            .find("(--replace-credential re-encrypts the DSN and restarts the service)")
            .expect("the hint");
        assert!(progress < hint, "{}", ran.err);

        // Already `worker`: the same, without the suffix.
        let remote = ScriptedRemote::new(vec![provisioned(), verified()]);
        let verifier = FakeVerifier {
            executor: Ok(false),
            ..FakeVerifier::new()
        };
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert_eq!(ran.result.expect("ok"), Outcome::AlreadyProvisioned);
        assert_eq!(
            ran.out,
            format!("{DEST} is already provisioned with this build; box {BOX} (box1)\n")
        );
        assert!(
            ran.err
                .contains("provisioning alice@box1: executor already worker\n"),
            "{}",
            ran.err
        );
        assert!(
            !ran.err.contains("executor set to worker"),
            "stderr agrees with stdout that nothing changed"
        );
    }

    /// E-25: on the already-provisioned path a VERIFY or a local connect that fails is a warning.
    #[tokio::test]
    async fn already_provisioned_warns_when_its_box_cannot_be_checked() {
        let sha = payload().sha().to_owned();
        let provisioned = || {
            preflight(&[
                ("bin_sha", sha.as_str()),
                ("unit", "yes"),
                ("active", "active"),
            ])
        };
        let prompt = FakePrompt::new(Ok(PASSWORD));

        let remote = ScriptedRemote::new(vec![
            provisioned(),
            exit(5, "htui.active=activating\n", "a journal line\n"),
        ]);
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert_eq!(ran.result.expect("exit 0"), Outcome::AlreadyProvisioned);
        assert_eq!(
            ran.out,
            format!("{DEST} is already provisioned with this build; nothing was changed\n")
        );
        assert!(
            ran.err
                .contains("warning: the box on alice@box1 could not be read")
        );
        assert!(ran.err.contains("Settings › Boxes"));
        assert!(verifier.log().is_empty());

        let remote = ScriptedRemote::new(vec![provisioned(), verified()]);
        let verifier = FakeVerifier {
            baseline: Err("unreachable".to_owned()),
            ..FakeVerifier::new()
        };
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert_eq!(ran.result.expect("exit 0"), Outcome::AlreadyProvisioned);
        assert_eq!(
            ran.out,
            format!("{DEST} is already provisioned with this build; box {BOX} (box1)\n")
        );
        assert!(ran.err.contains(&format!(
            "warning: service active, box {BOX}; not verified in Postgres from here: unreachable\n"
        )));
        assert!(ran.err.contains(&format!(
            "set box {BOX}'s executor to worker in Settings › Boxes\n"
        )));
        assert_eq!(verifier.log(), ["baseline"]);

        let remote = ScriptedRemote::new(vec![provisioned(), verified()]);
        let verifier = FakeVerifier {
            executor: Err("box gone".to_owned()),
            ..FakeVerifier::new()
        };
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert_eq!(ran.result.expect("exit 0"), Outcome::AlreadyProvisioned);
        assert!(ran.err.contains(&format!(
            "warning: box {BOX}'s executor is not set to worker: box gone; set it in Settings › \
             Boxes\n"
        )));
        assert!(!ran.out.contains("executor set to worker"));
    }

    #[tokio::test]
    async fn local_refusals_make_no_remote_call() {
        let cases = [
            (
                "-oProxyCommand=x",
                DSN,
                "the ssh destination must not be empty",
            ),
            (
                DEST,
                "postgres://u:p@localhost/htui",
                "the DSN's host is loopback, which would mean alice@box1 itself",
            ),
            (
                DEST,
                "postgres://u:p@localhost/db?host=/run/postgresql",
                "the DSN names a Unix socket",
            ),
            (DEST, "postgres://u:p@/htui", "the DSN names no host"),
            (
                DEST,
                "mysql://u:p@db.example/htui",
                "the DSN cannot be used: ",
            ),
        ];
        for (dest, dsn, expected) in cases {
            let remote = ScriptedRemote::default();
            let prompt = FakePrompt::new(Ok(PASSWORD));
            let verifier = FakeVerifier::new();
            let mut out = Vec::new();
            let mut err = Vec::new();
            let result = run_with(Ctx {
                destination: dest,
                root: "",
                replace_credential: false,
                remote: &remote,
                payload: payload(),
                dsn: Zeroizing::new(dsn.to_owned()),
                prompter: &prompt,
                verifier: &verifier,
                timings: TIMINGS,
                out: &mut out,
                err: &mut err,
            })
            .await;
            let exit = result.expect_err(expected);
            assert_eq!(exit.code(), 2, "{exit}");
            assert!(
                exit.to_string()
                    .starts_with(&format!("not provisioning {dest}: ")),
                "{exit}"
            );
            assert!(exit.to_string().contains(expected), "{exit}");
            assert!(remote.calls().is_empty(), "{expected}");
            assert!(verifier.log().is_empty());
        }
    }

    #[tokio::test]
    async fn preflight_refusals_are_refusals() {
        let cases = [
            (
                ok("Welcome to box1\n"),
                "the remote login shell did not run the preflight; htui provision needs a POSIX \
                 login shell (sh, bash, zsh, ksh)",
            ),
            (
                exit(
                    255,
                    "",
                    "ssh: connect to host box1 port 22: Connection refused\n",
                ),
                "ssh to alice@box1 failed (exit 255): ssh: connect to host box1 port 22: \
                 Connection refused",
            ),
            (
                preflight(&[("arch", "aarch64")]),
                "the remote host is aarch64 and this htui build is x86_64",
            ),
            (
                preflight(&[("home", "/home/a b")]),
                "the remote home \"/home/a b\" is not a path",
            ),
            (
                preflight(&[("group", "Staff")]),
                "the remote group \"Staff\" is not a name",
            ),
        ];
        for (answer, expected) in cases {
            let remote = ScriptedRemote::new(vec![answer]);
            let prompt = FakePrompt::new(Ok(PASSWORD));
            let verifier = FakeVerifier::new();
            let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
            let sentence = expect_exit(&ran, 2);
            assert!(sentence.contains(expected), "{sentence}");
            assert_eq!(remote.calls().len(), 1);
            assert_eq!(prompt.count(), 0);
        }

        let remote = ScriptedRemote::new(vec![Err(io::Error::from(io::ErrorKind::NotFound))]);
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert!(expect_exit(&ran, 2).contains("cannot run ssh: "));
    }

    #[tokio::test]
    async fn an_aborted_prompt_is_a_refusal_before_any_write() {
        for (answer, expected) in [
            (
                PromptError::Aborted,
                "not provisioning alice@box1: the sudo password prompt was cancelled; nothing \
                 was written",
            ),
            (
                PromptError::NoTerminal,
                "not provisioning alice@box1: sudo needs a password and there is no terminal; \
                 configure NOPASSWD or run interactively",
            ),
        ] {
            let remote = fresh(false, Vec::new());
            let prompt = FakePrompt::new(Err(answer));
            let verifier = FakeVerifier::new();
            let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
            assert_eq!(expect_exit(&ran, 2), expected);
            assert_eq!(remote.calls().len(), 1, "the preflight only");
            assert_eq!(prompt.count(), 1);
            assert!(verifier.log().is_empty());
        }
    }

    #[tokio::test]
    async fn exit_codes_map_to_their_sentences() {
        let sudo_refused = "provisioning alice@box1 failed: sudo refused to run the installer as \
                            root (or needs a tty) on alice@box1; nothing privileged was written";
        let cases: Vec<(Vec<io::Result<RemoteOutput>>, String)> =
            vec![
            (
                vec![exit(
                    3,
                    "",
                    "a login banner\n./htui: /lib/x86_64-linux-gnu/libc.so.6: version \
                     `GLIBC_2.39' not found\n\n",
                )],
                "provisioning alice@box1 failed: the htui binary does not run on alice@box1: \
                 ./htui: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.39' not found"
                    .to_owned(),
            ),
            (
                vec![exit(6, "", "")],
                "provisioning alice@box1 failed: the upload to alice@box1 was corrupted (its \
                 sha256 is not this build's); nothing was replaced"
                    .to_owned(),
            ),
            (
                vec![exit(255, "", "Connection reset by peer\n")],
                "provisioning alice@box1 failed: preparing alice@box1 failed (exit 255): \
                 Connection reset by peer"
                    .to_owned(),
            ),
            (vec![ok(""), exit(4, "", "")], sudo_refused.to_owned()),
            (
                vec![ok(""), exit(1, "", "sudo: a password is required\n")],
                "provisioning alice@box1 failed: sudo refused to run the installer as root (or \
                 needs a tty) on alice@box1 (sudo: a password is required); nothing privileged \
                 was written"
                    .to_owned(),
            ),
            (
                vec![ok(""), exit(255, "", "Connection reset by peer\n")],
                "provisioning alice@box1 failed: the ssh session to alice@box1 dropped before the \
                 privileged part (exit 255); nothing privileged was written: Connection reset \
                 by peer"
                    .to_owned(),
            ),
            (
                vec![
                    ok(""),
                    Ok(RemoteOutput {
                        code: None,
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                    }),
                ],
                "provisioning alice@box1 failed: the ssh session to alice@box1 dropped before the \
                 privileged part (exit by a signal); nothing privileged was written"
                    .to_owned(),
            ),
            (
                vec![
                    ok(""),
                    exit(255, "htui.root=start\n", "Connection reset by peer\n"),
                ],
                "provisioning alice@box1 failed: installing the service on alice@box1 failed \
                 (exit 255): Connection reset by peer"
                    .to_owned(),
            ),
            (
                vec![
                    ok(""),
                    exit(1, "htui.root=start\n", "install: cannot create directory\n"),
                ],
                "provisioning alice@box1 failed: installing the service on alice@box1 failed \
                 (exit 1): install: cannot create directory"
                    .to_owned(),
            ),
            (
                vec![
                    ok(""),
                    ok("htui.root=start\n"),
                    exit(5, "htui.active=activating\n", "a journal line\na log line\n"),
                ],
                "provisioning alice@box1 failed: the htui-worker service on alice@box1 did not \
                 start with a box.toml within 0 s (systemctl says activating); if the worker \
                 cannot use its DSN, fix it and re-run with --replace-credential; its last \
                 journal and log lines follow:\na journal line\na log line"
                    .to_owned(),
            ),
            (
                vec![ok(""), ok("htui.root=start\n"), exit(255, "", "broken pipe\n")],
                "provisioning alice@box1 failed: waiting for the worker on alice@box1 failed \
                 (exit 255): broken pipe"
                    .to_owned(),
            ),
            (
                vec![ok(""), ok("htui.root=start\n"), ok("htui.active=active\n")],
                "provisioning alice@box1 failed: the worker on alice@box1 wrote a box.toml htui \
                 cannot read: no box.toml in its answer"
                    .to_owned(),
            ),
        ];
        for (rest, expected) in cases {
            let remote = fresh(true, rest);
            let prompt = FakePrompt::new(Ok(PASSWORD));
            let verifier = FakeVerifier::new();
            let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
            assert_eq!(expect_exit(&ran, 1), expected);
            assert!(ran.out.is_empty());
        }
    }

    #[tokio::test]
    async fn no_baseline_is_a_warning_and_exit_0() {
        let remote = fresh(true, vec![ok(""), ok("htui.root=start\n"), verified()]);
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier {
            baseline: Err("unreachable".to_owned()),
            ..FakeVerifier::new()
        };
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert_eq!(
            ran.result.expect("exit 0"),
            Outcome::Provisioned {
                box_id: box_id(),
                hostname: "box1".to_owned(),
                verified: false,
                executor_set: false,
            }
        );
        assert!(
            ran.err
                .contains("not verified in Postgres from here: unreachable\n")
        );
        assert!(ran.err.contains("Settings › Boxes"));
        assert_eq!(verifier.log(), ["baseline"]);
        assert_eq!(ran.out, format!("box {BOX} (box1) provisioned on {DEST}\n"));
    }

    #[tokio::test]
    async fn a_box_never_seen_is_a_warning_and_exit_0() {
        let remote = fresh(true, vec![ok(""), ok("htui.root=start\n"), verified()]);
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier {
            seen: Err(format!("box {BOX} did not check in within 60 s")),
            ..FakeVerifier::new()
        };
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert!(matches!(
            ran.result,
            Ok(Outcome::Provisioned {
                verified: false,
                executor_set: false,
                ..
            })
        ));
        assert!(ran.err.contains(&format!(
            "warning: service active, box {BOX}; not verified in Postgres from here: box {BOX} \
             did not check in within 60 s; see journalctl -u htui-worker and \
             ~/.local/state/htui/worker.log on alice@box1\n"
        )));
        assert!(ran.err.contains(&format!(
            "set box {BOX}'s executor to worker in Settings › Boxes\n"
        )));
        assert_eq!(
            verifier.log(),
            ["baseline".to_owned(), format!("box_seen({BOX})")]
        );
    }

    #[tokio::test]
    async fn an_executor_write_that_fails_is_a_warning() {
        let remote = fresh(true, vec![ok(""), ok("htui.root=start\n"), verified()]);
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier {
            executor: Err("stale twice".to_owned()),
            ..FakeVerifier::new()
        };
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert!(matches!(
            ran.result,
            Ok(Outcome::Provisioned {
                verified: true,
                executor_set: false,
                ..
            })
        ));
        assert!(ran.err.contains(&format!(
            "warning: box {BOX}'s executor is not set to worker: stale twice; set it in \
             Settings › Boxes\n"
        )));
    }

    #[tokio::test]
    async fn the_sentinels_reach_only_stdin() {
        let remote = ScriptedRemote::new(vec![
            preflight(&[]),
            ok(""),
            ok("htui.root=start\n"),
            verified(),
        ]);
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, true, &remote, &prompt, &verifier).await;
        assert!(ran.result.is_ok());
        for (command, _) in remote.calls() {
            assert!(!command.contains("SENTINEL"), "a command holds a sentinel");
        }
        assert!(!ran.out.contains("SENTINEL"), "stdout holds a sentinel");
        assert!(!ran.err.contains("SENTINEL"), "stderr holds a sentinel");
        let install = String::from_utf8(remote.stdin_of("INSTALL")).expect("utf-8");
        assert!(install.contains("SENTINEL-SUDO-PW") && install.contains("SENTINEL-DSN-PW"));
        assert!(remote.command_of("INSTALL").contains(" 1 password '"));
    }

    /// Review finding 8: every session's whole `ssh` argv, as `SshRemote` would spawn it, is free
    /// of both sentinels.
    #[tokio::test]
    async fn ssh_argv_of_every_session_carries_no_sentinel() {
        let remote = ScriptedRemote::new(vec![
            preflight(&[]),
            ok(""),
            ok("htui.root=start\n"),
            verified(),
        ]);
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, true, &remote, &prompt, &verifier).await;
        assert!(ran.result.is_ok());
        assert_eq!(
            remote.sessions(),
            ["PREFLIGHT", "PREPARE", "INSTALL", "VERIFY"]
        );
        for (command, _) in remote.calls() {
            let argv = remote::ssh_argv(DEST, &command);
            assert_eq!(
                argv.last().map(|a| a.to_string_lossy().into_owned()),
                Some(command)
            );
            for arg in argv {
                let arg = arg.to_string_lossy();
                assert!(
                    !arg.contains("SENTINEL"),
                    "an ssh argument holds a sentinel"
                );
                assert!(!arg.contains(DSN) && !arg.contains(PASSWORD));
            }
        }
    }

    /// Review finding 3: the baseline comes after INSTALL, before VERIFY.
    #[tokio::test]
    async fn the_baseline_is_taken_after_install() {
        let timeline: Arc<Mutex<Vec<String>>> = Arc::default();
        let remote = ScriptedRemote {
            timeline: Arc::clone(&timeline),
            ..fresh(true, vec![ok(""), ok("htui.root=start\n"), verified()])
        };
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier {
            timeline: Arc::clone(&timeline),
            ..FakeVerifier::new()
        };
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert!(ran.result.is_ok());
        assert_eq!(
            *timeline.lock().expect("timeline"),
            [
                "PREFLIGHT".to_owned(),
                "PREPARE".to_owned(),
                "INSTALL".to_owned(),
                "baseline".to_owned(),
                "VERIFY".to_owned(),
                format!("box_seen({BOX})"),
                format!("set_executor({BOX})"),
            ]
        );
        assert!(!ran.err.contains("best-effort"), "a new box needs no note");

        // INSTALL that fails takes no baseline.
        let remote = fresh(true, vec![ok(""), exit(4, "", "")]);
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        expect_exit(&ran, 1);
        assert!(verifier.log().is_empty());
    }

    /// Review finding 3: a box already in the baseline (a re-provision) is verified only by a
    /// later last_seen_at, and the run says that is best-effort.
    #[tokio::test]
    async fn a_box_already_in_the_baseline_is_verified_best_effort() {
        let remote = fresh(true, vec![ok(""), ok("htui.root=start\n"), verified()]);
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let before = chrono::Utc::now();
        let verifier = FakeVerifier {
            baseline: Ok([(box_id(), before)].into_iter().collect()),
            ..FakeVerifier::new()
        };
        let ran = drive(DEST, DSN, true, &remote, &prompt, &verifier).await;
        assert!(matches!(
            ran.result,
            Ok(Outcome::Provisioned {
                verified: true,
                executor_set: true,
                ..
            })
        ));
        assert!(
            ran.err.contains(&format!(
                "note: box {BOX} was registered before this install, so the check-in is \
                 best-effort: an htui TUI on alice@box1 with the same box.toml would also count\n"
            )),
            "{}",
            ran.err
        );
    }

    /// Review finding 2: a session that never ends is closed at its bound: a refusal for the
    /// preflight, a failure after it.
    #[tokio::test]
    async fn a_session_that_never_ends_is_bounded() {
        let cases = [
            (
                0,
                2,
                "not provisioning alice@box1: the preflight on alice@box1 did not finish within \
                 50 ms; its ssh session was closed",
            ),
            (
                1,
                1,
                "provisioning alice@box1 failed: preparing alice@box1 did not finish within 50 \
                 ms; its ssh session was closed",
            ),
            (
                2,
                1,
                "provisioning alice@box1 failed: installing the service on alice@box1 did not \
                 finish within 50 ms; its ssh session was closed",
            ),
            (
                3,
                1,
                "provisioning alice@box1 failed: waiting for the worker on alice@box1 did not \
                 finish within 50 ms; its ssh session was closed",
            ),
        ];
        for (hang_at, code, expected) in cases {
            let remote = ScriptedRemote {
                hang_at: Some(hang_at),
                ..fresh(true, vec![ok(""), ok("htui.root=start\n"), verified()])
            };
            let prompt = FakePrompt::new(Ok(PASSWORD));
            let verifier = FakeVerifier::new();
            let ran = tokio::time::timeout(
                Duration::from_secs(10),
                drive_with(SHORT, DEST, DSN, false, &remote, &prompt, &verifier),
            )
            .await
            .expect("the run ends at the bound");
            assert_eq!(expect_exit(&ran, code), expected);
            assert_eq!(remote.calls().len(), hang_at + 1);
        }
    }

    #[test]
    fn the_production_bounds() {
        let t = Timings::PRODUCTION;
        assert_eq!(t.verify_wait_secs(), 58, "29 pauses of 2 s");
        assert_eq!(t.verify_limit(), Duration::from_secs(180));
        assert_eq!(t.poll.deadline, Duration::from_secs(90));
        assert_eq!(t.preflight_limit, Duration::from_secs(120));
        assert_eq!(t.install_limit, Duration::from_secs(300));
        assert_eq!(duration_text(Duration::from_millis(50)), "50 ms");
        assert_eq!(duration_text(Duration::from_secs(120)), "120 s");
    }

    /// Review finding 4: in password mode the sentence names the password and quotes sudo.
    #[tokio::test]
    async fn a_refused_password_quotes_sudo() {
        let remote = fresh(
            false,
            vec![
                ok(""),
                exit(
                    1,
                    "",
                    "Sorry, try again.\nsudo: 3 incorrect password attempts\n",
                ),
            ],
        );
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert_eq!(
            expect_exit(&ran, 1),
            "provisioning alice@box1 failed: sudo refused the password (or needs a tty or a \
             second factor) on alice@box1 (sudo: 3 incorrect password attempts); nothing \
             privileged was written"
        );
    }

    /// Review finding 9: remote text reaches neither stream nor the exit sentence with a control
    /// character other than a newline or a tab.
    #[tokio::test]
    async fn remote_text_is_sanitised_before_it_reaches_the_terminal() {
        let raw = |text: &str| {
            text.chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        };

        let remote = fresh(
            true,
            vec![
                ok(""),
                ok("htui.root=start\n"),
                exit(
                    5,
                    "htui.active=\u{1b}[2Jfailed\n",
                    "\u{1b}]0;owned\u{7}a journal line\n",
                ),
            ],
        );
        let prompt = FakePrompt::new(Ok(PASSWORD));
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        let sentence = expect_exit(&ran, 1);
        assert!(!raw(&sentence), "{sentence:?}");
        assert!(
            sentence.contains("\\u{1b}]0;owned\\u{7}a journal line"),
            "{sentence}"
        );
        assert!(
            sentence.contains("systemctl says \\u{1b}[2Jfailed"),
            "{sentence}"
        );

        let remote = ScriptedRemote::new(vec![preflight(&[("os", "Plan9\u{1b}[2J")])]);
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        let sentence = expect_exit(&ran, 2);
        assert!(!raw(&sentence), "{sentence:?}");

        let remote = fresh(
            true,
            vec![
                ok(""),
                ok("htui.root=start\n"),
                ok(&format!(
                    "{}\nbox_id = \"{BOX}\"\nhostname = \"box1\\u001b]0;owned\\u0007\"\n{}\n",
                    script::BOX_BEGIN,
                    script::BOX_END
                )),
            ],
        );
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert!(ran.result.is_ok(), "{:?}", ran.result);
        assert!(!raw(&ran.out), "{:?}", ran.out);
        assert!(!raw(&ran.err), "{:?}", ran.err);
        assert_eq!(
            ran.out,
            format!("box {BOX} (box1\\u{{1b}}]0;owned\\u{{7}}) provisioned on {DEST}\n")
        );
    }

    /// Review finding 10: with a terminal of its own, the prompt and its newline go there, not to
    /// stderr.
    #[tokio::test]
    async fn the_prompt_goes_to_the_prompters_terminal() {
        let remote = fresh(false, vec![ok(""), ok("htui.root=start\n"), verified()]);
        let terminal = Shared::default();
        let prompt = FakePrompt {
            terminal: Some(terminal.clone()),
            ..FakePrompt::new(Ok(PASSWORD))
        };
        let verifier = FakeVerifier::new();
        let ran = drive(DEST, DSN, false, &remote, &prompt, &verifier).await;
        assert!(ran.result.is_ok());
        assert_eq!(
            terminal.text(),
            "[sudo] password for alice on alice@box1: \n"
        );
        assert!(!ran.err.contains("[sudo]"), "{}", ran.err);
    }
}
