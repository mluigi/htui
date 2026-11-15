# MOD-130 — Execution targets: target record, child boxes, target cascade

> Routed as **PRD** by `/handoff-run MOD-130` (C2 and C4 fired). Ultracode recommended for
> implement; accepted. The contract is `R-ORCH-15`, `R-ORCH-12`, `R-ORCH-11`, `R-BOX-1..4` and
> `R-NF-2`; the design is `docs/ANA-42.md` §7.1, §7.2 and §7.8, with all six §8 decisions accepted
> by the maintainer on 2026-11-14 (`docs/decisions/ana/ana-42.md`). Neighbours: MOD-44 (container
> box, open), MOD-43 (remote dispatch, open), MOD-131 and MOD-132 (blocked on this item), MOD-47
> (enrolled boxes, trigger-gated).

## Problem

`htui` has no way to say *where* an item runs. A run already carries `target_box_id` and
`executing_box_id`, but the engine always writes this box into the first and the claim copies the
claiming box into the second, so the two never differ. The queue treats any run whose target is not
this box as "live elsewhere" and never starts it. Nothing in the store describes a docker container,
an ssh machine or an enrolled box, a `box` row cannot be a child of another, and there is no
Settings → workspace → project → workflow → item default. So the four follow-up items that make
`R-ORCH-15` real (MOD-44, MOD-131, MOD-132, MOD-47) have no record to hang off, and the redesigned
surfaces that show "runs on …" (MOD-116, MOD-119, MOD-123, MOD-124) have nothing to read.

## Evidence

Facts gathered from the tree on 2026-11-14 while routing; each is re-checked by the plan fact-check.

- **A box is a machine with no kind or parent.** `box` has `os_family TEXT NOT NULL CHECK (os_family IN
  ('windows','linux','macos'))`, `htui_version NOT NULL`, tags, `settings` JSONB
  (`crates/htui-store/migrations/0001_init.sql:53-75`, CHECK at `:57`); `0005_box_identity.sql:15`
  dropped `UNIQUE (user_id, hostname)` and added `machine_fingerprint`, `edit_version`,
  `probe_spec_digest`. `BoxRow` and `OsFamily` mirror it (`crates/htui-core/src/model/box_.rs:13-70`).
  No `kind`, `parent_box_id`, transport or address exists anywhere.
- **Capability hangs off `box_id`.** `box_tool` (`0001_init.sql:81-87`), `agent_box` (`:112-122`),
  `workspace_box_path` (`:173-178`) and `repo_box_path` (`:202-207`) all key on a box, so a child box
  reuses them unchanged (ANA-42 §7.1).
