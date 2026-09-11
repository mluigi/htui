//! Demo fixtures (feature `demo`), blueprint §G.
//!
//! Everything here is deterministic: identifiers come from [`demo_uuid`], timestamps from
//! [`demo_at`], and [`demo_data`] is a pure function with no I/O and no randomness. That is what
//! lets the conformance suite (`store::conformance`) assert against a known graph and lets the
//! TUI snapshot tests of MOD-1 compare byte for byte — a wall clock or a random UUID in a fixture
//! row would show up in a rendered frame.
//!
//! The data set is the one `docs/ANA-9.md` §5.10 seeds, filled out with two workspaces, three
//! projects and thirteen items covering all eight `Status` values.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::model::{
    Agent, AppUser, BoxRow, CommandQueue, Document, EventKind, EventRole, Gate, GateOutcome, Item,
    ItemKind, ItemKindId, ItemLink, ItemRevision, LinkKind, Note, NoteId, OsFamily, PhaseId,
    Project, ProjectId, PromptTemplate, PromptTemplateId, Run, RunKind, RunMode, RunStatus,
    RunStep, SessionEvent, Status, StepGraph, StepGraphId, StepGraphPhase, StepId, StepStatus,
    Workspace, WorkspaceProject,
};

/// Milliseconds of `2026-09-03T00:00:00Z`, the timestamp field of every [`demo_uuid`].
pub const DEMO_EPOCH_MS: u64 = 1_788_393_600_000;

/// Seconds of `2026-09-01T00:00:00Z`, the origin [`demo_at`] counts days and hours from.
const DEMO_AT_EPOCH_SECS: i64 = 1_788_220_800;

/// Class code of a `demo_uuid`, one per §5 table. Kept next to [`demo_uuid`] so a new fixture row
/// picks a free code rather than colliding with an existing one.
mod class {
    /// `app_user`.
    pub const USER: u8 = 1;
    /// `box`.
    pub const BOX: u8 = 2;
    /// `workspace`.
    pub const WORKSPACE: u8 = 3;
    /// `project`.
    pub const PROJECT: u8 = 4;
    /// `step_graph`.
    pub const STEP_GRAPH: u8 = 5;
    /// `step_graph_phase`.
    pub const PHASE: u8 = 6;
    /// `item_kind`.
    pub const ITEM_KIND: u8 = 7;
    /// `item`.
    pub const ITEM: u8 = 8;
    /// `item_note`.
    pub const NOTE: u8 = 10;
    /// `document`.
    pub const DOCUMENT: u8 = 11;
    /// `run`.
    pub const RUN: u8 = 12;
    /// `run_step`.
    pub const RUN_STEP: u8 = 13;
    /// `agent`.
    pub const AGENT: u8 = 14;
    /// `prompt_template`.
    pub const PROMPT_TEMPLATE: u8 = 15;
}

/// A v7-shaped, fully deterministic UUID: 48-bit timestamp = [`DEMO_EPOCH_MS`] + `class` * 1000 +
/// `n`, version nibble 7, variant `0b10`, `rand_a` zero, and `rand_b` carrying `(class, n)` so a
/// fixture id is readable in a failure message.
///
/// `demo_uuid(1, 0) == 01a06490-b7e8-7000-8000-000000000100`.
#[must_use]
pub const fn demo_uuid(class: u8, n: u8) -> Uuid {
    let ms = DEMO_EPOCH_MS + (class as u64) * 1000 + n as u64;
    let b = ms.to_be_bytes();
    Uuid::from_bytes([
        b[2], b[3], b[4], b[5], b[6], b[7], 0x70, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, class,
        n,
    ])
}

/// `2026-09-01T00:00:00Z + day * 24h + hour * 1h`, so no fixture row and no snapshot ever
/// contains a wall clock.
///
/// # Panics
///
/// Never for the day and hour values this module uses; the range check is on the sum of seconds.
#[must_use]
pub fn demo_at(day: i64, hour: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(DEMO_AT_EPOCH_SECS + day * 86_400 + hour * 3_600, 0)
        .expect("demo timestamps are inside the representable range")
}

/// Every identifier of the fixture, named as in the blueprint §G tables.
///
/// Phase and prompt-template ids are not listed: they are generated from the per-project formula
/// in [`demo_data`] and nothing outside this module refers to one.
pub mod ids {
    use super::{class, demo_uuid};
    use crate::model::{
        AgentId, BoxId, DocumentId, ItemId, ItemKindId, NoteId, ProjectId, RunId, StepGraphId,
        StepId, UserId, WorkspaceId,
    };

