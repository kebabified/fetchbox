// Fetchbox extension — background service worker.
// Owns the connection to the Fetchbox helper (native messaging), the right-click menu,
// the toolbar badge, and answers questions from the page buttons.

const HOST = "com.fetchbox.host";
const DEFAULTS = { postButtons: true, hoverButtons: true, sendCookies: false };

let port = null;
let lastError = "";
let nextId = 1;
const pending = new Map();      // id -> {resolve, timer}
const cache = new Map();        // url -> {supported, extractor, collection}

// ── helper connection ──
function connect() {
  if (port) return port;
  try {
    port = chrome.runtime.connectNative(HOST);
  } catch (e) {
    lastError = String(e?.message || e);
    port = null;
    return null;
  }
  lastError = "";
  port.onMessage.addListener((msg) => {
    const p = pending.get(msg.id);
    if (!p) return;
    pending.delete(msg.id);
    clearTimeout(p.timer);
    p.resolve(msg);
  });
  port.onDisconnect.addListener(() => {
    lastError = chrome.runtime.lastError?.message || "Fetchbox helper disconnected";
    port = null;
    for (const [, p] of pending) { clearTimeout(p.timer); p.resolve({ ok: false, error: lastError }); }
    pending.clear();
  });
  return port;
}

function ask(msg, timeoutMs = 30000) {
  return new Promise((resolve) => {
    const p = connect();
    if (!p) return resolve({ ok: false, error: friendly(lastError) });
    const id = nextId++;
    const timer = setTimeout(() => { pending.delete(id); resolve({ ok: false, error: "Fetchbox didn't answer" }); }, timeoutMs);
    pending.set(id, { resolve, timer });
    try { p.postMessage({ ...msg, id }); }
    catch (e) { pending.delete(id); clearTimeout(timer); resolve({ ok: false, error: friendly(String(e)) }); }
  });
}

function friendly(err) {
  if (/not found|not registered|Specified native messaging host/i.test(err))
    return "Fetchbox isn't connected to this browser — open Fetchbox, go to Settings and click \"Connect browsers\", then restart the browser.";
  if (/forbidden/i.test(err))
    return "This copy of the extension isn't allowed to use Fetchbox — open Fetchbox → Settings → Connect browsers.";
  return err || "Can't reach Fetchbox";
}

const clean = (u) => { try { const x = new URL(u); x.hash = ""; return x.href; } catch { return u; } };

async function check(urls) {
  const out = {}, need = [];
  for (const raw of urls) {
    const u = clean(raw);
    if (cache.has(u)) out[raw] = cache.get(u); else need.push(u);
  }
  if (need.length) {
    const r = await ask({ type: "check", urls: [...new Set(need)] });
    const res = r.ok ? r.results : {};
    for (const u of need) {
      const v = res[u] || { supported: false, error: !r.ok };
      if (r.ok) cache.set(u, v);
      if (cache.size > 3000) cache.clear();
    }
    for (const raw of urls) out[raw] = cache.get(clean(raw)) || { supported: false };
  }
  return out;
}

async function settings() {
  return { ...DEFAULTS, ...(await chrome.storage.sync.get(DEFAULTS)) };
}

