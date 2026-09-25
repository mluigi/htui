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
use htui_store::{MIGRATOR, MigrationState, PgStore, identity};
use sqlx::Row as _;

/// The 39 tables, in creation order: blueprint B.1's 32, then the one `0003_orchestration.sql`
/// adds (`run_step_tree`, ANA-2 §9), then the six of `0005_requirements.sql` (ANA-11 §5).
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
    // 0005_requirements.sql (ANA-11 §5, MOD-38), in its creation order.
    "requirement_spec",
    "requirement_area",
    "requirement_key_counter",
    "requirement",
    "requirement_revision",
    "item_requirement",
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
        vec![1, 2, 3, 4, 5],
        "0001_init.sql, MOD-2 milestone 5's 0002_agent_probe.sql, MOD-4 milestone 1's \
         0003_orchestration.sql, MOD-4 milestone 4's 0004_max_agents_per_run_default.sql and \
         MOD-38's 0005_requirements.sql, in ordinal order"
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
        39,
        "blueprint B.1 lists 32 tables (ANA-9 §3's prose count of 30 is wrong, H.1), \
         0003_orchestration.sql adds run_step_tree and 0005_requirements.sql adds ANA-11 §5's six"
    );
    // `_sqlx_migrations` is the only extra table sqlx adds.
    assert_eq!(
        present.len(),
        TABLES.len() + 1,
        "the migrations create the 39 tables of B.1 as amended by ANA-2 §9 and ANA-11 §5 and \
         nothing else, got {present:?}"
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

/// The twenty-five `COMMENT ON COLUMN` texts of `0002_agent_probe.sql` and
/// `0003_orchestration.sql`, verbatim.
///
/// `agent.name` is ANA-4 §9 as amended by plan D43 (its `COMMENT ... IS NULL` would have cleared a
/// comment `0001_init.sql` never wrote); the next five are ANA-5 §9 copied from
/// `docs/ANA-5.md:2153-2181`; the last nineteen are ANA-2 §9 (`docs/ANA-2.md:1862-1999`). They
/// live here as literals on purpose: this test is the guard against a paraphrase drifting into a
/// forward-only migration that cannot be edited afterwards.
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
        "run_step",
        "prompt_digest",
        "ANA-5 4.7: sha256, lowercase hex, over the canonical assembled prompt TEXT as sent - LF \
         normalised, BOM stripped, one trailing LF, scrubbed before hashing. Not over the payload \
         and not over sections[]. An audit field, never a replay key (ANA-2 4.9).",
    ),
    (
        "run_step",
        "trim_record",
        "ANA-5 5.1: {v, template, budget, budget_source, reserve, target, estimator, \
         estimated_before, estimated_after, sections[], excerpts, notes}. Canonical; the prompt \
         payload sections[] array is its abridged projection. Written at stage 3 by \
         set_step_prompt, before the session starts.",
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

    for (table, column, expected) in ANA_COLUMN_COMMENTS {
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
            "{table}.{column}'s comment is the ANA text byte for byte"
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
    // twenty-five contracts the three ANAs wrote and no half-finished twenty-sixth.
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
        .map(|(table, column, _)| ((*table).to_owned(), (*column).to_owned()))
        .collect();
    expected.sort();
    assert_eq!(
        commented, expected,
        "exactly the twenty-five commented columns, and no others"
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

/// MOD-38 (plan D8): every item closed before 0005 closed through the old `-> closed` edges, which
/// only a finished item took, so 0005 backfills `done` before it adds the iff constraint. Staged
/// like the 0004 case: `run_to(4)`, plant a closed row, then the plain `run` applies 0005.
#[tokio::test]
async fn closed_rows_backfill_to_done() {
    let Some(db) = common::bare_db().await else {
        return;
    };

    MIGRATOR
        .run_to(4, &db.pool)
        .await
        .expect("apply 0001 through 0004");
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
             created_by) VALUES ($1, $2, $3, 'RES', $4, 'r', $5, $6)",
        )
        .bind(id)
        .bind(project)
        .bind(kind)
        .bind(number)
        .bind(status)
        .bind(user)
        .execute(&db.pool)
        .await
        .expect("insert a pre-0005 item");
        planted.push((id, status));
    }

    MIGRATOR.run(&db.pool).await.expect("apply 0005");

    for (id, status) in planted {
        let resolution: Option<String> =
            sqlx::query_scalar("SELECT resolution FROM item WHERE id = $1")
                .bind(id)
                .fetch_one(&db.pool)
                .await
                .expect("read item.resolution");
        let expected = (status == "closed").then(|| "done".to_owned());
        assert_eq!(
            resolution, expected,
            "a pre-0005 `{status}` row backfills to {expected:?}"
        );
    }

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
        MigrationState::Pending(5),
        "five embedded migrations, none applied"
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
        24,
        "cache_refresh_seconds, cache_overlap_seconds and ANA-5's ten, plus the twelve ANA-2 §5.4 \
         defaults `0003_orchestration.sql` seeds — none of which collides with the twelve before \
         it, and all of which survive the second and third seed pass unchanged"
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

#[tokio::test]
async fn register_box_upserts_and_adopts() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let hostname = format!("HTUI-TEST-{}", uuid::Uuid::now_v7().simple());
    let first = identity::Identity {
        box_id: BoxId::new(),
        hostname: hostname.clone(),
    };
    let adopted = db
        .store
        .register_box(&first)
        .await
        .expect("register a new box");
    assert_eq!(adopted, first.box_id, "a first registration keeps its id");

    let second = identity::Identity {
        box_id: BoxId::new(),
        hostname: hostname.clone(),
    };
    let again = db
        .store
        .register_box(&second)
        .await
        .expect("register the same hostname under another id");
    assert_eq!(
        again, first.box_id,
        "the hostname already has a row, so its id wins (D6 adopt-DB-id)"
    );

    assert_eq!(
        db.store.identity().box_id,
        db.store.this_box(),
        "the store carries the adopted id, for the caller to write back"
    );

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM box WHERE hostname = $1")
        .bind(&hostname)
        .fetch_one(&db.pool)
        .await
        .expect("count box rows");
    assert_eq!(rows, 1, "the upsert never adds a second row for a hostname");

    let family: String = sqlx::query("SELECT os_family, htui_version FROM box WHERE id = $1")
        .bind(adopted.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read the box row")
        .get("os_family");
    assert!(
        OsFamily::ALL.iter().any(|f| f.as_str() == family),
        "os_family is one of the three CHECK values, got {family}"
    );

    // The caller-side half of D6: `box.toml` is rewritten to the id the database handed back.
    let root = tempfile::tempdir().expect("a temp config root");
    identity::store(
        root.path(),
        &identity::Identity {
            box_id: again,
            hostname,
        },
    )
    .expect("write box.toml");
    assert_eq!(
        identity::load_or_mint(root.path())
            .expect("read box.toml")
            .box_id,
        again,
        "the adopted id is what the next launch reads"
    );

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
    ];
    let mut before = Vec::new();
    for table in tables {
        before.push(common::count(&db.pool, table).await);
    }

    let data = htui_core::fixtures::demo_data();
    db.store.load_demo(&data).await.expect("load the fixture");

    let expected: [(&str, usize); 19] = [
        ("app_user", data.users.len()),
        ("box", data.boxes.len()),
        ("workspace", data.workspaces.len()),
        ("project", data.projects.len()),
        ("workspace_project", data.workspace_projects.len()),
        ("agent", data.agents.len()),
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
    ];

    for (i, (table, len)) in expected.iter().enumerate() {
        let after = common::count(&db.pool, table).await;
        let expected_rows = i64::try_from(*len).expect("a fixture vector fits in i64");

        if *table == "agent" {
            // `agent` is the one table the fixture *replaces* rather than adds to: since MOD-2,
            // `seed_if_empty_as` has already put `claude` and `agy` there, and the fixture carries
            // the same two names under its own ids, so `load_demo` deletes them by name first. The
            // delta is therefore zero and the absolute count is what carries meaning.
            assert_eq!(
                before[i], expected_rows,
                "the seed put the two §5.3 rows in"
            );
            assert_eq!(
                after, expected_rows,
                "`agent` holds the fixture's rows, not the seed's plus the fixture's"
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
