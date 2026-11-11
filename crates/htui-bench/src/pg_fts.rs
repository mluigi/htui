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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CorpusId, DocKind};

    #[test]
    fn schema_names_are_checked() {
        assert!(is_valid_schema(SCHEMA));
        assert!(is_valid_schema("_x"));
        assert!(is_valid_schema("htui_bench_t_0190f5e2a1b27c3d8e9f0a1b2c3d4e5f"));
        assert!(is_valid_schema(&format!("a{}", "b".repeat(62))));
        assert!(!is_valid_schema(&format!("a{}", "b".repeat(63))));
        for bad in ["a;b", "Upper", "", "1abc", "a-b", "a b", "a\"b", "é"] {
            assert!(!is_valid_schema(bad), "{bad:?}");
        }
    }

    #[test]
    fn search_sql_uses_websearch_and_rewrites_only_for_or() {
        for variant in [LVariant::And, LVariant::AndLen] {
            let sql = search_sql("htui_bench", variant);
            assert!(sql.contains("websearch_to_tsquery('english', $1)"), "{sql}");
            assert!(!sql.contains("replace("), "{sql}");
            assert!(sql.contains("FROM htui_bench.doc d"), "{sql}");
            assert!(sql.contains("ts_rank_cd(d.tsv, w.q, $2)"), "{sql}");
            assert!(sql.contains("ORDER BY score DESC, d.ord"), "{sql}");
            assert!(sql.contains("LIMIT $3"), "{sql}");
        }
        for variant in [LVariant::Or, LVariant::OrLen] {
            let sql = search_sql("htui_bench", variant);
            assert!(
                sql.contains(
                    "replace(websearch_to_tsquery('english', $1)::text, ' & ', ' | ')::tsquery"
                ),
                "{sql}"
            );
        }
    }

    fn doc(n: usize, text: String) -> CorpusDoc {
        CorpusDoc {
            id: CorpusId::new(format!("D{n}")),
            kind: DocKind::OpenItem,
            key: format!("D{n}"),
            title: String::new(),
            text,
            source: format!("fixture:{n}"),
        }
    }

    fn ids(ranked: &[Ranked]) -> Vec<&str> {
        ranked.iter().map(|r| r.id.as_str()).collect()
    }

    fn sorted(ranked: &[Ranked]) -> Vec<&str> {
        let mut v = ids(ranked);
        v.sort_unstable();
        v
    }

    async fn exercise(pool: &sqlx::PgPool, schema: &str) -> anyhow::Result<()> {
        let filler: Vec<String> = (0..2_000).map(|i| format!("filler{i}")).collect();
        let long = format!("{} {}", vec!["alpha beta"; 50].join(" "), filler.join(" "));
        let docs = [
            doc(1, "alpha beta".to_owned()),
            doc(2, "alpha gamma".to_owned()),
            doc(3, "beta gamma delta".to_owned()),
            doc(4, long),
            doc(5, "unrelated text".to_owned()),
        ];
        load(pool, schema, &docs).await?;

        let and = search(pool, schema, LVariant::And, "alpha beta", 10).await?;
        assert_eq!(sorted(&and), vec!["D1", "D4"]);
        let and_len = search(pool, schema, LVariant::AndLen, "alpha beta", 10).await?;
        assert_eq!(sorted(&and_len), vec!["D1", "D4"]);

        let or = search(pool, schema, LVariant::Or, "alpha beta", 10).await?;
        assert_eq!(sorted(&or), vec!["D1", "D2", "D3", "D4"]);
        assert_eq!(ids(&or)[0], "D4", "{or:?}");
        let or_len = search(pool, schema, LVariant::OrLen, "alpha beta", 10).await?;
        assert_eq!(sorted(&or_len), vec!["D1", "D2", "D3", "D4"]);
        assert_eq!(ids(&or_len)[0], "D1", "{or_len:?}");
        assert!(or.windows(2).all(|w| w[0].score >= w[1].score));

        let limited = search(pool, schema, LVariant::Or, "alpha beta", 2).await?;
        assert_eq!(limited.len(), 2);

        for variant in [LVariant::And, LVariant::Or, LVariant::AndLen, LVariant::OrLen] {
            assert_eq!(search(pool, schema, variant, "the and", 10).await?, vec![]);
        }

        // Reloading replaces the schema rather than appending.
        load(pool, schema, &docs[..1]).await?;
        let or = search(pool, schema, LVariant::Or, "alpha beta", 10).await?;
        assert_eq!(ids(&or), vec!["D1"]);
        Ok(())
    }

    #[tokio::test]
    async fn pg_fts_variants_on_a_five_document_corpus() {
        let Ok(url) = std::env::var("HTUI_TEST_DATABASE_URL") else {
            eprintln!("skipped: HTUI_TEST_DATABASE_URL not set");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        let version = server_version(&pool).await.expect("version");
        assert!(version.starts_with("PostgreSQL"), "{version}");
        let schema = format!("htui_bench_t_{}", uuid::Uuid::now_v7().simple());
        assert!(is_valid_schema(&schema));
        let outcome = exercise(&pool, &schema).await;
        drop_schema(&pool, &schema).await.expect("drop");
        let gone: bool = sqlx::query_scalar(
            "SELECT NOT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $1)",
        )
        .bind(&schema)
        .fetch_one(&pool)
        .await
        .expect("pg_namespace");
        assert!(gone, "{schema} dropped");
        outcome.expect("the five-document corpus");
    }

    #[tokio::test]
    async fn invalid_schema_names_are_refused_before_any_sql() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://nobody@127.0.0.1:1/none")
            .expect("lazy pool");
        assert!(load(&pool, "a;b", &[]).await.is_err());
        assert!(search(&pool, "Upper", LVariant::And, "x", 1).await.is_err());
        let err = drop_schema(&pool, "").await.expect_err("refused");
        assert!(err.to_string().contains("schema"), "{err}");
    }

    #[tokio::test]
    async fn connect_errors_never_echo_the_dsn() {
        let dsn = "postgres://bench_user:s3cr3t-pw@127.0.0.1:1/benchdb";
        let err = connect(dsn).await.expect_err("nothing listens on port 1");
        let shown = format!("{err:#} {err:?}");
        assert!(!shown.contains("s3cr3t-pw"), "{shown}");
        assert!(!shown.contains(dsn), "{shown}");
        let err = connect("not a url :// s3cr3t-pw").await.expect_err("malformed");
        let shown = format!("{err:#} {err:?}");
        assert!(!shown.contains("s3cr3t-pw"), "{shown}");
    }
}
