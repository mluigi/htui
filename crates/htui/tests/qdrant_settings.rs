//! `Settings > Qdrant`, the section half (MOD-115 M3 L-sec, D-13; MOD-158 D9, D10): the URL row
//! over the database-wide `qdrant_url` setting, the key row, the old keyring URL's offer, the two
//! editors and the questions, drawn through a [`SectionBench`] and through the shell at 80 columns.
//!
//! Every snapshot the section is fed is its own reply built as a literal, so no case here reaches
//! the keyring or a database (`tests/connection.rs`'s rule): the bench has no store, and the one
//! shell case runs `--demo`, which answers every row not applicable without opening one. The
//! worker half of the section is `tests/qdrant_settings_pg.rs`'s and `tests/settings.rs`'s.
//!
//! No case types a real key: the one the key editor takes is a fixture, and the frame shows its
//! dots and count only.
#![cfg(feature = "testkit")]

use chrono::{DateTime, Utc};
use htui::app::{Action, Handled};
use htui::concepts_worker::QdrantCheck;
use htui::qdrant_settings_info::{
    OldUrl, QdrantSnapshot, QdrantState, URL_WRITE_OFFLINE, UrlRowState, old_entry_kept,
};
use htui::store_worker::{StoreReply, StoreRequest};
use htui::testkit::{Harness, SectionBench};
use htui::ui::Theme;
use htui::ui::tabs::settings::{QdrantSection, SettingsSection, SettingsTab};
use htui_core::qdrant::{QdrantUrl, QdrantUrlError};
use htui_core::store::StoreError;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::{Terminal, TerminalOptions, Viewport, backend::TestBackend};

/// The URL a set row carries.
const SUMMARY: &str = "https://qdrant.example:6334";

/// The URL another box set meanwhile.
const OTHER: &str = "https://other.example:6334";

/// What this box's keyring holds from before (D10).
const OLD: &str = "http://old.example:6334";

/// The key the key editor cases type: a fixture, never shown, so `contains` finding it means the
/// section drew it.
const TYPED_KEY: &str = "qk-fixture-0123456789";

/// The sentence a box with no unlocked collection answers.
const UNREADABLE_WHY: &str =
    "cannot read the keyring entry (htui/qdrant-key): Secret Service: no result found";

/// A row read that failed with something other than a lost server.
const ROW_WHY: &str = "the database answered: permission denied for table app_setting";

/// What the clear question on the URL row says, its answers under the compiled keys.
const CLEAR_URL: &str = "Remove the Qdrant URL for every box? y / n";

/// What `changed elsewhere` reads with no editor left open (`settings::CHANGED_ELSEWHERE_CLOSED`).
const CHANGED: &str = "changed elsewhere; nothing was written \u{2014} reopen the editor and retry";

fn url(text: &str) -> QdrantUrl {
    QdrantUrl::parse(text).expect("a valid URL")
}

/// The token the set row carries.
fn token() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

/// The token the row carries after another box wrote it.
fn later() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH + chrono::Duration::seconds(1)
}

/// A snapshot with `url` and `key_state`, no override and no old URL.
fn snapshot(url: UrlRowState, key_state: QdrantState) -> QdrantSnapshot {
    QdrantSnapshot {
        url,
        url_override: None,
        key_state,
        key_override: false,
        old_url: None,
    }
}

/// The row set and the key stored.
fn stored() -> QdrantSnapshot {
    snapshot(
        UrlRowState::Set {
            url: url(SUMMARY),
            token: token(),
        },
        QdrantState::Stored,
    )
}

/// No row and no key.
fn empty() -> QdrantSnapshot {
    snapshot(UrlRowState::NotSet, QdrantState::NotStored)
}

/// The row set and a keyring that answers nothing at all: the key row is not empty, unreadable.
fn unreadable() -> QdrantSnapshot {
    snapshot(
        UrlRowState::Set {
            url: url(SUMMARY),
            token: token(),
        },
        QdrantState::Unreadable(UNREADABLE_WHY.to_owned()),
    )
}

/// No row and this box's keyring holding `old` from before (D10).
fn offer(old: OldUrl) -> QdrantSnapshot {
    QdrantSnapshot {
        old_url: Some(old),
        ..empty()
    }
}

/// A bench and a section with `snapshot` already delivered as the read's reply.
async fn bench_with(snapshot: QdrantSnapshot) -> (SectionBench, QdrantSection) {
    let bench = SectionBench::new().await;
    let mut section = QdrantSection::new();
    bench.reply(&mut section, &StoreReply::Qdrant(snapshot));
    let _ = bench.drained();
    (bench, section)
}

/// Types `text` one key at a time.
fn typed(bench: &SectionBench, section: &mut QdrantSection, text: &str) {
    for c in text.chars() {
        assert_eq!(bench.key(section, &c.to_string()), Handled::Consumed);
    }
}

/// The section drawn into the bench's 100x30 buffer.
fn frame_of(bench: &SectionBench, section: &QdrantSection) -> String {
    bench.render_section(section, 100)
}

/// The frame as one line, so a sentence is found wherever it wrapped.
fn flattened(frame: &str) -> String {
    frame.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every store request the section sent since the queue was last drained, its probes included.
fn sent_by(bench: &SectionBench) -> Vec<StoreRequest> {
    bench
        .drained()
        .into_iter()
        .filter_map(|action| match action {
            Action::Store(request) => Some(request),
            _ => None,
        })
        .collect()
}

/// The store requests the section sent since the queue was last drained, but the Qdrant probe a
/// snapshot asks for (MOD-160 P-11): the probe cases below assert it themselves.
fn requests_of(bench: &SectionBench) -> Vec<StoreRequest> {
    sent_by(bench)
        .into_iter()
        .filter(|request| !matches!(request, StoreRequest::ProbeQdrant))
        .collect()
}

/// How many Qdrant probes the section sent since the queue was last drained.
fn probes_of(bench: &SectionBench) -> usize {
    sent_by(bench)
        .iter()
        .filter(|request| matches!(request, StoreRequest::ProbeQdrant))
        .count()
}

/// Both rows of a set URL and a stored key: the URL itself, the key by its presence only.
#[tokio::test]
async fn the_rows_show_what_is_stored_and_never_the_key() {
    let (bench, section) = bench_with(stored()).await;
    let frame = frame_of(&bench, &section);
    assert!(frame.contains(SUMMARY), "{frame}");
    assert!(!section.captures_input(), "a set URL opens nothing");
    insta::assert_snapshot!("stored", frame);
}

/// D9: the URL row says the setting is the database's, one for every box, set or not.
#[tokio::test]
async fn the_url_row_says_it_applies_to_every_box() {
    let (bench, section) = bench_with(stored()).await;
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!("{SUMMARY} \u{b7} for every box")),
        "{frame}"
    );

    let (bench, mut section) = bench_with(empty()).await;
    bench.key(&mut section, "esc");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(frame.contains("not set \u{b7} for every box"), "{frame}");
}

