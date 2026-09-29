---
name: gortex-htui-orch-src-6-dirs
description: "Work in the htui-orch/src +6 dirs area — 972 symbols across 20 files (80% cohesion)"
---

# htui-orch/src +6 dirs

972 symbols | 20 files | 80% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/cli/claude.rs`
- `crates/htui-agent/src/conformance.rs`
- `crates/htui-agent/src/event.rs`
- `crates/htui-core/src/model/run.rs`
- `crates/htui-core/src/store/conformance.rs`
- `crates/htui-core/src/store/mem.rs`
- `crates/htui-core/src/store/traits.rs`
- `crates/htui-orch/src/command.rs`
- `crates/htui-orch/src/conformance.rs`
- `crates/htui-orch/src/engine.rs`
- `crates/htui-orch/src/fake.rs`
- `crates/htui-orch/src/fanout.rs`
- `crates/htui-orch/src/gate.rs`
- `crates/htui-orch/src/overlap.rs`
- `crates/htui-orch/src/recover.rs`
- `crates/htui-orch/src/select.rs`
- `crates/htui-orch/src/status.rs`
- `crates/htui-orch/tests/gix_isolator.rs`
- `crates/htui-store/src/pg/read.rs`
- `crates/htui-store/src/pg/write.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/cli/claude.rs` | line, body_of |
| `crates/htui-agent/src/conformance.rs` | set_step_prompt, note, trim, step, from, ... |
| `crates/htui-agent/src/event.rs` | DoneEvent, stop_reason |
| `crates/htui-core/src/model/run.rs` | usage, started_by, graph, template, verify_outcome, ... |
| `crates/htui-core/src/store/conformance.rs` | run, fixture_steps |
| `crates/htui-core/src/store/mem.rs` | on, RefreshLease, ItemTransition, ReleaseLease, run_step_rows, ... |
| `crates/htui-core/src/store/traits.rs` | new, create_step, id, lease_until, run_steps, ... |
| `crates/htui-orch/src/command.rs` | position, stale_run, EngineError, GroupBase, attempt, ... |
| `crates/htui-orch/src/conformance.rs` | key, after_done, resume, phase, step, ... |
| `crates/htui-orch/src/engine.rs` | a_dead_walk_taken_again_leaves_the_set_and_keeps_its_lease, item_kind_name, judge, step, walk_live_step, ... |
| `crates/htui-orch/src/fake.rs` | item, step, phase, write_output, take_stall, ... |
| `crates/htui-orch/src/fanout.rs` | 0, JudgeFailure, call, forward, judge_failure_display_is_byte_exact, ... |
| `crates/htui-orch/src/gate.rs` | note, now, run, S, reject_step, ... |
| `crates/htui-orch/src/overlap.rs` | Snapshot |
| `crates/htui-orch/src/recover.rs` | steps, Reset, Candidate, winner_at, Judge, ... |
| `crates/htui-orch/src/select.rs` | run_spend, steps, run_spend_sums_cost_micros |
| `crates/htui-orch/src/status.rs` | attempt, MissingOutput, attempt, group_at_excludes_the_judge, Create, ... |
| `crates/htui-orch/tests/gix_isolator.rs` | item, step, key, _done, item, ... |
| `crates/htui-store/src/pg/read.rs` | run_steps, run, overlapping_runs, scope |
| `crates/htui-store/src/pg/write.rs` | now, box_id, lease_until, owner, new, ... |

## Connected Communities

- **htui-orch/src +2 dirs · steps** (32 cross-edges)
- **src/model +14 dirs** (30 cross-edges)
- **htui-orch/src +3 dirs · steps_of** (23 cross-edges)
- **htui-agent/tests +3 dirs · lock** (8 cross-edges)
- **src/prompt · app** (7 cross-edges)
- **htui-orch/src · from_app** (7 cross-edges)
- **htui-agent/src +7 dirs** (6 cross-edges)
- **htui-agent/src +4 dirs · record** (5 cross-edges)
- **htui-orch/src +2 dirs · FakeOrchestrator** (4 cross-edges)
- **htui-orch/src · cursor** (4 cross-edges)
- **htui-store/tests +13 dirs** (4 cross-edges)
- **htui/tests +3 dirs** (3 cross-edges)
- **htui-orch/src · next_attempt** (3 cross-edges)
- **htui-orch/src · step · gate** (3 cross-edges)
- **htui-orch/src · apply** (3 cross-edges)
- **htui-orch/src +1 dirs** (3 cross-edges)
- **htui-core · wrap** (2 cross-edges)
- **htui-orch/src · FakeIsolator** (2 cross-edges)
- **src/model +25 dirs** (2 cross-edges)
- **htui-orch/src · judge_phase** (2 cross-edges)
- **htui-orch/src · step · recover** (2 cross-edges)
- **htui-orch/src · done** (2 cross-edges)
- **htui-agent/tests +13 dirs** (2 cross-edges)
- **htui-orch/src · classify** (2 cross-edges)
- **htui-orch/src · judge_inputs_reverse_only_the_o…** (1 cross-edges)
- **htui-orch/src · verify_of_maps_command_runs_to_…** (1 cross-edges)
- **htui-orch/src · now** (1 cross-edges)
- **tabs/settings +11 dirs** (1 cross-edges)
- **htui-orch/src · judge_candidate_resolves_the_mo…** (1 cross-edges)
- **htui-agent/tests +2 dirs · heartbeat** (1 cross-edges)
- **src/prompt +5 dirs** (1 cross-edges)
- **src/model +3 dirs · matches** (1 cross-edges)
- **src/install +6 dirs** (1 cross-edges)
- **htui-store/src +8 dirs** (1 cross-edges)
- **src/prompt +2 dirs · phase_implement_attempt2** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-117")
explore(operation:"context", task:"understand htui-orch/src +6 dirs", format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
