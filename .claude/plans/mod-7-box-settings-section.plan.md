# Plan: MOD-7 milestone 2 — the maintainer sees and edits it

**Source**: `.claude/prds/mod-7-box-registry.prd.md`, milestone 2 (Delivery Milestones table, row 2,
`:234`): "The Settings `box` section lists boxes with profile, tools and tags; re-probe on this box;
declared tags and multi-line quirks edited as a compare-and-set a reconnect cannot stale." Scope
bullet "Settings `box` section (`R-TUI-8`)" (`:146-149`); success-metric rows "Declared tags and
quirks CAS" (`:119`) and "UI never blocks" (`:123`). Design authority: the PRD's gate decisions
**PRD D3** (quirks get a multi-line widget, small and reusable, not a general editor; `:197-199`)
and **PRD D4** (the section lists every box; declared tags and quirks editable on any box the user
owns; the probe runs on this box only; `:200-201`), with PRD D0–D7 otherwise settled and not
reopened here. Milestone 1's plan and blueprint
(`.claude/plans/mod-7-box-identity-probe.plan.md`, `.blueprint.md`) are the base this milestone
builds on; their decisions D1–D38 stand.

**Requirements**: `R-BOX-3` (declared tags and "a free-form quirks note editable in the TUI",
`docs/REQUIREMENTS.md:71-73`), `R-TUI-8` ("box profile with capability edits", `:314`), `R-NF-3`
(`:343`; no store handle on the render side), ANA-16 C7 (`docs/ANA-16.md:487`: "Any future writer
is a CAS").

**Complexity**: Medium. No migration: milestone 1's `0005_box_identity` already added
`box.edit_version` as this milestone's token (`crates/htui-store/migrations/0005_box_identity.sql:19`,
comment `:25-26`). One new `BoxRow` field; one new `WriteStore` method across five implementations
plus three conformance cases; one new worker module and two new `StoreRequest` and two new
`StoreReply` variants; one new small widget (`TextArea`); one new Settings section. A seventh task,
the `box_probe_spec` editor that milestone 1 promised to this milestone (OQ-18), is planned as a
separable last task and can be cut without touching the others.

**Routing**: routed as **plan** by `/handoff-run MOD-7` (the PRD and its milestone table exist;
milestone 1 is complete). **Staffing: Opus 5.5 for every step — plan, fact-check, architect,
implementers, verifiers and reviewer (`rust-reviewer`); Fable is not used (maintainer standing
instruction).** Ultracode for the implementers only, one workflow per task, verify fan-out per
round; the architect and the reviewer stay plain agents.

**Numbering**: milestone 1's plan used D1–D18, R-1–R-11, OQ-1–OQ-12 and tasks T0–T5; its blueprint
used D19–D38 and R-12–R-19. This plan's decisions are **D39…D53**, risks start at **R-20**, open
questions at **OQ-13**. Tasks restart at **T0** and are always cited as "milestone 2 T*n*" outside
this file. The PRD's gate decisions are cited as **PRD D0…PRD D7**.

