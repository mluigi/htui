# Graph Report - htui  (2026-09-15)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 6394 nodes · 17483 edges · 281 communities (232 shown, 41 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 373 edges (avg confidence: 0.84)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `3e346107`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- recorder.rs
- State
- Ctx
- htui-agent/tests/probe.rs
- cli/mod.rs
- App
- Result
- DriverEnvelope
- record.rs
- cli_live.rs
- ResolvedLaunch
- src/probe.rs
- store_worker.rs
- agent_worker.rs
- PgStore
- validate-workflow-docs.sh
- render.rs
- Backend
- src/conformance.rs
- src/fixtures.rs
- cache.rs
- quota.rs
- refresh.rs
- .new
- Theme
- Result
- scrubber
- Event
- update.rs
- cli_driver.rs
- UsageSpy<'_, S>
- .serve
- ReadStore
- .new
- DriverCaps
- htui-agent/tests/auth.rs
- SettingsSection
- Overlay
- driver.rs
- FakeSession
- tests/launch.rs
- runs.rs
- install/mod.rs
- Layout
- template.rs
- htui/tests/install.rs
- claude.rs
- store/conformance.rs
- htui/tests/auth.rs
- Harness
- run_pass
- AgentRuntime
- cache/mod.rs
- tests/settings.rs
- Staged
- Fixture
- Handshake
- cli_map.rs
- http.rs
- SessionEvent
- WorkspaceSummary
- RunSummary
- keymap.rs
- agy_live.rs
- src/connect.rs
- chat.rs
- install/registry.rs
- wire_flow
- prompt/settings.rs
- append_pending
- pg_criteria.rs
- validate-workflow-docs.ps1
- cli_conformance.rs
- DriverEvent
- htui-agent/tests/replay.rs
- LinkGraph
- .run
- Vec
- src/permission.rs
- driver_contract.rs
- Document
- WorkspaceSwitcher
- client.rs
- AgentSettings
- chat/mod.rs
- PgStore
- htui-store/src/testkit.rs
- run_turn
- InstallError
- map.rs
- htui/tests/replay.rs
- Transcript
- BoundSkill
- transcript.rs
- writer_buffered.rs
- acp_conformance.rs
- acp/auth.rs
- AuthEvent
- plan
- caps_for
- auth_live.rs
- BoxProfile
- secret.rs
- migrations.rs
- T70: criteria sweep and MOD-2 close-out
- tools.rs
- htui-agent/tests/install.rs
- ItemSummary
- MigrationPrompt
- ChatTab
- StepGraphPhase
- fs.rs
- AgentBox
- extensibility.rs
- Agent
- DemoData
- usage.rs
- Fixture
- shell.rs
- PgStore
- chat_offline.rs
- MOD-2 — agent driver + chat tab
- probe_agent
- acp_driver.rs
- boxed
- prompt/fixtures.rs
- Path
- download
- defaults.rs
- composer.rs
- chat_live_agy.rs
- chat_usage_pg.rs
- HANDOFF.md — outstanding work
- model/mod.rs
- htui/src/testkit.rs
- BodyTab
- DocumentsTab
- NotesTab
- identity.rs
- D6: recorder flush, seq/turn, digest, scrub order
- Value
- auth/run.rs
- InstallRecord
- install_live.rs
- PromptSpec
- htui agent_worker.rs: AgentRuntime, run_chat, run_turn
- assemble() — the pure prompt assembler
- docs/ANA-9.md — data model v2
- CONCEPTS.md — standing architectural intent
- .start
- Scope
- estimate.rs
- excerpt.rs
- TerminalGuard
- GraphTab
- htui-store/src/error.rs
- SkillsTab
- chat_live_cli.rs
- htui-agent replay.rs: envelope_from_row, envelope_or_other
- install-workflow-hooks.sh
- browser.rs
- backlog.rs
- T59: template.rs + estimate.rs — scanner and estimator
- quota::normalize
- acp_map.rs
- .new
- SectionName
- run_read_case
- prompt_render.rs
- tests/connect.rs
- acp/mod.rs: AcpDriver, AcpSession, run_session
- Recorder::latch_quota
- PromptSpec
- htui-core prompt module surface (assemble)
- install-workflow-hooks.ps1
- next-item.ps1
- .new
- chat/permission.rs
- Plan: MOD-2 agy over ACP (milestone 6)
- D79: a second registry row, claude-cli
- cli/mod.rs supervisor
- ANA-4: agent driver design authority
- Recorder::record
- T4: the registry reader and the pre-flight
- AgentSummary
- .build
- T15: mirror the agent registry and the offline user
- D18: three install requests, one streamed reply
- htui::preview (build_spec, run_preview, PromptPreview)
- assemble() — the §4.7 pipeline
- TokenEstimator (chars-v2)
- prompt::render — the section renderers
- T39: the §7 blob and the passive latch
- wire_value
- AgentSession
- Seen
- AppUser
- htui/src/lib.rs
- pg/mod.rs
- Blueprint: MOD-2 milestone 8 CLI transport
- D91: DriverCaps gains usage_mid_turn
- Pipeline
- digest.rs
- integration.rs
- Recorder
- MOD-20 Registry-Driven Adapter Install Plan
- D16: staging, promotion, rollback, retention
- UpstreamEntry + sort_canonical
- T40: run-cap enforcement
- .new
- htui-agent/src/lib.rs
- settings_over
- probe_agent ordered steps
- cli/claude.rs mapper
- ProjectCaps
- D17: the consent record is manifest.json on the box's disk
- MOD-2 milestone 6 blueprint — agy over ACP
- prompt::defaults (DEFAULT_TEMPLATES, COMMAND_QUEUE_TEXT)
- sync-workflow-surface.sh
- QuotaLatch
- wait_until
- T5: fetch, verify, unpack, promote, manifest
- MOD-2 Quota and Caps Plan
- acp_live.rs
- snapshot
- event
- pg_conformance.rs
- ChildGuard
- argv
- htui
- D76: the default column shows the whole model id
- fixture_document_body
- row_ids
- main.rs
- D21: Windows written and lint-checked from Linux, verified by MOD-16
- tool_kind
- body_lines
- run_loop.sh
- D87: DriverFactory::with_acp becomes production
- D88: name-keyed seed top-up, not a migration
- D89: Settings name column becomes the flexible one
- D15: open-question rulings (ReadStore conformance, prompt_digest)
- D16: deliberately absent from M1-M2
- D100: assembler scrubs by wrapping each section in Value::String
- D101: assemble() takes a fully resolved Budget
- D102: the preview is a sixth Backlog detail sub-tab
- D103: preview stand-ins declared in trim_record.notes
- D104: ten default bodies ship as DEFAULT_TEMPLATES
- D105: skill model types land here, read-only
- D106: RunStepSummary gains prompt_tokens and trimmed
- D107: all three template roles implemented in the pure assembler
- D109: preview refuses on Backend::Offline (PROMPT_ON_SERVER_ONLY)
- D110: no Repo fixture rows and no repos() read this milestone
- D111: §4.2's command_queue section ships verbatim
- D95: PromptScope is the upstream_summaries argument
- D96: additive READ_CASES harness beside run_case
- D98: excerpt rendering is uniform across agent families
- F-19: literal_text ships behind #[expect(dead_code)]
- F-20: fence delimiter line is billed as code in both directions
- F-21: file toggles match the raw line, fence toggle matches the trimmed line
- F-22: for_model's o[0-9] boundary means opus-4 is the Claude row
- F-23: three B.11 signatures moved
- F-25: the fixture id collision E-5 predicted is real
- F-26: the Judge-role {{item}} claim is false as a general claim
- F-27: none of the ten §5.4 bodies needed a character changed
- F-29: MemStore::documents_of_kinds recursed forever
- F-30: §7.3 SQL had three defects, all found by running it
- F-31: H-29 overstated sqlx's nullability inference
- F-32: the two walks agree transitively through the shared spec
- H-17: latch_quota awaits a store write inside record
- D71: per-run cap enforced, per-batch cap deferred to MOD-12
- T44: close-out
- MOD-28 — rataflow execution view

## God Nodes (most connected - your core abstractions)
1. `Ctx` - 101 edges
2. `Agent` - 85 edges
3. `State` - 78 edges
4. `MemStore` - 76 edges
5. `SessionEvent` - 74 edges
6. `Backend` - 72 edges
7. `Scope` - 70 edges
8. `DriverEnvelope` - 67 edges
9. `DriverEvent` - 67 edges
10. `Harness` - 63 edges

## Surprising Connections (you probably didn't know these)
- `Trait-level conformance suite (store::conformance)` --references--> `MOD-4 — orchestrator, manual mode`  [AMBIGUOUS]
  .claude/plans/mod-1-tui-scaffold.blueprint.md → HANDOFF.md
- `CLEAN-1 plan — cargo doc for htui-agent` --conceptually_related_to--> `MOD-2 — agent driver + chat tab`  [INFERRED]
  .claude/plans/clean-1.plan.md → HANDOFF.md
- `DSN in the OS keyring, no env or config fallback` --conceptually_related_to--> `Postgres is the single source of truth`  [INFERRED]
  README.md → CONCEPTS.md
- `CLEAN-1 plan — cargo doc for htui-agent` --conceptually_related_to--> `sqlx offline query data committed under crates/htui-store/.sqlx`  [AMBIGUOUS]
  .claude/plans/clean-1.plan.md → README.md
- `MOD-11 — htui MCP server` --implements--> `htui MCP server for agent-proposed writes`  [INFERRED]
  HANDOFF.md → CONCEPTS.md

## Import Cycles
- 1-file cycle: `crates/htui/src/cli.rs -> crates/htui/src/cli.rs`
- 2-file cycle: `crates/htui-agent/src/install/manifest.rs -> crates/htui-agent/src/install/mod.rs -> crates/htui-agent/src/install/manifest.rs`
- 2-file cycle: `crates/htui-agent/src/install/archive.rs -> crates/htui-agent/src/install/mod.rs -> crates/htui-agent/src/install/archive.rs`
- 2-file cycle: `crates/htui-store/src/backend.rs -> crates/htui-store/src/writer.rs -> crates/htui-store/src/backend.rs`
- 2-file cycle: `crates/htui/src/store_worker.rs -> crates/htui/src/ui/tabs/registry.rs -> crates/htui/src/store_worker.rs`
- 2-file cycle: `crates/htui/src/store_worker.rs -> crates/htui/src/ui/overlay/registry.rs -> crates/htui/src/store_worker.rs`
- 3-file cycle: `crates/htui-agent/src/install/manifest.rs -> crates/htui-agent/src/install/mod.rs -> crates/htui-agent/src/install/plan.rs -> crates/htui-agent/src/install/manifest.rs`

