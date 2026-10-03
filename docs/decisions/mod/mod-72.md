# MOD-72 - Tool-call chips in the Runs flow view (done, 2026-10-03)

**Requirements:** `R-TUI-4` (Runs sub-tab steps and actions).
**Origin:** MOD-28 (`docs/decisions/mod/mod-28.md` "Carried"), from ANA-12 §3.2. Deferred out of
MOD-28 by the maintainer, 2026-10-02.
**Artifacts:**
- plan [`.claude/plans/mod-72-tool-call-chips.plan.md`](../../../.claude/plans/mod-72-tool-call-chips.plan.md): D1-D8, with its verified-claims table;
- blueprint `.claude/plans/mod-72-tool-call-chips.blueprint.md`: deviations B-1-B-11, hazards H-1-H-11, decisions E1-E17.

Decision numbers are local to MOD-72 (the MOD-31 convention).

Routed as **plan** (0 criteria fired from the item text; C4 borderline). Run in a TOOL-7 sandbox
(`hr/MOD-72`). Read from the tree, the change touches 18 code files, so C4 would have fired; the
path stays plan (PRD needs two), and the tasks were one dependency chain, so they ran serially.

**Decisions (maintainer, 2026-10-03):**
- route accepted, no ultracode;
- plan confirmed as written and fact-checked;
- review: M1, M2, L3 and N6 applied; L4 corrected in the docs; N7, N8 and L5 left as they are
  (cheap double clone, a harmless re-sort, a forward-looking test).

**Commits:**
- plan and blueprint: `d3bcaad4`, `0a12defa`;
- T1 store read: `d1ddbe38` (red), `c38816d9` (MemStore), `f86609d0` (Postgres, mirror, `.sqlx`);
- T2 worker protocol: `3572c0da` (red), `bf8474b3`;
- T3 chips: `86469aee` (red), `e5f8289c`;
- review fixes: `3c59a332` (M1), `7e34aab3` (N6), `09035258` (L3), `be4a04b0` (M2).

---

## What was built

In the Runs flow view (`v`), every step node has a third line that counts the step's tool calls
by kind:

```
┌──────────────────┐
│1.1 done          │
│plan              │
│⚒ read×1          │
└──────────────────┘
```

- The line is `⚒` followed by `label×n` chips. The largest count comes first, and ties go by label.
  It never shows `×0`. Chips that do not fit in the node's 18 interior cells become a trailing
  `+N`, the number of kinds left out, and room for it is reserved. A step with no calls has a blank
  line. The line is drawn in `theme.dim`.
- Labels are the ACP tool kinds, four of them shortened: `read edit del move find exec think fetch
  mode other`. A kind htui does not know is drawn as itself.
- At zoom 0.5 a node is a bare box, as before (review L3).
- The list view is unchanged.

**Data path.**
- `ReadStore::tool_call_counts(item) -> Vec<ToolCallCount { step, tool_kind, calls }>`: one row
  per `(step, tool_kind)` over the item's runs. Only `kind = 'tool_call'` rows count. A payload
  whose `tool_kind` is missing or not a JSON string counts as `other`. Rows are sorted in Rust,
  so the three backends hand back the same bytes.
  - Postgres (`pg/read.rs`, `query!`, one new `.sqlx` entry), the SQLite mirror (`cache/read.rs`,
    `json_type`-guarded `json_extract`), `MemStore`; `Backend`, `Writer` and the two
    `htui-agent` spies delegate.
- `StoreRequest::ToolCalls { item }` → `StoreReply::ToolCalls { item, counts }`, served through
  the ordinary read path. Offline, the mirror answers from its last-N-steps window.
- `RunsTab` asks for it after each `Runs` reply **only while the flow view is shown**, and once on
  `v` into the flow. A reply for another item is dropped; an item change clears the counts; a
  reply re-syncs the graph without moving the viewport.

## Decisions as built (plan D1-D8)

- **D1, group by `tool_kind`.** `claude-cli` titles a call with the tool name (`Bash`), while ACP
  agents title it in prose (`Read src/main.rs`). Grouping by title would give one chip per call on
  ACP. The ANA-12 example `bash ×3` reads `exec×3`.
- **D2, a `ReadStore` method.** `session_event` is mirrored, so the read sits where every backend
  answers it, rather than on `WriteStore` beside `relay_view`. This widened the item text's
  "Postgres plus MemStore" by the mirror.
- **D3, the row.** `tool_kind` is a `String`, because `ToolKind` lives in `htui-agent`, which no
  store crate depends on.
- **D4, timing.** The read is asked next to `RunActions` and `RelayView` in `on_runs`, so the live
  `RunStream` re-read and the active-run poll refresh the chips.
- **D5, node height 4 → 5** for every node, so a running step's first call does not move the
  layout. `Y_STEP` is 8.
- **D6-D8:** the chip format above; the pane keeps the counts by step; chips in the list view,
  tool names, chip clicks (MOD-71) and failed-call colouring are out of scope.

**Deviations (blueprint, review):**
- **B-1.** Three MOD-28 tests asserted that `v` sends nothing; D4 makes it send `ToolCalls`, so
  they were updated.
- **B-2 / E3.** A bare `COALESCE` would have made the backends disagree on a numeric `tool_kind`
  (Postgres `"7"`, SQLite an integer that fails the read). All three check the JSON type.
- **B-5 / E10.** Chips are fitted at draw time to the drawn interior, not precomputed at 18.
- **Review M2.** The mirror's type guard is now exercised by
  `the_mirror_counts_tool_calls_like_postgres` (`htui-store/tests/cache.rs`).
- **Review L3.** With `NODE_H = 5`, rataflow's separate flooring of the top and bottom edges makes
  a zoom-0.5 node 2 or 3 rows tall depending on the pan, so `StepNode` draws no text into a
  one-row interior.
- **Review L4.** The plan's Risks row named `idx_session_event_tool`; that index is partial
  (`WHERE tool_call_id IS NOT NULL`) and the query does not use it. The count reads the item's
  events through `run.item_id` → `run_step` → `session_event`'s primary key, once per `Runs`
  reply while the flow is shown. Acceptable at today's log sizes.

## Gate

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -D warnings` | clean |
| `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` | 107 suites, 3939 passed, 0 failed, 30 ignored; no SIGABRT (on `be4a04b0`) |
| `cargo sqlx prepare --check` (`crates/htui-store`, migrated scratch DB) | exit 0 |

**Pins moved:** store conformance `CASES` 131, `READ_CASES` 15, `StoreRequest` 97, `StoreReply` 56,
319 `.sqlx` files, 135 snapshots (`backlog__runs_flow_tool_chips` is new; the two MOD-28 flow
snapshots gained one blank row per node). `EXPECTED_CASES` and the `CASES` pin are bumped by other
open branches too; on merge, the value is the sum of the bumps.

## Carried

Nothing minted. Possible follow-ups, if wanted: chips in the list view; colouring a kind whose
calls failed (pairing `tool_call` with `tool_result`); per-tool names, which need a title parser
per transport.
