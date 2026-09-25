# ANA-11 - Models for requirements and decisions

> **Scope note:** How `htui` stores product requirements (`R-<AREA>-<N>`) and architectural
> decisions (resolved `ANA`/`MOD`/... items with their index line and write-up) in its own Postgres
> store, so a project managed by `htui` needs no `REQUIREMENTS.md`, `DECISIONS.md` or
> `docs/decisions/**`. Extends the schema of `docs/ANA-9.md` by forward-only migration. Governed by
> `.claude/rules/workflow-docs.md`, `CONCEPTS.md` and `docs/REQUIREMENTS.md`.
>
> **Requirements addressed:** `R-ID-3`, `R-ID-6`, `R-ENT-5`, `R-ENT-8..10`, `R-ENT-12`, `R-NF-4`,
> `R-TUI-9`, `R-STO-3`, `R-PRM-1`, `R-LATER-3`. Proposes new requirements (§7) for maintainer
> decision.
>
> **Status (2026-09-25): concluded.** Implementation spawned as MOD-38 (schema, seam, close-out
> resolution) and MOD-39 (Requirements tab and item traceability in the TUI). MOD-8's importer scope
> is widened to fill the new tables. Requirement amendments in §7 are a maintainer decision and gate
> MOD-38.

---

## 1. Context and problem statement

`R-ID-3` says Postgres holds everything `htui` knows, and `R-TUI-9` says close-out produces no
markdown. Two workflow artefacts still exist only as markdown, because the schema has no place for
them:

1. **Requirements.** `docs/REQUIREMENTS.md` holds 96 distinct `R-<AREA>-<N>` IDs across 15 areas.
   Each has a priority (`must` / `later`) and may be withdrawn or amended by maintainer decision.
   `R-NF-4` requires every `ANA` and `MOD` to cite the IDs it addresses. Today that citation is
   free text: `HANDOFF.md` alone carries 77 `R-` mentions, and `docs/ANA-10.md` carries 305.
   Nothing checks that a cited ID exists, is still live, or has changed since it was cited.
2. **Decisions.** A resolved item appears as a `DECISIONS.md` index line (`ID`, title, status,
   date, write-up link), a write-up file `docs/decisions/<prefix>/<prefix>-N.md`, and, for an
   analysis, the research doc `docs/ANA-N.md`. The index uses a resolution vocabulary: 24 `done`,
   8 `concluded`, 2 `shipped`, 1 `rejected`, plus `withdrawn` and `superseded` in bodies. That
   vocabulary is richer than the item status machine.

`R-LATER-3` (MOD-8) already promises to import exactly these files, but only into projects, items,
documents and links. It has nowhere to put a requirement row or a resolution. This analysis
decides the target shape.

### 1.1 What the store has today (read at HEAD)

| Concern | Current shape | Source |
|---|---|---|
| Work unit | `item`: key `PREFIX-N` generated from `(key_prefix, key_number)`, `status` in 8 values, `version` CAS, `closed_at` | `migrations/0001_init.sql:310` |
| Terminal state | `Status::is_terminal` = `done` or `closed`. `close_out` moves `blocked` / `failed` / `done` to `closed` in one transaction with a `summary` document and commit rows | `htui-core/src/model/item.rs:34-60`, `pg/write.rs:3902` |
| Why it closed | No column. Scattered across the `summary` document, `run.failure`, `run_step.gate_note`, `item_revision.reason` (edits only) | Explore survey |
| Documents | `document(item_id NOT NULL, kind TEXT open set, version)`, append-only. Seeded phase kinds: `research`, `verdict`, `prd`, `plan`, `implement`, `review`, `reproduce`, `fix`, plus `summary` | `0001_init.sql:389`, `seed.rs:230` |
| Links | `item_link` kinds `blocked_by`, `origin`, `relates`, `supersedes`, tombstoned. No write method on any trait yet (MCP and importer only, `R-ENT-9`) | `0001_init.sql:357`, `store/traits.rs` |
| Requirements | **None.** No table, type or column. Only prose references in item bodies | grep over `crates/**` |
| Search | Substring match (`position` / `instr`). No `tsvector`. Qdrant code (MOD-34) indexes markdown files, not rows, and is not wired in | `pg/read.rs:103`, `vector_sync.rs` |
| Migrations | `0001`..`0004` present; the next is `0005` | `crates/htui-store/migrations/` |

