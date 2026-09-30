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
//!   an excerpt with the pasted `code` and `state` blanked — as pasted, decoded, percent-encoded
//!   in either case, as HTML character references, and as the trailing prefix an answer cut at
//!   [`RESPONSE_CAP`] ends on — and with every control and Unicode format character (the bidi
//!   overrides among them) stripped.
//! - **Never echoed.** The pane's field is masked, is emptied on submit, on cancel and when the
//!   flow ends, and nothing it draws afterwards names more than `127.0.0.1:<port>`. And the paste
//!   reaches nothing else (review M-1): the TUI runs with the terminal's bracketed-paste mode on,
//!   so a paste is one `Event::Paste` — wrapped in `Zeroizing` on arrival — that only a view
//!   capturing input takes, into its focused field, and that is dropped anywhere else. Without
//!   the mode a paste made before `p` was replayed as keystrokes: its digits switched tabs, its `/`
//!   opened the Requirements filter, and the `?code=…` after it was echoed there in clear.
//! - **Sent to one place.** The loopback port the running flow advertised, over a plain socket no
//!   proxy setting can reroute, following no redirect. One caveat, a browser's own (review L-1):
//!   for a `localhost` redirect `127.0.0.1` is tried before `[::1]`, so when the adapter bound
//!   only `[::1]:<port>`, another local user who holds `127.0.0.1:<port>` receives the code.
//!   PKCE bounds what that user can do with it: the code is worthless without the verifier the
//!   adapter never sent.
//!
//! Outside the rule, and stated rather than hidden: `url::Url`'s parse buffer (the worker's
//! [`validate`]; the pane's [`precheck`] parses nothing) and the kernel's socket buffers are not
//! wiped; the terminal's and crossterm's own input buffers, and the `String` crossterm hands the
//! paste over in before it is wrapped, are not either; a paste longer than [`PASTE_MAX`] is refused
//! whole by the field, but text *typed* past it may leave a reallocated prefix before `validate`
//! refuses it; and an adapter that prints the callback on its own stderr is shown by MOD-21's pane
//! as every stderr line is.

use std::cmp::Reverse;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout, timeout_at};
use url::{Host, Url};
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
/// its buffer is wiped on drop, and only [`validate`] and [`precheck`] read it.
#[derive(Clone)]
pub struct RedirectUrl(Zeroizing<String>);

impl RedirectUrl {
    /// Wraps what the pane's field `take()`s; the allocation moves, nothing is copied.
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    /// The text. Private to this module: only [`validate`] and the pane's [`precheck`] read it
    /// (D280, review L-8).
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
        let link = Url::parse(link.trim()).ok()?;
        let redirect = first_pair(&link, "redirect_uri")?;
        let redirect = Url::parse(&redirect).ok()?;
        if redirect.scheme() != "http" || !is_loopback(&redirect.host()?) {
            return None;
        }
        Some(Self {
            host: redirect.host_str()?.to_owned(),
            port: redirect.port_or_known_default()?,
            path: redirect.path().to_owned(),
            state: first_pair(&link, "state").filter(|state| !state.is_empty()),
        })
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

    /// D290: where to connect, from this host and never from a paste. `localhost` is
    /// `127.0.0.1` then `[::1]`; no name is ever resolved.
    ///
    /// The order is a browser's, and so is its one exposure (review L-1): a local user who binds
    /// `127.0.0.1:<port>` while the adapter listens only on `[::1]:<port>` is connected to first
    /// and receives the code. PKCE bounds it — the code cannot be redeemed without the adapter's
    /// verifier — and the browser the redirect was meant for would have sent it to the same place.
    fn socket_addrs(&self) -> Vec<SocketAddr> {
        if self.host.eq_ignore_ascii_case("localhost") {
            return vec![
                SocketAddr::new(Ipv4Addr::LOCALHOST.into(), self.port),
                SocketAddr::new(Ipv6Addr::LOCALHOST.into(), self.port),
            ];
        }
        let literal = self.host.trim_start_matches('[').trim_end_matches(']');
        literal
            .parse::<IpAddr>()
            .map(|ip| vec![SocketAddr::new(ip, self.port)])
            .unwrap_or_default()
    }
}

