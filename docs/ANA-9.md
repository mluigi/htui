# ANA-9 - Data model v2: Postgres schema, key sequences, divergence, event rows, cache

> **Scope note:** Design authority for the Postgres schema of `htui` and for the per-box read-only
> cache. Replaces the DDL of `docs/ANA-1.md` and `docs/ANA-8.md` in full. Governed by
> `.claude/rules/workflow-docs.md`, `CONCEPTS.md` and `docs/REQUIREMENTS.md`.
>
> **Requirements addressed:** `R-USR-2`, `R-BOX-1..4`, `R-ENT-1..12`, `R-STO-1`, `R-STO-3..6`,
> `R-HIS-1..3`, `R-SKL-1..2`, `R-PRM-3..4`, `R-AGT-4`, `R-AGT-6..7`, `R-ORCH-1`, `R-ORCH-9..11`,
> `R-MCP-3`. Reserved shape for `R-USR-3`, `R-ORCH-12..13`, `R-LATER-2..3`.
>
> **Status (2026-09-03): concluded.** Implementation tracked as MOD-6 (Postgres store + cache) in
> `HANDOFF.md`. ANA-2, ANA-4, ANA-5 and ANA-7 amend this schema by forward-only migration where
> their verdicts need columns this document only reserves.

---

## 1. Context and problem statement

`docs/REQUIREMENTS.md` fixed the product contract: Postgres is the single source of truth
(`R-ID-3`), repos stay clean (`R-ID-4`), boxes keep a read-only cache and open offline from it
(`R-STO-3`, `R-STO-4`), keys are minted online (`R-ENT-7`), concurrent edits never merge silently
(`R-ENT-10`), and every session event persists as an ordered row (`R-HIS-1`). ANA-1 and ANA-8 were
written before that contract and carry a schema that contradicts it in four places: `items.json`
as a second replica, timestamp last-writer-wins, a `sync_state` engine, and the `artifact` facts
tables (`R-ENT-13`, withdrawn).

This document produces the schema that implements the contract, and settles the four questions the
HANDOFF item names:

1. **Per-project key sequences** - how `<PREFIX>-<N>` is minted per project and prefix, gapless
   enough to never reuse, and how the legacy importer preserves existing keys.
2. **Divergence detection on `version`** - what the optimistic lock covers, what it does not, and
   how the common ancestor is found for the three-way view.
3. **Event row shape for replay** - one row per session event, the kind vocabulary, the payload
   contract, and how the offline chat buffer reuses the same shape.
4. **Cache file format and refresh cursor** - what the per-box cache is, how it is refreshed
   incrementally without missing rows, and when it is rebuilt.

Everything else in this document is the table set those decisions hang on. It is a design, not a
migration file: MOD-6 turns §5 into `migrations/0001_init.sql` under `sqlx`.

---

## 2. Invariants the schema enforces

Restated from `CONCEPTS.md`; each one has a mechanical enforcement point below.

1. **One writable store.** Only Postgres accepts writes. The cache is written by the refresh task
   alone, and never by user actions (§6.3). Enforced by the store trait split: `ReadStore` for both
   backends, `WriteStore` implemented only by the Postgres backend.
2. **Keys are minted online and never reused.** The counter row for `(project, prefix)` only ever
   increases; there is no delete path for `item` (§4.1). Enforced by `item_key_counter` and the
   absence of any `DELETE FROM item` in the store.
3. **No silent merge.** Every spec edit is a compare-and-set on `item.version`; a lost race is
   surfaced with the ancestor revision, never retried blindly (§4.2).
4. **Nothing about a run exists only on one box.** Session events, prompts, follow-ups and
   permission answers are rows in `session_event` (§4.3); the offline chat buffer is a queue in
   front of that table, not a second store.
5. **No agent in a bookkeeping path.** Every query in §7 is issued by `htui` code. No trigger calls
   out; no table is written from an agent tool except through the MCP server's own scoped code.
6. **Logical identity never depends on a path.** Paths live only in `repo_box_path` and
   `workspace_box_path`, keyed by box (`R-BOX-4`).

---

## 3. Entity map

```
app_user ─┬─< box ─┬─< box_tool
          │        ├─< repo_box_path >─ repo
          │        ├─< workspace_box_path >─ workspace
          │        └─< agent_box >─ agent
          │
          └─ created_by on item, item_revision, item_note, document, run, skill_version

workspace ─< workspace_project >─ project ─┬─< repo
                                           ├─< item_kind ──> step_graph (default)
                                           ├─< item_key_counter
                                           ├─< step_graph ─< step_graph_phase ─< phase_agent >─ agent
                                           ├─< prompt_template
                                           ├─< skill_binding >─ skill ─< skill_version
                                           └─< item ─┬─< item_revision
                                                     ├─< item_link (from/to, cross-project by UUID)
                                                     ├─< item_note
                                                     ├─< document
                                                     └─< run ─< run_step ─┬─< session_event
                                                                          ├─< run_step_commit >─ repo
                                                                          └─< command_run

app_setting (key/value)    capability_tag (open vocabulary, seeded)
```

Thirty tables. The count is deliberate: every requirement in the scope list maps to at least one
row shape, and no table exists for a `later` requirement beyond a reserved column.

Conventions, applied throughout §5:

| Convention | Choice | Why |
|---|---|---|
| Primary keys | `UUID`, supplied by `htui` as UUIDv7, `DEFAULT gen_random_uuid()` as fallback | Client-side minting lets the offline chat buffer name a run before Postgres is reachable; v7 keeps B-tree inserts ordered. Native `uuidv7()` needs Postgres 18; the app generates instead. |
| Enumerations | `TEXT` + `CHECK` | Adding a value is an `ALTER ... DROP/ADD CONSTRAINT`, not a type change; `sqlx` maps plain strings without a custom type per enum. |
| Timestamps | `TIMESTAMPTZ`, `updated_at` maintained by one trigger function | The cache cursor (§4.4) depends on `updated_at` being set on every write, so it is not left to the application. `clock_timestamp()` rather than `now()` so long transactions do not backdate rows. |
| Soft delete | `deleted_at` on `item_link` only | It is the only user-visible row that can be removed; a tombstone lets the cache cursor see the removal. Everything else is append-only or versioned. |
| JSON | `JSONB` for shapes another ANA owns (`box.probe`, `run.graph_snapshot`, `session_event.payload`, `settings`) | Columns are reserved for what is queried; blobs for what is displayed or replayed. |
| Minimum server | Postgres 16 | `UNIQUE NULLS NOT DISTINCT` (15+) is used for bindings; 16 is the oldest release still in support through the version one window. |

---

## 4. Settled questions

### 4.1 Per-project key sequences (`R-ENT-5`, `R-ENT-7`, `R-LATER-3`)

