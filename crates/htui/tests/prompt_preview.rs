//! The Prompt sub-tab and the deferred preview (T67; plan D102, D103, D109, D110).
//!
//! `#![cfg(feature = "testkit")]` and `--features testkit`, like `backlog.rs`: `htui` declares
//! exactly one feature and the plan's `--features demo,test-support` names two it does not have
//! (blueprint E-1, finding F-50).
//!
//! Everything here runs the **whole** path the binary runs: the Backlog tab issues the request,
//! the agent runtime spawns the deferred task on an owned `Backend` clone, the task does the nine
//! store reads and calls the same `assemble()` MOD-4 will call, and the reply is rendered by the
//! sub-tab. That is the point of the preview and the reason ANA-5 §12 criterion 12 is asserted from
//! here rather than only over a fake reader.
#![cfg(feature = "testkit")]

use std::time::Duration;

use htui::agent_worker::{AgentRuntime, Served};
use htui::app::Action;
use htui::preview::{self, STAND_INS};
use htui::store_worker::{Origin, RequestEnvelope, StoreReply, StoreRequest};
use htui::testkit::Harness;
use htui::ui::tabs::backlog::BacklogTab;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::{
    Activation, ChoiceReason, ItemId, Scope, SkillChoice, SkillLevel, WorkspaceSummary,
};
use htui_core::store::{MemStore, ReadStore as _};
use htui_store::Backend;

/// How far right the Prompt sub-tab sits: Body, Runs, Graph, Documents, Notes, **Prompt**.
const TO_PROMPT: usize = 5;

/// Rows down from the arrival row to htui `FEAT-1`, the item every sub-tab has data for.
const TO_FEAT_1: usize = 3;

/// Rows down to htui `ANA-2`, the item nothing in the fixture attaches to.
const TO_ANA_2: usize = 1;

/// The workspace of the demo fixture with this slug.
async fn workspace(slug: &str) -> WorkspaceSummary {
    MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|workspace| workspace.slug == slug)
        .unwrap_or_else(|| panic!("the demo fixture holds the `{slug}` workspace"))
}

/// The `Platform` scope, the one the Backlog cases navigate in.
async fn platform_scope() -> Scope {
    Scope::from_workspace(&workspace("platform").await)
}

/// A settled Backlog tab over the demo store, wide enough that the preview's own lines are
/// readable rather than clipped at the 43-column detail pane of a 100x30 frame.
///
/// The runtime is the empty one: a preview spawns no process and needs no transport, and installing
/// the production factory here would only give the case a registry it never reads.
async fn backlog(width: u16, height: u16) -> Harness {
    let mut harness = Harness::demo()
        .size(width, height)
        .with_tab(Box::new(BacklogTab::new()))
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    harness.drive().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive_to_end().await;
    harness
}

/// Moves the selection down `n` rows, serving what each move asks the store for.
async fn down(harness: &mut Harness, n: usize) {
    for _ in 0..n {
        harness.key("j");
        harness.drive_to_end().await;
    }
}

/// Cycles to the sub-tab `n` steps to the right of Body.
fn sub_tab(harness: &mut Harness, n: usize) {
    for _ in 0..n {
        harness.key("l");
    }
}

/// The id of the demo item with this key, in the `Platform` workspace.
async fn item_id(key: &str) -> ItemId {
    let store = MemStore::demo();
    let scope = platform_scope().await;
    store
        .items(&scope, &htui_core::model::ItemFilter::default())
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|item| item.key == key)
        .unwrap_or_else(|| panic!("the demo fixture holds `{key}`"))
        .id
}

// ---------------------------------------------------------------------------------------------
// Criterion 12, from the running binary
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_prompt_sub_tab_previews_feat_1() {
    let mut harness = backlog(200, 60).await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, TO_PROMPT);
    let frame = harness.render();

    assert!(frame.contains("┌ FEAT-1"), "the preview is of htui FEAT-1");
    assert!(frame.contains("digest"), "the digest line is rendered");
    assert!(
        frame.contains("section") && frame.contains("template"),
        "the section list is rendered"
    );
    assert!(
        frame.contains("no_path"),
        "ANA-5 §12 criterion 12: the excerpt section is absent and the audit says why\n{frame}"
    );
    insta::assert_snapshot!("preview_feat_1", frame);
}