    /// Declares the fixture identifiers of one §5 table.
    macro_rules! demo_ids {
        ($( $(#[$meta:meta])* $name:ident : $ty:ident = ($class:expr, $n:expr) ),* $(,)?) => {
            $(
                $(#[$meta])*
                pub const $name: $ty = $ty::from_uuid(demo_uuid($class, $n));
            )*
        };
    }

    demo_ids!(
        /// `app_user` `luigi`.
        USER: UserId = (class::USER, 0),
        /// `box` `DESKTOP-HTUI`, the box `MemStore::this_box` points at.
        BOX: BoxId = (class::BOX, 0),
        /// `agent` `claude` (`acp`, `subscription`).
        AGENT_CLAUDE: AgentId = (class::AGENT, 0),
        /// `agent` `agy` (`acp`, `subscription` — ANA-4 §5.3; it was `cli`/`per_token` before
        /// MOD-2 corrected the fixture to the seed).
        AGENT_AGY: AgentId = (class::AGENT, 1),
        /// `agent` `claude-cli` (`cli`, `subscription`), MOD-2 D79's third seed row.
        AGENT_CLAUDE_CLI: AgentId = (class::AGENT, 2),
        /// `workspace` `Platform`.
        WORKSPACE_PLATFORM: WorkspaceId = (class::WORKSPACE, 0),
        /// `workspace` `Graphics`.
        WORKSPACE_GRAPHICS: WorkspaceId = (class::WORKSPACE, 1),
        /// `project` `htui`.
        PROJECT_HTUI: ProjectId = (class::PROJECT, 0),
        /// `project` `agy`.
        PROJECT_AGY: ProjectId = (class::PROJECT, 1),
        /// `project` `vulkan-tutorials`.
        PROJECT_VULKAN: ProjectId = (class::PROJECT, 2),
    );

    demo_ids!(
        /// `item_kind` `ANA` of `htui`.
        KIND_HTUI_ANA: ItemKindId = (class::ITEM_KIND, 0),
        /// `item_kind` `FEAT` of `htui`.
        KIND_HTUI_FEAT: ItemKindId = (class::ITEM_KIND, 1),
        /// `item_kind` `FIX` of `htui`.
        KIND_HTUI_FIX: ItemKindId = (class::ITEM_KIND, 2),
        /// `item_kind` `CLEAN` of `htui`.
        KIND_HTUI_CLEAN: ItemKindId = (class::ITEM_KIND, 3),
        /// `item_kind` `TOOL` of `htui`.
        KIND_HTUI_TOOL: ItemKindId = (class::ITEM_KIND, 4),
        /// `item_kind` `ANA` of `agy`.
        KIND_AGY_ANA: ItemKindId = (class::ITEM_KIND, 5),
        /// `item_kind` `FEAT` of `agy`.
        KIND_AGY_FEAT: ItemKindId = (class::ITEM_KIND, 6),
        /// `item_kind` `FIX` of `agy`.
        KIND_AGY_FIX: ItemKindId = (class::ITEM_KIND, 7),
        /// `item_kind` `CLEAN` of `agy`.
        KIND_AGY_CLEAN: ItemKindId = (class::ITEM_KIND, 8),
        /// `item_kind` `TOOL` of `agy`.
        KIND_AGY_TOOL: ItemKindId = (class::ITEM_KIND, 9),
        /// `item_kind` `ANA` of `vulkan-tutorials`.
        KIND_VULKAN_ANA: ItemKindId = (class::ITEM_KIND, 10),
        /// `item_kind` `FEAT` of `vulkan-tutorials`.
        KIND_VULKAN_FEAT: ItemKindId = (class::ITEM_KIND, 11),
        /// `item_kind` `FIX` of `vulkan-tutorials`.
        KIND_VULKAN_FIX: ItemKindId = (class::ITEM_KIND, 12),
        /// `item_kind` `CLEAN` of `vulkan-tutorials`.
        KIND_VULKAN_CLEAN: ItemKindId = (class::ITEM_KIND, 13),
        /// `item_kind` `TOOL` of `vulkan-tutorials`.
        KIND_VULKAN_TOOL: ItemKindId = (class::ITEM_KIND, 14),
    );

    demo_ids!(
        /// Default `step_graph` of `ANA` in `htui`.
        GRAPH_HTUI_ANA: StepGraphId = (class::STEP_GRAPH, 0),
        /// Default `step_graph` of `FEAT` in `htui`.
        GRAPH_HTUI_FEAT: StepGraphId = (class::STEP_GRAPH, 1),
        /// Default `step_graph` of `FIX` in `htui`.
        GRAPH_HTUI_FIX: StepGraphId = (class::STEP_GRAPH, 2),
        /// Default `step_graph` of `CLEAN` in `htui`.
        GRAPH_HTUI_CLEAN: StepGraphId = (class::STEP_GRAPH, 3),
        /// Default `step_graph` of `TOOL` in `htui`.
        GRAPH_HTUI_TOOL: StepGraphId = (class::STEP_GRAPH, 4),
        /// Default `step_graph` of `ANA` in `agy`.
        GRAPH_AGY_ANA: StepGraphId = (class::STEP_GRAPH, 5),
        /// Default `step_graph` of `FEAT` in `agy`.
        GRAPH_AGY_FEAT: StepGraphId = (class::STEP_GRAPH, 6),
        /// Default `step_graph` of `FIX` in `agy`.
        GRAPH_AGY_FIX: StepGraphId = (class::STEP_GRAPH, 7),
        /// Default `step_graph` of `CLEAN` in `agy`.
        GRAPH_AGY_CLEAN: StepGraphId = (class::STEP_GRAPH, 8),
        /// Default `step_graph` of `TOOL` in `agy`.
        GRAPH_AGY_TOOL: StepGraphId = (class::STEP_GRAPH, 9),
        /// Default `step_graph` of `ANA` in `vulkan-tutorials`.
        GRAPH_VULKAN_ANA: StepGraphId = (class::STEP_GRAPH, 10),
        /// Default `step_graph` of `FEAT` in `vulkan-tutorials`.
        GRAPH_VULKAN_FEAT: StepGraphId = (class::STEP_GRAPH, 11),
        /// Default `step_graph` of `FIX` in `vulkan-tutorials`.
        GRAPH_VULKAN_FIX: StepGraphId = (class::STEP_GRAPH, 12),
        /// Default `step_graph` of `CLEAN` in `vulkan-tutorials`.
        GRAPH_VULKAN_CLEAN: StepGraphId = (class::STEP_GRAPH, 13),
        /// Default `step_graph` of `TOOL` in `vulkan-tutorials`.
        GRAPH_VULKAN_TOOL: StepGraphId = (class::STEP_GRAPH, 14),
    );

    demo_ids!(
        /// `htui` `ANA-1`, done.
        HTUI_ANA_1: ItemId = (class::ITEM, 0),
        /// `htui` `ANA-2`, open.
        HTUI_ANA_2: ItemId = (class::ITEM, 1),
        /// `htui` `FEAT-1`, in progress: the item every detail sub-tab has data for.
        HTUI_FEAT_1: ItemId = (class::ITEM, 2),
        /// `htui` `FEAT-2`, blocked by `FEAT-1`.
        HTUI_FEAT_2: ItemId = (class::ITEM, 3),
        /// `htui` `FEAT-3`, queued.
        HTUI_FEAT_3: ItemId = (class::ITEM, 4),
        /// `htui` `FIX-1`, closed.
        HTUI_FIX_1: ItemId = (class::ITEM, 5),
        /// `htui` `TOOL-1`, awaiting approval.
        HTUI_TOOL_1: ItemId = (class::ITEM, 6),
        /// `htui` `CLEAN-1`, failed.
        HTUI_CLEAN_1: ItemId = (class::ITEM, 7),
        /// `agy` `ANA-1`, done.
        AGY_ANA_1: ItemId = (class::ITEM, 8),
        /// `agy` `FEAT-1`, open; links across projects to `htui` `FEAT-2`.
        AGY_FEAT_1: ItemId = (class::ITEM, 9),
        /// `agy` `FIX-1`, open.
        AGY_FIX_1: ItemId = (class::ITEM, 10),
        /// `vulkan-tutorials` `FEAT-1`, in progress.
        VULKAN_FEAT_1: ItemId = (class::ITEM, 11),
        /// `vulkan-tutorials` `TOOL-1`, open.
        VULKAN_TOOL_1: ItemId = (class::ITEM, 12),
    );

    demo_ids!(
        /// First note on `htui` `FEAT-1`.
        NOTE_1: NoteId = (class::NOTE, 0),
        /// Second note on `htui` `FEAT-1`.
        NOTE_2: NoteId = (class::NOTE, 1),
        /// `research` v1 of `htui` `ANA-1`, hand-written.
        DOC_ANA_1_RESEARCH: DocumentId = (class::DOCUMENT, 0),
        /// `verdict` v1 of `htui` `ANA-1`, hand-written.
        DOC_ANA_1_VERDICT: DocumentId = (class::DOCUMENT, 1),
        /// `summary` v1 of `htui` `ANA-1`, hand-written.
        DOC_ANA_1_SUMMARY: DocumentId = (class::DOCUMENT, 2),
        /// `prd` v1 of `htui` `FEAT-1`, hand-written.
        DOC_FEAT_1_PRD: DocumentId = (class::DOCUMENT, 3),
        /// `plan` v1 of `htui` `FEAT-1`, produced by [`STEP_PLAN`].
        DOC_FEAT_1_PLAN_V1: DocumentId = (class::DOCUMENT, 4),
        /// `plan` v2 of `htui` `FEAT-1`, produced by [`STEP_PLAN`].
        DOC_FEAT_1_PLAN_V2: DocumentId = (class::DOCUMENT, 5),
    );

    demo_ids!(
        /// The finished graph run on `htui` `FEAT-1`.
        RUN_1: RunId = (class::RUN, 0),
        /// The queued graph run on `htui` `FEAT-3`: the fixture's only active run.
        RUN_2: RunId = (class::RUN, 1),
        /// `prd` step of [`RUN_1`].
        STEP_PRD: StepId = (class::RUN_STEP, 0),
        /// `plan` step of [`RUN_1`]: the only step with a cached event log.
        STEP_PLAN: StepId = (class::RUN_STEP, 1),
        /// `implement` step of [`RUN_1`].
        STEP_IMPL: StepId = (class::RUN_STEP, 2),
        /// `review` step of [`RUN_1`].
        STEP_REVIEW: StepId = (class::RUN_STEP, 3),
        /// The pending `prd` step of [`RUN_2`].
        STEP_R2_PRD: StepId = (class::RUN_STEP, 4),
    );
}

/// The whole fixture: one collection per §5 table `MemStore`'s state holds.
///
/// A store loads it wholesale (`MemStore::from_demo`); nothing here is derived at load time, so
/// two backends loading the same `DemoData` hold the same rows.
#[derive(Debug, Clone, PartialEq)]
pub struct DemoData {
    /// `app_user` rows.
    pub users: Vec<AppUser>,
    /// `box` rows.
    pub boxes: Vec<BoxRow>,
    /// The box this process runs on, for the top bar.
    pub this_box: Option<crate::model::BoxId>,
    /// `workspace` rows.
    pub workspaces: Vec<Workspace>,
    /// `workspace_project` rows.
    pub workspace_projects: Vec<WorkspaceProject>,
    /// `project` rows.
    pub projects: Vec<Project>,
    /// `item_kind` rows.
    pub kinds: Vec<ItemKind>,
    /// `step_graph` rows.
    pub graphs: Vec<StepGraph>,
    /// `step_graph_phase` rows.
    pub phases: Vec<StepGraphPhase>,
    /// `prompt_template` rows.
    pub templates: Vec<PromptTemplate>,
    /// `agent` rows.
    pub agents: Vec<Agent>,
    /// `item_key_counter` rows, keyed by `(project_id, prefix)` (§4.1).
    pub item_key_counter: HashMap<(ProjectId, String), i32>,
    /// `item` rows.
    pub items: Vec<Item>,
    /// `item_revision` rows: version 1 for every item, so §4.2's ancestor always exists.
    pub revisions: Vec<ItemRevision>,
    /// `item_link` rows, tombstones included.
    pub links: Vec<ItemLink>,
    /// `item_note` rows.
    pub notes: Vec<Note>,
    /// `document` rows, bodies included.
    pub documents: Vec<Document>,
    /// `run` rows.
    pub runs: Vec<Run>,
    /// `run_step` rows.
    pub steps: Vec<RunStep>,
    /// `session_event` rows.
    pub events: Vec<SessionEvent>,
}

/// Builds the fixture. Pure: no I/O, no randomness, no clock.
#[must_use]
pub fn demo_data() -> DemoData {
    let (kinds, graphs, phases, templates) = catalogue();
    DemoData {
        users: users(),
        boxes: boxes(),
        this_box: Some(ids::BOX),
        workspaces: workspaces(),
        workspace_projects: workspace_projects(),
        projects: projects(),
        kinds,
        graphs,
        phases,
        templates,
        agents: agents(),
        item_key_counter: counters(),
        items: items(),
        revisions: revisions(),
        links: links(),
        notes: notes(),
        documents: documents(),
        runs: runs(),
        steps: steps(),
        events: events(),
    }
}

/// The moment every catalogue row was created.
fn epoch() -> DateTime<Utc> {
    demo_at(0, 0)
}

/// `app_user` (§5.2): version one seeds exactly one row.
fn users() -> Vec<AppUser> {
    vec![AppUser {
        id: ids::USER,
        name: "luigi".to_owned(),
        email: Some("luigi@example.invalid".to_owned()),
        created_at: epoch(),
        updated_at: epoch(),
    }]
}

/// `box` (§5.2): the one box the demo runs on.
fn boxes() -> Vec<BoxRow> {
    vec![BoxRow {
        id: ids::BOX,
        user_id: ids::USER,
        hostname: "DESKTOP-HTUI".to_owned(),
        os_family: OsFamily::Windows,
        os_version: "10.0.26200".to_owned(),
        arch: "x86_64".to_owned(),
        cpu: "AMD Ryzen 9 7950X".to_owned(),
        ram_mb: Some(65_536),
        gpu_present: true,
        gpu_vendor: Some("nvidia".to_owned()),
        htui_version: "0.1.0".to_owned(),
        probed_tags: strings(&["rust", "msvc", "cmake"]),
        declared_tags: strings(&["gpu"]),
        quirks: String::new(),
        settings: json!({ "max_concurrent_items": 2 }),
        registered_at: epoch(),
        last_seen_at: demo_at(2, 8),
        last_probed_at: Some(epoch()),
        updated_at: demo_at(2, 8),
    }]
}

/// `agent` (§5.7, §5.10): every agent the seed installs.
///
/// **The real seed rows, re-stamped** — [`seed_rows`](crate::model::agent::seed_rows) with the
/// fixture's deterministic ids and epoch in place of the minted id and wall clock. The fixture
/// used to carry its own hand-written pair (`claude` with `["claude","--acp"]`, `agy` as
/// `cli`/`per_token`/`["agy","run"]`), which was wrong in every field ANA-4 §5.3 settles: the
/// argv shape predates §5.1, and `claude-agent-acp` is the one artifact `htui` must never spawn by
/// name. A demo agent that cannot launch is a trap for MOD-2's own tests, so the fixture is
/// derived from the seed rather than kept beside it.
///
/// # Panics
/// When the id array and the seed no longer have the same length. The pairing used to be a plain
/// `zip`, which stops at the shorter side **silently**: a seed row added by a later milestone
/// simply vanished from the demo registry, and every fixture-based test went on passing over a
/// registry smaller than the one a real box seeds. The length is therefore checked first, so
/// adding a seed without an id fails loudly here rather than quietly everywhere else.
fn agents() -> Vec<Agent> {
    let ids = [ids::AGENT_CLAUDE, ids::AGENT_AGY, ids::AGENT_CLAUDE_CLI];
    let seeds = crate::model::agent::seed_rows(epoch());
    assert_eq!(
        seeds.len(),
        ids.len(),
        "every seed row owns a fixture id, or `zip` would drop it"
    );
    seeds
        .into_iter()
        .zip(ids)
        .map(|(agent, id)| Agent { id, ..agent })
        .collect()
}

/// `workspace` (§5.3).
fn workspaces() -> Vec<Workspace> {
    vec![
        Workspace {
            id: ids::WORKSPACE_PLATFORM,
            slug: "platform".to_owned(),
            name: "Platform".to_owned(),
            description: "The agent platform and its driver".to_owned(),
            created_by: ids::USER,
            created_at: epoch(),
            updated_at: epoch(),
        },
        Workspace {
            id: ids::WORKSPACE_GRAPHICS,
            slug: "graphics".to_owned(),
            name: "Graphics".to_owned(),
            description: "Rendering experiments".to_owned(),
            created_by: ids::USER,
            created_at: epoch(),
            updated_at: epoch(),
        },
    ]
}

/// `workspace_project` (§5.3): membership and order.
fn workspace_projects() -> Vec<WorkspaceProject> {
    vec![
        WorkspaceProject {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_id: ids::PROJECT_HTUI,
            position: 0,
        },
        WorkspaceProject {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_id: ids::PROJECT_AGY,
            position: 1,
        },
        WorkspaceProject {
            workspace_id: ids::WORKSPACE_GRAPHICS,
            project_id: ids::PROJECT_VULKAN,
            position: 0,
        },
    ]
}

/// `(id, slug, name, description)` of the three demo projects, in `demo_uuid` order.
const PROJECT_SPECS: [(ProjectId, &str, &str, &str); 3] = [
    (
        ids::PROJECT_HTUI,
        "htui",
        "htui",
        "The terminal UI and orchestrator",
    ),
    (ids::PROJECT_AGY, "agy", "agy", "The agent driver"),
    (
        ids::PROJECT_VULKAN,
        "vulkan-tutorials",
        "Vulkan Tutorials",
        "Renderer walkthroughs",
    ),
];

/// `project` (§5.3).
fn projects() -> Vec<Project> {
    PROJECT_SPECS
        .iter()
        .map(|(id, slug, name, description)| Project {
            id: *id,
            slug: (*slug).to_owned(),
            name: (*name).to_owned(),
            description: (*description).to_owned(),
            secret_provider: None,
            secret_scope: None,
            settings: json!({
                "token_budget": 120_000,
                "retention_days": null,
                "keep_raw_events": false,
            }),
            created_by: ids::USER,
            created_at: epoch(),
            updated_at: epoch(),
        })
        .collect()
}

/// One of the five `R-ENT-6` kinds, with the phase names of its default graph.
struct KindSpec {
    /// `item_kind.prefix`.
    prefix: &'static str,
    /// `item_kind.name`, also `step_graph.name`.
    name: &'static str,
    /// `item_kind.description`.
    description: &'static str,
    /// `step_graph_phase.name`, in position order.
    phases: &'static [&'static str],
}

/// The `R-ENT-6` kinds every project is seeded with (§5.10).
const KIND_SPECS: [KindSpec; 5] = [
    KindSpec {
        prefix: "ANA",
        name: "analysis",
        description: "A question answered in writing",
        phases: &["research", "verdict"],
    },
    KindSpec {
        prefix: "FEAT",
        name: "feature",
        description: "New behaviour",
        phases: &["prd", "plan", "implement", "review"],
    },
    KindSpec {
        prefix: "FIX",
        name: "bug",
        description: "Behaviour that is wrong",
        phases: &["reproduce", "fix", "review"],
    },
    KindSpec {
        prefix: "CLEAN",
        name: "refactor",
        description: "Behaviour kept, shape improved",
        phases: &["plan", "implement", "review"],
    },
    KindSpec {
        prefix: "TOOL",
        name: "tooling",
        description: "The workshop rather than the product",
        phases: &["plan", "implement", "review"],
    },
];

/// Distinct phase names across the five default graphs, plus the two reserved names: one
/// `prompt_template` v1 each (§5.10 as amended by ANA-5 §5.4; plan D104).
///
/// The order is [`crate::prompt::DEFAULT_TEMPLATES`]'s, because the bodies come from there and the
/// ids are minted from the position.
const TEMPLATE_NAMES: [&str; 10] = [
    "prd",
    "plan",
    "implement",
    "review",
    "research",
    "verdict",
    "reproduce",
    "fix",
    "judge",
    "handoff",
];

/// Kinds, their default graphs, the graphs' phases and one template version per phase name.
///
/// Ids follow blueprint §G: kind and graph `n` are `project_index * 5 + kind_index`, phases are
/// numbered across the whole project (15 per project), templates are
/// `project_index * TEMPLATE_NAMES.len() + template_index`.
///
/// The template stride is [`TEMPLATE_NAMES`]`.len()` and not a literal, because it is a primary
/// key and not a formatting choice: [`demo_uuid`] is a pure function of `(class, n)`, so a stride
/// below the number of templates per project hands project *i*'s first template the id project
/// *i−1*'s late templates already hold, and `load_demo` fails on the second insert. The stride was
/// `8` while there were eight names and had to move with them (blueprint E-5); the same holds for
/// the other two strides if a kind or a phase is ever added.
fn catalogue() -> (
    Vec<ItemKind>,
    Vec<StepGraph>,
    Vec<StepGraphPhase>,
    Vec<PromptTemplate>,
) {
    let mut kinds = Vec::new();
    let mut graphs = Vec::new();
    let mut phases = Vec::new();
    let mut templates = Vec::new();

    for (project_index, (project_id, _, _, _)) in PROJECT_SPECS.iter().enumerate() {
        let project_index = project_index as u8;
        let mut phase_n = project_index * 15;

        for (kind_index, spec) in KIND_SPECS.iter().enumerate() {
            let n = project_index * 5 + kind_index as u8;
            let graph_id = StepGraphId::from_uuid(demo_uuid(class::STEP_GRAPH, n));

            graphs.push(StepGraph {
                id: graph_id,
                project_id: *project_id,
                name: spec.name.to_owned(),
                description: format!("Default graph for {} items", spec.name),
                created_at: epoch(),
                updated_at: epoch(),
            });
            kinds.push(ItemKind {
                id: ItemKindId::from_uuid(demo_uuid(class::ITEM_KIND, n)),
                project_id: *project_id,
                prefix: spec.prefix.to_owned(),
                name: spec.name.to_owned(),
                description: spec.description.to_owned(),
                default_graph_id: graph_id,
                position: kind_index as i32,
                updated_at: epoch(),
            });

            for (position, phase) in spec.phases.iter().enumerate() {
                phases.push(StepGraphPhase {
                    id: PhaseId::from_uuid(demo_uuid(class::PHASE, phase_n)),
                    graph_id,
                    position: position as i32,
                    name: (*phase).to_owned(),
                    fan_out: 1,
                    gate: Gate::Always,
                    gate_hard: false,
                    retry_limit: 1,
                    input_kinds: position
                        .checked_sub(1)
                        .map(|previous| vec![spec.phases[previous].to_owned()])
                        .unwrap_or_default(),
                    output_kind: (*phase).to_owned(),
                    isolation: None,
                    command_queue: CommandQueue::FanOutOnly,
                    verify_command: None,
                    template_name: (*phase).to_owned(),
                    template_version: None,
                    token_budget: None,
                    updated_at: epoch(),
                });
                phase_n += 1;
            }
        }

        for (template_index, name) in TEMPLATE_NAMES.iter().enumerate() {
            templates.push(PromptTemplate {
                id: PromptTemplateId::from_uuid(demo_uuid(
                    class::PROMPT_TEMPLATE,
                    project_index * TEMPLATE_NAMES.len() as u8 + template_index as u8,
                )),
                project_id: *project_id,
                name: (*name).to_owned(),
                version: 1,
                body: crate::prompt::body_of(name)
                    .expect("every TEMPLATE_NAMES entry is a DEFAULT_TEMPLATES name")
                    .to_owned(),
                created_by: ids::USER,
                created_at: epoch(),
                updated_at: epoch(),
            });
        }
    }

    (kinds, graphs, phases, templates)
}

/// `item_key_counter` (§4.1) after the fixture is loaded: the highest number ever minted per
/// `(project, prefix)`, so the next mint continues rather than collides.
fn counters() -> HashMap<(ProjectId, String), i32> {
    [
        (ids::PROJECT_HTUI, "ANA", 2),
        (ids::PROJECT_HTUI, "FEAT", 3),
        (ids::PROJECT_HTUI, "FIX", 1),
        (ids::PROJECT_HTUI, "TOOL", 1),
        (ids::PROJECT_HTUI, "CLEAN", 1),
        (ids::PROJECT_AGY, "ANA", 1),
        (ids::PROJECT_AGY, "FEAT", 1),
        (ids::PROJECT_AGY, "FIX", 1),
        (ids::PROJECT_VULKAN, "FEAT", 1),
        (ids::PROJECT_VULKAN, "TOOL", 1),
    ]
    .into_iter()
    .map(|(project_id, prefix, last_value)| ((project_id, prefix.to_owned()), last_value))
    .collect()
}

/// One row of the blueprint §G item table.
struct ItemSpec {
    /// `demo_uuid(8, n)`.
    n: u8,
    /// `item.project_id`.
    project_id: ProjectId,
    /// `item.kind_id`.
    kind_id: ItemKindId,
    /// `item.key_prefix`.
    prefix: &'static str,
    /// `item.key_number`.
    number: i32,
    /// `item.title`.
    title: &'static str,
    /// `item.status`.
    status: Status,
    /// `item.priority`.
    priority: i16,
    /// `item.required_tags`.
    tags: &'static [&'static str],
    /// `item.body`.
    body: &'static str,
    /// Whether `item.closed_at` is set.
    closed: bool,
}

/// Body of `htui` `ANA-1`: three short paragraphs.
const ANA_1_BODY: &str = "\
The store seam is the one interface every other module reaches through, so it is settled first.

Postgres holds the truth; every box keeps a read-only cache it refreshes on connect. Writes are
compare-and-set on a version column, so a losing edit is shown against its common ancestor rather
than silently dropped.

Item keys are minted from a per-project counter table, which keeps numbers gapless and lets the
legacy importer align the counter above the numbers it brings in.";

/// Body of `htui` `FEAT-1`: long enough that the detail pane has to scroll.
const FEAT_1_BODY: &str = "\
Stand up the terminal application: a workspace-scoped shell with a tab strip, a top bar and a
backlog tab, all reading through the store seam.

Shape
- Two crates: `htui-core` holds the domain model and the store traits, `htui` holds the terminal
  application. Nothing in the view layer holds a store handle.
- A store worker task owns the backend. The UI sends a request, the worker replies, the event
  loop folds the reply into state. Three select arms: terminal events, store replies, a tick.
- Tabs, detail sub-tabs and overlays are trait objects in registries, so a later module adds a
  screen by registering it rather than by editing the loop.

Scope
- Backlog list grouped by project and key prefix, with the five detail sub-tabs: body, runs,
  graph, documents, notes.
- A workspace switcher overlay, opened with `w`.
- The top bar reads workspace, box, store label and the active run count.

Out of scope
- Editing anything. Filters, actions and the item editor are MOD-13.
- Driving an agent; the chat tab is MOD-2.
- Any real database. MOD-1 runs against the in-memory store and its demo fixture.

Done when
- `cargo run -- --demo` opens on the backlog of the Platform workspace.
- The snapshot tests render at 100x30 and every sub-tab has an empty state.
- The terminal is restored on quit, on panic and on an error out of the run loop.";

/// The thirteen items of blueprint §G, in `demo_uuid` order.
const ITEM_SPECS: [ItemSpec; 13] = [
    ItemSpec {
        n: 0,
        project_id: ids::PROJECT_HTUI,
        kind_id: ids::KIND_HTUI_ANA,
        prefix: "ANA",
        number: 1,
        title: "Data model, box registry and sync topology",
        status: Status::Done,
        priority: 0,
        tags: &[],
        body: ANA_1_BODY,
        closed: false,
    },
    ItemSpec {
        n: 1,
        project_id: ids::PROJECT_HTUI,
        kind_id: ids::KIND_HTUI_ANA,
        prefix: "ANA",
        number: 2,
        title: "Orchestrator step graphs and gates",
        status: Status::Open,
        priority: 0,
        tags: &[],
        body: "",
        closed: false,
    },
    ItemSpec {
        n: 2,
        project_id: ids::PROJECT_HTUI,
        kind_id: ids::KIND_HTUI_FEAT,
        prefix: "FEAT",
        number: 1,
        title: "TUI scaffold",
        status: Status::InProgress,
        priority: 2,
        tags: &["rust"],
        body: FEAT_1_BODY,
        closed: false,
    },
    ItemSpec {
        n: 3,
        project_id: ids::PROJECT_HTUI,
        kind_id: ids::KIND_HTUI_FEAT,
        prefix: "FEAT",
        number: 2,
        title: "Agent driver and chat tab",
        status: Status::Blocked,
        priority: 0,
        tags: &["rust"],
        body: "",
        closed: false,
    },
    ItemSpec {
        n: 4,
        project_id: ids::PROJECT_HTUI,
        kind_id: ids::KIND_HTUI_FEAT,
        prefix: "FEAT",
        number: 3,
        title: "Postgres store and cache",
        status: Status::Queued,
        priority: 1,
        tags: &["rust"],
        body: "",
        closed: false,
    },
    ItemSpec {
        n: 5,
        project_id: ids::PROJECT_HTUI,
        kind_id: ids::KIND_HTUI_FIX,
        prefix: "FIX",
        number: 1,
        title: "Terminal left raw after panic",
        status: Status::Closed,
        priority: 0,
        tags: &[],
        body: "",
        closed: true,
    },
    ItemSpec {
        n: 6,
        project_id: ids::PROJECT_HTUI,
        kind_id: ids::KIND_HTUI_TOOL,
        prefix: "TOOL",
        number: 1,
        title: "CI matrix for the three OSes",
        status: Status::AwaitingApproval,
        priority: 0,
        tags: &["docker"],
        body: "",
        closed: false,
    },
    ItemSpec {
        n: 7,
        project_id: ids::PROJECT_HTUI,
        kind_id: ids::KIND_HTUI_CLEAN,
        prefix: "CLEAN",
        number: 1,
        title: "Drop the legacy markdown exporter",
        status: Status::Failed,
        priority: 0,
        tags: &[],
        body: "",
        closed: false,
    },
    ItemSpec {
        n: 8,
        project_id: ids::PROJECT_AGY,
        kind_id: ids::KIND_AGY_ANA,
        prefix: "ANA",
        number: 1,
        title: "Prompt assembly survey",
        status: Status::Done,
        priority: 0,
        tags: &[],
        body: "",
        closed: false,
    },
    ItemSpec {
        n: 9,
        project_id: ids::PROJECT_AGY,
        kind_id: ids::KIND_AGY_FEAT,
        prefix: "FEAT",
        number: 1,
        title: "ACP transport upgrade",
        status: Status::Open,
        priority: 0,
        tags: &["rust"],
        body: "",
        closed: false,
    },
    ItemSpec {
        n: 10,
        project_id: ids::PROJECT_AGY,
        kind_id: ids::KIND_AGY_FIX,
        prefix: "FIX",
        number: 1,
        title: "Session leak on cancel",
        status: Status::Open,
        priority: 0,
        tags: &[],
        body: "",
        closed: false,
    },
    ItemSpec {
        n: 11,
        project_id: ids::PROJECT_VULKAN,
        kind_id: ids::KIND_VULKAN_FEAT,
        prefix: "FEAT",
        number: 1,
        title: "Chapter 12 parity",
        status: Status::InProgress,
        priority: 0,
        tags: &["gpu", "vulkan"],
        body: "",
        closed: false,
    },
    ItemSpec {
        n: 12,
        project_id: ids::PROJECT_VULKAN,
        kind_id: ids::KIND_VULKAN_TOOL,
        prefix: "TOOL",
        number: 1,
        title: "Shader build script",
        status: Status::Open,
        priority: 0,
        tags: &["cmake"],
        body: "",
        closed: false,
    },
];

/// `item` (§5.5). `created_at` is `demo_at(0, n)` and `updated_at` `demo_at(1, n)`, so the rows
/// are distinguishable without a clock.
fn items() -> Vec<Item> {
    ITEM_SPECS
        .iter()
        .map(|spec| Item {
            id: crate::model::ItemId::from_uuid(demo_uuid(class::ITEM, spec.n)),
            project_id: spec.project_id,
            kind_id: spec.kind_id,
            key_prefix: spec.prefix.to_owned(),
            key_number: spec.number,
            key: format!("{}-{}", spec.prefix, spec.number),
            title: spec.title.to_owned(),
            body: spec.body.to_owned(),
            status: spec.status,
            priority: spec.priority,
            required_tags: strings(spec.tags),
            touched_paths: Vec::new(),
            step_graph_id: None,
            version: 1,
            created_by: ids::USER,
            created_at: demo_at(0, i64::from(spec.n)),
            updated_at: demo_at(1, i64::from(spec.n)),
            closed_at: spec.closed.then(|| demo_at(1, i64::from(spec.n))),
        })
        .collect()
}

/// `item_revision` (§5.5): version 1 for every item, `reason = "created"` (§4.2).
fn revisions() -> Vec<ItemRevision> {
    items()
        .into_iter()
        .map(|item| ItemRevision {
            item_id: item.id,
            version: 1,
            title: item.title,
            body: item.body,
            required_tags: item.required_tags,
            author_id: ids::USER,
            box_id: Some(ids::BOX),
            reason: "created".to_owned(),
            created_at: item.created_at,
        })
        .collect()
}

/// `item_link` (§5.5): six live edges and one tombstone.
fn links() -> Vec<ItemLink> {
    let specs: [(crate::model::ItemId, LinkKind, crate::model::ItemId, bool); 7] = [
        (ids::HTUI_FEAT_1, LinkKind::Origin, ids::HTUI_ANA_1, false),
        (
            ids::HTUI_FEAT_2,
            LinkKind::BlockedBy,
            ids::HTUI_FEAT_1,
            false,
        ),
        (ids::HTUI_FEAT_3, LinkKind::Origin, ids::HTUI_ANA_1, false),
        (ids::HTUI_FEAT_3, LinkKind::Relates, ids::HTUI_FEAT_1, false),
        (
            ids::HTUI_CLEAN_1,
            LinkKind::Supersedes,
            ids::HTUI_FIX_1,
            false,
        ),
        (ids::AGY_FEAT_1, LinkKind::Relates, ids::HTUI_FEAT_2, false),
        (ids::HTUI_TOOL_1, LinkKind::Relates, ids::HTUI_FEAT_1, true),
    ];
    specs
        .into_iter()
        .enumerate()
        .map(|(index, (from_item_id, kind, to_item_id, tombstoned))| {
            let created_at = demo_at(2, index as i64);
            let deleted_at = tombstoned.then(|| demo_at(2, 9));
            ItemLink {
                from_item_id,
                to_item_id,
                kind,
                proposed_by_step_id: None,
                created_at,
                updated_at: deleted_at.unwrap_or(created_at),
                deleted_at,
            }
        })
        .collect()
}

/// `item_note` (§5.5): two notes on `htui` `FEAT-1`, ascending.
fn notes() -> Vec<Note> {
    let bodies: [(NoteId, &str, i64); 2] = [
        (
            ids::NOTE_1,
            "Skeleton only — filters, actions and editing are MOD-13.",
            9,
        ),
        (ids::NOTE_2, "Snapshot sizes fixed at 100x30.", 14),
    ];
    bodies
        .into_iter()
        .map(|(id, body, hour)| Note {
            id,
            item_id: ids::HTUI_FEAT_1,
            body: body.to_owned(),
            created_by: ids::USER,
            box_id: Some(ids::BOX),
            via_step_id: None,
            created_at: demo_at(1, hour),
        })
        .collect()
}

/// `document` (§5.5): three hand-written documents on `ANA-1`, and a hand-written `prd` plus two
/// step-produced `plan` versions on `FEAT-1`, so the Documents sub-tab can show both origins.
fn documents() -> Vec<Document> {
    vec![
        document(
            ids::DOC_ANA_1_RESEARCH,
            ids::HTUI_ANA_1,
            "research",
            1,
            "Research: store topology",
            None,
            12,
        ),
        document(
            ids::DOC_ANA_1_VERDICT,
            ids::HTUI_ANA_1,
            "verdict",
            1,
            "Verdict: Postgres with a per-box cache",
            None,
            13,
        ),
        document(
            ids::DOC_ANA_1_SUMMARY,
            ids::HTUI_ANA_1,
            "summary",
            1,
            "Summary: ANA-1",
            None,
            14,
        ),
        document(
            ids::DOC_FEAT_1_PRD,
            ids::HTUI_FEAT_1,
            "prd",
            1,
            "PRD: TUI scaffold",
            None,
            15,
        ),
        document(
            ids::DOC_FEAT_1_PLAN_V1,
            ids::HTUI_FEAT_1,
            "plan",
            1,
            "Plan: TUI scaffold",
            Some(ids::STEP_PLAN),
            16,
        ),
        document(
            ids::DOC_FEAT_1_PLAN_V2,
            ids::HTUI_FEAT_1,
            "plan",
            2,
            "Plan: TUI scaffold (revised)",
            Some(ids::STEP_PLAN),
            17,
        ),
    ]
}

/// One `document` row; the body is a one-line stand-in, MOD-1 never renders it.
fn document(
    id: crate::model::DocumentId,
    item_id: crate::model::ItemId,
    kind: &str,
    version: i32,
    title: &str,
    produced_by_step_id: Option<StepId>,
    hour: i64,
) -> Document {
    Document {
        id,
        item_id,
        kind: kind.to_owned(),
        version,
        title: title.to_owned(),
        body: format!("# {title}\n\nDemo body for the {kind} document, version {version}.\n"),
        produced_by_step_id,
        created_by: ids::USER,
        created_at: demo_at(1, hour),
    }
}

/// `run` (§5.8): one finished graph run and one still-queued run, the fixture's only active one.
fn runs() -> Vec<Run> {
    vec![
        Run {
            id: ids::RUN_1,
            project_id: ids::PROJECT_HTUI,
            item_id: Some(ids::HTUI_FEAT_1),
            kind: RunKind::Graph,
            mode: RunMode::Manual,
            status: RunStatus::Done,
            target_box_id: ids::BOX,
            executing_box_id: Some(ids::BOX),
            graph_snapshot: None,
            started_by: ids::USER,
            queued_at: demo_at(1, 8),
            started_at: Some(demo_at(1, 8)),
            finished_at: Some(demo_at(1, 12)),
            failure: None,
            updated_at: demo_at(1, 12),
        },
        Run {
            id: ids::RUN_2,
            project_id: ids::PROJECT_HTUI,
            item_id: Some(ids::HTUI_FEAT_3),
            kind: RunKind::Graph,
            mode: RunMode::Auto,
            status: RunStatus::Queued,
            target_box_id: ids::BOX,
            executing_box_id: None,
            graph_snapshot: None,
            started_by: ids::USER,
            queued_at: demo_at(2, 8),
            started_at: None,
            finished_at: None,
            failure: None,
            updated_at: demo_at(2, 8),
        },
    ]
}

/// `run_step` (§5.8): the four approved steps of `RUN_1` and the pending step of `RUN_2`.
fn steps() -> Vec<RunStep> {
    let mut steps = vec![
        done_step(ids::STEP_PRD, 0, "prd", ids::AGENT_CLAUDE, "sonnet", 8),
        done_step(ids::STEP_PLAN, 1, "plan", ids::AGENT_CLAUDE, "sonnet", 9),
        done_step(
            ids::STEP_IMPL,
            2,
            "implement",
            ids::AGENT_CLAUDE,
            "opus",
            10,
        ),
        done_step(ids::STEP_REVIEW, 3, "review", ids::AGENT_AGY, "default", 11),
    ];
    if let Some(plan) = steps.get_mut(1) {
        plan.prompt_digest = Some(PROMPT_DIGEST.to_owned());
        plan.usage = Some(json!({
            "input_tokens": 12_000,
            "output_tokens": 2_400,
            "cache_read_tokens": 0,
            "cache_write_tokens": 0,
        }));
    }
    steps.push(RunStep {
        id: ids::STEP_R2_PRD,
        run_id: ids::RUN_2,
        position: 0,
        attempt: 0,
        fanout_index: 0,
        phase_name: "prd".to_owned(),
        agent_id: Some(ids::AGENT_CLAUDE),
        model: Some("sonnet".to_owned()),
        status: StepStatus::Pending,
        gate_outcome: None,
        gate_note: None,
        selected: None,
        exit_code: None,
        prompt_digest: None,
        trim_record: None,
        usage: None,
        isolation_path: None,
        started_at: None,
        finished_at: None,
        updated_at: demo_at(2, 8),
    });
    steps
}

/// One finished, gate-approved step of `RUN_1`, running for an hour from `hour`.
fn done_step(
    id: StepId,
    position: i32,
    phase_name: &str,
    agent_id: crate::model::AgentId,
    model: &str,
    hour: i64,
) -> RunStep {
    RunStep {
        id,
        run_id: ids::RUN_1,
        position,
        attempt: 0,
        fanout_index: 0,
        phase_name: phase_name.to_owned(),
        agent_id: Some(agent_id),
        model: Some(model.to_owned()),
        status: StepStatus::Done,
        gate_outcome: Some(GateOutcome::Approved),
        gate_note: None,
        selected: None,
        exit_code: None,
        prompt_digest: None,
        trim_record: None,
        usage: None,
        isolation_path: None,
        started_at: Some(demo_at(1, hour)),
        finished_at: Some(demo_at(1, hour + 1)),
        updated_at: demo_at(1, hour + 1),
    }
}

/// `run_step.prompt_digest` of [`ids::STEP_PLAN`], also the `digest` key of its `prompt` event: a
/// fixed literal, because a real sha256 of a fixture body would move whenever the body is
/// reworded (§4.3).
const PROMPT_DIGEST: &str = "9f2c1b7e4a08d3556c9e1af0b74d28e63c05a91f7d4b8e2016a3c5d7f908b1e2";

/// The `tool_call_id` pairing seq 3 with seq 4 (§4.3).
const TOOL_CALL_ID: &str = "call_1";

/// `session_event` (§4.3): the replay log of the `plan` step, `turn = 0` throughout.
///
/// MOD-1 renders none of this; it exists so `step_events` has real data and so MOD-2 has a stream
/// to replay. No `raw` payloads: `keep_raw_events` is off for the demo projects.
fn events() -> Vec<SessionEvent> {
    let rows: [(EventKind, EventRole, Option<&str>, Value); 8] = [
        (
            EventKind::Prompt,
            EventRole::Htui,
            None,
            json!({
                "text": "Plan the TUI scaffold against the store seam.",
                "digest": PROMPT_DIGEST,
                "sections": [
                    { "name": "documents:prd", "tokens": 800, "trimmed": false },
                    { "name": "skills", "tokens": 300, "trimmed": false },
                ],
            }),
        ),
        (
            EventKind::AssistantText,
            EventRole::Agent,
            None,
            json!({ "text": "Reading the PRD and the ANA-9 seam." }),
        ),
        (
            EventKind::Thought,
            EventRole::Agent,
            None,
            json!({ "text": "The store trait is the only seam that matters here." }),
        ),
        (
            EventKind::ToolCall,
            EventRole::Agent,
            Some(TOOL_CALL_ID),
            json!({
                "title": "Read docs/ANA-9.md",
                "tool_kind": "read",
                "input": { "path": "docs/ANA-9.md", "offset": 824, "limit": 24 },
                "locations": [{ "path": "docs/ANA-9.md", "line": 824 }],
            }),
        ),
        (
            EventKind::ToolResult,
            EventRole::Agent,
            Some(TOOL_CALL_ID),
            json!({
                "status": "completed",
                "output": "pub trait ReadStore: Send + Sync { ... }",
                "locations": [{ "path": "docs/ANA-9.md", "line": 824 }],
            }),
        ),
        (
            EventKind::Plan,
            EventRole::Agent,
            None,
            json!({
                "entries": [
                    { "content": "Split the crates", "status": "completed", "priority": "high" },
                    { "content": "Fix the store seam", "status": "in_progress", "priority": "high" },
                    { "content": "Snapshot the shell", "status": "pending", "priority": "medium" },
                ],
            }),
        ),
        (
            EventKind::Usage,
            EventRole::Agent,
            None,
            json!({
                "input_tokens": 12_000,
                "output_tokens": 2_400,
                "cache_read_tokens": 0,
                "cache_write_tokens": 0,
                "cost_micros": null,
            }),
        ),
        (
            EventKind::Done,
            EventRole::Agent,
            None,
            json!({ "stop_reason": "end_turn" }),
        ),
    ];

    rows.into_iter()
        .enumerate()
        .map(|(seq, (kind, role, tool_call_id, payload))| SessionEvent {
            run_step_id: ids::STEP_PLAN,
            seq: seq as i32,
            turn: 0,
            kind,
            role,
            tool_call_id: tool_call_id.map(ToOwned::to_owned),
            payload,
            raw: None,
            at: demo_at(1, 9) + chrono::TimeDelta::minutes(seq as i64),
        })
        .collect()
}

/// `&[&str]` to `Vec<String>`, for the array columns of §5.
fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::{DemoData, demo_at, demo_data, demo_uuid, ids};
    use crate::model::Status;
    use std::collections::HashSet;

