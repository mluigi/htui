# MOD-21 — In-app agent authentication

> Routed as **PRD** by `/handoff-run MOD-21` (criteria C2, C3, C4 fired). Ultracode recommended for
> the implement phase. Like MOD-20, no ANA precedes this item — `docs/ANA-4.md` §4.5 *concluded the
> opposite* ("`htui` cannot log in non-interactively") and `R-AGT-9` reverses it by maintainer
> decision on 2026-09-09, so this document carries the evidence a design would otherwise cite and
> the plan settles the mechanism. `R-AGT-9` is the contract.

## Problem

MOD-2 milestone 6 made `unauthenticated` visible and honest: the probe records it, the credential
tier decides it (D59), and the agent's own refusal now reaches the user instead of being swallowed.
Visible is not actionable. Today the row reads `unauthenticated`, the Chat tab refuses the session,
and the entire cure is a vendor login `htui` neither performs nor explains — for `agy_acp_server`,
in a directory (`$GEMINI_HOME/antigravity-acp/`) that the `agy` CLI's own login does not even write.
`MOD-2`'s last task, T34, is blocked on exactly this: not on code, on a human leaving the app.

The gap is the mirror image of the one MOD-20 just closed. `Settings > i` turns `missing` into an
install; nothing turns `unauthenticated` into a session. Both are the same user sentence — "make
this box able to run this agent" — and after MOD-20 only half of it is answered.

## Evidence

Every fact below was verified live against `agy_acp_server` 1.1.1 as installed on this box, or read
out of the vendored schema and this tree, during PRD research on 2026-09-09. Nothing here is
recalled.

- **The call exists, `htui` speaks it, and it is trivially shaped.** `authenticate`
  (`agent-client-protocol-schema-1.7.0/src/v1/agent.rs:295`) takes one field, `methodId`, and
  `AuthenticateResponse` (`:341`) carries **nothing but `_meta`**. Success is "the call returned";
  there is no token, no status and no URL in the reply. `logout` (`:385`) is the same shape and is
  gated by `agentCapabilities.auth.logout` (`:464`).
- **The methods are not opaque ids — the wire already carries their prose.** A live `initialize`
  against the installed server returns four methods, each with `id`, `name` and `description`:
  `oauth-personal` "Log in with Google", `oauth-business` "Log in with Gemini Enterprise",
  `gemini-api-key` "Gemini API key", `agent-platform` "Gemini Enterprise Agent Platform … with
  Application Default Credentials or an API key". **The probe throws all of that away**:
  `Handshake::from_response` keeps `authMethods[].id` only (`acp/handshake.rs:70-74`), so the
  snapshot cannot feed a chooser a human can read.
- **Two method *kinds* exist and only one is a protocol call.** `AuthMethod` is a `type`-tagged
  enum (`agent.rs:575`): `terminal` means *the client re-runs the agent's own invocation in an
  interactive terminal* and the spec states "The client MUST NOT pass this method to
  `authenticate`"; anything untagged is `agent`, handled by the call. Terminal entries are opt-in —
  an agent may only offer them when the client advertised `clientCapabilities.auth.terminal`
  (`client.rs:2002`, `AuthCapabilities`). **All four of `agy`'s methods are untagged**, so on the
  agents this box has, `authenticate` is the whole story.
- **`authenticate` blocks for the human round trip.** Sent live, it produced **no response in
  45 s** and was still pending when the adapter was killed. There is no timeout, no progress
  notification and no cancellation verb in v1 — the call simply does not return until the user has
  finished or the process dies.
- **The URL arrives on stderr, as prose, and not through the protocol.** The live run advertised
  `elicitation: { url: {}, form: {} }` — the schema's own answer for this, whose
  `ElicitationScope::Request` is documented verbatim as "tied to a specific JSON-RPC request outside
  of a session (e.g., during auth/configuration phases before any session is started)"
  (`elicitation.rs:1489-1496`). **`agy` sent no `elicitation/create` anyway.** What it wrote was one
  stderr line: `Open the following link to authenticate the ACP server: https://accounts.google.com/o/oauth2/v2/auth?…`
- **The redirect is a loopback listener inside the adapter process.** The URL carries
  `redirect_uri=http://127.0.0.1:39879/` (a different ephemeral port on each run: 39879, then
  50651). The flow is therefore bound to that child's lifetime — kill the adapter and the URL is
  dead, whatever the browser is showing.
