# MOD-15 — Workspace, project, repo and kind management

> Routed as **PRD** by `/handoff-run MOD-15` (criteria C2, C3, C4 fired). Ultracode recommended for
> the implement and review phases. No single ANA precedes this item: its contract is assembled from
> `docs/ANA-9.md` §5.10/§7.1/§10 (the seed, the mint, the prefix warning), `docs/ANA-5.md`
> §5.3/§5.4/§9 (ten templates, two reserved names, ten settings keys), `docs/ANA-2.md` §4.1/§4.7
> (seed graphs, typed settings) and `docs/REQUIREMENTS.md` (`R-ENT-1..4`, `R-ENT-6`, `R-BOX-4`,
> `R-TUI-8`, `R-STO-1`). Where those four disagree, the decisions at the PRD gate below settle it.

## Problem

`htui` can read a hierarchy it cannot create. `0001_init.sql` builds every table the entity model
needs — `workspace`, `workspace_project`, `workspace_box_path`, `project`, `repo`, `repo_box_path`,
`item_kind`, `step_graph`, `step_graph_phase`, `prompt_template`, `app_setting` — and the model
layer (`model/hierarchy.rs`, `model/kind.rs`) types all of them. **Not one has a write path in
product code.** The only rows that have ever existed were written by the demo loader
(`pg/demo.rs:99-338`) and by fixtures. A maintainer pointing `htui` at a fresh database gets a
schema with nothing in it and no way, inside the app, to put anything there.

Several of them cannot even be *read*: `repo`, `repo_box_path` and `workspace_box_path` have no
reader anywhere in the tree, and `step_graph`/`step_graph_phase` are held by `MemStore`
(`mem.rs:62-65`) and exposed by nothing. `R-BOX-4` — "each box holds per-box paths for every repo
and workspace root it has checked out" — is a table and a struct with no code on either side of it.

The gap reaches past this item. MOD-2 shipped the read half of the ten prompt settings and left the
write half undone by name: `token_budget` (default 120 000) and nine siblings are resolvable
through four rungs but changeable only by SQL against the database. MOD-9's template editor, MOD-4's
step graphs and MOD-12's caps each assume a project that was seeded correctly, and nothing seeds
one. And since MOD-25 made `htui` online-only, a box with no DSN is not a degraded box — it is a
box that cannot work at all, which today parks permanently offline with a message naming a CLI flag
(`connect.rs:41`, `:189-208`).

## Evidence

Every fact was read out of this tree on 2026-09-16 during PRD research. Nothing is recalled.

- **Zero write paths, confirmed per entity.** `WriteStore` (`store/traits.rs:135-295`) carries
  eleven methods, all about items, events, agents and chat runs. Its own trailing comment is
  `// links, notes, templates, box ...` (`:294`). Grep for an `UPDATE project` anywhere in
  `crates/htui-store/src` returns nothing.
- **`MemStore::set_app_setting` exists and documents its own absence from the product**
  (`mem.rs:377`): *"**Tests only** … edited nowhere in the product (MOD-15 owns any editor)"*.
- **The ten settings rows are already in the database and pinned.** `0002_agent_probe.sql:68-79`
  inserts them `ON CONFLICT DO NOTHING`, and `prompt/settings.rs`'s `DEFAULTS` is asserted verbatim
  against that migration from both sides (`settings.rs` `the_defaults_are_migration_0002s_ten_rows_verbatim`;
  `htui-store/tests/migrations.rs:268-290`). This item changes values, never the key set.
- **The read half fails silently on a bad value.** `positive_i64` (`settings.rs:198-200`) treats
  absent, null, non-numeric and non-positive identically — all fall through to the next rung. A
  malformed write is therefore invisible: the editor would show one number and the prompt would use
  another. Every clamp lives in the reader (`resolve_hops` `1..=2`, `resolve_reserve_bp`
  `0..=5_000`, `head_lines` clamped to `file_line_cap`).