/// The first `name` pair of `url`'s query, percent-decoded.
fn first_pair(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// D264 (3): `127.0.0.0/8`, `::1`, or the name `localhost` in any case.
fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => *ip == Ipv6Addr::LOCALHOST,
        Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
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
    #[error(
        "the pasted text is longer than {max} bytes; paste the address bar only",
        max = PASTE_MAX
    )]
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
        /// The `error` value, filtered to `[A-Za-z0-9._-]` and cut to 64 characters; never empty.
        error: String,
    },
    /// An OAuth `error` whose value is empty once filtered: the same refusal, with nothing to name.
    #[error("the browser came back with an error instead of a code; the login was not granted")]
    BrowserErrorUnnamed,
    /// No non-empty `code`.
    #[error("the address has no code; copy the whole address the browser could not open")]
    MissingCode,
    /// No non-empty `state`.
    #[error("the address has no state; copy the whole address the browser could not open")]
    MissingState,
    /// No non-empty `state`, and the running login's link carried none either (maintainer OQ-3
    /// keeps `state` required): there is nothing to tell this adapter's redirect from another's.
    #[error(
        "the address has no state and the login link carried no state, so this adapter's \
         redirect cannot be verified; it cannot be pasted here"
    )]
    UnverifiableWithoutState,
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
        let mut out = format!("{} answered {}", self.target, self.status);
        if !self.reason.is_empty() {
            out.push(' ');
            out.push_str(&self.reason);
        }
        if let Some(host) = &self.location_host {
            out.push_str(", redirecting to ");
            out.push_str(host);
        }
        if let Some(said) = &self.said {
            out.push_str(": \"");
            out.push_str(said);
            out.push('"');
        }
        out
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
        kind: ErrorKind,
    },
}

/// D265: checks a pasted address against the advertised redirect, rules (1)–(12) in order, the
/// first failure wins. Pure: no I/O, no DNS.
///
/// # Errors
///
/// The [`PasteError`] of the first rule the paste breaks.
pub fn validate(url: &RedirectUrl, advertised: &Advertised) -> Result<Delivery, PasteError> {
    let text = with_scheme(url.as_str())?;
    // (3)
    let parsed = Url::parse(&text).map_err(|_| PasteError::NotAUrl)?;
    check_address(&parsed, advertised)?;
    let query = parsed.query().unwrap_or_default();
    let secrets = check_query(query, advertised)?;
    Ok(Delivery {
        targets: advertised.socket_addrs(),
        host_header: advertised.target(),
        request_target: Zeroizing::new(format!("{}?{query}", parsed.path())),
        secrets,
    })
}

/// D265 (1)–(2): the paste trimmed, bounded, and given `http://` when it names no scheme of its
/// own, in a wiped buffer.
fn with_scheme(pasted: &str) -> Result<Zeroizing<String>, PasteError> {
    // (1)
    let text = pasted.trim();
    if text.is_empty() {
        return Err(PasteError::Empty);
    }
    if text.len() > PASTE_MAX {
        return Err(PasteError::TooLong);
    }
    // (2): `Url::parse("localhost:39879/?…")` succeeds with the scheme `localhost`. A scheme is
    // looked for at the start only: a `://` inside the query is not one (review L-6).
    Ok(if has_scheme(text) {
        Zeroizing::new(text.to_owned())
    } else {
        Zeroizing::new(format!("http://{text}"))
    })
}

/// D265 (4)–(8): the scheme, the userinfo, the host, the port and the path are the advertised
/// redirect's.
fn check_address(parsed: &Url, advertised: &Advertised) -> Result<(), PasteError> {
    if parsed.scheme() != "http" {
        return Err(PasteError::NotHttp);
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(PasteError::HasUserinfo);
    }
    if parsed.host_str() != Some(advertised.host()) {
        return Err(PasteError::WrongHost {
            advertised: advertised.target(),
        });
    }
    let pasted = parsed.port_or_known_default().unwrap_or(80);
    if pasted != advertised.port() {
        return Err(PasteError::WrongPort {
            pasted,
            advertised: advertised.port(),
        });
    }
    if parsed.path() != advertised.path() {
        return Err(PasteError::WrongPath {
            advertised: format!("{}{}", advertised.target(), advertised.path()),
        });
    }
    Ok(())
}

