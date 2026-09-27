//! Path globs for skill activation (ANA-22 §6 item 7; plan D70, D71). Hand-written, deliberately:
//! `globset` matches `*` across `/` by default and its `backslash_escape` default differs per
//! platform, so both would have to be configured away to get the dialect below.
//!
//! The dialect, over repo-relative `/`-separated paths, is normative: `?` is one non-`/` character,
//! `*` is any run of non-`/` characters and never crosses `/`, `**` matches zero or more whole
//! components and is legal only as a whole component, `{a,b}` alternates and is legal only as a
//! whole component, `[abc]` / `[!abc]` / `[a-z]` are character classes, `\` escapes the next
//! metacharacter, and a leading `!`, a trailing `/`, an absolute path and a NUL are refused.
//! Matching is case-sensitive. One implementation of "compiles" serves both the writer and the
//! matcher, so a stored glob is always a compiling one (plan D78, D99).
//!
//! The price of owning the syntax is stated once, by plan D70: **anything this dialect does not
//! implement is refused at save time rather than silently matching nothing.** Every refusal is a
//! [`GlobError`] variant carrying the byte it happened at, so the editor can put a cursor on it.

use std::collections::BTreeMap;

use thiserror::Error;

use super::excerpt::RepoPath;
use crate::model::ids::SkillId;
use crate::model::skill::BoundSkill;

/// A compiled glob: segments separated by `/`, each one of [`Segment`].
///
/// Compiled by [`compile`], which is the only way to make one: a `Pattern` is well-formed by
/// construction, so nothing downstream re-checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    /// The `<repo>:` qualifier, when the pattern names one. `None` matches in any repo (D71).
    pub repo: Option<String>,
    /// The path part, `/`-separated.
    pub segments: Vec<Segment>,
}

/// One `/`-separated piece of a [`Pattern`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// `*` anywhere inside a component, and the `?` / `[…]` / `\` forms that share a component.
    Parts(Vec<Tok>),
    /// A `**` whole component: zero or more components.
    AnyComponents,
    /// `{a,b}` over whole components (D96: never nested).
    Alt(Vec<Vec<Segment>>),
}

/// One atom inside a [`Segment::Parts`] component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    /// A literal character. `\` produces these, so a metacharacter can be written literally.
    Char(char),
    /// `?`
    Any,
    /// `*`
    Star,
    /// `[abc]`, `[!abc]`, `[a-z]`. A range is inclusive and the negation flips the whole class.
    Class {
        /// `!` opened the class: match a character no range holds.
        negated: bool,
        /// The ranges, in the order they were written. Duplicates are kept, not folded.
        ranges: Vec<(char, char)>,
    },
}

/// One variant per refusal, each carrying the byte it happened at (plan D103).
///
/// The `Display` strings are a contract (plan D99): the matrix view pre-flights a typed glob with
/// [`compile`] and shows this text, and the writer's `Constraint` carries the identical string, so
/// the two can never disagree about why a glob was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GlobError {
    /// The pattern is nothing, or one of its `/`-separated components is (`a//b`).
    ///
    /// The empty component is refused rather than folded away: a stored glob has to mean what the
    /// maintainer typed, and `a//b` is not how anyone means `a/b`.
    #[error("a glob must not be empty")]
    Empty,
    /// `!` is not negation in this dialect — negation is `activation = off`, on the attachment.
    #[error("a leading `!` is not negation in htui globs, at byte {at}")]
    Negation {
        /// The byte offset of the `!`.
        at: usize,
    },
    /// A trailing `/` asks for directories, and a file set only ever holds files.
    #[error("a trailing `/` matches directories, which a file set never holds, at byte {at}")]
    TrailingSlash {
        /// The byte offset of the `/`.
        at: usize,
    },
    /// Paths are repo-relative; a leading `/` would name the repo root, not a file in a repo.
    #[error("a glob must be repo-relative, at byte {at}")]
    Absolute {
        /// The byte offset of the `/`.
        at: usize,
    },
    /// `**` swallows whole components, so it is only meaningful as one. `a/**b` is a typo for
    /// `a/**/b`, and a typo that compiles is a glob that quietly matches nothing (R-25).
    #[error("`**` is only legal as a whole component, at byte {at}")]
    MisplacedGlobstar {
        /// The byte offset of the first `*` of the offending `**`.
        at: usize,
    },
    /// An alternation is a whole component, so it can neither sit inside a longer component
    /// (`pre{a,b}fix.rs`) nor inside another alternation (`{a,{b,c}}`).
    #[error("`{{a,{{b,c}}}}` nests, which this dialect does not allow, at byte {at}")]
    NestedAlternation {
        /// The byte offset of the offending brace.
        at: usize,
    },
    /// `{,a}`, `{a,}` and `{}` all name an alternative that is not there.
    #[error("`{{a,}}` has an empty alternative, at byte {at}")]
    EmptyAlternative {
        /// The byte offset the empty alternative would have started at.
        at: usize,
    },
    /// A `{` that never reaches its `}` inside its component.
    #[error("`{{` without `}}`, at byte {at}")]
    UnterminatedBrace {
        /// The byte offset of the `{`.
        at: usize,
    },
    /// A `[` that never reaches its `]`, including one that runs into the next `/`.
    #[error("`[` without `]`, at byte {at}")]
    UnterminatedClass {
        /// The byte offset of the `[`.
        at: usize,
    },
    /// `[]` and `[!]` hold no character at all.
    #[error("`[]` matches nothing, at byte {at}")]
    EmptyClass {
        /// The byte offset of the `[`.
        at: usize,
    },
    /// A `\` with nothing after it.
    #[error("`\\` at the end of a glob escapes nothing, at byte {at}")]
    DanglingEscape {
        /// The byte offset of the `\`.
        at: usize,
    },
    /// Postgres `text` cannot hold `U+0000` (`22021`), so neither can a stored glob.
    #[error("a glob must not contain a NUL character, at byte {at}")]
    Nul {
        /// The byte offset of the NUL.
        at: usize,
    },
    /// `:` opens the `<repo>:` qualifier, so a pattern that starts with one names no repository.
    #[error("`{{repo}}:` names no repository, at byte {at}")]
    BadQualifier {
        /// The byte offset of the `:`.
        at: usize,
    },
}

