# MOD-55 - Ask an agent for help while editing a template or skill (done, 2026-10-06)

**Requirements:** `R-SKL-3`, `R-PRM-4`, bounded by `R-ID-5`, `R-ID-7`, `R-SEC-2`, `R-SEC-3`, `R-HIS-1`.
**Origin:** MOD-9's PRD gate, 2026-09-25 (`.claude/prds/mod-9-skill-library-templates.prd.md` D2,
`docs/decisions/mod/mod-9.md`).
**Artifacts:**
- PRD [`.claude/prds/mod-55-editor-agent-help.prd.md`](../../../.claude/prds/mod-55-editor-agent-help.prd.md):
  gate decisions D1-D4, open questions resolved, one milestone;
- plan [`.claude/plans/mod-55-editor-agent-help.plan.md`](../../../.claude/plans/mod-55-editor-agent-help.plan.md):
  P1-P11, the verified-claims table (two claims falsified and amended), the blueprint amendments as accepted;
- blueprint `.claude/plans/mod-55-editor-agent-help.blueprint.md`: signatures, instruction text, fence rule,
  the `AgentHelp` state machine, test lists, hazards H-1 to H-9.

Decision numbers are local to MOD-55 (the MOD-31 convention).

Routed as **PRD** (C2 and C3 fired, C4 undecided from the item text; low confidence at the threshold). No ultracode.
Run in a TOOL-7 sandbox (`hr/MOD-55`). T1 and T2 ran in parallel; T3, T4 and T5 ran in that order.

**Decisions (maintainer, 2026-10-06):**
- route accepted, PRD path;
- PRD gate: D1 a registry agent for one turn through the existing driver; D2 recorded as a chat-style run;
  D3 scrubbed before send, fail closed; D4 templates and skills in one action;
- PRD open questions: the project is the active one (refined at planning to the template's own project for
  templates), a fixed instruction in code, the whole body in one fenced block, cancellable with the buffer locked,
  unavailable offline;
- plan confirmed; the PRD constraint "the key is a named action in MOD-67's catalogue" amended, because the catalogue
  does not exist yet;
- blueprint amendments A-1 to A-11 accepted; M-1 accept/discard not recorded (self-report instead); M-2 all ten tool
  kinds denied; M-3 a cancel before acceptance is deferred and a cancelling help cannot be abandoned;
- review: H1, M1, M2, L2, L3 and the NITs fixed; L4 found closed by H1; L1 deferred to MOD-10;
- MOD-86 and MOD-87 filed for chat-path problems found on the way.

**Commits:**
- PRD, plan, blueprint, amendments: `80677348`, `59b25f77`, `8dda9c47`, `fe3fd20f`, `aac4f07e`;
- T1 `prompt::edit_help`, `scrub_section`: `0723ddef`;
- T2 `ChatRunSpec.phase_name`: `a39893a0`, `519c2fa4`;
- T3 `StoreRequest::EditHelp`, help mode: `5ea63b55`;
- T4 `AgentHelp`, Templates editor: `92393ee0`;
- T5 Library editor: `00315ba4`;
- review fixes: `7c73318e`, `65da4fc9` (L2, L3), `ea763082` (NIT), `39d7f857`, `b1b4b532` (M1), `cc9a38dd`,
  `4859693f` (H1), `7cb7b616` (M2).

---

## What was built

`Ctrl+G` in the Skills tab's Templates and Library editors opens a panel in the draft area. The maintainer picks an
enabled agent (the first one by default), types a request and presses `Enter`. The body, the request, the template's
name and (for templates only) its role's placeholder table go to the agent as one **help turn**. The reply's last
closed fenced block is shown as a diff against the body that was sent. `Enter`/`y` puts it in the buffer and
`Esc`/`n` discards it. Saving is still `Ctrl+S` through the existing gate: `template::parse` for templates, and for
skills the blank/NUL check plus the unchanged-from-head check. While the panel is open the buffer is locked, and
`Ctrl+S`, `Ctrl+E` and `Ctrl+G` only say so.

### Core (`htui-core`)
- `prompt::edit_help`: `HelpTarget::{Template { name }, Skill { name }}`, `HelpPrompt` (its `Debug` prints lengths
  only), `HelpPrompt::scrubbed`, the fixed `INSTRUCTION`, `assemble`, `sections`, `placeholder_table`, `proposal`,
  `closed_blocks` and `holds_mask`. The body goes in a fence one backtick longer than any run inside it.
