//! `Settings > r`: the box probe end to end, through the shell (MOD-2 milestone 5, T27).
//!
//! **Fixture rule (blueprint H-7).** No test outside `htui-agent`'s `probe_live.rs` probes an
//! unmodified seed row. This box really has `node`, `claude` and the ACP adapter installed, so a
//! probe of the seeded `claude` row would spawn the real adapter inside `cargo test` and wait up
//! to `HANDSHAKE_TIMEOUT` on it. Everything here runs against [`unresolvable_registry`], whose
//! `discovery` names one tool that cannot exist: tier 1 answers `missing` and nothing is spawned.
#![cfg(feature = "testkit")]

use htui::agent_worker::AgentRuntime;
use htui::testkit::Harness;
use htui::ui::tabs::settings::{AgentsSection, SettingsTab};
use htui_core::store::{MemStore, WriteStore};
use serde_json::json;

/// The demo registry with every row's launch rewritten to resolve nowhere (H-7).
async fn unresolvable_registry() -> MemStore {
    let store = MemStore::demo();
    for summary in store.agents().await.expect("the memory store never fails") {
        let mut agent = summary.agent;
        agent.launch = json!({
            "command": "${gone}",
            "args": [],
            "env": {},
            "discovery": {
                "tools": {
                    "gone": { "kind": "path", "names": ["htui-no-such-binary-2f8e"] }
                },
                "handshake": true
            }
        });
        store.upsert_agent(&agent).await.expect("the row updates");
    }
    store
}

/// A settled Settings tab over `store`, with the agent section registered.
async fn settings_over(store: MemStore, runtime: Option<AgentRuntime>) -> Harness {
    let mut harness =
        Harness::over(store).with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
            AgentsSection::new(),
        )])));
    if let Some(runtime) = runtime {
        harness = harness.with_agent_runtime(runtime);
    }
    harness.settle().await;
    harness
}

/// D54 end to end: `r` asks, the column says so while it waits, and the probe's own reply replaces
/// it with what this box can run.
#[tokio::test]
async fn r_in_the_agents_section_probes_and_the_column_reads_the_status() {
    let store = unresolvable_registry().await;
    let mut harness = settings_over(store.clone(), Some(AgentRuntime::production())).await;

    assert!(
        harness.render().contains("not probed"),
        "nothing has been probed before `r`"
    );

    harness.key("r");
    let waiting = harness.render();
    assert_eq!(
        waiting.matches("probing\u{2026}").count(),
        2,
        "both rows say so while the one probe runs, before anything is served: {waiting}"
    );

    harness.drive_to_end().await;
    let probed = harness.render();
    assert_eq!(
        probed.matches("missing").count(),
        2,
        "a tool that resolves nowhere is `missing` on both rows: {probed}"
    );
    assert!(
        !probed.contains("probing"),
        "the probe's own reply clears the in-flight state: {probed}"
    );

    let rows = store.agents().await.expect("the memory store never fails");
    assert_eq!(
        rows.iter().filter(|row| row.on_box.is_some()).count(),
        2,
        "the probe wrote an `agent_box` row per enabled registry row"
    );
    insta::assert_snapshot!("agents_probed_missing", probed);
}

/// D54's second half: a probe already in flight refuses the next `r` rather than starting a second.
#[tokio::test]
async fn a_second_r_while_probing_is_refused_on_the_status_line() {
    let store = unresolvable_registry().await;
    let mut harness = settings_over(store.clone(), Some(AgentRuntime::production())).await;

    harness.key("r");
    harness.key("r");
    let status = harness.app().status.clone();
    assert!(
        status
            .as_deref()
            .is_some_and(|line| line.contains("already running")),
        "the second `r` is refused on the status line: {status:?}"
    );

    harness.drive_to_end().await;
    let rows = store.agents().await.expect("the memory store never fails");
    assert_eq!(
        rows.iter().filter(|row| row.on_box.is_some()).count(),
        2,
        "one probe ran, not two"
    );
    assert!(
        harness.render().contains("missing"),
        "and it finished normally"
    );
}

/// The refusal path clears the in-flight state too: a column stuck on `probing…` would be a lie
/// about a probe nobody is running.
#[tokio::test]
async fn a_harness_without_a_runtime_clears_the_probing_state_on_the_refusal() {
    let store = unresolvable_registry().await;
    let mut harness = settings_over(store, None).await;

    harness.key("r");
    harness.drive().await;

    let refused = harness.render();
    assert!(
        refused.contains("not probed") && !refused.contains("probing"),
        "the column goes back to what it knew: {refused}"
    );
    assert_eq!(
        harness.app().status.as_deref(),
        Some("probe_agents: no agent runtime in this harness"),
        "and the status line names the request that was refused"
    );
}
