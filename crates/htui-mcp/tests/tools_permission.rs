//! MOD-11 T9: `permission_prompt` through `McpHost::client` (blueprint §12, §2.7, §2.10; plan D18,
//! I-7).
//!
//! The host is `McpHost<Backend>` over a `MemStore::demo()` (blueprint B-1). A `Transport::Cli`
//! scope's lease carries the session end of the bridge (B-21); each case takes its receiver and
//! plays the CLI session's part — read the request, answer a verdict — while the client plays the
//! CLI's `--permission-prompt-tool` call.

use std::path::PathBuf;
use std::time::Duration;

use htui_agent::prompt_bridge::{PromptCall, PromptClosed, PromptRequest, PromptVerdict};
use htui_core::fixtures::ids;
use htui_core::model::{ItemId, RunId, StepId, Transport};
use htui_core::prompt::render::HostnameLine;
use htui_core::store::{MemStore, StepFence};
use htui_mcp::protocol::CallResult;
use htui_mcp::{ENV_TOKEN, McpClient, McpHost};
use htui_orch::tools::{ToolHost, ToolLease, ToolScope};
use htui_store::Backend;
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// How long the session side waits for the tool to ask before the case fails.
const PATIENCE: Duration = Duration::from_secs(10);

/// The next request the tool sends the session, or a named failure rather than a hung case.
async fn asked(rx: &mut mpsc::Receiver<PromptRequest>) -> PromptRequest {
    tokio::time::timeout(PATIENCE, rx.recv())
        .await
        .expect("the tool asked the session in time")
        .expect("the tool asks the session")
}

/// A phase step's scope on `transport`.
fn scope(transport: Transport) -> ToolScope {
    ToolScope {
        run_id: RunId::new(),
        step_id: StepId::new(),
        project_id: ids::PROJECT_HTUI,
        item_id: Some(ItemId::new()),
        box_id: ids::BOX,
        user: ids::USER,
        fence: StepFence::Lease(uuid::Uuid::new_v4()),
        output_kind: Some("implementation".to_owned()),
        hostname: HostnameLine::Omitted,
        command_queue: false,
        cwd: PathBuf::from("."),
        transport,
    }
}

/// A live session on `scope` and its initialised in-process client.
async fn open(host: &McpHost<Backend>, scope: ToolScope) -> (ToolLease, McpClient) {
    let lease = host.open(scope).expect("a lease");
    let mut client = host
        .client(&lease.spec.env[ENV_TOKEN])
        .expect("a live session");
    client.initialize().await.expect("initialize");
    (lease, client)
}

/// A host over a fresh demo store.
fn host() -> McpHost<Backend> {
    McpHost::new(Backend::memory(MemStore::demo())).expect("a host")
}

/// What a CLI case holds: the host (its sessions end with it), the lease, the client, and the
/// receiver the CLI session would have taken.
type CliSession = (
    McpHost<Backend>,
    ToolLease,
    McpClient,
    mpsc::Receiver<PromptRequest>,
);

/// A CLI session on a fresh host; see [`CliSession`].
async fn cli_session() -> CliSession {
    let host = host();
    let (lease, client) = open(&host, scope(Transport::Cli)).await;
    let rx = lease
        .prompt
        .as_ref()
        .expect("B-21: a CLI scope's lease carries a port")
        .take()
        .expect("nobody took the receiver yet");
    (host, lease, client, rx)
}

/// The tool's answer as JSON; the CLI contract is never `isError`.
fn answer(result: &CallResult) -> Value {
    assert!(!result.is_error, "{}", result.text);
    serde_json::from_str(&result.text).expect("the answer is JSON")
}

/// Answers the next request with `verdict`, and returns the call it carried.
async fn answering(rx: &mut mpsc::Receiver<PromptRequest>, verdict: PromptVerdict) -> PromptCall {
    let request = asked(rx).await;
    request
        .answer
        .send(verdict)
        .expect("the tool waits for its verdict");
    request.call
}

