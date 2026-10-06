# htui's secret provider: Infisical

htui can read a project's secrets from [Infisical](https://infisical.com), so that an agent
session can get them as environment variables. It logs in with an Infisical **machine identity**
whose client ID and client secret sit in the OS keyring, reads one project scope (a project, an
environment and a folder), merges the folder's imports, and checks that every secret can be an
environment variable. This is MOD-10 (`docs/REQUIREMENTS.md` R-SEC-1, R-SEC-2, R-SEC-4); the code
is the `htui-secrets` crate.

**Not yet available.** Milestone 2 ships the client only. Passing the secrets into agent sessions
(milestone 3) and the Settings section that stores the identity and the project scope
(milestone 4) come later. Until then, nothing in htui calls Infisical except the live test
described [at the end](#testing-against-a-real-server).

- [Create a machine identity](#create-a-machine-identity)
- [Keyring entries](#keyring-entries)
- [Base URL](#base-url)
- [Scope](#scope)
- [Imports and precedence](#imports-and-precedence)
- [What htui refuses](#what-htui-refuses)
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
refusal names the field, not its value. Only the folder itself is read, not its subfolders. Secret references
(`${OTHER_KEY}`) are expanded by Infisical before htui sees the values.

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
control characters removed and cut to 200 characters. A failed login's body is never quoted.

The client ID, the client secret, the access token and the resolved values are wiped from memory
when htui drops them. Copies made on
the way (the HTTP request and response buffers, and, from milestone 3, the agent's environment)
are not.

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
| `LoginCoolingDown` | an earlier login with this machine identity got no answer and may have counted as a failed attempt; no new login is tried for another `<n>` s | Wait the `<n>` seconds (30 at most) and try again; the preceding `Unreachable` names why the login got no answer. |
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
