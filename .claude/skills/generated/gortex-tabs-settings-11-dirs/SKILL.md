---
name: gortex-tabs-settings-11-dirs
description: "Work in the tabs/settings +11 dirs area — 781 symbols across 35 files (82% cohesion)"
---

# tabs/settings +11 dirs

781 symbols | 35 files | 82% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/auth/mod.rs`
- `crates/htui-agent/src/cli/mod.rs`
- `crates/htui/src/app/action.rs`
- `crates/htui/src/app/state.rs`
- `crates/htui/src/app/update.rs`
- `crates/htui/src/catalogue.rs`
- `crates/htui/src/hierarchy.rs`
- `crates/htui/src/qdrant_settings_info.rs`
- `crates/htui/src/testkit.rs`
- `crates/htui/src/ui/overlay/migration_prompt.rs`
- `crates/htui/src/ui/overlay/registry.rs`
- `crates/htui/src/ui/overlay/workspace_switcher.rs`
- `crates/htui/src/ui/tabs/backlog/detail/documents.rs`
- `crates/htui/src/ui/tabs/backlog/detail/mod.rs`
- `crates/htui/src/ui/tabs/backlog/detail/notes.rs`
- `crates/htui/src/ui/tabs/backlog/detail/prompt.rs`
- `crates/htui/src/ui/tabs/backlog/list.rs`
- `crates/htui/src/ui/tabs/backlog/mod.rs`
- `crates/htui/src/ui/tabs/chat/composer.rs`
- `crates/htui/src/ui/tabs/chat/mod.rs`
- `crates/htui/src/ui/tabs/chat/transcript.rs`
- `crates/htui/src/ui/tabs/registry.rs`
- `crates/htui/src/ui/tabs/settings/agents.rs`
- `crates/htui/src/ui/tabs/settings/connection.rs`
- `crates/htui/src/ui/tabs/settings/hierarchy.rs`
- `crates/htui/src/ui/tabs/settings/kinds.rs`
- `crates/htui/src/ui/tabs/settings/mod.rs`
- `crates/htui/src/ui/tabs/settings/prompt.rs`
- `crates/htui/src/ui/tabs/settings/qdrant.rs`
- `crates/htui/src/ui/tabs/skills.rs`
- `crates/htui/src/ui/text_field.rs`
- `crates/htui/src/ui/theme.rs`
- `crates/htui/tests/connection.rs`
- `crates/htui/tests/replay.rs`
- `crates/htui/tests/settings.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/auth/mod.rs` | 0, Authenticate, Logout, AuthCall |
| `crates/htui-agent/src/cli/mod.rs` | Line, Text, Json, 0, 0 |
| `crates/htui/src/app/action.rs` | Pass, Handled, Consumed |
| `crates/htui/src/app/state.rs` | emit, keymap, Ctx, theme, origin, ... |
| `crates/htui/src/app/update.rs` | _ctx, _key, on_key |
| `crates/htui/src/catalogue.rs` | phases, ProjectCatalogue, graph, graph, project, ... |
| `crates/htui/src/hierarchy.rs` | summary, workspace, HierarchySnapshot, projects, root_path |
| `crates/htui/src/qdrant_settings_info.rs` | 0, QdrantState, Unreadable, NotStored, Stored |
| `crates/htui/src/testkit.rs` | on_key, _ctx, key |
| `crates/htui/src/ui/overlay/migration_prompt.rs` | ctx, on_key, key |
| `crates/htui/src/ui/overlay/registry.rs` | ctx, key, on_key |
| `crates/htui/src/ui/overlay/workspace_switcher.rs` | enter, key, up, ctx, on_key, ... |
| `crates/htui/src/ui/tabs/backlog/detail/documents.rs` | _ctx, on_key, key |
| `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | cycle_prev, len, ctx, key, select, ... |
| `crates/htui/src/ui/tabs/backlog/detail/notes.rs` | _ctx, on_key, ctx, lines, key |
| `crates/htui/src/ui/tabs/backlog/detail/prompt.rs` | ctx, the_window_renders_the_cells_the_scrolled_whole_used_to, one_element_of_lines_is_one_rendered_row, on_key, key |
| `crates/htui/src/ui/tabs/backlog/list.rs` | selected, area, frame, height, view, ... |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | ctx, items, BacklogTab, reply, ctx, ... |
| `crates/htui/src/ui/tabs/chat/composer.rs` | theme, frame, composer, hint, render, ... |
| `crates/htui/src/ui/tabs/chat/mod.rs` | ctx, text, submit, answer, ctx, ... |
| `crates/htui/src/ui/tabs/chat/transcript.rs` | on_key, key |
| `crates/htui/src/ui/tabs/registry.rs` | on_key, ctx, key |
| `crates/htui/src/ui/tabs/settings/agents.rs` | selected, Running, key, answer_chooser, agent_id, ... |
| `crates/htui/src/ui/tabs/settings/connection.rs` | wants_requests, submit, on_editor_key, row, selected, ... |
| `crates/htui/src/ui/tabs/settings/hierarchy.rs` | 0, hint, project, 0, width, ... |
| `crates/htui/src/ui/tabs/settings/kinds.rs` | key, new_kind_fields, p, phase_at, theme, ... |
| `crates/htui/src/ui/tabs/settings/mod.rs` | wrapped, some_text, is_empty, text, area, ... |
| `crates/htui/src/ui/tabs/settings/prompt.rs` | integer_sentence, busy, down, target, stored, ... |
| `crates/htui/src/ui/tabs/settings/qdrant.rs` | on_scope_change, theme, busy, key, move_cursor, ... |
| `crates/htui/src/ui/tabs/skills.rs` | _key, on_key, _ctx |
| `crates/htui/src/ui/text_field.rs` | Cancel, Submit, Pass, FieldOutcome, Consumed |
| `crates/htui/src/ui/theme.rs` | title, selected, dim, Theme, accent, ... |
| `crates/htui/tests/connection.rs` | the_section_asks_for_the_connection_and_nothing_else |
| `crates/htui/tests/replay.rs` | _area, _key, _frame, on_key, _ctx, ... |
| `crates/htui/tests/settings.rs` | _ctx, key, _ctx, on_key, render, ... |

