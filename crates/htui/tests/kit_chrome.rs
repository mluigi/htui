//! MOD-115 M3 T4, blueprint §9: the ported views (group A) draw only through `ui/kit`. No
//! `Block::` or `Borders::` path (nor one through an alias, `use … Block as Framed`) may appear in
//! their production code, and nothing else under `src` may frame itself unless [`GROUP_B`] lists
//! it for M4; a new site fails with its `path:line`, and a stale `GROUP_B` entry fails too, so M4
//! shrinks the list. The whole of `src` is read (RV1 L7), not only `ui/` and `app/`.
//!
//! Mirrors `tests/no_hardcoded_keys.rs`, through the same scanner (`tests/token_scan`): test
//! items are skipped, an import, a comment or a string is no site. It reads sources only, so it
//! runs without the testkit feature.

mod token_scan;

use std::path::Path;

use token_scan::{aliases, relative, sites, sites_except, sources, src_root};

/// The idents that build a frame by hand.
const NEEDLES: &[&str] = &["Block", "Borders"];

/// Files that declare a `Block` of their own, with the paths that name it: no frame, and no work
/// for M4, so they are not in [`GROUP_B`]. Only those paths are exempt, so a ratatui frame added
/// to such a file (`ratatui::widgets::Block::bordered()`, `Borders::ALL`) is still a site.
const OWN_BLOCK: &[(&str, &[&str], &str)] = &[(
    "ui/markdown.rs",
    &[
        "Block::Blank",
        "Block::Plain",
        "Block::Literal",
        "Block::Paragraph",
        "Block::Item",
    ],
    "its `enum Block` is a parsed markdown block, not a ratatui frame",
)];

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
/// `src` outside group A and the kit, less the [`OWN_BLOCK`] name collisions (all of them under
/// `src/ui` and `src/app`, which RV1 L7 checked when it widened the scan to the whole of `src`).
const GROUP_B: &[(&str, &str)] = &[
    ("app/state.rs", "the `?` box and the no-tab body"),
    ("ui/editor_pane.rs", "the in-pane editor"),
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

/// The idents that build a frame in `src`: [`NEEDLES`] and the names `src` gives them with `as`.
fn needles(src: &str) -> Vec<String> {
    let mut needles: Vec<String> = NEEDLES.iter().map(|needle| (*needle).to_owned()).collect();
    needles.extend(aliases(src, NEEDLES));
    needles
}

/// Every frame-building site in `src`'s production code.
fn violations(src: &str) -> Vec<usize> {
    let needles = needles(src);
    let needles: Vec<&str> = needles.iter().map(String::as_str).collect();
    sites(src, &needles, &[])
}

/// The paths that name `name`'s own `Block`, if [`OWN_BLOCK`] lists it.
fn own_block(name: &str) -> &'static [&'static str] {
    OWN_BLOCK
        .iter()
        .find(|(listed, _, _)| *listed == name)
        .map_or(&[], |(_, paths, _)| paths)
}

/// Every frame-building site in the production code of `src`, the source of `name`.
fn violations_in(name: &str, src: &str) -> Vec<usize> {
    let needles = needles(src);
    let needles: Vec<&str> = needles.iter().map(String::as_str).collect();
    sites_except(src, &needles, &[], own_block(name))
}

/// `path` relative to `root`, with `/` separators.
fn name_under(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("under the scanned root")
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Every frame-building site in every `.rs` file under `root` (all of them, RV1 L7), as the file
/// relative to `root` and the line.
fn scan(root: &Path) -> Vec<(String, usize)> {
    let mut found = Vec::new();
    for path in sources(root) {
        let name = name_under(root, &path);
        let src = std::fs::read_to_string(&path).expect("a readable source");
        for line in violations_in(&name, &src) {
            found.push((name.clone(), line));
        }
    }
    found
}

/// Every `src/<path>:<line>` site, with the file it is in.
fn scanned() -> Vec<(String, usize)> {
    scan(&src_root())
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
            !violations_in(name, &src).is_empty(),
            "src/{name} ({what}) frames itself no more: drop it from GROUP_B"
        );
    }
}

