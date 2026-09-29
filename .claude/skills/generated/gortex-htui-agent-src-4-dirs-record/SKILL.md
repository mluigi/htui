---
name: gortex-htui-agent-src-4-dirs-record
description: "Work in the htui-agent/src +4 dirs · record area — 440 symbols across 12 files (82% cohesion)"
---

# htui-agent/src +4 dirs · record

440 symbols | 12 files | 82% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/conformance.rs`
- `crates/htui-agent/src/driver.rs`
- `crates/htui-agent/src/event.rs`
- `crates/htui-agent/src/record.rs`
- `crates/htui-agent/src/replay.rs`
- `crates/htui-agent/tests/cli_driver.rs`
- `crates/htui-agent/tests/cli_live.rs`
- `crates/htui-agent/tests/recorder.rs`
- `crates/htui-agent/tests/replay.rs`
- `crates/htui-core/src/scrub.rs`
- `crates/htui-core/tests/prompt_digest.rs`
- `crates/htui/src/ui/overlay/registry.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/conformance.rs` | set_agent_box_quota, recorder, S, session, box_id, ... |
| `crates/htui-agent/src/driver.rs` | id, new |
| `crates/htui-agent/src/event.rs` | cost_amount, cost_currency, context_used, input_tokens, cost_micros, ... |
| `crates/htui-agent/src/record.rs` | Held, 0, latch, raw, error, ... |
| `crates/htui-agent/src/replay.rs` | row, envelope_from_row, envelope_or_other, rows, row, ... |
| `crates/htui-agent/tests/cli_driver.rs` | script, template |
| `crates/htui-agent/tests/cli_live.rs` | pid, name, waited, finish, assert_not_running, ... |
| `crates/htui-agent/tests/recorder.rs` | env_values_are_masked_in_persisted_rows, bounded_ui_channel_drops_are_counted, claim_run, to, event, ... |
| `crates/htui-agent/tests/replay.rs` | scrubber, step, rows_re_recorded_from_their_envelopes_are_the_same_rows, the_fixture_plan_step_decodes, driver_script, ... |
| `crates/htui-core/src/scrub.rs` | rule, Unmasked, path |
| `crates/htui-core/tests/prompt_digest.rs` | the_only_section_entry_constructor_is_the_records_projection |
| `crates/htui/src/ui/overlay/registry.rs` | pop |

## Entry Points

- `crates/htui-agent/tests/recorder.rs::seq_is_gapless_turns_count_and_digest_reaches_the_step`
- `crates/htui-agent/tests/recorder.rs::an_edit_proposal_reserves_its_seq_and_survives_a_flush`
- `crates/htui-agent/tests/recorder.rs::coalesces_chunks_and_replays_byte_identically`

## Connected Communities

- **src/model +14 dirs** (49 cross-edges)
- **htui-agent/tests +4 dirs · json** (37 cross-edges)
- **htui-agent/tests +13 dirs** (26 cross-edges)
- **htui-agent/src +7 dirs** (17 cross-edges)
- **htui-agent · push** (14 cross-edges)
- **htui-agent/tests +3 dirs · lock** (13 cross-edges)
- **htui-agent/src +2 dirs · DriverEvent** (6 cross-edges)
- **htui/tests +14 dirs** (4 cross-edges)
- **src/install +6 dirs** (3 cross-edges)
- **src/model · normalize** (2 cross-edges)
- **htui-agent/tests +4 dirs · boxed** (2 cross-edges)
- **htui-agent/tests +1 dirs · add_payload** (2 cross-edges)
- **src/prompt +2 dirs · phase_implement_attempt2** (1 cross-edges)
- **src/model +3 dirs · matches** (1 cross-edges)
- **htui-agent/tests · write_fixture** (1 cross-edges)
- **src/model · usage_row** (1 cross-edges)
- **htui-agent/src +3 dirs** (1 cross-edges)
- **htui-orch/src +6 dirs** (1 cross-edges)
- **htui-agent · spawn_sh** (1 cross-edges)
- **htui-agent/tests +3 dirs · spawn** (1 cross-edges)
- **src/acp · map** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-25")
explore(operation:"context", task:"understand htui-agent/src +4 dirs · record", format:"gcx")
relations(operation:"usages", target:{symbol:"crates/htui-agent/tests/recorder.rs::seq_is_gapless_turns_count_and_digest_reaches_the_step"}, format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
