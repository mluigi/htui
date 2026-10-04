# Blueprint: MOD-10 milestone 1 — Scrubber hardening

**Plan**: `.claude/plans/mod-10-m1-scrubber-hardening.plan.md`. D1–D8, T0–T6 and the plan's
"Verified claims" table are binding. They were re-checked against the tree at `523e5a66`
(`hr/MOD-10`, T0 committed) on 2026-10-04.
**PRD**: `.claude/prds/mod-10-secret-provider.prd.md`.
**Rule for the tree vs. the plan**: where they disagree, the tree wins. Each such point is
recorded under **Amendments** (A-n) with its evidence. Anything not proven in source is marked
**VERIFY — implementer must check**.

Conventions inherited unchanged:
- toolchain and lint headers:
  - MSRV 1.98 (`Cargo.toml:8`), so `std::sync::LazyLock` (stable since 1.80) is available;
  - `#![warn(missing_docs)]` in `htui-core`, `htui-orch` and `htui-agent` (`lib.rs:9`, `:25`,
    `:50`);
  - workspace lints `unsafe_code = forbid`, `missing_debug_implementations`,
    `unused_qualifications` and `clippy::all` (not pedantic), with the gate at `-D warnings`;
- the `Scrubber` trait and `Unmasked` are unchanged (PRD constraint), and no store-trait
  signature changes;
- no new error variant beyond `RunFailure::ScrubRefused`;
- one test-helper set per file: the repo duplicates fixtures per file rather than sharing them.

T0 (`scripts/scrub-audit.sql`) is done and out of scope here. T1's final rule list still waits
on the maintainer's T0 counts. This blueprint uses D2 as drafted.

---

## Amendments (plan ≠ tree)

| # | Plan says | Tree says | Resolution |
|---|---|---|---|
| A-1 | D8: "join the string values of the row's raw chunks in order with no separator" | Each raw chunk is a whole wire message. An ACP `session/update` carries the string leaves `"2.0"` (`jsonrpc`), `"session/update"` (`method`), the `sessionId`, the `sessionUpdate` tag and `content.type` **around** `content.text` in every chunk. This holds in both key orders: sorted under `-p`, and insertion order under `--workspace`, because `agent-client-protocol` turns on `serde_json/preserve_order` (`prompt/trim.rs:198-204`, F-35). A flat join puts chunk 2's protocol leaves between the two halves of a split secret, so neither an exact-match mask nor a pattern rule sees it. D8 would pass a single-leaf test fixture and catch nothing on a real transport | **Join per JSON pointer.** For each pointer that holds a string leaf in any chunk, concatenate that pointer's strings across chunks in chunk order with no separator. Then scrub all joined strings together. Streamed text lives at one fixed pointer in every chunk (ACP `/params/update/content/text`, claude stream-json `/event/delta/text`), so this rebuilds exactly the split D8 targets. Keys are excluded from the joined text: they are protocol field names and are already scanned per chunk at capture (`record.rs:1656-1661`). D8's intent (re-check the assembled raw, NULL on change or residue, keep the payload, count the drop) is unchanged |
| A-2 | Grounding: "a credential split across chunks persists in `raw`"; T5 test 2: "a pattern credential split across two chunks: same result" (raw NULL, payload masked) | When the split credential is in the coalesced **payload** text, the flush's payload re-scrub (`record.rs:1122-1129`) returns `Unmasked`. The **whole row** is then replaced by `residue_row` (`raw: Vec::new()`, `:1715`), so raw is already gone today. A pattern rule never masks, so "payload masked" cannot occur for a pattern credential. What really persists is a **known secret** split across chunks (the payload is masked and the row kept with its raw halves), or a split in a **raw-only** leaf the payload does not mirror | T5 test 2 becomes "a pattern credential split across two chunks **in a raw-only leaf**: payload kept unchanged, `raw = NULL`, the session still finishes `Ok`". A new test 2b pins the existing behaviour: a pattern credential split in the payload text still becomes a `scrub_residue` row |
| A-3 | T3 file set `{status.rs, engine.rs}` | The fan-out and judge fixtures (`fan_research`, `set_judge`, `research_candidates`, `judge_of`, `notes_of`, `steps_of`) live in `crates/htui-orch/src/conformance.rs` (`:3225-3295`), and its `mod fanout_paths` (`:7354`) is where "the judge failures criteria 8-10 do not name" are tested. `engine.rs` has no judge harness | T3's candidate and judge tests (plan T3 cases 4, 5) go in `conformance.rs` `mod fanout_paths`, **not** in `CASES` (H-1). T3's file set gains `crates/htui-orch/src/conformance.rs`. No other task touches it, so the lanes stay disjoint |
| A-4 | T1 fixture list: the seven listed files | `scrub.rs`'s **own** inline tests use short fixtures that the D2 rules stop matching: `sk-ant-api03-aaaaaaaaaaaa` (18 chars after `sk-ant-`), `sk-proj-aaaaaaaaaaaaaaaa` (16), `ghp_aaaaaaaaaaaaaaaaaaaa` (20 < 30) in `an_unknown_credential_prefix_fails_closed_with_a_json_pointer`; `sk-ant-api03-AAAA` in `a_credential_shaped_object_key_is_refused`; `ghp_aaaaaaaaaaaaaaaaaaaa` in `a_masked_secret_never_appears_in_the_error_path`; `sk-ant-api03-zzzzzzzzzzzz` in `a_credential_mid_sentence_is_caught_and_the_root_pointer_is_empty` | Same file as T1, so there is no file-set change. T1 lengthens them (§B.1 table). Every other D2 hit was rechecked tree-wide and is either already long enough (`sk-ant-api03-abcdefghijklmnopqrstuvwx`, `AKIAAAAAAAAAAAAAAAAA` = AKIA+16, the `AIza` fixture = AIza+37, `sk-live0123456789abcdef` = 20) or not scrub-dependent (`probe.rs:1214`, a `Debug` test) |
| A-5 | D7: R-SEC-3 becomes "marks the **run** failed with a typed reason" | Under D5 the run fails only on the plain-step path. On the candidate path the candidate fails and the run may still succeed through a sibling. On the judge path the run **parks** with `run.failure` NULL. The D7 sentence is accurate for one of the three paths | **Not applied; flagged to the maintainer.** T6 writes D7's wording unless the maintainer amends it at review. A suggested accurate form is under §B.6. This is the one point here that needs a maintainer word |
| A-6 | D1: "one `RegexSet` decides whether anything hit; a per-rule scan in table order then names the first rule" | `RegexSet::matches(text).iter()` yields matching pattern indices in **ascending** order, so the first index is the first rule in table order | Realised as `is_match` (the gate) then `matches(...).iter().next()`. That is one compiled automaton and no second `Vec<Regex>`. It is semantically identical to D1, so it is recorded as a conforming realisation, not a change |

