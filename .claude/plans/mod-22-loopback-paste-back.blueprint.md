# Blueprint: MOD-22, the loopback paste-back in the login pane

**Status**: PROPOSED 2026-09-30 by the code-architect, from the plan confirmed the same day
(OQ-1…OQ-6 on their recommended answers; T1 → T2 → T3 → T4 **serially**). Findings F-1…F-16 (§0.2)
and decisions D275–D290 (§7) are this blueprint's. **Major** means a named test, signature, file
list or behaviour in the plan is wrong, missing or ambiguous enough that two implementers would
build different things. **Minor** is a citation, wording, placement or a hazard with a cheap guard.
Where §0.2 amends the plan, this file wins.

**Plan**: `.claude/plans/mod-22-loopback-paste-back.plan.md` (CONFIRMED 2026-09-30, fact-checked:
55 verified, 15 amended). Its D263–D274, R-1…R-10, its "Verified claims" table and every amendment
are binding and are not reopened here, except where F-1…F-16 show that the plan's own text cannot
do what it says or leaves a choice open.

**Verified at**: HEAD `8981d39` on `hr/MOD-22`. `git diff --stat f5de3d4 HEAD -- crates Cargo.toml
Cargo.lock` is empty, so the plan's line numbers (taken at `f5de3d4`) are HEAD's. **Line numbers are
pre-edit.** Once a task makes its first commit, a citation into a file that task edits has moved.
Counts at HEAD: `crates/htui/tests/snapshots/` **115**; `StoreRequest` **90**, `StoreReply` **49**,
`AuthFrame` **10**. `df -h /`: **96 GB free (79 % used)**. `cargo-insta` 1.48.0 is installed.

**Tooling**: the Gortex daemon calls this checkout INACTIVE but answered every symbol, source and
windowed-file query used here (`agent_worker.rs`, `store_worker.rs`, `testkit.rs`, `agents.rs`,
`text_field.rs`, `connection.rs`, `settings/mod.rs`, `app/state.rs`, `auth/mod.rs`,
`extensibility.rs`). Its hook blocks `Read`/`grep` on indexed Rust files, so Rust was read through
Gortex; manifests and the integration-test files were read with `sed`. The tokio 1.53.1 and `url`
2.5.8 sources were read in `~/.cargo/registry` (F-9).

**Coupling verdict.** Serial, as confirmed. Every task runs in the **primary checkout on
`hr/MOD-22`**: no worktrees, no second `target/`, one Postgres. The plan's "T2 ∥ T3" analysis still
holds and is what makes the serial order safe to reorder in an emergency, but nothing here depends on
it. **T3's file set grows by one** (`crates/htui/src/ui/text_field.rs`, D275). No other file moves
between tasks.

**Scope at a glance**:
- **New**: `crates/htui-agent/src/auth/loopback.rs`, `crates/htui-agent/tests/loopback.rs`,
  `crates/htui/tests/snapshots/settings__agents_paste_redirect.snap`.
- **Dependencies**: `url = "2.5"` promoted (2.5.8, already locked); `htui-agent` declares `url`,
  `zeroize` and tokio `net`. `Cargo.lock` gains **two lines** and **no** `[[package]]`.
- **Protocol**: `StoreRequest` 90 → **91** (`AuthDeliver`); `StoreReply` stays **49**; `AuthFrame`
  10 → **11** (`Delivered`); `AuthCommand` 2 → **3** (`Deliver`).
- **Snapshots** 115 → **116** (one new; none of the existing 115 moves, F-13).
- **No change** to anything in the plan's D274 list. `crates/htui-agent/src/lib.rs` is **not**
  touched either (D279).

**House style (carried)**: `unsafe_code = "forbid"`; `missing_docs`, `missing_debug_implementations`
and `unused_qualifications` warn; clippy `all` at `-D warnings`, never `#[allow]` to get green;
rustdoc denies broken and **private** intra-doc links, so a `pub` item's doc never links a private
fn; `rustfmt.toml` `max_width = 100`. Every new `pub` item has a doc comment and a `Debug`. Red tests
come first; a `todo!()` body goes **only** where no existing path calls it. Every commit compiles and
no test is loosened. Implementers stage their own paths only: never `git add -A`, never `stash`,
never `--amend`, and they commit after each green step (uncommitted work dies with the session).
**No test, assertion message, `tracing` call, panic or snapshot contains a pasted code**: tests use
`CODE-SENTINEL-4f1c` / `STATE-SENTINEL-9a2e` and assert the code sentinel's absence. Nothing branches
on an agent's name, and no `src` file spells a string from `extensibility.rs`'s `VOCABULARY`
(`R-AGT-5`; `the_auth_flow_names_no_vendor_or_method`, `extensibility.rs:350-386`).

---

## 0. Environment, findings

### 0.1 Environment and gates (this sandbox)

```bash
# Already set by the sandbox (docs/hr-sandbox.md "Databases"); nothing to export.
pg_isready -h localhost -p 5439     # only the workspace gate (§6) needs Postgres; T1..T4 gates do not
df -h /                             # before each task; one target/ only (serial, primary checkout)
```

`.cargo/config.toml` sets `SQLX_OFFLINE = "true"`; no task touches SQL or `.sqlx`. Every `cargo
test` line is `--test-threads=1`: the htui suite's green depends on scheduling (the keyring fake is
process-wide). Before believing a Postgres failure in the workspace gate, run `df -h /` and re-run the
case alone.

### 0.2 Findings the plan and its fact-check left open

| # | Severity | Task | Plan says | Tree / reading | Fix |
|---|---|---|---|---|---|
| **F-1** | **Major** | T3 | R-3: "the architect decides" between pricing the field's reallocation residue and a larger reservation. | `TextField::masked()` reserves 256 bytes (`text_field.rs:91-97`); `insert` is `String::insert` (`:373-380`), which reallocates only past capacity. A ~500-char redirect therefore leaves an unwiped copy of every prefix at 256 and 512 bytes. | **D275**: `text_field.rs` gains `TextField::masked_with_capacity(bytes: usize) -> Self` and `masked()` becomes `masked_with_capacity(256)`. The pane opens its field with `loopback::PASTE_MAX` (8192): every paste `validate` would accept never reallocates. A paste longer than that is refused `TooLong` anyway; its residue is priced in the module doc. **`crates/htui/src/ui/text_field.rs` joins T3's file set.** |
| **F-2** | **Major** | T1 | D268: `DeliverError` has four variants. R-10: "the sentence for 'closed without answering' says the login may have completed". | None of the four variants can say that sentence: `NothingListening` is a refused connect, `NotHttp` is bytes that are not HTTP, `Io { kind }` is a generic kind. | **D276**: a fifth variant `ClosedWithoutAnswer { target }`. §2.3 fixes the read loop's end rules exactly: what ends a read, which endings are an answer and which are an error. |
| **F-3** | **Major** | T2 | D269(d): "If the loop broke on the cancel token (`x`, shutdown, idle), the delivery is dropped …; otherwise it is awaited to its own deadline." | `run_auth`'s loop (`agent_worker.rs:2896-2945`) breaks **only** when the flow future returns. The token never breaks it; the flow sees the token and returns `Cancelled`. The idle clock is a **child** token inside `htui-agent` (`auth/run.rs`), so `cancel.is_cancelled()` is `false` after an idle end. | **D277**: "ended" is `cancel.is_cancelled() \|\| matches!(outcome, Ok(AuthOutcome::Cancelled \| AuthOutcome::Declined \| AuthOutcome::Idle { .. }))`. An ended flow's delivery is answered `LOGIN_ENDED` at once. Otherwise the delivery is awaited inside a `select!` against `cancel.cancelled()`, so an `x` pressed during the re-probe window still ends the wait. `commands.close()` stays **first** (as today), then the delivery is settled, then the queue is drained, then the event drain and the re-probe. |
| **F-4** | **Major** | T2 | D267/D269: the limits reach `run_auth` "through a new `AuthArgs` field, set by `auth_start`". | `auth_start` (`:1533-1608`) builds `AuthArgs` from `self`; the runtime has no limits to copy. The plan's T2 cases need milliseconds, and `login_runtime` (`:7090-7094`) must keep its signature (plan condition 3). | **D278**: `AgentRuntime.deliver_limits: DeliverLimits` (default in `new`), `pub fn with_deliver_limits(self, DeliverLimits) -> Self` beside `with_opener` (`:681-684`), copied into `AuthArgs.deliver_limits`. A case writes `login_runtime(dir).with_deliver_limits(..)`. |
| **F-5** | Major | T2 | The silent-listener cases use "milliseconds" limits. | `cancel_is_served_while_a_delivery_is_in_flight` proves `x` does not wait for the delivery; with millisecond limits the delivery would time out first and the case would prove nothing. And no listed case reaches `DeliverError::Timeout` through the worker, which is the only reason the field exists. | The cancel and second-deliver cases use the **default** limits (15 s) and assert `Cancelled` inside 5 s. A new case, `a_silent_listener_times_the_delivery_out_and_the_login_keeps_running`, is the millisecond one (D286). |
| **F-6** | Major | T2 | R-10: the delivery is answered before `Done`. No case pins it. | The ordering is D269(d)'s whole point and is observable: a listener that reads the request, lets the adapter complete, and closes without answering. | New case `a_delivery_the_listener_drops_is_answered_before_the_logins_result` (D286). |
| **F-7** | Major | T4 | "asserts … that the note line shows the listener's answer, that the `on this box` cell ends at `ready`". | `Delivered` sets the note and `Done` overwrites it with `logged in: ready` (`agents.rs:716-724`). If the adapter completes in the same `drive()` as the delivery answer, no rendered frame ever shows the note, and the case is a race. | **D284**: the T4 case holds the adapter itself. The listener answers and the **test** creates the `FIXTURE_WAIT_FOR` marker only after `until` has seen the note line. |
| **F-8** | Major | T3 | D270: `p` "while `Running` and not `cancelling`". | Undefined: `p` in `Starting`, `p` in `Running { cancelling: true }`, and a `Cancelling` frame arriving while the field is open. | **D282**: `p` is bound whenever `auth_in_flight()` (as `o` is, `agents.rs:1857-1860`). `Starting`, or `Running` with no advertised redirect → `NO_LOOPBACK_REDIRECT`. `cancelling` → `PASTE_CANCELLING`. `delivering` → `DELIVERY_IN_FLIGHT`. An `AuthFrame::Cancelling` closes an open field (its buffer is wiped on drop). |
| **F-9** | Minor | T1 | R-10: "`deliver` treats EOF or a reset **after** a status line as the end of the body". | A T1 test needs a reset on demand. tokio 1.53.1 has `TcpStream::set_zero_linger()` (`net/tcp/stream.rs:1353`), not deprecated; `set_linger` is deprecated (`:1319`). | The reset case calls `set_zero_linger()` then drops the stream. No `#[allow(deprecated)]`. |
| **F-10** | Minor | T1 | D273's verbatim rule: "every `PasteError` … is a fixed sentence naming a host, a port or a rule — never a byte that was pasted". | D265(9)'s `BrowserError { error }` quotes the pasted `error` value (filtered to `[A-Za-z0-9._-]`, at most 64 chars). The plan contradicts itself. | **D285**: the rule's bullet gains "…never a byte that was pasted, except an OAuth `error` value such as `access_denied`, filtered to `[A-Za-z0-9._-]` and cut to 64 characters". §2.2 gives the full text. |
| **F-11** | Minor | T1 | "the two name tables gain `(AuthDeliver, "auth_deliver")`" **and** a separate `auth_deliver_without_a_runtime_is_refused_by_name`. | The second table is `try_serve_without_a_runtime_refuses_all_four_by_name` (`store_worker.rs:3537`): its name is a count. Adding a fifth row makes the name false, and the separate test would duplicate the row. | **D283**: `name_arms_are_stable` (`:3454`) gains the `AuthDeliver` assertion; the `all_four` table stays at four; `auth_deliver_without_a_runtime_is_refused_by_name` is the fifth. |
| **F-12** | Minor | T3 | D270's render: the existing lines, `link: …`, then a `redirect:` line, then two field lines (or a `delivering` line). | The pane is a `Constraint::Length(pane.len())` over a `Min(3)` table (`agents.rs:1990-2000`). With six stderr lines that is 10 lines, 3 more than MOD-21's worst case. And `auth_pane(&self, theme)` (`:784`) has no width, which `TextField::line(width, ..)` needs. | **D282**: the prompt line and the field line **replace** the `redirect:` line, and `delivering to …` replaces it too. The worst case is 6 + link + 2 = 9. `auth_pane` becomes `auth_pane(&self, width: u16, theme: &Theme)` and `pane` (`:1078`) passes `width`. |
| **F-13** | Minor (record) | T3 | New snapshot `settings__agents_paste_redirect`. | No existing snapshot renders a running login (plan row 39), and `HINT_AUTH_RUNNING` is drawn only in `Starting`/`Running` (`agents.rs:1098`). | `cargo insta pending-list` after T3 shows **exactly one** entry. Anything else is a regression. |
| **F-14** | Minor | T2 | `AuthArgs` gains a field. | `a_command_still_queued_when_the_flow_ends_is_refused_rather_than_dropped` (`:8188-8250`) builds `AuthArgs` as a struct literal. | T2 adds `deliver_limits: DeliverLimits::default()` there, in its red commit, or nothing compiles. |
| **F-15** | Minor | T3 | "`AuthState::Running` gains three fields." | `Running` is constructed at two sites: `send_choice` (`agents.rs:~612-618`) and `begin_auth_cancel` (`:634-640`). | Both write `advertised: None, paste: None, delivering: false`. |
| **F-16** | Minor | T1 | "`auth/mod.rs`: `pub mod loopback;` plus a module-doc line". | `auth/mod.rs:8-11` says "Nothing here can carry a credential". After T1 one submodule does. | T1 rewords that sentence (§2.1). No `pub use` of loopback items anywhere (D279). |

