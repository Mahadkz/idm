const seen = new Set();
const api = typeof chrome !== "undefined" ? chrome : (typeof browser !== "undefined" ? browser : null);

function isAd(url) {
  return /(?:[?&](?:ad_id|ad_url|adv_id|adid)=|\/ad_preroll|\/ads?\/|\/advert|\bdoubleclick\b|\bpagead\b)/i.test(url);
}

// B25: Keyboard and mouse modifier state
var modifierState = { alt: false, ctrl: false, shift: false, meta: false };

function keydownHandler(e) {
  if (!e) return;
  modifierState.alt = Boolean(e.altKey || e.key === "Alt");
  modifierState.ctrl = Boolean(e.ctrlKey || e.key === "Control");
  modifierState.shift = Boolean(e.shiftKey || e.key === "Shift");
  modifierState.meta = Boolean(e.metaKey || e.key === "Meta");
}

function keyupHandler(e) {
  if (!e) return;
  if (e.key === "Alt") modifierState.alt = false;
  else if (e.altKey !== undefined) modifierState.alt = Boolean(e.altKey);
  if (e.key === "Control") modifierState.ctrl = false;
  else if (e.ctrlKey !== undefined) modifierState.ctrl = Boolean(e.ctrlKey);
  if (e.key === "Shift") modifierState.shift = false;
  else if (e.shiftKey !== undefined) modifierState.shift = Boolean(e.shiftKey);
  if (e.key === "Meta") modifierState.meta = false;
  else if (e.metaKey !== undefined) modifierState.meta = Boolean(e.metaKey);
}

function resetModifiers() {
  modifierState.alt = false;
  modifierState.ctrl = false;
  modifierState.shift = false;
  modifierState.meta = false;
}

if (typeof window !== "undefined" && window.addEventListener) {
  window.addEventListener("keydown", keydownHandler, true);
  window.addEventListener("keyup", keyupHandler, true);
  window.addEventListener("blur", resetModifiers, true);
} else if (typeof document !== "undefined" && document.addEventListener) {
  document.addEventListener("keydown", keydownHandler, true);
  document.addEventListener("keyup", keyupHandler, true);
}
if (typeof document !== "undefined" && document.addEventListener) {
  // A hidden tab never receives the key-up, so drop any held modifier.
  document.addEventListener("visibilitychange", () => { if (document.hidden) resetModifiers(); });
}

// B25: user-configurable force/bypass capture on link clicks.
// force = hand the link to XDM even if filtered; bypass = let the browser download it.
const DEFAULT_CAPTURE_KEYS = { force: "alt+shift", bypass: "alt" };
var captureKeys = { ...DEFAULT_CAPTURE_KEYS };

function parseCombo(text) {
  if (typeof text !== "string" || !text.trim()) return null;
  const combo = { alt: false, ctrl: false, shift: false, meta: false };
  for (const part of text.toLowerCase().split("+").map(p => p.trim()).filter(Boolean)) {
    const name = part === "control" ? "ctrl" : part === "option" ? "alt" : part === "cmd" || part === "command" ? "meta" : part;
    if (!(name in combo)) return null;
    combo[name] = true;
  }
  return Object.values(combo).some(Boolean) ? combo : null;
}

function sameCombo(a, b) {
  return a.alt === b.alt && a.ctrl === b.ctrl && a.shift === b.shift && a.meta === b.meta;
}

function setCaptureKeys(config) {
  const force = parseCombo(config?.force);
  const bypass = parseCombo(config?.bypass);
  // Identical or unparsable combinations are ambiguous, so fall back to the defaults.
  if (!force || !bypass || sameCombo(force, bypass)) {
    captureKeys = { ...DEFAULT_CAPTURE_KEYS };
  } else {
    captureKeys = { force: config.force, bypass: config.bypass };
  }
  return captureKeys;
}

// The event's own flags are authoritative; the tracked state is only a fallback for synthetic events.
function modifiersFromEvent(e) {
  const pick = (flag, tracked) => typeof flag === "boolean" ? flag : tracked;
  return {
    alt: pick(e?.altKey, modifierState.alt),
    ctrl: pick(e?.ctrlKey, modifierState.ctrl),
    shift: pick(e?.shiftKey, modifierState.shift),
    meta: pick(e?.metaKey, modifierState.meta)
  };
}

function captureModeFor(e) {
  const held = modifiersFromEvent(e);
  if (!Object.values(held).some(Boolean)) return null;
  if (sameCombo(held, parseCombo(captureKeys.force))) return "force";
  if (sameCombo(held, parseCombo(captureKeys.bypass))) return "bypass";
  return null;
}

function clickHandler(e) {
  if (!e || (typeof e.button === "number" && e.button !== 0)) return null;
  let node = e.target;
  while (node && !(String(node.tagName).toUpperCase() === "A" && (node.href || node.getAttribute?.("href")))) node = node.parentNode;
  if (!node) return null;
  const url = node.href || node.getAttribute("href");
  if (!/^https?:/i.test(url || "")) return null;
  const mode = captureModeFor(e);
  if (!mode) return null;
  if (mode === "force" && typeof e.preventDefault === "function") e.preventDefault();
  api?.runtime?.sendMessage({
    type: "capture.override",
    mode,
    url,
    pageUrl: typeof location !== "undefined" ? location.href : ""
  })?.catch?.(() => {});
  return mode;
}

if (typeof document !== "undefined" && document.addEventListener) {
  document.addEventListener("click", clickHandler, true);
}

if (api?.storage?.local?.get) {
  api.storage.local.get({ captureKeys: DEFAULT_CAPTURE_KEYS }).then(v => setCaptureKeys(v.captureKeys)).catch?.(() => {});
  api.storage.onChanged?.addListener?.((changes, area) => {
    if (area === "local" && changes.captureKeys) setCaptureKeys(changes.captureKeys.newValue);
  });
}

