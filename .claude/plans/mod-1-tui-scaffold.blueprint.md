# Blueprint: MOD-1 TUI scaffold

**Binding inputs**: `.claude/plans/mod-1-tui-scaffold.plan.md` (D1-D11, T1-T6, V1-V7),
`.claude/prds/mod-1-tui-scaffold.prd.md`, `docs/ANA-9.md` (authoritative for data shapes and the
store seam), `docs/REQUIREMENTS.md` (`R-TUI-1..3`, `R-NF-1`, `R-NF-3`).

Every decision below traces to a plan decision (`D<n>`), a plan task (`T<n>`), an ANA-9 section
(`§n`) or a requirement. Decisions the plan does not spell out are marked **`[blueprint]`** and
carry their rationale inline. Implementers must not invent anything outside this file; if something
is missing, ask, do not improvise.

Toolchain is already verified (plan V3-V5): rustc 1.98.1, edition 2024, ratatui 0.30.2,
crossterm 0.29 (`event-stream`), tokio 1.53, uuid 1.26 (`v7`), chrono 0.4, serde_json 1,
thiserror 2, clap 4, insta 1.48. Do not re-probe.

---

## A. Crate layout

### A.1 File tree

```
Cargo.toml                      T1  workspace root
rust-toolchain.toml             T1
rustfmt.toml                    T1
clippy.toml                     T1
.gitignore                      T1
README.md                       T6
crates/
  htui-core/
    Cargo.toml                  T1
    src/
      lib.rs                    T1  pub mod model; pub mod store; #[cfg(feature="demo")] pub mod fixtures;
      model/
        mod.rs                  T1  re-exports, `str_enum!` macro, ParseEnumError
        ids.rs                  T1  every ID newtype + `id_newtype!` macro
        user.rs                 T1  AppUser, CapabilityTag
        box_.rs                 T1  BoxRow, BoxTool, BoxInfo, OsFamily
        hierarchy.rs            T1  Workspace, Project, WorkspaceProject, Repo, *BoxPath,
                                    WorkspaceSummary, ProjectRef
        kind.rs                 T1  ItemKind, StepGraph, StepGraphPhase, PhaseAgent,
                                    PromptTemplate, Gate, Isolation, CommandQueue
        agent.rs                T1  [blueprint] Agent, AgentBox, Transport, Billing
        item.rs                 T1  Item, ItemSummary, ItemFilter, NewItem, ItemPatch,
                                    ItemRevision, Status
        link.rs                 T1  ItemLink, LinkKind, LinkGraph, LinkNode, LinkEdge
        note.rs                 T1  Note
        document.rs             T1  Document, DocumentHead
        run.rs                  T1  Run, RunSummary, RunStep, RunStepSummary, RunStepCommit,
                                    RunKind, RunMode, RunStatus, StepStatus, GateOutcome
        event.rs                T1  SessionEvent, EventKind, EventRole
        scope.rs                T1  Scope
      store/
        mod.rs                  T1  re-exports
        error.rs                T1  StoreError, Result<T>
        traits.rs               T1  ReadStore, WriteStore, UpdateOutcome (§6.1 verbatim)
        backend.rs              T1  Backend enum + delegating impl ReadStore + inherent reads
        mem.rs                  T2  MemStore
        conformance.rs          T2  feature `test-support`
      fixtures.rs               T2  feature `demo`
    tests/
      mem_store.rs              T2  runs the conformance suite over MemStore
  htui/
    Cargo.toml                  T3
    src/
      main.rs                   T3  thin binary: args -> lib::run
      lib.rs                    T3  [blueprint] library target, see A.2
      cli.rs                    T3  clap Args
      terminal.rs               T3  TerminalGuard, panic hook
      store_worker.rs           T3  StoreRequest/StoreReply, envelopes, serve(), spawn()
      event_loop.rs             T3  tokio::select! loop
      keymap.rs                 T3  Keymap, Binding, KeyChord, KeyScope
      testkit.rs                T3  [blueprint] Harness for T4/T5 snapshot tests
      app/
        mod.rs                  T3 (T6 edits) App construction + registrations
        state.rs                T3  App fields, Ctx, TopBarState, Origin, Handled
        action.rs               T3  Action, TabAction, OverlayAction
        update.rs               T3  App::update
      ui/
        mod.rs                  T3
        theme.rs                T3
        layout.rs               T3  frame split
        top_bar.rs              T3
        tabs/
          mod.rs                T3 (T6 edits) Tab re-export + default registry builder
          registry.rs           T3  Tab trait, TabId, TabRegistry, tab strip widget
          skills.rs             T3  stub
          settings.rs           T3  stub
          backlog/
            mod.rs              T4  BacklogTab
            list.rs             T4  grouped list pane
            detail/
              mod.rs            T4  DetailTab trait, DetailId, DetailRegistry
              body.rs           T4
              runs.rs           T4
              graph.rs          T4
              documents.rs      T4
              notes.rs          T4
        overlay/
          mod.rs                T3 (T6 edits) Overlay re-export + default registry builder
          registry.rs           T3  Overlay trait, OverlayId, OverlayRegistry, OverlayStack
          workspace_switcher.rs T5  WorkspaceSwitcher
    tests/
      backlog.rs                T4  [blueprint] moved up one level, see A.3
      shell.rs                  T5  [blueprint] moved up one level, see A.3
      snapshots/*.snap          T4/T5  insta output, one prefix per test file
```

Files added beyond the plan's table, with the reason (nothing was removed):

- `crates/htui-core/src/model/agent.rs` — T1 requires the `Transport` and `Billing` enums and
  §5.10 seeds two agents; the Patterns table says one type per file, and folding agents into
  `box_.rs` would mix two §5 sections. **`[blueprint]`**
- `crates/htui/src/lib.rs` — an integration test under `tests/` can only link a **lib** target.
  The plan's T4/T5 snapshot tests live under `tests/`, so the crate must expose a lib. `main.rs`
  stays a ~20-line binary. `Cargo.toml` declares `[lib] name = "htui"` and
  `[[bin]] name = "htui" path = "src/main.rs"`. **`[blueprint]`**
- `crates/htui/src/testkit.rs` — T4 and T5 both need to build an `App`, feed keys, settle store
  replies and render to a `TestBackend`. If each wrote its own harness the two tasks would drift;
  if they shared a file under `tests/` they would collide and break V6. Putting the harness in T3
  keeps T4 ∩ T5 = ∅ and gives both tasks a stable interface to test against. **`[blueprint]`**

### A.2 `crates/htui/src/lib.rs`

```rust
pub mod app;
pub mod cli;
pub mod event_loop;
pub mod keymap;
pub mod store_worker;
pub mod terminal;
pub mod ui;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;

/// Whole application, terminal included. `main` is `Args::parse()` + this + exit-code mapping.
pub async fn run(args: cli::Args) -> anyhow::Result<()>;
```

### A.3 Test target correction **`[blueprint]`**

The plan's table lists `crates/htui/tests/snapshots/backlog.rs`. Cargo only builds `tests/*.rs`
(depth 1) as test targets, so a file at `tests/snapshots/backlog.rs` would never run. The test
targets are therefore `crates/htui/tests/backlog.rs` (T4) and `crates/htui/tests/shell.rs` (T5);
insta's default snapshot directory for a test at `tests/x.rs` is `tests/snapshots/`, so the plan's
`tests/snapshots/*.snap` path is unchanged and V6 (disjoint file sets, per-file snapshot prefixes
`backlog__*.snap` / `shell__*.snap`) still holds.

### A.4 Workspace `Cargo.toml`

```toml
[workspace]
members  = ["crates/htui-core", "crates/htui"]
resolver = "3"

[workspace.package]
edition      = "2024"
rust-version = "1.85"          # MSRV = edition 2024 floor (T1); toolchain pin is separate
license      = "MIT"
publish      = false

[workspace.dependencies]
htui-core          = { path = "crates/htui-core" }
uuid               = { version = "1.26", features = ["v7", "serde"] }
chrono             = { version = "0.4", features = ["serde"] }
serde              = { version = "1", features = ["derive"] }
serde_json         = "1"
thiserror          = "2"
tokio              = { version = "1.53", features = ["rt-multi-thread", "sync", "macros", "time"] }
ratatui            = "0.30.2"
crossterm          = { version = "0.29", features = ["event-stream"] }
futures            = "0.3"     # StreamExt for crossterm::event::EventStream (V4)
clap               = { version = "4", features = ["derive", "env"] }
tracing            = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow             = "1"
insta              = "1.48"

[workspace.lints.rust]
unsafe_code                   = "forbid"
missing_debug_implementations = "warn"
unused_qualifications         = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
# clippy::pedantic is deliberately NOT enabled.
```

Crate manifests: `htui-core` deps = uuid, chrono, serde, serde_json, thiserror; dev-deps = tokio
(`macros`, `rt`); features `default = []`, `demo = []`, `test-support = ["demo"]`.
`htui` deps = htui-core (`features = ["demo"]`), ratatui, crossterm, futures, tokio, clap,
tracing, tracing-subscriber, anyhow; dev-deps = insta, tokio (`test-util`); features
`default = []`, `testkit = []`. Both carry `[lints] workspace = true`.

