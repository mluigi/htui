//! `htui-bench`: the MOD-162 retrieval benchmark's command line (`docs/htui-bench.md`).

use std::path::PathBuf;

use clap::Parser as _;

#[derive(Debug, clap::Parser)]
#[command(name = "htui-bench", about = "MOD-162 retrieval benchmark (dev tool)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Index the corpus, run every backend over the frozen set, write results.json and report.md.
    Run(RunArgs),
    /// Check the frozen set's hash, shape and targets against the corpus (no network).
    Validate(SetArgs),
    /// Write the query file's sha256 sidecar.
    Freeze(FreezeArgs),
    /// Compare two runs (results.json files or their directories).
    Compare(CompareArgs),
}

#[derive(Debug, clap::Args)]
struct SetArgs {
    /// Commit to build the corpus from, or WORKTREE for the files on disk.
    #[arg(long, default_value = htui_bench::corpus::DEFAULT_PIN)]
    corpus_ref: String,
    /// The repository (default: this workspace).
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))]
    repo: PathBuf,
    /// The frozen query file; its sidecar is `<file>.sha256`.
    #[arg(long, default_value = htui_bench::queries::DEFAULT_FILE)]
    queries: PathBuf,
}

#[derive(Debug, clap::Args)]
struct RunArgs {
    #[command(flatten)]
    set: SetArgs,
    /// Output directory for results.json and report.md (refused if results.json exists, unless --force).
    #[arg(long)]
    out: PathBuf,
    /// Overwrite an existing results.json.
    #[arg(long)]
    force: bool,
    /// Qdrant gRPC URL; falls back to HTUI_TEST_QDRANT_URL.
    #[arg(long, env = "HTUI_BENCH_QDRANT_URL", hide_env_values = true)]
    qdrant_url: Option<String>,
    /// Postgres DSN for the L baselines; falls back to HTUI_TEST_DATABASE_URL.
    #[arg(long, env = "HTUI_BENCH_DATABASE_URL", hide_env_values = true)]
    database_url: Option<String>,
    /// Default: htui_bench_<corpus ref>.
    #[arg(long)]
    collection: Option<String>,
    /// Postgres schema for the L baselines (dropped at the end).
    #[arg(long, default_value = htui_bench::pg_fts::SCHEMA)]
    schema: String,
}

#[derive(Debug, clap::Args)]
struct FreezeArgs {
    /// The query file to freeze.
    #[arg(long, default_value = htui_bench::queries::DEFAULT_FILE)]
    queries: PathBuf,
    /// Overwrite a sidecar holding another hash.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, clap::Args)]
struct CompareArgs {
    /// The first run.
    a: PathBuf,
    /// The second run.
    b: PathBuf,
}

/// The flag (or its `HTUI_BENCH_*` variable, which clap reads), else the `HTUI_TEST_*` fallback;
/// a blank value is unset.
fn pick(flag: Option<String>, fallback: Option<String>) -> Option<String> {
    let _ = (flag, fallback);
    None
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let _ = cli.command;
    anyhow::bail!("htui-bench: the CLI lands with MOD-162 T6")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("htui-bench").chain(args.iter().copied()))
    }

    #[test]
    fn run_parses_with_defaults() {
        let cli = parse(&["run", "--out", "target/bench/x"]).expect("parses");
        let Command::Run(args) = cli.command else {
            panic!("not run: {cli:?}");
        };
        assert_eq!(args.out, PathBuf::from("target/bench/x"));
        assert!(!args.force);
        assert_eq!(args.set.corpus_ref, htui_bench::corpus::DEFAULT_PIN);
        assert_eq!(
            args.set.queries,
            PathBuf::from(htui_bench::queries::DEFAULT_FILE)
        );
        assert_eq!(args.schema, htui_bench::pg_fts::SCHEMA);
        assert_eq!(args.collection, None);
    }

    #[test]
    fn run_takes_every_flag() {
        let cli = parse(&[
            "run",
            "--out",
            "o",
            "--force",
            "--corpus-ref",
            "WORKTREE",
            "--repo",
            "/r",
            "--queries",
            "/q.json",
            "--qdrant-url",
            "http://q:6334",
            "--database-url",
            "postgres://x",
            "--collection",
            "c",
            "--schema",
            "s",
        ])
        .expect("parses");
        let Command::Run(args) = cli.command else {
            panic!("not run");
        };
        assert!(args.force);
        assert_eq!(args.set.corpus_ref, "WORKTREE");
        assert_eq!(args.set.repo, PathBuf::from("/r"));
        assert_eq!(args.set.queries, PathBuf::from("/q.json"));
        assert_eq!(args.qdrant_url.as_deref(), Some("http://q:6334"));
        assert_eq!(args.database_url.as_deref(), Some("postgres://x"));
        assert_eq!(args.collection.as_deref(), Some("c"));
        assert_eq!(args.schema, "s");
    }

    #[test]
    fn run_without_out_is_an_error() {
        let err = parse(&["run"]).expect_err("--out is required");
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn validate_freeze_and_compare_parse() {
        let cli = parse(&["validate", "--corpus-ref", "abc"]).expect("validate");
        assert!(matches!(cli.command, Command::Validate(ref a) if a.corpus_ref == "abc"));

        let cli = parse(&["freeze"]).expect("freeze");
        let Command::Freeze(args) = cli.command else {
            panic!("not freeze");
        };
        assert!(!args.force);
        assert_eq!(args.queries, PathBuf::from(htui_bench::queries::DEFAULT_FILE));
        assert!(matches!(
            parse(&["freeze", "--force"]).expect("freeze").command,
            Command::Freeze(FreezeArgs { force: true, .. })
        ));

        let cli = parse(&["compare", "a/results.json", "b"]).expect("compare");
        let Command::Compare(args) = cli.command else {
            panic!("not compare");
        };
        assert_eq!(args.a, PathBuf::from("a/results.json"));
        assert_eq!(args.b, PathBuf::from("b"));
        assert!(parse(&["compare", "only-one"]).is_err());
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn urls_fall_back_and_blank_is_unset() {
        assert_eq!(
            pick(Some("flag".into()), Some("test".into())).as_deref(),
            Some("flag")
        );
        assert_eq!(pick(None, Some("test".into())).as_deref(), Some("test"));
        assert_eq!(pick(Some("  ".into()), Some("test".into())).as_deref(), Some("test"));
        assert_eq!(pick(None, Some(String::new())), None);
        assert_eq!(pick(None, None), None);
    }
}
