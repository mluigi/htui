//! Recorded transcripts replayed through the §6.1 mapper (`docs/ANA-4.md` §8 test strategy 3).
//!
//! The fixtures are **real**. `claude_acp_turn.jsonl` was captured from
//! `@agentclientprotocol/claude-agent-acp` 0.48.0 on 2026-09-07, driving one prompt ("reply with
//! exactly the word ok") against the subscription login on this box. `agy_acp_turn.jsonl` was
//! captured from `antigravity-acp` 1.1.1 on 2026-09-10, driving three turns — a plain reply, a
//! file read and a file write — through the production runtime (MOD-2 `T34`,
//! `crates/htui/tests/chat_live_agy.rs`). Session ids are substituted out, `claude`'s
//! `available_commands_update` list is trimmed to two entries because that list is the
//! developer's own installed commands and not part of the protocol's shape, and `agy`'s scratch
//! directory is rewritten to `/scratch`; nothing else is edited.
//!
//! This is the regression net for adapter drift: when an adapter ships a new update kind, the
//! snapshot shows it landing in `other` instead of the build breaking. Two adapters rather than
//! one is what keeps that claim honest — the two dialects disagree about almost everything except
//! the envelope.

use htui_agent::acp::map::{Mapper, RATE_LIMIT_META_KEY};
use htui_agent::event::{DriverEvent, UsageEvent};
use htui_core::model::UsageTotals;
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

