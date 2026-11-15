const $ = (id) => document.getElementById(id);
const send = (msg) => chrome.runtime.sendMessage(msg);
const DEFAULTS = { postButtons: true, hoverButtons: true, sendCookies: false };
let tabUrl = "";

function setStatus(el, text, cls = "") { el.textContent = text; el.className = `${el.id === "result" ? "result" : "status"} ${cls}`; }

async function init() {
  // settings
  const s = await chrome.storage.sync.get(DEFAULTS);
  for (const k of ["postButtons", "hoverButtons"]) {
    $(k).checked = !!s[k];
    $(k).addEventListener("change", () => chrome.storage.sync.set({ [k]: $(k).checked }));
  }
  // "use my login": the browser asks for the cookies permission only when this is switched on
  const perm = { permissions: ["cookies"] };
  $("sendCookies").checked = !!s.sendCookies && await chrome.permissions.contains(perm);
  $("sendCookies").addEventListener("change", async () => {
    let on = $("sendCookies").checked;
    if (on) on = await chrome.permissions.request(perm).catch(() => false);
    else await chrome.permissions.remove(perm).catch(() => {});
    $("sendCookies").checked = on;
    chrome.storage.sync.set({ sendCookies: on });
  });

  // connection
  send({ type: "ping" }).then((r) => {
    const c = $("conn");
    if (r?.ok) { c.className = "conn ok"; c.textContent = "connected"; c.title = "Downloads run in the background — the Fetchbox window doesn't need to be open"; }
    else { c.className = "conn bad"; c.textContent = "not connected"; c.title = r?.error || ""; setStatus($("result"), r?.error || "Can't reach Fetchbox", "bad"); }
  });

  // current page
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  tabUrl = tab?.url || "";
  let host = "";
  try { host = new URL(tabUrl).hostname.replace(/^www\./, ""); } catch {}
  $("pageHost").textContent = tab?.title || host || "this page";
  $("pageHost").title = tabUrl;
  if (!/^https?:/.test(tabUrl)) {
    setStatus($("pageStatus"), "not a web page", "bad");
    document.querySelectorAll(".actions button").forEach((b) => (b.disabled = true));
    return;
  }
  const r = await send({ type: "check", urls: [tabUrl] });
  const v = r?.results?.[tabUrl];
  if (!r?.ok || v?.error) setStatus($("pageStatus"), "couldn't check this page — Fetchbox isn't connected", "bad");
  else if (v?.supported && v.collection) {
    setStatus($("pageStatus"), `${host} · ${v.extractor} (a list of videos)`, "ok");
    $("plRow").classList.remove("hidden");
  } else if (v?.supported) setStatus($("pageStatus"), `${host} · supported (${v.extractor})`, "ok");
  else setStatus($("pageStatus"), `${host} isn't a downloadable page — use the buttons on posts, or right-click a video/link`);
}

document.querySelectorAll(".actions button").forEach((b) =>
  b.addEventListener("click", async () => {
    setStatus($("result"), "sending…");
    const r = await send({ type: "download", url: tabUrl, mode: b.dataset.mode, playlist: $("playlist").checked });
    setStatus($("result"), r?.ok ? "✓ downloading in the background" : `✕ ${r?.error || "failed"}`, r?.ok ? "ok" : "bad");
    if (r?.ok) refreshDownloads();
  }));

// ── background downloads list ──
function renderDownloads(jobs) {
  const list = $("dlList");
  $("dlSection").classList.toggle("hidden", !jobs.length);
  list.replaceChildren(...jobs.slice(0, 6).map((j) => {
    const active = j.state === "queued" || j.state === "running";
    const row = document.createElement("div");
    row.className = `dl ${j.state}`;
    const t = Object.assign(document.createElement("div"), { className: "t", textContent: j.file || j.title, title: j.url });
    // finished rows already show the file name as the title: keep just "✓ size · via engine"
    const detail = j.state === "done" ? `✓ ${(j.text || "").split("·").slice(1).join("·").trim()}` : j.text;
    const s = Object.assign(document.createElement("div"), { className: "s", textContent: detail, title: j.text });
    if (j.state === "cancelled") row.className = "dl";
    const b = Object.assign(document.createElement("button"), {
      className: "b", textContent: active ? "✕" : j.state === "done" ? "📂" : "↻",
      title: active ? "Cancel" : j.state === "done" ? "Show in folder" : "Try again",
    });
    if (active) b.onclick = () => send({ type: "cancel", job: j.id });
    else if (j.state === "done") b.onclick = () => send({ type: "reveal", job: j.id });
    else b.onclick = () => send({ type: "retry", job: j.id }).then(refreshDownloads);
    const bar = Object.assign(document.createElement("div"), { className: "bar" + (active && j.progress == null ? " ind" : "") });
    const fill = document.createElement("i");
    if (j.progress != null) fill.style.width = `${Math.round(j.progress * 100)}%`;
    bar.append(fill);
    row.append(t, b, s);
    if (active) row.append(bar);
    return row;
  }));
}
async function refreshDownloads() {
  const r = await send({ type: "status" });
  if (r?.ok) renderDownloads(r.jobs || []);
}
refreshDownloads();
setInterval(refreshDownloads, 1000);

$("open").addEventListener("click", async () => {
  const r = await send({ type: "open" });
  if (!r?.ok) setStatus($("result"), `✕ ${r?.error || "couldn't open Fetchbox"}`, "bad");
});

init();
