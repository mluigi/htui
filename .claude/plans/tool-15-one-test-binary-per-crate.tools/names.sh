#!/usr/bin/env bash
# names.sh <crate> - TOOL-15: every test name of a crate's unit and integration targets, one per line,
# prefixed with the old test-file name so a one-binary layout (tests/it/main.rs, names "file::test")
# and the per-file layout (tests/file.rs, names "test") list identically. "T <name>" a test,
# "I <name>" an ignored one. Run it inside the checkout to list; output is sorted.
#   tests/<f>.rs -> prefix "<f>::"   tests/it/main.rs -> no prefix   src/... -> prefix "unit(<path>)::"
#   tests/<d>/main.rs (d != it) -> prefix "<d>::"
set -u
crate="${1:?usage: names.sh <crate>}"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/names.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

list() { # $1 = extra libtest flag (may be empty)
    # shellcheck disable=SC2086
    cargo test -p "$crate" --all-features --tests --no-fail-fast -- --list $1 2>&1 | awk '
        /^ +Running / {
            p = $0; sub(/^ +Running /, "", p); sub(/ \(.*$/, "", p)
            if (p ~ /^tests\/it\/main\.rs$/) prefix = ""
            else if (p ~ /^tests\/[^\/]+\.rs$/) { sub(/^tests\//, "", p); sub(/\.rs$/, "", p); prefix = p "::" }
            else if (p ~ /^tests\/[^\/]+\/main\.rs$/) { sub(/^tests\//, "", p); sub(/\/main\.rs$/, "", p); prefix = p "::" }
            else prefix = "unit(" p ")::"
            next
        }
        /: (test|benchmark)$/ { n = $0; sub(/: (test|benchmark)$/, "", n); print prefix n }
    '
}

list "" | sort > "$tmp/all"
list "--ignored" | sort > "$tmp/ign"
# --list prints every test; --list --ignored prints only the ignored ones.
{ comm -23 "$tmp/all" "$tmp/ign" | sed 's/^/T /'; sed 's/^/I /' "$tmp/ign"; } | sort