/// No row: the URL editor opens itself, once, over the sentence that says so.
#[tokio::test]
async fn no_url_row_opens_the_url_editor() {
    let (bench, section) = bench_with(empty()).await;
    assert!(section.captures_input(), "the URL editor opened itself");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(frame.contains("no Qdrant URL is set"), "{frame}");
    insta::assert_snapshot!("empty", frame_of(&bench, &section));
}

/// A key row that could not be read is never shown as an empty one, and opens no editor.
#[tokio::test]
async fn an_unreadable_keyring_says_so_and_opens_nothing() {
    let (bench, section) = bench_with(unreadable()).await;
    assert!(
        !section.captures_input(),
        "no field over a keyring nobody can read"
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(frame.contains("the keyring could not be read"), "{frame}");
    assert!(!frame.contains("not stored"), "{frame}");
    insta::assert_snapshot!("unreadable", frame_of(&bench, &section));
}

/// A URL row whose read failed says why, is never "not set", opens nothing, and refuses `e` and
/// `c` with its own words and the key that reads it again.
#[tokio::test]
async fn an_unreadable_url_row_says_so_and_refuses_e_and_c() {
    let (bench, mut section) = bench_with(snapshot(
        UrlRowState::Unreadable(ROW_WHY.to_owned()),
        QdrantState::NotStored,
    ))
    .await;
    assert!(!section.captures_input(), "no field over an unread row");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!("could not be read: {ROW_WHY}")),
        "{frame}"
    );
    assert!(!frame.contains("not set"), "{frame}");
    for chord in ["e", "c"] {
        assert_eq!(bench.key(&mut section, chord), Handled::Consumed);
        assert!(requests_of(&bench).is_empty(), "`{chord}` sends nothing");
        assert!(!section.captures_input(), "`{chord}` opens nothing");
        let frame = flattened(&frame_of(&bench, &section));
        assert!(
            frame.contains(&format!(
                "the Qdrant URL could not be read: {ROW_WHY}; r reads it again"
            )),
            "`{chord}`: {frame}"
        );
    }
}

/// D9: offline the URL row cannot be read, `e` and `c` on it are refused with the worker's
/// sentence, and the key, which is this box's keyring's, still opens its editor.
#[tokio::test]
async fn offline_the_url_row_cannot_be_read_and_e_is_refused() {
    let (bench, mut section) =
        bench_with(snapshot(UrlRowState::Offline, QdrantState::Stored)).await;
    assert!(!section.captures_input(), "no field over an offline row");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("cannot be read while Postgres is offline"),
        "{frame}"
    );
    insta::assert_snapshot!("offline", frame_of(&bench, &section));

    for chord in ["e", "c"] {
        assert_eq!(bench.key(&mut section, chord), Handled::Consumed);
        assert!(requests_of(&bench).is_empty(), "`{chord}` sends nothing");
        assert!(!section.captures_input(), "`{chord}` opens nothing");
        let frame = flattened(&frame_of(&bench, &section));
        assert!(frame.contains(URL_WRITE_OFFLINE), "`{chord}`: {frame}");
    }

    bench.key(&mut section, "j");
    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    assert!(section.captures_input(), "the key editor opens offline");
}

/// D3, D4: an override in force is said under its row: a usable `HTUI_QDRANT_URL`, which this
/// process uses instead of the row (so no editor opens over no row), and one that is not, with
/// D2's own sentence (blueprint P-20); the key's under the key row.
#[tokio::test]
async fn an_override_is_shown_under_its_row() {
    let with_override = QdrantSnapshot {
        url_override: Some(Ok(url("http://env.example:6334"))),
        key_override: true,
        ..empty()
    };
    let (bench, mut section) = bench_with(with_override).await;
    assert!(
        !section.captures_input(),
        "this process does not use the row: nothing opens"
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(
            "HTUI_QDRANT_URL=http://env.example:6334 is in force in this process, which does not \
             use this setting"
        ),
        "{frame}"
    );
    insta::assert_snapshot!("override", frame_of(&bench, &section));
    bench.key(&mut section, "j");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("HTUI_QDRANT_API_KEY is in force in this process"),
        "{frame}"
    );

    let (bench, section) = bench_with(QdrantSnapshot {
        url_override: Some(Err(QdrantUrlError::Scheme)),
        ..stored()
    })
    .await;
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!(
            "HTUI_QDRANT_URL is not usable ({}); this process cannot search",
            QdrantUrlError::Scheme
        )),
        "{frame}"
    );
}

/// `e` on the URL row over a set URL: a fresh field that replaces it for every box.
#[tokio::test]
async fn e_opens_the_url_editor_over_a_stored_url() {
    let (bench, mut section) = bench_with(stored()).await;
    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    typed(&bench, &mut section, "https://q.example:6334");
    let frame = frame_of(&bench, &section);
    assert!(frame.contains("https://q.example:6334"), "{frame}");
    assert!(
        flattened(&frame).contains("replaces the Qdrant URL for every box"),
        "{frame}"
    );
    insta::assert_snapshot!("editor", frame);
}

/// D9: the submit sends the URL with the row's token, so a row that moved meanwhile is a miss.
#[tokio::test]
async fn the_url_submit_carries_the_row_token() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "e");
    typed(&bench, &mut section, OTHER);
    assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);
    match requests_of(&bench).as_slice() {
        [
            StoreRequest::SetQdrantUrl {
                url: sent,
                expected,
            },
        ] => {
            assert_eq!(sent, &url(OTHER));
            assert_eq!(*expected, Some(token()));
        }
        other => panic!("one URL write, got {other:?}"),
    }
    assert!(!section.captures_input(), "the field closed");
    bench.reply(
        &mut section,
        &StoreReply::Qdrant(snapshot(
            UrlRowState::Set {
                url: url(OTHER),
                token: later(),
            },
            QdrantState::Stored,
        )),
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("the Qdrant URL is set for every box"),
        "{frame}"
    );

    // Over no row, the write expects none.
    let (bench, mut section) = bench_with(empty()).await;
    typed(&bench, &mut section, SUMMARY);
    bench.key(&mut section, "Enter");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::SetQdrantUrl { expected: None, .. }]
        ),
        "no row, no token"
    );
}

/// D2 in the field: a URL that carries credentials is refused before anything is sent, with D2's
/// own sentence under the field, which stays open.
#[tokio::test]
async fn the_field_refuses_a_url_with_credentials_before_sending() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "e");
    typed(&bench, &mut section, "http://user:pass@q.example:6334");
    assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);
    assert!(requests_of(&bench).is_empty(), "nothing is sent");
    assert!(section.captures_input(), "the field stays open");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&flattened(&QdrantUrlError::Credentials.to_string())),
        "{frame}"
    );
}

