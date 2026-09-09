# Blueprint: MOD-20 — registry-driven adapter install (milestones 1–4)

**Plan**: `.claude/plans/mod-20-registry-adapter-install.plan.md` (D8–D22, T1–T10 binding; its "Verified claims" and "Fact-check verdicts" tables taken as read; the two fact-check amendments — the explicit rustls provider install and the `content-length` **header** — are constraints here). **PRD**: `.claude/prds/mod-20-registry-adapter-install.prd.md` (D1–D7, the maintainer's). Re-checked against the tree at `7392584` on 2026-09-09; the live registry was read once to pin field spelling (`license_url`, `distribution.binary.<platform>.{archive,cmd,args?,sha256?}` — snake_case throughout). Where the plan and the tree disagree the tree wins and the point is marked **plan ≠ tree**; anything decided here that the plan left open is marked **decided here**; anything not verified in source is **UNVERIFIED — implementer must check**.

Conventions inherited unchanged: `#![warn(missing_docs)]`, `unsafe_code = "forbid"` (no `std::env::set_var`; every environment-dependent unit takes an injected `ProbeEnv` or an injected config), MSRV 1.98, edition 2024, no lock across an `.await`, nothing spawned or awaited long on the UI task or inside the worker's `select!` arm (`R-NF-3`), the H-7 fixture rule (only the `*_live.rs` files touch an unmodified seed row), and `R-AGT-5` — no agent name, registry id, vendor URL or vendor binary name outside `crates/htui-core/seeds/*.json` and `tests/` (`tests/extensibility.rs`'s new sweep enforces it for `antigravity`, `amp-acp`, `dl.google.com`, `agy_acp_server`).

---

## Plan ≠ tree, resolved

| # | Plan says | Tree says | Resolution |
|---|---|---|---|
| P-1 | T6: `install(plan, agent, existing, ctx, tier2, progress, cancel)` | `probe_agent` takes a `BoxId` (`probe.rs:1206-1212`), and `agent_box_row` keys on it | The re-probe needs it, so the call carries it. Nine arguments trips `clippy::too_many_arguments`; the row-side inputs travel as one `InstallJob<'_>` (B.9) |
| P-2 | T7 file set: `store_worker.rs`, `agent_worker.rs` | `Harness::drive` enumerates the runtime-served requests **by hand** (`testkit.rs:181-186`), and `drive_to_end` awaits only `background` (`:269`) | `crates/htui/src/testkit.rs` joins T7's file set: the three install variants join the list, and `finish_background` also awaits a live install (B.11). Without this T8's harness test answers every `i` with "no agent runtime in this harness" |
| P-3 | D16: `.staging/` is swept at the next install, `abort()` stays the shutdown path | `JoinHandle::abort` does **not** stop a `spawn_blocking` thread, and D10 unpacks on one | `AgentRuntime::shutdown` trips the install's `CancellationToken` **before** aborting; the unpack checks the token per entry (H-8). Recorded as the one place shutdown does more than `abort()` |
| P-4 | D14: `walk` returns `Vec<GlobMatch>` | `walk` is `pub` with one direct caller outside `glob_first`: `the_glob_walk_caps_its_candidate_fan_out` (`tests/probe.rs:514`) reads `.len()` only | The signature change is safe; that test is untouched |
| P-5 | T8 file set has no manifest | `crates/htui/Cargo.toml:49` dev-`tokio` enables `test-util` only; the harness test needs a `TcpListener` | `crates/htui/Cargo.toml` joins T8's file set: dev `tokio` gains `"net"` (the plan already does this for `htui-agent` in T4) |
| P-6 | D12/D18 name `PlanError`, `InstallError` | The house rule since milestone 5 is one `thiserror` type per crate (`DriverError`) | The plan's explicit types win — each variant is something the consent pane or the fallback renders differently, which a `Transport(String)` cannot carry. Neither is folded into `DriverError`; `InstallError` has `From<DriverError>` for `resolve_tool`'s `Transport` (B.5) |
| P-7 | D9: registry/`HEAD` 15 s **total**; archive connect 15 s + read 60 s, **no** total | `reqwest::ClientBuilder::timeout` is per **client** (`client.rs:1450`), not per request | `install/http.rs` builds two clients from one provider install: `short` (total 15 s) and `long` (connect 15 s, read 60 s) (B.4) |
| P-8 | Files table: `agent_claude.json` UPDATE at T1 "the `install` block" | T1's own test says the `claude` seed declares **no** `install` (its adapter is a `NodePackage`, PRD "Not for") | The test wins: T1 touches `agent_agy.json` only; `agent_claude.json` is untouched by this item and the plan's file table is amended in the phase note |
| P-9 | D14 + D16(a): the highest semver present is what the glob resolves, and other versions are deleted only **after** the re-probe | An install of a **lower** version beside a higher one would promote, watch the glob resolve the sibling, fail the post-promote check and roll back — a wasted download refused after the fact | **Decided here**: refused at pre-flight, before consent and before the `HEAD`: `PlanError::Outranked { installed, offered, path }` names the directory to remove by hand. The registry serves `latest` only, so this fires only when the registry moves backwards or a newer directory was placed by hand |
| P-10 | D16: a same-version re-install first moves the existing directory into `.staging/`; the sweep deletes entries older than an hour | A task aborted between that move and the restore would leave the *working* version in `.staging/` for the sweep to delete — D3's "a failed install never costs the working one" broken by the sweep itself | **Decided here**: set-aside entries carry a `.previous` suffix; the sweep **restores** a `.previous` whose `<root>/<id>/<version>/` is absent and deletes one whose target exists (the promote succeeded) (B.7, H-7) |
| P-11 | D18: `InstallFrame::Progress { phase, done, total }`; the router lists `InstallProgress` separately | — | `InstallProgress { phase, done, total }` is the `htui-agent` struct the sink receives; `InstallFrame::Progress` keeps the plan's struct-variant shape and is filled field for field in `run_install` |
| P-12 | D19: `Action::Error` for refusals; success text on "the status line" | `Action` has `Error(String)` for the status line (`action.rs:44`); **UNVERIFIED** whether a neutral status action exists | Refusals use `Action::Error` exactly as `r` does (`agents.rs:117`). Outcome text ("installed …", "rolled back …") is a section-local `notice` rendered on the hint line, so a success is never styled as an error |
| P-13 | D18: `Installer` "(client, config) is owned by the runtime" | `AgentRuntime::new` is sync and `reqwest::Client::build` can fail | The runtime owns the `InstallConfig`; the `Installer` (the two clients) is built inside the plan task, so a build failure is a `Failed` frame at the request's address, not a panic on the loop, and `AgentRuntime::production()` opens no socket until `i` |

---

## A. Per-file change table

