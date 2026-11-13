#!/usr/bin/env bash
# names.sh [crate] - TOOL-15: every test name of the workspace's (or one crate's) unit and integration
# targets, one per line as "<T|I> <crate>|<name>", sorted. T a test, I an ignored one. Names carry the old
# test-file name as a prefix so the one-binary layout (tests/it/main.rs, libtest names "file::test") and
# the per-file layout (tests/file.rs, libtest names "test") list identically:
#   tests/<f>.rs -> "<f>::"   tests/it/main.rs -> ""   tests/<d>/main.rs -> "<d>::"   other -> "unit(<path>)::"
# The executables come from cargo's own JSON (cargo 1.100 has no target/debug/deps), so the crate and source
# file of each are exact. Run inside a checkout; CARGO_TARGET_DIR is honoured. Workspace mode (no argument)
# reuses a `--workspace --all-features` build. With NAMES_EXES=<file> it also writes one line per
# executable: "<bytes>\t<crate>\t<source path>".
set -u
export LC_ALL=C   # sort and comm must agree on the order
crate="${1:-}"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/names.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
if [ -n "$crate" ]; then sel=(-p "$crate"); else sel=(--workspace); fi

cargo test "${sel[@]}" --all-features --tests --no-run --message-format=json 2>"$tmp/cargo.err" \
    | jq -r 'select(.reason == "compiler-artifact" and .profile.test == true and .executable != null)
             | [.executable, .target.src_path] | @tsv' | sort -u > "$tmp/exes"
if [ ! -s "$tmp/exes" ]; then echo "names.sh: no test executables (build failed?)" >&2; tail -5 "$tmp/cargo.err" >&2; exit 1; fi

: > "$tmp/all"; : > "$tmp/ign"
while IFS=$'\t' read -r exe src; do
    c="${src#*/crates/}"; c="${c%%/*}"
    rel="${src#*/crates/"$c"/}"
    case "$rel" in
        tests/it/main.rs) prefix="" ;;
        tests/*/main.rs) q="${rel#tests/}"; prefix="${q%/main.rs}::" ;;
        tests/*.rs) q="${rel#tests/}"; prefix="${q%.rs}::" ;;
        *) prefix="unit($rel)::" ;;
    esac
    if [ -n "${NAMES_EXES:-}" ]; then printf '%s\t%s\t%s\n' "$(stat -c %s "$exe")" "$c" "$rel" >> "$NAMES_EXES"; fi
    "$exe" --list 2>/dev/null | sed -n 's/: \(test\|benchmark\)$//p' | sed "s|^|$c\|$prefix|" >> "$tmp/all"
    "$exe" --list --ignored 2>/dev/null | sed -n 's/: \(test\|benchmark\)$//p' | sed "s|^|$c\|$prefix|" >> "$tmp/ign"
done < "$tmp/exes"

sort -o "$tmp/all" "$tmp/all"; sort -o "$tmp/ign" "$tmp/ign"
# --list prints every test; --list --ignored prints only the ignored ones.
{ comm -23 "$tmp/all" "$tmp/ign" | sed 's/^/T /'; sed 's/^/I /' "$tmp/ign"; } | sort
