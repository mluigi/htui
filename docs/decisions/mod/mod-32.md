# MOD-32 - `trim_record`'s own strings reach the store unscrubbed (done, 2026-09-28)

## What was built

`run_step.trim_record` was written by three production sites in
`crates/htui-orch/src/engine.rs` — `walk_live_step`, `candidate_live`, `judge_sessions` — each a
bare `serde_json::to_value(&prompt.trim)` that reached `set_step_prompt` with nothing having
scrubbed it. `TrimRecord::to_value()`, the one method that exists to produce exactly that
`Value`, was called from tests and fixtures only; the engine re-serialised the struct around it.
So the record's own strings were governed by nothing, and `R-SEC-3` gates the **persist** path.

The signature changed rather than a method being added beside the old one:

```rust
pub fn to_value(&self, scrubber: &dyn Scrubber) -> Result<Value, Unmasked> {
    let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
    scrubber.scrub(&mut value)?;
    Ok(value)
}
```

All three engine sites call it. The first hop of the error chain is explicit —
`Unmasked` converts to `RecordError` (`record.rs:251`) and `RecordError` converts to
`EngineError` (`command.rs:493`), and `?` applies exactly one `From`; no new error variant was
added.

A refusal is raised before the record is written and before that step's own session starts, so
nothing is persisted and the step spends no token. It is **not**, however, a step failure in the
usual sense: the `?` propagates to `fail_hard` / `fail_candidate` / `fail_judge`, each of which
settles the run as `RunStatus::Failed` with the rendered message as an **untyped** `run.failure`
— outside the closed `RunFailure` vocabulary — and runs `cleanup_run`. It does not block the item
the way the assembler's own refusals do through `refuse_prompt`, so an item whose
`template.name`, a caller note or a repo slug trips a prefix rule is left actionable and will be
re-run into the same abort. Plan **D8** chose that shape over moving the scrub into `assemble`;
the maintainer reviewed the consequence and chose to document it rather than change it.

The pass is **whole-record and enumerates nothing**: one `scrub` over the serialised `Value`,
reaching every string leaf and every object key. That is the point of the fix, not a stylistic
choice — the defect *was* a guarantee expressed as an enumeration.

## Why the whole record, and what was actually exposed

Most of the record was already masked, because it derives from the spec `scrubbed_inputs` masked.
Five things were not:

| Field | Why it leaked |
|---|---|
| `trim_record.template.name` | a `prompt_template` row name; no `mask` call in `scrubbed_inputs` names `spec.template` |
| `trim_record.notes[0..spec.notes.len()]` | `PromptSpec.notes`, caller free text, cloned verbatim by `notes()` |
| `trim_record.notes[…excerpts]` | `ExcerptSet.notes` |
| `trim_record.excerpts.roots[].repo` | `scrubbed_inputs` walks `spec.excerpts.files` and **never** `spec.excerpts.audit`; `surviving_audit` clones the audit wholesale (`prompt/mod.rs:1084`) and rebuilds only `files[]` |
| `trim_record.excerpts.provider_set[]` | the same gap |

`budget_source` and `estimator` are closed spellings and were never at risk.

**The HANDOFF item's own description was partly wrong**, and the correction is part of the
result. It named "the `excerpts` audit's paths and root strings" and "repo-relative paths and enum
spellings". `excerpts.files[]`'s paths *are* already masked — the item's `scrub_text` citation
was right about them. The root strings are not. And two of the five fields it did not mention at
all. The plan's first fact-check pass repeated the item's error and was itself corrected; the
correction is recorded in the plan's claims table rather than quietly fixed, because a claim the
first pass got wrong is more useful recorded.

The item also posed a false alternative: "decide between scrubbing `to_value`'s output before the
write **and** refusing the write on residue". They are not alternatives — `Scrubber::scrub` masks
in place *and then* returns `Err(Unmasked)` (`scrub.rs:228-233`), and the assembler already does
both in that order. The real decision was **where** the call goes, and it goes at the write site:
the bytes that are persisted are the bytes that get scrubbed, with no second representation in
between.

## The store is not the enforcement point

Stated plainly, because it is a real limit and not an oversight. `PgStore::set_step_prompt`
binds `$3` straight into the `UPDATE`, `State::set_step_prompt` clones the `Value`, and
`State::finish_step` / `PgStore::finish_step` are a **second** writer of the column
(`trim_record = COALESCE($4, trim_record)`). Neither store validates or scrubs. The store-level
case at `pg_criteria.rs:1076` deliberately still writes a hand-built record.

`finish_step` is **latent**: `StepOutcome.trim_record` is `Some(..)` at exactly two sites, both in
test code, and every production initialiser is `None`. After this item no production caller writes
an unscrubbed record, and the SQLite cache mirror only ever carries what Postgres already holds.

## Commits

| Commit | What |
|---|---|
| `90d2b1a` | the scrubbed serialiser, the three call sites, the six serialisations converted |
| `968e418` | the pins |
| `5377a8f` | the implementation blueprint |
| `8c1e92f` | the review gate: a pin that could not fail, and docs that claimed more than the code |
| `921654e` | the refusal's real cost, and the status narrative stripped |
| `b01e90f` | the close-out, and MOD-61 opened |
| `d9836b7` | the vacuous workspace walk, and two reverts nothing caught |

Branch `mod-32`, branched off `mod-58`'s `8cc3fda`. Plan:
`.claude/plans/mod-32-trim-record-scrub.plan.md`.

## Tests, and the mutations behind them

