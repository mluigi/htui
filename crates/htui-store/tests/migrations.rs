//! ANA-9 §11 criterion 1: the schema applies on a clean Postgres, a second run is a no-op, and a
//! database whose schema this binary does not know is refused rather than repaired (`R-STO-5`).
//!
//! Every case creates and drops its own database; with `HTUI_TEST_DATABASE_URL` unset each one
//! prints `common::SKIP` and passes, so the suite is green on a box without a server (plan D13).
//! `box_toml_mint_is_stable_across_two_reads` needs no server and therefore never skips.

use htui_store::testkit as common;

use std::collections::BTreeSet;

use htui_core::model::{BoxId, OsFamily};
use htui_core::store::StoreError;
use htui_store::pg::PoolSize;
use htui_store::{
    HTUI_VERSION, HeadlessError, MIGRATOR, MigrationState, PgStore, Registration,
    TARGET_VERSION_KEY, identity,
};
use serde_json::json;
use sqlx::Row as _;

/// The `connect_timeout` every headless connect here passes.
const HEADLESS_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The 42 tables, in creation order: blueprint B.1's 32, then the one `0003_orchestration.sql`
/// adds (`run_step_tree`, ANA-2 §9), then the six of `0006_requirements.sql` (ANA-11 §5), then
/// the two of `0011_permission_relay.sql` (MOD-42 plan D1), then the one of `0012_persona.sql`
/// (MOD-26 plan D1).
const TABLES: &[&str] = &[
    "app_user",
    "capability_tag",
    "box",
    "box_tool",
    "agent",
    "agent_box",
    "workspace",
    "project",
    "workspace_project",
    "workspace_box_path",
    "repo",
    "repo_box_path",
    "step_graph",
    "step_graph_phase",
    "phase_agent",
    "prompt_template",
    "item_kind",
    "item_key_counter",
    "item",
    "item_revision",
    "item_link",
    "item_note",
    "document",
    "skill",
    "skill_version",
    "skill_binding",
    "run",
    "run_step",
    "run_step_commit",
    "session_event",
    "command_run",
    "app_setting",
    // 0003_orchestration.sql (ANA-2 §9), and therefore last.
    "run_step_tree",
    // 0006_requirements.sql (ANA-11 §5, MOD-38), in its creation order.
    "requirement_spec",
    "requirement_area",
    "requirement_key_counter",
    "requirement",
    "requirement_revision",
    "item_requirement",
    // 0011_permission_relay.sql (MOD-42)
    "step_permission",
    "run_command",
    // 0012_persona.sql (MOD-26)
    "persona",
];