| # | File | Action | Task | What changes (and what must **not**) |
|---|---|---|---|---|
| 1 | `crates/htui-agent/src/launch.rs` | UPDATE | T1 | `Install`, `InstallSource` (B.1); `Discovery.install` after `credential` (`:107-108`). **Not**: `resolve`, `spawn`, `ChildGuard` |
| 2 | `crates/htui-core/seeds/agent_agy.json` | UPDATE | T1, T2 | T1: `"install": { "source": "acp_registry", "id": "antigravity-acp", "tool": "agy_acp_server" }` after `"credential"`. T2: the three `htui`-managed patterns become `%HTUI_AGENTS_ROOT%/antigravity-acp/*/…`; the JetBrains pattern (`:20`, `:23`) untouched (C.1) |
| 3 | `crates/htui-agent/src/tools.rs` | UPDATE | T1 | `:143-149` literal gains `install: None`. Doc only otherwise |
| 4 | `crates/htui-agent/tests/launch.rs`, `tests/probe.rs` (`:140-144`, `:1993-2003`), `tests/acp_driver.rs` (`:519-530`), `tests/extensibility.rs` | UPDATE | T1 | Round-trip cases; the four `Discovery` literals gain `install: None`; `the_installer_names_no_vendor` beside `the_codebase_has_never_heard_of_zeta` (`:109`) over `tree_files()` (`:76`) filtered to `src/` |
| 5 | `crates/htui-agent/src/probe.rs` | UPDATE | T2, T3 | T2: `INSTALL_ROOT_VAR`, `default_install_root`, `install_root`; `ProbeEnv::host` seeds the var (`:99-116`). T3: `GlobMatch`, `walk` (`:666-703`), `newest` (`:705-723`), `version_key`. **Not**: `expand`, `segment_matches`, `glob_first`'s signature, `probe_agent`'s steps |
| 6 | `crates/htui-agent/tests/probe.rs` | UPDATE | T2, T3 | `env()` (`:36-57`) gains `HTUI_AGENTS_ROOT = <tmp>/agents`; `:440-499` move the server under it; T2/T3 cases per the plan |
| 7 | `Cargo.toml` | UPDATE | T4, T5 | Workspace: `reqwest`, `rustls`, `zip`, `tar`, `flate2`, `fs4` with the D9/D10/D11 feature sets (B.0) |
| 8 | `crates/htui-agent/Cargo.toml` | UPDATE | T4, T5 | The six above as `workspace = true`; dev `tokio` gains `"net"` |
| 9 | `crates/htui-agent/src/lib.rs` | UPDATE | T4 | `pub mod install;` + re-exports (B.3); `pub use launch::{…, Install, InstallSource, …}`; `pub use probe::{…, GlobMatch, INSTALL_ROOT_VAR, default_install_root, install_root, …}` |
| 10 | `crates/htui-agent/src/install/mod.rs` | CREATE | T4, T5, T6 | Module doc, `InstallConfig`, `Installer`, `InstallPlan`, `PlanError`, `InstallError`, `InstallOutcome`, `InstallProgress`, `InstallPhase`, `ManualSteps`, `InstallJob`, `Throttle`; re-exports of the submodules' public names. **Not**: any agent name, any URL but the ACP registry's |
| 11 | `crates/htui-agent/src/install/http.rs` | CREATE | T4 | `HttpClient` (two `reqwest::Client`s), the `Once` provider install, `content_length_header`, `HeadInfo`, `RegistryFetch`, `ByteStream`. The **only** file that names `reqwest` |
| 12 | `crates/htui-agent/src/install/registry.rs` | CREATE | T4 | Owned parse types, `RegistryCache` (`<root>/.registry/`), `read_registry` |
| 13 | `crates/htui-agent/src/install/plan.rs` | CREATE | T4 | `plan()`, `ArchiveFormat::for_url`, `cmd_relative`, `InstallPlan::consent_lines`, `ManualSteps::lines` |
| 14 | `crates/htui-agent/src/install/fetch.rs` | CREATE | T5 | `download()`: stream → file + `Sha256`, progress through the `Throttle`, cancellation by `select!` |
| 15 | `crates/htui-agent/src/install/archive.rs` | CREATE | T5 | `unpack()` (sync, under `spawn_blocking`), `safe_target`, `make_executable`, the zip/tar arms. The **only** file that names `zip`, `tar`, `flate2` |
| 16 | `crates/htui-agent/src/install/layout.rs` | CREATE | T5 | `Layout`: every path under the root, `sweep`, `set_aside`, `promote` (Windows retry), `restore`, `retain_only`, `existing_versions`, `nonce` |
| 17 | `crates/htui-agent/src/install/manifest.rs` | CREATE | T5 | `Manifest`, `Consent`, `InstallRecord`, `load`/`store` (tmp-then-rename), `consent_covers` |
| 18 | `crates/htui-agent/src/install/run.rs` | CREATE | T6 | `install()`: sweep → download → verify → unpack → set-aside → promote → glob check → re-probe → retention/rollback → manifest |
| 19 | `crates/htui-agent/tests/install.rs` | CREATE | T4, T5, T6 | The fixture server, the pinned registry, archives built at run time, every offline case (F) |
| 20 | `crates/htui-agent/tests/fixtures/registry.json` | CREATE | T4 | The two live entries verbatim (C.2) wrapped in `{ "version": "1.0.0", "agents": [..], "extensions": [] }` |
| 21 | `crates/htui-agent/tests/install_live.rs` | CREATE | T9 | The `amp-acp` proof, `#[ignore]` |
| 22 | `crates/htui/src/store_worker.rs` | UPDATE | T7 | Three variants (`:50-136`), `name()` arms (`:141-162`), `StoreReply::Install` (`:167-246`), the `try_serve` refusal arm (`:343-350`), the loop interception (`:484-488`), the loop-freedom test beside `:1233` |
| 23 | `crates/htui/src/agent_worker.rs` | UPDATE | T7 | `LiveInstall`, `AgentRuntime.{installer, install}`, `with_installer`, `serve` arms (`:285-342`), `shutdown` (`:353-364`), `finish_background` (`:238-246`), `install_plan`/`install_confirm`/`install_cancel`, `PlanArgs`/`InstallArgs`, `run_plan`/`run_install`, in-module tests |
| 24 | `crates/htui/src/testkit.rs` | UPDATE | T7 | P-2: `drive`'s runtime list (`:181-186`) |
| 25 | `crates/htui/src/ui/tabs/settings/agents.rs` | UPDATE | T8 | `TableState` cursor, `InstallState`, `i`/`j`/`k`/`y`/`n`/`Esc`/`x`, `on_reply` arms for `Install(..)` and the three `Failed` names, the pane and hint line under the table (B.12). **Not**: the column set, `on_box_cell`'s existing order |
| 26 | `crates/htui/tests/settings.rs`, `tests/snapshots/settings__agents_{demo,probed,unknown_row,empty}.snap`, `crates/htui/tests/install.rs`, `crates/htui/Cargo.toml` | UPDATE / CREATE | T8 | Section cases; snapshots re-accepted for the hint line; the harness end-to-end over a second fixture server (per-file helper, the repo's rule); dev `tokio/net` (P-5) |
| 27 | `README.md:234-267`, `crates/htui-agent/tests/agy_live.rs:101-111` + module doc, `HANDOFF.md`, the PRD, the plan | UPDATE (docs) | T10 | D22. `README.md:268-274` **not touched** |

Not touched, on purpose (section H): `crates/htui-store/**`, `crates/htui-store/migrations/**`, `crates/htui/src/keymap.rs`, `crates/htui/src/ui/overlay/**`, `crates/htui-agent/src/acp/**`, `crates/htui-core/src/**`, `crates/htui-core/seeds/agent_claude.json`.

### B.0 Manifests, exactly

```toml
# Cargo.toml [workspace.dependencies]
# MOD-20 (plan D9): the installer's HTTP client. `default-features = false` is load-bearing
# (`default` = default-tls + charset + http2 + system-proxy). `rustls-no-provider` keeps ring
# — sqlx's provider — the only one in the process; `install/http.rs` installs it explicitly.
reqwest = { version = "0.13", default-features = false, features = ["rustls-no-provider", "stream", "system-proxy"] }
rustls  = { version = "0.23", default-features = false, features = ["ring", "std", "tls12", "logging"] }
# MOD-20 (plan D10): deflate-only zip, plain tar, pure-Rust gzip.
zip     = { version = "8.6", default-features = false, features = ["deflate-flate2"] }
tar     = { version = "0.4", default-features = false }
flate2  = "1.1"
# MOD-20 (plan D11): free space without `unsafe` (`rustix` / `windows-sys`, both already in the lock).
fs4     = { version = "1.1", default-features = false, features = ["sync"] }
```

`crates/htui-agent/Cargo.toml`: the six as `{ workspace = true }` (T4: `reqwest`, `rustls`; T5: `zip`, `tar`, `flate2`, `fs4`); `[dev-dependencies] tokio = { workspace = true, features = ["macros", "rt", "rt-multi-thread", "test-util", "net"] }`. `crates/htui/Cargo.toml` dev `tokio` gains `"net"` (T8). `tokio-util` is already a dependency with `compat`; `CancellationToken` needs nothing more (fact-check verdict).

---

## B. Interfaces, exactly

### B.1 T1 — `launch.rs` (D12)

```rust
pub struct Discovery {
    #[serde(default)] pub tools: BTreeMap<String, ToolProbe>,
    #[serde(default = "default_true")] pub handshake: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialProbe>,
    /// Where this row's adapter comes from when the box does not have it (plan MOD-20 D12,
    /// `R-AGT-10`). Data, never code: the installer reads the registry entry `id` names and
    /// writes where `discovery.tools[tool]`'s glob will find it. A row without it — every
    /// `NodePackage`-served adapter — cannot be installed from the app, and `Settings > i` says so.
    /// `skip_serializing_if` for the round-trip rule at `credential`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<Install>,
}

/// `agent.launch.discovery.install` (plan D12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Install {
    /// Which registry the `id` is looked up in.
    pub source: InstallSource,
    /// The entry id in that registry — never an agent name, never a URL.
    pub id: String,
    /// The `${tool}` placeholder the installed `cmd` fills: the key into `discovery.tools`
    /// whose glob must resolve the file the installer wrote (D12's post-promote agreement).
    pub tool: String,
}

wire_enum!(
    /// `install.source` (plan D12): one value today; a second source is a variant, not a string.
    InstallSource {
        /// The ACP registry, `registry/v1/latest/registry.json`.
        AcpRegistry => "acp_registry",
    }
);
```

An unknown `source` fails as serde's own ``unknown variant `github_release`, expected `acp_registry` `` — T1's third case asserts the text names the value.

### B.2 T2, T3 — `probe.rs` (D15, D14)

```rust
/// The variable that names this box's install root (plan D15). `ProbeEnv::host` seeds it from
/// [`default_install_root`] unless the environment sets it; the seeds reach it as
/// `%HTUI_AGENTS_ROOT%`, so no seed document spells a root on any platform.
pub const INSTALL_ROOT_VAR: &str = "HTUI_AGENTS_ROOT";

/// `dirs::data_local_dir()/htui/agents`: `~/.local/share` (or `$XDG_DATA_HOME`), `~/Library/Application
/// Support`, `%LOCALAPPDATA%` — the three roots `agent_agy.json` used to hand-write. `None` on a
/// box with no local data directory.
#[must_use] pub fn default_install_root() -> Option<PathBuf>;

/// `env.var(INSTALL_ROOT_VAR)` as a path (the Windows case-insensitive lookup included).
/// `None` when the token is absent — a hand-built `ProbeEnv` that did not inject it.
#[must_use] pub fn install_root(env: &ProbeEnv) -> Option<PathBuf>;

// ProbeEnv::host, after `vars` is collected:
//   if !vars.contains_key(INSTALL_ROOT_VAR) — exact key; a Windows block spelling it in another case
//   is found by `var()` later and must not be shadowed —
//     if let Some(root) = default_install_root() { vars.insert(INSTALL_ROOT_VAR.to_owned(), root.to_string_lossy().into_owned()); }

/// One leaf [`walk`] found, with what each `*` segment matched (plan D14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobMatch {
    pub path: PathBuf,
    /// One entry per segment containing `*`, in pattern order: the directory (or leaf) name that
    /// segment matched. The seeds' version segment is the last one.
    pub captures: Vec<String>,
}

#[must_use] pub fn walk(root: &Path, segments: &[String]) -> Vec<GlobMatch>;

/// Plan D14, amending MOD-2 D48: `max()` over `(Option<semver::Version>, mtime, path)`.
/// The version is parsed from the **last** capture, a leading `v` stripped, strictly; `None <
/// Some` puts every parsable version above every unparsable one; among parsable ones semver
/// decides (a prerelease below its release); among unparsable ones the mtime-then-path-descending
/// rule of D48 stands unchanged, which is what keeps the JetBrains build-number shape resolving
/// exactly as before. Unreadable metadata still sorts as the epoch. A pattern with no `*` has no
/// capture and takes the fallback.
#[must_use] pub fn newest(matches: Vec<GlobMatch>) -> Option<PathBuf>;

/// `Version::parse(capture.strip_prefix('v').unwrap_or(capture)).ok()`.
fn version_key(capture: &str) -> Option<Version>;
```

`walk` internals: `current: Vec<(PathBuf, Vec<String>)>`; a `*` segment pushes `(dir.join(&name), captures + [name])`, a literal segment pushes the literal with captures unchanged; the cap and the `is_file` filter unchanged. `glob_first` (`:729-744`) is unchanged but for the type flowing through `newest`.

### B.3 `install/mod.rs` — the public surface

```rust
//! Plan MOD-20: a registry row's declared source made real. `plan` reads the ACP registry entry
//! the row names and answers, before consent, what this box would fetch and how it can be
//! verified; `install` fetches, verifies, unpacks into staging, promotes in one rename, and asks
//! the **probe** what the box can now run (`R-AGT-6`: nothing here writes a status). Nothing here
//! knows an agent's name (`R-AGT-5`): every coordinate is the row's `install` block, the
//! registry document, and `probe::install_root`.

pub mod archive; pub mod fetch; pub mod http; pub mod layout; pub mod manifest; pub mod plan;
pub mod registry; pub mod run;

pub use archive::ArchiveFormat;
pub use manifest::{Consent, InstallRecord, Manifest};
pub use plan::plan;
pub use registry::{BinaryEntry, Distribution, RegistryAgent, RegistryDocument, RegistrySource};
pub use run::install;

