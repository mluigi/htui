# Blueprint: MOD-55 milestone 1, agent help in the Skills editors (T1 core prompt, T2 phase name, T3 runtime, T4 component + Templates, T5 Library, T6 docs)

**Plan**: `.claude/plans/mod-55-editor-agent-help.plan.md` (confirmed 2026-10-06; P1-P11 approved). **PRD**:
`.claude/prds/mod-55-editor-agent-help.prd.md`. Produced by `code-architect`. Wave 1 is T1 ∥ T2, then T3, T4, T5 and T6
in sequence, as the plan says. Line numbers are as of `8dda9c47` and are anchors, not contracts: insert next to the
named symbol.

## Plan amendments needed (session to accept or reject before T1)

- **A-1 (P8, defect): frames cannot be matched by `step_id` or request `seq` in a view.** `Tab::on_reply(&reply, ctx)`
  gets no `seq`, and `ChatFrame::Event`, `Ended` and `Failed` carry no `step_id` (only `ChatAccepted` does). What
  keeps frames apart is `App::is_fresh` (newest request per `(origin, kind)`) plus one rule: only one help can be live
  per Skills tab, because the editor that owns it captures input and the `h/l` view switch is off while an editor is
  open. §5.3 has the per-state accept table that makes a stray frame harmless. Rejected: the `Tab` trait would need
  the `seq`, which is a shell change.
- **A-2 (P8, defect): `Esc` before `ChatAccepted` has no `step_id` to cancel.** The component goes to
  `Starting { cancel: true }` and sends the `ChatCancel` when the acceptance arrives. Nothing can "abandon" a help that
  is cancelling: the buffer stays locked until the stream's terminal frame. That frame always comes (run end, start
  failure, and the panic answer all send one), and refusing to abandon is what keeps a late `Ended` from landing on the
  next help (§5.3, H-4).
- **A-3 (P5, defect): `#[serde(default)]` on a `String` gives `""`, not `"chat"`.** Use
  `#[serde(default = "chat_phase")]` with a private `fn chat_phase() -> String`.
- **A-4 (P6, refinement): `HelpTarget::Template { name: String }` and `HelpTarget::Skill { name: String }`, not
  `Template(TemplateRole)`.** The role is `TemplateRole::of_name(name)`, so carrying the name loses nothing. It lets the
  instruction say which template (`implement`, a phase template) and which skill. Names are scrubbed as section `name`.
- **A-5 (P8, naming): the module is `ui/tabs/skills/agent_help.rs` and the type is `AgentHelp`.** In `templates.rs`,
  "help" already means the placeholder column (`HELP_WIDTH`, "The help column's width"), and
  `templates__edit_help.snap` is a snapshot of that column. New snapshots are named `*__agent_help_*`.
- **A-6 (P3, stricter): deny all ten kinds, `deny_kinds: ToolKind::ALL.to_vec()`, not only write/delete/move/execute.**
  A help turn needs no tool. `read`/`search` would read under the process cwd (the htui checkout or `$HOME`) into a
  transcript that goes to the provider, and `fetch` is egress. `switch_mode` could move an agent into a bypass mode.
  On `claude-cli` this gives `--disallowedTools=Read,NotebookRead,Edit,Write,MultiEdit,NotebookEdit,Bash,BashOutput,KillShell,Glob,Grep,WebFetch,WebSearch`
  (`cli/claude.rs` `tool_names`; delete, move, think, switch_mode and other name no tool). Rejected: use
  `[Edit, Delete, Move, Execute, Fetch, SwitchMode]` and change test T3-c.
- **A-7 (P4, defect): a recorder scrub refusal drops the chunk's frame silently.** `Recorder::refuse` writes a
  `scrub_residue` row and "tells the UI nothing" (`htui-agent/src/record.rs` ~1684). A proposal assembled from frames
  would then be missing text and could be accepted. In help mode, `run_chat` sends `ChatFrame::Failed` when
  `recorder.finish()` fails, before `Ended`. The component offers a proposal only on `Ended { EndTurn }` with no
  `Failed` before it.
- **A-8 (files, omission): `crates/htui/src/testkit.rs` must change in T3.** `Harness::drive` routes the agent
  runtime's requests through an or-pattern (`testkit.rs:281-296`). Without `EditHelp` in that pattern, every
  integration test in T4/T5 gets "no agent runtime in this harness". Also `crates/htui/tests/templates.rs:399`
  asserts the old hint text (T4).
- **A-9 (P2, shape): a scrub refusal is answered `Served::Reply(StoreReply::Failed { request: "edit_help", .. })` from
  inside `start`, not as a `StoreError`.** Every `StoreError` `Display` has a prefix ("constraint violated: ...") that
  would misdescribe it. The message is `not sent: the body matches the github_token rule`. It names the section
  (`name`, `body`, `request`) and the rule, never the text.
- **A-10 (T2 conformance): extend `start_chat_run_mints_chat_rows`, do not add a case.** A new case moves two count pins
  the plan does not list (`htui-core/tests/mem_store.rs:36` `148`, `htui-store/tests/pg_conformance.rs`
  `EXPECTED_CASES = 148` plus its prose). Rejected: add `start_chat_run_binds_its_phase_name` and bump both to 149.
- **A-11 (P9, location): `htui_core::scrub::REDACTED` becomes `pub`.** The marker is defined once, today as a private
  `const`. `edit_help::holds_mask` reads it.
- **B-1 (decision): agents are read with `StoreRequest::Agents` when `Ctrl+G` opens the help, not in
  `wants_requests`.** That leaves the Skills tab's activation requests (and the tests that count them) unchanged, and
  the list is fresh each time. The default is the **first enabled agent in name order**, the Chat tab's rule
  (`chat/mod.rs:541-548`). "An agent with a default model" was considered and rejected: D1 already falls back to the
  agent's own default.
- **B-2 (decision): the help draws inside the draft area, not in an overlay.** For templates that is the left pane;
  the placeholder column stays visible, which is what a reader checks a proposal against. For the library it is the
  whole content. Asking, Starting, Streaming and Cancelling split the area: a locked draft on top, a 4-row panel below.
  Proposal and Answered replace the draft with the diff or the reply text. No `Clear`, no `ui::layout::centered`. No
  existing snapshot shows a help, so only the hint row moves (§5.6).
- **B-3 (decision): the staleness re-probe and the D60 spawn re-probe stay on for help starts.** A help start uses the
  row's launch like any chat. The re-probe refreshes the row for the next start and shares nothing with this session.
  A branch to skip it buys nothing. (The fake rows in tests are `cli`, so no re-probe runs there.)
- **B-4 (decision): the project cap and the quota latch apply to help turns, unchanged.** A help turn is spend. A cap
  breach ends the turn `TurnEnd::CapExceeded`; by D73 no `frames.failed` goes with it, and `Ended { Cancelled }` follows.
  The component proposes nothing on any stop reason but `end_turn`.

## Maintainer decisions needed

- **M-1 (PRD metric, not covered by the plan): accept vs discard is not recorded anywhere.** PRD D2 says accept/discard
  is "countable for the success metrics", but the help run is closed before the user decides, and no plan task writes
  the outcome. Options: (a) defer. Help runs are countable by `run_step.phase_name = 'edit_help'`, and the accept rate
  comes from the maintainer's self-report (recommended for M1; no new write path). (b) Add a write: a new
  `StoreRequest` appending an `htui`-role `other` row `edit_help_outcome {accepted|discarded}` to the closed step. That
  means a store method, both stores, a conformance case and a `.sqlx` entry.
- **M-2: A-6's all-kinds denial** (stricter than P3).
- **M-3: no abandon while cancelling (A-2).** The worst case is the handshake timeout: 60 s for ACP, the CLI first-byte
  bound for `cli`.

---

## 1. T1 `htui-core`: help prompt, reply parser, string scrub

Files: `crates/htui-core/src/prompt/edit_help.rs` (new), `crates/htui-core/src/prompt/mod.rs`,
`crates/htui-core/src/scrub.rs` (A-11 only).

### 1.1 `prompt/mod.rs`

- `pub mod edit_help;` between `pub mod digest;` and `pub mod estimate;`. Add one sentence to the module doc's
  milestone paragraph: "MOD-55 adds [`edit_help`], the fixed prompt and reply parser of the editors' agent help, and
  [`scrub_section`]."
- Directly after `scrub_text` (~1276-1294), and `scrub_text` re-expressed over the same private core so there is one
  wrap/scrub/unwrap:

```rust
/// One string, wrapped, scrubbed and unwrapped (plan D100's two lines); the scrubber's own refusal.
fn scrub_string(scrubber: &dyn Scrubber, content: &str) -> Result<String, crate::scrub::Unmasked>

/// MOD-55 P2: `content` masked for **sending**, under `section`'s name, or the refusal naming the
/// section and the rule and never the text (`R-ID-7`, `R-SEC-3`). The one public string scrub.
///
/// # Errors
/// [`SectionRefused`] when something credential-shaped survives masking.
pub fn scrub_section(
    scrubber: &dyn Scrubber,
    content: &str,
    section: &'static str,
) -> Result<String, SectionRefused>

/// MOD-55 P2: a section that still matched a credential rule after masking. Its `Display` and
/// `Debug` are part of the security contract: a section name and a rule name, never the text,
/// never the pointer (a lone string's pointer is always `""`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the {section} matches the {rule} rule")]
pub struct SectionRefused {
    /// `"name"`, `"body"` or `"request"` for a help prompt.
    pub section: &'static str,
    /// The scrubber's rule name, e.g. `"github_token"`.
    pub rule: &'static str,
}
```

`scrub_text` becomes `scrub_string(..).map_err(|u| AssembleError::Unmasked { section: section.to_owned(), rule:
u.rule, path: u.path })`. Its existing tests must stay green unchanged.

### 1.2 `scrub.rs` (A-11)

`const REDACTED` → `pub const REDACTED: &str = "[REDACTED]";`. Add to its doc: "Public for MOD-55 P9: a proposal
that brings the marker back is flagged before it can replace a real value."

### 1.3 `prompt/edit_help.rs`

Module doc: "The editors' agent help (MOD-55 P6): the fixed prompt a help turn sends and the parser its reply goes
through. Pure: no store, no I/O, no clock. The text is htui's own (`R-ID-5`), not a versioned template (PRD open
question 2)."

```rust
use crate::prompt::template::{Placeholder, TemplateRole};
use crate::prompt::{SectionRefused, scrub_section};
use crate::scrub::{REDACTED, Scrubber};

/// What a help turn edits (A-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpTarget {
    /// A prompt template; its role is `TemplateRole::of_name(name)`.
    Template {
        /// `prompt_template.name`.
        name: String,
    },
    /// A skill body; skills have no placeholders.
    Skill {
        /// `skill.name`.
        name: String,
    },
}

/// One help request: what is edited, the body as the editor holds it, and what the user asked.
/// `Debug` is hand-written (lengths only): it rides in `StoreRequest`, which derives `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct HelpPrompt {
    /// What the body is.
    pub target: HelpTarget,
    /// The editor's buffer, unscrubbed.
    pub body: String,
    /// What the user typed.
    pub request: String,
}

impl core::fmt::Debug for HelpPrompt { /* target (its name is a slug), body_len, request_len */ }

impl HelpPrompt {
    /// MOD-55 P2: the name, the body and the request, each through [`scrub_section`] under
    /// `"name"`, `"body"` and `"request"` in that order; the first refusal wins.
    ///
    /// # Errors
    /// [`SectionRefused`].
    pub fn scrubbed(&self, scrubber: &dyn Scrubber) -> Result<Self, SectionRefused>
}

/// The rewrite paragraph every help prompt carries.
pub const INSTRUCTION: &str = /* §1.4, verbatim */;

/// The prompt a help turn sends (§1.4). Call on [`HelpPrompt::scrubbed`]'s output.
#[must_use]
pub fn assemble(prompt: &HelpPrompt) -> String

/// The `sections[]` names of the recorded `prompt` row, in the order [`assemble`] writes them.
#[must_use]
pub const fn sections(target: &HelpTarget) -> &'static [&'static str]
//   Template => &["instruction", "placeholders", "body", "request"]
//   Skill    => &["instruction", "body", "request"]

/// The role's placeholder table as the editor's column draws it: `{{token}} section|scalar`, plus
/// ` required`, in `Placeholder::ALL` order, filtered by `allowed_in(role)`. One table for the
/// prompt and for `templates.rs` `render_editor` (T4 switches to it; the output is byte-identical,
/// which `templates__edit_help.snap` pins).
#[must_use]
pub fn placeholder_table(role: TemplateRole) -> Vec<String>

/// The proposed body in `reply`: the content of its last closed fenced block (§1.5), or `None`.
#[must_use]
pub fn proposal(reply: &str) -> Option<String>

/// MOD-55 P9: whether `proposed` holds the mask marker more often than `sent` did: a masked
/// value came back as `[REDACTED]`.
#[must_use]
pub fn holds_mask(sent: &str, proposed: &str) -> bool
//   proposed.matches(REDACTED).count() > sent.matches(REDACTED).count()

/// The fence for `body`: backticks, one more than its longest backtick run, at least three.
fn fence(body: &str) -> String
```

### 1.4 The text, verbatim

`INSTRUCTION` (one line in the source, `\` continued):

> Rewrite the body below as the request asks. Reply with the whole new body in one fenced code block, fenced with more
> backticks than any run of backticks inside it. Put nothing in the block but the body, and keep anything else you say
> short and outside it. Do not use tools, run commands or edit files: only your reply is read.

The intro line, per target (`{role}` is `TemplateRole::as_str()`):

- Template: ``You are helping edit `{name}`, a {role} prompt template in htui, a terminal tool that runs coding agents.
  It may use only these placeholders, written in double braces, and must keep every one marked required:``
- Skill: ``You are helping edit `{name}`, a skill in htui, a terminal tool that runs coding agents: Markdown
  instructions added to an agent's prompt. It has no placeholders.``

`assemble` writes, joined with `\n`:

1. the intro line;
2. templates only: the `placeholder_table(role)` lines, one per line;
3. a blank line, then `INSTRUCTION`;
4. a blank line, then `The body:`;
5. `fence(body)`, then the body, with `\n` appended when the body is non-empty and does not end in one, then
   `fence(body)` again (an empty body gives the two fence lines adjacent);
6. a blank line, then `The request:`, then `request.trim()`, then a final `\n`.

**Fence rule:** `n = max(3, longest run of consecutive '`' anywhere in body + 1)`. A run anywhere, not only at a line
start, is deliberately conservative, and it is never wrong to fence longer. Tilde fences inside the body cannot close
a backtick fence, so they do not count.

Exact expected output (test E-1), for `HelpPrompt { target: Skill { name: "rust-style" }, body: "Use \`cargo fmt\`.\n",
request: "  add clippy \n" }`:

````text
You are helping edit `rust-style`, a skill in htui, a terminal tool that runs coding agents: Markdown instructions added to an agent's prompt. It has no placeholders.

Rewrite the body below as the request asks. Reply with the whole new body in one fenced code block, fenced with more backticks than any run of backticks inside it. Put nothing in the block but the body, and keep anything else you say short and outside it. Do not use tools, run commands or edit files: only your reply is read.

The body:
```
Use `cargo fmt`.
```

The request:
add clippy
````

### 1.5 `proposal` extraction rule

Over `reply.lines()` (CRLF safe), a CommonMark-shaped fence scan:

- **Opener**: a line with `indent ≤ 3` spaces whose rest starts with a run of `n ≥ 3` of `` ` `` or `~`. For a backtick
  run, the rest after the run must hold no `` ` `` (CommonMark §4.5; MOD-84 A-4). It opens a block with
  `(char, n, indent)`.
- **Closer**: while a block is open, a line with `indent ≤ 3` whose rest starts with a run of `m ≥ n` of the same char,
  followed only by whitespace. Any other line, a shorter or different fence included, is content. **This is the
  nested-fence rule.**
- **Content line**: up to `indent` leading spaces removed. A block's content is its lines, each followed by `\n`.
- **Result**: the content of the **last closed** block, else `None`. Also `None` when:
  - a block is **still open at the end** of the reply (a cut-off reply is never a proposal, even if an earlier block
    closed);
  - the last closed block's content is empty.

Whitespace-only content is `Some`: the save gate decides (T5 shows it).

| # | Reply (Rust literal) | `proposal` |
|---|---|---|
| P-1 | `"Here:\n```\nnew body\n```\nDone."` | `Some("new body\n")` |
| P-2 | `"no block at all"` | `None` |
| P-3 | `"```\nold\n```\ntext\n```md\nnew\n```"` | `Some("new\n")` (last block wins) |
| P-4 | `"````markdown\nA\n```rust\nx\n```\nB\n````"` | `Some("A\n```rust\nx\n```\nB\n")` (nested) |
| P-5 | `"```\na\n```\n```\nb"` | `None` (unclosed after a closed one) |
| P-6 | `"~~~\nx\n~~~"` | `Some("x\n")` |
| P-7 | `"```\nx\n`````"` | `Some("x\n")` (a longer closer closes) |
| P-8 | `"```a`b\nx\n```"` | `None` (line 1 is no opener, line 3 opens and never closes) |
| P-9 | `"```\n```"` | `None` (empty) |
| P-10 | `"```\r\nx\r\n```\r\n"` | `Some("x\n")` |
| P-11 | `"  ```\n  a\n b\n  ```"` | `Some("a\nb\n")` (indent stripped up to the fence's) |
| P-12 | `"```\nx\n``` y\n```"` | `Some("x\n``` y\n")` (text after a run is not a close) |
| P-13 | `"```\n \n```"` | `Some(" \n")` |
| P-14 | `"~~~\n```\n~~~"` | `Some("```\n")` |

### 1.6 T1 tests (`#[cfg(test)] mod tests` in `edit_help.rs`, plus two in `prompt/mod.rs`)