---

## 1. Build order and validation, at a glance

| Task | Crates | Commits (each compiles) | Gate (all `--test-threads=1`) |
|---|---|---|---|
| T1 loopback module + protocol seam | htui-agent, htui, manifests | 3 (§2.6) | fmt; `htui-agent --test loopback`; `--test extensibility`; `htui --lib store_worker`; `htui --test settings`; workspace clippy; `cargo tree`; lock diff |
| T2 the worker | htui | 2 (§3.5) | fmt; `htui --lib agent_worker`; `htui --lib store_worker`; clippy `-p htui` |
| T3 the pane | htui | 3 (§4.6) | fmt; `htui --lib text_field`; insta (one pending); `htui --test settings`; `htui --test auth`; snapshots 116; clippy `-p htui` |
| T4 end to end | htui | 1 (§5.3) | fmt; `htui --test auth`; then §6 |
| end | — | — | §6 workspace gate |

Order: T1 → T2 → T3 → T4 on `hr/MOD-22`, each task starting from the previous task's last commit.

---

## 2. T1: the loopback module and the protocol seam (D263–D268, D273; D275–D281, D283, D285)

**Files (complete)**: `Cargo.toml`, `Cargo.lock`, `crates/htui-agent/Cargo.toml`,
`crates/htui-agent/src/auth/loopback.rs` (new), `crates/htui-agent/src/auth/mod.rs`,
`crates/htui-agent/tests/loopback.rs` (new), `crates/htui/src/store_worker.rs`,
`crates/htui/src/testkit.rs`, `crates/htui/src/ui/tabs/settings/agents.rs` (one arm).

### 2.1 Manifests and `auth/mod.rs`

Root `Cargo.toml` `[workspace.dependencies]`, after `shell-words` (`:133`):

```toml
# MOD-22 D264/D267 (OQ-5): the login pane's paste-back reads a `redirect_uri` nested in another
# URL's query and compares loopback hosts. 2.5.8 is already compiled as a dependency of `reqwest`
# and `tower-http`, so this promotes a transitive crate and adds nothing.
url                    = "2.5"
```

`crates/htui-agent/Cargo.toml` `[dependencies]`: `:24`'s tokio list becomes `["sync", "rt", "time",
"process", "io-util", "fs", "net"]`, and after `similar` (`:34`):

```toml
# MOD-22 T1: `auth/loopback.rs` parses the advertised redirect (`url`), holds a pasted authorization
# code in a wiped buffer (`zeroize`) and delivers it over a plain loopback socket (tokio `net`).
url                   = { workspace = true }
zeroize               = { workspace = true }
```

`crates/htui-agent/src/auth/mod.rs`:
- `pub mod loopback;` between `pub mod browser;` and `pub mod run;` (`:18-20`, alphabetical).
- **No** `pub use loopback::…` (D279). Callers write `htui_agent::auth::loopback::X`.
- `:8-11` becomes: "Nothing here can carry a credential **but [`loopback`]**: every other field is an
  id, a sentence the agent already wrote to its own stderr, or a status (`R-SEC-2`, `R-ID-7`).
  `loopback` holds a pasted authorization code for the length of one request, and states its own
  rule (MOD-22 D273)." The rest of the paragraph is unchanged.
- If `mod.rs` ever names the `url` **crate** (a doc link, a path), it spells it `::url::Url`: inside
  `auth/`, a bare `url::` is the sibling module `auth::url` (E0425, plan row 51). The doc line above
  names no crate item, so today nothing needs it.

### 2.2 `crates/htui-agent/src/auth/loopback.rs`: module doc

Order: a "what this is" preface, then the credential rule. Preface (text, adjust wording only):

```text
//! MOD-22: completing a loopback OAuth login from a box the browser cannot reach.
//!
//! An agent's own login redirects the browser to a listener **inside the adapter**, on the loopback
//! of the box `htui` runs on (RFC 8252 §7.3). When the browser is on another machine, that address
//! fails there, and the user holds a `http://127.0.0.1:<port>/?code=…&state=…` their browser could
//! not open. This module is the three things the login pane needs to finish the job on this box:
//! [`Advertised::from_auth_url`] reads which loopback address the running flow is listening on,
//! from the standard `redirect_uri` parameter of the link the adapter printed (never from a
//! vendor's sentence, `R-AGT-5`); [`validate`] checks a pasted address against it; [`deliver`]
//! issues that one request to that port and reports what the listener said.
//!
//! Pure except for [`deliver`], which is the only function here that touches a socket and runs on
//! the login's own task, never on the UI task (`R-NF-3`). No DNS lookup ever happens.
```

Then the credential rule, which is the plan's D273 text with **one** amendment (D285, the first
bullet's last clause):

```text
//! # The credential rule (`R-SEC-2`, `R-ID-7`)
//!
//! The address a user pastes here carries an OAuth authorization `code`; for the length of one
//! request it is a credential, and everything below is written around that:
//!
//! - **Never logged.** No `tracing` field, no panic message and no `Debug` or `Display` prints it:
//!   [`RedirectUrl`] prints `RedirectUrl(<redacted>)`, [`Delivery`] and [`Advertised`] print a host
//!   and a port, and every [`PasteError`] and [`DeliverError`] is a fixed sentence naming a host, a
//!   port or a rule — never a byte that was pasted, except an OAuth `error` value such as
//!   `access_denied`, filtered to `[A-Za-z0-9._-]` and cut to 64 characters.
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
//! socket buffers are not wiped, a paste longer than [`PASTE_MAX`] may leave a reallocated prefix in
//! the pane's field before it is refused, and an adapter that prints the callback on its own stderr
//! is shown by MOD-21's pane as every stderr line is.
```

The module doc names no `VOCABULARY` string (no vendor host, method or variable).

### 2.3 `loopback.rs`: the public surface (exact)

```rust
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use url::{Host, Url};           // resolves to the crate here (plan row 51)
use zeroize::Zeroizing;

/// D265 (1): the longest paste `validate` accepts, in bytes after trimming. Also the capacity the
/// pane's masked field is opened with (D275), so an acceptable paste never reallocates there.
pub const PASTE_MAX: usize = 8 * 1024;
/// D267: the most `deliver` reads of an answer, head and body together.
pub const RESPONSE_CAP: usize = 16 * 1024;
/// D268: the widest excerpt `ListenerReply::said` holds, in chars, the `…` included.
pub const EXCERPT_WIDTH: usize = 80;
/// Shared by the pane (`p`) and the worker (`AuthCommand::Deliver`).
pub const NO_LOOPBACK_REDIRECT: &str =
    "this login's link advertises no loopback redirect, so there is nothing to paste";
/// Shared by the pane (`p`) and the worker (a second `Deliver`).
pub const DELIVERY_IN_FLIGHT: &str =
    "the pasted address is still being delivered; wait for the listener's answer";

