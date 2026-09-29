# Blueprint: MOD-23, agent registry editing in the Settings agents section

**Status**: PROPOSED 2026-09-29 by the code-architect, from the plan confirmed the same day
(OQ-1…OQ-5 on their recommended answers). Findings F-1…F-24 (§0.2) and decisions D249–D262 (§8) are
this blueprint's. **Blocker** means the plan read literally fails at run time, fails its own named
test or fails the gate. **Major** means a named test, pin, file list or signature is wrong or
missing, or a design consequence the plan did not state. **Minor** is a citation, wording, placement
or a hazard with a cheap guard. The Fix column is what the implementer builds. Where §0.2 amends the
plan, this file wins.

**Plan**: `.claude/plans/mod-23-agent-registry-editing.plan.md` (CONFIRMED 2026-09-29,
fact-checked: 53 claims). Its D230–D248, its "Verified claims" table and every amendment are binding
and are not reopened here, except where F-1 and F-8 show that the plan's own text cannot do what it
says.

**Verified at**: HEAD `0ddb232` on `hr/MOD-23`. `git diff --stat b5e481f HEAD -- crates Cargo.toml
Cargo.lock` is empty, so the plan's line numbers (taken at `b5e481f`) are HEAD's. **Line numbers
are pre-edit.** Once a task makes its first commit, a citation into a file that task edits has moved.
Counts at HEAD: `crates/htui-store/.sqlx/` **288**, `crates/htui/tests/snapshots/` **107**,
migrations `0001`..`0007`. `df -h /`: **100 GB free (78 % used)**.

**Tooling**: the Gortex graph does not track this checkout. Its hook claims files are indexed, but the
index is another checkout's, so every fact here was read with `sed`/`grep`/Read on
`/home/mluigi/projects/htui`. `graphify-out/` does not exist. Probes ran on the sandbox Postgres 16
(`localhost:5439`, trust auth). The scratch database `mod23_probe` was migrated `0001`..`0007` with
`psql -f`, then given `user_off`, and was dropped afterwards (§0.3).

**Coupling verdict.** The plan's waves stand: **T0 ∥ T1, then T2, then T3**, with T0 merged before
T1. No hidden coupling breaks T0/T1 independence (F-21 lists what was checked). Every file this
blueprint adds work to is already in the owning task's list, and no file moves between tasks.

**Scope at a glance**:
- **Order**: Wave 1 T0 (worktree A) ∥ T1 (worktree B), merged T0 then T1, re-running each touched
  crate's gate on the merged tree. Wave 2 T2 from the merged tree. Wave 3 T3 from T2.
- **One migration**: `0008_agent_box_user_off.sql` (one column, one comment). The next migration is
  `0009`. `TABLES` stays 39; pinned commented columns go 34 → **35**.
- **`WriteStore` +1** (`set_agent_box_enabled`) across five implementors. Store `CASES` 96 → **97**,
  `READ_CASES` unchanged.
- **`.sqlx`** 288 → **289**: one new statement, and two edited statements replace their files.
  `git status --porcelain crates/htui-store/.sqlx` shows **2 ` D` and 3 `??`** (F-5).
- **`StoreRequest` / `StoreReply`** 85 / 47 → **88 / 48** (T2). `agent_settings::REQUEST_NAMES` = 3.
- **Snapshots** 107 → **110** (T3 adds three and updates six).
- **Dependency**: `shell-words` 1.1.1 is promoted. It is already locked, so the lock gains no package.

**House style (carried)**: `unsafe_code = "forbid"`; `missing_docs` on lib roots;
`missing_debug_implementations` and `unused_qualifications` warn; clippy `all` at `-D warnings`
(pedantic off); rustdoc denies broken and **private** intra-doc links, so a `pub` item's doc must not
link to a private fn, and a doc written before its target exists uses plain backticks.
`rustfmt.toml` `max_width = 100`. Every new `pub` item has a doc comment and `Debug`. Red tests come
first: a `todo!()` body goes **only** where no existing path calls it (MOD-7 blueprint H-6). Every
commit compiles, and no test is loosened. A moved pin names its reason in the assertion message.
Implementers stage their own paths only: never `git add -A`, never `stash`, never `--amend`. No
`env` value, token or raw `launch` document goes into a log line, notice, snapshot or `Debug` of a
reply. Nothing branches on an agent's name (`R-AGT-5`; `crates/htui-agent/tests/extensibility.rs`
is the sweep).

---

## 0. Environment, findings, probes

### 0.1 Environment and gates (this sandbox)

```bash
# Already set by the sandbox (docs/hr-sandbox.md "Databases"); nothing to export:
#   HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres   USERNAME=htui-ci
pg_isready -h localhost -p 5439          # accepting connections at blueprint time
df -h /                                  # before each wave: two worktrees are two target/ dirs (~10 GB each)
sqlx --version                           # sqlx-cli 0.9.0, already installed (/opt/rust/cargo/bin)
```

`.cargo/config.toml` sets `SQLX_OFFLINE = "true"`, so build, clippy and doc never need a server.

**Regenerating `.sqlx`** (T0 only). This is the recipe from `docs/hr-sandbox.md` "Changing SQL
queries in a run", made explicit for a **new migration**. The scratch database is recreated each
time, because a scratch database migrated before `0008` existed lacks `user_off`, and the new query
then fails to prepare with an error that looks like a broken crate (project memory):

```bash
psql -h localhost -p 5439 -U postgres -c 'DROP DATABASE IF EXISTS htui_sqlx' -c 'CREATE DATABASE htui_sqlx'
cd /home/mluigi/projects/htui/crates/htui-store          # the worktree's own path in a worktree
DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx migrate run --source migrations   # 0001..0008
DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare -- --all-targets --all-features
DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features
ls .sqlx | wc -l                                   # 289
git status --porcelain .sqlx                       # exactly two " D" and three "??" lines (F-5)
```

The flags `--all-targets --all-features` are mandatory. Without them, `prepare` deletes the
feature-gated and test-only query files (it took the tree from 268 to 233 on 2026-09-27). Recover
with `git checkout -- .sqlx && git clean -fq .sqlx` and re-run with the flags. Never run
`prepare --workspace` from the root. `warning: potentially unused queries found in .sqlx` is
expected from `--check`.

**Per-crate gates** (every one `--test-threads=1`: the htui suite's green depends on scheduling,
because the keyring fake is process-wide):

| Crate | Command |
|---|---|
| htui-core | `cargo test -p htui-core --all-features -- --test-threads=1` |
| htui-store | `cargo test -p htui-store --all-features -- --test-threads=1` (Postgres suites run, not `SKIP`) |
| htui-agent | `cargo test -p htui-agent --all-features -- --test-threads=1` |
| htui | `cargo test -p htui --all-features -- --test-threads=1` |
| all | `cargo fmt --all -- --check`; `cargo check --workspace --all-features --all-targets`; `cargo clippy --workspace --all-features --all-targets -- -D warnings` |

Before believing a Postgres failure, run `df -h /` and re-run the case alone. The dev Postgres
crash-loops under disk pressure and can report "healthy" while it is still recovering.

### 0.2 Findings the plan and its fact-check missed

