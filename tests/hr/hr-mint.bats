#!/usr/bin/env bats
# hr-mint.bats — cross-run ID leases (TOOL-7 T3). Contract: .claude/plans/tool-7-hr-sandbox.blueprint.md §T3.
# Run: bats tests/hr/hr-mint.bats

load test_helper

setup() {
    hr_base_setup
    FIX="$BATS_TEST_TMPDIR/repo"
    make_fixture "$FIX"
}

mint() { "$HR_MINT" --repo-root "$FIX" "$@"; }

TS_ROW_RE='^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$'

@test "1. --leasing is 1 without a lease file, 0 after --init; --init is idempotent" {
    run "$HR_MINT" --leasing
    [[ $status -eq 1 ]]
    [[ ! -e "$HR_STATE" ]]

    run "$HR_MINT" --init
    [[ $status -eq 0 ]]
    [[ -d "$HR_STATE" && -f "$(lease_file)" ]]
    [[ "$(head -n 1 "$(lease_file)")" == '# hr-mint leases v1 - owner: scripts/hr-mint (never edit by hand)' ]]
    [[ "$(sed -n 2p "$(lease_file)")" == '# id'$'\t''owner'$'\t''leased_at_utc'$'\t''title' ]]
    [[ "$(wc -l <"$(lease_file)")" -eq 2 ]]

    run "$HR_MINT" --leasing
    [[ $status -eq 0 ]]
    [[ -z "$output" ]]

    cp "$(lease_file)" "$BATS_TEST_TMPDIR/before"
    run "$HR_MINT" --init
    [[ $status -eq 0 ]]
    cmp "$(lease_file)" "$BATS_TEST_TMPDIR/before"
}

@test "2. no state dir: plain tree mint, 'leasing off' on stderr, nothing created" {
    run --separate-stderr mint --prefix MOD --title 'A thing'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-6' ]]
    [[ "$stderr" == *'leasing off'* ]]
    [[ ! -e "$HR_STATE" ]]
}

@test "2b. state dir without a lease file: tree mint, 'leasing off', no file created" {
    mkdir -p "$HR_STATE"
    run --separate-stderr mint --prefix MOD --title 'A thing'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-6' ]]
    [[ "$stderr" == *'leasing off'*'--init'* ]]
    [[ -z "$(ls -A "$HR_STATE")" ]]
}

@test "3. first leased mint is the tree's next ID; one well-formed row, owner host" {
    init_leases
    run --separate-stderr mint --prefix MOD --title 'Ctrl-c leaves the terminal raw'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-6' ]]
    [[ "$stderr" == *'hr-mint: leased MOD-6 for host (tree next MOD-6, lease floor none)'* ]]
    [[ "$(lease_rows | wc -l)" -eq 1 ]]
    local id owner ts title
    IFS=$'\t' read -r id owner ts title < <(lease_rows)
    [[ "$id" == 'MOD-6' ]]
    [[ "$owner" == 'host' ]]
    [[ "$ts" =~ $TS_ROW_RE ]]
    [[ "$title" == 'Ctrl-c leaves the terminal raw' ]]
}

@test "4. lease floor beats the tree: leased MOD-9 -> MOD-10" {
    init_leases
    seed_lease MOD-9 hr/MOD-1 "$(ts_ago '30 days ago')" 'Old lease'
    run --separate-stderr mint --prefix MOD --title 'New'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-10' ]]
    [[ "$stderr" == *'(tree next MOD-6, lease floor MOD-9)'* ]]
}

@test "5. tree beats the lease floor: leased MOD-2 -> MOD-6" {
    init_leases
    seed_lease MOD-2 hr/MOD-1 "$(ts_ago '30 days ago')" 'Ancient lease'
    run --separate-stderr mint --prefix MOD --title 'New'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-6' ]]
    [[ "$stderr" == *'lease floor MOD-2'* ]]
}

