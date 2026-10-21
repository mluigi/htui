//! MOD-115 M3 T4, blueprint §9: the ported views (group A) draw only through `ui/kit`. No
//! `Block::` or `Borders::` path may appear in their production code, and nothing else under
//! `src/ui` or `src/app` may frame itself unless [`GROUP_B`] lists it for M4; a new site fails
//! with its `path:line`, and a stale `GROUP_B` entry fails too, so M4 shrinks the list.
//!
//! Mirrors `tests/no_hardcoded_keys.rs`, through the same scanner (`tests/token_scan`): test
//! items are skipped, an import, a comment or a string is no site. It reads sources only, so it
//! runs without the testkit feature.

mod token_scan;

use token_scan::{relative, sites, sources, src_root};

/// The idents that build a frame by hand.
const NEEDLES: &[&str] = &["Block", "Borders"];

/// The trees the guard reads, relative to `src/`.
const SCOPE: &[&str] = &["ui/", "app/"];

/// Group A (MOD-115 M3): no frame of its own anywhere in these.
const GROUP_A: &[&str] = &[
    "ui/tabs/settings/",
    "ui/tabs/requirements/",
    "ui/tabs/chat/",
    "ui/path_picker.rs",
];

/// The kit draws the frames.
const KIT: &str = "ui/kit/";

/// Still framing themselves, each with what it frames; M4 ports each and deletes its entry. Built
/// from a scan of the merged M3 tree (`8e4568ba`): every non-test `Block::`/`Borders::` site in
/// `src/ui` and `src/app` outside group A and the kit.
const GROUP_B: &[(&str, &str)] = &[
    ("app/state.rs", "the `?` box and the no-tab body"),
    ("ui/editor_pane.rs", "the in-pane editor"),
    ("ui/markdown.rs", "markdown blocks"),
    ("ui/overlay/abort_confirm.rs", "overlay"),
    ("ui/overlay/concepts_search.rs", "overlay"),
    ("ui/overlay/migration_prompt.rs", "overlay"),
    ("ui/overlay/queue.rs", "overlay"),
    ("ui/overlay/workspace_switcher.rs", "overlay"),
    ("ui/tabs/backlog/detail/compose.rs", "Backlog"),
    ("ui/tabs/backlog/detail/mod.rs", "Backlog"),
    ("ui/tabs/backlog/detail/runs/execution_graph.rs", "Backlog"),
    ("ui/tabs/backlog/divergence.rs", "Backlog"),
    ("ui/tabs/backlog/filter.rs", "Backlog"),
    ("ui/tabs/backlog/item_form.rs", "Backlog"),
    ("ui/tabs/backlog/list.rs", "Backlog"),
    ("ui/tabs/skills/agent_help.rs", "Skills"),
    ("ui/tabs/skills/attach.rs", "Skills"),
    ("ui/tabs/skills/library.rs", "Skills"),
    ("ui/tabs/skills/templates.rs", "Skills"),
];

/// Whether `name` (relative to `src/`) is `prefix`, or under it when `prefix` ends in `/`.
fn under(name: &str, prefix: &str) -> bool {
    if prefix.ends_with('/') {
        name.starts_with(prefix)
    } else {
        name == prefix
    }
}

fn in_group_a(name: &str) -> bool {
    GROUP_A.iter().any(|prefix| under(name, prefix))
}

/// Every frame-building site in `src`'s production code.
fn violations(src: &str) -> Vec<usize> {
    sites(src, NEEDLES, &[])
}