/// A stale exemption fails too: each listed file still names its own `Block`, through a path
/// the exemption is what keeps from being a site.
#[test]
fn every_own_block_entry_is_still_needed() {
    for (name, paths, what) in OWN_BLOCK {
        let src = std::fs::read_to_string(src_root().join(name)).expect("a listed file exists");
        let blocks = sites(&src, &["Block"], &[]);
        let frames = sites_except(&src, &["Block"], &[], paths);
        assert!(
            blocks.len() > frames.len(),
            "src/{name} ({what}) names its own `Block` no more: drop it from OWN_BLOCK"
        );
        assert!(
            !in_group_a(name) && !under(name, KIT),
            "src/{name} ({what}) is group A or the kit"
        );
        assert!(
            !GROUP_B.iter().any(|(listed, _)| listed == name),
            "src/{name} ({what}) frames nothing: it is no work for M4"
        );
    }
}

#[test]
fn an_own_block_path_is_no_frame_but_a_ratatui_block_still_is() {
    let src = "enum Block { Blank, Item { at: usize } }\n\
               fn a() -> Block { Block::Blank }\n\
               fn b(x: &Block) -> bool { matches!(x, Block::Item { .. }) }\n\
               fn c() { let _ = ratatui::widgets::Block::bordered(); }\n\
               fn d() { let _ = Borders::ALL; }\n";
    let exempt = ["Block::Blank", "Block::Item"];
    assert_eq!(sites_except(src, NEEDLES, &[], &exempt), [4, 5]);
    assert_eq!(violations(src), [2, 3, 4, 5]);
    // The exemption is the listed file's alone.
    assert_eq!(violations_in("ui/markdown.rs", src), [4, 5]);
    assert_eq!(violations_in("ui/editor_pane.rs", src), [2, 3, 4, 5]);
}

#[test]
fn no_group_a_file_is_listed_for_m4() {
    for (name, what) in GROUP_B {
        assert!(
            !in_group_a(name) && !under(name, KIT),
            "src/{name} ({what}) is group A or the kit: it may not be listed for M4"
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

/// RV1 L7: an alias is the frame under another name: `use ratatui::widgets::Block as Framed`
/// makes `Framed::bordered()` a site, in a group or alone.
#[test]
fn an_aliased_frame_is_still_a_frame() {
    let src = "use ratatui::widgets::Block as Framed;\n\
               use ratatui::widgets::{Borders as Edges, Paragraph};\n\
               fn a() { let _ = Framed::bordered(); }\n\
               fn b() { let _ = Edges::ALL; }\n\
               fn c() { let _ = Paragraph::new(\"\"); }\n\
               use ratatui::widgets::Block as _;\n";
    assert_eq!(violations(src), [3, 4]);
    assert_eq!(
        violations_in("ui/markdown.rs", src),
        [3, 4],
        "no exemption covers an alias"
    );
}

/// RV1 L7: the guard reads every file under `src/`, not only `ui/` and `app/`.
#[test]
fn the_guard_reads_all_of_src() {
    // A tree shaped like `src`, with a frame at the root, one in a tree other than `ui/` and
    // `app/`, and one under `ui/` for contrast: the scan reports all three.
    let root = tempfile::tempdir().expect("a throwaway source tree");
    let frame = "fn a() {}\nfn b() { let _ = Block::bordered(); }\n";
    for name in ["main.rs", "store/sync.rs", "ui/view.rs"] {
        let path = root.path().join(name);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a writable tree");
        std::fs::write(&path, frame).expect("a writable source");
    }
    let found = scan(root.path());
    assert_eq!(
        found,
        [
            ("main.rs".to_owned(), 2),
            ("store/sync.rs".to_owned(), 2),
            ("ui/view.rs".to_owned(), 2),
        ],
        "a frame outside `ui/` and `app/` is reported too"
    );
    // The real `src` is read through the same scan, and has files outside the two trees.
    let names: Vec<String> = sources(&src_root())
        .iter()
        .map(|path| relative(path))
        .collect();
    assert!(names.iter().any(|name| name == "main.rs"), "{names:?}");
}

#[test]
fn group_a_is_matched_by_tree_or_by_file() {
    assert!(in_group_a("ui/tabs/settings/agents/mod.rs"));
    assert!(in_group_a("ui/tabs/chat/composer.rs"));
    assert!(in_group_a("ui/path_picker.rs"));
    assert!(!in_group_a("ui/path_picker_extra.rs"));
    assert!(!in_group_a("ui/tabs/skills/library.rs"));
}
