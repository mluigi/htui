//! Command line.

use std::path::PathBuf;

/// `htui` command line.
#[derive(Debug, Clone, Default, clap::Parser)]
#[command(group = clap::ArgGroup::new("concepts").args(["index_items", "search_items"]))]
#[command(
    name = "htui",
    version,
    about = "Terminal UI for the htui workflow store",
    args_conflicts_with_subcommands = true
)]
pub struct Args {
    /// A subcommand instead of the TUI. With one, no flag of the TUI may be given.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Load the demo fixture instead of connecting to Postgres.
    #[arg(long)]
    pub demo: bool,

    /// Write logs to this file. Never stdout: stdout is the TUI. After a subcommand
    /// (`htui worker --log PATH`), not before it; `HTUI_LOG` works with either.
    #[arg(long, env = "HTUI_LOG", value_name = "PATH", global = true)]
    pub log: Option<PathBuf>,

    /// Read a Postgres DSN from stdin, store it in the OS keyring and exit (`R-STO-1`).
    #[arg(long, conflicts_with_all = ["clear_dsn", "demo"])]
    pub set_dsn: bool,

    /// Remove the stored DSN from the OS keyring and exit.
    #[arg(long, conflicts_with_all = ["set_dsn", "demo"])]
    pub clear_dsn: bool,

    /// Read a Postgres DSN for this session from one line of stdin, held in memory only and never
    /// stored (`R-STO-1`): for a TUI over `ssh -t` on a box with no keyring.
    #[arg(
        long,
        conflicts_with_all = ["set_dsn", "clear_dsn", "demo", "offline", "index_items", "search_items"]
    )]
    pub dsn_stdin: bool,

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

    /// With `--search-items`: decisions only (items closed as done, concluded or rejected, and
    /// their documents).
    #[arg(long, requires = "search_items", conflicts_with = "index_items")]
    pub decisions: bool,

    /// With `--search-items`: how many hits to print, 1 to 1000 (default 10).
    #[arg(
        long,
        value_name = "N",
        value_parser = clap::value_parser!(u64).range(1..=1000),
        requires = "search_items",
        conflicts_with = "index_items"
    )]
    pub limit: Option<u64>,
}

/// `htui`'s subcommands (MOD-41 plan D14).
#[derive(Debug, Clone, PartialEq, Eq, clap::Subcommand)]
pub enum Command {
    /// Run this box's runs with no terminal: claim queued runs targeted at this box, walk them,
    /// heartbeat, recover, and stop on SIGINT/SIGTERM. The DSN comes from the OS keyring, the
    /// systemd credential `htui-dsn`, or `--dsn-stdin`; never argv or the environment.
    /// Engine-driven ACP steps still fail on their first permission request (MOD-42).
    Worker(WorkerArgs),
    /// Install this htui build as the `htui-worker` service on a Linux host over your own `ssh`
    /// (system unit, sudo), with the DSN encrypted on that host by `systemd-creds`; then set the
    /// new box's executor to `worker`. The DSN comes from the OS keyring or `--dsn-stdin`; never
    /// argv or the environment.
    Provision(ProvisionArgs),
}

/// `htui provision`'s arguments. None reads the environment (MOD-45 D291).
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct ProvisionArgs {
    /// The ssh destination: a host, `user@host`, or a `~/.ssh/config` alias.
    #[arg(value_name = "DESTINATION")]
    pub destination: String,
    /// Read the DSN the remote host should use from one line of stdin instead of the keyring
    /// (when it reaches Postgres at another address).
    #[arg(long)]
    pub dsn_stdin: bool,
    /// Re-encrypt the DSN on a host that already has one, and restart the service.
    #[arg(long)]
    pub replace_credential: bool,
}

