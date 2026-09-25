# ANA-11 - Models for requirements and decisions (concluded, 2026-09-25)

Evaluate database schema models to track product requirements (`R-<AREA>-<N>`) and architectural
decisions (resolved MOD/ANA items) inside `htui`'s Postgres store instead of `docs/REQUIREMENTS.md`,
`DECISIONS.md` and `docs/decisions/**`.

**Requirements.** Dedicated typed tables: `requirement_spec`, `requirement_area`,
`requirement_key_counter`, `requirement` (generated key `R-<AREA>-<N>`, priority `must`/`later`,
state `active`/`withdrawn`, `version` compare-and-set) and `requirement_revision` naming the
deciding item. Rejected alternatives:
- requirements as items of a `REQ` kind: the key shape and status machine don't fit, and
  requirements would leak into the Backlog and `ready_items`;
- a generic node/attribute store: forbidden by the `R-ENT-13` precedent;
- a parsed project document;
- keeping the markdown.

Items cite requirements through `item_requirement` (`addresses`, `amends`, `withdraws`,
`reserves`). Each citation is stamped with the requirement version it was made against, and a newer
version makes it a derived **suspect** link (the Doorstop/Polarion pattern).

**Decisions.** No new entity. A decision is a closed item plus its `summary` (or `verdict`)
document, the `supersedes` link already covers replacement, and `DECISIONS.md` becomes a query. The
one missing field is added as `item.resolution` (`done`, `concluded`, `rejected`, `withdrawn`,
`superseded`, `duplicate`), orthogonal to `status` (Jira's split) and non-null iff `closed`.
`close_out` gains a guarded `open` → `closed` path for non-success resolutions: today an item
withdrawn before it ran cannot close at all. This amends ANA-2 §4.3.

Spawned **MOD-38** (schema `0005_requirements.sql`, seam, cache, close-out resolution). It is gated
on the maintainer applying the requirement amendments proposed in §7 (`R-ENT-14`, `R-ENT-15`, and
amendments to `R-ENT-8`, `R-NF-4`, `R-STO-3`, `R-TUI-1`, `R-MCP-2`). Also spawned **MOD-39**
(Requirements tab, item traceability, resolution picker), and widened **MOD-8** to import the
markdown corpus into the new tables. `htui`'s own repo stays on markdown until MOD-13 and MOD-39
let it run its own lifecycle (MOD-4 is done).

See `docs/ANA-11.md` for the full analysis.
