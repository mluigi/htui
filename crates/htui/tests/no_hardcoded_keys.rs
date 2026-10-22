//! MOD-115 M2 D7 (MD1), blueprint §10: every key goes through the keymap. No `KeyCode::` path
//! may appear in `crates/htui/src` outside a test item and the allow-list below, so a key matched
//! by hand cannot slip back in; a new site fails this test with its `path:line`.
//!
//! The sources are tokenised with `proc_macro2`, by the scanner `tests/kit_chrome.rs` shares
//! (`tests/token_scan`, MOD-115 M3 T4). Comments, strings and raw strings, such as the
//! `r#"…"#` test messages, are single tokens, which a brace scan does not handle. An outer
//! `#[cfg(test)]`, `#[test]` or `#[tokio::test]` skips the item it is on (up to and including its
//! brace group, or its `;`); an inner `#![cfg(test)]` skips the file. `#[cfg(any(test, feature =
//! "testkit"))]` is **not** skipped: it is compiled in testkit builds.

mod token_scan;

use token_scan::{relative, sites, sources, src_root};

/// Files that may name `KeyCode::`, each with why. Every entry must still name one (a stale
/// entry fails too).
const ALLOWED: &[(&str, &str)] = &[
    ("keys/chord.rs", "parsing, labels and the canonical chords"),
    ("keys/translate.rs", "the [field] rows' canonical events"),
    (
        "ui/text_field.rs",
        "canonical editing events, reached only through the translation",
    ),
    (
        "ui/text_area.rs",
        "canonical editing events, reached only through the translation",
    ),
    (
        "ui/tabs/chat/composer.rs",
        "canonical editing events, reached only through the translation",
    ),
    (
        "provision/secret_prompt.rs",
        "a raw tty prompt that runs before the keymap loads, outside the TUI",
    ),
];

/// A file where only one function may name `KeyCode::`, and that function.
const ALLOWED_FN: &[(&str, &str, &str)] = &[(
    "editor/pane.rs",
    "encode_key",
    "the bytes a key writes to the editor's PTY",
)];

/// The 1-based lines of every `KeyCode::` in `src` outside a test item and outside a function
/// named in `skip_fns`.
fn violations(src: &str, skip_fns: &[&str]) -> Vec<usize> {
    sites(src, &["KeyCode"], skip_fns)
}

#[test]
fn no_key_is_matched_by_hand_outside_the_allow_list() {
    let mut found = Vec::new();
    for path in sources(&src_root()) {
        let name = relative(&path);
        if ALLOWED.iter().any(|(allowed, _)| *allowed == name) {
            continue;
        }
        let skip: Vec<&str> = ALLOWED_FN
            .iter()
            .filter(|(file, ..)| *file == name)
            .map(|(_, function, _)| *function)
            .collect();
        let src = std::fs::read_to_string(&path).expect("a readable source");
        for line in violations(&src, &skip) {
            found.push(format!("src/{name}:{line}"));
        }
    }
    assert!(
        found.is_empty(),
        "a key matched by hand (MOD-115 M2 D7): resolve it through `Keys`, or decode a \
         canonical field event with `keys::translate::FieldEvent`:\n  {}",
        found.join("\n  ")
    );
}

/// A stale allow-list entry fails too: each allowed file, and each allowed function, still
/// names `KeyCode::` outside its tests.
#[test]
fn every_allow_list_entry_is_still_needed() {
    for (name, why) in ALLOWED {
        let src = std::fs::read_to_string(src_root().join(name)).expect("an allowed file exists");
        assert!(
            !violations(&src, &[]).is_empty(),
            "src/{name} ({why}) names no KeyCode:: any more: drop it from ALLOWED"
        );
    }
    for (name, function, why) in ALLOWED_FN {
        let src = std::fs::read_to_string(src_root().join(name)).expect("an allowed file exists");
        assert!(
            violations(&src, &[]).len() > violations(&src, &[function]).len(),
            "src/{name}'s {function} ({why}) names no KeyCode:: any more: drop it from ALLOWED_FN"
        );
    }
}

// The scanner's own cases, on inline sources.

#[test]
fn a_real_site_is_reported_with_its_line() {
    let src = "fn a() {}\nfn b(key: KeyEvent) -> bool {\n    key.code == KeyCode::Esc\n}\n";
    assert_eq!(violations(src, &[]), [3]);
    let nested = "fn b() { match key.code { crossterm::event::KeyCode::Enter => {}, _ => {} } }";
    assert_eq!(violations(nested, &[]), [1]);
    let in_macro = "fn c() -> bool {\n    matches!(key.code, KeyCode::Char(_))\n}\n";
    assert_eq!(violations(in_macro, &[]), [2]);
}

#[test]
fn an_import_a_comment_or_a_string_is_no_site() {
    let src = r##"use crossterm::event::{KeyCode, KeyEvent};
// KeyCode::Esc in a comment
/// KeyCode::Enter in a doc comment
fn a() -> &'static str { "KeyCode::Esc" }
fn b() -> &'static str { r#"KeyCode::Tab"# }
fn c(code: KeyCode) {}
"##;
    assert_eq!(violations(src, &[]), Vec::<usize>::new());
}

#[test]
fn a_test_item_is_skipped_even_with_braces_in_raw_strings() {
    let src = r##"fn real() {}
#[cfg(test)]
mod tests {
    const MESSAGE: &str = r#"} not the end { of the module }"#;
    fn key() { let _ = KeyCode::Esc; }
}
#[test]
fn alone() { let _ = KeyCode::Enter; }
#[tokio::test]
async fn awaited() { let _ = KeyCode::Tab; }
fn after() { let _ = KeyCode::Left; }
"##;
    assert_eq!(violations(src, &[]), [11]);
}

#[test]
fn a_testkit_item_is_not_skipped_and_an_inner_cfg_test_skips_the_file() {
    let src = "#[cfg(any(test, feature = \"testkit\"))]\nfn kit() { let _ = KeyCode::Esc; }\n";
    assert_eq!(violations(src, &[]), [2]);
    let src = "#![cfg(test)]\nfn only_tests() { let _ = KeyCode::Esc; }\n";
    assert_eq!(violations(src, &[]), Vec::<usize>::new());
}

#[test]
fn an_allowed_function_is_skipped_by_name() {
    let src = "fn encode_key(key: KeyEvent) { let _ = KeyCode::Esc; }\nfn other() { let _ = KeyCode::Up; }\n";
    assert_eq!(violations(src, &["encode_key"]), [2]);
    assert_eq!(violations(src, &[]), [1, 2]);
}