/// D265 (9)–(12), over the raw query: no `error`, one `code`, one `state` and the advertised one.
/// Only the `error`, `code` and `state` values are ever decoded, each into a wiped buffer, so an
/// encoded code leaves no unwiped decoded copy behind. What comes back is what an echo of them is
/// blanked of.
fn check_query(query: &str, advertised: &Advertised) -> Result<Vec<Zeroizing<String>>, PasteError> {
    // (9)
    if let Some(refusal) = browser_error(query) {
        return Err(refusal);
    }
    // (10)–(11)
    let (code, raw_code) = exactly_one(query, "code", PasteError::MissingCode)?;
    let no_state = if advertised.state.is_some() {
        PasteError::MissingState
    } else {
        PasteError::UnverifiableWithoutState
    };
    let (state, raw_state) = exactly_one(query, "state", no_state)?;
    // (12)
    if let Some(expected) = &advertised.state
        && state.as_str() != expected
    {
        return Err(PasteError::StaleState);
    }
    Ok(secrets_of([
        code.as_str(),
        raw_code,
        state.as_str(),
        raw_state,
    ]))
}

/// D265 (9): the refusal of a redirect that came back with an OAuth `error`, its value filtered to
/// `[A-Za-z0-9._-]` and cut to 64 characters.
fn browser_error(query: &str) -> Option<PasteError> {
    let (_, raw) = raw_pairs(query).find(|(key, _)| key.as_str() == "error")?;
    let error = form_decode(raw)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(64)
        .collect::<String>();
    Some(if error.is_empty() {
        PasteError::BrowserErrorUnnamed
    } else {
        PasteError::BrowserError { error }
    })
}

/// What an echo is blanked of: the values as decoded and as they travel, non-empty, deduplicated,
/// longest first so a secret that contains another is blanked whole.
fn secrets_of(values: [&str; 4]) -> Vec<Zeroizing<String>> {
    let mut secrets: Vec<Zeroizing<String>> = Vec::with_capacity(values.len());
    for value in values {
        if !value.is_empty() && !secrets.iter().any(|known| known.as_str() == value) {
            secrets.push(Zeroizing::new(value.to_owned()));
        }
    }
    secrets.sort_by_key(|secret| Reverse(secret.len()));
    secrets
}

/// Review L-8: the pane's courtesy check before `Enter` sends a paste — rule (1), the scheme of
/// rule (4) when one is named, and the host and the port of rules (6)–(7), read off the text by
/// hand. No URL parse: everything else, and the authority on all of it, is [`validate`]'s, which
/// the worker runs on the paste it is sent. Refuses with `validate`'s own sentences, so a paste
/// the pane refuses is one the worker would have refused the same way.
///
/// # Errors
///
/// [`PasteError::Empty`], [`PasteError::TooLong`], [`PasteError::NotHttp`],
/// [`PasteError::NotAUrl`] for a port that is not one, [`PasteError::WrongHost`] or
/// [`PasteError::WrongPort`].
pub fn precheck(url: &RedirectUrl, advertised: &Advertised) -> Result<(), PasteError> {
    let text = url.as_str().trim();
    if text.is_empty() {
        return Err(PasteError::Empty);
    }
    if text.len() > PASTE_MAX {
        return Err(PasteError::TooLong);
    }
    let rest = match text.split_once("://") {
        Some((scheme, rest)) if has_scheme(text) => {
            if !scheme.eq_ignore_ascii_case("http") {
                return Err(PasteError::NotHttp);
            }
            rest
        }
        _ => text,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // A user name is `validate`'s refusal; the host is what follows it.
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host_port)| host_port);
    let (host, port) = split_host_port(host_port);
    if !host.eq_ignore_ascii_case(advertised.host()) {
        return Err(PasteError::WrongHost {
            advertised: advertised.target(),
        });
    }
    let pasted = match port {
        None | Some("") => 80,
        Some(port) => port.parse().map_err(|_| PasteError::NotAUrl)?,
    };
    if pasted != advertised.port() {
        return Err(PasteError::WrongPort {
            pasted,
            advertised: advertised.port(),
        });
    }
    Ok(())
}

