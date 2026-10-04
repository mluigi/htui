# Plan: MOD-10 M1 — Scrubber hardening

**Source PRD**: `.claude/prds/mod-10-secret-provider.prd.md`
**Selected Milestone**: 1 — Scrubber hardening
**Complexity**: Medium
**Status**: CONFIRMED by the maintainer 2026-10-04 (fact-checked: 33 claims; 7 amended, 3 falsified)

## Summary

Make the scrubber ready for real resolved secrets before any are resolved. Its pattern rules become
whole-token rules — a prefix *plus* a charset and a minimum length — for a broader set of known key
formats, chosen against counts from the maintainer's own stored rows. A constructor for a resolved
`key → value` map applies a short-value floor. The opt-in `raw` column gets the second, assembled
check that `payload` already has. A scrub refusal gets one typed sentence wherever it lands today:
- the run's `run.failure` (the plain step path);
- the candidate's item note (the fan-out path);
- the judge's `gate_note` (the judge path);
- the chat run's failure.

`R-SEC-3` is amended from "step" to "run". The `Scrubber` trait and every call site stay unchanged
(PRD constraint). Nothing here resolves a secret; M3 feeds the map.

## Grounding (read on `hr/MOD-10`, base `187ca50e`; verified by the fact-check)

- `crates/htui-core/src/scrub.rs`:
  - `Scrubber` trait (`:49`), `Unmasked { path, rule: &'static str }` (`:69`),
    `MinimalScrubber::new(impl IntoIterator<Item = String>)` (`:106`).
  - The rules are `PREFIX_RULES` (`:19-28`, eight bare prefixes) plus the `PRIVATE KEY` marker
    (`:42`). `starts_a_token_with` (`:253`) matches them at a token start, using Unicode
    `is_alphanumeric`.
  - A bare prefix is very loose: `sk-learn`, `src/sk-live.rs` and any `AKIA…` token all fail
    closed today.
  - `htui-core` has no dependency on `regex`. The workspace pins `regex = "1.13"`.
- **`payload` is scrubbed whole, `raw` is not.**
  - The recorder scrubs every chunk's `payload` and `raw` at capture (`record.rs:1656-1661`).
  - At the flush it scrubs the assembled **`payload` only** (`record.rs:1122`, `:1208`).
  - `event_row` then stores the per-chunk raw blobs as an array (`record.rs:1628-1633`).
  - So a credential split across chunks persists in `raw` (opt-in, `keep_raw_events`), and so does
    a known secret split that way. Bare prefixes partly cover this today by refusing the chunk
    that holds the prefix; whole-token rules would let both halves through.
- **Where a scrub refusal comes from.** Only two producers:
  - the recorder's `finish()` (`record.rs:1066-1068`); while recording, it writes a `scrub_residue`
    row instead (`:1608-1620`);
  - `TrimRecord::to_value` (`prompt/trim.rs:299`).

  `DriverError::Scrub` is unreachable in practice: its one producer is the `From<RecordError>` at
  `record.rs:285`. Both producers reach `EngineError::Record(RecordError::Unmasked)`
  (`command.rs:498-503`).
- **Three catch-alls, three outcomes:**

  | Path | Site | What it writes | Run outcome |
  |---|---|---|---|
  | Plain step (`walk_live_step` trim `:3522`, finish `:5866/5870`) | `walk_step` → `fail_hard(…, &err.to_string())` `engine.rs:3422` | `run.failure` | `Failed` |
  | Fan-out candidate (trim `:4183`, finish `:4220/4224`) | `fail_candidate(…, &err.to_string())` `:4090` | item note only | later `NoSurvivingCandidate`, retry, park, or a sibling wins |
  | Judge (trim `:5044`, finish `:5061`) | `run_judge` `:4658` → `JudgeFailure::SessionFailed(err.to_string())` → `fail_judge` | judge step's `gate_note` plus an item note | **parked**, `run.failure` NULL |

  MOD-32's write-up (`docs/decisions/mod/mod-32.md:28-31`) and the `trim.rs:274-285` doc comment
  both say all three "settle the run as Failed". That is wrong for two of the three.
- **Already typed and out of D5's scope:**
  - the assembler's own residue, `AssembleError::Unmasked`, becomes `PromptRefused` and blocks
    the item (`engine.rs:3484`, `:3889-3897`);
  - the same residue in `judge_prompts` becomes `SessionFailed` (`:4925`);
  - promote's `assemble(...)?` (`:1449`) becomes `EngineError::Prompt`.
