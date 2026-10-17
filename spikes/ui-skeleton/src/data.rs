//! Mock data. Nothing here is real; it only has to look like htui's.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Open,
    Queued,
    InProgress,
    AwaitingApproval,
    Blocked,
    Failed,
    Done,
    Closed,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::Queued => "queued",
            Status::InProgress => "running",
            Status::AwaitingApproval => "needs you",
            Status::Blocked => "blocked",
            Status::Failed => "failed",
            Status::Done => "done",
            Status::Closed => "closed",
        }
    }
}

pub struct Item {
    pub key: &'static str,
    pub title: &'static str,
    pub project: &'static str,
    pub status: Status,
    pub priority: &'static str,
    pub deps: &'static [&'static str],
    /// Other relationships: (kind, item key). Kinds: "relates to", "follows up", "duplicates".
    pub rels: &'static [(&'static str, &'static str)],
    pub body: &'static str,
}

pub struct Run {
    pub step: &'static str,
    pub status: Status,
    pub agent: &'static str,
    pub took: &'static str,
    pub cost: &'static str,
}

pub struct Requirement {
    /// The requirement this one refines (nesting).
    pub parent: Option<&'static str>,
    pub ws: usize,
    /// None: a workspace-level requirement.
    pub project: Option<&'static str>,
    pub id: &'static str,
    pub area: &'static str,
    pub title: &'static str,
    pub text: &'static str,
    pub status: Status,
    pub items: &'static [&'static str],
}

pub struct Skill {
    pub name: &'static str,
    pub group: &'static str,
    pub desc: &'static str,
    pub attach: &'static str,
    pub body: &'static str,
}

/// A way an item's run can be executed. Wherever it runs, htui prepares it there: the code, the
/// item's components (docker compose), env and secrets, and the agents.
pub struct Target {
    pub name: &'static str,
    pub kind: &'static str,
    pub address: &'static str,
    /// What htui does there to run an item, in order.
    pub how: &'static [&'static str],
    pub status: Status,
    /// Only in the plans: shown, not selectable.
    pub planned: Option<&'static str>,
}

/// One step of a workflow template.
#[derive(Clone, Debug)]
pub struct WfStep {
    pub id: String,
    pub name: String,
    /// Who the step needs (a persona from the Toolbox: planner, coder, reviewer…). Empty: a check.
    pub persona: String,
    /// The agent that plays it; "persona's default" leaves it to the persona.
    pub agent: String,
    /// A fan-out: (part, agent) per parallel agent. Empty: one agent. At run time a planner may
    /// split it differently.
    pub parts: Vec<(String, String)>,
}

/// A link of a workflow template. Loop kinds point back to an earlier step.
#[derive(Clone, Debug)]
pub struct WfLink {
    pub from: String,
    pub to: String,
    pub kind: &'static str,
}

/// The relations a template link can have, and what each means.
pub const LINK_KINDS: [(&str, &str); 7] = [
    ("next", "then the next step"),
    ("approved", "a review passes it on"),
    ("changes requested", "loop: a review sends it back"),
    ("fails", "loop: a failed check sends it back"),
    ("asks you", "waits for your go-ahead"),
    ("if", "taken only when a condition holds (e.g. docs only)"),
    ("retries", "loop: the same step again, up to a cap"),
];

/// Kinds that go back to an earlier step.
pub fn is_loop(kind: &str) -> bool {
    matches!(kind, "changes requested" | "fails" | "retries")
}

/// A template for how an item is executed: steps and the links between them, loops included. A
/// fast model picks one when a run starts; the run unrolls it (round 1, round 2, the parts of a
/// fan-out), and the agent may change it at run time.
pub struct Workflow {
    pub name: &'static str,
    pub desc: &'static str,
    /// What the fast pick looks for.
    pub when: &'static str,
    pub steps: Vec<WfStep>,
    pub links: Vec<WfLink>,
    /// Bumped on every edit, so cached graphs are rebuilt.
    pub version: usize,
    /// Where items using it run, if the workflow pins it (an index into `targets`). Item > workflow
    /// > project > workspace > Settings.
    pub target: Option<usize>,
}

fn st(id: &str, name: &str, agent: &str, parts: &[(&str, &str)]) -> WfStep {
    let persona = match id {
        "plan" | "merge" => "planner",
        "research" => "researcher",
        "code" => "coder",
        "review" => "reviewer",
        "write" => "writer",
        _ => "",
    };
    WfStep { id: id.into(), name: name.into(), persona: persona.into(), agent: agent.into(), parts: parts.iter().map(|(p, a)| (p.to_string(), a.to_string())).collect() }
}

/// The personas a step can ask for (Toolbox → personas), and each one's default agent.
pub const PERSONAS: [(&str, &str); 6] =
    [("planner", "claude · opus"), ("researcher", "claude · sonnet"), ("coder", "claude · sonnet"), ("reviewer", "rust-reviewer"), ("writer", "claude · opus"), ("architect", "claude · opus")];

fn ln(from: &str, to: &str, kind: &'static str) -> WfLink {
    WfLink { from: from.into(), to: to.into(), kind }
}

pub fn workflows() -> Vec<Workflow> {
    vec![
        Workflow {
            name: "standard",
            desc: "Plan, code, review until approved, gate.",
            when: "most items: one area, a clear goal",
            steps: vec![st("plan", "plan", "claude · opus", &[]), st("code", "code", "claude · sonnet", &[]), st("review", "review", "rust-reviewer", &[]), st("gate", "gate", "cargo test", &[])],
            links: vec![
                ln("plan", "code", "next"),
                ln("code", "review", "next"),
                ln("review", "code", "changes requested"),
                ln("review", "gate", "approved"),
                ln("gate", "code", "fails"),
            ],
            version: 0,
            target: None,
        },
        Workflow {
            name: "research fan-out",
            desc: "Plan; agents research one part each in parallel; merge; you confirm; then code and review.",
            when: "items touching several areas, or needing prior art first",
            steps: vec![
                st("plan", "plan", "claude · opus", &[]),
                st("research", "research", "claude · sonnet", &[("API", "claude · sonnet"), ("prior art", "codex · gpt-5"), ("perf", "gemini · pro")]),
                st("merge", "merge findings", "claude · opus", &[]),
                st("code", "code", "claude · sonnet", &[]),
                st("review", "review", "rust-reviewer", &[]),
                st("gate", "gate", "cargo test", &[]),
            ],
            links: vec![
                ln("plan", "research", "next"),
                ln("research", "merge", "next"),
                ln("merge", "code", "asks you"),
                ln("code", "review", "next"),
                ln("review", "code", "changes requested"),
                ln("review", "gate", "approved"),
                ln("gate", "code", "fails"),
            ],
            version: 0,
            target: None,
        },
        Workflow {
            name: "quick fix",
            desc: "Code the fix with a regression test, review, gate.",
            when: "bugs with a known cause, small changes",
            steps: vec![st("code", "code · fix + test", "claude · sonnet", &[]), st("review", "review", "rust-reviewer", &[]), st("gate", "gate", "cargo test", &[])],
            links: vec![ln("code", "review", "next"), ln("review", "code", "changes requested"), ln("review", "gate", "approved"), ln("gate", "code", "retries")],
            version: 0,
            target: None,
        },
        Workflow {
            name: "analysis",
            desc: "Research the question from two sides, write it up, review the write-up. No code.",
            when: "questions: why, compare, should we",
            steps: vec![
                st("plan", "frame the question", "claude · opus", &[]),
                st("research", "research", "claude · sonnet", &[("options", "claude · sonnet"), ("prior art", "codex · gpt-5")]),
                st("write", "write-up", "claude · opus", &[]),
                st("review", "review", "claude · opus", &[]),
            ],
            links: vec![ln("plan", "research", "next"), ln("research", "write", "next"), ln("write", "review", "next"), ln("review", "write", "changes requested")],
            version: 0,
            target: None,
        },
    ]
}