#[tokio::test]
async fn migrations_apply_on_a_clean_database() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY 1")
        .fetch_all(&db.pool)
        .await
        .expect("read _sqlx_migrations");
    let embedded: Vec<i64> = MIGRATOR
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .map(|m| m.version)
        .collect();
    assert_eq!(applied, embedded, "every embedded migration is applied");
    assert_eq!(
        applied,
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        "0001_init.sql, MOD-2 milestone 5's 0002_agent_probe.sql, MOD-4 milestone 1's \
         0003_orchestration.sql, MOD-4 milestone 4's 0004_max_agents_per_run_default.sql, \
         MOD-7 milestone 1's 0005_box_identity.sql, MOD-38's 0006_requirements.sql, MOD-9 \
         milestone 2's 0007_skill_attachments.sql, MOD-9 milestone 5's 0008_trim_record_v3.sql, \
         MOD-23's 0009_agent_box_user_off.sql, MOD-33's 0010_prompt_digest_undigested.sql, \
         MOD-42's 0011_permission_relay.sql and MOD-26's 0012_persona.sql, in ordinal order"
    );

    let present: BTreeSet<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE'",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read information_schema.tables")
    .into_iter()
    .collect();

    for table in TABLES {
        assert!(present.contains(*table), "table `{table}` was created");
    }
    assert_eq!(
        TABLES.len(),
        42,
        "blueprint B.1 lists 32 tables (ANA-9 §3's prose count of 30 is wrong, H.1), \
         0003_orchestration.sql adds run_step_tree, 0006_requirements.sql adds ANA-11 §5's six, \
         MOD-42's 0011_permission_relay.sql adds step_permission and run_command and MOD-26's \
         0012_persona.sql adds persona"
    );
    // `_sqlx_migrations` is the only extra table sqlx adds.
    assert_eq!(
        present.len(),
        TABLES.len() + 1,
        "the migrations create the 42 tables of B.1 as amended by ANA-2 §9, ANA-11 §5, \
         MOD-42's 0011_permission_relay.sql and MOD-26's 0012_persona.sql and nothing else, \
         got {present:?}"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn a_second_run_is_a_no_op() {
    let Some(mut db) = common::fresh_db().await else {
        return;
    };

    let before: i64 = common::count(&db.pool, "_sqlx_migrations").await;
    let settings_before: i64 = common::count(&db.pool, "app_setting").await;
    db.store
        .apply_migrations()
        .await
        .expect("a second apply_migrations succeeds");
    let after: i64 = common::count(&db.pool, "_sqlx_migrations").await;
    let settings_after: i64 = common::count(&db.pool, "app_setting").await;

    assert_eq!(before, after, "re-running the migrator applies nothing new");
    assert_eq!(
        settings_before, settings_after,
        "0002's `ON CONFLICT (key) DO NOTHING` keeps a re-run row-neutral"
    );
    db.drop_db().await;
}

/// MOD-2 milestone 5 (plan D43), section 1 of `0002_agent_probe.sql`: the ANA-4 §4.6 snapshot
/// column. Nullable, because a box that has never probed a row has nothing to say about it.
#[tokio::test]
async fn agent_box_gains_a_jsonb_probe_column() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let column = sqlx::query(
        "SELECT data_type::text, is_nullable::text FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'agent_box' AND column_name = 'probe'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("agent_box.probe exists after 0002");

    let data_type: String = column.get("data_type");
    let is_nullable: String = column.get("is_nullable");
    assert_eq!(data_type, "jsonb", "the snapshot is a JSONB document (D44)");
    assert_eq!(
        is_nullable, "YES",
        "a row that has never been probed carries NULL, not an empty document"
    );

    db.drop_db().await;
}

/// The twenty-three `COMMENT ON COLUMN` texts of `0002_agent_probe.sql` and
/// `0003_orchestration.sql`, verbatim.
///
/// `agent.name` is ANA-4 §9 as amended by plan D43 (its `COMMENT ... IS NULL` would have cleared a
/// comment `0001_init.sql` never wrote); the next three are ANA-5 §9 copied from
/// `docs/ANA-5.md:2153-2181`; the last nineteen are ANA-2 §9 (`docs/ANA-2.md:1862-1999`).
/// `run_step.prompt_digest` and `run_step.trim_record` are restated by
/// `0010_prompt_digest_undigested.sql` (MOD-33 D273) and are pinned in [`MOD33_COLUMN_COMMENTS`].
/// They live here as literals on purpose: this test is the guard against a
/// paraphrase drifting into a forward-only migration that cannot be edited afterwards.
const ANA_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[
    (
        "agent",
        "name",
        "unique registry name, e.g. claude or agy. Rows are seeded by htui, not by 0001_init.sql: \
         htui_core::model::agent::seed_rows (crates/htui-core/seeds/*.json) inserted by \
         PgStore::seed_if_empty_as when the table is empty. The inline comment in 0001_init.sql \
         predates that and is stale (ANA-4 9 as amended by MOD-2 plan D43).",
    ),
    (
        "prompt_template",
        "body",
        "ANA-5 4.1: {{name}} placeholders over a closed per-role set; {{{{ escapes a literal {{; \
         no conditionals and no loops, because a section whose data is absent renders empty. Role \
         is derived from name: judge and handoff are reserved, everything else is a phase \
         template.",
    ),
    (
        "prompt_template",
        "name",
        "phase name, or one of the reserved names judge and handoff (ANA-5 4.6); defaults are \
         copied into each new project by the ANA-9 5.10 seed as amended by ANA-5",
    ),
    (
        "step_graph_phase",
        "token_budget",
        "ANA-5 4.4: phase, then project.settings.token_budget, then app_setting.token_budget; the \
         assembler targets budget * (1 - app_setting.prompt_reserve_fraction)",
    ),
    // ---------------------------------------------------------------------------------------
    // 0003_orchestration.sql (ANA-2 §9), section by section.
    // ---------------------------------------------------------------------------------------
    (
        "step_graph",
        "is_override",
        "true = an <item.key>-override clone (R-ORCH-1, ANA-2 §4.1); hidden from the R-TUI-8 \
         graph list",
    ),
    (
        "step_graph_phase",
        "judge_agent_id",
        "R-ORCH-7 judge; NULL = human selection is required whenever fan_out > 1 (ANA-2 §4.5)",
    ),
    (
        "step_graph_phase",
        "judge_model",
        "model for the judge; overrides agent.default_model (ANA-2 §7)",
    ),
    (
        "step_graph_phase",
        "deadline_seconds",
        "wall clock for one step attempt; NULL = project.settings.step_deadline_seconds \
         (ANA-2 §4.1)",
    ),
    (
        "step_graph_phase",
        "verify_command",
        "ANA-2 §4.2: runs after the session and before the gate, in the primary repo tree, \
         through command_run when the phase advertises it; outcome lands in \
         run_step.verify_outcome",
    ),
    (
        "step_graph_phase",
        "input_kinds",
        "ANA-2 §4.2: each kind resolves to the latest document version on this item whose \
         producing step is not a fan-out loser, preferring this run's own output; a missing kind \
         fails the step",
    ),
    (
        "run",
        "repo_scope",
        "repos this run may touch, resolved at queue time from item.touched_paths (ANA-2 §4.7)",
    ),
    (
        "run",
        "lease_owner",
        "per-process id of the orchestrator holding this run; a zero-row lease refresh means \
         abandon (ANA-2 §4.9)",
    ),
    (
        "run",
        "graph_snapshot",
        "ANA-2 §5.1: {v, graph, topology, mode, phases[], settings}; carries both gate and \
         gate_effective so the R-ORCH-6 downgrade is auditable",
    ),
    (
        "run_step",
        "verify_outcome",
        "ANA-2 §4.2: pass | fail | unavailable; unavailable never fails a step",
    ),
    (
        "run_step",
        "exit_code",
        "the agent process exit code (R-ORCH-11); verification has its own verify_exit_code",
    ),
    (
        "run_step",
        "fanout_index",
        "0..fan_out-1; -1 = the R-ORCH-7 judge step for this position and attempt (ANA-2 §4.5)",
    ),
    (
        "run_step",
        "selected",
        "fan-out winner; NULL when fan_out = 1; false on a loser, which is also superseded \
         (ANA-2 §4.5)",
    ),
    (
        "run_step",
        "attempt",
        "1-based; retry_limit is the number of additional attempts, so attempt <= retry_limit + 1 \
         (ANA-2 §4.2)",
    ),
    (
        "run_step",
        "isolation_path",
        "the primary repo tree; every repo in scope has a run_step_tree row (ANA-2 §4.6)",
    ),
    (
        "run_step",
        "promoted_at",
        "set when the step was promoted to an interactive chat (R-ORCH-5, ANA-2 §4.8)",
    ),
    (
        "item",
        "touched_paths",
        "R-ORCH-9 declared overlap set: \"<repo_name>:<glob>\" entries, or a bare glob meaning \
         the primary repo. Empty means unknown, which overlaps the whole primary repo \
         (ANA-2 §4.7).",
    ),
    (
        "box",
        "settings",
        "ANA-2 §4.7 BoxSettings: max_concurrent_items (R-ORCH-9), command_limits {class: n} \
         (R-MCP-3). Every field optional; defaults come from app_setting.",
    ),
    (
        "project",
        "settings",
        "ANA-2 §4.7 ProjectSettings: token_budget, retention_days, cached_transcript_steps, \
         keep_raw_events, default_isolation, per_token_cap_run, per_token_cap_batch, \
         step_deadline_seconds, default_agent_id, judge_agent_id, copy_exclude. Every field \
         optional.",
    ),
];

/// The four `COMMENT ON COLUMN` texts of `0005_box_identity.sql` (MOD-7 blueprint §3.1, D30),
/// verbatim, for the same reason as [`ANA_COLUMN_COMMENTS`]: a forward-only migration cannot be
/// corrected afterwards, so a paraphrase must fail here first.
const MOD7_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[
    (
        "box",
        "machine_fingerprint",
        "MOD-7 D1: HMAC-SHA256 keyed by the OS machine identity (/etc/machine-id, \
         IOPlatformUUID, MachineGuid) over the constant htui/box-fingerprint/v1, lowercase hex. \
         NULL means no identity was readable. It checks a box.toml id and never keys the row: a \
         mismatch means a copied box.toml, and a new box is minted.",
    ),
    (
        "box",
        "edit_version",
        "MOD-7 D14: compare-and-set token of the declared_tags, quirks and settings editors, \
         bumped by them only. Registration and the probe never write it, so a reconnect cannot \
         stale an open editor.",
    ),
    (
        "box",
        "probe_spec_digest",
        "MOD-7 D18: sha256 hex of the effective box probe spec (the compiled seed overlaid by \
         app_setting.box_probe_spec) at the last successful probe. Another digest re-probes at \
         the next connect. NULL means never probed since 0005.",
    ),
    (
        "box",
        "htui_version",
        "MOD-7 D5: the htui version at the last successful probe, and the re-probe trigger \
         (R-BOX-2). Inserted at first registration; rewritten only by the probe writer, never by \
         a reconnect.",
    ),
];

/// The five `COMMENT ON COLUMN` texts of `0007_skill_attachments.sql` (ANA-22 §7.1, MOD-9 plan
/// D38), verbatim, for [`ANA_COLUMN_COMMENTS`]'s reason.
const MOD9_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[
    (
        "skill_version",
        "source",
        "import provenance and raw frontmatter; prefills an attachment, never read by the prompt \
         builder",
    ),
    (
        "skill_binding",
        "project_id",
        "NULL = global attachment (every project)",
    ),
    (
        "skill_binding",
        "activation",
        "always | glob | off; the most specific attachment of a skill wins (ANA-22)",
    ),
    (
        "skill_binding",
        "globs",
        "effective globs: typed plus languages expanded at save; <repo>:<glob> only on project or \
         phase rows",
    ),
    (
        "skill_binding",
        "languages",
        "languages as authored; display only",
    ),
];

/// The one `COMMENT ON COLUMN` text of `0009_agent_box_user_off.sql` (MOD-23 plan D242), verbatim,
/// for [`ANA_COLUMN_COMMENTS`]'s reason.
const MOD23_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[(
    "agent_box",
    "user_off",
    "MOD-23 D242: the per-box switch, written only by set_agent_box_enabled. true keeps enabled \
     false through every probe: upsert_agent_box writes enabled = EXCLUDED.enabled AND NOT \
     user_off on conflict. Switching on re-derives enabled from the stored probe status.",
)];

/// The two `COMMENT ON COLUMN` texts of `0010_prompt_digest_undigested.sql` (MOD-33 D273),
/// verbatim, for [`ANA_COLUMN_COMMENTS`]'s reason.
const MOD33_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[
    (
        "run_step",
        "prompt_digest",
        "ANA-5 4.7 as amended by MOD-33: sha256, lowercase hex, over the canonical assembled \
         prompt TEXT as sent with each undigested span replaced by its fixed stand-in - today \
         only the box section's hostname value, as [hostname] - LF normalised, BOM stripped, one \
         trailing LF, scrubbed before hashing. trim_record.undigested lists the spans. Not over \
         the payload and not over sections[]. An audit field, never a replay key (ANA-2 4.9).",
    ),
    (
        "run_step",
        "trim_record",
        "ANA-5 5.1 as amended by MOD-9 D42 and D118 and MOD-33: {v, template, budget, \
         budget_source, reserve, target, estimator, estimated_before, estimated_after, \
         sections[], skill_choices[], undigested[], excerpts, notes}, v 4. undigested[] names \
         every span rendered into the prompt but excluded from prompt_digest: box.hostname, or \
         empty. skill_choices[] is every candidate skill, ordered by position then name, each \
         {skill, name, version, level, activation, active, reason} with reason always, matched, \
         no_match, off, no_path, missing_version or not_placed (ANA-22 6 item 8); a matched \
         choice adds path, <repo>:<path>, the first matching file in repo then path byte order. \
         A v 3 record, written before 0010, has no undigested and digested the hostname; a v 2 \
         record, written before 0008, has no matched or no_match; a v 1 record, written before \
         0007, has no skill_choices. Canonical; the prompt payload sections[] array is its \
         abridged projection. Written at stage 3 by set_step_prompt, before the session starts.",
    ),
];

/// The nine `COMMENT ON COLUMN` texts of `0012_persona.sql` (MOD-26 plan D1, D5), verbatim and in
/// the migration's order, for [`ANA_COLUMN_COMMENTS`]'s reason. `step_graph_phase` is already a
/// checked table, so its new `persona_id` is listed with the eight `persona` columns.
const MOD26_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[
    (
        "persona",
        "id",
        "MOD-26 D1: client-minted UUIDv7; what step_graph_phase.persona_id references.",
    ),
    (
        "persona",
        "name",
        "MOD-26 D1, D3: the registry name, unique; 1-64 of a-z, 0-9 and single inner hyphens, \
         checked by the writers. A run snapshot freezes a persona under this name.",
    ),
    (
        "persona",
        "description",
        "MOD-26 D1: the one-line summary a picker shows; never rendered into a prompt.",
    ),
    (
        "persona",
        "body",
        "MOD-26 D1, D13: the role text stage 3 renders as the protected persona section ahead of \
         the phase template. Never blank.",
    ),
    (
        "persona",
        "tools",
        "MOD-26 D2, D10: {allow, deny, deny_kinds, command_run}, narrow-only against the agent \
         row. allow keeps built-in tool names (empty keeps all), deny removes tool names, \
         deny_kinds denies ACP tool kinds (read, edit, delete, move, search, execute, fetch), \
         command_run false withdraws the command queue. Unknown keys are refused.",
    ),
    (
        "persona",
        "permission",
        "MOD-26 D2, D10: {default, rules[]}, deny-only. default is null, ask or deny; every rule \
         answers reject_once or reject_always and is evaluated before the agent row rules and \
         remembered choices. Unknown keys are refused.",
    ),
    (
        "persona",
        "created_at",
        "MOD-26 D1: when the row was inserted.",
    ),
    (
        "persona",
        "updated_at",
        "MOD-26 D1: the update_persona compare-and-set token, stamped by trg_persona_updated_at.",
    ),
    (
        "step_graph_phase",
        "persona_id",
        "MOD-26 D5: the persona this phase runs under; NULL for none. StartRun freezes it by name \
         and content into run.graph_snapshot (phases[].persona, personas[]); ON DELETE RESTRICT.",
    ),
];

/// The one `COMMENT ON TABLE` of `0003_orchestration.sql` (ANA-2 §9), verbatim. Kept beside
/// [`ANA_COLUMN_COMMENTS`] rather than in it: `col_description` cannot read it, because a table
/// comment is `objsubid = 0`.
const ANA_TABLE_COMMENT: (&str, &str) = (
    "run_step_tree",
    "R-ORCH-8 isolation, per repo; ANA-2 §4.6. A dirty local or shared_serialized tree is never \
     reset by the recovery sweep (ANA-2 §4.9).",
);

#[tokio::test]
async fn the_ana_column_comments_are_present_and_verbatim() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    for (table, column, expected) in ANA_COLUMN_COMMENTS
        .iter()
        .chain(MOD7_COLUMN_COMMENTS)
        .chain(MOD9_COLUMN_COMMENTS)
        .chain(MOD23_COLUMN_COMMENTS)
        .chain(MOD33_COLUMN_COMMENTS)
        .chain(MOD26_COLUMN_COMMENTS)
    {
        let actual: Option<String> = sqlx::query_scalar(
            "SELECT pg_catalog.col_description(c.oid, a.attnum) \
             FROM pg_class c JOIN pg_attribute a ON a.attrelid = c.oid \
             WHERE c.relnamespace = 'public'::regnamespace AND c.relkind = 'r' \
               AND c.relname = $1 AND a.attname = $2",
        )
        .bind(table)
        .bind(column)
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|err| panic!("read col_description for {table}.{column}: {err}"));

        assert_eq!(
            actual.as_deref(),
            Some(*expected),
            "{table}.{column}'s comment is the ANA (or MOD-7, MOD-9, MOD-23, MOD-33 or MOD-26) text \
             byte for byte"
        );
    }

    // The one table comment, which `col_description` above cannot see: a table's own comment is
    // `objsubid = 0` on the class, not on any attribute.
    let (table, expected) = ANA_TABLE_COMMENT;
    let actual: Option<String> = sqlx::query_scalar(
        "SELECT pg_catalog.obj_description(c.oid, 'pg_class') FROM pg_class c \
         WHERE c.relnamespace = 'public'::regnamespace AND c.relkind = 'r' AND c.relname = $1",
    )
    .bind(table)
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|err| panic!("read obj_description for {table}: {err}"));
    assert_eq!(
        actual.as_deref(),
        Some(expected),
        "{table}'s table comment is the ANA-2 §9 text byte for byte"
    );

    // And nothing else in those tables carries one, so a reader of `\d+` sees exactly the
    // forty-four contracts the three ANAs, MOD-7, ANA-22, MOD-23, MOD-33 and MOD-26 wrote and no
    // half-finished forty-fifth.
    let commented: Vec<(String, String)> = sqlx::query_as(
        "SELECT c.relname::text, a.attname::text FROM pg_class c \
         JOIN pg_attribute a ON a.attrelid = c.oid \
         WHERE c.relnamespace = 'public'::regnamespace AND c.relkind = 'r' \
           AND c.relname = ANY($1) AND a.attnum > 0 \
           AND pg_catalog.col_description(c.oid, a.attnum) IS NOT NULL \
         ORDER BY 1, 2",
    )
    .bind(
        ANA_COLUMN_COMMENTS
            .iter()
            .chain(MOD7_COLUMN_COMMENTS)
            .chain(MOD9_COLUMN_COMMENTS)
            .chain(MOD23_COLUMN_COMMENTS)
            .chain(MOD33_COLUMN_COMMENTS)
            .chain(MOD26_COLUMN_COMMENTS)
            .chain(MOD26_COLUMN_COMMENTS)
            .map(|(table, _, _)| (*table).to_owned())
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect::<Vec<String>>(),
    )
    .fetch_all(&db.pool)
    .await
    .expect("list the commented columns of the named tables");

    let mut expected: Vec<(String, String)> = ANA_COLUMN_COMMENTS
        .iter()
        .chain(MOD7_COLUMN_COMMENTS)
        .chain(MOD9_COLUMN_COMMENTS)
        .chain(MOD23_COLUMN_COMMENTS)
        .chain(MOD33_COLUMN_COMMENTS)
        .chain(MOD26_COLUMN_COMMENTS)
        .map(|(table, column, _)| ((*table).to_owned(), (*column).to_owned()))
        .collect();
    expected.sort();
    assert_eq!(
        commented, expected,
        "exactly the forty-four commented columns, and no others"
    );

    db.drop_db().await;
}