/// MOD-158 R1 (H2): a host outside the name characters is refused in the field with
/// `QdrantUrlError::Host`'s own sentence (blueprint P-20), before anything is sent.
#[tokio::test]
async fn the_field_refuses_a_host_with_odd_characters_before_sending() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "e");
    typed(&bench, &mut section, "http://q.example{x}:6334");
    assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);
    assert!(requests_of(&bench).is_empty(), "nothing is sent");
    assert!(section.captures_input(), "the field stays open");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&flattened(&QdrantUrlError::Host.to_string())),
        "{frame}"
    );
}

/// R-3: a stale reply says the row changed elsewhere, shows the row now, and the next write
/// presents the fresh token.
#[tokio::test]
async fn a_stale_reply_says_changed_elsewhere() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "e");
    typed(&bench, &mut section, "https://mine.example:6334");
    bench.key(&mut section, "Enter");
    let _ = requests_of(&bench);
    bench.reply(
        &mut section,
        &StoreReply::QdrantStale(Box::new(snapshot(
            UrlRowState::Set {
                url: url(OTHER),
                token: later(),
            },
            QdrantState::Stored,
        ))),
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(frame.contains(CHANGED), "{frame}");
    assert!(frame.contains(OTHER), "the row now: {frame}");
    assert!(
        !bench.key_hint(&section).contains('\u{2026}'),
        "nothing in flight"
    );

    bench.key(&mut section, "e");
    typed(&bench, &mut section, "https://mine.example:6334");
    bench.key(&mut section, "Enter");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::SetQdrantUrl { expected: Some(at), .. }] if *at == later()
        ),
        "the fresh token"
    );
}

/// P-3: a row that does not parse says why and that it is every box's, and can be overwritten and
/// cleared by its token.
#[tokio::test]
async fn a_malformed_row_can_be_overwritten_and_cleared() {
    let malformed = snapshot(
        UrlRowState::Unusable {
            why: QdrantUrlError::NotAString,
            token: token(),
        },
        QdrantState::NotStored,
    );
    let (bench, mut section) = bench_with(malformed).await;
    assert!(!section.captures_input(), "a row is there: nothing opens");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!(
            "not usable: {} \u{b7} for every box",
            QdrantUrlError::NotAString
        )),
        "{frame}"
    );

    bench.key(&mut section, "e");
    typed(&bench, &mut section, SUMMARY);
    bench.key(&mut section, "Enter");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::SetQdrantUrl { expected: Some(at), .. }] if *at == token()
        ),
        "an overwrite by the row's token"
    );

    let (bench, mut section) = bench_with(QdrantSnapshot {
        url: UrlRowState::Unusable {
            why: QdrantUrlError::NotAString,
            token: token(),
        },
        ..empty()
    })
    .await;
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::ClearQdrantUrl { expected }] if *expected == token()
        ),
        "a clear by the row's token"
    );
}

/// `e` on the key row: a masked field, its dots and count only.
#[tokio::test]
async fn e_opens_the_masked_key_editor() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "j");
    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    typed(&bench, &mut section, TYPED_KEY);
    let frame = frame_of(&bench, &section);
    assert!(!frame.contains(TYPED_KEY), "the key is never drawn");
    assert!(!frame.contains("qk-"), "not even a fragment of it");
    assert!(frame.contains("(21)"), "the count beside the dots: {frame}");
    insta::assert_snapshot!("editor_key", frame);
}

/// Q2: `c` on the URL row asks before it clears the URL for every box.
#[tokio::test]
async fn c_asks_before_it_clears() {
    let (bench, mut section) = bench_with(stored()).await;
    assert_eq!(bench.key(&mut section, "c"), Handled::Consumed);
    assert!(bench.drained().is_empty(), "the question sends nothing");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(frame.contains(CLEAR_URL), "{frame}");
    insta::assert_snapshot!("confirm", frame_of(&bench, &section));
    bench.key(&mut section, "y");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::ClearQdrantUrl { expected }] if *expected == token()
        ),
        "the URL, by its token"
    );
}

/// Q2: `c` on the key row asks about this box's key alone, and clears only that.
#[tokio::test]
async fn c_on_the_key_row_clears_only_this_box_s_key() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "j");
    assert_eq!(bench.key(&mut section, "c"), Handled::Consumed);
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("Remove this box's Qdrant API key from the keyring? y / n"),
        "{frame}"
    );
    assert!(!frame.contains("every box?"), "{frame}");
    bench.key(&mut section, "y");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::ClearQdrantApiKey]
        ),
        "the key alone"
    );
    bench.reply(
        &mut section,
        &StoreReply::Qdrant(snapshot(
            UrlRowState::Set {
                url: url(SUMMARY),
                token: token(),
            },
            QdrantState::NotStored,
        )),
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("this box's API key is gone from the keyring"),
        "{frame}"
    );
}

/// D10: the old keyring URL is a row of its own under the URL row, naming the keys that use and
/// discard it; no editor opens over no row while it is offered.
#[tokio::test]
async fn the_offer_row_sits_under_the_url_row() {
    let (bench, section) = bench_with(offer(OldUrl::Usable(url(OLD)))).await;
    assert!(!section.captures_input(), "the offer is the way to a URL");
    let buffer = drawn(&bench, &section);
    // Each row by its label, after the gutter.
    let at = |label: &str| {
        (0..buffer.area.height)
            .find(|y| {
                row(&buffer, *y)
                    .chars()
                    .skip(1)
                    .collect::<String>()
                    .starts_with(label)
            })
            .unwrap_or_else(|| panic!("{label} is drawn"))
    };
    let (url_row, offer_row, key_row) = (at("URL "), at("old URL"), at("API key"));
    assert!(
        url_row < offer_row && offer_row < key_row,
        "URL, old URL, key"
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!(
            "this box's keyring holds {OLD} from before: u uses it for every box, c discards it"
        )),
        "{frame}"
    );
    insta::assert_snapshot!("offer", frame_of(&bench, &section));
}

/// D10: `u` on the offer row asks, then adopts; anywhere else it is not the section's.
#[tokio::test]
async fn u_asks_then_adopts() {
    let (bench, mut section) = bench_with(offer(OldUrl::Usable(url(OLD)))).await;
    assert_eq!(bench.key(&mut section, "u"), Handled::Pass, "the URL row");
    assert!(requests_of(&bench).is_empty());

    bench.key(&mut section, "j");
    assert_eq!(
        bench.key_hint(&section),
        "u use for every box \u{b7} c discard \u{b7} r reload \u{b7} j/k rows"
    );
    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("u uses the old URL, c discards it"),
        "{frame}"
    );
    assert_eq!(bench.key(&mut section, "u"), Handled::Consumed);
    assert!(requests_of(&bench).is_empty(), "the question sends nothing");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!("Use {OLD} for every box? y / n")),
        "{frame}"
    );
    bench.key(&mut section, "y");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::AdoptOldQdrantUrl]
        ),
        "the adopt"
    );
    let hint = bench.key_hint(&section);
    assert!(
        hint.ends_with("using the old Qdrant URL for every box\u{2026}"),
        "{hint}"
    );
    bench.reply(
        &mut section,
        &StoreReply::Qdrant(snapshot(
            UrlRowState::Set {
                url: url(OLD),
                token: token(),
            },
            QdrantState::NotStored,
        )),
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(
            "the old URL is now the Qdrant URL for every box; this box's keyring entry is gone"
        ),
        "{frame}"
    );
    assert!(
        !frame.contains("from before"),
        "the offer row is gone: {frame}"
    );
}

