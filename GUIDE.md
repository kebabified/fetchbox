# Fetchbox — user guide

Fetchbox saves videos and audio from YouTube, TikTok, Instagram, X, Reddit, SoundCloud, Twitch, Vimeo
and 1,700+ other sites. Use the window, or the browser extension that adds a download button to posts.

Only download what you have the right to keep.

## Install

Windows 10 or 11. Pick one:

**Installer** (`Fetchbox-…-setup.exe`): run it and click through. It installs for your account only
(no admin prompt), adds Fetchbox to the Start menu, connects your browsers and adds an uninstaller.

**Portable** (`Fetchbox-…-portable.zip`): unzip it anywhere, including a USB stick, and double-click
**fetchbox.exe**. Everything stays inside that folder: settings, downloaded tools and logs.

Either way:
- Windows may show "Windows protected your PC" because the program isn't signed yet.
  Click **More info**, then **Run anyway**.
- The first download fetches yt-dlp, and ffmpeg if your PC doesn't have it. This takes a moment.

That's all you need for the window. The browser extension is optional and takes about a minute more.

### Add the browser extension (optional)

Works in Chrome, Edge, Brave, Vivaldi, Helium and other Chromium-based browsers, and in Firefox 128 or newer.
Browsers don't let installers add extensions, so this part is by hand.

First, in Fetchbox, open **Settings**. Portable copy: click **Connect browsers** (the installer already did this).

**Chrome, Edge, Brave, Helium and the like**

1. Open the extensions page (`chrome://extensions`, or `edge://extensions` in Edge).
2. Turn on **Developer mode** (top right).
3. Click **Load unpacked** and choose the `extension` folder inside the Fetchbox folder.
   (Settings shows the exact path, with a Copy button.)
4. Restart the browser, then pin Fetchbox from the puzzle-piece menu.

**Firefox**

1. Open `about:debugging`, then **This Firefox**.
2. Click **Load Temporary Add-on…** and choose `manifest.json` inside the `extension` folder.
3. Pin Fetchbox from the puzzle-piece menu.

Firefox removes add-ons loaded this way when it closes, so repeat these steps after restarting it.
That goes away once a signed version of the extension is published.

If you move the Fetchbox folder later, click **Connect browsers** again and reload the extension.

## Use the window

1. Copy a link and press **Ctrl+V** in Fetchbox (or click **Paste**). You can paste several links at once.
2. Choose what to save:
   - **Video**: pick the quality and file type (MP4 by default).
   - **Audio**: pick the format (MP3 by default) and bitrate.
   - **Video, no sound**
3. Press **Enter** or click **Download**.

Each download shows its progress. When it finishes, **Show in folder** opens it in Explorer.
**Retry** runs a failed download again. **Cancel** stops one in progress.

Files go to your Downloads folder. Click **Change** at the bottom to pick another folder.

**More options** has the rest: whole playlists, file naming, adding the title and cover art to the file,
and using your browser's login for videos that need you signed in.

## Use the browser extension

- **On a post or thumbnail**: click the round download button in its corner, then choose
  **Video**, **Audio only** or **Video, no sound**.
- **On any other video**: hover over it and the same button appears, if the page can be downloaded.
- **Right-click** a link, video or page and choose **Fetchbox**.
- **Toolbar button**: downloads the current page, and lists your downloads with progress.

The Fetchbox window doesn't need to be open. Downloads run in the background and you get a notification
when each one finishes. Open the window any time to see the same list.

**Private or members-only posts**: in the extension's popup, turn on **use my login for this site**.
Your browser asks for permission once. Fetchbox then uses that site's login for the download only.

## Good to know

- **YouTube** always downloads with yt-dlp. For other sites, Fetchbox tries yt-dlp first and uses cobalt
  if that fails.
- **cobalt** runs on your own PC. The first time it's needed, Fetchbox sets it up by itself
  (about 150 MB, one time). You can use a different cobalt server in Settings.
- **Something stopped working on one site?** Sites change often. Open **Settings** and click
  **Update yt-dlp**, and **Update cobalt** if you use it.
- **Updates**: when the window opens, Fetchbox asks GitHub whether there's a newer version and shows
  a bar if there is. **Update** downloads the installer, checks it and installs it, then reopens Fetchbox.
  A portable copy opens the download page instead. Turn this off, or check by hand, under Settings → About.
- **Privacy**: nothing is sent anywhere except to the site you're downloading from, and to GitHub for the
  update check. There is no account and no tracking.

## Problems

| What you see | What to do |
| --- | --- |
| The extension says "not connected" | Open Fetchbox → Settings → **Connect browsers**, then restart the browser. |
| No buttons on a site | Refresh the tab. If it's still missing, right-click the video or link and use the Fetchbox menu. |
| A download fails | Click **Retry**. If it keeps failing, update yt-dlp in Settings. |
| Windows blocks the program | Click **More info**, then **Run anyway**. |
| You want to see what happened | Settings → **Open logs folder**. |

## Remove Fetchbox

1. Remove the extension from your browser's extensions page.
2. Installed copy: uninstall Fetchbox from Windows Settings → Apps. It asks whether to remove your settings too.
   Portable copy: delete the folder.