- **`app_setting` has no version column.** `key TEXT PRIMARY KEY, value JSONB NOT NULL, updated_at
  TIMESTAMPTZ` (`0001_init.sql:557`), with a `BEFORE UPDATE` trigger whose migration comment states
  *"no write path may set updated_at by hand"* because the cache cursor rides on it. Optimistic
  concurrency has to use what is there.
- **`project.settings` is one JSONB blob shared by four items.** ANA-2 §4.7 defines eleven fields
  (`docs/ANA-2.md:1126-1138`), ANA-5 adds `upstream_hops` (`docs/ANA-5.md:1597-1599`), and MOD-4 and
  MOD-12 own most of them. A typed struct round-trip would erase every key the writing item does not
  know about — silently, and months later.
- **The per-project seed is 35 rows in a fixed order.** ANA-9 §5.10 (`docs/ANA-9.md:820-824`):
  5 `step_graph`, 15 `step_graph_phase`, 5 `item_kind` — kinds last, because `default_graph_id` is
  `NOT NULL` (`:501`) — and, per ANA-5 §5.3 (`:1242-1246`), **ten** `prompt_template` rows rather
  than ANA-9's eight. `item_key_counter` is deliberately **not** seeded: the mint's
  `INSERT … ON CONFLICT` creates it on first use (`docs/ANA-9.md:123`).
- **The reserved-name refusal is assigned to this item verbatim.** `docs/ANA-5.md:1238`:
  *"**MOD-15's kind and graph editor refuses a phase named `judge` or `handoff`**"*. The reason is
  structural: `prompt_template` is `UNIQUE (project_id, name, version)`, so a project cannot hold
  both a `judge` phase template and a `judge` judge template.
- **A prefix rename is not a renumbering, and the store forbids making it one.**
  `docs/ANA-9.md:141-145`: *"`key_prefix` is copied from the kind at mint time and never rewritten"*;
  `:151-152`: *"no `DELETE FROM item` … No `UPDATE` of `key_prefix`/`key_number`. Counters are never
  decremented."* The risk table (`:1031`) says only *"The TUI warns on prefix change"* — the text is
  unwritten.
- **No migration is required.** Every table this item writes exists in `0001_init.sql`; the ten
  settings rows exist in `0002`. MOD-4's `0003_orchestration.sql` is still the next migration and is
  not raced. `step_graph.is_override` arrives with `0003`, so this item relies on the column default
  rather than writing it.
- **A missing DSN is a dead end, not a retry loop.** `connect::start` reads the DSN once
  (`connect.rs:179-182`); with none, `connecting = false`, `reconnect: None` (`:189`, `:198-208`) —
  no 30-second ticker is ever installed. The cache file is keyed on
  `identity::db_fingerprint(dsn)`, or the literal `"offline"` when there is none (`:184-187`), so
  changing the DSN changes which mirror file is correct.
- **The settings surface is inherently online-only, and already refuses correctly.**
  `app_setting` is absent from `MIRRORED_TABLES` (`cache/mod.rs:41-58`, 16 tables), and
  `Backend::app_settings()` answers `Offline` with `PROMPT_ON_SERVER_ONLY`
  (`backend.rs:394-400`). No new refusal vocabulary is needed for MOD-25's online-only rule.
- **The Settings tab is a registry with exactly one section, and knows what is missing.**
  `SettingsSection` is a seven-method trait (`settings/mod.rs:42-58`) mirroring `Tab` one level
  down; `SettingsRegistry` (`:62-65`) renders the strip in registration order; `app/mod.rs:47-49`
  registers `AgentsSection` alone. The empty-registry placeholder reads *"Workspace, project, repo
  and kind management arrives with MOD-15."* (`mod.rs:235`).
- **A text-input widget exists, in the wrong place and the wrong shape.**
  `chat/composer.rs:29-32` is `{ text: String, active: bool }` with Char/Backspace/Enter/Esc
  (`:59-86`) and renders `> text_` (`:89-100`) — no cursor position, no insertion point, no
  masking, single line. Nothing under `settings/` or `ui/overlay/` captures characters.
  `TabAction` is `Next | Prev | Select(usize) | Focus(TabId)` (`app/action.rs:50-60`);
  `FocusSection`, `MaskedField`, `captures_input` and `SectionId("connection")` have **zero
  occurrences** in `crates/`.
