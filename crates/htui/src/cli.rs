//! Command line.

use std::path::PathBuf;

/// `htui` command line.
#[derive(Debug, Clone, Default, clap::Parser)]
#[command(group = clap::ArgGroup::new("concepts").args(["index_items", "search_items"]))]
#[command(
    name = "htui",
    version,
    about = "Terminal UI for the htui workflow store"
)]
pub struct Args {
    /// Load the demo fixture instead of connecting to Postgres.
    #[arg(long)]
    pub demo: bool,

    /// Write logs to this file. Never stdout: stdout is the TUI.
    #[arg(long, env = "HTUI_LOG", value_name = "PATH")]
    pub log: Option<PathBuf>,

    /// Read a Postgres DSN from stdin, store it in the OS keyring and exit (`R-STO-1`).
    #[arg(long, conflicts_with_all = ["clear_dsn", "demo"])]
    pub set_dsn: bool,

    /// Remove the stored DSN from the OS keyring and exit.
    #[arg(long, conflicts_with_all = ["set_dsn", "demo"])]
    pub clear_dsn: bool,

    /// Open from the local cache and never attempt a connection (demos, tests, a flaky network).
    #[arg(long)]
    pub offline: bool,

    /// Bring the Qdrant concepts index in step with Postgres items and documents, then exit
    /// (`R-STO-8`).
    #[arg(long, conflicts_with_all = ["set_dsn", "clear_dsn", "demo", "offline"])]
    pub index_items: bool,

    /// Search items and their documents by meaning and exact term, print the hits and exit.
    #[arg(long, value_name = "QUERY", conflicts_with_all = ["set_dsn", "clear_dsn", "demo", "offline"])]
    pub search_items: Option<String>,

    /// With `--index-items` or `--search-items`: only the project with this slug.
    #[arg(long, value_name = "SLUG", requires = "concepts")]
    pub project: Option<String>,

    /// With `--search-items`: decisions only (done and closed items and their documents).
    #[arg(long, requires = "search_items", conflicts_with = "index_items")]
    pub decisions: bool,

    /// With `--search-items`: how many hits to print (default 10).
    #[arg(
        long,
        value_name = "N",
        requires = "search_items",
        conflicts_with = "index_items"
    )]
    pub limit: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::Args;
    use clap::Parser as _;

    #[test]
    fn the_dsn_flags_exclude_each_other_and_demo() {
        for pair in [
            ["--set-dsn", "--clear-dsn"],
            ["--set-dsn", "--demo"],
            ["--clear-dsn", "--demo"],
        ] {
            assert!(
                Args::try_parse_from(["htui", pair[0], pair[1]]).is_err(),
                "{pair:?} must not parse together"
            );
        }
    }

    #[test]
    fn the_concepts_flags_parse_and_exclude_each_other() {
        let args = Args::try_parse_from([
            "htui",
            "--search-items",
            "MOD-34",
            "--project",
            "htui",
            "--decisions",
            "--limit",
            "3",
        ])
        .expect("parse");
        assert_eq!(args.search_items.as_deref(), Some("MOD-34"));
        assert_eq!(args.project.as_deref(), Some("htui"));
        assert!(args.decisions);
        assert_eq!(args.limit, Some(3));
        assert!(Args::try_parse_from(["htui", "--index-items", "--project", "htui"]).is_ok());
        for bad in [
            &["htui", "--index-items", "--search-items", "q"][..],
            &["htui", "--index-items", "--demo"],
            &["htui", "--project", "htui"],
            &["htui", "--decisions"],
            &["htui", "--index-items", "--decisions"],
            &["htui", "--index-items", "--limit", "3"],
        ] {
            assert!(Args::try_parse_from(bad).is_err(), "{bad:?} must not parse");
        }
    }

    #[test]
    fn offline_parses_next_to_the_log_flag() {
        let args = Args::try_parse_from(["htui", "--offline", "--log", "htui.log"]).expect("parse");
        assert!(args.offline);
        assert!(!args.demo);
        assert_eq!(args.log.as_deref(), Some(std::path::Path::new("htui.log")));
    }
}
