---
name: gortex-src-app-5-dirs
description: "Work in the src/app +5 dirs area — 250 symbols across 14 files (75% cohesion)"
---

# src/app +5 dirs

250 symbols | 14 files | 75% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-orch/src/fake.rs`
- `crates/htui/src/app/action.rs`
- `crates/htui/src/app/mod.rs`
- `crates/htui/src/app/state.rs`
- `crates/htui/src/app/update.rs`
- `crates/htui/src/event_loop.rs`
- `crates/htui/src/keymap.rs`
- `crates/htui/src/terminal.rs`
- `crates/htui/src/ui/layout.rs`
- `crates/htui/src/ui/overlay/migration_prompt.rs`
- `crates/htui/src/ui/overlay/registry.rs`
- `crates/htui/src/ui/overlay/workspace_switcher.rs`
- `crates/htui/src/ui/tabs/registry.rs`
- `crates/htui/src/ui/tabs/skills.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-orch/src/fake.rs` | tick |
| `crates/htui/src/app/action.rs` | Overlay, SetScope, Open, workspace, Action, ... |
| `crates/htui/src/app/mod.rs` | register_all, app |
| `crates/htui/src/app/state.rs` | on_terminal_event, new, requests, top_bar, take, ... |
| `crates/htui/src/app/update.rs` | set_scope, _ctx, seq, render, workspace, ... |
| `crates/htui/src/event_loop.rs` | replies, app, run |
| `crates/htui/src/keymap.rs` | help_line, scope |
| `crates/htui/src/terminal.rs` | terminal_mut |
| `crates/htui/src/ui/layout.rs` | chrome, area |
| `crates/htui/src/ui/overlay/migration_prompt.rs` | id, is_modal, question, area, MigrationPrompt, ... |
| `crates/htui/src/ui/overlay/registry.rs` | create, id, is_empty, open, OverlayStack, ... |
| `crates/htui/src/ui/overlay/workspace_switcher.rs` | new |
| `crates/htui/src/ui/tabs/registry.rs` | area, Tab, title, active_mut, iter_mut, ... |
| `crates/htui/src/ui/tabs/skills.rs` | new |

## Connected Communities

- **src/model +14 dirs** (10 cross-edges)
- **htui/src · default_global** (8 cross-edges)
- **htui/tests +14 dirs** (3 cross-edges)
- **htui/tests +3 dirs** (3 cross-edges)
- **src/install +6 dirs** (2 cross-edges)
- **ui/overlay** (2 cross-edges)
- **htui/tests +6 dirs** (2 cross-edges)
- **src/install +4 dirs** (1 cross-edges)
- **htui/tests +1 dirs** (1 cross-edges)
- **htui · bench_with · connection** (1 cross-edges)
- **src/prompt +5 dirs** (1 cross-edges)
- **htui-agent/src +4 dirs · record** (1 cross-edges)
- **htui-agent/tests +13 dirs** (1 cross-edges)
- **tabs/settings +11 dirs** (1 cross-edges)
- **src/ui · apply** (1 cross-edges)
- **htui-agent/tests +3 dirs · lock** (1 cross-edges)
- **tabs/backlog · new** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-236")
explore(operation:"context", task:"understand src/app +5 dirs", format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
