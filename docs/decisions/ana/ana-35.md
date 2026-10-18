# ANA-35 - Entity model for any kind of project (concluded, 2026-10-18)

Full analysis: `docs/ANA-35.md` (surface as read, prior art, options, schema sketch, migration order,
what was not verified). Origin: the 2026-10-15 requirements review,
`docs/requirements-review-2026-10-15.md`. Addresses `R-ID-1`, `R-ENT-1..9`, `R-ENT-12`, `R-ENT-14`,
`R-ENT-16`, `R-BOX-4`, `R-STO-8`, `R-TUI-2`; touches `R-ID-7`, `R-STO-3`, `R-ORCH-8`, `R-TUI-11` and
`R-MCP-1..2` at their seams.

## Verdict

The maintainer accepted all twenty answers of `docs/ANA-35.md` §8 ("all as recommended"), Q20
included: the §10 requirement edits are applied as written.

- **Three stored statuses (Q1, Q6).** `item.status` is `open`, `blocked` or `closed`; queued,
  running, needs you, failed and done are a display state derived from the item's latest graph run,
  cancelled runs skipped, by one pure function (`ItemState`). `blocked` always carries a note. One live
  graph run per item becomes a check in `create_run` backed by a partial unique index on `run`.
- **The migration closes nothing a run can explain (Q2, Q3).** A pre-flight count runs first. The five
  removed values become `open`, so a finished item shows "done" and waits for a person's close-out;
  a `done` row with no finished run is closed `done` with a note, and a `failed` row with no failed run
  becomes `blocked` with a note.
- **Release and close-out law (Q4, Q5).** A blocker releases its dependents when closed (any
  resolution) or finished (latest graph run `done` and, once MOD-133 exists, its work landed).
  `done`/`concluded` need a finished latest run; the other four resolutions close an open or blocked
  item; `default_for` reads the run's own `graph_snapshot`. `done → open` goes: rework is a new run.
- **Repo kinds (Q7, Q8, Q9).** A repo is `git` or `folder`, checked at registration. Isolation resolves
  per repo: a folder under `worktree` is copied, `shared_serialized` over a folder refuses at snapshot,
  `copy` copies back unchanged files and parks on conflict. A folder never gets a worktree, merge,
  push, clone or driven target. `needs_repo` is an explicit phase flag, seeded true for software
  `reproduce`, `implement`, `fix`, `review`.