// B30: Batch link extraction across frames and selected text
function extractLinks(options = {}) {
  const results = [];
  const seenUrls = new Set();
  const doc = options.document || (typeof document !== "undefined" ? document : null);
  if (!doc) return results;

  const addLink = (url, text = "", type = "link") => {
    if (!url || typeof url !== "string") return;
    const trimmed = url.trim();
    if (!/^https?:/i.test(trimmed)) return;
    if (seenUrls.has(trimmed)) return;
    seenUrls.add(trimmed);
    results.push({ url: trimmed, text: (text || "").trim(), type });
  };

  const selectedText = options.selectedText || (typeof window !== "undefined" && window.getSelection ? (window.getSelection()?.toString() || "") : "");
  if (options.selectionOnly || (selectedText && options.useSelection)) {
    const urlRegex = /https?:\/\/[^\s"'<>]+/gi;
    let match;
    while ((match = urlRegex.exec(selectedText)) !== null) {
      addLink(match[0], match[0], "selection");
    }
    // Links the user selected without their URL being visible text.
    const sel = options.selection || (typeof window !== "undefined" && window.getSelection ? window.getSelection() : null);
    if (sel && sel.rangeCount > 0 && typeof sel.containsNode === "function" && doc.querySelectorAll) {
      for (const a of doc.querySelectorAll("a")) {
        if (sel.containsNode(a, true)) {
          addLink(a.href || a.getAttribute?.("href"), a.innerText || a.textContent || "", "anchor");
        }
      }
    }
    return results;
  }

  const anchors = doc.querySelectorAll ? [...doc.querySelectorAll("a")] : [];
  for (const a of anchors) {
    const href = a.href || a.getAttribute?.("href") || a.attributes?.href;
    const text = a.innerText || a.textContent || a.title || a.getAttribute?.("title") || a.attributes?.title || "";
    addLink(href, text, "anchor");
  }

  const areas = doc.querySelectorAll ? [...doc.querySelectorAll("area")] : [];
  for (const area of areas) {
    const href = area.href || area.getAttribute?.("href") || area.attributes?.href;
    const text = area.alt || area.getAttribute?.("alt") || area.attributes?.alt || "";
    addLink(href, text, "area");
  }

  const images = doc.querySelectorAll ? [...doc.querySelectorAll("img")] : [];
  for (const img of images) {
    const src = img.src || img.getAttribute?.("src") || img.attributes?.src;
    const text = img.alt || img.title || img.getAttribute?.("alt") || img.attributes?.alt || "";
    addLink(src, text, "image");
  }

  const mediaNodes = doc.querySelectorAll ? [...doc.querySelectorAll("video"), ...doc.querySelectorAll("audio")] : [];
  for (const m of mediaNodes) {
    const src = m.currentSrc || m.src || m.getAttribute?.("src") || m.attributes?.src;
    addLink(src, m.title || "", "media");
  }

  return results;
}

// B32: Selection detection and coordinates
var _currentSelection = null;

function selectionHandler(customSel) {
  const sel = customSel ||
              (typeof window !== "undefined" && window.getSelection ? window.getSelection() : null) ||
              (typeof document !== "undefined" && document.getSelection ? document.getSelection() : null);
  if (!sel || sel.isCollapsed || !sel.toString?.().trim()) {
    _currentSelection = null;
    return null;
  }
  const text = sel.toString().trim();
  let rect = { x: 0, y: 0, width: 0, height: 0, top: 0, left: 0, bottom: 0, right: 0 };
  if (sel.rangeCount > 0) {
    const range = sel.getRangeAt(0);
    if (typeof range?.getBoundingClientRect === "function") {
      rect = range.getBoundingClientRect();
    }
  }
  const extracted = extractLinks({ selectedText: text, selectionOnly: true, selection: sel });
  _currentSelection = {
    text,
    rect,
    linkCount: extracted.length,
    links: extracted
  };
  return _currentSelection;
}

if (typeof document !== "undefined" && document.addEventListener) {
  document.addEventListener("selectionchange", selectionHandler);
  document.addEventListener("mouseup", selectionHandler);
}

// B41: DOM property and selector extraction helpers
function queryElementMetadata(elementOrSelector, properties) {
  let el = elementOrSelector;
  if (typeof el === "string") {
    el = typeof document !== "undefined" && document.querySelector ? document.querySelector(el) : null;
  }
  if (!el || !(el instanceof Element)) return null;

  const isConnected = el.isConnected !== undefined
    ? el.isConnected
    : (typeof document !== "undefined" && document.documentElement ? (document.documentElement === el || (document.contains ? document.contains(el) : true)) : true);
  if (!isConnected) return null;

  const propsToExtract = Array.isArray(properties)
    ? properties
    : ["src", "currentSrc", "title", "tagName", "videoWidth", "videoHeight", "duration", "poster"];

  const metadata = {};
  for (const prop of propsToExtract) {
    if (prop in el) {
      metadata[prop] = el[prop];
    } else if (typeof el.getAttribute === "function") {
      metadata[prop] = el.getAttribute(prop) ?? undefined;
    } else if (el.attributes && el.attributes[prop] !== undefined) {
      metadata[prop] = el.attributes[prop];
    }
  }
  return metadata;
}

// B42: Site adapters for dynamic players.
// Each adapter is independent and versioned; an adapter whose minHostVersion is above the
// protocol version this extension speaks is disabled instead of guessing at host behavior.
var HOST_PROTOCOL_VERSION = 1;

// Expiry hints are read from common signed-URL query parameters only (no site-specific deciphering).
function urlExpiry(url) {
  let params;
  try { params = new URL(url).searchParams; } catch { return null; }
  const asMs = value => {
    const n = Number(value);
    if (!Number.isFinite(n) || n <= 0) return null;
    return n < 1e11 ? n * 1000 : n;
  };
  for (const key of ["expires", "expire", "exp", "Expires", "se", "e"]) {
    const ms = asMs(params.get(key));
    if (ms) return ms;
  }
  const amzDate = params.get("X-Amz-Date");
  const amzTtl = Number(params.get("X-Amz-Expires"));
  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(amzDate || "");
  if (m && Number.isFinite(amzTtl)) return Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]) + amzTtl * 1000;
  return null;
}

function urlExpired(url, now = Date.now()) {
  const at = urlExpiry(url);
  return at !== null && at <= now;
}

function playerUrls(p) {
  const sources = p.querySelectorAll ? [...p.querySelectorAll("source")].map(s => s.src || s.getAttribute?.("src")) : [];
  return [p.currentSrc, p.src, ...sources].filter(Boolean);
}

