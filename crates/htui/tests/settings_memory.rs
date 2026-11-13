//! Settings › Hierarchy › Memory (MOD-163 M3, D20-D22): a project's memory pane, opened from its
//! row, driven through a [`SectionBench`] with replies built from fixed-timestamp entries (a
//! `MemStore` stamps real time), and once end to end through a [`Harness`] over the worker.
#![cfg(feature = "testkit")]

use chrono::{DateTime, TimeZone as _, Utc};
use htui::app::Action;
use htui::keys::{load_str, views};
use htui::memory::{MemoryWrite, READ_NAME};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::tabs::settings::hierarchy::PICK_A_PROJECT;
use htui::ui::tabs::settings::memory::{
    CLASH_QUESTION, CLASH_QUESTION_AMEND, CLOSED_ELSEWHERE, MEMORY_NEEDS_THE_DATABASE, UNAVAILABLE,
    UNCHANGED,
};
use htui::ui::tabs::settings::{AgentsSection, HierarchySection, SettingsSection, SettingsTab};
use htui_core::fixtures::ids;
use htui_core::model::{MemoryEntry, MemoryEntryId, MemoryKind, MemoryState, ProjectId};
use htui_core::store::{MemStore, ReadStore as _};
use htui_store::Backend;
use uuid::Uuid;

/// A fixed instant in November 2026.
fn at(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, day, 9, 30, 0)
        .single()
        .expect("a valid instant")
}

/// One entry of the demo `vulkan-tutorials` project with a fixed id and fixed timestamps.
fn entry(
    n: u128,
    kind: MemoryKind,
    subject: &str,
    body: &str,
    state: MemoryState,
    version: i32,
    day: u32,
) -> MemoryEntry {
    MemoryEntry {
        id: MemoryEntryId::from_uuid(Uuid::from_u128(n)),
        project_id: ids::PROJECT_VULKAN,
        kind,
        subject: subject.to_owned(),
        body: body.to_owned(),
        state,
        version,
        confirmed_at: at(day),
        superseded_by: None,
        created_by: ids::USER,
        created_at: at(1),
        updated_at: at(day),
        source: htui_core::model::MemorySource::Person,
        provenance: None,
        evidence: None,
        usage: htui_core::model::MemoryUsage::default(),
    }
}

/// The live list, newest first: an active gotcha at version 2, an active decision, a suspect fact.
fn live() -> Vec<MemoryEntry> {
    vec![
        entry(
            1,
            MemoryKind::Gotcha,
            "Build cache",
            "Clear target/ when the sqlx offline data goes stale.",
            MemoryState::Active,
            2,
            7,
        ),
        entry(
            2,
            MemoryKind::Decision,
            "Error crates",
            "Library crates use thiserror; binaries use anyhow.",
            MemoryState::Active,
            1,
            5,
        ),
        entry(
            3,
            MemoryKind::Fact,
            "Shader dir",
            "Shaders live under assets/shaders.",
            MemoryState::Suspect,
            1,
            3,
        ),
    ]
}

/// Every state: the live list plus a retired and a superseded entry.
fn every() -> Vec<MemoryEntry> {
    let mut all = live();
    all.push(entry(
        4,
        MemoryKind::Lesson,
        "Flaky test",
        "The swapchain test needs a display.",
        MemoryState::Retired,
        2,
        2,
    ));
    all.push(entry(
        5,
        MemoryKind::Fact,
        "Old shader dir",
        "Shaders lived under res/.",
        MemoryState::Superseded,
        2,
        1,
    ));
    all
}

/// A bench whose Hierarchy section holds the demo tree, the cursor on `vulkan-tutorials`, its
/// memory pane open and answered with `entries`.
async fn opened(entries: Option<Vec<MemoryEntry>>) -> (SectionBench, HierarchySection) {
    answered(&StoreReply::Memory {
        project: ids::PROJECT_VULKAN,
        entries,
    })
    .await
}

/// The demo tree's reply to `Hierarchy(WORKSPACE_GRAPHICS)`.
async fn demo_tree() -> StoreReply {
    serve(
        &Backend::memory(MemStore::demo()),
        &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS),
    )
    .await
}

/// As [`opened`], the pane's read answered with `first`.
async fn answered(first: &StoreReply) -> (SectionBench, HierarchySection) {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new().with_home(Some("/home/x"));
    bench.reply(&mut section, &demo_tree().await);
    bench.key(&mut section, "j");
    bench.key(&mut section, "m");
    let _ = bench.drained();
    bench.reply(&mut section, first);
    (bench, section)
}

/// Types `text` one key at a time.
fn type_text(bench: &SectionBench, section: &mut HierarchySection, text: &str) {
    for c in text.chars() {
        let chord = if c == ' ' {
            "space".to_owned()
        } else {
            c.to_string()
        };
        bench.key(section, &chord);
    }
}