/// D10: an old URL that fails D2, or one offered over a row already set, can only be discarded.
#[tokio::test]
async fn an_unusable_old_url_can_only_be_discarded() {
    let (bench, mut section) =
        bench_with(offer(OldUrl::Unusable(QdrantUrlError::Credentials))).await;
    let why = QdrantUrlError::Credentials.to_string();
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&flattened(&format!(
            "this box's keyring holds an old Qdrant URL that is not usable ({why}): c discards it"
        ))),
        "{frame}"
    );
    insta::assert_snapshot!("offer_discard_only", frame_of(&bench, &section));

    bench.key(&mut section, "j");
    assert_eq!(
        bench.key_hint(&section),
        "c discard \u{b7} r reload \u{b7} j/k rows"
    );
    assert_eq!(bench.key(&mut section, "u"), Handled::Consumed);
    assert!(requests_of(&bench).is_empty(), "nothing is adopted");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&flattened(&format!(
            "the old Qdrant URL is not usable ({why}); c discards it"
        ))),
        "{frame}"
    );
    assert_eq!(bench.key(&mut section, "c"), Handled::Consumed);
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("Discard the old Qdrant URL from this box's keyring? y / n"),
        "{frame}"
    );
    bench.key(&mut section, "y");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::DiscardOldQdrantUrl]
        ),
        "the discard"
    );
    bench.reply(&mut section, &StoreReply::Qdrant(empty()));
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("the old Qdrant URL is gone from this box's keyring"),
        "{frame}"
    );

    // Usable, but a row is already set: no longer used, discard only.
    let (bench, mut section) = bench_with(QdrantSnapshot {
        old_url: Some(OldUrl::Usable(url(OLD))),
        ..stored()
    })
    .await;
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!(
            "this box's keyring holds {OLD} from before, no longer used: c discards it"
        )),
        "{frame}"
    );
    bench.key(&mut section, "j");
    bench.key(&mut section, "u");
    assert!(requests_of(&bench).is_empty(), "nothing is adopted");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("a Qdrant URL is already set for every box; c discards it"),
        "{frame}"
    );
}

/// The old URL offered over no row, the key stored.
fn offered_with_key() -> QdrantSnapshot {
    QdrantSnapshot {
        key_state: QdrantState::Stored,
        ..offer(OldUrl::Usable(url(OLD)))
    }
}

/// The y of the first drawn line holding `text`.
fn line_of(buffer: &Buffer, text: &str) -> u16 {
    (0..buffer.area.height)
        .find(|y| row(buffer, *y).contains(text))
        .unwrap_or_else(|| panic!("{text:?} is drawn"))
}

/// MOD-158 R1 (C-M1, C-L8): a write that fails part-way (the adopt's row landed, the old keyring
/// entry stayed) leaves no stale view: the section reads itself again, keeps the failure's
/// sentence, and then draws what is there now, so a second `u` is told the row is set.
#[tokio::test]
async fn a_failed_write_reads_the_section_again() {
    let (bench, mut section) = bench_with(offer(OldUrl::Usable(url(OLD)))).await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "u");
    bench.key(&mut section, "y");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::AdoptOldQdrantUrl]
        ),
        "the adopt"
    );
    let kept = old_entry_kept(&StoreError::Backend("the collection is locked".to_owned()));
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "adopt_old_qdrant_url",
            message: kept.clone(),
        },
    );
    assert!(
        matches!(requests_of(&bench).as_slice(), [StoreRequest::QdrantInfo]),
        "a failed write reads the section again"
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(frame.contains(&flattened(&kept)), "{frame}");

    bench.reply(
        &mut section,
        &StoreReply::Qdrant(QdrantSnapshot {
            old_url: Some(OldUrl::Usable(url(OLD))),
            ..snapshot(
                UrlRowState::Set {
                    url: url(OLD),
                    token: token(),
                },
                QdrantState::NotStored,
            )
        }),
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!("{OLD} \u{b7} for every box")),
        "the row as it is now: {frame}"
    );
    assert!(frame.contains("from before, no longer used"), "{frame}");
    assert!(
        frame.contains(&flattened(&kept)),
        "the failure's sentence stays: {frame}"
    );
    assert!(!section.captures_input(), "nothing opened by the re-read");

    bench.key(&mut section, "u");
    assert!(requests_of(&bench).is_empty(), "nothing is adopted twice");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("a Qdrant URL is already set for every box"),
        "{frame}"
    );
    assert!(!frame.contains("changed elsewhere"), "{frame}");
}

/// MOD-158 R1 (C-L4, C-L8): a stale reply to the adopt says the URL changed elsewhere in words
/// that name no editor (none was open), and draws the row now.
#[tokio::test]
async fn a_stale_adopt_says_so_without_an_editor() {
    let (bench, mut section) = bench_with(offer(OldUrl::Usable(url(OLD)))).await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "u");
    bench.key(&mut section, "y");
    let _ = requests_of(&bench);
    bench.reply(
        &mut section,
        &StoreReply::QdrantStale(Box::new(QdrantSnapshot {
            old_url: Some(OldUrl::Usable(url(OLD))),
            ..snapshot(
                UrlRowState::Set {
                    url: url(OTHER),
                    token: later(),
                },
                QdrantState::NotStored,
            )
        })),
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!("{OTHER} \u{b7} for every box")),
        "{frame}"
    );
    assert!(
        frame.contains(
            "changed elsewhere; nothing was written \u{2014} the URL row shows the Qdrant URL now"
        ),
        "{frame}"
    );
    assert!(!frame.contains("reopen the editor"), "{frame}");
    assert!(requests_of(&bench).is_empty(), "a stale reply is the read");
}

/// MOD-158 R1 (C-L1): the adopt's question, asked while the old URL could be used, never turns
/// into the discard's when a snapshot says it no longer can: it asks about the old URL generically.
#[tokio::test]
async fn the_adopt_question_never_reads_as_the_discard() {
    let (bench, mut section) = bench_with(offer(OldUrl::Usable(url(OLD)))).await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "u");
    bench.reply(
        &mut section,
        &StoreReply::Qdrant(offer(OldUrl::Unusable(QdrantUrlError::Credentials))),
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("Use the old Qdrant URL for every box? y / n"),
        "{frame}"
    );
    assert!(!frame.contains("Discard the old Qdrant URL"), "{frame}");
}

