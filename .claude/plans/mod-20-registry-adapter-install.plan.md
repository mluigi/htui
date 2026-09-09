# Plan: MOD-20 registry-driven adapter install (milestones 1–4)

**Source PRD**: `.claude/prds/mod-20-registry-adapter-install.prd.md` (APPROVED; gate decisions
D1–D7 are the maintainer's and are implemented here, not re-opened).
**Selected Milestones**: all four. They are small enough to plan as one dependency chain, and
milestones 1–2 are pure `htui-agent` work that milestone 3 consumes whole; splitting the plan would
only move the same task table across two files.
**Numbering**: the PRD owns **D1–D7**. This plan's decisions start at **D8**; tasks at **T1**. A
plan decision that follows from a gate decision cites it by name (`per D4`), never re-uses its
number.

**Design authority**: the PRD's Evidence and Constraints sections (the registry's shape, the
measured sizes, the mtime hazard, the empty dependency graph); `docs/ANA-4.md` §4.6 (the deferral
this item reverses, the glob tier, the per-platform `args` append, the Windows shim rule) and §9
(the migration ledger this item must not disturb: `0002_agent_probe.sql` exists, MOD-4's `0003`
does not yet); `docs/REQUIREMENTS.md` `R-AGT-10` (the contract), `R-AGT-4`, `R-AGT-5`, `R-AGT-6`,
`R-TUI-8`, `R-NF-3`. Prior MOD-2 plans: D46 (one resolver, two callers), D48 (the hand-rolled
glob and its mtime rule — **amended here by D4**), D52/D53 (refuse before spawning; `Deferred` with
an owned `Writer`), D54 (`r` in the agents section), D57 (`htui` does not download — **reversed by
`R-AGT-10`**, recorded at close-out), D58 (the driver spawns `probe.resolved`).