/// `htui worker`'s flags. None reads the environment.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct WorkerArgs {
    /// Postgres connections, 2 to 8 (clamped).
    #[arg(long, value_name = "N", default_value_t = 4)]
    pub pool_size: u32,
    /// Read the DSN from one line of stdin instead of the keyring or the credential.
    #[arg(long)]
    pub dsn_stdin: bool,
}

#[cfg(test)]
mod tests {
    use super::{Args, Command, ProvisionArgs, WorkerArgs};
    use clap::{CommandFactory as _, Parser as _};

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
            &["htui", "--search-items", "q", "--limit", "0"],
            &[
                "htui",
                "--search-items",
                "q",
                "--limit",
                "18446744073709551615",
            ],
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

    /// MOD-41 plan D14: `htui worker` takes `--pool-size` (default 4) and `--dsn-stdin`.
    #[test]
    fn worker_parses_its_two_flags() {
        let args = Args::try_parse_from(["htui", "worker", "--pool-size", "6", "--dsn-stdin"])
            .expect("parse");
        assert_eq!(
            args.command,
            Some(Command::Worker(WorkerArgs {
                pool_size: 6,
                dsn_stdin: true,
            }))
        );
        let args = Args::try_parse_from(["htui", "worker"]).expect("parse");
        assert_eq!(
            args.command,
            Some(Command::Worker(WorkerArgs {
                pool_size: 4,
                dsn_stdin: false,
            }))
        );
    }

    /// Plan D14: every flat flag parses as before, with no subcommand; none may stand beside one.
    #[test]
    fn flat_flags_still_parse_and_refuse_a_subcommand_beside_them() {
        let args = Args::try_parse_from(["htui", "--offline"]).expect("parse");
        assert!(args.offline);
        assert_eq!(args.command, None);
        assert!(
            Args::try_parse_from(["htui", "--demo", "worker"]).is_err(),
            "a TUI flag beside `worker` must not parse"
        );
    }

    /// PRD D3, plan D14: no argument of `htui worker` reads the environment, except the
    /// propagated global `--log`, whose `HTUI_LOG` is the TUI's.
    #[test]
    fn no_worker_argument_reads_the_environment() {
        let mut command = Args::command();
        command.build();
        let worker = command
            .find_subcommand("worker")
            .expect("`worker` is a subcommand");
        let mut saw_log = false;
        for arg in worker.get_arguments() {
            if arg.get_id() == "log" {
                saw_log = true;
                assert_eq!(
                    arg.get_env(),
                    Some(std::ffi::OsStr::new("HTUI_LOG")),
                    "the global `--log` keeps `HTUI_LOG`"
                );
            } else {
                assert_eq!(
                    arg.get_env(),
                    None,
                    "`htui worker --{}` must not read the environment",
                    arg.get_id()
                );
            }
        }
        assert!(saw_log, "`--log` is propagated to `worker`");
    }

    /// Plan D14 (clap 4.6.6 with `args_conflicts_with_subcommands`): the global `--log` goes after
    /// the subcommand.
    #[test]
    fn log_goes_after_the_subcommand() {
        let args = Args::try_parse_from(["htui", "worker", "--log", "x"]).expect("parse");
        assert_eq!(args.log.as_deref(), Some(std::path::Path::new("x")));
        assert!(matches!(args.command, Some(Command::Worker(_))));
        assert!(
            Args::try_parse_from(["htui", "--log", "x", "worker"]).is_err(),
            "`--log` before `worker` must not parse"
        );
    }

    /// PRD D3: the DSN is never an argument.
    #[test]
    fn worker_takes_no_dsn_argument() {
        assert!(Args::try_parse_from(["htui", "worker", "--dsn", "x"]).is_err());
    }