## Hyperedges (group relationships)
- **The per-run cap breach, detection to closing rows** — claude_plans_mod_2_quota_caps_plan_d69, claude_plans_mod_2_quota_caps_plan_d70, claude_plans_mod_2_quota_caps_plan_d71, claude_plans_mod_2_quota_caps_blueprint_p1, claude_plans_mod_2_quota_caps_plan_t40 [EXTRACTED 0.85]
- **The byte-exact digest flow: LF-normalise, scrub, estimate, trim, canonicalise, hash** — claude_plans_mod_2_prompt_assembler_blueprint_render, claude_plans_mod_2_prompt_assembler_blueprint_scrubber_wrap, claude_plans_mod_2_prompt_assembler_blueprint_estimate_fn, claude_plans_mod_2_prompt_assembler_blueprint_trim, claude_plans_mod_2_prompt_assembler_blueprint_digest, claude_plans_mod_2_prompt_assembler_blueprint_determinism_rules [EXTRACTED 0.85]
- **Milestone 1 opens the store seam once: traits, chat-run ownership, scrubber, recorder, conformance** — _claude_plans_mod_2_driver_seam_registry_plan_d3_store_seam_in_milestone_1, _claude_plans_mod_2_driver_seam_registry_plan_d4_chat_run_ownership, _claude_plans_mod_2_driver_seam_registry_plan_d5_scrubber_seam, _claude_plans_mod_2_driver_seam_registry_plan_d6_recorder_rules, _claude_plans_mod_2_driver_seam_registry_plan_d7_conformance_suite [EXTRACTED 0.85]
- **D74 single-writer invariant enforced across backends and the probe** — _claude_plans_mod_2_quota_caps_blueprint_d74, _claude_plans_mod_2_quota_caps_blueprint_upsert_agent_box, _claude_plans_mod_2_quota_caps_blueprint_set_agent_box_quota, _claude_plans_mod_2_quota_caps_blueprint_agent_box_row, _claude_plans_mod_2_quota_caps_blueprint_store_conformance [EXTRACTED 0.85]
- **Live probe findings (F-2/F-4/F-7/F-8/F-10/F-13) rewrite the mapper and supervisor before they are written** — _claude_plans_mod_2_cli_transport_plan_f2_sigint, _claude_plans_mod_2_cli_transport_plan_f4_cumulative, _claude_plans_mod_2_cli_transport_plan_f7_coalesce_key, _claude_plans_mod_2_cli_transport_plan_f8_subtype_lies, _claude_plans_mod_2_cli_transport_plan_f10_budget_zero, _claude_plans_mod_2_cli_transport_plan_f13_quota_gap, _claude_plans_mod_2_cli_transport_blueprint_cli_claude, _claude_plans_mod_2_cli_transport_blueprint_cli_mod [EXTRACTED 0.90]
- **Which file answers which question (contract / present / future / past)** — docs_requirements, concepts, handoff, decisions, documentation_structure_analysis [EXTRACTED 0.90]
- **The install pipeline's staging-promote-rollback safety net** — claude_plans_mod_20_registry_adapter_install_plan_d16, claude_plans_mod_20_registry_adapter_install_blueprint_h3, claude_plans_mod_20_registry_adapter_install_blueprint_h4, claude_plans_mod_20_registry_adapter_install_blueprint_h5, claude_plans_mod_20_registry_adapter_install_blueprint_p10 [EXTRACTED 0.90]
- **Milestone 6 closes H-2, H-3 and H-4 as one chain** — _claude_plans_mod_2_agy_acp_plan_d58, _claude_plans_mod_2_agy_acp_plan_d59, _claude_plans_mod_2_agy_acp_plan_d61, _claude_plans_mod_2_agy_acp_blueprint_launch_for, _claude_plans_mod_2_agy_acp_blueprint_resolve_credential, _claude_plans_mod_2_agy_acp_blueprint_open_session_timeout [EXTRACTED 0.90]
- **Offline chat flow: buffered writer → .open buffer → seal → upload fills step columns** — _claude_plans_mod_2_durable_history_replay_plan_d34_writer_buffered, _claude_plans_mod_2_durable_history_replay_blueprint_buffered_writer, _claude_plans_mod_2_durable_history_replay_blueprint_h1_open_suffix, _claude_plans_mod_2_durable_history_replay_plan_d36_usage_totals_move, _claude_plans_mod_2_durable_history_replay_blueprint_usage_totals, _claude_plans_mod_2_durable_history_replay_plan_d33_offline_this_user, _claude_plans_mod_2_durable_history_replay_plan_d31_mirror_agent_only [EXTRACTED 0.90]
- **The read-only preview as the assembler's only binary-level caller (D102, D103)** — claude_plans_mod_2_prompt_assembler_blueprint_preview, claude_plans_mod_2_prompt_assembler_blueprint_prompt_tab, claude_plans_mod_2_prompt_assembler_blueprint_store_worker_preview, claude_plans_mod_2_prompt_assembler_blueprint_agent_worker_preview, claude_plans_mod_2_prompt_assembler_blueprint_assemble [EXTRACTED 0.90]
- **The passive quota latch, wire to column** — claude_plans_mod_2_quota_caps_plan_d66, claude_plans_mod_2_quota_caps_plan_d67, claude_plans_mod_2_quota_caps_plan_d68, claude_plans_mod_2_quota_caps_plan_d74, claude_plans_mod_2_quota_caps_plan_d73 [EXTRACTED 0.90]
- **The §7 passive quota latch: wire blob to stored agent_box document** — _claude_plans_mod_2_quota_caps_blueprint_mapper_usage, _claude_plans_mod_2_quota_caps_blueprint_usageevent_quota, _claude_plans_mod_2_quota_caps_blueprint_latch_quota, _claude_plans_mod_2_quota_caps_blueprint_normalize, _claude_plans_mod_2_quota_caps_blueprint_set_agent_box_quota, _claude_plans_mod_2_quota_caps_blueprint_quota_cell [EXTRACTED 0.90]
- **Replay path: rows → decoder → transcript → read-only mode, one renderer shared with live** — _claude_plans_mod_2_durable_history_replay_plan_d37_replay_decodes_rows, _claude_plans_mod_2_durable_history_replay_blueprint_replay_module, _claude_plans_mod_2_durable_history_replay_plan_d38_step_events_request, _claude_plans_mod_2_durable_history_replay_plan_d39_action_replay, _claude_plans_mod_2_durable_history_replay_plan_d40_replay_is_a_mode, _claude_plans_mod_2_live_acp_chat_blueprint_transcript [EXTRACTED 0.90]
- **Per-run cap enforcement: settings read to cancelled turn** — _claude_plans_mod_2_quota_caps_blueprint_project_settings, _claude_plans_mod_2_quota_caps_blueprint_projectcaps, _claude_plans_mod_2_quota_caps_blueprint_check_cap, _claude_plans_mod_2_quota_caps_blueprint_enforce_breach, _claude_plans_mod_2_quota_caps_blueprint_record_cap_breach, _claude_plans_mod_2_quota_caps_blueprint_run_turn, _claude_plans_mod_2_quota_caps_blueprint_run_chat [EXTRACTED 0.90]
- **The amended §7.3 upstream walk on three backends, reconciled by one sort** — claude_plans_mod_2_prompt_assembler_blueprint_pg_upstream_cte, claude_plans_mod_2_prompt_assembler_blueprint_sqlite_upstream_cte, claude_plans_mod_2_prompt_assembler_blueprint_mem_upstream_walk, claude_plans_mod_2_prompt_assembler_blueprint_upstreamentry, claude_plans_mod_2_prompt_assembler_blueprint_read_cases [EXTRACTED 0.90]
- **MOD-25 online-only supersedes the local-only chain** — handoff_mod_25, handoff_mod_17, handoff_mod_18, handoff_mod_19, decisions_ana_10, docs_ana_10 [EXTRACTED 0.95]
- **The permission_answer denial route: E-1 blocks D85, D93 adds the variant, D94 closes replay** — _claude_plans_mod_2_cli_transport_plan_d85, _claude_plans_mod_2_cli_transport_blueprint_e1, _claude_plans_mod_2_cli_transport_plan_d93, _claude_plans_mod_2_cli_transport_plan_d94, _claude_plans_mod_2_cli_transport_plan_f12b_two_sources [EXTRACTED 0.95]
- **One driver seam, three transports reporting the same conformance cases** — concepts_one_driver_trait_acp_first, handoff_mod_2, readme_claude_cli_row, claude_plans_mod_2_agy_acp_blueprint_launch_for, claude_plans_mod_1_tui_scaffold_blueprint_conformance [INFERRED 0.80]

## Communities (281 total, 41 thin omitted)

### Community 0 - "recorder.rs"
Cohesion: 0.07
Nodes (84): a_bare_first_row_latches_nothing_and_a_later_one_keeps_the_last_blob(), a_breach_is_reported_once_and_the_closing_rows_are_error_then_done(), a_breach_survives_the_refused_flush_of_the_row_that_caused_it(), a_cap_of_zero_cancels_on_the_first_costed_row(), a_credential_split_across_chunks_is_refused_at_the_flush(), a_held_edit_proposal_is_written_by_a_cancel_and_by_finish(), a_masked_usage_row_is_summed_but_not_latched(), a_masked_vocabulary_token_costs_readability_not_the_turn() (+76 more)

### Community 1 - "State"
Cohesion: 0.06
Nodes (42): a_chat_run_mints_the_two_rows_the_offline_upload_would(), a_preview_style_bound_skills_read_collapses_overrides(), agents_join_this_box_only(), MemStore, project_settings_reads_the_column_or_nothing(), require_author(), AgentId, Arc (+34 more)

### Community 2 - "Ctx"
Cohesion: 0.04
Nodes (62): Handled, Ctx, KeyEvent, KeyEvent, KeyEvent, KeyEvent, AgentsSection, AUTH_CANCELLED (+54 more)

### Community 3 - "htui-agent/tests/probe.rs"
Cohesion: 0.06
Nodes (86): a_checked_override_pointing_nowhere_is_missing(), a_cli_row_stops_at_tier_1(), a_declared_file_makes_it_ready_and_the_home_candidate_answers_when_the_var_is_unset(), a_declared_variable_counts_only_when_set_and_non_empty(), a_dropped_handshake_future_still_kills_the_child(), a_dropped_version_capture_still_kills_the_child(), a_hung_npm_root_is_killed_at_the_version_timeout(), a_leading_v_is_tolerated() (+78 more)

### Community 4 - "cli/mod.rs"
Cohesion: 0.06
Nodes (66): ADAPTER_ID, agent_version(), banner_models(), cancel_session(), classify(), ClaudeStreamAdapter, CliDriver, CliSession (+58 more)

### Community 5 - "App"
Cohesion: 0.05
Nodes (40): App, Ctx<'a>, DRAIN_ROUNDS, Emit, Box, Frame, HashMap, KeyEvent (+32 more)

### Community 6 - "Result"
Cohesion: 0.07
Nodes (32): a_memory_backend_answers_the_fixture_user(), a_memory_writer_round_trips_a_chat_run(), BufferedWriter, item_writes_need_the_server(), PROMPT_ON_SERVER_ONLY, REGISTRY_ON_SERVER_ONLY, registry_writes_need_the_server(), AgentId (+24 more)

### Community 7 - "DriverEnvelope"
Cohesion: 0.08
Nodes (66): ActiveSession, ConnectionTo, AcpSession, ADAPTER_ID, answer(), answer_from_connection(), answer_parked_cancelled(), answer_permission() (+58 more)

### Community 8 - "record.rs"
Cohesion: 0.09
Nodes (46): DriverError, String, CAP_EXCEEDED, CapBreach, CHUNK_FLUSH_BYTES, DriverError, encode(), enforce_breach() (+38 more)

### Community 9 - "cli_live.rs"
Cohesion: 0.10
Nodes (60): args_with(), assert_not_running(), base_args(), CANCEL_WINDOW, case_10_a_denial_that_does_not_depend_on_this_box(), case_1_auth_and_the_first_stdin_message(), case_2_session_id_is_ours(), case_3_cancellation_semantics() (+52 more)

### Community 10 - "ResolvedLaunch"
Cohesion: 0.07
Nodes (52): AcpAgentConfig, ChildWrapper, Compat, AgentLaunch, ChildIo, CliSettings, CREATE_NO_WINDOW, CredentialProbe (+44 more)

### Community 11 - "src/probe.rs"
Cohesion: 0.08
Nodes (60): below_min(), capture_version(), default_install_root(), exists(), expand(), expand_vars(), extract_version(), glob_first() (+52 more)

### Community 12 - "store_worker.rs"
Cohesion: 0.07
Nodes (67): Vec, a_failed_connect_event_turns_connecting_into_an_age(), a_failed_step_events_read_names_the_request(), a_served_step_log_is_never_empty_but_the_reply_can_be(), a_step_this_backend_does_not_hold_answers_none(), an_unreachable_read_drops_an_online_backend_onto_the_mirror(), apply_migrations_without_a_held_store_applies_nothing(), ChatFrame (+59 more)

### Community 13 - "agent_worker.rs"
Cohesion: 0.08
Nodes (69): a_choose_with_no_flow_pending_is_refused(), a_command_for_an_unknown_step_is_refused(), a_command_still_queued_when_the_flow_ends_is_refused_rather_than_dropped(), a_flow_holds_the_reprobe_claim_for_its_row(), a_login_whose_runtime_let_go_of_it_cancels_itself_rather_than_waiting(), a_probe_while_a_login_runs_is_refused(), a_refusal_writes_nothing_and_keeps_probed_at(), a_second_start_while_one_runs_is_refused() (+61 more)

