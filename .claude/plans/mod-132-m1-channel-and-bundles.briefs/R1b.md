# MOD-132 M1 R1b — review fix round: htui-orch seed, collect and git verbs

Fix round over the M1 change set (`a335fdb2..3a5526c9`), from the `rust-reviewer` pass of 2026-11-16. **Confirm each
finding against the code before acting**. A finding that does not hold goes in `rejected_findings` with the reason.
Tests first: each fix gets a failing test before the code. Plan for reference: `.claude/plans/mod-132-m1-channel-and-bundles.plan.md`
(P5 seed, P6 collect, P7 bundle header, P9 host-executing paths). Read only the sections you need.

## Medium

- **M1 — a non-commit tip.** (`crates/htui-orch/src/target/collect.rs` ~663-671, ~740.) A target can answer
  `rev-parse` with a tree or tag hash and ship a matching bundle; every check passes, and `create_branch` (gix has
  no kind check) points `refs/heads/<local>` at a tree. After `connected`, require the tip to be a **commit** in
  `req.repo` (`git::has_commit` or an object-kind read). Add `CollectRefusal::NotACommit { tip }` and its sentence.
  Test: a canned bundle whose tip is a tree; another whose tip is an annotated tag.
- **M2 — a cancelled collect leaves its temp ref behind.** (`collect.rs` ~433-456, ~532-537.) The temp ref is deleted
  after `.run()`. If the future is dropped (cancel, timeout) or `before_cas` panics, the ref survives. Hold a guard
  whose `Drop` deletes the ref with the synchronous `git::delete_ref` (logging a failure), beside the quarantine
  guard. Test: drop the `collect` future mid-way, through the `collect_racing` seam (e.g. a `before_cas` that panics
  inside `catch_unwind`, or a future aborted while the seam blocks), and assert no `refs/htui-collect/*` ref remains.
  Also add `pub fn sweep_temp_refs(repo: &Path) -> Result<usize, IsolateError>`, which deletes every ref under
  `TEMP_REF_PREFIX` (M2 calls it at startup), with a test.
- **M3 — bind a consent to the local tip it rewrites.** (`collect.rs` ~159-168, ~714-722.) Add
  `pub from: Option<String>` to `Consent`. A consent applies only when `from` equals this call's
  `rewritten_from` (or the local tip when only paths are consented; `None` when the branch is new). If the local
  branch moved after the person saw the prompt, the outcome is `NeedsConsent` again. Test: consent for `L1`, then a
  local commit moves the branch to `L2`; collect returns `NeedsConsent { rewritten_from: Some(L2), .. }` and moves
  nothing.
- **M4 — quarantine the host object store.** (`collect.rs` ~652-662; `git.rs` `index_pack` ~1231.)
  - Index the pack into a quarantine object directory `<scratch>/<uuid>-objects` (`0700`), running `index-pack`
    and `connected` with `GIT_OBJECT_DIRECTORY=<quarantine>` and
    `GIT_ALTERNATE_OBJECT_DIRECTORIES=<repo's objects dir>` through `extra_env`. Find the objects dir with
    `git rev-parse --git-path objects` or gix. These are set after `SCRUBBED_ENV`'s removal, so they win.
  - Run the commit-kind check (M1) and the path diff (`name_only`) under the same env, or read through it.
  - Only when the decision moves the branch, migrate the `.pack`/`.idx` from the quarantine's `pack/` into
    `<objects>/pack/` (rename; copy-then-rename across filesystems), **then** write the branch.
  - The quarantine dir is removed on every exit, through the same guard as the temp ref.
  - Pass `--max-input-size=<cap>` to `index-pack`.
  - Decide whether the temp ref (P6 step 5) is still needed once objects stay out of the repo until the move. If it
    is not, drop it and say so in `deviations`, but keep `sweep_temp_refs` for refs left by earlier builds.
  - Tests: after `NeedsConsent`, `NotConnected` and an fsck refusal, `req.repo`'s pack count is unchanged and the
    quarantine is gone; after a move, the tip's objects are in the repo.
- **M5 — no credentials in the seeded remote URL.** (`crates/htui-orch/src/target/seed.rs` ~46-47, ~218-222.) Refuse a
  `remote_url` that carries userinfo with a password or token (`scheme://user:pass@host`, `scheme://token@host`
  for http(s)). An ssh `git@host:path` or `ssh://git@host/path` user without a password is fine. Add
  `PrepareRefusal::CredentialInUrl` (the sentence names the host only, never the secret). Tests: https with
  `user:token@`, https with `token@`, ssh `git@` accepted.