/// MOD-158 R1 (C-L2): a question stays on the row it was asked on when a snapshot adds the old
/// URL's row above it, and the cursor follows that row; a question whose row is gone closes.
#[tokio::test]
async fn a_question_stays_on_its_row_when_the_rows_move() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "c");
    bench.reply(
        &mut section,
        &StoreReply::Qdrant(QdrantSnapshot {
            old_url: Some(OldUrl::Usable(url(OLD))),
            ..stored()
        }),
    );
    let buffer = drawn(&bench, &section);
    let (offer_at, key_at, asked_at) = (
        line_of(&buffer, "old URL"),
        line_of(&buffer, "API key"),
        line_of(&buffer, "Remove this box's Qdrant API key"),
    );
    assert!(
        offer_at < key_at && key_at < asked_at,
        "the question under the key row: {offer_at} {key_at} {asked_at}"
    );
    bench.key(&mut section, "y");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::ClearQdrantApiKey]
        ),
        "the key's clear"
    );
    let hint = bench.key_hint(&section);
    assert!(
        hint.starts_with("e edit \u{b7} c clear \u{b7} r reload \u{b7} j/k rows"),
        "the cursor followed the key row, not the old URL's: {hint}"
    );

    // The old URL's row goes while its question is up: the question goes with it.
    let (bench, mut section) = bench_with(offered_with_key()).await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "c");
    assert!(section.captures_input(), "the discard's question");
    bench.reply(&mut section, &StoreReply::Qdrant(stored()));
    assert!(
        !section.captures_input(),
        "its row is gone, so is the question"
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(!frame.contains("Discard the old"), "{frame}");
    assert!(requests_of(&bench).is_empty());
}

/// MOD-158 R1 (C-L8): at 80 columns, the old URL's row wraps under its label, and each override
/// is drawn under its row while that row is focused: the URL's first, then the key's.
#[tokio::test]
async fn the_offer_and_overrides_at_80_columns() {
    let (bench, mut section) = bench_with(QdrantSnapshot {
        url_override: Some(Ok(url(OTHER))),
        key_override: true,
        ..offered_with_key()
    })
    .await;
    let frame = bench.render_section(&section, 80);
    let flat = flattened(&frame);
    assert!(
        flat.contains(&format!("this box's keyring holds {OLD} from before")),
        "{flat}"
    );
    assert!(flat.contains(&format!("HTUI_QDRANT_URL={OTHER}")), "{flat}");
    insta::assert_snapshot!("offer_url_override_80", frame);

    bench.key(&mut section, "j");
    bench.key(&mut section, "j");
    let frame = bench.render_section(&section, 80);
    let flat = flattened(&frame);
    assert!(flat.contains("HTUI_QDRANT_API_KEY is in force"), "{flat}");
    insta::assert_snapshot!("offer_key_override_80", frame);
}

/// MOD-81: the section at 80 columns, through the shell (`--demo`: no keyring is read).
#[tokio::test]
async fn the_section_at_80_columns() {
    let mut harness = Harness::demo()
        .size(80, 24)
        .with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
            QdrantSection::new(),
        )])));
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("n/a in a demo session"), "{frame}");
    insta::assert_snapshot!("demo_80", frame);
}

/// The section asks for its own read and is called Qdrant.
#[tokio::test]
async fn the_section_names_itself() {
    let section = QdrantSection::new();
    assert_eq!(section.id(), QdrantSection::ID);
    assert_eq!(section.title(), "Qdrant");
}

// ---- MOD-115 M3 (lane L-sec): the section on the kit ----------------------------------------------

/// `field.submit` and `field.cancel` moved (MOD-115 M3 C6): every field's `Enter` and `Esc`.
const FIELD_REBOUND: &str = "version = 2\n[field]\nsubmit = [\"ctrl-t\"]\ncancel = [\"ctrl-o\"]\n";

/// A bench over `keys`, `snapshot` already delivered.
async fn bench_with_keys(file: &str, snapshot: QdrantSnapshot) -> (SectionBench, QdrantSection) {
    let keys = htui::keys::load_str(file).expect("the key file loads");
    let bench = SectionBench::new().await.with_keys(keys);
    let mut section = QdrantSection::new();
    bench.reply(&mut section, &StoreReply::Qdrant(snapshot));
    let _ = bench.drained();
    (bench, section)
}

/// One section drawn into a 100x30 buffer: a marker's colour is a style, not text.
fn drawn(bench: &SectionBench, section: &QdrantSection) -> Buffer {
    let area = Rect::new(0, 0, 100, 30);
    let mut terminal = Terminal::with_options(
        TestBackend::new(100, 30),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )
    .expect("a test terminal");
    let ctx = bench.ctx();
    terminal
        .draw(|frame| section.render(frame, frame.area(), &ctx))
        .expect("the section draws");
    terminal.backend().buffer().clone()
}

/// Row `y` of `buffer` as text, its trailing blanks dropped.
fn row(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Both editors' hints read as before under the compiled keys and name the rebound chords once
/// `[field]` moves them (MD-A, C6).
#[tokio::test]
async fn every_field_hint_names_the_field_keys_in_force() {
    let (bench, mut section) = bench_with_keys(FIELD_REBOUND, stored()).await;
    let compiled = SectionBench::new().await;

    bench.key(&mut section, "e");
    assert_eq!(compiled.key_hint(&section), "Enter store \u{b7} Esc cancel");
    assert_eq!(
        bench.key_hint(&section),
        "Ctrl+t store \u{b7} Ctrl+o cancel"
    );
    bench.key(&mut section, "esc");

    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    assert_eq!(
        compiled.key_hint(&section),
        "Enter store \u{b7} Esc cancel \u{b7} typed text is never shown"
    );
    assert_eq!(
        bench.key_hint(&section),
        "Ctrl+t store \u{b7} Ctrl+o cancel \u{b7} typed text is never shown"
    );
}

/// The guides under the URL field name the submit key in force (C5, D-15): no row, and a set
/// one.
#[tokio::test]
async fn the_guides_name_the_submit_key_in_force() {
    let (bench, section) = bench_with_keys(FIELD_REBOUND, empty()).await;
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("no Qdrant URL is set; type one and press Ctrl+t"),
        "{frame}"
    );

    let (bench, mut section) = bench_with_keys(FIELD_REBOUND, stored()).await;
    bench.key(&mut section, "e");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("Ctrl+t replaces the Qdrant URL for every box"),
        "{frame}"
    );
    assert!(!frame.contains("Enter"), "{frame}");
}