@test "6. 20 parallel mints under the flock get exactly MOD-6..MOD-25" {
    init_leases
    local out="$BATS_TEST_TMPDIR/par" i
    local -a pids=()
    mkdir -p "$out"
    for i in $(seq 1 20); do
        HR_MINT_LOCK_TIMEOUT=120 "$HR_MINT" --repo-root "$FIX" --prefix MOD --title "parallel $i" \
            --branch "hr/P-$i" >"$out/$i.out" 2>"$out/$i.err" &
        pids+=("$!")
    done
    local failed=0 p
    for p in "${pids[@]}"; do wait "$p" || failed=$((failed + 1)); done
    [[ $failed -eq 0 ]]

    local want got
    want="$(seq 6 25 | sed 's/^/MOD-/')"
    got="$(cat "$out"/*.out | sort -t- -k2,2n)"
    [[ "$got" == "$want" ]]
    [[ "$(lease_ids | sort -t- -k2,2n)" == "$want" ]]
    [[ "$(lease_rows | wc -l)" -eq 20 ]]
    # Every row well-formed: 4 tab fields, owner hr/P-N, ISO ts, its own title.
    local id owner ts title
    while IFS=$'\t' read -r id owner ts title; do
        [[ "$owner" =~ ^hr/P-([0-9]+)$ ]]
        [[ "$title" == "parallel ${BASH_REMATCH[1]}" ]]
        [[ "$ts" =~ $TS_ROW_RE ]]
        [[ "$(cat "$out/${BASH_REMATCH[1]}.out")" == "$id" ]]
    done < <(lease_rows)
}

@test "7. next-item-id findings block the mint: exit 1, no stdout, lease file untouched" {
    init_leases
    seed_lease MOD-9 hr/MOD-1 "$(ts_ago '1 day ago')" 'Existing'
    cp "$(lease_file)" "$BATS_TEST_TMPDIR/before"

    # MOD-4 open twice -> duplicate-id.
    printf -- '- [ ] **MOD-4 - Thing again.** body\n' >>"$FIX/HANDOFF.md"
    run --separate-stderr mint --prefix MOD --title 'Blocked'
    [[ $status -eq 1 ]]
    [[ -z "$output" ]]
    [[ "$stderr" == *'duplicate-id'* ]]
    cmp "$(lease_file)" "$BATS_TEST_TMPDIR/before"

    # No HANDOFF.md at all -> files finding.
    rm "$FIX/HANDOFF.md"
    run --separate-stderr mint --prefix MOD --title 'Blocked'
    [[ $status -eq 1 ]]
    [[ -z "$output" ]]
    cmp "$(lease_file)" "$BATS_TEST_TMPDIR/before"
}

@test "8. out-of-set prefix is usage (2, no row); lowercase prefix is canonicalized" {
    init_leases
    run --separate-stderr mint --prefix FOO --title 'Nope'
    [[ $status -eq 2 ]]
    [[ -z "$output" ]]
    [[ "$stderr" == *FOO* ]]
    [[ -z "$(lease_rows)" ]]

    run --separate-stderr mint --prefix mod --title 'Yes'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-6' ]]
    [[ "$(lease_ids)" == 'MOD-6' ]]
}

@test "9. empty title is usage (2); tab/CR/newline in a title are stored on one line" {
    init_leases
    run --separate-stderr mint --prefix MOD --title ''
    [[ $status -eq 2 ]]
    [[ -z "$output" ]]
    run --separate-stderr mint --prefix MOD
    [[ $status -eq 2 ]]
    run --separate-stderr mint --prefix MOD --title $' \t\n '
    [[ $status -eq 2 ]]
    [[ -z "$(lease_rows)" ]]

    run --separate-stderr mint --prefix MOD --title $'one\ttwo\nthree\r\nfour'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-6' ]]
    [[ "$(lease_rows | wc -l)" -eq 1 ]]
    [[ "$(lease_rows | cut -f4-)" == 'one two three  four' ]]
}

@test "10. owner: HR_ITEM -> hr/<item>; --branch overrides; neither -> host" {
    init_leases
    HR_ITEM=MOD-65 run --separate-stderr mint --prefix MOD --title 'a'
    [[ $status -eq 0 ]]
    HR_ITEM=MOD-65 run --separate-stderr mint --prefix MOD --title 'b' --branch x
    [[ $status -eq 0 ]]
    run --separate-stderr mint --prefix MOD --title 'c'
    [[ $status -eq 0 ]]
    [[ "$(lease_rows | cut -f1,2 | tr '\t' ' ')" == $'MOD-6 hr/MOD-65\nMOD-7 x\nMOD-8 host' ]]

    run --separate-stderr mint --prefix MOD --title 'd' --branch ''
    [[ $status -eq 2 ]]
}

