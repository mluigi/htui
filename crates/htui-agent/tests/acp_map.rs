//! Recorded transcripts replayed through the §6.1 mapper (`docs/ANA-4.md` §8 test strategy 3).
//!
//! The fixtures are **real**: they were captured from `@agentclientprotocol/claude-agent-acp`
//! 0.48.0 on 2026-09-07, driving one prompt ("reply with exactly the word ok") against the
//! subscription login on this box. Session ids are substituted out and the
//! `available_commands_update` list is trimmed to two entries, because that list is the
//! developer's own installed commands and not part of the protocol's shape; nothing else is
//! edited.
//!
//! This is the regression net for adapter drift: when an adapter ships a new update kind, the
//! snapshot shows it landing in `other` instead of the build breaking.

use htui_agent::acp::map::Mapper;
use htui_agent::event::DriverEvent;
use serde_json::Value;

/// One recorded line.
fn lines(name: &str) -> Vec<Value> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("a fixture line is JSON"))
        .collect()
}

/// Every `session/update` in a fixture, mapped in order.
fn mapped(name: &str) -> Vec<DriverEvent> {
    let mut mapper = Mapper::new();
    lines(name)
        .iter()
        .filter(|line| line["method"] == "session/update")
        .flat_map(|line| mapper.map(&line["params"]["update"]))
        .collect()
}

#[test]
fn a_recorded_turn_maps_to_the_rows_it_did_when_it_was_captured() {
    let events = mapped("claude_acp_turn.jsonl");
    // `Debug`, not JSON: `DriverEvent` is deliberately not `Serialize` — what reaches
    // `session_event.payload` is the *payload* struct, and the recorder owns that step. The
    // snapshot's job is to pin the decode, and the variant names are what a drift diff should show.
    insta::assert_debug_snapshot!("acp_map__turn", events);
}

/// The handshake is not a `session/update` stream: what the fixture pins is that the two responses
/// still carry the fields the banner is built from (§4.4).
#[test]
fn the_recorded_handshake_still_carries_what_the_banner_needs() {
    let handshake = lines("claude_acp_handshake.jsonl");
    let initialize = &handshake[0]["result"];
    assert_eq!(initialize["protocolVersion"], 1);
    assert!(
        initialize["agentInfo"]["name"].is_string(),
        "the banner names the agent"
    );
    assert!(
        initialize["agentInfo"]["version"].is_string(),
        "the banner records the adapter version (risk 6: version skew per box)"
    );
    assert!(
        initialize["agentCapabilities"]["sessionCapabilities"]["resume"].is_object(),
        "session capabilities are empty **objects**, not booleans (ANA-4 §3): a decoder that \
         types them as bool fails on a real handshake"
    );

    let new_session = &handshake[1]["result"];
    let models: Vec<&str> = new_session["configOptions"]
        .as_array()
        .expect("configOptions is a list")
        .iter()
        .find(|option| option["id"] == "model")
        .expect("the adapter offers a `model` option, keyed by id and not by category")["options"]
        .as_array()
        .expect("the option lists its values")
        .iter()
        .filter_map(|option| option["value"].as_str())
        .collect();
    assert!(
        models.contains(&"sonnet"),
        "the model vocabulary is per installation, which is why the row stores an option id: \
         {models:?}"
    );
}

/// The reason the mapper reads JSON rather than the SDK's `SessionUpdate` enum: the adapter ships
/// kinds the schema does not know, and they must be **stored**, not refused.
#[test]
fn an_update_kind_the_schema_never_heard_of_still_lands_in_other() {
    let mut mapper = Mapper::new();
    let events = mapper.map(&serde_json::json!({
        "sessionUpdate": "subagent_spawned",
        "subagentId": "sub_1",
        "prompt": "go"
    }));
    match &events[..] {
        [DriverEvent::Other(row)] => {
            assert_eq!(row.update, "subagent_spawned");
            assert_eq!(row.body["subagentId"], "sub_1");
        }
        other => panic!("expected one `other` row: {other:?}"),
    }
}

/// What the capture proves about `usage_update`, recorded as a test so a future adapter that
/// changes it is caught here rather than in milestone 7.
///
/// Two findings from the live run, both new against `docs/ANA-4.md` §11.14:
///
/// 1. The Claude rate-limit blob really does arrive under `_meta["_claude/rateLimit"]`, and it
///    arrives on a **later** `usage_update` than the first one — the first carries `used`/`size`
///    only. Nothing in milestone 3 depends on it; milestone 7 latches it.
/// 2. `cost` appears only once the turn has produced assistant output, so a client must not
///    expect a cost figure on every `usage_update`.
#[test]
fn the_recorded_usage_updates_show_what_acp_reports() {
    let updates: Vec<Value> = lines("claude_acp_turn.jsonl")
        .into_iter()
        .filter(|line| line["method"] == "session/update")
        .map(|line| line["params"]["update"].clone())
        .filter(|update| update["sessionUpdate"] == "usage_update")
        .collect();
    assert!(
        updates.len() >= 2,
        "the capture holds more than one usage report"
    );
    assert!(
        updates
            .iter()
            .all(|update| update["used"].is_number() && update["size"].is_number()),
        "every usage_update carries context occupancy"
    );
    assert!(
        updates.iter().any(|update| update["cost"].is_object()),
        "a cost arrives once the turn has produced output"
    );
    assert!(
        updates
            .iter()
            .any(|update| update["_meta"]["_claude/rateLimit"].is_object()),
        "the subscription quota blob of ANA-4 §7 is real and arrives under `_meta`"
    );
    assert!(
        updates
            .iter()
            .all(|update| update.get("inputTokens").is_none()),
        "ACP reports no per-turn token counts on `usage_update` (§7), which is why the \
         conformance case asserts null token fields"
    );
}
