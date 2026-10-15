# UI redesign — findings so far (2026-10-15)

**Status: review in progress. Nothing here is a requirement yet.** `docs/REQUIREMENTS.md` and
`HANDOFF.md` are untouched on purpose; §10 lists the requirements to file once the maintainer
finishes reviewing the skeleton. This document exists so the findings survive a compacted or new
session.

## 0. How to resume

- **Branch:** `spike/ui-skeleton` (cut from `main` at `809f8d22`). The spike and this doc are
  **uncommitted** unless a later commit says otherwise — check `git status` first.
- **Skeleton:** `spikes/ui-skeleton/`, a standalone crate with its own `[workspace]` (the htui
  workspace never builds it). ~6.3k lines of Rust, ratatui 0.30.2 + rataflow 0.1, mock data only:
  no database, no agents, no model calls. It is a **throwaway**: what we keep is the decisions, not
  the code.
- **Run:** `cargo run --manifest-path spikes/ui-skeleton/Cargo.toml`. Layout, colours, the about
  panel and the new-item modal size persist in `spikes/ui-skeleton/.ui-skeleton.state` (delete it
  to reset; it is gitignored).
- **Headless check:** `./target/debug/htui-ui-skeleton --dump <tab 1-6> [preset 0-3] [WxH] [keys]`
  prints one frame as text. Keys replay from the list pane: `_` down, `^` up, `!` Enter, `~` Tab,
  `<` `>` left/right, any other char as itself (e.g. `'__!]]'` opens MOD-67's graph).
- **Interactive checks** were driven through tmux (`send-keys` + `capture-pane`) from bash scripts;
  zsh's profile in this environment breaks inline tmux functions. The mouse was never tested this
  way: drag-resize of panes and of the modal, and graph pan/zoom by mouse, still need a manual try.
- **Files:** `main.rs` (event loop, input, navigation), `app.rs` (state, tree, layout presets, back
  stack, persistence), `ui.rs` (header, about panel, pane layout, help), `views.rs` (lists, Backlog,
  Chat, Skills, Settings, activity, graph rendering), `wsview.rs` (Workspaces, Logs, compose
  generation), `flow.rs` (rataflow graphs), `modal.rs` (new item, add component), `forms.rs` (leave,
  "new what?", workspace/project/requirement forms), `data.rs` (mock data), `theme.rs`.

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
- `F2` / `F3` (also `Alt-1` / `Alt-2`) show or hide list / activity; `z` zooms the focused pane.
- Every tab keeps its own layout; all of it persists.
- The original idea of digits for panel toggles was dropped: digits stay tab switches.

### 3.5 Colour

Three schemes, switchable live with `c`, to compare:
1. **tab + kind colours** (current default): each tab has an accent (workspaces amber, backlog cyan,
   chat green, skills violet, logs orange, settings blue) used on its strip entry and focused
   borders; item keys are coloured by kind (ANA, MOD, CLEAN, TOOL, FIX, FEAT, VAL, R-).
2. **tab colours only.**
3. **kind colours only** (one neutral accent).

Status never relies on colour alone: glyph + word everywhere (`● done ◐ running ⚠ needs you ✗ failed
⊘ blocked ◌ queued ○ open ✓ closed`). Status words are human (`needs you`, not
`awaiting_approval`). **Not decided yet** which scheme wins.

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
- **Back after a jump** (explicit request): any jump (Enter in a graph, the activity panel, the
  requirements list) records (from, to). `⌫ Backspace` or `Alt ←` goes back; chained jumps unwind one
  at a time. The way back is **forgotten as soon as the user moves** away from where the jump landed.
  The header shows `⌫ back to MOD-67 · graph`.
- **Leaving:** `q` / `Ctrl-C` never just quit. A "leave htui" modal lists every live workspace with
  running / waiting counts: `Enter` leaves the window and **the work keeps running in the
  background**, `S` stops everything gracefully and leaves, `Esc` stays; a second `Ctrl-C` leaves.
  After leaving, the terminal prints e.g. "htui keeps running in the background: 4 run(s)".
- `?` opens a two-column key reference; `/` filters the focused list.
- `c` (colour scheme) and other single letters are global; graph-specific letters (`L m d f + - 0`)
  only act when a graph has focus.

## 4. Tabs

Tab set changed during the review: **workspaces · backlog · chat · skills · logs · settings**. The
Requirements tab is gone (absorbed by Workspaces); Logs is new.

### 4.1 Workspaces (new tab; replaces the `w` switcher overlay and Settings' workspace field)

- **List:** every saved workspace as a group, with its state; under it an `overview` entry and one
  entry per project. Selecting the workspace's own row shows its overview.
- **States:** `● open` (the one on screen), `◐ background · n running` (runs keep going),
  `◌ stopping…`, `○ stopped`.
- **Opening another workspace** (`o`) sends the current one to the background; **its running items
  keep running.** The header shows `+N in background`.
- **Stopping** (`s`) is graceful (runs finish their current step); `s` again **forces** it. The open
  workspace cannot be stopped from here ("open another first, or stop single runs in the Backlog").
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
- **Project level:** for each workspace component, `Enter` cycles **the workspace's instance · its
  own instance (on a free port) · not used**; the selected row shows the env var the project gets
  (e.g. `DB_URL=postgres://localhost:5433/agy`). `N` adds a component only this project uses.
- **Add-component wizard:** type → kind → **version** → **services** (only for emulators; `Space`
  toggles) → name & port (picked free, editable) → compose preview with `+` on the new lines.
- **Types:** database, cache, queue, **cloud emulator** (floci, LocalStack, ElasticMQ, Azurite, GCP
  emulators — explicit request: "something like floci to emulate s3, sqs and others"), object
  storage, search, custom image.
- **The catalogue is data, not code** (explicit request): seeded into the database at first start,
  updated on request (`U` in the wizard; Settings → Catalogue → "update now"; or ask the htui agent).
  The wizard shows "catalogue updated 3 days ago".
- **Unverified mock values:** the floci entry (image `floci/floci`, port 4566, service list) is a
  placeholder; the real image name and port were not checked. Other images are real public ones,
  but their version lists are illustrative.

#### Requirements (moved here from their own tab)

- Per workspace (workspace-wide ones) and per project.
- **Nested:** a requirement can refine another (`R-2` → `R-2.1`, `R-2.2`); the list draws a tree
  with `├─ └─ │` connectors; the new-requirement form has a "refines" parent picker and the child id
  is the parent's plus `.n`.
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
- Empty workspace: "Nothing here yet. Press N to create the first one."

### 4.3 Chat

- **Groups, in order:** **htui agent** · **workspace** · **projects** · **item runs** · **archived**
  (folded). Explicit requests: separate workspace and project coordinators; always show the
  workspace coordinator and one coordinator per project; add a global agent.
- **htui agent** (`✦`, shown in every workspace): changes htui's settings, adds skills and agents,
  updates the component catalogue. Badge: "settings · skills · agents · catalogue".
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

- Same sections minus the workspace picker. Hierarchy now only shows how projects nest ("switching
  workspace is w, from anywhere").
- Fields edit **inline on their row**; the help shows on the line under the selected field (fixes
  audit item 3). Toggles `● on` / `○ off`, choices `‹ value ›`, secrets `•••••••• stored`.
- New **Catalogue** section (source, last updated, "update now" action).

### 4.6 Skills

Unchanged from the first skeleton (list + detail with description, attach glob, body). Not reviewed
yet.

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
  `f` fit (never above zoom 1), `Enter` opens (below).
- **Mouse:** drag pans, wheel zooms. The maintainer's **tmux grabs the wheel** (enters copy-mode), so
  the keyboard zoom matters; tmux's default bindings normally pass the wheel to mouse-capturing apps,
  so their tmux config likely sends it to copy-mode unconditionally.
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
- Dense neighbourhoods get crowded and labels on parallel links can collide; the legend mode reads
  better there. Not solved.
- Graph state (pan / zoom / selection) is cached per graph so it survives redraws.

## 6. Smaller findings

- `Shift ←→` for sub-tabs and `Ctrl ←→` for panes avoid AltGr-only keys on non-US layouts; keep
  that in mind for every default binding (MOD-67's keymap).
- Right-aligned border titles start at `area.right() - 1 - width`; clickable chips in borders must
  compute their x from that.
- Bottom-border hints must be trimmed to the pane width, or they overwrite the corner.
- Group rows in a tree that represent an entity (a workspace) should show that entity's detail, not
  "nothing selected".
- "Back" chains while the user does not move, then resets; it is not browser history.

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

## 8. Open questions (not decided)

1. Which colour scheme (§3.5)?
2. Default preset per tab, and the default list width.
3. Digits for tabs vs. anything else (they are taken from Chat's permission answers).
4. Back navigation: keep single-chain-until-you-move, or a real history (`Alt ←/→`)?
5. Workspaces: can the open workspace be stopped from its own tab? What exactly does "stop" do to an
   item mid-step (finish the step, then park the run)?
6. Components: where the compose file and `.env` live when a workspace points at existing repos;
   how secrets are injected (Secrets provider → env); whether htui starts/stops the compose stack
   with the workspace.
7. Catalogue: source of updates (a URL htui publishes? registry tags?), and whether users can add
   their own kinds.
8. Requirements: id scheme for nested ones (`R-2.1`) vs. the current flat ids; who may edit them
   (user only, or coordinators too).
9. Archived transcripts: retention (ties to `retention_days`, MOD-108).
10. New-item flow: which model does the rewrite (`utility_model` was rejected by ANA-30), and does the
    talk-it-through agent become an item chat once the item exists?
11. The htui agent's permissions: which settings it may change without asking.
12. Skills tab: not reviewed.
13. Mouse: drag-resize and graph mouse handling untested in this session.

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

- **Shell:** one-row header (workspace, tabs, status, back chip); hints in the focused pane's border;
  toggleable per-tab "about" panel; per-tab layout presets with resizable splits, show/hide panes and
  zoom, persisted.
- **Navigation:** arrows never leave a pane; pane switching by `Tab` / `Ctrl ←→` / click; sub-tabs by
  `Shift ←→` / click; back after a jump until the user moves.
- **Leaving** never stops work by default; graceful stop-all is an explicit choice.
- **Workspaces tab:** saved workspaces with lifecycle states; opening one backgrounds the other
  without interrupting runs; graceful then forced stop; a folder per workspace (`.htui/`); projects
  from a folder, a git URL or a new repo; descriptions and goals.
- **Components:** typed components (database, cache, queue, cloud emulator, object storage, search,
  custom) from a **catalogue stored in the database and updatable**, with versions and emulated
  services; a generated compose file with free ports and secrets from the Secrets provider; per
  project: shared instance, own instance, or unused.
- **Requirements** live in Workspaces, per workspace and per project, **nest**, show their covering
  items as navigable rows, and have a graph view.
- **Backlog:** IDE proportions; new-item modal with LLM rewrite and a talk-it-through agent,
  defaulting to the current item's project, resizable.
- **Graphs:** item graph top-down with requirements and typed links (legend or labels); run graph
  with fan-out (parallel parts), merge, and an unrolled review loop; Enter navigates (item, live
  chat, archived transcript, requirement); keyboard zoom.
- **Chat:** a global htui agent; one workspace coordinator; one coordinator per project; item-run
  chats read-only until they ask; archived step transcripts.
- **Activity:** newest first, pinned "needs you", jump to source, filters.
- **Logs tab** for htui's own log with level filter and follow.
- **Status** shown as glyph + human word everywhere; colour never the only signal.
