# MOD-22 - Complete a loopback OAuth login from a box the browser cannot reach (done, 2026-09-30)

**Requirements:** `R-AGT-9` (in-app login, amended by this item), `R-TUI-8` (Settings text input),
`R-NF-3` (off the UI task), `R-SEC-2` / `R-ID-7` (the pasted URL is a credential for one request).
`R-AGT-5` held as a constraint: the advertised redirect is read from the standard `redirect_uri`
parameter (RFC 8252 §7.3), with no code path per agent.
**Origin:** MOD-21 (found on its live proof, 2026-09-10).
**Artifacts:** plan
[`.claude/plans/mod-22-loopback-paste-back.plan.md`](../../../.claude/plans/mod-22-loopback-paste-back.plan.md)
(decisions D263-D274, 68 fact-checked claims: 55 verified, 11 partial, 2 falsified, all amended)
and blueprint `.claude/plans/mod-22-loopback-paste-back.blueprint.md` (D275-D290, findings F-1..F-16).
Routed as **plan** (0 criteria fired; C2 borderline and settled by the fact-check: MOD-15's masked
`TextField` already existed). Run in a TOOL-7 sandbox (`hr/MOD-22`). D262 was already claimed by two
plans (MOD-23, MOD-64), so this item's D-numbers may collide with a parallel run's (plan R-8).
**Decisions:** maintainer, 2026-09-30: route accepted; plan confirmed with OQ-1-OQ-6 on their
recommended answers (OQ-1: amend `R-AGT-9`; OQ-2: masked field; OQ-3: `state` always required;
OQ-4: status plus a redacted 80-column excerpt; OQ-5: `url` as a declared dependency; OQ-6: the live
proof is a manual TUI run). Review: "fix M-1 (bracketed paste) in MOD-22" and "apply all the rest";
re-review: "apply all but LOW-5".
**Commits:** `30f6a07`..`b403c26` (plan, fact-check, CONFIRM, blueprint), `f5987a1`..`65e55c5`
(T1-T4 and the `LOGIN_ENDED` follow-up), `86cf661`..`8b70c00` (review round), `bb700b3`
(`R-AGT-9` amendment), `9ff8518`..`5517bc9` (re-review round). No migration.

## What shipped