/// `Enter API key` named a verb with a key's name (C5): the key field's guide is a sentence.
#[tokio::test]
async fn the_key_guide_is_a_sentence() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("type the API key, or leave it blank if there is none"),
        "{frame}"
    );
    assert!(!frame.contains("Enter API key"), "{frame}");
}

/// The field is its own row right under the row it replaces (D3, audit item 3), behind the
/// form's marker, with what is set still on screen; no label is drawn in the accent: only the
/// focused row's gutter carries the tab's (MOD-85).
#[tokio::test]
async fn the_field_is_under_the_row_it_replaces() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "e");
    typed(&bench, &mut section, "https://q.example");
    let buffer = drawn(&bench, &section);
    let url_row = (0..buffer.area.height)
        .find(|y| {
            row(&buffer, *y).contains("https://q.example") && row(&buffer, *y).contains("new URL")
        })
        .expect("the field is drawn");
    let text = row(&buffer, url_row);
    assert!(text.starts_with("\u{258c}new URL"), "{text}");
    let above = row(&buffer, url_row - 1);
    assert!(
        above.starts_with(" URL") && above.contains(SUMMARY),
        "the row it replaces is right above: {above}"
    );
    assert!(
        !(0..buffer.area.height).any(|y| row(&buffer, y).contains("URL:")),
        "the old field under the rows is gone"
    );
    let theme = Theme::default();
    let accent = theme.tab_accent("settings");
    assert_eq!(buffer[(0, url_row)].fg, accent.fg.unwrap_or(Color::Reset));
    for y in 0..buffer.area.height {
        for x in 1..buffer.area.width {
            assert_ne!(
                buffer[(x, y)].style().fg,
                theme.accent.fg,
                "no label in the accent: {:?} at ({x}, {y})",
                row(&buffer, y)
            );
        }
    }
}

/// The key row is a secret value: dots and a word, never a placeholder that looks like data.
#[tokio::test]
async fn a_stored_key_reads_as_a_secret() {
    let (bench, section) = bench_with(stored()).await;
    let frame = frame_of(&bench, &section);
    assert!(
        frame.contains("\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022} stored"),
        "{frame}"
    );
    assert!(!frame.contains("<redacted>"), "{frame}");
}

/// MOD-83: an unreadable keyring names why, so the user knows where to look.
#[tokio::test]
async fn an_unreadable_keyring_names_the_reason() {
    let (bench, section) = bench_with(unreadable()).await;
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!("the keyring could not be read: {UNREADABLE_WHY}")),
        "{frame}"
    );
}

/// MOD-83: a refused read says so, and what brings the rows back.
#[tokio::test]
async fn a_refused_read_says_unavailable_and_what_to_do() {
    let bench = SectionBench::new().await;
    let mut section = QdrantSection::new();
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "qdrant_info",
            message: "the keyring is locked".to_owned(),
        },
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("Qdrant settings are unavailable: the keyring is locked"),
        "{frame}"
    );
    assert!(
        frame.contains("r asks again once the store answers"),
        "{frame}"
    );
}

/// MOD-83, Q2: `c` on a row with nothing in it says so and names the key that fills it, rather
/// than calling the settings unavailable: the URL row, then the key row.
#[tokio::test]
async fn c_with_nothing_stored_names_the_way_forward() {
    let (bench, mut section) = bench_with(empty()).await;
    bench.key(&mut section, "esc");
    assert_eq!(bench.key(&mut section, "c"), Handled::Consumed);
    assert!(bench.drained().is_empty());
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("no Qdrant URL is set, so there is nothing to clear; e sets one"),
        "{frame}"
    );
    assert!(!frame.contains("unavailable"), "{frame}");

    bench.key(&mut section, "j");
    assert_eq!(bench.key(&mut section, "c"), Handled::Consumed);
    assert!(bench.drained().is_empty());
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(
            "no Qdrant API key is stored on this box, so there is nothing to clear; e stores one"
        ),
        "{frame}"
    );
}

/// MOD-83: `c` on an unreadable key row says the keyring could not be read, with why and the key
/// that reads it again, and never that nothing is stored: nobody knows that.
#[tokio::test]
async fn c_over_an_unreadable_keyring_never_says_nothing_is_stored() {
    let (bench, mut section) = bench_with(unreadable()).await;
    bench.key(&mut section, "j");
    assert_eq!(bench.key(&mut section, "c"), Handled::Consumed);
    assert!(bench.drained().is_empty(), "nothing is cleared");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!(
            "the keyring could not be read: {UNREADABLE_WHY}; r reads it again"
        )),
        "{frame}"
    );
    assert!(!frame.contains("is stored"), "{frame}");
    assert!(!frame.contains("nothing to clear"), "{frame}");
}

/// The same over no row and an unreadable key: one unknown row is enough to make "nothing is
/// stored" a guess.
#[tokio::test]
async fn c_over_an_unreadable_key_row_never_says_nothing_is_stored() {
    let (bench, mut section) = bench_with(snapshot(
        UrlRowState::NotSet,
        QdrantState::Unreadable(UNREADABLE_WHY.to_owned()),
    ))
    .await;
    bench.key(&mut section, "esc");
    bench.key(&mut section, "j");
    assert_eq!(bench.key(&mut section, "c"), Handled::Consumed);
    assert!(bench.drained().is_empty(), "nothing is cleared");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!(
            "the keyring could not be read: {UNREADABLE_WHY}; r reads it again"
        )),
        "{frame}"
    );
    assert!(!frame.contains("is stored"), "{frame}");
}

/// A write in flight reads in words, never as a store request's name.
#[tokio::test]
async fn a_write_in_flight_reads_in_words() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    let _ = bench.drained();
    let hint = bench.key_hint(&section);
    assert!(
        hint.ends_with("\u{b7} clearing the Qdrant URL for every box\u{2026}"),
        "{hint}"
    );
    bench.key(&mut section, "e");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(
            "still clearing the Qdrant URL for every box \u{2014} wait for the store's answer"
        ),
        "{frame}"
    );
    assert!(!frame.contains('_'), "no request name: {frame}");
}

/// The clear question is the focused row's error line, under the row it is about.
#[tokio::test]
async fn the_clear_question_sits_under_its_row() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "c");
    let buffer = drawn(&bench, &section);
    let url = (0..buffer.area.height)
        .find(|y| row(&buffer, *y).starts_with("\u{258c}URL"))
        .expect("the URL row is focused");
    assert!(
        row(&buffer, url + 1).contains(CLEAR_URL),
        "{:?}",
        row(&buffer, url + 1)
    );
}

