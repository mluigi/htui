//! The Postgres full-text baselines L-and, L-or, L-and-len and L-or-len (MOD-162 T4, P5):
//! one throwaway schema per run, `websearch_to_tsquery('english', ..)` and `ts_rank_cd`.

use crate::types::{CorpusDoc, LVariant, Ranked};

/// The run's schema.
pub const SCHEMA: &str = "htui_bench";

/// `^[a-z_][a-z0-9_]{0,62}$`: the schema name is interpolated into SQL, so it is checked first.
pub fn is_valid_schema(name: &str) -> bool {
    let _ = name;
    false
}

/// The search statement for `variant` (pure; unit-tested). Panics never; callers check the schema.
pub fn search_sql(schema: &str, variant: LVariant) -> String {
    let _ = (schema, variant);
    String::new()
}

/// A small pool (`PgPoolOptions::new().max_connections(2)`); the error never contains the DSN.
pub async fn connect(url: &str) -> anyhow::Result<sqlx::PgPool> {
    let _ = url;
    anyhow::bail!("pg_fts::connect: not implemented yet (MOD-162 T4)")
}

/// `SELECT version()`.
pub async fn server_version(pool: &sqlx::PgPool) -> anyhow::Result<String> {
    let _ = pool;
    anyhow::bail!("pg_fts::server_version: not implemented yet (MOD-162 T4)")
}

/// Drops `schema` if it exists, creates it and its table, loads one row per doc, ANALYZEs.
pub async fn load(pool: &sqlx::PgPool, schema: &str, docs: &[CorpusDoc]) -> anyhow::Result<()> {
    let _ = (pool, schema, docs);
    anyhow::bail!("pg_fts::load: not implemented yet (MOD-162 T4)")
}

/// Top `limit` docs for `text` under `variant`, ties by doc order.
pub async fn search(
    pool: &sqlx::PgPool,
    schema: &str,
    variant: LVariant,
    text: &str,
    limit: usize,
) -> anyhow::Result<Vec<Ranked>> {
    let _ = (pool, schema, variant, text, limit);
    anyhow::bail!("pg_fts::search: not implemented yet (MOD-162 T4)")
}

/// `DROP SCHEMA IF EXISTS <schema> CASCADE`.
pub async fn drop_schema(pool: &sqlx::PgPool, schema: &str) -> anyhow::Result<()> {
    let _ = (pool, schema);
    anyhow::bail!("pg_fts::drop_schema: not implemented yet (MOD-162 T4)")
}
