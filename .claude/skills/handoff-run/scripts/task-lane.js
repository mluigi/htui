export const meta = {
  name: 'handoff-task-lane',
  description: 'Run plan tasks serially on one branch: one TDD implementer per task from a task brief; a single verifier only where the task asks for one',
  phases: [
    { title: 'Implement', detail: 'one implementer per task, brief-only context, crate-scoped tests' },
    { title: 'Verify', detail: 'one read-only verifier, only for tasks marked verify' },
    { title: 'Repair', detail: 'one repair of medium/high findings' },
  ],
}

// Run with Workflow({ scriptPath: '.claude/skills/handoff-run/scripts/task-lane.js', args }).
// args: {
//   item:   'MOD-150 M1'            commit-subject tag
//   dir:    '/abs/checkout'         the checkout this lane owns (primary or a worktree)
//   branch: 'hr/MOD-150'
//   env:    'hr sandbox: ...'       one or two lines of environment facts (database, sandbox, no push)
//   notes:  '...'                   repo gotchas that apply to this item (from memory), a few lines
//   models: { implementer: 'sonnet', verifier: null }   optional; a missing or null stage inherits the session model
//   tasks: [{
//     key:    'T1', title: '...',
//     brief:  '.claude/plans/<name>.briefs/T1.md'   the ONLY planning text the agents read
//     files:  'crates/x/src/a.rs, ...'              files the task owns
//     test:   'cargo test -p htui-store --all-features --no-fail-fast -- --test-threads=1'
//     verify: false | 'foundation' | 'fix-round'    default false: no verifier
//     attack: '...'                                 optional: what the verifier should try to break
//     skipImpl: 'reason'                            optional: work already committed, go to verify
//     resume: true                                  optional: a previous implementer was cut off
//   }]
// }
// The full gate is NOT run by any agent here: the main thread runs it once per lane.
const A = args
const M = A.models || {}
// Spread into agent() options: no `model` key at all when the stage inherits.
const on = stage => (M[stage] && M[stage] !== 'inherit' ? { model: M[stage] } : {})

const common = t => `
Repo: htui (Rust workspace). ${A.env || ''} Work ONLY in \`${A.dir}\` (cd there first; branch \`${A.branch}\`).
Your task is **${t.key}: ${t.title}**. Its brief is \`${t.brief}\`: read it once, in full. It is the complete
specification for this task; do not open the plan, the PRD or any blueprint unless the brief names a section to read.
Files this task owns (touch nothing else; if you must, say so in the report): ${t.files}.
Reading code: Gortex MCP tools when working in the primary checkout (ToolSearch
"select:mcp__gortex__read,mcp__gortex__search,mcp__gortex__relations"); read one symbol or a line window, never a whole
large file. A Gortex hook may block Read/Grep on indexed source there; then use \`git grep -n\` and
\`git show HEAD:<path> | sed -n 'A,Bp'\`, and edit with an anchored scripted replace that asserts one match. A worktree
is not indexed: use native Read/Grep/Edit there, and never a Gortex edit tool.
${A.notes || ''}`

const git = t => `
Git: commit incrementally, staging explicit paths only. Subjects use the conventional prefix and contain
\`${A.item} ${t.key}\`; the body ends with the line "Co-Authored-By: <the model you are running as> <noreply@anthropic.com>" (e.g. Claude Opus 5.5).
FORBIDDEN: git add -A / git add . (repo-wide), git stash, commit --amend, rebase, reset --hard, other branches,
WORKFLOW_SKIP_HOOK, --no-verify, pushing. Never kill cargo processes you did not start; never wait with a
\`until ! pgrep ...\` loop.
Tests: run ONLY this task's check, not the workspace gate (the main thread runs the gate once for the lane):
  ${t.test}
plus clippy and rustfmt on the crates you touched. \`--no-fail-fast\` goes BEFORE the \`--\`.
Keep every single command under 4 minutes of wall time: a longer one must run in the background and be checked at
intervals under 4 minutes, because an agent idle for 5 minutes pays to reload its whole context.
Report every commit hash. Keep the report short: what changed, what is owed, nothing the diff already says.`

const REPORT = {
  type: 'object',
  properties: {
    commits: { type: 'array', items: { type: 'string' } },
    checks_passed: { type: 'boolean' },
    summary: { type: 'string' },
    deviations: { type: 'array', items: { type: 'string' } },
    owed_on_host: { type: 'array', items: { type: 'string' } },
    rejected_findings: { type: 'array', items: { type: 'string' } },
  },
  required: ['commits', 'checks_passed', 'summary', 'deviations'],
}

