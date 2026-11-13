//! The Unix-socket channel end to end (MOD-11 D3, D5, blueprint §2.8, B-1, B-12): an
//! `McpHost<Backend>` over `Backend::memory(MemStore::demo())`, reached through its listener the
//! way `htui mcp` reaches it — the handshake, its three refusals, a dropped lease and `close`.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use htui_core::fixtures::ids;
use htui_core::model::{RunId, StepId, Transport};
use htui_core::prompt::render::HostnameLine;
use htui_core::store::{MemStore, StepFence};
use htui_mcp::channel::{
    Address, HandshakeLine, HandshakeReply, Refusal, RelayError, Token, relay,
};
use htui_mcp::host::McpHost;
use htui_mcp::{ENV_ADDR, ENV_TOKEN, RELAY_VERSION};
use htui_orch::tools::{ToolHost, ToolLease, ToolScope};
use htui_store::Backend;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const BUDGET: Duration = Duration::from_secs(10);

fn host() -> McpHost<Backend> {
    McpHost::new(Backend::memory(MemStore::demo())).expect("a host")
}

fn scope(transport: Transport) -> ToolScope {
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

/// The lease's address and token, as the agent's child reads them from its environment.
fn env_of(lease: &ToolLease) -> (Address, Token) {
    let addr = Address::new(lease.spec.env[ENV_ADDR].clone());
    let token = Token::parse(&lease.spec.env[ENV_TOKEN]).expect("a well-formed token");
    (addr, token)
}

/// A raw connection: sends one handshake line and answers the reply and the open stream.
async fn handshake(addr: &Address, line: &Value) -> (HandshakeReply, BufReader<UnixStream>) {
    let stream = UnixStream::connect(addr.as_str()).await.expect("connect");
    let mut stream = BufReader::new(stream);
    let mut bytes = line.to_string().into_bytes();
    bytes.push(b'\n');
    stream.write_all(&bytes).await.expect("write");
    let mut reply = String::new();
    tokio::time::timeout(BUDGET, stream.read_line(&mut reply))
        .await
        .expect("a reply in time")
        .expect("read");
    let reply = serde_json::from_str(&reply).expect("the reply is JSON");
    (reply, stream)
}

async fn send(stream: &mut BufReader<UnixStream>, message: Value) {
    let mut bytes = message.to_string().into_bytes();
    bytes.push(b'\n');
    stream.write_all(&bytes).await.expect("write");
}

async fn recv(stream: &mut BufReader<UnixStream>) -> Value {
    let mut line = String::new();
    let read = tokio::time::timeout(BUDGET, stream.read_line(&mut line))
        .await
        .expect("an answer in time")
        .expect("read");
    assert_ne!(read, 0, "the host closed the stream");
    serde_json::from_str(&line).expect("JSON")
}

/// Whether the host closed the stream (EOF within the budget).
async fn closed(stream: &mut BufReader<UnixStream>) -> bool {
    let mut rest = Vec::new();
    matches!(
        tokio::time::timeout(BUDGET, stream.read_to_end(&mut rest)).await,
        Ok(Ok(0))
    )
}

fn socket_path(addr: &Address) -> PathBuf {
    PathBuf::from(addr.as_str())
}

fn directory_of(addr: &Address) -> PathBuf {
    socket_path(addr)
        .parent()
        .map(Path::to_path_buf)
        .expect("the socket has a directory")
}

#[tokio::test]
async fn a_unix_round_trip_serves_initialize_after_the_handshake() {
    let host = host();
    let lease = host.open(scope(Transport::Acp)).expect("a lease");
    let (addr, token) = env_of(&lease);

    let (agent, child) = tokio::io::duplex(64 * 1024);
    let (child_in, child_out) = tokio::io::split(child);
    let relayed =
        htui_agent::contained::spawn(
            async move { relay(&addr, &token, child_in, child_out).await },
        );
    let (agent_read, mut agent_write) = tokio::io::split(agent);
    let mut agent_read = BufReader::new(agent_read).lines();

    let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
                      "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                                 "clientInfo": {"name": "t"}}});
    agent_write
        .write_all(format!("{init}\n").as_bytes())
        .await
        .expect("write");
    let line = tokio::time::timeout(BUDGET, agent_read.next_line())
        .await
        .expect("in time")
        .expect("read")
        .expect("a line");
    let answer: Value = serde_json::from_str(&line).expect("JSON");
    assert_eq!(answer["id"], 0);
    assert_eq!(answer["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(answer["result"]["serverInfo"]["name"], "htui");

    // The agent closing the child's stdin ends the relay cleanly.
    agent_write.shutdown().await.expect("shutdown");
    tokio::time::timeout(BUDGET, relayed)
        .await
        .expect("the relay ends")
        .expect("no panic")
        .expect("a clean end");
    drop(lease);
}

#[tokio::test]
async fn the_socket_directory_is_private() {
    let host = host();
    let lease = host.open(scope(Transport::Acp)).expect("a lease");
    let (addr, _) = env_of(&lease);
    assert_eq!(host.address(), Some(addr.clone()));
    let socket = socket_path(&addr);
    assert_eq!(socket.file_name().and_then(|n| n.to_str()), Some("s"));
    assert!(
        addr.as_str().len() < 108,
        "sun_path fits: {}",
        addr.as_str()
    );
    let dir = directory_of(&addr);
    let name = dir.file_name().and_then(|n| n.to_str()).expect("a name");
    assert!(
        name.starts_with(&format!("htui-mcp-{}-", std::process::id())),
        "{name}"
    );
    let mode = std::fs::metadata(&dir)
        .expect("the directory")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700, "mode {mode:o}");
    assert!(socket.exists());
    drop(lease);
}

#[tokio::test]
async fn a_bad_token_is_refused_with_a_reason_and_closed() {
    let host = host();
    let lease = host.open(scope(Transport::Acp)).expect("a lease");
    let (addr, _) = env_of(&lease);
    let stranger = Token::mint();
    let (reply, mut stream) = handshake(
        &addr,
        &json!({"token": stranger.as_str(), "version": RELAY_VERSION}),
    )
    .await;
    assert!(!reply.ok);
    let reason = reply.reason.expect("a reason");
    assert_eq!(reason, Refusal::UnknownToken.to_string());
    assert!(
        !reason.contains(stranger.as_str()),
        "the reason never names the token"
    );
    assert!(
        closed(&mut stream).await,
        "the host closes a refused stream"
    );

    // Through the relay, the same refusal is the relay's error.
    let (_agent, child) = tokio::io::duplex(1024);
    let (child_in, child_out) = tokio::io::split(child);
    let refused = relay(&addr, &stranger, child_in, child_out).await;
    assert!(
        matches!(refused, Err(RelayError::Refused(Refusal::UnknownToken))),
        "{refused:?}"
    );

    // A line that is not a handshake is malformed.
    let (reply, _) = handshake(&addr, &json!({"hello": "there"})).await;
    assert_eq!(reply.reason, Some(Refusal::Malformed.to_string()));
    drop(lease);
}

#[tokio::test]
async fn a_version_mismatch_is_refused_naming_both_versions() {
    let host = host();
    let lease = host.open(scope(Transport::Acp)).expect("a lease");
    let (addr, token) = env_of(&lease);
    let line = HandshakeLine {
        token: token.as_str().to_owned(),
        version: "0.0.9+relay.1".to_owned(),
    };
    let (reply, mut stream) =
        handshake(&addr, &serde_json::to_value(&line).expect("serialises")).await;
    assert!(!reply.ok);
    let reason = reply.reason.expect("a reason");
    assert_eq!(
        reason,
        Refusal::Version {
            host: RELAY_VERSION.to_owned(),
            relay: "0.0.9+relay.1".to_owned(),
        }
        .to_string()
    );
    assert!(
        reason.contains(RELAY_VERSION) && reason.contains("0.0.9+relay.1"),
        "{reason}"
    );
    assert!(!reason.contains(token.as_str()));
    assert!(closed(&mut stream).await);
    drop(lease);
}

#[tokio::test]
async fn a_dropped_lease_refuses_new_connections_and_ends_open_ones() {
    let host = host();
    let lease = host.open(scope(Transport::Acp)).expect("a lease");
    let (addr, token) = env_of(&lease);
    let hello = json!({"token": token.as_str(), "version": RELAY_VERSION});
    let (reply, mut open) = handshake(&addr, &hello).await;
    assert!(reply.ok, "{reply:?}");
    assert_eq!(reply.reason, None);

    drop(lease);

    send(
        &mut open,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
               "params": {"name": "box_profile", "arguments": {}}}),
    )
    .await;
    let answer = recv(&mut open).await;
    assert_eq!(answer["id"], 1);
    assert_eq!(answer["result"]["isError"], true, "{answer}");
    assert_eq!(answer["result"]["content"][0]["text"], "session ended");

    let (reply, _) = handshake(&addr, &hello).await;
    assert!(!reply.ok);
    assert_eq!(reply.reason, Some(Refusal::UnknownToken.to_string()));
}

#[tokio::test]
async fn close_removes_the_socket_and_its_directory() {
    let host = host();
    let lease = host.open(scope(Transport::Acp)).expect("a lease");
    let (addr, token) = env_of(&lease);
    let dir = directory_of(&addr);
    assert!(dir.exists());
    let (reply, mut open) = handshake(
        &addr,
        &json!({"token": token.as_str(), "version": RELAY_VERSION}),
    )
    .await;
    assert!(reply.ok);

    host.close();

    assert!(!socket_path(&addr).exists(), "the socket is gone");
    assert!(!dir.exists(), "the directory is gone");
    assert_eq!(host.address(), None);
    assert!(UnixStream::connect(addr.as_str()).await.is_err());
    assert!(closed(&mut open).await, "an open connection ends");
    drop(lease);
}
