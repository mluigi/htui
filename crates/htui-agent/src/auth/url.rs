//! Plan MOD-21 D15: finding the login link in a sentence an adapter wrote for a human.
//!
//! A scan, not a parse. The line an agent prints while it waits is prose — "open the following
//! link to log in: …", or whatever the next agent writes instead — and the one thing every one of
//! them shares is the scheme. Matching a vendor's sentence would be the hard-coded second agent
//! `R-AGT-5` exists to forbid; parsing the result would add a dependency to validate a string this
//! crate only ever hands to [`crate::auth::browser::open_url`], which checks the scheme again
//! before it spawns anything.
//!
//! A miss is cheap on purpose: the pane shows the line whether or not a URL was found, so the
//! worst a false negative costs is the key that opens it. A false *positive* costs nothing either
//! — the opener refuses everything but `http` and `https`, and this scan admits nothing else.

/// What ends a URL besides whitespace: the characters a human, a shell, or a Markdown renderer
/// wraps one in.
///
/// Not `(` or `[`: those appear *inside* real authorisation links, and a link is far more often
/// followed by a closing bracket than wrapped in a matched pair — which is what [`TRAILING`]
/// handles instead.
const CLOSERS: [char; 5] = ['<', '>', '"', '\'', '`'];

/// The punctuation a sentence ends with, stripped from the tail of a match.
///
/// `:` is here as well as in the scheme because a scheme's colon is followed by `//` and can never
/// be the last character of a match.
const TRAILING: [char; 7] = ['.', ',', ';', ':', ')', ']', '}'];

/// The first `http://` or `https://` substring of `line`, extended to the first whitespace or one
/// of `<`, `>`, `"`, `'`, `` ` ``, with trailing `.`, `,`, `;`, `:`, `)`, `]`, `}` stripped.
///
/// Never parsed and never validated beyond the scheme: what comes back is a substring of a line
/// the user is already being shown.
#[must_use]
pub fn first_url(line: &str) -> Option<String> {
    // `https://` contains no `http://` prefix, so the two searches are independent and the smaller
    // index is simply whichever link comes first.
    let start = line
        .find("http://")
        .into_iter()
        .chain(line.find("https://"))
        .min()?;
    let rest = &line[start..];
    let end = rest
        .char_indices()
        .find(|(_, character)| character.is_whitespace() || CLOSERS.contains(character))
        .map_or(rest.len(), |(index, _)| index);
    let url = rest[..end].trim_end_matches(&TRAILING[..]);
    (!url.is_empty()).then(|| url.to_owned())
}
