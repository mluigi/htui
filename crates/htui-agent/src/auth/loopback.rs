//! MOD-22: completing a loopback OAuth login from a box the browser cannot reach.
//!
//! An agent's own login redirects the browser to a listener **inside the adapter**, on the loopback
//! of the box `htui` runs on (RFC 8252 §7.3). When the browser is on another machine, that address
//! fails there, and the user holds a `http://127.0.0.1:<port>/?code=…&state=…` their browser could
//! not open. This module is the three things the login pane needs to finish the job on this box:
//! [`Advertised::from_auth_url`] reads which loopback address the running flow is listening on,
//! from the standard `redirect_uri` parameter of the link the adapter printed (never from a
//! vendor's sentence, `R-AGT-5`); [`validate`] checks a pasted address against it; [`deliver`]
//! issues that one request to that port and reports what the listener said.
//!
//! Pure except for [`deliver`], which is the only function here that touches a socket and runs on
//! the login's own task, never on the UI task (`R-NF-3`). No DNS lookup ever happens.
//!
//! # The credential rule (`R-SEC-2`, `R-ID-7`)
//!
//! The address a user pastes here carries an OAuth authorization `code`; for the length of one
//! request it is a credential, and everything below is written around that:
//!
//! - **Never logged.** No `tracing` field, no panic message and no `Debug` or `Display` prints it:
//!   [`RedirectUrl`] prints `RedirectUrl(<redacted>)`, [`Delivery`] and [`Advertised`] print a host
//!   and a port, and every [`PasteError`] and [`DeliverError`] is a fixed sentence naming a host, a
//!   port or a rule — never a byte that was pasted, except an OAuth `error` value such as
//!   `access_denied`, filtered to `[A-Za-z0-9._-]` and cut to 64 characters.
//! - **Never persisted.** No row, no file, no keyring entry, no mirror.
//! - **Never on a frame that outlives the request.** It crosses the store worker once, inside
//!   `StoreRequest::AuthDeliver`, is moved into the running flow's task, and is dropped when the
//!   one `GET` resolves. What comes back, [`ListenerReply`], holds a status, a reason, a host and
//!   an excerpt with the pasted `code` and `state` blanked.
//! - **Never echoed.** The pane's field is masked, is emptied on submit, on cancel and when the
//!   flow ends, and nothing it draws afterwards names more than `127.0.0.1:<port>`.
//! - **Sent to one place.** The loopback port the running flow advertised, over a plain socket no
//!   proxy setting can reroute, following no redirect.
//!
//! Outside the rule, and stated rather than hidden: `url::Url`'s parse buffer and the kernel's
//! socket buffers are not wiped, a paste longer than [`PASTE_MAX`] may leave a reallocated prefix in
//! the pane's field before it is refused, and an adapter that prints the callback on its own stderr
//! is shown by MOD-21's pane as every stderr line is.

use std::net::SocketAddr;
use std::time::Duration;

use zeroize::Zeroizing;

/// D265 (1): the longest paste [`validate`] accepts, in bytes after trimming. Also the capacity
/// the pane's masked field is opened with (D275), so an acceptable paste never reallocates there.
pub const PASTE_MAX: usize = 8 * 1024;

/// D267: the most [`deliver`] reads of an answer, head and body together.
pub const RESPONSE_CAP: usize = 16 * 1024;

/// D268: the widest excerpt [`ListenerReply::said`] holds, in chars, the `…` included.
pub const EXCERPT_WIDTH: usize = 80;

/// What a paste is refused with when the running login's link advertised no loopback redirect.
/// Shared by the pane (`p`) and the worker (`AuthCommand::Deliver`).
pub const NO_LOOPBACK_REDIRECT: &str =
    "this login's link advertises no loopback redirect, so there is nothing to paste";

/// What a second paste is refused with while the first is still on its way. Shared by the pane
/// (`p`) and the worker (a second `Deliver`).
pub const DELIVERY_IN_FLIGHT: &str =
    "the pasted address is still being delivered; wait for the listener's answer";

/// D266: the pasted address, as typed. The `htui_store::Dsn` shape.
///
/// A credential for the length of one request (see the module's credential rule): its `Debug` is
/// `RedirectUrl(<redacted>)`, it has no `Display`, no `Serialize`, no `Deref` and no `PartialEq`,
/// its buffer is wiped on drop, and only [`validate`] reads it.
#[derive(Clone)]
pub struct RedirectUrl(Zeroizing<String>);

