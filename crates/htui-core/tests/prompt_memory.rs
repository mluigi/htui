//! MOD-163 milestone 2, T1: the project memory section (plan D11-D17; blueprint §T1.4-§T1.12).
//!
//! A run freezes the project's `Active` entries at `StartRun` ([`select`]: newest first, an id
//! tiebreak, a hard cap that skips an entry rather than splitting it). Every phase step renders
//! them as the internal `memory` section after the persona and before the template body, as
//! quoted data with source and trust; the section's trailing blank line belongs to its own span,
//! so trimming it away leaves only sent bytes in `estimated_after`. `TrimStep::Memory` drops
//! whole entries off the tail after excerpts and before upstream, and `trim_record` v5 records
//! which entries reached the prompt. A spec with no memory renders exactly as before: the
//! `prompt_golden__*` snapshots are the byte proof.
#![cfg(feature = "test-support")]

use chrono::{DateTime, Utc};
use htui_core::model::{MemoryEntry, MemoryEntryId, MemoryKind, MemoryState};
use htui_core::prompt::memory::{MEMORY_TOKEN_CAP, select};
use htui_core::prompt::render::memory_entry_block;
use htui_core::prompt::{TokenEstimator, fixtures};
use uuid::Uuid;

/// `2026-11-01T00:00:00Z + hour` hours: a fixed clock for the entries.
fn at(hour: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_793_491_200 + hour * 3_600, 0).expect("in range")
}

/// An active entry `n` of kind fact, updated at `hour`.
fn entry(n: u128, subject: &str, body: &str, hour: i64) -> MemoryEntry {
    fixtures::memory_entry(n, MemoryKind::Fact, subject, body, at(hour))
}

/// `entry` in `state`.
fn in_state(mut e: MemoryEntry, state: MemoryState) -> MemoryEntry {
    e.state = state;
    e
}

fn id(n: u128) -> MemoryEntryId {
    MemoryEntryId::from_uuid(Uuid::from_u128(n))
}

/// The ids `select` kept, in order.
fn kept(entries: &[MemoryEntry], cap: i64) -> Vec<MemoryEntryId> {
    select(entries, cap, TokenEstimator::DEFAULT)
        .expect("an active entry")
        .entries
        .iter()
        .map(|e| e.id)
        .collect()
}

// -- select (D12, D13; blueprint P-3, P-4, P-8) --------------------------------------------------

#[test]
fn select_answers_none_with_no_active_entry() {
    assert_eq!(select(&[], MEMORY_TOKEN_CAP, TokenEstimator::DEFAULT), None);
    let closed = [
        in_state(entry(1, "a", "retired", 1), MemoryState::Retired),
        in_state(entry(2, "b", "superseded", 2), MemoryState::Superseded),
        in_state(entry(3, "c", "suspect", 3), MemoryState::Suspect),
    ];
    assert_eq!(
        select(&closed, MEMORY_TOKEN_CAP, TokenEstimator::DEFAULT),
        None,
        "only Active is live in M2"
    );
}

#[test]
fn select_takes_active_entries_only() {
    let entries = [
        entry(1, "one", "active one", 1),
        in_state(entry(2, "two", "retired", 2), MemoryState::Retired),
        in_state(entry(3, "three", "suspect", 3), MemoryState::Suspect),
        entry(4, "four", "active four", 4),
        in_state(entry(5, "five", "superseded", 5), MemoryState::Superseded),
    ];
    let memory = select(&entries, MEMORY_TOKEN_CAP, TokenEstimator::DEFAULT).expect("two active");
    assert_eq!(memory.total, 2, "total counts the active entries only");
    assert_eq!(
        memory.entries.iter().map(|e| e.id).collect::<Vec<_>>(),
        [id(4), id(1)]
    );
    assert_eq!(memory.entries[0].body, "active four", "the text is frozen");
    assert_eq!(memory.entries[0].version, 1);
}