pub fn targets() -> Vec<Target> {
    vec![
        Target {
            name: "this box",
            kind: "local",
            address: "DESKTOP-HTUI",
            how: &[
                "a git worktree of the project on this machine, branch <item>",
                "the item's components started here with docker compose (ports from the workspace .env)",
                "the agent runs as you, with your installed agents",
            ],
            status: Status::Done,
            planned: None,
        },
        Target {
            name: "docker · this box",
            kind: "docker",
            address: "local docker daemon",
            how: &[
                "a fresh clone on branch <item> inside a container, like scripts/hr",
                "its own compose project: a private Postgres, cache… on free ports",
                "the toolchain image; agent logins on a named volume",
                "nothing leaves the container until you collect the branch",
            ],
            status: Status::Done,
            planned: None,
        },
        Target {
            name: "ssh · build-01",
            kind: "remote ssh",
            address: "luigi@build-01.lan",
            how: &[
                "clones the project into ~/htui-runs/<item> on build-01",
                "starts the item's components there with docker compose",
                "env from Infisical, secrets resolved on build-01",
                "the htui agent installs and logs in the agents on build-01 if they are missing (asks you for each login)",
            ],
            status: Status::Done,
            planned: None,
        },
        Target {
            name: "docker · build-01",
            kind: "remote docker",
            address: "docker context build-01 (over ssh)",
            how: &[
                "the docker sandbox, on build-01's docker daemon (the htui agent sets up the docker context)",
                "fresh clone, private compose project, toolchain image, as on this box",
                "the branch comes back over ssh when you collect it",
            ],
            status: Status::AwaitingApproval,
            planned: None,
        },
        Target {
            name: "box · LAPTOP-2",
            kind: "htui server",
            address: "enrolled with the htui server",
            how: &["a box enrolled with a central htui server claims the run and prepares it like this box"],
            status: Status::Queued,
            planned: Some("planned: the htui server is MOD-47, phase 2"),
        },
    ]
}

