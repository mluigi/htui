# MOD-7 — Box registry and capabilities

> Routed as **PRD** by `/handoff-run MOD-7` (criteria C2, C3, C4 fired). Ultracode recommended for
> the implement and review phases; the maintainer accepted both. No ANA precedes this item: the
> contract is `R-BOX-1..4`, `R-ORCH-10`, `R-AGT-6` and the box-profile clause of `R-TUI-8`, with the
> capability refusal already designed in `docs/ANA-2.md` §4.10 (criterion 14) and the box profile
> projection and excerpt fallback root in `docs/ANA-5.md` §4.2 and §4.5. The decisions at the PRD
> gate below settle what those leave open.

## Problem

`htui` records a box and knows almost nothing about it. First launch writes a `box` row with the
hostname, OS family, architecture and `htui` version, and stops there: `os_version` is the empty
string, `cpu`, `ram_mb`, `gpu_present`, `gpu_vendor`, `probed_tags`, `declared_tags`, `quirks` and
`last_probed_at` are never written by production code, and `box_tool` is written only by the demo
loader. Everything downstream that reads a box therefore reads a stub. The prompt's box section
renders a hostname and `os: linux  (x86_64)`; an item that declares `required_tags` runs on any box,
because nothing enforces `R-ORCH-10`; agents are never probed when a box registers, although
`R-AGT-6` says they are; and every production prompt carries empty excerpts, because the fallback
root ANA-5 §4.5 reads from `repo_box_path` was never wired. Three open items are blocked on this
one (MOD-12 needs real tags and path rows, MOD-41 and MOD-44 need box registration they can build on).

## Evidence

Facts gathered from the tree on 2026-09-25 while routing, each re-checked by the plan fact-check.

- **Registration writes the minimum on purpose.** `PgStore::register_box`
  (`crates/htui-store/src/pg/mod.rs:353-377`) inserts id, user, hostname, `os_family`, `''` for
  `os_version`, arch and `htui_version`; its doc defers "the full probe (`box_tool`, tags, RAM, GPU)"
  to MOD-7 (`:337-342`). Its only caller is `PgStore::bootstrap` (`:418-424`), on the spawned connect
  task, never the UI task.
- **Registration is keyed on the hostname, and a hostname change is a probable crash.** The upsert's
  only conflict target is `(user_id, hostname)` and `box` carries `UNIQUE (user_id, hostname)`
  (`0001_init.sql:73`). `box.toml` keeps its minted id across a hostname change
  (`identity.rs:190`, `a_hostname_change_keeps_the_box_id`), so re-registration after a rename would
  insert an id that already exists — a duplicate primary key with no `ON CONFLICT (id)` arm. Inferred
  from the SQL, not yet reproduced; the plan's first test reproduces it. The same key is ANA-16's C4:
  a cloned VM or container with a duplicate hostname merges into another box's row.
- **A machine identity is readable without root.** `/etc/machine-id` exists and is world-readable on
  this box; `/sys/class/dmi/id/product_uuid` is root-only (`Permission denied`). macOS exposes
  `IOPlatformUUID` and Windows `MachineGuid` equivalently. systemd documents the raw machine-id as
  confidential, so only a keyed hash may leave the box. Cloned images share a machine-id unless it is
  regenerated, and an OS reinstall changes it — so it cannot be the key, only a check.
- **The capability refusal exists in the store and nowhere else.** `Backend::missing_tags`
  (`backend.rs:558`) and `Backend::ready_items` (`:543`, the tag-subset filter) are implemented for
  Postgres and `MemStore` and called only by tests. The engine cannot reach them: they are inherent
  methods, and `GraphSource` (`crates/htui-orch/src/graph.rs:57-101`) reads no box row. `Claim` has
  no tag variant and `RunFailure` no missing-tags case. `item.required_tags` exists
  (`0001_init.sql:322`) and every conformance fixture leaves it empty. MOD-4 proved criterion 14's
  `Unblock` half through a no-candidate refusal instead
  (`crates/htui-orch/src/conformance.rs`, `unblock_opens_a_blocked_item_with_no_run`).
