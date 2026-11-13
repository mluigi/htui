//! Settings › Hierarchy › Documents (MOD-147 M3 D9, D10): a workspace's or a project's owner
//! documents pane, opened from its row, driven through a [`SectionBench`] with replies built from
//! fixed-timestamp documents (a `MemStore` stamps real time), and once end to end through the
//! worker's `serve`.
#![cfg(feature = "testkit")]

use chrono::{DateTime, TimeZone as _, Utc};
use htui::app::Action;
use htui::editor::{ExternalEdit, ExternalEditOutcome, NO_CHANGES};
use htui::keys::{KeyChord, views};
use htui::owner_documents::{DocumentOwner, READ_NAME, WRITE_NAME};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::SectionBench;
use htui::ui::tabs::registry::Tab as _;
use htui::ui::tabs::settings::hierarchy::PICK_AN_OWNER;
use htui::ui::tabs::settings::owner_documents::{
    EDITOR_OPEN, GOALS_NONE_YET, STILL_SAVING, UNAVAILABLE,
};
use htui::ui::tabs::settings::{AgentsSection, HierarchySection, SettingsSection, SettingsTab};
use htui_core::fixtures::ids;
use htui_core::model::document::GOALS_KIND;
use htui_core::model::{DocumentId, OwnerDocument};
use htui_core::store::{MemStore, StoreError};
use htui_store::Backend;
use uuid::Uuid;

/// The demo workspace's documents.
const WORKSPACE: DocumentOwner = DocumentOwner::Workspace(ids::WORKSPACE_GRAPHICS);

/// A fixed instant in November 2026.
fn at(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, day, 9, 30, 0)
        .single()
        .expect("a valid instant")
}

/// One document of the demo workspace's scope row with a fixed id and timestamp.
fn document(n: u128, kind: &str, version: i32, title: &str, body: &str, day: u32) -> OwnerDocument {
    OwnerDocument {
        id: DocumentId::from_uuid(Uuid::from_u128(n)),
        project_id: ids::PROJECT_SCOPE_GRAPHICS,
        kind: kind.to_owned(),
        version,
        title: title.to_owned(),
        body: body.to_owned(),
        produced_by_step_id: None,
        created_by: ids::USER,
        created_at: at(day),
    }
}

/// A goals document at version 1.
fn goals_v1() -> OwnerDocument {
    document(
        1,
        GOALS_KIND,
        1,
        "Goals",
        "# Goals\n\n- Ship the tutorials.\n- Keep the samples building.",
        3,
    )
}

/// Two versions of a brief, no goals: the pane lists `goals — none yet` first.
fn briefs() -> Vec<OwnerDocument> {
    vec![
        document(2, "brief", 1, "Scope", "First brief.", 2),
        document(3, "brief", 2, "Scope, revised", "Second brief.", 4),
    ]
}

/// The demo tree's reply to `Hierarchy(WORKSPACE_GRAPHICS)`.
async fn demo_tree(backend: &Backend) -> StoreReply {
    serve(backend, &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS)).await
}

/// A bench whose Hierarchy section holds the demo tree, the cursor on the workspace row, its
/// documents pane open and answered with `documents`.
async fn opened(documents: Vec<OwnerDocument>) -> (SectionBench, HierarchySection) {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new().with_home(Some("/home/x"));
    bench.reply(
        &mut section,
        &demo_tree(&Backend::memory(MemStore::demo())).await,
    );
    bench.key(&mut section, "o");
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &StoreReply::OwnerDocuments {
            owner: WORKSPACE,
            documents,
            writable: true,
        },
    );
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

