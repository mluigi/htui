# Agent personas

A persona is a named posture for a step: a role text that opens the step's prompt, plus a
narrowing of the tools and permissions the step's agent already has. Examples are a reviewer that
reads and runs tests but never edits, or an architect that designs without touching the tree. A
persona belongs to a step-graph phase, not to an agent, so the same agent can serve a plain
`implement` phase and a read-only `review` phase without its row changing.

This page describes MOD-26. Personas are registry rows, which Settings › Personas lists, edits,
deletes and imports. A phase names one in Settings › Kinds, and the engine applies it to every step
of that phase. The limits are listed under [What MOD-26 does not do](#what-mod-26-does-not-do).

- [What a persona is](#what-a-persona-is)
- [The two seeds](#the-two-seeds)
- [Binding a persona to a phase](#binding-a-persona-to-a-phase)
- [Settings › Personas](#settings--personas)
- [Rule lines](#rule-lines)
- [Deleting a persona](#deleting-a-persona)
- [Importing persona files](#importing-persona-files)
- [The persona file format](#the-persona-file-format)
- [Narrow-only: what a persona can and cannot change](#narrow-only-what-a-persona-can-and-cannot-change)
- [Precedence](#precedence)
- [Where the body goes in the prompt](#where-the-body-goes-in-the-prompt)
- [Freezing: a started run keeps its personas](#freezing-a-started-run-keeps-its-personas)
- [Enforcement, per transport](#enforcement-per-transport)
- [What a step records](#what-a-step-records)
- [What MOD-26 does not do](#what-mod-26-does-not-do)

## What a persona is

A persona is one row of the global `persona` table (`0012_persona.sql`), like a skill: it belongs
to no project and its name is unique. A row holds:

- `name`: 1-64 characters of `a-z`, `0-9` and single inner hyphens.
- `description`: a one-line summary for a picker. It is never rendered into a prompt.
- `body`: the role text, rendered at the top of the step's prompt. It may not be blank.
- `tools`: `{allow, deny, deny_kinds, command_run}`, what the persona takes away from the agent's
  tool exposure (see [Narrow-only](#narrow-only-what-a-persona-can-and-cannot-change)).
- `permission`: `{default, rules}`, reject-only additions to the agent's permission policy.

A persona never names, chooses or influences a model. The step runs on the model of the phase
candidate that wins it, exactly as it would without a persona, so quota fallback across
candidates (`R-AGT-8`) keeps working. A file that tries to set one is refused (see
[the file format](#the-persona-file-format)).

Postgres is the only source of truth (`R-ID-3`). htui reads no persona file while it runs: the two
seed files are compiled into the binary and only ever create rows.

Every persona shape refuses an unknown key, both in the database JSON (`tools`, `permission`) and
in a file. The writers check every row before it is stored, so a persona that would widen
anything cannot be saved (see [Narrow-only](#narrow-only-what-a-persona-can-and-cannot-change)).

## The two seeds

htui seeds two personas, `reviewer` and `architect`, from
`crates/htui-core/seeds/persona_reviewer.md` and `persona_architect.md`. Both:

- deny the tool kinds `edit`, `delete` and `move`;
- keep `execute`, so a reviewer can run the test suite;
- carry no `allow` list, no `deny` list, no `permission-default` and keep `command-run`.

The seeds have no `allow` list because the `.claude/agents` files they mirror list Gortex MCP tools
that htui does not provide. Because `execute` stays, the shell remains a way to write files under
either seed (see [Enforcement](#enforcement-per-transport)).

Postgres gets the seeds each time htui (the TUI or `htui worker`) connects to a migrated database,
keyed by name with `ON CONFLICT (name) DO NOTHING`: a seed the table lacks is added, and an
operator's edit to a seeded row survives every later connect.

Because the top-up is keyed by name, **renaming a seed brings it back**: rename `reviewer` (or
`architect`) and the next connect finds no row called `reviewer`, so it creates a fresh one, with
a new id, beside your renamed row. **Deleting a seed brings it back too**, for the same reason. To
retire a seed's behaviour, edit the `reviewer` row rather than renaming or deleting it.

## Binding a persona to a phase

A `step_graph_phase` names at most one persona through its nullable `persona_id` column. The
binding applies to whichever candidate wins the phase, on any rung; per-candidate binding is not
part of MOD-26.

**Settings › Kinds.** `e` on a phase opens its edit form. The seventh and last field is `persona`,
prefilled with the bound persona's name, or blank when there is none.

- Type a persona's name to bind it, or clear the field to unbind.
- A field left as it opened writes nothing, so a binding another writer made since stays.
- A name the registry does not hold is refused before anything is sent, with the known names:
  "no persona named \`<name>\`; known: architect, reviewer". With no personas at all the refusal
  is "no personas exist; add one in Settings › Personas".
- A bound phase's line in the list ends with `· persona <name>`. A phase without a persona draws
  as before.

A new phase (`n`) has no `persona` field, so bind it with `e` once it exists. Kinds reads the
persona list with its catalogue. A persona created in Settings › Personas shows up in Kinds after
Kinds reloads (`r`).

The store API and SQL remain for scripts.

**Store API.** `WriteStore::update_phase(id, expected, PhasePatch { persona: Some(Some(persona_id)),
..PhasePatch::default() })` binds, `Some(None)` clears and `None` leaves the binding alone.
`create_phase` accepts a `StepGraphPhase` whose `persona_id` is set. A `persona_id` that names no
row is refused with "step_graph_phase.persona_id \`<id>\` references no persona".
`WriteStore::personas()` lists the rows by name. `create_persona` adds one, `update_persona` edits
one under compare-and-set on `updated_at`, and `delete_persona` deletes one (see
[Deleting a persona](#deleting-a-persona)).

**SQL.** Against your Postgres, for the `review` phase of one project's step graph:

```sql
UPDATE step_graph_phase
   SET persona_id = (SELECT id FROM persona WHERE name = 'reviewer')
 WHERE name = 'review'
   AND graph_id = (SELECT id FROM step_graph
                    WHERE name = '<graph>'
                      AND project_id = (SELECT id FROM project WHERE slug = '<project>'));
```

Set `persona_id = NULL` to remove the binding. The foreign key `fk_step_graph_phase_persona` is
`ON DELETE RESTRICT`, so a persona bound to any phase cannot be deleted. The trigger moves the
phase's `updated_at`, so an editor that still holds the old row sees its next save of that phase
refused as stale. Use SQL to bind and unbind only: an `INSERT` or `UPDATE` on `persona` itself
bypasses the save rules the store applies, which are the only thing keeping a row narrow-only.

Bind before you start the run. A run started earlier keeps the bindings it froze (see
[Freezing](#freezing-a-started-run-keeps-its-personas)).

## Settings › Personas

**Personas** is the eighth Settings section, after Boxes. It lists every persona row by name, one
line each:

```
reviewer · Reviews the step's inputs and code for correctness, ri… · deny edit,delete,move · allow 0 · rules 0
```

The description is cut with `…` to fit the line; the name and the counts never are. The row under the cursor has a second line with
its lists: `allow <names, or all> · disallowed <names, or none> · command-run y|n · default
inherit|ask|deny`.

| Key | What it does |
|---|---|
| `j` / `k` (`↓` / `↑`) | Move the cursor. |
| `n` | New persona: the fields, then the body. |
| `e` | Edit the selected persona's fields. |
| `b` | Edit its body. |
| `r` | Edit its permission rules (see [Rule lines](#rule-lines)). |
| `d` | Delete it, after a `y`/`n` question (see [Deleting a persona](#deleting-a-persona)). |
| `I` | Import a persona file or a directory of them (see [Importing persona files](#importing-persona-files)). |

The section reads its list when the Settings tab asks for it, and it has no reload key. Personas are
not mirrored to the local copy, so offline the section shows "personas unavailable:" with the
reason, and offers no key but navigation.

**The fields.** `n` and `e` open seven one-line fields, labelled with the
[frontmatter](#the-persona-file-format) spellings: `name`, `description`, `tools`,
`disallowed-tools`, `deny-kinds`, `command-run (y/n)` and `permission-default`. `Tab`/`Shift+Tab`
(or `↓`/`↑`) move between them, and `Esc` closes the form.

- The three lists are comma-separated, as in a file (`Read, Grep, Glob`).
- `command-run (y/n)` takes `y`, `yes`, `n` or `no` ("\`command-run (y/n)\` is y or n").
- `permission-default` is blank (the agent's default stands), `ask` or `deny` ("\`permission-default\`
  is blank, ask or deny").
- Every field is checked with the store's own rule, so a refusal is the store's sentence (see
  [The save rules](#the-save-rules)). The cursor moves to the field it names.

**New (`n`).** The fields start blank, with `command-run` at `y`. `Enter` moves on to the body
editor, because a persona needs a body. `Ctrl+S` there creates the row, and `Esc` goes back to the
fields with the body kept. A new persona has no rules: add them with `r` once it exists.

**Edit (`e`).** The fields open prefilled, and `Enter` saves only the fields you changed. A form that
changed nothing writes nothing ("nothing changed; nothing was written").

**Body (`b`) and rules (`r`).** Each opens a multi-line editor. `Enter` breaks the line, `Ctrl+S`
saves, and `Esc` closes. `Esc` over unsaved text warns first ("unsaved changes — Esc again
discards"), and a second `Esc` discards. A rules save keeps the row's `permission-default`.

**Saves are compare-and-set.** A save meets a row changed elsewhere since the editor opened like
this:

- In the fields form, the untouched fields take the new values and yours stay. The notice says
  "changed elsewhere — reloaded; Enter retries" and names any field changed on both sides.
- In the body and rules editors, an untouched text is reloaded ("changed elsewhere since you opened
  it — reloaded; Ctrl+S retries against the current row").
- A row deleted elsewhere closes its editor.
- Only one write runs at a time. A second write key is refused with "\`<request>\` is still in
  flight".

## Rule lines

The rules editor (`r`) holds one rule per line:

```
<answer> <key>=<value> ... [# <reason>]
```

- `answer` is `reject_once` or `reject_always`.
- The keys are `kind`, `name`, `path` and `command`. They match the tool kind, the tool name, a
  path prefix and a command prefix. Each key may appear once. An absent key does not constrain the
  match, and `key=""` matches the empty string.
- `kind` is one of the ten ACP kinds: `read`, `edit`, `delete`, `move`, `search`, `execute`,
  `think`, `fetch`, `switch_mode`, `other`. Unlike `deny-kinds`, the last three are allowed here.
- A value is a bare word or a `"`-quoted string. A value holding a space, `#`, `"`, `=`, `\` or a
  control character must be quoted. Inside quotes, `\"`, `\\`, `\n`, `\t` and `\r` are the only
  escapes, and every other character is literal.
- The reason runs from the first unquoted `#` to the end of the line, trimmed. A reason that needs
  leading or trailing spaces, or a control character, is written quoted (`# "  …  "`), and nothing
  may follow it.
- Blank lines and lines starting with `#` are ignored. They are not stored: the editor reopens on
  the stored rules, one line each in their shortest form.

```
reject_once kind=execute command="rm -rf" # never wipe
reject_always kind=edit path=src/generated/
reject_once name=WebFetch
```

A line that cannot be read is refused with its number, for example "rules line 2: \`exec\` is not
\`key=value\`", "rules line 1: a rule starts with reject_once or reject_always, not \`deny\`" or
"rules line 3: persona.permission.rules entry kind \`exec\` is not one of read, edit, delete, move,
search, execute, think, fetch, switch_mode, other". The parsed rules then go through the store's
save rules, so a rule with no key at all is refused as an empty match.

## Deleting a persona

`d` asks "delete persona \`<name>\`? a persona bound to a phase is refused.". `y` deletes it, and
`n` or `Esc` leaves it alone.

The store refuses to delete a persona that any phase still names, and the refusal names the
phases as `<project>/<graph>/<phase>`, sorted:

"persona \`reviewer\` is bound to 2 phases (\`web/default/review\`, \`web/FEAT-12-override/review\`);
clear them in Settings › Kinds first"

At most five phases are named, then "and *n* more". An item's override graph copies its source
graph's bindings, so its phases count too. Clear each binding in Settings › Kinds, then delete.
Both stores refuse the same way, and on Postgres a bind racing the delete either lands first (and
the delete is refused) or is refused itself ("references no persona"). Migration
`0013_persona_phase_index.sql` indexes `step_graph_phase.persona_id`, so the check reads only the
phases that name the persona.

Deleting a persona never breaks a started run, because the run reads its personas from its
snapshot (see [Freezing](#freezing-a-started-run-keeps-its-personas)). Deleting a seed brings it
back on the next connect (see [The two seeds](#the-two-seeds)).

## Importing persona files

`I` opens a path field ("type a path to a persona \`.md\` file or a directory of them"). `Enter`
imports and `Esc` cancels. The store worker reads the files, not the screen. Each file becomes one
new row, and htui stores no path. The row is the truth from then on: editing or deleting the file
changes nothing.

- **The path** is a file or a directory. A leading `~/`, or a bare `~`, is your home directory;
  `~user` and a `~` later in the path are taken as typed. A file named directly is read whatever
  its extension.
- **A directory** contributes its own `*.md` files (no subdirectories), sorted by file name, links
  to files followed. Other files and subdirectories are left alone and not reported. So
  `~/.claude/agents` or a repository's `.claude/agents` imports in one go.
- **No frontmatter.** A file that does not open with a `---` fence (a `README.md`) is skipped: "no
  frontmatter: the file does not open with a \`---\` fence".
- **The format** is [the persona file format](#the-persona-file-format), with one difference:
  `tools` entries starting with `mcp__` are dropped from `allow` and named in the report ("dropped
  from \`tools\`: \`mcp__gortex__search\`, … — \`allow\` keeps built-in tools only"). `--tools`
  never filtered MCP tools, so htui cannot enforce those entries. To keep an MCP tool from a
  persona, name it in `disallowed-tools`.
- **Only MCP tools.** A file whose `tools` lists only `mcp__` entries is refused, because an empty
  `allow` keeps every built-in tool: "every \`tools\` entry is an MCP tool; htui's \`allow\` keeps
  built-in tools only, so this file would keep all of them — write \`disallowed-tools\` or
  \`deny-kinds\` instead".
- **Other refusals.** A file the format refuses (a `model:` line, an unknown key, a widening
  shape) is refused with that sentence, and nothing is written for it.
- **Known names.** A name that already exists is refused rather than overwritten: "persona
  \`reviewer\` exists; edit it in Settings › Personas, or delete it and import again". A name met
  twice in one import is refused the second time, naming the first file.
- **Caps.** A file over 262144 bytes (256 KiB), one that is not UTF-8, or one that cannot be read is
  skipped. A directory contributes at most 64 files, and only its first 256 entries are examined.
  Each overflow is one skipped row, never a silent cut.
- **A lost store** stops the import: the files after it are one row, "\<n\> more file(s) not
  attempted: the store is unreachable", and what was already imported stays.
- **Offline**, the import is refused before any file is read.

A clean import says "imported \<names\>" in the notice, and a path with nothing to import says
"nothing to import at that path". Anything refused, skipped or dropped opens a **report**. It starts
with a count line (`imported n · refused n · skipped n`). Then it has one row per file: `+ name
(path)` for an import, with any dropped tools; `! path — sentence` for a refusal; and `· path —
reason` for a skip. `j`/`k` scroll it and `Esc` or `Enter` closes it. If you opened an editor while
the import ran, the notice gives the counts and the report opens when the editor closes.

Every real `.claude/agents` file lists Gortex MCP tools, so importing one always opens the report.
Permission rules cannot be written in a file. Add them with `r` after the import.

## The persona file format

A persona file is the `.claude/agents/*.md` shape: a frontmatter block between two `---` lines,
then the body. The seeds are written this way, and the import reads the same format, except for its
`mcp__` rule (see [Importing persona files](#importing-persona-files)).

```markdown
---
name: reviewer
description: Reviews the step's inputs and code without editing files.
deny-kinds: edit, delete, move
---

You are the reviewer for this step. ...
```

The grammar is deliberately small:

- The first line is `---`; a closing `---` must follow within 200 lines.
- Between the fences, each line is one `key: value`. A value may be quoted (`"a, b: c"`) and is
  still one line.
- A list value is comma-separated on that one line (`tools: Read, Grep, Glob`). Each item is
  trimmed and empty items are dropped, so `tools:` alone is the empty list.
- A flow list (`[Read]`), a block list (`- Read` lines), a block scalar (`|`, `>`, `|-`, `>-`) or a
  nested map is refused: "persona key \`<key>\` takes one \`key: value\` line; write a list as
  \`a, b, c\`".
- A key written twice is refused: "persona key \`<key>\` appears more than once".
- Anything else the frontmatter reader cannot read is refused with its line: "persona frontmatter
  line <n>: <message>".
- The body is everything after the closing fence, with at most one leading blank line removed.
  CRLF is read as LF and the body ends with exactly one newline.

The seven keys:

| Key | Row field | Value |
|---|---|---|
| `name` | `name` | Required. |
| `description` | `description` | Optional; empty when absent. |
| `tools` | `tools.allow` | Built-in tool names to keep. |
| `disallowed-tools` | `tools.deny` | Tool names to remove, MCP tools included. |
| `deny-kinds` | `tools.deny_kinds` | Any of `read`, `edit`, `delete`, `move`, `search`, `execute`, `fetch`. |
| `command-run` | `tools.command_run` | `true` (the default) or `false`; anything else: "\`command-run\` is \`true\` or \`false\`, not \`<value>\`". |
| `permission-default` | `permission.default` | `ask` or `deny`; anything else: "\`permission-default\` is \`ask\` or \`deny\`, not \`<value>\`". |

`model` is refused with its own sentence: "a persona does not set the model; the phase candidate's
model is used (MOD-26)". Any other key, such as `color`, is refused by name: "\`color\` is not a
persona key; a persona file takes name, description, tools, disallowed-tools, deny-kinds,
command-run and permission-default". A file without `name` is refused: "a persona file needs a
\`name\`".

Permission rules cannot be written in a file. Only the row's `permission.rules` holds them, written
by the rules editor in Settings › Personas (see [Rule lines](#rule-lines)) or by the store API.

A parsed file then goes through the same save rules as a row, so a file the store would refuse is
refused when it is read.

### The save rules

The store checks every write against these rules, in this order, and refuses the first one that
fails:

- A bad name: "persona.name \`<name>\` must be 1-64 of a-z, 0-9 and single inner hyphens".
- A NUL in the description or the body: "persona.description must not contain a NUL character",
  "persona.body must not contain a NUL character".
- A blank body: "a persona needs a prompt body".
- A tool name in `allow` or `deny` that is empty or holds whitespace, a comma or a NUL (each list
  becomes one comma-joined argument): "persona.tools.allow entry \`<name>\` is not a tool name: one
  or more characters, no whitespace, comma or NUL" (or `persona.tools.deny`).
- An `allow` entry starting with `mcp__`, because `--tools` filters built-in tools only:
  "persona.tools.allow entry \`<name>\` is an MCP tool; \`allow\` keeps built-in tools only, so deny
  an MCP tool by name instead".
- A `deny_kinds` entry outside the seven: "persona.tools.deny_kinds entry \`<kind>\` is not one of
  read, edit, delete, move, search, execute, fetch". `think`, `switch_mode` and `other` cannot be
  denied.
- A rule whose match is empty, which would deny every request: "a persona rule with an empty match
  would deny every request; use \`default: deny\` instead".
- A NUL in any rule matcher string or reason: "persona.permission.rules must not contain a NUL
  character".
- Once every rule has passed the two checks above, a rule whose `tool_kind` is not one of the ten
  ACP kinds: "persona.permission.rules entry kind \`<kind>\` is not one of read, edit, delete, move,
  search, execute, think, fetch, switch_mode, other".

`permission.default` can only be `ask` or `deny`, and a rule's answer can only be `reject_once` or
`reject_always`: the types have no `allow`. A rule's `tool_kind` is compared as text, so a misspelt
kind would match nothing. That is why both stores and the rules editor accept only the ten kinds.

## Narrow-only: what a persona can and cannot change

A persona never makes a step's tools or permissions looser than the agent's. The engine computes
the step's exposure and policy with one pure function, `htui_agent::persona::narrow(base, policy,
persona)`, and every clause of it only removes:

- **`allow`**: if the base list is empty (which means "every tool"), the persona's list is used. If
  the persona's list is empty, the base list is kept. If both are non-empty, the result is the
  intersection in base order, and every base name the persona's list omits is also added to
  `deny`. If that intersection is empty, the base list is kept and every name in it is now denied,
  so the step has no tool rather than every tool (an empty list would read as "everything").
- **`deny`**: the base's, then the persona's, then the names added above, each kept once.
- **`deny_kinds`**: the base's, then the persona's. A kind string htui cannot read is denied as
  `other`, never dropped (the save rules make that impossible to store).
- **`command_run`**: the base's AND the persona's, so a persona can only turn it off.
- **Rules**: the persona's own rules first (each rejects; an empty reason is recorded as
  `persona <name>`), then one rule per denied kind, `{match: {tool_kind: <kind>}, answer:
  reject_once, reason: "persona <name> denies <kind>"}`, then the agent's own rules.
- **Remembered choices**: the agent's, unchanged. They are evaluated after every rule, so an
  "always allow" the user once gave for `edit` loses to a persona that denies `edit`.
- **`default`**: the stricter of the two, in the order `allow` < `ask` < `deny`.

No transport receives a flag that approves or adds a tool. In particular htui never emits
`--allowedTools` for a persona, because that flag auto-approves the tools it names rather than
restricting the set.

## Precedence

For every key a persona carries:

**agent row → persona narrows → the step; the model comes from the phase candidate.**

The agent row supplies the base: its permission policy (`agent.settings.permission`: rules,
remembered choices, default) and its tool exposure. No agent row carries a tool exposure yet, so the base exposure is the empty one: no allow-list, no deny-list, no denied kind.
The persona then narrows that base with `narrow`. The model is never part of it: it is the winning
phase candidate's `model`, whatever the persona says.

This replaces ANA-27's "agent row, then persona, then the phase candidate's `model`", which put the
model rung after the persona; personas carry no model, so the order above has no model rung.

## Where the body goes in the prompt

The body is inlined into the step's prompt as an htui-owned section, never handed to the agent as a
file (`R-ID-5`, `R-PRM-1`). It renders first in the prompt text, before the phase template:

```
<section name="persona" persona="reviewer">
You are the reviewer for this step. ...
</section>

<the phase template, as before>
```

The persona's name is escaped as an attribute. In the prompt's `sections[]`, `template` stays
`sections[0]` and `persona` is `sections[1]`. The section is protected, like the template: no
token budget trims it. It is scrubbed like every other section, so a secret in the body is masked,
and a body that cannot be masked refuses the prompt. A template cannot place it: `{{persona}}` is
not one of the twenty placeholders, and a template that writes it is refused as an unknown
placeholder.

A persona with `command-run: false` also removes the prompt's `command_queue` section. That is the
only effect `command_run` has today: no transport reads `ToolExposure.command_run` until
MOD-11 exposes the `command_run` tool.

A prompt for a phase without a persona is byte-for-byte what it was before MOD-26.

## Freezing: a started run keeps its personas

At `StartRun`, htui freezes every persona the graph's phases name into the run's snapshot
(`run.graph_snapshot`): each phase records its persona's **name** (`phases[].persona`) and the
snapshot carries one frozen copy per name (`personas[]`, each `{name, digest, body, tools,
permission}`). From then on the run reads personas only from its snapshot.

- **Editing a persona row does not change a started run.** Its steps keep the body, tools and
  permissions frozen at `StartRun`. The next run picks up the edit.
- **Rebinding a phase does change the graph.** The phase's persona name is part of the graph's
  topology digest, so a run resumed after its phase was bound, unbound or rebound sees a topology
  mismatch and is parked, like any other graph change made under a run.
- **Renaming a persona changes the graph too.** The topology digest hashes the persona's name, not
  its id, so renaming a persona that is bound to a phase moves the topology of every graph bound
  to it: a run on such a graph that is resumed afterwards sees a topology mismatch and is parked.
  Rename only when no run on those graphs is waiting to resume. (Renaming a seed also brings the
  seed back on the next connect; see [The two seeds](#the-two-seeds).)

A snapshot without a persona key and a phase without a persona serialise exactly as before, so
persona-less runs keep their topology digests.

If a step's phase names a persona its snapshot does not carry, that step fails before it sends a
token and the item is blocked, with the reason "prompt refused at \`<phase>\`: persona \`<name>\`
is not in the run's snapshot, so the step does not run un-narrowed (MOD-26 I-4)". The run settles;
the step never runs un-narrowed.

## Enforcement, per transport

A persona narrows two things, and each transport can enforce a different part of it:

- **Tool names** (`allow`, `deny`) are agent-native: `Read`, `Bash`, `mcp__server__tool`. ACP
  carries no tool name on the wire, so names only mean something to `claude-cli`.
- **Tool kinds** (`deny_kinds`) are the ACP kinds every transport shares.

Among the seeded agents, `claude` and `agy` are ACP agents; only `claude-cli` uses the CLI
transport.

| Transport | Tool names | Denied kinds | Persona `rules` and `default` |
|---|---|---|---|
| CLI (`claude-cli`) | `allow` becomes `--tools=<a,b>`, which restricts the built-in tool set; it is omitted when `allow` is empty (`--tools=""` would disable every tool). `deny` becomes `--disallowedTools=<x,y>`. | Inverted to Claude tool names and appended to `--disallowedTools`: `read` → `Read`, `NotebookRead`; `edit` → `Edit`, `Write`, `MultiEdit`, `NotebookEdit`; `execute` → `Bash`, `BashOutput`, `KillShell`; `search` → `Glob`, `Grep`; `fetch` → `WebFetch`, `WebSearch`. Denying a name a given CLI release lacks is harmless. | No effect: the CLI transport has no permission channel. |
| ACP (`claude`, `agy`) | Kept on the row, not enforced. | The permission relay rejects a request whose tool call is of a denied kind (`reject_once`, reason `persona <name> denies <kind>`). htui's own file handlers refuse `fs/read_text_file` when `read` is denied and `fs/write_text_file` when `edit` is denied. | Applied by the relay, before the agent's own rules and remembered choices. |

Both CLI flags are single `=`-joined arguments, placed after htui's own flag pairs and before the
agent's `extra_args`. `--disallowedTools` lists `deny` first, then the inverted kinds, each name
once.

On ACP, a denied file request is refused before htui checks the path, reads the old text or
records an edit proposal: the file is not touched. The agent gets a JSON-RPC `invalid_params`
error, and the step records a `DriverEvent::Error` with code `tool_kind_denied` and the message
"the step's persona denies \`<kind>\`: <method> of \`<path>\` refused".

### What is not enforced (residuals)

These gaps are real; a persona narrows what htui can see, not everything an agent can do.

- **The shell.** A persona that keeps `execute`, as both seeds do, leaves the agent a shell, and a
  shell can write files whatever `edit` says.
- **ACP calls made without asking.** An ACP agent may run a tool without a permission request and
  outside htui's `fs/*` handlers. The relay never sees such a call, so it cannot reject it. Whether
  `claude-agent-acp` routes its file tools through htui's handlers is not verified.
- **A request with no prior tool call.** The relay matches a permission request to the tool call
  with the same id that the agent announced earlier. A request with no such call has no kind, so no
  kind rule or matcher rule matches it; it falls through to the remembered choices and the default.
- **A matched reject the agent does not offer.** If a rule matches but the agent offers no reject
  option, the relay does not guess: the request parks and asks the user.
- **`delete` and `move` on the CLI.** Claude has no tool of either kind, so denying them adds no
  name to `--disallowedTools`.
- **Persona `rules` and `default` on the CLI.** They have no effect there (no permission channel).
- **MCP tools on the CLI.** `--tools` filters built-in tools only, and `deny_kinds` inverts to
  built-in names only. An MCP tool is removed only by naming it in `disallowed-tools`.
- **Tool names on ACP.** `allow` and `deny` are stored but no ACP transport enforces them.
- **An operator's `extra_args`.** The agent's own `extra_args` come last on the CLI argv, so a flag
  an operator put there is read after the persona's. In particular, a `--disallowedTools=` in the
  agent row's `extra_args` replaces the persona's whole deny list, the inverted `deny_kinds`
  included, so it can loosen the persona (an `extra_args` list that omits `Edit` lets a
  `reviewer`-bound step edit). **Do not set `--disallowedTools` in the `extra_args` of an agent
  that runs a persona-bound phase**; put the names in the persona's `disallowed-tools` instead.

## What a step records

htui does not store a separate "layers in force" record per step: the layers are a fixed function
of the transport, given by the table above. A step's persona is traceable through what is already
recorded:

- the `persona` entry in the step's prompt `sections[]` (`run_step.trim_record`);
- the prompt digest (`run_step.prompt_digest`), which covers the persona block, so a different body
  is a different digest;
- the run snapshot's frozen copy, `{name, digest}` among the rest, where `digest` is `sha256:` over
  the canonical JSON of the persona's name, body, tools and permission (not its id or timestamps).

## What MOD-26 does not do

- **No `$EDITOR` for the body or the rules.** Both editors are the embedded multi-line editor. An
  external `$EDITOR` round-trip would come with or after MOD-13's milestone 4 (MOD-26 OQ-12).
- **No picker in Settings › Kinds.** A phase's persona is typed by name, and a new phase is bound
  through `e` after it is created.
- **`~` in the skill import.** The persona import expands a leading `~/`; the Skills tab's import
  still takes its path as typed.
- **Engine steps only.** Chat sessions and promoted steps run without a persona: a promoted step's
  handoff prompt opens without the persona block, and the chat tab never applies one. A fan-out
  judge step also runs without the phase's persona; its prompt replays the candidates' recorded
  prompts, persona block included, as the task they were given. The Backlog prompt preview shows
  no persona block.
- **No per-candidate binding.** A persona is bound to the phase, not to one of its candidates.
- **No export** of a persona back to a Markdown file.