- **The adapter spawns a browser itself, and that browser writes into the ACP stdout channel.**
  With `$BROWSER` unset and no `DISPLAY`, the second live run produced alt-screen escape sequences
  and a terminal browser's progress text **on the adapter's stdout** — the JSON-RPC stream:
  `(B0[?1049h[1;24r… Acquisizione di https://accounts.google.com/o/oauth2/v2/auth?response_type=code`.
  The first run, with `BROWSER=/bin/true`, was clean. So the hijack is real, it is controllable by
  one environment variable, and for a full-screen TUI it is the sharpest constraint in this
  document: an uncontrolled `authenticate` both corrupts the protocol stream and fights `htui` for
  the terminal.
- **The API-key methods refuse cleanly and name their variable.** `authenticate` with
  `gemini-api-key` and no key present answers, in ~0 s:
  `{"code":-32602,"message":"The GEMINI_API_KEY environment variable must be set in the environment
  the ACP server is launched from for Gemini API Key authentication."}` — "launched from", i.e. the
  variable must be in the spawn environment, which is MOD-10's seam and not a browser flow at all.
- **The re-probe path already exists and is already declared.** `agent_agy.json`'s
  `discovery.credential` names `GEMINI_API_KEY` and
  `%GEMINI_HOME%/antigravity-acp/acp_token.json`, and `probe::status_for` (`probe.rs:1197`) maps a
  non-empty `authMethods` plus a present credential to `Ready`. A successful login therefore needs
  **no new status vocabulary** — only a re-probe. Whether the vendor writes that exact filename is
  unverified: no box here has ever completed the flow.
- **`htui` advertises neither capability the auth story touches, and the rows already carry both
  flags.** `client_capabilities()` builds `fs` plus a hard-coded `.terminal(false)`
  (`acp/client.rs:32-38`); `elicitation` is never set at all, though `ClientCapabilities`
  (`launch.rs`) and both seed rows carry `terminal` and `elicitation` fields.
- **The stderr channel is already plumbed.** `Spawned::stderr_tail` keeps the last
  `STDERR_TAIL_LINES = 64` lines (`launch.rs:31`, `:558-568`) and both `handshake_error` paths
  already surface it to the user. Reading a URL out of it is a use of existing machinery, not new
  machinery.
- **MOD-20 built the whole delivery shape one item ago.** `StoreRequest::{InstallPlan,
  InstallConfirm, InstallCancel}` served as `Served::Deferred` off an owned `Writer`
  (`agent_worker.rs:439-455, 562-662`), a `LiveInstall { cancel, task, phase }` the runtime holds
  one of, an `InstallFrame` stream answering one request with many replies, and a consent pane plus
  hint line inside `settings/agents.rs` (`i`, `y`/`n`, `x`). This item is that machine with a
  different payload.
- **`claude` needs none of this and must stay unaffected**: its adapter returns an empty
  `authMethods`, which `status_for` maps to `Ready` whatever the credential says. The subjects are
  `agy` (four methods) and MOD-20's live proof `amp-acp` (`authMethods: ["setup"]`).

## Users

- **Primary**: the maintainer on a box where an agent is installed and refuses to work. Today they
  read `unauthenticated` in Settings, leave `htui`, discover that the vendor CLI's login is the
  wrong login, and hunt for a directory. The need fires the first time an install succeeds — which,
  after MOD-20, is a keystroke away.
- **Also served**: MOD-2, whose milestone-6 task T34 and three `docs/ANA-4.md` §11.14 questions
  (does `agy_acp_server` emit `usage_update`, does it issue `session/request_permission` in
  `default` mode, what shape are its edits) are blocked on a logged-in server by *any* means.
- **Also served**: anyone adding a third agent under `R-AGT-5` — an agent that advertises auth
  methods is logged in through the same code path, because nothing here keys on an agent name.
- **Not for**: agents that advertise no auth methods (`claude`), which are already `ready`. Not a
  credential manager: `htui` never sees the token. Not a replacement for `HTUI_TOOL_<NAME>` or for a
  vendor CLI a user prefers.

## Hypothesis

We believe **triggering the agent's own `authenticate` from Settings, surfacing what the agent says
on its stderr, and letting the probe decide the outcome**, will **turn `unauthenticated` from a
report into an action** for **the maintainer and every box after the first**.

