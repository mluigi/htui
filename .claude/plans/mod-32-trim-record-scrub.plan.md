# Plan: MOD-32 — the `trim_record` write path is not scrubbed

**Status: CONFIRMED by the maintainer 2026-09-28 at 12:46 CEST, after the fact-check amendments
recorded in `7c23865`.**

**On the two earlier status commits.** `8952d21` recorded a maintainer CONFIRM at 12:10. There was
no such turn: the maintainer accepted the **routing** at 12:07 and had not been shown this plan.
`7c23865` recorded that gap and voided the status, and amended the plan against a second
fact-check pass. The maintainer's confirmation at 12:46 was given against **that** text — the
amended plan — not against the version `8952d21` blessed. A draft of this block claiming a 12:10
confirmation, and attributing `7c23865` to a subagent, was written during implementation and is
withdrawn: it is not what happened.

What the second pass found is substantive and stands. It falsified part of claim 8 —
`surviving_audit` clones `spec.excerpts.audit` wholesale (`mod.rs:1084`) and rebuilds only
`files[]`, and `scrubbed_inputs` never walks `spec.excerpts.audit` at all, so
`excerpts.roots[].repo` and `excerpts.provider_set[]` reach the record unmasked. That is the part
of the HANDOFF item this plan's first fact-check wrongly wrote off, and it is corrected in place.
The pass also found three further sites stating the "persisted unscrubbed" premise, now folded into
D5. Every correction is comment- or claim-text only; **no production line changed as a result of
any of them.**

