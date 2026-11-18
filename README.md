# Fetchbox

A Windows media downloader that drives **yt-dlp** and **cobalt**, with a browser extension that puts a
download button on posts and videos.

- **Window** (`fetchbox.exe`): paste links, pick Video / Audio / Video without sound, watch the queue.
- **Browser extension** (`extension/`): buttons on YouTube, TikTok, Instagram, X and Reddit posts, a hover
  button on any supported video, a right-click menu and a toolbar popup. Works with the window closed.
  One folder serves Chromium-based browsers and Firefox 128+.
- **Updates**: the window checks GitHub releases (`REPO` in `src/update.rs`); an installed copy downloads
  the new installer, checks it against GitHub's published SHA-256 and runs it with `/SILENT /RELAUNCH`.
  The unpacked extension notices its files on disk changed version, reloads itself (at most once per
  version) and re-injects its page script into open tabs.
- **One queue**: the window and the extension talk to the same background service, so each shows the
  other's downloads.

Only download what you have the right to keep. New here? Read [GUIDE.md](GUIDE.md).

## How it routes a link

| Link | Engine |
| --- | --- |
| YouTube (any youtube.com / youtu.be link) | yt-dlp only, always |
| Everything else, engine = Automatic | yt-dlp first, cobalt if that fails |
| Engine = yt-dlp or cobalt | just that one |

With the container set to MP4, yt-dlp must produce an MP4; if it can't, that counts as a failure and the
link goes to cobalt.

## Using it

1. Run `fetchbox.exe`. On first use it downloads yt-dlp (and ffmpeg, if the PC has none).
2. For the browser: **Settings → Connect browsers**, then load `extension/` as an unpacked extension
   (extensions page → Developer mode → Load unpacked) and restart the browser.

cobalt runs locally by default: the first time a download needs it, Fetchbox sets up a private cobalt server
(Node.js + cobalt, about 150 MB) that only this PC can reach. You can point Fetchbox at another cobalt
instance in Settings instead.

## Layout

```
fetchbox.exe         the window (client of the service)
fetchbox-host.exe    background service · browser helper (native messaging) · installer
extension/           the browser extension for Chromium and Firefox (plain JavaScript, no build step)
src/                 Rust source      assets/extractors.json   yt-dlp's site URL patterns
tools/               dump_extractors.py (refresh the patterns), make_icon.py
runtime/             downloaded tools: yt-dlp, ffmpeg, node, cobalt     logs/   log files
```

`fetchbox-host.exe` commands: `service`, `install`, `uninstall`, `check <url>`, `download <url> [mode]`,
`status`, `tools`.

## Building

No admin rights needed:

```
rustup-init -y --default-host x86_64-pc-windows-gnu      (from https://rustup.rs)
winget install BrechtSanders.WinLibs.POSIX.UCRT
powershell -ExecutionPolicy Bypass -File build.ps1 -Test
```

## Packaging

```
winget install JRSoftware.InnoSetup --scope user
powershell -ExecutionPolicy Bypass -File package.ps1
```

Writes into `dist\`:

- `Fetchbox-<version>-setup.exe`: per-user installer (`installer/fetchbox.iss`)
- `Fetchbox-<version>-portable.zip`: a `portable.txt` next to the programs keeps settings in the folder
- `Fetchbox-<version>-firefox.xpi`: the Firefox add-on, signed by Mozilla

Attach all three to the GitHub release `v<version>`.

### Versions and the Firefox add-on

The app's version is in `Cargo.toml`. The browser extension has its own, in `extension/manifest.json`,
because Mozilla signs each add-on version once. `package.ps1` runs `tools\sign-firefox.ps1`, which
fingerprints `extension/` and compares it with `firefox/signed.json`:

- **unchanged**: reuses `firefox/fetchbox.xpi`. No network, no questions.
- **changed**: raises the add-on's version if needed, has Mozilla sign it (channel "unlisted", not shown
  in the store), and updates `firefox/fetchbox.xpi`, `firefox/signed.json` and `firefox/updates.json`.
  Commit those and `extension/manifest.json`, and publish the release, since `updates.json` links to
  this release's `.xpi`. Installed add-ons read `updates.json` from `main` and update themselves.

Signing uses your addons.mozilla.org API key, asked for the first time, checked with Mozilla, and saved
encrypted for your Windows account in `%LOCALAPPDATA%\Fetchbox-dev\amo-credentials.xml`.
`tools\sign-firefox.ps1 -Check` says whether signing is needed; `-SaveCredentials` and `-Forget`
replace or delete the saved key.

## Security and privacy

- **No network port, no telemetry.** The window, the browser helper and the service talk over a Windows
  named pipe that only your Windows account (and SYSTEM) can open, and that refuses remote clients.
  The local cobalt server listens on 127.0.0.1 only.
- **Only the Fetchbox extension can start the helper.** The browser enforces the allowed extension ID from
  the helper's manifest, and the helper checks it again (Chrome passes the extension's origin, Firefox
  its ID `fetchbox@fetchbox`). Add more IDs (a store build, a fork) one per line in
  `native/allowed_extensions.txt`, then reconnect.
- **Links are data, never commands.** Only `http(s)` links are accepted; yt-dlp receives them after `--`
  and runs with `--ignore-config --no-plugin-dirs`, so nothing on the page or the PC can add options.
  Message sizes, queue length and cookie counts are capped.
- **Downloads are verified.** yt-dlp, ffmpeg and Node.js are fetched over HTTPS from their official release
  pages and checked against the published SHA-256 before being unpacked or run. Archives can't write
  outside their folder. cobalt's source comes from its GitHub repository over HTTPS (it publishes no
  checksums) and its dependencies install from its lockfile. A Fetchbox update is only run if it was
  downloaded from github.com and matches the SHA-256 GitHub publishes for that release file.
- **Files stay in your download folder.** File names from servers are sanitised and can't escape it.
- **Cookies are opt-in.** The extension asks for the browser's cookies permission only when you turn on
  "use my login for this site", sends only the cookies for the site being downloaded, and Fetchbox deletes
  the temporary cookie file when that download ends.
- **Secrets at rest.** A cobalt API key is stored encrypted with Windows DPAPI (your account only), and is
  only sent over HTTPS (plain HTTP is allowed for localhost).
- The extension runs on all sites (needed for the hover button) but only reads post links and video
  positions; it makes no network requests of its own.

### Before a public release

- Fetchbox is MIT-licensed (see LICENSE). The tools it downloads keep their own licences: yt-dlp is
  Unlicense, cobalt is AGPL-3.0 (run as a separate program, not linked), ffmpeg builds are GPL.
- Sign the two programs (unsigned downloads trigger SmartScreen warnings).
- Publish the extension to a store, or keep the unpacked build; a store build gets a new ID — add it to
  `EXTENSION_ID` in `src/install.rs`. Keep `native/extension_key.pem` private.
- Firefox only keeps signed add-ons: the extension is signed "unlisted" on addons.mozilla.org (not shown in
  the store) by `tools/sign-firefox.ps1`, and updates itself through `firefox/updates.json`.
- Site patterns in `assets/extractors.json` are a snapshot; refresh with `tools/dump_extractors.py` when
  yt-dlp adds sites.
