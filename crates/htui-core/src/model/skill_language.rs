//! MOD-9 D73: the language map (ANA-22 §6 item 5), data not code: `skill_languages.json`,
//! compiled in and parsed once. The writer stores the expanded globs, so the map moving never
//! changes a saved attachment (R-26). No `app_setting` overlay (OQ-19).

use std::collections::BTreeMap;
use std::sync::LazyLock;

/// Language name → globs, in name byte order.
static MAP: LazyLock<BTreeMap<String, Vec<String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("skill_languages.json"))
        .expect("the compiled skill language map parses")
});

/// A language name the map does not hold, after trim and ASCII lowercase (D73).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownLanguage {
    /// The name as normalised.
    pub name: String,
}

// Hand-written rather than `thiserror`'s, because the sentence lists the map (D105): "unknown
// language `klingon` (known: c, cpp, csharp, go, java, javascript, markdown, python, rust, shell,
// sql, toml, typescript, yaml)".
impl core::fmt::Display for UnknownLanguage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "unknown language `{}` (known: ", self.name)?;
        for (at, name) in known().enumerate() {
            if at > 0 {
                f.write_str(", ")?;
            }
            f.write_str(name)?;
        }
        f.write_str(")")
    }
}

impl std::error::Error for UnknownLanguage {}

/// Every language name, in byte order.
pub fn known() -> impl Iterator<Item = &'static str> {
    MAP.keys().map(String::as_str)
}

/// D78's stored `languages`: each trimmed and ASCII-lowercased, empties dropped, duplicates
/// dropped keeping the first. Does not check the names.
#[must_use]
pub fn normalise(languages: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(languages.len());
    for name in languages {
        let name = name.trim().to_ascii_lowercase();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// The globs of `languages` (normalised first), concatenated in input order, duplicates dropped
/// keeping the first.
///
/// # Errors
/// [`UnknownLanguage`] for the first name the map does not hold.
pub fn expand(languages: &[String]) -> Result<Vec<String>, UnknownLanguage> {
    let mut globs: Vec<String> = Vec::new();
    for name in normalise(languages) {
        let Some(own) = MAP.get(&name) else {
            return Err(UnknownLanguage { name });
        };
        for glob in own {
            if !globs.contains(glob) {
                globs.push(glob.clone());
            }
        }
    }
    Ok(globs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::skill_glob::SkillGlob;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|&item| item.to_owned()).collect()
    }

    /// D73: "a unit test compiles every glob in the file", through the parser the writer uses.
    #[test]
    fn every_seed_glob_compiles() {
        let mut seen = 0;
        for name in known() {
            for glob in expand(&strings(&[name])).expect("a known name expands") {
                let parsed = SkillGlob::parse(&glob)
                    .unwrap_or_else(|err| panic!("`{name}`'s glob does not compile: {err}"));
                assert_eq!(parsed.repo, None, "a seed glob is bare: `{glob}`");
                assert_eq!(parsed.to_string(), glob, "a seed glob is canonical");
                seen += 1;
            }
        }
        assert!(
            seen >= 14,
            "every language holds at least one glob ({seen} seen)"
        );
    }

    /// D73: ANA-22 §9's fourteen, and only those, in byte order.
    #[test]
    fn the_fourteen_seed_names_are_present() {
        assert_eq!(
            known().collect::<Vec<_>>(),
            vec![
                "c",
                "cpp",
                "csharp",
                "go",
                "java",
                "javascript",
                "markdown",
                "python",
                "rust",
                "shell",
                "sql",
                "toml",
                "typescript",
                "yaml",
            ]
        );
    }

    /// D73: names are matched after trim and ASCII lowercase; `rust` is exactly the acceptance's
    /// `**/*.rs, **/Cargo.toml`.
    #[test]
    fn names_are_trimmed_and_lowercased() {
        let rust = strings(&["**/*.rs", "**/Cargo.toml"]);
        assert_eq!(expand(&strings(&["Rust"])), Ok(rust.clone()));
        assert_eq!(expand(&strings(&[" rust "])), Ok(rust));
        assert_eq!(
            normalise(&strings(&[" Rust ", "rust", "", "TOML"])),
            strings(&["rust", "toml"]),
            "trimmed, lowercased, empties and duplicates dropped, first kept"
        );
    }

    /// D105: the refusal names the language as normalised and lists the known ones.
    #[test]
    fn an_unknown_language_is_refused_by_name() {
        let err = expand(&strings(&["klingon"])).expect_err("`klingon` is not in the map");
        assert_eq!(
            err,
            UnknownLanguage {
                name: "klingon".to_owned()
            }
        );
        let sentence = err.to_string();
        assert!(sentence.contains("`klingon`"), "{sentence}");
        assert!(sentence.contains("rust"), "{sentence}");
    }
}