impl RedirectUrl {
    /// Wraps what the pane's field `take()`s; the allocation moves, nothing is copied.
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    /// The text. Private to this module: only [`validate`] reads it (D280).
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for RedirectUrl {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RedirectUrl(<redacted>)")
    }
}

/// D264: the loopback redirect a running login's link advertised.
///
/// Read from the link's standard `redirect_uri` parameter, never from a sentence. Its `Debug`
/// prints the host, the port and the path, and `state: <redacted>` (D281).
#[derive(Clone, PartialEq, Eq)]
pub struct Advertised {
    /// `Url::host_str()`: `127.0.0.1`, `[::1]` or `localhost` (lower-cased by `url`).
    host: String,
    /// `port_or_known_default()`, so 80 when the redirect names none.
    port: u16,
    /// `Url::path()`, `/` at least.
    path: String,
    /// The link's own `state`, when it carried a non-empty one.
    state: Option<String>,
}

impl Advertised {
    /// D264 (1)–(4): the loopback redirect `link` advertises.
    ///
    /// `None` for a non-URL, a link with no or an unparseable `redirect_uri`, a redirect that is
    /// not `http`, or a host that is not `127.0.0.0/8`, `::1` or `localhost` (ASCII
    /// case-insensitive). The **first** `redirect_uri` wins; `state` is the first `state` of the
    /// link, `None` when absent or empty. No DNS lookup happens.
    #[must_use]
    pub fn from_auth_url(link: &str) -> Option<Self> {
        let _ = link;
        todo!("MOD-22 T1 (b)")
    }

    /// The host as `url` serialises it: `127.0.0.1`, `[::1]`, `localhost`.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port, 80 when the redirect named none.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// The path, `/` at least.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// `host:port`: what the pane draws and what every sentence names (`127.0.0.1:39879`).
    #[must_use]
    pub fn target(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl core::fmt::Debug for Advertised {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut out = f.debug_struct("Advertised");
        out.field("host", &self.host)
            .field("port", &self.port)
            .field("path", &self.path);
        match self.state {
            Some(_) => out.field("state", &format_args!("<redacted>")),
            None => out.field("state", &None::<()>),
        };
        out.finish()
    }
}

/// D265's product: everything one `GET` needs, and nothing a caller can read back but its target.
///
/// Its `Debug` prints the target only; the request line and the secrets it blanks are wiped on
/// drop.
pub struct Delivery {
    /// Where to connect, from the **advertised** host (D290): an IPv4 literal is itself, `[::1]`
    /// is itself, `localhost` is `127.0.0.1` then `[::1]`.
    targets: Vec<SocketAddr>,
    /// [`Advertised::target`], sent as `Host:`.
    host_header: String,
    /// The pasted path, `?`, and the pasted raw query; never the fragment.
    request_target: Zeroizing<String>,
    /// The pasted `code` and `state`, raw and percent-decoded, non-empty, deduplicated, longest
    /// first: what an echo in the listener's answer is blanked of (D268).
    secrets: Vec<Zeroizing<String>>,
}

impl Delivery {
    /// `host:port` of the advertised redirect, as [`Advertised::target`] gives it.
    #[must_use]
    pub fn target(&self) -> String {
        self.host_header.clone()
    }
}

impl core::fmt::Debug for Delivery {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Delivery")
            .field("target", &self.host_header)
            .finish_non_exhaustive()
    }
}

/// D265: why a paste was refused. Every `Display` is fixed text (D273, D285).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PasteError {
    /// Nothing but whitespace.
    #[error("nothing was pasted")]
    Empty,
    /// Longer than [`PASTE_MAX`] bytes after trimming.
    #[error("the pasted text is longer than 8192 bytes; paste the address bar only")]
    TooLong,
    /// Not parseable as a URL.
    #[error("the pasted text is not an address")]
    NotAUrl,
    /// A scheme other than `http`.
    #[error("the address is not plain http; a loopback redirect always is")]
    NotHttp,
    /// A user name or a password.
    #[error("the address carries a user name or password; a loopback redirect never does")]
    HasUserinfo,
    /// Another host than the advertised one.
    #[error("the address is for another host; this login is listening on {advertised}")]
    WrongHost {
        /// [`Advertised::target`].
        advertised: String,
    },
    /// Another port than the advertised one.
    #[error("the address is for port {pasted}; this login is listening on {advertised}")]
    WrongPort {
        /// The pasted port, 80 when the paste named none.
        pasted: u16,
        /// The advertised port.
        advertised: u16,
    },
    /// Another path than the advertised one.
    #[error("the address is for another path; this login is listening on {advertised}")]
    WrongPath {
        /// [`Advertised::target`] followed by the advertised path.
        advertised: String,
    },
    /// The browser came back with an OAuth `error` instead of a `code`.
    #[error("the browser came back with `{error}` instead of a code; the login was not granted")]
    BrowserError {
        /// The `error` value, filtered to `[A-Za-z0-9._-]` and cut to 64 characters.
        error: String,
    },
    /// No non-empty `code`.
    #[error("the address has no code; copy the whole address the browser could not open")]
    MissingCode,
    /// No non-empty `state`.
    #[error("the address has no state; copy the whole address the browser could not open")]
    MissingState,
    /// `code` or `state` more than once.
    #[error("the address carries {0} more than once")]
    RepeatedParameter(&'static str),
    /// A `state` other than the one the running login's link carried.
    #[error(
        "the address is from an earlier attempt (its state is not this login's); finish the \
         consent again from the link above"
    )]
    StaleState,
}

