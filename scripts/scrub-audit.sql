-- MOD-10 M1 T0: how often would each candidate scrub rule fire on rows already stored?
--
-- Read-only. Prints COUNTS ONLY: no matched string, key, pointer or row id ever leaves the
-- database. Run it against the database htui actually uses, e.g. on the host:
--
--   docker exec -i htui-postgres psql -U postgres -d htui -f - < scripts/scrub-audit.sql
--
-- What it walks, matching the scrubber (crates/htui-core/src/scrub.rs): every string value AND
-- every object key, at any depth, of session_event.payload, session_event.raw and
-- run_step.trim_record. Matching `payload::text` instead would undercount: a JSON escape such as
-- `\n` puts a letter right before the token, so the token-start anchor would miss it.
--
-- Rules are the plan's D2 table (.claude/plans/mod-10-m1-scrubber-hardening.plan.md), each
-- anchored at a token start as in D1. Plain '...' literals only (standard_conforming_strings);
-- an E'...' literal would need `\\.`.
--
-- Rows already stored passed today's bare-prefix rules, so a hit here is either a new rule's
-- false positive or a credential the old rules missed. Both are worth knowing before the list is
-- fixed. The last query counts the refusals the current rules already made, by rule.

\set ON_ERROR_STOP on
-- Not `READ ONLY`: Postgres refuses CREATE (even of a temp view) in a read-only transaction. The
-- views only SELECT, and the closing ROLLBACK discards them.
BEGIN;

CREATE TEMP VIEW scrub_audit_rules (rule, re) AS VALUES
    ('anthropic_api_key',  'sk-ant-[A-Za-z0-9_-]{20,}'),
    ('openai_api_key',     'sk-(?:(?:proj|svcacct|admin)-[A-Za-z0-9_-]{20,}|[A-Za-z0-9]{20,})'),
    ('github_pat',         'github_pat_[A-Za-z0-9_]{20,}'),
    ('github_token',       'gh[pousr]_[A-Za-z0-9]{30,}'),
    ('gitlab_pat',         'glpat-[A-Za-z0-9_-]{20,}'),
    ('aws_access_key_id',  '(?:AKIA|ASIA|ABIA|ACCA)[A-Z0-9]{16}'),
    ('slack_bot_token',    'xoxb-[A-Za-z0-9-]{10,}'),
    ('slack_user_token',   'xoxp-[A-Za-z0-9-]{10,}'),
    ('slack_token',        'xox[ars]-[A-Za-z0-9-]{10,}'),
    ('google_api_key',     'AIza[0-9A-Za-z_-]{35}'),
    ('stripe_secret_key',  '[rs]k_(?:live|test)_[A-Za-z0-9]{20,}'),
    ('npm_token',          'npm_[A-Za-z0-9]{36}'),
    ('pypi_token',         'pypi-AgEIcHlwaS5vcmc[A-Za-z0-9_-]{50,}'),
    ('sendgrid_api_key',   'SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}'),
    ('jwt',                'eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}');

-- Every document to audit, one row each: (source column, opaque row key, document).
CREATE TEMP VIEW scrub_audit_docs (src, row_key, doc) AS
    SELECT 'session_event.payload', run_step_id::text || '/' || seq, payload
      FROM session_event
    UNION ALL
    SELECT 'session_event.raw', run_step_id::text || '/' || seq, raw
      FROM session_event WHERE raw IS NOT NULL
    UNION ALL
    SELECT 'run_step.trim_record', id::text, trim_record
      FROM run_step WHERE trim_record IS NOT NULL;

-- Every string value and every object key in those documents. `strict $.**` yields every level,
-- the root included.
CREATE TEMP VIEW scrub_audit_strings (src, row_key, s) AS
    SELECT d.src, d.row_key, v #>> '{}'
      FROM scrub_audit_docs d
      CROSS JOIN LATERAL jsonb_path_query(d.doc, 'strict $.**') AS v
     WHERE jsonb_typeof(v) = 'string'
    UNION ALL
    SELECT d.src, d.row_key, k
      FROM scrub_audit_docs d
      CROSS JOIN LATERAL jsonb_path_query(d.doc, 'strict $.**') AS o
      CROSS JOIN LATERAL jsonb_object_keys(o) AS k
     WHERE jsonb_typeof(o) = 'object';

\echo '== 1. rows scanned per source'
SELECT src, count(*) AS rows FROM scrub_audit_docs GROUP BY src ORDER BY src;

\echo '== 2. candidate rule hits (rows = distinct rows with >= 1 hit; strings = matching values/keys)'
SELECT r.rule, s.src,
       count(DISTINCT s.row_key) AS rows,
       count(*)                  AS strings
  FROM scrub_audit_rules r
  JOIN scrub_audit_strings s ON s.s ~ ('(^|[^A-Za-z0-9_])' || r.re)
 GROUP BY r.rule, s.src
 ORDER BY r.rule, s.src;

\echo '== 3. rules with no hit at all'
SELECT r.rule
  FROM scrub_audit_rules r
 WHERE NOT EXISTS (
       SELECT 1 FROM scrub_audit_strings s WHERE s.s ~ ('(^|[^A-Za-z0-9_])' || r.re))
 ORDER BY r.rule;

\echo '== 4. refusals the current rules already made (scrub_residue rows, by rule)'
SELECT split_part(payload ->> 'message', ' at ', 1) AS rule, count(*) AS rows
  FROM session_event
 WHERE kind = 'error' AND payload ->> 'code' = 'scrub_residue'
 GROUP BY 1
 ORDER BY 2 DESC, 1;

ROLLBACK;