### Community 14 - "PgStore"
Cohesion: 0.06
Nodes (43): ACTIVE_RUN_STATUSES, filter_uuids(), PgStore, project_uuids(), BoxId, BTreeMap, DocumentId, Item (+35 more)

### Community 15 - "validate-workflow-docs.sh"
Cohesion: 0.06
Nodes (64): get_blocked_refs(), get_in_flight_signals(), get_open_ids_only(), get_open_items(), get_remaining_note(), get_workspace_root(), add_finding(), build_rows() (+56 more)

### Community 16 - "render.rs"
Cohesion: 0.06
Nodes (58): a_diff_degrades_to_its_stat_and_keeps_its_range(), a_fenced_section_survives_content_that_contains_a_fence(), a_judge_candidate_names_its_index_and_bounds_its_three_parts(), a_long_assistant_tail_is_head_tail_windowed_with_the_marker(), a_section_tag_in_content_is_inert(), ASSISTANT_TAIL_LINES, attr(), box_profile() (+50 more)

### Community 17 - "Backend"
Cohesion: 0.06
Nodes (36): age(), an_offline_backend_reads_connecting_until_the_first_attempt_answers(), Backend, cache(), HOUR, MINUTE, prompt_offline(), BoxId (+28 more)

### Community 18 - "src/conformance.rs"
Cohesion: 0.13
Nodes (58): a_transport_without_a_capability_takes_the_other_arm(), cancel_answers_parked_permissions(), cancel_without_a_permission_channel(), CaseHarness, CASES, chunk(), chunk_flush_at_16kib(), coalesce_across_message_id() (+50 more)

### Community 19 - "src/fixtures.rs"
Cohesion: 0.07
Nodes (55): AGENT, agents(), ANA_1_BODY, BOX, box_tools(), boxes(), counters_sit_above_every_minted_number(), demo_at() (+47 more)

### Community 20 - "cache.rs"
Cohesion: 0.12
Nodes (57): a_fingerprint_change_rebuilds(), a_long_transaction_is_caught_by_the_overlap(), a_malformed_pending_file_is_left_alone(), a_pass_mirrors_the_agent_registry(), a_pass_mirrors_the_demo_projects(), a_pending_file_the_server_refuses_does_not_block_the_ones_after_it(), a_running_step_is_mirrored_only_once_it_has_finished(), a_schema_version_change_rebuilds() (+49 more)

### Community 21 - "quota.rs"
Cohesion: 0.09
Nodes (43): a_partial_or_foreign_blob_costs_one_field_and_never_a_panic(), a_source_that_reports_nothing_normalizes_to_spend_only(), ALLOWED, at(), Availability, available(), available_skips_a_cap_already_reached(), available_skips_an_exhausted_document_first() (+35 more)

### Community 22 - "refresh.rs"
Cohesion: 0.04
Nodes (47): AGENT_COLUMNS, APP_USER_COLUMNS, BOX_COLUMNS, DEFAULT_INTERVAL_SECONDS, DEFAULT_OVERLAP_SECONDS, DEFAULT_TRANSCRIPT_STEPS, DOCUMENT, DOCUMENT_COLUMNS (+39 more)

### Community 23 - ".new"
Cohesion: 0.16
Nodes (52): a_done_frame_reads_the_registry_again_rather_than_claiming_success(), a_failed_install_with_manual_steps_renders_them_under_the_table(), a_failed_or_idle_flow_leaves_a_notice_and_an_idle_section(), a_failed_read_says_the_registry_needs_postgres(), a_failure_without_manual_steps_is_a_notice(), a_methods_frame_does_not_undo_a_cancel_pressed_while_starting(), a_methods_frame_renders_the_chooser_with_names_descriptions_logout_and_the_hidden_count(), a_on_a_cli_row_is_refused_by_name_and_sends_nothing() (+44 more)

### Community 24 - "Theme"
Cohesion: 0.06
Nodes (28): .ID, .ID, DetailId, DetailRegistry, DetailTab, message(), PAGE, render() (+20 more)

### Community 25 - "Result"
Cohesion: 0.15
Nodes (33): bad(), bool_col(), CacheStore, document_of(), get(), item_of(), item_summary_of(), json_col() (+25 more)

### Community 26 - "scrubber"
Cohesion: 0.09
Nodes (44): scrubber(), a_credential_mid_sentence_is_caught_and_the_root_pointer_is_empty(), a_credential_shaped_object_key_is_refused(), a_known_secret_that_looks_like_a_credential_is_masked_not_refused(), a_masked_secret_never_appears_in_the_error_path(), a_payload_with_no_strings_is_ok(), a_pem_footer_without_a_header_is_refused(), a_pem_header_without_a_footer_is_refused() (+36 more)

### Community 27 - "Event"
Cohesion: 0.07
Nodes (35): clip(), GAP, groups(), INDENT, lines(), ListView, pad(), render() (+27 more)

### Community 28 - "update.rs"
Cohesion: 0.09
Nodes (33): Action, OverlayAction, StepId, String, TabAction, a_failure_overtaken_by_a_newer_request_never_reaches_the_status_line(), a_reply_addressed_to_another_view_never_reaches_a_tab(), a_reply_overtaken_by_a_newer_request_of_the_same_kind_is_dropped() (+25 more)

### Community 29 - "cli_driver.rs"
Cohesion: 0.09
Nodes (47): argv(), a_budget_that_is_absent_or_zero_or_negative_passes_no_flag(), a_cancel_drains_a_late_result_and_reports_its_done_rather_than_a_second_one(), a_cancel_with_no_grace_synthesizes_exactly_one_cancelled_done(), a_resuming_session_names_the_old_id_and_mints_nothing(), a_session_over_a_scripted_agent_streams_a_turn_and_then_a_follow_up(), a_stream_with_no_init_times_out_and_kills_its_child(), an_empty_permission_mode_passes_no_flag() (+39 more)

### Community 30 - "UsageSpy<'_, S>"
Cohesion: 0.10
Nodes (25): Played, QuotaCall, rate_limit_blob(), AgentId, BoxId, DateTime, DocumentId, Item (+17 more)

### Community 31 - ".serve"
Cohesion: 0.10
Nodes (22): ParseEnumError, String, ProjectCaps, String, StoreError, an_unmirrored_project_is_unbounded_offline_and_refused_online(), AuthCommand, declares_a_source() (+14 more)

### Community 32 - "ReadStore"
Cohesion: 0.09
Nodes (24): chat_step_status(), not_a_terminal_status(), ReadStore, AgentId, BoxId, DateTime, DocumentId, Item (+16 more)

### Community 33 - ".new"
Cohesion: 0.11
Nodes (38): a_batch_cap_is_read_and_not_enforced(), a_buffered_writer_gets_no_latch(), a_buffered_writer_never_re_probes(), a_chat_latches_the_quota_of_the_row_it_runs_on(), a_chat_over_its_run_cap_is_cancelled_and_its_run_fails(), a_chat_start_on_a_stale_acp_row_re_probes_in_the_background(), a_cli_row_and_a_fresh_row_are_not_re_probed(), a_negative_run_cap_refuses_the_chat_start() (+30 more)

### Community 34 - "DriverCaps"
Cohesion: 0.07
Nodes (22): AcpDriver, AuthSource, AgentRow, Debug, Formatter, Path, Result, Self (+14 more)

### Community 35 - "htui-agent/tests/auth.rs"
Cohesion: 0.09
Nodes (41): a_spawn_failure_is_a_spawn_error_naming_the_command(), a_stale_recording_falls_back_to_resolution(), a_stderr_line_resets_the_idle_clock(), a_url_is_reported_once_per_flow(), agent_row(), AGENT_SH, an_idle_flow_is_killed_after_the_cap_and_reported_idle(), Answer (+33 more)

### Community 36 - "SettingsSection"
Cohesion: 0.09
Nodes (21): .ID, message(), render_strip(), Box, Debug, Display, Formatter, Frame (+13 more)

### Community 37 - "Overlay"
Cohesion: 0.08
Nodes (21): .ID, Overlay, OverlayId, .ANY, OverlayRegistry, OverlayStack, Box, Debug (+13 more)

### Community 38 - "driver.rs"
Cohesion: 0.09
Nodes (29): permission_event(), AgentSessionRef, McpServerSpec, PermissionAnswer, PermissionMatch, PermissionRequestId, PermissionRule, REDACTED (+21 more)

### Community 39 - "FakeSession"
Cohesion: 0.09
Nodes (22): DriverFactory, FAKE_AGENT_NAME, FAKE_AGENT_VERSION, FakeAdapter, FakeDriver, FakeSession, Arc, Box (+14 more)

### Community 40 - "tests/launch.rs"
Cohesion: 0.08
Nodes (34): a_non_utf8_byte_neither_ends_the_stream_nor_hides_the_line_after_it(), a_second_tap_replaces_the_first(), a_signal_reaches_the_whole_process_group(), a_tap_installed_late_replays_the_tail_first_then_streams(), a_tap_receives_every_stderr_line_in_order(), AGY_LAUNCH, alive(), an_interrupt_lets_the_child_exit_on_its_own_terms() (+26 more)

### Community 41 - "runs.rs"
Cohesion: 0.11
Nodes (28): a_new_item_takes_the_cursor_with_it(), a_pane_with_no_step_passes_enter_on(), a_runs_reply_puts_the_cursor_on_the_first_step(), CURSOR, enter_emits_a_replay_of_the_step_under_the_cursor(), key(), pane(), PENDING (+20 more)

### Community 42 - "install/mod.rs"
Cohesion: 0.08
Nodes (34): ArchiveFormat, Option, Self, CONNECT_TIMEOUT, DISK_HEADROOM_FACTOR, InstallOutcome, InstallPlan, ManualSteps (+26 more)

### Community 43 - "Layout"
Cohesion: 0.13
Nodes (22): age_of(), ARCHIVE, io(), Layout, PREVIOUS, PROMOTE_ATTEMPTS, PROMOTE_BACKOFF, remove_any() (+14 more)

### Community 44 - "template.rs"
Cohesion: 0.09
Nodes (25): DEFAULT_TEMPLATES, ROLES, a_handoff_body_needs_two(), a_judge_body_without_candidates_is_missing_required(), duplicates_substitute_twice_and_used_lists_once(), every_placeholder_round_trips_its_token(), HANDOFF_REQUIRED, is_token_shaped() (+17 more)

### Community 45 - "htui/tests/install.rs"
Cohesion: 0.10
Nodes (32): ENTRY_ID, ENTRY_VERSION, Fixture, i_shows_the_consent_pane_without_fetching_the_archive(), install_row(), n_closes_the_pane_and_still_nothing_was_fetched(), ON_BOX_AT, on_box_cell() (+24 more)

### Community 46 - "claude.rs"
Cohesion: 0.13
Nodes (29): ABORTED, body_of(), chunk_key(), error_message(), kind_of(), locations_of(), Mapper, MICROS_PER_UNIT (+21 more)

### Community 47 - "store/conformance.rs"
Cohesion: 0.13
Nodes (37): append_events_idempotent_and_ordered(), CASES, chat_event(), documents_ordered_by_version(), events_ordered_by_seq(), filter_status_project_tags_ready(), links_hops_1_vs_2(), mint_consecutive_keys() (+29 more)

### Community 48 - "htui/tests/auth.rs"
Cohesion: 0.10
Nodes (31): a_then_enter_into_no_credential_still_reads_unauthenticated(), a_then_enter_logs_in_and_the_cell_reads_the_probes_verdict(), AGENT_SH, CREDENTIAL, executable(), init_result(), LINK, login_row() (+23 more)

### Community 49 - "Harness"
Cohesion: 0.09
Nodes (20): an_empty_store_renders_the_shell_with_the_switcher_over_it(), an_overlay_survives_startup_sees_keys_first_and_closes_on_esc(), Harness, App, Box, ChatTask, Debug, Formatter (+12 more)

### Community 50 - "run_pass"
Cohesion: 0.26
Nodes (37): ts_bind(), Batch, cursor(), finish(), json_text(), opt_json_text(), PassReport, refresh_box() (+29 more)

### Community 51 - "AgentRuntime"
Cohesion: 0.12
Nodes (29): OpenerCommand, InstallConfig, Default, AgentRuntime, AuthArgs, ChatArgs, ChatCommand, InstallArgs (+21 more)