#[test]
fn select_orders_newest_first_with_an_id_tiebreak() {
    // Handed over in no particular order: `select` re-sorts and never trusts the store's.
    let entries = [
        entry(3, "c", "older", 1),
        entry(9, "i", "newest", 5),
        entry(7, "g", "tied, larger id", 3),
        entry(2, "b", "tied, smaller id", 3),
    ];
    assert_eq!(
        kept(&entries, MEMORY_TOKEN_CAP),
        [id(9), id(2), id(7), id(3)]
    );
}

#[test]
fn select_skips_an_over_cap_entry_and_keeps_walking() {
    let newest = entry(1, "short new", "Use cargo nextest.", 3);
    let long = entry(2, "long", &"A long note about the build. ".repeat(40), 2);
    let oldest = entry(3, "short old", "Run the gate.", 1);
    let cost = |e: &MemoryEntry| {
        let frozen = htui_core::model::SnapshotMemoryEntry::freeze(
            e,
            htui_core::model::MemorySource::Person,
        );
        TokenEstimator::DEFAULT.estimate(&memory_entry_block(&frozen))
    };
    let cap = cost(&newest) + cost(&oldest);
    assert!(
        cost(&long) > cap,
        "the long entry alone is over the cap, so the test can fail"
    );
    let entries = [newest, long, oldest];
    let memory = select(&entries, cap, TokenEstimator::DEFAULT).expect("active entries");
    assert_eq!(
        memory.entries.iter().map(|e| e.id).collect::<Vec<_>>(),
        [id(1), id(3)],
        "the long middle entry is skipped and the walk continues to the shorter older one"
    );
    assert_eq!(
        memory.total, 3,
        "total counts what the project had, not what fit"
    );
    // One token less and the oldest no longer fits behind the newest.
    assert_eq!(kept(&entries, cap - 1), [id(1)]);
}

#[test]
fn select_keeps_every_entry_over_cap_out_as_some_empty() {
    let entries = [
        entry(1, "a", "first body", 2),
        entry(2, "b", "second body", 1),
    ];
    let memory = select(&entries, 5, TokenEstimator::DEFAULT).expect("Some: active entries exist");
    assert!(memory.entries.is_empty(), "{memory:?}");
    assert_eq!(memory.total, 2);
}

#[test]
fn select_records_the_estimator_id() {
    let entries = [entry(1, "a", "b", 1)];
    let memory = select(&entries, MEMORY_TOKEN_CAP, TokenEstimator::DEFAULT).expect("active");
    assert_eq!(memory.estimator, "chars-v2");
    let wide = select(&entries, MEMORY_TOKEN_CAP, TokenEstimator::WIDE).expect("active");
    assert_eq!(wide.estimator, "chars-v1-gpt");
}

#[test]
fn the_cap_is_fifteen_hundred_tokens() {
    assert_eq!(MEMORY_TOKEN_CAP, 1_500, "D13; MOD-169 tunes it");
}

#[test]
fn an_entry_block_is_the_documented_bytes() {
    let memory = fixtures::memory_two_of_three();
    assert_eq!(
        memory_entry_block(&memory.entries[0]),
        "<entry kind=\"gotcha\" subject=\"Build cache\" source=\"person\" \
         trust=\"person-written\">\nClear target/ when the sqlx offline data goes stale.\n</entry>"
    );
    assert_eq!(
        memory_entry_block(&memory.entries[1]),
        "<entry kind=\"decision\" subject=\"Errors &amp; &quot;context&quot;\" source=\"person\" \
         trust=\"person-written\">\nLibrary crates use thiserror; binaries use anyhow.\n</entry>"
    );
    // Blueprint §T1.9: the cap units of the worked example.
    assert_eq!(
        TokenEstimator::DEFAULT.estimate(&memory_entry_block(&memory.entries[0])),
        58
    );
    assert_eq!(
        TokenEstimator::DEFAULT.estimate(&memory_entry_block(&memory.entries[1])),
        66
    );
}
