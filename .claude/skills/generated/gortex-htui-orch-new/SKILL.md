---
name: gortex-htui-orch-new
description: "Work in the htui-orch · new area — 202 symbols across 4 files (89% cohesion)"
---

# htui-orch · new

202 symbols | 4 files | 89% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-orch/src/conformance.rs`
- `crates/htui-orch/src/engine.rs`
- `crates/htui-orch/src/isolate/git.rs`
- `crates/htui-orch/tests/gix_isolator.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-orch/src/conformance.rs` | orch, SupersedeWinner, winner |
| `crates/htui-orch/src/engine.rs` | SessionSink, Stranger, 0, truncated, now |
| `crates/htui-orch/src/isolate/git.rs` | paths, merge_conflict |
| `crates/htui-orch/tests/gix_isolator.rs` | isolation, cap, a_rejected_reviews_commits_never_reach_the_primary, K, stalled, ... |

## Entry Points

- `crates/htui-orch/tests/gix_isolator.rs::a_three_way_worktree_fan_out_merges_only_the_winner`
- `crates/htui-orch/tests/gix_isolator.rs::shared_serialized_siblings_run_in_turn_and_the_branch_ends_on_the_winner`

## Connected Communities

- **src/model +14 dirs** (20 cross-edges)
- **src/isolate +10 dirs** (17 cross-edges)
- **htui-orch/src +3 dirs · steps_of** (6 cross-edges)
- **src/model +3 dirs · matches** (4 cross-edges)
- **src/install +6 dirs** (4 cross-edges)
- **src/isolate · excludes** (3 cross-edges)
- **src/store +2 dirs** (2 cross-edges)
- **htui-orch/src · now** (2 cross-edges)
- **htui-agent/tests +3 dirs · lock** (2 cross-edges)
- **htui-orch/src +2 dirs · FakeOrchestrator** (1 cross-edges)
- **src/store +1 dirs** (1 cross-edges)
- **htui-agent/src +7 dirs** (1 cross-edges)
- **htui-orch/src · run** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-156")
explore(operation:"context", task:"understand htui-orch · new", format:"gcx")
relations(operation:"usages", target:{symbol:"crates/htui-orch/tests/gix_isolator.rs::a_three_way_worktree_fan_out_merges_only_the_winner"}, format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
