$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
Set-StrictMode -Version Latest

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    & bash (Join-Path $PSScriptRoot 'release.sh') @args
    exit $LASTEXITCODE
}

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $repoRoot
try {
    . (Join-Path $PSScriptRoot 'gates.ps1')

    Invoke-Audit

    & mise exec -- cargo fmt --all
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & mise exec -- cargo clippy --workspace --all-targets --release --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & mise exec -- cargo build --workspace --all-targets --release --locked
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & mise exec -- cargo test --workspace --release --locked
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & mise exec -- cargo install cargo-packager --version 0.11.8 --locked
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & mise exec -- cargo packager --release --packages gpui-agent-desktop --formats nsis --out-dir (Join-Path $repoRoot 'target/release/bundle')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host "Release artifacts: $(Join-Path $repoRoot 'target/release/bundle')"
}
finally {
    Pop-Location
}
