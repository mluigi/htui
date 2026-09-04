//! Command line.

use std::path::PathBuf;

/// `htui` command line.
#[derive(Debug, Clone, Default, clap::Parser)]
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
    fn offline_parses_next_to_the_log_flag() {
        let args = Args::try_parse_from(["htui", "--offline", "--log", "htui.log"]).expect("parse");
        assert!(args.offline);
        assert!(!args.demo);
        assert_eq!(args.log.as_deref(), Some(std::path::Path::new("htui.log")));
    }
}
