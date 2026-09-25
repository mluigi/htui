# ANA-2 - Orchestration design: step graphs, phase contract, status machines, fan-out, isolation, resume

> **Scope note:** Design authority for how `htui` runs an item through a step graph: the step graph
> data shape and its semantics, the phase contract (inputs, output document kind, gate, retry,
> verification), the status machines on `item`, `run` and `run_step`, the review-to-implement loop,
> fan-out selection and the judge contract, the four isolation modes with per-repo commit capture,
> the overlap rule for concurrent items, promotion to chat, and resume after process death or an
> offline window. Governed by `.claude/rules/workflow-docs.md`, `CONCEPTS.md`,
> `docs/REQUIREMENTS.md` and `docs/ANA-9.md` (§4.2, §5.4, §5.5, §5.8, §5.9, §6.1, §7.4, §9).
>
> **Requirements addressed:** `R-ORCH-1..11`, `R-ENT-6`, `R-ENT-8`.
> Touched at their seams only, and settled elsewhere: `R-AGT-1..8` (ANA-4 supplies the driver this
> document schedules; §7 consumes its quota verdict verbatim), `R-PRM-1..3` (ANA-5 assembles the
> prompt from the inputs this document resolves), `R-SEC-2` and `R-SEC-4` (ANA-7 supplies the
> environment and the refusal this document turns into a status), `R-MCP-1..4` (MOD-11 supplies the
> `document_write` and `command_run` paths this document reserves), `R-ENT-9..12` (ANA-9 owns the
> row shapes; this document owns which step writes them), `R-TUI-4` and `R-TUI-9` (the action set
> and the close-out contract this document maps onto orchestrator commands), `R-ORCH-12..13`
> (`later`; the target-box column and the scheduler window key are reserved here, not built).
>
> **Status (2026-09-06): concluded.** Implementation tracked as MOD-4 (manual mode) and MOD-12
> (auto mode) in `HANDOFF.md`. One forward-only migration amendment to the ANA-9 schema, plus one
> cache-mirror amendment, is named in §9.

---

## 1. Context and problem statement

`docs/REQUIREMENTS.md` §6 fixes the contract: a step graph is a named, ordered list of phases with
a fixed per-phase column set (`R-ORCH-1`), a three-valued gate plus a hard flag (`R-ORCH-2`), a
review loop bounded by a retry limit (`R-ORCH-3`), manual and auto modes (`R-ORCH-4`, `R-ORCH-6`),
promotion to chat with resume (`R-ORCH-5`), fan-out with human or judge selection (`R-ORCH-7`),
four isolation modes (`R-ORCH-8`), an overlap rule with a per-box limit (`R-ORCH-9`), a capability
refusal that lists missing tags (`R-ORCH-10`), and an eleven-field run record (`R-ORCH-11`).
`docs/ANA-9.md` §5.4 puts every one of `R-ORCH-1`'s phase fields into `step_graph_phase` and then
hands this document the semantics, verbatim (`docs/ANA-9.md:495-496`):

> "The phase column set is exactly what `R-ORCH-1` enumerates; ANA-2 owns the semantics and amends
> by migration."

`docs/ANA-9.md:1009-1010` repeats the delegation and names the owner:

> "**MOD-4** (orchestrator) owns the status machines on `run`, `run_step` and `item.status`
> (compare-and-set, never version bumps); ANA-2 may add phase columns by migration `0002`."

This document settles the nine questions the `HANDOFF.md` ANA-2 item names, in its order:

1. the step graph data shape, and whether the ANA-9 §5.4 DDL suffices,
2. the phase contract: inputs, output document kind, gate, retry, verification command,
3. item status transitions (`R-ENT-8`) and the `run` / `run_step` status machines,
4. the review-to-implement loop (`R-ORCH-3`): attempt counting, what the loop carries, escalation,
5. fan-out selection: human when gated, judge otherwise (`R-ORCH-7`), and the judge contract,
6. isolation modes, especially `copy` and `local` (`R-ORCH-8`), with commit capture per repo,
7. the overlap rule for concurrent items (`R-ORCH-9`) and the per-box limit,
8. promotion to chat (`R-ORCH-5`) and resume from the next step,
9. resume of an interrupted run after process death or an offline window.

Plus the four extras the item names: the capability check (`R-ORCH-10`), run records
(`R-ORCH-11`), close-out (`R-TUI-9`), the auto-mode gate downgrade and caps (`R-ORCH-6`,
`R-AGT-7..8`), and the Runs tab actions (`R-TUI-4`).

Five premises that were current when the item was written are now false or stale, and every one of
them changes an answer. They are recorded here so the next reader does not re-derive them:

| Stale premise | State on 2026-09-06 |
|---|---|
| ANA-2's migration is `0002` (`docs/ANA-9.md:1010`) | Superseded. ANA-4 claimed `0002_agent_probe.sql` (`docs/ANA-4.md:1267-1273`). **ANA-2's migration is `0003_orchestration.sql`**, and it lands after MOD-2's `0002`, which does not exist on disk yet: `crates/htui-store/migrations/` holds exactly one file. §9 states the ordering rule. |
| `step_graph_phase` needs a column review | False. `docs/ANA-9.md:498-550` and `crates/htui-store/migrations/0001_init.sql:214-276` are byte-identical once comments and blanks are stripped, and the column set is `R-ORCH-1`'s enumeration item for item. The gap is semantics, not columns; §4.1 adds five columns and one table, and rewrites none. |
| A judge has somewhere to live | False. Nothing named `judge` exists in `docs/`, in the DDL or in `crates/`. `phase_agent` is documented as "candidate agents in priority order (`R-ORCH-1`, `R-AGT-8`)", an execution priority list, not a judge slot. §4.5 gives the judge a column and a step row. |
| `blocked` is a status something writes | False. `item.status` admits `'blocked'` (`docs/ANA-9.md:588`) and nothing in ANA-9, MOD-1 or MOD-6 ever writes it, while `docs/ANA-9.md:946-958` computes blockedness dynamically and filters `i.status = 'open'`. The two designs are mutually exclusive as written; §4.3 splits them. |
| A seeded project can run | False. `phase_agent` has no fixture row and no demo INSERT (`crates/htui-store/src/pg/demo.rs:24`), MOD-6 deliberately seeded no `agent` rows, and `phase_agent.model` is `TEXT NOT NULL` with a FK to `agent(id)`. Every seeded graph carries zero candidates today, so `R-AGT-8`'s priority list is empty. §7 fixes the fallback. |

Two further facts shape every answer and are stated once here rather than repeated.

**The persistence layer is complete and the behaviour layer is empty.** Every column `R-ORCH-11`
enumerates already exists; every enum `R-ORCH-2`, `R-ORCH-8` and `R-ENT-8` name is already a Rust
type whose variant order is asserted against the DDL `CHECK` list by a unit test in
`crates/htui-core/src/model/mod.rs`. What does not exist is a single writer: `WriteStore` is
`mint_item`, `update_item`, `transition` and a literal comment,
`crates/htui-core/src/store/traits.rs:50`:

```rust
// runs, steps, events, links, notes, documents, skills, templates, box, agents ...
```

`MemStore` already holds graphs, phases, templates and agents behind
`#[expect(dead_code, reason = "loaded now, read by MOD-2 / MOD-4 / MOD-15")]`. MOD-4's first commit
is a trait extension, and §8 names every method.

**Offline cannot advance a run, by construction and not by policy.** `Backend::writable()` returns
`Some` only for `Online`, there is deliberately no `impl WriteStore for Backend`, and
`crates/htui-store/src/backend.rs:6-9` records the reason: "a write attempt against an offline
store is a **compile error** rather than a runtime flag". §4.9 states what that means for a run
that is live when the connection drops.

---

## 2. Invariants

Restated from `CONCEPTS.md`, `docs/REQUIREMENTS.md` and `docs/ANA-9.md`; each has a mechanical
enforcement point in this design.

1. **One writer per status, and the write is a compare-and-set.** Every move on `item.status`,
   `run.status` and `run_step.status` is `UPDATE ... WHERE id = $1 AND status = $expected
   RETURNING *`; zero rows is a race, reported as such, never retried blindly, and it never touches
   `version` or the revision log (`docs/ANA-9.md:165-169`, `CONCEPTS.md`). Enforced by the store
   methods of §8 being the only write path and by the legality tables of §4.3 being `const fn`
   predicates in `htui-core` that the conformance suite exercises.
2. **The executing copy of a graph is `run.graph_snapshot`, never the live tables.** A phase edited
   mid-run does not affect a live run, and a resumed run refuses to resume against a graph whose
   topology hash changed. Enforced by the snapshot being taken in the same transaction that inserts
   the `run` row, by `graph_snapshot` becoming `NOT NULL` for `kind = 'graph'` (§9), and by the
   orchestrator reading phases only from the snapshot.
3. **No agent in a bookkeeping path.** Gate resolution, status moves, overlap admission, isolation
   setup, commit capture, selection bookkeeping and close-out are deterministic code (`R-ID-6`).
   The judge is the single exception `R-ORCH-7` grants, and it decides one thing only: which
   `fanout_index` wins. Enforced by the judge's output being an integer index plus a rationale
   document, never a status or a row.
4. **`htui` writes no files into a managed repository.** Worktrees and copies are created under a
   box-local scratch root outside every `repo_box_path` (`R-ID-4`). Enforced by `run_step_tree.path`
   being validated against every known `repo_box_path` prefix before creation, and by the
   `local` and `shared_serialized` modes being the only two that touch a managed tree, both of
   which run the agent there rather than writing `htui`'s own files.
5. **A gate stops only at a durable boundary.** A step reaches `awaiting_approval` only after its
   session has ended, its output document is persisted, its verification has run and its
   `after_hash` is captured. Enforced by the step lifecycle of §4.2 being ordered and by the
   recovery sweep of §4.9 using exactly those three artefacts as its completion test.
6. **A gate holds no lock and no compute slot.** An `awaiting_approval` run keeps its trees and its
   branches, so it still participates in the overlap predicate, but it releases the per-box
   concurrency slot. Enforced by §4.7's admission counting `status = 'running'` only, and by the
   overlap predicate ranging over non-terminal runs.
7. **Every refusal persists.** A capability mismatch, a secret-provider outage, an exhausted retry
   budget and a failed judge all leave a row a human can read (`R-ID-3`). Enforced by §4.3's stored
   `blocked` status plus an `item_note`, so no refusal exists only as a transient message.
8. **Fan-out losers are kept, and are invisible downstream.** Losing steps stay as rows with
   `selected = false` and `status = 'superseded'`, their documents stay, and no later phase can read
   them (`R-ORCH-7`). Enforced by the single `input_kinds` resolution query of §4.2 joining
   `document.produced_by_step_id -> run_step.selected IS NOT FALSE`.
9. **Resume is decided from the store, never from memory.** Position, attempt, tree state and
   completion are re-derived from `run_step`, `run_step_tree`, `run_step_commit` and `document` on
   every start. Enforced by the orchestrator holding no cross-restart state and by §4.9's lease
   being the only liveness marker.
10. **One cache writer, and the mirror is not the orchestrator's store.** The SQLite mirror is
    written only by the refresh task (`CONCEPTS.md`); MOD-4 reads and writes Postgres through
    `WriteStore` and never opens the mirror. Enforced by the orchestrator crate depending on
    `htui-core`, not on `htui-store`.

---

## 3. Surface as read

Read from the working tree on 2026-09-06, HEAD `2ec833d`.