/// The `agy` transcript, mapped: milestone 7's `T34` capture, replayed the same way.
///
/// Captured on 2026-09-10 from `antigravity-acp` 1.1.1 driving three turns through the production
/// `AgentRuntime` (`crates/htui/tests/chat_live_agy.rs`). Session id substituted out and the
/// scratch directory rewritten to `/scratch`; nothing else edited (blueprint H-13).
///
/// Its value beside the `claude` one is that it holds a **different** dialect of the same
/// protocol, and the snapshot is where that shows: no `usage_update` at all, an `edit`-kinded
/// `tool_call` whose `rawInput` keys are the vendor's (`code_content` / `target_file`), a
/// `content[].type == "diff"` block that the mapper turns into an `edit_proposal` before the call
/// row, and — this is the interesting one — the same `tool_call` **re-sent verbatim** where the
/// spec's own example would send a `tool_call_update`. Nothing in §6.1 forbids it, and the mapper
/// handles it as it handles any `tool_call`: what the snapshot pins is that it decodes to the same
/// rows it did the day it was recorded.
#[test]
fn a_recorded_agy_turn_maps_to_the_rows_it_did_when_it_was_captured() {
    let events = mapped("agy_acp_turn.jsonl");
    insta::assert_debug_snapshot!("acp_map__agy_turn", events);
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

/// Every `usage` event of a fixture, in order.
fn usage_events(name: &str) -> Vec<UsageEvent> {
    mapped(name)
        .into_iter()
        .filter_map(|event| match event {
            DriverEvent::Usage(usage) => Some(usage),
            _ => None,
        })
        .collect()
}

/// D66: the vendor rate-limit blob reaches `UsageEvent.quota` **verbatim**, and it cannot reach a
/// total.
///
/// Both halves are load-bearing. Verbatim, because milestone 7 normalizes the blob in
/// `htui_core::model::quota` and MOD-12 may want a key this milestone does not read — the mapper
/// that edits it is the mapper that loses it. And never summed, because the blob rides in the same
/// `usage` payload the recorder persists and `UsageTotals::add_payload` walks that document:
/// `add_payload` reads five *fixed* key names (`htui-core/src/model/usage.rs:48-54`), so an object
/// under a sixth name is structurally incapable of moving a number. That is asserted rather than
/// argued, because the day someone reaches for `for (key, value) in payload` this test is what says
/// no.
///
/// The fixture supplies both cases of the presence question on its own: the first report of the
/// turn carries no `_meta` at all (line 2), a later one carries the blob (line 5).
#[test]
fn the_rate_limit_blob_is_captured_verbatim_and_never_summed() {
    // Parsed out of the fixture line rather than transcribed: a hand-copied literal would pass a
    // mapper that silently reshaped the document, which is the one thing this test is for.
    let from_the_wire = lines("claude_acp_turn.jsonl")
        .into_iter()
        .filter(|line| line["method"] == "session/update")
        .map(|line| line["params"]["update"].clone())
        .find(|update| update["_meta"][RATE_LIMIT_META_KEY].is_object())
        .map(|update| update["_meta"][RATE_LIMIT_META_KEY].clone())
        .expect("the capture carries the blob on one of its usage reports");

    let usage = usage_events("claude_acp_turn.jsonl");
    assert!(
        usage.len() >= 2,
        "the capture holds more than one usage report: {usage:?}"
    );
    assert_eq!(
        usage[0].quota, None,
        "the first report of a turn carries no `_meta`, so a client must not expect an allowance \
         on every one: {:?}",
        usage[0]
    );
    let carried = usage
        .iter()
        .find(|event| event.quota.is_some())
        .expect("a later report carries it");
    assert_eq!(
        carried.quota.as_ref(),
        Some(&from_the_wire),
        "the blob is stored exactly as it arrived, keys the mapper does not understand included"
    );

    // The capture reports the allowance and the cost on separate updates, so the cost is spliced
    // onto the blob-carrying event here: with both present, "only `cost_micros` moved" is a claim
    // about the blob rather than about an absent number.
    let mut both = carried.clone();
    both.cost_micros = Some(1_337);
    let payload = serde_json::to_value(&both).expect("a usage event serializes");
    assert!(
        payload["quota"].is_object(),
        "the blob is in the document `add_payload` is handed, not just in the struct: {payload}"
    );
    let mut totals = UsageTotals::default();
    totals.add_payload(&payload);
    assert_eq!(
        totals,
        UsageTotals {
            cost_micros: Some(1_337),
            ..UsageTotals::default()
        },
        "the four token totals stay `None` — the difference between nobody reporting and a \
         reported zero — and no total is invented from the blob"
    );
}

/// `docs/ANA-4.md` §11.14 `:1384-1388`, answered by the `agy` capture and pinned here so a future
/// adapter that changes any of the three is caught by a test rather than by surprise (MOD-2 plan
/// D65: `T34`'s answers are inputs to milestone 7, not a footnote after it).
///
/// The three answers, as this transcript holds them:
///
/// 1. **No `usage_update` at all**, over three turns including a tool call and a file write. So
///    `agy` reports neither cost nor context occupancy, §7's "Unverified — MOD-2 must confirm" row
///    is confirmed as *nothing*, the seed's `settings.quota.source` stays `"none"`, and the quota
///    column reads `—` for this agent by design.
/// 2. **`session/request_permission` is issued in `default` mode, for the write and not for the
///    read**, offering exactly two options: `allow` / `allow_once` and `deny` / `reject_once`. No
///    `allow_always`, no `reject_always`. The request itself is not a `session/update`, so it is
///    not in this fixture — it is in the live test's output — but the `tool_call` it gates is the
///    one below, in `status: "pending"`.
/// 3. **The edit is a standard `tool_call` with `kind: "edit"` and a `diff` content block**, not a
///    vendor shape in `other`. The `rawInput` keys are the vendor's own (`code_content`,
///    `target_file`) and the diff block carries a vendor `_meta.kind`, but both ride inside
///    schema-shaped fields the mapper already reads, so no mapper amendment is owed (plan D62).
#[test]
fn the_agy_capture_answers_the_three_open_11_14_questions() {
    let updates: Vec<Value> = lines("agy_acp_turn.jsonl")
        .into_iter()
        .filter(|line| line["method"] == "session/update")
        .map(|line| line["params"]["update"].clone())
        .collect();
    assert!(!updates.is_empty(), "the capture holds updates at all");

    // 1. `:1384` — nothing. An absence, asserted, because milestone 7 builds on it.
    assert!(
        !updates
            .iter()
            .any(|update| update["sessionUpdate"] == "usage_update"),
        "`agy_acp_server` 1.1.1 emits no `usage_update`; if a release starts to, this is where it \
         is noticed and §7's `agy` row has to be revisited"
    );

    // 3. `:1387-1388` — a schema-shaped `tool_call`, with the vendor's names inside it.
    let edit = updates
        .iter()
        .find(|update| update["sessionUpdate"] == "tool_call" && update["kind"] == "edit")
        .expect("the write arrived as a `tool_call` of kind `edit`, not as a vendor `other` row");
    assert_eq!(
        edit["status"], "pending",
        "the gated call is announced before the permission request, so the diff is on screen \
         while the user decides: {edit}"
    );
    let diff = edit["content"]
        .as_array()
        .expect("the call carries content blocks")
        .iter()
        .find(|block| block["type"] == "diff")
        .expect("one of them is a `diff` block, which is what becomes an `edit_proposal`");
    assert!(diff["path"].is_string(), "{diff}");
    assert_eq!(
        diff["newText"], "ok\n",
        "the block carries the whole new text, never a patch: {diff}"
    );
    assert!(
        diff.get("oldText").is_none(),
        "a created file has no old text, which is `a_new_file_diff_block_has_no_old_text`'s case \
         seen live: {diff}"
    );
    assert!(
        edit["rawInput"]["target_file"].is_string(),
        "the vendor's own input keys ride inside the schema's `rawInput`, so nothing about them \
         reaches the mapper: {edit}"
    );

    // The finding that is neither an assertion about `htui` nor about the protocol: this adapter
    // re-sends `tool_call` for a call it has already announced, where the spec's example would
    // send `tool_call_update`. Pinned because `htui`'s edit-proposal dedup is keyed per flush
    // window, so the repeat is what produced three `edit_proposal` rows for one file in the live
    // run — a recorder question, not a mapper one.
    let repeats = updates
        .iter()
        .filter(|update| {
            update["sessionUpdate"] == "tool_call" && update["toolCallId"] == edit["toolCallId"]
        })
        .count();
    assert!(
        repeats >= 2,
        "the capture is the evidence for the re-sent `tool_call` finding: {repeats} announcement(s)"
    );
}
