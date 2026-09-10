# MOD-21 - In-app agent authentication (done, 2026-09-10)

**Requirements:** `R-AGT-9` (the contract), `R-AGT-1`, `R-AGT-4..6`, `R-TUI-8`, `R-NF-3`,
`R-SEC-2`, `R-ID-7`.
**Artifacts:** PRD `.claude/prds/mod-21-in-app-agent-auth.prd.md`, plan
`.claude/plans/mod-21-in-app-agent-auth.plan.md`, blueprint
`.claude/plans/mod-21-in-app-agent-auth.blueprint.md`.
**Commits:** `63ee3e9`, `d503a9a`, `7531d7b`, `81921e1`, `d24e79e`, `90ce9cc`, `050e4a4`, and this
close-out.

## What shipped

An agent that reports itself installed-but-unauthenticated is now logged in **from inside `htui`**.
In `Settings > Agents`, `a` on the highlighted row starts the agent's own ACP `authenticate` flow:
the row's methods arrive in the agent's own words ("Log in with Google", not `oauth-personal`),
`j`/`k` and `Enter` choose one, the pane streams what the agent says while it waits for the human,
`o` opens the link it printed, `x` cancels, and logout is offered where the agent advertises it.

`docs/ANA-4.md` §4.5's conclusion - "`htui` cannot log in non-interactively; ... the agent is left
disabled on that box until the user authenticates through the vendor's own flow" - is **reversed in
fact as well as in `R-AGT-9`**. It is proven on this box: `agy` went `unauthenticated` -> `ready`
through the app on 2026-09-10, the flow reaching `Completed` for `oauth-personal` in 21.6 s.

`htui` never reads, holds, transmits or stores the credential. It triggers the flow and observes
it; the vendor writes its own token where it keeps it, and what the app records afterwards is a
**re-probe**. Authentication is a fact about a box: the `agent` row is byte-identical before and
after, pinned by a test.

## The evidence the design rests on

No ANA preceded this item, so the PRD carries the research. Every fact below was measured live
against `agy_acp_server` 1.1.1 on this box during routing on 2026-09-09, not recalled.

- **`authenticate` blocks for the whole human round trip.** Sent live, no response in 45 s. There is
  no timeout, no progress notification and no cancellation verb in ACP v1, so D3 made the flow
  cancellable rather than timed, with a cap on *silence* rather than on elapsed time.
- **The URL arrives on stderr as prose, not through the protocol.** The run advertised
  `elicitation: { url: {}, form: {} }` - the schema's own answer, whose `ElicitationScope::Request`
  is documented for exactly this phase - and `agy` sent no `elicitation/create` anyway. What it
  wrote was one stderr line: `Open the following link to authenticate the ACP server: https://…`.
- **The adapter spawns a browser itself and that browser writes into the ACP channel.** With
  `$BROWSER` unset and no `DISPLAY`, the adapter's opener fell through to a terminal browser which
  emitted alt-screen escapes on **stdout** - the JSON-RPC stream. With `BROWSER=/bin/true` the same
  run was clean. This is D2, and it is the sharpest constraint in the item: for a full-screen TUI an
  uncontrolled `authenticate` both corrupts the protocol and fights for the terminal.
- **The API-key methods are not browser flows.** `gemini-api-key` with no key answers `-32602`:
  "The GEMINI_API_KEY environment variable must be set in the environment the ACP server is launched
  from…". D5 relays that rather than hiding the method; injecting the value is MOD-10's seam.
- **Only `terminal` methods are distinguishable.** The schema's `AuthMethod` is `#[non_exhaustive]`,
  but its `Agent` arm is `#[serde(untagged)]`, so an unrecognised `type` deserialises **as** `Agent`
  - measured in a scratch project during the plan fact-check. A wildcard arm therefore cannot hide a
  future kind; `hidden` holds terminal methods, which is the one kind the spec forbids sending.

## Decisions

The PRD gate settled **D1-D8** (2026-09-09, maintainer): the driver seam over the probe's process
shape; the browser neutralised and URLs opened only on an explicit key; cancellation instead of a
timeout; terminal methods refused for the MVP; API-key methods offered, attempted and relayed;
the probe as sole authority with a failed flow changing nothing; the chooser fed by the flow's own
`initialize` rather than by a widened `agent_box.probe` snapshot; and Settings-only for the MVP.

The plan settled **D9-D23**: the two-crate module split (`acp/auth.rs` speaks the wire,
`htui-agent::auth` holds the policy); `DriverCaps.authenticate` with a **default trait body that
refuses**, so a transport declines by saying nothing; one spawn serving both the method list and the
call, with the choice awaited *inside* the connection's foreground future; `handshake.rs`'s kill
discipline extended by two human exits; `AUTH_IDLE_CAP` measured from the last sign of life; the
stderr **tap**; URL detection as a scan rather than a parse; `BROWSER` neutralised on the auth spawn
only; the opener as a plain unsupervised `Command` with null stdio; four requests and a streamed
`AuthFrame` with the reply address switching at the choice; one claim with three holders; and the
Settings pane's states, keys and words.

## What the tree corrected

Four times the implementation contradicted the plan, and each correction is worth more than the
plan's original text:

1. **A dead adapter would have been reported as the agent's own refusal.** `block_task()` does not
   drop the foreground future when the child dies - the SDK synthesises `"Incoming transport
   closed"`. The planned mapping would have produced `Ok(Refused { message })` for a process that
   died, and at milestone 3 that writes a row for an answer that never came. Discriminated now with
   the SDK's public `is_incoming_transport_closed`.
