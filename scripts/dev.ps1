$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $repoRoot
try {
    . (Join-Path $PSScriptRoot 'gates.ps1')
    Invoke-Audit

    & mise exec -- cargo run --package gpui-agent-desktop --locked -- --home (Join-Path $repoRoot '.gpui-agent-dev')
    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
