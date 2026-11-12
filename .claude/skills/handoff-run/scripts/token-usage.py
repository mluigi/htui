#!/usr/bin/env python3
"""Token usage of /handoff-run work, read from Claude Code's session transcripts.

  python3 .claude/skills/handoff-run/scripts/token-usage.py                  # last 14 days, by item
  python3 .claude/skills/handoff-run/scripts/token-usage.py --item MOD-150   # one item, by stage and task
  python3 .claude/skills/handoff-run/scripts/token-usage.py --by stage       # item | stage | task | model
  python3 .claude/skills/handoff-run/scripts/token-usage.py --item MOD-150 --tsv

An item is the session's git branch (hr/<ITEM>), else the argument of its first /handoff-run command.
A stage and task come from the agent's label (implement:T1, verify:T1:r1, repair:T1) or its agent type
(code-architect, the reviewer). The main session is stage "main".

COST is input-equivalent tokens: input x1, cache write x1.25, cache read x0.1, output x5. It ignores
the price difference between models; pass --weight MODEL=FACTOR (repeatable) to apply one, e.g. a
cheaper model's price relative to the session model. RELOADS counts turns that rebuilt most of the
context after a pause of more than five minutes (an expired prompt cache).
"""
import argparse
import collections
import datetime
import glob
import json
import os
import re
import subprocess
import sys

W_IN, W_CW, W_CR, W_OUT = 1.0, 1.25, 0.1, 5.0
STAGES = {
    'implement': 'implementer', 'impl': 'implementer', 'verify': 'verifier', 'reverify': 'verifier',
    'repair': 'repair', 'code-architect': 'architect', 'check': 'factcheck',
}


def reviewer_name():
    try:
        return json.load(open(os.path.join('.claude', 'workflow-config.json'))).get('reviewer')
    except (OSError, ValueError):
        return None


def project_dir():
    try:
        common = subprocess.run(['git', 'rev-parse', '--path-format=absolute', '--git-common-dir'],
                                capture_output=True, text=True, check=True).stdout.strip()
        top = os.path.dirname(common)
    except Exception:
        top = os.getcwd()
    base = os.environ.get('CLAUDE_CONFIG_DIR') or os.path.join(os.path.expanduser('~'), '.claude')
    return os.path.join(base, 'projects', re.sub(r'[^A-Za-z0-9]', '-', top))


def ts(s):
    return datetime.datetime.fromisoformat(s.replace('Z', '+00:00')).timestamp()


def scan(path):
    """One transcript -> (usage counter, model, item hint, verifier findings by severity)."""
    seen, order = {}, []
    branch = cmd = None
    findings = collections.Counter()
    with open(path, errors='replace') as fh:
        for line in fh:
            try:
                d = json.loads(line)
            except ValueError:
                continue
            b = d.get('gitBranch')
            if b and b.startswith('hr/'):
                branch = b[3:]
            m = d.get('message') or {}
            if cmd is None and d.get('type') == 'user':
                c = m.get('content')
                text = c if isinstance(c, str) else ' '.join(
                    x.get('text', '') for x in c or [] if isinstance(x, dict))
                hit = re.search(r'<command-name>/handoff-run</command-name>.*?<command-args>\s*([A-Z]+-\d+)',
                                text, re.S) or re.search(r'<command-args>\s*([A-Z]+-\d+)', text)
                if hit and 'handoff-run' in text:
                    cmd = hit.group(1)
            if d.get('type') != 'assistant':
                continue
            u = m.get('usage')
            if u:
                k = m.get('id') or d.get('uuid')
                if k not in seen:
                    order.append(k)
                seen[k] = (ts(d['timestamp']) if d.get('timestamp') else None, u, m.get('model') or '?')
            for c in m.get('content') or []:
                if isinstance(c, dict) and c.get('type') == 'tool_use' and c.get('name') == 'StructuredOutput':
                    fl = (c.get('input') or {}).get('findings')
                    if isinstance(fl, list):
                        findings.clear()
                        for f in fl:
                            if isinstance(f, dict):
                                findings[str(f.get('severity', '?')).lower()] += 1
    t = collections.Counter()
    models = collections.Counter()
    prev = None
    for i, k in enumerate(order):
        when, u, model = seen[k]
        cw = u.get('cache_creation_input_tokens') or 0
        cr = u.get('cache_read_input_tokens') or 0
        t['in'] += u.get('input_tokens') or 0
        t['cw'] += cw
        t['cr'] += cr
        t['out'] += u.get('output_tokens') or 0
        t['turns'] += 1
        t['peak'] = max(t['peak'], (u.get('input_tokens') or 0) + cw + cr)
        if i and cw > 20000 and cw > cr and when and prev and when - prev > 300:
            t['reloads'] += 1
        prev = when or prev
        models[model] += 1
    model = models.most_common(1)[0][0] if models else '?'
    return t, model, branch or cmd, findings


