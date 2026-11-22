# Shared monorepo gates, dot-sourced by dev.ps1, run.ps1 and release.ps1.
# Each gate runs through mise and the Cargo harness so scripts and CI share one definition:
#   audit = cargo-deny + cargo-machete + cargo-geiger
# Set SKIP_GATES=1 to bypass them (e.g. offline, where cargo-deny cannot fetch advisories).

function Assert-GateTools {
    param([string[]] $Tools)
    $missing = @()
    foreach ($tool in $Tools) {
        & mise exec -- cargo $tool --version *> $null
        if ($LASTEXITCODE -ne 0) { $missing += "cargo-$tool" }
    }
    if ($missing.Count -gt 0) {
        [Console]::Error.WriteLine("Missing tools: $($missing -join ' ')")
        [Console]::Error.WriteLine('Install: mise install, then mise run audit:install')
        exit 1
    }
}

function Invoke-Audit {
    if ($env:SKIP_GATES -eq '1') { return }
    Assert-GateTools @('deny', 'machete', 'geiger')
    & mise run audit
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