/// ANA-5 §5.3's ten defaults, folded into `0002` as section 3 (`docs/ANA-5.md:2183-2189`).
/// `prompt_reserve_fraction` is the one non-integer and is checked beside this table.
const ANA5_INTEGER_DEFAULTS: &[(&str, i64)] = &[
    ("excerpt_file_line_cap", 400),
    ("excerpt_head_lines", 200),
    ("excerpt_max_file_bytes", 524_288),
    ("excerpt_max_files", 12),
    ("excerpt_max_scan_files", 20_000),
    ("excerpt_provider_deadline_ms", 1_500),
    ("max_skill_tokens", 20_000),
    ("prompt_upstream_hops", 2),
    ("token_budget", 120_000),
];

#[tokio::test]
async fn the_ten_ana5_defaults_land_with_their_values() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let mut keys: Vec<String> = ANA5_INTEGER_DEFAULTS
        .iter()
        .map(|(key, _)| (*key).to_owned())
        .collect();
    keys.push("prompt_reserve_fraction".to_owned());
    keys.sort();

    let rows: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT key, value FROM app_setting WHERE key = ANY($1) ORDER BY key")
            .bind(&keys)
            .fetch_all(&db.pool)
            .await
            .expect("read the ANA-5 defaults");

    assert_eq!(
        rows.len(),
        10,
        "all ten ANA-5 §5.3 keys landed, got {rows:?}"
    );

    for (key, expected) in ANA5_INTEGER_DEFAULTS {
        let value = &rows
            .iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("app_setting.{key} exists"))
            .1;
        assert_eq!(
            value.as_i64(),
            Some(*expected),
            "app_setting.{key} carries ANA-5's value"
        );
    }

    let fraction = &rows
        .iter()
        .find(|(k, _)| k == "prompt_reserve_fraction")
        .expect("app_setting.prompt_reserve_fraction exists")
        .1;
    assert_eq!(
        fraction.as_f64(),
        Some(0.10),
        "the one fractional default decodes as a JSON number"
    );

    db.drop_db().await;
}