/// Feeds each chord in turn.
fn keys(bench: &SectionBench, section: &mut HierarchySection, chords: &[&str]) {
    for chord in chords {
        bench.key(section, chord);
    }
}

/// The store requests the section emitted since the last drain.
fn requests(bench: &SectionBench) -> Vec<StoreRequest> {
    bench
        .drained()
        .into_iter()
        .filter_map(|action| match action {
            Action::Store(request) => Some(request),
            _ => None,
        })
        .collect()
}

/// The section at 100x30 with its hint row under it, as text.
fn frame(bench: &SectionBench, section: &HierarchySection) -> String {
    format!(
        "{}\n{}",
        bench.render_section(section, 100),
        bench.key_hint(section)
    )
}

/// Whether the frame holds `sentence`, wrapped or not.
fn shows(frame: &str, sentence: &str) -> bool {
    let flat = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat(frame).contains(&flat(sentence))
}

/// `n`, a subject, `Enter`, a body, `Ctrl+S`: a new entry's write.
fn create(bench: &SectionBench, section: &mut HierarchySection, subject: &str, body: &str) {
    bench.key(section, "n");
    type_text(bench, section, subject);
    bench.key(section, "enter");
    type_text(bench, section, body);
    bench.key(section, "ctrl-s");
}

#[tokio::test]
async fn m_on_a_project_row_opens_the_pane_and_reads_its_live_entries() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new().with_home(Some("/home/x"));
    let tree = serve(
        &Backend::memory(MemStore::demo()),
        &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS),
    )
    .await;
    bench.reply(&mut section, &tree);
    bench.key(&mut section, "j");
    let _ = bench.drained();

    bench.key(&mut section, "m");

    let sent = requests(&bench);
    assert!(
        matches!(
            sent.as_slice(),
            [StoreRequest::MemoryEntries {
                project,
                all: false
            }] if *project == ids::PROJECT_VULKAN
        ),
        "{sent:?}"
    );
    assert_eq!(section.key_stack(), Some(views::MEMORY_BROWSE));
    assert!(section.captures_input(), "sections do not cycle over it");
    // Re-activation reads the pane's list again beside the tree.
    let wanted = section.wants_requests(&htui_core::model::Scope {
        workspace_id: ids::WORKSPACE_GRAPHICS,
        project_ids: vec![ids::PROJECT_VULKAN],
    });
    assert!(
        wanted
            .iter()
            .any(|request| matches!(request, StoreRequest::MemoryEntries { .. })),
        "{wanted:?}"
    );
}

/// `m` on the row `down` steps below the workspace row of `tree`, the cursor row naming `name`: no
/// request, the tree keeps the keys, and the notice says to pick a project.
async fn m_refuses_off_a_project_row(tree: &StoreReply, down: usize, name: &str) {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new().with_home(Some("/home/x"));
    bench.reply(&mut section, tree);
    for _ in 0..down {
        bench.key(&mut section, "j");
    }
    let _ = bench.drained();
    let before = frame(&bench, &section);
    assert!(
        before
            .lines()
            .any(|line| line.contains('\u{258c}') && line.contains(name)),
        "the cursor is on {name}: {before}"
    );

    bench.key(&mut section, "m");

    let sent = requests(&bench);
    assert!(sent.is_empty(), "{name}: {sent:?}");
    assert_eq!(section.key_stack(), Some(views::HIERARCHY_BROWSE), "{name}");
    assert!(
        shows(&frame(&bench, &section), PICK_A_PROJECT),
        "{name}: {}",
        frame(&bench, &section)
    );
}

#[tokio::test]
async fn m_on_the_workspace_or_a_repo_row_says_pick_a_project() {
    let backend = Backend::memory(MemStore::demo());
    let tree = serve(&backend, &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS)).await;
    m_refuses_off_a_project_row(&tree, 0, "Graphics (graphics)").await;

    // The demo seeds no repo: one under `vulkan-tutorials` puts a repo row below the project's.
    let tree = serve(
        &backend,
        &StoreRequest::CreateRepo {
            project: ids::PROJECT_VULKAN,
            name: "core".to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary: false,
        },
    )
    .await;
    m_refuses_off_a_project_row(&tree, 2, "core").await;
}

#[tokio::test]
async fn the_list_shows_kind_subject_state_and_date() {
    let (bench, section) = opened(Some(live())).await;
    insta::assert_snapshot!("list", frame(&bench, &section));
}

#[tokio::test]
async fn a_toggles_every_state_and_rereads() {
    let (bench, mut section) = opened(Some(live())).await;

    bench.key(&mut section, "a");

    let sent = requests(&bench);
    assert!(
        matches!(
            sent.as_slice(),
            [StoreRequest::MemoryEntries {
                project,
                all: true
            }] if *project == ids::PROJECT_VULKAN
        ),
        "{sent:?}"
    );
    bench.reply(
        &mut section,
        &StoreReply::Memory {
            project: ids::PROJECT_VULKAN,
            entries: Some(every()),
        },
    );
    insta::assert_snapshot!("all", frame(&bench, &section));
}

