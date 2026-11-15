# Plan: MOD-127 M2 - MCP server kind

**Source PRD**: `.claude/prds/mod-127-toolbox-binding.prd.md`
**Selected Milestone**: 2 - MCP server kind
**Complexity**: Large (eight serial tasks, one migration, six crates; routed PRD, ultracode for implement)
**Plan-only waiver**: `plan`'s confirm gate is waived (run from `/handoff-run`, sandbox `hr/MOD-127`).
**Builds on**: M1 (`.claude/plans/mod-127-toolbox-binding-m1.plan.md`, landed at `c00a878f`): table
`toolbox_binding` (`kind`, `ref_id`, four scope keys), five levels, `bound_skills(ResolveScope)` on both stores.

## Summary
A person defines an MCP server once (a Postgres record, edited as a form in the Skills tab), attaches it at any of
the five levels through the M1 binding table, and keeps its secrets as names of project secrets. At run start the
engine resolves, per agent-launching node, the servers that apply and **freezes** them into the run's
`GraphSnapshot` (personas precedent). Each step launches its agent with htui's own server plus exactly that frozen
set, rendered by the Claude CLI and ACP drivers; what an agent cannot use is skipped, and the step record names
every server with its level, or its skip reason. M1 left two things for this milestone: the Rust `BindingKind`
type and the frozen set. Command and env-preset kinds, and the htui agent adding entries unasked (`R-AGT-11`),
are M3.

## Shared decisions (every task reads this section)

- **D1 - Generic binding types (rename, no behaviour).** `SkillLevel` becomes `BindingLevel`, `SkillBinding`
  `Binding`, `SkillBindingKey` `BindingKey`; the key's `skill: SkillId` becomes `target: BindingRef`, with
  `BindingRef::{Skill(SkillId), McpServer(McpServerId)}` and `BindingRef::kind() -> BindingKind`,
  `BindingKind::{Skill, McpServer}` (serde `skill`, `mcp_server`; column `kind`). `BoundSkill`, `SkillChoice`,
  `resolve`/`collapse` keep their names (they stay skill-specific: pins, versions, globs). Store methods keep
  their names (`skill_bindings`, `set_skill_binding`, `StoreRequest::SetSkillBinding`): the key decides the kind;
  renaming the seam (`set_skill_binding`: ~60 code sites, `StoreRequest::SetSkillBinding`: 20, all in `htui`)
  buys nothing. *Deviation from "rename with the generic types" in M1 D1: only the types that carry the kind are
  renamed.* The Postgres row type does the same split (`ref_id` + `kind` -> `BindingRef`). Size of the rename
  (claim 2): `SkillLevel` ~184 lines, `SkillBindingKey` ~136, 13 + 14 struct literals, at least 12 reads of
  `key.skill`; a rename tool misses the string `"SkillLevel"` in `label.rs:492` (`word_tables`).
- **D2 - Record.** `McpServer { id: McpServerId, name, description, transport: McpTransport, command: String,
  args: Vec<String>, url: Option<String>, headers: Vec<(String, EnvValue)>, env: Vec<(String, EnvValue)>,
  allow: Vec<String>, deny: Vec<String>, created_by, created_at, updated_at }` in new
  `htui-core/src/model/mcp.rs`. `McpTransport::{Stdio, Http}`: Stdio needs `command` (no `url`), Http needs `url`
  (no `command`/`args`). `name` matches `^[a-z][a-z0-9_-]{0,31}$` (it forms the agent's `mcp__<name>__<tool>`
  names), is unique, and is never htui's own server name. `SERVER_NAME` (`"htui"`) is defined in `htui-mcp`,
  which depends on `htui-core`, so T2 **moves the const into `htui-core`** and `htui-mcp` re-exports it
  (`pub use`), keeping every existing path. Env and header keys are valid names; an env key starting with
  `secret::RESERVED_PREFIX` (`HTUI_`, ASCII case-insensitive) is refused by the same test (`check_env` itself
  takes `ResolvedSecrets`, not a list). No versions (a server is one row;
  edits are CAS on `updated_at`, as `update_skill`).
- **D3 - Secrets by reference.** `EnvValue::{Literal(String), Secret(String)}`, stored as JSON `{"literal":..}` /
  `{"secret":..}`. `Secret(name)` is the **name** of a project secret that `RunSecrets::env_for(project)` already
  supplies (`R-SEC-2`); the value is substituted only when the spec is built for a launch, into
  `McpServerSpec.env`/headers, which are already `Debug`-redacted. A `Literal` under a key that looks secret
  (case-insensitive contains `token`, `secret`, `password`, `passwd`, `key`, `authorization`, `cookie`) is refused
  at write time with a sentence that points at `Secret`. The frozen snapshot, step record, logs and `Debug`
  carry names only. `drive_once` already fetches `RunSecrets::env_for(&Project)` (`engine.rs:10446`, once per run,
  failure cached); `plan_mcp` takes that map. A name the map does not hold is a skip (`secret_missing`), not a run
  failure. If the fetch itself fails (a refused provider, `Err`), the step fails exactly as it does today: no
  server is launched on a half-resolved set. Keys are the provider's names, case-sensitive.
