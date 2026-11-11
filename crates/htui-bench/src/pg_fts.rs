//! The Postgres full-text baselines L-and, L-or, L-and-len and L-or-len (MOD-162 T4, P5):
//! one throwaway schema per run, `websearch_to_tsquery('english', ..)` and `ts_rank_cd`.
//!
//! The schema name is interpolated into SQL (sqlx 0.9 takes only `&'static str` or
//! [`sqlx::AssertSqlSafe`]), so every function that builds SQL checks it with
//! [`is_valid_schema`] first and refuses an invalid name before talking to the server.

use std::time::Duration;

use anyhow::Context as _;
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool, Row as _};

use crate::types::{CorpusDoc, CorpusId, LVariant, Ranked};

/// The run's schema.
pub const SCHEMA: &str = "htui_bench";

/// How long [`connect`] waits for a first connection (sqlx retries a refused one until then).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// `^[a-z_][a-z0-9_]{0,62}$`: the schema name is interpolated into SQL, so it is checked first.
pub fn is_valid_schema(name: &str) -> bool {
    let bytes = name.as_bytes();
    match bytes.split_first() {
        Some((first, rest)) => {
            (first.is_ascii_lowercase() || *first == b'_')
                && rest.len() <= 62
                && rest
                    .iter()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
        }
        None => false,
    }
}

/// `Err` unless [`is_valid_schema`].
fn checked(schema: &str) -> anyhow::Result<&str> {
    anyhow::ensure!(
        is_valid_schema(schema),
        "invalid schema name {schema:?}: expected ^[a-z_][a-z0-9_]{{0,62}}$"
    );
    Ok(schema)
}

/// The search statement for `variant` (pure; unit-tested). Panics never; callers check the schema.
pub fn search_sql(schema: &str, variant: LVariant) -> String {
    let tsquery = if variant.is_or() {
        "replace(websearch_to_tsquery('english', $1)::text, ' & ', ' | ')::tsquery"
    } else {
        "websearch_to_tsquery('english', $1)"
    };
    format!(
        "SELECT d.id, ts_rank_cd(d.tsv, w.q, $2)::float8 AS score\n  \
         FROM {schema}.doc d\n \
         CROSS JOIN (SELECT {tsquery} AS q) w\n \
         WHERE d.tsv @@ w.q\n \
         ORDER BY score DESC, d.ord\n \
         LIMIT $3"
    )
}

/// A small pool (`PgPoolOptions::new().max_connections(2)`); the error never contains the DSN.
pub async fn connect(url: &str) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(CONNECT_TIMEOUT)
        .connect(url)
        .await
        .map_err(|err| connect_error(&err, url))
}

/// A connect error with no DSN in it. sqlx's configuration errors can quote the URL, so they
/// collapse to a fixed message; any other message is redacted defensively. The sqlx error is not
/// kept as a source, so `{:?}` cannot reach it either.
fn connect_error(err: &sqlx::Error, url: &str) -> anyhow::Error {
    let detail = match err {
        sqlx::Error::Configuration(_) => {
            "the URL is not a valid Postgres connection string".to_owned()
        }
        other => {
            let mut shown = other.to_string();
            if !url.is_empty() {
                shown = shown.replace(url, "<redacted>");
            }
            if let Some(password) = password_of(url) {
                shown = shown.replace(password, "<redacted>");
            }
            shown
        }
    };
    anyhow::anyhow!("connecting to the benchmark Postgres failed: {detail}")
}

/// The password part of `scheme://user:password@host...`, when there is one.
fn password_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let userinfo = rest.split_once('@')?.0;
    let password = userinfo.split_once(':')?.1;
    (!password.is_empty()).then_some(password)
}

/// `SELECT version()`.
pub async fn server_version(pool: &PgPool) -> anyhow::Result<String> {
    sqlx::query_scalar("SELECT version()")
        .fetch_one(pool)
        .await
        .context("SELECT version()")
}

/// One statement that names the (checked) schema.
async fn execute(pool: &PgPool, sql: String) -> anyhow::Result<()> {
    sqlx::query(AssertSqlSafe(sql.clone()))
        .execute(pool)
        .await
        .with_context(|| sql)?;
    Ok(())
}

/// Drops `schema` if it exists, creates it and its table, loads one row per doc, ANALYZEs.
pub async fn load(pool: &PgPool, schema: &str, docs: &[CorpusDoc]) -> anyhow::Result<()> {
    let s = checked(schema)?;
    execute(pool, format!("DROP SCHEMA IF EXISTS {s} CASCADE")).await?;
    execute(pool, format!("CREATE SCHEMA {s}")).await?;
    execute(
        pool,
        format!(
            "CREATE TABLE {s}.doc (\n    \
             ord  integer PRIMARY KEY,\n    \
             id   text    NOT NULL UNIQUE,\n    \
             body text    NOT NULL,\n    \
             tsv  tsvector GENERATED ALWAYS AS (to_tsvector('english', body)) STORED\n)"
        ),
    )
    .await?;

    let ords = (0..docs.len())
        .map(i32::try_from)
        .collect::<Result<Vec<i32>, _>>()
        .context("more documents than an int4 ord holds")?;
    let ids: Vec<String> = docs.iter().map(|doc| doc.id.as_str().to_owned()).collect();
    let bodies: Vec<String> = docs.iter().map(|doc| doc.text.clone()).collect();
    let insert = format!(
        "INSERT INTO {s}.doc (ord, id, body) SELECT * FROM UNNEST($1::int4[], $2::text[], $3::text[])"
    );
    sqlx::query(AssertSqlSafe(insert))
        .bind(ords)
        .bind(ids)
        .bind(bodies)
        .execute(pool)
        .await
        .with_context(|| format!("loading {} documents into {s}.doc", docs.len()))?;

    execute(
        pool,
        format!("CREATE INDEX doc_tsv ON {s}.doc USING gin (tsv)"),
    )
    .await?;
    execute(pool, format!("ANALYZE {s}.doc")).await
}

