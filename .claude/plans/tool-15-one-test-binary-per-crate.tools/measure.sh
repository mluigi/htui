#!/usr/bin/env bash
# measure.sh <label> <checkout> <out-dir> - TOOL-15 D8: cold and warm test-build cost of one checkout.
#   cold: fresh CARGO_TARGET_DIR, `cargo test --workspace --all-features --no-run`, CARGO_BUILD_JOBS=4
#   warm: touch one htui and one htui-core test file (old or new layout), rebuild with --no-run
# Writes <out-dir>/<label>.log. Run it detached; a cold build takes several minutes.
set -u
label="${1:?label}"; repo="$(cd "${2:?checkout}" && pwd)"; out="${3:?out-dir}"
mkdir -p "$out"; log="$out/$label.log"; tgt="$out/$label.target"
rm -rf "$tgt"; : > "$log"
say() { echo "$*" | tee -a "$log"; }
build() { # $1 = tag
    local s=$SECONDS rc
    ( cd "$repo" && CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR="$tgt" cargo test --workspace --all-features --no-run ) >> "$log" 2>&1
    rc=$?
    say "$1: exit $rc wall $((SECONDS - s))s"
}
say "== $label  $(git -C "$repo" rev-parse --short HEAD)  $(date -Is)"
build cold
say "target size: $(du -sh "$tgt" | cut -f1)   incremental: $(du -sh "$tgt/debug/incremental" 2>/dev/null | cut -f1)"
exes=$(find "$tgt/debug/deps" -maxdepth 1 -type f -perm -u+x ! -name '*.*' | wc -l)
bytes=$(find "$tgt/debug/deps" -maxdepth 1 -type f -perm -u+x ! -name '*.*' -printf '%s\n' | awk '{s += $1} END {print s + 0}')
say "executables: $exes   total bytes: $bytes"
touch_one() { # $1 = old path, $2 = new path
    if [ -f "$repo/$1" ]; then touch "$repo/$1"; else touch "$repo/$2"; fi
}
touch_one crates/htui/tests/backlog.rs crates/htui/tests/it/backlog.rs
build warm-htui-backlog
touch_one crates/htui-core/tests/prompt_render.rs crates/htui-core/tests/it/prompt_render.rs
build warm-core-prompt_render
say "== done $(date -Is)"
