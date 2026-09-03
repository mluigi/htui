# workflow-patterns.sh — Shared regex pattern definitions and helpers for handoff workflow scripts.
#
# Single source of truth for section ranks, ID prefixes, index line patterns, open checklist line patterns,
# and compound-aware legacy header patterns.
#
# Kept in lockstep with workflow-patterns.ps1.

PREFIXES=("ANA" "MOD" "NEXT" "VAL" "TOOL" "CLEAN")
PREFIX_ALT="ANA|MOD|NEXT|VAL|TOOL|CLEAN"
# ERE, not PCRE: bash has no `(?:...)`, and the alternation is spelled per-prefix so a
# caller can wrap it in ONE capture group without nesting a second one - every consumer
# below addresses BASH_REMATCH by position.
ID_ALT="ANA-[0-9]+|MOD-[0-9]+|NEXT-[0-9]+|VAL-[0-9]+|TOOL-[0-9]+|CLEAN-[0-9]+"

# Index line pattern in DECISIONS.md
# NOTE: the .ps1 twin writes the title group lazily, `(?<title>.+?)`. Bash ERE has no lazy
# quantifier - `(.+?)` is a repetition operator applied to a repetition operator and
# every [[ =~ ]] against it dies with `repetition-operator operand invalid`, which reads
# as a stderr line per input line and an exit code, i.e. a validator that never validated.
# Greedy is correct here anyway: `.+` backtracks so the trailing `(status, date)` group
# still anchors, and on a title that itself ends in `(x, 2026-01-01)` the LAST pair is
# the one the law means.
#
# The ID group is the six-prefix alternation, not `[A-Z]+`: the twin spells it
# `$PrefixPattern`, and with `[A-Z]+` a line like `- **[FOO-3](docs/decisions/foo/foo-3.md)**
# - ... (done, 2026-01-03)` parses STRICTLY, so a prefix outside the law is silently
# admitted to the ID space instead of being reported unparseable (next-item-id.sh
# --self-test case 7 measures exactly that).
INDEX_LINE_PATTERN="^- \\*\\*\\[(${ID_ALT})\\]\\((docs/decisions/[a-z]+/[a-z]+-[0-9]+\\.md)\\)\\*\\*[[:space:]]*[-–—][[:space:]]*(.+)[[:space:]]*\\(([^,()]+),[[:space:]]*([0-9]{4}-[0-9]{2}-[0-9]{2})\\)[[:space:]]*\$"
INDEX_LINE_LOOSE_PATTERN='^[[:space:]]*[-*+][[:space:]]*\*\*\[([A-Z]+-[0-9]+)\]'

# Open checklist line pattern in HANDOFF.md
CHECKLIST_LINE_PATTERN='^- \[ \] \*\*(([A-Z]+)-[0-9]+)'
CHECKLIST_LINE_LOOSE_PATTERN='^[[:space:]]*[-*+][[:space:]]*\[[^\]]*\][[:space:]]*\*\*([A-Z]+-[0-9]+)'

# Compound-aware legacy write-up header in DECISIONS.md
LEGACY_HEADER_PATTERN='^##[[:space:]]+(ANA|MOD|NEXT|VAL|TOOL|CLEAN)-([0-9]+(/[0-9]+)*)'

parse_legacy_header_ids() {
    local line="$1"
    if [[ "$line" =~ $LEGACY_HEADER_PATTERN ]]; then
        local prefix="${BASH_REMATCH[1]}"
        local nums_str="${BASH_REMATCH[2]}"
        local -a nums
        IFS='/' read -ra nums <<< "$nums_str"
        for n in "${nums[@]}"; do
            echo "${prefix}-${n}"
        done
    fi
}
