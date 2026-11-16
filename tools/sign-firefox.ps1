# The Firefox add-on, signed by Mozilla (channel "unlisted": signed, but not shown in the add-ons store).
# package.ps1 runs this after packaging; it only goes to Mozilla when the add-on has changed.
#
#   Add-on unchanged since it was last signed:  reuses firefox\fetchbox.xpi (no network, no questions).
#   Add-on changed:  raises its version in extension\manifest.json if needed, signs it, and updates
#                    firefox\fetchbox.xpi, firefox\signed.json and firefox\updates.json (commit them).
#   Either way:      dist\Fetchbox-<app version>-firefox.xpi, to attach to the GitHub release.
#
# The add-on has its own version (extension\manifest.json), separate from the app's (Cargo.toml).
#
# Credentials: your addons.mozilla.org API key (https://addons.mozilla.org/developers/addon/api/key/).
# Asked for once, checked with Mozilla, then saved encrypted for your Windows account (DPAPI) in
# %LOCALAPPDATA%\Fetchbox-dev\amo-credentials.xml. WEB_EXT_API_KEY / WEB_EXT_API_SECRET override it.
#
#   tools\sign-firefox.ps1                    sign if needed (what package.ps1 runs)
#   tools\sign-firefox.ps1 -Check             only say whether the add-on needs signing
#   tools\sign-firefox.ps1 -SaveCredentials   enter (or replace) the saved credentials
#   tools\sign-firefox.ps1 -Forget            delete the saved credentials
param([switch]$Check, [switch]$SaveCredentials, [switch]$Forget)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot)
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$repo = "kebabified/fetchbox"
$credFile = Join-Path $env:LOCALAPPDATA "Fetchbox-dev\amo-credentials.xml"
$stateFile = "firefox\signed.json"
$signedXpi = "firefox\fetchbox.xpi"
$appVersion = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value

function Write-Utf8([string]$path, [string]$text) {
    # no byte-order mark: Windows PowerShell's "utf8" adds one, which JSON readers may reject
    [IO.File]::WriteAllText((Join-Path (Get-Location) $path), $text, (New-Object Text.UTF8Encoding $false))
}

function Get-Manifest { Get-Content extension\manifest.json -Raw | ConvertFrom-Json }

# A fingerprint of the add-on's files. Line endings are ignored, so a fresh git checkout matches.
function Get-SourceHash {
    $root = (Resolve-Path extension).Path
    $sha = [Security.Cryptography.SHA256]::Create()
    $lines = Get-ChildItem extension -Recurse -File | Where-Object Name -ne ".amo-upload-uuid" | ForEach-Object {
        $rel = $_.FullName.Substring($root.Length + 1).Replace('\', '/')
        $bytes = [IO.File]::ReadAllBytes($_.FullName)
        if ($_.Extension -in ".js", ".json", ".html", ".css", ".md", ".txt") { $bytes = [byte[]]($bytes | Where-Object { $_ -ne 13 }) }
        "$rel " + (-join ($sha.ComputeHash($bytes) | ForEach-Object { $_.ToString("x2") }))
    } | Sort-Object { $_ } -CaseSensitive
    -join ($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes(($lines -join "`n"))) | ForEach-Object { $_.ToString("x2") })
}

# ── credentials ──
function ConvertTo-B64Url([byte[]]$b) { [Convert]::ToBase64String($b).TrimEnd('=').Replace('+', '-').Replace('/', '_') }

# Asks Mozilla who these credentials belong to: true if they work.
function Test-AmoCredentials([string]$issuer, [string]$secret) {
    # UTC seconds ("Get-Date -UFormat %s" is local time in Windows PowerShell, which Mozilla rejects)
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $header = ConvertTo-B64Url ([Text.Encoding]::UTF8.GetBytes('{"alg":"HS256","typ":"JWT"}'))
    $payload = ConvertTo-B64Url ([Text.Encoding]::UTF8.GetBytes((@{ iss = $issuer; jti = [guid]::NewGuid().ToString(); iat = $now; exp = $now + 60 } | ConvertTo-Json -Compress)))
    $hmac = New-Object Security.Cryptography.HMACSHA256 (, [Text.Encoding]::UTF8.GetBytes($secret))
    $token = "$header.$payload." + (ConvertTo-B64Url $hmac.ComputeHash([Text.Encoding]::UTF8.GetBytes("$header.$payload")))
    try {
        $r = Invoke-WebRequest "https://addons.mozilla.org/api/v5/accounts/profile/" -Headers @{ Authorization = "JWT $token" } -UseBasicParsing
        return $r.StatusCode -eq 200
    } catch { return $false }
}

function Read-AmoCredentials {
    Write-Host "Your addons.mozilla.org API credentials (https://addons.mozilla.org/developers/addon/api/key/):"
    while ($true) {
        do {
            $issuer = (Read-Host "JWT issuer (the short one, like user:12345:67)").Trim()
            if ($issuer -notmatch '^user:\d+:\d+$') { Write-Host "That isn't the issuer. It starts with 'user:'. Try again." -ForegroundColor Yellow }
        } until ($issuer -match '^user:\d+:\d+$')
        $secure = Read-Host "JWT secret (the long one; hidden as you type)" -AsSecureString
        $cred = New-Object Management.Automation.PSCredential ($issuer, $secure)
        if (Test-AmoCredentials $issuer $cred.GetNetworkCredential().Password.Trim()) { break }
        Write-Host "Mozilla didn't accept those. Check both values (or regenerate them) and try again." -ForegroundColor Yellow
    }
    New-Item -ItemType Directory -Force (Split-Path $credFile) | Out-Null
    $cred | Export-Clixml $credFile   # the secret is encrypted for this Windows account
    Write-Host "Saved for next time (encrypted for your Windows account): $credFile" -ForegroundColor Green
    $cred
}