**Status**: **draft** (2026-09-25), fact-checked 2026-09-26 (see "Verified claims"; amendments
are marked inline). Branch `mod-7-m2` at `9a4911a`
(main after PR #7, which carries milestone 1 and MOD-38's `0006_requirements`).

**Graphify note**: `graphify-out/` does not exist in this checkout (`ls graphify-out` fails), so
nothing here was read from it. Tree facts were located through the Gortex index and then re-read
at their line in the file itself, because the index is stale for some files MOD-38 touched (for
example Gortex places `Writer::boxes` at `writer.rs:375`; the file has it at `:436`). Every line
number below is the file's, at `9a4911a`, pre-edit.

---

## Open questions for the maintainer (read these first)

Each has a default this plan adopts so implementation is not blocked.

**Answered at the CONFIRM gate (2026-09-26).** The maintainer confirmed the fact-checked plan with
the defaults of OQ-13, OQ-14, OQ-15, OQ-16, OQ-17 and OQ-19, and decided:

- **OQ-18: T6 is deferred** to its own item, **MOD-51** (`box_probe_spec` editor). Milestone 2 is
  T0–T5; the read-only view of the effective spec still ships in T3/T4. Task 6 below stays in this
  plan as the recorded shape of MOD-51 and is **not** implemented here.
- **`ctrl-c` does not quit** (OQ-16's fact-check finding) is tracked as **MOD-52**. This plan keeps
  the pass-through rule and fixes nothing about it.
- **The other-user `NotFound` assertion** moves out of the generic conformance suite to a `mem.rs`
  unit test and `box_identity.rs::another_users_box_is_not_found_by_edit_box`, as amended at
  fact-check; the demo fixture is not extended.

- [ ] **OQ-13 — Where `edit_version` lives in the model.** Milestone 1 kept it off `BoxRow` (D14:
      "every `BoxRow` constructor … would move for a value nothing reads yet") and named
      "`BoxRow.edit_version`" as this milestone's (D15). **Default adopted (D40):** `BoxRow` gains
      `pub edit_version: i32`. The constructors that move are few and all in T0: the `row()`
      helper at `crates/htui-core/src/model/box_.rs:273`, the fixture at
      `crates/htui-core/src/fixtures.rs:459`, the literal in `PgStore::boxes`
      (`crates/htui-store/src/pg/write.rs:1200`) and the two `query_as!(BoxRow, …)` statements in
      `pg/read.rs` (`box_profile`, fn `:1519`, macro `:1520`; `box_row`, fn `:1796`, macro `:1797`;
      amended at fact-check); every other
      `BoxRow { … }` in the tree spreads `..row()` (`box_.rs:316`, `:330`, `:400`) or clones the
      fixture (`conformance.rs:4919`). **Alternative:** a side field `BoxRecord.edit_version`, as
      milestone 1 did for `probe_spec_digest` (D18), which moves no `BoxRow` constructor but gives
      `MemStore` a second side map beside `box_probe_digests` (`mem.rs:123`, a field of the private
      `State` struct at `mem.rs:86` behind `MemStore`'s `state: Arc<RwLock<State>>`, not of
      `MemStore` itself; amended at fact-check) and makes the writer return a `BoxRecord`.
- [ ] **OQ-14 — The syntax of a declared tag.** Nothing constrains tags today: `box.declared_tags`
      is a plain `TEXT[]` (`0001_init.sql:66`) and `capability_tag.tag` is an unconstrained `TEXT`
      key (`:43-47`). **Default adopted (D42):** a declared tag is 1–64 characters of
      `[a-z0-9_-]`, starting with a letter or digit — which every tag the seed derives already is
      (`R-BOX-3`'s ten plus milestone 1's five, `heavy_build` included). The store sorts and
      deduplicates; an invalid tag is refused, never lower-cased on the user's behalf.
      **Alternative:** accept any non-empty, comma-free, whitespace-free string.
- [ ] **OQ-15 — A length cap on quirks.** Quirks go into every prompt's box section
      (`crates/htui-core/src/prompt/render.rs:410-420`). **Default adopted (D43):** no cap in the
      store or the widget; the prompt's own trimming budget is what bounds a prompt.
      **Alternative:** a cap (for example 2 000 characters) enforced by the store as a
      `Constraint`.
- [ ] **OQ-16 — The key that saves a multi-line editor.** `Enter` must insert a newline in a
      multi-line field, and no terminal mode that reports `Ctrl+Enter` is enabled (the crate
      enables neither keyboard enhancement nor bracketed paste). **Default adopted (D44):**
      `ctrl-s` saves, `Esc` cancels, `Enter` breaks the line. `ctrl-s` reaches the program because
      crossterm's raw mode goes through `cfmakeraw`
      (`crossterm-0.29.0/src/terminal/sys/unix.rs:302`), which turns off XON/XOFF flow control, and
      no binding in the crate uses `ctrl-s` today. In fact no `ctrl-` chord is bound at all, not even
      `ctrl-c`: `Keymap::default_global` (`crates/htui/src/keymap.rs:198-238`) binds `q`, `Tab`,
      `BackTab`, `1`–`9`, `?` and the overlay `Esc`, `register_all` adds `w`
      (`crates/htui/src/app/mod.rs:63`), and `ctrl-c` appears only in the chord parser's unit test
      (`keymap.rs:329-330`). Raw mode also clears `ISIG`, so `ctrl-c` raises no `SIGINT`. Sections
      pass `CONTROL` chords to the shell by house rule, and the comments say this is "so `ctrl-c`
      still quits", but at `9a4911a` nothing quits on `ctrl-c` (amended at fact-check).
      **Alternative:** `Esc` leaves the editor and asks "save? y/n".
- [ ] **OQ-17 — Where the tag editor's vocabulary hint comes from.** Milestone 1's OQ-12 said
      "Milestone 2's tag picker can list `capability_tag` together with every tag seen on a box";
      the PRD scope says "declared tags use a comma list over the open vocabulary" (`:149`), and no
      non-test Rust code reads `capability_tag`. `PgStore` only seeds it (`pg/mod.rs:315`), only
      `htui-store/tests/migrations.rs:992` and `:1037` read it, and an unused model type
      `CapabilityTag` already exists (`crates/htui-core/src/model/user.rs:25`, re-exported at
      `model/mod.rs:145`). The alternative's read would return that type instead of a new one
      (amended at fact-check). **Default adopted (D50):** no picker and no new read; the
      tag editor shows one dim line, "seen: …", the sorted union of every probed and declared tag
      on the listed boxes. **Alternative:** a new `capability_tags()` read so the seeded
      declared-only `heavy_build` appears before any box carries it.
- [ ] **OQ-18 — The `box_probe_spec` editor.** Milestone 1's plan, as amended at maintainer review,
      says "Milestone 2 ships the editor (D15)" (OQ-9, `mod-7-box-identity-probe.plan.md:136`; D15,
      `:224`: "a validated writer of that `app_setting` row … and a view of the effective spec").
      The PRD's milestone 2 row does not mention it. **Default adopted (D51, D52):** the *view*
      ships in T3/T4 (the section says whether the stored overlay is in force, the effective
      digest, and on each box whether it was probed under it); the *editor* is T6, the last task,
      with no file another milestone 2 task needs, so answering "not now" drops T6 and nothing else
      and the editor becomes a new MOD item. **Alternative:** drop T6 now and raise the item.
- [ ] **OQ-19 — The section's keys and title.** **Default adopted (D47):** section id `boxes`,
      title `Boxes`, appended last in the strip; `j`/`k` move, `t` edits declared tags, `e` edits
      quirks, `p` probes (this box only), `r` reloads — the MOD-15 sections' `r` means reload
      (`crates/htui/src/ui/tabs/settings/prompt.rs:839`, `kinds.rs:1450`, `hierarchy.rs:1115`),
      while the agents section's `r` means probe (`agents.rs:1185-1204`, the probing arm at `:1196`;
      `:535` is only `refuse_during_login`'s label; amended at fact-check). **Alternative:** `r` probes,
      as in the agents section, and the section has no reload key.

---

## Summary

Milestone 1 made every box row true and left it invisible: `WriteStore::boxes` exists
(`crates/htui-core/src/store/traits.rs:391`) but no `StoreRequest` serves it, `StoreRequest::ProbeBox`
exists but its own doc says "Milestone 1 binds it to no key" (`crates/htui/src/store_worker.rs:205-209`),
and nothing writes `declared_tags`, `quirks` or `edit_version` — registration never writes them
(`crates/htui-store/src/pg/mod.rs:393-395`) and neither does the probe (`traits.rs:370-374`). The
Settings tab expects the section by name (`crates/htui/src/ui/tabs/settings/mod.rs:4`, "MOD-7's box
profile") and has room for it: six sections cost 54 of the 100 columns the strip test pins
(`crates/htui/tests/settings.rs:948-965`; titles `Agents`, `Hierarchy`, `Kinds`, `Prompt`,
`Connection`, `Qdrant`, each `chars + 2`).

**The token (T0, T1).** `box.edit_version` (`0005_box_identity.sql:19`) is bumped by the new writer
and by nothing else, so a reconnect — which rewrites `hostname`, `last_seen_at` and, through the
`set_updated_at` trigger, `updated_at` — and a registration probe — which rewrites every probe column
— leave an open editor's token valid. `BoxRow` gains the field (OQ-13); `WriteStore::edit_box(id,
expected, BoxEdit)` is an integer compare-and-set in the shape MOD-38's `set_requirement_spec`
already uses in its `Some(version)` branch (`crates/htui-store/src/pg/write.rs:4481-4500`: `… SET
…, version = version + 1 WHERE … AND version = $2 RETURNING …`, else `cas_miss` at `:4505`; the
function's `None` branch, `:4462-4480`, is an `INSERT … ON CONFLICT DO NOTHING` a box row never
needs, since the row always exists; amended at fact-check), answering `CasOutcome<BoxRow>`
(`traits.rs:1598`). It writes only the fields its `BoxEdit` names.

**The widget (T2).** `TextArea`, a small multi-line sibling of `TextField`
(`crates/htui/src/ui/text_field.rs`, single-line by design, `:1-7`), reusing its `FieldOutcome`
(`:25-36`); `ctrl-s` saves (OQ-16).

**The worker side (T3).** `crate::box_settings`, a serve module in the shape of
`crate::prompt_settings` (`crates/htui/src/prompt_settings.rs:137-256`) and `crate::hierarchy`
(`crates/htui/src/hierarchy.rs:169-173`, which resolves this box through `backend.box_info()`):
`StoreRequest::Boxes` and `StoreRequest::EditBox` in, `StoreReply::Boxes` and `StoreReply::BoxesStale`
out, one or-ed arm in `try_serve` (`store_worker.rs:975`).

**The section (T4).** `BoxesSection`: a list of every box of this user with this box marked, and
for the selected box its profile, tools, probed and declared tags, quirks and last probe; `t`, `e`,
`p`, `r`. The editors keep the token they opened on; a plain reload never refreshes it, a
`BoxesStale` does and says so (the kinds section's rule, `kinds.rs:1085-1135`).

**Postgres end to end (T5)** proves the metric with a real reconnect (`register_box`) and a real
registration probe between the read and the write. **T6** is the optional `box_probe_spec` editor
(OQ-18).

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D39 | **The compare-and-set token is `box.edit_version`** (PRD open question "Which token the declared-tags and quirks CAS compares", `:254`; milestone 1 OQ-4/D14). Confirmed in the tree: the column is `INTEGER NOT NULL DEFAULT 0` (`0005_box_identity.sql:19`), commented "bumped by them only. Registration and the probe never write it, so a reconnect cannot stale an open editor" (`:25-26`); `register_box`'s doc says `edit_version` is "never written by a reconnect" (`pg/mod.rs:393-395`); `record_box_probe`'s doc lists it among the columns it never writes (`traits.rs:370-374`). The new writer bumps it by exactly one on every applied write (`edit_version = edit_version + 1`); nothing else in this milestone writes it. | A token only human edits move is the PRD constraint "Every new box writer is a compare-and-set keyed on a token a reconnect does not bump" (`:219`). `updated_at` would go stale on every connect: the `set_updated_at` trigger fires on every `UPDATE box` (`0001_init.sql:575-580`). |
| D40 | **`BoxRow.edit_version: i32`** (OQ-13), documented as the editors' token. T0 moves every constructor listed in OQ-13 and the three statements that build a `BoxRow` from Postgres (`box_profile`, `box_row`, `boxes`), which changes three `.sqlx` hashes and no file count. The SQLite mirror is not touched: `refresh_box` names its columns (`crates/htui-store/src/cache/refresh.rs:603-623`, `:625-640`), the mirror reads `box` only for `box_info` (`cache/read.rs:1317-1337`; amended at fact-check), and `edit_version` is read online only; `MIRRORED_TABLES` and `cache_migrations/` do not move. The demo loader's `INSERT INTO box` names its columns (`crates/htui-store/src/pg/demo.rs:65-71`) and the fixture's `edit_version` is `0`, the column default, so it is not touched either. | One field on the row type is what the section, the writer and the conformance case all read; a side field would need a second `MemStore` side map. |
| D41 | **The writer.** `WriteStore` (`traits.rs:257`) gains, after `boxes` (`:391`), `async fn edit_box(&self, id: BoxId, expected: i32, edit: BoxEdit) -> Result<CasOutcome<BoxRow>>`, with `pub struct BoxEdit { pub declared_tags: Option<Vec<String>>, pub quirks: Option<String> }` in `htui-core`'s `model::box_`. It writes `declared_tags` when `Some`, `quirks` when `Some`, and `edit_version + 1`, in one statement, and **never** `hostname`, the probe columns, `htui_version`, `settings`, `machine_fingerprint`, `probe_spec_digest`, `last_seen_at` or `box_tool` (the narrow-writer rule, MOD-2 D74, `traits.rs:253-270` region). Postgres: `UPDATE box SET declared_tags = COALESCE($3, declared_tags), quirks = COALESCE($4, quirks), edit_version = edit_version + 1 WHERE id = $1 AND user_id = $5 AND edit_version = $2 RETURNING <the BoxRow columns>`; no row back → `cas_miss` (`pg/write.rs:77`) over `box_row`, filtered to this user. **Answers:** `Applied(row as written)`; `Stale(row as it is now)` for a spent token; `StoreError::NotFound { entity: "box", .. }` for an unknown id **or a box of another `app_user`** (the same user filter `boxes()` applies, `pg/write.rs:1160`; amended at fact-check). The "filtered to this user" re-read after a miss either filters `box_row`'s result in Rust (no new statement) or adds a query (`.sqlx` +2 in T1 instead of +1); T1 prefers the former (amended at fact-check); `StoreError::Constraint` for a tag D42 refuses. **Precedence** NotFound, then Stale, then Constraint — the order the kinds case already pins ("a spent token and an invalid prefix together answer Stale, not Constraint", `crates/htui-core/src/store/conformance.rs:3024-3027`). An empty `BoxEdit` is legal and still bumps (the section never sends one, D48). `BoxEdit` derives at least `Debug`, `Clone`, `PartialEq` and `Eq`, because `StoreRequest` derives `Debug, Clone` (`store_worker.rs:92`) and T3 carries it in `EditBox` (amended at fact-check). Implemented by all five `WriteStore` implementations: `MemStore` (`crates/htui-core/src/store/mem.rs:5130`), `PgStore` (`pg/write.rs:586`), `Writer` (`crates/htui-store/src/writer.rs:349`), `UsageSpy` (`crates/htui-agent/src/conformance.rs:709`) and `SpyStore` (`crates/htui-agent/tests/recorder.rs:389`). | ANA-16 C7 and the PRD constraint; `CasOutcome` is the house type for "the row as it is now" (`traits.rs:1592-1603`). The user filter keeps the writer's reach equal to the list's: PRD D4's "any box the user owns". |
| D42 | **The tag rule is one function in `htui-core`** (OQ-14): `pub fn is_declared_tag(tag: &str) -> bool` (1–64 chars of `[a-z0-9_-]`, first char `[a-z0-9]`) and `pub fn declared_tags_from_text(text: &str) -> Result<Vec<String>, String>` (split on `,`, trim, drop empties, validate, sort, deduplicate; the error is one sentence naming the first bad tag). Both stores call the same validator before writing and store the sorted, deduplicated list, so `Constraint` carries the same sentence on `MemStore` and Postgres. The section parses with the same function before sending, so the user sees the sentence without a round trip, and the store still refuses on its own. | "The map is data" (PRD D2) does not reach tag *syntax*; one rule in the crate every other crate already depends on keeps the section and the two stores from disagreeing. |
| D43 | **Quirks are free text** (OQ-15). Stored as given, lines separated by `\n` (the widget never produces `\r`); no cap. The prompt already collapses newlines to `; ` and drops blank lines (`render.rs:410-420`), which is why a multi-line note is the right shape (PRD D3). | `R-BOX-3`: "a free-form quirks note editable in the TUI". |
| D44 | **`TextArea` (PRD D3)**, `crates/htui/src/ui/text_area.rs`, re-exported from `crate::ui` beside `TextField` (`crates/htui/src/ui/mod.rs:7`, `:11`). Hard lines only (no soft wrap), counted in `char`s like `TextField`; cursor `(row, col)`. **Keys:** a printable `Char` inserts; `Enter` splits the line; `Backspace` at column 0 joins with the previous line and `Delete` at the end joins the next; `Left`/`Right` cross line ends; `Up`/`Down` keep the column, clamped; `Home`/`End` are per line; **`ctrl-s` → `FieldOutcome::Submit`** (OQ-16); `Esc` → `Cancel`; every other chord, `Tab` and `F(n)` → `Pass`, so `ctrl-c` still reaches the shell (the carve-out `hierarchy.rs:677-682` documents). The shell binds nothing to `ctrl-c` today, so the `Pass` follows the house rule and does not by itself make `ctrl-c` quit (OQ-16; amended at fact-check). **API:** `new()`, `with_text(&str)`, `on_key(KeyEvent) -> FieldOutcome`, `text() -> String` (lines joined by `\n`), `is_empty()`, `line_count()`, `lines(width: u16, height: u16, focused: bool, theme: &Theme) -> Vec<Line<'static>>` — a vertical window that keeps the cursor row visible and, on the cursor row, `TextField::line`'s horizontal window with its leading `…`; other rows are clipped with a trailing `…`. That window lives inside `pub fn line` (`text_field.rs:225`) and `TextField` has no public way to set its cursor, so T2 re-implements the window in `text_area.rs` and leaves `text_field.rs` untouched. Extracting a shared helper instead would add `text_field.rs` to T2's files; it would still be disjoint from T1 and T3 (amended at fact-check). A hand-written `Debug` prints `line_count` and `len` only, never the text (the rule `TextField`'s `Debug` follows, `text_field.rs:53-64`; amended at fact-check). No history, no selection, no undo, no mask, no `Zeroizing` (quirks are not secret: they go into every prompt). Without bracketed paste a pasted block arrives as keys, and its line breaks arrive as `Enter`, which this widget turns into newlines. | "Small and reusable, not a general editor" (PRD D3). Reusing `FieldOutcome` means a section treats both widgets alike. |
| D45 | **`crate::box_settings`**, the worker side, in the shape of `crate::prompt_settings` (`:170-256`). `pub struct BoxesSnapshot { pub this_box: Option<BoxId>, pub boxes: Vec<BoxRecord>, pub spec: SpecView }`, where `pub struct SpecView { pub digest: String, pub overlay: bool, pub error: Option<String> }` is the effective probe spec as milestone 1's `htui_agent::box_probe::spec::effective(spec::seed(), stored)` computes it (`crates/htui-agent/src/box_probe/spec.rs:130`, `:137`) over the stored `box_probe_spec` value from `backend.app_settings()` (`crates/htui-store/src/backend.rs:397`; key `spec::SETTING_KEY`, `spec.rs:46`). `snapshot` reads `backend.box_info()` for this box's id (as `hierarchy::serve` does, `hierarchy.rs:173`), `writer.boxes()`, and `app_settings()`. `serve(backend, request)`: `writer()` or `StoreError::Unreachable(DATABASE_UNREACHABLE)` (`backend.rs:153-159`; `writer.rs:143`); `Boxes` re-reads; `EditBox` calls `edit_box` and answers `Boxes` on `Applied`, `BoxesStale` on `Stale`, and **also `BoxesStale` on `NotFound { entity: "box" }`**, so a box deleted by hand under an open editor reaches the section as a snapshot without it (D48's `DELETED_ELSEWHERE`) rather than as a bare failure. `pub const REQUEST_NAMES: [&str; 2] = ["boxes", "edit_box"]` and `READ_NAME` as `prompt_settings.rs:249-256` has them. The residue `prompt_settings.rs:8-11` records (a failed re-read after an applied write answers `Failed`) is carried and documented. | One read per event, one reply out, the section renders only the last snapshot (`prompt_settings.rs:4-6`). Computing `SpecView` here keeps the render side free of `htui_agent` calls and gives milestone 1 D15's "view of the effective spec" whether or not T6 ships. |
| D46 | **Requests and replies.** `StoreRequest::Boxes` (unit) and `StoreRequest::EditBox { box_id: BoxId, expected: i32, edit: BoxEdit }`; `StoreReply::Boxes(Box<BoxesSnapshot>)` and `StoreReply::BoxesStale(Box<BoxesSnapshot>)`, each with a doc in the house style (`store_worker.rs:745-753`). `name()` (`:550`) answers `"boxes"` and `"edit_box"`; `try_serve` gains one or-ed arm `StoreRequest::Boxes \| StoreRequest::EditBox { .. } => box_settings::serve(backend, request).await?`, or-ed rather than guarded for the E0004 reason the neighbouring arms state (`:1021-1025` and `:1038-1040`; amended at fact-check). Both are served in the loop like the catalogue requests, not by the agent runtime; the harness's runtime-routed list (`crates/htui/src/testkit.rs`) is unchanged. The freshness gate is keyed on `(origin, discriminant)` (`crates/htui/src/app/state.rs:283-296`, `:304-308`), so `Boxes`, `EditBox` and `ProbeBox` from the Settings tab never supersede one another. `StoreRequest` goes 64 → 66, `StoreReply` 35 → 37. `ProbeBox`'s doc (`:205-209`, "Milestone 1 binds it to no key") is corrected. | The catalogue and prompt settings pairs (`Catalogue`/`CatalogueStale`, `PromptSettings`/`PromptSettingsStale`, `:747-768`) are the precedent. |
| D47 | **`BoxesSection`** (`crates/htui/src/ui/tabs/settings/boxes.rs`), `SectionId("boxes")`, title `Boxes` (OQ-19), registered **last** in `register_all` (`crates/htui/src/app/mod.rs:49-58`, whose comment already says appending moves no existing section). It holds no store handle and no `BoxId` of its own beyond what snapshots carry (`R-NF-3`; the contract of `SettingsSection`, `settings/mod.rs:130-134`). `wants_requests` → `[Boxes]` whatever the scope (boxes are per user, not per workspace); `on_scope_change` drops nothing. **Layout:** a list pane (one row per box: hostname, `(this box)` on this box, and — only when two listed boxes share a hostname — the last eight hex digits of the id, the PRD's collision mitigation, `:262`) and a detail pane for the selected box: OS family and version, arch, CPU, RAM, GPU, `htui_version`, last probe (or "never probed"), whether it was probed under the effective spec (`BoxRecord.probe_spec_digest == SpecView.digest`), probed tags, declared tags, quirks (multi-line), and tools as `name version`; a hint line; a notice line. Offline or refused, one line in the style of the prompt section's `UNAVAILABLE` (`prompt.rs:56`). **Keys (browse):** `j`/`k`/`Down`/`Up`; `t` opens the tag editor; `e` opens the quirks editor; `p` sends `StoreRequest::ProbeBox` **only on this box** — on any other box it sends nothing and says "the probe runs on this box only" (PRD D4); `r` re-sends `Boxes`. `captures_input` is true exactly while an editor is open (`settings/mod.rs:141-149`). `SettingsTab::wants_requests` (`settings/mod.rs:304-313`) collects every section's reads, so on an offline backend opening Settings through `register_all` now also puts `Failed { request: "boxes", .. }` on the status line (`app/update.rs:171`); no current test does this, so nothing breaks, and the section's own `UNAVAILABLE` line stays the primary signal. `connection.rs`'s `the_product_registers_connection_after_prompt` (about `:2046`) walks the production strip with four `l` presses and still reaches `Connection` with `Boxes` last; its doc ("five sections", "the fifth `l` would wrap") is already stale and T4 may correct it (amended at fact-check). | PRD D4 and `R-TUI-8`. Appending last keeps every existing snapshot's strip where it is: every snapshot that draws a strip builds its own section list (`kinds__demo.snap:8` shows `Agents  Hierarchy  Kinds`), none draws the production list. |
| D48 | **The editors' compare-and-set behaviour** (the kinds section's rules, `kinds.rs:1085-1135`, and the shared sentences, `settings/mod.rs:100-118`). An editor records the box id and the `edit_version` it opened on. **A plain `Boxes` reply never touches that token** — whether it answers `r`, activation, the re-read after a probe, or the re-read the shell issues after a reconnect — it replaces the list and leaves the typed text and the token alone; that is the render-side half of "a reconnect cannot stale", the store-side half being D39. `Enter` (tags) or `ctrl-s` (quirks) sends `EditBox` with **only the edited field** set, and only when the text differs from what the editor opened on (unchanged text closes the editor with no request, the kinds section's "only a user who typed writes it", `kinds.rs:1118-1128`). `Boxes` after a write closes the editor. `BoxesStale`: the editor keeps its text, takes the current row's token, and says `CHANGED_ELSEWHERE`; with no editor open it says `CHANGED_ELSEWHERE_CLOSED`; a snapshot without the box closes the editor with `DELETED_ELSEWHERE`. A tag list `declared_tags_from_text` refuses keeps the editor open with the sentence and sends nothing. `Failed` for a name in `REQUEST_NAMES` goes to the notice line, as `prompt.rs:859-870` does. | PRD metric "a concurrent edit is reported as changed elsewhere, never overwritten; a reconnect does not stale an open editor" (`:119`). |
| D49 | **`p` is milestone 1's `ProbeBox`, unchanged.** The runtime already refuses offline (`REGISTRY_ON_SERVER_ONLY`), with no box row, with a claim held (`BOX_PROBE_RUNNING`, `crates/htui/src/agent_worker.rs:92-93`) — all "before anything is spawned" (`agent_worker.rs:1181-1204`, `tokio::spawn` at `:1195`; amended at fact-check) — and answers once with `StoreReply::BoxProbed` at the requester's address. The shell already renders every `BoxProbed` on the status line above the freshness gate (`crates/htui/src/app/update.rs:264`). The section's only additions: on a `BoxProbed` it re-sends `Boxes` (the report carries counts, not rows, `agent_worker.rs:103-120`), and a `Failed { request: "probe_box", .. }` goes to its notice line. No runtime file moves. | PRD D4: "the probe runs on this box only"; milestone 1 D15 exposed `ProbeBox` for exactly this. |
| D50 | **No tag picker** (OQ-17). The tag editor is a `TextField` pre-filled with the declared tags joined by `, `, above one dim line "seen: …", the sorted union of every probed and declared tag on the listed boxes. No `capability_tag` read is added. | PRD scope: "a comma list over the open vocabulary" (`:149`); `R-BOX-3`: "Vocabulary is open". |
| D51 | **The effective-spec view ships regardless of T6** (OQ-18): the detail pane's "probed under the current spec: yes/no" and one line for the spec — `seed` or `seed + stored overlay`, the first twelve hex digits of the digest, and `SpecView.error` when a stored overlay was ignored. | Milestone 1 D15's "a view of the effective spec", at no store cost (D45 reuses `app_settings()`). |
| D52 | **T6, the `box_probe_spec` editor** (only if OQ-18 keeps it). **Store:** `WriteStore` gains `async fn box_probe_spec(&self) -> Result<Option<StoredSetting>>` (the row's value and `updated_at`, reusing `StoredSetting`, `traits.rs:1733-1740`) and `async fn set_box_probe_spec(&self, overlay: Option<Value>, expected: Option<DateTime<Utc>>) -> Result<CasOutcome<Option<StoredSetting>>>`: `Some` + `None` inserts when no row exists (else `Stale`), `Some` + `Some(t)` updates where `updated_at = t`, `None` + `Some(t)` deletes where `updated_at = t`, `None` + `None` is a `Constraint`; a value that is not a JSON object is a `Constraint`. It is keyed by a new `htui_core` constant `BOX_PROBE_SPEC_KEY = "box_probe_spec"`, declared in `model::box_` and re-exported from `crates/htui-core/src/model/mod.rs` the way T0 re-exports `BoxEdit` (so T6 also edits `model/mod.rs`; amended at fact-check), and `htui_agent::box_probe::spec::SETTING_KEY` (`spec.rs:46`) becomes an alias of it, so the store (which must not depend on `htui-agent`) and the probe cannot disagree on the key. The generic `set_setting` is not used: its keys are the closed `SettingKey` enum (milestone 1 D17). **Validation is the probe's own:** `box_settings::serve` runs `spec::effective(spec::seed(), Some(&overlay))` and refuses with its `error` sentence (as a `Constraint`) before any write, so the editor can never store an overlay the probe would ignore. **Requests:** `StoreRequest::SetProbeSpec { overlay: Option<Value>, expected: Option<DateTime<Utc>> }` → `Boxes`/`BoxesStale`; `BoxesSnapshot.spec` gains the stored row and its token. **Section:** `s` opens a `TextArea` over the stored overlay, pretty-printed, or empty; `ctrl-s` parses it as JSON on the render side (a parse error stays in the editor) and sends it; empty text clears the row. After a save the section says the next `p` (or the next connect) re-probes under it (milestone 1 D18). Two conformance cases; `StoreRequest` 66 → 67; `CASES` 71 → 73. | Milestone 1's maintainer-amended D15. The `TextArea` is the same small widget; JSON editing stays "not a general editor" because validation, not the widget, carries the schema. |
| D53 | **Not changed, on purpose:** no migration (the next stays `0007`); `BoxInfo` and the top bar (the section reads `BoxRecord`; milestone 1 D15's "`BoxInfo`'s missing fields" are not needed by anything this milestone renders); `BoxProfile` and the prompt; the SQLite mirror; `htui-orch` (milestone 3); `repo_box_path`, inference and excerpts (milestone 4); `box.settings` and caps (MOD-12, PRD out of scope `:167-168`); box deletion or merge (`:176`); the agent runtime. | The PRD's out-of-scope list. |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| An integer-version compare-and-set on one row, `Applied`/`Stale`, `cas_miss` | `PgStore::set_requirement_spec` and its `MemStore` twin | `crates/htui-store/src/pg/write.rs:4481-4500` (the `Some(version)` branch of `:4454-4511`; amended at fact-check), `cas_miss` `:77`; `crates/htui-core/src/store/mem.rs` `State::set_requirement_spec` |
| A narrow writer that names what it never writes | `record_box_probe`'s doc | `crates/htui-core/src/store/traits.rs:370-383` |
| A conformance case over a CAS writer, spent token and invalid input together | `update_item_kind` cases | `crates/htui-core/src/store/conformance.rs:3010-3035` |
| Adding a case: the list, the `run_case` arm, the two pins | milestone 1's three box cases | `conformance.rs:96-98`, `:203-209`; `crates/htui-core/tests/mem_store.rs:36-37`; `crates/htui-store/tests/pg_conformance.rs:19` |
| A worker serve module: snapshot, serve, `REQUEST_NAMES`, `READ_NAME` | `crate::prompt_settings` | `crates/htui/src/prompt_settings.rs:137-256` |
| Resolving this box inside a serve module | `crate::hierarchy::serve` | `crates/htui/src/hierarchy.rs:169-173` |
| An or-ed `try_serve` arm | the catalogue arm | `crates/htui/src/store_worker.rs:1038-1049` |
| A section with an editor that survives a CAS miss | `KindsSection::on_stale` | `crates/htui/src/ui/tabs/settings/kinds.rs:1085-1135` |
| The shared CAS sentences and helpers | `CHANGED_ELSEWHERE`, `CHANGED_ELSEWHERE_CLOSED`, `DELETED_ELSEWHERE`, `is_error`, `wrapped` | `crates/htui/src/ui/tabs/settings/mod.rs:65`, `:78`, `:103-118` |
| A single-line widget, its keys, its window, its redacting `Debug` | `TextField` | `crates/htui/src/ui/text_field.rs:25-64` (amended at fact-check), `:113-165`, `:225-260` |
| Section tests without a store: keys in, requests out | `SectionBench` | `crates/htui/src/testkit.rs:579-600` |
| A worker-then-section test file with snapshots | `tests/prompt_settings.rs` | `crates/htui/tests/prompt_settings.rs:1-20` |
| A Postgres stack with a runtime, `swap()` and `register_box` | `Stack` | `crates/htui/tests/box_probe_pg.rs:155-257`, `:382-400` |
| A reconnect as the store makes it | `register_box` in `a_renamed_box_keeps_its_row_and_its_probe` | `crates/htui/tests/box_probe_pg.rs:382-400` |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/model/box_.rs` | edit | T0 | `BoxRow.edit_version`; `BoxEdit`; `is_declared_tag`, `declared_tags_from_text` and unit tests (D40–D42); the `row()` helper `:273` |
| `crates/htui-core/src/model/mod.rs` | edit | T0, T6 | re-export `BoxEdit` and the two functions; T6: re-export `BOX_PROBE_SPEC_KEY` (D52; amended at fact-check) |
| `crates/htui-core/src/fixtures.rs` | edit | T0 | `edit_version: 0` in the demo box (`:459`) |
| `crates/htui-store/src/pg/read.rs` | edit | T0 | `box_profile` (`:1519`) and `box_row` (`:1796`) select `edit_version` |
| `crates/htui-store/src/pg/write.rs` | edit | T0, T1, T6 | T0: `boxes` (`:1151`) selects it and fills the literal (`:1200`); T1: `edit_box`; T6: the spec pair |
| `crates/htui-store/.sqlx/` | regenerate | T0, T1, T6 | T0: three hashes move; T1: +1; T6: about +4 |
| `crates/htui-core/src/store/traits.rs` | edit | T1, T6 | `edit_box` (D41); T6: `box_probe_spec`, `set_box_probe_spec` (D52) |
| `crates/htui-core/src/store/mem.rs` | edit | T1, T6 | both on `MemStore`; T1 also holds the unit test for "another user's box is `NotFound`" (amended at fact-check) |
| `crates/htui-store/src/writer.rs` | edit | T1, T6 | dispatch arms |
| `crates/htui-core/src/store/conformance.rs` | edit | T1, T6 | three cases (T1), two (T6); `CASES` 68 → 71 → 73 |
| `crates/htui-core/tests/mem_store.rs` | edit | T1, T6 | pin (`:36-37`) |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T1, T6 | `EXPECTED_CASES` (`:19`) |
| `crates/htui-agent/src/conformance.rs` | edit | T1, T6 | `UsageSpy` forwards |
| `crates/htui-agent/tests/recorder.rs` | edit | T1, T6 | `SpyStore` forwards |
| `crates/htui-store/tests/box_identity.rs` | edit | T1 | a reconnect and a probe leave `edit_version` alone on Postgres; another user's box is `NotFound` (amended at fact-check) |
| `crates/htui/src/ui/text_area.rs` | create | T2 | `TextArea` and its unit tests (D44) |
| `crates/htui/src/ui/mod.rs` | edit | T2 | `pub mod text_area;` and re-export (`:7`, `:11`) |
| `crates/htui/src/box_settings.rs` | create | T3, T6 | snapshot, serve, `SpecView`, `REQUEST_NAMES` (D45); T6: `SetProbeSpec` |
| `crates/htui/src/lib.rs` | edit | T3 | `pub mod box_settings;` (`:12-28`) |
| `crates/htui/src/store_worker.rs` | edit | T3, T6 | the two requests and two replies, `name()`, `try_serve` arm, `ProbeBox` doc (D46); T6: `SetProbeSpec` |
| `crates/htui/tests/box_settings.rs` | create | T3, T4, T6 | T3: worker half over `MemStore`, offline; T4: section half; T6: spec editor |
| `crates/htui/src/ui/tabs/settings/boxes.rs` | create | T4, T6 | `BoxesSection` (D47–D51); T6: `s` |
| `crates/htui/src/ui/tabs/settings/mod.rs` | edit | T4 | `pub mod boxes;`, `pub use boxes::BoxesSection;` (`:11-17`, `:32-37`); module doc `:3-6` names the section |
| `crates/htui/src/app/mod.rs` | edit | T4 | `Box::new(BoxesSection::new())` last (`:49-58`) |
| `crates/htui/tests/settings.rs` | edit | T4 | the strip test gains the seventh section; its stale doc (`:942-946`) rewritten to 7 sections and 61 columns |
| `crates/htui/tests/snapshots/box_settings__*.snap` | create | T4, T6 | the section's frames |
| `crates/htui/tests/box_probe_pg.rs` | edit | T5 | the reconnect-between-read-and-write cases on Postgres |
| `crates/htui-agent/src/box_probe/spec.rs` | edit | T6 | `SETTING_KEY` aliases `htui_core`'s constant (D52) |

**Not touched, on purpose:** every migration and `cache_migrations/`; `crates/htui-store/src/cache/**`;
`crates/htui-store/src/pg/demo.rs`; `crates/htui-store/src/pg/mod.rs` (registration); `BoxInfo` and
`crates/htui/src/ui/top_bar.rs`; `crates/htui/src/agent_worker.rs` and `crates/htui/src/testkit.rs`
(D49); `crates/htui/src/app/update.rs` (`BoxProbed` is already on the status line); `htui-orch`;
`crates/htui/src/ui/text_field.rs` (T2 copies its window logic rather than extracting a helper,
D44); `crates/htui-store/tests/migrations.rs` (it reads `edit_version` and `capability_tag` by raw
SQL and pins the column comments, none of which move); `docs/**`, `HANDOFF.md`, the PRD (the main
thread records deviations). (Last two items amended at fact-check.)

## Tasks

**Order.** T0 alone first. Then **Wave A**: lane 1 is T1 then T3 (serial, one worktree); lane 2 is
T2 (own worktree). Merge T1, T2, then T3, re-running the touched crates' gates on the real tree after
each merge. Then **Wave B**: T4 and T5 in parallel, each in its own worktree. Then T6 (optional,
OQ-18). Independence is decided by intersecting the file sets below and by build coupling: a red or
mid-edit commit in a dependency crate stops every dependent crate compiling, which is why each
parallel task runs in its own worktree.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T0 | `crates/htui-core/src/model/box_.rs`, `crates/htui-core/src/model/mod.rs`, `crates/htui-core/src/fixtures.rs`, `crates/htui-store/src/pg/read.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/.sqlx/` | first, alone, until green |
| T1 | `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/writer.rs`, `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/tests/box_identity.rs`, `crates/htui-store/.sqlx/`, `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` | Wave A lane 1, after T0 |
| T2 | `crates/htui/src/ui/text_area.rs`, `crates/htui/src/ui/mod.rs` | Wave A lane 2, own worktree, parallel with T1 and T3 |
| T3 | `crates/htui/src/box_settings.rs`, `crates/htui/src/lib.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/tests/box_settings.rs` | Wave A lane 1, serial after T1 |
| T4 | `crates/htui/src/ui/tabs/settings/boxes.rs`, `crates/htui/src/ui/tabs/settings/mod.rs`, `crates/htui/src/app/mod.rs`, `crates/htui/tests/settings.rs`, `crates/htui/tests/box_settings.rs`, `crates/htui/tests/snapshots/box_settings__*.snap` | Wave B, own worktree, after T2 and T3 merged |
| T5 | `crates/htui/tests/box_probe_pg.rs` | Wave B, own worktree, parallel with T4, after T3 merged |
| T6 | `crates/htui-core/src/model/box_.rs`, `crates/htui-core/src/model/mod.rs` (amended at fact-check), `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/writer.rs`, `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/.sqlx/`, `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs`, `crates/htui-agent/src/box_probe/spec.rs`, `crates/htui/src/box_settings.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/ui/tabs/settings/boxes.rs`, `crates/htui/tests/box_settings.rs`, `crates/htui/tests/snapshots/box_settings__*.snap` | serial, last, optional |

**Intersections, checked.** T0 ∩ T1 = `{pg/write.rs, .sqlx/}` — serial. T1 ∩ T2 = ∅ and T3 ∩ T2 =
∅: T2 owns `ui/text_area.rs` and `ui/mod.rs`, T3 owns `lib.rs`, `store_worker.rs`,
`box_settings.rs` and its test file; `crates/htui/src/lib.rs` declares `pub mod ui;` and never
lists `ui`'s children, so T2 does not touch it. T3 ∩ T4 = `{tests/box_settings.rs}` — serial. T4 ∩
T5 = ∅ (T5 edits only `box_probe_pg.rs`, which no other milestone 2 task touches). T6 intersects
T0, T1, T3 and T4 — last. **Hidden coupling checked:** `.sqlx/` moves in T0, T1 and T6 only, all in
the serial lane, and each runs `cargo sqlx prepare` on a tree that already holds its predecessor;
the `StoreRequest`/`StoreReply` enums move in T3 and T6 only; `CASES` and its two pins move in T1
and T6 only; snapshots are created in T4 and T6 only, under one new prefix `box_settings__`, and no
existing snapshot draws the production strip (D47); the demo fixture moves in T0 only, and no seed
moves. **Build coupling:** T0 adds a field every `BoxRow` constructor must name, so T0 includes every
constructor (OQ-13) and its tree compiles alone. T1 adds a trait method and implements it in all
five implementations in the same task. T2 is a new leaf module. T3 depends on T1's method; T4 on
T2's widget and T3's request; T5 on T3's request.

**Task independence (fact-checked).** Every intersection above was re-derived from the file lists
against the tree at `9a4911a`. T0 ∩ T1 = `{pg/write.rs, .sqlx/}` (serial, as planned). Wave A:
(T1 ∪ T3) ∩ T2 = ∅. `lib.rs:12-28` declares `pub mod ui;` and none of `ui`'s children, so T2 does
not touch it. If T2 extracted a shared window helper from `text_field.rs`, that file would join T2
and the lanes would still be disjoint; D44 has T2 copy the logic instead. T3 ∩ T4 =
`{tests/box_settings.rs}` (serial, as planned). Wave B: T4 ∩ T5 = ∅. `box_probe_pg.rs` uses runtime
`sqlx::query*` calls and `crates/htui` has no `.sqlx/`, so there is no `.sqlx` coupling. The one
coupling is T5's live check, which opens `Settings > Boxes` and so needs T4. It runs on the real
tree after both Wave B lanes merge, not in T5's worktree. The only exhaustive matches over
`StoreRequest` are `name()` (`store_worker.rs:550`) and `try_serve` (`:975`), both in T3's file.
The worker loop (about `:1527-1573`), `testkit.rs:259-320` and `agent_worker.rs:1005` end in
wildcards, and nothing outside `store_worker.rs` matches `StoreReply` exhaustively. No test pins
the variant counts. The only files that pin the store `CASES` count are `mem_store.rs:36-37` and
`pg_conformance.rs:19`. `htui-agent`'s `acp`, `cli` and `fake_conformance` tests pin a different
`CASES` (`htui_agent::conformance`) and do not move. T6's list was missing
`crates/htui-core/src/model/mod.rs` (the `BOX_PROBE_SPEC_KEY` re-export). It is added above, and T6
stays serial and last. Both parallel markings stand. (Amended at fact-check.)

Every implementer prompt carries: PRD D0–D7 win over this plan where they disagree; read the tree,
not `graphify-out/` (it does not exist); nothing sets `updated_at` by hand; no store handle and no
`BoxId` minting on the render side; **commit incrementally** (uncommitted subagent work does not
survive the session, and there is no stash on a shared tree), staging your own paths only; verify
your gate with `--test-threads=1` on the real tree after your merge.

### Task 0: foundations — the token on the row and the tag rule (D40, D42)
- **Files**: as tabled.
- **Tests first** (`box_.rs` unit tests): `a_declared_tag_is_lowercase_digits_underscore_and_dash`;
  `a_declared_tag_list_is_split_on_commas_trimmed_sorted_and_deduplicated`;
  `an_empty_tag_list_is_no_tags`; `an_uppercase_or_spaced_tag_is_refused_by_name`;
  `every_seeded_and_derived_tag_is_a_valid_declared_tag` (the fifteen of milestone 1 D9 and OQ-12,
  written out in the test); `a_sixty_five_char_tag_is_refused`.
- **Action**: `BoxRow.edit_version: i32` with its doc (D39's words); `BoxEdit`; the two functions;
  re-exports; every constructor; `box_profile`, `box_row` and `boxes` select `edit_version`.
  `cargo sqlx prepare` against a scratch database migrated through `0006` (the compose `htui`
  database is empty; project memory).
- **Mirror**: `BoxRecord::needs_probe`'s doc and test style (`box_.rs:133-142`, `:396-420`).
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`; `cargo build -p
  htui-store --all-features` with `SQLX_OFFLINE=true`; `cargo sqlx prepare --check`; `ls
  crates/htui-store/.sqlx | wc -l` still 263. Commit boundary: one red commit, one green.

### Task 1: the writer on every store (D41)
- **Files**: as tabled.
- **Tests first**, three conformance cases appended to `CASES` after
  `"project_delete_counts_requirements"` (`conformance.rs:110`) with their `run_case` arms:
  `edit_box_is_cas_on_edit_version` (an applied edit bumps `edit_version` by one, writes only the
  named field, leaves hostname, probe columns, `htui_version`, `settings`, `last_seen_at`, the
  recorded digest and `box_tool` equal; a spent token is `Stale` carrying the current row and
  writes nothing; an unknown id is `NotFound`). **Another user's box is not asserted in the generic
  case** (amended at fact-check). `run_all` gives each case a store loaded with `DemoData` only,
  which holds one `app_user` and one box (`fixtures.rs:447-479`), and no trait method creates a box
  (`register_box` is `PgStore`-only). The assertion goes into a `mem.rs` unit test built with
  `MemStore::from_demo` plus a planted second user's box (the pattern at `mem.rs:6356`) and into
  `tests/box_identity.rs` on Postgres, which keeps `fixtures.rs` out of T1;
  `edit_box_survives_a_probe_between_read_and_write` (read the token, `record_box_probe` on the same
  box, write with the token: `Applied`, and the probe's columns are still there — the in-trait half
  of "a reconnect cannot stale", since `register_box` is not a trait method);
  `edit_box_refuses_an_invalid_tag` (`Constraint` with D42's sentence, `edit_version` unchanged; a
  spent token with an invalid tag is `Stale`, not `Constraint`). Pins 68 → 71 in `mem_store.rs:37`
  and `pg_conformance.rs:19`. Postgres-only, in `tests/box_identity.rs`:
  `a_reconnect_leaves_edit_version_and_the_edited_fields_alone` (edit, then `register_box` under a
  new hostname, then an `edit_box` with the token the first edit returned is `Applied`) and
  `another_users_box_is_not_found_by_edit_box`. A module test after `conformance.rs:9509` checks that
  every test name a doc comment in that module cites exists. So a case doc may cite
  `a_reconnect_leaves_edit_version_and_the_edited_fields_alone` only under exactly that name
  (amended at fact-check).
- **Action**: D41 on `MemStore`, `PgStore`, `Writer`, and the two forwarding spies.
- **Mirror**: `set_requirement_spec` (Patterns table).
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`; the Postgres
  conformance run (`cargo test -p htui-store --test pg_conformance --all-features --
  --test-threads=1` with `HTUI_TEST_DATABASE_URL`); `cargo test -p htui-store --test box_identity`;
  `cargo test -p htui-agent --all-features -- --test-threads=1`; `prepare --check`.

### Task 2: `TextArea` (D44)
- **Files**: as tabled.
- **Tests first** (unit, in `text_area.rs`): `enter_splits_the_line_at_the_cursor`;
  `backspace_at_column_zero_joins_the_previous_line`; `delete_at_the_end_joins_the_next_line`;
  `up_and_down_keep_the_column_clamped`; `left_and_right_cross_line_ends`;
  `ctrl_s_submits_and_esc_cancels`; `other_chords_tab_and_function_keys_pass`;
  `a_control_char_is_swallowed_not_inserted`; `text_joins_lines_with_newline_and_with_text_round_trips`;
  `debug_never_prints_the_text`; `the_window_keeps_the_cursor_row_visible`;
  `a_long_cursor_line_is_windowed_like_a_text_field`; `a_multi_byte_char_counts_as_one`.
- **Action**: D44.
- **Mirror**: `TextField` and its tests (`text_field.rs:113-165`, `:400-420`).
- **Validate**: `cargo test -p htui --all-features --lib ui::text_area -- --test-threads=1`;
  clippy on the crate.

### Task 3: the worker side (D45, D46)
- **Files**: as tabled.
- **Tests first**, `crates/htui/tests/box_settings.rs` (worker half, driving
  `htui::store_worker::serve` over a `Backend`, as `tests/prompt_settings.rs:3-6` does):
  `boxes_lists_the_demo_box_as_this_box`; `boxes_lists_every_box_of_this_user_by_id` (a second box
  planted in the `MemStore`); `edit_box_applied_answers_the_fresh_snapshot`;
  `edit_box_with_a_spent_token_answers_boxes_stale_and_writes_nothing`;
  `edit_box_on_a_vanished_box_answers_boxes_stale_without_it`;
  `edit_box_with_an_invalid_tag_is_failed_with_the_sentence`;
  `the_spec_view_names_a_stored_overlay_and_an_ignored_one` (via `MemStore::set_app_setting`,
  `mem.rs:500`); `offline_both_are_refused_with_the_database_sentence`. `store_worker.rs` unit
  tests: the two `name()` arms beside `name_arms_are_stable` (`:2720`).
- **Action**: D45, D46.
- **Mirror**: `crate::prompt_settings`; the catalogue arm.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`.

### Task 4: the section (D47–D51)
- **Files**: as tabled.
- **Tests first** (section half of `tests/box_settings.rs`, a `SectionBench` for keys and requests,
  a `Harness` or `draw_section` for frames): snapshots `box_settings__demo`,
  `box_settings__two_boxes` (a hand-built snapshot with a hostname collision, so the id suffix
  shows), `box_settings__offline`, `box_settings__tag_editor`, `box_settings__quirks_editor`,
  `box_settings__stale`; and `activation_asks_for_boxes`; `j_and_k_move_the_selection`;
  `t_opens_the_tag_editor_prefilled_and_enter_sends_only_the_tags`;
  `e_opens_the_quirks_editor_and_ctrl_s_sends_only_the_quirks`;
  `unchanged_text_closes_the_editor_without_a_request`;
  `an_invalid_tag_keeps_the_editor_open_and_sends_nothing`;
  **`a_reload_between_open_and_save_keeps_the_editor_token`** (open on `edit_version = 3`, feed a
  `Boxes` snapshot whose row has a new `updated_at`, `last_seen_at` and probe columns but
  `edit_version = 3`, save: the request carries `expected: 3` — the section half of the PRD metric);
  `boxes_stale_keeps_the_text_takes_the_new_token_and_says_changed_elsewhere`;
  `a_stale_snapshot_without_the_box_closes_the_editor`;
  `p_on_this_box_sends_probe_box`; `p_on_another_box_sends_nothing_and_says_why`;
  `a_box_probed_reply_asks_for_boxes_again`; `a_failed_probe_box_lands_on_the_notice_line`;
  `the_section_captures_input_only_while_an_editor_is_open`;
  `ctrl_c_passes_through_an_open_quirks_editor` (asserts `Pass` only; nothing in the shell quits
  on `ctrl-c` today, OQ-16; amended at fact-check). `tests/settings.rs`: the strip test lists seven
  sections and stays within 100 columns (61).
- **Action**: D47–D51; register the section last; rewrite the strip test's doc.
- **Mirror**: `KindsSection` (editor and stale), `PromptSection` (read, unavailable, notice).
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; `cargo insta` reviewed by
  eye for the six new snapshots only (no existing snapshot may move).

### Task 5: Postgres end to end (PRD metric "Declared tags and quirks CAS")
- **Files**: `crates/htui/tests/box_probe_pg.rs`.
- **Tests** over `Stack` (`:155-257`), driving `store_worker::serve` with `Boxes`/`EditBox`:
  `an_open_editor_survives_a_reconnect_and_a_registration_probe` (read the snapshot and the
  token; `register_box` again, as `a_renamed_box_keeps_its_row_and_its_probe` does at `:393`; then
  a stored spec change and `swap()`, so the registration probe really writes; `updated_at` has
  moved; `EditBox` with the first token answers `Boxes` and the tags are written);
  `a_concurrent_edit_is_reported_and_never_overwritten` (two editors on one token: the second
  answers `BoxesStale` and the first one's tags survive); `another_box_of_this_user_is_editable`
  (a second row inserted by SQL); `another_users_box_is_not_listed_and_not_editable`.
- **Validate**: the workspace gate below, in T5's worktree. The live check opens
  `Settings > Boxes` and so needs T4. It runs on the real tree after both Wave B lanes merge, not in
  T5's worktree (amended at fact-check).

### Task 6 (optional, OQ-18): the `box_probe_spec` editor (D52)
- **Files**: as tabled.
- **Tests first**: conformance `box_probe_spec_set_is_cas_on_updated_at` and
  `box_probe_spec_clear_is_cas_and_needs_a_token` (pins 71 → 73);
  `the_probe_and_the_store_name_one_key` (in `htui`, which sees both crates);
  worker: `an_overlay_the_probe_would_ignore_is_refused_before_any_write`,
  `a_saved_overlay_changes_the_spec_view`, `clearing_returns_to_the_seed`; section:
  `s_opens_the_overlay_editor`, `invalid_json_stays_in_the_editor`, `box_settings__spec_editor`.
- **Action**: D52.
- **Validate**: as T1 and T4.

## Test plan

TDD per repo convention: every task's first commit is its failing tests, named in the task. **The
first red test of the milestone is T0's `a_declared_tag_list_is_split_on_commas_trimmed_sorted_and_deduplicated`.**

**Store conformance** (both stores): three new cases in T1, two in T6. **Postgres**: T1's reconnect
case in `box_identity.rs`; T5's four end-to-end cases. **`htui`**: T2's widget units, T3's worker
cases, T4's section cases and six snapshots, the strip test. The PRD metric "Conformance case plus
section test with a reconnect between read and write" is met three ways: in the trait with a probe
between read and write (T1), in the section with a reload between open and save (T4), and on
Postgres with `register_box` and a real registration probe between them (T5).

**Count pins that move:**

| Pin | Now | After | Where |
|---|---|---|---|
| Store `CASES` | 68 | 71 (T1); 73 with T6 | `crates/htui-core/src/store/conformance.rs:42-111`; `crates/htui-core/tests/mem_store.rs:36-37`; `crates/htui-store/tests/pg_conformance.rs:19` |
| `READ_CASES` | 14 | 14 | `conformance.rs:270-285` |
| `htui-orch` `CASES` | 70 | 70 | `HANDOFF.md:31` |
| `StoreRequest` variants | 64 | 66 (T3); 67 with T6 | `crates/htui/src/store_worker.rs:93` (counted: `Workspaces` `:95` to `RunActions` `:544`) |
| `StoreReply` variants | 35 | 37 | `store_worker.rs:628` (counted: `Workspaces` `:630` to `Failed` `:786`) |
| `.sqlx` files | 263 | 264 (T0 ±0, T1 +1, or +2 if D41's filtered re-read adds a query; amended at fact-check); about 268 with T6 (exact figure recorded at close) | `crates/htui-store/.sqlx/` |
| Migrations | `0001`..`0006` | unchanged; the next is still `0007` | `crates/htui-store/migrations/` |
| `MIRRORED_TABLES` | 21 | 21 | `HANDOFF.md:32` |
| Settings strip | 6 sections, 54 columns | 7 sections, 61 columns | `crates/htui/tests/settings.rs:948-965` |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-20** — An edit of this box's declared tags or quirks reaches the SQLite mirror only at the next refresh pass, so an offline `box_info` shows the old declared tags until then | Low | The mirror is a read cache and `htui` is online-only for writes; `refresh_box` copies the whole row each pass (`cache/refresh.rs:625-660`) |
| **R-21** — The registration probe's `BoxProbed` goes to `Origin::App` at `UNSOLICITED` and is dropped by the freshness gate after the status line (`app/update.rs:264`), so a section already open keeps the old rows | Medium | The status line says a probe ran; `r` reloads; activation re-reads. An open editor is unaffected by design (D48) |
| **R-22** — A large paste arrives key by key (no bracketed paste) | Low | Each key is O(line length); quirks are short notes. Bracketed paste is a later item if it ever matters |
| **R-23** — `ctrl-s` is swallowed where flow control survives raw mode (a multiplexer or a serial line) | Low | crossterm's raw mode is `cfmakeraw` (`unix.rs:302`); OQ-16's alternative is the fallback |
| **R-24** — Quirks go into every prompt's box section, so an edit changes the next step's prompt digest | Certain | Expected and visible in the preview; the digest is recorded per step |
| **R-25** — T0's new field moves three `.sqlx` hashes; a stale `.sqlx` breaks every offline build | Medium | `prepare --check` in T0's gate; the scratch database must be migrated through `0006` first (project memory: the compose `htui` database is empty) |
| **R-26** — `edit_version` is an `INTEGER`; it overflows after 2³¹ edits | Negligible | None needed |
| **R-27** — T6 lets a user store an overlay that makes every box run an executable from its own `PATH` | Low | The same trust boundary as milestone 1 R-11; the overlay is validated by the probe's own `effective` before any write, so only `kind: "path"` tools with bare names are storable |
| **R-28** — Deviations the main thread must record: `BoxRow` gains `edit_version` (OQ-13), the tag syntax (OQ-14), `ctrl-s` (OQ-16), no `capability_tag` read (OQ-17), the spec editor's placement (OQ-18) | Medium | Listed under "Where the PRD, HANDOFF or tree disagree" |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
# from crates/htui-store, against a scratch database migrated through 0006 (the compose `htui`
# database is empty):
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check \
  cargo sqlx prepare --check -- --all-targets --all-features
ls crates/htui-store/.sqlx | wc -l                    # 264 (about 268 with T6)
cargo doc --workspace --no-deps --keep-going          # exactly the two baseline errors (HANDOFF.md:32-33)
git diff --stat 9a4911a -- crates/htui-store/migrations crates/htui-store/cache_migrations   # empty
```

`--test-threads=1` is not optional (the keyring fake is process-wide). Before believing a Postgres
failure, run `df -h /` (the dev Postgres crash-loops under disk pressure) and re-run the case alone.

**Live check on this box (after T4 and T5 are both merged; amended at fact-check).** Launch `htui` against the dev database, open Settings,
cycle to `Boxes`: this box is marked, its hardware, tools and tags match milestone 1's live check,
"probed under the current spec" reads yes. `t`, type `heavy_build, gpu`, `Enter`: the declared tags
read `gpu, heavy_build`. `e`, type two lines, `ctrl-s`: both lines show; the prompt preview's box
section shows them joined by `; `. With the tag editor open, restart the Postgres container and
wait for the `Online` swap, then `Enter`: the write applies (no "changed elsewhere"). In a second
`htui`, edit the same box; back in the first, save: "changed elsewhere … Enter retries". `p` on this
box: the status line reports the probe and the list refreshes; `p` on another box (if any): "the
probe runs on this box only".

## Acceptance

- [ ] The section lists every box of this user, marks this box, and shows profile, tools, probed
      and declared tags, quirks and the last probe (PRD D4).
- [ ] `p` probes this box through milestone 1's `ProbeBox` and refuses on any other box without a
      request.
- [ ] Declared tags and quirks are editable on any listed box; each write is a compare-and-set on
      `box.edit_version` that writes only the edited field.
- [ ] A reconnect (`register_box`) and a registration probe between opening an editor and saving
      it do not stale it (T1, T4, T5).
- [ ] A concurrent edit is reported as changed elsewhere and never overwritten; the typed text
      survives and `Enter` retries against the current row.
- [ ] Quirks are edited in a multi-line `TextArea`; `TextField` is unchanged.
- [ ] No store handle, `UserId` or minted `BoxId` on the render side; every read and write goes
      through the store worker (`R-NF-3`).
- [ ] Store `CASES` 71 (73 with T6) in all three places; no migration; `.sqlx` regenerated and
      `prepare --check` clean; the strip test passes at seven sections.
- [ ] The workspace gate above is green.

## Where the PRD, HANDOFF or tree disagree

1. **Milestone 1's plan promises the `box_probe_spec` editor to this milestone** (OQ-9 at
   `mod-7-box-identity-probe.plan.md:136`, D15 at `:224`, both amended at maintainer review); the
   PRD's milestone 2 row (`:234`) does not mention it. OQ-18; T6 is separable.
2. **The brief for this plan places the Settings code at `crates/htui/src/settings/`**; it lives at
   `crates/htui/src/ui/tabs/settings/` (`mod.rs:4` there names "MOD-7's box profile").
3. **`tests/settings.rs:942-946` says "the five together cost 46"**, and **`HANDOFF.md:470` says "the
   strip is five titles wide (46 of the pinned 100 columns"**; the test's own list has six sections
   (`:950-955`, amended at fact-check; `QdrantSection` added by MOD-34) costing 54, as the PRD says (`:72`). T4 rewrites
   the test's doc; the HANDOFF line is the main thread's.
4. **The PRD's Evidence says "No request reads the full box row or `box_tool`"** (`:73`). Milestone 1
   added `WriteStore::boxes` (`traits.rs:391`), but still no `StoreRequest` serves it; T3 does.
5. **The PRD's Evidence says "`BoxInfo` has no `quirks`, `os_version`, tools or `updated_at`"**, and
   milestone 1 D15 lists "`BoxInfo`'s missing fields" for this milestone. Still true
   (`crates/htui-core/src/model/box_.rs:144-159`), and not changed here (D53): the section reads
   `BoxRecord`, and nothing this milestone renders needs a wider `BoxInfo`.
6. **Milestone 1 D14 kept `edit_version` off `BoxRow`; D15 named `BoxRow.edit_version` as this
   milestone's**; the migration comment says "nothing reads or writes it yet"
   (`0005_box_identity.sql:11`), which T1 makes false. The migration file is not edited
   (forward-only, `R-STO-5`). The comment was already loose for test code:
   `htui-store/tests/migrations.rs` checks the column (`:1335-1343`), reads its default `0`
   (`:1366-1371`) and pins its comment (`:350`). The accurate claim is "no non-test code". Those
   tests do not move, because neither the column nor its comment changes (amended at fact-check).
11. **Many section comments say `CONTROL` chords pass "so `ctrl-c` still quits"** (for example
    `hierarchy.rs:678`, `kinds.rs:681`, `text_field.rs:413`). At `9a4911a` no `ctrl-c` binding exists
    (`keymap.rs:198-238`), and raw mode clears `ISIG`, so `ctrl-c` does nothing. This plan keeps the
    pass-through rule and does not fix the binding. The main thread should raise it as its own item
    (amended at fact-check).
7. **The PRD metric names a "Conformance case … with a reconnect between read and write"**, but
   `register_box` is a `PgStore` method, not a trait method; the conformance case uses a probe
   write as the in-between write and the reconnect proper is Postgres-only (T1's `box_identity.rs`
   case and T5).
8. **`store_worker.rs:205-209` documents `ProbeBox` as bound to no key**; T3 corrects it when T4
   binds it to `p`.
9. **Milestone 1 OQ-12 anticipated a tag picker over `capability_tag`**; the PRD scope says a comma
   list. This plan follows the PRD (OQ-17).
10. **`HANDOFF.md:238-241`'s original MOD-7 text calls the section a "quirks editor (Settings tab box
    profile section)"**; the PRD widens it to every box and declared tags (PRD D4). This plan follows
    the PRD.

---

## Verified claims

Every claim below is a tree fact this plan relies on. The fact-check pass (2026-09-26, against the working tree at `9a4911a`) filled the verdicts: 69 ✓, 9 partial and 0 ✗ on the tree claims, plus eight independence rows at the end (6 ✓, 1 partial, 1 ✗). Every partial and ✗ is amended in the body and marked "(amended at fact-check)".

| Claim | Verdict | Evidence |
|---|---|---|
| `box.edit_version` is `INTEGER NOT NULL DEFAULT 0`, added by `0005_box_identity.sql:19` | ✓ | `0005_box_identity.sql:19`: `ALTER TABLE box ADD COLUMN edit_version INTEGER NOT NULL DEFAULT 0;` |
| `0005_box_identity.sql:25-26` comments `edit_version` as bumped by the editors only, never by registration or the probe | ✓ | `:25-26`: "compare-and-set token of the declared_tags, quirks and settings editors, bumped by them only. Registration and the probe never write it" |
| `0005_box_identity.sql:11` says "nothing reads or writes it yet" | ✓ | `:11`: "milestone 2's compare-and-set token; nothing reads or writes it yet." |
| No Rust code reads or writes `edit_version` at `9a4911a` (only doc mentions in `pg/mod.rs:394` and `traits.rs:374`) | partial | No non-test code reads or writes it. The only production mentions are the docs at `pg/mod.rs:394` and `traits.rs:374`. `htui-store/tests/migrations.rs` does: `:350` pins the column comment, `:1335-1343` checks the column exists, `:1366-1371` reads the default `0`. None of them move. Disagreement item 6 amended. |
| `register_box`'s doc says `edit_version` is never written by a reconnect (`pg/mod.rs:393-395`) | ✓ | `pg/mod.rs:393-395`, the doc of `register_box` (`:405`): "`settings` and `edit_version` are never written by a reconnect (plan D5)" |
| `record_box_probe`'s doc lists `edit_version` among columns it never writes (`traits.rs:370-374`) | ✓ | `traits.rs:371-374` (`:370` is blank), the doc of `record_box_probe` (`:383`) |
| The `set_updated_at` trigger fires on every `UPDATE box` (`0001_init.sql:575-580`, `box` in the list) | ✓ | `0001_init.sql:574-580`: the `DO` block opens at `:574` with `box` in its array at `:575`, and `BEFORE UPDATE … FOR EACH ROW EXECUTE FUNCTION set_updated_at()` is at `:578-579`. The function (`:21-25`) sets `updated_at` unconditionally, and no later migration drops the trigger. |
| `box.declared_tags` is `TEXT[] NOT NULL DEFAULT '{}'` and `quirks` `TEXT NOT NULL DEFAULT ''`, with no `CHECK` (`0001_init.sql:66-67`) | ✓ | `0001_init.sql:66-67` exactly. The `box` table (`:53-74`) has no `CHECK`, and `0002`–`0006` add none on these columns (`0005`'s are on `machine_fingerprint` and `probe_spec_digest`). |
| `capability_tag` is `(tag TEXT PRIMARY KEY, description, seeded)` with no format constraint (`0001_init.sql:43-47`) | ✓ | `0001_init.sql:43-47`: `tag TEXT PRIMARY KEY`, `description TEXT NOT NULL DEFAULT ''`, `seeded BOOLEAN NOT NULL DEFAULT false`. No later migration touches it. |
| No Rust code reads `capability_tag` | partial | No production code reads it. `PgStore` only seeds it (`pg/mod.rs:315`), and tests read it (`htui-store/tests/migrations.rs:992`, `:1037`). An unused `CapabilityTag` model exists (`htui-core/src/model/user.rs:25`, re-exported at `model/mod.rs:145`). OQ-17 amended. |
| `BoxRow` spans `box_.rs:27-66` and has no `edit_version` field | ✓ | `box_.rs:27` `pub struct BoxRow {` to `:66` (derive `:26`). The fields end at `updated_at`. |
| `BoxRow` struct literals needing the new field: `box_.rs:273`, `fixtures.rs:459`, `pg/write.rs:1200`; `box_.rs:316`, `:330`, `:400` spread `..row()`; `conformance.rs:4919` clones the fixture | ✓ | `git grep 'BoxRow {'` finds literals only at `box_.rs:273`, `fixtures.rs:459` and `pg/write.rs:1200`. `box_.rs:316`/`:330`/`:400` spread `..row()`, and `fixture_box` finds the row in `demo_data().boxes`. `pg/mod.rs:414` is `NewBoxRow`. |
| `query_as!(BoxRow, …)` appears exactly twice, in `pg/read.rs` `box_profile` (`:1519`, macro `:1521`) and `box_row` (`:1796`, macro `:1798`) | partial | Two, in those functions (`box_profile` `:1519`, `box_row` `:1796`), but the `sqlx::query_as!(` lines are `:1520` and `:1797`. `:1521` and `:1798` are the `BoxRow,` argument lines. OQ-13 amended. |
| `PgStore::boxes` is `pg/write.rs:1151-1226`, filters `WHERE user_id = $1`, and builds `BoxRow` at `:1200` | ✓ | `pg/write.rs:1151` `async fn boxes`; `:1160` `FROM box WHERE user_id = $1 ORDER BY id`; literal `:1200`; close `:1226`. D41's `:1163` corrected to `:1160`. |
| `BoxRecord` is `box_.rs:122-131` with `row`, `tools`, `probe_spec_digest` | ✓ | `box_.rs:122` doc, `:124` `pub struct BoxRecord`, fields `:126-130`, close `:131` |
| `BoxInfo` (`box_.rs:144-159`) has no `quirks`, `os_version`, tools or `updated_at` | ✓ | `box_.rs:144-159`: fields are only `box_id`, `hostname`, `os_family`, `probed_tags`, `declared_tags`, `settings` |
| `WriteStore` is declared at `traits.rs:257`; `record_box_probe` at `:383`; `boxes` at `:391` | ✓ | `traits.rs:257` `pub trait WriteStore: ReadStore`, `:383`, `:391` |
| `CasOutcome<T>` with `Applied(T)`/`Stale(T)` is at `traits.rs:1598` | ✓ | `traits.rs:1598` `pub enum CasOutcome<T>`, `Applied(T)` `:1600`, `Stale(T)` `:1602` |
| `StoredSetting { value: Option<Value>, updated_at }` is at `traits.rs:1735` | ✓ | `traits.rs:1735`, `value: Option<Value>` `:1737`, `updated_at: DateTime<Utc>` `:1739` |
| `PgStore::set_requirement_spec` (`pg/write.rs:4454-4511`) is `UPDATE … version = version + 1 WHERE … AND version = $2 RETURNING` then `cas_miss` | partial | The span and the `cas_miss` fallback (`:4505`) are right, but there are two branches. `None` is `INSERT … ON CONFLICT (project_id) DO NOTHING RETURNING` (`:4462-4480`). `Some(v)` is the `UPDATE … version = version + 1 … AND version = $2 RETURNING` (`:4481-4500`). Summary and Patterns now cite the `Some` branch only. |
| `cas_miss` is defined at `pg/write.rs:77` | ✓ | `pg/write.rs:77` `fn cas_miss<T>(current: Option<T>, entity, id) -> Result<CasOutcome<T>>` |
| Exactly five `impl WriteStore`: `mem.rs:5130`, `pg/write.rs:586`, `writer.rs:349`, `htui-agent/src/conformance.rs:709`, `htui-agent/tests/recorder.rs:389` | ✓ | A repo-wide grep for `impl.*WriteStore for` finds exactly these five. The `backend.rs` hit is a doc comment. |
| `Writer::record_box_probe` is at `writer.rs:429` and `Writer::boxes` at `:436` | ✓ | `writer.rs:429`, `:436`, both inside `impl WriteStore for Writer` (`:349`) |
| The kinds conformance case pins "a spent token and an invalid prefix together answer Stale, not Constraint" (`conformance.rs:3024-3027`) | ✓ | `conformance.rs:3024-3027`, in `item_kind_round_trip_and_prefix_rules` |
| `CASES` spans `conformance.rs:42-111` and holds 68 names; milestone 1's box cases are `:96-98`; `"project_delete_counts_requirements"` is last (`:110`) | ✓ | `:42` to `];` at `:111`, names at `:43-110` = 68; box cases `:96-98`; `"project_delete_counts_requirements"` at `:110` |
| `READ_CASES` spans `conformance.rs:270-285` (14) | ✓ | `:270` to `];` at `:285`, names at `:271-284` = 14 |
| `mem_store.rs:36-37` pins `CASES.len()` at 68; `pg_conformance.rs:19` has `EXPECTED_CASES: usize = 68` | ✓ | `mem_store.rs:36` `conformance::CASES.len(),`, `:37` `68,`; `pg_conformance.rs:19` `const EXPECTED_CASES: usize = 68;` |
| `fixture_box` (`conformance.rs:4919`) returns the fixture's box | ✓ | `conformance.rs:4919` `fn fixture_box() -> BoxRow` finds `ids::BOX` in `fixtures::demo_data().boxes` |
| The demo fixture has one box, `declared_tags ["gpu"]`, `quirks ""` (`fixtures.rs:457-480`) | ✓ | `fixtures.rs:457` doc, `:458` `fn boxes()`, a one-element vec; `declared_tags` `:472`, `quirks` `:473`; close `:480` |
| The demo loader's `INSERT INTO box` names its columns and omits `edit_version` (`pg/demo.rs:65-71`) | ✓ | `pg/demo.rs:65-71`: 19 named columns (`id` to `updated_at`), no `edit_version` |
| `refresh_box`'s `BOX_COLUMNS` (`cache/refresh.rs:603-623`) omit `edit_version`; the mirror's `box_info` reads `id, hostname, os_family, probed_tags, declared_tags, settings` (`cache/read.rs:1317-1334`) | ✓ | `BOX_COLUMNS` has 19 columns and no `edit_version` (none in the file). `box_info` is at `cache/read.rs:1317` with its SELECT at `:1319-1320`. The function ends at `:1337`, not `:1334`, and D40 was corrected. |
| `crates/htui-store/.sqlx/` holds 263 files | ✓ | `ls -A crates/htui-store/.sqlx \| wc -l` = 263; `git ls-files` also 263 |
| The migrations are `0001`..`0006`; the next is `0007` | ✓ | `0001_init` … `0006_requirements`; `cache_migrations/` is numbered separately (`0001`–`0004`) |
| `StoreRequest` (`store_worker.rs:93`) has 64 variants, `Workspaces` `:95` to `RunActions` `:544` | ✓ | A brace-depth count gives 64, from `Workspaces` `:95` to `RunActions(ItemId)` `:544`; the enum closes at `:545` |
| `StoreReply` (`store_worker.rs:628`) has 35 variants, `Workspaces` `:630` to `Failed` `:786` | ✓ | A brace-depth count gives 35, from `Workspaces` `:630` to `Failed {` `:786` |
| `ProbeBox` is at `store_worker.rs:209` and its doc (`:205-208`) says milestone 1 binds it to no key | ✓ | `:209` `ProbeBox,`; the doc at `:205-208` ends "Milestone 1 binds it to no key (plan D15)." |
| `StoreReply::BoxProbed(BoxProbeReport)` is at `store_worker.rs:784` | ✓ | `store_worker.rs:784` `BoxProbed(crate::agent_worker::BoxProbeReport),` |
| `StoreRequest::name` is at `store_worker.rs:550`; `name_arms_are_stable` at `:2720` | ✓ | `:550` `pub const fn name(&self) -> &'static str`; `:2720` `fn name_arms_are_stable()` |
| `try_serve` is at `store_worker.rs:975`; `ProbeBox` is in its no-runtime refusal arm; the catalogue or-ed arm is `:1038-1049` with the E0004 comment | ✓ | `:975`. `ProbeBox` is at `:1010` in the "no agent runtime in this build" arm. The catalogue comment is at `:1038-1040` (E0004 at `:1040`) and its arms at `:1041-1049`. The hierarchy arm's E0004 comment is at `:1021-1025`, so D46's `:1022-1026` was corrected. |
| `UNSOLICITED: Seq = Seq::MAX` at `store_worker.rs:52` | ✓ | `store_worker.rs:52` `pub const UNSOLICITED: Seq = Seq::MAX;` |
| `App::dispatch` keys `latest` on `(origin, discriminant)` (`app/state.rs:283-296`); `is_fresh` at `:304-308` | ✓ | `dispatch` at `:282` (body `:283-296`) inserts `(origin.clone(), std::mem::discriminant(&request))` at `:286-287`; `is_fresh` is at `:304-308` |
| `observe_reply` sets the status line for every `BoxProbed` (`app/update.rs:264`) | ✓ | `app/update.rs:264`, an unconditional arm inside `observe_reply` (`:238`) |
| `BOX_PROBE_RUNNING` is at `agent_worker.rs:92-93`; `BoxProbeReport` at `:103` carries counts and tags, not rows | ✓ | const at `:92-93`; derive `:103`, `pub struct` `:104`. The fields are counts, tag lists, failure options and `unchanged`, with no rows. |
| `AgentRuntime::probe_box` (`agent_worker.rs:1181`) refuses offline with `REGISTRY_ON_SERVER_ONLY`, with no box row, and with a claim held, before spawning | ✓ | `agent_worker.rs:1181-1204`: `writer()` or `Unreachable(REGISTRY_ON_SERVER_ONLY)`, `registered_box` (`NotFound`), `claim_is_free`, `probe_env`, all before `tokio::spawn` (`:1195`). D49's `:1175` corrected. |
| `Backend::writer()` (`backend.rs:153-159`) is `None` offline; `DATABASE_UNREACHABLE` is at `writer.rs:143` | ✓ | `backend.rs:153-159` `Self::Offline { .. } => None`; `writer.rs:143` `pub const DATABASE_UNREACHABLE` |
| `Backend::app_settings()` is at `backend.rs:397` | ✓ | `backend.rs:397` `pub async fn app_settings(&self)` |
| `MemStore::set_app_setting` is at `mem.rs:500`; `MemStore` keeps `app_settings: BTreeMap<String, (Value, DateTime<Utc>)>` (`mem.rs:131`) | partial | `set_app_setting` at `mem.rs:500` is right, but the `app_settings` map (`mem.rs:131`) is a field of the private `State` (`mem.rs:86`). `MemStore` (`mem.rs:61`) holds it as `state: Arc<RwLock<State>>`, and so does `box_probe_digests` (`mem.rs:123`). OQ-13 amended. |
| `htui_agent::box_probe::spec` exposes `SETTING_KEY = "box_probe_spec"` (`:46`), `seed()` (`:130`), `effective(seed, stored)` (`:137`), `validate` (`:179`) | ✓ | `spec.rs:46`, `:130`, `:137`, `:179`, all `pub`; `pub mod spec` at `box_probe/mod.rs:12` |
| `crate::hierarchy::serve` (`hierarchy.rs:169`) resolves this box with `backend.box_info()` at `:173` | ✓ | `hierarchy.rs:169` `pub async fn serve`, `:173` `backend.box_info().await?.map(\|info\| info.box_id)` |
| `crate::prompt_settings` has `snapshot` `:137`, `serve` `:182`, `REQUEST_NAMES` `:249`, `READ_NAME` `:256`, and the re-read residue note at `:8-11` | ✓ | `prompt_settings.rs:137`, `:182`, `:249`, `:256`; residue note at `:8-11` |
| `crate::catalogue::REQUEST_NAMES` is at `catalogue.rs:321` | ✓ | `catalogue.rs:321` `pub const REQUEST_NAMES: [&str; 9]` |
| `settings/mod.rs:4` names "MOD-7's box profile" | ✓ | `settings/mod.rs:4` contains "MOD-7's box profile" |
| `settings/mod.rs` declares sections at `:11-17` and re-exports at `:32-37` | ✓ | `:11` `pub mod agents;` to `:17` `pub mod qdrant;`; `:32` to `:37` the `pub use` lines |
| `CHANGED_ELSEWHERE`, `CHANGED_ELSEWHERE_CLOSED`, `DELETED_ELSEWHERE` are at `settings/mod.rs:103`, `:111`, `:118`; `is_error` `:65`; `wrapped` `:78` | ✓ | `:103`, `:111`, `:118`, `:65`, `:78`, confirmed on disk |
| `SettingsSection::captures_input` defaults to false (`settings/mod.rs:147`) | ✓ | `settings/mod.rs:147` `fn captures_input(&self) -> bool {`, `:148` `false` |
| `register_all` registers six Settings sections, Qdrant last, with the "appending moves no existing section" comment (`app/mod.rs:49-58`) | ✓ | `app/mod.rs:49-58`: Agents, Hierarchy, Kinds, Prompt, Connection, then Qdrant last (`:57`); the comment at `:54-55` says "appending moves no existing section's line" |
| Section titles are `Agents`, `Hierarchy`, `Kinds`, `Prompt`, `Connection`, `Qdrant`; the strip costs 54 columns | ✓ | `title()` at `agents.rs:1107`, `hierarchy.rs:1019`, `kinds.rs:1362`, `prompt.rs:791`, `connection.rs:729`, `qdrant.rs:288`; the sum of (len + 2) is 42 + 12 = 54 |
| The strip test is `tests/settings.rs:948-965` over six sections; its doc at `:942-946` still says five and 46; `SECTION_WIDE` is 100 (`:46`) | ✓ | `:948` `fn the_section_strip_fits_the_frame` to `:965`; six sections at `:950-955`; the doc says "the five together cost 46" (`:943-945`); `SECTION_WIDE` at `:46` |
| `HANDOFF.md:470` says the strip is five titles wide (46 columns) | ✓ | `HANDOFF.md:470` "the strip is five titles wide (46 of the pinned 100 columns, `tests/settings.rs`)" |
| No existing snapshot draws the production section strip (every strip in `tests/snapshots/` is a test-built list) | ✓ | The widest strip in `tests/snapshots/` is `Agents  Hierarchy  Kinds  Prompt` (`prompt_settings__*.snap:8`), and no `.snap` contains `Connection` or `Qdrant`. `connection.rs`'s `the_product_registers_connection_after_prompt` (about `:2046`) walks the production list without a snapshot and still passes with `Boxes` last (D47 amended). |
| `TextField` is single-line by design (`text_field.rs:1-7`); `FieldOutcome` is at `:25-36`; `Enter` submits, `Esc` cancels, chords other than `SHIFT` pass (`:113-165`) | ✓ | The doc at `:1-7`; `FieldOutcome` at `:25-36`. `on_key` (`:113-165`) passes `CONTROL`/`ALT`/`SUPER`/`META`/`HYPER` (`:114-122`), `Enter` → `Submit` at `:161`, `Esc` → `Cancel` at `:162`. |
| `TextField`'s `Debug` never prints the text (`text_field.rs:53-60`) | partial | True, but the doc and the hand-written `impl core::fmt::Debug` span `:53-64` (doc `:53-55`, impl `:56-64`, printing `masked`, `len` and `cursor`). D44 and Patterns amended. |
| `ui/mod.rs` declares `text_field` at `:7` and re-exports it at `:11` | ✓ | `ui/mod.rs:7` `pub mod text_field;`, `:11` `pub use text_field::{FieldOutcome, TextField};` |
| `crates/htui/src/lib.rs` declares `pub mod ui;` and none of `ui`'s children | ✓ | `lib.rs:28` `pub mod ui;`. No child of `ui` is declared or re-exported in `lib.rs:12-31`. |
| The only `ctrl-` chord bound in `crates/htui/src` is `ctrl-c`; no keyboard-enhancement or bracketed-paste mode is enabled | partial | The first half is false: no `ctrl-` chord is bound at all. `Keymap::default_global` (`keymap.rs:198-238`) binds `q`, `Tab`, `BackTab`, `1`–`9`, `?` and the overlay `Esc`, and `register_all` (`app/mod.rs:63`) adds `w`. `ctrl-c` appears only in the parser test (`keymap.rs:329-330`) and in `CONTROL` pass-through guards, and raw mode clears `ISIG`, so `ctrl-c` does not quit. The second half holds: `ratatui::init` (0.30.2 `init.rs:399-400`) enables only raw mode and the alternate screen. OQ-16, D44, T4 and disagreement item 11 amended. |
| crossterm is `0.29.0` in `Cargo.lock` and its unix raw mode calls `cfmakeraw` (`src/terminal/sys/unix.rs:302`) | ✓ | `Cargo.lock:1058-1059` crossterm `0.29.0`; `crossterm-0.29.0/src/terminal/sys/unix.rs:302` `unsafe { cfmakeraw(termios) }` |
| The MOD-15 sections bind `r` to reload (`prompt.rs:839`, `kinds.rs:1450`, `hierarchy.rs:1115`); the agents section binds `r` to probe (`agents.rs:535`) | partial | `prompt.rs:839`, `kinds.rs:1450` and `hierarchy.rs:1115` are the reload arms. The agents section's `r`-to-probe arm is `agents.rs:1196` (`KeyCode::Char('r') if !self.probing` → `ProbeAgents`), with guards at `:1185` and `:1192` and a busy arm at `:1201`. `:535` is only `refuse_during_login`'s label. OQ-19 amended. |
| The hierarchy section passes `CONTROL` chords so `ctrl-c` still works (`hierarchy.rs:674-682`) | ✓ | `hierarchy.rs:677-682`: the doc at `:677-678`, `Handled::Pass` on `CONTROL` at `:680`, and the cited `:674` opens three lines early (D44 corrected). The pass-through exists, but the implied "so `ctrl-c` still works" does not hold (row on `ctrl-` chords above). |
| `KindsSection::on_stale` keeps the text, re-takes the token and says `CHANGED_ELSEWHERE` (`kinds.rs:1085-1135`) | ✓ | `kinds.rs:1085-1135`: doc `:1085`, fn `:1087`. `Reload::Token` takes the token and says `CHANGED_ELSEWHERE` (`:1132`), and `Keep` also says it (`:1107`). With no editor open it says `CHANGED_ELSEWHERE_CLOSED`; when the row is gone, `DELETED_ELSEWHERE`. |
| The prompt renders quirks with newlines collapsed to `; ` and blank lines dropped (`render.rs:410-420`) | ✓ | `render.rs:410-420`: `normalise_newlines`, split on `\n`, trim, drop empties, `join("; ")` |
| `SectionBench` is at `testkit.rs:579` | ✓ | `testkit.rs:579` `pub struct SectionBench {` |
| `tests/box_probe_pg.rs` has a `Stack` with `swap()` and calls `register_box` in `a_renamed_box_keeps_its_row_and_its_probe` (`:382-400`) | ✓ | `box_probe_pg.rs:157` `struct Stack`, `:201` `async fn swap`, `:382` the test, `:393` `.register_box(` |
| `R-BOX-3` (`REQUIREMENTS.md:71-73`) asks for a quirks note editable in the TUI and says the vocabulary is open | ✓ | `docs/REQUIREMENTS.md:71-73`: "free-form quirks note editable in the TUI … Vocabulary is open." |
| `R-TUI-8` (`REQUIREMENTS.md:314`) names "box profile with capability edits" | ✓ | `docs/REQUIREMENTS.md:314`: "box profile with capability edits" |
| ANA-16 C7 (`ANA-16.md:487`) says any future box-settings writer is a CAS | ✓ | `docs/ANA-16.md:487`, the C7 row "Box settings have no write path" … "Any future writer is a CAS" |
| `HANDOFF.md:28-33` pins `CASES` 68, `READ_CASES` 14, `htui-orch` `CASES` 70, `StoreRequest` 64, `StoreReply` 35, 263 `.sqlx`, `MIRRORED_TABLES` 21, next migration `0007` | partial | The values are right, but they sit at `HANDOFF.md:27-32`: "Live coordinates" `:27`, next migration `0007` `:29`, the pins `:31-32`. The Count pins table's `:31`/`:32` and Validation's `:32-33` (doc baseline) are correct as cited. |
| Milestone 1's plan says "Milestone 2 ships the editor" (`mod-7-box-identity-probe.plan.md:136`) and lists the `box_probe_spec` editor in D15 (`:224`) | ✓ | `mod-7-box-identity-probe.plan.md:136` "Milestone 2 ships the editor (D15)"; `:224` D15 lists "the `box_probe_spec` editor" |
| The PRD's milestone 2 row (`:234`) does not mention the spec editor | ✓ | `mod-7-box-registry.prd.md:234` lists the box section, profile/tools/tags, re-probe, and the declared tags and quirks CAS, with no spec editor |
| T0 ∩ T1 = `{pg/write.rs, .sqlx/}`, so T1 runs serial after T0 | ✓ | Both lists name `crates/htui-store/src/pg/write.rs` and `.sqlx/`, and nothing else is shared. Serial, as planned. |
| Wave A: lane 1 (T1 then T3) and lane 2 (T2) touch disjoint files | ✓ | The intersection is ∅. `lib.rs:12-28` declares only `pub mod ui;`, so T2 leaves it alone. A shared-helper variant of D44 would add `text_field.rs` to T2 and still be disjoint. Parallel marking stands. |
| T3 ∩ T4 = `{tests/box_settings.rs}`, so T4 runs after T3 | ✓ | Serial, as planned |
| Wave B: T4 and T5 touch disjoint files | ✓ | The intersection is ∅. `box_probe_pg.rs` uses runtime `sqlx::query*` calls and `crates/htui` has no `.sqlx/`. Only T5's live check needs T4, so it now runs after both lanes merge (T5 and Validation amended). Parallel marking stands. |
| Every task's file list is complete | partial | T0–T5 are complete. T6 lacked `crates/htui-core/src/model/mod.rs` for the `BOX_PROBE_SPEC_KEY` re-export, which is now added to the T6 row, Files to Change and D52. T6 stays serial and last. |
| `StoreRequest`/`StoreReply` move only in T3's and T6's file (`store_worker.rs`) | ✓ | The only exhaustive matches are `name()` (`:550`) and `try_serve` (`:975`). The worker loop, `testkit.rs:259-320` and `agent_worker.rs:1005` end in wildcards, `app/update.rs` ends in `_ => {}`, and no test pins the variant counts. |
| `CASES` pins move only in T1 and T6 | ✓ | The only files that pin the count are `mem_store.rs:36-37` and `pg_conformance.rs:19`. `conformance.rs:9492`/`:9504` compare against `CASES.len()`. `htui-agent`'s `acp`, `cli` and `fake_conformance` tests pin `htui_agent::conformance::CASES`, a different list. |
| T1's "another user's box is `NotFound`" fits the generic conformance suite | ✗ | `run_all` loads only `DemoData`, with one user and one box (`fixtures.rs:447-479`), and no trait method creates a box. The assertion moves to a `mem.rs` unit test and `box_identity.rs`, both already T1 files, and `fixtures.rs` stays T0-only (T1 amended). |
