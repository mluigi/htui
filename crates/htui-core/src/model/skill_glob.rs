//! MOD-9 D74: attachment globs. One place turns typed text into stored globs, for the writer and
//! the Skills form's `effective:` line alike. Syntax is `globset`'s (ANA-22 §6 item 7): `*` and
//! `?` never cross `/`, `**` as a whole segment does, `{a,b}`, `[...]`, `\` escapes. Paths are
//! repo-relative with `/`. A bare glob matches in **any** repo of the step's scope (§6 item 6),
//! unlike `touched_paths`' primary-repo rule (R-30).

use crate::model::skill_language::{self, UnknownLanguage};

/// Characters that end the qualifier search: a `:` after any of these is part of the glob.
const META: [char; 7] = ['*', '?', '[', ']', '{', '}', '\\'];

/// One glob, parsed: `<repo>:<glob>` or a bare `<glob>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SkillGlob {
    /// The repo qualifier; `None` for a bare glob.
    pub repo: Option<String>,
    /// The glob proper, compiled once by [`SkillGlob::parse`].
    pub glob: String,
}

impl SkillGlob {
    /// Parses one **trimmed** entry. The qualifier is the text before the first `:` when, trimmed,
    /// it is non-empty and holds no `/` and no glob metacharacter (`*?[]{}\`), so `src/a:b.rs` and
    /// `:foo` are bare. Whitespace around a qualifier's `:` is dropped: `htui: src` is `htui:src`.
    ///
    /// # Errors
    /// [`GlobError::Invalid`]: a NUL anywhere ("contains a NUL character"); nothing after the
    /// qualifier ("has no glob after `<repo>:`"); `globset` refusing the glob (its `kind()`).
    pub fn parse(text: &str) -> Result<Self, GlobError> {
        Self::compiled(text).map(|(glob, _)| glob)
    }

    /// [`SkillGlob::parse`], keeping the matcher it compiled.
    fn compiled(text: &str) -> Result<(Self, globset::GlobMatcher), GlobError> {
        let invalid = |message: String| GlobError::Invalid {
            glob: text.to_owned(),
            message,
        };
        if text.contains('\0') {
            return Err(invalid("contains a NUL character".to_owned()));
        }
        let (repo, glob) = match text
            .split_once(':')
            .map(|(repo, glob)| (repo.trim(), glob.trim()))
        {
            Some((repo, glob))
                if !repo.is_empty() && !repo.contains('/') && !repo.contains(META) =>
            {
                (Some(repo), glob)
            }
            _ => (None, text),
        };
        if let Some(repo) = repo
            && glob.is_empty()
        {
            return Err(invalid(format!("has no glob after `{repo}:`")));
        }
        // `matcher` names the glob proper; the refusal names the entry, qualifier included.
        let compiled = matcher(glob).map_err(|err| match err {
            GlobError::Invalid { message, .. } => invalid(message),
            other @ GlobError::UnknownLanguage(_) => other,
        })?;
        Ok((
            Self {
                repo: repo.map(str::to_owned),
                glob: glob.to_owned(),
            },
            compiled,
        ))
    }
}

/// `<repo>:<glob>` or `<glob>`: the canonical stored text.
impl core::fmt::Display for SkillGlob {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.repo {
            Some(repo) => write!(f, "{repo}:{}", self.glob),
            None => f.write_str(&self.glob),
        }
    }
}

/// Why a list of globs cannot be stored (D94).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GlobError {
    /// One entry is not a glob. `glob` is the whole entry as typed (qualifier included).
    #[error("glob `{glob}`: {message}")]
    Invalid {
        /// The entry.
        glob: String,
        /// Why: `globset`'s `ErrorKind` text, or this module's own sentence.
        message: String,
    },
    /// A language name the map does not hold.
    #[error(transparent)]
    UnknownLanguage(#[from] UnknownLanguage),
}