#[tokio::test]
async fn the_preview_assembles_from_real_store_reads() {
    // The same call the deferred task makes, with the outcome inspected rather than rendered:
    // criterion 12's three clauses as data.
    let backend = Backend::memory(MemStore::demo());
    let scope = platform_scope().await;
    let preview = preview::build(&backend, item_id("FEAT-1").await, None, &scope)
        .await
        .expect("the demo store answers every prompt read");

    let assembled = preview.outcome.as_ref().expect("the demo item assembles");
    assert_eq!(assembled.digest.len(), 64, "lowercase sha256 hex, all 64");
    assert!(
        !assembled.text.contains("<section name=\"excerpts\">"),
        "no root resolved, so there is no excerpt section"
    );
    assert!(
        assembled.trim.excerpts.roots.is_empty(),
        "plan D110: no `repo` row is read in this milestone, so `roots` is empty"
    );
    assert_eq!(assembled.trim.excerpts.considered, 0);
    assert_eq!(assembled.trim.excerpts.selected, 0);
    assert_eq!(
        assembled.trim.excerpts.provider_set,
        vec!["builtin@1".to_owned()],
        "the built-in ranker is registered first even when it had nothing to rank"
    );
    assert!(
        assembled.trim.excerpts.caps.max_files > 0,
        "`resolve_excerpt_caps` answered, so the audit records the caps the pass would have run \
         under"
    );
}

#[tokio::test]
async fn the_preview_declares_its_stand_ins() {
    let backend = Backend::memory(MemStore::demo());
    let scope = platform_scope().await;
    let preview = preview::build(&backend, item_id("FEAT-1").await, None, &scope)
        .await
        .expect("the demo store answers every prompt read");
    let notes = &preview
        .outcome
        .as_ref()
        .expect("the demo item assembles")
        .trim
        .notes;

    for stand_in in STAND_INS {
        assert!(
            notes.iter().any(|note| note == stand_in),
            "every stand-in is recorded in `trim_record.notes`; `{stand_in}` is not in {notes:#?}"
        );
    }
    // The blueprint's four verbatim strings (D.4), spelled out here so a reworded constant has to
    // be a deliberate edit in two files rather than a silent one in one.
    for stand_in in [
        "preview: documents are latest-per-kind; ANA-2 input_kinds resolution arrives with MOD-4",
        "preview: phase-level skills come from the first phase of the item's graph that uses \
         this template; a glob attachment records no_path because no root resolves",
        "preview: output_kind defaults to the template name; the phase row's value arrives with \
         MOD-4",
        "preview: command_queue exposure is a phase setting (R-MCP-4); absent until MOD-4",
    ] {
        assert!(
            notes.iter().any(|note| note == stand_in),
            "blueprint D.4's verbatim note is missing: `{stand_in}`"
        );
    }
}

/// MOD-9 D45's skills stand-in, verbatim.
const SKILLS_NOTE: &str = "preview: phase-level skills come from the first phase of the item's \
                           graph that uses this template; a glob attachment records no_path \
                           because no root resolves";

/// The start of MOD-9 D45's second note, the one a preview adds when no phase uses its template.
const NO_PHASE: &str = "preview: no phase of this item's graph uses template";

/// An `Always` choice that rendered, as the record spells it.
fn active(
    skill: htui_core::model::SkillId,
    name: &str,
    version: i32,
    level: SkillLevel,
) -> SkillChoice {
    SkillChoice {
        skill,
        name: name.to_owned(),
        version: Some(version),
        level,
        activation: Activation::Always,
        active: true,
        reason: ChoiceReason::Always,
    }
}

#[tokio::test]
async fn the_preview_carries_the_phase_skills_of_the_matching_phase() {
    // MOD-9 D45: FEAT-1 runs the `feature` graph, whose `implement` phase uses the `implement`
    // template and carries the demo's one phase-level binding (`rust-style` pinned to v1). The
    // preview under that template resolves the phase and shows what the step would get.
    let backend = Backend::memory(MemStore::demo());
    let scope = platform_scope().await;
    let preview = preview::build(&backend, item_id("FEAT-1").await, Some("implement"), &scope)
        .await
        .expect("the demo store answers every prompt read");
    let assembled = preview.outcome.as_ref().expect("the demo item assembles");

    let tests = assembled
        .text
        .find("<skill name=\"tests\" version=\"1\">")
        .expect("the project's `tests` skill renders");
    let rust_style = assembled
        .text
        .find("<skill name=\"rust-style\" version=\"1\">")
        .expect("the phase pin holds `rust-style` at v1, not the project's v2");
    assert!(tests < rust_style, "collapse order: position 0, then 2");
    assert_eq!(
        assembled.trim.skill_choices,
        vec![
            active(ids::SKILL_TESTS, "tests", 1, SkillLevel::Project),
            active(ids::SKILL_RUST_STYLE, "rust-style", 1, SkillLevel::Phase),
        ],
        "D45: the phase-level binding wins over the project one"
    );
    let notes = &assembled.trim.notes;
    assert!(
        notes.iter().any(|note| note == SKILLS_NOTE),
        "the skills stand-in is recorded: {notes:#?}"
    );
    assert!(
        !notes.iter().any(|note| note.starts_with(NO_PHASE)),
        "a phase uses `implement`, so there is no no-phase note: {notes:#?}"
    );
}