We'll know we're right when **`agy` on this box goes from `unauthenticated` to `ready` without the
maintainer leaving `htui`, and MOD-2's T34 chat runs against the server that flow logged in** —
with no agent name, method id or vendor URL anywhere outside seed JSON and test fixtures.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Vendor-specific code paths | 0 | No agent name, auth-method id or vendor URL outside seed JSON and fixtures; `R-AGT-5`, `R-AGT-9` |
| The credential is never touched | `htui` never reads, holds, logs or persists a token; the flow's only artifacts are a method id and an outcome | `R-AGT-9`, `R-ID-7`, `R-SEC-2`; a test asserting no auth frame or record carries anything but ids and status |
| Probe is the authority | Post-auth status comes from a re-probe, never from the flow's own claim | `R-AGT-6`; a test where `authenticate` returns success into a box with no credential and the row still reads `unauthenticated` |
| The protocol stream survives the browser | An adapter that spawns a browser cannot write to the ACP channel or to `htui`'s terminal | The live hijack above, reproduced as a regression test with a fake "browser" the flow must neutralize |
| UI never blocks | A login pending for minutes leaves the TUI responsive and every other request answered | `R-NF-3`; the `Served::Deferred` pattern of `agent_worker.rs:562-662` |
| Capability gate is structural | A transport with no `authenticate` refuses rather than pretends, and a row with no auth methods offers no action | `R-AGT-1`; `DriverCaps` test over both transports |
| Abandoned flow is inert | Cancel, adapter death and app shutdown each leave the row exactly as found and no surviving process | `R-NF-3`, MOD-2's orphan class; a test asserting no child survives a cancelled flow |
| Live proof | `agy` reaches `ready` on this box through the app, and T34's chat runs | `crates/htui-agent/tests/*_live.rs`, mirroring `agy_live.rs` |
| Nothing writes `agent` | The registry row is byte-identical before and after a login | `R-AGT-9`; a store test comparing the row |

## Scope

**MVP** — a capability-gated `authenticate` on the driver seam, a chooser fed by the agent's own
method list, the agent's stderr surfaced as the hand-off, a cancel that kills the child, a re-probe
that decides the outcome, and logout where the agent advertises it.

Concretely in scope:

- **An `authenticate` operation on the driver seam** (`R-AGT-1`), gated by a new `DriverCaps`
  predicate so the CLI transport refuses rather than pretends. It is not a session operation: it
  spawns, `initialize`s, sends `authenticate`, and dies — the `probe`/`handshake.rs` lifetime shape
  (`ChildGuard` in the caller's frame), not `open_session`'s.
- **A method chooser built from the flow's own `initialize`**, using each method's `name` and
  `description`. The probe's snapshot still decides *whether* the action is offered; the live
  response decides *what it offers*, so no `agent_box.probe` shape change is needed (D7).
- **The hand-off**: the agent's stderr lines rendered in the app as they arrive, with any URL
  detected and offered for opening on an explicit keystroke. `htui` opens nothing on its own.
- **Neutralizing the adapter's own browser launch** on the auth spawn, so a child browser can
  neither corrupt the JSON-RPC stream nor seize the terminal (D2).
- **A Settings action beside `r` and `i`** (`R-TUI-8`), served exactly as MOD-20's install is:
  `Served::Deferred`, an owned `Writer`, one live flow at a time, a frame stream, never the
  worker's `select!` arm and never the UI task (`R-NF-3`).
- **Cancellation instead of a timeout** (D3): human-paced, cancellable from the pane, with a long
  idle cap that kills the child rather than leaking it.
- **A re-probe on success**, through the probe's own path, so `agent_box` is written by the probe
  (`R-AGT-6`) and `unauthenticated` becomes `ready` by the same rule that decided it.
- **Logout** where `agentCapabilities.auth.logout` is advertised, with the same re-probe afterwards.
- **API-key methods offered but not injected** (D5): the flow attempts them and surfaces the
  agent's own refusal naming the variable; the injection seam is MOD-10's.
- **Documentation**: the README's `agy` section gains the in-app flow, and MOD-2's T34 note in
  `HANDOFF.md` is answered rather than restated.

**Out of scope**

- **Storing, reading or transmitting any credential.** `R-AGT-9` and MOD-2's D59 rule stand: the
  probe checks existence and records a tier, never a value.
- **Terminal-type auth methods** (D4) — no agent this box can reach offers one, and supporting them
  means suspending the TUI onto the real tty. `clientCapabilities.auth.terminal` stays `false`, so
  a conforming agent may not offer one.
- **Secret injection for API-key methods** — MOD-10 (`R-SEC-1..4`). This item calls that seam when
  it exists and refuses informatively until then.
- **Windows runtime facts** — MOD-16, which already owns MOD-2's and MOD-20's. Opening a URL on
  Windows (`ShellExecute`) is written and reviewed here, verified there.
- **Authentication as a box-registration step** — MOD-7 is the natural second caller, as it is for
  MOD-20's installer; the MVP is Settings only (D8).
- **Auto-authentication.** Nothing triggers a login on startup, on `PROBE_TTL` expiry, or on a
  failed chat spawn. The user asks.
- **A box with no server**, refused on the same terms as `ProbeAgents` and the installer
  (`REGISTRY_ON_SERVER_ONLY`).

## Constraints (fixed before planning)

- **`R-AGT-5` is structural.** No agent name, method id or vendor string outside seed JSON and
  fixtures.
- **Nothing here writes `agent`.** Authentication is a fact about a *box*; it reaches `agent_box`
  through the probe and nowhere else.
- **`htui` never sees the credential** (`R-AGT-9`, `R-SEC-2`, `R-ID-7`).
- **`R-NF-3` is enforced by ownership**: no store handle on the render side, no long await in the
  worker's `select!` arm, and a flow that may pend for minutes lives in its own task.
- **The child must never outlive its flow.** MOD-2 spent two review gates on this class
  (`run_bounded`, `ChildGuard`, D61); the auth flow's child is longer-lived than any of them and
  gets the same ownership discipline.
- **The overlay factory signature is `Fn() -> Box<dyn Overlay>`** with no constructor argument, so
  any pane receives its text through `wants_requests` / `on_reply`, as MOD-20's consent pane does.
- **`unsafe_code = "forbid"`, MSRV 1.98, workspace lint set unchanged; TDD per repo convention.**
- **No new migration** unless a decision forces one; MOD-4's `0003_orchestration.sql` is still the
  next migration and must not be raced (`docs/ANA-2.md` §9).

## Delivery Milestones

<!-- Business outcomes, not engineering tasks. /plan turns each into a plan. -->
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | The call, capability-gated | `htui` can ask an agent, off any UI, "which ways can I log you in, and start this one" — spawning, initializing, sending `authenticate`, and killing the child on every exit path. A transport without the call refuses. | complete | [plan](../plans/mod-21-in-app-agent-auth.plan.md) |
| 2 | The hand-off a TUI can survive | What the agent says during the flow reaches the user as it happens, the URL is offered for opening on an explicit key, and the agent's own browser cannot touch the protocol stream or the terminal. Cancel kills the child; nothing leaks. | complete | [plan](../plans/mod-21-in-app-agent-auth.plan.md) |
| 3 | The action in the app | The maintainer logs an agent in from Settings: choose a method, watch the flow, cancel it, and see the row change — decided by a re-probe, with the TUI responsive throughout. Logout where advertised. | complete | [plan](../plans/mod-21-in-app-agent-auth.plan.md) |
| 4 | Proven on a real login | `agy` on this box goes `unauthenticated` → `ready` through the app, the declared credential path is confirmed or corrected, MOD-2's T34 chat runs, and the README says what to do instead of what to install. | complete¹ | [plan](../plans/mod-21-in-app-agent-auth.plan.md) |

¹ **Milestone 4's T34 clause is met as far as this item can meet it, and no further.** The login
landed on 2026-09-10, the seed's credential path is confirmed, and `agy_live.rs` passes 4/4 against
the now-authenticated server — including `session_new_reports_its_config_options`, which was
unreachable while unauthenticated. But **no live chat *turn* was driven**: none of `agy_live.rs`'s
four cases sends a prompt, so ANA-4 §11.14's three questions (does `agy_acp_server` emit
`usage_update` and in what field; does it issue `session/request_permission` in `default` mode and
with what option ids; do its edits arrive as a standard `tool_call` + `diff`) remain open. Writing
that turn is **MOD-2's own T34 work**, which this item unblocked rather than performed.

Milestones 1 and 2 are testable without a UI and without a real account (a fixture agent that
advertises methods, blocks, and prints a URL). Milestone 3 is the first that needs the store worker.
Milestone 4 is the one that closes `R-AGT-9` and unblocks MOD-2's last task.

## Decisions taken at the PRD gate

