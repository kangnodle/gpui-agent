$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $repoRoot
try {
    . (Join-Path $PSScriptRoot 'gates.ps1')
    Invoke-Audit

    & mise exec -- cargo harness dev --surface desktop
    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
