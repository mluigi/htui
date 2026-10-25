# pre-commit.ps1 (TOOL-9) - the Windows body of scripts/githooks/pre-commit. Keep the two in
# lockstep: same pattern, same --scope-paths hand-over, same escape hatch.
$ErrorActionPreference = 'Stop'

$staged = @(git diff --cached --name-only)
$docsPattern = '^HANDOFF\.md$|^DECISIONS\.md$|^docs/decisions/.+\.md$|^docs/ANA-[^/]+\.md$|^\.claude/rules/(workflow|concept)-docs\.md$|^\.clinerules/(workflow|concept)-docs\.md$'

if ($staged.Count -gt 0 -and $env:WORKFLOW_SKIP_HOOK -ne '1' -and ($staged -match $docsPattern)) {
    & pwsh -NoProfile -File '.claude/skills/handoff-run/scripts/validate-workflow-docs.ps1' -ScopePaths ($staged -join "`n")
    if ($LASTEXITCODE -ne 0) {
        [Console]::Error.WriteLine("[workflow-hook] validate-workflow-docs.ps1 exited $LASTEXITCODE - findings above, in files this commit stages.")
        [Console]::Error.WriteLine('[workflow-hook] escape hatch: WORKFLOW_SKIP_HOOK=1 git commit ...')
        exit 1
    }
}
exit 0