---

## 2. Invariants

1. **Postgres is canonical** (`R-ID-3`). Nothing in this design writes, reads or depends on a
   markdown file at runtime. Exporting markdown views stays out of scope (`REQUIREMENTS.md` §14).
2. **Requirement text changes only by maintainer decision.** This is the rule in
   `workflow-docs.md` for `REQUIREMENTS.md`, carried over. No agent, MCP tool, importer re-run or
   close-out edits requirement text. Agents may only *propose citations* (§4.3).
3. **Deterministic bookkeeping** (`R-ID-6`). Key minting, citation parsing on import, and
   suspect-link detection are code, never an LLM.
4. **No silent overwrite** (`R-ENT-10`). Requirement edits use the same `version` compare-and-set
   and revision trail as items.
5. **Nothing generic.** `R-ENT-13` withdrew the ANA-1 atomic-facts table as over-engineered. Every
   new table here has typed columns and CHECKs. There is no attribute-value store.
6. **The status machine is not widened for bookkeeping.** `item.status` belongs to the
   orchestrator (ANA-2 §4.3). How an item was resolved is a separate, orthogonal fact.

---

## 3. Prior art

| System | Requirement model | Decision model | Taken / left |
|---|---|---|---|
| **ReqIF** (OMG exchange format) | `SpecObject` of a `SpecType`, typed attributes, `SpecRelation` of a `RelationType`, hierarchy via `SpecHierarchy` | n/a | *Left:* the fully typed-attribute meta-model is the generic store invariant 5 forbids. *Taken:* typed relation kinds between requirement and work. |
| **Doorstop** | One YAML file per requirement in git, `UID` = prefix + number, `links` to parent UIDs, per-link *fingerprint* stamp, `reviewed` hash | n/a | *Taken:* the link records what it was linked against, so a later edit makes the link **suspect** until someone re-confirms it. |
| **StrictDoc / sphinx-needs** | Text-DSL requirement nodes with UID, STATUS, typed `Parent`/`Child` relations, coverage matrices | sphinx-needs lets any "need type" (spec, test, decision) share one node model | *Left:* one node model for everything means requirements would inherit a lifecycle that doesn't fit them (§4.1 option A). *Taken:* coverage as a query over typed relations. |
| **Polarion / Jama** (commercial ALM) | Versioned work items with baselines, traceability matrix, **suspect links** on upstream change | Change requests as work items | *Taken:* suspect links. *Left:* baselines. `htui` has `requirement_revision`, and a named baseline is a later nicety. |
| **ADR / MADR, adr-tools, log4brains** | n/a | One markdown record per decision, `status` proposed / accepted / deprecated / superseded, a `supersedes` pointer | *Taken:* a decision is a record with a status and a supersede edge. `htui` already has both halves: the item with its `summary` / `verdict` document, and the `supersedes` link. |
| **Jira** | n/a | `status` (workflow) and `resolution` (Fixed / Won't Do / Duplicate / Done) are **separate fields**; resolution is set on the transition into a closed status | *Taken:* `resolution` orthogonal to `status` (§4.2). |
| **OpenFastTrace** | Requirement IDs as tags in markdown and code, trace by scanning | n/a | *Left:* scan-based tracing is the free-text status quo this analysis replaces. |

---

## 4. Options and verdicts

### 4.1 Where requirements live

**Need:** each `R-ID` needs a stable identity, text, priority, a live / withdrawn state, an
amendment history, and many-to-many citation by items.

| Option | Verdict |
|---|---|
| **A. Requirements as items of a `REQ` kind** | Rejected. It reuses minting, revisions and links, but everything else fights it. The key shape `R-ENT-5` puts the area mid-key and `item_kind.prefix` forbids `-`, so it needs either a kind per area or a mangled key. `item.status` (`open`..`closed`) and `priority SMALLINT` mean nothing for a requirement. A requirement would appear in the Backlog, qualify for `ready_items`, and be runnable through a step graph unless every query excludes it, which is a filter every future query must remember. |
| **B. Dedicated tables** (`requirement_area`, `requirement`, `requirement_revision`, `requirement_key_counter`) | **Adopted.** Typed columns and CHECKs, its own key format, no accidental entry into orchestration. It follows the item patterns (per-scope counter per ANA-9 §4.1, `version` CAS per §4.2, append-only revisions) without sharing their rows. |
| C. Generic typed node table (`node(type, attrs JSONB)`, ReqIF-style) | Rejected by invariant 5 and the `R-ENT-13` precedent. |
| D. One project-level `requirements` document, parsed on read | Rejected. No per-ID identity to link to, and a parser on the read path. This is the markdown model again in a column. |
| E. Keep `REQUIREMENTS.md` in the repo and index it | Rejected for `htui`-managed projects: a second source of truth (`R-ID-3`), and `R-ID-4` forbids `htui` writing it back. It stays true for `htui`'s *own* repo until cutover (§6, phase 3). |

### 4.2 Where decisions live

**Need:** an index of resolved items with status and date, a write-up per item, the research doc
for an analysis, and the "replaced by" relationship.

| Option | Verdict |
|---|---|
| **A. A decision is a resolved item, with no new entity** | **Adopted.** Every field of the `DECISIONS.md` index line already exists: key, title, `closed_at`, and the write-up is the `summary` document `close_out` already writes (`R-TUI-9`). An analysis's `docs/ANA-N.md` is its `verdict` (or `research`) phase document. "Superseded by" is the existing `supersedes` link. The index is a query: `item WHERE status = 'closed' ORDER BY closed_at DESC`. |
| B. A separate `decision` table (ADR-style) | Rejected. It duplicates item identity and key space, and would need its own link, revision and cache rows for data the item already has. |
| C. A `decision` document kind | Rejected as a *new* kind. `summary` already has this meaning, and the upstream walk already feeds it into prompts (`upstream_summaries`). |

**The one missing field: resolution.** Every close-out currently lands in `closed`, so
`concluded`, `rejected`, `withdrawn`, `shipped` and `done` are indistinguishable. Options:

| Option | Verdict |
|---|---|
| Widen `item.status` with `rejected`, `withdrawn`, ... | Rejected by invariant 6. It multiplies the ANA-2 §4.3 transition table and every `is_terminal` / readiness predicate for a fact the orchestrator never branches on. |
| Put the resolution in the summary document's title or body | Rejected: not queryable and not validated. It is the markdown model again. |
| **`item.resolution TEXT NULL` with a CHECK, set only by `close_out`** | **Adopted.** Jira's split. The value set is `done`, `concluded`, `rejected`, `withdrawn`, `superseded`, `duplicate`. `shipped` folds into `done`: the corpus uses it for two tooling items with no distinction a query needs. A table CHECK enforces `(status = 'closed') = (resolution IS NOT NULL)`. |

**A consequence that must be fixed with it.** `close_out` accepts only `blocked`, `failed` or
`done` (ANA-2 §4.3, `item.rs:50-63`). An item withdrawn or rejected before it ever ran, like
MOD-17..19 under MOD-25, or an ANA rejected at triage, has no legal path to `closed`. The fix is an
ANA-2 §4.3 amendment. `open` → `closed` becomes legal **only** through `close_out` and **only**
with resolution in `rejected`, `withdrawn`, `superseded` or `duplicate`. `done` and `concluded`
still require the item to have reached `done`. The guard is in `close_out`, not in the generic
`transition`, so the orchestrator still cannot jump `open` → `closed`.

### 4.3 Traceability between items and requirements

| Option | Verdict |
|---|---|
| Parse `R-` IDs out of item bodies on read | Rejected: free text again, and a regex on the read path. It survives only as the **importer's** deterministic one-shot (`R-ID-6`). |
| Extend `item_link` to point at requirements | Rejected. `item_link` has two `item` FKs, and a polymorphic target drops referential integrity. |
| **`item_requirement(item_id, requirement_id, kind, requirement_version, ...)`** | **Adopted.** `kind` is one of `addresses` (the `R-NF-4` citation), `amends` or `withdraws` (the item was the maintainer decision that changed the requirement), or `reserves` (designs shape for a `later` requirement). `requirement_version` is stamped at link time. When `requirement.version` is newer than the stamp, the link is **suspect** (Doorstop / Polarion). The TUI flags it until a human re-confirms by re-stamping. It uses the same tombstone and `proposed_by_step_id` columns as `item_link`, so MCP proposals and importer rows are traceable. |

Suspect state is **derived** (`requirement.version > item_requirement.requirement_version`), never
stored, so it cannot drift.

### 4.4 Non-ID prose in `REQUIREMENTS.md`

"How to read", §12's later-tier preamble, §14 *Out of scope* and §15 *Superseded material* carry
no IDs. They go to `requirement_area.description` (per area) and one `requirement_spec` row per
project: an owner and a preamble markdown, with its own `version` CAS. The amendment log in the file
header ("amended 2026-09-09 by maintainer decision...") is **not** stored as prose. It becomes a
query over `requirement_revision` joined to the item named in `amended_by_item_id`.

---

## 5. Schema (migration `0005_requirements.sql`, forward-only)

```sql
-- One spec header per project: preamble, out-of-scope, superseded-material prose (§4.4).
CREATE TABLE requirement_spec (
    project_id  UUID PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
    owner_id    UUID NOT NULL REFERENCES app_user(id),
    preamble    TEXT NOT NULL DEFAULT '',
    version     INTEGER NOT NULL DEFAULT 1,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE requirement_area (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    code        TEXT NOT NULL CHECK (code ~ '^[A-Z][A-Z0-9]{1,15}$'),   -- 'ENT','STO','NF'
    title       TEXT NOT NULL,                                          -- 'Entity model'
    description TEXT NOT NULL DEFAULT '',
    position    INTEGER NOT NULL DEFAULT 0,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, code)
);

CREATE TABLE requirement_key_counter (           -- ANA-9 §4.1 semantics, one per (project, area)
    area_id     UUID PRIMARY KEY REFERENCES requirement_area(id) ON DELETE CASCADE,
    last_value  INTEGER NOT NULL CHECK (last_value >= 0)
);

CREATE TABLE requirement (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id   UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    area_id      UUID NOT NULL REFERENCES requirement_area(id),
    area_code    TEXT NOT NULL,                  -- copied at mint, like item.key_prefix
    number       INTEGER NOT NULL CHECK (number >= 1),
    key          TEXT GENERATED ALWAYS AS ('R-' || area_code || '-' || number::text) STORED,
    body         TEXT NOT NULL,                  -- one or two sentences, markdown
    rationale    TEXT NOT NULL DEFAULT '',
    priority     TEXT NOT NULL CHECK (priority IN ('must','later')),
    state        TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','withdrawn')),
    version      INTEGER NOT NULL DEFAULT 1,     -- CAS, ANA-9 §4.2
    created_by   UUID NOT NULL REFERENCES app_user(id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, area_code, number)
);
CREATE INDEX idx_requirement_updated_at ON requirement(project_id, updated_at);  -- cache cursor

CREATE TABLE requirement_revision (
    requirement_id     UUID NOT NULL REFERENCES requirement(id) ON DELETE CASCADE,
    version            INTEGER NOT NULL,
    body               TEXT NOT NULL,
    rationale          TEXT NOT NULL,
    priority           TEXT NOT NULL,
    state              TEXT NOT NULL,
    author_id          UUID NOT NULL REFERENCES app_user(id),
    box_id             UUID REFERENCES box(id) ON DELETE SET NULL,
    reason             TEXT NOT NULL,            -- 'created','amended','withdrawn','imported','divergence_resolution'
    amended_by_item_id UUID REFERENCES item(id) ON DELETE SET NULL,   -- the decision that changed it
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (requirement_id, version)
);

CREATE TABLE item_requirement (
    item_id             UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    requirement_id      UUID NOT NULL REFERENCES requirement(id) ON DELETE CASCADE,
    kind                TEXT NOT NULL CHECK (kind IN ('addresses','amends','withdraws','reserves')),
    requirement_version INTEGER NOT NULL,        -- stamp; suspect when requirement.version is newer
    proposed_by_step_id UUID REFERENCES run_step(id) ON DELETE SET NULL,   -- NULL = human or importer
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at          TIMESTAMPTZ,             -- tombstone, as item_link
    PRIMARY KEY (item_id, requirement_id, kind)
);
CREATE INDEX idx_item_requirement_req ON item_requirement(requirement_id) WHERE deleted_at IS NULL;

ALTER TABLE item ADD COLUMN resolution TEXT
    CHECK (resolution IN ('done','concluded','rejected','withdrawn','superseded','duplicate'));
ALTER TABLE item ADD CONSTRAINT item_resolution_iff_closed
    CHECK ((status = 'closed') = (resolution IS NOT NULL));
```

`set_updated_at` triggers attach to `requirement_spec`, `requirement_area`, `requirement` and
`item_requirement`. Cross-project citation is legal: the PK is by UUID, as with `item_link`, so a
workspace can share one requirement set across its projects.

**Existing rows.** `item_resolution_iff_closed` would reject any already-`closed` row. The migration
backfills `resolution = 'done'` for `status = 'closed'` before adding the constraint. The dev store
and the demo fixture are the only populated stores.

### 5.1 Seam (`htui-core` store traits)

- `ReadStore`: `requirement_areas(project)`, `requirements(project, filter)`,
  `requirement(id)`, `requirement_revisions(id)`, `item_requirements(item)` returning each citation
  with a derived `suspect: bool`, and `requirement_coverage(requirement)` returning citing items
  with status and resolution.
- `WriteStore`: `create_requirement_area`, `mint_requirement(area, NewRequirement)` (counter
  `UPDATE ... RETURNING`, as §7.1), `amend_requirement(id, expected_version, patch,
  amended_by_item)` returning `UpdateOutcome`-shaped divergence, `withdraw_requirement(id,
  expected_version, by_item)`, `cite(item, requirement, kind)` and `uncite` (tombstone),
  `reconfirm(item, requirement, kind)` (re-stamp), and `set_requirement_spec`.
- `close_out(item, resolution, summary, commits)`: `resolution` becomes a required argument. The
  `open` → `closed` guard of §4.2 lives here.
- `MemStore`, `PgStore` and the cache implement the reads. There is no offline write, per MOD-25.

### 5.2 Cache (`cache_migrations/0004_requirements.sql`)

Mirror `requirement_area`, `requirement`, `item_requirement` (live rows only; a tombstone becomes a
delete, as with `item_link`) and `item.resolution`. Revisions and the spec header are not mirrored,
consistent with `item_revision`. `R-STO-3`'s content list gains "requirements and citations".

### 5.3 Prompt and MCP touch points (named, not designed here)

- **Prompt:** an item's `addresses` citations are the natural source for a `requirements` section,
  the exact text of each cited `R-ID`, placed between Item and Documents in ANA-5's trim order.
  That is an ANA-5 contract amendment and belongs to MOD-9 or a follow-up, not to MOD-38.
- **MCP:** `R-MCP-2` would gain `requirement_cite` (propose an `addresses` or `reserves`
  citation, scoped like `item_link`). Agents never get `amend_requirement` (invariant 2). This is
  an addition to MOD-11's tool list once the requirement is amended (§7).
- **Search (MOD-34):** requirements are short, high-value embedding targets. MOD-34 should index
  `requirement` rows as payload `type = "requirement"` once they exist, rather than
  `docs/REQUIREMENTS.md` chunks.

---

## 6. Verdict and phasing

**Verdict.** Requirements get **dedicated typed tables**, with suspect-aware citations from items.
Decisions get **no new entity**: a decision is a closed item with a `resolution` and its `summary`
(or `verdict`) document, and `DECISIONS.md` becomes a query. The markdown workflow files become
import sources (MOD-8) and nothing more for any `htui`-managed project.

1. **MOD-38 - Requirements schema, seam and close-out resolution** (from ANA-11).
   - Migration `0005_requirements.sql` per §5, and cache migration per §5.2.
   - Trait methods of §5.1 on `MemStore`, `PgStore` and the cache read path.
   - `close_out` takes a resolution and gains the `open` → `closed` guard; ANA-2 §4.3 is amended by
     this item's write-up.
   - Demo fixture gains a small requirement set with one suspect citation.
   - **Gated on the maintainer applying §7** to `docs/REQUIREMENTS.md`.
2. **MOD-39 - Requirements tab and item traceability** (from ANA-11; blocked on MOD-38).
   - A **Requirements** tab: areas, then requirements, with coverage (citing items, their
     status and resolution), withdrawn rows dimmed, and the revision trail with its deciding item.
   - Item detail shows cited requirements with suspect markers and a re-confirm action.
   - The close-out flow picks a resolution. That flow is MOD-4's Runs-pane two-step confirmation
     (`htui-orch/src/closeout.rs`), so the picker goes into its first confirmation.
   - Requirement create, amend and withdraw are maintainer-only human actions, and amend asks for
     the deciding item.
3. **MOD-8 (widened, still later tier).**
   - Import `REQUIREMENTS.md` into spec, area and requirement rows, with revisions at `reason =
     'imported'`.
   - Take each `DECISIONS.md` status as the resolution (`shipped` becomes `done`).
   - Write-ups become `summary` documents, and `docs/ANA-N.md` becomes the `verdict` document.
   - Scan `R-` IDs in item bodies once into `item_requirement(addresses)`, deterministically,
     skipping unknown IDs with a report.
   - Cut `htui`'s *own* repo over from markdown only after MOD-13 and MOD-39 exist (MOD-4, the
     orchestrator, is done), so the
     tool can run its own lifecycle. Until then the markdown workflow stays authoritative for this
     repo.

