# UI redesign — findings so far (2026-10-15)

**Status: review in progress. Nothing here is a requirement yet.** `docs/REQUIREMENTS.md` and
`HANDOFF.md` are untouched on purpose; §10 lists the requirements to file once the maintainer
finishes reviewing the skeleton. This document exists so the findings survive a compacted or new
session.

## 0. How to resume

- **Branch:** `spike/ui-skeleton` (cut from `main` at `809f8d22`). Each review round is committed
  there (first: `c9bb1615`); `git log` on the branch lists them.
- **Skeleton:** `spikes/ui-skeleton/`, a standalone crate with its own `[workspace]` (the htui
  workspace never builds it). ~6.3k lines of Rust, ratatui 0.30.2 + rataflow 0.1, mock data only:
  no database, no agents, no model calls. It is a **throwaway**: what we keep is the decisions, not
  the code.
- **Run:** `cargo run --manifest-path spikes/ui-skeleton/Cargo.toml`. Layout, the about panel, the
  new-item modal size and the Backlog grouping persist in `spikes/ui-skeleton/.ui-skeleton.state`,
  written **as soon as any of them changes** (delete it to reset; it is gitignored). Until round 7
  it was only written on a clean quit, so closing the tmux pane lost every resize.
- **Headless check:** `./target/debug/htui-ui-skeleton --dump <tab 1-6> [preset 0-3] [WxH] [keys]`
  prints one frame as text. Keys replay from the list pane: `_` down, `^` up, `!` Enter, `~` Tab,
  `<` `>` left/right, any other char as itself (e.g. `'__!]]'` opens MOD-67's graph).
- **Interactive checks** were driven through tmux (`send-keys` + `capture-pane`) from bash scripts
  on a private server (`tmux -L uitest -f /dev/null`); zsh's profile in this environment breaks
  inline tmux functions. Wheel events can be faked with `send-keys -l $'\e[<65;X;YM'` (65 = down,
  64 = up). The maintainer tried the mouse in round 7: drags work, the wheel does not (§5, tmux).
- **Files:** `main.rs` (event loop, input, navigation), `app.rs` (state, tree, layout presets, back
  stack, persistence), `ui.rs` (header, about panel, pane layout, help), `views.rs` (lists, Backlog,
  Chat, Skills, Settings, activity, graph rendering), `wsview.rs` (Workspaces, Logs, compose
  generation), `flow.rs` (rataflow graphs), `modal.rs` (new item / new requirement, add component),
  `forms.rs` (leave, "new what?", workspace/project forms), `data.rs` (mock data), `theme.rs`.

## 1. What is wrong with the current UI (the starting audit)

From the 199 insta snapshots and `crates/htui/src/ui/`:

1. **Chrome repeated on every screen.** Status line, tab strip (` 1 Chat`), a bordered box
   repeating the tab name (`┌ Chat ─`), and two hint rows; the second hint row
   (`q quit · Tab next tab · …`) is identical everywhere.
2. **Hints list keys, they do not guide.** `j/k select · n new · e edit · m paths · t this box ·
   r probe · i install · a authenticate`. Empty states give no next step ("no agents registered").
3. **Forms render far from what they edit.** Secrets and Queue settings put the input at the bottom
   of the pane, ~20 blank rows below the field.
4. **Raw internals on screen.** `awaiting_approval`, `in_progress`, tool calls as `[…]`, a Chat
   permission split between the top of the transcript and the bottom.
5. **Vim-first modal keys with no on-screen cue** (`j/k`, `h/l`, `i compose`).
6. **No shared building blocks.** The theme is centralised (MOD-80), but every tab draws its own
   lists, forms, empty states and footers inside very large files (`backlog/mod.rs` 4.5k lines,
   `settings/agents.rs` 3k), so restyling one tab does not carry over.
7. **Proportions.** Backlog's item list takes ~55% of the width and squeezes the detail.
8. **Tabs all look the same**; nothing says which tab you are on at a glance.

## 2. Direction (maintainer's words, then what it became)

- **"btop mostly."** Panels with rounded borders, titles and key hints *in* the borders, no global
  hint rows, dense use of space.
- **"Like an IDE":** a narrow list on the left (the file tree), a wide detail on the right.
- **"A hint on what the tab we are in does, toggleable, for both power users and new users."**
- **Colours per item kind and per feature/tab**, and an **editable layout**.
- Asked for a runnable skeleton to try layouts before deciding anything → the spike.

## 3. Shell decisions (tried in the skeleton, liked so far)

### 3.1 Header (one row)

`─ ▣ Graphics ─ ¹workspaces ─ ²backlog ─ ³chat ─ ⁴skills ─ ⁵logs ─ ⁶settings ──── ⌫ back to … ─ ◐ 3 working ⚠ 1 waiting ─ +1 in background ◐ 1 ─ DESKTOP-HTUI ─ IDE ─ F1 about ? keys ─`

- The open **workspace** sits at the far left, clickable (goes to the Workspaces tab).
- **Tabs** with superscript numbers; the active tab is a filled block in its colour; tabs are
  clickable.
- Right side, in priority order, dropping the least useful parts when narrow: the **back chip**
  (§3.6), running/waiting counts for the open workspace, `+N in background ◐ n`, box name, layout
  preset name, `F1 about ? keys`.
- Transient messages ("flash") replace the right side for ~2.6 s, cut with `…` to fit; the back chip
  stays visible next to them.

### 3.2 Panes

- Rounded borders; title in the top border, **sub-tabs as chips** right-aligned in the top border
  (`[ body runs graph docs notes ]`), **key hints in the bottom border of the focused pane only**,
  trimmed to what fits.
- Focused pane: border in the tab's accent. Unfocused: faint.
- Selected row: background band plus a `▌` gutter marker in the accent.
- Long rows end with `…` instead of being chopped.
- Long content gets a **scrollbar** in the right border, and **scrolling stops at the end**
  (an earlier build scrolled forever — reported as a bug).
- Wrapped paragraph lines keep a one-column indent (pad the area, not each line).
- When the layout hides something, the pane says how to get it back (`F2 show list · p layouts`,
  `z unzoom`).

### 3.3 "About this tab" panel

- A bordered 2-line panel under the header: what the tab is for, then the 4–5 usual actions as key
  chips.
- `F1` toggles it; on at first launch; the choice persists.

### 3.4 Layout: presets plus editing (btop-style)

- `p` / `P` cycle presets per tab: **IDE** (list 24% | detail), **IDE + activity** (activity panel
  underneath), **three columns** (list | detail | activity), **focus** (detail only).
- `<` `>` (or drag the list's border) resize the list; `{` `}` (or drag) resize the activity panel.
- **Defaults stay as they are** (round 7): IDE with the list at 24%, Backlog with the activity panel.
  What matters is that a resize is **remembered across restarts**, written the moment it happens.
- `F2` / `F3` (also `Alt-1` / `Alt-2`) show or hide list / activity; `z` zooms the focused pane.
- Every tab keeps its own layout; all of it persists.
- The original idea of digits for panel toggles was dropped: digits stay tab switches.

### 3.5 Colour

**Decided (round 7): tab + kind colours.** Each tab has an accent (workspaces amber, backlog cyan,
chat green, skills violet, logs orange, settings blue) used on its strip entry and focused borders;
item keys are coloured by kind (ANA, MOD, CLEAN, TOOL, FIX, FEAT, VAL, R-). The two alternatives
tried (tab colours only, kind colours only) and the `c` key that cycled them are gone.

Status never relies on colour alone: glyph + word everywhere (`● done ◐ running ⚠ needs you ✗ failed
⊘ blocked ◌ queued ○ open ✓ closed`). Status words are human (`needs you`, not
`awaiting_approval`).

### 3.6 Navigation and keys

- **Arrows never leave a pane** (explicit request): in a list `←→` fold/unfold groups; in a detail
  `↑↓` scroll or move a row cursor; in a graph the arrows pick nodes.
- **Moving between panes:** `Tab` / `Shift-Tab`, `Ctrl ←→` (`Ctrl-h/l`), or a click. This changes
  current htui, where `Tab` switches tabs.
- **Tabs:** `1`–`6` or click; `w` jumps to Workspaces.
- **Sub-tabs:** `Shift ←→` everywhere (including inside graphs), `[` `]`, or click a chip. `[ ]`
  needs AltGr on the maintainer's Italian keyboard, which is why sub-tabs looked unreachable from a
  graph; `Shift ←→` was added for that.
- **Enter** opens / focuses the detail; **Esc** returns to the list.
- **History, like a browser** (round 7; it replaced "back until you move"): every jump (Enter in a
  graph, the activity panel, a requirement's item or linked requirement) is recorded, and since
  round 8 also **every change of tab, sub-tab or pane** (keys or clicks), like an IDE's back /
  forward. Moving a cursor inside a pane is not recorded, but going back from there remembers the
  cursor, so forward returns to it. `⌫ Backspace` or `Alt ←` goes back, `Alt →` forward; going
  somewhere new drops what was forward. 50 entries. The header shows `⌫ back to MOD-67 · body ·
  list` (the pane is named when it is not the detail) and `Alt→ forward` when there is somewhere to
  go.
- **Leaving:** `q` / `Ctrl-C` never just quit. A "leave htui" modal lists every live workspace with
  running / waiting counts: `Enter` leaves the window and **the work keeps running in the
  background**, `S` stops everything gracefully and leaves, `Esc` stays; a second `Ctrl-C` leaves.
  After leaving, the terminal prints e.g. "htui keeps running in the background: 4 run(s)".
- **Confirmations** (round 8): both workspace stops ask first (§4.1). Leaving already asks.
- `?` opens a two-column key reference; `/` filters the focused list.
- Single letters are global unless a pane claims them: graph letters (`L m d f r M + - 0`, `h j k l`
  pan) only act when a graph has focus; Backlog's `h` / `g` act everywhere in the Backlog except a
  focused graph.

## 4. Tabs

Tab set changed during the review: **workspaces · backlog · chat · toolbox · logs · settings**. The
Requirements tab is gone (absorbed by Workspaces); Logs is new; Skills became Toolbox (round 8).

### 4.1 Workspaces (new tab; replaces the `w` switcher overlay and Settings' workspace field)

- **List:** every saved workspace as a group, with its state; under it an `overview` entry and one
  entry per project. Selecting the workspace's own row shows its overview.
- **States:** `● open` (the one on screen), `◐ background · n running` (runs keep going),
  `◌ stopping…`, `○ stopped`.
- **Opening another workspace** (`o`) sends the current one to the background; **its running items
  keep running.** The header shows `+N in background`.
- **Stopping means stopping the running items** (round 9 correction: it never removes anything, and
  it works on the open workspace too, so a single workspace can be stopped). `s` once is
  **graceful**: each run stops **before its agent's next turn** when the agent can resume a saved
  session, otherwise **at once**; the overview says which ("TOOL-8 code · round 2 · claude: stops
  before its next turn; the conversation is saved and resumes" / "cannot pause between turns: stops
  now; the step runs again on resume"). It takes seconds, not a whole step. `s` twice stops every run
  **now, mid-turn**. **The conversation is always saved**, and everything is resumable: the overview
  lists the stopped runs ("⏸ TOOL-8 stopped between turns in code · round 2: resumes the
  conversation" / "cut mid-turn …: resumes the conversation, redoing that turn" / "stopped in gate:
  the step runs again"), and `r` resumes them all. A background workspace becomes `○ stopped`; the
  open one stays open with its runs stopped.
- **Both stops ask first** (round 8): `s` opens "stop the runs in Graphics?" listing each run, its
  step and agent, and what will happen to it; `s` on a stopping workspace asks again before stopping
  them mid-turn. `Enter` confirms, `Esc` cancels (a cancelled second stop leaves the graceful one
  going). "Nothing is running in Graphics" when there is nothing to stop.
- The panel's key hints follow the state (`o open · s stop`, `s force stop`, …). The earlier
  sentence "This is the open workspace. Opening another sends it to the background…" was removed at
  the maintainer's request.
- **Sub-tabs:** `overview · components · requirements`, for a workspace and for each project.
- **Overview (workspace):** state, path, counts; description; goals; projects (items, running,
  waiting, repo path); components.
- **Overview (project):** repo, description, item counts by status, its coordinator chat, the
  components it uses.
- **`N` = "new what?"** chooser: workspace · project · component · requirement (component directly on
  the components sub-tab, requirement directly on the requirements sub-tab).
- **New workspace form:** name, **folder** (defaults to `~/workspaces/<name>`, follows the name until
  edited), description; preview of what htui creates there:
  ```
  ~/workspaces/mobile/
    .htui/workspace.toml       name, description, goals, projects
    .htui/docker-compose.yml   the components, generated
    .htui/.env                 ports and URLs per project (no secrets)
    projects/                  projects cloned from a URL go here
  ```
  Existing repos stay where they are; the workspace only points at them. A new workspace starts
  stopped and gets a workspace coordinator chat.
- **New project form:** name, source (existing folder · git URL · new empty repo), location; it gets
  a project coordinator chat and uses the workspace's components by default.

#### Components

- **Workspace level:** a table (name, type, kind, image:tag, port, status, used by) and the
  **generated `docker-compose.yml`** shown underneath: ports exposed on localhost, passwords as
  `${NAME_PASSWORD}` "from Secrets", named volumes for databases, `SERVICES: s3,sqs` for emulators,
  one extra service per project that runs its own instance. `d` removes a component.
- **Start / stop one component** (round 8): `Space` on a row starts or stops it ("db stopped · its
  data stays in its volume"). htui does **not** start or stop the stack with the workspace
  (decided round 8). These are the workspace's local instances; a run starts its own compose
  wherever it executes (§4.2a).
- **Where the files live** (round 7 decision): the compose file goes in the **workspace folder**
  (`<workspace>/.htui/docker-compose.yml`). The `.env` (ports and URLs per project) goes to
  **Infisical when it is available** (Settings → Secrets provider = infisical; the skeleton shows
  "in Infisical (url), folder /graphics"), **otherwise next to the compose file**
  (`<workspace>/.htui/.env`). Passwords come from Secrets either way. The new-workspace preview
  says the same.
- **Project level:** for each workspace component, `Enter` cycles **the workspace's instance · its
  own instance (on a free port) · not used**; the selected row shows the env var the project gets
  (e.g. `DB_URL=postgres://localhost:5433/agy`). `N` adds a component only this project uses.
- **Add-component wizard:** type → kind → **version** → **services** (only for emulators; `Space`
  toggles) → name & port (picked free, editable) → compose preview with `+` on the new lines.
- **floci is a family of emulators, all included, no choice** (rounds 8–9). Checked against the
  floci-io GitHub org: one container per cloud, one port each, every service of that cloud always
  on, no auth tokens or feature gates, plus a web console. Adding floci writes five services:
  | service | image | port | serves |
  |---|---|---|---|
  | `<name>-aws` | `floci/floci` | 4566 | EC2, ECS, EKS, Lambda, RDS, S3, DynamoDB, SQS, SNS, IAM, KMS, Step Functions, CloudFormation… |
  | `<name>-az` | `floci/floci-az` | 4577 | Blob, Queue, Table, Cosmos DB, Functions, Key Vault, Event Hubs, Azure SQL, AKS… (mounts the docker socket: Functions run in containers) |
  | `<name>-gcp` | `floci/floci-gcp` | 4588 | Cloud Storage, Pub/Sub, Firestore, Datastore, Secret Manager, IAM, Managed Kafka |
  | `<name>-oci` | `floci/floci-oci` | 4599 | Identity, Object Storage, Queue, Streaming, Vault + KMS, Secrets, Functions |
  | `<name>-ui` | `floci/floci-ui` | 4500 | web console for all four |
  Tags: `latest` (stable releases on the 15th and the last day of each month) or `nightly`. A project
  with its own instance shifts all five ports together. The project's env gets one endpoint per
  cloud (`AWS_ENDPOINT_URL=http://localhost:4566`, …). The wizard skips the services step for floci;
  LocalStack, Azurite and the GCP emulators still let you pick.
- **Types:** database, cache, queue, **cloud emulator** (floci, LocalStack, ElasticMQ, Azurite, GCP
  emulators — explicit request: "something like floci to emulate s3, sqs and others"), object
  storage, search, custom image.
- **The catalogue is data, not code** (explicit request): seeded into the database at first start,
  updated on request (`U` in the wizard; Settings → Catalogue → "update now"; or ask the htui agent).
  The wizard shows "catalogue updated 3 days ago".
- **Mock values:** floci's images and ports were checked (round 9, floci-io on GitHub); the
  environment variable names per cloud other than `AWS_ENDPOINT_URL` are placeholders. Other images
  are real public ones, but their version lists are illustrative.

#### Requirements (moved here from their own tab)

- Per workspace (workspace-wide ones) and per project.
- **Nested:** a requirement can refine another (`R-2` → `R-2.1`, `R-2.2`); the list draws a tree
  with `├─ └─ │` connectors.
- **Adding one works like adding an item** (round 7): `N` on the requirements sub-tab (or "new
  what? → requirement") opens the same centred, resizable modal: *describe* (`↑↓` picks the scope:
  workspace-wide or a project, defaulting to where you are) → *rewrite* (an agent words it: `# Must`,
  `# How to check`, `# Open questions`) → *review*, or `Tab` to talk it through. Review shows what it
  **refines** (guessed, `p` cycles; a workspace requirement may only refine a workspace one, a
  project one its own project's or a workspace one). The old field form is gone.
- **Workspace requirements split into project requirements** (round 7): for a workspace-wide one the
  agent proposes one requirement per project it touches (`W-3` → `R-42 htui`, `R-43 agy`, each
  refining `W-3`); `↑↓ Space` keeps or drops each before `Enter` creates them all. In the list a
  workspace requirement shows **"split into ⇢ R-42 htui …" rows** and is "covered" through them; each
  project requirement shows **"↑ from W-3"**. Both are selectable; `Enter` jumps (with history). Ids:
  workspace ones `W-n`, project ones the workspace's shared `R-n` (§8 Q8 still open).
- **Each covering item is its own selectable row** (`↳ ● MOD-80 Theme roles…`); `↑↓` select,
  `Enter` jumps to the item (with back). Arriving from a graph puts the cursor on that requirement.
- **`v` toggles a graph view**: parent → child (`refines`) and requirement → covering items
  (`covered by`).

### 4.2 Backlog

- IDE shape: items grouped by project on the left (glyph, key in kind colour, title), detail on the
  right with a status line (status, project, priority, depends on) and sub-tabs
  `body · runs · graph · docs · notes`.
- **Runs** sub-tab: the run graph (§5.2) by default, `v` toggles a step table.
- **Graph** sub-tab: the item graph (§5.1).
- **`N` new item: a centred modal** (explicit request: "modal or new window in the middle"):
  1. *describe*: free text, project picker (`↑↓`); **the project defaults to the item on screen**
     (or the group under the cursor);
  2. *rewrite*: an LLM turns the text into a proper item (kind guessed from the words → key, title,
     Goal / Acceptance / Open questions) — mocked in the skeleton;
  3. *review*: `Enter` creates, `e` edits the description, `r` rewrites again, `t` talks it through.
  - **Talk it through** (`Tab`): the modal splits into a live draft and a chat with an agent that
    asks questions and folds the answers into the draft; `Ctrl-D` goes to review.
  - **Resizable:** `Ctrl ←→↑↓`, drag the border, `F11` maximise; the size persists.
- **List options as keys** (round 7; they were Settings → Backlog → Hierarchy): `h` shows / hides
  done and closed items (hidden by default; the list's last row says "5 done hidden · h shows"), `g`
  cycles the grouping **project → kind → status** (status groups in attention order: needs you,
  failed, running, blocked, queued, open, done, closed). The list title says "items by kind". Both
  persist. A jump to a hidden done item turns `h` on rather than failing.
- Empty workspace: "Nothing here yet. Press N to create the first one."
- A second status line: **"runs on docker · this box (docker)   workflow research fan-out (fast
  pick)   t changes"** (§4.2a).

### 4.2a Where an item runs, and how (round 8)

The maintainer: the user chooses, **per item**, where it is executed: locally, in docker (as the
`scripts/hr` sandboxes do), over ssh on another machine, in docker on another machine, or on another
box connected through a central htui server (only in the plans). **Wherever it runs, htui starts the
docker compose there and configures everything the item needs.**

- **Default target, by cascade** (round 9): Settings (default **this box**, i.e. local) → workspace →
  project → item; each level inherits unless it sets one. Workspace and project overviews show
  "items run on docker · this box (from workspace Graphics) · T changes" (`T` cycles inherit and
  every target). In `t`'s picker the first row is **"inherit · this box (from Settings)"**.
- **Work comes back by collecting** (round 9), like `scripts/hr collect`: on every target except this
  box the run graph ends with a `collect the branch` step that waits for you.
- **The htui agent sets targets up** (round 9): adding a target, installing and logging in the agents
  on a remote box, setting up a docker context; it asks for each login. Its Chat badge now reads
  "settings · toolbox · agents · targets · catalogue".
- **Targets** (Settings → Running → Targets lists them; "add a target"; a default for new items):
  | target | kind | what htui does there |
  |---|---|---|
  | this box | local | a git worktree on branch `<item>`; the item's components via compose here; agents run as you |
  | docker · this box | docker | a fresh clone in a container, a private compose project (own Postgres, cache… on free ports), the toolchain image, agent logins on a named volume; nothing leaves until collected (like `scripts/hr`) |
  | ssh · build-01 | remote ssh | clone into `~/htui-runs/<item>`, compose there, env from Infisical; the htui agent installs and logs in missing agents |
  | docker · build-01 | remote docker | the docker sandbox on build-01's daemon (docker context over ssh); the branch comes back when collected |
  | box · LAPTOP-2 | htui server | **planned** (MOD-47, phase 2): a box enrolled with the server claims the run; shown, not selectable |
- **`t` in the Backlog** opens "how MOD-67 runs": *where* on the left (each target with its state and
  what htui prepares there, including "components started there: db, vectors"), *workflow* on the
  right (§4.6). `Tab` switches side, `Enter` saves.
- **The run graph starts with htui's own two stages**: `prepare · docker · this box` (Enter says what
  it did there) and `pick workflow` (Enter says what the fast pick chose and why), then the
  workflow's steps (§5.2).
- Existing items it touches: MOD-43 (remote dispatch: a target box for a run), MOD-44 (container
  execution environment), MOD-47 (`htui server`, phase 2), TOOL-7 (`scripts/hr`). Remote ssh and
  remote docker as targets, and "compose wherever it runs", are not covered by any of them (§10).

### 4.3 Chat

- **Groups, in order:** **htui agent** · **workspace** · **projects** · **item runs** · **archived**
  (folded). Explicit requests: separate workspace and project coordinators; always show the
  workspace coordinator and one coordinator per project; add a global agent.
- **htui agent** (`✦`, shown in every workspace): changes htui's settings, adds toolbox entries and
  agents, sets up targets (installs and logs in agents on remote boxes), updates the component
  catalogue. Badge: "settings · toolbox · agents · targets · catalogue".
- **Coordinators** (`◆` workspace, `◇` project) always take messages.
- **Item runs** are **view only while the agent works** (`🔒 view only` composer: "claude is working
  on … on its own. This unlocks when it asks you something") and **editable only when they ask the
  user** (a question or a permission). Answering locks them again.
- **Permissions** are answered inline with `a` allow once · `A` always · `r` reject (current htui
  uses digits, which clash with tab switching).
- **Archived** transcripts of finished run steps are read only.
- Transcript: `▍you` / `▍claude` gutters, tool calls as `▸ read  path  ● 0.1s`, permission as a
  highlighted card with key chips; it sticks to the bottom.

### 4.4 Logs (new tab)

- htui's own log: sources on the left (`all`, ui, store, orchestrator, agents, mcp, worker, secrets)
  with line and warning counts; lines on the right with coloured levels.
- `v` minimum level, `F` follow on/off, scrolling up pauses following, `G` resumes; new lines stream
  in.

### 4.5 Settings

- Same sections minus the workspace picker. **Connection is renamed Postgres** (round 7). The
  Backlog → Hierarchy section is **gone**: its two options became Backlog keys (`h`, `g`, §4.2);
  Backlog → Kinds stays.
- Fields edit **inline on their row**; the help shows on the line under the selected field (fixes
  audit item 3). Toggles `● on` / `○ off`, choices `‹ value ›`, secrets `•••••••• stored`.
- New **Catalogue** section (source, last updated, "update now" action).
- New **Targets** section (round 8; default target, round 9) and **Workflows** section (round 9):
  *fast pick model* (default **claude · haiku**, chosen from the models the configured agents offer),
  *changes at run time* (**ask me** / automatic: may the agent change a run's workflow on its own, or
  must it ask through a permission in the item's chat), *default workflow* (auto, or one).

### 4.6 Toolbox (was Skills; round 8)

The maintainer: the tab must hold **every kind of tool a prompt item's execution can use**: skills,
personas, mods, **workflows**, and other things (to be settled by an analysis).

- **List groups** (round 9 added the last six): workflows · skills · personas · templates · mods
  (Claude Code mods, claude only) · MCP servers · agent help · **hooks** · **commands** ·
  **sub-agents** · **prompts** (the rewrite prompt, the fast-pick prompt) · **env presets** ·
  **tools** (integrations that bundle an MCP server, hooks, a CLI or a daemon: gortex, headroom,
  serena, graphify). **How to show all of these is an ANA** (round 9, §10).
- **Workflows are templates; runs are their actuation** (round 9 correction). The template has **one
  node per step and real loops**: `code → review`, and `review → code` "changes requested" back up,
  `review → gate` "approved" onwards. A run **unrolls** it: each loop taken becomes `code · round 1 →
  review · round 1 → code · round 2 …`, each fan-out one node per part (`research ×3` in the template,
  three parallel nodes in the run).
- **Template relations** (round 9; "possibly other relations?"): `next`, `approved`, **`changes
  requested`** (loop), **`fails`** (loop: a failed check sends it back, e.g. `gate → code`),
  **`retries`** (loop: the same kind of step again, up to a cap), **`asks you`** (waits for your
  go-ahead, e.g. `merge → code` in research fan-out), **`if`** (taken only when a condition holds, e.g.
  docs only). A fan-out is a step property (its parts), not a link.
- **Viewed and edited as a graph in the Toolbox** (round 9; the "used by" list is gone). The template
  is drawn with rataflow: loops leave and enter by the **sides** (review loops on the right, failure
  loops on the left) so they never hide under the chain. Editing keys on the graph: `a` adds a step
  after the selected one (form: name, agent, parts for a fan-out; it takes over the selected step's
  outgoing links), `e` / `Enter` edits it, `x` deletes it (asks first; what led into it now leads to
  what came after), `c` connects: arrows pick the target, `Enter` opens the relation picker (a loop
  can point back), `i` lists the step's links (`← →` changes a relation, `d` deletes one), `M` / drag
  moves a node. Runs already going keep the version they started with.
- **Every run starts with a fast pick**: a light model (Settings → Workflows, default claude · haiku)
  reads the item and picks the workflow; the run graph shows it as the `pick workflow` step. The user
  can **pin** one per item instead (`t` → workflow; "auto" is the default).
- **The model may change the workflow at run time**; Settings → Workflows says whether it does so on
  its own or asks first (default: ask me).
- Workflows are **global** for now. **Project workflows** (one that drives a whole project's
  implementation, coordinating its items automatically) are wanted **later** (§8).
- Existing items it touches: **MOD-113** (template library with its own Templates tab, skill levels,
  resources, import of skill folders and plugins), **ANA-33** (step graphs v2: graphs planned at run
  time, division of work), **ANA-30** (rejected a separate `utility_model` setting, which the fast
  pick needs in some form), **ANA-31** (agy with its own skills off). **No item covers a toolbox of
  all kinds, the per-item workflow pick, or mods as a run resource**: an ANA is needed (§10).

### 4.7 Activity panel (in the IDE + activity / three-column presets)

The maintainer asked "what features does it have?" — the first version had none. Now:
- **Newest first, on top** (explicit request). "Needs you" events are pinned above the rest, also
  newest first, with a count in the panel title.
- `Enter` (or click + Enter) **jumps to the source** item or chat (with back); across workspaces it
  says which workspace to open.
- `f` filter: all workspaces · needs you · failures · this workspace.
- Live: new events stream in, background workspaces included.

## 5. Graphs (rataflow)

htui already uses rataflow for the Runs pane (MOD-28; MOD-71/72/74 for mouse and chips) and has
MOD-88 open for a flow-graph mode of the item graph. The skeleton uses rataflow 0.1 for three
graphs. Shared behaviour:

- Opens at **zoom 1, centred on the node that matters and its direct neighbours** (fitting the whole
  graph shrank boxes until their text disappeared).
- **Keys:** arrows pick a node (rataflow's spatial navigation), `h j k l` pan, `+ - 0` zoom / reset,
  `f` fit (never above zoom 1), `Enter` opens (below), `M` moves the selected node, `r` lays the
  graph out again.
- **Dotted background** on every graph (round 7): rataflow's `Background` with `Dots`, gap 6×3, in
  the faint border colour; it moves with pan and zoom, so the canvas reads as a surface.
- **Moving nodes without touching links** (round 7): drag a node with the mouse, or `M` then arrows
  (`Enter` / `Esc` places it). Links follow the node; nothing can create, reconnect or delete a link
  (nodes `connectable(false)`, `deletable(false)`, flow `with_edges_reconnectable(false)`; no keys are
  forwarded to rataflow's own bindings). Positions live as long as the graph is cached: `r`, or
  changing `L` / `d`, lays it out again; not saving them is fine (decided round 8).
- **tmux and the wheel** (round 9: do both): (a) the docs explain how to let the wheel through tmux
  (the mighty-scroll fix below, and tmux's own `mouse on`); (b) inside tmux (`$TMUX` set) a graph shows
  a small box "wheel not zooming? tmux may be keeping it: see docs › tmux · + - zoom from the
  keyboard" until a wheel event reaches htui once or the user zooms with `+` / `-`; both are
  remembered, so it never comes back.
- **Mouse:** drag pans (empty canvas) or moves (a node), wheel zooms. The maintainer reported in
  round 7 that **the wheel does nothing**, while drags work. Cause, found in their tmux config: the
  **tmux-mighty-scroll** plugin binds `WheelUpPane`/`WheelDownPane` and only passes the wheel to the
  processes listed in `@mighty-scroll-pass-through` (default `vim nvim`); everything else in the
  alternate screen gets copy-mode on wheel-up and nothing on wheel-down. It matches
  `/proc/<pid>/comm`, which **Linux truncates to 15 characters**: the skeleton is `htui-ui-skeleto`.
  Fix on their side: `set -g @mighty-scroll-pass-through 'vim nvim htui htui-ui-skeleto'` (or
  `@mighty-scroll-fallback-mode 'pass-through'`). The skeleton's own wheel handling was verified by
  feeding SGR wheel events through tmux. The real htui should say this in its docs, and keep the
  keyboard zoom.
- **View options** (global): `L` link kinds as **legend box · labels on links · both · none**; `m`
  minimap; `d` top-to-bottom / left-to-right.
- **Every arrow points down the page**, from what comes first to what follows (explicit request: the
  item graph used to read bottom-up).
- Nodes: rounded box per item/step, **dashed box with `◇` for requirements**, thick border for the
  root and the selected node, border colour by status.

### 5.1 Item graph (Backlog → graph)

Top to bottom: parent requirements → the requirements the item covers → its blockers → **the item**
→ what follows it.

| link | line | meaning |
|---|---|---|
| refines | solid amber | parent → child requirement |
| covered by | dotted amber | requirement → item covering it |
| blocks | solid | must be done first |
| follow-up | solid cyan | item → what came out of it |
| relates to | dotted violet | related, no order |

**Enter:** on another item → go to that item (stays on the graph sub-tab); on the item itself → its
live chat; on a requirement → Workspaces → requirements with the cursor on it.

### 5.2 Run graph (Backlog → runs)

The **fan-out** is several agents each working on a **different part of the same item** (e.g. research
A, B, C), then merged — explicitly **not** competing implementations with a "not picked" loser (the
first skeleton got this wrong). The **review loop** is shown unrolled:

```
plan → ┬ research · API        (claude)  ┐
       ├ research · prior art  (codex)   ├→ merge findings → code · round 1 → review · round 1
       └ research · perf       (gemini)  ┘        ─(changes requested)→ code · round 2 → review · round 2
                                                   ─(approved)→ gate
```

Since round 8 the graph follows the item's **workflow** (§4.6), and since round 9 it is computed by
**unrolling the template**: the first `changes requested` loop is taken once (two rounds), fan-outs
become one node per part, the template's relation kinds carry over (`asks you` before code in
research fan-out, `approved` into the gate). Every run graph starts with `prepare · <target>` →
`pick workflow` and, off this box, ends with `collect the branch`; the legend shows only the kinds
the run uses.

Links: `fans out`, `merges`, `changes requested` (red), `approved` (green), plain `next`; links into
the running step are animated. **Enter:** running / waiting step → its live chat; finished step → its
**archived transcript** (read only); not started → nothing ("has not started yet").

### 5.3 Requirements graph (Workspaces → requirements → `v`)

Requirements (`refines` parent → child) and the items covering them (`covered by`). Enter as in the
item graph.

### 5.4 rataflow findings for the real implementation

- **Disconnected components overlap:** `Flow::apply_layout` lays out each connected component from
  the same origin (`layout.rs`, `from_edges` returns one layout per component), so separate clusters
  land on top of each other. The skeleton fixes it by union-finding the components after layout and
  shifting each one sideways (`flow.rs::spread_components`). htui will need the same whenever a graph
  is disconnected (requirements graphs nearly always are).
- `request_fit_view` zooms small graphs to 2× and big ones below 1; use
  `request_fit_view_with_options(FitViewOptions::default().with_zoom_range(1.0, 1.0).with_nodes(…))`.
- Available customisation (rataflow 0.1): per-edge `label`, `animated`, `StepEdge::with_style(EdgeStyle)`
  (stroke and label style, `dotted()`, `braille()`, `ascii()`, markers), `MiniMap`, `Background`,
  `Controls`, `Sugiyama::{vertical, horizontal}` with node/rank spacing, parent/child nodes,
  multi-select, `Theme::Custom(Palette)` (set `canvas_bg` / `surface` to `Color::Reset` to blend in).
- Default key bindings include `Delete`/`Backspace` = delete node: never forward those (htui's MOD-28
  already forwards no keys to rataflow's own bindings).
- **Loops need side handles** (round 9): a back edge between the default bottom/top handles draws on
  top of the forward chain and disappears. Give nodes extra handles (`Handle::source/target(Right |
  Left).with_id(…).with_offset(…).with_hidden(true)`) and route loops through them with
  `Edge::with_source_handle / with_target_handle`; once a node has more than one handle of a kind,
  **every** handle needs an id (`AmbiguousHandles` otherwise, even for the default ones). Lay out
  without the loops, then `add_edge` them, so they never bend the ranks.
- **Never use rank spacing 1** (round 9): with a one-row gap between ranks, an edge's arrow vanishes
  whenever the fitted view's pan lands on a whole row; which graphs it hit depended on how many nodes
  the fit centred on. Spacing 2 is safe.
- Two fit requests before the first draw (e.g. `request_fit_view_with_options` twice) also left the
  edges undrawn in one test; make one request.
- Dense neighbourhoods get crowded and labels on parallel links can collide; the legend mode reads
  better there. Not solved.
- Graph state (pan / zoom / selection / moved nodes) is cached per graph so it survives redraws.
- `Background::new(&flow)` is a separate widget drawn under the flow; the flow does not clear it
  when its palette's `canvas_bg` is `Color::Reset`. Its dots use `·`, like `EdgeStyle::dotted()`
  links, so keep the dots faint and the dotted links coloured.
- rataflow always draws the connection handles (`○` on node borders), dimmed when a node is not
  connectable; hiding them needs `set_handles_hidden` per node. Left as is.
- Dragging nodes needs `with_draggable(true)`; `Flow::move_node(id, (dx, dy))` moves one from the
  keyboard. Edges are reconnectable **by default**, so a read-only graph must opt out.

## 6. Smaller findings

- `Shift ←→` for sub-tabs and `Ctrl ←→` for panes avoid AltGr-only keys on non-US layouts; keep
  that in mind for every default binding (MOD-67's keymap).
- Right-aligned border titles start at `area.right() - 1 - width`; clickable chips in borders must
  compute their x from that.
- Bottom-border hints must be trimmed to the pane width, or they overwrite the corner.
- Group rows in a tree that represent an entity (a workspace) should show that entity's detail, not
  "nothing selected".
- History is browser-style (round 7); it no longer resets when the user moves.
- `cargo clippy` does not rebuild the binary: a `--dump` after clippy shows the old build.
- A list's "what is hidden" note goes in a row under the last entry, not in the border title: a
  narrow list cut "items by project · 5 done hidden" in half.
- Mock id clash: a new mock requirement reused `R-30`; ids are per workspace, so anything that mints
  them (the rewrite, the real store) must take the workspace's maximum.

## 7. Iteration log (what the maintainer asked, in order)

1. "Not modern, not intuitive" → audit (§1); chose to try a skeleton before any spec.
2. btop; IDE proportions; toggleable "about this tab"; colours per item kind / feature; editable
   layout → shell, presets, schemes, about panel.
3. Use rataflow for graph and runs; `N` opens a centred modal with AI rewrite and a talk-it-through
   chat; body must not scroll forever; arrows must not change pane; workspace choice out of Settings.
4. A Workspaces tab (projects, saved workspaces, background running, graceful/forced stop,
   components at workspace and project level with generated compose, descriptions/goals, absorbs
   Requirements); "what does activity do?"; a Logs tab; Chat with workspace + project coordinators
   and item runs, view-only until asked.
5. Quit must keep things running; sub-tabs unreachable from a graph; graph customisation; Enter in a
   graph; tmux wheel; drop the open-workspace sentence; new workspace needs a folder; nested
   requirements and a requirements graph; item graph shows requirements and link kinds (legend or
   on links).
6. Activity newest first; back after Enter; fan-out means parallel parts, not competing
   implementers; Enter on run steps by state (live / archived / nothing); review loop in the run
   graph; pick a requirement's items and jump; item graph top-down; emulator components, versions,
   catalogue in the DB; new-item modal defaults to the item's project and is resizable; separate
   workspace/project coordinators plus a global htui agent.
8. Answers: both stops need a confirmation; history records tab, pane and sub-tab changes too;
   moved graph nodes may reset; no starting/stopping the stack with the workspace, but each
   component can be started or stopped; Q3 (stopping) and Q5 (wheel) need explaining; the mouse is
   all fine. New: choose per item where it runs (local, docker like `scripts/hr`, remote ssh, remote
   docker, a box through a central htui server, planned), with compose and everything set up there;
   Skills becomes a tab for every tool a run can use (skills, personas, mods, workflows — templates
   for how an item runs, changeable by the model at run time, picked at the start by a fast prompt —
   and more, probably an ANA); floci always includes every service.
9. floci also emulates other clouds: include them (checked: AWS, Azure, GCP, Oracle + console). The
   workflow pane: no "used by"; view and edit the template as a graph; the template needs real loops
   (code → review → back to code on changes, else next) and maybe other relations, while runs show
   the unrolled rounds. Stopping means stopping running items (fine with one workspace); graceful =
   before the next agent turn if supported, else at once; conversations always saved. Wheel: do both
   the docs and the hint. Answers: default target local, set in Settings, workspace, project, by
   cascade; collect like `scripts/hr`; an htui agent does the remote setup; fast pick model light
   (haiku) and configurable from the agents' models; the agent may change workflows, with a setting
   for automatic or ask; workflows global, project workflows later; add hooks, commands, sub-agents,
   prompts, env presets, MCPs and tools like gortex and headroom, with an ANA on how to show them.
7. Answers: tab + kind colours; default presets fine but sizes must survive a restart; digits for
   tabs fine; real back/forward history; stop once = wait for the next stoppable step then close,
   twice = stop outright, both resumable; compose in the workspace folder, `.env` in Infisical if
   available else in the workspace folder; the mouse works except the wheel. New: rename Settings →
   Connection to Postgres; Backlog/Hierarchy becomes Backlog keys (`h` done items, `g` grouping);
   dotted background on every graph and nodes movable without editing links; new requirements go
   through the new-item flow, and a workspace requirement shows the project requirements it created.

## 8. Open questions (not decided)

Answered in round 7 (see the sections): colour scheme (§3.5), default presets and remembered sizes
(§3.4), digits stay tab switches (permissions moved to `a A r`), browser-style history (§3.6), what
stop does and resuming (§4.1), where compose and `.env` live (§4.1 Components). Answered in round 8:
history records tabs, sub-tabs and panes too (§3.6); moved graph nodes reset on re-layout, and that
is fine (§5); the stack is not started or stopped with the workspace, components are one by one
(§4.1). Still open:

Answered in round 9: stopping (§4.1), the wheel (§5), default targets by cascade, collecting, the
htui agent's setup role (§4.2a), the fast pick model and run-time changes (§4.5, §4.6), global
workflows (§4.6), the extra toolbox kinds (§4.6). Still open:

1. Project workflows: a workflow that drives a whole project, coordinating its items (wanted later;
   how it relates to the workspace and project coordinators and to the queue).
2. Templates: stay a Toolbox group, or get their own tab as MOD-113 says? (The toolbox ANA decides.)
3. Workflow editing: is a step's agent a fixed agent, a persona, or a role the run fills? Can a
   workflow pin a target too, or is that only per item?
4. Which steps count as "can pause between turns": only agents with resumable sessions (now), or
   also checks that can be killed and rerun cheaply?
5. The Infisical folder layout for a workspace's `.env` (the skeleton shows `/<workspace>`).
6. floci: when a project uses only one cloud, should it still get all five containers?
7. Requirements: does a workspace requirement's status come from its project requirements (all done
   → done)? Can an agent propose splits for an existing workspace requirement later?
7. Catalogue: source of updates (a URL htui publishes? registry tags?), and whether users can add
   their own kinds.
8. Requirements: id scheme for nested ones (`R-2.1`) vs. the current flat ids; who may edit them
   (user only, or coordinators too).
9. Archived transcripts: retention (ties to `retention_days`, MOD-108).
10. New-item flow: which model does the rewrite (`utility_model` was rejected by ANA-30), and does the
    talk-it-through agent become an item chat once the item exists?
11. The htui agent's permissions: which settings it may change without asking.
12. Mouse: all fine (round 8), including dragging graph nodes.

## 9. What the real implementation needs (not decided, notes only)

- A shared widget kit (pane with border title/hints/sub-tabs, tree list with groups + filter +
  selection band, form rows with inline help, key chips, empty state, modal frame, legend) so tabs
  stop re-implementing them (audit item 6). `backlog/mod.rs` and `settings/agents.rs` would be split
  along these lines.
- Layout state (preset, splits, visibility, zoom) per tab, persisted with the rest of the UI state.
- A navigation model with focusable panes and a back stack.
- A workspace lifecycle in the store and worker (open / background / stopping / stopped) that the
  UI only reflects.
- Snapshot churn: every tab changes; the 199 insta snapshots will be rewritten tab by tab.

## 10. Candidate requirements to file after the review (NOT filed)

Drafted for discussion only; wording and ids to be settled with the maintainer.

- **Shell:** one-row header (workspace, tabs, status, back/forward chip); hints in the focused pane's
  border; toggleable per-tab "about" panel; per-tab layout presets with resizable splits, show/hide
  panes and zoom, persisted the moment they change; tab accents plus kind colours.
- **Navigation:** arrows never leave a pane; pane switching by `Tab` / `Ctrl ←→` / click; sub-tabs by
  `Shift ←→` / click; browser-style history of jumps and of tab, sub-tab and pane changes (`⌫` /
  `Alt ←` back, `Alt →` forward).
- **Leaving** never stops work by default; graceful stop-all is an explicit choice.
- **Workspaces tab:** saved workspaces with lifecycle states; opening one backgrounds the other
  without interrupting runs; graceful stop (runs reach their next stoppable step, then park) and
  forced stop (cut mid-step), both resumable and both confirmed first; a folder per workspace
  (`.htui/`); projects from a folder, a git URL or a new repo; descriptions and goals.
- **Components:** typed components (database, cache, queue, cloud emulator, object storage, search,
  custom) from a **catalogue stored in the database and updatable**, with versions and emulated
  services; a generated compose file in the workspace folder with free ports and secrets from the
  Secrets provider; the `.env` in Infisical when available, else in the workspace folder; per
  project: shared instance, own instance, or unused; each component started or stopped on its own;
  floci always emulates every service.
- **Execution targets:** each item chooses where it runs: this box, docker on this box (a sandbox
  like `scripts/hr`), a remote machine over ssh, docker on a remote machine, or (phase 2, MOD-47) a
  box enrolled with an htui server; the default cascades Settings (local) → workspace → project →
  item; wherever it runs, htui prepares the code, starts the item's compose, and configures env,
  secrets and agents there (the htui agent installs and logs in what is missing); work comes back by
  collecting the branch. The run graph shows prepare first and collect last.
- **Stopping runs:** graceful stop before the agent's next turn (at once for steps that cannot
  resume), forced stop mid-turn; conversations always saved; everything resumable; works on the open
  workspace.
- **Workflows:** global templates with real loops and typed relations (next, approved, changes
  requested, fails, retries, asks you, if), viewed and edited as a graph; runs unroll them; a fast
  pick by a configurable light model chooses one when a run starts; the user can pin one per item;
  the agent may change it at run time, automatically or after asking (a setting). Project workflows
  that coordinate a whole project: later.
- **Toolbox tab** (replaces Skills): workflows, skills, personas, templates, mods, MCP servers,
  hooks, commands, sub-agents, prompts, env presets, and tools like gortex and headroom. **Needs an
  ANA first** (no existing item covers it): which kinds exist, how each is shown and attached to a
  run, how tools that bundle several kinds (MCP + hooks + daemon) appear, the fast pick's model
  (against ANA-30), and how it relates to MOD-113 and ANA-33.
- **Mouse wheel under tmux:** a docs section, and a one-time in-app hint inside tmux.
- **Requirements** live in Workspaces, per workspace and per project, **nest**, show their covering
  items as navigable rows, and have a graph view; they are added through the same describe → rewrite
  → review modal as items; a workspace requirement is split into project requirements, and each side
  links to the other.
- **Backlog:** IDE proportions; new-item modal with LLM rewrite and a talk-it-through agent,
  defaulting to the current item's project, resizable; `h` done items, `g` grouping (project, kind,
  status), persisted.
- **Graphs:** item graph top-down with requirements and typed links (legend or labels); run graph
  with fan-out (parallel parts), merge, and an unrolled review loop; Enter navigates (item, live
  chat, archived transcript, requirement); keyboard zoom; dotted background; nodes movable by drag or
  keyboard, links read-only.
- **Settings:** Connection renamed Postgres; no Backlog/Hierarchy section.
- **Chat:** a global htui agent; one workspace coordinator; one coordinator per project; item-run
  chats read-only until they ask; archived step transcripts.
- **Activity:** newest first, pinned "needs you", jump to source, filters.
- **Logs tab** for htui's own log with level filter and follow.
- **Status** shown as glyph + human word everywhere; colour never the only signal.
