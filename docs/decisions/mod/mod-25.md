# MOD-25 - `htui` is online-only (done, 2026-09-16)

A box that cannot reach its configured Postgres shows that it is offline and browses its read-only
cache. It does **not** carry a second, writable life. Satisfies the maintainer decision of
2026-09-11 against `R-STO-1`, `R-STO-3..7`, `R-ID-3`, `R-ENT-7`, `R-HIS-1`, `R-AGT-4`, `R-PRM-4`,
`R-SKL-1`, `R-TUI-1`, `R-TUI-8`, `R-NF-2`.

Artifact: plan `.claude/plans/mod-25.plan.md` (routed **plan**, no PRD). Thirteen commits,
`9dcf4b2`..`ba4b82c`, on `main`.

## What this withdraws

ANA-10's verdict — "a box with no DSN becomes a *complete* box over a separate `local.sqlite`" — is
**concluded and therefore superseded rather than edited**. `docs/ANA-10.md` stays in the tree as the
analysis that was done and not taken.

**MOD-17, MOD-18 and MOD-19 are withdrawn, not done.** They must not be read as completed work:

- **MOD-17 - Local-only mode: a writable local store.** Withdrawn — there is no writable local
  store, so there is nothing to build. Its M3 was MOD-4's ordering blocker; that blocker is gone by
  removal, not by satisfaction.
- **MOD-18 - Adoption of local rows into a server.** Withdrawn — with no local rows there is nothing
  to adopt.
- **MOD-19 - In-process transition out of local-only.** Withdrawn — there is no local-only mode to
  transition out of.

`docs/ANA-10.md` §9.1's milestone set, §5.3's `local_migrations/`, §7.3's local mint and §5.4's
`local_setting` are all withdrawn with them.

## What was built

The disable is spelled as **one arm**. `Backend::writer()` answers `None` on `Backend::Offline`
(`crates/htui-store/src/backend.rs`), where before it built
`Writer::Buffered(BufferedWriter::new(cache))`. That seam was chosen because the code had already
reserved it: its own doc said *"a fourth variant that cannot write should be a one-arm change here
rather than a signature change at every call site."* Every one of the three production callers
already handled `None`, so the change is one line of behaviour and no change of shape. Reversing
MOD-25 is restoring that arm.

Three alternatives were weighed and rejected in the plan: a guard in `AgentRuntime::start` only
(leaves the seam advertising a write path no consumer accepts), a Cargo feature (the documented test
command is `--all-features`, so feature unification would re-enable the buffer under the standard
run and the refusal proof and the buffer proofs could not both be green), and `#[cfg]`-ing the
machinery out (violates "stays in the tree, compiling").

A chat off the server is refused with one sentence, `htui_store::DATABASE_UNREACHABLE`, carried by
`StoreError::Unreachable`. The user sees `store unreachable: this box browses its read-only cache and
starts no run` in the Chat body, and the status line prefixes the request name. **No new warning
surface was built**: the standing warning is the top bar's `offline · <age>`, which is `R-STO-4`'s
own word, and the refusal is what the chat says over it.

The two registry closures at `agent_worker.rs:736` and `:1742` now answer `REGISTRY_ON_SERVER_ONLY`
— the sentence the `matches!(Writer::Buffered(_))` guard below each of them already produced — so
their behaviour is byte-identical and the offline probe, install, login and plan tests pass
untouched. They now prove the closure rather than the guard; the contract they prove is the same.

**The upload side is deliberately still live.** `cache/refresh.rs:300` still calls `upload_pending`
on every refresh pass and `CacheStore::open` still seals orphans, so a box that buffered a chat under
an earlier build still lands those rows on its next successful connection. Only the write side is
disabled.

## Disable, do not delete

`Writer::Buffered`, `BufferedWriter`, `append_pending`, `upload_pending`, `BUFFERED_LABEL` /
`BUFFERED_NOTE`, the D42 chat-header branch and five tests stay in the tree, compiling and `pub`, for
one release so a reversal costs nothing. **`CLEAN-2` is minted to delete them** once the decision has
sat; its checklist line carries the full deletion list.

Five tests are parked with `#[ignore = "MOD-25: …"]` and their bodies left byte-for-byte, because
they are the evidence the reversal would need:

| Test | What it proved |
|---|---|
| `chat_offline.rs` `an_offline_chat_is_accepted_and_its_header_says_it_is_buffered` | D42's header |
| `chat_offline.rs` `an_offline_chat_writes_its_rows_to_the_pending_buffer_in_seq_order` | the buffer's line order |
| `chat_offline.rs` `a_buffered_chat_lands_in_postgres_on_the_next_connection` | **ANA-4 §11 criterion 12**, end to end |
| `chat_offline.rs` `an_offline_box_that_never_synced_this_user_refuses_and_says_so` | D33's `app_user` refusal — now unreachable from the shell, since `start()` refuses at the writer before `this_user()` |
| `agent_worker.rs` `a_buffered_writer_never_re_probes` | the re-probe skip |

**ANA-4 §11 criterion 12 is withdrawn *with the mode*, which is a different sentence from
"unproven".** MOD-2's close-out restated rather than silently dropped its claim on it; it was proven
end to end on 2026-09-08, and the proof is parked above rather than deleted.