#[tokio::test]
async fn a_template_no_phase_uses_is_noted_and_shows_project_skills() {
    // MOD-9 D45's second note: the `feature` graph has no `research` phase, so only global and
    // project attachments apply and the record says why.
    let backend = Backend::memory(MemStore::demo());
    let scope = platform_scope().await;
    let preview = preview::build(&backend, item_id("FEAT-1").await, Some("research"), &scope)
        .await
        .expect("the demo store answers every prompt read");
    let assembled = preview.outcome.as_ref().expect("the demo item assembles");

    let notes = &assembled.trim.notes;
    assert!(
        notes.iter().any(|note| note
            == "preview: no phase of this item's graph uses template `research`; global and \
                project skills only"),
        "D45's second note, verbatim: {notes:#?}"
    );
    assert_eq!(
        assembled.trim.skill_choices,
        vec![
            active(ids::SKILL_TESTS, "tests", 1, SkillLevel::Project),
            active(ids::SKILL_RUST_STYLE, "rust-style", 2, SkillLevel::Project),
        ],
        "no phase, so the project binding (unpinned, v2) is the one in force"
    );
}

#[tokio::test]
async fn the_preview_records_the_latest_version_it_chose() {
    // §4.7 rule 9: the assembler never resolves `latest`, so the preview pins a version and records
    // which one. The demo fixture seeds one version per name, so the recorded version is that row's.
    let backend = Backend::memory(MemStore::demo());
    let scope = platform_scope().await;
    let item = item_id("FEAT-1").await;
    let preview = preview::build(&backend, item, None, &scope)
        .await
        .expect("the demo store answers every prompt read");

    let chosen = preview
        .template
        .as_ref()
        .expect("the project has templates");
    let assembled = preview.outcome.as_ref().expect("the demo item assembles");
    assert_eq!(
        assembled.trim.template.name, chosen.name,
        "the record names the row the picker shows"
    );
    assert_eq!(assembled.trim.template.version, chosen.version);

    let latest = backend
        .prompt_templates(
            MemStore::demo()
                .item(item)
                .await
                .expect("the memory store never fails")
                .expect("the demo item exists")
                .project_id,
        )
        .await
        .expect("the memory store never fails")
        .into_iter()
        .filter(|row| row.name == chosen.name)
        .map(|row| row.version)
        .max()
        .expect("the chosen name has at least one row");
    assert_eq!(chosen.version, latest, "the latest version, not the first");
}

#[tokio::test]
async fn the_default_template_is_the_first_default_name_the_project_has() {
    // Blueprint D.4: `prd` for every seeded project, and the two reserved names are never offered
    // because their inputs are MOD-4's (plan D107).
    let backend = Backend::memory(MemStore::demo());
    let scope = platform_scope().await;
    let preview = preview::build(&backend, item_id("FEAT-1").await, None, &scope)
        .await
        .expect("the demo store answers every prompt read");

    assert_eq!(
        preview.template.as_ref().map(|row| row.name.as_str()),
        Some("prd")
    );
    assert!(
        !preview.available.iter().any(|name| name == "judge"),
        "the reserved names are not offered: {:?}",
        preview.available
    );
    assert!(!preview.available.iter().any(|name| name == "handoff"));
    assert!(
        preview.available.len() >= 8,
        "the eight seeded phase names are offered: {:?}",
        preview.available
    );
}

#[tokio::test]
async fn n_cycles_to_the_next_template_and_p_back() {
    let mut harness = backlog(200, 60).await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, TO_PROMPT);
    let first = harness.render();
    assert!(first.contains("prd"), "the default template is `prd`");

    harness.key("n");
    harness.drive_to_end().await;
    let second = harness.render();
    assert_ne!(first, second, "`n` re-previews under the next template");

    harness.key("p");
    harness.drive_to_end().await;
    assert_eq!(
        harness.render(),
        first,
        "`p` is `n`'s inverse, and the picker's own reply is never dropped as stale (H-18)"
    );
}

