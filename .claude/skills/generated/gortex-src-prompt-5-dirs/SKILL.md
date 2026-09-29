---
name: gortex-src-prompt-5-dirs
description: "Work in the src/prompt +5 dirs area — 315 symbols across 14 files (82% cohesion)"
---

# src/prompt +5 dirs

315 symbols | 14 files | 82% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/excerpt.rs`
- `crates/htui-agent/tests/excerpt.rs`
- `crates/htui-core/src/model/overlap.rs`
- `crates/htui-core/src/prompt/defaults.rs`
- `crates/htui-core/src/prompt/digest.rs`
- `crates/htui-core/src/prompt/estimate.rs`
- `crates/htui-core/src/prompt/excerpt.rs`
- `crates/htui-core/src/prompt/fixtures.rs`
- `crates/htui-core/src/prompt/render.rs`
- `crates/htui-core/src/prompt/settings.rs`
- `crates/htui-core/src/prompt/template.rs`
- `crates/htui-core/src/prompt/trim.rs`
- `crates/htui-core/tests/prompt_digest.rs`
- `crates/htui/tests/panic_hook.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/excerpt.rs` | req, caps, new, req |
| `crates/htui-agent/tests/excerpt.rs` | new, Behaviour, name, FakeRepoReader, behaviour, ... |
| `crates/htui-core/src/model/overlap.rs` | prefixes_come_from_the_excerpt_path_prefix |
| `crates/htui-core/src/prompt/defaults.rs` | the_review_body_states_the_front_matter_verbatim, the_judge_body_states_the_verdict_block_verbatim, command_queue_text_is_two_sentences, every_phase_body_is_wrong_role_for_judge_and_handoff_and_vice_versa |
| `crates/htui-core/src/prompt/digest.rs` | sha256_hex, text, sha256_matches_the_recorder_call |
| `crates/htui-core/src/prompt/estimate.rs` | DEFAULT, TokenEstimator, code_cpt, code_cpt, id, ... |
| `crates/htui-core/src/prompt/excerpt.rs` | path, entries, req, cap, req, ... |
| `crates/htui-core/src/prompt/fixtures.rs` | the_fixtures_carry_no_absolute_path_and_no_clock, spec, with_crlf, crlf, with_crlf_touches_every_body_and_nothing_else, ... |
| `crates/htui-core/src/prompt/render.rs` | bytes, excerpt_blocks_are_unindented_and_path_ordered, truncated, excerpt, files, ... |
| `crates/htui-core/src/prompt/settings.rs` | fmt, contains, other, f |
| `crates/htui-core/src/prompt/template.rs` | allowed_in, role |
| `crates/htui-core/src/prompt/trim.rs` | bare, content, reclaim, est, measure, ... |
| `crates/htui-core/tests/prompt_digest.rs` | the_record_hashes_the_rendered_excerpt_block |
| `crates/htui/tests/panic_hook.rs` | Boom, propose, version, name, request, ... |

## Connected Communities

- **src/model +14 dirs** (33 cross-edges)
- **src/prompt +2 dirs · phase_implement_attempt2** (9 cross-edges)
- **htui-core · wrap** (7 cross-edges)
- **htui-agent/tests +2 dirs · read** (6 cross-edges)
- **htui-agent/tests +13 dirs** (4 cross-edges)
- **src/prompt · entry** (4 cross-edges)
- **src/isolate +10 dirs** (2 cross-edges)
- **src/prompt +2 dirs · parse** (1 cross-edges)
- **src/install +4 dirs** (1 cross-edges)
- **htui-agent/tests +3 dirs · lock** (1 cross-edges)
- **src/model +25 dirs** (1 cross-edges)
- **src/prompt · Windowed** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-77")
explore(operation:"context", task:"understand src/prompt +5 dirs", format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