- **M6 — P9 widened (maintainer decision 2026-11-16).** (`collect.rs` ~61-99.) Match `.cargo/` as a directory at **any
  depth**, and `rust-toolchain`, `rust-toolchain.toml`, `package.json` and `.envrc` as basenames **anywhere**: each
  runs code when a tool is used in that directory. `Cargo.toml` and lockfiles stay off the list: build-time execution
  is the person's step, like any source change. Put that reasoning in the doc comment on `HOST_EXECUTING_PATHS`.
  Tests: `crates/x/.cargo/config.toml`, `sub/rust-toolchain.toml`, `packages/a/package.json`, `sub/.envrc` each need
  consent; `crates/x/Cargo.toml` does not.

## Low

- **L1 — matching beyond ASCII.** (`collect.rs` ~117-150.) A changed path with a non-ASCII byte, or one ending in `.` or
  space (NTFS), is listed when its Unicode-lowercased form (`str::to_lowercase`) with trailing `.`/space trimmed
  per segment matches a rule. Covers U+017F `ſ` and U+212A `K`. Test with `.huſky/x` and `CLAUDE.md.`.
- **L2 — control characters.** Target-supplied text reaches a person through `TargetGit.stderr` (`collect.rs`
  ~784-797, `seed.rs` ~282-294) and `NeedsConsent.config_paths`. Escape control characters (C0, DEL, C1, and
  U+2028/2029) where the sentences format stderr, as `\u{..}`. Add `pub fn display_path(&str) -> String` in
  collect.rs that does the same for a consent path; M2 renders with it. `config_paths` keeps the raw names, because
  the consent comparison uses them. Tests.
- **L4 — a reused seed checks its run branch.** (`seed.rs` ~344-358.) A dir with the same base is `reused` only if
  `refs/heads/<run_branch>` exists there. Otherwise it is refused with a sentence naming both, never re-seeded.
  Also add `pub fn` docs saying that an interrupted seed (no marker) is `Occupied` until the caller clears the dir
  (M2's cleanup).
- **L5 — `index_pack` is always strict.** (`git.rs` ~1231.) Remove the `strict: bool` parameter. If a test needs the
  lax form, it runs `git` itself.
- **L6 — a stdin clone failure propagates.** (`git.rs` ~500.) If `try_clone()` of the stdin file fails, return
  `IsolateError::Git`. Do not run with null stdin.
- **L8 — small items.**
  - Split `extra` once in `host_executing`, not per path.
  - Factor `Quarantine`, `git_in` and the last-stderr-line helper, duplicated between `seed.rs` and `collect.rs`,
    into one private home (e.g. `target.rs` or a `target/util.rs`).
  - Fix the `crates/htui-orch/src/lib.rs` ~31-32 doc ("M1 lands the bundle header parser"): it now has seed and
    collect.
  - Write the seed bundle `0600` (create the file first with the mode, then let `bundle_create` write it; or
    `set_permissions` straight after, before the channel reads it).
  - Pass `connected`'s `not` list on stdin (`rev-list --stdin`) rather than as argv.
  - Do not touch the `NameOnly.unreadable` change from R1: the reviewer accepted it.

## Files you own

- `crates/htui-orch/src/target.rs`, `crates/htui-orch/src/target/*.rs` (a new `util.rs` is allowed)
- `crates/htui-orch/src/isolate/git.rs`, `crates/htui-orch/src/lib.rs`
- `crates/htui-orch/tests/it/target_*.rs`, `crates/htui-orch/tests/it/main.rs`

## Check

`cargo test -p htui-orch --all-features --no-fail-fast --test it target_`, then
`cargo test -p htui-orch --all-features --no-fail-fast --lib isolate::git`, then clippy (`--all-features --all-targets`
and featureless) on `htui-orch`, then `RUSTDOCFLAGS='-D warnings' cargo doc -p htui-orch --no-deps --all-features`,
then `cargo fmt -p htui-orch --check`. The existing callers of `index_pack`, `name_only` and `run_captured` outside
`target/` (the MOD-9 paths in `isolate/real.rs`) must stay green: also run
`cargo test -p htui-orch --all-features --no-fail-fast --test it gix_isolator::`.
