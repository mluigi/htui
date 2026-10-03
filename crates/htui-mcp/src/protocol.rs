//! JSON-RPC 2.0 / MCP over newline-delimited JSON (MOD-11 D2, blueprint §2.7, B-10, B-11).
//!
//! [`serve`] answers one connection — a Unix socket, a named pipe or an in-process duplex — until
//! the client closes it. The server side of MCP that htui needs is small: `initialize`, `ping`,
//! `tools/list` and `tools/call`, plus two notifications it reads (`notifications/initialized`,
//! ignored, and `notifications/cancelled`, which aborts a call). Everything about *which* tools
//! exist and what they do belongs to a [`Handler`].
//!
//! **Concurrency.** Each `tools/call` runs in its own task (`htui_agent::contained::spawn_in`,
//! blueprint B-20), so a thirty-minute `command_run` never blocks a `ping`. Every answer goes
//! through one writer, in the order the answers complete. `notifications/cancelled` aborts that
//! request's task: whatever a tool must undo lives in a drop guard (blueprint H-18). A client that
//! reuses an id still in flight (it must not) gets both calls served, and a cancel of that id
//! aborts both. A tool that panics is answered `-32603`; the panic stays inside its task. The end
//! of the input aborts every task still in flight; nothing is answered for them.
//!
//! **Robustness.** No input panics the loop. A line that is not JSON is `-32700`, a JSON value
//! that is not a request object is `-32600`, a line over [`MAX_LINE_BYTES`] is discarded up to its
//! newline and answered `-32700`, and the next line is served either way.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, Id, JoinSet};

/// The protocol revisions this server speaks, newest first (blueprint B-10). Every method it
/// answers is unchanged across the four.
pub const SUPPORTED_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The most one input line may hold, newline excluded; mirrors the CLI transport's stdout cap
/// (`htui-agent/src/cli/mod.rs`).
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// JSON-RPC: the line is not JSON, or is too long to read.
const PARSE_ERROR: i64 = -32700;
/// JSON-RPC: JSON, but not a request object.
const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC: no such method.
const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC: the method's parameters are wrong; MCP uses it for an unknown tool.
const INVALID_PARAMS: i64 = -32602;
/// JSON-RPC: the server failed; here, the tool's task panicked.
const INTERNAL_ERROR: i64 = -32603;

/// How many outgoing messages may wait for the writer.
const OUTBOX: usize = 64;

/// What the protocol loop asks of whoever owns the tools: `McpHost`'s session, or a test double.
pub trait Handler: Send + Sync + 'static {
    /// `tools/list`'s `tools` array, already filtered to the advertised set (I-7).
    fn tools(&self) -> Vec<ToolInfo>;

    /// One call. `progress` is `Some` when the client sent `_meta.progressToken` (B-11).
    fn call(
        &self,
        name: String,
        arguments: Value,
        progress: Option<Progress>,
    ) -> Pin<Box<dyn Future<Output = Result<CallResult, CallRefused>> + Send>>;
}

/// One entry of `tools/list`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ToolInfo {
    /// The tool's name, as the agent calls it.
    pub name: &'static str,
    /// One sentence the agent reads.
    pub description: &'static str,
    /// A JSON Schema of the arguments object.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// A tool's answer: `{content:[{type:"text",text}], isError}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallResult {
    /// The one text block.
    pub text: String,
    /// Whether the tool refused or failed (the MCP convention: a tool error is a result).
    pub is_error: bool,
}

/// A call the protocol refuses rather than the tool: unknown or unadvertised name (-32602). The
/// string is the error's whole `message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRefused(pub String);

/// Sends `notifications/progress {progressToken, progress}` on the connection's writer (B-11).
#[derive(Debug, Clone)]
pub struct Progress {
    token: Value,
    tx: mpsc::Sender<Value>,
}

impl Progress {
    /// Reports `progress` under the client's token. A closed connection is not an error: the
    /// tick is dropped.
    pub async fn tick(&self, progress: u64) {
        let message = json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": {"progressToken": self.token, "progress": progress},
        });
        let _ = self.tx.send(message).await;
    }
}

/// Serves one connection until EOF; never panics on input. Requests are dispatched concurrently
/// (`contained::spawn_in` into a `JoinSet`, B-20); responses go through one writer task in the
/// order they complete. `notifications/cancelled {requestId}` aborts that request's task (its drop
/// guards run: `command_run` cancels its row). A call whose task panics is answered `-32603`. EOF
/// aborts every in-flight task.
///
/// # Errors
///
/// The connection's own I/O failure, reading or writing.
pub async fn serve<C>(conn: C, handler: Arc<dyn Handler>) -> std::io::Result<()>
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (read, write) = tokio::io::split(conn);
    let (out_tx, out_rx) = mpsc::channel::<Value>(OUTBOX);
    let (line_tx, line_rx) = mpsc::channel::<Line>(1);
    let (read_result, (), write_result) = tokio::join!(
        read_lines(read, line_tx),
        dispatch(line_rx, out_tx, handler),
        write_messages(write, out_rx),
    );
    read_result.and(write_result)
}

