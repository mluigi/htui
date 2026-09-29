---
name: gortex-htui-store-tests-13-dirs
description: "Work in the htui-store/tests +13 dirs area — 575 symbols across 35 files (81% cohesion)"
---

# htui-store/tests +13 dirs

575 symbols | 35 files | 81% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/acp/mod.rs`
- `crates/htui-agent/src/conformance.rs`
- `crates/htui-agent/src/fake.rs`
- `crates/htui-agent/tests/acp_map.rs`
- `crates/htui-core/src/model/item.rs`
- `crates/htui-core/src/model/overlap.rs`
- `crates/htui-core/src/prompt/fixtures.rs`
- `crates/htui-core/src/store/mem.rs`
- `crates/htui-orch/src/engine.rs`
- `crates/htui-orch/src/isolate/real.rs`
- `crates/htui-orch/src/recover.rs`
- `crates/htui-orch/src/verify.rs`
- `crates/htui-orch/tests/gix_isolator.rs`
- `crates/htui-store/src/backend.rs`
- `crates/htui-store/src/cache/mod.rs`
- `crates/htui-store/src/cache/read.rs`
- `crates/htui-store/src/cache/refresh.rs`
- `crates/htui-store/src/connect.rs`
- `crates/htui-store/src/dsn.rs`
- `crates/htui-store/src/identity.rs`
- `crates/htui-store/src/pg/demo.rs`
- `crates/htui-store/src/pg/mod.rs`
- `crates/htui-store/src/pg/read.rs`
- `crates/htui-store/src/pg/write.rs`
- `crates/htui-store/src/testkit.rs`
- `crates/htui-store/tests/cache.rs`
- `crates/htui-store/tests/connect.rs`
- `crates/htui-store/tests/dsn.rs`
- `crates/htui-store/tests/migrations.rs`
- `crates/htui-store/tests/pg_conformance.rs`
- `crates/htui-store/tests/pg_criteria.rs`
- `crates/htui/tests/chat_offline.rs`
- `crates/htui/tests/hierarchy.rs`
- `crates/htui/tests/kinds.rs`
- `crates/htui/tests/prompt_settings.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/acp/mod.rs` | prompt, start, io, spec, spec |
| `crates/htui-agent/src/conformance.rs` | item, mint_item, id, project, create_project, ... |
| `crates/htui-agent/src/fake.rs` | open, caps, name, script, spec |
| `crates/htui-agent/tests/acp_map.rs` | an_update_kind_the_schema_never_heard_of_still_lands_in_other |
| `crates/htui-core/src/model/item.rs` | statuses, ItemFilter, text, ready, tags, ... |
| `crates/htui-core/src/model/overlap.rs` | claim_display_names_the_rule_and_the_holding_run, is_admitted |
| `crates/htui-core/src/prompt/fixtures.rs` | handoff_event, handoff_events, turn, payload, seq, ... |
| `crates/htui-core/src/store/mem.rs` | set_agent_box_quota_leaves_probe_and_version_alone |
| `crates/htui-orch/src/engine.rs` | app, min_budget_reads_only_a_positive_integer, min_budget |
| `crates/htui-orch/src/isolate/real.rs` | checkouts, step, run, acquire |
| `crates/htui-orch/src/recover.rs` | commit, after, repo, step |
| `crates/htui-orch/src/verify.rs` | request, execute, no_primary_tree, deadline_elapsed, without_running, ... |
| `crates/htui-orch/tests/gix_isolator.rs` | key, step, phase, commit |
| `crates/htui-store/src/backend.rs` | root, cache |
| `crates/htui-store/src/cache/mod.rs` | key, dir, value, pool, open, ... |
| `crates/htui-store/src/cache/read.rs` | name, this_user, user_named |
| `crates/htui-store/src/cache/refresh.rs` | RefreshSettings, this_box, overlap, interval, transcript_steps, ... |
| `crates/htui-store/src/connect.rs` | connect_timeout, offline, refresh_settings, 0, dsn, ... |
| `crates/htui-store/src/dsn.rs` | as_str, mode, summary, ssl_mode_name, fingerprint |
| `crates/htui-store/src/identity.rs` | os_user_name, Identity, dsn, hostname, cache_dir, ... |
| `crates/htui-store/src/pg/demo.rs` | load_demo, data |
| `crates/htui-store/src/pg/mod.rs` | connect_timeout, identity, pool, migrations, lazy, ... |
| `crates/htui-store/src/pg/read.rs` | run, id, id, repo_row |
| `crates/htui-store/src/pg/write.rs` | begin_repeatable_read, from, entity, append_events, box_id, ... |
| `crates/htui-store/src/testkit.rs` | with_database, cache, url, demo_db, fresh_db, ... |
| `crates/htui-store/tests/cache.rs` | canonical, open_cache_named, a_running_step_is_mirrored_only_once_it_has_finished, platform_scope, step, ... |
| `crates/htui-store/tests/connect.rs` | offline_never_dials_and_starts_at_an_age, start_opens_the_mirror_offline_and_reports_online_over_a_migrated_database, started, an_online_backend_falls_back_onto_its_own_mirror, close, ... |
| `crates/htui-store/tests/dsn.rs` | apply_dsn_stores_then_opens_the_new_mirror, root, apply_dsn_leaves_the_old_mirror_on_disk, context |
| `crates/htui-store/tests/migrations.rs` | load_demo_round_trips_a_count_per_table, the_ana_column_comments_are_present_and_verbatim, agent_box_gains_a_jsonb_probe_column, register_box_upserts_and_adopts, connect_writes_no_files_of_its_own, ... |
| `crates/htui-store/tests/pg_conformance.rs` | pg_store_read_conformance, pg_store_conformance |
| `crates/htui-store/tests/pg_criteria.rs` | race_item, outcome, cas_tokens_advance_by_the_trigger_alone, step_usage, set_step_usage_keeps_the_digest_a_none_call_does_not_supply, ... |
| `crates/htui/tests/chat_offline.rs` | agent_id, offline_mirror |
| `crates/htui/tests/hierarchy.rs` | offline_refuses_every_hierarchy_request_by_name |
| `crates/htui/tests/kinds.rs` | offline_refuses_every_catalogue_request_by_name |
| `crates/htui/tests/prompt_settings.rs` | offline_refuses_all_three_by_name, a_migrated_postgres_presents_ten_app_rows_with_tokens |

## Entry Points

- `crates/htui-store/tests/pg_criteria.rs::inherent_orchestration_reads_answer_the_fixture`
- `crates/htui-store/tests/cache.rs::run_step_tree_refreshes_off_its_parent_step`
- `crates/htui-store/tests/cache.rs::mirror_reads_equal_the_reference_store`
- `crates/htui-store/tests/cache.rs::the_0003_columns_reach_the_mirror`
- `crates/htui-store/tests/pg_criteria.rs::command_run_round_trips_and_orders_by_queued_at`

## Connected Communities

- **src/model +14 dirs** (80 cross-edges)
- **htui-agent/tests +13 dirs** (20 cross-edges)
- **htui-store/src +4 dirs · close** (14 cross-edges)
- **htui/tests +14 dirs** (11 cross-edges)
- **src/isolate +10 dirs** (8 cross-edges)
- **htui-store** (6 cross-edges)
- **src/install +6 dirs** (6 cross-edges)
- **src/model +3 dirs · text** (4 cross-edges)
- **src/pg · project_reach** (4 cross-edges)
- **htui-agent/tests +3 dirs · lock** (4 cross-edges)
- **htui/tests +9 dirs** (3 cross-edges)
- **htui-agent/tests +4 dirs · json** (3 cross-edges)
- **htui-store/src +9 dirs** (3 cross-edges)
- **htui · bench_with · keymap** (3 cross-edges)
- **htui-store/src +2 dirs** (3 cross-edges)
- **htui-orch/src · from_app** (2 cross-edges)
- **src/cache · get** (2 cross-edges)
- **src/model +3 dirs · matches** (2 cross-edges)
- **htui/tests +1 dirs** (2 cross-edges)
- **src/store +7 dirs** (2 cross-edges)
- **htui-orch/src · scrubbed** (1 cross-edges)
- **htui-agent · session_main** (1 cross-edges)
- **htui-agent · ResolvedLaunch** (1 cross-edges)
- **htui/tests +6 dirs** (1 cross-edges)
- **htui-agent/src +3 dirs** (1 cross-edges)
- **htui-agent · spec** (1 cross-edges)
- **src/acp · map** (1 cross-edges)
- **htui-agent · run** (1 cross-edges)
- **src/cache · run_pass** (1 cross-edges)
- **htui-orch/src · spawn_and_wait** (1 cross-edges)
- **htui · vulkan_scope** (1 cross-edges)
- **htui-agent/tests +4 dirs · Item** (1 cross-edges)
- **htui-agent/src +4 dirs · record** (1 cross-edges)
- **htui-agent/tests +2 dirs · heartbeat** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-218")
explore(operation:"context", task:"understand htui-store/tests +13 dirs", format:"gcx")
relations(operation:"usages", target:{symbol:"crates/htui-store/tests/pg_criteria.rs::inherent_orchestration_reads_answer_the_fixture"}, format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
