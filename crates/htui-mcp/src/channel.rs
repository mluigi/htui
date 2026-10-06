//! The channel between the agent's `htui mcp` child and the htui process that hosts its session
//! (MOD-11 D3, OQ-1, blueprint §2.8, B-12).
//!
//! One [`Listener`] per process, bound lazily by the first session: a Unix socket in a private
//! (`0700`) directory, or a Windows named pipe that refuses remote clients. The child connects,
//! sends one [`HandshakeLine`] — the session's [`Token`] and its [`RELAY_VERSION`] — and gets a
//! [`HandshakeReply`]. After `ok` the stream carries the MCP NDJSON unchanged in both directions;
//! the child never parses it (I-2). A refusal carries a sentence for the child's stderr and the
//! host closes the stream; the sentence never names the token (I-6).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::RELAY_VERSION;
use crate::protocol::{Handler, serve};

/// How long the host waits for a connection's handshake line before dropping it.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// The most a handshake line (or its reply) may hold. A token and a version need ~120 bytes.
const HANDSHAKE_MAX_BYTES: u64 = 4096;

/// How long the relay waits for the host's last answers after the agent closes stdin, when the
/// stream cannot half-close (a Windows named pipe).
const DRAIN_WITHOUT_HALF_CLOSE: Duration = Duration::from_secs(1);

/// How long a Windows child keeps retrying a busy pipe, and how often.
#[cfg(windows)]
const PIPE_BUSY_BUDGET: Duration = Duration::from_secs(5);
#[cfg(windows)]
const PIPE_BUSY_POLL: Duration = Duration::from_millis(50);

/// A session's secret: 32 bytes from two `Uuid::new_v4()` (getrandom-backed), lowercase hex, 64
/// chars. `Debug` prints `Token(…)` — never the value (I-6).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Token(String);

impl Token {
    /// A fresh token.
    #[must_use]
    pub fn mint() -> Self {
        let a = uuid::Uuid::new_v4().simple().to_string();
        let b = uuid::Uuid::new_v4().simple().to_string();
        Self(a + &b)
    }

    /// The token's text, for the child's environment and the handshake only.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A token from its text: exactly 64 lowercase hex characters, or `None`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let well_formed = s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        well_formed.then(|| Self(s.to_owned()))
    }
}

impl core::fmt::Debug for Token {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Token(…)")
    }
}

/// Where the child connects: a socket path (Unix) or a pipe name (Windows), rendered into
/// `HTUI_MCP_ADDR` as is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address(String);

impl Address {
    /// An address from its text (the child reads it from `HTUI_MCP_ADDR`).
    #[must_use]
    pub fn new(address: impl Into<String>) -> Self {
        Self(address.into())
    }

    /// The address's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Resolves a handshake to the session that serves the connection, or refuses it.
pub type Lookup = Arc<dyn Fn(&HandshakeLine) -> Result<Arc<dyn Handler>, Refusal> + Send + Sync>;

/// The first line the child sends.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandshakeLine {
    /// The session's token, 64 lowercase hex.
    pub token: String,
    /// The child's [`RELAY_VERSION`].
    pub version: String,
}

/// The host's answer to the handshake.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandshakeReply {
    /// Whether the stream now carries MCP.
    pub ok: bool,
    /// Why not: a [`Refusal`]'s sentence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Why the host refused a handshake.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// No live session has the token.
    #[error("no live htui session has this token (it ended, or it never existed)")]
    UnknownToken,
    /// The child is not the host's build of the relay (B-12).
    #[error("version mismatch: the host is htui {host}, this relay is {relay}; restart the agent")]
    Version {
        /// The host's [`RELAY_VERSION`].
        host: String,
        /// The child's.
        relay: String,
    },
    /// The line was not a handshake.
    #[error("malformed handshake")]
    Malformed,
}

