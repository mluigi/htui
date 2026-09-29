---
name: gortex-src-install-4-dirs
description: "Work in the src/install +4 dirs area — 188 symbols across 9 files (78% cohesion)"
---

# src/install +4 dirs

188 symbols | 9 files | 78% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/excerpt.rs`
- `crates/htui-agent/src/install/http.rs`
- `crates/htui-agent/src/install/layout.rs`
- `crates/htui-agent/src/install/manifest.rs`
- `crates/htui-agent/src/install/mod.rs`
- `crates/htui-agent/src/install/run.rs`
- `crates/htui-agent/tests/install.rs`
- `crates/htui/src/terminal.rs`
- `crates/htui/src/ui/layout.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/excerpt.rs` | panic_is_contained |
| `crates/htui-agent/src/install/http.rs` | new, install_crypto_provider, config |
| `crates/htui-agent/src/install/layout.rs` | root, max_age, id, id, new, ... |
| `crates/htui-agent/src/install/manifest.rs` | path, path, path, load, io, ... |
| `crates/htui-agent/src/install/mod.rs` | ctx, expected, Network, box_id, resolved, ... |
| `crates/htui-agent/src/install/run.rs` | stop_if_cancelled, job, previous, roll_back, layout, ... |
| `crates/htui-agent/tests/install.rs` | a_manifest_that_does_not_parse_reads_as_an_empty_one, the_manifest_is_written_through_a_temporary_and_a_rename, tmp, the_sweep_removes_residue_older_than_an_hour_and_nothing_younger, a_promote_onto_an_occupied_target_is_refused_by_name, ... |
| `crates/htui/src/terminal.rs` | install_panic_hook, restores_the_terminal |
| `crates/htui/src/ui/layout.rs` | width, area, height, centered |

## Connected Communities

- **src/isolate +10 dirs** (23 cross-edges)
- **src/install +3 dirs** (13 cross-edges)
- **src/install +6 dirs** (2 cross-edges)
- **htui-agent · download · fetch · mod (28)** (2 cross-edges)
- **htui-agent/tests +4 dirs · boxed** (2 cross-edges)
- **htui-agent · download · fetch · mod (26)** (2 cross-edges)
- **htui-agent/tests +7 dirs** (1 cross-edges)
- **htui-agent · plan** (1 cross-edges)
- **htui-agent · archive_file** (1 cross-edges)
- **htui-agent/tests +3 dirs · lock** (1 cross-edges)
- **htui-agent/tests +13 dirs** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-54")
explore(operation:"context", task:"understand src/install +4 dirs", format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