#[tokio::test]
async fn n_form_enter_body_ctrl_s_creates() {
    let (bench, mut section) = opened(Some(live())).await;

    bench.key(&mut section, "n");
    assert_eq!(section.key_stack(), Some(views::MEMORY_EDIT));
    type_text(&bench, &mut section, "Vulkan SDK");
    bench.key(&mut section, "enter");
    assert_eq!(section.key_stack(), Some(views::MEMORY_BODY));
    type_text(&bench, &mut section, "Install 1.4 or later.");
    bench.key(&mut section, "ctrl-s");

    let sent = requests(&bench);
    match sent.as_slice() {
        [
            StoreRequest::CreateMemoryEntry {
                project,
                all,
                kind,
                subject,
                body,
            },
        ] => {
            assert_eq!(*project, ids::PROJECT_VULKAN);
            assert!(!all);
            assert_eq!(*kind, MemoryKind::Fact);
            assert_eq!(subject.as_str(), "Vulkan SDK");
            assert_eq!(body.as_str(), "Install 1.4 or later.");
        }
        other => panic!("one create, not {other:?}"),
    }
}

#[tokio::test]
async fn the_kind_choice_cycles_the_four_kinds() {
    let (bench, mut section) = opened(Some(live())).await;

    bench.key(&mut section, "n");
    bench.key(&mut section, "tab");
    for (chord, shown) in [
        ("right", "decision"),
        ("l", "gotcha"),
        ("right", "lesson"),
        ("right", "fact"),
        ("left", "lesson"),
    ] {
        bench.key(&mut section, chord);
        assert!(
            frame(&bench, &section).contains(shown),
            "{chord} shows {shown}"
        );
    }
    bench.key(&mut section, "tab");
    type_text(&bench, &mut section, "Lesson one");
    bench.key(&mut section, "enter");
    type_text(&bench, &mut section, "Learned.");
    bench.key(&mut section, "ctrl-s");

    let sent = requests(&bench);
    assert!(
        matches!(
            sent.as_slice(),
            [StoreRequest::CreateMemoryEntry {
                kind: MemoryKind::Lesson,
                ..
            }]
        ),
        "{sent:?}"
    );
}

#[tokio::test]
async fn a_scrub_refusal_stays_in_the_form_and_sends_nothing() {
    let (bench, mut section) = opened(Some(live())).await;

    bench.key(&mut section, "n");
    type_text(&bench, &mut section, "key sk-ant-api03-do-not-print-me");
    bench.key(&mut section, "enter");

    assert!(requests(&bench).is_empty());
    assert_eq!(section.key_stack(), Some(views::MEMORY_EDIT));
    let shown = frame(&bench, &section);
    assert!(shows(&shown, "anthropic_api_key"), "{shown}");

    // A body refusal stays in the body editor, nothing sent.
    let (bench, mut section) = opened(Some(live())).await;
    create(
        &bench,
        &mut section,
        "Keys",
        "key sk-ant-api03-do-not-print-me",
    );
    assert!(requests(&bench).is_empty());
    assert_eq!(section.key_stack(), Some(views::MEMORY_BODY));
    assert!(shows(&frame(&bench, &section), "anthropic_api_key"));
}

#[tokio::test]
async fn e_then_ctrl_s_amends_with_the_token() {
    let (bench, mut section) = opened(Some(live())).await;

    keys(&bench, &mut section, &["e", "enter"]);
    type_text(&bench, &mut section, " Twice.");
    bench.key(&mut section, "ctrl-s");

    let sent = requests(&bench);
    match sent.as_slice() {
        [
            StoreRequest::AmendMemoryEntry {
                project,
                id,
                expected,
                kind,
                subject,
                body,
                ..
            },
        ] => {
            assert_eq!(*project, ids::PROJECT_VULKAN);
            assert_eq!(*id, live()[0].id);
            assert_eq!(*expected, 2);
            assert_eq!(*kind, MemoryKind::Gotcha);
            assert_eq!(subject.as_str(), "Build cache");
            assert_eq!(
                body.as_str(),
                "Clear target/ when the sqlx offline data goes stale. Twice."
            );
        }
        other => panic!("one amend, not {other:?}"),
    }
}

#[tokio::test]
async fn an_unchanged_amend_writes_nothing() {
    let (bench, mut section) = opened(Some(live())).await;

    keys(&bench, &mut section, &["e", "enter", "ctrl-s"]);

    assert!(requests(&bench).is_empty());
    assert_eq!(section.key_stack(), Some(views::MEMORY_BROWSE));
    assert!(shows(&frame(&bench, &section), UNCHANGED));
}

