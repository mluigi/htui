//! Plan MOD-21: the transport-neutral half of a login (D9, D11, `R-AGT-9`).
//!
//! Everything a *user* sees of a flow — a method's name, a stderr line, a link, an outcome — is a
//! type here; the wire (`acp/auth.rs`) produces them and the runtime consumes them. A CLI
//! transport that one day grows a login of its own produces the same [`AuthEvent`]s without
//! touching `acp/`.
//!
//! Nothing here can carry a credential: every field is an id, a sentence the agent already wrote
//! to its own stderr, or a status (`R-SEC-2`, `R-ID-7`). Nothing here names an agent, a method id
//! or a host (`R-AGT-5`): the method list is the agent's own `initialize` answer and the outcome
//! is the probe's.
//!
//! [`run`] is the one submodule that names a transport, and it names exactly one thing about it:
//! which driver it is resolving a launch out of. Everything it then does between the spawn and the
//! outcome — the environment the child starts in, the lines it writes, the clock over them — is the
//! same for any transport that ever grows a login.

pub mod run;

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub use run::authenticate;

/// Plan MOD-21 D13: a flow silent for this long — no stderr line, no event, no choice — is
/// cancelled and reported [`AuthOutcome::Idle`].
///
/// Measured from the last sign of life, never from the spawn: a human OAuth round trip is minutes
/// of nothing on stderr, so a cap measured from the spawn would kill a slow but live login while a
/// cap measured from the last line kills only an abandoned one. Injected through [`AuthFlow::idle`]
/// so a test can use milliseconds.
pub const AUTH_IDLE_CAP: Duration = Duration::from_secs(10 * 60);

/// Everything one login needs from its caller (plan MOD-21 D11).
///
/// One spawn serves both the method list and the call, so the human's answer arrives *into* the
/// running operation rather than through a second one.
#[derive(Debug)]
pub struct AuthFlow {
    /// Where the adapter runs; the probe's `cwd`, never a session's — a login belongs to a box,
    /// not to a chat.
    pub cwd: PathBuf,
    /// The method list, then every stderr line and every new URL, as they happen.
    pub events: mpsc::UnboundedSender<AuthEvent>,
    /// The one answer to [`AuthEvent::Methods`]. A sender dropped unused is
    /// [`AuthOutcome::Declined`].
    pub choice: oneshot::Receiver<AuthChoice>,
    /// Tripped by the pane, by shutdown, or (through a child token) by the idle clock.
    pub cancel: CancellationToken,
    /// [`AUTH_IDLE_CAP`] in production.
    pub idle: Duration,
    /// Plan MOD-21 D16: [`BrowserPolicy::Neutralised`] in production;
    /// [`BrowserPolicy::Inherit`] exists for the regression pair.
    pub browser: BrowserPolicy,
}

/// What a flow tells its caller (plan MOD-21 D11). Text and ids only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthEvent {
    /// The agent's `initialize` answer, once, before anything else.
    Methods {
        /// Every `agent`-kind method, in the agent's order. An unrecognised `type` is one of
        /// these: the schema declares its agent arm untagged, so it swallows every unknown kind.
        methods: Vec<AuthMethodInfo>,
        /// The agent advertised a logout capability.
        logout: bool,
        /// `terminal`-typed methods: named so the chooser can say why they are missing, never
        /// sent (plan MOD-21 D4, D21 — the spec forbids passing one to `authenticate`).
        hidden: Vec<AuthMethodInfo>,
    },
    /// One stderr line, as the adapter wrote it.
    Line(String),
    /// A `http(s)` URL seen on stderr for the first time in this flow (plan MOD-21 D15).
    Url(String),
}

/// One advertised method, in the agent's own words.
///
/// Crosses `StoreReply`, so it derives what that enum derives (plan MOD-21 T1): `Debug` and
/// `Clone` for the reply, `Serialize`/`Deserialize` because the frame is a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthMethodInfo {
    /// `authMethods[].id` — the value `authenticate` is sent.
    pub id: String,
    /// `authMethods[].name`, the label the chooser shows.
    pub name: String,
    /// `authMethods[].description`, the sentence under the label.
    pub description: Option<String>,
}

/// The user's answer to [`AuthEvent::Methods`]. Crosses `StoreRequest`, hence the same derives as
/// [`AuthMethodInfo`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthChoice {
    /// `authenticate` with this method id.
    Method(String),
    /// `logout`.
    Logout,
}

/// Which call the flow made, carried on the outcome so a caller can say which one ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthCall {
    /// `authenticate` with this method id.
    Authenticate(String),
    /// `logout`.
    Logout,
}

/// How a flow ended (plan MOD-21 D11).
///
/// `Ok` covers everything the agent got to say, including "no": a JSON-RPC error to the call is an
/// *answer*, not a transport failure, because it names what the user has to do next (D5). Spawn
/// and wire failures stay [`crate::error::DriverError`]s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    /// The call returned. Says nothing about the box: the probe decides (plan MOD-21 D6,
    /// `R-AGT-6`).
    Completed {
        /// Which call returned.
        call: AuthCall,
    },
    /// The agent answered the call with a JSON-RPC error.
    Refused {
        /// Which call was refused.
        call: AuthCall,
        /// The agent's own `Display` text, plus the stderr tail on a new line when there was one.
        message: String,
    },
    /// The token tripped: the pane, or shutdown.
    Cancelled,
    /// The idle clock tripped after this much silence (plan MOD-21 D13).
    Idle {
        /// The cap that elapsed with no sign of life.
        after: Duration,
    },
    /// The [`AuthFlow::choice`] sender was dropped before choosing.
    Declined,
}

/// Plan MOD-21 D16: what the auth spawn's environment says to the adapter's own browser opener.
///
/// Applied to the login spawn only — never to a chat's, never to the probe's, never recorded in
/// `agent_box.probe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowserPolicy {
    /// `BROWSER` is set to an existing no-op that exits 0, so a chain-style opener stops rather
    /// than falling through to a terminal browser that would write escape sequences into the
    /// JSON-RPC channel. Production.
    #[default]
    Neutralised,
    /// The environment exactly as the row resolved it. The regression pair's control.
    Inherit,
}

/// Plan MOD-21 D17: what opening a link spawns. Production is [`OpenerCommand::Platform`]; a test
/// injects a script and reads what it recorded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OpenerCommand {
    /// The platform's own opener, chosen by `cfg`.
    #[default]
    Platform,
    /// This program, with the URL as its one argument.
    Custom(PathBuf),
}