**Decision: no `[lints.rust] warnings = "deny"` in the manifest; `-D warnings` stays on the
command line.** **`[blueprint]`** Reasons: (1) a manifest-level deny fires during every local
`cargo build`, so a half-written function with a `dead_code` warning fails to compile — hostile to
the TDD loop the plan mandates; (2) it lets any future rustc release break the build on a
newly-added lint, which a pinned repo should absorb deliberately; (3) the plan's Validate section
already runs `cargo clippy --workspace --all-targets --all-features -- -D warnings`, which denies
rustc **and** clippy warnings at the gate — identical guarantee, chosen moment. Consequence for
D2: the `#[allow(async_fn_in_trait)]` on `ReadStore` / `WriteStore` is still mandatory, because
`-D warnings` at the gate would otherwise fail on V3's warning.

### A.5 `rust-toolchain.toml`

```toml
[toolchain]
channel    = "1.98.1"          # exact pin: snapshots and -D warnings must not move under us
components = ["rustfmt", "clippy"]
profile    = "minimal"
```

**`[blueprint]`**: the plan pins the MSRV (1.85) but not the toolchain. An exact channel keeps
`-D warnings` and the insta snapshots reproducible across the maintainer's boxes (`R-NF-1`).
MSRV stays 1.85 in `[workspace.package].rust-version`; `clippy.toml` carries `msrv = "1.85"` so
clippy never suggests a post-1.85 API.

### A.6 `.gitignore` / `rustfmt.toml` / `clippy.toml`

```gitignore
/target
**/*.rs.bk
*.snap.new
/.idea/
/*.log
```

```toml
# rustfmt.toml
edition   = "2024"
max_width = 100
```

```toml
# clippy.toml
msrv = "1.85"
```

---

## B. Core crate interfaces (`htui-core`)

### B.1 §5 column → Rust type mapping rule (D3)

| DDL | Rust | Note |
|---|---|---|
| `UUID` PK / FK | ID newtype (`ItemId`, `ProjectId`, …) | never a bare `Uuid` in a struct field |
| `UUID` nullable FK | `Option<XId>` | |
| `TEXT NOT NULL` | `String` | |
| `TEXT` nullable | `Option<String>` | |
| `TEXT NOT NULL CHECK (… IN …)` | Rust enum, `as_str()` / `FromStr` / `Display` | variant order = CHECK order |
| `TEXT[] NOT NULL` | `Vec<String>` | default `vec![]` |
| `INTEGER` | `i32` | `key_number`, `version`, `position`, `seq`, `turn`, `attempt`, `exit_code` |
| `SMALLINT` | `i16` | `item.priority` only |
| `BOOLEAN` | `bool`; nullable → `Option<bool>` (`run_step.selected`) | |
| `TIMESTAMPTZ NOT NULL` | `chrono::DateTime<chrono::Utc>` | |
| `TIMESTAMPTZ` nullable | `Option<DateTime<Utc>>` | |
| `JSONB NOT NULL` | `serde_json::Value` | |
| `JSONB` nullable | `Option<serde_json::Value>` | |
| generated `key` | `String` field, derived at construction | never settable |

Field names are the §5 column names verbatim (D3). Every struct derives
`Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize`; enums add `Copy, Eq, Hash`.

### B.2 `model/ids.rs`

```rust
macro_rules! id_newtype { ($($name:ident),* $(,)?) => { /* … */ } }

// generated per name:
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemId(pub Uuid);
impl ItemId {
    #[must_use] pub fn new() -> Self { Self(Uuid::now_v7()) }   // §3: client-side UUIDv7
    #[must_use] pub const fn from_uuid(u: Uuid) -> Self { Self(u) }
    #[must_use] pub const fn as_uuid(self) -> Uuid { self.0 }
}
impl fmt::Display for ItemId { /* hyphenated */ }
impl FromStr for ItemId { type Err = uuid::Error; }

id_newtype!(
    UserId, BoxId, WorkspaceId, ProjectId, RepoId, ItemKindId, ItemId, NoteId, DocumentId,
    StepGraphId, PhaseId, PromptTemplateId, AgentId, RunId, StepId, SkillId, CommandRunId,
);
```

`StepId` is the name §6.1 uses for `run_step.id`; keep it.

### B.3 Enums (`str_enum!`, `model/mod.rs`)

```rust
macro_rules! str_enum { ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => { /* … */ } }
```

Generates per enum: `pub const ALL: &'static [Self]`, `pub const fn as_str(self) -> &'static str`,
`impl Display`, `impl FromStr { type Err = ParseEnumError }`, and serde renames driven by the same
literal, so `as_str()` and serde can never disagree.

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` is not a valid {enum_name}")]
pub struct ParseEnumError { pub enum_name: &'static str, pub value: String }
```

Enum set, each in CHECK order (§5, §4.3):

| Enum | Variants (`as_str`) | Source |
|---|---|---|
| `Status` | open, queued, in_progress, awaiting_approval, blocked, done, failed, closed | §5.5 |
| `LinkKind` | blocked_by, origin, relates, supersedes | §5.5 |
| `RunKind` | graph, chat | §5.8 |
| `RunMode` | manual, auto | §5.8 |
| `RunStatus` | queued, running, awaiting_approval, done, failed, cancelled | §5.8 |
| `StepStatus` | pending, running, awaiting_approval, done, failed, cancelled, superseded | §5.8 |
| `GateOutcome` | approved, rejected, retried, skipped | §5.8 |
| `EventKind` | prompt, follow_up, assistant_text, thought, tool_call, tool_result, edit_proposal, permission_request, permission_answer, plan, usage, error, done, other | §4.3 |
| `EventRole` | user, agent, htui | §4.3 |
| `OsFamily` | windows, linux, macos | §5.2 |
| `Transport` | acp, cli | §5.7 |
| `Billing` | subscription, per_token | §5.7 |
| `Gate` | always, on_failure, never | §5.4 |
| `Isolation` | worktree, copy, shared_serialized, local | §5.4 |
| `CommandQueue` | off, fan_out_only, always | §5.4 |

`Status` also gets `pub const fn is_terminal(self) -> bool` (`done | closed`) — used by the
readiness rule (§7.4) and by the blocked/active computations. `RunStatus` gets
`pub const fn is_active(self) -> bool` (`queued | running | awaiting_approval`) for the top bar.

### B.4 Model structs (fields = §5 columns)

Structs whose whole column set is mirrored are marked *(all columns)*: the implementer reads the
DDL for the field list and applies B.1.

```rust
// user.rs
pub struct AppUser { pub id: UserId, pub name: String, pub email: Option<String>,
                     pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc> }
pub struct CapabilityTag { pub tag: String, pub description: String, pub seeded: bool }

// box_.rs   (`Box` is in the prelude, so the row type is `BoxRow`)  [blueprint] naming
pub struct BoxRow { /* all §5.2 `box` columns; probe/settings as serde_json::Value */ }
pub struct BoxTool { pub box_id: BoxId, pub name: String, pub version: String,
                     pub path: String, pub probed_at: DateTime<Utc> }
/// Top-bar projection (`R-TUI-1` "box"), not a table.
pub struct BoxInfo { pub box_id: BoxId, pub hostname: String, pub os_family: OsFamily }

// hierarchy.rs
pub struct Workspace { /* all §5.3 columns */ }
pub struct Project   { /* all §5.3 columns; settings: serde_json::Value */ }
pub struct WorkspaceProject { pub workspace_id: WorkspaceId, pub project_id: ProjectId,
                              pub position: i32 }
pub struct Repo { /* all §5.3 columns */ }
/// Join of workspace_project + project, ordered by position. Not a table.
pub struct ProjectRef { pub project_id: ProjectId, pub slug: String, pub name: String,
                        pub position: i32 }
/// Switcher row; `projects` is ordered by position and is what a `Scope` is built from (D10).
pub struct WorkspaceSummary { pub workspace_id: WorkspaceId, pub slug: String, pub name: String,
                              pub projects: Vec<ProjectRef> }

// kind.rs
pub struct ItemKind { pub id: ItemKindId, pub project_id: ProjectId, pub prefix: String,
                      pub name: String, pub description: String,
                      pub default_graph_id: StepGraphId, pub position: i32,
                      pub updated_at: DateTime<Utc> }
pub struct StepGraph      { /* all §5.4 columns */ }
pub struct StepGraphPhase { /* all §5.4 columns; gate: Gate, isolation: Option<Isolation>,
                               command_queue: CommandQueue */ }
pub struct PhaseAgent { pub phase_id: PhaseId, pub position: i32, pub agent_id: AgentId,
                        pub model: String }
pub struct PromptTemplate { /* all §5.4 columns */ }

// agent.rs
pub struct Agent    { /* all §5.7 columns; transport: Transport, billing: Billing */ }
pub struct AgentBox { /* all §5.7 columns */ }
```