### Community 52 - "cache/mod.rs"
Cohesion: 0.11
Nodes (30): BUSY_TIMEOUT, CACHE_FILE, CacheMeta, CacheStore, connect(), create_dir(), FULL_REFRESH_MAX_AGE, KEY_BUILT_AT (+22 more)

### Community 53 - "tests/settings.rs"
Cohesion: 0.09
Nodes (35): an_agent_no_fixture_contains_appears_from_its_row_alone(), an_empty_registry_says_so_rather_than_drawing_a_bare_table(), as_drawn(), Bench, cell_at(), default_at(), default_cell(), DEFAULT_WIDTH (+27 more)

### Community 54 - "Staged"
Cohesion: 0.15
Nodes (33): Installer, InstallJob, InstallProgress, BoxId, Debug, emit(), failed_without_rollback(), install() (+25 more)

### Community 55 - "Fixture"
Cohesion: 0.13
Nodes (26): a_registry_document_larger_than_the_client_will_hold_is_refused(), a_registry_failure_with_a_warm_cache_plans_and_states_the_age(), amp_row(), an_entry_whose_cmd_climbs_out_of_the_tree_is_refused_before_consent(), archive_route(), demo_row(), Fixture, one_entry_registry() (+18 more)

### Community 56 - "Handshake"
Cohesion: 0.09
Nodes (24): Handshake, handshake_error(), AcpIo, DateTime, DriverError, Duration, Error, InitializeResponse (+16 more)

### Community 57 - "cli_map.rs"
Cohesion: 0.12
Nodes (30): a_budget_breach_is_an_error_row_and_a_done(), a_plain_turn_ends_with_usage_then_done(), a_policy_denial_is_a_permission_answer_by_policy(), a_rate_limit_event_is_an_other_row_and_rides_the_turns_usage(), a_rate_limit_event_that_is_not_an_object_is_not_carried(), a_recorded_cancelled_turn_maps_to_the_rows_it_did_when_it_was_captured(), a_recorded_thinking_turn_maps_to_the_rows_it_did_when_it_was_captured(), a_recorded_tool_call_and_its_result_map_to_a_joined_pair() (+22 more)

### Community 58 - "http.rs"
Cohesion: 0.13
Nodes (24): Bytes, ByteStream, content_length_header(), header_text(), HeadInfo, HttpClient, HttpError, install_crypto_provider() (+16 more)

### Community 59 - "SessionEvent"
Cohesion: 0.11
Nodes (30): driver_rows(), kinds(), EventKind, From, Self, chunk(), decode(), envelope_from_row() (+22 more)

### Community 60 - "WorkspaceSummary"
Cohesion: 0.12
Nodes (28): workspace_projects(), Project, ProjectRef, Repo, RepoBoxPath, BoxId, DateTime, Option (+20 more)

### Community 61 - "RunSummary"
Cohesion: 0.15
Nodes (29): ChatRunSpec, prompt_summary(), AgentId, BoxId, DateTime, GateOutcome, ItemId, Option (+21 more)

### Community 62 - "keymap.rs"
Cohesion: 0.15
Nodes (20): a_shifted_character_keeps_the_character_and_drops_the_flag(), an_unknown_chord_and_a_foreign_scope_resolve_to_none(), Binding, chord(), every_overlay_inherits_esc_and_can_shadow_it(), key_code(), KeyChord, Keymap (+12 more)

### Community 63 - "agy_live.rs"
Cohesion: 0.12
Nodes (32): agy_row(), an_unauthenticated_box_is_told_what_the_agent_said(), assert_no_survivors(), children_by_ppid(), children_of_this_process(), credential_probe(), expand_by_hand(), expected_platform_args() (+24 more)

### Community 64 - "src/connect.rs"
Cohesion: 0.12
Nodes (29): attempt(), CACHE_OVERLAP_SECONDS, CACHE_REFRESH_SECONDS, ConnEvent, EVENT_QUEUE, NO_DSN, NO_DSN_FINGERPRINT, no_dsn_is_a_failed_event_and_not_an_error() (+21 more)

### Community 65 - "chat.rs"
Cohesion: 0.17
Nodes (32): a_chat_on_an_unprobed_row_refreshes_agent_box_in_the_background(), a_degraded_transport_banners_what_it_cannot_do(), a_follow_up_opens_a_second_turn(), a_fresh_row_is_not_re_probed_and_a_stale_one_is(), a_harness_without_a_runtime_renders_the_refusal(), a_parked_permission_is_answered_with_a_digit_and_recorded_as_the_users(), a_recorded_step_replays_through_the_live_transcript(), a_replayed_turn_renders_the_lines_the_live_one_did() (+24 more)

### Community 66 - "install/registry.rs"
Cohesion: 0.15
Nodes (25): age_of(), BinaryEntry, CACHE_BODY, CACHE_META, CacheMeta, Distribution, read_registry(), registry_url() (+17 more)

### Community 67 - "wire_flow"
Cohesion: 0.22
Nodes (32): a_json_rpc_error_is_refused_in_the_agents_own_words(), a_prepared_transport_runs_the_flow_without_a_process(), a_terminal_typed_method_is_hidden_and_named_but_never_sent(), an_adapter_that_dies_during_the_chooser_is_left_to_the_callers_clock(), an_agent_that_dies_after_initialize_is_a_transport_error_with_its_stderr(), an_unknown_method_type_is_offered_as_an_agent_method(), assert_reaped(), cancel_before_the_choice_ends_the_connection_and_answers_cancelled() (+24 more)

### Community 68 - "prompt/settings.rs"
Cohesion: 0.14
Nodes (23): a_non_positive_rung_falls_through(), app(), as_rows_is_key_byte_order(), Budget, BudgetSource, chain_phase_project_app_default(), DEFAULTS, HOPS_RANGE (+15 more)

### Community 69 - "append_pending"
Cohesion: 0.14
Nodes (31): append_pending(), Buffer, EXTENSION, list(), next_sealed_name(), OPEN_SUFFIX, parse(), BoxId (+23 more)

### Community 70 - "pg_criteria.rs"
Cohesion: 0.09
Nodes (20): a_rolled_back_mint_leaves_no_gap(), agent_row(), concurrent_mints_produce_consecutive_numbers(), counter(), race_item(), race_kind(), RACE_PREFIX, revision_count() (+12 more)

### Community 71 - "validate-workflow-docs.ps1"
Cohesion: 0.14
Nodes (27): Get-DecisionFileIds(), Get-IdSpace(), Get-NextOf(), Invoke-SelfTest(), New-Finding(), New-Fixture(), Update-Max(), Get-ArchiveEntries() (+19 more)

### Community 72 - "cli_conformance.rs"
Cohesion: 0.16
Nodes (28): Block, CLI_SESSION_ID, delta(), DENIAL_MESSAGE, DENIAL_REASON, deny(), DUPLEX_BYTES, flush_run() (+20 more)

### Community 73 - "DriverEvent"
Cohesion: 0.16
Nodes (29): DoneEvent, DriverEvent, EditProposalEvent, ErrorEvent, OtherEvent, PermissionAnswerEvent, PermissionOption, PermissionRequestEvent (+21 more)

### Community 74 - "htui-agent/tests/replay.rs"
Cohesion: 0.16
Nodes (29): envelopes(), Vec, a_recorded_acp_turn_decodes_to_what_it_rendered(), adjacent_text_rows_decode_one_row_at_a_time(), an_unknown_kind_never_costs_the_transcript(), at(), driver_script(), env() (+21 more)

### Community 75 - "LinkGraph"
Cohesion: 0.11
Nodes (24): links(), a_non_ascii_key_sorts_by_bytes(), classification_separates_scope_from_summary(), entry(), ItemLink, keys(), LinkEdge, LinkGraph (+16 more)

### Community 76 - ".run"
Cohesion: 0.14
Nodes (26): AtomicBool, Send, Sync, Tier2, a_clean_download_the_probe_cannot_handshake_ends_failed_with_the_tree_left(), a_previous_version_comes_back_and_the_row_describes_what_is_left(), a_ready_install_deletes_the_previous_version_after_the_row_was_produced(), a_rollback_whose_removal_is_refused_retries_before_it_gives_up() (+18 more)

### Community 77 - "Vec"
Cohesion: 0.18
Nodes (26): nonce(), a_cancelled_download_removes_its_own_partial_file(), a_kill_between_unpack_and_promote_leaves_nothing_the_glob_resolves(), a_promote_the_rows_glob_cannot_see_is_refused_by_name_and_rolled_back(), a_published_digest_that_does_not_match_stops_before_anything_is_unpacked(), a_token_tripped_mid_download_ends_cancelled_with_staging_swept(), a_token_tripped_mid_unpack_ends_cancelled_with_staging_swept(), a_zip_unpacks_the_whole_tree_and_counts_what_it_wrote() (+18 more)

### Community 78 - "src/permission.rs"
Cohesion: 0.15
Nodes (27): a_kind_the_agent_does_not_offer_falls_back_to_its_sibling(), a_matched_rule_the_agent_cannot_honour_asks_the_user(), a_remembered_entry_answers_only_after_every_rule_missed(), an_empty_matcher_matches_every_request_including_one_with_no_call(), call(), evaluate(), every_field_of_a_matcher_must_hold(), matches() (+19 more)

### Community 79 - "driver_contract.rs"
Cohesion: 0.10
Nodes (21): a_driver_that_says_nothing_about_authenticate_refuses_it(), caps_for_an_acp_row_advertises_authenticate_and_a_cli_row_does_not(), check_wire_enum(), driver_events_reach_every_event_kind_htui_does_not_author(), every_adapter(), every_driver_event(), every_registered_adapter_agrees_with_its_predicate(), flow() (+13 more)

### Community 80 - "Document"
Cohesion: 0.09
Nodes (24): Document, DocumentHead, DateTime, DocumentId, ItemId, Option, StepId, String (+16 more)

### Community 81 - "WorkspaceSwitcher"
Cohesion: 0.10
Nodes (15): CHROME, CURSOR, GAP, HINT, NO_CURSOR, projects_label(), Frame, KeyEvent (+7 more)

### Community 82 - "client.rs"
Cohesion: 0.12
Nodes (25): AcpClientCapabilities, client_capabilities(), client_info(), forward_permission(), forward_read(), forward_write(), Inbound, option_kind() (+17 more)

### Community 83 - "AgentSettings"
Cohesion: 0.13
Nodes (21): AcpAdapter, AgentSettings, UsageScope, UsageSettings, adapter_id(), adapter_id_from(), caps_from(), DriverFactory (+13 more)

### Community 84 - "chat/mod.rs"
Cohesion: 0.20
Nodes (20): a_live_frame_keeps_applying_under_an_open_replay(), a_step_not_on_this_box_does_not_read_as_a_step_that_said_nothing(), an_unanswered_permission_replays_parked_and_stays_unanswered(), BUFFERED_LABEL, BUFFERED_NOTE, entering_replay_disarms_the_cancel(), esc_leaves_replay_and_the_live_chat_is_exactly_as_it_was(), every_key_the_live_tab_acts_on_sends_nothing_in_replay() (+12 more)

### Community 85 - "PgStore"
Cohesion: 0.13
Nodes (15): kind_not_in_project(), PgStore, AgentId, BoxId, DateTime, Display, Item, ItemId (+7 more)

### Community 86 - "htui-store/src/testkit.rs"
Cohesion: 0.12
Nodes (24): bare_db(), count(), demo_db(), ENV_URL, fresh_db(), json(), parse_dsn(), CacheStore (+16 more)

### Community 87 - "run_turn"
Cohesion: 0.15
Nodes (21): PermissionPolicy, close_run(), drain(), ends(), forward(), Frames, prompt_sections(), record() (+13 more)

### Community 88 - "InstallError"
Cohesion: 0.26
Nodes (26): apply_mode(), create_link(), descend(), io(), Last, link_stays_inside(), make_executable(), MAX_LINK_BYTES (+18 more)