/// Every `src/<path>:<line>` site in scope, with the file it is in.
fn scanned() -> Vec<(String, usize)> {
    let mut found = Vec::new();
    for path in sources(&src_root()) {
        let name = relative(&path);
        if !SCOPE.iter().any(|prefix| under(&name, prefix)) {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("a readable source");
        for line in violations(&src) {
            found.push((name.clone(), line));
        }
    }
    found
}

#[test]
fn group_a_draws_only_through_the_kit() {
    let found: Vec<String> = scanned()
        .into_iter()
        .filter(|(name, _)| in_group_a(name))
        .map(|(name, line)| format!("src/{name}:{line}"))
        .collect();
    assert!(
        found.is_empty(),
        "a ported view frames itself (MOD-115 M3): draw it through `ui/kit` (`Pane`, `Modal`, \
         `kit::list`, `kit::form`):\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn nothing_else_frames_itself_unless_m4_lists_it() {
    let found: Vec<String> = scanned()
        .into_iter()
        .filter(|(name, _)| !under(name, KIT) && !in_group_a(name))
        .filter(|(name, _)| !GROUP_B.iter().any(|(listed, _)| listed == name))
        .map(|(name, line)| format!("src/{name}:{line}"))
        .collect();
    assert!(
        found.is_empty(),
        "a new hand-built frame (MOD-115 M3): draw it through `ui/kit`; only M4's GROUP_B may \
         still frame itself:\n  {}",
        found.join("\n  ")
    );
}

/// A stale entry fails, as `no_hardcoded_keys.rs`'s `every_allow_list_entry_is_still_needed`
/// does: each listed file still frames itself outside its tests.
#[test]
fn every_m4_entry_still_frames_itself() {
    for (name, what) in GROUP_B {
        let src = std::fs::read_to_string(src_root().join(name)).expect("a listed file exists");
        assert!(
            !violations(&src).is_empty(),
            "src/{name} ({what}) frames itself no more: drop it from GROUP_B"
        );
    }
}

#[test]
fn no_group_a_file_is_listed_for_m4() {
    for (name, what) in GROUP_B {
        assert!(
            !in_group_a(name) && !under(name, KIT),
            "src/{name} ({what}) is group A or the kit: it may not be listed for M4"
        );
        assert!(
            SCOPE.iter().any(|prefix| under(name, prefix)),
            "src/{name} ({what}) is outside the scanned trees"
        );
    }
}

// The scanner's own cases, on inline sources.

#[test]
fn a_real_frame_is_reported_with_its_line() {
    let src = "fn a() {}\nfn b() -> Block<'static> {\n    Block::bordered()\n}\n";
    assert_eq!(violations(src), [3]);
    let nested = "fn c() { let b = ratatui::widgets::Block::default().borders(Borders::ALL); }";
    assert_eq!(violations(nested), [1, 1]);
    let in_macro = "fn d() -> bool {\n    matches!(x, Borders::NONE)\n}\n";
    assert_eq!(violations(in_macro), [2]);
}

#[test]
fn an_import_a_comment_or_a_string_is_no_frame() {
    let src = r##"use ratatui::widgets::{Block, Borders};
// Block::bordered() in a comment
/// Borders::ALL in a doc comment
fn a() -> &'static str { "Block::default()" }
fn b() -> &'static str { r#"Borders::ALL"# }
fn c(block: Block<'_>) {}
fn d() { let _ = BorderType::Rounded; let _ = Blocks::new(); }
"##;
    assert_eq!(violations(src), Vec::<usize>::new());
}

#[test]
fn a_frame_in_a_test_item_is_skipped() {
    let src = r##"fn real() {}
#[cfg(test)]
mod tests {
    const MESSAGE: &str = r#"} not the end { of the module }"#;
    fn frame() { let _ = Block::bordered(); }
}
#[test]
fn alone() { let _ = Borders::ALL; }
#[tokio::test]
async fn awaited() { let _ = Block::default(); }
fn after() { let _ = Block::new(); }
"##;
    assert_eq!(violations(src), [11]);
    let src = "#![cfg(test)]\nfn only_tests() { let _ = Block::bordered(); }\n";
    assert_eq!(violations(src), Vec::<usize>::new());
}

#[test]
fn group_a_is_matched_by_tree_or_by_file() {
    assert!(in_group_a("ui/tabs/settings/agents/mod.rs"));
    assert!(in_group_a("ui/tabs/chat/composer.rs"));
    assert!(in_group_a("ui/path_picker.rs"));
    assert!(!in_group_a("ui/path_picker_extra.rs"));
    assert!(!in_group_a("ui/tabs/skills/library.rs"));
}