**Need.** `<PREFIX>-<N>` unique per project; prefixes are per project and open (`R-ENT-6`); numbers
are never reused even after a kind is renamed or an item is closed; the legacy importer must insert
items with their existing numbers and leave the counter above them.

**Options.**

| Option | Mechanism | Verdict |
|---|---|---|
| A. Native `SEQUENCE` per `(project, prefix)` | `CREATE SEQUENCE` at kind creation, `nextval()` at mint | Rejected. Runtime DDL from the application, sequences are not transactional (a rolled-back mint burns a number, harmless but noisy), no foreign key ties a sequence to its project, and importer alignment needs `setval` juggling. |
| B. `max(key_number) + 1` with unique-violation retry | Read max, insert, retry on conflict | Rejected. Correct only with a retry loop; two boxes minting at once collide on the unique index and one retries. Reuse becomes possible if the highest item is ever deleted. |
| C. Counter table, `INSERT ... ON CONFLICT DO UPDATE ... RETURNING` | One row per `(project_id, prefix)`, upsert-increment inside the item's insert transaction | **Adopted.** One statement mints and creates the counter row on first use; the row lock serializes concurrent mints for the same prefix only (the pattern the Postgres lists recommend for gapless numbering); rollback returns the number. The importer aligns with `GREATEST`. |

Contention is bounded by the transaction that holds the row lock: the mint runs in the same
transaction as the `item` insert and nothing else, so the lock is held for milliseconds. The
gapless property is a side effect, not a goal; what matters is that the counter is monotonic and
outlives the items.

**Shape.**

```sql
CREATE TABLE item_key_counter (
    project_id  UUID    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    prefix      TEXT    NOT NULL,
    last_value  INTEGER NOT NULL CHECK (last_value >= 0),
    PRIMARY KEY (project_id, prefix)
);
```

`item` stores `key_prefix` and `key_number` as columns and derives `key` as a stored generated
column, so `UNIQUE (project_id, key_prefix, key_number)` is the uniqueness rule and sorting by
number is natural (no `MOD-10 < MOD-9` text-sort trap). `key_prefix` is copied from the kind at
mint time and never rewritten: renaming a kind's prefix starts a new counter for the new prefix
while existing keys keep their text.

**Mint** (§7.1) is one statement with a CTE; **import** uses
`ON CONFLICT ... DO UPDATE SET last_value = GREATEST(item_key_counter.last_value, EXCLUDED.last_value)`
per imported item, and only then inserts the item with its explicit number.

**Rules the store enforces:** no `DELETE FROM item`; closing is a status. No `UPDATE` of
`key_prefix`/`key_number`. Counters are never decremented.

### 4.2 Divergence detection on `version` (`R-ENT-5`, `R-ENT-10`)

**Need.** Two online boxes (or the TUI and an external editor on one box) may edit the same item.
The losing write must be rejected and shown against the common ancestor. Status transitions are
driven by the orchestrator (`R-ENT-8`) and must not be caught by a user's in-flight edit.

**What `version` covers.** The spec columns a human edits: `title`, `body`, `kind_id`,
`required_tags`, `priority`, `touched_paths`, `step_graph_id`. Every change to any of them is one
compare-and-set that increments `version` and inserts an `item_revision` snapshot in the same
transaction.

**What `version` does not cover.** `status`, `closed_at`, `updated_at`. Status moves are their own
compare-and-set on `status` (`UPDATE item SET status = $new WHERE id = $1 AND status = $expected`);
a failed status CAS is an orchestrator race, reported as such, and it never touches the revision
log. Rationale: if status bumped `version`, every step completion would invalidate the body edit
the user has open in `$EDITOR`.

**Options for the conflict rule.**

| Option | Verdict |
|---|---|
| Timestamp last-writer-wins | Rejected by `R-ENT-10`; also the ANA-1 rule this document retires. |
| Server-side merge (three-way in SQL or a trigger) | Rejected. Merging markdown bodies is a UI decision, not a storage one; a wrong automatic merge is the silent overwrite the requirement forbids. |
| Client compare-and-set on `version`, ancestor from `item_revision` | **Adopted.** Zero rows updated means someone else committed first; the TUI loads the current head and the revision at the version it started from and renders the three-way view. |

**Divergence flow.**

1. TUI reads item at `version = v`, user edits (in the TUI or via `$EDITOR`).
2. Store issues §7.2 with `expected_version = v`. One row updated: done, revision `v+1` written.
3. Zero rows: store returns `Diverged { head, ancestor }` where `head` is the current row and
   `ancestor` is `item_revision(item_id, v)`. The TUI shows ancestor / theirs (head) / mine (buffer).
   The user resolves by editing and re-submitting against `head.version`; the resolution is
   recorded as a revision with `reason = 'divergence_resolution'`.
4. Offline mode never edits, so there is no queued write to reconcile (`R-STO-4`).

`item_revision.version = 1` is written at creation, so the ancestor always exists.

### 4.3 Event row shape for replay (`R-AGT-1`, `R-HIS-1..3`, `R-STO-4`)

**Need.** Every driver event, plus the assembled prompt, every follow-up and every permission
answer, stored as ordered rows per run step after scrubbing; the chat tab replays any past step;
the free-standing offline chat buffers locally and lands later with no loss of order.

**Options.**

| Option | Verdict |
|---|---|
| One `transcript TEXT` (or file URI) per step, as ANA-1 `transcript_ref` | Rejected. No per-event query, no partial cache, no replay without re-parsing, and the offline buffer would need a merge step. |
| Raw wire messages (ACP `session/update` JSON) one per row | Rejected as the only form. The CLI fallback adapter has no ACP wire shape, so replay would need two renderers; ACP chunks agent text one token-run at a time, which multiplies rows for no replay value. |
| Normalized `htui` event rows, kind vocabulary fixed by `R-AGT-1`, payload `JSONB` per kind, optional raw retained | **Adopted.** The driver trait already defines the event model; storing it means one renderer for live and replay. Raw is kept only when a project opts in, for adapter debugging. |

**Shape.**

```sql
CREATE TABLE session_event (
    run_step_id   UUID        NOT NULL REFERENCES run_step(id) ON DELETE CASCADE,
    seq           INTEGER     NOT NULL,             -- 0-based, assigned by htui at capture
    turn          INTEGER     NOT NULL DEFAULT 0,   -- increments on each prompt/follow_up
    kind          TEXT        NOT NULL,
    role          TEXT        NOT NULL,             -- 'user' | 'agent' | 'htui'
    tool_call_id  TEXT,                             -- pairs tool_call / tool_result / edit_proposal / permission_*
    payload       JSONB       NOT NULL,
    raw           JSONB,                            -- wire message, only when project.settings.keep_raw_events
    at            TIMESTAMPTZ NOT NULL,             -- capture time on the executing box
    PRIMARY KEY (run_step_id, seq),
    CONSTRAINT chk_event_kind CHECK (kind IN (
        'prompt', 'follow_up', 'assistant_text', 'thought', 'tool_call', 'tool_result',
        'edit_proposal', 'permission_request', 'permission_answer', 'plan', 'usage',
        'error', 'done', 'other')),
    CONSTRAINT chk_event_role CHECK (role IN ('user', 'agent', 'htui'))
);
CREATE INDEX idx_session_event_tool ON session_event (run_step_id, tool_call_id)
    WHERE tool_call_id IS NOT NULL;
```

