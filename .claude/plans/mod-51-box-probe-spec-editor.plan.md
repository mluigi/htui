# Plan: MOD-51 — `box_probe_spec` editor in the Settings box section

> **Status: complete** (2026-10-01). T1–T4 implemented (`e1eb188`..`a8f1270`), review fixes `c7ecb69`, `5e1119b` (D6's blank rule amended at review LOW-1: the live token decides; the empty list draws the spec, LOW-6). Write-up: `docs/decisions/mod/mod-51.md`.

**Source**: `HANDOFF.md` item MOD-51 (from MOD-7 milestone 2, OQ-18). The planned shape is **MOD-7
D52** / milestone 2 **T6** in `.claude/plans/mod-7-box-settings-section.plan.md` (`:200`, file list
`:281`), which the maintainer deferred here at that milestone's CONFIRM gate (2026-09-26). This
plan re-grounds D52 against the tree at `de177dd` (main after MOD-59), about thirty merges after
milestone 2 landed, and records where it amends D52.

**Requirements**: `R-BOX-2` (the probe spec is data, overlayable in `app_setting`), `R-TUI-8` (box
profile edits in Settings), `R-NF-3` (no store handle on the render side; the write runs on the
store worker). ANA-16 C7: every human writer is a compare-and-set.

**Routing**: plan path, **no ultracode** (`/handoff-run MOD-51`, accepted by the maintainer
2026-10-01): C4 fired (≈18 files), C1–C3 did not; the work is one serial chain, so there is nothing
to fan out. **Staffing**: every agent inherits the session model (Opus 5.5); no `model:` override,
never Fable.

**Complexity**: Medium. No migration (`app_setting` already exists, `0001_init.sql:557`: `key TEXT
PRIMARY KEY, value JSONB NOT NULL, updated_at TIMESTAMPTZ NOT NULL DEFAULT now()`, stamped by the
`BEFORE UPDATE` trigger loop). Two new `WriteStore` methods across the five implementations, two
conformance cases, one new `StoreRequest` variant, one new pure checker in `htui-agent`, one new
editor mode in the existing section reusing the existing `TextArea`.

**Numbering**: this plan's decisions are **D1…**, risks **R-1…**; MOD-7's are always cited as
"MOD-7 D*n*".

---

## Summary

The Settings `Boxes` section today shows the effective probe spec read-only (MOD-7 D51: `seed` or
`seed + stored overlay`, twelve hex digits of the digest, and why a stored overlay was ignored). The
only way to change the overlay is the SQL in `crates/htui-agent/src/box_probe/spec.rs:13-24`. This
item adds the writer: `s` opens a multi-line JSON editor over the stored overlay, `ctrl-s` saves it
as a compare-and-set on `app_setting.updated_at`, and the worker refuses, before any write, every
overlay the probe itself would ignore — so the editor can never store an overlay that silently
falls back to the seed.

## Requirements restated

1. A maintainer can view, set, replace and clear the `app_setting.box_probe_spec` overlay from
   `Settings > Boxes` without SQL.
2. The write is a compare-and-set: an overlay changed elsewhere since the editor opened is never
   overwritten; the editor reloads its token, keeps the typed text, and a second save retries.
3. An overlay the probe would reject (bad JSON shape, unknown key, non-bare tool name, bad regex,
   a tag rule naming a missing tool, …) is refused with the probe's own fault sentence and nothing
   is stored.
4. The store validates only what it can know without `htui-agent` (the value is a JSON object);
   the spec's semantics stay in `htui-agent` (the store must not depend on it).
5. Nothing on the render side touches the store (`R-NF-3`); one request, one reply.
6. After a save, the section tells the maintainer that the next `p` (or the next connect) re-probes
   under the new spec (MOD-7 D18: a different digest re-probes).

## Patterns to mirror

| Category | Source | Pattern |
|---|---|---|
| App-rung CAS in Postgres | `crates/htui-store/src/pg/write.rs:3209-3267` (`set_setting`, `App` arm), `:3355-3380` (`clear_setting`) | `INSERT … ON CONFLICT (key) DO NOTHING RETURNING updated_at` for "expect no row"; `UPDATE … WHERE key = $1 AND updated_at = $3 RETURNING updated_at` under a token; `DELETE … WHERE key = $1 AND updated_at = $2 RETURNING updated_at`; a miss re-reads and answers `Stale` |
| App-rung CAS in memory | `crates/htui-core/src/store/mem.rs:3306-3345` (`set_setting`, `App` arm) | `(stored, expected)` match: `(None, None)` and `(Some(t), Some(t))` apply; token minted from `self.now()` |
| Token type | `crates/htui-core/src/store/traits.rs:2300-2312` (`StoredSetting`) | `{ value: Option<Value>, updated_at: DateTime<Utc> }`; reused, not a new type (MOD-7 D52) |
| CAS result | `traits.rs:2138` (`CasOutcome<T>`) | `Applied(row)` / `Stale(row as it is now)` |
| Worker module | `crates/htui/src/box_settings.rs` (whole file, 132 lines) | `serve` takes `writer()` or `Unreachable(DATABASE_UNREACHABLE)`; a write re-reads the snapshot and answers `Boxes` (applied) / `BoxesStale` (spent token); `Constraint` bubbles to `Failed`; `REQUEST_NAMES` is the single source of the request names |
| Worker routing | `crates/htui/src/store_worker.rs:1530-1535` | or-ed arm to `box_settings::serve` (no guards in the wildcard-free match) |
| Name pin | `store_worker.rs:3604-3620` (`box_requests_are_named_as_box_settings_lists_them`) | the test lists every variant's `name()` against `box_settings::REQUEST_NAMES` |
| Multi-line editor | `crates/htui/src/ui/tabs/settings/boxes.rs:120-198` (`Mode::Quirks(Editor<TextArea, String>)`), `HINT_QUIRKS` `:60`, `CHANGED_ELSEWHERE_QUIRKS` `:86` | `ctrl-s` saves, `Esc` cancels, `Enter` breaks the line (MOD-7 OQ-16); `opened_on` never refreshed; token replaced only by `BoxesStale` (MOD-7 D48) |
| Reply handling | `boxes.rs:613-640` (`on_reply`) | `Failed { request }` matched by name; a refused write leaves the editor open over its text and clears `busy` |
| Errors | `StoreError::Constraint(sentence)` | the sentence is shown verbatim as the section's notice |
| Tests | `crates/htui-core/src/store/conformance.rs` (generic `async fn case<S: WriteStore>(store: &S)` + `CASES` list), `crates/htui/tests/box_settings.rs` (worker half over `MemStore`; section half with `insta` snapshots `box_settings__*.snap`) | TDD: conformance cases first, then worker tests, then section tests and snapshots |
| Logging | `spec.rs:156` | `effective` warns once when a *stored* overlay is ignored at probe time; a refused *edit* is not a probe event and must not log that warning (D3) |

## Decisions

| # | Decision | Why |
|---|---|---|
| D1 | **One key constant.** `htui_core::model::box_::BOX_PROBE_SPEC_KEY: &str = "box_probe_spec"`, re-exported from `crates/htui-core/src/model/mod.rs` beside `BoxEdit`. `htui_agent::box_probe::spec::SETTING_KEY` (`spec.rs:46`) becomes `pub const SETTING_KEY: &str = htui_core::model::BOX_PROBE_SPEC_KEY;` — every current reader (`agent_worker.rs:2339`, `box_settings.rs:70`, the tests) keeps compiling unchanged. | MOD-7 D52: the store (no `htui-agent` dependency) and the probe cannot disagree on the key. |
| D2 | **Store seam (MOD-7 D52, unchanged).** `WriteStore` gains `async fn box_probe_spec(&self) -> Result<Option<StoredSetting>>` (`None` = no row; a row always has `value: Some`) and `async fn set_box_probe_spec(&self, overlay: Option<Value>, expected: Option<DateTime<Utc>>) -> Result<CasOutcome<Option<StoredSetting>>>`. Semantics: `Some(v)` + `None` inserts when no row exists, else `Stale(Some(row))`; `Some(v)` + `Some(t)` updates where `updated_at = t`, else `Stale(current)` — **`Stale(None)` when the row is gone** (not `NotFound`, unlike `set_setting`'s `App` arm: a row deleted under an open editor is a miss like a spent token, MOD-7 D48); `None` + `Some(t)` deletes where `updated_at = t` → `Applied(None)`, else `Stale(current)`; `None` + `None` is a `Constraint`; a `Some(v)` that is not a JSON object is a `Constraint`. Precedence: both `Constraint`s first (no read needed), then the CAS. The generic `set_setting` is not used: its keys are the closed `SettingKey` enum (MOD-7 milestone 1 D17). | A narrow typed writer, same shape as the App-rung CAS it sits beside. |
| D3 | **Validation is the probe's own, without the probe's warning** *(amends MOD-7 D52)*. `spec.rs` gains `pub fn check(overlay: &Value) -> Result<String, String>`: `merge(seed(), overlay)` → `Ok(digest)` or `Err(fault)`, pure, **no `warn!`**, and `pub const SPEC_REFUSED: &str = "box_probe_spec refused"`. `box_settings::serve` calls it for `Some(overlay)` before the store and answers `Err(Constraint(format!("{SPEC_REFUSED}: {fault}")))` — nothing is written. D52 had `serve` call `spec::effective`, which logs "the box probe runs under its seed spec" on every rejection and prefixes the sentence with `SPEC_IGNORED`; both are wrong for a refused edit. `effective` itself is untouched. | Same merge, same fault sentences, so "the editor can never store an overlay the probe would ignore" holds by construction (the checker and the probe share `merge`); no misleading log line per typo. |
| D4 | **Request.** `StoreRequest::SetProbeSpec { overlay: Option<Value>, expected: Option<DateTime<Utc>> }`, name `"set_probe_spec"`, appended to `box_settings::REQUEST_NAMES` (2 → 3) and routed by the existing or-ed arm (`store_worker.rs:1533`). Answers: `Applied` → `Boxes`, `Stale` → `BoxesStale`, a `Constraint` → `Failed { request: "set_probe_spec" }`. `StoreRequest` derives only `Debug, Clone` (`store_worker.rs:104`), so a `Value` field is fine; the overlay is not a secret (the enum's no-plain-secret rule, `:97`). | One request, one reply; same reply pair as `EditBox` so the section has one reload path. |
| D5 | **Snapshot carries the token.** `SpecView` gains `stored: Option<StoredSetting>` (the row and its token; `StoredSetting` is `Eq`, so `SpecView` keeps `Eq`). `box_settings::snapshot` reads `writer.box_probe_spec()` instead of `backend.app_settings()` for the overlay, so the view and the token come from one read. `spec_view` computes `digest`/`overlay`/`error` from `stored.value` as today. | The editor opens on the token of the value it shows. |
| D6 | **Section.** *(Amended at review, LOW-1: a blank save keys on the live token `expected` alone — `None` closes with nothing sent, `Some(t)` clears under `t` — and `opened_on` was dropped from the editor; LOW-6: with no box listed, Browse draws the spec lines under the no-boxes message.)* In Browse, `s` opens `Mode::Spec` — an editor holding `expected: Option<DateTime<Utc>>`, `opened_on: Option<Value>` and a `TextArea` seeded with the stored overlay pretty-printed (`serde_json::to_string_pretty`) or empty. `s` works whenever a snapshot is present and the read was not refused, **with or without a listed box** (the spec is app-wide, not per box). `ctrl-s`: text blank → `overlay: None` (clear); blank over `opened_on: None` → close, nothing sent; otherwise `serde_json::from_str` on the render side, and a parse error stays in the editor as the notice (nothing sent). `Esc` cancels; `Enter` breaks the line. `busy` holds `"set_probe_spec"` while in flight (a second save says it is in flight). `BoxesStale` with the spec editor open replaces `expected` with the fresh token and keeps the text, notice `changed elsewhere since you opened it — reloaded; ctrl-s retries against the current row` (the `CHANGED_ELSEWHERE_QUIRKS` wording, so `is_error` colours it). `Failed { "set_probe_spec" }` keeps the editor open with the sentence. An applied save closes the editor with the notice `probe spec saved — the next p (or the next connect) re-probes under it`. `HINT_BROWSE` gains `s spec` and `p probe this box` shortens to `p probe` (blueprint F-10, maintainer 2026-10-01); `HINT_NO_LIST` becomes `s spec · r reload` over an empty list only, and a new `HINT_RELOAD = "r reload"` covers not-read and refused (F-3); a blank save also closes with nothing sent when the token is `None` (F-4). The editor takes the body area (list and detail), since it is not about the selected box. | MOD-7 D52's section shape plus the no-box case D52 left open; everything else mirrors the quirks editor. |
| D7 | **Conformance: two cases** (MOD-7 D52): `box_probe_spec_is_cas_on_updated_at` (insert on no row; insert over a row is `Stale(Some)`; update under the token; spent token is `Stale(current)`; delete under the token is `Applied(None)`, then `box_probe_spec()` is `None`; update or delete under a token after the row is gone is `Stale(None)`) and `set_box_probe_spec_refuses_a_non_object_and_a_clear_without_a_token` (nothing written either way). `CASES` 104 → 106; the pins at `crates/htui-core/tests/mem_store.rs:36-37` and `crates/htui-store/tests/pg_conformance.rs:21` move with their comments. | The seam's contract runs against `MemStore` and `PgStore` alike. |
| D8 | **No re-probe on save.** Saving changes the digest; the next `p` or the next connect re-probes (MOD-7 D18). The section says so (D6); it does not send `ProbeBox` itself. | The probe runs on this box only and is the maintainer's key; an automatic probe on save would also fire for an edit made while looking at another box. |

## Files to change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/model/box_.rs` | UPDATE | T1 | `BOX_PROBE_SPEC_KEY` (D1) |
| `crates/htui-core/src/model/mod.rs` | UPDATE | T1 | re-export it (D1) |
| `crates/htui-agent/src/box_probe/spec.rs` | UPDATE | T1 | `SETTING_KEY` aliases the core constant; `check`, `SPEC_REFUSED` + unit tests (D1, D3) |
| `crates/htui-agent/tests/box_probe.rs` | UPDATE | T1 | `check` agrees with `effective` over the existing overlay fixtures (accept → same digest; refuse → same fault) |
| `crates/htui-core/src/store/traits.rs` | UPDATE | T2 | the two methods and their docs (D2) |
| `crates/htui-core/src/store/mem.rs` | UPDATE | T2 | `MemStore` impl over `app_settings` (`:175`) |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | T2 | two cases, `CASES` (D7) |
| `crates/htui-core/tests/mem_store.rs` | UPDATE | T2 | pin 104 → 106 |
| `crates/htui-store/src/pg/write.rs` | UPDATE | T2 | `PgStore` impl (D2) |
| `crates/htui-store/src/writer.rs` | UPDATE | T2 | `Writer` dispatch arms |
| `crates/htui-store/tests/pg_conformance.rs` | UPDATE | T2 | `EXPECTED_CASES` 104 → 106 |
| `crates/htui-store/.sqlx/` | REGENERATE | T2 | new query hashes, if any (R-3) |
| `crates/htui-agent/src/conformance.rs` | UPDATE | T2 | `UsageSpy` forwards (`:741`) |
| `crates/htui-agent/tests/recorder.rs` | UPDATE | T2 | `SpyStore` forwards (`:429`) |
| `crates/htui/src/box_settings.rs` | UPDATE | T3 | `SpecView.stored`, `snapshot`, `SetProbeSpec` in `serve`, `REQUEST_NAMES` (D3–D5) |
| `crates/htui/src/store_worker.rs` | UPDATE | T3 | variant + doc, `name()`, routing arm, name pin test (D4) |
| `crates/htui/tests/box_settings.rs` | UPDATE | T3, T4 | worker half (T3), section half (T4) |
| `crates/htui/src/ui/tabs/settings/boxes.rs` | UPDATE | T4 | `Mode::Spec`, `s`, hints, reply handling (D6) |
| `crates/htui/tests/snapshots/box_settings__*.snap` | UPDATE + CREATE | T4 | hint change moves the existing frames; new `spec_editor` frame(s) |

## Tasks

All four run **serially** (T2 needs T1's constant; T3 needs T2's seam; T4 needs T3's request and
snapshot). Each implementer commits at its task boundary. TDD per task: the tests named under
**Tests first** are written and seen failing before the implementation.

### T1 — key constant and pure checker
- **Action**: D1 and D3's `spec.rs` half.
- **Tests first**: `check` accepts `{}` and the terraform overlay with `effective`'s digest;
  refuses `42`, `{"nope": 1}`, a non-bare tool name, a bad regex, with the same fault text
  `effective` puts after `SPEC_IGNORED: `; `SETTING_KEY == BOX_PROBE_SPEC_KEY`.
- **Mirror**: `effective`'s `merge` call (`spec.rs:137-164`).
- **Validate**: `cargo test -p htui-agent --all-features box_probe`.

### T2 — store seam across five implementations
- **Action**: D2, D7.
- **Tests first**: the two conformance cases (fail to compile, then fail, then pass on `MemStore`).
- **Mirror**: `set_setting` / `clear_setting` `App` arms (Pg and Mem), `edit_box` forwarding in the
  two spies.
- **Validate**: `cargo test -p htui-core --all-features`, then the Postgres conformance
  (`HTUI_TEST_DATABASE_URL` set in the sandbox) `cargo test -p htui-store --all-features --test
  pg_conformance -- --test-threads=1`; `cargo sqlx prepare --check` against a migrated scratch DB.

### T3 — worker
- **Action**: D3's `serve` half, D4, D5.
- **Tests first** (`crates/htui/tests/box_settings.rs`, over `MemStore`): set applied → `Boxes`
  with the overlay in force and its token; spent token → `BoxesStale`, nothing written; a refused
  overlay → `Err(Constraint)` starting `box_probe_spec refused:` and nothing written; clear under
  the token → `Boxes` with `stored: None`; offline → `Unreachable`; the name pin.
- **Mirror**: `EditBox` arm of `box_settings::serve`.
- **Validate**: `cargo test -p htui --all-features --test box_settings -- --test-threads=1` and
  `cargo test -p htui --lib store_worker`.

### T4 — section
- **Action**: D6.
- **Tests first**: `s` opens the editor over the pretty-printed overlay / empty; `s` with no box
  listed still opens; `ctrl-s` sends `SetProbeSpec` with the parsed value and the token; a parse
  error sends nothing; blank over no row sends nothing; blank over a row sends `overlay: None`;
  `BoxesStale` keeps the text and swaps the token; `Failed` keeps the editor; applied closes with
  the notice; snapshot `box_settings__spec_editor`.
- **Mirror**: `Mode::Quirks` end to end.
- **Validate**: `cargo test -p htui --all-features --test box_settings -- --test-threads=1`;
  `cargo insta` review of the moved and new frames.

## Validation (whole change)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1     # testkit needed, see memory
cargo sqlx prepare --check                                     # against a migrated scratch DB
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| # | Risk | Likelihood | Mitigation |
|---|---|---|---|
| R-1 | `updated_at` precision: the token round-trips Rust `DateTime<Utc>` (ns) ↔ `TIMESTAMPTZ` (µs). | Low | The existing `set_setting` App arm already relies on the round-trip; the conformance case's second write under the returned token proves it on Pg. |
| R-2 | Changing `HINT_BROWSE` / `HINT_NO_LIST` moves every existing `box_settings__*.snap` that shows the hint. | Certain | Expected churn; review each moved frame shows only the hint change. |
| R-3 | `.sqlx` drift: new `query!` texts need a prepare against a migrated DB (the compose `htui` DB is empty). | Medium | Sandbox Postgres on `localhost:5439`, scratch DB migrated first (`docs/hr-sandbox.md`); reusing the existing App-rung statements byte for byte (indentation included: the offline hash is the SHA-256 of the literal query string) keeps their hashes and adds no entry. |
| R-4 | A long overlay does not fit the editor height. | Low | The editor takes the body area; `TextArea` already scrolls to the cursor. |
| R-5 | The spies in `htui-agent` forward every `WriteStore` method; a missed forward is a compile error, not a silent gap. | Low | Compile catches it. |
| R-6 | A stored overlay that a *later* seed change makes invalid shows as "ignored" and opens in the editor as-is; saving it unchanged is refused. | Low | Intended: the refusal sentence says why; clearing (blank + `ctrl-s`) is always accepted. |

## Acceptance

- [ ] `s` → edit → `ctrl-s` stores the overlay; the spec line shows `seed + stored overlay` with the new digest.
- [ ] A spent token never overwrites; the editor reloads its token and keeps its text.
- [ ] An overlay the probe would ignore is refused with the probe's fault sentence; nothing is stored.
- [ ] Blank + `ctrl-s` clears the row; the spec line returns to `seed`.
- [ ] Two new conformance cases pass on `MemStore` and `PgStore`; `CASES` = 106.
- [ ] Whole-change validation passes; reviewer (`rust-reviewer`) findings applied or deferred.

## Verified claims

Checked against the tree at `de177dd` (2026-10-01). Task independence: none claimed — T1→T4 are
serial, so no file-set intersection is needed (T3 and T4 share `crates/htui/tests/box_settings.rs`,
which the serial order already covers).

| Claim | Verdict | Evidence |
|---|---|---|
| Five `WriteStore` implementations | ✓ | `impl … WriteStore for` at `htui-agent/src/conformance.rs:741`, `htui-store/src/writer.rs:310`, `htui-store/src/pg/write.rs:714`, `htui-agent/tests/recorder.rs:429`, `htui-core/src/store/mem.rs:5801` |
| `StoredSetting { value: Option<Value>, updated_at }`, derives `Eq` | ✓ | `traits.rs:2306-2312` |
| `CasOutcome<T>` = `Applied`/`Stale` | ✓ | `traits.rs:2138-2143` |
| `app_setting` has `updated_at` stamped on UPDATE, `DEFAULT now()` on INSERT; no migration needed | ✓ | `0001_init.sql:557-561` and the trigger loop that follows |
| Pg `set_setting` App arm: `INSERT … ON CONFLICT DO NOTHING` / `UPDATE … AND updated_at = $3`; a miss under a token with no row is `NotFound` (so D2's `Stale(None)` is a deliberate difference) | ✓ | `pg/write.rs:3209`, `:3236`, `:3248`, `:3266`; `cas_miss` `:82-93` |
| Mem App-rung CAS `(stored, expected)` match, token from `now` | ✓ | `mem.rs:3305-3345` |
| `SETTING_KEY` readers: `agent_worker.rs:2339`, `box_settings.rs:70`, tests; none in `htui-worker` | ✓ | text search over `crates/` |
| `spec::effective` warns on a rejected overlay | ✓ (line amended `:152` → `:156`) | `spec.rs:137`, `warn!` `:156`; private `merge` `:211` is what `check` reuses |
| `htui-agent` depends on `htui-core` (alias compiles) | ✓ | `crates/htui-agent/Cargo.toml:18` |
| `model/mod.rs` re-exports `BoxEdit` (site for the new re-export) | ✓ | `model/mod.rs:105` |
| `StoreRequest` derives only `Debug, Clone`; `Value` field is fine | ✓ | `store_worker.rs:104` |
| Box requests routed by one or-ed arm; name pin test exists | ✓ | `store_worker.rs:1533`, `:3604-3620` |
| `REQUEST_NAMES` has two entries today | ✓ | `box_settings.rs:128` (`["boxes", "edit_box"]`) |
| `s` is unbound in the section and globally (only a test overlay in `app/update.rs:1090` uses it) | ✓ | `boxes.rs:562-611`; `keymap.rs` has no `Char('s')` |
| Browse hint is `j/k move · t tags · e quirks · w executor · p probe this box · r reload`; no-list hint `r reload` | ✓ | `boxes.rs:51`, `:54` |
| Quirks editor: `ctrl-s` → `FieldOutcome::Submit` via `TextArea::on_key` | ✓ | `boxes.rs:420-445`; `text_area.rs:1060` |
| `TextArea` scrolls its viewport to the cursor (R-4) | ✓ | `text_area.rs:27`, test `:1166` |
| `is_error` colours notices starting `changed elsewhere` | ✓ | `ui/tabs/settings/mod.rs:67-69` |
| `CASES` = 104; pins at `mem_store.rs:36-37` and `pg_conformance.rs:21` | ✓ | read at those lines |
| Existing `box_settings__*.snap`: 7 frames (hint change moves those that draw the hint) | ✓ | `ls crates/htui/tests/snapshots` |
| sqlx offline entry name = SHA-256 of the literal query string (R-3 reuse) | ✓ (compile-free probe) | `query-39fbb9fb….json`: `sha256(query)` equals its `hash` field and file name |
| Gate commands (`--all-features`, `--test-threads=1`, `sqlx prepare --check`) | ✓ | `docs/hr-sandbox.md:184`, `:196-205` |