#[tokio::test]
async fn the_preview_writes_nothing() {
    // Plan D102: no `set_step_prompt`, no `prompt` event, no run. The store is the witness.
    let store = MemStore::demo();
    let before = store
        .runs(item_id("FEAT-1").await)
        .await
        .expect("the memory store never fails");

    let backend = Backend::memory(store.clone());
    let scope = platform_scope().await;
    preview::build(&backend, item_id("FEAT-1").await, None, &scope)
        .await
        .expect("the demo store answers every prompt read");

    let after = store
        .runs(item_id("FEAT-1").await)
        .await
        .expect("the memory store never fails");
    assert_eq!(before, after, "a preview leaves no row behind");
}

#[tokio::test]
async fn an_item_the_backend_does_not_hold_is_a_store_error() {
    // The `Err` arm of `build` is **only** a store failure: a project with no template and a
    // refused assembly are both `Ok` previews the pane renders. Nothing is written on this path
    // either, which is what makes a failed preview a non-event rather than a half-finished one.
    let backend = Backend::memory(MemStore::new());
    let scope = platform_scope().await;
    let error = preview::build(&backend, item_id("FEAT-1").await, None, &scope)
        .await
        .expect_err("an empty store holds no such item");
    assert!(
        matches!(
            error,
            htui_core::store::StoreError::NotFound { entity: "item", .. }
        ),
        "the read that failed is named: {error}"
    );
}

#[tokio::test]
async fn the_preview_refuses_offline_with_one_sentence() {
    // Plan D109 / blueprint H-16: the refusal happens **before** anything is spawned, so an offline
    // box does not pay for a task that could only fail on its first read. `background_len() == 0`
    // afterwards is the whole assertion — a refused preview spawned nothing rather than spawning
    // and then discarding, which is what `background_len` was given to `probe` for.
    let mut runtime = AgentRuntime::new(DriverFactory::new());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (backend, _mirror) = offline_backend().await;
    let envelope = RequestEnvelope {
        seq: 7,
        origin: Origin::Tab(BacklogTab::ID),
        request: StoreRequest::PromptPreview {
            item: item_id("FEAT-1").await,
            template_name: None,
            scope: platform_scope().await,
        },
    };

    let served = runtime.serve(&backend, &tx, &envelope).await;
    assert_eq!(runtime.background_len(), 0, "nothing was spawned");
    let Served::Reply(StoreReply::Failed { request, message }) = served else {
        panic!("an offline preview is refused inline, not deferred: {served:?}");
    };
    assert_eq!(request, "prompt_preview");
    assert!(
        message.contains(preview::offline_refusal()),
        "one named sentence, not a bare `Unreachable`: {message}"
    );
    assert_eq!(
        preview::offline_refusal(),
        htui_store::PROMPT_ON_SERVER_ONLY
    );
    assert!(
        rx.try_recv().is_err(),
        "the refusal is the `Served::Reply`, so nothing went down the frame channel too"
    );
}

#[tokio::test]
async fn an_item_with_nothing_attached_still_previews() {
    // htui `ANA-2` is the item no document and no link points at, so its preview is ANA-5 §12
    // criterion 4 from the running binary: the three optional sections are absent and the section
    // table has a row only for what contributed bytes.
    let mut harness = backlog(100, 30).await;
    down(&mut harness, TO_ANA_2).await;
    sub_tab(&mut harness, TO_PROMPT);
    let frame = harness.render();
    assert!(frame.contains("┌ ANA-2"));
    // Two of criterion 4's three sections would have a row in the table if they had bytes; the
    // third is the excerpt section, whose absence is what the `roots` line accounts for.
    assert!(
        !frame.contains("documents:"),
        "no input document, so no `documents:` row:\n{frame}"
    );
    assert!(
        !frame.contains("upstream"),
        "no upstream link, so no `upstream` row:\n{frame}"
    );
    assert!(frame.contains("no_path"), "criterion 12 again, at 100x30");
    insta::assert_snapshot!("preview_ana_2", frame);
}

#[tokio::test]
async fn the_preview_is_deferred_onto_a_task_the_runtime_owns() {
    // `R-NF-3` as a fact about the runtime rather than about a comment: serving a preview pushes
    // exactly one background task and returns `Served::Deferred`, so the worker's `select!` arm
    // returns having awaited nothing. The reply then arrives on the frame channel, from the task.
    let mut runtime = AgentRuntime::new(DriverFactory::new());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let backend = Backend::memory(MemStore::demo());
    let envelope = RequestEnvelope {
        seq: 11,
        origin: Origin::Tab(BacklogTab::ID),
        request: StoreRequest::PromptPreview {
            item: item_id("FEAT-1").await,
            template_name: None,
            scope: platform_scope().await,
        },
    };

    let served = runtime.serve(&backend, &tx, &envelope).await;
    assert!(
        matches!(served, Served::Deferred),
        "the worker's arm gets `Deferred`, not a reply it had to wait for: {served:?}"
    );
    assert_eq!(
        runtime.background_len(),
        1,
        "one task, owned by the runtime"
    );

    runtime.finish_background(Duration::from_secs(5)).await;
    let answer = rx.try_recv().expect("the task answered its own request");
    assert_eq!(answer.seq, 11, "at the asking request's own seq");
    assert_eq!(answer.origin, Origin::Tab(BacklogTab::ID));
    assert!(matches!(answer.reply, StoreReply::PromptPreview(_)));
    assert!(
        rx.try_recv().is_err(),
        "exactly one reply per request, whatever the backend did"
    );
}

