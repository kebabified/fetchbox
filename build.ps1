# Builds Fetchbox and puts fetchbox.exe + fetchbox-host.exe in this folder.
#
# Needs (one time, no admin rights):
#   Rust:      https://rustup.rs  ->  rustup-init -y --default-host x86_64-pc-windows-gnu
#   MinGW-w64: winget install BrechtSanders.WinLibs.POSIX.UCRT
#
# Usage:  powershell -ExecutionPolicy Bypass -File build.ps1 [-Test]
param([switch]$Test)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$mingw = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Directory -Filter "BrechtSanders.WinLibs*" -ErrorAction SilentlyContinue |
    ForEach-Object { Join-Path $_.FullName "mingw64\bin" } | Where-Object { Test-Path $_ } | Select-Object -First 1
$env:PATH = "$env:USERPROFILE\.cargo\bin;" + $(if ($mingw) { "$mingw;" }) + $env:PATH

if ($Test) { cargo test --release --lib; if ($LASTEXITCODE) { exit $LASTEXITCODE } }
cargo build --release
if ($LASTEXITCODE) { exit $LASTEXITCODE }

foreach ($exe in "fetchbox.exe", "fetchbox-host.exe") {
    try { Copy-Item "target\release\$exe" . -Force }
    catch { Write-Warning "$exe is in use - close Fetchbox (and wait for its background service to stop), then build again." }
}
"Built Fetchbox $((Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value)"