/// The ACP registry's `latest` directory: `<base>/registry.json` is the document.
pub const REGISTRY_BASE: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest";
/// D11: refuse when `available < DISK_HEADROOM_FACTOR * content_length`.
pub const DISK_HEADROOM_FACTOR: u64 = 4;
/// D18: at most one progress frame per this; `event_loop::TICK` is the same 250 ms.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// D16: `.staging/` entries older than this are swept at the next install.
pub const STAGING_MAX_AGE: Duration = Duration::from_secs(60 * 60);
/// `cache-control: max-age` fallback when the CDN sends none.
pub const REGISTRY_DEFAULT_MAX_AGE: Duration = Duration::from_secs(300);

/// Everything an installer is told about the box that is not in `ProbeEnv` (plan D18).
/// Production is `Default`; a test injects the fixture server and a `tempdir` root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallConfig {
    /// `REGISTRY_BASE`, or `http://127.0.0.1:<port>` in a test.
    pub registry_base: String,
    /// When `Some`, inserted into `ProbeEnv.vars` as `INSTALL_ROOT_VAR` before anything reads
    /// it — the `set_var`-free way to keep a test off the maintainer's real root.
    pub root_override: Option<PathBuf>,
    /// D11's free-space input when `Some`; `None` asks `fs4::available_space` (T5; T4's default
    /// answers "unknown").
    pub disk_override: Option<u64>,
    pub registry_timeout: Duration,      // 15 s total (registry GET and HEAD)
    pub connect_timeout: Duration,       // 15 s
    pub read_timeout: Duration,          // 60 s between chunks
    pub progress_every: Duration,        // PROGRESS_EVERY
    pub staging_max_age: Duration,       // STAGING_MAX_AGE
    pub headroom_factor: u64,            // DISK_HEADROOM_FACTOR
}
impl Default for InstallConfig { /* the constants above, no overrides */ }
impl InstallConfig {
    /// `Default` with the two knobs every test sets.
    #[must_use] pub fn new(registry_base: impl Into<String>, root_override: Option<PathBuf>) -> Self;
    /// `env` with `root_override` applied (D18): the one place the override becomes a `vars` entry.
    #[must_use] pub fn apply_to(&self, env: ProbeEnv) -> ProbeEnv;
}

/// The client and the config, built once per task (P-13).
#[derive(Debug)]
pub struct Installer { http: http::HttpClient, config: InstallConfig }
impl Installer {
    /// # Errors — `InstallError::Http` when the client cannot be built.
    pub fn new(config: InstallConfig) -> Result<Self, InstallError>;
    #[must_use] pub fn config(&self) -> &InstallConfig;
}

wire_enum!(
    /// What the progress cell says (plan D19).
    InstallPhase {
        Planning => "planning", Downloading => "downloading", Verifying => "verifying",
        Unpacking => "unpacking", Probing => "probing",
    }
);

/// One progress frame as the sink receives it (P-11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallProgress { pub phase: InstallPhase, pub done: u64, pub total: Option<u64> }

/// D18's "at most every 250 ms", as a pure decision over an injected instant: a phase change and
/// a phase's final frame (`done == total`) are always admitted, anything else only when
/// `every` has elapsed since the last admitted frame.
#[derive(Debug)]
pub struct Throttle { every: Duration, last: Option<(InstallPhase, Instant)> }
impl Throttle {
    #[must_use] pub fn new(every: Duration) -> Self;
    pub fn admit(&mut self, frame: InstallProgress, now: Instant) -> bool;
}

/// The pre-flight's product and the consent evidence (plan D12): what `y` says yes to is
/// exactly what `install` executes. Serialisable so `StoreRequest::InstallConfirm` can carry it
/// back unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPlan {
    pub agent_id: AgentId,
    pub agent_name: String,
    /// `Install.tool`.
    pub tool: String,
    pub registry_id: String,
    pub registry_name: String,
    pub version: String,
    /// `platform_key()` at plan time.
    pub platform: String,
    pub archive_url: String,
    pub format: ArchiveFormat,
    /// From the `HEAD`'s `content-length` **header**; `None` when the `HEAD` failed or carried none.
    pub content_length: Option<u64>,
    /// Lowercase hex when the entry publishes one.
    pub sha256: Option<String>,
    /// As the registry spells it (`./agy_acp_server.par`); `cmd_relative` normalises it at run time.
    pub cmd: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub root: PathBuf,
    /// `<root>/<registry_id>/<version>/`.
    pub install_dir: PathBuf,
    /// Directory names under `<root>/<registry_id>/` at plan time (the retention the consent lists).
    pub existing_versions: Vec<String>,
    pub available_bytes: Option<u64>,
    /// `headroom_factor * content_length` when both are known.
    pub need_bytes: Option<u64>,
    /// The registry's `args` differ from the row's `PlatformGlob.args` for this platform.
    pub args_differ: bool,
    /// The manifest's recorded consent when it covers this entry's `license`/`license_url`
    /// (plan D17); `None` means the pane says the terms are accepted by `y`.
    pub consent: Option<Consent>,
    /// The manifest's record for this same version, when it was installed before (D2, D17).
    pub recorded: Option<InstallRecord>,
    /// `Some(age)` when the registry was read from the cache after a network failure.
    pub registry_cached_age_secs: Option<u64>,
    pub planned_at: DateTime<Utc>,
}
impl InstallPlan {
    /// Every D13 line, in order, ready to draw — the wording lives here so T4 tests it offline
    /// and the section composes nothing.
    #[must_use] pub fn consent_lines(&self) -> Vec<String>;
    /// `sha256 published: verified before unpacking` or
    /// `none published: htui cannot verify this download and will record what it receives`.
    #[must_use] pub fn digest_sentence(&self) -> &'static str;
    /// D20's fallback derived from this plan.
    #[must_use] pub fn manual_steps(&self, registry_base: &str) -> ManualSteps;
}

/// D20: everything the user needs to do by hand, every word derived from the row and the helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualSteps {
    pub registry_url: String,      // `<base>/registry.json`
    pub id: String,
    pub platform: String,
    pub version: Option<String>,   // `None` renders `<version>`
    pub unpack_into: PathBuf,      // `<root>/<id>/<version>/`
    pub cmd: Option<String>,       // the file to make executable, when known
    pub override_key: String,      // `tools::env_override_key(tool)`
}
impl ManualSteps { #[must_use] pub fn lines(&self) -> Vec<String>; }

/// Why there is no plan (plan D12, D20, P-9). Each variant is rendered differently.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("nothing declares how to install `{agent}`")]
    NoSource { agent: String },
    #[error("`{agent}` names the tool `{tool}`, which its discovery does not declare as a glob")]
    NoGlobTool { agent: String, tool: String },
    #[error("{var} is unset and this box has no local data directory")]
    NoRoot { var: &'static str },
    #[error("the registry lists no entry `{id}`")]
    UnknownId { id: String },
    #[error("`{id}` {version} is not available for {platform}")]
    NotAvailable { id: String, version: String, platform: String },
    #[error("`{url}` is not a .zip, .tar.gz or .tgz archive; not installable from this entry")]
    Unsupported { url: String },
    #[error("the entry's cmd `{cmd}` is not a relative path inside the archive")]
    BadCmd { cmd: String },
    #[error("{installed} is already installed at {path} and outranks {offered}; remove it to install this version")]
    Outranked { installed: String, offered: String, path: PathBuf },
    #[error("{need} bytes needed ({factor}× the archive), {available} available under {root}")]
    Disk { need: u64, available: u64, factor: u64, root: PathBuf },
    #[error("the registry document does not parse: {message}")]
    Malformed { message: String },
    #[error("{message}")]
    Network { message: String, manual: Box<ManualSteps> },
}

/// Why the pipeline stopped short of the probe. `Ok(InstallOutcome)` covers everything the probe
/// got to say, including "no".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    #[error("cancelled")]
    Cancelled,
    #[error("{message}")]
    Network { message: String, manual: Box<ManualSteps> },
    #[error("HTTP client: {0}")]
    Http(String),
    #[error("sha256 mismatch: the registry publishes {expected}, the download is {computed}; nothing was unpacked")]
    DigestMismatch { expected: String, computed: String },
    #[error("the archive is refused: {message}")]
    Archive { message: String },
    #[error("{what}: {message}")]
    Io { what: String, message: String },
    #[error("promoted {promoted} but the row's glob resolves {resolved:?}; rolled back")]
    NotWhereTheRowLooks { promoted: PathBuf, resolved: Option<PathBuf> },
    #[error(transparent)]
    Driver(#[from] DriverError),
}

/// What the pipeline ended with, once the probe has spoken (plan D16).
#[derive(Debug, Clone, PartialEq)]
pub enum InstallOutcome {
    /// D16(a): promoted, re-probed `ready`/`unauthenticated`, siblings removed, manifest written.
    Installed {
        record: InstallRecord,
        version: String,
        dir: PathBuf,
        /// What the probe wrote; the caller upserts it.
        row: AgentBox,
        status: ProbeStatus,
        removed_versions: Vec<String>,
        /// D2/D17: a same-version re-install whose digest differs from the recorded one.
        digest_changed: bool,
    },
    /// D16(b)/(c): the re-probe said `missing`/`failed`.
    Failed {
        record: InstallRecord,
        version: String,
        status: ProbeStatus,
        stderr_tail: Option<Vec<String>>,
        /// (b): the version put back, and the row of the second re-probe; (c): `None`, tree left.
        restored: Option<String>,
        /// The last probe's answer, for the caller to write (`Kept` writes nothing, D51).
        probe: ProbeOutcome,
    },
}

/// The row-side inputs of [`install`] (P-1).
pub struct InstallJob<'a> {
    pub plan: &'a InstallPlan,
    pub agent: &'a Agent,
    pub box_id: BoxId,
    pub existing: Option<&'a AgentBox>,
    pub ctx: &'a ProbeContext,
    pub tier2: &'a dyn Tier2,
}
```

`lib.rs`: `pub use install::{DISK_HEADROOM_FACTOR, InstallConfig, InstallError, InstallJob, InstallOutcome, InstallPhase, InstallPlan, InstallProgress, Installer, ManualSteps, PlanError, REGISTRY_BASE, install, plan as plan_install};` (`plan` alone is too bare at the crate root; the `resolve_tools` precedent).

### B.4 `install/http.rs` (D9, both fact-check amendments)

```rust
//! The one file that names `reqwest`. Two clients from one provider install (P-7).

static PROVIDER: std::sync::Once = std::sync::Once::new();

/// D9 as amended: `reqwest`'s `rustls-no-provider` does not infer the single enabled provider
/// and `Client::build` panics without one. `install_default`'s `Err` (a provider already
/// installed by this or any other crate) is ignored on purpose. T4's "a client builds" case
/// is what fails the day this call goes.
fn install_crypto_provider() {
    PROVIDER.call_once(|| { let _ = rustls::crypto::ring::default_provider().install_default(); });
}

