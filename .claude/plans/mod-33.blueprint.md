# Blueprint: MOD-33 — the box hostname leaves the digest and gains a settings switch

**Architect: code-architect. Read-only except this file. No cargo command was run; every compile
fact below is read off the tree (Gortex, primary checkout at `12d2c01` = `f5de3d4` + plan
commits), not observed from a build.** Spec: `.claude/plans/mod-33.plan.md` (CONFIRMED
2026-09-30; D263–D277 are settled and not reopened here). Line numbers are at `12d2c01`; the
source under `crates/` is byte-identical to `f5de3d4`.

rustfmt is `max_width = 100`; clippy is `all = warn` with pedantic **off** (`Cargo.toml`
`[workspace.lints.clippy]`), so `-D warnings` enforces the `complexity` group — including
`too_many_arguments` (> 7 parameters). The house override is
`#[expect(clippy::too_many_arguments, reason = "…")]` (precedent `engine.rs:4623`).

---

## Execution order

```
T0 (recorder) ─┐
T1 (registry) ─┼─► T2 (assembler + callers)        T3 (migration 0010) — any time, droppable
               ┘
```

T0, T1, T3 are pairwise file-disjoint and each leaves the workspace green. T2 needs T0
(`record_prompt_digesting`) and T1 (`settings::resolve_box_hostname`). Serial order when disk is
short (`df -h /` first; a worktree `target/` is ~10 GB): **T0, T1, T3, T2**.

Binding on every implementer (plan "Conditions", restated): no store / trait / `.sqlx` /
`BoxProfile` / `ProjectSettings` change; no `Deserialize` on `TrimRecord`, no `#[serde(default)]`;
never find the hostname by searching text; no edit to `docs/ANA-*.md`, `HANDOFF.md`,
`DECISIONS.md`; test first; commit incrementally, staging only your own paths — no `git stash`,
no `git add -A`, no `--amend`, no push. Every commit message ends with
`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

---

## Plan deltas (read before starting)

No file outside the plan's Files table has to change. Four points where the plan text is
internally inconsistent or under-specified, with the resolution this blueprint takes:

1. **T1's snapshot-review line contradicts D276.** Plan T1 says the new row reads
   `box_hostname  unset | on (app_setting_default)  on/off`; D276 says the source label for an
   unset project-only key reads `(default)`, "no `app_setting` row can exist for it". D276 is the
   decision; the review line is a slip. **Row text is
   `  box_hostname                  unset | on (default)   on/off`.** Consequence inside
   `ui/tabs/settings/prompt.rs` (in the Files table): `Effective.source` changes type from
   `BudgetSource` to `&'static str`, because `BudgetSource` is also `trim_record.budget_source`'s
   closed vocabulary and must not gain a `Default`-label variant.
2. **`trim::record()` reaches eight parameters** once it takes `undigested` (plan T2 step 2).
   That trips `clippy::too_many_arguments` under `-D warnings`. Resolution: an `#[expect(…)]`
   with a reason, as `engine.rs:4623` does. No signature bundling (it would touch nothing else, but
   the plan says "`record()` takes the list").
3. **`app_keys().len()`** (plan T1 test list) — `app_keys()` is specified here as an iterator
   (the shape of the existing `prompt_settings::project_keys()`), so the tests write
   `SettingKey::app_keys().count()`. Same meaning.
4. **T2's engine test needs a box with another hostname** and the fake has no constructor for one.
   `FakeOrchestrator.store` and `.clock` are `pub` (`fake.rs:1270`, `:1276`), so the test swaps the
   store for `MemStore::from_demo(edited demo_data()).with_clock(Arc::new(orch.clock.clone()))`
   — the exact expression `FakeOrchestrator::demo()` uses (`fake.rs:1311`). **No `fake.rs` edit.**

Also confirmed free: `0010` is still the next migration number (no `0010_*` on any local or
remote branch; `migrations/` ends at `0009_agent_box_user_off.sql`).

---

## T0 — the recorder hashes the text ANA-5 supplies for digesting (D271)

**Files (complete):** `crates/htui-agent/src/record.rs`, `crates/htui-agent/tests/recorder.rs`.

### Signature

`record.rs`, inserted directly **above** `record_prompt` (doc `:630-645`, fn `:646-680`):

```rust
pub async fn record_prompt_digesting(
    &mut self,
    text: &str,
    digest_text: &str,
    sections: Value,
    at: DateTime<Utc>,
) -> Result<(), RecordError>
```

`record_prompt` keeps its signature and becomes a one-line delegate:
`self.record_prompt_digesting(text, text, sections, at).await`.

### Body (order matters)

1. `stamp(at)`, `turn = 0`, `turns = 1` — unchanged.
2. Scrub `json!({ "text": text, "sections": sections })` exactly as today; `Err` →
   `return self.refuse(unmasked, at).await` (`:1547`).
3. The digested string:
   - **`digest_text == text` (string equality of the two arguments):** hash the scrubbed
     payload `text`, i.e. today's `:660-664` verbatim. This is what keeps the chat path
     (`agent_worker.rs` `run_chat`, untouched) byte-identical.
   - **otherwise:** `let mut value = Value::String(digest_text.to_owned());`
     `self.scrubber.scrub(&mut value)`; on `Err(mut unmasked)` set
     `unmasked.path = format!("/digest_text{}", unmasked.path)` (a bare string reports `""`, so
     the row reads `<rule> at /digest_text`) and `return self.refuse(unmasked, at).await`. On `Ok`
     hash `value.as_str()` with the same `format!("{:x}", Sha256::digest(…))`.
4. `payload["digest"]`, `self.prompt_digest`, `self.digest_pending` from that hash; flush, push
   the `Prompt` row, flush, `sync_step()` — unchanged.

The payload schema is unchanged: `digest_text` is **never** a payload key and is never persisted.

### Docs

- New method doc, key sentences: "Records the assembled initial prompt with the text the digest
  is over (MOD-33 D271). `text` is what was sent and is what the payload stores; `digest_text` is
  what ANA-5 supplies for digesting — the same canonical text with each undigested span's value
  replaced by its stand-in. Both are scrubbed with the one scrubber; a residue in either refuses
  the prompt through the same `scrub_residue` row. The recorder still computes the digest (ANA-4
  §4 'the driver computes the digest; ANA-5 owns what is digested'), so `run_step.prompt_digest`
  equals the assembler's pre-flight digest (ANA-5 §12 criterion 11)."
- `record_prompt` doc (`:630`): "…the digest is `sha256` over the scrubbed text **when the
  text sent is also the text digested** — a chat's user text, a judge's or a handoff's prompt.
  An assembled phase prompt goes through [`Recorder::record_prompt_digesting`]."
- Module doc item 3 (`:22-27`): "`sha256` over the text the assembler supplies for digesting
  (the sent text itself unless a span is undigested, MOD-33), computed once…".