/// ANA-2 §5.4's twelve defaults, seeded by `0003_orchestration.sql` section 8, as they stand once
/// every migration has run.
///
/// One of them is not 0003's literal: 0003 seeds `max_agents_per_run` = 6 and
/// `0004_max_agents_per_run_default.sql` moves an untouched 6 to 8, so the seeded `feature` graph
/// with a judged 3-way `implement` (seven planned agents) runs by default. The 6 itself is pinned
/// by [`the_0004_bump_moves_only_an_untouched_six`].
///
/// Written as JSON **text** rather than as `i64`, because three of the twelve are `null` and one is
/// an object: `as_i64()` would answer `None` for all four and a value pin that cannot fail on those
/// keys is not one. Each right-hand side is the literal of the migration, parsed here and compared
/// as `serde_json::Value` so key order inside `command_limits` is not part of the assertion.
const ANA2_DEFAULTS: &[(&str, &str)] = &[
    ("command_limits", r#"{"build":1,"test":4,"verify":1}"#),
    ("copy_max_total_bytes", "21474836480"),
    ("default_isolation", r#""worktree""#),
    ("lease_refresh_seconds", "60"),
    ("lease_ttl_seconds", "120"),
    ("max_agents_per_run", "8"),
    ("max_concurrent_items", "2"),
    ("max_fan_out", "4"),
    ("per_token_cap_batch", "null"),
    ("per_token_cap_run", "null"),
    ("scheduler_window", "null"),
    ("step_deadline_seconds", "7200"),
];

/// The twelve are pinned by value, not only by the row count `0003_orchestration.sql` moved to 24
/// (and `0004_max_agents_per_run_default.sql`, an `UPDATE`, leaves at 24).
///
/// A migration is forward-only and can never be edited, so a transcription slip is permanent:
/// `step_deadline_seconds` 7200 -> 720, `copy_max_total_bytes` losing a digit, or
/// `lease_ttl_seconds` and `lease_refresh_seconds` transposed would each keep the count at 24 and
/// pass every other test in the workspace. ANA-5's ten are pinned this way twice over
/// ([`ANA5_INTEGER_DEFAULTS`] and `pg_criteria.rs`); ANA-2's twelve were not (T2 audit).
#[tokio::test]
async fn the_twelve_ana2_defaults_land_with_their_values() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let keys: Vec<String> = ANA2_DEFAULTS
        .iter()
        .map(|(key, _)| (*key).to_owned())
        .collect();
    let rows: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT key, value FROM app_setting WHERE key = ANY($1) ORDER BY key")
            .bind(&keys)
            .fetch_all(&db.pool)
            .await
            .expect("read ANA-2 defaults");

    assert_eq!(
        rows.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(),
        keys.iter().map(String::as_str).collect::<Vec<_>>(),
        "all twelve ANA-2 §5.4 keys landed, and the const is in the key order the query returns"
    );

    for ((key, expected), (_, actual)) in ANA2_DEFAULTS.iter().zip(&rows) {
        let expected: serde_json::Value =
            serde_json::from_str(expected).expect("the const carries JSON the migration wrote");
        assert_eq!(
            *actual, expected,
            "app_setting.{key} carries ANA-2 §5.4's value, as 0004 amends it"
        );
    }

    db.drop_db().await;
}

/// `0004_max_agents_per_run_default.sql` moves the 6 `0003_orchestration.sql` seeded to 8, and
/// only that 6: a value somebody chose is theirs, and a forward-only migration cannot tell a
/// deliberate 5 from a default, so it touches nothing but the exact seed.
///
/// Staged through [`MIGRATOR`] itself: `run_to(3)` stops after 0003, so the seed is observable
/// before 0004 runs, and `run_to(4)` afterwards applies 0004 alone (a plain `run` would carry on
/// into 0005 and later, which this case is not about).
#[tokio::test]
async fn the_0004_bump_moves_only_an_untouched_six() {
    let Some(db) = common::bare_db().await else {
        return;
    };
    let read = || async {
        sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT value FROM app_setting WHERE key = 'max_agents_per_run'",
        )
        .fetch_one(&db.pool)
        .await
        .expect("app_setting.max_agents_per_run exists")
    };

    MIGRATOR
        .run_to(3, &db.pool)
        .await
        .expect("apply 0001 through 0003");
    assert_eq!(
        read().await,
        serde_json::json!(6),
        "0003_orchestration.sql seeds ANA-2 §5.4's 6, unedited"
    );

    sqlx::query("UPDATE app_setting SET value = '5'::jsonb WHERE key = 'max_agents_per_run'")
        .execute(&db.pool)
        .await
        .expect("a user lowers the cap to 5");
    MIGRATOR
        .run_to(4, &db.pool)
        .await
        .expect("apply 0004 alone");
    let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY 1")
        .fetch_all(&db.pool)
        .await
        .expect("read _sqlx_migrations");
    assert_eq!(applied, vec![1, 2, 3, 4], "0004 ran");
    assert_eq!(
        read().await,
        serde_json::json!(5),
        "0004 leaves a value that is not the seeded 6 alone"
    );

    db.drop_db().await;
}

/// The smallest FK chain an `item` row needs (project -> step_graph -> item_kind), under the
/// `created_by` the caller names. Returns `(project, kind)`; the kind's prefix is `RES`.
async fn plant_item_kind(pool: &sqlx::PgPool, user: uuid::Uuid) -> (uuid::Uuid, uuid::Uuid) {
    let project = uuid::Uuid::now_v7();
    let graph = uuid::Uuid::now_v7();
    let kind = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO project (id, slug, name, created_by) VALUES ($1, 'res', 'Res', $2)")
        .bind(project)
        .bind(user)
        .execute(pool)
        .await
        .expect("insert project");
    sqlx::query("INSERT INTO step_graph (id, project_id, name) VALUES ($1, $2, 'default')")
        .bind(graph)
        .bind(project)
        .execute(pool)
        .await
        .expect("insert step_graph");
    sqlx::query(
        "INSERT INTO item_kind (id, project_id, prefix, name, default_graph_id) \
         VALUES ($1, $2, 'RES', 'resolution', $3)",
    )
    .bind(kind)
    .bind(project)
    .bind(graph)
    .execute(pool)
    .await
    .expect("insert item_kind");
    (project, kind)
}

/// MOD-38 (ANA-11 §4.2, blueprint F10): `chk_item_resolution_iff_closed` holds both halves. A
/// `closed` row must say how it closed, and a row that is not closed must not claim a resolution.
#[tokio::test]
async fn item_resolution_iff_closed_rejects_both_halves() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    let user = db.store.this_user().as_uuid();
    let (project, kind) = plant_item_kind(&db.pool, user).await;

    let insert = |number: i32, status: &'static str, resolution: Option<&'static str>| {
        sqlx::query(
            "INSERT INTO item (project_id, kind_id, key_prefix, key_number, title, status, \
             resolution, created_by) VALUES ($1, $2, 'RES', $3, 'r', $4, $5, $6)",
        )
        .bind(project)
        .bind(kind)
        .bind(number)
        .bind(status)
        .bind(resolution)
        .bind(user)
        .execute(&db.pool)
    };

    for (number, status, resolution, why) in [
        (1, "closed", None, "a closed row with no resolution"),
        (2, "open", Some("done"), "an open row with a resolution"),
    ] {
        let err = insert(number, status, resolution)
            .await
            .expect_err("the CHECK refuses the row");
        let constraint = err
            .as_database_error()
            .and_then(|e| e.constraint())
            .map(str::to_owned);
        assert_eq!(
            constraint.as_deref(),
            Some("chk_item_resolution_iff_closed"),
            "{why} is refused by the iff constraint, got {err}"
        );
    }

    insert(3, "closed", Some("withdrawn"))
        .await
        .expect("a closed row with a resolution is accepted");
    insert(4, "open", None)
        .await
        .expect("an open row without one is accepted");

    db.drop_db().await;
}

/// MOD-38 (plan D8): every item closed before 0006 closed through the old `-> closed` edges, which
/// only a finished item took, so 0006 backfills `done` before it adds the iff constraint, with
/// `trg_item_updated_at` off so no row's `updated_at` moves. Staged like the 0004 case:
/// `run_to(5)`, plant a closed row, then `run_to(6)` applies 0006 (MOD-9 D69: the plain `run`
/// would apply 0007 too).
#[tokio::test]
async fn closed_rows_backfill_to_done() {
    const PLANTED_AT: &str = "2020-01-02T03:04:05Z";
    let Some(db) = common::bare_db().await else {
        return;
    };

    MIGRATOR
        .run_to(5, &db.pool)
        .await
        .expect("apply 0001 through 0005");
    let user = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO app_user (id, name) VALUES ($1, 'backfill')")
        .bind(user)
        .execute(&db.pool)
        .await
        .expect("insert app_user");
    let (project, kind) = plant_item_kind(&db.pool, user).await;
    let mut planted = Vec::new();
    for (number, status) in [(1, "closed"), (2, "open"), (3, "done")] {
        let id = uuid::Uuid::now_v7();
        sqlx::query(
            "INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, title, status, \
             created_by, updated_at) VALUES ($1, $2, $3, 'RES', $4, 'r', $5, $6, $7::timestamptz)",
        )
        .bind(id)
        .bind(project)
        .bind(kind)
        .bind(number)
        .bind(status)
        .bind(user)
        .bind(PLANTED_AT)
        .execute(&db.pool)
        .await
        .expect("insert a pre-0006 item");
        planted.push((id, status));
    }

    MIGRATOR.run_to(6, &db.pool).await.expect("apply 0006");

    for (id, status) in planted {
        let (resolution, kept): (Option<String>, bool) = sqlx::query_as(
            "SELECT resolution, updated_at = $2::timestamptz FROM item WHERE id = $1",
        )
        .bind(id)
        .bind(PLANTED_AT)
        .fetch_one(&db.pool)
        .await
        .expect("read item.resolution and updated_at");
        let expected = (status == "closed").then(|| "done".to_owned());
        assert_eq!(
            resolution, expected,
            "a pre-0006 `{status}` row backfills to {expected:?}"
        );
        assert!(
            kept,
            "the backfill leaves a pre-0006 `{status}` row's updated_at at {PLANTED_AT}"
        );
    }
    let enabled: String = sqlx::query_scalar(
        "SELECT tgenabled::text FROM pg_trigger \
          WHERE tgrelid = 'item'::regclass AND tgname = 'trg_item_updated_at'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read trg_item_updated_at");
    assert_eq!(
        enabled, "O",
        "0006 re-enables trg_item_updated_at after the backfill"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn connect_reports_up_to_date_after_apply() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let connected = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("connect to the migrated database");
    assert_eq!(connected.migrations, MigrationState::UpToDate);

    db.drop_db().await;
}

#[tokio::test]
async fn connect_reports_pending_on_a_bare_database() {
    let Some(db) = common::bare_db().await else {
        return;
    };

    assert_eq!(
        db.migrations_at_connect,
        MigrationState::Pending(12),
        "twelve embedded migrations, none applied (through MOD-26's 0012_persona.sql)"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn a_newer_applied_version_is_refused() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (9999, 'from a newer htui', true, '\\x00'::bytea, 0)",
    )
    .execute(&db.pool)
    .await
    .expect("plant a newer applied migration");

    let err = PgStore::connect(&db.url, &db.identity)
        .await
        .expect_err("a newer schema is refused, not adopted");
    match &err {
        StoreError::Backend(text) => assert!(
            text.contains("schema is newer"),
            "the refusal names the reason, got {text}"
        ),
        other => panic!("expected StoreError::Backend, got {other:?}"),
    }

    db.drop_db().await;
}

#[tokio::test]
async fn a_checksum_mismatch_is_refused() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    sqlx::query("UPDATE _sqlx_migrations SET checksum = '\\xdeadbeef'::bytea WHERE version = 1")
        .execute(&db.pool)
        .await
        .expect("corrupt the recorded checksum");

    let err = PgStore::connect(&db.url, &db.identity)
        .await
        .expect_err("a modified migration is refused");
    match &err {
        StoreError::Backend(text) => assert!(
            text.contains("checksum"),
            "the refusal names the reason, got {text}"
        ),
        other => panic!("expected StoreError::Backend, got {other:?}"),
    }

    db.drop_db().await;
}