/// D74 as amended by D94: expand `languages` (refusing an unknown one), then trim `typed`, drop
/// empties, parse each; the result is the typed entries in order followed by the expanded
/// language globs, duplicates dropped keeping the first. Idempotent on its own output.
///
/// # Errors
/// The first [`GlobError`], languages before globs (D78's order).
pub fn canonical_globs(typed: &[String], languages: &[String]) -> Result<Vec<String>, GlobError> {
    let expanded = skill_language::expand(languages)?;
    let mut globs: Vec<String> = Vec::with_capacity(typed.len() + expanded.len());
    for entry in typed {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let glob = SkillGlob::parse(entry)?.to_string();
        if !globs.contains(&glob) {
            globs.push(glob);
        }
    }
    for glob in expanded {
        if !globs.contains(&glob) {
            globs.push(glob);
        }
    }
    Ok(globs)
}

/// D93: a comma list as the Skills form takes it. Splits on `,` only outside `{…}` (depth 0) and
/// `[…]`, with `\x` taken literally; trims each part; drops empties.
#[must_use]
pub fn split_list(text: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut part = String::new();
    // `{` nesting outside a class, and whether a `[...]` class is open. Inside a class `globset`
    // takes every char literally except the closing `]`, and a `]` first in it (after an optional
    // `!` or `^`) is a member; the splitter follows the same rule so it never cuts a class.
    let mut depth = 0_usize;
    let mut class = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        part.push(c);
        if class {
            if c == ']' {
                class = false;
            }
            continue;
        }
        match c {
            '\\' => part.extend(chars.next()),
            '[' => {
                class = true;
                if let Some(&negate @ ('!' | '^')) = chars.peek() {
                    part.push(negate);
                    chars.next();
                }
                if let Some(&']') = chars.peek() {
                    part.push(']');
                    chars.next();
                }
            }
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                part.pop();
                parts.push(core::mem::take(&mut part));
            }
            _ => {}
        }
    }
    parts.push(part);
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A compiled attachment, for PRD milestone 5's `select` (D86). Tested now, called there.
#[derive(Debug, Clone)]
pub struct SkillGlobs {
    /// Each glob's qualifier and matcher, in stored order.
    globs: Vec<(Option<String>, globset::GlobMatcher)>,
}

impl SkillGlobs {
    /// Compiles stored (canonical) globs, keeping their order.
    ///
    /// # Errors
    /// The first entry [`SkillGlob::parse`] refuses.
    pub fn compile(globs: &[String]) -> Result<Self, GlobError> {
        let globs = globs
            .iter()
            .map(|text| SkillGlob::compiled(text).map(|(glob, compiled)| (glob.repo, compiled)))
            .collect::<Result<_, _>>()?;
        Ok(Self { globs })
    }

    /// The first path, in `paths` order, that any glob matches in `repo` — a bare glob in any
    /// repo, a qualified one only in its own. Matching is on raw bytes (`Candidate::from_bytes`).
    #[must_use]
    pub fn first_match<'a>(
        &self,
        repo: &str,
        paths: impl IntoIterator<Item = &'a str>,
    ) -> Option<&'a str> {
        paths.into_iter().find(|path| {
            let candidate = globset::Candidate::from_bytes(path.as_bytes());
            self.globs.iter().any(|(qualifier, compiled)| {
                qualifier.as_deref().is_none_or(|own| own == repo)
                    && compiled.is_match_candidate(&candidate)
            })
        })
    }
}