// Re-reads the live player that produced a media item so a rotated URL replaces the stale one.
async function refreshFromPage(mediaItem) {
  const old = mediaItem?.url;
  let fresh = null;
  for (const entry of playerRegistry.entries.values()) {
    const el = typeof entry.ref?.deref === "function" ? entry.ref.deref() : entry.ref;
    if (!el || (entry.url !== old && !playerUrls(el).includes(old))) continue;
    fresh = playerUrls(el).find(u => /^https?:/i.test(u)) || null;
    break;
  }
  if (!fresh && typeof document !== "undefined" && document.querySelectorAll) {
    for (const el of [...document.querySelectorAll("video"), ...document.querySelectorAll("audio")]) {
      if (playerUrls(el).includes(old)) { fresh = playerUrls(el).find(u => /^https?:/i.test(u)); break; }
    }
  }
  if (!fresh) return { url: old, expired: urlExpired(old), refreshed: false, reason: "player-gone" };
  return { url: fresh, expired: urlExpired(fresh), refreshed: fresh !== old };
}

var siteAdapters = [
  {
    // YouTube watch pages, shorts, embeds and youtu.be. The <video> element has a blob: source, so it is
    // identified by the page and video id instead of a URL (see ytState below).
    id: "youtube",
    version: 1,
    minHostVersion: 1,
    name: "YouTube Page Video Adapter",
    matches: url => typeof XdmVideo !== "undefined" && !!XdmVideo.ytVideoId(url),
    extractMedia: node => {
      if (!node || !(node instanceof Element) || typeof ytPageItem !== "function") return [];
      const item = ytPageItem();
      return item ? [item] : [];
    },
    refreshUrl: async item => ({ url: item?.url, expired: false, refreshed: false })
  },
  {
    id: "embedded-stream",
    version: 1,
    minHostVersion: 1,
    name: "Embedded Stream Adapter",
    matches: url => /\b(?:stream|manifest|live|embed)\b/i.test(url),
    extractMedia: node => {
      if (!node || !(node instanceof Element)) return [];
      const sources = node.querySelectorAll ? [...node.querySelectorAll("source, track")] : [];
      return sources.map(s => ({
        url: s.src || s.getAttribute?.("src"),
        mime: s.type || s.getAttribute?.("type") || "application/x-mpegURL",
        title: s.title || (typeof document !== "undefined" ? document.title : "")
      })).filter(item => item.url && /^https?:/i.test(item.url));
    },
    refreshUrl: refreshFromPage
  },
  {
    id: "generic-html5",
    version: 2,
    minHostVersion: 1,
    name: "Generic HTML5 Media Adapter",
    matches: url => /^https?:\/\//i.test(url),
    extractMedia: node => {
      if (!node || !(node instanceof Element)) return [];
      const players = node.matches("video,audio") ? [node] : (node.querySelectorAll ? [...node.querySelectorAll("video,audio")] : []);
      const out = [];
      for (const p of players) {
        const mime = (p.tagName || "video").toLowerCase() + "/*";
        const title = p.title || (typeof document !== "undefined" ? document.title : "");
        for (const url of playerUrls(p)) if (/^https?:/i.test(url)) out.push({ url, mime, title });
      }
      return out;
    },
    refreshUrl: refreshFromPage
  }
];

function enabledAdapters(hostVersion = HOST_PROTOCOL_VERSION) {
  return siteAdapters.filter(a => (a.minHostVersion || 1) <= hostVersion);
}

// Specific adapters are listed before the generic fallback, so the first match is the most specific.
function getSiteAdapter(url, hostVersion = HOST_PROTOCOL_VERSION) {
  const list = enabledAdapters(hostVersion);
  return list.find(a => a.matches(url)) || list[list.length - 1] || null;
}

// Collects media for a player from every enabled adapter that matches the page or a player URL.
function adapterMedia(player, pageUrl = "") {
  const urls = [pageUrl, ...playerUrls(player)].filter(Boolean);
  const items = [];
  const seenUrls = new Set();
  for (const adapter of enabledAdapters()) {
    if (!urls.some(u => adapter.matches(u))) continue;
    for (const item of adapter.extractMedia(player)) {
      if (seenUrls.has(item.url)) continue;
      seenUrls.add(item.url);
      items.push({ ...item, adapterId: adapter.id, adapterVersion: adapter.version });
    }
  }
  return items;
}

// Overlay policy from the versioned `mediaSettings` storage object: full or mini mode and per-site disable.
var overlayPolicy = { mode: "full", disabled: false, raw: null };

function hostInList(host, list) {
  host = String(host || "").toLowerCase();
  return (Array.isArray(list) ? list : []).some(pattern => {
    const p = String(pattern || "").trim().toLowerCase();
    if (!p) return false;
    if (p.startsWith("*.")) return host === p.slice(2) || host.endsWith("." + p.slice(2));
    return host === p;
  });
}

function setOverlayPolicy(raw) {
  const r = raw && typeof raw === "object" ? raw : {};
  const host = typeof location !== "undefined" ? location.hostname : "";
  overlayPolicy = {
    mode: r.overlayMode === "mini" ? "mini" : "full",
    disabled: hostInList(host, r.overlayDisabledSites) || hostInList(host, r.panelExcludedSites),
    raw: r
  };
  if (typeof playerPanel !== "undefined") playerPanel.applyPolicy();
}

if (api?.storage?.local?.get) {
  api.storage.local.get({ mediaSettings: {} }).then(v => setOverlayPolicy(v.mediaSettings)).catch?.(() => {});
  api.storage.onChanged?.addListener?.((changes, area) => {
    if (area === "local" && changes.mediaSettings) setOverlayPolicy(changes.mediaSettings.newValue);
  });
}

// Chromium injects page_video.js as a MAIN-world content script (manifest). Firefox MV2 has no such world, so
// the extension's own web-accessible script is added to the page instead.
if (typeof document !== "undefined" && api?.runtime?.getManifest?.().manifest_version === 2 && /(^|\.)youtube(?:-nocookie)?\.com$/i.test(location.hostname)) {
  try {
    const tag = document.createElement("script");
    tag.src = api.runtime.getURL("page_video.js");
    tag.onload = () => tag.remove();
    (document.head || document.documentElement).appendChild(tag);
  } catch {}
}