### Community 89 - "map.rs"
Cohesion: 0.12
Nodes (21): a_diff_content_block_becomes_an_edit_proposal_before_the_result(), a_new_file_diff_block_has_no_old_text(), a_non_text_chunk_lands_in_other_rather_than_becoming_empty_text(), a_non_usd_cost_is_reported_as_it_arrived_and_sums_nothing(), a_plan_is_the_complete_entry_list_and_drops_what_it_cannot_read(), a_thought_chunk_maps_the_same_way_and_may_have_no_message_id(), a_tool_call_carries_its_id_kind_input_and_locations(), an_assistant_chunk_carries_its_text_and_grouping_key() (+13 more)

### Community 90 - "htui/tests/replay.rs"
Cohesion: 0.12
Nodes (19): a_second_replay_supersedes_the_first(), a_shell_that_registered_no_replay_tab_says_so(), a_step_that_recorded_nothing_arrives_as_a_missing_log(), enter_on_a_pane_that_cannot_replay_says_where_to_press_it(), enter_on_a_project_header_still_folds(), enter_on_a_step_addresses_its_rows_to_the_replay_tab(), on_the_runs_pane(), registered() (+11 more)

### Community 91 - "Transcript"
Cohesion: 0.14
Nodes (14): Cell, CallStatus, Resolution, Line, Option, Scroll, StopReason, String (+6 more)

### Community 92 - "BoundSkill"
Cohesion: 0.16
Nodes (21): at(), binding(), bound(), BoundSkill, equal_positions_break_on_name_bytes(), phase_overrides_project_once(), DateTime, Option (+13 more)

### Community 93 - "transcript.rs"
Cohesion: 0.20
Nodes (19): a_permission_request_is_parked_until_its_answer_arrives(), a_plan_update_replaces_the_plan_rather_than_appending_one(), a_policy_denial_with_no_request_is_still_a_row(), a_second_proposal_for_one_file_updates_the_row_it_already_has(), a_tool_result_folds_into_its_call_rather_than_becoming_a_row(), chunk(), chunks_of_one_message_coalesce_and_a_new_key_starts_a_row(), diff_style() (+11 more)

### Community 94 - "writer_buffered.rs"
Cohesion: 0.18
Nodes (24): a_batch_spanning_two_runs_lands_in_two_files(), a_clone_shares_the_registration(), an_event_for_an_unregistered_step_is_not_found_and_writes_nothing(), an_offline_backend_hands_out_a_buffered_writer(), cache(), chat(), event(), finish_chat_run_seals_the_buffer() (+16 more)

### Community 95 - "acp_conformance.rs"
Cohesion: 0.13
Nodes (19): AsyncWriteExt, AcpHarness, claude_row(), DUPLEX_BYTES, model_option(), permission_request(), Arc, AtomicU64 (+11 more)

### Community 96 - "acp/auth.rs"
Cohesion: 0.14
Nodes (20): answered(), methods_of(), AcpIo, CancellationToken, Duration, Error, InitializeResponse, Option (+12 more)

### Community 97 - "AuthEvent"
Cohesion: 0.14
Nodes (20): AUTH_IDLE_CAP, AuthCall, AuthChoice, AuthEvent, AuthFlow, AuthMethodInfo, AuthOutcome, BrowserPolicy (+12 more)

### Community 98 - "plan"
Cohesion: 0.12
Nodes (16): available_space(), cmd_relative(), human_age(), human_bytes(), InstallPlan, ManualSteps, plan(), DateTime (+8 more)

### Community 99 - "caps_for"
Cohesion: 0.20
Nodes (24): caps_for(), a_manual_snapshot_is_not_used_as_a_launch(), a_probe_column_that_does_not_parse_resolves_as_before(), a_recorded_command_that_is_gone_falls_back_to_resolution(), a_recorded_command_that_is_now_a_directory_falls_back_to_resolution(), a_snapshot_probed_for_another_transport_is_not_used_as_a_launch(), a_usable_snapshot_is_what_the_driver_would_spawn_marker_included(), entry_point() (+16 more)

### Community 100 - "auth_live.rs"
Cohesion: 0.14
Nodes (23): children_by_ppid(), children_of_this_process(), chosen_method(), credential_report(), DEFAULT_METHOD, Entry, INSTALL_HINT, listing() (+15 more)

### Community 101 - "BoxProfile"
Cohesion: 0.20
Nodes (22): a_bare_version_renders_the_name_alone(), at(), BoxInfo, BoxProfile, .MAX_TOOLS, BoxRow, BoxTool, path_never_reaches_the_profile() (+14 more)

### Community 102 - "secret.rs"
Cohesion: 0.17
Nodes (18): a_blank_entry_reads_as_no_dsn(), a_dsn_round_trips_and_a_missing_entry_is_none(), backend(), clear_dsn(), get_dsn(), install_mock(), mock_slot(), Option (+10 more)

### Community 103 - "migrations.rs"
Cohesion: 0.08
Nodes (3): ANA5_INTEGER_DEFAULTS, ANA_COLUMN_COMMENTS, TABLES

### Community 104 - "T70: criteria sweep and MOD-2 close-out"
Cohesion: 0.11
Nodes (22): D9: ANA-5 skill types defer to milestone 9, MOD-2 Live ACP Chat Blueprint, D-2: the nine channels and their capacities, C-2: the permission request round trip, D-3: shutdown orders (cancel, transport close, quit), MOD-2 Live ACP Chat Plan (milestone 3), MOD-2 Probe Autodiscovery Blueprint, E-1: Settings r → ProbeAgents → task → one Agents reply (+14 more)

### Community 105 - "tools.rs"
Cohesion: 0.18
Nodes (21): a_glob_probe_that_matches_nothing_is_unresolved(), a_node_package_prefers_the_local_tree_over_the_global_root(), a_path_probe_resolves_through_which(), a_row_without_discovery_resolves_to_an_empty_map(), a_tool_on_no_path_is_unresolved_under_its_own_name(), discovery(), env_override(), env_override_key() (+13 more)

### Community 106 - "htui-agent/tests/install.rs"
Cohesion: 0.13
Nodes (20): a_chained_symlink_is_never_written_through_in_either_shape(), a_dot_prefixed_archive_unpacks_in_either_shape(), a_file_entry_that_names_the_tree_itself_is_refused(), a_pax_global_header_is_skipped_and_the_rest_of_the_tarball_unpacks(), a_symlink_that_leaves_the_tree_is_refused_in_either_shape(), a_zip_symlink_whose_target_is_enormous_is_refused_as_an_archive(), an_in_tree_relative_symlink_is_written_in_either_shape(), CMD (+12 more)

### Community 107 - "ItemSummary"
Cohesion: 0.25
Nodes (18): Item, ItemFilter, ItemPatch, ItemRevision, ItemSummary, NewItem, BoxId, DateTime (+10 more)

### Community 108 - "MigrationPrompt"
Cohesion: 0.12
Nodes (13): centered(), Chrome, Rect, CHROME, HINT, MigrationPrompt, Frame, Line (+5 more)

### Community 109 - "ChatTab"
Cohesion: 0.17
Nodes (13): ChatSessionState, ChatTab, .ID, replay_body(), ReplayState, Default, Frame, Line (+5 more)

### Community 110 - "StepGraphPhase"
Cohesion: 0.17
Nodes (21): CommandQueue, catalogue(), ItemKind, PhaseAgent, PromptTemplate, AgentId, DateTime, ItemKindId (+13 more)

### Community 111 - "fs.rs"
Cohesion: 0.16
Nodes (16): a_missing_file_reads_as_the_empty_document_and_a_write_creates_its_parents(), a_new_file_diffs_as_a_run_of_plus_lines_that_reproduces_the_content(), a_refused_path_writes_nothing(), an_edit_diff_carries_both_sides(), guard(), normalise(), PathOutside, read_current() (+8 more)

### Community 112 - "AgentBox"
Cohesion: 0.19
Nodes (22): advertises_auth(), agy_logs_in_through_the_app_and_the_probe_reads_ready(), agy_logs_out_when_asked(), agy_row(), assert_no_survivors(), box_in_state(), credential_probe(), credential_tier_of() (+14 more)

### Community 113 - "extensibility.rs"
Cohesion: 0.14
Nodes (19): an_unknown_agent_reaches_a_driver_from_its_row_alone(), production_half(), Box, DriverFactory, PathBuf, Script, Vec, the_auth_flow_names_no_vendor_or_method() (+11 more)

### Community 114 - "Agent"
Cohesion: 0.16
Nodes (21): every_seed_row_deserialises_into_the_launch_types(), the_production_factory_holds_one_adapter_per_transport_not_per_agent(), Agent, AgentSeed, Billing, DateTime, Option, String (+13 more)

### Community 115 - "DemoData"
Cohesion: 0.11
Nodes (20): counters(), DemoData, DOCUMENT, done_step(), ITEM, ITEM_SPECS, ItemSpec, PROJECT_SPECS (+12 more)

### Community 116 - "usage.rs"
Cohesion: 0.15
Nodes (14): a_key_without_an_integer_adds_nothing(), add_delta(), at(), deltas_saturate(), row(), DateTime, Option, Self (+6 more)

### Community 117 - "Fixture"
Cohesion: 0.21
Nodes (18): a_plan_for_a_row_without_install_is_refused_by_name(), a_probe_and_an_install_never_write_the_same_row_at_once(), a_second_plan_while_one_runs_is_refused(), cancel_answers_cancelling_then_the_stream_ends_cancelled(), confirm_streams_frames_at_the_confirm_seq(), demo_plan(), Fixture, install_row() (+10 more)

### Community 118 - "shell.rs"
Cohesion: 0.15
Nodes (20): register_all(), App, an_empty_store_starts_with_the_switcher_open_over_no_workspaces(), a_pending_migration_opens_the_prompt(), an_empty_store_shows_no_workspaces(), answering_n_does_not_reopen_the_prompt(), answering_y_asks_the_store_to_apply_them(), enter_switches_the_scope_and_the_top_bar_follows() (+12 more)

### Community 119 - "PgStore"
Cohesion: 0.20
Nodes (9): Identity, BoxId, Connected, PgStore, BoxId, PgPool, Result, Self (+1 more)

### Community 120 - "chat_offline.rs"
Cohesion: 0.21
Nodes (21): a_buffered_chat_lands_in_postgres_on_the_next_connection(), an_offline_box_that_never_synced_this_user_refuses_and_says_so(), an_offline_chat_is_accepted_and_its_header_says_it_is_buffered(), an_offline_chat_writes_its_rows_to_the_pending_buffer_in_seq_order(), an_online_chat_header_says_nothing_about_a_buffer(), compose(), offline_harness(), offline_mirror() (+13 more)

### Community 121 - "MOD-2 — agent driver + chat tab"
Cohesion: 0.13
Nodes (21): CLEAN-1 plan — cargo doc for htui-agent, Discovery.credential — declarative credential probe (D59), compose.yaml — development Postgres 16 on port 5439, Capability matching refuses mismatched runs, One driver trait, ACP first, MOD-16 — Windows runtime verification of the agent driver, MOD-2 — agent driver + chat tab, MOD-22 — complete a loopback OAuth login from an unreachable box (+13 more)

### Community 122 - "probe_agent"
Cohesion: 0.16
Nodes (18): agent_box_row(), probe_agent(), ProbeContext, BoxId, DateTime, Utc, Value, children_by_ppid() (+10 more)

### Community 123 - "acp_driver.rs"
Cohesion: 0.16
Nodes (20): a_failed_handshake_still_reaps_its_child(), a_refused_session_new_answers_with_the_agents_own_message(), assert_not_running(), assert_reaped(), DUPLEX_BYTES, KILL_WINDOW, MARKER, open_session_times_out_and_kills_its_child() (+12 more)

### Community 124 - "boxed"
Cohesion: 0.27
Nodes (19): a_head_that_stalls_leaves_the_size_unknown_and_still_plans(), a_plan_answers_every_coordinate_for_this_platform(), a_platform_the_entry_lacks_is_not_available_here(), a_row_that_declares_no_source_is_refused_by_name(), a_second_entry_plans_with_a_published_digest_and_a_tarball(), a_second_plan_within_max_age_issues_no_registry_request(), agy_row(), an_unsupported_archive_is_refused_before_any_head() (+11 more)

### Community 125 - "prompt/fixtures.rs"
Cohesion: 0.21
Nodes (20): demo_box(), demo_budget(), demo_caps(), diff_block(), every_fixture_body_parses_in_its_own_role(), handoff_basic(), judge_three_candidates(), phase_all_empty() (+12 more)