- **`repo_box_path` already has a writer, and HANDOFF says otherwise.** MOD-15's Settings >
  Hierarchy editor writes it through `StoreRequest::SetRepoPath` (`crates/htui/src/hierarchy.rs:303-321`),
  canonicalised by `htui_core::root_path::canonical_root`, which resolves a link and refuses a
  dangling or relative one — F-102 is already honoured for that writer. Nothing writes the rows
  automatically, and a missing row hard-fails isolation in all four modes
  (`crates/htui-orch/src/isolate/real.rs:32-38`, "no checkout for this repo on this box"). Comments
  at `crates/htui-orch/src/engine.rs:4928` and `:5615` still claim no writer exists.
- **Excerpts are empty in every production prompt.** Phase and judge prompts use `no_excerpts`
  (`engine.rs:4311`, `:4931`); the preview uses `empty_excerpts` with no roots
  (`crates/htui/src/preview.rs:266-285`). `RootSource::RepoBoxPath` appears only in tests, and
  `FsRepoReader` has no production caller.
- **Agents are probed on demand, on chat start and after install or login, never on registration.**
  `ProbeAgents` from Settings `r` (`agents.rs:1196`), the lazy chat-start re-probe
  (`agent_worker.rs:1481-1499`, `PROBE_TTL` 24 h), and the install and login paths. Install is
  reachable only from Settings `i`.
- **Reusable probing exists for agent tools only.** `resolve_tool`, `probe_tools`, `capture_version`
  and `VERSION_TIMEOUT` in `crates/htui-agent/src/probe.rs` are scoped to an agent's
  `discovery.tools`. No crate collects OS version, CPU, RAM or GPU; no system-information dependency
  is in any manifest.
- **The Settings tab expects this section and has room for it.** `settings/mod.rs:4` names "MOD-7's
  box profile". Six sections cost 54 of 100 strip columns (`tests/settings.rs:948-965`, whose doc
  comment is stale at "five … 46"). No request reads the full box row or `box_tool`; `BoxInfo` has no
  `quirks`, `os_version`, tools or `updated_at`; no request writes `declared_tags` or `quirks`.
  `TextField` is single-line by design (`crates/htui/src/ui/text_field.rs:6-7`) and nothing in the
  crate edits multi-line text.
- **No box write is a compare-and-set today**, and every reconnect bumps `box.updated_at` through
  `register_box`'s `last_seen_at` update and the `set_updated_at` trigger — so a naive `updated_at`
  CAS on the whole row would go stale on every reconnect. ANA-16 C7 asks that any box-settings writer
  be a CAS.

## Users

- **Primary**: the maintainer running items on this box today. The need fires when an item that
  needs a toolchain runs anyway and fails deep in a step, when a prompt describes the box as a
  hostname and nothing else, or when a run fails with "no checkout for this repo on this box" for a
  repo that is checked out under the workspace root.
- **Also served**: the next box. MOD-44's container child boxes and MOD-45's SSH-provisioned boxes
  register through the path this item fixes; a copied `box.toml` or a cloned hostname must not merge
  them into this one.
- **Also served**: MOD-12 (auto mode filters the queue by tags and needs path rows) and MOD-41
  (the headless worker registers a box without a UI).
- **Not for**: editing box settings or caps (`max_concurrent_items`, `command_limits`) — MOD-12
  owns the caps editor; probing inside containers or over SSH — MOD-44 and MOD-45.

## Hypothesis

We believe **a real box probe, editable declared tags and quirks, a box identity that survives a
rename and refuses a copy, and a tag check at enqueue and at claim** will **make every box fact
`htui` reads true and every capability mismatch a named refusal** for **the maintainer and every box
after the first**.