// Page-video state for the YouTube adapter. The page script (page_video.js, main world) only reports the
// page's own player response: which video is playing and whether it is downloadable at all. The formats
// themselves come from yt-dlp (see ytdlpMenu); no stream address is ever taken from the page.
var ytState = { videoId: null, model: null };

function ytPostToPage(message) {
  try { if (typeof window !== "undefined" && typeof window.postMessage === "function") window.postMessage({ xdmVideo: 1, ...message }, location.origin); } catch {}
}

function ytPageItem() {
  const m = ytState.model;
  if (!m || !m.playable || typeof XdmVideo === "undefined") return null;
  const here = XdmVideo.ytVideoId(typeof location !== "undefined" ? location.href : "");
  if (here && here !== m.videoId) return null;
  return { url: `https://www.youtube.com/watch?v=${m.videoId}`, mime: "video/*", title: m.title, kind: "page-video", videoId: m.videoId };
}

function ytOnPlayer(response) {
  if (typeof XdmVideo === "undefined") return;
  const model = XdmVideo.normalizePlayerResponse(response);
  if (!model || !model.videoId) return;
  const here = XdmVideo.ytVideoId(typeof location !== "undefined" ? location.href : "");
  if (here && here !== model.videoId) return;
  const previous = ytState.model;
  // A later response for the same video may carry more formats (or addresses); keep the richer one.
  if (previous && previous.videoId === model.videoId && previous.formats.length > model.formats.length && !model.hasUrls) return;
  if (!previous || previous.videoId !== model.videoId) ytState.videoId = null;
  ytState.videoId = model.videoId;
  ytState.model = model;
  if (model.drm) {
    for (const entry of playerRegistry.entries.values()) if (entry.videoId === model.videoId) api?.runtime?.sendMessage({ type: "page.protected", url: `https://www.youtube.com/watch?v=${model.videoId}` })?.catch?.(() => {});
  }
  if (typeof document !== "undefined" && document.documentElement) debouncedInspect(document.documentElement, 50);
}

var lastVideoMenu = [];

// Asks the background script (and through the native host, the daemon and yt-dlp) for the formats of this
// page's video. Every entry names the yt-dlp format ids it needs; the daemon resolves the addresses when the
// download runs.
async function ytdlpMenu(title) {
  const pageUrl = typeof location !== "undefined" ? location.href : "";
  const reply = await api.runtime.sendMessage({ type: "video.ytdlp", pageUrl, title: title || (typeof document !== "undefined" ? document.title : "") });
  if (!reply?.ok) return { options: [], note: reply?.missing ? reply.reason : `Could not get formats: ${reply?.reason || "no answer from XDM"}` };
  const entries = XdmVideo.buildYtdlpMenu(reply);
  if (!entries.length) return { options: [], note: reply.live ? "Live streams cannot be downloaded from here." : "yt-dlp found no downloadable formats for this page." };
  return {
    options: entries.map(entry => ({
      label: entry.label,
      detail: entry.detail,
      audioOnly: !!entry.audioOnly,
      run: () => api.runtime.sendMessage({ type: "video.download", choice: entry.choice, pageUrl })
    }))
  };
}

if (typeof window !== "undefined" && window.addEventListener) {
  window.addEventListener("message", event => {
    const m = event?.data;
    if (!m || m.xdmVideo !== 1 || (event.source !== undefined && event.source !== window)) return;
    if (m.type === "player") ytOnPlayer(m.response);
    else if (m.type === "ready") ytPostToPage({ type: "hello" });
  });
}

// Pages whose <video> plays a blob: (MSE) source have no address of their own. When the background script has
// captured an HLS or DASH manifest for the tab, the player is offered that stream's qualities instead.
var streamProbes = typeof WeakMap !== "undefined" ? new WeakMap() : new Map();

function probeStreams(player) {
  if (!api?.runtime?.sendMessage || ytState.model || typeof XdmVideo !== "undefined" && XdmVideo.ytVideoId(typeof location !== "undefined" ? location.href : "")) return;
  const st = streamProbes.get(player) || { tries: 0, found: false };
  if (st.found || st.tries >= 8) return;
  st.tries++;
  streamProbes.set(player, st);
  api.runtime.sendMessage({ type: "media.list" }).then(reply => {
    const stream = (reply?.items || []).filter(item => item.kind === "hls" || item.kind === "dash").pop();
    if (stream) {
      st.found = true;
      playerRegistry.register(player, { url: stream.url, title: document.title || "", kind: "stream", streamKind: stream.kind });
    } else if (st.tries < 8) {
      setTimeout(() => probeStreams(player), 2500);
    }
  }).catch?.(() => {});
}

// Any other player: the background script lists what it captured for the tab (direct files, one entry each;
// HLS/DASH variants, one entry per resolution) plus the player's own source address.
async function genericMenu(info) {
  const reply = await api.runtime.sendMessage({ type: "video.options", playerUrl: info.url, title: info.title || document.title, pageUrl: typeof location !== "undefined" ? location.href : "" });
  if (!reply?.ok || !reply.options?.length) return { options: [], moreFormats: true, note: reply?.reason || "No downloadable formats found for this video yet." };
  return {
    moreFormats: true,
    options: reply.options.map(option => ({
      label: option.label,
      detail: option.detail,
      audioOnly: !!option.audioOnly,
      run: () => api.runtime.sendMessage({ type: "video.download", choice: option.choice, pageUrl: typeof location !== "undefined" ? location.href : "" })
    }))
  };
}

