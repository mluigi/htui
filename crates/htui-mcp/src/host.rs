//! `McpHost<H>`: htui's [`ToolHost`] (MOD-11 D5, blueprint §2.9, B-2, B-19, B-21).
//!
//! A registry of live sessions keyed by [`Token`], the process's lazy [`Listener`], and what the
//! tools need beyond a session's own scope (the concept index, the scrubber, the clock). Each
//! [`open`](ToolHost::open) captures the store **once** (B-2: a walk's tool writes go to the server
//! its `Kit` writes to), mints a token and answers a lease whose spec starts `<binary> mcp`; the
//! lease's drop unregisters the token and ends the session (I-6).

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError, Weak};

use htui_agent::driver::McpServerSpec;
use htui_agent::prompt_bridge::PromptAsk;
use htui_core::clock::{Clock, SystemClock};
use htui_core::model::Transport;
use htui_core::scrub::{MinimalScrubber, Scrubber};
use htui_orch::tools::{ToolHost, ToolHostError, ToolLease, ToolScope};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};
use tokio_util::sync::CancellationToken;

use crate::channel::{Address, HandshakeLine, Listener, Lookup, Refusal, Token};
use crate::protocol::{
    CallRefused, CallResult, Handler, Progress, SUPPORTED_VERSIONS, ToolInfo, serve,
};
use crate::search::ConceptSearch;
use crate::tools::{self, Ctx, HostCaps, ToolError};
use crate::{ENV_ADDR, ENV_TOKEN, SERVER_NAME};

/// The in-process client's pipe size (D5).
const CLIENT_PIPE_BYTES: usize = 64 * 1024;

/// Locks `mutex`, through a poison: every critical section here leaves its data whole.
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// htui's MCP tool host over a [`WorkerHost`](htui_core::store::WorkerHost): `PgStore` in the
/// headless worker, `Backend` in the TUI. Cloning shares one registry and one listener.
pub struct McpHost<H: htui_core::store::WorkerHost> {
    inner: Arc<Inner<H>>,
}

/// What every clone of one [`McpHost`] shares.
struct Inner<H: htui_core::store::WorkerHost> {
    /// B-2: the current host; `open` takes its writer, the TUI's store loop replaces it.
    host: StdMutex<H>,
    /// Every live session, by token.
    sessions: StdMutex<HashMap<Token, Arc<Served<H>>>>,
    /// Bound by the first `open` (D3).
    listener: StdMutex<Option<Listener>>,
    /// The builder's settings.
    config: StdMutex<Config>,
}

/// MOD-78 D7: the last `McpHost` is gone. New calls already end through `Bound`'s `Weak`; an
/// in-flight one holds its `Arc<Served>` and ends through the token.
impl<H: htui_core::store::WorkerHost> Drop for Inner<H> {
    fn drop(&mut self) {
        let sessions = self
            .sessions
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        for served in sessions.values() {
            served.session.cancel.cancel();
        }
    }
}

/// What the `with_*` builders set.
struct Config {
    /// D12: `None` leaves `search_concepts` unadvertised.
    search: Option<Arc<dyn ConceptSearch>>,
    /// The absolute binary the agent starts as `<binary> mcp`.
    binary: PathBuf,
    /// Every text a tool persists or returns is scrubbed first (I-5).
    scrubber: Arc<dyn Scrubber>,
    /// The tools' clock.
    clock: Arc<dyn Clock>,
}

/// One session's tool state: its scope, the store it was opened on, and its prompt bridge.
pub(crate) struct Session<S> {
    /// Everything the tools are scoped to (I-1).
    pub(crate) scope: ToolScope,
    /// The store captured at `open` (B-2).
    pub(crate) store: S,
    /// B-21: the asking end of the CLI permission bridge, for a `Transport::Cli` scope.
    pub(crate) ask: Option<PromptAsk>,
    /// MOD-78 D6 (I-6): cancelled when the lease drops, the host closes, or the last `McpHost`
    /// drops (D7). `Served::call` races every call against it, so an in-flight call ends with its
    /// session.
    pub(crate) cancel: CancellationToken,
    /// D12: the concept index, when the host has one.
    pub(crate) search: Option<Arc<dyn ConceptSearch>>,
    /// I-5: the host's scrubber.
    pub(crate) scrubber: Arc<dyn Scrubber>,
    /// The host's clock.
    pub(crate) clock: Arc<dyn Clock>,
    /// What the host offers beyond the store, for `advertised`.
    pub(crate) caps: HostCaps,
    /// MOD-11 R1 L3: one permit — the session's one `command_run` at a time, held for the whole
    /// call (a cancelled call's dropped future gives it back), so one agent cannot fill the
    /// store's connection pool with waiting calls.
    pub(crate) command_slot: Arc<tokio::sync::Semaphore>,
}