/// `host:port` split as an authority is: a bracketed IPv6 literal keeps its brackets, and the
/// port is what follows the host's `:`, if anything does.
fn split_host_port(authority: &str) -> (&str, Option<&str>) {
    if authority.starts_with('[') {
        return match authority.find(']') {
            Some(close) => {
                let (host, rest) = authority.split_at(close + 1);
                (host, rest.strip_prefix(':'))
            }
            None => (authority, None),
        };
    }
    match authority.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    }
}

/// Whether `text` starts with an RFC 3986 scheme and `://`: a letter, then letters, digits, `+`,
/// `-` or `.`. Only the text before the **first** `://` is looked at, so a URL in the query of a
/// scheme-less paste is not mistaken for the paste's own scheme.
fn has_scheme(text: &str) -> bool {
    text.split_once("://").is_some_and(|(scheme, _)| {
        let mut chars = scheme.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// A query's pairs as `(decoded name, raw value)`, split as `form_urlencoded` splits them: on
/// `&`, empty segments skipped, then on the first `=`. No value is decoded here.
fn raw_pairs(query: &str) -> impl Iterator<Item = (Zeroizing<String>, &str)> {
    query
        .split('&')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let (name, value) = segment.split_once('=').unwrap_or((segment, ""));
            (form_decode(name), value)
        })
}

/// `application/x-www-form-urlencoded` decoding, as `Url::query_pairs` does it (`+` is a space,
/// `%XX` a byte, invalid UTF-8 replaced), into a wiped buffer.
fn form_decode(raw: &str) -> Zeroizing<String> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(raw.len()));
    let raw = raw.as_bytes();
    let digit = |at: usize| raw.get(at).and_then(|byte| char::from(*byte).to_digit(16));
    let mut at = 0;
    while let Some(&byte) = raw.get(at) {
        match (byte, digit(at + 1), digit(at + 2)) {
            (b'%', Some(high), Some(low)) => {
                // Two hex digits are at most 255.
                bytes.push((high * 16 + low) as u8);
                at += 3;
            }
            (b'+', ..) => {
                bytes.push(b' ');
                at += 1;
            }
            (byte, ..) => {
                bytes.push(byte);
                at += 1;
            }
        }
    }
    Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned())
}

/// D265 (10)/(11): the one non-empty `name` of the query, decoded, and its raw spelling.
fn exactly_one<'q>(
    query: &'q str,
    name: &'static str,
    missing: PasteError,
) -> Result<(Zeroizing<String>, &'q str), PasteError> {
    let mut values: Vec<(Zeroizing<String>, &str)> = raw_pairs(query)
        .filter(|(key, _)| key.as_str() == name)
        .map(|(_, raw)| (form_decode(raw), raw))
        .collect();
    if values.iter().all(|(value, _)| value.is_empty()) {
        return Err(missing);
    }
    if values.len() > 1 {
        return Err(PasteError::RepeatedParameter(name));
    }
    values.pop().ok_or(missing)
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
    let target = delivery.target();
    let mut stream = connect(&delivery.targets, &target, limits.connect).await?;
    let deadline = Instant::now() + limits.response;
    write_request(&mut stream, &delivery, deadline)
        .await
        .map_err(|failure| failure.into_error(&target, limits.response))?;
    let answer = read_answer(stream, deadline)
        .await
        .map_err(|failure| failure.into_error(&target, limits.response))?;
    classify(target, &answer, limits.response, &delivery.secrets)
}

/// Why writing or reading stopped short, before it is given the target and the deadline that
/// name it in a [`DeliverError`].
enum Failure {
    /// The response deadline elapsed.
    Deadline,
    /// The listener went away mid-write.
    Closed,
    /// Any other socket error.
    Io(ErrorKind),
}

