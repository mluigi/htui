import re

with open('/home/mluigi/projects/htui/docs/REQUIREMENTS.md', 'r') as f:
    content = f.read()

# Update header
header_old = """**Status:** approved draft, 2026-09-03; amended 2026-09-08 by maintainer decision on ANA-10
(`docs/ANA-10.md` §6.1, `docs/decisions/ana/ana-10.md`) — R-STO-7 added; R-ID-3, R-ENT-7, R-STO-1,
R-STO-4, R-STO-5, R-HIS-1, R-AGT-4, R-PRM-4, R-SKL-1, R-TUI-1 and R-TUI-8 amended in place;
amended 2026-09-09 by maintainer decision during MOD-2 milestone 6 — R-AGT-9 added (in-app agent
authentication, reversing `docs/ANA-4.md` §4.5; implemented as MOD-21), R-AGT-10 added
(in-app adapter installation, reversing `docs/ANA-4.md` §4.6; implemented as MOD-20), and R-AGT-4's
"`agy` over ACP is unverified" clause withdrawn as settled by ANA-4 and proven by MOD-2 milestone 6"""

header_new = """**Status:** approved draft, 2026-09-03; amended 2026-09-08 by maintainer decision on ANA-10
(`docs/ANA-10.md` §6.1, `docs/decisions/ana/ana-10.md`) — R-STO-7 added; R-ID-3, R-ENT-7, R-STO-1,
R-STO-4, R-STO-5, R-HIS-1, R-AGT-4, R-PRM-4, R-SKL-1, R-TUI-1 and R-TUI-8 amended in place;
amended 2026-09-09 by maintainer decision during MOD-2 milestone 6 — R-AGT-9 added (in-app agent
authentication, reversing `docs/ANA-4.md` §4.5; implemented as MOD-21), R-AGT-10 added
(in-app adapter installation, reversing `docs/ANA-4.md` §4.6; implemented as MOD-20), and R-AGT-4's
"`agy` over ACP is unverified" clause withdrawn as settled by ANA-4 and proven by MOD-2 milestone 6;
amended 2026-09-11 by maintainer decision on MOD-25 withdrawing ANA-10 — R-STO-7 withdrawn,
and previous 11 ANA-10 amendments reverted to online-only requirements;
amended 2026-09-14 by maintainer decision — R-MCP-2 added spawn_subagent, R-AGT-4 added claude-cli."""
content = content.replace(header_old, header_new)

content = re.sub(
    r"- \*\*R-ID-3 \(must\).\*\* Wherever a server is configured, Postgres is the single source of truth\.\n  Everything `htui` knows lives there: items, documents, runs, transcripts, skills, templates, box\n  profiles, agent registry, settings\. A box with no server configured is the exception R-STO-7\n  defines, and is the only one\. Amended by ANA-10 \(`docs/ANA-10\.md` §6\.1\), 2026-09-08\.",
    r"- **R-ID-3 (must).** Postgres is the single source of truth.\n  Everything `htui` knows lives there: items, documents, runs, transcripts, skills, templates, box\n  profiles, agent registry, settings.",
    content
)

content = re.sub(
    r"- \*\*R-ENT-7 \(must\).\*\* Item keys are minted from a per-project, per-prefix sequence held by the store\n  that owns the project: Postgres for a project that exists on a server, the local store for a\n  project created on a box with no server configured \(R-STO-7\)\. Never reused: a project has exactly\n  one counter and exactly one writer of it, and adoption seals a project's local counter in the same\n  transaction that fast-forwards the server's\. Creating an item in a project mirrored from a server,\n  while that server is unreachable, is not supported \(see R-STO-4\)\. Amended by ANA-10\n  \(`docs/ANA-10\.md` §4\.3, §6\.1\), 2026-09-08: the \"never reused\" guarantee is a one-writer-per-counter\n  property, and a project that exists on no server satisfies it by construction\.",
    r"- **R-ENT-7 (must).** Item keys are minted from a per-project, per-prefix sequence held by Postgres.\n  Never reused: a project has exactly one counter and exactly one writer of it. Creating an item\n  in a project mirrored from a server, while that server is unreachable, is not supported (see R-STO-4).",
    content
)

content = re.sub(
    r"- \*\*R-STO-1 \(must\).\*\* Postgres is the only writable \*\*shared\*\* store: a box with a server configured\n  writes nothing anywhere else\. A box with no server configured writes to a local store on that box\n  alone, whose rows reach a server only through an explicit, confirmed adoption \(R-STO-7\)\. Connection\n  string and provider identities live in the OS keyring \(Windows Credential Manager, macOS Keychain,\n  Linux secret service\), never in a file — including when the connection string is typed into the\n  TUI rather than passed to `htui --set-dsn`\. Sentence 1 amended by ANA-10 \(`docs/ANA-10\.md` §6\.1\),\n  2026-09-08; the keyring sentence is unchanged and now binds two entry paths\.",
    r"- **R-STO-1 (must).** Postgres is the only writable store. Connection\n  string and provider identities live in the OS keyring (Windows Credential Manager, macOS Keychain,\n  Linux secret service), never in a file — including when the connection string is typed into the\n  TUI rather than passed to `htui --set-dsn`.",
    content
)