### Tests first (`crates/htui-agent/tests/recorder.rs`, after `seq_is_gapless_turns_count_and_digest_reaches_the_step`, `:1181-1284`)

Fixtures to reuse: `chat_spec()` `:126`, `open_chat()` `:137`, `rows()` `:147`, `at()` `:67`,
`scrubber()` (masks `SECRET = "fake-secret-9f8e7d"`) `:72`, `step_digest()` `:3915`,
`store.usage_calls()`.

| Test | Asserts |
|---|---|
| `a_split_prompt_stores_the_sent_text_and_digests_the_digest_text` | `record_prompt_digesting("A x Z", "A [s] Z", json!([]), at())`: seq-0 row is `Prompt`, payload `text == "A x Z"`, payload `digest == sha256("A [s] Z")` and `!= sha256("A x Z")`; `finish()`'s `RecorderSummary.prompt_digest` equals it; the one `Some` digest in `usage_calls()` equals it. |
| `record_prompt_is_the_split_with_one_string` | Two fresh recorders over two `open_chat` stores: `record_prompt(T, S, at())` vs `record_prompt_digesting(T, T, S, at())` with `T` containing `SECRET`; the seq-0 rows agree on `seq`, `turn`, `kind`, `role`, `payload` (incl. masked text and `digest`), `at`; summaries' `prompt_digest` equal. (Step ids differ by construction — compare fields, not whole rows.) |
| `a_secret_in_the_digest_text_is_masked_before_the_hash` | `record_prompt_digesting("A x Z", &format!("A {SECRET} Z"), …)` → digest `== sha256("A [REDACTED] Z")`; payload text `"A x Z"`. |
| `residue_in_the_digest_text_refuses_the_prompt` | `digest_text = "A sk-ant-api03-abcdefghijklmnopqrstuvwx Z"`, `text = "A x Z"`: `record_prompt_digesting` returns `Ok(())`; the log has **no** `Prompt` row and exactly one `Error` row at seq 0 with `code == "scrub_residue"` and `message == "anthropic_api_key at /digest_text"`; `finish()` is `Err(RecordError::Unmasked(_))`; `step_digest()` is `None`; no `usage_calls()` entry carries `Some` digest; the serialised log contains no `sk-ant-`. |

Existing expectations that must pass **unchanged**: `seq_is_gapless_turns_count_and_digest_reaches_the_step`,
`a_handoff_is_a_follow_up_at_the_next_turn_not_a_second_prompt`, and the conformance cases
`seq_gapless_and_turns` / `usage_deltas_sum_to_step_usage` (`htui-agent/src/conformance.rs`
`:1421`, `:2114`, `sha256(PROMPT)`).

### Commit

One commit: `feat(mod-33): the recorder digests the text ANA-5 supplies for digesting` (tests +
method + docs). Stage exactly the two files.

### Verify

```bash
cargo test -p htui-agent --all-features --test recorder -- --test-threads=1
cargo test -p htui-agent --all-features -- --test-threads=1
cargo clippy -p htui-agent --all-features --all-targets -- -D warnings
cargo fmt --all -- --check
```

### Hazards

- The equal-string shortcut compares the **arguments**, before scrubbing. Do not compare the
  scrubbed payload text with the scrubbed digest text — that would take the split path for a chat
  whose text the scrubber masked, and would still be equal, but it is a second scrub for nothing.
- Scrub order: payload first, digest text second. A payload residue refuses exactly as today and
  the digest text is never examined.
- `Unmasked.path` is a JSON pointer; prefix, don't replace, so a future scrubber that reports a
  sub-path keeps it.

---

## T1 — the `box_hostname` key and its Settings row (D267, D276)

**Files (complete):** `crates/htui-core/src/prompt/settings.rs`,
`crates/htui/src/prompt_settings.rs`, `crates/htui/src/ui/tabs/settings/prompt.rs`,
`crates/htui/tests/prompt_settings.rs`,
`crates/htui/tests/snapshots/prompt_settings__{demo,clamped,editor_fraction,stale}.snap`.

### `settings.rs`

- **`SettingKind`** (`:274-282`) gains a third variant, doc'd:
  ```rust
  /// A JSON `true`/`false` (MOD-33 D267). `min` 0 and `max` 1 are nominal: the reader takes the
  /// boolean and nothing else, so a number, a string or `null` is refused rather than coerced.
  Boolean,
  ```
  Update the enum doc ("every key but one is an integer") to name the three kinds.