**Requirements**: `R-AGT-10` (installed from the app, from the row's declared source, re-probed,
digest verified or its absence stated first, licence surfaced before the fetch), `R-AGT-5` (a third
agent is a registry row, so no code path here may know an agent's name), `R-AGT-6` (the probe is the
authority on what the box can run), `R-AGT-4` (installed-ness is a box fact and lives in
`agent_box`), `R-TUI-8` (the action sits in the Settings agent section), `R-NF-3` (never on the UI
task, never in the worker's `select!` arm).

**Complexity**: Large — two new dependency families, one shipped resolution rule edited, a new
request/reply pair with a streamed reply, and the first Settings action that needs a row cursor.

**Routing**: PRD path (C2, C3, C4 fired). Ultracode recommended for implement and review; this plan
is shaped for it — two parallel chains in `htui-agent`, then a serial `htui` chain, each task with
its file set stated for the mechanical intersection gate. `rust-reviewer` gate per
`.claude/workflow-config.json`.

## Summary

Today `ProbeStatus::Missing` is the one thing the app observes and refuses to act on. The registry
row already says *where* the adapter comes from in everything but name: `agent.launch.discovery`
holds a per-platform glob whose version segment is a wildcard, `PlatformGlob.args` carries the
registry's own `--uid=`, and `probe::platform_key()` speaks the registry's `<os>-<arch>` vocabulary.
What is missing is a **source**: a block in the row naming the ACP registry id, a reader for the
registry document, a consented download that is verified where it can be and honest where it
cannot, an unpack that is invisible until it is whole, and a re-probe that decides the outcome.

The plan adds exactly that, in the crates that already own each half:

- **`htui-agent`** gains `launch::Install` (the declared source), `probe::install_root` (D5's one
  helper, reached from the seeds through a `%HTUI_AGENTS_ROOT%` token so the seed can no longer
  spell a root), a version-aware `newest()` (D4), and an `install` module — registry reader with an
  `ETag` cache, pre-flight plan, fetch-verify-unpack-promote pipeline, manifest, re-probe, retention.
- **`htui`** gains three requests and one streamed reply served through `AgentRuntime::serve` as
  `Served::Deferred` with an owned `Writer` (the `ProbeAgents` shape, D52/D53), and the Settings
  agent section gains a row cursor, `i`, an in-section consent pane and a progress cell.
- **`htui-core`** changes only its two seed documents. **`htui-store`** does not change at all: D6's
  consent record lives on the box's disk, so MOD-4's `0003_orchestration.sql` stays the next
  migration (§ D17).

Nothing is keyed on an agent's name. The proof is `amp-acp` — different vendor, `.tar.gz`, a
published digest, Apache-2.0 — installed live from a registry row the test file itself writes (T9).

## Prerequisite: what the tree and the network already settle

Checked 2026-09-09 on this box, before writing a task. Facts the routing brief supplied are used
as given; the ones below are the additional facts this plan needed and verified itself.

- **The candidate archives are plain.** A 256 KiB tail range request on the antigravity
  `linux-x86_64` zip (`content-length: 681969407`) shows **two entries, both method 8 (deflate),
  no zip64**, with unix modes recorded in the central directory (`agy_acp_server.par` is
  `0o100775`, `localharness_external` `0o100575`). The amp-acp `linux-x86_64` archive begins
  `1f 8b 08 00` — gzip — behind a 302 to `github.com/.../releases/download/v0.9.0/amp-acp-linux-x86_64.tar.gz`.
  So one deflate implementation covers both, and D10 does not enable `deflate64`, `bzip2`, `zstd`
  or `lzma`.
- **The tree's TLS stack** is `rustls 0.23.43` with the **`ring`** provider and
  `rustls-native-certs 0.8.4`, brought in by sqlx's `tls-rustls-ring-native-roots`. There is no
  `hyper`, no `http`, no `tokio-rustls`, no `webpki-roots` in `Cargo.lock`. `sha2` is present
  **twice** (0.10.9, the workspace's, and 0.11.0 transitively); the installer uses the workspace's
  0.10 exactly as `identity.rs` does.
- **Crate facts** (crates.io index, 2026-09-09): `reqwest 0.13.5` MSRV 1.85 / edition 2021, whose
  only rustls option without a second crypto provider is `rustls-no-provider` (= `rustls-platform-verifier`
  + the rustls transport; the crate has no `native-roots` feature any more); `ureq 3.4.1` MSRV 1.85 /
  edition 2024, blocking, `rustls` feature = ring + `webpki-roots`; `zip 8.6.0` MSRV 1.88 (its
  `9.0.0-pre3` is a pre-release and is not a choice), `deflate-flate2` = flate2 alone;
  `tar 0.4.46` MSRV 1.63 (default feature `xattr`); `flate2 1.1.10` MSRV 1.67, default backend
  `miniz_oxide` (pure Rust); `fs4 1.1.0` MSRV 1.75 (unix `rustix`, windows `windows-sys`, both
  already in the lock); `hyper-rustls 0.27.9` MSRV 1.85; `rustls-platform-verifier 0.7.0` MSRV 1.85;
  `hyper-util 0.1.20` MSRV 1.64; `tokio-rustls 0.26.5` MSRV 1.71. Every one clears MSRV 1.98.
- **`dirs::data_local_dir()`** answers `~/.local/share` (or `$XDG_DATA_HOME`) on Linux,
  `~/Library/Application Support` on macOS and `%LOCALAPPDATA%` on Windows — the three roots the
  seed hand-writes today (`agent_agy.json:19-28`), which is what makes D5 a one-line helper.
  `XDG_DATA_HOME` is unset on this box, so the existing install keeps resolving.
- **std has no free-space API**, and both `libc::statvfs` and `GetDiskFreeSpaceExW` are `unsafe`
  calls the workspace forbids (`unsafe_code = "forbid"`). D3's size pre-flight therefore needs a
  crate (D11).
- **The Settings agent table has no row cursor** (`agents.rs:149-212`): `r` acts on every row. An
  install acts on one, so D19 adds the first cursor to that section.
- **Existing tests that will move with D5's token**: `tests/probe.rs` `env()` (`:36-57`) builds a
  `ProbeEnv` by hand, and `:440-472`/`:474-499` resolve the seed's `agy` row against a fake home at
  `home/.local/share/htui/...` and `home/Library/Application Support/htui/...`. `tests/launch.rs:56-63,188`
  hold a *copy* of the seed patterns as a serde fixture and do not read the seed, so they stay.
  `Discovery` literals needing the new field: `tools.rs:148`, `tests/probe.rs:143,1998`,
  `tests/acp_driver.rs:525` (milestone 6's `credential: None` lesson, recorded so no task is
  surprised by it).
- **Test fixtures that answer `initialize`** exist only in-process (`tests/probe.rs:869`
  `scripted_initialize` over a duplex, behind the `Tier2` seam). The production runtime uses
  `SpawnTier2`, so a harness-level install test can prove the *broken tree* metric (the row reads
  `failed`) but the *ready* outcome is proven in `htui-agent` through the seam and live through T9.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D8 | **No new crate.** The installer is `htui_agent::install` (registry reader, pre-flight, pipeline, manifest); the declared source is a type in `launch.rs`; D5's helper sits in `probe.rs` beside `platform_key`. The HTTP and archive dependencies become `htui-agent` dependencies. `htui` gains the requests, the runtime arms and the section; `htui-core` changes seeds only; `htui-store` is untouched. | The existing split is *pure / driver-and-probe / persistence / TUI*, and an installer is a **producer for the probe's glob**: it needs `ProbeEnv`, the expander, `resolve_tool` (for the post-promote "does the glob now find what I wrote" check) and `probe_agent` (the outcome), all of which are `probe.rs`'s. A separate crate would depend on `htui-agent` for every one of those and exist only to keep `reqwest` out of a manifest — the same reasoning that kept `probe.rs` in `htui-agent` at milestone 5 (D46: one resolver, two callers). The client is confined to one file (`install/http.rs`) so it stays replaceable, which is what the PRD's "narrow seam" asks for. |
| D9 | **HTTP client: `reqwest = "0.13"` (0.13.5), `default-features = false`, features `rustls-no-provider`, `stream`, `system-proxy`.** Timeouts: registry and `HEAD` 15 s total; archive `connect_timeout` 15 s and `read_timeout` 60 s (idle between chunks), no total. Redirects followed (default policy, 10 hops). Progress is `Response::bytes_stream()`; cancellation is dropping the future. Rejected: `ureq 3.4.1`. | D1 names the two things a shell-out cannot give — progress bytes and cancellation — and they decide the client. `reqwest` is async on the runtime the task already lives on: a chunk stream yields the byte count the progress cell needs, dropping the future cancels a stalled body **now**, and `read_timeout` bounds the "captive network turns a failure into a hang" risk per read rather than per body. `ureq` is the smaller graph (no `hyper`) and its `rustls` feature is `ring`, the tree's provider — genuinely close — but it is blocking: the body loop would run on `spawn_blocking`, cancellation would be a flag polled per chunk, and a stalled server would hold that thread until `timeout_recv_body`, which for a 682 MB download has to be tens of minutes. That is exactly the hang D1 was written to avoid, so `reqwest` wins on the criterion the gate stated. `rustls-no-provider` keeps **one** crypto provider in the process — sqlx already enables `rustls/ring`, so no `aws-lc-rs` (and no C toolchain on the Windows target) enters the tree — but **the provider must be installed explicitly, and this was falsified at the gate, not assumed**: with `rustls/ring` enabled and nothing else, `reqwest::Client::builder().build()` panics with *"No rustls crypto provider is configured. When using the `rustls-no-provider` feature you must install a crypto provider before building a Client"* (`reqwest-0.13.5/src/async_impl/client.rs:2501`). reqwest does not infer the single enabled feature. So `install/http.rs`'s constructor calls `rustls::crypto::ring::default_provider().install_default()` once and **ignores its `Err`** (the process may already hold a default), before building the client; `rustls` becomes a direct `htui-agent` dependency (`default-features = false, features = ["ring", "std", "tls12", "logging"]`) so the call has a name to say. Verified working at the gate: the amended form builds a client and completes a live `HEAD` against `dl.google.com` (200, `accept-ranges: bytes`). T4's "a client builds" test therefore asserts the *installed-provider* path and would fail the day someone removes the call. What reqwest *does* add beyond the tree's `rustls` + `rustls-native-certs`: `hyper`, `hyper-util`, `hyper-rustls`, `tokio-rustls`, `http`, `http-body`(-util), `tower`, `tower-service`, `sync_wrapper`, `ipnet`, `rustls-platform-verifier` (on Linux it delegates to the tree's `rustls-native-certs` with `webpki-roots` as the fallback bundle; on Windows/macOS it uses the OS verifier) — roughly fifteen crates, none with a build script beyond what `windows-sys` already has. `http2`, `charset`, `json`, `gzip` are off: the CDN answers HTTP/1.1, the registry is parsed with `serde_json` from bytes, and the archives are already compressed. `system-proxy` is on because a Windows box configures its proxy in the OS, and the proxy fallback is a PRD metric. |
| D10 | **Archive extraction: `zip = "8.6"` with `default-features = false, features = ["deflate-flate2"]`; `tar = "0.4"` with `default-features = false`; `flate2 = "1.1"` with its default pure-Rust backend.** Format is decided **at pre-flight** from the archive URL's suffix — `.zip`, `.tar.gz`, `.tgz` — and any other suffix is "not installable from this registry entry" before consent. Extraction runs on `spawn_blocking` from the downloaded file (never streamed), rejects absolute paths, `..`, and symlinks whose target leaves the staging directory, applies the archive's own unix modes where it carries them, and **always** sets the executable bit on the registry's `cmd` on unix afterwards. | The two real archives are deflate-only and gzip; enabling `deflate64`/`bzip2`/`zstd`/`lzma` would buy four decompressors nobody has an archive for (`zip`'s default set even pulls `aes`, `time`, `zopfli`). `tar`'s default `xattr` feature is a unix-only extra with no use here. Download-then-unpack rather than streaming because a zip's directory is at its **end**, and because the digest must be checked over the whole file *before* a byte of it is unpacked (D2). The archive carries the `.par`'s `0775` (verified in the Prerequisite), but `launch::spawn` runs `which` even on an absolute path (`launch.rs:743`), and a `.tar.gz` from another vendor may carry `0644`, so the bit is set regardless — which is the README's `chmod +x` made structural. |
| D11 | **Free space through `fs4 = "1.1"` (`default-features = false, features = ["sync"]`), and the pre-flight refuses when `available < DISK_HEADROOM_FACTOR × content-length`, with `DISK_HEADROOM_FACTOR = 4`.** A `HEAD` that fails or carries no `content-length` makes the size *unknown* — stated in the consent text, not refused. **The size is read from the `content-length` header, never from `Response::content_length()`**: on a `HEAD`, reqwest reports the *body* length, which is `Some(0)` (verified at the gate against the real archive, whose header says `681969407`). | D3 says a `HEAD`-derived pre-flight *may* refuse; this makes "may" a number. The factor comes from measurement: 682 MB archive, 2.0 GB unpacked — 2.95× — plus the archive itself on disk during the unpack, 3.95×; 4 is that rounded up, not a guess. `fs4` is the one maintained cross-platform wrapper whose unix half is `rustix` and Windows half is `windows-sys`, both already in the lock, so it adds one crate and no `unsafe` in this workspace. |
| D12 | **The declared source is `Discovery.install: Option<Install>`**, `#[serde(default, skip_serializing_if = "Option::is_none")]` beside `credential`, with `Install { source: InstallSource, id: String, tool: String }` and `InstallSource` a `wire_enum!`-style string enum with one value, `acp_registry`. `tool` names the `${tool}` placeholder the installed `cmd` fills. **The registry reader** parses into owned types with every field `#[serde(default)]` where the schema marks it optional and unknown keys ignored; a platform absent from `distribution.binary` is `PlanError::NotAvailable { platform }`, a first-class answer. The document is cached at `<install_root>/.registry/latest.json` + `latest.meta.json { etag, fetched_at, max_age }`: within `max-age` no request; past it a conditional `GET` with `If-None-Match`; a network failure with a cached copy plans from the cache and says `registry: cached <age>` in the consent text; a network failure with **no** cache is the no-network path (D20). **The pre-flight's product is one serialisable `InstallPlan` value** — agent id and name, `tool`, registry id and version, platform, archive URL and format, `content-length` (optional), `sha256` (optional), `cmd`, `args`, `env`, `license`, `license_url`, the install directory, the version directories that already exist, free bytes, whether the registry's `args` differ from the seed's `PlatformGlob.args`, and the consent state from D17. | The round-trip pair is the rule at `launch.rs:103-106`: a pre-MOD-20 document must re-serialise byte for byte. One source kind rather than a bare string keeps a future `github_release` from becoming a stringly `match`. The registry is mutable with no pinned snapshot URL, so **the plan value is the consent evidence**: what the user said yes to is what is installed, and `InstallConfirm` carries the plan back rather than re-reading a document that may have moved in the meantime. The `tool` name is what lets the pipeline check, after promotion, that `resolve_tool(discovery.tools[tool])` now answers the file it wrote — the D5 agreement enforced at run time, not only by test. The cache honours the headers the CDN sends (`ETag`, `max-age=300`) because 56 KB re-fetched on every `i` is wasteful and a 304 is free. |
| D13 | **What happens before consent, exactly**: one `GET` of the registry document (or none, from cache) and one `HEAD` of the archive URL. **No archive body byte is requested before `InstallConfirm`.** The consent text states: agent and version, platform, archive URL and size (or "size unknown"), install directory, licence and `license_url`, the digest line — `sha256 published: verified before unpacking` **or** `none published: htui cannot verify this download and will record what it receives` — free space against the D11 need, which existing versions will be replaced and when they are deleted (D16), and whether the registry's `args` differ from the row's. `y` accepts the licence terms **and** the install in one keystroke; `n`/`Esc` declines. | `R-AGT-10`'s literal wording is "told that before the download, not after" and "surfaced before `htui` fetches it": the `HEAD` is the size the PRD's own scope puts in the pre-flight and transfers no adapter bytes; the body is the fetch. The metric "a test asserting no request is issued before consent" is therefore encoded as *the fixture server has seen exactly `GET /registry.json` and `HEAD /archive` and no `GET /archive` until `y`* — precise enough to fail if anyone adds a speculative range request. A second keystroke for the licence would be ceremony the PRD did not ask for; recording the plan's `license`/`license_url` with the acceptance (D17) is what gives the single `y` its meaning. |
| D14 | **D4 made concrete.** `walk` returns `Vec<GlobMatch { path, captures: Vec<String> }>`, one capture per `*` segment in order; `newest(Vec<GlobMatch>) -> Option<PathBuf>` orders by the key `(Option<semver::Version>, mtime, path)` under `max()`, where the version is parsed from the **last** capture (an optional leading `v` stripped, `semver::Version::parse` strict). `Option`'s ordering puts every parsable version above every unparsable one; among parsable ones the version decides; **among unparsable ones today's rule stands unchanged** — newest mtime, then path descending. Unreadable metadata still sorts as the epoch. `glob_first`'s signature does not change. | The two failures D4 names are both in the last capture: `unzip` preserving the vendor's build mtime, and `"1.9.0" > "1.10.0"` under lexicographic path order. Keying on that capture fixes both without inventing a rule for the JetBrains shape, whose build directories (`20260501`, `20260818`) do not parse as semver and therefore keep the selection they have today — which is the PRD's explicit requirement that a box that installs nothing resolves the same file, and why `the_jetbrains_two_star_shape_resolves_to_the_newest_install` (`tests/probe.rs:389-437`) is **kept verbatim**, its doc amended to say it now pins the fallback. A parsable version beating an unparsable sibling is the deliberate reading of "a directory named like a version is an install"; it is stated and tested rather than left to `Option`'s derive by accident. `semver` is already a direct dependency (MOD-2 D47). This amends MOD-2 plan D48's "newest mtime first" and is recorded as such at close-out. |
| D15 | **D5 made concrete.** `probe.rs` gains `pub const INSTALL_ROOT_VAR: &str = "HTUI_AGENTS_ROOT"`, `pub fn default_install_root() -> Option<PathBuf>` (= `dirs::data_local_dir()?/htui/agents`) and `pub fn install_root(env: &ProbeEnv) -> Option<PathBuf>` (= `env.var(INSTALL_ROOT_VAR)`). `ProbeEnv::host` inserts `INSTALL_ROOT_VAR = default_install_root()` into `vars` **unless the environment already sets it**. The seeds stop spelling any root: `agent_agy.json`'s `htui`-managed patterns become `%HTUI_AGENTS_ROOT%/antigravity-acp/*/agy_acp_server.par` (linux, darwin) and `%HTUI_AGENTS_ROOT%/antigravity-acp/*/agy_acp_server.exe` (windows); the JetBrains `%LOCALAPPDATA%/JetBrains/...` pattern is untouched. The installer writes to `install_root(env)`; the consent text and the manual instructions print it. | "Agree by construction rather than by review" (D5) means the seed must have **nothing to agree with**: a token the existing expander already understands (`%VAR%` from `env.vars`, `probe.rs:588-608`, backslashed Windows values kept as one segment by `PathBuf::push`, `:546-547`) removes the spelling from the document entirely, on all three platforms at once. Routing the default through `ProbeEnv` rather than a global keeps the test rule the tree already lives by — `std::env::set_var` is `unsafe` and forbidden, so every test injects `vars` — and gives the maintainer a relocation knob for a 1.9 GB-per-version tree without a second mechanism. `data_local_dir` is the Windows `%LOCALAPPDATA%` the README documents, not `%APPDATA%` (roaming), which is why it is not `config_root()`'s `dirs::config_dir()`. Behaviour change, stated: a Linux box with `XDG_DATA_HOME` set now resolves there instead of a hard-coded `~/.local/share`; this box has it unset. |
| D16 | **D3 made concrete — staging, promotion, rollback, retention.** The download lands in `<root>/.staging/<id>-<version>-<nonce>.archive`; the unpack goes to `<root>/.staging/<id>-<version>-<nonce>/`; promotion is one `rename` to `<root>/<id>/<version>/`. A same-version re-install first renames the existing directory into `.staging/`. After promotion the row is re-probed through `probe_agent` (the installer never writes a status). Then: **(a)** re-probe `ready`/`unauthenticated` → every *other* version directory under `<root>/<id>/` is deleted, and the manifest records the install; **(b)** re-probe `missing`/`failed` **and a previous version existed** → the promoted directory is removed and the previous one restored, the row is re-probed once more so `agent_box` describes what is left, and the outcome is `Failed` with the probe's `stderr_tail`; **(c)** re-probe `missing`/`failed` **with no previous version** → the tree is left in place, the row says what the probe said, and the outcome is `Failed` with the same text. Every install begins by sweeping `.staging/` entries older than one hour. | `.staging/` is a sibling of `<id>/`, and the seed pattern's literal root ends at `<id>` (`expand` puts every leading literal segment into the root, `:570-571`), so nothing in staging can ever be walked — the "interrupted install leaves nothing the glob resolves" metric holds by directory layout, and `rename` on one filesystem makes promotion atomic. D3's "a failed install never costs the working one" is case (b): the working version comes back and the row is re-probed to prove it. Case (c) exists because "the adapter is fine and the sibling CLI is missing" is `missing` too — rolling back a 682 MB download for a `PATH` problem would punish the wrong thing; the probe's text tells the user which. The one-hour sweep is what an aborted task (`AgentRuntime::shutdown`) leaves behind; sweeping on the next install rather than at startup keeps startup free of filesystem work. |
| D17 | **D6 made concrete — the consent record is `<install_root>/<id>/manifest.json`**, one file per registry id, `{ consent: { license, license_url, accepted_at, version } \| null, installs: { "<version>": { sha256, published: bool, archive, platform, installed_at } } }`, written tmp-then-rename like `identity::store`. Consent is **re-asked** when the registry entry's `license` or `license_url` differs from the recorded pair. The `installs` map is D2's computed-digest record; a same-version re-install whose computed digest differs from the recorded one is reported in the consent text and the outcome. **No migration, no new store method.** | Consent is a fact about this box (D6), and the strongest box-local home the tree has is the box's own disk — the same reasoning `R-AGT-9` applies to credentials. The rejected homes, each for a reason: `agent_box` needs a column, therefore a migration, therefore it *is* the `0003` MOD-4 has been promised (ANA-2 §9, HANDOFF MOD-4), and `agent_box_row` rebuilds the row from the snapshot so the column would need the `quota` carry-over treatment; `box.settings` is MOD-7's profile with no write path today and undefined JSONB merge semantics, so writing it here builds MOD-7's seam early and badly; `app_setting` is global, not per box; `box.toml` is `htui-store`'s identity file and the installer lives in `htui-agent`. One manifest per id rather than a record inside the version directory because D3 deletes version directories, and D2's "a later re-install of the same version producing a different digest is detectable" needs the digest to outlive the directory. The cost is honesty about visibility: consent is not in `psql`, and a re-imaged box asks again — which is correct. |
| D18 | **Requests and replies.** `StoreRequest::InstallPlan { agent_id }`, `StoreRequest::InstallConfirm { plan: Box<InstallPlan> }`, `StoreRequest::InstallCancel`, and `StoreReply::Install(InstallFrame)` with `InstallFrame::{ Plan(Box<InstallPlan>), Progress { phase, done, total: Option<u64> }, Done(Box<InstallOutcome>), Cancelling, Cancelled, Failed { message, manual: Option<ManualSteps> } }`. All three are intercepted in the worker loop before `try_serve` beside `ProbeAgents` and served by `AgentRuntime::serve`: `InstallPlan` and `InstallConfirm` return `Served::Deferred` with a task that answers at the request's own address; `InstallCancel` answers `Cancelling` at once and the running task's stream ends with `Cancelled`. **Refusals before anything is spawned**, in the probe's order: no `Writer` or `Writer::Buffered` → `REGISTRY_ON_SERVER_ONLY` (at *plan* time, so no network is spent on an install that cannot be written); no box row; an install already running (`AgentRuntime.install: Option<LiveInstall { agent_id, cancel: CancellationToken, task }>`). Progress frames are sent at most every 250 ms. The `Installer` (client, config) is owned by the runtime and injected for tests via `AgentRuntime::with_installer(InstallConfig { registry_base, root_override })`; production uses the constant URL and `ProbeEnv::host`'s root. | The `ProbeAgents` shape is the PRD's stated model and `R-NF-3` is enforced by the same ownership: the `Writer` is taken before the spawn (`agent_worker.rs:373-414`), the task owns it, the loop `continue`s (`store_worker.rs:484-498`). One request, many replies is the chat stream: every frame of a confirm carries the `InstallConfirm` request's `seq`, and `App::is_fresh` passes them for as long as the section has not issued a newer `InstallConfirm` — no new architecture, one new reply variant, exactly as the PRD scopes it. The plan-then-confirm split is what D13 needs (nothing fetched before `y`) and what D12 needs (the plan value is the evidence). Cooperative cancellation through a token rather than `abort()` so the task itself can sweep its staging entry and send the final frame; `abort()` stays the shutdown path. 250 ms matches `event_loop.rs`'s `TICK`, so a faster stream would only queue frames nobody sees. The injected `InstallConfig` exists for the same reason `tools::resolve_with` does: `set_var` is forbidden, so tests inject. |
| D19 | **The Settings UI.** The agents section gains a `TableState` row cursor (`j`/`k`), `i` on the highlighted row, and an **in-section consent pane** rendered under the table — not an overlay. Key rules: `i` is refused (`Action::Error`) while `probing`, while an install is running, or when the row declares no `install` block ("nothing declares how to install `<name>`"); `r` is refused while an install runs; with a plan pending the section consumes `y`/`n`/`Esc` and swallows every other key it is offered; `x` while running sends `InstallCancel`. The `on this box` cell reads `planning…`, `downloading 42%`, `verifying…`, `unpacking…`, `probing…` for that row, then the probe's status once `Done` triggers a fresh `StoreRequest::Agents`. A one-line hint under the table (`j/k select · r probe · i install`, or `y install · n cancel` while a plan is pending) stands in for the help line, since sections have no `KeyScope`. | An overlay was the first candidate and the constraint the PRD records is why it loses: the factory is `Fn() -> Box<dyn Overlay>` with no argument, so an overlay cannot be told *which row*, and `MigrationPrompt` only works because `StoreState` is a parameterless read. Two workarounds exist — the runtime caching a "pending plan" for a parameterless `InstallPending` read, or the overlay emitting a request whose stream is addressed to the section — and both add cross-view state to answer a question the section already holds in its cursor. Worse, replies to a popped overlay are dropped (`app/update.rs:166-181`), so the progress stream could never be addressed to the view that asked. The section receives the plan the way the PRD asks any consent view to — through `on_reply` — and is the one view that outlives the whole install. Modality is local and sufficient: the tab's own `h`/`l` still switch sections, and a pending plan waits. A cursor is new to this section but not to the app (`TableState` is ratatui's), and `R-TUI-8`'s quota column will need one anyway. |
| D20 | **The no-network and proxy path.** A `PlanError::Network` with no cached registry, a refused or timed-out `HEAD`-then-`GET`, or any failure during the download, answers `Failed { manual: Some(ManualSteps) }`, and the section renders the steps under the table: the registry URL and the row's `id`, this box's `platform_key()`, the directory to unpack the **whole** archive into (`<install_root>/<id>/<version>/` — version from the plan when known, `<version>` otherwise), the `cmd` to make executable, and the `HTUI_TOOL_<NAME>` override (`tools::env_override_key(tool)`). Proxies are whatever `reqwest`'s `system-proxy` and the `HTTP(S)_PROXY` variables say; nothing is configured in `htui`. | Every word of the manual steps is derived from the row and the helper, so the fallback replaces `README.md:234-267` without re-creating its defect (a pinned URL that goes stale) and without naming a vendor in code (`R-AGT-5`). Failing *with* the steps is the PRD metric; failing opaquely is what it forbids. |
| D21 | **Windows: written and lint-checked from Linux, verified by MOD-16.** Written here: `cfg(unix)` for the executable bit and modes (no-op on Windows); the registry's Windows `cmd` (`./agy_acp_server.exe`) resolved relative to the version directory; promotion `rename` retried three times with backoff on `PermissionDenied` (Defender scanning a freshly written `.exe`); paths compared through `canonicalize` on both sides of the post-promote glob check (the `\\?\` prefix); the `%HTUI_AGENTS_ROOT%` token expanded with the Windows case-insensitive `var` lookup; the seed's `windows-x86_64` and `windows-aarch64` patterns. The Windows clippy line runs for `htui-agent` **and** `htui`. **Deferred to MOD-16 explicitly**: that the retry actually clears Defender's lock; long-path behaviour past 260 characters; that a `.cmd` `cmd` (none in the registry today) spawns through `which`'s `PATHEXT` rule; `system-proxy` reading the registry's proxy; `fs4::available_space` on a junction; and every runtime fact about `agy_acp_server.exe` itself. | The PRD's out-of-scope line and MOD-16's charter: this box has the `x86_64-pc-windows-msvc` target for clippy and nothing to run it on, exactly as for every MOD-2 milestone. Listing the deferred facts by name is what lets MOD-16 pick them up instead of rediscovering them. |
| D22 | **Documentation replacement.** `README.md:234-267` becomes "Installing an agent's adapter": `Settings > Agents`, `j`/`k`, `i`, what the consent shows, the digest and licence rules in plain words, `HTUI_AGENTS_ROOT`, `HTUI_TOOL_<NAME>`, and the no-network fallback; **lines 268-274 (MOD-21's authentication paragraph) are not touched.** `agy_live.rs`'s `INSTALL_HINT` becomes the in-app instruction plus the override, and its module doc's "plan D57 — `htui` never downloads it" is amended to cite `R-AGT-10`. `HANDOFF.md`'s MOD-20 line gains a phase note per milestone (lifecycle step 4); the PRD's four rows go `complete`; MOD-2's D48 and D57 are recorded as amended/reversed in the note, not edited in their plan. | The README section and the test constant are the third and fourth copies of a coordinate that goes stale silently (PRD Problem). The old plans are history and stay as written; the phase note is where the tree says what changed and why, as milestone 5 did for ANA-4 §4.6. |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Long op off the loop | `crates/htui/src/agent_worker.rs:373-414`, `:665-715` | take the `Writer` before spawning, refuse `Buffered` with `REGISTRY_ON_SERVER_ONLY`, `self.background.push(tokio::spawn(..))`, one reply at the request's own `ReplyAddr` |
| One request, many replies | `crates/htui/src/agent_worker.rs:9-12`; `store_worker.rs:204-208, 250-268` | `StoreReply::Chat(ChatFrame)` all carrying the `ChatStart` `seq`; `App::is_fresh` |
| Loop-freedom test | `crates/htui/src/store_worker.rs:1231-1273` | probe is seq 1, `Workspaces` seq 2, the first reply back is seq 2 — T7 copies it for `InstallConfirm` |
| Refuse-before-spawn test | `agent_worker.rs` `background_len()` (`:229`) | a refused request spawned **nothing** |
| Round-trip rule | `crates/htui-agent/src/launch.rs:103-108` | `#[serde(default, skip_serializing_if = "Option::is_none")]` so an old document re-serialises byte for byte |
| Injected box, no `set_var` | `crates/htui-agent/tests/probe.rs:36-57`; `tools.rs:82-88` | `ProbeEnv { vars, .. }` built by hand; an injected override closure |
| Atomic file write | `crates/htui-store/src/identity.rs:104-124` | write `<name>.<nonce>.tmp`, `rename` over the target |
| Digest | `crates/htui-store/src/identity.rs:132-144` | `sha2::Sha256`, lowercase hex |
| Settings section | `crates/htui/src/ui/tabs/settings/agents.rs` | `wants_requests` + `on_reply` + `on_key`; `probing` flag refuses a second `r`; `on_box_cell` in-flight text wins |
| Section render test | `crates/htui/tests/settings.rs:186-301` | `probed_row` + `render_section` + `insta` snapshot per outcome |
| Harness end to end | `crates/htui/tests/probe.rs` | `Harness::over(store).with_tab(..).with_agent_runtime(..)`, `settle`, `key`, `render` |
| Live suite | `crates/htui-agent/tests/agy_live.rs`, `probe_live.rs` | `#[ignore]`, module doc with the exact run line, preconditions checked by name, nothing asserted that the vendor may change |
| `R-AGT-5` sweep | `crates/htui-agent/tests/extensibility.rs:76-131` | walk the tree, fail on a name outside the allowed files |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-agent/src/launch.rs` | UPDATE | T1 | `Install`, `InstallSource`; `Discovery.install` (D12) |
| `crates/htui-core/seeds/agent_agy.json`, `agent_claude.json` | UPDATE | T1, T2 | the `install` block (T1); root token in the glob patterns (T2) |
| `crates/htui-agent/src/tools.rs` | UPDATE | T1 | the `#[cfg(test)]` `Discovery` literal gains `install: None` |
| `crates/htui-agent/tests/launch.rs`, `tests/acp_driver.rs`, `tests/extensibility.rs` | UPDATE | T1 | round-trip cases; literal sites; the vendor sweep |
| `crates/htui-agent/src/probe.rs` | UPDATE | T2, T3 | D15 helper + `ProbeEnv::host` seeding (T2); `GlobMatch`, `walk`, `newest` (T3) |
| `crates/htui-agent/tests/probe.rs` | UPDATE | T1, T2, T3 | literal sites (T1); `env()` gains the token, seed-row cases move under it, agreement tests (T2); D14 cases (T3) |
| `Cargo.toml`, `crates/htui-agent/Cargo.toml` | UPDATE | T4, T5 | `reqwest` (T4); `zip`, `tar`, `flate2`, `fs4`, dev `tokio/net` (T5) |
| `crates/htui-agent/src/install/mod.rs`, `http.rs`, `registry.rs`, `plan.rs` | CREATE | T4 | client, reader + cache, pre-flight, `ManualSteps` |
| `crates/htui-agent/src/install/fetch.rs`, `archive.rs`, `layout.rs`, `manifest.rs` | CREATE | T5 | streamed download + digest, extraction, staging/promote, the manifest |
| `crates/htui-agent/src/install/run.rs` | CREATE | T6 | the pipeline end to end: re-probe, rollback, retention, cancellation |
| `crates/htui-agent/src/lib.rs` | UPDATE | T4 | `pub mod install` and re-exports |
| `crates/htui-agent/tests/install.rs` | CREATE | T4, T5, T6 | the fixture HTTP server, fixture registry, fixture archives, every offline case |
| `crates/htui-agent/tests/fixtures/registry.json`, `fixtures/install/*` | CREATE | T4, T5 | a pinned copy of the two registry entries; a tiny `.zip` and `.tar.gz` built by the test |
| `crates/htui-agent/tests/install_live.rs` | CREATE | T9 | the `amp-acp` proof |
| `crates/htui/src/store_worker.rs` | UPDATE | T7 | three variants, `name()` arms, interception, no-runtime refusal, loop-freedom test |
| `crates/htui/src/agent_worker.rs` | UPDATE | T7 | `LiveInstall`, `with_installer`, the plan/confirm/cancel arms, `run_plan`/`run_install` tasks |
| `crates/htui/src/ui/tabs/settings/agents.rs` | UPDATE | T8 | cursor, `i`, consent pane, progress cell, hint line, manual steps |
| `crates/htui/tests/settings.rs`, `tests/snapshots/settings__*.snap` | UPDATE | T8 | cell/pane rendering per frame; snapshots re-accepted for the hint line |
| `crates/htui/tests/install.rs` | CREATE | T8 | harness end to end over the fixture server |
| `README.md`, `crates/htui-agent/tests/agy_live.rs`, `HANDOFF.md`, the PRD | UPDATE | T10 | D22 |

Not touched, on purpose: anything under `crates/htui-store/` (D17), `crates/htui-store/migrations/`
(MOD-4's `0003` stays next), `crates/htui/src/keymap.rs` (no section scope, D19),
`crates/htui/src/ui/overlay/*` (D19), `crates/htui-agent/src/acp/*`.

## Milestone → task map

| PRD milestone | Outcome | Delivered by |
|---|---|---|
| 1 — Declared source + registry reader | a row states its source; from a cached document `htui` answers archive / verification / terms / size for this box, offline-testably | **T1**, **T2**, **T4** |
| 2 — Fetch, verify, unpack, promote | an archive becomes a tree at the path the glob reads, executable, digest-checked where published, invisible until whole | **T5**, **T6** (the pipeline as one call, through the `Tier2` seam) |
| 3 — The action in the app | consent, progress, re-probe, responsive TUI | **T7**, **T8** |
| 4 — Living with it | retention and disk policy (T6, T4's D11 check), version selection that survives a downgrade (**T3**), no-network and proxy fallbacks (T4's `ManualSteps`, rendered in T8), docs and `INSTALL_HINT` (**T10**), `amp-acp` live (**T9**) | **T3**, **T9**, **T10**, plus the named parts of T4/T6/T8 |

T3 and T6 are built before milestone 3 because milestone 3's re-probe needs D4 to select the
version it just installed and D16 to know what "previous version" means; their *outcomes* are
milestone 4's and the phase note for milestone 4 will say which tests prove them.

## Tasks

TDD per repo convention: the test that fails for the stated reason comes first, and a task whose
only test is "it compiles" is not a task. **Independence is by file set** and the sets are listed
for the mechanical gate; a dependency on a *type* from an earlier task without a shared file is
stated as "needs Tn landed", which is a sequencing note, not a file intersection.

Two chains run in parallel inside `htui-agent` once T1 has landed: **chain A** (T2 → T3, the
resolver) and **chain B** (T4 → T5 → T6, the installer). T5 needs T2 landed (it calls
`install_root`), T6 needs T3 landed (it relies on the version-aware selection). Then the `htui`
chain T7 → T8, then T9 and T10.

### T1: The declared source (milestone 1; first, alone)
- **Files**: `crates/htui-agent/src/launch.rs`, `crates/htui-agent/src/tools.rs`,
  `crates/htui-core/seeds/agent_agy.json`, `crates/htui-core/seeds/agent_claude.json`,
  `crates/htui-agent/tests/launch.rs`, `crates/htui-agent/tests/probe.rs`,
  `crates/htui-agent/tests/acp_driver.rs`, `crates/htui-agent/tests/extensibility.rs`
- **Tests first** (`tests/launch.rs`): a `Discovery` document with
  `"install": { "source": "acp_registry", "id": "x", "tool": "y" }` deserialises into
  `Install { source: InstallSource::AcpRegistry, id, tool }` and round-trips; a document **without**
  the key re-serialises **byte for byte** (the milestone-6 credential case, copied); an unknown
  `source` value is a deserialisation error naming it. (`tests/extensibility.rs`)
  `every_seed_row_deserialises_into_the_launch_types` extended: the `agy` seed's `install` names
  a `tool` that exists in its `discovery.tools` and whose probe is a `Glob`; the `claude` seed
  declares **no** `install` (its adapter is a `NodePackage`, PRD "Not for"). New
  `the_installer_names_no_vendor`: walks `crates/htui-agent/src`, `crates/htui/src`,
  `crates/htui-core/src` and fails on `antigravity`, `amp-acp`, `dl.google.com`, `agy_acp_server`
  outside seeds and `tests/` — vacuous until T4 creates `install/`, load-bearing from then on.
- **Action**: D12's types; the seed block; `install: None` at the three literal sites.
- **Validate**: `cargo test -p htui-agent --features test-support`; `cargo test -p htui-core --all-features`

### T2: D5 — the install root, the token, the seeds (milestone 1; chain A, serial after T1)
- **Files**: `crates/htui-agent/src/probe.rs`, `crates/htui-core/seeds/agent_agy.json`,
  `crates/htui-agent/tests/probe.rs`
- **Tests first** (`tests/probe.rs`): `env()` gains `HTUI_AGENTS_ROOT = <tmp>/agents`;
  `the_seeded_agy_row_resolves_by_glob_on_linux_with_the_uid_arg` and
  `the_same_row_on_darwin_appends_no_args` place the server under `<tmp>/agents/antigravity-acp/1.1.1/`
  and must still resolve **with the unmodified seed row** (they fail first because the seed still
  says `~/.local/share`); `the_windows_seed_pattern_expands_the_root_token_case_insensitively`
  (`platform: windows-x86_64`, `vars` holding `htui_agents_root` in another case and a backslashed
  value) resolves the `.exe`; `no_seed_pattern_spells_an_install_root` greps both seed documents
  for `.local/share/htui`, `Application Support/htui`, `LOCALAPPDATA%/htui`;
  `host_seeds_the_root_token_from_the_helper_unless_the_environment_sets_it` (over
  `ProbeEnv::host`, asserting the key is present and equals `default_install_root()` when the real
  environment lacks it — the "sets it" half is asserted through `install_root(&env)` on a
  hand-built env, since the process environment cannot be set); `install_root_is_none_when_the_token_is_absent`.
- **Action**: D15 — the three items in `probe.rs`, the seed rewrite (JetBrains pattern untouched).
- **Validate**: `cargo test -p htui-agent --features test-support`;
  `cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features -- -D warnings`

### T3: D4 — version-aware `newest()` (milestone 4 outcome; chain A, serial after T2)
- **Files**: `crates/htui-agent/src/probe.rs`, `crates/htui-agent/tests/probe.rs`
- **Tests first** (`tests/probe.rs`): `a_semver_directory_beats_a_newer_mtime` (`1.10.0` touched
  older, `1.9.0` touched newer → `1.10.0`); `semver_beats_path_order_where_the_old_rule_was_wrong`
  (`1.9.0` and `1.10.0`, equal mtimes → `1.10.0`; the assertion message names the lexicographic
  trap); `a_prerelease_sorts_below_its_release`; `a_version_directory_beats_a_non_version_sibling`
  (`1.1.1` vs `current`, `current` newer → `1.1.1`); `a_leading_v_is_tolerated`;
  `the_jetbrains_two_star_shape_resolves_to_the_newest_install` **kept verbatim** with its doc
  amended to "non-semver captures keep the mtime rule (D14 fallback)"; `walk_reports_one_capture_per_star`.
- **Action**: D14 — `GlobMatch`, `walk` returns it, `newest` re-keyed, `glob_first` unchanged
  outside; `newest`'s doc rewritten; the D48 amendment noted in the doc.
- **Validate**: `cargo test -p htui-agent --features test-support`

### T4: The registry reader and the pre-flight (milestone 1; chain B, needs T1 landed)
- **Files**: `Cargo.toml`, `crates/htui-agent/Cargo.toml`, `crates/htui-agent/src/lib.rs`,
  `crates/htui-agent/src/install/mod.rs`, `crates/htui-agent/src/install/http.rs`,
  `crates/htui-agent/src/install/registry.rs`, `crates/htui-agent/src/install/plan.rs`,
  `crates/htui-agent/tests/install.rs`, `crates/htui-agent/tests/fixtures/registry.json`
- **Tests first** (`tests/install.rs`, over a hand-rolled `tokio::net::TcpListener` HTTP/1.1
  responder that records every request line): the pinned `registry.json` (the real document's
  `antigravity-acp` and `amp-acp` entries, verbatim) parses, `antigravity-acp` has five platforms
  and no `sha256`, `amp-acp` has `sha256` on every platform; `plan()` for `linux-x86_64` answers
  the archive URL, `cmd`, `args == ["--uid="]`, `license == "proprietary"`, the `license_url`,
  `sha256: None`, format `Zip`, and `content_length` from the fixture's `HEAD`; the same for
  `amp-acp` with `Some(sha256)` and format `TarGz`; a platform the entry lacks (`darwin-x86_64`)
  is `NotAvailable`; an archive URL ending `.7z` is `Unsupported` **before** any `HEAD`; the
  recorder shows exactly `GET /registry.json` then `HEAD /archive` and nothing else (D13); a second
  `plan()` within `max-age` issues **no** request; past it, a `GET` with `If-None-Match` answered
  `304` keeps the cache; a `HEAD` failure yields `content_length: None` and a plan; a registry
  `GET` failure with a warm cache plans with `registry_cached_age: Some(_)`; the same with no cache
  is `PlanError::Network` whose `ManualSteps` name the fixture URL, the id, `platform_key()`, the
  injected root and `HTUI_TOOL_Y` (`env_override_key`); free space below `4 × content_length`
  (injected `available_bytes`) is `PlanError::Disk` with both numbers; a `reqwest::Client` builds
  **after `install/http.rs` has installed the ring provider**, and removing that call fails the test
  with the vendor's own panic text (guards D9's amended single-provider rule); a `HEAD` whose header
  carries `content-length: 681969407` yields that number and **not** the `Some(0)` that
  `Response::content_length()` reports for a `HEAD`. `the_installer_names_no_vendor` (T1) now bites.
- **Action**: D9 (workspace + crate dependency, `http.rs` wrapping the client, timeouts),
  D12 (types, reader, cache under `<root>/.registry/`), D11's arithmetic (with `fs4` arriving in
  T5, this task takes `available_bytes` as an input and T5 wires the real value), D20's
  `ManualSteps`. Tokio's `net` feature as a **dev**-dependency for the fixture server.
- **Validate**: `cargo test -p htui-agent --features test-support --test install`;
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
  `cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features -- -D warnings`

### T5: Fetch, verify, unpack, promote, manifest (milestone 2; chain B, serial after T4, needs T2 landed)
- **Files**: `crates/htui-agent/Cargo.toml`, `crates/htui-agent/src/install/mod.rs`,
  `crates/htui-agent/src/install/fetch.rs`, `crates/htui-agent/src/install/archive.rs`,
  `crates/htui-agent/src/install/layout.rs`, `crates/htui-agent/src/install/manifest.rs`,
  `crates/htui-agent/tests/install.rs`, `crates/htui-agent/tests/fixtures/install/` (archives
  written by the test at run time from a small tree; nothing binary checked in)
- **Tests first** (`tests/install.rs`): a fixture `.zip` (two files, one with mode `0755` in the
  archive, one without) downloads through the stream into `.staging/<id>-<v>-<nonce>.archive`
  with progress callbacks whose `done` sums to `content_length`; the computed sha256 equals
  `sha2` over the same bytes; a plan with `sha256: Some(wrong)` is `InstallError::DigestMismatch`
  and **nothing is unpacked** (no directory under `.staging/`); a plan with `sha256: None` records
  the computed digest in `manifest.json` `installs["<v>"].published == false`; the `.tar.gz` twin
  of the same tree unpacks identically; an entry `../escape` and an absolute entry are refused with
  the archive left in staging and nothing promoted; after promotion `<root>/<id>/<v>/cmd` is
  executable (`PermissionsExt` on unix) even when the archive said `0644`; the whole tree is
  present, not the `cmd` alone; a `cmd` of `./sub/../x` is refused; a kill between unpack and
  promote (the test drops the future at an injected await point) leaves `<root>/<id>/` without the
  version and `.staging/` with the residue, and `resolve_tool` over the seed-shaped glob answers
  `None`; the next install sweeps residue older than one hour; a same-version re-install moves the
  old directory into `.staging/` before promoting; `manifest.json` is written tmp-then-rename;
  consent re-asked when `license_url` changes (`Manifest::consent_covers(&plan)`).
- **Action**: D10, D11 (real `fs4::available_space` wired into the plan's input), D16's first half
  (layout, staging, promote, same-version swap, sweep), D17 (manifest read/write).
- **Validate**: as T4, plus `cargo test -p htui-agent --features test-support`

### T6: The pipeline: re-probe, rollback, retention, cancellation (milestone 2; chain B, serial after T5, needs T3 landed)
- **Files**: `crates/htui-agent/src/install/mod.rs`, `crates/htui-agent/src/install/run.rs`,
  `crates/htui-agent/tests/install.rs`
- **Tests first** (`tests/install.rs`, through the `Tier2` seam of `tests/probe.rs` copied
  locally): `install(plan, agent, existing, ctx, tier2, progress, cancel) -> InstallOutcome` with
  a scripted `initialize` yields `ProbeStatus::Ready` **from the probe's row**, and the outcome's
  `agent_box` is what `probe_agent` returned (the installer sets no status: a test that hands a
  `Tier2` answering an error gets `failed` however cleanly the download went — the PRD's
  "installer succeeds into a broken tree" metric); with a previous `1.1.0` present and the new
  `1.1.1` probing `failed`, the outcome is `Failed`, `1.1.1` is gone, `1.1.0` is back and the
  returned row describes `1.1.0` (D16 b); with no previous version the failed tree stays (D16 c);
  with the new version `ready`, `1.1.0` is deleted **after** the row was produced (D16 a) and
  `manifest.json` lists the install; D4 in situ: `1.1.0` present with a *newer* mtime than the
  freshly promoted `1.1.1`, the re-probe still resolves `1.1.1`; the post-promote check — a plan
  whose `tool` glob cannot see the promoted `cmd` (the test uses a discovery pattern pointing
  elsewhere) is `InstallError::NotWhereTheRowLooks` naming both paths, and the tree is rolled back
  as in (b); a cancellation token tripped mid-download ends with `Cancelled`, the partial archive
  swept; tripped mid-unpack, same; the `HTUI_TOOL_<NAME>` override still wins after an install
  (`probe_tools` with the override set in `vars` records the override's path, not the installed
  one). Progress frames observed through the callback are monotonic and never more than one per
  250 ms of injected clock.
- **Action**: D16's second half, D18's task-side contract (`InstallOutcome`, `InstallProgress`,
  `CancellationToken` from `tokio_util::sync`, which the gate's compile probe proved needs **no**
  extra `tokio-util` feature — the workspace's `default-features = false, features = ["compat"]`
  already compiles and runs it, so the `AtomicBool` + `Notify` fallback is struck).
- **Validate**: `cargo test -p htui-agent --features test-support`; Windows clippy line

### T7: Requests, replies, runtime (milestone 3; serial after T6)
- **Files**: `crates/htui/src/store_worker.rs`, `crates/htui/src/agent_worker.rs`
- **Tests first** (inline `mod tests`, mirroring `:1231-1273`):
  `the_loop_answers_other_requests_while_an_install_is_in_flight` (an `InstallConfirm` at seq 1
  over a fixture server that stalls the body for 2 s, `Workspaces` at seq 2, the first reply is
  seq 2); `a_plan_is_refused_before_any_request_on_a_buffered_writer` (`REGISTRY_ON_SERVER_ONLY`,
  `background_len() == 0`, the fixture server saw nothing); `a_second_plan_while_one_runs_is_refused`;
  `a_plan_for_a_row_without_install_is_refused_by_name`; `confirm_streams_frames_at_the_confirm_seq`
  (every `Install(..)` reply carries seq 1, ends with `Done` or `Failed`, and the agents
  reply after it shows the re-probed row); `cancel_answers_cancelling_then_the_stream_ends_cancelled`;
  `shutdown_aborts_a_running_install_and_leaves_nothing_resolvable`; `try_serve` without a runtime
  answers "no agent runtime in this build" for all three.
- **Action**: D18 — three variants, `name()` arms, loop interception, `try_serve` refusal,
  `LiveInstall`, `with_installer`, `run_plan`/`run_install` as plain `async fn`s the harness can
  poll (the `run_chat` rule).
- **Validate**: `cargo test -p htui --features testkit`; `cargo clippy --target x86_64-pc-windows-msvc -p htui --all-targets --all-features -- -D warnings`

### T8: The Settings action (milestone 3, with milestone 4's fallback rendering; serial after T7)
- **Files**: `crates/htui/src/ui/tabs/settings/agents.rs`, `crates/htui/tests/settings.rs`,
  `crates/htui/tests/install.rs`, `crates/htui/tests/snapshots/settings__agents_demo.snap`,
  `settings__agents_probed.snap`, `settings__agents_unknown_row.snap`, `settings__agents_empty.snap`
- **Tests first** (`tests/settings.rs`, the `probed_row` style): `j`/`k` move the cursor and wrap
  nowhere; `i` on a row without `install` puts "nothing declares how to install `claude`" on the
  status line and sends nothing; `i` while `probing` is refused; the cell renders each frame
  (`planning…`, `downloading 42%`, `verifying…`, `unpacking…`, `probing…`) and the pane renders a
  `Plan` with every D13 line, the digest line in both wordings, a `Failed { manual }` as the steps,
  a `Cancelled` as a status message; snapshots re-accepted for the hint line. (`tests/install.rs`,
  harness over the fixture server, D19 end to end): `i` then `settle` shows the consent pane and
  the recorder has seen no `GET /archive`; `n` clears it and nothing was fetched; `y` → the stream
  → `Done` → a fresh `Agents` read → the `on this box` cell reads **`failed`** (the fixture `cmd`
  is a shell script the production `SpawnTier2` cannot handshake — the probe is the authority,
  and this is the broken-tree metric at the app level); `x` mid-download → `Cancelled`, the
  staging entry swept, the cell back to its previous text; `r` during an install refused.
- **Action**: D19, D20's rendering.
- **Validate**: `cargo test -p htui --features testkit`; `cargo insta` review of the four snapshots

### T9: `amp-acp` live — the `R-AGT-5` proof (milestone 4; after T6; no shared files with T7/T8)
- **Files**: `crates/htui-agent/tests/install_live.rs`
- **Test first**: `#[ignore]`, run by hand, module doc with the run line. It writes an `amp-acp`
  registry **row as JSON inside the test** (a `Glob` tool `%HTUI_AGENTS_ROOT%/amp-acp/*/amp-acp`,
  `install { acp_registry, "amp-acp", "amp-acp" }`, `handshake: true`), points
  `HTUI_AGENTS_ROOT` at a temp directory through `ProbeEnv.vars` so the maintainer's real root is
  untouched, runs the **production** `plan()` against the real registry and then `install()` with
  `SpawnTier2`, and asserts: the plan carried `Some(sha256)` and `license == "Apache-2.0"`; the
  download verified; the tree promoted; the re-probe resolved the `cmd` the registry named and
  completed `initialize` at `protocolVersion 1`; the status is `ready` **or** `unauthenticated`,
  printed — whether `amp-acp` advertises auth methods on a box with no Amp credential is observed,
  never assumed (the `agy_live.rs` rule); no surviving process. Skips by name, not by failure,
  when the registry is unreachable.
- **Validate**: `cargo test -p htui-agent --features test-support --test install_live -- --ignored --nocapture`

### T10: Close-out (serial, last)
- **Files**: `README.md`, `crates/htui-agent/tests/agy_live.rs`, `HANDOFF.md`,
  `.claude/prds/mod-20-registry-adapter-install.prd.md`, this plan
- **Action**: D22. Maintainer acceptance step recorded, not tested: run `i` on the `agy` row on
  this box (a same-version re-install of 1.1.1, which exercises D16's swap and D17's recorded
  digest against the archive the box already holds) and report the row's status afterwards.
  `agy_live.rs:6`'s "is not a failing build" stays true.
- **Validate**: the full block below

## Validation

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features -- -D warnings
cargo clippy --target x86_64-pc-windows-msvc -p htui --all-targets --all-features -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features

# T9, explicitly (36 MB from the real registry into a temp root; no model tokens)
cargo test -p htui-agent --features test-support --test install_live -- --ignored --nocapture

# still true after T10 (the adapter this box holds; burns no tokens)
cargo test -p htui-agent --features test-support --test agy_live -- --ignored --nocapture
```

`USERNAME=htui-ci` is TOOL-2's standing workaround. No task here adds a migration, so
`cargo sqlx prepare --check` is unaffected; it is run once at T10 to prove that.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| A second rustls crypto provider enters the tree through a feature and `ClientConfig::builder()` panics at run time | Low | High | D9's `rustls-no-provider`; T4's "a client builds" test runs in the suite so the panic would be a red test, not a field failure |
| The registry document changes shape (mutable, `max-age=300`, no pinned URL) | Medium | High | D12's defensive types with every optional field defaulted; the pinned fixture; a missing platform is `NotAvailable`, a strange suffix `Unsupported`, both before consent |
| A staging or promote path lands inside the glob's reach | Low | High | D16's layout is a sibling of `<id>/`; T5's kill-mid-install test asserts `resolve_tool` answers `None` |
| D14 changes what a box that installs nothing resolves | Medium | High | The JetBrains test is kept verbatim as the fallback's pin; the semver rule applies only where a capture parses; the Windows half of the fallback is MOD-16's |
| D15's token breaks a box whose adapter was placed by hand under the old spelling | Low | Medium | The helper answers the same three roots the seed spelled (`data_local_dir`); only `XDG_DATA_HOME` boxes move, and that is stated; `HTUI_AGENTS_ROOT` is the escape hatch |
| The section grows a second responsibility (cursor + consent + progress) and drifts from the one-column-per-fact shape | Medium | Low | The consent pane and the hint are two `Paragraph`s under the same `Table`; the cell text is one `match` in `on_box_cell`; a MOD-7 quota column adds a column, not a mode |
| Windows rename fails under Defender and the install reports failure with a whole tree in staging | Medium (on Windows) | Medium | D21's retry; the sweep on the next attempt; the fact deferred to MOD-16 by name |
| `hyper` and friends enlarge the Windows clippy build set and the audit surface | High | Medium | The whole client is `install/http.rs`; D9 records why the smaller `ureq` lost; no `http2`/`gzip`/`json` |
| The user reads the installer's `Done` as "works" while the row says `failed` | Low | High | `Done` carries the probe's row and the cell shows the probe's status; there is no separate "installed" state anywhere in the UI |
| The maintainer's real install root is touched by a test | Low | High | Every test injects `HTUI_AGENTS_ROOT` through `ProbeEnv.vars`; `tests/install.rs` asserts the injected root is under `tempdir()` before touching disk |
| `amp-acp`'s live behaviour (auth methods, its `cmd` layout) differs from the registry's promise | Medium | Low | T9 asserts consistency with the probe's own answer and prints the rest; a difference is a finding for the phase note, not a red build |

## Verified claims

Checked against the tree at `7392584` and the live endpoints on 2026-09-09, before the tasks were
written. Where the routing brief supplied a fact, it was re-read at the cited line; three findings
are recorded as findings.

| Claim | Verdict | Evidence |
|---|---|---|
| `Discovery` carries the `default` + `skip_serializing_if` pair for `credential` and its reason | true | `launch.rs:103-108` |
| `newest()` maxes `(mtime, PathBuf)` with unreadable metadata as the epoch; `walk` returns `Vec<PathBuf>` | true | `probe.rs:705-723`, `:666-703` |
| `expand` puts every leading literal segment into the root, so `.staging/` beside `<id>/` is never walked | true | `probe.rs:570-581` |
| `%VAR%` expands from `env.vars` with case-insensitive lookup on Windows; a backslashed value stays one segment | true | `probe.rs:134-146`, `:546-547`, `:588-608` |
| `ProbeEnv::host` fills `home` from `dirs::home_dir()` and snapshots `vars`; no `data_dir`/`data_local_dir` call exists in the workspace | true | `probe.rs:99-116`; `grep dirs::` → `identity.rs:46`, `probe.rs:111` only |
| The seeds hand-write three roots for the `htui`-managed install | true | `agent_agy.json:19-28` |
| **Finding — the antigravity zip records unix modes** (`0775` on the `.par`), so an extractor that applies them would make `chmod +x` redundant for *this* archive; D10 sets the bit regardless | finding | tail range request, central directory entries, external attributes `33277`/`33149` |
| **Finding — the zip is deflate-only, no zip64; amp-acp is gzip** | finding | same request; `1f 8b 08 00` on the amp-acp URL after its 302 |
| **Finding — `sha2` is in `Cargo.lock` at two versions** (0.10.9 workspace, 0.11.0 transitive) | finding | `Cargo.lock` |
| No `hyper`, `http`, `tokio-rustls`, `webpki-roots`, `flate2`, `crc32fast`, `zip`, `tar` in the lock; `rustix`, `windows-sys`, `miniz_oxide`, `filetime`, `indexmap`, `memchr` present | true | `Cargo.lock` grep |
| `reqwest 0.13.5` has no native-roots feature; `rustls-no-provider` requires `rustls-platform-verifier` | true | `cargo info reqwest` feature table |
| `ureq 3.4.1` is blocking; its `rustls` feature is `ring` + `webpki-roots` | true | `cargo info ureq` |
| `zip` stable is 8.6.0 (MSRV 1.88); 9.0.0-pre3 is a pre-release | true | crates.io index `3/z/zip` |
| `fs4 1.1.0` MSRV 1.75, `rustix`/`windows-sys` only | true | index `3/f/fs4` |
| `tokio`'s workspace features omit `net` | true | `Cargo.toml:20-21` |
| `AgentRuntime::probe` takes the `Writer` first, refuses `Buffered`, pushes a spawned task, returns `Deferred`; the loop `continue`s on `Deferred` | true | `agent_worker.rs:373-414`; `store_worker.rs:484-498` |
| `App::is_fresh` is `latest[(origin, discriminant(request))] == seq`; replies to a popped overlay are dropped | true | `app/state.rs:158, 269-294`; `app/update.rs:166-181` |
| The overlay factory takes no argument; `MigrationPrompt` reads a parameterless `StoreState` | true | `overlay/registry.rs:80`; `migration_prompt.rs:84-87` |
| The agents section has no row cursor; `r` is a literal arm guarded by `probing`; `on_reply` clears `probing` on any `Agents` | true | `agents.rs:107-147`, `:149-212` |
| The tab consumes `h`/`l`/`[`/`]`/arrows before a section; the global table binds `q ? digits ctrl-c -` | true | `settings/mod.rs:197-213`; `agents.rs:108-109` |
| `crates/htui-store/migrations/` holds `0001`, `0002`; MOD-4's `0003` does not exist; ANA-2 §9 says it lands after `0002` | true | directory listing; `docs/ANA-2.md:67`, HANDOFF MOD-4 line |
| `WriteStore` has no box-settings or app-setting write; `box.settings` is documented as MOD-7's | true | `store/traits.rs:57-154`; `box_.rs:55-56` |
| `launch::spawn` runs `which` even on an absolute command | true | `launch.rs:741-743` |
| `README.md:268-274` is the authentication paragraph (MOD-21's) | true | read |
| `Discovery` literal sites needing the new field: 4 | true | `tools.rs:148`, `tests/probe.rs:143,1998`, `tests/acp_driver.rs:525` |
| Task independence: (T2→T3) ∩ (T4→T5→T6) = ∅ by file; T1 ∩ T2 = seeds + `tests/probe.rs` (serial); T3 ∩ T2 = `probe.rs` + `tests/probe.rs` (serial); T7 ∩ T8 = ∅ by file but T8 needs T7's types (serial); T9 ∩ everything = ∅ | true | file sets above |

## Fact-check verdicts (`/handoff-run` step 3.5, 2026-09-09)

Run by the router on the session model **after** the plan was drafted and **before** the CONFIRM
gate, independently of the plan's own "Verified claims" table above. Every claim that could change
the plan's shape was re-checked against the tree, the crates.io index, a compile probe on this
toolchain, or the live endpoints. Two amendments were made to the plan as a result; they are folded
into D9, D11, T4 and T6 above rather than left as notes.

| Claim | Verdict | Evidence |
|---|---|---|
| D9: `rustls-no-provider` lets rustls "select the single enabled provider", so no explicit install is needed | **FALSE — plan amended** | Compile-and-run probe, `reqwest 0.13.5` + `rustls 0.23` with only the `ring` feature: `Client::builder().build()` **panics** — *"No rustls crypto provider is configured. When using the `rustls-no-provider` feature you must install a crypto provider before building a Client"* (`reqwest-0.13.5/src/async_impl/client.rs:2501`). D9 now requires `rustls::crypto::ring::default_provider().install_default()` (Err ignored) and a direct `rustls` dependency |
| The amended form works end to end | true | Same probe with the install call: client builds, live `HEAD` to `dl.google.com` → `200`, `accept-ranges: bytes` |
| The archive size can be read from a `HEAD` | true, **but not the way the plan implied — amended** | Same probe: `Response::content_length()` is `Some(0)` for a `HEAD` while the header says `681969407`. D11/T4 now specify the header |
| T6: `CancellationToken` may need an extra `tokio-util` feature | **false — the doubt is resolved, fallback struck** | Compile probe, `tokio-util 0.7.19` with `default-features = false, features = ["compat"]` (the workspace's own set): `CancellationToken::new()`, `child_token()`, `cancel()` compile and run. `tokio-util` has no `sync` feature at all in its manifest |
| `reqwest`'s `rustls` feature would drag `aws-lc-rs`, so `rustls-no-provider` is the right choice | true | index `features2`: `rustls = ["__rustls-aws-lc-rs", "dep:rustls-platform-verifier", "__rustls"]`; `rustls-no-provider = ["dep:rustls-platform-verifier", "__rustls"]` |
| `reqwest`'s `stream` and `system-proxy` are the right feature names and add nothing surprising | true | index: `stream = ["tokio/fs", "dep:futures-util", "dep:tokio-util", "dep:wasm-streams"]` (the workspace already enables `tokio/fs`); `system-proxy = ["hyper-util/client-proxy-system"]`. `default = ["default-tls", "charset", "http2", "system-proxy"]`, so `default-features = false` is load-bearing |
| `zip 8.6.0` has a `deflate-flate2` feature and its defaults must be turned off | true | index `features2`: `deflate-flate2 = ["_deflate-any", "dep:flate2"]`; `default = ["aes-crypto", "bzip2", "deflate64", "deflate", "lzma", "ppmd", "time", "zstd", "xz"]` |
| `fs4 1.1.0` offers `sync` and clears MSRV | true | index: features `["default", "sync"]`, `rust_version 1.75.0`, normal deps `rustix`/`windows-sys` (+ optional async runtimes) |
| D15: `dirs::data_local_dir()` is exactly the three roots the seeds hand-write | true | `dirs-6.0.0/src/lin.rs:12` (`= data_dir()`, i.e. `$XDG_DATA_HOME` or `~/.local/share`), `mac.rs:13` (`app_support_dir()`), `win.rs:11` (`known_folder_local_app_data()` = `%LOCALAPPDATA%`) |
| D14: `the_jetbrains_two_star_shape_resolves_to_the_newest_install` survives version-aware `newest()` unchanged | true — and for the reason the plan gives | `tests/probe.rs:389-437` read in full: its last capture is a build number (`20260501`, `20260818`), which `semver::Version::parse` rejects, so **both** assertions stay on the mtime-then-path-descending fallback. Rust's `Ord for Option` (`None < Some`) is what puts a parsable version above an unparsable sibling |
| Task independence by file set | true | Chain A `{probe.rs, agent_agy.json, tests/probe.rs}` ∩ chain B `{Cargo.toml ×2, lib.rs, install/*, tests/install.rs, tests/fixtures/*}` = ∅; T7 `{store_worker.rs, agent_worker.rs}` ∩ T8 `{settings/agents.rs, tests/settings.rs, tests/install.rs, 4 snaps}` = ∅ (serial anyway, T8 needs T7's types); T9 `{tests/install_live.rs}` ∩ all = ∅. T1 touches the seeds and `tests/probe.rs`, so it must land before both chains, which is what the plan says |
| `crates/htui/tests/settings.rs` really has the `probed_row` + `render_section` helpers the plan mirrors | true | `settings.rs:35` `render_section`, `:186` `probed_row` |
| `tokio`'s workspace feature set omits `net` (so T4's fixture server needs it as a dev-dependency) | true | `Cargo.toml:20-21` |
| `semver` is already a direct workspace dependency, so D14 adds nothing | true | `Cargo.toml:53` |

One item is **not** a fact question and goes to the maintainer at CONFIRM rather than being settled
here: D13 reads the PRD metric "no request is issued before consent" as *no archive **body** byte*,
and issues one `HEAD` before the consent text so the size can appear in it. That is a defensible
reading of `R-AGT-10` ("told that before the download") and the PRD's own scope does put the size in
the pre-flight, but it is a narrowing of the metric's literal wording and the maintainer should say
so explicitly.

## Acceptance

1. A row declares its source; `plan()` answers archive / verification / terms / size / disk for this
   box from a cached document, and a platform the entry lacks is "not available here" (milestone 1).
2. A fixture archive becomes a tree at the path the seed glob reads, executable, digest-checked
   where published and refused on mismatch, with a kill mid-install leaving nothing resolvable
   (milestone 2).
3. `i` in Settings shows consent with licence, terms URL, size and the digest sentence **before**
   any archive byte is requested; the download streams progress; the loop answers other requests
   meanwhile; the row's status afterwards is the probe's (milestone 3).
4. A downgrade or a vendor-dated mtime no longer misselects; the previous version is deleted only
   after the new one probes usable; a failed re-probe restores the previous version; no network
   degrades to derived manual steps (milestone 4).
5. `amp-acp` installs live from a row the test wrote, through the same code path, differing only
   by data; no source file outside seeds and tests names a vendor (`R-AGT-5`, `R-AGT-10`).
6. `HTUI_TOOL_<NAME>` still wins over anything installed; `htui-store` and `migrations/` unchanged.
7. Full suite green on Linux with Postgres live; both Windows clippy lines green.
8. `rust-reviewer` gate clear.

## Close-out

Per `.claude/rules/workflow-docs.md` lifecycle step 4: MOD-20 stays one open `HANDOFF.md` checklist
line with a **phase note per milestone** (each naming the commit, the tests that prove it, and any
amendment to this plan), and archives as one write-up when milestone 4 lands. The PRD's four rows
move to `complete` as each phase note is written. The note for milestone 4 records the two MOD-2
plan amendments — D48's mtime rule (amended by D14) and D57's "`htui` does not download" (reversed
by `R-AGT-10`) — and the `XDG_DATA_HOME` behaviour change of D15. Push only when agreed.
