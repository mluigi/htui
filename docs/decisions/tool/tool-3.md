# TOOL-3 - The Windows lint target cannot be built on this box (decided, 2026-09-28)

**Decision: accept the loss. `MOD-16` is the only Windows check.** No C toolchain is installed and
the cross-target lint line is not replaced by a scoped or vendored substitute.

**The problem.** `cargo clippy --target x86_64-pc-windows-msvc` dies in `ring`'s build script.
Cross-compiling `ring`'s C needs an MSVC-capable compiler; this box has `gcc` only. Re-verified
verbatim on 2026-09-28, unchanged since the item was raised on 2026-09-09:

```
warning: GNU compiler is not supported for this target
error occurred in cc-rs: failed to find tool "lib.exe": No such file or directory (os error 2)
```

Setting `AR_x86_64_pc_windows_msvc=llvm-lib` reaches one step further and dies on `not a COFF
object`. There is no `clang`, no `clang-cl`, no `cargo-xwin`, no `zig` and no `sudo`. The
`x86_64-pc-windows-msvc` standard library is installed; only the C toolchain is missing.

**The scope, stated precisely because it was wider than it looked.** The failure is **pre-existing
for `-p htui`** — `ring <- rustls <- sqlx-core <- sqlx <- htui-core <- htui-store` is a path that
predates MOD-20 — and MOD-20's `reqwest`/`rustls` extended the same failure to `htui-agent`. The
line was green for `htui-agent` before `ring` entered its graph and is green for **neither** crate
now. The two clauses of the README's "Checking the Windows-only code from Linux" section therefore
both named a command that cannot run.

**Why the loss was accepted.** The two alternatives both cost more than they buy here. Vendoring a
toolchain (`cargo install cargo-zigbuild` + `pip install ziglang`) restores a check that is not
worth a vendored C toolchain and a per-target rebuild of every C dependency on a box with no
Windows to run the result on. Scoping the line to a feature set that excludes TLS is cheaper but
leaves `install/http.rs` — one of the files with the most Windows-conditional surface — unlinted,
and a narrowed line is easy to mistake later for full coverage. Neither recovers what is actually
lost, which is compile-level visibility of `#[cfg(windows)]` branches on a machine that has no
Windows. `MOD-16` verifies those branches on real hardware, which is the check that actually
matters and was always going to run there anyway.

**What was amended, so nothing still asserts a gate that never went green.**

- `.claude/plans/mod-20-registry-adapter-install.plan.md` **D21** — retitled "written, reviewed by
  eye (lint cross-check unavailable, amended 2026-09-28 per TOOL-3), verified by MOD-16", and the
  sentence claiming "The Windows clippy line runs for `htui-agent` **and** `htui`" replaced with
  the actual state. D21's Windows code — `archive.rs`'s `cfg(unix)`/`cfg(not(unix))` arms,
  `Layout::promote`'s retry, and the `canonicalize` on both sides of the post-promote check — was
  reviewed by eye and never linted.
- The same plan's **Validation** block — the two `x86_64-pc-windows-msvc` lines are replaced by a
  comment recording that they do not run, the exact error, why, and the decision. The rest of the
  block is untouched and still a gate.
- `README.md` "Checking the Windows-only code from Linux" — the section now says up front that the
  command does not run on a Linux box without an MSVC-capable C compiler, that this project does
  not require one, and that it is not green for either crate. It keeps the command and says what
  restoring it would take, and states plainly that the two defects it once caught have no
  replacement. Its closing line already deferred the runtime facts to running the suite on
  Windows; that is now the only check there is.

**What MOD-16 inherits.** Every runtime fact D21 named, plus the compile-level branch coverage that
is now gone: the job object's kill-on-close guarantee, the `.cmd` shim `CreateProcess` refuses,
`CREATE_NO_WINDOW`, the `PATHEXT` lookup, `Layout::promote`'s retry actually clearing Defender's
lock, long paths past 260 characters, `system-proxy` reading the registry, and `fs4::available_space`
on a junction. `R-AGT-1` and `R-NF-3` are unaffected by the decision — the requirements were never
at fault, only the check in front of them.

**If this is revisited.** The decision is about this box, not about the value of the check. On a
box with `clang`, `cargo-xwin` restores the line for `htui-agent` alone; a full restoration needs
whatever makes `ring` build, which is the `sqlx` path too. Nothing in the code depends on the loss,
so a reversal is purely a toolchain change.
