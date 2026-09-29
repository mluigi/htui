---
name: gortex-htui-store-src-9-dirs
description: "Work in the htui-store/src +9 dirs area — 578 symbols across 25 files (72% cohesion)"
---

# htui-store/src +9 dirs

578 symbols | 25 files | 72% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/fake.rs`
- `crates/htui-agent/tests/recorder.rs`
- `crates/htui-core/src/model/item.rs`
- `crates/htui-core/src/model/kind.rs`
- `crates/htui-core/src/model/mod.rs`
- `crates/htui-core/src/model/run.rs`
- `crates/htui-core/src/prompt/settings.rs`
- `crates/htui-core/src/prompt/template.rs`
- `crates/htui-core/src/seed.rs`
- `crates/htui-core/src/store/error.rs`
- `crates/htui-core/src/store/mem.rs`
- `crates/htui-core/src/store/traits.rs`
- `crates/htui-orch/src/verify.rs`
- `crates/htui-store/src/embed.rs`
- `crates/htui-store/src/error.rs`
- `crates/htui-store/src/identity.rs`
- `crates/htui-store/src/pg/mod.rs`
- `crates/htui-store/src/pg/read.rs`
- `crates/htui-store/src/pg/write.rs`
- `crates/htui-store/src/qdrant_settings.rs`
- `crates/htui-store/src/vector.rs`
- `crates/htui-store/src/vector_sync.rs`
- `crates/htui-store/src/writer.rs`
- `crates/htui/src/agent_worker.rs`
- `crates/htui/src/store_worker.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/fake.rs` | report, hold_usage |
| `crates/htui-agent/tests/recorder.rs` | expected, update_item_kind, id, patch |
| `crates/htui-core/src/model/item.rs` | is_terminal |
| `crates/htui-core/src/model/kind.rs` | ItemKind, description, prefix, default_graph_id, project_id, ... |
| `crates/htui-core/src/model/mod.rs` | run_status_activity_matches_top_bar_rule |
| `crates/htui-core/src/model/run.rs` | is_terminal, a_terminal_run_status_is_one_that_reaches_nothing, is_terminal, from, new, ... |
| `crates/htui-core/src/prompt/settings.rs` | bitor, or, rhs, other |
| `crates/htui-core/src/prompt/template.rs` | Error |
| `crates/htui-core/src/seed.rs` | every_prefix_passes_the_check |
| `crates/htui-core/src/store/error.rs` | ParseEnum, Backend, 0, Unreachable, 0, ... |
| `crates/htui-core/src/store/mem.rs` | now, quota, now, create_step, create_run, ... |
| `crates/htui-core/src/store/traits.rs` | id, invalid_prefix, Stale, status, to, ... |
| `crates/htui-orch/src/verify.rs` | cannot_wait, err |
| `crates/htui-store/src/embed.rs` | test_dense_embedder, new, embed_sparse, Embedder, dense_model, ... |
| `crates/htui-store/src/error.rs` | id, schema_is_newer, err, err, err, ... |
| `crates/htui-store/src/identity.rs` | config_root |
| `crates/htui-store/src/pg/mod.rs` | pending |
| `crates/htui-store/src/pg/read.rs` | id, id, item_kind_rows, box_id, item_kind, ... |
| `crates/htui-store/src/pg/write.rs` | create_item_kind, patch, version, graph, item_kinds, ... |
| `crates/htui-store/src/qdrant_settings.rs` | api_key, new, url |
| `crates/htui-store/src/vector.rs` | generate_point_id, search_concepts, upsert, item_id, doc_id, ... |
| `crates/htui-store/src/vector_sync.rs` | sync_docs, text, repo_root, run_once, sync_handoff, ... |
| `crates/htui-store/src/writer.rs` | expected, patch, expected_version, update_workspace, update_project, ... |
| `crates/htui/src/agent_worker.rs` | settings, project_id, _writer, project_caps_for |
| `crates/htui/src/store_worker.rs` | lost_the_server_ignores_a_pass_that_failed_for_another_reason, health, lost_the_server |

## Connected Communities

- **htui-agent/tests +13 dirs** (23 cross-edges)
- **src/store +7 dirs** (14 cross-edges)
- **src/model +14 dirs** (10 cross-edges)
- **src/app +5 dirs** (4 cross-edges)
- **src/isolate +10 dirs** (4 cross-edges)
- **htui-store/src +2 dirs** (3 cross-edges)
- **src/model +3 dirs · matches** (3 cross-edges)
- **htui-orch/src +6 dirs** (3 cross-edges)
- **htui-core · wrap** (2 cross-edges)
- **htui-agent · run** (2 cross-edges)
- **htui-store/tests +13 dirs** (2 cross-edges)
- **htui/tests +14 dirs** (2 cross-edges)
- **src/install +6 dirs** (2 cross-edges)
- **src/pg · kind_not_in_project** (1 cross-edges)
- **src/model +3 dirs · BoxRow** (1 cross-edges)
- **src/model · from_settings** (1 cross-edges)
- **htui/tests +9 dirs** (1 cross-edges)
- **htui-core/src · can_move_to · run · traits (4) #1** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-104")
explore(operation:"context", task:"understand htui-store/src +9 dirs", format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
