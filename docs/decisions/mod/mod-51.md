# MOD-51 - `box_probe_spec` editor in the Settings box section (done, 2026-10-01)

**Requirements:** `R-BOX-2`, `R-TUI-8`, `R-NF-3`; ANA-16 C7 (every human writer is a CAS).
**Origin:** MOD-7 milestone 2, OQ-18 (`docs/decisions/mod/mod-7.md`): milestone 1's plan promised a
validated writer of `app_setting.box_probe_spec` (D15), and the maintainer deferred it here at
milestone 2's CONFIRM gate on 2026-09-26. Its recorded shape was MOD-7 D52 / milestone 2 T6.
**Artifacts:** plan with its verified-claims table:
[`.claude/plans/mod-51-box-probe-spec-editor.plan.md`](../../../.claude/plans/mod-51-box-probe-spec-editor.plan.md);
blueprint:
[`.claude/plans/mod-51-box-probe-spec-editor.blueprint.md`](../../../.claude/plans/mod-51-box-probe-spec-editor.blueprint.md).
There is no PRD. The item was routed as a plan on 2026-10-01 with 1 of C1-C4 fired (C4, about 18
files), no ultracode. It ran in a TOOL-7 sandbox (`hr/MOD-51`).
**Commits:**
- Plan and blueprint: `626d5bf` plan (fact-checked, confirmed), `36fcae1` blueprint with the
  maintainer's F-3/F-4/F-10 decisions.
- T1, key and checker: `e1eb188`, `842bd82`.
- T2, store seam: `8bca020`, `1e2297f`.
- T3, worker: `7207ae9`/`f71fb61` (red, then green).
- T4, section: `d1c8a94`/`a8f1270` (red, then green).
- Review fixes: `c7ecb69`, `5e1119b`; re-review nit `78da8d8`.
- Outside the item: `37fb4da`, a gate fix (see "Also in this branch").
- Then this write-up.

## What shipped

Before this item, the only way to change the box probe spec overlay was the SQL in
`crates/htui-agent/src/box_probe/spec.rs`'s module doc. Now `Settings > Boxes` edits it:

- **`s`** opens a multi-line JSON editor over the stored overlay, pretty-printed, or empty when no
  row exists. It works with or without a listed box, because the spec is app-wide. Before the
  first read and over a refused read, `s` does nothing and the hint is `r reload` (F-3).