/// The `$EDITOR` handoffs the section emitted since the last drain.
fn edits(bench: &SectionBench) -> Vec<ExternalEdit> {
    bench
        .drained()
        .into_iter()
        .filter_map(|action| match action {
            Action::EditExternally(edit) => Some(edit),
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

/// The `(kind, title, body)` of a `WriteOwnerDocument` for the workspace, else `None`.
fn written(request: &StoreRequest) -> Option<(&str, &str, &str)> {
    match request {
        StoreRequest::WriteOwnerDocument {
            owner,
            kind,
            title,
            body,
        } if *owner == WORKSPACE => Some((kind, title, body.as_str())),
        _ => None,
    }
}

#[tokio::test]
async fn o_on_the_workspace_row_opens_the_pane_with_goals_none_yet_first() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new().with_home(Some("/home/x"));
    bench.reply(
        &mut section,
        &demo_tree(&Backend::memory(MemStore::demo())).await,
    );
    let _ = bench.drained();

    bench.key(&mut section, "o");

    let sent = requests(&bench);
    assert!(
        matches!(sent.as_slice(), [StoreRequest::OwnerDocuments(owner)] if *owner == WORKSPACE),
        "{sent:?}"
    );
    assert_eq!(section.key_stack(), Some(views::DOCUMENTS_BROWSE));
    assert!(section.captures_input(), "sections do not cycle over it");
    let wanted = section.wants_requests(&htui_core::model::Scope {
        workspace_id: ids::WORKSPACE_GRAPHICS,
        project_ids: vec![ids::PROJECT_VULKAN],
    });
    assert!(
        wanted
            .iter()
            .any(|request| matches!(request, StoreRequest::OwnerDocuments(_))),
        "re-activation reads the pane again: {wanted:?}"
    );

    bench.reply(
        &mut section,
        &StoreReply::OwnerDocuments {
            owner: WORKSPACE,
            documents: briefs(),
            writable: true,
        },
    );
    let shown = frame(&bench, &section);
    let rows: Vec<&str> = shown
        .lines()
        .filter(|line| line.contains("goals") || line.contains("brief \u{b7}"))
        .collect();
    assert!(
        rows.len() == 3 && rows[0].contains(GOALS_NONE_YET),
        "goals first, then the briefs: {shown}"
    );
    assert!(
        rows[1].contains("brief \u{b7} v2 \u{b7} Scope, revised \u{b7} 2026-11-04"),
        "newest version first: {shown}"
    );
    insta::assert_snapshot!("list", shown);
}

#[tokio::test]
async fn a_project_row_opens_its_own_documents() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new().with_home(Some("/home/x"));
    bench.reply(
        &mut section,
        &demo_tree(&Backend::memory(MemStore::demo())).await,
    );
    bench.key(&mut section, "j");
    let _ = bench.drained();

    bench.key(&mut section, "o");

    let owner = DocumentOwner::Project(ids::PROJECT_VULKAN);
    let sent = requests(&bench);
    assert!(
        matches!(sent.as_slice(), [StoreRequest::OwnerDocuments(asked)] if *asked == owner),
        "{sent:?}"
    );
    bench.reply(
        &mut section,
        &StoreReply::OwnerDocuments {
            owner,
            documents: Vec::new(),
            writable: true,
        },
    );
    let shown = frame(&bench, &section);
    assert!(
        !shown.contains(GOALS_NONE_YET),
        "only a workspace offers goals: {shown}"
    );
    assert!(shows(&shown, "no documents yet"), "{shown}");
}

#[tokio::test]
async fn n_then_enter_hands_an_empty_body_and_an_edit_writes_then_rereads_the_list() {
    let backend = Backend::memory(MemStore::demo());
    let (bench, mut section) = opened(Vec::new()).await;

    // The cursor is on `goals — none yet`: `n` pre-fills the kind.
    bench.key(&mut section, "n");
    assert_eq!(section.key_stack(), Some(views::DOCUMENTS_FORM));
    type_text(&bench, &mut section, "Goals");
    bench.key(&mut section, "enter");

    let handed = edits(&bench);
    assert!(
        matches!(handed.as_slice(), [edit] if edit.text.is_empty() && edit.stem == "graphics-goals"),
        "{handed:?}"
    );
    assert_eq!(section.key_stack(), Some(views::DOCUMENTS_BROWSE));

    section.on_external_edit(
        ExternalEditOutcome::Edited("Ship the tutorials.\n".to_owned()),
        &mut bench.ctx(),
    );
    let sent = requests(&bench);
    assert!(
        matches!(sent.as_slice(), [write] if written(write) == Some((GOALS_KIND, "Goals", "Ship the tutorials."))),
        "the editor's added newline is dropped: {sent:?}"
    );

    let reply = serve(&backend, &sent[0]).await;
    assert!(
        matches!(&reply, StoreReply::OwnerDocumentWritten { version: 1, .. }),
        "{reply:?}"
    );
    bench.reply(&mut section, &reply);
    let reread = requests(&bench);
    assert!(
        matches!(reread.as_slice(), [StoreRequest::OwnerDocuments(owner)] if *owner == WORKSPACE),
        "the written reply rereads the list: {reread:?}"
    );
    bench.reply(&mut section, &serve(&backend, &reread[0]).await);

    let shown = frame(&bench, &section);
    assert!(shown.contains("goals \u{b7} v1 \u{b7} Goals"), "{shown}");
    assert!(!shown.contains(GOALS_NONE_YET), "{shown}");
    assert!(shows(&shown, "saved goals \u{b7} v1"), "{shown}");
}

#[tokio::test]
async fn v_prefills_the_title_and_hands_that_versions_body() {
    let (bench, mut section) = opened(vec![goals_v1()]).await;

    bench.key(&mut section, "v");
    assert_eq!(section.key_stack(), Some(views::DOCUMENTS_FORM));
    let shown = frame(&bench, &section);
    assert!(shown.contains("new version of `goals`"), "{shown}");
    insta::assert_snapshot!("form", shown);

    type_text(&bench, &mut section, " 2027");
    bench.key(&mut section, "enter");
    let handed = edits(&bench);
    assert!(
        matches!(handed.as_slice(), [edit] if edit.text == goals_v1().body),
        "{handed:?}"
    );

    section.on_external_edit(
        ExternalEditOutcome::Edited("# Goals\n\n- Ship it all.".to_owned()),
        &mut bench.ctx(),
    );
    let sent = requests(&bench);
    assert!(
        matches!(sent.as_slice(), [write] if written(write) == Some((GOALS_KIND, "Goals 2027", "# Goals\n\n- Ship it all."))),
        "{sent:?}"
    );
}

#[tokio::test]
async fn an_unchanged_body_sends_nothing() {
    let (bench, mut section) = opened(vec![goals_v1()]).await;
    bench.key(&mut section, "v");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    section.on_external_edit(
        ExternalEditOutcome::Unchanged { quick: false },
        &mut bench.ctx(),
    );
    let sent = requests(&bench);
    assert!(sent.is_empty(), "{sent:?}");
    assert!(shows(&frame(&bench, &section), NO_CHANGES));

    // An `Edited` that only adds the editor's final newline is no change either.
    bench.key(&mut section, "v");
    bench.key(&mut section, "enter");
    let _ = bench.drained();
    section.on_external_edit(
        ExternalEditOutcome::Edited(format!("{}\n", goals_v1().body)),
        &mut bench.ctx(),
    );
    let sent = requests(&bench);
    assert!(sent.is_empty(), "{sent:?}");
    assert!(shows(&frame(&bench, &section), NO_CHANGES));

    // A failure says its sentence and writes nothing.
    bench.key(&mut section, "v");
    bench.key(&mut section, "enter");
    let _ = bench.drained();
    section.on_external_edit(
        ExternalEditOutcome::Failed("the editor could not start".to_owned()),
        &mut bench.ctx(),
    );
    let sent = requests(&bench);
    assert!(sent.is_empty(), "{sent:?}");
    assert!(shows(
        &frame(&bench, &section),
        "the editor could not start"
    ));
}

#[tokio::test]
async fn enter_shows_the_body_and_esc_goes_back() {
    let (bench, mut section) = opened(vec![goals_v1()]).await;

    bench.key(&mut section, "enter");
    assert_eq!(section.key_stack(), Some(views::DOCUMENTS_VIEW));
    let shown = frame(&bench, &section);
    assert!(shown.contains("- Ship the tutorials."), "{shown}");
    insta::assert_snapshot!("view", shown);

    bench.key(&mut section, "esc");
    assert_eq!(section.key_stack(), Some(views::DOCUMENTS_BROWSE));
    assert!(frame(&bench, &section).contains("goals \u{b7} v1 \u{b7} Goals"));

    // `Esc` in the list closes the pane back to the tree.
    bench.key(&mut section, "esc");
    assert_eq!(section.key_stack(), Some(views::HIERARCHY_BROWSE));
}

#[tokio::test]
async fn o_on_a_repo_row_shows_the_notice() {
    let backend = Backend::memory(MemStore::demo());
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
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new().with_home(Some("/home/x"));
    bench.reply(&mut section, &tree);
    bench.key(&mut section, "j");
    bench.key(&mut section, "j");
    let _ = bench.drained();
    let before = frame(&bench, &section);
    assert!(
        before
            .lines()
            .any(|line| line.contains('\u{258c}') && line.contains("core")),
        "the cursor is on the repo: {before}"
    );

    bench.key(&mut section, "o");

    let sent = requests(&bench);
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(section.key_stack(), Some(views::HIERARCHY_BROWSE));
    assert!(
        shows(&frame(&bench, &section), PICK_AN_OWNER),
        "{}",
        frame(&bench, &section)
    );
}

#[tokio::test]
async fn the_outcome_reaches_the_pane_through_the_settings_tab() {
    let (bench, mut section) = opened(Vec::new()).await;
    bench.key(&mut section, "n");
    type_text(&bench, &mut section, "Goals");
    bench.key(&mut section, "enter");
    assert_eq!(edits(&bench).len(), 1);

    // Review Low 2: two sections, `Agents` active first; the outcome goes to the active one only.
    let mut tab =
        SettingsTab::with_sections(vec![Box::new(AgentsSection::new()), Box::new(section)]);
    let _ = bench.drained();
    tab.on_external_edit(
        ExternalEditOutcome::Edited("Ship.".to_owned()),
        &mut bench.ctx(),
    );
    let sent = requests(&bench);
    assert!(
        sent.is_empty(),
        "the inactive Hierarchy got the outcome: {sent:?}"
    );

    let next = KeyChord::parse("l").expect("a chord").to_event();
    tab.on_key(next, &mut bench.ctx());
    let _ = bench.drained();
    assert_eq!(
        tab.key_stack(),
        Some(views::DOCUMENTS_BROWSE),
        "Hierarchy, its documents pane open, is active"
    );
    tab.on_external_edit(
        ExternalEditOutcome::Edited("Ship.".to_owned()),
        &mut bench.ctx(),
    );

    let sent = requests(&bench);
    assert!(
        matches!(sent.as_slice(), [write] if written(write) == Some((GOALS_KIND, "Goals", "Ship."))),
        "{sent:?}"
    );
}

#[tokio::test]
async fn a_bad_title_stays_in_the_form_and_hands_nothing_out() {
    let (bench, mut section) = opened(Vec::new()).await;
    bench.key(&mut section, "n");
    bench.key(&mut section, "enter");

    assert_eq!(edits(&bench), Vec::<ExternalEdit>::new());
    assert_eq!(section.key_stack(), Some(views::DOCUMENTS_FORM));
    assert!(
        shows(&frame(&bench, &section), "title: a document needs a title"),
        "{}",
        frame(&bench, &section)
    );
}

// ---- review fix round R1 (M2, Low 2, Low 3) ----

/// A worker's `Failed` for the write.
fn write_failed(message: &str) -> StoreReply {
    StoreReply::Failed {
        request: WRITE_NAME,
        message: message.to_owned(),
    }
}

/// The refusal a constraint answers: nothing was written.
fn constraint() -> String {
    StoreError::Constraint("a document needs a title".to_owned()).to_string()
}

/// The answer to a write whose COMMIT may have landed before the connection dropped.
fn ambiguous() -> String {
    StoreError::Backend("connection reset by peer".to_owned()).to_string()
}

/// A second goals version, as the store would hold it once the write landed.
fn goals_v2() -> OwnerDocument {
    document(9, GOALS_KIND, 2, "Goals 2027", "Draft.", 5)
}

/// `v` on the goals row, the title typed on, `$EDITOR` returning `Draft.`: the write is in flight.
async fn writing_a_new_goals_version() -> (SectionBench, HierarchySection) {
    let (bench, mut section) = opened(vec![goals_v1()]).await;
    bench.key(&mut section, "v");
    type_text(&bench, &mut section, " 2027");
    bench.key(&mut section, "enter");
    assert_eq!(edits(&bench).len(), 1);
    section.on_external_edit(
        ExternalEditOutcome::Edited("Draft.".to_owned()),
        &mut bench.ctx(),
    );
    let sent = requests(&bench);
    assert!(
        matches!(sent.as_slice(), [write] if written(write) == Some((GOALS_KIND, "Goals 2027", "Draft."))),
        "{sent:?}"
    );
    (bench, section)
}

/// Review M2: a refused write (a constraint, or the offline refusal) keeps the kind, the title
/// and the text; `Enter`, `n` and `v` each reopen `$EDITOR` on the kept text, and its edit
/// writes with the kept kind and title.
#[tokio::test]
async fn a_refused_write_keeps_the_draft_and_reopening_hands_the_kept_text() {
    let offline = format!(
        "{}",
        StoreError::Unreachable(htui_store::DATABASE_UNREACHABLE.to_owned())
    );
    for message in [constraint(), offline] {
        for reopen in ["enter", "n", "v"] {
            let (bench, mut section) = writing_a_new_goals_version().await;
            bench.reply(&mut section, &write_failed(&message));

            assert!(
                requests(&bench).is_empty(),
                "a refusal is not re-read: {message}"
            );
            let shown = frame(&bench, &section);
            assert!(shows(&shown, &format!("not saved: {message}")), "{shown}");
            assert!(shows(&shown, "your text is kept"), "{shown}");

            bench.key(&mut section, reopen);
            let handed = edits(&bench);
            assert!(
                matches!(handed.as_slice(), [edit] if edit.text == "Draft." && edit.stem == "graphics-goals"),
                "`{reopen}` reopens $EDITOR on the kept text: {handed:?}"
            );
            assert_eq!(section.key_stack(), Some(views::DOCUMENTS_BROWSE));

            section.on_external_edit(
                ExternalEditOutcome::Edited("Draft, fixed.".to_owned()),
                &mut bench.ctx(),
            );
            let sent = requests(&bench);
            assert!(
                matches!(sent.as_slice(), [write] if written(write) == Some((GOALS_KIND, "Goals 2027", "Draft, fixed."))),
                "{sent:?}"
            );
        }
    }
}

/// Review M2: closing `$EDITOR` untouched on a kept draft is the retry, and writes the kept text.
#[tokio::test]
async fn an_untouched_reopen_writes_the_kept_text() {
    let (bench, mut section) = writing_a_new_goals_version().await;
    bench.reply(&mut section, &write_failed(&constraint()));
    bench.key(&mut section, "enter");
    assert_eq!(edits(&bench).len(), 1);

    section.on_external_edit(
        ExternalEditOutcome::Unchanged { quick: false },
        &mut bench.ctx(),
    );

    let sent = requests(&bench);
    assert!(
        matches!(sent.as_slice(), [write] if written(write) == Some((GOALS_KIND, "Goals 2027", "Draft."))),
        "{sent:?}"
    );
}

/// Review M2: an ambiguous failure re-reads the list, and a newer version of the kind there means
/// the write landed: the pane says saved and keeps no draft.
#[tokio::test]
async fn an_ambiguous_failure_whose_reread_shows_the_new_version_reports_saved() {
    let (bench, mut section) = writing_a_new_goals_version().await;
    let why = ambiguous();
    bench.reply(&mut section, &write_failed(&why));

    let reread = requests(&bench);
    assert!(
        matches!(reread.as_slice(), [StoreRequest::OwnerDocuments(owner)] if *owner == WORKSPACE),
        "{reread:?}"
    );
    let shown = frame(&bench, &section);
    assert!(
        shows(
            &shown,
            &format!("{why} \u{2014} it may have been written; the list is being re-read to check")
        ),
        "{shown}"
    );
    // Still settling: another write waits.
    bench.key(&mut section, "n");
    assert!(shows(&frame(&bench, &section), STILL_SAVING));
    assert!(edits(&bench).is_empty());

    bench.reply(
        &mut section,
        &StoreReply::OwnerDocuments {
            owner: WORKSPACE,
            documents: vec![goals_v1(), goals_v2()],
            writable: true,
        },
    );

    let shown = frame(&bench, &section);
    assert!(shows(&shown, "saved goals \u{b7} v2"), "{shown}");
    assert!(!shows(&shown, "your text is kept"), "{shown}");
    // No draft: `Enter` views the version the cursor followed to.
    bench.key(&mut section, "enter");
    assert!(edits(&bench).is_empty());
    assert_eq!(section.key_stack(), Some(views::DOCUMENTS_VIEW));
    assert!(frame(&bench, &section).contains("goals \u{b7} v2 \u{b7} Goals 2027"));
}

/// Review M2: an ambiguous failure whose re-read holds no newer version was not written: the draft
/// is kept and `Enter` reopens `$EDITOR` on it.
#[tokio::test]
async fn an_ambiguous_failure_whose_reread_shows_nothing_new_keeps_the_draft() {
    let (bench, mut section) = writing_a_new_goals_version().await;
    let why = ambiguous();
    bench.reply(&mut section, &write_failed(&why));
    let _ = requests(&bench);
    bench.reply(
        &mut section,
        &StoreReply::OwnerDocuments {
            owner: WORKSPACE,
            documents: vec![goals_v1()],
            writable: true,
        },
    );

    let shown = frame(&bench, &section);
    assert!(
        shows(
            &shown,
            &format!("{why} \u{2014} the re-read shows it was not written")
        ),
        "{shown}"
    );
    assert!(shows(&shown, "your text is kept"), "{shown}");
    bench.key(&mut section, "enter");
    let handed = edits(&bench);
    assert!(
        matches!(handed.as_slice(), [edit] if edit.text == "Draft."),
        "{handed:?}"
    );
}

/// Review M2: a write key while `$EDITOR` holds a body says so, and one while a write is in
/// flight says to wait; neither hands anything out.
#[tokio::test]
async fn a_write_key_while_the_editor_is_open_or_a_write_is_in_flight_is_refused() {
    let (bench, mut section) = opened(vec![goals_v1()]).await;
    bench.key(&mut section, "v");
    bench.key(&mut section, "enter");
    assert_eq!(edits(&bench).len(), 1);

    for key in ["n", "v"] {
        bench.key(&mut section, key);
        assert!(edits(&bench).is_empty(), "`{key}`");
        assert!(shows(&frame(&bench, &section), EDITOR_OPEN), "`{key}`");
    }

    section.on_external_edit(
        ExternalEditOutcome::Edited("Changed.".to_owned()),
        &mut bench.ctx(),
    );
    assert_eq!(requests(&bench).len(), 1);
    for key in ["n", "v"] {
        bench.key(&mut section, key);
        assert!(edits(&bench).is_empty(), "`{key}`");
        assert!(shows(&frame(&bench, &section), STILL_SAVING), "`{key}`");
    }
}

/// Review Low 3: a refused read shows `documents unavailable: …`, offers only `Esc`, and refuses
/// the write keys with the same sentence.
#[tokio::test]
async fn a_refused_read_shows_unavailable_and_refuses_the_write_keys() {
    let (bench, mut section) = opened(vec![goals_v1()]).await;
    let why = StoreError::Backend("the pool is closed".to_owned()).to_string();
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: READ_NAME,
            message: why.clone(),
        },
    );

    let unavailable = format!("{UNAVAILABLE}: {why}");
    let shown = frame(&bench, &section);
    assert!(shows(&shown, &unavailable), "{shown}");
    assert!(
        !shown.contains("goals \u{b7} v1"),
        "no list while unavailable: {shown}"
    );
    assert!(
        bench.key_hint(&section).ends_with("back"),
        "{}",
        bench.key_hint(&section)
    );
    bench.key(&mut section, "n");
    assert!(edits(&bench).is_empty());
    assert_eq!(section.key_stack(), Some(views::DOCUMENTS_BROWSE));
    assert!(
        shows(&frame(&bench, &section), &unavailable),
        "{}",
        frame(&bench, &section)
    );
}

