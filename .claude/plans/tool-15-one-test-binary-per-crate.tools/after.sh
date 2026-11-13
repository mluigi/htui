#!/usr/bin/env bash
# after.sh [checkout] [out-dir] - TOOL-15 T6: the after-measurement and what an item run costs.
#   1. measure.sh: cold `cargo test --workspace --all-features --no-run` at 4 jobs (fresh target dir), size,
#      test-executable count and bytes, test-name list, two warm rebuilds (D8; base: 598 s, 28 GB, 147 exes)
#   2. one timed run per big crate of its merged `it` binary at DEFAULT libtest threads, then one run
#      shuffled (seed 11): wall time of an item-sized run, and an order/parallelism-independence check
# Needs the host dev Postgres (HTUI_TEST_DATABASE_URL, defaulted below); Qdrant stays skipped.
# Run it detached and alone: any other cargo job on the machine skews step 1.
#   nohup bash .claude/plans/tool-15-one-test-binary-per-crate.tools/after.sh > /tmp/tool-15-after.out 2>&1 &
# Results: <out-dir>/after.log (summary), after.names, after.exes, run.<crate>.<mode>.log.
set -u
WT="$(cd "${1:-.}" && pwd)"
OUT="${2:-/media/projects/tool-15/out}"
TOOLS="$WT/.claude/plans/tool-15-one-test-binary-per-crate.tools"
export HTUI_TEST_DATABASE_URL="${HTUI_TEST_DATABASE_URL:-postgres://postgres@localhost:5439/postgres}"
unset HTUI_TEST_QDRANT_URL
mkdir -p "$OUT"

bash "$TOOLS/measure.sh" after "$WT" "$OUT"

cd "$WT" || exit 2
export CARGO_TARGET_DIR="$OUT/after.target"
cargo test --workspace --all-features --no-run --message-format=json 2>/dev/null \
  | jq -r 'select(.reason == "compiler-artifact" and .profile.test == true and .executable != null)
           | [.executable, .target.src_path] | @tsv' | sort -u > "$OUT/after.exe-paths"
for c in htui-store htui-agent htui; do
  exe=$(awk -F'\t' -v c="crates/$c/tests/it/main.rs" 'index($2, c) {print $1}' "$OUT/after.exe-paths")
  for mode in parallel shuffled; do
    args=(); [ "$mode" = shuffled ] && args=(-Z unstable-options --shuffle-seed 11)
    s=$SECONDS
    # RUSTC_BOOTSTRAP=1 at run time only: lets stable libtest accept -Z unstable-options (no rebuild).
    ( cd "$WT/crates/$c" && RUSTC_BOOTSTRAP=1 CARGO_MANIFEST_DIR="$WT/crates/$c" "$exe" "${args[@]}" > "$OUT/run.$c.$mode.log" 2>&1 )
    echo "$c $mode exit $? wall $((SECONDS - s))s : $(grep -E '^test result' "$OUT/run.$c.$mode.log" | tail -1)" | tee -a "$OUT/after.log"
  done
done
echo "== AFTER DONE $(date -Is)" | tee -a "$OUT/after.log"