| Test | Asserts |
|---|---|
| E-1 `a_skill_prompt_reads_exactly` | §1.4's listing, byte for byte |
| E-2 `a_template_prompt_carries_its_role_s_table` | `Template { name: "implement" }`: the intro names `` `implement`, a phase prompt template ``; the lines after it equal `placeholder_table(TemplateRole::Phase)`; for `judge` the `{{candidates}} section required` line is present |
| E-3 `a_skill_prompt_has_no_placeholder_table` | no line matching `^\{\{.*\}\} (section\|scalar)` |
| E-4 `the_fence_outruns_every_backtick_run` | body `"a\n```rust\nx\n```\n"` → 4-backtick fence; `` "x `````` y" `` → 7; `"plain"` → 3; `"``"` → 3; and the body round-trips: `proposal` over `assemble`'s output cut after the closing fence gives the body back unchanged |
| E-5 `the_request_comes_last_trimmed` | output ends `"The request:\nadd clippy\n"` |
| E-6 `an_empty_body_is_two_adjacent_fences` | contains `"The body:\n```\n```\n"` |
| E-7 `proposal_cases` | P-1 … P-14, one `assert_eq!` each with the reply in the message |
| E-8 `scrubbed_masks_a_known_value_and_refuses_a_pattern` | `MinimalScrubber::new(["hunter2-secret".into()])`: body `"pw hunter2-secret"` → `"pw [REDACTED]"`; body holding `format!("ghp_{}", "A1b2".repeat(9))` → `Err(SectionRefused { section: "body", rule: "github_token" })`; the same in `request` → `section: "request"`; `to_string()` is `"the body matches the github_token rule"` and contains no `ghp_` |
| E-9 `every_compiled_default_scrubs_clean` | each body in `DEFAULT_TEMPLATES` through `scrub_section(&MinimalScrubber::new([]), body, "body")` is `Ok(body)` unchanged (no false refusal of htui's own texts) |
| E-10 `holds_mask_counts_new_markers_only` | `("a", "a [REDACTED]")` true; `("[REDACTED]", "[REDACTED] b")` false; `("", "")` false |
| E-11 `a_help_prompt_debugs_lengths_only` | `format!("{:?}", prompt)` with body `"SECRET-BODY"` and request `"SECRET-ASK"` holds neither, and holds `body_len: 11` |
| E-12 `sections_follow_the_assembly_order` | the two lists of §1.3 |
| S-1 (`prompt/mod.rs`) `scrub_section_names_the_section_never_the_text` | refusal `Debug` and `Display` hold neither the key nor a pointer |
| S-2 (`prompt/mod.rs`) `scrub_text_still_reports_the_pointer` | the existing `AssembleError::Unmasked` shape is unchanged (`path: ""`) |

Validate: `cargo test -p htui-core --all-features prompt::` and `cargo test -p htui-core --all-features scrub`.
Commit: `feat(mod-55): prompt::edit_help, the help prompt and reply parser; scrub_section (T1)`.

## 2. T2 `ChatRunSpec.phase_name` (P5)

### 2.1 `crates/htui-core/src/model/run.rs`

Before `ChatRunSpec` (after `TIMESTAMPTZ_DIGITS`):

```rust
/// `run_step.phase_name` of a chat's only step (MOD-2 plan D4).
pub const CHAT_PHASE: &str = "chat";

/// `run_step.phase_name` of an editor's help turn (MOD-55 P5): still `run.kind = 'chat'`.
pub const EDIT_HELP_PHASE: &str = "edit_help";

/// `ChatRunSpec.phase_name`'s serde default (A-3).
fn chat_phase() -> String { CHAT_PHASE.to_owned() }
```

The new field goes last in `ChatRunSpec`. Update the struct doc's "(`phase_name = 'chat'`, position 0)" to "(`phase_name`
[`CHAT_PHASE`], or [`EDIT_HELP_PHASE`] for a help turn, position 0)".

```rust
    /// `run_step.phase_name`: [`CHAT_PHASE`], or [`EDIT_HELP_PHASE`] through
    /// [`ChatRunSpec::for_edit_help`] (MOD-55 P5). No CHECK constrains the column.
    #[serde(default = "chat_phase")]
    pub phase_name: String,
```

`mint` sets `phase_name: CHAT_PHASE.to_owned()`. After `mint`:

```rust
    /// MOD-55 P5: the same pair, marked as an editor's help turn.
    #[must_use]
    pub fn for_edit_help(mut self) -> Self {
        self.phase_name = EDIT_HELP_PHASE.to_owned();
        self
    }
```

Export `CHAT_PHASE` and `EDIT_HELP_PHASE` wherever `model/mod.rs` re-exports `ChatRunSpec`.

### 2.2 Both stores

- `crates/htui-core/src/store/mem.rs` `start_chat_run` (~2184): `phase_name: chat.phase_name.clone(),`.
- `crates/htui-store/src/pg/write.rs` `start_chat_run` (~1932): the step insert becomes
  `"... VALUES ($1, $2, 0, 1, 0, $3, $4, $5, 'running', $6, NULL) ..."`, binding `chat.step_id.as_uuid()`,
  `chat.run_id.as_uuid()`, `chat.phase_name.as_str()`, `chat.agent_id.map(AgentId::as_uuid)`,
  `chat.model.as_deref()`, `chat.started_at`. Doc (line ~1903): "`phase_name 'chat'`" becomes "the spec's
  `phase_name` (`'chat'`, or `'edit_help'` for a help turn, MOD-55 P5)". Keep the literal's line breaks and `\`
  continuations in the existing style. The hash is over the joined literal.

### 2.3 sqlx offline entry (sandbox recipe)

`query-a9a4ffc803fe6a57ddf84987a27e4412ee1972530e0568f612c6ced1f2a14ce8.json` (the `'chat'` insert) goes, and one new
file comes. In the hr sandbox: no docker, Postgres on `localhost:5439`, user `postgres`, trust auth. The compose `htui`
DB is empty and `prepare` needs a **migrated** scratch DB (`docs/hr-sandbox.md` "Changing SQL queries in a run"):

```bash
psql -h localhost -p 5439 -U postgres -c "DROP DATABASE IF EXISTS htui_sqlx;"
psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx;"
cd /home/mluigi/projects/htui/crates/htui-store          # inside the crate, never the workspace root
export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
cargo sqlx migrate run --source migrations
cargo sqlx prepare -- --all-targets --all-features       # both flags, or feature-gated entries are deleted
cargo sqlx prepare --check                               # "potentially unused queries" warning is expected
git status --short .sqlx                                 # exactly: one D (a9a4ffc8...), one ?? (new hash)
ls .sqlx | wc -l                                         # still 348
```

If the count or the status shows anything else, run `git checkout -- .sqlx && git clean -fq .sqlx` and re-run with the
flags.

### 2.4 T2 tests

| Test | Where | Asserts |
|---|---|---|
| R-1 `start_chat_run_mints_chat_rows` (extended, A-10) | `htui-core/src/store/conformance.rs:1862` | after the first mint: `store.run_steps(chat.run_id)` is one step with `phase_name == CHAT_PHASE`. A second spec `ChatRunSpec::mint(..).for_edit_help()` mints, its `run_steps` step has `phase_name == EDIT_HELP_PHASE`, and `finish_chat_run` closes it `Done`. Runs on Mem and on Pg (`pg_conformance.rs`, feature `demo`) |
| R-2 `a_help_chat_run_records_edit_help` | `mem.rs` tests, beside `a_chat_run_mints_both_rows_running_and_finish_closes_them` (~8540) | the step row's `phase_name == "edit_help"`, `run.kind == RunKind::Chat`, `item_id == None` |
| R-3 `a_spec_without_phase_name_deserialises_as_chat` | `model/run.rs` tests | `serde_json` of a minted spec with the key removed reads back `phase_name == "chat"`; `for_edit_help` keeps every other field |