---

## 7. Proposed requirement amendments (maintainer decision, not applied here)

`workflow-docs.md` reserves `REQUIREMENTS.md` edits to explicit maintainer decision, so this
analysis proposes the text and does not apply it:

- **R-ENT-14 (must, new).** `requirement`: key `R-<AREA>-<N>` minted per project and area, never
  reused. It has body, rationale, priority `must`/`later`, state `active`/`withdrawn`, and a
  `version` with revisions naming the deciding item. Text changes only by human action.
- **R-ENT-15 (must, new).** Items cite requirements with kind `addresses`, `amends`, `withdraws` or
  `reserves`. Each citation records the requirement version it was made against, and a newer
  version marks it suspect until re-confirmed.
- **R-ENT-8 (amend).** Append: "A closed item carries a resolution: `done`, `concluded`,
  `rejected`, `withdrawn`, `superseded` or `duplicate`. Resolution is not a status."
- **R-NF-4 (amend).** "...cites, as `addresses` citations, the requirement IDs it addresses."
- **R-STO-3 (amend).** Cache contents gain "requirements and citations".
- **R-TUI-1 (amend).** Tabs gain **Requirements**.
- **R-MCP-2 (amend, optional).** Add `requirement_cite`.

---

## 8. Risks

| Risk | Mitigation |
|---|---|
| Requirement sets are small (about 100 rows), so a dedicated table set looks heavy | It is five small tables with no new crate, following existing patterns. The alternative is a `REQ` kind that every orchestration query must filter out forever (§4.1 A). |
| Suspect flags pile up after a broad amendment | Suspect is derived and re-confirm is one keystroke. The Requirements tab lists suspects per requirement for batch re-confirm. |
| The resolution vocabulary is wrong for a future project | The CHECK is a migration away. Six values cover every resolution in this repo's 35-line archive. |
| Opening `open` → `closed` weakens the ANA-2 status law (already amended once, by MOD-4 plan D161's `blocked` → `awaiting_approval`) | Only through `close_out`, only for non-success resolutions, and still refused while a run is live. |
| A dogfooding cutover strands this repo's markdown workflow mid-way | Phase 3 cuts over only after the TUI can run the lifecycle. The markdown files remain the authority for `htui`'s own repo until then. |