def label_of(path):
    meta = path[:-6] + '.meta.json'
    desc, kind = '', ''
    if os.path.exists(meta):
        try:
            j = json.load(open(meta))
            desc, kind = j.get('description') or '', j.get('agentType') or ''
        except ValueError:
            pass
    if '/workflows/' in path.replace(os.sep, '/') and ':' in desc:
        parts = desc.split(':')
        return STAGES.get(parts[0].strip().lower(), parts[0].strip().lower()), parts[1].strip()
    if kind == 'workflow-subagent':
        return desc.split(' ')[0].lower() or 'workflow', '-'
    if kind and kind == REVIEWER:
        return 'reviewer', '-'
    return STAGES.get(kind, kind or 'agent'), '-'


REVIEWER = reviewer_name()


def cost(t, factor=1.0):
    return factor * (W_IN * t['in'] + W_CW * t['cw'] + W_CR * t['cr'] + W_OUT * t['out'])


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--item', help='only this item, e.g. MOD-150 (default grouping becomes stage,task)')
    ap.add_argument('--days', type=float, default=14, help='sessions touched in the last N days (default 14)')
    ap.add_argument('--by', help='comma list of item,stage,task,model (default: item; with --item: stage,task,model)')
    ap.add_argument('--weight', action='append', default=[], metavar='MODEL=FACTOR',
                    help='price factor for a model id or a substring of it, e.g. sonnet=0.6')
    ap.add_argument('--project-dir', default=None, help='transcript directory (default: this repo\'s)')
    ap.add_argument('--tsv', action='store_true')
    a = ap.parse_args()

    root = a.project_dir or project_dir()
    if not os.path.isdir(root):
        sys.exit(f'token-usage: no transcripts at {root} (pass --project-dir)')
    weights = []
    for w in a.weight:
        name, _, val = w.partition('=')
        try:
            weights.append((name, float(val)))
        except ValueError:
            sys.exit(f'token-usage: --weight wants MODEL=FACTOR, got {w!r}')
    by = [x.strip() for x in (a.by or ('stage,task,model' if a.item else 'item')).split(',')]
    bad = [x for x in by if x not in ('item', 'stage', 'task', 'model')]
    if bad:
        sys.exit(f'token-usage: unknown --by field {bad[0]!r}')

    cut = datetime.datetime.now().timestamp() - a.days * 86400
    rows = collections.defaultdict(collections.Counter)
    for main_path in glob.glob(os.path.join(root, '*.jsonl')):
        sid = os.path.basename(main_path)[:-6]
        subs = glob.glob(os.path.join(root, sid, 'subagents', '**', '*.jsonl'), recursive=True)
        if max(os.path.getmtime(p) for p in [main_path] + subs) < cut:
            continue
        t, model, item, _ = scan(main_path)
        item = item or '-'
        if a.item and item != a.item:
            continue
        entries = [(('main', '-'), t, model, collections.Counter())]
        for sp in subs:
            st, smodel, _, fnd = scan(sp)
            entries.append((label_of(sp), st, smodel, fnd))
        for (stage, task), st, smodel, fnd in entries:
            if not st['turns']:
                continue
            factor = next((f for n, f in weights if n in smodel), 1.0)
            key = tuple({'item': item, 'stage': stage, 'task': task, 'model': smodel}[b] for b in by)
            r = rows[key]
            for k in ('in', 'cw', 'cr', 'out', 'turns', 'reloads'):
                r[k] += st[k]
            r['peak'] = max(r['peak'], st['peak'])
            r['agents'] += 1
            r['cost'] += cost(st, factor)
            if stage == 'verifier':
                for sev, n in fnd.items():
                    r['f_' + sev] += n

    if not rows:
        sys.exit('token-usage: nothing matched')
    total = sum(r['cost'] for r in rows.values())
    head = [b.upper() for b in by] + ['AGENTS', 'TURNS', 'COST_M', 'SHARE', 'OUT_K', 'CACHE_WR_M', 'CACHE_RD_M',
                                     'PEAK_CTX_K', 'RELOADS', 'FINDINGS_H/M/L']
    table = []
    for key, r in sorted(rows.items(), key=lambda kv: -kv[1]['cost']):
        table.append(list(key) + [
            str(r['agents']), str(r['turns']), '%.2f' % (r['cost'] / 1e6), '%.0f%%' % (100 * r['cost'] / total),
            '%.0f' % (r['out'] / 1e3), '%.2f' % (r['cw'] / 1e6), '%.1f' % (r['cr'] / 1e6),
            '%.0f' % (r['peak'] / 1e3), str(r['reloads']),
            '%d/%d/%d' % (r['f_high'], r['f_medium'], r['f_low']) if any(k.startswith('f_') for k in r) else '-',
        ])
    table.append(['TOTAL'] + [''] * (len(by) - 1) + [
        str(sum(r['agents'] for r in rows.values())), str(sum(r['turns'] for r in rows.values())),
        '%.2f' % (total / 1e6), '100%', '', '', '', '', str(sum(r['reloads'] for r in rows.values())), ''])
    if a.tsv:
        for line in [head] + table:
            print('\t'.join(line))
        return
    widths = [max(len(x[i]) for x in [head] + table) for i in range(len(head))]
    for line in [head] + table:
        print('  '.join(c.ljust(w) if i < len(by) else c.rjust(w) for i, (c, w) in enumerate(zip(line, widths))))


if __name__ == '__main__':
    main()
