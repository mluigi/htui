---
name: gortex-src-model-14-dirs
description: "Work in the src/model +14 dirs area — 445 symbols across 36 files (52% cohesion)"
---

# src/model +14 dirs

445 symbols | 36 files | 52% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/tests/cli_map.rs`
- `crates/htui-agent/tests/excerpt.rs`
- `crates/htui-agent/tests/recorder.rs`
- `crates/htui-core/src/fixtures.rs`
- `crates/htui-core/src/model/document.rs`
- `crates/htui-core/src/model/hierarchy.rs`
- `crates/htui-core/src/model/item.rs`
- `crates/htui-core/src/model/kind.rs`
- `crates/htui-core/src/model/link.rs`
- `crates/htui-core/src/model/quota.rs`
- `crates/htui-core/src/model/run.rs`
- `crates/htui-core/src/model/scope.rs`
- `crates/htui-core/src/prompt/excerpt.rs`
- `crates/htui-core/src/prompt/fixtures.rs`
- `crates/htui-core/src/prompt/trim.rs`
- `crates/htui-core/src/seed.rs`
- `crates/htui-core/src/store/conformance.rs`
- `crates/htui-core/src/store/mem.rs`
- `crates/htui-core/src/store/traits.rs`
- `crates/htui-core/tests/mem_store.rs`
- `crates/htui-core/tests/prompt_digest.rs`
- `crates/htui-orch/src/conformance.rs`
- `crates/htui-orch/src/engine.rs`
- `crates/htui-orch/tests/fake_conformance.rs`
- `crates/htui-store/src/backend.rs`
- `crates/htui-store/src/cache/read.rs`
- `crates/htui-store/src/pg/read.rs`
- `crates/htui-store/src/pg/write.rs`
- `crates/htui-store/src/writer.rs`
- `crates/htui-store/tests/cache.rs`
- `crates/htui-store/tests/pg_criteria.rs`
- `crates/htui/src/ui/overlay/registry.rs`
- `crates/htui/src/ui/overlay/workspace_switcher.rs`
- `crates/htui/src/ui/tabs/backlog/detail/runs.rs`
- `crates/htui/src/ui/tabs/settings/kinds.rs`
- `crates/htui/src/ui/tabs/settings/prompt.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/tests/cli_map.rs` | a_thinking_block_is_a_thought_with_no_prose_and_never_its_signature, the_denial_is_recorded_in_the_position_it_occurred, other_updates, a_result_reporting_less_than_the_session_total_is_not_a_measurement, a_rate_limit_event_is_an_other_row_and_rides_the_turns_usage, ... |
| `crates/htui-agent/tests/excerpt.rs` | the_reader_and_the_pass_run_under_one_max_file_bytes |
| `crates/htui-agent/tests/recorder.rs` | new, create_project |
| `crates/htui-core/src/fixtures.rs` | class, users, steps, boxes, every_id_is_distinct, ... |
| `crates/htui-core/src/model/document.rs` | item_id, version, kind, created_by, produced_by_step_id, ... |
| `crates/htui-core/src/model/hierarchy.rs` | updated_at, created_by, created_by, id, description, ... |
| `crates/htui-core/src/model/item.rs` | summary |
| `crates/htui-core/src/model/kind.rs` | is_override, updated_at, project_id, name, description, ... |
| `crates/htui-core/src/model/link.rs` | id, node |
| `crates/htui-core/src/model/quota.rs` | the_source_vocabulary_is_the_four_strings_of_the_settings_document |
| `crates/htui-core/src/model/run.rs` | a_terminal_step_status_is_done_cancelled_or_superseded |
| `crates/htui-core/src/model/scope.rs` | project, project, from_workspace, project_only, scope, ... |
| `crates/htui-core/src/prompt/excerpt.rs` | max_file_bytes |
| `crates/htui-core/src/prompt/fixtures.rs` | demo_trim_record |
| `crates/htui-core/src/prompt/trim.rs` | lines, content, kept, reclaim, tail_only, ... |
| `crates/htui-core/src/seed.rs` | rows_carry_the_frozen_defaults, graph_description, body, now, kind_names_and_prefixes_are_unique, ... |
| `crates/htui-core/src/store/conformance.rs` | fixture_document_body, project_create_seeds_the_catalogue, S, S, F, ... |
| `crates/htui-core/src/store/mem.rs` | phase, neighbourhood, create_project, link_node, item_kinds, ... |
| `crates/htui-core/src/store/traits.rs` | resolve_inputs, id, item, item_kind_is_held, kinds, ... |
| `crates/htui-core/tests/mem_store.rs` | mem_store_read_conformance |
| `crates/htui-core/tests/prompt_digest.rs` | a_reordered_body_reorders_sections_and_changes_the_digest |
| `crates/htui-orch/src/conformance.rs` | cases_are_unique_and_fifty_two |
| `crates/htui-orch/src/engine.rs` | required_inputs, walk, chosen, forward, phase, ... |
| `crates/htui-orch/tests/fake_conformance.rs` | case_names_are_unique |
| `crates/htui-store/src/backend.rs` | item, kinds, run, resolve_inputs |
| `crates/htui-store/src/cache/read.rs` | run, item, id, resolve_inputs, kinds, ... |
| `crates/htui-store/src/pg/read.rs` | documents, id, step_graph_rows, runs, item, ... |
| `crates/htui-store/src/pg/write.rs` | new, create_step_graph, project, step_graphs |
| `crates/htui-store/src/writer.rs` | create_step_graph, create_project, documents, new, new, ... |
| `crates/htui-store/tests/cache.rs` | mem, mem, documents_agree, cache, scope, ... |
| `crates/htui-store/tests/pg_criteria.rs` | inherent_reads_answer_the_fixture |
| `crates/htui/src/ui/overlay/registry.rs` | iter |
| `crates/htui/src/ui/overlay/workspace_switcher.rs` | reply, ctx, on_reply |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | steps |
| `crates/htui/src/ui/tabs/settings/kinds.rs` | reload, kind, lines, locate_kind, id, ... |
| `crates/htui/src/ui/tabs/settings/prompt.rs` | target, holds |

## Entry Points

- `crates/htui-core/src/store/mem.rs::the_eleven_inherent_reads_answer_from_the_fixture`
- `crates/htui-core/src/store/mem.rs::delete_project_leaves_no_row_in_any_map`
- `crates/htui-store/tests/pg_criteria.rs::inherent_reads_answer_the_fixture`

## Connected Communities

- **src/model +3 dirs · matches** (19 cross-edges)
- **htui-agent/tests +13 dirs** (16 cross-edges)
- **htui/tests +14 dirs** (13 cross-edges)
- **htui-agent/tests +4 dirs · json** (9 cross-edges)
- **src/store +5 dirs · UpstreamEntry** (6 cross-edges)
- **htui-store/tests +13 dirs** (5 cross-edges)
- **htui/tests +1 dirs** (5 cross-edges)
- **src/model +25 dirs** (5 cross-edges)
- **htui-store/src +9 dirs** (4 cross-edges)
- **src/prompt +5 dirs** (4 cross-edges)
- **src/prompt +2 dirs · phase_implement_attempt2** (4 cross-edges)
- **htui-core · wrap** (2 cross-edges)
- **htui-agent/src +7 dirs** (2 cross-edges)
- **src/store +7 dirs** (2 cross-edges)
- **src/cache · get** (2 cross-edges)
- **src/isolate +10 dirs** (2 cross-edges)
- **htui-store/src +8 dirs** (2 cross-edges)
- **src/model +3 dirs · BoxRow** (2 cross-edges)
- **src/store +4 dirs · workspace_links_and_box_paths_u…** (2 cross-edges)
- **tabs/settings +11 dirs** (2 cross-edges)
- **htui-agent/tests +4 dirs · boxed** (2 cross-edges)
- **tabs/settings · text** (1 cross-edges)
- **src/store +2 dirs** (1 cross-edges)
- **src/prompt +2 dirs · parse** (1 cross-edges)
- **htui-orch/src +3 dirs · PhaseAgent** (1 cross-edges)
- **htui-agent/tests +1 dirs · add_payload** (1 cross-edges)
- **htui-orch/src +6 dirs** (1 cross-edges)
- **htui-agent/tests +3 dirs · lock** (1 cross-edges)
- **htui · bench_with · keymap** (1 cross-edges)
- **src/ui · on_key** (1 cross-edges)
- **src/prompt · app** (1 cross-edges)
- **src/model +3 dirs · text** (1 cross-edges)
- **src/model · bound** (1 cross-edges)
- **src/store +4 dirs · RepoBoxPath** (1 cross-edges)
- **htui-core · SectionName** (1 cross-edges)
- **src/store +5 dirs · Note** (1 cross-edges)
- **htui-agent/tests +2 dirs · read** (1 cross-edges)
- **htui-agent/tests +7 dirs** (1 cross-edges)
- **src/store +1 dirs** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-244")
explore(operation:"context", task:"understand src/model +14 dirs", format:"gcx")
relations(operation:"usages", target:{symbol:"crates/htui-core/src/store/mem.rs::the_eleven_inherent_reads_answer_from_the_fixture"}, format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