- **D4 - Attachment rules for the kind.** `kind = 'mcp_server'` rows: pin is `NULL` (no versions), activation is
  `always` or `off` (`glob` refused: nothing to match a path against), glob list empty, position irrelevant
  (servers order by name bytes). Same five levels and D2 shapes of M1; `check_attachment` gains the refusals;
  Postgres gets a CHECK for them. `ref_id` has no FK (M1 D3): the write path checks the server exists.
- **D5 - Resolution.** `resolve_mcp(&[Candidate]) -> Vec<BoundMcp>`: per server (`ref_id`) the most specific
  level wins whole (PRD default); `off` at the winning level hides it, and the hidden row is returned inactive
  (the step record shows it, dim). Result sorted by server name bytes. `BoundSkill::collapse`
  (`skill.rs:438`) is not generic (it keys on `skill_id`, orders by position then name), so `mcp.rs` **mirrors**
  it: stable sort by `Reverse(level)`, first row per `ref_id`, off winners kept, then name bytes; one function
  used by both stores, as skills. New seam
  `bound_mcp_servers(scope: ResolveScope) -> Vec<BoundMcp>` (`BoundMcp { server: McpServer, level,
  activation, active }`) on `WorkerStore`, `GraphSource` and both stores; test fakes return `vec![]`.
- **D6 - Freeze.** `GraphSnapshot` is `phases: Vec<SnapshotPhase>` plus `edges` (no per-node map; a node's key is
  `SnapshotPhase.name`; the judge is `SnapshotPhase.judge`, not a node, and sees the judged phase's attachments).
  Add `GraphSnapshot.toolbox: Option<SnapshotToolbox { servers: Vec<SnapshotMcpServer> }>` and
  `SnapshotPhase.mcp: Vec<McpPick>` (both `#[serde(default, skip_serializing_if)]`, not hashed by `topology`,
  `GraphSnapshot::V` not bumped, like `personas`). `SnapshotMcpServer` is the full neutral record (secret
  **names**, never values) deduped by content, sorted by name; `McpPick { server: usize, level, active }` indexes
  it. Built at run start in `graph::resolve_or_refused_with` **after `expand_divide`** (`graph.rs:1248`), for
  every `SnapshotPhase` row (step, planner, integrate, part, subgraph), scope `{workspace, project, item_kind:
  item.kind_id, phase: row.phase.id}`. The judge shares its phase's picks. **The snapshot is built before the
  run exists** (`enqueue_with`: `resolve_with` at `engine.rs:1517`, `create_run` at `:1543`), so `workspace:
  Option<WorkspaceId>` becomes a parameter of `resolve_with`/`resolve_or_refused_with` (about 25 callers, among
  them `htui/src/run_worker.rs:1722`, `htui-worker/src/runtime.rs:5315`,
  `htui/src/ui/graph_editor/notes.rs:673`); callers with no workspace pass `None`, and `resume_window`
  (`engine.rs:5314`) must not re-freeze. A run whose snapshot has no `toolbox` (made before M2) launches with
  htui's own server only: the frozen set is empty, nothing is resolved live. Later edits or deletes of a server
  never reach a started run.
- **D7 - Launch and support.** `DriverCaps` gains `mcp_stdio: bool`, `mcp_http: bool`, set in
  `registry::caps_from` (static per agent row; `Transport` has only `Acp` and `Cli`): every `Cli` agent
  `true/true`; ACP `true/false` in M2 (the ACP schema has `McpServer::Http`, but an agent advertises
  `mcp_capabilities.http` only in its `initialize` reply; reading it is out of scope, so http on ACP is skipped with
  reason `transport_unsupported`); the test fake `false/false` unless a test turns them on, which is how the
  "agent cannot use servers" case is exercised (no third real transport exists). At each step the
  engine partitions the frozen picks by the step agent's caps: rendered servers go into `SessionSpec.mcp`
  after htui's own lease server; skipped ones are not rendered and are recorded. A server whose name equals
  htui's own is dropped with `reserved_name` (a record made before the rule cannot exist; belt and braces).
- **D8 - Neutral spec grows.** `McpServerSpec` (htui-agent, `driver.rs:252`) gains `transport`, `url`, `headers`
  (`#[serde(default)]`, stdio stays the default so existing JSON and tests are unchanged). Claude CLI
  `mcp_config` emits `{"type":"http","url","headers"}` for Http; ACP `acp_server` builds `McpServer::Http` only if
  the pinned `agent-client-protocol` `McpServer::Http(McpServerHttp { name, url, headers })` (exists in the
  schema crate's v1 module; used only if D7 caps ever turn it on, so M2 builds and tests the mapping but no
  real ACP session sends it). Headers
  join `Debug` redaction. The CLI config file stays in the private 0700 directory (`R-ID-4`); nothing is written
  into a managed repo.
- **D9 - Tool lists.** `allow` pre-approves the server's tools (`mcp__<name>__<tool>`; empty `allow` means the
  existing policy decides, as for any unlisted tool). `tools::pre_approve(&mut PermissionPolicy, &ToolLease)`
  (`tools.rs:168`) reads only the lease, so T6 adds a sibling `pre_approve_server(policy, server, tools)` that
  appends the same `AllowOnce` rules via `qualified_tool_name` (only when `policy.default == Ask`, as now); it
  never builds a second `ToolLease` (token, drop hook). `deny` removes tools: pushed as `mcp__<name>__<tool>`
  into `SessionSpec.tools.deny` (`driver.rs:224`), which `cli::disallowed(&ToolExposure)` already turns into
  `--disallowedTools` (no signature change), and on the ACP path as `PermissionRule`s with a reject answer
  placed **before** the allow rules (rules run in order). `ToolExposure.allow` is a real allow-list: MCP `allow`
  never goes there, and never restricts (a `--allowedTools` entry widens, it does not narrow).
