// Fetchbox extension — page buttons.
//  * post buttons: next to posts/thumbnails on YouTube, TikTok, Instagram, X and Reddit
//  * hover button: on any <video> on any site, when the page (or embed) is downloadable
// Every candidate link is checked against yt-dlp's own list of supported sites (via Fetchbox)
// before a button appears, and channel/playlist/feed links are skipped.
(() => {
  // Run once per page, but take over from a copy left behind by an extension reload.
  if (window.__fetchboxAlive?.()) return;
  const isAlive = () => alive && !!chrome.runtime?.id;
  window.__fetchboxAlive = isAlive;

  const TOP = window === window.top;
  let S = { postButtons: true, hoverButtons: true };
  let alive = true; // false once the extension is reloaded/removed (context invalidated)

  const send = (msg) => {
    if (!alive) return Promise.resolve({ ok: false, error: "Fetchbox extension was reloaded — refresh the page" });
    try {
      return chrome.runtime.sendMessage(msg).catch((e) => fail(e));
    } catch (e) {
      return Promise.resolve(fail(e));
    }
  };
  const fail = (e) => {
    if (/context invalidated/i.test(String(e?.message || e))) { alive = false; teardown(); }
    return { ok: false, error: String(e?.message || e) };
  };

  // ── shadow-DOM overlay so site CSS can't touch our buttons ──
  const host = document.createElement("fetchbox-ui");
  host.style.cssText = "all:initial;position:fixed;inset:0 auto auto 0;width:0;height:0;z-index:2147483647;pointer-events:none";
  const root = host.attachShadow({ mode: "open" });
  // NB: no innerHTML/outerHTML anywhere — YouTube and others enforce Trusted Types, which throws on them.
  const style = document.createElement("style");
  // Buttons live inside the page (see pins below); this overlay only holds the menu and toasts.
  const BTN_CSS = `
    .fb{all:unset;position:absolute;top:0;left:0;pointer-events:auto;box-sizing:border-box;width:34px;height:34px;border-radius:50%;
      display:grid;place-items:center;cursor:pointer;color:#fff;background:rgba(14,14,16,.62);
      backdrop-filter:blur(10px) saturate(1.4);box-shadow:0 2px 8px rgba(0,0,0,.28),inset 0 0 0 1px rgba(255,255,255,.22);
      transition:background .15s,transform .15s,box-shadow .15s,color .15s}
    .fb:hover{background:rgba(14,14,16,.88);transform:scale(1.07);box-shadow:0 3px 12px rgba(0,0,0,.38),inset 0 0 0 1px rgba(255,255,255,.42)}
    .fb.open{background:#fafafa;color:#111;transform:none;box-shadow:0 3px 12px rgba(0,0,0,.38)}
    .fb:focus-visible{box-shadow:0 0 0 2px #3fb950}
    .fb svg{width:18px;height:18px}
    .fb.busy svg{animation:spin .9s linear infinite}
    .fb.ok{background:#1f9d48;color:#fff} .fb.err{background:#d1242f;color:#fff}
    @keyframes spin{to{transform:rotate(360deg)}}
  `;
  style.textContent = `
    .menu{position:fixed;pointer-events:auto;box-sizing:border-box;width:188px;padding:6px;border-radius:14px;
      background:rgba(24,24,27,.97);color:#f4f4f5;font:500 13px/1.2 system-ui,"Segoe UI",sans-serif;
      box-shadow:0 12px 32px rgba(0,0,0,.45),inset 0 0 0 1px rgba(255,255,255,.08);
      opacity:0;transform:scale(.97);transition:opacity .12s,transform .12s}
    .menu.show{opacity:1;transform:none}
    .menu .hd{padding:6px 10px 5px;font-size:10.5px;font-weight:650;color:#9b9ba5;letter-spacing:.6px;text-transform:uppercase}
    .menu button{all:unset;box-sizing:border-box;width:100%;display:flex;align-items:center;gap:10px;padding:9px 10px;
      border-radius:9px;cursor:pointer}
    .menu button:hover,.menu button:focus-visible{background:rgba(255,255,255,.09)}
    .menu button svg{width:17px;height:17px;flex:none;color:#d4d4d8}
    .toast{position:fixed;right:18px;bottom:18px;pointer-events:none;padding:10px 14px;border-radius:10px;
      background:rgba(18,18,20,.92);color:#fff;font:500 13px/1.3 system-ui,"Segoe UI",sans-serif;max-width:360px;
      box-shadow:0 4px 18px rgba(0,0,0,.35);opacity:0;transform:translateY(6px);transition:all .2s}
    .toast.show{opacity:1;transform:none} .toast.ok{border-left:4px solid #3fb950} .toast.err{border-left:4px solid #f85149}
  `;
  root.appendChild(style);

  const SVG = "http://www.w3.org/2000/svg";
  const ICONS = {
    dl: [2.2, [["path", { d: "M12 3.5v11" }], ["path", { d: "m7.5 10 4.5 4.5 4.5-4.5" }], ["path", { d: "M4.5 16.5v2a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2v-2" }]]],
    video: [2, [["rect", { x: 2.5, y: 6, width: 13, height: 12, rx: 2.5 }], ["path", { d: "m15.5 10.5 6-3.5v10l-6-3.5" }]]],
    audio: [2.2, [["path", { d: "M9 18V5l11-2v13" }], ["circle", { cx: 6, cy: 18, r: 3 }], ["circle", { cx: 17, cy: 16, r: 3 }]]],
    mute: [2.2, [["path", { d: "M11 5 6 9H3v6h3l5 4z" }], ["path", { d: "m22 9-6 6" }], ["path", { d: "m16 9 6 6" }]]],
    spin: [2.4, [["path", { d: "M12 3a9 9 0 1 0 9 9" }]]],
    ok: [2.6, [["path", { d: "m5 12 5 5 9-10" }]]],
    err: [2.6, [["path", { d: "M6 6l12 12M18 6 6 18" }]]],
  };
  function icon(name) {
    const [w, parts] = ICONS[name];
    const s = document.createElementNS(SVG, "svg");
    for (const [k, v] of Object.entries({ viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": w,
      "stroke-linecap": "round", "stroke-linejoin": "round" })) s.setAttribute(k, v);
    for (const [tag, attrs] of parts) {
      const e = document.createElementNS(SVG, tag);
      for (const k in attrs) e.setAttribute(k, attrs[k]);
      s.appendChild(e);
    }
    return s;
  }
  function el(tag, props = {}, ...kids) {
    const e = document.createElement(tag);
    for (const k in props) k === "dataset" ? Object.assign(e.dataset, props[k]) : (e[k] = props[k]);
    e.append(...kids);
    return e;
  }
  const mount = () => { if (!host.isConnected) (document.documentElement || document.body)?.appendChild(host); };
  // Keep page click handlers (play/pause, open post…) from reacting to clicks on our buttons.
  for (const ev of ["click", "mousedown", "mouseup", "pointerdown", "pointerup", "dblclick", "contextmenu"])
    host.addEventListener(ev, (e) => e.stopPropagation());

  // ── toast (top frame) ──
  let toastEl;
  function toast(text, ok) {
    if (!TOP) return;
    mount();
    if (!toastEl) { toastEl = document.createElement("div"); toastEl.className = "toast"; root.appendChild(toastEl); }
    toastEl.textContent = text;
    toastEl.className = `toast ${ok ? "ok" : "err"} show`;
    clearTimeout(toastEl._t);
    toastEl._t = setTimeout(() => (toastEl.className = "toast"), ok ? 2200 : 5000);
  }
  chrome.runtime.onMessage.addListener((m) => { if (m?.type === "toast") toast(m.text, m.ok); });

  // ── button + choice menu: the button only highlights on hover; a click opens the menu ──
  const CHOICES = [["auto", "video", "Video"], ["audio", "audio", "Audio only"], ["mute", "mute", "Video, no sound"]];
  let menuEl = null, openMenu = null; // openMenu: {btn, getUrl, corner}

  function makeButton(getUrl, corner) {
    const btn = el("button", { className: "fb", title: "Download with Fetchbox" }, icon("dl"));
    btn.setAttribute("aria-haspopup", "menu");
    btn.addEventListener("click", (e) => {
      e.preventDefault();
      if (btn.classList.contains("busy")) return;
      openMenu?.btn === btn ? closeMenu() : showMenu(btn, getUrl, corner);
    });
    return btn;
  }

  function showMenu(btn, getUrl, corner) {
    closeMenu();
    if (!menuEl) {
      menuEl = el("div", { className: "menu" }, el("div", { className: "hd", textContent: "Fetchbox" }),
        ...CHOICES.map(([mode, ic, label]) => el("button", { dataset: { mode } }, icon(ic), el("span", { textContent: label }))));
      menuEl.setAttribute("role", "menu");
      menuEl.addEventListener("click", (e) => {
        const b = e.target.closest("button");
        if (!b || !openMenu) return;
        const { btn: owner, getUrl: url } = openMenu;
        closeMenu();
        runDownload(owner, url(), b.dataset.mode);
      });
    }
    openMenu = { btn, getUrl, corner };
    btn.classList.add("open");
    root.appendChild(menuEl);
    placeMenu();
    void menuEl.offsetWidth;          // commit the start state so the fade-in plays (no rAF: paused in hidden tabs)
    menuEl.classList.add("show");
    menuEl.querySelector("button")?.focus({ preventScroll: true });
  }

  function placeMenu() {
    if (!openMenu) return;
    const r = openMenu.btn.getBoundingClientRect(), W = innerWidth, H = innerHeight;
    const mw = menuEl.offsetWidth || 188, mh = menuEl.offsetHeight || 160;
    let top = r.bottom + 6;
    if (top + mh > H - 8) top = Math.max(8, r.top - mh - 6);       // no room below: open upward
    let left = openMenu.corner === "tl" ? r.left : r.right - mw;     // line up with the button's corner
    left = Math.min(Math.max(8, left), W - mw - 8);
    menuEl.style.top = `${top}px`;
    menuEl.style.left = `${left}px`;
    menuEl.style.transformOrigin = `${openMenu.corner === "tl" ? "left" : "right"} ${top < r.top ? "bottom" : "top"}`;
  }

  function closeMenu() {
    if (!openMenu) return;
    openMenu.btn.classList.remove("open");
    openMenu = null;
    menuEl?.classList.remove("show");
    menuEl?.remove();
  }
  addEventListener("pointerdown", (e) => {
    if (openMenu && !e.composedPath().some((n) => n === menuEl || n === openMenu.btn)) closeMenu();
  }, true);
  addEventListener("keydown", (e) => { if (e.key === "Escape" && openMenu) closeMenu(); }, true);

  async function runDownload(btn, url, mode) {
    const setIcon = (name) => btn.replaceChild(icon(name), btn.firstElementChild);
    btn.classList.remove("ok", "err");
    btn.classList.add("busy");
    setIcon("spin");
    const r = await send({ type: "download", url, mode });
    btn.classList.remove("busy");
    btn.classList.add(r?.ok ? "ok" : "err");
    setIcon(r?.ok ? "ok" : "err");
    btn.title = r?.ok ? "Downloading in the background" : (r?.error || "Failed");
    toast(r?.ok ? "Downloading in the background" : `Fetchbox: ${r?.error || "failed"}`, r?.ok);
    setTimeout(() => { btn.classList.remove("ok", "err"); setIcon("dl"); btn.title = "Download with Fetchbox"; }, 2500);
  }

  // ── anchors: each button lives *inside the page*, next to its post, so the browser scrolls it
  //    together with the thumbnail (no lag, no wobble) and the site's sticky headers cover it normally.
  const anchors = new Map(); // element -> {url, corner, hover, btn, pin, cont}
  const REPLACED = /^(A|VIDEO|IMG|PICTURE|CANVAS|IFRAME|SVG)$/i;
  const BTN = 34, GAP = 8;

  // Where a pin lives:
  //  * normally at page level — above everything the site layers over a thumbnail (YouTube's hover
  //    preview would otherwise cover the button and swallow the click) and scrolled by the browser
  //    together with the page;
  //  * for a post inside its own scrolling box, inside that box, so it scrolls with the post there too.
  function scrollBox(el) {
    for (let c = el.parentElement; c && c !== document.body && c !== document.documentElement; c = c.parentElement) {
      const s = getComputedStyle(c);
      if (/(auto|scroll)/.test(s.overflowY + s.overflowX) && (c.scrollHeight > c.clientHeight + 1 || c.scrollWidth > c.clientWidth + 1)) return c;
    }
    return null;
  }
  function containerFor(el) {
    const box = scrollBox(el);
    if (!box) return document.body;
    // nearest positioned ancestor inside the scrolling box: never inside a link (a click would follow it)
    // and never inside an element with its own shadow DOM (unslotted children aren't rendered there)
    let c = REPLACED.test(el.tagName) ? el.parentElement : el;
    for (; c && box.contains(c); c = c.parentElement) {
      if (c.closest("a") || c.shadowRoot) continue;
      if (getComputedStyle(c).position !== "static") return c;
    }
    return document.body;
  }

  function setAnchor(el, url, corner, hover = false) {
    const cur = anchors.get(el);
    if (cur) { cur.url = url; return cur; }
    mount();
    const a = { url, corner, hover, btn: null, pin: null, cont: null };
    a.btn = makeButton(() => a.url, corner);
    a.pin = document.createElement("fetchbox-pin");
    a.pin.style.cssText = "all:initial;display:block;position:absolute;z-index:2147483646;width:0;height:0;pointer-events:none";
    const sr = a.pin.attachShadow({ mode: "open" });
    sr.append(Object.assign(document.createElement("style"), { textContent: BTN_CSS }), a.btn); // (`el` is the post here)
    // keep the page (play/pause, open post, hover previews…) from reacting to our clicks
    for (const ev of ["click", "mousedown", "mouseup", "pointerdown", "pointerup", "dblclick", "contextmenu"])
      a.pin.addEventListener(ev, (e) => e.stopPropagation());
    anchors.set(el, a);
    place(el, a);
    return a;
  }
  function dropAnchor(el) {
    const a = anchors.get(el);
    if (!a) return;
    if (openMenu?.btn === a.btn) closeMenu();
    a.pin.remove();
    anchors.delete(el);
  }

  // Position the pin relative to its container. Only needs redoing when the layout changes —
  // scrolling moves the post and its pin together.
  function place(el, a) {
    if (!el.isConnected) return dropAnchor(el);
    const r = el.getBoundingClientRect();
    const show = r.width >= 90 && r.height >= 56 && getComputedStyle(el).visibility !== "hidden";
    if (!a.pin.isConnected || !a.cont?.isConnected || !a.cont.contains(el)) {
      a.cont = containerFor(el);
      a.cont.appendChild(a.pin);
    }
    a.pin.style.display = show ? "block" : "none";
    if (!show) return;
    const c = a.cont, docLevel = c === document.body && getComputedStyle(c).position === "static";
    let ox, oy;
    if (docLevel) { ox = -scrollX; oy = -scrollY; }      // positioned against the whole page
    else {
      const cr = c.getBoundingClientRect(), cs = getComputedStyle(c);
      ox = cr.left + parseFloat(cs.borderLeftWidth) - c.scrollLeft;
      oy = cr.top + parseFloat(cs.borderTopWidth) - c.scrollTop;
    }
    const x = (a.corner === "tl" ? r.left + GAP : r.right - GAP - BTN) - ox;
    a.pin.style.left = `${Math.round(x)}px`;
    a.pin.style.top = `${Math.round(r.top + GAP - oy)}px`;
  }

  let layoutQueued = false;
  function scheduleLayout() {
    if (layoutQueued) return;
    layoutQueued = true;
    requestAnimationFrame(layout);
    setTimeout(layout, 120); // rAF is paused in background tabs
  }
  function layout() {
    if (!layoutQueued) return;
    layoutQueued = false;
    for (const [el, a] of [...anchors]) place(el, a);
    findBars();
    occlude();
  }

  // ── the site's own sticky/fixed bars (YouTube masthead + chip bar, X header…) must cover our buttons.
  //    Buttons are drawn high so they stay above hover previews, so we hide any that slide under a bar.
  let bars = [];
  function stuckAncestor(e, memo) {
    const path = [];
    for (; e && e !== document.body && e !== document.documentElement; e = e.parentElement) {
      if (memo.has(e)) { const v = memo.get(e); for (const p of path) memo.set(p, v); return v; }
      path.push(e);
      const pos = getComputedStyle(e).position;
      if (pos === "fixed" || pos === "sticky") { for (const p of path) memo.set(p, e); return e; }
    }
    for (const p of path) memo.set(p, null);
    return null;
  }
  function findBars() {
    if (!anchors.size) return void (bars = []);
    const found = new Set(), memo = new Map(), W = innerWidth;
    for (const fx of [0.08, 0.3, 0.5, 0.7, 0.92]) for (const y of [2, 30, 60, 90, 120, 150, 180]) {
      for (const e of document.elementsFromPoint(W * fx, y)) {
        if (e === host || e.tagName === "FETCHBOX-PIN") continue;
        const bar = stuckAncestor(e, memo);
        if (bar) found.add(bar);
      }
    }
    bars = [...found];
  }
  function occlude() {
    const rects = bars.filter((b) => b.isConnected).map((b) => [b, b.getBoundingClientRect()]).filter(([, r]) => r.width && r.height);
    for (const a of anchors.values()) {
      if (a.pin.style.display === "none") continue;
      const r = a.btn.getBoundingClientRect();
      const under = rects.some(([b, br]) => !b.contains(a.pin) &&
        r.right > br.left && r.left < br.right && r.bottom > br.top && r.top < br.bottom);
      a.pin.style.visibility = under ? "hidden" : "";
    }
  }

  addEventListener("resize", scheduleLayout, { passive: true });
  addEventListener("transitionend", scheduleLayout, { capture: true, passive: true }); // carousels that slide
  // page-level pins must follow layout changes (content loading above them, expanding sections…)
  new ResizeObserver(scheduleLayout).observe(document.documentElement);
  if (document.body) new ResizeObserver(scheduleLayout).observe(document.body);
  const layoutTimer = setInterval(() => {
    // the extension reloaded itself (e.g. after a Fetchbox update): step aside for the new copy
    if (!chrome.runtime?.id) { fail("context invalidated"); return; }
    scheduleLayout(); // catch posts that move (lazy loading, expanding text…)
  }, 800);
  addEventListener("scroll", (e) => {
    occlude();  // same frame as the scroll: hide buttons the moment they reach a sticky bar
    if (e.target !== document && e.target !== document.documentElement) scheduleLayout(); // a box inside the page scrolled
    // an open menu belongs to a spot on the page: close it when the page scrolls
    if (openMenu && !e.composedPath?.().includes(menuEl)) closeMenu();
  }, { capture: true, passive: true });

  // ── site rules for post buttons ──
  const abs = (h) => { try { return new URL(h, location.href).href; } catch { return null; } };
  const MEDIA = "img, picture, video, yt-image, yt-thumbnail-view-model, canvas";

  function thumbLinks(selector, norm = (u) => u) {
    const out = [];
    for (const a of document.querySelectorAll(selector)) {
      if (!a.querySelector(MEDIA)) continue;
      const r = a.getBoundingClientRect();
      if (r.width < 100 || r.height < 56) continue;
      const url = norm(abs(a.getAttribute("href")));
      if (url) out.push({ el: a, url });
    }
    return out;
  }

  const ytNorm = (u) => {
    try {
      const x = new URL(u);
      if (x.pathname === "/watch") return `https://www.youtube.com/watch?v=${x.searchParams.get("v")}`;
      const m = x.pathname.match(/^\/shorts\/([\w-]+)/);
      return m ? `https://www.youtube.com/shorts/${m[1]}` : u;
    } catch { return u; }
  };
  // Instagram links now look like /nasa/reel/ID/ — normalise to the canonical /reel/ID/
  const igNorm = (u) => {
    const m = u && u.match(/instagram\.com\/(?:[\w.]+\/)?(p|reel|reels|tv)\/([\w-]+)/);
    return m ? `https://www.instagram.com/${m[1] === "reels" ? "reel" : m[1]}/${m[2]}/` : u;
  };
  const largestMedia = (scope) => {
    let best = null, area = 0;
    for (const m of scope.querySelectorAll("video, img")) {
      const r = m.getBoundingClientRect();
      if (r.width * r.height > area && r.width >= 150 && r.height >= 100) { best = m; area = r.width * r.height; }
    }
    return best;
  };

  const RULES = [
    { host: /(^|\.)youtube\.com$/, corner: "tl",
      scan: () => thumbLinks('a[href*="/watch?v="], a[href*="/shorts/"]', ytNorm) },
    { host: /(^|\.)tiktok\.com$/, corner: "tr",
      scan: () => {
        const out = thumbLinks('a[href*="/video/"], a[href*="/photo/"]');
        // For You / Following feed: no link around the player, but the player id carries the video id.
        for (const art of document.querySelectorAll('article[data-e2e="recommend-list-item-container"]')) {
          const w = art.querySelector('[id^="xgwrapper-"]');
          const id = w?.id.match(/(\d{15,})$/)?.[1];
          // the video card (players get recycled between posts, so don't pin to the <video> itself)
          const media = art.querySelector('section[data-e2e="feed-video"]') || art;
          // Author isn't in the feed markup; yt-dlp and cobalt only need the id, "_" fills the username slot.
          if (id) out.push({ el: media, url: `https://www.tiktok.com/@_/video/${id}` });
        }
        return out;
      } },
    { host: /(^|\.)instagram\.com$/, corner: "tl",
      scan: () => {
        const out = thumbLinks('a[href*="/p/"], a[href*="/reel/"], a[href*="/tv/"]', igNorm);
        for (const art of document.querySelectorAll("article")) { // home feed posts
          const link = [...art.querySelectorAll('a[href*="/p/"], a[href*="/reel/"]')].map((a) => igNorm(abs(a.getAttribute("href")))).find(Boolean);
          const media = largestMedia(art);
          if (link && media && !media.closest("a")) out.push({ el: media, url: link });
        }
        return out;
      } },
    { host: /(^|\.)(x|twitter)\.com$/, corner: "tr",
      scan: () => {
        const out = [];
        for (const art of document.querySelectorAll("article")) {
          // A quoted tweet is an <article> inside the quoting one: each tweet uses only its own video and link,
          // so a video in the quoted tweet downloads that tweet, not the one quoting it.
          const own = (n) => n.closest("article") === art;
          const media = [...art.querySelectorAll('video, img[src*="video_thumb"]')].find(own);
          if (!media) continue;
          const status = [...art.querySelectorAll('a[href*="/status/"]')].filter(own)
            .map((a) => abs(a.getAttribute("href"))?.match(/^(https:\/\/[^/]+\/[^/]+\/status\/\d+)/)?.[1]).find(Boolean);
          const el = media.closest('[data-testid="videoPlayer"], [data-testid="videoComponent"]') || media;
          if (status) out.push({ el, url: status });
        }
        return out;
      } },
    { host: /(^|\.)reddit\.com$/, corner: "tr",
      scan: () => {
        const out = [];
        for (const p of document.querySelectorAll("shreddit-post")) {
          const type = p.getAttribute("post-type");
          const content = p.getAttribute("content-href") || "";
          const perma = abs(p.getAttribute("permalink") || "");
          const url = type === "video" || type === "gif" || /v\.redd\.it/.test(content) ? perma
                    : (type === "link" || type === "embed") && content && !/reddit\.com/.test(content) ? content : null;
          const el = p.querySelector("shreddit-player, shreddit-player-2, shreddit-embed, shreddit-aspect-ratio") || p;
          if (url) out.push({ el, url });
        }
        for (const t of document.querySelectorAll(".thing.link[data-url]")) { // old.reddit.com
          if (!/v\.redd\.it/.test(t.dataset.url)) continue;
          const el = t.querySelector(".thumbnail") || t;
          out.push({ el, url: abs(t.dataset.permalink) });
        }
        return out;
      } },
  ];
  const rule = RULES.find((r) => r.host.test(location.hostname));

  let seen = new WeakMap(); // element -> url already evaluated
  let scanQueued = 0;
  function scheduleScan(ms = 350) {
    if (scanQueued) return;
    scanQueued = setTimeout(() => { scanQueued = 0; scan(); }, ms);
  }
  async function scan() {
    if (!alive || !rule || !S.postButtons) return;
    const items = rule.scan().filter((it) => it.url && seen.get(it.el) !== it.url);
    if (!items.length) return;
    for (const it of items) { seen.set(it.el, it.url); dropAnchor(it.el); } // DOM nodes get recycled (YouTube): re-evaluate
    const r = await send({ type: "check", urls: [...new Set(items.map((i) => i.url))] });
    if (!r?.ok) return;
    for (const it of items) {
      const v = r.results[it.url];
      if (v?.supported && !v.collection && seen.get(it.el) === it.url) setAnchor(it.el, it.url, rule.corner);
    }
    scheduleLayout(); // new buttons: check them against the site's sticky bars right away
  }
  const mo = new MutationObserver(() => { scheduleScan(); scheduleLayout(); });

  // ── hover button for any <video> ──
  let hoverEl = null, hoverHide = 0, lastMove = 0;
  const pageUrl = () => location.href;
  async function onMove(e) {
    if (!alive || !S.hoverButtons) return;
    const now = Date.now();
    if (now - lastMove < 120) return;
    lastMove = now;
    const els = document.elementsFromPoint(e.clientX, e.clientY);
    if (els.some((n) => n === host || n.tagName === "FETCHBOX-PIN")) { clearTimeout(hoverHide); return; } // over our own button
    const video = els.find((x) => x.tagName === "VIDEO");
    if (!video) { if (hoverEl) armHide(); return; }
    clearTimeout(hoverHide);
    if (video === hoverEl) return;
    const r = video.getBoundingClientRect();
    if (r.width < 160 || r.height < 90) return;
    // Already covered by a post button?
    for (const [el, a] of anchors) if (!a.hover && (el === video || el.contains(video) || video.contains(el))) return;
    const url = pageUrl();
    const res = (await send({ type: "check", urls: [url] }))?.results?.[url];
    if (!res?.supported || res.collection) return;
    if (hoverEl && hoverEl !== video) dropAnchor(hoverEl);
    hoverEl = video;
    setAnchor(video, url, "tr", true);
  }
  function armHide() {
    clearTimeout(hoverHide);
    hoverHide = setTimeout(() => {
      if (!hoverEl) return;
      if (openMenu && openMenu.btn === anchors.get(hoverEl)?.btn) return armHide(); // keep it while its menu is open
      dropAnchor(hoverEl);
      hoverEl = null;
    }, 1200);
  }
  addEventListener("pointermove", onMove, { passive: true });

  // ── SPA navigation: re-check everything when the URL changes ──
  let lastHref = location.href;
  const navTimer = setInterval(() => {
    if (location.href === lastHref) return;
    lastHref = location.href;
    if (hoverEl) { dropAnchor(hoverEl); hoverEl = null; }
    scheduleScan(600);
  }, 500);

  function teardown() {
    clearInterval(layoutTimer); clearInterval(navTimer); mo.disconnect();
    removeEventListener("pointermove", onMove);
    for (const el of [...anchors.keys()]) dropAnchor(el);
    host.remove();
    if (window.__fetchboxAlive === isAlive) delete window.__fetchboxAlive;
  }

  function applySettings() {
    if (!S.postButtons) for (const [el, a] of [...anchors]) if (!a.hover) dropAnchor(el);
    if (!S.hoverButtons && hoverEl) { dropAnchor(hoverEl); hoverEl = null; }
    if (S.postButtons) { seen = new WeakMap(); scheduleScan(50); }
  }

  send({ type: "settings" }).then((s) => {
    if (s && !s.error) S = { ...S, ...s };
    if (rule) {
      mo.observe(document.documentElement, { childList: true, subtree: true });
      addEventListener("scroll", () => scheduleScan(500), { capture: true, passive: true });
      scheduleScan(200);
    }
  });
  chrome.storage.onChanged.addListener((ch, area) => {
    if (area !== "sync") return;
    for (const k in ch) S[k] = ch[k].newValue;
    applySettings();
  });
})();