/// D266: the pasted address, as typed. The `htui_store::Dsn` shape.
#[derive(Clone)]
pub struct RedirectUrl(Zeroizing<String>);
impl RedirectUrl {
    /// Wraps what the pane's field `take()`s; the allocation moves, nothing is copied.
    #[must_use] pub fn new(text: String) -> Self;
    /// The text. **Private** (a fn of this module, not `pub(crate)`): only `validate` reads it.
    fn as_str(&self) -> &str;
}
impl core::fmt::Debug for RedirectUrl { /* "RedirectUrl(<redacted>)" */ }
// No Display, no Serialize, no Deref, no PartialEq.

/// D264: the loopback redirect a running login's link advertised.
#[derive(Clone, PartialEq, Eq)]
pub struct Advertised {
    host: String,          // `Url::host_str()`: "127.0.0.1", "[::1]", "localhost" (lower-cased by `url`)
    port: u16,             // `port_or_known_default()`, so 80 when the redirect names none
    path: String,          // `Url::path()`, "/" at least
    state: Option<String>, // the link's own `state`; plain, see D281
}
impl Advertised {
    /// D264 (1)–(4). `None` for a non-URL, a link with no or an unparseable `redirect_uri`, a
    /// redirect that is not `http`, or a host that is not `127.0.0.0/8`, `::1` or `localhost`
    /// (ASCII case-insensitive). The **first** `redirect_uri` in `query_pairs()` wins; `state` is
    /// the first `state` pair of the link, `None` when absent or empty.
    #[must_use] pub fn from_auth_url(link: &str) -> Option<Self>;
    /// As `url` serialises it: `127.0.0.1`, `[::1]`, `localhost`.
    #[must_use] pub fn host(&self) -> &str;
    #[must_use] pub const fn port(&self) -> u16;
    #[must_use] pub fn path(&self) -> &str;
    /// `host:port`: what the pane draws and what every sentence names (`127.0.0.1:39879`).
    #[must_use] pub fn target(&self) -> String;
}
impl core::fmt::Debug for Advertised {
    /* debug_struct("Advertised").host.port.path, state: "<redacted>" when Some, None otherwise */
}

/// D265's product: everything one `GET` needs, and nothing a caller can read back but its target.
pub struct Delivery {
    targets: Vec<SocketAddr>,                // IPv4 literal → [v4]; `[::1]` → [v6]; `localhost` → [127.0.0.1, [::1]]
    host_header: String,                     // `Advertised::target()`
    request_target: Zeroizing<String>,       // pasted `path` + `?` + pasted raw `query`; no fragment
    secrets: Vec<Zeroizing<String>>,         // `code`/`state`, raw and percent-decoded, non-empty, deduped
}
impl Delivery { #[must_use] pub fn target(&self) -> String; }   // host_header
impl core::fmt::Debug for Delivery { /* debug_struct("Delivery").field("target", ..).finish_non_exhaustive() */ }

/// D265: why a paste was refused. Every `Display` is fixed text (D273, D285).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PasteError {
    #[error("nothing was pasted")]                                                                  Empty,
    #[error("the pasted text is longer than 8192 bytes; paste the address bar only")]               TooLong,
    #[error("the pasted text is not an address")]                                                   NotAUrl,
    #[error("the address is not plain http; a loopback redirect always is")]                       NotHttp,
    #[error("the address carries a user name or password; a loopback redirect never does")]        HasUserinfo,
    #[error("the address is for another host; this login is listening on {advertised}")]          WrongHost { advertised: String },          // target
    #[error("the address is for port {pasted}; this login is listening on {advertised}")]          WrongPort { pasted: u16, advertised: u16 },
    #[error("the address is for another path; this login is listening on {advertised}")]          WrongPath { advertised: String },          // target + path
    #[error("the browser came back with `{error}` instead of a code; the login was not granted")]  BrowserError { error: String },             // filtered, ≤ 64
    #[error("the address has no code; copy the whole address the browser could not open")]        MissingCode,
    #[error("the address has no state; copy the whole address the browser could not open")]       MissingState,
    #[error("the address carries {0} more than once")]                                              RepeatedParameter(&'static str),           // "code" | "state"
    #[error("the address is from an earlier attempt (its state is not this login's); finish the consent again from the link above")] StaleState,
}
// `TooLong`'s 8192 is written as a literal because `thiserror` cannot format a const; a unit test in
// `tests/loopback.rs` pins `TooLong.to_string().contains(&PASTE_MAX.to_string())`.

/// D267: the two deadlines. Production is `Default`; a test injects milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliverLimits {
    /// Per connect attempt.
    pub connect: Duration,
    /// From the connect to the end of the read: covers the write and the whole answer.
    pub response: Duration,
}
impl Default for DeliverLimits { /* connect: 2 s, response: 15 s */ }

/// D268: what the listener said. Crosses `AuthFrame::Delivered`, so every field is safe to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerReply {
    /// `Advertised::target()`: `127.0.0.1:39879`.
    pub target: String,
    /// The status code, `100..=599`.
    pub status: u16,
    /// The reason phrase, sanitised as `said` is and cut to 40 chars; may be empty.
    pub reason: String,
    /// The `<title>`, else the first non-blank body line with tags stripped; `None` when empty.
    pub said: Option<String>,
    /// For a `3xx` with an absolute `Location`: its host only. A relative one is `None`.
    pub location_host: Option<String>,
}
impl ListenerReply {
    /// `{target} answered {status}` + ` {reason}` (when non-empty) + `, redirecting to {host}`
    /// (when `location_host`) + `: "{said}"` (when `said`). Byte-exact, pinned in T1.
    #[must_use] pub fn summary(&self) -> String;
}

/// D268 + D276: why nothing usable came back. Every `Display` names `target` and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeliverError {
    #[error("nothing is listening on {target} \u{2014} the login may have ended; x cancels it and a new attempt listens elsewhere")]
    NothingListening { target: String },
    #[error("{target} did not answer within {after:?}")]
    Timeout { target: String, after: Duration },
    #[error("{target} answered, but not in HTTP")]
    NotHttp { target: String },
    #[error("{target} closed the connection without answering; if the login completed, its result follows")]
    ClosedWithoutAnswer { target: String },
    #[error("delivering to {target} failed: {kind}")]
    Io { target: String, kind: std::io::ErrorKind },
}

/// D265, rules (1)–(12) in order, the first failure wins. Pure: no I/O, no DNS.
pub fn validate(url: &RedirectUrl, advertised: &Advertised) -> Result<Delivery, PasteError>;