Answered by the maintainer on 2026-09-09, before planning, each resting on a fact under Evidence.
D2, D3, D5 and D7 reversed or replaced the positions held before the live runs. They are decisions,
not proposals — the plan implements them and records deviations as it would from any other
constraint.

- **D1 — The flow is a driver-seam operation over the probe's process shape.** `R-AGT-1` and the
  item text both say the seam; the *lifetime* is `handshake.rs`'s, not `open_session`'s, because
  the flow has no session, no prompt and no cwd to invent. One spawn serves both the method list
  and the call, which is what makes D7 possible.
- **D2 — `htui` neutralizes the adapter's browser launch and opens URLs itself, on an explicit
  key.** The live hijack makes an uncontrolled spawn a protocol-corruption bug, not a preference.
  The flow sets the launch environment so the agent's own opener is a no-op, renders the stderr
  line, and opens it only when the user presses the key — `xdg-open`/`open` here, `ShellExecute` on
  Windows (verification MOD-16's).
- **D3 — Cancellation, not a timeout.** `HANDSHAKE_TIMEOUT` is meaningless against a human OAuth
  round trip. The pane cancels, shutdown cancels, and a long idle cap (order of ten minutes)
  kills a flow nobody is watching rather than leaking a child with an open listener.
- **D4 — Terminal-type methods are refused for the MVP.** `clientCapabilities.auth.terminal` stays
  `false`, which makes it a spec violation for an agent to offer one; if one appears anyway, the
  chooser hides it and says why. Supporting them means suspending the TUI onto the real tty, and no
  reachable agent needs it.
- **D5 — API-key methods are offered, attempted, and refused in the agent's own words.** The live
  `-32602` message names the variable precisely; relaying it is more useful than hiding the method,
  and injecting the variable is MOD-10's seam. The flow does read a variable already present in the
  spawn environment — it does not grow a second environment mechanism.
- **D6 — The outcome is the probe's, and a failure changes nothing.** Success re-probes; a failed,
  cancelled or abandoned flow leaves `agent_box` exactly as found, including its `probed_at`.
- **D7 — The probe snapshot keeps ids only; the chooser reads the live response.** The flow must
  spawn anyway, so the human-readable `name`/`description` come from that `initialize`. This avoids
  a shape change to `agent_box.probe` and the compatibility rule that would follow it. Cost: the
  action's *availability* is decided by a snapshot up to `PROBE_TTL` old, which is already how
  `Settings > i` behaves.
- **D8 — Settings only.** As MOD-20's D7: the action lives in the Settings agent section for the
  MVP. MOD-7's box registration is the intended second caller and a later item; the chat-spawn
  failure path (D60) is not extended to offer a login.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The adapter's child browser corrupts the JSON-RPC stream or seizes the terminal | High (observed live) | High | D2: neutralize the opener on the auth spawn; regression test with a fake opener that would write to stdout |
| `authenticate` never returns and the child leaks with an open loopback listener | High (it blocks by design) | High | D3's cancel plus idle cap, `ChildGuard` ownership, and the shutdown path MOD-2 already exercises |
| The declared credential file (`%GEMINI_HOME%/antigravity-acp/acp_token.json`) is not what the vendor writes, so a successful login still reads `unauthenticated` | Medium | Medium | Milestone 4 is a real login; the seed is corrected from what the flow actually leaves on disk, which is the only way to learn it |
| A second agent surfaces the URL somewhere other than stderr (or uses `elicitation/create`, which the schema intends) | Medium | Medium | Surface the stream, do not parse a vendor's sentence; keep URL detection generic and treat elicitation as a later, additive capability |
| The user completes the browser flow after `htui` cancelled, believing it worked | Medium | Low | The re-probe is the only thing that changes the row, and the pane says the flow was cancelled |
| Advertising `auth.terminal` later changes what agents offer | Low | Medium | D4 keeps it `false` and the chooser hides what it cannot serve |
| Windows: opening a URL, and the browser-neutralizing variable, behave differently | High | Medium | Written and reviewed here; **TOOL-3 means it may not even be lint-checkable from Linux**, so MOD-16 inherits the runtime half, as with MOD-20 |
| The flow races the probe or an install for the same row | Medium | Medium | The runtime's existing one-at-a-time claim (MOD-20 hazard H-10, MOD-2 D60's claim) covers all three, extended rather than duplicated |

---
*Status: COMPLETE — all four milestones landed 2026-09-10; write-up at `docs/decisions/mod/mod-21.md`.*
