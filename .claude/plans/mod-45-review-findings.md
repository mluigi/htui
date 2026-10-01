# MOD-45 — rust-reviewer findings (review gate, 2026-10-01, over de177dd..d2803e7)

Verdict: approve with fixes. No CRITICAL/HIGH. All findings below are to be applied in the review
fix round (each re-verified against the tree first). Orchestrator dispositions are in **bold**.

1. **MEDIUM — Sentry egress.** `crates/htui/src/main.rs:81-83` with `provision/mod.rs:467-475`,
   `:442-451`: every `ProvisionExit::Failed` (exit 1) goes to the hardcoded GlitchTip via
   `sentry_anyhow::capture_anyhow`, carrying `user@host`, remote home, 20 journal lines, 20 worker-log
   lines and remote stderr tails. These are environment states, not crashes (MOD-41 E-1 rationale).
   **Apply: no Sentry capture for any `ProvisionExit` (both codes). Pin with a test if `main.rs` allows
   (e.g. extract the capture predicate into a fn and unit-test it).**
2. **MEDIUM — unbounded hang.** `remote.rs:98-101` (`ssh_argv`), `:139` (`run_piped` waits for EOF on
   the output pipes): only `ConnectTimeout=15`. **Apply: add `-o ServerAliveInterval=15 -o
   ServerAliveCountMax=4`; wrap each `remote.run` in `tokio::time::timeout` (PREFLIGHT 120 s →
   Refused; PREPARE an upload-sized bound, e.g. 120 s + payload_len / 1 MiB·s; INSTALL 300 s; VERIFY
   tries×pause + 120 s → Failed), via `Timings` so tests can shorten them; kill_on_drop reaps.**
3. **MEDIUM — false "verified".** `mod.rs:364` takes the DB-side baseline before PREPARE;
   `verify.rs:160-169` accepts any `last_seen_at` move. Old worker (re-provision with
   `--replace-credential`) or an operator TUI on the box can heartbeat in between. **Apply: take the
   baseline after INSTALL returns (the restart is done, the old process gone). Keep the
   "baseline None → not verified" warning path. Additionally, on the `--replace-credential` path,
   state in the success line/warning that verification on re-provision is best-effort if no stronger
   evidence exists. Adjust tests (S1 verifier log order: now PREPARE/INSTALL before baseline).**
4. **LOW — "sudo refused" too broad.** `mod.rs:428-441`. **Apply: `Some(255)`/`None` first → "ssh
   dropped before the privileged part (exit N)"; append `installed.last_stderr_line()` to the sudo
   sentence; in nopasswd mode don't mention "the password".**
5. **LOW — `printf` builtin assumption.** `script.rs:110`. **Apply: PREFLIGHT reports `htui.printf=
   builtin|external` (`case $(command -v printf) in printf) …`); `decide` refuses `SudoMode::Password`
   when external (sentence names posh/mksh-style shells and suggests NOPASSWD); T4 adds a logging
   `printf` stub scenario proving a refusal and no leak. Keep scripts backslash/single-quote free.**
6. **LOW — hand-written unit elsewhere.** `plan.rs:84-105` + PREFLIGHT's `/etc` check only.
   **Apply: `decide` refuses when `!facts.unit && facts.active == "active"` ("an htui-worker service
   is already running from a unit htui did not write …").**
7. **LOW — `StartOptions` derives Debug over the DSN.** `crates/htui-store/src/connect.rs:199-202`.
   **Apply (D314 amended: `connect.rs` joins the allowed htui-store files): hand-written redacting
   `Debug` (dsn shown as `Some(<redacted>)`/`None`), fix the stale doc comment, add a test that
   `format!("{opts:?}")` never contains the DSN.**
8. **LOW — vacuous test.** `remote.rs:197-217` `ssh_argv_carries_no_sentinel`. **Apply: rewrite it to
   run a full `run_with` over the scripted fake and pass every recorded command through `ssh_argv`,
   asserting no sentinel — or delete it if fully duplicated; prefer the rewrite.**
9. **LOW — raw remote text to the terminal.** `mod.rs:467-475`, `:522-526`, `:611-615`. **Apply: one
   sanitiser replacing control chars other than `\n`/`\t` (escape_default) used by `with_detail`,
   `stderr_tail`, `last_stderr_line` and the hostname; unit test with an ESC/OSC sequence.**
10. **LOW — prompt to stderr, input from /dev/tty.** `mod.rs:340-341`, `secret_prompt.rs`. **Apply:
    write the prompt and trailing newline to `/dev/tty` when it opens, else `err`.**
11. **LOW — upload not checksummed.** `script.rs:85-90`. **Apply: PREPARE takes the expected sha as
    an extra positional; after `cat >`, compare `sha256sum < "$tmp" | cut -d" " -f1` (no backslash,
    no single quote) and `exit 3`-style failure with a distinct sentence ("the upload to <dest> was
    corrupted"); use a distinct exit code (6) so it maps separately.**
12. **NIT — tmp left on signal.** `script.rs:82`, `:136`. **Apply: `trap` covering EXIT HUP INT TERM
    with `exit` in the signal handler.**
13. **NIT — docs.** `docs/htui-worker.md:284` "fish and csh are refused" → csh/tcsh refused, fish
    untested/likely works; "within 60 s" wording (`:320`, `:334`, `:377`, `mod.rs:473`) → compute
    (tries−1)×pause or say "about a minute". **Apply.**
14. **NIT — small.** `group` validated but unused → **keep validation (D296), fix the docs line to say
    it is checked only**; failed-start sentence suggests `--replace-credential` → **apply**;
    `Payload::new` → `format!("{digest:x}")` → **apply**; the `--replace-credential` hint ordering on
    the already-provisioned path → **apply (progress line first)**.
