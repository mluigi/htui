# Plan: MOD-23 — agent registry editing in the Settings agents section

> **Status: draft, awaiting fact-check and the maintainer's answers to OQ-1…OQ-5** (2026-09-29).
> Every tree fact this plan relies on is listed under "Claims to verify"; "Verified claims" is left
> empty for the fact-check pass.

**Source**: `HANDOFF.md` MOD-23 (the checklist line at `:601-632`, from MOD-2). Create a manual
agent row and edit an existing one — transport (`acp` | `cli`), launch command and args, model
list, default model, billing mode, the `agent` row's `enabled` and the per-box `agent_box.enabled` —
in the **`agents` section of the Settings tab** (`crates/htui/src/ui/tabs/settings/{mod.rs,agents.rs}`),
off the UI task. Registry writes are server-only (`REGISTRY_ON_SERVER_ONLY`, MOD-6 plan D52); since
MOD-25 a box with no DSN browses read-only and edits nothing (`docs/decisions/mod/mod-25.md`). The
item recommends an **edit pane below the table** over a ninth column (MOD-2 plan D76's budget). Out
of scope: the caps editor (MOD-12), box-profile capability edits (MOD-7, done), anything keyed on an
agent's name (`R-AGT-5`).

**Requirements**: `R-AGT-4` (registry fields, "enabled per box"), `R-AGT-6` ("mark enabled on that
box … Manual entries allowed"), `R-TUI-8` (Settings: agent registry), `R-NF-3` (no store handle and
no blocking work on the render side). `R-AGT-5` is a constraint, not a deliverable: nothing added
here may branch on an agent's name.

**Complexity**: Medium. Under OQ-1's recommended answer: **one migration** (`0008`, one column on
`agent_box`), one new `WriteStore` method across the five implementors and one store conformance
case, one field on `AgentSummary`, one new `crates/htui` module (`agent_settings`, beside
`box_settings`), three `StoreRequest` variants and one `StoreReply` variant, one promoted dependency
(`shell-words`, already compiled), and the edit pane in `settings/agents.rs`. **No change** to
`htui-orch`, the probe's status mapping (MOD-2 D50), `ProbeSource` (D45/D51), `GraphSource` or the
cache mirror.

**Routing**: routed as **plan** (maintainer accepted 2026-09-29). **Staffing: Opus 5.5 for every
step — plan, fact-check, architect, implementers, verifiers and reviewer (`rust-reviewer`); Fable is
not used (maintainer standing instruction).** Ultracode for the implementers only, one workflow per
task, verify fan-out per round; the architect and the reviewer stay plain agents.

**Numbering**: decision IDs continue the global sequence of `.claude/plans/*.plan.md`, whose highest
is **D229** (`mod-50-concepts-index-requirements.plan.md`). This plan's decisions are **D230…D248**.
Risks **R-1…R-10**, open questions **OQ-1…OQ-5**, tasks **T0…T3**, all local to this plan and cited as
"MOD-23 R-n" etc. outside it. Earlier items' decisions are cited with their item: "MOD-2 D45",
"MOD-40 D5", "MOD-7 D48".

**Base**: `hr/MOD-23` at `b5e481f`. Every line number below is the file's at `b5e481f`, pre-edit.
`df -h /` showed 101 GB free (78 % used) when this plan was drafted.

**Tooling note**: the Gortex graph does not track this checkout (every call answers
`repo_not_tracked`) and `graphify-out/` does not exist, so every tree fact here was read with plain
file reads and `grep`. The fact-check pass re-reads each at its line.

---

## Open questions for the maintainer

Each has a recommended answer the plan adopts, so implementation is not blocked once it is
confirmed.

- [ ] **OQ-1 — How a per-box "off" survives a re-probe.** Today the probe owns
      `agent_box.enabled`: `agent_box_row` sets `enabled: snapshot.status == ProbeStatus::Ready`
      (`crates/htui-agent/src/probe.rs:1530`), and `PgStore::upsert_agent_box` rewrites it on every
      conflict (`enabled = EXCLUDED.enabled`, `crates/htui-store/src/pg/write.rs:1206`). A switch the
      user flips in Settings would be undone by the next `r`, the next chat's re-probe or the next
      box probe. `R-AGT-4` lists "enabled per box" as a registry field; `R-AGT-6` says the probe
      "mark[s] enabled on that box". **Recommended (B, D242):** the probe proposes, the human
      vetoes. Migration `0008_agent_box_user_off.sql` adds `agent_box.user_off BOOLEAN NOT NULL
      DEFAULT false`; `upsert_agent_box` writes `enabled = EXCLUDED.enabled AND NOT
      agent_box.user_off` on conflict; a new narrow writer `set_agent_box_enabled` is the only
      writer of `user_off`; `AgentSummary` gains `user_off` so the section can show the switch.
      MOD-2 D50, ANA-4 §11 criterion 9 and every probe test keep their meaning, and no reader of
      `agent_box.enabled` changes. Cost: the first migration since `0007`, the migration-count pins
      and one `AgentSummary` field. **Alternative (A):** no migration — `agent_box.enabled` becomes
      the human's column (the probe inserts it `true` and never updates it, MOD-2 D74's
      single-writer shape) and "can this box run it" lives in `probe.status` alone. That reverses
      MOD-2 D50's `enabled = false` for `missing`/`failed`/`unauthenticated`, rewrites ANA-4 §11
      criterion 9, changes `htui-orch`'s `rung_three` (`crates/htui-orch/src/graph.rs:792-812`, which
      reads only `row.enabled`) and about fifteen test assertions (`htui-agent/tests/probe.rs`,
      `probe_live.rs`, `agy_live.rs`, `crates/htui/src/agent_worker.rs`, `crates/htui/tests/chat.rs`).
      **Alternative (C):** drop the per-box switch from MOD-23 and open a follow-up item.
- [ ] **OQ-2 — How `args` is typed in one line.** `agent.launch.args` is a `Vec<String>`
      (`crates/htui-agent/src/launch.rs:80`), and an argument may hold a space or a quote.
      **Recommended (D236):** POSIX shell words through the `shell-words` crate — `join` prefills the
      field, `split` parses it, an unclosed quote is refused by name — promoted to a declared
      dependency the way `zeroize` and `hmac` were (`Cargo.toml` `[workspace.dependencies]`). It is
      already compiled, at 1.1.1, as a dependency of `agent-client-protocol` (`Cargo.lock:5934-5936`,
      listed by that package at `:195`), so nothing new is downloaded. Cost: a Windows path with
      backslashes has to be quoted or written with `/` (R-5). **Alternative:** a JSON array
      (`["--uid=", "${x}"]`) through `serde_json`, lossless and dependency-free but clumsy to type.
- [ ] **OQ-3 — Is the ANA-4 §4.6 per-box "manual entry" part of MOD-23?** ANA-4 describes it as
      "writing `agent_box.path` and `probe.resolved` by hand in the Settings tab and setting
      `probe.status = "ready"`" (`docs/ANA-4.md:796-798`), protected by MOD-2 D45/D51's
      `source: manual`. In the tree that recording would not reach a spawn:
      `ProbeSnapshot::recorded_launch` answers `None` for any `source` other than `probe`
      (`probe.rs:1174-1177`), and the driver resolves `agent.launch` against `probe.tools` instead.
      **Recommended (D248):** out of scope; the main thread opens a follow-up item for a per-box tool
      path editor. A registry row whose `launch.command` is a literal path needs no probe at all
      (`crates/htui-core/src/model/agent.rs:41-42`), so MOD-23's create form already gives "manual
      entries allowed" for the common case. **Alternative:** build the per-box entry here (a second
      form writing `probe.tools`, `source: manual`), roughly doubling T3.
- [ ] **OQ-4 — ANA-21's `settings.weights`.** `docs/ANA-21.md:552` says "MOD-23's editor surfaces
      `settings.weights` alongside the model list and default model". MOD-36, which consumes the
      weights, is open. **Recommended (D248):** not in MOD-23; `settings` is carried through every
      edit untouched (D239), and MOD-36 adds a weights field to this form, which D231's field list
      makes a one-entry change. **Alternative:** a read-only weights line in the edit pane now.