- **The run's two box ids exist and are always equal for graph runs.** `run.target_box_id UUID NOT NULL
  REFERENCES box(id)` carries the comment "R-ORCH-12 reserved: executes only when local", beside a
  nullable `executing_box_id` (`0001_init.sql:455-456`); `0003_orchestration.sql:40-43` added
  `lease_box_id`, `lease_owner`, `lease_expires_at`. `Run.target_box_id`'s doc still says "version one
  executes only when it is local" (`crates/htui-core/src/model/run.rs:295-296`).
- **Who writes them.** `Engine::enqueue_with` checks `missing_tags(item, self.parts.box_id)`, resolves the
  graph with `self.parts.box_id`, and inserts `NewRun { target_box_id: self.parts.box_id, .. }`
  (`crates/htui-orch/src/engine.rs:1413-1477`; `NewRun` at `crates/htui-core/src/model/run.rs:496-529`
  has no executing box). `claim_run` refuses unless `status = queued` **and** `target_box_id == box_id`
  (`crates/htui-core/src/store/mem.rs:6845`; documented on `Claim::NotClaimable`,
  `crates/htui-core/src/model/overlap.rs:134`), checks tags against the claiming box, counts slots and
  overlap over `executing_box_id == box_id`, then writes `executing_box_id = lease_box_id = box_id`
  (`mem.rs:6829-6948`). The Postgres twin reads `target_box_id` under `FOR UPDATE`
  (`PgStore::claim_run`, `crates/htui-store/src/pg/write.rs:7273`; `.sqlx/query-f31771a6…json`).
  Chat runs insert `executing_box_id = target_box_id` at `running` (`.sqlx/query-92b6e515…json`;
  `ChatRunSpec`, `run.rs:439-441`).
- **The worker polls by target.** `queued_runs_on_box` is `WHERE target_box_id = $1 AND status = 'queued'`
  (`crates/htui-store/src/pg/read.rs:2580`, `mem.rs:1136`), so a run driven by this box but aimed at a
  child box would never be polled by this box's `htui worker`.
- **The queue's `Elsewhere` keys on the target.** `classify_entry` returns `Running` when
  `run.target_box_id == here` and `Elsewhere { hostname }` otherwise
  (`crates/htui-core/src/model/queue.rs:469-540`, branch at `:475`; `EntryState` at `:336-367`, whose
  doc says "MOD-43 owns choosing; M3 reports"). The hostname is the target box's
  (`mem.rs:6505-6508`); `run_hostname` already prefers `executing_box_id` (`mem.rs:2256`). The overlay
  sentence is `crates/htui/src/ui/overlay/queue.rs:303`.
- **The cascade's levels live in four tables, and the workspace one is a scope row.** `workspace` has no
  settings column (`0001_init.sql:129-137`); `project.settings` is JSONB (`:143-155`);
  `workspace_project` is many-to-many, `PRIMARY KEY (workspace_id, project_id)` (`:161-167`), so a project
  can sit in several workspaces. Since MOD-147 every workspace owns one hidden scope `project` row
  (`0026_workspace_scope.sql:53-59`). The workflow is a `step_graph`, owned by a project, a workspace or
  the library, with a `revision` every graph write bumps (`0020_step_graph_v2.sql:43-58`). An item pins
  its workflow with `item.step_graph_id` (`0001_init.sql:324`, "NULL = kind default"), which is a
  version-guarded spec field (`ItemSpec`, `crates/htui-core/src/model/item_spec.rs:83`; `SpecField`,
  `crates/htui-core/src/model/item_merge.rs:25`).
- **A run knows its workspace.** `NewRun.workspace_id` (MOD-125 D3) is the workspace a run was started
  in; `None` is never gated by a stop (`run.rs:524-527`).
- **The house pattern for "which level answered" exists.** `resolve_budget(phase, project, app)` returns
  a value and a `BudgetSource { Phase, Project, AppSetting, AppSettingDefault }`
  (`crates/htui-core/src/prompt/settings.rs:726-735`, `:813-833`), recorded in `trim_record`. Settings
  rungs are `SettingRung { App, Project(ProjectId), Phase(PhaseId) }`
  (`crates/htui-core/src/store/traits.rs:4737-4745`), compare-and-set on the App rung
  (`settings_app_rung_validates_and_cas`, `crates/htui-core/src/store/conformance.rs:6787`). The
  skeleton's `inherited_for_item`/`inherited_target` show the product order (workflow pin, project,
  workspace, Settings) and the source label (`spikes/ui-skeleton/src/app.rs:709-737`); its `Target`
  carries `planned` (`spikes/ui-skeleton/src/data.rs:82`). The skeleton is throwaway (ANA-42 §2.6).
- **Store layering for a new writer.** Trait method on `WriteStore`/`ReadStore`, `MemStore` and `PgStore`
  bodies, `Writer` dispatch (`crates/htui-store/src/writer.rs:1074` for `claim_run`), the worker seam
  (`WorkerStore`, `crates/htui-core/src/store/worker.rs:264`; `crates/htui-store/src/worker.rs:794`), the
  spy stores in `htui-agent` (`UsageSpy`, `crates/htui-agent/src/conformance.rs:1210`), a conformance
  case in `CASES` (`crates/htui-core/src/store/conformance.rs:120`), and `.sqlx` offline data
  (`crates/htui-store/.sqlx/`, 533 entries).
- **A new migration moves pins.** Latest is `0036_activity_event.sql`. `tests/it/migrations.rs` pins the
  applied list `1..=36` (`:128-152`) and `TABLES` (57 entries, `:37-111`); `MigrationState::Pending(36)`
  and `HeadlessError::MigrationsPending(36)` are pinned at `crates/htui-store/tests/it/connect.rs:140,241`
  and `migrations.rs:4270,4371,4380,4403`; `COMMENT ON COLUMN` texts are pinned per migration
  (`migrations.rs:252-501`).
- **Mirrored tables move the cache.** `box`, `workspace`, `project`, `item` and `run` are mirrored;
  `step_graph` is not (`MIRRORED_TABLES: [&str; 23]`, `crates/htui-store/src/cache/mod.rs:51-75`; length
  pinned at `crates/htui-store/tests/it/cache.rs:3150`). A new column on a mirrored table needs a cache
  migration (last: `cache_migrations/0012_requirement_keys.sql`) and rebuilds each box's mirror once
  (`0026_workspace_scope.sql:19-21`). A new mirrored table also moves the "the 23 mirrored tables" text
  in `crates/htui/tests/it/snapshots/it__connection__confirm.snap`.
- **The probe already yields the gate R-NF-2 needs.** `docker` is a probed tool and the `docker` tag fires
  on `docker` or `podman` (`crates/htui-agent/src/box_probe/spec.json:10`, `:50`); `git` is probed
  (`:14`); `ssh` is not.
- **R-NF-2 today** (`docs/REQUIREMENTS.md:892-902`) names Postgres, the shared Qdrant (failing open), the
  agents and the code-context tools, and has no `dockerd`/`sshd` sentence; ANA-37 replaced it, so the
  ANA-42 sentence is merged in, not substituted (`HANDOFF.md:431-433`).
- **The surfaces that set a target are other items'.** MOD-123 adds Settings › Targets (list, add
  through the htui agent, default) (`HANDOFF.md:1003-1007`); MOD-116 the workspace's default target with
  `T` (`:928-933`); MOD-119 the Backlog's "runs on …" line and `t` "how it runs" (`:965-974`, "targets
  beyond this box need MOD-130"); MOD-124 `T` pins a target in the workflow editor (`:1013-1019`).
  `R-TUI-8` lists the targets and default target among Settings sections (`docs/REQUIREMENTS.md:761`).
- **Neighbours are open.** MOD-44 (child box of kind `container`, `docker exec -i` decorator; writes no
  `R-NF-2` itself, `HANDOFF.md:375-382`) and MOD-43 (target box on run start; "a non-local target stays
  `queued`" now applies to the enrolled kind only, `:359-363`) are not built.

## Users

- **Primary**: the maintainer, who today runs items only on this box and does container sandboxing by
  hand with `scripts/hr` (TOOL-7). The need fires once MOD-44 lands: an item should run in docker here
  because a project says so, without a per-run choice.
- **Also served**: MOD-44, MOD-131, MOD-132 and MOD-47, which plug a transport, preparation and
  collection into the record; MOD-116, MOD-119, MOD-123 and MOD-124, which read and write the cascade;
  the htui agent (MOD-121), which adds targets on the person's behalf (`R-AGT-11`).
- **Not for**: running anywhere but this box in this item. No transport, preparation or collection ships
  here.

## Hypothesis

We believe **a `target` record that points at a box, a nullable `target_id` at each cascade level with a
resolver that names the level that answered, a run that snapshots the resolved target, and a queue that
keys on the box that drives a run rather than the box it runs on** will **give every later target kind
one place to plug in and every surface one answer to "where does this run, and why"** for **the
maintainer and the items that follow**.

We'll know we're right when **an item whose project names a docker target and whose workflow inherits
resolves to that target with source `project`; its run records the target and source; a run whose
`target_box_id` is a child box of this box and whose driver is this box is claimed and classified
`Running` here, not `Elsewhere`; and every non-local kind is refused at run start with a sentence naming
the item that brings it** — with no change to how a local run behaves today.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Schema | `target`, `box.kind`, `box.parent_box_id`, the cascade columns and the run snapshot apply forward-only; pins move by exactly what the migration adds | `migrations::` and `connect::` tests (applied list, `TABLES`, `Pending(N)`, column comments) |
| Store parity | Every new reader and writer behaves alike on `MemStore` and `PgStore`, refusals included | Conformance cases in `CASES`, run on both stores |
| Resolver | Every combination of set/inherit across item, workflow, project, workspace and Settings yields the right target and source; absent everywhere yields the seeded local target with source `default` | Table-driven unit test beside `resolve_budget`'s |
| Run snapshot | A run records the resolved target and source at enqueue; changing the cascade afterwards moves no queued or running run | Engine test over `MemStore`; conformance case |
| Driver semantics | A queued run driven by this box with a child-box target is claimed here, polled by this box's worker, counted in this box's slots, and classified `Running`; a run driven elsewhere is `Elsewhere` and names the driving box | `classify_entry` unit tests; `claim_run` and `queued_runs_on_box` conformance cases |
| Local unchanged | Every existing local run path (manual, auto, worker, chat) behaves as before | The full gate, existing engine and queue tests unedited except pinned counts |
| Refusals | A non-runnable kind, a target bound to another box, and a missing target each refuse at run start with a sentence, writing no run | Engine tests; queue refusal test |
| Mirror | The offline reader names a run's and an item's target | Cache test after the cache migration |
| R-NF-2 | The requirement text carries the `dockerd`/`sshd` sentence beside ANA-37's text | Doc review; `validate-workflow-docs` |

## Scope

**MVP** — the target record and box kinds in the store, the cascade columns and their resolver with a
named source, the run's target snapshot, the driver-keyed queue and claim, the refusals for kinds that
cannot run yet, read-only display of where a run runs, and the `R-NF-2` amendment. Only the `local`
kind runs.

Concretely in scope:

- **Target record (ANA-42 §7.1).** A `target` table: `kind` (`local | docker | ssh | ssh_docker |
  enrolled`), a name, a JSONB `locator` holding names and addresses only (never values), a nullable
  `box_id`, and the probe outcome the derived `status` reads. One seeded `local` target means this box.
  Readers, writers (create, edit, retire) and conformance cases.
- **Box kinds.** `box.kind` and `box.parent_box_id`, with the `os_family` CHECK relaxed for child boxes
  as the open questions settle; a writer that creates or reuses a child box under a parent and attaches
  it to a `docker` target, which MOD-44 calls once its image is probed ("wires MOD-44's container box to
  the record").
- **Cascade columns and default.** A nullable `target_id` at item, workflow (`step_graph`), project and
  workspace, `NULL` meaning inherit; the Settings default; writers for each level under that level's
  existing compare-and-set token.
- **Resolver.** A pure function in `htui-core`, shaped like `resolve_budget`, returning the target and
  `TargetSource { Item, Workflow, Project, Workspace, Settings, Default }`.
- **Run snapshot.** `enqueue` resolves the target, records it and its source on the run (`R-ORCH-11`),
  sets `target_box_id` to where the work runs and names the driving box.
- **Driver-keyed queue and claim (ANA-42 §7.2).** `claim_run`, `queued_runs_on_box` and `classify_entry`
  key on the box that drives the run; `Elsewhere` names the driver; the enrolled kind alone waits for
  its own worker (MOD-43's rule as amended).
- **Run-start refusals.** A kind with no transport in this build (`docker` until MOD-44, `ssh` and
  `ssh_docker` until MOD-131, `enrolled` until MOD-47) and a target bound to another box are refused
  with a sentence naming why; nothing is written.
- **Read-only display.** Where a run runs and why ("runs on docker-here · from project"), on the
  surfaces that already show a run; the queue overlay's `Elsewhere` sentence names the driving box.
- **Cache.** The new columns on mirrored tables, and the `target` table if mirrored, through a cache
  migration.
- **`R-NF-2` (ANA-42 §7.8).** Merge into the current text: `dockerd` and `sshd` are opt-in per target,
  gated by the probe's `docker` tag and a successful target probe; a project that never adds a target
  needs neither. Also the stale doc comments (`0001_init.sql:455`'s intent stays, the `Run`/`EntryState`
  docs move to the new meaning).

**Out of scope**

- **The container box itself** (image, per-session container, `docker exec -i` decorator, probe and
  agent login inside it, kill-tree by removal) — **MOD-44**.
- **ssh and remote-docker transport**, relay forwarding, helper shipping, remote probe, install and
  login — **MOD-131**.
- **`prepare · <target>` and `collect branch`** (clone, compose project, bundle collect) — **MOD-132**.
- **Choosing another registered box with its own worker**, and the Runs view's worker liveness —
  **MOD-43**.
- **Enrolled boxes** and server-side secrets — **MOD-47**, MOD-48.
- **Setting surfaces**: Settings › Targets (MOD-123), the workspace default with `T` (MOD-116), the
  Backlog's `t` "how it runs" (MOD-119), the workflow editor's `T` pin (MOD-124), adding a target
  through the htui agent (MOD-121).
- **Folder-only and repo-less projects on a driven target** — MOD-145 (they run on this box only).
- **Integration after collect** — MOD-133.
- **Cached preparation, retention sweeps of ephemeral boxes** — later (ANA-42 §9).

## Constraints (fixed before planning)

- **Forward-only migrations (`R-STO-5`).** One new migration (`0037`) plus a cache migration; every pin
  listed in Evidence moves in the same commit. A headless worker never migrates.
- **Local behaviour is the regression bar.** A run on the seeded local target is byte-for-byte today's
  run: same claim, same slots, same tags, same snapshot apart from the new target fields.
- **Names and locators, never values.** A `locator` holds an ssh `Host`, a docker context or image name,
  a run root; never a secret, token or password (`R-SEC-2`, ANA-42 §7.1). The scrubber is not a
  substitute.
- **`R-AGT-5`.** Nothing keys on an agent name; a target kind enters later as a launch decorator, not here.
- **`R-BOX-4`.** No identity depends on a path; a child box has its own UUID and no `box.toml`.
- **`R-NF-3`.** Every new read and write goes through the store worker.
- **Every writer is a compare-and-set** on a token only its own writes change; the item's target rides the
  item `version` if it is a spec field.
- **New `query!` macros need `cargo sqlx prepare`** against a migrated scratch database.
- **`unsafe_code = "forbid"`, MSRV and the workspace lint set unchanged; TDD per repo convention.**
- **`docs/REQUIREMENTS.md` is edited only by maintainer decision:** the `R-NF-2` sentence is the one
  accepted on ANA-42 §8 Q4; no other requirement text changes.

## Delivery Milestones

<!-- Status: pending | in-progress | complete | blocked -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Targets and child boxes exist | Migration `0037` and cache migration: `target` (seeded local row), `box.kind`, `box.parent_box_id`, relaxed `os_family` CHECK, `target_id` at item, `step_graph`, project (workspace via its scope row, or a `workspace` column, per OQ5), the Settings default, `run.target_id` and `run.target_source`. Model types, derived `status`, readers and writers (target create/edit/retire, per-level set, child-box attach) on both stores, `Writer` dispatch, spies, conformance, `.sqlx`, every moved pin. `R-NF-2` merged. | complete (`9ea41c6b`..`f8c3d2b3`, 2026-11-15) | [plan](../plans/mod-130-m1-targets-and-child-boxes.plan.md), [blueprint](../plans/mod-130-m1-targets-and-child-boxes.blueprint.md) |
| 2 | Runs know where they run | The pure resolver with `TargetSource`; `enqueue` resolves, snapshots target and source, sets `target_box_id` and the driving box; run-start refusals for kinds without a transport, for a target bound to another box and for a retired target. Chat runs unchanged. | complete (`c52e87be`..`8b222d18`, 2026-11-15) | [plan](../plans/mod-130-m2-runs-know-where-they-run.plan.md) |
| 3 | The queue keys on the driver | `claim_run`, `queued_runs_on_box`, `classify_entry` and `Elsewhere` key on the driving box; tags and agent candidates are checked against the box the work runs on; slots and overlap stay on the driver; the enrolled kind waits for its own worker. Proven with a child-box target over `MemStore` and `PgStore`. | complete (`cf55f77f`..`d5030c43`, 2026-11-15) | [plan](../plans/mod-130-m3-queue-keys-on-the-driver.plan.md) |
| 4 | Where a run runs is visible | Read-only "runs on <target> · from <source>" on the surfaces that already show a run; the queue overlay names the driving box; the Boxes section shows a child box under its parent. Snapshots updated. | in-progress | [plan](../plans/mod-130-m4-where-a-run-runs-is-visible.plan.md) |

Milestone 1 is the foundation. Milestones 2 and 3 both need it and touch the same engine and store
paths (`enqueue_with`, `claim_run`), so they run in order, not in parallel. Milestone 4 needs 2.

## Open Questions

All eighteen answered by the maintainer on 2026-11-14: **every recommendation accepted** (the
recommended answer of each line below is the decision; cite them as D1..D18 = OQ1..OQ18).

- [x] **OQ1 — What can a person do with a `docker`, `ssh`, `ssh_docker` or `enrolled` target before
  MOD-44, MOD-131 or MOD-47 land?** Recommended: the store accepts all five kinds and the resolver
  resolves them, but their derived status is `planned` and run start refuses them with a sentence naming
  the item that brings them. Why: the record and cascade are complete once, and each later item only
  registers a transport.
- [x] **OQ2 — How is "this box" represented: one seeded `local` target with `box_id NULL` meaning "the
  box that starts the run", or one `local` row per box?** Recommended: one seeded row, `box_id NULL`. Why:
  the Settings default stays portable across boxes, and choosing another box with its own worker is
  MOD-43's, not a `local` row's.
- [x] **OQ3 — How is inherit stored, and what is the Settings default when none is set?** Recommended:
  `NULL` means inherit at every level; the Settings default is one `app_setting` key holding a target id;
  absent, the resolver answers the seeded local target with source `default`. Why: it mirrors
  `resolve_budget`'s `AppSettingDefault` and needs no row per level.
- [x] **OQ4 — Is `status` stored or derived?** Recommended: derived at read time from the kind's support in
  this build, the bound box (present, probed, `docker` tag for docker kinds) and stored last-probe
  columns (`probed_at`, `probe_error`) that MOD-44/MOD-131 write. Why: ANA-42 §7.1 says derived, and a
  stored status goes stale the moment the box or build changes.
- [x] **OQ5 — Where does the workspace level live: the workspace's scope `project` row
  (`0026_workspace_scope.sql`) or a new `workspace.target_id`?** Recommended: the scope row's
  `project.target_id`. Why: one column on `project` covers both levels, and a workspace-owned item's
  project and workspace levels are then the same row instead of two that could disagree.
- [x] **OQ6 — A project in several workspaces: whose workspace level applies?** Recommended: the run's
  `workspace_id` (the workspace it was started in); a run without one skips the level. Why:
  `workspace_project` is many-to-many and the run already records its workspace (MOD-125 D3).
- [x] **OQ7 — Does a run snapshot the resolved target and its source?** Recommended: yes,
  `run.target_id` and `run.target_source` at enqueue; retry and resume keep them. Why: `R-ORCH-11`
  records where a run ran, and a later cascade edit must not move a queued run.
- [x] **OQ8 — How does a queued run name its driver, given `executing_box_id` is `NULL` until claim?**
  Recommended: `enqueue` writes `executing_box_id` with the driving box for every kind but `enrolled`;
  claim and the worker poll key on `COALESCE(executing_box_id, target_box_id)`. Why: ANA-42 §7.2 says
  `executing_box_id` is who drives it; no new column, and MOD-43's other-box dispatch keeps working by
  leaving it `NULL`.
- [x] **OQ9 — Which box do tags (`R-ORCH-10`) and agent candidates check against?** Recommended: the box
  the work runs on (`target_box_id`); slots and overlap stay on the driver. Why: the toolchain and agents
  are inside the container, while the compute and the trees' owner is the driving box.
- [x] **OQ10 — A target bound to another box (docker on box A) chosen while starting on box B?**
  Recommended: refuse at run start, naming the box it belongs to. Why: driving it from B is remote docker
  (MOD-131), and dispatching to A's worker is MOD-43.
- [x] **OQ11 — Is the item's `target_id` a spec field, and does a workflow's pin bump its revision?**
  Recommended: yes to both: the item's target joins `ItemSpec`/`SpecField` under `version` and the
  divergence merge, like `step_graph_id`; a graph pin is a graph write. Why: both are human-edited
  configuration that `R-ENT-10` and the graph's undo already govern.
- [x] **OQ12 — Which `box.kind` values now: `machine | container`, or also `remote`?** Recommended:
  `machine | container`; an ssh machine is a `machine` row with no parent, registered on its first probe
  by MOD-131. Why: it is a real development machine; only a container is a child.
- [x] **OQ13 — How far is the `os_family` CHECK relaxed?** Recommended: the three values stay; a
  `container` box may hold `NULL` until its first probe, and a `machine` may not. Why: lets MOD-44 attach
  the box to a target before the image is probed without weakening machine rows.
- [x] **OQ14 — Deleting a target that a level or a run references?** Recommended: refused while any level
  names it, listing the referrers; a target with run history is retired (hidden from pickers, refused at
  run start), never deleted. Why: a silent fall back to inherit would move work, and ANA-42 §9 asks for
  history-keeping retention.
- [x] **OQ15 — Is `target` mirrored in the cache?** Recommended: yes. Why: the offline Backlog and Runs
  pane must name where a run ran; it moves `MIRRORED_TABLES` to 24 and the `connection__confirm`
  snapshot.
- [x] **OQ16 — Which TUI surfaces are in this item?** Recommended: read-only ones only (milestone 4);
  every setter stays with MOD-123, MOD-116, MOD-119, MOD-124 and the htui agent (MOD-121). Why: those
  items already own the tabs being redesigned, and only `local` can run until MOD-44.
- [x] **OQ17 — Do chat runs follow the cascade?** Recommended: no; chats stay on this box. Why:
  `R-ORCH-15` is about items' work, and a chat run starts `running` here (`ChatRunSpec`).
- [x] **OQ18 — The merged `R-NF-2` wording.** Recommended: after "except the code-context tools in coding
  sessions (R-LATER-7)", add "`dockerd` and `sshd` are opt-in per execution target (R-ORCH-15): only a
  docker or ssh target needs them, gated by the box probe's `docker` tag and a successful target probe;
  a project that never adds such a target needs neither", and append "and on 2026-11-14 (ANA-42, §8 Q4)"
  to the amendment trail. Why: it keeps ANA-37's text whole, as the item requires.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Re-keying claim and the worker poll changes who picks up a local run | Medium | High | Local runs keep `executing_box_id = target_box_id = here`; existing claim, worker and queue tests stay unedited except counts |
| A queued run with `executing_box_id` set reads as started somewhere (Runs pane, activity triggers, `run_hostname`) | Medium | Medium | Audit every reader of `executing_box_id` in milestone 3; slot and overlap already filter by status |
| Migration pins and the mirror rebuild ripple through many tests | High | Low | Move every pin listed in Evidence in the migration's own commit; run the full insta suite, not a grep |
| The cascade's five levels disagree with the redesign's surfaces | Low | Medium | `TargetSource` names the level; the skeleton's order is the reference; MOD-119/123 read it rather than recompute |
| A `locator` gains a secret through a later kind | Low | High | Schema doc and writer validation refuse known secret-shaped keys; MOD-131's review re-checks |
| Child-box rows accumulate (one per image) and crowd pickers | Medium | Low | `kind = container` boxes are listed under their parent, never in box pickers; retention left to ANA-42 §9's follow-up |
| MOD-44 needs a different child-box shape than milestone 1 guesses | Medium | Medium | Milestone 1 ships the minimum (kind, parent, nullable `os_family` until probe); MOD-44 may add columns |

---
*Status: accepted at the PRD gate (2026-11-14); planning per milestone via /plan.*