```rust
// item.rs
pub struct Item {
    pub id: ItemId, pub project_id: ProjectId, pub kind_id: ItemKindId,
    pub key_prefix: String, pub key_number: i32, pub key: String,   // generated column, §4.1
    pub title: String, pub body: String, pub status: Status, pub priority: i16,
    pub required_tags: Vec<String>, pub touched_paths: Vec<String>,
    pub step_graph_id: Option<StepGraphId>, pub version: i32,
    pub created_by: UserId, pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}
/// List projection: what a Backlog row and its grouping need, nothing else.
pub struct ItemSummary {
    pub id: ItemId, pub project_id: ProjectId, pub kind_id: ItemKindId, pub key: String,
    pub key_prefix: String, pub key_number: i32, pub title: String, pub status: Status,
    pub priority: i16, pub required_tags: Vec<String>, pub updated_at: DateTime<Utc>,
}
impl Item { pub fn summary(&self) -> ItemSummary; }

#[derive(Default)]
pub struct ItemFilter {
    pub statuses: Option<Vec<Status>>,       // None = any
    pub project_ids: Option<Vec<ProjectId>>, // None = every project in the Scope
    pub tags: Option<Vec<String>>,           // item.required_tags contains all of these
    pub ready: Option<bool>,                 // §7.4, see below
    pub text: Option<String>,                // case-insensitive substring on key + title
}

pub struct NewItem {
    pub id: ItemId,                          // minted by the caller, §3
    pub project_id: ProjectId, pub kind_id: ItemKindId,
    pub title: String, pub body: String,
    pub required_tags: Vec<String>, pub touched_paths: Vec<String>,
    pub priority: i16, pub step_graph_id: Option<StepGraphId>,
    pub created_by: UserId, pub box_id: Option<BoxId>,
}
// No key_prefix / key_number: §4.1 copies the prefix from the kind at mint time and the counter
// supplies the number. The importer variant (explicit number) belongs to MOD-8.

#[derive(Default)]
pub struct ItemPatch {                       // exactly the §4.2 version-covered columns
    pub title: Option<String>, pub body: Option<String>, pub kind_id: Option<ItemKindId>,
    pub required_tags: Option<Vec<String>>, pub priority: Option<i16>,
    pub touched_paths: Option<Vec<String>>, pub step_graph_id: Option<Option<StepGraphId>>,
    pub author_id: UserId, pub box_id: Option<BoxId>, pub reason: String,
}

pub struct ItemRevision {                    // all §5.5 item_revision columns
    pub item_id: ItemId, pub version: i32, pub title: String, pub body: String,
    pub required_tags: Vec<String>, pub author_id: UserId, pub box_id: Option<BoxId>,
    pub reason: String, pub created_at: DateTime<Utc>,
}
```

```rust
// link.rs
pub struct ItemLink { pub from_item_id: ItemId, pub to_item_id: ItemId, pub kind: LinkKind,
                      pub proposed_by_step_id: Option<StepId>, pub created_at: DateTime<Utc>,
                      pub updated_at: DateTime<Utc>, pub deleted_at: Option<DateTime<Utc>> }
pub struct LinkNode { pub item_id: ItemId, pub project_id: ProjectId, pub project_slug: String,
                      pub key: String, pub title: String, pub status: Status, pub depth: u8 }
pub struct LinkEdge { pub from_item_id: ItemId, pub to_item_id: ItemId, pub kind: LinkKind }
pub struct LinkGraph { pub root: ItemId, pub nodes: Vec<LinkNode>, pub edges: Vec<LinkEdge> }
impl LinkGraph { pub fn node(&self, id: ItemId) -> Option<&LinkNode>; }

// note.rs / document.rs
pub struct Note         { /* all §5.5 item_note columns */ }
pub struct Document     { /* all §5.5 document columns, body included */ }
pub struct DocumentHead { pub id: DocumentId, pub item_id: ItemId, pub kind: String,
                          pub version: i32, pub title: String,
                          pub produced_by_step_id: Option<StepId>, pub created_by: UserId,
                          pub created_at: DateTime<Utc> }        // document minus `body`

// run.rs
pub struct Run            { /* all §5.8 run columns */ }
pub struct RunStep        { /* all §5.8 run_step columns */ }
pub struct RunStepCommit  { /* all §5.8 columns */ }
pub struct RunStepSummary { pub id: StepId, pub position: i32, pub attempt: i32,
                            pub fanout_index: i32, pub phase_name: String,
                            pub agent_id: Option<AgentId>, pub model: Option<String>,
                            pub status: StepStatus, pub gate_outcome: Option<GateOutcome>,
                            pub started_at: Option<DateTime<Utc>>,
                            pub finished_at: Option<DateTime<Utc>> }
pub struct RunSummary { pub id: RunId, pub item_id: Option<ItemId>, pub project_id: ProjectId,
                        pub kind: RunKind, pub mode: RunMode, pub status: RunStatus,
                        pub target_box_id: BoxId, pub executing_box_id: Option<BoxId>,
                        pub box_hostname: String,        // joined: the Runs table shows a name
                        pub queued_at: DateTime<Utc>, pub started_at: Option<DateTime<Utc>>,
                        pub finished_at: Option<DateTime<Utc>>, pub failure: Option<String>,
                        pub steps: Vec<RunStepSummary> }

// event.rs
pub struct SessionEvent { pub run_step_id: StepId, pub seq: i32, pub turn: i32,
                          pub kind: EventKind, pub role: EventRole,
                          pub tool_call_id: Option<String>, pub payload: serde_json::Value,
                          pub raw: Option<serde_json::Value>, pub at: DateTime<Utc> }

// scope.rs  (D10 verbatim)
pub struct Scope { pub workspace_id: WorkspaceId, pub project_ids: Vec<ProjectId> }
impl Scope {
    pub fn from_workspace(ws: &WorkspaceSummary) -> Self;   // project_ids ordered by position
    pub fn contains(&self, p: ProjectId) -> bool;
    pub fn is_empty(&self) -> bool;
}
```

`ItemFilter::ready` semantics **`[blueprint]`**: §7.4 combines two predicates. In MOD-1 `ready`
covers only the store-side half — `status == open` **and** no live `blocked_by` edge to an item
whose status is not `done`/`closed`. The capability half (`required_tags <@ box tags`) is expressed
through `ItemFilter::tags` by the caller; MOD-4 owns matching against a real box. Fixed here so
MOD-6's `PgStore` implements the same predicate. `ItemFilter::text` is unused by MOD-1's UI but
belongs to the filter shape MOD-13 needs; defining it now keeps both stores in step.

### B.5 `store/error.rs`

```rust
pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("{entity} `{id}` not found")]
    NotFound { entity: &'static str, id: String },
    #[error("constraint violated: {0}")]
    Constraint(String),           // unknown kind, cross-project kind, unique key, §4.1 no-delete
    #[error("this backend is read-only ({0})")]
    ReadOnly(&'static str),       // MOD-6 Offline; unreachable in MOD-1
    #[error("store backend error: {0}")]
    Backend(String),              // MOD-6 wraps sqlx here
    #[error(transparent)]
    ParseEnum(#[from] crate::model::ParseEnumError),
}
```

`Result<T>` is the `Result<T>` the §6.1 signatures refer to.

### B.6 `store/traits.rs` — §6.1 verbatim

```rust
#[allow(async_fn_in_trait)] // D2 / plan V3: rustc 1.98 warns on async fn in public traits; the
                            // signatures are ANA-9 §6.1 verbatim and `Backend` is concrete, so
                            // Send-ness is inferred at the call site instead of specified.
pub trait ReadStore: Send + Sync {
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> Result<Vec<ItemSummary>>;
    async fn item(&self, id: ItemId) -> Result<Option<Item>>;
    async fn links(&self, id: ItemId, hops: u8) -> Result<LinkGraph>;
    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>>;
    async fn notes(&self, id: ItemId) -> Result<Vec<Note>>;
    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>>;
    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>>; // None = not cached
}

#[allow(async_fn_in_trait)]
pub trait WriteStore: ReadStore {
    async fn mint_item(&self, new: NewItem) -> Result<Item>;
    async fn update_item(&self, id: ItemId, expected_version: i32, patch: ItemPatch) -> Result<UpdateOutcome>;
    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool>;
    // runs, steps, events, links, notes, documents, skills, templates, box, agents ...
}

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateOutcome { Updated(Item), Diverged { head: Item, ancestor: ItemRevision } }
```

Method names, parameter names, order and the `// None = not cached` comment are copied, not
paraphrased (plan V2). Nothing is added to these two traits in MOD-1.

### B.7 `store/backend.rs`

```rust
#[derive(Debug, Clone)]
pub enum Backend {
    Memory(MemStore),
    // MOD-6: Online(PgStore, CacheStore), Offline(CacheStore)  — §6.1
}

impl Backend {
    pub fn memory(store: MemStore) -> Self;
    /// Top-bar store-state text (T5). MOD-6 returns "online" / "offline · 3m".
    pub fn label(&self) -> String;                 // "memory"
    pub fn is_writable(&self) -> bool;             // true for Memory; Offline will be false
}

impl ReadStore for Backend { /* one `match self` per method, delegating */ }
```

**Hierarchy reads are inherent `Backend` methods, not trait methods** **`[blueprint]`**: §6.1's
`ReadStore` is quoted verbatim (V2) and has no `workspaces()`, yet the switcher (T5) and the top
bar need workspaces, the box row and an active-run count. `WriteStore`'s trailing comment
(`// runs, steps, events, … box, agents ...`) shows the trait set is explicitly unfinished, so
adding methods to `ReadStore` now would guess at MOD-6's shape. Inherent methods on the concrete
`Backend` give the worker what it needs, keep §6.1 untouched, and MOD-6/MOD-15 can promote them
into a `HierarchyStore` trait without touching a view — the worker is the only caller.