- **`RunFailure`** (`crates/htui-orch/src/status.rs:41`):
  - `PromptRefused { phase, reason }` is at `:96-101`, with `Display` "prompt refused at
    `{phase}`: {reason}" (`:138-140`).
  - The only exhaustive `match` is that `Display` (`:116-143`).
  - All variants share the test `run_failure_display_is_ana2s_bytes` (`:451`).
- **Nothing parses `run.failure` back.**
  - The UI renders it verbatim (`ui/tabs/backlog/detail/runs.rs:1045-1049`).
  - Promote quotes it as text (`engine.rs:1436-1440`).
  - No snapshot contains "unmasked" or "scrub".
- **Chat.** It ends with `finish_chat_run(run, step, status, at)`, which takes no failure text
  (`traits.rs:610`, called at `agent_worker.rs:4446`). Residue sets `Failed` at `:4143-4145`. The
  store already has `fail_run(run, failure, at)` (`traits.rs:1524`): it sets `failed` and the
  failure from any non-terminal status, and returns `Constraint` when the run is already terminal.
  `crates/htui` depends on `htui-orch`.
- **Item blocking.** `fail_hard` never blocks the item. Only `refuse_prompt` and
  `block_and_fail` do (`engine.rs:5389-5407`). MOD-32 plan D8 records the maintainer's choice to
  keep it that way.
- **Stored transcripts.** `session_event.payload` / `.raw` (JSONB, `migrations/0001_init.sql:513`).
  Earlier refusals are rows with `kind = 'error'`, code `scrub_residue`, and the message
  `<rule> at <path>` (`record.rs:90`).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming | `scrub.rs:19-28` | Rule names are `snake_case` provider + kind. They are persisted in `scrub_residue` rows, so existing names are kept |
| Errors | `scrub.rs:62-80` | `Unmasked` carries a pointer and a rule name, **never text**. New reports carry key names or counts, never values |
| Typed failure | `status.rs:96-101`, `:138-140` | Struct variant with a fixed `Display` sentence, added to `run_failure_display_is_ana2s_bytes` |
| Debug hygiene | `scrub.rs:217-226` | `Debug` prints counts only |
| Tests | `scrub.rs:266-597`; engine residue tests `engine.rs:16734-16870` | Inline `#[cfg(test)]` with `json!` payloads. Engine residue is driven by a scripted chunk or option label carrying `sk-ant-api03-abcdefghijklmnopqrstuvwx` (`parks_with_residue`, `:16734`) |

## Decisions (proposed; CONFIRM accepts or overrides)

- **D1 — Whole-token rules.**
  - Every pattern rule is a regex anchored at an ASCII token start, `(?:^|[^A-Za-z0-9_])`,
    followed by the prefix, a charset and a minimum length.
  - One `RegexSet` decides whether anything hit; a per-rule scan in table order then names the
    first rule, so the reported rule is deterministic.
  - The PEM marker rule is unchanged, and `regex` is added to `htui-core`.
  - Moving from a Unicode to an ASCII token start makes `éAKIA…` count as a token start. That
    errs toward failing closed, which is accepted.