Validate: `cargo test -p htui-core --all-features store`; `cargo test -p htui-store --all-features --test pg_conformance
-- --test-threads=1` (`HTUI_TEST_DATABASE_URL` is preset in the sandbox; "skipped" means it did not run);
`SQLX_OFFLINE=true cargo check -p htui-store --all-features --all-targets`.
Commit: `feat(mod-55): ChatRunSpec.phase_name, edit_help runs marked in both stores (T2)`.

## 3. T3 `EditHelp` through the agent runtime

Files: `crates/htui/src/store_worker.rs`, `crates/htui/src/agent_worker.rs`, `crates/htui/src/testkit.rs` (A-8).

### 3.1 `store_worker.rs`

- After `PROMOTION_NEEDS_CHAT` (~103): `/// MOD-55: the help request's name, shared by the runtime and the editors. pub const EDIT_HELP: &str = "edit_help";`
- Variant, after `ChatFollow` (~240):

```rust
    /// MOD-55: one agent help turn for a Skills editor (P1-P4). The runtime assembles and scrubs
    /// the prompt **before** anything is minted (a refusal is a `Failed` naming the section and the
    /// rule, and leaves no run), mints a `chat` run whose step is `edit_help`, and opens a session
    /// with no tool lease, every tool kind denied and a deny-all policy. The session ends itself
    /// after its first turn. Answered as [`StoreRequest::ChatStart`] is: `ChatAccepted`, `Chat`
    /// frames, `Ended`. `ChatCancel` ends it early.
    EditHelp {
        /// The run's project: the template's own, or the scope's first for a skill (P7).
        project_id: ProjectId,
        /// Which registry row answers, with its default model (PRD D1).
        agent_id: AgentId,
        /// What is edited, the body and the request. Its `Debug` is lengths only.
        prompt: htui_core::prompt::edit_help::HelpPrompt,
    },
```

- `name()`: `Self::EditHelp { .. } => EDIT_HELP,` after the `ChatFollow` arm (1033).
- `try_serve`'s "no agent runtime" or-pattern (1810-1826): add `| StoreRequest::EditHelp { .. }`. The comment's
  "all seventeen" becomes "all eighteen", and "The five chat requests" becomes "The six chat requests".
- `spawn_with_concepts`'s runtime arm (2649-2665): add `| StoreRequest::EditHelp { .. }`. `Served::Start` is attached
  exactly as for `ChatStart`.

### 3.2 `testkit.rs`

`Harness::drive`'s or-pattern (281-296): add `| StoreRequest::EditHelp { .. }`.

### 3.3 `agent_worker.rs`: new items

Imports: `htui_agent::driver::{PermissionDefault, PermissionPolicy, ToolExposure}` (whichever are not already in),
`htui_agent::event::ToolKind`, `htui_core::prompt::edit_help::{self, HelpPrompt}`, `crate::store_worker::EDIT_HELP`.

After `impl ChatBinding` (before `PROMOTE_STEP`, ~2425):

```rust
/// MOD-55 P1: what `start` sends first.
#[derive(Debug)]
enum Opening {
    /// `ChatStart`'s text, sent as typed (the chat path's own `R-ID-7` gap, out of scope here).
    Chat(String),
    /// `EditHelp`'s prompt: assembled and scrubbed before anything is minted (P2).
    Help(HelpPrompt),
}

/// MOD-55 P1: what a session `start` opened is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatMode {
    /// MOD-2's chat: turns until the user ends it.
    Conversation,
    /// An editor's help turn: one turn, then the session ends itself (P4).
    Help {
        /// The `prompt` row's section names ([`edit_help::sections`]).
        sections: &'static [&'static str],
    },
}

impl ChatMode {
    /// Whether this is a help turn.
    const fn is_help(self) -> bool

    /// The request a failed start is answered as: [`EDIT_HELP`] for help, else the binding's.
    const fn request(self, binding: &ChatBinding) -> &'static str

    /// `answering`'s label: `"edit_help"` or `"chat"`.
    const fn label(self) -> &'static str
}
```

After `chat_policy` (~2276):

```rust
/// MOD-55 P3: a help session answers every permission request "no" by policy (stage 3 is
/// never reached, so nothing parks on a user who is not looking at a permission prompt).
fn help_policy() -> PermissionPolicy {
    PermissionPolicy { default: PermissionDefault::Deny, rules: Vec::new(), remembered: Vec::new() }
}

/// MOD-55 P3, A-6: every tool kind denied. `claude-cli` inverts it to `--disallowedTools`; the ACP
/// `fs/*` handlers refuse `read` and `edit`.
fn help_exposure() -> ToolExposure {
    ToolExposure { deny_kinds: ToolKind::ALL.to_vec(), ..ToolExposure::default() }
}

/// The `sections[]` of a help turn's `prompt` row: `prompt_sections`' shape, one entry per name.
fn help_sections(names: &[&str]) -> Value
//   Value::Array(names.iter().map(|name| json!({ "name": name, "tokens": Value::Null, "trimmed": false })).collect())
```

`ChatArgs` gains `/// MOD-55 P1: a chat, or a help turn. mode: ChatMode,`, which also goes into its hand-written
`Debug`. All three `ChatArgs { .. }` literals set it: `bind_promoted` (~1124) `ChatMode::Conversation`, `start` (~2245)
the computed mode, and the test `the_resume_failed_frame_is_the_scrubbed_row` (~5992) `ChatMode::Conversation`.

### 3.4 `serve` (~1171)

After the `ChatStart` arm, which now passes `Opening::Chat(prompt.clone())`:

```rust
            StoreRequest::EditHelp { project_id, agent_id, prompt } => {
                match self
                    .start(backend, replies, addr, *project_id, *agent_id, None,
                           Opening::Help(prompt.clone()))
                    .await
                {
                    Ok(served) => served,
                    Err(err) => Served::Reply(failed(EDIT_HELP, &err)),
                }
            }
```

### 3.5 `start`: where help diverges, in order

The signature's last parameter `prompt: String` becomes `opening: Opening`. The arity is unchanged, so the existing
`#[expect(clippy::too_many_arguments)]` stays. Run `change(operation:"verify")`. `serve` is the only caller.

1. `writer` check unchanged. **Offline refuses first** with `StoreError::Unreachable(DATABASE_UNREACHABLE)`, so P11
   needs no new code. `serve`'s arm renders it `edit_help: store unreachable: <sentence>`.
2. **New, immediately after the writer check:** `let env: BTreeMap<String, String> = BTreeMap::new();` (moved up from
   the `SessionSpec` literal, keeping its MOD-10 comment). Then:

```rust
        let (prompt, mode) = match opening {
            Opening::Chat(text) => (text, ChatMode::Conversation),
            Opening::Help(help) => {
                // P2: the session's own scrubber, before the box, the registry row, the lease or the
                // run: a refusal leaves nothing behind and reaches no driver.
                let scrubber = MinimalScrubber::new(env.values().cloned());
                match help.scrubbed(&scrubber) {
                    Ok(clean) => (
                        edit_help::assemble(&clean),
                        ChatMode::Help { sections: edit_help::sections(&clean.target) },
                    ),
                    Err(refused) => {
                        return Ok(Served::Reply(StoreReply::Failed {
                            request: EDIT_HELP,
                            message: format!("not sent: {refused}"),
                        }));
                    }
                }
            }
        };
```

3. `box_id`, `user`, `cwd`, registry row, `enabled`, `refuse_switched_off`, driver, settings, `model` (None → the
   row's `default_model`), project caps and quota latch: **unchanged**. Help shares every chat refusal.
4. `let chat = ChatRunSpec::mint(..);` then `let chat = if mode.is_help() { chat.for_edit_help() } else { chat };`.
5. `let lease = if mode.is_help() { None } else { /* the existing match on self.tools */ };`. **No MCP lease**, so no
   `prompt` port and no `mcp` servers.
6. `let policy = if mode.is_help() { help_policy() } else { chat_policy(settings.permission, lease.as_ref()) };`.
7. `writer.start_chat_run(&chat)` and `tests::minted(&chat)`: unchanged. The refusal in step 2 is above this line.
8. `SessionSpec`: `env` (from step 2), `tools: if mode.is_help() { help_exposure() } else { ToolExposure::default() }`;
   `mcp` and `prompt` are empty and `None` because the lease is `None`. Everything else unchanged.
9. Re-probes unchanged (B-3). `answering(mode.label(), run_chat(args), Some(answer))`.

### 3.6 `run_chat`: where help diverges

- Destructure `mode` from `ChatArgs`.
- Start-failure path (~4207): `request: binding.request()` becomes `request: mode.request(&binding)`, so a help whose
  spawn fails is answered `Failed { request: "edit_help" }` followed by `ChatFrame::Failed`.
- `ChatBinding::Fresh` prompt row (~4273):
  `recorder.record_prompt(&opening_text, match mode { ChatMode::Help { sections } => help_sections(sections), ChatMode::Conversation => prompt_sections() }, now)`.
  `prompt_sections()` itself is unchanged.
- **End after the first turn (P4).** In the `loop`, after the `match run_turn(..)` (whose `Done` arm sets `last_stop`)
  and **before** `commands.recv()`:

```rust
        // MOD-55 P4: a help session is one turn, ended exactly as a cancel between turns ends a
        // chat: nothing was cut, so the run closes `done` with the turn's own stop reason, and the
        // `Ended` below is the stream's last frame. No `ChatCommand` is read: a `ChatCancel` that
        // raced the turn's end is dropped with the receiver and the tab ends on this `Ended`.
        if mode.is_help() {
            let _ = session.cancel(grace).await;
            drain(session.as_mut(), &mut recorder, &mut ui_rx, &frames).await;
            break;
        }
