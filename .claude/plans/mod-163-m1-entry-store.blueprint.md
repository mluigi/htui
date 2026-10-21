# Blueprint: MOD-163 M1 — Entry store (T1–T4)

> Produced by `code-architect` 2026-10-21 from the confirmed plan
> `.claude/plans/mod-163-m1-entry-store.plan.md` (D1–D10, Verified claims C1–C12) and the PRD
> `.claude/prds/mod-163-memory-entry-store.prd.md`. Tree read at `d751a1e1` (branch `hr/MOD-163`).
> The plan stays the spec. §0 lists where the tree or a probe disagrees with it. Every row is
> **adopted**: each one is implementation detail, or the only reading of a decision that works. None
> reopens D1–D10. The 0020 SQL in §T3.1 was applied to a scratch database (`mod163_probe*` on the
> sandbox's `localhost:5439`, PostgreSQL 16, since dropped) on top of `0001..0019`. The probe
> outcomes are in §T3.2.

## 0. Plan deltas

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| P-1 | D5 orders the text checks as trim, empty, **cap, then scrub**. Masking lengthens text: a secret becomes `[REDACTED]` (10 chars), so a 6-character secret grows by 4. A subject capped at 120 *before* masking can be 120+ after it and then hits the DB CHECK as a raw `memory_entry_subject_check` Constraint. The plan's own Patterns row says "Scrub, then cap, then write", which is `note.rs`'s order. | `scrub.rs:22` (`REDACTED`), `:230-242` (any non-empty secret is masked); `htui-mcp/src/tools/note.rs:56-62` (scrub, then cap); plan "Patterns to Mirror" Scrub row | **adopted**: trim → `Empty` → scrub (`Unmasked` refuses) → cap the **scrubbed** text in chars → `TooLong { max }`. A T1 unit test pins it (`masking_that_overflows_the_cap_is_too_long`). |
| P-2 | D1 makes `superseded_by` an immediate FK, and D6 writes "the old entry moves to `superseded` with `superseded_by = new.id` … **then** the new entry is created". An immediate FK refuses the old row's UPDATE, because `new.id` does not exist yet. Inserting the new row first instead trips `uq_memory_entry_live_subject` when the subject is reused (D4 Replace, and D6's "new entry may reuse the old subject"). | Probe §T3.2 row 7: immediate FK, separate statements → `23503` on the UPDATE | **adopted**: `CONSTRAINT fk_memory_entry_superseded_by … ON DELETE SET NULL DEFERRABLE INITIALLY DEFERRED`. The two statements run in D6's order and the FK is checked at COMMIT (probed, rows 3–4). |
| P-3 | D1's "a CHECK ties the two". A **two-way** tie `(state='superseded') = (superseded_by IS NOT NULL)` contradicts D1's own `ON DELETE SET NULL`: deleting a successor would leave a superseded row with NULL, and the CHECK refuses that delete. | Probe §T3.2 row 8: two-way CHECK, delete successor → `23514`; one-way → allowed, state stays `superseded` | **adopted**: one-way `chk_memory_entry_superseded_by CHECK (superseded_by IS NULL OR (state = 'superseded' AND superseded_by <> id))`, which is "set only together with `state = 'superseded'`". `MemoryEntry::superseded_by` documents that `None` is possible on a superseded entry whose successor is gone. In M1 that state is unreachable: there is no entry delete, and a project delete takes both rows. |
| P-4 | D4's helper "names the existing entry's id". Postgres's `23505` carries only the key `(project_id, lower(btrim(subject)))=(…, foo)` in DETAIL, not the clashing row's id. | Probe §T3.2 row 1 | **adopted**: PgStore maps the index's `23505` by **rolling back, then running one lookup** for the live holder of the folded subject (§T3.4 `memory_subject_refused`). `memory_subject_taken(subject, existing)` is given the **holder's stored subject** on both stores, so the string is byte-identical across backends and the conformance case can compare it exactly. |
| P-5 | `crates/htui-core/src/store/mod.rs` is missing from Files to Change. Refusal helpers reach other crates through its `pub use traits::{…}` list (the `htui` crate imports `requirement_withdrawn` that way). | `store/mod.rs` `pub use traits::{…}`; `htui/src/requirements.rs:61-64` | **adopted**: T1 adds `memory_entry_closed, memory_subject_taken` to that list. |
| P-6 | `pg_criteria.rs`'s `TABLES` gains `memory_entry` / `memory_entry_revision` in T1, but those tables only exist from T3's migration. Between T1 and T3, `every_cascade_table_loses_exactly_what_the_report_names` panics on `count(memory_entry)` (relation does not exist). The case also says a seeded table must not pass on `0 == 0`, and nothing seeds memory rows. | `pg_criteria.rs:3178-3187, 3294-3316`; plan T1 Validate (compile only) | **adopted**: T1 makes the plan's edit, so the file compiles and the Pg case is red until T3. **T3 also edits `pg_criteria.rs`**: it seeds one entry with one amend (1 entry, 2 revisions) before the delete and adds `(memory_entries, memory_revisions) == (1, 2)` to the "not `0 == 0`" assertion. T3's file set grows by `pg_criteria.rs`, which T2 never touches, so T2 ∩ T3 is still empty. T3 runs the case. |
| P-7 | T2's validate commands run **zero tests**. `htui-core` has no self dev-dependency enabling `test-support`, `tests/mem_store.rs` is `#![cfg(feature = "test-support")]`, and `store::conformance` is `#[cfg(feature = "test-support")]`. Without the feature, `cargo test -p htui-core --test mem_store` reports `ok` with 0 tests. | `htui-core/Cargo.toml` `[features]`/`[dev-dependencies]`; `mem_store.rs:6`; `store/mod.rs` `#[cfg(feature = "test-support")] pub mod conformance` | **adopted**: every `-p htui-core` command that touches the suite carries `--features test-support` (§T2.6). |
| P-8 | `store/conformance.rs` has a self-scanning test, `every_cross_referenced_test_name_exists` (`:21354`). Any **backticked** snake_case token with ≥4 underscores, anywhere in that file (docs, comments, panic strings), must be a fn defined in `conformance.rs` or `mem.rs`. The index name `uq_memory_entry_live_subject` has four underscores. | `conformance.rs:21388-21453` | **adopted** (gotcha): never backtick that name in `conformance.rs`. Write "the live-subject index" instead. `mem.rs` is not scanned for this. |
| P-9 | D2 says `NewMemoryEntry` and `MemoryPatch` do not derive `Deserialize`. `MemoryText` follows `FollowUpText`, which has no serde at all (I-5), so neither struct can derive `Serialize` either. | `model/relay.rs:238-242, 296-308` (`NewFollowUp`: `Debug, Clone, PartialEq, Eq`) | **adopted**: `NewMemoryEntry`/`MemoryPatch` derive `Debug, Clone, PartialEq, Eq` (+`Default` on the patch). `StoreRequest` is `Debug, Clone` only (C4). |
| P-10 | D1 lists two indexes. `ON DELETE SET NULL` on the self-FK scans `memory_entry` for referencing rows on every deleted entry, and a project cascade deletes all of them. Without an index that is quadratic. | `0006_requirements.sql:108-109` (`idx_requirement_revision_amended_by`, same reason) | **adopted**: a third, partial index `idx_memory_entry_superseded_by … WHERE superseded_by IS NOT NULL`. |
| P-11 | Two small model helpers are needed by T2 (Mem) and T3 (Pg docs), and T2/T3 may not touch the model. | plan Risks row 1 | **adopted**: T1 ships `MemoryState::is_live()` and `memory_subject_key(&str) -> String` (Postgres's `lower(btrim())` in Rust). M3 can use the same fold to find the clashing entry in its loaded list instead of parsing the refusal string. |
| P-12 | D5 lists three `MemoryTextError` variants. Postgres `text` cannot hold U+0000 (`22021`), so without a construction-time check `MemStore` would keep a text that `PgStore` fails with as a backend error, and the two stores would disagree. | Postgres `text` type (`22021 character_not_in_repertoire` on a NUL); commit `edf56188` | **adopted**: a fourth variant, `MemoryTextError::Nul`, checked in `checked` after `Empty` and before the scrub. Unit test `a_nul_is_refused`. M3's error-to-message mapping and T2/T3 expect **four** variants: `Empty`, `Nul`, `TooLong { max }`, `Unmasked`. |
| P-13 | Review R1 finding 1. A single `MemoryText` did not record which cap built it, so a body-capped value (up to 2000 chars) type-checked as `NewMemoryEntry.subject` / `MemoryPatch.subject`: `MemStore` kept a 121..=2000-char subject that `PgStore` refused with the raw `memory_entry_subject_check` text, and the two stores disagreed. | rust-reviewer R1 F1 (verified); `pg_criteria.rs` `memory_text()` built subjects with `MemoryText::body` | **adopted**: two newtypes, `MemorySubject::new` (cap 120) and `MemoryBody::new` (cap 2000), both built through the one private `checked` path (trim, `Empty`, `Nul`, scrub, cap the scrubbed text); no serde, `Debug` prints `MemorySubject { len }` / `MemoryBody { len }`. `MemoryTextError` keeps its name and four variants. `NewMemoryEntry { subject: MemorySubject, body: MemoryBody }`, `MemoryPatch { subject: Option<MemorySubject>, body: Option<MemoryBody> }`. `compile_fail` doctests on both types pin that neither stands for the other; unit test `a_body_length_text_is_not_a_subject`. The T1.1 code block above still shows `MemoryText`; this row supersedes it. The `0020_memory.sql` header comment still says `MemoryText` and stays (a migration's bytes are checksummed). |
| P-14 | Review R1 finding 2. The conformance cases did not pin a supersede whose `new.id` is the old entry's own id (Pg refuses it through `chk_memory_entry_superseded_by`, Mem through the duplicate-id check) or a third entry's id, nor a supersede/amend/retire with an unknown `created_by` or `box_id`; the create duplicate-id case matched `Constraint(_)` only. | rust-reviewer R1 F2 (verified) | **adopted** (tests only): `memory_supersede_refuses_stale_closed_and_clashing` asserts `already_exists("memory_entry", old.id)` and `already_exists("memory_entry", third.id)` exactly, and `Constraint(_)` for an unknown `created_by` / `box_id`; after each, the old entry's history, the third entry's history and the active list are unchanged and no would-be new id has a revision (`memory_supersede_wrote_nothing`). `memory_unknown_rows_and_authors_are_refused` asserts the create duplicate id as `already_exists` exactly and adds amend and retire on an unknown `box_id` (`Constraint(_)`: the FK text differs per store). Both stores already passed; no store code moved. |

---

## 1. Shared decisions for every task

- **Names.** Table `memory_entry` / `memory_entry_revision`. Rust `MemoryEntry` / `MemoryRevision`.
  Trait reads `memory_entries`, `memory_revisions`. `DeleteReach` fields `memory_entries`,
  `memory_revisions` (D8). `NotFound` entities: `"project"`, `"memory_entry"`,
  `"memory_entry_revision"` (id `"{id}@{expected_version}"`, as `requirement_revision`'s,
  `mem.rs:7221-7224`).
- **Decision order** (C9), on every writer: NotFound → divergence (`Diverged`, nothing written) →
  Constraint. Inside Constraint, Mem checks closed (`memory_entry_closed`), then duplicate id
  (`already_exists("memory_entry", id)`), then subject clash (`memory_subject_taken`), then
  `created_by`/`author_id` (`require_user`), then `box_id`. Pg decides closed under the row lock and
  the rest come out of the statement's `23xxx`. No case combines two of them.
- **Clock.** Mem passes `self.now()` into `State` (`mem.rs:9161-9171`). Pg: `confirmed_at`,
  `created_at` and the revision's `created_at` are `now()` (transaction start). `updated_at` on
  UPDATE is the trigger's `clock_timestamp()` (`0001_init.sql:21-25`). A create therefore has
  `confirmed_at == created_at == updated_at` on both stores. An amend has `confirmed_at` strictly
  after the previous one. Retire and supersede leave `confirmed_at` alone (D3).
- **Who writes the old entry's revision on a supersede:** `new.created_by` / `new.box_id`. The
  verb has no separate author argument (D6's signature).
- **List order across one supersede is not asserted.** In Pg the old row's `updated_at`
  (`clock_timestamp()`) is later than the new row's (`now()`). In Mem both are `now` and `id`
  breaks the tie. Conformance asserts order only across separate calls (§T2.3 case 2).
- **State filter:** `None` = every state; `Some(vec![])` = no rows on both stores (`= ANY('{}')` is
  false; Mem `contains` is false).

---

## T1 — Types, verbs and the compile-green skeleton (serial)

### T1.1 `crates/htui-core/src/model/memory.rs` (CREATE)

Mirrors `model/requirement.rs:12-30` (enums), `:105-136` (row), `:146-180` (New/Patch),
`:182-208` (revision) and `:279-291` (Update). `MemoryText` mirrors `relay.rs:238-293`
(`FollowUpText`) and `htui/src/requirements.rs:69-94` (length-only `Debug`).

```rust
//! Project memory entries (MOD-163 M1, plan D1-D6; `R-ENT-13`, ANA-38 §4.6): the row, its
//! revisions, the write arguments, and [`MemoryText`], the only way prose reaches the store
//! (`R-ID-7`). Not mirrored (plan D7).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::ids::{BoxId, MemoryEntryId, ProjectId, UserId};
use crate::scrub::{Scrubber, Unmasked};

/// `memory_entry.subject`'s cap in characters (`char_length`), the CHECK's upper bound (plan D5).
pub const MEMORY_SUBJECT_MAX: usize = 120;

/// `memory_entry.body`'s cap in characters (plan D5).
pub const MEMORY_BODY_MAX: usize = 2000;

str_enum!(
    /// `memory_entry.kind` (MOD-163 plan D1).
    MemoryKind {
        /// A standing fact about the project.
        Fact => "fact",
        /// A decision taken, and why.
        Decision => "decision",
        /// A trap and how to step around it.
        Gotcha => "gotcha",
        /// What a failure taught, and the fix.
        Lesson => "lesson",
    }
);

str_enum!(
    /// `memory_entry.state` (MOD-163 plan D1, D4). `active` and `suspect` are **live**: at most one
    /// live entry per folded subject per project. `superseded` and `retired` are closed: no amend,
    /// retire or supersede. Nothing in M1 sets `suspect` (MOD-166 does).
    MemoryState {
        /// In force.
        Active => "active",
        /// In force but doubted (MOD-166's freshness sweep); never written in M1.
        Suspect => "suspect",
        /// Replaced by `superseded_by` (plan D6).
        Superseded => "superseded",
        /// Retired by a person.
        Retired => "retired",
    }
);

impl MemoryState {
    /// Plan D4: `active | suspect`, the states the live-subject index covers and the only ones a
    /// writer moves.
    #[must_use]
    pub const fn is_live(self) -> bool { /* matches!(self, Self::Active | Self::Suspect) */ }
}

/// Plan D4: the fold `uq_memory_entry_live_subject` compares by, Postgres's
/// `lower(btrim(subject))`. `btrim` strips spaces only, so this is `trim_matches(' ')` then
/// `to_lowercase()`, not `trim()`. The two agree on ASCII. A non-ASCII subject can fold differently
/// in `MemStore` and Postgres, as [`RequirementFilter::text`] warns.
///
/// [`RequirementFilter::text`]: crate::model::requirement::RequirementFilter::text
#[must_use]
pub fn memory_subject_key(subject: &str) -> String;

/// A row of `memory_entry` (plan D1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// `memory_entry.id`, client-minted (UUIDv7).
    pub id: MemoryEntryId,
    /// `memory_entry.project_id`.
    pub project_id: ProjectId,
    /// `memory_entry.kind`.
    pub kind: MemoryKind,
    /// `memory_entry.subject`: trimmed, scrubbed, 1..=[`MEMORY_SUBJECT_MAX`] characters.
    pub subject: String,
    /// `memory_entry.body`: trimmed, scrubbed, 1..=[`MEMORY_BODY_MAX`] characters.
    pub body: String,
    /// `memory_entry.state`.
    pub state: MemoryState,
    /// `memory_entry.version`: the compare-and-set token (plan D3, ANA-9 §4.2).
    pub version: i32,
    /// `memory_entry.confirmed_at`: when the text was last written or confirmed (plan D3). Create
    /// and amend set it; retire and supersede leave it.
    pub confirmed_at: DateTime<Utc>,
    /// `memory_entry.superseded_by`: the entry that replaced this one. `Some` only on a
    /// `superseded` entry. `None` on a superseded entry whose successor was deleted
    /// (`ON DELETE SET NULL`), which M1 cannot reach.
    pub superseded_by: Option<MemoryEntryId>,
    /// `memory_entry.created_by` (a person in M1, plan D9).
    pub created_by: UserId,
    /// `memory_entry.created_at`.
    pub created_at: DateTime<Utc>,
    /// `memory_entry.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// A row of `memory_entry_revision`: the entry as it stood at `version`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRevision {
    /// `memory_entry_revision.entry_id`.
    pub entry_id: MemoryEntryId,
    /// `memory_entry_revision.version`.
    pub version: i32,
    /// `memory_entry_revision.kind`.
    pub kind: MemoryKind,
    /// `memory_entry_revision.subject`.
    pub subject: String,
    /// `memory_entry_revision.body`.
    pub body: String,
    /// `memory_entry_revision.state`.
    pub state: MemoryState,
    /// `memory_entry_revision.author_id`.
    pub author_id: UserId,
    /// `memory_entry_revision.box_id`.
    pub box_id: Option<BoxId>,
    /// `memory_entry_revision.reason`: `created`, `amended`, `retired`, `superseded` (no CHECK,
    /// like `requirement_revision.reason`).
    pub reason: String,
    /// `memory_entry_revision.created_at`.
    pub created_at: DateTime<Utc>,
}

/// Arguments of [`WriteStore::create_memory_entry`](crate::store::WriteStore::create_memory_entry)
/// and the new half of
/// [`WriteStore::supersede_memory_entry`](crate::store::WriteStore::supersede_memory_entry). No
/// serde (plan D2, blueprint P-9): the text is a [`MemoryText`], so nothing builds one by decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMemoryEntry {
    /// Client-minted id (`MemoryEntryId::new()`).
    pub id: MemoryEntryId,
    /// The kind.
    pub kind: MemoryKind,
    /// The checked subject.
    pub subject: MemoryText,
    /// The checked body.
    pub body: MemoryText,
    /// `memory_entry.created_by`, and the `author_id` of revision 1 (and of the old entry's
    /// `superseded` revision on a supersede).
    pub created_by: UserId,
    /// `box_id` of those revisions.
    pub box_id: Option<BoxId>,
}

/// The edit of [`WriteStore::amend_memory_entry`](crate::store::WriteStore::amend_memory_entry):
/// `None` leaves a column as it is. All three `None` re-confirms the entry (a new revision and
/// `confirmed_at` = now, plan D3). The revision's reason is always `amended`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryPatch {
    /// New kind.
    pub kind: Option<MemoryKind>,
    /// New subject.
    pub subject: Option<MemoryText>,
    /// New body.
    pub body: Option<MemoryText>,
    /// The revision's `author_id`.
    pub author_id: UserId,
    /// The revision's `box_id`.
    pub box_id: Option<BoxId>,
}

