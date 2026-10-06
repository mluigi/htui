# htui's secret provider: Infisical

htui can read a project's secrets from [Infisical](https://infisical.com), so that an agent
session can get them as environment variables. It logs in with an Infisical **machine identity**
whose client ID and client secret sit in the OS keyring, reads one project scope (a project, an
environment and a folder), merges the folder's imports, and checks that every secret can be an
environment variable. This is MOD-10 (`docs/REQUIREMENTS.md` R-SEC-1, R-SEC-2, R-SEC-4); the
client is the `htui-secrets` crate.

A project whose `secret_provider` is set gets its secrets when a run or a chat starts an agent
session ([At run start](#at-run-start)). **Not yet available:** the Settings section that stores
the identity and the project's scope (milestone 4). Until then htui has no command or screen that
writes the keyring entries or the project's two secret columns.

- [Create a machine identity](#create-a-machine-identity)
- [Keyring entries](#keyring-entries)
- [Base URL](#base-url)
- [Scope](#scope)
- [Imports and precedence](#imports-and-precedence)
- [What htui refuses](#what-htui-refuses)
- [At run start](#at-run-start)
- [Logins, tokens and lockout safety](#logins-tokens-and-lockout-safety)
- [What htui never prints](#what-htui-never-prints)
- [Errors and what to do](#errors-and-what-to-do)
- [Testing against a real server](#testing-against-a-real-server)

## Create a machine identity

htui uses Universal Auth, Infisical's client ID and client secret login. The labels below are
those of recent Infisical versions; older ones may word them differently.

1. In the organisation's access control settings, create a **machine identity** and give it the
   **Universal Auth** method.
2. In its Universal Auth settings, create a **client secret**. Copy the client ID and the client
   secret: Infisical shows the secret once.
   - The client secret's number of uses, if you set one, counts logins. htui logs in once per
     access token and again for each health check (see
     [Logins, tokens and lockout safety](#logins-tokens-and-lockout-safety)), so leave it
     unlimited or generous.
   - Leave the access token's number of uses unlimited. A token that runs out of uses early
     costs one extra login.
3. Open the project, add the identity to it, and give it a role that can **read secret values**
   in each environment htui should read (the built-in viewer role can).

Read access to secret **values** is needed for every call, listing the key names included. htui
always asks for the values (it has to check them, see [What htui refuses](#what-htui-refuses)),
and Infisical refuses the whole request with 403 when the identity may not read one of them. A
role that can see names but not values gets
[`PermissionDenied`](#errors-and-what-to-do) from both.

Use one identity per purpose, with read access to only what htui needs.

## Keyring entries

htui keeps the server and the identity in three entries of the OS keyring, under the service
`htui`, next to the Postgres DSN:

| Entry | Holds |
|---|---|
| `htui/infisical-url` | The Infisical [base URL](#base-url) |
| `htui/infisical-client-id` | The machine identity's client ID |
| `htui/infisical-client-secret` | The machine identity's client secret |

The two identity entries are written together and removed together. If only one of them is
there, htui reports an error naming the missing one (`the Infisical machine identity is half
stored: htui/infisical-client-secret is missing; enter the identity again`) rather than treating
it as "no identity". If a write of the client secret fails, htui removes both entries, so an old
secret never pairs with a new client ID. A blank entry reads as absent.

htui does not yet have a command or a screen that writes these entries; the Settings section
(milestone 4) will.

## Base URL

Give the server's root, the address you open Infisical at in a browser: `https://app.infisical.com`,
`https://eu.infisical.com`, or your own `https://infisical.example.com`. Not an API path.

- **https only**, except on a loopback host (`localhost`, any `127.x.x.x`, `::1`), where `http` is
  allowed too. Any other `http` URL is refused, because the client secret would cross the network
  in plain text. `localhost.` (with a trailing dot) and `0.0.0.0` are not loopback.
- Leading and trailing spaces, a trailing `/` and a trailing `/api` are dropped, and the host is
  lowercased: `https://Infisical.Example.com/api/` becomes `https://infisical.example.com`.
- A path prefix is kept, for an Infisical served below a path: `https://example.com/infisical/api`
  becomes `https://example.com/infisical`. Anything else in the path is kept too, so
  `https://example.com/api/v4` is taken as a prefix and every request then fails: give the root.
- A user name or password, a query and a fragment are refused.
- htui never follows a redirect. If your server redirects (for example `http` to `https`, or to a
  login page in front of it), use the address it redirects to.
- Requests to a non-loopback host go through the system proxy (`HTTPS_PROXY` and friends);
  requests to a loopback host never do.
- Connecting times out after 5 seconds, and a whole request after 20.

A refused URL is a `Config` error naming the reason; it never repeats the URL as entered, so a
password typed into it is not echoed.

## Scope

A project reads one scope, kept in the project's `secret_scope` column as JSON, with
`secret_provider` set to `infisical`:

```json
{"project_id":"<Infisical project ID>","environment":"dev","path":"/"}
```

- `project_id`: the Infisical project's ID (in the project's settings), not its name.
- `environment`: the environment **slug** (`dev`, `staging`, `prod`), not its display name.
- `path`: the folder, starting with `/`. Optional; defaults to `/`.

Unknown fields are refused, as are an empty project ID or environment, a path without a
leading `/`, and a control character (a newline, a tab, an escape) in any of the three; the
refusal names the field, not its value. Only the folder itself is read, not its subfolders.
Secret references (`${OTHER_KEY}`) are expanded by Infisical before htui sees the values.

## Imports and precedence

A folder can import secrets from other folders and environments. htui merges them the way
Infisical's own CLI does:

- a secret defined in the folder itself beats an imported one with the same name;
- among imports, the one listed **last** (bottom-most in Infisical's list) beats the earlier ones;
- personal overrides are ignored: htui reads shared secrets only, whoever the identity is.

Infisical v0.150 to v0.158 spell the "include imports" flag differently from later versions and
leave imports out unless asked; htui sends both spellings, so imports are merged on either. A
server older than v0.150 lacks the endpoint htui uses and is reported as `UnsupportedServer`.

## What htui refuses

After the merge, htui checks every secret, in key order, and refuses the whole scope at the first
problem, in this order:

1. **A hidden value.** The identity can see the secret but not its value: `PermissionDenied`
   naming the key. Give the identity read access to the value.
2. **A name that is not an environment variable name** (`^[A-Za-z_][A-Za-z0-9_]*$`, for example
   `API-KEY` or `1TOKEN`): `InvalidKey`. Rename the secret in Infisical.
3. **A value holding a NUL byte**, which an environment variable cannot carry: `InvalidValue`.
   Fix the value in Infisical.

Nothing is dropped or rewritten silently. Values are kept byte for byte, spaces and newlines
included.

## At run start

A project's `secret_provider` column is the switch ([Scope](#scope)). When it is unset, htui
resolves nothing and injects nothing: it reads neither the keyring nor Infisical, and ignores
`secret_scope` whatever it holds. When it is `infisical`, htui resolves the scope before the
project's agents start, hands the values to each agent session as environment variables, and
masks them in everything it stores. Any other value is refused
([When htui refuses](#when-htui-refuses)).

### When htui resolves

- **Once per walk**, at the walk's first live path: the first plain step that goes live, the
  first group of fan-out candidates, or a judge, whichever comes first. A walk is one worker task
  driving a run (a claim, a resume, a retry, a gate answer, an adoption by the sweep). Every
  later step, candidate and judge of the same walk gets the same values without asking Infisical
  again.
- **Before anything of the session is stored**: before htui prepares the step's tree, writes its
  prompt or trim record, or starts its agent.
- **Again on every walk.** Values are never stored, so a resumed, retried or adopted walk resolves
  again. A value changed in Infisical reaches the next walk, never the middle of one.
- **Before an accepted artifact's verify.** Accepting a chat's artifact resolves before the verify
  command runs, so its output can be masked with the values
  ([The verify command](#the-verify-command)).
- **Not at all** for a walk that starts no agent session, such as a gate answer that only
  settles the step, a park or a promotion.
- **Chats** resolve each time a chat starts or a promoted step's chat binds, in the chat's own
  task, so a slow Infisical never stalls the rest of the TUI. The column checks run first and need
  no network.

The keyring is read at every resolution, on a blocking thread, one read at a time per process so
that two walks starting together never stack two OS unlock prompts. A read that has not answered
after 120 seconds (an unlock prompt nobody answered) refuses that walk or chat as `Config`
(`the OS keyring did not answer`); the read itself cannot be cancelled and finishes in the
background, and the next walk reads again. A process keeps one provider: the
TUI shares its one between its runs and its chats, and `htui worker` has its own. The provider is
rebuilt only when the stored base URL (after [normalisation](#base-url)), client ID or client
secret has changed, so an identity entered again takes effect at the next walk or chat, and
[one refused login](#logins-tokens-and-lockout-safety) holds across walks and chats: after a
`BadCredentials`, every later walk of that process meets `LoginRefusedEarlier` until the identity
is entered again.

On a plain step, resolution runs after the step has started, so the keyring read and the time
Infisical takes to answer (up to 5 seconds to connect and 20 per request) are spent from the
step's deadline.

### What the agent gets

- **Exactly the resolved map**: every key and value of the scope after the
  [merge](#imports-and-precedence), byte for byte. Nothing else from htui goes in (R-SEC-2).
- **Applied last**, over the agent row's own environment, so a secret wins over a row variable of
  the same name.
- **Only the agent.** htui never puts a value into its own environment (it cannot: `set_var` is
  unsafe, and htui forbids unsafe code), so no other process htui starts, the verify command
  included, receives the values.

### Reserved names

A resolved key that starts with `HTUI_`, in any letter case (`htui_token` and `Htui_Log`
included, since Windows environment names are case-insensitive), is refused as `ReservedKey`:
``the secret name `HTUI_…` is reserved for htui; rename it in Infisical``. htui's own
variables (`HTUI_MCP_*`, `HTUI_LOG*`, `HTUI_TOOL_*`) must never be shadowed by a secret that is
applied last. The first such key, in key order, is named. `HTUIX`, `HTUI` and `MY_HTUI_X` are
allowed.

### When htui refuses

A refusal is decided before any agent starts. The run's failure reason reads `secrets_refused: `
followed by the cause's sentence, for example:

```
secrets_refused: no Infisical machine identity is stored in the OS keyring
```

- **A plain step** fails, and the run and its item fail with it.
- **A group of fan-out candidates** fails as one: each candidate fails with an item note
  ``fan-out candidate <i> of `<phase>` attempt <n>: secrets_refused: …``, and the run fails. There
  is no group retry and no selection.
- **A judge** (only when it is its walk's first live path, as in a resumed walk) fails the run;
  the selection is not parked.
- **An accept** is refused with the same sentence and writes nothing: the step stays promoted and
  parked, and can be accepted again once the secrets resolve.
- **A chat** is refused with the same sentence and starts nothing. A new chat leaves no run
  behind.

Transient causes (`Unreachable`, `RateLimited`, `LoginCoolingDown`, `IdentityLocked`) fail the run
too. htui never retries a resolution by itself, so it never chases a cool-down or a lockout: fix
the cause, or wait, then run again.

| Cause | Error |
|---|---|
| `secret_provider` names a provider this build does not know | `Config` (1) |
| A provider but no `secret_scope` | `Config` (2) |
| A `secret_scope` that does not parse | `Config`, naming the field ([Scope](#scope)) |
| No base URL in the keyring | `Config` (3) |
| A stored base URL htui refuses | `Config`, naming the reason ([Base URL](#base-url)) |
| A keyring that cannot be read, or a half-stored identity | `Config` (4) |
| A keyring that does not answer within 120 seconds | `Config` (5) |
| No machine identity in the keyring | `NoIdentity` |
| Anything Infisical or the network answers | Its variant ([Errors](#errors-and-what-to-do)) |
| A key htui refuses after the merge | [What htui refuses](#what-htui-refuses) |
| A key starting with `HTUI_`, in any case | `ReservedKey` ([Reserved names](#reserved-names)) |

The `Config` sentences, after `secret provider configuration: `:

1. `project.secret_provider "<value>" is not a provider this build knows (expected
   "infisical")`, the value escaped;
2. `the project names a secret provider but has no secret_scope`;
3. `no Infisical base URL is stored in the OS keyring`;
4. `the OS keyring could not be read: ` and the keyring's own message, which names the entry
   (for a half-stored identity, the one that is missing), never a value;
5. `the OS keyring did not answer`.

Three more guard htui's own wiring and should never be seen: `this process has no secret source,
so the project's secrets cannot be resolved`, ``the secret source answered a `<kind>` provider for
a `<column>` project`` and `this walk's secrets were resolved for another project`.

### Masking

What htui stores (session events, a step's prompt and trim record, a verify command's output,
and an agent's error text, such as the stderr tail of an agent that failed, wherever it is kept:
a run's failure, a step's failure, and a fan-out candidate's or a judge's note) is masked with
the resolved values once the walk or the chat has resolved them: every occurrence of a value
becomes `[REDACTED]`. When the credential rules refuse an agent's error text, htui keeps
`stderr withheld: the walk's scrubber refused it` (or `cause withheld: …`) in its place, and a
refused verify output is kept as `<scrub refused: N bytes withheld>`. htui's credential rules
(known key formats such as `sk-ant-…`, `ghp_…` or `AKIA…`) apply as before: a write that still holds one is refused, and the session fails closed.
Before its first resolution, a walk is masked by the credential rules only.

#### Short values

A value shorter than 6 characters is injected but **not masked**: masking a 3-character value
would shred every transcript. htui logs the **key names** of such values at `warn`, once per walk
and once per chat (`these secrets are shorter than the masking floor: injected, not masked`), never
the values. Lengthen them in Infisical.

#### Trailing newlines and multi-line values

A value stored with a trailing line end (`\n` or `\r\n`) is masked both as stored and without its
trailing line ends, so the bare token an agent echoes is masked too.

A multi-line value (a certificate, a service-account file) is also masked in the forms a tool
prints it in: with its `\n` line ends turned into `\r\n`, JSON-escaped (`\n` written as two
characters, as inside a printed JSON document), each with and without its trailing line ends,
and line by line, so one line printed alone is masked too. PEM armour lines
(`-----BEGIN PRIVATE KEY-----`, `-----END …-----`) are not masked as lines, since every key shares
them; a printed private-key armour line is refused by the credential rules anyway.

Every extra form is masked only when it is itself at least 6 characters long; whether a value is
[short](#short-values) is decided on the value as stored. The value is still injected byte for
byte.

#### Escaped token starts

A credential rule counts only at a token start: the start of the text, or after a character that
is not a letter, a digit or `_`. A token start also follows a JSON or percent escape (`\n`,
`\r`, `\t`, `\b`, `\f`, `\"`, `\/`, `\\`, `\uXXXX`, `%XX`), so a key serialised inside an escaped
string (`…\nsk-ant-…`, `%22ghp_…`) is refused, while `subtask-…` is still prose. This widens what
fails closed. `scripts/scrub-audit.sql` uses the same token start: run it on the host, against the
database htui uses, to count the stored rows the wider rule would refuse (it prints counts only).

#### The row seam

htui stores an agent's streamed text in rows of up to 16 KiB. When a text run reaches that bound,
htui cuts it and carries the rest into the next row. The cut keeps the end of the run open: as
many bytes as the longest resolved value, or 76 for the credential rules, whichever is larger,
less one. It moves back so that no value or credential is split: one complete before the cut is
masked whole in the first row, and one still arriving lands whole in the next, masked there (a
resolved value) or refused there (a credential). When every cut would split an occurrence of a
value (a value longer than the run, or a run that opens with one), the run stays open past the
bound instead: until the run is about twice the hold-back long, plus one streamed chunk, or about
three times the hold-back plus one chunk when the run is refused at the bound because a value or
credential is still arriving. A run with no safe cut for any other reason
is cut at the bound as before, and htui logs `no safe seam was found` at `warn`.

The cut is decided from the run as it is at the bound, not from the bytes still to come, so four
cases stay open:

- **A credential already in the run before the cut point**, at its rule's minimum length, refuses
  the run whatever the cut: the run is replaced by one `scrub_residue` row and the session fails
  closed. A value still arriving in the same run is cut with it, and its rest lands, without its
  start, in the next row.
- **An `sk-` key whose body reads as words** (`sk-learn-preprocessing-…`) for longer than the
  hold-back can be cut with both halves clean, then continue with a key-shaped part: neither row
  is refused, while the two together would be.
- **A JWT** only matches once its third segment arrives. When its header and payload are longer
  than the hold-back (every real one is, unless a long resolved value widens it), a JWT cut inside
  its payload leaves two clean rows that together would match.
- **A cut inside a word, right before `sk-`**, makes `sk-` a token start in the carried row. If
  the body grows into a key shape, the carried row is refused although the whole text would not
  be. That is a false refusal: the session fails closed.

### What is wiped

The resolved map and the masking list are wiped from memory when htui drops them: a walk's when
the walk ends, a chat's map once its environment is built and its masking list when the chat
ends, and a refused map at once. Two copies are not htui's to wipe:

- the environment handed to the agent's driver (`SessionSpec.env`, a plain map, and the copies the
  driver makes from it to start the agent);
- the agent process's own environment block, which the operating system keeps for the life of
  that process.

### The verify command

A phase's verify command runs with htui's own environment, so it never receives the resolved
values. Its output can still hold one, for example a command that prints a file the agent wrote
(`cat .env`, a test log). The verify runner keeps the output's last 64 KiB and masks it with the
credential rules; htui then masks it again with the walk's resolved values before storing it in
`command_run.output`. Output the walk's masking refuses is dropped and stored as
`<scrub refused: N bytes withheld>`, and a `warn` log says so.

The 64 KiB cut comes before the second masking, so a value that straddles the start of the kept
tail would leave its end in the output, where it no longer matches the value. When the output
was cut, htui therefore drops the tail's first bytes before masking it again, as many as the
longest resolved value (or 76 for the credential rules, whichever is larger), less one, and opens
the stored tail with `[… earlier output truncated]`. An `unavailable` verify keeps its reason line
whole above the tail.

## Logins, tokens and lockout safety

Universal Auth locks an identity after repeated failed logins (by default 3 failures lock it for
300 seconds). A wrong client secret retried in a loop would keep it locked, so htui gives each
provider **one** refused login:

- When Infisical refuses a login with 401 (`BadCredentials`, or `IdentityLocked` when its message
  says the identity is temporarily locked), the provider remembers it. Every later call on that
  provider, resolving, listing or a health check, answers `LoginRefusedEarlier` at once and sends
  nothing.
- The refusal is decided by the 401 status alone. A 401 whose body is cut short still counts as a
  refused login (`BadCredentials`): only a body read in full can show the lockout text.
- To try again, enter the identity again. htui builds a new provider for the new identity, and
  the new provider starts with a clean slate.
- A login runs to its end even when the call that started it gives up (a timeout, a cancelled
  task), and concurrent calls wait for that one login. So a provider spends at most one refused
  login, however many calls were waiting or gave up.
- A login that was sent but got no answer (the 20-second timeout, a dropped connection) may have
  been counted by Infisical as a failed attempt. The call that sent it fails with `Unreachable`;
  for the next 30 seconds (Infisical's default window for forgetting failed attempts) every call,
  health checks included, answers `LoginCoolingDown` at once and sends nothing. After that, one
  new login is tried.
- Other failures that are not refusals (the server not reachable at all, so nothing was sent; a
  429; a 5xx; an unexpected answer) are not remembered: the next call logs in again.

htui keeps the access token in memory only and reuses it until shortly before it expires (it
stops a minute early, or a tenth of the token's lifetime early when that is longer). If
Infisical refuses the token on a data request (401, or 403 `TokenError`), htui logs in once more
and retries once; a token refused right after a fresh login is a `Protocol` error. A health check
always logs in afresh, to prove the stored identity works, and the new token replaces the cached
one.

## What htui never prints

No secret value, client secret or access token appears in any error, log line, `Debug` output or
test failure. Errors may contain an endpoint path, an HTTP status, a key name, the scope's
environment and folder, and, for requests other than the login, Infisical's own error message with
control characters removed and cut to 200 characters. A failed login's body is never quoted. A log
line may name a key, never its value ([Short values](#short-values)).

The client ID, the client secret, the access token and the resolved values are wiped from memory
when htui drops them. Copies made on the way (the HTTP request and response buffers, and the
agent's environment, see [What is wiped](#what-is-wiped)) are not.

## Errors and what to do

Every call fails with one of these. The sentence is what htui shows; `…` is filled in by htui.

| Variant | Sentence | What to do |
|---|---|---|
| `NoIdentity` | no Infisical machine identity is stored in the OS keyring | Enter the identity's client ID and client secret. |
| `Config` | secret provider configuration: … | Fix what it names: the [base URL](#base-url), a blank client ID or client secret, a [scope](#scope) that does not parse, or an HTTP client that would not build. |
| `Unreachable` | cannot reach Infisical at `<endpoint>`: … | Check the base URL, DNS, the network, the proxy and the server's TLS certificate; the cause after the colon says which. A timeout means no connection within 5 seconds, or no full answer within 20. |
| `BadCredentials` | Infisical refused the machine identity's login: the client ID or client secret is wrong, expired or used up | Create a new client secret (or check the client ID) and enter the identity again. |
| `IdentityLocked` | Infisical has temporarily locked the machine identity after repeated failed logins; wait for the lockout to end before trying again | Wait for the lockout to end (5 minutes by default), check the client secret, then enter the identity again. |
| `LoginRefusedEarlier` | an earlier login with this machine identity was refused; no new login is tried until the identity is entered again | Read the earlier `BadCredentials` or `IdentityLocked`, fix the identity and enter it again. |
| `LoginCoolingDown` | an earlier login with this machine identity got no answer and may have counted as a failed attempt; no new login is tried for another `<n>` s | Wait the `<n>` seconds (30 with the default `login_cool_down`) and try again; the preceding error names why the login got no answer (`Unreachable`, or `Protocol` when the runtime shut down mid-login). |
| `ProjectNotFound` | Infisical has no project with the configured project ID | Copy the project ID (not its name) from the project's settings into the scope. |
| `PathNotFound` | Infisical has no environment `<environment>` or folder `<path>` in the project | Use the environment's slug and an existing folder. |
| `PermissionDenied` | the machine identity may not read these secrets: … | Add the identity to the project, or give its role read access to the secret values in that environment and folder (the detail says which, or names a hidden key). |
| `RateLimited` | Infisical rate-limited the request, followed by `; retry after <n> s` when the server sent `Retry-After` as whole seconds | Wait (the `<n>` seconds, when given) and try again; htui does not retry by itself. Infisical Cloud only. |
| `UnsupportedServer` | this Infisical predates v0.150 (`<endpoint>` does not exist); upgrade it | Upgrade the server, or enable Universal Auth on it when the endpoint is the login's. |
| `InvalidKey` | the secret name `"<key>"` is not a valid environment variable name; rename it in Infisical | Rename the secret to letters, digits and `_`, not starting with a digit. |
| `InvalidValue` | the secret `<key>` holds a NUL byte, which an environment variable cannot carry; fix its value in Infisical | Remove the NUL from the value in Infisical. |
| `Protocol` | unexpected answer from Infisical at `<endpoint>`: … | Check the base URL points at Infisical and not at a proxy or login page (a redirect lands here); otherwise the detail names the status or what was wrong with the answer. An answer larger than htui reads (64 KiB for a login or an error, 8 MiB for a secrets list) lands here too, naming the limit. |

## Testing against a real server

The live test, `crates/htui-secrets/tests/infisical_live.rs`, runs a health check, a resolve and a
key listing against a real Infisical. It reads five variables:

| Variable | Value |
|---|---|
| `HTUI_TEST_INFISICAL_URL` | The base URL |
| `HTUI_TEST_INFISICAL_CLIENT_ID` | A test identity's client ID |
| `HTUI_TEST_INFISICAL_CLIENT_SECRET` | Its client secret |
| `HTUI_TEST_INFISICAL_PROJECT_ID` | A test project's ID |
| `HTUI_TEST_INFISICAL_ENVIRONMENT` | An environment slug whose root folder holds at least one secret |

```
cargo test -p htui-secrets --test infisical_live -- --nocapture
```

Without `HTUI_TEST_INFISICAL_URL` it prints `skipped: HTUI_TEST_INFISICAL_URL not set` and passes.
With it, the other four are required, and a missing one fails the test by name. On success it
prints `resolved <n> keys`; on failure it names the step and the error variant (look it up in
[Errors and what to do](#errors-and-what-to-do)). It never prints a key name, a value, the client
secret or the token. Use a throwaway identity with read access to one test environment: the
client secret sits in your shell's environment while the test runs.