- **`SettingKey`** (`:150-172`): new first variant `BoxHostname = 0` (doc
  "`box_hostname`, the one [`SettingKind::Boolean`] and the one key only a project holds
  (MOD-33 D267)."); the ten existing variants shift to `1..=10`. `"box_hostname"` sorts before
  `"excerpt_file_line_cap"` bytewise, so byte order and discriminant order still coincide.
  Enum doc (`:142-148`): "The eleven keys of the registry — ANA-5 §5.3's ten `app_setting` keys
  and MOD-33's project-only `box_hostname` — declared in **key byte order**: the discriminant
  indexes [`SPECS`]."
- **`ALL`** (`:176-187`): `[Self; 11]`, `BoxHostname` first. Doc: "Every key, in key byte
  order. Not the `app_setting` list — that is [`Self::app_keys`]."
- **New** `app_keys` (after `ALL`):
  ```rust
  /// The keys whose spec admits the `App` rung, in [`Self::ALL`] order: the ten `app_setting`
  /// keys migration `0002` seeds, and no project-only key (MOD-33 D276). [`Defaults::as_rows`]
  /// and the `app` group of `Settings > Prompt` iterate this, never `ALL`.
  pub fn app_keys() -> impl Iterator<Item = Self> {
      Self::ALL.into_iter().filter(|key| key.spec().rungs.contains(Rungs::APP))
  }
  ```
- `from_key` doc (`:196-200`): "`None` for a string outside the eleven."
- **`SPECS`** (`:320`): `[SettingSpec; 11]`, new entry **at index 0**:
  ```rust
  SettingSpec {
      key: "box_hostname",
      project_key: Some("box_hostname"),
      kind: SettingKind::Boolean,
      min: 0,
      max: 1,
      rungs: Rungs::PROJECT,
      not_above: None,
      unit: "on/off",
      doc: "Whether a phase prompt's box section names this box; the digest never includes it.",
  },
  ```
  `SettingSpec.key` doc (`:292`) gains: "For a project-only key it is the registry's name and no
  `app_setting` row ever carries it."
- **New constant** near `DEFAULTS`:
  ```rust
  /// `project.settings.box_hostname`'s compiled default: the hostname is rendered (MOD-33 D267).
  /// Not a field of [`Defaults`], which is the `app_setting` table: no App rung carries the key.
  pub const BOX_HOSTNAME_DEFAULT: bool = true;
  ```
- **Exhaustive matches** — add the arm to both:
  `Defaults::value_of` (`:101-113`): `SettingKey::BoxHostname => Value::Bool(BOX_HOSTNAME_DEFAULT),`;
  `Defaults::integer` (`:124-138`): `SettingKey::BoxHostname => i64::from(BOX_HOSTNAME_DEFAULT),`
  (only reachable as a `not_above` peer, which nothing names; must still be total).
- **`Defaults::as_rows`** (`:88-93`): iterate `SettingKey::app_keys()`; doc "Iterated out of
  [`SettingKey::app_keys`]…".
- **`validate`** (`:481`): the `match spec.kind` (`:498-527`) gains
  ```rust
  SettingKind::Boolean => {
      let Some(on) = value.as_bool() else {
          return Err(format!("`{key}` must be a JSON boolean, got {value}"));
      };
      i64::from(on)
  }
  ```
  The range check that follows the integer/fraction arms is inside those arms, so nothing else
  moves. Extend the `# Errors` list with the boolean sentence.
- **New resolver**, after `resolve_max_skill_tokens` (`~:714-717`):
  ```rust
  /// `project.settings.box_hostname` (MOD-33 D267): whether the box section renders the hostname.
  ///
  /// The house fall-through rule: absent, `null` or anything but a JSON boolean is not a value, and
  /// [`BOX_HOSTNAME_DEFAULT`] answers. Project rung only — there is no `app_setting` row to fall to.
  /// Resolved by the caller and carried on [`PromptSpec`](crate::prompt::PromptSpec), because
  /// `assemble` is pure (ANA-5 invariant 2).
  #[must_use]
  pub fn resolve_box_hostname(project: Option<&Value>) -> bool {
      project_key(project, "box_hostname")
          .and_then(Value::as_bool)
          .unwrap_or(BOX_HOSTNAME_DEFAULT)
  }
  ```

### `crates/htui/src/prompt_settings.rs`

- `snapshot` (`:137-168`): the App loop becomes `for key in SettingKey::app_keys()`
  (`Vec::with_capacity(SettingKey::ALL.len())` may stay; it only over-reserves). **Required**:
  `setting(SettingRung::App, BoxHostname)` is a rung refusal on both stores, so looping `ALL` would
  fail the whole read.
- Docs: `SettingsSnapshot.app` (`:36`) "in [`SettingKey::app_keys`] order — always ten entries";
  `snapshot` doc (`:121-125`) "ten `setting(App, _)`" stays true. `project_keys` (`:109-117`) is
  unchanged: filtering `ALL` puts `BoxHostname` **first** in every project group.

### `crates/htui/src/ui/tabs/settings/prompt.rs`

- **Import** `resolve_box_hostname` beside the other resolvers (`:34-36`).
- **`Effective.source`** (`:235`): `BudgetSource` → `&'static str`, doc "Which rung answered, as
  the row prints it." Every existing `effective` arm passes `budget.source.as_str()` /
  `present.as_str()`; `value_line` (`:1020-1033`) prints `effective.source` directly.
  (Plan delta 1.)
- **New constants** (with the others, `:51-86`):
  ```rust
  /// What the source column says for a project-only key nothing stores: no `app_setting` row can
  /// exist for it, so `app_setting_default` would name a rung it does not have (MOD-33 D276).
  const COMPILED_DEFAULT: &str = "default";
  /// What a [`SettingKind::Boolean`] row's effective column and its pane say.
  const SWITCH_ON: &str = "on";
  const SWITCH_OFF: &str = "off";
  const SWITCH_RANGE: &str = "on or off";
  ```
- **`effective`** (`:938-995`): new arm, `number: None` (the fraction key's precedent, so D13's
  clamp line can never fire):
  ```rust
  SettingKey::BoxHostname => {
      let on = resolve_box_hostname(project);
      let source = if project_value.is_some() { BudgetSource::Project.as_str() } else { COMPILED_DEFAULT };
      ((if on { SWITCH_ON } else { SWITCH_OFF }).to_owned(), None, source)
  }
  ```
- **`submit`** (`:534-549`): third arm
  `SettingKind::Boolean => parse_switch(&text).map(Value::Bool).ok_or_else(|| switch_sentence(target.key)),`
  with, beside `integer_sentence` / `fraction_sentence` (`:1041-1049`):
  ```rust
  /// D276: `on`, `off`, `true`, `false`, ASCII case-insensitive; anything else is not a switch.
  fn parse_switch(text: &str) -> Option<bool> { … eq_ignore_ascii_case … }
  /// D11's sentence for a switch: shape, never a bound.
  fn switch_sentence(key: SettingKey) -> String {
      format!("`{key}` is on or off, or empty to clear")
  }
  ```
  `open_edit`'s prefill (`Value::to_string`) already yields `true` / `false` — no change.
- **`pane`** (`:655-661`): the range line is
  `match spec.kind { SettingKind::Boolean => format!("{SWITCH_RANGE} \u{b7} rungs {}", spec.rungs), SettingKind::Integer | SettingKind::Fraction => /* today's */ }`
  → `on or off · rungs project`.
- **Module doc** (`:11-18`): restate — "…read from the registry, so a key of an existing kind
  appears here without this file being touched. Two exceptions: the **effective** column needs
  each key's own resolver (an exhaustive `match key`, blueprint flag G), and a new
  [`SettingKind`] needs its parse in `submit` and its line in the pane — MOD-33's `Boolean` did."
- **In-file test** (tests module `:1085+`): `a_switch_parses_on_off_true_false_in_any_case`
  (`ON`, `Off`, `TRUE`, `false` parse; `maybe`, `1`, `""`, `yes` do not).
- **Never spell `"box_hostname"`** in this file: `no_key_name_is_spelled_in_the_section`
  (`tests/prompt_settings.rs:917-927`) loops `ALL`, now including it.

### Tests first

`settings.rs` in-file (tests module `:758+`):

| Test | Asserts |
|---|---|
| `box_hostname_is_a_project_only_boolean` (new) | spec fields exactly as above; `rung_refusal(BoxHostname, Rungs::APP)` and `(…, Rungs::PHASE)` are `Some`; `(…, PROJECT)` is `None`; `SettingKey::ALL[0] == BoxHostname`. |
| `validate_takes_a_boolean_and_nothing_else` (new) | `validate(BoxHostname, PROJECT, &json!(true)/json!(false), None) == Ok(())`; `json!(1)`, `json!("on")`, `Value::Null` refused with a sentence starting `` `box_hostname` must be a JSON boolean, got ``; on `Rungs::APP` the refusal is the rung sentence. |
| `resolve_box_hostname_falls_through_to_on` (new) | `true` for `None`, `json!({})`, `{"box_hostname": null}`, `{"box_hostname": 1}`, `{"box_hostname": "off"}`, `json!("not an object")`; `false` for `{"box_hostname": false}`; `true` for `{"box_hostname": true}`. |
| `app_keys_are_the_ten_app_setting_keys_in_byte_order` (new) | `app_keys().count() == 10`, excludes `BoxHostname`, equals `ALL` without it, strictly ascending bytes. |
| `every_default_validates_under_its_own_spec` (adapt `:836-846`) | validate each key on the **first rung its spec accepts** (`[APP, PROJECT, PHASE]`), not `APP` unconditionally. |
| `specs_are_indexed_by_discriminant` (adapt `:928-954`) | unchanged logic over eleven keys; message "eleven distinct keys". |
| `the_defaults_are_migration_0002s_ten_rows_verbatim`, `as_rows_is_key_byte_order` | **unchanged**; green because `as_rows` now reads `app_keys()`. |

`crates/htui/tests/prompt_settings.rs` — adapt, then add:

| Test | Change |
|---|---|
| `the_demo_snapshot_has_ten_app_entries_and_two_keys_per_project` (`:157-179`) | rename `…_and_three_keys_per_project`; `keys == SettingKey::app_keys().collect::<Vec<_>>()`; `value_keys == vec![BoxHostname, UpstreamHops, TokenBudget]`; `values[0].value == None`, `values[1].value == None`, `values[2].value == Some(json!(120_000))`. |
| `set_setting_on_project_stores_under_the_project_key_and_keeps_foreign_keys` (`:378-379`) | `entry.values[1]` is `UpstreamHops` / `Some(json!(2))`. |
| `app_row` (`:651-657`), `project_row` (`:661-667`) | `SettingKey::app_keys().position(…)` / `SettingKey::app_keys().count() + 2 + …`. |
| `the_demo_snapshot_renders_the_tree` (`:781-822`) | loop `SettingKey::app_keys()`; add `row_of(&frame, SettingKey::BoxHostname, 0).contains("unset | on (default)")`. |
| `no_workspace_lists_the_app_group_alone` (`:845`) | `SettingKey::app_keys().count()`. |
| `no_key_is_listed_on_a_rung_its_spec_refuses` (`:896-913`) | `app_keys().count()` in both places. |
| `set_setting_box_hostname_on_a_project_stores_a_boolean_and_keeps_foreign_keys` (new, worker half, beside `:334`) | `serve(demo(), SetSetting { rung: Project(PROJECT_VULKAN), key: BoxHostname, value: json!(false), expected: Some(token) })` → blob `box_hostname == false`; `retention_days`, `keep_raw_events`, `token_budget` kept; `values[0] == (BoxHostname, Some(json!(false)))`; `resolve_box_hostname(Some(blob)) == false`; project token moved. |
| `e_then_off_then_enter_sends_set_setting_false_on_the_project_rung` (new, shape of `:963-989`) | `move_to(project_row(BoxHostname))`, `e`, type `off`, `enter` → exactly one `SetSetting { rung: Project(PROJECT_VULKAN), key: BoxHostname, value: Value::Bool(false), expected: Some(project_token) }`. |
| `a_switch_that_is_not_on_or_off_is_refused_locally` (new, shape of `a_shape_refusal_names_the_key` `:1047`) | type `maybe`, `enter` → `bench.drained()` empty; `error_text` contains `` `box_hostname` is on or off, or empty to clear ``. |
| `the_switch_pane_says_on_or_off_and_the_project_rung` (new) | cursor on the `box_hostname` row → rendered section contains `on or off · rungs project`, no `range ` line, no `clamped to`. |

Then implement. Regenerate snapshots (review rule below).

### Commit

One commit (the new `SettingKey` variant breaks `crates/htui`'s two exhaustive matches, so the
core and UI halves cannot land apart): `feat(mod-33): box_hostname, a per-project switch in
Settings > Prompt`. Stage the four source/test files and the four `.snap` files by path.

### Verify

```bash
cargo test -p htui-core --all-features --lib prompt::settings -- --test-threads=1
cargo insta test -p htui --all-features --test prompt_settings -- --test-threads=1   # leaves .snap.new
cargo insta pending-list
# read every pending diff (diff -u X.snap X.snap.new) against the rule below, then:
cargo insta accept
cargo test -p htui --all-features --test prompt_settings -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres \
  cargo test -p htui --all-features --test prompt_settings -- --test-threads=1   # PG-gated case
cargo test -p htui-core --all-features -- --test-threads=1
cargo clippy -p htui-core -p htui --all-features --all-targets -- -D warnings
find crates -name '*.snap.new' | wc -l   # 0
```

**Snapshot rule (T1):** `demo`, `clamped`, `editor_fraction`, `stale` each gain **exactly one**
line, `  box_hostname                  unset | on (default)   on/off`, as the first row under
`project Vulkan Tutorials`, and lose one trailing blank line of padding; nothing else moves (the
cursor rows in those cases are `App` rows, so no scroll change). `app_only` and `offline` must not
produce a `.snap.new` — if they do, a key leaked into the `app` group.

### Hazards

- **Discriminant shift.** Every `SettingKey` discriminant moves up by one and `SPECS[0]` is the
  new entry. `SettingKey` derives no `Serialize` and is never persisted (plan fact 17), so nothing
  on disk moves — but a `SPECS` entry inserted anywhere but index 0 silently misindexes every key;
  `specs_are_indexed_by_discriminant` is the guard.
- `Effective.number = None` for the Boolean key, or D13's clamp arithmetic runs on a boolean.
- `KEY_WIDTH` (`:1011-1017`) still iterates `ALL`; `box_hostname` (12) < `excerpt_provider_deadline_ms`
  (28), so no column moves.
- `from_key("box_hostname")` is now `Some`, so `value_rows` (test helper `:756-765`) counts the
  project row — which is why the counts become `app_keys().count() + project_keys().count()`.
- The `(default)` label is D276's (plan delta 1); do not reuse `BudgetSource::AppSettingDefault`.

---

## T2 — the assembler splits sent from digested; the callers read the switch

**Files (complete):** `crates/htui-core/src/prompt/{render.rs, mod.rs, digest.rs, trim.rs, fixtures.rs}`,
`crates/htui-core/src/fixtures.rs`, `crates/htui-core/tests/prompt_hostname.rs` (new),
`crates/htui-core/tests/prompt_render.rs`,
`crates/htui-core/tests/snapshots/prompt_render__section_box_stand_in.snap` (new),
`crates/htui-core/tests/snapshots/prompt_render__section_box_no_hostname.snap` (new),
`crates/htui-core/tests/prompt_digest.rs`, `crates/htui-orch/src/engine.rs`,
`crates/htui-orch/src/conformance.rs` (doc), `crates/htui/src/preview.rs`,
`crates/htui/tests/prompt_preview.rs`,
`crates/htui/tests/snapshots/{prompt_preview__preview_feat_1, prompt_preview__preview_ana_2, backlog__detail_prompt}.snap`.

### T2.1 `render.rs` — the three hostname forms (behaviour-preserving)

After `EXCERPT_PREAMBLE` (`~:57`):

```rust
/// The fixed stand-in the digest sees in place of the box's hostname (MOD-33 D264).
///
/// A digest input for ever — changing these bytes changes every box-bearing digest — and never
/// sent or persisted: the digest text is hashed and dropped (D271). Ten characters, the length of
/// the fixture hostname `dev-win-01`, so the golden token figures do not move (D265).
pub const HOSTNAME_STAND_IN: &str = "[hostname]";

/// Which `hostname:` line [`box_profile`] renders (MOD-33 D263, D268, D269).
///
/// Policy about a prompt, not a fact about a box, so it is an argument and not a `BoxProfile`
/// field: `BoxProfile` is also the MCP `box_profile` tool's payload (D277).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostnameLine {
    /// The project's switch is off: no `hostname:` line; the section starts with `os:`.
    Omitted,
    /// The digest form, `hostname: [hostname]` — what the residue scan, the estimate, the trim
    /// ladder and the record see.
    StandIn,
    /// The sent form: the (already masked) hostname, single-lined (D266).
    Shown,
}
```

`box_profile` (`:370-426`) becomes `pub fn box_profile(profile: &BoxProfile, hostname: HostnameLine) -> Rendered`.
Only the first push (`:376`) changes:

```rust
match hostname {
    HostnameLine::Omitted => {}
    HostnameLine::StandIn => lines.push(format!("hostname: {HOSTNAME_STAND_IN}")),
    HostnameLine::Shown => lines.push(format!("hostname: {}", single_line(&profile.hostname))),
}
```

plus a private helper (D266 — **not** `attr`, whose collapse is per character, `:148`):

```rust
/// MOD-33 D266: each `\r\n` pair becomes one space, then any remaining `\r` or `\n` one space, so
/// the sent hostname can never add or remove a line and `canonical()` commutes with the swap.
fn single_line(value: &str) -> String {
    value.replace("\r\n", " ").replace(['\r', '\n'], " ")
}
```

Doc of `box_profile` gains: "`hostname` picks the first line's form; every other line is
identical in all three forms, which is what confines the sent/digest difference to one value."
Rewrite the H-1 paragraph of `template_text`'s doc (`:255-258`) to end: "…are the same strings
(review finding H-1). The one byte range the estimate does not measure as sent is the box
section's hostname value, which is estimated as [`HOSTNAME_STAND_IN`] (MOD-33 D265)."

Callers updated in the same commit, all to `HostnameLine::Shown` (today's bytes):
`prompt/mod.rs:631` (`render_sections`), `tests/prompt_render.rs:40`, `:207`, `:232`, and the
in-file test `:1183`, `:1205`, `:1209`.

Tests (write first):

- `render.rs` in-file: `the_box_profile_omits_what_the_row_does_not_have_and_never_a_path`
  (`:1164-1213`) passes `HostnameLine::Shown`, expectations **byte-for-byte unchanged**. New
  `the_hostname_line_has_three_forms`: `StandIn` content equals the `Shown` content with its first
  line replaced by `hostname: [hostname]`; `Omitted` content equals the `Shown` content minus its
  first line and starts with `os: `. New `a_shown_hostname_is_one_line`: `"a\r\nb"` → first line
  `hostname: a b`; `"a\rb\nc"` → `hostname: a b c`; `"a\r\n\r\nb"` → `hostname: a  b`; no
  output contains `\r`.
- `tests/prompt_render.rs`: new `the_box_section_renders_its_digest_and_off_forms` —
  `insta::assert_snapshot!("section_box_stand_in", render::wrap(&render::box_profile(&spec.box_profile, HostnameLine::StandIn)))`
  and `"section_box_no_hostname"` with `Omitted`, over `fixtures::phase_implement_attempt2()`.

Commit **T2.1**: `feat(mod-33): render the box hostname as shown, stood-in or omitted`. At this
commit no prompt, digest or snapshot other than the two new files changes.

### T2.2 the split — trim v 4, the assembler, every caller, the recorder switch

This is one commit because the `PromptSpec` field breaks every full literal and the digests shown
in three `crates/htui` snapshots change with it.

**`trim.rs`**

- `RECORD_VERSION` (`:56-59`): `4`; doc "`trim_record.v`: version 4 since MOD-33 D270 added
  `undigested`; 3 since MOD-9 D118…" (keep the rest).
- New closed vocabulary after `TrimStrategy::as_str` (`~:95`):
  ```rust
  /// A span rendered into the prompt text but excluded from `prompt_digest` (MOD-33 D263, D270).
  ///
  /// Closed vocabulary, like [`TrimStrategy`]; the digest text carries the span's fixed stand-in
  /// in its place, and the recorder hashes the digest text (D271).
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
  pub enum UndigestedSpan {
      /// The box section's `hostname:` value; the digest sees [`render::HOSTNAME_STAND_IN`].
      #[serde(rename = "box.hostname")]
      BoxHostname,
  }
  impl UndigestedSpan {
      /// The one spelling, as `trim_record.undigested[]` serialises it.
      #[must_use]
      pub const fn as_str(self) -> &'static str { match self { Self::BoxHostname => "box.hostname" } }
  }
  ```
- `TrimRecord` doc (`:175-183`): "`run_step.trim_record`, version 4 (ANA-5 §5.1 as amended by
  MOD-9 D42/D118 and MOD-33 D270)." New field **between `skill_choices` and `excerpts`** (the
  order `0010`'s comment lists):
  ```rust
  /// Every span rendered into the prompt but excluded from `prompt_digest` (MOD-33 D270):
  /// `["box.hostname"]` when the box section named the hostname, else `[]`. Always present, and
  /// listed once however often the body places `{{box}}`. In v 4, a `box` entry in `sections`
  /// beside an empty list means the project's switch was off.
  pub undigested: Vec<UndigestedSpan>,
  ```
- `record()` (`:1078-1111`): new parameter `undigested: Vec<UndigestedSpan>` after
  `skill_choices`, stored into the field; add
  `#[expect(clippy::too_many_arguments, reason = "the record's eight inputs are eight different facts from one assembly; a struct would be built at its one call site only")]`
  (plan delta 2).
- In-file test: `the_undigested_spelling_is_the_serde_one` (`serde_json::to_value(BoxHostname) == json!(BoxHostname.as_str())`).

**`mod.rs`**

- `pub use trim::{…, UndigestedSpan};` (`:49`).
- `PromptSpec` (`:73-130`): new field **after `box_profile` (`:99`)**:
  ```rust
  /// `project.settings.box_hostname`, resolved by the caller (MOD-33 D267, D268): whether the box
  /// section names the hostname. `true` renders it and keeps it out of the digest (D263);
  /// `false` omits the line and neither masks nor scans the value (D269).
  pub box_hostname: bool,
  ```
- `AssembledPrompt` (`:322-337`), doc rewritten:
  "The assembled prompt: two canonical strings from one set of trimmed sections (§4.7 step 8 as
  amended by MOD-33). `text` is what is sent and what is persisted. `digest_text` is what is
  digested: the same text with every undigested span's value replaced by its fixed stand-in —
  today only the box section's hostname, as [`render::HOSTNAME_STAND_IN`] — and the two differ in
  exactly those spans, which `trim.undigested` names. With no such span they are one string. …
  (keep the 'canonicalise for the hash while sending the original bytes was rejected' sentence,
  and add: the digest text is not that option — it differs from the sent text only in declared,
  recorded spans, never in formatting)."
  Fields: `text` "The canonical assembled prompt, as sent."; **new** `digest_text: String` after
  `text`, doc "The canonical text [`digest`](Self::digest) is over: [`text`](Self::text) with each
  span of `trim.undigested` replaced by its stand-in. Handed to the recorder, which hashes it
  (D271); never sent, never persisted."; `digest` "`sha256` over
  [`digest_text`](Self::digest_text)'s UTF-8 bytes, lowercase hex, 64 characters."