- **D10 - Step record.** `TrimRecord` gains `mcp_choices: Vec<McpChoice>` (always present, `[]` when none), the
  trim-record style of `skill_choices`: `McpChoice { server: McpServerId, name (scrubbed), level, activation,
  active, outcome: McpOutcome }`, `McpOutcome::{Rendered, Skipped(SkipReason)}`, `SkipReason::{Off,
  AgentUnsupported, TransportUnsupported, SecretMissing(String), ReservedName}` (serde snake_case, names only).
  The field is `skip_serializing_if = Vec::is_empty` (like `memory`), so a record with no server differs from v5
  only in `v`. `RECORD_VERSION` 5 -> 6 in `prompt/trim.rs:64` **and** `TRIM_RECORD_VERSION` in `capture.rs:47`
  (whose `TrimRecord` literal at `:417-431` gets the field). Pins that move: `htui-core/tests/it/prompt_digest.rs`
  `:1011-1030` (`v == 5`), `store/conformance.rs:36231` (`"v": 5`), the column comment pinned at
  `htui-store/tests/it/migrations.rs:904-916`, and the snapshots a full insta run finds. As `0007`, `0008`,
  `0010` and `0027` did, `0042` restates the `run_step.trim_record` comment for v 6. The Prompt pane adds one
  "mcp" row per choice, same dim style.
- **D11 - Scope limits.** Item runs only. Chat sessions (`AgentRuntime::start`) keep htui's own server and no
  bound servers; recorded in the HANDOFF phase note as a gap, not silently. The htui agent's unasked-add tool
  is M3 (it covers all three kinds at once).
- **D12 - Forms.** New "MCP servers" view beside Skills and Templates in the Skills tab (PRD default; MOD-124
  regroups it): a third `View` variant with entries in `ALL` and `VIEWS`. The attach pane works over a
  `BindingRef`: `AttachPane.skill`, `AttachMode::..{key}`, `Form.key`, `key_of`, `SkillsSnapshot::{entry, head,
  binding}`, `StaleWhat::Binding`, `SkillWrite` (`skills.rs:293-301`), `SkillsSnapshot` name lookups
  (`skill_name`, `snapshot_name`). `Detail` (shared right pane) needs no change. Every key and hint goes
  through `Keys`.
- **Migration** `0042_mcp_server.sql` (next free after `0041`; renumbers at `scripts/hr collect` if a sibling
  lands first, which moves the count pins listed in T3). Not mirrored into the offline cache (skills are not).
- **Rules for every task.** TDD (tests first); crate-scoped checks only; no workspace gate in an agent;
  `cargo fmt` and `cargo clippy -p <crate> --all-features -- -D warnings` on each crate touched; commit the task
  when its check is green (small commits); never `--no-verify`.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Generic keys | `crates/htui-core/src/model/skill.rs:37-46, 113-143, 231-242` | level by declaration order; key derives level |
| Shared rule | `crates/htui-core/src/store/traits.rs` `check_attachment` (M1) | one refusal chain both stores call |
| Pure resolver | `skill.rs` `collapse`/`resolve` | level-tagged list in, winners out |
| Record CRUD + CAS | `traits.rs:1265-1326` (`create_skill`, `update_skill`), `pg/write.rs:6698-6743`, `mem.rs:5023` | CAS on `updated_at`: NotFound, Stale, Constraint |
| Worker + serve | `crates/htui/src/store_worker.rs:891-950`, `skills.rs:338-469` | request enum, `StaleWhat`, re-read snapshot |
| Frozen data | `model/run.rs:729-781`, `graph.rs:776-979`, `command.rs:1147-1169` | `#[serde(default)]` field, built at resolve, read by `snapshot_of` |
| Launch | `engine.rs:10412-10638`, `tools.rs:118` | `SessionSpec.mcp` from lease; `pre_approve` |
| Render | `cli/mod.rs:399-416`, `cli/mcp_file.rs:51-102`, `acp/mod.rs:1155-1166` | neutral spec to stdio shape; private file |
| Secrets | `htui-orch/src/secrets.rs:81-129` | `env_for(project)` map; scrubber built from it |
| Step record | `prompt/trim.rs:266, 348, 1364`, `ui/tabs/backlog/detail/prompt.rs:217-261` | choices list, `to_value(&dyn Scrubber)`, `choice_line` |
| Migration pins | `crates/htui-store/tests/it/migrations.rs` | TABLES, column comments, `Pending(N)` counts |