**Source**: `HANDOFF.md:239-250` (MOD-32, found at MOD-2 close-out, 2026-09-15; previously deferred
as F-80 and again at the MOD-7 milestone 4 review, MEDIUM).
Requirements `R-SEC-3`, `R-PRM-3`. Precedent: the milestone-9 CRITICAL `f48b82b` ("scrub the
assembler's inputs, not its renders").

**Complexity: small, one crate's public method plus three call sites.** No migration, no
`.sqlx` entry, no new dependency, no new error variant, no shared `WriteStore` trait method, no
conformance case, no change to either store.

**Routing**: routed as plan by `/handoff-run MOD-32` (1 of C1–C4 fired). Ultracode not needed.
Staffing: the session model for every step, no model override on any agent, per the maintainer's
instruction for this session.

**Numbering**: MOD-32's own plan. Decisions **D1…D8**, tasks **T0**, **T1**.

**Gortex note**: `graphify-out/` does not exist, and Gortex MCP indexes only the primary checkout —
at `68c058f` (main) when this plan was drafted. **That index is valid here**: the worktree is
`8cc3fda`, whose merge-base with `68c058f` *is* `68c058f`, and the only files MOD-58 changed are
`crates/htui-core/src/store/mem.rs` and `crates/htui-store/tests/pg_criteria.rs` — neither of which
this plan edits, and `pg_criteria.rs:1076` is unmoved. Every other fact-check pass was served from
the graph on that basis; the file-level facts carry a `file:line` either way.

---

## Summary

Every production write of `run_step.trim_record` is the same three lines, and none of them scrubs:

```rust
let trim = serde_json::to_value(&prompt.trim).map_err(|err| {
    htui_agent::RecordError::Encode(format!("the step's trim record: {err}"))
})?;
self.parts.store.set_step_prompt(step.id, &prompt.digest, &trim).await?;
```

(`engine.rs:3144-3151`, `:3726-3731`, `:4547-4553` — `walk_live_step`, `candidate_live`,
`judge_sessions`. These three are the **only** production `set_step_prompt` call sites in the
workspace; every other one is a test or a trait impl.)

**The finding that sets the shape of the fix**: `TrimRecord::to_value()` (`trim.rs:216`) — the
method that exists to produce exactly this `Value` — is called from **tests and fixtures only**
(`fixtures.rs:2489`, `prompt_skills.rs:128,212`, `prompt_digest.rs:963-964`). The engine does not
call it; it re-serialises the struct with `serde_json::to_value` directly. So the record that
reaches the store never passes through the one function on its type that could have owned the
rule, and the record's own strings are governed by nothing at all.

**How much is actually unscrubbed.** Most of the record is masked already, because it is derived
from the spec that `scrubbed_inputs` masked: `sections[].name` (`SectionName`, an enum),
`skill_choices` (built at `mod.rs:861` from the masked `spec.skills`), and the `excerpts` audit's
`files[]` rows (rebuilt at `mod.rs:1080-1106` from the masked `spec.excerpts.files`, and their
rendered blocks pass `scrub_text`, which masks *and* refuses, at `mod.rs:1098-1102`).

**`excerpts.files[]` is covered; `excerpts.roots[]` and `provider_set[]` are not.**
`scrubbed_inputs` walks `spec.excerpts.files` and never `spec.excerpts.audit` — the audit's
`roots[].repo` (a repo slug) and `provider_set[]` (`name@version` provider ids) are cloned into the
record verbatim by `surviving_audit` (`mod.rs:1084`, `excerpt::file_record` at `excerpt.rs:935-947`
likewise clones `repo`/`path` from the already-masked file). The genuinely uncovered strings are
five:

| Field | Where it comes from | Masked today? |
|---|---|---|
| `trim_record.template.name` | `spec.template.name`, a `prompt_template` row name | **No.** `scrubbed_inputs` (`mod.rs:726-871`) has no `mask` call on `spec.template`. |
| `trim_record.notes[0..spec.notes.len()]` | `PromptSpec.notes`, caller free text (`mod.rs:120-122`, copied verbatim at `mod.rs:1110-1114`) | **No.** |
| `trim_record.notes[…excerpts]` | `ExcerptSet.notes`, composed by `drop_unmaskable_excerpts` | **No** — and the code says so. See D5. |
| `trim_record.excerpts.roots[].repo` | `RepoRoot.repo`, via `spec.excerpts.audit` (`excerpt.rs:275-283`) | **No.** `scrubbed_inputs` masks `excerpts.files` only. |
| `trim_record.excerpts.provider_set[]` | `ExcerptAudit.provider_set` (`excerpt.rs:318`) | **No.** Same gap. |

`budget_source` and `estimator` are closed spellings (`&'static str` and a `BudgetSource`), so
they are structural, not text.

The defect is therefore not "five fields are unscrubbed" but the structural one the item names:
**the guarantee is an enumeration, and a field nobody remembered is unmasked.** The fix is
field-agnostic for exactly that reason (D4).

---

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **The scrub runs at the write site, inside `TrimRecord::to_value`, which grows a `&dyn Scrubber` parameter and returns `Result<Value, Unmasked>`.** | The bytes that are persisted are the bytes that get scrubbed — no second representation and no window in which a masked value and a written value differ. The engine already holds the scrubber (`engine.rs:415` `pub scrubber: &'a dyn Scrubber`, used at `:1326`, `:4422`, `:4920`), so no plumbing is needed. |
| D2 | **The signature change, not a new method beside the old one.** `to_value` becomes the only way to serialise a `TrimRecord`, so there is no unscrubbed serialiser left to reach for. A future caller cannot bypass the rule by accident. | The alternative — keep `to_value()` and add `to_scrubbed_value()` — leaves today's bypass in the tree as the shorter name. `trim.rs:145-148` already states the house rule this item is an instance of: one construction path, nothing deriving the other way. |
| D3 | **The refusal surfaces as `RecordError::Unmasked` and fails the step, exactly as any other `set_step_prompt` failure does today.** | `record.rs:247-251` already carries `Unmasked(#[from] Unmasked)`, and the three engine functions already convert `RecordError` into their own error. **No new error variant.** The step fails after stage 3 and **before any session starts**, so nothing is persisted and no token is spent — which is the half of `R-SEC-3` that is about *persist*. It is not the graceful `refuse_prompt` block the assembler's own refusals take (D8). |
| D4 | **One whole-record pass, enumerating nothing.** The scrubber walks the serialised `Value` — every string leaf and every object key — in a single `scrub` call. | The three uncovered fields above are what the enumeration misses *today*. A field-agnostic pass is the only shape that stays true when the next `TrimRecord` field is added, and it is the same rule the assembler already applies to the rendered bytes (`mod.rs:472-483`). |
| D5 | **The excerpt-note convention stays; its rationale sentence is corrected in all three places the premise is written down.** | `drop_unmaskable_excerpts` deliberately names a repo or path only when the scrubber returned it unchanged, on the stated ground that "`trim_record.notes` is persisted unscrubbed" (`mod.rs:895-896`). That ground is now false, so the sentence is corrected — but the convention itself is a *stricter, cheaper* guarantee for that one producer than the record-wide pass, and is left in place. The fact-check found the same false premise in **two more places the first draft missed**: `mod.rs:966` ("`trim_record.notes` is persisted unscrubbed and the preview shows it") and `htui-agent/src/excerpt.rs:924`. A test comment at `htui-agent/tests/excerpt.rs:1291` states it a third time. All four are comment-only corrections. Recording a falsified premise rather than quietly deleting the code is the mod-58 precedent. |
| D6 | **No store change, on either side.** `set_step_prompt` keeps its signature; `traits.rs`, `mem.rs`, `writer.rs`, `pg/write.rs` and `conformance.rs` are untouched. | The store has no scrubber and giving it one is a shared trait change across both backends plus the two test doubles — the R-4 shape the mod-58 plan declined. The input-layer precedent `f48b82b` put the scrub above the store, not in it. |
| D7 | **The store-level case `pg_criteria.rs:1076` is deliberately left writing a hand-built record.** | It exercises `set_step_prompt` with a literal `json!({ "v": 1 })`, not the production path, and it asserts what the *column* holds. Changing it would test nothing this item fixes. **The honest limit, stated once and plainly: the store will still accept an unscrubbed `trim_record` from a caller that is not the engine.** After this item there is no such production caller (D1's site list is exhaustive), but the store is not, and will not become, the enforcement point. |
| D8 | **The residue refusal is a step error, not a `StageThree::Refused` block.** | Moving the check into `assemble` would buy the graceful path, but `assemble` returns a typed `TrimRecord`, so a scrub there would have to be either (a) a field-by-field enumeration — the defect again — or (b) a `Value` round-trip, which needs `Deserialize` on the whole record graph and collides with the four `skip_serializing_if` fields on `Section` (`trim.rs:115-125`) that will not round-trip without `#[serde(default)]`. Paying that for a nicer error message is the wrong trade for a fail-closed persist gate. If the maintainer wants the graceful path, that is a different plan and D8 is where it is argued. |

---

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/prompt/trim.rs` | edit | T0 | `to_value` signature, doc and in-file cases (D1, D2, D4) |
| `crates/htui-orch/src/engine.rs` | edit | T0 | the three `serde_json::to_value(&*.trim)` sites become `*.to_value(self.parts.scrubber)` (D1, D3) |
| `crates/htui-core/src/fixtures.rs` | edit | T0 | one call site, `:2489` |
| `crates/htui-core/tests/prompt_skills.rs` | edit | T0 | two call sites, `:128`, `:212` |
| `crates/htui-core/tests/prompt_digest.rs` | edit | T0, T1 | two call sites, `:963-964`; new cases in T1 |
| `crates/htui-core/src/prompt/mod.rs` | edit | T0 | the D5 comments at `:895-896` and `:966` |
| `crates/htui-agent/src/excerpt.rs` | comment only | T0 | the D5 doc sentence at `:924` |
| `crates/htui-agent/tests/excerpt.rs` | comment only | T0 | the D5 test comment at `:1291` |

**Not touched, on purpose:** `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`,
`crates/htui-core/src/store/conformance.rs`, `crates/htui-store/src/writer.rs`,
`crates/htui-store/src/pg/write.rs`, `crates/htui-store/tests/pg_criteria.rs` (D6, D7);
`crates/htui-core/src/scrub.rs` and `crates/htui-core/src/prompt/mod.rs`'s mask pass (D4 — the
scrubber is used, not changed, and `Scrubber` keeps the trait MOD-10 replaces behind);
every migration and `cache_migrations/`; `crates/htui-store/.sqlx/`; `HANDOFF.md`, `DECISIONS.md`,
`docs/**`.

---

## Tasks

**Independence.** T0 and T1 are **not** file-disjoint — both touch
`crates/htui-core/src/prompt/trim.rs` and `crates/htui-core/tests/prompt_digest.rs` — and a
signature change forces every call site to move in the same commit that makes it, or the tree
does not build. They also share one worktree, one `.git/index` and one `target/` directory. They
run **serially on the main thread**, T0 then T1.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T0 | `trim.rs`, `engine.rs`, `fixtures.rs`, `prompt_skills.rs`, `prompt_digest.rs`, `prompt/mod.rs`, `htui-agent/src/excerpt.rs`, `htui-agent/tests/excerpt.rs` | serial; run first |
| T1 | `prompt_digest.rs`, `trim.rs` | serial; run second |

**Conditions binding on both implementers:**
1. **Do not add a `Scrubber` trait method and do not add a `Deserialize` derive** (D4, D8). If the
   whole-record pass cannot be written as one `scrub` call over the serialised value, stop and
   report.
2. **No new error variant.** `RecordError::Unmasked` already exists (`record.rs:251`).
3. **No store, trait, migration or `.sqlx` change** (D6). If a store change seems needed, stop and
   report — that is a finding about the placement, not a licence to move it.
4. **No `HANDOFF.md` edit.** Bookkeeping is the main thread's (close-out).
5. **Commit incrementally**, staging only your own path. Do not stash. Do not push.

### Task 0: the scrubbed serialiser and its call sites (D1–D6)

- **`crates/htui-core/src/prompt/trim.rs`**
  - `to_value` becomes:
    ```rust
    pub fn to_value(&self, scrubber: &dyn Scrubber) -> Result<Value, Unmasked> {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        scrubber.scrub(&mut value)?;
        Ok(value)
    }
    ```
    The `unwrap_or(Value::Null)` stays as it is — a `Null` record has no string leaf, so it
    scrubs clean, and that is the pre-existing behaviour, not a new hole.
  - The doc gains: this is the **only** serialisation of a record and the only one that may be
    persisted; the pass is whole-record and enumerates nothing, so a field added later is covered
    without being remembered; masking is idempotent, so an already-masked record is unchanged;
    the error names a rule and a JSON pointer and never the text.
  - Add `use crate::scrub::{Scrubber, Unmasked};` (or the path the file's existing imports imply).
- **`crates/htui-orch/src/engine.rs`**, three sites, each replacing the `serde_json::to_value`
  block with one line:
  - `walk_live_step` (`:3144-3147`) →
    `let trim = prompt.trim.to_value(self.parts.scrubber)?;`
  - `candidate_live` (`:3726-3729`) → the same
  - `judge_sessions` (`:4547-4550`) →
    `let trim = prompts.forward.trim.to_value(self.parts.scrubber)?;`
  - **Keep the existing comment at `:3142-3143`** ("`unwrap_or(Value::Null)` here wrote a **null**
  `trim_record`…") — it is still true of `to_value`, and it is the reason the new method does not
    swallow an encode failure either.
  - Each site's `?` goes `Unmasked` → `RecordError` (`record.rs:251`) → the function's own error.
    No `map_err` is needed. If any of the three does not compile that way, stop and report rather
    than adding a conversion.
- **`crates/htui-core/src/fixtures.rs:2489`**, **`prompt_skills.rs:128` and `:212`**,
  **`prompt_digest.rs:963-964`**: pass `&MinimalScrubber::new([])` (the file's existing
  stand-in, `prompt/fixtures.rs:740` already spells it that way) and unwrap or `expect` the
  `Result`. **A fixture that starts failing here is a finding, not a nuisance** — see T1 case 3.
- **The four falsified premises, comment only (D5).** `crates/htui-core/src/prompt/mod.rs:895-896`
  and `:966`, `crates/htui-agent/src/excerpt.rs:924`, `crates/htui-agent/tests/excerpt.rs:1291`. Each
  sentence that says `trim_record.notes` is persisted unscrubbed becomes: the record is scrubbed
  whole before the write, and this producer's convention is kept because it is a stricter guarantee
  for that one producer than the record-wide pass — a note here never names a value the scrubber
  would have masked, so the note itself carries no trace of it. **Comment only: no behaviour in
  `htui-agent` changes in this item.**
- **Validate**: `cargo build --workspace --all-features`;
  `cargo test -p htui-core --all-features -- --test-threads=1`;
  `cargo clippy --workspace --all-features --all-targets -- -D warnings`;
  `cargo fmt --all -- --check`.

### Task 1: the pins (D4, D7)

- **File**: `crates/htui-core/tests/prompt_digest.rs`, where the assembler's own scrub cases
  already live (`:732`, `:1101-1106`).
- **Case 1 — `a_known_secret_in_a_caller_note_is_masked_in_the_record`.** Take an existing fixture
  spec, set `notes: vec!["token alpha-secret here".to_owned()]`, assemble with
  `MinimalScrubber::new(["alpha-secret"])`, and assert `prompt.trim.to_value(&scrubber)` contains
  `"[REDACTED]"` and does not contain `alpha-secret` at `/notes/0`. This is the masking half, and
  it is the arm a bare residue scan would fail — the case is what proves `scrub`, not a scan, is
  what runs.
- **Case 2 — `a_credential_shaped_string_in_the_record_refuses_the_serialisation`.** The same
  spec with a note of `"AKIAAAAAAAAAAAAAAAAA"`. `to_value` must be `Err`, with
  `rule == "aws_access_key_id"` and a pointer that names the note (`/notes/0`), and neither the
  `Display` nor the `Debug` of the error may contain the offending text — the `scrub.rs:345-361`
  contract, re-pinned at this call site.
- **Case 3 — `the_template_name_is_reached` (the falsification guard).** A spec whose
  `template.name` is `"sk-ant-api03-zzzzzzzz"`. Case 2 already proves the pass walks `notes`;
  this proves it walks a field the old enumeration never named, which is the actual defect (D4).
  If a future change reverts to a field list, this is the case that fails. **Extend it to a second
  spec whose `excerpts.audit.roots[0].repo` carries the same string** — that field is the one the
  first draft of this plan wrongly recorded as already masked, so it is the field most worth a
  pin.
- **In-file case in `trim.rs`**: `to_value` is idempotent under masking, and a record whose
  strings are already `[REDACTED]` serialises unchanged. This is what lets D1 be safe for a caller
  that scrubs twice.
- **Mutation check** (below). Report the observed failure line for each.

---

## Test plan

TDD applies: each case is written before the line it pins.

**Mutation check — the only honest proof that a pin bites.** After the cases pass, break each rule
on purpose, confirm the case fails, then revert:

1. **Case 2** — delete the `scrubber.scrub(&mut value)?;` line from `to_value`. Case 2 must fail
  (`Err` expected, `Ok` returned) and case 1 must fail with `alpha-secret` present.
2. **Case 3** — replace the whole-record `scrub` with a mask of `self.notes` only (the shape this
  item is fixing). Case 3 must fail and case 2 must still pass. **This is the mutation that
  distinguishes the fix from the defect**, and it is the one a reviewer will ask for.
3. **Case 1** — make the pass residue-only by dropping the mask (e.g. scrub a clone and discard
  it). Case 1 must fail.

**Coverage map.**

| Property | Case |
|---|---|
| A known secret in a caller note is masked, not refused | T1 case 1 |
| Credential-shaped text in the record refuses the write, with a pointer and no text | T1 case 2 |
| The pass is field-agnostic — `template.name`, never enumerated before | T1 case 3 |
| Masking twice changes nothing | T1 in-file `trim.rs` case |
| The engine's three sites pass the scrubber | compile-forced by the signature (D2); no runtime case |

**Count pins that move:** none. Store `CASES` is unchanged (D6, D7); `.sqlx` stays 268; migrations
stay `0001`..`0007` and `0008` stays the next free number; no snapshot moves; no `StoreRequest` /
`StoreReply` variant is added; `RecordError` keeps its three variants (D3).

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-1** — The scrub introduces a **new class of refusal**: a `template.name`, a caller note, or a repo slug / provider id that merely *looks* like a credential now fails a step that used to run | Medium | Bounded and stated rather than hidden. Every string the record already carried through the prompt path was masked *and* refused there (`mod.rs:1098`), so the new pass cannot refuse anything the assembler would not already have refused for the same string. The genuinely new coverage is `template.name`, `spec.notes`, `excerpts.roots[].repo` and `excerpts.provider_set[]` — operator- and provider-authored, short, and never digested. `roots[].repo` is the widest of the four: a repo slug is operator-chosen, so a repo literally named after a credential prefix is conceivable. Fail-closed is `R-SEC-3`'s own direction; a step that refuses is the intended outcome, not a regression. |
| **R-2** — The store still accepts an unscrubbed `trim_record` from any non-engine caller (D7) | Certain, by design | Stated in D7 and in the acceptance list rather than left for a reader to discover. No such production caller exists after T0; the store-level case at `pg_criteria.rs:1076` is the one that exercises it deliberately. |
| **R-3** — A future `TrimRecord` field carries free text and someone reintroduces an enumeration "for performance" | Low | D4 plus T1 case 3. The mutation in the test plan *is* the enumeration, and it is written down as the shape that must fail. |
| **R-4** — A reviewer reads the residue refusal as a step crash and asks for the assembler's graceful `refuse_prompt` path | Medium | D8 states the trade in full, including what the graceful path would cost. This is the one design call most worth the maintainer's override at CONFIRM. |
| **R-5** — The `skip_serializing_if` fields on `Section` (`trim.rs:115-125`) make a `Value` round-trip lossy, tempting a future implementer to "just deserialize it back" | Low | D8 rules the round-trip out and names the four fields. A `Deserialize` derive would need `#[serde(default)]` on each. |
| **R-6** — `to_value` is a public method on a public type in `htui-core`; the signature change is breaking for any out-of-tree caller | Low | The workspace is the only caller set (D2's list is exhaustive). Stated here because it is the criterion-C2 boundary the routing verdict turned on. |
| **R-7** — A `pg_criteria` or workspace failure is read as real when the dev Postgres is in recovery (`SQLSTATE 57P03`) | High on this box | `df -h /` first, then re-run the case alone (project memory). |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test -p htui-core --all-features -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
cargo doc --workspace --no-deps --keep-going            # exactly the six baseline errors
ls crates/htui-store/.sqlx | wc -l                      # 268, unchanged
git diff --stat 8cc3fda -- crates/htui-core/src/store crates/htui-store/src \
  crates/htui-store/tests crates/htui-core/src/scrub.rs crates/htui-store/migrations \
  crates/htui-store/cache_migrations crates/htui-store/.sqlx              # empty
```

`--test-threads=1` is not optional (the keyring fake is process-wide). Postgres cases get the
`USERNAME=htui-ci` prefix. Before believing a Postgres failure, `df -h /`, then re-run the case
alone.

## Acceptance

- [ ] `TrimRecord::to_value` takes a `&dyn Scrubber`, returns `Result<Value, Unmasked>`, and is the
      only way a record is serialised (D2).
- [ ] The pass enumerates no field: a secret in `template.name` and a secret in a caller note are
      both masked, and a credential-shaped one in either refuses the serialisation with a pointer
      and no text (D4, T1 cases 1–3).
- [ ] All three production `set_step_prompt` sites go through the scrubbed serialiser, and the
      site list is exhaustive (D1, D7).
- [ ] The three mutation checks fail as described, and the failures are reported.
- [ ] No store, trait, error variant, conformance case, migration, `.sqlx` entry, dependency or
      count pin moved.
- [ ] The premise in `mod.rs:895-896` is corrected and the convention it justifies is kept (D5).
- [ ] `validate-workflow-docs.sh` exits 0 at close-out.

## Where the HANDOFF or the tree disagree

- **`HANDOFF.md:246-248` — "The exposure is small by construction — the strings are repo-relative
  paths and enum spellings."** Partly **falsified, and the correction is the plan.** Most of the
  record is indeed derived from already-masked spec data, and `excerpts.files[]`' paths do pass
  `scrub_text` at `mod.rs:1098-1102` — the item names `excerpts` as uncovered and half of it is
  covered. But `excerpts.roots[]` and `provider_set[]` are **not**: `scrubbed_inputs` never
  touches `spec.excerpts.audit`, so the root strings the item names *are* the part that leaks.
  Two further fields are not what the item describes at all: `PromptSpec.notes` is **caller free
  text** (`mod.rs:120-122`), not a repo-relative path, and `template.name` is a row name the item
  does not mention. The exposure is still small; the enumeration is what is wrong, which is why D4
  is field-agnostic.
- **`HANDOFF.md:247-248` — "Decide between scrubbing `TrimRecord::to_value()`'s output before the
  write and refusing the write on residue, the way the assembler refuses."** **The two are not
  alternatives.** `Scrubber::scrub` masks in place *and then* returns `Err(Unmasked)` on residue
  (`scrub.rs:50`, `:228-233`); the assembler already does both, in that order
  (`mod.rs:453-454`, then `:477-483`). The real decision this item poses is **where** the call
  goes, and D1 settles it. The item's own framing points the same way: `R-SEC-3` gates *persist*,
  and one `scrub` call on the persisted bytes is the only placement that gates it without a field
  list.
- **`HANDOFF.md:249-250` — "**Not MOD-10's**: MOD-10 replaces the `Scrubber` implementation behind
  an unchanged trait; this is a missing call site."** **Confirmed.** `scrub.rs:47-49` names MOD-10
  as the seam it replaces, and `MinimalScrubber` (`:95`) is the stand-in. This plan adds no method
  to the trait, changes no implementation, and depends on nothing MOD-10 will not also satisfy.
  MOD-10 remains free to replace `MinimalScrubber` and inherit the fix.
- **`HANDOFF.md:239` — "from MOD-2, finding F-80".** The MOD-7 milestone 4 review (MEDIUM) already
  hit the neighbouring half and MOD-7 D119/D129 worked around it: `mod.rs:895-896` says in as many
  words that `trim_record.notes` is persisted unscrubbed. That premise is written in **four**
  places, not one (D5), which is what a deferral that outlived two review cycles looks like from
  the inside. D5 is what happens to all four.

## Claims to verify

Every claim is a statement about the tree at `8cc3fda`, checked in the table below. Claims 1–21 were
checked when the plan was drafted. **Claims 22–27 are a second pass**, run after `8952d21` recorded a
maintainer CONFIRM that was never given; that pass falsified claim 8 in part and turned up three
premise sites the first pass had missed (23–25). Verdicts below are as amended, not as first
written — a claim the first pass got wrong is more useful recorded than quietly fixed.

## Verified claims

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | The only production `set_step_prompt` call sites are three, all in `engine.rs` | verified | `engine.rs:3149`, `:3731`, `:4552`; every other occurrence in the workspace is a trait impl (`traits.rs:465`, `mem.rs:5376`, `writer.rs:476`, `pg/write.rs:1440`, `conformance.rs:806`) or a test |
| 2 | None of the three scrubs what it writes | verified | `engine.rs:3144-3147`, `:3726-3729`, `:4547-4550` — bare `serde_json::to_value(&*.trim)` |
| 3 | `TrimRecord::to_value` is called from tests and fixtures only, never from production | verified | `fixtures.rs:2489`, `prompt_skills.rs:128`, `:212`, `prompt_digest.rs:963`, `:964`; the engine re-serialises the struct instead |
| 4 | `spec.template` is never masked | verified | `scrubbed_inputs` (`mod.rs:726-871`) has no `mask` call naming `spec.template`; `TemplateRef` is declared at `mod.rs:126-131` and only its name and version are persisted (`trim.rs:164-171`) |
| 5 | `spec.notes` is never masked | verified | `notes()` at `mod.rs:1110-1114` clones `spec.notes` verbatim; the field is declared at `mod.rs:120-122`; no `mask` call in `scrubbed_inputs` names it |
| 6 | `spec.excerpts.notes` is never masked, and the code says so | verified | `mod.rs:890-907`, in particular `:895-896` "Because `trim_record.notes` is persisted unscrubbed" |
| 7 | `skill_choices` is built from the **masked** spec | verified | `mod.rs:455` rebinds `spec` to `&masked.spec`; `mod.rs:861` calls `select(BoundSkill::collapse(spec.skills.clone()), placed)` after that rebinding |
| 8 | The `excerpts` audit's paths **and root strings** already pass a mask-and-refuse | **partly falsified — amended** | True of `files[]`: `surviving_audit` (`mod.rs:1080-1106`) runs `scrub_text(scrubber, &render::file_block(file), …)?` at `:1098-1102` over the masked spec's files. **False of `roots[]` and `provider_set[]`:** `scrubbed_inputs` walks `spec.excerpts.files` and never `spec.excerpts.audit`, and `surviving_audit` clones the audit wholesale at `mod.rs:1084` (`audit.files` is then rebuilt, `audit.roots`/`provider_set` are not). `file_record` (`excerpt.rs:935-947`) takes no `Scrubber` and clones `repo`/`path` — safe only because its `excerpt` argument is the already-masked file. |
| 9 | The excerpt *files* are masked in the input pass | verified | `mod.rs:784-792` — `file.repo`, `file.path`, `file.content`, `file.provider`; the loop is over `spec.excerpts.files` and names no other field of `ExcerptSet` |
| 10 | `budget_source` and `estimator` are closed spellings, not free text | verified | `trim.rs:191` `crate::prompt::settings::BudgetSource`; `trim.rs:196` `&'static str` set from `spec.estimator.id` |
| 11 | `Scrubber::scrub` masks in place and *then* fails closed on residue | verified | `scrub.rs:50` (doc), `:228-233` (`mask_value` then `find_residue`) |
| 12 | The assembler does exactly that, in that order | verified | `mod.rs:453-454` (input mask) then `:477-483` (rendered residue scan) |
| 13 | Masking is idempotent | verified | `scrub.rs:52`, `:124-129` (the `[REDACTED]` step-over), test `scrubbing_is_idempotent` at `:539-554` |
| 14 | The engine already holds the scrubber where all three sites are | verified | `engine.rs:415` `pub scrubber: &'a dyn Scrubber`; used at `:1326`, `:4422`, `:4423`, `:4920` |
| 15 | `RecordError` already has `Unmasked(#[from] Unmasked)`, so no new variant is needed | verified | `record.rs:247-251`; `Unmasked` is `htui_core::scrub::Unmasked` per the import at `record.rs:105` |
| 16 | All three engine functions already convert `RecordError` into their own error | verified | each site already ends in `RecordError::Encode(…)` and `?` (`engine.rs:3144-3148`, `:3726-3729`, `:4547-4550`) |
| 17 | The input-layer precedent is the milestone-9 CRITICAL | verified | `f48b82b` "fix(core): scrub the assembler's inputs, not its renders (C-1, H-1, M-2)", touching `scrubbed_inputs` |
| 18 | The store-level case writes a hand-built record and is not a production path | verified | `pg_criteria.rs:1076` `set_step_prompt_writes_only_the_digest_and_the_record`, with `json!({ "v": 1 })` at `:1101` and `:1108` |
| 19 | `Section`'s four `Option` fields would not survive a `Value` round-trip | verified | `trim.rs:115`, `:118`, `:121`, `:124` — each carries `skip_serializing_if = "Option::is_none"` with no matching `#[serde(default)]`; `Section` derives `Serialize` only (`:102`) |
| 20 | The assembler's scrub cases live in `prompt_digest.rs` | verified | `prompt_digest.rs:732` and `:1101-1106` destructure `AssembleError::Unmasked` |
| 21 | `.sqlx` holds 268 entries and the last migration is `0007` | verified | `ls crates/htui-store/.sqlx \| wc -l` → 268; `migrations/` holds `0001_init`..`0007_skill_attachments` |
| 22 | `withhold_unmaskable_notes` runs over the excerpt pass's own notes, and the caller's notes are appended after it | verified | `htui-agent/src/excerpt.rs:1011` calls it, `:1012` calls `drop_unmaskable_excerpts`, `:1013-1014` appends the caller's `notes` **after** both. The panic/cancel path at `:993-1006` returns `unscanned(&roots, caps, notes)` with no withholding at all. The producer of one of those caller notes is `with_excerpts` (`engine.rs:4950-4953`), which formats a repo id into free text. |
| 23 | `mod.rs:966` states the same falsified premise the plan corrected at `:895-896` | verified | "`trim_record.notes` is persisted unscrubbed and the preview shows it" — the draft listed only `:895-896` |
| 24 | `htui-agent/src/excerpt.rs:924` states it a third time | verified | "`reader returned it, and `trim_record.notes` is persisted unscrubbed, so a note the scrubber`" |
| 25 | `htui-agent/tests/excerpt.rs:1291` states it a fourth time, in a test comment | verified | "`trim_record.notes` is persisted unscrubbed. A file under a directory named after a known" — the test itself, `excerpts_for_never_persists_a_note_naming_a_masked_path`, stays |
| 26 | Neither store validates or scrubs `trim` | verified | `PgStore::set_step_prompt` (`pg/write.rs:1440-1459`) binds `$3` straight into `UPDATE run_step SET … trim_record = $3`; `State::set_step_prompt` (`mem.rs:1483-1501`) clones the `Value` |
| 27 | `State::finish_step` / `PgStore::finish_step` are a **second** writer of the column | verified | `mem.rs:3938-3957` overwrites `row.trim_record` when the outcome carries one; Postgres uses `trim_record = COALESCE($4, trim_record)`. **No orchestrator path ever populates it** — every `StepOutcome` the engine builds sets `trim_record: None` (`engine.rs:1413, 2292, 3202, 3800, 4241`; `recover.rs:849`), so this is a latent second write path, not a live leak. Recorded so D7's "the store is not the enforcement point" is stated against both writers. |
