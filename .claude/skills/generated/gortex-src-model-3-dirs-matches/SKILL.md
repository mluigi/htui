---
name: gortex-src-model-3-dirs-matches
description: "Work in the src/model +3 dirs · matches area — 636 symbols across 10 files (81% cohesion)"
---

# src/model +3 dirs · matches

636 symbols | 10 files | 81% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-core/src/model/kind.rs`
- `crates/htui-core/src/model/overlap.rs`
- `crates/htui-core/src/model/run.rs`
- `crates/htui-core/src/store/conformance.rs`
- `crates/htui-core/src/store/mem.rs`
- `crates/htui-core/src/store/traits.rs`
- `crates/htui-store/src/backend.rs`
- `crates/htui-store/src/pg/read.rs`
- `crates/htui-store/src/pg/write.rs`
- `crates/htui-store/src/writer.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-core/src/model/kind.rs` | default_graph_id, ItemKindPatch, name, prefix, description, ... |
| `crates/htui-core/src/model/overlap.rs` | limit, NotClaimable, running, Overlaps, SlotFull, ... |
| `crates/htui-core/src/model/run.rs` | model, id, run_step_id, attempt, agent_id, ... |
| `crates/htui-core/src/store/conformance.rs` | run_snapshot, store, set_step_prompt_writes_digest_and_trim, project, S, ... |
| `crates/htui-core/src/store/mem.rs` | run, run, now, note, finish_step_settles_the_columns_and_leaves_usage_when_it_is_none, ... |
| `crates/htui-core/src/store/traits.rs` | claim_run, position, workspace, quota_at, interrupt_step, ... |
| `crates/htui-store/src/backend.rs` | step, step_commits |
| `crates/htui-store/src/pg/read.rs` | step_commits, step |
| `crates/htui-store/src/pg/write.rs` | step, command_runs |
| `crates/htui-store/src/writer.rs` | step_commits, step, new, step, command_runs, ... |

## Entry Points

- `crates/htui-core/src/store/mem.rs::trees_and_commits_upsert_on_their_repo_key_and_the_delete_counts_them`
- `crates/htui-core/src/store/mem.rs::a_lease_refresh_is_a_cas_on_its_owner_and_the_sweep_adopts_it`
- `crates/htui-core/src/store/mem.rs::step_creation_and_the_two_compare_and_sets_follow_the_law`
- `crates/htui-core/src/store/mem.rs::gate_answers_write_their_outcome_and_promotion_lifts_the_run_and_the_item`
- `crates/htui-core/src/store/mem.rs::resolve_inputs_prefers_this_run_and_skips_a_loser`

## Connected Communities

- **src/model +14 dirs** (70 cross-edges)
- **htui-store/src +9 dirs** (35 cross-edges)
- **htui-agent/tests +13 dirs** (17 cross-edges)
- **htui/tests +14 dirs** (17 cross-edges)
- **htui-orch/src +6 dirs** (7 cross-edges)
- **src/store +7 dirs** (5 cross-edges)
- **src/store +4 dirs · workspace_links_and_box_paths_u…** (5 cross-edges)
- **src/store · filter_status_project_tags_ready** (4 cross-edges)
- **src/store +2 dirs** (4 cross-edges)
- **htui-agent/src +7 dirs** (2 cross-edges)
- **src/store +5 dirs · Note** (1 cross-edges)
- **src/model +25 dirs** (1 cross-edges)
- **src/store +4 dirs · Repo** (1 cross-edges)
- **src/store · upsert_agent_by_id_name_unique** (1 cross-edges)
- **src/store +4 dirs · RepoBoxPath** (1 cross-edges)
- **src/install +6 dirs** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-98")
explore(operation:"context", task:"understand src/model +3 dirs · matches", format:"gcx")
relations(operation:"usages", target:{symbol:"crates/htui-core/src/store/mem.rs::trees_and_commits_upsert_on_their_repo_key_and_the_delete_counts_them"}, format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