impl Refusal {
    /// The refusal a reply's `reason` names; a sentence this build does not know is `Malformed`.
    #[must_use]
    pub fn from_reason(reason: &str) -> Self {
        if reason == Self::UnknownToken.to_string() {
            return Self::UnknownToken;
        }
        let version = reason
            .strip_prefix("version mismatch: the host is htui ")
            .and_then(|rest| rest.strip_suffix("; restart the agent"))
            .and_then(|rest| rest.split_once(", this relay is "));
        match version {
            Some((host, relay)) => Self::Version {
                host: host.to_owned(),
                relay: relay.to_owned(),
            },
            None => Self::Malformed,
        }
    }
}

/// The child's failure (D6): it could not connect, the host refused it, or the stream broke.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    /// Nothing listens at the address.
    #[error("cannot reach the htui host at {0}: {1}")]
    Connect(String, std::io::Error),
    /// The host refused the handshake.
    #[error("{0}")]
    Refused(Refusal),
    /// The stream or stdio failed after the handshake.
    #[error("relay i/o: {0}")]
    Io(std::io::Error),
}

/// One per process, created lazily by the first `open` (D3). Drop = [`close`](Self::close).
pub struct Listener {
    address: Address,
    /// The private directory the socket lives in (Unix); removed on close.
    dir: Option<PathBuf>,
    stop: watch::Sender<bool>,
}

impl core::fmt::Debug for Listener {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Listener")
            .field("address", &self.address)
            .field("closed", &*self.stop.borrow())
            .finish_non_exhaustive()
    }
}

impl Listener {
    /// Binds and spawns the accept loop (`contained::spawn`). Each accepted stream gets its own
    /// task: read the handshake line ([`HANDSHAKE_TIMEOUT`]), resolve the token through `lookup`,
    /// answer, then [`serve`] the stream with the session.
    ///
    /// Must be called inside a tokio runtime.
    ///
    /// # Errors
    ///
    /// The bind's error, or a directory that is not private.
    pub fn bind(lookup: Lookup) -> std::io::Result<Self> {
        Self::bind_with(lookup, HANDSHAKE_TIMEOUT)
    }

    /// [`bind`](Self::bind) with a handshake budget of the caller's choosing (tests).
    pub(crate) fn bind_with(lookup: Lookup, budget: Duration) -> std::io::Result<Self> {
        let (stop, stopped) = watch::channel(false);
        let (address, dir) = platform::bind(lookup, budget, stopped)?;
        Ok(Self { address, dir, stop })
    }

    /// Where the child connects.
    #[must_use]
    pub fn address(&self) -> &Address {
        &self.address
    }