### Community 126 - "Path"
Cohesion: 0.19
Nodes (20): call_for(), calls(), executable(), fixture_argv(), fixture_calls(), fixture_launch(), fixture_launch_marked(), open_url_spawns_with_null_stdio_and_does_not_wait() (+12 more)

### Community 127 - "download"
Cohesion: 0.19
Nodes (18): abandon(), download(), Downloaded, hex(), io(), network(), CancellationToken, Error (+10 more)

### Community 128 - "defaults.rs"
Cohesion: 0.12
Nodes (15): body_of(), COMMAND_QUEUE_TEXT, FIX, HANDOFF, IMPLEMENT, JUDGE, PLAN, PRD (+7 more)

### Community 129 - "composer.rs"
Cohesion: 0.18
Nodes (12): backspace_edits_and_an_empty_enter_sends_nothing(), Composer, ComposerOutcome, esc_leaves_and_forgets(), key(), Frame, KeyCode, KeyEvent (+4 more)

### Community 130 - "chat_live_agy.rs"
Cohesion: 0.16
Nodes (18): a_real_agy_session_streams_three_turns_into_the_store_and_answers_11_14(), choose(), CONVERSATION_TIMEOUT, FIXTURE_OUT_ENV, matching_processes(), Observed, PROBE_FILE, report() (+10 more)

### Community 131 - "chat_usage_pg.rs"
Cohesion: 0.19
Nodes (18): an_acp_step_usage_equals_the_last_cost_total_on_postgres(), at(), env(), FIXTURE, mapped_turn(), max_cost_micros_total(), probe_snapshot(), DateTime (+10 more)

### Community 132 - "HANDOFF.md — outstanding work"
Cohesion: 0.14
Nodes (18): MOD-1 TUI scaffold plan, htui MCP server for agent-proposed writes, Offline is read-only from a per-box cache, Step graphs, not scripts, docs/ANA-10.md — local-only mode (done, not taken), HANDOFF.md — outstanding work, ANA-11 — models for requirements and decisions, ANA-14 — whether Redis could be beneficial (+10 more)

### Community 133 - "model/mod.rs"
Cohesion: 0.17
Nodes (11): box_and_agent_enums_match_check_lists(), check_enum(), check_id(), event_enums_match_check_lists(), id_newtypes_round_trip(), link_kind_matches_check_list(), phase_enums_match_check_lists(), T (+3 more)

### Community 134 - "htui/src/testkit.rs"
Cohesion: 0.14
Nodes (12): buffer_text(), CHAT_END, DEFAULT_SIZE, Probe, .ID, Duration, Frame, Rect (+4 more)

### Community 135 - "BodyTab"
Cohesion: 0.14
Nodes (10): BodyTab, HEAD, Frame, Item, ItemId, KeyEvent, Option, Rect (+2 more)

### Community 136 - "DocumentsTab"
Cohesion: 0.14
Nodes (11): BY_HAND, BY_STEP, DocumentsTab, Frame, ItemId, KeyEvent, Option, Rect (+3 more)

### Community 137 - "NotesTab"
Cohesion: 0.17
Nodes (10): NotesTab, Frame, ItemId, KeyEvent, Line, Option, Rect, Scroll (+2 more)

### Community 138 - "identity.rs"
Cohesion: 0.22
Nodes (16): a_hostname_change_keeps_the_box_id(), BOX_FILE, BoxToml, cache_dir(), config_root(), db_fingerprint(), hostname(), load_or_mint() (+8 more)

### Community 139 - "D6: recorder flush, seq/turn, digest, scrub order"
Cohesion: 0.17
Nodes (17): D1: new crate crates/htui-agent, D2: AgentDriver / AgentSession boxed-future seam, D3: every store-seam change lands in milestone 1, D4: start_chat_run / finish_chat_run and ChatRunSpec::mint, D5: Scrubber seam and MinimalScrubber in htui-core, D6: recorder flush, seq/turn, digest, scrub order, D7: transport-neutral conformance Script, CaseHarness, CASES, crates/htui-agent (+9 more)

### Community 140 - "Value"
Cohesion: 0.29
Nodes (15): diffs_of(), int_at(), locations_of(), Mapper, output_of(), result_of(), Option, ToolResultStatus (+7 more)

### Community 141 - "auth/run.rs"
Cohesion: 0.16
Nodes (15): authenticate(), closed_tap(), DRAIN_GRACE, forward(), Duration, HashSet, Result, String (+7 more)

### Community 142 - "InstallRecord"
Cohesion: 0.21
Nodes (13): Consent, InstallRecord, io(), Manifest, BTreeMap, DateTime, Error, Option (+5 more)

### Community 143 - "install_live.rs"
Cohesion: 0.18
Nodes (16): a_third_agent_installs_from_a_registry_row_alone(), amp_row(), children_by_ppid(), children_of_this_process(), LICENSE, LIVE_TIMEOUT, mode_of(), print_tree() (+8 more)

### Community 144 - "PromptSpec"
Cohesion: 0.29
Nodes (15): DiffBlock, HandoffInputs, InputDocument, JudgeCandidate, JudgeInputs, PromptSpec, Option, StepSummary (+7 more)

### Community 145 - "htui agent_worker.rs: AgentRuntime, run_chat, run_turn"
Cohesion: 0.15
Nodes (16): D8: append_pending primitive in cache/pending.rs, T5: append_pending, A2: a seal onto a taken name renames to a numbered stem, BufferedWriter: the offline WriteStore impl, H-1: the .open suffix for a live buffer, seal_orphaned at CacheStore::open, D34: Writer::Buffered, the offline sink, D35: the buffered writer refuses what the buffer cannot hold (+8 more)

### Community 146 - "assemble() — the pure prompt assembler"
Cohesion: 0.13
Nodes (16): assemble() — the pure prompt assembler, D100: scrubber wraps and unwraps a &str as a Value, D101: compiled-in DEFAULTS for app_setting, D102: preview assembly is deferred work, not a sub-tab call, D103: preview stand-ins for three PromptSpec fields, D106: two prompt projection fields on RunStepSummary across three backends, D108: the estimator ships as chars-v2, D98: one excerpt rendering for both families (+8 more)

### Community 147 - "docs/ANA-9.md — data model v2"
Cohesion: 0.18
Nodes (16): MOD-1 TUI scaffold blueprint, Trait-level conformance suite (store::conformance), Deterministic demo fixture (demo_uuid, demo_at), MemStore over std::sync::RwLock<State>, Tab / DetailTab / Overlay registries and the Keymap, Request seq, discriminant-keyed staleness and addressed replies, Store worker owns the Backend (D4), testkit::Harness with inline settle() (+8 more)

### Community 148 - "CONCEPTS.md — standing architectural intent"
Cohesion: 0.17
Nodes (16): CONCEPTS.md — standing architectural intent, No agent in any bookkeeping path, Postgres is the single source of truth, Prompt economy — inlined skills, bounded context, Quota-aware agent routing, Repos stay clean — htui writes no files into managed repos, Secrets: provider-injected, transcripts scrubbed fail-closed, DECISIONS.md — archive index (+8 more)

### Community 149 - ".start"
Cohesion: 0.37
Nodes (10): Box, DriverError, Option, Result, Send, String, StubSession, Future (+2 more)

### Community 150 - "Scope"
Cohesion: 0.14
Nodes (6): Vec, Scope, demo_scope(), ProbeSection, KeyEvent, the_section_asks_for_the_registry_once_and_unscoped()

### Community 151 - "estimate.rs"
Cohesion: 0.15
Nodes (5): Option, Self, TokenEstimator, .DEFAULT, .WIDE

### Community 152 - "excerpt.rs"
Cohesion: 0.25
Nodes (11): Excerpt, ExcerptAudit, ExcerptCaps, ExcerptReason, ExcerptSet, FileRecord, RootRecord, RootSource (+3 more)

### Community 153 - "TerminalGuard"
Cohesion: 0.20
Nodes (11): App, Duration, Result, UnboundedReceiver, run(), TICK, init(), install_panic_hook() (+3 more)

### Community 154 - "GraphTab"
Cohesion: 0.16
Nodes (10): GraphTab, .ID, IN, OUT, Frame, ItemId, Option, Rect (+2 more)

### Community 155 - "htui-store/src/error.rs"
Cohesion: 0.21
Nodes (11): checksum_drift(), is_unreachable(), map_migrate(), map_sqlx(), map_sqlx_for(), Display, Error, String (+3 more)

### Community 156 - "SkillsTab"
Cohesion: 0.19
Nodes (7): Frame, KeyEvent, Rect, Self, Vec, SkillsTab, .ID

### Community 157 - "chat_live_cli.rs"
Cohesion: 0.21
Nodes (14): a_real_claude_cli_session_streams_two_turns_into_the_store_and_then_ends(), CAPS_BANNER, cli_row(), compose(), CONVERSATION_TIMEOUT, key_sum(), matching_processes(), report() (+6 more)

### Community 158 - "htui-agent replay.rs: envelope_from_row, envelope_or_other"
Cohesion: 0.19
Nodes (14): The tool_call_id column-fill rule, The seq:N message_id stamp on decoded text rows, htui-agent replay.rs: envelope_from_row, envelope_or_other, ChatTab ReplayState and the read-only key branch, D37: replay decodes rows, it does not re-render them, D38: StoreRequest::StepEvents and its Option reply, D39: Action::Replay, focus-then-dispatch under the Chat tab's origin, D40: replay is a mode, and the mode is what makes it read-only (+6 more)

### Community 159 - "install-workflow-hooks.sh"
Cohesion: 0.33
Nodes (12): convert_to_lf_lines(), get_hook_targets(), install_workflow_hooks(), new_post_commit_block(), new_pre_commit_block(), resolve_target_entry(), run_self_test(), set_marker_block() (+4 more)

### Community 160 - "browser.rs"
Cohesion: 0.21
Nodes (12): BROWSER_VAR, BrowserPolicy, neutraliser(), open_url(), OPEN_URL_VAR, opener_command(), platform_command(), Option (+4 more)

### Community 161 - "backlog.rs"
Cohesion: 0.32
Nodes (13): a_scope_change_clears_the_list_and_the_detail(), backlog(), down(), enter_folds_and_unfolds_a_project_group(), every_sub_tab_says_so_when_it_has_nothing(), h_and_l_cycle_the_sub_tabs_both_ways(), j_k_g_and_shift_g_move_the_selection(), sub_tab() (+5 more)

### Community 162 - "T59: template.rs + estimate.rs — scanner and estimator"
Cohesion: 0.23
Nodes (13): F-24: two agents in one crate are coupled by the crate, not by their file sets, F-28: a per-crate gate is not a gate, F-33: cargo doc --workspace is red at HEAD (CLEAN-1), T59: template.rs + estimate.rs — scanner and estimator, T60: ten default bodies and the fixture corpus, T61: model types — skill, UpstreamEntry, BoxProfile, PromptScope, PathPrefix, T62: the store seam and MemStore, T63: PgStore, CacheStore and the amended §7.3 query (+5 more)

### Community 163 - "quota::normalize"
Cohesion: 0.19
Nodes (13): ACP conformance _meta encoder, D66: quota normalization in htui_core::model::quota, D73: quota column in the Settings agents table, Mapper::usage, quota::normalize, Quota struct, settings::agents::quota_cell, htui_core::model::quota (+5 more)

### Community 164 - "acp_map.rs"
Cohesion: 0.31
Nodes (11): a_recorded_agy_turn_maps_to_the_rows_it_did_when_it_was_captured(), a_recorded_turn_maps_to_the_rows_it_did_when_it_was_captured(), lines(), mapped(), Value, Vec, the_agy_capture_answers_the_three_open_11_14_questions(), the_rate_limit_blob_is_captured_verbatim_and_never_summed() (+3 more)

### Community 165 - ".new"
Cohesion: 0.23
Nodes (9): cli_row(), CliHarness, Box, DuplexStream, Script, Self, scripted_cli(), the_cli_transport_passes_every_case() (+1 more)

### Community 166 - "SectionName"
Cohesion: 0.21
Nodes (8): Display, Error, Formatter, Result, S, SectionName, Ok, Serialize

