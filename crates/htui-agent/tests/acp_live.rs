//! The one test that touches a real agent (`docs/ANA-4.md` §8 test strategy 5, §11 criterion 9).
//!
//! `#[ignore]` by default: it spawns whatever this box has installed, and a box without `node` or
//! without the adapter is not a failing build. Run it by hand:
//!
//! ```text
//! cargo test -p htui-agent --features test-support --test acp_live -- --ignored --nocapture
//! ```
//!
//! It **burns no model tokens**: `initialize` and `session/new` are protocol traffic, and the test
//! sends no prompt. What it proves is the half of criterion 9 that milestone 3 owns — the seeded
//! `claude` row resolves on this box to a launch that starts the adapter and completes a handshake
//! at `protocolVersion 1` — plus the process-group half of criterion 11, by killing the tree and
//! looking for survivors.

use std::path::PathBuf;
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::InitializeRequest;
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo};
use htui_agent::acp::client;
use htui_agent::launch::AgentLaunch;
use htui_core::model::Agent as AgentRow;
use tokio::sync::oneshot;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// The seeded `claude` row, exactly as `PgStore::seed_if_empty_as` inserts it.
fn claude_row() -> AgentRow {
    htui_core::model::agent::seed_rows(chrono::Utc::now())
        .into_iter()
        .find(|agent| agent.name == "claude")
        .expect("the seed rows carry `claude`")
}

#[tokio::test]
#[ignore = "spawns the agent installed on this box"]
async fn the_seeded_claude_row_reaches_a_v1_handshake() {
    let row = claude_row();
    let launch: AgentLaunch = serde_json::from_value(row.launch.clone()).expect("the seed parses");
    let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let tools = htui_agent::tools::resolve(launch.discovery.as_ref(), &cwd)
        .await
        .expect("the seeded row's tools resolve on this box");
    println!("resolved tools: {tools:#?}");

    let resolved = htui_agent::launch::resolve(&launch, &tools).expect("placeholders substitute");
    println!("command: {} {:?}", resolved.command, resolved.args);

    let mut spawned = htui_agent::launch::spawn(&resolved, &cwd)
        .await
        .expect("the adapter starts");
    let writer = spawned.take_stdin().expect("stdin is piped");
    let reader = spawned.take_stdout().expect("stdout is piped");

    let (report, response) = oneshot::channel();
    let transport = ByteStreams::new(
        writer.into_inner().compat_write(),
        reader.into_inner().compat(),
    );
    let handshake = Client.builder().name("htui-live-probe").connect_with(
        transport,
        async move |cx: ConnectionTo<Agent>| {
            let init = cx
                .send_request(
                    InitializeRequest::new(ProtocolVersion::V1)
                        .client_capabilities(client::client_capabilities(
                            &htui_agent::launch::ClientCapabilities {
                                fs_read: true,
                                fs_write: true,
                                terminal: false,
                                elicitation: false,
                            },
                        ))
                        .client_info(client::client_info()),
                )
                .block_task()
                .await?;
            let _ = report.send(init);
            Ok(())
        },
    );

    let init = tokio::time::timeout(Duration::from_secs(60), async {
        let (_, init) = tokio::join!(handshake, response);
        init
    })
    .await
    .expect("the handshake completes within a minute")
    .expect("the agent answered `initialize`");

    println!("agent: {:?}", init.agent_info);
    assert_eq!(
        init.protocol_version,
        ProtocolVersion::V1,
        "MOD-2 speaks wire protocol 1 (ANA-4 §3)"
    );
    let info = init.agent_info.expect("the adapter names itself");
    assert!(!info.version.is_empty(), "the adapter reports a version");

    spawned.kill_tree().await.expect("the process tree dies");
    let _ = spawned.wait().await;

    // Criterion 11, the process-group half: the wrapper (`node`) must not outlive the kill. The
    // job-object half is Windows's and is not verified on this box (plan D31).
    #[cfg(unix)]
    {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let survivors = std::process::Command::new("pgrep")
            .args(["-f", "claude-agent-acp"])
            .output()
            .expect("pgrep runs");
        let listed = String::from_utf8_lossy(&survivors.stdout);
        assert!(
            listed.trim().is_empty(),
            "killing the tree left processes behind: {listed}"
        );
    }
}