pub enum Msg {
    You(String),
    Agent(String),
    Tool { name: &'static str, arg: &'static str, ok: bool, took: &'static str },
    Permission { what: &'static str, answer: Option<&'static str> },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// htui's own agent, in every workspace: settings, skills, agents, the catalogue.
    Global,
    /// The workspace's coordinator: always open for messages.
    Workspace,
    /// One project's coordinator: always open for messages.
    Project,
    /// An agent working on an item: view only, unless it asks you something.
    Item(&'static str),
    /// A finished step's transcript: read only.
    Archived(&'static str, &'static str),
}

/// The workspace index of sessions shown in every workspace.
pub const EVERY_WORKSPACE: usize = usize::MAX;

pub struct Session {
    pub ws: usize,
    pub kind: SessionKind,
    pub name: &'static str,
    pub agent: &'static str,
    pub model: &'static str,
    pub project: &'static str,
    pub state: Status,
    pub msgs: Vec<Msg>,
}

#[derive(Clone, Copy)]
pub enum FieldKind {
    /// Enter runs it; the value is what it says.
    Action(&'static str),
    Text,
    Secret,
    Toggle,
    Choice(&'static [&'static str]),
    ReadOnly,
}

pub struct Field {
    pub label: &'static str,
    pub value: String,
    pub kind: FieldKind,
    pub help: &'static str,
}

pub struct Section {
    pub name: &'static str,
    pub group: &'static str,
    pub about: &'static str,
    pub fields: Vec<Field>,
}

pub struct AgentRow {
    pub name: &'static str,
    pub transport: &'static str,
    pub billing: &'static str,
    pub models: &'static str,
    pub state: Status,
    pub version: &'static str,
}

/// One activity event. `target` is the item key or chat name Enter jumps to.
pub struct LogLine {
    pub at: String,
    pub ws: usize,
    pub status: Status,
    pub text: String,
    pub target: Option<&'static str>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub const ALL: [Level; 5] = [Level::Trace, Level::Debug, Level::Info, Level::Warn, Level::Error];
    pub fn name(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

/// One line of htui's own log (the Logs tab).
pub struct LogRec {
    pub at: String,
    pub level: Level,
    pub target: &'static str,
    pub msg: String,
}

pub const LOG_TARGETS: [(&str, &str); 7] = [
    ("htui::ui", "ui"),
    ("htui_store", "store"),
    ("htui_orch", "orchestrator"),
    ("htui_agent", "agents"),
    ("htui_mcp", "mcp"),
    ("htui_worker", "worker"),
    ("htui_secrets", "secrets"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WsState {
    Open,
    /// Not on screen; its runs keep going.
    Background,
    /// Waiting for its runs to finish; a second stop forces it.
    Stopping,
    Stopped,
}

pub struct Workspace {
    pub name: &'static str,
    pub projects: Vec<&'static str>,
    pub opened: &'static str,
    pub path: &'static str,
    pub description: &'static str,
    pub goals: &'static [&'static str],
    pub state: WsState,
    /// Runs a stop parked; resuming the workspace picks them up again.
    pub parked: Vec<Parked>,
}

/// A run a stop put aside.
#[derive(Clone, Debug)]
pub struct Parked {
    pub key: &'static str,
    /// The step it stopped in (forced) or after (graceful).
    pub step: String,
    /// Forced: cut mid-turn; that turn runs again on resume. Graceful: stopped between turns.
    pub interrupted: bool,
    /// The agent can reopen its saved conversation. If not, the step runs again from its start.
    pub resumable: bool,
}

pub struct Project {
    pub name: &'static str,
    pub repo: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Usage {
    /// Uses the workspace's instance.
    Shared,
    /// Runs its own instance on this port.
    Own(u16),
    Unused,
}

impl Usage {
    pub fn label(self) -> String {
        match self {
            Usage::Shared => "workspace's".into(),
            Usage::Own(port) => format!("own · :{port}"),
            Usage::Unused => "not used".into(),
        }
    }
}

/// A service in the workspace's docker compose: a database, a cache, anything with an image.
pub struct Component {
    pub ws: usize,
    pub name: String,
    pub ctype: &'static str,
    pub kind: &'static str,
    pub image: String,
    pub port: u16,
    pub status: Status,
    /// For cloud emulators: the services it emulates.
    pub services: Vec<&'static str>,
    /// How each project of the workspace uses it.
    pub usage: Vec<(&'static str, Usage)>,
}

/// One kind of component the catalogue knows: its image, default port, versions, and (for cloud
/// emulators) the services it can emulate.
pub struct CatKind {
    pub name: &'static str,
    pub image: &'static str,
    pub port: u16,
    pub versions: Vec<&'static str>,
    pub services: Vec<&'static str>,
}

pub struct CatType {
    pub name: &'static str,
    pub kinds: Vec<CatKind>,
}

/// What can be added as a component. In htui this lives in the database, seeded on first start,
/// and is refreshed on request; here it is mock data.
pub struct Catalogue {
    pub types: Vec<CatType>,
    pub updated: String,
}

fn kind(name: &'static str, image: &'static str, port: u16, versions: &[&'static str], services: &[&'static str]) -> CatKind {
    CatKind { name, image, port, versions: versions.to_vec(), services: services.to_vec() }
}

pub fn catalogue() -> Catalogue {
    let t = |name, kinds| CatType { name, kinds };
    Catalogue {
        updated: "3 days ago".into(),
        types: vec![
            t("database", vec![
                kind("postgres", "postgres", 5432, &["17", "16", "15", "14"], &[]),
                kind("mysql", "mysql", 3306, &["8.4", "8.0"], &[]),
                kind("mariadb", "mariadb", 3306, &["11.4", "10.11"], &[]),
                kind("mongodb", "mongo", 27017, &["8.0", "7.0"], &[]),
            ]),
            t("cache", vec![
                kind("redis", "redis", 6379, &["7.4", "7.2"], &[]),
                kind("valkey", "valkey/valkey", 6379, &["8.1", "8.0"], &[]),
                kind("memcached", "memcached", 11211, &["1.6"], &[]),
            ]),
            t("queue", vec![
                kind("rabbitmq", "rabbitmq", 5672, &["4.1-management", "3.13-management"], &[]),
                kind("nats", "nats", 4222, &["2.11", "2.10"], &[]),
                kind("kafka", "apache/kafka", 9092, &["4.0.0", "3.9.0"], &[]),
            ]),
            t("cloud emulator", vec![
                // floci is a family: one container per cloud, each serving every service it has.
                kind("floci", "floci/floci", 4566, &["latest", "nightly"], &["AWS", "Azure", "GCP", "Oracle", "console"]),
                kind("localstack", "localstack/localstack", 4566, &["4.5", "4.4", "latest"], &["s3", "sqs", "sns", "dynamodb", "lambda", "kinesis", "secretsmanager"]),
                kind("elasticmq", "softwaremill/elasticmq-native", 9324, &["1.6"], &["sqs"]),
                kind("azurite", "mcr.microsoft.com/azure-storage/azurite", 10000, &["latest"], &["blob", "queue", "table"]),
                kind("gcp emulators", "gcr.io/google.com/cloudsdktool/google-cloud-cli", 8085, &["emulators"], &["pubsub", "firestore", "bigtable"]),
            ]),
            t("object storage", vec![kind("minio", "minio/minio", 9000, &["latest"], &[])]),
            t("search", vec![
                kind("qdrant", "qdrant/qdrant", 6333, &["v1.15", "v1.14"], &[]),
                kind("meilisearch", "getmeili/meilisearch", 7700, &["v1.15"], &[]),
                kind("elasticsearch", "elasticsearch", 9200, &["8.19.0"], &[]),
            ]),
            t("custom", vec![kind("any image", "ghcr.io/you/service", 8080, &["latest"], &[])]),
        ],
    }
}

pub struct Data {
    pub catalogue: Catalogue,
    pub workspaces: Vec<Workspace>,
    pub projects: Vec<Project>,
    pub components: Vec<Component>,
    pub htui_log: Vec<LogRec>,
    pub items: Vec<Item>,
    pub reqs: Vec<Requirement>,
    pub skills: Vec<Skill>,
    pub workflows: Vec<Workflow>,
    pub targets: Vec<Target>,
    pub sessions: Vec<Session>,
    pub sections: Vec<Section>,
    pub agents: Vec<AgentRow>,
    pub log: Vec<LogLine>,
}

const BODY_DEFAULT: &str = "# Goal
Make the thing described in the title work end to end, with tests.

# Acceptance
- the happy path is covered by an integration test
- errors surface in the UI with a next step, not a code
- `cargo test --workspace` stays green

# Notes
Mock body. In htui this is the item's markdown, rendered in place.";

const BODY_KEYMAP: &str = "# Goal
Every key htui reacts to comes from one keymap the user can rebind in `keys.toml`.

# Acceptance
- the queue and picker keys are rebindable
- a conflicting binding is rejected with the two actions named
- the old `keymap.rs` is gone

# Milestones
- M5 rebindable keys — done
- M6 queue/picker keys, remove the old keymap — in review

# Notes
The hint line reads from the same catalogue, so a rebound key shows its new name.";

pub fn mock() -> Data {
    let items = vec![
        Item { key: "ANA-28", title: "Retention policy for run transcripts", project: "htui", status: Status::Done, priority: "medium", deps: &[], rels: &[], body: BODY_DEFAULT },
        Item { key: "ANA-33", title: "UI audit: btop-style shell and layouts", project: "htui", status: Status::InProgress, priority: "high", deps: &["MOD-80"], rels: &[("follows up", "MOD-67")], body: BODY_DEFAULT },
        Item { key: "MOD-67", title: "Rebindable keymap", project: "htui", status: Status::AwaitingApproval, priority: "high", deps: &["MOD-80", "ANA-21"], rels: &[("relates to", "TOOL-8")], body: BODY_KEYMAP },
        Item { key: "MOD-80", title: "Theme roles and NO_COLOR", project: "htui", status: Status::Done, priority: "medium", deps: &["ANA-21"], rels: &[], body: BODY_DEFAULT },
        Item { key: "ANA-21", title: "Keymap survey across TUIs", project: "htui", status: Status::Done, priority: "low", deps: &[], rels: &[], body: BODY_DEFAULT },
        Item { key: "MOD-107", title: "spawn_subagent MCP tool", project: "htui", status: Status::Queued, priority: "high", deps: &["MOD-67"], rels: &[], body: BODY_DEFAULT },
        Item { key: "MOD-108", title: "Honour retention_days in the sweeper", project: "htui", status: Status::Open, priority: "medium", deps: &["ANA-28"], rels: &[], body: BODY_DEFAULT },
        Item { key: "MOD-109", title: "Multiple Chat tabs", project: "htui", status: Status::Blocked, priority: "medium", deps: &["MOD-107"], rels: &[("relates to", "MOD-67")], body: BODY_DEFAULT },
        Item { key: "CLEAN-11", title: "Drop the legacy markdown importer", project: "htui", status: Status::Failed, priority: "low", deps: &[], rels: &[], body: BODY_DEFAULT },
        Item { key: "CLEAN-12", title: "Split backlog/mod.rs", project: "htui", status: Status::Open, priority: "medium", deps: &[], rels: &[("follows up", "ANA-33")], body: BODY_DEFAULT },
        Item { key: "TOOL-8", title: "Guards, /gate, /next, /validate", project: "htui", status: Status::InProgress, priority: "high", deps: &[], rels: &[], body: BODY_DEFAULT },
        Item { key: "TOOL-9", title: "Gate hooks", project: "htui", status: Status::Closed, priority: "low", deps: &["TOOL-8"], rels: &[], body: BODY_DEFAULT },
        Item { key: "FEAT-1", title: "ACP transport upgrade", project: "agy", status: Status::Open, priority: "high", deps: &[], rels: &[], body: BODY_DEFAULT },
        Item { key: "FIX-1", title: "Session leak on cancel", project: "agy", status: Status::InProgress, priority: "high", deps: &["FEAT-1"], rels: &[], body: BODY_DEFAULT },
        Item { key: "ANA-1", title: "Prompt assembly survey", project: "agy", status: Status::Done, priority: "low", deps: &[], rels: &[], body: BODY_DEFAULT },
        Item { key: "VAL-2", title: "Validate the Windows build", project: "agy", status: Status::Queued, priority: "medium", deps: &["FIX-1"], rels: &[], body: BODY_DEFAULT },
        Item { key: "FEAT-3", title: "Postgres failover", project: "infra", status: Status::InProgress, priority: "high", deps: &["ANA-4"], rels: &[], body: BODY_DEFAULT },
        Item { key: "FIX-2", title: "TLS certificate rotation", project: "infra", status: Status::Failed, priority: "high", deps: &[], rels: &[], body: BODY_DEFAULT },
        Item { key: "ANA-4", title: "Latency budget per endpoint", project: "api", status: Status::Done, priority: "medium", deps: &[], rels: &[], body: BODY_DEFAULT },
        Item { key: "MOD-12", title: "Rate limiting", project: "api", status: Status::Open, priority: "medium", deps: &["ANA-4"], rels: &[], body: BODY_DEFAULT },
    ];

    let workspaces = vec![
        Workspace {
            name: "Graphics",
            projects: vec!["htui", "agy"],
            opened: "now",
            path: "~/projects/graphics",
            description: "The agent tooling: htui and the runtime it drives.",
            goals: &["ship the keymap and the new shell by November", "every run reproducible on any box"],
            state: WsState::Open,
            parked: vec![],
        },
        Workspace {
            name: "Platform",
            projects: vec!["infra", "api"],
            opened: "yesterday",
            path: "~/work/platform",
            description: "Production services and the infrastructure under them.",
            goals: &["zero-downtime deploys", "p99 under 200 ms"],
            state: WsState::Background,
            parked: vec![],
        },
        Workspace {
            name: "Personal",
            projects: vec![],
            opened: "3 weeks ago",
            path: "~/personal",
            description: "Side projects.",
            goals: &[],
            state: WsState::Stopped,
            parked: vec![],
        },
    ];

    let reqs = vec![
        Requirement { parent: None, ws: 0, project: Some("htui"), id: "R-1", area: "Backlog", title: "Items form a dependency graph", text: "Every item may depend on others; the queue never starts an item whose dependencies are not done.", status: Status::Done, items: &["ANA-2", "MOD-4"] },
        Requirement { parent: None, ws: 0, project: Some("htui"), id: "R-2", area: "Backlog", title: "Status is visible at a glance", text: "Every list that shows items shows their status with a glyph and a colour, never only a word.", status: Status::InProgress, items: &["MOD-80", "ANA-33"] },
        Requirement { parent: Some("R-2"), ws: 0, project: Some("htui"), id: "R-2.1", area: "Backlog", title: "A glyph per status", text: "Each status has its own glyph, so it reads without colour.", status: Status::Done, items: &["MOD-80"] },
        Requirement { parent: Some("R-2"), ws: 0, project: Some("htui"), id: "R-2.2", area: "Backlog", title: "Graphs colour nodes by status", text: "The Graph and Runs views colour each node by its status.", status: Status::InProgress, items: &["ANA-33", "MOD-67"] },
        Requirement { parent: Some("R-20"), ws: 0, project: Some("htui"), id: "R-20.1", area: "UI", title: "Conflicts are named", text: "A conflicting binding is rejected with both actions named.", status: Status::AwaitingApproval, items: &["MOD-67"] },
        Requirement { parent: None, ws: 0, project: Some("htui"), id: "R-7", area: "Agents", title: "Spawn subagents over MCP", text: "An agent can start a subagent through the htui MCP server and see its result.", status: Status::Open, items: &["MOD-107"] },
        Requirement { parent: None, ws: 0, project: Some("htui"), id: "R-9", area: "Agents", title: "Quota is shown per agent", text: "The Agents section shows the quota each agent reported, and when it was last refreshed.", status: Status::InProgress, items: &["MOD-41"] },
        Requirement { parent: None, ws: 0, project: Some("htui"), id: "R-12", area: "Chat", title: "Several chats at once", text: "The user can keep more than one chat open and switch between them without losing either.", status: Status::Open, items: &["MOD-109"] },
        Requirement { parent: None, ws: 0, project: Some("htui"), id: "R-13", area: "Chat", title: "Permission requests are answered inline", text: "A permission request appears in the transcript where it happened, with its options as keys.", status: Status::Done, items: &["MOD-2"] },
        Requirement { parent: None, ws: 0, project: Some("htui"), id: "R-20", area: "UI", title: "Every key is rebindable", text: "Every key htui reacts to can be rebound in keys.toml.", status: Status::InProgress, items: &["MOD-67"] },
        Requirement { parent: None, ws: 0, project: Some("htui"), id: "R-21", area: "UI", title: "Each tab explains itself", text: "A toggleable panel says what the tab is for and the common actions.", status: Status::Open, items: &["ANA-33"] },
        Requirement { parent: None, ws: 0, project: None, id: "W-1", area: "Workspace", title: "Every project builds offline", text: "No project in Graphics needs the network to build once its dependencies are cached.", status: Status::InProgress, items: &["VAL-2"] },
        Requirement { parent: None, ws: 0, project: None, id: "W-2", area: "Workspace", title: "Errors reach one tracker", text: "Every project in Graphics reports its errors to the same tracker, tagged with the project.", status: Status::Open, items: &[] },
        Requirement { parent: Some("W-2"), ws: 0, project: Some("htui"), id: "R-40", area: "htui", title: "htui reports panics and failed runs", text: "A panic or a failed run in htui reaches the tracker, tagged htui.", status: Status::Open, items: &[] },
        Requirement { parent: Some("W-2"), ws: 0, project: Some("agy"), id: "R-41", area: "agy", title: "agy reports session crashes", text: "A crashed agent session in agy reaches the tracker, tagged agy.", status: Status::Open, items: &[] },
        Requirement { parent: None, ws: 0, project: Some("agy"), id: "R-3", area: "Agents", title: "Cancelling a session frees it", text: "A cancelled agent session releases its process and its lease within a second.", status: Status::InProgress, items: &["FIX-1"] },
        Requirement { parent: None, ws: 1, project: None, id: "W-1", area: "Workspace", title: "Zero-downtime deploys", text: "Every service in Platform deploys without dropping requests.", status: Status::Open, items: &["FEAT-3"] },
        Requirement { parent: None, ws: 1, project: Some("api"), id: "R-1", area: "API", title: "p99 under 200 ms", text: "Every public endpoint answers within 200 ms at p99 under nominal load.", status: Status::InProgress, items: &["ANA-4", "MOD-12"] },
        Requirement { parent: None, ws: 0, project: Some("htui"), id: "R-30", area: "Storage", title: "Transcripts expire", text: "Run transcripts older than retention_days are deleted by the sweeper.", status: Status::Open, items: &["MOD-108"] },
    ];

    let skills = vec![
        Skill { name: "handoff-run", group: "skills", desc: "Run a HANDOFF item through its full lifecycle.", attach: "**/HANDOFF.md", body: "1. select the item\n2. route: PRD / plan / ANA\n3. confirm with the maintainer\n4. run the chain\n5. bookkeeping + validate" },
        Skill { name: "handoff-add", group: "skills", desc: "Create a HANDOFF item from one line.", attach: "**/HANDOFF.md", body: "Mint the ID, write the checklist line, recount, validate." },
        Skill { name: "plan", group: "skills", desc: "Restate requirements and write a step plan.", attach: "—", body: "Wait for CONFIRM before touching code." },
        Skill { name: "rust-review", group: "skills", desc: "Ownership, lifetimes, error handling.", attach: "**/*.rs", body: "Check unwraps, clones in hot paths, error context." },
        Skill { name: "planner", group: "personas", desc: "Turns an item into a plan of steps. Default agent: claude · opus.", attach: "plan, merge steps", body: "You plan. Restate the goal, list the steps, name the risks." },
        Skill { name: "researcher", group: "personas", desc: "Looks one part up and reports findings. Default: claude · sonnet.", attach: "research steps", body: "You research one part of the item and report what you found, with sources." },
        Skill { name: "coder", group: "personas", desc: "Writes the change and its tests. Default: claude · sonnet.", attach: "code steps", body: "You implement. Small commits, tests first where it helps." },
        Skill { name: "reviewer", group: "personas", desc: "Reviews a diff for correctness first, style last. Default: rust-reviewer.", attach: "review steps", body: "You review changes. Report findings ranked by severity,\neach with a failing scenario. No praise, no nits unless asked." },
        Skill { name: "writer", group: "personas", desc: "Writes analyses and docs. Default: claude · opus.", attach: "write-up steps", body: "You write for the maintainer: plain words, the verdict first." },
        Skill { name: "architect", group: "personas", desc: "Designs before anyone codes: files, interfaces, build order.", attach: "plan steps", body: "You are a software architect. Read the code before proposing.\nAnswer with a blueprint: files, interfaces, data flow, build order." },
        Skill { name: "mod-item", group: "templates", desc: "Template for a MOD item body.", attach: "MOD items", body: "# Goal\n# Acceptance\n# Milestones\n# Notes" },
        Skill { name: "ana-item", group: "templates", desc: "Template for an ANA item body.", attach: "ANA items", body: "# Question\n# Method\n# Findings\n# Verdict" },
        Skill { name: "htui-status", group: "mods", desc: "Claude Code mod: a live panel with the run's step and gate.", attach: "claude only", body: "Plugin mod (Claude Code ≥ 2.1.287): in-process handler.\nPanel: step, gate result, permission count. Other agents ignore it." },
        Skill { name: "permission-toast", group: "mods", desc: "Claude Code mod: a toast when a permission waits.", attach: "claude only", body: "Hook on permission requests; shows a toast in the agent's UI." },
        Skill { name: "htui", group: "MCP servers", desc: "htui's own MCP server: item, run and question tools.", attach: "every run", body: "stdio relay `htui mcp` (HTUI_MCP_ADDR, HTUI_MCP_TOKEN).\nTools: item_get, run_status, ask_user, spawn_subagent (MOD-107)." },
        Skill { name: "gortex", group: "MCP servers", desc: "Code graph queries for agents.", attach: "code steps", body: "search, read, relations, trace, change." },
        Skill { name: "claude", group: "agent help", desc: "How htui drives Claude Code over ACP.", attach: "—", body: "Transport: acp. Billing: subscription.\nPermissions are forwarded to the Chat tab." },
        Skill { name: "codex", group: "agent help", desc: "How htui drives Codex.", attach: "—", body: "Transport: acp. Billing: api key." },
        Skill { name: "gate-on-stop", group: "hooks", desc: "Runs the project's gate when an agent stops.", attach: "code steps", body: "Hook: Stop → scripts/gate.sh; a red gate sends the step back." },
        Skill { name: "no-secrets-in-diff", group: "hooks", desc: "Blocks a write that adds a secret-shaped value.", attach: "every run", body: "Hook: PreToolUse(Write|Edit) → scan the new text; deny on a match." },
        Skill { name: "/gate", group: "commands", desc: "Slash command: run the gate and report.", attach: "claude", body: "Runs scripts/gate.sh, summarises failures." },
        Skill { name: "/next", group: "commands", desc: "Slash command: what to do next on this item.", attach: "claude", body: "Reads the item, the run so far, and proposes the next step." },
        Skill { name: "code-architect", group: "sub-agents", desc: "A sub-agent the main agent can call for a design.", attach: "plan steps", body: "Persona: architect. Tools: read-only code graph. Returns a blueprint." },
        Skill { name: "rust-reviewer", group: "sub-agents", desc: "A sub-agent that reviews Rust diffs.", attach: "review steps", body: "Persona: reviewer. Reads the diff, ranks findings by severity." },
        Skill { name: "rewrite-item", group: "prompts", desc: "The prompt behind the new-item rewrite.", attach: "new item modal", body: "Turn the user's words into an item: kind, title, Goal, Acceptance, Open questions." },
        Skill { name: "fast-pick", group: "prompts", desc: "The prompt the fast pick uses to choose a workflow.", attach: "every run start", body: "Given the item and the workflows' \"when\" lines, answer one workflow and one reason." },
        Skill { name: "rust-ci", group: "env presets", desc: "Environment for Rust runs: toolchain, caches, flags.", attach: "Rust projects", body: "RUSTFLAGS=-D warnings · CARGO_TARGET_DIR shared · sccache on." },
        Skill { name: "gortex", group: "tools", desc: "Code graph: an MCP server, hooks that steer reads, and a daemon.", attach: "code steps", body: "MCP: search, read, relations, trace, change.\nHooks: PreToolUse redirects raw reads.\nDaemon: indexes the tracked repos." },
        Skill { name: "headroom", group: "tools", desc: "Context compression: an MCP server and a proxy.", attach: "long runs", body: "MCP: compress / retrieve.\nProxy: shrinks tool results before the model sees them." },
        Skill { name: "serena", group: "tools", desc: "Symbol-level editing through language servers (MCP).", attach: "code steps", body: "MCP: find_symbol, replace_symbol_body, rename_symbol…" },
        Skill { name: "graphify", group: "tools", desc: "Turns code and docs into a knowledge graph.", attach: "analysis steps", body: "Skill + CLI: builds graphify-out/, then query / path / explain." },
    ];

    let sessions = vec![
        Session {
            ws: EVERY_WORKSPACE,
            kind: SessionKind::Global,
            name: "htui agent",
            agent: "claude",
            model: "opus",
            project: "htui",
            state: Status::Open,
            msgs: vec![
                Msg::You("add a skill for reviewing SQL migrations and attach it to migrations".into()),
                Msg::Tool { name: "skills", arg: "create sql-migration-review", ok: true, took: "0.2s" },
                Msg::Tool { name: "skills", arg: "attach sql-migration-review **/migrations/*.sql", ok: true, took: "0.1s" },
                Msg::Agent("Done: sql-migration-review exists and is attached to **/migrations/*.sql.\nI can also change settings (queue limits, agents), add skills or templates, and refresh the component catalogue.".into()),
            ],
        },
        Session {
            ws: 0,
            kind: SessionKind::Workspace,
            name: "Graphics coordinator",
            agent: "claude",
            model: "opus",
            project: "Graphics",
            state: Status::Open,
            msgs: vec![
                Msg::You("what should I look at first today?".into()),
                Msg::Tool { name: "query", arg: "items needing a person", ok: true, took: "0.1s" },
                Msg::Agent("Two things need you: MOD-67's review asks which conflict message wording you prefer, and CLEAN-11's gate wants permission to delete build/. TOOL-8 and FIX-1 are running fine.".into()),
            ],
        },
        Session {
            ws: 0,
            kind: SessionKind::Project,
            name: "htui coordinator",
            agent: "claude",
            model: "opus",
            project: "htui",
            state: Status::InProgress,
            msgs: vec![
                Msg::You("queue MOD-107 after MOD-67 lands".into()),
                Msg::Agent("Queued. MOD-107 starts when MOD-67's gate passes; I'll tell you here.".into()),
            ],
        },
        Session {
            ws: 0,
            kind: SessionKind::Project,
            name: "agy coordinator",
            agent: "codex",
            model: "gpt-5",
            project: "agy",
            state: Status::Open,
            msgs: vec![Msg::Agent("FIX-1 is implementing. VAL-2 waits for it.".into())],
        },
        Session {
            ws: 0,
            kind: SessionKind::Item("MOD-67"),
            name: "MOD-67 · review",
            agent: "claude",
            model: "opus",
            project: "htui",
            state: Status::AwaitingApproval,
            msgs: vec![
                Msg::Tool { name: "read", arg: "docs/decisions/MOD-67.md", ok: true, took: "0.1s" },
                Msg::Tool { name: "read", arg: "crates/htui/src/keys/validate.rs", ok: true, took: "0.1s" },
                Msg::Agent("Two review comments disagree on the conflict message. Which do you want?\n  1) \"x is already bound to quit\"\n  2) \"x: conflicts with quit (keys.toml:12)\"".into()),
            ],
        },
        Session {
            ws: 0,
            kind: SessionKind::Item("CLEAN-11"),
            name: "CLEAN-11 · gate",
            agent: "claude",
            model: "sonnet",
            project: "htui",
            state: Status::AwaitingApproval,
            msgs: vec![
                Msg::Agent("The gate failed on stale build output. I'll remove it and re-run.".into()),
                Msg::Tool { name: "read", arg: "CMakeLists.txt", ok: true, took: "0.1s" },
                Msg::Permission { what: "execute  rm -rf build", answer: None },
            ],
        },
        Session {
            ws: 0,
            kind: SessionKind::Item("TOOL-8"),
            name: "TOOL-8 · implement",
            agent: "claude",
            model: "sonnet",
            project: "htui",
            state: Status::InProgress,
            msgs: vec![
                Msg::Agent("Step 3/5: wiring /gate into the status line.".into()),
                Msg::Tool { name: "edit", arg: "crates/htui/src/state_cmd.rs", ok: true, took: "0.3s" },
                Msg::Tool { name: "execute", arg: "cargo test -p htui state_cmd", ok: true, took: "38s" },
                Msg::Agent("Tests pass. Moving on to /next.".into()),
            ],
        },
        Session {
            ws: 0,
            kind: SessionKind::Item("FIX-1"),
            name: "FIX-1 · implement",
            agent: "codex",
            model: "gpt-5",
            project: "agy",
            state: Status::InProgress,
            msgs: vec![
                Msg::Tool { name: "search", arg: "\"cancel\" in src/session/", ok: true, took: "0.2s" },
                Msg::Agent("The leak is a missing drop of the lease guard on the cancel path. Writing a test first.".into()),
            ],
        },
        Session {
            ws: 1,
            kind: SessionKind::Workspace,
            name: "Platform coordinator",
            agent: "claude",
            model: "opus",
            project: "Platform",
            state: Status::Open,
            msgs: vec![Msg::Agent("FEAT-3 is running in the background. FIX-2's gate failed overnight.".into())],
        },
        Session { ws: 1, kind: SessionKind::Project, name: "infra coordinator", agent: "claude", model: "sonnet", project: "infra", state: Status::Open, msgs: vec![] },
        Session { ws: 1, kind: SessionKind::Project, name: "api coordinator", agent: "claude", model: "sonnet", project: "api", state: Status::Open, msgs: vec![] },
        Session {
            ws: 1,
            kind: SessionKind::Item("FEAT-3"),
            name: "FEAT-3 · implement",
            agent: "claude",
            model: "sonnet",
            project: "infra",
            state: Status::InProgress,
            msgs: vec![Msg::Agent("Writing the failover script; replica promotion works in the test cluster.".into())],
        },
        Session { ws: 2, kind: SessionKind::Workspace, name: "Personal coordinator", agent: "claude", model: "sonnet", project: "Personal", state: Status::Open, msgs: vec![] },
    ];

    let sections = vec![
        Section { name: "Agents", group: "Agents", about: "The coding agents htui can drive, and whether each is ready on this box.", fields: vec![] },
        Section {
            name: "Personas",
            group: "Agents",
            about: "Named prompt + model presets an agent can run as.",
            fields: vec![
                Field { label: "default persona", value: "implementer".into(), kind: FieldKind::Choice(&["implementer", "reviewer", "architect"]), help: "used when an item names none" },
                Field { label: "import from", value: "~/.claude/agents".into(), kind: FieldKind::Text, help: "a directory of persona files" },
            ],
        },
        Section {
            name: "Prompt",
            group: "Agents",
            about: "What every agent prompt starts with.",
            fields: vec![
                Field { label: "preamble", value: "You are working in htui's sandbox.".into(), kind: FieldKind::Text, help: "prepended to every prompt" },
                Field { label: "include concepts", value: "on".into(), kind: FieldKind::Toggle, help: "attach CONCEPTS.md excerpts" },
            ],
        },
        Section {
            name: "Queue",
            group: "Running",
            about: "How much may run at once, and what it may cost.",
            fields: vec![
                Field { label: "run cap", value: "unbounded".into(), kind: FieldKind::Text, help: "USD per run; empty = unbounded" },
                Field { label: "batch cap", value: "1.50".into(), kind: FieldKind::Text, help: "USD per batch" },
                Field { label: "max concurrent", value: "2".into(), kind: FieldKind::Choice(&["1", "2", "4", "8"]), help: "runs at once on this box" },
                Field { label: "auto queue", value: "off".into(), kind: FieldKind::Toggle, help: "start ready items without asking" },
                Field { label: "window", value: "not set".into(), kind: FieldKind::Text, help: "e.g. 22:00-07:00" },
            ],
        },
        Section {
            name: "Targets",
            group: "Running",
            about: "Where an item can be executed. Each item picks one (t in the Backlog); htui prepares it there.",
            fields: vec![
                Field { label: "this box", value: "local · DESKTOP-HTUI · ● ready".into(), kind: FieldKind::ReadOnly, help: "a worktree here; compose on this machine" },
                Field { label: "docker · this box", value: "docker · local daemon · ● ready".into(), kind: FieldKind::ReadOnly, help: "fresh clone in a container, private compose (like scripts/hr)" },
                Field { label: "ssh · build-01", value: "remote ssh · luigi@build-01.lan · ● ready".into(), kind: FieldKind::ReadOnly, help: "clone and compose on build-01" },
                Field { label: "docker · build-01", value: "remote docker · context build-01 · ⚠ daemon unreachable".into(), kind: FieldKind::ReadOnly, help: "the docker sandbox on build-01's daemon" },
                Field { label: "box · LAPTOP-2", value: "htui server · planned (MOD-47)".into(), kind: FieldKind::ReadOnly, help: "boxes enrolled with a central htui server; phase 2" },
                Field { label: "add a target", value: "ssh host, docker context…".into(), kind: FieldKind::Action("mock: the htui agent asks for the address, probes it, and installs what is missing"), help: "Enter adds one, through the htui agent" },
                Field { label: "default", value: "this box".into(), kind: FieldKind::Choice(&["this box", "docker · this box", "ssh · build-01", "docker · build-01"]), help: "for new items; each item can change it" },
            ],
        },
        Section {
            name: "Workflows",
            group: "Running",
            about: "How items run: the fast pick that chooses a workflow, and what the agent may change on its own.",
            fields: vec![
                Field { label: "fast pick model", value: "claude · haiku".into(), kind: FieldKind::Choice(&["claude · haiku", "claude · sonnet", "codex · gpt-5-mini", "gemini · flash"]), help: "a light model, from the models your agents offer; it picks the workflow when a run starts" },
                Field { label: "run-time changes", value: "ask me".into(), kind: FieldKind::Choice(&["ask me", "automatic"]), help: "the agent may change a run's workflow; ask me shows it as a permission in the item's chat" },
                Field { label: "default workflow", value: "auto (fast pick)".into(), kind: FieldKind::Choice(&["auto (fast pick)", "standard", "research fan-out", "quick fix", "analysis"]), help: "for items that do not pin one" },
            ],
        },
        Section {
            name: "Boxes",
            group: "Running",
            about: "The machines htui has seen, and this one's limits.",
            fields: vec![
                Field { label: "this box", value: "DESKTOP-HTUI".into(), kind: FieldKind::ReadOnly, help: "fingerprinted from the machine id" },
                Field { label: "cpu", value: "16 cores".into(), kind: FieldKind::ReadOnly, help: "" },
                Field { label: "accept runs", value: "on".into(), kind: FieldKind::Toggle, help: "let the queue start runs here" },
            ],
        },
        Section {
            name: "Postgres",
            group: "Storage",
            about: "The Postgres database every box shares.",
            fields: vec![
                Field { label: "status", value: "● connected · 3 ms".into(), kind: FieldKind::ReadOnly, help: "" },
                Field { label: "DSN", value: "postgres://htui@db.local/htui".into(), kind: FieldKind::Secret, help: "stored in the OS keyring" },
                Field { label: "offline buffer", value: "off".into(), kind: FieldKind::Toggle, help: "queue writes while unreachable" },
            ],
        },
        Section {
            name: "Catalogue",
            group: "Storage",
            about: "What can be added as a component: images, versions, emulated services. Stored in the database.",
            fields: vec![
                Field { label: "source", value: "built-in list, copied to the database on first start".into(), kind: FieldKind::ReadOnly, help: "" },
                Field { label: "last updated", value: "3 days ago".into(), kind: FieldKind::ReadOnly, help: "" },
                Field { label: "update now", value: "fetch new kinds and versions".into(), kind: FieldKind::Action("catalogue updated: 3 new versions (postgres 18, kafka 4.1.0, valkey 8.2)"), help: "Enter runs it; the htui agent can do it too" },
            ],
        },
        Section {
            name: "Qdrant",
            group: "Storage",
            about: "The vector index behind concept search.",
            fields: vec![
                Field { label: "url", value: "http://localhost:6334".into(), kind: FieldKind::Text, help: "" },
                Field { label: "collection", value: "htui-concepts".into(), kind: FieldKind::Text, help: "" },
            ],
        },
        Section {
            name: "Secrets",
            group: "Storage",
            about: "Where project secrets come from.",
            fields: vec![
                Field { label: "provider", value: "infisical".into(), kind: FieldKind::Choice(&["none", "infisical", "env file"]), help: "" },
                Field { label: "URL", value: "https://infisical.example.com".into(), kind: FieldKind::Text, help: "" },
                Field { label: "client secret", value: "s3cr3t-value-123".into(), kind: FieldKind::Secret, help: "never shown; storing replaces it" },
            ],
        },
        Section {
            name: "Kinds",
            group: "Backlog",
            about: "Item prefixes (ANA, MOD, …) and what each one means.",
            fields: vec![
                Field { label: "ANA", value: "analysis".into(), kind: FieldKind::Text, help: "" },
                Field { label: "MOD", value: "module / feature".into(), kind: FieldKind::Text, help: "" },
                Field { label: "CLEAN", value: "cleanup".into(), kind: FieldKind::Text, help: "" },
                Field { label: "TOOL", value: "tooling".into(), kind: FieldKind::Text, help: "" },
            ],
        },
    ];

    let agents = vec![
        AgentRow { name: "claude", transport: "acp", billing: "subscription", models: "opus, sonnet, haiku", state: Status::Done, version: "2.1.290" },
        AgentRow { name: "codex", transport: "acp", billing: "api key", models: "gpt-5", state: Status::Done, version: "0.48.0" },
        AgentRow { name: "gemini", transport: "acp", billing: "subscription", models: "—", state: Status::AwaitingApproval, version: "1.4.0" },
        AgentRow { name: "aider", transport: "cli", billing: "api key", models: "—", state: Status::Failed, version: "—" },
    ];

    let ev = |at: &str, ws: usize, status: Status, text: &str, target: Option<&'static str>| LogLine {
        at: at.to_string(),
        ws,
        status,
        text: text.to_string(),
        target,
    };
    let log = vec![
        ev("20:39:50", 1, Status::Failed, "Platform · run FIX-2 · gate failed: cert fixture expired", Some("FIX-2")),
        ev("20:41:03", 0, Status::InProgress, "run MOD-67 · review started (claude · opus)", Some("MOD-67")),
        ev("20:41:40", 0, Status::Done, "run MOD-80 · gate passed (412 tests)", Some("MOD-80")),
        ev("20:42:12", 0, Status::AwaitingApproval, "CLEAN-11 · gate asks: rm -rf build", Some("CLEAN-11 · gate")),
        ev("20:43:05", 0, Status::AwaitingApproval, "MOD-67 · review asks which wording you prefer", Some("MOD-67 · review")),
        ev("20:44:31", 0, Status::InProgress, "run TOOL-8 · implement step 3/5", Some("TOOL-8")),
        ev("20:44:58", 1, Status::InProgress, "Platform · run FEAT-3 · implement step 2/5", Some("FEAT-3")),
        ev("20:45:02", 0, Status::Queued, "MOD-107 queued behind MOD-67", Some("MOD-107")),
        ev("20:46:18", 0, Status::Done, "concepts re-indexed (1 204 chunks)", None),
    ];

    let projects = vec![
        Project { name: "htui", repo: "~/projects/htui", description: "The terminal UI that runs coding agents over a shared backlog." },
        Project { name: "agy", repo: "~/projects/agy", description: "The agent runtime htui drives over ACP." },
        Project { name: "infra", repo: "~/work/platform/infra", description: "Terraform, the database cluster and its failover." },
        Project { name: "api", repo: "~/work/platform/api", description: "The public HTTP API." },
    ];

    let components = vec![
        Component { ws: 0, name: "db".into(), ctype: "database", kind: "postgres", image: "postgres:16".into(), port: 5432, status: Status::InProgress, services: vec![], usage: vec![("htui", Usage::Shared), ("agy", Usage::Own(5433))] },
        Component { ws: 0, name: "vectors".into(), ctype: "search", kind: "qdrant", image: "qdrant/qdrant:v1.15".into(), port: 6333, status: Status::InProgress, services: vec![], usage: vec![("htui", Usage::Shared), ("agy", Usage::Unused)] },
        Component { ws: 1, name: "db".into(), ctype: "database", kind: "postgres", image: "postgres:16".into(), port: 5432, status: Status::InProgress, services: vec![], usage: vec![("infra", Usage::Shared), ("api", Usage::Shared)] },
        Component { ws: 1, name: "cache".into(), ctype: "cache", kind: "redis", image: "redis:7.4".into(), port: 6379, status: Status::Queued, services: vec![], usage: vec![("infra", Usage::Unused), ("api", Usage::Shared)] },
    ];

    let htui_log = (0..60).map(mock_log).collect();

    Data { catalogue: catalogue(), workspaces, projects, components, htui_log, items, reqs, skills, workflows: workflows(), targets: targets(), sessions, sections, agents, log }
}

/// A run history derived from the item's status, so every item has something to show.
pub fn runs(item: &Item) -> Vec<Run> {
    const STEPS: [&str; 5] = ["plan", "architect", "implement", "review", "gate"];
    let (done, current) = match item.status {
        Status::Open | Status::Queued => return Vec::new(),
        Status::InProgress => (2, Some(Status::InProgress)),
        Status::AwaitingApproval => (3, Some(Status::AwaitingApproval)),
        Status::Blocked => (1, Some(Status::Blocked)),
        Status::Failed => (4, Some(Status::Failed)),
        Status::Done | Status::Closed => (5, None),
    };
    let mut runs: Vec<Run> = STEPS[..done]
        .iter()
        .map(|step| Run { step, status: Status::Done, agent: "claude · opus", took: "4m 12s", cost: "$0.38" })
        .collect();
    if let Some(status) = current {
        runs.push(Run { step: STEPS[done.min(4)], status, agent: "claude · opus", took: "1m 03s", cost: "$0.11" });
    }
    runs
}

/// The n-th line of htui's own log: deterministic, so every run looks the same.
pub fn mock_log(n: usize) -> LogRec {
    const MSGS: [(Level, usize, &str); 16] = [
        (Level::Info, 1, "connected to postgres://htui@db.local/htui in 3 ms"),
        (Level::Debug, 0, "frame drawn in 1.2 ms (34 widgets)"),
        (Level::Info, 2, "run MOD-67: step review started on claude/opus"),
        (Level::Debug, 3, "acp: session/update tool_call read docs/decisions/MOD-67.md"),
        (Level::Trace, 0, "key Down → backlog.list.move"),
        (Level::Info, 5, "worker DESKTOP-HTUI heartbeat, 2 runs, 0 queued"),
        (Level::Warn, 3, "gemini: not authenticated, skipping probe"),
        (Level::Debug, 1, "LISTEN run_events: 3 notifications"),
        (Level::Info, 4, "mcp: spawn_subagent called by TOOL-8"),
        (Level::Error, 2, "run CLEAN-11: gate failed (exit 101): 2 tests"),
        (Level::Debug, 6, "infisical: token refreshed, expires in 3600 s"),
        (Level::Info, 2, "queue: MOD-107 waits for MOD-67"),
        (Level::Warn, 1, "slow query 412 ms: SELECT … FROM run_step WHERE …"),
        (Level::Trace, 3, "acp: <- session/update agent_message_chunk (182 bytes)"),
        (Level::Info, 0, "workspace Platform moved to the background (1 run continues)"),
        (Level::Debug, 5, "lease renewed for run TOOL-8 (ttl 30 s)"),
    ];
    let (level, target, msg) = MSGS[n % MSGS.len()];
    let secs = 20 * 3600 + 30 * 60 + n * 7;
    LogRec {
        at: format!("{:02}:{:02}:{:02}.{:03}", secs / 3600, secs / 60 % 60, secs % 60, (n * 137) % 1000),
        level,
        target: LOG_TARGETS[target].0,
        msg: msg.to_string(),
    }
}
