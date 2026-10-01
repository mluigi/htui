# ANA-27 - OpenRig survey: ideas and concepts to assimilate (concluded, 2026-10-01)

Opened 2026-10-01 by the maintainer. OpenRig (https://openrig.dev/, `@openrig/cli`,
`github.com/mvschwarz/openrig`, Apache-2.0) is a Node CLI and daemon that declares a team of
Claude Code and Codex agents in a file: a lead delegates to durable specialists that own queue
items (`rig queue`), consults them without creating a task (`rig send`), and spawns temporary
subagents. The question was which of its ideas htui should take. Addresses `R-ID-2`..`R-ID-6`,
`R-AGT-4`, `R-AGT-7`, `R-AGT-8`, `R-ORCH-2`, `R-ORCH-4`, `R-ORCH-5`, `R-ORCH-7`, `R-PRM-1`,
`R-HIS-1`, `R-HIS-2`, `R-MCP-2`, `R-TUI-1`, `R-TUI-4`, `R-TUI-6`, `R-NF-1`, `R-NF-3`. Analysis:
`docs/ANA-27.md`.

**Model (`docs/ANA-27.md` §2).** A Hono daemon on SQLite (89 migrations) drives long-lived
interactive sessions in tmux panes and delivers work by typing into them, guarded against bare
shells, open prompts and a person typing. The team is a "rig" in a RigSpec YAML file; a "pod" is a
sub-group inside it, not the whole team. Queue rows carry the obligation, wakes are pointers, a
300 s sweep raises stuck findings, liveness is derived from rows, and a failed resume rolls back
and asks instead of starting fresh. Released since 2026-04-06 (52 npm versions, 0.6.3 latest).

**Verdict (`docs/ANA-27.md` §5).** OpenRig's three load-bearing ideas are **rejected**, each on a
recorded htui decision: an LLM lead that assigns and closes work (`R-ID-6`, ANA-2 invariant 3),
durable specialist sessions that accumulate context (`R-PRM-1`, MOD-24's rescope), and tmux
transport (`R-ID-2`, `R-NF-1`). Also rejected: a team file in the repo, an agent message bus,
untracked native subagents, least-loaded selection, agent-driven restart, writing agent config,
timed wakes, compaction enforcement, work-tree progress files, a Slack gateway, and pods as a
unit. **Taken**, as the discipline OpenRig wraps around its rows: the row is the obligation and
a notification only its id; no agent answers a prompt and an answer names its request; liveness
derived from rows with "unknown" as a value; deadlines enforced while a session runs; a
handoff opening is labelled and a failed resume reported; a swarm step yields through a typed
exit and a consult is a recorded read-only child step; one list of everything waiting on a
person; a persona only narrows its base; pid plus start time as a process identity; seeded kill
points in the crash test.

**Fact-check.** Two adversarial verifiers checked 164 claims against OpenRig's source and docs
and against the htui tree; 49 corrections were applied before the verdict was put to the
maintainer. Four changed a take: T5 (htui already hands off on every non-CLI promotion,
`promote::opening_kind`, D192, so the gap is labelling and a failed CLI `--resume`), T10 (only if
MOD-16 R-10 picks pid-and-signal), T6/T7 (overrides ANA-13 §3.3's shared context window, named
for the MOD-27 PRD), and the tmux reject (the delivery regressions OpenRig records are guard
bugs, not harness UI changes; the reject stands on `R-ID-2`/`R-NF-1`).

**Decisions (maintainer, 2026-10-01).** Verdict accepted as written. The proposed `R-TUI-11` and
the matching `R-TUI-1` top-bar line stay a proposal in `docs/ANA-27.md` §7; MOD-69 carries them
as an open question.

**Spawned and amended.** **MOD-69**: waiting-on-you list across items. ANA-27 notes appended to
MOD-42 (T1-T2), MOD-43 (T3), MOD-37 (a new Deadline entry, T4; R-48, T5), MOD-27 (T6-T7), MOD-26
(T9), MOD-16 R-10 (T10) and MOD-24 (T11).

Commits: analysis and close-out in the commit that added this file.