// B48: Floating "Download with XDM" button attached to detected players. The visible UI lives inside a
// closed shadow root so page CSS and page scripts cannot restyle or reach it; the host element only carries
// position. Keyboard: the button is focusable, Escape dismisses the overlay and returns focus to the player,
// and key events never leak to page shortcuts.
var playerPanel = (() => {
  const panels = new Map();
  const parts = new Map();
  const owned = typeof WeakSet !== "undefined" ? new WeakSet() : new Set();
  const MIN_SIZE = 40;
  const WIDTHS = { full: 144, mini: 44 };
  const CSS = ":host{all:initial}button{all:initial;box-sizing:border-box;display:block;cursor:pointer;font:600 12px/1.2 sans-serif;color:#fff;background:#1a56c4;border:1px solid #fff;border-radius:4px;padding:6px 10px;text-align:center;width:100%}button:hover{background:#2a6ae0}button:focus-visible{outline:2px solid #ffbf00;outline-offset:1px}.menu{position:absolute;top:34px;right:0;width:250px;max-height:60vh;overflow:auto;background:#fff;color:#111;border:1px solid #1a56c4;border-radius:4px;box-shadow:0 4px 14px rgba(0,0,0,.4);font:13px/1.3 sans-serif}.menu .head{padding:6px 10px;font-weight:600;background:#eef3fd;border-bottom:1px solid #c9d6f0}.menu .opt{display:flex;justify-content:space-between;gap:8px;width:100%;background:#fff;color:#111;border:0;border-bottom:1px solid #eee;border-radius:0;padding:7px 10px;text-align:left;font:13px/1.3 sans-serif}.menu .opt:hover,.menu .opt:focus-visible{background:#dce8fb}.menu .opt span{color:#555}.menu .note{padding:8px 10px;color:#333}";

  const kebab = name => name.replace(/[A-Z]/g, c => "-" + c.toLowerCase());
  // Plain assignment plus an !important inline declaration so page rules with !important cannot override it.
  function setStyle(el, prop, value) {
    if (!el.style) el.style = {};
    el.style[prop] = value;
    if (typeof el.style.setProperty === "function") el.style.setProperty(kebab(prop), value, "important");
  }

  function width() { return WIDTHS[overlayPolicy.mode] || WIDTHS.full; }

  function labelFor(btn, info) {
    const mini = overlayPolicy.mode === "mini";
    const text = "Download this video";
    btn.textContent = mini ? "XDM" : text;
    btn.setAttribute?.("aria-label", text);
    btn.title = text;
  }

  function create(playerId, mediaInfo = {}) {
    if (!playerId) return null;
    if (panels.has(playerId)) return panels.get(playerId);
    const doc = typeof document !== "undefined" ? document : null;
    if (!doc || typeof doc.createElement !== "function") {
      const dummyPanel = { id: `panel-${playerId}`, visible: true, mediaInfo };
      panels.set(playerId, dummyPanel);
      return dummyPanel;
    }

    const panel = doc.createElement("div");
    panel.className = "xdm-player-panel";
    panel.setAttribute?.("data-player-id", playerId);
    if (!panel.style) panel.style = {};
    setStyle(panel, "position", "fixed");
    setStyle(panel, "zIndex", "2147483647");
    setStyle(panel, "display", "none");
    setStyle(panel, "pointerEvents", "auto");
    setStyle(panel, "width", `${width()}px`);
    panel.mediaInfo = mediaInfo;
    owned.add(panel);

    let root = panel;
    if (typeof panel.attachShadow === "function") {
      try { root = panel.attachShadow({ mode: "closed" }); } catch { root = panel; }
    }
    const style = doc.createElement("style");
    style.textContent = CSS;
    root.appendChild(style);
    const btn = doc.createElement("button");
    btn.className = "xdm-download-btn";
    btn.type = "button";
    labelFor(btn, mediaInfo);
    // Reads the live entry so a replaced source is never downloaded from a stale URL.
    btn.onclick = event => {
      event?.stopPropagation?.();
      event?.preventDefault?.();
      const info = panel.mediaInfo || {};
      if (info.kind || info.url) {
        if (parts.get(playerId)?.menu) closeMenu(playerId); else openMenu(playerId);
      }
    };
    owned.add(btn);
    root.appendChild(btn);
    const part = { root, btn, focusTarget: null };
    root.addEventListener?.("keydown", event => {
      event?.stopPropagation?.();
      if (event?.key === "Escape") {
        event.preventDefault?.();
        if (part.menu) { closeMenu(playerId); try { btn.focus?.(); } catch {} return; }
        dismiss(playerId);
        try { part.focusTarget?.(); } catch {}
      }
    });
    for (const type of ["keyup", "keypress"]) root.addEventListener?.(type, event => event?.stopPropagation?.());

    panels.set(playerId, panel);
    parts.set(playerId, part);
    return panel;
  }

  function attach(playerId) {
    const panel = panels.get(playerId);
    const doc = typeof document !== "undefined" ? document : null;
    if (!panel || !doc || panel.parentNode) return false;
    const parent = doc.body || doc.documentElement;
    if (!parent) return false;
    parent.appendChild(panel);
    return true;
  }

  function closeMenu(playerId) {
    const part = parts.get(playerId);
    if (!part?.menu) return;
    try { part.root.removeChild(part.menu); } catch {}
    part.menu = null;
    if (part.outside && typeof window !== "undefined") { try { window.removeEventListener("click", part.outside, true); } catch {} part.outside = null; }
  }

  // The IDM-style quality list. Options come from the adapter (or the background script for captured streams);
  // choosing one reports the outcome inside the same menu.
  async function openMenu(playerId) {
    const part = parts.get(playerId);
    const panel = panels.get(playerId);
    const doc = typeof document !== "undefined" ? document : null;
    if (!part || !panel || !doc || typeof doc.createElement !== "function") return null;
    closeMenu(playerId);
    const menu = doc.createElement("div");
    menu.className = "menu";
    menu.setAttribute?.("role", "menu");
    part.menu = menu;
    part.root.appendChild(menu);
    part.outside = event => { if (event?.target !== panel) closeMenu(playerId); };
    if (typeof window !== "undefined") window.addEventListener?.("click", part.outside, true);
    const text = (cls, value) => { const el = doc.createElement("div"); el.className = cls; el.textContent = value; return el; };
    const clear = () => { while (menu.children?.length) menu.removeChild(menu.children[0]); };
    clear();
    menu.appendChild(text("head", "Download this video"));
    const status = text("note", panel.mediaInfo?.kind === "page-video" ? "Getting formats…" : "Loading qualities…");
    menu.appendChild(status);
    const info = panel.mediaInfo || {};
    const draw = result => {
      clear();
      menu.appendChild(text("head", "Download this video"));
      if (!result.options.length) menu.appendChild(text("note", result.note || "Nothing to download."));
      if (result.footnote) menu.appendChild(text("note", result.footnote));
      const note = text("note", "");
      note.style.display = "none";
      for (const option of result.options) {
        const row = doc.createElement("button");
        row.className = "opt";
        row.type = "button";
        row.setAttribute?.("role", "menuitem");
        const label = doc.createElement("div");
        label.textContent = option.label;
        row.appendChild(label);
        if (option.detail) { const size = doc.createElement("span"); size.textContent = option.detail; row.appendChild(size); }
        row.onclick = async event => {
          event?.stopPropagation?.();
          event?.preventDefault?.();
          note.style.display = "block";
          note.textContent = "Sending to XDM…";
          let reply;
          try { reply = await option.run(); } catch (error) { reply = { accepted: false, reason: String(error?.message || error) }; }
          const accepted = reply?.accepted || reply?.ok === true;
          note.textContent = accepted ? "Added to XDM" : `Could not hand off: ${reply?.reason || "unknown error"}`;
          if (accepted) setTimeout(() => closeMenu(playerId), 1500);
        };
        menu.appendChild(row);
      }
      if (result.moreFormats) {
        const more = doc.createElement("button");
        more.className = "opt more";
        more.type = "button";
        more.setAttribute?.("role", "menuitem");
        const label = doc.createElement("div");
        label.textContent = "More formats…";
        more.appendChild(label);
        const hint = doc.createElement("span");
        hint.textContent = "yt-dlp";
        more.appendChild(hint);
        more.onclick = async event => {
          event?.stopPropagation?.();
          event?.preventDefault?.();
          clear();
          menu.appendChild(text("head", "Download this video"));
          menu.appendChild(text("note", "Getting formats…"));
          let more;
          try { more = await ytdlpMenu(info.title); } catch (error) { more = { options: [], note: `Could not get formats: ${error?.message || error}` }; }
          if (part.menu === menu) draw(more);
        };
        menu.appendChild(more);
      }
      menu.appendChild(note);
    };
    let result;
    try {
      result = info.kind === "page-video" ? await ytdlpMenu(info.title) : await genericMenu(info);
    } catch (error) {
      result = { options: [], note: `Could not list qualities: ${error?.message || error}` };
    }
    if (part.menu !== menu) return null;
    draw(result);
    return menu;
  }

  function isOwn(node) {
    for (let cur = node, depth = 0; cur && depth < 4; cur = cur.parentNode, depth++) if (owned.has(cur)) return true;
    return false;
  }

  function show(playerId) {
    const panel = panels.get(playerId);
    if (panel) {
      if (panel.style) setStyle(panel, "display", "block");
      panel.visible = true;
    }
  }

  function hide(playerId) {
    closeMenu(playerId);
    const panel = panels.get(playerId);
    if (panel) {
      if (panel.style) setStyle(panel, "display", "none");
      panel.visible = false;
    }
  }

  // Escape: hide this overlay until the player gets a new source.
  function dismiss(playerId) {
    const panel = panels.get(playerId);
    if (!panel) return;
    panel.dismissed = true;
    hide(playerId);
  }

  function reset(playerId) {
    const panel = panels.get(playerId);
    if (panel) panel.dismissed = false;
  }

  function bind(playerId, focusTarget) {
    const part = parts.get(playerId);
    if (part) part.focusTarget = focusTarget;
  }

  function updatePosition(playerId, rect) {
    const panel = panels.get(playerId);
    if (!panel || !rect) return;
    if (panel.style) {
      setStyle(panel, "top", `${Math.max(rect.top || 0, 0) + 4}px`);
      setStyle(panel, "left", `${Math.max((rect.right || 0) - width() - 4, 0)}px`);
      setStyle(panel, "width", `${width()}px`);
    }
  }

  // Shows the panel only while policy allows it and its player is connected, big enough and inside the viewport.
  function sync(playerId, element, intersecting) {
    const panel = panels.get(playerId);
    if (!panel) return false;
    if (overlayPolicy.disabled || panel.dismissed) { hide(playerId); return false; }
    const doc = typeof document !== "undefined" ? document : null;
    const connected = !!element && (doc?.contains ? doc.contains(element) : element.isConnected !== false);
    const rect = typeof element?.getBoundingClientRect === "function" ? element.getBoundingClientRect() : null;
    const big = !rect || (rect.width >= MIN_SIZE && rect.height >= MIN_SIZE);
    const vw = typeof window !== "undefined" && window.innerWidth ? window.innerWidth : Infinity;
    const vh = typeof window !== "undefined" && window.innerHeight ? window.innerHeight : Infinity;
    const inView = intersecting !== false && (!rect || (rect.bottom > 0 && rect.right > 0 && rect.top < vh && rect.left < vw));
    if (connected && big && inView) {
      updatePosition(playerId, rect);
      show(playerId);
      return true;
    }
    hide(playerId);
    return false;
  }

  function applyPolicy() {
    for (const [id, part] of parts) labelFor(part.btn, panels.get(id)?.mediaInfo);
    if (typeof playerRegistry !== "undefined") playerRegistry.repositionAll();
  }

  function remove(playerId) {
    closeMenu(playerId);
    const panel = panels.get(playerId);
    if (panel) {
      if (panel.parentNode && typeof panel.parentNode.removeChild === "function") {
        panel.parentNode.removeChild(panel);
      }
      panels.delete(playerId);
      parts.delete(playerId);
    }
  }

  function get(playerId) {
    return panels.get(playerId) || null;
  }

  // The closed shadow root and its button, for the content script's own use and tests.
  function internals(playerId) {
    return parts.get(playerId) || null;
  }

  return { create, attach, show, hide, dismiss, reset, bind, updatePosition, sync, applyPolicy, remove, get, internals, isOwn, openMenu, closeMenu, panels };
})();