impl GlobError {
    /// The byte the refusal happened at, or `None` for [`GlobError::Empty`], which is about the
    /// pattern as a whole.
    #[must_use]
    pub const fn at(&self) -> Option<usize> {
        match self {
            Self::Empty => None,
            Self::Negation { at }
            | Self::TrailingSlash { at }
            | Self::Absolute { at }
            | Self::MisplacedGlobstar { at }
            | Self::NestedAlternation { at }
            | Self::EmptyAlternative { at }
            | Self::UnterminatedBrace { at }
            | Self::UnterminatedClass { at }
            | Self::EmptyClass { at }
            | Self::DanglingEscape { at }
            | Self::Nul { at }
            | Self::BadQualifier { at } => Some(*at),
        }
    }
}

/// Compiles one glob, or says why it will not compile.
///
/// Total: every pattern either yields a [`Pattern`] or a [`GlobError`] with a position. A caller
/// that is only matching — [`first_match`], [`matched_skills`] — skips a pattern that does not
/// compile rather than failing the whole set, because the writer (D78) is what refuses one, and a
/// glob that was stored before a rule tightened must not take every skill with it.
pub fn compile(pattern: &str) -> Result<Pattern, GlobError> {
    if pattern.is_empty() {
        return Err(GlobError::Empty);
    }
    if let Some(at) = pattern.find('\0') {
        return Err(GlobError::Nul { at });
    }
    let (repo, rest, base) = match qualifier_end(pattern) {
        Some(0) => return Err(GlobError::BadQualifier { at: 0 }),
        Some(at) => (Some(pattern[..at].to_owned()), &pattern[at + 1..], at + 1),
        None => (None, pattern, 0),
    };
    if rest.is_empty() {
        return Err(GlobError::Empty);
    }
    if rest.starts_with('!') {
        return Err(GlobError::Negation { at: base });
    }
    if rest.starts_with('/') {
        return Err(GlobError::Absolute { at: base });
    }
    if rest.ends_with('/') {
        return Err(GlobError::TrailingSlash {
            at: base + rest.len() - 1,
        });
    }
    let mut segments = Vec::new();
    let mut offset = base;
    for component in rest.split('/') {
        if component.is_empty() {
            return Err(GlobError::Empty);
        }
        segments.push(compile_component(component, offset)?);
        offset += component.len() + 1;
    }
    Ok(Pattern { repo, segments })
}

impl Pattern {
    /// Whether this pattern's **path part** matches `path`, a repo-relative `/`-separated path.
    ///
    /// The `<repo>:` qualifier is not consulted here — this is plan D71's signature — so a caller
    /// holding a [`RepoPath`] wants [`Pattern::matches_file`], and that is what [`first_match`] and
    /// [`matched_skills`] call.
    ///
    /// The cost is linear in the tokens of a component times the length of the path, and linear in
    /// the components of the pattern times the components of the path: `*` and `**` backtrack
    /// through a mark rather than through a recursive retry of every suffix (see [`toks_match`]).
    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        let parts: Vec<&str> = path.split('/').collect();
        segments_match(&self.segments, &parts)
    }

    /// Whether this pattern matches `file`, [`self.repo`](Self::repo) included: a `<repo>:`
    /// qualifier matches that repo and no other, and a bare glob matches any repo (D71).
    #[must_use]
    pub fn matches_file(&self, file: &RepoPath) -> bool {
        self.repo.as_deref().is_none_or(|repo| repo == file.repo) && self.matches(&file.path)
    }
}