## Requirements

`docs/REQUIREMENTS.md` had already been amended on 2026-09-11 (`R-STO-7` withdrawn, the eleven ANA-10
amendments reverted to online-only), so scope point (1) of the item was landed before this session.
One residue remained and was cleaned by explicit maintainer authorisation, the only edit to that
maintainer-owned file here: **`R-STO-6`** still cited withdrawn `R-STO-7`, `docs/ANA-10.md` §10.4 and
§12 criterion 13 — a measurement that was MOD-17's to take. It now reads that the budget "is
conditioned on a reachable server and binds no offline start (R-STO-4); that scope was decided, not
overlooked (maintainer, 2026-09-08)", which keeps the maintainer's decided-not-overlooked intent
while citing a live requirement instead of a withdrawn one.

## Deferred out of this item

**The masked DSN field is not built here.** The item claimed it from MOD-17, but MOD-15 owns building
and registering the Settings connection section and `SectionId("connection")`, `MaskedField` and
`TabAction::FocusSection` have **zero occurrences in `crates/`** — nothing exists to put a field
inside. Building the scaffolding here would have inverted the ownership the item itself states
("MOD-15 owns the Settings connection *section*; this item owns the credential *field* inside it").
**The field is owed to MOD-15's section** and MOD-15's checklist line now says so. Maintainer
decision, 2026-09-16.

## Unblocked by removal

- **MOD-4**'s build step 1 — ANA-10 §9.1 required MOD-17's M3 before it. MOD-4 is no longer blocked
  on anything, and **its M8 (`LocalStore` at graph parity, the four SQL ports, and
  `local_migrations/0002_orchestration_local.sql`) is struck from its scope**, which is the reason
  this item was run first.
- **MOD-13**, **MOD-15**, **MOD-23** — their create/edit paths for a server-less box no longer exist.

## Review

`rust-reviewer`: **sound, approve.** No CRITICAL, no HIGH. One MEDIUM and three LOW, all applied
before close-out:

- **M1** — `StoreError::Unreachable` displays as `store unreachable: {0}`, so a constant that also
  named the database made the rendered line say "unreachable" twice. The constant now carries only
  the half the variant does not (`79bd0b0`). Noted for whoever revisits it: a `crates/htui` test
  pins `htui-core`'s error `Display` format, and is the tripwire if that format moves.
- **L1** — six doc comments outside the plan's six rewritten sites still described the offline chat
  in the present tense, two of them in crates this item never named. Past tense now, and explicit
  that the upload side is live (`ba4b82c`).
- **L2** — the D33 case's `#[ignore]` reason said "the offline buffer is disabled", which is true but
  is not why that test cannot run; it never proved the buffer (`1eb7da4`).
- **L3** — the chat tab's module doc opened in the present tense about a behaviour the next sentence
  withdrew (`1eb7da4`).

The review also found two things the plan had wrong, both caught by implementers first: strict
equality against the bare constant was not achievable (the variant's prefix), and `recording_writer`
has three production callers rather than two — so the closure fix turned four tests green, not three.

One incidental fix: `README.md:99`'s top-bar table already claimed that offline "Reads come from the
mirror and there is no write path at all." That was **false** under the buffered build. It is now
literally true.

## Verification

`USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test
--workspace --all-features` → **1013 passed, 0 failed, 30 ignored** with Postgres live. Reconciles
against MOD-2's close-out baseline of 1017/0/25: −5 parked, +1 new refusal proof, 25 + 5 = 30.

`cargo clippy --workspace --all-features --all-targets -- -D warnings` clean; `cargo doc --workspace
--no-deps` clean; `cargo fmt --all --check` clean.

The new proof is `chat_offline.rs::an_offline_chat_is_refused_with_the_unreachable_warning`: the
refusal renders, `chat_steps()` is empty, and `pending/` holds no entry — neither an `.open` file nor
a sealed one. The parked set was run once with `-- --ignored` and Postgres live: all four compile,
all four run, all four fail. That is what "disabled" means.

## Commits

| Commit | What |
|---|---|
| `9dcf4b2` | the plan artifact |
| `b37a388` | `test(store)` the two seam tests flipped, red first |
| `5dd7e81` | `fix(store)` the `Offline` arm answers `None`; `DATABASE_UNREACHABLE` added and re-exported |
| `2f3a469` | `docs(store)` the offline write path is gone, the machinery is kept |
| `9f6c9f8` | `docs(readme)` the offline shell refuses a chat, it no longer buffers it |
| `e0f6622` | `docs(requirements)` `R-STO-6` cites a live requirement, not a withdrawn one |
| `f61a8b1` | `test(agent)` the chat refusal, red first |
| `177ba46` | `fix(agent)` an offline chat is refused, not buffered |
| `5024fd5` | `test(htui)` an offline chat is refused, and buffers nothing |
| `1b0a215` | `test(chat)` park the four proofs of the withdrawn offline buffer |
| `1eb7da4` | `docs(chat)` review findings L2 and L3 |
| `79bd0b0` | `fix(store)` the refusal sentence stops repeating "unreachable" (M1) |
| `ba4b82c` | `docs(core,agent,store,htui)` review finding L1 |
