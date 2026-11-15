# htui from scripts: `status`, `waiting`, `--search-items --json`, `stop` and `resume`

Three commands let a script, a status bar or another tool read htui's state without starting the
TUI, and two more stop and resume a workspace's runs from a shell (see
[Stopping and resuming a workspace](#stopping-and-resuming-a-workspace)). Each of the three
reading commands reads, prints one answer and exits:

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
- [Setting the Qdrant URL and API key](#setting-the-qdrant-url-and-api-key)
- [Stopping and resuming a workspace](#stopping-and-resuming-a-workspace)
- [Credentials and crash reports](#credentials-and-crash-reports)
- [Text output](#text-output)

See also: [the Claude Code plugin](htui-claude-plugin.md), which reads these commands from a
Claude Code session.

## The commands and their flags

| Command | Flags |
|---|---|
| `htui status` | `--json`, `--project <SLUG>` or `--cwd <PATH>`, `--log <PATH>` |
| `htui waiting` | `--json`, `--project <SLUG>` or `--cwd <PATH>`, `--log <PATH>` |
| `htui --search-items <QUERY>` | `--json`, `--project <SLUG>` or `--cwd <PATH>`, `--decisions`, `--limit <N>`, `--log <PATH>` |
| `htui stop <WORKSPACE>` | `--force`, `--yes` (`-y`), `--log <PATH>`. Writes: see [Stopping and resuming a workspace](#stopping-and-resuming-a-workspace) |
| `htui resume <WORKSPACE>` | `--yes` (`-y`), `--log <PATH>`. Writes: see [Stopping and resuming a workspace](#stopping-and-resuming-a-workspace) |

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
  (`htui status --log x.log`). `HTUI_LOG` works too. The TUI also keeps its log in memory and
  **Settings › Logging** can set a file; neither applies to these commands.

`status` and `waiting` are subcommands, so no TUI flag may stand beside them: `htui --demo status`
and `htui --offline waiting` are usage errors. `--json`, `--cwd`, `--decisions` and `--limit` at
the top level need `--search-items`; `htui --json` alone is a usage error.

## Scope: what a command reads

`status` and `waiting` read one scope:

- **By default, the first workspace.** That is the first workspace by name, the one the TUI opens
  on. Its projects are read in the workspace's own order. Both commands also read the workspace's
  own items (such as `WS-1`, made in the workspace rather than in one of its projects): `status`
  counts their runs and rows in `working` and `waiting`, and `waiting` lists them first. With
  `--project` or `--cwd`, neither command reads them.
- **`--project <SLUG>`** reads that project alone. Project slugs are unique across the store. A
  project can belong to several workspaces; the document then reports the first of them by name.
  A slug no workspace lists is refused with `unknown_project`.
- **`--cwd <PATH>`** reads the project whose checkout on this box contains `PATH`:
  - `PATH` is put through the file system's canonical form on this machine. A relative `PATH`
    resolves against the current directory, symlinks are followed, and on Windows the drive
    prefix and the casing compare alike. The registered paths are compared as stored: htui stores
    them in that same canonical form when a path is set or found, and never touches them on disk
    here, so a registered checkout on an unreachable network mount cannot stall the command.
  - Containment is per path component: `/src/htui2` is not inside `/src/htui`.
  - The deepest checkout wins, so a checkout nested inside another one is found.
  - A registered path that no longer exists on disk cannot contain `PATH`, so it never matches.
  - `PATH` that cannot be resolved is refused with `cwd_unresolved`; `PATH` in no registered
    checkout with `cwd_unmatched`.
  - When the deepest checkout is registered for two or more projects, htui does not pick one: it
    refuses with `cwd_ambiguous`. Name the project with `--project` instead.
  - `--cwd` needs this box's `box.toml`, because the box id decides which checkouts are "this
    box's". Without one it is refused with `no_box`; start the TUI once so the box gets an
    identity.

`--search-items` keeps its own default: **every workspace**, each workspace's own items and
documents (its `WS` items and its `goals` document) included, before its projects.
`--project` and `--cwd` narrow it to one project, with the same rules as above, and
`--project @<workspace>` (a workspace's slug after `@`, such as `@platform`) narrows it to that
workspace's own items and documents alone. `--index-items` covers the same scopes.

Each workspace has its own Qdrant collection, named after the workspace's id (`htui_ws_<id>`),
and there is one global collection (`htui_global`). A search reads each workspace's collection for
that workspace's projects, plus the global one, and merges the hits by score; a project in two
workspaces is found once. `--cwd` searches its project in every workspace that holds it.

`--index-items` indexes a workspace's collection only when no other htui process is indexing it.
Every TUI and `htui worker` connected to the database competes to index every collection, and the
one holding a collection's lease indexes it (see `htui-worker.md`, "The concepts index"). For a
workspace whose lease another box holds, `--index-items` prints
`workspace <slug>: indexing runs on <box>; not indexed here` on standard error, where `<box>` is
that box's hostname (or the last eight hex digits of its id when its row is gone), skips that
workspace and goes on; the run still exits 0. It lets go of the leases it took when it ends. When
it can take the global collection's lease it creates that collection, and drops the single shared
collection earlier versions wrote (`htui_concepts_v2`).

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
      "project": "htui",
      "qualified_key": "htui:MOD-4",
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
      "project": "htui",
      "qualified_key": "htui:MOD-7",
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

Without `--project` or `--cwd`, the rows also cover the workspace's own items, and those come
first. Such a row's `project` is the workspace's slug with an `@` in front (`@platform`), and it is
not in `projects`. With `--project` or `--cwd` the rows cover that one project only.

| Field | Type | Meaning |
|---|---|---|
| `reason` | string | Why the row waits. See the table below. |
| `item_key` | string | The item's key, such as `MOD-4`. |
| `project` | string | The slug of the item's project, such as `htui`; `@<workspace>` for a workspace's own item, such as `@platform`. |
| `qualified_key` | string | `project` and `item_key` joined by `:`, such as `htui:MOD-4` or `@platform:WS-1`. It names the item in every project, unlike `item_key`. |
| `item_id` | uuid | The item's id. |
| `run_id` | uuid or `null` | The run the row is about. `null` for an Unblock row that reopens the item (no run). |
| `step_id` | uuid or `null` | The step the row is about. `null` when the row is about a whole run, or about no run. |
| `step` | string or `null` | The step as the Runs pane labels it after its phase: `prd 0.1`, `research 0.1/1` (a fan-out slot), `research 0.1/j` (its judge). `null` when there is no step. |
| `text` | string | The reason's text: the gate note or judge note, the tool a permission asks for, the Unblock sentence, or a landing row's refused repos. |
| `since` | timestamp | When the row began waiting. See below. |
| `age_seconds` | integer | Whole seconds from `since` to now, never below 0. |

The `reason` values:

| `reason` | The row waits on you to... | `since` is |
|---|---|---|
| `gate` | approve or reject a step parked at `awaiting_approval` | the step's `updated_at` |
| `judge_failed` | pick a fan-out winner after the judge failed | the judge step's `updated_at` |
| `selection` | pick a fan-out winner (no judge verdict) | the run's `updated_at` |
| `unblock` | press `u` on a blocked item: reopen it, follow its parked run, or resume the run | the run's `updated_at`; for a reopen row (no run), the item's `updated_at` |
| `landing` | press `u` to retry a finished run's refused landing; the text names each refused repo, its mode and target branch, and the reason | the newest refusal's `updated_at` |
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
| `refused` | Qdrant answered and refused the API key (missing, wrong or without access). Fix it in **Settings › Qdrant**. | no `hits` | `qdrant_refused` | 2 |
| `error` | The embedding model could not be downloaded or loaded, or failed to turn the query into a vector. | no `hits` | `model_unavailable` | 1 |

| Field | Type | Meaning |
|---|---|---|
| `index` | string | The index's state, above. |
| `query` | string | The query, as given. |
| `decisions` | boolean | Whether `--decisions` was given. |
| `hits` | array | Present for `online` and `empty` only. The best hits first. |
| `hits[].key` | string | The item's or requirement's key, or the owning project's slug on a project or workspace document (`@platform` for the `platform` workspace's own). |
| `hits[].place` | string | What matched: `"item"`, `"document"` (one of the item's documents, or a project's or a workspace's own document) or `"requirement"`. |
| `hits[].document_kind` | string or `null` | The document's kind, such as `prd`, on a document hit; otherwise `null`. |
| `hits[].resolution` | string or `null` | How the item closed (`done`, `concluded`, `rejected`, `withdrawn`, `superseded`, `duplicate`), on an item or document hit of a closed item; otherwise `null`. |
| `hits[].requirement_state` | string or `null` | `active` or `withdrawn`, on a requirement hit; otherwise `null`. |
| `hits[].score` | number | The fused rank score. Compare scores within one search only. |
| `hits[].snippet` | string | The start of the indexed text, whitespace folded and control characters removed. |
| `error` | object | Present for `offline`, `refused` and `error` only: `code` and `message`, as in [Failure documents](#failure-documents). |
| `unavailable` | array of strings | Present only when some collections answered and others did not: the slugs of the workspaces whose collection is missing (never indexed) or refused the search. Their projects are not in `hits`. Absent otherwise. |

When every collection is missing, `index` is `empty` (exit 0), as before. When no workspace's
collection answered and one failed, `index` is that failure (`offline`, `refused` or `error`) with
its exit, and there is no `unavailable`. The global collection answering does not count here: it
is empty on most boxes, so a workspace whose collection refuses the search (another embedder's
vectors, say) is still a failure when it is the only workspace searched.

A failure before the index is asked (no DSN, Postgres unreachable, no Qdrant URL, an unknown
project) is a [failure document](#failure-documents) with `"kind": "search"`, and has no `index`,
`query` or `decisions`.

A search that cannot be answered because of Qdrant says so first: its `error.message` (and the
line on standard error) starts with `knowledge search unavailable: `, followed by the cause, for
example `knowledge search unavailable: cannot reach Qdrant: …`. That is every `index` failure
(`offline`, `refused`, `error`), and every failure to read the Qdrant settings after the connect:
`no_qdrant_url`, `qdrant_url_unusable`, `keyring_unreadable` for the Qdrant API key, and an
`unreachable` or `read_failed` while reading the `qdrant_url` setting. `empty` is not a failure
and has no message. A failure before the Qdrant settings are read (no DSN, `box.toml`, the connect)
and a scope refusal keep their own sentence. The prefix changes no `error.code`, `store` or exit
code; a program reads `error.code`, never the message.

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
| `keyring_unreadable` | `refused` | 2 | all | The OS keyring could not be read: for the DSN, or (search) for the Qdrant API key. |
| `box_identity` | `refused` | 2 | all (search only with `--cwd`) | `box.toml` exists but cannot be read or parsed. `status` and `waiting` always read it (for `box`), so a malformed `box.toml` refuses them even without `--cwd`, just as the TUI will not start with one. Fix or delete the file. |
| `migrations_pending` | `refused` | 2 | all | The database has migrations this build has not applied. Start the TUI once to apply them. |
| `below_target` | `refused` | 2 | all | The database's target version is newer than this build. Update htui. |
| `store_refused` | `refused` | 2 | all | Postgres answered and refused this htui: a newer or drifted schema, a partial migration, a wrong password, a missing database, a pooler that rejects the read-only session (see [No writes](#no-writes)). |
| `unknown_project` | `refused` | 2 | all | `--project` names no project (or, as `@<workspace>` on `--search-items`, no workspace). |
| `cwd_unresolved` | `refused` | 2 | all | `--cwd`'s path cannot be resolved on this machine. |
| `no_box` | `refused` | 2 | all | `--cwd` was given, but this machine has no `box.toml`. |
| `cwd_unmatched` | `refused` | 2 | all | `--cwd`'s path is in no repository registered on this box. |
| `cwd_ambiguous` | `refused` | 2 | all | `--cwd`'s deepest checkout is registered for two or more projects. |
| `read_failed` | `error` | 1 | all | A read failed after the connect, with something other than an unreachable store. |
| `no_qdrant_url` | `refused` | 2 | search | No Qdrant URL is set: no `HTUI_QDRANT_URL` in the environment and no `qdrant_url` setting in Postgres. Set one in **Settings › Qdrant** or with `htui --set-qdrant-url`. |
| `qdrant_url_unusable` | `refused` | 2 | search | The Qdrant URL cannot be used: `HTUI_QDRANT_URL` is not a valid Qdrant URL, or the `qdrant_url` setting in Postgres is not one. Set it again in **Settings › Qdrant** or with `htui --set-qdrant-url`. |
| `qdrant_unreachable` | `online` | 3 | search | Inside a search document: Qdrant did not answer (`index: "offline"`). |
| `embedder_mismatch` | `online` | 2 | search | Inside a search document: the index holds another model's vectors (`index: "refused"`). |
| `qdrant_refused` | `online` | 2 | search | Inside a search document: Qdrant refused the API key (`index: "refused"`). |
| `model_unavailable` | `online` | 1 | search | Inside a search document: the embedding model could not load, or failed on the query (`index: "error"`). |

The checks run in this order, and the first one that fails decides the document:

- `status` and `waiting`: the DSN, `box.toml`, the connect, the scope, then the reads.
- `--search-items`: the DSN, `box.toml` (only with `--cwd`), the connect, the Qdrant URL and
  API key, the scope, the index (whether Qdrant answers and the collection exists), the embedding
  model, then the search. The Qdrant URL is a setting in Postgres, so it is read after the connect:
  a box with no DSN or an unreachable Postgres is refused with `no_dsn` or `unreachable` before
  `no_qdrant_url`. Before MOD-158 the URL was checked first; a script that told "no URL" from "no
  Postgres" by the order should read `error.code`. The model is loaded only when there is a
  collection to search, so a Qdrant that does not answer, refuses the API key or holds no index is
  answered without it. Every failure to read the Qdrant URL or API key, and every index failure,
  has a `message` that starts with `knowledge search unavailable: `; a scope refusal keeps its own
  sentence (see [`search`](#search)).

## Exit codes

| Exit | Meaning | `store` | Document |
|---|---|---|---|
| 0 | Answered. For a search, the index answered or is empty. | `online` | yes |
| 3 | Postgres did not answer; for a search, Qdrant did not answer. | `offline`, or `online` with `index: "offline"` | yes |
| 2 | Refused: no DSN, the keyring, `box.toml`, the schema, the scope, an embedder mismatch, a refused Qdrant API key. | `refused`, or `online` with `index: "refused"` | yes |
| 1 | Failed: a read failed after the connect, or the embedding model could not load or failed on the query. | `error`, or `online` with `index: "error"` | yes |

An offline store is reported as offline: htui never answers from its local cache here, so a
document is never stale data presented as live.

`--search-items` without `--json` uses the same exit codes, and prints the document's
`error.message` on standard error: `knowledge search unavailable: <cause>` for a search Qdrant
could not answer. Earlier versions exited 1 for every failure.

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
directory if this machine does not have it yet and there is an index to search, because it needs
the model to turn the query into a vector. That is a download cache, not htui's state. Its progress notes go to standard error, never
standard output.

A connection pooler such as PgBouncer rejects the startup parameter that makes the session
read-only (`unsupported startup parameter: options`, SQLSTATE `08P01`). Every command then ends
with `"store": "refused"`, `error.code` `store_refused` and exit 2, never `offline`: the pooler
answered, and retrying does not help. Connect these commands to Postgres directly, not through
the pooler.

## Setting the Qdrant URL and API key

Four flags change what `--search-items`, `--index-items`, the TUI and `htui worker` use to reach
Qdrant. Unlike the three commands above, they write. Each acts, prints one line on standard error
and exits; none starts the TUI.

**The URL is one setting for every box; the key is this box's.** The URL lives in Postgres (the
`app_setting` row `qdrant_url`), so setting it on one box changes it for every box on the same
database, the next time each searches or indexes. The API key stays in each box's OS keyring.

| Flag | What it does |
|---|---|
| `htui --set-qdrant-url <URL>` | Sets the URL every box uses. |
| `htui --clear-qdrant-url` | Removes the URL every box uses. |
| `htui --set-qdrant-key` | Reads one line from standard input and stores it as this box's Qdrant API key in the OS keyring. On a terminal it prompts first; the key is visible as you type it. |
| `htui --clear-qdrant-key` | Removes this box's Qdrant API key from the OS keyring. |

- **The URL** starts with `http://` or `https://` and names a host, with an optional port and
  path, such as `http://qdrant.lan:6334`. A URL with a user name or password, a query (`?...`) or a
  fragment (`#...`) is refused, and so is any space or character outside printable ASCII: the API
  key goes in the keyring, never in the URL. The host is a name of letters, digits, `-`, `.`, `_`,
  `~` and `%XX` escapes, or an IPv6 address in brackets (`http://[::1]:6334`). The URL is not a
  secret, so it may be an argument.
- **The key** is never an argument: it would end up in your shell history and in the process
  list. Pipe it in (`printf '%s\n' "$KEY" | htui --set-qdrant-key`) or paste it at the prompt.
- **The URL flags connect to Postgres** with the DSN stored in the OS keyring, as
  `--index-items` does: run `htui --set-dsn` first. A pending migration or a database newer than
  this build is refused, never migrated. The write is a compare-and-set: when another box changes
  the URL between the read and the write, nothing is written and you are told to run the command
  again.
- **`HTUI_QDRANT_URL`** overrides the setting in every process that has it in its environment.
  `--set-qdrant-url` still writes the setting, and says so when the variable is set where you ran
  it.
- These flags stand alone: none of them may be combined with another of them, with `--set-dsn`,
  `--clear-dsn`, `--dsn-stdin`, `--demo`, `--offline`, `--index-items`, `--search-items` or its
  own flags (`--project`, `--decisions`, `--limit`, `--json`, `--cwd`), the key file flags or a
  subcommand.

| Ending | Standard error | Exit |
|---|---|---|
| The URL was set | `Qdrant URL set for every box: <URL>.`, plus `HTUI_QDRANT_URL overrides it in processes that have it.` when that variable is set | 0 |
| The URL was cleared | `Qdrant URL cleared for every box.`, or `no Qdrant URL is set; nothing was cleared` | 0 |
| The key was stored or removed | `Qdrant API key stored in this box's OS keyring.` / `Qdrant API key removed from this box's OS keyring.` | 0 |
| The URL is refused | `not setting the Qdrant URL: <why>` (the URL itself is never repeated) | 2 |
| Another box changed the URL meanwhile | `the Qdrant URL changed while it was being set; nothing was written — run the command again`, or `the Qdrant URL changed while it was being cleared; nothing was removed — run the command again` | 2 |
| The OS keyring could not be read for the DSN (URL flags) | `the OS keyring could not be read: <the keyring's error>` | 2 |
| The OS keyring refused the key (key flags) | `the Qdrant API key could not be stored in the OS keyring: <the keyring's error>`, or `the Qdrant API key could not be removed from the OS keyring: <the keyring's error>` | 2 |
| No DSN, `box.toml`, the schema | the same sentences as [Failure documents](#failure-documents) | 2 |
| A blank key line | `no Qdrant API key on stdin; nothing was stored` | 2 |
| Postgres did not answer | `cannot reach Postgres: ...` | 3 |
| Anything else | the error | 1 |

Each line on standard error starts with `htui: ` when the flag fails. Exits 2 and 3 are never sent
to htui's crash reporter.

## Stopping and resuming a workspace

`htui stop <WORKSPACE>` stops every run in a workspace, and `htui resume <WORKSPACE>` lets them go
on (`R-ORCH-16`). `<WORKSPACE>` is the workspace's slug, matched exactly; an unknown slug is
refused and the refusal lists the slugs there are. Unlike the three reading commands, these two
write: they connect to Postgres as the Qdrant URL flags do (the DSN in the OS keyring, `box.toml`
created on a first run, a pending migration or a database newer than this build refused, never
migrated).

**What a stop does.** A stop is one record on the workspace; each box running one of its runs
applies it at its next command poll, and no run of the workspace starts while it stands.

- **Graceful** (the default): each agent step stops at its turn boundary and keeps its session;
  checks and htui's own steps stop at once.
- **Forced** (`--force`): every step is killed mid-turn and redoes that turn when it resumes.
  `--force` on a workspace already stopped gracefully escalates the stop. A graceful request never
  downgrades a forced stop.

Asking again for a stop that is already in force (graceful over graceful, anything over forced)
writes nothing and asks nothing: it prints `workspace <slug> is already stopped (<mode>)` and the
report, plus `a graceful stop does not downgrade a forced one` for a graceful request over a forced
stop. That is also how you look at a stop again later.

**What a resume does.** It removes the record. Each stopped run resumes on the box that ran it, at
that box's next command poll; a box that is down resumes its runs when it comes back. Queued runs
can start again. A resume of a workspace that is not stopped writes nothing and asks nothing: it
prints `workspace <slug> is not stopped` and the report, which still lists any stopped run.

**The question.** Before it writes (not with `--yes`), each command prints the report as it
stands on standard output and asks one question on standard error, such as:

```
Stop workspace `graphics` gracefully? Each agent step stops at its turn boundary, checks stop at once; nothing new starts until `htui resume graphics`. [y/N]
```

`y` or `yes` (any case) confirms; anything else, an empty line or the end of input declines, and
nothing is written. `--yes` (`-y`) answers it without asking. Without a terminal on standard input,
a command that would write needs `--yes`: it is refused before the stop is written (or removed).
If someone else stops or resumes the workspace while the question is open, the answer writes
nothing: the command is refused with `the stop of workspace <slug> changed while asking; nothing
was stopped; run htui stop <slug> again` (`… resumed; run htui resume <slug> again`), and you run
it again to see the stop as it now stands.

**The wait.** After writing a stop, `htui stop` waits up to 20 seconds for the workspace's running
runs to stop, saying `waiting for <n> running runs to stop…` once on standard error. A graceful
stop settles within a few seconds and one command poll. If the wait runs out, it prints the report
as it stands and `still stopping; run htui stop <slug> again to see it settle`, and exits 0. Ctrl-C
during the wait is harmless: the stop is already recorded. `htui resume` does not wait.

**The report** goes to standard output, a header and one line per run that is not finished, oldest
first:

```
workspace graphics: stopped (graceful, requested 2026-11-14 10:02 UTC)
  GFX-3  0193a1b2  stopped on DESKTOP-HTUI  resumes: implement reopens its agent session
  GFX-4  0193a1c4  stopped on build-box     resumes: review starts a fresh session with the handoff
  GFX-5  0193a1d0  stopped, no box          cannot be resumed: cancel it from the Runs pane
  GFX-6  0193a1e9  running on build-box     stopping
  GFX-7  0193a1f3  queued                   starts when the workspace is resumed
  GFX-8  0193a201  awaiting approval        its gate can be answered once resumed
checks rerun after each resumed step
once resumed, each box resumes its stopped runs at its next command poll
```

- The header's state is `not stopped`, `stopping` (stopped, and a run is still running) or
  `stopped`, with the stop's mode and when it was requested.
- Each line names the item's key (`-` for a run without an item), the run id's first 8 hex digits,
  the run's state and its box (the box's hostname as your boxes list it, else its id).
- A stopped run says how each of its stopped steps resumes: `<phase> reopens its agent session`
  (the step's own session, when its agent can resume one), `<phase> starts a fresh session with the
  handoff` (a summary of what it did so far), or `<phase> runs again from its start` (a step that
  had not started a session, and a judge). A stopped run with no stopped step `continues from where
  it stopped`. Checks always rerun after a resumed step.
- A stopped run with no box can never be resumed: cancel it from the Runs pane.
- Whether a box is online is not checked: a line names the box, and that box resumes the run at
  its next command poll.
- A workspace with no unfinished run prints `no runs in this workspace`.
- After a resume the report follows
  `resume requested: each box resumes its stopped runs at its next command poll`.

| Ending | Exit |
|---|---|
| Stopped, resumed, or nothing to write and the state printed (the wait running out included) | 0 |
| Refused: no DSN, the keyring, `box.toml`, the schema, an unknown slug, `not confirmed; nothing was stopped` (`… resumed`), `needs confirmation: pass --yes, or run it in a terminal; nothing was stopped` (`… resumed`), `the stop of workspace <slug> changed while asking; …`, or `workspace <slug> no longer exists` | 2 |
| Postgres did not answer: `cannot reach Postgres: ...` | 3 |
| Anything else: `the workspace could not be stopped: ...` (`resumed`, `read`) | 1 |

The question and the wait line go to standard error; a refusal is printed there after `htui: `.
Exits 2 and 3 are never sent to htui's crash reporter.

## Credentials and crash reports

- **The DSN comes from the OS keyring only**, as for every htui command (`R-STO-1`). It is never
  read from the command line or the environment. Store it with `htui --set-dsn`.
- **The Qdrant URL** comes from `HTUI_QDRANT_URL` when it is set (for development and CI), else
  from the `qdrant_url` setting in Postgres, which every box shares. **The Qdrant API key** comes
  from `HTUI_QDRANT_API_KEY` when it is set, else from this box's OS keyring. See
  [Setting the Qdrant URL and API key](#setting-the-qdrant-url-and-api-key).
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
  the overlay), step, age (`42s`, `5m`, `3h`, `2d`) and text. No rows prints nothing. The key is
  printed `slug:KEY` (`htui:FEAT-2`, `@platform:WS-1`) when another project of the answer has a
  kind with the same prefix, and bare otherwise, as in the overlay. The key column is as wide as
  the longest key.
- **`htui --search-items`** prints one line per hit, as before. No hits, or no index yet, prints a
  note on standard error and exits 0. When some workspaces' collections could not be read, it
  also prints `knowledge search unavailable for workspace(s): <slug>, …` on standard error,
  followed, when one of them failed rather than being missing, by why the first one failed in
  parentheses (`(embedder mismatch: …)`). The `--json` document names the slugs only.

Control characters are removed from stored text before it is printed, so a note cannot drive your
terminal. Text output is for people; scripts should use `--json`.