/// One input line, or the fact that one was too long.
enum Line {
    /// The bytes before the newline (a trailing `\r` removed).
    Text(Vec<u8>),
    /// The line exceeded [`MAX_LINE_BYTES`]; its bytes were discarded up to the newline.
    TooLong,
}

/// Splits the input into lines, never holding more than [`MAX_LINE_BYTES`] + 1 of one.
async fn read_lines<R: AsyncRead>(read: R, lines: mpsc::Sender<Line>) -> std::io::Result<()> {
    let mut reader = Box::pin(BufReader::new(read));
    let limit = u64::try_from(MAX_LINE_BYTES).unwrap_or(u64::MAX) + 1;
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = (&mut reader)
            .take(limit)
            .read_until(b'\n', &mut buffer)
            .await?;
        if read == 0 {
            return Ok(());
        }
        let line = if buffer.last() == Some(&b'\n') {
            buffer.pop();
            if buffer.last() == Some(&b'\r') {
                buffer.pop();
            }
            Line::Text(std::mem::take(&mut buffer))
        } else if buffer.len() > MAX_LINE_BYTES {
            discard_to_newline(&mut reader).await?;
            Line::TooLong
        } else {
            // The last line, unterminated.
            Line::Text(std::mem::take(&mut buffer))
        };
        if lines.send(line).await.is_err() {
            return Ok(());
        }
    }
}

/// Reads and drops bytes up to and including the next newline (or EOF).
async fn discard_to_newline<R: AsyncBufReadExt + Unpin>(reader: &mut R) -> std::io::Result<()> {
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Ok(());
        }
        if let Some(at) = chunk.iter().position(|b| *b == b'\n') {
            reader.consume(at + 1);
            return Ok(());
        }
        let len = chunk.len();
        reader.consume(len);
    }
}

/// Writes each message as one line until every sender is gone.
async fn write_messages<W: AsyncWrite>(
    write: W,
    mut messages: mpsc::Receiver<Value>,
) -> std::io::Result<()> {
    let mut write = Box::pin(write);
    while let Some(message) = messages.recv().await {
        let mut line = serde_json::to_vec(&message).map_err(std::io::Error::other)?;
        line.push(b'\n');
        if let Err(err) = async {
            write.write_all(&line).await?;
            write.flush().await
        }
        .await
        {
            // Nobody reads any more: stop taking messages so senders see the channel close.
            messages.close();
            return Err(err);
        }
    }
    write.shutdown().await.or(Ok(()))
}

/// One `tools/call` task still running, keyed in the in-flight map by its task id.
struct InFlight {
    /// [`request_key`] of its request id: what `notifications/cancelled` names.
    key: String,
    /// The request id, for the `-32603` answer should the task panic.
    id: Value,
    /// Aborts the task.
    abort: AbortHandle,
}

/// Answers lines as they arrive and reaps finished calls; aborts the rest at EOF.
async fn dispatch(
    mut lines: mpsc::Receiver<Line>,
    out: mpsc::Sender<Value>,
    handler: Arc<dyn Handler>,
) {
    let mut calls: JoinSet<()> = JoinSet::new();
    // By task id, not request id: two calls under one reused id are two entries, and reaping one
    // never forgets the other.
    let mut in_flight: HashMap<Id, InFlight> = HashMap::new();
    loop {
        tokio::select! {
            line = lines.recv() => {
                let Some(line) = line else { break };
                let answer = match line {
                    Line::TooLong => Some(error(Value::Null, PARSE_ERROR, "line exceeds 1 MiB")),
                    Line::Text(bytes) => handle_line(
                        &bytes,
                        &handler,
                        &out,
                        &mut calls,
                        &mut in_flight,
                    ),
                };
                if let Some(answer) = answer
                    && out.send(answer).await.is_err()
                {
                    break;
                }
            }
            Some(done) = calls.join_next_with_id(), if !calls.is_empty() => {
                let (task, panicked) = match done {
                    Ok((task, ())) => (task, false),
                    Err(err) => (err.id(), err.is_panic()),
                };
                // A cancelled call left the map when it was cancelled: it stays unanswered.
                if let Some(call) = in_flight.remove(&task)
                    && panicked
                    && out
                        .send(error(call.id, INTERNAL_ERROR, "internal error: the tool panicked"))
                        .await
                        .is_err()
                {
                    break;
                }
            }
        }
    }
    calls.abort_all();
}

