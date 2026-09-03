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
    /// Load the demo fixture instead of an empty store.
    #[arg(long)]
    pub demo: bool,

    /// Write logs to this file. Never stdout: stdout is the TUI.
    #[arg(long, env = "HTUI_LOG", value_name = "PATH")]
    pub log: Option<PathBuf>,
}