```

  (`Cancelled`, `CapExceeded` and `Err` arms already `break` before this point.)
- **A-7:** in the `if let Err(err) = recorder.finish().await` block, after `record_failure`, add
  `if mode.is_help() { frames.failed(chat_failure(&err).unwrap_or_else(|| "the reply could not be recorded".to_owned())); }`.
  `chat_failure` keeps its `"chat"` label: it is the `run.failure` text, and the run is a chat.
- Then `binding.close(&writer, status)` and `frames.ended(last_stop)`, unchanged. The sequence a help run sends: a
  `ChatAccepted` at the request's address, `Event` frames (the local `prompt` frame first), then `Ended`, preceded by
  `Failed` when the recorder refused a row.

### 3.7 What the plan asked to confirm

- **`deny_kinds` to argv:** `cli/mod.rs` `disallowed()` (~235) appends `claude::tool_names(kind)` for each kind after
  `tools.deny`, which gives one `--disallowedTools=<names>` argument. `--allowedTools` is never emitted, and
  `--tools=` is omitted because `allow` is empty (A-6 has the list).
- **`--permission-mode` vs `Deny`:** on `claude-cli` without a lease there is no `--permission-prompt-tool`, so the
  CLI never asks htui and `PermissionPolicy` is inert there. The row's own `--permission-mode` (and its
  `extra_args`, last) still apply. On CLI, `--disallowedTools` is the only htui-side gate (H-6).
- **ACP:** `session/request_permission` reaches `run_turn` → `permission::evaluate`. With no rules or remembered
  entries, `default: Deny` answers `reject_once`, or `reject_always` through the sibling. `fs/read_text_file` and
  `fs/write_text_file` are refused by `deny_kinds` (`acp/mod.rs:1562,1573`).

### 3.8 T3 tests

`agent_worker.rs` tests. Add helpers next to `start` (~5384):

```rust
    fn help(agent_id: AgentId, body: &str, request: &str) -> StoreRequest // EditHelp, PROJECT_HTUI, Skill { name: "rust-style" }
    /// Drives an `EditHelp` to its end with **no** cancel queued: the session ends itself (P4).
    async fn run_help(runtime: &mut AgentRuntime, backend: &Backend, request: StoreRequest) -> (StepId, Vec<ReplyEnvelope>)
```

The script for the happy cases is `Script::one_turn` of
`AssistantChunk("Here:\n```\nnew\n```\n", message_id "m1")` and `Done(EndTurn)`.

| Test | Asserts |
|---|---|
| T3-a `a_help_turn_ends_itself_after_one_turn_and_closes_done` | no `ChatCancel` sent; `task.await` returns; replies: `[0]` `ChatAccepted`, exactly one `Chat(Ended { EndTurn })` and it is last, every stream frame at seq 7; the run (`tests::minted` gives `run_id`) is `RunStatus::Done`; `store.step_events(step)` holds a `prompt` row whose payload `sections` names are `["instruction","body","request"]` and whose text equals `edit_help::assemble(..)`, plus the `assistant_text` row |
| T3-b `a_body_with_a_key_pattern_is_refused_before_anything_is_minted` | body holding `format!("ghp_{}", "A1b2".repeat(9))`: `Served::Reply(Failed { request: "edit_help", message: "not sent: the body matches the github_token rule" })`; `runtime.steps().is_empty()`; `active_runs` unchanged; the `SpecSlot` is `None` (no driver start); `message` holds no `ghp_`. Same with the key in `request` (section `request`) |
| T3-c `a_help_session_has_no_tools_no_lease_and_denies_every_permission` | via `fixture_with_spec_spy`: `spec.mcp.is_empty()`, `spec.prompt.is_none()`, `spec.permission == PermissionPolicy { default: Deny, rules: vec![], remembered: vec![] }`, `spec.tools.deny_kinds == ToolKind::ALL`, `spec.tools.allow.is_empty()`, `!spec.tools.command_run`, `spec.model` is the row's `default_model` |
| T3-c2 `a_help_session_opens_no_tool_lease_on_a_hosted_runtime` | in the MCP module with `hosted(..)` (~6478): the same request on a runtime with a tool host gives `spec.mcp.is_empty()` and `spec.prompt.is_none()` (a chat on the same runtime gets a server, which is the control) |
| T3-d `a_help_step_is_phase_edit_help_and_a_chat_step_stays_chat` | `store.run_steps(run_id)`: `phase_name == "edit_help"` for help, `"chat"` for `start(..)` |
| T3-e `an_offline_backend_refuses_help_with_the_unreachable_sentence` | mirror of `an_offline_backend_refuses_a_chat_with_the_unreachable_warning` (~7195): `request == "edit_help"`, `message == format!("store unreachable: {}", DATABASE_UNREACHABLE)`, no steps |
| T3-f `a_refused_reply_row_fails_the_help_before_it_ends` (A-7) | the chunk text holds a real-shaped key: replies hold `Chat(Failed { .. })` **before** the final `Chat(Ended { .. })`; the run is `Failed` |
| T3-g `the_driver_is_sent_the_scrubbed_assembly` | `fixture_with_failing_starts(script, vec![])` + `starts_of`: the one start's prompt equals `edit_help::assemble(&prompt)` |
| T3-h `a_help_whose_spawn_fails_is_answered_as_edit_help` | `fixture_with_failing_starts(.., vec![DriverError::Spawn(..)])`: a `Failed { request: "edit_help" }` stream reply, then `Chat(Failed)` |
| T3-i `a_cancel_mid_turn_closes_the_help_cancelled` | a script whose turn stalls (`ScriptEvent` without its own `done`, as the existing mid-turn cancel tests use): `ChatCancel` → reply `Ended { Cancelled }` at the cancel's seq; run `Cancelled` |
| W-1 (`store_worker.rs` tests, ~4640) | `EditHelp { .. }.name() == "edit_help" == EDIT_HELP` |
| W-2 `an_edit_help_request_debugs_without_its_body` (beside `an_auth_deliver_request_debugs_without_its_url`) | neither body nor request text in `{request:?}` / `{envelope:?}` |
| W-3 (extend `try_serve_without_a_runtime_refuses_all_four_by_name` or a sibling) | `EditHelp` without a runtime is `Failed { "edit_help", "no agent runtime in this build" }` |

Validate: `cargo test -p htui --all-features --lib agent_worker -- --test-threads=1`, then `--lib store_worker`, then
`--lib testkit`. Commit: `feat(mod-55): StoreRequest::EditHelp, one scrubbed tool-less turn through the chat runtime
(T3)`.

## 4. T4 `AgentHelp` + Templates editor

Files: `crates/htui/src/ui/tabs/skills/agent_help.rs` (new), `skills/mod.rs` (`mod agent_help;` after
`mod attach;`), `skills/templates.rs`, `crates/htui/tests/templates.rs`, snapshots.

### 4.1 Types (`agent_help.rs`)

```rust
/// One line of report for the view's notice row (each view maps it onto its own `Notice`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Report { Info(String), Error(String) }

