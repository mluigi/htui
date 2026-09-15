//! The prompt preview: eight store reads, one `assemble()`, and no write at all (plan D102, D103).
//!
//! This is the milestone's reason to exist. MOD-4's stage 3 will build a [`PromptSpec`] from a
//! `run_step` and hand it to [`assemble`]; the preview builds **the same type** and hands it to
//! **the same function**, so the bytes a maintainer reads here are the bytes a run would produce.
//! What differs is who fills three of the fields, and plan D103's rule is that every one of those
//! is declared in `trim_record.notes` rather than silently defaulted — see [`STAND_INS`]. Inventing
//! an `input_kinds` order or pointing the excerpt walk at the process's working directory would
//! make the preview's bytes a thing no real run would ever produce, which defeats the only purpose
//! it has.
//!
//! **It writes nothing.** No `set_step_prompt`, no `prompt` event, no run. That is correctness
//! rather than caution: `set_step_prompt` writes *a step's* audit row and a preview has no step.
//!
//! **It runs off the UI task and off the worker's `select!` arm** (`R-NF-3`, plan D102). The
//! assembly reads five tables and can be hundreds of kilobytes of rendering, so
//! [`AgentRuntime::serve`](crate::agent_worker::AgentRuntime::serve) spawns [`run_preview`] on a
//! cloned [`Backend`] and returns `Served::Deferred`, the shape milestone 5's `ProbeAgents`
//! established. The clone is a snapshot: it cannot perform the backend swap the worker owns, and a
//! task holding one past a swap reads the old arm, maps its `StoreError` to a `Failed` reply and
//! exits (blueprint H-17).
//!
//! **Offline it refuses before anything is spawned** (plan D109, blueprint H-16). `prompt_template`,
//! `skill*` and `box_tool` are not mirrored, so an offline preview is not one missing setting but
//! four missing tables; `htui` is an online-only program and says so in one sentence.

use htui_core::model::{ItemId, PromptScope, Scope};
use htui_core::prompt::excerpt::{BUILTIN_ID, ExcerptAudit, ExcerptSet};
use htui_core::prompt::{
    AssembledPrompt, DEFAULT_TEMPLATES, InputDocument, PromptSpec, TemplateRef, TemplateRole,
    TokenEstimator, assemble, settings,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{ReadStore as _, Result as StoreResult, StoreError};
use htui_store::Backend;
use tokio::sync::mpsc;

use crate::agent_worker::ReplyAddr;
use crate::store_worker::{ReplyEnvelope, StoreReply};

/// The `document.kind` the preview never renders as an input document.
///
/// `documents_of_kinds(item, &[])` means "every kind this item has", and a `summary` is upstream
/// context — §4.3 renders it inside the `upstream` section — not an input of this item's own phase
/// (blueprint H-21). Excluding it here and saying so is what keeps the two from being shown twice.
const SUMMARY_KIND: &str = "summary";

/// The two reserved template names (ANA-5 §4.6), never offered by the picker.
///
/// Their inputs are MOD-4's — a judge prompt needs a fan-out's candidates and a handoff prompt
/// needs an abandoned step's events — so previewing them would mean inventing both (plan D107).
const RESERVED: [&str; 2] = ["judge", "handoff"];

/// Plan D103's declared stand-ins, in the order [`build`] records them.
///
/// Every one of them is a field MOD-4's stage 3 will fill from a `run_step` or a `ResolvedPhase`
/// and the preview has no row for. They are a public constant so a test can assert the set rather
/// than a spelling, and so the sub-tab can render them without re-deriving the list: the risk the
/// plan records against this task is that the preview drifts from what a run assembles, and a
/// stand-in nobody can see is exactly how that happens.
pub const STAND_INS: [&str; 8] = [
    TEMPLATE_NOTE,
    DOCUMENTS_NOTE,
    SUMMARY_NOTE,
    SKILLS_NOTE,
    OUTPUT_KIND_NOTE,
    COMMAND_QUEUE_NOTE,
    ATTEMPT_NOTE,
    EXCERPTS_NOTE,
];

/// The template is picked by name and pinned to its latest version, standing in for a phase row.
const TEMPLATE_NOTE: &str = "preview: the template is chosen by name at its latest version and \
                             stands in for the phase; the phase row's template_name and its pin \
                             arrive with MOD-4";
/// Blueprint D.4's first verbatim note.
const DOCUMENTS_NOTE: &str = "preview: documents are latest-per-kind; ANA-2 input_kinds resolution \
                              arrives with MOD-4";
/// Blueprint H-21: `documents_of_kinds(item, &[])` would otherwise return `summary` too.
const SUMMARY_NOTE: &str = "preview: the `summary` kind is excluded from documents; §4.3 renders \
                            it as upstream context instead";
/// Blueprint D.4's second verbatim note.
const SKILLS_NOTE: &str =
    "preview: project-level skill bindings only; a phase binding needs a phase id (MOD-4)";
/// Blueprint D.4's third verbatim note.
const OUTPUT_KIND_NOTE: &str = "preview: output_kind defaults to the template name; the phase \
                                row's value arrives with MOD-4";
/// Blueprint D.4's fourth verbatim note.
const COMMAND_QUEUE_NOTE: &str =
    "preview: command_queue exposure is a phase setting (R-MCP-4); absent until MOD-4";
/// Attempt 1, so `verify_failure` and `previous_diff` are absent by the normal empty-section rule.
const ATTEMPT_NOTE: &str = "preview: attempt 1, so verify_failure and previous_diff are absent; \
                            the real attempt comes from run_step (MOD-4)";
/// ANA-5 §12 criterion 12, as the preview meets it (plan D103, D110).
const EXCERPTS_NOTE: &str = "preview: no run_step_tree row and no repo_box_path row, so no root \
                             resolves; the excerpt section is absent and roots records no_path per \
                             repo (plan D110: no repo row is read in this milestone)";

/// What the Prompt sub-tab receives: one reply per request, the assembler's verdict either way.
///
/// `outcome` is a `Result` rather than an `Option` because a refusal **is** a preview: "prompt
/// budget too small" is the most useful thing this pane can say to a maintainer who has just
/// mistyped a `token_budget`, and it is exactly what the run would have said.
#[derive(Debug, Clone)]
pub struct PromptPreview {
    /// The item previewed. The sub-tab drops a reply whose item is not its own.
    pub item: ItemId,
    /// The project's template names, deduplicated, minus the two reserved ones: the picker's list.
    pub available: Vec<String>,
    /// The row rendered, pinned to its version; `None` when `available` is empty.
    pub template: Option<TemplateRef>,
    /// The assembled prompt, or `AssembleError::to_string()`.
    pub outcome: Result<AssembledPrompt, String>,
}

/// The one sentence an offline preview is refused with (plan D109).
///
/// A function rather than a re-export so the refusal has one name in this crate and a test can
/// assert the sentence without reaching for the arm that produces it.
#[must_use]
pub fn offline_refusal() -> &'static str {
    htui_store::PROMPT_ON_SERVER_ONLY
}

