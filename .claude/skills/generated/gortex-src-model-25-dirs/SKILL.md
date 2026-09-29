---
name: gortex-src-model-25-dirs
description: "Work in the src/model +25 dirs area — 1316 symbols across 104 files (67% cohesion)"
---

# src/model +25 dirs

1316 symbols | 104 files | 67% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/acp/auth.rs`
- `crates/htui-agent/src/acp/client.rs`
- `crates/htui-agent/src/acp/fs.rs`
- `crates/htui-agent/src/acp/handshake.rs`
- `crates/htui-agent/src/acp/mod.rs`
- `crates/htui-agent/src/auth/browser.rs`
- `crates/htui-agent/src/auth/mod.rs`
- `crates/htui-agent/src/cli/claude.rs`
- `crates/htui-agent/src/cli/mod.rs`
- `crates/htui-agent/src/conformance.rs`
- `crates/htui-agent/src/driver.rs`
- `crates/htui-agent/src/event.rs`
- `crates/htui-agent/src/excerpt.rs`
- `crates/htui-agent/src/install/fetch.rs`
- `crates/htui-agent/src/install/http.rs`
- `crates/htui-agent/src/install/layout.rs`
- `crates/htui-agent/src/install/manifest.rs`
- `crates/htui-agent/src/install/mod.rs`
- `crates/htui-agent/src/install/registry.rs`
- `crates/htui-agent/src/launch.rs`
- `crates/htui-agent/src/permission.rs`
- `crates/htui-agent/src/probe.rs`
- `crates/htui-agent/src/record.rs`
- `crates/htui-agent/src/replay.rs`
- `crates/htui-agent/tests/auth.rs`
- `crates/htui-agent/tests/auth_live.rs`
- `crates/htui-agent/tests/cli_conformance.rs`
- `crates/htui-agent/tests/cli_live.rs`
- `crates/htui-agent/tests/driver_contract.rs`
- `crates/htui-agent/tests/estimator_live.rs`
- `crates/htui-agent/tests/extensibility.rs`
- `crates/htui-agent/tests/install.rs`
- `crates/htui-agent/tests/launch.rs`
- `crates/htui-agent/tests/recorder.rs`
- `crates/htui-core/src/fixtures.rs`
- `crates/htui-core/src/model/agent.rs`
- `crates/htui-core/src/model/box_.rs`
- `crates/htui-core/src/model/hierarchy.rs`
- `crates/htui-core/src/model/item.rs`
- `crates/htui-core/src/model/kind.rs`
- `crates/htui-core/src/model/link.rs`
- `crates/htui-core/src/model/mod.rs`
- `crates/htui-core/src/model/note.rs`
- `crates/htui-core/src/model/overlap.rs`
- `crates/htui-core/src/model/quota.rs`
- `crates/htui-core/src/model/run.rs`
- `crates/htui-core/src/model/skill.rs`
- `crates/htui-core/src/model/usage.rs`
- `crates/htui-core/src/model/user.rs`
- `crates/htui-core/src/prompt/excerpt.rs`
- `crates/htui-core/src/prompt/mod.rs`
- `crates/htui-core/src/prompt/settings.rs`
- `crates/htui-core/src/prompt/template.rs`
- `crates/htui-core/src/prompt/trim.rs`
- `crates/htui-core/src/seed.rs`
- `crates/htui-core/src/store/mem.rs`
- `crates/htui-core/src/store/traits.rs`
- `crates/htui-core/tests/prompt_digest.rs`
- `crates/htui-orch/src/engine.rs`
- `crates/htui-orch/src/fake.rs`
- `crates/htui-orch/src/fanout.rs`
- `crates/htui-orch/src/graph.rs`
- `crates/htui-orch/src/isolate.rs`
- `crates/htui-orch/src/isolate/git.rs`
- `crates/htui-orch/src/recover.rs`
- `crates/htui-orch/src/select.rs`
- `crates/htui-store/src/cache/mod.rs`
- `crates/htui-store/src/identity.rs`
- `crates/htui-store/src/pg/mod.rs`
- `crates/htui-store/src/pg/read.rs`
- `crates/htui-store/src/pg/rows.rs`
- `crates/htui-store/src/pg/write.rs`
- `crates/htui-store/src/qdrant_settings.rs`
- `crates/htui-store/src/secret.rs`
- `crates/htui-store/src/writer.rs`
- `crates/htui/src/agent_worker.rs`
- `crates/htui/src/app/action.rs`
- `crates/htui/src/app/state.rs`
- `crates/htui/src/cli.rs`
- `crates/htui/src/connection.rs`
- `crates/htui/src/hierarchy.rs`
- `crates/htui/src/preview.rs`
- `crates/htui/src/prompt_settings.rs`
- `crates/htui/src/qdrant_settings_info.rs`
- `crates/htui/src/testkit.rs`
- `crates/htui/src/ui/layout.rs`
- `crates/htui/src/ui/overlay/registry.rs`
- `crates/htui/src/ui/tabs/backlog/detail/mod.rs`
- `crates/htui/src/ui/tabs/backlog/list.rs`
- `crates/htui/src/ui/tabs/chat/mod.rs`
- `crates/htui/src/ui/tabs/chat/permission.rs`
- `crates/htui/src/ui/tabs/chat/transcript.rs`
- `crates/htui/src/ui/tabs/registry.rs`
- `crates/htui/src/ui/tabs/settings/connection.rs`
- `crates/htui/src/ui/tabs/settings/hierarchy.rs`
- `crates/htui/src/ui/tabs/settings/kinds.rs`
- `crates/htui/src/ui/tabs/settings/mod.rs`
- `crates/htui/src/ui/tabs/settings/prompt.rs`
- `crates/htui/src/ui/tabs/settings/qdrant.rs`
- `crates/htui/src/ui/tabs/skills.rs`
- `crates/htui/tests/chat_live_agy.rs`
- `crates/htui/tests/install.rs`
- `crates/htui/tests/replay.rs`
- `crates/htui/tests/settings.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/acp/auth.rs` | methods_of, init |
| `crates/htui-agent/src/acp/client.rs` | request, id, permission_event |
| `crates/htui-agent/src/acp/fs.rs` | PathOutside, path |
| `crates/htui-agent/src/acp/handshake.rs` | at, protocol_version, agent_version, auth_methods, Handshake, ... |
| `crates/htui-agent/src/acp/mod.rs` | AnswerPermission, 1, 0, AcpAdapter |
| `crates/htui-agent/src/auth/browser.rs` | opener, url, url, opener_command, platform_command |
| `crates/htui-agent/src/auth/mod.rs` | Neutralised, AuthMethodInfo, 0, name, id, ... |
| `crates/htui-agent/src/cli/claude.rs` | Totals, cost_micros, input, cache_write, output, ... |
| `crates/htui-agent/src/cli/mod.rs` | ClaudeStreamAdapter |
| `crates/htui-agent/src/conformance.rs` | usage, box_id, agent_id, QuotaCall, prompt_digest, ... |
| `crates/htui-agent/src/driver.rs` | deny, matcher, McpServerSpec, added_by, PermissionRequestId, ... |
| `crates/htui-agent/src/event.rs` | ToolCallEvent, PlanEvent, locations, accepted, Stamp, ... |
| `crates/htui-agent/src/excerpt.rs` | IgnoreRule, Suffix, rules, Name, 0, ... |
| `crates/htui-agent/src/install/fetch.rs` | sha256, bytes, path, Downloaded |
| `crates/htui-agent/src/install/http.rs` | content_length, status, HeadInfo |
| `crates/htui-agent/src/install/layout.rs` | restored, SweepReport, removed |
| `crates/htui-agent/src/install/manifest.rs` | Manifest, license, Consent, published, accepted_at, ... |
| `crates/htui-agent/src/install/mod.rs` | InstallPlan, done, args_differ, version, version, ... |
| `crates/htui-agent/src/install/registry.rs` | fetched_at, license, etag, cmd, Cached, ... |
| `crates/htui-agent/src/launch.rs` | tap, session, Interrupt, FallbackCommand, extra_args, ... |
| `crates/htui-agent/src/permission.rs` | kind, Rule, Default, PolicyStage, reason, ... |
| `crates/htui-agent/src/probe.rs` | settings, launch, handshake, env, SpawnTier2, ... |
| `crates/htui-agent/src/record.rs` | billing, QuotaLatch, turn, RunCap, row, ... |
| `crates/htui-agent/src/replay.rs` | reason, ReplayError, seq, kind |
| `crates/htui-agent/tests/auth.rs` | neutralised_sets_browser_to_an_existing_executable_that_exits_zero, launch_with_env, initialize, Script, pairs, ... |
| `crates/htui-agent/tests/auth_live.rs` | logout, mode, lines, methods, bytes, ... |
| `crates/htui-agent/tests/cli_conformance.rs` | edits, messages, Wire, message, tools, ... |
| `crates/htui-agent/tests/cli_live.rs` | Text, Line, claude_launch, 0, args, ... |
| `crates/htui-agent/tests/driver_contract.rs` | wire_enums_match_their_ana4_vocabularies, check_wire_enum, all, texts, T |
| `crates/htui-agent/tests/estimator_live.rs` | added_chars, bare_input, claude_probe, label, result, ... |
| `crates/htui-agent/tests/extensibility.rs` | every_seed_row_deserialises_into_the_launch_types |
| `crates/htui-agent/tests/install.rs` | delay, line, body, etag, status, ... |
| `crates/htui-agent/tests/launch.rs` | empty_settings_document_yields_every_documented_default, an_unknown_install_source_names_the_value_it_refused, a_row_without_an_install_block_re_serialises_unchanged, debug_never_prints_an_environment_value, claude_settings_round_trip, ... |
| `crates/htui-agent/tests/recorder.rs` | _request_id, _answer, usage, update_workspace, quota, ... |
| `crates/htui-core/src/fixtures.rs` | skill_versions, skills, revisions, skill_bindings, links |
| `crates/htui-core/src/model/agent.rs` | name, billing, models, default_model, AgentSeed, ... |
| `crates/htui-core/src/model/box_.rs` | max_concurrent_items, settings, BoxSettings, probed_tags, BoxInfo, ... |
| `crates/htui-core/src/model/hierarchy.rs` | id, NewRepo, default_branch, description, remote_url, ... |
| `crates/htui-core/src/model/item.rs` | reason, required_tags, project_id, required_tags, body, ... |
| `crates/htui-core/src/model/kind.rs` | ProjectSettings, token_budget, description, ResolvedPhase, copy_exclude, ... |
| `crates/htui-core/src/model/link.rs` | kind, project_slug, depth, deleted_at, from_item_id, ... |
| `crates/htui-core/src/model/mod.rs` | check_enum, run_enums_match_check_lists, link_kind_matches_check_list, box_and_agent_enums_match_check_lists, texts, ... |
| `crates/htui-core/src/model/note.rs` | created_by, via_step_id, NewNote, id, created_at, ... |
| `crates/htui-core/src/model/overlap.rs` | prefixes, Paths, OverlapRule, local, isolated, ... |
| `crates/htui-core/src/model/quota.rs` | found, source, observed_at, status, exhausted, ... |
| `crates/htui-core/src/model/run.rs` | output, default_isolation, a_snapshot_judge_without_a_template_still_decodes, RunStepSummary, SnapshotGraph, ... |
| `crates/htui-core/src/model/skill.rs` | id, description, created_by, created_by, SkillBinding, ... |
| `crates/htui-core/src/model/usage.rs` | UsageTotals, cache_write_tokens, cost_micros, input_tokens, cache_read_tokens, ... |
| `crates/htui-core/src/model/user.rs` | tag, updated_at, CapabilityTag, id, Deserialize, ... |
| `crates/htui-core/src/prompt/excerpt.rs` | max_file_bytes, scan_truncated, ExcerptSet, Identifier, notes, ... |
| `crates/htui-core/src/prompt/mod.rs` | TemplateRef, output, exit_code, diff_so_far, errors, ... |
| `crates/htui-core/src/prompt/settings.rs` | BudgetSource, excerpt_head_lines, Integer, unit, SettingKind, ... |
| `crates/htui-core/src/prompt/template.rs` | Ord, PartialOrd |
| `crates/htui-core/src/prompt/trim.rs` | elided_bytes, section_entries, name, tokens, TemplateRecord, ... |
| `crates/htui-core/src/seed.rs` | description, input_kinds, phases, PhaseSeed, prefix, ... |
| `crates/htui-core/src/store/mem.rs` | runs, items, ProjectReach, phases, repos, ... |
| `crates/htui-core/src/store/traits.rs` | patch, expected_version, id, new, create_workspace, ... |
| `crates/htui-core/tests/prompt_digest.rs` | oversize_with_secrets, value, serialised, no_trim_rung_re_renders_an_unscrubbed_input, the_record_vocabularies_spell_themselves_once |
| `crates/htui-orch/src/engine.rs` | caps, no_excerpts, the_first_candidate_selector_takes_the_first_eligible, FirstCandidate, NoSink |
| `crates/htui-orch/src/fake.rs` | trees, commits, DiffRequest |
| `crates/htui-orch/src/fanout.rs` | reasons, winner, JudgeVerdict |
| `crates/htui-orch/src/graph.rs` | repo_scope, snapshot, Resolved |
| `crates/htui-orch/src/isolate.rs` | now, labelled, before_hash, SystemClock, ResetReport, ... |
| `crates/htui-orch/src/isolate/git.rs` | locked, locked, Head, stdout, branch, ... |
| `crates/htui-orch/src/recover.rs` | Abandoned, Expired, Heartbeat |
| `crates/htui-orch/src/select.rs` | agent_id, agent_name, Skipped, cause |
| `crates/htui-store/src/cache/mod.rs` | CacheMeta, last_full_refresh_at, schema_version, db_fingerprint, built_at |
| `crates/htui-store/src/identity.rs` | hostname, box_id, BoxToml |
| `crates/htui-store/src/pg/mod.rs` | UpToDate, MigrationState, Pending, 0 |
| `crates/htui-store/src/pg/read.rs` | box_info |
| `crates/htui-store/src/pg/rows.rs` | promoted_at, selected, phase_name, status, title, ... |
| `crates/htui-store/src/pg/write.rs` | new, create_workspace |
| `crates/htui-store/src/qdrant_settings.rs` | Clone, api_key, QdrantSettings, url |
| `crates/htui-store/src/secret.rs` | qdrant_url, qdrant_key, FakeSlots, pg |
| `crates/htui-store/src/writer.rs` | new, create_workspace |
| `crates/htui/src/agent_worker.rs` | box_id, _writer, delay, Planning, text, ... |
| `crates/htui/src/app/action.rs` | 0, FocusSection, 1 |
| `crates/htui/src/app/state.rs` | box_name, active_runs, workspace, store, TopBarState |
| `crates/htui/src/cli.rs` | log, clear_dsn, demo, Args, offline_parses_next_to_the_log_flag, ... |
| `crates/htui/src/connection.rs` | Failed, AttemptOutcome, built_at, MigrationsPending, MirrorInfo, ... |
| `crates/htui/src/hierarchy.rs` | repo, project, link, local_path, ProjectEntry, ... |
| `crates/htui/src/preview.rs` | empty_excerpts, caps, the_empty_audit_registers_the_builtin_and_records_the_caps |
| `crates/htui/src/prompt_settings.rs` | ProjectValue, value, key, value, project, ... |
| `crates/htui/src/qdrant_settings_info.rs` | key_state, QdrantSnapshot, url_state, url_summary |
| `crates/htui/src/testkit.rs` | area, keys, title, replies, frame, ... |
| `crates/htui/src/ui/layout.rs` | tab_strip, Chrome, status, top_bar, body |
| `crates/htui/src/ui/overlay/registry.rs` | ANY, Hash, 0, OverlayId |
| `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | DetailId, 0, Scroll |
| `crates/htui/src/ui/tabs/backlog/list.rs` | Project, 0, Selection, Item, 0 |
| `crates/htui/src/ui/tabs/chat/mod.rs` | caps, ended, ChatSessionState, step_id, session_ref |
| `crates/htui/src/ui/tabs/chat/permission.rs` | PermissionStrip |
| `crates/htui/src/ui/tabs/chat/transcript.rs` | by, At, Tail, option_id, denied, ... |
| `crates/htui/src/ui/tabs/registry.rs` | 0, TabId |
| `crates/htui/src/ui/tabs/settings/connection.rs` | input, attempt_text, InFlight, Asking, Editor, ... |
| `crates/htui/src/ui/tabs/settings/hierarchy.rs` | index, Editing, stage, Project, index, ... |
| `crates/htui/src/ui/tabs/settings/kinds.rs` | Row, 0, 0, id, p, ... |
| `crates/htui/src/ui/tabs/settings/mod.rs` | 0, SectionId |
| `crates/htui/src/ui/tabs/settings/prompt.rs` | ProjectHeader, AppHeader, v, Error, Row, ... |
| `crates/htui/src/ui/tabs/settings/qdrant.rs` | 0, Row, Info, Error, Url, ... |
| `crates/htui/src/ui/tabs/skills.rs` | frame, ID, _ctx, Default, _scope, ... |
| `crates/htui/tests/chat_live_agy.rs` | usage, permissions, Observed, tool_calls, edits, ... |
| `crates/htui/tests/install.rs` | status, chunk_delay, Route, body |
| `crates/htui/tests/replay.rs` | on_scope_change, seen, ID, _scope, title, ... |
| `crates/htui/tests/settings.rs` | on_reply, CapturingProbe, id, on_scope_change, _ctx, ... |

## Connected Communities

- **src/model +14 dirs** (8 cross-edges)
- **htui-agent/tests +13 dirs** (5 cross-edges)
- **src/prompt +2 dirs · phase_implement_attempt2** (5 cross-edges)
- **htui/tests +1 dirs** (3 cross-edges)
- **src/acp · map** (2 cross-edges)
- **htui-agent/tests +4 dirs · json** (2 cross-edges)
- **src/acp · Wire** (1 cross-edges)
- **htui-agent · session_main** (1 cross-edges)
- **htui-agent/src +7 dirs** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-111")
explore(operation:"context", task:"understand src/model +25 dirs", format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
