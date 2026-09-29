# shellcheck shell=bash
# test_helper.bash — shared fixtures for the TOOL-7 `scripts/hr*` bats suites.
#
# Loaded with `load test_helper` from tests/hr/*.bats (T4's hr_helper.bash reuses it read-only
# for make_fixture). Everything a test writes lives under $BATS_TEST_TMPDIR: the lease state dir,
# the fixture repos. Nothing here may point at /media/projects/htui-hr or at this repo's own
# HANDOFF.md — hr_base_setup asserts the first, and every helper takes an explicit directory.
#
# No bats-assert (not installed): plain [[ ]] and `run --separate-stderr`.

HR_TEST_REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
HR_MINT="$HR_TEST_REPO_ROOT/scripts/hr-mint"
HR_WF_SCRIPTS="$HR_TEST_REPO_ROOT/.claude/skills/handoff-run/scripts"

# Per-test baseline. A suite calls this from its own setup() so a second helper can layer on top.
hr_base_setup() {
    bats_require_minimum_version 1.5.0
    export HR_STATE="$BATS_TEST_TMPDIR/state"
    [[ "$HR_STATE" == "$BATS_TEST_TMPDIR"/* ]] || {
        echo "HR_STATE escaped BATS_TEST_TMPDIR: $HR_STATE" >&2
        return 1
    }
    unset HR_ITEM HR_MINT_LOCK_TIMEOUT
}

# Die unless $1 is inside this test's tmpdir — every fixture path goes through this.
hr_assert_tmp_path() {
    [[ "$1" == "$BATS_TEST_TMPDIR"/* ]] || {
        echo "path outside BATS_TEST_TMPDIR: $1" >&2
        return 1
    }
}

# make_fixture DIR — a one-commit repo on `main` that is green under both
# `next-item-id.sh --repo-root DIR` and `validate-workflow-docs.sh --repo-root DIR`.
# Open: MOD-4, MOD-5. Archived: MOD-3. Tree next for MOD = MOD-6.
make_fixture() {
    local dir="$1"
    hr_assert_tmp_path "$dir" || return 1
    mkdir -p "$dir/docs/decisions/mod"
    git -C "$dir" init -q -b main
    git -C "$dir" config user.name 'hr fixture'
    git -C "$dir" config user.email 'hr-fixture@example.invalid'
    git -C "$dir" config commit.gpgsign false
    cat >"$dir/HANDOFF.md" <<'EOF'
# HANDOFF - Outstanding Work (fixture)

**Current status (2026-01-05):** fixture.

## Open items

### Next features

- [ ] **MOD-4 - Thing.** body
- [ ] **MOD-5 - Other thing.** body

## Summary

| Area  | Open |
|-------|------|
| MOD-N | 2 (MOD-4 thing, MOD-5 other) |
EOF
    cat >"$dir/DECISIONS.md" <<'EOF'
# DECISIONS

- **[MOD-3](docs/decisions/mod/mod-3.md)** - Old thing (done, 2026-01-05)
EOF
    printf '# MOD-3 - Old thing (done, 2026-01-05)\n\nWrite-up.\n' >"$dir/docs/decisions/mod/mod-3.md"
    git -C "$dir" add -A
    git -C "$dir" commit -q -m 'fixture'
}

# lease_file — path of the lease file under the test's HR_STATE.
lease_file() { printf '%s/id-leases.tsv' "$HR_STATE"; }

# init_leases — create the state dir + lease file through the script under test.
init_leases() { "$HR_MINT" --init; }

# seed_lease ID OWNER TS TITLE — append one raw row (fixture only; bypasses hr-mint on purpose).
seed_lease() {
    local f
    f="$(lease_file)"
    hr_assert_tmp_path "$f" || return 1
    [[ -f "$f" ]] || { echo "seed_lease: no lease file at $f (call init_leases)" >&2; return 1; }
    printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" >>"$f"
}

# ts_ago SPEC — UTC ISO timestamp for a GNU `date -d` spec ("1 day ago", "8 days ago").
ts_ago() { date -u -d "$1" +%FT%TZ; }

# lease_rows — the lease file minus comment lines.
lease_rows() { grep -v '^#' "$(lease_file)" || true; }

# lease_ids — the ID column, one per line, in file order.
lease_ids() { lease_rows | cut -f1; }
