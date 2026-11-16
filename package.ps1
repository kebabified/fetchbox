# Builds Fetchbox and packages it into dist\:
#   Fetchbox-<version>-setup.exe     installer (current user, no admin)
#   Fetchbox-<version>-portable.zip  unzip-and-run copy that keeps everything in its own folder
#   Fetchbox-<version>-firefox.xpi   the Firefox add-on, signed by Mozilla (tools\sign-firefox.ps1)
#
# Needs what build.ps1 needs, plus Inno Setup for the installer:
#   winget install JRSoftware.InnoSetup --scope user
#
# Usage:  powershell -ExecutionPolicy Bypass -File package.ps1 [-SkipBuild] [-SkipFirefox]
param([switch]$SkipBuild, [switch]$SkipFirefox)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
if (-not $SkipBuild) {
    & powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 -Test
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
}

# what users get: the two programs, the extension, the guide and the licence
$stage = "dist\Fetchbox"
if (Test-Path dist) { Remove-Item dist -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item target\release\fetchbox.exe, target\release\fetchbox-host.exe, GUIDE.md, LICENSE $stage
Copy-Item extension $stage -Recurse
Set-Content "$stage\portable.txt" "This file makes Fetchbox portable: settings, tools and logs all stay in this folder.`r`nDelete it to keep settings in %APPDATA%\Fetchbox instead."

$zip = "dist\Fetchbox-$version-portable.zip"
Compress-Archive -Path $stage -DestinationPath $zip -CompressionLevel Optimal

$iscc = "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe" |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if ($iscc) {
    & $iscc /Qp "/DAppVersion=$version" "/DStage=$PSScriptRoot\$stage" "/DOut=$PSScriptRoot\dist" installer\fetchbox.iss
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
} else {
    Write-Warning "Inno Setup not found - only the portable zip was built."
}
Remove-Item $stage -Recurse -Force

# the Firefox add-on: reused if unchanged, otherwise signed by Mozilla (see tools\sign-firefox.ps1)
if (-not $SkipFirefox) {
    & powershell -NoProfile -ExecutionPolicy Bypass -File tools\sign-firefox.ps1
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
}

Get-ChildItem dist | ForEach-Object {
    "{0}  {1:N1} MB  sha256 {2}" -f $_.Name, ($_.Length / 1MB), (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
}