content = re.sub(
    r"- \*\*R-STO-4 \(must\).\*\* When a \*\*configured\*\* Postgres is unreachable, the TUI opens in offline\n  read-only mode from the cache: browse items, graph, documents and cached transcripts\. No item\n  creation, no runs\. A free-standing chat session against a repo \(R-TUI-6\) remains allowed; its\n  events are buffered locally, scrubbed, and persisted on the next successful connection so R-HIS-1\n  still holds\. This requirement covers \"server known, unreachable\" only; \"no server configured\" is\n  R-STO-7\. One word added by ANA-10 \(`docs/ANA-10\.md` §6\.1\), 2026-09-08; the state it describes is\n  otherwise unchanged, and every document that cites it for the offline case still cites it\n  correctly\.",
    r"- **R-STO-4 (must).** When Postgres is unreachable, the TUI opens in offline\n  read-only mode from the cache: browse items, graph, documents and cached transcripts. No item\n  creation, no runs.",
    content
)

content = re.sub(
    r"- \*\*R-STO-5 \(must\).\*\* Schema migrations are versioned, forward-only in version one, applied by\n  `htui` on connect after confirmation\. The per-box SQLite schemas are versioned and forward-only on\n  the same rule but are applied without confirmation when the local file is opened: the read-only\n  cache \(R-STO-3\) answers a version mismatch by rebuilding, and the local store \(R-STO-7\), which\n  holds rows that exist nowhere else, migrates its data forward and is never rebuilt\. Extended by\n  ANA-10 \(`docs/ANA-10\.md` §6\.1\), 2026-09-08: \"on connect after confirmation\" was already false of\n  the cache's own migrations, and the extension makes an existing gap explicit rather than changing\n  the Postgres rule\.",
    r"- **R-STO-5 (must).** Schema migrations are versioned, forward-only in version one, applied by\n  `htui` on connect after confirmation. The per-box SQLite schemas for the read-only cache (R-STO-3)\n  are versioned and forward-only, but answer a version mismatch by rebuilding.",
    content
)

content = re.sub(
    r"- \*\*R-STO-7 \(must\).\*\* A box that has never been given a DSN opens in \*\*local-only\*\* mode against a\n  local store held separately from the read-only cache of R-STO-3\. It is a complete box: it may\n  create the hierarchy, items and free-standing chats, and may run step graphs against its own\n  projects under the full orchestration contract of R-ORCH\. Its rows stay local when a DSN is later\n  configured — they are never uploaded as a side effect of connecting, and a project that is still\n  local is not runnable while a DSN is configured\. Adoption into a server is an explicit, confirmed\n  action, recorded against the server adopted into, and refused for any second or different server\n  without a further explicit confirmation\. Added by ANA-10 \(`docs/ANA-10\.md`,\n  `docs/decisions/ana/ana-10\.md`\), 2026-09-08; implemented as MOD-17, with adoption as MOD-18\.",
    r"- **R-STO-7 (withdrawn).** Local-only mode. Withdrawn by maintainer decision MOD-25, 2026-09-11.",
    content
)

content = re.sub(
    r"- \*\*R-AGT-4 \(must\).\*\* Agent registry in the store that owns the box — Postgres for a box with a\n  server configured, the local store for a box with none \(R-STO-7\): name, transport \(`acp` or\n  `cli`\), launch command, model list, billing mode \(`subscription` or `per_token`\), default model,\n  enabled per box\. Version one entries: `claude` and `agy`\. Amended by ANA-10\n  \(`docs/ANA-10\.md` §6\.1\), 2026-09-08: a box that never connects never reaches the first-connect\n  seed, so its registry has no other home\. Amended 2026-09-09: `agy` over ACP is \*\*settled and\n  proven\*\* — ANA-4 §4\.5 adopted Google's first-party `agy_acp_server` with `transport: 'acp'`, and\n  MOD-2 milestone 6 ran it live on Linux \(`docs/ANA-4\.md` §11 criterion 10\); the clause requiring\n  ANA-4 to settle it is withdrawn\.",
    r"- **R-AGT-4 (must).** Agent registry in Postgres: name, transport (`acp` or\n  `cli`), launch command, model list, billing mode (`subscription` or `per_token`), default model,\n  enabled per box. Version one entries: `claude`, `claude-cli`, and `agy`. Amended 2026-09-09:\n  `agy` over ACP is **settled and proven**.",
    content
)

