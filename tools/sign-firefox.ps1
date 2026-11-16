# Signs the Firefox add-on with Mozilla so it installs permanently (channel "unlisted": signed, but not
# shown in the Firefox add-ons store). Writes:
#   dist\Fetchbox-<version>-firefox.xpi   the signed add-on, to attach to the GitHub release
#   firefox\updates.json                  tells installed copies about this version (commit it after
#                                         the release is published)
#
# Needs your addons.mozilla.org API credentials (https://addons.mozilla.org/developers/addon/api/key/).
# They are read from WEB_EXT_API_KEY / WEB_EXT_API_SECRET, or asked for (the secret is hidden as you
# type). They are never written to disk.
#
# Run after package.ps1 (which empties dist\):
#   powershell -ExecutionPolicy Bypass -File tools\sign-firefox.ps1
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot)

$manifest = Get-Content extension\manifest.json -Raw | ConvertFrom-Json
$version = $manifest.version
$id = $manifest.browser_specific_settings.gecko.id
$repo = "kebabified/fetchbox"

# Node.js: the copy Fetchbox downloaded for cobalt, or one on PATH
$node = @("runtime\node\node.exe", (Get-Command node -ErrorAction SilentlyContinue).Source) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $node) { throw "Node.js not found. Install it (winget install OpenJS.NodeJS.LTS) or let Fetchbox set up cobalt once." }
$node = (Resolve-Path $node).Path
$npm = Join-Path (Split-Path $node) "node_modules\npm\bin\npm-cli.js"
$env:Path = (Split-Path $node) + ";" + $env:Path   # npm's install scripts call "node"

# Mozilla's web-ext tool, kept in tools\.web-ext (not committed)
$webext = "tools\.web-ext\node_modules\web-ext\bin\web-ext.js"
if (-not (Test-Path $webext)) {
    Write-Host "Installing web-ext..."
    if (Test-Path $npm) { & $node $npm install --prefix tools\.web-ext --no-audit --no-fund --loglevel error web-ext@8.10.0 }
    else { & npm install --prefix tools\.web-ext --no-audit --no-fund --loglevel error web-ext@8.10.0 }
    if (-not (Test-Path $webext)) { throw "Couldn't install web-ext" }
}

$cleanup = @()
if (-not $env:WEB_EXT_API_KEY) {
    # the issuer is the short "user:…" value; refuse anything else (e.g. the secret pasted in the wrong box)
    do {
        $issuer = (Read-Host "JWT issuer (the short one, like user:12345:67)").Trim()
        if ($issuer -notmatch '^user:\d+:\d+$') { Write-Host "That isn't the issuer. It starts with 'user:'. Try again." -ForegroundColor Yellow }
    } until ($issuer -match '^user:\d+:\d+$')
    $env:WEB_EXT_API_KEY = $issuer
    $cleanup += "WEB_EXT_API_KEY"
}
if (-not $env:WEB_EXT_API_SECRET) {
    $secure = Read-Host "JWT secret (the long one; hidden as you type)" -AsSecureString
    $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try { $env:WEB_EXT_API_SECRET = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr).Trim() }
    finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }
    $cleanup += "WEB_EXT_API_SECRET"
}

$out = "dist\firefox-signed"
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
try {
    Write-Host "Sending Fetchbox $version ($id) to Mozilla for signing. This usually takes a few minutes..."
    & $node $webext sign --channel unlisted --source-dir extension --artifacts-dir $out --timeout 900000
    if ($LASTEXITCODE) { throw "Signing failed (see above)." }
} finally {
    foreach ($v in $cleanup) { Remove-Item "Env:$v" -ErrorAction SilentlyContinue }
    # web-ext leaves this note in the source folder; it must not end up in packages
    Remove-Item extension\.amo-upload-uuid -ErrorAction SilentlyContinue
}

$signed = Get-ChildItem $out -Filter *.xpi | Select-Object -First 1
if (-not $signed) { throw "Mozilla didn't return a signed file." }
$xpi = "dist\Fetchbox-$version-firefox.xpi"
Move-Item $signed.FullName $xpi -Force
Remove-Item $out -Recurse -Force
$hash = (Get-FileHash $xpi -Algorithm SHA256).Hash.ToLower()

# installed copies check this file and update themselves
New-Item -ItemType Directory -Force firefox | Out-Null
$updates = [ordered]@{ addons = [ordered]@{ $id = [ordered]@{ updates = @(
    [ordered]@{
        version = $version
        update_link = "https://github.com/$repo/releases/download/v$version/Fetchbox-$version-firefox.xpi"
        update_hash = "sha256:$hash"
    }
) } } }
# plain UTF-8 without a byte-order mark (Windows PowerShell's "utf8" adds one, which JSON readers may reject)
[IO.File]::WriteAllText((Join-Path (Get-Location) "firefox\updates.json"), ($updates | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding $false))

Write-Host ""
Write-Host "Signed: $xpi"
Write-Host "sha256 $hash"
Write-Host "Updated firefox\updates.json - commit it once the release is published."