/// The stored `htui_target_version` document, if any (MOD-40 plan D9).
async fn target(pool: &sqlx::PgPool) -> Option<serde_json::Value> {
    sqlx::query_scalar("SELECT value FROM app_setting WHERE key = $1")
        .bind(TARGET_VERSION_KEY)
        .fetch_optional(pool)
        .await
        .expect("read the target version")
}

/// Plants `value` as the target, over whatever is stored: another box's newer build, or a hand
/// edit.
async fn plant_target(pool: &sqlx::PgPool, value: serde_json::Value) {
    sqlx::query(
        "INSERT INTO app_setting (key, value) VALUES ($1, $2) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
    )
    .bind(TARGET_VERSION_KEY)
    .bind(value)
    .execute(pool)
    .await
    .expect("plant a target version");
}

/// How many tables the `public` schema holds.
async fn public_tables(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE schemaname = 'public'")
        .fetch_one(pool)
        .await
        .expect("count the public tables")
}

/// MOD-40 plan D8, `R-STO-5` as amended: a headless connect over a pending schema refuses with
/// the count and applies nothing, and over a database with no migrations table it does not even
/// create one (blueprint F-22: a role without `CREATE` could not).
#[tokio::test]
async fn a_headless_connect_never_migrates() {
    let Some(db) = common::bare_db().await else {
        return;
    };
    assert_eq!(db.migrations_at_connect, MigrationState::Pending(12));

    let refused = PgStore::connect_headless(&db.url, &db.identity, HEADLESS_WAIT, PoolSize::TUI)
        .await
        .expect_err("a pending schema is refused");
    assert_eq!(refused, HeadlessError::MigrationsPending(12));
    assert_eq!(
        common::count(&db.pool, "_sqlx_migrations").await,
        0,
        "a headless connect applies nothing"
    );
    assert_eq!(
        public_tables(&db.pool).await,
        1,
        "a headless connect applies nothing: the bookkeeping table the TUI's connect made is all"
    );

    sqlx::query("DROP TABLE _sqlx_migrations")
        .execute(&db.pool)
        .await
        .expect("drop the bookkeeping table");
    let refused = PgStore::connect_headless(&db.url, &db.identity, HEADLESS_WAIT, PoolSize::TUI)
        .await
        .expect_err("no migrations table is every migration pending");
    assert_eq!(refused, HeadlessError::MigrationsPending(12));
    let absent: bool = sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NULL")
        .fetch_one(&db.pool)
        .await
        .expect("ask for the bookkeeping table");
    assert!(
        absent,
        "it does not even create the bookkeeping table (blueprint F-22)"
    );
    assert_eq!(
        public_tables(&db.pool).await,
        0,
        "the schema is still empty"
    );

    db.drop_db().await;
}

/// MOD-40 plan D8: a headless connect refuses what `PgStore::connect` refuses, a newer, dirty or
/// drifted schema, and refuses it before the bootstrap writes anything (blueprint B19).
#[tokio::test]
async fn a_headless_connect_refuses_a_newer_schema() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    let boxes = common::count(&db.pool, "box").await;
    let stranger = identity::Identity {
        box_id: BoxId::new(),
        hostname: format!("HTUI-TEST-{}", uuid::Uuid::now_v7().simple()),
    };
    let refusal = |why: &'static str| {
        let (url, stranger) = (db.url.clone(), stranger.clone());
        async move {
            match PgStore::connect_headless(&url, &stranger, HEADLESS_WAIT, PoolSize::TUI).await {
                Err(HeadlessError::Store(StoreError::Backend(text))) => text,
                other => panic!("expected a store refusal ({why}), got {other:?}"),
            }
        }
    };

    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (9999, 'from a newer htui', true, '\\x00'::bytea, 0)",
    )
    .execute(&db.pool)
    .await
    .expect("plant a newer applied migration");
    let text = refusal("newer").await;
    assert!(text.contains("schema is newer"), "got {text}");
    assert_eq!(
        common::count(&db.pool, "box").await,
        boxes,
        "refused before the bootstrap"
    );
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 9999")
        .execute(&db.pool)
        .await
        .expect("remove the newer migration");

    sqlx::query("UPDATE _sqlx_migrations SET success = false WHERE version = 7")
        .execute(&db.pool)
        .await
        .expect("mark the last migration dirty");
    let text = refusal("dirty").await;
    assert!(text.contains("partially applied"), "got {text}");
    assert_eq!(
        common::count(&db.pool, "box").await,
        boxes,
        "refused before the bootstrap"
    );
    sqlx::query("UPDATE _sqlx_migrations SET success = true WHERE version = 7")
        .execute(&db.pool)
        .await
        .expect("mark it clean again");

    sqlx::query("UPDATE _sqlx_migrations SET checksum = '\\xdeadbeef'::bytea WHERE version = 1")
        .execute(&db.pool)
        .await
        .expect("corrupt the recorded checksum");
    let text = refusal("drifted").await;
    assert!(text.contains("checksum"), "got {text}");
    assert_eq!(
        common::count(&db.pool, "box").await,
        boxes,
        "refused before the bootstrap"
    );

    db.drop_db().await;
}

/// MOD-40 plan D9, blueprint B17, B18: every `apply_migrations` raises the target to this build
/// and never lowers it; a malformed or deleted target is written afresh.
#[tokio::test]
async fn applying_migrations_raises_the_target_and_never_lowers_it() {
    let Some(mut db) = common::bare_db().await else {
        return;
    };

    db.store
        .apply_migrations()
        .await
        .expect("apply the embedded migrations");
    assert_eq!(
        target(&db.pool).await,
        Some(json!(HTUI_VERSION)),
        "the first migrator writes its version"
    );
    assert_eq!(db.store.below_target(), None);

    plant_target(&db.pool, json!("0.0.1")).await;
    db.store.apply_migrations().await.expect("apply again");
    assert_eq!(target(&db.pool).await, Some(json!(HTUI_VERSION)), "raised");
    assert_eq!(db.store.below_target(), None);

    plant_target(&db.pool, json!("99.0.0")).await;
    db.store.apply_migrations().await.expect("apply again");
    assert_eq!(
        target(&db.pool).await,
        Some(json!("99.0.0")),
        "never lowered"
    );
    assert_eq!(
        db.store.below_target(),
        Some("99.0.0"),
        "a migrator below the target knows it"
    );

    plant_target(&db.pool, json!(42)).await;
    db.store.apply_migrations().await.expect("apply again");
    assert_eq!(
        target(&db.pool).await,
        Some(json!(HTUI_VERSION)),
        "a malformed target is replaced (blueprint B18)"
    );
    assert_eq!(db.store.below_target(), None);

    sqlx::query("DELETE FROM app_setting WHERE key = $1")
        .bind(TARGET_VERSION_KEY)
        .execute(&db.pool)
        .await
        .expect("delete the target by hand");
    db.store.apply_migrations().await.expect("apply again");
    assert_eq!(
        target(&db.pool).await,
        Some(json!(HTUI_VERSION)),
        "re-inserted"
    );

    assert_eq!(
        common::count(&db.pool, "_sqlx_migrations").await,
        12,
        "the later applies migrate nothing: the twelve embedded migrations (through MOD-26's \
         0012_persona.sql) are applied once"
    );

    db.drop_db().await;
}