/// The preview request `seq` asks for, from `origin`.
fn preview_request(seq: u64, origin: Origin, item: ItemId, scope: Scope) -> RequestEnvelope {
    RequestEnvelope {
        seq,
        origin,
        request: StoreRequest::PromptPreview {
            item,
            template_name: None,
            scope,
        },
    }
}

#[tokio::test]
async fn a_second_preview_from_one_origin_aborts_the_first() {
    // Review finding M3: holding `j` across 30 rows used to spawn 30 preview tasks, each doing
    // eight store reads against an eight-connection pool and assembling to the token budget. The
    // shell's staleness index discarded 29 of the replies — *after* their work was finished — and
    // the worker's own `Item`/`Runs` reads queued behind them on `acquire`. Only the newest
    // selection's preview is wanted, so the older one is aborted rather than raced.
    //
    // Neither task is polled between the two `serve` calls: a preview's arm awaits nothing
    // (`R-NF-3`), so the current-thread scheduler never gets control, which is exactly the shape of
    // a held key — the worker drains the channel faster than a task can reach its first store read.
    let mut runtime = AgentRuntime::new(DriverFactory::new());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let backend = Backend::memory(MemStore::demo());
    let item = item_id("FEAT-1").await;
    let scope = platform_scope().await;
    let origin = Origin::Tab(BacklogTab::ID);

    let first = preview_request(21, origin.clone(), item, scope.clone());
    let second = preview_request(22, origin.clone(), item, scope);
    runtime.serve(&backend, &tx, &first).await;
    runtime.serve(&backend, &tx, &second).await;

    runtime.finish_background(Duration::from_secs(5)).await;
    let answer = rx.try_recv().expect("the newest preview still answers");
    assert_eq!(
        answer.seq, 22,
        "the reply delivered is the second request's, not the first's"
    );
    assert_eq!(answer.origin, origin);
    assert!(matches!(answer.reply, StoreReply::PromptPreview(_)));
    assert!(
        rx.try_recv().is_err(),
        "the superseded preview was aborted, so it never finished its reads to answer"
    );
}

#[tokio::test]
async fn a_preview_abort_is_per_origin() {
    // The abort is keyed on the asking `Origin`, so the Backlog tab superseding its own preview
    // cannot cancel one an overlay or the shell is waiting on. Two origins, one request each, and
    // both answer.
    let mut runtime = AgentRuntime::new(DriverFactory::new());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let backend = Backend::memory(MemStore::demo());
    let item = item_id("FEAT-1").await;
    let scope = platform_scope().await;

    let tab = preview_request(31, Origin::Tab(BacklogTab::ID), item, scope.clone());
    let app = preview_request(32, Origin::App, item, scope);
    runtime.serve(&backend, &tx, &tab).await;
    runtime.serve(&backend, &tx, &app).await;

    runtime.finish_background(Duration::from_secs(5)).await;
    let mut answered: Vec<(u64, Origin)> = Vec::new();
    while let Ok(reply) = rx.try_recv() {
        assert!(matches!(reply.reply, StoreReply::PromptPreview(_)));
        answered.push((reply.seq, reply.origin));
    }
    answered.sort_by_key(|(seq, _)| *seq);
    assert_eq!(
        answered,
        vec![(31, Origin::Tab(BacklogTab::ID)), (32, Origin::App)],
        "a second origin's preview is untouched by the first origin's supersession"
    );
}

/// A `Backend::Offline` over a throwaway mirror: the arm plan D109 refuses.
///
/// The mirror is empty and stays empty — the refusal happens before the first read, which is the
/// whole point of H-16 — so the directory only has to outlive the call, which the returned guard
/// is what arranges.
async fn offline_backend() -> (Backend, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("a temporary directory");
    let cache = htui_store::cache::CacheStore::open(
        root.path(),
        "prompt-preview-offline",
        htui_store::PgStore::schema_version(),
    )
    .await
    .expect("a fresh mirror opens");
    (Backend::Offline { cache, since: None }, root)
}