- **Primary repo (Q10, MOD-106's part).** Exactly one primary when any live repo exists, a folder
  allowed; enforced at the store write with a deferred constraint trigger behind it; removal is
  archiving with promotion; names unique among live repos only.
- **Project types (Q11).** A `project_type` table; built-ins software, research, docs, ops, general and
  the internal workspace type; seeds copied once and never re-applied; prefixes RES, Q, DOC, OPS, INC,
  TASK, WS; user-defined types arrive with ANA-33's editor; a `coding` flag for ANA-39 and MOD-128.
- **Workspace scope row (Q12, Q13, Q14).** One hidden `project` row per workspace (type `workspace`,
  slug `@<workspace-slug>`) owns the workspace's items, kinds, counter, graphs, documents, requirements
  and runs, so every per-project path serves it. Counters are per owner and prefix; keys are qualified
  `slug:KEY` where ambiguous. The workspace folder (default `~/workspaces/<slug>`) holds only
  generated files and URL clones; goals are a workspace document; a lone project has no folder.
- **Workspace requirement keys (Q15).** `W-<AREA>-<N>`; `.n` nesting only within one owner, a child
  storing `child_number` and a NULL `number`; stored keys unique per owner; no shared `R-n`.
- **Owner documents and search (Q16).** `project_id` on every document, `item_id` nullable; owner
  documents are written by a person or an accepted draft, MCP stays item-only; item-less documents
  index as their own points, collection `htui_concepts_v4`.
- **Attachments (Q17).** Owned by an item or a document version; metadata and `bytea` blob apart,
  `project_id` pinned by composite keys; 10 MiB default, 64 MiB ceiling; text scrubbed and fail-closed;
  a binary only from a person, stored as given (`R-ID-7` amended).
- **Links (Q18).** `parent` single, acyclic, depth at most 8; close-out records `duplicates` and
  `supersedes` from the person's choice; `implements` as defined; document and attachment targets in
  `item_artifact_link`, a document target pinning a version.
- **Spawn order (Q19).** The project-type table is its own small spawn so the workspace scope waits on
  nothing else; MOD-136 gains the workspace scope as a blocker; MOD-114 (3) waits on it and on links.

## Spawned items

| Filed | From | Subject | Blocked on |
|---|---|---|---|
| MOD-144 | P1 | Item status reduced to open, blocked, closed | none |
| MOD-145 | P2 | Plain-folder repos and repo-less runs | none (lands with MOD-106) |
| MOD-146 | P3 | Project type table | none |
| MOD-147 | P4 | Workspace scope: items, kinds, documents, folder | MOD-146 |
| MOD-148 | P5 | Per-type seeds and the type picker | MOD-145, MOD-146 |
| MOD-149 | P6 | Workspace requirements and nested keys | MOD-147 |
| MOD-150 | P7 | Link kinds and links to documents and attachments | MOD-144 |

Effects on open items (`docs/ANA-35.md` §9): MOD-106 gains archiving, the deferred trigger, the
backfill and the live-name partial unique, and lands with MOD-145; MOD-114 (3) waits on MOD-147 and
MOD-150 with §7.8 as its specification; MOD-135 builds on MOD-144's close-out law; MOD-136 gains
MOD-147 as a blocker (its workspace runs belong to the scope row); MOD-116 needs MOD-147 for the folder
and maps its new-project sources to `vcs`; MOD-118 waits on MOD-149 instead of ANA-35, its nesting
example now `R-TUI-2` → `R-TUI-2.1`; MOD-112 (4) waits on MOD-147; MOD-119 consumes `ItemState`; MOD-8
follows MOD-144, MOD-147 and MOD-150 with the §9 mapping; MOD-133 reads `repo.vcs` and extends
readiness with the landed check; ANA-33 owns the recipe format for user-defined types; ANA-39 and
MOD-128 read `project_type.coding`; ANA-42 inherits "a folder runs on this box only". The findings
doc's `W-n` (`docs/ui-redesign-findings-2026-10-15.md` §4.1 and §8 Q3) points at §7.7.

## Requirement edits applied

Applied by explicit maintainer decision (Q20), not as close-out routine, in the conclusion commit;
`docs/REQUIREMENTS.md` header entry of 2026-10-18: `R-ID-7` (binary attachment exception), `R-ENT-2` (what the workspace folder holds; a lone
project has none), `R-ENT-3` (types copied once, primary may be a folder, removal archives),
`R-ENT-4` (folder refusals and copy-back), `R-ENT-5` (unique per owner), `R-ENT-6` (seed table for
every type), `R-ENT-7` (per-owner counters), `R-ENT-8` (ANA-34 §10.4's transition sentence plus the
derived state, release rule and close-out law), `R-ENT-9` (single parent, version-pinned document
links, close-out records `duplicate`/`superseded`), `R-ENT-12` (attachment owners, cap, scrubbing),
`R-ENT-14` (`W-<AREA>-<N>`), `R-ENT-16` (repo-less workspace items), `R-STO-3` (workspaces and
attachment metadata in the cache), `R-ORCH-8` (default isolation per type, folders), `R-TUI-2`
(filters by state), `R-TUI-11` (blocked items). `CONCEPTS.md` gained §10.17's two clauses. One
reconciliation: `R-ENT-9`'s older "there is no manual link action" clause was dropped, since §10.8's
added sentence restates it with the close-out exception. ANA-34's other requirement edits stay owed.

## Commits

`0106ab7d` (draft), and the conclusion commit that adds this file.

## Not verified

See `docs/ANA-35.md` §11: the auto-generated constraint names to drop; live row counts per item
status and duplicate live runs (no database from the sandbox; MOD-144's plan runs the count first);
`DROP EXPRESSION` and deferrable constraint triggers against the vendored sqlx; whether project slugs
already forbid `@`; how `GixIsolator::prepare` combines trees of different modes; the owner-document
point payload in Qdrant; `requirement.area_code` against the key-shape CHECK; repo-less behaviour of
`reset`, `release` and recovery; copy limits for non-code folders; ANA-42's §8 still open; `.sqlx` and
snapshot counts are estimates; a few prior-art rows and the 10/64 MiB figures are judgement, not
sourced.