/// Result of an amend or a retire (plan D6): the edit landed, or someone else committed first.
#[derive(Debug, Clone, PartialEq)]
pub enum MemoryUpdate {
    /// The compare-and-set matched; this is the new head.
    Updated(MemoryEntry),
    /// The compare-and-set found a different version. Nothing was written.
    Diverged {
        /// The row as it is now.
        head: MemoryEntry,
        /// The revision at the version the caller edited from.
        ancestor: MemoryRevision,
    },
}

/// Result of a supersede (plan D4 "replace", D6).
#[derive(Debug, Clone, PartialEq)]
pub enum MemorySupersede {
    /// One transaction: `old` is now `superseded` with `superseded_by = new.id`; `new` is live
    /// at version 1 in `old`'s project.
    Superseded {
        /// The old entry's new head.
        old: MemoryEntry,
        /// The new entry.
        new: MemoryEntry,
    },
    /// The old entry's compare-and-set found a different version. Nothing was written, the new
    /// entry included.
    Diverged {
        /// The old entry as it is now.
        head: MemoryEntry,
        /// Its revision at the version the caller edited from.
        ancestor: MemoryRevision,
    },
}

/// Plan D5 (`R-ID-7`): a subject or body, checked when built. The text is trimmed and must be
/// non-empty, then scrubbed fail-closed by the caller's [`Scrubber`] (a known secret masked,
/// credential-shaped text refused), and the **scrubbed** text is then capped in characters
/// (blueprint P-1). The DB CHECKs are the backstop. `Debug` prints the length only; no `Display`,
/// no serde (I-5, as [`FollowUpText`](crate::model::FollowUpText)).
#[derive(Clone, PartialEq, Eq)]
pub struct MemoryText(String);