/// D267: one `GET`, following nothing, reading at most `RESPONSE_CAP`.
pub async fn deliver(delivery: Delivery, limits: DeliverLimits) -> Result<ListenerReply, DeliverError>;
```

**`validate`, exactly** (D265 with the tree's details):
1. `text = url.as_str().trim()`. Empty → `Empty`; `text.len() > PASTE_MAX` → `TooLong`.
2. No `"://"` in `text` → `Zeroizing::new(format!("http://{text}"))`, else a `Zeroizing` copy.
   Needed because `Url::parse("localhost:39879/?…")` **succeeds** with scheme `localhost` (plan
   row 47).
3. `Url::parse` or `NotAUrl`. 4. `scheme() == "http"` or `NotHttp`. 5. `username().is_empty() &&
   password().is_none()` or `HasUserinfo`. 6. `host_str() == Some(advertised.host())` (both
   lower-cased and IP-normalised by `url`, so `127.1` compares as `127.0.0.1`) or `WrongHost {
   advertised: advertised.target() }`. 7. `port_or_known_default() == Some(port)` or `WrongPort {
   pasted, advertised }`; a missing port reads 80. 8. `path() == advertised.path()` or `WrongPath {
   advertised: format!("{}{}", target, path) }`.
9. Any `error` pair in `query_pairs()` → `BrowserError { error }`, the value filtered to
   `[A-Za-z0-9._-]` and cut to 64 chars.
10. `code` pairs: 0 or only empty → `MissingCode`; more than one → `RepeatedParameter("code")`.
11. `state` likewise → `MissingState` / `RepeatedParameter("state")`.
12. `advertised.state` is `Some(s)` and the pasted state `!= s` → `StaleState`.
- On success: `targets` from `advertised`'s host (never from the pasted text); `request_target`
  = `url.path()` + `?` + `url.query()` (the **raw** query as `url` holds it; the fragment is
  dropped); `secrets` = the decoded `code` and `state` plus their raw substrings, split out of
  `url.query()` on `&` and the first `=` (each `Zeroizing`, empty ones skipped, deduped).

**`deliver`, exactly** (D267, D276):
- **Connect**: for each of `targets` in order, `timeout(limits.connect, TcpStream::connect(addr))`.
  `ConnectionRefused` → try the next; none left → `NothingListening`. A connect timeout →
  `Timeout { after: limits.connect }`. Any other error → `Io { kind }`.
- **Deadline**: `let deadline = Instant::now() + limits.response` taken right after the connect;
  the write and every read run under `timeout_at(deadline, …)`.
- **Write** from a `Zeroizing<Vec<u8>>`: `GET {request_target} HTTP/1.1\r\nHost: {host_header}\r\n
  User-Agent: htui\r\nAccept: text/html, text/plain, */*\r\nConnection: close\r\n\r\n` (one line per
  header, exactly this order). `BrokenPipe`/`ConnectionReset`/`ConnectionAborted` on the write →
  `ClosedWithoutAnswer`; any other error → `Io`.
- **Read** into a `Zeroizing<Vec<u8>>` until one of:
  - (a) EOF (`Ok(0)`);
  - (b) `ConnectionReset`/`ConnectionAborted`;
  - (c) `RESPONSE_CAP` bytes in hand;
  - (d) the head is complete and the body is complete by its framing: `Content-Length` bytes read,
    or a chunked body's terminal `0` chunk seen;
  - (e) the deadline.

  Then decide:

  | Bytes in hand when the read ended | Result |
  |---|---|
  | none, by (a) or (b) | `ClosedWithoutAnswer` |
  | none, by (e) | `Timeout { after: limits.response }` |
  | some, not starting `HTTP/1.` or with no 3-digit status in `100..=599` on a complete first line | `NotHttp` |
  | an incomplete first line, by (a), (b) or (e) | `NotHttp` for (a)/(b), `Timeout` for (e) |
  | a complete status line, by **any** of (a)–(e) | `Ok(ListenerReply)` from what is in hand (**R-10**: a close, a reset or the deadline after the status line is the end of the answer, never an error) |

  Any other read error → `Io { kind }`.
- **Parse**: headers are case-insensitive, and a partial head is fine. The body is de-chunked when
  `Transfer-Encoding: chunked`; malformed chunk sizes end the body at what was decoded. Body bytes
  are read as lossy UTF-8.
- **Excerpt** (D268, D289): (1) replace every `secrets` entry with `…` in the decoded body, in the
  reason and in the `Location` value; (2) `said` is the text between the first `<title…>` and
  `</title>` (ASCII case-insensitive), else the first body line that is non-blank after tags
  (`<…>`) are stripped; (3) drop control chars, collapse whitespace runs to one space, trim; (4)
  blank the secrets again; (5) cut to `EXCERPT_WIDTH` chars, the last being `…` when cut. HTML
  entities are **not** decoded. `reason` gets (1), (3), (4) and a 40-char cut. `location_host` is
  `Url::parse(location).ok()?.host_str()` for status `300..=399`, else `None`.
- No redirect is followed and no second request is ever made.

### 2.4 `crates/htui/src/store_worker.rs`, `testkit.rs`, `agents.rs`

```rust
use htui_agent::auth::loopback::{ListenerReply, RedirectUrl};

// StoreRequest, after AuthOpen (:324-333), before AuthCancel:
    /// Relay the address the browser could not open to the running login's own loopback
    /// listener (MOD-22 D263, D269).
    ///
    /// Served **inside the live flow's task**, never on the loop: one plain `GET` to the port the
    /// flow's link advertised, while the flow keeps serving its wire, its stderr and `x`.
    /// Answered exactly once at its own `seq`: [`AuthFrame::Delivered`] with what the listener
    /// said, or [`StoreReply::Failed`] naming the rule the paste broke or why nothing answered.
    /// The flow then ends through MOD-21's own path. The address carries an authorization code,
    /// so it travels as a [`RedirectUrl`], whose `Debug` is `RedirectUrl(<redacted>)`; the rule it
    /// is held to is the credential rule of `htui_agent::auth::loopback`.
    AuthDeliver {
        /// The pasted address.
        url: RedirectUrl,
    },
// name(): StoreRequest::AuthDeliver { .. } => "auth_deliver", after "auth_open" (:835)

// AuthFrame, after Opened (:1239):
    /// What the login's loopback listener answered a delivered paste (MOD-22 D268).
    ///
    /// An **answer**, not an outcome: a `4xx` or `5xx` is reported here too, and the adapter
    /// decides what it means. The flow keeps running either way, and ends with its own terminal
    /// frame. Carries a status, a reason, a host and an excerpt with the pasted `code` and `state`
    /// blanked. Not a terminal frame.
    Delivered(ListenerReply),
```

Two docs move with the variants:
- **`:1211-1214`** becomes "**Nothing here can hold a credential** (`R-SEC-2`, `R-ID-7`): every field
  is an id the agent advertised, a sentence the agent itself wrote to its own stderr, a link it
  printed, a status the probe decided, or what a loopback listener answered a delivered paste, with
  the pasted `code` and `state` blanked out of it (MOD-22 D268). `htui` never reads the credential a
  login leaves behind — it asks the probe whether one exists."
- **`:1339-1343`**: "…the three install requests, MOD-21's four login ones and MOD-22's delivery need
  the runtime that owns their tasks, so all **sixteen** are served ahead of this function…".

Or-patterns gain `| StoreRequest::AuthDeliver { .. }` after `AuthOpen` at `store_worker.rs:1358`
(no-runtime arm), `:1973` (runtime arm) and `testkit.rs:293`. `AuthFrame` is matched exhaustively
only in `agents.rs::on_auth_frame` (plan row 22). T1's one arm goes after `Opened` (`:715`):

```rust
            // MOD-22 D268: what the listener said, on the note line. T3 adds the rest.
            AuthFrame::Delivered(reply) => self.notice = Some(reply.summary()),
```

Until T2 lands, `AuthDeliver` reaches `AgentRuntime::serve`'s wildcard (`agent_worker.rs:1117-1120`,
"not a chat request"). Nothing in T1 exercises that path.

### 2.5 Tests (first)

**`crates/htui-agent/tests/loopback.rs`** (new; not `cfg(unix)`: no process, only sockets). Helpers
are per-file:
- `const CODE: &str = "CODE-SENTINEL-4f1c"` and `const STATE: &str = "STATE-SENTINEL-9a2e"`.
- `fn link(host_port: &str, state: &str) -> String` builds
  `https://auth.example.invalid/o?client_id=c&redirect_uri=http%3A%2F%2F{host_port}%2F&state={state}`.
- `fn advertised(port) -> Advertised` is `from_auth_url(&link(&format!("127.0.0.1:{port}"), STATE)).unwrap()`.
- `fn paste(text: &str) -> RedirectUrl`.
- `async fn answering(reply: Vec<u8>) -> (u16, JoinHandle<String>)` binds `127.0.0.1:0`, accepts
  **exactly one** connection, reads the head to `\r\n\r\n`, writes `reply`, shuts down, and returns
  the head.

| Test | Asserts |
|---|---|
| `advertised_is_read_from_a_percent_encoded_redirect_uri` | **The first red test.** The plan's link → `host() == "127.0.0.1"`, `port() == 39879`, `path() == "/"`, `target() == "127.0.0.1:39879"`; a paste with `state=S1` validates and one with `state=S2` is `StaleState` (the state is private, so it is read through `validate`) |
| `advertised_accepts_localhost_and_the_ipv6_loopback` | `LOCALHOST`, `localhost`, `127.0.0.2`, `%5B%3A%3A1%5D` (host `[::1]`); a redirect with no port has port 80; `/oauth2callback` is kept as the path |
| `advertised_is_none_for_https_a_lan_host_a_name_no_redirect_uri_or_no_url` | `https://127.0.0.1…`, `192.168.1.2`, `example.invalid`, `[::ffff:127.0.0.1]`, no `redirect_uri`, an unparseable one, and `not a url` are all `None` |
| `the_newest_link_is_not_this_modules_business` | Two links with different ports give two independent values; calling the first again gives the first port. There is no hidden state |
| `a_valid_paste_becomes_a_delivery_for_the_advertised_target` | `Ok`; `delivery.target() == "127.0.0.1:39879"`; leading and trailing whitespace are accepted |
| `a_paste_without_a_scheme_is_read_as_http` | `127.0.0.1:39879/?code=…&state=…` → `Ok` |
| `wrong_port_names_both_ports_and_nothing_else` | `to_string() == "the address is for port 50651; this login is listening on 39879"` byte for byte |
| `stale_state_is_refused` | `StaleState`; with a link without `state`, any non-empty pasted `state` is `Ok` (OQ-3's rule) |
| `an_error_redirect_is_named_by_its_error_value` | `?error=access_denied&state=…` → `BrowserError { error: "access_denied" }`; `error=a%3Cb%3Ec` filters to `abc`; a 100-char value is cut to 64 |
| `each_rule_refuses_with_its_own_variant` | One row per D265 rule, including `code=` (empty → `MissingCode`), `code=a&code=b`, `state` twice, `https://`, `http://u:p@…`, `127.0.0.2` against `127.0.0.1`, `localhost` against `127.0.0.1`, path `/x`, `PASTE_MAX + 1` bytes; and `TooLong.to_string()` contains `8192` |
| `every_paste_error_sentence_is_free_of_pasted_text` | For every variant, input built with `CODE`/`STATE` in the `code`/`state` slots (for `BrowserError`, `error=access_denied` plus both sentinels): `to_string()` and `{:?}` contain neither sentinel |
| `redirect_url_debug_is_redacted` | `format!("{:?}", paste(..)) == "RedirectUrl(<redacted>)"`, and its clone's too |
| `delivery_and_advertised_debug_print_host_and_port_only` | Neither `{:?}` contains `CODE` or `STATE`; `Advertised`'s contains `<redacted>`; `Delivery`'s contains `127.0.0.1:39879` |
| `deliver_sends_one_get_with_the_pasted_path_query_and_the_advertised_host` | The paste `  http://127.0.0.1:P/?code=CODE&state=STATE#frag \n` → the head is **byte for byte** `GET /?code=CODE-SENTINEL-4f1c&state=STATE-SENTINEL-9a2e HTTP/1.1\r\nHost: 127.0.0.1:P\r\nUser-Agent: htui\r\nAccept: text/html, text/plain, */*\r\nConnection: close\r\n\r\n` (no fragment) |
| `deliver_reports_the_status_and_the_title_of_an_html_answer` | `200 OK` + `<html><head><TITLE>Signed  in</TITLE>…` → `status 200`, `reason "OK"`, `said Some("Signed in")`, `summary() == "127.0.0.1:P answered 200 OK: \"Signed in\""` |
| `deliver_reports_the_first_text_line_when_there_is_no_title` | `400 Bad Request`, a `text/plain` body `\n\n  <b>invalid</b> state\nmore` → `said Some("invalid state")` |
| `deliver_decodes_a_chunked_body` | A chunked title split across chunks → the whole title |
| `deliver_blanks_an_echoed_code_and_state_in_the_excerpt` | The paste carries `code=CODE-SENTINEL%2D4f1c` (decoded: `CODE-SENTINEL-4f1c`); the title echoes the raw form, the decoded form and `STATE`. `said` and `summary()` contain none of the three and contain `…` |
| `deliver_cuts_the_excerpt_at_eighty_columns` | A 300-char title → `said` is 80 chars, the last `…` |
| `deliver_does_not_follow_a_redirect_and_names_the_location_host` | `302 Found`, `Location: https://example.com/done?code=CODE` → `location_host Some("example.com")`, `summary()` ends `, redirecting to example.com`, no sentinel; the listener accepted exactly one connection (a second `accept` times out after 200 ms) |
| `deliver_to_a_closed_port_is_nothing_listening` | Bind, read the port, drop → `NothingListening`; the sentence equals D268's |
| `deliver_times_out_on_a_silent_listener` | Accept and hold; `DeliverLimits { connect: 500 ms, response: 100 ms }` → `Timeout { after: 100 ms }` inside 2 s |
| `deliver_treats_a_close_after_the_status_line_as_the_answer` | Writes `HTTP/1.1 200 OK\r\n`, then closes → `Ok`, `status 200`, `said None` (R-10) |
| `deliver_treats_a_reset_after_the_status_line_as_the_answer` | The same, then `set_zero_linger()` and drop (F-9) → `Ok` |
| `a_listener_that_closes_without_answering_is_closed_without_answer` | Read the head, drop → `ClosedWithoutAnswer`; the sentence contains `if the login completed` |
| `deliver_stops_at_content_length_without_waiting_for_close` | `Content-Length: 5`, then hold the socket open 10 s → `Ok` well inside 1 s under `response: 15 s` |
| `deliver_reads_no_more_than_the_cap` | The listener writes 64 KiB with no framing and holds → `Ok` inside 2 s, `said` ≤ 80 chars |
| `a_non_http_answer_is_not_http` | `SSH-2.0-x\r\n` then close → `NotHttp` |
| `localhost_falls_back_to_the_ipv6_loopback` | Bind `[::1]:0` (on failure `eprintln!` the reason and return, R-6); advertised `localhost:P` → the head arrives on the IPv6 listener with `Host: localhost:P` |
| `the_listener_reply_summary_reads_status_reason_redirect_and_excerpt` | Pure: four literal `ListenerReply`s → four byte-exact `summary()`s, including an empty `reason` (`… answered 204`) |

