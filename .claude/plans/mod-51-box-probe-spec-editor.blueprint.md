# Blueprint: MOD-51, the `box_probe_spec` editor in `Settings > Boxes`

**Status**: **accepted** (2026-10-01). **Maintainer decisions on the ack findings (2026-10-01):**
F-3 accepted (separate `HINT_RELOAD = "r reload"` before the first read and over a refused read);
F-4 accepted (a blank save closes with nothing sent when `opened_on` **or** `expected` is `None`);
F-10 resolved by **shortening `p probe this box` to `p probe`** in `HINT_BROWSE` (71 chars, fits an
80-column terminal); `THIS_BOX_ONLY` still explains `p` on another box. §3.4's `HINT_BROWSE` is
already amended; R-3 is closed. Findings F-1 to F-21 (§0) are this blueprint's. Plan
decisions D1–D8 are confirmed and are not reopened here; where a finding touches one, the Fix column
is the narrowest implementation reading that keeps the decision's text true, and it is marked
**needs main-thread ack** when it adds a state the decision did not name.

**Plan**: `.claude/plans/mod-51-box-probe-spec-editor.plan.md` (confirmed 2026-10-01). Historical
shape: MOD-7 D52 (`.claude/plans/mod-7-box-settings-section.plan.md:200`). Style: the MOD-7
milestone 2 blueprint.