2. **The SDK skips a corrupt stdout line rather than failing on it.** So the browser hijack's real
   failure mode is not a transport error but an answer thrown away and a login that waits forever.
   The regression pair asserts `Idle`, and the browser policy and the idle cap are one defence
   rather than two.
3. **The stderr reader was not lossy UTF-8 though its doc said so.** `BufReader::lines()` ends the
   reader on one invalid byte, freezing the tail, closing the tap and leaving the pipe undrained -
   so a link printed after a byte in a legacy encoding never arrives. Pre-existing; a login is the
   first flow where it costs the user something. Now `read_until` + `from_utf8_lossy`.
4. **The `AuthMethod` wildcard cannot fire** (above). The plan's first amendment during fact-check
   said to hide unknown kinds; that amendment was itself false and was replaced by a documented
   test.

## Review gate

`rust-reviewer` over the whole change set: **no CRITICAL, no HIGH**, one MEDIUM, five LOW, all
applied in `90ce9cc`.

The MEDIUM was another face of hazard H-1: `LiveAuth` drops its command sender and its
`CancellationToken` clone together, and a dropped token is not a cancelled one - so a runtime that
went away without `shutdown` left a flow nobody could cancel, bounded only by an idle cap that any
periodic stderr line resets forever. That is the orphan with an open loopback listener, reached by
a path the plan had not listed. A closed command channel now cancels.

Fixing the deferred-command finding opened a second window worth recording: `auth_command`'s
`self.auth = None` on a failed send fired *during* the re-probe, releasing the `ReprobeClaim` while
the flow was still writing its row. It is now `take_if(|live| live.task.is_finished())`.

One finding was **half refuted with a test**: an adapter that dies while the user is choosing does
not render "the agent ended" - the flow hangs, because this SDK does not give up on a foreground
future parked on a human rather than on a request. What ends a flow there is the pane's cancel or
the idle cap.

## Live proof

`crates/htui-agent/tests/auth_live.rs`, `#[ignore]`, run by the maintainer on 2026-09-10:

```
flow ended 21.6s, 1 stderr line(s), 1 link(s)
flow reached `Completed` for Authenticate("oauth-personal")
re-probed agent_box.probe.status = ready, credential tier = file, enabled = true
CONFIRMED: `/home/mluigi/.gemini/antigravity-acp/acp_token.json`
```

The seed's declared credential path was a reasoned guess that no box had ever tested. It is
**confirmed**: the vendor writes `acp_token.json` (510 bytes, mode 600) exactly where
`crates/htui-core/seeds/agent_agy.json` says, beside a `settings.json`. D23's failure-by-name branch
stayed unused and **no seed correction was needed**.

Afterwards `agy_live.rs` passes 4/4 against the logged-in server, including
`session_new_reports_its_config_options`, which was unreachable while unauthenticated: `session/new`
succeeds and reports its `config_options` (the permission modes `default`, `auto_edit`, `yolo`).
That is MOD-2's D64 coordinate, now learnable.

## Known, accepted, and handed on

- **A remote box cannot finish the flow from the app alone.** The agent's redirect listener lives
  inside the adapter process on *that box's* loopback, on a fresh ephemeral port per attempt, so a
  browser on another machine resolves `127.0.0.1` to itself and finds nothing. Found while running
  this item's own live proof. Discovered workaround, and the one that worked here: copy the failed
  `http://127.0.0.1:<port>/?code=…&state=…` out of the address bar and re-issue it on the box.
  Doing that **inside the TUI** is **MOD-22**, minted for it; `ssh -L` forwarding is the other
  route and did not work on this maintainer's setup.
- **The Windows half is written and reviewed by eye, never linted** - TOOL-3 means
  `cargo clippy --target x86_64-pc-windows-msvc` cannot build on this box at all. **MOD-16**
  inherits, by name: whether any Windows opener honours `BROWSER` and whether `cmd.exe /c exit 0`
  is a value it accepts (it is honoured only by an opener that word-splits the variable, and if
  `agy_acp_server.exe` treats it as one path it falls through to exactly the hijack D16 exists to
  stop); that `Start-Process -FilePath <url>` reaches the default browser from a `-NonInteractive`
  host under `CREATE_NO_WINDOW`; and that the job object reaps the auth child **with its listener**.
- **Terminal-typed auth methods are refused**, and `clientCapabilities.auth.terminal` stays `false`,
  which makes offering one a spec violation. Supporting them means suspending the TUI onto the real
  tty; no reachable agent needs it.
- **`elicitation` is still not advertised.** The schema intends it for exactly this hand-off and
  `agy` ignores it; advertising a capability with no UI behind it turns a working session into a
  hung one. An additive later step if a second agent uses it.
- **MOD-7** is the intended second caller (box registration), and **MOD-10** owns every credential
  value the API-key methods need.

## Tests

687 passed, 0 failed, 12 ignored workspace-wide on **Linux** with Postgres live
(`USERNAME=htui-ci`, TOOL-2), up from 589 at MOD-20's close-out. Two of the ignored are this item's
live cases; `cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo fmt --check` are clean, and `cargo doc -p htui` is clean after this item's own three rustdoc
errors were fixed at close-out - `cargo doc -p htui-agent` is still red for eight pre-existing
reasons, which is **CLEAN-1**.