/// The one builder (D72): `literal_separator(true)`, `backslash_escape(true)`,
/// `empty_alternates(false)`.
fn matcher(glob: &str) -> Result<globset::GlobMatcher, GlobError> {
    globset::GlobBuilder::new(glob)
        .literal_separator(true)
        .backslash_escape(true)
        .empty_alternates(false)
        .build()
        .map(|built| built.compile_matcher())
        .map_err(|err| GlobError::Invalid {
            glob: glob.to_owned(),
            // `err`'s own `Display` already reads "error parsing glob '<g>': …" (F-F).
            message: err.kind().to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|&item| item.to_owned()).collect()
    }

    /// Whether `glob`, compiled alone, matches `path` in repo `r`.
    fn matches(glob: &str, path: &str) -> bool {
        SkillGlobs::compile(&strings(&[glob]))
            .unwrap_or_else(|err| panic!("`{glob}` compiles: {err}"))
            .first_match("r", [path])
            .is_some()
    }

    /// The `message` of an `Invalid` refusal of `text`.
    fn refusal(text: &str) -> String {
        match SkillGlob::parse(text) {
            Err(GlobError::Invalid { glob, message }) => {
                assert_eq!(glob, text, "the refusal names the whole entry");
                message
            }
            other => panic!("`{text:?}` is refused as an invalid glob, got {other:?}"),
        }
    }

    /// D72/D74, pinned against `globset` 0.4.20 as probed (blueprint §0.3, F-S): a bump that moves
    /// any row fails here first.
    #[test]
    fn the_d74_table_matches_as_documented() {
        let table: &[(&str, &str, bool)] = &[
            ("*.rs", "src/a.rs", false),
            ("*.rs", "a.rs", true),
            ("**/*.rs", "a.rs", true),
            ("**/*.rs", "src/a/b.rs", true),
            ("src/*.{rs,toml}", "src/a.toml", true),
            ("src/*.{rs,toml}", "src/x/a.rs", false),
            ("[ab].md", "a.md", true),
            ("[ab].md", "c.md", false),
            ("?.md", "a.md", true),
            ("?.md", "ab.md", false),
            ("?.md", "/.md", false),
            ("src/**", "src/a/b", true),
            ("src/**", "src", false),
            ("a**b", "axxb", true),
            ("{a,}", "a", true),
            ("{a,{b,c}}", "c", true),
            (":foo", ":foo", true),
            ("src/a:b.rs", "src/a:b.rs", true),
        ];
        for &(glob, path, expected) in table {
            assert_eq!(
                matches(glob, path),
                expected,
                "`{glob}` against `{path}` should be {expected}"
            );
        }

        assert_eq!(
            SkillGlob::parse("htui:**/*.rs"),
            Ok(SkillGlob {
                repo: Some("htui".to_owned()),
                glob: "**/*.rs".to_owned(),
            })
        );
        assert_eq!(
            SkillGlob::parse("src/a:b.rs"),
            Ok(SkillGlob {
                repo: None,
                glob: "src/a:b.rs".to_owned(),
            }),
            "a `:` after a `/` is part of the glob"
        );
        assert_eq!(
            SkillGlob::parse(":foo"),
            Ok(SkillGlob {
                repo: None,
                glob: ":foo".to_owned(),
            }),
            "an empty qualifier is no qualifier"
        );
        for text in ["htui:**/*.rs", "src/a:b.rs", ":foo"] {
            let parsed = SkillGlob::parse(text).expect("parses");
            assert_eq!(parsed.to_string(), text, "`Display` is the canonical text");
        }
        for text in ["htui: src", "htui :src"] {
            let parsed = SkillGlob::parse(text);
            assert_eq!(
                parsed,
                Ok(SkillGlob {
                    repo: Some("htui".to_owned()),
                    glob: "src".to_owned(),
                }),
                "whitespace around the `:` of `{text}` is dropped"
            );
            assert_eq!(
                parsed.expect("parses").to_string(),
                "htui:src",
                "`{text}` stores as `htui:src`"
            );
        }
        assert_eq!(
            canonical_globs(&strings(&["htui: src", "htui:src"]), &[]),
            Ok(strings(&["htui:src"])),
            "a spaced qualifier is the same glob as the unspaced one"
        );
        assert_eq!(refusal("htui:  "), "has no glob after `htui:`");
    }

    /// D94: the refusal names the entry and says why, in `globset`'s words or this module's.
    #[test]
    fn a_bad_glob_is_refused_by_name() {
        let message = refusal("src/[a");
        assert!(message.contains("unclosed character class"), "{message}");
        assert_eq!(
            GlobError::Invalid {
                glob: "src/[a".to_owned(),
                message: message.clone(),
            }
            .to_string(),
            format!("glob `src/[a`: {message}"),
            "the glob is named once"
        );

        assert_eq!(refusal("htui:"), "has no glob after `htui:`");
        assert!(refusal("a\0").contains("NUL"), "a NUL is refused by name");

        assert_eq!(
            canonical_globs(&strings(&["**/*.md", " htui:[a "]), &[]),
            Err(GlobError::Invalid {
                glob: "htui:[a".to_owned(),
                message: refusal("htui:[a"),
            }),
            "`canonical_globs` refuses the trimmed entry, qualifier included"
        );
        assert!(
            SkillGlobs::compile(&strings(&["**/*.md", "a}"])).is_err(),
            "`compile` refuses what `parse` refuses"
        );
    }

    /// D74: typed entries trimmed and in order, empties dropped, then the language globs, each
    /// once.
    #[test]
    fn canonical_globs_orders_dedups_and_appends_languages() {
        assert_eq!(
            canonical_globs(&strings(&[" src/** ", "", "**/*.rs"]), &strings(&["Rust"])),
            Ok(strings(&["src/**", "**/*.rs", "**/Cargo.toml"]))
        );
    }

    /// D80's clone passes stored globs back as typed with the stored languages; nothing moves.
    #[test]
    fn canonical_globs_is_idempotent_on_its_output() {
        let out = canonical_globs(
            &strings(&[" src/** ", "htui:docs/*.md", "", "**/*.rs"]),
            &strings(&["Rust"]),
        )
        .expect("canonical");
        assert_eq!(canonical_globs(&out, &strings(&["rust"])), Ok(out.clone()));
        assert_eq!(canonical_globs(&out, &[]), Ok(out));
    }

    /// D94 and D78's order: an unknown language is refused before a bad glob is looked at.
    #[test]
    fn canonical_globs_refuses_a_language_before_a_glob() {
        assert_eq!(
            canonical_globs(&strings(&["[a"]), &strings(&["klingon"])),
            Err(GlobError::UnknownLanguage(UnknownLanguage {
                name: "klingon".to_owned()
            }))
        );
    }

    /// D93 (F-E): D74's own `src/*.{rs,toml}` survives the form's comma list.
    #[test]
    fn split_list_keeps_braces_and_classes_whole() {
        assert_eq!(
            split_list("src/*.{rs,toml}, docs/**,[a,b].md,, a\\,b "),
            strings(&["src/*.{rs,toml}", "docs/**", "[a,b].md", "a\\,b"])
        );
        assert_eq!(
            split_list(" rust, Toml ,"),
            strings(&["rust", "Toml"]),
            "the languages field splits the same way"
        );
        assert_eq!(
            split_list("[],x].md,[!],y].md, [\\,]"),
            strings(&["[],x].md", "[!],y].md", "[\\,]"]),
            "a `]` first in a class is a member, and a `\\` in a class escapes nothing"
        );
        assert!(split_list(" , ").is_empty(), "nothing typed is no entry");
    }

    /// D74/ANA-22 §6 item 6: a qualified glob matches only in its repo, a bare one in any.
    #[test]
    fn first_match_respects_the_qualifier() {
        let globs = SkillGlobs::compile(&strings(&["htui:src/**", "**/*.md"])).expect("compiles");
        assert_eq!(
            globs.first_match("htui", ["a.rs", "src/x.rs"]),
            Some("src/x.rs")
        );
        assert_eq!(
            globs.first_match("web", ["src/x.rs", "README.md"]),
            Some("README.md"),
            "`htui:src/**` does not match in `web`; the bare `**/*.md` does"
        );
        assert_eq!(globs.first_match("web", ["src/x.rs"]), None);
    }
}