- **`Settings > Rebuild cache` does not exist.** `CacheStore::rebuild()` (`cache/mod.rs:176-195`)
  is real and documents itself as *"This is `Settings > Rebuild cache`"*, but its only callers are
  tests. There is no `StoreRequest` variant, no key binding, no section and no confirmation
  overlay — and therefore no confirmation copy to correct. `HANDOFF.md:289` describes it as landed;
  it is not.
- **The DSN already has its keyring home.** `secret.rs:3-7` binds `("htui", "postgres-dsn")` with
  `get_dsn`/`set_dsn`/`clear_dsn` (`:29`, `:38`, `:47`) and states *"**nowhere else**: there is no
  env-var fallback (`R-STO-1`)"*. `--set-dsn` reads one stdin line and prints *"paste the DSN and
  press Enter (it will be visible):"* (`lib.rs:117-130`). This item adds a second front end to that
  same store, not a second store.
- **Conformance is the real cost of a seam method.** `conformance.rs` holds `CASES` (23 names,
  `:24-48`) and `READ_CASES` (6, `:126-133`), pinned by `EXPECTED_CASES = 23`
  (`pg_conformance.rs:19`) and twinned in `mem_store.rs:36,42`; the Postgres runner creates and
  drops one database per case (`pg_conformance.rs:31-42`). Any new `WriteStore` method lands on
  four implementations — `PgStore`, `MemStore`, `Writer`, `BufferedWriter`.

## Users

- **Primary**: the maintainer standing up `htui` against a database that is not the demo loader's.
  Today that is a schema with no rows, an app with no create action, and a DSN flag in a README.
  The need fires on first launch and never again — which is exactly why it has gone unbuilt while
  every other item assumed its output.
- **Also served**: the maintainer tuning a prompt. `token_budget` and its nine siblings are
  resolvable through four rungs and editable only by SQL; the phase rung
  (`step_graph_phase.token_budget`) has never been written by anything but the demo loader.
- **Also served, by name**: MOD-9 (template editor over the ten rows this item seeds), MOD-4 (step
  graphs and the `ResolvedPhase` budget), MOD-12 (the same `app_setting` writer for its caps),
  MOD-13 (items minted into kinds this item creates), MOD-7 (the second writer of `repo_box_path`),
  MOD-22 and MOD-23 (the text-input widget the Settings tab does not have).
- **Not for**: item creation or editing — MOD-13. Not the template *editor* — MOD-9 owns new
  versions; this item seeds version 1. Not the orchestration caps or scheduler window — MOD-12.
  Not box capability tags or the quirks note — MOD-7.

## Hypothesis

We believe **create and edit paths for the hierarchy, a correct per-project seed, and a typed
settings writer that validates what it stores**, will **turn `htui` from an app that reads a
hierarchy into one that owns it** for **a maintainer starting from an empty database**.