impl Failure {
    fn into_error(self, target: &str, after: Duration) -> DeliverError {
        let target = target.to_owned();
        match self {
            Self::Deadline => DeliverError::Timeout { target, after },
            Self::Closed => DeliverError::ClosedWithoutAnswer { target },
            Self::Io(kind) => DeliverError::Io { target, kind },
        }
    }
}

/// D267: the request line, `Host:` and three fixed headers, written by `deadline`.
///
/// Built in place with room to spare so it never reallocates, and wiped on drop.
async fn write_request(
    stream: &mut TcpStream,
    delivery: &Delivery,
    deadline: Instant,
) -> Result<(), Failure> {
    let mut request = Zeroizing::new(Vec::with_capacity(
        delivery.request_target.len() + delivery.host_header.len() + 128,
    ));
    for part in [
        "GET ",
        delivery.request_target.as_str(),
        " HTTP/1.1\r\nHost: ",
        delivery.host_header.as_str(),
        "\r\nUser-Agent: htui\r\nAccept: text/html, text/plain, */*\r\nConnection: close\r\n\r\n",
    ] {
        request.extend_from_slice(part.as_bytes());
    }
    match timeout_at(deadline, stream.write_all(&request)).await {
        Err(_) => Err(Failure::Deadline),
        Ok(Err(err)) if is_closed(err.kind()) => Err(Failure::Closed),
        Ok(Err(err)) => Err(Failure::Io(err.kind())),
        Ok(Ok(())) => Ok(()),
    }
}

/// What came back, in a wiped buffer, and why the read stopped.
struct Answer {
    buffer: Zeroizing<Vec<u8>>,
    len: usize,
    ended: Ended,
}

impl Answer {
    fn bytes(&self) -> &[u8] {
        &self.buffer[..self.len]
    }
}

/// D276: read until EOF, a reset, the cap, a complete framing, or the deadline. The stream is
/// dropped on the way out, so the listener sees the close as soon as the answer is in.
async fn read_answer(mut stream: TcpStream, deadline: Instant) -> Result<Answer, Failure> {
    let mut buffer = Zeroizing::new(vec![0u8; RESPONSE_CAP]);
    let mut len = 0;
    let ended = loop {
        if len == RESPONSE_CAP {
            break Ended::Cap;
        }
        if is_framed(&buffer[..len]) {
            break Ended::Framed;
        }
        match timeout_at(deadline, stream.read(&mut buffer[len..])).await {
            Err(_) => break Ended::Deadline,
            Ok(Ok(0)) => break Ended::Eof,
            Ok(Ok(read)) => len += read,
            Ok(Err(err))
                if matches!(
                    err.kind(),
                    ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
                ) =>
            {
                break Ended::Reset;
            }
            Ok(Err(err)) => return Err(Failure::Io(err.kind())),
        }
    };
    Ok(Answer { buffer, len, ended })
}

/// D276: what an answer amounts to — nothing, not HTTP, a timeout on a status line still coming,
/// or a status line and what the listener said.
fn classify(
    target: String,
    answer: &Answer,
    after: Duration,
    secrets: &[Zeroizing<String>],
) -> Result<ListenerReply, DeliverError> {
    let bytes = answer.bytes();
    if bytes.is_empty() {
        return Err(match answer.ended {
            Ended::Deadline => DeliverError::Timeout { target, after },
            _ => DeliverError::ClosedWithoutAnswer { target },
        });
    }
    let Some(line_end) = find(bytes, b"\n") else {
        let may_be_http = bytes.starts_with(HTTP) || HTTP.starts_with(bytes);
        return Err(match answer.ended {
            Ended::Deadline if may_be_http => DeliverError::Timeout { target, after },
            _ => DeliverError::NotHttp { target },
        });
    };
    let Some((status, reason)) = status_line(&bytes[..line_end]) else {
        return Err(DeliverError::NotHttp { target });
    };
    let truncated = matches!(answer.ended, Ended::Cap);
    Ok(reply(target, status, reason, bytes, secrets, truncated))
}

/// What every HTTP/1 status line starts with.
const HTTP: &[u8] = b"HTTP/1.";