```rust
impl Backend {
    pub async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>>;   // ordered by name
    pub async fn box_info(&self) -> Result<Option<BoxInfo>>;           // this box's row
    pub async fn active_runs(&self, scope: &Scope) -> Result<usize>;   // RunStatus::is_active
    pub async fn projects(&self, scope: &Scope) -> Result<Vec<ProjectRef>>;
}
```

### B.8 `store/mem.rs` — `MemStore`

```rust
#[derive(Debug, Clone, Default)]
pub struct MemStore { state: Arc<RwLock<State>> }     // std::sync::RwLock

#[derive(Debug, Default)]
struct State {
    users: HashMap<UserId, AppUser>, boxes: HashMap<BoxId, BoxRow>, this_box: Option<BoxId>,
    workspaces: HashMap<WorkspaceId, Workspace>, workspace_projects: Vec<WorkspaceProject>,
    projects: HashMap<ProjectId, Project>, kinds: HashMap<ItemKindId, ItemKind>,
    graphs: HashMap<StepGraphId, StepGraph>, phases: Vec<StepGraphPhase>,
    templates: Vec<PromptTemplate>, agents: HashMap<AgentId, Agent>,
    item_key_counter: HashMap<(ProjectId, String), i32>,   // §4.1
    items: HashMap<ItemId, Item>, revisions: HashMap<(ItemId, i32), ItemRevision>,
    links: Vec<ItemLink>,                                  // tombstones kept (§5.5 deleted_at)
    notes: Vec<Note>, documents: Vec<Document>,
    runs: HashMap<RunId, Run>, steps: HashMap<StepId, RunStep>, events: Vec<SessionEvent>,
}

impl MemStore {
    pub fn new() -> Self;                                     // empty (D7)
    #[cfg(feature = "demo")] pub fn demo() -> Self;           // loads fixtures::DemoData
    #[cfg(feature = "demo")] pub fn from_demo(data: crate::fixtures::DemoData) -> Self;
    pub fn item_count(&self) -> usize;                        // tests only

    fn read<R>(&self, f: impl FnOnce(&State) -> R) -> R;      // lock, call f, drop guard, return
    fn write<R>(&self, f: impl FnOnce(&mut State) -> R) -> R;
}
```

**Locking rule (plan risk row, D6): the lock is never held across an `.await`.** Every trait method
body is one call to `read` / `write` with a plain (non-async) closure that computes and clones
owned values; the async fn bodies contain **no `.await` at all**, so holding a guard across a
suspension point is structurally impossible, not a convention. Poison recovery:
`unwrap_or_else(std::sync::PoisonError::into_inner)` — `State` mutations are infallible map
inserts, so an unrelated panic must not take the store down. **`[blueprint]`**

Behaviour `MemStore` must enforce (D6):

- **`mint_item`** (§7.1, §4.1): resolve `kind_id` → `key_prefix` (absent, or kind belonging to
  another project → `StoreError::Constraint`); `item_key_counter[(project, prefix)] += 1`, first
  use starts at 1; `version = 1`; `status = Open`; `key = "{prefix}-{number}"`; write
  `ItemRevision { version: 1, reason: "created" }` in the same `write` closure.
- **`update_item`** (§7.2, §4.2): `version != expected_version` →
  `Diverged { head, ancestor }` with `ancestor = revisions[(id, expected_version)]` (missing →
  `NotFound { entity: "item_revision", .. }`). On match: apply only `Some` fields, `version += 1`,
  `updated_at = now`, push a revision carrying the patch's `reason`. `status` / `closed_at` are
  never touched here.
- **`transition`** (§4.2): `if item.status == from { status = to; updated_at = now;
  if to.is_terminal() { closed_at = Some(now) }; Ok(true) } else { Ok(false) }`. Never bumps
  `version`, never writes a revision.
- No delete path for items exists (§4.1); counters never decrease.
- **`links(id, hops)`**: BFS from `id` over links with `deleted_at.is_none()`, following edges in
  both directions, `hops` levels, across projects; `LinkNode.depth` is the BFS depth (root = 0);
  nodes deduplicated; `edges` = live edges between visited nodes; `hops == 0` → root node only.
- **`items(scope, filter)`**: project ∈ `scope.project_ids` (∩ `filter.project_ids` when set), then
  the B.4 predicates; ordered **`[blueprint]`** by `(index of project in scope.project_ids,
  key_prefix, key_number)` so the Backlog list renders the store's order directly and MOD-6's
  `ORDER BY` is fixed now.
- **`documents(id)`** ordered by `(kind, version)`; **`notes(id)`** by `created_at`;
  **`runs(id)`** by `queued_at DESC` (§5.8 `idx_run_item`), `steps` by
  `(position, attempt, fanout_index)`; **`step_events(step)`** by `seq`, `Ok(None)` for an unknown
  step id (`MemStore` is not a cache, but the `None = not cached` contract stays exercised).

### B.9 `store/conformance.rs` (feature `test-support`)

```rust
/// Runs every case in `CASES`. `make` must return a store freshly loaded with
/// `fixtures::DemoData` and nothing else; each case gets its own store.
pub async fn run_all<S, F, Fut>(make: F)
where S: WriteStore, F: Fn() -> Fut, Fut: Future<Output = S>;

/// Case names in run order. A name never changes: MOD-6 reports per case.
pub const CASES: &[&str] = &[ /* the 13 names below */ ];
```

`Cargo.toml`: `test-support = ["demo"]` **`[blueprint]`** — the suite asserts against the demo
fixture's known graph, so the features cannot be independent, and MOD-6 inherits a seed data set
instead of inventing a second one. `MemStore` is not mentioned anywhere in the suite.