We'll know we're right when **on this box the Settings box section shows real hardware, a tool list
with versions and derived tags; an item requiring a tag this box lacks is refused at enqueue with
exactly the missing tags, `blocked`, and no `run` row; and after `Unblock` the same item runs on a
box that has the tags** — criterion 14 in full, with the capability half no longer proved by proxy.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Box row completeness | `os_version`, `cpu`, `ram_mb`, `gpu_present`, `last_probed_at` non-default on this box after first launch | Live check on this box; a conformance case over `MemStore` and `PgStore` for the probe writer |
| Tool list | Every `R-BOX-2` tool present on the box appears in `box_tool` with its version; absent ones do not | Unit tests over a fake `PATH`; live check |
| Re-probe triggers | Probe runs at first registration, when `htui_version` changes, and on demand; not on every connect | Tests driving `register_box` twice with the same and a bumped version |
| Rename survives | A hostname change keeps the box id, updates `hostname`, and raises no error | New migration test reproducing today's duplicate-key failure first |
| Copy refused | A `box.toml` carried to a machine with another fingerprint mints a new box, never adopts the old row | Migration test with two fingerprints over one id |
| Fingerprint confidentiality | The raw machine-id never reaches Postgres, a log line or a file | Test asserting the stored value is the keyed hash; grep of tracing fields |
| Capability refusal | Criterion 14 capability half: no `run` row, `item.status = 'blocked'`, note body exactly the missing tags; `Unblock` reopens | New `htui-orch` conformance case; the ANA-2 claim-time case fails the run with `missing tags: …` |
| Declared tags and quirks CAS | A concurrent edit is reported as changed elsewhere, never overwritten; a reconnect does not stale an open editor | Conformance case plus section test with a reconnect between read and write |
| Path inference | Every repo whose checkout sits under the workspace root and matches by remote URL, or else by name, gets a canonical row; ambiguous or absent repos get none | Tests over a temporary tree with two, one and zero candidates, and a symlinked root |
| Manual wins | An existing manual row is never replaced by inference | Test |
| Excerpts non-empty | A phase prompt for an item touching a repo with a path row carries excerpts from that root | `htui-orch` test; preview test |
| UI never blocks | Probe, inference and every write run off the UI task | `R-NF-3`; the existing store-worker pattern, no store handle on the render side |

## Scope

**MVP** — a box identity that is keyed on its id and checked by a fingerprint, a box probe that fills
the row and `box_tool` and derives tags, agent autodiscovery on registration, a Settings box section
with a declared-tag and multi-line quirks editor, the `R-ORCH-10` refusal at enqueue and at claim,
repo path inference with a manual fallback, and excerpts read from those paths.

Concretely in scope:

- **Box identity (D1).** Registration looks the box up by its `box.toml` id, updates `hostname` as a
  display field, and checks a keyed hash of the OS machine identity. A mismatched fingerprint means
  the file was copied: a new id is minted and the old row is left alone. A box with no readable
  machine identity registers by id alone. `UNIQUE (user_id, hostname)` goes. Migration `0005`.
- **Box probe.** OS version, CPU, RAM, GPU presence and vendor; the `R-BOX-2` tool list with
  versions into `box_tool` (compilers `rustc`, `cargo`, `gcc`, `clang`, `cl`; build tools `cmake`,
  `ninja`, `vcpkg`; shells; container runtime); `probed_tags` derived from those facts (D2);
  `last_probed_at`. Runs at first registration, when `htui_version` differs from the stored one, and
  on demand from the Settings section. `last_seen_at` stays per session.
- **Agent autodiscovery on registration (`R-AGT-6`).** The same trigger runs the existing agent
  probe for this box. A `missing` agent whose row declares an install source is surfaced with the
  existing install offer; nothing installs without the MOD-20 consent and licence flow (D7).
- **Settings `box` section (`R-TUI-8`).** Lists every box the user owns; shows profile, tools,
  probed and declared tags, quirks, last probe; re-probe on this box only; declared tags and quirks
  editable on any box (D4), as a compare-and-set that a reconnect cannot stale. Quirks use a new
  small multi-line text widget (D3); declared tags use a comma list over the open vocabulary.
- **Capability refusal (`R-ORCH-10`).** ANA-2 §4.10 as designed: at enqueue, a missing tag blocks
  the item with a note naming exactly the missing tags and writes no `run`; at claim, inside the
  admission transaction, the run fails with `missing tags: a, b`. The engine reaches box tags through
  a seam it does not have today. Criterion 14's capability half gets its own conformance case.
- **Repo path inference (D5).** For every repo of every project in the workspace, look under this
  box's `workspace_box_path` root for its checkout, matching normalised `remote_url` first and the
  directory name `repo.name` second. One match writes a canonical row (F-102: resolve or refuse a
  linked root); zero or several matches write nothing and the section offers a manual path text box.
  A manual row is never replaced by inference.