- `render_sections` (`:620-683`), box arm (`:631`):
  `Placeholder::Box => vec![render::box_profile(&spec.box_profile, if spec.box_hostname { HostnameLine::StandIn } else { HostnameLine::Omitted })],`
  — the pipeline's box is the **digest form**.
- `scrubbed_inputs` (`:734-889`), box block (`:767-780`): copy `let shown = spec.box_hostname;`
  before borrowing `profile`; mask `profile.hostname` only `if shown`; the other five strings and
  the tools as today. Comment rewrite: "`box` and `skills` are protected and never re-rendered by
  the trimmer; `assemble` re-renders the box once more after the trim, in its sent form, from
  these same masked inputs (MOD-33 D263). The hostname is masked only when the switch renders it:
  a value the model never sees must not refuse a prompt (D269)."
- `assemble` (`:438-527`):
  1. **After the residue loop (`:484-490`)**, render and scrub the sent box through the same pass:
     ```rust
     // MOD-33 D263/D269: the sent form of the box section, when it carries the hostname. Rendered
     // from the masked spec and put through the same mask-and-scan as every section above, so a
     // known secret masked in the digest form cannot survive in the sent form.
     let sent_box = if spec.box_hostname && parsed.used.contains(&Placeholder::Box) {
         let mut section = render::box_profile(&spec.box_profile, HostnameLine::Shown);
         let name = section.name.render();
         section.content = scrub_text(scrubber, &section.content, &name)?;
         Some(section)
     } else {
         None
     };
     ```
     (`spec` here is already `&masked.spec` — the shadow at `:465`. The box has no `attrs`.)
  2. **Replace `:506-511`** (substitute / canonical / hash):
     ```rust
     // 7. Substitute in span order, once per form. 8. Canonicalise both and digest the digest text.
     let digest_text = digest::canonical(&substitute(&parsed, &masked.literals, &live, &scalars));
     let text = match &sent_box {
         None => digest_text.clone(),
         Some(sent) => {
             // Swapped by section name, never by searching the text (D263 (b)).
             let sent_live: Vec<(Placeholder, Rendered)> = live
                 .iter()
                 .map(|(placeholder, section)| {
                     let section = if section.name == SectionName::Box { sent.clone() } else { section.clone() };
                     (*placeholder, section)
                 })
                 .collect();
             digest::canonical(&substitute(&parsed, &masked.literals, &sent_live, &scalars))
         }
     };
     let digest = digest::sha256_hex(&digest_text);
     let undigested = if sent_box.is_some() { vec![UndigestedSpan::BoxHostname] } else { Vec::new() };
     ```
  3. `trim::record(…, masked.skill_choices.clone(), undigested, surviving_audit(…)?, notes(spec))`;
     `AssembledPrompt { text, digest_text, digest, sections, trim }`.
