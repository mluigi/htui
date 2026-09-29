---
name: gortex-htui-agent-tests-4-dirs-json
description: "Work in the htui-agent/tests +4 dirs · json area — 365 symbols across 20 files (72% cohesion)"
---

# htui-agent/tests +4 dirs · json

365 symbols | 20 files | 72% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/acp/auth.rs`
- `crates/htui-agent/src/acp/mod.rs`
- `crates/htui-agent/src/auth/mod.rs`
- `crates/htui-agent/src/probe.rs`
- `crates/htui-agent/tests/acp_conformance.rs`
- `crates/htui-agent/tests/acp_driver.rs`
- `crates/htui-agent/tests/agy_live.rs`
- `crates/htui-agent/tests/auth.rs`
- `crates/htui-agent/tests/cli_conformance.rs`
- `crates/htui-agent/tests/cli_driver.rs`
- `crates/htui-agent/tests/cli_live.rs`
- `crates/htui-agent/tests/cli_map.rs`
- `crates/htui-agent/tests/driver_contract.rs`
- `crates/htui-agent/tests/install.rs`
- `crates/htui-agent/tests/install_live.rs`
- `crates/htui-agent/tests/launch.rs`
- `crates/htui-agent/tests/probe.rs`
- `crates/htui-agent/tests/probe_live.rs`
- `crates/htui-agent/tests/replay.rs`
- `crates/htui-core/src/model/agent.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/acp/auth.rs` | WireFlow, handshake_timeout, events, choice, cancel |
| `crates/htui-agent/src/acp/mod.rs` | flow, caps, agent, from_row, authenticate |
| `crates/htui-agent/src/auth/mod.rs` | AuthChoice, 0, Logout, Method |
| `crates/htui-agent/src/probe.rs` | Row, reason, env, tier2, Kept, ... |
| `crates/htui-agent/tests/acp_conformance.rs` | session_id, writer, request, claude_row, stream, ... |
| `crates/htui-agent/tests/acp_driver.rs` | stream, refuse_first_request, row, refuse_session_new, stream, ... |
| `crates/htui-agent/tests/agy_live.rs` | agy_row, resolved, mode_of, the_seeded_agy_row_resolves_through_the_glob_appends_uid_and_answers_v1, session_new_reports_its_config_options, ... |
| `crates/htui-agent/tests/auth.rs` | cwd, message, agent_row, dir, a_terminal_typed_method_is_hidden_and_named_but_never_sent, ... |
| `crates/htui-agent/tests/cli_conformance.rs` | cli_row |
| `crates/htui-agent/tests/cli_driver.rs` | executable, a_session_over_a_scripted_agent_streams_a_turn_and_then_a_follow_up, drain, start, what, ... |
| `crates/htui-agent/tests/cli_live.rs` | records, user_message, json, replacement, records, ... |
| `crates/htui-agent/tests/cli_map.rs` | a_tool_input_naming_a_path_carries_it_as_a_location, a_rate_limit_event_that_is_not_an_object_is_not_carried, a_user_line_with_no_tool_result_maps_to_nothing, a_line_that_is_not_an_envelope_is_still_recorded |
| `crates/htui-agent/tests/driver_contract.rs` | _answer, adapter_id, send_follow_up, _request_id, usage_mid_turn_is_a_capability_the_row_answers, ... |
| `crates/htui-agent/tests/install.rs` | settings, env, row, handshake, archive, ... |
| `crates/htui-agent/tests/install_live.rs` | amp_row |
| `crates/htui-agent/tests/launch.rs` | next_line, tap |
| `crates/htui-agent/tests/probe.rs` | walk_reports_one_capture_per_star, a_dropped_version_capture_still_kills_the_child, scripted_initialize, contents, credential_row, ... |
| `crates/htui-agent/tests/probe_live.rs` | children_of_this_process, row_of, the_seeded_claude_row_probes_ready_with_a_v1_handshake, the_same_row_without_node_is_missing_and_spawns_nothing, claude_row, ... |
| `crates/htui-agent/tests/replay.rs` | kind, row, seq, an_unknown_kind_never_costs_the_transcript, an_undecodable_payload_is_strict_err_and_lossy_other, ... |
| `crates/htui-core/src/model/agent.rs` | billing, Agent, now, enabled, id, ... |

## Entry Points

- `crates/htui-agent/tests/agy_live.rs::session_new_reports_its_config_options`
- `crates/htui-agent/tests/agy_live.rs::the_seeded_agy_row_resolves_through_the_glob_appends_uid_and_answers_v1`
- `crates/htui-agent/tests/probe.rs::probe_agent_records_the_tier_and_never_the_value`
- `crates/htui-agent/tests/cli_driver.rs::a_session_over_a_scripted_agent_streams_a_turn_and_then_a_follow_up`
- `crates/htui-agent/tests/probe.rs::a_manual_entry_survives_a_probe_that_finds_nothing_and_is_refreshed_by_one_that_does`

## Connected Communities

- **htui-agent/tests +3 dirs · lock** (35 cross-edges)
- **src/model +14 dirs** (19 cross-edges)
- **htui-agent/tests +13 dirs** (17 cross-edges)
- **htui-agent/tests +4 dirs · boxed** (14 cross-edges)
- **htui/tests +14 dirs** (11 cross-edges)
- **htui-agent/src +4 dirs · record** (11 cross-edges)
- **src/isolate +10 dirs** (8 cross-edges)
- **htui-agent · ResolvedLaunch** (7 cross-edges)
- **htui-agent/tests +3 dirs · spawn** (6 cross-edges)
- **src/prompt +5 dirs** (5 cross-edges)
- **htui-agent/tests +2 dirs · heartbeat** (4 cross-edges)
- **htui-agent · session_main** (3 cross-edges)
- **htui-agent · push** (3 cross-edges)
- **src/install +3 dirs** (3 cross-edges)
- **htui-agent · drained** (3 cross-edges)
- **htui-agent/tests · demanding** (2 cross-edges)
- **htui-agent/tests +7 dirs** (2 cross-edges)
- **htui-agent/tests · executable** (2 cross-edges)
- **htui-orch/src +6 dirs** (2 cross-edges)
- **htui-agent · handshake** (2 cross-edges)
- **htui-agent/tests · trait_pair_is_dyn_compatible_an…** (2 cross-edges)
- **src/install +6 dirs** (1 cross-edges)
- **htui-agent · spec** (1 cross-edges)
- **htui-agent/tests +2 dirs · read** (1 cross-edges)
- **htui-store/tests +13 dirs** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-40")
explore(operation:"context", task:"understand htui-agent/tests +4 dirs · json", format:"gcx")
relations(operation:"usages", target:{symbol:"crates/htui-agent/tests/agy_live.rs::session_new_reports_its_config_options"}, format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