- **`ctrl-s`** saves as a compare-and-set on `app_setting.updated_at`. `Esc` cancels, `Enter`
  breaks the line (the quirks editor's keys, MOD-7 OQ-16).
  - A JSON parse error stays in the editor as the notice, and nothing is sent.
  - A blank save clears the row under the live token. With no token (no row, or the row vanished
    under the editor), a blank save closes with nothing sent (D6, F-4, amended at review LOW-1).
  - A spent token answers `BoxesStale`: the editor keeps its text, takes the new token, and the next
    `ctrl-s` retries. A refusal keeps the editor open with the sentence.
  - An applied save closes the editor with `probe spec saved — the next p (or the next connect)
    re-probes under it`. Saving does not probe (D8): a new digest re-probes at the next `p` or
    connect (MOD-7 D18).
- The spec line (`seed` or `seed + stored overlay`, digest, why an overlay was ignored) is now also
  drawn under "no box is registered" (review LOW-6), so a save over an empty list is visible.
- The Browse hint is `j/k move · t tags · e quirks · w executor · p probe · s spec · r reload`;
  `p probe this box` was shortened to fit an 80-column terminal (F-10).

## Decisions worth keeping

- **D1, one key.** `htui_core::model::BOX_PROBE_SPEC_KEY`; `spec::SETTING_KEY` is an alias of it, so
  the store (which must not depend on `htui-agent`) and the probe cannot disagree on the key.
- **D2, a narrow typed writer.** `WriteStore::box_probe_spec()` and
  `set_box_probe_spec(overlay: Option<Value>, expected: Option<DateTime<Utc>>)
  -> CasOutcome<Option<StoredSetting>>`. The two refusals come first and need no read: a value that
  is not a JSON object (`BOX_PROBE_SPEC_NOT_AN_OBJECT`) and a clear without a token
  (`BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN`). Then comes the CAS. **A row gone under a token is
  `Stale(None)`, not `NotFound`**, which deliberately differs from `set_setting`'s `App` arm: for
  the editor a deleted row is a miss like a spent token (MOD-7 D48). The generic `set_setting` is
  not used, because its keys are the closed `SettingKey` enum.
- **D3, the probe's validation without the probe's warning** (amends MOD-7 D52). `spec::check(&Value)
  -> Result<digest, fault>` runs the same private `merge` that `effective` runs, with no `warn!`.
  The worker refuses with `box_probe_spec refused: <fault>` (`SPEC_REFUSED`) before any write. D52
  had reused `effective`, which logs "the box probe runs under its seed spec" on every refusal and
  prefixes the sentence with `SPEC_IGNORED`. Because the checker and the probe share `merge`, the
  editor cannot store an overlay the probe would ignore.
- **D4/D5.** `StoreRequest::SetProbeSpec { overlay, expected }` (`"set_probe_spec"`, the third of
  `box_settings::REQUEST_NAMES`) answers `Boxes` / `BoxesStale` / `Failed`. `SpecView.stored`
  carries the row and its token from the same read the view is computed from
  (`writer.box_probe_spec()`).
- **No `.sqlx` change (F-9).** The `PgStore` methods reuse the four existing `app_setting`
  statements of `set_setting` / `clear_setting` byte for byte, indentation included. The offline
  entry's name is the SHA-256 of the literal query string, so all four hashes already existed
  (291 files before and after; `cargo sqlx prepare --check` green against a migrated scratch DB).

## Tests

- Conformance: `box_probe_spec_is_cas_on_updated_at` and
  `set_box_probe_spec_refuses_a_non_object_and_a_clear_without_a_token`, run on `MemStore` and
  `PgStore`; `CASES` 104 → 106 (both pins moved).
- `crates/htui-agent/tests/box_probe.rs`: `check` accepts what `effective` merges, with the same
  digest, and refuses what it ignores, with the same fault text (four tests; the module keeps no
  `#[cfg(test)]` block, MOD-7 D29).
- `crates/htui/tests/box_settings.rs` (73 tests in the file): the worker half (applied, spent
  token, vanished row, refused overlay at both layers, clear, clear without a token, offline before
  the checker) and the section half (every D6 clause, F-3, F-4 and both review behaviour fixes).
  Frames: `spec_editor` and `no_boxes` are new; `demo` and `two_boxes` moved by the hint line only;
  `offline` is unchanged.
- Workspace gate (`cargo test --workspace --all-features -- --test-threads=1`, sandbox Postgres):
  3077 passed at the base and 3108 after the review fixes, with no failures; fmt and clippy clean.

## Review

`rust-reviewer` approved with fixes and raised no CRITICAL, HIGH or MEDIUM finding. It confirmed the
CAS precedence on both stores, the microsecond token round-trip, `.sqlx` byte-identity, the
validate-before-write order, and that the "next `p` re-probes" notice is true. The maintainer
decided the two behaviour findings:
- **LOW-1:** a blank save keys on the live token, not on what the editor opened over.
- **LOW-6:** the empty list draws the spec line.

The rest were applied: a worker test for `Stale(None)`, a refused overlay in the offline test, `s` in
the in-flight test, the H-9 race sentence, and four nits. A re-review approved the fix commits with no finding above NIT; one NIT (a stronger assertion in
the no-box editor test) was applied, the rest accepted as residue.

**Known residue** (documented in `boxes.rs`'s module doc):
- **F-11:** a plain read served ahead of a save's reply is taken for the reply, so "probe spec
  saved" can show early. The real reply's notice then replaces it.
- **H-9:** a stray read can land a tags or quirks save's `BoxesStale` in the spec editor's branch.
  It refreshes the spec token, and nothing is written wrongly.

## Also in this branch

`37fb4da` is `cargo fmt` over `crates/htui/src/run_worker.rs` and `crates/htui-worker/src/lib.rs`,
both left unformatted by the MOD-41 merge. The workspace `cargo fmt --check` gate failed on them.
Separately, `RUSTDOCFLAGS=-D warnings cargo doc -p htui-core` fails on a broken intra-doc link to
`crate::store::MIRRORED_TABLES`, a constant that lives in `htui-store`. That predates this item and
is not fixed here.