- **D2 — Candidate rule set.** Compile-probed with `regex` 1.13.1 and Postgres 16 ARE (both
  accept every pattern unchanged); the final list is fixed by T0's counts.

  | Rule | Status | Regex |
  |---|---|---|
  | `anthropic_api_key` | existing | `sk-ant-[A-Za-z0-9_-]{20,}` |
  | `openai_api_key` | existing, **amended three times** | Gate `sk-[A-Za-z0-9_-]{20,}`, then refuse when the strict form `sk-(?:(?:proj\|svcacct\|admin\|None\|or-v[0-9]+\|lf)-[A-Za-z0-9_-]{20,}\|[A-Za-z0-9]{20,})` hits, **or** some `sk-` body is neither `ant-…` nor prose (`SK_PROSE`: every `-`/`_` segment a lower/Title/CamelCase word with optional trailing digits, or all digits). History: (1) the hyphen-free legacy form stops `sk-learn-preprocessing-pipeline-v2` failing closed; (2) lane A's leak verifier found OpenRouter `sk-or-v1-` / `sk-None-` / Langfuse `sk-lf-` unrefused (`6242d4a8`); (3) review H1 found ~47% of LiteLLM keys (`sk-`+`token_urlsafe(16)`) and every `sk-<uuid>` unrefused (`0f2d0687`) |
  | `github_pat` | existing | `github_pat_[A-Za-z0-9_]{20,}` |
  | `github_token` | existing, broadened | `gh[pousr]_[A-Za-z0-9]{30,}` |
  | `gitlab_pat` | new | `glpat-[A-Za-z0-9_-]{20,}` |
  | `aws_access_key_id` | existing | `(?:AKIA\|ASIA\|ABIA\|ACCA)[A-Z0-9]{16}` |
  | `slack_bot_token` / `slack_user_token` | existing | `xoxb-[A-Za-z0-9-]{10,}` / `xoxp-[A-Za-z0-9-]{10,}` |
  | `slack_token` | new | `xox[ars]-[A-Za-z0-9-]{10,}` |
  | `google_api_key` | existing | `AIza[0-9A-Za-z_-]{35}` |
  | `stripe_secret_key` | new, **narrowed** | `[rs]k_live_[A-Za-z0-9]{20,}` — live keys only; test-mode keys dropped by maintainer decision at review (M1), since they cannot move money and Stripe's doc sample key is common in code an agent reads |
  | `npm_token` | new | `npm_[A-Za-z0-9]{36}` |
  | `pypi_token` | new | `pypi-AgEIcHlwaS5vcmc[A-Za-z0-9_-]{50,}` |
  | `sendgrid_api_key` | new | `SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}` |
  | `jwt` | new | `eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}` — covers Infisical access tokens; kept only if T0 shows no hits |

  - Any rule that T0 finds hitting stored rows is tightened or dropped; the maintainer decides
    which at the T0 checkpoint.
  - There is no generic entropy rule: failing closed on entropy alone would halt runs.
  - No string in the tracked tree, the 184 snapshots or production code newly matches any rule
    (verified).
- **D3 — Short-value floor applies only to resolved maps.**
  - `MinimalScrubber::new` keeps today's contract: it masks every non-empty value. That matters
    for its 98 call sites, five of which are tests that mask values of 3–4 characters.
  - A new `MinimalScrubber::from_resolved(&BTreeMap<String, String>) -> (Self, Vec<String>)` masks
    values of at least `MIN_MASKED_LEN = 6` characters (ANA-7 §3.4). It returns the **key names**
    of values below the floor. Those values are still injected by M3, just not masked.
- **D4 — Mask label stays `[REDACTED]`.** ANA-7's `[REDACTED: KEY_NAME]` is not adopted, for three
  reasons:
  - the MOD-61 test pins `[REDACTED]`;
  - the idempotence scan steps over one fixed marker (`scrub.rs:118-140`);
  - a key name in a transcript says which credential was used.
- **D5 — One typed sentence for a scrub refusal, on all three engine paths.**
  - Add `RunFailure::ScrubRefused { phase: String, rule: String }`. Its `Display` names the phase
    and the rule, and never the text or the pointer.
  - A helper maps `EngineError::Record(RecordError::Unmasked(u))` to it. Each path writes the
    sentence where it already writes text:
    - `walk_step`: `fail_hard`, so `run.failure`;
    - candidate: `fail_candidate`'s note;
    - judge: `SessionFailed`'s text, which becomes the `gate_note`.
  - **Run outcomes are unchanged** (failed, candidate failed, parked), and so is item blocking
    (MOD-32 D8). Every other error keeps writing its own text.
- **D6 — Chat records the typed sentence too.**
  - When `recorder.finish()` returns `Unmasked`, the chat path records the same `ScrubRefused`
    sentence (phase `chat`) through the existing `fail_run`. There is no signature change.
    Changing `finish_chat_run` instead was costed at 9 files plus `.sqlx` and rejected.
  - **Ordering hazard for the blueprint:** `fail_run` refuses an already-terminal run, and
    `finish_chat_run` may also close the step. The blueprint fixes the sequence:
    `fail_run` then a step-only close, or the reverse with a pre-check.
- **D7 — Requirement and doc corrections.**
  - `R-SEC-3` becomes "marks the run failed with a typed reason", with a dated amendment line at
    the head of `docs/REQUIREMENTS.md`.
  - The `trim.rs:274-285` doc comment is corrected to the three real outcomes, and the MOD-32
    write-up gets a dated correction note.
