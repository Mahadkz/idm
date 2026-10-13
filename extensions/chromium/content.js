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
  const extracted = extractLinks({ selectedText: text, selectionOnly: true });
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

// B42: Site adapters for dynamic players
var siteAdapters = [
  {
    id: "generic-html5",
    version: 1,
    name: "Generic HTML5 Media Adapter",
    matches: url => /^https?:\/\//i.test(url),
    extractMedia: node => {
      if (!node || !(node instanceof Element)) return [];
      const players = node.matches("video,audio") ? [node] : (node.querySelectorAll ? [...node.querySelectorAll("video,audio")] : []);
      return players.map(p => ({
        url: p.currentSrc || p.src,
        mime: (p.tagName || "video").toLowerCase() + "/*",
        title: p.title || (typeof document !== "undefined" ? document.title : ""),
      })).filter(item => item.url && /^https?:/i.test(item.url));
    },
    refreshUrl: async mediaItem => {
      return { url: mediaItem?.url, expired: false };
    }
  },
  {
    id: "embedded-stream",
    version: 1,
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
    refreshUrl: async mediaItem => {
      return { url: mediaItem?.url, expired: false };
    }
  }
];

function getSiteAdapter(url) {
  return siteAdapters.find(a => a.matches(url)) || siteAdapters[0];
}

// B48: Floating video download panel overlay
var playerPanel = (() => {
  const panels = new Map();

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
    panel.style.position = "absolute";
    panel.style.zIndex = "2147483647";
    panel.style.display = "block";

    const btn = doc.createElement("button");
    btn.className = "xdm-download-btn";
    btn.textContent = "Download with XDM";
    btn.onclick = () => {
      if (mediaInfo.url) {
        api?.runtime?.sendMessage({
          type: "media.download",
          url: mediaInfo.url,
          title: mediaInfo.title
        })?.catch(() => {});
      }
    };
    panel.appendChild(btn);

    panels.set(playerId, panel);
    return panel;
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
      panel.style.top = `${rect.top || 0}px`;
      panel.style.left = `${rect.right ? rect.right - 140 : 0}px`;
    }
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

  return { create, show, hide, updatePosition, remove, get, panels };
})();

// B44: Player registry with WeakRef tracking and removal cleanup
var playerRegistry = (() => {
  let nextId = 1;
  const entries = new Map();
  const elementToId = typeof WeakMap !== "undefined" ? new WeakMap() : new Map();

  function register(element, info = {}) {
    if (!element) return null;
    let id = elementToId.get(element);
    if (!id) {
      id = `player-${nextId++}`;
      try { elementToId.set(element, id); } catch {}
    }
    const ref = typeof WeakRef !== "undefined" ? new WeakRef(element) : element;
    const entry = {
      id,
      ref,
      url: info.url || element.currentSrc || element.src || "",
      title: info.title || element.title || "",
      registeredAt: Date.now()
    };
    entries.set(id, entry);
    playerPanel.create(id, entry);
    return id;
  }

  function unregister(elementOrId) {
    let id = typeof elementOrId === "string" ? elementOrId : elementToId.get(elementOrId);
    if (!id) return false;
    const entry = entries.get(id);
    if (entry) {
      entries.delete(id);
      playerPanel.remove(id);
      return true;
    }
    return false;
  }

  function get(id) {
    const entry = entries.get(id);
    if (!entry) return null;
    const el = typeof entry.ref?.deref === "function" ? entry.ref.deref() : entry.ref;
    if (!el) {
      entries.delete(id);
      playerPanel.remove(id);
      return null;
    }
    return { ...entry, element: el };
  }

  function cleanup() {
    for (const [id, entry] of entries) {
      const el = typeof entry.ref?.deref === "function" ? entry.ref.deref() : entry.ref;
      const isConnected = el && (el.isConnected !== undefined ? el.isConnected : (typeof document !== "undefined" && document.contains ? document.contains(el) : true));
      if (!el || !isConnected) {
        entries.delete(id);
        playerPanel.remove(id);
      }
    }
  }

  function size() {
    return entries.size;
  }

  return { register, unregister, get, cleanup, size, entries };
})();

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

  for (const player of players) {
    if (typeof player.getBoundingClientRect === "function") {
      const rect = player.getBoundingClientRect();
      if (rect && rect.width <= 1 && rect.height <= 1) continue; // suppress 1x1 tracking pixel
    }
    const sources = player.querySelectorAll ? [...player.querySelectorAll("source")].map(s => s.src) : [];
    const urls = [player.currentSrc, player.src, ...sources].filter(Boolean);
    for (const url of urls) {
      if (!/^https?:/i.test(url) || seen.has(url) || isAd(url)) continue;
      seen.add(url);
      const title = player.title || (typeof document !== "undefined" ? document.title : "");
      playerRegistry.register(player, { url, title });
      api?.runtime?.sendMessage({
        type: "page.media",
        url,
        mime: player.tagName.toLowerCase() + "/*",
        title
      })?.catch(() => {});
    }
  }
}

if (typeof document !== "undefined" && document.documentElement) {
  inspect(document.documentElement);
  if (typeof MutationObserver !== "undefined") {
    new MutationObserver(records => {
      let hasRemovals = false;
      for (const record of records) {
        if (record.addedNodes) {
          for (const node of record.addedNodes) {
            inspect(node);
          }
        }
        if (record.removedNodes && record.removedNodes.length > 0) {
          hasRemovals = true;
          for (const node of record.removedNodes) {
            playerRegistry.unregister(node);
          }
        }
      }
      if (hasRemovals) {
        playerRegistry.cleanup();
      }
      debouncedInspect(document.documentElement);
    }).observe(document.documentElement, {
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
    return false;
  });
}

if (typeof globalThis !== "undefined") {
  globalThis.modifierState = modifierState;
  globalThis.keydownHandler = keydownHandler;
  globalThis.keyupHandler = keyupHandler;
  globalThis.resetModifiers = resetModifiers;
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