    /// MOD-45 D291: a positional destination and two flags.
    #[test]
    fn provision_parses_its_destination_and_both_flags() {
        let args = Args::try_parse_from([
            "htui",
            "provision",
            "box1",
            "--dsn-stdin",
            "--replace-credential",
        ])
        .expect("parse");
        assert_eq!(
            args.command,
            Some(Command::Provision(ProvisionArgs {
                destination: "box1".to_owned(),
                dsn_stdin: true,
                replace_credential: true,
            }))
        );
        assert!(
            !args.dsn_stdin,
            "the flag is the subcommand's, not the TUI's"
        );
        let args = Args::try_parse_from(["htui", "provision", "box1"]).expect("parse");
        assert_eq!(
            args.command,
            Some(Command::Provision(ProvisionArgs {
                destination: "box1".to_owned(),
                dsn_stdin: false,
                replace_credential: false,
            }))
        );
    }

    /// MOD-45 D291, PRD D3: the destination is required and the DSN is never an argument.
    #[test]
    fn provision_needs_a_destination_and_takes_no_dsn_argument() {
        assert!(Args::try_parse_from(["htui", "provision"]).is_err());
        assert!(Args::try_parse_from(["htui", "provision", "box1", "--dsn", "x"]).is_err());
    }

    /// MOD-45 D291: no argument of `htui provision` reads the environment, except the propagated
    /// global `--log`.
    #[test]
    fn no_provision_argument_reads_the_environment() {
        let mut command = Args::command();
        command.build();
        let provision = command
            .find_subcommand("provision")
            .expect("`provision` is a subcommand");
        let mut saw_log = false;
        for arg in provision.get_arguments() {
            if arg.get_id() == "log" {
                saw_log = true;
                assert_eq!(arg.get_env(), Some(std::ffi::OsStr::new("HTUI_LOG")));
            } else {
                assert_eq!(
                    arg.get_env(),
                    None,
                    "`htui provision --{}` must not read the environment",
                    arg.get_id()
                );
            }
        }
        assert!(saw_log, "`--log` is propagated to `provision`");
    }

    /// MOD-45 D307: `--log` goes after `provision`, as after `worker`.
    #[test]
    fn log_goes_after_provision() {
        let args =
            Args::try_parse_from(["htui", "provision", "box1", "--log", "x"]).expect("parse");
        assert_eq!(args.log.as_deref(), Some(std::path::Path::new("x")));
        assert!(Args::try_parse_from(["htui", "--log", "x", "provision", "box1"]).is_err());
    }

    /// MOD-45 OQ-1 (a), E-13: the TUI's `--dsn-stdin` stands alone.
    #[test]
    fn the_tui_dsn_stdin_flag_parses_alone_and_excludes_the_keyring_flags() {
        let args = Args::try_parse_from(["htui", "--dsn-stdin"]).expect("parse");
        assert!(args.dsn_stdin);
        assert_eq!(args.command, None);
        for other in [
            &["--set-dsn"][..],
            &["--clear-dsn"],
            &["--demo"],
            &["--offline"],
            &["--index-items"],
            &["--search-items", "q"],
        ] {
            let mut argv = vec!["htui", "--dsn-stdin"];
            argv.extend_from_slice(other);
            assert!(
                Args::try_parse_from(&argv).is_err(),
                "{argv:?} must not parse"
            );
        }
        assert!(Args::try_parse_from(["htui", "--dsn-stdin", "worker"]).is_err());
        let args = Args::try_parse_from(["htui", "worker", "--dsn-stdin"]).expect("parse");
        assert!(!args.dsn_stdin);
        assert!(matches!(
            args.command,
            Some(Command::Worker(WorkerArgs {
                dsn_stdin: true,
                ..
            }))
        ));
    }

    /// `R-STO-1`: no top-level argument reads the environment but `--log`.
    #[test]
    fn no_tui_argument_but_log_reads_the_environment() {
        let mut command = Args::command();
        command.build();
        for arg in command.get_arguments() {
            if arg.get_id() == "log" {
                continue;
            }
            assert_eq!(
                arg.get_env(),
                None,
                "`htui --{}` must not read the environment",
                arg.get_id()
            );
        }
    }
}