- **Excerpts from path rows (D6).** Phase and judge prompts and the preview resolve the excerpt root
  through ANA-5 §4.5's fallback (`run_step_tree.path`, then `repo_box_path.local_path`, then
  `no_path`) instead of `no_excerpts`.
- **Record corrections.** The stale "no writer" comments at `engine.rs:4928` and `:5615`, the stale
  strip comment in `tests/settings.rs`, and the HANDOFF text of MOD-7 and MOD-40 (C4 moves here).

**Out of scope**

- **Box settings and caps editing** (`max_concurrent_items`, `command_limits`) — MOD-12's caps
  editor. This item's CAS covers only the fields it writes.
- **Box heartbeat** (`last_seen_at` bumped while running, not only at connect) — stays MOD-40 C4.
- **Probing inside a container or over SSH** — MOD-44, MOD-45.
- **Automatic install** — install stays consented, from Settings, per MOD-20.
- **Interactive path picker** — a popup for choosing a repo path instead of typing it is **MOD-49**,
  opened with this PRD.
- **Auto-mode queue filtering** — `ready_items` already filters by tags; wiring it into a queue is
  MOD-12's.
- **Deleting or merging boxes** from the section.
- **Windows runtime verification** of the probe (`cl`, `MachineGuid`, `where`) — written and
  lint-checked here, verified by MOD-16.

## Decisions taken at the PRD gate

Answered by the maintainer on 2026-09-25, before planning.

- **D0 — MOD-40 is not a dependency.** MOD-7 builds on the store as it is. Its own new box writers are
  compare-and-set, which satisfies ANA-16 C7 for them.
- **D1 — Box id is the key; a machine fingerprint checks it.** The random id in `box.toml` stays the
  identity. Registration matches by id and updates the hostname. A keyed HMAC-SHA256 of the OS
  machine identity (`/etc/machine-id`, `IOPlatformUUID`, `MachineGuid`) is stored beside it; a
  mismatch means a copied `box.toml`, answered by minting a new id. No fingerprint available means id
  alone. The raw identity never leaves the box. This absorbs ANA-16 C4's re-key from MOD-40; MOD-40
  keeps the heartbeat half. MOD-44 and MOD-45 no longer wait on MOD-40 for identity.
- **D2 — Probed tags come from a presence map.** `rustc` or `cargo` gives `rust`; `cmake` gives
  `cmake`; `clang` gives `clang`; `cl` gives `msvc`; a MinGW `gcc` gives `mingw`; `vcpkg` gives
  `vcpkg`; `docker` or `podman` gives `docker`; a detected GPU gives `gpu`; `vulkaninfo` gives
  `vulkan`. `heavy_build` is declared only. The map is data, not a `match` on tool names scattered
  through code.
- **D3 — Quirks get a multi-line widget.** R-BOX-3 calls quirks a free-form note and ANA-5 §4.2 puts
  it into every prompt; a single-line field is the wrong tool. The widget is small and reusable, not a
  general editor.
- **D4 — The section lists every box.** Declared tags and quirks are editable on any box the user
  owns; the probe runs on this box only.
- **D5 — Paths are inferred per repo, with a manual text box when inference fails.** Remote URL
  first, then directory name, under the workspace root; ambiguity is a failure. An interactive popup
  picker is a separate item, MOD-49.
- **D6 — Excerpts are wired here.** The path rows this item produces are what ANA-5 §4.5's fallback
  root reads; replacing `no_excerpts` is in scope.
- **D7 — Registration offers install, never performs it.** A `missing` agent with an install source
  is surfaced with the MOD-20 action; consent and licence rules are unchanged.

## Constraints (fixed before planning)

- **Next migration is `0005`.** Forward-only (`R-STO-5`); `tests/migrations.rs` pins the applied
  list and table count and must be updated; every new `query!` needs `cargo sqlx prepare` against a
  migrated scratch database.
- **`R-NF-3` is enforced by ownership.** Probe, inference and writes run on the store or agent
  worker; no store handle on the render side.
- **The probe is the authority on agent state (`R-AGT-6`)**; a manual `agent_box` row is never
  overwritten by a probe that finds nothing (MOD-2 D45).