---

## A. Per-file change table

| # | File | Action | Task | What changes (and what must **not**) |
|---|---|---|---|---|
| 1 | `crates/htui-core/Cargo.toml` | UPDATE | T1 | `regex = { workspace = true }` under `[dependencies]` (the workspace pins `"1.13"`, `Cargo.toml:110`; `htui-agent/Cargo.toml:44` already uses it) |
| 2 | `Cargo.lock` | UPDATE | T1 | Only `htui-core`'s `dependencies` list gains `"regex"`. `regex` is already locked, so no version moves. **Never `cargo update`** |
| 3 | `crates/htui-core/src/scrub.rs` | UPDATE | T1 | `PREFIX_RULES` / `starts_a_token_with` → `PATTERN_RULES` + `TOKEN_START` + `static PATTERNS: LazyLock<RegexSet>`; `residue_rule` rewritten; module docs (no longer a "stand-in"; describe whole-token rules); struct doc `:82-97` "known credential prefixes" → "whole-token pattern rules"; inline fixtures lengthened (A-4) and new T1 tests. **Not**: `mask`, `mask_value`, `find_residue`, `Unmasked`, `Scrubber`, `REDACTED`, `PEM_*` |
| 4 | `crates/htui-core/src/scrub.rs` | UPDATE | T2 | `pub const MIN_MASKED_LEN: usize = 6;` `MinimalScrubber::from_resolved`; T2 tests. **Not**: `new`'s contract |
| 5 | `crates/htui-core/src/prompt/mod.rs` | UPDATE | T1 | Fixtures `:1371-1373`, `:1403-1427` (`Xsk-1`), `:1459`, `:1601` per §B.1 |
| 6 | `crates/htui-core/tests/prompt_digest.rs` | UPDATE | T1 | `sk-ant-api03-DEADBEEF` (`:730/736`, `:1122/1137`, `:1477`) and `sk-ant-api03-ROOTSLUG` (`:1505`) lengthened. The `!contains(...)` assertions follow the new literal |
| 7 | `crates/htui-core/tests/prompt_persona.rs` | UPDATE | T1 | `:202/208` same |
| 8 | `crates/htui-agent/tests/excerpt.rs` | UPDATE | T1 | `sk-live` at `:1595`, `:1619`, `:1631/1639`, `:1731` |
| 9 | `crates/htui-orch/src/verify.rs` | UPDATE | T1 | `sk-ant-notarealkey` `:817` |
| 10 | `crates/htui-orch/src/status.rs` | UPDATE | T3 | `RunFailure::ScrubRefused { phase, rule }`; its `Display` arm; `RunFailure::scrub_refused` constructor; a row in `run_failure_display_is_ana2s_bytes` (`:451`), plus one hygiene test |
| 11 | `crates/htui-orch/src/engine.rs` | UPDATE | T3 | `fn failure_text` beside `is_fenced` (`:6427`); three call sites `:3422`, `:4090`, `:4658`; plain-step tests beside the residue tests (`:16730+`); one `Harness` helper `add_primary_repo_named`; fixtures `:14944`, `:14975` lengthened. **Not**: `fail_hard`, `fail_candidate`, `fail_judge`, `refused_over_finish`, `is_fenced`, or any `Cancelled`/fenced arm |
| 12 | `crates/htui-orch/src/conformance.rs` | UPDATE | T3 (A-3) | Two tests in `mod fanout_paths` (`:7354`) plus a local `leaks(body)` helper. **Not** a `CASES` entry |
| 13 | `crates/htui/src/agent_worker.rs` | UPDATE | T4 | `fn chat_failure(&RecordError) -> Option<String>`; `ChatBinding::record_failure`; the residue arm at `:4143-4146`; one in-module test. **Not**: `close_run`, `ChatBinding::close`, or the panic answer path (`:3720-3750`) |
| 14 | `crates/htui-agent/src/record.rs` | UPDATE | T5 | `raw_withheld` counter (struct field, `Debug`, accessor, `RecorderSummary` field, `finish`); `withhold_split_raw` method; free fn `joined_raw_leaves`; called in `flush` (`:1122`) and `release_held` (`:1208`); unit test for the join |
| 15 | `crates/htui-agent/tests/recorder.rs` | UPDATE | T5 | Integration tests §D.5 |
| 16 | `crates/htui-core/src/prompt/trim.rs` | UPDATE (doc) | T6 | `to_value` doc `:274-285`: the three real outcomes |
| 17 | `docs/REQUIREMENTS.md` | UPDATE (doc) | T6 | Amendment line at the head (after `:34`); R-SEC-3 `:311-313` |
| 18 | `docs/decisions/mod/mod-32.md` | UPDATE (doc) | T6 | A dated correction note after `:28-31` |

No migration, no SQL, no `.sqlx` change, no `insta` snapshot change.

---

## B. Interfaces, exactly

### B.1 T1 — whole-token rules (`scrub.rs`, D1/D2)

```rust
use std::sync::LazyLock;
use regex::RegexSet;

/// Whole-token credential rules, as `(rule name, pattern body)`, in reporting order (D1, D2).
///
/// Each body is a prefix plus a charset plus a minimum length. It is anchored at an ASCII token
/// start by [`TOKEN_START`] when compiled. Names are persisted in `scrub_residue` rows, so an
/// existing name never changes.
const PATTERN_RULES: &[(&str, &str)] = &[
    // the eight existing names, in their existing order (the reporting order a two-hit string
    // has always had)
    ("anthropic_api_key", r"sk-ant-[A-Za-z0-9_-]{20,}"),
    ("github_pat",        r"github_pat_[A-Za-z0-9_]{20,}"),
    ("github_token",      r"gh[pousr]_[A-Za-z0-9]{30,}"),
    ("aws_access_key_id", r"(?:AKIA|ASIA|ABIA|ACCA)[A-Z0-9]{16}"),
    ("slack_bot_token",   r"xoxb-[A-Za-z0-9-]{10,}"),
    ("slack_user_token",  r"xoxp-[A-Za-z0-9-]{10,}"),
    ("google_api_key",    r"AIza[0-9A-Za-z_-]{35}"),
    ("openai_api_key",    r"sk-(?:(?:proj|svcacct|admin)-[A-Za-z0-9_-]{20,}|[A-Za-z0-9]{20,})"),
    // new in MOD-10 (D2); the T0 checkpoint may tighten or drop any of these
    ("gitlab_pat",        r"glpat-[A-Za-z0-9_-]{20,}"),
    ("slack_token",       r"xox[ars]-[A-Za-z0-9-]{10,}"),
    ("stripe_secret_key", r"[rs]k_(?:live|test)_[A-Za-z0-9]{20,}"),
    ("npm_token",         r"npm_[A-Za-z0-9]{36}"),
    ("pypi_token",        r"pypi-AgEIcHlwaS5vcmc[A-Za-z0-9_-]{50,}"),
    ("sendgrid_api_key",  r"SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}"),
    ("jwt",               r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"),
];

/// An ASCII token start: the string start, or one character that is not `[A-Za-z0-9_]` (D1).
const TOKEN_START: &str = r"(?:^|[^A-Za-z0-9_])";

/// [`PATTERN_RULES`] compiled once per process, index for index.
static PATTERNS: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new(PATTERN_RULES.iter().map(|(_, body)| format!("{TOKEN_START}(?:{body})")))
        .expect("the pattern rules are literals and compile")
});

fn residue_rule(text: &str) -> Option<&'static str> {
    if PATTERNS.is_match(text) {
        if let Some(index) = PATTERNS.matches(text).iter().next() {   // ascending = table order (A-6)
            return Some(PATTERN_RULES[index].0);
        }
    }
    text.contains(PEM_MARKER).then_some(PEM_RULE)
}
```