/// MOD-115 M3 RV2 (RV1 M2's span): the scroll names the rows the form was drawn into, so the
/// pane's bar is measured against them; the total is the form's own lines, unpadded.
#[tokio::test]
async fn the_scroll_names_the_form_rows_it_measures() {
    let (bench, section) = bench_with(stored()).await;
    let _ = drawn(&bench, &section);
    let span = section.scroll().expect("the form reports its scroll");
    let rows = span.rows.expect("the rect the form was drawn into");
    assert_eq!((rows.x, rows.y, rows.width), (0, 0, 100), "{rows:?}");
    assert!(span.total < 30, "the form's own lines, unpadded: {span:?}");
    assert_eq!(span.offset, 0, "{span:?}");
}

// ---- MOD-160 D9: the probe's warning and info rows -----------------------------------------------

/// The key the probe cases' snapshots stand for: never in a check, so never drawn.
const KEY_SENTINEL: &str = "KEY-SENTINEL";

/// What a probe of each state answers, its sentence a fixture in the shape the probe writes.
fn no_url() -> QdrantCheck {
    QdrantCheck::NotConfigured(
        "no Qdrant URL is set; set one in Settings > Qdrant or with `htui --set-qdrant-url`"
            .to_owned(),
    )
}

fn unreachable() -> QdrantCheck {
    QdrantCheck::Unavailable(
        "cannot reach Qdrant at https://qdrant.example:6334: connection refused".to_owned(),
    )
}

fn mismatch() -> QdrantCheck {
    QdrantCheck::Unavailable(
        "qdrant: embedder mismatch: the collection was written by another embedder".to_owned(),
    )
}

fn key_refused() -> QdrantCheck {
    QdrantCheck::KeyRefused("qdrant: access refused: the API key was not accepted".to_owned())
}

fn no_key() -> QdrantCheck {
    QdrantCheck::NoKey("qdrant: access refused: an API key is required".to_owned())
}

/// The section over `snapshot` with `check` delivered as its probe's answer.
async fn probed(
    snapshot: QdrantSnapshot,
    check: Option<QdrantCheck>,
) -> (SectionBench, QdrantSection) {
    let (bench, mut section) = bench_with(snapshot).await;
    bench.reply(&mut section, &StoreReply::QdrantProbe(check));
    let _ = bench.drained();
    (bench, section)
}

/// D9: the section's snapshot asks for one probe, and the same snapshot again asks for none; a
/// snapshot that moved (the key cleared) asks again.
#[tokio::test]
async fn a_snapshot_asks_for_one_probe() {
    let bench = SectionBench::new().await;
    let mut section = QdrantSection::new();
    bench.reply(&mut section, &StoreReply::Qdrant(stored()));
    assert!(
        matches!(sent_by(&bench).as_slice(), [StoreRequest::ProbeQdrant]),
        "one probe for the snapshot"
    );
}

#[tokio::test]
async fn the_same_snapshot_does_not_ask_again() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.reply(&mut section, &StoreReply::Qdrant(stored()));
    assert_eq!(probes_of(&bench), 0, "nothing moved, nothing probed");

    bench.reply(
        &mut section,
        &StoreReply::Qdrant(QdrantSnapshot {
            key_state: QdrantState::NotStored,
            ..stored()
        }),
    );
    assert_eq!(probes_of(&bench), 1, "the key moved: probed again");
}

/// D9, "on its reload key": `r` reads the section again, and that snapshot probes again even
/// though nothing moved.
#[tokio::test]
async fn r_probes_again() {
    let (bench, mut section) = bench_with(stored()).await;
    assert_eq!(bench.key(&mut section, "r"), Handled::Consumed);
    assert!(
        matches!(sent_by(&bench).as_slice(), [StoreRequest::QdrantInfo]),
        "the read first"
    );
    bench.reply(&mut section, &StoreReply::Qdrant(stored()));
    assert!(
        matches!(sent_by(&bench).as_slice(), [StoreRequest::ProbeQdrant]),
        "then the probe"
    );
}