impl MemoryText {
    /// A subject: [`MEMORY_SUBJECT_MAX`] characters after scrubbing.
    ///
    /// # Errors
    /// [`MemoryTextError::Empty`] for an empty or whitespace-only text;
    /// [`MemoryTextError::Nul`] when the text holds a U+0000 (P-12);
    /// [`MemoryTextError::Unmasked`] when `scrubber` leaves a credential-shaped leaf;
    /// [`MemoryTextError::TooLong`] `{ max: MEMORY_SUBJECT_MAX }` when the scrubbed text is longer.
    pub fn subject(raw: &str, scrubber: &dyn Scrubber) -> Result<Self, MemoryTextError>;

    /// A body: [`MEMORY_BODY_MAX`] characters after scrubbing.
    ///
    /// # Errors
    /// As [`MemoryText::subject`], with `max: MEMORY_BODY_MAX`.
    pub fn body(raw: &str, scrubber: &dyn Scrubber) -> Result<Self, MemoryTextError>;

    /// The checked text.
    #[must_use]
    pub fn as_str(&self) -> &str;

    /// The checked text, moved out.
    #[must_use]
    pub fn into_string(self) -> String;
}

impl core::fmt::Debug for MemoryText {
    /// `MemoryText { len: <bytes> }`, never the text.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result;
}

/// Why a text was not accepted (plan D5). Nothing reached a store.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MemoryTextError {
    /// Empty or whitespace-only.
    #[error("the text is empty")]
    Empty,
    /// Holds a U+0000, which Postgres `text` cannot store (`22021`, P-12).
    #[error("the text must not contain a NUL character")]
    Nul,
    /// Longer than `max` characters after scrubbing.
    #[error("the text is longer than {max} characters")]
    TooLong {
        /// The cap that was exceeded.
        max: usize,
    },
    /// The scrubber refused the text. Carries the pointer and the rule, never the text
    /// ([`Unmasked`]).
    #[error("not saved: the text looks like it holds a credential ({})", .0.rule)]
    Unmasked(Unmasked),
}
```

Body of the shared private `fn checked(raw: &str, max: usize, scrubber: &dyn Scrubber)`:
`let text = raw.trim()`. If empty → `Empty`. If it contains `'\0'` → `Nul` (P-12). Then `let mut v = Value::String(text.to_owned())` and
`scrubber.scrub(&mut v).map_err(MemoryTextError::Unmasked)?`. Take the `String` back out (a
non-string result is impossible for `Value::String`; treat it as
`Unmasked(Unmasked { path: String::new(), rule: "type_changed" })` or `unreachable!`. Prefer the
error, since nothing in this crate panics on input). Then `if s.chars().count() > max` →
`TooLong { max }`, else `Ok(Self(s))`.

Unit tests (write first, `#[cfg(test)] mod tests` in `memory.rs`; scrubber
`MinimalScrubber::new(Vec::<String>::new())`):

| Test | Asserts |
|---|---|
| `a_blank_text_is_empty` | `""`, `"   \n\t"` → `Err(Empty)` for `subject` and `body` |
| `the_text_is_trimmed` | `"  Build cache \n"` → `as_str() == "Build cache"` |
| `one_character_over_the_cap_is_too_long` | 120×`a` ok, 121×`a` → `TooLong { max: 120 }`; 2000/2001 for `body` → `TooLong { max: 2000 }` |
| `the_cap_counts_characters_not_bytes` | 120×`é` (240 bytes) ok; 121×`é` → `TooLong { max: 120 }` |
| `a_known_secret_is_masked` | scrubber `new(["hunter2-secret".to_owned()])`, body `"pass is hunter2-secret"` → `"pass is [REDACTED]"` |
| `masking_that_overflows_the_cap_is_too_long` | secret `"abcdef"`, subject = 17×`"abcdef"` joined by spaces (118 chars) → masked 186 chars → `TooLong { max: 120 }` (P-1) |
| `a_credential_shaped_text_is_refused_without_repeating_it` | `"key sk-ant-api03-do-not-print-me"` → `Err(Unmasked(u))`, `u.rule == "anthropic_api_key"`; neither `err.to_string()` nor `{err:?}` contains `do-not-print-me` or `sk-ant-` |
| `debug_prints_the_length_only` | `format!("{t:?}") == "MemoryText { len: 11 }"` for `"Build cache"` |
| `memory_subject_key_folds_like_btrim_lower` | `" Build Cache "` → `"build cache"`; `"\tX"` → `"\tx"` (a tab is kept, as `btrim` keeps it) |
| `a_nul_is_refused` | a text holding `'\0'` → `Err(Nul)` for `subject` and `body` (P-12) |
| `only_active_and_suspect_are_live` | `is_live` over `MemoryState::ALL` == `[true, true, false, false]` |

### T1.2 `crates/htui-core/src/model/mod.rs` and `model/ids.rs`

- `ids.rs:127-129`: append after `BatchId`:
  `/// \`memory_entry.id\` (MOD-163 plan D1): one project memory entry.` then `MemoryEntryId,`.
- `mod.rs`: `pub mod memory;` between `link` and `note` (`:92-93`). Add `MemoryEntryId` to the
  `pub use ids::{…}` list (`:125-130`). New `pub use memory::{MEMORY_BODY_MAX, MEMORY_SUBJECT_MAX,
  MemoryEntry, MemoryKind, MemoryPatch, MemoryRevision, MemoryState, MemorySupersede, MemoryText,
  MemoryTextError, MemoryUpdate, NewMemoryEntry, memory_subject_key};` (rustfmt orders it).
- Tests in `mod.rs`, beside `stop_mode_matches_check_list` (`:379-383`):
  ```rust
  /// `0020_memory.sql`'s `memory_entry.kind` CHECK (MOD-163 plan D1).
  #[test]
  fn memory_kind_matches_check_list() {
      check_enum(MemoryKind::ALL, &["fact", "decision", "gotcha", "lesson"]);
  }

  /// `0020_memory.sql`'s `memory_entry.state` CHECK (MOD-163 plan D1).
  #[test]
  fn memory_state_matches_check_list() {
      check_enum(MemoryState::ALL, &["active", "suspect", "superseded", "retired"]);
  }
  ```
  `id_newtypes_round_trip` (`:483-511`) gains `check_id(MemoryEntryId::from_uuid(u), u);`.

### T1.3 `crates/htui-core/src/store/traits.rs`

**Module doc** (`:1-43`): one paragraph after MOD-70's:
"**MOD-163** (plan D6, D7) adds two `ReadStore` reads and four `WriteStore` writers over
`memory_entry` and `memory_entry_revision`. Neither table is mirrored, so `CacheStore` answers both
reads `None` ("not available here"), as it answers `requirement_revisions`."

**Imports** (`:53-73`): add `MemoryEntry, MemoryEntryId, MemoryRevision, MemoryState,
MemorySupersede, MemoryUpdate, NewMemoryEntry, MemoryPatch`.

**ReadStore**: append after `waiting_candidates` (`:316`), mirroring `:253-261`:

```rust
    // ---- MOD-163 M1: project memory entries (plan D6, D7) ------------------------------------

    /// The project's memory entries whose `state` is in `states` (`None`: every state; an empty
    /// list: none), in `(updated_at DESC, id)` order. `Some(vec![])` for an unknown project on
    /// `MemStore`/`PgStore`. `None` means "not available here": `memory_entry` is not mirrored
    /// (plan D7), so the offline mirror answers `None` and the M3 pane says so.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn memory_entries(
        &self,
        project: ProjectId,
        states: Option<Vec<MemoryState>>,
    ) -> Result<Option<Vec<MemoryEntry>>>;

    /// The entry's revisions in `version` order. `Some(vec![])` for an unknown id on
    /// `MemStore`/`PgStore`; `None` = not available here (the mirror, plan D7), as
    /// [`requirement_revisions`](Self::requirement_revisions) answers.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn memory_revisions(&self, id: MemoryEntryId) -> Result<Option<Vec<MemoryRevision>>>;
```

**WriteStore**: append after `open_permissions` (`:2213`, before the closing `}` at `:2214`),
mirroring `:1798-1842`:

```rust
    // ---- MOD-163 M1: project memory entries (plan D3-D6) ------------------------------------

    /// Inserts a live (`active`) entry at version 1 in `project` with `confirmed_at` = now, and
    /// its revision 1 (`reason = "created"`, `author_id = new.created_by`, `box_id = new.box_id`),
    /// in one statement. The text arrives as [`MemoryText`](crate::model::MemoryText), already
    /// trimmed, scrubbed and capped (`R-ID-7`). Checked in the order NotFound, Constraint.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "project" }` for an
    /// unknown project; [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a
    /// duplicate id, a subject that folds to a live entry's of the same project
    /// ([`memory_subject_taken`], naming that entry; plan D4), or a `created_by` / `box_id` that
    /// names no row.
    async fn create_memory_entry(
        &self,
        project: ProjectId,
        new: NewMemoryEntry,
    ) -> Result<MemoryEntry>;

    /// Compare-and-set amend, one transaction (plan D3, D6): the patched columns, `version + 1`,
    /// `confirmed_at` = now and a revision with `reason = "amended"`; the state is kept. A patch that
    /// sets nothing re-confirms the entry. Checked in the order NotFound, divergence, Constraint.
    /// A token that does not match answers `Diverged` with the revision at `expected_version` and
    /// writes nothing.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "memory_entry" }`,
    /// or `{ entity: "memory_entry_revision", id: "<id>@<expected_version>" }` for a token newer
    /// than the head; [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a
    /// `retired` or `superseded` entry ([`memory_entry_closed`]), a new subject that folds to
    /// another live entry's ([`memory_subject_taken`]), or an author / box that names no row.
    async fn amend_memory_entry(
        &self,
        id: MemoryEntryId,
        expected_version: i32,
        patch: MemoryPatch,
    ) -> Result<MemoryUpdate>;

    /// `active | suspect → retired`, compare-and-set like
    /// [`amend_memory_entry`](WriteStore::amend_memory_entry): `version + 1`, a revision with
    /// `reason = "retired"`, and `confirmed_at` left as it was (plan D3). A retired subject is free
    /// again (plan D4).
    ///
    /// # Errors
    /// As [`amend_memory_entry`](WriteStore::amend_memory_entry), an already-retired or superseded
    /// entry included ([`memory_entry_closed`]).
    async fn retire_memory_entry(
        &self,
        id: MemoryEntryId,
        expected_version: i32,
        author_id: UserId,
        box_id: Option<BoxId>,
    ) -> Result<MemoryUpdate>;

    /// Plan D4's "replace" and MOD-164's automatic supersede, in one transaction (plan D6). `old`
    /// (`active | suspect`, compare-and-set on `expected_version`) moves to `superseded` with
    /// `superseded_by = new.id`, `version + 1` and a revision with `reason = "superseded"`, written by
    /// `new.created_by` / `new.box_id`; its `confirmed_at` is left. Then `new` is created in
    /// `old`'s project as [`create_memory_entry`](WriteStore::create_memory_entry) creates one.
    /// The new subject may equal or differ from the old one. Checked in the order NotFound (old),
    /// divergence, Constraint. A divergence or a refusal writes nothing, the new entry included.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "memory_entry" }`
    /// for an unknown `old` (or `{ entity: "memory_entry_revision" }` as above);
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for an `old` that is
    /// already `retired` or `superseded` ([`memory_entry_closed`]), a duplicate `new.id`, a new
    /// subject that folds to a **third** live entry's ([`memory_subject_taken`]), or a
    /// `created_by` / `box_id` that names no row.
    async fn supersede_memory_entry(
        &self,
        old: MemoryEntryId,
        expected_version: i32,
        new: NewMemoryEntry,
    ) -> Result<MemorySupersede>;
```

**Refusal helpers**: after `citation_key` (`:2758-2762`), mirroring `requirement_withdrawn`
(`:2740-2744`):

```rust
// ---- MOD-163: the memory writers' refusals (plan D4, D6) ----

/// Plan D4: a live entry of the same project already holds this folded subject. `subject` is
/// **that entry's** stored subject, and `existing` its id, so both stores spell the refusal alike
/// and M3 can offer "edit it, or replace it".
#[must_use]
pub fn memory_subject_taken(subject: &str, existing: MemoryEntryId) -> String {
    format!("memory subject `{subject}` is taken by live entry {existing}")
}

/// Plan D6: only a live (`active | suspect`) entry is amended, retired or superseded.
#[must_use]
pub fn memory_entry_closed(id: MemoryEntryId, state: MemoryState) -> String {
    format!("memory entry {id} is `{state}`; only an active or suspect entry changes")
}
```

**`DeleteReach`** (`:3059-3132`): add to the type doc "MOD-163's two memory fields are counted on
both stores (plan D8)", then append after `item_requirements`:

```rust
    /// `memory_entry` rows of the project, every state (MOD-163 plan D8).
    pub memory_entries: u64,
    /// `memory_entry_revision` rows of those entries.
    pub memory_revisions: u64,
