# MOD-20 — Registry-driven adapter install

> Routed as **PRD** by `/handoff-run MOD-20` (criteria C2, C3, C4 fired). Ultracode recommended for
> the implement and review phases. Unlike MOD-2, no ANA precedes this item: `docs/ANA-4.md` §4.6
> *deferred* the work rather than designing it, so this document carries the evidence a design would
> otherwise cite, and the plan settles the mechanism. `R-AGT-10` is the contract.

## Problem

A box that cannot run an agent's adapter is a box `htui` cannot use that agent on, and today the
only cure is a paragraph of `README.md` the user follows by hand. `README.md:234-267` is that
paragraph: a `curl` pinning **one** version (`1.1.1`) and **one** platform (`linux-x86_64`), a
`mkdir -p` naming a path the seed glob happens to expect, an `unzip`, and a `chmod +x` with a
footnote explaining that `launch::spawn` runs `which` even on an absolute path and so rejects a file
without the executable bit. The same coordinates are duplicated a third time as a test constant
(`crates/htui-agent/tests/agy_live.rs:101-111`, `INSTALL_HINT`). Every one of those goes stale
silently when the registry moves — and the registry moves: the document `htui` would read was last
modified **2026-09-09 05:37 UTC**, hours before this item was routed.

The gap is asymmetric with everything around it. The probe already answers *what is installed*
truthfully, per box, without configuration (`R-AGT-6`, MOD-2 milestone 5). MOD-21 will make an
installed-but-unauthenticated agent actionable (`R-AGT-9`). Between the two sits the one state
`htui` observes and refuses to act on: `ProbeStatus::Missing` — the row resolves nowhere, and the
app's entire answer is a static column reading `missing`.

## Evidence

Every fact below was verified against the live registry and this box during routing, not recalled.

- **The registry already carries the whole install recipe, in `htui`'s own vocabulary.** The
  document (`https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`, schema `1.0.0`,
  56 KB) lists **40 agents**, of which **19** distribute a `binary`. Each platform entry is
  `{ archive, cmd, args?, env?, sha256? }` keyed by `<os>-<arch>` — the exact key set
  `probe::platform_key()` produces (`probe.rs:152-159`) and the exact shape `PlatformGlob
  { patterns, args }` already models (`launch.rs:206-216`). `antigravity-acp`'s `linux-x86_64`
  entry carries `"cmd": "./agy_acp_server.par"` and `"args": ["--uid="]` — the argument milestone 6
  proved mandatory. **An installer reads a recipe `htui` already knows how to spell.**
- **Digest coverage is genuinely partial, and it is not an antigravity quirk.** 10 of the 19 binary
  entries publish `sha256` per platform (`amp-acp`, `goose`, `harn`, `kilo`, `kimchi`, `kimi`,
  `mistral-vibe`, `opencode`, `poolside`, `sigit`); 8 publish none, `antigravity-acp` among them.
  `R-AGT-10` is written for exactly this split.
- **Size is knowable before the download even though the registry never states it.** The registry
  has no size field anywhere. `HEAD` answers instead: `antigravity-acp` `linux-x86_64` is
  **681 969 407 bytes** with `accept-ranges: bytes`; `amp-acp` `linux-x86_64` is **36 091 565 bytes**,
  behind a 302 to GitHub's CDN, also range-capable. Two orders of magnitude apart — a fixed disk
  constant would be wrong for one of them.
- **Unpacked cost is measured, not estimated.** `~/.local/share/htui/agents/antigravity-acp/1.1.1/`
  on this box is **1.9 GB**: `agy_acp_server.par` 1 880 360 328 bytes plus `localharness_external`
  128 966 920 bytes.
- **The mtime hazard is real and observable here.** Those two files carry mtime **2026-09-02**, the
  archive's build date preserved by `unzip`, while the directory that holds them was created
  2026-09-08. `newest()` (`probe.rs:711-723`) maxes on `(mtime, PathBuf)`, so version selection
  keys on *when the vendor built it*, and its tie-break is lexicographic path descending — under
  which `1.9.0` beats `1.10.0`.
