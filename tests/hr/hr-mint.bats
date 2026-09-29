#!/usr/bin/env bats
# hr-mint.bats — cross-run ID leases (TOOL-7 T3). Contract: .claude/plans/tool-7-hr-sandbox.blueprint.md §T3.
# Run: bats tests/hr/hr-mint.bats

load test_helper

setup() {
    hr_base_setup
    # Host behaviour unless a case opts in; the host tree override never points at a real path.
    unset HR_SANDBOX
    export HR_HOST_TREE="$BATS_TEST_TMPDIR/no-host-tree"
    # scripts/hr's host-only marker dir: never the real /media/projects/htui-hr/.runs.
    export HR_RUNS="$BATS_TEST_TMPDIR/runs"
    FIX="$BATS_TEST_TMPDIR/repo"
    make_fixture "$FIX"
}

mint() { "$HR_MINT" --repo-root "$FIX" "$@"; }

TS_ROW_RE='^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$'

@test "0. fixture is green under next-item-id.sh and validate-workflow-docs.sh; tree next MOD-6" {
    run --separate-stderr bash "$HR_WF_SCRIPTS/next-item-id.sh" --prefix MOD --id-only --repo-root "$FIX"
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-6' ]]
    run --separate-stderr bash "$HR_WF_SCRIPTS/validate-workflow-docs.sh" --repo-root "$FIX"
    [[ $status -eq 0 ]]
    [[ "$output" == *'0 error(s), 0 warning(s)'* ]]
    [[ -z "$(git -C "$FIX" status --porcelain)" ]]
}

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
    local id owner ts title k
    while IFS=$'\t' read -r id owner ts title; do
        [[ "$owner" =~ ^hr/P-([0-9]+)$ ]]
        k="${BASH_REMATCH[1]}"
        [[ "$title" == "parallel $k" ]]
        [[ "$ts" =~ $TS_ROW_RE ]]
        [[ "$(cat "$out/$k.out")" == "$id" ]]
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
    [[ "$stderr" == *'(tree next MOD-5, lease floor MOD-6)'* ]]
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

    run --separate-stderr "$HR_MINT" --prune --ref --output=x --repo-root "$FIX"
    [[ $status -eq 2 ]]
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

    # An unreadable lease file never reads as empty (that would drop the floor to 0) and never
    # falls back to a tree-only mint.
    rm "$(lease_file)"
    mkdir "$(lease_file)"
    run --separate-stderr "$HR_MINT" --leasing
    [[ $status -eq 0 ]]
    run --separate-stderr mint --prefix MOD --title 'x'
    [[ $status -eq 3 ]]
    [[ -z "$output" ]]
    rmdir "$(lease_file)"
    if [[ $(id -u) -ne 0 ]]; then
        cp "$BATS_TEST_TMPDIR/before" "$(lease_file)"
        chmod 000 "$(lease_file)"
        run --separate-stderr mint --prefix MOD --title 'x'
        [[ $status -eq 3 ]]
        [[ -z "$output" ]]
        run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX"
        [[ $status -eq 3 ]]
        chmod 600 "$(lease_file)"
    fi
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

@test "21. sandbox: no state dir or no lease file blocks (exit 1), never a tree-only mint" {
    HR_SANDBOX=1 run --separate-stderr mint --prefix MOD --title 'A thing'
    [[ $status -eq 1 ]]
    [[ -z "$output" ]]
    [[ "$stderr" == *'sandbox'*'no state dir'* ]]
    [[ "$stderr" != *'leasing off'* ]]
    [[ ! -e "$HR_STATE" ]]

    mkdir -p "$HR_STATE"
    HR_SANDBOX=1 run --separate-stderr mint --prefix MOD --title 'A thing'
    [[ $status -eq 1 ]]
    [[ -z "$output" ]]
    [[ "$stderr" == *'sandbox'*'no lease file'* ]]
    [[ "$stderr" != *'leasing off'* ]]
    HR_SANDBOX=1 run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX"
    [[ $status -eq 1 ]]
    [[ -z "$(ls -A "$HR_STATE")" ]]

    # Leasing is always on in a sandbox, so the skills reach the blocked mint rather than
    # falling back to next-item-id.sh.
    HR_SANDBOX=1 run --separate-stderr "$HR_MINT" --leasing
    [[ $status -eq 0 ]]
    # The lease file is the host's to create or restore: a sandbox never recreates it.
    HR_SANDBOX=1 run --separate-stderr "$HR_MINT" --init
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'host'* ]]
    HR_SANDBOX=1 run --separate-stderr "$HR_MINT" --init --force
    [[ $status -eq 1 ]]
    [[ -z "$(ls -A "$HR_STATE")" ]]

    # With the lease file in place a sandbox mints as usual.
    init_leases
    HR_SANDBOX=1 HR_ITEM=MOD-65 run --separate-stderr mint --prefix MOD --title 'A thing'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-6' ]]
    [[ "$(lease_rows | cut -f1,2 | tr '\t' ' ')" == 'MOD-6 hr/MOD-65' ]]
}

