//! `protocol::serve` against a `Handler` double over `tokio::io::duplex` (MOD-11 D2, blueprint
//! §2.7, B-10, B-11): the recorded claude transcript, version negotiation, every error code, and
//! the concurrency the serve loop promises (a slow call blocks nothing; a cancelled one is
//! aborted and answered nothing; progress ticks carry the client's token).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use htui_mcp::protocol::{
    CallRefused, CallResult, Handler, MAX_LINE_BYTES, Progress, SUPPORTED_VERSIONS, ToolInfo, serve,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, WriteHalf};
use tokio::sync::mpsc;

/// What the double reports from inside a call, so a test can wait for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    SlowStarted,
    SlowDropped,
}

/// Sends [`Seen::SlowDropped`] when the slow call's future is dropped (finished or aborted).
struct DropGuard(mpsc::UnboundedSender<Seen>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        let _ = self.0.send(Seen::SlowDropped);
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EchoArgs {
    text: String,
}

/// Five tools: `echo` (decodes its arguments), `slow` (never finishes on its own), `ticker`
/// (two progress ticks, then `done`), `permission_prompt` (the transcript's call) and `panic`
/// (panics; not advertised).
struct Double {
    seen: mpsc::UnboundedSender<Seen>,
}

impl Handler for Double {
    fn tools(&self) -> Vec<ToolInfo> {
        ["echo", "slow", "ticker", "permission_prompt"]
            .into_iter()
            .map(|name| ToolInfo {
                name,
                description: "A test tool.",
                input_schema: json!({"type": "object"}),
            })
            .collect()
    }

    fn call(
        &self,
        name: String,
        arguments: Value,
        progress: Option<Progress>,
    ) -> Pin<Box<dyn Future<Output = Result<CallResult, CallRefused>> + Send>> {
        let seen = self.seen.clone();
        Box::pin(async move {
            match name.as_str() {
                "echo" => Ok(match serde_json::from_value::<EchoArgs>(arguments) {
                    Ok(args) => CallResult {
                        text: args.text,
                        is_error: false,
                    },
                    Err(err) => CallResult {
                        text: format!("invalid arguments: {err}"),
                        is_error: true,
                    },
                }),
                "slow" => {
                    let _guard = DropGuard(seen.clone());
                    let _ = seen.send(Seen::SlowStarted);
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    Ok(CallResult {
                        text: "slept".to_owned(),
                        is_error: false,
                    })
                }
                "ticker" => {
                    if let Some(progress) = progress {
                        progress.tick(1).await;
                        progress.tick(2).await;
                    }
                    Ok(CallResult {
                        text: "done".to_owned(),
                        is_error: false,
                    })
                }
                "permission_prompt" => Ok(CallResult {
                    text: json!({"behavior": "allow", "updatedInput": {}}).to_string(),
                    is_error: false,
                }),
                "panic" => panic!("the double's tool panics"),
                other => Err(CallRefused(format!("unknown tool: {other}"))),
            }
        })
    }
}

/// The client's end of one served connection.
struct Client {
    lines: Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
    write: WriteHalf<DuplexStream>,
    seen: mpsc::UnboundedReceiver<Seen>,
    served: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Client {
    fn start() -> Self {
        let (ours, theirs) = tokio::io::duplex(4 * 1024 * 1024);
        let (tx, seen) = mpsc::unbounded_channel();
        let handler: Arc<dyn Handler> = Arc::new(Double { seen: tx });
        let served = htui_agent::contained::spawn(serve(theirs, handler));
        let (read, write) = tokio::io::split(ours);
        Self {
            lines: BufReader::new(read).lines(),
            write,
            seen,
            served,
        }
    }

    async fn send_raw(&mut self, line: &[u8]) {
        self.write.write_all(line).await.expect("write");
        self.write.write_all(b"\n").await.expect("write");
        self.write.flush().await.expect("flush");
    }

    async fn send(&mut self, message: Value) {
        self.send_raw(message.to_string().as_bytes()).await;
    }

    async fn recv(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(10), self.lines.next_line())
            .await
            .expect("an answer within 10 s")
            .expect("read")
            .expect("a line, not EOF");
        serde_json::from_str(&line).expect("the server writes JSON")
    }

    async fn seen(&mut self) -> Seen {
        tokio::time::timeout(Duration::from_secs(10), self.seen.recv())
            .await
            .expect("the double reports within 10 s")
            .expect("the double is alive")
    }

    /// Closes our write half and answers every line the server writes before it ends.
    async fn finish(mut self) -> Vec<Value> {
        self.write.shutdown().await.expect("shutdown");
        let mut rest = Vec::new();
        while let Some(line) = tokio::time::timeout(Duration::from_secs(10), self.lines.next_line())
            .await
            .expect("EOF within 10 s")
            .expect("read")
        {
            rest.push(serde_json::from_str(&line).expect("JSON"));
        }
        self.served
            .await
            .expect("serve did not panic")
            .expect("serve ends cleanly at EOF");
        rest
    }
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn initialize(id: u64, version: &str) -> Value {
    request(
        id,
        "initialize",
        json!({"protocolVersion": version, "capabilities": {}, "clientInfo": {"name": "t"}}),
    )
}

#[tokio::test]
async fn the_recorded_claude_transcript_is_answered() {
    let transcript = include_str!("transcripts/claude-2.1.287.ndjson");
    let mut client = Client::start();
    for line in transcript.lines() {
        client.send_raw(line.as_bytes()).await;
    }
    let mut answers = Vec::new();
    for _ in 0..3 {
        answers.push(client.recv().await);
    }
    answers.extend(client.finish().await);
    assert_eq!(
        answers.len(),
        3,
        "nothing for the notification: {answers:#?}"
    );
    let ids: Vec<&Value> = answers.iter().map(|a| &a["id"]).collect();
    assert_eq!(ids, [&json!(0), &json!(1), &json!(2)]);
    for answer in &answers {
        assert_eq!(answer["jsonrpc"], "2.0");
    }
    let init = &answers[0]["result"];
    assert_eq!(init["protocolVersion"], "2025-11-25");
    assert_eq!(init["capabilities"], json!({"tools": {}}));
    assert_eq!(init["serverInfo"]["name"], "htui");
    assert_eq!(init["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    let names: Vec<&Value> = answers[1]["result"]["tools"]
        .as_array()
        .expect("a tools array")
        .iter()
        .map(|t| &t["name"])
        .collect();
    assert_eq!(names.len(), 4);
    assert!(answers[1]["result"].get("nextCursor").is_none());
    assert_eq!(
        answers[1]["result"]["tools"][0]["inputSchema"],
        json!({"type": "object"})
    );
    let call = &answers[2]["result"];
    assert_eq!(call["isError"], false);
    assert_eq!(call["content"][0]["type"], "text");
    let text: Value =
        serde_json::from_str(call["content"][0]["text"].as_str().expect("text")).expect("JSON");
    assert_eq!(text["behavior"], "allow");
}

#[tokio::test]
async fn initialize_echoes_each_supported_version() {
    let mut client = Client::start();
    for (id, version) in (0u64..).zip(SUPPORTED_VERSIONS) {
        client.send(initialize(id, version)).await;
        let answer = client.recv().await;
        assert_eq!(answer["id"], id);
        assert_eq!(answer["result"]["protocolVersion"], version);
    }
    assert_eq!(
        SUPPORTED_VERSIONS[0], "2025-11-25",
        "the newest leads the list"
    );
    client.finish().await;
}

#[tokio::test]
async fn initialize_answers_the_newest_for_an_unknown_version() {
    let mut client = Client::start();
    client.send(initialize(7, "1999-01-01")).await;
    let answer = client.recv().await;
    assert_eq!(answer["result"]["protocolVersion"], "2025-11-25");
    client.finish().await;
}

#[tokio::test]
async fn ping_answers_an_empty_result() {
    let mut client = Client::start();
    client
        .send(json!({"jsonrpc": "2.0", "id": "p-1", "method": "ping"}))
        .await;
    let answer = client.recv().await;
    assert_eq!(answer, json!({"jsonrpc": "2.0", "id": "p-1", "result": {}}));
    client.finish().await;
}

#[tokio::test]
async fn an_unknown_method_is_minus_32601() {
    let mut client = Client::start();
    client.send(request(3, "resources/list", json!({}))).await;
    let answer = client.recv().await;
    assert_eq!(answer["id"], 3);
    assert_eq!(answer["error"]["code"], -32601);
    assert_eq!(
        answer["error"]["message"],
        "method not found: resources/list"
    );
    assert!(answer.get("result").is_none());
    client.finish().await;
}

#[tokio::test]
async fn a_non_json_line_is_minus_32700_and_the_next_is_served() {
    let mut client = Client::start();
    client.send_raw(b"this is not json").await;
    let answer = client.recv().await;
    assert_eq!(answer["id"], Value::Null);
    assert_eq!(answer["error"]["code"], -32700);
    assert_eq!(answer["error"]["message"], "parse error");
    client.send(request(4, "ping", json!({}))).await;
    assert_eq!(client.recv().await["id"], 4);
    client.finish().await;
}

#[tokio::test]
async fn a_batch_array_is_minus_32600() {
    let mut client = Client::start();
    client
        .send(json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}]))
        .await;
    let answer = client.recv().await;
    assert_eq!(answer["id"], Value::Null);
    assert_eq!(answer["error"]["code"], -32600);
    client
        .send(json!({"jsonrpc": "1.0", "id": 9, "method": "ping"}))
        .await;
    let answer = client.recv().await;
    assert_eq!(answer["id"], 9, "a bad jsonrpc keeps its id");
    assert_eq!(answer["error"]["code"], -32600);
    client.send(json!({"jsonrpc": "2.0", "id": 10})).await;
    let answer = client.recv().await;
    assert_eq!(answer["id"], 10, "no method keeps its id");
    assert_eq!(answer["error"]["code"], -32600);
    client.finish().await;
}

#[tokio::test]
async fn an_oversized_line_is_refused_and_the_next_is_served() {
    let mut client = Client::start();
    let long = vec![b'x'; MAX_LINE_BYTES + 1];
    client.send_raw(&long).await;
    let answer = client.recv().await;
    assert_eq!(answer["id"], Value::Null);
    assert_eq!(answer["error"]["code"], -32700);
    assert_eq!(answer["error"]["message"], "line exceeds 1 MiB");
    client.send(request(5, "ping", json!({}))).await;
    assert_eq!(client.recv().await["id"], 5);
    client.finish().await;
}

#[tokio::test]
async fn an_unadvertised_tool_is_minus_32602() {
    let mut client = Client::start();
    client
        .send(request(
            6,
            "tools/call",
            json!({"name": "command_run", "arguments": {}}),
        ))
        .await;
    let answer = client.recv().await;
    assert_eq!(answer["id"], 6);
    assert_eq!(answer["error"]["code"], -32602);
    assert_eq!(answer["error"]["message"], "unknown tool: command_run");
    client.finish().await;
}

#[tokio::test]
async fn bad_arguments_are_is_error_not_a_protocol_error() {
    let mut client = Client::start();
    client
        .send(request(
            7,
            "tools/call",
            json!({"name": "echo", "arguments": {"run_id": "x"}}),
        ))
        .await;
    let answer = client.recv().await;
    assert!(answer.get("error").is_none(), "{answer}");
    assert_eq!(answer["result"]["isError"], true);
    let text = answer["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(text.starts_with("invalid arguments: "), "{text}");
    // `arguments` absent is `{}`, which `echo` refuses for the missing field.
    client
        .send(request(8, "tools/call", json!({"name": "echo"})))
        .await;
    let answer = client.recv().await;
    assert_eq!(answer["result"]["isError"], true);
    let text = answer["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(text.contains("missing field `text`"), "{text}");
    client.finish().await;
}

#[tokio::test]
async fn a_slow_call_does_not_block_ping() {
    let mut client = Client::start();
    client
        .send(request(1, "tools/call", json!({"name": "slow"})))
        .await;
    assert_eq!(client.seen().await, Seen::SlowStarted);
    client.send(request(2, "ping", json!({}))).await;
    let answer = client.recv().await;
    assert_eq!(answer["id"], 2, "ping is answered while the call runs");
    let rest = client.finish().await;
    assert!(
        rest.is_empty(),
        "EOF aborts the slow call unanswered: {rest:?}"
    );
}

#[tokio::test]
async fn a_cancelled_call_is_aborted_and_answered_nothing() {
    let mut client = Client::start();
    client
        .send(request(5, "tools/call", json!({"name": "slow"})))
        .await;
    assert_eq!(client.seen().await, Seen::SlowStarted);
    client
        .send(
            json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                     "params": {"requestId": 5, "reason": "user"}}),
        )
        .await;
    assert_eq!(client.seen().await, Seen::SlowDropped, "the drop guard ran");
    client.send(request(6, "ping", json!({}))).await;
    assert_eq!(
        client.recv().await["id"],
        6,
        "nothing was answered for id 5"
    );
    let rest = client.finish().await;
    assert!(rest.is_empty(), "{rest:?}");
}

#[tokio::test]
async fn progress_ticks_carry_the_clients_token() {
    let mut client = Client::start();
    client
        .send(request(
            3,
            "tools/call",
            json!({"name": "ticker", "arguments": {}, "_meta": {"progressToken": "tok-7"}}),
        ))
        .await;
    for expected in [1, 2] {
        let tick = client.recv().await;
        assert_eq!(tick["method"], "notifications/progress", "{tick}");
        assert!(tick.get("id").is_none(), "a notification has no id");
        assert_eq!(tick["params"]["progressToken"], "tok-7");
        assert_eq!(tick["params"]["progress"], expected);
    }
    let answer = client.recv().await;
    assert_eq!(answer["id"], 3);
    assert_eq!(answer["result"]["content"][0]["text"], "done");
    // Without a token, no tick at all.
    client
        .send(request(4, "tools/call", json!({"name": "ticker"})))
        .await;
    assert_eq!(client.recv().await["id"], 4);
    client.finish().await;
}

#[tokio::test]
async fn a_panicking_call_is_minus_32603_and_the_next_is_served() {
    let mut client = Client::start();
    client
        .send(request(9, "tools/call", json!({"name": "panic"})))
        .await;
    let answer = client.recv().await;
    assert_eq!(answer["id"], 9, "{answer}");
    assert_eq!(answer["error"]["code"], -32603);
    assert_eq!(
        answer["error"]["message"],
        "internal error: the tool panicked"
    );
    assert!(answer.get("result").is_none());
    client.send(request(10, "ping", json!({}))).await;
    assert_eq!(client.recv().await["id"], 10);
    let rest = client.finish().await;
    assert!(rest.is_empty(), "{rest:?}");
}

#[tokio::test]
async fn a_reused_id_stays_cancellable_after_its_twin_finishes() {
    let mut client = Client::start();
    client
        .send(request(1, "tools/call", json!({"name": "slow"})))
        .await;
    assert_eq!(client.seen().await, Seen::SlowStarted);
    // The client reuses id 1 (it must not, but the loop accepts it): the quick twin finishes first.
    client
        .send(request(
            1,
            "tools/call",
            json!({"name": "echo", "arguments": {"text": "twin"}}),
        ))
        .await;
    let answer = client.recv().await;
    assert_eq!(answer["id"], 1);
    assert_eq!(answer["result"]["content"][0]["text"], "twin");
    client
        .send(
            json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                     "params": {"requestId": 1}}),
        )
        .await;
    assert_eq!(
        client.seen().await,
        Seen::SlowDropped,
        "the slow call with the same id is still aborted"
    );
    let rest = client.finish().await;
    assert!(rest.is_empty(), "{rest:?}");
}