- **Every new box writer is a compare-and-set** keyed on a token a reconnect does not bump.
- **F-102**: any new `repo_box_path` writer canonicalises or refuses a root that is a link.
- **No tool or agent name outside data** for the tag map and tool list (`R-AGT-5` spirit).
- **`unsafe_code = "forbid"`, MSRV and the workspace lint set unchanged; TDD per repo convention.**
- **A new system-information dependency is a decision**, justified in the plan, and must build on
  Linux, macOS and Windows.

## Delivery Milestones

<!-- Business outcomes, not engineering tasks. /plan turns each into a plan. -->
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | A box knows itself | Registration keys on the id and survives a rename, a copied `box.toml` mints a new box, and the probe fills hardware, `box_tool` and `probed_tags` at registration, on `htui` version change and on demand; agents are probed on registration and missing ones offered for install. No UI yet beyond what exists. | in-progress | [plan](../plans/mod-7-box-identity-probe.plan.md) |
| 2 | The maintainer sees and edits it | The Settings `box` section lists boxes with profile, tools and tags; re-probe on this box; declared tags and multi-line quirks edited as a compare-and-set a reconnect cannot stale. | pending | — |
| 3 | A mismatch is refused by name | `R-ORCH-10` at enqueue and at claim, the engine's box-tag seam, criterion 14's capability half as its own conformance case. | pending | — |
| 4 | Paths and excerpts are real | Repo paths inferred per project under the workspace root with a manual text box on failure, and phase, judge and preview prompts carry excerpts read from those roots. | pending | — |

Milestones 1 and 3 are testable without UI. Milestone 3 depends only on tags existing, so it can run
beside milestone 2. Milestone 4 needs no probe and could run first; its order here follows value.

## Open Questions

- [x] Is MOD-40 needed? — No (D0).
- [x] Hostname-change failure: fix here or in MOD-40? — Here, as a fingerprint-checked id key (D1).
- [x] Tag derivation — presence map (D2).
- [x] Quirks editor — multi-line widget (D3).
- [x] Section scope — all boxes (D4).
- [x] Missing path rows — inference plus manual box, picker as MOD-49 (D5).
- [x] Excerpts — in scope (D6).
- [x] Install from registration — offer only (D7).
- [ ] Which crate, if any, reads CPU, RAM and GPU across the three platforms — plan's call, justified.
- [ ] How GPU presence and vendor are detected without root on Linux (`/sys/class/drm`, `lspci`,
  `nvidia-smi`) — plan verifies on this box.
- [ ] Which token the declared-tags and quirks CAS compares, given reconnects bump `updated_at` — plan's call.
- [ ] How far below the workspace root inference searches, and how `remote_url` is normalised
  (`https` vs `ssh`, trailing `.git`) — plan's call, tested.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Dropping `UNIQUE (user_id, hostname)` lets two live rows share a hostname and confuses a hostname-keyed display | Medium | Low | Hostname becomes display only; the section shows the id suffix on collision |
| A fingerprint that changes legitimately (OS reinstall, regenerated machine-id) forks the box and orphans its history | Low | Medium | Only a mismatch **with a stored fingerprint** mints; the event is logged and the old row stays visible in the section |
| Raw machine-id leaks through a log or the database | Low | High | Keyed hash computed before any write; `Debug` of the identity type prints nothing; test |
| A probe that shells out to many tools slows startup | Medium | Medium | Probe runs off the UI task, bounded per tool by the existing `VERSION_TIMEOUT`, only on first registration and version change |
| Tag check at claim races a re-probe | Low | Medium | ANA-2 §4.10 puts the check inside the admission transaction; the conformance case covers the claim path |
| Inference writes a wrong path (a fork or a second clone of the same remote) | Medium | Medium | Several matches count as failure; manual rows always win; path shown in the section |
| A reconnect bumps `box.updated_at` and stales every open editor | High | Medium | CAS keyed on a token only this item's writers change; section test with a reconnect in between |
| Wiring excerpts changes every production prompt digest | High | Low | Expected; digests are recorded per step, and the preview shows the change before a run |
| New system-information dependency enlarges the build | Medium | Low | Plan justifies it or reads the few facts directly per platform |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
