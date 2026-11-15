mod activity;
mod agent_settings;
mod agent_settings_pg;
mod auth;
mod backlog;
mod backlog_detail;
mod backlog_docs;
mod backlog_forms;
mod backlog_keys;
mod backlog_runs;
mod box_probe;
mod box_probe_pg;
mod box_settings;
mod chat;
mod chat_live;
mod chat_live_agy;
mod chat_live_cli;
mod chat_offline;
mod chat_usage_pg;
mod claude_plugin;
mod concepts_search;
mod connection;
mod editor_pane;
mod graph_editor;
mod hand_written_pg;
mod hierarchy;
mod hierarchy_pg;
mod install;
mod integration;
mod item_writes_pg;
mod keys;
mod keys_file;
mod kinds;
mod kit_chrome;
mod mcp_stdio;
mod no_hardcoded_keys;
mod overlays;
mod panic_hook_order;
mod personas;
mod probe;
mod project_types;
mod prompt_preview;
mod prompt_settings;
mod provision;
mod provision_pg;
mod qdrant_cli_pg;
mod qdrant_settings;
mod qdrant_settings_pg;
mod qdrant_worker;
mod queue_overlay;
mod queue_settings;
mod replay;
mod requirements;
mod requirements_pg;
mod reveal;
mod runs_pg;
mod secrets_settings;
mod settings;
mod settings_documents;
mod settings_memory;
mod shell;
mod skills;
mod skills_pg;
mod state_cli_pg;
mod stop_cli_pg;
mod templates;
mod templates_pg;
mod token_scan;
mod waiting;
mod worker_crash_pg;
mod worker_log_pg;
mod worker_pg;
mod workspaces_tab;
#[path = "../../../../tests/rust/write_executable.rs"]
mod write_executable;

#[cfg(not(feature = "testkit"))]
#[test]
fn the_testkit_feature_is_off_so_most_of_this_binary_is_not_compiled() {
    panic!(
        "run with --features testkit (or --all-features): the gated modules compile to nothing without it"
    );
}