function Get-AmoCredentials {
    if ($env:WEB_EXT_API_KEY -and $env:WEB_EXT_API_SECRET) {
        return New-Object Management.Automation.PSCredential ($env:WEB_EXT_API_KEY, (ConvertTo-SecureString $env:WEB_EXT_API_SECRET -AsPlainText -Force))
    }
    if (Test-Path $credFile) {
        try { return Import-Clixml $credFile } catch { Write-Host "The saved credentials can't be read on this account; enter them again." -ForegroundColor Yellow }
    }
    Read-AmoCredentials
}

if ($Forget) {
    Remove-Item $credFile -ErrorAction SilentlyContinue
    Write-Host "Saved credentials deleted."
    exit 0
}
if ($SaveCredentials) {
    Read-AmoCredentials | Out-Null
    exit 0
}

# ── does the add-on need signing? ──
$state = if (Test-Path $stateFile) { Get-Content $stateFile -Raw | ConvertFrom-Json } else { $null }
$source = Get-SourceHash
$unchanged = $state -and $state.source_sha256 -eq $source -and (Test-Path $signedXpi)
$out = "dist\Fetchbox-$appVersion-firefox.xpi"

if ($Check) {
    if ($unchanged) { Write-Host "Unchanged since add-on $($state.version) was signed: no signing needed." }
    else { Write-Host "The add-on has changed since it was last signed: it will be signed (add-on $((Get-Manifest).version))." }
    Write-Host ("Saved credentials: " + $(if (Test-Path $credFile) { "yes" } else { "no (you'll be asked once)" }))
    exit 0
}

New-Item -ItemType Directory -Force dist | Out-Null
if ($unchanged) {
    Copy-Item $signedXpi $out -Force
    Write-Host "Firefox add-on unchanged since $($state.version): reusing the signed copy -> $out"
    exit 0
}

# Mozilla signs each version once: a changed add-on needs a new version number
$manifest = Get-Manifest
if ($state -and $manifest.version -eq $state.version) {
    $parts = $state.version.Split('.')
    $parts[-1] = [string]([int]$parts[-1] + 1)
    $next = $parts -join '.'
    $raw = Get-Content extension\manifest.json -Raw
    Write-Utf8 extension\manifest.json ([regex]::new('"version":\s*"[^"]+"').Replace($raw, "`"version`": `"$next`"", 1))
    Write-Host "The add-on changed, so its version goes from $($state.version) to $next."
    $manifest = Get-Manifest
    $source = Get-SourceHash
}
$version = $manifest.version
$id = $manifest.browser_specific_settings.gecko.id

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

$cred = Get-AmoCredentials
$tmp = "dist\firefox-signed"
if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }
try {
    $env:WEB_EXT_API_KEY = $cred.UserName
    $env:WEB_EXT_API_SECRET = $cred.GetNetworkCredential().Password.Trim()
    Write-Host "Sending add-on $version ($id) to Mozilla for signing. This usually takes a few minutes..."
    & $node $webext sign --channel unlisted --source-dir extension --artifacts-dir $tmp --timeout 900000
    if ($LASTEXITCODE) { throw "Signing failed (see above)." }
} finally {
    Remove-Item Env:WEB_EXT_API_KEY, Env:WEB_EXT_API_SECRET -ErrorAction SilentlyContinue
    # web-ext leaves this note in the source folder; it must not end up in packages
    Remove-Item extension\.amo-upload-uuid -ErrorAction SilentlyContinue
}

$signed = Get-ChildItem $tmp -Filter *.xpi | Select-Object -First 1
if (-not $signed) { throw "Mozilla didn't return a signed file." }
Move-Item $signed.FullName $signedXpi -Force
Remove-Item $tmp -Recurse -Force
Copy-Item $signedXpi $out -Force
$hash = (Get-FileHash $signedXpi -Algorithm SHA256).Hash.ToLower()

# what was signed, so unchanged releases can reuse it
Write-Utf8 $stateFile ([ordered]@{ version = $version; source_sha256 = $source; xpi_sha256 = $hash; release = "v$appVersion" } | ConvertTo-Json)

# installed copies read this from GitHub and update themselves; the link is this release's copy
$link = "https://github.com/$repo/releases/download/v$appVersion/Fetchbox-$appVersion-firefox.xpi"
Write-Utf8 firefox\updates.json (([ordered]@{ addons = [ordered]@{ $id = [ordered]@{ updates = @(
    [ordered]@{ version = $version; update_link = $link; update_hash = "sha256:$hash" }
) } } }) | ConvertTo-Json -Depth 6)

Write-Host ""
Write-Host "Signed add-on $version -> $out"
Write-Host "Commit firefox\ and extension\manifest.json, and publish release v$appVersion with $out attached."