| Case | Asserts |
|---|---|
| `mint_consecutive_keys` | three mints with the `FEAT` kind of project `htui` return `FEAT-4`, `FEAT-5`, `FEAT-6` (counter starts above the fixture's highest, §4.1) |
| `mint_prefix_isolation` | minting `ANA` afterwards returns `ANA-3`; the `FEAT` counter is unchanged |
| `mint_writes_revision_v1` | minted item has `version == 1`, `status == open`, `key_prefix` = the kind's prefix; `update_item(id, 1, …)` succeeds, proving revision 1 exists (§4.2) |
| `mint_unknown_kind_rejected` | `mint_item` with a kind from another project → `StoreError::Constraint` |
| `update_cas_success` | `update_item(id, v, patch)` → `Updated(item)`, `version == v + 1`, only patched columns changed |
| `update_cas_diverged` | a second update at the stale `v` → `Diverged { head, ancestor }`, `head.version == v + 1`, `ancestor.version == v`, `ancestor.title` is the pre-edit title |
| `status_cas_keeps_version` | `transition(id, open, queued)` → `true`, `version` unchanged, no new revision; a following `transition(id, open, done)` → `false` |
| `no_delete_path` | the trait exposes no delete; after `transition(.., closed)` the item is still returned by `item()` and by `items()` when `statuses` includes `closed` (§4.1) |
| `links_hops_1_vs_2` | from `htui FEAT-2`: hops 1 = {FEAT-2, FEAT-1}; hops 2 adds {ANA-1, FEAT-3, agy FEAT-1}; the tombstoned `TOOL-1 → FEAT-1` edge never appears; the cross-project node carries its own `project_id` |
| `filter_status_project_tags_ready` | four `items()` assertions: `statuses=[open]`, `project_ids=[agy]`, `tags=["rust"]`, `ready=Some(true)` (excludes `FEAT-2`, blocked by the non-terminal `FEAT-1`, and every non-`open` item) |
| `documents_ordered_by_version` | `documents(FEAT-1)` returns `plan` v1 before `plan` v2, grouped by kind |
| `notes_ordered_by_created_at` | `notes(FEAT-1)` ascending and stable |
| `events_ordered_by_seq` | `step_events(plan step)` → seq 0..7 in order, seq 0 is `EventKind::Prompt` / `EventRole::Htui` (§4.3); an unknown `StepId` → `Ok(None)` |

`crates/htui-core/tests/mem_store.rs`:

```rust
#[tokio::test]
async fn mem_store_conformance() {
    htui_core::store::conformance::run_all(|| async { MemStore::demo() }).await;
}
```

---

## C. TUI crate interfaces (`htui`)

### C.1 `store_worker.rs` (D4)

```rust
pub type Seq = u64;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Origin { App, Tab(TabId), Overlay(OverlayId) }

#[derive(Debug, Clone)]
pub enum StoreRequest {
    Workspaces,                                  // T3/T5
    BoxInfo,                                     // [blueprint] top-bar "box" (R-TUI-1), no probe (MOD-7)
    ActiveRuns { scope: Scope },                 // [blueprint] top-bar run count (R-TUI-1)
    Items { scope: Scope, filter: ItemFilter },
    Item(ItemId),
    Links { id: ItemId, hops: u8 },
    Documents(ItemId),
    Notes(ItemId),
    Runs(ItemId),
}

#[derive(Debug, Clone)]
pub enum StoreReply {
    Workspaces(Vec<WorkspaceSummary>),
    BoxInfo(Option<BoxInfo>),
    ActiveRuns(usize),
    Items(Vec<ItemSummary>),
    Item(Box<Option<Item>>),
    Links(LinkGraph),
    Documents(Vec<DocumentHead>),
    Notes(Vec<Note>),
    Runs(Vec<RunSummary>),
    Failed { request: &'static str, message: String },
}

#[derive(Debug)] pub struct RequestEnvelope { pub seq: Seq, pub origin: Origin, pub request: StoreRequest }
#[derive(Debug)] pub struct ReplyEnvelope   { pub seq: Seq, pub origin: Origin, pub reply: StoreReply }

/// Pure: one request, one reply, no channels. The spawned task is a loop around it; the test
/// harness calls it inline (C.8) so snapshots need no sleeps.  [blueprint]
pub async fn serve(backend: &Backend, request: &StoreRequest) -> StoreReply;

/// Owns the `Backend`. No other task ever holds one — that is `R-NF-3` by construction (D4).
pub fn spawn(backend: Backend,
             rx: mpsc::UnboundedReceiver<RequestEnvelope>,
             tx: mpsc::UnboundedSender<ReplyEnvelope>) -> tokio::task::JoinHandle<()>;
```

`BoxInfo` and `ActiveRuns` are additions to the plan's request list **`[blueprint]`**: `R-TUI-1`
requires the top bar to show the box and the active run count, and neither is derivable from the
plan's seven variants. Both are served by `Backend`'s inherent reads (B.7).

Channels are **unbounded** in both directions **`[blueprint]`**: `UnboundedSender::send` takes
`&self` and never awaits, so the render side can enqueue without a runtime handle and without any
possibility of blocking (`R-NF-3`). Depth is bounded in practice by one request per keystroke.

### C.2 Staleness and routing rules **`[blueprint]`** (plan says only "a request `seq`")

- `App` owns `next_seq: Seq` and `latest: HashMap<(Origin, Discriminant<StoreRequest>), Seq>`.
  Every dispatch stamps `seq = next_seq++` and overwrites `latest[(origin, discriminant)]`.
  `std::mem::discriminant` is `Copy + Eq + Hash`, so no parallel "request kind" enum is needed.
- A reply is **stale** and dropped when `latest[(origin, discriminant(request))] != reply.seq`.
  Keying on the discriminant (not on the id inside it) is what makes fast selection movement in
  the Backlog correct: only the newest `Item(_)` reply per origin survives.
- Replies are **addressed**, not broadcast: `App::update(Action::Reply(env))` delivers to
  `env.origin` (`Tab(id)` → that tab's `on_reply`, `Overlay(id)` → that overlay if still on the
  stack, `App` → `App` itself). Rationale: a broadcast would let any tab parse another tab's data
  and would make every future tab's `on_reply` a filter over unrelated variants.
- Before routing, `App::observe_reply(&StoreReply)` gets a read-only look at every reply and
  updates `TopBarState` only (`Workspaces` → workspace name, `BoxInfo` → box, `ActiveRuns` →
  count). This is the single exception to addressing and it never mutates a tab.
- A reply addressed to an overlay that has been popped, or to a tab that no longer exists, is
  dropped silently.

### C.3 `app/action.rs`, `app/state.rs`

```rust
pub enum Action {
    Quit,
    Tab(TabAction),
    Overlay(OverlayAction),
    Store(StoreRequest),                 // stamped with the emitting Ctx's Origin at drain time
    Reply(ReplyEnvelope),
    SetScope { workspace: WorkspaceSummary },   // T5
    ToggleHelp,                                 // [blueprint] `?` binding
    Error(String),                              // [blueprint] status-line text for StoreReply::Failed
    Tick,
}
pub enum TabAction { Next, Prev, Select(usize), Focus(TabId) }
pub enum OverlayAction { Open(OverlayId), Close, CloseAll }

pub enum Handled { Consumed, Pass }   // [blueprint]: side effects go through Ctx::emit, so one
                                      // key may produce several actions and the return type stays
                                      // a two-state answer to "did you take it?"

pub struct TopBarState { pub workspace: String, pub box_name: String,
                         pub store: String,          // Backend::label()
                         pub active_runs: usize }

/// Action sink handed to views. Views hold no channel and no store handle (R-NF-3).
#[derive(Debug, Default)]
pub struct Emit(RefCell<Vec<Action>>);

pub struct Ctx<'a> {
    pub scope: &'a Scope,
    pub projects: &'a [ProjectRef],      // scope's projects, ordered by workspace_project.position
    pub top_bar: &'a TopBarState,
    pub keymap: &'a Keymap,
    pub theme: &'a Theme,
    origin: Origin,
    emit: &'a Emit,
}
impl Ctx<'_> {
    pub fn request(&self, req: StoreRequest);   // = emit(Action::Store(req)), stamped with origin
    pub fn emit(&self, action: Action);
    pub fn origin(&self) -> &Origin;
}

pub struct App {
    pub scope: Scope,
    pub projects: Vec<ProjectRef>,
    pub top_bar: TopBarState,
    pub tabs: TabRegistry,
    pub overlays: OverlayStack,
    pub overlay_factories: OverlayRegistry,
    pub keymap: Keymap,
    pub theme: Theme,
    pub help_visible: bool,
    pub status: Option<String>,       // last Error(..) text
    pub should_quit: bool,
    pub dirty: bool,                  // set by update, cleared by the draw
    emit: Emit,
    requests: mpsc::UnboundedSender<RequestEnvelope>,
    latest: HashMap<(Origin, Discriminant<StoreRequest>), Seq>,
    next_seq: Seq,
    ticks: u64,
}

impl App {
    pub fn new(requests: mpsc::UnboundedSender<RequestEnvelope>, keymap: Keymap) -> Self;
    pub fn on_key(&mut self, key: KeyEvent);        // C.4 propagation order, then drain
    pub fn update(&mut self, action: Action);       // app/update.rs
    pub fn render(&mut self, frame: &mut Frame<'_>);
    fn drain(&mut self);                            // apply every Action the Emit collected
    fn dispatch(&mut self, origin: Origin, req: StoreRequest);   // stamp seq, record latest, send
    fn ctx(&self, origin: Origin) -> Ctx<'_>;
}
```

`Action::Store` carries no origin: `drain` knows which view it just called and stamps there.

### C.4 Key propagation (plan D5, made exact) **`[blueprint]`**

`App::on_key` in order, stopping at the first `Handled::Consumed`:

1. `overlays.top_mut()` — only the topmost overlay; if it returns `Pass` **and** `is_modal()` is
   true, propagation stops anyway (a modal swallows unhandled keys).
2. `keymap.resolve(KeyScope::Overlay(top_id), chord)` — overlay-scoped bindings (`Esc` → Close).
3. `tabs.active_mut().on_key(..)` (skipped when a modal overlay is open).
4. `keymap.resolve(KeyScope::Tab(active_id), chord)`.
5. `keymap.resolve(KeyScope::Global, chord)` → `q`, `Tab`/`Shift+Tab`, `1..9`, `w`, `?`.

After the chain, `drain()` applies every emitted `Action` and sets `dirty`.

### C.5 Traits and registries (`ui/tabs/registry.rs`, `ui/tabs/backlog/detail/mod.rs`, `ui/overlay/registry.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] pub struct TabId(pub &'static str);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] pub struct OverlayId(pub &'static str);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] pub struct DetailId(pub &'static str);

pub trait Tab {
    fn id(&self) -> TabId;
    fn title(&self) -> &str;
    /// Called on activation and after every scope change. Never called during render.
    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest>;
    fn on_scope_change(&mut self, scope: &Scope);          // clear caches (T5)
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled;
    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);
}

pub trait DetailTab {                                       // T4, per selected item
    fn id(&self) -> DetailId;
    fn title(&self) -> &str;
    fn on_item_change(&mut self, item: Option<ItemId>);      // [blueprint] mirror of on_scope_change
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled;
    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);
}

pub trait Overlay {
    fn id(&self) -> OverlayId;
    fn title(&self) -> &str;
    fn is_modal(&self) -> bool;
    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest>;   // [blueprint] called on push
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled;
    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);   // area = whole frame
}

pub struct TabRegistry { /* Vec<Box<dyn Tab>>, active: usize */ }
impl TabRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, tab: Box<dyn Tab>);          // registration order = strip order
    pub fn active(&self) -> &dyn Tab;
    pub fn active_mut(&mut self) -> &mut dyn Tab;
    pub fn active_id(&self) -> TabId;
    pub fn by_id_mut(&mut self, id: TabId) -> Option<&mut dyn Tab>;
    pub fn select(&mut self, idx: usize) -> bool;
    pub fn focus(&mut self, id: TabId) -> bool;
    pub fn next(&mut self); pub fn prev(&mut self);
    pub fn titles(&self) -> Vec<(TabId, &str)>;             // tab strip widget input
}
// DetailRegistry: the same API over Box<dyn DetailTab>, plus `cycle_next`/`cycle_prev` (h/l, [/]).

