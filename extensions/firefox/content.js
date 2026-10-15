const seen = new Set();
const api = typeof browser !== "undefined" ? browser : (typeof chrome !== "undefined" ? chrome : null);

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

// B48: Floating "Download with XDM" button attached to detected players.
var playerPanel = (() => {
  const panels = new Map();
  const owned = typeof WeakSet !== "undefined" ? new WeakSet() : new Set();
  const MIN_SIZE = 40;
  const PANEL_WIDTH = 144;

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
    panel.style = panel.style || {};
    panel.style.position = "fixed";
    panel.style.zIndex = "2147483647";
    panel.style.display = "none";
    panel.style.pointerEvents = "auto";
    panel.mediaInfo = mediaInfo;
    owned.add(panel);

    const btn = doc.createElement("button");
    btn.className = "xdm-download-btn";
    btn.textContent = "Download with XDM";
    btn.type = "button";
    // Reads the live entry so a replaced source is never downloaded from a stale URL.
    btn.onclick = event => {
      event?.stopPropagation?.();
      event?.preventDefault?.();
      const info = panel.mediaInfo || {};
      if (info.url) {
        api?.runtime?.sendMessage({ type: "media.download", url: info.url, title: info.title, pageUrl: typeof location !== "undefined" ? location.href : "" })?.catch?.(() => {});
      }
    };
    owned.add(btn);
    panel.appendChild(btn);

    panels.set(playerId, panel);
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

  function isOwn(node) {
    for (let cur = node, depth = 0; cur && depth < 4; cur = cur.parentNode, depth++) if (owned.has(cur)) return true;
    return false;
  }

  function show(playerId) {
    const panel = panels.get(playerId);
    if (panel) {
      if (panel.style) panel.style.display = "block";
      panel.visible = true;
    }
  }

  function hide(playerId) {
    const panel = panels.get(playerId);
    if (panel) {
      if (panel.style) panel.style.display = "none";
      panel.visible = false;
    }
  }

  function updatePosition(playerId, rect) {
    const panel = panels.get(playerId);
    if (!panel || !rect) return;
    if (panel.style) {
      panel.style.top = `${Math.max(rect.top || 0, 0) + 4}px`;
      panel.style.left = `${Math.max((rect.right || 0) - PANEL_WIDTH - 4, 0)}px`;
    }
  }

  // Shows the panel only while its player is connected, big enough and inside the viewport.
  function sync(playerId, element, intersecting) {
    const panel = panels.get(playerId);
    if (!panel) return false;
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

  function remove(playerId) {
    const panel = panels.get(playerId);
    if (panel) {
      if (panel.parentNode && typeof panel.parentNode.removeChild === "function") {
        panel.parentNode.removeChild(panel);
      }
      panels.delete(playerId);
    }
  }

  function get(playerId) {
    return panels.get(playerId) || null;
  }

  return { create, attach, show, hide, updatePosition, sync, remove, get, isOwn, panels };
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

  function dispose(entry, notify) {
    entries.delete(entry.id);
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
      // Same element with a new source keeps its identity, panel and observers.
      existing.url = info.url || existing.url;
      existing.title = info.title || existing.title;
      playerPanel.sync(id, element);
      return id;
    }
    const entry = {
      id,
      ref: typeof WeakRef !== "undefined" ? new WeakRef(element) : element,
      url: info.url || element.currentSrc || element.src || "",
      title: info.title || element.title || "",
      registeredAt: Date.now(),
      io: null,
      ro: null
    };
    entries.set(id, entry);
    playerPanel.create(id, entry);
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

  return { register, unregister, get, cleanup, repositionAll, disposeAll, size, entries };
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
    for (const item of adapterMedia(player, pageUrl)) {
      const url = item.url;
      if (isAd(url)) continue;
      const known = seen.has(url);
      // A replacement element showing an already reported URL still needs its own overlay.
      playerRegistry.register(player, { url, title: item.title });
      if (known) continue;
      seen.add(url);
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
}