- **D8 — Check `raw` again when the row is stored.**
  - At the flush, after scrubbing `payload`, join the string values of the row's raw chunks in
    order with no separator. Scrub the joined string with the same scrubber.
  - If masking changed it, or a residue remains, store the row with **`raw = NULL`** (the payload
    is kept) and count the drop.
  - Fail-safe direction: `raw` is opt-in debug data. A false positive only loses `raw`.
  - This closes the split-chunk gap for pattern rules **and** for exact-match secrets, which M3
    makes real.

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `scripts/scrub-audit.sql` | CREATE | Per-rule hit counts over stored string values and keys. Counts only, never text | T0 |
| `crates/htui-core/Cargo.toml` | UPDATE | `regex = { workspace = true }` | T1 |
| `Cargo.lock` | UPDATE | `htui-core`'s dependency list | T1 |
| `crates/htui-core/src/scrub.rs` | UPDATE | D1/D2 rules, D3 `from_resolved`, module docs, tests | T1, T2 |
| `crates/htui-core/src/prompt/mod.rs` | UPDATE | fixtures `:1371-1373`, `:1409-1427`, `:1459`, `:1601` | T1 |
| `crates/htui-core/tests/prompt_digest.rs` | UPDATE | fixtures `:730`, `:1122`, `:1477`, `:1505` | T1 |
| `crates/htui-core/tests/prompt_persona.rs` | UPDATE | fixture `:202` | T1 |
| `crates/htui-agent/tests/excerpt.rs` | UPDATE | fixtures `:1595`, `:1631/1639`, `:1731` | T1 |
| `crates/htui-orch/src/verify.rs` | UPDATE | fixture `:817` | T1 |
| `crates/htui-orch/src/status.rs` | UPDATE | D5 variant, `Display`, test row | T3 |
| `crates/htui-orch/src/engine.rs` | UPDATE | D5 helper and the three mappings, engine tests, **plus** the fixtures at `:14944`, `:14975` (moved here from T1) | T3 |
| `crates/htui/src/agent_worker.rs` | UPDATE | D6 | T4 |
| `crates/htui-agent/src/record.rs` | UPDATE | D8, plus its tests | T5 |
| `crates/htui-agent/tests/recorder.rs` | UPDATE | D8 integration test (raw split across chunks) | T5 |
| `crates/htui-core/src/prompt/trim.rs` | UPDATE | D7 doc comment | T6 |
| `docs/REQUIREMENTS.md`, `docs/decisions/mod/mod-32.md` | UPDATE | D7 | T6 |

## Tasks

### T0: Rule audit query (gates T1's final list)
- **Action**: Write `scripts/scrub-audit.sql`. For each D2 rule it counts:
  - `session_event` rows where any **string value or object key** of `payload` or `raw` matches.
    It walks values with `jsonb_path_query(…, 'strict $.**')` and `#>> '{}'`, plus keys. Matching
    `::text` would hide a token right after a JSON escape such as `\n`.
  - The same for `run_step.trim_record`.
  - Existing `scrub_residue` rows grouped by rule name.

  It outputs counts only and never selects a matched string. Plain `'…'` literals only; `E''`
  strings would need `\\.`.
- **Checkpoint**: the maintainer runs it on the host database (`docker exec htui-postgres psql …
  -f`) and pastes the counts. T1 fixes the final list from them.
- **Validate**: runs clean against a migrated scratch database in the sandbox (`localhost:5439`).

