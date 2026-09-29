---
name: gortex-htui-tests-14-dirs
description: "Work in the htui/tests +14 dirs area — 796 symbols across 50 files (73% cohesion)"
---

# htui/tests +14 dirs

796 symbols | 50 files | 73% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/conformance.rs`
- `crates/htui-agent/src/driver.rs`
- `crates/htui-agent/src/fake.rs`
- `crates/htui-agent/src/probe.rs`
- `crates/htui-agent/src/registry.rs`
- `crates/htui-agent/tests/acp_conformance.rs`
- `crates/htui-agent/tests/cli_conformance.rs`
- `crates/htui-agent/tests/driver_contract.rs`
- `crates/htui-agent/tests/extensibility.rs`
- `crates/htui-agent/tests/fake_conformance.rs`
- `crates/htui-agent/tests/recorder.rs`
- `crates/htui-core/src/fixtures.rs`
- `crates/htui-core/src/store/mem.rs`
- `crates/htui-core/src/store/traits.rs`
- `crates/htui-core/tests/mem_store.rs`
- `crates/htui-orch/src/verify.rs`
- `crates/htui-orch/tests/fake_conformance.rs`
- `crates/htui-store/src/backend.rs`
- `crates/htui-store/src/cache/refresh.rs`
- `crates/htui-store/src/connect.rs`
- `crates/htui-store/src/writer.rs`
- `crates/htui/src/agent_worker.rs`
- `crates/htui/src/app/update.rs`
- `crates/htui/src/connection.rs`
- `crates/htui/src/keymap.rs`
- `crates/htui/src/lib.rs`
- `crates/htui/src/main.rs`
- `crates/htui/src/preview.rs`
- `crates/htui/src/prompt_settings.rs`
- `crates/htui/src/qdrant_settings_info.rs`
- `crates/htui/src/store_worker.rs`
- `crates/htui/src/terminal.rs`
- `crates/htui/src/testkit.rs`
- `crates/htui/src/ui/overlay/migration_prompt.rs`
- `crates/htui/src/ui/tabs/chat/mod.rs`
- `crates/htui/src/ui/tabs/settings/agents.rs`
- `crates/htui/src/ui/tabs/settings/connection.rs`
- `crates/htui/src/ui/tabs/settings/prompt.rs`
- `crates/htui/tests/chat.rs`
- `crates/htui/tests/chat_live.rs`
- `crates/htui/tests/chat_live_agy.rs`
- `crates/htui/tests/chat_live_cli.rs`
- `crates/htui/tests/chat_offline.rs`
- `crates/htui/tests/connection.rs`
- `crates/htui/tests/hierarchy.rs`
- `crates/htui/tests/kinds.rs`
- `crates/htui/tests/prompt_preview.rs`
- `crates/htui/tests/replay.rs`
- `crates/htui/tests/settings.rs`
- `crates/htui/tests/shell.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/conformance.rs` | step, Fut, agent, run_all, F, ... |
| `crates/htui-agent/src/driver.rs` | as_str |
| `crates/htui-agent/src/fake.rs` | FakeAdapter, load, new, script, script, ... |
| `crates/htui-agent/src/probe.rs` | platform_key, host, default, without_versions, default_install_root, ... |
| `crates/htui-agent/src/registry.rs` | TransportBuilder, agent, driver_for, id, DriverFactory, ... |
| `crates/htui-agent/tests/acp_conformance.rs` | new, the_acp_transport_passes_every_case |
| `crates/htui-agent/tests/cli_conformance.rs` | the_cli_transport_passes_every_case |
| `crates/htui-agent/tests/driver_contract.rs` | every_adapter, every_adapter |
| `crates/htui-agent/tests/extensibility.rs` | an_unknown_agent_reaches_a_driver_from_its_row_alone, ZetaHarness, the_resolved_token_reaches_no_persisted_row, the_unknown_agent_passes_every_conformance_case, the_factory_is_keyed_by_transport_not_by_agent, ... |
| `crates/htui-agent/tests/fake_conformance.rs` | fake_driver_passes_every_case |
| `crates/htui-agent/tests/recorder.rs` | upsert_agent, agent |
| `crates/htui-core/src/fixtures.rs` | agents |
| `crates/htui-core/src/store/mem.rs` | MemStore, from_demo, data, project_settings, box_info, ... |
| `crates/htui-core/src/store/traits.rs` | upsert_agent, agent |
| `crates/htui-core/tests/mem_store.rs` | demo_store_loads_the_fixture, mem_store_conformance |
| `crates/htui-orch/src/verify.rs` | Verifier |
| `crates/htui-orch/tests/fake_conformance.rs` | fake_orchestrator_passes_every_case |
| `crates/htui-store/src/backend.rs` | box_info, memory, cache, cache, pg, ... |
| `crates/htui-store/src/cache/refresh.rs` | abort, handle, Refresher, wake, health |
| `crates/htui-store/src/connect.rs` | offline_opens_the_cache_and_never_dials |
| `crates/htui-store/src/writer.rs` | a_memory_backend_answers_the_fixture_user |
| `crates/htui/src/agent_worker.rs` | agent_id, start_login, extra, install, a_chat_latches_the_quota_of_the_row_it_runs_on, ... |
| `crates/htui/src/app/update.rs` | observe_reply, reply |
| `crates/htui/src/connection.rs` | err, snapshot, request, seam_sentence, backend, ... |
| `crates/htui/src/keymap.rs` | label |
| `crates/htui/src/lib.rs` | run, args |
| `crates/htui/src/main.rs` | main |
| `crates/htui/src/preview.rs` | offline_refusal |
| `crates/htui/src/prompt_settings.rs` | cas, scope, scope, T, writer, ... |
| `crates/htui/src/qdrant_settings_info.rs` | fetch |
| `crates/htui/src/store_worker.rs` | runtime, RequestEnvelope, reply, tx, seq, ... |
| `crates/htui/src/terminal.rs` | init |
| `crates/htui/src/testkit.rs` | with_agent_runtime, runtime, over, width, height, ... |
| `crates/htui/src/ui/overlay/migration_prompt.rs` | on_reply, reply, _ctx |
| `crates/htui/src/ui/tabs/chat/mod.rs` | transcript, on_scope_change, shell, ID, _ctx, ... |
| `crates/htui/src/ui/tabs/settings/agents.rs` | on_reply, on_auth_frame, reply, frame, clamp_cursor, ... |
| `crates/htui/src/ui/tabs/settings/connection.rs` | on_reply, reply, _ctx |
| `crates/htui/src/ui/tabs/settings/prompt.rs` | reply, on_reply, _ctx |
| `crates/htui/tests/chat.rs` | transport, caps, SharedAdapter, 0, harness_with, ... |
| `crates/htui/tests/chat_live.rs` | a_real_claude_session_streams_into_the_store_and_then_ends, matching_processes |
| `crates/htui/tests/chat_live_agy.rs` | a_real_agy_session_streams_three_turns_into_the_store_and_answers_11_14, choose, options, matching_processes |
| `crates/htui/tests/chat_live_cli.rs` | store, key, key_sum, matching_processes, rows, ... |
| `crates/htui/tests/chat_offline.rs` | offline_harness, script, 0, an_online_chat_header_says_nothing_about_a_buffer, SharedAdapter, ... |
| `crates/htui/tests/connection.rs` | a_demo_session_never_redirects, demo, try_serve_refuses_every_writer_by_name, memory_answers_not_applicable_and_never_opens_the_keyring |
| `crates/htui/tests/hierarchy.rs` | a_canonical_path_that_is_not_utf8_is_refused, reply, delete_workspace_keeps_its_projects, an_unlinked_project_is_refused_before_anything_is_written, a_box_with_no_row_is_refused_before_the_path_is_read, ... |
| `crates/htui/tests/kinds.rs` | a_two_project_scope_answers_both_in_scope_order, no_workspace_says_so, the_demo_catalogue_renders_the_tree, q_quits_from_browse, kinds_over, ... |
| `crates/htui/tests/prompt_preview.rs` | a_second_preview_from_one_origin_aborts_the_first, origin, preview_request, platform_scope, the_default_template_is_the_first_default_name_the_project_has, ... |
| `crates/htui/tests/replay.rs` | _ctx, on_reply, reply |
| `crates/htui/tests/settings.rs` | an_empty_registry_says_so_rather_than_drawing_a_bare_table, store, an_agent_no_fixture_contains_appears_from_its_row_alone, the_demo_registry_lists_every_seeded_agent, settings_over |
| `crates/htui/tests/shell.rs` | store_state, migrations_pending, label |

## Entry Points

- `crates/htui/tests/chat_live_agy.rs::a_real_agy_session_streams_three_turns_into_the_store_and_answers_11_14`
- `crates/htui/tests/chat_live_cli.rs::a_real_claude_cli_session_streams_two_turns_into_the_store_and_then_ends`
- `crates/htui-agent/tests/extensibility.rs::the_resolved_token_reaches_no_persisted_row`
- `crates/htui/tests/chat_live.rs::a_real_claude_session_streams_into_the_store_and_then_ends`
- `crates/htui/src/agent_worker.rs::a_scripted_chat_records_its_turn_and_closes_its_run`

## Connected Communities

- **src/model +14 dirs** (46 cross-edges)
- **htui-store/src +4 dirs · close** (27 cross-edges)
- **htui-agent/src +7 dirs** (26 cross-edges)
- **htui/tests +3 dirs** (19 cross-edges)
- **htui-store/tests +13 dirs** (18 cross-edges)
- **htui-agent/tests +13 dirs** (17 cross-edges)
- **tabs/settings +11 dirs** (15 cross-edges)
- **htui-agent/src +4 dirs · record** (11 cross-edges)
- **src/isolate +10 dirs** (10 cross-edges)
- **htui/tests +9 dirs** (10 cross-edges)
- **htui-agent/tests +3 dirs · lock** (9 cross-edges)
- **htui-agent/tests +4 dirs · boxed** (9 cross-edges)
- **htui/tests +2 dirs** (8 cross-edges)
- **htui-agent/tests +4 dirs · json** (8 cross-edges)
- **htui/tests +6 dirs** (7 cross-edges)
- **htui-agent/src +2 dirs · build** (7 cross-edges)
- **htui-store/src +2 dirs** (7 cross-edges)
- **src/app +5 dirs** (5 cross-edges)
- **htui · drive_to_end** (5 cross-edges)
- **src/install +6 dirs** (5 cross-edges)
- **src/ui · apply** (5 cross-edges)
- **htui/tests +1 dirs** (4 cross-edges)
- **htui-store/src +9 dirs** (3 cross-edges)
- **htui/src · default_global** (3 cross-edges)
- **src/prompt +5 dirs** (3 cross-edges)
- **htui-agent/src · adapter_id_from** (2 cross-edges)
- **htui-agent/src +2 dirs · DriverEvent** (2 cross-edges)
- **htui · bench_with · keymap** (2 cross-edges)
- **src/model +25 dirs** (2 cross-edges)
- **htui-orch/src +6 dirs** (2 cross-edges)
- **src/cache · run_pass** (1 cross-edges)
- **htui/src · needs_reprobe** (1 cross-edges)
- **htui · vulkan_scope** (1 cross-edges)
- **htui/src · TerminalGuard** (1 cross-edges)
- **htui-agent/src · evaluate** (1 cross-edges)
- **src/install +4 dirs** (1 cross-edges)
- **htui-agent · push** (1 cross-edges)
- **htui-store/src · active_runs** (1 cross-edges)
- **htui-agent/tests +7 dirs** (1 cross-edges)
- **htui-agent/tests · CliHarness** (1 cross-edges)
- **htui-agent/src +4 dirs · DriverCaps** (1 cross-edges)
- **tabs/chat · entering_replay_disarms_the_can…** (1 cross-edges)
- **htui-store/src · run** (1 cross-edges)
- **htui-agent/tests +2 dirs · heartbeat** (1 cross-edges)
- **src/model · usage_row** (1 cross-edges)
- **htui-orch/src +12 dirs** (1 cross-edges)
- **htui/tests · backlog** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-233")
explore(operation:"context", task:"understand htui/tests +14 dirs", format:"gcx")
relations(operation:"usages", target:{symbol:"crates/htui/tests/chat_live_agy.rs::a_real_agy_session_streams_three_turns_into_the_store_and_answers_11_14"}, format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