- `starts_a_token_with` and `PREFIX_RULES` are deleted (both private, with no other caller).
- `^` without `(?m)` anchors only at the haystack start. `\n`, a space, `/`, `]` (after
  `[REDACTED]`) and `"` are all token starts, so `htu[REDACTED]sk-…` still refuses
  (`prompt/mod.rs:1403-1427`).
- A unit test, `every_pattern_rule_compiles_alone`, iterates `PATTERN_RULES` with
  `regex::Regex::new`. It tells a broken edit apart from a `LazyLock` panic.

**Fixture lengthening (T1).** Each value must match under the **new** rules and still start at a
token start.

| Site | Old | New |
|---|---|---|
| `scrub.rs` tests (A-4) | `sk-ant-api03-aaaaaaaaaaaa` / `-zzzzzzzzzzzz` / `-AAAA` | `sk-ant-api03-aaaaaaaaaaaaaaaaaaaa` (and `z…`/`A…` likewise, ≥ 20 after `sk-ant-`) |
| `scrub.rs` tests | `sk-proj-aaaaaaaaaaaaaaaa` | `sk-proj-aaaaaaaaaaaaaaaaaaaa` (20) |
| `scrub.rs` tests | `ghp_aaaaaaaaaaaaaaaaaaaa` | `ghp_` + 36 `a` |
| `prompt/mod.rs:1371-1373`, `:1459` | `src/sk-live.rs`, `docs/ghp_token.md`, `sk-repo` | `src/sk-live0123456789abcdefghij.rs`, `docs/ghp_0123456789abcdefghijklmnopqrstuvwxyz.md`, `sk-repo0123456789abcdefghij`; the `!note.contains("sk-")` asserts stay as they are |
| `prompt/mod.rs:1403-1427` | `Xsk-1.rs` | `Xsk-1234567890abcdefghij.rs` (keeps `X`, D1 note in T1 step 3) |
| `prompt/mod.rs:1601` | `sk-repo0123456789` | `sk-repo0123456789abcdefghij` |
| `prompt_digest.rs`, `prompt_persona.rs` | `sk-ant-api03-DEADBEEF`, `-ROOTSLUG` | `sk-ant-api03-DEADBEEFDEADBEEFDEADBEEF`, `sk-ant-api03-ROOTSLUGROOTSLUGROOTSLUG` (the `!contains("ROOTSLUG")` asserts still hold) |
| `excerpt.rs` `:1595`… | `sk-live…` | `sk-live0123456789abcdefghij…` |
| `verify.rs:817` | `sk-ant-notarealkey` | `sk-ant-notarealkeynotarealkey00` |

### B.2 T2 — `from_resolved` (`scrub.rs`, D3)

```rust
/// ANA-7 §3.4's floor: a resolved value shorter than this many characters is injected but not
/// masked, because masking a 3-character value would shred every transcript.
pub const MIN_MASKED_LEN: usize = 6;

impl MinimalScrubber {
    /// A scrubber over a resolved `key → value` map (MOD-10 D3).
    ///
    /// Masks every value of at least [`MIN_MASKED_LEN`] characters (`chars().count()`). Returns the
    /// **key names** of the values below the floor, in map order. An empty value is below the floor
    /// and is listed. No value is ever returned or logged. An empty map is legal and still fails
    /// closed on the pattern rules.
    #[must_use]
    pub fn from_resolved(resolved: &BTreeMap<String, String>) -> (Self, Vec<String>);
}
```

The implementation is one pass over the map: values at or above the floor go into
`Self::new(...)`, and keys below it go into `short`. `new` is untouched (98 callers, D3).
`use std::collections::BTreeMap;` is added to `scrub.rs`.

### B.3 T3 — `RunFailure::ScrubRefused` and the engine helper (D5)

`crates/htui-orch/src/status.rs`:

```rust
/// MOD-10 D5: a scrub refusal on a live step. It is either `TrimRecord::to_value` refusing the
/// step's record, or the recorder's `finish` reporting a dropped row. Names the phase and the rule,
/// **never** the text and never the JSON pointer.
ScrubRefused {
    /// `step_graph_phase.name` of the step, `<phase>:judge` for a judge, `chat` for a chat.
    phase: String,
    /// The rule name `Unmasked::rule` carried, e.g. `anthropic_api_key`.
    rule: String,
},
```

```rust
Self::ScrubRefused { phase, rule } => write!(f, "scrub refused at `{phase}`: unmasked {rule}"),
```

The `Display` sentence is fixed byte for byte as **``scrub refused at `<phase>`: unmasked <rule>``**.
It mirrors `PromptRefused`'s "prompt refused at \`{phase}\`: …" and `Unmasked`'s "unmasked {rule} at …",
without the pointer.

```rust
impl RunFailure {
    /// The one projection of an [`Unmasked`] into the run vocabulary: the rule, never the path.
    #[must_use]
    pub fn scrub_refused(phase: impl Into<String>, unmasked: &htui_core::scrub::Unmasked) -> Self {
        Self::ScrubRefused { phase: phase.into(), rule: unmasked.rule.to_owned() }
    }
}
```

It is already re-exported (`lib.rs:70-71` `pub use status::{… RunFailure …}`), so T4 reaches it as
`htui_orch::RunFailure`.

