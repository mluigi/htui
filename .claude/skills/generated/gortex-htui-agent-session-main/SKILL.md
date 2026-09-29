---
name: gortex-htui-agent-session-main
description: "Work in the htui-agent · session_main area — 206 symbols across 6 files (78% cohesion)"
---

# htui-agent · session_main

206 symbols | 6 files | 78% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/acp/client.rs`
- `crates/htui-agent/src/acp/fs.rs`
- `crates/htui-agent/src/acp/handshake.rs`
- `crates/htui-agent/src/acp/map.rs`
- `crates/htui-agent/src/acp/mod.rs`
- `crates/htui-agent/tests/cli_driver.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/acp/client.rs` | InboundTx, forward_write, send, inbound, 0, ... |
| `crates/htui-agent/src/acp/fs.rs` | a_sibling_directory_sharing_a_name_prefix_is_not_admitted, normalise, limit, cwd, line, ... |
| `crates/htui-agent/src/acp/handshake.rs` | handshake, timeout, io, settings |
| `crates/htui-agent/src/acp/map.rs` | stop_reason, stop_reasons_map_one_to_one_and_an_unknown_one_ends_the_turn, text |
| `crates/htui-agent/src/acp/mod.rs` | name, kill, open_calls, 0, FollowUp, ... |
| `crates/htui-agent/tests/cli_driver.rs` | a_stream_with_no_init_times_out_and_kills_its_child, what, assert_not_running, pid |

## Connected Communities

- **src/acp · map** (6 cross-edges)
- **htui-agent/src +3 dirs** (5 cross-edges)
- **htui-agent/tests +13 dirs** (3 cross-edges)
- **src/prompt +5 dirs** (3 cross-edges)
- **src/acp · a_missing_file_reads_as_the_emp…** (3 cross-edges)
- **src/model +14 dirs** (3 cross-edges)
- **htui/tests +14 dirs** (3 cross-edges)
- **htui-agent/tests +3 dirs · spawn** (2 cross-edges)
- **src/install +6 dirs** (2 cross-edges)
- **htui-agent · spec** (2 cross-edges)
- **htui-store/tests +13 dirs** (2 cross-edges)
- **htui-orch/src +6 dirs** (2 cross-edges)
- **htui-agent/src +6 dirs** (1 cross-edges)
- **backlog/detail +3 dirs** (1 cross-edges)
- **htui-agent/src +4 dirs · record** (1 cross-edges)
- **htui-agent/tests +3 dirs · lock** (1 cross-edges)
- **htui-agent/src +2 dirs · DriverEvent** (1 cross-edges)
- **src/model +25 dirs** (1 cross-edges)
- **src/isolate +10 dirs** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-4")
explore(operation:"context", task:"understand htui-agent · session_main", format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