// B44: Player registry with WeakRef tracking, per-player observers and removal cleanup.
var playerRegistry = (() => {
  let nextId = 1;
  const entries = new Map();
  const elementToId = typeof WeakMap !== "undefined" ? new WeakMap() : new Map();

  const deref = entry => typeof entry.ref?.deref === "function" ? entry.ref.deref() : entry.ref;

  function observe(element, entry) {
    if (typeof IntersectionObserver !== "undefined") {
      entry.io = new IntersectionObserver(records => {
        for (const r of records) if (r.target === element) playerPanel.sync(entry.id, element, r.isIntersecting);
      });
      entry.io.observe(element);
    }
    if (typeof ResizeObserver !== "undefined") {
      entry.ro = new ResizeObserver(() => playerPanel.sync(entry.id, element));
      entry.ro.observe(element);
    }
  }

  const isEncrypted = el => { try { return !!(el.mediaKeys || el.webkitKeys); } catch { return false; } };

  // DRM/EME media: no overlay and no download offer, ever.
  function markProtected(entry) {
    if (entry.protected) return;
    entry.protected = true;
    const el = deref(entry);
    try { entry.io?.disconnect(); } catch {}
    try { entry.ro?.disconnect(); } catch {}
    entry.io = entry.ro = null;
    playerPanel.remove(entry.id);
    const urls = new Set([entry.url, ...(el ? playerUrls(el) : [])].filter(Boolean));
    for (const url of urls) api?.runtime?.sendMessage({ type: "page.protected", url })?.catch?.(() => {});
  }

  function dispose(entry, notify) {
    entries.delete(entry.id);
    if (entry.drmHandler) {
      const el = deref(entry);
      for (const type of ["encrypted", "waitingforkey"]) { try { el?.removeEventListener?.(type, entry.drmHandler); } catch {} }
    }
    try { entry.io?.disconnect(); } catch {}
    try { entry.ro?.disconnect(); } catch {}
    entry.io = entry.ro = null;
    playerPanel.remove(entry.id);
    if (notify && entry.url) {
      api?.runtime?.sendMessage({ type: "page.mediaGone", url: entry.url })?.catch?.(() => {});
    }
  }

  function register(element, info = {}) {
    if (!element) return null;
    let id = elementToId.get(element);
    if (!id) {
      id = `player-${nextId++}`;
      try { elementToId.set(element, id); } catch {}
    }
    const existing = entries.get(id);
    if (existing) {
      if (existing.protected) return id;
      if (isEncrypted(element)) { markProtected(existing); return id; }
      // Same element with a new source keeps its identity, panel and observers.
      if (info.url && info.url !== existing.url) playerPanel.reset(id);
      existing.url = info.url || existing.url;
      existing.title = info.title || existing.title;
      if (info.kind) { existing.kind = info.kind; existing.videoId = info.videoId; existing.streamKind = info.streamKind; }
      playerPanel.sync(id, element);
      return id;
    }
    const entry = {
      id,
      ref: typeof WeakRef !== "undefined" ? new WeakRef(element) : element,
      url: info.url || element.currentSrc || element.src || "",
      title: info.title || element.title || "",
      kind: info.kind || null,
      videoId: info.videoId || null,
      streamKind: info.streamKind || null,
      registeredAt: Date.now(),
      io: null,
      ro: null
    };
    entries.set(id, entry);
    entry.drmHandler = () => markProtected(entry);
    for (const type of ["encrypted", "waitingforkey"]) { try { element.addEventListener?.(type, entry.drmHandler); } catch {} }
    if (isEncrypted(element)) { markProtected(entry); return id; }
    playerPanel.create(id, entry);
    playerPanel.bind(id, () => element.focus?.());
    playerPanel.attach(id);
    observe(element, entry);
    playerPanel.sync(id, element);
    return id;
  }

  function unregister(elementOrId, notify = true) {
    const id = typeof elementOrId === "string" ? elementOrId : elementToId.get(elementOrId);
    if (!id) return false;
    const entry = entries.get(id);
    if (!entry) return false;
    dispose(entry, notify);
    return true;
  }

  function isProtected(elementOrId) {
    const id = typeof elementOrId === "string" ? elementOrId : elementToId.get(elementOrId);
    return !!entries.get(id)?.protected;
  }

  function get(id) {
    const entry = entries.get(id);
    if (!entry) return null;
    const el = deref(entry);
    if (!el) {
      dispose(entry, false);
      return null;
    }
    return { ...entry, element: el };
  }

  function cleanup() {
    for (const entry of [...entries.values()]) {
      const el = deref(entry);
      const connected = el && (typeof document !== "undefined" && document.contains ? document.contains(el) : el.isConnected !== false);
      if (!el || !connected) dispose(entry, !!el);
    }
  }

  function repositionAll() {
    for (const entry of [...entries.values()]) {
      const el = deref(entry);
      if (!el) { dispose(entry, false); continue; }
      playerPanel.sync(entry.id, el);
    }
  }

  function disposeAll() {
    for (const entry of [...entries.values()]) dispose(entry, false);
  }

  function size() {
    return entries.size;
  }

  return { register, unregister, isProtected, get, cleanup, repositionAll, disposeAll, size, entries };
})();

