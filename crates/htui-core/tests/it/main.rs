mod capture;
mod mem_store;
mod prompt_digest;
mod prompt_golden;
mod prompt_hostname;
mod prompt_memory;
mod prompt_output;
mod prompt_part;
mod prompt_parts;
mod prompt_persona;
mod prompt_render;
mod prompt_skills;
mod skill_import;

#[cfg(not(feature = "test-support"))]
#[test]
fn the_test_support_feature_is_off_so_most_of_this_binary_is_not_compiled() {
    panic!(
        "run with --features test-support (or --all-features): the gated modules compile to nothing without it"
    );
}
