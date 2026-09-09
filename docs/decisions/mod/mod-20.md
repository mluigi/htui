# MOD-20 - Registry-driven adapter install (done, 2026-09-09)

`htui` installs an ACP agent's adapter itself, from the ACP registry, for **any** agent that
declares how — never one code path per vendor. Satisfies **`R-AGT-10`**, with `R-AGT-4..6`,
`R-TUI-8`, `R-NF-3`.

Artifacts: PRD `.claude/prds/mod-20-registry-adapter-install.prd.md`, plan
`.claude/plans/mod-20-registry-adapter-install.plan.md`, blueprint
`.claude/plans/mod-20-registry-adapter-install.blueprint.md`.

Landed on branch `feat/mod-20-registry-adapter-install`: **`6682406`**
(`feat(agent)` — the install module, the root helper, version-aware `newest()`, the seed's declared
source), **`e3b3b19`** (`feat(tui)` — the three requests, the runtime arms, the Settings action),
and this close-out. The three are split so each builds on its own: `htui-agent` gains the module
before `htui` uses it.

## What it reverses

`docs/ANA-4.md` §4.6 rejected "download every agent from the ACP registry and manage the install
(Zed's approach)" with *"MOD-2 should not become a package manager"*, reserving the shape in
`agent.launch.discovery` for "MOD-7 or a later MOD"; MOD-2's plan D57 restated it. `R-AGT-10` was
added to `docs/REQUIREMENTS.md` on 2026-09-09 by maintainer decision and reversed both. The cost of
the deferral was concrete: `README.md` carried a hand-written `curl` pinning **one** version
(`1.1.1`) and **one** platform, duplicated a third time as a test constant, and the registry
document those coordinates came from had been modified hours before this item was routed.

## What was built

- **A declared source.** `agent.launch.discovery.install` — `{ source, id, tool }`, where `source`
  is a one-value enum (`acp_registry`), `id` is the registry entry and `tool` the `${tool}`
  placeholder whose glob must resolve what the install writes. Data, never code: a third agent is a
  registry row.
- **A registry reader** for `registry/v1/latest/registry.json`, cached under
  `<install root>/.registry/` and honouring `ETag` / `cache-control: max-age` — within `max-age` no
  request is made at all, past it a conditional `GET`, and a network failure with a warm cache
  plans from the cache and says so.
- **A pre-flight** (`plan()`) producing one serialisable `InstallPlan`: the archive URL and format,
  its size, the published digest or its absence, the licence and terms URL, the install directory,
  the versions it would replace, and the disk check. Exactly one registry read and one `HEAD`;
  **no archive byte before consent**.
- **A pipeline** (`install()`): download with a streaming sha256, verify where a digest is
  published, unpack into `<root>/.staging/`, promote with one `rename`, check the row's own glob
  resolves what was written, **re-probe**, then prune or roll back.
- **The Settings action.** `Settings > Agents`, a row cursor (`j`/`k`, the first in any Settings
  section), `i` to pre-flight, a consent pane, `y`/`n`, streamed progress in the `on this box` cell,
  `x` to cancel. Served through `AgentRuntime::serve` as `Served::Deferred` with an owned `Writer`,
  never in the worker's `select!` arm and never on the UI task (`R-NF-3`).
- **`HTUI_AGENTS_ROOT`**, seeded from `dirs::data_local_dir()/htui/agents`. The seed documents no
  longer spell an install root on any platform; one helper owns all three spellings, so the
  installer and the glob agree by construction rather than by review.

## The decisions that shaped it

Gate decisions **D1–D7** were the maintainer's, taken before planning: a Rust HTTP client and Rust
extractors rather than shelling out; consent that states the absence of a digest and records the
computed one anyway; the previous version deleted on success; `newest()` made version-aware; a
shared install-root helper; consent remembered per box; and the action in Settings only.

Plan decisions **D8–D22** followed. The load-bearing ones:

- **D14** amends **MOD-2 plan D48**. `newest()` keyed on mtime, which `unzip` sets to the vendor's
  build date — measured on this box: the installed `agy_acp_server.par` carries `2026-09-02` while
  the directory holding it was created `2026-09-08` — and tie-broke on lexicographic path, under
  which `1.9.0` beats `1.10.0`. It now maxes over `(Option<semver::Version>, mtime, path)`, parsing
  the **last** glob capture. Non-semver captures keep the old rule exactly, which is what leaves the
  JetBrains build-number shape (`20260501`, `20260818`) resolving as before.
- **D17** keeps consent and the recorded digest in `<install root>/<id>/manifest.json`, on the box's
  own disk. `agent_box` would have needed a column, therefore a migration, therefore the `0003`
  MOD-4 has been promised — so **`htui-store` and `migrations/` are untouched by this item** and
  `0003_orchestration.sql` is still MOD-4's next.
- **D16 + P-10** put staging in `<root>/.staging/`, a sibling of `<id>/`. The seed pattern's literal
  root ends at `<id>`, so nothing half-written is ever reachable by the glob. A set-aside previous
  version carries a `.previous` suffix and the sweep *restores* an orphan whose target is absent.
- **D9** was **falsified at the fact-check gate and amended before implementation**: `reqwest`'s
  `rustls-no-provider` does not infer the single enabled provider, and `Client::builder().build()`
  panics with *"No rustls crypto provider is configured"*. `install/http.rs` installs the `ring`
  provider explicitly behind a `Once`.

## What the numbers say

Baseline 485 tests; **589 passing, 0 failing** at close-out, whole workspace with Postgres live.
Verified on **Linux**.

`R-AGT-5`/`R-AGT-10`'s proof is `crates/htui-agent/tests/install_live.rs`: **`amp-acp`** — a
different vendor, a `.tar.gz`, Apache-2.0, with a published digest, and an agent no seed, no source
file and no code path knows — installs from a registry row the test itself writes. Live on this box:
`plan()` 1.05 s, `install()` 5.26 s, 36,091,565 bytes streamed matching the `HEAD` header exactly,
the published `sha256` verified, the tree promoted, the row's own glob resolving it, `initialize` at
`protocolVersion 1`, status `unauthenticated` (`authMethods: ["setup"]`), no surviving process. A
sweep test (`tests/extensibility.rs::the_installer_names_no_vendor`) fails the build if any `src/`
file outside an in-module test block names a vendor.

## The review gate

Two scoped `rust-reviewer` passes. The `htui` half returned one HIGH and two MEDIUM; the
`htui-agent` half **blocked** with one CRITICAL and two HIGH. Every finding was verified against the
tree before being applied. The three that mattered:

- **CRITICAL, arbitrary write through a chained symlink.** `link_stays_inside` counted path depth
  lexically and could not know that a `Normal` component was itself a symlink: `d/a -> ..`, then
  `b -> d/a/..` (lexically depth 1, really the tree's parent), then a plain file `b/pwned` — whose
  target lexically starts with the staging root — was written *through* the link, outside the tree.
  Reachable from any archive without a published digest, which per `R-AGT-10` includes every
  `antigravity-acp` platform. Fixed by `descend`, which walks each component down from the staging
  root, refuses any component that already exists as a symlink, and creates with `create_dir` rather
  than `create_dir_all` so nothing is made outside the tree even on the refusal path.
- **HIGH, `.previous` resurrection.** `retain_only` walks `<root>/<id>/`; the set-aside copy lives
  under `.staging/`. So a successful same-version re-install left it, and once a later install's
  retention had deleted `<id>/X/`, the next sweep saw an orphan whose target was absent and
  *restored* it — silently undoing D16(a). No test covered a same-version install that succeeds.
- **HIGH, a `tar -C dir -czf x.tgz .` tarball was refused outright** on its first `./` entry — the
  most common way a tarball is built.

The `htui` half's HIGH was mine, not the implementer's: blueprint B.12 said the consent pane
"swallows every key it is offered", assuming the global keymap ran first. `App::on_key` offers the
active tab the key **before** the `Tab` and `Global` scopes and returns on `Consumed`, so an open
pane killed `q`, `?`, `Tab` and the digit switches — the user could not quit. The pane is modal over
the table beneath it, not over the application.

## Known and accepted

- **The Windows lint gate cannot run on this box** and this item is what made that bite for
  `htui-agent`. See **TOOL-3**. Every Windows-conditional line was reviewed by eye instead, and the
  runtime facts were already MOD-16's.
- **Progress during unpack is per completed entry**, so a single-file archive reads as a phase word
  with no percentage until it finishes. The cell renders the phase alone whenever `done == 0`
  rather than a misleading `0%`.
- **A single very large archive entry cannot be interrupted mid-copy** — cancellation is checked
  between entries. Accepted by the blueprint.
- **Consent is not visible in `psql`** (D17): it is a box-local fact, and a re-imaged box is asked
  again, which is correct.
- `agy_acp_server` remains `unauthenticated` after installing until the vendor's own flow has run.
  Logging it in from the app is **MOD-21** (`R-AGT-9`).

## Live coordinates

- Install root: `HTUI_AGENTS_ROOT`, default `dirs::data_local_dir()/htui/agents`
  (`~/.local/share/htui/agents` here). `HTUI_TOOL_<NAME>` still overrides everything.
- Registry: `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json` — schema 1.0.0,
  40 agents, 19 distributing a `binary`, 10 of those publishing `sha256`. Mutable, `max-age=300`,
  no pinned snapshot URL.
- `antigravity-acp` 1.1.1: proprietary, terms `https://antigravity.google/terms`, **no digest**,
  682 MB archive, 1.9 GB unpacked. `amp-acp` 0.9.0: Apache-2.0, digest on every platform, 36 MB.