impl<S> Session<S> {
    /// Whether the lease dropped, the host closed, or the last `McpHost` dropped.
    fn has_ended(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// The tools this session is offered, in table order (I-7).
    fn advertised(&self) -> impl Iterator<Item = &'static tools::ToolDef> + '_ {
        tools::ALL
            .into_iter()
            .filter(|def| (def.advertised)(&self.scope, &self.caps))
    }
}

/// A session and the host it was opened on; a connection reaches it through a [`Bound`].
struct Served<H: htui_core::store::WorkerHost> {
    session: Arc<Session<H::Store>>,
    host: H,
}

impl<H: htui_core::store::WorkerHost> Handler for Served<H> {
    fn tools(&self) -> Vec<ToolInfo> {
        self.session
            .advertised()
            .map(|def| ToolInfo {
                name: def.name,
                description: def.description,
                input_schema: (def.schema)(),
            })
            .collect()
    }

    fn call(
        &self,
        name: String,
        arguments: Value,
        progress: Option<Progress>,
    ) -> Pin<Box<dyn Future<Output = Result<CallResult, CallRefused>> + Send>> {
        let session = Arc::clone(&self.session);
        let host = self.host.clone();
        Box::pin(async move {
            if session.has_ended() {
                return Ok(session_ended());
            }
            if !session.advertised().any(|def| def.name == name) {
                return Err(CallRefused(format!("unknown tool: {name}")));
            }
            let ctx = Ctx {
                session: &session,
                host,
                progress,
            };
            // MOD-78 D6: an end of the session wins over a ready answer. The dropped call undoes
            // itself: `Enqueued` cancels its row, and `run_shell`'s `GroupGuard` kills the child.
            tokio::select! {
                biased;
                () = session.cancel.cancelled() => Ok(session_ended()),
                result = tools::dispatch(&name, ctx, arguments) => Ok(match result {
                    Ok(value) => CallResult {
                        text: value.to_string(),
                        is_error: false,
                    },
                    Err(ToolError(reason)) => CallResult {
                        text: reason,
                        is_error: true,
                    },
                }),
            }
        })
    }
}

/// What every call to an ended session answers (I-6).
fn session_ended() -> CallResult {
    CallResult {
        text: "session ended".to_owned(),
        is_error: true,
    }
}

/// What a connection (socket or in-process) serves: the session `token` names, found again on
/// every message through a [`Weak`] to the host (§2.9). Dropping the lease or the last
/// [`McpHost`] ends the session for the connection, and the connection keeps neither the store
/// nor the host alive.
struct Bound<H: htui_core::store::WorkerHost> {
    inner: Weak<Inner<H>>,
    token: Token,
}

impl<H: htui_core::store::WorkerHost> Bound<H> {
    /// The session, while both the host and the lease live.
    fn served(&self) -> Option<Arc<Served<H>>> {
        let inner = self.inner.upgrade()?;
        lock(&inner.sessions).get(&self.token).cloned()
    }
}

impl<H: htui_core::store::WorkerHost> Handler for Bound<H> {
    fn tools(&self) -> Vec<ToolInfo> {
        self.served()
            .map(|served| served.tools())
            .unwrap_or_default()
    }

    fn call(
        &self,
        name: String,
        arguments: Value,
        progress: Option<Progress>,
    ) -> Pin<Box<dyn Future<Output = Result<CallResult, CallRefused>> + Send>> {
        match self.served() {
            Some(served) => served.call(name, arguments, progress),
            None => Box::pin(async { Ok(session_ended()) }),
        }
    }
}

impl<H: htui_core::store::WorkerHost> McpHost<H> {
    /// A host over `host`. Resolves the binary once: Linux `/proc/<std::process::id()>/exe`
    /// (valid for the process's life even after the file is replaced, the `provision/mod.rs`
    /// precedent); elsewhere `std::env::current_exe()` (absolute, canonicalised).
    ///
    /// # Errors
    ///
    /// `ToolHostError::Listener` when no absolute binary path can be had.
    pub fn new(host: H) -> Result<Self, ToolHostError> {
        let binary = this_binary()?;
        Ok(Self {
            inner: Arc::new(Inner {
                host: StdMutex::new(host),
                sessions: StdMutex::new(HashMap::new()),
                listener: StdMutex::new(None),
                config: StdMutex::new(Config {
                    search: None,
                    binary,
                    scrubber: Arc::new(MinimalScrubber::new([])),
                    clock: Arc::new(SystemClock),
                }),
            }),
        })
    }