    #[test]
    fn demo_uuid_matches_the_blueprint() {
        assert_eq!(
            demo_uuid(1, 0).hyphenated().to_string(),
            "01a06490-b7e8-7000-8000-000000000100"
        );
        assert_eq!(
            demo_uuid(8, 3).hyphenated().to_string(),
            "01a06490-d343-7000-8000-000000000803"
        );
        assert_eq!(demo_uuid(4, 2).get_version_num(), 7, "v7 shaped");
    }

    #[test]
    fn demo_at_is_anchored_at_the_first_of_september() {
        assert_eq!(demo_at(0, 0).to_rfc3339(), "2026-09-01T00:00:00+00:00");
        assert_eq!(demo_at(1, 9).to_rfc3339(), "2026-09-02T09:00:00+00:00");
    }

    #[test]
    fn every_id_is_distinct() {
        let data = demo_data();
        let mut seen = HashSet::new();
        let mut push = |uuid: uuid::Uuid| assert!(seen.insert(uuid), "duplicate fixture id {uuid}");
        for row in &data.items {
            push(row.id.as_uuid());
        }
        for row in &data.kinds {
            push(row.id.as_uuid());
        }
        for row in &data.graphs {
            push(row.id.as_uuid());
        }
        for row in &data.phases {
            push(row.id.as_uuid());
        }
        for row in &data.templates {
            push(row.id.as_uuid());
        }
        for row in &data.documents {
            push(row.id.as_uuid());
        }
        for row in &data.notes {
            push(row.id.as_uuid());
        }
        for row in &data.runs {
            push(row.id.as_uuid());
        }
        for row in &data.steps {
            push(row.id.as_uuid());
        }
    }