## Files to Change
| Task | Files | Action |
|---|---|---|
| T1 | `htui-core/src/model/skill.rs`, `fixtures.rs`, every literal/`match` site of `SkillLevel`/`SkillBinding`/`SkillBindingKey` (M1 claim 11 list, plus what `cargo check --workspace --all-targets --all-features` flags) | UPDATE (rename; `BindingRef`, `BindingKind` with only `Skill` live) |
| T2 | `htui-core/src/model/mcp.rs` (CREATE), `model/mod.rs`, `model/ids.rs`, `store/traits.rs` (`check_attachment`, trait methods, `NewMcpServer`/`McpServerPatch`/`BindingFacts`) | CREATE/UPDATE |
| T3 | `htui-store/migrations/0042_mcp_server.sql` (CREATE; also restates the `run_step.trim_record` comment for v 6); `htui-core/src/store/{mem,conformance,worker}.rs`; `htui-store/src/pg/{read,write,rows}.rs`, `src/{worker,backend,writer}.rs`; `.sqlx/query-*.json`; `htui-store/tests/it/{migrations,mcp_servers,pg_conformance}.rs` (the `migrations.rs:904-916` comment pin moves); `htui-orch/src/{graph,fake}.rs`, `htui-worker/src/{graphs,runtime}.rs`, `htui-agent/src/conformance.rs`, `htui-agent/tests/it/recorder.rs` (trait plumbing only) | CREATE/UPDATE |
| T4 | `htui-agent/src/{driver,registry}.rs`, `cli/{mod,mcp_file}.rs`, `acp/mod.rs`, `htui-agent/tests/it/{cli_driver,acp_driver}.rs` | UPDATE |
| T5 | `htui-core/src/model/run.rs`, `htui-orch/src/{graph,command,engine}.rs` (`enqueue_with`, `resolve_with` workspace parameter), the ~25 `resolve` callers (`htui/src/run_worker.rs`, `htui-worker/src/runtime.rs`, `htui/src/ui/graph_editor/notes.rs`, tests) and their tests | UPDATE |
| T6 | `htui-orch/src/{engine,tools}.rs`, `htui-core/src/prompt/trim.rs`, `htui-core/src/capture.rs`, `htui-core/tests/it/prompt_digest.rs`, `htui-core/src/store/conformance.rs` (one `"v": 5` literal), `htui/src/ui/tabs/backlog/detail/prompt.rs`, `htui/tests/it/snapshots/*.snap` that move | UPDATE |
| T7 | `htui/src/{skills,store_worker}.rs` (incl. `StaleWhat`, `SkillWrite`, `SetSkillBinding.key` generalised to `BindingRef`) | UPDATE |
| T8 | `htui/src/ui/tabs/skills/{mod,library,attach,mcp}.rs` (`mcp.rs` CREATE; third `View` variant), `htui/src/ui/kit/label.rs`, `htui/tests/it/{skills,mcp_servers}.rs`, `snapshots/it__skills__*.snap` | CREATE/UPDATE |
| close | `HANDOFF.md` phase note (M2), PRD milestone row | per lifecycle P1 |

File-set independence (checked in step 3.5, which owns the final intersection): T4 (`htui-agent` only) shares
no file with T2/T3 but the lane is serial anyway; T6 and T7/T8 are disjoint (`engine.rs`/`trim.rs`/`prompt.rs`
vs `skills.rs`/`store_worker.rs`/`tabs/skills/*`); T3 touches `graph.rs`/`fake.rs` mechanically before T5 edits
`graph.rs`, and T5 then T6 both edit `engine.rs`, so the lane runs T1..T8 in order (all `engine.rs`/`graph.rs`
sharing tasks are serial).

## Tasks

### Task 1: generic binding types, no behaviour  (foundation: verifier)
- **Owns**: the rename list in D1 and every site it breaks.
- **Change**: D1. Use Gortex `refactor` rename for the three types, then the `skill` -> `target` field change by
  compiler-driven edits. `BindingRef`/`BindingKind` added, `McpServerId` **not yet** (T2); `BindingRef` has only
  `Skill` until T2 adds the variant. Pg row mapping: `ref_id` + `kind` -> `BindingRef::Skill`.
- **Tests first**: none new; the proof is that every existing test passes with constructor edits only. Add
  `a_binding_ref_knows_its_kind` and serde-spelling tests for `BindingKind` in `skill.rs`.
- **Check**: `cargo test -p htui-core --all-features --lib model::skill`, `cargo check --workspace --all-targets
  --all-features`, `cargo test -p htui-store --all-features --test it skill_attachments:: skill_writers::`, fmt,
  clippy on every crate touched. Do not edit an assertion.
- **Commit**: `refactor(core): MOD-127 M2 T1 generic binding types`.

