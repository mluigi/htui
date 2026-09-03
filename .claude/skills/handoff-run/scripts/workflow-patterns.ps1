# workflow-patterns.ps1 — Shared regex pattern definitions and helpers for handoff workflow scripts.
#
# Single source of truth for section ranks, ID prefixes, index line patterns, open checklist line patterns,
# and compound-aware legacy header patterns.
#
# Kept in lockstep with workflow-patterns.sh.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Prefixes = @('ANA', 'MOD', 'NEXT', 'VAL', 'TOOL', 'CLEAN')
$PrefixPattern = '(?:ANA|MOD|NEXT|VAL|TOOL|CLEAN)'

# Index line in DECISIONS.md, per .claude/rules/workflow-docs.md "Index line format".
# Separator alternation accepts hyphen/en dash/em dash for mojibake tolerance.
$IndexLinePattern = "^- \*\*\[(?<id>$PrefixPattern-\d+)\]\((?<path>docs/decisions/[a-z]+/[a-z]+-\d+\.md)\)\*\*\s*[-–—]\s*(?<title>.+?)\s*\((?<status>[^,()]+),\s*(?<date>\d{4}-\d{2}-\d{2})\)\s*$"

# Loose index line pattern to catch unparseable/malformed index entries.
$IndexLineLoosePattern = '^\s*[-*+]\s*\*\*\[(?<id>[A-Z]+-\d+)\]'

# Open checklist line in HANDOFF.md, strict + loose.
$ChecklistLinePattern = "^- \[ \] \*\*(?<id>$PrefixPattern-\d+)\b"
$ChecklistLineLoosePattern = '^\s*[-*+]\s*\[[^\]]*\]\s*\*\*(?<id>[A-Z]+-\d+)\b'

# Compound-aware legacy write-up header in DECISIONS.md.
# E.g. `## MOD-18/19/20 - Relocated to engine as MOD-2/3/4`
$LegacyHeaderPattern = "^##\s+(?<prefix>$PrefixPattern)-(?<nums>\d+(?:/\d+)*)"

function Get-LegacyHeaderIds {
    param([string]$Line)
    if ($Line -match $LegacyHeaderPattern) {
        $prefix = $Matches['prefix']
        $nums = $Matches['nums'] -split '/'
        return @($nums | ForEach-Object { "$prefix-$_" })
    }
    return @()
}