async function download(url, mode, playlist = false) {
  if (!/^https?:\/\//i.test(url || "")) return { ok: false, error: "not a web link" };
  if (!["auto", "audio", "mute"].includes(mode)) mode = "auto";
  const s = await settings();
  let cookies = null;
  // Only with the user's opt-in AND the browser permission they granted for it; only this one site's cookies.
  if (s.sendCookies && chrome.cookies && await chrome.permissions.contains({ permissions: ["cookies"] })) {
    try { cookies = await chrome.cookies.getAll({ url }); } catch { cookies = null; }
  }
  const r = await ask({ type: "download", url: clean(url), mode, cookies, playlist: !!playlist }, 45000);
  if (r.ok && r.job != null) { known.set(r.job, "queued"); pollDownloads(); }
  return r;
}

// ── background downloads: follow progress, notify when each one finishes ──
const known = new Map(); // job id -> last seen state
let polling = false;
async function pollDownloads() {
  if (polling) return;
  polling = true;
  try {
    for (;;) {
      const r = await ask({ type: "status" }, 10000);
      const jobs = r.ok ? r.jobs || [] : [];
      let active = 0;
      for (const j of jobs) {
        if (j.state === "queued" || j.state === "running") active++;
        const prev = known.get(j.id);
        if (prev && prev !== j.state && (j.state === "done" || j.state === "failed")) notify(j);
        known.set(j.id, j.state);
      }
      chrome.action.setTitle({ title: active ? `Fetchbox — ${active} downloading` : "Fetchbox" });
      if (!active || !r.ok) break;
      await new Promise((res) => setTimeout(res, 1500));
    }
  } finally {
    polling = false;
  }
}

function notify(j) {
  const ok = j.state === "done";
  chrome.notifications.create(`fb-${j.id}`, {
    type: "basic",
    iconUrl: "icons/128.png",
    title: ok ? "Downloaded" : "Download failed",
    message: ok ? (j.file || j.title) : (j.text || "failed"),
    contextMessage: ok ? (j.text || "").split("·").slice(1).join("·").trim() : j.title,
    buttons: ok ? [{ title: "Show in folder" }] : [],
    priority: 0,
  });
}
const revealFrom = (nid) => nid.startsWith("fb-") && ask({ type: "reveal", job: Number(nid.slice(3)) });
chrome.notifications.onButtonClicked.addListener((nid) => { revealFrom(nid); chrome.notifications.clear(nid); });
chrome.notifications.onClicked.addListener((nid) => { revealFrom(nid); chrome.notifications.clear(nid); });

// ── messages from page buttons and the popup ──
chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  if (sender.id !== chrome.runtime.id || !msg || typeof msg.type !== "string") return; // our own pages and scripts only
  (async () => {
    switch (msg.type) {
      case "check": return reply({ ok: true, results: await check(msg.urls || []) });
      case "download": return reply(await download(msg.url, msg.mode, !!msg.playlist));
      case "settings": return reply(await settings());
      case "ping": return reply(await ask({ type: "ping" }, 8000));
      case "open": return reply(await ask({ type: "open" }));
      case "status": return reply(await ask({ type: "status" }, 8000));
      case "cancel": return reply(await ask({ type: "cancel", job: msg.job }));
      case "reveal": return reply(await ask({ type: "reveal", job: msg.job }));
      case "retry": return reply(await ask({ type: "retry", job: msg.job }));
      default: return reply({ ok: false, error: "unknown" });
    }
  })();
  return true; // async reply
});

// ── right-click menu ──
const MENU = [
  ["fb-auto", "Download with Fetchbox", "auto"],
  ["fb-audio", "Download audio only", "audio"],
  ["fb-mute", "Download video without sound", "mute"],
];
chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({ id: "fb", title: "Fetchbox", contexts: ["page", "link", "video", "audio", "image", "frame"] });
    for (const [id, title] of MENU)
      chrome.contextMenus.create({ id, parentId: "fb", title, contexts: ["page", "link", "video", "audio", "image", "frame"] });
  });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const item = MENU.find(([id]) => id === info.menuItemId);
  if (!item) return;
  const src = info.srcUrl && /^https?:/.test(info.srcUrl) ? info.srcUrl : null;
  // Prefer the link; for a video/image use the post or page it sits on (players use blob: URLs).
  const candidates = [info.linkUrl, info.frameUrl, info.pageUrl, src].filter(Boolean);
  const results = await check(candidates);
  const url = candidates.find((u) => results[u]?.supported && !results[u]?.collection)
           || candidates.find((u) => results[u]?.supported) || candidates[0];
  const r = await download(url, item[2]);
  if (tab?.id != null) {
    chrome.tabs.sendMessage(tab.id, { type: "toast", ok: r.ok, text: r.ok ? "Sent to Fetchbox" : r.error }, { frameId: 0 })
      .catch(() => {});
  }
});

// ── toolbar badge: show ↓ when the current page itself is downloadable ──
async function badge(tabId, url) {
  if (!url || !/^https?:/.test(url)) return chrome.action.setBadgeText({ tabId, text: "" });
  const r = (await check([url]))[url];
  const text = r?.supported ? (r.collection ? "≡" : "↓") : "";
  chrome.action.setBadgeBackgroundColor({ tabId, color: "#3fb950" });
  chrome.action.setBadgeText({ tabId, text });
}
chrome.tabs.onUpdated.addListener((tabId, change, tab) => {
  if (change.url || change.status === "complete") badge(tabId, tab.url).catch(() => {});
});
chrome.tabs.onActivated.addListener(({ tabId }) => {
  chrome.tabs.get(tabId).then((t) => badge(tabId, t.url)).catch(() => {});
});