**Payload contract per kind** (keys are the minimum; the driver may add keys, never rename these).

| kind | role | payload keys |
|---|---|---|
| `prompt` | htui | `text`, `digest` (sha256 hex), `sections[]` (`name`, `tokens`, `trimmed` bool) - the assembled initial prompt, always `seq = 0`, `turn = 0` |
| `follow_up` | user | `text` |
| `assistant_text` | agent | `text` (coalesced contiguous chunks; the driver flushes on kind change or turn end) |
| `thought` | agent | `text` (coalesced the same way) |
| `tool_call` | agent | `title`, `tool_kind` (ACP `read|edit|delete|move|search|execute|think|fetch|other`), `input`, `locations[]` |
| `tool_result` | agent | `status` (`completed|failed`), `output` (scrubbed), `locations[]` |
| `edit_proposal` | agent | `path`, `diff` (unified), `accepted` (bool or null while pending) |
| `permission_request` | agent | `request_id`, `options[]` (`id`, `label`, `kind`) |
| `permission_answer` | user or htui | `request_id`, `option_id`, `by` (`user|policy`) |
| `plan` | agent | `entries[]` (`content`, `status`, `priority`) - ACP plan updates, kept because the Runs tab shows them |
| `usage` | agent | `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `cost_micros` (nullable) |
| `error` | agent or htui | `code`, `message` |
| `done` | agent | `stop_reason` |
| `other` | agent | `update` - any ACP `sessionUpdate` value not mapped above (`available_commands_update`, `current_mode_update`, `config_option_update`, `session_info_update`), stored verbatim so "every event" holds without a schema change per protocol revision |

`seq` is the replay order and is total per step. `turn` groups a prompt or follow-up with the
agent's response to it. `at` is informational; ordering never uses it.

**Chunk coalescing** lives in the driver (MOD-2), not the store: the store receives one
`assistant_text` per contiguous run of chunks. Replay in the chat tab re-streams a coalesced row by
character to keep the live look; that is rendering, not data.

**Offline chat buffer** (`R-STO-4`). A free-standing chat started while Postgres is unreachable
mints its `run.id` and `run_step.id` client-side (UUIDv7), scrubs every event on the box, and
appends rows to `<cache_dir>/pending/<run_id>.jsonl` - one JSON object per line with exactly the
`session_event` columns. On the next successful connection the store inserts `run` (`kind =
'chat'`, `item_id NULL`), `run_step` (`phase_name = 'chat'`) and the events in one transaction,
then deletes the file. `PRIMARY KEY (run_step_id, seq)` makes a replayed upload idempotent.

**Retention** (`R-HIS-3`). `project.settings.retention_days` (null = keep everything). The one
delete path on this table is the retention sweep, which removes whole steps older than the window,
never single events.

### 4.4 Cache file format and refresh cursor (`R-STO-3`, `R-STO-4`, `R-STO-6`)

**Need.** Per box, read-only, under the user's config directory; contents are items, links,
documents, notes, run and step summaries and the transcripts of the last N steps; startup under one
second warm; refresh in the background on every successful connection without blocking input.

**Format options.**

| Option | Verdict |
|---|---|
| JSON files per project (`serde_json`) | Rejected. Whole-file parse on every start scales with documents and transcripts; no partial read; graph and filter queries reimplemented in memory. |
| `rkyv` mmap archives | Rejected. Near-zero load, but no schema evolution story (every cache rebuild on any struct change), and still one blob per project rewritten on refresh. |
| `redb` / `native_db` | Rejected. Pure Rust and stable, but key-value only: every filter the Backlog tab needs (status, project, tag, readiness, link traversal) becomes a hand-maintained secondary index. |
| **SQLite via `sqlx` (`sqlite` feature)** | **Adopted.** Same crate family as the Postgres backend, so the `ReadStore` trait has two `sqlx` implementations and no second query dialect to maintain beyond SQLite's. Indexed filters and recursive CTEs work as in Postgres. Partial refresh is an upsert per row. The bundled build adds a C dependency but no daemon (`R-NF-2` holds). |

**Layout.**

```
<config_dir>/htui/                     # dirs::config_dir(): %APPDATA%\htui, ~/.config/htui, ~/Library/Application Support/htui
  cache/<db_fingerprint>/cache.sqlite  # fingerprint = sha256(host:port/dbname), never includes credentials
  cache/<db_fingerprint>/pending/*.jsonl
  box.toml                             # box_id (UUID) + last known hostname; the box identity survives hostname changes
```

`cache.sqlite` runs in WAL mode: the refresh task is the single writer, the TUI is a reader, and a
second `htui` process on the same box is just another reader (two refreshers racing produce
idempotent upserts).

**Schema.** A mirror of the Postgres tables the offline views read, same table and column names,
plus two local tables:

```sql
CREATE TABLE cache_meta   (key TEXT PRIMARY KEY, value TEXT NOT NULL);
   -- 'schema_version' (Postgres migration version the mirror was built from),
   -- 'db_fingerprint', 'built_at', 'last_full_refresh_at'
CREATE TABLE cache_cursor (project_id TEXT NOT NULL, table_name TEXT NOT NULL,
                           high_water INTEGER NOT NULL,   -- server updated_at, microseconds since epoch
                           PRIMARY KEY (project_id, table_name));
```

Mirrored: `app_user`, `agent`, `box` (own row only), `workspace`, `workspace_project`, `project`,
`repo`, `item_kind`, `item`, `item_link`, `item_note`, `document`, `run`, `run_step`,
`run_step_commit`, `session_event` (last N steps per project,
`project.settings.cached_transcript_steps`, default 20). Not mirrored: revisions, skills,
templates, graphs, `agent_box`, counters, settings, command queue - none is browsed offline. UUIDs
are stored as `TEXT`, timestamps as `INTEGER` microseconds, `TEXT[]` as JSON arrays, `JSONB` as
`TEXT`.

`agent` was in the "not mirrored" list until MOD-2 milestone 4 (plan D31) and moved because an
offline chat has to resolve its driver from the registry row; it is unscoped, so it is a full
replace with no `cache_cursor` row. `agent_box` stays out: it is a probe snapshot whose columns
milestone 5 changes, and offline "which box" is always this box.

**Cursor options.**

| Option | Verdict |
|---|---|
| `updated_at > cursor` strict | Rejected. The visibility gap is real: a row written by a transaction that started before the cursor and committed after it has an `updated_at` below the cursor and is never fetched. |
| `pg_current_snapshot()` stored per refresh, re-fetch rows whose `xmin` was in-progress | Rejected for version one. Correct, but `xmin` is not indexable (a scan per table per refresh), and it ties the cache to MVCC internals for a case that a bounded overlap already covers given that `htui` is the only writer and its transactions are milliseconds long. |
| Trigger-fed `change_log` with a `BIGSERIAL` cursor | Rejected. Same commit-order gap as timestamps, plus a table that grows on every write for a single reader. |
| Logical replication (`pgoutput`) | Rejected. Needs `wal_level = logical` and slot management on a server `htui` does not administer (§14 of REQUIREMENTS). |
| **`updated_at` high-water mark with a fixed overlap window and idempotent upserts** | **Adopted.** Per `(project, table)`: fetch `WHERE project_id = $p AND updated_at > $high_water - interval '5 minutes' ORDER BY updated_at`, upsert on primary key, then set `high_water = max(updated_at)` of the fetched rows (server time, so clock skew between boxes is irrelevant). Any transaction shorter than the window is caught on the next pass. `item_link` removals are tombstones (`deleted_at`) and ride the same cursor; the mirror drops the row. |

The overlap is a setting (`app_setting.cache_overlap_seconds`, default 300). Rebuild from scratch
happens when `cache_meta.schema_version` differs from the connected server's migration version, on
`Settings > Rebuild cache`, and after a full refresh older than seven days (a background full pull
on a quiet connection, idempotent, to close any gap the window did not).

**Refresh sequence on connect** (`R-STO-6`).

1. Open `cache.sqlite`, render from it immediately (warm start is a few queries on a local file).
2. Connect to Postgres off the UI thread; on success, check migration version (§5.0), then run the
   cursor pass per opened project in the background, project by project, tables in FK order.
3. After the pass, upload any `pending/*.jsonl` chat buffers (§4.3).
4. Repeat the cursor pass every `app_setting.cache_refresh_seconds` (default 30) while connected.
   `LISTEN`/`NOTIFY` as a wake-up is a later optimization, not part of version one; polling at this
   rate on a single-developer database is negligible.

Live TUI reads while connected go to Postgres, not the cache; the cache exists for startup and for
offline. That keeps "what you see" always the source of truth when it is reachable.

---

## 5. Postgres schema (DDL)

### 5.0 Migrations

`sqlx::migrate!` with `migrations/0001_init.sql` holding everything below. Forward-only in version
one (`R-STO-5`): `htui` compares `_sqlx_migrations` against its embedded set on connect, refuses to
run against a newer schema, and asks before applying pending ones. Later ANAs add `000N_*.sql`
files; they never edit `0001`.

The sections below are grouped by topic, not by dependency order. The migration file creates tables
in FK order (`app_user`, `capability_tag`, `box`, `agent`, `workspace`, `project`, ..., `run`,
`run_step`, `session_event`, `command_run`) and adds the forward references to `run_step`
(`item_link.proposed_by_step_id`, `item_note.via_step_id`, `document.produced_by_step_id`) with
`ALTER TABLE ... ADD CONSTRAINT` at the end.

### 5.1 Common function

```sql
CREATE FUNCTION set_updated_at() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at := clock_timestamp();
    RETURN NEW;
END $$;
-- Attached at the end of the migration to every table that has updated_at:
-- DO $$ DECLARE t text; BEGIN
--   FOREACH t IN ARRAY ARRAY['app_user','box','workspace','project','repo','repo_box_path',
--     'workspace_box_path','item_kind','item','item_link','step_graph','step_graph_phase',
--     'prompt_template','skill','skill_binding','agent','agent_box','run','run_step','app_setting']
--   LOOP EXECUTE format('CREATE TRIGGER trg_%s_updated_at BEFORE UPDATE ON %I
--                        FOR EACH ROW EXECUTE FUNCTION set_updated_at()', t, t);
--   END LOOP; END $$;
```

### 5.2 Users and boxes (`R-USR-2`, `R-BOX-1..4`, `R-AGT-6..7`)

```sql
CREATE TABLE app_user (                          -- "user" is reserved in SQL
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL UNIQUE,
    email       TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Version one seeds exactly one row on first connect; R-USR-3 adds rows, never columns.

CREATE TABLE capability_tag (
    tag         TEXT PRIMARY KEY,                -- 'gpu','vulkan','msvc','mingw','clang','cmake','vcpkg','docker','rust','heavy_build' seeded
    description TEXT NOT NULL DEFAULT '',
    seeded      BOOLEAN NOT NULL DEFAULT false
);

CREATE TABLE box (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),   -- minted on first launch, kept in box.toml
    user_id         UUID NOT NULL REFERENCES app_user(id),
    hostname        TEXT NOT NULL,
    os_family       TEXT NOT NULL CHECK (os_family IN ('windows','linux','macos')),
    os_version      TEXT NOT NULL,
    arch            TEXT NOT NULL,
    cpu             TEXT NOT NULL DEFAULT '',
    ram_mb          INTEGER,
    gpu_present     BOOLEAN NOT NULL DEFAULT false,
    gpu_vendor      TEXT,
    htui_version    TEXT NOT NULL,               -- re-probe trigger (R-BOX-2)
    probed_tags     TEXT[] NOT NULL DEFAULT '{}',
    declared_tags   TEXT[] NOT NULL DEFAULT '{}',
    quirks          TEXT NOT NULL DEFAULT '',
    settings        JSONB NOT NULL DEFAULT '{}', -- command_limits {class: n} (R-MCP-3), max_concurrent_items (R-ORCH-9)
    registered_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_probed_at  TIMESTAMPTZ,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, hostname)
);
CREATE INDEX idx_box_tags ON box USING GIN ((probed_tags || declared_tags));

CREATE TABLE box_tool (                          -- compilers, build tools, shells, container runtime (R-BOX-2)
    box_id      UUID NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,                   -- 'rustc','cargo','gcc','clang','cl','cmake','ninja','vcpkg','pwsh','bash','docker',...
    version     TEXT NOT NULL,
    path        TEXT NOT NULL,
    probed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (box_id, name)
);
```

### 5.3 Hierarchy (`R-ENT-1..4`, `R-BOX-4`)

```sql
CREATE TABLE workspace (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug        TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_by  UUID NOT NULL REFERENCES app_user(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE project (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug            TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    secret_provider TEXT,                        -- 'infisical' | NULL; ANA-7 may add columns
    secret_scope    TEXT,                        -- provider-specific project/env reference
    settings        JSONB NOT NULL DEFAULT '{}', -- token_budget, retention_days, cached_transcript_steps,
                                                 -- keep_raw_events, default_isolation, per_token_cap_run, per_token_cap_batch
    created_by      UUID NOT NULL REFERENCES app_user(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE workspace_project (
    workspace_id UUID NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id   UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (workspace_id, project_id)
);
CREATE INDEX idx_workspace_project_project ON workspace_project(project_id);

CREATE TABLE repo (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id      UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    remote_url      TEXT,
    default_branch  TEXT NOT NULL DEFAULT 'main',
    is_primary      BOOLEAN NOT NULL DEFAULT false,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, name)
);
CREATE UNIQUE INDEX uq_repo_primary ON repo(project_id) WHERE is_primary;   -- exactly one primary per project

CREATE TABLE repo_box_path (
    repo_id     UUID NOT NULL REFERENCES repo(id) ON DELETE CASCADE,
    box_id      UUID NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    local_path  TEXT NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (repo_id, box_id)
);

CREATE TABLE workspace_box_path (
    workspace_id UUID NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    box_id       UUID NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    root_path    TEXT NOT NULL,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, box_id)
);
```

### 5.4 Step graphs and templates (`R-ORCH-1`, `R-ORCH-8`, `R-MCP-3`, `R-PRM-4`)

Defined before `item_kind` because kinds point at their default graph. The phase column set is
exactly what `R-ORCH-1` enumerates; ANA-2 owns the semantics and amends by migration.

```sql
CREATE TABLE step_graph (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, name)
);

CREATE TABLE step_graph_phase (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    graph_id         UUID NOT NULL REFERENCES step_graph(id) ON DELETE CASCADE,
    position         INTEGER NOT NULL,
    name             TEXT NOT NULL,              -- 'prd','plan','implement','review','research','verdict','reproduce','fix',...
    fan_out          INTEGER NOT NULL DEFAULT 1 CHECK (fan_out >= 1),
    gate             TEXT NOT NULL DEFAULT 'always' CHECK (gate IN ('always','on_failure','never')),
    gate_hard        BOOLEAN NOT NULL DEFAULT false,
    retry_limit      INTEGER NOT NULL DEFAULT 1 CHECK (retry_limit >= 0),
    input_kinds      TEXT[] NOT NULL DEFAULT '{}',
    output_kind      TEXT NOT NULL,              -- document.kind this phase produces; defaults to name
    isolation        TEXT CHECK (isolation IN ('worktree','copy','shared_serialized','local')),  -- NULL = project default
    command_queue    TEXT NOT NULL DEFAULT 'fan_out_only' CHECK (command_queue IN ('off','fan_out_only','always')),
    verify_command   TEXT,
    template_name    TEXT NOT NULL,              -- prompt_template.name, defaults to phase name
    template_version INTEGER,                    -- NULL = follow latest
    token_budget     INTEGER,                    -- NULL = project.settings.token_budget
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (graph_id, position),
    UNIQUE (graph_id, name)
);

CREATE TABLE phase_agent (                       -- candidate agents in priority order (R-ORCH-1, R-AGT-8)
    phase_id    UUID NOT NULL REFERENCES step_graph_phase(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    agent_id    UUID NOT NULL REFERENCES agent(id),
    model       TEXT NOT NULL,
    PRIMARY KEY (phase_id, position)
);

CREATE TABLE prompt_template (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,                   -- phase name; defaults are copied into each new project
    version     INTEGER NOT NULL CHECK (version >= 1),
    body        TEXT NOT NULL,                   -- placeholder contract per ANA-5
    created_by  UUID NOT NULL REFERENCES app_user(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, name, version)
);
```

Item override of a graph (`R-ORCH-1`): the TUI clones the kind's graph into a new `step_graph` row
named `<key>-override` and points `item.step_graph_id` at it. No per-item phase table.

### 5.5 Kinds, keys, items, links, revisions, notes, documents (`R-ENT-5..12`)

```sql
CREATE TABLE item_kind (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id       UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    prefix           TEXT NOT NULL CHECK (prefix ~ '^[A-Z][A-Z0-9]{1,15}$'),
    name             TEXT NOT NULL,              -- 'analysis','feature','bug','refactor','tooling' seeded
    description      TEXT NOT NULL DEFAULT '',
    default_graph_id UUID NOT NULL REFERENCES step_graph(id),
    position         INTEGER NOT NULL DEFAULT 0,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, prefix),
    UNIQUE (project_id, name)
);

CREATE TABLE item_key_counter (                  -- §4.1
    project_id  UUID    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    prefix      TEXT    NOT NULL,
    last_value  INTEGER NOT NULL CHECK (last_value >= 0),
    PRIMARY KEY (project_id, prefix)
);

CREATE TABLE item (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id     UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    kind_id        UUID NOT NULL REFERENCES item_kind(id),
    key_prefix     TEXT NOT NULL,
    key_number     INTEGER NOT NULL CHECK (key_number >= 1),
    key            TEXT GENERATED ALWAYS AS (key_prefix || '-' || key_number::text) STORED,
    title          TEXT NOT NULL,
    body           TEXT NOT NULL DEFAULT '',
    status         TEXT NOT NULL DEFAULT 'open' CHECK (status IN
                     ('open','queued','in_progress','awaiting_approval','blocked','done','failed','closed')),
    priority       SMALLINT NOT NULL DEFAULT 0,  -- R-ORCH-6 ordering; higher first
    required_tags  TEXT[] NOT NULL DEFAULT '{}', -- R-ORCH-10
    touched_paths  TEXT[] NOT NULL DEFAULT '{}', -- R-ORCH-9 declared overlap set, repo-relative globs
    step_graph_id  UUID REFERENCES step_graph(id),  -- NULL = kind default
    version        INTEGER NOT NULL DEFAULT 1,   -- §4.2
    created_by     UUID NOT NULL REFERENCES app_user(id),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at      TIMESTAMPTZ,
    UNIQUE (project_id, key_prefix, key_number)
);
CREATE INDEX idx_item_project_status ON item(project_id, status);
CREATE INDEX idx_item_required_tags ON item USING GIN (required_tags);
CREATE INDEX idx_item_updated_at ON item(project_id, updated_at);   -- cache cursor

CREATE TABLE item_revision (
    item_id        UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    version        INTEGER NOT NULL,
    title          TEXT NOT NULL,
    body           TEXT NOT NULL,
    required_tags  TEXT[] NOT NULL,
    author_id      UUID NOT NULL REFERENCES app_user(id),
    box_id         UUID REFERENCES box(id) ON DELETE SET NULL,
    reason         TEXT NOT NULL DEFAULT '',     -- 'created','edited','divergence_resolution','imported',...
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (item_id, version)
);

CREATE TABLE item_link (                         -- from --kind--> to; 'blocked_by': from is blocked by to
    from_item_id  UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    to_item_id    UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL CHECK (kind IN ('blocked_by','origin','relates','supersedes')),
    proposed_by_step_id UUID REFERENCES run_step(id) ON DELETE SET NULL,   -- NULL = importer
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at    TIMESTAMPTZ,                   -- tombstone; live edges have NULL
    PRIMARY KEY (from_item_id, to_item_id, kind),
    CHECK (from_item_id <> to_item_id)
);
CREATE INDEX idx_item_link_to ON item_link(to_item_id) WHERE deleted_at IS NULL;

CREATE TABLE item_note (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    item_id     UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    body        TEXT NOT NULL,
    created_by  UUID NOT NULL REFERENCES app_user(id),
    box_id      UUID REFERENCES box(id) ON DELETE SET NULL,
    via_step_id UUID REFERENCES run_step(id) ON DELETE SET NULL,   -- set when added through MCP note_add
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_item_note_item ON item_note(item_id, created_at);

CREATE TABLE document (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    item_id             UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    kind                TEXT NOT NULL,           -- open set: phase output kinds plus 'summary'
    version             INTEGER NOT NULL CHECK (version >= 1),
    title               TEXT NOT NULL,
    body                TEXT NOT NULL,
    produced_by_step_id UUID REFERENCES run_step(id) ON DELETE SET NULL,   -- NULL = written by hand
    created_by          UUID NOT NULL REFERENCES app_user(id),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (item_id, kind, version)
);
```

Notes and documents are append-only (`R-ENT-11`, `R-ENT-12`), so they carry `created_at` alone;
the cache cursor uses it in place of `updated_at` for those two tables.

### 5.6 Skills (`R-SKL-1..2`)

```sql
CREATE TABLE skill (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL UNIQUE,            -- global library; scoping is by binding
    description TEXT NOT NULL DEFAULT '',
    created_by  UUID NOT NULL REFERENCES app_user(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE skill_version (
    skill_id    UUID NOT NULL REFERENCES skill(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL CHECK (version >= 1),
    body        TEXT NOT NULL,
    created_by  UUID NOT NULL REFERENCES app_user(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (skill_id, version)
);

CREATE TABLE skill_binding (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    skill_id       UUID NOT NULL REFERENCES skill(id) ON DELETE CASCADE,
    project_id     UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    phase_id       UUID REFERENCES step_graph_phase(id) ON DELETE CASCADE,   -- NULL = project level
    pinned_version INTEGER,                      -- NULL = follow latest
    position       INTEGER NOT NULL DEFAULT 0,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)
);
```

Resolution for a step: project-level bindings of the item's project, overridden per `skill_id` by
a binding whose `phase_id` is the current phase (`R-SKL-2`).

### 5.7 Agents (`R-AGT-4`, `R-AGT-6..7`)

```sql
CREATE TABLE agent (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name           TEXT NOT NULL UNIQUE,         -- 'claude','agy' seeded
    transport      TEXT NOT NULL CHECK (transport IN ('acp','cli')),
    launch         JSONB NOT NULL,               -- {argv: [...], env: {...}}; ANA-4 fixes the shape
    models         TEXT[] NOT NULL DEFAULT '{}',
    default_model  TEXT,
    billing        TEXT NOT NULL CHECK (billing IN ('subscription','per_token')),
    enabled        BOOLEAN NOT NULL DEFAULT true,
    settings       JSONB NOT NULL DEFAULT '{}', -- adapter-specific, ANA-4
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE agent_box (                         -- per-box enablement, discovery and quota snapshot
    agent_id       UUID NOT NULL REFERENCES agent(id) ON DELETE CASCADE,
    box_id         UUID NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    enabled        BOOLEAN NOT NULL DEFAULT true,
    version        TEXT,
    path           TEXT,
    probed_at      TIMESTAMPTZ,
    quota          JSONB,                        -- {remaining, reset_at, ...} as reported (R-AGT-7)
    quota_at       TIMESTAMPTZ,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_id, box_id)
);
```

### 5.8 Runs, steps, events, commands (`R-ORCH-9..11`, `R-HIS-1..3`, `R-MCP-3`)

```sql
CREATE TABLE run (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id       UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    item_id          UUID REFERENCES item(id) ON DELETE CASCADE,      -- NULL for free-standing chat
    kind             TEXT NOT NULL CHECK (kind IN ('graph','chat')),
    mode             TEXT NOT NULL CHECK (mode IN ('manual','auto')),
    status           TEXT NOT NULL DEFAULT 'queued' CHECK (status IN
                       ('queued','running','awaiting_approval','done','failed','cancelled')),
    target_box_id    UUID NOT NULL REFERENCES box(id),               -- R-ORCH-12 reserved: executes only when local
    executing_box_id UUID REFERENCES box(id),
    graph_snapshot   JSONB,                       -- step_graph + phases + phase_agent at start (R-ORCH-11)
    started_by       UUID NOT NULL REFERENCES app_user(id),
    queued_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at       TIMESTAMPTZ,
    finished_at      TIMESTAMPTZ,
    failure          TEXT,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_run_item ON run(item_id, queued_at DESC);
CREATE INDEX idx_run_project_status ON run(project_id, status);

CREATE TABLE run_step (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id         UUID NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    position       INTEGER NOT NULL,             -- phase index in the snapshot
    attempt        INTEGER NOT NULL DEFAULT 1,   -- retry / review loop counter
    fanout_index   INTEGER NOT NULL DEFAULT 0,   -- 0..fan_out-1
    phase_name     TEXT NOT NULL,                -- 'chat' for run.kind = 'chat'
    agent_id       UUID REFERENCES agent(id),
    model          TEXT,
    status         TEXT NOT NULL DEFAULT 'pending' CHECK (status IN
                     ('pending','running','awaiting_approval','done','failed','cancelled','superseded')),
    gate_outcome   TEXT CHECK (gate_outcome IN ('approved','rejected','retried','skipped')),
    gate_note      TEXT,
    selected       BOOLEAN,                      -- fan-out winner; NULL when fan_out = 1
    exit_code      INTEGER,
    prompt_digest  TEXT,                         -- sha256 of session_event seq 0
    trim_record    JSONB,                        -- R-PRM-3: what was trimmed and by how much
    usage          JSONB,                        -- summed from usage events; kept for the Runs tab
    isolation_path TEXT,                         -- worktree / copy location on the executing box
    started_at     TIMESTAMPTZ,
    finished_at    TIMESTAMPTZ,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, position, attempt, fanout_index)
);

CREATE TABLE run_step_commit (                   -- per repo, since a project may have several
    run_step_id  UUID NOT NULL REFERENCES run_step(id) ON DELETE CASCADE,
    repo_id      UUID NOT NULL REFERENCES repo(id),
    before_hash  TEXT NOT NULL,
    after_hash   TEXT,
    PRIMARY KEY (run_step_id, repo_id)
);

-- session_event: §4.3

CREATE TABLE command_run (                       -- R-MCP-3 queue; runtime state lives in Postgres like everything else
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_step_id  UUID NOT NULL REFERENCES run_step(id) ON DELETE CASCADE,
    box_id       UUID NOT NULL REFERENCES box(id),
    class        TEXT NOT NULL,                  -- 'build','test','run',... limits in box.settings.command_limits
    command      TEXT NOT NULL,
    cwd          TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','done','failed','cancelled')),
    exit_code    INTEGER,
    output       TEXT,                           -- scrubbed
    queued_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at   TIMESTAMPTZ,
    finished_at  TIMESTAMPTZ
);
CREATE INDEX idx_command_run_queue ON command_run(box_id, class, status, queued_at);
```

### 5.9 Settings

```sql
CREATE TABLE app_setting (
    key        TEXT PRIMARY KEY,                 -- 'cache_refresh_seconds','cache_overlap_seconds','per_token_cap_run',...
    value      JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Global defaults live here; `project.settings` and `box.settings` override per scope. Settings that
`R-LATER` items need (scheduler window, remote dispatch) are keys, not columns.

### 5.10 Seed on first connect

One `app_user`, the `capability_tag` vocabulary, agents `claude` and `agy` (transport per ANA-4),
default `app_setting` rows. Per new project: the five `item_kind` rows with their default graphs
and phases from the `R-ENT-6` table, and one `prompt_template` version 1 per phase name.

---

## 6. Cache contract

### 6.1 Store trait split

```rust
pub trait ReadStore: Send + Sync {
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> Result<Vec<ItemSummary>>;
    async fn item(&self, id: ItemId) -> Result<Option<Item>>;
    async fn links(&self, id: ItemId, hops: u8) -> Result<LinkGraph>;
    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>>;
    async fn notes(&self, id: ItemId) -> Result<Vec<Note>>;
    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>>;
    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>>; // None = not cached
}
pub trait WriteStore: ReadStore {
    async fn mint_item(&self, new: NewItem) -> Result<Item>;
    async fn update_item(&self, id: ItemId, expected_version: i32, patch: ItemPatch) -> Result<UpdateOutcome>;
    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool>;
    // runs, steps, events, links, notes, documents, skills, templates, box, agents ...
}
pub enum UpdateOutcome { Updated(Item), Diverged { head: Item, ancestor: ItemRevision } }
```

`PgStore: WriteStore`. `CacheStore: ReadStore` only. `MemStore: WriteStore` (MOD-1, tests). The
TUI holds a `Backend` enum { Online(PgStore, CacheStore), Offline(CacheStore) } and every write
path is unreachable in `Offline`, at the type level rather than by a runtime flag.

### 6.2 Refresh algorithm

```
for project in opened_projects:
    for table in [project, repo, item_kind, item, item_link, item_note, document, run, run_step, run_step_commit]:
        hw   = cache_cursor[project, table] or 0
        rows = pg.select(table, where project scope and ts_col > hw - overlap, order by ts_col)
        sqlite.upsert(table, rows)          # PK upsert; item_link rows with deleted_at set are deleted from the mirror
        cache_cursor[project, table] = max(rows.ts_col) if rows else hw
    steps = last N finished run_step ids for project (by finished_at)
    for step not in sqlite.session_event: copy all events for step
    delete session_event rows for steps outside the last N
upload pending/*.jsonl
```

`ts_col` is `updated_at`, or `created_at` for `item_note` and `document`. Workspaces and
`workspace_project` refresh unscoped (small). The own `box` row refreshes unscoped.

### 6.3 Who writes the cache

Only the refresh task. The offline chat buffer writes `pending/*.jsonl`, not `cache.sqlite`.
User actions never touch the cache; when online they go to Postgres and the next cursor pass
mirrors them. That is the whole reason the mirror never diverges: it has one writer and one
direction.

---

## 7. Key queries

### 7.1 Mint an item (`R-ENT-7`)

```sql
WITH c AS (
    INSERT INTO item_key_counter (project_id, prefix, last_value)
    VALUES ($project, $prefix, 1)
    ON CONFLICT (project_id, prefix)
    DO UPDATE SET last_value = item_key_counter.last_value + 1
    RETURNING last_value
), i AS (
    INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, title, body,
                      required_tags, created_by)
    SELECT $id, $project, $kind, $prefix, c.last_value, $title, $body, $tags, $user FROM c
    RETURNING id, version, title, body, required_tags
)
INSERT INTO item_revision (item_id, version, title, body, required_tags, author_id, box_id, reason)
SELECT id, version, title, body, required_tags, $user, $box, 'created' FROM i
RETURNING item_id;
```

Importer variant: `DO UPDATE SET last_value = GREATEST(item_key_counter.last_value, $n)` and an
explicit `key_number = $n`.

### 7.2 Compare-and-set edit (`R-ENT-10`)

```sql
WITH u AS (
    UPDATE item
       SET title = $title, body = $body, required_tags = $tags, version = version + 1
     WHERE id = $id AND version = $expected
    RETURNING id, version, title, body, required_tags
)
INSERT INTO item_revision (item_id, version, title, body, required_tags, author_id, box_id, reason)
SELECT id, version, title, body, required_tags, $user, $box, $reason FROM u
RETURNING version;
-- zero rows → SELECT head FROM item WHERE id = $id; SELECT ancestor FROM item_revision WHERE item_id = $id AND version = $expected
```

### 7.3 Upstream summaries within the workspace (`R-PRM-1..2`)

```sql
WITH RECURSIVE up AS (
    SELECT l.to_item_id AS item_id, 1 AS depth
      FROM item_link l
     WHERE l.from_item_id = $item AND l.kind IN ('blocked_by','origin') AND l.deleted_at IS NULL
    UNION
    SELECT l.to_item_id, up.depth + 1
      FROM item_link l JOIN up ON l.from_item_id = up.item_id
     WHERE up.depth < $hops AND l.kind IN ('blocked_by','origin') AND l.deleted_at IS NULL
),
scope AS (                                        -- active workspace, or the current project when none
    SELECT project_id FROM workspace_project WHERE workspace_id = $workspace
    UNION SELECT $project WHERE $workspace IS NULL
)
SELECT p.slug || ':' || i.key AS qualified_key, i.title, i.status,
       CASE WHEN s.project_id IS NOT NULL THEN d.body END AS summary   -- NULL → one-line stub (R-PRM-2)
  FROM up
  JOIN item i ON i.id = up.item_id
  JOIN project p ON p.id = i.project_id
  LEFT JOIN scope s ON s.project_id = i.project_id
  LEFT JOIN LATERAL (
       SELECT body FROM document WHERE item_id = i.id AND kind = 'summary'
        ORDER BY version DESC LIMIT 1) d ON s.project_id IS NOT NULL
 ORDER BY up.depth, qualified_key;
```

### 7.4 Ready items for a box (`R-ORCH-6`, `R-ORCH-10`)

```sql
SELECT i.*
  FROM item i
  JOIN box b ON b.id = $box
 WHERE i.project_id = ANY($projects)
   AND i.status = 'open'
   AND i.required_tags <@ (b.probed_tags || b.declared_tags)
   AND NOT EXISTS (
       SELECT 1 FROM item_link l JOIN item t ON t.id = l.to_item_id
        WHERE l.from_item_id = i.id AND l.kind = 'blocked_by' AND l.deleted_at IS NULL
          AND t.status NOT IN ('done','closed'))
 ORDER BY i.priority DESC, i.created_at;
```

Missing tags for the refusal message: `SELECT unnest(i.required_tags) EXCEPT SELECT unnest(b.probed_tags || b.declared_tags)`.

### 7.5 Replay a step (`R-HIS-2`)

```sql
SELECT seq, turn, kind, role, tool_call_id, payload, at
  FROM session_event WHERE run_step_id = $step ORDER BY seq;
```

Identical text against the SQLite mirror.

---

## 8. What this replaces

| ANA-1 / ANA-8 element | Fate |
|---|---|
| `repo` as item anchor, `item.repo_id` | Replaced by `item.project_id`; repo is the execution target via `run_step_commit`. |
| `items.json`, `htui-workspace.json`, `sync_state` | Dropped (`R-ID-4`, CONCEPTS "repos stay clean"). Discovery on startup is `repo_box_path` / `workspace_box_path` for the current box, then the switcher. |
| `artifact`, `artifact_link` | Dropped (`R-ENT-13` withdrawn). |
| `external_issue_mapping` | Dropped; `R-LATER-2` adds its own table when its ANA lands. |
| `box.toolchains JSONB`, `env_quirks` | Split into `box_tool` rows, `probed_tags`/`declared_tags`, `quirks`. |
| `run_step.transcript_ref`, `commit_hash` | `session_event` rows; `run_step_commit` per repo. |
| `item_document` | `document`, open `kind` set, `produced_by_step_id`. |
| Timestamp LWW for status | Status is a compare-and-set on `status`; spec is a compare-and-set on `version`. |
| `project.primary_repo_id` | `repo.is_primary` with a partial unique index; no FK cycle. |
| Recursive CTE for prompt injection | Kept, without the facts branch (§7.3). |

---

## 9. Phasing and downstream impact

**MOD-6 - Postgres store + cache** (already open, blocked on this ANA; now unblocked). Scope:

1. `migrations/0001_init.sql` from §5, `sqlx` offline query checking, seed (§5.10), migration
   confirmation prompt on connect (`R-STO-5`).
2. Keyring DSN storage (`keyring` crate: Windows Credential Manager, macOS Keychain, Linux secret
   service), optional TLS (`R-STO-1..2`).
3. `ReadStore` / `WriteStore` traits (§6.1), `PgStore`, `CacheStore`, `MemStore` sharing one test
   suite.
4. Cache directory layout, `cache.sqlite` mirror schema, refresh task (§6.2), rebuild triggers,
   pending chat buffer upload.
5. Offline mode: `Backend::Offline` when the connection fails, with the top bar showing the state.

**Other items this schema shapes:**

- **MOD-1** builds against `MemStore`; the trait in §6.1 is the seam.
- **MOD-2** (driver) owns chunk coalescing and emits `session_event` rows; `agent.launch` and
  `agent.settings` shapes come from ANA-4.
- **MOD-4** (orchestrator) owns the status machines on `run`, `run_step` and `item.status`
  (compare-and-set, never version bumps); ANA-2 may add phase columns by migration `0002`.
- **MOD-7** (box) writes `box`, `box_tool`, tags, paths, `agent_box`.
- **MOD-9** (skills) writes `skill*`, `prompt_template`.
- **MOD-11** (MCP) writes `item_link` (with `proposed_by_step_id`), `item_note.via_step_id`,
  `document.produced_by_step_id`, `command_run`.
- **MOD-8** (legacy import, later) uses the importer mint variant (§7.1) and `reason = 'imported'`.

---

## 10. Risks

| Risk | Mitigation |
|---|---|
| A transaction longer than the overlap window commits a row the cursor never sees | Window is 5 minutes against millisecond writes; the weekly full pass closes any residual gap; `Rebuild cache` is one keystroke. |
| `session_event` grows without bound | Retention sweep per project (`R-HIS-3`); the table has no index beyond the PK and one partial, so inserts stay cheap. Partitioning by month is a `0002` option if a project ever needs it. |
| A kind's prefix is renamed and users expect renumbering | Documented: old keys keep their text, the new prefix starts at 1. The TUI warns on prefix change. |
| Cursor pass and TUI both hit Postgres on connect | The pass is throttled per table and runs on the tokio runtime, not the UI task (`R-NF-3`); the TUI reads from Postgres directly, so a slow pass delays only the mirror. |
| `sqlx` SQLite bundled build fails on a box | Feature-gated: `--no-default-features` builds `PgStore` only and offline mode reports "no cache". The store trait does not change. |
| Postgres 16 minimum excludes an older server | Stated in `README`; the only 15-or-newer feature is `UNIQUE NULLS NOT DISTINCT`, replaceable by a `COALESCE` expression index if a downgrade is ever needed. |

---

## 11. Validation criteria (for MOD-6)

1. `0001_init.sql` applies on a clean Postgres 16 and 17; `sqlx` offline checks pass for every
   query in §7.
2. Two connections minting the same prefix concurrently produce consecutive numbers; a rolled-back
   mint leaves no gap; the importer variant leaves `last_value >= max(key_number)`.
3. Two compare-and-set edits from the same `version` produce exactly one `Updated` and one
   `Diverged` whose `ancestor.version` equals the starting version.
4. A step with 5 000 events replays from Postgres and from the mirror in identical order and content.
5. Warm start (cache present, Postgres reachable) renders the Backlog tab under one second on the
   reference workstation; cold start with Postgres unreachable renders from the cache.
6. A row updated by a transaction held open for two minutes during a refresh pass is present in the
   mirror after the following pass.
7. A chat session started offline lands in Postgres on reconnect with `seq` order preserved and the
   pending file removed; running the upload twice does not duplicate rows.