/// [`Pattern::matches`] as a free function, the spelling blueprint §2.1 gives it.
///
/// Both spellings are in the plan's two documents (D71 names the method, blueprint §2.1 this), so
/// both exist and neither is a second definition: this one delegates.
#[must_use]
pub fn matches(pattern: &Pattern, path: &str) -> bool {
    pattern.matches(path)
}

/// The first file of `files`, in file order, that any of `globs` matches (D71).
///
/// A glob that does not compile is skipped, not fatal — see [`compile`]. The order is the
/// caller's file order and nothing else, so the recorded `matched` path is reproducible from the
/// same walk that produced the set.
#[must_use]
pub fn first_match(globs: &[String], files: &[RepoPath]) -> Option<RepoPath> {
    let compiled: Vec<Pattern> = globs.iter().filter_map(|g| compile(g).ok()).collect();
    files
        .iter()
        .find(|file| compiled.iter().any(|p| p.matches_file(file)))
        .cloned()
}

/// Each skill's matched path, rendered `<repo>:<path>` (D71).
///
/// One entry per skill whose `globs` matched something; a skill whose globs matched nothing is
/// absent, which is what `no_match` records and what `no_path` (an absent *file set*) is not. The
/// map is a `BTreeMap` because it rides on [`PromptSpec`](crate::prompt::PromptSpec), whose
/// assembled bytes and digest must not depend on iteration order.
#[must_use]
pub fn matched_skills(skills: &[BoundSkill], files: &[RepoPath]) -> BTreeMap<SkillId, String> {
    let mut matched = BTreeMap::new();
    for skill in skills {
        if let Some(file) = first_match(&skill.globs, files) {
            matched.insert(skill.skill_id, format!("{}:{}", file.repo, file.path));
        }
    }
    matched
}