We'll know we're right when **a fresh Postgres, reached by a DSN typed into the app rather than
passed on a command line, ends up holding a workspace, a project with its 35 seeded rows, a primary
repo with this box's path, and a changed `token_budget` — with no SQL, no restart, and no `--set-dsn`.**

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Nothing needs SQL | Every entity this item owns is creatable and editable from the app | An end-to-end test that seeds nothing and drives the store seam only |
| The seed is exact | 5 `step_graph` + 15 `step_graph_phase` + 5 `item_kind` + 10 `prompt_template` per project, in an order the `NOT NULL` `default_graph_id` accepts | A conformance case asserting row counts and names on both stores; `R-ENT-6` |
| Stored equals effective | No value can be written that the read half would ignore or clamp | Property test: every value the writer accepts survives its resolver unchanged; `R-PRM-3` |
| The reserved names are refused | A phase named `judge` or `handoff` cannot be created or renamed into | `docs/ANA-5.md:1238`; a store test and an editor test |
| `project.settings` loses nothing | Writing one key leaves every unknown key byte-identical | A test writing `upstream_hops` into a blob carrying MOD-4's orchestration keys |
| Concurrent edits are rejected, not merged | A stale write refuses and reports; nothing is last-writer-wins | `R-ENT-10`; a conformance case over a compare-and-set miss |
| A prefix rename keeps history | Existing keys keep their text, the old counter survives, the new prefix mints from 1 | `docs/ANA-9.md:141-145`; a store test over the mint |
| A referenced kind cannot be deleted | Delete refuses and names what references it | `item.kind_id` FK; a store test |
| A destructive delete is never one keystroke | Two confirmations, the second typed, and the counts shown match what the cascade removes | A test asserting the counted rows equal the rows the delete actually takes; `R-TUI-8` |
| A deleted project leaves no ghost | The Backlog stops showing it without the user knowing to rebuild | A test over the mirror after a delete |
| The DSN never lands in a file | Typed input reaches the keyring only; never echoed, logged, or persisted elsewhere | `R-STO-1`, `R-TUI-8`; a test asserting no frame or record carries the string |
| First launch is recoverable | A box with no DSN opens on the connection section and can reach `Online` without a restart | `R-STO-1`, `R-TUI-8`; a live test from an empty keyring |
| No new migration | `0003_orchestration.sql` is still MOD-4's and still next | `htui-store/tests/migrations.rs` table pin unchanged |
| UI never blocks | Every create, edit and connection attempt is served off the UI task | `R-NF-3`; the `Served::Deferred` pattern |

## Scope

**MVP** — the store seam for the hierarchy and the three settings rungs, a correct per-project
seed, a reusable text field, three new Settings sections (hierarchy; kinds and graphs; connection),
and a first launch that can recover from having no DSN.

Concretely in scope:

- **Readers and writers on the store seam** for `workspace`, `workspace_project`,
  `workspace_box_path`, `project`, `repo`, `repo_box_path`, `item_kind`, `step_graph`,
  `step_graph_phase` and the three settings rungs — on `MemStore` and `PgStore`, with conformance
  cases. Several of these need the *reader* built too (`repo`, both path tables, graphs and phases).
  Delete belongs to the seam for `workspace`, `project` (cascading, D13) and `item_kind` (refused
  when referenced, D6); items, runs and documents are never deleted individually.
- **The per-project seed** (D4, D5, D6): 5 graphs, 15 phases, 5 kinds, 10 templates, with ANA-2
  §4.1's amendments applied at seed time and `is_override` left to its column default.
- **A typed settings registry and a rung-aware writer** (D7): `SettingSpec` per key, validation at
  write against the reader's own rules, `set`/`clear` as distinct operations, key-level JSONB merge
  for `project.settings`, and compare-and-set on `updated_at`.
- **A reusable text field widget** (D1) with cursor, insertion, and an optional mask — used by every
  field here and named as the thing MOD-22 and MOD-23 are waiting for.
