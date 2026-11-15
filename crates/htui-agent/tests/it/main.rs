mod acp_conformance;
mod acp_driver;
mod acp_live;
mod acp_map;
mod agy_live;
mod auth;
mod auth_live;
mod box_probe;
mod cli_conformance;
mod cli_driver;
mod cli_live;
mod cli_map;
mod contained_spawn;
mod driver_contract;
mod estimator_live;
mod excerpt;
mod fake_conformance;
mod install;
mod install_live;
mod isolation_live;
mod launch;
mod loopback;
mod probe;
mod probe_live;
mod recorder;
mod relay;
mod replay;
mod target_channel;
#[path = "../../../../tests/rust/write_executable.rs"]
mod write_executable;

#[cfg(not(feature = "test-support"))]
#[test]
fn the_test_support_feature_is_off_so_most_of_this_binary_is_not_compiled() {
    panic!(
        "run with --features test-support (or --all-features): the gated modules compile to nothing without it"
    );
}