Five pins in `crates/htui-core/tests/prompt_digest.rs`, where the assembler's own scrub cases
already live, plus three grep guards over the whole workspace. A pass is a guarantee; the
mutations are the proof it bites. Each was applied, run, and reverted — the maintainer authorised
each explicitly, and `trim.rs` is clean in every case.

| # | Mutation | Observed |
|---|---|---|
| M1 | delete the `scrub` call | 5 pins fail: `:1407` (secret persisted in plain text), `:1434`, `:1473`, `:1499`, `:1545` |
| M2 | scrub the `notes` slot only — the field enumeration this item exists to remove | the template-name and audit pins fail; the refusal pin fails too, and on a fact worth recording: the pointer degrades from `/notes/0` to `/0`, because a sub-value scrub has lost the field it sat in. An enumeration does not only lose coverage, it loses the location a maintainer acts on. |
| M3 | residue-only — scrub a throwaway clone, keep the unmasked original | the refusal pin passes (as designed), the masking pin fails at `:1407` with `"clamped hops to hunter2 on this box"`. Masking and refusing are one call, not interchangeable halves. |
| M4 | keep the mask, discard the refusal (`let _ = scrubber.scrub(&mut value)`) | compiles, masks correctly, never fails closed; the three refusal pins fail at `:1437`, `:1476`, `:1502` |

Gate: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-features --all-targets -D
warnings`, and `cargo test --workspace --all-features -- --test-threads=1` — **2276 tests, 82
binaries, 0 failures**, including the Postgres gate. `.sqlx` still 268; migrations still
`0001`–`0007`; no store, trait, error variant, conformance case, migration, `.sqlx` entry or count
pin moved.

## The review gate

`rust-reviewer` over the change set: **Warning**. No CRITICAL, four MEDIUM, three LOW, all
applied. The one that mattered was a pin that could not fail —
`scrubbing_a_record_twice_changes_nothing` called `to_value` twice on the same *unmasked* record
and compared the results, which asserts that a pure function is pure; a `mask` that re-masked the
`[REDACTED]` marker would have passed it. Rewritten as
`a_masked_record_survives_a_second_pass_unchanged`, which feeds an already-masked value back in
and asserts the record really carries a mask so the case cannot be vacuous.

The gate also found a limit the plan never stated, and it is now stated in
`TrimRecord::to_value`'s doc: **the masking half of the new pass is inert in production.** The
run engine builds its `MinimalScrubber` with an **empty** secret list (`run_worker.rs:1253`,
`:935`) and `mask` returns its input unchanged when the list is empty (`scrub.rs:119-121`), so
only the prefix rules and the PEM marker fire. The fail-closed half — what `R-SEC-3` requires and
what the item asks for — is live and works. The chat path's populated scrubber
(`agent_worker.rs:3059`) never reaches `set_step_prompt`. That is **MOD-61**, opened at the gate.

The gate also closed a gap the signature change cannot: `TrimRecord` is `pub` and still derives
`Serialize`, so `serde_json::to_value(&trim)` — the literal line this item deleted — remains
valid Rust and nothing in the build fails if it returns.
`a_trim_record_is_never_serialised_outside_to_value` is the grep that keeps the rule a
convention rather than an accident, in the same idiom as
`the_only_section_entry_constructor_is_the_records_projection` beside it.

One reversion nothing catches, recorded rather than left for a reader to find: at the **engine
layer**, replacing `.map_err(RecordError::from)?` with `.unwrap_or(Value::Null)` compiles and
leaves every pin green. The plan's coverage map claimed "compile-forced by the signature" for the
three sites, which is true of `to_value` and false of the `?` that consumes it.

**Three guards a signature cannot enforce**, added at the second review. `TrimRecord` is `pub` and
still derives `Serialize`, so `serde_json::to_value(&trim)` stays valid Rust — the grep guard is
what keeps the rule a convention. The signature forces *a* `&dyn Scrubber`, not *the* one the run
holds, so swapping a site for a locally built `MinimalScrubber::new([])` compiles, passes every
behavioural case, and silently disables the masking half the moment the run engine's secret list
is populated; a second guard pins the three sites to `self.parts.scrubber`. And a `HashMap` in the
record graph is the one thing that could make `serde_json::to_value` fail and so turn a record
into `Value::Null`, so a third bans the hashing maps across the whole graph —
`the_prompt_module_reads_no_clock` covers `prompt/`'s nine files, but the graph reaches into
`model/skill.rs` and `model/ids.rs` and that guard was narrower than the claim it was cited for.

Two of those three guards failed on their first run, both against themselves. The serialiser guard
flagged `payload_sections_value` at `prompt/mod.rs:345` — a `Vec<SectionEntry>`, not a record —
and a doc comment in `probe.rs` naming `serde_json::to_value` in prose, because the rule matched a
bare `self`. The map guard flagged `trim.rs` for a `BTreeMap` that appears only in prose
explaining that `serde_json::Map` is one; a `BTreeMap<String, _>` is harmless and only the hashing
maps are a hazard. Both rules were narrowed against the measurement rather than argued for.

## A note on how this ran

Two earlier commits on the branch (`8952d21`, and a draft status block written during
implementation) recorded a maintainer CONFIRM whose timing and wording do not agree with each
other, and three accounts of it were written into the plan before it was settled. The plan's status
block now carries the confirmation and the substantive fact-check result, and **leaves the account
of which turn said what to the maintainer** — which is the right call and is recorded here rather
than argued. Two things are not in doubt and are worth a reader's attention: a CONFIRM was
recorded on this branch before the plan had been amended, and the amendments above (roots and
provider ids) are part of what was confirmed, not a change made after approval. The falsified
premises are kept in the plan's claims table and in D5 rather than deleted, because a premise
recorded as wrong is more useful than one removed.
