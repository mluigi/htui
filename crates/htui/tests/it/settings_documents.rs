//! Settings › Hierarchy › Documents (MOD-147 M3 D9, D10): a workspace's or a project's owner
//! documents pane, opened from its row, driven through a [`SectionBench`] with replies built from
//! fixed-timestamp documents (a `MemStore` stamps real time), and once end to end through the
//! worker's `serve`.
#![cfg(feature = "testkit")]

use chrono::{DateTime, TimeZone as _, Utc};
use htui::app::Action;
use htui::editor::{ExternalEdit, ExternalEditOutcome, NO_CHANGES};
use htui::keys::views;
use htui::owner_documents::DocumentOwner;
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::SectionBench;
use htui::ui::tabs::registry::Tab as _;
use htui::ui::tabs::settings::hierarchy::PICK_AN_OWNER;
use htui::ui::tabs::settings::owner_documents::GOALS_NONE_YET;
use htui::ui::tabs::settings::{HierarchySection, SettingsSection, SettingsTab};
use htui_core::fixtures::ids;
use htui_core::model::document::GOALS_KIND;
use htui_core::model::{DocumentId, OwnerDocument};
use htui_core::store::MemStore;
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

    let mut tab = SettingsTab::with_sections(vec![Box::new(section)]);
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