- **`SectionId("hierarchy")`**: workspaces, projects, repos with the primary flag, and this box's
  paths for each repo and workspace root (`R-BOX-4`), canonicalising or refusing a root that is
  itself a symlink (D11, MOD-2's F-102). Deleting a workspace or project is offered behind two
  confirmations, the second typed, with the cascade's true reach counted on screen and the mirror
  rebuilt afterwards (D13).
- **`SectionId("kinds")`**: the item-kind editor with the prefix-change warning (D12), delete
  refused when referenced (D10), and the step-graph view — phases editable for name, position,
  `template_name`, `token_budget`, `gate_hard` and `input_kinds`, the rest rendered read-only until
  MOD-4 gives them meaning (D2).
- **`SectionId("connection")`** (D8, D9): whether a DSN is stored, a masked field to enter or
  replace it, an action to clear it, the state of the last attempt, and the `Rebuild cache` action
  that does not exist yet — with confirmation copy naming what survives (the file,
  `schema_version`, `db_fingerprint`, `built_at`) and what goes (the 16 `MIRRORED_TABLES`,
  `cache_cursor`, `last_full_refresh_at`).
- **The ten `app_setting` keys exposed in the app**, driven off the registry rather than a
  hard-coded list, with provenance shown (which rung answered) since `trim_record.budget_source`
  already records it.
- **`TabAction::FocusSection`** and `SettingsSection::captures_input` under ANA-10 M0's names, since
  a section that owns a text field must be able to hold keys the tab strip would otherwise consume.
- **Documentation**: README gains the in-app DSN path beside `--set-dsn`; `HANDOFF.md`'s false
  claim that `Settings > Rebuild cache` already exists is corrected at close-out.

**Out of scope**

- **Item creation, editing and the divergence view** — MOD-13 (`R-ENT-5`, `R-ENT-10..12`). This item
  creates the kinds items are minted into; it mints nothing.
- **New `prompt_template` versions and the template editor** — MOD-9 (`R-SKL-*`, `R-PRM-4`). This
  item writes version 1 of ten rows and never a second version.
- **Orchestration caps, concurrency and the scheduler window** — MOD-12, which reuses this item's
  settings writer for its own keys (D7).
- **Phase columns MOD-4 owns** — `fan_out`, `isolation`, `command_queue`, `verify_command`,
  `retry_limit`, `phase_agent` (D2). Rendered, not edited: nothing consumes them yet, and an editor
  for an unread column is a guess at semantics that has not been decided.
- **`step_graph.is_override` and item-level graph overrides** — the column arrives with MOD-4's
  `0003` (`docs/ANA-2.md:1869`); seeded graphs take the column default.
- **Box capability tags, the quirks note and box registration** — MOD-7, which is `repo_box_path`'s
  second writer and reuses this item's symlink guard.
- **Any schema change.** If a decision seems to need one, it is the wrong decision — `0003` is
  MOD-4's.
- **Offline anything.** Since MOD-25 `htui` is online-only; `app_setting` is not mirrored and
  `Backend::app_settings()` already refuses `Offline`. Every path here is server-backed, and none
  of them grows an offline variant.

## Constraints (fixed before planning)

- **No new migration.** Every table exists; `0003_orchestration.sql` stays MOD-4's and stays next.
- **The seed order is fixed by the schema**: graphs → phases → kinds (`default_graph_id NOT NULL`),
  templates independent. `item_key_counter` is never seeded.
- **`updated_at` is never set by hand** on any of the twenty triggered tables (`0001_init.sql`'s
  trigger comment) — the cache cursor rides on it.
- **`project.settings` is merged at key level, never serialized whole.**
- **Nothing here is last-writer-wins** (`R-ENT-10`).
- **The DSN is a credential**: keyring only, never echoed, never logged, never written to a file,
  never on a frame that outlives the request (`R-STO-1`, `R-TUI-8`, `R-SEC-2`).
- **`R-NF-3` is enforced by ownership**: sections hold no store handle; every request is served off
  the UI task, as `AgentsSection` already does.
- **`unsafe_code = "forbid"`, MSRV 1.98, workspace lint set unchanged; TDD per repo convention.**
- **Windows runtime facts defer to MOD-16**, which already owns MOD-2's, MOD-20's and MOD-21's.
  The path-handling in `repo_box_path` and the keyring backend are its newest entries.

## Delivery Milestones

<!-- Business outcomes, not engineering tasks. /plan turns each into a plan. -->
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | The seam can write the hierarchy | Every entity this item owns is readable and writable through `WriteStore` on both stores, including the three settings rungs with validation, key-level merge and compare-and-set. No UI. | complete | [plan](../plans/mod-15-hierarchy-seam.plan.md) |
| 2 | A created project is a working project | Creating a project seeds 5 graphs, 15 phases, 5 kinds and 10 templates in an order the schema accepts, with ANA-2 §4.1's amendments applied and the reserved names refused. A prefix rename keeps history. | complete | [plan](../plans/mod-15-project-seed.plan.md) |
| 3 | The app can take typed input | A reusable text field with cursor, insertion and optional masking, and the hierarchy section built on it: workspaces, projects, repos with the primary flag, and this box's paths — a symlinked root canonicalised or refused. Delete is reachable but twice-guarded, and says what it takes. | complete | [plan](../plans/mod-15-hierarchy-section.plan.md) |
| 4 | Kinds and graphs are editable | The kinds section: kind create/edit with the prefix warning, delete refused when referenced, and the step-graph view with the six editable phase columns and the rest read-only. | pending | — |
| 5 | The prompt is tunable from the app | The ten settings, driven off the registry, editable at whichever rung applies, showing which rung answered, and refusing any value the reader would ignore. | pending | — |
| 6 | A box with no DSN can fix itself | First launch with an empty keyring opens on the connection section; a typed DSN reaches the keyring, re-opens the mirror under its fingerprint, installs the reconnect, and reaches `Online` without a restart. Clear and Rebuild cache work and say what they do. | pending | — |

Milestones 1 and 2 are testable with no UI at all. Milestone 3 is the first that needs the Settings
tab. Milestone 6 is the riskiest and the one that closes `R-STO-1`'s in-app half; it is last because
it needs milestone 3's widget, not because it matters least.

## Decisions taken at the PRD gate

Answered by the maintainer on 2026-09-16, before planning. Each rests on a fact under Evidence.
D8 and D9 were reframed by the maintainer from the research's own proposal; D10's rule is stricter
than what was offered.

- **D1 — A reusable text field is built here, not a third one-off.** `chat/composer.rs` is
  single-line with no cursor and no mask, and Settings has nothing. MOD-22 (paste-back URL) and
  MOD-23 (registry editing) both wait on the same widget, so it lands as a shared widget with an
  optional mask and is named in both items.
- **D2 — The graph editor edits what has meaning today.** Phase name, position, `template_name`,
  `token_budget`, `gate_hard` and `input_kinds` are editable; `fan_out`, `isolation`,
  `command_queue`, `verify_command` and `retry_limit` are rendered read-only. No document says any
  phase column is editable or by whom; editing fields no code consumes would be inventing MOD-4's
  semantics a milestone early. Editing `token_budget` closes the "phase rung is read but never
  written" gap the HANDOFF names.
- **D3 — `gate_hard` follows ANA-2's narrow wording, not HANDOFF's.** ANA-2 open item 5
  (`docs/ANA-2.md:2053`): the feature graph's `prd` and `plan`, the analysis graph's `verdict`,
  **"none elsewhere"**. `HANDOFF.md:290`'s "`prd`, `plan` and `verdict`" would harden `plan` in
  CLEAN and TOOL and destroy MOD-12's own reason for naming them the first unattended targets.
- **D4 — This item writes the ANA-2 seed amendments, and MOD-4's duplicate claim is struck.**
  ANA-2 `:1845` assigns them to "whichever of MOD-15 and MOD-4 lands second"; MOD-15 lands first and
  owns the seeder, so it writes them. `is_override` needs no write — the column arrives with `0003`
  defaulting to `false`, so HANDOFF's "`is_override = false`" is satisfied by omission. MOD-4's
  HANDOFF entry lists the same fixture corrections and loses them at this item's close-out.
- **D5 — `review` joins `implement.input_kinds` on CLEAN and TOOL too.** ANA-2 names the feature
  graph only and no document rules on the other two, but they carry the same `implement, review`
  pair and the same `R-ORCH-3` loop; without it a CLEAN retry cannot read its own review.
- **D6 — Deletion is refused whenever anything references the row.** ANA-9 documents no kind-delete
  rule; the FK on `item.kind_id` makes a used kind undeletable at the database level anyway, and
  `item_kind` has no `enabled` column to hide one with. A delete of an unreferenced kind succeeds; a
  referenced one refuses and names what holds it.
- **D7 — A typed setting registry, not a free-string writer.** `SettingSpec` per key (kind, valid
  range, which rungs accept it, unit, doc) becomes the single source that `DEFAULTS`, the seeder,
  the migration pin, the validator and the editor all read. The writer takes a typed key, so a typo
  does not compile. Three consequences, each chosen deliberately:
  - **Validation happens on write, with the reader's own rules, and refuses rather than clamps.**
    `positive_i64` fails silently, so an unvalidated write puts a number on screen that the prompt
    does not use; a silent clamp does the same thing more quietly.
  - **`set` and `clear` are separate operations.** Clear deletes the row so the compiled default
    answers, rather than the editor guessing at a constant.
  - **`project.settings` is merged key by key** — JSONB on Postgres, `serde_json` on `MemStore` —
    because a typed round-trip would erase MOD-4's and MOD-12's keys without a word.
  Seam cost is two methods (`set_setting`, `clear_setting`) over
  `rung = App | Project(ProjectId) | Phase(StepGraphPhaseId)`, not one per key.
- **D8 — Compare-and-set on `updated_at`.** `app_setting` has no `version` column and this item adds
  no migration, but the `BEFORE UPDATE` trigger maintains `updated_at` on all twenty triggered
  tables, which makes it a usable compare token: `WHERE key = $1 AND updated_at = $2`. This keeps
  `R-ENT-10`'s rule intact — the timestamp decides *whether* the write applies, it never decides
  *who wins*. Cost, accepted: the editor needs a reload-and-retry path on a miss.
- **D9 — The DSN is a startup requirement, and a box without one is redirected rather than
  stranded.** Maintainer's reasoning: MOD-25 made `htui` online-only, so no DSN is not a degraded
  mode. Today `connecting = false` means `reconnect: None` and no ticker is ever installed
  (`connect.rs:189-208`), so the box parks offline forever pointing at a CLI flag. Instead: launch
  with no DSN focuses Settings → connection (which is what `TabAction::FocusSection` is for), and
  the typed DSN goes to the keyring, re-opens `CacheStore` under `identity::db_fingerprint`'s new
  answer, installs the `Reconnect` closure, and attempts. The same path serves *replacing* a DSN,
  which is a mirror swap rather than a reconnect — the cache file is keyed on the fingerprint.
- **D10 — `Settings > Rebuild cache` is built here, not merely re-worded.** HANDOFF treats it as a
  landed MOD-6 artifact; in the tree it is a store method with no caller outside tests. It lands in
  the connection section with the confirmation copy the HANDOFF asks for, and it is never
  "helpfully" extended to clear anything else.
- **D11 — This item is `repo_box_path`'s first writer and carries the symlink discipline.** MOD-2's
  F-102 left the guard unowned because nothing wrote the row: `FsRepoReader` refuses a symlink below
  the root, but a `RepoRoot` that is *itself* a link is whoever-writes-the-row's problem. MOD-7 is
  the second writer and reuses the guard rather than repeating it.
- **D12 — The prefix-change warning's semantics are fixed now, its wording is drafted in the plan.**
  Old keys keep their text, the old counter row survives, the new prefix mints from 1. ANA-9 §10
  specifies only that the TUI warns.
- **D13 — Workspaces and projects can be deleted, behind two confirmations with red warnings.**
  Maintainer decision, 2026-09-16, against the research's own proposal of no delete at all. What it
  costs is exact and the copy must say it: `project` is the parent of an `ON DELETE CASCADE` chain
  that reaches `item` (`0001_init.sql:300`), `item_key_counter` (`:300`), `item_kind` (`:284`),
  `step_graph` (`:216`) and its phases (`:230`), `prompt_template` (`:268`), `repo` (`:187`) and
  `repo_box_path` (`:203`), `skill_binding` (`:435`), `run` (`:449`) and every `run_step`,
  `session_event` and `run_step_commit` below it, plus `item_note`, `item_revision`, `item_link` and
  `document`. Deleting a project deletes its entire history, not its registration. Three
  consequences:
  - **This amends `docs/ANA-9.md` §4.1's prohibition** — *"no `DELETE FROM item`; closing is a
    status … Counters are never decremented"* (`:151-152`) — which a cascade performs exactly. The
    amendment is narrow and deliberate: the rule still forbids deleting an *item*; it no longer
    forbids dropping a project whole. Per the milestone-5 precedent an ANA edit is maintainer-only,
    so this is recorded in the write-up rather than in the file, and the write-up carries the
    section it touches.
  - **Two confirmations, and the second is typed.** A red warning naming the row counts that will
    go, then a confirmation that requires typing the project's slug — a `y`/`n` twice is two
    keystrokes on the same reflex.
  - **The mirror must be pruned or rebuilt.** Cache refresh is cursor-based on `updated_at`
    (`cache/refresh.rs:223`) and the only delete propagation that exists is `item_link`'s tombstone
    (`:918-924`); `workspace` and `workspace_project` happen to be full-table replaced (`:516`,
    `:552`), but `project`, `item`, `repo`, `item_kind` and `document` are not. A deleted project
    therefore keeps showing in the Backlog off a stale mirror. The delete path triggers the rebuild
    D10 builds rather than inventing a second pruning mechanism.
  - Asymmetry with D6 is intended: deleting an *item kind* that is referenced refuses, because it
    would orphan live items inside a surviving project. Deleting the project takes the items with
    it, on purpose, with the user told exactly that.

## Open Questions

- [ ] Whether `workspace_box_path` and `repo_box_path` should be mirrored into the SQLite cache.
      They are absent from `MIRRORED_TABLES` and have no reader today. An offline box cannot use
      them for anything MOD-25 still permits, so the assumption is **not mirrored**; MOD-4's
      isolation modes may disagree and would amend it.
- [ ] Whether the `agents` section stays first in the strip once four more sections register.
      Registration order is strip order (`settings/mod.rs:91`); no document ranks them.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| A typed `ProjectSettings` round-trip silently erases MOD-4's or MOD-12's keys | High if unguarded | High — invisible for months | D7's key-level merge, plus a test writing one key into a blob that carries unknown ones |
| A value is stored that the read half ignores, so the editor and the prompt disagree | High (`positive_i64` fails silently) | Medium | D7's validate-on-write using the reader's own rules; property test that stored equals effective |
| The seed lands in an order the schema rejects, or drifts from `R-ENT-6` | Medium | High — a project that cannot mint | Order fixed by `default_graph_id NOT NULL`; conformance case asserting counts and names on both stores |
| Changing the DSN leaves the mirror open under the old fingerprint | Medium | High — reads answered from another database's cache | D9 re-opens `CacheStore` as part of the DSN change; a test that swaps DSNs and asserts the file |
| A project delete destroys irreplaceable history — every item, run and transcript, with no undo and no backup in this tree | Low per use, catastrophic once | Highest in the item | D13's two confirmations with the second typed, counts shown before the act, and copy that says "history" rather than "project"; the cascade is the database's, so nothing partial can be left behind |
| The mirror keeps serving a deleted project because refresh rides `updated_at` and only `item_link` tombstones | High if unguarded | Medium | D13 rebuilds the mirror on the delete path; a test asserting the Backlog is empty of it without a manual rebuild |
| Compare-and-set on `updated_at` fights the trigger, or two writes inside one clock tick compare equal | Medium | Medium | `clock_timestamp()` in the trigger, not `now()`; a conformance case over two writes in one transaction-free sequence |
| The item is large enough that milestone 1's seam churns under milestones 3–6 | Medium | Medium | Milestones 1 and 2 are UI-free and land first; sections consume a settled seam |
| Five registered sections overflow the strip at the pinned 100×30 harness width | Medium | Low | The same class as MOD-30, which is open and owns the fix; this item pins strip width against the pane in a test rather than re-accepting snapshots |
| `repo_box_path`'s symlink guard is written here and re-implemented by MOD-7 | Medium | Medium | D11 makes it a shared guard and names MOD-7 as its second caller |
| The keyring backend behaves differently on Windows, or is absent on a headless Linux box | Medium | Medium | Existing `secret.rs` surface is unchanged; runtime verification is MOD-16's, and `--set-dsn` remains a working fallback |

---
*Status: DRAFT — requirements only. Implementation planning pending via `/plan`.*