#[tokio::test]
async fn a_stale_reply_keeps_the_text_and_moves_the_token() {
    let (bench, mut section) = opened(Some(live())).await;
    keys(&bench, &mut section, &["e", "enter"]);
    type_text(&bench, &mut section, " Mine.");
    bench.key(&mut section, "ctrl-s");
    let _ = bench.drained();

    let mut head = live()[0].clone();
    head.version = 3;
    head.body = "Theirs.".to_owned();
    let mut fresh = live();
    fresh[0] = head.clone();
    bench.reply(
        &mut section,
        &StoreReply::MemoryStale {
            project: ids::PROJECT_VULKAN,
            head,
            entries: fresh,
        },
    );

    let shown = frame(&bench, &section);
    assert_eq!(section.key_stack(), Some(views::MEMORY_BODY));
    assert!(
        shows(&shown, "goes stale. Mine."),
        "the text is kept: {shown}"
    );
    assert!(shows(&shown, "changed elsewhere"), "{shown}");
    bench.key(&mut section, "ctrl-s");
    let sent = requests(&bench);
    assert!(
        matches!(
            sent.as_slice(),
            [StoreRequest::AmendMemoryEntry { expected: 3, .. }]
        ),
        "{sent:?}"
    );
}

#[tokio::test]
async fn d_asks_then_retires() {
    let (bench, mut section) = opened(Some(live())).await;
    bench.key(&mut section, "j");

    bench.key(&mut section, "d");

    assert_eq!(bench.key_hint(&section), "y retire \u{b7} n/Esc stop");
    assert!(shows(&frame(&bench, &section), "retire `Error crates`?"));
    assert!(requests(&bench).is_empty(), "nothing before the answer");
    bench.key(&mut section, "y");
    let sent = requests(&bench);
    assert!(
        matches!(
            sent.as_slice(),
            [StoreRequest::RetireMemoryEntry { id, expected: 1, .. }] if *id == live()[1].id
        ),
        "{sent:?}"
    );

    let fresh: Vec<MemoryEntry> = live()
        .into_iter()
        .filter(|row| row.id != live()[1].id)
        .collect();
    bench.reply(
        &mut section,
        &StoreReply::MemoryWritten {
            project: ids::PROJECT_VULKAN,
            written: MemoryWrite::Retired { id: live()[1].id },
            entries: Ok(fresh),
        },
    );
    assert_eq!(section.key_stack(), Some(views::MEMORY_BROWSE));
    assert!(shows(&frame(&bench, &section), "retired `Error crates`"));
}

/// A new entry on a taken subject, answered with the holder.
async fn clashed() -> (SectionBench, HierarchySection) {
    let (bench, mut section) = opened(Some(live())).await;
    create(&bench, &mut section, "build cache", "Mine.");
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &StoreReply::MemorySubjectTaken {
            project: ids::PROJECT_VULKAN,
            existing: live()[0].clone(),
        },
    );
    (bench, section)
}

#[tokio::test]
async fn a_clash_asks_and_e_opens_the_existing_entry() {
    let (bench, mut section) = clashed().await;

    let shown = frame(&bench, &section);
    assert!(shows(&shown, CLASH_QUESTION), "{shown}");
    assert!(shows(&shown, "Build cache"), "{shown}");
    assert_eq!(
        bench.key_hint(&section),
        "e edit it \u{b7} r replace it \u{b7} Esc back"
    );
    assert_eq!(section.key_stack(), Some(views::MEMORY_CLASH));

    bench.key(&mut section, "e");

    assert_eq!(section.key_stack(), Some(views::MEMORY_EDIT));
    assert!(shows(&frame(&bench, &section), "edit `Build cache`"));
    keys(&bench, &mut section, &["enter", "ctrl-s"]);
    assert!(requests(&bench).is_empty(), "the holder, unchanged");
}

#[tokio::test]
async fn a_clash_and_r_supersedes_the_existing_entry() {
    let (bench, mut section) = clashed().await;

    bench.key(&mut section, "r");

    let sent = requests(&bench);
    match sent.as_slice() {
        [
            StoreRequest::SupersedeMemoryEntry {
                old,
                expected,
                kind,
                subject,
                body,
                ..
            },
        ] => {
            assert_eq!(*old, live()[0].id);
            assert_eq!(*expected, 2);
            assert_eq!(*kind, MemoryKind::Fact);
            assert_eq!(subject.as_str(), "build cache");
            assert_eq!(body.as_str(), "Mine.");
        }
        other => panic!("one supersede, not {other:?}"),
    }
}

/// `Esc` over the question goes back to the body, the draft kept.
#[tokio::test]
async fn esc_over_a_clash_goes_back_to_the_body() {
    let (bench, mut section) = clashed().await;

    bench.key(&mut section, "esc");

    assert_eq!(section.key_stack(), Some(views::MEMORY_BODY));
    assert!(shows(&frame(&bench, &section), "Mine."));
}