@test "11. siblings: same prefix within 7 days listed on stderr; old, other prefix and own row not" {
    init_leases
    seed_lease MOD-7 hr/MOD-60 "$(ts_ago '2 days ago')" 'Recent sibling'
    seed_lease MOD-8 hr/MOD-61 "$(ts_ago '8 days ago')" 'Stale sibling'
    seed_lease TOOL-9 hr/MOD-62 "$(ts_ago '1 day ago')" 'Other prefix'
    seed_lease MOD-9 host "$(ts_ago '1 hour ago')" 'Earlier host lease'
    run --separate-stderr mint --prefix MOD --title 'Mine' --branch hr/MOD-65
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-10' ]]   # stdout is the ID and nothing else
    [[ "$stderr" == *'hr-mint: other MOD leases in the last 7 days'*'ask the maintainer before filing'* ]]
    [[ "$stderr" == *'  MOD-7  hr/MOD-60  '*'  Recent sibling'* ]]
    [[ "$stderr" == *'  MOD-9  host  '*'  Earlier host lease'* ]]
    [[ "$stderr" != *'Stale sibling'* ]]
    [[ "$stderr" != *'Other prefix'* ]]
    [[ "$stderr" != *'  MOD-10  '* ]]

    # No recent siblings -> no sibling header at all.
    run --separate-stderr mint --prefix TOOL --title 'Tool thing'
    [[ $status -eq 0 ]]
    [[ "$output" == 'TOOL-10' ]]
    [[ "$stderr" == *'Other prefix'* ]]
    run --separate-stderr mint --prefix ANA --title 'Lonely'
    [[ "$output" == 'ANA-1' ]]
    [[ "$stderr" != *'other ANA leases'* ]]
}

@test "12. --prune drops leases on the ref (open or archived), keeps the rest" {
    init_leases
    local ts
    ts="$(ts_ago '1 day ago')"
    seed_lease MOD-3 hr/A "$ts" 'archived on main'
    seed_lease MOD-5 hr/B "$ts" 'open on main'
    seed_lease MOD-7 hr/C "$ts" 'not on main'
    seed_lease MOD-8 hr/D "$ts" 'not on main, max'
    run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX"
    [[ $status -eq 0 ]]
    [[ -z "$output" ]]
    [[ "$stderr" == *'pruned 2, kept 2'* ]]
    [[ "$(lease_ids | tr '\n' ' ')" == 'MOD-7 MOD-8 ' ]]
    [[ "$(head -n 1 "$(lease_file)")" == '# hr-mint leases v1'* ]]
}

@test "13. --prune keeps each prefix's highest lease as the floor (F1)" {
    init_leases
    local ts
    ts="$(ts_ago '1 day ago')"
    seed_lease MOD-3 hr/A "$ts" 'a'
    seed_lease MOD-4 hr/B "$ts" 'b'
    seed_lease MOD-5 hr/C "$ts" 'c'
    run --separate-stderr "$HR_MINT" --prune --ref main --repo-root "$FIX"
    [[ $status -eq 0 ]]
    [[ "$(lease_ids)" == 'MOD-5' ]]

    run --separate-stderr mint --prefix MOD --title 'next'
    [[ "$output" == 'MOD-6' ]]
    # A stale clone whose tree predates MOD-5 still cannot re-mint it.
    sed -i '/MOD-5 - Other thing/d' "$FIX/HANDOFF.md"
    run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX"
    [[ "$(lease_ids)" == 'MOD-6' ]]
    run --separate-stderr mint --prefix MOD --title 'from a stale tree'
    [[ "$output" == 'MOD-7' ]]
    [[ "$stderr" == *'(tree next MOD-6, lease floor MOD-6)'* ]]
}

@test "14. --purged-owner (repeatable) drops those owners' rows except the per-prefix max" {
    init_leases
    local ts
    ts="$(ts_ago '1 day ago')"
    seed_lease MOD-7 hr/MOD-65 "$ts" 'purged, not max'
    seed_lease MOD-8 hr/keep "$ts" 'survivor'
    seed_lease MOD-10 hr/Y "$ts" 'purged too'
    seed_lease MOD-11 hr/MOD-65 "$ts" 'purged but MOD max'
    seed_lease TOOL-3 hr/MOD-65 "$ts" 'purged but TOOL max'
    run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX" \
        --purged-owner hr/MOD-65 --purged-owner hr/Y
    [[ $status -eq 0 ]]
    [[ "$(lease_ids | tr '\n' ' ')" == 'MOD-8 MOD-11 TOOL-3 ' ]]
}