- [ ] **OQ-5 — What a new row's `settings` document holds.** A `cli` row reaches an adapter only
      through `settings.cli.stream` (`adapter_id_from`, `crates/htui-agent/src/registry.rs:137-144`;
      a missing block yields the bare id `cli`, which no build registers). A new row with
      `settings = {}` is therefore a `cli` row that can never chat, and the form cannot edit
      `settings`. **Recommended (D239):** `n` copies the **`settings` document only** from the
      highlighted row, and the pane's header names that row (`new agent · settings from
      claude-cli`). `launch` starts blank (`{command, args, env: {}}`, no `discovery`), because a
      copied `discovery` block would make the probe look for the source row's tools and report
      `missing` for a row whose literal command is fine. On an empty table, `settings` is `{}`.
      **Alternative:** always `{}`, and state in the notice that a new `cli` row needs its
      `settings.cli` block written by SQL.

---

## Summary

The Agents section is read-only today: it lists the registry and this box's `agent_box` row and binds
`j`/`k`, `r` probe, `i` install, `a` authenticate, and `o`/`x` during a flow
(`crates/htui/src/ui/tabs/settings/agents.rs:1117-1207`). The store seam for the registry row
already exists: `WriteStore::upsert_agent(agent, expected)` is a compare-and-set on
`agent.updated_at` (MOD-40 D5; `crates/htui-core/src/store/traits.rs:324-355`), and MOD-40's PRD
names MOD-23 as its first production caller (`.claude/prds/mod-40-multi-writer-hardening.prd.md:103`).
Its only callers today are tests and `fixtures::edit_agent` (`crates/htui-core/src/fixtures.rs:2021`).

**The per-box switch (T0).** Under OQ-1's recommendation: migration `0008` adds
`agent_box.user_off`; `WriteStore::set_agent_box_enabled(agent_id, box_id, enabled)` sets it and
re-derives `enabled`; `upsert_agent_box` can no longer turn a switched-off row back on;
`AgentSummary.user_off` carries it to the section.

**The draft (T1).** A new `crate::agent_settings` module holds `AgentDraft` and the pure parsing and
merging rules: transport, command, shell-word args, comma models, default model, billing, enabled,
and the create-only name. The section uses them for instant refusals and the worker uses them again
before it writes.

**The writes (T2).** `StoreRequest::{CreateAgent, EditAgent, SetAgentOnBox}` are served in the store
loop by `agent_settings::serve`: the worker mints the id and the clock, merges `command`/`args` into
the stored `launch` object, and calls `upsert_agent` or `set_agent_box_enabled`. It re-reads the
registry and answers one self-naming `StoreReply::AgentWritten { agents, outcome }`. Offline, all
three are refused with `REGISTRY_ON_SERVER_ONLY`.

**The pane (T3).** `n` opens a create form and `e` an edit form under the table. Both use
hierarchy's `Field`/`Editor` shape over the existing `TextField`. `t` flips this box's switch. The
hint becomes two lines: the keys, then the notice or the quota note.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D230 | **The surface is a pane under the table, not a ninth column.** The eight-column table and its constraints (`agents.rs:1078-1089`) do not change. The edit form takes the pane slot the consent and login panes use (`pane`, `agents.rs:927-948`; `render`, `:1271-1294`). The form is only open while no install and no login is in flight (D232), so the slots never compete. | The item's recommendation. The seven fixed columns sum to 81 plus 7 gaps, so `name` (`Fill(1)`) gets 10 at the bordered 98 (`agents.rs:1045-1047`); there is no free column. |
| D231 | **The form is hierarchy's `Field`/`Editor` shape** (`crates/htui/src/ui/tabs/settings/hierarchy.rs:150-170`, `:779-822`, `:1301-1360`), private to `agents.rs`. It has one `TextField` per field, `Tab`/`Down` and `BackTab`/`Up` move focus, `Enter` submits and `Esc` cancels. `CONTROL` chords pass (so `ctrl-c` quits) and everything else is swallowed. `captures_input` answers `true` while the form is open (`mod.rs:149-151`; `SettingsTab::on_key` gives such a section every key first, `:323-331`). **Fields, in tab order:** `name` (create only), `transport`, `command`, `args`, `models`, `default model`, `billing`, `enabled (y/n)`. The label column is 13 wide (`default model`), which leaves 83 columns of text at the bordered 98. | The multi-field form already exists and is tested; MOD-15 PRD D1 built `TextField` for MOD-22 and MOD-23 (`docs/decisions/mod/mod-15.md:99`). A form of one-line fields keeps `Enter` meaning "save" (Boxes' quirks editor needed `ctrl-s` only because `TextArea` takes `Enter`). |
| D232 | **Keys and exclusion.** `n` opens the create form, `e` the edit form over the highlighted row, and `t` flips this box's switch for the highlighted row. All three are free in the section: the global keymap binds `q`, `?`, `ctrl-c` and `-` (`crates/htui/src/keymap.rs:209`, `:235`, `:248`, `:353`), and the section binds `j`, `k`, `i`, `a`, `r`, and `o`/`x` during a flow. `n`/`e`/`t` are refused with one sentence while a probe, an install or a login is in flight (the `begin_install` rule, `agents.rs:729-750`). `r`/`i`/`a` are refused while a registry write is in flight (`busy`). There is no reload key: `r` stays the probe. | A probe, an install and a login each end by writing this box's `agent_box` row and re-reading, and an edit that changed `launch` under a running probe would be recorded against the old recipe. One write at a time is the Boxes/hierarchy `busy` rule (`boxes.rs:251-260`). |
| D233 | **`name` is set at create and never edited.** The edit form has no `name` field; its header line says `edit <name>`. | `PgStore::seed_if_empty_as` tops the seed rows up by name with `ON CONFLICT (name) DO NOTHING` (`crates/htui-store/src/pg/mod.rs:445-450`), so a renamed seeded row would come back under its old name on the next connect. And `agent_box_row` falls back to `snapshot.tools.get(&agent.name)` for the version (`probe.rs:1535`). Retiring a row is `enabled = false`, not deletion or renaming (`pg/mod.rs:377-378`). |
| D234 | **Name rule** (`agent_settings::valid_name`): 1–64 chars from `[a-z0-9._-]`, first char `[a-z0-9]`. The section also refuses a name already in its list, locally. The store's `UNIQUE` is the authority (`Constraint`, `traits.rs:348-349`). | The seeds `claude`, `agy` and `claude-cli` pass. The name is printed in every refusal sentence the section coins (`agents.rs:378-381`) and in tracing fields, so control characters and spaces are refused at the door. |
| D235 | **The launch edit is a merge, not a rewrite** (`agent_settings::merge_launch`). The stored `launch` must be a JSON object. The form's `command` (trimmed, non-empty) replaces `"command"` and its parsed `args` replace `"args"`. Every other key is carried byte for byte: `env`, `discovery` (with MOD-20's `install` inside it, `launch.rs:153-158`) and any key this build does not know. The merged document must deserialise as `htui_agent::launch::AgentLaunch` (`launch.rs:75-88`), or the save is refused with the serde sentence. `env` is **never shown** in the form. | `env` may carry `${tool}` placeholders for tokens (`crates/htui-agent/tests/extensibility.rs:30-49`), and `R-SEC-2` keeps secrets out of rows. A form that cannot see `discovery` must not be able to drop it. |
| D236 | **`args` is shell words** (OQ-2 recommended): prefilled with `shell_words::join(&args)` and parsed with `shell_words::split`. An unclosed quote is refused as `` `args`: <error> ``. `shell-words = "1.1"` is promoted to `[workspace.dependencies]` with a one-line reason and declared in `crates/htui/Cargo.toml`. A table test pins `split(join(a)) == a` for the empty list, an empty argument, spaces, both quote kinds, `--uid=`, `${tool}` and a backslash. | Lossless for every seed. The prefill round-trips, so an edit of `models` cannot mangle `args`. |
| D237 | **`models`** is comma-separated: trimmed, empties dropped, order kept, a repeat refused by name. **`default model`**: empty is `None`. When `models` is non-empty, the default must be one of them; when it is empty, any value is accepted. | Order matters: `rung_three` falls back to `models.first()` (`graph.rs:815`). The seed test states the default-in-list rule (`crates/htui-core/src/model/agent.rs:250-256`). An empty list with a default is `claude-cli`'s shape: the model travels as an argv flag and nothing reports a list (`agent.rs:119-123`). |
| D238 | **`transport`** and **`billing`** parse through their `FromStr` (`str_enum!`, `crates/htui-core/src/model/mod.rs:64`) after trim and ASCII-lowercase. A refusal names the accepted values (`` `transport` is acp or cli ``). **`enabled (y/n)`** uses `yes_or_no` (`mod.rs:53-59`). | One convention per field kind, shared with the other editors. |
| D239 | **Writes go through `upsert_agent`.** Create: the worker mints `AgentId::new()` and stamps `created_at = updated_at = Utc::now()`. It builds `launch = {command, args, env: {}}` and takes `settings` from the `settings_from` row (OQ-5 recommended), else `{}`. It calls `upsert_agent(&row, None)`. Edit: the worker reads the row from `backend.agents()`, applies the draft (D235, D237, D238), keeps `name`, `settings`, `created_at` and every `launch` key but two, and calls `upsert_agent(&merged, Some(expected))`. `expected` is the `updated_at` the section read from an `Agents` reply, never one it built (MOD-40 blueprint F-17, `traits.rs:337-339`). Outcomes: `Applied` → `Created`/`Edited`; `Stale` → `Stale`; an id missing from the re-read, or `NotFound { entity: "agent" }` → `Gone`; `Constraint` (a taken name) → `StoreReply::Failed` with the store's sentence, and the form stays open. | The CAS already exists and is conformance-tested on both stores; MOD-23 adds no registry writer. Minting in the worker keeps the section free of ids and clocks, as `hierarchy::serve` does for `CreateWorkspace` (`crates/htui/src/hierarchy.rs:239`). |
| D240 | **One self-naming reply.** `StoreReply::AgentWritten { agents: Vec<AgentSummary>, outcome: AgentWrite }` (boxed if clippy asks, like `Boxes(Box<BoxesSnapshot>)`). `AgentWrite = Created { id, name } \| Edited { id, name } \| Stale { id } \| Gone { id } \| Switched { id, name, enabled }`. `agents` is the registry re-read after the write in every outcome. The section lands a write on this variant alone. A plain `StoreReply::Agents` (activation, a probe's answer, a login's re-read) replaces the rows and never closes the form or refreshes its token. | This is MOD-59's recommended shape (`HANDOFF.md:302-313`). It retires the `busy` attribution residue Boxes documents (`boxes.rs:12-16`) before this section acquires it. `Agents` is also the probe's reply (`store_worker.rs:210-212`), so it cannot double as a write's. |
| D241 | **Served in the store loop by `crate::agent_settings::serve`**, routed from `try_serve` with one or-ed arm, as `box_settings` is (`crates/htui/src/store_worker.rs:1356-1361`). `REQUEST_NAMES: [&str; 3] = ["create_agent", "edit_agent", "set_agent_on_box"]` feeds `StoreRequest::name` and the section's `Failed` arms, with a names test like `box_requests_are_named_as_box_settings_lists_them` (`:3313`). With no writer (`Backend::writer()` is `None` offline, `crates/htui-store/src/backend.rs:153-159`), all three answer `Unreachable(REGISTRY_ON_SERVER_ONLY)`; the constant is at `crates/htui-store/src/writer.rs:76`, the `recording_writer` sentence (`agent_worker.rs:2521-2526`). `SetAgentOnBox` resolves this box through `backend.box_info()` and answers `Failed` naming an unregistered box when there is none. | `R-NF-3`: a registry write is one statement plus one read, the cost of the loop's other catalogue writes. MOD-25: a box with no DSN edits nothing, and there is no second path to build. |
| D242 | **The per-box switch** (OQ-1 recommended, B). **Migration** `crates/htui-store/migrations/0008_agent_box_user_off.sql`: `ALTER TABLE agent_box ADD COLUMN user_off BOOLEAN NOT NULL DEFAULT false;` with a `COMMENT ON COLUMN` that names its one writer and the upsert rule. There is no cache migration, because `agent_box` is not mirrored (`crates/htui-store/cache_migrations/0002_agent_mirror.sql` header). **`WriteStore::set_agent_box_enabled(&self, agent_id: AgentId, box_id: BoxId, enabled: bool) -> Result<()>`**, on Postgres one statement: `INSERT INTO agent_box (agent_id, box_id, enabled, user_off) VALUES ($1, $2, $3, NOT $3) ON CONFLICT (agent_id, box_id) DO UPDATE SET user_off = NOT $3, enabled = $3 AND (agent_box.probe IS NULL OR agent_box.probe->>'status' = 'ready')`. It never writes `probe`, `version`, `path`, `probed_at`, `quota` or `quota_at`; `updated_at` moves by the trigger. `23503` maps to `Constraint`. **`upsert_agent_box`** keeps its insert and changes one `SET` term to `enabled = EXCLUDED.enabled AND NOT agent_box.user_off`. **MemStore** keeps a `HashSet<(AgentId, BoxId)>` of switched-off rows beside `agent_boxes` (`crates/htui-core/src/store/mem.rs:179`) and mirrors both rules in `State::upsert_agent_box` (`:1656-1690`) and the new setter. **`AgentSummary.user_off: bool`** (`#[serde(default)]`) is filled by `PgStore::agents` (`ab.user_off`, `crates/htui-store/src/pg/read.rs:1285-1352`) and `State::agent_summaries` (`mem.rs:1477-1489`); `CacheStore::agents` answers `false` (`crates/htui-store/src/cache/read.rs:1420-1434`). **No compare-and-set**: the switch is an absolute set, so there is no read-modify-write to lose, and a token on `agent_box.updated_at` would be spent by every probe. Two people flipping one switch at once end on the last one, and both see it on the re-read. **Switching on** re-derives `enabled` from the stored probe with `agent_box_row`'s rule (`probe.rs:1530`), so an `unauthenticated` row stays unavailable and says so. A row the switch inserted has `probed_at` and `probe` `NULL`, so `select::not_ready` treats it as unknown (`crates/htui-orch/src/select.rs:190-201`). Once switched back on it counts as "enabled on this box" in `rung_three` (`graph.rs:792-812`); that is the user's explicit statement, and R-7 records it. | The probe keeps MOD-2 D50's meaning, and every reader of `agent_box.enabled` (`select.rs:195`, `graph.rs:802`, the section's `(off)`) is right without a change. The single-writer-per-column shape is MOD-2 D67/D74's (`traits.rs:376-416`). |
| D243 | **The probe's own reply honours the switch.** `probe_agents_on` builds each written row in memory and hands it back as the section's `Agents` reply (`crates/htui/src/agent_worker.rs:2147-2180`, the write at `:2173`). It gains `row.enabled &= !summary.user_off` before `summary.on_box = Some(row)`, so the reply agrees with what the store kept. | Without it, `r` on a switched-off row would show it enabled until the next read. The write itself is already right (D242). |
| D244 | **The `on this box` cell.** `on_box_cell` (`agents.rs:866-895`) gains two rules after the install, login and probing states. `user_off` reads `switched off` (12 chars, within the 13-wide column), ahead of the probe words, because it decides selection before they do. A row with `probed_at` and `probe` both `None` reads `not probed`, as an absent row does. | The switch has to be visible where the row is. A row the switch created is not a probed row. |
| D245 | **The hint becomes two lines.** The layout becomes `[rows Min(3), pane, keys Length(1), note Length(1)]`. Line one is the keys; line two is the notice, else `QUOTA_NOTE` in the idle state. Idle keys are `j/k select · n new · e edit · t this box · r probe · i install · a authenticate` (79 chars). The form's keys are `Tab next field · Enter saves · Esc cancels` (42). | Today's single line is 95 of 98 columns (`HINT_IDLE` 49 + ` · ` + `QUOTA_NOTE` 43, `agents.rs:85`, `:98`). Three more keys cannot share it, and ANA-4 §7 wants the quota limit stated. This is Boxes' `[body, hint, notice_area]` (`boxes.rs:519`). `the_idle_hint_says_r_cannot_refresh_quota` reads the **last** line (`crates/htui/tests/settings.rs:788-791`), which stays the note. |
| D246 | **No forced re-probe after an edit.** `needs_reprobe` already treats `agent.updated_at > probed_at` as stale (`agent_worker.rs:2350-2359`), so the next chat on an edited row re-probes by itself. When `transport`, `command` or `args` changed, the `Edited` notice adds `the next chat re-probes; r probes now`. | The existing trigger covers it. An automatic probe after every save would spawn processes for a model-list edit. |
| D247 | **Validation runs twice, from one set of functions.** The section calls `agent_settings::draft_from_fields` for instant, local refusals (the notice names the field). The worker calls the same parsers again in `serve` before it writes, and a refusal there is `Failed` with the same sentence. The functions are pure and cheap, so running them on the UI task is `R-NF-3`-clean. | The store boundary must not trust the render side. One implementation means the two cannot disagree. |
| D248 | **Not changed, on purpose:** `htui-orch` (`select.rs`, `graph.rs`, `conformance.rs`); the probe's status mapping and `ProbeSource` (MOD-2 D45, D50, D51; the `Manual` doc line at `probe.rs:326` stays true, OQ-3); `agent.settings` beyond OQ-5's copy (OQ-4); deleting a row (retirement is `enabled = false`, `pg/mod.rs:377-378`); the caps banner (MOD-12); box capabilities (MOD-7); the cache mirror (`agent` is full-replaced every pass, so an edit reaches an offline box on its next refresh); `docs/**`, `HANDOFF.md`, `DECISIONS.md`, `REQUIREMENTS.md` (main thread). | Scope. |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A settings write module: serve, re-read, stale reply, request names | `crate::box_settings` | `crates/htui/src/box_settings.rs:74-131` |
| Routing a module's requests from `try_serve`; the names test | the box arm; `box_requests_are_named_as_box_settings_lists_them` | `crates/htui/src/store_worker.rs:1356-1361`, `:3313` |
| Minting an id in the worker, never in the section | `CreateWorkspace` | `crates/htui/src/hierarchy.rs:231-240` |
| A multi-field one-line form: `Field`, `Editor`, focus keys, `Enter`/`Esc` | Settings > Hierarchy | `crates/htui/src/ui/tabs/settings/hierarchy.rs:150-170`, `:779-822`, `:825-930`, `:1301-1360` |
| An editor's CAS token, stale and gone handling, "unchanged closes" | Settings > Boxes | `crates/htui/src/ui/tabs/settings/boxes.rs:120-135`, `:204-246`, `:368-416` |
| Shared sentences and parsers | `yes_or_no`, `CHANGED_ELSEWHERE`, `DELETED_ELSEWHERE`, `is_error` | `crates/htui/src/ui/tabs/settings/mod.rs:53-59`, `:105`, `:120`, `:67-69` |
| A narrow single-purpose `agent_box` writer across five stores | `set_agent_box_quota` | trait `crates/htui-core/src/store/traits.rs:376-423`; `mem.rs:1696`, `:5718`; `pg/write.rs:1243`; `writer.rs:376`; `htui-agent/src/conformance.rs:774`; `htui-agent/tests/recorder.rs:505` |
| A write-only conformance case, read-back in concrete-store tests | `set_agent_box_quota_updates_two_columns_or_not_found`; `set_agent_box_quota_leaves_probe_and_version_alone` | `crates/htui-core/src/store/conformance.rs:1693`; `crates/htui-core/src/store/mem.rs:7024` |
| A migration's column comment pinned verbatim | `MOD7_COLUMN_COMMENTS` | `crates/htui-store/tests/migrations.rs:347-380`, checked at `:424-470` |
| Refusing a registry action by name before any work | `begin_install` | `crates/htui/src/ui/tabs/settings/agents.rs:729-762` |
| A Postgres suite in `crates/htui` that skips without a DSN | `box_probe_pg.rs` | `crates/htui/tests/box_probe_pg.rs` (the `testkit::SKIP` pattern) |
| Section tests over `SectionBench` and a row helper | `section_over`, `registry_row` | `crates/htui/tests/settings.rs:1080`, `:981` |
| Promoting an already-compiled transitive crate | `zeroize`, `hmac` | root `Cargo.toml` `[workspace.dependencies]` comments |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-store/migrations/0008_agent_box_user_off.sql` | **new** | T0 | D242 column + comment |
| `crates/htui-store/tests/migrations.rs` | edit | T0 | applied list `[1..=8]` (`:86-93`); `Pending(7)` → `Pending(8)` at `:884`, `:983`, `:988`, `:1007`; `MOD23_COLUMN_COMMENTS` chained into the comment check (`:424-433`) |
| `crates/htui-store/tests/connect.rs` | edit | T0 | `Pending(7)`/"seven" pins at `:141-143`, `:157-159`, `:243-245` |
| `crates/htui-core/src/model/agent.rs` | edit | T0 | `AgentSummary.user_off` (`:190-195`) |
| `crates/htui-core/src/store/traits.rs` | edit | T0 | `set_agent_box_enabled`; `upsert_agent_box` doc gains the `user_off` rule; module doc line |
| `crates/htui-core/src/store/mem.rs` | edit | T0 | side set, both rules, `agent_summaries`, forward, unit tests |
| `crates/htui-core/src/store/conformance.rs` | edit | T0 | one case + `run_case` arm; `CASES` 96 → 97 |
| `crates/htui-core/tests/mem_store.rs` | edit | T0 | pin 96 → 97 (`:36-37`) and a MOD-23 clause in its message |
| `crates/htui-store/src/pg/write.rs` | edit | T0 | setter; `upsert_agent_box` `SET` term (`:1200-1226`) |
| `crates/htui-store/src/pg/read.rs` | edit | T0 | `agents()` selects `ab.user_off` (`:1285-1352`) |
| `crates/htui-store/src/cache/read.rs` | edit | T0 | `user_off: false` (`:1420-1434`) |
| `crates/htui-store/src/writer.rs` | edit | T0 | forward (beside `:376`) |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T0 | `EXPECTED_CASES` 96 → 97 (`:19`) |
| `crates/htui-store/tests/pg_criteria.rs` | edit | T0 | Postgres read-back cases (D242) |
| `crates/htui-store/.sqlx/` | regenerate | T0 | two edited statements, one new |
| `crates/htui-agent/src/conformance.rs` | edit | T0 | `UsageSpy` forward (beside `:774`) |
| `crates/htui-agent/tests/recorder.rs` | edit | T0 | `SpyStore` forward (beside `:505`) |
| `crates/htui/tests/settings.rs` | edit | T0, T3 | T0: `user_off: false` in the one `AgentSummary` literal (`:244-259`). T3: the section cases and the stale "a notice and the keys share it" comment (`:1565-1568`) |
| `Cargo.toml` | edit | T1 | `shell-words = "1.1"` in `[workspace.dependencies]` (OQ-2) |
| `crates/htui/Cargo.toml` | edit | T1 | `shell-words = { workspace = true }` |
| `Cargo.lock` | update | T1 | `htui` lists `shell-words` (no new package) |
| `crates/htui/src/agent_settings.rs` | **new** | T1, T2 | T1: `AgentDraft`, parsers, `merge_launch`, `valid_name`, unit tests. T2: `serve`, `AgentWrite`, `REQUEST_NAMES` |
| `crates/htui/src/lib.rs` | edit | T1 | `pub mod agent_settings;` |
| `crates/htui/src/store_worker.rs` | edit | T2 | three request variants, one reply variant, `name`, the `try_serve` arm, the names test |
| `crates/htui/src/agent_worker.rs` | edit | T2 | D243 and its unit test |
| `crates/htui/tests/agent_settings.rs` | **new** | T2 | MemStore and Offline serve cases |
| `crates/htui/tests/agent_settings_pg.rs` | **new** | T2 | Postgres serve cases |
| `crates/htui/src/ui/tabs/settings/agents.rs` | edit | T3 | the form, `n`/`e`/`t`, D232, D244, D245, `on_reply` arms |
| `crates/htui/tests/snapshots/settings__agents_{demo,empty,probed,quota,unknown_row}.snap`, `probe__agents_probed_missing.snap` | update | T3 | the two-line hint (D245) |
| `crates/htui/tests/snapshots/settings__agents_{create_form,edit_form,switched_off}.snap` | **new** | T3 | the pane and the cell |

**Not touched, on purpose:** `crates/htui-orch/**`; `crates/htui-agent/src/probe.rs` and
`launch.rs`; `crates/htui-store/cache_migrations/**`; `crates/htui-core/src/fixtures.rs`;
`crates/htui/tests/probe.rs` (only its snapshot moves); `docs/**`, `HANDOFF.md`, `DECISIONS.md`.

## Tasks

**Order.** **Wave 1:** T0 and T1 in parallel, each in its own worktree; merge T0, then T1, re-running
the gates of every crate each touched on the merged tree (`--test-threads=1`). **Wave 2:** T2 (needs
T0's setter and `user_off`, and T1's module). **Wave 3:** T3 (needs T2's request and reply variants).

| Task | Files (complete list) | Parallel |
|---|---|---|
| T0 | `crates/htui-store/migrations/0008_agent_box_user_off.sql` (new), `crates/htui-store/tests/migrations.rs`, `crates/htui-store/tests/connect.rs`, `crates/htui-core/src/model/agent.rs`, `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/pg/read.rs`, `crates/htui-store/src/cache/read.rs`, `crates/htui-store/src/writer.rs`, `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/tests/pg_criteria.rs`, `crates/htui-store/.sqlx/`, `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs`, `crates/htui/tests/settings.rs` (the literal only) | Wave 1, independent of T1 |
| T1 | `Cargo.toml`, `crates/htui/Cargo.toml`, `Cargo.lock`, `crates/htui/src/agent_settings.rs` (new), `crates/htui/src/lib.rs` | Wave 1, independent of T0 |
| T2 | `crates/htui/src/agent_settings.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/agent_worker.rs`, `crates/htui/tests/agent_settings.rs` (new), `crates/htui/tests/agent_settings_pg.rs` (new) | Wave 2, after T0 and T1 |
| T3 | `crates/htui/src/ui/tabs/settings/agents.rs`, `crates/htui/tests/settings.rs`, the six updated and three new snapshots listed above | Wave 3, after T2 |

**Independence of T0 and T1, justified by file sets and build coupling.** T0 ∩ T1 = ∅: T0 touches
`crates/htui` only in `tests/settings.rs`; T1 touches only `crates/htui/src/{agent_settings,lib}.rs`,
the two `Cargo.toml`s and `Cargo.lock`, none of which T0 touches. Build coupling has three conditions,
binding on the implementers:
1. **T1 constructs no `AgentSummary` and calls no `set_agent_box_enabled`.** T0 adds a field to the
   first and a trait method to `WriteStore`, so T1's code must compile against both base and merged
   `htui-core`. T1's unit tests use `Agent` rows only.
2. **T0 adds no `crates/htui/src` code.** Its `crates/htui` edit is the one test literal, so T1's new
   module cannot conflict.
3. **`.sqlx` and the store `CASES` pins move in T0 only.** T1–T3 add no `query!` and no store case.
   `Cargo.lock` moves in T1 only.

T2 and T3 are serial after Wave 1: T2 adds `StoreRequest`/`StoreReply` variants, whose two exhaustive
matches (`StoreRequest::name`, `store_worker.rs:749-851`; `try_serve`, `:1251`) are both in T2's
file. T3 cannot compile before them. T2 ∩ T3 = ∅.

Every implementer prompt carries: nothing branches on an agent's name (`R-AGT-5`; the
`extensibility.rs` sweep is the check); no `env` value, token or raw `launch` document in a log line,
a notice or a snapshot; **commit incrementally, staging your own paths only** (uncommitted subagent
work does not survive the session, and there is no stash on a shared tree); verify your gate with
`--test-threads=1` on the real tree after your merge.

### Task 0: the per-box switch in the store (D242)
- **Tests first.** The conformance case `set_agent_box_enabled_switches_one_row_or_refuses`,
  appended to `CASES` with its `run_case` arm. It checks that `false` on an absent pair lands, that
  `true` then `false` on an existing pair lands, and that an unknown agent and an unknown box are
  each `Constraint`. Pins move 96 → 97 (`mem_store.rs:36-37`, `pg_conformance.rs:19`). MemStore unit
  tests beside `set_agent_box_quota_leaves_probe_and_version_alone` (`mem.rs:7024`):
  - `a_switched_off_row_stays_off_under_an_upsert_that_says_enabled`
  - `switching_on_restores_the_probe_verdict`: `ready` → on, `unauthenticated` → still off
  - `the_switch_leaves_probe_version_path_probed_at_and_quota_alone`
  - `agents_reports_user_off_for_this_box_only`
  - `a_switch_on_an_unprobed_agent_inserts_a_bare_row`

  The same four read-backs go in `pg_criteria.rs` against Postgres. `migrations.rs`: the applied
  list is `[1, …, 8]` and `agent_box.user_off`'s comment is byte-exact. The first commit is red, with
  `todo!()` bodies in the new methods only.
- **Action.** The migration; the trait method with its doc (order: referents, then write); MemStore;
  `PgStore` (the setter and the one `SET` term); the `agents()` projection on both stores; the three
  forwards; `AgentSummary.user_off` with `#[serde(default)]` and its two constructors plus the cache
  `false`; the settings test literal. Then `cargo sqlx prepare` against a scratch database migrated
  through `0008` (Validation).
- **Validate.** `cargo test -p htui-core --all-features -- --test-threads=1`;
  `cargo test -p htui-store --all-features -- --test-threads=1` (sandbox env already sets
  `HTUI_TEST_DATABASE_URL`); `cargo test -p htui-agent --all-features -- --test-threads=1`;
  `cargo check --workspace --all-features --all-targets`; `cargo sqlx prepare --check`.

### Task 1: the draft and its rules (D233–D238, D247)
- **Tests first** (in-module `#[cfg(test)]`, as `ui/text_field.rs` does):
  - `valid_name` over the three seed names, and over empty, 65 chars, uppercase, a space, a
    leading `-` and a control character.
  - `parse_models` covers order kept, empties dropped and a repeat refused.
  - `parse_default` checks that a default must be in a non-empty list and is free with an empty one.
  - `parse_args`: the D236 round-trip table, and an unclosed quote refused.
  - `parse_transport` and `parse_billing` accept case and whitespace and name the valid values.
  - `merge_launch` keeps `env`, `discovery.install` and an unknown key byte for byte, replaces
    `command` and `args`, refuses a non-object `launch`, and refuses a result that does not parse as
    `AgentLaunch`.
  - `apply_draft` keeps `name`, `settings`, `created_at` and `updated_at`.
  - `draft_from_fields` returns the first refusal in field order.
- **Action.** The module with `pub struct AgentDraft { transport, command, args, models,
  default_model, billing, enabled }` (derives `Debug`, `Clone`, `PartialEq`; no `env`) and the pure
  functions above. Plus the dependency promotion with its one-line comment, and `pub mod
  agent_settings;`.
- **Validate.** `cargo test -p htui --all-features --lib agent_settings -- --test-threads=1`;
  `cargo clippy -p htui --all-features --all-targets -- -D warnings`; `cargo tree -p htui -i
  shell-words` shows 1.1.1 only.

### Task 2: the writes, served (D239–D241, D243, D246)
- **Tests first.** `crates/htui/tests/agent_settings.rs` over `Backend::memory(MemStore::demo())`:
  - `create_agent_lands_with_a_minted_id_and_answers_created`: the row is in `agents` with a blank
    `launch` plus `command`/`args`, and `settings` copied from `settings_from`.
  - `create_agent_with_a_taken_name_is_failed_with_the_store_sentence`
  - `edit_agent_applies_and_keeps_name_env_discovery_and_settings`
  - `edit_agent_with_a_spent_token_answers_stale_and_writes_nothing`
  - `edit_agent_on_an_unknown_id_answers_gone`
  - `edit_agent_with_an_invalid_draft_is_failed_by_field`
  - `set_agent_on_box_off_then_on_answers_switched_and_the_summary_says_so`
  - `offline_refuses_all_three_by_name_with_the_registry_sentence` (`Backend::Offline`, as
    `box_settings.rs:245-270` builds one)
  - `a_request_from_elsewhere_is_named_not_panicked`

  `crates/htui/tests/agent_settings_pg.rs` (skips without `HTUI_TEST_DATABASE_URL`, panics under
  `CI`) covers two things. An edit round-trips through a token read from `PgStore::agents` (MOD-40
  F-17's microseconds). And `set_agent_on_box(false)` survives an `upsert_agent_box` that says
  `enabled: true`. In `store_worker.rs`, `agent_requests_are_named_as_agent_settings_lists_them`. In
  `agent_worker.rs` tests, `a_probe_reply_keeps_a_switched_off_row_off` (D243) over `MemStore` with
  a scripted probe environment the existing probe tests use.
- **Action.** `serve`, `AgentWrite`, `REQUEST_NAMES`. `StoreRequest::CreateAgent { name: String,
  draft: AgentDraft, settings_from: Option<AgentId> }`, `EditAgent { agent_id: AgentId, expected:
  DateTime<Utc>, draft: AgentDraft }` and `SetAgentOnBox { agent_id: AgentId, enabled: bool }`, each
  documented. `StoreReply::AgentWritten`. The `name` arms, the or-ed `try_serve` arm and its comment
  (the counts in the neighbouring comments are not restated). D243's line.
- **Validate.** `cargo test -p htui --all-features -- --test-threads=1`;
  `cargo test -p htui --test agent_settings_pg --all-features -- --test-threads=1` (it must run, not
  print `SKIP`); `cargo clippy -p htui --all-features --all-targets -- -D warnings`.

### Task 3: the pane (D230–D232, D244, D245)
- **Tests first**, in `crates/htui/tests/settings.rs` over `SectionBench` and the existing
  `section_over` / `registry_row` helpers:
  - `n_opens_the_create_form_and_captures_input`: `l` and `h` are typed, not cycled; `q` is a
    letter; `ctrl-c` passes.
  - `enter_on_the_create_form_sends_create_agent_with_the_parsed_draft_and_the_source_row`
  - `a_bad_field_is_refused_locally_by_name_and_sends_nothing`, one case per rule.
  - `e_prefills_the_highlighted_row_without_a_name_field_and_sends_its_token`
  - `an_unchanged_edit_closes_without_a_write`
  - `agent_written_edited_closes_the_form_and_says_re_probe_when_launch_changed`
  - `agent_written_created_selects_the_new_row`
  - `agent_written_stale_keeps_the_text_and_takes_the_new_token`
  - `agent_written_gone_closes_with_deleted_elsewhere`
  - `a_refused_write_keeps_the_form_open_with_the_sentence`
  - `an_agents_reply_does_not_close_the_form_or_move_its_token`
  - `t_sends_set_agent_on_box_with_the_inverse_of_the_switch`
  - `n_e_and_t_are_refused_while_a_probe_an_install_or_a_login_runs`
  - `r_i_and_a_are_refused_while_a_registry_write_is_in_flight`
  - `a_switched_off_row_reads_switched_off`
  - `a_bare_row_reads_not_probed`
  - `the_idle_keys_and_the_quota_note_are_two_lines`

  New snapshots `settings__agents_create_form` and `settings__agents_edit_form` (the `agy` row: an
  11-model list clipped with `…`, and the full 21-char default) and `settings__agents_switched_off`.
  The six existing snapshots are re-accepted with `cargo insta review`; only the hint lines move.
- **Action.** D230–D232, D244, D245 in `agents.rs`: `Mode { Browse, Editing(Editor) }` beside the
  install and login states, `busy`, the `AgentWritten` and `Failed` arms, and the render split. Also
  reword the `:1565-1568` comment in `settings.rs`.
- **Validate.** `cargo test -p htui --all-features -- --test-threads=1`; `ls
  crates/htui/tests/snapshots | wc -l` (110); `cargo clippy -p htui --all-features --all-targets --
  -D warnings`.

## Test plan

TDD per repo convention: every task's first commit is its failing tests. **The first red tests** are
T0's `set_agent_box_enabled_switches_one_row_or_refuses` and T1's `parse_args` round-trip table, one
per Wave 1 lane.

**The item's asks, mapped.** Create a manual row: T2 `create_agent_lands…`, T3 `enter_on_the_create_form…`.
Edit transport, command, args, models, default and billing: T1's parsers, T2 `edit_agent_applies…`
and T3 `e_prefills…`. The agent row's `enabled`: the form's `enabled (y/n)` field through the same
path. The per-box `agent_box.enabled`: T0's store cases, T2 `set_agent_on_box…` plus the Postgres
survival case, and T3's `t` case. Off the UI task: every write is served in the store loop, and the
section holds no store handle (reviewer check). Server-only: T2
`offline_refuses_all_three…`. The CAS: T2's stale cases on both stores.

**Count pins that move:**

| Pin | Now | After | Where |
|---|---|---|---|
| Store `CASES` | 96 | 97 (T0) | `crates/htui-core/src/store/conformance.rs:44`; `crates/htui-core/tests/mem_store.rs:36-37`; `crates/htui-store/tests/pg_conformance.rs:19`; doc pin `HANDOFF.md:42` (main thread) |
| Migrations | `0001`..`0007` | `0001`..`0008`; next `0009` (T0) | `crates/htui-store/migrations/`; `tests/migrations.rs`, `tests/connect.rs`; `HANDOFF.md:36-39` (main thread) |
| Pinned commented columns | 34 | 35 (T0) | `crates/htui-store/tests/migrations.rs` |
| `.sqlx` files | 288 | 289 (T0: one new statement; the two edited statements replace their files — the gate records the real number) | `crates/htui-store/.sqlx/` |
| `StoreRequest` / `StoreReply` | 85 / 47 | 88 / 48 (T2) | `crates/htui/src/store_worker.rs` |
| `agent_settings::REQUEST_NAMES` | — | 3 (T2) | `crates/htui/src/agent_settings.rs` |
| `crates/htui/tests/snapshots` | 107 | 110 (T3 adds three, updates six) | — |
| `htui-orch` `CASES`, `GraphSource`, `MIRRORED_TABLES`, Settings sections | 73, 7, 21, 7 | unchanged | — |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-1** — Migration `0008` collides with another in-flight item that also claims `0008` | Low | HANDOFF says next is `0008`; whichever lands second renumbers, and `tests/migrations.rs` + `connect.rs` catch a gap |
| **R-2** — A probe, install or login writes the same `agent_box` row while `t` flips it | Low | D242's two statements are each atomic and compose (`enabled AND NOT user_off`); D232 refuses overlap in this section; another process is covered by the SQL |
| **R-3** — An edit lands while a probe of the same row is mid-flight on another box, recording the old recipe | Low | `needs_reprobe` re-probes on `agent.updated_at > probed_at` (D246) |
| **R-4** — The merged `launch` drops `discovery`/`install`/`env` | Low | D235's merge and T1's byte-for-byte tests |
| **R-5** — `shell-words` treats `\` as an escape, so a pasted Windows path loses its backslashes | Medium on Windows | The prefill quotes via `join`; the round-trip test pins a backslash; the notice for a parse is the crate's sentence; OQ-2's alternative is JSON |
| **R-6** — A renamed or deleted seeded row resurrects through the name-keyed top-up | Retired | D233: no rename, no delete |
| **R-7** — A row the switch inserted (never probed) and later switched on counts in `rung_three` as "enabled on this box" | Low | Accepted as the user's explicit statement (D242); `select::not_ready` still treats its absent probe as unknown |
| **R-8** — An `AgentSummary` field breaks a caller outside the listed files | Low | Four constructors in the tree (claim 22); `#[serde(default)]` for any serialised copy |
| **R-9** — A new `cli` row still cannot chat (no `settings.cli`) when copied from an `acp` row | Medium | OQ-5's header names the source row; the create notice names the adapter id when the row is `cli` and `settings.cli` is absent |
| **R-10** — The htui suite is scheduling-dependent (process-wide keyring fake) | Medium | Every gate runs `--test-threads=1`; the main thread re-runs gates on the merged tree |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
# HR sandbox: HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres is already set
cargo test --workspace --all-features -- --test-threads=1
# sqlx: a scratch database migrated through 0008 (docs/hr-sandbox.md "Changing SQL queries in a run")
psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx;"      # once
cd crates/htui-store
export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
cargo sqlx migrate run --source migrations
cargo sqlx prepare -- --all-targets --all-features    # T0 only
cargo sqlx prepare --check                            # every gate
cd -
ls crates/htui-store/.sqlx | wc -l                    # 289 (or as T0 recorded)
ls crates/htui/tests/snapshots | wc -l                # 110
cargo doc --workspace --no-deps --keep-going          # exactly the six baseline errors (HANDOFF.md:49-53)
git diff --stat b5e481f -- crates/htui-orch crates/htui-store/cache_migrations \
  crates/htui-agent/src/probe.rs crates/htui-agent/src/launch.rs   # empty
```

`--test-threads=1` is not optional. Before believing a Postgres failure, run `df -h /` and re-run the
case alone.

**Live check (optional, after Wave 3).** Launch `htui` against the dev database. In Settings >
Agents, `n` creates a row with a literal `command`; `r` probes it; `e` edits its models and default;
`t` switches it off for this box and `r` leaves it `switched off`; `t` on returns the probe's verdict.

## Acceptance

- [ ] A new `agent` row can be created from Settings > Agents with name, transport, command, args,
      models, default model, billing and enabled; a taken name is refused by the store's sentence.
- [ ] An existing row's transport, command, args, models, default model, billing and enabled can be
      edited as a compare-and-set on `updated_at`; `name`, `settings` and every other `launch` key are
      kept; a spent token reloads and keeps the typed text.
- [ ] This box's `agent_box.enabled` can be switched off and on, and no probe, re-probe or box
      probe turns a switched-off row back on (OQ-1 as answered).
- [ ] Every write is served on the store worker; the section holds no store handle; offline, every
      write is refused with `REGISTRY_ON_SERVER_ONLY`.
- [ ] The table keeps eight columns; the form lives in the pane below it; the frame fits 98 columns.
- [ ] Nothing added branches on an agent's name; `extensibility.rs` stays green.
- [ ] Store `CASES` 97 in all three places; `.sqlx` regenerated and `prepare --check` clean; the
      workspace gate above is green.

## Where the HANDOFF or tree disagree

1. **"Needs the same text-input widget MOD-22 needs and the Settings tab does not have yet"** is
   stale. `crates/htui/src/ui/text_field.rs` (`TextField`, `FieldOutcome`) and `ui/text_area.rs`
   exist and are re-exported at `crates/htui/src/ui/mod.rs:14-15`. Settings > Boxes uses both
   (`boxes.rs:29`) and Settings > Hierarchy builds its forms on `TextField`
   (`settings/hierarchy.rs:150-157`). MOD-23 builds on them (D231).
2. **The column-budget paragraph describes MOD-2 D76's packing, which D89 reversed.** The HANDOFF
   says `name` sits "at `amp-acp`" and `on this box` "at `unauthenticated`". In the tree `name` is
   `Constraint::Fill(1)` (`agents.rs:1081`), and `on this box` is `Length(13)`, the donor that clips
   `unauthenticated` to `unauthenticat` (`agents.rs:1063-1069`). The conclusion stands: a pane, not
   a ninth column.
3. **"A guard that today protects rows no UI can create"** refers to `agent_box.probe.source` (MOD-2
   D45/D51), a per-box row. MOD-23's "manual agent row" is an `agent` row, which that guard does not
   cover. Under OQ-3's recommendation it stays a guard with no UI, and `ProbeSource::Manual`'s doc
   line ("the editor is a later milestone's", `probe.rs:326`) stays true.
4. **ANA-4 §4.6's manual-entry recipe would not reach a spawn.** Writing `probe.resolved` by hand
   (`docs/ANA-4.md:796-798`) is ignored: `recorded_launch` answers `None` unless `source == probe`
   (`probe.rs:1174-1177`). The main thread records this against ANA-4 (OQ-3).
5. **`R-AGT-4`'s "enabled per box" and `R-AGT-6`'s "mark enabled on that box" name one column with
   two writers** (`REQUIREMENTS.md:183-190`; `probe.rs:1530`). OQ-1 reconciles them; its answer is
   an ANA-4 §4.6 / MOD-2 D50 note for the main thread.
6. **ANA-21 item 8** (`docs/ANA-21.md:552`) assigns the `settings.weights` UI to MOD-23; OQ-4
   recommends leaving it to MOD-36.
7. **The HANDOFF says the section binds `o` open and `x` cancel**, which is true only while an
   install or a login is in flight (`agents.rs:1148-1177`). Informational.
8. **Decision numbering conventions differ in the tree.** Standalone plans such as MOD-31 restart
   at D1 (`.claude/plans/mod-31-preview-blocks-install.plan.md:62`); the most recent plans continue
   a global sequence (MOD-56 D217–D221, MOD-50 D222–D229). This plan continues the global sequence
   at D230, as instructed.

---

## Claims to verify

Every checkable fact this plan asserts, for the fact-check pass. Line numbers are at `b5e481f`.

1. `HEAD` is `b5e481f` on `hr/MOD-23`; `graphify-out/` does not exist; the highest decision number in `.claude/plans/*.plan.md` is D229 (`mod-50-concepts-index-requirements.plan.md`).
2. `crates/htui/src/ui/text_field.rs` defines `pub enum FieldOutcome` (`:31`) and `pub struct TextField` (`:45`) with `with_text` (`:101`), `on_key` (`:119`), `text` (`:181`) and `line` (`:249`); `crates/htui/src/ui/mod.rs:14-15` re-exports `TextArea`, `FieldOutcome` and `TextField`.
3. `settings/boxes.rs:29` imports `FieldOutcome, TextArea, TextField`; `settings/hierarchy.rs:150-157` defines `Field` with a `TextField` input.
4. `SettingsSection::captures_input` exists with a default `false` (`settings/mod.rs:149-151`), and `SettingsTab::on_key` delegates every key to a capturing section before section cycling (`:323-331`).
5. `yes_or_no` is at `settings/mod.rs:53`, `CHANGED_ELSEWHERE` at `:105`, `DELETED_ELSEWHERE` at `:120`, `is_error` at `:67`.
6. `agents.rs`: `HINT_IDLE` at `:85` is 49 chars; `QUOTA_NOTE` at `:98` is 43 chars; `hint()` at `:956-978` joins keys and note with ` · ` on one line; the idle line is 95 chars.
7. `agents.rs:1078-1089`: the table's constraints are `Fill(1), Length(9), Length(12), Length(6), Length(21), Length(7), Length(13), Length(13)`.
8. `agents.rs:1117-1207` binds `j`, `k`, `i`, `a`, `o` (flow only), `x` (flow only), `Esc` (manual steps only) and `r`; `n`, `e` and `t` are unbound in the section.
9. The global keymap binds `q` (`keymap.rs:209`), `?` (`:235`), `ctrl-c` (`:248`) and `-` (`:353`), and no `n`, `e` or `t`.
10. `agents.rs` `render` (`:1271-1294`) lays out `[Min(3), Length(pane), Length(1)]`; `pane` is at `:927-948`; `on_box_cell` at `:866-895` renders `(off)` for a disabled row.
11. `begin_install` (`agents.rs:729-762`) refuses by sentence before sending a request, including while a login or a probe is in flight.
12. `WriteStore::upsert_agent(&self, agent: &Agent, expected: Option<DateTime<Utc>>) -> Result<CasOutcome<Agent>>` is at `traits.rs:351-355` with the CAS doc at `:324-350`; its non-test callers are `fixtures::edit_agent` (`fixtures.rs:2021-2029`) and the store forwards only.
13. `WriteStore::upsert_agent_box` is at `traits.rs:374` and `set_agent_box_quota` at `:417-423`; `set_agent_box_enabled` and `user_off` appear nowhere in `crates/`.
14. `WriteStore` has exactly five implementors: `MemStore` (`mem.rs:5666`), `PgStore` (`pg/write.rs:684`), `Writer` (`writer.rs:307`), `UsageSpy` (`htui-agent/src/conformance.rs:711`), `SpyStore` (`htui-agent/tests/recorder.rs:428`).
15. `PgStore::upsert_agent_box` (`pg/write.rs:1200-1226`) sets `enabled = EXCLUDED.enabled` on conflict (`:1206`) and never names `quota`/`quota_at`.
16. `agent_box_row` sets `enabled: snapshot.status == ProbeStatus::Ready` (`probe.rs:1530`) and uses `snapshot.tools.get(&agent.name)` as the version fallback (`:1535`).
17. `probe_agent` returns `Kept` only when nothing resolved and the stored row is `source: manual` (`probe.rs:1336-1357`); `ProbeSource::Manual`'s doc says "the editor is a later milestone's" (`:326`).
18. `ProbeSnapshot::recorded_launch` returns `None` when `source != Probe` (`probe.rs:1174-1177`).
19. `probe_agents_on` (`agent_worker.rs:2147-2180`) writes each `ProbeOutcome::Row` with `upsert_agent_box` (`:2173`) and returns the in-memory row as `on_box`, without a re-read.
20. `needs_reprobe` (`agent_worker.rs:2350-2359`) answers `true` when `agent.updated_at > probed_at`.
21. `select::not_ready` (`crates/htui-orch/src/select.rs:190-201`) skips `agent.enabled = false`, `agent_box.enabled = false`, and a parsed probe whose status is not `ready`; an absent row or probe does not skip.
22. `rung_three` (`crates/htui-orch/src/graph.rs:792-816`) counts rows with `row.enabled` and `agent.enabled`, reads no probe status, and falls back to `models.first()`.
23. `AgentSummary` is `{ agent, on_box }` (`crates/htui-core/src/model/agent.rs:190-195`) and is constructed at exactly four sites: `pg/read.rs:1349`, `mem.rs:1481`, `cache/read.rs:1420`, `crates/htui/tests/settings.rs:244`.
24. `AgentBox` literals appear 65 times across 24 files (why OQ-1 B adds no `AgentBox` field).
25. `MemStore`'s `State` holds `agent_boxes: HashMap<(AgentId, BoxId), AgentBox>` (`mem.rs:179`); `State::upsert_agent_box` is at `:1656-1690`; `agent_summaries` at `:1477-1489`.
26. `PgStore::agents` (`pg/read.rs:1285-1352`) is one `LEFT JOIN agent_box … AND ab.box_id = $1` query; `CacheStore::agents` (`cache/read.rs:1408`) sets `on_box: None` (`:1434`).
27. `0001_init.sql:94-106` defines `agent` (`name TEXT NOT NULL UNIQUE`, `transport` and `billing` CHECKs) and `:112-123` defines `agent_box` with no `user_off`; `0002_agent_probe.sql:25` adds `probe JSONB`; `cache_migrations/0002_agent_mirror.sql` mirrors `agent` and not `agent_box`.
28. The migrations are `0001`..`0007`; `HANDOFF.md:36-39` says the next is `0008`.
29. `tests/migrations.rs` pins the applied list `[1..7]` (`:86-93`) and `Pending(7)` / `MigrationsPending(7)` at `:884`, `:983`, `:988`, `:1007`; `tests/connect.rs` pins seven at `:141-143`, `:157-159`, `:243-245`.
30. `tests/migrations.rs` checks column comments from `ANA_COLUMN_COMMENTS`, `MOD7_COLUMN_COMMENTS` and `MOD9_COLUMN_COMMENTS` chained at `:424-433`; `HANDOFF.md` records 34 pinned commented columns.
31. The store `CASES` has 96 entries (`conformance.rs:44`), pinned at `mem_store.rs:36-37` and `pg_conformance.rs:19` (`EXPECTED_CASES`).
32. `crates/htui-store/.sqlx` holds 288 files; `crates/htui/tests/snapshots` holds 107.
33. `StoreRequest` has 85 variants (`store_worker.rs:101-747`) and `StoreReply` 47 (`:855-1068`), matching `HANDOFF.md:43`.
34. `StoreRequest::name` (`store_worker.rs:749-851`) and `try_serve` (`:1251`) are the only exhaustive matches over `StoreRequest`; no exhaustive match over `StoreReply` exists (compile probe).
35. `try_serve` routes `Boxes | EditBox` to `box_settings::serve` at `:1356-1361`; `box_requests_are_named_as_box_settings_lists_them` is at `:3313`.
36. `box_settings::serve` (`box_settings.rs:82-121`) answers `Unreachable(DATABASE_UNREACHABLE)` without a writer, re-reads after a write, and maps `Stale` and `NotFound { entity: "box" }` to `BoxesStale`; `REQUEST_NAMES` is at `:127`.
37. `REGISTRY_ON_SERVER_ONLY` is `"the agent registry is written on the server only"` at `writer.rs:76`, re-exported at `htui-store/src/lib.rs:41`; `recording_writer` answers it at `agent_worker.rs:2521-2526`.
38. `Backend::writer()` answers `None` for `Offline` (`backend.rs:153-159`); `Backend::agents()` answers on all three arms (`:299-305`).
39. `hierarchy::serve` mints `WorkspaceId::new()` for `CreateWorkspace` (`crates/htui/src/hierarchy.rs:231-240`).
40. `PgStore::seed_if_empty_as` inserts `seed_rows` with `ON CONFLICT (name) DO NOTHING` (`pg/mod.rs:445-450`), and its doc says retiring an agent is `enabled = false`, not deletion (`:377-378`).
41. `AgentLaunch` is `{ command: String, args: Vec<String> (default), env: BTreeMap (default), discovery: Option<Discovery> }` (`launch.rs:75-88`); `install` lives under `discovery` (`declares_install`, `:153-158`).
42. `adapter_id_from` (`registry.rs:137-144`) answers `acp`, `cli/<stream>`, or bare `cli` when `settings.cli` is absent.
43. `str_enum!` implements `FromStr` (`crates/htui-core/src/model/mod.rs:64`); `Transport` is `acp`/`cli` and `Billing` is `subscription`/`per_token` (`model/agent.rs:9-27`).
44. `shell-words` 1.1.1 is in `Cargo.lock` (`:5934-5936`) as a dependency of `agent-client-protocol` (`:195`), and no crate in the workspace declares it.
45. The idle-hint test reads the last rendered line (`crates/htui/tests/settings.rs:783-793`); `SECTION_WIDE` is 100 (`:47`) and `SECTION_BORDERED` 98 (`:54`).
46. Exactly six snapshots contain the agents hint: `settings__agents_{demo,empty,probed,quota,unknown_row}.snap` and `probe__agents_probed_missing.snap`.
47. The keys line `j/k select · n new · e edit · t this box · r probe · i install · a authenticate` is 79 chars and `switched off` is 12 chars.
48. `docs/ANA-4.md:796-798` describes manual entries as writing `agent_box.path` and `probe.resolved` and setting `probe.status = "ready"`.
49. `docs/ANA-21.md:552` assigns surfacing `settings.weights` to MOD-23's editor; `.claude/prds/mod-40-multi-writer-hardening.prd.md:103` names MOD-23 the first production caller of the C6 CAS.
50. `docs/REQUIREMENTS.md` defines `R-AGT-4` (`:183-186`), `R-AGT-5` (`:187-188`), `R-AGT-6` (`:189-190`), `R-TUI-8` (`:330-334`) and `R-NF-3` (`:359-360`).
51. `HANDOFF.md:302-313` (MOD-59) recommends that a write's reply identify itself.
52. `extensibility.rs`'s `ZETA` row carries `env` with a `${zeta_token}` placeholder (`crates/htui-agent/tests/extensibility.rs:30-49`).
53. T0 ∩ T1 = ∅ and T2 ∩ T3 = ∅ by the file lists above; T1 compiles against base `htui-core` (it names no `AgentSummary` constructor and no `set_agent_box_enabled`).

---

## Verified claims

Filled by the fact-check pass.

| Claim | Verdict | Evidence |
|---|---|---|