/// One line: answered inline (`Some`), by a spawned call later, or never (a notification).
fn handle_line(
    bytes: &[u8],
    handler: &Arc<dyn Handler>,
    out: &mpsc::Sender<Value>,
    calls: &mut JoinSet<()>,
    in_flight: &mut HashMap<Id, InFlight>,
) -> Option<Value> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let Ok(message) = serde_json::from_slice::<Value>(bytes) else {
        return Some(error(Value::Null, PARSE_ERROR, "parse error"));
    };
    let Value::Object(mut message) = message else {
        return Some(error(
            Value::Null,
            INVALID_REQUEST,
            "invalid request: not a request object (batches are not supported)",
        ));
    };
    let id = message.remove("id");
    let reply_id = id.clone().unwrap_or(Value::Null);
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error(
            reply_id,
            INVALID_REQUEST,
            "invalid request: jsonrpc must be \"2.0\"",
        ));
    }
    let Some(Value::String(method)) = message.remove("method") else {
        return Some(error(
            reply_id,
            INVALID_REQUEST,
            "invalid request: no method",
        ));
    };
    let params = message.remove("params").unwrap_or(Value::Null);
    let Some(id) = id else {
        notification(&method, &params, in_flight);
        return None;
    };
    match method.as_str() {
        "initialize" => Some(result(id, initialize(&params))),
        "ping" => Some(result(id, json!({}))),
        "tools/list" => Some(result(id, json!({"tools": handler.tools()}))),
        "tools/call" => call(id, params, handler, out, calls, in_flight),
        other => Some(error(
            id,
            METHOD_NOT_FOUND,
            &format!("method not found: {other}"),
        )),
    }
}

/// The two notifications that mean something; every other one is ignored. A cancel aborts every
/// call in flight under that request id and forgets it, so nothing is answered for it.
fn notification(method: &str, params: &Value, in_flight: &mut HashMap<Id, InFlight>) {
    if method == "notifications/cancelled"
        && let Some(request) = params.get("requestId")
    {
        let key = request_key(request);
        in_flight.retain(|_, call| {
            let cancelled = call.key == key;
            if cancelled {
                call.abort.abort();
            }
            !cancelled
        });
    }
}

/// `initialize`: the client's revision when this server speaks it, else the newest.
fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = SUPPORTED_VERSIONS
        .into_iter()
        .find(|v| Some(*v) == asked)
        .unwrap_or(SUPPORTED_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": crate::SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
    })
}

/// `tools/call`: spawns the handler's call and answers `None`; the task answers when it ends.
fn call(
    id: Value,
    params: Value,
    handler: &Arc<dyn Handler>,
    out: &mpsc::Sender<Value>,
    calls: &mut JoinSet<()>,
    in_flight: &mut HashMap<Id, InFlight>,
) -> Option<Value> {
    let Value::Object(mut params) = params else {
        return Some(error(
            id,
            INVALID_PARAMS,
            "invalid params: tools/call needs an object",
        ));
    };
    let Some(Value::String(name)) = params.remove("name") else {
        return Some(error(
            id,
            INVALID_PARAMS,
            "invalid params: tools/call needs a name",
        ));
    };
    let arguments = match params.remove("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(arguments) => arguments,
    };
    let progress = params
        .get("_meta")
        .and_then(|meta| meta.get("progressToken"))
        .filter(|token| !token.is_null())
        .map(|token| Progress {
            token: token.clone(),
            tx: out.clone(),
        });
    let key = request_key(&id);
    let future = handler.call(name, arguments, progress);
    let out = out.clone();
    let reply_id = id.clone();
    let abort = htui_agent::contained::spawn_in(calls, async move {
        let answer = match future.await {
            Ok(done) => result(
                id,
                json!({
                    "content": [{"type": "text", "text": done.text}],
                    "isError": done.is_error,
                }),
            ),
            Err(CallRefused(message)) => error(id, INVALID_PARAMS, &message),
        };
        let _ = out.send(answer).await;
    });
    in_flight.insert(
        abort.id(),
        InFlight {
            key,
            id: reply_id,
            abort,
        },
    );
    None
}

/// A request id as a map key: its JSON text, so `5` and `"5"` stay distinct.
fn request_key(id: &Value) -> String {
    id.to_string()
}

/// A success response.
fn result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// An error response.
fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}