#[tokio::test]
async fn the_prompt_tool_is_advertised_only_on_cli_scopes() {
    let host = host();
    let (_cli_lease, mut cli) = open(&host, scope(Transport::Cli)).await;
    let names = cli.tool_names().await.expect("tools/list");
    assert_eq!(
        names.last().map(String::as_str),
        Some("permission_prompt"),
        "last in table order: {names:?}"
    );

    let (_acp_lease, mut acp) = open(&host, scope(Transport::Acp)).await;
    let names = acp.tool_names().await.expect("tools/list");
    assert!(
        !names.iter().any(|name| name == "permission_prompt"),
        "{names:?}"
    );
    let refused = acp
        .call(
            "permission_prompt",
            json!({"tool_name": "Bash", "input": {}}),
        )
        .await
        .expect("a call");
    assert!(refused.is_error, "I-7: unadvertised is refused");
    assert_eq!(refused.text, "unknown tool: permission_prompt");
}

/// §2.7 line id 2, verbatim: the probe's call gets `allow` with its input echoed as
/// `updatedInput`, and the session saw exactly the call the CLI sent.
#[tokio::test]
async fn the_recorded_prompt_call_is_answered_allow_with_its_input() {
    let (_host, _lease, mut client, mut rx) = cli_session().await;
    let transcript = include_str!("transcripts/claude-2.1.287.ndjson");
    let line: Value = transcript
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("a JSON line"))
        .find(|line| line["id"] == 2)
        .expect("the recorded prompt call");
    assert_eq!(line["params"]["name"], "permission_prompt");

    let (response, call) = tokio::join!(
        client.request("tools/call", line["params"].clone()),
        answering(&mut rx, PromptVerdict::Allow),
    );
    let response = response.expect("tools/call");
    assert_eq!(
        call,
        PromptCall {
            tool_name: "mcp__htui__hello".to_owned(),
            input: json!({}),
            tool_use_id: Some("toolu_01HuMymhasmoPmGaLNFxJ4GV".to_owned()),
        }
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("a text answer");
    assert_eq!(
        serde_json::from_str::<Value>(text).expect("JSON"),
        json!({"behavior": "allow", "updatedInput": {}})
    );

    // A non-empty input is echoed untouched, and a key the CLI may add is not refused.
    let input = json!({"command": "cargo test", "description": "run the suite"});
    let (result, _call) = tokio::join!(
        client.call(
            "permission_prompt",
            json!({"tool_name": "Bash", "input": input, "tool_use_id": "toolu_2", "extra": 1}),
        ),
        answering(&mut rx, PromptVerdict::Allow),
    );
    assert_eq!(
        answer(&result.expect("a call")),
        json!({"behavior": "allow", "updatedInput": input})
    );
}

#[tokio::test]
async fn deny_carries_its_message() {
    let (_host, _lease, mut client, mut rx) = cli_session().await;
    let (result, call) = tokio::join!(
        client.call(
            "permission_prompt",
            json!({"tool_name": "Write", "input": {"file_path": "/etc/passwd"}}),
        ),
        answering(
            &mut rx,
            PromptVerdict::Deny {
                message: "denied in htui".to_owned()
            }
        ),
    );
    assert_eq!(call.tool_use_id, None, "an absent id decodes as none");
    assert_eq!(
        answer(&result.expect("a call")),
        json!({"behavior": "deny", "message": "denied in htui"})
    );
}

#[tokio::test]
async fn a_closed_session_answers_deny() {
    // The session took the receiver and is gone: the ask cannot even be sent.
    let (_host, _lease, mut client, rx) = cli_session().await;
    drop(rx);
    let result = client
        .call(
            "permission_prompt",
            json!({"tool_name": "Bash", "input": {}}),
        )
        .await
        .expect("a call");
    assert_eq!(
        answer(&result),
        json!({"behavior": "deny", "message": PromptClosed.to_string()})
    );

    // The session received the request and dropped its answer unanswered.
    let (_host, _lease, mut client, mut rx) = cli_session().await;
    let (result, ()) = tokio::join!(
        client.call(
            "permission_prompt",
            json!({"tool_name": "Bash", "input": {}})
        ),
        async {
            drop(asked(&mut rx).await);
        },
    );
    assert_eq!(
        answer(&result.expect("a call")),
        json!({
            "behavior": "deny",
            "message": "the agent session ended before the permission was answered"
        })
    );
}