@test "15. --prune reads the ref, not the worktree" {
    init_leases
    local ts
    ts="$(ts_ago '1 day ago')"
    seed_lease MOD-7 hr/A "$ts" 'new item'
    seed_lease MOD-8 hr/B "$ts" 'max'
    # Uncommitted open line for MOD-7: not on main yet.
    printf -- '- [ ] **MOD-7 - New.** body\n' >>"$FIX/HANDOFF.md"
    run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX"
    [[ $status -eq 0 ]]
    [[ "$(lease_ids | tr '\n' ' ')" == 'MOD-7 MOD-8 ' ]]

    # Committed on another branch: --ref selects it, main still does not prune.
    git -C "$FIX" switch -q -c feature
    git -C "$FIX" commit -q -am 'add MOD-7'
    git -C "$FIX" switch -q main
    run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX"
    [[ "$(lease_ids | tr '\n' ' ')" == 'MOD-7 MOD-8 ' ]]
    run --separate-stderr "$HR_MINT" --prune --ref feature --repo-root "$FIX"
    [[ $status -eq 0 ]]
    [[ "$(lease_ids)" == 'MOD-8' ]]
}

@test "16. --prune on a bad ref is usage (2) and leaves the lease file byte-identical" {
    init_leases
    seed_lease MOD-3 hr/A "$(ts_ago '1 day ago')" 'on main'
    seed_lease MOD-7 hr/A "$(ts_ago '1 day ago')" 'max'
    cp "$(lease_file)" "$BATS_TEST_TMPDIR/before"
    run --separate-stderr "$HR_MINT" --prune --ref no-such-ref --repo-root "$FIX"
    [[ $status -eq 2 ]]
    [[ "$stderr" == *'no-such-ref'* ]]
    cmp "$(lease_file)" "$BATS_TEST_TMPDIR/before"

    # A ref without HANDOFF.md is just as unusable.
    git -C "$FIX" switch -q --orphan empty
    git -C "$FIX" commit -q --allow-empty -m empty
    git -C "$FIX" switch -q main
    run --separate-stderr "$HR_MINT" --prune --ref empty --repo-root "$FIX"
    [[ $status -eq 2 ]]
    cmp "$(lease_file)" "$BATS_TEST_TMPDIR/before"
}

@test "17. a corrupt lease line fails closed (exit 1 naming the line) for mint and prune" {
    init_leases
    seed_lease MOD-9 hr/A "$(ts_ago '1 day ago')" 'fine'
    printf 'MOD-10 hr/B not-a-timestamp title\n' >>"$(lease_file)"
    cp "$(lease_file)" "$BATS_TEST_TMPDIR/before"
    run --separate-stderr mint --prefix MOD --title 'x'
    [[ $status -eq 1 ]]
    [[ -z "$output" ]]
    [[ "$stderr" == *'line 4 unparseable'* ]]
    cmp "$(lease_file)" "$BATS_TEST_TMPDIR/before"

    run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX"
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'line 4 unparseable'* ]]
    cmp "$(lease_file)" "$BATS_TEST_TMPDIR/before"
}

@test "18. lock held elsewhere + HR_MINT_LOCK_TIMEOUT=1 -> exit 3, no row" {
    init_leases
    local lock="$HR_STATE/id-leases.lock"
    local lockfd
    exec {lockfd}>>"$lock"
    flock "$lockfd"
    HR_MINT_LOCK_TIMEOUT=1 run --separate-stderr mint --prefix MOD --title 'waits'
    local rc=$status
    HR_MINT_LOCK_TIMEOUT=1 run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX"
    local prune_rc=$status
    exec {lockfd}>&-
    [[ $rc -eq 3 ]]
    [[ $prune_rc -eq 3 ]]
    [[ -z "$(lease_rows)" ]]

    # Released -> the same mint goes through.
    run --separate-stderr mint --prefix MOD --title 'goes'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-6' ]]
}

@test "19. usage errors exit 2" {
    run --separate-stderr "$HR_MINT"
    [[ $status -eq 2 ]]
    run --separate-stderr "$HR_MINT" --bogus
    [[ $status -eq 2 ]]
    run --separate-stderr "$HR_MINT" --prefix
    [[ $status -eq 2 ]]
    run --separate-stderr "$HR_MINT" --leasing --init
    [[ $status -eq 2 ]]
    run --separate-stderr "$HR_MINT" --prune --prefix MOD --title x
    [[ $status -eq 2 ]]
    run --separate-stderr "$HR_MINT" --prefix MOD --title x --repo-root "$BATS_TEST_TMPDIR/nope"
    [[ $status -eq 2 ]]
    HR_MINT_LOCK_TIMEOUT=soon run --separate-stderr mint --prefix MOD --title x
    [[ $status -eq 2 ]]
    run --separate-stderr "$HR_MINT" --help
    [[ $status -eq 0 ]]
    [[ "$output" == *'Usage'* ]]
}

@test "20. --prune without a lease file is a no-op notice; nothing is created" {
    run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX"
    [[ $status -eq 0 ]]
    [[ "$stderr" == *'leasing off'* ]]
    [[ ! -e "$HR_STATE" ]]
}