/// What a key or a reply did to the help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HelpOutcome {
    /// Taken; nothing for the view to do.
    Consumed,
    /// Not the help's: `Tab`/`Shift+Tab` and chords it does not own go to the shell.
    Pass,
    /// The help stays open; the view shows this.
    Note(Report),
    /// The help closes, the buffer as it was; the view shows the report, if any.
    Close(Option<Report>),
    /// The help closes and this text replaces the buffer.
    Accept(String),
}

/// The editors' agent help (MOD-55 P8). `Debug` is hand-written: lengths, never the body, the
/// request, the reply or the proposal.
pub(super) struct AgentHelp {
    /// What is edited, for the request.
    target: HelpTarget,
    /// The run's project (P7).
    project: ProjectId,
    /// The buffer when the help opened: what is sent and what the diff is against (PRD OQ-4).
    sent: String,
    /// Enabled agents from the last `Agents` reply, name order; `None` before it.
    agents: Option<Vec<(AgentId, String)>>,
    /// Index into `agents`.
    agent: usize,
    /// The request as typed; kept after sending so the panel still shows it.
    request: TextField,
    /// Where it is.
    state: State,
    /// The Proposal/Answered pane's scroll, and its rows at the last draw.
    scroll: Scroll,
    rows: Cell<usize>,
}

enum State {
    /// Typing the request, choosing the agent.
    Asking,
    /// `EditHelp` sent, no `ChatAccepted` yet. `cancel`: `Esc` was pressed (A-2).
    Starting { agent: String, cancel: bool },
    /// Accepted; the reply's text accumulates.
    Streaming { step_id: StepId, agent: String, reply: String },
    /// `ChatCancel` sent; waiting for the stream's terminal frame.
    Cancelling,
    /// The reply's last fenced block, diffed against `sent`.
    Proposal { agent: String, proposed: String, unified: String, masked: bool, armed: bool },
    /// A reply with nothing to accept: no block, or the body as sent.
    Answered { agent: String, text: String, same: bool },
}
```

API (all `pub(super)`):

```rust
impl AgentHelp {
    /// Opens on `body` and asks for the agents (B-1): `ctx.request(StoreRequest::Agents)`.
    fn open(target: HelpTarget, project: ProjectId, body: &str, ctx: &Ctx<'_>) -> Self
    fn on_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> HelpOutcome
    fn on_paste(&mut self, text: &str)               // Asking: into `request`; otherwise dropped
    fn on_reply(&mut self, reply: &StoreReply, ctx: &Ctx<'_>) -> HelpOutcome  // Consumed = nothing to do
    /// Draws into `area`; returns the rect left for the locked draft, `None` when the help takes it all.
    fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) -> Option<Rect>
    fn hint(&self) -> &'static str
}
```

### 4.2 Keys per state (first match wins; `chord` = `CONTROL` with `SHIFT` allowed)

| State | Key | Outcome |
|---|---|---|
| any | `Tab`, `BackTab` | `Pass` (the shell switches tabs; the help and the draft are kept) |
| any | chord `s`/`S`, `e`/`E`, `g`/`G` | `Note(Info(HELP_OPEN))` |
| any | other chords | `Pass` |
| Asking | `Up` / `Down` | previous / next agent, wrapping; `Consumed` |
| Asking | `Enter` | empty `request.text().trim()` → `Note(Error(ASK_SOMETHING))`; agents `None` → `Note(Info(READING_AGENTS))`; empty → `Note(Error(NO_AGENT))`; else `ctx.request(EditHelp { project_id, agent_id, prompt: HelpPrompt { target, body: sent, request } })` → `Starting { agent, cancel: false }`, `Consumed` |
| Asking | `Esc` | `Close(None)` (nothing was sent) |
| Asking | anything else | `request.on_key(key)`; `Consumed` |
| Starting | `Esc` | `cancel = true`; `Note(Info(CANCELLING))` |
| Streaming | `Esc` | `ctx.request(ChatCancel { step_id })` → `Cancelling`; `Note(Info(CANCELLING))` |
| Starting, Streaming, Cancelling | anything else | `Consumed` (the buffer is locked) |
| Proposal | `Enter`, `y` | `masked && !armed` → `armed = true`, `Note(Error(MASKED))`; else `Accept(proposed)` |
| Proposal | `Esc`, `n` | `Close(Some(Info(DISCARDED)))` |
| Proposal, Answered | `J`/`K`/`PageDown`/`PageUp` | `scroll.on_key(key, rows)` |
| Answered | `Esc`, `Enter`, `n` | `Close(None)` |
| Proposal, Answered | anything else | `Consumed` |

### 4.3 Replies per state (A-1: what makes a stray frame harmless)

| State | Reply | Outcome |
|---|---|---|
| Asking | `Agents(list)` | keep `enabled` rows as `(id, name)`; clamp `agent`; `Consumed` |
| Starting | `ChatAccepted { step_id }` | `cancel` → `ctx.request(ChatCancel { step_id })`, `Cancelling`; else `Streaming { reply: "" }` |
| Starting | `Failed { request: EDIT_HELP, message }` or `Chat(Failed { message })` | `Close(Some(Error(message)))` |
| Starting | `Chat(Ended)`, `Chat(Event)` | ignored (a late cancel reply of an earlier help) |
| Streaming | `Chat(Event(env))` with `DriverEvent::AssistantChunk(c)` | `reply.push_str(&c.text)` |
| Streaming | `Chat(Ended { EndTurn })` | `finish()` (below) |
| Streaming | `Chat(Ended { other })` | `Close(Some(Error(format!("the agent stopped ({other}) \u{2014} nothing to accept"))))` |
| Streaming | `Chat(Failed { message })`, `Failed { EDIT_HELP, message }` | `Close(Some(Error(message)))` |
| Cancelling | `Chat(Ended)`, `Chat(Failed)`, `Failed { request: "chat_cancel" \| EDIT_HELP }` | `Close(Some(Info(CANCELLED)))` |
| Proposal, Answered | anything | ignored |

`finish()`: `match edit_help::proposal(&reply)`. If `Some(mut p)` and `!sent.ends_with('\n')`, strip one trailing
`\n` from `p` (it matches the sent body's last line ending, so the diff is not a lone "no newline" hunk). Then
`p == sent` → `Answered { same: true }`; otherwise `Proposal { unified: diff::unified(&sent, &p, "sent", "proposed"),
masked: edit_help::holds_mask(&sent, &p), armed: false }`. `None` → `Answered { same: false }`.

Texts (consts in `agent_help.rs`):

| Const | Text |
|---|---|
| `HELP_OPEN` | `agent help is open \u{2014} Esc leaves it first` |
| `ASK_SOMETHING` | `type what you want changed` |
| `READING_AGENTS` | `reading the agents\u{2026}` |
| `NO_AGENT` | `no enabled agent \u{2014} add one in Settings > Agents` |
| `CANCELLING` | `cancelling\u{2026}` |
| `CANCELLED` | `agent help cancelled` |
| `DISCARDED` | `proposal discarded` |
| `MASKED` | `the proposal holds [REDACTED] where a masked value was \u{2014} Enter again replaces the body anyway` |
| `ACCEPTED` (used by the views) | `proposal accepted \u{2014} Ctrl+S saves it` |

Hints (`hint()`):

| State | Hint |
|---|---|
| Asking | `Enter ask  Up/Down agent  Esc back` |
| Starting, Streaming | `waiting for the agent  Esc cancel` |
| Cancelling | `cancelling\u{2026}` |
| Proposal | `Enter accept  Esc discard  J/K PgUp/PgDn scroll` |
| Answered | `nothing to accept  Esc close  J/K scroll` |

### 4.4 Render (B-2)

- Asking, Starting, Streaming, Cancelling: `Layout::vertical([Min(1), Length(4)])`. Returns `Some(top)` for the draft.
  The bottom is a bordered block titled ` ask an agent ` with two lines:
  - `ask: ` + `request.line(budget, focused = Asking, theme)`;
  - `agent: {name}` (Asking; `reading\u{2026}` before the reply; `none enabled` when empty), `{agent} is
    starting\u{2026}`, `{agent} is answering\u{2026} {n} chars`, or `cancelling\u{2026}`.
- Proposal: returns `None`. One block over `area`, titled ` proposal from {agent} \u{b7} sent \u{2192} proposed `, plus
  ` \u{b7} holds [REDACTED] ` when `masked` (P9), drawn in `theme.error`. Lines: `diff::lines(&unified, theme)`,
  wrapped, `scroll`, and `rows` set as `render_browse` sets `pane_rows`.
- Answered: returns `None`. Titled ` {agent} replied \u{b7} no fenced block ` (or ` \u{b7} the body as sent ` when
  `same`). Lines: the reply text, wrapped and scrolled.

### 4.5 `templates.rs` wiring

- `Editor` gains `/// MOD-55: the agent help, while open; the draft is locked under it. help: Option<AgentHelp>,`.
  `Editor::new` sets `None`, and the hand-written `Debug` adds `.field("help", &self.help)` (that `Debug` is
  lengths-only).
- `EDIT_HINT` → `"Ctrl+S save  Ctrl+G ask agent  Ctrl+E $EDITOR  Esc cancel"`.
- `on_editor_key`, at the **top** (before the `Ctrl+E` check):
  1. if the editor's `help` is `Some`: `let outcome = help.on_key(key, ctx);`, then apply it with `apply_help(outcome)`
     (below) and return `Handled::Consumed`, or `Handled::Pass` for `Pass`;
  2. then `chord && Char('g' | 'G')`:
     - `busy` → `Notice::Error(format!("`{busy}` is still in flight"))`;
     - else `editor.help = Some(AgentHelp::open(HelpTarget::Template { name: editor.name.clone() }, editor.project,
       editor.area.text(), ctx))`, `editor.esc_armed = false`, `self.notice = None`.
  