**`store_worker.rs` tests**:
- `name_arms_are_stable` (`:3454`) gains `assert_eq!(StoreRequest::AuthDeliver { url: RedirectUrl::new("http://127.0.0.1:1/".to_owned()) }.name(), "auth_deliver")`.
- `an_auth_deliver_request_debugs_without_its_url`: the request's `{:?}`, and a `RequestEnvelope`'s
  around it, contain `RedirectUrl(<redacted>)` and neither sentinel.
- `auth_deliver_without_a_runtime_is_refused_by_name`: `serve(&demo(), &AuthDeliver{..})` →
  `Failed { request: "auth_deliver", message: "no agent runtime in this build" }`.

### 2.6 Commits (T1)

1. **(a) red** — `test(agent): the loopback paste-back cases, red (MOD-22 T1)`. It holds:
   - the manifests and the lock;
   - `auth/mod.rs`;
   - `loopback.rs` with its module doc, every type, constant and `Debug`/`Display` of §2.3, and
     `todo!()` bodies in `Advertised::from_auth_url`, `validate`, `deliver` and
     `ListenerReply::summary` only. No existing path calls them;
   - `tests/loopback.rs`.
2. **(b) green** — `feat(agent): read the advertised loopback redirect and deliver a pasted one
   (MOD-22 D264-D268)`, the bodies.
3. **(c)** — `feat(tui): AuthDeliver and AuthFrame::Delivered on the protocol seam (MOD-22 D263,
   D266)`: §2.4 and its three tests. They are written in the same commit because they cannot compile
   before the variants exist.

Each message ends with the attribution line.

### 2.7 Gate (T1)

```bash
cargo fmt --all -- --check
cargo test -p htui-agent --all-features --test loopback -- --test-threads=1
cargo test -p htui-agent --all-features --test extensibility -- --test-threads=1
cargo test -p htui --all-features --lib store_worker -- --test-threads=1
cargo test -p htui --all-features --test settings -- --test-threads=1        # the one arm; nothing moved
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo tree -p htui-agent -i url -e normal --offline | head -1                # url v2.5.8
git diff f5de3d4 -- Cargo.lock | grep -c '^+'                                 # 3: the `+++` header and the "url", "zeroize" lines
git diff f5de3d4 -- Cargo.lock | grep -c '^+\[\[package\]\]'                  # 0
```

---

## 3. T2: the worker (D269; D277, D278, D286, D287)

**Files (complete)**: `crates/htui/src/agent_worker.rs`.

### 3.1 Types and the runtime

```rust
use futures::future::BoxFuture;
use htui_agent::auth::AuthEvent;                                   // if not already imported
use htui_agent::auth::loopback::{
    self, Advertised, DELIVERY_IN_FLIGHT, DeliverError, DeliverLimits, ListenerReply,
    NO_LOOPBACK_REDIRECT, RedirectUrl,
};

/// What every request a finished login can no longer serve is told (MOD-21's two literals, and
/// MOD-22's two new sites).
const LOGIN_ENDED: &str = "this login has ended";                  // D287: replaces the literals at :1632 and :2962

// AuthCommand (:290-305), after Open:
    /// Relay a pasted redirect to the flow's own loopback listener (MOD-22 D269). Validated again
    /// here against the flow's own record of the advertised redirect, never the pane's; the
    /// address is a credential for the length of the one `GET` (the credential rule of
    /// `htui_agent::auth::loopback`), and `RedirectUrl`'s `Debug` keeps this enum's derive safe.
    Deliver {
        /// The pasted address.
        url: RedirectUrl,
        /// Who to answer, once.
        reply: ReplyAddr,
    },

// AgentRuntime (:372-454), after `opener` (:436):
    /// What a delivered paste may wait for (MOD-22 D267, D278). [`DeliverLimits::default`] in
    /// production; a login case injects milliseconds through
    /// [`with_deliver_limits`](Self::with_deliver_limits).
    deliver_limits: DeliverLimits,
// new() (:544-562): `deliver_limits: DeliverLimits::default(),`. Debug (:456-464) unchanged.

    /// The deadlines a delivered paste runs under (MOD-22 D278): the seam a case needs to reach
    /// `DeliverError::Timeout` without waiting fifteen seconds.
    #[must_use]
    pub fn with_deliver_limits(mut self, limits: DeliverLimits) -> Self;   // after with_opener (:681-684)

// serve(), after the AuthOpen arm (:1109-1115):
            StoreRequest::AuthDeliver { url } => self.auth_command(
                "auth_deliver",
                AuthCommand::Deliver { url: url.clone(), reply: addr },
            ),

// AuthArgs (:2814-2825), after `opener`:
    deliver_limits: DeliverLimits,
// auth_start (:1586-1597): `deliver_limits: self.deliver_limits,`. AuthArgs' Debug unchanged.
```

The `url.clone()` is a `Zeroizing` copy. The envelope's own copy is wiped when the loop drops the
envelope.

### 3.2 `run_auth` (`:2860-3026`), the four changes

```rust
/// A delivery in flight (MOD-22 D269): who asked, and the one `GET`.
struct Delivering {
    reply: ReplyAddr,
    answer: BoxFuture<'static, Result<ListenerReply, DeliverError>>,
}

/// The in-flight delivery's answer, or never.
///
/// `tokio::select!` evaluates a branch's expression even while its precondition is false, so this
/// is an `async fn` that touches the slot only when polled. It takes the slot **after** the answer,
/// never before: when another arm wins, this future is dropped, and the delivery must still be there.
async fn settle(slot: &mut Option<Delivering>) -> (ReplyAddr, Result<ListenerReply, DeliverError>) {
    let Some(delivering) = slot.as_mut() else {
        return std::future::pending().await;
    };
    let answer = delivering.answer.as_mut().await;
    let delivering = slot.take().expect("settled only while a delivery is in flight");
    (delivering.reply, answer)
}

/// A delivery's answer as the reply its request is owed (D268).
fn delivered(answer: Result<ListenerReply, DeliverError>) -> StoreReply {
    match answer {
        Ok(reply) => StoreReply::Auth(AuthFrame::Delivered(reply)),
        Err(err) => StoreReply::Failed { request: "auth_deliver", message: err.to_string() },
    }
}
```

In `run_auth`: destructure `deliver_limits`, and declare `let mut advertised: Option<Advertised> =
None;` and `let mut delivering: Option<Delivering> = None;` beside `listening`/`serving`.

- **(a) events arm**:
  ```rust
  Some(event) => {
      if let AuthEvent::Url(link) = &event
          && let Some(found) = Advertised::from_auth_url(link)
      {
          advertised = Some(found);
      }
      frames.reply(&addr, StoreReply::Auth(auth_frame(event)));
  }
  ```
  Newest wins; a link that parses to nothing leaves the previous value.
- **(b) commands arm**, after `Open`:
  ```rust
  Some(AuthCommand::Deliver { url, reply }) => {
      // A bool, not `&delivering` in the scrutinee: the arm below assigns the slot.
      let in_flight = delivering.is_some();
      let refusal = match (advertised.as_ref(), in_flight) {
          (None, _) => Some(NO_LOOPBACK_REDIRECT.to_owned()),
          (Some(_), true) => Some(DELIVERY_IN_FLIGHT.to_owned()),
          (Some(found), false) => match loopback::validate(&url, found) {
              Ok(delivery) => {
                  delivering = Some(Delivering {
                      reply: reply.clone(),
                      answer: Box::pin(loopback::deliver(delivery, deliver_limits)),
                  });
                  None
              }
              Err(err) => Some(err.to_string()),
          },
      };
      drop(url); // the pasted text ends here; the delivery holds only what the GET needs
      if let Some(message) = refusal {
          frames.reply(&reply, StoreReply::Failed { request: "auth_deliver", message });
      }
  }
  ```
- **(c) new arm**, last in the `biased` order (`running`, events, commands, delivery):
  ```rust
  (reply, answer) = settle(&mut delivering), if delivering.is_some() => {
      frames.reply(&reply, delivered(answer));
  }
  ```
- **(d) after the loop** (D277): `commands.close();` stays first. Then:
  ```rust
  if let Some(pending) = delivering.take() {
      let ended = cancel.is_cancelled()
          || matches!(
              outcome,
              Ok(AuthOutcome::Cancelled | AuthOutcome::Declined | AuthOutcome::Idle { .. })
          );
      let answer = if ended {
          ended_reply("auth_deliver")
      } else {
          tokio::select! {
              biased;
              () = cancel.cancelled() => ended_reply("auth_deliver"),
              answer = pending.answer => delivered(answer),
          }
      };
      frames.reply(&pending.reply, answer);
  }
  ```
  Here `ended_reply(request) = StoreReply::Failed { request, message: LOGIN_ENDED.to_owned() }`, a
  private fn. Then comes the existing drain, whose match gains `AuthCommand::Deliver { reply, .. }
  => ("auth_deliver", reply)`. The event drain, the re-probe and the terminal frame are unchanged.
  The comment says why: the child is reaped by now (`acp::auth::run`), so its own listener's socket
  has already ended (R-10); a listener that is not the child's would otherwise hold `Done` for the
  full response deadline.

`run_auth`'s doc gains one paragraph: the D273 cross-reference plus "a pasted redirect is validated
here against the flow's own record and delivered from this task; every `AuthDeliver` is answered
once, before the flow's own last frame".

`tracing`: nothing added. The `Kept` arm's call is untouched and takes nothing from any of this.