@test "22. host: --init creates the lock; a vanished lease file blocks --init, mint and prune; --init --force restores it" {
    run --separate-stderr "$HR_MINT" --init
    [[ $status -eq 0 ]]
    [[ -f "$HR_STATE/id-leases.lock" ]]
    seed_lease MOD-9 hr/A "$(ts_ago '1 day ago')" 'the lost floor'
    rm "$(lease_file)"

    run --separate-stderr "$HR_MINT" --init
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'lease file vanished'*'scripts/hr-mint --init --force'* ]]
    [[ ! -e "$(lease_file)" ]]

    # Nothing turns leasing off, so the skills reach the blocked mint instead of the plain one.
    run --separate-stderr "$HR_MINT" --leasing
    [[ $status -eq 0 ]]
    run --separate-stderr mint --prefix MOD --title 'x'
    [[ $status -eq 1 ]]
    [[ -z "$output" ]]
    [[ "$stderr" == *'lease file vanished'* ]]
    [[ "$stderr" != *'leasing off'* ]]
    run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX"
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'lease file vanished'* ]]
    [[ ! -e "$(lease_file)" ]]

    run --separate-stderr "$HR_MINT" --init --force
    [[ $status -eq 0 ]]
    [[ "$stderr" == *"created $(lease_file)"* ]]
    [[ "$(wc -l <"$(lease_file)")" -eq 2 ]]
    run --separate-stderr mint --prefix MOD --title 'x'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-6' ]]
}

@test "23. --force only goes with --init; on an existing lease file --init --force changes nothing" {
    run --separate-stderr "$HR_MINT" --force
    [[ $status -eq 2 ]]
    run --separate-stderr mint --prefix MOD --title x --force
    [[ $status -eq 2 ]]
    run --separate-stderr "$HR_MINT" --prune --force --repo-root "$FIX"
    [[ $status -eq 2 ]]
    [[ ! -e "$HR_STATE" ]]

    init_leases
    seed_lease MOD-9 hr/A "$(ts_ago '1 day ago')" 'keep me'
    cp "$(lease_file)" "$BATS_TEST_TMPDIR/before"
    run --separate-stderr "$HR_MINT" --init --force
    [[ $status -eq 0 ]]
    cmp "$(lease_file)" "$BATS_TEST_TMPDIR/before"
}

@test "24. control characters: stripped from a --title on write, and from lease rows and prune lines on print" {
    init_leases
    local esc=$'\e' bel=$'\a'
    seed_lease MOD-7 $'hr/\e[2Jevil' "$(ts_ago '1 day ago')" $'Title \e]0;pwned\a\e[31mred\e[0m'
    seed_lease MOD-3 $'hr/\e[2Jevil' "$(ts_ago '1 day ago')" $'on main \e[5m'
    run --separate-stderr mint --prefix MOD --title $'Mine \e[31mred\e[0m\x7f' --branch $'hr/\e[1mB'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-8' ]]
    [[ "$stderr" != *"$esc"* && "$stderr" != *"$bel"* ]]
    [[ "$stderr" == *'  MOD-7  hr/[2Jevil  '*'  Title ]0;pwned[31mred[0m'* ]]
    [[ "$(lease_rows | tail -n 1 | cut -f2)" == 'hr/[1mB' ]]
    [[ "$(lease_rows | tail -n 1 | cut -f4-)" == 'Mine [31mred[0m' ]]

    run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX"
    [[ $status -eq 0 ]]
    [[ "$stderr" == *'prune MOD-3 (hr/[2Jevil): on main'* ]]
    [[ "$stderr" != *"$esc"* ]]

    # An unparseable line is echoed on the host terminal as well.
    printf 'junk \e[2J line\n' >>"$(lease_file)"
    run --separate-stderr mint --prefix MOD --title x
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'unparseable'*'junk [2J line'* ]]
    [[ "$stderr" != *"$esc"* ]]
}

@test "25. sandbox: a readable host tree raises the mint and never lowers it; the host itself ignores it" {
    init_leases
    # The run's tree is ahead of a plain host tree: the host next never lowers the mint.
    printf -- '- [ ] **MOD-9 - Run-only item.** body\n' >>"$FIX/HANDOFF.md"
    local low="$BATS_TEST_TMPDIR/host-low" high="$BATS_TEST_TMPDIR/host-high"
    make_fixture "$low"
    HR_SANDBOX=1 HR_HOST_TREE="$low" run --separate-stderr mint --prefix MOD --title 'a'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-10' ]]
    [[ "$stderr" == *'(tree next MOD-10, host next MOD-6, lease floor none)'* ]]

    # Host main carries an item that was never leased (filed elsewhere): it raises the mint.
    make_fixture "$high"
    printf -- '- [ ] **MOD-12 - Filed on another machine.** body\n' >>"$high/HANDOFF.md"
    HR_SANDBOX=1 HR_HOST_TREE="$high" run --separate-stderr mint --prefix MOD --title 'b'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-13' ]]
    [[ "$stderr" == *'(tree next MOD-10, host next MOD-13, lease floor MOD-10)'* ]]
    [[ "$(lease_ids | tr '\n' ' ')" == 'MOD-10 MOD-13 ' ]]

    # On the host the host tree is not consulted.
    HR_HOST_TREE="$high" run --separate-stderr mint --prefix MOD --title 'c'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-14' ]]
    [[ "$stderr" == *'(tree next MOD-10, lease floor MOD-13)'* ]]
    [[ "$stderr" != *'host next'* ]]
}