pub(crate) struct HttpClient { short: reqwest::Client, long: reqwest::Client }
impl HttpClient {
    pub(crate) fn new(config: &InstallConfig) -> Result<Self, InstallError>;
    // short: .timeout(registry_timeout).connect_timeout(connect_timeout)
    // long:  .connect_timeout(connect_timeout).read_timeout(read_timeout)      — no .timeout
    // both:  .user_agent("htui/<CARGO_PKG_VERSION>"), default redirect policy (10 hops)

    /// `GET url` with `If-None-Match` when given. `304` → `NotModified`.
    pub(crate) async fn get_registry(&self, url: &str, if_none_match: Option<&str>) -> Result<RegistryFetch, HttpError>;
    /// `HEAD url`, redirects followed; the size from the **header**, never `Response::content_length()`.
    pub(crate) async fn head(&self, url: &str) -> Result<HeadInfo, HttpError>;
    /// `GET url` on the long client, the body as a chunk stream (`Response::bytes_stream`).
    pub(crate) async fn stream(&self, url: &str) -> Result<(HeadInfo, ByteStream), HttpError>;
}

pub(crate) enum RegistryFetch {
    NotModified,
    Body { bytes: Vec<u8>, etag: Option<String>, max_age: Option<Duration> },
}
pub(crate) struct HeadInfo { pub status: u16, pub content_length: Option<u64>, pub final_url: String }
pub(crate) type ByteStream = Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>;

/// The fact-check trap made a function with a test: `headers.get(CONTENT_LENGTH)` parsed as
/// `u64`, or `None`. On a `HEAD`, `Response::content_length()` reports the empty body's `0`.
pub(crate) fn content_length_header(headers: &reqwest::header::HeaderMap) -> Option<u64>;
/// `cache-control: …max-age=N…` → `Duration`, or `None`.
pub(crate) fn max_age_header(headers: &reqwest::header::HeaderMap) -> Option<Duration>;