/// Why the read loop stopped (D276).
enum Ended {
    Eof,
    Reset,
    Cap,
    Framed,
    Deadline,
}

/// A write error that means the listener went away rather than that the socket broke.
fn is_closed(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
    )
}

/// D267: each target in order; a refusal moves to the next, and none left is
/// [`DeliverError::NothingListening`].
async fn connect(
    targets: &[SocketAddr],
    target: &str,
    limit: Duration,
) -> Result<TcpStream, DeliverError> {
    for addr in targets {
        match timeout(limit, TcpStream::connect(*addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            // A `localhost` fallback to `[::1]` on a box with no IPv6 loopback is "not there"
            // too, not a broken socket.
            Ok(Err(err))
                if matches!(
                    err.kind(),
                    ErrorKind::ConnectionRefused | ErrorKind::AddrNotAvailable
                ) => {}
            Ok(Err(err)) => {
                return Err(DeliverError::Io {
                    target: target.to_owned(),
                    kind: err.kind(),
                });
            }
            Err(_) => {
                return Err(DeliverError::Timeout {
                    target: target.to_owned(),
                    after: limit,
                });
            }
        }
    }
    Err(DeliverError::NothingListening {
        target: target.to_owned(),
    })
}

/// Where `needle` first starts in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// `HTTP/1.x SSS reason` → the status in `100..=599` and the raw reason; `None` otherwise.
fn status_line(line: &[u8]) -> Option<(u16, String)> {
    let line = String::from_utf8_lossy(line);
    let line = line.trim_end_matches('\r');
    let mut parts = line.splitn(3, ' ');
    let version = parts.next()?;
    let code = parts.next()?;
    if !version.starts_with("HTTP/1.")
        || code.len() != 3
        || !code.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let status: u16 = code.parse().ok()?;
    (100..=599)
        .contains(&status)
        .then(|| (status, parts.next().unwrap_or_default().to_owned()))
}

/// The end of the head, blank line included, and the head's header lines.
fn split_head(answer: &[u8]) -> Option<(usize, &[u8])> {
    let crlf = find(answer, b"\r\n\r\n").map(|at| at + 4);
    let lf = find(answer, b"\n\n").map(|at| at + 2);
    let end = match (crlf, lf) {
        (Some(a), Some(b)) => a.min(b),
        (a, b) => a.or(b)?,
    };
    Some((end, &answer[..end]))
}

/// The value of header `name` (case-insensitive) in `head`, past the status line.
fn header(head: &[u8], name: &str) -> Option<String> {
    String::from_utf8_lossy(head)
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim().to_owned())
}

