---
name: gortex-htui-tests-9-dirs
description: "Work in the htui/tests +9 dirs area — 284 symbols across 23 files (67% cohesion)"
---

# htui/tests +9 dirs

284 symbols | 23 files | 67% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/conformance.rs`
- `crates/htui-core/src/model/scope.rs`
- `crates/htui-store/src/backend.rs`
- `crates/htui/src/agent_worker.rs`
- `crates/htui/src/catalogue.rs`
- `crates/htui/src/store_worker.rs`
- `crates/htui/src/testkit.rs`
- `crates/htui/src/ui/overlay/migration_prompt.rs`
- `crates/htui/src/ui/overlay/registry.rs`
- `crates/htui/src/ui/overlay/workspace_switcher.rs`
- `crates/htui/src/ui/tabs/backlog/mod.rs`
- `crates/htui/src/ui/tabs/chat/mod.rs`
- `crates/htui/src/ui/tabs/registry.rs`
- `crates/htui/src/ui/tabs/settings/agents.rs`
- `crates/htui/src/ui/tabs/settings/hierarchy.rs`
- `crates/htui/src/ui/tabs/settings/mod.rs`
- `crates/htui/src/ui/tabs/settings/qdrant.rs`
- `crates/htui/src/ui/tabs/skills.rs`
- `crates/htui/tests/hierarchy.rs`
- `crates/htui/tests/kinds.rs`
- `crates/htui/tests/prompt_settings.rs`
- `crates/htui/tests/replay.rs`
- `crates/htui/tests/settings.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/conformance.rs` | create_item_kind, scope, links, id, id, ... |
| `crates/htui-core/src/model/scope.rs` | workspace_id, project_ids, Scope |
| `crates/htui-store/src/backend.rs` | items, documents, step, scope, filter, ... |
| `crates/htui/src/agent_worker.rs` | start, prompt, agent_id |
| `crates/htui/src/catalogue.rs` | writer, backend, reread, T, store, ... |
| `crates/htui/src/store_worker.rs` | 0, scope, remote_url, SetDsn, StoreRequest, ... |
| `crates/htui/src/testkit.rs` | wants_requests, _scope |
| `crates/htui/src/ui/overlay/migration_prompt.rs` | _scope, wants_requests |
| `crates/htui/src/ui/overlay/registry.rs` | wants_requests, scope |
| `crates/htui/src/ui/overlay/workspace_switcher.rs` | _scope, wants_requests |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | wants_requests, scope |
| `crates/htui/src/ui/tabs/chat/mod.rs` | wants_requests, _scope |
| `crates/htui/src/ui/tabs/registry.rs` | wants_requests, scope |
| `crates/htui/src/ui/tabs/settings/agents.rs` | wants_requests, _scope |
| `crates/htui/src/ui/tabs/settings/hierarchy.rs` | wants_requests, scope |
| `crates/htui/src/ui/tabs/settings/mod.rs` | wants_requests, scope, wants_requests, scope |
| `crates/htui/src/ui/tabs/settings/qdrant.rs` | wants_requests, _scope |
| `crates/htui/src/ui/tabs/skills.rs` | _scope, wants_requests |
| `crates/htui/tests/hierarchy.rs` | delete_reach_equals_deleted_reach, hierarchy_requests, hierarchy_names_are_stable |
| `crates/htui/tests/kinds.rs` | nil_scope, catalogue_names_are_stable, catalogue_requests |
| `crates/htui/tests/prompt_settings.rs` | prompt_settings_names_are_stable, prompt_requests, nil_scope, serve_refuses_a_foreign_request_by_name |
| `crates/htui/tests/replay.rs` | wants_requests, _scope |
| `crates/htui/tests/settings.rs` | the_section_asks_for_the_registry_once_and_unscoped, wants_requests, _scope, demo_scope, _scope, ... |

## Connected Communities

- **htui/tests +14 dirs** (11 cross-edges)
- **src/model +14 dirs** (7 cross-edges)
- **htui/tests +6 dirs** (3 cross-edges)
- **src/prompt +5 dirs** (2 cross-edges)
- **htui-store/tests +13 dirs** (2 cross-edges)
- **htui-agent/src +2 dirs · build** (1 cross-edges)
- **htui-store/src +4 dirs · links** (1 cross-edges)
- **src/store +5 dirs · Note** (1 cross-edges)
- **htui-store/src +4 dirs · RunSummary** (1 cross-edges)
- **htui · bench_with · keymap** (1 cross-edges)
- **htui-store/src · active_runs** (1 cross-edges)
- **htui-store/src +4 dirs · close** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-240")
explore(operation:"context", task:"understand htui/tests +9 dirs", format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
