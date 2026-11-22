$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
Set-StrictMode -Version Latest

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $repoRoot
try {
    . (Join-Path $PSScriptRoot 'gates.ps1')

    Invoke-Audit

    & mise exec -- cargo fmt --all
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & mise exec -- cargo clippy --workspace --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & mise exec -- cargo build --workspace --all-targets --locked
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & mise exec -- cargo test --workspace --locked
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    $homeArgs = @('--home', (Join-Path $repoRoot '.gpui-agent-dev'))
    if ($args -contains '--home' -or @($args -like '--home=*').Count -gt 0) { $homeArgs = @() }
    & mise exec -- cargo run --package gpui-agent-desktop --locked -- @homeArgs @args
    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