/// Review M2 (optional half): a list read offline says it cannot be written, and `n`/`v` refuse
/// with the offline sentence before `$EDITOR` opens.
#[tokio::test]
async fn offline_the_write_keys_refuse_before_the_editor_opens() {
    let (bench, mut section) = opened(Vec::new()).await;
    bench.reply(
        &mut section,
        &StoreReply::OwnerDocuments {
            owner: WORKSPACE,
            documents: vec![goals_v1()],
            writable: false,
        },
    );
    let offline = StoreError::Unreachable(htui_store::DATABASE_UNREACHABLE.to_owned());
    for key in ["n", "v"] {
        bench.key(&mut section, key);
        assert!(edits(&bench).is_empty(), "`{key}`");
        assert_eq!(
            section.key_stack(),
            Some(views::DOCUMENTS_BROWSE),
            "`{key}`"
        );
        assert!(
            shows(&frame(&bench, &section), &format!("not saved: {offline}")),
            "`{key}`: {}",
            frame(&bench, &section)
        );
    }
}

/// Review M2: after an `Unreachable` failure the re-read is the mirror's, which cannot hold a
/// write whose answer was lost: nothing new there says it could not be checked, and keeps the
/// draft.
#[tokio::test]
async fn an_unreachable_failure_cannot_be_settled_by_the_mirror() {
    let (bench, mut section) = writing_a_new_goals_version().await;
    let why = StoreError::Unreachable("connection reset by peer".to_owned()).to_string();
    bench.reply(&mut section, &write_failed(&why));
    assert_eq!(requests(&bench).len(), 1, "the list is re-read");
    bench.reply(
        &mut section,
        &StoreReply::OwnerDocuments {
            owner: WORKSPACE,
            documents: vec![goals_v1()],
            writable: false,
        },
    );

    let shown = frame(&bench, &section);
    assert!(
        shows(
            &shown,
            "it may have been written, and the list could not be re-read to check"
        ),
        "{shown}"
    );
    assert!(shows(&shown, "your text is kept"), "{shown}");
    assert!(!shows(&shown, "it was not written"), "{shown}");
}