/// MOD-40 plan D9, PRD D3: a headless process below the target refuses before it registers, and
/// refuses a target that is not a version; at or above it, it connects as `connect` does.
#[tokio::test]
async fn a_headless_connect_below_the_target_refuses() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    let stranger = identity::Identity {
        box_id: BoxId::new(),
        hostname: format!("HTUI-TEST-{}", uuid::Uuid::now_v7().simple()),
    };
    let registered = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM box WHERE id = $1")
            .bind(stranger.box_id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("count the stranger's box rows")
    };

    plant_target(&db.pool, json!("99.0.0")).await;
    let refused = PgStore::connect_headless(&db.url, &stranger, HEADLESS_WAIT, PoolSize::TUI)
        .await
        .expect_err("a build below the target is refused");
    assert_eq!(
        refused,
        HeadlessError::BelowTarget {
            ours: HTUI_VERSION.into(),
            target: "99.0.0".into(),
        }
    );
    assert_eq!(
        registered().await,
        0,
        "refused before the bootstrap: nothing registered"
    );

    plant_target(&db.pool, json!("banana")).await;
    match PgStore::connect_headless(&db.url, &stranger, HEADLESS_WAIT, PoolSize::TUI).await {
        Err(HeadlessError::Store(StoreError::Backend(text))) => assert!(
            text.contains("htui_target_version") && text.contains("not a version"),
            "the refusal names the key and the reason, got {text}"
        ),
        other => panic!("a headless process never guesses a malformed target, got {other:?}"),
    }
    assert_eq!(registered().await, 0, "refused before the bootstrap");

    plant_target(&db.pool, json!(HTUI_VERSION)).await;
    let store = PgStore::connect_headless(&db.url, &stranger, HEADLESS_WAIT, PoolSize::TUI)
        .await
        .expect("at the target a headless process connects");
    assert_eq!(
        store.this_box(),
        stranger.box_id,
        "at the target it connects and registers as `connect` does"
    );
    assert_eq!(store.below_target(), None);

    plant_target(&db.pool, json!("0.0.1")).await;
    PgStore::connect_headless(&db.url, &stranger, HEADLESS_WAIT, PoolSize::TUI)
        .await
        .expect("above the target a headless process connects");

    db.drop_db().await;
}

/// MOD-41 PRD D7: a headless pool holds exactly the connections its caller asked for, not the
/// TUI's eight (blueprint §13.4).
#[tokio::test]
async fn a_headless_pool_honours_its_size() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let store =
        PgStore::connect_headless(&db.url, &db.identity, HEADLESS_WAIT, PoolSize::clamped(3))
            .await
            .expect("a headless connect over an up-to-date schema");
    assert_eq!(
        store.pool().options().get_max_connections(),
        3,
        "the pool is the size `--pool-size` asked for"
    );
    store.pool().close().await;

    db.drop_db().await;
}

/// MOD-41 PRD D7: `--pool-size` is clamped to `2..=8` before the pool is built, so a real connect
/// never holds fewer than two connections or more than the TUI's eight (blueprint §13.4).
#[tokio::test]
async fn a_headless_pool_is_clamped() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    for (requested, used) in [(1, PoolSize::MIN), (64, PoolSize::MAX)] {
        let size = PoolSize::clamped(requested);
        assert_eq!(size.get(), used, "{requested} is clamped to {used}");
        let store = PgStore::connect_headless(&db.url, &db.identity, HEADLESS_WAIT, size)
            .await
            .expect("a headless connect over an up-to-date schema");
        assert_eq!(
            store.pool().options().get_max_connections(),
            used,
            "a pool of {requested} is built with {used} connections"
        );
        store.pool().close().await;
    }

    db.drop_db().await;
}

/// MOD-40 plan D9, PRD D3: a TUI below the target connects, runs and records the target; at it,
/// or over a malformed one, there is nothing to say (blueprint B18).
#[tokio::test]
async fn a_tui_connect_below_the_target_reports_it() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    plant_target(&db.pool, json!("99.0.0")).await;
    let connected = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("a TUI below the target connects");
    assert_eq!(connected.migrations, MigrationState::UpToDate);
    assert_eq!(
        connected.store.below_target(),
        Some("99.0.0"),
        "a TUI runs below the target and knows it (PRD D3)"
    );

    plant_target(&db.pool, json!(HTUI_VERSION)).await;
    let connected = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("a TUI at the target connects");
    assert_eq!(connected.store.below_target(), None);

    plant_target(&db.pool, json!(42)).await;
    let connected = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("a TUI over a malformed target connects");
    assert_eq!(
        connected.store.below_target(),
        None,
        "a TUI ignores a malformed target (blueprint B18)"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn the_trigger_bumps_updated_at_on_update_only() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    // The smallest FK chain that reaches `item`: project -> step_graph -> item_kind -> item.
    let user = db.store.this_user().as_uuid();
    let project = uuid::Uuid::now_v7();
    let graph = uuid::Uuid::now_v7();
    let kind = uuid::Uuid::now_v7();
    let item = uuid::Uuid::now_v7();
    let stamp = chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
        .expect("a literal RFC-3339 timestamp")
        .with_timezone(&chrono::Utc);

    sqlx::query("INSERT INTO project (id, slug, name, created_by) VALUES ($1, 'trg', 'Trg', $2)")
        .bind(project)
        .bind(user)
        .execute(&db.pool)
        .await
        .expect("insert project");
    sqlx::query("INSERT INTO step_graph (id, project_id, name) VALUES ($1, $2, 'default')")
        .bind(graph)
        .bind(project)
        .execute(&db.pool)
        .await
        .expect("insert step_graph");
    sqlx::query(
        "INSERT INTO item_kind (id, project_id, prefix, name, default_graph_id) \
         VALUES ($1, $2, 'TRG', 'trigger', $3)",
    )
    .bind(kind)
    .bind(project)
    .bind(graph)
    .execute(&db.pool)
    .await
    .expect("insert item_kind");
    sqlx::query(
        "INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, title, created_by, \
         created_at, updated_at) VALUES ($1, $2, $3, 'TRG', 1, 'before', $4, $5, $5)",
    )
    .bind(item)
    .bind(project)
    .bind(kind)
    .bind(user)
    .bind(stamp)
    .execute(&db.pool)
    .await
    .expect("insert item");

    let inserted: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM item WHERE id = $1")
            .bind(item)
            .fetch_one(&db.pool)
            .await
            .expect("read updated_at");
    assert_eq!(
        inserted, stamp,
        "the BEFORE UPDATE trigger leaves an explicit INSERT timestamp alone"
    );

    sqlx::query("UPDATE item SET title = 'after' WHERE id = $1")
        .bind(item)
        .execute(&db.pool)
        .await
        .expect("update item");
    let updated: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM item WHERE id = $1")
            .bind(item)
            .fetch_one(&db.pool)
            .await
            .expect("read updated_at");
    assert!(
        updated > inserted,
        "every UPDATE gets clock_timestamp(), which is what the cache cursor rides on"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn seed_is_idempotent() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    // `apply_migrations` already seeded once; this is the second and third pass.
    let first = db.store.seed_if_empty().await.expect("seed once more");
    let second = db.store.seed_if_empty().await.expect("and again");
    assert_eq!(first, second, "seeding twice yields the same app_user");
    assert_eq!(
        first,
        db.store.this_user(),
        "the store points at the seeded user"
    );

    assert_eq!(common::count(&db.pool, "app_user").await, 1, "one app_user");
    assert_eq!(
        common::count(&db.pool, "capability_tag").await,
        10,
        "the ten seeded capability tags of §5.10"
    );
    assert_eq!(
        common::count(&db.pool, "app_setting").await,
        25,
        "cache_refresh_seconds, cache_overlap_seconds and ANA-5's ten, plus the twelve ANA-2 §5.4 \
         defaults `0003_orchestration.sql` seeds — none of which collides with the twelve before \
         it, and all of which survive the second and third seed pass unchanged — plus the \
         `htui_target_version` row `apply_migrations` writes (MOD-40 plan D9; moved from 24), \
         which no seed pass touches"
    );
    // MOD-6 left this at zero because `agent.launch`'s shape was still ANA-4's to settle. It is
    // settled (§5.1, §5.3), so MOD-2 seeds the rows and this asserts they arrive exactly once.
    assert_eq!(
        common::count(&db.pool, "agent").await,
        3,
        "the ANA-4 §5.3 agent rows as MOD-2 amended them, and no duplicate from the second and \
         third seed passes"
    );

    let agents = db.store.agents().await.expect("the registry reads");
    let names: Vec<&str> = agents
        .iter()
        .map(|summary| summary.agent.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["agy", "claude", "claude-cli"],
        "every seed, ordered by name"
    );
    for summary in &agents {
        assert!(
            summary.on_box.is_none(),
            "{}: nothing is probed until milestone 5",
            summary.agent.name
        );
        assert!(summary.agent.enabled, "{}", summary.agent.name);
        assert_eq!(
            summary.agent.launch["command"].as_str().map(str::is_empty),
            Some(false),
            "{}: the launch row survives the JSONB round trip",
            summary.agent.name
        );
    }

    let seeded: bool = sqlx::query_scalar("SELECT bool_and(seeded) FROM capability_tag")
        .fetch_one(&db.pool)
        .await
        .expect("read capability_tag.seeded");
    assert!(seeded, "every seeded tag is marked as such");

    db.drop_db().await;
}

/// MOD-2 D88: a database that was seeded before a row existed gains it on the next pass, and an
/// operator's edit to a row that is already there survives.
///
/// The guard `seed_if_empty_as` used to carry was "insert nothing unless the `agent` table is
/// empty", which meant every box that had ever launched — which is every real box — would never
/// see a row added by a later milestone, and the feature would be verifiable only on a fresh
/// database. The guard is now `ON CONFLICT (name) DO NOTHING` alone, which is a *name*-keyed
/// top-up: it adds the names the table lacks and touches nothing it already holds. That is what
/// the second half asserts, because it is the reason the empty-table guard existed — a maintainer
/// who disabled a row has decided something, and a later seed pass must not undo it.
///
/// The price, accepted at CONFIRM: a row the maintainer *deleted* comes back. MOD-23's model is
/// `enabled = false`, not deletion, so the exposure is one row on a screen that can disable it.
#[tokio::test]
async fn a_seeded_database_gains_a_later_row_and_keeps_an_operator_edit() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    // The state every real box is in: seeded once, under an older `seed_rows` that knew two names,
    // and then edited. `enabled = false` is MOD-23's disable, written here in SQL because the
    // editor is not built yet.
    sqlx::query("DELETE FROM agent WHERE name = 'claude-cli'")
        .execute(&db.pool)
        .await
        .expect("wind the registry back to the two rows an older build seeded");
    sqlx::query("UPDATE agent SET enabled = false WHERE name = 'agy'")
        .execute(&db.pool)
        .await
        .expect("an operator disables a row");
    assert_eq!(
        common::count(&db.pool, "agent").await,
        2,
        "two rows, one off"
    );

    db.store.seed_if_empty().await.expect("the next seed pass");

    let agents = db.store.agents().await.expect("the registry reads");
    let names: Vec<&str> = agents
        .iter()
        .map(|summary| summary.agent.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["agy", "claude", "claude-cli"],
        "a non-empty table still gains the name it lacks"
    );

    let disabled = agents
        .iter()
        .find(|summary| summary.agent.name == "agy")
        .expect("the disabled row is still there");
    assert!(
        !disabled.agent.enabled,
        "`ON CONFLICT (name) DO NOTHING`: a row the operator edited is not re-seeded over"
    );

    // And it is still idempotent: a third pass adds nothing.
    db.store.seed_if_empty().await.expect("and once more");
    assert_eq!(common::count(&db.pool, "agent").await, 3);

    db.drop_db().await;
}

