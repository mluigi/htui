# MOD-115 M1 blueprint (architect fallback, condensed)

Companion to `mod-115-tui-shell-m1.plan.md`. Where they differ, this file wins (it is the result of reading the code).

## Maintainer decisions (2026-10-17)
1. **Help keeps `?` and `F1`. About is `F9`.** Add `Act::About` with a Global row `row(Act::About, Global, "about", &["f9"], "about")`
   next to the Help row (`keys/catalogue.rs` ~855). Do not touch Help's chords. Header chip reads `F9 about ? keys`
   (labels resolved through `Keys`, never hard-coded). Check `no_two_actions_in_one_context_share_a_default_chord`
   and `validate::check` pass; never allow-list a clash.
2. **About on/off is in memory in M1** (`App.about_visible: bool`, default true). No state file; M2 adds it.
3. **`Harness::over_backend` (testkit) sets `about_visible = false`**; add exactly one `tests/shell.rs` snapshot with it on.

## Conventions
Render fns take `(frame, area, ..., &Theme)`; pure geometry `#[must_use]`; `//!` module doc; styles from `Theme` roles
only, each with a monochrome test (`fg == None && bg == None`, emphasis via BOLD/DIM/UNDERLINED); tests in-file,
`TestBackend` read back as strings, sentence-named. Labels arrive pre-resolved from `Keys::label/labels/hint`
(`keys/hint.rs`); no chord text hard-coded in the kit. Glyph widths via `cells::cell_width` (◐ ⚠ ⊘ ambiguous).

## ui/mod.rs
Add `pub mod kit; pub mod header; pub mod about;`. Keep `pub mod top_bar;` until Task 5, then remove.

## kit API
**pane.rs**: `enum Focus{Focused,Unfocused}`; `struct Pane<'a>`; builder `new(title)`, `sub_tabs(&[&str], active)`,
`hints(Line)` (drawn only when Focused), `focus(Focus)`, `accent(Style)`, `scrollbar(total, offset)`;
`render(self, frame, area, &Theme) -> Rect` (inner). `sub_tab_rects(area, titles) -> Vec<Rect>` (chip x = `area.right()-1-width`);
`clamp_offset(offset,total,viewport)` (scroll stops at end); `paint_bottom_hints(buf, row, Line, Style)` (M1 adapter below).
Rounded border; focused = tab accent; unfocused = `theme.border_faint`; monochrome focused BOLD, unfocused DIM.
Hints start at x+2, clipped to width-4 with `cells::clip_line`, never touch corners. Title clipped first; chips dropped whole.
**list.rs**: `GUTTER="▌"`; `RowKind{Item,Group{folded},Note}`; `Row{line,kind}`;
`render(frame,area,rows,selected,offset,accent,&Theme)`; `selection_band`, `ellipsize` (wraps `cells::clip_line`),
`group_row(label,count,folded,&Theme)`, `hidden_note(count,noun,key_label,&Theme)`, `scroll_into_view(selected,offset,height,total)`.
Unselected rows get a blank gutter. Band reuses `Theme::select`.
**status.rs**: `StatusMark{glyph,word}`; `TABLE`; `mark(Status)`, `spans(Status,&Theme)`. Map over `htui_core::model::Status`:
● done=Done, ◐ running=InProgress, ⚠ needs you=AwaitingApproval, ✗ failed=Failed, ⊘ blocked=Blocked, ◌ queued=Queued,
○ open=Open, ✓ closed=Closed. Tests: every Status variant covered, no word contains `_`.
**chips.rs**: `key_chip(key,&Theme)`, `chip_line(&[(String,&str)],&Theme)`, `legend(&[(&str,Style)],&Theme)`.
**empty.rs**: `NextStep<'a>(pub &'a str)` (required), `lines(what,next,&Theme)`, `render(frame,area,what,next,&Theme)`.
**modal.rs**: `CHROME=3`; `ModalSize{Fit{content_w,content_h},Fixed{w,h}}`; `Modal` builder `new(title)`, `size`, `hint`;
`render(self,frame,area,&Theme)->Rect` (Clear + `layout::centered` + block; Fit clamps to area-4 wide, area-2 tall, copying
`waiting_list` arithmetic).

## header.rs (replaces top_bar.rs; `TopBarState` and `App.top_bar` keep their names)
`Flash{text,style}`; `HeaderHits{workspace:Rect, tabs:Vec<(TabId,Rect)>}`;
`render(frame,area,&TopBarState,&TabRegistry,flash:Option<&Flash>,&Keys?,&Theme)->HeaderHits`; `store_style` moved unchanged.
Left: workspace then tab chips (superscript ¹..⁹ then digits; active = filled block in tab accent, monochrome BOLD|UNDERLINED).
Right, priority-dropped when narrow: `N working`, `M waiting` (warning when owed), store label (warning when degraded),
box name, `F9 about ? keys`. A flash replaces the right side. History chip and background count render nothing in M1.
Return `HeaderHits` and test it; **wire no mouse handler** (mouse is gated by `App::wants_mouse`).
Move the `top_bar.rs:161-245` tests to header.rs.

## about.rs
`About{purpose,actions:&[(Act,&str)]}`; `about_for(TabId)->Option<&'static About>` (static table keyed by `TabId.0`);
`render(frame,area,&About,&Keys,Stack,&Theme)`; an act with no label is skipped. Tabs unchanged in M1.