### T1: Whole-token rule set (TDD)
- **Action**:
  1. Write tests first, per rule: a real-shaped key at the string start, after a space, in
     `Bearer …`, and as an object key, each refused. Prose not refused: `sk-learn`,
     `sk-learn-preprocessing-pipeline-v2`, `AKIA` alone, `subtask-x`, `ghp_short`. `sk-ant-` plus
     20 or more characters reported as `anthropic_api_key`.
  2. Replace `PREFIX_RULES` / `starts_a_token_with` with the D1 table and `RegexSet`.
  3. Lengthen the listed fixtures. Each new value has 20 or more `[A-Za-z0-9]` characters after
     `sk-` (or meets its rule's own length) and still starts at a token start. `Xsk-…` keeps its
     `X` so the unjoined path stays clean.
- **Validate**: `cargo test -p htui-core`, `-p htui-agent --features test-support --test excerpt`,
  `-p htui-orch --features test-support verify`.

### T2: `from_resolved` and the short-value floor (TDD; after T1, same file)
- **Action**: Write tests first:
  - a value of 6 or more characters is masked;
  - a value of 5 characters is not masked and its key is listed;
  - the list holds key names only;
  - `Debug` shows counts only;
  - an empty map is legal.

  Then implement D3.
- **Validate**: `cargo test -p htui-core scrub`.

### T3: Typed scrub sentence on the three engine paths (TDD)
- **Action**:
  - Write tests first:
    1. `ScrubRefused` gets its row in `run_failure_display_is_ana2s_bytes`.
    2. Plain step: a credential-shaped template name or repo slug makes `trim_record` refuse,
       and the run ends `Failed` with `run.failure` set to the sentence.
    3. Plain step: a scripted chunk makes `finish` refuse, with the same result.
    4. Candidate: the candidate is `failed` and its note carries the sentence.
    5. Judge: the run parks and the judge's `gate_note` carries the sentence.
    6. The three existing cancel/fence residue tests (`:16754`, `:16790`, `:16844`) still pass
       unchanged.
  - Then add the helper and mappings at `:3422`, `:4090` and `:4658`.
  - Lengthen the fixtures at `:14944` and `:14975` so they match under both the old and new rules.
- **Validate**: `cargo test -p htui-orch --features test-support --no-fail-fast`, then grep for
  `SIGABRT` (stack headroom).

### T4: Chat records the typed sentence (TDD; after T3 for the variant)
- **Action**:
  - Write a test first: a chat whose recorder refuses ends with `run.failure` equal to the
    `ScrubRefused` sentence (phase `chat`) and with the step closed.
  - Then implement D6 in the order the blueprint fixes.
- **Validate**: `cargo test -p htui --features testkit agent_worker`.

### T5: `raw` checked again at the flush (TDD; independent)
- **Action**:
  - Write tests first, all with `retain_raw` on:
    1. A known secret split across two chunks: the row is stored with `raw = NULL`, and the
       payload is masked.
    2. A pattern credential split across two chunks: same result.
    3. A clean coalesced row keeps its `raw` array.
    4. A single-chunk row is unchanged.
  - Then implement D8.
- **Validate**: `cargo test -p htui-agent --features test-support --test recorder`, plus the lib tests.

### T6: Documents
- **Action**: D7. Also `scrub.rs` module docs: `MinimalScrubber` stops being described as a
  "stand-in", though the name is kept. That edit rides with T1, since T1 owns the file.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

**Independence** (by file set; verified by intersection):
- T0 `{scripts/scrub-audit.sql}`
- T1 `{htui-core/Cargo.toml, Cargo.lock, scrub.rs, prompt/mod.rs, tests/prompt_digest.rs,
  tests/prompt_persona.rs, htui-agent/tests/excerpt.rs, htui-orch/src/verify.rs}`
- T2 `{scrub.rs}` — runs serially after T1
- T3 `{htui-orch/src/status.rs, htui-orch/src/engine.rs}`
- T4 `{htui/src/agent_worker.rs}` — runs after T3, because it needs the variant
- T5 `{htui-agent/src/record.rs, htui-agent/tests/recorder.rs}`
- T6 `{prompt/trim.rs, docs/REQUIREMENTS.md, docs/decisions/mod/mod-32.md}`

That gives three parallel lanes: **A** T1 → T2, **B** T3 → T4, **C** T5. T0 and T6 sit outside
the lanes.

T1's *final* rule list waits on T0's checkpoint. T1 can start from D2 as drafted and the T0
outcome amends the table.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # grep SIGABRT
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A new rule hits real stored text, so runs fail closed | Medium | T0 counts on the host database before the list is fixed |
| Tightening drops a fail-closed catch the bare prefix had | Low | `payload` is scrubbed whole; D8 extends the same check to `raw` |
| A flush cut splits a key across two persisted rows | Low | **Corrected at review (M2):** pre-existing for every rule, and **widened** by M1 for the 8 legacy rules — the window where both halves persist grows from the bare prefix (3–7 chars) to prefix + minimum length (15–39 chars), e.g. `sk-ant-` 7 → 27. Before, a cut in that window left a partial key; now rows n and n+1 can rebuild it. Needs a message over 16 KiB with a key at the cut. The seam scan is deferred to M3, where it must also cover exact-match secrets |
| Judge path: `judge_sessions` reads `finished?` only when both calls succeed (`:5061-5070`), so a residue alongside a failing judge call is not reported (the row was still dropped) | Low | Pre-existing; T3 does not widen it. Noted in the write-up |
| `fail_run` / `finish_chat_run` ordering in chat | Medium | Blueprint fixes it; T4 test pins it |
| `htui-orch` tests near the 2 MiB stack | Low | No new futures; `--no-fail-fast` with a `SIGABRT` grep |

## Acceptance

- [ ] T0 counts reviewed by the maintainer; D2 final list recorded here
- [x] All tasks complete, tests written first
- [ ] Validation passes on the merged tree
- [x] Patterns mirrored, not reinvented

## Verified claims

Fact-check 2026-10-03: two parallel checkers, plus inline reads. Compile probes ran on `regex`
1.13.1 (`/tmp/rxprobe`) and Postgres 16.15 at `localhost:5439`.

| Claim | Verdict | Evidence |
|---|---|---|
| `Scrubber` / `Unmasked` / `MinimalScrubber::new` / `PREFIX_RULES` / PEM marker locations | CONFIRMED | `scrub.rs:49`, `:69`, `:106`, `:19-28`, `:42` |
| `htui-core` has no `regex` dependency; the workspace pins `regex = "1.13"` | CONFIRMED | `crates/htui-core/Cargo.toml`; root `Cargo.toml:110` |
| Every D2 regex compiles in Rust `regex`, with the token-start prefix | CONFIRMED | Probe built `--offline`, as a `RegexSet` and per rule |
| Every D2 regex is accepted by Postgres ARE unchanged | CONFIRMED | `select 'x' ~ '<re>'` per rule on PG 16.15 (`standard_conforming_strings=on`) |
| The D2 negatives (`sk-learn`, `subtask-42`, `AKIA`, `ghp_short`, …) match no rule | CONFIRMED, **amended** | Pass as drafted, but the drafted `openai_api_key` matched `sk-learn-preprocessing-pipeline-v2`; the regex was amended |
| A `sk-ant-` key is always reported as `anthropic_api_key` | FALSIFIED for short keys, **amended** | With fewer than 20 characters after `sk-ant-` the drafted OpenAI rule claimed it; the amended OpenAI rule no longer can |
| Matching `payload::text` counts every hit | FALSIFIED, **amended** | `"line\nAKIA…"` → `f` on `::text`, `t` per leaf; T0 walks string values and keys |
| Stored rows are scrubbed whole, so whole-token rules lose nothing on persist | FALSIFIED for `raw`, **amended** | `record.rs:1122`/`:1208` re-scrub `payload` only; `:1628-1633` stores raw chunks. D8 and T5 added |
| No tracked string, snapshot or production constant newly matches D2 | CONFIRMED | Tree-wide search; 184 `*.snap`, fixtures, seeds and migrations |
| Fixtures relying on bare-prefix refusal exist in 7 files | CONFIRMED | Listed under Files to Change |
| T1 and T3 are file-disjoint | FALSIFIED as drafted, **amended** | `engine.rs:14944`/`:14975` fixtures moved from T1 to T3 |
| `MinimalScrubber::new` callers are unaffected by D3 | CONFIRMED | 98 calls / 26 files; production `agent_worker.rs:3946`, `runtime.rs:437`, `:929`, `preview.rs:315`; short-value tests stay on `new` |
| Scrub refusals reach `EngineError::Record(RecordError::Unmasked)` | CONFIRMED | `command.rs:498-503` |
| Producers are only recorder `finish()` and `TrimRecord::to_value` | CONFIRMED | `record.rs:1066-1068`, `:1608-1620`; `trim.rs:299` |
| `DriverError::Scrub` can reach the engine catch-alls | FALSIFIED (unreachable) | Only producer is `record.rs:285` inside `drive`, where `record()` never yields `Unmasked` |
| Two catch-alls write the refusal as run failure text | **AMENDED** | Three catch-alls: `:3422` → `run.failure`; `:4090` → item note only; `:4658` → `SessionFailed` → `gate_note`, park |
| MOD-32 write-up and `trim.rs` doc say all three settle the run Failed, and that is accurate | FALSIFIED | `mod-32.md:28-31`, `trim.rs:274-285` vs the code paths above; D7 corrects them |
| `RunFailure::PromptRefused { phase, reason }` with a fixed `Display` | CONFIRMED | `status.rs:96-101`, `:138-140` |
| Adding a `RunFailure` variant forces edits beyond `Display` | FALSIFIED (it doesn't) | Only exhaustive match is `status.rs:116-143` |
| `run.failure` is parsed back by some consumer | FALSIFIED (it isn't) | UI verbatim `runs.rs:1045-1049`; promote quotes it `engine.rs:1436-1440`; no snapshot holds the text |
| Existing tests pin a scrub refusal ending a run | FALSIFIED (none do) | The three residue tests `:16754`, `:16790`, `:16844` assert cancel/fence settle nothing; unchanged under D5 |
| An engine test can drive a recorder refusal | CONFIRMED | `parks_with_residue` `:16734`; `ScriptedStep` `fake.rs:1076` |
| `finish_chat_run` takes no failure text | CONFIRMED | `traits.rs:610`; call `agent_worker.rs:4446` |
| Typing chat via `finish_chat_run` costs about 6 files | FALSIFIED (9 plus `.sqlx`), **amended** | D6 uses existing `fail_run` (`traits.rs:1524`) instead |
| `fail_run` sets failed and failure from any non-terminal status | CONFIRMED | `traits.rs:1517-1524` |
| `crates/htui` depends on `htui-orch` | CONFIRMED | `crates/htui/Cargo.toml:30` |
| A scrub refusal never blocks the item | CONFIRMED | `fail_hard` does not; only `refuse_prompt` / `block_and_fail` (`engine.rs:5389-5407`); MOD-32 plan D8 |
| `fail_candidate` writes an item note | CONFIRMED | `engine.rs:4325-4333` |
| The assembler's own residue is already typed (`PromptRefused`) | CONFIRMED | `prompt/mod.rs:440`; `engine.rs:3484`, `:3889-3897` |
| `session_event.payload` / `.raw` hold stored transcripts | CONFIRMED | `migrations/0001_init.sql:513-528` |
| `scrub_residue` message is `<rule> at <path>` | CONFIRMED | `record.rs:90` |
| The sandbox Postgres is reachable with trust auth | CONFIRMED | `psql -h localhost -p 5439 -U postgres` lists databases |
| ASCII token start changes behaviour on non-ASCII input | CONFIRMED (accepted) | Current check uses Unicode `is_alphanumeric`; D1 errs toward failing closed |

## Review (2026-10-04)

`rust-reviewer` over `187ca50e..HEAD`: **approve with fixes** (1 HIGH, 2 MEDIUM, 4 LOW, NITs). Each
actionable finding went through one adversarial verifier (review-phase ultracode).

| Finding | Verified | Disposition |
|---|---|---|
| H1 `openai_api_key` fails open on hyphenated/underscored `sk-` keys (LiteLLM ~47%, `sk-<uuid>` 100%) | real, high | **fixed** `0f2d0687` (gate + `SK_PROSE` filter; audit SQL mirrored) |
| M1 T0 audit not run; `jwt` / `sk_test_` may fail closed on public sample tokens | process | **decided 2026-10-04:** keep `jwt` (real bearer credentials; resolved ones are masked, not refused, once M3 lands); Stripe narrowed to live keys |
| M2 cross-row split window widened | real, **low** | Risks row corrected; seam scan → M3 |
| L1 token start misses `\n`-escaped / `%20`-encoded keys | real, low | → M3 (widens fail-closed; needs a fresh host audit) |
| L2 weakened no-leak needles | real, nit | **fixed** `b8130def` |
| L3 no one-character-short boundary tests | real, low | **fixed** `a7c7acc9` (`ONE_SHORT`) |
| L4 residue beside a failing judge call unreported | pre-existing | documented (write-up) |
| N1 `(?-u:\b)` anchor for literal-prefix acceleration | equivalent, nit | → M3 with a benchmark |
| N2 loose candidate-path assertion | real, nit | **fixed** `3c3403e2` (sibling-wins pinned) |
| `unrecovered` sweep path writes untyped `Unmasked` | **refuted** | no producer reachable on the recovery path |
| `from_resolved` masks values verbatim (trailing newline) | note | → M3 |