- **The workspace has no download machinery of any kind.** No `reqwest`, `ureq`, `hyper`, `zip`,
  `tar`, `flate2`, `zstd`, `bzip2` in any manifest or in `Cargo.lock`. `rustls` and `url` are
  present transitively for the Postgres TLS transport only; `sha2` is already a first-party
  workspace dependency (`Cargo.toml:37`), so hashing is the one piece already in hand.
- **The seam this fills changes no resolution rule.** `agent.launch.discovery` is per-agent data
  (`launch.rs:92-109`), the glob tier resolves `…/agents/<id>/*/…` with the version segment already
  a wildcard, and `AcpAdapter::build` spawns `agent_box.probe.resolved` (MOD-2 D58). An installer
  supplies a *source* for the file the glob finds.
- **Five open items name a box's readiness as their concern** — MOD-7 (box registration and the
  probe hook that would call this), MOD-16 (every Windows runtime fact), MOD-21 (the other half of
  "make this box ready"), and MOD-2, whose README section and `INSTALL_HINT` this replaces.

## Users

- **Primary**: the maintainer standing up `htui` on a new box, holding a registry row whose adapter
  is not there. Today they leave the app, read a README, and paste four commands. The need fires the
  first time `Settings` shows `missing`.
- **Also served**: anyone adding a **third** agent. `R-AGT-5` says a new agent is one registry row;
  today it is one registry row *plus* a hand-written install ritual nobody documented for that
  agent. This item makes the ritual declarative.
- **Also served**: MOD-7, whose box registration and probe hook is the natural trigger for an
  install offer, and MOD-16, which inherits the Windows unpack path as a runtime fact to verify.
- **Not for**: agents distributed by `npx`/`uvx` (≈21 of the 40 registry entries) — those already
  resolve through `ToolProbe::NodePackage` and its `npx` fallback, and nothing here changes them.
  Not a general package manager: `htui` installs adapters its own registry rows declare, not
  arbitrary registry ids.

## Hypothesis

We believe **a declared install source in the agent's own registry row, resolved against the ACP
registry and executed off the UI task**, will **turn `missing` from a report into an action** for
**the maintainer and every box after the first**.

We'll know we're right when **an agent nobody wrote install code for — `amp-acp`, a different
vendor, a different archive format, a published digest and a permissive licence — installs, verifies,
unpacks and re-probes to `ready` through the same code path that installs Google's unverifiable
682 MB proprietary zip, differing only by registry row.**

That clause is the proof obligation, exactly as `R-AGT-5`'s was for MOD-2: an installer that works
for the two seeded agents and would need a new `match` arm for a third has not met `R-AGT-10`.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Vendor-specific code paths | 0 | No agent name, registry id, archive URL or vendor string appears outside seed JSON and test fixtures; `R-AGT-5`, `R-AGT-10` |
| Third agent installed with no code change | `amp-acp` (36 MB, `sha256`, Apache-2.0, `.tar.gz`) installs and re-probes to `ready` from a registry row alone | Live test on this box, mirroring `crates/htui-agent/tests/agy_live.rs` |
| Digest handling | Published digest verified and a mismatch refuses the install; absent digest stated **before** the download | `R-AGT-10`; unit tests over both registry entries plus a corrupted-archive case |
| Licence surfaced before fetch | Proprietary terms and `license_url` shown and accepted before a byte is requested | `R-AGT-10`; a test asserting no request is issued before consent |
| Probe is the authority | Post-install status comes from a re-probe, never from the installer's own claim | `R-AGT-6`, `R-AGT-10`; a test where an install "succeeds" into a broken tree and the row still reads `missing`/`failed` |
| UI never blocks | A 682 MB download leaves the TUI responsive and every other request answered | `R-NF-3`; the `store_worker.rs:1231-1270` pattern extended to the install request |
| Override precedence | `HTUI_TOOL_<NAME>` still wins over anything installed | `tools.rs:42-53` unchanged; regression test |
| Interrupted install | A killed or failed install leaves nothing the glob will resolve | Test that kills mid-unpack and asserts `resolve_tool` still answers `None` |
| Manual path preserved | With no network (or behind a refusing proxy) the action degrades to the documented manual instructions rather than failing opaquely | Test with an unreachable registry host |

## Scope

**MVP** — a declared install source, a registry reader, a consented and verified download, an
unpack that is invisible until it is complete, and a re-probe that decides the outcome.

Concretely in scope:

- **An `install` block in `agent.launch.discovery`**, naming the ACP registry id and the tool
  placeholder it fills. It is data: `#[serde(default, skip_serializing_if = "Option::is_none")]`
  alongside `credential`, per the round-trip rule at `launch.rs:103-106`. Both seed rows gain one.
- **A registry reader** for `registry/v1/latest/registry.json`: fetch, cache with `ETag` /
  `cache-control: max-age=300`, select this box's `platform_key()` entry, and answer with the
  archive URL, `cmd`, `args`, `sha256` where present, plus `license` / `license_url`. A platform
  the entry does not list is a first-class "not available for this box".
- **A pre-flight the user consents to**: agent, version, archive size (from `HEAD`), the licence and
  its terms URL, and — in plain words — whether a digest will be verified or cannot be.
- **Download, verify, unpack, promote**: into a staging location, digest-checked where published,
  then made visible at the path the glob already reads, with the executable bit set (`launch::spawn`
  runs `which` even on an absolute path). The **whole** archive is unpacked, not the named binary
  alone — `localharness_external` ships beside `agy_acp_server.par`.
- **A Settings action beside `r`** (`R-TUI-8`), served the way `ProbeAgents` is served: through
  `AgentRuntime::serve` returning `Served::Deferred` with an owned `Writer`, never in the worker's
  `select!` arm and never on the UI task (`R-NF-3`, `agent_worker.rs:366-414`).
- **Progress the user can read.** A 682 MB download behind a single static `installing…` cell is
  indistinguishable from a hang. The existing "one request, many replies" mechanism carries it
  (`App::is_fresh`, `app/state.rs:290-294`; `StoreReply::Chat` is the precedent) — no new
  architecture, one new frame type.
- **A re-probe of the row on success**, through `probe`'s existing path, so `agent_box` is written
  by the probe and not by the installer (`R-AGT-6`).
- **A shared install-root helper** (D5) that both the installer and the seed's glob patterns agree
  with by construction, on all three platforms.
- **Version-aware resolution** (D4): `newest()` stops keying on the archive's build mtime. This
  changes shipped MOD-2 behaviour and is in scope deliberately.
- **Retention** (D3): the version being replaced is deleted once the new one is promoted and
  probed, and a `HEAD`-derived size pre-flight may refuse an install before the download.
- **Per-box licence consent** (D6), remembered so an accepted proprietary licence is not re-asked on
  every install of the same adapter on that box.
- **Documentation replacement**: `README.md:234-267` becomes a description of the in-app action plus
  the `HTUI_TOOL_<NAME>` escape hatch and the no-network fallback; `INSTALL_HINT` in
  `agy_live.rs:101-111` follows it.

**Out of scope**

- **Authentication** — MOD-21 (`R-AGT-9`). Installing an adapter and logging it in are one story to
  the user and two items; this one ends at `unauthenticated`, which is a correct outcome here.
- **Windows runtime verification** — MOD-16. This item writes and lint-checks the Windows unpack
  path (long paths, the `.cmd` shim rule of `docs/ANA-4.md` §4.6, Defender); whether it *behaves* on
  a Windows box is MOD-16's, as it was for every MOD-2 milestone.
- **Installing the agent CLI itself** (`agy`, `claude`) — only adapters are declared installable.
- **Automatic or background updates.** The user asks; nothing installs on a timer, on startup, or as
  a side effect of `PROBE_TTL` expiry.
- **Non-`binary` distributions** (`npx`, `uvx`) — already served by `ToolProbe::NodePackage`.
- **A box with no server.** The probe refuses a `Writer::Buffered` before spawning anything
  (`agent_worker.rs:373-414`, `REGISTRY_ON_SERVER_ONLY`); this action refuses on the same terms.
  Local-only boxes gain a registry through MOD-17, and this item is not the place to invent one.
- **Uninstall as a general feature** beyond whatever retention requires.

## Constraints (fixed before planning)

- **`R-AGT-5` is structural.** No agent name, registry id or vendor URL outside seed JSON and
  fixtures. A third agent is a registry row.
- **Nothing here writes `agent`.** What is installed is a fact about a *box*: it belongs to
  `agent_box` and reaches it through the probe.
