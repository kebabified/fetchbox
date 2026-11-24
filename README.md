# Fetchbox

Save videos and audio from YouTube, TikTok, Instagram, X, Reddit and 1,700+ other sites on Windows.
Paste a link into the app, or use the browser extension to get a download button on posts.

Only download what you have the right to keep.

## Install

Download from the [latest release](https://github.com/kebabified/fetchbox/releases/latest). You need Windows 10 or 11.

- **Installer** (`Fetchbox-…-setup.exe`): run it. No admin rights needed.
- **Portable** (`Fetchbox-…-portable.zip`): unzip it anywhere and open **fetchbox.exe**.

If Windows says "Windows protected your PC", click **More info**, then **Run anyway**.
The first download takes a moment while Fetchbox fetches its tools.

## Download something

1. Copy a link and press **Ctrl+V** in Fetchbox.
2. Choose **Video**, **Audio** or **Video, no sound**.
3. Press **Enter**.

Files go to your Downloads folder. Click **Change** at the bottom to pick another one.

## Browser extension (optional)

Puts a download button on videos and posts. It works even when the Fetchbox window is closed.

**Chrome, Edge, Brave and similar**

1. If you used the portable version, open Fetchbox → **Settings** → **Connect browsers** first.
2. Go to `chrome://extensions` (or `edge://extensions`) and turn on **Developer mode**.
3. Click **Load unpacked** and choose the `extension` folder inside the Fetchbox folder.
4. Restart the browser.

**Firefox**

Download `Fetchbox-…-firefox.xpi` from the [latest release](https://github.com/kebabified/fetchbox/releases/latest),
drag it into Firefox and click **Add**.

**A video needs you signed in?** Click the Fetchbox button in the browser toolbar and turn on
**use my login for this site**.

## Problems

| What you see | What to do |
| --- | --- |
| The extension says "not connected" | Fetchbox → Settings → **Connect browsers**, then restart the browser. |
| No buttons on a site | Refresh the page, or right-click the video and use the Fetchbox menu. |
| A download fails | Click **Retry**. Still failing? Settings → **Update yt-dlp**. |

## Remove

Remove the extension from your browser, then uninstall Fetchbox from Windows Settings → Apps
(or just delete the folder if you used the portable version).