    /// Stops the accept loop and ends every connection it accepted; on Unix removes `<dir>/s`
    /// then `<dir>`, best effort. Idempotent.
    pub fn close(&mut self) {
        self.stop.send_replace(true);
        if let Some(dir) = self.dir.take() {
            let _ = std::fs::remove_file(dir.join(platform::SOCKET_NAME));
            let _ = std::fs::remove_dir(&dir);
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.close();
    }
}

/// Accepts until `stopped` turns true (or its sender drops), each connection in its own task.
/// Ending the loop drops the set, which aborts every connection still open.
async fn accept_loop<S, F>(
    mut accept: impl FnMut() -> F,
    lookup: Lookup,
    budget: Duration,
    mut stopped: watch::Receiver<bool>,
) where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    F: Future<Output = std::io::Result<S>>,
{
    let mut connections = JoinSet::new();
    loop {
        if *stopped.borrow_and_update() {
            break;
        }
        tokio::select! {
            changed = stopped.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            accepted = accept() => match accepted {
                Ok(stream) => {
                    htui_agent::contained::spawn_in(
                        &mut connections,
                        connection(stream, Arc::clone(&lookup), budget),
                    );
                }
                Err(err) => {
                    // Out of descriptors, or a client that vanished mid-accept: keep serving the
                    // sessions that exist, without spinning.
                    tracing::warn!(error = %err, "htui-mcp: accept failed");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

/// One accepted stream: the handshake, then MCP until the child closes it.
async fn connection<S>(stream: S, lookup: Lookup, budget: Duration)
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let mut stream = BufReader::new(stream);
    let mut line = Vec::new();
    let read = tokio::time::timeout(
        budget,
        (&mut stream)
            .take(HANDSHAKE_MAX_BYTES)
            .read_until(b'\n', &mut line),
    )
    .await;
    let Ok(Ok(read)) = read else {
        // Silent past the budget, or broken: dropped without a word.
        return;
    };
    if read == 0 {
        return;
    }
    let verdict = match serde_json::from_slice::<HandshakeLine>(&line) {
        Err(_) => Err(Refusal::Malformed),
        Ok(hello) if hello.version != RELAY_VERSION => Err(Refusal::Version {
            host: RELAY_VERSION.to_owned(),
            relay: hello.version,
        }),
        Ok(hello) => lookup(&hello),
    };
    let (reply, handler) = match verdict {
        Ok(handler) => (
            HandshakeReply {
                ok: true,
                reason: None,
            },
            Some(handler),
        ),
        Err(refusal) => (
            HandshakeReply {
                ok: false,
                reason: Some(refusal.to_string()),
            },
            None,
        ),
    };
    if write_line(&mut stream, &reply).await.is_err() {
        return;
    }
    match handler {
        Some(handler) => {
            if let Err(err) = serve(stream, handler).await {
                tracing::debug!(error = %err, "htui-mcp: a connection ended with an error");
            }
        }
        None => {
            let _ = stream.shutdown().await;
        }
    }
}

/// Writes `value` as one JSON line and flushes.
async fn write_line<W, T>(write: &mut W, value: &T) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: serde::Serialize,
{
    let mut bytes = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    write.write_all(&bytes).await?;
    write.flush().await
}

/// The child side (D6): connect, send the line, read the reply, then splice stdin → stream and
/// stream → stdout until either side ends.
///
/// The host closing the stream ends the relay at once. On Unix the agent closing stdin half-closes
/// the socket, and the relay then waits for the host to finish answering. A Windows named pipe
/// cannot half-close (tokio's `poll_shutdown` only flushes; the host never reads EOF), so there the
/// relay waits at most [`DRAIN_WITHOUT_HALF_CLOSE`] for answers already on their way, then ends;
/// dropping the pipe is the host's EOF.
///
/// # Errors
///
/// [`RelayError`].
pub async fn relay<R, W>(
    addr: &Address,
    token: &Token,
    mut stdin: R,
    mut stdout: W,
) -> Result<(), RelayError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let stream = platform::connect(addr)
        .await
        .map_err(|err| RelayError::Connect(addr.as_str().to_owned(), err))?;
    let mut stream = BufReader::new(stream);
    let hello = HandshakeLine {
        token: token.as_str().to_owned(),
        version: RELAY_VERSION.to_owned(),
    };
    write_line(&mut stream, &hello)
        .await
        .map_err(RelayError::Io)?;
    let mut line = Vec::new();
    let read = (&mut stream)
        .take(HANDSHAKE_MAX_BYTES)
        .read_until(b'\n', &mut line)
        .await
        .map_err(RelayError::Io)?;
    if read == 0 {
        return Err(RelayError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "the htui host closed the connection during the handshake",
        )));
    }
    let reply: HandshakeReply =
        serde_json::from_slice(&line).map_err(|_| RelayError::Refused(Refusal::Malformed))?;
    if !reply.ok {
        return Err(RelayError::Refused(Refusal::from_reason(
            reply.reason.as_deref().unwrap_or_default(),
        )));
    }

    let (from_host, to_host) = tokio::io::split(stream);
    let drain = cfg!(windows).then_some(DRAIN_WITHOUT_HALF_CLOSE);
    splice(&mut stdin, &mut stdout, from_host, to_host, drain)
        .await
        .map_err(RelayError::Io)
}

/// Copies `stdin` → host and host → `stdout` until either side ends. The host's end ends the
/// splice at once. Stdin's end shuts the host-bound half down; then, with `drain` `None` (a stream
/// that half-closes), the splice waits for the host to close, and with `Some(d)` for at most `d`.
async fn splice<R, W, HR, HW>(
    mut stdin: R,
    mut stdout: W,
    mut from_host: HR,
    mut to_host: HW,
    drain: Option<Duration>,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    HR: AsyncRead + Unpin,
    HW: AsyncWrite + Unpin,
{
    let up = async {
        tokio::io::copy(&mut stdin, &mut to_host).await?;
        to_host.shutdown().await
    };
    // `copy` flushes `stdout` whenever the host has nothing more to read, so a drain that runs out
    // has already delivered every answer the host wrote.
    let down = async {
        tokio::io::copy(&mut from_host, &mut stdout).await?;
        stdout.flush().await
    };
    tokio::pin!(up, down);
    tokio::select! {
        down = &mut down => down,
        up = &mut up => match (up, drain) {
            (Err(err), _) => Err(err),
            (Ok(()), None) => down.await,
            (Ok(()), Some(drain)) => tokio::time::timeout(drain, down).await.unwrap_or(Ok(())),
        },
    }
}

/// One instance of a listener that serves a single client per instance: a Windows named pipe's
/// server end. A trait so the accept step ([`connect_next`]) is tested on every platform.
#[cfg(any(windows, test))]
trait PipeInstance: Sized {
    /// Waits for a client on this instance.
    fn connect(&self) -> impl Future<Output = std::io::Result<()>> + Send;
}

/// One accept on a pipe-like listener (tokio's named-pipe pattern): waits for a client on the
/// instance in `slot`, puts a fresh instance from `create` in its place **before** handing the
/// connected one off, and returns it.
///
/// A failed connect replaces the instance too, before the error goes back to the accept loop. A
/// client that opens and closes the pipe before `ConnectNamedPipe` leaves the instance answering
/// `ERROR_NO_DATA` to every later connect; retrying it would lock every later child out.
#[cfg(any(windows, test))]
async fn connect_next<P: PipeInstance>(
    slot: &mut P,
    create: impl Fn() -> std::io::Result<P>,
) -> std::io::Result<P> {
    if let Err(err) = slot.connect().await {
        match create() {
            // Dropping the broken instance closes it.
            Ok(fresh) => *slot = fresh,
            Err(create) => {
                tracing::warn!(error = %create, "htui-mcp: cannot replace a failed pipe instance");
            }
        }
        return Err(err);
    }
    let fresh = create()?;
    Ok(std::mem::replace(slot, fresh))
}

#[cfg(unix)]
mod platform {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use tokio::net::{UnixListener, UnixStream};
    use tokio::sync::watch;

    use super::{Address, Lookup, accept_loop};

    /// The socket's file name inside the private directory: short, for `sun_path` (H-12).
    pub(super) const SOCKET_NAME: &str = "s";

    pub(super) fn bind(
        lookup: Lookup,
        budget: Duration,
        stopped: watch::Receiver<bool>,
    ) -> std::io::Result<(Address, Option<PathBuf>)> {
        // `<base>/htui-mcp-<pid>-<8 hex>`, `0700` and checked: the helper the CLI driver's MCP
        // config shares, so both sit under one base (MOD-79 D2).
        let dir = htui_agent::private_dir::create("htui-mcp")?;
        let socket = dir.join(SOCKET_NAME);
        let listener = match UnixListener::bind(&socket) {
            Ok(listener) => listener,
            Err(err) => {
                remove(&dir);
                return Err(err);
            }
        };
        let Some(address) = socket.to_str().map(Address::new) else {
            remove(&dir);
            return Err(std::io::Error::other("the socket path is not UTF-8"));
        };
        let listener = std::sync::Arc::new(listener);
        htui_agent::contained::spawn(accept_loop(
            move || {
                let listener = std::sync::Arc::clone(&listener);
                async move { listener.accept().await.map(|(stream, _)| stream) }
            },
            lookup,
            budget,
            stopped,
        ));
        Ok((address, Some(dir)))
    }

    fn remove(dir: &Path) {
        let _ = std::fs::remove_file(dir.join(SOCKET_NAME));
        let _ = std::fs::remove_dir(dir);
    }

    pub(super) async fn connect(addr: &Address) -> std::io::Result<UnixStream> {
        UnixStream::connect(addr.as_str()).await
    }
}

#[cfg(windows)]
mod platform {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };
    use tokio::sync::{Mutex, watch};

    use super::{
        Address, Lookup, PIPE_BUSY_BUDGET, PIPE_BUSY_POLL, PipeInstance, accept_loop, connect_next,
    };

    /// Unused on Windows: a pipe has no file to remove.
    pub(super) const SOCKET_NAME: &str = "s";

    /// `ERROR_PIPE_BUSY`: every instance is taken; the server is about to create the next.
    const ERROR_PIPE_BUSY: i32 = 231;

    impl PipeInstance for NamedPipeServer {
        fn connect(&self) -> impl Future<Output = std::io::Result<()>> + Send {
            NamedPipeServer::connect(self)
        }
    }

    pub(super) fn bind(
        lookup: Lookup,
        budget: Duration,
        stopped: watch::Receiver<bool>,
    ) -> std::io::Result<(Address, Option<PathBuf>)> {
        let name = format!(r"\\.\pipe\htui-mcp-{}", uuid::Uuid::new_v4().simple());
        let first = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)?;
        // tokio's documented pattern: create the next instance **before** handing the connected
        // one off, so a client never finds no instance at all.
        let next = Arc::new(Mutex::new(first));
        let pipe = name.clone();
        htui_agent::contained::spawn(accept_loop(
            move || {
                let next = Arc::clone(&next);
                let pipe = pipe.clone();
                async move {
                    let mut server = next.lock().await;
                    connect_next(&mut *server, || {
                        ServerOptions::new()
                            .reject_remote_clients(true)
                            .create(&pipe)
                    })
                    .await
                }
            },
            lookup,
            budget,
            stopped,
        ));
        Ok((Address::new(name), None))
    }

    pub(super) async fn connect(addr: &Address) -> std::io::Result<NamedPipeClient> {
        let deadline = tokio::time::Instant::now() + PIPE_BUSY_BUDGET;
        loop {
            match ClientOptions::new().open(addr.as_str()) {
                Ok(client) => return Ok(client),
                Err(err)
                    if err.raw_os_error() == Some(ERROR_PIPE_BUSY)
                        && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(PIPE_BUSY_POLL).await;
                }
                Err(err) => return Err(err),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

    use super::{Listener, Lookup, PipeInstance, Refusal, Token, connect_next, splice};

    #[test]
    fn a_token_is_64_lowercase_hex_and_debug_hides_it() {
        let token = Token::mint();
        let text = token.as_str();
        assert_eq!(text.len(), 64);
        assert!(
            text.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "{text}"
        );
        assert_eq!(Token::parse(text), Some(token.clone()));
        assert_ne!(Token::mint(), token, "two mints differ");
        let debug = format!("{token:?}");
        assert_eq!(debug, "Token(…)");
        assert!(!debug.contains(text));
        assert_eq!(Token::parse(&text.to_uppercase()), None);
        assert_eq!(Token::parse(&text[..63]), None);
        assert_eq!(Token::parse(&format!("{}g", &text[..63])), None);
    }

    #[test]
    fn a_refusal_round_trips_through_its_reason() {
        for refusal in [
            Refusal::UnknownToken,
            Refusal::Malformed,
            Refusal::Version {
                host: "0.1.0+relay.1".to_owned(),
                relay: "0.2.0+relay.1".to_owned(),
            },
        ] {
            assert_eq!(Refusal::from_reason(&refusal.to_string()), refusal);
        }
        assert_eq!(Refusal::from_reason("something new"), Refusal::Malformed);
    }

    /// A writer whose `shutdown` only flushes, as tokio's Windows named-pipe client does: the peer
    /// never reads EOF.
    struct NoHalfClose<W>(W);

    impl<W: AsyncWrite + Unpin> AsyncWrite for NoHalfClose<W> {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.0).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_flush(cx)
        }
    }

    /// ADV-1: over a stream that cannot half-close, the agent closing stdin ends the relay after
    /// the drain, with the host's answers already written delivered, although the host never
    /// closes its side.
    #[tokio::test]
    async fn without_a_half_close_stdin_eof_ends_the_relay_after_the_drain() {
        let (relay_side, mut host_side) = tokio::io::duplex(1024);
        host_side
            .write_all(b"{\"answer\":1}\n")
            .await
            .expect("the host answers");
        let (from_host, to_host) = tokio::io::split(relay_side);
        let mut stdout = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            splice(
                &b"{\"request\":1}\n"[..],
                &mut stdout,
                from_host,
                NoHalfClose(to_host),
                Some(Duration::from_millis(100)),
            ),
        )
        .await
        .expect("the relay ends after the drain, not when the host closes")
        .expect("a clean end");
        assert_eq!(stdout, b"{\"answer\":1}\n");
        let mut request = vec![0; 64];
        let read = host_side.read(&mut request).await.expect("a read");
        assert_eq!(&request[..read], b"{\"request\":1}\n");
    }

