---
name: gortex-src-isolate-10-dirs
description: "Work in the src/isolate +10 dirs area — 659 symbols across 14 files (91% cohesion)"
---

# src/isolate +10 dirs

659 symbols | 14 files | 91% cohesion

## When to Use

Use this skill when working on files in:
- `crates/htui-agent/src/install/layout.rs`
- `crates/htui-agent/tests/excerpt.rs`
- `crates/htui-agent/tests/probe.rs`
- `crates/htui-core/src/model/run.rs`
- `crates/htui-core/src/prompt/trim.rs`
- `crates/htui-core/src/store/traits.rs`
- `crates/htui-orch/src/isolate.rs`
- `crates/htui-orch/src/isolate/copy.rs`
- `crates/htui-orch/src/isolate/git.rs`
- `crates/htui-orch/src/isolate/real.rs`
- `crates/htui-orch/tests/gix_isolator.rs`
- `crates/htui-store/src/cache/mod.rs`
- `crates/htui-store/src/pg/read.rs`
- `crates/htui-store/src/writer.rs`

## Key Files

| File | Symbols |
|------|---------|
| `crates/htui-agent/src/install/layout.rs` | exists, path |
| `crates/htui-agent/tests/excerpt.rs` | body, root, write, path, fs_reader_skips_git_gitignored_binary_large_and_lockfiles_in_order |
| `crates/htui-agent/tests/probe.rs` | a_node_package_prefers_the_local_tree_and_reads_its_package_json_version |
| `crates/htui-core/src/model/run.rs` | run_step_id, mode, repo_id, path, base_ref, ... |
| `crates/htui-core/src/prompt/trim.rs` | Entry, rendered, source, record, weight, ... |
| `crates/htui-core/src/store/traits.rs` | step_trees, step |
| `crates/htui-orch/src/isolate.rs` | run, index, Git, cwd, FanoutSlot, ... |
| `crates/htui-orch/src/isolate/copy.rs` | check_source, linked_worktree_source, a_half_finished_copy_is_never_reusable, walk_error, not_a_git_checkout, ... |
| `crates/htui-orch/src/isolate/git.rs` | reconcile_parent, chunk, head_buffer_keeps_the_first_64_kib, path, copy_range_moves_every_object_between_odbs, ... |
| `crates/htui-orch/src/isolate/real.rs` | a_truncated_patch_leaves_the_next_repo_header_on_its_own_line, dir, cli, is_primary, reset_refuses_a_local_checkout_whose_head_moved, ... |
| `crates/htui-orch/tests/gix_isolator.rs` | run, cleanup, base, scope, trees, ... |
| `crates/htui-store/src/cache/mod.rs` | path, create_dir |
| `crates/htui-store/src/pg/read.rs` | step, step_trees |
| `crates/htui-store/src/writer.rs` | step, step_trees |

## Entry Points

- `crates/htui-orch/src/isolate/real.rs::cleanup_removes_every_tree_and_leaves_the_repo_untouched`
- `crates/htui-orch/src/isolate/git.rs::untracked_paths_base_tracks_sees_a_file_at_a_path_the_base_tracks_as_a_directory`
- `crates/htui-orch/src/isolate/copy.rs::a_dirty_source_is_reset_and_labelled_in_the_partial_before_it_is_finished`
- `crates/htui-orch/src/isolate/real.rs::worktree_prepare_writes_one_tree_per_repo_outside_every_repo`

## Connected Communities

- **src/model +14 dirs** (23 cross-edges)
- **src/isolate · excludes** (16 cross-edges)
- **src/install +6 dirs** (10 cross-edges)
- **htui-agent/tests +3 dirs · lock** (9 cross-edges)
- **htui-agent/tests +13 dirs** (6 cross-edges)
- **src/isolate · parse_version** (4 cross-edges)
- **htui-agent/tests +2 dirs · heartbeat** (3 cross-edges)
- **src/model +3 dirs · matches** (3 cross-edges)
- **htui-agent/tests +2 dirs · read** (2 cross-edges)
- **htui-orch/src · tail_buffer_keeps_the_last_64_k…** (2 cross-edges)
- **src/install · get_registry** (2 cross-edges)
- **htui-orch · new** (2 cross-edges)
- **src/isolate · budget_text** (2 cross-edges)
- **src/prompt +5 dirs** (2 cross-edges)
- **htui-agent/src +3 dirs** (1 cross-edges)
- **htui-store/tests +13 dirs** (1 cross-edges)
- **htui-agent/tests +4 dirs · json** (1 cross-edges)
- **htui-orch/src · spawn_and_wait** (1 cross-edges)
- **htui/tests +14 dirs** (1 cross-edges)
- **src/isolate · commit_at** (1 cross-edges)

## How to Explore

```
analyze(operation:"communities", id:"community-143")
explore(operation:"context", task:"understand src/isolate +10 dirs", format:"gcx")
relations(operation:"usages", target:{symbol:"crates/htui-orch/src/isolate/real.rs::cleanup_removes_every_tree_and_leaves_the_repo_untouched"}, format:"gcx")
```

_`format: "gcx"` returns the [GCX1 compact wire format](../../docs/wire-format.md) — round-trippable, ~27% fewer tokens than JSON. Drop it for JSON output; agents using `@gortex/wire` or the Go `github.com/gortexhq/gcx-go` package decode either._