const FINDINGS = {
  type: 'object',
  properties: {
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          severity: { type: 'string', enum: ['high', 'medium', 'low'] },
          file: { type: 'string' },
          line: { type: 'number' },
          claim: { type: 'string' },
          evidence: { type: 'string' },
          fix: { type: 'string' },
        },
        required: ['severity', 'file', 'claim', 'evidence'],
      },
    },
    verdict: { type: 'string' },
  },
  required: ['findings', 'verdict'],
}

const verify = (t, range, round) => agent(
  `${common(t)}
You are the VERIFIER for ${t.key}. STRICTLY no edits, no commits, no file writes inside the repo (scratch only under
/tmp). Review exactly this range and nothing else: \`git -C ${A.dir} diff ${range}\` (write it to a /tmp file once and
read that; do not re-run the diff).
Check two things. Conformance: does the change do what the brief requires, are the tests asserting the behaviour they
are named for, was anything changed that the brief did not ask for? Breakage: try to break it. ${t.attack || ''}
${t.verify === 'fix-round' ? 'This is a fix round: look hardest for behaviour the fix regressed next to the lines it touched.' : ''}
Run only the tests that cover the touched code. Report a defect only with evidence (file:line, a failing command, a
quoted brief line); default to no finding when uncertain. Style, naming and wording points are not findings.`,
  { label: `verify:${t.key}:r${round}`, phase: 'Verify', schema: FINDINGS, ...on('verifier') },
)

const results = []

for (const t of A.tasks) {
  phase('Implement')
  const impl = t.skipImpl
    ? { commits: [], checks_passed: true, summary: 'implement skipped: ' + t.skipImpl, deviations: [] }
    : await agent(
      `${common(t)}
You are the IMPLEMENTER for ${t.key}. Implement the brief test-first: write the tests, watch them fail, then implement.
${t.resume ? 'RESUME: a previous implementer was cut off. Read `git -C ' + A.dir + ' log --oneline -15` and `git -C ' + A.dir + ' status`, then continue; do not restart.' : ''}
Before you start, note the current commit (\`git -C ${A.dir} rev-parse HEAD\`) and return it as the first entry of
"deviations" in the form "base=<hash>".
${git(t)}`,
      { label: `implement:${t.key}`, phase: 'Implement', schema: REPORT, ...on('implementer') },
    )
  if (!impl) {
    log(`${t.key}: implementer died; lane stopped here, resume with tasks from ${t.key} and resume:true`)
    results.push({ key: t.key, impl: null, stopped: true })
    break
  }
  log(`${t.key} implement: checks_passed=${impl.checks_passed} commits=${impl.commits.length}`)
  const out = { key: t.key, impl, verify: 'not requested', remaining: [], low: [] }

  if (t.verify) {
    const baseNote = (impl.deviations.find(d => d.startsWith('base=')) || '').slice(5)
    const base = t.base || baseNote
    if (!base) {
      log(`${t.key}: no base commit known; verifier skipped, main thread reviews this task's diff`)
      out.verify = 'skipped: no base'
    } else {
      const v1 = await verify(t, `${base}..HEAD`, 1)
      if (!v1) {
        log(`${t.key}: verifier died; result is UNVERIFIED`)
        out.verify = 'UNVERIFIED: verifier died'
      } else {
        const act = v1.findings.filter(f => f.severity !== 'low')
        out.low = v1.findings.filter(f => f.severity === 'low')
        out.verify = v1.verdict
        if (act.length) {
          phase('Repair')
          const rep = await agent(
            `${common(t)}
You are the REPAIR agent for ${t.key}. For EACH finding below: first confirm it against the code (verifiers can be
wrong); fix only what you confirm, test-first where a test can pin it; list what you reject, with the reason, in
"rejected_findings". Before you start, note the current commit and return it as the first entry of "deviations" in the
form "base=<hash>".
FINDINGS:
${JSON.stringify(act, null, 2)}
${git(t)}`,
            { label: `repair:${t.key}`, phase: 'Repair', schema: REPORT, ...on('implementer') },
          )
          out.repair = rep
          const repBase = rep ? (rep.deviations.find(d => d.startsWith('base=')) || '').slice(5) : ''
          // One re-check, of the repair's own diff only, and only when a high finding was repaired.
          if (rep && rep.commits.length && repBase && act.some(f => f.severity === 'high')) {
            const v2 = await verify({ ...t, verify: 'fix-round' }, `${repBase}..HEAD`, 2)
            out.remaining = v2 ? v2.findings.filter(f => f.severity !== 'low') : []
            if (!v2) out.verify = 'UNVERIFIED: re-check died'
          } else if (!rep) {
            out.remaining = act
          }
        }
      }
    }
  }
  if (out.remaining.length) log(`${t.key}: ${out.remaining.length} finding(s) remain; main thread decides`)
  results.push(out)
  if (!impl.checks_passed) {
    log(`${t.key}: checks did not pass; lane stopped so later tasks do not build on it`)
    break
  }
}

return { item: A.item, results }