**`step_graph_phase`, seventeen columns, exactly `R-ORCH-1`'s enumeration**
(`crates/htui-store/migrations/0001_init.sql:228-248` = `docs/ANA-9.md:509-529`): `id`, `graph_id`,
`position`, `name`, `fan_out` (`>= 1`, default 1), `gate` (`always|on_failure|never`, default
`always`), `gate_hard` (default false), `retry_limit` (`>= 0`, default 1), `input_kinds` (`TEXT[]`),
`output_kind`, `isolation` (nullable, `worktree|copy|shared_serialized|local`, "NULL = project
default"), `command_queue` (`off|fan_out_only|always`, default `fan_out_only`), `verify_command`
(nullable, no semantics attached anywhere), `template_name`, `template_version`, `token_budget`,
`updated_at`, with `UNIQUE (graph_id, position)` and `UNIQUE (graph_id, name)`.

**`phase_agent`** is `PRIMARY KEY (phase_id, position)` with `agent_id UUID NOT NULL REFERENCES
agent(id)` and `model TEXT NOT NULL`, header comment "candidate agents in priority order
(R-ORCH-1, R-AGT-8)".

**Item override** is already settled (`docs/ANA-9.md:552-553`): "the TUI clones the kind's graph
into a new `step_graph` row named `<key>-override` and points `item.step_graph_id` at it. No
per-item phase table."

**`run`** (`0001_init.sql:447-467`): `kind IN ('graph','chat')`, `mode IN ('manual','auto')`,
`status IN ('queued','running','awaiting_approval','done','failed','cancelled')` default `queued`,
`target_box_id NOT NULL`, `executing_box_id` nullable, `graph_snapshot JSONB` nullable,
`started_by NOT NULL`, `queued_at`/`started_at`/`finished_at`, `failure TEXT`.

**`run_step`** (`0001_init.sql:472-499`): `position` ("phase index in the snapshot"), `attempt`
(`NOT NULL DEFAULT 1`, "retry / review loop counter"), `fanout_index` (`NOT NULL DEFAULT 0`,
"0..fan_out-1"), `phase_name`, `agent_id`, `model`, `status IN
('pending','running','awaiting_approval','done','failed','cancelled','superseded')` default
`pending`, `gate_outcome IN ('approved','rejected','retried','skipped')` nullable, `gate_note`,
`selected BOOLEAN` ("fan-out winner; NULL when fan_out = 1"), `exit_code`, `prompt_digest`,
`trim_record`, `usage`, `isolation_path TEXT` ("worktree / copy location on the executing box"),
timings, `UNIQUE (run_id, position, attempt, fanout_index)`.

**`run_step_commit`** is `(run_step_id, repo_id, before_hash TEXT NOT NULL, after_hash TEXT)` with
`PRIMARY KEY (run_step_id, repo_id)`, header comment "per repo, since a project may have several".
It carries no `updated_at` and is deliberately outside the trigger loop; the refresher rides it on
its parent step (`docs/decisions/mod/mod-6.md` §5).

**`item`** carries `status` (the eight `R-ENT-8` values), `required_tags TEXT[]` with a GIN index,
`touched_paths TEXT[]` ("R-ORCH-9 declared overlap set, repo-relative globs", no index),
`priority`, `step_graph_id` (nullable, "NULL = kind default"), `version`, `closed_at`.
`Status::is_terminal()` is `Done | Closed` only (`crates/htui-core/src/model/item.rs:30-36`), and
its doc names the reason: "a `blocked_by` edge to a terminal item no longer blocks".

**The one shipped status writer** is `WriteStore::transition(&self, id, from, to) -> Result<bool>`
(`crates/htui-core/src/store/traits.rs:49`). Its Postgres body sets `closed_at` following the
terminal status in both directions, returns `Ok(false)` on a `from` mismatch and `NotFound` on a
missing row (`crates/htui-store/src/pg/write.rs:228-269`). **It takes no legality table**: any
`(from, to)` pair whose `from` matches is accepted. There is no `Status::can_move_to` anywhere.

**The store-side half of readiness is implemented and the box half is not.**
`ItemFilter.ready` implements `docs/ANA-9.md` §7.4 minus the two box clauses and minus the queue
ordering (`crates/htui-store/src/pg/read.rs:96-102`); its doc-comment is explicit:
"the capability half is expressed through `tags` by the caller; **MOD-4 owns matching against a
real box**" (`crates/htui-core/src/model/item.rs:137-140`).

**Settings are JSONB comments, not types.** `box.settings` is documented as "command_limits
{class: n} (R-MCP-3), max_concurrent_items (R-ORCH-9)" (`0001_init.sql:68`) and typed as an untyped
`serde_json::Value`; `project.settings` is documented as "token_budget, retention_days,
cached_transcript_steps, keep_raw_events, default_isolation, per_token_cap_run,
per_token_cap_batch" (`0001_init.sql:150-151`), likewise untyped. `SEEDED_SETTINGS` in
`crates/htui-store/src/pg/mod.rs:44-48` inserts exactly two `app_setting` keys, both cache-related.
`BoxInfo`, the only box projection the TUI has, carries `box_id`, `hostname`, `os_family` and
nothing else.

**Nothing behavioural exists.** No orchestrator crate, no git library (`git2` and `gix` are absent
from `Cargo.lock`), no process spawn (`tokio` is built without the `process` feature), no glob
matcher, no `WriteStore` method that touches `run` or `run_step`, and no Runs-tab action:
`RunsTab::on_key` is scroll-only with an unused `_ctx`, and the step list is fetched and dropped.

**The mirror carries the run tables and not the graph tables.**
`crates/htui-store/cache_migrations/0001_mirror.sql` creates `run`, `run_step`, `run_step_commit`,
`session_event` and `item` (including `touched_paths`, confirmed at line 80), and does not create
`step_graph`, `step_graph_phase`, `phase_agent`, `prompt_template`, `agent`, `agent_box`,
`command_run` or `app_setting`. Offline, therefore, the only route to a graph is
`run.graph_snapshot`, which is mirrored. That is not a defect: `R-STO-4` starts no runs offline.

**Two shipped facts contradict each other and one of them is a bug.** The DDL and the mirror both
declare `attempt INTEGER NOT NULL DEFAULT 1`; every demo fixture step writes `attempt: 0`
(`crates/htui-core/src/fixtures.rs:1169`, `:1203`) and the demo loader passes `row.attempt`
straight into the INSERT, so the default never applies. §4.4 picks 1 and MOD-4 corrects the
fixture.

---

## 4. Settled questions

### 4.1 Step graph data shape (`R-ORCH-1`, `R-ENT-6`)

**The constraint.** `R-ORCH-1` (`docs/REQUIREMENTS.md:154-157`), verbatim:

> "A step graph is a named, ordered list of phases owned by a project. Each phase has: candidate
> agents with model in priority order, fan-out count, gate, retry limit, input document kinds,
> output document kind, fan-out isolation mode, command queue mode, optional verification command.
> An item inherits its kind's default graph and may override it."

`R-ENT-6` (`docs/REQUIREMENTS.md:78-88`) fixes the seeded kind-to-graph table: `ANA`/analysis to
`research, verdict`; `FEAT`/feature to `prd, plan, implement, review`; `FIX`/bug to `reproduce,
fix, review`; `CLEAN`/refactor and `TOOL`/tooling to `plan, implement, review`.

**Options.**

| Option | Verdict | Reason |
|---|---|---|
| Rewrite `step_graph_phase` around a richer model (phase roles, DAG edges, per-task child rows) | Rejected | The column set is `R-ORCH-1`'s enumeration item for item and shipped byte-identical in `0001_init.sql`, with a Rust mirror (`crates/htui-core/src/model/kind.rs:88-124`) and a DDL-agreement unit test. A rewrite is a re-migration of live code for no requirement gain, and ANA-9 delegated *semantics*, not columns (`docs/ANA-9.md:495-496`). |
| Model the graph as a DAG with explicit edges | Rejected | `R-ORCH-1` says "ordered list". The only back-edge any requirement names is `R-ORCH-3`'s review to implement, and §4.4 expresses it as `attempt + 1` at the implement `position`, which the shipped `UNIQUE (run_id, position, attempt, fanout_index)` already addresses. LangGraph and Microsoft Agent Framework both need a graph because their nodes fan out arbitrarily (https://docs.langchain.com/oss/python/langgraph/checkpointers, https://learn.microsoft.com/en-us/agent-framework/workflows/checkpoints); `htui` does not. |
| Add a `phase_role` discriminator (`produce` / `verify` / `gate` / `integrate`) so an amend-the-previous-artifact phase like the in-house `fact-check` step is expressible | Rejected for v1, recorded as prior art | The maintainer's own `/handoff-run` surface runs `fact-check` (amends the plan in place) and `blueprint` (a kind `R-ENT-6` does not seed) as real phases (`.claude/skills/handoff-run/SKILL.md:110-124`). Both are expressible today without a new column: `fact-check` is a phase whose `output_kind` is a *new version* of `plan` (documents are append-only, `R-ENT-12`), and `blueprint` is an open `document.kind` (`0001_init.sql:391` has no CHECK). A discriminator would buy nothing the version chain does not already give. |
| Add a per-task child table so N parallel sessions can run N *different* prompts | Rejected for v1, open for the maintainer | This is the maintainer's actual practice (MOD-1 wave C, MOD-6 wave B: disjoint file sets, all results merged, no losers), and it is *not* what `R-ORCH-7` describes ("runs them in parallel on the same prompt ... Only the selected result continues"). `docs/REQUIREMENTS.md` is the contract; a second fan-out kind is a requirement change, not an ANA verdict. Recorded as **Open for the maintainer 1** with the default "rival fan-out only in v1". |
| Keep §5.4 and amend by migration for the semantics that genuinely need a column | **Adopted** | Five columns and one table, listed below. Everything else is a resolution rule, not storage. |

**Verdict.** The ANA-9 §5.4 DDL suffices for `R-ORCH-1`. Migration `0003_orchestration.sql` (§9)
adds, on `step_graph_phase`: `judge_agent_id`, `judge_model` (§4.5), `deadline_seconds` (§4.2,
§4.9); on `run`: `repo_scope UUID[]` and the lease columns (§4.7, §4.9); on `run_step`:
`verify_outcome`, `verify_exit_code`, `promoted_at` (§4.2, §4.8); on `step_graph`: `is_override`
(below); plus the new table `run_step_tree` (§4.6). No column is dropped, renamed or retyped.

**Semantics this fixes.**

*The graph is linear and positions are dense.* `position` is `0..n-1` with no gaps, enforced by the
snapshot builder rather than by a constraint (`UNIQUE (graph_id, position)` permits gaps). A run
walks `position` upward; the only way to revisit a position is `attempt + 1`.

*Resolution order for the graph itself.* `item.step_graph_id`, else `item_kind.default_graph_id`.
Both are FKs, so there is no third case; a project seeded per `docs/ANA-9.md` §5.10 always has a
kind graph.

*Resolution order for a phase field.* Each nullable phase field has exactly one fallback chain,
fixed here so MOD-4 does not invent one per field:

| Field | Chain | Built-in default |
|---|---|---|
| `isolation` | phase, then `project.settings.default_isolation`, then built-in | `worktree` (`R-ORCH-8` says "default") |
| `token_budget` | phase, then `project.settings.token_budget`, then `app_setting.token_budget` | ANA-5 owns the number |
| `template_version` | phase, then latest version of `template_name` in the project | latest |
| `deadline_seconds` | phase, then `project.settings.step_deadline_seconds`, then `app_setting.step_deadline_seconds` | 7200 (two hours) |
| `judge_agent_id` | phase, then `project.settings.judge_agent_id`, then none | none, meaning human selection (§4.5) |
| candidate agents | `phase_agent` ordered by `position`, then `project.settings.default_agent_id`, then the single enabled agent on the box, then refuse (§7) | refuse |

*Item override, made collision-free and collectable.* The clone is **deep over `step_graph_phase`
and `phase_agent`, and not over `skill_binding`**: a phase binding is keyed on
`skill_binding.phase_id` with `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)`
(`docs/ANA-9.md:679-692`), and copying bindings onto cloned phase ids would silently double every
project binding the item's phases inherit. An override therefore starts with project-level bindings
only, and a phase binding on an override graph is created explicitly.

The name is `<item.key>-override`. `step_graph.name` is `UNIQUE (project_id, name)` and `item.key`
is unique per project, so a second override *of the same item* is not a collision but a re-override:
MOD-4 deletes the existing override graph's phases and re-clones in one transaction, leaving
`item.step_graph_id` untouched. That is safe because a live run reads its own `graph_snapshot`
(invariant 2), never the table. `step_graph.is_override BOOLEAN NOT NULL DEFAULT false` (0003) hides
overrides from the Settings graph list (`R-TUI-8`) and is what MOD-15's project view filters on.
Orphan collection: an override graph whose item has been `closed` for longer than
`project.settings.retention_days` is dropped by the same sweep that drops steps; until then it is
history a run snapshot may still be compared against.

*Seeded phase defaults are frozen as `crates/htui-core/src/fixtures.rs:601-623` already writes
them*, with two amendments MOD-4 makes in the same change:

1. `input_kinds` on an `implement` phase gains `review`, so `R-ORCH-3`'s "with the review attached"
   is data rather than special-case code (§4.4). The seeded feature graph's `implement` becomes
   `input_kinds = ['plan','review']`, and `fix` in the bug graph becomes
   `input_kinds = ['reproduce','review']`.
2. `gate_hard` is set on the phases named in **Open for the maintainer 5**; the fixture's uniform
   `false` is otherwise kept.

Everything else stays: `fan_out: 1`, `gate: always`, `retry_limit: 1`, `isolation: None`,
`command_queue: fan_out_only`, `verify_command: None`, `output_kind = name`, `template_name = name`,
`template_version: None`, `token_budget: None`.

*A design notation for the resolved phase.* The orchestrator never reads `step_graph_phase` directly
at run time; it reads a `ResolvedPhase` off the snapshot, with every chain already walked and every
auto-mode downgrade already applied (§4.10):

```rust
// crates/htui-orch/src/graph.rs - design notation, not code

/// One phase of `run.graph_snapshot`, with every fallback chain resolved at snapshot time.
pub struct ResolvedPhase {
    pub position: i32,
    pub name: String,
    pub fan_out: i32,
    pub gate: Gate,              // as configured
    pub gate_effective: Gate,    // after the R-ORCH-6 downgrade; equals `gate` in manual mode
    pub gate_hard: bool,
    pub retry_limit: i32,
    pub input_kinds: Vec<String>,
    pub output_kind: String,
    pub isolation: Isolation,    // resolved: never None here
    pub command_queue: CommandQueue,
    pub verify_command: Option<String>,
    pub deadline: Duration,
    pub template: TemplateRef,   // name + pinned version
    pub token_budget: Option<i32>,
    pub candidates: Vec<PhaseAgentRef>,          // agent_id + model, priority order
    pub judge: Option<PhaseAgentRef>,            // judge_agent_id + judge_model
}
```

`ResolvedPhase` is what `graph_snapshot` serialises (§5.1) and what every later subsection means by
"the phase".

### 4.2 Phase contract: inputs, output kind, gate, retry, verification (`R-ORCH-1`, `R-ORCH-2`, `R-ENT-12`, `R-MCP-3`)

**The constraint.** `R-ORCH-2` (`docs/REQUIREMENTS.md:158-160`), verbatim:

> "Gate is `always`, `on_failure` or `never`, plus a `hard` flag. A gated step ends in
> `awaiting_approval`; the user approves, rejects with a note, edits the artifact, or retries. Auto
> mode (R-ORCH-6) downgrades every gate to `never` except those flagged `hard`."

`R-ENT-12` (`:103-105`): "`document`: versioned per item and kind. Kinds follow the phases of the
graph ... Produced by steps or written by hand." `R-MCP-3` (`:238-241`) fixes the three
`command_queue` values and the `heavy_build` force-on.

**Options.**

| Option | Verdict | Reason |
|---|---|---|
| `input_kinds` resolves to *every* version of each named kind | Rejected | Unbounded prompt growth, and `R-PRM-3`'s trim order would spend its whole budget re-reading superseded drafts. |
| `input_kinds` resolves to the latest version of each kind, full stop | Rejected | It reads fan-out losers. `document UNIQUE (item_id, kind, version)` forces N concurrent writers of one kind onto N versions, so "latest" is whichever loser finished last. |
| `input_kinds` resolves to the latest version whose producing step is not a loser, preferring this run's own steps | **Adopted** | One query, decidable, and it is the only reading under which `R-ORCH-7`'s "the rest are kept as history" and "only the selected result continues" are both true. |
| The agent allocates `document.version` | Rejected | N parallel `document_write` calls race on `UNIQUE (item_id, kind, version)`; an agent retrying a unique violation is an agent in a bookkeeping path (`R-ID-6`). |
| The orchestrator allocates `document.version` inside the insert transaction | **Adopted** | `INSERT ... SELECT coalesce(max(version),0)+1` under the row lock of §8's `write_document`; the MCP tool calls it rather than writing directly. |
| `verify_command`'s exit lands in `run_step.exit_code` | Rejected | `R-ORCH-11` already claims `exit_code` for the step's own exit code (`docs/REQUIREMENTS.md:185-186`); the column would be double-booked and the agent's code lost. |
| `verify_command` gets its own outcome and exit columns, with a third outcome for "could not run" | **Adopted** | Anthropic's `/deep-research` draws exactly this distinction: "When the verifier agents can't check a claim, such as after a rate limit or API error, the report lists that claim as unverified instead of counting it as refuted" (https://code.claude.com/docs/en/workflows). Without it a rate-limited or offline box marks good work failed and `R-ORCH-3` burns retries on it. |
| Run `verify_command` only on the selected fan-out candidate | Rejected | It is the only execution evidence the judge can have, and judging code without it is measurably worse than not judging: JETTS reports that on MBPP+ every judge protocol was worse than the greedy baseline (https://arxiv.org/pdf/2504.15253). |
| Run `verify_command` on every candidate, before selection | **Adopted** | §4.5's prefilter. CodeT establishes the same ordering for code selection generally (https://arxiv.org/abs/2207.10397). |

**Verdict: the phase is a total function from resolved inputs to exactly one output document, run in
a fixed six-stage order.**

```
1. admit      capability check (R-ORCH-10), overlap admission (R-ORCH-9), agent selection (R-AGT-8)
2. prepare    isolation trees per repo, before_hash per repo (R-ORCH-8)
3. prompt     input_kinds resolution -> ANA-5 assembly -> prompt_digest
4. session    AgentDriver::start .. Done  (ANA-4)
5. settle     output document present? -> verify_command -> after_hash per repo
6. gate       gate_effective evaluated against the settle outcome; judge if fan-out
```

Stages 1 to 4 are the only ones that can be skipped (a superseded step never runs them). Stage 5 is
what makes invariant 5 true: nothing reaches `awaiting_approval` until the artefact, the
verification and the commit hashes are durable.

**Input resolution, exactly.** For each `kind` in `input_kinds`, in order:

```sql
SELECT d.*
  FROM document d
  LEFT JOIN run_step s ON s.id = d.produced_by_step_id
 WHERE d.item_id = $item AND d.kind = $kind
   AND (s.id IS NULL OR s.selected IS NOT FALSE)      -- hand-written, winner, or fan_out = 1
 ORDER BY (s.run_id = $run) DESC NULLS LAST,          -- this run's own output first
          d.version DESC
 LIMIT 1;
```

`s.selected IS NOT FALSE` is the loser exclusion of invariant 8: `NULL` (fan_out = 1) and `true`
(winner) both pass, `false` does not. `(s.run_id = $run) DESC` is the rule that a phase reads what
*this* run produced when it exists and falls back to the item's history otherwise, which is what
makes a re-run of a graph on an item that already has documents behave sensibly.

A kind with no row is a **hard failure at stage 3**, before a token is spent: the step goes to
`failed` with `run.failure = "missing input document: <kind>"`, the run parks per §4.3, and the item
goes to `blocked`. Silent omission was rejected because a phase silently running without its plan
produces an artefact that looks valid and is not.

`document.produced_by_step_id` is `ON DELETE SET NULL` and the retention sweep removes whole steps
(`docs/ANA-9.md:262-264`), which would reclassify a loser's document as hand-written and readmit it
to this query. The sweep is therefore amended in `0003`'s companion rule: **the sweep skips a step
whose `selected IS FALSE`**, keeping the loser row (which is small: status, index and timings) while
still dropping its `session_event` rows. That is stated in §9 rather than left as a latent bug.

**Output.** Exactly one document of `output_kind` per step. It is written by MOD-11's
`document_write` tool, which calls §8's `write_document` rather than the store directly, so version
allocation and the `produced_by_step_id` stamp have one owner. MOD-11 is blocked on MOD-4
(`HANDOFF.md`), so MOD-4 ships with two non-MCP producers of the same row and neither is a
workaround: the gate action `accept artifact` (§4.8) and a hand-written document (`R-ENT-12` permits
it). Until MOD-11 lands, a phase whose `gate_effective = never` and whose agent produced no document
fails with `missing_output`; a gated phase parks and a human supplies the artefact. This is a real
degradation and is named as **Risk 4**.

**Gate resolution.** The gate is evaluated once, at stage 6, against a three-valued *settle
outcome*:

| Settle outcome | Definition |
|---|---|
| `ok` | output document present, and `verify_outcome IN ('pass','unavailable')` |
| `failed` | any of: `DriverEvent::Error`; `done` with `stop_reason IN ('refusal','max_tokens','max_turn_requests')`; a cap breach (`R-AGT-7`); a missing output document; `verify_outcome = 'fail'`; the step deadline elapsed |
| `rejected` | a `review`-phase output document whose front matter says `verdict: request-changes` (§4.4) |

`stop_reason = 'cancelled'` is not a settle outcome: it is a cancel, which is its own status.
`verify_outcome = 'unavailable'` never fails a step, by the `/deep-research` rule quoted above; it is
surfaced in the Runs tab and recorded.

| `gate_effective` | settle `ok` | settle `failed` | settle `rejected` |
|---|---|---|---|
| `always` | step to `awaiting_approval`, `gate_outcome` NULL until answered | step to `awaiting_approval` | step to `awaiting_approval` |
| `on_failure` | step to `done`, `gate_outcome = 'skipped'` | step to `awaiting_approval` | step to `awaiting_approval` |
| `never` | step to `done`, `gate_outcome = 'skipped'` | §4.4 retry if budget remains, else step to `failed` and run to `awaiting_approval` (escalation) | §4.4 loop |

`gate_outcome = 'skipped'` is therefore the trace of *any* gate that did not stop, whether because it
was configured `never`, because `on_failure` did not trip, or because auto mode downgraded it. Which
of the three it was is recoverable from `run.mode` plus the snapshot's `gate` versus `gate_effective`,
so no fourth value is needed. That closes the "downgrade leaves no trace" gap.

**Gate answers.** `R-ORCH-2` names four user actions and `GateOutcome` has four variants, but they
are not the same four: the enum has `skipped` and lacks "edits the artifact". The resolution is that
**editing the artifact is not a gate outcome**: `R-ENT-12` makes documents append-only, so an edit is
a new document version followed by `approved`. The mapping is:

| `R-ORCH-2` action | `gate_outcome` | Step lands | Run lands |
|---|---|---|---|
| approves | `approved` | `done` | `running` (next position) |
| rejects with a note | `rejected` + `gate_note` | `failed` | `running` when the phase can loop and budget remains (§4.4), else `failed` |
| edits the artifact | `approved` + a new `document` version | `done` | `running` |
| retries | `retried` | `superseded`, new step at `attempt + 1` | `running` |
| (no stop) | `skipped` | `done` | `running` |

The reviewer's real third verdict, "approve with warnings"
(`~/.claude/agents/rust-reviewer.md:97-102` grades Approve / Warning / Block), has no enum value and
does not get one: a MEDIUM-only review is `approved` with the findings living in the review document,
which is where they are readable. Adding `approved_with_warnings` would be a CHECK-constraint
migration buying a distinction no orchestrator branch consumes.

**Capability interlock.** ANA-4 binds MOD-4 to a refusal, verbatim (`docs/ANA-4.md:554`): "`R-ORCH`
gates that require an inline approval must not be scheduled onto a CLI-only agent", and restates it
as its risk 9: "MOD-4 refuses to schedule a gated phase onto a CLI-only agent". The enforcement point
is stage 1: a candidate whose `DriverCaps.permission_requests` and `edit_proposals` are both false is
skipped for a phase whose `gate_effective != 'never'`, exactly as an exhausted-quota candidate is
skipped (§7). If no candidate survives, the run is refused with
`missing_capability: inline_approval` and the item goes to `blocked`.

**Retry counting is fixed here and used by §4.4.** `attempt` is 1-based, matching the DDL default and
the mirror; `crates/htui-core/src/fixtures.rs`'s `attempt: 0` is a fixture bug MOD-4 corrects.
`retry_limit` is the number of *additional* attempts, so the admission predicate is
`attempt <= retry_limit + 1` and the shipped default `retry_limit = 1` permits two attempts.
`CHECK (retry_limit >= 0)` therefore means zero retries, one attempt, which is the reading that makes
the constraint meaningful.

**Verification.** `verify_command` runs at stage 5, after the session and before the gate, in the
step's own tree for the project's primary repo (`run_step_tree` where the repo is `is_primary`), with
the agent's environment minus the secrets ANA-7 resolved. Where it runs is a function of
`command_queue`:

| `command_queue` | Effective for the phase | `verify_command` runs |
|---|---|---|
| `off` | tool not advertised (`R-MCP-3`) | directly by the orchestrator |
| `fan_out_only` (default) | advertised when `fan_out > 1`, or when the item carries `heavy_build` | through `command_run` with `class = 'verify'` when advertised, else directly |
| `always` | always advertised | through `command_run` with `class = 'verify'` |

Running it through `command_run` is what puts it behind `box.settings.command_limits`, which is the
whole point of `R-MCP-3`'s per-class limits: a fan-out of four `cargo test` runs on one box must
queue. Before MOD-11 exists, MOD-4 runs it directly in every case and honours the class limit with the
same in-process semaphore §4.7 uses for `max_concurrent_items`; the semantics do not change when
MOD-11 lands, only the enqueue path.

The three outcomes land in `0003`'s two columns:

| Situation | `verify_outcome` | `verify_exit_code` |
|---|---|---|
| exit 0 | `pass` | 0 |
| nonzero exit | `fail` | the code |
| no `verify_command` on the phase | NULL | NULL |
| command could not run (binary missing, box offline, class queue cancelled, deadline elapsed) | `unavailable` | NULL |

`run_step.exit_code` stays `R-ORCH-11`'s field: the agent process's own exit code, or NULL for an ACP
session that ended without one.

### 4.3 Status transitions: item, run, run_step (`R-ENT-8`, `R-ORCH-11`)

**The constraint.** `R-ENT-8` (`docs/REQUIREMENTS.md:91-93`), verbatim:

> "Item status is one of `open`, `queued`, `in_progress`, `awaiting_approval`, `blocked`, `done`,
> `failed`, `closed`. Transitions are driven by the orchestrator and by close-out, not edited by
> hand."

The mechanism is already fixed (`docs/ANA-9.md:165-169`): a status move is its own compare-and-set
on `status`, "a failed status CAS is an orchestrator race, reported as such, and it never touches
the revision log", with the stated rationale that "if status bumped `version`, every step completion
would invalidate the body edit the user has open in `$EDITOR`".

**Options.**

| Option | Verdict | Reason |
|---|---|---|
| `blocked` is derived only, never stored; keep the CHECK value reserved-unused | Rejected | `R-ENT-8` lists it as a status and `R-ID-3` says everything `htui` knows lives in Postgres. A capability refusal, a secret-provider outage and an exhausted retry budget currently persist nowhere at all, which is the `R-ORCH-10` gap. A derived-only `blocked` leaves three refusals as transient messages. |
| `blocked` is stored and means "a dependency is open" | Rejected | `docs/ANA-9.md:946-958` already computes that dynamically from live `blocked_by` edges and filters `i.status = 'open'`, so a stored value would take the item out of the ready query permanently and would need an un-blocking duty on every close-out, crossing the item boundary. It also duplicates a fact the edge already carries. |
| **`blocked` is stored and means "this run cannot proceed until a human clears something"; dependency-blocked stays derived** | **Adopted** | Two different facts get two different mechanisms. The stored value is written by the orchestrator only, at three named events, and is cleared by an explicit human action the Backlog and Runs tabs offer. `docs/ANA-9.md` §7.4 is unchanged. |
| Enforce transition legality with a database trigger | Rejected | The only triggers in `0001_init.sql` are the twenty `updated_at` ones, whose doc block is emphatic that "no write path may set `updated_at` by hand" (`0001_init.sql:566-571`). A trigger vocabulary would be a second, untested state machine, and `MemStore` could not honour it. |
| Enforce legality in Rust beside `is_terminal` / `is_active`, exercised by the conformance suite | **Adopted** | `Status::is_terminal()` and `RunStatus::is_active()` are already `const fn` predicates in `htui-core` with a DDL-agreement test; `can_move_to` joins them and `MemStore`, `PgStore` and any future store are held to it by the one suite. |
| Item status is written independently of run status | Rejected | Nothing would keep them consistent, and the two enums are not one-to-one (item has `queued`, `blocked`, `closed`; run has `cancelled`). |
| **Item status is a function of the item's non-terminal runs, written in the same transaction as the run move** | **Adopted** | One writer, one transaction, and the mapping is a total function (below). Cursor's cloud-agent API draws the same split, keeping a recoverable error on the run while the durable identity returns to `IDLE` (https://cursor.com/docs/cloud-agent/api/endpoints). |

**Verdict 1: `blocked` is stored, written by the orchestrator at exactly three events, and cleared
by exactly one human action.**

| Event | Also written | Cleared by |
|---|---|---|
| `R-ORCH-10` capability refusal: `item.required_tags` is not a subset of the box's tags | an `item_note` whose body is the missing-tag list from `docs/ANA-9.md:960` | `unblock` (Backlog action) once tags are added or the target box changes |
| `R-ORCH-3` escalation: the review loop exhausted its retry budget, or the judge could not decide | the run parks in `awaiting_approval`; the last review step keeps `gate_outcome = 'rejected'` | `retry` or `approve` on the Runs tab, or `unblock` |
| `R-SEC-4` refusal: the secret provider is unreachable and the run needs its secrets | `run.failure` when a run row exists | `unblock` after the provider returns |
| Stage-3 prompt refusal: `assemble()` refuses the step's prompt before a token is spent (amended by MOD-4 milestone 6, 2026-09-25; MOD-4 plan D162, D195) | the step `failed`, an `item_note` with the assembler's sentence, then `finish_run(Failed, PromptRefused)`; no session starts | `unblock` returns the item to `open` |

*Amended by MOD-4 milestone 6, 2026-09-25:* the table above is no longer exactly three events. ANA-5
criterion 3 asks only an unknown placeholder to block; MOD-4 plan D162 generalises that to every
stage-3 prompt refusal, at both the single-step and the fan-out call site. MOD-4 milestone 4's
no-candidate refusal (plan D62) also writes `blocked`.

A `blocked` item is visible in the Backlog with its note, so the "invisible to the queue forever"
failure the two designs would otherwise produce cannot happen: `docs/ANA-9.md` §7.4 still filters
`i.status = 'open'`, and auto mode is right not to pick up an item a human has been asked to clear.

**Verdict 2: `failed` upstream still blocks, and the escape is `close`.** `Status::is_terminal()`
stays `Done | Closed` as shipped. A `failed` blocker genuinely should block its dependents, and the
release valve is the `close` action `R-TUI-2` already lists: `failed -> closed` sets `closed_at` and
makes `is_terminal()` true. No readiness change, no new state.

**Verdict 3: the item status transition table, all eight states.**

| from | event | guard | to | who |
|---|---|---|---|---|
| `open` | `run` (manual) or auto admission | capability check passes; a `run` row is inserted `queued` in the same transaction | `queued` | orchestrator |
| `open` | capability refusal | `required_tags` not a subset of box tags | `blocked` | orchestrator |
| `open` | secret provider unreachable | run needs project secrets | `blocked` | orchestrator |
| `queued` | run claimed | lease acquired, `executing_box_id` = local box | `in_progress` | orchestrator |
| `queued` | cancel | no other non-terminal run on the item | `open` | user |
| `queued` | capability or secret refusal at claim | box tags changed since queueing | `blocked` | orchestrator |
| `in_progress` | a step reaches a gate | `gate_effective` stops (§4.2) | `awaiting_approval` | orchestrator |
| `in_progress` | last position done | no further position in the snapshot | `done` | orchestrator |
| `in_progress` | step failed, budget exhausted, `gate_effective = never`, phase not loopable | - | `failed` | orchestrator |
| `in_progress` | escalation (`R-ORCH-3` exhausted, judge undecided, judge quota exhausted) | - | `blocked` | orchestrator |
| `in_progress` | cancel | no other non-terminal run | `open` | user |
| `awaiting_approval` | approve, or accept artifact after a chat promotion | a document of `output_kind` exists | `in_progress` | user |
| `awaiting_approval` | retry | `attempt <= retry_limit + 1` | `in_progress` | user |
| `awaiting_approval` | reject with note, phase loopable | budget remains (§4.4) | `in_progress` | user |
| `awaiting_approval` | reject with note, terminal | no budget, or the phase has no predecessor to loop to | `failed` | user |
| `awaiting_approval` | cancel | - | `open` | user |
| `blocked` | unblock | - | `open` | user |
| `blocked` | unblock | the item's run is parked `awaiting_approval` (an escalation); added by MOD-4 plan D161 (amended by MOD-4 milestone 6, 2026-09-25) | `awaiting_approval` | user |
| `blocked` | close | - | `closed` | close-out |
| `failed` | retry | - | `queued` | user |
| `failed` | close | - | `closed` | close-out |
| `done` | close-out | summary document written, commits recorded (`R-TUI-9`) | `closed` | close-out |
| `done` | reopen | - | `open` | user |
| `closed` | - | terminal | - | - |

*Amended by MOD-4 milestone 6, 2026-09-25 (plan D161, R-42):* the `blocked → awaiting_approval`
row is new. As first written, the table had no way back from `blocked` to a parked run, while
verdict 1's escalation row is cleared by "`retry` or `approve`", which a `blocked` item cannot reach
(MOD-4 risk R-4). `Unblock` now has three cases: a blocked item with no live run goes to `open`; a
blocked item whose run is parked follows it to `awaiting_approval`, so the Runs tab's promote,
approve, retry and cancel reach the run; and an `awaiting_approval` item whose run is parked by a
refused reconcile or a crash is resumed (`Engine::resume`). `Status::can_move_to` and its
`SANCTIONED` test pin the one added pair.

Three notes on this table. **`queued` is exactly "a `run` row exists in status `queued` for this
item"** - the two are written in one transaction, so the state is never a lie. **`done` and `closed`
are genuinely different**: `done` means the graph finished, `closed` means close-out ran, which is
what `R-TUI-9` describes and what `closed_at` records. **A cancelled run does not make the item
`cancelled`**, because `R-ENT-8` has no such value; the item returns to `open`, which is correct: a
cancelled run completed nothing the item should carry forward.

**Verdict 4: the `run.status` transition table.**

| from | event | guard | to | who |
|---|---|---|---|---|
| (insert) | queue | admission passed | `queued` | orchestrator |
| `queued` | claim | lease acquired, `target_box_id` = local box (`R-ORCH-12`), overlap slot free | `running` | orchestrator |
| `queued` | cancel | - | `cancelled` | user |
| `queued` | refusal at claim | capability, secret or agent-selection refusal | `failed` | orchestrator |
| `running` | a step entered `awaiting_approval` | - | `awaiting_approval` | orchestrator |
| `running` | last position's step reached `done` | no further position | `done` | orchestrator |
| `running` | a step reached `failed` with no retry budget and no gate | - | `failed` | orchestrator |
| `running` | escalation | `R-ORCH-3` exhausted, judge undecided | `awaiting_approval` | orchestrator |
| `running` | cancel | - | `cancelled` | user |
| `running` | lease expired and the step is unrecoverable | recovery sweep (§4.9) | `awaiting_approval` | recovery sweep |
| `awaiting_approval` | gate answered `approved` / `retried` | - | `running` | orchestrator |
| `awaiting_approval` | gate answered `rejected`, loopable | §4.4 admission passes, new attempt inserted | `running` | orchestrator |
| `awaiting_approval` | gate answered `rejected`, terminal | no loop, or no budget | `failed` | orchestrator |
| `awaiting_approval` | cancel | - | `cancelled` | user |
| `done`, `failed`, `cancelled` | - | terminal | - | - |

`run.status` is **derived from its steps** at every step transition, except `queued` (set at insert)
and `cancelled` (only a human sets it). That derivation is the single reason the two machines cannot
drift, and it is what the recovery sweep re-runs on adoption. `RunStatus::is_active()` stays
`Queued | Running | AwaitingApproval` as shipped, and it is what `R-TUI-1`'s active-run count reads.

`R-ORCH-6`'s "escalates failures" is therefore `awaiting_approval`, not a new value: a run that
escalated is parked for a human, which is precisely what `awaiting_approval` means everywhere else,
and it keeps `failed` for the case where nothing a human does at this gate can help. This is the
resolution of the "escalation has no run status" gap.

**Verdict 5: the `run_step.status` transition table.**

| from | event | guard | to | who |
|---|---|---|---|---|
| (insert) | step planned | - | `pending` | orchestrator |
| `pending` | scheduled | trees built, `before_hash` captured, agent selected, session spawned | `running` | orchestrator |
| `pending` | replaced before start | a retry or a fan-out selection landed first | `superseded` | orchestrator |
| `pending` | run cancelled | - | `cancelled` | orchestrator |
| `running` | settle done, gate stops | §4.2's gate table | `awaiting_approval` | orchestrator |
| `running` | settle `ok`, gate does not stop | output document present, `verify_outcome != 'fail'` | `done` | orchestrator |
| `running` | settle `failed`, gate does not stop, no budget | - | `failed` | orchestrator |
| `running` | driver error, cap breach, deadline elapsed, spawn failure | - | `failed` | orchestrator |
| `running` | cancel | - | `cancelled` | user |
| `running` | promote to chat | session cancelled with the driver's grace window first (§4.8) | `awaiting_approval` with `promoted_at` set | user |
| `running` | lease expired, `after_hash` present and output document present | recovery sweep (§4.9) | `done` | recovery sweep |
| `running` | lease expired, otherwise | recovery sweep | `failed` | recovery sweep |
| `awaiting_approval` | approve, or accept artifact after chat | document of `output_kind` exists | `done` (`gate_outcome = 'approved'`) | user |
| `awaiting_approval` | reject with note | - | `failed` (`gate_outcome = 'rejected'`, `gate_note` set) | user |
| `awaiting_approval` | retry | `attempt <= retry_limit + 1` | `superseded` (`gate_outcome = 'retried'`), new step at `attempt + 1` | user |
| `awaiting_approval` | promote to chat | `DriverCaps` permits, or a handoff prompt is built (§4.8) | `awaiting_approval` with `promoted_at` set | user |
| `awaiting_approval` | cancel | - | `cancelled` | user |
| `failed` | promote to chat | the run is non-terminal (parked by escalation, §4.4); `DriverCaps` permits, or a handoff prompt is built (§4.8) | `awaiting_approval` with `promoted_at` set | user |
| `done` | fan-out selection chose another candidate | `selected = false` written in the same transaction | `superseded` | orchestrator or judge |
| `done` | review-loop iteration supersedes the implement attempt it reviewed | §4.4 | `superseded` | orchestrator |
| any non-terminal | run cancelled | - | `cancelled` | orchestrator |
| `done`, `cancelled`, `superseded` | - | terminal | - | - |
| `failed` | - | terminal, except for the promotion row above while its run is non-terminal | - | - |

`superseded` carries exactly the two meanings its shipped doc comment gives it, "Replaced by a retry
or by a fan-out winner", and this table is the complete list of who writes it.

**`awaiting_approval` at three levels, and the propagation rule.** The three enums all carry the
value and the rule is one line: **a step is the only thing that decides to wait; the run mirrors it;
the item mirrors the run.** A run is `awaiting_approval` iff at least one of its steps is; an item
is `awaiting_approval` iff at least one of its non-terminal runs is. Both mirrors are written in the
step's transaction, so a reader never sees a run waiting with no waiting step.

**Notation for the legality predicate.**

```rust
// crates/htui-core/src/model/item.rs and run.rs - design notation, not code

impl Status {
    /// Whether the orchestrator or close-out may move an item from `self` to `to` (ANA-2 §4.3).
    /// Every legal pair is a row of the item transition table; a pair outside it is a bug, not a
    /// race, and `WriteStore::transition` rejects it before it reaches Postgres.
    pub const fn can_move_to(self, to: Self) -> bool { /* the table */ }
}

impl RunStatus  { pub const fn can_move_to(self, to: Self) -> bool { /* the table */ } }
impl StepStatus { pub const fn can_move_to(self, to: Self) -> bool { /* the table */ } }
```

`WriteStore::transition` keeps its shipped signature and semantics (`Ok(false)` on a `from`
mismatch, `NotFound` on a missing row) and gains one guard: an illegal `(from, to)` pair returns
`StoreError::Constraint` rather than performing the update. `StoreError` is `Clone + PartialEq + Eq`
and `Constraint(String)` already exists, so no error-type change is needed. Three new conformance
cases cover it: an illegal pair is refused, a legal pair on a stale `from` returns `Ok(false)`, and
a status move leaves `version` untouched (the shipped `status_cas_keeps_version` case already covers
the third and is extended rather than duplicated).

### 4.4 The review-to-implement loop (`R-ORCH-3`)

**The constraint.** `R-ORCH-3` (`docs/REQUIREMENTS.md:161-162`), verbatim:

> "A failed `review` loops back to the preceding `implement` with the review attached, up to the
> retry limit, then escalates to the user."

**Options.**

| Option | Verdict | Reason |
|---|---|---|
| The loop appends a new `position` for each iteration | Rejected | It makes `position` a run-time property rather than a snapshot index, so `run_step.position` ("phase index in the snapshot") stops meaning what its comment says, and resume can no longer map a step back to a phase. |
| The loop re-runs the implement phase at the same `position` with `attempt + 1` | **Adopted** | `UNIQUE (run_id, position, attempt, fanout_index)` was built for exactly this, `attempt`'s own comment is "retry / review loop counter", and `RunSummary.steps` is already ordered `(position, attempt, fanout_index)`. |
| The loop is bounded by the **review** phase's `retry_limit` | Rejected | The retried work is the implement phase's, so the budget belongs where the cost is; a reviewer configured with `retry_limit = 0` would otherwise silently forbid all fixing. |
| The loop is bounded by the **implement** phase's `retry_limit` | **Adopted** | Same predicate as an ordinary post-gate retry (§4.2), so there is one counter and one rule rather than two. |
| The loop carries the whole previous transcript | Rejected | `R-PRM-1` forbids raw transcripts of other work in a prompt, and SWE-agent's retry loop passes only an explicit forwarded set via `get_forwarded_vars()` with the review written back as `info["review"]` (https://github.com/SWE-agent/SWE-agent/blob/main/sweagent/agent/reviewer.py). |
| The loop carries a named, small forwarded set | **Adopted** | Three items, below. |
| Escalation is `run.status = 'failed'` | Rejected | `R-ORCH-3` says "escalates to the user", and `failed` ends the run instead of parking it, so the user has nothing to approve or retry. |
| Escalation is `run.status = 'awaiting_approval'` plus `item.status = 'blocked'` | **Adopted** | The run parks where every other human decision parks; the item carries the "a human must clear this" marker of §4.3. |
| Stop only on the retry count | Rejected as insufficient | Anthropic publishes the second predicate as an idiom: "keep fixing the reported errors until the type check passes or two rounds in a row make no progress" (https://code.claude.com/docs/en/workflows). A count-only loop spends the whole budget on a stuck agent. |

**Verdict.** A `review` step whose settle outcome is `rejected` (or `failed`) triggers the loop:

1. Find `p_impl`, the greatest `position < p_review` in the snapshot whose phase name is
   `implement`, or, when there is none, the immediately preceding position. Naming the phase rather
   than assuming adjacency is what makes the rule work for the `FIX` graph, whose implement-shaped
   phase is called `fix`.
2. Admission: `attempt(p_impl) + 1 <= retry_limit(p_impl) + 1`, and the no-progress predicate below
   is false.
3. Mark the reviewed implement step and the rejecting review step `superseded`; the review step
   keeps `gate_outcome = 'rejected'` and its `gate_note`, so the reason survives the supersession.
4. Insert a new step at `(position = p_impl, attempt = a + 1, fanout_index = 0)` in `pending`.
5. Resume the walk from `p_impl`. Every position between `p_impl` and `p_review` re-runs, at the
   same `attempt` value, so a `plan -> implement -> review` graph re-runs only implement and review.

**What the loop carries.** Exactly three things, and the phase reads all three through the ordinary
`input_kinds` machinery of §4.2 rather than through special-case code:

| Carried | Mechanism |
|---|---|
| the review document | `review` is in the implement phase's `input_kinds` (§4.1's seed amendment); §4.2's resolver picks the latest non-loser version, which is the rejecting review |
| the failing verification output | `run_step.verify_exit_code` and the stored `command_run.output` of the previous attempt, injected by ANA-5 as a prompt section |
| the previous attempt's diff | the range `before_hash..after_hash` from that attempt's `run_step_tree` rows; ANA-5 renders a diff stat plus the diff, subject to its own trim order |

Nothing else crosses. The previous attempt's session transcript does not, by `R-PRM-1`.

**The no-progress predicate.** The loop stops early, and escalates, when two consecutive implement
attempts produce **either** an identical `after_hash` per repo (including "both NULL", meaning
neither attempt committed anything) **or** an identical `sha256` of the review document body. Both
are cheap to compute from rows the step already wrote, and both catch the failure mode a retry count
cannot: an agent that confidently reproduces the same wrong answer.

**Escalation.** On exhaustion, or on a true no-progress predicate:

| Row | Value |
|---|---|
| the last review `run_step` | `status = 'failed'`, `gate_outcome = 'rejected'`, `gate_note` = the review's own verdict line |
| `run` | `status = 'awaiting_approval'`, `failure` = `"review loop exhausted after N attempts"` |
| `item` | `status = 'blocked'` plus an `item_note` naming the phase, the attempt count and the stop reason |

From there the Runs tab offers `approve` (accept the work despite the review), `retry` (which raises
the attempt budget by one for this run only, recorded on the step's `gate_note`), `promote to chat`
(§4.8) and `cancel`. That is the whole escalation surface, and every one of the four is an existing
`R-TUI-4` action.

**Interaction with fan-out.** A loop iteration re-runs the implement phase at its configured
`fan_out`, so a fanned-out implement phase produces N fresh candidates at `attempt + 1` and the
previous attempt's winner and losers are all `superseded`. The judge runs again. This is the only
sensible reading: reusing the previous winner would mean the review rejected a candidate the loop
then re-submits unchanged.

**Escalate is not terminate.** The signal that ends the loop must not end the run: Google's ADK
conflates them and the result is that a sub-agent's `escalate=True` "stops the loop, but also the
parent SequentialAgents" (https://github.com/google/adk-python/issues/1376), so the phase after the
loop never runs. Here the run parks at `awaiting_approval` with its `position` intact, and an
`approve` resumes at `p_review + 1`.

### 4.5 Fan-out selection and the judge contract (`R-ORCH-7`)

**The constraint.** `R-ORCH-7` (`docs/REQUIREMENTS.md:172-175`), verbatim:

> "Fan-out: a phase with N candidate sessions runs them in parallel on the same prompt, each
> producing its own artifact and, for code phases, its own tree. Selection is a human choice when
> gated, otherwise a judge step using a configured agent. Only the selected result continues; the
> rest are kept as history."

**Options for where the judge lives.**

| Option | Verdict | Reason |
|---|---|---|
| A free-form convention: a `run_step` whose `phase_name` is the literal `"judge"` | Rejected | `phase_name TEXT NOT NULL` is free-form, so it is the zero-migration option, but `R-ORCH-7` says "a **configured** agent" and a convention configures nothing: there would be no place to name the agent or the model. |
| Reserve the last `phase_agent.position` as the judge | Rejected | It collides with `R-AGT-8`, which walks that same list in priority order as execution candidates; the last entry is the last fallback, not a different role. |
| An extra `step_graph_phase` row per judged phase | Rejected | It doubles the graph a user edits, breaks the "ordered list of phases" reading of `R-ORCH-1` for a purely internal step, and makes `position` arithmetic (§4.4's `p_impl` search) walk over rows that are not work. |
| **`judge_agent_id` / `judge_model` columns on `step_graph_phase`, plus a real `run_step` at the same position with a reserved `fanout_index`** | **Adopted** | The configuration lives with the phase it judges; the execution is an ordinary step, so `R-ORCH-11`'s per-step recording (agent, model, usage, timing, prompt digest) and `R-HIS-1`'s "nothing about a run exists only on one box" hold for the judge's own reasoning without a single special case. |

**Options for the judge protocol.**

| Option | Verdict | Reason |
|---|---|---|
| Pointwise scoring, one call per candidate, highest score wins | Rejected as the primary | SWE-agent's pointwise reviewer needs `n_sample: 5` with a mean-minus-standard-deviation reduction to be stable (https://github.com/SWE-agent/SWE-agent/blob/main/sweagent/agent/reviewer.py), which is 5N calls for N candidates. |
| Pairwise round-robin, both orders per pair | Rejected | O(N squared) with intransitivity, and pairwise preferences are the more manipulable protocol: they flip in about 35% of cases under an injected distractor versus 9% for absolute scores (https://arxiv.org/pdf/2504.14716). |
| **One comparative call over all survivors, repeated once with the candidate order reversed** | **Adopted** | Two calls at any N. Position bias is real and large (a model-average 64.3% first-shown pick rate, with the median model flipping its choice in 41.3% of decisive swapped-order pairs, https://github.com/lechmazur/position_bias), so a single-order judge is not sound; agreement across the two orders is the cheapest sound test at the fan-out sizes `htui` will see. |
| No execution prefilter; the judge sees everything | Rejected | JETTS finds that on MBPP+, a code benchmark, every judge protocol was worse than the greedy baseline (https://arxiv.org/pdf/2504.15253). A judge without execution evidence is a liability on code. |
| **Eliminate candidates whose `verify_outcome = 'fail'` before consulting the judge, when at least two survive** | **Adopted** | SWE-agent's chooser does the same with exit status, disabling the filter when fewer than two candidates qualify; CodeT establishes that execution evidence dominates agreement for code selection (https://arxiv.org/abs/2207.10397). |
| Judge failure falls back to candidate index 0 | Rejected | It picks silently and records a choice nobody made. `R-ORCH-7` already has a human branch; using it costs one park. |
| **Judge failure, disagreement across the two orders, or an out-of-range index escalates to human selection** | **Adopted** | The run parks at `awaiting_approval` with all candidates rendered, which is exactly the gated path. |

**Verdict: the judge is a configured agent, a column pair, and a real step.**

*Configuration.* `step_graph_phase.judge_agent_id UUID REFERENCES agent(id)` and
`judge_model TEXT`, both nullable (0003). Resolution chain per §4.1. `judge_agent_id IS NULL` on a
phase with `fan_out > 1` means **human selection is required regardless of the gate**: the phase
parks at `awaiting_approval` after the candidates settle, and `R-TUI-4`'s "select fan-out result"
action resolves it. That is the safe default and it means an unconfigured project never silently
delegates a choice to a model.

*Selection routing.*

| `fan_out` | `gate_effective` | `judge_agent_id` | Selection |
|---|---|---|---|
| 1 | any | any | none; `selected` stays NULL as the DDL comment says |
| > 1 | `always`, or `on_failure` that tripped | any | human, at the gate (`R-ORCH-7`: "a human choice when gated") |
| > 1 | not stopping | set | judge step |
| > 1 | not stopping | NULL | human; the step parks at `awaiting_approval` anyway |

*The judge run is a `run_step`.* `position` = the judged phase's position, `attempt` = the judged
attempt, **`fanout_index = -1`**, `phase_name = "<phase>:judge"`, `agent_id` / `model` from the
judge columns. `run_step.fanout_index` has no CHECK constraint, so -1 is legal today and
`UNIQUE (run_id, position, attempt, fanout_index)` is satisfied; the SQL comment `-- 0..fan_out-1`
becomes `-- 0..fan_out-1; -1 = the R-ORCH-7 judge step` in `0003`. The Rust doc comment on
`RunStep.fanout_index` (`0..fan_out` in Rust range notation) gains the same sentence, resolving the
cosmetic SQL-versus-Rust range mismatch in the same change.

*What the judge receives.* The prompt is assembled by ANA-5 from a template named `judge`, with
these sections and no others:

| Section | Content |
|---|---|
| task | the judged phase's own assembled prompt, verbatim, so the judge knows what was asked |
| candidates | one block per surviving candidate, in `fanout_index` order (and reversed on the second call): the `fanout_index`, the candidate's output document body, the diff stat and unified diff over `before_hash..after_hash` per repo, `verify_outcome`, `verify_exit_code`, and the tail of the verification output |
| instruction | return the winning `fanout_index` and a one-line reason per candidate |

The judge does **not** receive any candidate's session transcript. That is `R-PRM-1`'s rule and it
is also what bounds the call: SWE-agent truncates an over-long submission to the literal string
"Solution invalid." at `max_len_submission: 5000` rather than letting one candidate crowd out the
others, and `htui`'s equivalent is ANA-5's trim order applied per candidate block, with a candidate
whose diff exceeds its share rendered as a diff stat only.

*What the judge emits.* A document of kind `judge` (an open `document.kind`, `R-ENT-12`) produced by
the judge step, whose body is the ranking table and the per-candidate reasons, plus the winning
index parsed from a fenced JSON block:

```json
{ "winner": 2, "reasons": { "0": "...", "1": "...", "2": "..." } }
```

The orchestrator parses that block, never prose. A missing or unparseable block, a `winner` outside
the surviving set, or a disagreement between the two orderings is a judge failure.

*Bookkeeping after selection.* One transaction:

| Row | Write |
|---|---|
| winner `run_step` | `selected = true`, `status = 'done'` |
| every other candidate | `selected = false`, `status = 'superseded'` |
| judge `run_step` | `status = 'done'`, `gate_note` = the one-line reason for the winner |
| losers' documents | untouched; they stay, and §4.2's resolver excludes them |
| losers' trees | untouched until the run is terminal (§4.6); their branches are the history `R-ORCH-7` asks for |

*Judge failure is never a run failure.* On any of the failure modes above, or on the judge's own
quota being exhausted (§7), or on a judge whose agent is missing on this box: the judge step goes to
`failed` with `gate_note` naming the reason, the judged steps stay `done` with `selected` NULL, the
run parks at `awaiting_approval`, and the item goes to `blocked`. A human then picks. Nothing is
lost and no choice is fabricated.

*Cost accounting.* The judge's `usage` is on its own `run_step` row, so it is included in the batch
sum of §7 and excluded from any per-candidate figure. SWE-agent keeps the same separation for the
same reason: its attempt statistics "only accumulate the states of the sub-agent, not the reviewer".

*Bounds.* `fan_out` is capped at `app_setting.max_fan_out`, default **4**. The only shipped product
with a human-select fan-out, Codex cloud, caps attempts at 4
(https://help.openai.com/en/articles/11428266-codex-changelog). A run is additionally capped at
`app_setting.max_agents_per_run`, default **8** (was 6; raised by MOD-4 milestone 4's
`0004_max_agents_per_run_default.sql`, because 6 refused the seeded `feature` graph's judged 3-way
`implement`; amended by MOD-4 milestone 6, 2026-09-25), counting every candidate plus every judge plus every
retry attempt planned so far. Both refusals are loud: the run is refused at admission naming the cap
and the requested figure, never silently truncated, following Anthropic's stated reason for refusing
an over-long parallel list rather than capping it ("A silent cap would drop part of the workload
without telling the script", https://code.claude.com/docs/en/workflows). Both numbers are
**Open for the maintainer 2**.

*What this deliberately does not build.* `R-ORCH-7` is rival fan-out: N sessions, one prompt, one
winner, the rest history. The maintainer's own practice is task fan-out: N sessions, N prompts,
disjoint declared file sets, every result merged, no losers and no judge (`SKILL.md:129-133`; MOD-1
wave C, MOD-6 wave B, merge commit `73ac577`). The two are different mechanisms and `R-ORCH-7`
describes only the first. `htui` v1 builds only the first, and the isolation and overlap machinery of
§4.6 and §4.7 is the half that both would share. **Open for the maintainer 1** carries the question.

### 4.6 Isolation modes and commit capture (`R-ORCH-8`, `R-ORCH-11`, `R-ID-4`)

**The constraint.** `R-ORCH-8` (`docs/REQUIREMENTS.md:176-179`), verbatim:

> "Fan-out isolation mode per project or phase: `worktree` (default, one git worktree per session),
> `copy` (directory copy for trees where worktrees break build caches), `shared_serialized` (one
> tree, sessions run one after another), `local` (run directly in the working tree with no isolation
> and no serialization)."

`R-ID-4` (`:33-35`): "`htui` writes no files into managed repositories. The only changes to a
working tree are made by agents doing the item's work."

**Options.**

| Option | Verdict | Reason |
|---|---|---|
| One `run_step.isolation_path TEXT` describes the step's tree | Rejected as sufficient | `R-ENT-3` gives a project "one or more repos, one marked primary" and `run_step_commit` is already keyed `(run_step_id, repo_id)`, so a multi-repo step needs one path, one mode and one base ref per repo. One TEXT column cannot carry three fields times N repos. |
| Put a JSON blob in `isolation_path` | Rejected | It would be the only untyped structure on a row whose every other field is typed, unqueryable for the overlap predicate of §4.7, and not mirrorable in a useful shape. |
| **A `run_step_tree(run_step_id, repo_id, mode, path, base_ref, dirty)` table; `isolation_path` keeps the primary repo's path** | **Adopted** | Per-repo by construction, joins to `run_step_commit` on the same key, and `isolation_path` stays populated so the shipped mirror column and any existing reader keep working. |
| Place worktrees inside the managed repo (for example `<repo>/.htui/worktrees/`) | Rejected | `R-ID-4` forbids `htui` writing into a managed repository, and a worktree directory inside the repo would appear in `git status` of the main tree. |
| Place them under a box-local scratch root outside every `repo_box_path` | **Adopted** | `<config_dir>/trees/<run_id>/<step_id>/<repo_slug>/`, validated against every known `repo_box_path` prefix before creation. |
| Justify `copy` on Rust build caches, as `R-ORCH-8` does | Rejected as written | A worktree does not break a Cargo cache; it starts cold. What breaks is *sharing* one `CARGO_TARGET_DIR` across divergent worktrees: cargo#14053 documents a race where "locks are only held on a per-target basis, thus leading to a race condition where a dependency has been overwritten". And copying a `target/` does not preserve it either: the `.d` files hold absolute paths, and one measurement put a `target/` copy at about two minutes, "almost as bad as doing a cold build in the first place" (https://blog.howardjohn.info/posts/shared-rust-build/). |
| Re-justify `copy` on the cases where it is genuinely the only option | **Adopted** | Three named cases, below. |

**Verdict: isolation is per repo, and each mode is a triple (tree setup, serialization,
reconciliation).**

| Mode | Tree | Serialization | Build cache | Reconciliation | Refused when |
|---|---|---|---|---|---|
| `worktree` (default) | `git worktree add --lock <path> -b htui/<step_id> <before_hash>` per (step, repo) under the scratch root | none; siblings run in parallel | cold per tree; **never** share `CARGO_TARGET_DIR` across them; a compiler cache (`sccache` as `RUSTC_WRAPPER`) is the supported way to warm them | winner's branch is merged, or cherry-picked, into the primary tree's current branch | the repo has submodules; `git worktree`'s own BUGS section says "the support for submodules is incomplete. It is NOT recommended to make multiple checkouts of a superproject" (https://git-scm.com/docs/git-worktree) |
| `copy` | filesystem copy of the repo, including its `.git`, minus the excluded build directories, then `git reset --hard <before_hash>` in the copy | none | cold, and *deliberately* cold: a copied configured CMake build tree is silently wrong, not cold, because `CMakeCache.txt` and every generated build file hold absolute paths (https://cmake.org/pipermail/cmake/2007-June/014502.html) | the copy carries its own `.git`; the winner's branch is fetched from the copy into the primary repo and merged | the box has no room for N copies of the tree (checked before creation) |
| `shared_serialized` | the `repo_box_path` tree itself | a Postgres advisory lock keyed `(box_id, repo_id)` held for the whole step; fan-out siblings run one after another, each reset to `before_hash` before it starts and committing to `htui/<step_id>` | fully warm, which is the only reason to choose it | the winner's branch is checked out; the losers' branches remain | the tree is dirty at step start and the user has not accepted the reset |
| `local` | the `repo_box_path` tree itself | none, by requirement | fully warm | nothing to reconcile: the work is already in the tree | `fan_out > 1`; or any other non-terminal run holds any tree in this repo on this box (§4.7) |

*Why `copy` still exists, restated honestly.* Not for Cargo. It is the mode for: a repo with
submodules or LFS where `git worktree` is refused or unreliable; a toolchain whose cache is
path-anchored **outside** the tree and keyed on the tree path, so a worktree at a new path is cold
anyway and a copy at least reproduces the layout; and a tree that is not a git checkout at all. The
mandatory exclusion list is `project.settings.copy_exclude`, defaulting to
`["target/", "build/", "cmake-build-*/", "node_modules/", ".venv/", "out/", "dist/"]`, and it is
mandatory precisely because copying those directories is at best slow and at worst produces a build
tree that is wrong rather than absent.

*The Windows cost is real and is shown.* Reflink and block cloning are unavailable on stock NTFS;
`copy` there is a full byte-for-byte copy of the tree
(https://www.ctrl.blog/entry/file-cloning/). MOD-4 measures the source tree before the first copy of
a run and refuses with a named size when N copies would exceed
`app_setting.copy_max_total_bytes`. Whether `copy` is offered at all by default on Windows is
**Open for the maintainer 4**; the default is "offered, with the size shown".

*Git contention is measured and must be retried.* Worktrees share the object store, the ref store and
the index lock, so concurrent `git add` / `git commit` across worktrees contend: with thirteen
parallel agents, "5 committed successfully, 8 failed due to lock contention"
(https://github.com/anthropics/claude-code/issues/55724). Every git write MOD-4 issues retries three
times with exponential backoff at 200 ms, 400 ms and 800 ms on `index.lock` and `cannot lock ref`
errors. `git gc --prune=now` is forbidden while any run on the box is non-terminal, per git-gc's own
warning that it "increases the risk of corruption if another process is writing to the repository at
the same time" (https://git-scm.com/docs/git-gc).

*Branch exclusivity is a free interlock.* Naming the branch `htui/<step_id>` makes it unique per
step, and `git worktree add` "will refuse to create the worktree" for a branch already checked out
elsewhere. That refusal is a second, independent guard against two steps sharing a tree.

*Base ref.* `worktree` and `copy` branch from **the primary repo's current HEAD at step start**, not
from the default branch. Claude Code branches its subagent worktrees from the default branch instead
(https://code.claude.com/docs/en/sub-agents), which is right for an independent task and wrong here:
`htui` runs a graph whose later phases must see the earlier phases' commits, so a phase that branched
from `main` would silently discard the plan phase's work. `run_step_tree.base_ref` records which
hash was used, so the choice is auditable rather than implicit.

**Commit capture, per repo, in every mode including `local`.**

| When | Write |
|---|---|
| stage 2 (prepare), per repo in scope | `run_step_tree(run_step_id, repo_id, mode, path, base_ref, dirty)` and `run_step_commit(run_step_id, repo_id, before_hash, NULL)` |
| stage 5 (settle), per repo in scope | `run_step_commit.after_hash = <HEAD in that tree>`, or left NULL when the step committed nothing |

`before_hash` is `git rev-parse HEAD` in that repo, taken before the agent starts, which puts git on
the critical path ahead of the first token in every mode. It is `NOT NULL`, and for a `local` or
`shared_serialized` step on a dirty tree it is still the HEAD hash, with
`run_step_tree.dirty = true` recording that uncommitted work was present. That is the answer to "who
supplies `before_hash` for a `local` step": the tree always has a HEAD, and `dirty` is what
distinguishes "the step started from this commit" from "the step started from this commit plus
whatever the user had open". A `dirty` tree also makes the step **non-resettable** (§4.9).

A step normally produces several commits, as every close-out in this repo shows (MOD-1 six commits,
MOD-6 ten). `after_hash` is therefore the step's final HEAD, and the step's work is the range
`before_hash..after_hash`; no row is written per commit and none is needed, because the range is
exact and `git log` recovers the rest.

**Winner reconciliation back to the primary tree.** One transaction plus one git sequence, run by
the orchestrator, never by an agent:

1. Confirm the primary tree is clean, or refuse and park at `awaiting_approval` with
   `dirty_primary_tree`.
2. `worktree` and `shared_serialized`: `git merge --no-ff htui/<winner_step_id>` in the primary
   tree. `copy`: `git fetch <copy_path> htui/<winner_step_id>` first, then the same merge.
3. On a merge conflict: stop, leave the branch in place, park at `awaiting_approval` with
   `merge_conflict` and the conflicting path list. The orchestrator never resolves a conflict; Gas
   Town's Refinery does resolve them and states the price plainly ("The refinery prioritizes
   throughput over perfect accuracy, relying on subsequent review to catch errors",
   https://yegge.ai/gastown), which is a trade `R-ID-6` does not permit here.
4. Record the merge commit as the *run's* reconciliation hash by updating the winner step's
   `after_hash` to the merge commit in the primary repo's `run_step_commit` row.

Losing branches are kept until the run is terminal and then, at close-out, kept: they are the
history `R-ORCH-7` requires, they cost one ref each, and deleting them would destroy the only
artefact of a losing candidate that a diff can be recovered from.

**Cleanup.** Trees are removed only when the run reaches `done`, `failed` or `cancelled`, never at
step end, and never while a step is `awaiting_approval` (invariant 6; a promoted chat needs its tree,
§4.8). `git worktree remove` for `worktree`, a directory delete for `copy`, lock release for
`shared_serialized`, nothing for `local`. A worktree that produced no commit is removed at step end
as a special case, matching Claude Code's "The worktree is automatically cleaned up if the subagent
makes no changes". `git worktree prune` is run once after every cleanup. The `--lock` taken at
creation is what stops an unrelated `prune` from removing a live tree.

**Fan-out and isolation interact.** `fan_out > 1` requires `worktree`, `copy` or
`shared_serialized`; `local` with `fan_out > 1` is refused at snapshot time, because a mode defined
as "no isolation and no serialization" cannot run two sessions over one tree at once without them
overwriting each other. `shared_serialized` with `fan_out > 1` is legal and is exactly what
`R-ORCH-8` describes: the sessions run one after another in the one tree, each reset to
`before_hash` first.

### 4.7 The overlap rule for concurrent items (`R-ORCH-9`)

**The constraint.** `R-ORCH-9` (`docs/REQUIREMENTS.md:180-182`), verbatim:

> "Concurrent items: both modes may run several items at once when they do not overlap. Overlap:
> same repo unless both use isolated trees, or declared touched-path sets intersect. Overlapping
> items are serialized; others run in parallel up to a per-box limit."

**Options.**

| Option | Verdict | Reason |
|---|---|---|
| Derive "same repo" from `run_step_commit` | Rejected | `run_step_commit` is written after the fact, at stage 2 of a step that has already been admitted. Admission needs the repo set before anything runs. |
| Derive it from the project: two items in one project always share every repo | Rejected as sole rule | It is safe but serialises everything in a multi-repo project, which is the case `R-ENT-3` exists for. |
| **Store an explicit `run.repo_scope UUID[]` at queue time** | **Adopted** | Computed once from the item's `touched_paths` and the project's repos, queryable with the `&&` array-overlap operator, and mirrorable. |
| Keep `touched_paths` globs repo-relative and unqualified | Rejected | `docs/ANA-9.md:591` documents them as "repo-relative globs" with no repo qualifier, so in a two-repo project two items each declaring `src/**` falsely intersect. |
| **Qualify as `repo_name:glob`, with a bare glob meaning the primary repo** | **Adopted** | Backward compatible with every existing value (`crates/htui-core/src/fixtures.rs` and the demo loader write bare globs), and the primary repo is already unique per project via `uq_repo_primary`. |
| Intersect globs with a real matcher | Rejected for v1 | It needs a glob crate the workspace does not have, and glob-versus-glob intersection is not what a matcher computes (matchers test a path against a pattern, not a pattern against a pattern). |
| **Intersect on the non-wildcard prefix: two globs overlap when either prefix is a prefix of the other** | **Adopted** | Decidable, dependency-free, and conservative in the safe direction: it over-reports overlap (serialising a pair that would not have collided) and never under-reports. |
| Empty `touched_paths` means "touches nothing" | Rejected | It would make the safest-looking item the most dangerous one. |
| **Empty `touched_paths` means unknown, and unknown overlaps the whole primary repo** | **Adopted** | Declaration becomes the way to buy parallelism, which is the maintainer's own rule: "implementer fan-out never trusts prose independence, file-set intersection decides" (`.claude/skills/handoff-run/SKILL.md:170`). |
| Let agents self-coordinate instead of declaring | Rejected | Measured worse: Co-Coder reports Claude Code agent teams at the lowest pass rate (54.1%) despite the lowest latency, concluding that "self-coordinated inter-agent orchestration cannot substitute for explicit cohesion-aware task partitioning" (https://arxiv.org/html/2606.00953v1). A study of 33,596 agent PRs puts the textual conflict rate between identical concurrent agents at 19.8% (https://arxiv.org/html/2607.04697v2). |

**Verdict: overlap is a decidable predicate over `(repo scope, isolation, touched paths)`, and the
per-box limit is a separate, later check.**

*Inputs, all resolved at queue time and stored on the run.*

```rust
// crates/htui-orch/src/overlap.rs - design notation, not code

/// Everything admission needs about one run, resolved once at queue time and stored.
pub struct RunScope {
    pub run_id: RunId,
    pub box_id: BoxId,
    pub repos: Vec<RepoId>,                        // run.repo_scope
    pub isolated: BTreeMap<RepoId, bool>,          // worktree | copy => true; shared_serialized | local => false
    pub local: BTreeMap<RepoId, bool>,             // the `local` mode specifically
    pub paths: BTreeMap<RepoId, Vec<PathPrefix>>,  // normalised touched_paths, empty = whole repo
}
```

`repos` is every repo named by a qualified `touched_paths` entry, plus the primary repo when any
bare glob or no glob at all is declared; a phase whose isolation resolves per project rather than per
repo applies to every repo in scope. `isolated[r]` is true when **every** phase of the snapshot uses
`worktree` or `copy` for `r`; a graph that mixes an isolated implement phase with a `local` verify
phase is not isolated, because the non-isolated phase is where the collision happens.

*The predicate.*

```
overlaps(A, B) :=
    let R = A.repos INTERSECT B.repos
    if R is empty                                      -> false
    for each r in R:
        if A.local[r] or B.local[r]                    -> true      # rule L
        if not (A.isolated[r] and B.isolated[r])       -> true      # rule I, R-ORCH-9 clause 1
        if intersect(A.paths[r], B.paths[r])           -> true      # rule P, R-ORCH-9 clause 2
    -> false

intersect(xs, ys) :=
    xs is empty or ys is empty                         -> true      # unknown overlaps everything
    exists x in xs, y in ys: x startswith y or y startswith x
```

`PathPrefix` is the glob truncated at its first wildcard metacharacter (`*`, `?`, `[`, `{`) and then
at the last `/`, so `src/**/*.rs` becomes `src/`, `crates/htui-core/src/model/item.rs` stays whole,
and `**` becomes the empty prefix, which is a prefix of everything and so overlaps everything. That
last case is deliberate and is what makes a `**` declaration equivalent to no declaration.

Three things this predicate is careful about. **Rule P applies even when both runs are isolated**,
because two isolated trees editing the same files do not collide during execution but collide
guaranteed at reconciliation, which is where Gas Town's whole Refinery exists ("You merge the first
one. The other nine are now stale.", https://yegge.ai/gastown). **Rule L is unconditional**, which
is how the requirement's own tension resolves: `R-ORCH-8` says `local` has "no serialization" and
`R-ORCH-9` says non-isolated same-repo items are serialized. The reading adopted is that `local` does
not serialize *internally* (no lock, no queueing behind a sibling) and is instead **refused** when
another non-terminal run holds any tree in that repo on that box, with a message naming the holding
run. A refusal is honest where a silent queue would contradict `R-ORCH-8`. **The predicate ranges
over non-terminal runs**, including `awaiting_approval` ones, because a parked run still owns its
trees and its unmerged branch (invariant 6).

*Admission.* Overlapping runs are serialized by leaving the loser in `queued`. Non-overlapping runs
proceed to the per-box limit:

```sql
-- one transaction; the row lock is the critical section
SELECT settings FROM box WHERE id = $box FOR UPDATE;
SELECT count(*) FROM run
 WHERE executing_box_id = $box AND status = 'running';
-- admit iff count < max_concurrent_items AND no overlapping non-terminal run
UPDATE run SET status = 'running', executing_box_id = $box, started_at = now(),
               lease_box_id = $box, lease_owner = $owner, lease_expires_at = now() + $ttl
 WHERE id = $run AND status = 'queued';
```

The `FOR UPDATE` on the box row is the critical section, mirroring Airflow's, which takes "a
row-level write lock on every row of the Pool table"
(https://airflow.apache.org/docs/apache-airflow/stable/administration-and-deployment/scheduler.html).
It counts `running` only: an `awaiting_approval` run consumes no compute and must not hold a slot,
the mistake Restate warns about for an exclusive handler that awaits an external event ("all other
calls to this object will be queued", https://docs.restate.dev/develop/ts/external-events). It counts
**claimed** work rather than started work, because the count and the claim are the same transaction;
Airflow's own bug class is the opposite ("the scheduler is for some reason queuing more tasks than
there are slots in the pool", https://github.com/apache/airflow/issues/15793).

*Typed settings, at last.* `box.settings` and `project.settings` are JSONB comments today with no
Rust type, no default and no accessor, and `BoxInfo` cannot reach either. MOD-4 adds:

```rust
// crates/htui-core/src/model/box_.rs and hierarchy.rs - design notation, not code

pub struct BoxSettings {
    pub max_concurrent_items: u32,               // R-ORCH-9; app_setting default 2
    pub command_limits: BTreeMap<String, u32>,   // R-MCP-3; app_setting default { build: 1, test: 4, verify: 1 }
}

pub struct ProjectSettings {
    pub default_isolation: Isolation,            // default Worktree
    pub token_budget: Option<i32>,
    pub retention_days: Option<i32>,
    pub cached_transcript_steps: Option<i32>,
    pub keep_raw_events: bool,
    pub per_token_cap_run: Option<i64>,          // micros
    pub per_token_cap_batch: Option<i64>,        // micros
    pub step_deadline_seconds: Option<u32>,
    pub default_agent_id: Option<AgentId>,
    pub judge_agent_id: Option<AgentId>,
    pub copy_exclude: Vec<String>,
}
```

Both are `Deserialize` with `#[serde(default)]` on every field, so a hand-edited `'{}'` stays valid,
which is what the `DEFAULT '{}'` on both columns already promises. The defaults come from
`app_setting`, and MOD-4 extends `SEEDED_SETTINGS` (currently two cache keys) with:
`max_concurrent_items` 2, `command_limits` `{"build":1,"test":4,"verify":1}`, `default_isolation`
`"worktree"`, `step_deadline_seconds` 7200, `max_fan_out` 4, `max_agents_per_run` 8 (6 until MOD-4's `0004`; amended by MOD-4 milestone 6, 2026-09-25),
`copy_max_total_bytes`, and the two `R-AGT-7` caps as NULL. `max_concurrent_items = 2` matches the
demo fixture (`crates/htui-core/src/fixtures.rs:375`) and is **Open for the maintainer 3**.

*Projection changes this forces.* `touched_paths` is on the full `Item` only, not on `ItemSummary`,
so an N-item overlap check would cost N `item()` round trips. MOD-4 adds `touched_paths` to
`ItemSummary` (it is already in the mirror, `cache_migrations/0001_mirror.sql:80`, so no mirror
change is needed) and adds `probed_tags`, `declared_tags` and `settings` to `BoxInfo`, which today
carries only `box_id`, `hostname` and `os_family`. Without the second change neither the capability
check nor the per-box limit is reachable from the TUI seam at all.

*Auto mode uses the same predicate.* `R-ORCH-9` says "both modes", so MOD-12 does not get a second
rule: it calls the same admission with the same transaction, having first selected candidates with
the ready query of §4.10.

### 4.8 Promotion to chat and resume from the next step (`R-ORCH-5`, `R-TUI-6`, `R-HIS-2`)

**The constraint.** `R-ORCH-5` (`docs/REQUIREMENTS.md:165-167`), verbatim:

> "Any step can be promoted to an interactive chat, on failure or by user request, preserving the
> session context. When the chat yields the step's artifact, the pipeline resumes from the next
> step."

**Options.**

| Option | Verdict | Reason |
|---|---|---|
| The promoted chat is a new `run(kind='chat')` linked to the graph step by a new column | Rejected | It forks the transcript across two `run_step` rows, which breaks `R-HIS-2`'s single-step replay ("The chat view can reopen any past step read-only and replay it") and would need `promoted_from_step_id` plus a rule for whose `usage` and whose commit hashes count for `R-ORCH-11`. |
| **The promoted chat is the same `run_step`, continued with `follow_up` events** | **Adopted** | `session_event.turn` "increments on each prompt/follow_up" and `follow_up` is already in the fourteen-kind vocabulary, so the mechanism exists. `R-TUI-6` explicitly allows a chat "Bound to a run step", which is what this is. One step, one transcript, one usage total, one set of commit hashes. |
| No marker at all, so a promoted step is indistinguishable from a gated one | Rejected | The Runs tab must render it differently, and resume must know which step to reattach a live session to. |
| **`run_step.promoted_at TIMESTAMPTZ`** | **Adopted** | One nullable column, no new vocabulary. |
| Migrate a column for the agent-side session id | Rejected | ANA-4 already decided against it, verbatim (`docs/ANA-4.md:616-619`): "That id has no column in ANA-9, and rather than migrate for it, MOD-2 records a session banner as the step's first `other` row ... Resuming is a query for that row." |
| Infer "the chat yielded the artifact" from the transcript | Rejected | There is no sound way to tell which assistant message was the deliverable, and guessing wrong resumes the graph on the wrong document. |
| **An explicit `accept artifact` action, guarded on a document of `output_kind` existing** | **Adopted** | One click, no inference, and it is the same guard the gate already uses. |

**Verdict: promotion is in place, and resume is the ordinary gate answer.**

*Promotion.* Available from the Runs tab whenever the step is `running`, `awaiting_approval` or
`failed` (the "on failure or by user request" of `R-ORCH-5`). On promotion:

| Row | Write |
|---|---|
| `run_step` | `status = 'awaiting_approval'` if not already, `promoted_at = now()` |
| `run` | `status = 'awaiting_approval'` |
| `item` | `status = 'awaiting_approval'` |
| `run.kind` | unchanged; it stays `'graph'` |
| `run_step_tree` rows | unchanged and **not cleaned up**: the chat works in the step's own tree |

A promoted step that was `running` is cancelled first with the driver's grace window, so its
`session/cancel` and every parked permission answer land before the chat attaches (ANA-4 §4.3 makes
answering every outstanding request a MUST).

*Amended by MOD-4 milestone 6, 2026-09-25 (plan OQ-5, D163, R-38):* as built, a `running` step is
promoted by **preempting its walk**. The run's cancel token drops the walk, which kills the agent
through `ChildGuard::drop`; the engine then moves the step `running → awaiting_approval` and calls
`promote_step`. The grace window and the answers to parked permission requests are **not** honoured.
The transcript is intact up to the kill. A graceful path needs a cancel seam inside the driver's
pump and is carried in MOD-37.

*Preserving the session context.* Two paths, chosen by `DriverCaps`:

| Condition | Path |
|---|---|
| `DriverCaps.follow_up_in_session` and the session is still live | send a `follow_up` into the running session; `turn` increments; nothing respawns |
| the session ended, and `DriverCaps.resume` | query the step's first `other` row with `update = "session_started"` for the agent-side id, then `session/load` or `session/resume` (ACP) or `claude --resume <id>` (CLI); the driver replays or restores, and `htui` drops the replayed `user_message_chunk` rows as ANA-4 §6.1 specifies |
| neither | ANA-5 builds a **handoff prompt** from the step's own stored transcript (a summary, the input documents, the diff so far, the failure reason) and starts a fresh agent session against the same `run_step`, same `isolation_path`, next `turn` |

*Amended by MOD-4 milestone 6, 2026-09-25 (blueprint F-G, D192, D193, R-48):* as built, the first
path is unreachable: a walk's session ends at its `done` before the step parks, and a preempted one
was killed. The second path is taken only for a **CLI** agent with `DriverCaps.resume` and a
`session_started` banner. The ACP driver never reads `SessionSpec.resume`, so every ACP step gets
the handoff path, and ACP `session/load` is carried in MOD-37. A resumed session is opened with
`htui`'s own one-sentence follow-up (`promote::RESUME_OPENING`), recorded as the `follow_up` at the
next `turn`. Either way the step's `prompt_digest` and `trim_record` are never rewritten.

The third path is what makes `R-ORCH-5` true for every agent rather than only for resumable ones.
Claude Code's Explore and Plan agents "return no agent ID, so Claude can't resume them"
(https://code.claude.com/docs/en/sub-agents), and an ACP agent may advertise neither `loadSession`
nor `sessionCapabilities.resume`; a design that assumed a handle would simply refuse to promote those
steps. It is a fresh model context, not a fake one, and the Runs tab says so.

*The cwd coupling is a hard constraint, and it is why trees survive a park.* Claude Code's session
lookup "is scoped to the current project directory and its git worktrees", and resuming from a
different directory "creates a new session" (https://code.claude.com/docs/en/cli-reference). A
promoted step must therefore run from the same `run_step_tree.path` it ran from originally, which is
exactly why §4.6 defers cleanup to run termination rather than step end.

*The retention objection, answered.* The `session_started` row is swept with its step by
`R-HIS-3`'s retention pass, and the mirror keeps events only for the last N *finished* steps. Neither
matters: the retention sweep removes only steps of terminal runs, and a promotable step belongs to a
non-terminal run; and the mirror is never the orchestrator's store (invariant 10). ANA-4's
no-column decision is therefore accepted without amendment.

*"When the chat yields the step's artifact".* Mechanically: a `document` exists whose `item_id` is
the run's item, whose `kind` is the phase's `output_kind` and whose `produced_by_step_id` is this
step, **and** the user invokes `accept artifact`. On that action the orchestrator runs the rest of
stage 5 and stage 6 exactly as if the session had ended normally:

1. run `verify_command`, writing `verify_outcome` / `verify_exit_code`;
2. capture `after_hash` per repo;
3. resolve the gate; on `approved` the step goes `done` with `gate_outcome = 'approved'`;
4. **resume at `position + 1`** (or at the fan-out selection when this was a candidate), which is
   `R-ORCH-5`'s "the pipeline resumes from the next step".

A promoted step can also be answered `retried` (discard and re-run the phase), `rejected` (fail it),
or `cancel`. Promotion adds no new outcome vocabulary at all.

*Amended by MOD-4 milestone 6, 2026-09-25 (plan D166, blueprint D194, D211, D212):* `accept
artifact` is refused while a chat on the step is live, and it is refused when `verify_command`
fails (the outcome is recorded and the step stays promoted), which the list above leaves open. An
`unavailable` verify is recorded and noted, not refused. The verify's deadline is a fresh copy of
the phase deadline, measured from the accept. `GateAnswer::Skipped` is not exposed as an accept
shape: accept needs the document and lands `approved`. While a chat is live on any step of the run,
approve, reject, retry, select and cancel are refused too.

*Free-standing chat is untouched.* `run(kind='chat', item_id NULL)` remains what
`crates/htui-store/src/cache/pending.rs` already writes offline, and MOD-4 neither reads nor writes
it. The two chat shapes stay disjoint: bound-to-a-step chat is a promoted graph step, free-standing
chat is a chat run.

### 4.9 Resume of an interrupted run (`R-HIS-1`, `R-STO-4`, `R-ORCH-11`)

**The constraint.** No requirement names crash resume directly. The three that bound it are
`R-HIS-1` ("Nothing about a run exists only on one box"), `R-STO-4` ("When Postgres is unreachable,
the TUI opens in offline read-only mode from the cache ... No item creation, no runs"), and
`CONCEPTS.md`'s one-cache-writer rule.

**Options.**

| Option | Verdict | Reason |
|---|---|---|
| Auto-resume a `running` step after a restart by re-attaching to the agent | Rejected | The child process, the ACP connection, the parked permission responders and the recorder's coalescing buffer are all gone (`docs/ANA-4.md:321-330`). There is nothing to re-attach to. |
| Auto-re-run a `running` step after a restart | Rejected as unconditional | The step may have half-mutated a tree, so a blind re-run replays against unknown state. DBOS states the general form: a non-transactional step "may execute twice ... they should be idempotent or otherwise resilient to re-execution" (https://www.dbos.dev/blog/why-postgres-durable-execution). |
| **Re-derive the step's outcome from its durable artefacts, and reset the tree before any re-run** | **Adopted** | `after_hash` plus an output document is a complete completion test, and `before_hash` is a complete reset target. Both are already `NOT NULL` or written at stage 5. |
| Detect a dead run from `box.last_seen_at` | Rejected | It is per box, not per run, so it cannot distinguish a crashed orchestrator from a healthy one on a box that is otherwise alive, and it says nothing about which process owned the run. |
| **A run-level lease with an owner and an expiry, refreshed at half the TTL** | **Adopted** | It is the gap Microsoft Agent Framework is criticised for leaving open ("there is no automatic failure detection. The workflow runner has no heartbeat, no lease mechanism, no watchdog", https://www.diagrid.io/blog/still-not-durable-how-microsoft-agent-framework-and-strands-agents-repeat-the-same-mistake), and Oban's answer is the shape adopted: an `INSERT ... ON CONFLICT` with a TTL where "The leader refreshes at 2x the normal rate to hold the lease" (https://www.dimamik.com/posts/oban_py/). |
| Cancel a live run when the store goes offline | Rejected | It cannot be recorded: `Backend::writable()` returns `None` offline, so the cancellation itself is unwritable. |
| **Let the live session finish into the existing pending buffer; the lease expires; the reconnect sweep adjudicates** | **Adopted** | It reuses machinery MOD-6 already shipped and loses nothing. |

**Verdict: what is durable, and what restarts.**

| Durable (Postgres, and mirrored where the mirror carries it) | Not durable |
|---|---|
| `run` (status, mode, boxes, `graph_snapshot`, `repo_scope`, `failure`, timings) | the agent child process and its process tree |
| `run_step` (status, `position`, `attempt`, `fanout_index`, `selected`, `gate_outcome`, `gate_note`, `prompt_digest`, `verify_outcome`, `verify_exit_code`, `promoted_at`, partial `usage`) | the ACP connection and the session task |
| `run_step_tree` (mode, path, base ref, dirty) and `run_step_commit` (`before_hash`, `after_hash`) | parked permission responders |
| `session_event` rows to the last flush, plus the `session_started` banner | the recorder's coalescing buffer (flushed on kind change, message-id change, `Done`, 16 KiB, session end) |
| `document` versions and the `produced_by_step_id` stamp | in-memory queues, admission counters, the overlap set |
| `command_run` rows | the worktree or copy directory itself, if a human deleted it |

`run_step.usage` is durable to the last flush by construction: "The recorder maintains a running
total over the `usage` rows of the step and writes it to `run_step.usage` at every flush and at step
end, so a crashed step still has a partial figure" (`docs/ANA-4.md:1105-1107`).

**The lease.** `0003` adds `run.lease_box_id UUID REFERENCES box(id)`, `run.lease_owner UUID` and
`run.lease_expires_at TIMESTAMPTZ`. `lease_owner` is minted per orchestrator process at start, so a
second `htui` on the same box cannot silently adopt the first's runs. TTL default **120 seconds**,
refreshed every **60**, both `app_setting` keys and **Open for the maintainer 6**. The refresh is one
`UPDATE ... WHERE id = $run AND lease_owner = $owner`; a zero-row refresh means the lease was taken
and the orchestrator abandons the run without writing further, which is the single-writer rule of
invariant 1 applied to processes.

**The recovery sweep** runs at orchestrator start and every TTL thereafter:

```sql
SELECT * FROM run
 WHERE status = 'running'                    -- queued runs have no executing box yet; admission
   AND executing_box_id = $this_box          --   picks them up. R-ORCH-12: local execution only
   AND (lease_expires_at IS NULL OR lease_expires_at < now());
```

For each adopted run, for each `run_step` in `running`:

| Condition | Action |
|---|---|
| `after_hash` present for every repo in scope **and** a document of `output_kind` produced by this step exists | the step finished and only the bookkeeping was lost: `status = 'done'`, then re-derive the run status and continue the walk |
| otherwise, and every tree is resettable (`mode IN ('worktree','copy')`, or `mode IN ('shared_serialized','local')` with `dirty = false`) | reset each tree to `before_hash`, `status = 'failed'` with `gate_note = 'interrupted'`, then §4.4's retry admission: a new attempt if budget remains, else the run parks at `awaiting_approval` and the item goes to `blocked` |
| otherwise (a `dirty` `local` or `shared_serialized` tree) | **never reset**: resetting would destroy the user's own uncommitted work. `status = 'failed'` with `gate_note = 'interrupted, tree not reset'`, run parks at `awaiting_approval`, item `blocked`, and the Runs tab names the tree and the `before_hash` so a human can decide |

The first row is what River's own documentation identifies as the case a naive rescuer gets wrong:
"if a job finishes successfully, but fails to be marked as completed, in which case it'll be rescued
and run again" (https://riverqueue.com/docs/reliable-workers). Checking the artefact rather than the
status is what avoids it.

Steps in `awaiting_approval` are never touched by the sweep: a gate holds no lease (invariant 6), and
a run parked for a week is not a crashed run. Steps in `pending` are simply re-scheduled.

**No prompt-digest replay.** Anthropic's workflow runtime resumes by comparing prompts, re-running
"the first agent whose prompt differs from the previous run ... and so does every agent after it"
(https://code.claude.com/docs/en/workflows), which requires enforced determinism in prompt assembly
(it makes `Date.now()` and `Math.random()` throw). `htui` does not adopt that: a step's work is
commits in a tree, not a memoizable return value, so "the prompt changed, re-run it" would re-do
merged work. `run_step.prompt_digest` stays what `R-ORCH-11` asks it to be, an audit field that says
which prompt produced which commits, and resume is decided by artefacts instead.

**Completed siblings survive a failed sibling.** When one fan-out candidate fails or is interrupted,
the others keep their `done` status and their trees; only the failed index is re-attempted, and
selection then runs over whatever survived. Anthropic's runtime does the opposite, re-running every
agent that started after the failed one "even ones that completed"; that is right for a cached pure
function and wrong here, where each sibling has already burned quota (`R-AGT-7`) and produced commits.
LangGraph's pending-writes behaviour is the model followed: on restart "you don't re-run the
successful nodes" (https://docs.langchain.com/oss/python/langgraph/checkpointers).

**The offline window.** `R-STO-4` forbids *starting* a run offline; it does not say what happens to
one already live, and the answer is forced by the type system rather than chosen:

1. On `go_offline`, the orchestrator stops admitting and stops scheduling new steps. It cannot write,
   so it writes nothing.
2. The live session keeps running. Its recorder falls back to
   `<cache_dir>/pending/<project_id>.<run_id>.jsonl`, the path MOD-6 already implements, so
   `R-HIS-1` still holds.
3. The `run` row stays `running` with a lease that will expire, because the refresh is a write.
4. On reconnect, `upload_pending` lands the events idempotently and the recovery sweep adjudicates
   the step by the artefact test above. A step that finished during the window is marked `done`; one
   that did not is reset and retried.

Nothing new is built for this: it is the lease plus the pending buffer plus the artefact test,
composed. The only rule that had to be chosen is that a graph run **stalls** rather than being
cancelled or buffered as a run, and that is stated so MOD-4 does not invent a third path.

### 4.10 Capability check, run records, close-out, auto mode and caps

**Capability check (`R-ORCH-10`).** "a run is refused when the item's required tags are not a subset
of the box's tags, listing the missing tags" (`docs/REQUIREMENTS.md:183-184`). Evaluated twice, at
queue time and again inside the admission transaction at claim time, because a box's `probed_tags`
can change between the two (MOD-7 re-probes). The predicate is
`i.required_tags <@ (b.probed_tags || b.declared_tags)`, exactly `docs/ANA-9.md` §7.4's clause, and
the message is `docs/ANA-9.md:960`'s query verbatim:
`SELECT unnest(i.required_tags) EXCEPT SELECT unnest(b.probed_tags || b.declared_tags)`.

The refusal persists (invariant 7): `item.status = 'blocked'` plus an `item_note` whose body names
the box and the missing tags. A queue-time refusal writes **no `run` row**, so there is no orphan run
for a human to cancel; a claim-time refusal fails the run row that already exists with
`run.failure = "missing tags: a, b"`. Never fail silently and never substitute: Claude Code's own
capability handling substitutes a model but warns "naming both the requested and substituted models"
(https://code.claude.com/docs/en/workflows), and `R-ORCH-10` asks for the same naming.

**Ready-item selection for auto mode (`R-ORCH-6`).** The queue query is `docs/ANA-9.md` §7.4
unchanged, which the shipped `ItemFilter.ready` implements only partially: it has the scope,
`status = 'open'` and the `blocked_by` clause, and lacks the box join and the queue ordering
(`crates/htui-store/src/pg/read.rs:96-102` orders by display order, not by
`priority DESC, created_at`). MOD-12 therefore calls a new `ready_items(scope, box_id)` (§8) rather
than reusing `items()`. Two orderings sit above `R-ORCH-6`'s "dependency order and priority" and
neither is adopted for v1: in-flight-first (the maintainer's R1 rule, "finishing beats opening a
front", `.claude/skills/handoff-run/references/selection.md`) and dependent-count-first (R2). Both are
computable (`EXISTS(run WHERE item_id = i.id)` and a `COUNT` over inbound `blocked_by` edges) and
both are requirement changes, so they are recorded as prior art here and left out of the query.
`R-ORCH-6`'s literal text is what ships.

**Run records (`R-ORCH-11`).** All eleven fields already have columns; the audit checks out
field for field. Three amendments make the record complete rather than merely present:

| Field | State | Amendment |
|---|---|---|
| graph snapshot | `graph_snapshot JSONB` nullable, so a graph run with a missing snapshot is indistinguishable from a chat run that legitimately has none | `CHECK (kind <> 'graph' OR graph_snapshot IS NOT NULL)` in `0003`, plus the JSON shape of §5.1 |
| commit hashes | `run_step_commit` per `(step, repo)` | joined by `run_step_tree` so the mode and path are recorded beside the hashes |
| gate outcome | present | `verify_outcome` / `verify_exit_code` added beside it so a `skipped` gate is explicable |

**Close-out (`R-TUI-9`).** "Close-out writes the final summary document, sets status, records commit
hashes from the run. No markdown files are produced." One transaction, refusing while any run of the
item is non-terminal:

1. `document(kind = 'summary')` at the next version, `produced_by_step_id = NULL` when a human wrote
   it, or the last step's id when a `close-out` phase produced it. Its body embeds the commit table
   read from `run_step_commit` joined to `run_step_tree` and `repo`, one row per `(repo, step)` with
   `before_hash..after_hash`.
2. `item.status -> 'closed'`, which sets `closed_at` through the shipped `transition` behaviour.
3. No new commit table. "Records commit hashes from the run" is satisfied by the summary body plus
   the rows that already exist; a `run_commit` or `item_commit` table would duplicate
   `run_step_commit` with no new fact.

Two in-house close-out artefacts have no database home and are recorded as gaps rather than invented
here: the "live coordinates" recap (`.claude/rules/workflow-docs.md:123-125`) and the
"watch items for later modules" note addressed to a *named future item*. The second has a near-fit,
an agent-proposed `item_link` of kind `relates` through MOD-11 (`R-ENT-9` forbids a manual link
action), and the first has none. Both are named in §10 as risks rather than solved.

**Auto-mode gate downgrade (`R-ORCH-6`, `R-ORCH-2`).** The downgrade is applied **at snapshot
time**, so `run.graph_snapshot` carries both the configured `gate` and the effective
`gate_effective` per phase (§5.1). The reasons are that a mode change mid-run then cannot
retroactively skip a gate the run already passed, that a resumed run does not have to re-derive the
downgrade, and that the audit record shows both what was configured and what was applied. The rule
is one line: `gate_effective = if run.mode == Auto && !gate_hard { Never } else { gate }`. The trace
is `gate_outcome = 'skipped'` per §4.2.

Timer-based auto-approval is rejected. Jules auto-approves a plan on a timer ("if you navigate away,
Jules will eventually auto-approve the plan, which is set on a timer",
https://jules.google/docs/review-plan/), which makes `gate_outcome` record a decision nobody made.
`R-ORCH-2`'s downgrade is a configured, recorded choice; a timeout is not.

Substituting a judge for a downgraded gate is *not* adopted either, though it is the most
interesting piece of prior art found: Jules added a Planning Critic specifically "to review all plans
that do not require human intervention" (https://jules.google/docs/changelog/2026-01-26-1/). `htui`'s
equivalent already exists as `verify_command` plus `gate = on_failure`, which is deterministic and
free; a second model in the auto path would be an agent in a bookkeeping path.

**Caps (`R-AGT-7`, `R-AGT-8`).** ANA-4 §7 settles the enforcement point and MOD-4 does not move it:
"**The recorder is the enforcement point** because it is the only place that sees every `usage` row"
(`docs/ANA-4.md:1143-1150`), cancelling the session and marking the step failed on breach. MOD-4 adds
the admission half, which the recorder cannot do: a new attempt or a new fan-out candidate is
admitted only when the remaining budget could plausibly finish it, following SWE-agent's
`min_budget_for_new_attempt`, whose sibling `cost_limit` carries the reason in its own docstring
("The last review is not included in the cost limit, because we would waste the last attempt if we
couldn't score it"). The batch total has no column and does not get one: it is
`SUM(run_step.usage->>'cost_micros')` over the batch's runs, computed at scheduling time by MOD-12,
which is cheaper than a migration and cannot drift from the rows it sums.

---

## 5. JSONB shapes

### 5.1 `run.graph_snapshot` (`JSONB`, `NOT NULL` for `kind = 'graph'` after `0003`)

ANA-9 reserves the column with a prose comment ("step_graph + phases + phase_agent at start
(R-ORCH-11)") and no schema; both fixture runs set it to `None`, so there is not even an example.
It is the only route to a graph while offline, since `step_graph` and `step_graph_phase` are not
mirrored, which makes its shape load-bearing.

```json
{
  "v": 1,
  "graph": { "id": "…", "name": "feature", "is_override": false },
  "topology": "sha256:…",
  "mode": "manual",
  "phases": [
    {
      "position": 0,
      "name": "plan",
      "fan_out": 1,
      "gate": "always",
      "gate_effective": "always",
      "gate_hard": true,
      "retry_limit": 1,
      "input_kinds": ["prd"],
      "output_kind": "plan",
      "isolation": "worktree",
      "command_queue": "fan_out_only",
      "verify_command": null,
      "deadline_seconds": 7200,
      "template": { "name": "plan", "version": 3 },
      "token_budget": null,
      "candidates": [ { "agent_id": "…", "agent_name": "claude", "model": "opus" } ],
      "judge": null
    }
  ],
  "settings": {
    "default_isolation": "worktree",
    "per_token_cap_run": null,
    "per_token_cap_batch": null,
    "max_fan_out": 4,
    "max_agents_per_run": 8
  }
}
```

Four rules. **`v` is a schema version**, incremented only when a reader must branch; a snapshot with
an unknown `v` refuses to resume rather than guessing. **`topology` is `sha256` over the canonical
serialisation of `phases[]` with `gate_effective`, `template.version` and `candidates` included**;
the recovery sweep compares it against a freshly resolved snapshot of the live graph and, on a
mismatch, refuses to resume and parks the run, which is the guarantee Microsoft Agent Framework gets
from its `graph_signature_hash` (https://learn.microsoft.com/en-us/agent-framework/workflows/checkpoints).
**Both `gate` and `gate_effective` are present**, per §4.10, so the audit shows configured and
applied. **`agent_name` is denormalised** because `agent` is not mirrored and an offline Runs tab
would otherwise render a UUID.

### 5.2 `box.settings` and `project.settings`

Typed as §4.7's `BoxSettings` and `ProjectSettings`, `#[serde(default)]` on every field, defaults
from `app_setting`. No column changes; the JSONB comments in `0001_init.sql:68` and `:150-151` become
true for the first time. `0003` adds the two `COMMENT ON COLUMN` statements naming ANA-2 §4.7 as the
contract, so the next reader finds the shape from the database.

### 5.3 The judge verdict block

Emitted inside the judge's `judge` document, parsed by the orchestrator, never read as prose:

```json
{ "winner": 2,
  "reasons": { "0": "no tests for the new branch", "1": "duplicates the existing helper",
               "2": "smallest diff, tests pass, matches the plan" } }
```

Keys are `fanout_index` values rendered as strings. `winner` must be an index that survived the
verification prefilter; anything else is a judge failure per §4.5.

### 5.4 `app_setting` keys this document reserves

| Key | Type | Default | Owner |
|---|---|---|---|
| `max_concurrent_items` | integer | 2 | §4.7 |
| `command_limits` | object | `{"build":1,"test":4,"verify":1}` | §4.2, `R-MCP-3` |
| `default_isolation` | string | `"worktree"` | §4.1 |
| `step_deadline_seconds` | integer | 7200 | §4.1 |
| `max_fan_out` | integer | 4 | §4.5 |
| `max_agents_per_run` | integer | 8 (6 until MOD-4's `0004`, amended by MOD-4 milestone 6, 2026-09-25) | §4.5 |
| `copy_max_total_bytes` | integer | 20 GiB | §4.6 |
| `lease_ttl_seconds` | integer | 120 | §4.9 |
| `lease_refresh_seconds` | integer | 60 | §4.9 |
| `per_token_cap_run` | integer, nullable | null | `R-AGT-7`, ANA-4 §7 |
| `per_token_cap_batch` | integer, nullable | null | `R-AGT-7`, MOD-12 |
| `scheduler_window` | object, nullable | null | `R-ORCH-13`, reserved unused in v1 |

`scheduler_window` is reserved here so the key name exists before `R-ORCH-13` needs it; ANA-9 left
the scheduler window as "a key, not a column" without naming it.

---

## 6. Mapping tables

### 6.1 Requirement to mechanism

| Requirement | Mechanism | Where |
|---|---|---|
| `R-ORCH-1` | `step_graph_phase` unchanged; `ResolvedPhase` off the snapshot; per-field fallback chains | §4.1 |
| `R-ORCH-2` | six-stage step lifecycle; settle outcome; gate table; four `GateOutcome` values with "edit" as a new document version | §4.2 |
| `R-ORCH-3` | same `position`, `attempt + 1`, implement phase's `retry_limit`, three-item forwarded set, no-progress predicate, escalation to `awaiting_approval` + `blocked` | §4.4 |
| `R-ORCH-4` | manual mode is `run.mode = 'manual'`; `gate_effective = gate` | §4.10 |
| `R-ORCH-5` | promotion in place, `promoted_at`, `accept artifact`, resume at `position + 1` | §4.8 |
| `R-ORCH-6` | ready query, downgrade at snapshot time, escalation as `awaiting_approval`, admission caps | §4.10, §7 |
| `R-ORCH-7` | `judge_agent_id` / `judge_model`; judge step at `fanout_index = -1`; verification prefilter; two-order comparative call; `selected` + `superseded` | §4.5 |
| `R-ORCH-8` | `run_step_tree` per repo; four mode triples; scratch root outside every managed repo | §4.6 |
| `R-ORCH-9` | `run.repo_scope`; qualified `touched_paths`; prefix intersection; `FOR UPDATE` admission counting `running` | §4.7 |
| `R-ORCH-10` | `<@` predicate at queue and claim; `EXCEPT` message; `blocked` + `item_note` | §4.10 |
| `R-ORCH-11` | every field already columnar; `graph_snapshot` CHECK; `verify_*`; `run_step_tree` | §4.10, §5.1 |
| `R-ENT-6` | seeded graphs unchanged except `review` in `implement.input_kinds` and the `gate_hard` seed | §4.1 |
| `R-ENT-8` | stored `blocked` for human-clearable refusals, derived blocked for dependencies; three CAS tables | §4.3 |
| `R-TUI-4` | §6.2 | §6.2 |
| `R-TUI-9` | one transaction, summary document embedding the commit table, no new commit table | §4.10 |

### 6.2 `R-TUI-4` actions to orchestrator commands

`R-TUI-4` (`docs/REQUIREMENTS.md:254-256`) lists seven actions. Each is one `OrchestratorCommand`
sent through the existing `Ctx::request` seam (a `StoreRequest::Orch` variant plus one `name()` arm
plus one `try_serve` arm, per `crates/htui/src/store_worker.rs`), and each resolves as a
compare-and-set whose zero-row result renders the actual state rather than a generic error.

| `R-TUI-4` action | Command | Effect | Enabled when |
|---|---|---|---|
| approve | `AnswerGate { step, Approved, note: None }` | step `done`, run `running`, walk resumes at `position + 1` | step `awaiting_approval` and a document of `output_kind` exists |
| reject with note | `AnswerGate { step, Rejected, note }` | step `failed`; §4.4 loop if the phase is loopable and budget remains, else run `failed` | step `awaiting_approval` |
| retry | `RetryStep { step }` | step `superseded` with `gate_outcome = 'retried'`, new step at `attempt + 1` | step `awaiting_approval` or `failed`, and `attempt <= retry_limit + 1` |
| promote to chat | `PromoteStep { step }` | `promoted_at` set, run and item to `awaiting_approval`, chat tab opens bound to the step | step `running`, `awaiting_approval` or `failed`, and the run non-terminal |
| cancel | `CancelRun { run }` (`CancelStep` not built, MOD-4 plan D178; amended by MOD-4 milestone 6, 2026-09-25) | driver `cancel(grace)`, non-terminal steps to `cancelled`, run `cancelled`, item back to `open`, trees cleaned | run non-terminal |
| open artifact | a `StoreRequest::Document` read, not a command (MOD-4 plan D173) | opens the step's `output_kind` document read-only in a view inside the Runs pane, not the Documents sub-tab, which a sibling sub-tab cannot switch to (MOD-4 plan OQ-9; amended by MOD-4 milestone 6, 2026-09-25) | a document produced by the step exists |
| select fan-out result | `SelectFanout { run, position, attempt, winner }` | §4.5's bookkeeping transaction, then the walk resumes | more than one candidate at `(position, attempt)` and none is `selected` yet |

Three actions beyond the seven are needed by verdicts above and are added in the same registry:
`AcceptArtifact { step }` (§4.8), `Unblock { item }` (§4.3) and `CloseOut { item, summary }`
(`R-TUI-9`, which is also `R-TUI-2`'s `close`). `R-TUI-2`'s `run` and `queue` are `StartRun { item,
mode }` with `mode` `manual` or `auto`.

*Amended by MOD-4 milestone 6, 2026-09-25:* the Runs pane binds twelve keys: the seven above, the
three verbs just named, `run`, and a manual retry of a terminal run's failed cleanup. Each key is
greyed by the same admission function the engine refuses with, evaluated by the run worker over real
rows (`StoreRequest::RunActions`). In production, `approve` and `accept artifact` stay greyed until
MOD-11 lets an agent write its phase's `output_kind` document (MOD-4 blueprint F-R, R-50).

**Projection changes `R-TUI-4` forces.** `RunStepSummary` today carries `id`, `position`, `attempt`,
`fanout_index`, `phase_name`, `agent_id`, `model`, `status`, `gate_outcome` and the two timestamps.
It cannot render usage, cannot mark a fan-out winner and cannot show an exit code, so
`R-TUI-4`'s "step list with agent, model, gate state, usage, duration" and "select fan-out result"
are both unrenderable. MOD-4 adds `usage`, `selected`, `exit_code`, `verify_outcome` and
`promoted_at` to `RunStepSummary`, and `agent_name` (denormalised, since `agent` is not mirrored).
ANA-4 already flagged the usage half: "`RunStepSummary` has no `usage` field, so the Runs tab column
MOD-4 wants needs a projection change as well as a render change" (`docs/ANA-4.md:1107-1108`).

### 6.3 Status vocabulary, complete

| Enum | Values | Written by |
|---|---|---|
| `item.status` | `open`, `queued`, `in_progress`, `awaiting_approval`, `blocked`, `done`, `failed`, `closed` | orchestrator and close-out only (`R-ENT-8`); §4.3's table is the complete list |
| `run.status` | `queued`, `running`, `awaiting_approval`, `done`, `failed`, `cancelled` | derived from steps except `queued` (insert) and `cancelled` (human) |
| `run_step.status` | `pending`, `running`, `awaiting_approval`, `done`, `failed`, `cancelled`, `superseded` | orchestrator, human gate answers, recovery sweep |
| `run_step.gate_outcome` | `approved`, `rejected`, `retried`, `skipped` | the gate answer, or `skipped` for any non-stopping gate |
| `run_step.verify_outcome` (new) | `pass`, `fail`, `unavailable` | stage 5 |
| `run_step_tree.mode` (new) | `worktree`, `copy`, `shared_serialized`, `local` | stage 2 |

No existing CHECK list is widened and no value is renamed. The two new CHECK lists are additions on
new columns and a new table.

---

## 7. Agent selection and caps (`R-AGT-7`, `R-AGT-8`, consumed not settled)

ANA-4 §7 settles the predicate and this document adopts it verbatim as MOD-4's rule
(`docs/ANA-4.md:1137-1141`):

> "The orchestrator walks the phase's candidate agents in priority order and skips one when:
> `quota.exhausted` is true; any window has `utilization >= 1.0`; `quota.status` is not `allowed`;
> or the per-token cap is already reached for this run or batch. A **null** quota means *unknown*,
> and unknown is treated as available - otherwise `agy`, which reports nothing, would never be
> selected."

MOD-4 adds three skip conditions of its own, all from verdicts above:

| Skip condition | Reason |
|---|---|
| `DriverCaps.permission_requests` and `edit_proposals` both false, and `gate_effective != 'never'` | ANA-4 `:554` and its risk 9; a CLI-only agent cannot answer an inline approval |
| `agent_box.enabled = false` or `probe.status != 'ready'` on this box | ANA-4 §4.6; an unauthenticated or missing agent cannot start |
| the remaining run or batch budget is below `min_budget_for_new_attempt` | §4.10; starting an attempt that cannot finish wastes it |

**The empty-candidate fallback, which every seeded project needs today.** `phase_agent` has no
fixture row, no demo INSERT and no seed, and MOD-6 seeded no `agent` rows at all, so every seeded
graph carries zero candidates and `R-AGT-8`'s priority list is empty. The chain of §4.1 applies:
`phase_agent` in `position` order, then `project.settings.default_agent_id` with that agent's own
`default_model`, then the single enabled agent on this box when there is exactly one, then refuse
with `no_candidate_agent` and `item.status = 'blocked'`. `phase_agent.model TEXT NOT NULL` is not
relaxed: an explicit candidate always names a model, and the fallback path writes
`run_step.model` from the agent's default without needing a `phase_agent` row at all.

**Model beats the agent definition.** The maintainer's own rule is that a per-call model overrides
the agent file's declared model, and that the tier depends on the *shape* of the call rather than
its name (the maintainer's standing rule for delegated work). The htui equivalent is exactly
`phase_agent.model` beating `agent.default_model`, which the chain above already gives, and
`step_graph_phase.judge_model` beating both for the judge. No new mechanism.

**Batch accounting.** A batch spans runs and there is no aggregate column; MOD-12 computes
`SUM(run_step.usage->>'cost_micros')` over the runs of the batch at scheduling time. The recorder
keeps per-step enforcement (ANA-4 §7); MOD-12 keeps admission. Neither figure is a billing statement:
"the cap is a guard rail and never a billing statement" (`docs/ANA-4.md:1150`).

---

## 8. Crate and module layout for MOD-4 and MOD-12

**A fifth workspace crate, `htui-orch`.** Policy lives there; the supervising task lives in the
binary. This mirrors ANA-4 §8's split rather than half of it: ANA-4 put the driver in `htui-agent`
and the *runtime task* in `crates/htui/src/agent_worker.rs` (`docs/ANA-4.md:1183`), and the same
split applies here.

Two arguments, and the second is the stronger one. **Dependency weight**, ANA-4's own test
(`docs/ANA-4.md:1156-1161`): MOD-4 drags a git library (absent from `Cargo.lock` today), recursive
filesystem copy, process spawn for `verify_command` (`tokio` is built without `process`), a path
prefix matcher, and a dependency on `htui-agent` itself. None of that may sit behind `htui-core`,
which is deliberately dependency-light and is the crate every other one depends on; and it does not
belong in `htui-store`, which is "the crate that may name a database driver"
(`crates/htui-core/src/lib.rs:6-8`), a different concern. **Headlessness**, which ANA-4 did not face:
`R-ORCH-12` foresees "a headless `htui` worker per box polling Postgres", and a separate crate is the
only layout in which that worker can link the orchestrator without linking `ratatui` and `crossterm`.
The name is three characters shorter than `htui-orchestrator` and matches the `htui-core` /
`htui-store` / `htui-agent` convention.

```
crates/htui-orch/
  Cargo.toml
  src/lib.rs          #![warn(missing_docs)]; re-exports engine, graph, command
  src/graph.rs        ResolvedPhase, snapshot build, topology hash, override clone, field chains
  src/status.rs       the three transition tables as const predicates + the CAS wrappers
  src/engine.rs       the walk: admit -> prepare -> prompt -> session -> settle -> gate
  src/gate.rs         settle outcome, gate resolution, GateOutcome writes, R-ORCH-3 loop
  src/fanout.rs       candidate planning, verification prefilter, judge call, selection transaction
  src/isolate.rs      the four modes, scratch root, git ops with backoff, run_step_tree,
                      before/after hash capture, winner reconciliation, cleanup
  src/overlap.rs      RunScope, the predicate, PathPrefix normalisation, admission transaction
  src/verify.rs       verify_command execution, direct and via command_run, three outcomes
  src/recover.rs      lease refresh, recovery sweep, artefact completion test, tree reset
  src/select.rs       R-AGT-8 candidate walk incl. the three MOD-4 skip conditions and the fallback
  src/queue.rs        MOD-12: ready items, priority order, batch caps, scheduler window key
  src/command.rs      OrchestratorCommand (the R-TUI-4 verbs of §6.2) and its outcomes
  src/fake.rs         FakeOrchestrator over a FakeDriver and MemStore, feature `test-support`
  src/conformance.rs  CASES + run_case + run_all, feature `test-support`
  tests/fixtures/*    recorded graphs and their expected step sequences
crates/htui-core/src/store/traits.rs   + the WriteStore and ReadStore methods below
crates/htui-core/src/model/{item,run,kind}.rs  + can_move_to, BoxSettings, ProjectSettings,
                                                RunStepSummary fields, RunStepTree
crates/htui/src/run_worker.rs          the supervising task: one per box, owns the lease refresh,
                                       spawns one engine task per active run
crates/htui/src/ui/tabs/backlog/detail/runs.rs  the R-TUI-4 action set in on_key + Ctx::emit
crates/htui/src/store_worker.rs        + StoreRequest::Orch / StoreReply::Orch, + name() arm
```

**No fourth `select!` arm in `event_loop.rs`.** `run_worker` holds a clone of the existing
`mpsc::UnboundedSender<ReplyEnvelope>` and emits run progress as replies stamped with the Runs
tab's `Origin`, exactly as ANA-4 does for the chat stream. `App::latest` is keyed by
`(Origin, Discriminant<StoreRequest>)`, so run progress gets its own discriminant pair
(`StoreRequest::RunStream`) that nothing else uses, or a later request of the same discriminant would
orphan the stream.

**The registry rule holds.** `crates/htui/src/ui/tabs/backlog/detail/mod.rs:4-7` already names this
work: "MOD-4's approve/reject and MOD-2's 'promote to chat' are `DetailTab::on_key` bodies inside
their own file plus one `register` line, never a `match` arm in the Backlog tab." MOD-4 adds no
`match` arm to the Backlog tab.

**Store-trait additions MOD-4 needs, by method name.** Every one is absent today. Reads go on
`ReadStore` only when the SQLite mirror can answer them; the rest are inherent methods on `PgStore`
dispatched by `Backend`, which is the precedent `workspaces`, `box_info`, `active_runs` and
`projects` already set (`crates/htui-store/src/pg/read.rs:378-382`). That split matters because
`step_graph`, `step_graph_phase`, `phase_agent`, `prompt_template`, `agent`, `agent_box`, `repo`,
`repo_box_path`, `command_run` and `app_setting` are **not mirrored**, so putting their readers on
`ReadStore` would oblige `CacheStore` to answer questions the mirror cannot.

*Reads, `PgStore` inherent (online only).*

| Method | For |
|---|---|
| `step_graph(id)` | §4.1 graph resolution |
| `phases(graph)` ordered by `position` | §4.1 |
| `phase_agents(phase)` ordered by `position` | `R-AGT-8` |
| `item_kind(id)` | `default_graph_id` when `item.step_graph_id` is NULL |
| `prompt_template(project, name, version)` | `R-PRM-4`; NULL version means latest |
| `resolve_graph(item)` returning the whole `ResolvedPhase` list | §5.1's snapshot, one round trip |
| `agents()` and `agent_boxes(box)` | `R-AGT-7..8` quota and `DriverCaps` |
| `box_row(id)` | tags and `BoxSettings`; `BoxInfo` cannot carry them |
| `repos(project)` and `repo_paths(box)` | §4.6; there is no reader for either today |
| `app_settings()` | §5.4 |
| `ready_items(scope, box)` | `docs/ANA-9.md` §7.4 in full, with the box join and the queue ordering |
| `missing_tags(item, box)` | `R-ORCH-10`'s refusal message |
| `active_runs_on_box(box)` | §4.7; the shipped `active_runs` is a scope-wide count |
| `overlapping_runs(scope)` | §4.7's predicate, evaluated in SQL over `repo_scope` |

*Reads, `ReadStore` (mirrorable).*

| Method | For |
|---|---|
| `document(id)` with body | §4.2; `documents()` returns heads only |
| `documents_of_kinds(item, kinds)` | §4.2's resolver |
| `run(id)` and `run_steps(run)` | the engine's own state re-derivation |
| `step_trees(step)` and `step_commits(step)` | §4.6, §4.9 |

*Writes, all `WriteStore`.*

| Method | For |
|---|---|
| `create_run(NewRun)` | inserts the run and its `graph_snapshot` and moves `item.status` in one transaction |
| `claim_run(run, box, owner, ttl)` | §4.7's admission transaction, returning `false` when the slot or the overlap check refuses |
| `refresh_lease(run, owner, ttl)` | §4.9 |
| `adopt_runs(box, owner)` | §4.9's sweep |
| `create_step(NewRunStep)` | respects `UNIQUE (run_id, position, attempt, fanout_index)` |
| `transition_run(run, from, to)` and `transition_step(step, from, to)` | §4.3's CAS |
| `finish_step(step, StepOutcome)` | `exit_code`, `usage`, `trim_record`, `verify_outcome`, `verify_exit_code`, `finished_at` |
| `answer_gate(step, outcome, note)` | the four `R-ORCH-2` answers |
| `select_fanout(run, position, attempt, winner)` | §4.5's one transaction: winner, losers, judge note |
| `supersede_step(step)` | §4.4's loop half |
| `upsert_step_tree(step, &[RunStepTree])` | §4.6 stage 2 |
| `record_commits(step, &[RunStepCommit])` | `R-ORCH-11`, both hashes |
| `write_document(NewDocument)` | version allocated inside the transaction; MOD-11's tool calls this |
| `promote_step(step)` | §4.8's `promoted_at` |
| `fail_run(run, failure)` | `run.failure` |
| `close_out(item, summary, commits)` | `R-TUI-9`'s three effects, atomically |
| `add_note(item, body, via_step)` | the refusal notes of invariant 7 |

**Cost this prices, deliberately.** Every `WriteStore` addition obliges `MemStore` to implement it,
because `MemStore: WriteStore` is the third conformance target, and obliges new cases in
`crates/htui-core/src/store/conformance.rs`, whose module doc is explicit that it is "written against
`WriteStore` alone: no concrete store is named anywhere in this module". That is sixteen methods and
roughly as many cases. It is the price of the seam and it is better paid at MOD-4's first commit than
discovered at its last.

**The fake orchestrator.** `htui-orch::fake::FakeOrchestrator` composes `MemStore` with ANA-4's
`FakeDriver` and a `FakeIsolator` that creates directories and returns synthetic hashes instead of
touching git. It is what the Runs-tab `insta` snapshots run against, driven by the existing
`Harness::settle()` inline model so snapshots stay byte-stable with no sleeps
(`crates/htui/src/testkit.rs:3-7`). Because the isolation seam is a trait
(`trait Isolator { fn prepare(..); fn capture(..); fn reconcile(..); fn cleanup(..); }`), the whole
engine including the review loop, fan-out, selection, the overlap predicate and the recovery sweep is
testable with no Postgres, no git and no agent.

**New workspace dependencies.**

| Crate | Why | Note |
|---|---|---|
| `gix` or `git2` | `R-ORCH-8` worktrees, `R-ORCH-11` hashes | **Open for the maintainer 7**; default `gix` for the pure-Rust build and no `libgit2`/OpenSSL linkage on Windows. Shelling out to `git` is the third option and is rejected: it needs the same process machinery plus output parsing, and the failure modes (`index.lock`) are easier to classify through an API. **Superseded 2026-09-22 (MOD-4 milestone 3, OQ-1):** `gix` 0.87.1 has no worktree mutation at all — `worktrees()`, `worktree_proxy_by_id`, `main_repo`, `worktree` and `is_bare` are the whole surface, and `worktree::Proxy` is read-only — so `isolate/git.rs` shells out to `git` (>= 2.33.0) for exactly `worktree add --lock`, `worktree remove`, `merge --no-ff` and `reset --hard`, while `gix` keeps every read and every ref write. `index.lock` is classified from stderr text under `LC_ALL=C`. Consequences: `git` becomes a runtime dependency of the `worktree` mode and of reconciliation, and a test dependency; MOD-16 must verify `git.exe` on Windows; `git worktree prune` is never run, because it refuses locked entries and cannot be scoped, so its only reachable effect is on the maintainer's own worktrees. |
| `tokio` `process` and `io-util` | `verify_command` | ANA-4 §8 already adds both for MOD-2; MOD-4 inherits them |
| `process-wrap` | job object and `CREATE_NO_WINDOW` for `verify_command` children | already added by ANA-4 §8 |
| `fs_extra` or a hand-rolled walker | `copy` isolation with an exclusion list | hand-rolled is ~80 lines and avoids a dependency; either is acceptable |
| `sha2` | the snapshot topology hash and the no-progress predicate | already a workspace dependency |

No glob crate: §4.7's prefix rule is deliberately matcher-free.

---

## 9. Phasing and downstream impact

**MOD-4 build order (manual mode).**

1. `htui-core` first: `can_move_to` on the three status enums, `BoxSettings` / `ProjectSettings`,
   `RunStepTree`, the `RunStepSummary` and `ItemSummary` and `BoxInfo` field additions, and the
   sixteen `WriteStore` methods plus the `ReadStore` reads of §8 with their conformance cases.
   `MemStore` implements them all. Nothing spawns anything yet and the whole seam is testable.
2. Migration `0003_orchestration.sql` and its mirror companion, plus `PgStore` implementations of
   the same methods and the `PgStore` inherent reads. `cargo sqlx prepare -- --all-targets
   --all-features` from `crates/htui-store`, since `SQLX_OFFLINE=true` is set in
   `.cargo/config.toml`.
3. `htui-orch` skeleton: `graph.rs` (snapshot, topology hash, field chains, override clone),
   `status.rs`, `command.rs`, `fake.rs`, `conformance.rs`. The `Isolator` trait is defined here and
   only `FakeIsolator` implements it.
4. `engine.rs` and `gate.rs`: the six-stage walk, the settle outcome, the gate table, `R-ORCH-3`'s
   loop with its no-progress predicate. Against `FakeDriver` and `FakeIsolator`, so the whole of
   §4.2, §4.3 and §4.4 lands with no git, no Postgres and no agent.
5. `isolate.rs` and `verify.rs`: the real `Isolator` over `gix`, the four modes, `run_step_tree`,
   commit capture, winner reconciliation, cleanup, and `verify_command` with its three outcomes.
   This is where the git dependency enters the workspace.
6. `fanout.rs` and `select.rs`: candidate planning, the verification prefilter, the two-order judge
   call, the selection transaction, and `R-AGT-8`'s walk with the three MOD-4 skip conditions and
   the empty-candidate fallback.
7. `overlap.rs` and `recover.rs`: the predicate, the admission transaction, the lease and the
   recovery sweep. Manual mode needs both even without a queue: two manual runs on one box must not
   collide, and a crashed manual run must be adoptable.
8. `crates/htui/src/run_worker.rs`, the `StoreRequest::Orch` / `RunStream` pair, the `R-TUI-4`
   action set in `runs.rs::on_key`, the step list render, and close-out (`R-TUI-9`) as the `close`
   action of `R-TUI-2`. Snapshot tests regenerate `backlog__detail_runs.snap` and
   `backlog__empty_runs.snap`.

Steps 1 to 4 are the half that needs nothing from MOD-2; steps 5 to 8 need the driver.

**MOD-12 build order (auto mode).** Blocked on MOD-4 and adding no new machinery, only a caller:

1. `queue.rs`: `ready_items(scope, box)` against `docs/ANA-9.md` §7.4 in full, with the box join and
   `ORDER BY priority DESC, created_at`, plus the `missing_tags` refusal path.
2. The auto admission loop: for each ready item in order, resolve the graph, apply the `R-ORCH-6`
   downgrade at snapshot time, run the same `claim_run` admission MOD-4 uses, stop at
   `max_concurrent_items`.
3. Batch caps: the `SUM(run_step.usage->>'cost_micros')` accounting of §7 and the
   `min_budget_for_new_attempt` admission rule.
4. Escalation surfacing: a run that parked at `awaiting_approval` while unattended, and every item
   that reached `blocked`, appear in the queue overlay (`R-TUI-1`) with the reason.
5. The Settings sections `R-TUI-8` names for caps and the scheduler window; the window is stored
   under the `scheduler_window` key of §5.4 and is **not** enforced in v1 (`R-ORCH-13` is `later`).
6. The `queue` action of `R-TUI-2`, and `run.target_box_id` written from the current box with
   execution refused when it is not local (`R-ORCH-12`'s v1 half).

**What MOD-4 needs from other items.**

| From | What | State |
|---|---|---|
| MOD-2 (ANA-4) | `AgentDriver` / `AgentSession`, `DriverCaps` (the §4.2 gate interlock and the §4.8 promotion paths both branch on it), `SessionSpec.cwd` and `extra_dirs` (the tree of §4.6), `session_ref` and the `session_started` banner (§4.8), the recorder's `run_step.usage` writes and cap enforcement (§7), and migration `0002_agent_probe.sql` | blocked on ANA-5 |
| ANA-5 | the assembled prompt and its `sections[]`, the trim record, a stable serialisation so `prompt_digest` is reproducible, plus three templates this document names: `judge` (§4.5), the review-loop forwarded set as prompt sections (§4.4), and the promotion handoff prompt (§4.8) | open |
| MOD-11 | `document_write` calling §8's `write_document`, `command_run` with per-box class limits for `verify_command`, and `item_status` request adjudication | blocked on MOD-2 and MOD-4 |
| MOD-7 | `box.probed_tags` and `declared_tags` written by a real probe, `repo_box_path` rows, and `agent_box` with `probe.status` so §7's second skip condition has data | not blocked |
| MOD-15 | project creation seeding kinds, graphs and templates; it is **not blocked**, so it may write graph rows before ANA-2's semantics land. The seed amendments of §4.1 (`review` in `implement.input_kinds`, the `gate_hard` seed) belong to whichever of MOD-15 and MOD-4 lands second, and the one that lands first must not contradict them | not blocked |
| MOD-13 | the `touched_paths` edit path, which must accept the `repo:glob` qualification of §4.7 and validate the repo name against the project's repos | not blocked |

**Migration numbering and the ordering hazard.** `docs/ANA-9.md:349-350` fixes the rule: "Later ANAs
add `000N_*.sql` files; they never edit `0001`." ANA-4 claimed `0002_agent_probe.sql`
(`docs/ANA-4.md:1267-1273`), so `docs/ANA-9.md:1010`'s "ANA-2 may add phase columns by migration
`0002`" is **superseded and must be read as `0003`**. The hazard is real: `0002` does not exist on
disk, MOD-2 authors it, and `PgStore::connect` refuses on an applied version the binary does not know
("schema is newer than this htui", `docs/decisions/mod/mod-6.md` §2). **MOD-4 therefore does not
apply `0003` before MOD-2 has landed `0002`**; if MOD-4 is ready first, the file is written and held,
not applied. That is a sequencing constraint on the two items, stated here so it is not discovered by
a refused connect. `sqlx::migrate!` applies pending files in order and MOD-6's confirmation prompt
(`R-STO-5`) surfaces them as `MigrationState::Pending(n)`, so the user sees both.

**`0003_orchestration.sql`, in full.**

```sql
-- 0003_orchestration.sql - ANA-2 (orchestration) amendments to the ANA-9 schema.
-- Forward-only (R-STO-5): 0001_init.sql and 0002_agent_probe.sql are never edited.
-- Depends on 0002_agent_probe.sql (ANA-4 §9), authored by MOD-2.

-- --------------------------------------------------------------------------------------------
-- 1. step_graph: item override graphs are hidden from the project graph list (ANA-2 §4.1)
-- --------------------------------------------------------------------------------------------
ALTER TABLE step_graph ADD COLUMN is_override BOOLEAN NOT NULL DEFAULT false;
CREATE INDEX idx_step_graph_listed ON step_graph(project_id) WHERE NOT is_override;
COMMENT ON COLUMN step_graph.is_override IS
  'true = an <item.key>-override clone (R-ORCH-1, ANA-2 §4.1); hidden from the R-TUI-8 graph list';

-- --------------------------------------------------------------------------------------------
-- 2. step_graph_phase: the R-ORCH-7 judge and the step deadline (ANA-2 §4.5, §4.2)
-- --------------------------------------------------------------------------------------------
ALTER TABLE step_graph_phase ADD COLUMN judge_agent_id   UUID REFERENCES agent(id);
ALTER TABLE step_graph_phase ADD COLUMN judge_model      TEXT;
ALTER TABLE step_graph_phase ADD COLUMN deadline_seconds INTEGER
  CHECK (deadline_seconds IS NULL OR deadline_seconds > 0);
ALTER TABLE step_graph_phase ADD CONSTRAINT ck_phase_judge_model
  CHECK (judge_model IS NULL OR judge_agent_id IS NOT NULL);

COMMENT ON COLUMN step_graph_phase.judge_agent_id IS
  'R-ORCH-7 judge; NULL = human selection is required whenever fan_out > 1 (ANA-2 §4.5)';
COMMENT ON COLUMN step_graph_phase.judge_model IS
  'model for the judge; overrides agent.default_model (ANA-2 §7)';
COMMENT ON COLUMN step_graph_phase.deadline_seconds IS
  'wall clock for one step attempt; NULL = project.settings.step_deadline_seconds (ANA-2 §4.1)';
COMMENT ON COLUMN step_graph_phase.verify_command IS
  'ANA-2 §4.2: runs after the session and before the gate, in the primary repo tree, through '
  'command_run when the phase advertises it; outcome lands in run_step.verify_outcome';
COMMENT ON COLUMN step_graph_phase.input_kinds IS
  'ANA-2 §4.2: each kind resolves to the latest document version on this item whose producing '
  'step is not a fan-out loser, preferring this run''s own output; a missing kind fails the step';

-- --------------------------------------------------------------------------------------------
-- 3. run: repo scope (R-ORCH-9), the recovery lease, the R-ORCH-11 snapshot guarantee
-- --------------------------------------------------------------------------------------------
ALTER TABLE run ADD COLUMN repo_scope       UUID[] NOT NULL DEFAULT '{}';
ALTER TABLE run ADD COLUMN lease_box_id     UUID REFERENCES box(id);
ALTER TABLE run ADD COLUMN lease_owner      UUID;
ALTER TABLE run ADD COLUMN lease_expires_at TIMESTAMPTZ;

-- NOT VALID: demo and fixture rows predate ANA-2 and carry a NULL snapshot on a graph run.
-- New rows are checked; the backfill is a separate, optional VALIDATE CONSTRAINT.
ALTER TABLE run ADD CONSTRAINT ck_run_graph_snapshot
  CHECK (kind <> 'graph' OR graph_snapshot IS NOT NULL) NOT VALID;

CREATE INDEX idx_run_lease ON run(executing_box_id, lease_expires_at)
  WHERE status IN ('queued','running');
CREATE INDEX idx_run_repo_scope ON run USING GIN (repo_scope);

COMMENT ON COLUMN run.repo_scope IS
  'repos this run may touch, resolved at queue time from item.touched_paths (ANA-2 §4.7)';
COMMENT ON COLUMN run.lease_owner IS
  'per-process id of the orchestrator holding this run; a zero-row lease refresh means abandon '
  '(ANA-2 §4.9)';
COMMENT ON COLUMN run.graph_snapshot IS
  'ANA-2 §5.1: {v, graph, topology, mode, phases[], settings}; carries both gate and '
  'gate_effective so the R-ORCH-6 downgrade is auditable';

-- --------------------------------------------------------------------------------------------
-- 4. run_step: verification outcome (ANA-2 §4.2) and chat promotion (ANA-2 §4.8)
-- --------------------------------------------------------------------------------------------
ALTER TABLE run_step ADD COLUMN verify_outcome   TEXT
  CHECK (verify_outcome IN ('pass','fail','unavailable'));
ALTER TABLE run_step ADD COLUMN verify_exit_code INTEGER;
ALTER TABLE run_step ADD COLUMN promoted_at      TIMESTAMPTZ;

COMMENT ON COLUMN run_step.verify_outcome IS
  'ANA-2 §4.2: pass | fail | unavailable; unavailable never fails a step';
COMMENT ON COLUMN run_step.exit_code IS
  'the agent process exit code (R-ORCH-11); verification has its own verify_exit_code';
COMMENT ON COLUMN run_step.fanout_index IS
  '0..fan_out-1; -1 = the R-ORCH-7 judge step for this position and attempt (ANA-2 §4.5)';
COMMENT ON COLUMN run_step.selected IS
  'fan-out winner; NULL when fan_out = 1; false on a loser, which is also superseded (ANA-2 §4.5)';
COMMENT ON COLUMN run_step.attempt IS
  '1-based; retry_limit is the number of additional attempts, so attempt <= retry_limit + 1 '
  '(ANA-2 §4.2)';
COMMENT ON COLUMN run_step.isolation_path IS
  'the primary repo tree; every repo in scope has a run_step_tree row (ANA-2 §4.6)';
COMMENT ON COLUMN run_step.promoted_at IS
  'set when the step was promoted to an interactive chat (R-ORCH-5, ANA-2 §4.8)';

-- --------------------------------------------------------------------------------------------
-- 5. run_step_tree: one row per (step, repo), because a project has one or more repos (R-ENT-3)
--    No updated_at and no trigger: like run_step_commit it rides its parent step (ANA-9 §6.2).
-- --------------------------------------------------------------------------------------------
CREATE TABLE run_step_tree (
    run_step_id UUID NOT NULL REFERENCES run_step(id) ON DELETE CASCADE,
    repo_id     UUID NOT NULL REFERENCES repo(id),
    mode        TEXT NOT NULL CHECK (mode IN ('worktree','copy','shared_serialized','local')),
    path        TEXT NOT NULL,                 -- absolute, on the executing box, outside every repo
    base_ref    TEXT NOT NULL,                 -- the commit the tree was created at (ANA-2 §4.6)
    dirty       BOOLEAN NOT NULL DEFAULT false,-- the tree had uncommitted work at step start
    PRIMARY KEY (run_step_id, repo_id)
);
COMMENT ON TABLE run_step_tree IS
  'R-ORCH-8 isolation, per repo; ANA-2 §4.6. A dirty local or shared_serialized tree is never '
  'reset by the recovery sweep (ANA-2 §4.9).';

-- --------------------------------------------------------------------------------------------
-- 6. item: touched_paths become repo-qualified (ANA-2 §4.7). Existing bare globs keep meaning
--    the primary repo, so no data migration is required.
-- --------------------------------------------------------------------------------------------
COMMENT ON COLUMN item.touched_paths IS
  'R-ORCH-9 declared overlap set: "<repo_name>:<glob>" entries, or a bare glob meaning the '
  'primary repo. Empty means unknown, which overlaps the whole primary repo (ANA-2 §4.7).';

-- --------------------------------------------------------------------------------------------
-- 7. settings contracts (ANA-2 §4.7, §5.2)
-- --------------------------------------------------------------------------------------------
COMMENT ON COLUMN box.settings IS
  'ANA-2 §4.7 BoxSettings: max_concurrent_items (R-ORCH-9), command_limits {class: n} (R-MCP-3). '
  'Every field optional; defaults come from app_setting.';
COMMENT ON COLUMN project.settings IS
  'ANA-2 §4.7 ProjectSettings: token_budget, retention_days, cached_transcript_steps, '
  'keep_raw_events, default_isolation, per_token_cap_run, per_token_cap_batch, '
  'step_deadline_seconds, default_agent_id, judge_agent_id, copy_exclude. Every field optional.';

-- --------------------------------------------------------------------------------------------
-- 8. app_setting defaults (ANA-2 §5.4). Idempotent, so a re-run and the MOD-6 seed agree.
-- --------------------------------------------------------------------------------------------
INSERT INTO app_setting (key, value) VALUES
  ('max_concurrent_items',  '2'::jsonb),
  ('command_limits',        '{"build":1,"test":4,"verify":1}'::jsonb),
  ('default_isolation',     '"worktree"'::jsonb),
  ('step_deadline_seconds', '7200'::jsonb),
  ('max_fan_out',           '4'::jsonb),
  ('max_agents_per_run',    '8'::jsonb),  -- 8 since MOD-4's 0004 (0003 shipped '6'); amended by MOD-4 milestone 6, 2026-09-25
  ('copy_max_total_bytes',  '21474836480'::jsonb),
  ('lease_ttl_seconds',     '120'::jsonb),
  ('lease_refresh_seconds', '60'::jsonb),
  ('per_token_cap_run',     'null'::jsonb),
  ('per_token_cap_batch',   'null'::jsonb),
  ('scheduler_window',      'null'::jsonb)
ON CONFLICT (key) DO NOTHING;
```

Three notes on the migration. **Nothing is dropped, renamed or retyped**, so no shipped query
breaks and `cargo sqlx prepare` regenerates rather than fails. **`ck_run_graph_snapshot` is
`NOT VALID`** because the demo loader writes graph runs with a NULL snapshot today
(`crates/htui-core/src/fixtures.rs:1113`, `:1130`); MOD-4 fixes the fixture and a later
`VALIDATE CONSTRAINT` is a one-line follow-up, not a blocker. **`run_step_tree` is deliberately
outside the `updated_at` trigger loop**, matching `run_step_commit`, whose exclusion
`0001_init.sql:565-570` already documents; the refresher rides it on its parent step's `updated_at`.

**Cache mirror amendment, `cache_migrations/0002_orchestration.sql`.** `run`, `run_step` and
`run_step_commit` are mirrored, so every column added above to `run` and `run_step` needs a mirror
column, and `run_step_tree` needs a mirror table for the offline Runs tab to render an isolation
mode:

```sql
-- 0002_orchestration.sql - mirror side of ANA-2 (ANA-9 §4.4, §6.2)
ALTER TABLE run      ADD COLUMN repo_scope TEXT NOT NULL DEFAULT '[]';  -- JSON array of uuids
ALTER TABLE run      ADD COLUMN lease_box_id TEXT;
ALTER TABLE run      ADD COLUMN lease_expires_at INTEGER;
ALTER TABLE run_step ADD COLUMN verify_outcome TEXT;
ALTER TABLE run_step ADD COLUMN verify_exit_code INTEGER;
ALTER TABLE run_step ADD COLUMN promoted_at INTEGER;
CREATE TABLE run_step_tree (
    run_step_id TEXT NOT NULL, repo_id TEXT NOT NULL, mode TEXT NOT NULL,
    path TEXT NOT NULL, base_ref TEXT NOT NULL, dirty INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_step_id, repo_id));
```

`run.lease_owner` is not mirrored: it is a liveness token for a process that by definition is not
running when the mirror is being read. `cache_meta.schema_version` bumps in the same change, which
forces a full rebuild (`docs/ANA-9.md:322-326`), so no backfill is needed and the mirror is correct
by construction on the first refresh. The refresher's fixed ten-table list
(`crates/htui-store/src/cache/refresh.rs:200-210`) becomes eleven, with `run_step_tree` riding its
parent step's `updated_at` exactly as `run_step_commit` already does.

**Superseded material this document supersedes.** `docs/ANA-9.md:1010`'s "migration `0002`" for
ANA-2, and the `fanout_index` SQL comment's `0..fan_out-1` as an exhaustive range. Nothing in ANA-4
is superseded; its `DriverCaps`, `run_step.usage` and quota verdicts are consumed as written.

---

## 10. Open for the maintainer

Each has a default this document adopts, so MOD-4 is not blocked on any of them; each is a choice a
verdict could reasonably have gone the other way on.

| # | Question | Default adopted |
|---|---|---|
| 1 | Task fan-out (N sessions, N different prompts, disjoint declared file sets, every result merged, no judge) is the maintainer's own practice and is not what `R-ORCH-7` describes. Should `htui` build both fan-out kinds? | **Rival fan-out only in v1.** `docs/REQUIREMENTS.md` is the contract; adding a second kind is a requirement change, and the isolation and overlap machinery of §4.6 and §4.7 is the half both would share, so nothing is foreclosed. |
| 2 | `max_fan_out` and `max_agents_per_run` | **4 and 6.** Four matches the only shipped human-select fan-out (Codex cloud); six matches the maintainer's own standing cap on subagents per run, adopted after a run spawned 56 agents and exhausted a five-hour quota in about a minute. **Amended (MOD-4 milestone 4, maintainer; recorded here as amended by MOD-4 milestone 6, 2026-09-25):** `max_agents_per_run` is **8**. Six refused the seeded `feature` graph, whose judged 3-way `implement` needs 7 agents; `0004_max_agents_per_run_default.sql` moves an untouched seeded 6 to 8. |
| 3 | `max_concurrent_items` default | **2**, matching the demo fixture. A CPU-derived default (Claude Code caps at 16 concurrent agents, "fewer when Claude Code has fewer CPUs available") is the alternative and would be a better default for a large box. |
| 4 | Is `copy` isolation offered by default on Windows, where it is a full byte-for-byte copy with no reflink on stock NTFS? | **Offered, with the measured size shown and a refusal above `copy_max_total_bytes`.** |
| 5 | Which seeded phases carry `gate_hard`, since a graph containing a hard gate is not fully unattended in auto mode | **`prd` and `plan` on the feature graph, `verdict` on the analysis graph, none elsewhere.** This mirrors the maintainer's own never-disappearing gates (route confirm, PRD open questions, plan CONFIRM, deliberate push) minus the two that have no htui analogue. It makes the `FIX`, `CLEAN` and `TOOL` graphs the first fully unattended targets for MOD-12. |
| 6 | Lease TTL and refresh interval | **120 s and 60 s.** Short enough that a crashed run is adoptable within two minutes; long enough that a GC pause or a slow write does not steal a live run. Graphile Worker's fixed four-hour horizon double-ran a six-hour job, and Oban warns of "inaccurate rescues"; the artefact test of §4.9 is what makes a short TTL safe here. |
| 7 | `gix` or `git2` | **`gix`**, for a pure-Rust build with no `libgit2` or OpenSSL linkage on Windows. `git2` is the mature alternative; shelling out to `git` is rejected in both cases. **Amended 2026-09-22 (MOD-4 milestone 3, maintainer):** `gix`, **plus the `git` CLI** for the four worktree and merge verbs `gix` lacks; `git2` is still rejected. |
| 8 | May close-out run on a `failed` item without a retry? | **Yes.** `failed -> closed` is in §4.3's table and is the release valve for a `failed` blocker (§4.3 verdict 2). A rule that forced a retry first would strand dependents. |

---

## 11. Risks

| # | Risk | Mitigation |
|---|---|---|
| 1 | **Migration ordering.** `0003` applied without `0002` makes `PgStore::connect` refuse with "schema is newer than this htui" on a box running an older binary. | MOD-4 does not apply `0003` before MOD-2 lands `0002`; the file is written and held. Stated in §9 as a sequencing constraint on the two items, and MOD-6's confirmation prompt surfaces pending files rather than applying them silently. |
| 2 | **Git contention across worktrees.** Measured: with thirteen parallel agents, eight of thirteen commits failed on lock contention, and an auto-cleanup then destroyed the uncommitted work. | Three retries with exponential backoff on `index.lock` and ref-lock errors; `--lock` on every worktree; `git gc --prune=now` forbidden while any run is non-terminal; trees are never cleaned up before the run is terminal, and a no-commit worktree is the only step-end removal. |
| 3 | **A `copy` of a configured build tree is silently wrong, not cold.** CMake build trees embed absolute paths and are not relocatable. | `project.settings.copy_exclude` is mandatory and defaults to the seven usual build directories; the mode's documentation says plainly that a warm cache comes from a compiler cache, not from copying artifacts. |
| 4 | **`document_write` does not exist until MOD-11, which is blocked on MOD-4.** A phase whose agent cannot write its artefact produces nothing the next phase can read. | Two non-MCP producers ship with MOD-4: the `accept artifact` gate action and hand-written documents (`R-ENT-12` permits both). An ungated phase with no document fails loudly with `missing_output` rather than continuing. The dependency is circular by design and the gate is the break. |
| 5 | **The judge is a model deciding which code ships.** Position bias alone flips the median model's choice in 41.3% of decisive swapped-order pairs. | Verification prefilter before the judge; two calls with reversed candidate order; disagreement escalates to a human rather than picking; `judge_agent_id IS NULL` (the seeded default) means human selection always. |
| 6 | **A stored `blocked` could still strand an item** if the TUI does not surface the clearing action. | `blocked` is always written with an `item_note` naming the reason, the Backlog shows it, and `Unblock` is a first-class command in §6.2 beside the seven `R-TUI-4` actions. Validation criterion 14 tests exactly this round trip. |
| 7 | **Sixteen new `WriteStore` methods oblige `MemStore` and the conformance suite.** A method added without a case is a method `MemStore` and `PgStore` can silently disagree on. | Every method in §8 lands with its conformance case in the same commit; the suite is the acceptance gate, not an afterthought. `CacheStore` implements `ReadStore` only, which is why the unmirrored reads are `PgStore` inherent methods. |
| 8 | **The overlap prefix rule over-serialises.** `src/**` and `src/**` in one repo always overlap, even when the two items touch disjoint subtrees. | Conservative in the safe direction by choice; the escape is a more specific declaration, which is the behaviour the rule is meant to encourage. A real matcher is a later refinement that changes no stored data. |
| 9 | **Offline stall is invisible.** A run whose box went offline stays `running` in the mirror with an expiring lease nobody can see expire. | The top bar already shows the Postgres state (`R-TUI-1`); the Runs tab shows the lease expiry and labels the run "stalled, offline" once it has passed. MOD-6's own reviewer already found the adjacent "running steps frozen in the mirror" issue, so the render must not trust a mirrored `running`. |
| 10 | **Close-out's "live coordinates" and "watch items for later modules" have no database home.** Both are real outputs of the maintainer's own close-out and neither maps onto a `summary` document, a note or a link. | Named, not solved. The nearest fit for a watch item is an agent-proposed `item_link` of kind `relates` through MOD-11 (`R-ENT-9` forbids a manual link action); live coordinates have no fit at all, and `R-PRM-1`'s upstream-summary walk is the only channel that would surface them. Recorded so a later ANA can take it up. |
| 11 | **`item_status` requests from agents (`R-MCP-2`) have no adjudicator.** `R-ENT-8` says transitions are orchestrator and close-out driven, while `R-MCP-2` gives agents a request tool. | ANA-2's answer: an `item_status` request is an `item_note` with `via_step_id` set, never a transition. MOD-11 owns the tool; MOD-4 owns the rule that nothing an agent says moves a status. |
| 12 | **`fanout_index = -1` relies on the absence of a CHECK.** A later migration adding `CHECK (fanout_index >= 0)` would break every judged run. | The `0003` comment on the column names `-1` explicitly, which is the only durable place to record it; validation criterion 8 asserts a judge step round-trips. |
| 13 | **Two orchestrators on one box.** A second `htui` instance could adopt runs the first is running. | `lease_owner` is per process, the refresh is a CAS on it, and a zero-row refresh means abandon without writing. Adoption requires an expired lease, so a live orchestrator is never displaced. |
| 14 | **The seeded graphs cannot run until agents exist.** `phase_agent` has no rows anywhere and MOD-6 seeded no `agent` rows. | §7's fallback chain ends in a named refusal (`no_candidate_agent`, item `blocked`) rather than a panic or a silent skip, and MOD-2's seed fills `agent` on first connect. |

---

## 12. Validation criteria

**For MOD-4 (manual mode).**

1. A `FEAT` item runs `prd -> plan -> implement -> review` end to end against `FakeDriver` and
   `FakeIsolator`, producing four `run_step` rows at positions 0 to 3, four documents whose kinds are
   the phase names, and `item.status` moving `open -> queued -> in_progress -> done`.
2. Editing a phase's `gate` while a run is live changes nothing about that run: the run's steps
   resolve gates from `graph_snapshot`, and a fresh run of the same graph sees the new value.
3. A run whose live graph's topology hash no longer matches its snapshot refuses to resume after a
   restart and parks at `awaiting_approval` rather than proceeding.
4. Every `(from, to)` pair outside §4.3's three tables is refused by `transition`,
   `transition_run` and `transition_step` with `StoreError::Constraint`; every pair inside it
   succeeds; a stale `from` returns `Ok(false)`; and `version` is unchanged in every case.
   `MemStore` and `PgStore` give identical answers for all three.
5. A gated step with a missing output document cannot be approved: `AnswerGate { Approved }` returns
   an error naming the missing kind, and the step stays `awaiting_approval`.
6. A `review` step whose document says `request-changes` re-runs the implement phase at the same
   `position` with `attempt = 2`, marks the previous implement and review steps `superseded`, and
   the new implement step's resolved inputs contain the review document. With `retry_limit = 1` a
   second rejection escalates: run `awaiting_approval`, item `blocked`, an `item_note` naming the
   phase and the attempt count.
7. Two consecutive implement attempts producing an identical `after_hash` stop the loop before the
   retry budget is exhausted, with the stop reason recorded.
8. A phase with `fan_out = 3` and a configured judge produces three candidate steps at
   `fanout_index` 0, 1, 2 plus one judge step at `fanout_index = -1`; the winner is `selected = true`
   and `done`, the losers are `selected = false` and `superseded`, all three documents survive, and
   the next phase's resolved inputs contain only the winner's.
9. A candidate whose `verify_outcome = 'fail'` is eliminated before the judge is called when at
   least two candidates pass; when exactly one passes, the judge is not called at all and that
   candidate wins.
10. A judge whose two orderings disagree, or whose verdict block is unparseable, leaves every
    candidate `done` with `selected` NULL and parks the run at `awaiting_approval`; a human
    `SelectFanout` then completes it.
11. A `worktree` step on a two-repo project writes two `run_step_tree` rows and two
    `run_step_commit` rows, both trees are outside every `repo_box_path`, both branches are named
    `htui/<step_id>`, and `git worktree list` in the managed repo shows them.
12. A `local` step on a dirty tree records `before_hash` = HEAD with `dirty = true`, and the
    recovery sweep refuses to reset it, parking instead.
13. Cancelling a run removes every worktree and copy it created, leaves the managed repo's own tree
    untouched, and leaves no `htui/` branch checked out anywhere.
14. A capability refusal writes no `run` row, sets `item.status = 'blocked'`, and writes an
    `item_note` whose body is exactly the missing tags; `Unblock` returns the item to `open` and a
    subsequent run on a box with those tags succeeds.
15. Two items declaring `src/**` in the same repo do not run concurrently; the same two items in
    different repos of a multi-repo project do; an item with empty `touched_paths` runs concurrently
    with nothing else in its primary repo.
16. A third run is not admitted while two are `running` on a box with
    `max_concurrent_items = 2`; a run in `awaiting_approval` does not consume a slot but does still
    block an overlapping run.
17. Promoting a step keeps its `run_step` id, appends `follow_up` events with an incrementing `turn`
    to the same `session_event` stream, sets `promoted_at`, and creates no `run(kind='chat')` row.
    `accept artifact` then runs verification, captures `after_hash`, marks the step `done` and
    resumes at `position + 1`.
18. Killing the orchestrator mid-step and restarting: a step whose `after_hash` and output document
    exist is marked `done` and the run continues; a step with neither has its worktree reset to
    `before_hash` and is retried; neither case leaves an orphaned agent process.
19. A run whose box goes offline mid-step stays `running` with an expiring lease, its events land in
    `pending/<project_id>.<run_id>.jsonl`, and on reconnect `upload_pending` plus the sweep resolve
    the step with no duplicate events.
    *Amended by MOD-4 milestone 6, 2026-09-25 (plan OQ-11, D175):* MOD-25 made `htui` online-only,
    so nothing is buffered and there is no `pending/` file. The criterion is re-scoped and proved as:
    a store outage mid-step fences the walk before its lease lapses, and after the store returns
    this process's sweep adopts and adjudicates the run.
20. Close-out on a `done` item writes one `summary` document whose body contains one row per
    `(repo, step)` with `before_hash..after_hash`, moves the item to `closed`, sets `closed_at`, and
    is refused while any run of the item is non-terminal.
21. All seven `R-TUI-4` actions are reachable from `runs.rs::on_key` and each renders the actual
    state on a compare-and-set miss; the step list renders agent, model, gate state, usage and
    duration, which requires the `RunStepSummary` additions of §6.2.
    *Amended by MOD-4 milestone 6, 2026-09-25 (blueprint F-R):* every action is reachable. In
    production `approve` is greyed with the guard's sentence until MOD-11, because no step writes
    its `output_kind` document before then.

**For MOD-12 (auto mode).**

22. `ready_items` returns exactly `docs/ANA-9.md` §7.4's rows in `priority DESC, created_at` order,
    including the box tag subset clause, and excludes an item with a live `blocked_by` edge to a
    non-terminal item and an item whose status is `blocked`.
23. In auto mode, a phase with `gate = 'always'` and `gate_hard = false` resolves to
    `gate_effective = 'never'` **in the snapshot**, its steps land `done` with
    `gate_outcome = 'skipped'`, and the same phase with `gate_hard = true` still parks.
24. Switching a run's mode is impossible mid-run: `run.mode` is set at insert and the snapshot is
    already downgraded, so a manual run cannot become unattended by an edit.
25. A batch whose accumulated `run_step.usage` reaches `per_token_cap_batch` admits no further run,
    and a run whose remaining budget is below one attempt's estimate admits no further attempt.
26. Auto mode uses the same admission transaction as manual mode: a manual run and an auto run that
    overlap serialise against each other, in either order.
27. An escalation raised while unattended (review loop exhausted, judge undecided, capability
    refusal) appears in the queue overlay with its reason and its item, and pausing the queue stops
    admission without cancelling anything already `running`.
28. A run whose `target_box_id` is not the local box is never claimed, stays `queued`, and is
    reported as such rather than silently ignored (`R-ORCH-12`'s v1 half).

---

## 13. Sources

**Local repository** (branch `main`, HEAD `2ec833d`, read 2026-09-05 and 2026-09-06):

`docs/REQUIREMENTS.md` (read in full; §3 `R-ENT-3..12` lines 71-107, §5 `R-AGT-1..8` lines 129-150,
§6 `R-ORCH-1..13` lines 154-190, §7 `R-HIS-1..3` lines 194-198, §8 `R-PRM-1..4` and `R-SKL-1..4`
lines 202-217, §9 `R-SEC-1..4` lines 221-230, §10 `R-MCP-1..4` lines 234-243, §11 `R-TUI-1..9`
lines 247-267, §15 superseded material lines 301-308);
`CONCEPTS.md` (read in full; single source of truth, agents and orchestration, one cache writer,
status compare-and-set);
`docs/ANA-9.md` (§2 invariants, §4.2 version and status lines 161-169, §4.3 event payloads, §4.4
mirror and cursor lines 266-339, §5.0 migration rule lines 345-356, §5.4 step graphs lines 493-554
including the delegation at 495-496 and the override rule at 552-553, §5.5 items and documents lines
555-656, §5.6 skill bindings lines 679-692, §5.8 runs and steps lines 725-799, §5.9 `app_setting`
lines 801-812, §5.10 seed lines 814-818, §6.1 store traits lines 824-847, §6.2 refresh, §7.1 mint,
§7.2 compare-and-set, §7.3 upstream walk, §7.4 ready items lines 944-960, §7.5 replay, §8 superseded
table, §9 phasing lines 990-1016 including the MOD-4 ownership clause at 1009-1010, §10 risks);
`docs/ANA-4.md` (read in full; §1 stale premises, §3 protocol surface, §4.1 `AgentDriver`,
`SessionSpec`, `DriverCaps`, the recorder and the coalescing rule lines 181-372, §4.3 permissions
and the CLI capability gap lines 470-555 including the gate interlock at 554, §4.4 session load and
resume lines 614-622, §5 registry JSONB shapes, §7 quota, caps and the `R-AGT-8` predicate lines
1077-1151, §8 crate layout and its rationale lines 1154-1237, §9 phasing and migration `0002`
lines 1240-1284, §10 risks including risk 9 at 1319-1321, §11 validation);
`HANDOFF.md` (read in full; the ANA-2 item text, MOD-2, MOD-4, MOD-7, MOD-11, MOD-12, MOD-13,
MOD-15 lines and the summary table);
`DECISIONS.md`; `.claude/rules/workflow-docs.md` (document law: file roles, ID minting, the index
line format and its parse regex, the lifecycle, the live-coordinates rule at lines 123-125);
`docs/decisions/ana/ana-4.md`; `docs/decisions/ana/ana-9.md`; `docs/decisions/mod/mod-1.md`;
`docs/decisions/mod/mod-6.md` (the 32-table migration, migration gating and the "schema is newer
than this htui" refusal, the seed, no `agent` seed rows, the mirror tables and the refresher, the
pending buffer, the review gate and its "running steps frozen in the mirror" finding);
`docs/ANA-1.md` and `docs/ANA-8.md` (superseded per `docs/REQUIREMENTS.md` §15; read only to confirm
that their `run`, `run_step` and `item_document` shapes are not re-derived here).

`crates/htui-store/migrations/0001_init.sql` (the DDL as shipped: `box` 53-75, `agent` 94-106,
`agent_box` 112-123, `project` 143-155, `repo` 185-196, `repo_box_path` 202-208, `step_graph`
214-222, `step_graph_phase` 228-248, `phase_agent` 254-260, `prompt_template` 266-277, `item_kind`
282-296, `item` 310-338, `item_link` 357-372, `document` 389-404, `run` 447-467, `run_step` 472-499,
`run_step_commit` 501-511, `session_event` 513-535, `command_run` 537-553, `app_setting` 557-561,
the `updated_at` trigger loop and its exclusions 563-580, the deferred FKs 582-597);
`crates/htui-store/cache_migrations/0001_mirror.sql` (seventeen mirror objects; `item` with
`touched_paths` at line 80, `run` 103-110, `run_step` 112-119, `run_step_commit` 121-123,
`session_event` 125-128; no `step_graph`, `step_graph_phase`, `phase_agent`, `prompt_template`,
`agent`, `agent_box`, `command_run` or `app_setting`);
`crates/htui-core/src/model/item.rs` (`Status` 8-28, `is_terminal` 30-36, `Item` including
`touched_paths` and `step_graph_id`, `ItemSummary` 102-125, `ItemFilter.ready` and its
"MOD-4 owns matching against a real box" doc at 137-140, `NewItem`, `ItemPatch`, `ItemRevision`);
`crates/htui-core/src/model/run.rs` (`RunKind` 9-17, `RunMode` 19-27, `RunStatus` 29-45,
`is_active` 47-52, `StepStatus` 55-72 with the `Superseded` doc at 69-70, `GateOutcome` 75-84,
`Run`, `RunStep` with `attempt` at 135 and `fanout_index` at 136, `RunStepCommit`, `RunStepSummary`
184-209, `RunSummary` 211-243);
`crates/htui-core/src/model/kind.rs` (`Gate` 10-20, `Isolation` 22-34, `CommandQueue` 36-47,
`ItemKind`, `StepGraph`, `StepGraphPhase` 88-124, `PhaseAgent` 127-137, `PromptTemplate`);
`crates/htui-core/src/model/{mod,document,link,ids,box_,agent,hierarchy,event}.rs` (the
enum-versus-CHECK agreement test, the append-only document rule and its open kind set, the ID
newtype and client-minted UUIDv7 rule, `box.settings` and `BoxInfo`, `Project.settings` as an
untyped `Value`, `Repo` and `RepoBoxPath`, the fourteen `EventKind` values);
`crates/htui-core/src/store/traits.rs` (`ReadStore` 18-32, `WriteStore` 38-51 and the
"runs, steps, events, ..." comment at 50, `UpdateOutcome`);
`crates/htui-core/src/store/{error,mem,conformance,mod}.rs` (`StoreError`'s six variants and the
`Unreachable` versus `Backend` split, `MemStore`'s `#[expect(dead_code, reason = "loaded now, read
by MOD-2 / MOD-4 / MOD-15")]` fields, the fifteen conformance `CASES` and the trait-only rule);
`crates/htui-core/src/fixtures.rs` (`KIND_SPECS` matching `R-ENT-6`, the uniform phase construction
at 601-623, `box.settings.max_concurrent_items` at 375, the run and step fixtures at 1102-1219 with
`attempt: 0` at 1169 and 1203, and the NULL `graph_snapshot` at 1113 and 1130);
`crates/htui-store/src/pg/{mod,read,write,demo,rows}.rs` (`SEEDED_TAGS` and `SEEDED_SETTINGS` 44-48,
the no-`agent`-rows note, `MigrationState`, `items()` and its partial readiness clause 96-102,
`runs()` and its step ordering at 329, `step_events`, the inherent-method rationale at 378-382,
`active_runs` 478-496, `transition` 228-269, the demo loader's untouched-table list at 23-25);
`crates/htui-store/src/backend.rs` (the no-`WriteStore`-for-`Backend` rule at 6-9, `writable`
97-102);
`crates/htui-store/src/cache/{refresh,pending}.rs` (the ten-table refresh list 200-210,
`run_step_commit` riding its parent step 221-222, the offline chat insert 28-31);
`crates/htui/src/store_worker.rs` (the single-`Backend` rule, `Origin` and `Seq`, `StoreRequest` and
its `name()` and `try_serve` arms, the unbounded-channel rationale);
`crates/htui/src/{event_loop,testkit}.rs` (the three-arm promise, `Harness::settle`'s inline serve);
`crates/htui/src/app/{action,state,mod}.rs` (the no-mutation rule, `Ctx::request`, the tab and
overlay registries);
`crates/htui/src/ui/tabs/backlog/detail/{mod,runs,graph}.rs` (the registry rule naming MOD-4's
approve and reject at 4-7, the scroll-only `RunsTab` and its dropped step list, the Graph sub-tab
being the link graph rather than the step graph);
`Cargo.toml` and `Cargo.lock` (workspace members, MSRV, the tokio feature list without `process`,
the absence of `git2`, `gix` and any glob crate, the presence of `sha2`, `similar` and `tempfile`);
`rust-toolchain.toml`.

**In-house workflow surface, read as prior art** (this repo and the user scope):
`.claude/skills/handoff-run/SKILL.md` (the phase chain actually run, the fact-check contract at
110-124, the file-set intersection rule at 120-121 and 131-133, the reviewer gate at 138-147, the
hard rules at 166-180);
`.claude/skills/handoff-run/references/{routing,selection,lifecycle}.md` (the routing decision table
and the maintainer override, the R1 to R4 selection ranking and the ask rule, close-out steps P0 to
P3 and the push gate);
`.claude/skills/handoff-run/scripts/next-item.ps1` (derived blockedness recomputed on every run, the
phase-scoped blocker rule, the in-flight signals);
`.claude/workflow-config.json`; `~/.claude/agents/rust-reviewer.md` (the Approve / Warning / Block
criteria at 97-102 and the pre-review command set at 19-24);
`~/.claude/commands/plan.md` (the CONFIRM gate);
`.claude/plans/mod-1-tui-scaffold.plan.md` and `.claude/plans/mod-6-postgres-store-cache.plan.md`
(the wave structure, the verified-claims tables and their `T4 ∩ T5 = ∅` file-set proofs, the
standing implementer preamble, `isolation: "worktree"` as a literal Agent parameter);
`docs/decisions/mod/mod-1.md` and `docs/decisions/mod/mod-6.md` (the review-fix-re-review loop as
practised, the worktree waves and their two integration strategies, the commit lists showing several
commits per item).

**Web** (fetched 2026-09-05, cited only where a verdict rests on them):

*Durable execution and resume.*
https://docs.temporal.io/encyclopedia/retry-policies (retry belongs to the activity, not the
workflow; bound by timeout rather than attempt count);
https://docs.langchain.com/oss/python/langgraph/checkpointers (super-step checkpoints, `next`,
pending writes preserving completed siblings);
https://docs.langchain.com/oss/python/langgraph/interrupts (a resumed node "restarts the entire node
from the beginning", so pre-gate side effects duplicate; static interrupts are "not recommended for
human-in-the-loop workflows");
https://learn.microsoft.com/en-us/agent-framework/workflows/checkpoints (`graph_signature_hash`,
`previous_checkpoint_id`, pending request info events);
https://www.diagrid.io/blog/still-not-durable-how-microsoft-agent-framework-and-strands-agents-repeat-the-same-mistake
("no heartbeat, no lease mechanism, no watchdog");
https://docs.restate.dev/develop/ts/external-events (awakeables, and the caveat that awaiting one in
an exclusive handler queues every other call to the object);
https://www.dbos.dev/blog/why-postgres-durable-execution (a non-transactional step "may execute
twice ... should be idempotent or otherwise resilient to re-execution");
https://code.claude.com/docs/en/workflows (the replay rule by start order and prompt difference, the
enforced determinism, "No mid-run user input", the concurrency caps and the refusal to cap silently,
the advisory large-workflow warning, the `/workflows` action set, `/deep-research`'s unverified
versus refuted distinction);
https://learn.microsoft.com/en-us/azure/azure-functions/durable/durable-functions-phone-verification
(an approval raced against a durable timer, and the rule that the timer must be cancelled).

*Queues, leases and claiming.*
https://www.postgresql.org/docs/current/sql-select.html (`FOR UPDATE ... SKIP LOCKED` and its
inconsistent-view caveat; the `ORDER BY` plus locking-clause ordering caveat);
https://riverqueue.com/docs/maintenance-services and https://riverqueue.com/docs/reliable-workers
(the rescuer, the stuck horizon that must exceed the job timeout, the "finished but not marked"
case, leader election for maintenance);
https://worker.graphile.org/docs/error-handling and https://github.com/graphile/worker/issues/169
(the fixed four-hour horizon and the six-hour job it double-ran);
https://www.dimamik.com/posts/oban_py/ (`INSERT ... ON CONFLICT` leadership with a TTL refreshed at
twice the rate);
https://hexdocs.pm/oban/2.11.2/changelog.html (rescue after sixty minutes; "inaccurate rescues");
https://airflow.apache.org/docs/apache-airflow/stable/administration-and-deployment/scheduler.html
(the critical section as a row-level write lock over the limit table; batching defeating priority);
https://github.com/apache/airflow/issues/15793 (queuing more tasks than there are slots by ignoring
claimed work).

*State machines and status vocabularies.*
https://kevin.burke.dev/kevin/state-machines/ (the guarded `UPDATE ... WHERE status IN (...)
RETURNING *` form and the anti-pattern it replaces);
https://thoughtbot.com/blog/inserting-state-transitions-in-postgres (why a transition log unmasks
what a status column hides);
https://cursor.com/docs/cloud-agent/api/endpoints (agent status `ACTIVE|IDLE|ARCHIVED` split from
run status, with recoverable errors kept on the run; the per-agent-not-per-run git snapshot, cited
as the shape *not* to copy);
https://statecharts.dev/glossary/guard.html (a guard cannot wait and must have no side effects,
which is why a human approval is a state and not a guard);
https://www.state-machine.com/doc/AN_Crash_Course_in_UML_State_Machines.pdf (extended state
variables and guards as "the primary mechanism of architectural decay");
https://linear.app/docs/configuring-workflows and
https://support.atlassian.com/jira-cloud-administration/docs/configure-advanced-issue-workflows/
(a small machine-meaningful status category set beneath a larger human-facing status set; the
mandatory ordered post-function list per transition).

*Fan-out selection and judging.*
https://github.com/SWE-agent/SWE-agent/blob/main/sweagent/agent/reviewer.py (`ScoreRetryLoop`'s
multi-predicate exit including `min_budget_for_new_attempt` and `cost_limit` with its docstring,
`get_forwarded_vars`, the pointwise `Reviewer` with `n_sample: 5`, the comparative `Chooser` with its
exit-status prefilter, `Preselector`, `max_len_submission`, and the index-0 fallback cited as the
shape *not* copied; the separate reviewer cost accounting);
https://arxiv.org/pdf/2504.15253 (JETTS: judge protocols worse than the greedy baseline on MBPP+);
https://github.com/lechmazur/position_bias (64.3% first-shown pick rate, 41.3% flip rate on swapped
order);
https://arxiv.org/pdf/2504.14716 (pairwise preferences flipping in about 35% of cases under a
distractor versus 9% for absolute scores);
https://arxiv.org/abs/2207.10397 (CodeT: execution agreement for code selection);
https://help.openai.com/en/articles/11428266-codex-changelog (the shipped human-select fan-out capped
at four attempts).

*Isolation, worktrees and build caches.*
https://git-scm.com/docs/git-worktree (what is shared and what is per-worktree, branch exclusivity,
`--lock` and its race-free form, `prune`, and the BUGS section on submodules);
https://git-scm.com/docs/git-gc (`--prune=now` and concurrent-write corruption);
https://github.com/anthropics/claude-code/issues/55724 (thirteen parallel agents, five commits
succeeded and eight failed on lock contention; auto-cleanup destroying uncommitted work; the
retry-with-backoff mitigation);
https://code.claude.com/docs/en/sub-agents (`isolation: worktree`, the enforced cwd and git-command
checks, the default-branch base ref, auto-cleanup of a no-change worktree, resumability by agent id
and the one-shot agents that have none, the model resolution order);
https://doc.rust-lang.org/cargo/reference/build-cache.html (`CARGO_TARGET_DIR` and `sccache` as the
sharing mechanism);
https://github.com/rust-lang/cargo/issues/14053 (per-target locking and the cross-worktree
overwrite race);
https://blog.howardjohn.info/posts/shared-rust-build/ (absolute paths in `.d` files; a `target/`
copy measured at about two minutes);
https://cmake.org/pipermail/cmake/2007-June/014502.html (CMake build trees are not relocatable);
https://www.ctrl.blog/entry/file-cloning/ (no reflink on stock NTFS);
https://github.com/dagger/container-use (branch-per-environment with an explicit merge, and what
worktrees do not isolate);
https://yegge.ai/gastown (the Refinery: serialising at merge rather than at edit, and the stated
accuracy-for-throughput trade).

*Concurrency and partitioning.*
https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax (concurrency
groups as a computed key; the cancel-pending default, cited as the behaviour *not* copied);
https://buildkite.com/docs/pipelines/configure/workflows/controlling-concurrency (limits snapshotted
onto the job at creation, and the FIFO breakage when they are not);
https://arxiv.org/html/2606.00953v1 (Co-Coder: self-coordinated agent teams at the lowest pass rate;
file-based parallelism costing 44% more for a negligible gain);
https://arxiv.org/html/2607.04697v2 (33,596 agent PRs; a 19.8% textual conflict rate between
identical concurrent agents).

*Promotion, resume and gates in shipped products.*
https://agentclientprotocol.com/protocol/session-setup (`loadSession`, `session/load` replaying the
whole conversation, `session/resume` restoring without replay);
https://code.claude.com/docs/en/cli-reference and https://code.claude.com/docs/en/headless
(`--resume` by explicit id, the cwd-scoped session lookup, `--continue`'s unreliability in
non-interactive mode);
https://docs.openhands.dev/sdk/guides/convo-persistence (persisting the agent configuration so a
resume can detect a changed agent or model);
https://jules.google/docs/review-plan/ (timer-based auto-approval, cited as the behaviour rejected)
and https://jules.google/docs/changelog/2026-01-26-1/ (the Planning Critic substituted for a skipped
human gate);
https://docs.factory.ai/cli/configuration/settings (an org-managed autonomy ceiling no per-call
setting can raise, the generalisation of `R-ORCH-2`'s hard flag);
https://github.blog/news-insights/product-news/github-copilot-meet-the-new-coding-agent/ (human
approval required before CI runs, and the triggerer barred from approving, as the canonical examples
of what a hard gate is for);
https://antigravity.google/docs/artifacts/ (a review policy attached to the artifact kind);
https://arxiv.org/abs/2503.13657 (MAST: "Unaware of Termination Conditions" as a near-fatal failure
mode, which the gate, retry-limit and verification triple exists to eliminate).
