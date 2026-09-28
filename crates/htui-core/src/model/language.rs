//! The language → globs map of a `glob` attachment (ANA-22 §6 item 5, §9; plan D83, blueprint
//! §5.2).
//!
//! **Data, expanded at save.** The activation form shows the **effective globs** — the typed globs
//! unioned with every named language's expansion — before the save, and the save writes that
//! union into `skill_binding.globs`, with `languages` keeping what was typed. A later change to
//! this table therefore never changes a saved attachment, and the matcher reads only `globs` and
//! never this file. That is the whole reason it is data and not code (D83).
//!
//! The seed is deliberately small: fourteen languages, ANA-22 §9's list. A name this table does
//! not hold contributes nothing and is **not** an error — the map is a convenience, and the
//! writer's rule is the glob dialect ([`crate::prompt::glob`]), not this list.

/// Language name to the globs it expands into, **sorted by name** so a rendered list and a stored
/// `TEXT[]` are stable and a test can assert the order.
///
/// Fourteen entries, ANA-22 §9's list. Every pattern here is one [`crate::prompt::glob`] compiles
/// — `every_pattern_compiles_under_the_matcher` is the cross-check between this data and that
/// matcher, and it would fail loudly if a language were ever given a `**` in the wrong place.
pub const LANGUAGE_GLOBS: &[(&str, &[&str])] = &[
    ("c", &["**/*.c", "**/*.h"]),
    (
        "cpp",
        &[
            "**/*.cc", "**/*.cpp", "**/*.cxx", "**/*.hh", "**/*.hpp", "**/*.hxx",
        ],
    ),
    ("csharp", &["**/*.cs"]),
    ("go", &["**/*.go"]),
    ("java", &["**/*.java"]),
    ("javascript", &["**/*.js", "**/*.cjs", "**/*.mjs"]),
    ("markdown", &["**/*.md", "**/*.markdown"]),
    ("python", &["**/*.py"]),
    ("rust", &["**/*.rs"]),
    ("shell", &["**/*.sh", "**/*.bash", "**/*.zsh"]),
    ("sql", &["**/*.sql"]),
    ("toml", &["**/*.toml"]),
    ("typescript", &["**/*.ts", "**/*.tsx"]),
    ("yaml", &["**/*.yaml", "**/*.yml"]),
];

/// The effective globs of an attachment: `typed` first, then every named language's patterns in
/// [`LANGUAGE_GLOBS`]'s order, de-duplicated and **in that order** — which is the order the form
/// shows before the save and the order the store receives, so the two cannot disagree (H-33).
///
/// Each language name is trimmed, so a trailing space typed into the form's line still names the
/// language (H-34). A name the map does not hold contributes nothing and is **not** an error.
#[must_use]
pub fn effective_globs<'a>(
    typed: impl IntoIterator<Item = &'a str>,
    named: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for glob in typed {
        if !out.iter().any(|seen| seen == glob) {
            out.push(glob.to_owned());
        }
    }
    for language in named {
        let Some((_, patterns)) = LANGUAGE_GLOBS
            .iter()
            .find(|(name, _)| *name == language.trim())
        else {
            continue;
        };
        for pattern in *patterns {
            if !out.iter().any(|seen| seen == pattern) {
                out.push((*pattern).to_owned());
            }
        }
    }
    out
}

/// The names [`LANGUAGE_GLOBS`] holds, in order: the activation form's list.
#[must_use]
pub fn languages() -> Vec<&'static str> {
    LANGUAGE_GLOBS.iter().map(|(name, _)| *name).collect()
}

#[cfg(test)]
mod tests {
    use super::{LANGUAGE_GLOBS, effective_globs, languages};
    use crate::prompt::glob;

    /// D83 / §5.2: fourteen entries, strictly ascending names, and no two pattern lists equal —
    /// the two things a rendered list and a stored `TEXT[]` depend on.
    #[test]
    fn the_map_is_sorted_and_has_no_duplicate() {
        assert_eq!(
            LANGUAGE_GLOBS.len(),
            14,
            "ANA-22 §9's fourteen languages, and the seed is deliberately small"
        );
        for pair in LANGUAGE_GLOBS.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "`{}` is not before `{}`: the order is what the stored array and the form's list \
                 both rely on",
                pair[0].0,
                pair[1].0
            );
        }
        for (index, (name, patterns)) in LANGUAGE_GLOBS.iter().enumerate() {
            assert!(!patterns.is_empty(), "`{name}` expands to nothing");
            for (other, other_patterns) in &LANGUAGE_GLOBS[index + 1..] {
                assert_ne!(
                    *patterns, *other_patterns,
                    "`{name}` and `{other}` expand to the same globs, so one of them is a \
                     duplicate under another name"
                );
            }
        }
        assert_eq!(
            languages(),
            LANGUAGE_GLOBS
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>(),
            "the completion list is the map's own order"
        );
    }

    /// H-21: the cross-check between this data and the matcher. A pattern here is only useful if
    /// [`glob::compile`] accepts it, and the writer refuses a glob it does not — so a pattern in
    /// the wrong shape would make the map unusable without anything saying so until a save.
    #[test]
    fn every_pattern_compiles_under_the_matcher() {
        for (name, patterns) in LANGUAGE_GLOBS {
            for pattern in *patterns {
                if let Err(err) = glob::compile(pattern) {
                    panic!("`{name}`'s `{pattern}` does not compile: {err}");
                }
            }
        }
    }

    /// H-33: the union is the typed globs first and then every named language's patterns **in the
    /// map's own order**, de-duplicated — the order the form previews is the order the store
    /// receives, so a re-save cannot move a glob for no reason.
    #[test]
    fn the_union_is_the_typed_globs_then_every_language() {
        let union = effective_globs(["docs/**"], ["shell", "rust"]);
        assert_eq!(
            union,
            ["docs/**", "**/*.sh", "**/*.bash", "**/*.zsh", "**/*.rs",],
            "typed first, then each named language's patterns in the order they were named"
        );
        assert_eq!(
            effective_globs(["docs/**"], ["rust", "shell"]),
            ["docs/**", "**/*.rs", "**/*.sh", "**/*.bash", "**/*.zsh"],
            "and in the order the languages were named, which is the form's line order"
        );
        assert_eq!(
            effective_globs(["**/*.rs"], ["rust"]),
            ["**/*.rs"],
            "H-33: a language that only repeats a typed glob adds nothing, so a re-save is stable"
        );
    }

    /// H-34: the form splits its buffer on `\n` and a maintainer types a trailing space.
    #[test]
    fn a_name_with_surrounding_space_still_names_its_language() {
        assert_eq!(
            effective_globs([], ["  shell  "]),
            ["**/*.sh", "**/*.bash", "**/*.zsh"],
            "the name is trimmed before the map is asked"
        );
    }

    /// H-37: an unknown language contributes nothing and is not a refusal. The writer's rule is
    /// the dialect, not the map, and the form's notice is what names the language it did not know.
    #[test]
    fn an_unknown_language_contributes_nothing() {
        assert_eq!(
            effective_globs(["**/*.rs"], ["cobol", "rust"]),
            ["**/*.rs"],
            "an unknown name between a known one changes nothing"
        );
        assert_eq!(
            effective_globs(["**/*.rs"], ["cobol"]),
            ["**/*.rs"],
            "a name the map does not hold adds nothing and is not an error"
        );
        assert_eq!(
            effective_globs([], ["cobol", "fortran"]),
            Vec::<String>::new(),
            "and contributes nothing even when it is the only name"
        );
    }
}