### Community 167 - "run_read_case"
Cohesion: 0.20
Nodes (12): document_body_round_trip(), documents_of_kinds_latest_per_kind_in_order(), platform_prompt_scope(), project_row_has_settings(), F, run_all(), run_all_reads(), run_read_case() (+4 more)

### Community 168 - "prompt_render.rs"
Cohesion: 0.21
Nodes (6): crlf_inputs_render_identical_bytes(), no_rendered_byte_carries_a_path_a_clock_or_a_carriage_return(), phase_sections(), String, Vec, the_phase_sections_render_their_golden_bytes()

### Community 169 - "tests/connect.rs"
Cohesion: 0.33
Nodes (11): a_terminated_backend_is_an_unreachable_error(), an_online_backend_falls_back_onto_its_own_mirror(), an_unreachable_server_is_a_failed_event_and_the_shell_still_starts(), close(), EVENT_TIMEOUT, next_event(), offline_never_dials_and_starts_at_an_age(), Duration (+3 more)

### Community 170 - "acp/mod.rs: AcpDriver, AcpSession, run_session"
Cohesion: 0.24
Nodes (11): D10: MSRV 1.98 and the new dependency set, D11: launch.rs types, ${tool} resolution, job-object spawn, D12: data-keyed DriverFactory and adapter ids, T7: MSRV bump, dependency set, launch types, spawn, T9: DriverFactory and the R-AGT-5 zeta proof, X9: ANA-4's MSRV 1.88 vs the 1.98.1 pin, acp/client.rs: capabilities, permission decode, Inbound forwarders, acp/fs.rs: path guard, unified diff, write (+3 more)

### Community 171 - "Recorder::latch_quota"
Cohesion: 0.24
Nodes (11): D96: READ_CASES beside CASES, not a relaxed bound, READ_CASES store conformance list, probe::agent_box_row, D67: set_agent_box_quota writes two columns only, D74: single-writer quota columns, Recorder::latch_quota, quota_blob_latches_agent_box, WriteStore::set_agent_box_quota (+3 more)

### Community 172 - "PromptSpec"
Cohesion: 0.24
Nodes (11): BoundSkill::collapse, BoxProfile::project, Budget / target(), The nine ordering/determinism rules (§4.7), Hazard table H-1…H-30, Inherent prompt reads (prompt_templates, bound_skills, box_profile, app_settings, repos, repo_paths, item_kind), model::skill (Skill, SkillVersion, SkillBinding, BoundSkill), tests/prompt_digest.rs (+3 more)

### Community 173 - "htui-core prompt module surface (assemble)"
Cohesion: 0.20
Nodes (11): BufferedWriter::set_step_prompt (offline refusal, E-8), BuiltinRanker (builtin@1), prompt::digest (canonical, sha256_hex), prompt::excerpt (core, pure), ExcerptProvider trait, PathPrefix, htui-core prompt module surface (assemble), Recorder::record_prompt (unchanged, B.14) (+3 more)

### Community 174 - "install-workflow-hooks.ps1"
Cohesion: 0.44
Nodes (10): ConvertTo-LfLines(), ConvertTo-ShSingleQuoted(), Find-MarkerIndex(), Get-HookTargets(), Install-WorkflowHooks(), Invoke-SelfTest(), New-PostCommitBlock(), New-PreCommitBlock() (+2 more)

### Community 175 - "next-item.ps1"
Cohesion: 0.36
Nodes (10): Get-BlockedRefs(), Get-InFlightSignals(), Get-OpenItems(), Get-RemainingNote(), Get-WorkspaceRoot(), Invoke-NextItem(), Invoke-SelfTest(), Build-Candidates() (+2 more)

### Community 176 - ".new"
Cohesion: 0.31
Nodes (10): a_client_builds_after_the_provider_is_installed(), a_manifest_that_does_not_parse_reads_as_an_empty_one(), a_promote_onto_an_occupied_target_is_refused_by_name(), a_same_version_reinstall_sets_the_old_directory_aside_before_promoting(), existing_versions_lists_directories_and_ignores_the_manifest(), rooted(), the_manifest_is_written_through_a_temporary_and_a_rename(), the_sweep_removes_residue_older_than_an_hour_and_nothing_younger() (+2 more)

### Community 177 - "chat/permission.rs"
Cohesion: 0.22
Nodes (5): PermissionStrip, row(), Frame, Option, Rect

### Community 178 - "Plan: MOD-2 agy over ACP (milestone 6)"
Cohesion: 0.27
Nodes (10): chat_live_agy.rs (T34), CredentialProbe, Plan: MOD-2 agy over ACP (milestone 6), D57: htui does not download the adapter, D59: declarative credential check, D62: §11.14 answers as fixtures, mapper amended only if the wire demands, D63: subscription OAuth, not the API key, D64: models filled from the live session/new answer or left empty (+2 more)

### Community 179 - "D79: a second registry row, claude-cli"
Cohesion: 0.22
Nodes (10): ChatArgs.reprobe, ReprobeArgs, run_reprobe, D60: re-probe on spawn failure, no in-row cli fallback, E-0: --bare never reads OAuth, pre-init buffer, agent_claude_cli.json seed, D79: a second registry row, claude-cli (+2 more)

### Community 180 - "cli/mod.rs supervisor"
Cohesion: 0.22
Nodes (10): H-4: unauthenticated snapshots are usable launches, launch_for, D58: the driver prefers agent_box.probe.resolved, CliSession::answer_permission returns Unsupported, Cancel drains before it synthesizes the done, cli/mod.rs supervisor, stdin_line, D81: cancel is stdin-close then SIGINT then grace then kill (+2 more)

### Community 181 - "ANA-4: agent driver design authority"
Cohesion: 0.29
Nodes (10): Plan: MOD-2 driver seam, registry and extensibility proof (M1-M2), Blueprint: MOD-2 milestone 4 — durable history and replay, Plan: MOD-2 durable history and replay (milestone 4), Blueprint: MOD-2 milestone 3 — live claude over ACP and the chat tab, Plan: MOD-2 live ACP chat (milestone 3), Availability / SkipReason, quota::available, D72: available() implements R-AGT-8's four skip rules (+2 more)

### Community 182 - "Recorder::record"
Cohesion: 0.29
Nodes (10): htui-agent conformance CASES, CapBreach, Recorder::check_cap, record::enforce_breach, record::pump, Recorder::record, Recorder::record_cap_breach, Recorder::record_unreadable (+2 more)

### Community 183 - "T4: the registry reader and the pre-flight"
Cohesion: 0.20
Nodes (10): production_half truncates in-module test blocks for the vendor sweep, P-9: a downgrade is refused at pre-flight as Outranked, D12: Discovery.install as the declared source, InstallPlan as consent evidence, D13: exactly one GET and one HEAD before consent, D14: version-aware newest() keyed on the last glob capture, D15: HTUI_AGENTS_ROOT owns the install root, T1: the declared source, T2: the install root, the token, the seeds (+2 more)

### Community 184 - "AgentSummary"
Cohesion: 0.31
Nodes (10): AgentSummary, login_row(), probed_row(), quota_row(), render_section_at(), Option, ProbeStatus, Value (+2 more)

### Community 185 - ".build"
Cohesion: 0.18
Nodes (9): probed(), AgentId, Box, DateTime, DriverError, Option, Result, Utc (+1 more)

### Community 186 - "T15: mirror the agent registry and the offline user"
Cohesion: 0.25
Nodes (9): D13: agent seed rows and fixture correction, D14: SettingsRegistry and the agent section, crates/htui, T10: Settings tab agent section, T8: seed rows, fixture correction, PgStore seed, D31: the mirror gains agent, and only agent, D32: agent is unscoped, a full replace with no cursor, D33: CacheStore::this_user over the mirrored app_user (+1 more)

### Community 187 - "D18: three install requests, one streamed reply"
Cohesion: 0.22
Nodes (9): H-10: the single-install claim covers planning and installing, H-8: abort() does not stop spawn_blocking, D18: three install requests, one streamed reply, D19: in-section consent pane with a row cursor, not an overlay, D20: the no-network path renders ManualSteps, T7: requests, replies, runtime, T8: the Settings install action, D73: an eighth quota column in Settings, Chat tab untouched (+1 more)

### Community 188 - "htui::preview (build_spec, run_preview, PromptPreview)"
Cohesion: 0.28
Nodes (9): AgentRuntime::preview, FsRepoReader (agent half), htui::preview (build_spec, run_preview, PromptPreview), PromptTab (backlog detail sub-tab), PromptScope (D95), RepoReader trait, excerpt::select (§4.5 steps 3–9), excerpt::skip_by_path (+1 more)

### Community 189 - "assemble() — the §4.7 pipeline"
Cohesion: 0.31
Nodes (9): assemble() — the §4.7 pipeline, AssembledPrompt, AssembleError, ExcerptAudit, run::prompt_summary (D106), Runs pane prompt indicators (D106), Per-section scrub (D100), SectionEntry (+1 more)

### Community 190 - "TokenEstimator (chars-v2)"
Cohesion: 0.25
Nodes (9): Build order T59–T70, tests/estimator_live.rs (T69 probe), Out of scope (MOD-4, MOD-9, MOD-15, MOD-10, MOD-11, ANA-3), TokenEstimator (chars-v2), F-16 — §4.4's Claude constants are low by 28.4%, F-17 — the GPT/Gemini row is not measurable here, F-18 — the conservative-default argument holds at 2.44, MOD-2 milestone 9 plan (prompt assembler + preview) (+1 more)

### Community 191 - "prompt::render — the section renderers"
Cohesion: 0.28
Nodes (9): TokenEstimator::estimate, render::fence_for, trim::head_tail, prompt::render — the section renderers, SectionName, StepSummary::from_events, prompt::trim — kept-first trim, trim::trim_order (+1 more)

### Community 192 - "T39: the §7 blob and the passive latch"
Cohesion: 0.25
Nodes (9): H-7: allowed_warning makes MOD-4 skip a warned-but-allowed agent, D66: capture the vendor rate-limit blob verbatim, normalize outside the mapper, D68: the latch is best-effort and never fails a chat, D72: R-AGT-8 ships as a pure predicate, quota::available, D78: nothing_to_say reads billing as well as source, T37: _meta rate-limit capture, T39: the §7 blob and the passive latch, T42: R-AGT-8's predicate (+1 more)

### Community 193 - "wire_value"
Cohesion: 0.22
Nodes (6): plan(), PlanEntryPriority, PlanEntryStatus, T, wire_value(), WireText

### Community 194 - "AgentSession"
Cohesion: 0.33
Nodes (4): AgentSession, DriverFuture, Duration, Option

### Community 195 - "Seen"
Cohesion: 0.31
Nodes (9): completed(), drive(), env_flag(), DriverError, Instant, Result, Sender, Seen (+1 more)

### Community 196 - "AppUser"
Cohesion: 0.28
Nodes (8): users(), AppUser, CapabilityTag, DateTime, Option, String, UserId, Utc

### Community 197 - "htui/src/lib.rs"
Cohesion: 0.36
Nodes (8): init_tracing(), Duration, Option, Path, Result, run(), set_dsn_from_stdin(), SHUTDOWN

### Community 198 - "pg/mod.rs"
Cohesion: 0.25
Nodes (7): CONNECT_TIMEOUT, MigrationState, Duration, OsFamily, SEEDED_SETTINGS, SEEDED_TAGS, this_os_family()

### Community 199 - "Blueprint: MOD-2 milestone 8 CLI transport"
Cohesion: 0.39
Nodes (8): Blueprint: MOD-2 milestone 8 CLI transport, E-2: modelUsage token cumulativeness unknown, E-3: normalize discards the CLI blob, E-5: protocol_version == 1 is pinned, Plan: MOD-2 degraded CLI transport (milestone 8), D86: usage sums modelUsage[*], not result.usage, F-13: normalize discards the CLI blob (measured), F-4: modelUsage is cumulative, result.usage is per-turn

### Community 200 - "D91: DriverCaps gains usage_mid_turn"
Cohesion: 0.32
Nodes (8): E-1: permission_answer is htui-authored only, E-4: six cases need a second arm, not three, D80: capability-gated conformance case, not a skipped one, D85: permission_answer rows synthesized with by policy, D91: DriverCaps gains usage_mid_turn, D93: permission_answer gets a twelfth DriverEvent variant, D94: replay decodes permission_answer to the typed variant, F-12b: permission denial has two sources

