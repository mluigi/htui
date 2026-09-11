//! The degraded CLI transport: an agent that speaks only its own headless JSON stream, reaching
//! the same chat tab, recorder, store rows and replay as an ACP one (`docs/ANA-4.md` §4.3, §4.4,
//! §6.2, §7; MOD-2 milestone 8).
//!
//! The split mirrors `acp/`: the supervisor and the session task live in this file, and the
//! wire → [`DriverEvent`] mapping lives alone in [`claude`], which imports no process type and is
//! unit-testable from a single recorded line.
//!
//! **What differs from `acp/`, stated once.** There is no protocol layer at all: a line out is a
//! user message, a line in is a JSON value, and nothing negotiates. So there is no handshake beyond
//! the first `system/init`, no permission channel (§4.3 fixes
//! `DriverCaps { permission_requests: false, edit_proposals: false, plans: false }` for this
//! transport and [`AgentSession::answer_permission`] answers [`DriverError::Unsupported`]), and a
//! cancel is a **signal** rather than a notification — which is why it is the one sequence in this
//! file written from measurements instead of from a specification (plan D81, findings F-1..F-3).
//!
//! [`DriverEvent`]: crate::event::DriverEvent

pub mod claude;

use std::time::Duration;

use crate::driver::SessionSpec;
use crate::launch::CliSettings;

/// The adapter id this transport registers under (plan D12): `cli/<settings.cli.stream>`.
pub const ADAPTER_ID: &str = "cli/claude_stream_json";

/// The `settings.cli.stream` value that selects it — half of [`ADAPTER_ID`], and what a registry
/// row declares.
///
/// A **dialect** name, not an agent name (`R-AGT-5`): two rows may declare it, and nothing in this
/// module ever reads `agent.name` to decide anything.
pub const STREAM: &str = "claude_stream_json";

/// How long the first `system/init` may take before [`open_session`] gives up.
///
/// The CLI's login refusal prints to stderr and exits, which is EOF and is reported at once; this
/// bounds the other shape — an agent that hangs before saying anything — so a failed `ChatStart`
/// cannot hold the tab that issued it forever.
pub const INIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Depth of the session task's event channel; [`crate::acp::EVENTS_CAPACITY`]'s reason, and the
/// same number so the two transports back-pressure a slow consumer alike.
pub const EVENTS_CAPACITY: usize = 256;

/// The grace window a session gets when its **handle** is dropped rather than cancelled;
/// [`crate::acp::DROP_GRACE`]'s reason.
pub const DROP_GRACE: Duration = Duration::from_secs(1);

/// `other.update` of a stdout line that is not JSON at all.
///
/// Not an error, and deliberately not a reason to stop reading (blueprint H-23): §6.2's rule for a
/// shape `htui` does not recognize is "stored verbatim", and a line a future release prints in
/// front of its stream — a warning, a progress bar, a crash trace — is exactly the thing a reader
/// of the transcript will want. No recorded transcript contains one (F-15), which is why this is
/// written from the rule rather than from a fixture.
pub const UNPARSED: &str = "<unparsed>";

// ---------------------------------------------------------------------------------------------
// The invocation (`docs/ANA-4.md` §4.4)
// ---------------------------------------------------------------------------------------------

/// The argv of `docs/ANA-4.md` §4.4, assembled from the row and the spec. Pure, and unit-tested as
/// a list rather than through a process.
///
/// Order, and every position in it is a decision:
///
/// 1. the row's own resolved `args` first, so a registry row that wraps the CLI in something (`npx`,
///    a shim) keeps its own leading arguments where that something expects them;
/// 2. the fixed flags §4.4 verified: `-p` with **no positional prompt**, because the prompt travels
///    on stdin as the first user message and an argv is visible in `ps` to every account on the box
///    (blueprint P-2);
/// 3. the row's `--permission-mode`, omitted when the row names none rather than guessed at;
/// 4. the session id **or** the resume id, never both — `--session-id` mints (D84), `--resume`
///    continues, and the CLI refuses the pair (blueprint H-18);
/// 5. the spec's model and extra directories;
/// 6. the budget, **only above zero** — see below;
/// 7. `settings.cli.extra_args` **last**, so an operator's repeated flag is the one the CLI keeps.
///
/// **The budget flag is omitted at zero, and that is a measurement, not a nicety** (plan F-10):
/// `--max-budget-usd 0` is refused before the CLI reads a byte of stdin — the process exits 1 with
/// no stdout at all — so passing it for an absent cap would turn "no cap" into "no turn".
#[must_use]
pub fn argv(
    row_args: &[String],
    cli: &CliSettings,
    spec: &SessionSpec,
    session_id: &str,
) -> Vec<String> {
    let mut args: Vec<String> = row_args.to_vec();
    // `--verbose` is what makes `stream-json` emit every envelope rather than the terminal
    // `result` alone, and `--include-partial-messages` is what turns the `stream_event` channel on
    // — the deltas the chat tab renders as the reply arrives (§4.4).
    args.extend(
        [
            "-p",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
        ]
        .map(ToOwned::to_owned),
    );

    // Scoped so the borrow ends before `extra_args` is appended: the flags below are all pairs,
    // and spelling `push` twice per pair is what a reader has to check for a transposition.
    {
        let mut push = |flag: &str, value: &str| {
            args.push(flag.to_owned());
            args.push(value.to_owned());
        };
        if !cli.permission_mode.is_empty() {
            push("--permission-mode", &cli.permission_mode);
        }
        match spec.resume.as_ref() {
            Some(resume) => push("--resume", resume.as_str()),
            None => push("--session-id", session_id),
        }
        if let Some(model) = spec.model.as_deref() {
            push("--model", model);
        }
        for dir in &spec.extra_dirs {
            push("--add-dir", &dir.to_string_lossy());
        }
        if let Some(micros) = spec.budget_micros.filter(|micros| *micros > 0) {
            push("--max-budget-usd", &usd(micros));
        }
    }

    args.extend(cli.extra_args.iter().cloned());
    args
}

/// USD micros as the decimal `--max-budget-usd` takes: integer arithmetic, six places, no float.
///
/// `300` → `"0.000300"`, `1_500_000` → `"1.500000"`. A float round-trip is what this exists to
/// avoid — the recorder's client-side cap and the CLI's server-side one must read **one** number
/// (D83, D90), and two caps that disagree in the sixth decimal place are worse than one.
///
/// A negative figure never reaches here: `ProjectCaps::from_settings` refuses it and [`argv`] gates
/// on `> 0` besides. The `debug_assert!` says so where it would be violated, and the clamp keeps
/// the release build producing a well-formed decimal rather than the `-0.-000300` the naive
/// arithmetic would emit.
#[must_use]
pub fn usd(micros: i64) -> String {
    debug_assert!(micros >= 0, "a per-run cap in micros is never negative");
    let micros = micros.max(0);
    format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000)
}
