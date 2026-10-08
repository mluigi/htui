# htui from scripts: `status`, `waiting` and `--search-items --json`

Three commands let a script, a status bar or another tool read htui's state without starting the
TUI. Each one reads, prints one answer and exits:

- `htui status` prints the top bar's facts: the store's state, the workspace, this box, and how
  many runs are working and how many rows wait on you.
- `htui waiting` prints the waiting-on-you list (`R-TUI-11`): the same rows as the TUI's overlay,
  with how long each one has waited.
- `htui --search-items <QUERY>` prints the concepts index's best hits.

With `--json`, each prints exactly one JSON document on one line, and its exit code says how it
ended. This is MOD-92. The documents are pinned by snapshot tests
(`crates/htui/src/snapshots/htui__state_cmd__tests__*.snap` and
`crates/htui/tests/snapshots/state_cli_pg__*.snap`); this page describes those shapes.

None of the three writes anything: not to Postgres, not to Qdrant, not to htui's files on this
machine. See [No writes](#no-writes).

- [The commands and their flags](#the-commands-and-their-flags)
- [Scope: what a command reads](#scope-what-a-command-reads)
- [The envelope](#the-envelope)
- [`status`](#status)
- [`waiting`](#waiting)
- [`search`](#search)
- [Failure documents](#failure-documents)
- [Exit codes](#exit-codes)
- [When there is no document](#when-there-is-no-document)
- [Versioning](#versioning)
- [No writes](#no-writes)
- [Credentials and crash reports](#credentials-and-crash-reports)
- [Text output](#text-output)

## The commands and their flags

| Command | Flags |
|---|---|
| `htui status` | `--json`, `--project <SLUG>` or `--cwd <PATH>`, `--log <PATH>` |
| `htui waiting` | `--json`, `--project <SLUG>` or `--cwd <PATH>`, `--log <PATH>` |
| `htui --search-items <QUERY>` | `--json`, `--project <SLUG>` or `--cwd <PATH>`, `--decisions`, `--limit <N>`, `--log <PATH>` |

- **`--json`** prints one versioned JSON document instead of text. Without it, the same answer is
  printed as text (see [Text output](#text-output)) and the exit codes are the same.
- **`--project <SLUG>`** scopes the command to one project.
- **`--cwd <PATH>`** scopes the command to the project whose repository, registered on this box,
  contains `PATH`. `--cwd .` means the current directory.
- `--project` and `--cwd` cannot be given together.
- **`--decisions`** (search only) keeps decisions only: items closed as `done`, `concluded` or
  `rejected`, and their documents.
- **`--limit <N>`** (search only) is how many hits to return, 1 to 1000. The default is 10.
- **`--log <PATH>`** appends logs to a file. It goes after `status` or `waiting`
  (`htui status --log x.log`). `HTUI_LOG` works too.

`status` and `waiting` are subcommands, so no TUI flag may stand beside them: `htui --demo status`
and `htui --offline waiting` are usage errors. `--json`, `--cwd`, `--decisions` and `--limit` at
the top level need `--search-items`; `htui --json` alone is a usage error.

## Scope: what a command reads

`status` and `waiting` read one scope:

- **By default, the first workspace.** That is the first workspace by name, the one the TUI opens
  on. Its projects are read in the workspace's own order.
- **`--project <SLUG>`** reads that project alone. Project slugs are unique across the store. A
  project can belong to several workspaces; the document then reports the first of them by name.
  A slug no workspace lists is refused with `unknown_project`.
- **`--cwd <PATH>`** reads the project whose checkout on this box contains `PATH`:
  - `PATH` and every repository path registered for this box are put through the file system's
    canonical form on this machine. A relative `PATH` resolves against the current directory,
    symlinks are followed, and on Windows the drive prefix and the casing compare alike.
  - Containment is per path component: `/src/htui2` is not inside `/src/htui`.
  - The deepest checkout wins, so a checkout nested inside another one is found.
  - A registered path that no longer exists on disk is skipped.
  - `PATH` that cannot be resolved is refused with `cwd_unresolved`; `PATH` in no registered
    checkout with `cwd_unmatched`.
  - When the deepest checkout is registered for two or more projects, htui does not pick one: it
    refuses with `cwd_ambiguous`. Name the project with `--project` instead.
  - `--cwd` needs this box's `box.toml`, because the box id decides which checkouts are "this
    box's". Without one it is refused with `no_box`; start the TUI once so the box gets an
    identity.

`--search-items` keeps its own default: **every workspace**. `--project` and `--cwd` narrow it to
one project, with the same rules as above.

## The envelope

Every document starts with the same three fields, in this order:

| Field | Type | Meaning |
|---|---|---|
| `version` | integer | The schema version, `1` today. See [Versioning](#versioning). |
| `kind` | string | `"status"`, `"waiting"` or `"search"`: which command answered. |
| `store` | string | Postgres's state: `"online"` (it answered), `"offline"` (it did not answer), `"refused"` (the command refused before an answer) or `"error"` (a read failed after the connect). |

When `store` is `"online"`, the command's own fields follow. Otherwise one `error` object follows,
and nothing else (see [Failure documents](#failure-documents)).

General rules:

- Ids are UUID strings.
- Timestamps are RFC 3339 in UTC, whole seconds, always ending in `Z`: `"2026-10-08T20:01:02Z"`.
- A field that has no value is `null`; it is never left out, except `hits` and `error` on a
  search document (see [`search`](#search)).
- Fields always appear in the order shown here, but do not depend on that: JSON objects are
  unordered.

## `status`

`htui status --json`:

```json
{
  "version": 1,
  "kind": "status",
  "store": "online",
  "workspace": {
    "id": "00000000-0000-0000-0000-000000000001",
    "slug": "platform",
    "name": "Platform"
  },
  "projects": [
    { "id": "00000000-0000-0000-0000-000000000010", "slug": "htui", "name": "htui" },
    { "id": "00000000-0000-0000-0000-000000000011", "slug": "agy", "name": "Antigravity" }
  ],
  "box": {
    "id": "00000000-0000-0000-0000-0000000000b0",
    "hostname": "desk",
    "registered": true
  },
  "working": 1,
  "waiting": 2,
  "permissions_known": true
}
```

| Field | Type | Meaning |
|---|---|---|
| `workspace` | object or `null` | The workspace the scope belongs to. `null` when the store holds no workspace. |
| `workspace.id` | uuid | The workspace's id. |
| `workspace.slug` | string | The workspace's slug. |
| `workspace.name` | string | The workspace's name, as the top bar shows it. |
| `projects` | array | The projects read, in the workspace's order. Empty when there is no workspace. |
| `projects[].id` | uuid | The project's id. |
| `projects[].slug` | string | The project's slug, the one `--project` takes. |
| `projects[].name` | string | The project's name. |
| `box` | object or `null` | This box. `null` when this machine has no `box.toml` (htui never started here). |
| `box.id` | uuid | The id in `box.toml`. |
| `box.hostname` | string | The `box` row's hostname when the box is registered; otherwise this machine's current hostname. |
| `box.registered` | boolean | Whether the store holds a `box` row with this id. `false` when the TUI or `htui worker` has never connected from this machine to this database. |
| `working` | integer | Active runs in the scope that own no waiting row. Chat runs count too. The top bar's first count. |
| `waiting` | integer | How many rows wait on a person: the length of `htui waiting`'s `rows`. The top bar's second count. |
| `permissions_known` | boolean | Whether the open permission requests could be listed. When `false`, permission rows are missing from the count, because that read failed with something other than an unreachable store. |

A store with no workspace answers `"workspace": null`, `"projects": []` and zero counts.

## `waiting`

`htui waiting --json`:

```json
{
  "version": 1,
  "kind": "waiting",
  "store": "online",
  "workspace": { "id": "00000000-0000-0000-0000-000000000001", "slug": "platform", "name": "Platform" },
  "projects": [
    { "id": "00000000-0000-0000-0000-000000000010", "slug": "htui", "name": "htui" },
    { "id": "00000000-0000-0000-0000-000000000011", "slug": "agy", "name": "Antigravity" }
  ],
  "working": 1,
  "permissions_known": true,
  "rows": [
    {
      "reason": "gate",
      "item_key": "MOD-4",
      "item_id": "00000000-0000-0000-0000-000000000101",
      "run_id": "00000000-0000-0000-0000-000000000201",
      "step_id": "00000000-0000-0000-0000-000000000301",
      "step": "prd 0.1",
      "text": "approve the PRD",
      "since": "2026-09-03T06:00:00Z",
      "age_seconds": 64800
    },
    {
      "reason": "unblock",
      "item_key": "MOD-7",
      "item_id": "00000000-0000-0000-0000-000000000104",
      "run_id": null,
      "step_id": null,
      "step": null,
      "text": "blocked, no active run: u reopens it",
      "since": "2026-09-02T00:00:00Z",
      "age_seconds": 172800
    }
  ]
}
```

`workspace`, `projects`, `working` and `permissions_known` mean what they mean in
[`status`](#status). There is no `box` and no `waiting` count: the count is the length of `rows`.

`rows` holds one object per row of the TUI's waiting-on-you overlay. The rows are computed by the
TUI's own code, over the same reads, so they match what the overlay shows for the same scope. They
come in the overlay's order: by project (in scope order), then item key, then run, then step, then
reason. An empty list is `[]`.

| Field | Type | Meaning |
|---|---|---|
| `reason` | string | Why the row waits. See the table below. |
| `item_key` | string | The item's key, such as `MOD-4`. |
| `item_id` | uuid | The item's id. |
| `run_id` | uuid or `null` | The run the row is about. `null` for an Unblock row that reopens the item (no run). |
| `step_id` | uuid or `null` | The step the row is about. `null` when the row is about a whole run, or about no run. |
| `step` | string or `null` | The step as the Runs pane labels it after its phase: `prd 0.1`, `research 0.1/1` (a fan-out slot), `research 0.1/j` (its judge). `null` when there is no step. |
| `text` | string | The reason's text: the gate note or judge note, the tool a permission asks for, or the Unblock sentence. |
| `since` | timestamp | When the row began waiting. See below. |
| `age_seconds` | integer | Whole seconds from `since` to now, never below 0. |

The `reason` values:

| `reason` | The row waits on you to... | `since` is |
|---|---|---|
| `gate` | approve or reject a step parked at `awaiting_approval` | the step's `updated_at` |
| `judge_failed` | pick a fan-out winner after the judge failed | the judge step's `updated_at` |
| `selection` | pick a fan-out winner (no judge verdict) | the run's `updated_at` |
| `unblock` | press `u` on a blocked item: reopen it, follow its parked run, or resume the run | the run's `updated_at`; for a reopen row (no run), the item's `updated_at` |
| `interrupted` | retry, promote or cancel a run parked by an interrupt | the run's `updated_at` |
| `permission` | answer an agent's permission request | the request's `created_at` |

**What `since` really is.** It is the last time the store stamped that row, on the database's
clock. The park itself stamps it, so it is the moment the row began waiting, until something else
edits the same row: a later edit (for example a new note on the parked step) moves `since` forward.
An exact "parked at" time is not stored yet.

**What `age_seconds` really is.** It is measured with the clock of the machine that ran
`htui waiting`, against `since` on the database's clock. When the two clocks disagree, the age is
off by the difference; an age that would be negative is reported as `0`.

## `search`

`htui --search-items "closed items" --json`:

```json
{
  "version": 1,
  "kind": "search",
  "store": "online",
  "index": "online",
  "query": "closed items",
  "decisions": false,
  "hits": [
    {
      "key": "ANA-11",
      "place": "item",
      "document_kind": null,
      "resolution": null,
      "requirement_state": null,
      "score": 0.731,
      "snippet": "a decision is a closed item"
    },
    {
      "key": "MOD-34",
      "place": "document",
      "document_kind": "prd",
      "resolution": "done",
      "requirement_state": null,
      "score": 0.5,
      "snippet": "the concepts index"
    }
  ]
}
```

`store` describes Postgres only, which a search reads for its scope. Once Postgres has answered,
`store` is `"online"`, and `index` describes the Qdrant concepts index:

| `index` | When | `hits` | `error.code` | Exit |
|---|---|---|---|---|
| `online` | The index answered. | array (may be empty) | no `error` | 0 |
| `empty` | There is no index yet. It is not created; run `htui --index-items`. | `[]` | no `error` | 0 |
| `offline` | Qdrant could not be reached, or the search failed. | no `hits` | `qdrant_unreachable` | 3 |
| `refused` | The index holds another embedding model's vectors. | no `hits` | `embedder_mismatch` | 2 |
| `error` | The embedding model could not be downloaded or loaded. | no `hits` | `model_unavailable` | 1 |

| Field | Type | Meaning |
|---|---|---|
| `index` | string | The index's state, above. |
| `query` | string | The query, as given. |
| `decisions` | boolean | Whether `--decisions` was given. |
| `hits` | array | Present for `online` and `empty` only. The best hits first. |
| `hits[].key` | string | The item's or requirement's key. |
| `hits[].place` | string | What matched: `"item"`, `"document"` (one of the item's documents) or `"requirement"`. |
| `hits[].document_kind` | string or `null` | The document's kind, such as `prd`, on a document hit; otherwise `null`. |
| `hits[].resolution` | string or `null` | How the item closed (`done`, `concluded`, `rejected`, `withdrawn`, `superseded`, `duplicate`), on an item or document hit of a closed item; otherwise `null`. |
| `hits[].requirement_state` | string or `null` | `active` or `withdrawn`, on a requirement hit; otherwise `null`. |
| `hits[].score` | number | The fused rank score. Compare scores within one search only. |
| `hits[].snippet` | string | The start of the indexed text, whitespace folded and control characters removed. |
| `error` | object | Present for `offline`, `refused` and `error` only: `code` and `message`, as in [Failure documents](#failure-documents). |

A failure before Postgres answered (no Qdrant URL, no DSN, Postgres unreachable, an unknown
project) is a [failure document](#failure-documents) with `"kind": "search"`, and has no `index`,
`query` or `decisions`.

## Failure documents

When there is no answer, the document is the envelope and one `error` object:

```json
{"version":1,"kind":"status","store":"offline","error":{"code":"unreachable","message":"cannot reach Postgres: store unreachable: connection refused"}}
```

```json
{"version":1,"kind":"waiting","store":"refused","error":{"code":"no_dsn","message":"no Postgres DSN is stored; run `htui --set-dsn` first"}}
```

| Field | Type | Meaning |
|---|---|---|
| `error.code` | string | Why there is no answer, for a program. The values are below. |
| `error.message` | string | The same sentence htui prints on standard error, for a person. Do not parse it; it may change. |

Every `error.code`, with the `store` it comes with and the exit code:

| `error.code` | `store` | Exit | Commands | When |
|---|---|---|---|---|
| `unreachable` | `offline` | 3 | all | Postgres did not answer: at the connect, or during a later read. |
| `no_dsn` | `refused` | 2 | all | No Postgres DSN is stored in the OS keyring. Run `htui --set-dsn`. |
| `keyring_unreadable` | `refused` | 2 | all | The OS keyring could not be read. |
| `box_identity` | `refused` | 2 | all (search only with `--cwd`) | `box.toml` exists but cannot be read or parsed. `status` and `waiting` always read it (for `box`), so a malformed `box.toml` refuses them even without `--cwd`, just as the TUI will not start with one. Fix or delete the file. |
| `migrations_pending` | `refused` | 2 | all | The database has migrations this build has not applied. Start the TUI once to apply them. |
| `below_target` | `refused` | 2 | all | The database's target version is newer than this build. Update htui. |
| `store_refused` | `refused` | 2 | all | Postgres answered and refused this htui: a newer or drifted schema, a partial migration, a wrong password, a missing database, a pooler that rejects the read-only session (see [No writes](#no-writes)). |
| `unknown_project` | `refused` | 2 | all | `--project` names no project. |
| `cwd_unresolved` | `refused` | 2 | all | `--cwd`'s path cannot be resolved on this machine. |
| `no_box` | `refused` | 2 | all | `--cwd` was given, but this machine has no `box.toml`. |
| `cwd_unmatched` | `refused` | 2 | all | `--cwd`'s path is in no repository registered on this box. |
| `cwd_ambiguous` | `refused` | 2 | all | `--cwd`'s deepest checkout is registered for two or more projects. |
| `read_failed` | `error` | 1 | all | A read failed after the connect, with something other than an unreachable store. |
| `no_qdrant_url` | `refused` | 2 | search | No Qdrant URL is stored. Set one in **Settings › Qdrant**. |
| `qdrant_url_unusable` | `refused` | 2 | search | The stored Qdrant URL cannot be used. |
| `qdrant_unreachable` | `online` | 3 | search | Inside a search document: Qdrant did not answer (`index: "offline"`). |
| `embedder_mismatch` | `online` | 2 | search | Inside a search document: the index holds another model's vectors (`index: "refused"`). |
| `model_unavailable` | `online` | 1 | search | Inside a search document: the embedding model could not load (`index: "error"`). |

The checks run in this order, and the first one that fails decides the document:

- `status` and `waiting`: the DSN, `box.toml`, the connect, the scope, then the reads.
- `--search-items`: the Qdrant URL, the DSN, `box.toml` (only with `--cwd`), the connect, the
  scope, the embedding model, then the index. The Qdrant URL is checked first, so a box without
  one is refused before Postgres is contacted.

## Exit codes

| Exit | Meaning | `store` | Document |
|---|---|---|---|
| 0 | Answered. For a search, the index answered or is empty. | `online` | yes |
| 3 | Postgres did not answer; for a search, Qdrant did not answer. | `offline`, or `online` with `index: "offline"` | yes |
| 2 | Refused: no DSN, the keyring, `box.toml`, the schema, the scope, an embedder mismatch. | `refused`, or `online` with `index: "refused"` | yes |
| 1 | Failed: a read failed after the connect, or the embedding model could not load. | `error`, or `online` with `index: "error"` | yes |

An offline store is reported as offline: htui never answers from its local cache here, so a
document is never stale data presented as live.

`--search-items` without `--json` uses the same exit codes, with the same sentences on standard
error as before. Earlier versions exited 1 for every failure.

## When there is no document

Three endings print nothing on standard output. **Treat an empty standard output as a failure,
whatever the exit code.**

| Ending | Exit | What you get |
|---|---|---|
| A usage error (an unknown flag, `--project` with `--cwd`, `htui --json` alone) | 2 | clap's usage message on standard error |
| The `--log` file cannot be opened | 1 | the error on standard error |
| A crash (panic) | 101 | the panic message on standard error |

A usage error and a refusal both exit 2; only the refusal prints a document.

## Versioning

`version` is `1`. It changes only on a breaking change: a field removed, renamed, or given another
type or meaning.

Adding a field, or a new value to an existing set (a new `reason`, `index`, `store`,
`error.code` or `place`), does not change `version`. So a reader must:

- ignore fields it does not know;
- treat a value it does not know as "something else", not as an error.

## No writes

The three commands read only. This is enforced, not just intended:

- **Postgres refuses writes.** The commands open their own session in which every transaction is
  read-only (`default_transaction_read_only = on`, set when the connection starts). Postgres
  itself rejects any write, whatever code path tries it.
- **No registration, no seed.** The connect runs the same migration checks as `htui worker` (all
  reads), and then nothing else. It does not register this box in the `box` table, and it does not
  seed an empty database. That is why `box.registered` can be `false`.
- **No `box.toml` is created.** `box.toml` is read if it exists and never written, not even when
  this machine's hostname changed. Without it, `box` is `null`.
- **No config directory is created.** htui's config directory is located, not created.
- **No local cache.** The commands never open htui's local copy of the store.
- **No index is created.** `--search-items` opens the Qdrant collection only if it exists. It
  creates no collection, no payload index, and stamps no embedder on an existing collection.
  A missing collection answers `"index": "empty"`.
- **`--log`** writes only the file you name.

One exception: `--search-items` downloads the embedding model (about 134 MB) into your user cache
directory if this machine does not have it yet, because it needs the model to turn the query into
a vector. That is a download cache, not htui's state. Its progress notes go to standard error, never
standard output.

A connection pooler such as PgBouncer rejects the startup parameter that makes the session
read-only (`unsupported startup parameter: options`, SQLSTATE `08P01`). Every command then ends
with `"store": "refused"`, `error.code` `store_refused` and exit 2, never `offline`: the pooler
answered, and retrying does not help. Connect these commands to Postgres directly, not through
the pooler.

## Credentials and crash reports

- **The DSN comes from the OS keyring only**, as for every htui command (`R-STO-1`). It is never
  read from the command line or the environment. Store it with `htui --set-dsn`. The Qdrant URL and
  API key also come from the keyring.
- **Crash reports.** Exit 2 and exit 3 are never sent to htui's crash reporter: an unreachable
  store and a refusal are states, not crashes. Exit 1 is reported.

## Text output

Without `--json`, a successful answer is printed as text on standard output; a failure prints
nothing there, and its sentence goes to standard error as `htui: <sentence>`.

- **`htui status`** prints the top bar's line:

  ```
  Platform · desk · online · 1 working · 2 waiting
  ```

  `no workspace` and `no box` stand in for a missing workspace or `box.toml`.
- **`htui waiting`** prints one line per row: item key, reason (`judge failed` with a space, as in
  the overlay), step, age (`42s`, `5m`, `3h`, `2d`) and text. No rows prints nothing.
- **`htui --search-items`** prints one line per hit, as before. No hits, or no index yet, prints a
  note on standard error and exits 0.

Control characters are removed from stored text before it is printed, so a note cannot drive your
terminal. Text output is for people; scripts should use `--json`.