/// D267: the two deadlines. Production is `Default`; a test injects milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliverLimits {
    /// Per connect attempt.
    pub connect: Duration,
    /// From the connect to the end of the read: covers the write and the whole answer.
    pub response: Duration,
}

impl Default for DeliverLimits {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(2),
            response: Duration::from_secs(15),
        }
    }
}

/// D268: what the listener said. Crosses `AuthFrame::Delivered`, so every field is safe to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerReply {
    /// [`Advertised::target`]: `127.0.0.1:39879`.
    pub target: String,
    /// The status code, `100..=599`.
    pub status: u16,
    /// The reason phrase, sanitised as `said` is and cut to 40 chars; may be empty.
    pub reason: String,
    /// The `<title>`, else the first non-blank body line with tags stripped; `None` when empty.
    pub said: Option<String>,
    /// For a `3xx` with an absolute `Location`: its host only. A relative one is `None`.
    pub location_host: Option<String>,
}

impl ListenerReply {
    /// `{target} answered {status}`, then ` {reason}` when there is one, `, redirecting to {host}`
    /// when there is a `location_host`, and `: "{said}"` when there is an excerpt.
    #[must_use]
    pub fn summary(&self) -> String {
        todo!("MOD-22 T1 (b)")
    }
}

/// D268 + D276: why nothing usable came back. Every `Display` names `target` and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeliverError {
    /// Every connect attempt was refused.
    #[error(
        "nothing is listening on {target} \u{2014} the login may have ended; x cancels it and a \
         new attempt listens elsewhere"
    )]
    NothingListening {
        /// [`Advertised::target`].
        target: String,
    },
    /// A connect attempt, or the answer, outran its deadline.
    #[error("{target} did not answer within {after:?}")]
    Timeout {
        /// [`Advertised::target`].
        target: String,
        /// The deadline that elapsed.
        after: Duration,
    },
    /// Bytes came back, but not an HTTP status line.
    #[error("{target} answered, but not in HTTP")]
    NotHttp {
        /// [`Advertised::target`].
        target: String,
    },
    /// The listener closed or reset the connection before a byte of answer.
    #[error(
        "{target} closed the connection without answering; if the login completed, its result \
         follows"
    )]
    ClosedWithoutAnswer {
        /// [`Advertised::target`].
        target: String,
    },
    /// Any other socket error, named by its kind (never the OS message).
    #[error("delivering to {target} failed: {kind}")]
    Io {
        /// [`Advertised::target`].
        target: String,
        /// The error's kind.
        kind: std::io::ErrorKind,
    },
}

/// D265: checks a pasted address against the advertised redirect, rules (1)–(12) in order, the
/// first failure wins. Pure: no I/O, no DNS.
///
/// # Errors
///
/// The [`PasteError`] of the first rule the paste breaks.
pub fn validate(url: &RedirectUrl, advertised: &Advertised) -> Result<Delivery, PasteError> {
    let _ = (url.as_str(), advertised);
    todo!("MOD-22 T1 (b)")
}

/// D267: one `GET` to the advertised loopback port, following nothing, reading at most
/// [`RESPONSE_CAP`].
///
/// # Errors
///
/// A [`DeliverError`] when nothing listens, nothing usable came back in time, or the socket
/// failed. A `4xx` or `5xx` answer is an `Ok`: the adapter decides what it means.
pub async fn deliver(
    delivery: Delivery,
    limits: DeliverLimits,
) -> Result<ListenerReply, DeliverError> {
    let Delivery {
        targets,
        host_header,
        request_target,
        secrets,
    } = delivery;
    let _ = (targets, host_header, request_target, secrets, limits);
    todo!("MOD-22 T1 (b)")
}
