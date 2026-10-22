//! The source scanner the guard tests share (`tests/no_hardcoded_keys.rs`, `tests/kit_chrome.rs`;
//! MOD-115 M3 T4, blueprint §9), so the two cannot drift.
//!
//! The sources are tokenised with `proc_macro2` (comments, strings and raw strings, such as the
//! `r#"…"#` test messages, are single tokens, which a brace scan does not handle). An outer
//! `#[cfg(test)]`, `#[test]` or `#[tokio::test]` skips the item it is on (up to and including its
//! brace group, or its `;`); an inner `#![cfg(test)]` skips the file. `#[cfg(any(test, feature =
//! "testkit"))]` is **not** skipped: it is compiled in testkit builds. A site is one of the named
//! idents followed by a joint `::` (`KeyCode::Esc`, `Block::bordered()`), so an import, a type in a
//! signature, a comment or a string is none.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};

/// The 1-based lines of every `<needle>::` in `src` outside a test item and outside a function
/// named in `skip_fns`.
pub fn sites(src: &str, needles: &[&str], skip_fns: &[&str]) -> Vec<usize> {
    sites_except(src, needles, skip_fns, &[])
}

/// [`sites`], less the paths in `exempt`: a `<needle>::<next>` whose `"<needle>::<next>"` is listed
/// there is no site (a file's own `Block::Blank` is not ratatui's `Block::bordered()`).
pub fn sites_except(src: &str, needles: &[&str], skip_fns: &[&str], exempt: &[&str]) -> Vec<usize> {
    let stream = TokenStream::from_str(src).expect("the source tokenises");
    let scan = Scan {
        needles,
        skip_fns,
        exempt,
    };
    let mut lines = Vec::new();
    walk(stream, &scan, &mut lines);
    lines
}

/// The names `src` gives the `needles` with `as` (`use ratatui::widgets::Block as Framed;`,
/// or inside a `use` group), so a guard can look for them too: `Framed::bordered()` is the same
/// frame (MOD-115 M3 RV1 L7). `as _` names nothing. Read everywhere, test items included: a
/// name's scope is not the scanner's to tell.
#[allow(dead_code)] // `tests/no_hardcoded_keys.rs` shares this module and does not use it.
pub fn aliases(src: &str, needles: &[&str]) -> Vec<String> {
    let stream = TokenStream::from_str(src).expect("the source tokenises");
    let mut found = Vec::new();
    collect_aliases(stream, needles, &mut found);
    found
}

fn collect_aliases(stream: TokenStream, needles: &[&str], found: &mut Vec<String>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (i, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Ident(ident) if needles.iter().any(|needle| ident == needle) => {
                if let (Some(TokenTree::Ident(word)), Some(TokenTree::Ident(alias))) =
                    (tokens.get(i + 1), tokens.get(i + 2))
                    && word == "as"
                    && alias != "_"
                {
                    let alias = alias.to_string();
                    if !found.contains(&alias) {
                        found.push(alias);
                    }
                }
            }
            TokenTree::Group(group) => collect_aliases(group.stream(), needles, found),
            _ => {}
        }
    }
}

/// What [`walk`] looks for.
struct Scan<'a> {
    needles: &'a [&'a str],
    skip_fns: &'a [&'a str],
    exempt: &'a [&'a str],
}

/// Whether a `#[…]` group's contents mark a test item: `cfg(test)`, `test`, `tokio::test`.
fn is_test_attr(group: &TokenStream) -> bool {
    let text: String = group.to_string().split_whitespace().collect();
    matches!(text.as_str(), "cfg(test)" | "test" | "tokio::test")
        || text.starts_with("tokio::test(")
}

fn walk(stream: TokenStream, scan: &Scan<'_>, lines: &mut Vec<usize>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Punct(hash) if hash.as_char() == '#' => {
                // `#![cfg(test)]` (inner): nothing after it in this stream is compiled outside tests.
                if let (Some(TokenTree::Punct(bang)), Some(TokenTree::Group(group))) =
                    (tokens.get(i + 1), tokens.get(i + 2))
                    && bang.as_char() == '!'
                    && group.delimiter() == Delimiter::Bracket
                    && is_test_attr(&group.stream())
                {
                    return;
                }
                if let Some(TokenTree::Group(group)) = tokens.get(i + 1)
                    && group.delimiter() == Delimiter::Bracket
                {
                    if is_test_attr(&group.stream()) {
                        i = skip_item(&tokens, i + 2);
                        continue;
                    }
                    i += 2; // any other attribute: its contents are no code
                    continue;
                }
                i += 1;
            }
            TokenTree::Ident(ident) if ident == "fn" => {
                if let Some(TokenTree::Ident(name)) = tokens.get(i + 1)
                    && scan.skip_fns.iter().any(|skip| name == skip)
                {
                    i = skip_item(&tokens, i + 2);
                    continue;
                }
                i += 1;
            }
            TokenTree::Ident(ident) if scan.needles.iter().any(|needle| ident == needle) => {
                if let (Some(TokenTree::Punct(first)), Some(TokenTree::Punct(second))) =
                    (tokens.get(i + 1), tokens.get(i + 2))
                    && first.as_char() == ':'
                    && first.spacing() == Spacing::Joint
                    && second.as_char() == ':'
                    && !matches!(tokens.get(i + 3), Some(TokenTree::Ident(next))
                        if scan.exempt.iter().any(|path| *path == format!("{ident}::{next}")))
                {
                    lines.push(ident.span().start().line);
                }
                i += 1;
            }
            TokenTree::Group(group) => {
                walk(group.stream(), scan, lines);
                i += 1;
            }
            _ => i += 1,
        }
    }
}

/// The index past the item that starts at `from`: through its first brace group, or its `;`.
fn skip_item(tokens: &[TokenTree], from: usize) -> usize {
    let mut i = from;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => return i + 1,
            TokenTree::Punct(punct) if punct.as_char() == ';' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// Every `.rs` file under `dir`, sorted.
pub fn sources(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("the source tree is readable") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// `crates/htui/src`.
pub fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// `path` relative to `src/`, with `/` separators.
pub fn relative(path: &Path) -> String {
    path.strip_prefix(src_root())
        .expect("under src/")
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}