/// D9: offline and `--demo` send no probe (the URL row already says why), and a probe answered
/// before the store went offline draws nothing over its row.
#[tokio::test]
async fn offline_and_demo_send_no_probe() {
    for snapshot in [
        snapshot(UrlRowState::Offline, QdrantState::Stored),
        QdrantSnapshot::not_applicable(),
    ] {
        let bench = SectionBench::new().await;
        let mut section = QdrantSection::new();
        bench.reply(&mut section, &StoreReply::Qdrant(snapshot.clone()));
        assert_eq!(probes_of(&bench), 0, "{snapshot:?}");
    }

    let (bench, mut section) = probed(stored(), Some(unreachable())).await;
    bench.reply(
        &mut section,
        &StoreReply::Qdrant(snapshot(UrlRowState::Offline, QdrantState::Stored)),
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(!frame.contains("knowledge search"), "{frame}");
}

/// D9: each warning state draws its row, `knowledge search unavailable: <state>: <cause>`, under
/// the rows, in the theme's warning style.
#[tokio::test]
async fn each_warning_state_draws_its_row() {
    let cases = [
        ("probe_no_url", empty(), no_url(), "no Qdrant is configured"),
        (
            "probe_unreachable",
            stored(),
            unreachable(),
            "Qdrant is unreachable or refuses the collection",
        ),
        (
            "probe_mismatch",
            stored(),
            mismatch(),
            "Qdrant is unreachable or refuses the collection",
        ),
        (
            "probe_key_refused",
            stored(),
            key_refused(),
            "the Qdrant refused this box's API key",
        ),
        (
            "probe_no_key",
            snapshot(
                UrlRowState::Set {
                    url: url(SUMMARY),
                    token: token(),
                },
                QdrantState::NotStored,
            ),
            no_key(),
            "this box lacks the Qdrant API key",
        ),
    ];
    for (name, snapshot, check, state) in cases {
        let (bench, section) = probed(snapshot, Some(check.clone())).await;
        let frame = frame_of(&bench, &section);
        let warning = check.warning().expect("a warning state");
        assert!(warning.starts_with(state), "{warning}");
        assert!(
            flattened(&frame).contains(&format!("knowledge search unavailable: {warning}")),
            "{name}: {frame}"
        );
        let buffer = drawn(&bench, &section);
        let y = line_of(&buffer, "knowledge search unavailable");
        assert_eq!(
            buffer[(0, y)].fg,
            Theme::default().warning.fg.unwrap_or(Color::Reset),
            "{name}: the warning style"
        );
        insta::assert_snapshot!(name, frame);
    }
}

/// Q4: a Qdrant that answers without a key is said, as information, never as a warning.
#[tokio::test]
async fn a_keyless_qdrant_draws_an_info_line() {
    let keyless = snapshot(
        UrlRowState::Set {
            url: url(SUMMARY),
            token: token(),
        },
        QdrantState::NotStored,
    );
    let (bench, section) = probed(keyless, Some(QdrantCheck::Keyless)).await;
    let frame = frame_of(&bench, &section);
    assert!(
        frame.contains("this Qdrant answers without an API key"),
        "{frame}"
    );
    assert!(!frame.contains("knowledge search unavailable"), "{frame}");
    let buffer = drawn(&bench, &section);
    let y = line_of(&buffer, "this Qdrant answers without an API key");
    assert_eq!(
        buffer[(0, y)].fg,
        Theme::default().dim.fg.unwrap_or(Color::Reset),
        "the info style"
    );
    insta::assert_snapshot!("probe_keyless_info", frame);
}

/// D9: a ready Qdrant, or nothing probed, draws no row at all: the frame is the one without a
/// probe.
#[tokio::test]
async fn ready_draws_no_row() {
    let (bench, plain) = bench_with(stored()).await;
    let before = frame_of(&bench, &plain);
    for check in [Some(QdrantCheck::Ready), None] {
        let (bench, section) = probed(stored(), check.clone()).await;
        assert_eq!(frame_of(&bench, &section), before, "{check:?}");
    }
}

/// D9: a probe that has not answered yet for a snapshot that moved draws nothing: the old answer
/// was about the settings before.
#[tokio::test]
async fn a_moved_snapshot_drops_the_old_answer() {
    let (bench, mut section) = probed(stored(), Some(unreachable())).await;
    bench.reply(
        &mut section,
        &StoreReply::Qdrant(QdrantSnapshot {
            key_state: QdrantState::NotStored,
            ..stored()
        }),
    );
    assert_eq!(probes_of(&bench), 1, "probed again");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(!frame.contains("knowledge search"), "{frame}");
}

/// D9: a compare-and-set miss installs a fresh snapshot too, and one whose URL another box moved
/// drops the old URL's answer and asks for one probe: the old warning never stands under the
/// new URL's row.
#[tokio::test]
async fn a_stale_snapshot_whose_url_moved_probes_again() {
    let (bench, mut section) = probed(stored(), Some(unreachable())).await;
    bench.key(&mut section, "e");
    typed(&bench, &mut section, "https://mine.example:6334");
    bench.key(&mut section, "Enter");
    let _ = sent_by(&bench);
    bench.reply(
        &mut section,
        &StoreReply::QdrantStale(Box::new(snapshot(
            UrlRowState::Set {
                url: url(OTHER),
                token: later(),
            },
            QdrantState::Stored,
        ))),
    );
    assert!(
        matches!(sent_by(&bench).as_slice(), [StoreRequest::ProbeQdrant]),
        "one probe for the stale snapshot"
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(frame.contains(OTHER), "the row now: {frame}");
    assert!(!frame.contains("knowledge search"), "{frame}");
}

/// D9: a key write answers with a fresh snapshot read, and that snapshot probes again even
/// when it reads the same as before (one stored key replaced by another): the refused key's
/// warning never stays on screen beside the new key's "stored".
#[tokio::test]
async fn a_key_written_over_a_refused_one_probes_again() {
    let (bench, mut section) = probed(stored(), Some(key_refused())).await;
    bench.key(&mut section, "j");
    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    typed(&bench, &mut section, "a-new-key");
    bench.key(&mut section, "Enter");
    assert!(
        matches!(
            requests_of(&bench).as_slice(),
            [StoreRequest::SetQdrantApiKey(_)]
        ),
        "the key is stored"
    );
    bench.reply(&mut section, &StoreReply::Qdrant(stored()));
    assert_eq!(probes_of(&bench), 1, "the new key's snapshot probes again");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(!frame.contains("knowledge search"), "{frame}");
    assert!(!frame.contains("refused this box's API key"), "{frame}");
}

/// D9: a refused key write may have landed before its follow-up read failed, so the re-read
/// it sends probes again even when it reads the same as before.
#[tokio::test]
async fn a_refused_key_write_re_read_probes_again() {
    let (bench, mut section) = probed(stored(), Some(key_refused())).await;
    bench.key(&mut section, "j");
    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    typed(&bench, &mut section, "a-new-key");
    bench.key(&mut section, "Enter");
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "set_qdrant_api_key",
            message: "the keyring is locked".to_owned(),
        },
    );
    assert!(
        matches!(requests_of(&bench).as_slice(), [StoreRequest::QdrantInfo]),
        "a failed write reads the section again"
    );
    bench.reply(&mut section, &StoreReply::Qdrant(stored()));
    assert_eq!(probes_of(&bench), 1, "the re-read probes again");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(!frame.contains("refused this box's API key"), "{frame}");
}

/// No probe row ever draws a key: a key typed and stored just before the probe answers is
/// nowhere in the frame or the section's `Debug`, whatever the probe said.
#[tokio::test]
async fn no_probe_line_renders_a_key() {
    for check in [
        no_url(),
        unreachable(),
        mismatch(),
        key_refused(),
        no_key(),
        QdrantCheck::Keyless,
        QdrantCheck::Ready,
    ] {
        let (bench, mut section) = bench_with(snapshot(
            UrlRowState::Set {
                url: url(SUMMARY),
                token: token(),
            },
            QdrantState::NotStored,
        ))
        .await;
        bench.key(&mut section, "j");
        assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
        typed(&bench, &mut section, KEY_SENTINEL);
        bench.key(&mut section, "Enter");
        assert!(
            matches!(
                requests_of(&bench).as_slice(),
                [StoreRequest::SetQdrantApiKey(_)]
            ),
            "the key is stored"
        );
        bench.reply(&mut section, &StoreReply::Qdrant(stored()));
        assert_eq!(probes_of(&bench), 1, "the stored key's snapshot probes");
        bench.reply(&mut section, &StoreReply::QdrantProbe(Some(check.clone())));
        let frame = frame_of(&bench, &section);
        assert!(!frame.contains(KEY_SENTINEL), "{check:?}: {frame}");
        assert!(!format!("{section:?}").contains(KEY_SENTINEL), "{check:?}");
        assert!(!format!("{check:?}").contains(KEY_SENTINEL), "{check:?}");
    }
}

/// D9: an outage forgets what the last probe was sent for, so the store coming back with the
/// same URL, override and key probes again: the pre-outage answer ("unreachable") is never drawn
/// under a Qdrant that may answer now, and nothing is drawn until the new probe answers.
#[tokio::test]
async fn the_same_snapshot_after_an_outage_probes_again() {
    for outage in [
        snapshot(UrlRowState::Offline, QdrantState::Stored),
        QdrantSnapshot::not_applicable(),
    ] {
        let (bench, mut section) = probed(stored(), Some(unreachable())).await;
        bench.reply(&mut section, &StoreReply::Qdrant(outage.clone()));
        assert_eq!(probes_of(&bench), 0, "{outage:?}: no probe while offline");

        bench.reply(&mut section, &StoreReply::Qdrant(stored()));
        assert_eq!(probes_of(&bench), 1, "{outage:?}: back online, probed again");
        let frame = flattened(&frame_of(&bench, &section));
        assert!(!frame.contains("knowledge search"), "{outage:?}: {frame}");
    }
}