### Task 2: MCP record, rules, resolver (pure)
- **Owns**: `model/mcp.rs` and the trait declarations in T2's row.
- **Add**: `McpServerId`; `McpServer`, `McpTransport`, `EnvValue`, `NewMcpServer`, `McpServerPatch`; D2/D3
  refusals as `new_mcp_server_refusal`, `mcp_server_patch_refusal`, `invalid_mcp_name` (sentences like
  `invalid_skill_name`); move `SERVER_NAME` into `htui-core` (re-exported by `htui-mcp`, D2); `BindingRef::McpServer`; D4 refusals in `check_attachment` (`BindingFacts` gains
  `target_exists`); `BoundMcp`, `resolve_mcp` (D5); `WriteStore::{mcp_servers, create_mcp_server,
  update_mcp_server(id, expected, patch) -> CasOutcome<McpServer>, delete_mcp_server(id) -> DeleteReach-like
  count of its bindings}`; `bound_mcp_servers(scope)` on `WorkerStore` and `GraphSource`.
- **Tests first** (in `mcp.rs`): name charset and reserved name; Stdio vs Http field combinations;
  `a_literal_under_a_secret_looking_key_is_refused` (each word in D3); `a_secret_reference_is_accepted`;
  `most_specific_level_wins_whole` (the mirror of `collapse`, D5); `off_at_a_more_specific_level_hides_the_server`; `off_row_is_returned_inactive`;
  `output_orders_by_name_bytes`; `glob_and_pin_are_refused_for_a_server_binding`; serde round trip of
  `EnvValue` pins `{"literal":..}`/`{"secret":..}`.
- **Check**: `cargo test -p htui-core --all-features --lib model::mcp model::skill`, `cargo check --workspace
  --all-targets --all-features` (trait additions break implementers: add `unimplemented!`-free stubs only in test
  fakes; T3 owns the real ones, so T2 commits only once the workspace compiles with minimal stubs that T3 replaces),
  fmt, clippy.
- **Commit**: `feat(core): MOD-127 M2 T2 MCP server record, rules and resolver`.