/// MOD-26 plan D7: the persona seeds are the agent loop's name-keyed top-up. A fresh database
/// holds `architect` and `reviewer`; a later pass brings back a deleted seed and leaves an
/// operator's edit to the other alone (`ON CONFLICT (name) DO NOTHING`).
#[tokio::test]
async fn seeding_adds_both_personas_once_and_keeps_an_operator_edit() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    let rows = |pool: sqlx::PgPool| async move {
        sqlx::query_as::<_, (String, String)>(
            "SELECT name, body FROM persona ORDER BY name COLLATE \"C\"",
        )
        .fetch_all(&pool)
        .await
        .expect("read the persona registry")
    };

    let seeded = rows(db.pool.clone()).await;
    assert_eq!(
        seeded
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["architect", "reviewer"],
        "a fresh database holds both seed personas"
    );

    sqlx::query("UPDATE persona SET body = 'You review, briefly.' WHERE name = 'reviewer'")
        .execute(&db.pool)
        .await
        .expect("an operator edits a seed");
    sqlx::query("DELETE FROM persona WHERE name = 'architect'")
        .execute(&db.pool)
        .await
        .expect("an operator deletes the other");

    db.store.seed_if_empty().await.expect("the next seed pass");
    let again = rows(db.pool.clone()).await;
    assert_eq!(
        again
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["architect", "reviewer"],
        "the deleted seed is back, and nothing is doubled"
    );
    assert_eq!(
        again[0].1, seeded[0].1,
        "the returning seed carries its file's body"
    );
    assert_eq!(
        again[1].1, "You review, briefly.",
        "`ON CONFLICT (name) DO NOTHING`: the operator's edit survives"
    );

    db.drop_db().await;
}

/// `R-USR-2`: one `app_user`, however many boxes and whatever their OS user is called.
///
/// The seed name is injected rather than pushed through `USERNAME` / `USER` so the case does not
/// mutate the process environment other tests read.
#[tokio::test]
async fn a_second_box_under_another_os_user_name_seeds_no_second_app_user() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    let seeded = db.store.this_user();

    // Another box: another `box.toml` identity, another hostname, another OS user name.
    let elsewhere = identity::Identity {
        box_id: BoxId::new(),
        hostname: format!("HTUI-TEST-{}", uuid::Uuid::now_v7().simple()),
    };
    let second = PgStore::connect(&db.url, &elsewhere)
        .await
        .expect("a second box connects to the same database")
        .store;
    let adopted = second
        .seed_if_empty_as("another-os-user")
        .await
        .expect("seed from the second box");

    assert_eq!(
        common::count(&db.pool, "app_user").await,
        1,
        "a differently named OS user must not seed a second app_user (R-USR-2)"
    );
    assert_eq!(
        adopted, seeded,
        "the second box adopts the row the first one seeded"
    );
    assert_eq!(
        second.this_user(),
        seeded,
        "and both stores answer the same app_user"
    );

    db.drop_db().await;
}

/// `R-USR-2` again, with the two first-ever connects *overlapping* rather than ordered.
///
/// `INSERT ... WHERE NOT EXISTS` does not serialise under READ COMMITTED: both statements read a
/// snapshot taken when they started, both find `app_user` empty and both insert, and two differing
/// names slip past `ON CONFLICT (name)`. `seed_if_empty_as` therefore locks `app_user` in SHARE
/// ROW EXCLUSIVE as its transaction's first statement. Each round empties `app_user` - and the
/// `box` rows that reference it - so both seeds race for a genuinely first-ever database; ten of
/// them, because one round losing the race is luck and ten is not.
#[tokio::test]
async fn two_overlapping_first_connects_seed_one_app_user() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let timeout = std::time::Duration::from_secs(10);
    let one = PgStore::connect_with(
        &db.url,
        &identity::Identity {
            box_id: BoxId::new(),
            hostname: format!("HTUI-TEST-{}", uuid::Uuid::now_v7().simple()),
        },
        timeout,
    )
    .await
    .expect("the first box connects")
    .store;
    let two = PgStore::connect_with(
        &db.url,
        &identity::Identity {
            box_id: BoxId::new(),
            hostname: format!("HTUI-TEST-{}", uuid::Uuid::now_v7().simple()),
        },
        timeout,
    )
    .await
    .expect("the second box connects")
    .store;

    for round in 0..10 {
        // `box.user_id` references `app_user`, so the boxes the two connects registered go first.
        sqlx::query("DELETE FROM box")
            .execute(&db.pool)
            .await
            .expect("clear box");
        sqlx::query("DELETE FROM app_user")
            .execute(&db.pool)
            .await
            .expect("clear app_user");

        let (first, second) = tokio::join!(
            one.seed_if_empty_as("os-user-one"),
            two.seed_if_empty_as("os-user-two"),
        );
        let first = first.expect("the first seed succeeds");
        let second = second.expect("the second seed succeeds");

        assert_eq!(
            common::count(&db.pool, "app_user").await,
            1,
            "round {round}: two overlapping first connects must seed one app_user (R-USR-2)"
        );
        assert_eq!(
            first, second,
            "round {round}: whichever seed lost the race adopts the row the other one wrote"
        );
    }

    one.pool().close().await;
    two.pool().close().await;
    db.drop_db().await;
}

/// MOD-7 D2: the row is keyed on the `box.toml` id. The hostname is a display field, so a second
/// id on the same hostname is a second box rather than the first one's row (ANA-16 C4).
#[tokio::test]
async fn register_box_keys_on_the_id() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let hostname = format!("HTUI-TEST-{}", uuid::Uuid::now_v7().simple());
    let first = identity::Identity {
        box_id: BoxId::new(),
        hostname: hostname.clone(),
    };
    let registered = db
        .store
        .register_box(&first, None)
        .await
        .expect("register a new box");
    assert_eq!(
        registered,
        Registration::New,
        "a first registration inserts"
    );
    assert_eq!(
        registered.box_id(first.box_id),
        first.box_id,
        "and keeps its id"
    );

    let second = identity::Identity {
        box_id: BoxId::new(),
        hostname: hostname.clone(),
    };
    let again = db
        .store
        .register_box(&second, None)
        .await
        .expect("register the same hostname under another id");
    assert_eq!(
        again,
        Registration::New,
        "another id is another box, whatever its hostname (MOD-7 D2)"
    );

    assert_eq!(
        db.store.identity().box_id,
        db.store.this_box(),
        "the store carries the id registration answered"
    );

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM box WHERE hostname = $1")
        .bind(&hostname)
        .fetch_one(&db.pool)
        .await
        .expect("count box rows");
    assert_eq!(rows, 2, "two ids on one hostname are two rows");

    let family: String = sqlx::query("SELECT os_family, htui_version FROM box WHERE id = $1")
        .bind(second.box_id.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read the box row")
        .get("os_family");
    assert!(
        OsFamily::ALL.iter().any(|f| f.as_str() == family),
        "os_family is one of the three CHECK values, got {family}"
    );

    // The caller-side half: `box.toml` is rewritten to the id registration answered.
    let root = tempfile::tempdir().expect("a temp config root");
    let answered = again.box_id(second.box_id);
    identity::store(
        root.path(),
        &identity::Identity {
            box_id: answered,
            hostname,
        },
    )
    .expect("write box.toml");
    assert_eq!(
        identity::load_or_mint(root.path())
            .expect("read box.toml")
            .box_id,
        answered,
        "the answered id is what the next launch reads"
    );

    db.drop_db().await;
}