### 3.3 Fixture change

`AGENT_SH` (`:6923-6944`) gains one line after the `FIXTURE_URL` echo and **before** `FIXTURE_HOLD`:

```sh
      if [ -n "$FIXTURE_WAIT_FOR" ]; then while [ ! -e "$FIXTURE_WAIT_FOR" ]; do sleep 0.05; done; fi
```

Every existing case leaves it unset. New per-module helpers, beside `LINK` (`:6902`); `LINK` itself
is unchanged:
- `pub(crate) const CODE: &str = "CODE-SENTINEL-4f1c"` and `pub(crate) const STATE: &str =
  "STATE-SENTINEL-9a2e"`.
- `fn loopback_link(port: u16) -> String` returns
  `https://h.invalid/login?redirect_uri=http%3A%2F%2F127.0.0.1%3A{port}%2F&state=STATE-SENTINEL-9a2e`.
- `fn pasted(port: u16, state: &str) -> RedirectUrl` returns
  `http://127.0.0.1:{port}/?code=CODE-SENTINEL-4f1c&state={state}`.
- `async fn listener() -> (u16, tokio::net::TcpListener)`.
- `async fn deliver_at(runtime, backend, tx, seq, url) -> Served` asserts `Deferred`.
- `async fn replies_until_terminal(rx) -> Vec<(Seq, StoreReply)>` is built on `next_reply`
  (`:7143`), because `next_frame` panics on a `Failed` (plan row 68).

`login_store`/`login_runtime` signatures are **unchanged** (`store_worker.rs:3581`, `:3593` call
them).

### 3.4 Tests (first; `mod auth`, `:6883`)

| Test | Setup | Asserts |
|---|---|---|
| `a_pasted_redirect_reaches_the_advertised_port_and_the_login_completes` | `FIXTURE_KEY=set`, `FIXTURE_CRED=CREDENTIAL`, `FIXTURE_URL=loopback_link(P)`, `FIXTURE_WAIT_FOR=<tmp>/delivered`. A listener task reads the head, answers `200 OK` with `<title>signed in</title>` and `Content-Length`, closes, **then** creates the marker | Deliver at seq 3 after the `Url` frame. The head starts `GET /?code=CODE-SENTINEL-4f1c&state=STATE-SENTINEL-9a2e HTTP/1.1`. `Delivered` (status 200, `said` `Some("signed in")`) at seq 3 **before** `Done { status: Ready }` at seq 2 |
| `a_paste_for_another_port_is_refused_and_nothing_connects` | `FIXTURE_HOLD`; listeners on P (advertised) and Q | Deliver for Q → `Failed { "auth_deliver", "the address is for port Q; this login is listening on P" }` at seq 3; neither listener accepts within 200 ms; `x` → `Cancelled` |
| `a_stale_state_is_refused_by_the_worker_even_if_the_pane_let_it_through` | As above | `pasted(P, "OTHER-STATE")` → the `StaleState` sentence; nothing connects |
| `a_deliver_before_any_loopback_redirect_is_refused` | `FIXTURE_URL=LINK` (the MOD-21 link) | After its `Url` frame: `Failed { "auth_deliver", NO_LOOPBACK_REDIRECT }` |
| `a_deliver_with_no_login_running_is_refused` | No flow | `serve` → `Served::Reply(Failed { "auth_deliver", "no login is running" })`; no pid file |
| `a_second_deliver_while_one_is_in_flight_is_refused` | `FIXTURE_HOLD`; a silent listener (accepts and holds); **default** limits (F-5) | The second deliver at seq 4 → `DELIVERY_IN_FLIGHT` at 4. `x` → the first answered `LOGIN_ENDED` at 3, then `Cancelled` at 2 |
| `cancel_is_served_while_a_delivery_is_in_flight` | As above | `x` → `Cancelling` at its own seq at once. The deliver's `Failed { "auth_deliver", LOGIN_ENDED }` and `Cancelled` both arrive within 5 s of `x` (the response deadline is 15 s). The deliver is answered exactly once. `assert_not_running(pid)` |
| `a_silent_listener_times_the_delivery_out_and_the_login_keeps_running` | `FIXTURE_HOLD`; `with_deliver_limits({ connect: 500 ms, response: 200 ms })` | `Failed { "auth_deliver", m }` with `m.contains("did not answer within")`; `runtime.auth_running()` is still true; `x` → `Cancelled` |
| `a_delivery_the_listener_drops_is_answered_before_the_logins_result` | Like the first case, but the listener reads the head, creates the marker and closes **without answering** | The deliver's `Failed` (the `ClosedWithoutAnswer` sentence) precedes `Done` in arrival order (R-10) |
| `a_deliver_queued_when_the_flow_ends_is_refused_rather_than_dropped` | `a_command_still_queued_…`'s shape (`:8188`): `RefusingDriver`; a `Deliver` at seq 3 queued before the flow is polled; `deliver_limits: DeliverLimits::default()` | `Failed { "auth_deliver", LOGIN_ENDED }` at 3, and the stream still ends `AuthFrame::Failed` at 2 |
| `no_frame_and_no_debug_carries_the_pasted_code` | The first case again | Every reply's `{:?}`, `format!("{runtime:?}")` taken while the flow waits on the marker, `AuthCommand::Deliver`'s `{:?}` and the `StoreRequest`'s `{:?}` contain no `CODE`. `STATE` is allowed: the link shows it |

`a_command_still_queued_when_the_flow_ends_is_refused_rather_than_dropped` gains
`deliver_limits: DeliverLimits::default()` in its literal (F-14).

### 3.5 Commits (T2)

1. **(a) red** — `test(tui): delivered pastes through the live login, red (MOD-22 T2)`. It holds:
   - §3.3 and §3.4;
   - `AuthCommand::Deliver`, `deliver_limits` (runtime, `new`, builder, `AuthArgs`, `auth_start`,
     F-14's literal);
   - the `serve` arm;
   - `LOGIN_ENDED`;
   - in `run_auth`, `todo!("MOD-22 T2")` in the new `Deliver` arms of the commands `select!` arm
     and of the drain. Only the new cases reach them.
2. **(b) green** — `feat(tui): serve AuthDeliver inside the running login (MOD-22 D269)`: §3.2
   (a)–(d), the doc paragraph and the `AuthCommand::Deliver` doc.

### 3.6 Gate (T2)

```bash
cargo fmt --all -- --check
cargo test -p htui --all-features --lib agent_worker -- --test-threads=1
cargo test -p htui --all-features --lib store_worker -- --test-threads=1      # login_store/login_runtime callers
cargo clippy -p htui --all-features --all-targets -- -D warnings
```

---

## 4. T3: the pane (D270–D272; D275, D282)

**Files (complete)**:
- `crates/htui/src/ui/tabs/settings/agents.rs`;
- `crates/htui/src/ui/text_field.rs` (**added**, D275);
- `crates/htui/tests/settings.rs`;
- `crates/htui/tests/auth.rs` (the `:491` hint assertion **only**);
- `crates/htui/tests/snapshots/settings__agents_paste_redirect.snap` (new).

### 4.1 `text_field.rs` (D275)

```rust
    /// An empty masked field reserving `bytes` up front (MOD-22 D275).
    ///
    /// For a secret longer than a DSN: a pasted redirect is hundreds of characters, and every
    /// time a `String` outgrows its capacity it frees the old allocation **unwiped**. Opened at
    /// the longest text its caller will accept, a field never reallocates on the way to a value
    /// that caller could use.
    #[must_use]
    pub fn masked_with_capacity(bytes: usize) -> Self;
    // masked() becomes `Self::masked_with_capacity(256)`; its doc keeps the DSN rationale.
```

The module doc's `:12-18` sentence ("because `masked` reserves 256 bytes") gains "(or what
`masked_with_capacity` was given)". In-module test:
`a_masked_field_opened_with_capacity_takes_a_long_paste_without_reallocating` inserts 600 ASCII
chars into `masked_with_capacity(8192)` and checks that `text.capacity()` is 8192 before and after,
and that `take()` returns the 600 chars.

### 4.2 State and constants (`agents.rs`)

```rust
use htui_agent::auth::loopback::{
    self, Advertised, DELIVERY_IN_FLIGHT, NO_LOOPBACK_REDIRECT, PASTE_MAX, RedirectUrl,
};

/// The hint line while a login is spawning or running (MOD-22 D270: 41 of the 98 columns).
const HINT_AUTH_RUNNING: &str = "o open link \u{b7} p paste redirect \u{b7} x cancel";   // replaces :162
/// The hint line while the paste field is open (MOD-22 D270).
const HINT_PASTING: &str = "Enter sends \u{b7} Esc cancels";
/// What `p` says while the login is being cancelled (D282).
const PASTE_CANCELLING: &str = "this login is being cancelled; there is nothing to paste into";

// AuthState::Running (:282-292) gains, after `cancelling`:
        /// MOD-22 D264: the newest loopback redirect the adapter's links advertised; `p` is
        /// offered only with one, and a paste is pre-checked against it.
        advertised: Option<Advertised>,
        /// MOD-22 D270, D272: the masked paste field, open between `p` and `Enter`/`Esc`. Dropped,
        /// and so wiped, on submit, on cancel and when the flow ends.
        paste: Option<TextField>,
        /// MOD-22 D270: an `AuthDeliver` is unanswered; a second one would make the first's answer
        /// stale in `App::is_fresh`, so `p` is refused until it lands.
        delivering: bool,
```

Both construction sites (`send_choice`, `begin_auth_cancel`, F-15) write `advertised: None, paste:
None, delivering: false`. The module doc gains a paragraph after the MOD-23 one (`:40-47`):

> Since MOD-22 a login can be **finished from another machine**. When the adapter's link advertises
> a loopback `redirect_uri`, the pane names it and `p` opens a masked field. The address the browser
> could not open is pasted there, checked here against that redirect, and sent as one
> `StoreRequest::AuthDeliver`. The login's own task relays it to the listener on this box and answers
> with what the listener said. The pasted address is a credential for the length of that request;
> the rule it is held to is `htui_agent::auth::loopback`'s (MOD-22 D273).

### 4.3 Keys (`on_key`, `:1787-1910`) and replies

- In `on_key`, after the chooser branch (`:1807-1809`): `if matches!(self.auth,
  AuthState::Running { paste: Some(_), .. }) { return self.on_paste_key(key, ctx); }`.