## theme.rs (Task 0)
Add `border_faint` (Indexed(240); mono DIM), `tab_accents` and `kind_colours` (Copy structs of Style), `tab_accent(&self,&str)`
(fallback `accent`), `kind_style(&self,&str)` (unknown prefix → `key`). Accents: backlog Cyan, chat Green, skills Magenta,
settings Blue, requirements Yellow; reserved workspaces Yellow, logs Indexed(208); all mono BOLD. Kinds: ANA, MOD, CLEAN, TOOL,
FIX, FEAT, VAL, `R-`. Update `the_default_palette_gives_each_role_its_colour` and `the_monochrome_theme_names_no_colour`.
Keyed by string so theme does not import `TabId`.

## layout.rs
`Chrome{header:Rect, about:Option<Rect>, body:Rect}`; `ABOUT_HEIGHT=4`; `chrome(area, about:bool)`; about is `Some` only when
header 1 + about 4 + body >= 6 rows fit. Keep a one-argument `chrome(area)` = no about panel (16 test call sites:
`backlog/{divergence,filter,item_form,mod}.rs`). Rewrite the five-Rect test.

## App::render (app/state.rs:873) new order
dirty=false; editor_area.set(None); `chrome(area, self.about_visible)`; `header::render(...)`; about panel if `Some`; active tab
into `chrome.body`; `draw_editor(frame, chrome.body)`; hint adapter (only when `overlays.is_empty() && !editor_visible()`); overlays;
`render_help` (unchanged).
New: `about_visible`, `Action::ToggleAbout` (`app/action.rs`), handled in `update.rs` next to `ToggleHelp` (~:42),
`Act::About => Some(Action::ToggleAbout)` in `action_for` (state.rs ~377).
**Hint adapter**: `Tab::key_hint(&self,width,ctx)` stays unchanged (no tab file changes); width = `chrome.body.width - 4`; after the tab
renders, `paint_bottom_hints` overwrites cells of the body's last row only where the symbol is a horizontal border glyph (`─`/`━`),
never corners, skipping otherwise. Delete in M3/M4 as tabs move to `Pane::hints`. A mode whose last row is not a border simply loses
the hint in M1; list those in the Task 7 review.
**Flash**: `fn flash(&self)->Option<Flash>` = first of `App.status` (error style, cleared on next key), `editor_status()` (dim),
`editor_notice()` with trailing ` · ` stripped (dim). Global `keys.status_line` text is dropped. `editor_status` is sticky: drop
other header parts before truncating it. No 2.6 s timer in M1. Check `Keys::status_line` dead-code on the featureless clippy run.
**Editor overlay**: only the body rect changes; PTY size follows `pane_rect`. Hard-coded-row tests break: `app/pane.rs:1684-1707`
(status row 29), `:2582` (`row_from(..,0,2)`), the `another_tab_while_editing` snapshot, `tests/chat.rs:631` (`body_lines`).
`SectionBench` bypasses chrome; overlay benches draw bodies only.

## Call sites
`top_bar::render`: `app/state.rs:879` only (+ its tests). `tabs::registry::render_strip`: `state.rs:880`, test `registry.rs:286`.
Do NOT touch `settings::render_strip` (`settings/mod.rs:480`, called :465, test :520), `backlog::detail::render_strip`
(`detail/mod.rs:383`), or Skills' inline switch (`skills/mod.rs:~220`). `Chrome` fields: `.top_bar/.tab_strip` (layout.rs, state.rs),
`.hint` (state.rs:902-909), `.status` (state.rs:931), `.body` (state.rs + 16 test sites). `Tab::key_hint` implementers:
`backlog/mod.rs:877`, `chat/mod.rs:557`, `requirements/mod.rs:1265`, `settings/mod.rs:452`, `skills/mod.rs:160`, trait default `registry.rs:85`.
Registration order today: Backlog, Skills, Requirements, Settings, Chat (`app/mod.rs register_all`). No UI-state persistence exists.

## Corrections to the plan
- "Body unchanged apart from row offset" is false: body height changes (26 → 29 rows without about, 25 with). Task 7 flags content
  diffs, not height-driven ones.
- Snapshots: 241 `.snap` files (170 in `crates/htui/tests/snapshots`). Only frames through `Harness::render`/`App::render` carry chrome.
  `integration__demo_shell.snap` shows the old layout.
- Task 5's gate is build + non-snapshot tests + fixing hard-coded-row tests; the snapshot gate closes in Task 7.
- Tasks 1-4 are disjoint only after Task 0 lands theme roles + stub files; Task 4 needs Task 3's `chips.rs`.

## Per-task files
0: CREATE `ui/kit/{mod,pane,list,chips,status,empty,modal}.rs` stubs, `ui/header.rs`, `ui/about.rs`; UPDATE `ui/mod.rs`, `ui/theme.rs`.
1: `kit/pane.rs`. 2: `kit/list.rs`, `kit/status.rs`. 3: `kit/chips.rs`, `kit/empty.rs`, `kit/modal.rs`. 4: `ui/header.rs`, `ui/about.rs`.
5: `ui/layout.rs`, `app/{state,action,update,pane}.rs`, `ui/tabs/registry.rs`, `keys/catalogue.rs` (+ keys tests/print/hint where the
   new Act forces it), `ui/mod.rs`, `testkit.rs`, `tests/chat.rs`, backlog test sites, `tests/shell.rs`; DELETE `ui/top_bar.rs`.
6: `ui/overlay/waiting_list.rs`, `ui/overlay/quit_confirm.rs` + their snapshots. 7: `*.snap` only.