pub struct OverlayRegistry { /* HashMap<OverlayId, Box<dyn Fn() -> Box<dyn Overlay>>> */ }
impl OverlayRegistry { pub fn register(&mut self, id: OverlayId, f: impl Fn() -> Box<dyn Overlay> + 'static);
                       pub fn create(&self, id: OverlayId) -> Option<Box<dyn Overlay>>; }
pub struct OverlayStack { /* Vec<Box<dyn Overlay>> */ }
impl OverlayStack { pub fn push(&mut self, o: Box<dyn Overlay>); pub fn pop(&mut self);
                    pub fn clear(&mut self); pub fn top(&self) -> Option<&dyn Overlay>;
                    pub fn top_mut(&mut self) -> Option<&mut dyn Overlay>;
                    pub fn by_id_mut(&mut self, id: OverlayId) -> Option<&mut dyn Overlay>;
                    pub fn is_empty(&self) -> bool;
                    pub fn iter(&self) -> impl Iterator<Item = &dyn Overlay>; }  // render bottom-up
```

### C.6 `keymap.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord { pub code: KeyCode, pub mods: KeyModifiers }
impl KeyChord { pub fn parse(spec: &str) -> Option<Self>; pub fn label(&self) -> String; }

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyScope { Global, Tab(TabId), Overlay(OverlayId) }

pub struct Binding { pub scope: KeyScope, pub key: KeyChord, pub action: Action, pub help: &'static str }

pub struct Keymap { bindings: Vec<Binding> }
impl Keymap {
    pub fn default_global() -> Self;                       // T3 table below
    pub fn bind(&mut self, b: Binding);                    // T6 adds `w`
    pub fn resolve(&self, scope: &KeyScope, chord: KeyChord) -> Option<&Action>;
    pub fn help_line(&self, scope: &KeyScope) -> String;   // "q quit · Tab next · w workspace · ? help"
}
```

Global table (T3): `q` Quit · `Tab` TabAction::Next · `Shift+Tab` Prev · `1`..`9` Select(n-1) ·
`?` ToggleHelp. Overlay scope: `Esc` OverlayAction::Close. T6 adds global `w` →
`OverlayAction::Open(OverlayId("workspace_switcher"))`. `Action` therefore derives `Clone`.

### C.7 `terminal.rs`, `event_loop.rs`, `cli.rs`

```rust
// terminal.rs  (D8)
pub struct TerminalGuard { terminal: DefaultTerminal, restored: bool }
pub fn init() -> TerminalGuard;                 // install_panic_hook() then ratatui::init()
pub fn install_panic_hook();                    // chains: ratatui::restore() then the previous hook
impl TerminalGuard {
    pub fn terminal_mut(&mut self) -> &mut DefaultTerminal;
    pub fn restore(&mut self);                  // idempotent
}
impl Drop for TerminalGuard { fn drop(&mut self) { self.restore(); } }
```

```rust
// event_loop.rs  (D4)  — io::Result, not anyhow (anyhow lives in main only, plan Patterns)
pub const TICK: Duration = Duration::from_millis(250);
pub async fn run(term: &mut TerminalGuard, app: &mut App,
                 replies: mpsc::UnboundedReceiver<ReplyEnvelope>) -> std::io::Result<()>;
```

```rust
let mut events = crossterm::event::EventStream::new();      // needs futures::StreamExt (V4)
let mut ticker = tokio::time::interval(TICK);
term.terminal_mut().draw(|f| app.render(f))?;               // first frame before the loop
loop {
    tokio::select! {
        Some(ev) = events.next()      => app.on_terminal_event(ev?),   // Key / Resize / Paste
        Some(env) = replies.recv()    => app.update(Action::Reply(env)),
        _ = ticker.tick()             => app.update(Action::Tick),
        else => break,
    }
    if app.should_quit { break; }
    if std::mem::take(&mut app.dirty) { term.terminal_mut().draw(|f| app.render(f))?; }
}
```

Three arms, forever: a new tab, overlay, action or request adds **no** arm (PRD extensibility
metric). Resize is handled by ratatui's autoresize; `on_terminal_event` only sets `dirty` for it.
`Action::Tick` sets `dirty` **only** every 4th tick (1 s), when it also re-issues
`ActiveRuns { scope }` from `Origin::App` so the top-bar count stays live **`[blueprint]`**.

```rust
// cli.rs
#[derive(Debug, clap::Parser)]
#[command(name = "htui", version)]
pub struct Args {
    /// Load the demo fixture instead of an empty store (D7).
    #[arg(long)] pub demo: bool,
    /// Log file; never stdout, it is the TUI (plan Patterns/Logging).
    #[arg(long, env = "HTUI_LOG", value_name = "PATH")] pub log: Option<PathBuf>,
}
```

### C.8 `testkit.rs` (T3, used by T4 and T5) **`[blueprint]`**

```rust
pub struct Harness { app: App, backend: Backend, rx: UnboundedReceiver<RequestEnvelope>,
                     term: Terminal<TestBackend> }
impl Harness {
    pub fn empty() -> Self;                                  // MemStore::new(), no tabs
    pub fn demo() -> Self;                                   // MemStore::demo(), scope = first workspace
    pub fn with_tab(self, tab: Box<dyn Tab>) -> Self;        // T4 registers BacklogTab, T6 not needed
    pub fn with_overlay(self, o: Box<dyn Overlay>) -> Self;  // T5 pushes WorkspaceSwitcher
    pub fn size(self, w: u16, h: u16) -> Self;               // default 100x30 (plan risk row)
    pub async fn settle(&mut self);                          // drain requests through
        // store_worker::serve(&backend, req) inline, feed replies to app.update, repeat until the
        // queue is empty — deterministic, no sleeps, no spawned task
    pub fn key(&mut self, chord: &str);                      // "j", "Enter", "shift-tab"
    pub fn render(&mut self) -> String;                      // TestBackend buffer as text
    pub fn app(&mut self) -> &mut App;
}
```

T4 and T5 both call `Harness::demo().with_tab(...)` / `.with_overlay(...)`, so **neither needs the
T6 registration to exist** and neither edits a T3 file. `settle()` is the reason a snapshot is
byte-stable: every request raised by activation, scope change or a key is served before the frame.

---

## D. Data flow

- **Startup**: `main` → `cli::Args::parse()` → `tracing` file layer if `--log`/`HTUI_LOG` →
  `MemStore::demo()` when `--demo` else `MemStore::new()` → `Backend::memory(..)` → two unbounded
  channels → `store_worker::spawn(backend, rx, tx)` (the backend moves into the task and is
  unreachable from the UI thereafter, `R-NF-3`/D4) → `terminal::init()` (panic hook + guard, D8) →
  `App::new(req_tx, Keymap::default_global())` → `app::mod::register_all(&mut app)` (T6: Backlog,
  Skills, Settings; switcher factory; `w` binding) → App dispatches `Workspaces`, `BoxInfo` from
  `Origin::App` → `event_loop::run` draws the first frame immediately (empty top bar, tab strip,
  Backlog "loading") → replies arrive → `SetScope` for the first workspace (with `--demo`) or the
  switcher opens over "no workspaces" (T5) → the active tab's `wants_requests` fire → redraw.
- **Key press**: `EventStream` → `Action`-less `App::on_key` → propagation chain C.4 → views call
  `ctx.emit` / `ctx.request` → `drain()` applies each `Action` through `App::update` → `dirty` →
  the loop redraws. No store call anywhere on this path.
- **Scope change**: `Action::SetScope { workspace }` → `scope = Scope::from_workspace(&ws)`,
  `projects = ws.projects`, `top_bar.workspace = ws.name` → `for tab in registry { tab.on_scope_change(&scope) }`
  (caches cleared) → `overlays.clear()` → for the **active** tab, dispatch `wants_requests(&scope)`
  with `Origin::Tab(id)`; the same happens lazily for another tab the first time it is activated →
  `ActiveRuns { scope }` from `Origin::App` → redraw with empty panes and their one-line empty
  states until replies land.
- **Reply arrival**: worker → `ReplyEnvelope` → `Action::Reply` → `App::observe_reply` (top bar
  only) → staleness check C.2 → drop, or route to `env.origin` → `on_reply` updates that view's
  cached snapshot, may `ctx.request` a follow-up (Backlog: an `Items` reply that changes the
  selection issues `Item`/`Runs`/`Links{hops:1}`/`Documents`/`Notes`) → `dirty` → redraw.
- **Tab activation**: `TabAction::{Next,Prev,Select,Focus}` → registry moves `active` → dispatch
  the newly active tab's `wants_requests(&scope)` → redraw.

---

## E. Build order and task hand-off

TDD per task: the listed tests are written first, then the code. Waves: A = {T1, T2} serial,
B = {T3}, C = {T4, T5} parallel, D = {T6}.

### T1 — workspace + core model + store traits
- **Owns**: `Cargo.toml`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `.gitignore`,
  `crates/htui-core/Cargo.toml`, `src/lib.rs`, `src/model/**` (13 files + `agent.rs`),
  `src/store/{mod,error,traits,backend}.rs`.
- **Exposes to T2/T3**: every type in §B.1-B.7. `Backend::Memory` references `MemStore`, so T1
  lands a `mem` module stub: `pub struct MemStore` + `impl ReadStore/WriteStore` whose bodies are
  `todo!()` and whose file is handed to T2 whole.
- **Tests first**: unit tests inside `model/mod.rs` — every enum round-trips `as_str()` →
  `FromStr` → itself and covers the CHECK list exactly (`ALL.len()` equals the DDL list length);
  ID newtypes round-trip through `Display`/`FromStr` and serde; `Scope::from_workspace` orders
  `project_ids` by `position`.
- **Validate**: `cargo build -p htui-core`, `cargo clippy -p htui-core -- -D warnings`.

### T2 — MemStore, conformance suite, fixtures
- **Owns**: `crates/htui-core/src/store/mem.rs`, `store/conformance.rs`, `src/fixtures.rs`,
  `tests/mem_store.rs` (plus the feature rows in `htui-core/Cargo.toml`).
- **Exposes to T3/T4/T5**: `MemStore::{new, demo, from_demo}`, `fixtures::DemoData`, the
  `fixtures::ids` constants of §G, `conformance::{run_all, CASES}`.
- **Tests first**: `tests/mem_store.rs` and the 13 conformance cases (B.9) before any `mem.rs`
  body; then the fixture, which the cases assert against.
- **Validate**: `cargo test -p htui-core --features test-support,demo`.

### T3 — TUI shell
- **Owns**: `crates/htui/Cargo.toml`, `src/{main,lib,cli,terminal,store_worker,event_loop,keymap,testkit}.rs`,
  `src/app/{mod,state,action,update}.rs`, `src/ui/{mod,theme,layout,top_bar}.rs`,
  `src/ui/tabs/{mod,registry,skills,settings}.rs`, `src/ui/overlay/{mod,registry}.rs`.
- **Exposes to T4/T5**: `Tab`, `Overlay`, `Ctx`, `Handled`, `TabId`/`OverlayId`, `Action`,
  `StoreRequest`/`StoreReply`/`Origin`, `TabRegistry`/`OverlayRegistry`/`OverlayStack`,
  `Keymap`/`Binding`/`KeyChord`/`KeyScope`, `ui::theme::Theme`, `ui::layout`, `testkit::Harness`.
  `DetailTab`/`DetailRegistry` are **not** T3's: they live under `ui/tabs/backlog/detail/mod.rs`
  and belong to T4 (plan's file table), so nothing outside the Backlog tab depends on them.
- **Tests first**: `src/keymap.rs` unit tests (resolution order per scope, unknown chord →
  `None`); `src/store_worker.rs` unit tests (`serve` returns the matching reply variant for each
  request against `MemStore::demo()`); an in-crate snapshot test that an empty-store shell renders
  top bar + tab strip + the Skills stub at 100x30.
- **Validate**: `cargo build -p htui`, `cargo clippy -p htui -- -D warnings`, `cargo test -p htui`.

### T4 — Backlog tab (parallel with T5)
- **Owns**: `src/ui/tabs/backlog/{mod,list}.rs`, `backlog/detail/{mod,body,runs,graph,documents,notes}.rs`,
  `tests/backlog.rs`, `tests/snapshots/backlog__*.snap`.
- **Exposes to T6**: `pub struct BacklogTab` with `pub fn new() -> Self` and
  `pub const ID: TabId = TabId("backlog")`.
- **Tests first, against T3's interfaces only**: `tests/backlog.rs` uses
  `Harness::demo().with_tab(Box::new(BacklogTab::new()))`, so it never touches a T3 or T5 file and
  does not need T6's registration. Cases: grouped list for the two-project demo workspace; fold a
  project group on `Enter`; `j`/`k`/`g`/`G` movement; each of the five sub-tabs with data; each of
  the five sub-tabs empty (a one-line message, never a blank pane, D11).
- **Validate**: `cargo test -p htui --features testkit`.

### T5 — Workspace switcher + top bar wiring (parallel with T4)
- **Owns**: `src/ui/overlay/workspace_switcher.rs`, `tests/shell.rs`,
  `tests/snapshots/shell__*.snap`.
- **Exposes to T6**: `pub struct WorkspaceSwitcher` with `pub fn new() -> Self` and
  `pub const ID: OverlayId = OverlayId("workspace_switcher")`.
- **Tests first, against T3's interfaces only**: `tests/shell.rs` uses
  `Harness::demo().with_overlay(Box::new(WorkspaceSwitcher::new()))`. Cases: switcher open listing
  the two demo workspaces with project counts; `Enter` emits `SetScope` and the top bar reads
  `Graphics · <box> · memory · 1 run`; `Esc` closes; empty store shows "no workspaces"
  (creation is MOD-15).
- **T5 does not edit `app/update.rs`**: the `SetScope` arm is T3's (C.3 lists the variant), so the
  switcher only emits it. This is what keeps T4 ∩ T5 = ∅ (V6).
- **Validate**: `cargo test -p htui --features testkit`.

### T6 — registration, README, cross-platform check
- **Owns**: edits to `src/app/mod.rs`, `src/ui/tabs/mod.rs`, `src/ui/overlay/mod.rs`; `README.md`.
- **Does**: `register_all` puts `BacklogTab` first, then Skills, Settings; registers the switcher
  factory under its `OverlayId`; binds global `w`. README: build, run, `--demo`, key table,
  Windows Terminal target / conhost best-effort.
- **Validate**: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings`, `cargo test --workspace --all-features`,
  `cargo run -p htui -- --demo` on Windows Terminal.

---

## F. Extension points

No downstream module adds a `match` arm to `event_loop::run` — its three `select!` arms are over
transports (terminal, replies, tick), not over features.

| Module | Adds | Files it touches | Event-loop change |
|---|---|---|---|
| MOD-2 Chat tab | `ChatTab: Tab` + `StoreRequest::StepEvents(StepId)` / `StoreReply::StepEvents` (`ReadStore::step_events` already exists, §6.1) + a `Backend::Chat`-side driver crate | new `ui/tabs/chat/**`; one `register` line in `app/mod.rs`; two variants in `store_worker.rs` | none |
| MOD-13 filters + editing | filter overlay (`Overlay`), `new`/`edit` bindings, `ItemPatch` writes through `WriteStore` | new `ui/overlay/filter.rs`, new `ui/overlay/item_edit.rs`, `Binding` rows, `ItemFilter` already carries `statuses`/`tags`/`ready`/`text` | none |
| MOD-14 Graph traversal | replaces `backlog/detail/graph.rs`'s flat table with a navigable view; re-rooting is `ctx.request(Links { id: other, hops: n })` | `ui/tabs/backlog/detail/graph.rs` only | none |
| MOD-15 hierarchy management | workspace/project/repo/kind editors as overlays + Settings sections | new `ui/overlay/*.rs`, new Settings sub-views; `Backend`'s inherent hierarchy reads (B.7) gain writes | none |
| MOD-6 Postgres + cache | `Backend::Online(PgStore, CacheStore)` / `Offline(CacheStore)`; `PgStore: WriteStore`, `CacheStore: ReadStore`; runs `conformance::run_all` against `PgStore` | `htui-core/src/store/backend.rs` (two variants + two delegating arms), new `htui-store` crate (D1), `Backend::label()` string | none; no view changes (`R-NF-3` / PRD hypothesis) |
| MOD-4 run actions | Runs sub-tab key handlers + orchestrator calls | `ui/tabs/backlog/detail/runs.rs`, new `WriteStore` methods | none |

Sub-tabs are a registry too: MOD-2's "promote to chat" and MOD-4's approve/reject are
`DetailTab::on_key` implementations inside their own file.

---

## G. Demo fixture spec (`fixtures.rs`, feature `demo`)

Deterministic IDs so T2's conformance cases and T4/T5's snapshots agree byte-for-byte:

```rust
pub const DEMO_EPOCH_MS: u64 = 1_788_393_600_000;   // 2026-09-03T00:00:00Z

/// v7-shaped, fully deterministic: 48-bit timestamp = DEMO_EPOCH_MS + class*1000 + n,
/// version nibble 7, variant 0b10, rand_a = 0, rand_b carries (class, n).
pub const fn demo_uuid(class: u8, n: u8) -> Uuid {
    let ms = DEMO_EPOCH_MS + (class as u64) * 1000 + n as u64;
    let b = ms.to_be_bytes();
    Uuid::from_bytes([b[2], b[3], b[4], b[5], b[6], b[7], 0x70, 0x00,
                      0x80, 0x00, 0x00, 0x00, 0x00, 0x00, class, n])
}
// demo_uuid(1, 0) == 01a06490-b7e8-7000-8000-000000000100
// demo_uuid(8, 3) == 01a06490-d343-7000-8000-000000000803
```

Class codes: 1 user · 2 box · 3 workspace · 4 project · 5 step_graph · 6 phase · 7 item_kind ·
8 item · 10 note · 11 document · 12 run · 13 run_step · 14 agent · 15 prompt_template.
Timestamps are equally deterministic: `demo_at(day, hour)` = `2026-09-01T00:00:00Z + day*24h +
hour*1h`, so no snapshot ever contains a wall clock.

```rust
pub struct DemoData { /* one Vec or HashMap per §5 table MemStore::State holds */ }
pub fn demo_data() -> DemoData;          // pure, no I/O, no randomness
pub mod ids { /* pub const USER: UserId = …; every id below, named as in the tables */ }
```

**Base rows.** `app_user` = `luigi` (class 1, n 0). `box` = `DESKTOP-HTUI`, `windows`, arch
`x86_64`, `probed_tags = ["rust","msvc","cmake"]`, `declared_tags = ["gpu"]` (class 2, n 0);
`MemStore::State.this_box` points at it, so the top bar reads `DESKTOP-HTUI`. Agents (§5.10):
`claude` (`acp`, `subscription`) and `agy` (`cli`, `per_token`) — class 14, n 0/1.

**Hierarchy.** Two workspaces (class 3), three projects (class 4):

| n | Workspace | slug | Projects (position) |
|---|---|---|---|
| 0 | Platform | `platform` | `htui` (0), `agy` (1) |
| 1 | Graphics | `graphics` | `vulkan-tutorials` (0) |

| n | Project | slug |
|---|---|---|
| 0 | htui | `htui` |
| 1 | agy | `agy` |
| 2 | Vulkan Tutorials | `vulkan-tutorials` |

Per project, the five `R-ENT-6` kinds (`ANA` analysis, `FEAT` feature, `FIX` bug, `CLEAN` refactor,
`TOOL` tooling) with their default graphs and phases, and one `prompt_template` v1 per phase name
(§5.10). Kind ids: `demo_uuid(7, project_index * 5 + kind_index)`; graphs `demo_uuid(5, …)`.

**Items** (class 8, `n` = the row number in this table, `created_by` = user, `priority` 0 unless
noted). `key` is derived, never stored by hand:

| n | Project | Key | Title | Status | tags | Notes |
|---|---|---|---|---|---|---|
| 0 | htui | `ANA-1` | Data model, box registry and sync topology | done | — | body: 3 short paragraphs |
| 1 | htui | `ANA-2` | Orchestrator step graphs and gates | open | — | |
| 2 | htui | `FEAT-1` | TUI scaffold | in_progress | `rust` | priority 2, body ~30 lines (scroll test) |
| 3 | htui | `FEAT-2` | Agent driver and chat tab | blocked | `rust` | |
| 4 | htui | `FEAT-3` | Postgres store and cache | queued | `rust` | priority 1 |
| 5 | htui | `FIX-1` | Terminal left raw after panic | closed | — | `closed_at` set |
| 6 | htui | `TOOL-1` | CI matrix for the three OSes | awaiting_approval | `docker` | |
| 7 | htui | `CLEAN-1` | Drop the legacy markdown exporter | failed | — | |
| 8 | agy | `ANA-1` | Prompt assembly survey | done | — | |
| 9 | agy | `FEAT-1` | ACP transport upgrade | open | `rust` | |
| 10 | agy | `FIX-1` | Session leak on cancel | open | — | |
| 11 | vulkan-tutorials | `FEAT-1` | Chapter 12 parity | in_progress | `gpu`, `vulkan` | |
| 12 | vulkan-tutorials | `TOOL-1` | Shader build script | open | `cmake` | |

Counters after load: `htui` {ANA 2, FEAT 3, FIX 1, TOOL 1, CLEAN 1}, `agy` {ANA 1, FEAT 1, FIX 1},
`vulkan-tutorials` {FEAT 1, TOOL 1}. Every item carries `item_revision` v1 (`reason = "created"`),
so §4.2's "the ancestor always exists" holds in the fixture too. All eight `Status` values appear
in project `htui`, which is what makes the Backlog snapshot a status-rendering test.

**Links** (`item_link`, live unless stated):

| from | kind | to |
|---|---|---|
| htui `FEAT-1` | origin | htui `ANA-1` |
| htui `FEAT-2` | blocked_by | htui `FEAT-1` |
| htui `FEAT-3` | origin | htui `ANA-1` |
| htui `FEAT-3` | relates | htui `FEAT-1` |
| htui `CLEAN-1` | supersedes | htui `FIX-1` |
| agy `FEAT-1` | relates | htui `FEAT-2` | *(cross-project, `R-ENT-9`)* |
| htui `TOOL-1` | relates | htui `FEAT-1` | **tombstoned**: `deleted_at = demo_at(2, 9)` |

**Notes** on htui `FEAT-1` (class 10, ascending `created_at`): n 0 "Skeleton only — filters,
actions and editing are MOD-13." at `demo_at(1, 9)`; n 1 "Snapshot sizes fixed at 100x30." at
`demo_at(1, 14)`.

**Documents** (class 11): htui `ANA-1` → `research` v1, `verdict` v1, `summary` v1 (all
`produced_by_step_id = None`, i.e. hand-written); htui `FEAT-1` → `prd` v1 and `plan` v1 / `plan`
v2, the two `plan` rows `produced_by_step_id = ids::STEP_PLAN` (the Documents sub-tab must show
"by step" vs "by hand").

**Runs** (class 12) and steps (class 13):

- `RUN_1` on htui `FEAT-1`, `kind = graph`, `mode = manual`, `status = done`,
  target/executing box = the demo box, `queued_at = demo_at(1, 8)`, `started_at = demo_at(1, 8)`,
  `finished_at = demo_at(1, 12)`. Steps, `(position, attempt, fanout_index)` ascending:
  `STEP_PRD` (prd, claude/sonnet, done, gate approved), `STEP_PLAN` (plan, claude/sonnet, done,
  gate approved), `STEP_IMPL` (implement, claude/opus, done, gate approved),
  `STEP_REVIEW` (review, agy/default, done, gate approved).
- `RUN_2` on htui `FEAT-3`, `kind = graph`, `mode = auto`, `status = queued`, one `pending` step
  (`prd`), `queued_at = demo_at(2, 8)`. **`[blueprint]`**: the plan asks for one finished run; a
  second, still-queued run is added so the top bar's active-run field is non-zero
  (`1 run`) in the T5 snapshot and `Backend::active_runs` is actually exercised. It is the only
  active run in the fixture, in the `Platform` workspace.

**Event stream** on `STEP_PLAN` (§4.3, `turn = 0`, `at = demo_at(1, 9) + seq minutes`):

| seq | kind | role | payload (keys per §4.3) |
|---|---|---|---|
| 0 | prompt | htui | `text`, `digest` (fixed sha256 hex literal), `sections[]` = [{prd, 800, false}, {skills, 300, false}] |
| 1 | assistant_text | agent | `text` = "Reading the PRD and the ANA-9 seam." |
| 2 | thought | agent | `text` = "The store trait is the only seam that matters here." |
| 3 | tool_call | agent | `title` = "Read docs/ANA-9.md", `tool_kind` = `read`, `input`, `locations[]` |
| 4 | tool_result | agent | `status` = `completed`, `output`, `locations[]`; same `tool_call_id` as seq 3 |
| 5 | plan | agent | `entries[]` = three `{content, status, priority}` rows |
| 6 | usage | agent | `input_tokens` 12000, `output_tokens` 2400, cache fields 0, `cost_micros` null |
| 7 | done | agent | `stop_reason` = "end_turn" |

`tool_call_id` = `"call_1"` on seq 3 and 4 (§4.3 pairing). No `raw` payloads
(`keep_raw_events` off). MOD-1 never renders these events — they exist so
`step_events` has real data for `events_ordered_by_seq` and for MOD-2 to replay.

---

## Reviewer checklist (derived from this blueprint)

1. `ReadStore`/`WriteStore`/`UpdateOutcome` are byte-identical to ANA-9 §6.1 (V2).
2. No `.await` appears in any `MemStore` method body; all state access goes through `read`/`write`.
3. No view module imports `htui_core::store::Backend` or any store trait; the only `Backend` is in
   `store_worker` (`R-NF-3`, D4).
4. `event_loop::run` still has exactly three `select!` arms.
5. Every model struct field name equals its §5 column name (D3).
6. Terminal is restored on: normal quit, `?` panic, and an `Err` bubbling out of `run` (D8).
7. T4 and T5 file sets remain disjoint; registration lives only in T6's three files (V6).

---

## Errata (recorded at implementation, 2026-09-03)

Found by the adversarial verifiers; the code follows the corrected reading.

- **§A.4 / §E T1**: the root `Cargo.toml` `members` list cannot name `crates/htui` before T3
  creates it; T1 lands `["crates/htui-core"]`, T3 adds `"crates/htui"`.
- **§B.9 `links_hops_1_vs_2`**: `agy FEAT-1 --relates--> htui FEAT-2` is a direct edge per §G,
  so with both-direction traversal (§B.8) it is at hop 1, not hop 2. Same node set at hops ≤ 2.
- **§E T4**: list rows sort by `(key_prefix, key_number)`, not `key_number` alone, so prefixes
  do not interleave inside a project.
- **§E T5**: after `Enter` on the second workspace the top bar reads
  `Platform · <box> · memory · 1 run`; the startup workspace is `Graphics` with `0 runs`
  (workspaces sort by name; the fixture's active run lives in Platform).
- **§C.5 `TabRegistry::active` / `active_id`**: return `Option`, because `Harness::empty()` has
  no tabs; `OverlayId::ANY` wildcard scope added so `Esc` can bind before any overlay exists.
- **§D "Startup"**: `register_all` names the switcher as `App::startup_overlay`; the first frame
  is the bare shell, and only an empty first `Workspaces` reply opens the switcher.