    /// Over a stream that half-closes (Unix), the relay waits for the host's last answer after
    /// stdin ends, and ends when the host closes.
    #[tokio::test]
    async fn with_a_half_close_the_relay_waits_for_the_host_to_finish() {
        let (relay_side, mut host_side) = tokio::io::duplex(1024);
        let host = async move {
            let mut request = Vec::new();
            host_side
                .read_to_end(&mut request)
                .await
                .expect("the request, then EOF");
            tokio::time::sleep(Duration::from_millis(200)).await;
            host_side
                .write_all(b"{\"answer\":1}\n")
                .await
                .expect("the late answer");
            request
        };
        let (from_host, to_host) = tokio::io::split(relay_side);
        let mut stdout = Vec::new();
        let relay = tokio::time::timeout(
            Duration::from_secs(5),
            splice(
                &b"{\"request\":1}\n"[..],
                &mut stdout,
                from_host,
                to_host,
                None,
            ),
        );
        let (request, relayed) = tokio::join!(host, relay);
        relayed
            .expect("the relay ends when the host closes")
            .expect("a clean end");
        assert_eq!(stdout, b"{\"answer\":1}\n");
        assert_eq!(request, b"{\"request\":1}\n");
    }

    /// A pipe instance for [`connect_next`]: `broken` fails every connect, as a Windows instance
    /// does once a client opened and closed it before `ConnectNamedPipe` (`ERROR_NO_DATA`).
    #[derive(Debug)]
    struct FakeInstance {
        serial: u32,
        broken: bool,
    }