/// MOD-163 O-1: an amend's clash offers only "edit it": no replace hint, and `r` sends nothing.
#[tokio::test]
async fn an_amend_clash_offers_only_edit() {
    let (bench, mut section) = opened(Some(live())).await;
    keys(&bench, &mut section, &["j", "e"]);
    for _ in 0.."Error crates".len() {
        bench.key(&mut section, "backspace");
    }
    type_text(&bench, &mut section, "Build cache");
    keys(&bench, &mut section, &["enter", "ctrl-s"]);
    let sent = requests(&bench);
    assert!(
        matches!(sent.as_slice(), [StoreRequest::AmendMemoryEntry { .. }]),
        "{sent:?}"
    );
    bench.reply(
        &mut section,
        &StoreReply::MemorySubjectTaken {
            project: ids::PROJECT_VULKAN,
            existing: live()[0].clone(),
        },
    );

    let shown = frame(&bench, &section);
    assert!(shows(&shown, CLASH_QUESTION_AMEND), "{shown}");
    assert!(!shows(&shown, CLASH_QUESTION), "{shown}");
    assert!(!bench.key_hint(&section).contains("replace"));
    assert_eq!(bench.key_hint(&section), "e edit it \u{b7} Esc back");
    assert_eq!(section.key_stack(), Some(views::MEMORY_CLASH_AMEND));

    bench.key(&mut section, "r");
    assert!(requests(&bench).is_empty(), "no supersede for an amend");
    assert_eq!(section.key_stack(), Some(views::MEMORY_CLASH_AMEND));

    bench.key(&mut section, "e");
    assert_eq!(section.key_stack(), Some(views::MEMORY_EDIT));
    assert!(shows(&frame(&bench, &section), "edit `Build cache`"));
}

#[tokio::test]
async fn none_shows_memory_needs_the_database_and_refuses_writes() {
    let (bench, mut section) = opened(None).await;

    assert!(shows(&frame(&bench, &section), MEMORY_NEEDS_THE_DATABASE));
    for chord in ["n", "e", "b", "d"] {
        bench.key(&mut section, chord);
        assert!(requests(&bench).is_empty(), "{chord} sends nothing");
        assert_eq!(section.key_stack(), Some(views::MEMORY_BROWSE), "{chord}");
    }
    assert!(shows(&frame(&bench, &section), MEMORY_NEEDS_THE_DATABASE));
}

#[tokio::test]
async fn esc_closes_the_pane_back_to_the_tree() {
    let (bench, mut section) = opened(Some(live())).await;

    bench.key(&mut section, "esc");

    assert_eq!(section.key_stack(), Some(views::HIERARCHY_BROWSE));
    assert!(!section.captures_input());
    let shown = frame(&bench, &section);
    assert!(shows(&shown, "vulkan-tutorials"), "{shown}");
    assert!(!shows(&shown, "Build cache"), "{shown}");
}

#[tokio::test]
async fn the_pane_hints_read_from_the_keys_in_force() {
    let keys_in_force =
        load_str("version = 2\n[settings.memory]\nall = [\"x\"]\n").expect("the file loads");
    let bench = SectionBench::new().await.with_keys(keys_in_force);
    let mut section = HierarchySection::new().with_home(Some("/home/x"));
    let tree = serve(
        &Backend::memory(MemStore::demo()),
        &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS),
    )
    .await;
    bench.reply(&mut section, &tree);
    keys(&bench, &mut section, &["j", "m"]);
    bench.reply(
        &mut section,
        &StoreReply::Memory {
            project: ids::PROJECT_VULKAN,
            entries: Some(live()),
        },
    );
    let _ = bench.drained();

    let hint = bench.key_hint(&section);
    assert!(hint.contains("x all"), "{hint}");
    assert!(!hint.contains("a all"), "{hint}");
    bench.key(&mut section, "x");
    let sent = requests(&bench);
    assert!(
        matches!(
            sent.as_slice(),
            [StoreRequest::MemoryEntries { all: true, .. }]
        ),
        "{sent:?}"
    );
}

#[tokio::test]
async fn a_reply_for_another_project_is_ignored() {
    let (bench, mut section) = opened(Some(live())).await;
    let before = frame(&bench, &section);

    let other = ProjectId::default();
    bench.reply(
        &mut section,
        &StoreReply::Memory {
            project: other,
            entries: Some(Vec::new()),
        },
    );
    bench.reply(
        &mut section,
        &StoreReply::MemorySubjectTaken {
            project: other,
            existing: live()[1].clone(),
        },
    );

    assert_eq!(frame(&bench, &section), before);
}