## Connected Communities

- **src/model +14 dirs** (40 cross-edges)
- **htui-agent/tests +13 dirs** (33 cross-edges)
- **src/ui · on_key** (13 cross-edges)
- **backlog/detail +3 dirs** (7 cross-edges)
- **src/store +7 dirs** (5 cross-edges)
- **tabs/settings · text** (4 cross-edges)
- **htui/tests +9 dirs** (4 cross-edges)
- **htui · bench_with · keymap** (3 cross-edges)
- **src/install +6 dirs** (2 cross-edges)
- **src/app +5 dirs** (2 cross-edges)
- **htui/src · MirrorAfterDelete** (2 cross-edges)
- **htui/src · DsnState** (2 cross-edges)
- **htui-orch/src +6 dirs** (1 cross-edges)
- **tabs/backlog · lines** (1 cross-edges)
- **htui-agent/tests +4 dirs · boxed** (1 cross-edges)
- **tabs/backlog · on_item_change** (1 cross-edges)
- **htui-store/src +4 dirs · close** (1 cross-edges)
- **ui/overlay** (1 cross-edges)
- **src/model +25 dirs** (1 cross-edges)
- **src/install +3 dirs** (1 cross-edges)
- **tabs/chat · pick** (1 cross-edges)
- **src/ui · apply** (1 cross-edges)
- **tabs/backlog · render** (1 cross-edges)
- **tabs/settings · render** (1 cross-edges)
- **htui-store/src +4 dirs · ItemSummary** (1 cross-edges)
- **tabs/settings · Field** (1 cross-edges)
- **htui · bench_with · connection** (1 cross-edges)
- **src/prompt +5 dirs** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-257")
explore(operation:"context", task:"understand tabs/settings +11 dirs", format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