content = re.sub(
    r"- \*\*R-MCP-2 \(must\).\*\* Tools: `item_link` \(propose an edge, kind\), `item_status` request,\n  `document_write` \(the step's output artifact\), `note_add`, `box_profile` read, `command_run`\.",
    r"- **R-MCP-2 (must).** Tools: `item_link` (propose an edge, kind), `item_status` request,\n  `document_write` (the step's output artifact), `note_add`, `box_profile` read, `command_run`,\n  `spawn_subagent`.",
    content
)

content = re.sub(
    r"- \*\*R-HIS-1 \(must\).\*\* Every session event from R-AGT-1, plus the assembled prompt, every follow-up\n  and every permission answer, is stored as an ordered row per run step, after scrubbing\. Nothing\n  about a run exists only on one box once that box has a server configured\. On a box that has never\n  been given a DSN \(R-STO-7\) the local store is the only copy — of a graph run as much as of a chat\n  — and the TUI says so\. Sentence 2 bounded by ANA-10 \(`docs/ANA-10\.md` §6\.1\), 2026-09-08; sentence 1\n  is unchanged and binds the local store exactly as it binds Postgres\.",
    r"- **R-HIS-1 (must).** Every session event from R-AGT-1, plus the assembled prompt, every follow-up\n  and every permission answer, is stored as an ordered row per run step, after scrubbing. Nothing\n  about a run exists only on one box.",
    content
)

content = re.sub(
    r"- \*\*R-PRM-4 \(must\).\*\* Prompt templates per phase are versioned rows in the store that owns the\n  project, with a documented placeholder contract, editable in the TUI\. Amended by ANA-10\n  \(`docs/ANA-10\.md` §6\.1\), 2026-09-08: a local run resolves its template at prompt assembly, so a\n  local-only project \(R-STO-7\) carries its own rows\.",
    r"- **R-PRM-4 (must).** Prompt templates per phase are versioned rows in Postgres,\n  with a documented placeholder contract, editable in the TUI.",
    content
)

content = re.sub(
    r"- \*\*R-SKL-1 \(must\).\*\* Skill library in the store that owns the project: name, description, versioned\n  markdown body\. Amended by ANA-10 \(`docs/ANA-10\.md` §6\.1\), 2026-09-08, for the same reason as\n  R-PRM-4 — R-PRM-1 puts bound skill text in every step prompt\.",
    r"- **R-SKL-1 (must).** Skill library in Postgres: name, description, versioned markdown body.",
    content
)

content = re.sub(
    r"- \*\*R-TUI-1 \(must\).\*\* Keyboard driven, mouse optional\. Top bar: workspace or project, box, store\n  state — distinguishing online, connecting, offline since T, and local-only \(no server configured\)\n  — active run count\. Tabs: Backlog, Chat \(one per session\), Skills, Settings\. Workspace switcher\n  overlay\. Queue overlay for auto mode with reorder and pause\. \"Postgres state\" replaced by ANA-10\n  \(`docs/ANA-10\.md` §6\.1\), 2026-09-08: three distinct states rendered one string\.",
    r"- **R-TUI-1 (must).** Keyboard driven, mouse optional. Top bar: workspace or project, box, store\n  state — distinguishing online, connecting, and offline since T\n  — active run count. Tabs: Backlog, Chat (one per session), Skills, Settings. Workspace switcher\n  overlay. Queue overlay for auto mode with reorder and pause.",
    content
)

content = re.sub(
    r"- \*\*R-TUI-8 \(must\).\*\* Settings tab: agent registry with quota, box profile with capability edits,\n  item kinds and step graphs per project, secret provider, caps, scheduler window, and the Postgres\n  connection: whether a DSN is stored, a masked field to enter or replace it, an action to clear it,\n  and the state of the last attempt\. What is typed goes to the OS keyring and nowhere else\n  \(R-STO-1\); it is not echoed, not logged, and not written to any file\. Connection section added by\n  ANA-10 \(`docs/ANA-10\.md` §4\.9, §6\.1\), 2026-09-08 — recorded as an \*\*extension\*\* of this\n  requirement rather than a reading of it, since the original list named no connection section and\n  its \"secret provider\" is R-SEC-1/R-SEC-2's agent provider config, from which R-SEC-2 explicitly\n  excludes `htui`'s own credentials\.",
    r"- **R-TUI-8 (must).** Settings tab: agent registry with quota, box profile with capability edits,\n  item kinds and step graphs per project, secret provider, caps, scheduler window, and the Postgres\n  connection: whether a DSN is stored, a masked field to enter or replace it, an action to clear it,\n  and the state of the last attempt. What is typed goes to the OS keyring and nowhere else\n  (R-STO-1); it is not echoed, not logged, and not written to any file.",
    content
)

with open('/home/mluigi/projects/htui/docs/REQUIREMENTS.md', 'w') as f:
    f.write(content)
