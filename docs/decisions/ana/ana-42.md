# ANA-42 - Execution targets: where an item runs (concluded, 2026-11-14)

Full analysis: `docs/ANA-42.md` (surface as read, prior art, options, verdict, risks). Origin: the UI
redesign review, `docs/ui-redesign-findings-2026-10-15.md` §4.2a. Addresses `R-ORCH-15`, `R-ORCH-12`,
`R-ENT-17`, `R-SEC-5`, `R-AGT-11`, `R-BOX-4`, `R-NF-2`; touches `R-SEC-2..4`, `R-AGT-5`, `R-AGT-9`,
`R-ID-2` and `R-BOX-1..3` at their seams.

## Verdict

Drafted 2026-10-17. A **target is a record that points at a box and says how to reach it** (`kind`:
`local | docker | ssh | ssh_docker | enrolled`, a JSONB `locator`, `box_id`, a derived `status`). The
engine, store writer, lease, recorder and MCP host **stay on this box** for every kind except an enrolled
box; only the agent process is remote, driven over stdio by a `TargetTransport` launch decorator under
`TransportBuilder`. The relay rides a forwarded socket with the token as a stdin preamble; secrets are
resolved here and injected into the exec's environment; the branch comes back as a **git bundle** through
the same channel, fetched under `scripts/hr collect`'s discipline. The default target cascades Settings →
workspace → project → workflow → item, each level able to inherit, with the resolver naming its source.

## Maintainer decisions (2026-11-14)

All six §8 decisions accepted as recommended:

1. Target = record pointing at a box (7.1), not target = box.
2. Executor on this box for the driven kinds: `target_box_id` is where the work runs, `executing_box_id`
   who drives it; the queue's `Elsewhere` keys on `executing_box_id`. MOD-43's "a non-local target stays
   `queued`" applies to the enrolled kind only.
3. Bundle collect is the single mechanism, ssh targets included.
4. `R-NF-2` amended: `dockerd` and `sshd` are opt-in per target, gated by the `docker` capability tag
   and a successful target probe; merged into ANA-37's replacement text (written by MOD-130). Closes
   MOD-44's open `R-NF-2` question too.
5. Phasing: this box and docker here (MOD-130, MOD-132 on MOD-44), then ssh and remote docker (MOD-131),
   then enrolled (MOD-47).
6. Secrets by injection: no `R-SEC-2` amendment; server-side resolution stays with MOD-47/48.

Folded in: ANA-34 Q9 (after collect the coordinator integrates per the repo's mode, MOD-133; "merging
stays the person's step" is the `local` mode) and ANA-35 §7.2 (a plain folder runs on this box only; a
repo-less project has nothing to clone, MOD-145).

## Spawned

- **MOD-130** - target record, child boxes, target cascade, `executing_box_id` in the queue, the
  `R-NF-2` amendment.
- **MOD-131** - driven targets: ssh and docker exec launch, relay tunnel, remote bootstrap (blocked on
  MOD-130, MOD-44).
- **MOD-132** - target preparation and branch collection by bundle (blocked on MOD-130).

No commits: analysis only (`docs/ANA-42.md`, drafted 2026-10-17, concluded 2026-11-14).