- **`HTUI_TOOL_<NAME>` keeps winning** over anything installed (`tools.rs:11-14, 42-53`).
- **The probe is the authority on outcome** (`R-AGT-6`): an install that cannot be probed into
  `ready`/`unauthenticated` did not succeed, whatever the download reported.
- **`R-NF-3` is enforced by ownership, not care**: no store handle on the render side, and no long
  await inside the worker's `select!` arm.
- **The overlay factory signature is `Fn() -> Box<dyn Overlay>`** with no constructor argument
  (`overlay/registry.rs:80`, `migration_prompt.rs:85-87`), so any consent view receives its text the
  way `MigrationPrompt` receives its count — through `wants_requests` and `on_reply`.
- **`unsafe_code = "forbid"`, MSRV 1.98, workspace lint set unchanged; TDD per repo convention.**
- **Every new dependency is a decision, not a detail** — the workspace currently has no HTTP or
  archive crate at all; D1 fixes the shape (Rust client, Rust extractors), the plan picks the
  crates and justifies each.
- **Editing `newest()` is sanctioned** (D4) and is the only shipped MOD-2 resolution rule this item
  may change. Its existing tests are updated, not deleted, and a box that installs nothing must be
  shown — by test — to resolve the same file it resolves today, except where mtime ordering was
  giving the wrong answer.

## Delivery Milestones

<!-- Business outcomes, not engineering tasks. /plan turns each into a plan. -->
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Declared source + registry reader | A row states where its adapter comes from, and `htui` can answer — offline-testably, from a cached document — "for this box, this agent installs from *this* archive, verified *this* way, under *these* terms, at *this* size". Nothing is downloaded yet. | complete | [plan](../plans/mod-20-registry-adapter-install.plan.md) |
| 2 | Fetch, verify, unpack, promote | An archive becomes a working tree at the path the glob reads, with the executable bit set, the digest checked where published, and an interruption leaving nothing resolvable. | complete | [plan](../plans/mod-20-registry-adapter-install.plan.md) |
| 3 | The action in the app | The maintainer installs from Settings: consent first with licence and size, progress while it runs, a re-probe deciding the result, and a responsive TUI throughout. | complete | [plan](../plans/mod-20-registry-adapter-install.plan.md) |
| 4 | Living with it | Retention and disk policy, version selection that survives re-installing an older version, the no-network and proxy fallbacks, `README.md` and `INSTALL_HINT` replaced, and `amp-acp` installed live as the `R-AGT-5` proof. | complete | [plan](../plans/mod-20-registry-adapter-install.plan.md) |

Milestones 1 and 2 are testable without a UI and without the network (a fixture registry document
and a fixture archive); milestone 3 is the first that needs the store worker; milestone 4 is the one
that closes `R-AGT-10`. Milestone 1 alone already replaces the README's hardest sentence — the
version pin — because the registry, not the document, becomes the coordinate.

## Decisions taken at the PRD gate

Answered by the maintainer on 2026-09-09, before planning. Each was a genuine fork in the plan's
shape; the facts each rests on are under Evidence. They are decisions, not proposals — the plan
implements them and records deviations as it would from any other constraint.

- **D1 — Rust client, Rust extractors.** An HTTP client in Rust (reusing the `rustls` already in
  the tree for the Postgres transport) plus Rust archive extraction for `.zip` and `.tar.gz`. Not a
  shell-out to `curl`/`unzip`: this code runs inside the TUI's own runtime and needs progress bytes
  and cancellation, neither of which a spawned process gives cleanly, and a shell-out would make the
  box's tooling a hidden requirement — worst exactly on Windows, where MOD-16 cannot verify it
  cheaply. The specific crates are the plan's call; the shape is not.