    /// Attaches the concept index `search_concepts` queries (D12).
    #[must_use]
    pub fn with_search(self, search: Arc<dyn ConceptSearch>) -> Self {
        lock(&self.inner.config).search = Some(search);
        self
    }

    /// Replaces the binary the agent starts (tests; a sandbox that cannot see `/proc`, H-11).
    #[must_use]
    pub fn with_binary(self, binary: PathBuf) -> Self {
        lock(&self.inner.config).binary = binary;
        self
    }

    /// Replaces the scrubber (default: `MinimalScrubber::new([])`, the prefix rules only).
    #[must_use]
    pub fn with_scrubber(self, scrubber: Arc<dyn Scrubber>) -> Self {
        lock(&self.inner.config).scrubber = scrubber;
        self
    }

    /// Replaces the clock (default: [`SystemClock`]).
    #[must_use]
    pub fn with_clock(self, clock: Arc<dyn Clock>) -> Self {
        lock(&self.inner.config).clock = clock;
        self
    }

    /// B-2: the host later sessions are opened on. A session already open keeps its own.
    pub fn set_host(&self, host: H) {
        *lock(&self.inner.host) = host;
    }

    /// The listener's address once bound (tests).
    #[must_use]
    pub fn address(&self) -> Option<Address> {
        lock(&self.inner.listener)
            .as_ref()
            .map(|listener| listener.address().clone())
    }

    /// D5's in-process client: the same protocol over `tokio::io::duplex(64 KiB)`, no socket and
    /// no handshake. Must be called inside a tokio runtime.
    ///
    /// # Errors
    ///
    /// `Refusal::UnknownToken` when `token` names no live session.
    pub fn client(&self, token: &str) -> Result<McpClient, Refusal> {
        let handler = resolve(&self.inner, token)?;
        let (ours, theirs) = tokio::io::duplex(CLIENT_PIPE_BYTES);
        htui_agent::contained::spawn(async move {
            if let Err(err) = serve(theirs, handler).await {
                tracing::debug!(error = %err, "htui-mcp: the in-process client's stream failed");
            }
        });
        let (read, write) = tokio::io::split(ours);
        Ok(McpClient {
            lines: BufReader::new(read).lines(),
            write,
            next_id: 0,
        })
    }

    /// The listener's address, binding it on first use (D3).
    fn listener_address(&self) -> Result<Address, ToolHostError> {
        let mut listener = lock(&self.inner.listener);
        if let Some(bound) = listener.as_ref() {
            return Ok(bound.address().clone());
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(ToolHostError::Listener(
                "no async runtime to accept connections on".to_owned(),
            ));
        }
        let weak = Arc::downgrade(&self.inner);
        let lookup: Lookup = Arc::new(move |hello: &HandshakeLine| {
            let inner = weak.upgrade().ok_or(Refusal::UnknownToken)?;
            resolve(&inner, &hello.token)
        });
        let bound =
            Listener::bind(lookup).map_err(|err| ToolHostError::Listener(err.to_string()))?;
        let address = bound.address().clone();
        *listener = Some(bound);
        Ok(address)
    }
}

/// The live session `token` names, as a connection's handler: a [`Bound`] that holds the host
/// weakly.
fn resolve<H: htui_core::store::WorkerHost>(
    inner: &Arc<Inner<H>>,
    token: &str,
) -> Result<Arc<dyn Handler>, Refusal> {
    let token = Token::parse(token).ok_or(Refusal::UnknownToken)?;
    if !lock(&inner.sessions).contains_key(&token) {
        return Err(Refusal::UnknownToken);
    }
    Ok(Arc::new(Bound {
        inner: Arc::downgrade(inner),
        token,
    }))
}

/// This process's binary, absolute (the ACP schema requires it).
fn this_binary() -> Result<PathBuf, ToolHostError> {
    #[cfg(target_os = "linux")]
    {
        Ok(PathBuf::from(format!("/proc/{}/exe", std::process::id())))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let exe = std::env::current_exe().map_err(|err| {
            ToolHostError::Listener(format!("cannot find htui's own binary: {err}"))
        })?;
        let exe = exe.canonicalize().unwrap_or(exe);
        if exe.is_absolute() {
            Ok(exe)
        } else {
            Err(ToolHostError::Listener(format!(
                "htui's own binary has no absolute path: {}",
                exe.display()
            )))
        }
    }
}

impl<H: htui_core::store::WorkerHost> Clone for McpHost<H> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<H: htui_core::store::WorkerHost> core::fmt::Debug for McpHost<H> {
    /// The session count and the address; never a token (I-6).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("McpHost")
            .field("sessions", &lock(&self.inner.sessions).len())
            .field("address", &self.address())
            .finish_non_exhaustive()
    }
}

