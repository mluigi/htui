# Blueprint: MOD-32 — the scrubbed `trim_record` write path

**Architect: code-architect. Read-only except this file. No cargo command was run; the compile
facts below are read off the tree, not observed from a build.**

**State.** Branch `mod-32` at **`8952d21`** ("docs(mod-32): the plan is CONFIRMED, as written"), on
top of `f49454e` and `8cc3fda` (MOD-58). The working tree is clean apart from this blueprint.
Every path below is absolute under `/media/projects/htui-mod-32`.

**Gortex.** The daemon's only indexed checkout is `/home/mluigi/projects/htui` — a different tree,
a different branch, and the primary checkout this task forbids reading. Graph queries would have
served the wrong code and a graph edit would have landed in the wrong checkout, so every fact here
comes from direct reads of the worktree. (The Gortex `PreToolUse` hook fires on most shell greps
in this session; the worktree path is the binding instruction.)

---

## Corrections to the plan

Six, and one of them changes what T0 must do. None invalidates the design; C1 and C2 are the two
that would have cost an implementer time.

### C1 — there are **six** `TrimRecord` serialisations, not five, and one of them bypasses `to_value`

- **Plan says** (D2): "`to_value` becomes the only way to serialise a `TrimRecord`, so there is no
  unscrubbed serialiser left to reach for." Verified claim 3 backs it with a list of **five** call
  sites of `to_value` and asserts "the engine re-serialises the struct instead".
- **Reality**: `crates/htui-core/tests/prompt_digest.rs:944`, inside
  `reserve_target_is_integer_arithmetic`, is
  `serde_json::to_value(&record).expect("plain data")["budget_source"]` where `record` is a
  `TrimRecord` (`:937`, `let record = ok(&spec).trim;`). That is a sixth serialisation of a record
  and it goes **around** `to_value` entirely. `to_value` is not reached by it, and after T0 it
  still would not be.
- **Why it matters**: it is a test, so it leaks nothing and persists nothing — but it is the exact
  bypass shape D2 claims to have removed, sitting in the file T1 is told to add its pins to. A
  reviewer who runs `rg 'to_value'` and `rg 'serde_json::to_value'` and reconciles the two will
  find this and ask what the plan missed. Better to have it in the diff.
- **Fix**: convert it. One line, in T0:
  `record.to_value(&scrubber()).expect("plain data")["budget_source"]`.
  `scrubber()` already exists at `prompt_digest.rs:20-22`. The assertion is unchanged — an
  empty-rule `MinimalScrubber` masks nothing (`scrub.rs:120-122`) and the fixture's
  `budget_source` is `"project"`, which trips no `PREFIX_RULES` entry.
- **After the fix** the exhaustive list is eight: three production (`engine.rs:3144`, `:3726`,
  `:4547` — all replaced, not called) and five `to_value` call sites
  (`fixtures.rs:2489`, `prompt_skills.rs:128`, `:212`, `prompt_digest.rs:963`, `:964`).
  `to_value` is then the only serialiser, and D2's claim is true as written.

### C2 — "keep the existing comment at `3142-3143`" is wrong: the comment describes a failure the
### new `to_value` still has

- **Plan says** (T0, engine bullet 3): keep the comment "`unwrap_or(Value::Null)` here wrote a
  **null** `trim_record`…"; it is still true of `to_value`, "and it is the reason the new method
  does not swallow an encode failure either."
- **Reality**: that last clause is false, and so is "keep". The comment (`engine.rs:3141-3143`,
  not `:3142-3143` — see C7) says that a null `trim_record` is written **and nothing says so**,
  which is "the one thing `run_step.trim_record` exists to rule out". The plan keeps
  `unwrap_or(Value::Null)` inside the new `to_value` — so the new `to_value` does swallow an
  encode failure, into a `Null`, silently, and there is no longer even a
  `RecordError::Encode` at the call site to notice. The comment, kept verbatim, would sit directly
  above a call that can still do the very thing it condemns.
- **The error channel is fixed by D3.** `to_value` returns `Result<Value, Unmasked>`; the only
  error is `Unmasked`. There is no variant for an encode failure, and D3 forbids adding one. So
  `unwrap_or(Value::Null)` is forced by the chosen signature, not chosen.
- **Fix**: keep the `unwrap_or`, and **amend** the comment to carry the forward guarantee as well
  as the history. Concretely, after the existing three lines add one sentence naming why the
  `Null` is not reachable for this type (see H-2 for the argument, which is three sentences and
  belongs in the file). A reader must not have to take the invariant on trust.
- **This is the one place where the plan's T0 instruction, followed literally, produces a comment
  that is false about the code beneath it.**

### C3 — verified claim 16 is true, but its evidence covers only the second of two `From` hops

- **Plan says** (verified claim 16): "All three engine functions already convert `RecordError`
  into their own error" — evidence: "each site already ends in `RecordError::Encode(…)` and `?`
  (`engine.rs:3144-3148`, `:3726-3729`, `:4547-4550`)".
- **Reality**: that evidence proves `RecordError → EngineError`. The hop that matters for a
  `Result<Value, Unmasked>` is the *first* one, `Unmasked → RecordError`, and it is a **different
  file**: `crates/htui-agent/src/record.rs:247-251`, with `Unmasked` being
  `htui_core::scrub::Unmasked` per the import at `record.rs:105`. The second hop is
  `crates/htui-orch/src/command.rs:490`, `EngineError::Record(#[from] htui_agent::RecordError)`,
  inside `enum EngineError` (`command.rs:310`).