    /// The demo registry is **every** seed row, one fixture id each.
    ///
    /// [`super::agents`] used to pair the seed rows with a fixed id array by `zip`, which is
    /// silently lossy in whichever direction is shorter: a seed row past the end of the array
    /// vanished, and the demo store would have come up holding a registry smaller than the one a
    /// real box seeds — every fixture-based test then passing over the wrong registry. This is the
    /// guard, and it is why the array and the rows are length-checked before they are paired.
    #[test]
    fn the_demo_registry_carries_one_row_per_seed() {
        let data = demo_data();
        let seeds = crate::model::agent::seed_rows(super::epoch());

        assert_eq!(
            data.agents.len(),
            seeds.len(),
            "the fixture drops no seed row"
        );
        let names: Vec<&str> = data.agents.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["claude", "agy", "claude-cli"]);
        assert_eq!(
            data.agents.iter().map(|row| row.id).collect::<Vec<_>>(),
            [ids::AGENT_CLAUDE, ids::AGENT_AGY, ids::AGENT_CLAUDE_CLI],
            "in seed order, each under its own fixture id"
        );
    }

    #[test]
    fn counters_sit_above_every_minted_number() {
        let data: DemoData = demo_data();
        for item in &data.items {
            let key = (item.project_id, item.key_prefix.clone());
            let last = data.item_key_counter.get(&key).copied().unwrap_or(0);
            assert!(
                last >= item.key_number,
                "counter for {key:?} is {last}, below {}",
                item.key_number
            );
        }
        assert_eq!(
            data.item_key_counter.len(),
            10,
            "one counter row per (project, prefix) in use"
        );
    }

    #[test]
    fn every_status_appears_in_the_htui_project() {
        let data = demo_data();
        for status in Status::ALL {
            assert!(
                data.items
                    .iter()
                    .any(|item| item.project_id == ids::PROJECT_HTUI && item.status == *status),
                "the backlog snapshot renders every status: {status} is missing"
            );
        }
    }

    #[test]
    fn each_project_carries_the_five_seeded_kinds() {
        let data = demo_data();
        for (project_id, ..) in super::PROJECT_SPECS {
            let kinds = data
                .kinds
                .iter()
                .filter(|kind| kind.project_id == project_id)
                .count();
            assert_eq!(kinds, 5, "R-ENT-6 seeds five kinds per project");
        }
        assert_eq!(data.graphs.len(), 15, "one default graph per kind");
        assert_eq!(data.phases.len(), 45, "fifteen phases per project");
        assert_eq!(
            data.templates.len(),
            30,
            "one template version per default template name per project"
        );
    }

    /// The demo corpus is the shipped bodies, not a stand-in: `fixtures.rs` seeds from
    /// [`crate::prompt::DEFAULT_TEMPLATES`] (plan D104).
    ///
    /// Before this, every fixture body was `"You are running the `{name}` phase.\n\n{{item}}\n"` —
    /// one placeholder, so no fixture ever exercised a section and no golden prompt could.
    #[test]
    fn ten_templates_per_project_from_the_default_bodies() {
        let data = demo_data();
        assert_eq!(
            super::TEMPLATE_NAMES.to_vec(),
            crate::prompt::DEFAULT_TEMPLATES
                .iter()
                .map(|(name, ..)| *name)
                .collect::<Vec<_>>(),
            "the fixture's names are §5.4's names, in §5.4's order"
        );

        for (project_id, ..) in super::PROJECT_SPECS {
            let names: Vec<&str> = data
                .templates
                .iter()
                .filter(|template| template.project_id == project_id)
                .map(|template| template.name.as_str())
                .collect();
            assert_eq!(names, super::TEMPLATE_NAMES, "ten per project, in order");
        }

        for template in &data.templates {
            let role = crate::prompt::TemplateRole::of_name(&template.name);
            assert_eq!(
                Some(template.body.as_str()),
                crate::prompt::body_of(&template.name),
                "`{}` is not the shipped body",
                template.name
            );
            crate::prompt::parse(role, &template.body)
                .unwrap_or_else(|error| panic!("`{}` does not parse: {error}", template.name));
            assert_eq!(template.version, 1, "one version each (§5.10)");
        }
    }

    /// `demo_uuid` is a pure function of `(class, n)`, so the per-project stride **is** the id
    /// space: a stride below the number of templates per project makes project *i*'s first
    /// template share a primary key with project *i−1*'s last ones, and `load_demo` fails on the
    /// second insert. At eight names the two numbers agreed by accident; at ten they only agree
    /// because the stride is `TEMPLATE_NAMES.len()` (blueprint E-5).
    #[test]
    fn template_ids_are_distinct_across_projects() {
        let data = demo_data();
        let ids: HashSet<uuid::Uuid> = data
            .templates
            .iter()
            .map(|template| template.id.as_uuid())
            .collect();
        assert_eq!(
            ids.len(),
            data.templates.len(),
            "three projects × ten templates is thirty distinct `prompt_template` ids"
        );
        assert_eq!(ids.len(), 30);
    }

    /// ANA-5 §5.2's `sections[]` names a section by its vocabulary — `documents:<kind>` for a
    /// document block — not by the bare kind. The demo `prompt` payload is the only place in the
    /// tree that spells one out, and MOD-2's assembler now owns the spelling.
    #[test]
    fn the_demo_prompt_payload_uses_the_section_vocabulary() {
        let data = demo_data();
        let prompt = data
            .events
            .iter()
            .find(|event| event.kind == crate::model::EventKind::Prompt)
            .expect("the fixture replay opens with the prompt");
        let names: Vec<&str> = prompt.payload["sections"]
            .as_array()
            .expect("`sections` is an array")
            .iter()
            .map(|section| section["name"].as_str().expect("a name per section"))
            .collect();
        assert_eq!(names, ["documents:prd", "skills"]);
    }
}