- `substitute`'s doc (`:549-553`): end with "…the very strings this substitutes (H-1) — except
  the box section's hostname value, which the estimate measured as the stand-in and the sent
  substitution carries as the hostname (MOD-33 D265). `assemble` calls this twice, once per form."
- `assemble`'s doc: one sentence — "Pure by contract… `text` and `digest_text` are two
  substitutions of one set of trimmed sections (MOD-33 D263)."

**`digest.rs`** (docs only): module doc `:4-8` — "…the text alone, canonicalised, as sent **with
each undigested span's value replaced by its fixed stand-in** (MOD-33: the box hostname)…";
`sha256_hex` doc `:66-71` — "The same `sha2` call `Recorder::record_prompt_digesting` makes over
the scrubbed **digest text**, so the recorder's recomputation is an identity over the string ANA-5
supplies for digesting. The text sent and the text digested differ only in the spans
`trim_record.undigested` names."

**Literals** (`box_hostname` placed after `box_profile` in each):
`prompt/fixtures.rs:161`, `:260`, `:363`, `:502` → `box_hostname: true,`;
`engine.rs:4421` (`judge_prompts`, box at `:4439`) and `:5128` (`phase_spec`, box at `:5144`) →
`box_hostname: settings::resolve_box_hostname(Some(&project.settings)),`;
`preview.rs:279` → a `let box_hostname = settings::resolve_box_hostname(Some(&project.settings));`
beside `budget` / `hops` (`:217-222`) and `box_hostname,` in the literal. Spreads need nothing
(`mod.rs:541`, `:1365`, `promote.rs:105`, `:429`, `excerpt.rs:1142`); the compiler is
authoritative.