- **D2 — Consent states the absence, and the computed digest is recorded.** An archive whose
  registry entry publishes no `sha256` is still installable, but the user is told there is nothing
  to verify against **before** the download (`R-AGT-10`'s literal requirement). `htui` hashes what
  it received anyway and records it, so a later re-install of the *same version* producing a
  different digest is detectable. A published digest that mismatches refuses the install outright.
- **D3 — The previous version is deleted on success.** Not "keep N", not "never delete". At 1.9 GB
  per antigravity version an unbounded install root is a bug with a slow fuse. Deletion happens
  only after the new version is promoted and probed, so a failed install never costs the working
  one. A size pre-flight from `HEAD` may refuse an install before the download.
- **D4 — `newest()` becomes version-aware.** Option (b): the resolver stops keying on mtime, which
  `unzip` sets to the vendor's build date rather than the install date, and which ties on
  lexicographic path where `1.9.0` beats `1.10.0`. This **edits shipped MOD-2 resolution**
  (`probe.rs:711-723`) and its tests, and changes selection for boxes that never install anything —
  deliberately, because the current ordering is wrong for them too. Semver ordering where the
  version segment parses as one (`semver` is already a workspace dependency, `Cargo.toml:53`),
  with a defined and tested fallback where it does not.
- **D5 — A shared helper owns the install root, for every platform.** Not the installer computing
  one root while the seed hand-writes matching glob patterns for three others: one helper answers
  "where does this box keep installed adapters", the installer writes there, and the seed patterns
  and Windows spelling (`%LOCALAPPDATA%\htui\agents\…`) agree with it by construction rather than by
  review. Windows is the reason — the two must not be able to drift there, where no test on this
  box would catch it.
- **D6 — Licence consent is remembered per box.** Consistent with `R-AGT-9`'s framing and with this
  item's own rule that what a box can run is a box fact: consent belongs with the box, not with the
  registry row and not with the user. Where exactly it is stored is the plan's call among the
  existing homes; if it needs a migration, that migration must sequence against MOD-4's
  `0003_orchestration.sql` (`docs/ANA-2.md` §9) rather than race it.
- **D7 — Settings only.** The action lives in the Settings agent section and nowhere else for the
  MVP. D60's chat-spawn-failure re-probe is not extended to offer an install; MOD-7's box
  registration is the intended second caller, and it is a later item.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The registry document changes shape under us (it is mutable, `max-age=300`, no pinned snapshot URL) | Medium | High | Parse defensively into an owned type, treat unknown fields as ignorable and a missing platform as "not available here"; pin a fixture copy for tests; failure degrades to the manual instructions, never to a wrong download |
| An unverifiable 682 MB proprietary binary is fetched on the user's behalf | High (it is the primary agent) | High | Consent before any request, with licence, terms URL, size and the absence of a digest stated; `R-AGT-10` makes this contractual, and Q2's recorded digest makes a silent substitution detectable |
| A partial unpack leaves a directory the glob resolves, producing an agent that spawns and dies | Medium | High | Unpack to a staging path the glob patterns cannot match, promote only on success, and treat promotion as the commit point; the "interrupted install" metric tests exactly this |
| Version selection picks the wrong install after a downgrade | Medium | Medium | D4: `newest()` becomes version-aware, tested with two installed versions where the newer one carries the older mtime |
| D4's edit to `newest()` regresses a box that installs nothing (JetBrains-managed antigravity copies, `%LOCALAPPDATA%\JetBrains\*\acp-agents\`) | Medium | High | The change ships with a test asserting the pre-existing selection for a tree of unrelated versioned directories, and a defined fallback for a version segment that is not semver; MOD-16 owns the Windows half |
| Disk exhaustion on a small box (1.9 GB per version, 118 GB free here) | Low | High | A size pre-flight from `HEAD` plus Q3's retention policy; the failure is refused before the download, not discovered at 90% |
| New dependencies (HTTP + two archive formats) enlarge the build and the audit surface | High | Medium | Q1 decided explicitly with the tree's existing `rustls` and `sha2` reused; the installer is one module with a narrow seam so the client is replaceable |
| Windows unpack differs (long paths, Defender, executable bit is meaningless, `.cmd` shims) | High | Medium | Written and lint-checked from Linux as MOD-2 did (`cargo clippy --target x86_64-pc-windows-msvc`), with runtime verification explicitly MOD-16's |
| The installer's success and the probe's answer disagree, and the user believes the installer | Low | High | The probe is the sole authority on the row; the installer reports what it did, never what the box can now run |
| A proxy or captive network turns a clean failure into a slow hang | Medium | Low | Bounded timeouts on both the registry read and the archive fetch, cancellable from the UI, falling back to the manual instructions |

---
*Status: COMPLETE — all four milestones landed 2026-09-09; write-up at `docs/decisions/mod/mod-20.md`.*