impl<H: htui_core::store::WorkerHost> ToolHost for McpHost<H> {
    fn open(&self, scope: ToolScope) -> Result<ToolLease, ToolHostError> {
        let host = lock(&self.inner.host).clone();
        let store = htui_core::store::WorkerHost::writer(&host).ok_or(ToolHostError::Offline)?;
        let address = self.listener_address()?;
        let token = Token::mint();
        let (port, ask) = match scope.transport {
            Transport::Cli => {
                let (port, ask) = htui_agent::prompt_bridge::bridge();
                (Some(port), Some(ask))
            }
            Transport::Acp => (None, None),
        };
        let (binary, session) = {
            let config = lock(&self.inner.config);
            let session = Session {
                scope,
                store,
                ask,
                cancel: CancellationToken::new(),
                search: config.search.clone(),
                scrubber: Arc::clone(&config.scrubber),
                clock: Arc::clone(&config.clock),
                caps: HostCaps {
                    search: config.search.is_some(),
                },
                command_slot: Arc::new(tokio::sync::Semaphore::new(1)),
            };
            (config.binary.clone(), Arc::new(session))
        };
        lock(&self.inner.sessions).insert(
            token.clone(),
            Arc::new(Served {
                session: Arc::clone(&session),
                host,
            }),
        );
        let spec = McpServerSpec {
            name: SERVER_NAME.to_owned(),
            command: binary.to_string_lossy().into_owned(),
            args: vec!["mcp".to_owned()],
            env: BTreeMap::from([
                (ENV_ADDR.to_owned(), address.as_str().to_owned()),
                (ENV_TOKEN.to_owned(), token.as_str().to_owned()),
            ]),
        };
        // MOD-11 R1 M2: the advertised names, which the engine and the chat pre-approve.
        let advertised = session
            .advertised()
            .map(|def| def.name.to_owned())
            .collect();
        let weak: Weak<Inner<H>> = Arc::downgrade(&self.inner);
        let cancel = session.cancel.clone();
        Ok(ToolLease::new(spec, port, move || {
            cancel.cancel();
            if let Some(inner) = weak.upgrade() {
                lock(&inner.sessions).remove(&token);
            }
        })
        .with_tools(advertised))
    }

    fn close(&self) {
        if let Some(mut listener) = lock(&self.inner.listener).take() {
            listener.close();
        }
        for (_, served) in lock(&self.inner.sessions).drain() {
            served.session.cancel.cancel();
        }
    }
}

/// The client half [`McpHost::client`] returns.
pub struct McpClient {
    lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
    write: WriteHalf<DuplexStream>,
    next_id: u64,
}

impl core::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("McpClient")
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