**Verified at**: HEAD `626d5bf`, branch `hr/MOD-51`, clean tree. Every anchor was located through
Gortex (`search` symbols/text, `read` source/file windows, `relations` usages) and re-read at its
line. **Line numbers are pre-edit**: a citation into a file a task edits moves after that task's
first commit. `crates/htui-store/.sqlx/` holds **291** files, `crates/htui/tests/snapshots/` holds
**121** `.snap` files, `df -h /` shows 62 GB free (86 % used; project memory: disk pressure
crash-loops the dev Postgres, check before T2's Postgres gate).

**Graphify**: `graphify-out/` does not exist in this checkout; nothing here comes from it.

---

## 0. Findings

### 0.1 Plan claims found false

| # | Plan says | Tree at `626d5bf` | Fix |
|---|---|---|---|
| **F-1** | Files table, T1 row: "`spec.rs` … `check`, `SPEC_REFUSED` **+ unit tests**". | `crates/htui-agent/tests/box_probe.rs:11-12`: "The box-probe module keeps no `#[cfg(test)]` module of its own (D29), so every case lives here and the tool-name grep over `src/box_probe/*.rs` is literal." `spec.rs` (423 lines) has no test module. A unit-test module in `spec.rs` breaks MOD-7 blueprint D29. | Every T1 test goes in `crates/htui-agent/tests/box_probe.rs` (which the plan already lists). `spec.rs` gains no `#[cfg(test)]`. |
| **F-8** | T3 tests first: "a refused overlay → `Err(Constraint)` starting `box_probe_spec refused:`". | The worker tests call `htui::store_worker::serve` (`tests/box_settings.rs:16`), which turns every `Err` into `Failed { request, message: err.to_string() }` (`store_worker.rs:1411-1416`, `failed` `:1602-1607`), and `StoreError::Constraint`'s `Display` is `"constraint violated: {0}"` (`htui-core/src/store/error.rs:21-22`). So the message **contains** `box_probe_spec refused: …` after a prefix. "Starts with" holds only one layer down, at `box_settings::serve`. | The worker test asserts `Failed { request: "set_probe_spec", message }` with `message.contains(&format!("{SPEC_REFUSED}: {fault}"))`, **and** one direct `box_settings::serve` call asserts `Err(StoreError::Constraint(s))` with `s.starts_with("box_probe_spec refused: ")` (§4.4). |

The plan's "Verified claims" table otherwise holds at `626d5bf`: the five `impl … WriteStore for`
lines (`conformance.rs:741`, `writer.rs:310`, `pg/write.rs:714`, `recorder.rs:429`, `mem.rs:5801`),
`StoredSetting` `traits.rs:2306-2312` (`Eq`), `CasOutcome` `:2137-2143`, `app_setting`
`0001_init.sql:557-561`, `spec.rs:46`/`:137`/`:156`/`:211`, `model/mod.rs:105`,
`store_worker.rs:97`/`:104`/`:1533`/`:3604-3620`, `box_settings.rs:128`, `boxes.rs:51`/`:54`/`:60`/`:86`/
`:420`/`:613`, `settings/mod.rs:67-69`, `mem_store.rs:36-37`, `pg_conformance.rs:21`, 7
`box_settings__*` frames. `text_area.rs:1060` is the **test** `ctrl_s_submits_and_esc_cancels`; the
submit arm itself is `text_area.rs:194` (evidence still valid).

### 0.2 Hazards and gaps

| # | Blocker? | Where | Hazard | Fix |
|---|---|---|---|---|
| **F-2** | Non-blocker (doc gate) | `spec.rs` `check` doc | `merge` is private (`spec.rs:211`); the house style denies private intra-doc links (MOD-7 blueprint house style). ``[`merge`]`` in `check`'s doc fails `cargo doc`. | Plain backticks for `merge`; links only to `effective`, `SPEC_IGNORED`, `SPEC_REFUSED`. Same for `box_settings::serve` / `htui_agent::…` in `htui-core` docs (another crate): plain backticks. |
| **F-3** | **needs main-thread ack** (D6 hint table) | `boxes.rs:54`, `:901-918` | D6: `HINT_NO_LIST` becomes `s spec · r reload`. Today `HINT_NO_LIST` is drawn whenever `listed` is false: **no snapshot yet**, **a refused read** (`unavailable`), and an empty list. D6 itself says `s` works only "when a snapshot is present and the read was not refused", so the literal change advertises `s` in two states where it does nothing, and moves `box_settings__offline.snap` (last content line `r reload`) to a hint that lies. | Keep D6's constant as written and use it exactly where `s` works (snapshot present, not refused, no box listed). Add `HINT_RELOAD = "r reload"` for not-read and refused. `box_settings__offline` then does **not** move. If the main thread prefers D6 literally, drop `HINT_RELOAD` and accept the offline frame move. |
| **F-4** | **needs main-thread ack** (D6 blank rule) | `boxes.rs` `submit` | D6: "blank → `overlay: None` (clear); blank over `opened_on: None` → close". After a save that answers `Stale(None)` (row deleted meanwhile), `expected` becomes `None` while `opened_on` stays `Some` (never refreshed). Blank + `ctrl-s` then sends `None + None`, which the store refuses as a `Constraint` (D2), so the user sees a refusal for "clear what is already gone". | Blank closes with nothing sent when `opened_on.is_none() **or** expected.is_none()`; otherwise sends `overlay: None, expected`. D6's stated case is unchanged; only the gone-row case is added. |
| **F-5** | Non-blocker (wrong sentence) | `boxes.rs:81`, `:318-328`, `:467-471` | `IN_FLIGHT` is the literal `"edit_box in flight"`, used by `blocked()` and `submit()`. With a spec save in flight, `t`/`e`/`w`/a second `ctrl-s` would name the wrong request. | `IN_FLIGHT` becomes the suffix `"in flight"` and both sites say `format!("{busy} {IN_FLIGHT}")`. For `edit_box` the text is byte-identical, so no existing test or frame moves (`tests/box_settings.rs:675`, `:717`, `:737`, `:1106` still see `edit_box in flight`). |
| **F-6** | **Blocker** (three code paths misroute the spec editor) | `boxes.rs:222-229`, `:279-312`, `:653-682`, `:1027-1046` | (a) `on_stale` decides "editor or not" through `editor_box()`, which has no box for the spec editor, so a `BoxesStale` over an open spec editor would say `CHANGED_ELSEWHERE_CLOSED` and keep the old token. (b) `render`'s editor-survives-a-refusal arm requires `!snapshot.boxes.is_empty()` (`:659`), so a spec editor opened over an empty list would vanish under a refused read while still taking keys. (c) The spec line (`spec_lines`) is drawn only inside `detail()` of a selected box, so with no box listed the effective spec is never on screen. | (a) `on_stale` handles `Mode::Spec` **before** `editor_box()` (§5.4). (b) `render` draws the spec editor before the existing `match`, with the refusal line on top when `unavailable` is set (§5.5). (c) The spec editor body starts with `spec_lines` (§5.5). |
| **F-7** | Non-blocker (silent test gap) | `tests/box_settings.rs:262-285` | `offline_both_are_refused_with_the_database_sentence` zips two requests with `REQUEST_NAMES`. At 3 names, `zip` stops at 2 and `SetProbeSpec`'s offline refusal goes untested without failing. | T3 adds the third request, asserts `requests.len() == REQUEST_NAMES.len()`, and renames it `offline_every_box_request_is_refused_with_the_database_sentence` (doc: "all three"). |
| **F-9** | Non-blocker (`.sqlx`) | `pg/write.rs:3236-3259`, `:3372-3378`; `pg/read.rs:2579-2581` | The plan hopes to reuse existing statements. Verified: all four statements `PgStore` needs already have offline entries. SHA-256 of the literal query text: `SELECT` `e798257b…` (`stored_setting`, App arm), `INSERT` `39fbb9fb…` and `UPDATE` `174cf062…` (`set_setting` App arm), `DELETE` `04f5b72f…` (`clear_setting` App arm). Each `query-<hash>.json` exists. The raw strings carry **24 / 24 / 20 spaces** of indentation, the closing `"#` line's trailing spaces included. The new method sits at a different nesting, and any re-indent changes the hash. | Copy the four literals byte for byte (§3.3). **No new statement, no `cargo sqlx prepare` needed; `.sqlx` stays 291.** `cargo sqlx prepare --check -- --all-targets --all-features` against a migrated scratch DB is still the T2 gate. If it reports a diff, a literal was re-indented: fix the literal, never commit a fifth file. |
| **F-10** | **needs main-thread ack** (cosmetic) | `boxes.rs:51` | `HINT_BROWSE` grows from 71 to 80 chars. The section's inner width in an 80-column terminal is 78, so `r reload` clips to `r relo`. In the 100-column test frames (inner 98) it fits, but Browse with both `probing…` and `saving…` (reachable: save, `Esc`, `p`) is 101 and clips. No snapshot draws that state. | Blueprint default: D6 as written, `s spec` before `r reload`. Shortening `p probe this box` to `p probe` would fit 80 columns but changes existing wording outside D6. The main thread decides. |
| **F-11** | Non-blocker (bounded residue) | `boxes.rs:13-17`, `:269-276` | H-9 (R-29) carries over: `StoreReply::Boxes` does not name the request it answers. A plain read that lands between a spec save and its reply (activation, `r`, the re-read after a probe) is taken for the reply. It closes the editor and shows `SPEC_SAVED` before the real answer. The real answer then corrects the notice: `BoxesStale` shows `CHANGED_ELSEWHERE_CLOSED` (`busy` already cleared, editor closed), and `Failed` shows the sentence. The typed text is lost, as for `edit_box` today. | Carried and documented (module doc + R-1 here). No self-naming reply: that would be MOD-59's shape for boxes and is out of scope. |
| **F-12** | Non-blocker (the `htui-core` gate) | `conformance.rs:13110-13216` | `every_cross_referenced_test_name_exists` panics on a backticked `a::b` span whose right half has ≥4 underscores, and asserts that any backticked bare snake_case name with ≥4 underscores is defined in `conformance.rs` or `mem.rs`. | New case docs cite only the two new case fns (defined in `conformance.rs`) or nothing. **Do not** cite the `htui` worker/section tests by name. `set_box_probe_spec` (3 underscores) and `box_probe_spec` are safe to backtick. |
| **F-13** | Non-blocker (platform) | `mem.rs:66-70`, `:850-852` | `MemStore` tokens are wall-clock untruncated. The doc says back-to-back compare-and-sets share one microsecond about half the time, and only nanoseconds keep them apart. Case 1 updates under `row1`'s token and then proves the same token spent, so it relies on `row2.updated_at != row1.updated_at`, like the existing App-rung and project cases do. | The case asserts `row2.updated_at > row1.updated_at` with a message, so a coarse-clock platform fails loudly rather than as a confusing `Applied`. |
| **F-14** | Non-blocker (cosmetic) | Postgres `JSONB` | `JSONB` re-orders object keys (by length, then bytes) and drops duplicate keys. The editor reopens on `to_string_pretty` of the stored value, so after a save on Postgres the key order differs from what was typed. `MemStore` keeps `serde_json`'s default `BTreeMap` order (the workspace `serde_json = "1"`, `Cargo.toml:32`, has no `preserve_order`). `Value` equality is map equality, so the conformance read-backs hold. | Fixtures use no floats (numeric round-trip) and no duplicate keys. Snapshots run over `MemStore` only. |
| **F-15** | Non-blocker (don't over-mirror) | `boxes.rs:493-503` | The quirks editor closes on unchanged text. D6 lists no such rule for the spec editor, and R-6 ("saving it unchanged is refused") depends on an unchanged save being **sent**. | No unchanged short-circuit in the spec editor, except D6's blank-over-no-row (F-4). An unchanged valid save re-stamps `updated_at`. The digest is unchanged, so nothing re-probes. |
| **F-16** | Non-blocker (docs that go stale) | `box_settings.rs:1-11`, `:58-60`, `:74-82`, `:124-128`; `store_worker.rs:1152-1157`, `:1530-1532`, `:846`, `:3604-3605`; `boxes.rs:1-17`, `:83-86`, `:96`, `:110`; `spec.rs:13-24`; `traits.rs:2303-2304` | They say "two" requests, "box edit", "`Boxes` and `EditBox`", "`backend.app_settings()` for `SETTING_KEY`", "Milestone 1's edit path is SQL", and "`set_setting`/`clear_setting` must present". | Each task updates the docs in its own files (listed per task). The `try_serve` arm comment becomes "The three box requests, or-ed for the same reason the twenty-nine above are". The count above the arm does not change. |
| **F-17** | Non-blocker (pin message already stale) | `pg_conformance.rs:18-20`, `:27-28` | The message still says "103 since MOD-41 T7's three executor edit cases" beside `EXPECTED_CASES = 104`. | T2 rewrites the comment and the message for 106 (§3.6). |
| **F-18** | Non-blocker (log noise, existing) | `box_settings.rs:49-56` | `spec_view` calls `spec::effective`, which `warn!`s once per read when a stored overlay is invalid. So every `Boxes` read over an ignored overlay logs. That behaviour exists today, and D5 keeps `spec_view` "as today". | Unchanged (D3: `effective` untouched). Recorded so the reviewer does not flag it as new. |
| **F-19** | Non-blocker (test helper) | `tests/box_settings.rs:401-411` | Typing JSON with `type_at` sends one chord per char, and `{`, `"`, `:` are unproven chord names. | Section tests put JSON in with `SectionBench::paste` (`testkit.rs:687-690`), which the spec editor's `on_paste` arm serves. Clearing uses `"backspace"` (a known chord, `keymap.rs:141`) with the cursor at the end. |
| **F-20** | Non-blocker (compile) | `boxes.rs:222-229`, `:420-460`, `:467-525`, `:551-560`, `:901-918`, `:279-312` | Six exhaustive `match`es over `Mode` must gain the new variant (`editor_box`, `on_editor_key`, `submit`, `on_paste`, `hint_text`, `on_stale`'s tuple match). `detail()`'s three matches end in `_` and need nothing. | Listed arm by arm in §5.3–§5.5. |
| **F-21** | Non-blocker (signature) | `box_settings.rs:47-56`, `tests/box_settings.rs:146` | `spec_view(stored: Option<&Value>)` changes to take the row. The only callers are `snapshot` (`:70`) and one test, `assert_eq!(snapshot.spec, spec_view(None))`, which keeps compiling with `Option<StoredSetting>`. `SpecView` and `BoxesSnapshot` are never built by literal outside `box_settings.rs` (text search). | `pub fn spec_view(stored: Option<StoredSetting>) -> SpecView`. |

### 0.3 Every site that enumerates `StoreRequest` variants or the box request names

| Site | What changes | Task |
|---|---|---|
| `store_worker.rs:105` `enum StoreRequest` | `SetProbeSpec { overlay, expected }` after `EditBox` (`:229-236`), before `CreateAgent`'s doc (`:237`) | T3 |
| `store_worker.rs:846-849` `StoreRequest::name` (wildcard-free) | `Self::SetProbeSpec { .. } => "set_probe_spec",` after `EditBox`; comment "The three of `box_settings::REQUEST_NAMES`" | T3 |
| `store_worker.rs:1530-1535` `try_serve` (wildcard-free) | `StoreRequest::Boxes \| StoreRequest::EditBox { .. } \| StoreRequest::SetProbeSpec { .. } => box_settings::serve(..)` | T3 |
| `store_worker.rs:3604-3620` `box_requests_are_named_as_box_settings_lists_them` | third element `SetProbeSpec { overlay: None, expected: None }.name()`; doc "three" | T3 |
| `box_settings.rs:124-128` `REQUEST_NAMES` | `[&str; 3] = ["boxes", "edit_box", "set_probe_spec"]` | T3 |
| `box_settings.rs:83-122` `serve` | new arm; the `other =>` arm comment "three variants" | T3 |
| `tests/box_settings.rs:262-285` offline zip | third request + length assert (F-7) | T3 |
| `boxes.rs:97` `EDIT_NAME = REQUEST_NAMES[1]` | add `SPEC_NAME = REQUEST_NAMES[2]` | T4 |
| `boxes.rs:613-640` `on_reply` `Failed` arms | new `Failed { request } if *request == SPEC_NAME` arm | T4 |
| `store_worker.rs:2076` (`spawn_with_concepts`), `testkit.rs:287` (`Harness::drive`), `agent_worker.rs:1114` | Runtime-routed lists that end in wildcards. `EditBox` is in none of them, and `SetProbeSpec` is served in the loop like `EditBox`. **Not touched.** | — |

No other file names `EditBox`, `"edit_box"` or `box_settings::REQUEST_NAMES` (text search:
`box_settings.rs`, `store_worker.rs`, `boxes.rs`, `tests/box_settings.rs`, `tests/box_probe_pg.rs`
only; the last builds `EditBox` requests and needs nothing).

---

## 1. Build order and validation, at a glance

Serial, one branch (`hr/MOD-51`), no worktree. Each implementer commits at its task boundary (project
memory: uncommitted subagent work dies with the session; stage own paths only, never `-A`, never
`stash`).

| Task | Crate(s) | Commits (each compiles) | Gate |
|---|---|---|---|
| T1 key + checker | htui-core, htui-agent | 2 (§2.5) | `cargo test -p htui-agent --all-features --test box_probe -- --test-threads=1`; `cargo test -p htui-core --all-features --lib -- --test-threads=1` |
| T2 store seam | htui-core, htui-store, htui-agent | 3 (§3.8) | `cargo test -p htui-core --all-features -- --test-threads=1`; Postgres `pg_conformance`; `cargo test -p htui-agent --all-features -- --test-threads=1`; `sqlx prepare --check`; `.sqlx` = 291 |
| T3 worker | htui | 2 (§4.6) | `cargo test -p htui --all-features --test box_settings -- --test-threads=1`; `cargo test -p htui --all-features --lib store_worker -- --test-threads=1` |
| T4 section | htui | 3 (§5.8) | `cargo test -p htui --all-features -- --test-threads=1`; `cargo insta test -p htui --all-features --review` (accept exactly the frames of §5.7) |
| close | — | — | workspace gate (§7) |

`--all-features` is required for `crates/htui/tests/*.rs` (project memory: without `testkit` they
run 0 tests and report ok).

---

## 2. T1: the key constant and the pure checker (D1, D3 `spec.rs` half)

**First failing test**: `check_refuses_what_effective_ignores_with_the_same_fault`.

**Files**: `crates/htui-core/src/model/box_.rs`, `crates/htui-core/src/model/mod.rs`,
`crates/htui-agent/src/box_probe/spec.rs`, `crates/htui-agent/tests/box_probe.rs`.

### 2.1 `box_.rs` (after `DECLARED_TAG_MAX`, `:148-149`)

```rust
/// The `app_setting` key of the box probe spec overlay (MOD-7 D52, MOD-51 D1): the one spelling
/// the store's typed writer (`WriteStore::set_box_probe_spec`) and the probe
/// (`htui_agent::box_probe::spec::SETTING_KEY`, an alias of this) share, so the two cannot
/// disagree on the key while `htui-core` stays free of `htui-agent`.
pub const BOX_PROBE_SPEC_KEY: &str = "box_probe_spec";
```

Plain backticks: the trait method arrives in T2, and rustdoc denies broken links.

### 2.2 `model/mod.rs:104-108`

`pub use box_::{ BOX_PROBE_SPEC_KEY, BoxEdit, … }`. `cargo fmt` orders the list; uppercase sorts
before `BoxEdit`.

### 2.3 `spec.rs`

- `:45-46`: 
  ```rust
  /// The `app_setting` key of the stored overlay (plan D17): `htui_core`'s
  /// [`BOX_PROBE_SPEC_KEY`](htui_core::model::BOX_PROBE_SPEC_KEY), aliased so every reader of
  /// this name keeps compiling and the store cannot spell it differently (MOD-51 D1).
  pub const SETTING_KEY: &str = htui_core::model::BOX_PROBE_SPEC_KEY;
  ```
  (A cross-crate public link; `htui-agent` depends on `htui-core`, `Cargo.toml:18`. The fully
  qualified path is not a `use`, so `unused_qualifications` does not fire.)
- After `SPEC_IGNORED` (`:48-49`):
  ```rust
  /// The first words of an overlay the `Settings > Boxes` editor tried to store and the worker
  /// refused before any write (MOD-51 D3). [`SPEC_IGNORED`] is the probe-time twin: the same
  /// fault sentence follows either prefix.
  pub const SPEC_REFUSED: &str = "box_probe_spec refused";
  ```
- After `effective` (`:137-164`), before `digest` (`:166-174`):
  ```rust
  /// Whether the probe would run under `overlay` (MOD-51 D3): the same `merge` into the seed
  /// that [`effective`] runs, so the editor refuses exactly the overlays the probe would ignore,
  /// with the probe's own sentence. Pure and **silent**: unlike [`effective`] it logs nothing,
  /// because a refused edit is not a probe event.
  ///
  /// Answers the merged spec's [`digest`], which is what the next probe records.
  ///
  /// # Errors
  ///
  /// The first fault, as one sentence naming its key, **without** a prefix: the worker puts
  /// [`SPEC_REFUSED`] in front of it, and [`effective`] puts [`SPEC_IGNORED`] in front of the
  /// same text.
  pub fn check(overlay: &Value) -> Result<String, String> {
      merge(seed(), overlay).map(|spec| digest(&spec))
  }
  ```
  (F-2: `merge` in plain backticks.)
- Module doc `:13-24`: after the `DELETE` block, add one paragraph: "Since MOD-51 the maintainer's
  path is `Settings > Boxes`, `s`: a JSON editor over this row, saved as a compare-and-set on its
  `updated_at`, refusing (through [`check`]) every overlay the probe would ignore." Keep the SQL.
  "Milestone 1's edit path is SQL" becomes "Milestone 1's edit path was SQL, and still works".

### 2.4 Tests (`crates/htui-agent/tests/box_probe.rs`, "The stored overlay" block after `:925-948`)

Import (`:29-32`) gains `SPEC_REFUSED`, `check`. New fixture helper `fn same_fault(stored: &Value)
-> String`: `effective(seed(), Some(stored)).error` stripped of `format!("{SPEC_IGNORED}: ")`,
panicking if absent.

| Test | Asserts |
|---|---|
| `check_accepts_what_effective_merges_with_the_same_digest` | For `json!({})`, the terraform overlay of `a_stored_tool_is_added` (`:736-741`), `{"tools":{"cmake":{"disabled":true}}}` and the two-rule `tags` overlay of `a_stored_tag_rule_replaces_in_place`: `check(&v) == Ok(effective(seed(), Some(&v)).digest)` and `effective(..).error == None`. `check(&json!({})) == Ok(digest(seed()))`. |
| `check_refuses_what_effective_ignores_with_the_same_fault` | For `json!(42)`, `{"nope": 1}`, `{"tools":{"x":{"kind":"path","names":["bin/x"]}}}` (non-bare), `{"tools":{"x":{"kind":"path","names":["x"],"version":{"args":[],"pattern":"("}}}}` (bad regex), `{"tags":[{"tag":"t","any_tool":["nosuch"]}]}` (missing tool), `{"tools":{"myglob":{"kind":"glob","patterns":["~/bin/x"]}}}` (not `path`): `check(&v) == Err(same_fault(&v))`, and the fault names its key (`"not a JSON object"`, `` "`nope`" ``, `"bin/x"`, `"tools.x.version.pattern"`, `"nosuch"`, `"myglob"`). |
| `the_refusal_prefix_is_not_the_ignore_prefix` | `SPEC_REFUSED == "box_probe_spec refused"`, `SPEC_REFUSED != SPEC_IGNORED`, and both start with `SETTING_KEY`. |
| `the_setting_key_is_the_core_constant` | `SETTING_KEY == htui_core::model::BOX_PROBE_SPEC_KEY` and `== "box_probe_spec"`. (`:182`'s literal assertion stays.) |

That `check` does not log is pinned by code review (no `warn!` in its body); `htui-agent`'s tests
carry no log-capture fixture, and adding one is out of scope.

### 2.5 Gate and commits

```bash
cargo test -p htui-agent --all-features --test box_probe -- --test-threads=1
cargo test -p htui-core --all-features --lib -- --test-threads=1
cargo clippy -p htui-agent -p htui-core --all-features --all-targets -- -D warnings
```

1. (a) Red: `BOX_PROBE_SPEC_KEY`, re-export, the alias, `SPEC_REFUSED`, `check` with a `todo!()`
   body (no caller outside the tests until T3), the four tests.
2. (b) Green: `check`'s body, the module doc.

---

## 3. T2: the store seam on five implementations (D2, D7)

**First failing test**: `store::conformance::box_probe_spec_is_cas_on_updated_at` over `MemStore`
(through `mem_store_conformance` and `run_case_accepts_every_name_in_cases`). T2 starts from T1.

**Files**: `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`,
`crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`,
`crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/writer.rs`,
`crates/htui-store/tests/pg_conformance.rs`, `crates/htui-agent/src/conformance.rs`,
`crates/htui-agent/tests/recorder.rs`. **No `.sqlx` change** (F-9).

### 3.1 `traits.rs`

**Refusal sentences**: after `BOX_SETTINGS_NOT_AN_OBJECT` (`:1720-1722`):

```rust
/// MOD-51 D2: [`WriteStore::set_box_probe_spec`]'s refusal of an overlay that is not a JSON
/// object. The probe would ignore one too; the store refuses it without the probe's help.
pub const BOX_PROBE_SPEC_NOT_AN_OBJECT: &str = "box_probe_spec is not a JSON object";

/// MOD-51 D2: [`WriteStore::set_box_probe_spec`]'s refusal of a clear with no token: a delete is
/// a compare-and-set on the row it deletes, and "expect no row" has nothing to delete.
pub const BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN: &str =
    "box_probe_spec clear needs the updated_at of the row it clears";
```

**Methods**: inside `WriteStore`, after `edit_box` (`:511-512`), the box group:

```rust
    /// The stored `app_setting` row keyed
    /// [`BOX_PROBE_SPEC_KEY`](crate::model::BOX_PROBE_SPEC_KEY) and its compare-and-set token
    /// (MOD-51 D2, MOD-7 D52), **unvalidated**: whether the probe would accept the value is
    /// `htui-agent`'s question (`htui_agent::box_probe::spec::effective`), not the store's.
    ///
    /// `None` when there is no row; a row always answers `value: Some`.
    ///
    /// # Errors
    ///
    /// Whatever the backend's read fails with.
    async fn box_probe_spec(&self) -> Result<Option<StoredSetting>>;

    /// Sets, replaces or clears the `box_probe_spec` overlay, a compare-and-set on the row's
    /// `updated_at` (MOD-51 D2, `docs/ANA-16.md` C7). The narrow typed writer beside the `App` rung
    /// of [`set_setting`](WriteStore::set_setting), whose keys are the closed `SettingKey` enum.
    ///
    /// - `overlay: Some(v)`, `expected: None` ("I expect no row"): inserts. A row already there is
    ///   [`CasOutcome::Stale`] carrying it, never an overwrite.
    /// - `overlay: Some(v)`, `expected: Some(t)`: replaces the value where `updated_at = t`.
    /// - `overlay: None`, `expected: Some(t)`: deletes the row where `updated_at = t`, answering
    ///   `Applied(None)`.
    ///
    /// `Applied(Some(row))` carries the value as written and the store's new token. A miss is
    /// `Stale` with the row as it is now, and **`Stale(None)` when the row is gone**. That differs
    /// from `set_setting`'s `NotFound` on purpose: a row deleted under an open editor is a miss
    /// like a spent token (MOD-7 D48), and the editor retries with `expected: None`. Nothing is
    /// written on a miss.
    ///
    /// The store checks only what it can know without the probe. Whether the keys, tools and
    /// patterns mean anything is checked by the caller first (`htui::box_settings::serve`, through
    /// `htui_agent::box_probe::spec::check`).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) carrying
    /// [`BOX_PROBE_SPEC_NOT_AN_OBJECT`] for a `Some(v)` that is not a JSON object, or
    /// [`BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN`] for `None` with `expected: None`. Both are decided
    /// before any read, so they win over `Stale`, and a refusal writes nothing.
    async fn set_box_probe_spec(
        &self,
        overlay: Option<Value>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Option<StoredSetting>>>;
```

`Value`, `DateTime`, `Utc`, `StoredSetting` and `CasOutcome` are already in scope in `traits.rs`
(`set_setting`, `:950`). `StoredSetting`'s doc (`:2303-2304`) becomes "…a later
[`WriteStore::set_setting`], [`WriteStore::clear_setting`] or [`WriteStore::set_box_probe_spec`]
must present."

### 3.2 `MemStore` (`mem.rs`)

- Imports: model list (`:23…`) gains `BOX_PROBE_SPEC_KEY`; traits list (`:52`) gains
  `BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN`, `BOX_PROBE_SPEC_NOT_AN_OBJECT`.
- `State`, after `clear_setting` (`:3426-3511`):

```rust
    /// The `box_probe_spec` row with its token (MOD-51 D2); `app_settings` (`:175`) holds it
    /// beside the ten `SettingKey` rows, under a key no `SettingKey` spells.
    fn box_probe_spec(&self) -> Option<StoredSetting> {
        self.app_settings
            .get(BOX_PROBE_SPEC_KEY)
            .map(|(value, updated_at)| StoredSetting {
                value: Some(value.clone()),
                updated_at: *updated_at,
            })
    }

    /// The overlay's compare-and-set (MOD-51 D2): both refusals first, then the App rung's
    /// `(stored, expected)` rule (`set_setting`, `:3322-3345`), except that a miss over no row is
    /// `Stale(None)`. `now` stands in for Postgres's `DEFAULT now()` / `set_updated_at`.
    fn set_box_probe_spec(
        &mut self,
        overlay: Option<Value>,
        expected: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Option<StoredSetting>>>;
```

  Body, in order:
  1. `if overlay.as_ref().is_some_and(|v| !v.is_object())` → `Err(Constraint(BOX_PROBE_SPEC_NOT_AN_OBJECT.to_owned()))`.
  2. `if overlay.is_none() && expected.is_none()` → `Err(Constraint(BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN.to_owned()))`.
  3. `let stored = self.box_probe_spec();` `let current = match (&stored, expected) { (None, None) => true, (Some(row), Some(want)) => row.updated_at == want, (None, Some(_)) | (Some(_), None) => false };`
  4. `if !current { return Ok(CasOutcome::Stale(stored)); }`
  5. `Some(value)`: `self.app_settings.insert(BOX_PROBE_SPEC_KEY.to_owned(), (value.clone(), now))` → `Applied(Some(StoredSetting { value: Some(value), updated_at: now }))`. `None`: `self.app_settings.remove(BOX_PROBE_SPEC_KEY)` → `Applied(None)`.
- `WriteStore for MemStore`, after `edit_box` (`:5884-5894`):

```rust
    async fn box_probe_spec(&self) -> Result<Option<StoredSetting>> {
        Ok(self.read(State::box_probe_spec))
    }

    async fn set_box_probe_spec(
        &self,
        overlay: Option<Value>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Option<StoredSetting>>> {
        let now = self.now();
        self.write(|state| state.set_box_probe_spec(overlay, expected, now))
    }
```

  `self.now()` is read before `write` (the F-K rule of MOD-7 carried; `now` takes no lock, but the
  shape stays uniform). `MemStore::set_app_setting` (`:554-557`, tests only) still plants a raw row
  under the same key, which T3's ignored-overlay test relies on.

### 3.3 `PgStore` (`pg/write.rs`, inside `impl WriteStore for PgStore` at `:714`, after `edit_box` `:1531-1613`)

- Imports: model list (`:22-38`) gains `BOX_PROBE_SPEC_KEY`; `:42` becomes `use
  htui_core::store::traits::{BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN, BOX_PROBE_SPEC_NOT_AN_OBJECT,
  BOX_SETTINGS_NOT_AN_OBJECT, EXECUTOR_MUST_BE_KNOWN};`. `StoredSetting`, `CasOutcome`, `Value`,
  `DateTime`, `Utc`, `WriteStore` are already imported (`:21`, `:43-46`, `:56`).

```rust
    /// The overlay row (MOD-51 D2): `PgStore::stored_setting`'s `App` statement, byte for byte,
    /// bound to [`BOX_PROBE_SPEC_KEY`], so it reuses that statement's offline entry.
    async fn box_probe_spec(&self) -> Result<Option<StoredSetting>>;

    /// The overlay's compare-and-set (MOD-51 D2): the `App` rung's three statements of
    /// `set_setting` and `clear_setting`, byte for byte, so no offline entry is added. A miss
    /// re-reads through [`box_probe_spec`](WriteStore::box_probe_spec) and answers `Stale` with
    /// whatever is there now, `None` included; unlike `cas_miss`, never `NotFound`.
    async fn set_box_probe_spec(
        &self,
        overlay: Option<Value>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Option<StoredSetting>>>;
```

**`box_probe_spec` body**: `sqlx::query!` with **exactly** the one-line literal of
`pg/read.rs:2580` (hash `e798257bef18e35b4d41997f0e874ac80b52257a479efe02b9d56932779039f2`):

```rust
        let row = sqlx::query!(
            "SELECT value, updated_at FROM app_setting WHERE key = $1",
            BOX_PROBE_SPEC_KEY,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(row.map(|row| StoredSetting {
            value: Some(row.value),
            updated_at: row.updated_at,
        }))
```

**`set_box_probe_spec` body**:
1. `if overlay.as_ref().is_some_and(|v| !v.is_object())` → `Err(Constraint(BOX_PROBE_SPEC_NOT_AN_OBJECT.to_owned()))`.
2. `let name = BOX_PROBE_SPEC_KEY;` `let landed: Option<DateTime<Utc>> = match (&overlay, expected) { … }`, whose four arms are:
   - `(None, None) => return Err(StoreError::Constraint(BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN.to_owned())),` (before any SQL, so D2's precedence holds; no `unreachable!`).
   - `(Some(value), None) =>` the **INSERT** of `set_setting`'s `None` arm (`:3236-3246`, hash `39fbb9fbf98bc9526966356c707e87315754342db995988a07e9318d498096d7`), binds `name, value`.
   - `(Some(value), Some(token)) =>` the **UPDATE** of its `Some(token)` arm (`:3247-3259`, hash `174cf062636909d34dd8140792cf4f81ca3656aa57ea4ddb365a8e7bf31286f0`), binds `name, value, token`.
   - `(None, Some(token)) =>` the **DELETE** of `clear_setting`'s `App` arm (`:3372-3383`, hash `04f5b72f9711b093631910f3c09b503a406e3ff7ffb9701aae6287d05aa84c61`), binds `name, token`.

   Each is `sqlx::query_scalar!(r#"…"#, …).fetch_optional(&self.pool).await.map_err(map_sqlx)?`.
   The literal text must be copied **exactly**. `·` below marks one space, and the leading newline
   and the trailing spaces before `"#` are part of the string:

   ```text
   INSERT, 24 spaces per line:
   r#"⏎
   ························INSERT INTO app_setting (key, value) VALUES ($1, $2)⏎
   ························ON CONFLICT (key) DO NOTHING⏎
   ························RETURNING updated_at AS "updated_at!"⏎
   ························"#

   UPDATE, 24 spaces (the WHERE line 25):
   r#"⏎
   ························UPDATE app_setting SET value = $2⏎
   ························ WHERE key = $1 AND updated_at = $3⏎
   ························RETURNING updated_at AS "updated_at!"⏎
   ························"#

   DELETE, 20 spaces:
   r#"⏎
   ····················DELETE FROM app_setting WHERE key = $1 AND updated_at = $2⏎
   ····················RETURNING updated_at AS "updated_at!"⏎
   ····················"#
   ```

   rustfmt never rewrites raw-string contents, so the literal may look mis-indented against its new
   nesting. That is intended, and a comment above the `match` says so: "The four literals are
   `set_setting`'s and `clear_setting`'s `App` statements byte for byte, indentation included:
   the offline cache keys a statement by the SHA-256 of its text (MOD-51 F-9)."
3. `match landed { Some(updated_at) => Ok(CasOutcome::Applied(overlay.map(|value| StoredSetting {
   value: Some(value), updated_at }))), None => Ok(CasOutcome::Stale(self.box_probe_spec().await?)) }`.

   The re-read after a miss is a second statement outside the write, the same race window every
   `cas_miss` caller has. A row inserted or deleted between the two is reported as it is at the
   re-read, which is "the row as it is now".

Rejected alternative: hoisting the four statements into shared private helpers used by
`set_setting`/`clear_setting` too. That would remove the duplication, but it rewrites two
behaviour-critical CAS paths for no behaviour gain.

**Postgres timing (R-1 carried)**: the INSERT stamps `DEFAULT now()` (transaction start) and the
UPDATE's trigger stamps `clock_timestamp()` (`0001_init.sql:21-25`, `:570-580`). The token
round-trips µs. The case's update under the returned token proves the round-trip.

### 3.4 `Writer` and the two spies

- `writer.rs`, after `edit_box` (`:425-435`):
  ```rust
      async fn box_probe_spec(&self) -> Result<Option<StoredSetting>> {
          match self {
              Self::Memory(store) => store.box_probe_spec().await,
              Self::Online(pg) => pg.box_probe_spec().await,
          }
      }

      async fn set_box_probe_spec(
          &self,
          overlay: Option<Value>,
          expected: Option<DateTime<Utc>>,
      ) -> Result<CasOutcome<Option<StoredSetting>>> {
          match self {
              Self::Memory(store) => store.set_box_probe_spec(overlay, expected).await,
              Self::Online(pg) => pg.set_box_probe_spec(overlay, expected).await,
          }
      }
  ```
  Imports unchanged (`set_setting`'s forward at `:767` already uses every type).
- `htui-agent/src/conformance.rs` (`UsageSpy`, after `edit_box` `:790-797`) and
  `htui-agent/tests/recorder.rs` (`SpyStore`, after `edit_box` `:498-505`): each forwards
  `self.inner.box_probe_spec().await` / `self.inner.set_box_probe_spec(overlay, expected).await`,
  returning `StoreResult<…>`. Imports unchanged (both already forward `set_setting`, `:1020`,
  `:747`).

### 3.5 Conformance cases (`CASES` 104 → 106)

Appended to `CASES` after `"an_executor_edit_is_a_compare_and_set"` (`conformance.rs:148`). `run_case`
arms after `:361-363`, before `other => panic!` (`:364`). The two `async fn`s after
`an_executor_edit_is_a_compare_and_set` (`:8732-8758`), before `interrupt_step_is_a_cas_on_running`'s
doc (`:8760`). The traits import (`:37-41`) gains the two refusal constants. The existing
`applied`/`stale` helpers (`:2156`…) unwrap outcomes. Fixtures: `a = json!({"tools": {"terraform":
{"kind": "path", "names": ["terraform"]}}})`, `b = json!({"nope": 1})`, `c = json!({"gpu_vendors":
[]})`, with no floats and no duplicate keys (F-14).

| Case | Asserts |
|---|---|
| `box_probe_spec_is_cas_on_updated_at` | (0) `box_probe_spec()` is `None` on the fixture (Postgres `0002` seeds only `SettingKey` rows; `MemStore` loads none). `setting(App, TokenBudget)` is read and kept as `before`. (1) `set_box_probe_spec(Some(a), None)` is `Applied(Some(row1))` with `row1.value == Some(a)`; `box_probe_spec() == Some(row1)`. (2) `set(Some(b), None)` is `Stale(Some(row1))`; the read is unchanged. (3) `set(Some(b), Some(row1.updated_at))` is `Applied(Some(row2))` with `row2.value == Some(b)` and `row2.updated_at > row1.updated_at` (F-13 message); the read is `Some(row2)`. **`b` is an object the probe refuses (`unknown key`), and the store accepts it: the store validates only shape (D2 requirement 4).** (4) `set(Some(c), Some(row1.updated_at))` is `Stale(Some(row2))`, and so is `set(None, Some(row1.updated_at))`. (5) `set(None, Some(row2.updated_at))` is `Applied(None)`; `box_probe_spec()` is `None`. (6) `set(Some(c), Some(row2.updated_at))` is `Stale(None)`, **not** `NotFound`, and so is `set(None, Some(row2.updated_at))`; the read is still `None`. (7) `set(Some(c), None)` is `Applied(Some(row3))`: the insert after a clear. (8) `setting(App, TokenBudget) == before`: no `SettingKey` row moved. |
| `set_box_probe_spec_refuses_a_non_object_and_a_clear_without_a_token` | Over no row: `set(Some(json!(42)), None)` and `set(Some(json!(["x"])), None)` are `Err(Constraint(s))` with `s == BOX_PROBE_SPEC_NOT_AN_OBJECT`; `set(None, None)` is `Err(Constraint(BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN))`; `box_probe_spec()` is `None`. Over a row (`row = applied(set(Some(a), None))`): `set(Some(json!("x")), Some(row.updated_at))` is the non-object `Constraint` (precedence over a current token) and so is `set(Some(json!(1)), Some(row.updated_at - TimeDelta::seconds(1)))` (precedence over a spent one); `set(None, None)` is the clear `Constraint` even though a row exists; `box_probe_spec() == Some(row)`, unchanged. |

Case docs cite no test outside `conformance.rs`/`mem.rs` (F-12).

### 3.6 Pins

- `crates/htui-core/tests/mem_store.rs:36-37`: `104` → `106`. The message's last clause becomes "…,
  and MOD-41 T7's three for the executor edit (plan D10), and MOD-51's two for the
  `box_probe_spec` writer (plan D7)".
- `crates/htui-store/tests/pg_conformance.rs:18-21`: the doc becomes "96 before MOD-41 and MOD-23's
  switch case; MOD-41 T1's four fence cases (plan D1) and T7's three executor edit cases (plan D10)
  make it 104, and MOD-51's two `box_probe_spec` cases (plan D7) 106." and `const EXPECTED_CASES:
  usize = 106;`. The message at `:27-28` becomes "(106 since MOD-51's two box_probe_spec cases)"
  (F-17).
- `READ_CASES` stays 14. `.sqlx` stays **291**.

### 3.7 Gate

```bash
cargo test -p htui-core --all-features -- --test-threads=1
df -h /            # before touching Postgres
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui-store --all-features --test pg_conformance -- --test-threads=1
cargo test -p htui-agent --all-features -- --test-threads=1
# the check against a migrated scratch DB (project memory: the compose `htui` DB is empty):
docker compose exec -T postgres dropdb -U postgres --if-exists htui_prepare_mod51
docker compose exec -T postgres createdb -U postgres htui_prepare_mod51
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod51 \
  sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod51 \
  cargo sqlx prepare --check -- --all-targets --all-features)
SQLX_OFFLINE=true cargo build -p htui-store --all-features
ls crates/htui-store/.sqlx | wc -l          # 291
git status --porcelain crates/htui-store/.sqlx   # empty
```

`cargo sqlx prepare` (without `--check`) is **not** run: nothing new to cache. If `--check` fails,
diff the offending literal against §3.3 (F-9).

### 3.8 Commit boundaries (T2)

1. (a) Red: the two constants and trait methods; `todo!()` bodies in `MemStore` and `PgStore` (the
   `PgStore` stub has no query, so it builds offline); the `Writer` and spy forwards (real, not
   `todo!()`); both cases, their `CASES` entries and arms; both pins. Only `mem_store_conformance`
   goes red: nothing else calls the new methods until T3.
2. (b) Green: `MemStore` (`State` twins and the two forwards).
3. (c) Green: `PgStore`; the gate above, including the Postgres suite and `--check`.

---

## 4. T3: the worker (D3 `serve` half, D4, D5)

**First failing test**: `set_probe_spec_applied_answers_boxes_with_the_overlay_in_force`. T3 starts
from T2.

**Files**: `crates/htui/src/box_settings.rs`, `crates/htui/src/store_worker.rs`,
`crates/htui/tests/box_settings.rs` (worker half).

### 4.1 `box_settings.rs`

- Imports: `use htui_core::store::{CasOutcome, Result, StoreError, StoredSetting, WriteStore};`
  (`serde_json::Value` is no longer needed by `spec_view`; keep it only if `serve` still names it).
- Module doc (`:1-11`): "Writes are `WriteStore::edit_box`, a compare-and-set on
  `box.edit_version` (D39, D41), and `WriteStore::set_box_probe_spec`, a compare-and-set on the
  overlay row's `updated_at` (MOD-51 D2), refused first by `spec::check` when the probe would
  ignore the overlay (MOD-51 D3)."
- `SpecView` (`:34-45`), last field:
  ```rust
      /// The stored `box_probe_spec` row and its token (MOD-51 D5), `None` when there is no row;
      /// the spec editor opens on it. Kept even when the probe ignores the value (R-6): the editor
      /// shows what is stored, and clearing it is always accepted.
      pub stored: Option<StoredSetting>,
  ```
  `StoredSetting` is `Eq` (`traits.rs:2306`), so `SpecView` keeps `#[derive(Debug, Clone,
  PartialEq, Eq)]`.
- `spec_view` (`:47-56`, F-21):
  ```rust
  /// The view of `spec::effective(spec::seed(), stored value)`, carrying the row. Pure.
  #[must_use]
  pub fn spec_view(stored: Option<StoredSetting>) -> SpecView {
      let value = stored.as_ref().and_then(|row| row.value.as_ref());
      let effective = spec::effective(spec::seed(), value);
      SpecView {
          overlay: value.is_some() && effective.error.is_none(),
          digest: effective.digest,
          error: effective.error,
          stored,
      }
  }
  ```
- `snapshot` (`:58-72`, D5): doc "…`writer.boxes()`, and `writer.box_probe_spec()`: the view and
  its token from one read"; body `let stored = writer.box_probe_spec().await?;` replaces
  `backend.app_settings()`; `spec: spec_view(stored)`.
- `serve` (`:74-122`): doc "Serves `Boxes`, `EditBox` and `SetProbeSpec` … `SetProbeSpec` answers
  `Boxes` on `Applied` and `BoxesStale` on `Stale` (the row gone included); an overlay
  `spec::check` refuses is `Err(Constraint(\"{SPEC_REFUSED}: {fault}\"))` before the store is
  reached". New arm after `EditBox`:
  ```rust
        StoreRequest::SetProbeSpec { overlay, expected } => {
            // D3: the probe's own merge, without its warning; nothing is written on a refusal.
            if let Some(overlay) = overlay {
                spec::check(overlay).map(drop).map_err(|fault| {
                    StoreError::Constraint(format!("{}: {fault}", spec::SPEC_REFUSED))
                })?;
            }
            let applied = match writer.set_box_probe_spec(overlay.clone(), *expected).await? {
                CasOutcome::Applied(_) => true,
                CasOutcome::Stale(_) => false,
            };
            let fresh = Box::new(snapshot(backend, &writer).await?);
            Ok(if applied {
                StoreReply::Boxes(fresh)
            } else {
                StoreReply::BoxesStale(fresh)
            })
        }
  ```
  The `other =>` comment (`:113-115`) says "exactly this module's three variants". Offline, the
  `writer()` check at the top answers `Unreachable` before `check` runs.
- `REQUEST_NAMES` (`:124-128`): doc "The three request names, in [`StoreRequest`] order."; `pub
  const REQUEST_NAMES: [&str; 3] = ["boxes", "edit_box", "set_probe_spec"];`. `READ_NAME` is
  unchanged.

### 4.2 `store_worker.rs`

- Imports: none (`DateTime`, `Utc` `:17`; `Value` `:36`).
- `StoreRequest`, after `EditBox` (`:229-236`):
  ```rust
    /// Set, replace or clear the `app_setting.box_probe_spec` overlay (MOD-51 D2–D4), a
    /// compare-and-set on the row's `updated_at`. `overlay: Some` is checked by
    /// `htui_agent::box_probe::spec::check` before anything is written, and a refusal is
    /// [`StoreReply::Failed`] carrying the probe's own fault sentence; `overlay: None` clears the
    /// row and needs `expected`. Answered with [`StoreReply::Boxes`] when it applied and
    /// [`StoreReply::BoxesStale`] when the token was spent or the row is gone. The overlay is a
    /// tool list, not a secret, so `Debug` may print it (the rule above).
    SetProbeSpec {
        /// The overlay to store, or `None` to clear the row.
        overlay: Option<Value>,
        /// The `updated_at` of the row the editor opened on (`SpecView.stored`); `None` expects
        /// no row.
        expected: Option<DateTime<Utc>>,
    },
  ```
- `name()` (`:846-849`): the comment becomes "The three of `box_settings::REQUEST_NAMES`, in that
  order (MOD-7 milestone 2, D46; MOD-51 D4).", plus `Self::SetProbeSpec { .. } =>
  "set_probe_spec",` after `EditBox`.
- `StoreReply::Boxes` doc (`:1152-1153`): "…the answer to [`StoreRequest::Boxes`] and to every box
  or probe spec write that applied (MOD-7 milestone 2, D46; MOD-51 D4)." `BoxesStale` (`:1155-1156`):
  "A box edit or a probe spec write missed its token, or its row is gone (D46, D48; MOD-51 D4): …".
- `try_serve` (`:1530-1535`):
  ```rust
        // The three box requests, or-ed for the same reason the twenty-nine above are: a guard
        // does not count towards exhaustivity in a wildcard-free `match`, so `_ if …` would be an
        // E0004 here (MOD-15 M3 plan F-12, MOD-7 milestone 2 D46, MOD-51 D4).
        StoreRequest::Boxes | StoreRequest::EditBox { .. } | StoreRequest::SetProbeSpec { .. } => {
            box_settings::serve(backend, request).await?
        }
  ```
- Test `box_requests_are_named_as_box_settings_lists_them` (`:3604-3620`): doc "The three box
  requests…"; the array gains `StoreRequest::SetProbeSpec { overlay: None, expected: None
  }.name(),`.

### 4.3 Build coupling

`boxes.rs` compiles unchanged at T3: it reads `REQUEST_NAMES[1]` and `SpecView`'s existing fields.
It does not handle `Failed { "set_probe_spec" }` until T4, and nothing sends the request before T4.

### 4.4 Tests (`crates/htui/tests/box_settings.rs`, worker half, before the `---- section (T4) ----` banner `:302`)

Imports gain `htui_core::store::traits::BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN` (or the `htui_core::store`
re-export if one exists at implementation time) and `htui_core::store::StoredSetting`. Helper:
`fn set_spec(overlay: Option<Value>, expected: Option<DateTime<Utc>>) -> StoreRequest`.

| Test | Asserts |
|---|---|
| `set_probe_spec_applied_answers_boxes_with_the_overlay_in_force` | Over `demo()`: `set_spec(Some(terraform_spec()), None)` is `Boxes(s)` with `s.spec.overlay`, `s.spec.error == None`, `s.spec.digest == spec::check(&terraform_spec()).unwrap()` and `!= spec::digest(spec::seed())`, `s.spec.stored.as_ref().map(|r| &r.value) == Some(&Some(terraform_spec()))`. A following `Boxes` read has `spec.stored == s.spec.stored` (same token). |
| `set_probe_spec_with_a_spent_token_answers_boxes_stale_and_writes_nothing` | Insert terraform (`expected: None`) → token `t1`; update to `{"tools":{"cmake":{"disabled":true}}}` under `t1` → `Boxes`; the same update again under `t1` → `BoxesStale(s)` whose `spec.stored.value` is the cmake overlay (the second write stored nothing) and whose token is the one the first update left. A second insert with `expected: None` over the row is also `BoxesStale`. |
| `a_refused_overlay_is_failed_with_the_probe_s_sentence_and_nothing_is_written` | `bad = {"tools":{"x":{"kind":"path","names":["bin/x"]}}}`; `fault = spec::check(&bad).unwrap_err()`. `serve(&backend, &set_spec(Some(bad), None))` is `Failed { request: "set_probe_spec", message }` with `message.contains(&format!("{}: {fault}", spec::SPEC_REFUSED))` (F-8). `json!(42)` → message contains `"box_probe_spec refused: the value is not a JSON object"`. A following `Boxes` has `spec.stored == None`. Directly, `box_settings::serve(&backend, &set_spec(Some(bad), None))` is `Err(StoreError::Constraint(s))` with `s.starts_with("box_probe_spec refused: ")`. |
| `clearing_under_the_token_answers_boxes_with_no_stored_overlay` | Insert, then `set_spec(None, Some(t))` → `Boxes(s)` with `s.spec.stored == None`, `!s.spec.overlay`, `s.spec.digest == spec::digest(spec::seed())`. |
| `a_clear_without_a_token_is_failed_by_the_store` | `set_spec(None, None)` → `Failed { request: "set_probe_spec", message }` containing `BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN` (the store's `Constraint`, bubbled). |
| `a_stored_overlay_the_probe_ignores_carries_its_token_and_clears` (R-6) | `MemStore::demo()` + `set_app_setting(spec::SETTING_KEY, json!(42))`: `Boxes(s)` has `s.spec.stored.value == Some(json!(42))`, `error` starting with `SPEC_IGNORED`, `!overlay`; `set_spec(None, Some(token))` → `Boxes` with `stored == None`; saving `json!(42)` back is `Failed` (refused). |
| `offline_every_box_request_is_refused_with_the_database_sentence` (renamed, F-7) | The existing body with `set_spec(Some(terraform_spec()), None)` as the third request and `assert_eq!(requests.len(), REQUEST_NAMES.len())` before the loop. |
| `the_spec_view_names_a_stored_overlay_and_an_ignored_one` (existing, `:228-257`) | Gains `accepted.stored.map(|r| r.value) == Some(Some(terraform))` and `ignored.stored.map(|r| r.value) == Some(Some(json!(42)))`. |
| `store_worker::tests::box_requests_are_named_as_box_settings_lists_them` (existing) | Three names equal `REQUEST_NAMES`. |

### 4.5 Gate

```bash
cargo test -p htui --all-features --test box_settings -- --test-threads=1
cargo test -p htui --all-features --lib store_worker -- --test-threads=1
cargo test -p htui --all-features --test box_probe_pg -- --test-threads=1   # with HTUI_TEST_DATABASE_URL; SpecView grew a field
cargo clippy -p htui --all-features --all-targets -- -D warnings
```

### 4.6 Commit boundaries (T3)

1. (a) Red: the variant, its `name()` arm, the `try_serve` arm, `REQUEST_NAMES`, `SpecView.stored`,
   `spec_view`, `snapshot` (real: the existing read tests must stay green), the `serve` arm with a
   `todo!()` body (only the new tests reach it), every test of §4.4, and the doc updates.
2. (b) Green: the `serve` arm.

---

## 5. T4: the section (D6, D8)

**First failing test**: `s_opens_the_spec_editor_over_the_pretty_printed_overlay`. T4 starts from
T3.

**Files**: `crates/htui/src/ui/tabs/settings/boxes.rs`, `crates/htui/tests/box_settings.rs`
(section half), `crates/htui/tests/snapshots/box_settings__{demo,two_boxes,spec_editor}.snap`.

### 5.1 Constants (`boxes.rs:40-97`)

```rust
/// The Browse keys (MOD-51 D6: `s` opens the probe spec editor).
const HINT_BROWSE: &str = "j/k move \u{b7} t tags \u{b7} e quirks \u{b7} w executor \u{b7} p probe \u{b7} s spec \u{b7} r reload";

/// The Browse keys over a read list with no box in it: the spec is app-wide, so `s` still works
/// (MOD-51 D6).
const HINT_NO_LIST: &str = "s spec \u{b7} r reload";

/// The Browse keys before the first read and over a refused one, where `s` does nothing (F-3,
/// needs main-thread ack).
const HINT_RELOAD: &str = "r reload";

/// The probe spec editor's keys (MOD-51 D6; OQ-16: `Enter` is a line break, so `ctrl-s` saves).
const HINT_SPEC: &str = "ctrl-s saves \u{b7} Esc cancels \u{b7} Enter breaks the line \u{b7} blank clears";

/// The dim line above the spec editor: what the text is and what blank does.
const SPEC_TITLE: &str = "stored overlay (app_setting.box_probe_spec), merged into the seed by name; blank clears it";

/// A spec save applied (MOD-51 D6, D8): nothing re-probes now; the next `p` or connect does.
const SPEC_SAVED: &str = "probe spec saved \u{2014} the next p (or the next connect) re-probes under it";

/// The opening of a parse error; `serde_json`'s own sentence follows.
const SPEC_NOT_JSON: &str = "the overlay is not JSON";

/// How many lines `PageUp`/`PageDown` move in the spec editor: its drawn height follows the
/// frame, which `on_key` cannot see.
const SPEC_PAGE: u16 = 10;

/// What a second write says while one is in flight, after the request's name (D56, F-5).
const IN_FLIGHT: &str = "in flight";

/// The name `busy` carries while a `SetProbeSpec` is in flight.
const SPEC_NAME: &str = REQUEST_NAMES[2];
```

`CHANGED_ELSEWHERE_QUIRKS` (`:83-86`) is reused verbatim for the spec editor (D6 names its
wording). Its doc becomes "for the two `ctrl-s` editors (quirks, and the probe spec, MOD-51 D6)".
`IN_FLIGHT`'s old value `"edit_box in flight"` is now produced by `format!("{EDIT_NAME}
{IN_FLIGHT}")`, byte-identical.

### 5.2 State

```rust
/// Browse, or one editor open.
enum Mode {
    …existing four…,
    /// The probe spec editor (MOD-51 D6): app-wide, over no box.
    Spec(SpecEditor),
}

/// An open probe spec editor: the token and value it opened on, and the widget.
///
/// `TextArea` redacts its `Debug`; `opened_on` is stored data (a tool list, not a secret).
#[derive(Debug)]
struct SpecEditor {
    /// `SpecView.stored`'s `updated_at` at open, `None` for no row; replaced only by a
    /// `BoxesStale` (D48), never by a plain `Boxes`.
    expected: Option<DateTime<Utc>>,
    /// `SpecView.stored`'s value at open. Never refreshed: D6's blank rule reads it.
    opened_on: Option<Value>,
    /// The overlay pretty-printed (`serde_json::to_string_pretty`), or empty; cursor at the end,
    /// as the quirks editor's.
    input: TextArea,
}
```

Imports gain `chrono::{DateTime, Utc}` and `serde_json::Value`. Module doc (`:1-17`) gains one
sentence: "`s` edits the app-wide probe spec overlay (MOD-51) in the body, box or no box." The
H-9 paragraph adds: "for a spec save the early close also shows `SPEC_SAVED`, which the real reply's
notice then replaces (MOD-51 F-11)."

### 5.3 Keys

**Browse** (`on_key`, `:562-611`), a new arm before `'p'`:
```rust
            KeyCode::Char('s') => {
                self.open_spec();
                Handled::Consumed
            }
```
`s` has no global binding (`app/update.rs:1090` is a test overlay; `keymap.rs` has none).

`open_spec`:
```rust
    /// `s` (MOD-51 D6): the probe spec editor over the stored overlay, with or without a listed
    /// box, whenever a snapshot is present and the read was not refused.
    fn open_spec(&mut self) {
        if self.blocked() {                    // refused read, or a write in flight (F-5 sentence)
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let stored = snapshot.spec.stored.as_ref();
        let opened_on = stored.and_then(|row| row.value.clone());
        let text = opened_on
            .as_ref()
            .map(|value| serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()))
            .unwrap_or_default();
        let mut input = TextArea::with_text(&text);
        input.set_cursor(usize::MAX);
        self.mode = Mode::Spec(SpecEditor {
            expected: stored.map(|row| row.updated_at),
            opened_on,
            input,
        });
        self.notice = None;
    }
```

`blocked()` (`:318-328`): `if let Some(name) = self.busy { self.notice = Some(format!("{name}
{IN_FLIGHT}")); return true; }`.

**Editor open** (`on_editor_key`, `:420-460`): a new match arm `Mode::Spec(editor) =>
editor.input.on_key(key, SPEC_PAGE),`. `Submit`/`Cancel`/`Pass` handling is shared unchanged:
`Esc` closes and leaves `busy`, and `CONTROL` chords pass, so `ctrl-c` reaches the shell.

**Paste** (`on_paste`, `:551-560`): `Mode::Spec(editor) => editor.input.on_paste(text),`.

**`submit`** (`:467-525`): the in-flight guard becomes `if let Some(name) = self.busy { self.notice
= Some(format!("{name} {IN_FLIGHT}")); return; }`. New arm:
```rust
            Mode::Spec(editor) => {
                let text = editor.input.text();
                if text.trim().is_empty() {
                    // D6 + F-4: nothing to clear closes; a row under a token is cleared.
                    match (&editor.opened_on, editor.expected) {
                        (None, _) | (_, None) => None,
                        (Some(_), Some(token)) => Some(StoreRequest::SetProbeSpec {
                            overlay: None,
                            expected: Some(token),
                        }),
                    }
                } else {
                    match serde_json::from_str::<Value>(text) {
                        Err(error) => {
                            self.notice = Some(format!("{SPEC_NOT_JSON}: {error}"));
                            return;
                        }
                        // No unchanged short-circuit (F-15): R-6 needs an unchanged save sent.
                        Ok(overlay) => Some(StoreRequest::SetProbeSpec {
                            overlay: Some(overlay),
                            expected: editor.expected,
                        }),
                    }
                }
            }
```
The tail: `Some(request) => { self.busy = Some(request.name()); ctx.request(request); }`
(`StoreRequest::name` returns `&'static str`, so one line serves `edit_box` and `set_probe_spec`;
`EDIT_NAME` stays for the `Failed` match).

### 5.4 Replies (`on_reply`, `:613-640`; `on_boxes` `:269-276`; `on_stale` `:279-312`)

| Reply | Mode `Spec` open | Mode `Browse` (spec editor closed) | Notes |
|---|---|---|---|
| `Boxes(s)`, `busy == Some("set_probe_spec")` | Browse; `notice = SPEC_SAVED`; `busy = None` | `notice = SPEC_SAVED`; `busy = None` | The applied save (D6, D8). F-11 residue: a stray read here is taken for it. |
| `Boxes(s)`, `busy == Some("edit_box")` | unreachable (`s` is blocked while busy) | existing: Browse, notice cleared | Unchanged. |
| `Boxes(s)`, `busy == None` | editor, text, `expected`, `opened_on` untouched; snapshot replaced | snapshot replaced | D48: a plain read never moves the token. |
| `BoxesStale(s)` | `busy = None`; snapshot replaced; `editor.expected = s.spec.stored.map(|r| r.updated_at)` (`None` when the row is gone); text and `opened_on` kept; `notice = CHANGED_ELSEWHERE_QUIRKS` | `busy = None`; `notice = CHANGED_ELSEWHERE_CLOSED` (existing path through `editor_box() == None`) | F-6(a): the `Spec` branch runs **before** `editor_box()`. |
| `Failed { request: "set_probe_spec", message }` | `busy = None`; editor stays over its text; `notice = message` | `busy = None`; `notice = message` | New arm, after the `EDIT_NAME` arm. |
| `Failed { request: "boxes", .. }` | `unavailable = Some(..)`; editor stays and stays **visible** (§5.5) | existing | `busy` untouched. |
| `BoxProbed`, `Failed { "probe_box" }` | unchanged (`p` is not reachable while the editor has the keys) | unchanged | — |

`on_boxes`:
```rust
    fn on_boxes(&mut self, snapshot: &BoxesSnapshot) {
        self.replace(snapshot);
        match self.busy.take() {
            None => {}
            Some(name) => {
                self.mode = Mode::Browse;
                self.notice = (name == SPEC_NAME).then(|| SPEC_SAVED.to_owned());
            }
        }
    }
```
`on_stale`: after `self.busy = None; self.replace(snapshot);`, insert
```rust
        if let Mode::Spec(editor) = &mut self.mode {
            editor.expected = snapshot.spec.stored.as_ref().map(|row| row.updated_at);
            self.notice = Some(CHANGED_ELSEWHERE_QUIRKS.to_owned());
            return;
        }
```
The tuple `match` (`:292-310`) then needs `Mode::Spec(_)` only for exhaustiveness: `(Some(_),
Mode::Browse | Mode::Spec(_)) => CHANGED_ELSEWHERE`.

`editor_box()` (`:222-229`): `Mode::Browse | Mode::Spec(_) => None`.

### 5.5 Render (`render`, `:641-700`; `hint_text`, `:901-918`)

**Where**: the body area (list **and** detail), D6. Before the existing `match (&self.unavailable,
&self.snapshot)`:
```rust
        if let (Mode::Spec(editor), Some(snapshot)) = (&self.mode, &self.snapshot) {
            // An editor that takes keys stays on screen (the rule of `:654-656`), box or no box:
            // a refused read takes the first line and the editor keeps the rest.
            let area = match &self.unavailable {
                Some(why) => {
                    let [refusal, rest] =
                        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
                    frame.render_widget(
                        Paragraph::new(Line::styled(format!("{UNAVAILABLE}: {why}"), theme.error)),
                        refusal,
                    );
                    rest
                }
                None => body,
            };
            self.render_spec(frame, area, snapshot, editor, theme);
        } else {
            match … { /* existing five arms, unchanged */ }
        }
```
The first existing arm's `!matches!(self.mode, Mode::Browse)` now only ever sees a box editor.

`render_spec` (a new `BoxesSection` method beside `render_body`, `:705`):
```rust
    /// The probe spec editor in the body (MOD-51 D6): the effective spec as it stands (D51's
    /// lines, the ignored-overlay sentence included), the dim title, then the editor over every
    /// line left.
    fn render_spec(&self, frame: &mut Frame<'_>, area: Rect, snapshot: &BoxesSnapshot, editor: &SpecEditor, theme: &Theme) {
        let mut lines = spec_lines(&snapshot.spec, usize::from(area.width).max(1), theme);
        lines.push(Line::styled(SPEC_TITLE, theme.dim));
        let used = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        lines.extend(editor.input.lines(area.width, area.height.saturating_sub(used), true, theme));
        frame.render_widget(Paragraph::new(lines), area);
    }
```
The spec line does not recompute while typing. It shows what is in force, and after an applied save
the next `Boxes` carries the new digest (acceptance line 1).

`hint_text`:
```rust
        let readable = self.unavailable.is_none() && self.snapshot.is_some();
        let listed = readable && !self.boxes().is_empty();
        let mut hint = match self.mode {
            Mode::Browse if listed => HINT_BROWSE.to_owned(),
            Mode::Browse if readable => HINT_NO_LIST.to_owned(),
            Mode::Browse => HINT_RELOAD.to_owned(),
            Mode::Tags(_) => HINT_TAGS.to_owned(),
            Mode::Quirks(_) => HINT_QUIRKS.to_owned(),
            Mode::Executor(_) => HINT_EXECUTOR.to_owned(),
            Mode::Spec(_) => HINT_SPEC.to_owned(),
        };
```
(`probing…`/`saving…` suffixes unchanged.) If the main thread rejects F-3, `readable` collapses into
`listed` and `HINT_RELOAD` is deleted.

`captures_input` (`:546-548`) is unchanged: `!matches!(self.mode, Mode::Browse)` covers `Spec`.

### 5.6 Tests (`crates/htui/tests/box_settings.rs`, section half)

Helpers: `terraform_store() -> MemStore` (`MemStore::demo()` + `set_app_setting(spec::SETTING_KEY,
terraform_spec())`); `only_spec(requests) -> (Option<Value>, Option<DateTime<Utc>>)` (panics unless
exactly one `SetProbeSpec`); `no_boxes() -> BoxesSnapshot` (`snap_of(MemStore::demo())` with
`boxes.clear()`); `clear_editor(bench, section, n)` presses `backspace` `n` times.

| Test | Asserts |
|---|---|
| `s_opens_the_spec_editor_over_the_pretty_printed_overlay` | Over `snap_of(terraform_store())`: `s` → `Consumed`, `captures_input()`; the frame contains `"terraform": {` and `"kind": "path"` (pretty-printed), `SPEC_TITLE`, `probe spec: seed + stored overlay`; the hint is `ctrl-s saves · Esc cancels · Enter breaks the line · blank clears`; no request. |
| `s_with_no_box_listed_still_opens_the_spec_editor` | Over `no_boxes()`: the frame says `no box is registered for this user yet` and the hint is `s spec · r reload`; `s` → `captures_input()`; paste `{"tools": {}}`, `ctrl-s` → `only_spec == (Some(json!({"tools": {}})), None)`. |
| `s_does_nothing_before_the_first_read_or_over_a_refused_read` | A fresh section: `s` → `Consumed`, `!captures_input()`, hint `r reload`. After `Failed { "boxes" }`: the same, and no request. (F-3) |
| `ctrl_s_sends_set_probe_spec_with_the_parsed_overlay_and_the_token` | Over `terraform_store()`, `token = snapshot.spec.stored.unwrap().updated_at`; `s`, `ctrl-s` unchanged → `(Some(terraform_spec()), Some(token))` (F-15); the hint shows `saving…`. |
| `a_parse_error_keeps_the_spec_editor_open_and_sends_nothing` | Over demo: `s`, paste `{"tools": `, `ctrl-s` → no request, `captures_input()`, the frame contains `the overlay is not JSON: `; the hint has no `saving…`. |
| `blank_over_no_row_closes_the_spec_editor_without_a_request` | Over demo (no row): `s`, `ctrl-s` → no request, `!captures_input()`. Also `s`, paste `   \n  `, `ctrl-s` → no request (whitespace is blank). |
| `blank_over_a_row_sends_a_clear_under_the_token` | Over `terraform_store()`: `s`, `clear_editor` with `to_string_pretty(&terraform_spec()).chars().count()` presses, `ctrl-s` → `(None, Some(token))`. |
| `a_plain_read_leaves_the_spec_editor_and_its_token_alone` | Open over `t1`, paste a change, feed a `Boxes` whose `spec.stored` has a later `updated_at`: still open with the text; `ctrl-s` → `expected == Some(t1)`. |
| `boxes_stale_over_the_spec_editor_keeps_the_text_and_takes_the_new_token` | Open over `t1`, paste, `ctrl-s`; feed `BoxesStale` with `stored.updated_at = t2`: `captures_input()`, the text is kept, the frame contains `changed elsewhere since you opened it` and `ctrl-s retries`; `ctrl-s` → `expected == Some(t2)` with the same overlay. Then feed `BoxesStale` with `stored: None` and `ctrl-s` again → `expected == None`. |
| `blank_after_the_row_vanished_closes_without_a_request` (F-4) | Open over a row, clear the text, `ctrl-s` → `(None, Some(t))`; feed `BoxesStale` with `stored: None`; `ctrl-s` → no request, `!captures_input()`. |
| `a_refused_spec_save_keeps_the_editor_and_frees_the_next_save` | `ctrl-s`; feed `Failed { request: "set_probe_spec", message: "constraint violated: box_probe_spec refused: tools: `bin/x` is not a bare tool name" }` → open, the frame shows the sentence, no `saving…`; `ctrl-s` again → one more `SetProbeSpec`. |
| `the_reply_to_a_spec_save_closes_the_editor_with_the_reprobe_notice` | `ctrl-s`; feed `Boxes` → `!captures_input()`; `last_line(frame)` contains `probe spec saved` and `re-probes under it`; no `ProbeBox` was sent (D8). |
| `a_second_spec_save_while_saving_sends_nothing` | `ctrl-s`, `ctrl-s` → exactly one request; the frame contains `set_probe_spec in flight`. |
| `no_editor_opens_across_the_two_writes` | Spec `ctrl-s`, `esc`, then `t`, `e`, `w` → nothing opens, notice `set_probe_spec in flight`. A fresh section: `t` save, `esc`, `s` → nothing opens, notice `edit_box in flight`. |
| `a_refused_read_keeps_the_spec_editor_visible` | Over `no_boxes()`: `s`, paste `{}`, feed `Failed { "boxes" }` → `captures_input()`, the frame shows `boxes unavailable: …` **and** `{}` (F-6(b)). |
| `ctrl_c_passes_through_an_open_spec_editor` | `s`, `ctrl-c` → `Handled::Pass`; still open. |
| `a_refused_read_keeps_an_open_editor_visible_and_blocks_new_ones` (existing, `:829-866`) | The key loop becomes `["t", "e", "p", "w", "s"]`. |
| snapshot `box_settings__spec_editor` (new) | `bench_with(&snap_of(terraform_store()).await)`, `s`, `render_section(&section, 100)` under `DIGEST_FILTER`: the spec line, the title, the pretty overlay, `HINT_SPEC`. |

Existing tests that keep passing unchanged (text asserts `edit_box in flight`, `changed elsewhere…`,
`ctrl-s retries`): `:664-676`, `:706-718`, `:722-757`, `:1094-1107`.

### 5.7 Snapshots

| Frame | Change |
|---|---|
| `box_settings__demo.snap` | **moves**: the hint line gains ` · s spec` before ` · r reload` (only that line). |
| `box_settings__two_boxes.snap` | **moves**: same hint line only. |
| `box_settings__offline.snap` | **unchanged** under F-3's `HINT_RELOAD`; under D6-literal it moves to `s spec · r reload`. |
| `tag_editor`, `quirks_editor`, `stale`, `executor_confirm` | unchanged (editor hints). |
| `box_settings__spec_editor.snap` | **new**. |

Count: 121 → **122**. No other test file's frame draws the boxes hint (checked:
`hierarchy__*`, `kinds__*`, `prompt_settings__*` carry their own sections' `r reload`).

### 5.8 Gate and commits

```bash
cargo test -p htui --all-features -- --test-threads=1
cargo insta test -p htui --all-features --review   # accept demo, two_boxes (hint line only) and spec_editor
git status --porcelain crates/htui/tests/snapshots   # exactly those three
cargo clippy --workspace --all-features --all-targets -- -D warnings
```

1. (a) Red: `SpecEditor`, `Mode::Spec` with every exhaustive arm (`editor_box`, `on_editor_key`,
   `on_paste`, `on_stale`'s tuple, `submit` returning `None` for `Spec`, `hint_text`), the constants,
   the `IN_FLIGHT` change (F-5, byte-identical for `edit_box`), every test of §5.6. **No `todo!()`**
   in any method the shell calls (MOD-7 F-F: `SettingsTab::on_reply` hands every reply to every
   section, and `register_all` already registers this one).
2. (b) Green: `s`/`open_spec`, `submit`'s spec arm, `on_boxes`, `on_stale`, the `Failed` arm.
3. (c) Green: `render_spec`, the render branch, the hints (F-3), the three snapshots.

---

## 6. Cross-task contracts

| Defined in | Item (exact) | Consumed by |
|---|---|---|
| T1 `htui_core::model` | `BOX_PROBE_SPEC_KEY: &str = "box_probe_spec"` | T2 (Mem, Pg), T1 alias |
| T1 `htui_agent::box_probe::spec` | `SPEC_REFUSED: &str = "box_probe_spec refused"`; `check(&Value) -> Result<String, String>` (digest / unprefixed fault); `SETTING_KEY` = alias | T3 `serve` |
| T2 `htui_core::store::traits` | `BOX_PROBE_SPEC_NOT_AN_OBJECT`, `BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN`; `WriteStore::box_probe_spec(&self) -> Result<Option<StoredSetting>>`; `WriteStore::set_box_probe_spec(&self, Option<Value>, Option<DateTime<Utc>>) -> Result<CasOutcome<Option<StoredSetting>>>` (`Stale(None)` for a gone row; both `Constraint`s before any read) | T3 |
| T3 `htui::box_settings` | `SpecView.stored: Option<StoredSetting>`; `spec_view(Option<StoredSetting>)`; `REQUEST_NAMES: [&str; 3]` | T4 |
| T3 `htui::store_worker` | `StoreRequest::SetProbeSpec { overlay: Option<Value>, expected: Option<DateTime<Utc>> }`, name `"set_probe_spec"`; answers `Boxes` / `BoxesStale` / `Failed { "set_probe_spec" }` | T4 |

---

## 7. Workspace gate (after T4)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
df -h /
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
# sqlx check exactly as §3.7 (scratch DB migrated first)
ls crates/htui-store/.sqlx | wc -l                  # 291
ls crates/htui/tests/snapshots/*.snap | wc -l       # 122
cargo doc --workspace --no-deps --keep-going        # no new errors (F-2)
git diff --stat 626d5bf -- crates/htui-store/migrations crates/htui-store/.sqlx   # empty
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

Before believing a Postgres failure: `df -h /`, then re-run the case alone. Check for orphaned test
processes and home-dir debris after the run (project memory).

---

## 8. Count pins

| Pin | Now | After | Task |
|---|---|---|---|
| Store `CASES` | 104 | 106 | T2 (`conformance.rs:44-149`, `mem_store.rs:37`, `pg_conformance.rs:21`) |
| `READ_CASES` | 14 | 14 | — |
| `box_settings::REQUEST_NAMES` | 2 | 3 | T3 |
| `StoreRequest` variants | n | n + 1 | T3 |
| `StoreReply` variants | m | m | — |
| `WriteStore` methods | k | k + 2 (on five impls) | T2 |
| `.sqlx` files | 291 | 291 (four statements reused, F-9) | — |
| Migrations | unchanged | unchanged | — |
| `crates/htui/tests/snapshots/*.snap` | 121 | 122 (`box_settings__spec_editor`); `demo`, `two_boxes` move | T4 |

---

## 9. Risks (R-1 onward, this blueprint's)

| # | Risk | Likelihood | Mitigation |
|---|---|---|---|
| R-1 | H-9 residue for the spec save (F-11): a stray read closes the editor and says "saved" before the real reply. | Low | The real reply's notice replaces it (`CHANGED_ELSEWHERE_CLOSED` or the refusal); nothing is written wrongly. Documented in the module doc. |
| R-2 | A literal re-indented by an editor adds `.sqlx` entries or fails `--check` (F-9). | Medium | The literals are shown whitespace-visible in §3.3; `--check` and the file count 291 are gates; the hashes are listed. |
| R-3 | The Browse hint overflows an 80-column terminal (F-10). | Certain at 80 cols | Main thread's call; no snapshot is affected. |
| R-4 | F-3/F-4 refine D6 and are not accepted. | Low | Each has a stated fallback (D6-literal hint; `opened_on`-only blank rule), and the tests named for them change accordingly. |
| R-5 | `MemStore` token collision on a coarse clock (F-13). | Low | The case asserts `row2.updated_at > row1.updated_at` with a message. |