**`engine.rs`**: `open_recorder` (`:5402-5404`) →
`.record_prompt_digesting(&prompt.text, &prompt.digest_text, prompt.payload_sections_value(), self.now())`;
its doc (`:5373-5375`) names `record_prompt_digesting`. Doc `:6119` "closed enum of ten" → "of
eleven". **`conformance.rs`** doc `:1849-1850` likewise.

**`crates/htui-core/src/fixtures.rs`**: in `step_impl_carries_the_golden_trim_record`
(`:2512-2536`), next to `live["v"] = …` (`:2531`), strip the key the literal predates:
`live.as_object_mut().expect("a record is an object").remove("undigested");`; comment "MOD-33
D270: `undigested` is v 4's; the demo literal predates it". Doc `:2501-2502` and the comment at
`:2524-2526` say "the assembler writes `v 4`". `IMPL_TRIM_RECORD` (`:1627-1678`) is **not**
edited (its `box` 116 tokens hold: `dev-win-01` and `[hostname]` are both 10 characters).

**`tests/prompt_digest.rs`** `to_value_is_byte_stable_and_carries_the_documented_keys`
(`:990-1010`): add `"undigested"` to the key set; message "…MOD-33 D270's undigested: fourteen";
`first["v"] == json!(4)`, message "MOD-33 D270 bumped the record to v 4"; add
`assert_eq!(first["undigested"], json!(["box.hostname"]))`.

### T2.2 tests first — new `crates/htui-core/tests/prompt_hostname.rs`