  `Ctrl+E` and `Ctrl+S` are refused while help is open, because step 1 takes them.
- `fn apply_help(&mut self, outcome: HelpOutcome) -> Handled` (private):
  - `Note(r)` → `self.notice` from `r`;
  - `Close(r)` → `editor.help = None`, `esc_armed = false`, notice from `r` or `None`;
  - `Accept(text)` → `editor.help = None`, then **exactly the `ExternalEditOutcome::Edited` gate**: `editor.area =
    TextArea::with_text(&text)`, `confirm_item = false`, `esc_armed = false`, `parse(role, text)`. An `Err` puts the
    cursor on `error_at` (or the end) and sets `Notice::Error(err)`. `Ok` sets `Notice::Info(ACCEPTED)`. `original`
    is not touched, so `Esc` then asks about unsaved changes.
- `on_reply`: first, `if let Mode::Editing(editor) = &mut self.mode && let Some(help) = editor.help.as_mut()`, then
  `apply_help(help.on_reply(reply, ctx))` when the outcome is not `Consumed`. Then the existing arms run unchanged.
  (`Failed { "edit_help" }` matches neither `READ_NAME` nor `REQUEST_NAMES`.)
- `on_paste`: in `Mode::Editing`, if `help` is `Some`, call `help.on_paste(text)` and return `true` without touching
  the area.
- `render`: with `help` `Some`, the hint is `help.hint()` (no `L:C`).
- `render_editor`: `left` → `match &editor.help { Some(help) => help.render(frame, left, theme), None => Some(left) }`.
  The draft block draws into the returned rect when `Some`. The placeholder column draws from
  `edit_help::placeholder_table(role)` (one table, §1.3), and the output stays byte-identical.
- `on_scope_change`: unchanged. The view resets and the help goes with it, while the session ends itself after its
  turn (H-9).

### 4.6 T4 tests

`agent_help.rs` unit tests (own small bench: a `Ctx` over `Scope`, `TopBarState`, `Keymap::default_global()`,
`Theme::default()`, `Emit`, as `templates.rs` `Bench`). Fixtures: `summary(name, enabled)` builds an
`AgentSummary` over `htui_core::fixtures` rows. Event frames come from
`StoreReply::Chat(ChatFrame::Event(Box::new(DriverEnvelope { event: AssistantChunk(..), raw: None, at })))`.

| Test | Asserts |
|---|---|
| H-1 `open_asks_for_the_agents` | the one emitted request is `StoreRequest::Agents` |
| H-2 `enter_sends_edit_help_to_the_first_enabled_agent` | `Agents([b (disabled), c, d])` → type `shorter`, `Enter` → `EditHelp { project_id, agent_id: c, prompt.request: "shorter", prompt.body: sent }`; state `Starting` |
| H-3 `up_and_down_cycle_the_agents_and_wrap` | `Down` ×2 from `c` → `c`; `Up` → `d` |
| H-4 `an_empty_request_or_no_agent_sends_nothing` | the three notes of §4.2; nothing emitted |
| H-5 `chunks_accumulate_and_end_turn_proposes_the_last_block` | accepted → two chunks split inside the fence → `Ended { EndTurn }` → `Proposal`, `unified` holds `-old` and `+new` |
| H-6 `a_reply_with_no_block_or_the_same_body_is_answered` | both `Answered` variants |
| H-7 `a_stop_other_than_end_turn_proposes_nothing` | `Cancelled`, `MaxTokens` → `Close(Some(Error(..)))` |
| H-8 `enter_accepts_and_esc_or_n_discards` | `Accept(proposed)`; `Close(Some(Info(DISCARDED)))` |
| H-9 `a_masked_proposal_needs_a_second_accept` | first `Enter` → `Note(Error(MASKED))`, second → `Accept`; a marker already in `sent` is not flagged |
| H-10 `esc_while_streaming_cancels_and_the_end_closes` | emits `ChatCancel { step_id }`; `Ended` → `Close(Some(Info(CANCELLED)))` |
| H-11 `esc_before_acceptance_cancels_on_acceptance` | `Esc` in `Starting` emits nothing; `ChatAccepted` emits `ChatCancel` |
| H-12 `a_failed_start_closes_with_its_message` | `Failed { "edit_help", m }` and `Chat(Failed { m })` in `Starting` → `Close(Some(Error(m)))` |
| H-13 `a_stray_end_before_acceptance_is_ignored` | `Chat(Ended)` in `Starting` → `Consumed`, state unchanged |
| H-14 `tab_passes_and_the_buffer_is_locked` | `Tab` → `Pass`; `x` in `Streaming` → `Consumed` |
| H-15 `ctrl_s_ctrl_e_ctrl_g_are_refused_while_open` | `Note(Info(HELP_OPEN))` in every state |
| H-16 `paste_reaches_the_request_only_while_asking` | |
| H-17 `the_proposal_keeps_the_sent_body_s_last_line_ending` | `sent = "a"`, block `"b\n"` → `proposed == "b"` |
| H-18 `help_debugs_lengths_only` | no body, request, reply or proposal text in `{:?}` |

`templates.rs` `Bench` tests:

| Test | Asserts |
|---|---|
| TV-1 `ctrl_g_opens_help_and_asks_for_agents` | after `e` on `implement`, `Ctrl+G` emits `Agents`; the editor's `help` is `Some` |
| TV-2 `the_help_request_carries_the_editor_s_project_and_name` | → `EditHelp { project_id: PROJECT_VULKAN, prompt.target: Template { name: "implement" } }` |
| TV-3 `ctrl_g_is_refused_while_a_save_is_in_flight` | via `save_implement`: notice `` `save_template` is still in flight ``; no help |
| TV-4 `an_accepted_proposal_replaces_the_draft_through_parse` | accept `{{itme}}` → cursor at its braces, `Notice::Error("unknown prompt placeholder...")`; nothing sent |
| TV-5 `typing_under_an_open_help_leaves_the_draft_alone` | |

`tests/templates.rs` (harness: `Harness::over(store).with_agent_runtime(AgentRuntime::new(factory).with_grace(0))`,
then `register_all`. The fixture's agents are disabled and a scripted `cli/fake` row is added, as `tests/chat.rs`
`harness_with` does; lift its `scripted_row`/`SharedAdapter` pattern):

| Test | Asserts |
|---|---|
| TI-1 `ctrl_g_proposal_accept_and_save` | `implement`, `e`, `ctrl+g`, type `shorter`, `enter`, `drive()` → the frame shows ` proposal from scripted`; snapshot **`templates__agent_help_proposal`**; `enter` → `ctrl+s` → `drive()` → `head(store, "implement")` is v2 with the proposed body; the store holds one chat run whose step is `edit_help`, closed `done` |
| TI-2 `the_asking_panel` | snapshot **`templates__agent_help_asking`** (request typed, agent shown, hint row) |
| TI-3 `a_body_holding_a_key_is_not_sent` | type a real-shaped key into the draft, `ctrl+g`, ask → the notice reads `not sent: the body matches the github_token rule`; the help is closed; no chat run exists |
| TI-4 update `a_refused_save_sends_no_request` (line 399) | `hint(&frame).ends_with("Ctrl+S save  Ctrl+G ask agent  Ctrl+E $EDITOR  Esc cancel  L1:C1")` |

Script for TI-1:
`AssistantChunk("Shorter:\n```\n{{item}}\n\nDo the item.\n```\n")` then `Done(EndTurn)`, a body `parse` accepts for a
phase.

Validate: `cargo test -p htui --all-features --lib skills -- --test-threads=1`; `cargo test -p htui --all-features
--test templates -- --test-threads=1`; then §5.6. Commit: `feat(mod-55): AgentHelp, Ctrl+G in the Templates editor
(T4)`.

## 5. T5 Library editor, T6 docs, snapshots, gates

### 5.1 T5 `library.rs`

Same as §4.5, with these differences:

- `EDIT_HINT` (line 113) changes as in templates.
- `Ctrl+G`: `busy` → `Notice::Error(in_flight(busy))`.
- The project is `ctx.projects.first()` (P7). `None` → `Notice::Error(NO_PROJECT)` with
  `NO_PROJECT = "no project in this workspace \u{2014} agent help records its run in one"`.
- The target is `HelpTarget::Skill { name: editor.target.name().to_owned() }`. An empty body is allowed: a new skill
  starts blank, which is the PRD's case.
- `Accept(text)` replaces the area as library's `on_external_edit` `Edited` arm does, with no parse (skills have
  none), and sets `Notice::Info(ACCEPTED)`. The gate stays `save_editor` (`skill_body_refusal`, head-identical).