- In the `match`, beside the `o` arm (`:1857-1860`): `KeyCode::Char('p') if self.auth_in_flight() =>
  { self.open_paste(ctx); Handled::Consumed }`. `p` stays unbound otherwise.

`open_paste(&mut self, ctx)` (D282), with the first matching row winning:

| State | Effect |
|---|---|
| `Running { cancelling: true, .. }` | `Action::Error(PASTE_CANCELLING)` |
| `Running { delivering: true, .. }` | `Action::Error(DELIVERY_IN_FLIGHT)` |
| `Running { advertised: Some(_), .. }` | `paste = Some(TextField::masked_with_capacity(PASTE_MAX))` |
| everything else (`Starting`, `Running` with no advertised redirect) | `Action::Error(NO_LOOPBACK_REDIRECT)` |

`on_paste_key(&mut self, key, ctx) -> Handled` mirrors `connection.rs:410-430`:

| `field.on_key(key)` | Effect |
|---|---|
| `Consumed` | `Consumed` |
| `Cancel` (`Esc`) | `paste = None`; `Consumed` |
| `Pass` with `CONTROL` | `Handled::Pass` (so `ctrl-c` still quits) |
| `Pass` otherwise | `Consumed` (`Tab`, `Up`, `Down`, …) |
| `Submit` (`Enter`) | `let url = RedirectUrl::new(field.take());` then `loopback::validate(&url, advertised)`. `Ok(_)` (the `Delivery` is dropped unused) → `paste = None`, `delivering = true`, `ctx.request(StoreRequest::AuthDeliver { url })`. `Err(e)` → `paste = Some(TextField::masked_with_capacity(PASTE_MAX))` (the old buffer was moved out and dies with `url`), `ctx.emit(Action::Error(e.to_string()))`. `Consumed` either way |

So `q`, `x`, `o`, `h`, `l`, `?` and the digits are typed into the field. `captures_input` (`:1776`)
becomes `matches!(self.mode, Mode::Editing(_)) || matches!(self.auth, AuthState::Running { paste:
Some(_), .. })`, so `SettingsTab::on_key` (`settings/mod.rs:323-331`) delegates `h`/`l`.

`on_auth_frame` (`:674-752`):

| Frame | Effect |
|---|---|
| `Url(u)` | `*shown = Some(u.clone())`, and `if let Some(found) = Advertised::from_auth_url(u) { *advertised = Some(found) }` (newest that parses wins, as the worker does) |
| `Delivered(reply)` | T1's arm plus `delivering = false` when `Running` |
| `Cancelling` | also sets `paste = None` (F-8) |
| `Done`, `Refused`, `Cancelled`, `Idle`, `Failed` | unchanged: `auth = Idle`, which drops the field and wipes it |

`on_reply` (`:1912-1988`) gains, beside the `auth_open` exception (`:1972-1975`):
```rust
            // MOD-22 D270: a refused or failed delivery is one request answered no; the login and
            // its link stay, and `p` works again.
            StoreReply::Failed { request, .. } if *request == "auth_deliver" => {
                if let AuthState::Running { delivering, .. } = &mut self.auth {
                    *delivering = false;
                }
            }
```

### 4.4 Render and hint (D282, F-12)

`pane` (`:1078`) calls `self.auth_pane(width, theme)`. The `Running` arm of `auth_pane` draws the
stderr lines (dim), then `link: {url}` (base), then **at most one** of these (the first that
applies):

| Condition | Lines (style) |
|---|---|
| `paste: Some(field)` | `paste the address the browser could not open ({target}):` (base), then `› ` (accent) followed by `field.line(width.saturating_sub(2), true, theme)`'s spans |
| `delivering` | `delivering to {target}…` (dim) |
| `advertised: Some(_)` | `redirect: {target} \u{b7} p pastes the address if the browser cannot reach it` (dim) |

`target` is always `Advertised::target()`, never the pasted text. The worst case is 9 lines.

`hint()` (`:1087-1114`): `AuthState::Running { paste: Some(_), .. } => HINT_PASTING` goes before
the `Starting | Running => HINT_AUTH_RUNNING` arm.

### 4.5 Tests (first)

In `crates/htui/tests/settings.rs`, a new block at the end of the MOD-21 one, just before the
MOD-23 divider (`:2658`):
`// MOD-22 T3: p, the paste field and the delivery (plan D270-D272; blueprint §4)`.

Helpers:
- `const CODE`/`STATE`, as in §3.3.
- `const LOOPBACK_LINK: &str =
  "https://h.invalid/o?redirect_uri=http%3A%2F%2F127.0.0.1%3A39879%2F&state=STATE-SENTINEL-9a2e"`.
  The port is fixed so the snapshot is deterministic; nothing here connects.
- `const PASTE: &str = "http://127.0.0.1:39879/?code=CODE-SENTINEL-4f1c&state=STATE-SENTINEL-9a2e"`.
- `fn running_over(bench, link: &str) -> AgentsSection` is `chooser_over(bench, false, 0)`, then
  `Enter`, then `Url(link)`, then drain.
- The existing `typed` (`:2698`) and `requests_of` (`:3933`).

