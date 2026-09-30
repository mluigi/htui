# Plan: MOD-22 — complete a loopback OAuth login from a box the browser cannot reach

> **Status: draft 2026-09-30, fact-checked 2026-09-30 (see "Verified claims"), awaiting maintainer
> confirmation.** Every open question has a recommended answer. The plan adopts each one, so
> implementation is not blocked once they are confirmed.

**Source**: `HANDOFF.md` MOD-22 (the checklist entry at `:505-535`, from MOD-21). An agent's own
login flow redirects to a listener **inside the adapter process**, on the loopback of the box `htui`
runs on. `agy_acp_server`'s link carries `redirect_uri=http://127.0.0.1:<ephemeral port>/`, and the
port changes on every attempt. When `htui` runs on a server, the user's browser resolves `127.0.0.1`
to its own machine, finds nothing listening there, and the flow cannot finish. The workaround proven
by hand on 2026-09-10 was to copy the failed `http://127.0.0.1:<port>/?code=…&state=…` out of the
address bar and re-issue it on the box with `curl`. **Scope:** do that paste-back inside the TUI. It
is a field in MOD-21's login pane that:

- accepts the redirect URL;
- validates it: loopback host, the port the flow actually advertised, `code` and `state` present;
- issues the request to that port on the box `htui` runs on;
- reports what the listener said.

The login then completes through MOD-21's existing path. **Out of scope:**

- changing what the agent binds;
- a `redirect_uri` override;
- general port forwarding;
- Windows verification (MOD-16);
- a recorded "this box is remote" fact (MOD-7 records none).

**Requirements**: `R-AGT-9` (the in-app login contract, whose remote-box half this item finishes),
`R-TUI-8` (the Settings tab; the masked-field precedent), `R-NF-3` (no network I/O on the UI task),
`R-SEC-2` and `R-ID-7` (the pasted URL is a credential for the length of one request). `R-AGT-5` is a
constraint, not a deliverable. Nothing here names an agent, a vendor or a method. The redirect is
read from the standard OAuth `redirect_uri` parameter (RFC 6749 §4.1.1, RFC 8252 §7.3), not from a
vendor's sentence.

**Complexity**: Small to medium. The work is:

- one new `htui-agent` module (`auth/loopback.rs`) with its integration test file;
- one promoted dependency (`url`, already compiled through `reqwest`);
- two dependencies declared by `htui-agent` that the workspace already has (`zeroize`, and tokio's
  `net` feature);
- one `StoreRequest` variant (`AuthDeliver`) and one `AuthFrame` variant (`Delivered`);
- one `AuthCommand` variant and one extra `select!` arm in `run_auth`;
- a masked paste field in the login pane.

**No change** to:

- the flow itself (`htui-agent`'s `auth/run.rs`, `acp/auth.rs`, `AuthFlow`, `BrowserPolicy`);
- the re-probe or the outcome mapping;
- any store, migration, `.sqlx` file or `htui-orch`.

**Routing**: routed as **plan** by the maintainer. **Staffing: Opus 5.5 for every step (plan,
fact-check, architect, implementers, verifiers and reviewer `rust-reviewer`). Fable is not used.**
Ultracode for the implementers only, one workflow per task, with verify fan-out per round. The
architect and the reviewer stay plain agents.

**Numbering**: decision IDs continue the global sequence of `.claude/plans/*.md`. The highest in the
tree is **D262** (`mod-23-agent-registry-editing.blueprint.md` §8). This plan's decisions are
**D263…D274**. Another sandbox planning at the same time may claim the same range (R-8). Risks are
**R-1…R-10**, open questions **OQ-1…OQ-6** and tasks **T1…T4**. All of these are local to this plan
and are cited as "MOD-22 R-n" etc. outside it. MOD-21's local decisions are cited as "MOD-21 Dn".

**Base**: `hr/MOD-22` at `f5de3d4`. Every line number below is the file's line at `f5de3d4`, before
any edit.

**Tooling note**: the Gortex daemon reported this checkout as INACTIVE but still answered symbol,
source, summary and usages queries; those answers are cited. Line ranges of Markdown, TOML and test
text were read with `grep`/`sed`. The fact-check pass re-reads each claim at its line.

---

## Open questions for the maintainer

- [ ] **OQ-1 — `R-AGT-9` says `htui` "does not read, hold, transmit or store the credential".**
      Paste-back does **hold** an authorization `code` for one request and **transmits** it once, to
      the agent's own listener on loopback. The token the agent then mints never touches `htui`.
      **Recommended:** accept. The main thread amends `R-AGT-9` with one sentence: "When the browser
      cannot reach the agent's loopback listener, the app relays the browser's redirect, which
      carries a one-use authorization code, to that listener once and holds it for that request only
      (MOD-22)." D273's module doc states the same bound in code. **Alternative:** keep the
      requirement literal and drop the item. The `curl` workaround then stays the only route, which
      is the gap `R-AGT-9` was written to close.
- [ ] **OQ-2 — Masked or plain field.** **Recommended (D272): masked.** `TextField::masked()` draws
      `•` per grapheme plus a count (`crates/htui/src/ui/text_field.rs:249-356`). Its `text()`
      answers `None` (`:181-187`), so the only way out of the buffer is `take()` (`:202-205`), and
      the buffer is `Zeroizing` (`:45-57`). "The pane must not echo it" then holds by construction,
      as it does for the DSN (`R-TUI-8`) and the Qdrant key (`settings/qdrant.rs:151`). Cost: the user
      cannot read back what they pasted. Each refusal names the rule it broke (wrong port, stale
      `state`, no `code`), which is what a user would have looked for. **Alternative:** a plain field
      while typing, emptied on submit. It is easier to eyeball, but a code sits on screen, in
      terminal scrollback and in screen recordings until the user presses `Enter`.
- [ ] **OQ-3 — `state` when the link carried none.** The item requires `code` and `state` present.
      OAuth makes `state` RECOMMENDED rather than REQUIRED (RFC 6749 §4.1.1), so an adapter that
      sends none makes every paste fail. **Recommended (D265):** keep the item's rule. `state` is
      always required, and it must equal the link's `state` when the link carried one. `agy`'s link
      carries one (MOD-21's fixture `LINK` models it, `crates/htui/src/agent_worker.rs:6902`), and
      relaxing the rule later is additive. **Alternative:** require `state` only when the link
      carried one.
- [ ] **OQ-4 — How much of the listener's answer to show.** **Recommended (D268):** the status line
      plus a short excerpt: the HTML `<title>`, else the first text line. The excerpt is at most 80
      columns, and the pasted `code`/`state` values are blanked if the listener echoes them. For a
      `3xx`, the pane shows the `Location` **host** only. This is "reports what the listener said"
      without printing a page. **Alternative:** status line only. Safer still, but "400 Bad Request"
      with no reason leaves the user guessing.
- [ ] **OQ-5 — `url` as a declared dependency.** **Recommended (D264, D267):** promote `url = "2.5"`
      the way `zeroize`, `hmac` and `shell-words` were promoted. 2.5.8 is already compiled as a
      `reqwest` 0.13.5 dependency (`Cargo.lock:7113-7114`; listed by `reqwest`), so nothing new is
      downloaded. Percent-decoding a `redirect_uri` nested inside another URL's query, and comparing
      bracketed IPv6 hosts, are exactly the parts a hand-written parser gets wrong. **Alternative:**
      `form_urlencoded` (also compiled) plus hand splitting.
- [ ] **OQ-6 — The live proof.** **Recommended:** a manual TUI run by the maintainer on the
      server-to-laptop setup that found the gap. The steps are under "Validation → live check", and
      the result goes into the decision doc as MOD-21's did. There is no new `#[ignore]` test: the
      deliverable is the TUI path, and `auth_live.rs` drives `htui-agent` without a pane.
      **Alternative:** extend `crates/htui-agent/tests/auth_live.rs` with an `HTUI_LIVE_PASTE=1` mode
      that reads the redirect from stdin and calls `loopback::deliver` itself.

---

## Summary

MOD-21 already does everything but the last hop:

- It spawns the adapter with its listener (`AgentRuntime::auth_start`, `agent_worker.rs:1533-1608`).
- It streams the adapter's stderr, and the link, to the pane as `AuthFrame::Line`/`AuthFrame::Url`
  (`run_auth`, `agent_worker.rs:2860-3026`; `auth_frame`, `:3030`).
- It keeps the flow alive while a human is in a browser.
- It re-probes when `authenticate` returns.

What is missing is a way to hand the listener the browser's redirect when the browser is on another
machine.

**Where the advertised port comes from.** No component records it today. `first_url` scans stderr
for the first `http(s)://` substring and deliberately parses nothing
(`crates/htui-agent/src/auth/url.rs:1-12`, `:34`). The worker forwards each `AuthEvent::Url` as it
arrives (`run.rs:171` `forward`, `run_auth`'s events arm) and keeps none. The pane keeps only the
newest link, for `o` (`AuthState::Running.url`, `settings/agents.rs:254-293`). The port lives in that
link's `redirect_uri` query parameter, percent-encoded. MOD-22 parses it (D264): authoritatively in
`run_auth`, and again in the pane so it can offer the key and pre-check a paste.

**The hop.** The pane gets a `p` key and a masked field (D270, D272). On `Enter`, the section runs the
pure validation (D265) and sends `StoreRequest::AuthDeliver { url: RedirectUrl }` (D266). The runtime
forwards it into the live flow as `AuthCommand::Deliver`. `run_auth` validates again against its own
record of the advertised redirect. It then issues **one** plain HTTP/1.1 `GET` over a
`tokio::net::TcpStream` to that loopback port (D267), from a new arm of the same `select!` that owns
the flow, so the wire, stderr and `x` keep being served (D269). It answers `AuthFrame::Delivered` with
the listener's status and a redacted excerpt (D268). The adapter, now holding its code, answers
`authenticate`. From there MOD-21's path runs unchanged: re-probe, `AuthFrame::Done`, `Agents`
re-read, and the cell reads the probe's verdict.

## Data flow

```
key p (Running, advertised redirect known)      settings/agents.rs  — UI task, no I/O
  └─ masked TextField ← pasted chars (no bracketed paste: one KeyEvent per char)
Enter
  └─ TextField::take() → RedirectUrl(Zeroizing<String>)          field dropped / reset (wiped)
  └─ loopback::validate(&url, &advertised)  (pure)               refusal → Action::Error(sentence),
  │                                                               field reset, nothing sent
  └─ ctx.request(StoreRequest::AuthDeliver { url })              pane: "delivering to 127.0.0.1:P…"
store worker loop (runtime arm, store_worker.rs:1959-1983)
  └─ AgentRuntime::serve → auth_command("auth_deliver", AuthCommand::Deliver { url, reply })
run_auth task (agent_worker.rs:2860-3026)       — off the UI task
  ├─ events arm: AuthEvent::Url → Advertised::from_auth_url → `advertised` (newest wins)
  └─ commands arm: Deliver → validate again (authoritative) → Delivery (url dropped here)
       └─ delivery arm: loopback::deliver(Delivery, DeliverLimits)
            TcpStream::connect(127.0.0.1:P) → "GET /?code=…&state=… HTTP/1.1" → read ≤ 16 KiB
            → ListenerReply { target, status, reason, said, location_host }   (Delivery dropped)
       └─ frames.reply(&reply, Auth(Delivered(reply)))  |  Failed { "auth_deliver", sentence }
adapter process: its listener got the code → it answers `authenticate`
  └─ MOD-21 unchanged: wire returns Completed → re-probe → upsert → AuthFrame::Done → pane Idle
```

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D263 | **An extension of MOD-21's running flow, not a new flow.** One key (`p`), one request (`StoreRequest::AuthDeliver`), one frame (`AuthFrame::Delivered`) and one command (`AuthCommand::Deliver`) go **into the live `run_auth` task**. There is no new runtime slot, no new claim and no new task: the flow's `LiveAuth` (`agent_worker.rs:312-327`) already holds the re-probe claim and the cancel token, and the listener exists only while that flow runs. A deliver with no live flow is refused by `auth_command`'s existing `"no login is running"` (`:1622-1637`). The reply addressing is `AuthOpen`'s: it is answered once, at its own `seq` (`store_worker.rs:329-333`, `:965-971`), so `App::is_fresh`'s kind-keyed check (`app/state.rs:328-332`) passes it without touching the `AuthChoose` stream. It passes only while no newer `AuthDeliver` from the same origin has replaced its `seq` in the `(origin, kind)` index (`app/state.rs:310-311`), which is why D270 refuses `p` while a delivery is in flight. | The item: "the login then completes through MOD-21's existing path, so nothing here re-implements `authenticate`, the re-probe, or the outcome". `AuthOpen` is the same shape, a side request into the live flow, and has been reviewed. |
| D264 | **The advertised redirect is the `redirect_uri` query parameter of the link the adapter printed.** `loopback::Advertised::from_auth_url(&str) -> Option<Advertised>` does the following. (1) It parses the link with `url::Url` and reads the first `redirect_uri` from `query_pairs()`, which percent-decodes it. (2) It parses that value as a URL. (3) It accepts it only if the scheme is `http` and the host is a loopback: an IPv4 literal in `127.0.0.0/8`, `::1`, or the name `localhost` (ASCII case-insensitive). (4) It records `host`, `port_or_known_default()`, `path` and the link's own `state` if present. `https`, any other host, a missing or unparseable `redirect_uri`, and a non-URL are all `None`. **`run_auth` is the authority.** It updates `advertised` on every `AuthEvent::Url` that parses and keeps the newest (the port changes per attempt; the HANDOFF measured 39879, then 50651). **The pane holds the same value**, computed from the same `AuthFrame::Url`, to decide whether `p` is offered and to pre-check a paste. No DNS lookup ever happens. | RFC 8252 §7.3 is the loopback-redirect convention every native OAuth client uses, so reading the standard parameter is not vendor code (`R-AGT-5`). The worker re-derives the value from the adapter's own events rather than trusting a port the render side sends (MOD-23 D247's rule: the boundary does not trust the UI). |
| D265 | **Validation rules** (`loopback::validate(&RedirectUrl, &Advertised) -> Result<Delivery, PasteError>`), applied in order, the first failure wins. (1) Trim whitespace. Reject empty (`Empty`) and longer than 8 KiB (`TooLong`). (2) If the text has no `://`, prefix `http://`: Chrome and Firefox copy the scheme, but a hand-trimmed paste may lack it. (3) `url::Url::parse` or `NotAUrl`. (4) The scheme is `http`, or `NotHttp`. (5) No userinfo, or `HasUserinfo`. (6) The host equals the advertised host (IP literals compared as parsed, `localhost` case-insensitively), or `WrongHost { advertised }`. (7) `port_or_known_default()` equals the advertised port, or `WrongPort { pasted, advertised }`. (8) The path equals the advertised path, or `WrongPath { advertised }`. (9) If the query has an `error` parameter, `BrowserError { error }`: the value is cut to 64 chars from `[A-Za-z0-9._-]`, since it is the authorization server's code such as `access_denied`, not a secret. (10) Exactly one non-empty `code`, or `MissingCode` / `RepeatedParameter("code")`. (11) Exactly one non-empty `state`, or `MissingState` / `RepeatedParameter("state")`. (12) If the advertised link carried a `state`, it is equal, or `StaleState` (a paste from an earlier attempt). The fragment is ignored and never sent. On success, `Delivery` carries the connect target(s), the `Host` header, the path and query as `Zeroizing<String>`, and the `code`/`state` values (also `Zeroizing`), which are used only to blank echoes in D268. **Every `PasteError` `Display` is a fixed sentence** naming at most the advertised host and port, the pasted port and the rule, for example `the address is for port 50651; this login is listening on 39879`. None ever quotes pasted text. | These are the item's three checks, made exact. The path and state checks stop the field from becoming a generic "GET anything on loopback" tool, and catch the one mistake the HANDOFF observed: the port and `state` change on every attempt. |
| D266 | **`RedirectUrl` is a redacting newtype**, modelled on `htui_store::Dsn` (`crates/htui-store/src/dsn.rs`): `pub struct RedirectUrl(Zeroizing<String>)`, `Clone` (because `StoreRequest` derives `Clone`, `store_worker.rs:103`), hand-written `Debug` → `RedirectUrl(<redacted>)`, no `Display`, no `Serialize`, no `Deref`, constructor `RedirectUrl::new(String)`, and the text readable only inside `htui_agent::auth::loopback` (a `pub(crate)` accessor). `Delivery`'s `Debug` prints `host:port` only. `Advertised`'s `Debug` prints host, port and path, and `state: <redacted>`. `StoreRequest`'s own rule demands this: "No variant may carry a secret as a plain `String`" (`store_worker.rs:90-102`). | The request enum, its envelope and every test's `{reply:?}` derive or print `Debug`. The type makes the leak impossible rather than merely avoided. |
| D267 | **Transport: one plain HTTP/1.1 `GET` over `tokio::net::TcpStream`, not `reqwest`.** `loopback::deliver(Delivery, DeliverLimits) -> Result<ListenerReply, DeliverError>`. It connects to the advertised IP literal. For `localhost` it tries `127.0.0.1` and then `[::1]` on connection-refused, and never resolves a name. It writes `GET <path>?<query> HTTP/1.1\r\nHost: <advertised host>:<port>\r\nUser-Agent: htui\r\nAccept: text/html, text/plain, */*\r\nConnection: close\r\n\r\n` from a `Zeroizing<Vec<u8>>`. It reads until EOF, `BODY_CAP` (16 KiB) or the response deadline. It **never follows a redirect**. `DeliverLimits { connect: 2 s, response: 15 s }` in production is injected by tests, as `AuthFlow.idle` is (`crates/htui-agent/src/auth/mod.rs`). `run_auth` itself does not inject `idle`: it hard-codes `AUTH_IDLE_CAP` (`agent_worker.rs:2888`). So the worker's `DeliverLimits` reach `run_auth` through a new `AuthArgs` field, set by `auth_start` (D269). `htui-agent` declares tokio's `net` feature (`crates/htui-agent/Cargo.toml:24` lacks it). `net` is already compiled: tokio's lock entry pulls `mio` and `socket2` through `reqwest`/`hyper`. | Three reasons. First, the workspace's `reqwest` is built with `system-proxy` (`Cargo.toml:90-91`), and a loopback request routed through a configured proxy would hand the code to a third process. Second, its default redirect policy is followed (`crates/htui-agent/src/install/http.rs:142`), and a listener's `302` to a vendor "success" page would carry the flow off the box. Third, loopback needs no TLS, and the request is one line: this is `curl http://127.0.0.1:P/?…`, which is what the maintainer proved. A raw socket has no proxy setting to honour. |
| D268 | **What the listener said.** `ListenerReply { target: String /* "127.0.0.1:39879" */, status: u16, reason: String, said: Option<String>, location_host: Option<String> }` (`Debug`, `Clone`, `PartialEq`) and `summary() -> String`. Examples: `127.0.0.1:39879 answered 200 OK: "Authentication successful"`, `127.0.0.1:39879 answered 302 Found, redirecting to example.com`. `said` is the HTML `<title>` if there is one, else the first non-blank line of the body with tags stripped. Control characters are dropped, whitespace is collapsed, and the result is cut to 80 chars with `…`. **Any occurrence of the pasted `code` or `state` value, raw or percent-decoded, is replaced by `…`.** A `Transfer-Encoding: chunked` body is de-chunked within the cap. `DeliverError` has four variants, each a fixed sentence naming `target` and nothing else: `NothingListening` ("nothing is listening on 127.0.0.1:39879 — the login may have ended; x cancels it and a new attempt listens elsewhere"), `Timeout { after }`, `NotHttp`, and `Io { kind }` (the `io::ErrorKind`, never the OS message). A `4xx`/`5xx` answer is a `Delivered`, not a failure. The adapter decides what it means, and the flow keeps running either way. | OQ-4. The item says "reports what the listener said", and an error page's title is usually the one useful sentence. Blanking echoes costs one `replace` and closes the last route by which the code could reach the screen. |
| D269 | **The worker.** `AuthCommand::Deliver { url: RedirectUrl, reply: ReplyAddr }`. `serve` routes `StoreRequest::AuthDeliver { url }` through `auth_command("auth_deliver", …)` beside `AuthOpen` (`agent_worker.rs:1109-1115`). In `run_auth`, four things change. (a) The events arm inspects `AuthEvent::Url` **before** `auth_frame` consumes it and updates `advertised: Option<Advertised>`. (b) The commands arm handles `Deliver`. With no `advertised`, it answers `Failed { "auth_deliver", NO_LOOPBACK_REDIRECT }`. With a delivery already in flight, it answers `Failed { …, DELIVERY_IN_FLIGHT }`. If validation fails, it answers `Failed { …, <PasteError sentence> }`. Otherwise it stores `(reply, Box::pin(deliver(delivery, limits)))` in a `delivering: Option<…>` slot. `limits` is the new `AuthArgs` field: `DeliverLimits::default()` in production, and milliseconds in T2's silent-listener cases. The `RedirectUrl` is consumed by `validate` and dropped at the end of the arm. (c) A new `select!` arm, guarded `if delivering.is_some()`, awaits it and answers `Auth(Delivered(reply))` or `Failed { "auth_deliver", <DeliverError sentence> }` at the deliver's own address. The wire, stderr, the choice and `x` are all still polled meanwhile: `run.rs`'s "the loop is the whole of it" rule (`crates/htui-agent/src/auth/run.rs:9-12`). (d) After the loop breaks, a delivery still in flight is answered **before** the queued-command drain and the re-probe. If the loop broke on the cancel token (`x`, shutdown, idle), the delivery is dropped and answered `"this login has ended"`. Otherwise it is awaited to its own deadline. By then the child has been killed **and reaped**: `acp::auth::run` calls `guard.kill_and_reap()` before `authenticate`'s future returns. The socket therefore ends at once, and the reply is only what the listener wrote before the adapter answered `authenticate` (R-10). Awaiting a cancelled flow's delivery would hold `Cancelled` for the full response deadline whenever the listener is not the child's own. T2's `cancel_is_served_while_a_delivery_is_in_flight` uses such a listener. Queued `Deliver`s get `"this login has ended"`, as queued `Choose`/`Open` do (`:2951-2966` in `run_auth`). `run_auth`'s one `tracing` call (the `Kept` arm) takes no field from any of this. | `R-NF-3`: the section does pure string work only, and the socket lives in the flow's own task. Awaiting inline in the commands arm, as `Open` does, would stop polling the wire for up to 15 s and delay `x`. The post-loop await keeps "every request the pane spent is answered exactly once" (`run_auth`'s own comment on queued commands). |
| D270 | **The pane.** `AuthState::Running` gains `advertised: Option<Advertised>`, `paste: Option<TextField>` and `delivering: bool`. **Keys.** `p` while `Running` and not `cancelling`: while `delivering` it gives `Action::Error(DELIVERY_IN_FLIGHT)`, because a second `AuthDeliver` would supersede the first's `seq` in `App::is_fresh` and its answer would be dropped as stale (D263). Otherwise, with `advertised` it opens `TextField::masked()`, and without it `Action::Error(NO_LOOPBACK_REDIRECT)`. With the field open, every key goes to `TextField::on_key` (`text_field.rs:119-174`). `Submit`: `take()` into `RedirectUrl`, then the local `validate`. A refusal gives `Action::Error(sentence)` and the field is **replaced by a fresh masked one** (the old buffer is wiped on drop), so a re-paste needs no second `p`. On success the section sends `AuthDeliver`, sets `paste = None` and `delivering = true`. `Cancel` (Esc) sets `paste = None`. `Pass` (a `CONTROL` chord such as `ctrl-c`) gives `Handled::Pass`. Everything else is `Consumed`, including `q`, `x`, `o`, `h` and `l`. `captures_input` (`agents.rs:1776-1778`) also answers `true` while `paste.is_some()`, so `SettingsTab::on_key` (`settings/mod.rs:323-331`) hands `h`/`l` to the field. **Frames.** `Delivered(reply)` sets `delivering = false` and `notice = reply.summary()`. `Failed { "auth_deliver" }` sets `delivering = false`, and the pane otherwise stays as it was, joining `auth_open`'s exception in `on_reply` (`:1972-1975`). Any flow end (`Done`, `Refused`, `Cancelled`, `Idle`, `Failed`) sets `AuthState::Idle`, which drops the field and wipes it. **Render** (`auth_pane`, `:784-834`): the existing lines and `link: …`. Then, when `advertised`, a dim `redirect: 127.0.0.1:39879 · p pastes the address if the browser cannot reach it`. While the field is open: `paste the address the browser could not open (127.0.0.1:39879):` then `› ` plus `TextField::line`, which shows dots and a count. While `delivering`: `delivering to 127.0.0.1:39879…`. **Hints:** `HINT_AUTH_RUNNING` (`:162`) becomes `o open link · p paste redirect · x cancel` (41 chars). A new `HINT_PASTING` is `Enter sends · Esc cancels`, chosen in `hint()` (`:1087-1114`) while the field is open. | One field, one key, and MOD-21's pane states unchanged in meaning. "Clear on submit, cancel and flow end" is carried by the types (`Option<TextField>` dropped, `Zeroizing` buffer), not by remembering to call `clear()`. |
| D271 | **Offered on every box whenever the flow advertised a loopback redirect.** There is no "is this box remote" test. | MOD-7 records no such fact (HANDOFF `:531-533`), and any guess (SSH env vars, `DISPLAY`) would be wrong somewhere. A same-machine user simply never presses `p`, and the extra pane line tells a remote user the key exists. |
| D272 | **The field is masked** (OQ-2). | See OQ-2. |
| D273 | **The credential rule, stated where the code is.** The canonical text is the `//! # The credential rule (R-SEC-2, R-ID-7)` section of `crates/htui-agent/src/auth/loopback.rs`, following the precedent of `crates/htui-agent/tests/auth_live.rs:68-75` ("What it must never print"). Its text is given below under "The credential rule". There are one-paragraph cross-references in four places: the `StoreRequest::AuthDeliver` doc, the `AuthCommand::Deliver` doc, `run_auth`'s doc, and `settings/agents.rs`'s module doc ("Since MOD-22 …", after the MOD-21 and MOD-23 paragraphs). | The item: "MOD-21's `auth_live.rs` module doc is the precedent for stating it where the code is." |
| D274 | **Not changed, on purpose:** `crates/htui-agent/src/auth/{run,url,browser}.rs`, `crates/htui-agent/src/acp/**`, `AuthFlow`, `AuthEvent`, `BrowserPolicy`, `AUTH_IDLE_CAP`; the re-probe and `AuthFrame::Done`'s meaning; every store, migration, `.sqlx` file and `htui-orch`; bracketed paste (the text widgets deliberately do not build it, `text_field.rs:10`); `auth_live.rs` (OQ-6); `docs/**`, `HANDOFF.md`, `DECISIONS.md`, `REQUIREMENTS.md` (main thread; OQ-1's amendment is theirs). | Scope. |

### The credential rule (D273, verbatim module-doc text for `auth/loopback.rs`)

```text
//! # The credential rule (`R-SEC-2`, `R-ID-7`)
//!
//! The address a user pastes here carries an OAuth authorization `code`; for the length of one
//! request it is a credential, and everything below is written around that:
//!
//! - **Never logged.** No `tracing` field, no panic message and no `Debug` or `Display` prints it:
//!   [`RedirectUrl`] prints `RedirectUrl(<redacted>)`, [`Delivery`] and [`Advertised`] print a host
//!   and a port, and every [`PasteError`] and [`DeliverError`] is a fixed sentence naming a host, a
//!   port or a rule — never a byte that was pasted.
//! - **Never persisted.** No row, no file, no keyring entry, no mirror.
//! - **Never on a frame that outlives the request.** It crosses the store worker once, inside
//!   `StoreRequest::AuthDeliver`, is moved into the running flow's task, and is dropped when the
//!   one `GET` resolves. What comes back, [`ListenerReply`], holds a status, a reason, a host and
//!   an excerpt with the pasted `code` and `state` blanked.
//! - **Never echoed.** The pane's field is masked, is emptied on submit, on cancel and when the
//!   flow ends, and nothing it draws afterwards names more than `127.0.0.1:<port>`.
//! - **Sent to one place.** The loopback port the running flow advertised, over a plain socket no
//!   proxy setting can reroute, following no redirect.
//!
//! Outside the rule, and stated rather than hidden: `url::Url`'s parse buffer and the kernel's
//! socket buffers are not wiped, and an adapter that prints the callback on its own stderr is shown
//! by MOD-21's pane as every stderr line is.
```

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A redacting credential newtype: hand `Debug`, no `Display`, `Zeroizing`, crate-private accessor | `htui_store::Dsn` | `crates/htui-store/src/dsn.rs` (the struct and its `Debug`) |
| A masked, zeroizing, `take()`-only input | `TextField::masked` | `crates/htui/src/ui/text_field.rs:45-57`, `:91-97`, `:181-187`, `:202-205`; used at `settings/connection.rs:365`, `:464` |
| A side request into the live login, answered once at its own `seq`; a refusal that leaves the pane alone | `StoreRequest::AuthOpen` / `AuthCommand::Open` / `AuthFrame::Opened` | `store_worker.rs:324-333`; `agent_worker.rs:299-304`, `:1109-1115`, the `Open` arm of `run_auth`; `settings/agents.rs:1972-1975` |
| A deadline injected by the caller so tests use milliseconds | `AuthFlow.idle` / `AUTH_IDLE_CAP` | `crates/htui-agent/src/auth/mod.rs` |
| A loopback `TcpListener` fixture answering HTTP in a test | the MOD-20 install fixture | `agent_worker.rs` `Fixture` (`:3974-4091`); `crates/htui-agent/tests/install.rs` |
| A scripted adapter printing a link to stderr and answering `authenticate` | `AGENT_SH`, `login_row`, `login_store`, `login_runtime` | `agent_worker.rs:6923-6944`, `:6978`, `:7056`, `:7090`; `crates/htui/tests/auth.rs:93-114` (`AGENT_SH`), `:121-238` (helpers) |
| Section tests over `SectionBench` with a chooser already answered | `chooser_over`, `typed`, `render_section` | `crates/htui/tests/settings.rs:1841`, `:2698`, `:93` |
| Promoting an already-compiled transitive crate, with a one-line reason | `zeroize`, `hmac`, `shell-words` | root `Cargo.toml` `[workspace.dependencies]` |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `Cargo.toml` | edit | T1 | `url = "2.5"` in `[workspace.dependencies]`, with its reason (OQ-5) |
| `Cargo.lock` | update | T1 | `htui-agent` lists `url` and `zeroize` (no new package) |
| `crates/htui-agent/Cargo.toml` | edit | T1 | `url`, `zeroize`; tokio `net` added to the `[dependencies]` feature list (`:24`) |
| `crates/htui-agent/src/auth/loopback.rs` | **new** | T1 | D264–D268, D273 |
| `crates/htui-agent/src/auth/mod.rs` | edit | T1 | `pub mod loopback;` plus a module-doc line |
| `crates/htui-agent/tests/loopback.rs` | **new** | T1 | validation table, redaction, `deliver` against a loopback listener |
| `crates/htui/src/store_worker.rs` | edit | T1 | `StoreRequest::AuthDeliver { url: RedirectUrl }`; `AuthFrame::Delivered(ListenerReply)`; `name` → `"auth_deliver"`; the no-runtime arm (`:1344-1361`); the runtime routing arm (`:1959-1983`); the name tests (`:3456-3476`, `:3540-3557`) |
| `crates/htui/src/testkit.rs` | edit | T1 | the runtime or-pattern (`:291-294`) |
| `crates/htui/src/ui/tabs/settings/agents.rs` | edit | T1, T3 | T1: the one `AuthFrame::Delivered(reply) => self.notice = Some(reply.summary())` arm that `on_auth_frame`'s exhaustive match needs. T3: D270 |
| `crates/htui/src/agent_worker.rs` | edit | T2 | D269, `AuthCommand::Deliver`, the `serve` arm, `run_auth`, its tests; `AGENT_SH` gains `FIXTURE_WAIT_FOR` |
| `crates/htui/tests/settings.rs` | edit | T3 | pane cases; the hint assertions at `:2023`, `:2581` |
| `crates/htui/tests/auth.rs` | edit | T3, T4 | T3: the hint assertion at `:491`. T4: the end-to-end paste case and its fixture's `FIXTURE_WAIT_FOR` |
| `crates/htui/tests/snapshots/settings__agents_paste_redirect.snap` | **new** | T3 | the open field (dots and count, the redirect line, the pasting hint) |

**Not touched:** everything in D274.

## Tasks

**Order.** **Wave 1:** T1. **Wave 2:** T2 and T3 in parallel, each in its own worktree. Merge T2 then
T3, and re-run both gates on the merged tree with `--test-threads=1`. **Wave 3:** T4, which needs
T2's worker and T3's pane.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T1 | `Cargo.toml`, `Cargo.lock`, `crates/htui-agent/Cargo.toml`, `crates/htui-agent/src/auth/loopback.rs` (new), `crates/htui-agent/src/auth/mod.rs`, `crates/htui-agent/tests/loopback.rs` (new), `crates/htui/src/store_worker.rs`, `crates/htui/src/testkit.rs`, `crates/htui/src/ui/tabs/settings/agents.rs` (the one arm) | Wave 1, alone |
| T2 | `crates/htui/src/agent_worker.rs` | Wave 2, independent of T3 |
| T3 | `crates/htui/src/ui/tabs/settings/agents.rs`, `crates/htui/tests/settings.rs`, `crates/htui/tests/auth.rs` (the `:491` hint assertion only), `crates/htui/tests/snapshots/settings__agents_paste_redirect.snap` (new) | Wave 2, independent of T2 |
| T4 | `crates/htui/tests/auth.rs` | Wave 3, after T2 and T3 |

**Independence of T2 and T3, justified by file sets and build coupling.** T2 ∩ T3 = ∅: T2 is
`agent_worker.rs` alone, and T3 touches no `src` file but `agents.rs`. Build coupling has two
conditions, binding on the implementers:

1. **Both compile against T1 alone.** T1 lands every shared type and variant: `RedirectUrl`,
   `Advertised`, `validate`, `deliver`, `ListenerReply`, `StoreRequest::AuthDeliver`,
   `AuthFrame::Delivered`, and the `agents.rs` arm. Until T2 merges, `AuthDeliver` reaches
   `AgentRuntime::serve`'s wildcard (`"not a chat request"`, `agent_worker.rs:1117-1120`), which
   T3's section tests never exercise, because they run over `SectionBench` with no runtime.
2. **Neither adds a shared type.** Sentences the pane and the worker share (`NO_LOOPBACK_REDIRECT`,
   `DELIVERY_IN_FLIGHT`) are `pub const`s in `loopback.rs`, landed by T1. So is every accessor
   either task reads: the pane draws `127.0.0.1:39879` from an `Advertised`, and T2 asserts
   `ListenerReply`'s `status` and `said`. T1 therefore lands a public `Advertised::target()` (or
   `host()`/`port()`) and public `ListenerReply` fields. Otherwise T2 and T3 would each add one to
   `loopback.rs`, a file neither owns.
3. **T2 keeps its test helpers' signatures.** `store_worker.rs`'s own test (T1's file) calls
   `crate::agent_worker::tests::auth::login_store` and `login_runtime` (`store_worker.rs:3581`,
   `:3593`). T2 adds its loopback link as a new constant beside `LINK` (`agent_worker.rs:6902`),
   which `a_deliver_before_any_loopback_redirect_is_refused` still uses unchanged.

T1 ∩ T3 = {`agents.rs`} and T3 ∩ T4 = {`tests/auth.rs`}. Both overlaps are serial by wave order.

Every implementer prompt carries these rules:

- nothing branches on an agent's name (`R-AGT-5`; the `extensibility.rs` sweep is the check);
- **no test, assertion message, `tracing` call or panic prints a pasted URL.** Tests use the
  sentinels `CODE-SENTINEL-4f1c` and `STATE-SENTINEL-9a2e` and assert their absence;
- **commit incrementally, staging your own paths only.** There is no stash on a shared tree;
- verify your gate with `--test-threads=1` on the real tree after your merge.

### Task 1: the loopback module and the protocol seam (D264–D268, D273; D263's variants)

- **Tests first**, in `crates/htui-agent/tests/loopback.rs`. Integration tests are used because the
  API is public, as `tests/auth.rs` does for MOD-21. Advertised:
  - `advertised_is_read_from_a_percent_encoded_redirect_uri` uses `https://auth.example.invalid/o?client_id=c&redirect_uri=http%3A%2F%2F127.0.0.1%3A39879%2F&state=S1` and gets host `127.0.0.1`, port 39879, path `/`, state `S1`.
  - `advertised_accepts_localhost_and_the_ipv6_loopback`
  - `advertised_is_none_for_https_a_lan_host_a_name_no_redirect_uri_or_no_url`
  - `the_newest_link_is_not_this_modules_business`: a pure-function sanity case; newest-wins is T2/T3's.

  Validation, as one table test per rule of D265 (1)–(12):
  - `a_valid_paste_becomes_a_delivery_for_the_advertised_target`
  - `a_paste_without_a_scheme_is_read_as_http`
  - `wrong_port_names_both_ports_and_nothing_else`
  - `stale_state_is_refused`
  - `an_error_redirect_is_named_by_its_error_value`
  - `every_paste_error_sentence_is_free_of_pasted_text`: each variant is built from input containing
    both sentinels, and each `to_string()` is checked to contain neither.

  Redaction:
  - `redirect_url_debug_is_redacted`
  - `delivery_and_advertised_debug_print_host_and_port_only`

  Deliver, against a `tokio::net::TcpListener` bound to `127.0.0.1:0`:
  - `deliver_sends_one_get_with_the_pasted_path_query_and_the_advertised_host`: the request line and
    the `Host` and `Connection: close` headers are asserted byte for byte.
  - `deliver_reports_the_status_and_the_title_of_an_html_answer`
  - `deliver_reports_the_first_text_line_when_there_is_no_title`
  - `deliver_decodes_a_chunked_body`
  - `deliver_blanks_an_echoed_code_and_state_in_the_excerpt`
  - `deliver_cuts_the_excerpt_at_eighty_columns`
  - `deliver_does_not_follow_a_redirect_and_names_the_location_host`: the listener accepts exactly
    one connection.
  - `deliver_to_a_closed_port_is_nothing_listening`: bind, read the port, drop the listener.
  - `deliver_times_out_on_a_silent_listener`: `DeliverLimits` in milliseconds.
  - `a_non_http_answer_is_not_http`
  - `localhost_falls_back_to_the_ipv6_loopback`: skips with a printed reason when `[::1]` cannot be
    bound.

  In `store_worker.rs` tests:
  - the two name tables gain `(AuthDeliver, "auth_deliver")`;
  - `an_auth_deliver_request_debugs_without_its_url`;
  - `auth_deliver_without_a_runtime_is_refused_by_name`: the no-runtime arm.

  The first commit is red, with `todo!()` bodies in `loopback.rs` only.
- **Action.** Three pieces:
  - The module, with its module doc: D273's section plus a short "what this is" preface citing
    RFC 8252 §7.3 and this item.
  - The dependency changes.
  - The two variants, documented. `AuthDeliver`'s doc says it is served inside the live flow and
    answered once at its own `seq`, and points to the credential rule. `Delivered`'s doc says a
    `4xx` is an answer, not a failure. Also: `name`, the two or-patterns in `store_worker.rs`, the
    one in `testkit.rs`, and the one-line `agents.rs` arm. Two existing docs in `store_worker.rs`
    move with them. The no-runtime arm's comment counts "MOD-21's four login ones" and "all
    fifteen" (`:1340-1341`). `AuthFrame`'s "Nothing here can hold a credential" (`:1211-1214`)
    gains the `Delivered` excerpt, blanked by D268.
  - The public surface T2 and T3 consume (condition 2 below): `Advertised`'s target accessor,
    `ListenerReply`'s fields and `summary()`, `validate`, `deliver`, `DeliverLimits`, and the two
    `pub const` sentences. `auth/mod.rs` already has a `url` module (`pub mod url;`, `:20`). Inside
    `auth/mod.rs`, `url::Url` therefore names that module, not the crate (compile probe: E0425), and
    must be spelled `::url::Url`. In `loopback.rs`, `use url::Url;` resolves to the crate.
- **Validate.**
  - `cargo fmt --all -- --check`
  - `cargo test -p htui-agent --all-features --test loopback -- --test-threads=1`
  - `cargo test -p htui --all-features --lib store_worker -- --test-threads=1`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo tree -p htui-agent -i url` shows 2.5.8 only.
  - `git diff --stat f5de3d4 -- Cargo.lock` adds no `[[package]]`.

### Task 2: the worker (D269)

- **Tests first**, in `agent_worker.rs`'s `#[cfg(unix)] mod auth` (`:6883`), with its existing
  `login_store`/`login_runtime` helpers. `AGENT_SH` (`:6923-6944`) gains one line: when
  `FIXTURE_WAIT_FOR` is set, `authenticate` polls for that path, every 50 ms, before answering. Every
  existing case leaves it unset. The fixture link becomes
  `https://h.invalid/login?redirect_uri=http%3A%2F%2F127.0.0.1%3A<port>%2F&state=STATE-SENTINEL-9a2e`,
  with `<port>` from a test-bound `TcpListener`. Cases:
  - `a_pasted_redirect_reaches_the_advertised_port_and_the_login_completes`: the listener receives
    `GET /?code=CODE-SENTINEL-4f1c&state=STATE-SENTINEL-9a2e HTTP/1.1`, answers
    `200` with `<title>signed in</title>`, and writes the `FIXTURE_WAIT_FOR` marker. The frames are
    `Delivered` (status 200, `said` "signed in") at the deliver's `seq`, then `Done { status: Ready }`
    at the choose's `seq`.
  - `a_paste_for_another_port_is_refused_and_nothing_connects`
  - `a_stale_state_is_refused_by_the_worker_even_if_the_pane_let_it_through`
  - `a_deliver_before_any_loopback_redirect_is_refused` uses the plain MOD-21 `LINK` (`:6902`).
  - `a_deliver_with_no_login_running_is_refused`
  - `a_second_deliver_while_one_is_in_flight_is_refused`: the listener accepts and stays silent.
  - `cancel_is_served_while_a_delivery_is_in_flight`: `x` gives `Cancelling`, then `Cancelled` well
    inside the response deadline, and the deliver itself is answered once.
  - `a_deliver_queued_when_the_flow_ends_is_refused_rather_than_dropped`: this extends the pattern
    of `a_command_still_queued_when_the_flow_ends_is_refused_rather_than_dropped` (`:8188`).
  - `no_frame_and_no_debug_carries_the_pasted_code`: every frame of the first case, formatted with
    `{:?}`, plus `format!("{runtime:?}")` mid-flow, contains neither code sentinel.
- **Action.** D269. The `serve` arm, `AuthCommand::Deliver` and its doc (the rule's
  cross-reference), and the `run_auth` changes (a)–(d) with their comments.
- **Validate.**
  - `cargo fmt --all -- --check`
  - `cargo test -p htui --all-features --lib agent_worker -- --test-threads=1`
  - `cargo clippy -p htui --all-features --all-targets -- -D warnings`

### Task 3: the pane (D270–D272)

- **Tests first**, in `crates/htui/tests/settings.rs`, over `chooser_over` plus `Enter` and a
  `Url` frame carrying a loopback `redirect_uri`:
  - `p_with_a_loopback_redirect_opens_a_masked_field_that_captures_input`: `q`, `h`, `l`, `x` and
    `o` are typed rather than acted on, `ctrl-c` passes, and `captures_input()` is true.
  - `p_without_a_loopback_redirect_is_refused_by_name_and_opens_nothing` uses
    `https://h.invalid/o`.
  - `p_is_not_bound_outside_a_running_login`
  - `the_field_draws_dots_and_a_count_and_never_the_text`
  - `enter_with_a_valid_paste_sends_auth_deliver_and_closes_the_field`: the emitted `{:?}` and the
    rendered frame both lack the code sentinel, and the pane reads `delivering to 127.0.0.1:<port>…`.
  - `enter_with_a_wrong_port_a_stale_state_or_no_code_is_refused_locally_and_sends_nothing`: the
    field stays open and empty.
  - `esc_closes_the_field`
  - `a_delivered_frame_notes_what_the_listener_said_and_the_login_keeps_running`
  - `a_refused_auth_deliver_keeps_the_login_pane_and_its_link`
  - `a_flow_that_ends_while_the_field_is_open_closes_it_and_releases_input`
  - `the_running_hint_offers_p_and_the_pasting_hint_offers_enter_and_esc`

  Update the `o open link · x cancel` assertions at `settings.rs:2023`, `:2581` and
  `tests/auth.rs:491` to the new hint. New snapshot `settings__agents_paste_redirect`.
- **Action.** D270 in `agents.rs`: the three `Running` fields, `p`, the paste key path,
  `captures_input`, `on_reply`'s `auth_deliver` exception, `auth_pane`'s lines, the two hints, and
  the module-doc paragraph (D273).
- **Validate.**
  - `cargo fmt --all -- --check`
  - `cargo test -p htui --all-features --test settings -- --test-threads=1`
  - `ls crates/htui/tests/snapshots | wc -l` gives 116.
  - `cargo clippy -p htui --all-features --all-targets -- -D warnings`

### Task 4: end to end (the item's acceptance, through the shell)

- **Tests first** (and only), in `crates/htui/tests/auth.rs`. The file's own fixture script gains
  the same `FIXTURE_WAIT_FOR` line, per that file's per-file helper rule (`:9-11`).
  - `p_paste_enter_delivers_to_the_listener_and_the_cell_reads_the_probes_verdict`. The test drives
    `j j a Enter`, a `Url` frame with a loopback redirect to a test listener, `p`, the typed
    redirect, and `Enter`. It asserts that the listener got the code, that the note line shows the
    listener's answer, that the `on this box` cell ends at `ready`, and that no rendered frame at
    any step contains the code sentinel.
- **Validate.**
  - `cargo fmt --all -- --check`
  - `cargo test -p htui --all-features --test auth -- --test-threads=1`
  - then the full workspace gate below.

## Test plan

TDD per repo convention: every task's first commit is its failing tests. **The first red test** is
T1's `advertised_is_read_from_a_percent_encoded_redirect_uri`.

**The item's asks, mapped:**

| Ask | Where it is tested |
|---|---|
| A field in MOD-21's pane | T3 `p_with_a_loopback_redirect_opens…` |
| Loopback host | T1 validation table (6), T1 `advertised_is_none_for…` |
| The port the flow actually advertised | T1 `wrong_port…`, T2 `a_paste_for_another_port…` (the worker's own record) |
| `code`/`state` present | T1 (10)–(12), T2 `a_stale_state…` |
| Issued on the box `htui` runs on | T1 `deliver_sends_one_get…`, T2 `a_pasted_redirect_reaches…` |
| Reports what the listener said | T1 `deliver_reports…`, T3 `a_delivered_frame_notes…` |
| Completes through MOD-21's path | T2 `…and_the_login_completes`, T4 |
| Off the UI task | The section holds no socket (reviewer check), and T2 `cancel_is_served_while_a_delivery_is_in_flight` |
| Never logged, persisted, on a lasting frame, or echoed | T1 redaction cases, T2 `no_frame_and_no_debug…`, T3 `the_field_draws_dots…` and `enter_with_a_valid_paste…`, T4's per-frame sentinel sweep |

**Count pins that move:**

| Pin | Now | After | Where |
|---|---|---|---|
| `StoreRequest` variants | 90 (`HANDOFF.md:55` says 88, stale) | 91 (T1) | `crates/htui/src/store_worker.rs` (90 `Self::` arms in `name`, `:802-907`); `HANDOFF.md:55` (main thread) |
| `StoreReply` variants | 49 (`HANDOFF.md:55` says 48, stale) | 49 | — |
| `AuthFrame` variants | 10 | 11 (T1) | `crates/htui/src/store_worker.rs:1216-1269` |
| `crates/htui/tests/snapshots` | 115 | 116 (T3) | `HANDOFF.md:56-57` (main thread) |
| Store `CASES`, migrations, `.sqlx`, `htui-orch` `CASES` | 97, `0001`..`0009`, 289, 73 | unchanged | — |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-1** — A vendor listener checks more than `code`/`state` (a `Host`, a `Referer`, a cookie) and refuses the relayed request | Low | The `curl` proof shows a bare `GET` works for `agy`. The listener's answer is reported verbatim (D268), so a refusal is visible, and MOD-16/MOD-7 can extend the request |
| **R-2** — The adapter prints the callback, code included, on its own stderr, which MOD-21's pane shows | Low | Outside `htui`'s control, and stated in the module doc (D273). `agy` printed one stderr line in MOD-21's live run (`docs/decisions/mod/mod-21.md`, "Live proof"). Scrubbing the pane's lines would mean holding the code for the rest of the flow, which is worse |
| **R-3** — Heap residue: `url::Url`'s buffer and the kernel socket buffers are not wiped. Also the field's own growth: `TextField::masked()` reserves 256 bytes (`text_field.rs:81-83`, `:94`), so a paste longer than that (R-4 expects ~500 chars) reallocates while it is typed. Each reallocation frees an **unwiped** copy of the prefix typed so far | Accepted | `Zeroizing` on everything `htui` owns. Stated in the module doc. The field's residue is either priced in that same sentence, or T3 opens the field with a larger reservation. A larger reservation needs a new `TextField` constructor, which adds `crates/htui/src/ui/text_field.rs` to T3's files; the architect decides |
| **R-4** — No bracketed paste, so a ~500-char paste arrives as ~500 `KeyEvent`s | Low | `TextField` inserts per grapheme. A trailing newline in a paste submits a complete URL, which still validates. Bracketed paste stays out of scope (`text_field.rs:10`) |
| **R-5** — After the adapter died, another local process holds the advertised port and receives the code | Low | A deliver is refused unless the flow is running (D263). The adapter owns the port while it runs. `NothingListening` names the dead-flow case |
| **R-6** — IPv6 loopback is unavailable in the sandbox container | Medium | `localhost_falls_back_to_the_ipv6_loopback` skips with a printed reason. The IPv4 path is the one `agy` advertises |
| **R-7** — The htui suite is scheduling-dependent (process-wide keyring fake) | Medium | Every gate runs `--test-threads=1`, and the main thread re-runs the gates on the merged tree |
| **R-8** — Decision IDs D263–D274 collide with another sandbox's plan drafted at the same time | Medium | Renumber at merge. IDs are local to plan text and cited only in this item's docs |
| **R-9** — A listener that answers only after exchanging the code takes several seconds | Low | The response deadline is 15 s, and the flow keeps being served meanwhile (D269) |
| **R-10** — The adapter answers `authenticate` before its listener's HTTP answer has been read. `acp::auth::run` kills and reaps the child before the flow's future returns, so the socket ends, possibly with a reset. The pane may then show a delivery failure just before `logged in: ready` | Medium | D269(d) answers the delivery before `Done`. `deliver` treats EOF or a reset **after** a status line as the end of the body, not as `Io`. The sentence for "closed without answering" says the login may have completed, and points at the result that follows |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1     # docs/hr-sandbox.md:184
cargo doc --workspace --no-deps --keep-going                  # exactly the baseline errors (HANDOFF.md:65-70)
cargo tree -p htui-agent -i url                               # 2.5.8, one instance
ls crates/htui/tests/snapshots | wc -l                        # 116
git diff --stat f5de3d4 -- crates/htui-store crates/htui-core crates/htui-orch \
  crates/htui-agent/src/auth/run.rs crates/htui-agent/src/auth/url.rs \
  crates/htui-agent/src/auth/browser.rs crates/htui-agent/src/acp   # empty
```

`--test-threads=1` is not optional.

**Live check (OQ-6, maintainer, after Wave 3).** Run `htui` on the server and the browser on the
laptop. In Settings > Agents, press `a` on `agy`, then `Enter` on `oauth-personal`. The pane shows the
link and `redirect: 127.0.0.1:<port>`. Open the link on the laptop and finish the consent; the
browser fails on `127.0.0.1:<port>`. Copy that address and press `p`, paste, `Enter`. The note line
reads `127.0.0.1:<port> answered 200 …`, then `logged in: ready`. `agy_live.rs` passes afterwards.
Record the listener's actual answer (status, title) in the decision doc.

## Acceptance

- [ ] During a login whose link advertises a loopback `redirect_uri`, `p` opens a masked field in the
      login pane; `Enter` validates the pasted address against the flow's host, port, path and
      `state`, and requires `code` and `state`.
- [ ] A valid paste is delivered as one `GET` to that loopback port from the flow's own task; the
      pane reports the listener's status and a redacted excerpt; the login then ends through
      MOD-21's `Done` with the probe's verdict.
- [ ] Every refusal is a fixed sentence naming a rule, a host or a port and never pasted text; no
      frame, `Debug`, log line, notice or snapshot contains the pasted code (sentinel-checked).
- [ ] `x` stays responsive while a delivery is in flight; every `AuthDeliver` is answered once.
- [ ] The workspace gate above is green; nothing in D274 changed.

## Where the HANDOFF or tree disagree

1. **"Needs a text-input widget the Settings tab does not have yet"** is stale, as MOD-23 found for
   the same sentence. `TextField` with a masked mode exists (`crates/htui/src/ui/text_field.rs`) and
   serves the DSN field (`settings/connection.rs:365`, `:464`), the Qdrant key
   (`settings/qdrant.rs:151`) and MOD-23's registry form. MOD-22 reuses it (D270, D272).
2. **`R-AGT-9`'s "does not read, hold, transmit or store the credential"** is contradicted in its
   letter by any in-app paste-back (OQ-1). The main thread amends the requirement.
3. **"The port the flow actually advertised" is recorded nowhere today.** MOD-21 D15 made link
   detection "a scan, not a parse" (`auth/url.rs:1-12`). MOD-22 adds the parse in a new module
   (D264). `url.rs`'s "never parsed" doc stays true of `first_url`.
4. **Pins**: `StoreRequest` 90 → 91 and snapshots 115 → 116, for the main thread's close-out.
   `HANDOFF.md:55-57` already disagrees with the tree at `f5de3d4`. It pins `StoreRequest` 88 and
   `StoreReply` 48, but the tree has 90 and 49. The close-out corrects both pins, not only this
   item's +1.

---

## Verified claims

Every checkable fact this plan asserts, checked against `f5de3d4` on 2026-09-30. Rows 1–43 are the
planner's. Rows 44–68 are the fact-check's own, covering claims in the body the table did not list
and facts the body needed. `amended §X` names the section corrected in this pass.

| claim | verdict | evidence |
|---|---|---|
| 1. `HEAD` is `f5de3d4` on `hr/MOD-22`; the highest decision ID under `.claude/plans/` is D262 (`mod-23-agent-registry-editing.blueprint.md`) | partially true | `HEAD` is `30f6a07`, this plan's commit. Its parent `f5de3d4` is `main` and `origin/main`, and `git diff --stat f5de3d4 HEAD` touches only this plan, so every cited line holds. D262 is the highest ID outside this plan, but two plans already claim it: `mod-23…blueprint.md:1158` (D249–D262) and `mod-64-concepts-search.blueprint.md:1326` (D241–D262). The collision R-8 guards against already exists once in the tree. |
| 2. `HANDOFF.md:505-535` is the MOD-22 entry and names `R-AGT-9`, `R-TUI-8`, `R-NF-3`, `R-SEC-2`, `R-ID-7`, ports 39879/50651, and the `curl` paste-back workaround | verified | Entry `:505`–`:535`; MOD-66 starts at `:536`. Requirements `:506`; ports `:508-509` (measured 2026-09-09); `curl` workaround `:513-515` (confirmed 2026-09-10). |
| 3. `docs/REQUIREMENTS.md` defines `R-ID-7` (`:62`), `R-AGT-9` (`:198-204`, including "does not read, hold, transmit or store the credential"), `R-SEC-2` (`:287`), `R-TUI-8` (`:330-334`), `R-NF-3` (`:359-360`) | partially true | All correct except that `R-AGT-9` runs `:198-205` (it ends at "MOD-21."). The quoted sentence is at `:201-202`. The body cites no line for it, so nothing is amended. |
| 4. `crates/htui-agent/src/auth/url.rs:1-12` says link detection is "a scan, not a parse" and `first_url` is at `:34` | verified | `url.rs:3` "A scan, not a parse."; module doc `:1-12`; `pub fn first_url` `:34`. |
| 5. `AuthEvent::Url(String)` is at `crates/htui-agent/src/auth/mod.rs:82`; `AuthFlow` has an injectable `idle: Duration` and `AUTH_IDLE_CAP` is 10 minutes | verified | `mod.rs:82`; `pub idle: Duration` `:59`; `AUTH_IDLE_CAP = Duration::from_secs(10 * 60)` `:40`. |
| 6. `run.rs`'s `forward` (`:171`) sends `Line` then `Url` for a first-seen link; its module doc (`:9-12`) says the loop is one `select!` | verified | `run.rs:171`: `Line` is sent unconditionally, then `Url` only if `seen.insert(url)`. `:9-12` "The loop is the whole of it … one `select!`". |
| 7. `AuthCommand` (`agent_worker.rs:290-305`) has exactly `Choose` and `Open` | verified | `:290-305`. It also derives `Debug` (`:289`), so `Deliver`'s `RedirectUrl` must redact (D266). |
| 8. `LiveAuth` (`agent_worker.rs:312-327`) holds `agent_id`, `cancel`, `commands`, `task`, `_claim`; its `Debug` (`:329-337`) prints no line or link | verified | Fields as stated. `Debug` prints `agent_id`, `finished` and `closed` only. |
| 9. `AgentRuntime::serve` routes `AuthStart`/`AuthChoose`/`AuthOpen`/`AuthCancel` at `agent_worker.rs:1096-1116` and has an `other =>` wildcard answering `"not a chat request"` (`:1117-1120`) | verified | `AuthStart` `:1096`, `AuthOpen` `:1109-1115`, `AuthCancel` `:1116`; wildcard `:1117-1120`. |
| 10. `auth_start` is at `agent_worker.rs:1533-1608`; `auth_command` at `:1622-1637` answers `"no login is running"` with no live flow; `auth_cancel` at `:1645-1654` | verified | All three ranges hold. `auth_command` also answers `"this login has ended"` on a closed channel. |
| 11. `run_auth` (`agent_worker.rs:2860-3026`) forwards events without keeping any, handles `Open` by awaiting `open_url` inline, drains queued commands with `"this login has ended"`, and its only `tracing` call is in the `Kept` arm | verified | `biased;` at `:2899`, with `running` first (`:2900`). Events arm `:2901` forwards `auth_frame(event)` and keeps nothing. `Open` arm `:2919-2932` awaits `open_url` (`:2922`). `commands.close()` `:2951`, drain `:2955-2966`. The only `tracing::info!` is `:3005`, in `Kept`. |
| 12. `auth_frame` is at `agent_worker.rs:3030` | verified | `:3030`, a total map with no state. |
| 13. The test module `auth` starts at `agent_worker.rs:6883`; `METHOD` `:6892`, `LINK` `:6902` (carries `state=SENTINEL-TOKEN-VALUE`), `AGENT_SH` `:6923-6944` (reads `FIXTURE_URL`, `FIXTURE_HOLD`, `FIXTURE_KEY`, `FIXTURE_CRED`), `login_row` `:6978`, `login_store` `:7056`, `login_runtime` `:7090` | verified | All lines hold (`#[cfg(unix)]` at `:6882`). `AGENT_SH` also reads `FIXTURE_DIR` and `FIXTURE_INIT`. |
| 14. `a_command_still_queued_when_the_flow_ends_is_refused_rather_than_dropped` is at `agent_worker.rs:8188`; the `Fixture` HTTP responder is at `:3975-4040` | partially true | The test is at `:8188`. `Fixture` spans `:3974-4091`: derive `:3974`, struct `:3975`, and `impl` through `answer`, which ends at `:4091`. Amended §Patterns to Mirror. |
| 15. `StoreRequest`'s doc (`store_worker.rs:90-102`) forbids a secret as a plain `String` and asks for a redacting newtype; `#[derive(Debug, Clone)]` at `:103` | verified | `:96` "No variant may carry a secret as a plain `String`"; `:99-101` the redacting newtype; `:103` derive. |
| 16. `StoreRequest::AuthStart`..`AuthCancel` are at `store_worker.rs:302-338`; `AuthOpen` is answered once at its own `seq` | verified | `AuthStart` doc `:302`, `AuthCancel` `:338`; `AuthOpen` `:324-333` with "Answered exactly once at its own `seq`" at `:326`. |
| 17. `StoreRequest::name` maps the four auth requests at `store_worker.rs:833-836` | verified | `:833-836`. |
| 18. `StoreReply::Auth(AuthFrame)` and its routing doc are at `store_worker.rs:965-971` | verified | `:965-971`. |
| 19. `AuthFrame` (`store_worker.rs:1215-1269`) derives `Debug, Clone` and has exactly 10 variants | verified | `:1215` derive; the 10 variants are `Methods`, `Line`, `Url`, `Opened`, `Done`, `Refused`, `Cancelling`, `Cancelled`, `Idle`, `Failed`. |
| 20. The no-runtime refusal arm lists the auth requests at `store_worker.rs:1344-1361`; the runtime routing arm at `:1959-1983`; name tests at `:3456-3476` and `:3540-3557` | verified | All ranges hold. The first name test (`name_arms_are_stable`, `:3454`) is a run of `assert_eq!`, not a table; the second (`try_serve_without_a_runtime_refuses_all_four_by_name`, `:3537`) is a table. |
| 21. `crates/htui/src/testkit.rs:291-294` or-matches the four auth requests | verified | `:291-294`. |
| 22. `AuthFrame` is referenced in exactly four files: `store_worker.rs`, `agent_worker.rs`, `settings/agents.rs`, `tests/settings.rs` (plus `tests/auth.rs` by text) | partially true | The four files hold (40/10/13/15 references). `tests/auth.rs` never names `AuthFrame`; it asserts rendered frames only. Only `on_auth_frame` (`agents.rs:674-752`) matches exhaustively. Every `agent_worker.rs` test match has an `other =>` arm, and `is_terminal` (`:7160`) uses `matches!`, so T1's new variant breaks nothing in T2's file. |
| 23. `App::is_fresh` (`app/state.rs:328-332`) keys freshness on origin and request kind | verified | `:328-332`. The key is `(origin, discriminant(&request))`, inserted at `:310-311`. See row 53. |
| 24. `settings/agents.rs`: `HINT_AUTH_RUNNING` `:162` is `"o open link · x cancel"`; `AUTH_PANE_LINES` `:193` is 6; `AuthState` `:254-293` with `Running { agent_id, call, lines, url, cancelling }` | verified | All hold. `AuthState` derives `Debug, Default` (`:253`); `url: Option<String>` is "the newest link", set on every `AuthFrame::Url` (`:709-713`). |
| 25. `settings/agents.rs`: `refuse_during_login` `:647`, `open_link` `:659-666`, `on_auth_frame` `:674-752` (an exhaustive match), `auth_pane` `:784-834` (renders `link: {url}`), `pane` `:1053-1079`, `hint` `:1087-1114`, `captures_input` `:1776-1778` (`Mode::Editing` only), `on_key` `:1787-1910`, `on_reply` `:1912-1988` with the `auth_open` exception at `:1968-1971`, `render` `:1990-2021` | partially true | Every range holds except the `auth_open` exception, which is at `:1972-1975`. `:1967-1971` is the `auth_start`/`auth_choose`/`auth_cancel` → `Idle` arm. Amended §D270 and §Patterns to Mirror. |
| 26. `p` is unbound in the agents section and in the global keymap; `SettingsTab::on_key` gives a capturing section every key first (`settings/mod.rs:323-331`); `captures_input` defaults to `false` (`:149`) | verified | No `Char('p')` in `agents.rs`. The global keymap binds `q`, `Tab`, `BackTab`, digits, `?`, `Esc` and `ctrl-c` (`keymap.rs:209-248`). `p` is bound only in other sections (`boxes.rs:464`, `hierarchy.rs:1117`). `App::on_key` hands the tab the key first, and a `Consumed` key never reaches the keymap (`app/state.rs:472-507`). |
| 27. `TextField` (`text_field.rs:45-57`) holds `Zeroizing<String>`; its `Debug` (`:63-69`) prints no text; `masked()` `:91-97`; `on_key` `:119-174` passes CONTROL/ALT/SUPER/META/HYPER chords and maps `Enter`/`Esc` to `Submit`/`Cancel`; `text()` `:181-187` is `None` when masked; `take()` `:202-205`; `clear()` `:211-214`; `line()` `:249-356` draws `•` and a count when masked | verified | Every range holds. `Debug` prints `masked`, `len` and `cursor`. `clear()` zeroizes in place, and drop wipes through `Zeroizing`. `masked()` reserves 256 bytes (`:94`); see row 57. |
| 28. `text_field.rs:10` says bracketed paste is deliberately not built; nothing in `crates/htui/src` enables bracketed paste | verified | `:10`. No `BracketedPaste` or `Event::Paste` anywhere under `crates/`. |
| 29. `TextField::masked()` is used at `settings/connection.rs:365`, `:464` and `settings/qdrant.rs:151` | verified | All three, plus a test use at `connection.rs:937`. |
| 30. `htui_store::Dsn` is `Dsn(Zeroizing<String>)` with a hand `Debug` printing `Dsn(<redacted>)` and no `Display` (`crates/htui-store/src/dsn.rs`) | verified | `dsn.rs:32-33` `#[derive(Clone)] pub struct Dsn(Zeroizing<String>)`; `Debug` `:35-37`; no `Display` (`:10`); `as_str` is `pub(crate)` (`:162`). |
| 31. `crates/htui-agent/tests/auth_live.rs:68-75` is the "What it must never print (`R-ID-7`, `R-SEC-2`)" section | verified | Heading `:68`, body `:70-75`. |
| 32. Root `Cargo.toml` declares `reqwest` with `system-proxy` (`:90-91`) and `zeroize = "1.9"` (`:51`), and does not declare `url` | verified | `:90-91`, `:51`; there is no `url` key in `[workspace.dependencies]`. |
| 33. `crates/htui-agent/Cargo.toml:24` lists tokio features `sync, rt, time, process, io-util, fs` (no `net`); its dev tokio (`:71`) has `net`; it declares neither `url` nor `zeroize` | verified | `:24`, `:71`. |
| 34. `Cargo.lock` has `url` 2.5.8 (`:7113-7114`) as a `reqwest` 0.13.5 dependency, and tokio 1.53.1 depends on `mio` and `socket2` | verified | `:7113-7114`. `reqwest` 0.13.5 lists `url`, and so does `tower-http` 0.6.11. tokio 1.53.1 lists `mio` and `socket2`. |
| 35. `crates/htui-agent/src/install/http.rs` follows redirects by the default policy (`:142`) and reads the system proxy configuration (`:72`) | verified | `:142` "redirects followed by the default policy"; `:72` "a system proxy configuration that does not parse". |
| 36. `crates/htui/Cargo.toml` dev tokio has `net`; `htui` declares `zeroize` and `futures` | verified | `:83` dev tokio `["test-util", "net"]`; `:50` zeroize; `:38` futures. |
| 37. `crates/htui/tests/auth.rs` is `#![cfg(all(feature = "testkit", unix))]`, has its own fixture helpers (`:121-230`), and asserts the running hint at `:491` | partially true | The `cfg` (`:16`) and the hint (`:491`) hold. `AGENT_SH` is at `:93-114` and the helpers at `:121-238` (`recorder` ends `:238`). Amended §Patterns to Mirror. |
| 38. `crates/htui/tests/settings.rs` asserts the running hint at `:2023` and `:2581`; `chooser_over` `:1841`, `typed` `:2698`, `render_section` `:93` | verified | All hold. With `auth.rs:491`, these are the only three assertion sites for the hint. |
| 39. `crates/htui/tests/snapshots` holds 115 files, none for a running login | verified | `ls \| wc -l` = 115. The nine `settings__agents_*` snapshots are idle, form or probe states. |
| 40. `HANDOFF.md:55-57` pins `StoreRequest` 88, `StoreReply` 48 and 115 snapshots | partially true | The text is as stated (`:55`, `:56-57`), but two of the pins are stale against the tree: 90 and 49 (row 52). Amended §Count pins and §Where the HANDOFF or tree disagree 4. |
| 41. `docs/hr-sandbox.md:184` is `cargo test --workspace --all-features -- --test-threads=1` | verified | `:184`. |
| 42. `docs/decisions/mod/mod-21.md` records that `agy` printed one stderr line in the live run and names MOD-22 for the remote-box gap | verified | `:40` quotes the one stderr line; `:119` "1 stderr line(s), 1 link(s)"; `:142` names MOD-22. |
| 43. T2 ∩ T3 = ∅, T1 ∩ T2 = ∅, T1 ∩ T3 = {`agents.rs`}, T3 ∩ T4 = {`tests/auth.rs`}; T3 compiles against T1 alone | verified | File sets as stated; T2 ∩ T3 = ∅. T1 needs no `agent_worker.rs` edit, because `serve` has a wildcard and no test match there is exhaustive (row 22). There are two hidden couplings, rows 58 and 59. Amended §Tasks (independence conditions 2 and 3). |
| 44. D263–D274 are unused | verified | `git grep -E '\bD(26[3-9]\|27[0-4])\b'` over every local and remote ref, in `.claude`, `docs`, `DECISIONS.md` and `HANDOFF.md`: only this plan matches. |
| 45. Declaring `url`, `zeroize` and tokio `net` in `htui-agent` resolves offline and adds no package (OQ-5, D267) | verified | Throwaway worktree at `f5de3d4` with the three edits. `cargo metadata --offline` succeeds. The `Cargo.lock` diff is +2 lines (`"url"` and `"zeroize"` in `htui-agent`'s list) and 0 `[[package]]`. `cargo tree --offline -p htui-agent -i url` shows 2.5.8 only. `~/.cargo/registry` holds the `url-2.5.8`, `zeroize-1.9.0` and `tokio-1.53.1` sources. A `/tmp` crate on the repo lock and toolchain 1.98.1 passes `cargo build --offline`. The worktree was removed afterwards. |
| 46. D264: `query_pairs()` percent-decodes `redirect_uri`; loopback hosts parse as expected | verified | Compile probe with `url` 2.5.8. `…redirect_uri=http%3A%2F%2F127.0.0.1%3A39879%2F&state=S1` decodes to `http://127.0.0.1:39879/`: `Ipv4(127.0.0.1)`, port 39879, path `/`. `LOCALHOST` parses to `Domain("localhost")`, because `url` lowercases hosts. `%5B%3A%3A1%5D` gives `Ipv6(::1)`. With no port, `port_or_known_default()` is 80. `query_pairs` is form-urlencoded, so `+` also decodes to a space. |
| 47. D265 (2): a pasted address without `://` needs the `http://` prefix | verified | Probe: `Url::parse("localhost:39879/?code=a&state=b")` **succeeds** with scheme `localhost` and no host. WHATWG parsing also normalises `127.1` and `2130706433` to `127.0.0.1`, so (6) compares parsed hosts. |
| 48. D267: the `Host` header for an IPv6 loopback is well-formed | verified | Probe: `host_str()` of `http://[::1]:5000/` is `"[::1]"`, bracketed. |
| 49. D267: tokio `net` is already compiled for `htui-agent` | verified | `cargo tree --offline -p htui-agent -e features,normal -i tokio` shows `tokio feature "net"` enabled through `reqwest` and `hyper-util`'s `client`. |
| 50. OQ-5: `zeroize`, `hmac` and `shell-words` were promoted from compiled transitive crates, and `form_urlencoded` is compiled | verified | Root `Cargo.toml:48-51`, `:57-59` and `:131-133` each carry that reason. `Cargo.lock` has `form_urlencoded` 1.2.2 (a dependency of `url`). |
| 51. `auth/loopback.rs` can name the `url` crate although `auth/` has a `url` module | verified | Probe with a sibling `url` module. `use url::Url;` in `loopback.rs` resolves to the crate. `url::Url` in `auth/mod.rs` fails with E0425 (it names `auth::url`) and needs `::url::Url`. Amended §Task 1 Action. |
| 52. Count pins: `StoreRequest` 88 → 89, `StoreReply` 48 | falsified | The tree at `f5de3d4` has **90** `StoreRequest` variants (90 `Self::` arms in `name`, `store_worker.rs:802-907`) and **49** `StoreReply` variants (enum body count). After T1: 91 and 49. Amended §Count pins and §Where the HANDOFF or tree disagree 4. |
| 53. D263: `is_fresh` passes the deliver's answer | partially true | It does only while no newer `AuthDeliver` has come from the same origin. A second one replaces the first's `seq` under `(origin, kind)` (`app/state.rs:310-311`), and the first's `Delivered` is then dropped as stale. Amended §D263, and §D270 (`p` is refused while `delivering`). |
| 54. D269(d): by the time the loop breaks, the child has been reaped | verified | `acp::auth::run` (`crates/htui-agent/src/acp/auth.rs:180-325`) calls `guard.kill_and_reap().await` before returning; its doc says "killed **and reaped** before this returns". Consequence: the listener's answer can be lost when the adapter answers `authenticate` first. Added R-10; amended §D269(d). |
| 55. T2 `cancel_is_served_while_a_delivery_is_in_flight`: `Cancelled` arrives well inside the response deadline | falsified | Not as D269 was written. (d) awaited an in-flight delivery "to its own deadline" before the terminal frame, and (b) hard-coded `DeliverLimits::default()`. The test's silent listener is the test's own, not the child's, so `Cancelled` would wait the full 15 s. Amended §D269(b) (limits come from `AuthArgs`) and (d) (dropped and answered on cancel). |
| 56. D267: `DeliverLimits` are injected by tests "as `AuthFlow.idle` is" | partially true | `AuthFlow.idle` is injectable at the `htui-agent` API (`auth/mod.rs:59`), but `run_auth` hard-codes `idle: AUTH_IDLE_CAP` (`agent_worker.rs:2888`). Amended §D267: the worker's limits travel through a new `AuthArgs` field. |
| 57. The masked field wipes what was pasted (OQ-2, D270, R-4) | partially true | `masked()` reserves 256 bytes (`text_field.rs:81-83`, `:94`). R-4's ~500-char paste outgrows that, and each reallocation frees an unwiped prefix. Amended §R-3. |
| 58. T3 compiles against T1 alone | partially true | Only if T1 exports what T3 draws and T2 asserts: an `Advertised` host/port accessor (for `127.0.0.1:39879`) and public `ListenerReply` fields. D264–D268 do not list them. Amended §Tasks condition 2 and §Task 1 Action. |
| 59. `store_worker.rs`'s tests (T1's file) call T2's `login_store` and `login_runtime` helpers | verified | `store_worker.rs`'s test calls `crate::agent_worker::tests::auth::login_store` and `login_runtime` (`store_worker.rs:3581`, `:3593`). T2 must keep their signatures. Amended §Tasks condition 3. |
| 60. D270: a `CONTROL` chord → `Handled::Pass`, everything else `Consumed` | verified | This mirrors `connection.rs:427-428` exactly. `FieldOutcome::Pass` also covers `Tab`, `Up`, `Down` and similar keys (`text_field.rs:172`), and this rule consumes them. |
| 61. Two `store_worker.rs` docs move with T1's variants | verified | The no-runtime arm's comment counts "MOD-21's four login ones … all fifteen" (`:1339-1343`). `AuthFrame`'s doc says "Nothing here can hold a credential" (`:1211-1214`). Amended §Task 1 Action. |
| 62. D270: the new running hint is 41 characters | verified | `o open link · p paste redirect · x cancel` is 41 characters. The same constant is also shown in `Starting` (`agents.rs:1095`). |
| 63. D271 / §Where the HANDOFF or tree disagree 1: HANDOFF says MOD-7 records no remote fact, and that the Settings tab lacks a text widget | verified | `HANDOFF.md:531-533`; `:521` "Needs a text-input widget the Settings tab does not have yet". This is stale: `agents.rs` already holds `TextField`s for MOD-23's form. |
| 64. D273: `settings/agents.rs`'s module doc has MOD-21 and MOD-23 paragraphs | verified | "Since MOD-21" `:24`; "Since MOD-23" `:40`. |
| 65. Referenced test files exist: `htui-agent/tests/{extensibility,install,auth,auth_live,agy_live}.rs` | verified | `ls crates/htui-agent/tests`. |
| 66. The pins that stay unchanged: store `CASES` 97, `htui-orch` `CASES` 73, migrations `0001`..`0009`, 289 `.sqlx` | verified | `pg_conformance.rs:19` `EXPECTED_CASES = 97`; `conformance.rs:364-533` has 73 entries; `migrations/` ends at `0009_agent_box_user_off.sql`; 289 `.sqlx/*.json`. |
| 67. RFC 6749 §4.1.1 makes `state` RECOMMENDED; RFC 8252 §7.3 is loopback redirection | verified | From the RFC texts as known to the checker. The sandbox has no network, so they were not re-fetched. |
| 68. T2's new worker cases fit the existing `auth` test module | verified | `agent_worker.rs:7572-7587` already tables `AuthChoose`/`AuthOpen`/`AuthCancel` against "no login is running", so `a_deliver_with_no_login_running_is_refused` can extend it. `next_frame` (`:7151-7157`) panics on a `Failed` reply, so T2's deliver cases need their own reader. |