impl McpClient {
    /// One raw request; answers the whole response object (`result` or `error`). Notifications
    /// that arrive first (progress) are skipped.
    ///
    /// # Errors
    ///
    /// The stream's I/O failure, or its end before the answer.
    pub async fn request(&mut self, method: &str, params: Value) -> std::io::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send(&message).await?;
        loop {
            let Some(line) = self.lines.next_line().await? else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "the MCP session closed before answering",
                ));
            };
            let answer: Value = serde_json::from_str(&line).map_err(std::io::Error::other)?;
            if answer.get("id") == Some(&json!(id)) {
                return Ok(answer);
            }
        }
    }

    /// `initialize` with the newest revision, then `notifications/initialized`.
    ///
    /// # Errors
    ///
    /// As [`request`](Self::request).
    pub async fn initialize(&mut self) -> std::io::Result<Value> {
        let answer = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": SUPPORTED_VERSIONS[0],
                    "capabilities": {},
                    "clientInfo": {"name": "htui-in-process", "version": env!("CARGO_PKG_VERSION")},
                }),
            )
            .await?;
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await?;
        Ok(answer)
    }

    /// The advertised names, in table order.
    ///
    /// # Errors
    ///
    /// As [`request`](Self::request).
    pub async fn tool_names(&mut self) -> std::io::Result<Vec<String>> {
        let answer = self.request("tools/list", json!({})).await?;
        Ok(answer["result"]["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect())
    }

    /// `tools/call`; `Err` only on transport failure; a refusal is `CallResult { is_error: true }`,
    /// an unknown tool `Ok(CallResult { is_error: true, text: "<-32602 message>" })`.
    ///
    /// # Errors
    ///
    /// As [`request`](Self::request).
    pub async fn call(&mut self, tool: &str, arguments: Value) -> std::io::Result<CallResult> {
        let answer = self
            .request("tools/call", json!({"name": tool, "arguments": arguments}))
            .await?;
        if let Some(error) = answer.get("error") {
            return Ok(CallResult {
                text: error["message"].as_str().unwrap_or_default().to_owned(),
                is_error: true,
            });
        }
        let result = &answer["result"];
        Ok(CallResult {
            text: result["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            is_error: result["isError"].as_bool().unwrap_or(false),
        })
    }

    /// Writes one message line.
    async fn send(&mut self, message: &Value) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(message).map_err(std::io::Error::other)?;
        line.push(b'\n');
        self.write.write_all(&line).await?;
        self.write.flush().await
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use chrono::Utc;
    use htui_core::fixtures::ids;
    use htui_core::model::{
        CommandRunId, CommandRunStatus, NewCommandRun, RunId, StepId, Transport,
    };
    use htui_core::prompt::render::HostnameLine;
    // Blueprint B-8: `WriteStore` only; with `WorkerStore` too, `MemStore`'s calls are E0034.
    use htui_core::store::{MemStore, StepFence, WriteStore};
    use htui_orch::tools::{ToolHost, ToolHostError, ToolLease, ToolScope};
    use htui_store::Backend;
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    use super::{McpHost, lock};
    use crate::channel::{Refusal, Token};
    use crate::protocol::CallResult;
    use crate::{ENV_ADDR, ENV_TOKEN};

    /// A demo-store host (blueprint B-1).
    pub(crate) fn demo_host() -> McpHost<Backend> {
        McpHost::new(Backend::memory(MemStore::demo())).expect("a host")
    }

    /// A scope on the demo box, no item.
    pub(crate) fn scope(transport: Transport) -> ToolScope {
        ToolScope {
            run_id: RunId::new(),
            step_id: StepId::new(),
            project_id: ids::PROJECT_HTUI,
            item_id: None,
            box_id: ids::BOX,
            user: ids::USER,
            fence: StepFence::Unleased,
            output_kind: None,
            hostname: HostnameLine::Omitted,
            command_queue: false,
            cwd: std::env::temp_dir(),
            transport,
        }
    }

    #[tokio::test]
    async fn the_spec_names_htui_the_binary_mcp_and_two_env_vars() {
        let host = demo_host();
        let lease = host.open(scope(Transport::Acp)).expect("a lease");
        let spec = &lease.spec;
        assert_eq!(spec.name, "htui");
        assert_eq!(
            PathBuf::from(&spec.command),
            lock(&host.inner.config).binary
        );
        assert_eq!(spec.args, ["mcp"]);
        let keys: Vec<&str> = spec.env.keys().map(String::as_str).collect();
        assert_eq!(keys, [ENV_ADDR, ENV_TOKEN]);
        assert_eq!(
            Some(spec.env[ENV_ADDR].as_str()),
            host.address().as_ref().map(|a| a.as_str())
        );
        assert!(Token::parse(&spec.env[ENV_TOKEN]).is_some());
    }

    #[tokio::test]
    async fn two_opens_mint_two_tokens() {
        let host = demo_host();
        let a = host.open(scope(Transport::Acp)).expect("a lease");
        let b = host.open(scope(Transport::Acp)).expect("a lease");
        assert_ne!(a.spec.env[ENV_TOKEN], b.spec.env[ENV_TOKEN]);
        assert_eq!(a.spec.env[ENV_ADDR], b.spec.env[ENV_ADDR], "one listener");
        assert_eq!(lock(&host.inner.sessions).len(), 2);
        drop(a);
        assert_eq!(
            lock(&host.inner.sessions).len(),
            1,
            "a drop unregisters its own"
        );
    }

    #[tokio::test]
    async fn a_cli_scope_gets_a_prompt_port_and_an_acp_scope_none() {
        let host = demo_host();
        let cli = host.open(scope(Transport::Cli)).expect("a lease");
        let acp = host.open(scope(Transport::Acp)).expect("a lease");
        assert!(acp.prompt.is_none());
        let port = cli.prompt.as_ref().expect("a CLI scope gets a port");
        let token = Token::parse(&cli.spec.env[ENV_TOKEN]).expect("a token");
        let sessions = lock(&host.inner.sessions);
        let ask = sessions[&token]
            .session
            .ask
            .as_ref()
            .expect("the session keeps the ask");
        assert_eq!(ask.id(), port.id(), "one bridge");
        let acp_token = Token::parse(&acp.spec.env[ENV_TOKEN]).expect("a token");
        assert!(sessions[&acp_token].session.ask.is_none());
    }

    /// MOD-11 R1 M2: the lease names exactly the tools the scope is advertised, in table order,
    /// so the engine and the chat pre-approve no tool the session cannot call.
    #[tokio::test]
    async fn the_lease_names_the_advertised_tools() {
        let host = demo_host();
        let acp = host.open(scope(Transport::Acp)).expect("a lease");
        assert_eq!(
            acp.tools,
            ["box_profile"],
            "no item, no queue, no prompt tool"
        );
        let cli = host
            .open(ToolScope {
                item_id: Some(ids::HTUI_FEAT_1),
                output_kind: Some("plan".to_owned()),
                command_queue: true,
                ..scope(Transport::Cli)
            })
            .expect("a lease");
        assert_eq!(
            cli.tools,
            [
                "box_profile",
                "document_write",
                "note_add",
                "item_status",
                "item_link",
                "command_run",
                "permission_prompt",
            ]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn on_linux_the_binary_is_proc_pid_exe() {
        let host = demo_host();
        assert_eq!(
            lock(&host.inner.config).binary,
            PathBuf::from(format!("/proc/{}/exe", std::process::id()))
        );
    }

    #[test]
    fn the_binary_is_absolute() {
        let host = demo_host();
        assert!(lock(&host.inner.config).binary.is_absolute());
    }

    #[tokio::test]
    async fn a_session_keeps_the_store_it_was_opened_on() {
        let host = demo_host();
        let first = host.open(scope(Transport::Acp)).expect("a lease");
        host.set_host(Backend::memory(MemStore::new()));
        let second = host.open(scope(Transport::Acp)).expect("a lease");
        let sessions = lock(&host.inner.sessions).clone();
        let on = |lease: &ToolLease| {
            let token = Token::parse(&lease.spec.env[ENV_TOKEN]).expect("a token");
            sessions[&token].session.store.clone()
        };
        let old = htui_core::store::WorkerStore::project(&on(&first), ids::PROJECT_HTUI)
            .await
            .expect("a read");
        assert!(
            old.is_some(),
            "the first session still reads the demo store"
        );
        let new = htui_core::store::WorkerStore::project(&on(&second), ids::PROJECT_HTUI)
            .await
            .expect("a read");
        assert!(new.is_none(), "the second reads the store set after it");
    }

    #[tokio::test]
    async fn client_refuses_an_unknown_token() {
        let host = demo_host();
        let lease = host.open(scope(Transport::Acp)).expect("a lease");
        let stranger = Token::mint();
        assert_eq!(
            host.client(stranger.as_str()).map(|_| ()),
            Err(Refusal::UnknownToken)
        );
        assert_eq!(
            host.client("not a token").map(|_| ()),
            Err(Refusal::UnknownToken)
        );
        let token = lease.spec.env[ENV_TOKEN].clone();
        let mut client = host.client(&token).expect("a live session");
        let answer = client.initialize().await.expect("initialize");
        assert_eq!(answer["result"]["serverInfo"]["name"], "htui");
        drop(lease);
        assert_eq!(host.client(&token).map(|_| ()), Err(Refusal::UnknownToken));
        let ended = client
            .call("box_profile", serde_json::json!({}))
            .await
            .expect("a call");
        assert!(ended.is_error);
        assert_eq!(ended.text, "session ended");
    }

    /// §2.9: the in-process client's server task holds the host weakly; dropping the last
    /// `McpHost` ends the session even while its lease and its client live.
    #[tokio::test]
    async fn dropping_the_last_host_ends_an_in_process_client() {
        let host = demo_host();
        let lease = host.open(scope(Transport::Acp)).expect("a lease");
        let mut client = host
            .client(&lease.spec.env[ENV_TOKEN])
            .expect("a live session");
        client.initialize().await.expect("initialize");
        let live = client
            .call("box_profile", serde_json::json!({}))
            .await
            .expect("a call");
        assert!(!live.is_error, "{}", live.text);

        let inner = std::sync::Arc::downgrade(&host.inner);
        drop(host);
        assert!(
            inner.upgrade().is_none(),
            "nothing else keeps the host alive"
        );
        let ended = client
            .call("box_profile", serde_json::json!({}))
            .await
            .expect("a call");
        assert!(ended.is_error);
        assert_eq!(ended.text, "session ended");
        assert!(
            client.tool_names().await.expect("tools/list").is_empty(),
            "an ended session offers nothing"
        );
        drop(lease);
    }

    /// The bound every MOD-78 in-flight wait gets.
    const IN_FLIGHT_BOUND: Duration = Duration::from_secs(5);

    /// Polls `step`'s `command_run` rows until one other than `skip` has `status`, within
    /// [`IN_FLIGHT_BOUND`]; answers its id.
    async fn until_row(
        store: &MemStore,
        skip: CommandRunId,
        status: CommandRunStatus,
    ) -> CommandRunId {
        tokio::time::timeout(IN_FLIGHT_BOUND, async {
            loop {
                let rows = store
                    .command_runs(ids::STEP_R2_PRD)
                    .await
                    .expect("the step's rows");
                if let Some(row) = rows
                    .iter()
                    .find(|row| row.id != skip && row.status == status)
                {
                    return row.id;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no {status} row within {IN_FLIGHT_BOUND:?}"))
    }

    /// MOD-78 T2 (blueprint §3): a `command_run` in flight that can never be admitted. Another
    /// claimant holds the box's only `build` slot, so the call waits in `admit` for ever. Answers
    /// the store, the host, the session's lease, the spawned call, and the id of the call's
    /// `queued` row.
    async fn queued_call() -> (
        MemStore,
        McpHost<Backend>,
        ToolLease,
        JoinHandle<std::io::Result<CallResult>>,
        CommandRunId,
    ) {
        let store = MemStore::demo();
        let host = McpHost::new(Backend::memory(store.clone())).expect("a host");
        let held = store
            .enqueue_command(NewCommandRun {
                id: CommandRunId::new(),
                run_step_id: ids::STEP_R2_PRD,
                box_id: ids::BOX,
                class: "build".to_owned(),
                command: "make".to_owned(),
                cwd: "/srv".to_owned(),
                status: CommandRunStatus::Queued,
                exit_code: None,
                output: None,
                queued_at: Utc::now(),
                started_at: None,
                finished_at: None,
            })
            .await
            .expect("queued");
        assert!(
            store
                .claim_command(held.id, Uuid::now_v7(), 1)
                .await
                .expect("a claim")
                .is_some(),
            "another claimant holds the one build slot"
        );
        // Blueprint B-5: a run and step the store knows, not `scope()`'s fresh ids.
        let lease = host
            .open(ToolScope {
                run_id: ids::RUN_2,
                step_id: ids::STEP_R2_PRD,
                command_queue: true,
                ..scope(Transport::Acp)
            })
            .expect("a lease");
        let mut client = host
            .client(&lease.spec.env[ENV_TOKEN])
            .expect("a live session");
        client.initialize().await.expect("initialize");
        let call = htui_agent::contained::spawn(async move {
            client
                .call(
                    "command_run",
                    serde_json::json!({"class": "build", "command": "true"}),
                )
                .await
        });
        let ours = until_row(&store, held.id, CommandRunStatus::Queued).await;
        (store, host, lease, call, ours)
    }

    /// Asserts `call` answers `session ended` within [`IN_FLIGHT_BOUND`], then that the dropped
    /// call's row `ours` ends `cancelled` (`Enqueued`'s drop cancels it in a spawned task).
    async fn ends_with_its_session(
        store: &MemStore,
        call: JoinHandle<std::io::Result<CallResult>>,
        ours: CommandRunId,
    ) {
        let answer = tokio::time::timeout(IN_FLIGHT_BOUND, call)
            .await
            .expect("the in-flight call ends with its session")
            .expect("the call task")
            .expect("an answer");
        assert!(answer.is_error, "{}", answer.text);
        assert_eq!(answer.text, "session ended");
        let cancelled = tokio::time::timeout(IN_FLIGHT_BOUND, async {
            loop {
                let rows = store
                    .command_runs(ids::STEP_R2_PRD)
                    .await
                    .expect("the step's rows");
                let row = rows.iter().find(|row| row.id == ours).expect("our row");
                if row.status == CommandRunStatus::Cancelled {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(cancelled.is_ok(), "our row ends cancelled");
    }

    /// MOD-78 R3, D6: dropping the lease ends a call already in flight, not only later calls.
    #[tokio::test]
    async fn an_in_flight_call_ends_when_its_lease_drops() {
        let (store, host, lease, call, ours) = queued_call().await;
        drop(lease);
        ends_with_its_session(&store, call, ours).await;
        drop(host);
    }

    /// MOD-78 R3, D6: `close()` ends a call already in flight.
    #[tokio::test]
    async fn an_in_flight_call_ends_when_the_host_closes() {
        let (store, host, lease, call, ours) = queued_call().await;
        host.close();
        ends_with_its_session(&store, call, ours).await;
        drop(lease);
    }

    /// MOD-78 D7: dropping the last `McpHost` ends a call already in flight, which holds its
    /// session's `Served` and so outlives the `Weak` in `Bound`.
    #[tokio::test]
    async fn an_in_flight_call_ends_with_the_last_host() {
        let (store, host, lease, call, ours) = queued_call().await;
        let inner = std::sync::Arc::downgrade(&host.inner);
        drop(host);
        assert!(
            inner.upgrade().is_none(),
            "nothing else keeps the host alive"
        );
        ends_with_its_session(&store, call, ours).await;
        drop(lease);
    }

    /// I-7: a fresh chat (no item, no command queue, ACP, no concept index) is offered
    /// `box_profile` only; `McpHost` itself refuses every other name with -32602, before any tool
    /// code runs.
    #[tokio::test]
    async fn an_unadvertised_tool_is_refused_by_the_host_with_minus_32602() {
        let host = demo_host();
        let lease = host.open(scope(Transport::Acp)).expect("a lease");
        let mut client = host
            .client(&lease.spec.env[ENV_TOKEN])
            .expect("a live session");
        client.initialize().await.expect("initialize");
        assert_eq!(
            client.tool_names().await.expect("tools/list"),
            ["box_profile"]
        );
        for name in crate::tools::ALL
            .iter()
            .map(|def| def.name)
            .filter(|name| *name != "box_profile")
            .chain(["no_such_tool"])
        {
            let answer = client
                .request(
                    "tools/call",
                    serde_json::json!({"name": name, "arguments": {}}),
                )
                .await
                .expect("an answer");
            assert_eq!(answer["error"]["code"], -32602, "{name}: {answer}");
            assert_eq!(
                answer["error"]["message"],
                format!("unknown tool: {name}"),
                "{name}"
            );
            assert!(answer.get("result").is_none(), "{name}: {answer}");
        }
    }

    #[test]
    fn open_outside_a_runtime_is_a_listener_error() {
        let host = demo_host();
        assert!(matches!(
            host.open(scope(Transport::Acp)),
            Err(ToolHostError::Listener(_))
        ));
        assert_eq!(host.address(), None);
        assert!(lock(&host.inner.sessions).is_empty(), "nothing registered");
    }

    #[tokio::test]
    async fn an_offline_host_answers_offline() {
        let root = tempfile::tempdir().expect("a dir");
        let cache = htui_store::CacheStore::open(root.path(), "offline-host", 1)
            .await
            .expect("a mirror");
        let host = McpHost::new(Backend::Offline { cache, since: None }).expect("a host");
        assert_eq!(
            host.open(scope(Transport::Acp)).map(|_| ()),
            Err(ToolHostError::Offline)
        );
        assert_eq!(host.address(), None, "no listener for nothing");
    }
}
