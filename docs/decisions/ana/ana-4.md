# ANA-4 - Agent protocol: ACP client and CLI fallback (done, 2026-09-05)

## Summary

Concluded how `htui` drives a coding agent: the `AgentDriver` trait and its event model, the ACP
client, the CLI stream adapter, permission and edit-proposal handling, the `agent.launch` and
`agent.settings` JSONB shapes that ANA-9 §5.7 left open, autodiscovery probes and quota capture.
Design in [`docs/ANA-4.md`](../../ANA-4.md). Requirements addressed: `R-AGT-1..8`, `R-HIS-1..3`,
`R-SEC-2..3`, `R-TUI-6`, `R-TUI-8`.

Method: six parallel Opus readers (ACP spec, `agent-client-protocol` crate, `claude` transport,
`agy` transport, prior art, `htui` seams), 48 load-bearing claims adversarially re-checked against
primary sources (9 refuted and corrected, 23 left for the writer to re-verify or mark), one writer
agent producing the doc with local probes (`claude`, `agy`, `agy_acp_server`, `cargo info`, a rustc
dyn-compatibility probe). Everything the evidence could not settle is an explicit
"Unverified - MOD-2 must confirm" line, collected in `docs/ANA-4.md` §11.14.

## What was decided

1. **Trait and event model (§4.1).** Two dyn-compatible traits: `AgentDriver: Send + Sync` as the
   per-agent factory and `AgentSession: Send` per live session, with hand-written
   `Pin<Box<dyn Future + Send>>` returns and a pull-style `next_event()`. `async fn` in trait was
   rejected after a compile probe returned `E0038` on the installed toolchain. `DriverEvent` is the
   ANA-9 `EventKind` vocabulary minus the three `htui`-authored kinds (`prompt`, `follow_up`,
   `permission_answer`), so the conversion to `EventKind` is total. A recorder task owns `seq`,
   `turn`, `prompt_digest`, usage summing, scrub-then-persist and chunk coalescing (flush on kind
   change, message id change, `Done`, or 16 KiB; never on a timer, so recorded fixtures replay
   byte-identically).
2. **ACP client (§4.2).** Adopt the `agent-client-protocol` crate pinned `=2.1.0` (Apache-2.0,
   `rust-version = 1.88.0`, wire protocol 1, no `unstable_*` features, `ProtocolVersion::V1` sent
   explicitly). MOD-2 raises the workspace MSRV from 1.85 to 1.88; the declared 1.85 was already
   fiction given the locked `sqlx-core 0.9.0` floor. A schema-crate-only transport (~500 LOC over
   tokio) stays as Plan B with a named trigger. Rejected: hand-rolled JSON-RPC from scratch, floating
   `"2"` version (two majors in ten weeks).
3. **Bridge (§4.2).** One tokio task per session owns the connection; the session handle is only
   channels, so a future `!Send` transport needs no trait change. `htui` spawns the child itself
   (`process-wrap` job object plus `CREATE_NO_WINDOW`) and feeds the SDK byte streams, because the
   SDK's process-group kill is `#[cfg(unix)]` only. `SentRequest::block_task` is forbidden inside
   handlers.
4. **Permissions and edits (§4.3).** Three-stage pipeline: policy rules, then `remembered` answers
   persisted in `agent.settings`, then an inline ask in the chat tab; the first two answer with
   `by = "policy"`. `fs/write_text_file` is advertised and intercepted: `htui` synthesizes a unified
   diff (`similar`), persists `edit_proposal`, then applies. `ToolCallContent::Diff` is normalised
   into the same kind and deduped per `(tool_call_id, path)`.
5. **`claude` over ACP (§4.4).** Launch `node <claude-agent-acp>/dist/index.js` with
   `CLAUDE_CODE_EXECUTABLE` in the environment; a Rust spawn probe proved the bare shim name fails on
   Windows (`program not found`) while the `node <entry>` form works. `npx -y …@0.75.0` is the
   pinned fallback. Model selection via `session/set_config_option` keyed on `configId` (the
   `category` field is advisory only). The `claude -p --output-format stream-json` CLI path is a
   degraded mode inside the same registry row (`agent.settings.cli`), selected per box by the probe,
   not a second row.
6. **`agy` (§4.5).** Verdict `acp`. The `agy` binary has no headless run subcommand and no ACP
   surface; Google's registry-listed `agy_acp_server` (1.1.1) is already installed on this box and
   is the transport. Billing corrected to `subscription`. The CLI parse contract is documented as a
   contingency only (§6.3), not built in version one.
7. **Autodiscovery (§4.6).** Two tiers per agent: a cheap `--version` probe and a definitive ACP
   `initialize` handshake, driven by a `launch.discovery` recipe; results land in a new
   `agent_box.probe JSONB`. Windows shims are never spawned by name.
8. **Registry JSONB (§5).** `agent.launch = { command, args, env, discovery }` and
   `agent.settings = { permissions, remembered, cli, … }` fixed with one example and one seed row
   per agent; both seed rows ship `models: []` until the `session/new` model list is confirmed.
9. **Quota (§7).** `agent_box.quota` holds what each transport can report; `agy` has no
   machine-readable subscription quota on either transport, so its quota stays null and `R-AGT-8`
   treats null as available. `per_token` caps are enforced at the recorder by cancelling the session.
10. **Crate layout (§8).** New fourth workspace crate `htui-agent` (trait, event enum, both
    transports, recorder, probes); persisted types stay in `htui-core`. New workspace deps:
    `agent-client-protocol =2.1.0`, `tokio-util 0.7.19`, `process-wrap 10.0.0`, `similar 3.2.0`,
    `which 8.0.6`; tokio gains `process` and `io-util`. No fourth `select!` arm in
    `event_loop.rs`.
11. **Schema amendment (§9).** One forward-only migration, `0002_agent_probe.sql`:
    `ALTER TABLE agent_box ADD COLUMN probe JSONB` plus the stale `agent.name` comment fix.

## Residual gaps (design settled, evidence pending)

- `R-AGT-7`: no subscription quota source for `agy`; `R-AGT-3`: the CLI transport structurally
  cannot produce `edit_proposal`, `permission_request` or `plan`, and its permission interception
  waits on MOD-11's MCP permission-prompt tool.
- Every "Unverified - MOD-2 must confirm" item is listed in `docs/ANA-4.md` §11.14 (claude-agent-acp
  model config ids, cancel signal semantics, thinking-block shape, `agy_acp_server` usage and
  permission behaviour, its `.par` mechanics on darwin/linux, the `agy` status-line quota payload).

## Downstream items

- **MOD-2** (agent driver + chat tab) now blocked on ANA-5 only; build order in `docs/ANA-4.md` §9.
- **ANA-5** must provide the assembled prompt text, `sections[]` and a stable serialization order
  for `prompt_digest`; **ANA-7** must provide the fail-closed `Scrubber` and the per-project
  environment map.
- **MOD-4** reads `DriverCaps` and `run_step.usage`; **MOD-7** calls `probe.rs` from box
  registration; **MOD-10** supplies scrubber and environment; **MOD-11** supplies the MCP
  permission-prompt tool; **MOD-12** enforces the batch cap with §7's accounting.

## Commits

- docs(ana-4): conclude agent protocol analysis (this close-out).
