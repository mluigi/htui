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
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Active | Self::Suspect)
    }
}

/// Plan D4: the fold `uq_memory_entry_live_subject` compares by, Postgres's
/// `lower(btrim(subject))`. `btrim` strips spaces only, so this is `trim_matches(' ')` then
/// `to_lowercase()`, not `trim()`. The two agree on ASCII. A non-ASCII subject can fold differently
/// in `MemStore` and Postgres, as [`RequirementFilter::text`] warns.
///
/// [`RequirementFilter::text`]: crate::model::requirement::RequirementFilter::text
#[must_use]
pub fn memory_subject_key(subject: &str) -> String {
    subject.trim_matches(' ').to_lowercase()
}

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
    /// [`MemoryTextError::Unmasked`] when `scrubber` leaves a credential-shaped leaf;
    /// [`MemoryTextError::TooLong`] `{ max: MEMORY_SUBJECT_MAX }` when the scrubbed text is longer.
    pub fn subject(raw: &str, scrubber: &dyn Scrubber) -> Result<Self, MemoryTextError> {
        Self::checked(raw, MEMORY_SUBJECT_MAX, scrubber)
    }

    /// A body: [`MEMORY_BODY_MAX`] characters after scrubbing.
    ///
    /// # Errors
    /// As [`MemoryText::subject`], with `max: MEMORY_BODY_MAX`.
    pub fn body(raw: &str, scrubber: &dyn Scrubber) -> Result<Self, MemoryTextError> {
        Self::checked(raw, MEMORY_BODY_MAX, scrubber)
    }

    /// The checked text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The checked text, moved out.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    /// Trim, refuse empty, scrub (fail-closed), then cap the scrubbed text in characters.
    fn checked(raw: &str, max: usize, scrubber: &dyn Scrubber) -> Result<Self, MemoryTextError> {
        let text = raw.trim();
        if text.is_empty() {
            return Err(MemoryTextError::Empty);
        }
        let mut value = Value::String(text.to_owned());
        scrubber
            .scrub(&mut value)
            .map_err(MemoryTextError::Unmasked)?;
        // A scrubber masks a string leaf in place; anything else is a scrubber fault, refused
        // rather than panicked on (nothing in this crate panics on input).
        let Value::String(scrubbed) = value else {
            return Err(MemoryTextError::Unmasked(Unmasked {
                path: String::new(),
                rule: "type_changed",
            }));
        };
        if scrubbed.chars().count() > max {
            return Err(MemoryTextError::TooLong { max });
        }
        Ok(Self(scrubbed))
    }
}

impl core::fmt::Debug for MemoryText {
    /// `MemoryText { len: <bytes> }`, never the text.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MemoryText")
            .field("len", &self.0.len())
            .finish()
    }
}

/// Why a text was not accepted (plan D5). Nothing reached a store.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MemoryTextError {
    /// Empty or whitespace-only.
    #[error("the text is empty")]
    Empty,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrub::MinimalScrubber;

    fn plain() -> MinimalScrubber {
        MinimalScrubber::new(Vec::<String>::new())
    }

    #[test]
    fn a_blank_text_is_empty() {
        for raw in ["", "   \n\t"] {
            assert_eq!(
                MemoryText::subject(raw, &plain()),
                Err(MemoryTextError::Empty)
            );
            assert_eq!(MemoryText::body(raw, &plain()), Err(MemoryTextError::Empty));
        }
    }

    #[test]
    fn the_text_is_trimmed() {
        let text = MemoryText::subject("  Build cache \n", &plain()).expect("a subject");
        assert_eq!(text.as_str(), "Build cache");
        assert_eq!(text.into_string(), "Build cache");
    }

    #[test]
    fn one_character_over_the_cap_is_too_long() {
        let at = "a".repeat(MEMORY_SUBJECT_MAX);
        assert_eq!(
            MemoryText::subject(&at, &plain())
                .expect("at the cap")
                .as_str(),
            at
        );
        assert_eq!(
            MemoryText::subject(&"a".repeat(121), &plain()),
            Err(MemoryTextError::TooLong { max: 120 })
        );
        let at = "a".repeat(MEMORY_BODY_MAX);
        assert_eq!(
            MemoryText::body(&at, &plain())
                .expect("at the cap")
                .as_str(),
            at
        );
        assert_eq!(
            MemoryText::body(&"a".repeat(2001), &plain()),
            Err(MemoryTextError::TooLong { max: 2000 })
        );
    }

    #[test]
    fn the_cap_counts_characters_not_bytes() {
        let at = "é".repeat(120);
        assert_eq!(at.len(), 240);
        assert!(MemoryText::subject(&at, &plain()).is_ok());
        assert_eq!(
            MemoryText::subject(&"é".repeat(121), &plain()),
            Err(MemoryTextError::TooLong { max: 120 })
        );
    }

    #[test]
    fn a_known_secret_is_masked() {
        let scrubber = MinimalScrubber::new(["hunter2-secret".to_owned()]);
        let text = MemoryText::body("pass is hunter2-secret", &scrubber).expect("masked");
        assert_eq!(text.as_str(), "pass is [REDACTED]");
    }

    /// Blueprint P-1: the cap applies to the scrubbed text, so masking that lengthens a subject
    /// past the cap is refused here, not by the DB CHECK.
    #[test]
    fn masking_that_overflows_the_cap_is_too_long() {
        let scrubber = MinimalScrubber::new(["abcdef".to_owned()]);
        let raw = vec!["abcdef"; 17].join(" ");
        assert_eq!(raw.chars().count(), 118);
        assert!(MemoryText::subject(&raw, &plain()).is_ok(), "fits unmasked");
        assert_eq!(
            MemoryText::subject(&raw, &scrubber),
            Err(MemoryTextError::TooLong { max: 120 })
        );
    }

    #[test]
    fn a_credential_shaped_text_is_refused_without_repeating_it() {
        let err = MemoryText::body("key sk-ant-api03-do-not-print-me", &plain())
            .expect_err("credential-shaped");
        let MemoryTextError::Unmasked(u) = &err else {
            panic!("expected Unmasked, got {err:?}");
        };
        assert_eq!(u.rule, "anthropic_api_key");
        for shown in [err.to_string(), format!("{err:?}")] {
            assert!(!shown.contains("do-not-print-me"), "{shown}");
            assert!(!shown.contains("sk-ant-"), "{shown}");
        }
    }

    #[test]
    fn debug_prints_the_length_only() {
        let text = MemoryText::subject("Build cache", &plain()).expect("a subject");
        assert_eq!(format!("{text:?}"), "MemoryText { len: 11 }");
    }

    #[test]
    fn memory_subject_key_folds_like_btrim_lower() {
        assert_eq!(memory_subject_key(" Build Cache "), "build cache");
        assert_eq!(memory_subject_key("\tX"), "\tx", "btrim keeps a tab");
    }

    #[test]
    fn only_active_and_suspect_are_live() {
        let live: Vec<bool> = MemoryState::ALL.iter().map(|s| s.is_live()).collect();
        assert_eq!(live, [true, true, false, false]);
    }
}