Header `#![cfg(feature = "test-support")]`; module doc "MOD-33: the box hostname is rendered but
not digested." Helper `ok(spec)` with `MinimalScrubber::new([])`, and
`with_host(host: &str) -> PromptSpec` = `phase_implement_attempt2()` with
`box_profile.hostname = host`. (No `Utc::now`, no `HashMap` — `the_prompt_module_reads_no_clock`
does not read this file, but the invariant is the same.)

| # | Test | Asserts |
|---|---|---|
| 1 | `the_digest_does_not_move_with_the_hostname` | `dev-win-01` vs `a-much-longer-build-host.example.internal`: equal `digest`, equal `digest_text`, equal `trim.to_value(&scrubber())`; different `text`; each `text` contains `hostname: <its host>\n`. |
| 2 | `the_digest_text_is_the_sent_text_with_the_stand_in` | `digest_text == text.replacen("hostname: dev-win-01", "hostname: [hostname]", 1)` (safe **only** for this fixture: `{{box}}` placed once, nothing else spells it); `canonical(&digest_text) == digest_text`; `sha256_hex(&digest_text) == digest`. |
| 3 | `the_switch_off_omits_the_line_and_records_nothing_undigested` | `box_hostname = false`: `!text.contains("hostname:")`; `text == digest_text`; `trim.undigested.is_empty()`; `trim.sections` still has `SectionName::Box`; `digest != ` the switch-on digest. |
| 4 | `switch_on_records_box_hostname_once` | body = implement body + `"\n{{box}}\n"` (placed twice): `text.matches("hostname: dev-win-01").count() == 2`, `digest_text.matches("hostname: [hostname]").count() == 2`, `trim.undigested == vec![UndigestedSpan::BoxHostname]`. |
| 5 | `an_off_switch_never_refuses_over_the_hostname` | host `sk-ant-api03-abcdefghijklmnopqrstuvwx`: on → `Err(AssembleError::Unmasked { section, .. })` with `section == "box"`; off → `Ok`, `!text.contains("sk-ant-")`. |
| 6 | `a_newline_in_the_hostname_is_one_line_in_the_text` | `"a\r\nb"` → `text` contains `"hostname: a b\n"`; `"a\rb\nc"` → `"hostname: a b c\n"`; both digests equal the `dev-win-01` digest; no `\r` in `text`. |
| 7 | `an_item_body_that_spells_the_box_is_left_alone` | `item_body` containing `<section name="box">\nhostname: [hostname]\n` and `dev-win-01`: the `<section name="item"…>…</section>` slice is identical in `text` and `digest_text`; `text` holds `dev-win-01` twice, `digest_text` once. |
| 8 | `the_record_is_v4_with_undigested` | `to_value(..)["v"] == 4`, `["undigested"] == json!(["box.hostname"])`; `judge_three_candidates()` and `handoff_basic()` → `["undigested"] == json!([])` and `text == digest_text`. |
| 9 | (criterion 5) | no new test: `fan_out_siblings_get_identical_bytes` (`prompt_digest.rs:62`) stays green untouched. |

**Engine** (`engine.rs` tests module, near `implement_prompt` `:13533`):
`a_step_digest_is_the_payload_digest_and_not_the_sent_text_hash`, `#[tokio::test]`:

1. `let first = Harness::new().await;` `let (run, step) = started(&first).await;` (`:7894`, parks
   FEAT-3 at `prd`, whose default body places `{{box}}`).
2. Read `first.orch.steps(run)` → the step's `prompt_digest`; `first.orch.store.step_events(step)`
   → seq-0 `Prompt` payload `text` and `digest`. Assert `prompt_digest == Some(digest)`
   (criterion 11), `digest != htui_core::prompt::digest::sha256_hex(text)`, and `text` contains
   `hostname: DESKTOP-HTUI`.
3. Second harness over a box with another hostname (plan delta 4):
   ```rust
   let mut orch = FakeOrchestrator::demo();
   let mut data = htui_core::fixtures::demo_data();
   data.boxes.iter_mut().find(|row| row.id == ids::BOX).expect("the demo box").hostname =
       "a-much-longer-build-host.example.internal".to_owned();
   orch.store = MemStore::from_demo(data).with_clock(std::sync::Arc::new(orch.clock.clone()));
   let second = Harness { orch };
   ```
   Walk it the same way; assert the two `prompt_digest`s are equal and the two payload `text`s
   differ.

**Preview** (`crates/htui/tests/prompt_preview.rs`, beside `the_preview_assembles_from_real_store_reads` `:129`):

- `two_boxes_that_differ_only_in_hostname_preview_one_digest`: two `Backend::memory(MemStore::from_demo(data))`
  where the second `data` has `ids::BOX`'s hostname changed; `preview::build(…, item_id("FEAT-1"), None, &platform_scope())`
  on both → `Ok` outcomes with equal `digest`, different `text`, each containing its own `hostname:` line.
- `a_project_that_turns_the_switch_off_previews_no_hostname`: FEAT-1's project blob
  (`ids::PROJECT_HTUI`) gets `"box_hostname": false` → outcome `text` has no `hostname:` and
  `text == digest_text`.

### T2.2 commit

`feat(mod-33): the box hostname is rendered but not digested` — everything above: `trim.rs`,
`mod.rs`, `digest.rs`, `prompt/fixtures.rs`, `crates/htui-core/src/fixtures.rs`,
`prompt_hostname.rs`, `prompt_digest.rs`, `engine.rs`, `conformance.rs`, `preview.rs`,
`prompt_preview.rs`, and the three regenerated `crates/htui` snapshots. If a token figure moved in
a preview snapshot (below), say so in the body.

### T2 verify

```bash
cargo test -p htui-core --all-features -- --test-threads=1
cargo insta test -p htui-core --all-features --test prompt_render -- --test-threads=1
cargo test -p htui-agent --all-features -- --test-threads=1        # step_pass / excerpt spreads
cargo test -p htui-orch --all-features -- --test-threads=1
cargo insta test -p htui --all-features --test prompt_preview --test backlog -- --test-threads=1
cargo insta pending-list          # read each diff (diff -u X.snap X.snap.new), then:
cargo insta accept
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres \
  cargo test -p htui --all-features -- --test-threads=1
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo fmt --all -- --check
find crates -name '*.snap.new' | wc -l                 # 0
```