#[tokio::test]
async fn an_end_to_end_create_lands_through_the_worker() {
    let store = MemStore::demo();
    let mut harness =
        Harness::over(store.clone()).with_tab(Box::new(SettingsTab::with_sections(vec![
            Box::new(AgentsSection::new()),
            Box::new(HierarchySection::new()),
        ])));
    harness.settle().await;
    harness.key("l");
    harness.settle().await;
    harness.key("j");
    harness.key("m");
    harness.settle().await;
    harness.key("n");
    for c in "Shaders".chars() {
        harness.key(&c.to_string());
    }
    harness.key("enter");
    for c in "Under assets.".chars() {
        harness.key(&if c == ' ' {
            "space".to_owned()
        } else {
            c.to_string()
        });
    }
    harness.key("ctrl-s");
    harness.settle().await;

    let rows = store
        .memory_entries(ids::PROJECT_VULKAN, None)
        .await
        .expect("the memory store never fails")
        .expect("the memory store holds memory");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].subject, "Shaders");
    assert_eq!(rows[0].body, "Under assets.");
    assert_eq!(rows[0].kind, MemoryKind::Fact);
    assert_eq!(harness.app().status, None, "nothing failed");
    assert!(harness.render().contains("saved `Shaders`"));
}

/// T4 verify finding (repair by the main thread): a re-read that answers offline while the retire
/// question or a new entry's clash question is open refuses `y` and `r` in place, as every other
/// write key does (blueprint §T4.4: "Writes are refused in place, nothing sent").
#[tokio::test]
async fn retire_and_replace_are_refused_in_place_once_the_reread_answers_offline() {
    let (bench, mut section) = opened(Some(live())).await;
    keys(&bench, &mut section, &["j", "d"]);
    bench.reply(
        &mut section,
        &StoreReply::Memory {
            project: ids::PROJECT_VULKAN,
            entries: None,
        },
    );
    bench.key(&mut section, "y");
    assert!(requests(&bench).is_empty(), "y sends nothing offline");
    assert!(shows(&frame(&bench, &section), MEMORY_NEEDS_THE_DATABASE));

    let (bench, mut section) = clashed().await;
    bench.reply(
        &mut section,
        &StoreReply::Memory {
            project: ids::PROJECT_VULKAN,
            entries: None,
        },
    );
    bench.key(&mut section, "r");
    assert!(requests(&bench).is_empty(), "r sends nothing offline");
    assert!(shows(&frame(&bench, &section), MEMORY_NEEDS_THE_DATABASE));
}

/// The list row drawn under the cursor (`▌`).
fn cursor_row(frame: &str) -> &str {
    frame
        .lines()
        .find(|line| line.starts_with('\u{258c}'))
        .unwrap_or_default()
}

/// How many times the frame holds `sentence`, wrapped or not.
fn count(frame: &str, sentence: &str) -> usize {
    let flat = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat(frame).matches(&flat(sentence)).count()
}

/// Review R4-1: a stale holder whose subject no longer folds to the draft's has freed it. The
/// question ends, the draft goes back to its editor, and `Ctrl+S` there creates.
#[tokio::test]
async fn a_clash_whose_holder_was_renamed_frees_the_subject() {
    let (bench, mut section) = clashed().await;
    bench.key(&mut section, "r");
    let _ = bench.drained();

    let mut head = live()[0].clone();
    head.subject = "Old build cache".to_owned();
    head.version = 3;
    let mut fresh = live();
    fresh[0] = head.clone();
    bench.reply(
        &mut section,
        &StoreReply::MemoryStale {
            project: ids::PROJECT_VULKAN,
            head,
            entries: fresh,
        },
    );

    let shown = frame(&bench, &section);
    assert_eq!(section.key_stack(), Some(views::MEMORY_BODY), "{shown}");
    assert!(shows(&shown, "Mine."), "the draft is kept: {shown}");
    assert!(shows(&shown, "the subject is free now"), "{shown}");
    assert!(shows(&shown, "Ctrl+s saves"), "{shown}");
    bench.key(&mut section, "r");
    assert!(
        requests(&bench).is_empty(),
        "r types; it supersedes nothing"
    );
    bench.key(&mut section, "ctrl-s");
    let sent = requests(&bench);
    assert!(
        matches!(
            sent.as_slice(),
            [StoreRequest::CreateMemoryEntry { subject, .. }] if subject.as_str() == "build cache"
        ),
        "{sent:?}"
    );
}

/// A clash answered again while its question is open names the new holder.
#[tokio::test]
async fn a_second_clash_names_the_new_holder() {
    let (bench, mut section) = clashed().await;
    bench.key(&mut section, "r");
    let _ = bench.drained();
    let mut holder = live()[0].clone();
    holder.id = MemoryEntryId::from_uuid(Uuid::from_u128(9));
    holder.version = 5;
    bench.reply(
        &mut section,
        &StoreReply::MemorySubjectTaken {
            project: ids::PROJECT_VULKAN,
            existing: holder.clone(),
        },
    );

    assert_eq!(section.key_stack(), Some(views::MEMORY_CLASH));
    bench.key(&mut section, "r");
    let sent = requests(&bench);
    assert!(
        matches!(
            sent.as_slice(),
            [StoreRequest::SupersedeMemoryEntry { old, expected: 5, .. }] if *old == holder.id
        ),
        "{sent:?}"
    );
}

