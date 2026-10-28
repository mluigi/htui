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
use htui::qdrant_settings_info::{
    OldUrl, QdrantSnapshot, QdrantState, URL_WRITE_OFFLINE, UrlRowState,
};
use htui::store_worker::{StoreReply, StoreRequest};
use htui::testkit::{Harness, SectionBench};
use htui::ui::Theme;
use htui::ui::tabs::settings::{QdrantSection, SettingsSection, SettingsTab};
use htui_core::qdrant::{QdrantUrl, QdrantUrlError};
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

/// The store requests the section sent since the queue was last drained.
fn requests_of(bench: &SectionBench) -> Vec<StoreRequest> {
    bench
        .drained()
        .into_iter()
        .filter_map(|action| match action {
            Action::Store(request) => Some(request),
            _ => None,
        })
        .collect()
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
