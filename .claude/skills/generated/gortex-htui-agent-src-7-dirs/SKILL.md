---
name: gortex-htui-agent-src-7-dirs
description: "Work in the htui-agent/src +7 dirs area — 283 symbols across 17 files (74% cohesion)"
---

# htui-agent/src +7 dirs

283 symbols | 17 files | 74% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/conformance.rs`
- `crates/htui-agent/src/driver.rs`
- `crates/htui-agent/src/excerpt.rs`
- `crates/htui-agent/src/probe.rs`
- `crates/htui-agent/src/record.rs`
- `crates/htui-core/src/fixtures.rs`
- `crates/htui-core/src/model/event.rs`
- `crates/htui-core/src/model/run.rs`
- `crates/htui-core/src/scrub.rs`
- `crates/htui-core/src/store/mem.rs`
- `crates/htui-core/src/store/traits.rs`
- `crates/htui-core/tests/prompt_digest.rs`
- `crates/htui-core/tests/prompt_golden.rs`
- `crates/htui-store/src/pg/read.rs`
- `crates/htui-store/src/writer.rs`
- `crates/htui/tests/chat_offline.rs`
- `crates/htui/tests/chat_usage_pg.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/conformance.rs` | store, run_case_accepts_every_name_in_cases, store, cancel_answers_parked_permissions, Emit, ... |
| `crates/htui-agent/src/driver.rs` | send_follow_up, answer_permission, answer, spec, grace, ... |
| `crates/htui-agent/src/excerpt.rs` | matches, relative, the_gitignore_subset_is_the_declared_one |
| `crates/htui-agent/src/probe.rs` | to_value |
| `crates/htui-agent/src/record.rs` | record_prompt, dropped, fmt, sections, pump, ... |
| `crates/htui-core/src/fixtures.rs` | events |
| `crates/htui-core/src/model/event.rs` | tool_call_id, kind, turn, role, payload, ... |
| `crates/htui-core/src/model/run.rs` | started_at, project_id, step_id, project_id, started_by, ... |
| `crates/htui-core/src/scrub.rs` | secrets, new, a_known_secret_that_looks_like_a_credential_is_masked_not_refused, longer_secrets_mask_before_their_prefixes, MinimalScrubber, ... |
| `crates/htui-core/src/store/mem.rs` | step, step_log |
| `crates/htui-core/src/store/traits.rs` | step_events, step |
| `crates/htui-core/tests/prompt_digest.rs` | an_attribute_is_masked_before_it_is_escaped_and_cut |
| `crates/htui-core/tests/prompt_golden.rs` | scrubber |
| `crates/htui-store/src/pg/read.rs` | step, step_events |
| `crates/htui-store/src/writer.rs` | step_events, step |
| `crates/htui/tests/chat_offline.rs` | one_turn |
| `crates/htui/tests/chat_usage_pg.rs` | probe_snapshot, mapped_turn, max_cost_micros_total, the_latch_lands_on_postgres_and_leaves_probe_alone, store, ... |

## Entry Points

- `crates/htui/tests/chat_usage_pg.rs::the_latch_lands_on_postgres_and_leaves_probe_alone`

## Connected Communities

- **src/model +14 dirs** (44 cross-edges)
- **htui-agent/src +4 dirs · record** (43 cross-edges)
- **htui-agent/tests +13 dirs** (29 cross-edges)
- **htui-agent · push** (10 cross-edges)
- **htui-store/tests +13 dirs** (6 cross-edges)
- **htui/tests +14 dirs** (4 cross-edges)
- **htui-agent/tests +3 dirs · lock** (3 cross-edges)
- **src/model · normalize** (3 cross-edges)
- **htui-core/src · scrub** (2 cross-edges)
- **htui-agent/src +4 dirs · DriverCaps** (1 cross-edges)
- **src/model · usage_row** (1 cross-edges)
- **htui · drive_to_end** (1 cross-edges)
- **htui-agent/src +3 dirs** (1 cross-edges)
- **src/prompt +2 dirs · phase_implement_attempt2** (1 cross-edges)
- **htui/tests +1 dirs** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-9")
explore(operation:"context", task:"understand htui-agent/src +7 dirs", format:"gcx")
relations(operation:"usages", target:{symbol:"crates/htui/tests/chat_usage_pg.rs::the_latch_lands_on_postgres_and_leaves_probe_alone"}, format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