/// Review R4-2 and R4-3: a stale retire whose entry moved to another row follows it there, and
/// the question still names it; `y` retires it under the head's version.
#[tokio::test]
async fn a_stale_retire_follows_its_entry_to_its_new_row() {
    let (bench, mut section) = opened(Some(live())).await;
    keys(&bench, &mut section, &["j", "j", "d", "y"]);
    let _ = bench.drained();

    let mut head = live()[2].clone();
    head.version = 2;
    head.updated_at = at(9);
    let fresh = vec![head.clone(), live()[0].clone(), live()[1].clone()];
    bench.reply(
        &mut section,
        &StoreReply::MemoryStale {
            project: ids::PROJECT_VULKAN,
            head: head.clone(),
            entries: fresh,
        },
    );

    let shown = frame(&bench, &section);
    assert!(cursor_row(&shown).contains("Shader dir"), "{shown}");
    assert!(shows(&shown, "retire `Shader dir`?"), "{shown}");
    assert!(shows(&shown, "changed elsewhere"), "{shown}");
    bench.key(&mut section, "y");
    let sent = requests(&bench);
    assert!(
        matches!(
            sent.as_slice(),
            [StoreRequest::RetireMemoryEntry { id, expected: 2, .. }] if *id == head.id
        ),
        "{sent:?}"
    );
}

/// Review R4-2: a re-read keeps the cursor on the entry it was on, wherever that entry now is.
#[tokio::test]
async fn an_all_toggle_reread_keeps_the_highlighted_entry() {
    let (bench, mut section) = opened(Some(live())).await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "a");
    let mut retired = every()[3].clone();
    retired.updated_at = at(9);
    let mut all = vec![retired];
    all.extend(live());
    bench.reply(
        &mut section,
        &StoreReply::Memory {
            project: ids::PROJECT_VULKAN,
            entries: Some(all),
        },
    );

    let shown = frame(&bench, &section);
    assert!(cursor_row(&shown).contains("Error crates"), "{shown}");
}

/// Review R4-3: the retire question is drawn from the question itself, so a failed write leaves
/// it asking, and it is drawn once.
#[tokio::test]
async fn a_failed_retire_still_asks() {
    let (bench, mut section) = opened(Some(live())).await;
    keys(&bench, &mut section, &["j", "d"]);
    assert_eq!(
        count(&frame(&bench, &section), "retire `Error crates`?"),
        1,
        "asked once"
    );
    bench.key(&mut section, "y");
    let _ = bench.drained();
    assert!(
        shows(&frame(&bench, &section), "retire `Error crates`?"),
        "in flight"
    );

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "retire_memory_entry",
            message: "connection closed".to_owned(),
        },
    );

    let shown = frame(&bench, &section);
    assert_eq!(section.key_stack(), Some(views::MEMORY_RETIRE));
    assert_eq!(count(&shown, "retire `Error crates`?"), 1, "{shown}");
    bench.key(&mut section, "y");
    assert!(
        matches!(
            requests(&bench).as_slice(),
            [StoreRequest::RetireMemoryEntry { .. }]
        ),
        "asked again, answered again"
    );
}

/// Review R4-4: a subject is quoted verbatim, only its control characters escaped.
#[tokio::test]
async fn a_subject_reads_verbatim_with_its_control_characters_escaped() {
    let quoted = entry(
        7,
        MemoryKind::Fact,
        "Don't \"x\"",
        "Quotes.",
        MemoryState::Active,
        1,
        6,
    );
    let broken = entry(
        8,
        MemoryKind::Fact,
        "two\nlines",
        "A break.",
        MemoryState::Active,
        1,
        5,
    );
    let (bench, mut section) = opened(Some(vec![quoted.clone(), broken])).await;

    bench.key(&mut section, "d");
    let shown = frame(&bench, &section);
    assert!(shows(&shown, "retire `Don't \"x\"`?"), "{shown}");
    keys(&bench, &mut section, &["esc", "j", "d"]);
    let shown = frame(&bench, &section);
    assert!(shows(&shown, "retire `two\\nlines`?"), "{shown}");

    keys(&bench, &mut section, &["esc", "k", "d", "y"]);
    bench.reply(
        &mut section,
        &StoreReply::MemoryWritten {
            project: ids::PROJECT_VULKAN,
            written: MemoryWrite::Retired { id: quoted.id },
            entries: Ok(Vec::new()),
        },
    );
    let shown = frame(&bench, &section);
    assert!(shows(&shown, "retired `Don't \"x\"`"), "{shown}");
}

/// Review R4-6 and R4-5: a refused first read draws the refusal, and a write key says the same,
/// not that memory needs the database.
#[tokio::test]
async fn a_refused_read_shows_unavailable_and_refuses_writes_as_such() {
    let (bench, mut section) = answered(&StoreReply::Failed {
        request: READ_NAME,
        message: "pool timed out".to_owned(),
    })
    .await;

    assert!(shows(
        &frame(&bench, &section),
        &format!("{UNAVAILABLE}: pool timed out")
    ));
    bench.key(&mut section, "n");
    let shown = frame(&bench, &section);
    assert!(requests(&bench).is_empty());
    assert_eq!(section.key_stack(), Some(views::MEMORY_BROWSE));
    assert_eq!(
        count(&shown, UNAVAILABLE),
        2,
        "the pane and the notice: {shown}"
    );
    assert!(!shows(&shown, MEMORY_NEEDS_THE_DATABASE), "{shown}");
}