// B47: Scroll/resize keep overlays aligned; navigation drops every overlay and observer.
if (typeof window !== "undefined" && window.addEventListener) {
  window.addEventListener("scroll", () => playerRegistry.repositionAll(), { capture: true, passive: true });
  window.addEventListener("resize", () => playerRegistry.repositionAll(), { passive: true });
  window.addEventListener("pagehide", () => playerRegistry.disposeAll());
  window.addEventListener("popstate", () => playerRegistry.cleanup());
  window.addEventListener("hashchange", () => playerRegistry.cleanup());
}

// B47: 200ms debounce timer for DOM inspection
var _debounceTimer = null;
const DEBOUNCE_MS = 200;

function debouncedInspect(node, delay = DEBOUNCE_MS) {
  if (_debounceTimer) clearTimeout(_debounceTimer);
  _debounceTimer = setTimeout(() => {
    _debounceTimer = null;
    inspect(node || (typeof document !== "undefined" ? document.documentElement : null));
    playerRegistry.cleanup();
  }, delay);
}

function inspect(node) {
  if (!(node instanceof Element)) return;
  const players = node.matches("video,audio")
    ? [node]
    : (node.querySelectorAll ? [...node.querySelectorAll("video"), ...node.querySelectorAll("audio")] : []);
  const pageUrl = typeof location !== "undefined" ? location.href || "" : "";

  for (const player of players) {
    if (typeof player.getBoundingClientRect === "function") {
      const rect = player.getBoundingClientRect();
      if (rect && rect.width <= 1 && rect.height <= 1) continue; // suppress 1x1 tracking pixel
    }
    const items = adapterMedia(player, pageUrl);
    // A blob: (MSE) player has no address of its own; look for a captured HLS/DASH manifest of this tab.
    if (!items.length && !playerUrls(player).some(u => /^https?:/i.test(u))) probeStreams(player);
    for (const item of items) {
      const url = item.url;
      if (isAd(url)) continue;
      const known = seen.has(url);
      // A replacement element showing an already reported URL still needs its own overlay.
      playerRegistry.register(player, { url, title: item.title, kind: item.kind, videoId: item.videoId });
      if (playerRegistry.isProtected(player)) continue;
      if (known) continue;
      seen.add(url);
      // A page video is not a downloadable address; the panel menu is its only route.
      if (item.kind === "page-video") continue;
      const expiresAt = urlExpiry(url);
      api?.runtime?.sendMessage({
        type: "page.media",
        url,
        mime: item.mime || player.tagName.toLowerCase() + "/*",
        title: item.title,
        adapterId: item.adapterId,
        adapterVersion: item.adapterVersion,
        expiresAt: expiresAt || undefined,
        expired: expiresAt ? expiresAt <= Date.now() : undefined
      })?.catch?.(() => {});
    }
  }
}