/// Plan D103's spec builder and the `assemble()` that follows it. **Never writes.**
///
/// Blueprint D.4's table, read top to bottom: the item, its project, its kind, the project's
/// templates, `app_setting`, the documents, the upstream walk, the box and the skills. Eight reads,
/// every one of them the same read MOD-4 will make, and three fields filled by [`STAND_INS`]
/// instead of by a `run_step`.
///
/// Returns a whole [`PromptPreview`] rather than a bare [`PromptSpec`] because both halves of the
/// answer are things the pane renders: a project with no template row is not an error, and neither
/// is a refusal. Only a **store** failure is — and that is what the `Err` arm carries.
///
/// # Errors
///
/// Whatever the backend's reads report, including [`StoreError::Unreachable`] with
/// [`offline_refusal`] on an offline arm, and [`StoreError::NotFound`] when the item, its project
/// or its kind is not in this backend.
pub async fn build(
    backend: &Backend,
    item: ItemId,
    template_name: Option<&str>,
    scope: &Scope,
) -> StoreResult<PromptPreview> {
    let row = backend
        .item(item)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "item",
            id: item.to_string(),
        })?;
    let project = backend
        .project(row.project_id)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "project",
            id: row.project_id.to_string(),
        })?;
    let kind = backend
        .item_kind(row.kind_id)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "item_kind",
            id: row.kind_id.to_string(),
        })?;

    let templates = backend.prompt_templates(row.project_id).await?;
    let available = offered(&templates);
    let Some(chosen) = choose(&templates, template_name, &available) else {
        return Ok(PromptPreview {
            item,
            available,
            template: None,
            outcome: Err("this project has no prompt template to preview".to_owned()),
        });
    };
    let pinned = TemplateRef {
        name: chosen.name.clone(),
        version: chosen.version,
    };

    // The settings chain, once, so `budget_source` is decided inside `resolve_budget` and nowhere
    // else — an offline preview would then *say* `app_setting_default` rather than look identical
    // to a project-configured one (plan D101).
    let app = backend.app_settings().await?;
    let mut notes: Vec<String> = STAND_INS.iter().map(|note| (*note).to_owned()).collect();
    let budget = settings::resolve_budget(None, Some(&project.settings), &app);
    let hops = settings::resolve_hops(Some(&project.settings), &app, &mut notes);
    let max_skill_tokens = settings::resolve_max_skill_tokens(&app);
    // The caps are recorded verbatim in the audit even though no pass ran: "the caps the selection
    // would have run under" is what makes `selected: 0` legible rather than ambiguous.
    let (caps, _scan, _deadline) = settings::resolve_excerpt_caps(&app);

    let documents = backend
        .documents_of_kinds(item, &[])
        .await?
        .into_iter()
        .filter(|document| document.kind != SUMMARY_KIND)
        .map(|document| InputDocument {
            kind: document.kind,
            version: document.version,
            body: document.body,
        })
        .collect();
    let upstream = backend
        .upstream_summaries(item, hops, &PromptScope::from_scope(scope, row.project_id))
        .await?;
    let box_profile = match backend.box_info().await? {
        Some(info) => {
            backend
                .box_profile(info.box_id)
                .await?
                .ok_or_else(|| StoreError::NotFound {
                    entity: "box",
                    id: info.box_id.to_string(),
                })?
        }
        None => {
            return Err(StoreError::NotFound {
                entity: "box",
                id: "this box is not registered".to_owned(),
            });
        }
    };
    let skills = backend.bound_skills(row.project_id, None).await?;

    let spec = PromptSpec {
        role: TemplateRole::of_name(&chosen.name),
        template: pinned.clone(),
        body: chosen.body.clone(),
        item_key: format!("{}:{}", project.slug, row.key),
        item_title: row.title.clone(),
        item_kind: kind.name.clone(),
        item_body: row.body.clone(),
        phase: chosen.name.clone(),
        output_kind: Some(chosen.name.clone()),
        attempt: 1,
        documents,
        upstream,
        box_profile,
        skills,
        excerpts: empty_excerpts(caps),
        command_queue: false,
        verify_failure: None,
        previous_diff: None,
        judge: None,
        handoff: None,
        budget,
        max_skill_tokens,
        estimator: TokenEstimator::DEFAULT,
        notes,
    };

    Ok(PromptPreview {
        item,
        available,
        template: Some(pinned),
        // No session secrets to mask, and still fail-closed on the prefix rules (`R-SEC-3`): the
        // preview shows the screen exactly what a run would send, never less masked.
        outcome: assemble(&spec, &MinimalScrubber::new([])).map_err(|error| error.to_string()),
    })
}

