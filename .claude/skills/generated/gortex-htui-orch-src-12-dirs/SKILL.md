---
name: gortex-htui-orch-src-12-dirs
description: "Work in the htui-orch/src +12 dirs area — 206 symbols across 32 files (74% cohesion)"
---

# htui-orch/src +12 dirs

206 symbols | 32 files | 74% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/acp/mod.rs`
- `crates/htui-agent/src/cli/mod.rs`
- `crates/htui-agent/src/driver.rs`
- `crates/htui-agent/src/error.rs`
- `crates/htui-agent/src/install/archive.rs`
- `crates/htui-agent/src/install/mod.rs`
- `crates/htui-agent/src/launch.rs`
- `crates/htui-agent/src/probe.rs`
- `crates/htui-core/src/model/overlap.rs`
- `crates/htui-core/src/model/quota.rs`
- `crates/htui-core/src/scrub.rs`
- `crates/htui-orch/src/engine.rs`
- `crates/htui-orch/src/fanout.rs`
- `crates/htui-orch/src/gate.rs`
- `crates/htui-orch/src/status.rs`
- `crates/htui-orch/src/verify.rs`
- `crates/htui-store/src/connect.rs`
- `crates/htui-store/src/dsn.rs`
- `crates/htui-store/src/embed.rs`
- `crates/htui-store/src/qdrant_settings.rs`
- `crates/htui-store/src/vector.rs`
- `crates/htui/src/agent_worker.rs`
- `crates/htui/src/lib.rs`
- `crates/htui/src/testkit.rs`
- `crates/htui/src/ui/overlay/registry.rs`
- `crates/htui/src/ui/tabs/backlog/detail/mod.rs`
- `crates/htui/src/ui/tabs/registry.rs`
- `crates/htui/src/ui/tabs/settings/connection.rs`
- `crates/htui/src/ui/tabs/settings/kinds.rs`
- `crates/htui/src/ui/tabs/settings/mod.rs`
- `crates/htui/src/ui/tabs/settings/prompt.rs`
- `crates/htui/src/ui/tabs/settings/qdrant.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/acp/mod.rs` | fmt, f, fmt, f, f, ... |
| `crates/htui-agent/src/cli/mod.rs` | f, fmt, fmt, f |
| `crates/htui-agent/src/driver.rs` | fmt, fmt, fmt, 0, fmt, ... |
| `crates/htui-agent/src/error.rs` | T, Result |
| `crates/htui-agent/src/install/archive.rs` | _path, make_executable, path, make_executable, link_at, ... |
| `crates/htui-agent/src/install/mod.rs` | fmt, f |
| `crates/htui-agent/src/launch.rs` | fmt, f, fmt, f, fmt, ... |
| `crates/htui-agent/src/probe.rs` | f, fmt |
| `crates/htui-core/src/model/overlap.rs` | fmt, f, fmt, f |
| `crates/htui-core/src/model/quota.rs` | fmt, f |
| `crates/htui-core/src/scrub.rs` | fmt, f |
| `crates/htui-orch/src/engine.rs` | fmt, f |
| `crates/htui-orch/src/fanout.rs` | f, fmt, fmt, f |
| `crates/htui-orch/src/gate.rs` | f, f, fmt, fmt |
| `crates/htui-orch/src/status.rs` | fmt, f |
| `crates/htui-orch/src/verify.rs` | f, fmt |
| `crates/htui-store/src/connect.rs` | f, fmt, f, fmt |
| `crates/htui-store/src/dsn.rs` | f, fmt |
| `crates/htui-store/src/embed.rs` | f, fmt |
| `crates/htui-store/src/qdrant_settings.rs` | fmt, f |
| `crates/htui-store/src/vector.rs` | f, fmt |
| `crates/htui/src/agent_worker.rs` | fmt, phase, f, caps, policy, ... |
| `crates/htui/src/lib.rs` | init_tracing, path |
| `crates/htui/src/testkit.rs` | fmt, f |
| `crates/htui/src/ui/overlay/registry.rs` | fmt, fmt, f, f, f, ... |
| `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | fmt, fmt, f, f |
| `crates/htui/src/ui/tabs/registry.rs` | fmt, f, f, fmt |
| `crates/htui/src/ui/tabs/settings/connection.rs` | fmt, f, fmt, fmt, f, ... |
| `crates/htui/src/ui/tabs/settings/kinds.rs` | f, fmt, fmt, f |
| `crates/htui/src/ui/tabs/settings/mod.rs` | f, fmt, f, fmt |
| `crates/htui/src/ui/tabs/settings/prompt.rs` | f, fmt, expected, Editor, target, ... |
| `crates/htui/src/ui/tabs/settings/qdrant.rs` | fmt, f |

## Connected Communities

- **htui-agent/src +7 dirs** (7 cross-edges)
- **src/model +14 dirs** (7 cross-edges)
- **htui/tests +14 dirs** (5 cross-edges)
- **src/app +5 dirs** (1 cross-edges)
- **src/isolate +10 dirs** (1 cross-edges)
- **tabs/settings +11 dirs** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-11")
explore(operation:"context", task:"understand htui-orch/src +12 dirs", format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