/// Transport-level failure, mapped to `PlanError::Network` / `InstallError::Network` by the callers.
#[derive(Debug)] pub(crate) struct HttpError { pub message: String, pub status: Option<u16> }
```

`bytes` is a transitive dependency of `reqwest`; naming its type in a `pub(crate)` alias needs it as a direct dependency — add `bytes = "1"` beside `reqwest` in T4 (**decided here**; the alternative, `Vec<u8>` per chunk, copies every chunk of a 682 MB body).

### B.5 `install/registry.rs` (D12)

```rust
/// The document's shape, owned, every optional field defaulted, unknown keys ignored (serde's
/// default). `#[serde(default)]` at struct level everywhere but `id`/`archive`/`cmd`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RegistryDocument { pub version: String, pub agents: Vec<RegistryAgent> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryAgent {
    pub id: String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub version: String,
    #[serde(default)] pub license: Option<String>,
    #[serde(default)] pub license_url: Option<String>,
    #[serde(default)] pub distribution: Distribution,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Distribution { pub binary: BTreeMap<String, BinaryEntry> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryEntry {
    pub archive: String,
    pub cmd: String,
    #[serde(default)] pub args: Vec<String>,
    #[serde(default)] pub env: BTreeMap<String, String>,
    #[serde(default)] pub sha256: Option<String>,
}
impl RegistryDocument { #[must_use] pub fn agent(&self, id: &str) -> Option<&RegistryAgent>; }

/// Where the document came from, for the consent text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrySource { Fresh, Cached { age: Duration } }

/// `<root>/.registry/latest.json` + `latest.meta.json` (D12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CacheMeta { pub etag: Option<String>, pub fetched_at: DateTime<Utc>, pub max_age_secs: u64 }

pub(crate) struct RegistryCache { dir: PathBuf }
impl RegistryCache {
    pub(crate) fn new(root: &Path) -> Self;                       // `<root>/.registry`
    pub(crate) async fn load(&self) -> Option<(RegistryDocument, CacheMeta)>;  // absent/unparsable → None
    pub(crate) async fn store(&self, bytes: &[u8], meta: &CacheMeta) -> Result<(), InstallError>;   // tmp-then-rename, both files
}

/// The read, in D12's order: within `max_age` → cache, **no request**; past it → conditional GET
/// (`304` refreshes `fetched_at` and keeps the body); a network failure with a cache → `Cached
/// { age }`; a network failure with none → `PlanError::Network` (the caller attaches `ManualSteps`).
pub(crate) async fn read_registry(
    http: &HttpClient, config: &InstallConfig, root: &Path, now: DateTime<Utc>,
) -> Result<(RegistryDocument, RegistrySource), PlanError>;
```

### B.6 `install/plan.rs` (D11, D13, P-9)

```rust
/// The pre-flight (plan D13): exactly one registry read (or none, from cache) and one `HEAD`.
/// **No archive body byte.** In this order, each a refusal before the next costs anything:
/// 1. `agent.launch` → `Discovery.install` (`NoSource`), `discovery.tools[tool]` is a `Glob`
///    (`NoGlobTool`); `install_root(env)` (`NoRoot`).
/// 2. `read_registry`; `document.agent(id)` (`UnknownId`); `distribution.binary[env.platform]`
///    (`NotAvailable`); `ArchiveFormat::for_url` (`Unsupported`); `cmd_relative` (`BadCmd`).
/// 3. `Layout::existing_versions(id)`; any that parses as semver **above** the entry's version
///    is `Outranked` (P-9).
/// 4. `Manifest::load`: `consent` when `consent_covers`, `recorded` for this version.
/// 5. `HEAD`: `content_length` from the header, or `None` and a plan.
/// 6. `available_bytes` = `config.disk_override` or `fs4::available_space(root)` (T5; T4 answers
///    `None`); `need = factor * content_length`; `available < need` → `Disk`.
/// 7. `args_differ` = entry.args != the row's `PlatformGlob.args` for `env.platform`.
pub async fn plan(
    installer: &Installer, agent: &Agent, env: &ProbeEnv, now: DateTime<Utc>,
) -> Result<InstallPlan, PlanError>;

/// `ArchiveFormat` from the URL's **path** (query and fragment stripped): `.zip`, `.tar.gz`, `.tgz`.
impl ArchiveFormat { #[must_use] pub fn for_url(url: &str) -> Option<Self>; }

/// `./x` → `x`; refuses absolute paths, `..`, and empty components. Checked at plan time (before
/// consent) and again at run time (the plan is data the user could have edited).
pub(crate) fn cmd_relative(cmd: &str) -> Result<PathBuf, PlanError>;
```

`fs4::available_space(path) -> io::Result<u64>` — **UNVERIFIED — implementer must check** the free function's name in `fs4 1.1.0` (it is `fs4::available_space` in 0.x; the crate was not in the local registry). It is sync and runs under `spawn_blocking`.

### B.7 `install/fetch.rs`, `archive.rs`, `layout.rs`, `manifest.rs` (D10, D16, D17)

```rust
// fetch.rs
pub(crate) struct Downloaded { pub path: PathBuf, pub sha256: String, pub bytes: u64 }

/// Streams `plan.archive_url` into `into`, hashing every chunk (`sha2::Sha256`, lowercase hex —
/// `identity.rs:142`'s digest). Progress: `Downloading { done, total: content_length }` through
/// `throttle.admit(frame, Instant::now())`. Cancellation: `tokio::select!` between the next chunk
/// and `cancel.cancelled()`; on cancel the partial file is removed and `Cancelled` returned. A
/// read past `read_timeout` is `Network` with `ManualSteps`. The file is written through
/// `tokio::fs::File` + `write_all`; `sync_all` before returning.
pub(crate) async fn download(
    http: &HttpClient, plan: &InstallPlan, into: &Path, config: &InstallConfig,
    throttle: &mut Throttle, progress: &mut dyn FnMut(InstallProgress), cancel: &CancellationToken,
) -> Result<Downloaded, InstallError>;

// archive.rs
wire_enum!(ArchiveFormat { Zip => "zip", TarGz => "tar.gz" });

/// **Synchronous**; the caller runs it under `spawn_blocking`. Reads the archive from disk (never a
/// stream: a zip's directory is at its end, and the digest was checked over the whole file first).
/// Per entry: `safe_target` (absolute, `..`, or a symlink whose target leaves `into` → `Archive`),
/// the archive's own unix mode applied where it carries one (`ZipFile::unix_mode()`,
/// `Header::mode()`), `written` bumped by the entry's size for the progress poller, `cancel`
/// checked **before each entry** — the one way a blocking thread stops (P-3).
pub(crate) fn unpack(
    archive: &Path, into: &Path, format: ArchiveFormat,
    cancel: &CancellationToken, written: &AtomicU64,
) -> Result<(), InstallError>;

/// `into.join(entry)` after `zip`'s `enclosed_name()` / our own component check for tar
/// (`Entry::unpack_in` returns `Ok(false)` for an escaping path — used as the second guard, not
/// the first; **UNVERIFIED** whether it also validates a symlink's *target*, which is why the
/// target check is ours in both formats).
fn safe_target(into: &Path, entry: &Path) -> Result<PathBuf, InstallError>;

/// `cfg(unix)`: mode |= 0o111 (the README's `chmod +x` made structural — `launch::spawn` runs
/// `which` even on an absolute path, `launch.rs:743`). `cfg(windows)`: no-op, `Ok(())`.
pub(crate) fn make_executable(path: &Path) -> std::io::Result<()>;

// layout.rs
/// Every path under the install root, in one place, so the "staging is never walkable" property
/// (D16: `.staging/` and `.registry/` are siblings of `<id>/`, and the seed pattern's literal root
/// ends at `<id>`) is a fact about this struct and not about six call sites.
pub(crate) struct Layout { root: PathBuf }
impl Layout {
    pub(crate) fn new(root: PathBuf) -> Self;
    pub(crate) fn staging(&self) -> PathBuf;                                  // <root>/.staging
    pub(crate) fn agent_dir(&self, id: &str) -> PathBuf;                      // <root>/<id>
    pub(crate) fn version_dir(&self, id: &str, version: &str) -> PathBuf;     // <root>/<id>/<version>
    pub(crate) fn manifest(&self, id: &str) -> PathBuf;                       // <root>/<id>/manifest.json
    pub(crate) fn staging_archive(&self, id: &str, version: &str, nonce: &str) -> PathBuf;  // .staging/<id>-<version>-<nonce>.archive
    pub(crate) fn staging_tree(&self, id: &str, version: &str, nonce: &str) -> PathBuf;     // .staging/<id>-<version>-<nonce>/
    pub(crate) fn staging_previous(&self, id: &str, version: &str, nonce: &str) -> PathBuf; // .staging/<id>-<version>-<nonce>.previous/
    /// Directory names under `<root>/<id>/`, `manifest.json` excluded; empty when absent.
    pub(crate) async fn existing_versions(&self, id: &str) -> Result<Vec<String>, InstallError>;
    /// D16 + P-10: entries older than `max_age` are removed — except a `.previous` whose
    /// `<root>/<id>/<version>/` is **absent**, which is restored instead (the working version an
    /// aborted install had set aside). Names are parsed back as `<id>-<version>-<nonce>[.previous|.archive]`;
    /// the `id` and `version` are the trailing-nonce split, never a vendor string.
    pub(crate) async fn sweep(&self, max_age: Duration, now: SystemTime) -> Result<SweepReport, InstallError>;
    /// Same-version re-install: `<id>/<version>/` → `.staging/….previous/`; `Ok(None)` when absent.
    pub(crate) async fn set_aside(&self, id: &str, version: &str, nonce: &str) -> Result<Option<PathBuf>, InstallError>;
    /// One `rename`; on Windows retried three times with 200/400/800 ms backoff on `PermissionDenied` (D21).
    pub(crate) async fn promote(&self, staged: &Path, id: &str, version: &str) -> Result<PathBuf, InstallError>;
    pub(crate) async fn restore(&self, previous: &Path, id: &str, version: &str) -> Result<(), InstallError>;
    pub(crate) async fn remove_version(&self, id: &str, version: &str) -> Result<(), InstallError>;
    /// D16(a): every version directory but `keep`; answers what it removed.
    pub(crate) async fn retain_only(&self, id: &str, keep: &str) -> Result<Vec<String>, InstallError>;
}
/// `<pid>-<micros>`: unique per process per microsecond, no new dependency (**decided here**).
pub(crate) fn nonce() -> String;
#[derive(Debug, Default, PartialEq, Eq)] pub(crate) struct SweepReport { pub removed: Vec<PathBuf>, pub restored: Vec<PathBuf> }

// manifest.rs (D17)
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest { pub consent: Option<Consent>, pub installs: BTreeMap<String, InstallRecord> }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consent { pub license: Option<String>, pub license_url: Option<String>, pub accepted_at: DateTime<Utc>, pub version: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallRecord { pub sha256: String, pub published: bool, pub archive: String, pub platform: String, pub installed_at: DateTime<Utc> }
impl Manifest {
    /// Absent → `Default`; unparsable → `Default` with a `warn!` (a hand-edited file must not block an install).
    pub async fn load(path: &Path) -> Self;
    /// `<name>.<nonce>.tmp` then `rename` (`identity.rs:104-124`).
    pub async fn store(&self, path: &Path) -> Result<(), InstallError>;
    /// D17: recorded and the `(license, license_url)` pair equals the plan's.
    #[must_use] pub fn consent_covers(&self, license: Option<&str>, license_url: Option<&str>) -> bool;
}
```

### B.8 Windows-specific shapes (D21, written here, verified by MOD-16)

`make_executable` is `cfg(unix)`; `unpack` applies `unix_mode()`/`mode()` under `cfg(unix)` only; `promote`'s retry is unconditional code with a Windows-only reason; both sides of the post-promote check go through `tokio::fs::canonicalize` (the `\\?\` prefix); the registry's `./agy_acp_server.exe` is `cmd_relative`'d like any other; `%HTUI_AGENTS_ROOT%` is looked up through `ProbeEnv::var`'s case-insensitive branch. Deferred to MOD-16 by name: the Defender retry, long paths, `PATHEXT` for a `.cmd` `cmd`, `system-proxy` reading the registry, `fs4` on a junction.

### B.9 `install/run.rs` (D16, T6)

```rust
/// The pipeline, one call, in this order. Every `?` before promotion leaves nothing the glob can
/// resolve (the tree is under `.staging/`); every failure after it is rolled back per D16.
///
///  0. `Layout::sweep` (P-10 rule). `nonce()`.
///  1. `download` → `.staging/<id>-<v>-<n>.archive`  (`Downloading` frames)
///  2. `Verifying`: `plan.sha256` `Some(expected)` and `expected != computed` → `DigestMismatch`
///     (archive removed, nothing unpacked). `None` → the computed digest is the record's
///     (`published: false`). `plan.recorded` present with a different digest → `digest_changed`.
///  3. `Unpacking`: `spawn_blocking(unpack(..))`, the `written` counter polled every
///     `progress_every` while the handle is awaited (`timeout(progress_every, &mut handle)` in a loop);
///     then `make_executable(tree/cmd_relative)`; the tree must contain the `cmd` (`Archive` otherwise).
///  4. `set_aside(id, version)` when the same version exists.
///  5. `promote` → `<root>/<id>/<version>/`. From here `rollback` is armed.
///  6. The post-promote check: `resolve_tool(&discovery.tools[plan.tool], &ctx.env)` must answer a
///     path whose `canonicalize` equals `canonicalize(dir/cmd)`; otherwise `NotWhereTheRowLooks`
///     and rollback (the promoted tree removed, `.previous` restored if any, the row re-probed so
///     `agent_box` describes what is left — D16(b)'s shape).
///  7. `Probing`: `probe_agent(agent, box_id, existing, ctx, tier2)`. The installer reads the
///     status off the returned row (`ProbeSnapshot::from_row`) and never sets one.
///  8. `ready`/`unauthenticated` → `retain_only(id, version)` (this removes the `.previous` too),
///     manifest `installs[version] = record`, `consent = Consent { .., accepted_at: ctx.now }`
///     when `plan.consent` was `None`, `Manifest::store` → `Installed`.
///     `missing`/`failed` with a previous → restore, remove promoted, `probe_agent` again → `Failed
///     { restored: Some(prev) }`. Without one → tree left, `Failed { restored: None }`.
///     `ProbeOutcome::Kept` counts as "the probe found nothing" (a manual row): the (b)/(c) split
///     applies and `row` is what `Kept` implies — nothing to write.
///  9. Cancellation: `cancel.cancelled()` observed by `download`'s `select!`, by `unpack` per entry,
///     and checked once between every numbered step; before step 5 it removes the staging entry
///     and answers `Cancelled`; **after** promotion it is ignored — a promoted tree is committed
///     and a re-probe is seconds, not minutes.
pub async fn install(
    installer: &Installer,
    job: InstallJob<'_>,
    progress: &mut dyn FnMut(InstallProgress),
    cancel: &CancellationToken,
) -> Result<InstallOutcome, InstallError>;
```

Wait-free by construction: nothing in `run.rs` holds a `std::sync::Mutex` across an `.await`; the only shared state is the `AtomicU64` written by the blocking thread.

### B.10 `crates/htui/src/store_worker.rs` (D18)

```rust
pub enum StoreRequest {
    /* existing */
    /// Pre-flight an adapter install for one registry row (MOD-20 D13, D18): one registry read,
    /// one `HEAD`, no archive byte. Served by the agent runtime's own task; answered once with
    /// [`StoreReply::Install`]`(`[`InstallFrame::Plan`]`)` or [`StoreReply::Failed`].
    InstallPlan { agent_id: AgentId },
    /// The user said `y` to exactly this plan (MOD-20 D12: the plan is the consent evidence).
    /// Many replies, all at this request's `seq`: `Progress` frames, then `Done`, `Cancelled` or
    /// `Failed`.
    InstallConfirm { plan: Box<InstallPlan> },
    /// Stop the running install. Answered `Cancelling` at once; the running stream ends `Cancelled`.
    InstallCancel,
}
// name(): "install_plan" | "install_confirm" | "install_cancel"

pub enum StoreReply {
    /* existing */
    /// One frame of an install (MOD-20 D18), the `Chat(ChatFrame)` shape.
    Install(InstallFrame),
}

/// One frame of an install stream.
#[derive(Debug, Clone)]
pub enum InstallFrame {
    Plan(Box<InstallPlan>),
    Progress { phase: InstallPhase, done: u64, total: Option<u64> },
    Done(Box<InstallOutcome>),
    Cancelling,
    Cancelled,
    Failed { message: String, manual: Option<Box<ManualSteps>> },
}
```

`try_serve` (`:343-350`): the three join the "no agent runtime in this build" arm. Loop (`:484-488`): the three join the runtime arm — `Served::Deferred => continue` already does the right thing. `testkit.rs:181-186`: the same three.

### B.11 `crates/htui/src/agent_worker.rs` (D18)

```rust
/// The one install this runtime allows at a time (plan D18): planning and installing are the same
/// claim, so a second `i` during either is refused by name.
pub struct LiveInstall {
    agent_id: AgentId,
    phase: LivePhase,                 // Planning | Installing
    cancel: CancellationToken,
    task: JoinHandle<()>,
}
// hand-written Debug: agent_id, phase, finished

pub struct AgentRuntime {
    /* existing */
    /// `None` until `with_installer` or `production()`: a `new()` runtime refuses `i` with
    /// "this runtime has no installer", so no test reaches the network by accident.
    installer: Option<InstallConfig>,
    install: Option<LiveInstall>,
}

impl AgentRuntime {
    // production(): installer = Some(InstallConfig::default())
    /// The injected seam of D18: the fixture server and a `tempdir` root.
    #[must_use] pub fn with_installer(mut self, config: InstallConfig) -> Self;
    /// Whether an install (plan or confirm) is running; tests.
    #[must_use] pub fn install_running(&self) -> bool;

    // serve(): after the two `retain`s, `self.install.take_if(|live| live.task.is_finished());`
    //   StoreRequest::InstallPlan { agent_id } => match self.install_plan(backend, replies, addr, *agent_id).await { Ok(s) => s, Err(e) => Served::Reply(failed("install_plan", &e)) }
    //   StoreRequest::InstallConfirm { plan }  => match self.install_confirm(backend, replies, addr, plan.clone()).await { .. "install_confirm" }
    //   StoreRequest::InstallCancel            => self.install_cancel(addr)

    /// D18's refusals, in the probe's order and **before anything is spawned**: no installer →
    /// `Backend("this runtime has no installer")`; `writer()` `None`/`Buffered` →
    /// `Unreachable(REGISTRY_ON_SERVER_ONLY)` (at plan time, so no network is spent on an install
    /// that cannot be written); `box_info()` `None` → `NotFound { "box", .. }`; `self.install`
    /// `Some` → `Backend("an install is already running for <agent>")`; the row (from
    /// `backend.agents()`) absent → `NotFound { "agent", id }`; its `launch.discovery.install`
    /// `None` → `Backend(PlanError::NoSource.to_string())`. Then `tokio::spawn(run_plan(..))`,
    /// `self.install = Some(LiveInstall { phase: Planning, .. })`, `Ok(Served::Deferred)`.
    async fn install_plan(&mut self, backend: &Backend, replies: &Sender, addr: ReplyAddr, agent_id: AgentId) -> Result<Served, StoreError>;

    /// The same refusals minus the plan's; the writer is **taken here** and moves into the task
    /// (`agent_worker.rs:379-413`'s ownership). `self.install = Some(LiveInstall { phase: Installing, .. })`.
    async fn install_confirm(&mut self, backend: &Backend, replies: &Sender, addr: ReplyAddr, plan: Box<InstallPlan>) -> Result<Served, StoreError>;

    /// `cancel.cancel()` on the live install and `Served::Reply(Install(Cancelling))`; with none,
    /// `Failed { "install_cancel", "no install is running" }`. The task ends its own stream.
    fn install_cancel(&mut self, addr: ReplyAddr) -> Served;

    // finish_background: also `if let Some(live) = self.install.take() { timeout(limit, live.task) … abort }`
    // shutdown: after the chats, `if let Some(live) = self.install.take() { live.cancel.cancel(); live.task.abort(); }` (P-3), then the background aborts as today
}

struct PlanArgs { config: InstallConfig, agent: Agent, cwd: PathBuf, frames: Frames }
struct InstallArgs { config: InstallConfig, plan: Box<InstallPlan>, writer: Writer, box_id: BoxId, agent: Agent, existing: Option<AgentBox>, cwd: PathBuf, cancel: CancellationToken, frames: Frames }

/// `Installer::new(config)` → `ProbeEnv::host(cwd)` through `config.apply_to` → `plan(..)`.
/// `Ok` → `Install(Plan(Box))`; `Err(PlanError::Network { manual, .. })` → `Install(Failed { manual: Some })`;
/// any other `Err` → `Failed { request: "install_plan", message }` (the section's status line).
/// Cancelled during planning (the token) → `Install(Cancelled)`.
async fn run_plan(args: PlanArgs, cancel: CancellationToken);

/// `Installer::new` → env as above → `install(installer, InstallJob { .., tier2: &SpawnTier2::default() },
/// &mut |p| frames.reply(&addr, Install(Progress { phase: p.phase, done: p.done, total: p.total })), &cancel)`.
/// The outcome's row (`Installed.row`, or `Failed.probe` when `Row`) is written with
/// `writer.upsert_agent_box` **before** the `Done` frame, so the `Agents` read the section issues on
/// `Done` sees it. A write failure is `Failed { message }` at the same address.
/// `Err(Cancelled)` → `Install(Cancelled)`; `Err(Network { manual, .. })` → `Install(Failed { manual: Some })`;
/// other `Err` → `Install(Failed { manual: None })`.
async fn run_install(args: InstallArgs);
```

Both tasks are plain `async fn`s taking an owned args struct (the `run_chat` rule), so the harness can poll them; production spawns them. `Frames::reply` (`:951`) is reused unchanged.

### B.12 `crates/htui/src/ui/tabs/settings/agents.rs` (D19, D20)

```rust
pub struct AgentsSection {
    agents: Vec<AgentSummary>,
    unavailable: Option<String>,
    probing: bool,
    /// The first row cursor in a Settings section (plan D19). `TableState` is ratatui's own.
    cursor: TableState,
    install: InstallState,
    /// The last outcome, one line on the hint row (P-12).
    notice: Option<String>,
}

enum InstallState {
    Idle,
    Planning { agent_id: AgentId },
    Pending { plan: Box<InstallPlan> },
    Running { agent_id: AgentId, phase: InstallPhase, done: u64, total: Option<u64>, cancelling: bool },
    Manual { steps: ManualSteps, message: String },
}
```

Keys (`on_key`), after the tab has consumed `h`/`l`/`[`/`]`/arrows and the global table `q ? digits ctrl-c -`:

| State | Key | Effect |
|---|---|---|
| any but `Pending` | `j`/`k` | move the cursor, no wrap, clamped to `agents.len()` |
| `Idle`/`Manual` | `i` | refused (`Action::Error`) while `probing` ("a probe is running") or when the row's `launch.discovery.install` is `None` ("nothing declares how to install `<name>`" — the row is parsed here, no I/O); otherwise `install = Planning`, `ctx.request(StoreRequest::InstallPlan { agent_id })` |
| `Planning`/`Running` | `i` | `Action::Error("an install is already running")` |
| `Planning`/`Running` | `r` | `Action::Error("an install is running; probe afterwards")` |
| `Pending` | `y` | `install = Running { phase: Planning-cell text, .. }`, `ctx.request(InstallConfirm { plan })` |
| `Pending` | `n`/`Esc` | `install = Idle`, `notice = Some("install declined")` |
| `Pending` | anything else | `Handled::Consumed` (swallowed, D19) |
| `Running` | `x` | `cancelling = true`, `ctx.request(InstallCancel)` |
| `Manual` | `Esc` | `install = Idle` |

`on_reply`: `Install(Plan(p))` → `Pending`; `Install(Progress { .. })` → update `Running`; `Install(Done(o))` → `Idle`, `notice` from the outcome (`installed <id> <v>` / `install of <v> failed: <first tail line>; <prev> restored` / `… failed; the tree is left at <dir>`), **`ctx.request(StoreRequest::Agents)`**; `Install(Cancelling)` → `cancelling = true`; `Install(Cancelled)` → `Idle`, `notice = Some("install cancelled")`; `Install(Failed { message, manual: Some(s) })` → `Manual { steps, message }`; `Install(Failed { manual: None })` → `Idle`, notice = message; `Failed { request: "install_plan" | "install_confirm" | "install_cancel", .. }` → `Idle` (the shell already put the message on the status line); `Agents(..)` → as today (**does not** touch `install`).

`on_box_cell` gains, before the `probing` check, one arm: the `Running`/`Planning` row (by `agent_id`) reads `planning…`, `downloading 42%` (`done * 100 / total`, or `downloading 12 MB` when `total` is `None`), `verifying…`, `unpacking…`, `probing…`; `cancelling…` when `cancelling`.

`render`: `Layout::vertical([Min(3), Length(pane), Length(1)])` where `pane` is `consent_lines().len() + 1` for `Pending`, `steps.lines().len() + 1` for `Manual`, `0` otherwise; the table is rendered stateful with the cursor row in `theme.accent`; the hint line reads `j/k select · r probe · i install` / `y install · n cancel` / `x cancel install` / `Esc close`, with `notice` appended after ` · `.

---

## C. The documents, worked

### C.1 `agent_agy.json` after T1 + T2 (`:11-38`)

```json
"discovery": {
  "tools": {
    "agy": { "…": "unchanged" },
    "agy_acp_server": {
      "kind": "glob",
      "patterns": [],
      "platform": {
        "windows-x86_64":  { "patterns": [
          "%LOCALAPPDATA%/JetBrains/*/acp-agents/antigravity-acp/*/agy_acp_server.exe",
          "%HTUI_AGENTS_ROOT%/antigravity-acp/*/agy_acp_server.exe" ] },
        "windows-aarch64": { "patterns": [
          "%LOCALAPPDATA%/JetBrains/*/acp-agents/antigravity-acp/*/agy_acp_server.exe",
          "%HTUI_AGENTS_ROOT%/antigravity-acp/*/agy_acp_server.exe" ] },
        "linux-x86_64":   { "patterns": ["%HTUI_AGENTS_ROOT%/antigravity-acp/*/agy_acp_server.par"], "args": ["--uid="] },
        "linux-aarch64":  { "patterns": ["%HTUI_AGENTS_ROOT%/antigravity-acp/*/agy_acp_server.par"], "args": ["--uid="] },
        "darwin-aarch64": { "patterns": ["%HTUI_AGENTS_ROOT%/antigravity-acp/*/agy_acp_server.par"] }
      }
    }
  },
  "handshake": true,
  "credential": { "…": "unchanged" },
  "install": { "source": "acp_registry", "id": "antigravity-acp", "tool": "agy_acp_server" }
}
```

`windows-aarch64` gains the `htui`-managed pattern it lacked (the registry has a `windows-aarch64` entry and D21 says the seed's Windows patterns are written here) — **decided here**, recorded in the phase note. `expand` on the new pattern: segment 0 = `%HTUI_AGENTS_ROOT%` → `expand_vars` → the root as one segment (a backslashed Windows value stays one `PathBuf::push`); `antigravity-acp` literal → root; `*` → `rest`; the leaf → `rest`. The literal root therefore ends at `<root>/antigravity-acp`, and `<root>/.staging`, `<root>/.registry`, `<root>/antigravity-acp/manifest.json` (a file, not matched by the `*` directory segment's leaf rule) are unreachable — the D16 property.

### C.2 `tests/fixtures/registry.json`

`{ "version": "1.0.0", "agents": [ <antigravity-acp entry verbatim>, <amp-acp entry verbatim> ], "extensions": [] }` — the two entries as fetched on 2026-09-09 (five platforms without `sha256` and `license: "proprietary"`; five platforms with `sha256`, `license: "Apache-2.0"`, `cmd: "./amp-acp"`, Windows `cmd: "amp-acp.exe"` with no `./` — which is why `cmd_relative` accepts both spellings). The fixture server rewrites nothing: tests point `archive` at the fixture by building their **own** registry row JSON in the test with `archive: "http://127.0.0.1:<port>/archive.zip"`; the pinned document is for the parse and `plan()` shape cases only, whose `HEAD` goes to the fixture through a per-test document with the URL substituted.

### C.3 `<root>/<id>/manifest.json`

```json
{
  "consent": { "license": "proprietary", "license_url": "https://…/terms", "accepted_at": "2026-09-09T…Z", "version": "1.1.1" },
  "installs": {
    "1.1.1": { "sha256": "…64 hex…", "published": false, "archive": "https://…linux-x86_64.zip", "platform": "linux-x86_64", "installed_at": "2026-09-09T…Z" }
  }
}
```

### C.4 The consent pane (`InstallPlan::consent_lines`, D13 order)

```
install <registry_name> <version> for <agent_name> (<platform>)
from    <archive_url>  — <size> / size unknown
into    <install_dir>
licence <license> — <license_url>   [accepted on <date>] | [y accepts these terms]
digest  sha256 published: verified before unpacking | none published: htui cannot verify this download and will record what it receives
disk    <available> free, <need> needed (4× the archive) | need unknown
existing <v1>, <v2> replaced and deleted once the new version probes usable | none
args    the registry's args differ from the row's: <a> vs <b>    (only when args_differ)
registry cached <age>                                            (only when cached)
this version was installed before with digest <d>               (only when recorded)
```

---

## D. Data flow and ownership

```
Settings (UI task)                    worker loop (select! arm)                  runtime task(s)
──────────────────                    ─────────────────────────                  ───────────────
i on row
  Planning; ctx.request(InstallPlan{agent_id})   —dispatch(Origin::Tab(settings), seq=n)→
                                      runtime.serve → install_plan:
                                        refusals (installer, Writer, box, claim, row, install block)
                                        spawn(run_plan(PlanArgs)); LiveInstall{Planning}; Deferred → continue
                                                                                  Installer::new (provider Once, 2 clients)
                                                                                  env = config.apply_to(ProbeEnv::host(cwd))
                                                                                  plan(): read_registry (cache/ETag) → entry → format → Outranked? → manifest → HEAD (header) → disk → InstallPlan
                                                                                  frames.reply(addr n, Install(Plan(Box)))
on_reply Install(Plan) → Pending; render consent_lines
y  → Running; ctx.request(InstallConfirm{plan})  —dispatch(seq=m)→
                                      runtime.serve → install_confirm:
                                        refusals; writer taken; spawn(run_install(InstallArgs)); LiveInstall{Installing}; Deferred
                                                                                  install(job, progress→Install(Progress@m), cancel):
                                                                                    sweep .staging (restore .previous / delete >1 h)
                                                                                    download → .staging/<id>-<v>-<n>.archive  (sha256 as it streams)
                                                                                    verify (DigestMismatch stops here; nothing unpacked)
                                                                                    spawn_blocking(unpack → .staging/<id>-<v>-<n>/) ; make_executable
                                                                                    set_aside same version → .previous
                                                                                    promote: rename → <root>/<id>/<v>/
                                                                                    resolve_tool(row's glob) == promoted cmd ? else rollback
                                                                                    probe_agent(SpawnTier2) → AgentBox (status is the probe's)
                                                                                    ready/unauth → retain_only, manifest store → Installed
                                                                                    else → restore prev + remove, probe again → Failed
                                                                                  writer.upsert_agent_box(row)
                                                                                  frames.reply(addr m, Install(Done(Box)))
on_reply Install(Done) → Idle, notice; ctx.request(Agents)  —dispatch(seq=k)→  try_serve → Agents  → on_reply Agents → cell = probe's status
x  → ctx.request(InstallCancel) —seq=c→ runtime.install_cancel: cancel.cancel(); Reply(Install(Cancelling)@c)
                                                                                  download's select!/unpack's per-entry check → Cancelled → staging removed → frames.reply(addr m, Install(Cancelled))
```

Types crossing each boundary: `StoreRequest::InstallPlan { AgentId }` → `PlanArgs` (owned `Agent`, `InstallConfig`, `Frames`) → `InstallPlan` (serialisable value) → `StoreReply::Install(InstallFrame::Plan(Box<InstallPlan>))` → `StoreRequest::InstallConfirm { Box<InstallPlan> }` → `InstallArgs` (owned `Writer`, `CancellationToken`) → `InstallJob<'_>` → `InstallProgress` (→ `InstallFrame::Progress`) → `InstallOutcome` (holds `AgentBox`) → `upsert_agent_box` → `InstallFrame::Done(Box<InstallOutcome>)` → `StoreRequest::Agents` → `StoreReply::Agents(Vec<AgentSummary>)`. `App::is_fresh` passes every `Progress` frame because `latest[(Tab(settings), discriminant(InstallConfirm))] == m` until the section confirms again; `Plan` is fresh under the `InstallPlan` discriminant; `Cancelling` under `InstallCancel`'s.

Ownership that enforces `R-NF-3`: the loop holds `&Backend` and a `CancellationToken` clone; the task holds the `Writer`, the clients, the file handles and the blocking unpack. Nothing in `serve` awaits more than `box_info()`/`agents()` — the same two awaits `probe()` already does on the arm.

---

## E. Build order and file-set intersections

```
T1 ──┬── chain A: T2 ── T3 ──────────────────────────┐
     └── chain B: T4 ── T5 (needs T2) ── T6 (needs T3) ┴── T7 ── T8 ── T10
                                                       └── T9 (after T6; ∥ T7, T8)
```

| Pair | Intersection | Parallel? |
|---|---|---|
| T2 × T4 | ∅ — A: `probe.rs`, `agent_agy.json`, `tests/probe.rs`; B: `Cargo.toml` ×2, `lib.rs`, `install/*`, `tests/install.rs`, `tests/fixtures/registry.json` | **yes** |
| T3 × T4, T3 × T5 | ∅ (as above) | **yes**; T5 waits for T2 (calls `install_root`), T6 waits for T3 (relies on version-aware `newest` in the re-probe) — sequencing notes, not file intersections |
| T1 × T2 | `agent_agy.json`, `tests/probe.rs` | serial (T1 first) |
| T2 × T3 | `probe.rs`, `tests/probe.rs` | serial |
| T4 × T5 × T6 | `install/mod.rs`, `tests/install.rs`, `htui-agent/Cargo.toml` (T4, T5) | serial |
| T7 × T8 | ∅ by file (T7: `store_worker.rs`, `agent_worker.rs`, `testkit.rs`; T8: `settings/agents.rs`, `tests/settings.rs`, `tests/install.rs`, four `.snap`, `htui/Cargo.toml`) but T8 needs T7's types | serial |
| T9 × everything | ∅ (`tests/install_live.rs`) | **yes**, after T6 |

Checkpoints: `cargo fmt --all -- --check` before each commit; after T2, T3, T6: `cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features -- -D warnings` (the `cfg(unix)` arms in `archive.rs` and the retry in `layout.rs` compile on both); after T7: the same for `-p htui`; the Postgres line after T7 (`agent_worker.rs` tests). One commit per task: `feat(agent): Discovery.install, the declared adapter source (D12)` · `feat(agent): HTUI_AGENTS_ROOT owns the install root on every platform (D15)` · `feat(agent): version-aware newest() (D14, amends MOD-2 D48)` · `feat(agent): registry reader and install pre-flight (D9, D11–D13)` · `feat(agent): fetch, verify, unpack, promote, manifest (D10, D16, D17)` · `feat(agent): the install pipeline with re-probe, rollback, retention (D16)` · `feat(tui): install requests served off the loop (D18)` · `feat(tui): Settings > i, consent pane and progress cell (D19, D20)` · `test(agent): amp-acp installed live from a row alone (R-AGT-5)` · `docs(mod-20): close-out (D22)`.

---

## F. Test seams, per layer

Each layer is testable **without the network and without the maintainer's install root**, by these seams and no others:

| Layer | Seam | How |
|---|---|---|
| `Discovery.install` (T1) | serde | `tests/launch.rs`: the three documents; byte-for-byte re-serialisation of a pre-MOD-20 row |
| `install_root` / the token (T2) | `ProbeEnv.vars` injection | `env()` gains `HTUI_AGENTS_ROOT = <tmp>/agents`; the "host seeds it" half asserts `ProbeEnv::host(..).var(INSTALL_ROOT_VAR) == default_install_root()` when the real environment lacks it (`std::env::var_os` read, never written); the "unless set" half goes through `install_root(&hand_built_env)` |
| `newest` (T3) | pure function over `GlobMatch` values with `touch_at` mtimes | the seven plan cases; `walk_reports_one_capture_per_star` over a two-star tree |
| HTTP (T4) | a hand-rolled `tokio::net::TcpListener` HTTP/1.1 responder in `tests/install.rs` that records every request line (`Arc<Mutex<Vec<String>>>`) and serves a scripted table `path → (status, headers, body)`; supports `HEAD` (headers only, real `content-length`), `If-None-Match` → `304`, a body that stalls for N s, a connection refused (a bound-then-dropped port) | `InstallConfig::new(format!("http://{addr}"), Some(tmp.join("agents")))`; the recorder asserts D13's exact sequence |
| the provider (T4) | `Installer::new(config)` in a test | asserts `Ok`; the plan's "removing the call fails with the vendor's panic text" is documented in the test's doc, not asserted (a panic test would need `catch_unwind` over a `Once`) |
| `content_length_header` (T4) | pure | a `HeaderMap` with `content-length: 681969407` → `Some(681969407)`, plus the live-shaped case through the fixture's `HEAD` |
| disk (T4/T5) | `InstallConfig.disk_override` | `Some(1)` with a `content-length: 1000` → `Disk { need: 4000, available: 1 }` |
| the cache (T4) | `now` is a `plan()` argument | second `plan()` at `now + 10 s` → recorder unchanged; at `now + 301 s` → `GET` with `If-None-Match`, `304` |
| fetch / archive / layout / manifest (T5) | archives built at run time under `tempdir` (`zip::ZipWriter` with `unix_permissions(0o755)` on one entry; `tar::Builder` + `flate2::write::GzEncoder`); the kill-mid-install case drops the `install()` future at an injected await point (`tokio::time::timeout(Duration::ZERO, ..)` after the fixture server has served the body) | every path assertion is under `tmp`; the test asserts `root.starts_with(tempdir)` before touching disk (plan Risks) |
| the pipeline (T6) | `Tier2` seam: `DuplexTier2`/`CannedTier2` copied from `tests/probe.rs:900-945` into `tests/install.rs` (the per-file rule) | `ready` from the row; `Canned::Failed` → `Failed` outcome, tree per (b)/(c); the post-promote check with a discovery whose glob points elsewhere |
| progress throttling (T6) | `Throttle::admit(frame, now)` with hand-made `Instant`s | monotonic `done`, ≤ 1 frame per 250 ms, phase changes always through |
| cancellation (T6) | `CancellationToken` tripped by the test between the fixture's chunks (the server stalls after chunk 1; the test cancels; the server is released) | `Err(Cancelled)`, `.staging/` empty, `resolve_tool` → `None` |
| the override (T6) | `vars[HTUI_TOOL_<NAME>] = <tmp>/elsewhere/x` on the job's `ProbeEnv` | after `Installed`, the row's `probe.resolved.command` is the override's path |
| runtime (T7) | `AgentRuntime::production().with_installer(InstallConfig::new(fixture, Some(tmp)))`; `serve()` called directly as `:2407` does; `background_len()`/`install_running()`; a `MemStore` whose row the test writes with an `install` block | the loop-freedom copy of `:1233-1273` with `InstallConfirm` at seq 1 over a stalling body and `Workspaces` at seq 2 |
| section (T8) | `render_section` + `probed_row` (`tests/settings.rs:35`, `:186`); `on_reply` fed `Install(..)` frames by hand; `on_key` with a `Ctx` whose `Emit` is inspected | one snapshot per frame; the consent pane in both digest wordings; `Manual` steps |
| harness (T8) | `Harness::over(store).with_tab(..).with_agent_runtime(runtime.with_installer(..))`, `key("i")`, `drive_to_end()` (awaits the install task through `finish_background`, P-2) | the recorder has seen no `GET /archive` after `i`; the cell reads `failed` after `y` (the fixture `cmd` is a shell script `SpawnTier2` cannot handshake — the broken-tree metric at the app level) |
| live (T9) | `#[ignore]`, `HTUI_AGENTS_ROOT` through `InstallConfig.root_override` → `tempdir` | skips by name when the registry is unreachable |

---

## G. Hazards

| # | Hazard | Failure it prevents | Code shape |
|---|---|---|---|
| H-1 | **The rustls provider** (fact-check finding) | `Client::builder().build()` panics with *"No rustls crypto provider is configured…"* the first time `i` is pressed in production | `install_crypto_provider()` behind a `std::sync::Once` at the top of `HttpClient::new`, `install_default()`'s `Err` ignored; `rustls` a direct dependency so the call has a name; T4's `a_client_builds_after_the_provider_is_installed` runs in the suite |
| H-2 | **`Response::content_length()` on a `HEAD`** (fact-check finding) | Every plan says "0 bytes", the D11 check never refuses, the consent text lies | `content_length_header(response.headers())` is the only reader; `Response::content_length` is named in the doc as the thing **not** to call; T4 asserts `681969407` from a fixture `HEAD` whose body is empty |
| H-3 | **Staging inside the glob's reach** | A half-unpacked tree resolves, spawns and dies; `R-AGT-6` broken at the source | `Layout` is the only path composer; `.staging/` and `.registry/` are siblings of `<id>/` and the seed's literal root ends at `<id>` (`expand`, `probe.rs:570-571`); T5's kill-mid-install case asserts `resolve_tool` → `None` with residue present |
| H-4 | **The post-promote resolve check** | The installer wrote where the seed does not look (a token typo, a Windows `%LOCALAPPDATA%` vs `data_local_dir` drift MOD-16 has not caught) and the row still says `missing` with a 2 GB tree on disk | Step 6 of `install()`: `resolve_tool(discovery.tools[tool])`, `canonicalize` both sides, `NotWhereTheRowLooks { promoted, resolved }` + rollback; this is the D5 agreement enforced at run time |
| H-5 | **Executable bit vs `launch::spawn`'s `which`** | `which::which(abs_path)` refuses a `0644` file (`launch.rs:743-748`), so an `amp-acp` tarball with plain modes probes `failed` with "is not executable" | `make_executable(tree/cmd)` unconditionally after unpack (`cfg(unix)`), **in staging, before promote**, so the promoted tree is never non-executable for an instant; T5 asserts `PermissionsExt` `0o111` after an archive that said `0644` |
| H-6 | **Cancellation leaking a partial tree** | `x` mid-download leaves a 400 MB `.archive` until the next sweep, or a partial unpack a later promote could pick up by nonce collision | `download` removes its file on `Cancelled`; `install()` removes `staging_tree`/`staging_archive` on every `Err` before step 5; the nonce is per process per microsecond; `Cancelled` after promotion is ignored (a commit is a commit) |
| H-7 | **`.staging` sweep vs `AgentRuntime::shutdown`'s `abort()`** (P-10) | An abort between set-aside and restore leaves the *working* version in `.staging/`, and the next install's sweep deletes it | The `.previous` suffix; `sweep` restores a `.previous` whose version directory is absent and deletes one whose target exists; T5's `the_sweep_restores_a_set_aside_version_an_abort_left_behind` |
| H-8 | **`abort()` does not stop `spawn_blocking`** (P-3) | Shutdown during a 2 GB unpack leaves a thread writing into `.staging/` after the runtime is gone, and the process exit waits on it | `unpack` checks `cancel.is_cancelled()` before every entry; `shutdown` calls `cancel.cancel()` before `task.abort()`; `finish_background` awaits the install task under its limit |
| H-9 | **`R-NF-3`: nothing long inside the worker's `select!`** | A registry `GET` or a `HEAD` on the arm stalls every other request for up to 15 s | `install_plan`/`install_confirm` await only `box_info()` and `agents()` (the awaits `probe()` already makes at `:390-398`); the `Installer` is built in the task (P-13); T7's loop-freedom test is the pin |
| H-10 | **The single-install claim** | Two installs of the same id race on `.staging/`, both promote, one's retention deletes the other's version | `AgentRuntime.install: Option<LiveInstall>` covers planning **and** installing; swept by `is_finished()` at the top of `serve`; the section refuses `i` in `Planning`/`Running` too |
| H-11 | **`HTUI_TOOL_<NAME>` precedence surviving an install** | The install's re-probe records the installed file although the user pinned another, and the next chat spawns the wrong one | The re-probe is `probe_agent` → `probe_tools`, whose first tier is the checked override (`probe.rs:378-401`); the post-promote check uses `resolve_tool` (no override) on purpose — it asks whether the *glob* sees the file, not what the box will run; T6's override case pins the row's `resolved.command` |
| H-12 | ~~**The four `Discovery` literal sites**~~ → **two**, corrected by T1 | T1 does not compile until every struct literal names the new field | **Corrected at T1**: only `tools.rs:143` and `tests/probe.rs:140` are `Discovery` literals. `tests/probe.rs:1993-2003` and `tests/acp_driver.rs:519-530` are **`ProbeSnapshot`** literals — they carry a `credential` field of their own, which is what this hazard's grep caught. `ProbeSnapshot` gains no `install` field, so `tests/acp_driver.rs` is untouched by this item |
| H-13 | **`Response::bytes_stream` with `read_timeout` on a captive network** | A portal that accepts the connection and sends nothing makes the download hang past the user's patience | `long` client's `read_timeout(60 s)` bounds each chunk; a timeout is `Network { manual }`, rendered as the steps; `x` cancels sooner through the `select!` |
| H-14 | **The registry cache under the root, written before the root exists** | First-ever `i` on a box: `<root>` absent, `.registry/` write fails, plan fails on a box that could install | `RegistryCache::store` and `Layout` create their directories with `create_dir_all`; a cache write failure is a `warn!`, never a `PlanError` (the document is in memory) |
| H-15 | **Downgrade under D14** (P-9) | Install of `1.1.0` beside `1.1.1` promotes, the glob resolves `1.1.1`, H-4 rolls back after a full download | `PlanError::Outranked` before consent, naming the directory to remove |
| H-16 | **`InstallPlan` is data the user could edit before confirming** | A `cmd` of `../../.bashrc` in a confirmed plan | `cmd_relative` at plan time **and** at run time; `safe_target` for every archive entry regardless |
| H-17 | **The section's `Agents` reply clears nothing it should not** | A `StoreReply::Agents` from a scope change during a download resets the install state (the `probing` precedent in the module doc) | `on_reply(Agents)` touches `agents`/`unavailable`/`probing` only; `install` is cleared by install frames alone |
| H-18 | **`tokio::test` paused time and the fixture server** | `#[tokio::test(start_paused = true)]` auto-advances past the 15 s timeouts while the socket is not yet readable, failing every fetch case | No paused time in `tests/install.rs`; the throttle is tested through `Throttle::admit` with hand-made `Instant`s instead |
| H-19 | **`sha2` at two versions in the lock** | An `install` that imported `sha2 0.11` through a transitive re-export would not match `identity.rs`'s digest text on the same bytes (it would, but the type would not unify) | `sha2 = { workspace = true }` (0.10) is already `htui-agent`'s; `fetch.rs` names `sha2::Sha256` only |
| H-20 | **Windows `rename` onto an existing directory** | `promote` fails on Windows when `<id>/<version>/` exists | `set_aside` runs first for the same version; `promote` asserts the target is absent and names it in the `Io` error |

---

## H. What this item does NOT touch

- **`crates/htui-store/**`** and **`crates/htui-store/migrations/**`**: no column, no method, no `.sqlx` regeneration; `0003_orchestration.sql` stays MOD-4's next migration (D17: consent lives in `manifest.json`).
- **`crates/htui-core/src/**`**: `AgentBox`, `AgentSummary`, `Agent` unchanged; only `seeds/agent_agy.json` moves. **`seeds/agent_claude.json` unchanged** (P-8).
- **`crates/htui/src/keymap.rs`**: no section `KeyScope`; the hint line stands in (D19).
- **`crates/htui/src/ui/overlay/**`**: no consent overlay (D19's reason: the factory takes no argument and replies to a popped overlay are dropped).
- **`crates/htui-agent/src/acp/**`**: the handshake, `open_session`, the mapper — untouched; the re-probe reaches tier 2 through the existing `SpawnTier2`.
- **`probe_agent`'s eight steps, `status_for`, `agent_box_row`**: the installer calls them and edits none.
- **`tools::resolve`'s override semantics** (`tools.rs:42-53, 96-99`): unchanged; H-11 relies on them being so.
- **`README.md:268-274`** (MOD-21's authentication paragraph): not touched by T10's rewrite of `:234-267`.
- **The `DriverError` enum**: no new variant; `PlanError`/`InstallError` are the plan's own types (P-6).
- **Anything named after an agent**: no file under `src/` names `antigravity`, `amp-acp`, `dl.google.com` or `agy_acp_server`; `the_installer_names_no_vendor` is vacuous until T4 and load-bearing from then on.
  **Corrected at T1**: the sweep as specified fails on the tree it was written for. `agy_acp_server` already appears in `src/` seven times — `crates/htui-agent/src/tools.rs:212,214,221,266,272` and `crates/htui-core/src/model/agent.rs:177,205` — every one inside an in-module `#[cfg(test)] mod tests` block, and `htui-core/src/**` is off-limits to this item, so the offenders cannot be edited away. T1's `production_half(text)` truncates each file at its trailing `\n#[cfg(test)]\nmod tests {` before sweeping: in-module tests are test code by the same rule that exempts `tests/`. Verified across all 33 `src/` files carrying `cfg(test)` that exactly one such trailing block exists per file. The cost, accepted: a vendor string inside an in-module test block passes the sweep. Every file with no in-module tests — which is all of `install/` — is still read whole.

Files most relevant to the implementer, absolute: `/home/mluigi/projects/htui/crates/htui-agent/src/probe.rs` (`:99-116`, `:554-744`), `/home/mluigi/projects/htui/crates/htui-agent/src/launch.rs` (`:91-109`, `:741-748`), `/home/mluigi/projects/htui/crates/htui/src/agent_worker.rs` (`:159-180`, `:268-414`, `:636-715`, `:905-963`), `/home/mluigi/projects/htui/crates/htui/src/store_worker.rs` (`:50-163`, `:315-357`, `:484-498`, `:1233-1273`), `/home/mluigi/projects/htui/crates/htui/src/testkit.rs` (`:168-241`, `:263-280`), `/home/mluigi/projects/htui/crates/htui/src/ui/tabs/settings/agents.rs`, `/home/mluigi/projects/htui/crates/htui-core/seeds/agent_agy.json`, `/home/mluigi/projects/htui/crates/htui-agent/tests/probe.rs` (`:36-57`, `:389-499`, `:869-945`).