| # | Severity | Task | Plan says | Tree / probe | Fix |
|---|---|---|---|---|---|
| **F-1** | **Blocker** | T0 | D242: the setter's conflict arm is `enabled = $3 AND (agent_box.probe IS NULL OR agent_box.probe->>'status' = 'ready')`. | Probed (§0.3). On a row whose `probe` is a document **without** `status` (`{"source":"probe"}`), or the JSON `null`, `->>'status'` is SQL `NULL` and `true AND NULL` is `NULL`. Switching on then fails with **23502 `null value in column "enabled"`**, and `map_sqlx` turns that into `Constraint`. Switching **off** is safe (`false AND NULL` is `false`). | **D249**: the arm is `enabled = $3 AND (agent_box.probe IS NULL OR COALESCE(agent_box.probe->>'status' = 'ready', false))` (§2.4, probed green). MemStore mirrors it exactly: `None` → `true`, `Some(doc)` → `doc.get("status").and_then(Value::as_str) == Some("ready")`, which is `false` for `Some(Value::Null)` and for a document without `status`. The conformance case pins a status-less probe that switches on to `enabled: false`, not to an error (§2.6 step 5). |
| **F-2** | Major | T0 | D242: "MemStore keeps a `HashSet<(AgentId, BoxId)>` beside `agent_boxes` (`mem.rs:179`)". | `State` is built in one literal (`mem.rs:296`, `agent_boxes: HashMap::new()`): a new field that is not initialised there does not compile. `State::upsert_agent_box` (`:1656-1690`) refuses an unknown agent, then an unknown box, each with its own sentence; the setter owes the same two checks in the same order, as Postgres's FK does. The insert branch of `upsert_agent_box` keeps `row.updated_at`, but an update sets `now`, because the trigger does. | §2.3 spells out the field, its initialiser, both rules and the setter. The setter inserts `AgentBox { enabled, version: None, path: None, probed_at: None, quota: None, quota_at: None, updated_at: now, probe: None }`. On an existing row it writes `enabled` and `updated_at = now` and nothing else. |
| **F-3** | Major | T0 | Files: `tests/migrations.rs` "applied list `[1..=8]` (`:86-93`); `Pending(7)` → `Pending(8)` at `:884`, `:983`, `:988`, `:1007`; `MOD23_COLUMN_COMMENTS` chained into the comment check (`:424-433`)". | The three constants are chained in **three** places: the value loop (`:424-428`), the `ANY($1)` table list (`:482-485`) and the expected pair list (`:495-498`). "thirty-four" is hard-coded in the comment at `:469-471` and in the assertion message at `:503`. The `TABLES` count (39) does not move. `:885`'s message says "seven". `connect.rs:142-143`, `:158-159` and `:244-245` say "seven … MOD-9 milestone 2's 0007_skill_attachments.sql". | §2.8 lists every edit. The two counts become "thirty-five" and every "seven" becomes "eight", each naming `0008_agent_box_user_off.sql`. |
| **F-4** | Major | T0 | T0: a new conformance case whose doc delegates the read-back to MemStore and Postgres tests. | `store::conformance::every_cross_referenced_test_name_exists` (`conformance.rs:12547`) scans every backticked span of `conformance.rs`. A `` `mem.rs::<name>` `` or `` `pg_criteria.rs::<name>` `` span must name a fn that exists **in that commit**. A bare snake_case span with four or more underscores must be a fn in `conformance.rs` or `mem.rs`. A `` `<file>::<name>` `` span without `.rs` panics. | The new case's doc cites its read-backs as `` `mem.rs::a_switched_off_row_stays_off_under_an_upsert_that_says_enabled` `` and `` `pg_criteria.rs::a_switched_off_row_survives_a_re_probe_on_postgres` ``, and commit (a) defines both fns (red bodies are fine). `set_agent_box_enabled` and `upsert_agent_box` have three and two underscores, so they are free to backtick. |
| **F-5** | Major | T0 | ".sqlx: two edited statements, one new … 288 → 289"; Validation runs `cargo sqlx prepare --check` in "every gate". | `pg_criteria.rs` uses `sqlx::query!` in places (`:51`, `:84`, …), and each macro there adds a `.sqlx` file. The quota read-back case reads back with the **runtime** `sqlx::query_scalar(…)` (`:1777-1783`), which adds none. `prepare --check` needs a scratch database migrated **through `0008`**. The sandbox database `postgres` is the test DSN, not a migrated one. | Every new Postgres read-back in `pg_criteria.rs` and `agent_settings_pg.rs` uses runtime `sqlx::query*` or the inherent `PgStore::agents()`, never a macro. The only macro changes are the three statements in `pg/write.rs` and `pg/read.rs`. The expected porcelain is exactly 2 ` D` + 3 `??` (the old hashes of `upsert_agent_box` and `agents()` deleted; their new hashes plus the setter added). Only T0 regenerates. T1–T3 run `prepare --check` only if they touched `htui-store`, which they do not. |
| **F-6** | Minor | T0 | Extra finding: "the `set_agent_box_quota` doc (`traits.rs` ~:397-403) says 'six implementations'". | At `traits.rs:397-398`. More doc drift that T0's change makes false: the module doc's MOD-2 paragraph (`traits.rs:8-17`: "Milestone 7 reopens it once, for a seventh"); `upsert_agent_box`'s doc (`traits.rs:357-374`, `pg/write.rs:1178-1195`, `mem.rs:1648-1655`); `AgentBox.enabled`'s doc (`model/agent.rs:70-71`, "`agent_box.enabled`." only). | T0 rewords: "five implementations"; a MOD-23 sentence in the module doc; the `user_off` rule in all three `upsert_agent_box` docs; `AgentBox.enabled` = "the probe's verdict (`ready`), vetoed by the per-box switch (`AgentSummary::user_off`, MOD-23 D242)". |
| **F-7** | Minor | T2 | D241: offline, all three answer `Unreachable(REGISTRY_ON_SERVER_ONLY)`. | `store_worker::serve` renders an `Err` through `failed()` → `err.to_string()`, and `StoreError::Unreachable`'s `Display` is `store unreachable: {0}` (`htui-core/src/store/error.rs:42-43`). The message is therefore `store unreachable: the agent registry is written on the server only`. An `Unreachable` from `try_serve` also makes `spawn` call `go_offline`, which is a no-op on an `Offline` backend (`went_offline()` answers `false`, `store_worker.rs:2151-2165`). That is the `recording_writer` precedent (`agent_worker.rs:2521-2526`). | Kept as the plan says. The offline test asserts `message.contains(REGISTRY_ON_SERVER_ONLY)`, not equality, as `tests/box_settings.rs:262-266` does for `DATABASE_UNREACHABLE`. |
| **F-8** | **Major** | T2 | D247: "a refusal there is `Failed` with the same sentence". D239: `Stale` on edit only. | Returning a draft refusal as `Err(StoreError::Constraint(s))` renders as `constraint violated: s`, so the sentence is **not** the same as the section's local refusal. And `upsert_agent(&row, None)` can answer `Stale` for a create (an id already stored). D239 maps `Stale` for edits only. | **D250**: `agent_settings::serve` answers a draft or name refusal as `Ok(StoreReply::Failed { request: request.name(), message: refusal.to_string() })`, which is byte-identical to what the section shows. A create's `Stale` (an id collision, unreachable with UUIDv7) answers `Failed` with `` agent id `<id>` is already stored; nothing was written ``. Store errors (`Constraint` for a taken name, `Unreachable`, `Backend`) still propagate as `Err`, so the `?` path of `go_offline` keeps working. |
| **F-9** | Minor | T2 | D240: "boxed if clippy asks". | `clippy::large_enum_variant` fires only when the largest variant exceeds the second-largest by 200 bytes or more. `AgentDraft` is about 104 B (three 24 B `String`/`Vec` headers, an `Option<String>`, two one-byte enums and a `bool`). `CreateAgent` is about 152 B. `AgentWritten` is about 72 B (a `Vec` plus `AgentWrite`, whose largest arm is `Switched { AgentId, String, bool }`). Neither is near the threshold. | No `Box` in the design. If clippy fires anyway, box the payload (`draft: Box<AgentDraft>`, or `agents: Box<[AgentSummary]>`). **Never** `#[allow]`. |
| **F-10** | Minor | T2 | D243: add the line in `probe_agents_on`; test "over `MemStore` with a scripted probe environment". | `probe_agents_on` (`agent_worker.rs:2147-2185`) is shared by the box probe (`run_box_probe`, `:2302`), so the one line covers both replies. A row with a **literal** `launch.command` and no `discovery` probes `ready` without spawning anything: `probe_agent` step 6 (`htui-agent/src/probe.rs:1319-1320`) runs tier 2 only for an `acp` row whose `discovery` asks for a handshake. | The D243 unit test calls `probe_agents_on` directly (it is a private fn in the same module) over `ProbeEnv::host(tmp)` and one such row (§4.6). |
| **F-11** | Minor | T3 | Test `n_opens_the_create_form_and_captures_input`: "`l` and `h` are typed, not cycled". | `AgentsSection` does not override `captures_input` today (default `false`, `settings/mod.rs:149-151`). Section cycling is `SettingsTab::on_key`'s (`:323-335`), which a `SectionBench` over a lone section never runs. | T3 adds `fn captures_input(&self) -> bool { matches!(self.mode, Mode::Editing(_)) }`. The test asserts `section.captures_input()`, feeds `l` and `h` to the section and reads them back in the field. A second test (`the_settings_tab_gives_an_open_form_every_letter`) drives a `SettingsTab` as the hierarchy suite does. |
| **F-12** | Minor | T3 | D245: "line two is the notice, else `QUOTA_NOTE` in the idle state". | `hint()` (`agents.rs:956-978`) returns one `String`, and the non-idle states have no note today. | `hint()` becomes `fn hint(&self) -> (&'static str, Option<String>)`: keys, plus the second line's text. The note is `self.notice.clone().or_else(|| idle.then(|| QUOTA_NOTE.to_owned()))`, where `idle` is Browse with no install and no login. A `None` note draws an empty line, so the layout never changes height. |
| **F-13** | Minor | T3 | R-9: "the create notice names the adapter id when the row is `cli` and `settings.cli` is absent". | `adapter_id_from` is private (`htui-agent/src/registry.rs:137`). | The section decides from data alone: `row.agent.transport == Transport::Cli && row.agent.settings.get("cli").is_none()` appends `` · a `cli` row needs a settings.cli block to chat (adapter id `cli` is not registered) ``. No name is read, so `R-AGT-5` holds. |
| **F-14** | Minor | T3 | D239/D240: the form sends its token and lands the reply on `AgentWritten`. | `Failed { request: "agents" }` clears `self.agents` (`agents.rs:1262-1266`), and every read re-sorts the rows by name. An editor that indexed `self.agents` by cursor could edit the wrong row after a create sorts one in above it. | The editor holds its own `agent_id`, `name`, `expected` and the draft it opened with (§5.2). It never reads `self.agents` by index, and it finds its row by id only when it takes a new token on `Stale`. |
| **F-15** | Minor | T1 | D238: `enabled (y/n)` "uses `yes_or_no` (`mod.rs:53-59`)". | `yes_or_no` is `pub(crate)` in `crate::ui::tabs::settings`, so it is visible from `crate::agent_settings`, but it makes a store-side module import a UI module. | `agent_settings::parse_enabled` calls `crate::ui::tabs::settings::yes_or_no`. That is one convention, kept, and T1 edits neither file. A `use` line records it. |
| **F-16** | Major | T1 | T1 action: "`AgentDraft` … and the pure functions above". | T2 needs a pure create builder (D239's blank `launch`), T3 needs a row → draft projection (prefill and "unchanged closes"), and D246 needs "did transport/command/args change". Without them, T2 and T3 would each parse `launch` themselves (D247's two-copies hazard). | T1 adds `new_agent`, `draft_of`, `launch_changed`, `DraftFields` and `Refusal` to `agent_settings.rs` (T1's file, §3.2). No file list changes. |
| **F-17** | Minor | T1 | "`cargo tree -p htui -i shell-words` shows 1.1.1 only". | `cargo tree -i` prints the inverted tree, with `agent-client-protocol` **and** `htui` as parents. The lock gains no package, only the `htui` entry's `dependencies` list. | Gate: `cargo tree -p htui -i shell-words --edges normal` lists `shell-words v1.1.1`; `git diff Cargo.lock` is exactly one added line in `[[package]] name = "htui"`. If resolution needs the network in the sandbox, use `--offline`: the crate is in `~/.cargo/registry`. |
| **F-18** | Minor | all | — | `rustfmt` rewraps long `format!` and SQL strings at `max_width = 100`, and every earlier blueprint's gate starts with `cargo fmt --all -- --check`. The plan's per-task Validate lines omit it. | Every gate in this file starts with `cargo fmt --all -- --check`. |
| **F-19** | Minor (record) | T0 | The conformance case over "an absent pair" and "an existing pair". | Each case gets a **fresh** demo store: `run_all(|| async { MemStore::demo() })` (`crates/htui-core/tests/mem_store.rs:13`) and one `common::demo_db()` per case on Postgres (`pg_conformance.rs:35-40`). The demo holds no `agent_box` row ("no fixture loads it", `mem.rs:177-179`), and `ids::BOX` exists on both stores (`upsert_agent_box_by_pk`, `conformance.rs:1633`, writes it). | The case uses `(ids::AGENT_AGY, ids::BOX)` (`fixtures.rs:145`) and makes the pair present itself before the "existing" steps. No ordering assumption is needed. |
| **F-20** | Minor | T3 | D232: "`r`/`i`/`a` are refused while a registry write is in flight (`busy`)". | A second `Enter` in an open form, and `n`/`e`/`t` pressed after `t`, would also send a second write before the first answers (the hierarchy `submit` rule, `settings/hierarchy.rs:825-830`). | `busy: Option<&'static str>` refuses `Enter`, `n`, `e`, `t`, `r`, `i` and `a` with `` `<name>` is still in flight ``. It is cleared by `AgentWritten` and by a `Failed` whose `request` is in `agent_settings::REQUEST_NAMES`, and by nothing else. |
| **F-21** | Minor (record) | T0 ∥ T1 | "T0 ∩ T1 = ∅ … three conditions". | Checked, no hidden coupling. `AgentSummary` is constructed at exactly four sites (`pg/read.rs:1349`, `mem.rs:1481`, `cache/read.rs:1420`, `crates/htui/tests/settings.rs:244`), with no destructuring pattern anywhere, so T1 cannot trip on the field. T1 adds no `WriteStore` implementor. T0 touches no `Cargo.*`, and T1 touches no `.sqlx`, snapshot, seed or pin. Neither edits `crates/htui/src/lib.rs` except T1. The one shared resource is disk: two worktrees are two `target/` dirs, and 100 GB are free. | No change to order. The reviewer runs `rg -n 'impl.*WriteStore for' crates/htui` (expects nothing) and `git diff --stat` per lane against §7's file lists. |
| **F-22** | Minor | T2 | New `tests/agent_settings.rs`, `tests/agent_settings_pg.rs`. | Both sibling suites open with `#![cfg(feature = "testkit")]` (`tests/box_settings.rs:1`, `tests/box_probe_pg.rs:1`). The Postgres one uses `htui_store::testkit::demo_db()`, which prints `SKIP` without a DSN and panics under `CI`. | Both new files carry `#![cfg(feature = "testkit")]`. The Postgres one follows `box_probe_pg.rs`'s `let Some(db) = testkit::demo_db().await else { return };` and ends with `db.drop_db().await`. It is not `#![cfg(unix)]`, because it touches no disk. |
| **F-23** | Minor | T2 | D241: "`SetAgentOnBox` … answers `Failed` naming an unregistered box when there is none". | `Backend::box_info()` answers `Ok(None)` before registration (`backend.rs:252`). | The sentence is `this box is not registered yet; the per-box switch needs its agent_box row`, returned as `Ok(StoreReply::Failed { .. })` (D250). |
| **F-24** | Minor | T3 | "New snapshot `settings__agents_edit_form` (the `agy` row: an 11-model list clipped with `…`)". | Verified: `TextField::line` draws a window ending at the cursor, with a leading `…` when the start is clipped (`ui/text_field.rs:236-243`). A prefilled field has its cursor at the end, so a long `models` line shows `…` and then its **tail**. The section suite has no `agy` fixture (`registry_row` makes up names, `settings.rs:975-980`). | The snapshot's row is built with `probed_row` and the seed's model list read from `htui_core::model::agent::seed_rows(demo_at(0,0))[1]` (data, not a name branch). It is named `agent-b` in the snapshot. |

### 0.3 Probes (Postgres 16, scratch database, dropped afterwards)

- `PREPARE setsw(uuid, uuid, boolean) AS INSERT INTO agent_box (agent_id, box_id, enabled, user_off)
  VALUES ($1, $2, $3, NOT $3) ON CONFLICT … DO UPDATE SET user_off = NOT $3, enabled = $3 AND (…)`
  has parameter types `{uuid,uuid,boolean}`. One parameter used in two roles is fine for `query!`.
- The plan's arm over `probe = '{"source":"probe"}'` or `probe = 'null'` with `$3 = true` gives
  `ERROR: null value in column "enabled" … violates not-null constraint` (F-1). The COALESCE arm
  (§2.4) gives `enabled = f`, `user_off = f`.
- Unknown agent gives `agent_box_agent_id_fkey`, and unknown box gives `agent_box_box_id_fkey`, both
  23503, so both map to `Constraint`.
- A switch-off on an absent pair inserts `enabled = f, user_off = t, probe NULL, probed_at NULL`.
- The upsert with `enabled = EXCLUDED.enabled AND NOT agent_box.user_off` over that row with
  `enabled = true` leaves `enabled = f, user_off = t`. The setter with `true` then gives
  `enabled = t` (`probe` now `{"status":"ready"}`), `user_off = f`.
- `COMMENT ON COLUMN agent_box.user_off IS '…' '…'` (adjacent literals on separate lines)
  round-trips through `col_description` as the single sentence pinned in §2.8.

---

## 1. Build order and validation, at a glance

| Task | Crates | Commits (each compiles) | Gate (all `--test-threads=1`) |
|---|---|---|---|
| T0 per-box switch | htui-core, htui-store, htui-agent, htui (one test literal) | 3 (§2.10) | fmt; core; `cargo check --workspace --all-features --all-targets`; store (Postgres runs); agent; `htui --test settings`; clippy on the three store crates; `.sqlx` recipe (289, 2 D + 3 ??) |
| T1 draft rules | htui, workspace manifests | 2 (§3.5) | fmt; `htui --lib agent_settings`; clippy `-p htui`; `cargo tree` / lock diff |
| merge | — | **T0, then T1** | after each: the gates of the crates it touched, on the merged tree; after T1 also `cargo check --workspace --all-features --all-targets` |
| T2 served writes | htui | 3 (§4.8) | fmt; `htui` (all); `agent_settings_pg` must **run**; clippy `-p htui` |
| T3 the pane | htui | 3 (§5.8) | fmt; `htui` (all); snapshots 110; clippy `-p htui` |
| end | — | — | §7 workspace gate |

---

## 2. T0: the per-box switch in the store (D242; D249, F-1…F-6, F-19)

**First failing test**: `store::conformance` case `set_agent_box_enabled_switches_one_row_or_refuses`
(through `crates/htui-core/tests/mem_store.rs::mem_store_conformance`).

**Files** (the plan's list, unchanged): `crates/htui-store/migrations/0008_agent_box_user_off.sql`
(new), `crates/htui-store/tests/migrations.rs`, `crates/htui-store/tests/connect.rs`,
`crates/htui-core/src/model/agent.rs`, `crates/htui-core/src/store/traits.rs`,
`crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`,
`crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`,
`crates/htui-store/src/pg/read.rs`, `crates/htui-store/src/cache/read.rs`,
`crates/htui-store/src/writer.rs`, `crates/htui-store/tests/pg_conformance.rs`,
`crates/htui-store/tests/pg_criteria.rs`, `crates/htui-store/.sqlx/`,
`crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs`,
`crates/htui/tests/settings.rs` (the literal at `:244-259` only).

### 2.1 Migration `crates/htui-store/migrations/0008_agent_box_user_off.sql` (complete)

```sql
-- 0008_agent_box_user_off.sql - MOD-23 (plan D242, OQ-1 answer B; blueprint D249).
-- Forward-only (R-STO-5).
--
-- agent_box.enabled keeps its one meaning, "this box may run this agent", and gains a second
-- author: the probe proposes it (MOD-2 D50: status = ready) and the human vetoes it from
-- Settings > Agents. The veto is its own column so that no re-probe can undo it:
-- upsert_agent_box writes enabled = EXCLUDED.enabled AND NOT agent_box.user_off on conflict, and
-- WriteStore::set_agent_box_enabled is the only writer of user_off (MOD-2 D74's single-writer
-- shape). Every reader of agent_box.enabled is right without a change. agent_box is not mirrored
-- (cache_migrations/0002_agent_mirror.sql), so there is no cache migration.

ALTER TABLE agent_box
    ADD COLUMN user_off BOOLEAN NOT NULL DEFAULT false;

COMMENT ON COLUMN agent_box.user_off IS
    'MOD-23 D242: the per-box switch, written only by set_agent_box_enabled. true keeps enabled '
    'false through every probe: upsert_agent_box writes enabled = EXCLUDED.enabled AND NOT '
    'user_off on conflict. Switching on re-derives enabled from the stored probe status.';
```

`build.rs` already registers `rerun-if-changed=migrations`, so a warm `target/` picks up the new
file and needs no clean build.

### 2.2 `model/agent.rs` (`AgentSummary`, `:182-195`; `AgentBox.enabled` doc, `:70-71`)

```rust
pub struct AgentSummary {
    /// The `agent` row.
    pub agent: Agent,
    /// This box's `agent_box` row; `Some` when a probe or the per-box switch wrote one.
    pub on_box: Option<AgentBox>,
    /// `agent_box.user_off` of this box's row (MOD-23 D242): the human switched this agent off on
    /// this box, so no probe turns `on_box.enabled` back on. `false` when there is no row, and
    /// always `false` from the offline mirror, which does not hold `agent_box`.
    #[serde(default)]
    pub user_off: bool,
}
```

The struct doc sentence "`Some` when a probe has run on this box" becomes "when a probe or the
per-box switch wrote one". `AgentBox.enabled`'s doc becomes the F-6 wording. `AgentBox` gains **no**
field: it has 64 struct literals in 23 files (plan claim 24).

### 2.3 `traits.rs`: the contract (after `set_agent_box_quota`, `:417-423`; before `record_box_probe`, `:425`)

```rust
    /// Switches this agent on or off **on one box** (MOD-23 D242): writes `agent_box.user_off` and
    /// re-derives `agent_box.enabled`, and nothing else. It never writes `probe`, `version`,
    /// `path`, `probed_at`, `quota` or `quota_at`.
    ///
    /// The probe proposes `enabled` and the human vetoes it. `false` sets `user_off` and
    /// `enabled = false`, and while `user_off` holds,
    /// [`upsert_agent_box`](WriteStore::upsert_agent_box) cannot turn `enabled` back on. `true`
    /// clears `user_off` and sets `enabled` to the stored probe's verdict: `true` when the row
    /// holds no probe document, else whether its `status` is `ready` (a document without a
    /// `status` is not ready).
    ///
    /// An absent row is inserted bare: `enabled` as switched, every probe column `NULL`, so a
    /// reader treats it as never probed. No compare-and-set: the switch is an absolute set, and a
    /// token on `agent_box.updated_at` would be spent by every probe. The last of two concurrent
    /// switches wins, and both see it on their re-read.
    ///
    /// The only writer of `user_off` (MOD-2 D74's single-writer shape).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the agent or the box
    /// does not exist; nothing is written.
    async fn set_agent_box_enabled(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        enabled: bool,
    ) -> Result<()>;
```

There is no default body (the trait has none), so all five implementors owe it in commit (a). The
`upsert_agent_box` doc (`:357-374`) gains a paragraph: "Since MOD-23 (D242) the update writes
`enabled` as `row.enabled && !user_off`: a row the human switched off on this box stays off whatever
the probe proposes. The insert is unchanged, since a fresh row has `user_off = false`." The module doc
gains one sentence after the MOD-9 paragraph: "**MOD-23** adds one narrow writer,
[`WriteStore::set_agent_box_enabled`], the per-box switch (plan D242)." Fix "six implementations"
→ "five" at `:397-398` (F-6).

### 2.4 Postgres (`pg/write.rs`, `pg/read.rs`, `cache/read.rs`)

**`upsert_agent_box`** (`pg/write.rs:1197-1224`): one `SET` term changes (`:1206`), and the doc
(`:1178-1195`) gains the D242 paragraph:

```sql
INSERT INTO agent_box (agent_id, box_id, enabled, version, path, probed_at, updated_at, probe)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
ON CONFLICT (agent_id, box_id) DO UPDATE SET
    enabled   = EXCLUDED.enabled AND NOT agent_box.user_off,
    version   = EXCLUDED.version,
    path      = EXCLUDED.path,
    probed_at = EXCLUDED.probed_at,
    probe     = EXCLUDED.probe
```

**`set_agent_box_enabled`** (new, after `set_agent_box_quota`, which ends about `:1280`):

```rust
    /// The per-box switch (MOD-23 D242, blueprint D249): one statement, keyed on the composite
    /// primary key. `user_off` and `enabled` and nothing else; `updated_at` is the `BEFORE UPDATE`
    /// trigger's on the conflict arm and the column default on the insert.
    ///
    /// Switching on re-derives `enabled` from the stored probe. The `COALESCE` is load-bearing: a
    /// probe document without `status` makes `->>'status'` `NULL`, and `true AND NULL` would
    /// violate `enabled NOT NULL` (blueprint F-1).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when the agent or the box does not exist (`23503`).
    async fn set_agent_box_enabled(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        enabled: bool,
    ) -> Result<()> {
        sqlx::query!(
            "INSERT INTO agent_box (agent_id, box_id, enabled, user_off) \
             VALUES ($1, $2, $3, NOT $3) \
             ON CONFLICT (agent_id, box_id) DO UPDATE SET \
                 user_off = NOT $3, \
                 enabled  = $3 AND (agent_box.probe IS NULL \
                                    OR COALESCE(agent_box.probe->>'status' = 'ready', false))",
            agent_id.as_uuid(),
            box_id.as_uuid(),
            enabled,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }
```

**`PgStore::agents`** (`pg/read.rs:1285-1352`): add `ab.user_off AS "box_user_off?",` after
`ab.probe AS "box_probe?"`, and build
`AgentSummary { agent, on_box, user_off: row.box_user_off.unwrap_or(false) }` at `:1349`.

**`CacheStore::agents`** (`cache/read.rs:1420-1434`): add `user_off: false,` after
`on_box: None,`. The mirror has no `agent_box`, so it has no switch to report.

### 2.5 `MemStore` (`mem.rs`; F-2)

- `State` field after `agent_boxes` (`:179`):

```rust
    /// `agent_box.user_off` (MOD-23 D242): the `(agent, box)` pairs the human switched off. A
    /// side set rather than an `AgentBox` field, because the column is not part of the row type.
    /// Written by [`WriteStore::set_agent_box_enabled`] only.
    agent_boxes_off: HashSet<(AgentId, BoxId)>,
```

  Initialise `agent_boxes_off: HashSet::new(),` in the `State` literal beside `agent_boxes` (`:296`).
  `HashSet` is already imported (it is used at `record_box_probe`).
- `agent_summaries` (`:1477-1489`): set
  `user_off: self.this_box.is_some_and(|box_id| self.agent_boxes_off.contains(&(agent.id, box_id)))`.
- `State::upsert_agent_box` (`:1656-1690`), update arm: after `*stored = row.clone();`, add
  `stored.enabled = row.enabled && !self.agent_boxes_off.contains(&(row.agent_id, row.box_id));`.
  The borrow of `self.agent_boxes` via `get_mut` conflicts with reading `self.agent_boxes_off`, so
  compute `let off = self.agent_boxes_off.contains(&key);` **before** the `match`. The insert arm
  is unchanged. Doc: add "and `enabled` is ANDed with the per-box switch (MOD-23 D242), which is
  `SET enabled = EXCLUDED.enabled AND NOT user_off`".
- New `State` fn after `set_agent_box_quota` (`:1696-1718`):

```rust
    /// The per-box switch (MOD-23 D242): Postgres's one statement, written out. The referents in
    /// `upsert_agent_box`'s order and with its sentences; then an absent row is inserted bare, and
    /// a present one gets `enabled` (switched off) or the stored probe's verdict (switched on), with
    /// `updated_at` bumped as the trigger does it there.
    fn set_agent_box_enabled(
        &mut self,
        agent_id: AgentId,
        box_id: BoxId,
        enabled: bool,
        now: DateTime<Utc>,
    ) -> Result<()>
```

  Body: the two existence checks copied from `upsert_agent_box` (the same `Constraint` sentences);
  `if enabled { self.agent_boxes_off.remove(&key) } else { self.agent_boxes_off.insert(key) }`;
  then `match self.agent_boxes.get_mut(&key)`: `Some(row)` → `row.enabled = enabled &&
  probe_says_ready(row.probe.as_ref()); row.updated_at = now;`; `None` → insert the bare row of F-2.
  Put a private free fn `probe_says_ready(probe: Option<&Value>) -> bool` beside it (`None` →
  `true`, `Some(doc)` → `doc.get("status").and_then(Value::as_str) == Some("ready")`) with a
  doc naming the Postgres `COALESCE` it mirrors (D249).
- `impl WriteStore for MemStore`, after `set_agent_box_quota` (`:5718-5727`):
  `async fn set_agent_box_enabled(&self, agent_id: AgentId, box_id: BoxId, enabled: bool) ->
  Result<()> { let now = self.now(); self.write(|state| state.set_agent_box_enabled(agent_id,
  box_id, enabled, now)) }`.

### 2.6 The conformance case (`store/conformance.rs`; `CASES` 96 → 97)

`CASES` entry appended after `"a_lease_ttl_out_of_range_is_refused",` (`:140`). `run_case` arm
before `other => panic!` (`:332`). The fn goes after `upsert_agent_box_cannot_write_quota`
(`:1760`), with the other `agent_box` writers.

```rust
/// MOD-23 (plan D242, blueprint D249): `set_agent_box_enabled`, the per-box switch. It writes an
/// absent pair bare, switches an existing pair off and back on, re-derives `enabled` from the
/// stored probe when switching on (a status-less probe is not ready and is not an error), and
/// refuses an unknown agent or box with `Constraint`. What the switch leaves alone, and that an
/// upsert cannot undo it, is read back where a concrete store can be named
/// (`mem.rs::a_switched_off_row_stays_off_under_an_upsert_that_says_enabled` and
/// `pg_criteria.rs::a_switched_off_row_survives_a_re_probe_on_postgres`), because [`WriteStore`]
/// has no registry read.
async fn set_agent_box_enabled_switches_one_row_or_refuses<S: WriteStore>(store: &S)
```

Body, in order (`const CASE`, `.expect(CASE)` on every `Ok`, messages prefixed `{CASE}:`):

| Step | Call | Asserts |
|---|---|---|
| 1 | `set_agent_box_enabled(ids::AGENT_AGY, ids::BOX, false)` (F-19: the demo holds no `agent_box` row) | `Ok(())`: an absent pair is written, not refused |
| 2 | `upsert_agent_box(&AgentBox { agent_id: AGY, box_id: BOX, enabled: true, probe: Some(json!({"status":"ready","source":"probe"})), .. })` | `Ok(())`: the upsert still lands on a switched-off row |
| 3 | `set_agent_box_enabled(AGY, BOX, true)` then `(AGY, BOX, false)` | both `Ok(())` |
| 4 | `upsert_agent_box(&AgentBox { probe: Some(json!({"source":"probe"})), .. })`, then `set_agent_box_enabled(AGY, BOX, true)` | `Ok(())`: **F-1**, a status-less probe switched on is not a 23502 |
| 5 | `set_agent_box_enabled(AgentId::new(), BOX, false)` | `Err(StoreError::Constraint(_))` |
| 6 | `set_agent_box_enabled(AGY, BoxId::new(), true)` | `Err(StoreError::Constraint(_))` |

**Pins**: `crates/htui-core/tests/mem_store.rs:37` becomes `97`, and the message's tail "…, and one
for the lease …" gains ", and MOD-23's one for the per-box switch (plan D242)" before its closing
quote. `crates/htui-store/tests/pg_conformance.rs:19` becomes `const EXPECTED_CASES: usize = 97;`.

### 2.7 Read-backs (tests first)

**MemStore unit tests** (`mem.rs` `mod tests`, after `set_agent_box_quota_leaves_probe_and_version_alone`,
`:7024`; each over `MemStore::demo()`, whose `this_box` is `ids::BOX`):

| Test | Asserts |
|---|---|
| `a_switched_off_row_stays_off_under_an_upsert_that_says_enabled` | A probed `ready` row (`enabled: true`); `set_agent_box_enabled(false)`; `upsert_agent_box` with `enabled: true` and a new `version`. `agents()` then shows `on_box.enabled == false`, `user_off == true`, and the **new** version: the upsert wrote the other columns. |
| `switching_on_restores_the_probe_verdict` | `ready` → off → on: `enabled == true`. `unauthenticated` → off → on: `enabled == false`, `user_off == false`. A status-less `{"source":"probe"}` → on: `enabled == false` (F-1). |
| `the_switch_leaves_probe_version_path_probed_at_and_quota_alone` | After a quota latch, off then on: `probe`, `version`, `path`, `probed_at`, `quota`, `quota_at` are byte-equal to before; `updated_at` moved. |
| `agents_reports_user_off_for_this_box_only` | Switched off on `ids::BOX` and on a second box: `agents()` reports `user_off` for `ids::BOX`'s pair only. After switching the `ids::BOX` pair back on, `false`. |
| `a_switch_on_an_unprobed_agent_inserts_a_bare_row` | No row; `set_agent_box_enabled(false)` → `on_box == Some(AgentBox { enabled: false, version: None, path: None, probed_at: None, probe: None, quota: None, .. })`, `user_off == true`. |

**Postgres** (`crates/htui-store/tests/pg_criteria.rs`, `#![cfg(feature = "demo")]` already, each
over `common::demo_db()` and `db.store.this_box()`, read back through `db.store.agents()` or runtime
`sqlx::query_scalar`, **never** `query!` (F-5)):

| Test | Asserts |
|---|---|
| `a_switched_off_row_survives_a_re_probe_on_postgres` | The first MemStore row, on Postgres. |
| `switching_on_restores_the_probe_verdict_on_postgres` | The second, including the status-less probe → `enabled = false` and no error. |
| `the_switch_leaves_the_probe_columns_byte_identical_on_postgres` | The third. The JSONB `probe` is compared as `Value`, and `updated_at` is read with runtime `sqlx::query_scalar` and is strictly later. |
| `a_switch_on_an_unprobed_agent_inserts_a_bare_row_on_postgres` | The fifth, plus `SELECT user_off FROM agent_box WHERE …` is `true`. |

### 2.8 Migration pins (`tests/migrations.rs`, `tests/connect.rs`; F-3)

- `migrations.rs:86-93`: `vec![1, 2, 3, 4, 5, 6, 7, 8]`. The message gains "… MOD-9 milestone 2's
  0007_skill_attachments.sql and MOD-23's 0008_agent_box_user_off.sql, in ordinal order".
- New constant after `MOD9_COLUMN_COMMENTS` (`:382-409`):

```rust
/// The one `COMMENT ON COLUMN` text of `0008_agent_box_user_off.sql` (MOD-23 plan D242), verbatim,
/// for [`ANA_COLUMN_COMMENTS`]'s reason.
const MOD23_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[(
    "agent_box",
    "user_off",
    "MOD-23 D242: the per-box switch, written only by set_agent_box_enabled. true keeps enabled \
     false through every probe: upsert_agent_box writes enabled = EXCLUDED.enabled AND NOT \
     user_off on conflict. Switching on re-derives enabled from the stored probe status.",
)];
```

- `.chain(MOD23_COLUMN_COMMENTS)` at all **three** chains: `:424-428`, `:482-485`, `:495-498`. The
  value assertion's message (`:433`) says "the ANA (or MOD-7, MOD-9 or MOD-23) text". The comment at
  `:469-471` becomes "…exactly the thirty-five contracts the three ANAs, MOD-7, ANA-22 and MOD-23
  wrote and no half-finished thirty-sixth". The message at `:503` becomes "exactly the thirty-five
  commented columns, and no others".
- `:884-885` `MigrationState::Pending(8)`, "eight embedded migrations, none applied"; `:983`
  `Pending(8)`; `:988` and `:1007` `HeadlessError::MigrationsPending(8)`.
- `connect.rs:141-143` `Pending(8)`, "…eight embedded migrations since MOD-23's
  0008_agent_box_user_off.sql"; `:156-159` `pending, 8`, "all eight … MOD-23 added
  0008_agent_box_user_off.sql"; `:243-245` `Pending(8)`, "(eight embedded migrations since MOD-23's
  0008_agent_box_user_off.sql)".
- `crates/htui/src/store_worker.rs:3076` (`MigrationsPending(…, 5)`) and `tests/connection.rs:871`
  (`…, 3`) are synthetic counts, not pins: **unchanged**.

### 2.9 The three forwards and the test literal

- `Writer` (`writer.rs`, after `set_agent_box_quota`, `:376-392`): `match self { Self::Memory(store)
  => store.set_agent_box_enabled(agent_id, box_id, enabled).await, Self::Online(pg) =>
  pg.set_agent_box_enabled(agent_id, box_id, enabled).await }`.
- `UsageSpy` (`htui-agent/src/conformance.rs`, after `set_agent_box_quota`, `:774`) and `SpyStore`
  (`htui-agent/tests/recorder.rs`, after `:505`): `self.inner.set_agent_box_enabled(agent_id,
  box_id, enabled).await`. Neither spy logs it: no recorder path calls it.
- `crates/htui/tests/settings.rs:244-259` (`probed_row`): add `user_off: false,` after
  `on_box: None,`. That is the only `crates/htui` edit in T0 (binding condition 2).

### 2.10 Commits (T0) and gate

1. **(a) red**: the migration; every §2.8 pin; the trait method and its doc; `AgentSummary.user_off`
   with `user_off: false` at all four constructors (Mem and Pg read placeholders; the real values
   land in (b)/(c)); the Mem `State::set_agent_box_enabled` and `PgStore::set_agent_box_enabled`
   with `todo!()` bodies (the Pg one names no `query!` yet, so it compiles offline and `.sqlx` does
   not move); the three forwards; the conformance case, its `CASES` entry and `run_case` arm; both
   `CASES` pins; the five MemStore and four Postgres tests. Red: the case panics at `todo!()`, and
   the read-backs fail. `migrations.rs`/`connect.rs` are green on arrival.
2. **(b) green, `htui-core`**: the side set, both Mem rules, `agent_summaries`, the Mem setter, and
   the F-6 docs in `traits.rs`, `mem.rs` and `model/agent.rs`.
3. **(c) green, `htui-store`**: the Pg setter, the `SET` term, the `agents()` projection, the
   `pg/write.rs` doc, and `.sqlx` per §0.1.

```bash
cargo fmt --all -- --check
cargo test -p htui-core --all-features -- --test-threads=1
cargo check --workspace --all-features --all-targets             # five implementors, four constructors
cargo test -p htui-store --all-features -- --test-threads=1      # pg_conformance, pg_criteria, migrations, connect
cargo test -p htui-agent --all-features -- --test-threads=1      # UsageSpy, SpyStore
cargo test -p htui --all-features --test settings -- --test-threads=1   # the one literal
cargo clippy -p htui-core -p htui-store -p htui-agent --all-features --all-targets -- -D warnings
# §0.1 .sqlx recipe: prepare --check clean, 289 files, 2 " D" + 3 "??"
git diff --stat HEAD~3 -- crates/htui/src crates/htui-orch crates/htui-agent/src/probe.rs   # empty
```

---

## 3. T1: the draft and its rules (D233–D238, D247; F-15, F-16, F-17)

**First failing test**: `agent_settings::tests::args_round_trip_through_shell_words`.

**Files** (the plan's list, unchanged): `Cargo.toml`, `crates/htui/Cargo.toml`, `Cargo.lock`,
`crates/htui/src/agent_settings.rs` (new), `crates/htui/src/lib.rs`.

**Binding (F-21)**: T1 constructs no `AgentSummary`, calls no `set_agent_box_enabled`, and adds no
`WriteStore` implementor. Its tests use `Agent` rows only.

### 3.1 Dependency and module

`Cargo.toml` `[workspace.dependencies]`, after `unicode-segmentation` (`:130`), before
`[workspace.lints.rust]` (`:132`):

```toml
# MOD-23 D236 (OQ-2): the agent form's `args` field as POSIX shell words. 1.1.1 is already compiled
# as a dependency of agent-client-protocol, so this promotes a transitive crate and adds nothing.
shell-words            = "1.1"
```

`crates/htui/Cargo.toml` `[dependencies]`, after `unicode-segmentation` (`:60`):
`# MOD-23 D236: the agent form's args field.` then `shell-words = { workspace = true }`.
`crates/htui/src/lib.rs`: `pub mod agent_settings;` before `pub mod agent_worker;` (`:12`).

### 3.2 `crates/htui/src/agent_settings.rs`: module doc and the pure rules

Module doc: "The agent registry editor of `Settings > Agents` (MOD-23): the draft a form produces,
the rules that parse it, and (T2) the three writes served in the store loop. Everything here but
`serve` is pure: no clock, no id, no store, so the section can run it on the UI task (`R-NF-3`) and
the worker runs it again before it writes (D247). Nothing here reads or branches on an agent's name
(`R-AGT-5`), and nothing prints `launch.env` (`R-SEC-2`)."

```rust
use chrono::{DateTime, Utc};
use htui_agent::launch::AgentLaunch;
use htui_core::model::{Agent, AgentId, Billing, Transport};
use serde_json::{Map, Value, json};

/// The form's labels, in tab order (D231). `name` is the create form's only; the edit form starts
/// at index 1. The section's `Field`s take their labels from here, so a refusal's field name and
/// the label on screen cannot drift.
pub const FIELD_LABELS: [&str; 8] =
    ["name", "transport", "command", "args", "models", "default model", "billing", "enabled (y/n)"];

/// What the form edits of an `agent` row (D231): everything but `name`, `settings` and the keys of
/// `launch` other than `command` and `args`. `env` is not here and never shown (D235, `R-SEC-2`).
/// `args` is not a secret channel: secrets travel through `env` or a `${tool}` placeholder, and
/// `AgentLaunch`'s own `Debug` prints `args` too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDraft {
    /// `agent.transport`.
    pub transport: Transport,
    /// `agent.launch.command`, trimmed, non-empty.
    pub command: String,
    /// `agent.launch.args`, as shell words split them.
    pub args: Vec<String>,
    /// `agent.models`, in order, no repeats.
    pub models: Vec<String>,
    /// `agent.default_model`.
    pub default_model: Option<String>,
    /// `agent.billing`.
    pub billing: Billing,
    /// `agent.enabled`: the row everywhere, not the per-box switch.
    pub enabled: bool,
}

/// One refused field (D247): which field and why, printed as `` `<field>`: <reason> ``.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// One of [`FIELD_LABELS`], or `launch` for a stored document the merge cannot use.
    pub field: &'static str,
    /// The sentence, lower case, no trailing period.
    pub reason: String,
}
// impl Display: "`{field}`: {reason}"; impl std::error::Error.

/// The form's text, one `&str` per field of [`FIELD_LABELS`] after `name`.
#[derive(Debug, Clone, Copy)]
pub struct DraftFields<'a> {
    pub transport: &'a str, pub command: &'a str, pub args: &'a str, pub models: &'a str,
    pub default_model: &'a str, pub billing: &'a str, pub enabled: &'a str,
}   // each field documented
```

| Fn (all `#[must_use]` where they return a value) | Rule | Refusal (`reason`, exact) |
|---|---|---|
| `pub fn valid_name(name: &str) -> bool` | D234: `1..=64` chars from `[a-z0-9._-]`, first `[a-z0-9]` | — |
| `pub fn parse_name(text: &str) -> Result<String, Refusal>` | trim, then `valid_name` | `1-64 of a-z 0-9 . _ -, starting with a letter or digit` |
| `pub fn parse_transport(text: &str) -> Result<Transport, Refusal>` | trim, ASCII-lowercase, `FromStr` | `is acp or cli` |
| `pub fn parse_command(text: &str) -> Result<String, Refusal>` | trim, non-empty | `is required` |
| `pub fn parse_args(text: &str) -> Result<Vec<String>, Refusal>` | `shell_words::split` | the crate's error `Display` (`missing closing quote`) |
| `pub fn format_args(args: &[String]) -> String` | `shell_words::join` | — |
| `pub fn parse_models(text: &str) -> Result<Vec<String>, Refusal>` | split `,`, trim, drop empties, keep order | `` `<m>` is listed twice `` |
| `pub fn format_models(models: &[String]) -> String` | join `", "` | — |
| `pub fn parse_default(text: &str, models: &[String]) -> Result<Option<String>, Refusal>` | trim; empty → `None`; non-empty list must contain it | `` `<d>` is not one of the models `` |
| `pub fn parse_billing(text: &str) -> Result<Billing, Refusal>` | as transport | `is subscription or per_token` |
| `pub fn parse_enabled(text: &str) -> Result<bool, Refusal>` | `crate::ui::tabs::settings::yes_or_no` (F-15) | `is y or n` |
| `pub fn draft_from_fields(fields: &DraftFields<'_>) -> Result<AgentDraft, Refusal>` | each parser in `FIELD_LABELS` order; the first refusal wins | — |
| `pub fn merge_launch(stored: &Value, command: &str, args: &[String]) -> Result<Value, Refusal>` | D235: `stored` must be an object; replace `command` and `args`, keep every other key byte for byte; the result must deserialise as `AgentLaunch` | field `launch`: `the stored launch document is not a JSON object`, or the serde sentence |
| `pub fn apply_draft(stored: &Agent, draft: &AgentDraft) -> Result<Agent, Refusal>` | `merge_launch`, then the six columns; `id`, `name`, `settings`, `created_at`, `updated_at` kept | from `merge_launch` |
| `pub fn new_agent(id: AgentId, name: String, draft: &AgentDraft, settings: Value, now: DateTime<Utc>) -> Result<Agent, Refusal>` | D239: `launch = {"command", "args", "env": {}}` (no `discovery`), checked as `AgentLaunch`; `created_at = updated_at = now` | from the `AgentLaunch` check |
| `pub fn draft_of(agent: &Agent) -> AgentDraft` | the prefill: `launch.command` as a string or `""`, `launch.args` string elements in order | — |
| `pub fn launch_changed(stored: &Agent, draft: &AgentDraft) -> bool` | D246: `transport`, `command` or `args` differ from `draft_of(stored)` | — |

`env` never appears in any `Refusal`: the serde sentence for a bad `launch` names a key path, not a
value. If a test shows otherwise, the refusal drops the serde text and says
`the merged launch document is not a valid AgentLaunch`.

### 3.3 Tests (first; in-module `#[cfg(test)] mod tests`, as `ui/text_field.rs` does)

| Test | Pins |
|---|---|
| `args_round_trip_through_shell_words` | `parse_args(&format_args(a)) == Ok(a)` for: `[]`, `[""]`, `["a b"]`, `["it's"]`, `["say \"hi\""]`, `["--uid="]`, `["${claude_agent_acp}"]`, `["C:\\tools\\x.exe"]`, and the claude seed's `args` read from `seed_rows` |
| `an_unclosed_quote_is_refused_by_field` | `parse_args("'abc")` → `Refusal { field: "args", .. }`, and its `Display` starts with `` `args`: `` |
| `names_follow_the_d234_rule` | The three seed names pass; `""`, 65 chars, `Claude`, `a b`, `-x`, `a\u{7}` fail |
| `models_keep_order_drop_empties_and_refuse_a_repeat` | `" b, a,, b "` → refused naming `b`; `" b, a ,"` → `["b", "a"]` |
| `a_default_must_be_listed_unless_the_list_is_empty` | `("x", ["a"])` refused; `("a", ["a"])` → `Some("a")`; `("x", [])` → `Some("x")`; `("", ["a"])` → `None` |
| `transport_and_billing_accept_case_and_space_and_name_the_values` | `" ACP "` → `Acp`; `"Per_Token"` → `PerToken`; `"ssh"` → `` `transport`: is acp or cli `` |
| `merge_launch_keeps_env_discovery_install_and_unknown_keys` | Over the `registry_row`-shaped launch plus `"env": {"TOKEN": "${tok}"}` and `"x_future": [1]`: `env`, `discovery` (with `install`) and `x_future` are `==` to the input; `command`/`args` replaced |
| `merge_launch_refuses_a_non_object_and_an_invalid_result` | `json!([])` → field `launch`; an object whose `discovery` is `7` → refused |
| `apply_draft_keeps_name_settings_and_both_stamps` | the four fields are equal before and after |
| `new_agent_has_a_blank_launch_and_the_given_settings` | `launch == {"command": c, "args": a, "env": {}}`, no `discovery`; `settings` as given; both stamps `now` |
| `draft_from_fields_returns_the_first_refusal_in_field_order` | bad `transport` **and** bad `models` → the `transport` refusal |
| `launch_changed_sees_only_transport_command_and_args` | a models-only edit → `false`; an args edit → `true` |

### 3.4 Gate (T1)

```bash
cargo fmt --all -- --check
cargo test -p htui --all-features --lib agent_settings -- --test-threads=1
cargo clippy -p htui --all-features --all-targets -- -D warnings
cargo tree -p htui -i shell-words --edges normal     # shell-words v1.1.1 (F-17)
git diff HEAD~2 -- Cargo.lock                        # one added line under name = "htui"
```

### 3.5 Commits (T1)

1. **(a) red**: both manifests, `Cargo.lock`, `lib.rs`, the module with every signature and `todo!()`
   bodies (nothing outside the module calls them), and all tests.
2. **(b) green**: the bodies.

---

## 4. T2: the writes, served (D239–D241, D243, D246; D250, F-7…F-10, F-22, F-23)

T2 branches from the merged Wave 1. It needs T0's `set_agent_box_enabled` and `user_off`, and T1's
module.

**First failing test**: `tests/agent_settings.rs::create_agent_lands_with_a_minted_id_and_answers_created`.

**Files** (the plan's list, unchanged): `crates/htui/src/agent_settings.rs`,
`crates/htui/src/store_worker.rs`, `crates/htui/src/agent_worker.rs`,
`crates/htui/tests/agent_settings.rs` (new), `crates/htui/tests/agent_settings_pg.rs` (new).

### 4.1 `store_worker.rs`: the variants

`StoreRequest`, directly after `EditBox { .. }` (`:225-232`):

```rust
    /// Create one `agent` row from the Settings form (MOD-23 D239). The worker mints the id and
    /// the clock, builds `launch` as `{command, args, env: {}}`, and copies `settings` from
    /// `settings_from` (OQ-5) or starts from `{}`. Answered with [`StoreReply::AgentWritten`], or
    /// with [`StoreReply::Failed`] for a refused field or a taken name. Offline:
    /// `REGISTRY_ON_SERVER_ONLY`.
    CreateAgent {
        /// `agent.name`, as typed; checked again here (D234).
        name: String,
        /// The parsed form.
        draft: AgentDraft,
        /// The row whose `settings` document the new row starts from.
        settings_from: Option<AgentId>,
    },
    /// Edit one `agent` row as a compare-and-set on `updated_at` (MOD-40 D5, MOD-23 D239). `name`,
    /// `settings` and every `launch` key but `command` and `args` are kept. Answered with
    /// [`StoreReply::AgentWritten`] (`Edited`, `Stale` or `Gone`) or [`StoreReply::Failed`].
    EditAgent {
        /// The row.
        agent_id: AgentId,
        /// The `updated_at` of the row as a registry read answered it, never a built one.
        expected: DateTime<Utc>,
        /// The parsed form.
        draft: AgentDraft,
    },
    /// Switch one agent on or off on **this** box (MOD-23 D242): `agent_box.user_off`. Answered
    /// with [`StoreReply::AgentWritten`] (`Switched`) or [`StoreReply::Failed`].
    SetAgentOnBox {
        /// The agent.
        agent_id: AgentId,
        /// `false` switches it off here; `true` returns it to the probe's verdict.
        enabled: bool,
    },
```

`use crate::agent_settings::{self, AgentDraft, AgentWrite};` beside the `box_settings` import
(`:40`). Check that `DateTime`/`Utc` and `AgentId` are imported; add what is missing.

`StoreRequest::name`, after `Self::EditBox { .. } => "edit_box",` (`:775`):

```rust
            // The three of `agent_settings::REQUEST_NAMES`, in that order (MOD-23 D241).
            Self::CreateAgent { .. } => "create_agent",
            Self::EditAgent { .. } => "edit_agent",
            Self::SetAgentOnBox { .. } => "set_agent_on_box",
```

`StoreReply`, directly after `BoxesStale(..)` (`:1050`):

```rust
    /// The answer to every registry write (MOD-23 D240): the registry re-read after the write, and
    /// what the write did. Self-naming (MOD-59): the section lands a write on this variant alone,
    /// and a plain [`StoreReply::Agents`] never closes the form or moves its token.
    AgentWritten {
        /// The registry as it is now, ordered by name.
        agents: Vec<AgentSummary>,
        /// What the write did.
        outcome: AgentWrite,
    },
```

`try_serve`: a new arm directly after the box arm (`:1356-1361`). Its comment states no count
(the plan's instruction), and the neighbouring counts are left as they are:

```rust
        // The three agent registry writes, or-ed for the reason the arms above are: a guard does
        // not count towards exhaustivity in a wildcard-free `match` (MOD-15 M3 plan F-12, MOD-23
        // D241). Served here, in the loop: one statement and one read each (`R-NF-3`).
        StoreRequest::CreateAgent { .. }
        | StoreRequest::EditAgent { .. }
        | StoreRequest::SetAgentOnBox { .. } => agent_settings::serve(backend, request).await?,
```

The runtime interception list (`store_worker.rs:1855-1869`) is explicit and does not name the new
variants, so they reach `try_serve`. Nothing else changes there.

### 4.2 `agent_settings.rs`: `AgentWrite`, `REQUEST_NAMES`

```rust
/// What one registry write did (MOD-23 D240). Carries ids and names only: no `launch`, no `env`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentWrite {
    /// A new row landed.
    Created { id: AgentId, name: String },
    /// The edit applied.
    Edited { id: AgentId, name: String },
    /// The token was spent: the row changed since the form read it. Nothing was written.
    Stale { id: AgentId },
    /// The row is gone. Nothing was written.
    Gone { id: AgentId },
    /// This box's switch for the agent is now `enabled`.
    Switched { id: AgentId, name: String, enabled: bool },
}   // every field documented

/// The three request names, in [`StoreRequest`] order (D241). [`StoreRequest::name`]'s arms and the
/// section's `Failed` match both read from here.
pub const REQUEST_NAMES: [&str; 3] = ["create_agent", "edit_agent", "set_agent_on_box"];
```

### 4.3 `serve` (D239–D241, D250)

```rust
/// Serves `CreateAgent`, `EditAgent` and `SetAgentOnBox` (MOD-23 D239-D241) in the store loop.
///
/// Offline there is no writer, and all three are `Err(Unreachable(REGISTRY_ON_SERVER_ONLY))`
/// before any read. A refused field or name is `Ok(Failed)` with the section's own sentence
/// (D247, blueprint D250). A store refusal (a taken name) propagates as `Err`. Every write that
/// reached the store answers `AgentWritten` with the registry re-read.
///
/// Known residue, `box_settings`'s: a re-read that fails after an applied write answers `Failed`,
/// though the row has changed.
///
/// # Errors
/// Whatever the store reports; `Backend` for a request that is not one of the three.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>
```

Body, in order:

1. `let writer = backend.writer().ok_or_else(|| StoreError::Unreachable(REGISTRY_ON_SERVER_ONLY.to_owned()))?;`
2. `CreateAgent { name, draft, settings_from }`:
   `let name = match parse_name(name) { Ok(n) => n, Err(r) => return Ok(refused(request, &r)) };`
   then `let rows = backend.agents().await?;` and `settings` = the `settings_from` row's
   `agent.settings.clone()`, else `json!({})`. A `settings_from` id that is no longer in `rows`
   is `{}` too; the header named it, and a vanished source is not a reason to refuse. Then
   `let row = new_agent(AgentId::new(), name, draft, settings, Utc::now())` (refusal →
   `Ok(refused)`). Then `match writer.upsert_agent(&row, None).await?`: `Applied(stored)` →
   `Created { id: stored.id, name: stored.name }`; `Stale(_)` → `Ok(Failed { .., message:
   format!("agent id `{}` is already stored; nothing was written", row.id) })` (F-8).
3. `EditAgent { agent_id, expected, draft }`: `let rows = backend.agents().await?;`. No row with
   the id → `Gone`. Else `apply_draft(&stored.agent, draft)` (refusal → `Ok(refused)`), then
   `match writer.upsert_agent(&merged, Some(*expected)).await`: `Ok(Applied(row))` →
   `Edited { id, name: row.name }`; `Ok(Stale(_))` → `Stale { id }`;
   `Err(StoreError::NotFound { entity: "agent", .. })` → `Gone { id }`; `Err(other)` →
   `return Err(other)`.
4. `SetAgentOnBox { agent_id, enabled }`: `let Some(info) = backend.box_info().await? else { return
   Ok(Failed(F-23 sentence)) };` then `writer.set_agent_box_enabled(*agent_id, info.box_id,
   *enabled).await?;`.
5. `let agents = backend.agents().await?;`. For `Switched`, `name` is the re-read row's
   (`agents.iter().find(|s| s.agent.id == *agent_id)`); if the re-read no longer holds the row,
   `Gone { id }`.
6. `Ok(StoreReply::AgentWritten { agents, outcome })`.
7. `other =>` `Err(StoreError::Backend(format!("not an agent registry request: {}", other.name())))`.

`fn refused(request: &StoreRequest, refusal: &Refusal) -> StoreReply` is private and builds
`Failed { request: request.name(), message: refusal.to_string() }`.

### 4.4 D243 (`agent_worker.rs:2172-2174`)

```rust
            ProbeOutcome::Row(mut row) => {
                writer.upsert_agent_box(&row).await?;
                // MOD-23 D243: the store kept `enabled AND NOT user_off`; the reply says the same.
                row.enabled &= !summary.user_off;
                summary.on_box = Some(row);
            }
```

That is the whole change. The box probe (`:2302`) shares the fn (F-10). The re-probe (`:2500`),
install (`:2759`) and login (`:2966`) need nothing (plan D243, verified).

### 4.5 Tests (first): `crates/htui/tests/agent_settings.rs` (new, F-22)

The header is `#![cfg(feature = "testkit")]`. Helpers: `demo() -> Backend` (`Backend::memory(MemStore::demo())`),
`written(reply) -> (Vec<AgentSummary>, AgentWrite)` (panics naming the reply otherwise),
`refusal(reply) -> (&'static str, String)`, and `draft()`, a valid `AgentDraft` with a literal
`command` `/usr/bin/true` and `args` `["--flag"]`.

| Test | Asserts |
|---|---|
| `create_agent_lands_with_a_minted_id_and_answers_created` | `CreateAgent { name: "agent-x", settings_from: Some(ids::AGENT_CLAUDE) }` → `Created { name: "agent-x", id }`; `agents` holds the row with `launch == {"command": "/usr/bin/true", "args": ["--flag"], "env": {}}` and `settings ==` claude's `settings`; the id is none of the three demo ids |
| `create_agent_without_a_source_starts_from_empty_settings` | `settings_from: None` → `settings == {}` |
| `create_agent_with_a_taken_name_is_failed_with_the_store_sentence` | the name `claude` → `Failed { request: "create_agent", message }`, `message.contains("agent_name_key")`; the registry still has three rows |
| `create_agent_with_a_bad_name_is_failed_with_the_sections_sentence` | `"Bad Name"` → the message **equals** `parse_name("Bad Name").unwrap_err().to_string()` (D250) |
| `edit_agent_applies_and_keeps_name_env_discovery_and_settings` | Over claude with `expected` from `backend.agents()`: `Edited`; `name`, `settings`, `created_at`, `launch.env`, `launch.discovery` equal to before; `models`/`billing` as drafted; `updated_at` moved |
| `edit_agent_with_a_spent_token_answers_stale_and_writes_nothing` | Edit once, then again with the **first** token → `Stale { id }`; the row equals the first edit's |
| `edit_agent_on_an_unknown_id_answers_gone` | `AgentId::new()` → `Gone` |
| `edit_agent_with_an_invalid_draft_is_failed_by_field` | `default_model` not in `models` → `Failed { request: "edit_agent" }`, message starts with `` `default model`: `` |
| `set_agent_on_box_off_then_on_answers_switched_and_the_summary_says_so` | off → `Switched { enabled: false }`, the summary `user_off == true`, `on_box.enabled == false`; on → `Switched { enabled: true }`, `user_off == false` |
| `offline_refuses_all_three_by_name_with_the_registry_sentence` | `Backend::Offline` over `CacheStore::open(tempdir, "agent-settings-offline", PgStore::schema_version())` (the `tests/box_settings.rs:245-257` shape); each request → `Failed { request == REQUEST_NAMES[i] }`, `message.contains(REGISTRY_ON_SERVER_ONLY)` (F-7) |
| `a_request_from_elsewhere_is_named_not_panicked` | `agent_settings::serve(&demo(), &StoreRequest::BoxInfo)` → `Err(Backend(m))`, `m.contains("box_info")` |

**`crates/htui/tests/agent_settings_pg.rs`** (new; `#![cfg(feature = "testkit")]`; `let Some(db) =
htui_store::testkit::demo_db().await else { return };`; `Backend::Online { pg: db.store.clone(),
cache }` over a tempdir `CacheStore`, as `box_probe_pg.rs` builds it; `cache.close()` and
`db.drop_db()` last):

| Test | Asserts |
|---|---|
| `an_edit_round_trips_a_token_read_from_postgres` | `expected` from `db.store.agents()` (microseconds, MOD-40 F-17) → `Edited`; a second edit with the reply's re-read token → `Edited`; with the first token → `Stale` |
| `a_switched_off_row_survives_an_upsert_that_says_enabled` | `SetAgentOnBox { enabled: false }`, then `db.store.upsert_agent_box(enabled: true, probe ready)` → `db.store.agents()` has `on_box.enabled == false`, `user_off == true` |

**`store_worker.rs` `mod tests`**, after `box_requests_are_named_as_box_settings_lists_them`
(`:3313-3327`): `agent_requests_are_named_as_agent_settings_lists_them` asserts
`[CreateAgent{..}.name(), EditAgent{..}.name(), SetAgentOnBox{..}.name()] ==
agent_settings::REQUEST_NAMES`.

### 4.6 D243 unit test (`agent_worker.rs` `mod tests`, beside `the_probe_task_answers_once_…`, `:5859`)

`a_probe_reply_keeps_a_switched_off_row_off`: `MemStore::demo()` plus one agent row whose
`launch = {"command": <a file that exists in a tempdir>, "args": [], "env": {}}` with no
`discovery`, transport `acp`. Per F-10 it probes `ready` with nothing spawned. Then
`set_agent_box_enabled(row, ids::BOX, false)`, then `store.agents()`. Call `probe_agents_on(&Writer::Memory(store.clone()), ids::BOX, agents, &ProbeEnv::host(tmp).without_versions())`.
Assert that the returned summary has `on_box.enabled == false`, and that `store.agents()` agrees.
If the literal command is not `ready` on this platform, check `probe_agent`'s step 4 (report
completeness) first; do not switch to a scripted adapter.

### 4.7 Gate (T2)

```bash
cargo fmt --all -- --check
cargo test -p htui --all-features -- --test-threads=1
cargo test -p htui --all-features --test agent_settings_pg -- --test-threads=1 --nocapture   # must RUN: no SKIP line
cargo clippy -p htui --all-features --all-targets -- -D warnings                               # F-9: no large_enum_variant
cargo test -p htui-agent --all-features --test extensibility -- --test-threads=1               # R-AGT-5 sweep
```

### 4.8 Commits (T2)

1. **(a) red**: the variants, `name` arms, the `try_serve` arm, `AgentWrite`, `REQUEST_NAMES`,
   `serve` with a `todo!()` body (reachable only through the three new requests, H-6), the names
   test, both new test files and the D243 unit test. Red: the serve cases panic, and the D243 case
   sees `enabled == true`.
2. **(b) green, worker**: `serve` and `refused`.
3. **(c) green, probe reply**: D243's line.

---

## 5. T3: the pane (D230–D232, D244, D245; F-11…F-14, F-20, F-24)

T3 branches from T2.

**First failing test**: `settings.rs::n_opens_the_create_form_and_captures_input`.

**Files** (the plan's list, unchanged): `crates/htui/src/ui/tabs/settings/agents.rs`,
`crates/htui/tests/settings.rs`, and the snapshots
`settings__agents_{demo,empty,probed,quota,unknown_row}.snap`, `probe__agents_probed_missing.snap`
(updated) and `settings__agents_{create_form,edit_form,switched_off}.snap` (new).

### 5.1 Constants (after `QUOTA_NOTE`, `agents.rs:98`)

```rust
const HINT_IDLE: &str =
    "j/k select \u{b7} n new \u{b7} e edit \u{b7} t this box \u{b7} r probe \u{b7} i install \u{b7} a authenticate";  // 79
const HINT_EDITING: &str = "Tab next field \u{b7} Enter saves \u{b7} Esc cancels";                             // 42
const SWITCHED_OFF: &str = "switched off";                                                                       // 12
const UNCHANGED: &str = "nothing changed; nothing was written";
```

`HINT_IDLE` replaces the old one (`:85`). Each constant gets its one-line doc.

### 5.2 State (`AgentsSection`, `:249-270`)

```rust
    /// Browsing, or a form open under the table (MOD-23 D230).
    mode: Mode,
    /// The registry write in flight, by request name (D232, F-20): refuses a second one and every
    /// probe, install and login key until `AgentWritten` or its `Failed` lands.
    busy: Option<&'static str>,
```

```rust
#[derive(Debug, Default)]
enum Mode { #[default] Browse, Editing(Editor) }

/// The open form (D231). Holds its own row identity (F-14): never an index into `agents`.
#[derive(Debug)]
struct Editor { target: Target, fields: Vec<Field>, focus: usize }

#[derive(Debug)]
enum Target {
    /// `n`: `settings` copied from this row, named in the header (OQ-5).
    Create { settings_from: Option<(AgentId, String)> },
    /// `e`: the row, its name for the header, the token and the draft it opened with.
    Edit { agent_id: AgentId, name: String, expected: DateTime<Utc>, opened: AgentDraft, sent: Option<AgentDraft> },
}

/// `settings/hierarchy.rs:150-157`'s `Field`, private here: a label from `FIELD_LABELS` and a
/// `TextField`.
#[derive(Debug)]
struct Field { label: &'static str, input: TextField }
```

`Editor::lines(width, theme)` is hierarchy's (`hierarchy.rs:1301-1360` shape): a label column
padded to 13 (`default model`), the focused label in `theme.accent`, and the focused field carrying
the cursor.

### 5.3 Keys (`on_key`, `:1117-1207`; D232)

- **First**, before the consent and chooser checks: `if let Mode::Editing(_) = self.mode { return
  self.on_editor_key(key, ctx); }`. The form cannot coexist with either modal (D232's exclusion), so
  the order only fixes which answers first.
- `on_editor_key` is hierarchy's (`hierarchy.rs:779-822`), verbatim in shape. The focused field
  answers first. `Submit` → `submit`. `Cancel` → `Browse`, notice cleared. `Pass` → `Tab`/`Down`
  next field, `BackTab`/`Up` previous, `CONTROL` chords → `Handled::Pass`, everything else consumed.
- The main match gains `n`, `e` and `t`, each after one shared guard,
  `fn refuse_write(&self, ctx) -> bool`. That guard refuses, in order: `busy` (`` `<name>` is still
  in flight ``); an install (`an install is running; edit afterwards`); a login (`a login is
  running; edit afterwards`); `probing` (`a probe is running; edit afterwards`). Then:
  - `n`: `Mode::Editing` with `Target::Create { settings_from: self.selected().map(|s| (s.agent.id,
    s.agent.name.clone())) }` and eight empty fields, except `transport` = `acp`, `billing` =
    `subscription`, `enabled (y/n)` = `y`. `focus: 0`.
  - `e`: the selected row, or `no agent row is selected`. Seven fields prefilled from
    `agent_settings::draft_of` with `format_args` / `format_models`, `expected =
    row.agent.updated_at`, `opened` = that draft.
  - `t`: the selected row → `busy = Some("set_agent_on_box")`, `ctx.request(SetAgentOnBox {
    agent_id, enabled: summary.user_off })`. The switch is `!user_off`, and `t` flips it.
- The existing `i`, `a` and `r` arms each gain `busy` as their first refusal, with the same sentence.
  Put one `KeyCode::Char('r' | 'i' | 'a') if self.busy.is_some()` arm ahead of them.
- `fn captures_input(&self) -> bool { matches!(self.mode, Mode::Editing(_)) }` in the
  `SettingsSection` impl (F-11).
- Update the key comment at `:1127-1129` to name the real globals (plan claim 9's amendment:
  `q`, `Tab`/`BackTab`, `1`–`9`, `?`, `ctrl-c`, and `w`), not `-`.

`submit` (hierarchy `:825` shape): refuse while `busy`. Build `DraftFields` from the fields. For
`Create`: `parse_name`, then the local "name already listed" check (`` `name`: `<n>` is already
registered ``), then `draft_from_fields`. A refusal sets `notice = refusal.to_string()`, moves
`focus` to the refused field's index in `FIELD_LABELS` (minus 1 on the edit form), and sends
nothing. For `Edit`: when `draft == opened`, close with `UNCHANGED` and send nothing. Otherwise
`sent = Some(draft.clone())`, `busy = Some("edit_agent")` and `EditAgent { agent_id, expected,
draft }`. For `Create`: `busy = Some("create_agent")`, `CreateAgent { name, draft, settings_from:
id }`.

### 5.4 Replies (`on_reply`, `:1209-1269`; D240)

- `StoreReply::AgentWritten { agents, outcome }` (new arm, first): `self.agents = agents.clone();
  self.unavailable = None; self.busy = None; self.clamp_cursor();`. It does **not** touch `probing`,
  `install` or `auth`. Then:
  - `Created { id, name }`: `Browse`; select the row with `id` (index by id in the new list); notice
    `` created `<name>` ``, plus F-13's clause when the new row is `cli` without `settings.cli`.
  - `Edited { id, name }`: `Browse`; notice `` saved `<name>` ``, plus `` · the next chat
    re-probes; r probes now `` when `launch_changed(old_row, sent)`. `old_row` is `draft_of`
    compared with `opened`. Use `opened != sent` on `transport`/`command`/`args`, which is what the
    form knew.
  - `Stale { id }`: the form stays with its text; `expected` = the new list's row `updated_at`;
    notice `CHANGED_ELSEWHERE`. If the id is absent from the list, handle it as `Gone`.
  - `Gone { .. }`: `Browse`; notice `DELETED_ELSEWHERE`.
  - `Switched { name, enabled, .. }`: notice `` `<name>` switched off on this box `` or
    `` `<name>` switched on; the probe's verdict decides ``.
- `StoreReply::Failed { request, message } if agent_settings::REQUEST_NAMES.contains(request)`:
  `busy = None`; the form stays open; `notice = Some(message.clone())`. The shell also puts it on
  the status line.
- `StoreReply::Agents` (`:1214-1219`) is **unchanged**. It never touches `mode`, `busy` or a
  token, and that is the whole of D240's rule on this side.

### 5.5 Cell, pane, hint, render (D244, D245; F-12)

- `on_box_cell` (`:866-895`), after the `probing` check and before `let Some(row)`: `if
  summary.user_off { return SWITCHED_OFF.to_owned(); }`. After `let Some(row) = …`: `if
  row.probed_at.is_none() && row.probe.is_none() { return NOT_PROBED.to_owned(); }`. Doc: add the
  two rules to the precedence sentence.
- `pane` (`:927-948`): `Mode::Editing(editor)` answers first with a header line — `new agent ·
  settings from <name>` / `new agent · settings {}` / `edit <name>` — in `theme.base`, then
  `editor.lines(width, theme)`. `pane` gains a `width: u16` parameter for that. Then the install
  and login panes as today.
- `hint()` → `(&'static str, Option<String>)` per F-12. `Editing` keys are `HINT_EDITING`; every
  other state keeps its keys.
- `render` (`:1271-1294`): `Layout::vertical([Min(3), Length(pane), Length(1), Length(1)])` →
  `[rows, pane_area, keys, note]`. The note line is `theme.error` when `is_error(note)`, else
  `theme.dim`. That is `settings/mod.rs:67-69`'s rule, and it is how `CHANGED_ELSEWHERE` renders in
  the other sections.
- `settings.rs:1565-1568` comment: "…the idle hint gained `a authenticate` (MOD-21 D20); since
  MOD-23 D245 the keys and the notice are two lines, so the notice no longer shares the keys' row."

### 5.6 Tests (first; `crates/htui/tests/settings.rs`, after the login block, `:2653`)

A `// --- MOD-23: the registry editor ---` divider. Helpers: `form_over(bench, rows) ->
AgentsSection`, `typed(section, bench, text)` feeding `KeyCode::Char` per char, and
`requests_of(bench)` (the existing `:2665` helper).

| Test | Asserts |
|---|---|
| `n_opens_the_create_form_and_captures_input` | `n` → `captures_input()`; `l`, `h` and `q` typed land in `name`; `ctrl-c` → `Handled::Pass`; the pane's first line names the highlighted row |
| `the_settings_tab_gives_an_open_form_every_letter` | Through `SettingsTab` (the hierarchy suite's shape): with the form open, `l` does not cycle sections (F-11) |
| `enter_on_the_create_form_sends_create_agent_with_the_parsed_draft_and_the_source_row` | exactly one `CreateAgent { name, draft, settings_from: Some(highlighted id) }` |
| `a_bad_field_is_refused_locally_by_name_and_sends_nothing` | one row per rule (name, taken name, transport, command, args quote, models repeat, default, billing, enabled): no request; the note line starts with the field; focus moves to it |
| `e_prefills_the_highlighted_row_without_a_name_field_and_sends_its_token` | seven fields; `args` shows `format_args`; `Enter` after one edit → `EditAgent { expected == row.updated_at }` |
| `an_unchanged_edit_closes_without_a_write` | `e`, `Enter` → no request, `Browse`, note `UNCHANGED` |
| `agent_written_edited_closes_the_form_and_says_re_probe_when_launch_changed` | an args edit → the note contains `r probes now`; a models edit → it does not |
| `agent_written_created_selects_the_new_row` | the cursor is on the new id after it sorted into the middle |
| `agent_written_stale_keeps_the_text_and_takes_the_new_token` | the text is unchanged; the next `Enter` sends the **new** `updated_at`; note `CHANGED_ELSEWHERE` |
| `agent_written_gone_closes_with_deleted_elsewhere` | `Browse`; note `DELETED_ELSEWHERE` |
| `a_refused_write_keeps_the_form_open_with_the_sentence` | `Failed { "create_agent", m }` → the form open, note `m`, `busy` cleared (a second `Enter` sends) |
| `an_agents_reply_does_not_close_the_form_or_move_its_token` | `StoreReply::Agents` mid-edit → the form open, `EditAgent.expected` unchanged |
| `t_sends_set_agent_on_box_with_the_inverse_of_the_switch` | on → `enabled: false`; a `user_off` row → `enabled: true` |
| `n_e_and_t_are_refused_while_a_probe_an_install_or_a_login_runs` | three states × three keys, no request each |
| `r_i_and_a_are_refused_while_a_registry_write_is_in_flight` | after `t`, before its reply |
| `a_switched_off_row_reads_switched_off` | a `user_off` summary → the cell reads `switched off` (at `SECTION_BORDERED`, 12 chars fit 13) |
| `a_bare_row_reads_not_probed` | `probed_at: None, probe: None` → `not probed` |
| `the_idle_keys_and_the_quota_note_are_two_lines` | the second-to-last line is `HINT_IDLE`; the last is `QUOTA_NOTE` |

**Snapshots**: `settings__agents_create_form` (`n` over two `registry_row`s),
`settings__agents_edit_form` (`e` over a row carrying the second seed row's 11 models and
21-char default, named `agent-b`; F-24), and `settings__agents_switched_off`, all rendered at
`SECTION_WIDE` like their siblings.

### 5.7 Gate (T3)

```bash
cargo fmt --all -- --check
cargo insta test -p htui --all-features -- --test-threads=1      # writes .snap.new for the six and the three
cargo insta pending-list                                         # nine entries; review each diff:
#   the six existing ones change ONLY in their last two lines (the keys row, then the note row)
cargo insta accept
cargo test -p htui --all-features -- --test-threads=1
ls crates/htui/tests/snapshots | wc -l                           # 110
cargo clippy -p htui --all-features --all-targets -- -D warnings
cargo test -p htui-agent --all-features --test extensibility -- --test-threads=1
```

`cargo insta review` is interactive and not usable from an agent, so use `pending-list`, read each
`.snap.new` diff, then `accept`.

### 5.8 Commits (T3)

1. **(a) red**: every §5.6 test (the snapshot ones fail with no snapshot); `Mode`, `Editor`,
   `Target` and `Field` declared; `captures_input`.
2. **(b) green, keys and replies**: §5.1, §5.3, §5.4.
3. **(c) green, render**: §5.5, the nine accepted snapshots, and the `:1565-1568` rewording.

---

## 6. Cross-task contracts

| Defined in | Item (exact) | Consumed by |
|---|---|---|
| T0 `htui_core::store::WriteStore` | `async fn set_agent_box_enabled(&self, agent_id: AgentId, box_id: BoxId, enabled: bool) -> Result<()>` | T2 `serve` |
| T0 `htui_core::model::AgentSummary` | `pub user_off: bool` (`#[serde(default)]`) | T2 D243, T3 `on_box_cell`/`t` |
| T1 `htui::agent_settings` | `FIELD_LABELS`, `AgentDraft`, `Refusal`, `DraftFields`, `valid_name`, `parse_*`, `format_args`, `format_models`, `draft_from_fields`, `merge_launch`, `apply_draft`, `new_agent`, `draft_of`, `launch_changed` | T2, T3 |
| T2 `htui::agent_settings` | `AgentWrite`, `REQUEST_NAMES: [&str; 3]`, `serve` | T2 `try_serve`, T3 |
| T2 `htui::store_worker` | `StoreRequest::{CreateAgent { name, draft, settings_from }, EditAgent { agent_id, expected, draft }, SetAgentOnBox { agent_id, enabled }}`; `StoreReply::AgentWritten { agents, outcome }` | T3 |

**Byte-exact strings**: the §3.2 refusal reasons; `REGISTRY_ON_SERVER_ONLY` (existing);
`` agent id `<id>` is already stored; nothing was written ``; F-23's sentence; §5.1's constants;
§5.3's refusals; §5.4's notices; the §2.8 comment.

---

## 7. Count pins, merge order and the workspace gate

| Pin | Now | After | Task |
|---|---|---|---|
| Store `CASES` | 96 | 97 | T0 (`conformance.rs:44`, `mem_store.rs:37`, `pg_conformance.rs:19`); `HANDOFF.md:42` is the main thread's |
| Migrations | `0001`..`0007` | `0001`..`0008`, next `0009` | T0; `HANDOFF.md:36-39` main thread |
| Pinned commented columns | 34 | 35 | T0 |
| `.sqlx` files | 288 | 289 (2 ` D`, 3 `??`) | T0 |
| `StoreRequest` / `StoreReply` | 85 / 47 | 88 / 48 | T2 |
| `agent_settings::REQUEST_NAMES` | — | 3 | T2 |
| Snapshots | 107 | 110 | T3 |
| `htui-orch` `CASES`, `GraphSource`, `MIRRORED_TABLES`, Settings sections, `TABLES` | 73, 7, 21, 7, 39 | unchanged | — |

[T0 ∥ T1] → merge T0 (core, store, agent, `htui --test settings`; workspace check) → merge T1 (`htui
--lib agent_settings`; clippy `-p htui`; workspace check) → T2 → merge (htui; `agent_settings_pg`
runs) → T3 → merge (htui; snapshots 110) → workspace gate → main thread updates the `HANDOFF.md`
pins.

```bash
df -h /
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
# .sqlx: §0.1 recipe, --check only
ls crates/htui-store/.sqlx | wc -l                   # 289
ls crates/htui/tests/snapshots | wc -l               # 110
cargo doc --workspace --no-deps --keep-going         # exactly the six baseline errors (HANDOFF.md:49-53)
git diff --stat b5e481f -- crates/htui-orch crates/htui-store/cache_migrations \
  crates/htui-agent/src/probe.rs crates/htui-agent/src/launch.rs crates/htui-core/src/fixtures.rs   # empty
rg -n 'impl.*WriteStore for' crates/htui crates/htui-orch   # nothing
```

---

## 8. Decisions (D249 onward) and risks (R-11 onward)

| # | Decision |
|---|---|
| D249 | The switch's conflict arm is `enabled = $3 AND (probe IS NULL OR COALESCE(probe->>'status' = 'ready', false))`; MemStore's `probe_says_ready` mirrors it, `Some(Null)` and status-less included (F-1). |
| D250 | `agent_settings::serve` answers a draft or name refusal as `Ok(Failed)` with the section's exact sentence; a create's `Stale` is `Failed`; store errors propagate as `Err` (F-8). |
| D251 | MemStore holds `user_off` as `agent_boxes_off: HashSet<(AgentId, BoxId)>`, initialised in the one `State` literal; the setter inserts a bare row stamped `now` and bumps `updated_at` on an update (F-2). |
| D252 | The new conformance case uses `(ids::AGENT_AGY, ids::BOX)` over its fresh demo store (F-19), and its doc delegates to two named read-backs, both defined in commit (a) (F-4). |
| D253 | New Postgres read-backs use runtime `sqlx::query*` or `PgStore::agents()`, never a macro, so `.sqlx` moves by exactly three statements (F-5). |
| D254 | T1's module carries `new_agent`, `draft_of`, `launch_changed`, `DraftFields`, `Refusal` and `FIELD_LABELS`, so T2 and T3 parse `launch` through one implementation (F-16). |
| D255 | `Refusal`'s `Display` is `` `<field>`: <reason> ``; the field is a `FIELD_LABELS` entry or `launch`, and the section focuses it. |
| D256 | `AgentWrite::Switched`'s `name` comes from the re-read; a row the re-read lacks is `Gone`. |
| D257 | The editor holds its own id, name, token, opened draft and sent draft; nothing indexes `agents` by cursor while it is open (F-14). |
| D258 | `busy` refuses `Enter`, `n`, `e`, `t`, `r`, `i` and `a`, and clears only on `AgentWritten` or a `Failed` named in `REQUEST_NAMES` (F-20). |
| D259 | `hint()` returns keys and an optional note; the note line always takes one row, and is `theme.error` under `is_error` (F-12). |
| D260 | A new `cli` row without `settings.cli` gets a data-only notice clause; `adapter_id_from` stays private (F-13). |
| D261 | `captures_input` is `matches!(mode, Mode::Editing(_))`, and one test drives it through `SettingsTab` (F-11). |
| D262 | Every task's gate starts with `cargo fmt --all -- --check` and runs `--test-threads=1` (F-18). |

| # | Risk | Likelihood | Mitigation |
|---|---|---|---|
| R-11 | A future probe document shape without `status` would read as "not ready" after a switch-on. | Low | D249 is explicit, and the conformance case pins it; `r` rewrites the probe and the row. |
| R-12 | The chat tab's agent picker does not see a created or edited row until its own next `Agents` read, because `AgentWritten` is addressed to the Settings tab only. | Low | The chat tab reads the registry on activation; a stale picker costs a re-open, never a wrong write (the chat start re-reads the row). |
| R-13 | `n` on a highlighted `cli` row copies a `settings.cli` block into a row that may speak a different stream. | Low | The header names the source row (OQ-5), and the row is editable only by SQL for `settings` (OQ-4); the first chat fails loudly by adapter id. |
| R-14 | A create's `Utc::now()` stamps nanoseconds on MemStore and microseconds on Postgres. | None | Tokens are only ever read back from a reply (MOD-40 F-17); no test compares a built stamp. |

`HANDOFF.md` (the pins, the MOD-23 line), `docs/**` (ANA-4 §4.6 / MOD-2 D50 note for OQ-1, the
OQ-3 follow-up item, ANA-21 item 8 for OQ-4) and `DECISIONS.md` are the main thread's.