fn is_chunked(head: &[u8]) -> bool {
    header(head, "transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
}

/// D276 (d): the head is complete and the body is complete by its framing.
fn is_framed(answer: &[u8]) -> bool {
    let Some((end, head)) = split_head(answer) else {
        return false;
    };
    let body = &answer[end..];
    let status = find(head, b"\n").and_then(|at| status_line(&head[..at]));
    if matches!(status, Some((204 | 304, _))) {
        return true;
    }
    if is_chunked(head) {
        return dechunk(body).1;
    }
    match header(head, "content-length").and_then(|value| value.parse::<usize>().ok()) {
        Some(length) => body.len() >= length,
        None => false,
    }
}

/// A chunked body, decoded as far as it goes, and whether its terminal chunk was seen (or its
/// framing broke, which ends it at what was decoded).
fn dechunk(mut body: &[u8]) -> (Zeroizing<Vec<u8>>, bool) {
    let mut out = Zeroizing::new(Vec::with_capacity(body.len()));
    loop {
        let Some(line_end) = find(body, b"\n") else {
            return (out, false);
        };
        let size = String::from_utf8_lossy(&body[..line_end]);
        let size = size.split(';').next().unwrap_or_default().trim();
        let Ok(size) = usize::from_str_radix(size, 16) else {
            return (out, true);
        };
        if size == 0 {
            return (out, true);
        }
        body = &body[line_end + 1..];
        if body.len() < size {
            out.extend_from_slice(body);
            return (out, false);
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size..];
        body = body
            .strip_prefix(b"\r\n")
            .or_else(|| body.strip_prefix(b"\n"))
            .unwrap_or(body);
    }
}

/// D268, D289: what the listener said, blanked of every secret. `truncated`: the read stopped at
/// [`RESPONSE_CAP`], so the body may end part-way through an echo (review L-5).
fn reply(
    target: String,
    status: u16,
    reason: String,
    answer: &[u8],
    secrets: &[Zeroizing<String>],
    truncated: bool,
) -> ListenerReply {
    let reason = Zeroizing::new(reason);
    let (head, body): (&[u8], Zeroizing<Vec<u8>>) = match split_head(answer) {
        Some((end, head)) if is_chunked(head) => (head, dechunk(&answer[end..]).0),
        Some((end, head)) => (head, Zeroizing::new(answer[end..].to_vec())),
        None => (answer, Zeroizing::new(Vec::new())),
    };
    let body = blank(
        Zeroizing::new(String::from_utf8_lossy(&body).into_owned()),
        secrets,
        truncated,
    );

    let location_host = if (300..=399).contains(&status) {
        header(head, "location")
            .map(|location| blank(Zeroizing::new(location), secrets, false))
            .and_then(|location| Url::parse(&location).ok())
            .and_then(|location| location.host_str().map(str::to_owned))
            .map(|host| cut(&blank(Zeroizing::new(host), secrets, false), EXCERPT_WIDTH))
    } else {
        None
    };
    let reason = cut(&tidy(&blank(reason, secrets, false), secrets), 40);
    let said = excerpt(&body)
        .map(|said| cut(&tidy(&said, secrets), EXCERPT_WIDTH))
        .filter(|said| !said.is_empty());

    ListenerReply {
        target,
        status,
        reason,
        said,
        location_host,
    }
}

/// D268 (2): the `<title>`, else the first body line that is non-blank once its tags are gone.
fn excerpt(body: &str) -> Option<Zeroizing<String>> {
    let lower = Zeroizing::new(body.to_ascii_lowercase());
    if let Some(open) = lower.find("<title")
        && let Some(start) = lower[open..].find('>').map(|at| open + at + 1)
        && let Some(end) = lower[start..].find("</title>").map(|at| start + at)
    {
        let title = Zeroizing::new(strip_tags(&body[start..end]));
        if !title.trim().is_empty() {
            return Some(title);
        }
    }
    body.lines()
        .map(|line| Zeroizing::new(strip_tags(line)))
        .find(|line| !line.trim().is_empty())
}

/// Everything between a `<` and the next `>`, gone.
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// D268 (1)/(4), review L-5: every secret replaced by `…`, however the listener spells it — as
/// pasted, percent-encoded with either case of hex (whole or in part), with `+` for a space, or
/// as HTML character references (`&#67;`, `&#x43;`, `&amp;` and the other four XML names).
///
/// With `truncated` — an answer the read cut at [`RESPONSE_CAP`] — a text that **ends** part-way
/// through a spelling of a secret, even inside an escape, has that trailing prefix blanked too.
fn blank(
    mut text: Zeroizing<String>,
    secrets: &[Zeroizing<String>],
    truncated: bool,
) -> Zeroizing<String> {
    for secret in secrets {
        text = blank_one(&text, secret.as_bytes(), truncated);
    }
    text
}

/// [`blank`] for one secret, into a buffer sized so it never reallocates: a one-byte secret
/// spelled as itself is the worst case, three bytes of `…` for each byte of text.
fn blank_one(text: &str, secret: &[u8], truncated: bool) -> Zeroizing<String> {
    let bytes = text.as_bytes();
    let mut out = Zeroizing::new(String::with_capacity(text.len().saturating_mul(3)));
    let mut copied = 0;
    let mut at = 0;
    while at < bytes.len() {
        match spelled_at(bytes, at, secret, truncated) {
            Some(end) if text.is_char_boundary(at) && text.is_char_boundary(end) => {
                out.push_str(&text[copied..at]);
                out.push('…');
                copied = end;
                at = end;
            }
            _ => at += 1,
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// Where a spelling of `secret` that starts at `at` ends. With `to_the_end`, also the end of the
/// text when it runs out — or stops inside an escape — after at least one byte of the secret.
///
/// Each step tries the escaped spellings of the secret's next byte before the byte itself, so a
/// `%` in a secret still matches a literal `%` when what follows is not its escape.
fn spelled_at(text: &[u8], at: usize, secret: &[u8], to_the_end: bool) -> Option<usize> {
    let mut j = at;
    let mut k = 0;
    while k < secret.len() {
        if j == text.len() || (to_the_end && is_cut_escape(&text[j..])) {
            return (to_the_end && k > 0).then_some(text.len());
        }
        let (read, matched) = step(&text[j..], &secret[k..])?;
        j += read;
        k += matched;
    }
    Some(j)
}

/// One spelling of the start of `secret` at the start of `text`: how many bytes of each it
/// covers, or `None` when `text` does not start with any spelling of it.
fn step(text: &[u8], secret: &[u8]) -> Option<(usize, usize)> {
    let hex = |at: usize| text.get(at).and_then(|b| char::from(*b).to_digit(16));
    match text.first()? {
        b'%' => {
            if let (Some(high), Some(low)) = (hex(1), hex(2))
                // Two hex digits are at most 255.
                && secret.first() == Some(&((high * 16 + low) as u8))
            {
                return Some((3, 1));
            }
        }
        b'+' if secret.first() == Some(&b' ') => return Some((1, 1)),
        b'&' => {
            if let Some((c, read)) = reference(text) {
                let mut utf8 = [0u8; 4];
                let spelled = c.encode_utf8(&mut utf8).as_bytes();
                if secret.starts_with(spelled) {
                    return Some((read, spelled.len()));
                }
            }
        }
        _ => {}
    }
    (text.first() == secret.first()).then_some((1, 1))
}

/// An HTML character reference at the start of `text` — `&#NN;`, `&#xHH;` (either case of `x`
/// and of hex) or one of the five XML names — and its length.
fn reference(text: &[u8]) -> Option<(char, usize)> {
    let end = text.iter().take(12).position(|b| *b == b';')?;
    let name = core::str::from_utf8(text.get(1..end)?).ok()?;
    let c = match name.strip_prefix('#') {
        Some(number) => char::from_u32(match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        })?,
        None => match name {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => return None,
        },
    };
    Some((c, end + 1))
}

/// Whether `rest` — the end of a cut text — is the start of an escape the cut split: a `%` and at
/// most one hex digit, or a `&` and at most ten more reference characters with no `;`.
fn is_cut_escape(rest: &[u8]) -> bool {
    match rest.split_first() {
        Some((b'%', tail)) => tail.len() < 2 && tail.iter().all(u8::is_ascii_hexdigit),
        Some((b'&', tail)) => {
            tail.len() <= 10 && tail.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'#')
        }
        _ => false,
    }
}

/// D268 (3)/(4), review L-5: control characters and Unicode format characters dropped,
/// whitespace runs collapsed to one space, trimmed, and the secrets blanked again.
fn tidy(text: &str, secrets: &[Zeroizing<String>]) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(text.len()));
    for c in text.chars() {
        if c.is_whitespace() {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
        } else if !c.is_control() && !is_format(c) {
            out.push(c);
        }
    }
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    blank(out, secrets, false)
}

/// Unicode general category `Cf` (Unicode 15.1): the bidi embeddings, overrides and isolates
/// (U+202A–202E, U+2066–2069), the marks (U+200E/F, U+061C), the zero-width characters, the BOM,
/// the soft hyphen and the rest — invisible, and able to reorder or hide what a line reads.
fn is_format(c: char) -> bool {
    matches!(
        c,
        '\u{ad}'
            | '\u{600}'..='\u{605}'
            | '\u{61c}'
            | '\u{6dd}'
            | '\u{70f}'
            | '\u{890}'..='\u{891}'
            | '\u{8e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

/// D268 (5): at most `width` chars, the last one `…` when cut.
fn cut(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push('…');
    out
}
