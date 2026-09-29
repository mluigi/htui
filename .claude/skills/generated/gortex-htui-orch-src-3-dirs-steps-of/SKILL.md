---
name: gortex-htui-orch-src-3-dirs-steps-of
description: "Work in the htui-orch/src +3 dirs · steps_of area — 489 symbols across 7 files (88% cohesion)"
---

# htui-orch/src +3 dirs · steps_of

489 symbols | 7 files | 88% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/conformance.rs`
- `crates/htui-core/src/store/mem.rs`
- `crates/htui-orch/src/conformance.rs`
- `crates/htui-orch/src/engine.rs`
- `crates/htui-orch/src/fake.rs`
- `crates/htui-orch/src/verify.rs`
- `crates/htui-orch/tests/fixtures.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/conformance.rs` | patch, id, step_commits, expected_version, update_item, ... |
| `crates/htui-core/src/store/mem.rs` | value, key, set_app_setting |
| `crates/htui-orch/src/conformance.rs` | H, O, attempt, steps_of, H, ... |
| `crates/htui-orch/src/engine.rs` | 0, only_skipped_rows_above_the_choice_are_substitutions, run, contains, phase_fixture, ... |
| `crates/htui-orch/src/fake.rs` | verdict, after_done_writes_the_scripted_document_and_elapses_time, done_without_output, message, pass, ... |
| `crates/htui-orch/src/verify.rs` | started_at, outcome, finished_at, VerifyReport, output, ... |
| `crates/htui-orch/tests/fixtures.rs` | feature_with_verify_snapshot_matches |

## Connected Communities

- **src/model +14 dirs** (67 cross-edges)
- **htui-orch/src +6 dirs** (13 cross-edges)
- **htui-orch/src +2 dirs · FakeOrchestrator** (9 cross-edges)
- **htui-store/tests +13 dirs** (9 cross-edges)
- **htui-agent/tests +3 dirs · lock** (8 cross-edges)
- **htui-orch/src · FakeIsolator** (6 cross-edges)
- **htui-orch/src +2 dirs · steps** (6 cross-edges)
- **src/store +1 dirs** (3 cross-edges)
- **htui-agent/src +7 dirs** (3 cross-edges)
- **htui-orch/src · now** (2 cross-edges)
- **htui-orch/src · resolve_feat** (2 cross-edges)
- **src/model · rfc3339** (1 cross-edges)
- **src/prompt +5 dirs** (1 cross-edges)
- **htui-orch · step_at** (1 cross-edges)
- **htui-agent/src +4 dirs · DriverCaps** (1 cross-edges)
- **src/model +3 dirs · matches** (1 cross-edges)
- **htui/tests +14 dirs** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-115")
explore(operation:"context", task:"understand htui-orch/src +3 dirs · steps_of", format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