/// Review R4-5: a tree that no longer holds the pane's project closes the pane.
#[tokio::test]
async fn the_pane_closes_when_its_project_leaves_the_tree() {
    let (bench, mut section) = opened(Some(live())).await;
    let StoreReply::Hierarchy(Some(mut tree)) = demo_tree().await else {
        panic!("the demo tree reads");
    };
    tree.projects
        .retain(|entry| entry.project.id != ids::PROJECT_VULKAN);

    bench.reply(&mut section, &StoreReply::Hierarchy(Some(tree)));

    assert_eq!(section.key_stack(), Some(views::HIERARCHY_BROWSE));
    let shown = frame(&bench, &section);
    assert!(
        shows(&shown, "the project is gone; its memory pane closed"),
        "{shown}"
    );
}

/// Review R4-5: a failed write frees the pane for the next one; the editor keeps its text.
#[tokio::test]
async fn a_failed_write_frees_the_pane() {
    let (bench, mut section) = opened(Some(live())).await;
    create(&bench, &mut section, "Vulkan SDK", "Install it.");
    assert_eq!(requests(&bench).len(), 1);
    bench.key(&mut section, "ctrl-s");
    assert!(requests(&bench).is_empty(), "one write at a time");

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "create_memory_entry",
            message: "connection closed".to_owned(),
        },
    );

    assert_eq!(section.key_stack(), Some(views::MEMORY_BODY));
    assert!(shows(&frame(&bench, &section), "Install it."));
    bench.key(&mut section, "ctrl-s");
    assert!(
        matches!(
            requests(&bench).as_slice(),
            [StoreRequest::CreateMemoryEntry { .. }]
        ),
        "free again"
    );
}

/// Review R4-5: a landed write whose re-read failed keeps the list it had and says both.
#[tokio::test]
async fn a_written_reply_whose_reread_failed_keeps_the_list() {
    let (bench, mut section) = opened(Some(live())).await;
    create(&bench, &mut section, "Vulkan SDK", "Install it.");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &StoreReply::MemoryWritten {
            project: ids::PROJECT_VULKAN,
            written: MemoryWrite::Created {
                id: MemoryEntryId::from_uuid(Uuid::from_u128(9)),
            },
            entries: Err("pool closed".to_owned()),
        },
    );

    let shown = frame(&bench, &section);
    assert_eq!(section.key_stack(), Some(views::MEMORY_BROWSE));
    assert!(shows(&shown, "Build cache"), "the list is kept: {shown}");
    assert!(
        shows(&shown, "saved \u{b7} the list was not re-read: pool closed"),
        "{shown}"
    );
}

/// Review R4-5: a stale answer whose entry closed elsewhere ends the edit.
#[tokio::test]
async fn a_stale_reply_whose_head_closed_returns_to_browse() {
    for state in [MemoryState::Retired, MemoryState::Superseded] {
        let (bench, mut section) = opened(Some(live())).await;
        keys(&bench, &mut section, &["e", "enter"]);
        type_text(&bench, &mut section, " Mine.");
        bench.key(&mut section, "ctrl-s");
        let _ = bench.drained();

        let mut head = live()[0].clone();
        head.state = state;
        head.version = 3;
        bench.reply(
            &mut section,
            &StoreReply::MemoryStale {
                project: ids::PROJECT_VULKAN,
                head,
                entries: live()[1..].to_vec(),
            },
        );

        let shown = frame(&bench, &section);
        assert_eq!(section.key_stack(), Some(views::MEMORY_BROWSE), "{state:?}");
        assert!(shows(&shown, CLOSED_ELSEWHERE), "{state:?}: {shown}");
    }
}

/// Review R4-5: every mode draws at 40 columns without a panic.
#[tokio::test]
async fn the_pane_renders_at_width_40() {
    let narrow = |bench: &SectionBench, section: &HierarchySection| {
        let shown = bench.render_section(section, 40);
        assert!(shown.contains("memory"), "{shown}");
    };
    let (bench, mut section) = opened(Some(live())).await;
    narrow(&bench, &section);
    bench.key(&mut section, "d");
    narrow(&bench, &section);
    keys(&bench, &mut section, &["esc", "n"]);
    narrow(&bench, &section);
    type_text(
        &bench,
        &mut section,
        "A subject long enough to wrap twice at forty",
    );
    bench.key(&mut section, "enter");
    narrow(&bench, &section);
    let (bench, section) = clashed().await;
    narrow(&bench, &section);
    let (bench, section) = opened(None).await;
    narrow(&bench, &section);
}