`crates/htui-orch/src/engine.rs`, a free function next to `is_fenced` (`:6427`):

```rust
/// MOD-10 D5: the text a live-step failure writes. A scrub refusal becomes
/// [`RunFailure::ScrubRefused`]'s sentence; every other error keeps its own `Display`.
fn failure_text(phase: &str, err: &EngineError) -> String {
    match err {
        EngineError::Record(htui_agent::RecordError::Unmasked(unmasked)) => {
            RunFailure::scrub_refused(phase, unmasked).to_string()
        }
        other => other.to_string(),
    }
}
```

`DriverError::Scrub` is deliberately not mapped: it is unreachable (plan, verified claims), so no
test could pin the arm.

The three sites, each with its phase in scope:

| Site | Today | After | Phase source |
|---|---|---|---|
| `walk_step` `:3422` | `self.fail_hard(run, &step, &err.to_string()).await?;` | `self.fail_hard(run, &step, &failure_text(&phase.name, &err)).await?;` | `phase: &SnapshotPhase`, a parameter of `walk_step` (`:3388`) |
| `run_candidate` `:4090` | `self.fail_candidate(stage.run, stage.phase, stage.step, &err.to_string())` | `… &failure_text(&stage.phase.name, &err))` | `CandidateStage.phase` (`:6402`) |
| `run_judge` `:4658` | `Err(err) => Err(JudgeFailure::SessionFailed(err.to_string())),` | `Err(err) => Err(JudgeFailure::SessionFailed(failure_text(&judge_phase_name(&phase.name), &err))),` | `phase` parameter (`:4579`); `judge_phase_name` is already imported (`:63`) and is the name the judge's recorder uses (`:5012`) |

The resulting texts:
- `run.failure` is `` scrub refused at `prd`: unmasked anthropic_api_key ``.
- The candidate's item note is `` fan-out candidate 0 of `research` attempt 1: scrub refused at `research`: unmasked anthropic_api_key `` (`:4325-4333`).
- The judge's `gate_note` is `` judge_session_failed: scrub refused at `research:judge`: unmasked anthropic_api_key `` (`fanout.rs:96`; `fail_judge` → `reject_step`, `:5172`).

