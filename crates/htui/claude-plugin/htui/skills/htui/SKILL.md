---
name: htui
description: "Read htui's state with the htui CLI and hand work to htui instead of doing it inline. Use when the person mentions htui, asks what is waiting on me or what is waiting on them, asks what is htui doing or for htui's status, says hand this to htui or put this in htui's queue, or asks is there an htui item for something."
---

# htui

Three htui commands read htui's state without starting the TUI. Each one reads, prints one JSON
document on one line with `--json`, and exits. This skill says how to run them, how to read what
they print, and what to do when work belongs in htui.

## When to use it

- The person asks about htui's state: what is working, what waits on them, which item a gate or
  permission belongs to.
- The person asks whether an htui item already covers something.
- The person wants work handed to htui, or a request looks like it belongs in htui's queue.

## The commands

Always pass `--json`. Without it the output is text for people, and the exit codes are the same.

| Command | Prints |
|---|---|
| `htui status --json` | The top bar's facts: the workspace, its projects, this box, how many runs are working (`working`) and how many rows wait on the person (`waiting`). |
| `htui waiting --json` | The waiting-on-you list: the same rows as the TUI's overlay, each with how long it has waited (`rows`). |
| `htui --search-items='<query>' --json` | The concepts index's best hits for the query (`hits`). Keep the query in one argument after `=`: a query starting with `-` would otherwise be read as a flag. |

Scope:

- `status` and `waiting` read the **first workspace** by default: the first by name, the one the
  TUI opens on.
- `--project <SLUG>` reads that one project alone (`htui waiting --project <SLUG> --json`). A slug
  no workspace lists is refused with `unknown_project`.
- `--search-items` searches **every workspace** by default; `--project <SLUG>` narrows it to one
  project. `--limit <N>` sets how many hits (1 to 1000, default 10), and `--decisions` keeps only
  items closed as `done`, `concluded` or `rejected`, and their documents.
- The first search on a machine may download the embedding model (about 134 MB) into the user
  cache directory; its progress goes to standard error, never standard output.

## The envelope

Every document starts with three fields:

- `version`: the schema version, `1` today.
- `kind`: `"status"`, `"waiting"` or `"search"`, the command that answered.
- `store`: Postgres's state. `"online"` (it answered), `"offline"` (it did not answer),
  `"refused"` (the command refused before an answer) or `"error"` (a read failed after the
  connect).

When `store` is `"online"`, the command's own fields follow. Otherwise one `error` object follows
and nothing else.

A search document also has `index`, the Qdrant concepts index's state: `online` (it answered;
`hits` may be empty), `empty` (no index yet; `hits` is `[]`; building it is the person's step: `htui --index-items`, which writes the index, so never run it yourself),
`offline`, `refused` or `error` (no `hits`, and an `error` object instead).

**Ignore fields you do not know, and treat a value you do not know as "something else", not as an
error.** New fields and new values (a new `reason`, `index`, `store`, `error.code` or `place`)
arrive without a `version` change.

## Exit codes and failures

| Exit | Meaning |
|---|---|
| 0 | Answered. For a search, the index answered or is empty. |
| 3 | Postgres did not answer; for a search, Qdrant did not answer. |
| 2 | Refused: no DSN, the keyring, `box.toml`, the schema, the scope, an embedder mismatch, a refused Qdrant API key. A usage error also exits 2, with no document. |
| 1 | Failed: a read failed after the connect, or the embedding model could not load or failed on the query. |

A failure document carries `error.code` (why, for a program; for example `unreachable`,
`no_dsn`, `unknown_project`, `migrations_pending`) and `error.message` (the sentence htui prints
for a person). Report `error.message` to the person as it is; do not parse it, it may change. An
offline store is reported as offline: htui never answers from a local cache here, so a document is
never stale data presented as live.

**Treat an empty standard output as a failure, whatever the exit code.** A usage error (exit 2), a
`--log` file that cannot be opened (exit 1) and a crash (exit 101) print nothing on standard
output; the reason is on standard error.

## Reading `waiting` rows

`rows` holds one object per row of the TUI's waiting-on-you overlay, in the overlay's order. An
empty list is `[]`, and the count of waiting rows is the length of `rows`.

- `reason`: why the row waits on the person.
  - `gate`: approve or reject a step parked at `awaiting_approval`.
  - `judge_failed`: pick a fan-out winner after the judge failed.
  - `selection`: pick a fan-out winner (no judge verdict).
  - `unblock`: press `u` on a blocked item: reopen it, follow its parked run, or resume the run.
  - `interrupted`: retry, promote or cancel a run parked by an interrupt.
  - `permission`: answer an agent's permission request.
- `item_key`: the item's key, such as `MOD-4`.
- `step`: the step as the Runs pane labels it after its phase, such as `prd 0.1`,
  `research 0.1/1` (a fan-out slot) or `research 0.1/j` (its judge); `null` when there is no step.
- `text`: the reason's text: the gate note or judge note, the tool a permission asks for, or the
  Unblock sentence.
- `age_seconds`: whole seconds from `since` (when the row began waiting) to now, never below 0.
  It is measured with the clock of the machine that ran `htui waiting`, against `since` on the
  database's clock; when the two clocks disagree, the age is off by the difference.

In `status` and `waiting`, `permissions_known: false` means the open permission requests could not
be listed, so permission rows are missing.

## Reading search hits

`hits` holds the best hits first. Each hit is one place where the query matched:

- `key`: the item's or requirement's key. Only an item or document hit carries an item key.
- `place`: what matched: `item`, `document` (one of the item's documents) or `requirement`.
- `document_kind`: the document's kind, such as `prd`, on a document hit; otherwise `null`.
- `resolution`: how the item closed (`done`, `concluded`, `rejected`, `withdrawn`, `superseded`,
  `duplicate`) on an item or document hit of a closed item; otherwise `null`. On an item or
  document hit, `null` means the item is not closed.
- `requirement_state`: `active` or `withdrawn` on a requirement hit; otherwise `null`.
- `score`: the fused rank score. Compare scores within one search only.
- `snippet`: the start of the indexed text, whitespace folded and control characters removed.

## Handing work to htui

- When a request matches an open htui item, or belongs in htui's queue, say so and point the
  person at the htui TUI. Name the item by its key when a row (`item_key`) or a search hit
  (`key`) gives one.
- Name a search hit's item as open only when its `place` is `item` or `document` and its
  `resolution` is `null`. A hit with a `resolution` is a closed item: say how it closed. A
  `requirement` hit's `key` is a requirement's key, not an item's.
- Never work such a request inline in this session.
- Never try to write htui's store. None of these three commands writes, and you run no other htui command.

## What this skill never does

The three commands only read, and this skill runs no other htui command. Never connect to htui's Postgres or Qdrant
directly; read htui's state through these commands only.

## Reference

The full schema is `docs/htui-cli.md` in htui's repository (`https://git.mluigi.it/htui.git`).