function removedPlayers(node) {
  if (!node || node.nodeType === 3) return [];
  const found = [];
  if (typeof node.matches === "function" && node.matches("video,audio")) found.push(node);
  if (typeof node.querySelectorAll === "function") found.push(...node.querySelectorAll("video,audio"));
  return found;
}

function handleMutations(records) {
  let hasRemovals = false;
  let relevant = false;
  for (const record of records) {
    // Our own overlay moving or restyling must not retrigger inspection.
    if (record.target && playerPanel.isOwn(record.target)) continue;
    relevant = true;
    for (const node of record.addedNodes || []) {
      if (!playerPanel.isOwn(node)) inspect(node);
    }
    if (record.removedNodes && record.removedNodes.length > 0) {
      hasRemovals = true;
      for (const node of record.removedNodes) {
        if (playerPanel.isOwn(node)) continue;
        for (const player of removedPlayers(node)) {
          const reattached = typeof document !== "undefined" && document.contains ? document.contains(player) : false;
          if (!reattached) {
            // Let the same URL be reported again if a replacement player appears.
            if (playerRegistry.unregister(player)) for (const u of playerUrls(player)) seen.delete(u);
          }
        }
      }
    }
  }
  if (hasRemovals) playerRegistry.cleanup();
  if (relevant) debouncedInspect(document.documentElement);
}

if (typeof document !== "undefined" && document.documentElement) {
  inspect(document.documentElement);
  if (typeof MutationObserver !== "undefined") {
    new MutationObserver(handleMutations).observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["src", "style", "class"]
    });
  }
}

if (api?.runtime?.onMessage?.addListener) {
  api.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
    if (msg?.type === "page.getDownloadAttribute") {
      const anchors = typeof document !== "undefined" && document.querySelectorAll ? [...document.querySelectorAll("a")] : [];
      const anchor = anchors.find(a => (a.href || a.attributes?.href) === msg.url);
      const filename = (anchor?.getAttribute ? anchor.getAttribute("download") : anchor?.attributes?.download) || anchor?.innerText || anchor?.textContent || undefined;
      sendResponse({ filename });
      return false;
    }
    if (msg?.type === "page.videoMenu") {
      const entry = [...playerRegistry.entries.values()].find(item => !item.protected);
      if (!entry) { sendResponse({ options: [] }); return false; }
      (entry.kind === "page-video" ? ytdlpMenu(entry.title) : genericMenu(entry)).then(result => {
        lastVideoMenu = result.options;
        sendResponse({ options: result.options.map((o, key) => ({ key, label: o.label, detail: o.detail })), note: result.note });
      }, () => sendResponse({ options: [] }));
      return true;
    }
    if (msg?.type === "page.videoChoose") {
      const option = lastVideoMenu[msg.key];
      if (!option) { sendResponse({ accepted: false, reason: "menu expired" }); return false; }
      option.run().then(sendResponse, error => sendResponse({ accepted: false, reason: String(error) }));
      return true;
    }
    if (msg?.type === "page.extractLinks") {
      sendResponse({ links: extractLinks(msg.options) });
      return false;
    }
    if (msg?.type === "page.queryMetadata") {
      sendResponse({ metadata: queryElementMetadata(msg.selector || msg.elementId, msg.properties) });
      return false;
    }
    if (msg?.type === "page.refreshMedia") {
      const adapter = getSiteAdapter(msg.url || "");
      if (!adapter) { sendResponse({ refreshed: false, expired: false, url: msg.url }); return false; }
      adapter.refreshUrl({ url: msg.url }).then(
        result => sendResponse({ ...result, adapterId: adapter.id, adapterVersion: adapter.version }),
        () => sendResponse({ url: msg.url, refreshed: false, expired: false })
      );
      return true;
    }
    return false;
  });
}

if (typeof globalThis !== "undefined") {
  globalThis.modifierState = modifierState;
  globalThis.keydownHandler = keydownHandler;
  globalThis.keyupHandler = keyupHandler;
  globalThis.resetModifiers = resetModifiers;
  globalThis.clickHandler = clickHandler;
  globalThis.captureModeFor = captureModeFor;
  globalThis.setCaptureKeys = setCaptureKeys;
  globalThis.extractLinks = extractLinks;
  Object.defineProperty(globalThis, "currentSelection", {
    get: () => _currentSelection,
    set: val => { _currentSelection = val; },
    configurable: true,
    enumerable: true
  });
  globalThis.selectionHandler = selectionHandler;
  globalThis.queryElementMetadata = queryElementMetadata;
  globalThis.siteAdapters = siteAdapters;
  globalThis.getSiteAdapter = getSiteAdapter;
  globalThis.enabledAdapters = enabledAdapters;
  globalThis.urlExpired = urlExpired;
  globalThis.playerRegistry = playerRegistry;
  Object.defineProperty(globalThis, "debounceTimer", {
    get: () => _debounceTimer,
    set: val => { _debounceTimer = val; },
    configurable: true,
    enumerable: true
  });
  globalThis.DEBOUNCE_MS = DEBOUNCE_MS;
  globalThis.debouncedInspect = debouncedInspect;
  globalThis.playerPanel = playerPanel;
  globalThis.setOverlayPolicy = setOverlayPolicy;
}