/// MOD-7 blueprint §3.1: `0005_box_identity.sql` drops the hostname key and adds three columns,
/// the two hashes guarded by a lowercase-hex `CHECK`.
#[tokio::test]
async fn the_0005_migration_drops_the_hostname_key_and_adds_three_columns() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let key: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_constraint WHERE conname = 'box_user_id_hostname_key'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read pg_constraint");
    assert_eq!(key, 0, "the (user_id, hostname) key is gone");

    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'box' \
           AND column_name IN ('machine_fingerprint', 'edit_version', 'probe_spec_digest') \
         ORDER BY 1",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read information_schema.columns");
    assert_eq!(
        columns,
        vec!["edit_version", "machine_fingerprint", "probe_spec_digest"],
        "the three MOD-7 columns exist"
    );

    for column in ["machine_fingerprint", "probe_spec_digest"] {
        let err = sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE box SET {column} = 'RAW' WHERE id = $1"
        )))
        .bind(db.store.this_box().as_uuid())
        .execute(&db.pool)
        .await
        .expect_err("a value that is not 64 lowercase hex is refused");
        let code = err
            .as_database_error()
            .and_then(|e| e.code())
            .map(|c| c.into_owned());
        assert_eq!(
            code.as_deref(),
            Some("23514"),
            "{column}'s CHECK refuses it: {err}"
        );
    }

    let edit_version: i32 = sqlx::query_scalar("SELECT edit_version FROM box WHERE id = $1")
        .bind(db.store.this_box().as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read edit_version");
    assert_eq!(edit_version, 0, "edit_version defaults to 0");

    db.drop_db().await;
}

#[cfg(feature = "demo")]
#[tokio::test]
async fn load_demo_round_trips_a_count_per_table() {
    let Some(mut db) = common::fresh_db().await else {
        return;
    };

    // `fresh_db` already seeded one `app_user` and registered this box, so the assertion is on the
    // delta the fixture adds, table by table - which is what "a count per table" has to mean once
    // seeding runs first (§5.10 before §G).
    let tables = [
        "app_user",
        "box",
        "workspace",
        "project",
        "workspace_project",
        "agent",
        "persona",
        "step_graph",
        "step_graph_phase",
        "prompt_template",
        "item_kind",
        "item_key_counter",
        "item",
        "item_revision",
        "item_link",
        "item_note",
        "document",
        "run",
        "run_step",
        "session_event",
        "requirement_spec",
        "requirement_area",
        "requirement_key_counter",
        "requirement",
        "requirement_revision",
        "item_requirement",
    ];
    let mut before = Vec::new();
    for table in tables {
        before.push(common::count(&db.pool, table).await);
    }

    let data = htui_core::fixtures::demo_data();
    db.store.load_demo(&data).await.expect("load the fixture");

    let expected: [(&str, usize); 26] = [
        ("app_user", data.users.len()),
        ("box", data.boxes.len()),
        ("workspace", data.workspaces.len()),
        ("project", data.projects.len()),
        ("workspace_project", data.workspace_projects.len()),
        ("agent", data.agents.len()),
        ("persona", data.personas.len()),
        ("step_graph", data.graphs.len()),
        ("step_graph_phase", data.phases.len()),
        ("prompt_template", data.templates.len()),
        ("item_kind", data.kinds.len()),
        ("item_key_counter", data.item_key_counter.len()),
        ("item", data.items.len()),
        ("item_revision", data.revisions.len()),
        ("item_link", data.links.len()),
        ("item_note", data.notes.len()),
        ("document", data.documents.len()),
        ("run", data.runs.len()),
        ("run_step", data.steps.len()),
        ("session_event", data.events.len()),
        ("requirement_spec", data.requirement_specs.len()),
        ("requirement_area", data.requirement_areas.len()),
        (
            "requirement_key_counter",
            data.requirement_key_counter.len(),
        ),
        ("requirement", data.requirements.len()),
        ("requirement_revision", data.requirement_revisions.len()),
        ("item_requirement", data.item_requirements.len()),
    ];
    // MOD-38 blueprint §8: the requirement set's own sizes, pinned so a fixture edit that drops a
    // row is loud here and not only a smaller delta. The citation tombstone is a row too.
    assert_eq!(
        [
            data.requirement_specs.len(),
            data.requirement_areas.len(),
            data.requirement_key_counter.len(),
            data.requirements.len(),
            data.requirement_revisions.len(),
            data.item_requirements.len(),
        ],
        [1, 2, 2, 3, 4, 5],
        "spec, areas, counters, requirements, revisions, citations"
    );

    for (i, (table, len)) in expected.iter().enumerate() {
        let after = common::count(&db.pool, table).await;
        let expected_rows = i64::try_from(*len).expect("a fixture vector fits in i64");

        if *table == "agent" || *table == "persona" {
            // `agent` and `persona` are the two tables the fixture *replaces* rather than adds
            // to: `seed_if_empty_as` has already put the seed rows there (MOD-2's agents, MOD-26
            // D7's `reviewer` and `architect`), and the fixture carries the same names under its
            // own ids, so `load_demo` deletes them by name first. The delta is therefore zero and
            // the absolute count is what carries meaning.
            assert_eq!(before[i], expected_rows, "the seed put `{table}`'s rows in");
            assert_eq!(
                after, expected_rows,
                "`{table}` holds the fixture's rows, not the seed's plus the fixture's"
            );
            continue;
        }

        assert_eq!(
            after - before[i],
            expected_rows,
            "`{table}` holds one row per DemoData entry"
        );
    }

    assert_eq!(
        db.store.this_box(),
        data.this_box.expect("the fixture names a box"),
        "load_demo points the store at the fixture's box, as MemStore::from_demo does"
    );
    assert_eq!(
        db.store.this_user(),
        data.users.first().expect("the fixture has a user").id,
        "and at the fixture's author"
    );

    // The generated column is computed, never inserted (blueprint C.5).
    let mismatched: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM item WHERE key <> key_prefix || '-' || key_number::text",
    )
    .fetch_one(&db.pool)
    .await
    .expect("check the generated key column");
    assert_eq!(mismatched, 0, "item.key is GENERATED ALWAYS");

    db.drop_db().await;
}

#[test]
fn box_toml_mint_is_stable_across_two_reads() {
    let root = tempfile::tempdir().expect("a temp config root");

    let first = identity::load_or_mint(root.path()).expect("mint box.toml");
    let second = identity::load_or_mint(root.path()).expect("read it back");

    assert_eq!(
        first, second,
        "the box identity is minted once and never re-minted (D6)"
    );
    assert!(
        root.path().join("box.toml").is_file(),
        "minting writes the file"
    );
    assert!(
        !first.hostname.is_empty(),
        "the hostname is recorded alongside the id"
    );
}

#[test]
fn db_fingerprint_is_stable_and_credential_free() {
    let a = identity::db_fingerprint("postgres://alice:secret@db.example:5433/htui");
    let b = identity::db_fingerprint("postgres://bob:other@db.example:5433/htui");
    let c = identity::db_fingerprint("postgres://alice:secret@db.example:5433/other");

    assert_eq!(a, b, "only host, port and dbname are hashed (ANA-9 §4.4)");
    assert_ne!(
        a, c,
        "a different database gets a different cache directory"
    );
    assert_eq!(a.len(), 64, "lowercase hex sha256");
    assert!(
        a.chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_uppercase())
    );
    assert!(
        !identity::db_fingerprint("not a dsn").is_empty(),
        "a DSN that does not parse still gets a stable directory instead of a panic"
    );
}

/// Blueprint H.8 and A.6: the derives `htui-core`'s `sqlx` feature adds are a run-time contract,
/// not a compile-time one - a missing `#[sqlx(type_name = "text")]` compiles and then fails to
/// decode. T2 leans on both, so T1 proves them.
#[cfg(feature = "demo")]
#[tokio::test]
async fn the_core_sqlx_derives_decode_against_text_and_uuid_columns() {
    use htui_core::model::{ItemId, Status};

    let Some(db) = common::demo_db().await else {
        return;
    };

    let statuses: Vec<Status> =
        sqlx::query_scalar("SELECT status FROM item ORDER BY key_prefix, key_number")
            .fetch_all(&db.pool)
            .await
            .expect("a str_enum! enum decodes from a TEXT column (H.8)");
    assert!(!statuses.is_empty(), "the fixture has items");

    let ids: Vec<ItemId> = sqlx::query_scalar("SELECT id FROM item")
        .fetch_all(&db.pool)
        .await
        .expect("a transparent ID newtype decodes from a UUID column (A.6)");
    assert_eq!(ids.len(), statuses.len());

    db.drop_db().await;
}

/// `PgStore::connect` takes its identity from the caller and does no file I/O of its own, so a test
/// never writes to the user's real config directory.
#[tokio::test]
async fn connect_writes_no_files_of_its_own() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let mut entries: Vec<String> = std::fs::read_dir(&db.config_root)
        .expect("the throwaway config root exists")
        .map(|e| {
            e.expect("a readable entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        vec![identity::BOX_FILE.to_owned()],
        "only the box.toml the harness minted; connect wrote nothing"
    );
    assert_eq!(
        db.store.this_box(),
        db.identity.box_id,
        "a first registration keeps the caller's id"
    );

    let root = db.config_root.clone();
    db.drop_db().await;
    assert!(!root.exists(), "drop_db removes the throwaway config root");
}