/// §4.5's audit for a pass that never ran (plan D103, D110; blueprint D.5).
///
/// **`select` is deliberately not called.** The preview resolves no root — `run_step_tree` does not
/// exist yet and `repo_box_path` has no writer — and with no root there is no reader work to do, so
/// calling the ranker would only be a longer way of writing this. `roots` is empty rather than one
/// `RootRecord { source: NoPath }` per repo because plan D110 reads no `repo` row in this
/// milestone; [`EXCERPTS_NOTE`] is where that is said, and `excerpt::tests::
/// no_roots_means_no_section_and_a_no_path_root_per_repo` is where the per-repo half is proved.
fn empty_excerpts(caps: htui_core::prompt::excerpt::ExcerptCaps) -> ExcerptSet {
    ExcerptSet {
        files: Vec::new(),
        audit: ExcerptAudit {
            provider_set: vec![BUILTIN_ID.to_owned()],
            roots: Vec::new(),
            considered: 0,
            selected: 0,
            caps,
            files: Vec::new(),
        },
        notes: Vec::new(),
    }
}

/// The picker's list: every distinct template name the project has, minus the two reserved ones,
/// in byte order.
fn offered(templates: &[htui_core::model::PromptTemplate]) -> Vec<String> {
    let mut names: Vec<String> = templates
        .iter()
        .map(|row| row.name.clone())
        .filter(|name| !RESERVED.contains(&name.as_str()))
        .collect();
    names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    names.dedup();
    names
}

/// The latest version of `wanted`, or of the default name when `wanted` is absent or unknown.
///
/// The default is the first of [`DEFAULT_TEMPLATES`]'s names the project actually has (`prd` for
/// every seeded project, blueprint D.4), falling back to the first offered name so a project whose
/// templates htui never shipped still previews.
fn choose<'a>(
    templates: &'a [htui_core::model::PromptTemplate],
    wanted: Option<&str>,
    available: &[String],
) -> Option<&'a htui_core::model::PromptTemplate> {
    let name = wanted
        .filter(|name| available.iter().any(|offered| offered == name))
        .map(str::to_owned)
        .or_else(|| {
            DEFAULT_TEMPLATES
                .iter()
                .map(|(name, ..)| *name)
                .find(|name| available.iter().any(|offered| offered == name))
                .map(str::to_owned)
        })
        .or_else(|| available.first().cloned())?;
    templates
        .iter()
        .filter(|row| row.name == name)
        .max_by_key(|row| row.version)
}