**The loopback module (T1).** New `crates/htui-agent/src/auth/loopback.rs`, whose module doc states
the credential rule where the code is (the precedent is `auth_live.rs`'s doc).
- `Advertised::from_auth_url` reads the `redirect_uri` of the link the adapter printed. It accepts
  only an `http` loopback (`127.0.0.0/8`, `::1`, `localhost`) and keeps its host, port, path and
  the link's `state`.
- `validate` checks a pasted `RedirectUrl` against it, first failure wins:
  - trim, cap at `PASTE_MAX` (8 KiB), `http://` added only when the paste starts with no scheme;
  - `http`, no userinfo;
  - host, port and path equal to the advertised ones;
  - an `error=` redirect named by its filtered value;
  - exactly one non-empty `code` and `state`, and `state` equal to the link's.
- Every refusal is a fixed sentence (15 `PasteError` variants). None quotes pasted text except the
  filtered OAuth `error` value (D285).
- `deliver` is one HTTP/1.1 `GET` over a raw `tokio::net::TcpStream`:
  - It connects only to the advertised host, and never resolves a name. For `localhost` it tries
    `127.0.0.1`, then `[::1]`.
  - It does not use `reqwest`: that client honours the system proxy and follows redirects, and
    either could carry the code off the box.
  - Limits: 2 s to connect, 15 s for the answer, 16 KiB read cap.
  - A connection closed after a complete status line counts as the answer (R-10). The adapter is
    killed and reaped as soon as `authenticate` returns.
- `ListenerReply::summary()` reports the status plus an 80-column excerpt: the page title, else
  the first body line. Echoes of the code or `state` are blanked in every spelling:
  - raw, or percent-encoded in either case, or with `+` for a space;
  - HTML character references;
  - a prefix cut off at the read cap.

  Control characters and Unicode format characters (the bidi overrides) are stripped.
- `RedirectUrl` is a redacting newtype modelled on `Dsn`. Its buffer is `Zeroizing`; it has a
  redacted `Debug`, no `Display`, no `Serialize`, and one private reader.
- `url` 2.5.8 and `zeroize` became direct dependencies of `htui-agent`, along with tokio's `net`
  feature. All three were already compiled, so `Cargo.lock` gained two dependency lines and no
  package.

**The protocol seam (T1).** `StoreRequest::AuthDeliver { url: RedirectUrl }` (88 → 91 counting the
drift the fact-check found: the tree held 90, not the pinned 88) and `AuthFrame::Delivered(ListenerReply)`
(10 → 11). `StoreReply` stays at 49 (pinned 48, stale).

**The worker (T2).** `AuthDeliver` goes into the live `run_auth` task as `AuthCommand::Deliver`,
beside `AuthOpen`: no new runtime slot, claim or task.
- `run_auth` keeps the newest advertised redirect from the adapter's own events. It never trusts a
  port the render side sends.
- It refuses, in order: `NO_LOOPBACK_REDIRECT`, `DELIVERY_IN_FLIGHT`, then the `PasteError`
  sentence.
- The GET runs as its own `select!` arm, so the wire, stderr and `x` keep being served during the
  request.
- When the flow ends: a delivery in flight is answered before the drain and the re-probe. A
  cancelled, declined or idle flow answers `LOGIN_ENDED` at once. A queued `Deliver` gets
  `LOGIN_ENDED`, like queued `Choose` and `Open` commands.
- `LOGIN_ENDED` and `NO_LOGIN_RUNNING` are shared `pub const`s, used by the worker and the pane.
- Limits reach the task through `AgentRuntime::with_deliver_limits` → `AuthArgs.deliver_limits`.

**The pane (T3).** In Settings > Agents, while a login is running and has advertised a loopback
redirect:
- A dim `redirect: 127.0.0.1:<port> · p pastes the address if the browser cannot reach it` line
  shows under the link.
- `p` opens a masked `TextField`, sized with the new `masked_with_capacity(PASTE_MAX)` so a paste
  never reallocates an unwiped buffer.
- `Enter` pre-checks the host and port only (`loopback::precheck`, no `Url` parse in the pane).
  It then sends `AuthDeliver` and shows `delivering to 127.0.0.1:<port>…`.
- Every other key goes into the field while it is open.
- The field is dropped, and its buffer wiped, on submit, `Esc`, `Cancelling` and every flow end.
- A refused delivery reopens a fresh, empty field.
- `Delivered` puts the listener's summary on the note line. MOD-21's existing `Completed` →
  re-probe → `Done` path finishes the login.
- The hint reads `o open link · p paste redirect · x cancel`.
- One new snapshot (115 → 116).

**End to end (T4).** `crates/htui/tests/auth.rs`
`p_paste_enter_delivers_to_the_listener_and_the_cell_reads_the_probes_verdict` drives `p`, the typed
address and `Enter` through `App::on_key` into a real loopback listener. It asserts the `GET`, the
note and then `logged in: ready`, and sweeps every rendered frame for the code sentinel. The sweep
was proven by temporarily drawing the field in clear, which made the test fail.

## Review round (`rust-reviewer`: approve with fixes)

No CRITICAL or HIGH findings, and no credential leak found.
- **M-1 (bracketed paste, `8b70c00`).** A paste made before `p` used to be replayed as keystrokes:
  digits switched tabs and `/` opened the Requirements filter, which is not masked, so `?code=…`
  could show in clear.
  - Bracketed paste is now on (`init`, resume from the editor) and off on every restore path
    (panic hook, guard, leaving for the editor).
  - `Event::Paste` is wrapped in `Zeroizing` on arrival and handed only to the top overlay, or to
    the active tab when no modal overlay is up. No keymap ever sees it.
  - Only a view that is capturing input takes it; otherwise it is dropped. Yes/no questions take
    no paste.
  - `TextField::on_paste` takes a masked paste only whole and only when it fits. `TextArea` and the
    composer take pastes too.
- **M-2 (`0678a74`).** `deliver`, `validate` and the worker's `Deliver` arm were split into smaller
  functions, with no behaviour change.
- **Lows:**
  - L-1: the `localhost` connect-order exposure is documented (PKCE bounds it).
  - L-2: `LOGIN_ENDED` waits for the flow's own last frame.
  - L-3: `NO_LOGIN_RUNNING` is a shared constant.
  - L-4: no `expect` in `settle`.
  - L-5: blanking covers every spelling, plus format characters.
  - L-6: the `validate` edge sentences, including `UnverifiableWithoutState` for a link that carried
    no `state`. `state` stays required (OQ-3).
  - L-7: `open_paste` names every variant.
  - L-8: the pane's local check reads only the host and port.
  - L-9: the cancel test proves the delivery was in flight, and the IPv6 fallback test holds its
    port's `127.0.0.1` twin.

## Re-review round (`rust-reviewer`: approve)

- R2-L1 (`7e064e7`): a refused delivery reopens the field.
- R2-L2 (`9ff8518`, `5672ab9`): `precheck` and `validate` agree on slashes before the host.
- R2-L3 (`213000c`): a paste before `p` opens the login's field, or is refused by the same sentence
  `p` would give. `TooLong` and `PASTE_DOES_NOT_FIT` are now told apart.
- R2-L4 (`d4f2887`): `TextArea::on_paste` inserts once. A 50 KB paste went from 113 s to 0.01 s.
- R2-L6 (`8f58a37`): an `Unsupported` bracketed-paste mode (the legacy Windows console) is
  tolerated, so it no longer panics in `init()`.
- A test pins that a paste under a modal overlay reaches no field (`5517bc9`).
- LOW-5 (a truncated tail over-blanked) was left as is by maintainer decision: it errs in the safe
  direction.

## Gates

At `5517bc9`, all run on the main thread:
- `cargo fmt --all -- --check` clean.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1`: 2883 passed, 0 failed, 26 ignored.

During the review round, `htui-store`'s `qdrant_live` failed (collection create timed out). The
sandbox Qdrant was measured taking 7 s to create a collection, against 41 s average in its own
telemetry. MOD-22 does not touch `htui-store`, and the test passed on every later run.

## Not done here

- **The live proof (OQ-6)** is a manual TUI run by the maintainer on the server-plus-laptop setup
  that found the gap:
  1. Start a login on the server.
  2. Open the link on the laptop.
  3. Copy the failed `http://127.0.0.1:<port>/?code=…&state=…` address.
  4. Press `p`, paste, `Enter`; expect `logged in: ready`.

  Record the result here when it has been run.
- **Windows** (the same paste-back from a Windows box) stays with MOD-16.
- **A "this box is remote" fact** stays with MOD-7's box registry. `p` is offered on every box
  (D271).
