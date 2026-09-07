//! One real conversation, through the whole seam (`docs/ANA-4.md` §11 criteria 9 and 11).
//!
//! `#[ignore]` by default: it spawns the agent installed on this box and spends a few tokens of
//! the maintainer's subscription. Run it by hand:
//!
//! ```text
//! cargo test -p htui --features testkit --test chat_live -- --ignored --nocapture
//! ```
//!
//! What it proves is the milestone's own claim, minus the terminal: the seeded `claude` row, the
//! production `AgentRuntime`, the ACP transport, the recorder and the chat seam together turn a
//! typed prompt into a live streamed turn whose rows land in a store — and then end the session
//! and kill its process tree. The tab's rendering of those same frames is covered, deterministically
//! and without a process, by `tests/chat.rs`.

use std::time::Duration;

use htui::agent_worker::{AgentRuntime, Served};
use htui::store_worker::{
    ChatFrame, Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest,
};
use htui::ui::tabs::ChatTab;
use htui_core::fixtures::ids;
use htui_core::model::EventKind;
use htui_core::store::{MemStore, ReadStore as _};
use htui_store::Backend;
use tokio::sync::mpsc;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "spawns the agent installed on this box and spends tokens"]
async fn a_real_claude_session_streams_into_the_store_and_then_ends() {
    let store = MemStore::demo();
    // The fixture's `claude` row **is** the seed row, re-stamped: milestone 2 derived both from
    // `crates/htui-core/seeds/agent_claude.json`, so this launches exactly what a real database
    // would (`docs/decisions/mod/mod-2.md` D13).
    let agent_id = store
        .agents()
        .await
        .expect("the registry reads")
        .into_iter()
        .find(|summary| summary.agent.name == "claude")
        .expect("the fixture carries `claude`")
        .agent
        .id;

    let backend = Backend::memory(store.clone());
    let mut runtime = AgentRuntime::production().with_grace(Duration::from_secs(1));
    let (tx, mut rx) = mpsc::unbounded_channel::<ReplyEnvelope>();

    let start = RequestEnvelope {
        seq: 1,
        origin: Origin::Tab(ChatTab::ID),
        request: StoreRequest::ChatStart {
            project_id: ids::PROJECT_HTUI,
            agent_id,
            model: None,
            prompt: "Reply with exactly the word ok, nothing else. Do not use any tools."
                .to_owned(),
        },
    };
    let Served::Start { step_id, task } = runtime.serve(&backend, &tx, &start).await else {
        panic!("a chat start opens a session")
    };
    let session = tokio::spawn(task);
    runtime.attach(step_id, tokio::spawn(async {}));

    // Wait for the turn to end, then end the session the way `Esc Esc` does.
    let mut ended = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    while let Ok(Some(envelope)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        match envelope.reply {
            StoreReply::ChatAccepted { session_ref, .. } => {
                println!("accepted: session {session_ref:?}");
            }
            StoreReply::Chat(ChatFrame::Event(frame)) => {
                println!("frame: {:?}", frame.event);
                if matches!(frame.event, htui_agent::event::DriverEvent::Done(_)) {
                    let cancel = RequestEnvelope {
                        seq: 2,
                        origin: Origin::Tab(ChatTab::ID),
                        request: StoreRequest::ChatCancel { step_id },
                    };
                    runtime.serve(&backend, &tx, &cancel).await;
                }
            }
            StoreReply::Chat(ChatFrame::Ended { stop_reason }) => {
                println!("ended: {}", stop_reason.as_str());
                ended = true;
                break;
            }
            StoreReply::Chat(ChatFrame::Failed { message }) => {
                panic!("the session failed: {message}")
            }
            StoreReply::Failed { request, message } => panic!("{request} failed: {message}"),
            _ => {}
        }
    }
    assert!(ended, "the session ended within the deadline");
    let _ = tokio::time::timeout(Duration::from_secs(10), session).await;

    let log = store
        .step_events(step_id)
        .await
        .expect("the log reads")
        .expect("the chat step has a log");
    for row in &log {
        println!("row {:>2} turn {} {:?}", row.seq, row.turn, row.kind);
    }

    assert_eq!(log[0].kind, EventKind::Prompt, "the prompt opens the log");
    assert!(
        log.iter().any(|row| row.kind == EventKind::AssistantText),
        "the agent answered"
    );
    assert!(
        log.iter().any(|row| row.kind == EventKind::Done),
        "the turn closed"
    );
    // An `other` row's payload is `{ update, body }`, so the banner's fields are one level down.
    let banner = log
        .iter()
        .find(|row| {
            row.kind == EventKind::Other
                && row
                    .payload
                    .get("update")
                    .and_then(serde_json::Value::as_str)
                    == Some(htui_agent::acp::SESSION_STARTED)
        })
        .expect("the session banner names the agent-side session (§4.4)");
    let body = &banner.payload["body"];
    assert!(body["session_id"].is_string(), "{body}");
    assert_eq!(body["protocol_version"], 1, "MOD-2 speaks wire protocol 1");
    assert!(
        body["agent_version"]
            .as_str()
            .is_some_and(|v| !v.is_empty()),
        "the adapter's version is recorded per session (risk 6: version skew per box)"
    );

    // Criterion 11, the process-group half: nothing of the session outlives it.
    #[cfg(unix)]
    {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let survivors = std::process::Command::new("pgrep")
            .args(["-f", "claude-agent-acp"])
            .output()
            .expect("pgrep runs");
        assert!(
            String::from_utf8_lossy(&survivors.stdout).trim().is_empty(),
            "the agent's process tree outlived its session"
        );
    }
}