/// The deferred task: [`build`], then exactly one reply at `addr`, whatever happened.
///
/// Owns its [`Backend`] clone (blueprint E-10): `Writer` is the only other owned handle the tree
/// has and it exposes none of the five inherent prompt reads. A clone that outlives a backend swap
/// reads the arm it was cloned from, fails on it and exits — one-shot, so there is nothing to leak
/// (blueprint H-17).
///
/// A [`StoreError`] becomes [`StoreReply::Failed`] rather than a dropped reply: exactly one reply
/// per request is the contract the whole worker is written to, and a pane waiting forever on a
/// preview that failed is the one outcome worse than a message.
pub async fn run_preview(
    backend: Backend,
    item: ItemId,
    template_name: Option<String>,
    scope: Scope,
    replies: mpsc::UnboundedSender<ReplyEnvelope>,
    addr: ReplyAddr,
) {
    let reply = match build(&backend, item, template_name.as_deref(), &scope).await {
        Ok(preview) => StoreReply::PromptPreview(Box::new(preview)),
        Err(error) => StoreReply::Failed {
            request: "prompt_preview",
            message: error.to_string(),
        },
    };
    // A UI that has gone away is not an error.
    let _ = replies.send(ReplyEnvelope {
        seq: addr.seq,
        origin: addr.origin,
        reply,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::model::PromptTemplate;

    /// Every `prompt_template` row of the demo fixture's first project.
    ///
    /// One project rather than all three: `offered` deduplicates, so the three projects' identical
    /// name sets would hide a duplicate rather than expose one.
    fn demo_templates() -> Vec<PromptTemplate> {
        let data = htui_core::fixtures::demo_data();
        let project = data
            .templates
            .first()
            .expect("the fixture seeds templates")
            .project_id;
        data.templates
            .into_iter()
            .filter(|row| row.project_id == project)
            .collect()
    }

    #[test]
    fn the_stand_in_list_is_the_one_the_notes_are_built_from() {
        // A set, not a bag: a duplicated constant would make the sub-tab render the same sentence
        // twice and would make the `STAND_INS` assertion in `prompt_preview.rs` vacuous.
        let mut sorted = STAND_INS.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "every stand-in is distinct");
        for note in STAND_INS {
            assert!(
                note.starts_with("preview: "),
                "a stand-in says which pass invented it: `{note}`"
            );
        }
    }

    #[test]
    fn the_reserved_names_are_never_offered() {
        let rows = demo_templates();
        let names = offered(&rows);
        assert!(!names.is_empty(), "the demo fixture seeds templates");
        for reserved in RESERVED {
            assert!(
                !names.iter().any(|name| name == reserved),
                "`{reserved}` is MOD-4's, not the picker's"
            );
        }
        let mut sorted = names.clone();
        sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(names, sorted, "byte order, not collation order");
    }

    #[test]
    fn the_default_is_the_first_default_name_the_project_has() {
        let rows = demo_templates();
        let available = offered(&rows);
        let chosen = choose(&rows, None, &available).expect("the fixture has templates");
        assert_eq!(chosen.name, "prd", "blueprint D.4's default");
    }

    #[test]
    fn an_unknown_wanted_name_falls_back_to_the_default() {
        let rows = demo_templates();
        let available = offered(&rows);
        let chosen =
            choose(&rows, Some("no-such-template"), &available).expect("the fixture has templates");
        assert_eq!(
            chosen.name, "prd",
            "a stale picker name must not make the pane empty"
        );
    }

    #[test]
    fn a_reserved_name_is_not_choosable_even_when_asked_for_by_name() {
        let rows = demo_templates();
        let available = offered(&rows);
        let chosen = choose(&rows, Some("judge"), &available).expect("the fixture has templates");
        assert_ne!(
            chosen.name, "judge",
            "plan D107: the reserved names are MOD-4's"
        );
    }

    #[test]
    fn the_empty_audit_registers_the_builtin_and_records_the_caps() {
        let caps = htui_core::prompt::excerpt::ExcerptCaps {
            max_files: 12,
            file_line_cap: 400,
            head_lines: 200,
            max_file_bytes: 262_144,
        };
        let set = empty_excerpts(caps);
        assert!(set.files.is_empty());
        assert!(set.audit.roots.is_empty(), "plan D110");
        assert_eq!(set.audit.provider_set, vec![BUILTIN_ID.to_owned()]);
        assert_eq!(set.audit.caps, caps);
        assert_eq!((set.audit.considered, set.audit.selected), (0, 0));
    }
}