- **Verdict: the claim holds.** Both hops exist, the types are the same `Unmasked` in both, and all
  three functions return `EngineError`. Section "The `?` chain" below spells out the proof so the
  implementer checks both halves. Recorded because an implementer who verified only `record.rs:251`
  — which is all the plan's evidence chain suggests — has checked half of it.

### C4 — "TDD applies: each case is written before the line it pins" is not achievable here

- **Plan says** (Test plan, first line): TDD applies; each case before the line it pins.
- **Reality**: for a signature change the cases cannot compile against the old signature, and the
  plan's own Independence paragraph insists the tree builds at every commit. The pins therefore
  land *after* the line they pin, necessarily.
- **Not a change of method**: the plan's real proof is the mutation check, and the acceptance
  list already asks for the three mutations' observed failure lines. That is the discipline this
  item can actually hold, and it is stronger than a red-green cycle here because M-4 (drop the
  `?`) *compiles* and would survive a red-green cycle that only watched for a red.
- **Fix**: strike the sentence or reword it as "the mutation checks are the proof; write them and
  run them". Do not let an implementer stage a half-built commit to honour it.

### C5 — `MinimalScrubber::new([])` is not the inert stand-in T0 makes it sound like

- **Plan says** (T0, fixtures bullet): pass `&MinimalScrubber::new([])` "and unwrap or `expect` the
  `Result`". The phrasing reads as a formality that makes the call site behaviourally identical to
  today's.
- **Reality**: an empty secret list makes `mask` a no-op (`scrub.rs:120-122`) but does **not**
  disable the second pass. `scrub` is `mask_value` then `find_residue` (`scrub.rs:227-232`), and
  `find_residue` runs the eight `PREFIX_RULES` (`scrub.rs:22-30`) plus the PEM marker. So
  `MinimalScrubber::new([])` still **refuses** a credential-shaped string. Every call site that
  takes it is arming a fail-closed check, and every `.expect()` added at those five sites is a real
  assertion (H-3, H-10).
- **Why it matters for the plan's own risk R-1**: the plan argues the new pass "cannot refuse
  anything the assembler would not already have refused for the same string". That argument holds
  for the *rendered* bytes, which pass `scrub_text` (`mod.rs:1098-1102`). It does **not** extend to
  `template.name` and `spec.notes`, which the plan itself names as the newly covered fields — and
  in the tests, it now also applies to the record's own strings. H-13 is where that lands.

### C6 — D5's rationale for keeping the excerpt-note convention is the wrong one, and an
### implementer writing the comment from it will write a false sentence

- **Plan says** (D5): the convention stays "because it is a *stricter, cheaper* guarantee for that
  one producer than the record-wide pass".
- **Reality**: it is no longer stricter. The stated ground was "`trim_record.notes` is persisted
  unscrubbed" (`mod.rs:895-896`), and that ground dies with this item. What the convention
  actually buys now is a **clearer sentence**: with the record-wide pass, a masked repo slug in a
  note serialises as ``a file in repo `[REDACTED]` dropped; the scrubber refused its repo slug``,
  which asserts a fact about a value the reader cannot resolve. The convention's output — "a file
  dropped; the scrubber refused it" — is the informative one. It is also the only statement the
  current doc makes about *why* only some notes name a repo, so the enumeration above it stays.
- **Fix**: write the corrected sentence on the *informational* ground plus the honest note that the
  leak the rule was guarding against is now impossible anyway. Do not leave "stricter" in the
  comment. The exact replacement text is in T0, step 6.

### C7 — the comment is at `engine.rs:3141-3143`, not `:3142-3143`

Cosmetic, but the plan cites it in a task step an implementer will go to with `sed`. The block is
three lines: `:3141`, `:3142`, `:3143`, immediately above `let trim =` at `:3144`. (The plan also
gives the block as `:3144-3147` and `:3144-3151`; the whole construct is `:3144-3150`, with
`:3147-3149` being the `self.parts.store.set_step_prompt(…)` chain.)

---

## The `?` chain: proved, site by site

This is the assertion the plan makes as verified claim 16 and it is **true**. The full chain, in
the order `?` walks it:

```
TrimRecord::to_value(&dyn Scrubber) -> Result<Value, Unmasked>        // trim.rs:216 (new)
        │  Err(Unmasked),  Unmasked = htui_core::scrub::Unmasked      // scrub.rs:60-71
        ▼  hop 1, #[from]
RecordError::Unmasked(#[from] Unmasked)                               // record.rs:247-251
                                                                            (import :105)
        │  Err(RecordError)
        ▼  hop 2, #[from]
EngineError::Record(#[from] htui_agent::RecordError)                  // command.rs:490
                                                                            (enum :310)
        ▼
the enclosing function's Result<_, EngineError>
```

Hop 1 is proved by `record.rs:247-251` reading

```rust
    /// Something credential-shaped survived scrubbing. Reported once, from
    /// [`Recorder::finish`]; the offending row was already dropped and replaced by a
    /// `scrub_residue` error row when it happened.
    #[error(transparent)]
    Unmasked(#[from] Unmasked),
```