- `render_editor` draws over `area` through `help.render`. `on_reply`, `on_paste` and the hint change as in templates.
  `on_reply` checks the help before the attach-pane arms. `Mode::Editing` and the attach pane never coexist.

Tests:

| Test | Asserts |
|---|---|
| LV-1 `ctrl_g_without_a_project_is_refused` (Bench, `projects: &[]`) | notice `NO_PROJECT`; nothing sent |
| LV-2 `ctrl_g_sends_edit_help_for_the_skill_in_the_first_project` (a `Ctx` over one `ProjectRef`, as `a_wide_project_slug...` builds one) | `EditHelp { project_id, prompt.target: Skill { name } }` |
| LI-1 (`tests/skills.rs`) `a_proposal_accepted_saves_as_the_next_version` | snapshot **`skills__agent_help_proposal`** |
| LI-2 (`tests/skills.rs`) `a_blank_proposal_still_meets_the_save_gate` | block `"```\n \n```"` → accept → `ctrl+s` → the blank-body refusal; no `SaveSkillVersion` |

### 5.2 T6 docs

`HANDOFF.md`:
- Append to MOD-67's entry (line ~377): "MOD-55 added `Ctrl+G` (ask an agent) to the Templates and Library editors,
  hard-coded beside `Ctrl+S`/`Ctrl+E` (MOD-55 plan P10); milestone 4 (Skills) registers it as a named action."
- MOD-55's own phase/close-out lines as the workflow requires.
- The write-up names `run_step.phase_name = 'edit_help'`. The `0001_init.sql:478` comment is not edited, because
  migrations are immutable.

Validate: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

### 5.3 Snapshots

Existing files that **will** change: the hint row only, `Ctrl+G ask agent` inserted. These are all six that draw
`EDIT_HINT`:

| Snapshot | Row |
|---|---|
| `crates/htui/tests/snapshots/skills__changed_elsewhere.snap` | hint `L1:C2` |
| `skills__edit.snap` | hint `L1:C2` |
| `templates__changed_elsewhere.snap` | hint `L1:C7` |
| `templates__edit_help.snap` | hint `L1:C1` (the placeholder column must be byte-identical: §1.3 table switch) |
| `templates__missing_item_confirm.snap` | hint |
| `templates__unknown_placeholder_cursor.snap` | hint `L3:C1` |

New: `templates__agent_help_asking`, `templates__agent_help_proposal`, `skills__agent_help_proposal`. No
`crates/htui/src/snapshots` file shows a Skills editor. Treat this list as a lower bound (memory: snapshot impact needs a
full insta run):

```bash
cargo insta test -p htui --all-features --review   # every changed .snap reviewed; never --accept blind
git diff --stat -- crates/htui/tests/snapshots      # the six above plus the three new, nothing else
```

### 5.4 Full gate (after T5)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod55-gate.log
grep -E "SIGABRT|overflowed|FAILED|panicked" /tmp/mod55-gate.log
SQLX_OFFLINE=true cargo check --workspace --all-features --all-targets
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## 6. Hazards

- **H-1 Double report (accepted, plan).** `App::on_reply` turns every fresh `Failed` into
  `Action::Error("{request}: {message}")`. A scrub refusal reads `edit_help: not sent: the body matches the
  github_token rule` on the status line, and the same sentence appears in the view's notice. One new noise case: `Esc`
  in the instant a help ends on its own can draw `chat_cancel: this chat has ended`. The component treats it as
  terminal while `Cancelling`.
- **H-2 Frames reach both views.** `SkillsTab::on_reply` forwards to both, but only the view whose open editor holds
  `help: Some` reacts. Two helps cannot coexist (A-1). The Chat tab never sees these frames, because the origin is
  `Tab(skills)`.
- **H-3 Stale frames.** A new `EditHelp` makes the previous one's stream stale (same discriminant, same origin). A
  previous `ChatCancel`'s `Ended` stays fresh until the next `ChatCancel`, and that is why `Starting` ignores
  `Chat(Ended)` and `Cancelling` never abandons (A-2). The ordering argument: in help mode a cancel is only ever
  answered from `run_turn` (mid-turn), and the reply precedes the stream's own `Ended`, so the help closes on the first
  of them.
- **H-4 `R-ID-7` scope.** Help scrubs before send. With `spec.env` empty until MOD-10, that means pattern refusal plus
  masking of nothing. The chat path's unscrubbed send is pre-existing and stays out of scope; offer a new item at
  close-out. Also: if MOD-10 fills `env`, a masked value can make a recorded row "unreadable", and that row reaches the
  UI with no frame (`record.rs` ~915). Help would then end with an incomplete reply and no `Failed`. Revisit when
  MOD-10 lands.
- **H-5 Parked permission.** `option_for(RejectOnce)` falls back to `reject_always`. An agent offering **no** reject
  option makes `evaluate` return `None` and the request parks. A help panel has no answer UI, so the user's only way
  out is `Esc`, which works (mid-turn `Cancel` answers parked requests `Cancelled`). No adapter in the registry does
  this today.
- **H-6 Residual tool surface on `claude-cli`.** `--disallowedTools` does not cover `Task`, `TodoWrite`,
  `ExitPlanMode` or an operator's own MCP servers (`--strict-mcp-config` is not passed). The row's
  `--permission-mode` and `extra_args` still apply. On ACP, an agent's native tools that never ask for permission are
  not gated by htui. The prompt it acts on is already scrubbed. Naming dialect tools in `agent_worker` would break
  `R-AGT-5`'s spirit, so this is documented, not fixed.
- **H-7 Cap and quota (B-4).** A breach closes the run `failed` with no `frames.failed` (D73) and `Ended { Cancelled }`.
  The component proposes nothing.
- **H-8 Re-probe (B-3).** On an ACP row a stale help start spawns the staleness re-probe in the background, exactly as
  a chat does.
- **H-9 Scope change mid-help.** `on_scope_change` resets the view and drops the help. The session finishes its one
  turn and records a `done` run nobody reads. The frames still pass `is_fresh` and are ignored, because no help is
  open.
- **H-10 Stack.** `run_chat` is boxed (`Served::Start` holds `Box::pin`), and `start` grows by one match. No engine
  path changes (`htui-orch` untouched). Gate with `--no-fail-fast` and grep for `SIGABRT`/`overflowed` anyway
  (`runs_pg.rs` was the last victim).
- **H-11 `StoreRequest` secrecy rule.** `HelpPrompt` carries a body that may hold a secret. Its `Debug` is lengths
  only (E-11, W-2). Never derive it.
- **H-12 Featureless clippy.** `cargo clippy --workspace -- -D warnings` without features: every new item has a non-test
  caller by T5. T1/T2 items are `pub` in a library crate, so there is no `dead_code` between waves. Do not add
  `#[allow(dead_code)]`.
- **H-13 Lints and conventions.** `missing_debug_implementations` is on: `Opening`, `ChatMode` and `State` derive or
  implement `Debug`, and `AgentHelp`/`HelpPrompt` implement it by hand. Every item, field and variant gets `///`.
  `unused_qualifications`: import, do not path-qualify twice.
- **H-14 Parallel wave coupling.** T1 and T2 share no file (`prompt/*`, `scrub.rs` vs `model/run.rs`, `store/*`,
  `pg/write.rs`, `.sqlx`). `htui-core` is rebuilt by both, so gate the merged tree, not each lane alone.