```

### T1.4 `crates/htui-core/src/store/mod.rs` (P-5)

Add `memory_entry_closed, memory_subject_taken` to `pub use traits::{…}`.

### T1.5 Skeleton implementors

| File | Anchor | T1 change |
|---|---|---|
| `htui-core/src/store/mem.rs` | `impl ReadStore` ends `:8380`; `impl WriteStore` from `:8382`; `project_reach` literal `:4751-4875` | Two reads and four writes, each `Err(StoreError::Backend("MOD-163 M1: not yet".into()))` with `_`-prefixed params. `memory_entries: 0, memory_revisions: 0` in the `project_reach` literal. No `State` change (T2). |
| `htui-store/src/pg/read.rs` | `impl ReadStore for PgStore` (`:74`), after `requirement_revisions` (`:1001`) | Two stub reads. No `query!` (no `.sqlx` churn in T1). |
| `htui-store/src/pg/write.rs` | `impl WriteStore for PgStore` (`:1013`); `project_reach` map `:574-603` | Four stub writes; `memory_entries: 0, memory_revisions: 0` in the `DeleteReach` map, **SQL untouched** (T3). |
| `htui-store/src/cache/read.rs` | after `requirement_revisions` (`:1214-1221`) | **Real**: both `Ok(None)`, doc "Always `None`: `memory_entry` is not mirrored (MOD-163 plan D7); `None` is the seam's 'not available here'." |
| `htui-store/src/writer.rs` | reads after `:292-300`; writes after `:1369-1398` | Forward, `Self::Memory(store)` / `Self::Online(pg)` (both arms, six methods). |
| `htui-store/src/backend.rs` | after `:1079-1088` | Forward the two reads, three arms (`Memory`, `Online { pg, .. }`, `Offline { cache, .. }`). Add the model types to `:27-36`. |
| `htui-agent/src/conformance.rs` | `ReadStore for UsageSpy` `:642-749`, `WriteStore` `:751-` | `self.inner.<verb>(…).await` for all six. |
| `htui-agent/tests/recorder.rs` | `ReadStore for SpyStore` `:329-437`, `WriteStore` `:439-` | Same. |
| `htui/src/hierarchy.rs` | `labelled()` `:729-790` | `-> [(u64, &'static str); 30]`; destructure gains `memory_entries, memory_revisions`; array gains `(memory_entries, "memory entries"), (memory_revisions, "memory revisions")` at the end (field order). |
| `htui-store/tests/pg_criteria.rs` | `:3178-3292` | `TABLES: [&str; 30]` + `"memory_entry", "memory_entry_revision"` at the end; destructure + `claimed: [u64; 30]` gain both fields at the end (struct index 28, 29: positional zip, `:3184-3187`); doc "Thirty entries for thirty fields … MOD-163's two memory tables close it." **Red at runtime until T3** (P-6). |
| `htui-store/tests/cache.rs` | new test near `:2549` | below |

### T1.6 Tests to write first

| Test | File | Asserts |
|---|---|---|
| §T1.1's ten | `model/memory.rs` | as listed |
| `memory_kind_matches_check_list`, `memory_state_matches_check_list` | `model/mod.rs` | CHECK order |
| `id_newtypes_round_trip` (extended) | `model/mod.rs` | `MemoryEntryId` round-trips |
| `reach_parts_names_the_memory_tables` | `htui/src/hierarchy.rs` (beside `:936-966`) | `DeleteReach { documents: 2, memory_entries: 3, memory_revisions: 5, .. }` → `["2 documents", "3 memory entries", "5 memory revisions"]`; `reach_totals == (10, 3)` |
| `memory_is_not_mirrored_so_the_cache_answers_none` | `htui-store/tests/cache.rs` | No database. `tempfile::tempdir()`, `CacheStore::open(dir.path(), "memory-not-mirrored", 1)`. `cache.memory_entries(ids::PROJECT_HTUI, None).await == Ok(None)`, `cache.memory_revisions(MemoryEntryId::new()).await == Ok(None)`, then `cache.close().await`. The file is `#![cfg(feature = "demo")]`, so it runs under `--all-features`. |

### T1.7 Validate

```bash
cargo test -p htui-core --lib model::   # memory.rs's ten, the two CHECK pins, id_newtypes_round_trip
cargo test -p htui --lib hierarchy::tests::reach_parts
cargo test -p htui-store --all-features --test cache memory_is_not_mirrored
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

Commit: `MOD-163 M1 T1: memory model, store verbs and the compile-green skeleton`.

---

## T2 — MemStore and the conformance cases (parallel with T3)

Files (exactly): `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`,
`crates/htui-core/tests/mem_store.rs`, `crates/htui-store/tests/pg_conformance.rs`.

### T2.1 `State` (`mem.rs:186-329`) and `from_demo` (`:371-445`)

```rust
    /// `memory_entry` (MOD-163 plan D1), by id. Not loaded by any fixture (plan D10).
    memory_entries: HashMap<MemoryEntryId, MemoryEntry>,
    /// `memory_entry_revision`, append-only, as [`State::requirement_revisions`] is.
    memory_revisions: Vec<MemoryRevision>,
```
`from_demo` lists every field explicitly, so add `memory_entries: HashMap::new(),
memory_revisions: Vec::new(),`. `State` derives `Default`, so `MemStore::new()` needs nothing.

### T2.2 `impl State` methods (a new `impl State` block beside the MOD-38 one, near `:7133`)

Each mirrors `mint_requirement` (`:7133-7192`) and `revise_requirement` (`:7199-7278`).

- `fn memory_entry_rows(&self, project: ProjectId, states: Option<&[MemoryState]>) -> Vec<MemoryEntry>`
  : filter `project_id == project && states.is_none_or(|s| s.contains(&row.state))`, sort by
  `b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id))`.
- `fn memory_revision_rows(&self, id: MemoryEntryId) -> Vec<MemoryRevision>`
  : filter `entry_id == id`, sort by `version`.
- `fn live_subject_holder(&self, project: ProjectId, subject: &str, except: Option<MemoryEntryId>) -> Option<&MemoryEntry>`
  : `let key = memory_subject_key(subject)`, then the first row (lowest id, for determinism) with
  `project_id == project && state.is_live() && Some(id) != except && memory_subject_key(&row.subject) == key`.
- `fn require_memory_box(&self, box_id: Option<BoxId>) -> Result<()>`
  : as `require_revision_box` (`:7029-7036`) with column `"memory_entry_revision.box_id"`.
- `fn create_memory_entry(&mut self, project: ProjectId, new: NewMemoryEntry, now: DateTime<Utc>) -> Result<MemoryEntry>`:
  1. `projects.contains_key(&project)` else `NotFound { entity: "project", id }`.
  2. `memory_entries.contains_key(&new.id)` → `Constraint(already_exists("memory_entry", new.id))`.
  3. `live_subject_holder(project, new.subject.as_str(), None)` → `Constraint(memory_subject_taken(&holder.subject, holder.id))`.
  4. `require_user(new.created_by, "memory_entry.created_by")?` (`:2817`), `require_memory_box(new.box_id)?`.
  5. Insert the row (`state: Active, version: 1, confirmed_at/created_at/updated_at: now, superseded_by: None`)
     and push revision 1 (`reason "created"`, `author_id: new.created_by`, `box_id: new.box_id`, `created_at: now`).
- A private struct `MemoryEdit { kind: Option<MemoryKind>, subject: Option<String>, body: Option<String>, to: Option<MemoryState>, superseded_by: Option<MemoryEntryId>, restamp: bool, author_id: UserId, box_id: Option<BoxId>, reason: &'static str }`.
- `fn memory_cas_head(&self, id: MemoryEntryId, expected_version: i32) -> Result<core::result::Result<MemoryEntry, (MemoryEntry, MemoryRevision)>>`
  : NotFound (`"memory_entry"`). If the version differs, `Ok(Err((head, ancestor)))`, where the
  ancestor is the revision at `expected_version`, else `NotFound { entity: "memory_entry_revision", id: "{id}@{v}" }`.
  If `!head.state.is_live()` → `Constraint(memory_entry_closed(id, head.state))`. Else `Ok(Ok(head))`.
  This one helper fixes the order NotFound → divergence → closed for all three writers.
- `fn apply_memory_edit(&mut self, id: MemoryEntryId, edit: MemoryEdit, now: DateTime<Utc>) -> MemoryEntry`
  : infallible. Applies the `Some` columns, `to`, `superseded_by`, `version += 1`, `updated_at = now`,
  `confirmed_at = now` iff `restamp`, then pushes the revision (`edit.reason`), and returns the new head.
- `fn revise_memory_entry(&mut self, id, expected_version, edit: MemoryEdit, now) -> Result<MemoryUpdate>`
  : `memory_cas_head` (`Err((head, ancestor))` → `Ok(Diverged)`). Then, when
  `edit.subject.is_some()`, `live_subject_holder(head.project_id, subject, Some(id))` →
  `Constraint(memory_subject_taken(..))`. Then `require_user(edit.author_id, "memory_entry_revision.author_id")?`
  and `require_memory_box`. Then `Ok(Updated(apply_memory_edit(..)))`.
  - The amend wrapper builds `MemoryEdit { kind, subject: patch.subject.map(MemoryText::into_string), body: …, to: None, superseded_by: None, restamp: true, reason: "amended", .. }`.
  - The retire wrapper builds `MemoryEdit { to: Some(Retired), restamp: false, reason: "retired", author_id, box_id, ..None }`.
- `fn supersede_memory_entry(&mut self, old: MemoryEntryId, expected_version: i32, new: NewMemoryEntry, now) -> Result<MemorySupersede>`
  : `memory_cas_head(old, expected_version)` (divergence → `Ok(MemorySupersede::Diverged { head, ancestor })`).
  Then `memory_entries.contains_key(&new.id)` → `already_exists`. Then
  `live_subject_holder(head.project_id, new.subject.as_str(), Some(old))` → `memory_subject_taken`.
  Then `require_user(new.created_by, ..)`, `require_memory_box(new.box_id)`. Only then write:
  `apply_memory_edit(old, MemoryEdit { to: Some(Superseded), superseded_by: Some(new.id), restamp: false, author_id: new.created_by, box_id: new.box_id, reason: "superseded", .. }, now)`,
  then `create_memory_entry(head.project_id, new, now)`. Every check has already passed, so the
  create cannot fail here: `.expect("checked above under the same lock")`, as `:7246` does.
  Return `Superseded { old, new }`.

Wrappers (`impl ReadStore` / `impl WriteStore for MemStore`), shaped as `:8358-8363` and `:9165-9216`:
reads `Ok(Some(self.read(|s| s.memory_entry_rows(project, states.as_deref()))))` and
`Ok(Some(self.read(|s| s.memory_revision_rows(id))))`. Writes:
`let now = self.now(); self.write(|state| state.…(…, now))`.

### T2.3 Project delete (`mem.rs:4696-4889`, `:4934-`, `ProjectReach` `:7524-7541`)

- `ProjectReach` gains `/// \`memory_entry\` ids of the project (MOD-163 plan D8).` and
  `memory_entries: HashSet<MemoryEntryId>`.
- `project_reach`: `let memory_entries: HashSet<MemoryEntryId> = self.memory_entries.values().filter(|r| r.project_id == id).map(|r| r.id).collect();`
  Replace T1's zeros with `memory_entries: rows(memory_entries.len())` and
  `memory_revisions: rows(self.memory_revisions.iter().filter(|r| memory_entries.contains(&r.entry_id)).count())`,
  then move the set into `ProjectReach`.
- `delete_project`, next to the MOD-38 block (`:4978-5011`): `memory_revisions.retain(|r| !gone.memory_entries.contains(&r.entry_id))`,
  then `memory_entries.retain(|id, _| !gone.memory_entries.contains(id))`, then the
  `ON DELETE SET NULL` emulation: a surviving entry whose `superseded_by` is in `gone` gets `None`
  and `updated_at = now` (the trigger fires on the SET NULL). That is unreachable in M1, but it keeps
  the FK's semantics, as the requirement block does.
- `delete_project_leaves_no_row_in_any_map` (`:11763`): stage one entry plus one amend in
  `PROJECT_HTUI` and one entry in `PROJECT_AGY`, through the trait. After the delete: no
  `memory_entries` row has `project_id == gone`, no `memory_revisions` row names a gone entry, and
  the `agy` entry and its revision survive.

### T2.4 Conformance cases (`conformance.rs`)

Helpers (beside the MOD-38 ones, `:13323-13393`), all `fn`/`async fn` taking `case: &str`:
`memory_scrubber() -> MinimalScrubber` (`MinimalScrubber::new(Vec::<String>::new())`);
`memory_new(case, kind, subject, body) -> NewMemoryEntry` (`created_by: ids::USER`,
`box_id: Some(ids::BOX)`, texts via `MemoryText::subject/body(.., &memory_scrubber()).expect(case)`);
`memory_updated(case, MemoryUpdate) -> MemoryEntry` (panics on `Diverged`, as `:13324`);
`memory_superseded(case, MemorySupersede) -> (MemoryEntry, MemoryEntry)`;
`memory_list(case, store, project, states) -> Vec<MemoryEntry>` (unwrap `Some`);
`memory_history(case, store, id) -> Vec<MemoryRevision>` (unwrap `Some`, as `revisions_of` `:13345`).
Imports: `MemoryEntryId, MemoryKind, MemoryPatch, MemoryState, MemorySupersede, MemoryText,
MemoryUpdate, NewMemoryEntry` (model), `memory_entry_closed, memory_subject_taken` (traits),
`crate::scrub::MinimalScrubber`.

Ten cases, appended to `CASES` (`:61-237`) and to `run_case`'s match (before `other =>`, `:639`),
all `async fn name<S: WriteStore>(store: &S)` against `MemStore::demo()` / `demo_db()`:

| # | Case | Assertions (one line each) |
|---|---|---|
| 1 | `memory_create_writes_the_row_and_revision_1` | Row: `Active`, v1, `superseded_by None`, `confirmed_at == created_at`, project and text as given. `memory_list(HTUI, None) == [row]`. `memory_history == [rev1 { reason "created", author USER, box Some(BOX), state Active }]` |
| 2 | `memory_list_orders_by_updated_at_and_filters_by_state` | Create A, B, C (separate calls), retire B. `None` → `[B, C, A]`; `Some([Active])` → `[C, A]`; `Some([Retired])` → `[B]`; `Some(vec![])` → `[]`; `PROJECT_AGY` → `[]`; `ProjectId::new()` → `Some(vec![])` |
| 3 | `memory_amend_is_cas_restamps_and_writes_a_revision` | Amend body at v1 → `Updated`: v2, body moved, subject and kind kept, `Active`, `confirmed_at > before.confirmed_at`. History `[created, amended]`. Stale v1 → `Diverged { head, ancestor: rev1 }`, still 2 revisions. Empty patch at v2 → v3, same text, `confirmed_at` moved (re-confirm). Token 9 → `NotFound { entity: "memory_entry_revision" }` |
| 4 | `memory_retire_closes_the_entry_once` | Retire at v1 → `Retired`, v2, `confirmed_at` unchanged, revision `"retired"`. Retire at v2 → `Constraint(memory_entry_closed(id, Retired))`. Amend at v2 → same Constraint. Retire at stale v1 → `Diverged` (divergence before closed). Still 2 revisions |
| 5 | `memory_subject_clash_is_refused_case_and_space_folded` | A = "Build cache". Create " build CACHE " → `Constraint(memory_subject_taken("Build cache", A.id))`, nothing written. B = "Other": amend subject to "BUILD cache" → same Constraint, B still v1. "Build cache" in `PROJECT_AGY` → ok. A's own amend to "build cache" → `Updated` (no self-clash) |
| 6 | `memory_retired_subject_can_be_reused` | Create "Flaky test", retire, create "flaky test" → ok; `memory_list(Some([Active]))` holds only the second |
| 7 | `memory_supersede_replaces_in_one_step` | A at v1 → `supersede(A, 1, N same subject)` → `old`: `Superseded`, `superseded_by Some(N.id)`, v2, `confirmed_at` unchanged; `new`: `Active` v1 in A's project. A history `[created, superseded(author N.created_by)]`, N history `[created]`. `Some([Active])` → `[N]`. Then `supersede(N, 1, M different subject)` → ok (the subject may differ) |
| 8 | `memory_supersede_refuses_stale_closed_and_clashing` | Stale → `MemorySupersede::Diverged`, and `memory_history(new.id) == []` and A unchanged. Retired R → `Constraint(memory_entry_closed(R, Retired))`. New subject = live C's → `Constraint(memory_subject_taken(C.subject, C.id))`, A still `Active` v1. Unknown old → `NotFound { entity: "memory_entry" }` |
| 9 | `memory_unknown_rows_and_authors_are_refused` | Create in `ProjectId::new()` → `NotFound { entity: "project" }`. Amend and retire of an unknown id → `NotFound { entity: "memory_entry" }`. `memory_revisions(unknown) == Some(vec![])`. Duplicate id → `Constraint`. `created_by: UserId::new()` → `Constraint`. Patch `author_id: UserId::default()` (nil) → `Constraint` |
| 10 | `project_delete_counts_memory` | Two entries in HTUI (one amended, one superseded by a third) plus one in AGY. `delete_reach == delete_project` report; `(memory_entries, memory_revisions) == (3, 5)`; afterwards `memory_list(HTUI, None) == Some(vec![])` (unwrap), each gone id's history `== []`, and the AGY entry is intact |

Case 10's counts: E1 (created + amended = 2), E2 (created + superseded = 2), E3 (created = 1) →
3 entries, 5 revisions.

Doc rule (P-8): no backticked `uq_memory_entry_live_subject` anywhere in `conformance.rs`.

### T2.5 Counts

- `htui-core/tests/mem_store.rs:35-37`: `175` → **`185`**, and the message gains "MOD-163 M1's
  ten memory cases (plan D4, D6, D8)".
- `htui-store/tests/pg_conformance.rs:39`: `EXPECTED_CASES: usize = 185`; doc chain `:17-38` gains
  "and MOD-163 M1's ten memory cases (plan D4, D6, D8) make it 185."; message `:46-49` likewise.

### T2.6 Validate (P-7: the feature is required)

```bash
cargo test -p htui-core --features test-support --test mem_store -- --test-threads=1
cargo test -p htui-core --features test-support --lib conformance::tests -- --test-threads=1
cargo test -p htui-core --features test-support --lib delete_project_leaves_no_row_in_any_map
cargo clippy -p htui-core --all-targets --all-features -- -D warnings
```
Check that each run reports a non-zero test count. `conformance::tests` holds
`run_case_accepts_every_name_in_cases` and the P-8 scanner.

Commits: (1) `MOD-163 M1 T2: ten memory conformance cases (red on MemStore)`;
(2) `MOD-163 M1 T2: MemStore memory entries, reach and delete`.

---

## T3 — Migration and PgStore (parallel with T2)

Files (exactly): `crates/htui-store/migrations/0020_memory.sql`, `crates/htui-store/src/pg/read.rs`,
`crates/htui-store/src/pg/write.rs`, `crates/htui-store/.sqlx/query-*.json`,
`crates/htui-store/tests/migrations.rs`, `crates/htui-store/tests/connect.rs`,
`crates/htui-store/tests/pg_criteria.rs` (P-6). Intersection with T2's four files: none.

### T3.1 `crates/htui-store/migrations/0020_memory.sql` (probed)

```sql
-- 0020_memory.sql - MOD-163 milestone 1 (plan D1, D3, D4, D6-D9): the project memory entry store
-- (R-ENT-13, ANA-38 §4.6). Forward-only (R-STO-5). Depends on 0001_init.sql (project, app_user,
-- box, set_updated_at()). Mirrors 0006_requirements.sql's requirement / requirement_revision pair.
--
-- memory_entry is one project-owned entry: kind, subject, prose, state, a compare-and-set version
-- and confirmed_at, the freshness stamp (D3). Text reaches it only through htui-core's MemoryText,
-- trimmed, scrubbed and capped in characters before the write (R-ID-7, D5); the two char_length
-- CHECKs are the backstop. One live entry per subject (D4): uq_memory_entry_live_subject folds
-- the subject with lower(btrim()) among active and suspect entries of a project; it is named so
-- that PgStore can tell its 23505 from the primary key's. superseded_by is DEFERRABLE INITIALLY
-- DEFERRED because a supersede moves the old entry first (freeing its subject) and inserts the
-- new one second, in one transaction (D6); it is set only on a superseded entry, and ON DELETE
-- SET NULL may clear it again, so the CHECK is one-way. Provenance, usage counters and pins are
-- later items' columns (D9).
--
-- Not mirrored (D7): there is no cache migration. cache_meta.schema_version moves to 20 through
-- PgStore::schema_version(), so each box rebuilds its mirror once on first start; a headless
-- worker never migrates: migrate from a TUI first.

-- --------------------------------------------------------------------------------------------
-- 1. memory_entry
-- --------------------------------------------------------------------------------------------

CREATE TABLE memory_entry (
    id            UUID        PRIMARY KEY,                   -- client-minted UUIDv7
    project_id    UUID        NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    kind          TEXT        NOT NULL CHECK (kind IN ('fact','decision','gotcha','lesson')),
    subject       TEXT        NOT NULL CHECK (char_length(subject) BETWEEN 1 AND 120),
    body          TEXT        NOT NULL CHECK (char_length(body) BETWEEN 1 AND 2000),
    state         TEXT        NOT NULL DEFAULT 'active'
                              CHECK (state IN ('active','suspect','superseded','retired')),
    version       INTEGER     NOT NULL DEFAULT 1,            -- CAS, ANA-9 §4.2
    confirmed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    superseded_by UUID
        CONSTRAINT fk_memory_entry_superseded_by REFERENCES memory_entry(id)
        ON DELETE SET NULL DEFERRABLE INITIALLY DEFERRED,
    created_by    UUID        NOT NULL REFERENCES app_user(id),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_memory_entry_superseded_by
        CHECK (superseded_by IS NULL OR (state = 'superseded' AND superseded_by <> id))
);
CREATE INDEX idx_memory_entry_updated_at ON memory_entry(project_id, updated_at);   -- list order
CREATE UNIQUE INDEX uq_memory_entry_live_subject
    ON memory_entry(project_id, lower(btrim(subject)))
    WHERE state IN ('active','suspect');
-- ON DELETE SET NULL's lookup when an entry is deleted (as 0006's idx_requirement_revision_amended_by)
CREATE INDEX idx_memory_entry_superseded_by ON memory_entry(superseded_by)
    WHERE superseded_by IS NOT NULL;

-- --------------------------------------------------------------------------------------------
-- 2. memory_entry_revision: append-only, no updated_at, no trigger
-- --------------------------------------------------------------------------------------------

CREATE TABLE memory_entry_revision (
    entry_id    UUID        NOT NULL REFERENCES memory_entry(id) ON DELETE CASCADE,
    version     INTEGER     NOT NULL,
    kind        TEXT        NOT NULL,
    subject     TEXT        NOT NULL,
    body        TEXT        NOT NULL,
    state       TEXT        NOT NULL,
    author_id   UUID        NOT NULL REFERENCES app_user(id),
    box_id      UUID        REFERENCES box(id) ON DELETE SET NULL,
    reason      TEXT        NOT NULL,                        -- 'created','amended','retired','superseded'
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (entry_id, version)
);

-- --------------------------------------------------------------------------------------------
-- 3. updated_at trigger, 0001 §5.1's shape (BEFORE UPDATE only). memory_entry_revision has no
-- updated_at and is deliberately absent.
-- --------------------------------------------------------------------------------------------

CREATE TRIGGER trg_memory_entry_updated_at BEFORE UPDATE ON memory_entry
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- --------------------------------------------------------------------------------------------
-- 4. Comments (pinned verbatim by tests/migrations.rs's MOD163_* constants)
-- --------------------------------------------------------------------------------------------

COMMENT ON TABLE memory_entry IS
    'MOD-163 D1: project memory entries (R-ENT-13), written by a person in M1. Not mirrored (D7).';
COMMENT ON TABLE memory_entry_revision IS
    'MOD-163 D1: append-only history of memory_entry, one row per version.';
COMMENT ON COLUMN memory_entry.state IS
    'MOD-163 D4: active and suspect are live, one per folded subject per project; superseded and '
    'retired are closed. Nothing in M1 sets suspect.';
COMMENT ON COLUMN memory_entry.version IS
    'MOD-163 D3: the compare-and-set token; every write moves it by one and writes a revision.';
COMMENT ON COLUMN memory_entry.confirmed_at IS
    'MOD-163 D3: when the text was last written or confirmed; create and amend set it, retire and '
    'supersede leave it.';
COMMENT ON COLUMN memory_entry.superseded_by IS
    'MOD-163 D4, D6: the entry that replaced this one, set only on a superseded entry; NULL again '
    'if that entry is deleted.';
COMMENT ON COLUMN memory_entry_revision.reason IS
    'MOD-163 D1: created, amended, retired or superseded (no CHECK, as requirement_revision.reason).';
```

Constraint names Postgres assigns, read back: `memory_entry_pkey`, `memory_entry_project_id_fkey`,
`memory_entry_kind_check`, `memory_entry_subject_check`, `memory_entry_body_check`,
`memory_entry_state_check`, `memory_entry_created_by_fkey`, `fk_memory_entry_superseded_by`,
`chk_memory_entry_superseded_by`, `memory_entry_revision_pkey`, plus indexes
`idx_memory_entry_updated_at`, `uq_memory_entry_live_subject`, `idx_memory_entry_superseded_by`.

### T3.2 Probe results (scratch DBs `mod163_probe`, `mod163_probe2`, dropped)

| # | Statement | Outcome |
|---|---|---|
| 1 | `'Foo'` then `' foo '` in one project | `23505`, `CONSTRAINT NAME: uq_memory_entry_live_subject`, DETAIL carries only the key |
| 2 | `superseded_by` on an `active` row | `23514 chk_memory_entry_superseded_by` |
| 3 | One tx: UPDATE A → superseded/`superseded_by = C`, then INSERT C with the same folded subject | committed |
| 4 | Deferred `superseded_by` naming no row | `23503 fk_memory_entry_superseded_by` **at COMMIT** |
| 5 | 121×`é` / 120×`é` subject | `23514 memory_entry_subject_check` / inserted (characters, not bytes) |
| 6 | Same subject in another project | inserted |
| 7 | (immediate FK variant) UPDATE old first | `23503` on the UPDATE → why P-2 |
| 8 | Delete a successor alone: one-way CHECK / two-way CHECK | allowed, old keeps `superseded` with NULL / `23514` → why P-3 |
| 9 | `DELETE FROM project` over a supersede chain + a revision | 0 entries, 0 revisions left; no error |
| 10 | Unknown kind | `23514 memory_entry_kind_check` |
| — | Comments | the seven texts read back byte for byte as in §T3.6 |

### T3.3 `pg/read.rs` (replace T1's stubs; shape of `:916-1001`)

```sql
-- memory_entries
SELECT id            AS "id: MemoryEntryId",
       project_id    AS "project_id: ProjectId",
       kind          AS "kind: MemoryKind",
       subject, body,
       state         AS "state: MemoryState",
       version, confirmed_at,
       superseded_by AS "superseded_by: MemoryEntryId",
       created_by    AS "created_by: UserId",
       created_at, updated_at
  FROM memory_entry
 WHERE project_id = $1
   AND ($2::text[] IS NULL OR state = ANY($2))
 ORDER BY updated_at DESC, id
```
Bind `states.map(|v| v.iter().map(|s| s.as_str().to_owned()).collect::<Vec<_>>())` and
`.as_deref()` (as `:921-925`). Answer `Ok(Some(rows))`.

```sql
-- memory_revisions
SELECT entry_id  AS "entry_id: MemoryEntryId", version,
       kind      AS "kind: MemoryKind", subject, body,
       state     AS "state: MemoryState",
       author_id AS "author_id: UserId",
       box_id    AS "box_id: BoxId",
       reason, created_at
  FROM memory_entry_revision WHERE entry_id = $1 ORDER BY version
```
Answer `Ok(Some(rows))`: an unknown id is an empty history (`:999-1000`).

### T3.4 `pg/write.rs` (replace T1's stubs; helpers beside the MOD-38 block `:606-779`)

Private items:

```rust
/// What an amend, a retire or a supersede's first half writes (plan D6); `None` leaves a column.
struct MemoryEdit {
    kind: Option<MemoryKind>,
    subject: Option<String>,
    body: Option<String>,
    state: Option<MemoryState>,
    superseded_by: Option<MemoryEntryId>,
    restamp: bool,            // confirmed_at = now() (amend only, D3)
    author_id: UserId,
    box_id: Option<BoxId>,
    reason: &'static str,
}

/// Locks the row `FOR UPDATE` and decides NotFound, divergence, closed in the seam's order.
/// `Ok(Err((head, ancestor)))` is a divergence.
async fn memory_cas_head(
    conn: &mut PgConnection,
    id: MemoryEntryId,
    expected_version: i32,
) -> Result<core::result::Result<MemoryEntry, (MemoryEntry, MemoryRevision)>>;

/// One statement: the UPDATE and its revision (the `u`/`v` CTE of `revise_requirement`, `:711-751`).
async fn apply_memory_edit(
    conn: &mut PgConnection,
    id: MemoryEntryId,
    edit: MemoryEdit,
) -> core::result::Result<MemoryEntry, sqlx::Error>;

/// One statement: the row and revision 1 (`mint_requirement`'s shape, `:6978-7024`); `None` = no
/// such project. Shared by create and supersede, so both use one `.sqlx` entry.
async fn insert_memory_entry(
    conn: &mut PgConnection,
    project: ProjectId,
    new: &NewMemoryEntry,
) -> core::result::Result<Option<MemoryEntry>, sqlx::Error>;

/// `persona_name_taken`'s rule (`:785-804`) plus one read (P-4). `memory_entry_pkey` →
/// `Constraint(already_exists("memory_entry", new_id))`. `uq_memory_entry_live_subject` → the live
/// holder of `subject`'s fold in `project` other than `except`, read on `pool` **after** the
/// caller rolled back → `Constraint(memory_subject_taken(&holder.subject, holder.id))`. If no
/// holder remains (it was retired in between), and for anything else, `map_sqlx(err)`.
async fn memory_write_refused(
    pool: &sqlx::PgPool,
    err: sqlx::Error,
    project: ProjectId,
    subject: &str,
    except: MemoryEntryId,
    new_id: Option<MemoryEntryId>,
) -> StoreError;
```

SQL:

```sql
-- memory_cas_head: lock (then, on a version mismatch, T3.3's revision SELECT with
-- "AND version = $2", fetch_optional → NotFound { entity: "memory_entry_revision", id: "{id}@{v}" })
SELECT <T3.3's memory_entries projection> FROM memory_entry WHERE id = $1 FOR UPDATE

-- apply_memory_edit
WITH u AS (
    UPDATE memory_entry SET
        kind          = COALESCE($2, kind),
        subject       = COALESCE($3, subject),
        body          = COALESCE($4, body),
        state         = COALESCE($5, state),
        superseded_by = COALESCE($6, superseded_by),
        confirmed_at  = CASE WHEN $7::boolean THEN now() ELSE confirmed_at END,
        version       = version + 1
     WHERE id = $1
    RETURNING id, project_id, kind, subject, body, state, version, confirmed_at, superseded_by,
              created_by, created_at, updated_at
), v AS (
    INSERT INTO memory_entry_revision (entry_id, version, kind, subject, body, state,
                                       author_id, box_id, reason)
    SELECT id, version, kind, subject, body, state, $8, $9, $10 FROM u
    RETURNING entry_id
)
SELECT u.id AS "id!: MemoryEntryId", u.project_id AS "project_id!: ProjectId",
       u.kind AS "kind!: MemoryKind", u.subject AS "subject!", u.body AS "body!",
       u.state AS "state!: MemoryState", u.version AS "version!",
       u.confirmed_at AS "confirmed_at!", u.superseded_by AS "superseded_by: MemoryEntryId",
       u.created_by AS "created_by!: UserId", u.created_at AS "created_at!",
       u.updated_at AS "updated_at!"
  FROM u, v

-- insert_memory_entry
WITH p AS (SELECT id FROM project WHERE id = $1),
     e AS (
    INSERT INTO memory_entry (id, project_id, kind, subject, body, created_by)
    SELECT $2, p.id, $3, $4, $5, $6 FROM p
    RETURNING id, project_id, kind, subject, body, state, version, confirmed_at, superseded_by,
              created_by, created_at, updated_at
), v AS (
    INSERT INTO memory_entry_revision (entry_id, version, kind, subject, body, state,
                                       author_id, box_id, reason)
    SELECT id, version, kind, subject, body, state, $6, $7, 'created' FROM e
    RETURNING entry_id
)
SELECT <e.* with the "!" overrides as above> FROM e, v

-- memory_write_refused's holder lookup (matches the partial index's predicate literally)
SELECT id AS "id: MemoryEntryId", subject
  FROM memory_entry
 WHERE project_id = $1 AND state IN ('active','suspect')
   AND lower(btrim(subject)) = lower(btrim($2)) AND id <> $3
 ORDER BY id LIMIT 1
```

Note `superseded_by` stays nullable (no `!`) in both projections.

Verbs:

- **create**: `let mut conn = self.pool.acquire()`. Run `insert_memory_entry(&mut conn, project, &new)`.
  `Ok(None)` → `NotFound { entity: "project" }`. On `Err(e)` →
  `memory_write_refused(&self.pool, e, project, new.subject.as_str(), new.id, Some(new.id))`. The
  statement is atomic, so there is no rollback to do.
- **amend / retire**: `let mut tx = pool.begin()`. Run `memory_cas_head(&mut tx, ..)?`. A divergence
  → `Ok(MemoryUpdate::Diverged)` (dropping `tx` rolls back). Then `apply_memory_edit`. On `Err(e)`:
  `tx.rollback().await` first, then
  `memory_write_refused(&pool, e, head.project_id, subject_or_head_subject, id, None)`. On success
  commit and return `Updated`. Edits: amend =
  `{ kind, subject, body, state: None, superseded_by: None, restamp: true, reason: "amended" }`;
  retire = `{ state: Some(Retired), restamp: false, reason: "retired", ..None }`.
- **supersede**: `begin`, then `memory_cas_head(old)` (divergence → `MemorySupersede::Diverged`).
  Then `apply_memory_edit(old, { state: Some(Superseded), superseded_by: Some(new.id),
  restamp: false, author_id: new.created_by, box_id: new.box_id, reason: "superseded" })`. Then
  `insert_memory_entry(&mut tx, head.project_id, &new)`. `None` there is impossible, because the
  old row's project is locked behind its FK: answer `StoreError::Backend(..)`, as `:768-775` does.
  Any `sqlx::Error` from either statement → rollback, then
  `memory_write_refused(&pool, e, head.project_id, new.subject.as_str(), old, Some(new.id))`.
  Commit (the deferred FK is checked here and always holds), then
  `Superseded { old: <edited head>, new }`. `except = old` is correct: after the rollback `old` is
  live again, and a same-subject clash can only be with a third entry, because the index forbids two
  live holders.
- **`project_reach`** (`:511-604`): add the CTE `me AS (SELECT id FROM memory_entry WHERE project_id = $1)`
  and two select items, `(SELECT count(*) FROM me) AS "memory_entries!"` and
  `(SELECT count(*) FROM memory_entry_revision WHERE entry_id IN (SELECT id FROM me)) AS "memory_revisions!"`.
  Map them in place of T1's zeros. `delete_project` itself is unchanged: the cascade does the work
  (probe row 9). Update the doc (`:494-510`): "`me` is the project's memory entries (MOD-163 D8)".

Imports in `write.rs`/`read.rs`: the model types plus `memory_entry_closed`,
`memory_subject_taken` (`htui_core::store::traits::{…}`, `:51-58`).

### T3.5 `.sqlx` regeneration

New: memory_entries, memory_revisions, revision-at-version, lock `FOR UPDATE`, `apply_memory_edit`,
`insert_memory_entry`, holder lookup (7). Changed: `project_reach` (old hash removed, new one
added). Recipe (memory: compose `htui` DB is empty; prepare needs a migrated scratch DB):

```bash
cd /home/mluigi/projects/htui/crates/htui-store
BASE="${HTUI_TEST_DATABASE_URL%/*}"
psql "$HTUI_TEST_DATABASE_URL" -c 'DROP DATABASE IF EXISTS mod163_prepare' -c 'CREATE DATABASE mod163_prepare'
DATABASE_URL="$BASE/mod163_prepare" sqlx migrate run --source migrations
DATABASE_URL="$BASE/mod163_prepare" cargo sqlx prepare -- --all-targets --all-features
DATABASE_URL="$BASE/mod163_prepare" cargo sqlx prepare --check -- --all-targets --all-features
psql "$HTUI_TEST_DATABASE_URL" -c 'DROP DATABASE mod163_prepare'
```
While writing the queries, keep `DATABASE_URL` exported to that migrated DB, because the macros
check online. Without it a new `query!` has no offline entry and does not compile.

### T3.6 Test pins

`tests/migrations.rs`:
- `:25-29` doc "The 48 tables … then the two of `0020_memory.sql` (MOD-163 plan D1)"; `TABLES`
  after `:83`: `// 0020_memory.sql (MOD-163)`, `"memory_entry"`, `"memory_entry_revision"`.
- `:105` `…, 18, 19, 20`; `:115` prose "… MOD-125's 0019_stop_runs.sql and MOD-163's
  0020_memory.sql, in ordinal order".
- `:133` `46` → `48`; the messages at `:134-138` and `:144-147` gain "and MOD-163's 0020_memory.sql
  adds memory_entry and memory_entry_revision".
- `:1903`, `:2004`: `MigrationState::Pending(20)`; `:2013`, `:2036`:
  `HeadlessError::MigrationsPending(20)`; `:2188`: `20`. Every "nineteen … through MOD-125's
  0019_stop_runs.sql" becomes "twenty … through MOD-163's 0020_memory.sql". `:208` ("the last
  nineteen are ANA-2 §9") is unrelated: leave it.
- After `MOD125_TABLE_COMMENT` (`:643-648`):
  ```rust
  /// The five `COMMENT ON COLUMN` texts of `0020_memory.sql` (MOD-163 plan D1, D3, D4, D6),
  /// verbatim and in migration order, for [`ANA_COLUMN_COMMENTS`]'s reason.
  const MOD163_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[
      ("memory_entry", "state",
       "MOD-163 D4: active and suspect are live, one per folded subject per project; superseded \
        and retired are closed. Nothing in M1 sets suspect."),
      ("memory_entry", "version",
       "MOD-163 D3: the compare-and-set token; every write moves it by one and writes a revision."),
      ("memory_entry", "confirmed_at",
       "MOD-163 D3: when the text was last written or confirmed; create and amend set it, retire \
        and supersede leave it."),
      ("memory_entry", "superseded_by",
       "MOD-163 D4, D6: the entry that replaced this one, set only on a superseded entry; NULL \
        again if that entry is deleted."),
      ("memory_entry_revision", "reason",
       "MOD-163 D1: created, amended, retired or superseded (no CHECK, as \
        requirement_revision.reason)."),
  ];

  /// The two `COMMENT ON TABLE` texts of `0020_memory.sql` (MOD-163 plan D1), verbatim.
  const MOD163_TABLE_COMMENTS: [(&str, &str); 2] = [
      ("memory_entry",
       "MOD-163 D1: project memory entries (R-ENT-13), written by a person in M1. Not mirrored \
        (D7)."),
      ("memory_entry_revision",
       "MOD-163 D1: append-only history of memory_entry, one row per version."),
  ];
  ```
  Each `\` continuation must keep exactly one space before the backslash, so the joined string is
  byte-identical to §T3.1. Chain `MOD163_COLUMN_COMMENTS` into all three chains (`:871-880`,
  `:932-941`, `:951-960`). The table loop `:904` becomes
  `[ANA_TABLE_COMMENT, MOD125_TABLE_COMMENT].into_iter().chain(MOD163_TABLE_COMMENTS)`. Add
  "MOD-163" to the two messages (`:897-898`, `:916`).
- New tests (staging as `migration_0019_admits_stopped_and_refuses_an_unknown_stop_mode`,
  `:1537-1545`: `fresh_db`, `this_user`, `plant_project_kind`; refusals through
  `assert_refused_by` `:3184`):

| Test | Asserts |
|---|---|
| `migration_0020_folds_live_subjects_per_project` | `'Foo'` then `' foo '` → `uq_memory_entry_live_subject`; after `UPDATE … state='retired'` the second insert lands; another project's `'FOO'` lands |
| `migration_0020_ties_superseded_by_to_the_superseded_state` | `superseded_by` on an active row → `chk_memory_entry_superseded_by`; one tx UPDATE-then-INSERT with the same subject commits; a dangling `superseded_by` fails `fk_memory_entry_superseded_by` at `commit()` |
| `migration_0020_caps_text_in_characters` | 120×`é` subject lands; 121×`é` → `memory_entry_subject_check`; a `kind` of `'note'` → `memory_entry_kind_check` |

`tests/connect.rs`: `:140` and `:242` `Pending(20)`, `:156` `20`. The messages at `:141-142`,
`:157-158` and `:243-244` read "twenty embedded migrations, through MOD-163's 0020_memory.sql".

`tests/pg_criteria.rs` (P-6): before `let mut before` (`:3219`), seed through the store, which T3
has just made real: `db.store.create_memory_entry(project, NewMemoryEntry { .. })` and one
`amend_memory_entry(id, 1, MemoryPatch { author_id: ids::USER, ..Default::default() })` (1 entry,
2 revisions). Extend the non-zero tuple (`:3307-3316`) with `memory_entries == 1,
memory_revisions == 2`.

### T3.7 Validate

```bash
# (T3.5 recipe: prepare, then --check)
cargo test -p htui-store --all-features --test migrations --test connect -- --test-threads=1
cargo test -p htui-store --all-features --test pg_criteria every_cascade_table_loses_exactly_what_the_report_names -- --test-threads=1
cargo clippy -p htui-store --all-targets --all-features -- -D warnings
```
`--test pg_conformance` only goes green once T2's cases are present (T4).

Commits: (1) `MOD-163 M1 T3: 0020_memory.sql and its migration pins`;
(2) `MOD-163 M1 T3: PgStore memory reads, writes and reach, .sqlx regenerated`.

---

## T4 — Integration gate (serial)

Merge T2 and T3 (their files are disjoint, so the merge has no conflicts). Re-run
`cargo sqlx prepare --check` after the merge: T2 adds no query, but run it anyway. Then:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/m1-gate.log
grep -c SIGABRT /tmp/m1-gate.log   # expect 0
(cd crates/htui-store && DATABASE_URL="$BASE/mod163_prepare" cargo sqlx prepare --check -- --all-targets --all-features)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

What to confirm in the gate:
- `pg_store_conformance` prints `case memory_*` ten times. `EXPECTED_CASES == 185 ==
  CASES.len()`.
- `every_cross_referenced_test_name_exists` passes (P-8).
- `agent_settings`, `hierarchy` and other `htui` tests compile against the 30-field `DeleteReach`.

---

## 6. Risks and gotchas for implementers

| Gotcha | What to do |
|---|---|
| `.sqlx` lives in `crates/htui-store/.sqlx`, not at the workspace root (C12) | Prepare from `crates/htui-store` against a **migrated scratch DB** (§T3.5). The compose `htui` DB is empty, and `HTUI_TEST_DATABASE_URL` (`localhost:5439`, user `postgres`) is the harness's admin DSN, not a prepare target. Only T3 touches `.sqlx`. |
| `DATABASE_URL` set while compiling | sqlx macros go online. In T2's lane, leave it **unset** so the offline `.sqlx` is used. T2 must not compile against T3's half-written queries: separate worktrees (memory: workflow worktree implementers). |
| `build.rs` reruns on `migrations/` | A warm `target/` still picks up `0020`. If `MIGRATOR` seems stale, `touch crates/htui-store/build.rs`. |
| `-p htui-core` without `test-support` runs 0 tests (P-7) | Always pass `--features test-support` (or `--all-features`) and read the test count. |
| Conformance self-scanner (P-8) | No backticked snake_case token with ≥4 underscores in `conformance.rs` unless it names a fn there or in `mem.rs`. That includes `uq_memory_entry_live_subject`. |
| Featureless clippy (`cargo clippy --workspace -- -D warnings`) | `--all-features` hides code that is dead without `test-support`. The model helpers are `pub` and fine. Do not put T2 helpers used only by conformance outside `conformance.rs`. |
| Mem fold vs Pg fold | `memory_subject_key` = `trim_matches(' ')` + `to_lowercase()`. `MemoryText` already trims all whitespace, so the btrim difference never shows through the seam. Conformance uses ASCII case and spaces only. |
| Pg `now()` vs `clock_timestamp()` | `confirmed_at` is `now()`; `updated_at` on UPDATE is `clock_timestamp()`. Never assert `confirmed_at == updated_at` after an amend. Do not assert list order across one supersede (§1). |
| Superseding a row with the same subject | Works only because the FK is `DEFERRABLE INITIALLY DEFERRED` and the old row is updated **before** the insert (probe rows 3, 7). Do not reorder. |
| Error inside a Pg transaction | Postgres aborts the transaction. Roll back **before** the holder lookup and run the lookup on `pool`, not `tx`. |
| Engine stack headroom | M1 touches no engine path, but the gate runs every engine test. Keep `--no-fail-fast` and grep `SIGABRT` (expect 0). |
| Scheduling-dependent suite | Gate with `--test-threads=1`. The keyring fake is process-wide. `qdrant_live` timeouts under load: re-run serially before calling one a regression. |
| Forbidden literal | `zeta` anywhere fails htui-agent's extensibility test. Keep test text plain ASCII plus `é`. |
| Disk | Each worktree's `target/` is about 10 GB. Check `df -h .` before fanning out T2 ∥ T3. A Postgres crash loop usually means disk pressure. |
| M3 forward note | The TUI's "edit it, or replace it?" needs the clashing id. Find it with `memory_subject_key` over the pane's loaded list. Do not parse `memory_subject_taken`'s string. |