with `use htui_core::scrub::{Scrubber, Unmasked};` at `record.rs:105` — so the `Unmasked` in the
variant is exactly the type `Scrubber::scrub` returns (`scrub.rs:58`) and exactly the type
`to_value` will return. `#[from]` on a `thiserror` variant generates
`impl From<Unmasked> for RecordError`.

Hop 2 is proved by `command.rs:490` and the enclosing `Result<_, EngineError>` of each function.

**Site 1 — `walk_live_step`, `engine.rs:3141-3150`.**
Signature at `engine.rs:3093-3099`:
`async fn walk_live_step(&self, run: &Run, snapshot: &GraphSnapshot, step: &RunStep, phase: &SnapshotPhase, started_at: DateTime<Utc>) -> Result<Option<Rest>, EngineError>`.
`Result<Option<Rest>, EngineError>` ✓. The existing `.map_err(|err| htui_agent::RecordError::Encode(…))?`
at `:3144-3146` is itself a live proof of hop 2 at this exact statement. **No `map_err` needed.**

**Site 2 — `candidate_live`, `engine.rs:3726-3732`.**
Signature at `engine.rs:3671-3676`:
`async fn candidate_live(&self, stage: &CandidateStage<'_>, trees: &mut Option<Vec<RunStepTree>>, captured: &mut bool) -> Result<(), EngineError>`.
`Result<(), EngineError>` ✓, and `:3726-3728` is the same `map_err`/`?` proving hop 2. **No
`map_err` needed.**

**Site 3 — `judge_sessions`, `engine.rs:4547-4553`.**
Signature at `engine.rs:4537-4544`:
`async fn judge_sessions(&self, run: &Run, phase: &SnapshotPhase, attempt: i32, judge: &RunStep, candidate: &SnapshotCandidate, prompts: &JudgePrompts) -> Result<Result<[Document; 2], JudgeFailure>, EngineError>`.
The outer `Result` is `EngineError`; the inner `JudgeFailure` is untouched, because the `?` fires
before the function ever returns the inner value. `:4547-4549` proves hop 2. **No `map_err` needed.**

**All three convert. The plan's assertion holds.** If any of them had not, the conversion needed
would have been `htui_agent::RecordError::Unmasked(err)` and nothing else — but none does.