### Community 201 - "Pipeline"
Cohesion: 0.29
Nodes (5): Fetched, Pipeline, BoxId, InstallPlan, TempDir

### Community 202 - "digest.rs"
Cohesion: 0.36
Nodes (5): canonical(), canonical_strips_a_bom_and_ends_with_one_lf(), String, sha256_hex(), sha256_matches_the_recorder_call()

### Community 203 - "integration.rs"
Cohesion: 0.39
Nodes (7): demo_shell(), String, the_backlog_tab_shows_the_scope_and_the_digits_reach_the_other_tabs(), the_demo_shell_starts_inside_the_first_workspace_on_the_backlog_tab(), the_switcher_reached_by_w_still_changes_the_scope(), top_bar(), w_opens_the_workspace_switcher_and_esc_closes_it_again()

### Community 204 - "Recorder"
Cohesion: 0.33
Nodes (7): D97: set_step_usage keeps its third parameter, D68: the latch never fails the turn, quota_latch_for, QuotaLatch, Recorder, agent_worker::run_chat, RunCap

### Community 205 - "MOD-20 Registry-Driven Adapter Install Plan"
Cohesion: 0.29
Nodes (7): MOD-20 Registry-Driven Adapter Install Blueprint, H-1: the rustls provider must be installed explicitly, MOD-20 Registry-Driven Adapter Install Plan, D22: documentation replacement and amendment records, D8: no new crate — the installer is htui_agent::install, D9: reqwest 0.13 with rustls-no-provider, explicit provider install, T10: close-out

### Community 206 - "D16: staging, promotion, rollback, retention"
Cohesion: 0.29
Nodes (7): H-11: HTUI_TOOL_<NAME> precedence survives an install, H-3: staging must stay outside the glob's reach, H-4: the post-promote resolve check, P-10: set-aside entries carry a .previous suffix, D16: staging, promotion, rollback, retention, T6: the pipeline — re-probe, rollback, retention, cancellation, T9: amp-acp live, the R-AGT-5 proof

### Community 207 - "UpstreamEntry + sort_canonical"
Cohesion: 0.48
Nodes (7): The fixture upstream diamond (C.4), MemStore upstream walk (C.3), Postgres upstream CTE (C.1), READ_CASES (D96), ReadStore additions (document, documents_of_kinds, upstream_summaries, project), SQLite mirror upstream CTE (C.2), UpstreamEntry + sort_canonical

### Community 208 - "T40: run-cap enforcement"
Cohesion: 0.29
Nodes (7): P-1: pump is not the production loop, run_turn is, D69: detection in the recorder, cancellation in the loop holding the session, D70: caps read from project.settings via inherent project_settings, no migration, D75: edit_proposal dedup index becomes step-scoped, D77: reserve the seq at announcement, defer only the write, T40: run-cap enforcement, T46: criterion 6's dedup survives a flush

### Community 209 - ".new"
Cohesion: 0.29
Nodes (6): an_absurd_cost_pair_clamps_instead_of_overflowing(), object(), IntoIterator, Item, Self, usage_costs_are_deltas_of_the_cumulative_total()

### Community 210 - "htui-agent/src/lib.rs"
Cohesion: 0.33
Nodes (3): Args, Option, PathBuf

### Community 211 - "settings_over"
Cohesion: 0.57
Nodes (6): a_harness_without_a_runtime_clears_the_probing_state_on_the_refusal(), a_second_r_while_probing_is_refused_on_the_status_line(), r_in_the_agents_section_probes_and_the_column_reads_the_status(), Option, settings_over(), unresolvable_registry()

### Community 212 - "probe_agent ordered steps"
Cohesion: 0.47
Nodes (6): agy_live.rs (T33), CredentialTier, probe_agent ordered steps, ProbeSnapshot, resolve_credential, status_for

### Community 213 - "cli/claude.rs mapper"
Cohesion: 0.33
Nodes (6): cli/claude.rs mapper, H-8: partial messages deliver text twice, D82: thinking blocks probed then mapped, F-6: thinking is signalled but not disclosed, F-7: coalescing key is (message.id, block index), Fixture redaction holes: split deltas and slugged paths

### Community 214 - "ProjectCaps"
Cohesion: 0.40
Nodes (6): CapError, D70: per-run cap enforced by the recorder, in USD micros, D71: the batch cap is read and reported, not enforced here, project_settings(ProjectId), ProjectCaps, agent_worker::start

### Community 215 - "D17: the consent record is manifest.json on the box's disk"
Cohesion: 0.40
Nodes (6): D17: the consent record is manifest.json on the box's disk, H-6: the latch firing on a row re-probed mid-session, D67: the narrow WriteStore::set_agent_box_quota setter, D74: quota columns become single-writer, T38: the narrow quota setter, T45: single-writer quota columns

### Community 216 - "MOD-2 milestone 6 blueprint — agy over ACP"
Cohesion: 0.53
Nodes (6): MOD-2 milestone 6 blueprint — agy over ACP, ChildGuard behind a Mutex, swapped out before the await (D61), AcpDriver::launch_for — spawn the probe's recorded launch, ProbeSnapshot::recorded_launch — D58's three row-side rules, Re-probe on a failed spawn, inline in run_chat (D60), SessionOptions.handshake_timeout injected per session

### Community 217 - "prompt::defaults (DEFAULT_TEMPLATES, COMMAND_QUEUE_TEXT)"
Cohesion: 0.33
Nodes (6): prompt::defaults (DEFAULT_TEMPLATES, COMMAND_QUEUE_TEXT), template::parse, ParsedTemplate, Placeholder (the twenty tokens), prompt::template — the {{name}} scanner, TemplateRole (Phase, Judge, Handoff)

### Community 218 - "sync-workflow-surface.sh"
Cohesion: 0.53
Nodes (4): get_synced_rel_paths(), resolve_target_entry(), sync-workflow-surface.sh script, usage()

### Community 219 - "QuotaLatch"
Cohesion: 0.40
Nodes (5): QuotaLatch, AgentId, Billing, BoxId, QuotaSource

### Community 220 - "wait_until"
Cohesion: 0.33
Nodes (6): F, Fn, Option, T, wait_until(), done()

### Community 221 - "T5: fetch, verify, unpack, promote, manifest"
Cohesion: 0.40
Nodes (5): H-2: Response::content_length() lies on a HEAD, H-5: the executable bit versus launch::spawn's which, D10: zip deflate-only, tar, flate2; download then unpack, D11: free space via fs4, refuse below 4× content-length, T5: fetch, verify, unpack, promote, manifest

### Community 222 - "MOD-2 Quota and Caps Plan"
Cohesion: 0.40
Nodes (5): MOD-2 Quota and Caps Blueprint, Snapshot files are part of a task's file set, MOD-2 Quota and Caps Plan, D65: T34 runs first, its answers are inputs, T34: a live agy chat through the production runtime

### Community 223 - "acp_live.rs"
Cohesion: 0.50
Nodes (4): Client, claude_row(), AgentRow, the_seeded_claude_row_reaches_a_v1_handshake()

### Community 224 - "snapshot"
Cohesion: 0.40
Nodes (5): recorded(), recorded_launch_applies_d58s_three_row_side_rules(), ProbeSource, ProbeStatus, snapshot()

### Community 225 - "event"
Cohesion: 0.40
Nodes (5): at(), event(), DateTime, Utc, Value

### Community 227 - "ChildGuard"
Cohesion: 0.50
Nodes (4): ChildGuard, H-1: tokio orphan queue reaps the signalled child, open_session handshake timeout, D61: session task's child moves into a ChildGuard

### Community 228 - "argv"
Cohesion: 0.83
Nodes (4): argv, D83: --max-budget-usd carries the per-run cap as a second cap, D90: SessionSpec gains budget_micros, F-10: --max-budget-usd 0 kills the turn

### Community 231 - "htui"
Cohesion: 0.83
Nodes (4): htui, htui-agent, htui-core, htui-store

### Community 232 - "D76: the default column shows the whole model id"
Cohesion: 1.00
Nodes (3): D76: the default column shows the whole model id, T47: the default column shows the whole model id, T48: the packing correction — name donates

### Community 233 - "fixture_document_body"
Cohesion: 0.67
Nodes (3): fixture_document_body(), DocumentId, String

### Community 234 - "row_ids"
Cohesion: 0.67
Nodes (3): row_ids(), ItemId, Vec

## Ambiguous Edges - Review These
- `sqlx offline query data committed under crates/htui-store/.sqlx` → `CLEAN-1 plan — cargo doc for htui-agent`  [AMBIGUOUS]
  .claude/plans/clean-1.plan.md · relation: conceptually_related_to
- `MOD-4 — orchestrator, manual mode` → `Trait-level conformance suite (store::conformance)`  [AMBIGUOUS]
  .claude/plans/mod-1-tui-scaffold.blueprint.md · relation: references
- `PathPrefix` → `D98: one excerpt rendering for both families`  [AMBIGUOUS]
  .claude/plans/mod-2-prompt-assembler.plan.md · relation: conceptually_related_to
- `D101: compiled-in DEFAULTS for app_setting` → `project_settings(ProjectId)`  [AMBIGUOUS]
  .claude/plans/mod-2-prompt-assembler.plan.md · relation: conceptually_related_to

## Knowledge Gaps
- **475 isolated node(s):** `SECRET`, `CREATE_NO_WINDOW`, `STDERR_TAIL_LINES`, `STDOUT_LIMIT`, `DEFAULT_METHOD` (+470 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 1353 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **41 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **What is the exact relationship between `sqlx offline query data committed under crates/htui-store/.sqlx` and `CLEAN-1 plan — cargo doc for htui-agent`?**
  _Edge tagged AMBIGUOUS (relation: conceptually_related_to) - confidence is low._
- **What is the exact relationship between `MOD-4 — orchestrator, manual mode` and `Trait-level conformance suite (store::conformance)`?**
  _Edge tagged AMBIGUOUS (relation: references) - confidence is low._
- **What is the exact relationship between `PathPrefix` and `D98: one excerpt rendering for both families`?**
  _Edge tagged AMBIGUOUS (relation: conceptually_related_to) - confidence is low._
- **What is the exact relationship between `D101: compiled-in DEFAULTS for app_setting` and `project_settings(ProjectId)`?**
  _Edge tagged AMBIGUOUS (relation: conceptually_related_to) - confidence is low._
- **Why does `Agent` connect `Agent` to `recorder.rs`, `State`, `htui-agent/tests/probe.rs`, `Result`, `DriverEnvelope`, `src/probe.rs`, `agent_worker.rs`, `install_live.rs`, `chat_live_cli.rs`, `UsageSpy<'_, S>`, `.serve`, `ReadStore`, `.new`, `DriverCaps`, `htui-agent/tests/auth.rs`, `.new`, `FakeSession`, `htui/tests/install.rs`, `store/conformance.rs`, `htui/tests/auth.rs`, `AgentRuntime`, `Staged`, `Fixture`, `AgentSummary`, `.build`, `chat.rs`, `pg_criteria.rs`, `cli_conformance.rs`, `Pipeline`, `driver_contract.rs`, `AgentSettings`, `PgStore`, `acp_conformance.rs`, `plan`, `caps_for`, `AgentBox`, `extensibility.rs`, `Fixture`, `chat_offline.rs`, `probe_agent`, `boxed`?**
  _High betweenness centrality (0.110) - this node is a cross-community bridge._
- **Why does `State` connect `State` to `AppUser`, `BoxProfile`, `ItemSummary`, `LinkGraph`, `update.rs`, `StepGraphPhase`, `BoundSkill`, `AgentBox`, `Document`, `Agent`, `shell.rs`, `SessionEvent`, `WorkspaceSummary`, `RunSummary`?**
  _High betweenness centrality (0.080) - this node is a cross-community bridge._
- **Why does `Recorder::latch_quota` connect `Recorder::latch_quota` to `quota::normalize`, `Recorder`, `Recorder::record`?**
  _High betweenness centrality (0.072) - this node is a cross-community bridge._