- `prompt::scrub_section` and `SectionRefused { section, rule }`, which never carries the text. `scrub::REDACTED` is
  now public.
- `ChatRunSpec.phase_name`: the serde default is `"chat"`, and `for_edit_help()` sets `"edit_help"`. Both stores bind
  it. The Postgres insert's `.sqlx` entry was regenerated (`query-a9a4ffc8...` replaced by `query-8fb05895...`). No
  migration: `run_step.phase_name` has no CHECK, and `run.kind` stays `'chat'`.

### Runtime (`htui`, `htui-agent`)
`StoreRequest::EditHelp { project_id, agent_id, prompt }` runs through `AgentRuntime::start` and `run_chat` in
`ChatMode::Help`. Help mode differs from a chat in these ways:
- **Scrub first, fail closed.** The offline check comes first. The prompt is then scrubbed before the box, registry
  row, lease or run are touched. A refusal is `Failed { "edit_help", "not sent: the <section> matches the <rule> rule" }`,
  with no run and no driver start.
- **No tools and no permissions.**
  - There is no MCP lease, and the policy is `PermissionPolicy { default: Deny }`.
  - `deny_kinds` holds every `ToolKind`, and `ToolExposure::no_tools` is set.
  - On `claude-cli`, `no_tools` becomes `--tools=` and `--strict-mcp-config`, with no `--mcp-config`. The row's
    `--permission-mode` is dropped, and extra args that would widen the session refuse the start before the spawn.
  - On ACP, the agent's own tools can't be removed by the protocol, so the denied kinds and the empty MCP list are
    what binds.
- **One turn.** After the first `Done` the session is cancelled and the run closed `done`. A recorder refusal sends
  `Failed` before `Ended`, and only `Ended { EndTurn }` makes a proposal.
- **Cancellable at any point.** The help turn reads commands while it pulls events, so `Esc` stops a streaming turn:
  the run is closed `cancelled` and `Ended { Cancelled }` is sent. Every queued command is answered exactly once.

### Measuring the PRD hypothesis
Help runs can be counted as `run_step.phase_name = 'edit_help'`. Whether proposals were accepted or discarded is not
recorded (M-1); the hypothesis is to be checked by the maintainer's own tally over the next 10 uses.

## Known limits and carried items

- **H-6 (accepted, then narrowed by M1).** On ACP the agent's own tools that never ask permission stay reachable.
  The deny-all kinds stop every `fs/*` request and every request that asks.
- **L1, deferred to MOD-10.** `holds_mask` compares counts of `[REDACTED]` in the sent body and in the proposal. Once
  MOD-10 masks real values, a body with a literal `[REDACTED]` could hide a masked value from the double-accept. The
  fix is for the runtime to report how many masks it applied. The note is on MOD-10's entry.
- **Key binding.** `Ctrl+G` is hard-coded like `Ctrl+E`/`Ctrl+S`. MOD-67's milestone 4 (Skills) moves it into the
  catalogue; the note is on MOD-67's entry.
- **Chat path, filed separately:**
  - **MOD-86:** the Chat tab sends prompts unscrubbed (scrubbed only before persisting), against `R-ID-7`.
  - **MOD-87:** a chat cancel during an unparked turn is answered only after the turn, and a second queued cancel is
    dropped unanswered.
- The schema comment `0001_init.sql:478` ("'chat' for run.kind = 'chat'") is in a shipped migration and was not
  edited. `edit_help` is the other value.

## Verification

Gates were run on the final tree, single-threaded:
- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets --all-features -D warnings`, and featureless clippy;
- `SQLX_OFFLINE=true cargo check --workspace --all-features`;
- `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1`: **4743 passed, 0 failed**.

An earlier run before the review fixes had one `qdrant_worker` "create collection" timeout, the known load flake; it
passed on the final run. Snapshots:
- changed (hint row only): `templates__changed_elsewhere`, `templates__edit_help`, `templates__missing_item_confirm`,
  `templates__unknown_placeholder_cursor`, `skills__edit`, `skills__changed_elsewhere`;
- new: `templates__agent_help_asking`, `templates__agent_help_proposal`, `skills__agent_help_proposal`.

The Postgres gate on the merged tree runs on the host after `scripts/hr collect MOD-55`.