**The scrubber is in scope at all three, with no plumbing.** `Parts::scrubber` is
`pub scrubber: &'a dyn Scrubber` (`engine.rs:415`), and the file already passes it to a
`&dyn Scrubber` parameter in three places — `:1326` (`assemble(&spec, self.parts.scrubber)`),
`:4422`-`:4423` (the judge's two assemblies), `:4920` — so the coercion from `&'a dyn Scrubber` to
`&dyn Scrubber` is already a proven pattern in this file, not something to discover.

**No new import in `engine.rs`.** `to_value` is an inherent method, so it needs no `use`.
`serde_json` stays used elsewhere in the file, so the removed `serde_json::to_value` call leaves no
unused import. Do **not** add a `use htui_agent::RecordError` while you are in there: the three
sites spell the path in full today, the new one-liner needs no name, and an added-and-unused
`use` is a warning that `-D warnings` turns into a build failure. (There is a real precedent
against it: `#[workspace.lints.rust]` sets `unused_qualifications = "warn"`, `Cargo.toml:120-123`.)

---

## Files to change

| File | Change | Task |
|---|---|---|
| `crates/htui-core/src/prompt/trim.rs` | `use` line; `to_value` signature, doc, body; T1's in-file case | T0, T1 |
| `crates/htui-orch/src/engine.rs` | three sites; the `3141-3143` comment | T0 |
| `crates/htui-core/src/fixtures.rs` | `:2489` | T0 |
| `crates/htui-core/tests/prompt_skills.rs` | `:128`, `:212` | T0 |
| `crates/htui-core/tests/prompt_digest.rs` | `:944` (C1), `:963`, `:964`; T1's three cases | T0, T1 |
| `crates/htui-core/src/prompt/mod.rs` | `:895-896` (C2's sibling, D5 + C6) | T0 |

## The exact edits

### `crates/htui-core/src/prompt/trim.rs`

**The import.** The crate-use block runs `:35-41`, external first (`:32-33`). Append after `:41`:

```rust
use crate::scrub::{Scrubber, Unmasked};
```

`prompt/mod.rs` already imports `Scrubber` the same way, so this is the path the crate uses.
Nothing here trips `the_prompt_module_reads_no_clock` (`prompt_digest.rs:109-140`), which greps
`trim.rs`'s shipped source for `Utc::now`, `Local::now`, `SystemTime`, `Instant::now` and, in the
shipped region, `use chrono` / `std::env` / `HashMap` / `HashSet` — H-14.

**The method, `:213-218`**, replacing the whole `impl TrimRecord` head:

```rust
impl TrimRecord {
    /// The record as JSON, for `run_step.trim_record` — **the only serialisation of a record, and
    /// the only one that may be persisted.**
    ///
    /// The `scrub` pass is whole-record and enumerates nothing: one call over the serialised
    /// [`Value`], reaching every string leaf **and every object key** (`scrub.rs:141-163`). That is
    /// what keeps the rule true when the next field is added — the defect this item fixes is a
    /// guarantee that was an enumeration, and an enumeration goes stale the day someone forgets
    /// to extend it. It is the same pass the assembler applies to the rendered bytes one layer up
    /// (`prompt/mod.rs:472-483`), and it is the same call the recorder makes on every payload
    /// (`record.rs`): scrub, then persist, or refuse and persist nothing.
    ///
    /// Masking is idempotent (`scrub.rs:52`, the `[REDACTED]` step-over at `:124-129`), so a
    /// caller that scrubs twice records the same bytes both times and an already-masked record is
    /// unchanged.
    ///
    /// # Errors
    ///
    /// [`Unmasked`] when a string leaf still matches a credential rule after masking. `R-SEC-3`
    /// is fail-closed, so the caller must **not** persist the value at all: in the engine that
    /// fails the step before any session starts and before a token is spent. The error names a
    /// rule and a JSON pointer and never the offending text — its `Display` and `Debug` are part
    /// of the security contract (`scrub.rs:60-71`), and both are persisted in an `error` event.
    ///
    /// An **encode** failure is deliberately not an error here: it becomes `Value::Null`, which
    /// scrubs clean because it has no string leaf. See the note at the engine's call site for why
    /// that is unreachable for this type.
    pub fn to_value(&self, scrubber: &dyn Scrubber) -> Result<Value, Unmasked> {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        scrubber.scrub(&mut value)?;
        Ok(value)
    }
```

**`#[must_use]` comes off.** The current attribute (`:215`) guards a return type that is no longer
`#[must_use]`. `Result` already is, so keeping it is redundant; the plan's snippet drops it and
the file reads better without. It cannot fail the build either way: `clippy::double_must_use` is in
`pedantic`, and `Cargo.toml:132-134` enables `clippy::all` at warn with
"`clippy::pedantic` is deliberately NOT enabled" spelled out in the file.

`Value` is still needed (`trim.rs:33`) — the signature and the `unwrap_or` both name it.

### `crates/htui-orch/src/engine.rs`

Site 1, `:3141-3150` — the comment amended (C2) and the three-line `map_err` block replaced by one
line. Final text of the block:

```rust
        // `unwrap_or(Value::Null)` here once wrote a **null** `trim_record` and said nothing: the
        // row that records which sections were dropped and why would silently become "there was
        // no record", which is the one thing `run_step.trim_record` exists to rule out. That is
        // inside `to_value` now, and it is unreachable for this record: every key is a field name
        // or a `&'static str`, no `Serialize` in the graph returns `Err`, and `serde_json` writes
        // a non-finite float as `null` rather than failing — so `reserve` cannot fail either. A
        // `Null` that did appear would scrub clean (no string leaf) and would be the one thing
        // this row must never hold, which is why it is argued here rather than left implied.
        let trim = prompt.trim.to_value(self.parts.scrubber)?;
        self.parts
            .store
            .set_step_prompt(step.id, &prompt.digest, &trim)
            .await?;
```

Site 2, `:3726-3732`:

```rust
        let trim = prompt.trim.to_value(self.parts.scrubber)?;
```

Site 3, `:4547-4553`:

```rust
        let trim = prompts.forward.trim.to_value(self.parts.scrubber)?;
```

Sites 2 and 3 carry no comment today and get none; the argument lives at site 1, which is the
first of the three a reader meets.

### `crates/htui-core/src/fixtures.rs` (`:2489`)

Inside the `#[cfg(feature = "test-support")]` `#[test] fn step_impl_carries_the_golden_trim_record`
(`:2477-2491`). The file's existing style is the fully-qualified path, and it has no
`use crate::scrub` — so use the full path rather than adding an import to a `demo`-feature file
that only needs it in one test:

```rust
        assert_eq!(
            step.trim_record.as_ref(),
            Some(
                &crate::prompt::fixtures::demo_trim_record()
                    .to_value(&crate::scrub::MinimalScrubber::new([]))
                    .expect("the demo record is plain data and carries nothing credential-shaped")
            ),
            "the fixture literal and the assembler's own record must not drift"
        );
```

`Some(&Value)` against `Option<&Value>` still types: `.expect` yields a `Value`, `&` borrows it,
`Some(..)` is `Option<&Value>`, and the borrow lives to the end of the statement.

### `crates/htui-core/tests/prompt_skills.rs` (`:128`, `:212`)

The file already has `use htui_core::scrub::MinimalScrubber;` (`:12`) and
`fn scrubber() -> MinimalScrubber { MinimalScrubber::new([]) }` (`:15-17`). No import change.

`:128`, in `a_missing_version_records_missing_version`:

```rust
        prompt.trim.to_value(&scrubber()).expect("plain data")["skill_choices"][1]["version"],
```

`:212`, in the masking case at `:198-213`:

```rust
    let serialised = prompt.trim.to_value(&scrubber()).expect("plain data").to_string();
```

**Use `scrubber()`, not the `MinimalScrubber::new(["s3cr3t"])` the prompt was assembled with.** H-4
is the whole reason, and it is the one place in T0 where the obvious choice and the right choice
differ. Note the indexing at `:128` is a `Value` index chain, so `.expect` must come before `["…"]`
— not after.

### `crates/htui-core/tests/prompt_digest.rs`

`:20-22` already provides `fn scrubber()`. No import change.

`:963-964`, in `to_value_is_byte_stable_and_carries_the_documented_keys`:

```rust
    let first = ok(&fixtures::phase_implement_attempt2())
        .trim
        .to_value(&scrubber())
        .expect("plain data");
    let second = ok(&fixtures::phase_implement_attempt2())
        .trim
        .to_value(&scrubber())
        .expect("plain data");
```

Both the **same** scrubber, and the case's doc comment (`:950-961`, the F-35 key-order note) should
gain one sentence: the assertion is now about the **scrubbed** serialisation, and the key set
checked at `:967-990` is read off the scrubbed object. H-5.

`:944`, in `reserve_target_is_integer_arithmetic` — C1:

```rust
        record.to_value(&scrubber()).expect("plain data")["budget_source"],
```

### `crates/htui-core/src/prompt/mod.rs` (`:895-896`)

Replace the sentence *"Because `trim_record.notes` is persisted unscrubbed, a note names a repo or
path only when the scrubber returns it unchanged — neither refused nor masked (a known secret
inside a file name is masked, and naming it would leak it)."* with (C6):

> `trim_record.notes` is now scrubbed whole before the write (`TrimRecord::to_value`, MOD-32), so a
> masked value can no longer leak through a note. The rule below is kept anyway, on the ground that
> survives: it is the **clearer** sentence, not the safer one. A note that named a masked repo
> would read ``a file in repo `[REDACTED]` dropped`` — an assertion about a value no reader can
> resolve — where the convention gives ``a file dropped; the scrubber refused it``. So a note here
> names a repo or path only when the scrubber returns it unchanged, neither refused nor masked.

Everything above and below that sentence — the five note shapes at `:897-907` and
`drop_unmaskable_excerpts`'s body at `:909+` — is unchanged. The convention stays (D5); only the
premise moves.

---

## Build order

Two tasks, serial, on the main thread. Neither is parallelisable, and the reason is different for
each.

**T0 — the change. One commit. `trim.rs`, `engine.rs`, `fixtures.rs`, `prompt_skills.rs`,
`prompt_digest.rs`, `prompt/mod.rs`.** The signature edit forces every call site to move in the same
commit that makes it; split across two commits, the intermediate tree does not build and there is
no useful bisect point in between. Nothing inside T0 depends on anything else in T0, so it is one
mechanical pass in one order: `trim.rs` first (everything else needs it to exist), then `engine.rs`
(the three sites, and the comment), then the three test/fixture files, then the `mod.rs` comment
last because it is the only edit whose content depends on what the other files now say.

**T1 — the pins. `prompt_digest.rs` (three cases) and `trim.rs` (one in-file case).** Not
parallelisable with T0: it touches both files T0 touches, on one worktree, one `.git/index` and one
`target/`. It is also strictly *after* T0 — T1's cases call `to_value(&dyn Scrubber)`, which does
not compile until T0's signature exists. C4 covers why the plan's "TDD" sentence cannot be honoured
literally and what stands in for it.

**Both must commit incrementally, staging only their own paths, and neither may stash or branch.**
An uncommitted subagent's work on a shared worktree dies with the session, and a stash on this
checkout is a lost afternoon (project memory: *implementer agents commit incrementally*; *workflow
worktree implementers* — a named branch first, no stash on a shared tree).

---

## Hazards

Each one names the concrete failure it prevents.

**H-1 — the four `skip_serializing_if` fields on `Section` rule out a `Value` round-trip.**
`trim.rs:115`, `:118`, `:121`, `:124` each carry `#[serde(skip_serializing_if = "Option::is_none")]`
with no matching `#[serde(default)]`, and `Section` derives `Serialize` **only** (`:102`).
*Prevents:* an implementer who finds the whole-record pass awkward reaching for the D8 alternative
— deserialise the `Value` back into a `TrimRecord` and scrub the struct. That needs a `Deserialize`
derive on the whole record graph, and it does not compile as written (`elided_lines`,
`elided_bytes`, `stubbed` and `dropped` are all missing on the way back), or compiles and silently
zeroes them. Either way `sections[]` loses the elision counts the row exists to carry. D8 named
these four fields for exactly this reason; do not let them be rediscovered.

**H-2 — `unwrap_or(Value::Null)` is a silent null write, and the comment above it must say so.**
`trim.rs:217` after the edit. *Prevents:* the C2 failure — the `engine.rs:3141-3143` comment kept
verbatim, asserting a property the code beneath it does not have. A `Null` scrubs clean
(`scrub.rs:156` and `:203` both treat `Value::Null` as a no-op), so the **scrub** is not what makes
it dangerous; the danger is precisely what the comment says — `run_step.trim_record` becomes "there
was no record", and `Null` says nothing. The mitigation is the amended comment, and the argument
that it is unreachable: every object key in the graph is a field name or a `&'static str`
(`trim.rs:185-210`, `SectionName` and `TemplateRole` are `rename_all` enums, `budget_source` is an
enum, `estimator` is `&'static str`), no derived `Serialize` in the graph can return `Err`, and
`serde_json` serialises a non-finite `f64` as `null` rather than failing, so `reserve: f64` (`:193`)
is not an error path either. The same argument is made, for a different type, at
`model/usage.rs:73-74`. Do not "improve" the `unwrap_or` into a `.expect` — that would be a new
error channel, which D3 forbids.

**H-3 — the three test call sites' `.expect()` is a tripwire; a panic there is a finding.**
`fixtures.rs:2489`, `prompt_skills.rs:128`, `prompt_skills.rs:212`.
*Prevents:* the reflex fix. The `fixtures.rs:2489` case compares the **demo golden literal**
`step.trim_record` against `demo_trim_record().to_value(…)`, and its failure message is "the fixture
literal and the assembler's own record must not drift" (`:2490`) — which is the wrong diagnosis
if what actually happened is that `find_residue` found a credential-shaped string in the demo
record's own `notes` / `template.name` / excerpt paths. That is **R-1 materialising**, and it is
exactly the new coverage the plan says is intended. Read the panic's `Unmasked` rule and pointer
before touching anything. The three ways to make it go away that must not be taken: swap in a
scrubber with rules, weaken `to_value`, or edit the golden literal.

**H-4 — `prompt_skills.rs:212` must use the file's empty-rule `scrubber()`, not the masking one in
scope.** The case assembles with `MinimalScrubber::new(["s3cr3t"])` at `:198` and asserts
`names == ["rust-style", "deploy-[REDACTED]"]` at `:206-209` — that is the assembler masking the
skill name — and then `!serialised.contains("s3cr3t")` at `:212`.
*Prevents:* passing the local masking scrubber to `to_value`. The assertion then passes **because
the serialiser masked it**, and the case silently stops being a test of the assembler and becomes a
tautology about `to_value`. With `scrubber()` it keeps its meaning: the record reached the store
already masked, and the serialiser is only being asked not to un-mask it. This is the one place in
T0 where the obvious choice and the right choice are different objects, both named `MinimalScrubber`
and both in scope.

**H-5 — `prompt_digest.rs:963-964` assemble two records and assert byte equality; the assertion has
narrowed.** The two `ok(&fixtures::phase_implement_attempt2())` calls are two separate assemblies of
the same spec, so the case pins determinism, not a single object. *Prevents:* two things. First,
giving the two calls **different** scrubbers — one with a rule, one without — which would make the
equality assert a scrubbed value against an unscrubbed one, and would fail for a reason that has
nothing to do with determinism. Second, leaving the case's own doc comment (`:950-961`, the F-35
key-order note) saying the serialisation is byte-stable without saying it is the **scrubbed**
serialisation: the `BTreeSet` of keys at `:967-990` is now read off the scrubbed object, and if a
scrubber ever masks or refuses a *key* the key set is what changes first (H-8). Both calls get
`scrubber()`; the doc comment gets one sentence.

**H-6 — `prompt_digest.rs:944` is the sixth serialisation and must be converted.** Covered as C1.
*Prevents:* D2's claim being false on arrival. Listed again here because the hazard is a reviewer's
`rg`, not a compiler: nothing in the build fails if `:944` is left alone.

**H-7 — the `?` chain has two hops and the plan only cites the second.** `record.rs:251` is hop 1;
`command.rs:490` is hop 2. *Prevents:* an implementer who checks `record.rs:251`, sees `#[from]`,
and assumes the chain ends there — which would be right for a function returning
`Result<_, RecordError>` and wrong for all three of these, which return `Result<_, EngineError>`. The
proof is in "The `?` chain" above; both ends are already in the tree and neither needs writing. The
day one of these functions is wrapped in a new type that does **not** carry `RecordError`, hop 2 is
the one that disappears and the site needs an explicit conversion.

**H-8 — the scrub masks object keys as well as values, and a masked key collapses entries.**
`scrub.rs:147-149` rebuilds the map and says so in as many words: "two keys that mask to the same
string collapse into one entry, which drops a value but never keeps a secret." *Prevents:* a future
`TrimRecord` field keyed by something operator-authored (a per-repo map, say) silently losing a row
from the persisted record under a scrubber that holds a secret — a data-loss bug, not a leak, and
one that only appears once someone adds the field and hands in a non-empty scrubber. Today's
`TrimRecord` has no map-shaped field, and `the_prompt_module_reads_no_clock`
(`prompt_digest.rs:109-140`) forbids `HashMap` in `prompt/`'s shipped source, so it cannot arise
inside the module itself. It can arise from a type the module imports.

**H-9 — the excerpt-note convention is corrected, not deleted.** `mod.rs:895-896`, and the five
note shapes at `:897-907`. *Prevents:* reading "the premise was false" as "the rule was pointless".
D5 is right that the rule stays; C6 is right that D5's stated reason is not the one that survives.
An implementer who deletes `drop_unmaskable_excerpts`'s "name only when unchanged" test has removed
the only thing that keeps a note from saying ``a file in repo `[REDACTED]` dropped``, and has done
so under cover of a comment fix. The convention is **kept**, in full, with its enumeration.

**H-10 — an empty-rule `MinimalScrubber` is not inert; it is a fail-closed scanner.**
`MinimalScrubber::new([])` sets `self.secrets.is_empty()`, which short-circuits `mask`
(`scrub.rs:120-122`) and nothing else. `scrub` is `mask_value` **then** `find_residue`
(`scrub.rs:227-232`), and `find_residue` runs all eight `PREFIX_RULES` (`scrub.rs:22-30`) and the
`PEM_MARKER` check. *Prevents:* treating the five test call sites as mechanically equivalent to
today's zero-argument call. They are not: each is now a new assertion, and each `.expect()` is
armed. It also prevents the opposite error — concluding from "the test scrubber is empty" that no
engine behaviour can change (H-13).

**H-11 — eight call sites, exhaustively.** The production three (`engine.rs:3144`, `:3726`,
`:4547`) plus the five `to_value` call sites (`fixtures.rs:2489`, `prompt_skills.rs:128`, `:212`,
`prompt_digest.rs:963`, `:964`) plus `prompt_digest.rs:944` (C1) = nine edits, of which three
replace the call entirely. *Prevents:* the one that bites worst, because it is silent:
`fixtures.rs:2489` is `#[cfg(feature = "test-support")]` (`:2476`) and a bare `cargo test -p
htui-core` compiles it out entirely. Run the feature-enabled command. The full validation set is
`--all-features` everywhere, which is correct; this hazard is only for an implementer reaching for
a quick `cargo check` and believing the fixture is covered.

**H-12 — a failing engine test may be R-1, not this item.** Every `htui-orch` engine test builds
its `Parts` with `MinimalScrubber::new([])` — `engine.rs:5946`, `:5962`, `:5978`, `:5992`,
`:6360`, `:6447`, `:8010`, `:8219`, `:8861`, `:10064` — which by H-10 is empty masking with the
prefix rules live. The one engine test that reads the persisted record back is at
`engine.rs:6938-6960` (`"attempt 2's trim record carries its notes"`, asserting two generated
notes survive into `trim_record.notes`). Neither note is credential-shaped, so it will pass.
*Prevents:* treating a red engine test as a regression in the write path rather than the newly
armed residue scan. The scrubber's rule list is the first thing to check.

**H-13 — the `engine.rs` comment is the only place the `Null` argument lives.** It is stated at site
1 and nowhere else, and sites 2 and 3 make the same call. *Prevents:* a future reader of
`candidate_live` concluding, from the absence of a comment, that the `Null` is a real possibility
there. Site 1's comment is the file's own convention for exactly this (`engine.rs:3141` is the
third comment of that shape in the file's own history); one argument, three call sites.

**H-14 — `the_prompt_module_reads_no_clock` greps `trim.rs`'s shipped source.** `prompt_digest.rs:109-140`
takes `include_str!("../src/prompt/trim.rs")` (`:117`) and asserts, on the region before the first
`#[cfg(test)]`, that it names none of `Utc::now`, `Local::now`, `SystemTime`, `Instant::now`,
`use chrono`, `std::env`, `HashMap`, `HashSet`. *Prevents:* an implementer reaching for a
`HashMap<String, String>` while thinking about the "one map" rule at `trim.rs:18-21` and tripping
a test with no visible connection to this item. The new `use crate::scrub::{Scrubber, Unmasked};`
names none of them and is safe.

**H-15 — one worktree, one index, one `target/`.** *Prevents:* the ordinary fan-out failure on
this repo: two agents on the same worktree produce two commits whose halves are each other's
context, and a `cargo` run from one while the other holds the lock writes to a shared target dir.
T0 and T1 are not file-disjoint and are not to be run concurrently. This is the same hazard the
project already records under *parallel fan-out hidden file coupling*.

---

## Mutation checks

Four, not three. The plan has three; **M-4 is the one a red-green cycle cannot catch**, because it
compiles. Run all four against T1's cases, revert each, and report the **observed failure line** for
each — a mutation check that was not run and reported is indistinguishable from one that was run.

**M-1 — delete the `scrubber.scrub(&mut value)?;` line entirely.**
*Case 2* must fail at its `expect_err`: `to_value` returns `Ok`, the case panics on
`expected an unmasked refusal`. *Case 1* must fail at its "does not contain `alpha-secret`"
assertion, with `alpha-secret` present at `/notes/0`. This is the coarse mutation; it proves the
call is there, not that it is the right one.

**M-2 — replace the whole-record pass with a mask of `self.notes` only.** The shape this item is
fixing: an enumeration, named as a `Value` the record happens to have today.
*Case 3* must fail — `template.name` is never masked and never scanned, so `to_value` answers
`Ok` where the case demands `Err(Unmasked { rule: "anthropic_api_key", path: "/template/name" })`.
*Case 2* must still pass, because `notes` is in the enumeration. **This is the mutation that
distinguishes the fix from the defect**, and the one a reviewer will ask for by name. If case 2
also fails under M-2, the enumeration was written wrong.

**M-3 — make the pass residue-only: `let mut copy = value.clone(); let _ = scrubber.scrub(&mut copy);`.**
The pass runs and its result is thrown away, which is what a "we already scrubbed this elsewhere"
comment produces. *Case 1* must fail (the secret is still there). *Case 2* must fail (`Ok` where
`Err` is expected). Case 3 may pass, which is fine — M-3 is not the case it is aimed at.

**M-4 — keep the call and drop the `?`: `let _ = scrubber.scrub(&mut value);`.**
This is the highest-value mutation of the four and the plan does not list it. It **compiles**,
it masks correctly, and it discards the refusal — so M-1, M-2 and M-3 all miss it, and any
red-green discipline that only watches for a red misses it too. *Case 2* must fail (`Ok` where
`Err` is expected) and *case 3* must fail (`Ok` where `Err` is expected). A step whose template name
is credential-shaped would then be written to the store, and the item's whole guarantee would be
quietly off while every masking case stayed green.

**T1's in-file case (`trim.rs`) has no independent mutation.** It pins `scrub.rs:52`'s idempotence
at the record, where the double-scrub risk actually lives; the property itself is already pinned at
`scrub.rs:539-554` (`scrubbing_is_idempotent`), and `scrub.rs` is not touched by this item, so the
only available mutation would be in a file this item must not open. Recorded so its absence is
deliberate rather than forgotten. What it does buy: it is the case that makes D1 safe for a caller
that scrubs twice, which is the one caller shape the signature change makes newly possible.

**T0's own check** is the build: `cargo build --workspace --all-features` and
`cargo clippy --workspace --all-features --all-targets -- -D warnings`. The signature change is
compile-forced at all eight in-tree call sites (H-11) — that is D2's payoff, and it is why no
runtime case is listed for the engine's three sites in the coverage map. The one runtime coverage
is accidental and welcome: `engine.rs:6938-6960` reads a persisted `trim_record` back through the
new path (H-12).

---

## What must NOT change

Each entry is a place where a well-meaning implementer would find a "cleaner" option that this
item has already rejected by name.

- **`crates/htui-core/src/store/traits.rs`** — `set_step_prompt` keeps its signature (D6). The store
  has no scrubber, and giving it one is a `WriteStore` trait change across `mem.rs` and
  `pg/write.rs` plus the two test doubles: the R-4 shape the mod-58 plan declined. The input-layer
  precedent `f48b82b` put the scrub **above** the store, not in it, and this item is that same move
  one layer down.
- **`crates/htui-core/src/store/mem.rs`, `store/conformance.rs`, `crates/htui-store/src/writer.rs`,
  `crates/htui-store/src/pg/write.rs`** — the two implementations and the shared case list. Untouched,
  so store `CASES` does not move.
- **`crates/htui-store/tests/pg_criteria.rs:1076-1108`** — `set_step_prompt_writes_only_the_digest_and_the_record`
  writes a hand-built `json!({ "v": 1 })` (`:1101`, `:1108`) and asserts what the **column** holds.
  It is not the production path; changing it would test nothing this item fixes (D7). The honest
  limit, stated once: **after this item the store still accepts an unscrubbed `trim_record` from a
  caller that is not the engine.** No such production caller exists after T0 (the three sites are
  exhaustive), and the store is not, and will not become, the enforcement point.
- **`crates/htui-core/src/scrub.rs`** — the scrubber is *used*, not changed. `Scrubber` keeps the
  trait MOD-10 replaces behind it (`scrub.rs:47-49`), and MOD-10 inherits this fix for free by
  replacing `MinimalScrubber` (`:95`). No trait method is added (T0 condition 1).
- **`RecordError`'s variant set** — three variants, unchanged. `Unmasked(#[from] Unmasked)` is
  already at `record.rs:247-251`; D3 needs nothing new, and no `Encode` site in the engine is left
  unconstructed because `record.rs:1593` and `:783` still build it.
- **No `Deserialize` anywhere** on `TrimRecord`, `Section`, `SectionEntry`, `TemplateRecord`,
  `ExcerptAudit` or `SkillChoice` (D8, H-1). And no `#[serde(default)]` added to `Section`'s four
  `skip_serializing_if` fields — that pair is the round-trip's whole cost and the reason D8 rules
  it out.
- **No conformance case** is added, and no store count pin moves.
- **No migration**, no `cache_migrations/` entry, and **no `.sqlx` entry** — `.sqlx` stays at 268,
  `migrations/` stays `0001_init`..`0007_skill_attachments`, and `0008` stays the next free number.
  `git diff --stat 8cc3fda -- crates/htui-core/src/store crates/htui-store/src crates/htui-store/tests
  crates/htui-core/src/scrub.rs crates/htui-store/migrations crates/htui-store/cache_migrations
  crates/htui-store/.sqlx` must be empty. Note `fixtures.rs` and `prompt_digest.rs` are outside
  that list and *are* expected to move.
- **`crates/htui-core/src/prompt/mod.rs`'s scrub machinery** — `scrubbed_inputs` (`:726-871`),
  `scrub_text`, `surviving_audit` (`:1080-1106`), the input-then-render order (`:453-454`, then
  `:472-483`), and `drop_unmaskable_excerpts`' body. Only the premise sentence at `:895-896` moves.
- **The record's shape** — no field added or removed, `RECORD_VERSION` stays `2` (`trim.rs:57`),
  `SectionEntry` construction stays confined to `trim.rs` (grepped at `prompt_digest.rs:227-260`),
  and `section_entries()` keeps projecting the record. Only *how a record is serialised* changes.
- **`HANDOFF.md`, `DECISIONS.md`, `docs/**`** — main-thread bookkeeping (T0 condition 4). The
  implementer does not close MOD-32.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test -p htui-core --all-features -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
cargo doc --workspace --no-deps --keep-going            # exactly the six baseline errors
ls crates/htui-store/.sqlx | wc -l                      # 268, unchanged
```

`--test-threads=1` is not optional: the keyring fake is process-wide (project memory). Every
`htui-core` command must carry `--all-features` or `fixtures.rs:2476`'s
`#[cfg(feature = "test-support")]` compiles the golden-record case out and one of the eight call
sites goes unchecked (H-11). Before believing a Postgres failure: `df -h /`, then re-run the case
alone — the dev Postgres crash-loops under disk pressure and answers `SQLSTATE 57P03` while docker
still reports it healthy (project memory; plan R-7). `validate-workflow-docs.sh` exits 0 at
close-out, which is the main thread's step, not the implementer's.