**Snapshot rules (T2):**
- `htui-core`: `section_box`, `section_box_minimal`, every `prompt_golden__*` and every other
  `prompt_render__*` **must not change** (the sent text is today's text). The two new files are
  accepted after reading: `section_box_stand_in` is `section_box` with its first content line
  `hostname: [hostname]`; `section_box_no_hostname` is `section_box` without that line.
- `crates/htui`: in `prompt_preview__preview_feat_1`, `prompt_preview__preview_ana_2` and
  `backlog__detail_prompt` the `digest` line changes (the backlog frame shows only its visible
  prefix). The preview box is `DESKTOP-HTUI` (12 chars) against a 10-char stand-in, so a
  `tokens` figure and the `box` row of the section table may move by **at most one** (D265);
  anything else moving is a bug.

### T2 hazards

- **`canonical` locality (D263/D266).** The two forms must differ in the hostname value only:
  one `box_profile` body, one `match` on the first push, identical remaining lines; the sent value
  single-lined; the swap by `SectionName::Box`. Never `str::replace` the hostname in `text`.
- **Same mask-and-scan (D269).** The sent box goes through `scrub_text` (mask **and** scan), from
  `masked.spec`, right after the residue loop — not a scan alone, and not from the caller's
  unmasked `spec`.
- **Only the digest form enters the trimmer.** `rendered` (scan, estimate, trim, record) carries
  `StandIn`/`Omitted`; `Shown` exists only in `sent_box`/`sent_live`.
- **`undigested` from the swap, not from the text.** Listed once however many `{{box}}` slots
  (`parsed.used` dedups; one `Rendered` feeds every slot, `substitute` `:556-561`).
- **Engine source greps** (`prompt_digest.rs:1680-1696`): `engine.rs` must keep exactly three
  `to_value(self.parts.scrubber` and no other `to_value(` line — the new engine test must not call
  `to_value`. Also no clock read in `crates/htui-core/src/prompt/*` (`:108-146`).
- **D265 token drift.** `phase_all_empty` (`ci-linux-01`, 11 chars) and any test box whose hostname
  is not 10 characters (`HTUI-TEST`, `SECOND-BOX`, `DESKTOP-HTUI`) may estimate one token
  differently. Read each such failure; never change the stand-in to make one pass.
- `excerpt_residual` / `room_without_excerpts` now measure the digest form — intended (D265).
- `judge_prompts` replays the stored payload `text` (the sent form) as the judge's task (D272):
  do not rewrite it.

### T2.3 (optional, docs only)

If any doc sweep did not fit T2.2: `docs(mod-33): …` touching only T2 files (e.g. the judge doc
`engine.rs:4573` naming `record_prompt` at seq 0 — still true for the judge, whose two strings are
equal; leave it unless it reads wrong).

---

## T3 — the column comments say what the columns now hold (D273)

**Files (complete):** `crates/htui-store/migrations/0010_prompt_digest_undigested.sql` (new),
`crates/htui-store/tests/migrations.rs`, `crates/htui-store/tests/connect.rs`.

### Tests first (`tests/migrations.rs`)

- New const after `MOD23_COLUMN_COMMENTS` (`:418-425`), doc "The two `COMMENT ON COLUMN` texts of
  `0010_prompt_digest_undigested.sql` (MOD-33 D273), verbatim, for [`ANA_COLUMN_COMMENTS`]'s
  reason":
  `const MOD33_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[("run_step", "prompt_digest", …), ("run_step", "trim_record", …)];`
  The two texts are the SQL literals below **concatenated**, with `''` read as `'` — i.e.
  `"… over the canonical assembled prompt TEXT as sent with each undigested span replaced by its fixed stand-in - today only the box section's hostname value, as [hostname] - LF normalised, …"`.
  Write them with `\` line continuations as the neighbouring consts do.
- Remove the `run_step.prompt_digest` and `run_step.trim_record` rows from `ANA_COLUMN_COMMENTS`
  (`:208-228`). Doc `:176-183`: "twenty-five" → "twenty-three", "the next five are ANA-5 §9" →
  "the next three"; replace the `0008` sentence with "`run_step.prompt_digest` and
  `run_step.trim_record` are restated by `0010_prompt_digest_undigested.sql` (MOD-33 D273) and are
  pinned in [`MOD33_COLUMN_COMMENTS`]."
- Chain `.chain(MOD33_COLUMN_COMMENTS)` at **all three** sites: the verbatim loop (`:444-447`),
  the table list (`:497-501`), the expected set (`:511-515`). Total stays 35 (23+4+5+1+2). Update
  the messages at `:469` "(or MOD-7, MOD-9, MOD-23 or MOD-33)" and the comment `~:486-488`.
- Count pins: `:88` `vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]` and message "… and MOD-33's
  0010_prompt_digest_undigested.sql, in ordinal order"; `:1174` `9` → `10`, message "the ten
  embedded migrations (through MOD-33's 0010_prompt_digest_undigested.sql)"; `:901`
  `Pending(10)` and `:902` "ten embedded migrations"; `:1000` `Pending(10)`; `:1005`, `:1024`
  `HeadlessError::MigrationsPending(10)`.

`tests/connect.rs`: `:140` `Pending(10)`, `:141-142` "ten embedded migrations, through MOD-33's
0010_prompt_digest_undigested.sql"; `:155` `pending, 10`, `:156-158` "all ten … through MOD-33's
0010_prompt_digest_undigested.sql"; `:242` `Pending(10)`, `:243-244` "ten embedded migrations
since MOD-33's 0010_prompt_digest_undigested.sql".

Run against Postgres: the comment and count pins must fail (no `0010` yet). Then add the migration.

### `0010_prompt_digest_undigested.sql`

```sql
-- 0010_prompt_digest_undigested.sql - MOD-33 (plan D263, D270, D271, D273).
-- Forward-only (R-STO-5).
--
-- run_step.prompt_digest is restated: the digest is over the digest text, the sent text with each
-- undigested span's value replaced by its fixed stand-in. run_step.trim_record is restated for
-- record v 4, which gains undigested[]. Comment only: no table, column, constraint or index
-- moves, and the SQLite mirror's schema is untouched. schema_version still becomes 10, so each
-- box rebuilds its mirror once on first start (0008's R-56).

<the two COMMENT ON COLUMN statements, verbatim from the plan, T3 section>
```

### Commit

`feat(mod-33): 0010 restates run_step.prompt_digest and trim_record`.

### Verify

```bash
df -h /                                       # R-8: a crash-looping dev Postgres is disk pressure first
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres \
  cargo test -p htui-store --all-features --test migrations --test connect -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres \
  cargo test -p htui-store --all-features -- --test-threads=1
cargo clippy -p htui-store --all-features --all-targets -- -D warnings
ls crates/htui-store/.sqlx | wc -l            # 289, unchanged
```

### Hazards

- Chaining only the first site fails "exactly the thirty-five commented columns".
- Re-check `0010` is still free right before committing (R-4); renumber at merge if another item
  took it.
- A Postgres failure while the container recovers is not a test failure: re-run the case alone.

---

## Whole-item gate (after all four tasks, on the real tree)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test -p htui-core --all-features -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
find crates -name '*.snap.new' | wc -l                       # 0
ls crates/htui-store/.sqlx | wc -l                           # 289
git diff --stat f5de3d4 -- crates/htui-core/src/store crates/htui-store/src \
  crates/htui-core/src/model crates/htui-store/.sqlx crates/htui-store/cache_migrations   # empty
```

`--test-threads=1` is not optional (the keyring fake is process-wide); a green parallel run
proves nothing.
