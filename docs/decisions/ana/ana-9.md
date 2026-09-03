# ANA-9 - Data model v2: Postgres schema, key sequences, divergence, event rows, cache (done, 2026-09-03)

## Summary

Concluded the Postgres schema that implements `docs/REQUIREMENTS.md` and the per-box read-only
cache, replacing the ANA-1 and ANA-8 DDL in full. Design in [`docs/ANA-9.md`](../../ANA-9.md).
Requirements addressed: `R-USR-2`, `R-BOX-1..4`, `R-ENT-1..12`, `R-STO-1`, `R-STO-3..6`,
`R-HIS-1..3`, `R-SKL-1..2`, `R-PRM-3..4`, `R-AGT-4`, `R-AGT-6..7`, `R-ORCH-1`, `R-ORCH-9..11`,
`R-MCP-3`.

## What was decided

1. **Per-project key sequences.** Counter table `item_key_counter (project_id, prefix, last_value)`
   incremented by one `INSERT ... ON CONFLICT DO UPDATE ... RETURNING` inside the item's insert
   transaction. `item` stores `key_prefix` and `key_number` with `key` as a stored generated column;
   uniqueness is `(project_id, key_prefix, key_number)`. No delete path for items; counters never
   decrement; the legacy importer aligns with `GREATEST`. Rejected: native sequences per prefix
   (runtime DDL, non-transactional), `max+1` with retry (races, reuse on delete).
2. **Divergence on `version`.** `version` guards the human-edited spec columns (`title`, `body`,
   `kind_id`, `required_tags`, `priority`, `touched_paths`, `step_graph_id`); every edit is a
   compare-and-set that also inserts `item_revision`. Status transitions are a separate
   compare-and-set on `status` and never bump `version`, so orchestrator progress cannot invalidate an
   open edit. Zero rows updated returns `Diverged { head, ancestor }` with the ancestor read from
   `item_revision` at the starting version. Rejected: timestamp last-writer-wins, server-side merge.
3. **Event rows.** `session_event (run_step_id, seq)` primary key, `turn`, `kind`, `role`,
   `tool_call_id`, `payload JSONB`, optional `raw`, `at`. Kind vocabulary is `R-AGT-1` plus
   `prompt`, `follow_up`, `permission_answer`, `plan`, `other` (unmapped ACP updates kept verbatim).
   Payload keys per kind are fixed in the doc. Chunk coalescing is the driver's job (MOD-2). The
   offline free-standing chat buffers rows of the same shape to `pending/<run_id>.jsonl` and uploads
   idempotently on reconnect. Rejected: one transcript blob per step, raw wire rows only.
4. **Cache.** SQLite via `sqlx` at `<config_dir>/htui/cache/<db_fingerprint>/cache.sqlite`, WAL
   mode, single writer (the refresh task), mirror of the offline-browsed tables with identical names,
   plus `cache_meta` and `cache_cursor`. Refresh cursor is an `updated_at` high-water mark per
   `(project, table)` with a fixed overlap window (default 300 s) and idempotent upserts; `item_link`
   removals are tombstones. Rebuild on schema-version mismatch, on demand, or when the last full pass
   is older than seven days. Live reads while connected go to Postgres, not the cache. Rejected: JSON
   files, `rkyv`, `redb`, snapshot-`xmin` tracking, trigger change log, logical replication.
5. **Schema conventions.** UUIDv7 primary keys minted by the app, `TEXT + CHECK` enumerations, one
   `set_updated_at()` trigger using `clock_timestamp()`, Postgres 16 minimum, `sqlx` forward-only
   migrations starting at `0001_init.sql`. Thirty tables covering users, boxes, tools, tags,
   hierarchy, per-box paths, kinds, counters, items, revisions, links, notes, documents, step graphs,
   phases, phase agents, templates, skills, bindings, agents, per-box agent state, runs, steps,
   per-repo commits, events, command queue, settings.
6. **Store trait split.** `ReadStore` implemented by `PgStore`, `CacheStore`, `MemStore`;
   `WriteStore` by `PgStore` and `MemStore` only. The TUI's `Backend` enum makes writes unreachable
   in offline mode at the type level.

## Downstream items

- **MOD-6** (Postgres store + cache) unblocked; scope in `docs/ANA-9.md` §9.
- **MOD-1** builds against `MemStore` behind the §6.1 trait.
- **MOD-2**, **MOD-4**, **MOD-7**, **MOD-9**, **MOD-11**, **MOD-8**: table ownership listed in §9.
- **ANA-2**, **ANA-4**, **ANA-5**, **ANA-7** amend by forward-only migration where they need columns
  this schema only reserves (`step_graph_phase`, `agent.launch`/`settings`, `prompt_template.body`
  contract, `project.secret_*`).

## Commits

- docs(ana-9): conclude data model v2 (this close-out).