/// The byte offset of the `<repo>:` qualifier's `:`, or `None` when the pattern has no qualifier.
///
/// A repo slug is never a path, so the scan stops at the first `/`; a `:` inside a character class
/// or behind a `\` is a literal, not a qualifier either.
fn qualifier_end(pattern: &str) -> Option<usize> {
    let mut chars = pattern.char_indices();
    while let Some((at, c)) = chars.next() {
        match c {
            '/' => return None,
            ':' => return Some(at),
            '\\' => {
                chars.next();
            }
            '[' => {
                while let Some((_, c)) = chars.next() {
                    match c {
                        ']' => break,
                        '\\' => {
                            chars.next();
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// One `/`-separated component, at `base` bytes into the original pattern.
fn compile_component(component: &str, base: usize) -> Result<Segment, GlobError> {
    if component == "**" {
        return Ok(Segment::AnyComponents);
    }
    if let Some(at) = misplaced_globstar(component) {
        return Err(GlobError::MisplacedGlobstar { at: base + at });
    }
    match component.find('{') {
        Some(open) => compile_alternation(component, open, base),
        None => Ok(Segment::Parts(tokens(component, base)?)),
    }
}

/// The byte of the first `**` that is not the whole component; escaped stars do not count.
fn misplaced_globstar(component: &str) -> Option<usize> {
    let mut chars = component.char_indices();
    let mut previous_star: Option<usize> = None;
    while let Some((at, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
                previous_star = None;
            }
            '*' => {
                if let Some(at) = previous_star {
                    return Some(at);
                }
                previous_star = Some(at);
            }
            _ => previous_star = None,
        }
    }
    None
}

/// `{a,b}`, opened at `open` in `component`, at `base` bytes into the original pattern.
///
/// D96: an alternation is a whole component, so it is one level deep. An empty alternative is
/// complained about before the alternation's span is, because `foo{,.txt}` is a maintainer's typo
/// in the alternation rather than in its position — and `pre{a,b}fix.rs`, whose alternatives are
/// all fine, is complained about as a nest.
fn compile_alternation(component: &str, open: usize, base: usize) -> Result<Segment, GlobError> {
    let mut chars = component[open + 1..].char_indices();
    let mut close = None;
    let mut content = String::new();
    while let Some((at, c)) = chars.next() {
        match c {
            '\\' => {
                content.push('\\');
                if let Some((_, escaped)) = chars.next() {
                    content.push(escaped);
                }
            }
            '}' => {
                close = Some(open + 1 + at);
                break;
            }
            c => content.push(c),
        }
    }
    let Some(close) = close else {
        return Err(GlobError::UnterminatedBrace { at: base + open });
    };
    let alternatives = split_alternatives(&content, base + open + 1)?;
    if open != 0 || close + 1 != component.len() {
        return Err(GlobError::NestedAlternation { at: base + open });
    }
    let mut branches = Vec::with_capacity(alternatives.len());
    for (text, at) in alternatives {
        branches.push(vec![Segment::Parts(tokens(text, base + at)?)]);
    }
    Ok(Segment::Alt(branches))
}

/// The `{a,b}` content split on its unescaped commas, each piece with its byte in the pattern.
fn split_alternatives(content: &str, base: usize) -> Result<Vec<(&str, usize)>, GlobError> {
    if content.contains('{') || content.contains('}') {
        return Err(GlobError::NestedAlternation {
            at: base + content.find(['{', '}']).expect("just found one"),
        });
    }
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut chars = content.char_indices();
    while let Some((at, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            ',' => {
                pieces.push((&content[start..at], start));
                start = at + 1;
            }
            _ => {}
        }
    }
    pieces.push((&content[start..], start));
    if let Some(at) = pieces
        .iter()
        .find(|(text, _)| text.is_empty())
        .map(|(_, at)| *at)
    {
        return Err(GlobError::EmptyAlternative { at: base + at });
    }
    Ok(pieces)
}

/// The tokens of one component: `*`, `?`, `[…]`, `\` and everything else literal.
fn tokens(component: &str, base: usize) -> Result<Vec<Tok>, GlobError> {
    let mut toks = Vec::new();
    let mut chars = component.char_indices();
    while let Some((at, c)) = chars.next() {
        match c {
            '*' => toks.push(Tok::Star),
            '?' => toks.push(Tok::Any),
            '\\' => match chars.next() {
                Some((_, escaped)) => toks.push(Tok::Char(escaped)),
                None => return Err(GlobError::DanglingEscape { at: base + at }),
            },
            '[' => toks.push(class(chars.by_ref(), base + at)?),
            '{' | '}' => return Err(GlobError::NestedAlternation { at: base + at }),
            c => toks.push(Tok::Char(c)),
        }
    }
    Ok(toks)
}

/// `[abc]`, `[!abc]`, `[a-z]`, with `\` escaping inside. `at` is the `[`, already rebased.
///
/// The body is gathered first and the ranges read out of it, so `]` needs no "unless it is first"
/// exception: the one-character class `[]` is the empty class this dialect refuses.
fn class(chars: &mut std::str::CharIndices<'_>, at: usize) -> Result<Tok, GlobError> {
    let mut body = String::new();
    let mut closed = false;
    while let Some((_, c)) = chars.next() {
        match c {
            ']' => {
                closed = true;
                break;
            }
            '\\' => match chars.next() {
                Some((_, escaped)) => body.push(escaped),
                None => break,
            },
            c => body.push(c),
        }
    }
    if !closed {
        return Err(GlobError::UnterminatedClass { at });
    }
    let Some((negated, ranges)) = ranges(&body) else {
        return Err(GlobError::EmptyClass { at });
    };
    Ok(Tok::Class { negated, ranges })
}

/// The `!` and the ranges of a class body, or `None` when it names no character at all.
///
/// A `-` with nothing after it is a literal `-`, as in `[a-]`, because a range needs two ends.
fn ranges(body: &str) -> Option<(bool, Vec<(char, char)>)> {
    let chars: Vec<char> = body.chars().collect();
    let mut at = 0;
    let negated = chars.first() == Some(&'!');
    if negated {
        at = 1;
    }
    let mut ranges = Vec::new();
    while at < chars.len() {
        let low = chars[at];
        at += 1;
        if chars.get(at) == Some(&'-') && chars.get(at + 1).is_some_and(|c| *c != ']') {
            ranges.push((low, chars[at + 1]));
            at += 2;
        } else {
            ranges.push((low, low));
        }
    }
    (!ranges.is_empty()).then_some((negated, ranges))
}

/// `segs` against `parts`, with `**` free to take any number of components.
///
/// The two-pointer matcher over components, the same shape as [`toks_match`] and for the same
/// reason. `**` is the only source of backtracking: it records where it was, and a later mismatch
/// rewinds to just after it and lets it take one more component. Every rewind advances the
/// position, so the walk is `O(segments + components)` — where "try zero components, then one, then
/// two, recursively, for every `**`" is `O(C(segments + components, components))`, which twenty
/// `**` against twenty components does not finish in a human lifetime. (Blueprint §2.1's descent is
/// that recursive form; its comment calls it bounded, and the adversarial pass measured otherwise.)
///
/// A `**` takes **zero** components first, which is what D71 asks for: `**/*.rs` prefers the
/// shallowest match, and the recorded path is the walk's first.
fn segments_match(segs: &[Segment], parts: &[&str]) -> bool {
    let (mut s, mut p) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    while p < parts.len() {
        match segs.get(s) {
            Some(Segment::AnyComponents) => {
                star = Some(s);
                s += 1;
            }
            Some(segment) if segment_matches(segment, parts[p]) => {
                s += 1;
                p += 1;
            }
            _ => {
                let Some(star_at) = star else {
                    return false;
                };
                s = star_at + 1;
                p += 1;
                star = Some(star_at);
            }
        }
    }
    segs[s..]
        .iter()
        .all(|segment| matches!(segment, Segment::AnyComponents))
}

/// One component of a path against one [`Segment`], which by construction is a whole component.
///
/// An alternation is one component (D96), so each of its branches is the single [`Segment::Parts`]
/// `compile_alternation` built; a `**` is a segment of the pattern, never a match for a component
/// of the path on its own.
fn segment_matches(segment: &Segment, part: &str) -> bool {
    match segment {
        Segment::Parts(toks) => toks_match(toks, part),
        Segment::Alt(branches) => branches.iter().any(
            |branch| matches!(branch.as_slice(), [Segment::Parts(toks)] if toks_match(toks, part)),
        ),
        Segment::AnyComponents => false,
    }
}

/// One component's tokens against one component: the two-pointer matcher, `*` the only token that
/// consumes more than one character.
///
/// The alternative — "on a `*`, try every suffix of what is left, recursively" — costs `O(stars)`
/// per star, so a component with N stars costs `O(len^N)`. That is a hang, not a slow path, and a
/// maintainer can type the pattern: `*a*a*...*z` took minutes against a forty-character component
/// before this form replaced it. Instead one `star` mark remembers where the most recent `*` was and
/// how much it had eaten, and a mismatch rewinds to it and lets it eat one more; every rewind
/// advances the position, so the walk is `O(tokens * len)`.
///
/// `*` never crosses a `/` because the caller already split on it: a component holds no separator,
/// so the `c != '/'` arms below cannot be reached. They are kept as the belt to D71's braces.
fn toks_match(toks: &[Tok], text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let (mut t, mut p) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while p < chars.len() {
        match toks.get(t) {
            Some(Tok::Star) => {
                star = Some((t, p));
                t += 1;
            }
            Some(Tok::Any) if chars[p] != '/' => {
                t += 1;
                p += 1;
            }
            Some(Tok::Class { negated, ranges }) if chars[p] != '/' => {
                let inside = ranges
                    .iter()
                    .any(|(low, high)| chars[p] >= *low && chars[p] <= *high);
                if inside != *negated {
                    t += 1;
                    p += 1;
                } else if let Some((star_at, mark)) = star {
                    t = star_at + 1;
                    p = mark + 1;
                    star = Some((star_at, mark + 1));
                } else {
                    return false;
                }
            }
            Some(Tok::Char(want)) if *want == chars[p] => {
                t += 1;
                p += 1;
            }
            _ => {
                let Some((star_at, mark)) = star else {
                    return false;
                };
                t = star_at + 1;
                p = mark + 1;
                star = Some((star_at, mark + 1));
            }
        }
    }
    toks[t..].iter().all(|tok| *tok == Tok::Star)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::skill::{Activation, SkillLevel};

    fn file(repo: &str, path: &str) -> RepoPath {
        RepoPath {
            repo: repo.to_owned(),
            path: path.to_owned(),
        }
    }

    /// The matcher walks a pattern in proportion to its own size, not exponentially in it. A `*` that
    /// retried every suffix recursively, or a `**` that retried every number of components
    /// recursively, would both be correct and both would hang: the adversarial pass that found this
    /// measured eleven `*` against a forty-character component at minutes, and the shapes a
    /// maintainer types reach either form. These two cases are the shapes, and the assertions are
    /// the answers as well as the cost — a rewrite that made them fast by being wrong would fail.
    #[test]
    fn a_pattern_full_of_stars_matches_in_proportion_to_its_own_size() {
        let stars = "*a".repeat(20);
        let compiled = compile(&stars).expect("twenty stars is a pattern a maintainer can type");

        // Sixty-four characters holding twenty `a`: each `*` eats the `b` before the next one, and
        // the last token is a literal `a`, so the component has to end with one.
        let hit = "b".repeat(25) + &"a" + &"ba".repeat(19);
        assert_eq!(hit.len(), 64, "the component is sixty-four characters long");
        assert!(
            compiled.matches(&hit),
            "twenty `*` against twenty `a` in sixty-four characters matches"
        );

        // The same pattern, and no `a` at all: one pass, and no match.
        let miss = "b".repeat(64);
        assert!(
            !compiled.matches(&miss),
            "the miss is the same walk, not a different one"
        );

        // The same for `**`, whose exponential form is "every number of components, for every `**`".
        let globstars = format!("{}x.rs", "**/".repeat(20));
        let compiled = compile(&globstars).expect("twenty globstars compile as whole components");
        let path = format!("{}x.rs", "a/".repeat(20));
        assert!(
            compiled.matches(&path),
            "twenty `**` against twenty components matches"
        );
        assert!(
            !compiled.matches(&format!("{}x.py", "a/".repeat(20))),
            "and the last component still has to match"
        );
    }

    /// Compiles `pattern` and asserts it does (or does not) match `path`. This is plan D71's dialect
    /// table, which under D70 is the **specification** this module is written to rather than a
    /// cross-check of another implementation.
    #[test]
    fn the_dialect_table() {
        for (pattern, path, expected) in [
            // `*` is a run of non-`/` characters and never crosses one.
            ("*.rs", "main.rs", true),
            ("*.rs", "crates/main.rs", false),
            ("src/*.rs", "src/main.rs", true),
            ("src/*.rs", "crates/main.rs", false),
            ("src/*.rs", "src/a/b.rs", false),
            // `**` is zero or more whole components, and a whole component only.
            ("**/*.rs", "crates/x.rs", true),
            ("**/*.rs", "x.rs", true),
            ("src/**", "src", true),
            ("src/**", "src/a.rs", true),
            ("src/**", "src/a/b.rs", true),
            ("src/**", "srcs/a.rs", false),
            ("src/**", "src2/x.rs", false),
            ("**/src", "src", true),
            ("**/src", "a/src", true),
            ("**/src", "a/b/src", true),
            ("**/src", "a/src2", false),
            ("**", "a/b/c.rs", true),
            ("a/**/b", "a/b", true),
            ("a/**/b", "a/x/y/b", true),
            ("a/**/b", "a/x/y", false),
            // `?` is exactly one non-`/` character.
            ("?.rs", "a.rs", true),
            ("?.rs", "ab.rs", false),
            ("a?c", "abc", true),
            ("a?c", "a/c", false),
            ("src/?.rs", "src/a.rs", true),
            ("src/?.rs", "src/ab.rs", false),
            ("src/?.rs", "src/.rs", false),
            // `*` inside a component is a run of characters, not of components.
            ("src/a*.rs", "src/a.rs", true),
            ("src/a*.rs", "src/abc.rs", true),
            ("src/a*.rs", "src/b.rs", false),
            // `{a,b}` alternates, as a whole component (D96).
            ("{a,b}/x.rs", "a/x.rs", true),
            ("{a,b}/x.rs", "b/x.rs", true),
            ("{a,b}/x.rs", "c/x.rs", false),
            ("src/{a,b}/x.rs", "src/b/x.rs", true),
            // Character classes.
            ("[abc].rs", "a.rs", true),
            ("[abc].rs", "b.rs", true),
            ("[abc].rs", "c.rs", true),
            ("[abc].rs", "d.rs", false),
            ("[!abc].rs", "d.rs", true),
            ("[!abc].rs", "a.rs", false),
            ("[!abc].rs", "b.rs", false),
            ("[a-c].rs", "a.rs", true),
            ("[a-c].rs", "b.rs", true),
            ("[a-c].rs", "c.rs", true),
            ("[a-c].rs", "d.rs", false),
            ("x[0-9][0-9].rs", "x42.rs", true),
            ("x[0-9][0-9].rs", "x4.rs", false),
            // `\` escapes the next metacharacter, so a metacharacter is literal.
            (r"a\*b", "a*b", true),
            (r"a\*b", "axb", false),
            (r"a\?b", "a?b", true),
            (r"a\{b\}.rs", "a{b}.rs", true),
            ("x[0-9\\]].rs", "x].rs", true),
            // Matching is case-sensitive everywhere.
            ("*.RS", "main.rs", false),
            ("*.RS", "X.RS", false),
            ("[a-c].rs", "B.rs", false),
            // A component is a whole path component, not a prefix or a suffix of one.
            ("src", "src", true),
            ("src", "src2/x.rs", false),
            ("crates/ax.rs", "crates/abc.rs", false),
        ] {
            let compiled =
                compile(pattern).unwrap_or_else(|err| panic!("`{pattern}` must compile: {err}"));
            assert_eq!(
                matches(&compiled, path),
                expected,
                "`{pattern}` against `{path}`"
            );
            assert_eq!(
                compiled.matches(path),
                expected,
                "the method agrees with the function"
            );
        }
    }

    /// The `<repo>:` qualifier matches one repo and a bare glob matches every repo (D71). The
    /// qualifier is checked by the entry points that hold a [`RepoPath`], not by
    /// [`Pattern::matches`], whose signature takes a path.
    #[test]
    fn a_qualified_glob_matches_one_repo_and_a_bare_one_matches_every_repo() {
        let qualified = compile("htui:**/*.rs").expect("a qualified glob compiles");
        assert_eq!(qualified.repo.as_deref(), Some("htui"));
        assert!(qualified.matches_file(&file("htui", "crates/x.rs")));
        assert!(
            !qualified.matches_file(&file("other", "crates/x.rs")),
            "the qualifier is the whole reason the pattern carries it"
        );

        let bare = compile("**/*.rs").expect("a bare glob compiles");
        assert_eq!(bare.repo, None);
        for repo in ["htui", "other", ""] {
            assert!(
                bare.matches_file(&file(repo, "crates/x.rs")),
                "a bare glob matches in any repo, including `{repo}`"
            );
        }
    }

    /// `matched_skills` is keyed by skill, valued `<repo>:<path>`, and holds only the skills whose
    /// globs matched something (D71). One call from two places, so the qualified-glob rule is
    /// written once.
    #[test]
    fn matched_skills_records_one_rendered_path_per_matching_skill() {
        let files = vec![file("htui", "src/main.rs"), file("htui", "docs/readme.md")];
        let globbed = |id: SkillId, activation: Activation, globs: &[&str]| BoundSkill {
            skill_id: id,
            name: "house".to_owned(),
            version: Some(1),
            position: 0,
            body: String::new(),
            level: SkillLevel::Project,
            activation,
            globs: globs.iter().map(|g| (*g).to_owned()).collect(),
        };
        let rust = SkillId::new();
        let other = SkillId::new();
        let missed = SkillId::new();
        let switched_off = SkillId::new();
        let always = SkillId::new();
        let matched = matched_skills(
            &[
                globbed(rust, Activation::Glob, &["**/*.rs"]),
                globbed(other, Activation::Glob, &["htui:docs/**"]),
                globbed(missed, Activation::Glob, &["**/*.py"]),
                globbed(switched_off, Activation::Off, &["**/*.rs"]),
                globbed(always, Activation::Always, &["**/*.rs"]),
            ],
            &files,
        );

        assert_eq!(
            matched.get(&rust).map(String::as_str),
            Some("htui:src/main.rs"),
            "the first file in file order wins"
        );
        assert_eq!(
            matched.get(&other).map(String::as_str),
            Some("htui:docs/readme.md")
        );
        assert!(
            !matched.contains_key(&missed),
            "a skill whose globs matched nothing is absent, which is what `no_match` records"
        );
        for (id, activation) in [
            (switched_off, Activation::Off),
            (always, Activation::Always),
        ] {
            assert!(
                !matched.contains_key(&id),
                "a `{activation:?}` attachment is decided by its activation, not by its globs: the \
                 map is what `select` reads to record `matched`, and recording a path for an \
                 attachment that never fired would say it did"
            );
        }
    }

    /// Plan R-25's "compile is total", as a test rather than a claim: over an alphabet of the
    /// dialect's own metacharacters plus one ordinary letter, every pattern of one, two and three
    /// bytes compiles to a `Result` and never panics. A panic here would be a `MemStore` or a
    /// `PgStore` write that aborts the process rather than answering `Constraint` (D78).
    #[test]
    fn no_panic_on_any_short_input() {
        const ALPHABET: [char; 12] = ['*', '?', '[', ']', '{', '}', ',', '\\', '!', '/', ':', 'a'];
        let mut inputs = Vec::new();
        for first in ALPHABET {
            inputs.push(first.to_string());
            for second in ALPHABET {
                inputs.push(format!("{first}{second}"));
                for third in ALPHABET {
                    inputs.push(format!("{first}{second}{third}"));
                }
            }
        }
        assert_eq!(
            inputs.len(),
            12 + 144 + 1728,
            "the sweep is the one this test names"
        );
        for input in &inputs {
            let _ = compile(input);
        }
    }

    /// An empty `globs` list matches nothing, and a glob the matcher cannot compile is skipped
    /// rather than fatal — the writer is where a bad glob is refused (D78), and a glob stored
    /// before a rule tightened must not take every other skill with it.
    #[test]
    fn an_empty_glob_list_and_an_uncompilable_glob_match_nothing_rather_than_panicking() {
        let files = vec![file("htui", "crates/x.rs")];
        assert_eq!(first_match(&[], &files), None, "no globs, no match");
        assert!(
            first_match(&["**/[".to_owned()], &files).is_none(),
            "a glob that does not compile is skipped, not an error at match time"
        );
        assert_eq!(
            first_match(&["**/*.rs".to_owned(), "src/**x/*.rs".to_owned()], &files).map(|f| f.path),
            Some("crates/x.rs".to_owned()),
            "one bad glob among good ones does not lose the good one"
        );
        assert_eq!(
            first_match(&["**/*.rs".to_owned()], &[]),
            None,
            "no files, no match"
        );
    }

    /// One test per [`GlobError`] variant, each asserting the variant **and** the byte position:
    /// the position is what puts the editor's cursor on the character that is wrong (D103).
    #[test]
    fn every_refusal_names_its_byte() {
        let refusals: [(&str, GlobError); 14] = [
            ("", GlobError::Empty),
            ("!**/*.rs", GlobError::Negation { at: 0 }),
            ("src/", GlobError::TrailingSlash { at: 3 }),
            ("htui:/src/*.rs", GlobError::Absolute { at: 5 }),
            ("a/**b/c.rs", GlobError::MisplacedGlobstar { at: 2 }),
            ("a/{b,{c,d}}/x.rs", GlobError::NestedAlternation { at: 5 }),
            ("x{,.rs}", GlobError::EmptyAlternative { at: 2 }),
            ("a/{b,c", GlobError::UnterminatedBrace { at: 2 }),
            ("**/[", GlobError::UnterminatedClass { at: 3 }),
            ("[].rs", GlobError::EmptyClass { at: 1 }),
            ("src/\\", GlobError::DanglingEscape { at: 4 }),
            ("src/\\x.rs", GlobError::DanglingEscape { at: 4 }),
            ("a\0b", GlobError::Nul { at: 1 }),
            (":/**/*.rs", GlobError::BadQualifier { at: 0 }),
        ];
        for (pattern, expected) in refusals {
            let got = compile(pattern).expect_err(&format!("`{pattern:?}` must be refused"));
            assert_eq!(got, expected, "`{pattern:?}`");
        }
    }

    /// `GlobError::Empty` is the one refusal with no position, because it is about the pattern as a
    /// whole: an empty pattern, a bare `:`, and an empty component (`a//b`) all land there. The
    /// empty component is refused rather than folded, so a stored glob means what was typed.
    #[test]
    fn empty_is_the_one_refusal_without_a_position() {
        for pattern in ["", "htui:", "a//b", "htui:a//b"] {
            assert_eq!(
                compile(pattern).expect_err("an empty piece is refused"),
                GlobError::Empty,
                "`{pattern:?}`"
            );
        }
        assert_eq!(GlobError::Empty.at(), None);
        assert_eq!(
            GlobError::Nul { at: 1 }.at(),
            Some(1),
            "every other variant has a position"
        );
    }

    /// An alternation is a whole component and never nests (D96). An empty alternative is
    /// complained about first, because `foo{,.txt}` is a typo in the list rather than in its
    /// position; a well-formed list that is not a whole component is a nest.
    #[test]
    fn an_alternation_is_a_whole_component_and_never_nests() {
        for (pattern, at) in [
            // The offending brace: the outer one when the list is fine and misplaced, the inner
            // one when a list sits inside a list.
            ("pre{a,b}fix.rs", 3),
            ("src/{a,b}x/c.rs", 4),
            ("src/x{a,b}/c.rs", 5),
            ("{a,{b,c}}", 3),
        ] {
            assert_eq!(
                compile(pattern).expect_err("a nested alternation is refused"),
                GlobError::NestedAlternation { at },
                "`{pattern}` names the brace that is in the wrong place"
            );
        }
        for (pattern, at) in [("{,}", 1), ("{a,}", 3), ("{}", 1), ("x{,.rs}", 2)] {
            assert_eq!(
                compile(pattern).expect_err("an empty alternative is refused"),
                GlobError::EmptyAlternative { at },
                "`{pattern}` names where the missing alternative would have started"
            );
        }
        assert!(
            compile("src/{a,b}/x.rs").is_ok(),
            "a whole component is the legal form"
        );
    }
}
