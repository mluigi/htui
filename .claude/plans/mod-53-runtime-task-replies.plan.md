# MOD-53 - Runtime tasks always send a terminal reply (plan)

Base: main @ 34b9888. Branch: `claude/mod-53-runtime-replies-563cz3`. No migration. Nothing in MOD-40's footprint.

## Routing verdict

```
Item:      MOD-53 - Runtime tasks always send a terminal reply
Path:      plan
Criteria:  C1 ✗  C2 ✗  C3 ✓ (catch the panic vs notice it in sweep_finished)  C4 ✗   → 1 fired
Ultracode: not needed
Reasoning: one file plus a small public helper in htui-agent, and the item already names the fix.
```

## What the code shows (and one thing the item misses)

1. Every task the runtime spawns answers once at its request's address (`Frames`), and the UI flag
   that waits for it clears only on that answer (`agents.rs:1224` for `probe_agents`,
   `boxes.rs:500-505` for `ProbeBox`, the terminal Install/Auth/Chat frames). A panic inside the
   task means that answer never comes.
2. **The panic also tears the terminal down.** `terminal.rs:70-97` installs a process-wide hook that
   calls `ratatui::restore()` for every panic that is not "contained" (only
   `htui_agent::excerpt` marks its provider panics contained). Tokio catches a task panic, but only
   after the hook has run, so today a probe panic leaves the event loop drawing into a terminal with
   no alternate screen and no raw mode: the MOD-56 wedged-UI shape, not only a stuck flag.
3. `sweep_finished` only runs when the next request arrives, so fixing it there would leave the flag
   set until the user presses something else, and it cannot stop the hook from running.

## Design

- **Catch at the task, not in the sweep.** One wrapper in `agent_worker.rs` that every spawn goes
  through: it polls the task inside `catch_unwind`, and on a panic logs it (`tracing::error!`) and
  sends the task's terminal failure at the stream's current address. `sweep_finished` is unchanged:
  a wrapped task finishes normally, so its handle is swept as today.
- **Mark the poll contained.** Add a small public `contain(f)` next to `panic_is_contained` in
  `htui-agent/src/excerpt.rs` (the same thread-local, restoring the previous value). The wrapper
  polls inside it, so the hook leaves the terminal alone for a panic the runtime survives.
- **The terminal reply per task:**

  | Task | Reply on panic |
  |---|---|
  | `run_probe` (ProbeAgents) | `Failed { request: "probe_agents" }` |
  | `run_box_probe` (ProbeBox, registration) | `Failed { request: "probe_box" }` (shows the notice, clears `probing`) |
  | `run_preview` | `Failed { request: PROMPT_PREVIEW }` |
  | `run_plan`, `run_install` | `Install(InstallFrame::Failed { manual: None })` |
  | `run_auth` | `Auth(AuthFrame::Failed)` |
  | `run_chat` (both start paths) | `Chat(ChatFrame::Failed)` at the stream's moved address |
  | `run_reprobe` | nothing owed (it answers no request); containment only |

  The message is the panic payload when it is a string: "the <task> task panicked: <payload>".
- Abort is untouched: a superseded preview or a shutdown still drops the task with no reply.

## Out of scope (stated in the close-out)

- A chat that panics mid-turn leaves its run row open: closing it is recovery work (MOD-24's area).
- A panic on a thread a task starts (`spawn_blocking` unpack, provider threads) still reaches the hook
  uncontained; those already come back to the task as errors.

## Tasks (serial; one file set)

1. Tests first: wrapper unit tests (terminal reply at the moved address; contained during the poll;
   abort sends nothing); a runtime case with a panicking `HardwareSource` for `ProbeBox` (Failed
   arrives, claim is released so a second `ProbeBox` is accepted); a panicking-driver case for a chat
   and a probe where an existing fake allows it.
2. `contain` in `htui-agent/src/excerpt.rs`.
3. The wrapper and the seven spawn sites in `crates/htui/src/agent_worker.rs`.
4. Close-out: HANDOFF, `docs/decisions/mod/mod-53.md`, DECISIONS index, validator.

Files: `crates/htui/src/agent_worker.rs`, `crates/htui-agent/src/excerpt.rs`, tests in those crates,
`HANDOFF.md`, `docs/decisions/`. Not touched: `store_worker.rs`, `engine.rs`, the store write core.

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| No `panic = "abort"` profile, so unwinding is catchable | true | `Cargo.toml`, `crates/htui/Cargo.toml` |
| The hook restores the terminal on any uncontained panic | true | `crates/htui/src/terminal.rs:70-97` |
| Only excerpt providers mark panics contained | true | `grep catch_unwind crates` |
| `probing` clears on `Failed probe_agents` / `probe_box` | true | `agents.rs:1224`, `boxes.rs:500-505` |
| Chat tasks are built in `agent_worker.rs`, so no `store_worker.rs` edit | true | `agent_worker.rs:976`, `:1900` (`Served::Start`) |
| `ProbeBox` has a panic seam (`HardwareSource`) | true | `htui-agent/src/box_probe/hardware.rs:37`, `AgentRuntime.hardware` |
| No migration needed | true | no store change |