/// Top `limit` docs for `text` under `variant`, ties by doc order.
pub async fn search(
    pool: &PgPool,
    schema: &str,
    variant: LVariant,
    text: &str,
    limit: usize,
) -> anyhow::Result<Vec<Ranked>> {
    let s = checked(schema)?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = sqlx::query(AssertSqlSafe(search_sql(s, variant)))
        .bind(text)
        .bind(variant.normalization())
        .bind(limit)
        .fetch_all(pool)
        .await
        .with_context(|| format!("{variant:?} search in {s}"))?;
    rows.iter()
        .map(|row| {
            Ok(Ranked {
                id: CorpusId::new(row.try_get::<String, _>("id")?),
                score: row.try_get::<f64, _>("score")?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .with_context(|| format!("{variant:?} search row in {s}"))
}

/// `DROP SCHEMA IF EXISTS <schema> CASCADE`.
pub async fn drop_schema(pool: &PgPool, schema: &str) -> anyhow::Result<()> {
    let s = checked(schema)?;
    execute(pool, format!("DROP SCHEMA IF EXISTS {s} CASCADE")).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CorpusId, DocKind};

    #[test]
    fn schema_names_are_checked() {
        assert!(is_valid_schema(SCHEMA));
        assert!(is_valid_schema("_x"));
        assert!(is_valid_schema(
            "htui_bench_t_0190f5e2a1b27c3d8e9f0a1b2c3d4e5f"
        ));
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

    async fn exercise(pool: &PgPool, schema: &str) -> anyhow::Result<()> {
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

        // Single-match ties (D2 `alpha gamma`, D3 `beta gamma delta`) keep doc order.
        let tail: Vec<&str> = ids(&or)
            .into_iter()
            .filter(|id| ["D2", "D3"].contains(id))
            .collect();
        assert_eq!(tail, vec!["D2", "D3"], "{or:?}");

        // websearch syntax (phrase, negation, quotes, `or`) survives the `|` rewrite's re-parse.
        for variant in [
            LVariant::And,
            LVariant::Or,
            LVariant::AndLen,
            LVariant::OrLen,
        ] {
            search(
                pool,
                schema,
                variant,
                "\"alpha beta\" -gamma or o'brien's",
                10,
            )
            .await?;
        }
        let negated = search(pool, schema, LVariant::And, "alpha -gamma", 10).await?;
        assert_eq!(sorted(&negated), vec!["D1", "D4"]);
        // P5's rewrite turns every `&` into `|`, the one in front of a negation too: L-or reads
        // `alpha -gamma` as `alpha | !gamma`, so a doc with neither word matches. Pinned, not fixed.
        let negated = search(pool, schema, LVariant::Or, "alpha -gamma", 10).await?;
        assert_eq!(sorted(&negated), vec!["D1", "D2", "D4", "D5"]);

        let limited = search(pool, schema, LVariant::Or, "alpha beta", 2).await?;
        assert_eq!(limited.len(), 2);

        for variant in [
            LVariant::And,
            LVariant::Or,
            LVariant::AndLen,
            LVariant::OrLen,
        ] {
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
        let gone: bool =
            sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $1)")
                .bind(&schema)
                .fetch_one(&pool)
                .await
                .expect("pg_namespace");
        assert!(gone, "{schema} dropped");
        outcome.expect("the five-document corpus");
    }

    #[tokio::test]
    async fn invalid_schema_names_are_refused_before_any_sql() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://nobody@127.0.0.1:1/none")
            .expect("lazy pool");
        assert!(load(&pool, "a;b", &[]).await.is_err());
        assert!(search(&pool, "Upper", LVariant::And, "x", 1).await.is_err());
        let err = drop_schema(&pool, "").await.expect_err("refused");
        assert!(err.to_string().contains("schema"), "{err}");
    }

    #[test]
    fn password_of_finds_the_userinfo_password() {
        assert_eq!(password_of("postgres://u:pw@h/db"), Some("pw"));
        assert_eq!(password_of("postgres://u@h/db"), None);
        assert_eq!(password_of("postgres://u:@h/db"), None);
        assert_eq!(password_of("postgres://h/db"), None);
        assert_eq!(password_of("not a url"), None);
    }

    #[tokio::test]
    async fn connect_errors_never_echo_the_dsn() {
        let dsn = "postgres://bench_user:s3cr3t-pw@127.0.0.1:1/benchdb";
        let err = connect(dsn).await.expect_err("nothing listens on port 1");
        let shown = format!("{err:#} {err:?}");
        assert!(!shown.contains("s3cr3t-pw"), "{shown}");
        assert!(!shown.contains(dsn), "{shown}");
        let err = connect("not a url :// s3cr3t-pw")
            .await
            .expect_err("malformed");
        let shown = format!("{err:#} {err:?}");
        assert!(!shown.contains("s3cr3t-pw"), "{shown}");
    }
}