| Test | Asserts |
|---|---|
| `p_with_a_loopback_redirect_opens_a_masked_field_that_captures_input` | `p` → `Consumed`, `captures_input()`. `q`, `h`, `l`, `x`, `o`, `?` and `1` are each `Consumed`, emit nothing and land in the field (the count reads `(7)`). `ctrl-c` → `Handled::Pass`. No request |
| `p_without_a_loopback_redirect_is_refused_by_name_and_opens_nothing` | Link `https://h.invalid/o` → `errors_of == [NO_LOOPBACK_REDIRECT]`, `!captures_input()`. The same in `Starting` (after `a`, before `Methods`) |
| `p_is_not_bound_outside_a_running_login` | Idle → `Handled::Pass`, nothing emitted |
| `p_while_cancelling_or_delivering_is_refused_by_name` | After `x` → `PASTE_CANCELLING`. After a sent paste → `DELIVERY_IN_FLIGHT`, and still exactly one `AuthDeliver` |
| `the_field_draws_dots_and_a_count_and_never_the_text` | After `typed(PASTE)` the render contains `(73)` (the paste's length, checked with `PASTE.chars().count()`) and `•`, no `CODE`, no `127.0.0.1:39879/?`, and the prompt line names `127.0.0.1:39879` |
| `enter_with_a_valid_paste_sends_auth_deliver_and_closes_the_field` | Exactly one `AuthDeliver { url }`. `validate(&url, &Advertised::from_auth_url(LOOPBACK_LINK).unwrap())` is `Ok`. The emitted `{:?}` and the render lack `CODE`. The pane reads `delivering to 127.0.0.1:39879…`. `!captures_input()` |
| `enter_with_a_wrong_port_a_stale_state_or_no_code_is_refused_locally_and_sends_nothing` | Three pastes → each exact `PasteError` sentence as `Action::Error`; no request; the field is still open with count `(0)` |
| `esc_closes_the_field` | `Esc` → `!captures_input()`, the redirect line is back, no request, the login still running (`x` still sends `AuthCancel`) |
| `a_delivered_frame_notes_what_the_listener_said_and_the_login_keeps_running` | `Delivered(ListenerReply { .. })` → the note line is `summary()`, the cell still reads `logging in…`, the redirect line is back, and `p` opens again |
| `a_refused_auth_deliver_keeps_the_login_pane_and_its_link` | `Failed { "auth_deliver", m }` → the link, the redirect line and `logging in…` stay; `p` opens again |
| `a_cancelling_frame_closes_an_open_field` | `Cancelling` with the field open → `!captures_input()` |
| `a_flow_that_ends_while_the_field_is_open_closes_it_and_releases_input` | For each of `Done`, `Refused`, `Cancelled`, `Idle`, `Failed`: open, type, frame → `!captures_input()`, and no prompt in the render |
| `the_running_hint_offers_p_and_the_pasting_hint_offers_enter_and_esc` | Running → `HINT_AUTH_RUNNING`'s text; field open → `Enter sends · Esc cancels` |
| `agents_paste_redirect` (snapshot) | `running_over(LOOPBACK_LINK)`, then `p`, then `typed("http://127.0.0.1:39879/?code=")` (29 dots and `(29)`). `insta::assert_snapshot!("agents_paste_redirect", render_section(..))` after asserting no `CODE` |

Existing assertions that move: `settings.rs:2023` and `:2581`, and `tests/auth.rs:491`,
`"o open link \u{b7} x cancel"` → `"o open link \u{b7} p paste redirect \u{b7} x cancel"`.

### 4.6 Commits (T3)

1. **(a) red** — `test(tui): the paste field cases, red (MOD-22 T3)`. It holds:
   - §4.5 with the three hint edits;
   - the text-field test;
   - `masked_with_capacity` with a `todo!()` body (nothing calls it yet);
   - the three `Running` fields at both construction sites;
   - `captures_input`'s new clause.
2. **(b) green, keys and frames** — `feat(tui): p pastes a redirect into the running login (MOD-22
   D270, D272)`: §4.1's body, §4.3.
3. **(c) green, render** — `feat(tui): the redirect, the paste field and the hints in the login pane
   (MOD-22 D270)`: §4.4, the module-doc paragraph and the accepted snapshot.

### 4.7 Gate (T3)

```bash
cargo fmt --all -- --check
cargo test -p htui --all-features --lib text_field -- --test-threads=1
cargo insta test -p htui --all-features --test settings -- --test-threads=1   # writes the one .snap.new
cargo insta pending-list          # exactly one: settings__agents_paste_redirect (F-13); read it
cargo insta accept
cargo test -p htui --all-features --test settings -- --test-threads=1
cargo test -p htui --all-features --test auth -- --test-threads=1            # :491 moved
ls crates/htui/tests/snapshots | wc -l                                         # 116
cargo clippy -p htui --all-features --all-targets -- -D warnings
```

`cargo insta review` is interactive; use `pending-list`, read the `.snap.new`, then `accept`.

---

## 5. T4: end to end (D284)

**Files (complete)**: `crates/htui/tests/auth.rs`.

### 5.1 Fixture

`AGENT_SH` (`:93-114`) gains the §3.3 line at the same place (per-file helpers, `:9-11`). New
constants beside `LINK` (`:65`): `CODE`, `STATE`. New helper `fn loopback_link(port: u16) ->
String` (§3.3's shape). `Rig::new` takes `&[(&str, &str)]`, so the case builds owned strings first.

### 5.2 The case

`p_paste_enter_delivers_to_the_listener_and_the_cell_reads_the_probes_verdict`:
1. Bind a `tokio::net::TcpListener` on `127.0.0.1:0` (port P). Spawn the listener task: accept one
   connection, read the head, send it on a `oneshot`, write `HTTP/1.1 200 OK\r\nContent-Type:
   text/html\r\nContent-Length: …\r\nConnection: close\r\n\r\n<html><head><title>signed
   in</title></head></html>`, shut down.
2. `Rig::new(&[("FIXTURE_KEY", "set"), ("FIXTURE_CRED", CREDENTIAL), ("FIXTURE_URL",
   &loopback_link(P)), ("FIXTURE_WAIT_FOR", <tmp>/delivered)])`. The marker path lives in a second
   `tempdir` created first, because the rig creates its own.
3. `start_login`, then `Enter`, then `until("advertised its redirect", |f| f.contains(&format!("redirect:
   127.0.0.1:{P}")))`.
4. `key("p")`, then `until("opened the field", |f| f.contains("paste the address"))`.
5. Type `http://127.0.0.1:P/?code=CODE-SENTINEL-4f1c&state=STATE-SENTINEL-9a2e` one char at a time
   with `rig.harness.app().on_key(KeyEvent::from(KeyCode::Char(c)))`. `Harness::key` parses chords
   and would reject `?`. The text holds `h`, `?` and digits, each of which the shell otherwise binds
   (section cycle, help, tab switch), so the case also proves D270's capture through the whole shell.
6. `key("Enter")`, then
   `until("heard the listener", |f| f.contains(&format!("127.0.0.1:{P} answered 200 OK")))`. Assert
   that it `contains("\"signed in\"")` and that the head from the `oneshot` starts `GET
   /?code=CODE-SENTINEL-4f1c&state=STATE-SENTINEL-9a2e HTTP/1.1`.
7. **Only now** create the marker (F-7, D284), then `until("finished", |f| f.contains("logged
   in:"))`. Assert `contains("logged in: ready")`, `stored_status() == Some("ready")` and `on_box_cell()
   != "unauthenticat"`.
8. Every `until` closure starts with `assert!(!frame.contains(CODE), "{frame}")`, so **every**
   rendered frame at every step is swept, including the status line.

### 5.3 Commit and gate (T4)

One commit: `test(tui): p, paste, Enter logs in from another machine (MOD-22 T4)`. If it is red,
the defect is T2's or T3's. Fix it in that file in a separate `fix(tui): …` commit and name it in
the report; do not widen this task's file set silently.

```bash
cargo fmt --all -- --check
cargo test -p htui --all-features --test auth -- --test-threads=1
```

Then the §6 workspace gate.

---

## 6. Cross-task contracts, pins and the workspace gate

| Defined in | Item (exact) | Consumed by |
|---|---|---|
| T1 `htui_agent::auth::loopback` | `RedirectUrl::new(String)`; `Advertised::{from_auth_url, host, port, path, target}`; `validate(&RedirectUrl, &Advertised) -> Result<Delivery, PasteError>`; `deliver(Delivery, DeliverLimits) -> Result<ListenerReply, DeliverError>`; `Delivery::target`; `DeliverLimits { pub connect, pub response }` + `Default`; `ListenerReply { pub target, pub status, pub reason, pub said, pub location_host }` + `summary()`; `PasteError` (13 variants), `DeliverError` (5); `PASTE_MAX`, `RESPONSE_CAP`, `EXCERPT_WIDTH`, `NO_LOOPBACK_REDIRECT`, `DELIVERY_IN_FLIGHT` | T2, T3, T4 |
| T1 `htui::store_worker` | `StoreRequest::AuthDeliver { url: RedirectUrl }` (`"auth_deliver"`); `AuthFrame::Delivered(ListenerReply)` | T2, T3, T4 |
| T2 `htui::agent_worker` | `AuthCommand::Deliver { url, reply }`; `AgentRuntime::with_deliver_limits(DeliverLimits) -> Self`; `AuthArgs.deliver_limits`; `LOGIN_ENDED` | T4 (through the shell only) |
| T3 `htui::ui::TextField` | `masked_with_capacity(usize) -> Self` | T3 |

**Byte-exact strings**:
- §2.3's two constants, 13 `PasteError` sentences, 5 `DeliverError` sentences and the
  `summary()` shape;
- §3.1's `LOGIN_ENDED`;
- §4.2's three constants;
- §4.4's three pane lines.

| Pin | Now | After | Where |
|---|---|---|---|
| `StoreRequest` variants | 90 (`HANDOFF.md:55` says 88, stale) | 91 | T1; `HANDOFF.md:55` is the main thread's |
| `StoreReply` variants | 49 (`HANDOFF.md:55` says 48, stale) | 49 | — |
| `AuthFrame` variants | 10 | 11 | T1 |
| `AuthCommand` variants | 2 | 3 | T2 |
| `crates/htui/tests/snapshots` | 115 | 116 | T3; `HANDOFF.md:56-57` main thread |
| Store `CASES`, migrations, `.sqlx`, `htui-orch` `CASES` | 97, `0001`..`0009`, 289, 73 | unchanged | — |

```bash
df -h /
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1     # docs/hr-sandbox.md:184; Postgres suites run, not SKIP
cargo doc --workspace --no-deps --keep-going                  # exactly the baseline errors (HANDOFF.md:65-70)
cargo tree -p htui-agent -i url -e normal --offline | head -1 # url v2.5.8
ls crates/htui/tests/snapshots | wc -l                        # 116
git diff --stat f5de3d4 -- crates/htui-store crates/htui-core crates/htui-orch \
  crates/htui-agent/src/lib.rs crates/htui-agent/src/auth/run.rs crates/htui-agent/src/auth/url.rs \
  crates/htui-agent/src/auth/browser.rs crates/htui-agent/src/acp          # empty
git grep -n 'CODE-SENTINEL' -- crates/htui/tests/snapshots                   # nothing
```

The live check (OQ-6) is the maintainer's, after T4, exactly as the plan's "Validation → live check".
The main thread then updates `HANDOFF.md`'s pins and amends `R-AGT-9` (OQ-1).

---

## 7. Decisions (D275 onward) and risks (R-11 onward)

| # | Decision |
|---|---|
| D275 | R-3 resolved as storage, not pricing: `TextField::masked_with_capacity`, and the pane's field opens at `PASTE_MAX`. `text_field.rs` joins T3 (F-1). |
| D276 | `DeliverError::ClosedWithoutAnswer` is a fifth variant. The read loop ends on EOF, reset, the cap, complete framing or the deadline; after a complete status line every one of those is an answer (R-10, F-2). |
| D277 | D269(d)'s "ended" is `cancel.is_cancelled()` or an outcome of `Cancelled`/`Declined`/`Idle`. Otherwise the delivery is awaited in a `select!` against the token. Order after the loop: close, settle the delivery, drain, events, re-probe (F-3). |
| D278 | `AgentRuntime.deliver_limits` + `with_deliver_limits` → `AuthArgs.deliver_limits`; `login_runtime` unchanged (F-4). |
| D279 | Nothing in `loopback` is re-exported from `auth` or the crate root; `lib.rs` is untouched; `auth/mod.rs` spells the crate `::url::Url` if it ever names it (F-16). |
| D280 | `RedirectUrl`'s reader is a private fn of `loopback.rs`, stricter than the plan's `pub(crate)`: `validate` is its only reader. |
| D281 | `Advertised.state` is a plain `Option<String>` with a redacted `Debug`. The link that carries it is already on screen as `link: …`; `Zeroizing` is kept for pasted text. |
| D282 | `p` is bound whenever a login is in flight, with three named refusals. `Cancelling` closes an open field. The prompt/field and `delivering` lines replace the redirect line, for a worst case of 9 lines. `auth_pane` takes the width (F-8, F-12). |
| D283 | `name_arms_are_stable` gains `AuthDeliver`; the `all_four` table keeps its count; a separate no-runtime test covers the fifth (F-11). |
| D284 | T4 withholds the adapter's completion until the note line has been rendered, so the listener's answer is asserted, not raced (F-7). |
| D285 | The credential rule's first bullet names the one pasted value a sentence may quote: an OAuth `error`, filtered and cut (F-10). |
| D286 | T2 adds `a_silent_listener_times_the_delivery_out_and_the_login_keeps_running` (the only user of millisecond limits) and `a_delivery_the_listener_drops_is_answered_before_the_logins_result` (R-10's ordering); the cancel cases run on default limits (F-5, F-6). |
| D287 | `LOGIN_ENDED` replaces the two existing `"this login has ended"` literals in `agent_worker.rs` and serves the two new sites. |
| D288 | Serial execution on `hr/MOD-22` in the primary checkout, no worktrees (CONFIRM). |
| D289 | The excerpt blanks secrets on the whole decoded body before extraction and again after sanitising. Entities are not decoded. The reason phrase is sanitised and cut at 40. `Location` is reported as a host only, and a relative one as nothing. |
| D290 | `validate` builds the connect targets from the **advertised** host (a `localhost` redirect → `127.0.0.1`, then `[::1]` on refusal), never from the pasted text, and `Host:` is `Advertised::target()`. |

| # | Risk | Likelihood | Mitigation |
|---|---|---|---|
| R-11 | A `Delivered` dropped by `App::is_fresh` for a reason other than a second deliver (for example `App::forget`, MOD-64 D240) leaves `delivering` set, so `p` stays refused. | Low | `x`, and any flow end, reset the pane to `Idle`; the refusal names itself (`DELIVERY_IN_FLIGHT`). |
| R-12 | A listener that answers with no `Content-Length`, no chunking and no close holds `delivering to …` for the full 15 s. | Low | The answer is still returned at the deadline as complete (D276), and `x` is served throughout (D269). |
| R-13 | `validate` runs twice (pane, worker), so `url::Url` leaves two unwiped parse buffers per paste. | Accepted | Priced in the module doc's last paragraph. The worker's run is the authority and cannot be dropped (MOD-23 D247's rule). |

`HANDOFF.md`, `DECISIONS.md`, `docs/**` (the `R-AGT-9` amendment, the pins, the live-check record)
are the main thread's. Decision IDs D275–D290 are unused at `8981d39`
(`git grep -E '\bD(27[5-9]|28[0-9])\b'` over `.claude`, `docs`, `DECISIONS.md` and `HANDOFF.md` is
empty); renumber at merge if another sandbox claims them (R-8).
