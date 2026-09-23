# MOD-4 — Orchestrator, manual mode

> Routed as **PRD** by `/handoff-run MOD-4` (criteria C2, C3, C4 fired). Ultracode recommended for
> the implement and review phases; the maintainer **scoped it to implement only**. The contract is
> concluded in `docs/ANA-2.md` (step graphs, three compare-and-set status tables, `htui-orch`) and
> amended by `docs/ANA-5.md` §4.6/§8 (the judge and handoff prompts, the review-loop forwarded set)
> and by MOD-25 (`docs/decisions/mod/mod-25.md`, which struck ANA-10's M8 from this item). Where
> those disagree, the decisions at the PRD gate below settle it. Requirement IDs: `R-ORCH-1..5`,
> `R-ORCH-7..11`, `R-TUI-4`, `R-TUI-9`, and the `run` and `close` actions of `R-TUI-2`.

## Problem

`htui` is a TUI for running a backlog of work items through step graphs, and it cannot run one. The
entity model has carried `run`, `run_step`, `run_step_commit`, `step_graph` and `step_graph_phase`
since `0001_init.sql`; MOD-15 made every graph and phase creatable and editable from the Settings
tab; MOD-2 shipped three agent transports that pass one conformance list, a prompt assembler that
knows how to build a judge prompt and a handoff prompt, and a recorder that persists and replays
every event. **Nothing walks a graph.** There is no engine, no crate to put one in, and no store
method that starts a run: grep for `finish_step` across `crates/` returns nothing, and the only
`run` rows any code writes are MOD-2's free-standing chats (`start_chat_run`, `finish_chat_run`).

The consequence is visible in three places. The Runs pane renders a four-column table and answers
exactly one key — `Enter`, which replays a stored transcript (`detail/runs.rs:200-213`); **all seven
`R-TUI-4` actions are absent**, and `backlog/mod.rs:3-5` says so in a comment. The Settings ▸ Kinds
editor renders eight phase columns read-only under the label `MOD-4 owns these`
(`settings/kinds.rs:94`), because nothing consumes them. And `WriteStore::set_step_prompt` — the
method that writes a step's `prompt_digest` before its session starts — **has no production caller
at all**; its offline arm refuses with a comment naming this item as the first code that will reach
it (`writer.rs:325-333`).

The gap also holds four other items still. MOD-11 and MOD-12 are blocked on this item by name.
MOD-27 presumes `htui-orch` exists. ANA-5's validation criteria 3, 16 and 18 were handed over here
by MOD-2 (D107). And `htui_core::model::quota::available()` — the function that decides which agent
a step gets — is documented "**no production caller**: MOD-4's selection loop is the consumer"
(`quota.rs:416-421`).

## Evidence

Every fact was read out of this tree on 2026-09-17/18 during PRD research, at HEAD `0cf232d`.
Nothing is recalled, and three of ANA-2's own citations are corrected below because the tree moved
under them.

- **No orchestrator exists, at any layer.** `crates/` holds exactly four members — `htui-core`,
  `htui-store`, `htui-agent`, `htui` (`Cargo.toml:2`). `crates/htui/src/run_worker.rs` does not
  exist. `grep -rn "Isolator"` over every `.rs` returns **prose in analysis docs only**; the trait
  reserved at `docs/ANA-2.md:1769` has no code. `htui-orch` appears in the tree solely as a doc
  comment (`prompt/mod.rs:6`).
- **No git library, and no shelling to `git`.** `gix`, `git2` and `libgit2` are absent from every
  `Cargo.toml` and from `Cargo.lock`. `grep` for `Command::new("git")`, `git worktree`, `git
  rev-parse` across `crates/` returns two prose comments and no code. The only diff machinery is
  `similar`, declared with the note "no crate consumes it yet". `gix` is the settled choice
  (`docs/ANA-2.md:2055`, `docs/decisions/ana/ana-2.md:109`) and enters at build step 5.
- **`can_move_to` does not exist and today's `transition` accepts any pair.**
  `WriteStore::transition(id, from, to)` (`traits.rs:150`) is a compare-and-set on `status` with no
  legality check; the conformance suite **actively pins** that arbitrary jumps are allowed, round-
  tripping `Open→Done` and `Done→Open` (`conformance.rs:537`, `:558`, `:580`, `:596`, `:649`,
  `:872`). Adding §4.3's three tables therefore changes the meaning of a shipped method and touches
  cases that currently assert the opposite.
- **The seam's real cost, counted.** `WriteStore` is **42** methods today (`traits.rs:139`),
  `ReadStore` **11** (`traits.rs:63`). A new `WriteStore` method lands on **six** impls — `MemStore`,
  `PgStore`, `Writer`, `BufferedWriter`, `UsageSpy`, `SpyStore`; a new `ReadStore` method on
  **eight**. `CacheStore` implements `ReadStore` only. **`BufferedWriter`'s impl must stay
  exhaustive**, so each of the 18 new methods needs a refusing arm even though MOD-25 left nothing
  constructing it (`writer.rs:66-71`).
- **Conformance is the dominant per-method cost.** `CASES` is **36** (`conformance.rs:31-68`),
  `READ_CASES` **6**, pinned by `EXPECTED_CASES = 36` (`pg_conformance.rs:19`) and twinned in
  `mem_store.rs:35-46`. The Postgres runner creates and drops **one database per case** — 42
  create/drop cycles per run today (`pg_conformance.rs:35-42`, `:61-68`). MOD-15 plan D12 refused a
  case-per-method for exactly this reason and grouped by entity.
- **ANA-2 §8's three conflicting counts, resolved by reading the table.** `:1739-1755` is **17
  markdown rows carrying 18 distinct method names** — the row at `:1744` names both
  `transition_run` and `transition_step`. The prose "sixteen" at `:1760` undercounts its own table.
  Separately, the inherent-read block `:1711-1724` is **14 rows carrying 16 method names** (`:1717`
  and `:1719` each name two), so `HANDOFF.md:184`'s "fifteen new inherent reads" is also short. The
  authoritative figures are **18 writes, 6 mirrorable reads, 16 inherent reads**.
- **Most of what `0003` needs already exists.** `step_graph_phase` already carries `fan_out`,
  `gate`, `gate_hard`, `retry_limit`, `input_kinds`, `isolation`, `command_queue` and
  `verify_command` (`0001_init.sql:228-248`); `run` already carries a nullable `graph_snapshot`
  (`:457`); `run_step` already carries `attempt`, `fanout_index`, `selected` and `isolation_path`
  (`:476-490`). `0003` adds three phase columns, four `run` columns, three `run_step` columns,
  `step_graph.is_override`, the `run_step_tree` table and twelve `app_setting` rows. **Nothing is
  dropped, renamed or retyped.**
- **The cache companion is `0003`, not ANA-2 §9's `0002`.** `cache_migrations/` holds
  `0001_mirror.sql` and MOD-2's `0002_agent_mirror.sql`; `0002_agent_probe.sql:6-8` states the
  correction in its own header.
- **The migration's blast radius on tests is concentrated in four assertions.**
  `migrations.rs:69-73` pins `applied == vec![1, 2]`; `:94-98` pins "the migration creates the 32
  tables and nothing else" (`run_step_tree` is a 33rd); `:258-260` pins "**exactly the six commented
  columns, and no others**" against `0003`'s ~13 new `COMMENT ON COLUMN`s — the largest single
  finding; `:353-357` and `connect.rs:95` pin `MigrationState::Pending(2)`.
- **A schema bump rebuilds every mirror.** `cache_meta.schema_version` is `PgStore::schema_version()`
  = the max embedded migration (`pg/mod.rs:389-391`), compared in `CacheStore::open` (`cache/mod.rs:140-142`);
  a mismatch **deletes and recreates the file** (`:144-151`). `cache.rs:1304-1335` hard-codes `2`
  twice. The mirror's own table list is `MIRRORED_TABLES: [&str; 16]` (`cache/mod.rs:41-58`) plus a
  ten-const cursor list in `refresh.rs:200-210` — which has **two more call sites ANA-2 does not
  cite**, an array literal at `:254-265` and a match at `:271-282` — and the Settings confirmation
  copy hard-codes "the 16 mirrored tables" (`settings/connection.rs:136`).
- **Two of ANA-2's four fixture corrections already landed with MOD-15.** `review` is in
  `implement.input_kinds` (`seed.rs:87-91`) and `gate_hard` is set on exactly three phases
  (`seed.rs:65-86`). Still outstanding: `graph_snapshot: None` on both graph runs
  (`fixtures.rs:1234`, `:1251` — ANA-2 cites the stale `:1113`/`:1130`) and `attempt: 0` on five
  step rows (`fixtures.rs:1296`, `:1330`), which contradicts the DDL default of 1. **The fixture
  phases *are* the seed rows**, asserted by `the_fixture_phases_are_the_seed_rows`
  (`fixtures.rs:1692-1729`), so a seed change and a fixture change are one change.
- **The prompt half is already built and waiting.** `PromptSpec` carries `judge`, `handoff`,
  `verify_failure` and `previous_diff` fields (`prompt/mod.rs:104-110`), `Some` iff the role
  matches; `DEFAULT_TEMPLATES` seeds ten bodies including `judge` and `handoff`
  (`defaults.rs:219-230`), whose machine-read parts — the verdict JSON block (`:188-193`) and the
  `review` front matter — are documented as **parsed by MOD-4**. `preview.rs:61-97` lists the eight
  `STAND_INS` this item replaces with real `run_step` values.
- **The quota skip that decides whether manual mode works on this box.** `available()`
  (`quota.rs:422-456`) skips any candidate whose status is present and not exactly `"allowed"`
  (`:439-443`). This box's live `claude-cli` blob is `status: "allowed_warning"` at 0.77
  utilization (`HANDOFF.md:71-74`). The doc at `:409-414` names MOD-4 as the owner of loosening it
  "knowingly rather than by accident".
- **`DriverCaps` makes the gate interlock bite immediately.** `Default` is all-false
  (`driver.rs:322-323`), and the **CLI transport declares `permission_requests: false` and
  `edit_proposals: false`** (`registry.rs:179-181`). §4.2's stage-1 interlock therefore skips a CLI
  agent for any phase whose `gate_effective != 'never'` — on a box whose only configured agent is
  `claude-cli`, that is every seeded gated phase.
- **The worker pattern to copy is one enum and one `continue`.** `Served { Reply, Deferred, Start }`
  (`agent_worker.rs:257-271`) is consumed in exactly one place (`store_worker.rs:1281-1291`), whose
  comment states the rule: "`Served::Deferred => continue` is the whole of `R-NF-3`". `StoreRequest`
  has **51 variants** today and forbids a secret as a plain `String` (`:65-78`). Channels are
  unbounded both ways by decision (MOD-1 D4).
- **The Runs pane renders none of what `R-TUI-4` asks for.** `runs.rs` is 610 lines rendering four
  columns; `agent_id`, `model`, `gate_outcome`, usage and duration are never read — `GateOutcome`
  has **zero occurrences** in all of `crates/htui/src`. `RunStepSummary` is 13 fields built in
  **three** places that must stay in step (`pg/rows.rs:125-141`, whose `:114-120` warns that new
  `query_as!` fields must be **appended** positionally; `mem.rs:828-845`; `cache/read.rs:537`).
- **The sub-tab strip is already two columns over.** `" Body "+" Runs "+" Graph "+" Documents "+" Notes "+" Prompt "`
  = **45** columns against a **43**-column inner pane at the pinned 100×30, verified in the recorded
  frame: `backlog__detail_runs.snap` line 8 ends `Promp`. 15 `backlog__*.snap` files carry the clip;
  the fix is MOD-30's, and MOD-30 is open.
- **No queue overlay, and two overlays in total.** `WorkspaceSwitcher` and `MigrationPrompt`
  (`app/mod.rs:60-73`). The reusable pieces MOD-4 inherits are `TextField` (`ui/text_field.rs`, 616
  lines, with `masked()` and a `Debug` that never prints the buffer) and MOD-15's two-stage delete —
  count first from the store, then a typed confirmation, with every unlisted key swallowed
  (`settings/hierarchy.rs:177-207`, `:640-735`).
- **Build and test conventions are unwritten but uniform.** No `CLAUDE.md` and no pre-commit hook
  exist; the gate is `cargo test --workspace --all-features`, `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-features --all-targets -- -D warnings` (`README.md:469-473`) plus
  `cargo doc --workspace --no-deps` and `cargo sqlx prepare --check` from inside `crates/htui-store`.
  `unsafe_code = "forbid"` (`Cargo.toml:105`), MSRV 1.98 against an exactly pinned 1.98.1 toolchain,
  edition 2024. **A new crate must repeat `[lints] workspace = true` or it silently loses the lint
  set.** The reviewer is `rust-reviewer` (`.claude/workflow-config.json:2`). Current suite: **1271
  passed, 0 failed, 30 ignored**, with Postgres live and at `--test-threads=1`.

## Users

- **Primary**: the maintainer running one item through its graph, by hand, on this box — selecting a
  `FEAT` item in the Backlog, pressing `run`, watching `prd → plan → implement → review` execute,
  answering each gate, and closing the item out. Today that whole sequence exists only as a
  requirement.
- **Also served**: the maintainer who has just configured a step graph in Settings ▸ Kinds and has
  no way to execute it; and the one whose review rejected an implementation and wants the loop run
  rather than re-prompted by hand.
- **Also served, by name**: MOD-12 (auto mode is this item's admission transaction with a queue in
  front of it), MOD-11 (`document_write`, `command_run`, `item_status` all land on methods this item
  defines), MOD-27 (`RunKind::Swarm` needs `htui-orch` to exist), MOD-7 (whose `repo_box_path` rows
  and real probe turn this item's isolation roots and capability check from degraded into complete).
- **Not for**: unattended execution, the queue, ready-item selection or batch caps — MOD-12. Not the
  MCP tool surface — MOD-11. Not remote dispatch — `R-ORCH-12` stores the target box and executes
  only locally. Not item editing or `touched_paths` authoring — MOD-13, whose editor is what makes
  the overlap rule useful.

## Hypothesis

We believe **a step-graph engine in its own crate, eighteen store methods under three compare-and-set
status tables, real git isolation, and the seven Runs-tab actions** will **turn `htui` from a tool
that configures pipelines into one that runs them** for **a maintainer driving one item at a time on
one box**.

We'll know we're right when **a `FEAT` item runs `prd → plan → implement → review` end to end
against a real agent in real worktrees, each gate answered in the TUI, a rejected review loops back
to `implement` with the review attached, and close-out writes a summary document carrying one
`before_hash..after_hash` row per (repo, step) — with no SQL, no shell, and no step the orchestrator
cannot resume after the process is killed.**

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| A graph runs end to end | `prd → plan → implement → review` produces four steps, four documents and `open → queued → in_progress → done` | ANA-2 §12 criterion 1, against `FakeDriver` + `FakeIsolator` |
| A run is decided by its snapshot, never by the live graph | Editing a phase mid-run changes nothing about that run; a topology-hash mismatch refuses to resume | criteria 2, 3 |
| Every status move is legal or refused | Every pair outside §4.3's three tables returns `StoreError::Constraint`; a stale `from` returns `Ok(false)`; `version` never moves; `MemStore` and `PgStore` answer identically | criterion 4 |
| The review loop terminates | A rejection re-runs `implement` at `attempt = 2` with the review resolved as an input; a second rejection escalates; two identical `after_hash` values stop the loop early | criteria 6, 7 |
| The judge never silently decides | Verification prefilter first; two reversed orderings; disagreement or an unparseable block parks at `awaiting_approval` with every candidate `done` and `selected` NULL | criteria 8, 9, 10 |
| Isolation is real and reversible | A two-repo `worktree` step writes two `run_step_tree` and two `run_step_commit` rows outside every `repo_box_path`; cancelling removes every tree and leaves no `htui/` branch checked out | criteria 11, 13 |
| A dirty tree is never reset | A `local` step on a dirty tree records `dirty = true` and the recovery sweep parks instead of resetting | criterion 12 |
| Overlap serialises, isolation parallelises | Two items declaring `src/**` in one repo never run concurrently; the same two in different repos do; a third run is refused at `max_concurrent_items = 2` while an `awaiting_approval` run holds no slot but still blocks an overlap | criteria 15, 16 |
| A killed orchestrator loses no finished work | A step with `after_hash` and its output document is adopted `done`; one with neither is reset and retried; no orphaned agent process either way | criterion 18 |
| Promotion keeps one identity | The promoted step keeps its id, appends `follow_up` events at an incrementing `turn`, sets `promoted_at`, and creates **no** `run(kind='chat')` row; `prompt_digest` is unchanged | criterion 17, ANA-5 criterion 18 |
| Close-out is a transaction, not a report | One `summary` document with a row per (repo, step), item `closed`, `closed_at` set, refused while any run is non-terminal | criterion 20, `R-TUI-9` |
| The Runs tab is complete | All seven `R-TUI-4` actions reachable from `on_key`, each rendering actual state on a compare-and-set miss; the step list shows agent, model, gate state, usage and duration | criterion 21 |
| Nothing regresses offline | A box with no server browses read-only and starts no run; every new seam method refuses with MOD-25's one sentence | `R-STO-4`; `writer_buffered.rs`'s one-sentence assertion |
| The schema moves exactly once | `0003_orchestration.sql` + `cache_migrations/0003_orchestration.sql`, nothing dropped or retyped, `cargo sqlx prepare --check` green | `migrations.rs`, `.sqlx/` regeneration |

## Scope

**MVP** — a manual-mode orchestrator: a new `htui-orch` crate holding the engine, the eighteen store
methods and three status tables beneath it, the migration pair, real git isolation over `gix`,
fan-out with a judge, overlap admission and crash recovery, and the Runs tab that drives all of it.

Concretely in scope:

- **The status law** (`R-ENT-8`, ANA-2 §4.3): `can_move_to` as a `const fn` on `Status`,
  `RunStatus` and `StepStatus`; `transition` gaining a `StoreError::Constraint` on an illegal pair
  while keeping its `Ok(false)`-on-stale-`from` and never bumping `version`; `transition_run` and
  `transition_step` as its two new siblings.
- **Eighteen `WriteStore` methods, six mirrorable `ReadStore` reads and sixteen `Backend`-inherent
  reads** (ANA-2 §8), each on `MemStore` and `PgStore` — **two stores, not three** (MOD-25 struck
  ANA-10's `LocalStore`) — with conformance cases grouped by entity rather than by method, and a
  refusing `BufferedWriter` arm apiece.
- **Typed settings and projections**: `BoxSettings` / `ProjectSettings` with `#[serde(default)]` on
  every field and **key-level merge, never a whole-blob round trip**; `touched_paths` on
  `ItemSummary`; `probed_tags`, `declared_tags` and `settings` on `BoxInfo`; `usage`, `selected`,
  `exit_code`, `verify_outcome`, `promoted_at` and a denormalised `agent_name` on `RunStepSummary`.
- **`0003_orchestration.sql` and `cache_migrations/0003_orchestration.sql`**: three phase columns,
  four `run` columns, three `run_step` columns, `step_graph.is_override`, the `run_step_tree` table,
  twelve `app_setting` defaults, and the mirror's eleventh cursor-driven table. **Never
  `CHECK (fanout_index >= 0)` on either schema.**
- **`htui-orch`**, the fifth workspace crate: `graph.rs` (snapshot, topology hash, field chains,
  override clone), `status.rs`, `engine.rs` (the six-stage walk), `gate.rs` (settle outcome, gate
  table, the `R-ORCH-3` loop and its no-progress predicate), `fanout.rs` and `select.rs`,
  `isolate.rs` and `verify.rs`, `overlap.rs` and `recover.rs`, `command.rs`, plus `fake.rs` and
  `conformance.rs` behind `test-support`. `queue.rs` is MOD-12's and is not written here.
- **The `Isolator` seam and all four `R-ORCH-8` modes**: `worktree`, `copy` (with `copy_exclude`
  mandatory and a measured-size refusal above `copy_max_total_bytes`), `shared_serialized` (a
  Postgres advisory lock per `(box, repo)`) and `local` (refused at `fan_out > 1`). Trees live under
  a scratch root validated against every `repo_box_path`; branches are `htui/<step_id>`; every git
  write retries three times at 200/400/800 ms on `index.lock` and ref-lock errors; `git gc
  --prune=now` is forbidden while any run is non-terminal.
- **Fan-out, the judge and selection**: candidate steps at `fanout_index 0..n-1` and the judge at
  `-1`, the verification prefilter, the two-order judge call through `assemble()` with the seeded
  `judge` template, the JSON verdict block parsed and never prose, and one selection transaction
  writing winner, losers and the judge's note.
- **Agent selection** (`R-AGT-8`): the ANA-4 predicate plus MOD-4's three skip conditions, the
  `phase_agent → project default → sole enabled agent → refuse` fallback, and `no_candidate_agent`
  written as a `blocked` item with an `item_note`.
- **Overlap, admission and the lease**: `RunScope`, the matcher-free `PathPrefix` rule, the
  `SELECT … FOR UPDATE` admission transaction counting `running` runs only, the per-process
  `lease_owner` with its zero-row-means-abandon refresh, and the recovery sweep's artefact test.
- **`crates/htui/src/run_worker.rs`** plus the `StoreRequest::Orch` / `RunStream` pair, serving
  every orchestrator command through the `Served::Deferred` pattern, holding no store handle on the
  UI task.
- **The Runs tab** (`R-TUI-4`): all seven actions plus `Unblock`, `AcceptArtifact` and `CloseOut`,
  each a compare-and-set that renders the actual state on a miss; the step list showing agent,
  model, gate state, usage and duration; close-out (`R-TUI-9`) behind the two-stage confirmation
  MOD-15 established; the `run` and `close` actions of `R-TUI-2`.
- **The three hand-overs MOD-2 named**: ANA-5 criterion 18's persistence half (a handoff prompt is a
  `follow_up` event at the next `turn`, leaving `prompt_digest` untouched), criterion 3's `blocked`
  transition, and criterion 16's `review` front-matter parser — three lines, unparseable read as
  `request-changes` with a note.
- **Fixture corrections**: `attempt` 1-based on five rows, non-NULL `graph_snapshot` on both graph
  runs, and the `ck_run_graph_snapshot` constraint left `NOT VALID` as ANA-2 specifies.

**Out of scope**

- **Auto mode, the queue, `ready_items`, batch caps, escalation surfacing and the scheduler
  window** — MOD-12, which adds a caller and no machinery. `queue.rs` is not written here.
- **The MCP tools** — MOD-11. `document_write` is defined as a store method here and called by
  MOD-11 later; `command_run` is executed directly by the orchestrator in v1, honouring the class
  limit with the same in-process semaphore.
- **Task fan-out** (N prompts, disjoint file sets, every result merged) — a `docs/REQUIREMENTS.md`
  change, not a MOD-4 call (D2).
- **Remote dispatch** — `R-ORCH-12` v1 stores `target_box_id` and refuses a run that is not local.
- **The excerpt provider seam and F-104's loosening** — deferred (D4); nothing registers a provider.
- **The sub-tab strip overflow** — MOD-30, landing before milestone 7 (D5).
- **Splitting `AgentRuntime::background` by what a task writes** — MOD-31 (D7).
- **Scrubbing `trim_record`'s own strings** — MOD-32; this item adds no new unscrubbed write path.
- **Any second migration.** If a decision seems to need `0004`, it is the wrong decision.
- **Offline anything.** `htui` is online-only since MOD-25; every method here refuses offline with
  the existing sentence, and no run starts without a server.

## Constraints (fixed before planning)

- **Two stores oblige the seam**, `MemStore` and `PgStore`. No `LocalStore`, no
  `local_migrations/`, no local-only graph parity (MOD-25).
- **The migration set is exactly `0003_orchestration.sql` + `cache_migrations/0003_orchestration.sql`.**
  `0002` is applied and landed (`fb626a8`), so ANA-2's sequencing hazard is satisfied.
- **Nothing is dropped, renamed or retyped**, so `cargo sqlx prepare` regenerates rather than fails.
- **`run_step_tree` stays outside the `updated_at` trigger loop**, riding its parent step exactly as
  `run_step_commit` does.
- **Never `CHECK (fanout_index >= 0)`** on either schema — `-1` is the judge step (ANA-2 risk 12).
- **`project.settings` is merged key by key**, never serialized whole: MOD-15's writer and this
  item's readers share one blob.
- **The engine reads `ResolvedPhase` off `graph_snapshot`, never `step_graph_phase` at run time.**
- **`R-NF-3` is enforced by ownership**: the UI task holds no store handle; every orchestrator
  command is served off it.
- **`R-PRM-1` bounds what crosses a loop**: the review document, the verification failure and the
  previous diff. The previous session transcript never does.
- **`unsafe_code = "forbid"`, MSRV 1.98, edition 2024, workspace lint set unchanged — and the new
  crate repeats `[lints] workspace = true`. TDD per repo convention.**
- **Windows runtime facts defer to MOD-16**, which gains this item's `gix` behaviour, `copy` on
  NTFS, advisory-lock behaviour and path handling under a scratch root.

## Delivery Milestones

<!-- Business outcomes, not engineering tasks. /plan turns each into a plan. -->
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | The seam knows what a run is | Eighteen `WriteStore` methods, five `ReadStore` reads, **eleven** `Backend`-inherent reads (not sixteen: five of ANA-2 §8's already existed) and the three `can_move_to` tables, over `MemStore` and `PgStore`, on `0003_orchestration.sql` and its mirror companion. No engine, no git, no UI. | complete (`33277b1`..`e3163ca`, 2026-09-18) | [plan](../plans/mod-4-orch-seam.plan.md), [blueprint](../plans/mod-4-orch-seam.blueprint.md) |
| 2 | A graph walks | `htui-orch` exists and runs `prd → plan → implement → review` against `FakeDriver` and `FakeIsolator`: the six-stage walk, the gate table, the review loop and its no-progress predicate — with no git, no Postgres and no agent. | complete (`cacab31`..`0a53d9c`, 2026-09-19; close-out `f87c2b7`..`1cbddc5`, 2026-09-22) | [plan](../plans/mod-4-orch-engine.plan.md), [blueprint](../plans/mod-4-orch-engine.blueprint.md) |
| 3 | Work happens in a real tree | The `Isolator` over `gix`: four isolation modes, `run_step_tree` rows, before/after hashes per repo, winner reconciliation, cleanup, and `verify_command` with its three outcomes. | complete (`6307a3d`..`1b78753`, 2026-09-22) | [plan](../plans/mod-4-orch-tree.plan.md), [blueprint](../plans/mod-4-orch-tree.blueprint.md) |
| 4 | Three candidates, one winner | Fan-out with the verification prefilter, the two-order judge call, the selection transaction, and `R-AGT-8`'s walk with the three skip conditions and the empty-candidate fallback. | complete (`fa03782`..`7ee0ac6`, 2026-09-23) | [plan](../plans/mod-4-orch-fanout.plan.md), [blueprint](../plans/mod-4-orch-fanout.blueprint.md) |
| 5 | Two runs do not collide, and a crash is survivable | The overlap predicate, the admission transaction, the lease and its refresh, and the recovery sweep's artefact test — including the refusal to reset a dirty tree. | complete (`a1fb291`..`8dc4755`, 2026-09-23) | [plan](../plans/mod-4-orch-lease.plan.md), [blueprint](../plans/mod-4-orch-lease.blueprint.md) |
| 6 | The maintainer drives it | `run_worker.rs`, the seven `R-TUI-4` actions plus `Unblock`/`AcceptArtifact`/`CloseOut`, the step list with agent, model, gate, usage and duration, promotion to chat, and close-out. | pending | — |

**Six milestones, not the seven D1 approved** — amended while planning milestone 1, for a reason
that is structural rather than editorial: the store traits carry **no default bodies** (`writer.rs`'s
`BufferedWriter` impl is exhaustive by construction), so the first new trait method stops
`htui-store` compiling, and `PgStore` cannot implement one before `0003` exists because
`SQLX_OFFLINE` prepares against real columns. The original milestones 1 and 2 therefore cannot each
close on a green workspace gate; they are one milestone. ANA-2 §9's build order is unchanged — one
cut point is removed. See `.claude/plans/mod-4-orch-seam.plan.md` **D0**.

Milestones 1–2 need nothing from MOD-2 and touch no git (ANA-2 §9). Milestone 3 is where `gix`
enters the workspace and where Windows runtime facts start accruing to MOD-16. Milestone 6 re-records
the Backlog snapshots, and **MOD-30 lands before it** (D5) — milestone 1 also re-records exactly one,
`connection__confirm.snap`, because the mirrored-table count is rendered into the rebuild
confirmation copy.

## Decisions taken at the PRD gate

Answered by the maintainer on 2026-09-18, before planning ("all defaults"). Each rests on a fact
under Evidence.

- **D1 — Seven milestones, in ANA-2 §9's build order.** The eight build steps group into seven
  landable outcomes; the split is the document's, not an invention. Steps 1–4 are the half that
  needs nothing from MOD-2, which makes milestones 1–3 independently verifiable before any git or
  agent code exists.
- **D2 — ANA-2 §10's eight defaults are adopted as written.** Rival fan-out only in v1;
  `max_fan_out` 4 and `max_agents_per_run` 6; `max_concurrent_items` 2; `copy` offered on Windows
  with a measured size and a refusal above the cap; `gate_hard` on `feature.prd`, `feature.plan` and
  `analysis.verdict` only (already seeded by MOD-15); lease TTL 120 s refreshed at 60 s; `gix` over
  `git2`; `failed → closed` permitted without a retry. **Task fan-out is explicitly not built**:
  it is the maintainer's own practice but not what `R-ORCH-7` describes, and adding it is a
  requirements change. The isolation and overlap machinery both kinds would share is built here, so
  nothing is foreclosed.
- **D3 — `allowed_warning` is loosened, and MOD-4 is where it happens.** `available()` skips every
  status that is not exactly `"allowed"` (`quota.rs:439-443`), and this box's live `claude-cli` row
  is `allowed_warning` at 0.77 utilization. Unloosened, the first selection loop would skip the only
  configured agent on the maintainer's own box and manual mode would be unusable where it is being
  built. The loosening is narrow — that one status becomes selectable — and the `exhausted`,
  `utilization >= 1.0` and cap rules are untouched. `quota.rs:409-414` reserved exactly this call
  for this item, "knowingly rather than by accident"; the pinning test changes with it.
- **D4 — F-104 is not loosened here.** `vetted()` refuses any provider candidate the reader's
  listing never offered, and a `scan_cap`-truncated listing is only a prefix. Nothing registers an
  excerpt provider today (`excerpt.rs:1327-1328`: `merged` is empty in every path), so the
  conservative rule refuses zero real candidates. The note stays on MOD-4's line; the per-candidate
  stat is a later call made against a real provider rather than a hypothetical one.
- **D5 — MOD-30 lands before milestone 7, as its own item.** The strip is 45 columns against 43 and
  15 `backlog__*.snap` files carry the clip. Milestone 7 re-records those snapshots for the step
  list's new columns; landing MOD-30 afterwards would churn them a second time. MOD-30 is also a
  maintainer-choice item (separator width versus `DETAIL_PERCENT`) and is not silently absorbed.
- **D6 — All four isolation modes ship in milestone 4.** `R-ORCH-8` names four and all four are
  `must`. `copy`'s extra cost is a size measurement and an exclusion walk (~80 lines), and
  `shared_serialized`'s is one Postgres advisory lock; deferring either would leave a mode whose
  column values the schema already accepts and the engine would refuse.
- **D7 — MOD-31 is not folded in.** The install guard keys on `AgentRuntime::background`, which
  belongs to the chat runtime; `run_worker` is its own task set and never pushes into it. MOD-4
  neither fixes nor worsens the finding.
- **D8 — MOD-4 ships with two document producers, neither of them MCP.** `document_write` is
  defined here as a store method; MOD-11 calls it later. Until then the gate's `accept artifact`
  action and hand-written documents are the producers `R-ENT-12` already permits, and an ungated
  phase whose agent wrote nothing fails loudly with `missing_output`. The circularity is by design
  and the gate is the break (ANA-2 risk 4).

## Open Questions

- [ ] Whether `run_step_tree` should be mirrored for an offline Runs tab to render an isolation
      mode. ANA-2 §9 mirrors it; MOD-25 made `htui` online-only, so the only reader is a box that
      can reach its server anyway. The assumption is **mirrored**, matching ANA-2, at the cost of
      one mirror table nothing offline strictly needs.
- [ ] Whether a `copy`-mode step on a repo with no `.git` at all (ANA-2 names it as a reason the
      mode exists) is in v1 or refused. The assumption is **refused with a message**, since
      `before_hash` is `NOT NULL` and every other mode derives it from git.
- [ ] Which of `verify_command`'s outcomes a missing `command_run` row should produce once MOD-11
      lands. v1 runs it directly, so the question is dormant; the assumption is that MOD-11 changes
      the execution path and not the three outcomes.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Git contention across parallel worktrees loses committed work — measured elsewhere at eight of thirteen commits failing on lock contention, with an auto-cleanup then destroying the uncommitted remainder | High under fan-out | Highest in the item | Three retries at 200/400/800 ms on `index.lock` and ref-lock errors; `--lock` on every worktree; `git gc --prune=now` forbidden while any run is non-terminal; trees removed only when the run is terminal, with the no-commit worktree the single step-end exception |
| A dirty `local` or `shared_serialized` tree is reset by recovery and the maintainer's uncommitted work is gone | Medium | Highest | `run_step_tree.dirty` recorded at stage 2; the sweep **never** resets a dirty tree, parking with `interrupted, tree not reset` and naming the tree and `before_hash` |
| The judge — a model — decides which code ships, and position bias flips a median model's choice in 41% of decisive swapped-order pairs | High where configured | High | Verification prefilter first; two calls with reversed order; disagreement escalates to a human; `judge_agent_id IS NULL` (the seeded default) means human selection always |
| `0003` breaks four shipped test assertions at once — the applied-version vector, the 32-table count, "exactly six commented columns", and `Pending(2)` — and any miss reads as an unrelated failure | Certain | Medium | Enumerated in Evidence with line numbers; milestone 2 amends all four in the same commit as the migration, and `cache.rs`'s two hard-coded `2`s with them |
| The `schema_version` bump silently rebuilds every box's mirror on first launch after upgrade | Certain | Low, once | Documented behaviour (`cache/mod.rs:144-151`); the Settings rebuild copy is updated in the same milestone, including its hard-coded "16 mirrored tables" |
| Eighteen seam methods at a database create/drop per conformance case turn the Postgres suite into the slowest gate in the repo | High | Medium | Cases grouped by entity rather than by method, following MOD-15 plan D12; the count pin moves once per milestone, not once per method |
| `RunStepSummary` is built in three places and `pg/rows.rs` binds positionally, so an added field silently mis-maps | Medium | High — wrong data, no error | The file's own warning says append only; a conformance case asserts the three builders agree field for field |
| A CLI-only agent cannot serve a gated phase (`permission_requests` and `edit_proposals` both false), which on this box is every seeded gated phase | Certain on this box | High — the engine refuses before the first token | The interlock is ANA-2's and correct; the surfaced message names the missing capability, and the ACP transports advertise both. Recorded so the first live run's refusal is expected rather than debugged |
| The engine is six stages over four subsystems and lands as one 7-milestone item; a mid-item design change strands earlier milestones | Medium | High | Milestones 1–3 are UI-free, git-free and agent-free and are provable against `MemStore` + `FakeDriver` + `FakeIsolator`; the snapshot is the only contract later milestones read |
| `htui-orch` is added without `[lints] workspace = true` and loses `unsafe_code = "forbid"` and the clippy set silently | Low | Medium | Named in the constraints; the crate's first commit carries the manifest and the clippy line runs on it |
| An override graph clone deep-copies `skill_binding` and doubles every inherited project binding | Medium if unguarded | Medium | ANA-2 §4.1 is explicit that the clone covers `step_graph_phase` and `phase_agent` and **not** `skill_binding`; a test asserts binding counts after a re-override |
| Two `htui` processes on one box adopt each other's runs | Low | High | `lease_owner` is per process, the refresh is a compare-and-set on it, a zero-row refresh means abandon, and adoption requires an expired lease |

---
*Status: DRAFT — requirements only. Implementation planning pending via `/plan`.*