@test "26. sandbox: an unreadable host tree is skipped with a note; a failing host mint only warns" {
    init_leases
    HR_SANDBOX=1 run --separate-stderr mint --prefix MOD --title 'a'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-6' ]]
    [[ "$stderr" == *"host tree $HR_HOST_TREE not readable - skipped"* ]]
    [[ "$stderr" == *'(tree next MOD-6, host next unavailable, lease floor none)'* ]]

    if [[ $(id -u) -ne 0 ]]; then
        local locked="$BATS_TEST_TMPDIR/host-locked"
        make_fixture "$locked"
        chmod 000 "$locked"
        HR_SANDBOX=1 HR_HOST_TREE="$locked" run --separate-stderr mint --prefix MOD --title 'b'
        chmod 755 "$locked"
        [[ $status -eq 0 ]]
        [[ "$output" == 'MOD-7' ]]
        [[ "$stderr" == *"host tree $locked not readable - skipped"* ]]
    fi

    # The host tree is not this run's data: its findings warn, they do not block.
    local broken="$BATS_TEST_TMPDIR/host-broken"
    make_fixture "$broken"
    printf -- '- [ ] **MOD-40 - Twice.** body\n- [ ] **MOD-40 - Twice.** body\n' >>"$broken/HANDOFF.md"
    HR_SANDBOX=1 HR_HOST_TREE="$broken" run --separate-stderr mint --prefix MOD --title 'c'
    [[ $status -eq 0 ]]
    [[ "$output" == 'MOD-8' || ( $(id -u) -eq 0 && "$output" == 'MOD-7' ) ]]
    [[ "$stderr" == *'duplicate-id'* ]]
    [[ "$stderr" == *'warning: host tree mint failed'*'exit 1'* ]]
    [[ "$stderr" == *'host next unavailable'* ]]
}

@test "27. host: scripts/hr's marker naming this state dir, lease file gone -> vanished even without a lock stamp" {
    mkdir -p "$HR_RUNS" "$HR_STATE"
    # scripts/hr records the state dir it turned leasing on for; a sandbox then deleted both the
    # lease file and the lock (or they never survived): the marker still says leasing is on.
    printf '%s\n' "$BATS_TEST_TMPDIR/other-state" "$HR_STATE" >"$HR_RUNS/.leases-initialized"
    run --separate-stderr "$HR_MINT" --leasing
    [[ $status -eq 0 ]]
    run --separate-stderr mint --prefix MOD --title 'x'
    [[ $status -eq 1 && -z "$output" && "$stderr" == *'lease file vanished'*'.leases-initialized'* ]]
    run --separate-stderr "$HR_MINT" --prune --repo-root "$FIX"
    [[ $status -eq 1 && "$stderr" == *'lease file vanished'* ]]
    run --separate-stderr "$HR_MINT" --init
    [[ $status -eq 1 && "$stderr" == *'lease file vanished'* ]]
    [[ ! -e "$(lease_file)" ]]
    # The same state dir spelled differently still matches.
    HR_STATE="$HR_STATE/." run --separate-stderr mint --prefix MOD --title 'x'
    [[ $status -eq 1 && "$stderr" == *'lease file vanished'* ]]

    # In a sandbox the marker is never read (HR_RUNS is host-only): the sandbox message.
    HR_SANDBOX=1 run --separate-stderr mint --prefix MOD --title 'x'
    [[ $status -eq 1 && "$stderr" == *'sandbox'*'no lease file'* && "$stderr" != *'.leases-initialized'* ]]

    # A marker for another state dir only: leasing off, the plain tree mint.
    printf '%s\n' "$BATS_TEST_TMPDIR/other-state" >"$HR_RUNS/.leases-initialized"
    run --separate-stderr "$HR_MINT" --leasing
    [[ $status -eq 1 ]]
    run --separate-stderr mint --prefix MOD --title 'x'
    [[ $status -eq 0 && "$output" == MOD-6 && "$stderr" == *'leasing off'* ]]

    # --init --force restores it.
    printf '%s\n' "$HR_STATE" >"$HR_RUNS/.leases-initialized"
    run --separate-stderr "$HR_MINT" --init --force
    [[ $status -eq 0 && -f "$(lease_file)" ]]
    run --separate-stderr mint --prefix MOD --title 'x'
    [[ $status -eq 0 && "$output" == MOD-6 ]]
}