    impl PipeInstance for FakeInstance {
        fn connect(&self) -> impl Future<Output = std::io::Result<()>> + Send {
            let broken = self.broken;
            async move {
                if broken {
                    Err(std::io::Error::from_raw_os_error(232))
                } else {
                    Ok(())
                }
            }
        }
    }

    /// ADV-3: an instance whose connect failed is replaced before the error is returned, so the
    /// accept loop's retry waits on a fresh instance instead of failing on the broken one forever.
    #[tokio::test]
    async fn a_failed_connect_replaces_the_instance() {
        let made = std::sync::atomic::AtomicU32::new(1);
        let create = || {
            Ok(FakeInstance {
                serial: made.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
                broken: false,
            })
        };
        let mut slot = FakeInstance {
            serial: 0,
            broken: true,
        };
        connect_next(&mut slot, create)
            .await
            .expect_err("the broken instance fails");
        assert_eq!(slot.serial, 1, "a fresh instance replaced the broken one");
        let connected = connect_next(&mut slot, create)
            .await
            .expect("the retry connects");
        assert_eq!(connected.serial, 1);
        assert_eq!(slot.serial, 2, "the next instance waits before the handoff");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_silent_child_is_dropped_after_the_handshake_budget() {
        let lookup: Lookup = Arc::new(|_| Err(Refusal::UnknownToken));
        let mut listener =
            Listener::bind_with(lookup, Duration::from_millis(100)).expect("a listener");
        let mut stream = tokio::net::UnixStream::connect(listener.address().as_str())
            .await
            .expect("connect");
        let started = tokio::time::Instant::now();
        let mut rest = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut rest))
            .await
            .expect("dropped well within 5 s")
            .expect("read");
        assert_eq!(read, 0, "dropped without a reply: {rest:?}");
        assert!(started.elapsed() >= Duration::from_millis(100));
        listener.close();
    }
}