What stays the same:
- `:4692` (a settle store error) and `:4925` (the assembler's own residue, already typed) are
  untouched.
- Every guard, cancel and fence arm runs before the helper, unchanged.
- The helper is synchronous, so no future grows.

### B.4 T4 — chat (D6): the fixed sequence

**What each store call does, with evidence:**

`fail_run(run, failure, at)`, for a chat run at `running`:
- **Postgres** (`pg/write.rs:5556-5600`): `UPDATE run SET status='failed', failure=$2, finished_at=COALESCE(finished_at,$3) WHERE id=$1 AND status IN ('queued','running','awaiting_approval')`. On 0 rows it reads the status back and returns `Constraint(illegal_move)` for a terminal run, or `NotFound`.
- **MemStore** (`mem.rs:5292-5313`): `legal_move(status, Failed)?`, then the same three columns plus `updated_at`.
- **run_step**: untouched by both stores.

`finish_chat_run(run, step, Failed, at)`:
- **Postgres** (`pg/write.rs:1837-1885`): one transaction. `UPDATE run SET status=$2, finished_at=$3 WHERE id=$1` with **no status guard**, then `UPDATE run_step SET status=$2, finished_at=$3 WHERE id=$1`. **`failure` is never touched.**
- **MemStore** (`mem.rs:2125-2158`): unconditional in the same way. It sets `status` and `finished_at` on both rows and leaves `failure` alone.
- **Schema** (`0001_init.sql:447-464`): neither store has a trigger or `CHECK` that guards `run.status` moves or ties `failure` to `status`.
- **Writer** (`htui-store/src/writer.rs:320`, `:472`, `:1112`): delegates both calls to the store as-is.

**The sequence is `fail_run` first, then the unchanged `finish_chat_run(…, Failed, …)`.** No
pre-check and no trait change. The reverse order fails: `finish_chat_run` first leaves the run
terminal, and `fail_run` then answers `Constraint` (`traits.rs:1517-1524`). The rows at the end, in
both stores:

| Row | After `fail_run` | After `finish_chat_run(Failed)` |
|---|---|---|
| `run.status` | `failed` | `failed` (rewritten, same value) |
| `run.failure` | the `ScrubRefused` sentence | unchanged, so **the sentence** |
| `run.finished_at` | `at₁` | `at₂` (overwritten by `close_run`'s `Utc::now()`) |
| `run_step.status` / `finished_at` | untouched (`running`, NULL) | `failed`, `at₂` |

`crates/htui/src/agent_worker.rs`:

```rust
/// MOD-10 D6: the `run.failure` a chat records when its recorder refused a payload, or `None`
/// for any other recorder error (which closes `failed` with no text, as before).
fn chat_failure(err: &RecordError) -> Option<String> {
    match err {
        RecordError::Unmasked(unmasked) => {
            Some(htui_orch::RunFailure::scrub_refused("chat", unmasked).to_string())
        }
        RecordError::Store(_) | RecordError::Encode(_) => None,
    }
}

impl ChatBinding {
    /// MOD-10 D6: writes `failure` onto a fresh chat's run **before** [`Self::close`]. `fail_run`
    /// refuses a terminal run, and `finish_chat_run` neither guards the status nor touches
    /// `failure`, so this order is the one in which both writes land. A promoted step's run is the
    /// engine's (blueprint D205) and is never failed from here.
    async fn record_failure(&self, writer: &Writer, failure: &str) {
        if let Self::Fresh(chat, _) = self {
            if let Err(err) = writer.fail_run(chat.run_id, failure, Utc::now()).await {
                tracing::error!(%err, run = %chat.run_id, "the chat run's failure could not be written");
            }
        }
    }
}
```

At `:4143-4146`:

```rust
if let Err(err) = recorder.finish().await {
    tracing::error!(%err, "the recorder did not close cleanly");
    status = RunStatus::Failed;
    if let Some(failure) = chat_failure(&err) {
        binding.record_failure(&writer, &failure).await;
    }
}
binding.close(&writer, status).await;
```

Notes on the change:
- `RecordError` is imported from `htui_agent::record` (the existing `use` at `:53`), or
  referenced as `htui_agent::RecordError`.
- `"chat"` is the literal the chat step is minted with (`mem.rs:2102`). No constant exists.
- If `fail_run` itself fails, the close still lands `failed` with no text, which is today's
  behaviour.
- The panic-answer path (`:3726-3740`) still closes with `finish_chat_run(Failed)`, and
  `finish_chat_run` accepts a run that `fail_run` already made terminal, so the two compose.

### B.5 T5 — `raw` re-checked at the flush (D8 as amended by A-1/A-2)

`crates/htui-agent/src/record.rs`:

1. **Counter**: the `Recorder` gains a field `raw_withheld: usize`:
   - `0` in `new` (`:518`, beside `dropped: 0`); `continuing` inherits it via `..Self::new`;
   - shown in `Debug` beside `dropped` (`:468`, counts only);
   - an accessor `pub const fn raw_withheld(&self) -> usize`, documented as "coalesced rows
     whose `raw` was withheld because their chunks, joined, held a secret no chunk held alone";
   - a matching `pub raw_withheld: usize` on `RecorderSummary` (`:293-312`), set in `finish`
     (`:1070`). The only literal construction is `:1070`, so the new field breaks no other site.
2. **Join**, a free function:
   ```rust
   /// MOD-10 D8 (A-1): every string leaf of `chunks`, concatenated **per JSON pointer** across
   /// chunks in chunk order with no separator, as one array of strings in pointer order. Object
   /// keys only form the pointer; they are never joined text.
   fn joined_raw_leaves(chunks: &[Value]) -> Value
   ```
   It uses a `BTreeMap<String, String>` keyed by RFC 6901 pointer (any injective encoding will
   do; reuse an `escape`-style `~0`/`~1`). A recursive helper `collect_leaves(value, &mut
   pointer, &mut map)` does `entry(pointer).or_default().push_str(text)` for each string. The
   output is `Value::Array(map.into_values().map(Value::String).collect())`, which does not
   depend on key order (H-11).
3. **Check**, a method:
   ```rust
   /// MOD-10 D8: a row of two or more raw chunks whose per-pointer join masking changes, or that
   /// still trips a rule, is stored with `raw = NULL`. The payload is kept. Counted and logged by
   /// rule name and cause only. **Never** `note_residue`: `raw` is opt-in debug data, and a
   /// raw-only finding must not fail the session.
   fn withhold_split_raw(&mut self, row: &mut PendingRow, seq: i32)
   ```
   - It returns early when `row.raw.len() < 2`: a single chunk was scrubbed whole at capture
     (`:1656-1661`).
   - It builds `let joined = joined_raw_leaves(&row.raw); let mut probe = joined.clone();` and
     runs `self.scrubber.scrub(&mut probe)`.
   - The cause is `"residue"` with `Some(unmasked.rule)` on `Err`, `"masked"` with `None` when
     `Ok` and `probe != joined`, and nothing otherwise. Re-masking an existing `[REDACTED]` is
     idempotent (`scrub.rs:118-140`), so already-masked chunks register no change.
   - On a cause it calls `row.raw.clear()`. Then `event_row`'s `0 => None` arm (`:1628-1630`)
     stores `raw` as SQL `NULL`, with no new representation.
   - On a cause it also bumps `self.raw_withheld += 1` and logs
     `tracing::warn!(step = %self.step, seq, chunks, cause, rule = rule.unwrap_or("none"), "a coalesced row's raw was withheld: its chunks joined hold what no chunk held alone")`.
   - The log carries no text, no pointer and no key.
4. **Call sites:**
   - In `flush` (`:1121-1133`), the `Ok(())` arm becomes `Ok(()) => { self.withhold_split_raw(&mut row, self.next_seq); row }`. `self.next_seq` is the seq the row is numbered with two lines later. The binding becomes `for mut row in pending` (it already is).
   - In `release_held` (`:1207-1214`), the `Ok(())` arm becomes `Ok(()) => { self.withhold_split_raw(&mut row, seq); row }`, because held `edit_proposal` rows accumulate raw too (`:1023-1027`).
   - A residue arm needs nothing: `residue_row` carries no raw (`:1715`).

The cost runs only for multi-chunk rows with `retain_raw` on (opt-in): one extra scrub of one
array per flushed row.

### B.6 T6 — documents (D7)

- `docs/REQUIREMENTS.md` head, after `:34`, adds a new line: `amended 2026-10-04 by maintainer decision on MOD-10 (`.claude/prds/mod-10-secret-provider.prd.md`, plan D7) — R-SEC-3 amended in place (a scrub refusal is recorded with a typed reason; "step" read as "run").`
- `docs/REQUIREMENTS.md` R-SEC-3 `:311-313`: "…and marks the step failed and blocks persistence…" becomes D7's "…and marks the run failed with a typed reason and blocks persistence…".
  - **A-5 (flagged):** a form accurate on all three paths would be "…records a typed scrub refusal on the run — failing it, or failing the fan-out candidate, or parking the judge — and blocks persistence…". Use it only if the maintainer says so.
- `prompt/trim.rs:274-285`: replace "each of which settles the run as `RunStatus::Failed` with the rendered message as an **untyped** reason" with the three real outcomes:
  - `fail_hard`: the run is `Failed`, with `RunFailure::ScrubRefused`'s sentence in `run.failure`;
  - `fail_candidate`: the candidate is `failed` with the sentence in its item note, and the run goes on;
  - `fail_judge`: the judge's `gate_note` is `judge_session_failed: <sentence>`, and the run parks with `run.failure` NULL.
  - Keep the "does not block the item" half (MOD-32 D8 stands).
  - "trips a prefix rule" becomes "trips a pattern rule".
- `docs/decisions/mod/mod-32.md`: a dated note after `:28-31`: "**Correction (2026-10-04, MOD-10 M1):** only the plain-step path settles the run `Failed`; the candidate path fails the candidate and the judge path parks. Since MOD-10 all three write `RunFailure::ScrubRefused`'s sentence."

---

## C. Data flow

1. **Capture** (unchanged): `scrub_envelope` scrubs each chunk's payload and raw. Residue →
   `refuse` → `scrub_residue` row plus `self.residue`.
2. **Flush**: the coalesced payload is re-scrubbed. Residue → `residue_row` (the whole row and
   its raw dropped). Ok → **T5** `withhold_split_raw` → the row keeps or loses `raw` → `event_row`
   → `append_events`.
3. **finish**: `Err(RecordError::Unmasked)` if any residue was noted. Raw withholds are never
   noted.
4. **Engine**:
   - `EngineError::Record(Unmasked)` comes from `finish` (`:5870`, `:4224`, `:5061`), or from
     `TrimRecord::to_value` (`:3522`, `:4183`, `:5044`) via `RecordError::from`.
   - It is caught at `:3422`, `:4090` or `:4658`, where `failure_text(phase, &err)` gives the
     `ScrubRefused` sentence.
   - The sentence is written to `run.failure`, the item note or the `gate_note` respectively.
5. **Chat**: `recorder.finish()` → `Err(Unmasked)` → `chat_failure` → `record_failure`
   (`fail_run`) → `close` (`finish_chat_run(Failed)`).
6. **M3 (future)**: `MinimalScrubber::from_resolved(&map)` gives the scrubber plus the
   short-key list. Nothing in M1 calls it outside tests.

---

## D. Tests, per task (written first; each must fail before its implementation)

### D.1 T1 (`scrub.rs` `mod tests`)
- `every_pattern_rule_compiles_alone`: each `PATTERN_RULES` body compiles as a `Regex` with
  `TOKEN_START`.
- `each_rule_refuses_a_real_shaped_key_in_four_positions`: for every rule, one real-shaped value:
  - at the string start, after a space, in `Bearer <key>`: each `Err` with that rule name and
    pointer `/payload/output`;
  - as an object key: `Err` at the **parent** pointer.
- `prose_that_shares_a_prefix_is_not_a_credential`: `sk-learn`,
  `sk-learn-preprocessing-pipeline-v2`, `AKIA`, `subtask-x`, `ghp_short`, `src/sk-live.rs`,
  `task-list, subtask-42, whisk-broom` all return `Ok`.
- `a_short_sk_ant_key_is_never_reported_as_openai`: `sk-ant-` plus 20 or more characters gives
  `anthropic_api_key`; `sk-ant-short` is `Ok`.
- `the_first_rule_in_table_order_is_reported`: a string holding an AWS key then an Anthropic key
  reports `anthropic_api_key` (table order, A-6).
- `a_non_ascii_letter_before_a_key_is_a_token_start`: `éAKIA0123456789ABCDEF` refuses (D1,
  accepted).
- Existing tests are kept, with fixtures lengthened (A-4). `unmasked_never_repeats_the_offending_text`
  and the key variant are unchanged.

### D.2 T2 (`scrub.rs` `mod tests`)
- `from_resolved_masks_values_at_the_floor`: `{"K":"abcdef"}` masks to `[REDACTED]`.
- `from_resolved_lists_the_keys_below_the_floor`: `{"A":"abcde","B":"longvalue"}` returns
  `["A"]`; `abcde` is not masked; `longvalue` is.
- `from_resolved_returns_key_names_never_values`: the list contains no value, and neither does the
  scrubber's `Debug`.
- `from_resolved_counts_only_in_debug`: `Debug` shows `secrets: 1`.
- `from_resolved_accepts_an_empty_map_and_still_fails_closed`: an empty map gives `(scrubber, [])`,
  and AWS-shaped text still refuses.
- `the_floor_counts_characters_not_bytes`: `"ééééé"` (5 chars, 10 bytes) is listed.

### D.3 T3

**`status.rs`:**
- In `run_failure_display_is_ana2s_bytes`, a row: `ScrubRefused { phase: "implement", rule: "anthropic_api_key" }` gives `` scrub refused at `implement`: unmasked anthropic_api_key ``.
- `scrub_refused_never_carries_the_pointer`: `RunFailure::scrub_refused("prd", &Unmasked { path: "/payload/sk-secret".into(), rule: "aws_access_key_id" })`; the `Display` and `Debug` contain neither `"/payload"` nor `"sk-secret"`.

**`engine.rs` tests**, beside `parks_with_residue` (`:16734`):
- Local helper `fn leaks(body: &str) -> ScriptedStep`, built from `ScriptedStep`'s pub fields (`fake.rs:1072-1080`) with no `fake.rs` change:
  - `script: Script::one_turn(vec![ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk { text: "the key is sk-ant-api03-abcdefghijklmnopqrstuvwx".into(), message_id: None })), ScriptEvent::Emit(DriverEvent::Done(DoneEvent { stop_reason: StopReason::EndTurn }))])`;
  - `output: Some(body.into())`, `spawn_failure: None`.
  - The key matches under both the old and the new rules (H-7).
- `Harness::add_primary_repo_named(&self, name: &str) -> RepoId`, beside `add_primary_repo` (`:7087`).
- `a_recorder_refusal_fails_the_run_with_the_scrub_sentence`:
  - setup: `feat_3_with_prd_ungated`, `script("prd", 1, leaks("the prd"))`, then dispatch;
  - the walk answers `Err(EngineError::Record(_))`;
  - `run.status == Failed`, and `run.failure == Some(RunFailure::ScrubRefused{phase:"prd",rule:"anthropic_api_key"}.to_string())`;
  - the `prd` step is `Failed`; a `scrub_residue` row exists (`assert_residue_recorded`);
  - the item is **not** `Blocked`; it follows `finish_run`'s `failed` mirror.
- `a_trim_record_refusal_fails_the_run_with_the_scrub_sentence`:
  - setup: `free_feat_3`, then `add_primary_repo_named("sk-ant-api03-abcdefghijklmnopqrstuvwx")`. The slug reaches `excerpts.roots[].repo` on every stage 3 (`engine.rs:5466-5482`), which is not rendered text; `prompt_digest.rs:1495-1515` is the core-level proof;
  - the run is `Failed`, with `run.failure` equal to the sentence for `prd`;
  - `trim_record` is NULL on the step;
  - **Precondition assert (H-15):** `!failure.starts_with("prompt refused")`.
- `failure_text_types_only_a_scrub_refusal` (sync `#[test]`):
  - `EngineError::Record(RecordError::Unmasked(..))` gives the sentence;
  - `EngineError::Record(RecordError::Encode("x".into()))` and `EngineError::Stalled{..}` give their own `to_string()`.
- The three existing residue tests (`:16754`, `:16790`, `:16844`) stay **unchanged** and green.

**`conformance.rs` `mod fanout_paths`**, with its own `leaks(body)` copy (per-file rule) and imports added to its `use super::{…}`:
- `a_candidate_whose_recorder_refuses_fails_with_the_scrub_sentence`:
  - setup: `fan_research(&orch, Gate::Never, false)`, `research_candidates(&orch, 1)`, then `orch.script_candidate("research", 1, 0, 0, leaks(...))` to override candidate 0;
  - candidate 0 is `Failed`;
  - `notes_of(item)` contains `` fan-out candidate 0 of `research` attempt 1: scrub refused at `research`: unmasked anthropic_api_key ``;
  - the run's outcome is whatever the sibling path gives today (assert only that `run.failure` does not hold the sentence: the run is not failed by it).
- `a_judge_whose_recorder_refuses_parks_with_the_scrub_sentence`:
  - setup: `fan_research`, `set_judge`, `research_candidates(&orch, 1)`;
  - call 0 is `script_candidate("research:judge", 1, -1, 0, leaks(verdict))`, where `verdict` is the `{"winner": 1, …}` block (`:7776`);
  - call 1 is `ScriptedStep::judge(1, &[])`. **Both calls must succeed**: `judge_sessions` reads `finished?` only then (`:5061-5065`, plan risk);
  - `rest.run == AwaitingApproval`, and `run.failure` is `None`;
  - the judge is `Failed`, with `gate_note == Some("judge_session_failed: scrub refused at `research:judge`: unmasked anthropic_api_key")`.

### D.4 T4 (`agent_worker.rs` `mod tests`)
- `a_chat_whose_recorder_refuses_fails_its_run_with_the_scrub_sentence`:
  - setup: `fixture(Script::one_turn(vec![Emit(AssistantChunk(TextChunk{ text: "sk-ant-api03-abcdefghijklmnopqrstuvwx", message_id: Some("m1") })), Emit(Done{EndTurn})]))`, then `run(&mut runtime, &backend, start(agent_id, "leak"))`, which queues the cancel (`:4965-4991`);
  - read via `run_of(step_id)` (`:10725`) and `store.run(..)`;
  - the run is `failed`, with `failure == Some(htui_orch::RunFailure::ScrubRefused{phase:"chat".into(), rule:"anthropic_api_key".into()}.to_string())` and `finished_at` set;
  - the step (`store.run_steps(run)`) is `Failed` with `finished_at` set;
  - the serialised `step_events` holds no `sk-ant-`.
- `chat_failure_types_only_unmasked` (sync): `Store` and `Encode` give `None`.
- A promoted-binding case is **not** added: `record_failure` is a no-op there by construction.
  The existing promoted tests prove the run is untouched.

### D.5 T5

**`tests/recorder.rs`**, all with `retain_raw = true`, using a local `acp_chunk(text, meta: Option<&str>) -> DriverEnvelope`:
- The helper's raw is the full wire shape: `{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":<text>}}},"_meta":{"trace":<meta>}}`.
- That shape is what proves A-1: the protocol leaves sit between the halves.

The tests:
1. `a_known_secret_split_across_chunks_withholds_raw`:
   - scrubber `MinimalScrubber::new(["hunter2hunter2"])`; chunks `"pass hunter2"` and `"hunter2 ok"`;
   - one `assistant_text` row whose payload `text == "pass [REDACTED] ok"` and `raw == None`;
   - `recorder.raw_withheld() == 1` before `finish`, and `summary.raw_withheld == 1`;
   - the log JSON never contains `hunter2`.
2. `a_pattern_credential_split_in_a_raw_only_leaf_withholds_raw` (A-2):
   - the text is clean (`"hello "`, `"world"`); `_meta.trace` is `"s"` then `"k-ant-api03-abcdefghijklmnopqrstuvwx"`;
   - the halves are clean under **both** rule sets, and the join is caught by both (H-7);
   - the payload is `"hello world"`, `raw == None`, and `finish()` is **`Ok`**: no `scrub_residue` row and no session failure.
   - 2b. `a_pattern_credential_split_in_the_payload_is_still_a_residue_row`: the text halves `"s"` and `"k-ant-api03-…"` give one `error` / `scrub_residue` row in place, with no raw. This pins existing behaviour.
3. `a_clean_coalesced_row_keeps_its_raw_array`: two clean chunks give `raw` as a 2-element array, equal to the two scrubbed wire messages; `raw_withheld == 0`.
4. `a_single_chunk_row_keeps_its_raw_object`: one chunk gives `raw` as that object, not an array.
5. `a_split_edit_proposal_raw_is_withheld_too`:
   - two announcements of one `(tool_call_id, path)` whose `raw` `wire` halves join to `hunter2hunter2`, then a `tool_result`;
   - the held row is released with `raw == None`. This covers the `release_held` site.

**`record.rs` `mod tests`:**
- `joined_raw_leaves_joins_per_pointer_in_chunk_order_and_excludes_keys`:
  - chunks `{"a":"x1","b":{"c":"y1"}}` and `{"b":{"c":"y2"},"a":"x2","k-key":"z"}` give the array `["x1x2", "y1y2", "z"]` in pointer order;
  - no key text appears in any element.

### D.6 T6
- `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` is green.

---

## E. Hazards

- **H-1 Stack headroom (`htui-orch`).**
  - `every_case_name_dispatches` (`conformance.rs:7334`) runs every `CASES` entry in one future near the 2 MiB test stack.
  - The new conformance tests go in `mod fanout_paths`, **never** in `CASES`.
  - `failure_text` is sync and adds nothing to engine futures.
  - Gate: `cargo test -p htui-orch --features testkit --no-fail-fast 2>&1 | tee /tmp/orch.log; grep -c SIGABRT /tmp/orch.log` must be 0. If a new test overflows, `Box::pin` its dispatch future rather than raising the stack.
- **H-2 `--features testkit`.** Without it, `crates/*/tests/*.rs` integration binaries run 0 tests and still say `ok` (`recorder.rs`, `excerpt.rs`, the `htui-orch` suites). Every validate command carries it.
- **H-3 `Cargo.lock`.** Only T1 touches it, and the only change is `htui-core`'s dependency list. Build with `--offline`; never `cargo update`. Lanes B and C must not commit a `Cargo.lock` change. If they see one, it is lane A's leaking through a shared tree.
- **H-4 Clippy `-D warnings`.**
  - New `pub` items need docs (`missing_docs`): `MIN_MASKED_LEN`, `from_resolved`, the `ScrubRefused` fields, `scrub_refused`, `raw_withheld` (both the field and the accessor).
  - `unused_qualifications`: use imported names rather than full paths where the `use` exists.
  - `RegexSet::new(...).expect(...)` is allowed (no `expect_used`; pedantic is off).
  - `clippy::all` includes `invalid_regex`, which only lints literals passed straight to `Regex::new`, not the `format!`-built set.
- **H-5 Regex cost in the hot path.**
  - `residue_rule` runs on every string leaf and key of every row and record.
  - Compile once (`static LazyLock`), and never build a `Regex` or `RegexSet` per call.
  - Use the `is_match` gate first: `matches()` scans for all patterns, while `is_match` can stop early, and clean leaves dominate.
  - The counted repetitions (`{50,}`, `{43}`, `{36}`) stay well under the default `size_limit` (plan compile probe).
  - The tests are not perf tests. Do not add a timing assert.
- **H-6 `scrub.rs` own fixtures** (A-4): T1's red step will include these existing tests going red for the wrong reason. Lengthen them in the same commit as the rule swap.
- **H-7 Dual-valid fixtures across lanes.**
  - Lanes B and C run before lane A lands, so every credential fixture they write must refuse under **both** the bare-prefix rules and D2:
    - `sk-ant-api03-abcdefghijklmnopqrstuvwx` (30 chars after `sk-ant-`);
    - lengthened `sk-live0123456789abcdefghij…`;
    - a split at the **first** character (`"s"` + `"k-ant-…"`, or `"AK"` + `"IA…"`), so neither half is a token-start prefix under either set.
  - Every split half must be clean under both.
  - Re-run lanes B and C on the merged tree.
- **H-8 Promoted chat.** `ChatBinding::Promoted`'s run is the engine's (`awaiting_approval`, promoted). `record_failure` must not call `fail_run` for it; a `fail_run` there would end an engine-owned graph run.
- **H-9 D6 order.** `fail_run` must come **before** `close`. The reverse order returns `Constraint` (`traits.rs:1517-1524`), which is logged and loses the text silently. The T4 test pins the text.
- **H-10 D8 must not fail the session.** `withhold_split_raw` never calls `note_residue` and never pushes a `residue_row`. Its log carries the rule name and the cause only, never the joined text, a pointer or a key. D.5 test 2 asserts that `finish()` returns `Ok`.
- **H-11 `serde_json/preserve_order` unification.** Key order differs between `-p htui-agent` and `--workspace` builds (`trim.rs:198-204`). The per-pointer join's output is ordered by pointer (`BTreeMap`), so tests must not assert an order that depends on insertion.
- **H-12 Byte-exact sentences.**
  - Tests compare whole strings built from `RunFailure::ScrubRefused{..}.to_string()`, never a re-typed literal, except the one `status.rs` row that pins the bytes.
  - The judge's `gate_note` carries the `judge_session_failed: ` prefix (`fanout.rs:96`).
  - The candidate's note carries the `fan-out candidate … : ` prefix.
- **H-13 Item status.**
  - `fail_hard`'s `finish_run(Failed)` mirrors the item `in_progress → failed`. That is not `blocked` (MOD-32 D8 is unchanged).
  - Assert `!= Blocked`, not a new status.
- **H-14 `insta`.** There are no snapshot changes. If `cargo insta` reports a pending `.snap.new`, a fixture leaked into rendered output: investigate it, do not accept it.
- **H-15 Trim-path fixture precondition.**
  - If the credential repo slug reaches a rendered section, the assembler refuses first. The step then ends `PromptRefused` and the item is blocked, a different path.
  - The test's precondition assert catches that.
  - **VERIFY — implementer must check.** If it fires, use a credential-shaped `excerpts.provider_set` entry or a trim note path that `step_pass` does not withhold, and record the switch in the commit.
- **H-16 No store or SQL change.** D6 needs none (§B.4). An implementer who reaches for `traits.rs`, `pg/write.rs` or `.sqlx` has left the plan.
- **H-17 Suite scheduling.** Verify the merged tree with `--test-threads=1`. The keyring fake is process-wide, and parallel green is scheduling-dependent.
- **H-18 Worktrees.** If lanes run in linked worktrees:
  - Gortex `edit` writes to the **primary** checkout, so edit with file tools inside the worktree;
  - give each worktree its own `CARGO_TARGET_DIR` (about 10 GB each), and watch `df -h .`.

**18 hazards.**

---

## F. Build sequence and lanes

The plan's split is **confirmed**, with A-3 (`conformance.rs` joins lane B):

| Lane | Order | Tasks | Touched files |
|---|---|---|---|
| **A** | T1 → T2 | T1 | `crates/htui-core/Cargo.toml`, `Cargo.lock`, `crates/htui-core/src/scrub.rs`, `crates/htui-core/src/prompt/mod.rs`, `crates/htui-core/tests/prompt_digest.rs`, `crates/htui-core/tests/prompt_persona.rs`, `crates/htui-agent/tests/excerpt.rs`, `crates/htui-orch/src/verify.rs` |
| | | T2 | `crates/htui-core/src/scrub.rs` |
| **B** | T3 → T4 | T3 | `crates/htui-orch/src/status.rs`, `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/conformance.rs` |
| | | T4 | `crates/htui/src/agent_worker.rs` |
| **C** | T5 | T5 | `crates/htui-agent/src/record.rs`, `crates/htui-agent/tests/recorder.rs` |
| docs | after A–C | T6 | `crates/htui-core/src/prompt/trim.rs`, `docs/REQUIREMENTS.md`, `docs/decisions/mod/mod-32.md` |

Lane intersections:
- **Files:** A ∩ B, A ∩ C, B ∩ C, and each lane ∩ T6 are all ∅.
- **Crates:** A (`verify.rs`) and B share `htui-orch` but not a file. A (`excerpt.rs`) and C share `htui-agent`'s tests directory but not a test binary.
- **Hidden coupling:**
  - the rule semantics: lanes B and C use dual-valid fixtures (H-7);
  - `Cargo.lock` belongs to A only (H-3);
  - there are no `.sqlx` files and no snapshots.

Within each lane:
1. **T1**: tests (D.1) red → `Cargo.toml`/`Cargo.lock` → rule table, `LazyLock`, `residue_rule` → fixtures (§B.1) → module docs → `cargo test -p htui-core`, `-p htui-agent --features testkit --test excerpt`, `-p htui-orch --features testkit verify`. Commit.
2. **T2**: tests (D.2) red → `MIN_MASKED_LEN` and `from_resolved` → `cargo test -p htui-core scrub`. Commit.
3. **T3**: `status.rs` row red → variant, `Display`, constructor → engine and conformance tests red → `failure_text` and the three sites → `:14944`/`:14975` fixtures → `cargo test -p htui-orch --features testkit --no-fail-fast` and the SIGABRT grep. Commit.
4. **T4** (needs T3's variant): test red → `chat_failure`, `record_failure`, the `:4143` arm → `cargo test -p htui --features testkit agent_worker`. Commit.
5. **T5**: unit and integration tests red → counter → `joined_raw_leaves` → `withhold_split_raw` → both call sites → `cargo test -p htui-agent --features testkit --test recorder` plus `cargo test -p htui-agent --lib record`. Commit.
6. **T6**: docs → `validate-workflow-docs.sh`. Commit.
7. **Merged tree**:
   - `cargo fmt --all -- --check`;
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
   - `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1`, with the SIGABRT grep;
   - `validate-workflow-docs.sh`.