### Task 3: stores - migration, Mem, Pg, conformance  (foundation: verifier)
- **Owns**: migration, stores, `.sqlx`, the plumbing files in the table.
- **Add**: `0042_mcp_server.sql` (one transaction, forward-only): `CREATE TABLE mcp_server (id, name UNIQUE,
  description, transport CHECK, command, args TEXT[], url, headers JSONB, env JSONB, allow TEXT[], deny TEXT[],
  created_by, created_at, updated_at)` with the D2 shape CHECK and the `updated_at` trigger of `skill`; widen
  `toolbox_binding.kind` CHECK to `('skill','mcp_server')`; add the D4 CHECK `toolbox_binding_mcp_shape` (`kind <> 'mcp_server' OR (pinned_version IS NULL AND
  activation IN ('always','off') AND cardinality(globs) = 0)`; `toolbox_binding_kind_check` is dropped and
  re-added with both kinds; probed on Postgres 18.7, claim 13); restate the `run_step.trim_record` comment for
  record v 6 (D10); column comments. Pg read/write per the mirror
  rows; `bound_mcp_servers` = two queries (`kind = 'mcp_server'` rows for the scope's levels, then the records)
  into the same `resolve_mcp`; `delete_mcp_server` deletes its `toolbox_binding` rows in the same transaction and
  returns their count. Mem mirrors the SQL `WHERE` in Rust. `set_skill_binding` accepts the new kind.
  Regenerate affected `.sqlx` entries (recipe `docs/hr-sandbox.md`; scratch DB migrated; byte-identical SQL
  reuses an entry).
- **Tests first**: conformance (Mem and Pg): `mcp_server_create_edit_delete_with_cas`,
  `mcp_server_refuses_by_rule`, `mcp_binding_attach_change_detach_at_each_level`, `an_mcp_binding_refuses_pin_and_glob`,
  `deleting_a_server_takes_its_bindings`, `bound_mcp_servers_resolves_all_five_levels`,
  `a_skill_and_a_server_of_one_name_do_not_collide`. `mcp_servers.rs`: Pg and Mem agree on a fixed table of
  cases. `migrations.rs`: TABLES entry `mcp_server`, comment pins, the `Pending(N)`/count pins a new migration
  moves (about seven; the enumerated list is the new-migration rule), the `0042` file, `kind` CHECK widened.
- **Check**: `cargo test -p htui-core --all-features --lib store::`, `cargo test -p htui-core --all-features
  --test it mem_store::`, `cargo test -p htui-store --all-features --test it mcp_servers:: migrations::
  pg_conformance::`, `cargo check --workspace --all-targets --all-features`, fmt, clippy on touched crates. Read
  the test names that ran (substring filter).
- **Commit**: `feat(store): MOD-127 M2 T3 mcp_server table and binding kind`.

### Task 4: agent crate - neutral spec, caps, renderers
- **Owns**: the `htui-agent` files in the table.
- **Change**: D8 and the caps of D7. `McpServerSpec { transport, url, headers }` (default Stdio); `Debug`
  redacts header values; `mcp_config` Http shape; `acp_server` per D8; `DriverCaps.{mcp_stdio, mcp_http}` and
  `caps_from`; `cli::disallowed` takes the deny list (D9); the fake driver reports `false/false` unless a test
  turns them on.
- **Tests first**: `argv_without_mcp_is_unchanged` stays as is; new `an_http_server_renders_the_clis_http_shape`,
  `headers_never_reach_debug`, `a_stdio_spec_json_without_transport_still_decodes`,
  `the_acp_path_renders_a_stdio_server_with_env`, `caps_say_cli_stdio_http_acp_stdio_other_none`,
  `deny_entries_in_tools_deny_become_disallowed_tools` (no signature change, D9), `the_acp_http_mapping_builds_mcp_server_http` (unit, not wired).
- **Mirror**: `cli_driver.rs:654-703, 2994-3035`, `acp_driver.rs:2257-2296`.
- **Check**: `cargo test -p htui-agent --all-features --test it cli_driver:: acp_driver::` (read the names),
  `cargo test -p htui-agent --all-features --lib`, fmt, clippy on `htui-agent`. `htui-agent`'s extensibility test
  scans for a forbidden literal (see memory note "zeta"): do not use it in test data.
- **Commit**: `feat(agent): MOD-127 M2 T4 http servers, headers and mcp caps`.

### Task 5: freeze the set at run start  (foundation: verifier)
- **Owns**: `run.rs` (`SnapshotToolbox`, `SnapshotMcpServer`, `McpPick`, `SnapshotPhase.mcp`), `graph.rs`
  (resolution at run start), `command.rs` (reading back), `engine.rs` (`enqueue_with` passes the workspace;
  `resume_window` must not re-freeze), the `resolve` callers that gain the `workspace` argument, their tests.
- **Change**: D6. `resolve_or_refused_with` gains `workspace: Option<WorkspaceId>`, and after `expand_divide`
  calls `GraphSource::bound_mcp_servers` once per `SnapshotPhase` row, dedupes into `toolbox.servers`, fills each
  row's `mcp`. A store error resolving the set refuses the run start with the
  existing refusal path (a run must not start with a silently empty set when the read failed). The override-graph
  copy of bindings (`graph.rs` M1 note) stays phase-level skills only; assert it.
- **Tests first**: `the_frozen_set_holds_the_resolved_servers_per_phase_row`, `a_judge_sees_the_judged_phases_picks`,
  `planner_and_integrate_rows_of_a_divided_phase_get_picks`, `a_run_with_a_workspace_freezes_the_workspace_level`,
  `resuming_a_run_does_not_refreeze`,
  `a_phase_attachment_reaches_only_its_phase`, `a_snapshot_without_toolbox_decodes_and_freezes_nothing`,
  `a_later_edit_does_not_change_a_started_runs_set`, `a_failed_read_refuses_the_run_start`,
  `the_snapshot_carries_secret_names_not_values` (serialise, search for a sentinel value), `topology_ignores_toolbox`.
- **Mirror**: `graph.rs` persona tests, `command.rs:1147-1169`.
- **Check**: `cargo test -p htui-orch --all-features --lib graph:: command::` (names), `cargo test -p htui-core
  --all-features --lib model::run`, fmt, clippy on `htui-core` and `htui-orch`.
- **Commit**: `feat(orch): MOD-127 M2 T5 frozen toolbox set in the run snapshot`.

### Task 6: launch, secrets, step record
- **Owns**: `engine.rs` (`drive_once`, `open_tools`), `secrets.rs`, `tools.rs`, `prompt/trim.rs`, `prompt.rs`
  (pane), snapshots.
- **Change**: at each step read the node's picks from the frozen snapshot (`Engine::snapshot_of`), partition by
  `DriverCaps` (D7) and secrets (the map `drive_once` already holds from `env_for(&project)`; D3), build `McpServerSpec`s (stdio: command, args, env;
  http: url, headers; every `Secret(name)` replaced from the map, in memory only), append after htui's own lease
  server, feed `allow` to the new `tools::pre_approve_server` and `deny` to `SessionSpec.tools.deny` plus reject rules ahead of the allow rules (D9). Build `mcp_choices` for the prompt record
  (D10) **before** assembly so the record and the launch agree: one function `plan_mcp(picks, caps, secrets) ->
  (Vec<McpServerSpec>, Vec<McpChoice>)`, pure, tested alone. `RECORD_VERSION` and `TRIM_RECORD_VERSION` 6 (D10); `TrimRecord::to_value` scrubs
  names.
- **Tests first**: `plan_mcp`: `renders_servers_by_name_after_the_own_server`, `an_off_server_is_listed_not_rendered`,
  `a_cli_agent_gets_http`, `an_acp_agent_skips_http_with_a_reason`, `an_unsupported_agent_skips_every_server_and_the_run_starts`,
  `a_missing_secret_skips_that_server_only`, `a_secret_value_reaches_the_spec_and_no_record`, `a_reserved_name_is_dropped`, `a_failed_secret_fetch_fails_the_step_and_launches_nothing`, `deny_rules_precede_allow_rules`.
  Engine: `a_run_launches_with_exactly_the_frozen_set` (fake driver records its `SessionSpec`; a server attached
  after the run started is absent), `a_pre_m2_snapshot_launches_with_the_own_server_only`. Prompt pane:
  snapshot test with one rendered and one skipped row.
- **Mirror**: `engine.rs` tool-lease tests near `open_tools`, `prompt/trim.rs` skill-choice tests,
  `prompt.rs:217-261`.
- **Check**: `cargo test -p htui-orch --all-features --lib engine:: secrets::` filtered to new names, `cargo test -p
  htui-core --all-features --lib prompt::`, `cargo test -p htui --features testkit --test it prompt::` (insta; run
  the full set, impact is found by running, not grepping), fmt, clippy on touched crates.
- **Commit**: `feat(orch): MOD-127 M2 T6 launch the frozen servers and record each choice`.

### Task 7: UI worker plumbing
- **Owns**: `skills.rs`, `store_worker.rs`.
- **Change**: `StoreRequest::{CreateMcpServer, EditMcpServer {scope, id, expected, patch}, DeleteMcpServer}`,
  replies mirroring skills (`StoreReply::McpServerWritten`, stale via `StaleWhat::McpServer`), `SkillsSnapshot`
  gains `mcp_servers` and their bindings (`BindingHome` rows already carry both kinds). Request names go into the
  names table at `store_worker.rs:1506`.
- **Tests first**: worker tests beside the skill ones: `create_edit_delete_an_mcp_server`,
  `a_stale_edit_answers_stale`, `the_snapshot_lists_servers_and_their_bindings`.
- **Check**: `cargo test -p htui --features testkit --lib skills:: store_worker::` (names), fmt, clippy `htui`.
- **Commit**: `feat(ui): MOD-127 M2 T7 mcp server requests and snapshot`.

### Task 8: Skills tab - MCP servers view, form, attach rows
- **Owns**: `tabs/skills/*`, `label.rs`, UI tests, snapshots.
- **Change**: D12. A "servers" list view and an edit-as-form (name, transport, command, args, url, headers, env,
  allow, deny; env/header rows are `literal` or `secret` entries; a refusal sentence from T2 shows inline);
  delete asks first and names the count of bindings it removes; the attach pane works over `BindingRef`
  (`key_of` builds `BindingKey` with `target`). Labels for kind and for each `SkillReason`-style outcome stay
  in `label.rs`. Keys and hints through `Keys` (guard `no_hardcoded_keys`).
- **Tests first**: `create_a_stdio_server_through_the_form`, `a_literal_secret_is_refused_inline`,
  `attach_a_server_at_workspace_and_item_kind_level`, `deleting_a_server_names_its_bindings`, snapshot
  `it__skills__mcp_servers`.
- **Mirror**: `library.rs` `InfoForm`/`Editor`, `attach.rs:709-765, 1380`.
- **Check**: `cargo test -p htui --features testkit --test it skills::` (names), guards `--test no_hardcoded_keys
  --test kit_chrome`, fmt, clippy `htui`; snapshot diffs read and justified.
- **Commit**: `feat(ui): MOD-127 M2 T8 MCP servers view and attachments`.

## Run shape and review
- **Lane shape**: `scripts/task-lane.js`, one lane T1-T8 serial on this checkout, one implementer per task from
  `.claude/plans/mod-127-toolbox-binding-m2.briefs/Tn.md`. **Verifier** for T1, T3, T5 (foundation) and the fix
  round only. T6/T7/T8 go unverified to the gate and the review by design.
- **Gate**: once after the lane, `bash scripts/gate.sh`, plus `gate.sh --only doc` (public docs in `htui-core`).
  Also `cargo clippy --workspace -- -D warnings` (featureless).
- **Review**: `rust-reviewer` once over `git diff c00a878f..HEAD`, one fix round.
- **Owed on the host**: merged-tree Postgres gate and count pins if a sibling migration lands first; a smoke of
  a real Claude CLI run with one stdio server (a sandbox has no keyring and no logged-in CLI).

## Validation
```bash
cargo test -p htui-core --all-features --lib model::mcp model::skill store::
cargo test -p htui-store --all-features --test it mcp_servers:: migrations:: pg_conformance:: skill_attachments::
cargo test -p htui-agent --all-features --test it cli_driver:: acp_driver::
cargo test -p htui-orch --all-features --lib graph:: engine::
cargo test -p htui --features testkit --test it skills:: prompt::
cargo clippy --workspace --all-features -- -D warnings && cargo clippy --workspace -- -D warnings
bash scripts/gate.sh
```

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| T1 rename breaks literals and `match`es across crates (about 40 sites) | High | T1 alone, no behaviour, compile-driven; verifier; old tests unchanged in assertion |
| A secret value lands in a record, snapshot, log or file | Low, High impact | D3: names only everywhere; `the_snapshot_carries_secret_names_not_values`, `a_secret_value_reaches_the_spec_and_no_record`; CLI file stays in the private dir |
| Record freezing drifts from what launches | Medium | one pure `plan_mcp` produces both spec and record; engine test compares |
| ACP `McpServer::Http` type or http caps differ from assumption | Medium | D7 keeps ACP http off; D8 builds it only if the type exists; claim 6 |
| `allow` read as a restriction | Medium | D9 states it; test names it; help text in the form says "pre-approves" |
| New migration moves count pins | High | T3 lists them (new-migration rule); renumber at collect |
| Pg/Mem resolver drift | Medium | one `resolve_mcp`, one conformance suite, parity table |
| Prompt record bump moves snapshots outside the pane | Medium | full insta run in T6 before the gate |
| Chat sessions get no bound servers | Known | D11, written into the phase note |

## Acceptance
- [ ] T1-T8 committed; every earlier skill and M1 test unchanged in assertion
- [ ] A server attached at each of five levels resolves most-specific-wins on both stores (conformance + parity)
- [ ] A run launched on the Claude CLI and on ACP receives htui's server plus exactly the frozen set; a server
      edited, attached or deleted after run start is not seen by it
- [ ] An agent that cannot use servers skips each with a recorded reason and the run starts
- [ ] No secret value in any stored record, snapshot, log, `Debug` output or managed-repo file
- [ ] Gate green (or the named flake); reviewer findings applied or deferred with the maintainer

## Verified claims
Step 3.5, 2026-11-15, against the tree at `c00a878f`. The plan above is already amended where a claim was off.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | `collapse` can be reused for a pin-less kind | **amended** | `BoundSkill::collapse(Vec<BoundSkill>)` (`skill.rs:438`) keys on `skill_id`, orders by position then name; not generic. D5 mirrors it in `mcp.rs` |
| 2 | Size of the D1 rename and whether the seam names need no rename | **amended** | `SkillLevel` ~184 lines (core 94, orch 14, store 24, htui 50, agent 2), `SkillBindingKey` ~136, 13+14 struct literals, >=12 `key.skill` reads, `set_skill_binding` ~60 code sites, `SetSkillBinding` 20 (all `htui`); string `"SkillLevel"` at `label.rs:492` escapes a rename tool. Seam names stay |
| 3 | `0042` is next; the pin column and constraint names | **amended** | latest is `0041_toolbox_binding.sql`; the pin column is `pinned_version` (`0001_init.sql`), constraints `toolbox_binding_activation_check`, `toolbox_binding_kind_check` (`0041:26`). D4/T3 use them |
| 4 | Snapshot keyed by node; judge scope has no phase | **falsified, amended** | `GraphSnapshot.phases: Vec<SnapshotPhase>`; key `SnapshotPhase.name`; roles `Step`, `Planner`, `Integrate`, `Part`, `Subgraph`; the judge is `SnapshotPhase.judge` and `judge_prompts` passes the judged phase (`engine.rs:8902-8915`). D6 stores picks per phase row, freezes after `expand_divide` (`graph.rs:1248`) |
| 5 | The run's workspace and item kind are reachable at snapshot build | **falsified, amended** | `resolve_or_refused_with` (`graph.rs:776`) has no `Run`; snapshot built at `engine.rs:1517`, run created at `:1543`. Item kind is reachable (`item.kind_id`); workspace becomes a parameter (~25 callers). `GraphSource` is the right seam |
| 6 | ACP schema has `McpServer::Http`; `caps_from` is static | confirmed | `agent-client-protocol-schema-1.7.0/src/v1/agent.rs` `McpServer::Http(McpServerHttp{name,url,headers})`, gated on `mcp_capabilities.http` from `initialize`; `registry.rs:157` matches `agent.transport` only; `Transport` has `Acp` and `Cli` only. D7 reworded (every Cli agent true/true; fake for unsupported) |
| 7 | `pre_approve` and `disallowed` take other servers' tool names | **amended** | `pre_approve(&mut PermissionPolicy, &ToolLease)` (`tools.rs:168`) reads only the lease: D9 adds `pre_approve_server`. `cli::disallowed(&ToolExposure)` already honours `tools.deny` (no change). Reject-rule variant names in the ACP policy to be read at T6 (`driver.rs:204-215`) |
| 8 | `env_for` is callable per step and gives secret names | **amended** | takes `&Project`, caches once (failure too), `Err` for a refused provider, empty map without provider; `drive_once` already calls it (`engine.rs:10446`). D3 reworded: refused provider still fails the step |
| 9 | `RECORD_VERSION` pins | **amended** | `trim.rs:64` and `capture.rs:47`; pins `prompt_digest.rs:1011-1030`, `conformance.rs:36231`, `migrations.rs:904-916`; `0042` restates the comment; `mcp_choices` skips when empty. Snapshots by full insta run in T6 |
| 10 | `SERVER_NAME` importable from `htui-core` | **amended** | defined `htui-mcp/src/lib.rs:29`; `htui-mcp` depends on `htui-core` (cycle). D2: move the const into core, re-export. `check_env` takes `ResolvedSecrets`; the rule uses `RESERVED_PREFIX` |
| 11 | Skills tab can host a second list; `AttachPane` selection by `SkillId` | **amended** | `View { Skills, Templates }` with `ALL`/`VIEWS`; `AttachPane { skill: SkillId, .. }` (`attach.rs:130`); types to generalise listed in D12 |
| 12 | Task file sets | confirmed by intersection | T4 (`htui-agent`) shares no file with other tasks; `engine.rs`/`graph.rs` are shared by T3 (mechanical), T5, T6, which run serially in lane order; T7/T8 disjoint from T5/T6 |
| 13 | Probe: widened `kind` CHECK, MCP shape CHECK, `mcp_server` shape CHECK on Postgres 18 | confirmed | scratch DB on the sandbox Postgres (18.7): a pinned or glob `mcp_server` row refused, a plain row inserted, a stdio record inserted; `NULLS NOT DISTINCT` key unaffected by the CHECK change |
