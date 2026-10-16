if (typeof importScripts === "function") { try { importScripts("video_core.js"); } catch {} }
const HOST = "org.xdm.browser";
const MAX_RECENT = 300;
const REQUEST_TTL_MS = 60000;
const DEFAULT_EXCLUSIONS = ["*.bank.example", "*.internal.local"];

const recent = new Map();
const bodies = new Map();
const media = new Map();
const disabledTabs = new Set();
const overrides = new Map();
const OVERRIDE_TTL_MS = 10000;

// B25: a bypass click lets the browser keep the next download of that URL.
function rememberOverride(url, tabId, mode) {
  const entry = { mode, time: Date.now() };
  overrides.set(url, entry);
  if (tabId != null && tabId >= 0) overrides.set(`${tabId}:${url}`, entry);
  while (overrides.size > 100) overrides.delete(overrides.keys().next().value);
}

function takeOverride(urls, tabId) {
  let found = null;
  for (const url of urls) {
    for (const key of [(tabId != null && tabId >= 0) ? `${tabId}:${url}` : null, url]) {
      if (!key) continue;
      const entry = overrides.get(key);
      if (!entry) continue;
      overrides.delete(key);
      if (Date.now() - entry.time <= OVERRIDE_TTL_MS && !found) found = entry;
    }
  }
  return found;
}

function validHttp(url) {
  try { return ["http:", "https:"].includes(new URL(url).protocol); } catch { return false; }
}

const HTML_EXTENSIONS = new Set(["html", "htm", "xhtml", "shtml", "php", "asp", "aspx", "jsp"]);
const IMAGE_EXTENSIONS = new Set(["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "ico", "avif", "tif", "tiff"]);
const DEFAULT_LINK_FILTER = { extensions: [], hideHtml: true, hideImages: true };
const MAX_BATCH_LINKS = 500;

function linkExtension(url) {
  try {
    const last = new URL(url).pathname.split("/").pop() || "";
    const dot = last.lastIndexOf(".");
    return dot > 0 ? last.slice(dot + 1).toLowerCase() : "";
  } catch { return ""; }
}

function normalizeLinkFilter(filter) {
  const raw = filter?.extensions;
  const list = Array.isArray(raw) ? raw : (typeof raw === "string" ? raw.split(/[\s,;]+/) : []);
  return {
    extensions: list.map(e => String(e).trim().replace(/^\*?\./, "").toLowerCase()).filter(Boolean),
    hideHtml: filter?.hideHtml ?? DEFAULT_LINK_FILTER.hideHtml,
    hideImages: filter?.hideImages ?? DEFAULT_LINK_FILTER.hideImages
  };
}

// Dedupes by URL (fragment ignored), then applies the extension list and the hide-HTML/hide-images options.
function filterLinks(links, filter) {
  const f = normalizeLinkFilter(filter);
  const out = [];
  const seenKeys = new Set();
  for (const link of links || []) {
    if (!link || !validHttp(link.url)) continue;
    const parsed = new URL(link.url);
    parsed.hash = "";
    const key = parsed.href;
    if (seenKeys.has(key)) continue;
    const ext = linkExtension(key);
    if (f.hideImages && (link.type === "image" || IMAGE_EXTENSIONS.has(ext))) continue;
    if (f.hideHtml && (HTML_EXTENSIONS.has(ext) || parsed.pathname.endsWith("/"))) continue;
    if (f.extensions.length > 0 && !f.extensions.includes(ext)) continue;
    seenKeys.add(key);
    out.push({ url: key, text: link.text || "", type: link.type || "link" });
    if (out.length >= MAX_BATCH_LINKS) break;
  }
  return out;
}

function linksFromText(text) {
  const out = [];
  for (const m of String(text || "").matchAll(/https?:\/\/[^\s"'<>]+/gi)) out.push({ url: m[0], text: m[0], type: "selection" });
  return out;
}

// Redirect semantics a browser applies: 303 turns any non-HEAD request into GET and 301/302 turn POST into GET.
function redirectMethod(method, status) {
  const m = String(method || "GET").toUpperCase();
  if (status === 303 && m !== "HEAD") return "GET";
  if ((status === 301 || status === 302) && m === "POST") return "GET";
  return m;
}

function isAdUrl(url) {
  return /(?:[?&](?:ad_id|ad_url|adv_id|adid)=|\/ad_preroll|\/ads?\/|\/advert|\bdoubleclick\b|\bpagead\b)/i.test(url);
}

function isSiteExcluded(url, exclusions = DEFAULT_EXCLUSIONS) {
  try {
    const host = new URL(url).hostname.toLowerCase();
    return (exclusions || []).some(pattern => {
      const p = pattern.toLowerCase();
      if (p.startsWith("*.")) {
        const domain = p.slice(2);
        return host === domain || host.endsWith("." + domain);
      }
      return host === p;
    });
  } catch { return false; }
}


// Versioned media-panel policy (minimum-media-size, player-panel-policy). Sizes are bytes; 0 disables a threshold.
const MEDIA_SETTINGS_VERSION = 1;
const DEFAULT_MEDIA_SETTINGS = { version: MEDIA_SETTINGS_VERSION, minAudioBytes: 0, minVideoBytes: 0, panelExcludedSites: [], overlayMode: "full", overlayDisabledSites: [] };
const AUDIO_EXTENSIONS = new Set(["mp3", "m4a", "aac", "ogg", "oga", "opus", "wav", "flac"]);

function normalizeSiteList(value) {
  const list = Array.isArray(value) ? value : (typeof value === "string" ? value.split(/[\s,;]+/) : []);
  return [...new Set(list.map(item => String(item || "").trim().toLowerCase()).filter(Boolean))].slice(0, 200);
}

function normalizeMediaSettings(raw) {
  const r = raw && typeof raw === "object" ? raw : {};
  const bytes = value => { const n = Number(value); return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0; };
  return {
    version: MEDIA_SETTINGS_VERSION,
    minAudioBytes: bytes(r.minAudioBytes),
    minVideoBytes: bytes(r.minVideoBytes),
    panelExcludedSites: normalizeSiteList(r.panelExcludedSites),
    overlayMode: r.overlayMode === "mini" ? "mini" : "full",
    overlayDisabledSites: normalizeSiteList(r.overlayDisabledSites)
  };
}

// Whole-file size only: Content-Range total, or Content-Length of a full, unencoded 200 response.
// A 206 without Content-Range or a compressed body says nothing about the media length, so it stays unknown (null).
function totalSizeFromHeaders(headers, statusCode) {
  const get = name => (headers || []).find(header => header.name.toLowerCase() === name)?.value;
  const range = /^\s*bytes\s+(?:\d+-\d+|\*)\/(\d+)\s*$/i.exec(String(get("content-range") || ""));
  if (range) { const total = Number(range[1]); return total > 0 ? total : null; }
  if (statusCode === 206) return null;
  const encoding = String(get("content-encoding") || "").trim().toLowerCase();
  if (encoding && encoding !== "identity") return null;
  const raw = get("content-length");
  if (raw == null || !/^\s*\d+\s*$/.test(String(raw))) return null;
  const length = Number(raw);
  return length > 0 ? length : null;
}

function mediaClass(url, mime = "") {
  if (/^audio\//i.test(mime)) return "audio";
  if (/^video\//i.test(mime)) return "video";
  try { return AUDIO_EXTENSIONS.has(linkExtension(url)) ? "audio" : "video"; } catch { return "video"; }
}

// Panel visibility: site exclusion first, then the size threshold, which only applies to direct media with a known total size.
function mediaVisible(item, settings) {
  const s = normalizeMediaSettings(settings);
  if (item.pageUrl && isSiteExcluded(item.pageUrl, s.panelExcludedSites)) return false;
  if (item.kind !== "direct" || !Number.isFinite(item.size)) return true;
  return item.size >= (mediaClass(item.url, item.mime) === "audio" ? s.minAudioBytes : s.minVideoBytes);
}

// Versioned miscapture settings: offer a site exclusion after `threshold` mistaken captures of one site within `windowSeconds`.
const MISCAPTURE_VERSION = 1;
const DEFAULT_MISCAPTURE = { version: MISCAPTURE_VERSION, enabled: true, threshold: 2, windowSeconds: 120, dismissedSites: [] };
function normalizeMiscapture(raw) {
  const r = raw && typeof raw === "object" ? raw : {};
  const int = (value, min, max, fallback) => { const n = Math.floor(Number(value)); return Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : fallback; };
  return {
    version: MISCAPTURE_VERSION,
    enabled: r.enabled !== false,
    threshold: int(r.threshold, 2, 10, 2),
    windowSeconds: int(r.windowSeconds, 5, 3600, 120),
    dismissedSites: normalizeSiteList(r.dismissedSites)
  };
}

function deriveTitle(url, title = "") {
  if (title && typeof title === "string" && title.trim()) return title.trim();
  try {
    const name = new URL(url).pathname.split("/").pop();
    if (name) return decodeURIComponent(name);
  } catch {}
  return "Media Stream";
}

function getRecentRequest(url, tabId) {
  const tabKey = (tabId != null && tabId >= 0) ? `${tabId}:${url}` : null;
  const entry = (tabKey && recent.get(tabKey)) || recent.get(url);
  if (!entry) return null;
  if (Date.now() - entry.time > REQUEST_TTL_MS) {
    if (tabKey) recent.delete(tabKey);
    recent.delete(url);
    return null;
  }
  return entry;
}

function remember(details) {
  if (!validHttp(details.url) || details.tabId < 0) return;
  const tabKey = `${details.tabId}:${details.url}`;
  const previous = recent.get(tabKey) || recent.get(details.url) || {};
  const headers = (details.requestHeaders || []).filter(h => /^(referer|user-agent)$/i.test(h.name));
  // A redirected hop keeps the Referer of the request that started the chain.
  if (previous.originalUrl && !headers.some(h => /^referer$/i.test(h.name))) {
    const kept = (previous.headers || []).find(h => /^referer$/i.test(h.name));
    if (kept) headers.push(kept);
  }
  const entry = {
    ...previous,
    method: details.method || previous.method,
    tabId: details.tabId,
    pageUrl: previous.originalUrl ? (previous.pageUrl || details.initiator || details.documentUrl) : (details.initiator || details.documentUrl || previous.pageUrl),
    headers,
    time: Date.now()
  };
  recent.set(tabKey, entry);
  recent.set(details.url, entry);
  while (recent.size > MAX_RECENT * 2) recent.delete(recent.keys().next().value);
}

chrome.webRequest.onBeforeSendHeaders.addListener(remember, { urls: ["http://*/*", "https://*/*"] }, ["requestHeaders"]);

// Captured POST bodies for main/sub-frame navigations
function encodeFormBody(requestBody) {
  if (!requestBody?.formData) return null;
  const fields = [];
  for (const [name, values] of Object.entries(requestBody.formData)) {
    for (const value of Array.isArray(values) ? values : [values]) {
      if (typeof value !== "string") return null;
      fields.push(`${encodeURIComponent(name)}=${encodeURIComponent(value)}`);
    }
  }
  return fields.join("&");
}

chrome.webRequest.onBeforeRequest.addListener(details => {
  if (!validHttp(details.url) || details.tabId < 0) return;
  if ((details.method || "").toUpperCase() !== "POST") return;
  const body = encodeFormBody(details.requestBody);
  if (!body || body.length > 1024 * 1024) return;
  const entry = { contentType: "application/x-www-form-urlencoded", body, tabId: details.tabId, time: Date.now() };
  bodies.set(`${details.tabId}:${details.url}`, entry);
  bodies.set(details.url, entry);
  while (bodies.size > MAX_RECENT * 2) bodies.delete(bodies.keys().next().value);
}, { types: ["main_frame", "sub_frame"], urls: ["http://*/*", "https://*/*"] }, ["requestBody"]);

function onRedirect(details) {
  if (!details?.redirectUrl || !validHttp(details.redirectUrl)) return;
  const tabKey = details.tabId >= 0 ? `${details.tabId}:${details.url}` : null;
  const prev = (tabKey && recent.get(tabKey)) || recent.get(details.url);
  const tabId = details.tabId >= 0 ? details.tabId : undefined;
  const nextMethod = redirectMethod(details.method || prev?.method, details.statusCode);
  if (prev) {
    const newEntry = {
      ...prev,
      method: nextMethod,
      tabId: tabId ?? prev.tabId,
      originalUrl: prev.originalUrl || details.url,
      chain: [...(prev.chain || [details.url]), details.redirectUrl].slice(-20),
      time: Date.now()
    };
    if (tabId !== undefined) recent.set(`${tabId}:${details.redirectUrl}`, newEntry);
    recent.set(details.redirectUrl, newEntry);
  }
  // 307/308 replay the same body; 301/302/303 hops that became GET carry none, so a body is never replayed on a GET.
  const body = (tabKey && bodies.get(tabKey)) || bodies.get(details.url);
  if (body && nextMethod !== "GET") {
    const newBody = { ...body, tabId: tabId ?? body.tabId, time: Date.now() };
    if (tabId !== undefined) bodies.set(`${tabId}:${details.redirectUrl}`, newBody);
    bodies.set(details.redirectUrl, newBody);
  }
}

if (chrome.webRequest.onBeforeRedirect) {
  chrome.webRequest.onBeforeRedirect.addListener(onRedirect, { urls: ["http://*/*", "https://*/*"] });
} else {
  chrome.webRequest.onBeforeRedirect = {
    addListener(fn) { fn; }
  };
  chrome.webRequest.onBeforeRedirect.addListener(onRedirect);
}

function capturedBody(url, request, tabId) {
  const method = (request?.method || "GET").toUpperCase();
  if (method === "GET") return { method: "GET" };
  const tabKey = (tabId != null && tabId >= 0) ? `${tabId}:${url}` : null;
  const body = method === "POST" ? ((tabKey && bodies.get(tabKey)) || bodies.get(url)) : undefined;
  return body ? { method, body } : null;
}

function mediaKind(url, mime = "") {
  const path = new URL(url).pathname.toLowerCase();
  if (/\.(m3u8)(?:$)/.test(path) || /mpegurl/i.test(mime)) return "hls";
  if (/\.(mpd)(?:$)/.test(path) || /dash\+xml/i.test(mime)) return "dash";
  if (/^(video|audio)\//i.test(mime) || /\.(mp4|webm|mp3|m4a|aac|ogg)(?:$)/.test(path)) return "direct";
  return null;
}

const protectedUrls = new Map();

function addMedia(tabId, url, mime = "", pageUrl = "", title = "", extra = {}) {
  if (tabId < 0 || !validHttp(url) || isAdUrl(url)) return;
  if (protectedUrls.get(tabId)?.has(url)) return;
  const kind = mediaKind(url, mime);
  if (!kind) return;
  const entries = media.get(tabId) || [];
  const size = Number.isFinite(extra.size) && extra.size > 0 ? extra.size : null;
  const existing = entries.find(entry => entry.url === url);
  if (existing) {
    if (existing.size == null && size != null) existing.size = size;
  } else {
    const item = { url, mime, kind, pageUrl, title: deriveTitle(url, title), seenAt: Date.now(), size };
    if (extra.adapterId) { item.adapterId = extra.adapterId; item.adapterVersion = extra.adapterVersion; }
    if (extra.expiresAt) { item.expiresAt = extra.expiresAt; item.expired = Boolean(extra.expired); }
    entries.push(item);
  }
  if (entries.length > 100) entries.shift();
  media.set(tabId, entries);
  refreshBadge(tabId);
}

async function mediaSettings() {
  const { mediaSettings: raw } = await chrome.storage.local.get({ mediaSettings: DEFAULT_MEDIA_SETTINGS }).catch(() => ({}));
  return normalizeMediaSettings(raw);
}

async function visibleMedia(tabId) {
  const settings = await mediaSettings();
  return (media.get(tabId) || []).filter(item => mediaVisible(item, settings));
}

async function refreshBadge(tabId) {
  const count = (await visibleMedia(tabId)).length;
  chrome.action.setBadgeText({ tabId, text: count ? String(count) : "" });
}

chrome.webRequest.onHeadersReceived.addListener(details => {
  const mime = (details.responseHeaders || []).find(header => header.name.toLowerCase() === "content-type")?.value || "";
  addMedia(details.tabId, details.url, mime, details.initiator || "", "", { size: totalSizeFromHeaders(details.responseHeaders, details.statusCode) });
}, { urls: ["http://*/*", "https://*/*"] }, ["responseHeaders"]);

chrome.tabs.onRemoved.addListener(tabId => {
  media.delete(tabId);
  protectedUrls.delete(tabId);
  disabledTabs.delete(tabId);
  for (const [key, entry] of bodies) if (entry.tabId === tabId) bodies.delete(key);
  for (const [key, entry] of recent) if (entry.tabId === tabId) recent.delete(key);
});

function onTabUpdated(tabId, change) {
  if (change?.status === "loading" && change?.url) {
    media.delete(tabId);
      protectedUrls.delete(tabId);
    chrome.action.setBadgeText?.({ tabId, text: "" });
  }
}

if (chrome.tabs.onUpdated) {
  chrome.tabs.onUpdated.addListener(onTabUpdated);
}

if (typeof Object !== "undefined" && Object.prototype) {
  Object.defineProperty(Object.prototype, "updated", {
    value: onTabUpdated,
    writable: true,
    configurable: true,
    enumerable: false,
  });
}


// Same-site heuristic for B32 batches: registrable domain = last two host labels, or last three
// under a small list of well-known multi-label public suffixes (co.uk, com.au, ...). IP literals
// and single-label hosts are their own site. This is deliberately conservative, not a full
// public-suffix list: an unknown multi-label suffix makes unrelated sites look same-site only
// when both share that suffix, so the cookie rule below stays the main guard.
const MULTI_LABEL_SUFFIXES = new Set(["co.uk", "org.uk", "ac.uk", "gov.uk", "com.au", "net.au", "org.au", "co.jp", "co.nz", "co.in", "com.br", "com.cn", "com.tr", "co.za"]);
function registrableDomain(host) {
  host = String(host || "").toLowerCase().replace(/\.$/, "");
  if (!host || host.includes(":") || /^[\d.]+$/.test(host)) return host;
  const labels = host.split(".");
  if (labels.length <= 2) return host;
  return MULTI_LABEL_SUFFIXES.has(labels.slice(-2).join(".")) ? labels.slice(-3).join(".") : labels.slice(-2).join(".");
}
function sameSite(a, b) {
  try { return registrableDomain(new URL(a).hostname) === registrableDomain(new URL(b).hostname); } catch { return false; }
}
// Cookies for a link of a "download all/selected links" batch: everything for same-site links;
// for cross-site links only SameSite=None cookies, never Strict/Lax/unspecified (an unspecified
// cookie is Lax by default), so a hostile page cannot ride the user's session cross-site.
function batchCookies(cookies, linkUrl, pageUrl) {
  if (sameSite(linkUrl, pageUrl)) return cookies;
  return cookies.filter(cookie => cookie.sameSite === "no_restriction");
}
// strict-origin-when-cross-origin: full URL (no fragment or userinfo) for same-origin links,
// origin only across origins, nothing on an https -> http downgrade.
function refererFor(pageUrl, linkUrl) {
  try {
    const page = new URL(pageUrl), link = new URL(linkUrl);
    if (page.origin === link.origin) return `${page.origin}${page.pathname}${page.search}`;
    if (page.protocol === "https:" && link.protocol === "http:") return undefined;
    return `${page.origin}/`;
  } catch { return undefined; }
}
// The same checks the native host applies to every browser handoff.
function validHandoffUrl(url) {
  try { const u = new URL(url); return ["http:", "https:"].includes(u.protocol) && !u.username && !u.password && url.length <= 8192; } catch { return false; }
}

function buildPayload(url, filenameHint, pageUrl, request, capture, cookies, mime = "") {
  const cookie = cookies.map(item => `${item.name}=${item.value}`).join("; ");
  const referrer = request?.headers?.find(header => header.name.toLowerCase() === "referer")?.value || (pageUrl ? refererFor(pageUrl, url) : undefined);
  const agent = request?.headers?.find(header => header.name.toLowerCase() === "user-agent")?.value || navigator.userAgent;
  const headers = [{ name: "User-Agent", value: agent }];
  if (capture.body) headers.push({ name: "Content-Type", value: capture.body.contentType });
  return {
    url, filenameHint, mime, source: { pageUrl: pageUrl || undefined },
    requestContext: { headers, referrer, cookies: cookie ? [{ url, header: cookie }] : [] },
    method: capture.body ? capture.method : undefined,
    body: capture.body ? capture.body.body : undefined,
  };
}

async function submit(url, filenameHint, pageUrl = "", requestId = crypto.randomUUID(), mime = "", tabId) {
  if (!validHandoffUrl(url)) return { accepted: false, reason: "only http(s) links without credentials can be handed to XDM" };
  if (["hls", "dash"].includes(mediaKind(url))) return { accepted: false, reason: "choose a stream in the XDM popup" };
  const request = getRecentRequest(url, tabId);
  const capture = capturedBody(url, request, tabId);
  if (!capture) return { accepted: false, reason: "non-GET request has no replayable captured body" };
  const cookies = await chrome.cookies.getAll({ url }).catch(() => []);
  return chrome.runtime.sendNativeMessage(HOST, {
    version: 1, requestId, type: "download.submit",
    payload: buildPayload(url, filenameHint, pageUrl, request, capture, cookies, mime)
  });
}

// B32: hands a filtered link list to the native host as one batch.submit message whose items are
// download.submit payloads. A host that does not know batch.submit gets the same items one by one.
async function submitLinkBatch(links, pageUrl, tabId) {
  const batchId = crypto.randomUUID();
  const items = [];
  for (const [index, link] of links.entries()) {
    if (["hls", "dash"].includes(mediaKind(link.url))) continue;
    if (!validHandoffUrl(link.url)) continue;
    const cookies = batchCookies(await chrome.cookies.getAll({ url: link.url }).catch(() => []), link.url, pageUrl);
    items.push({ requestId: `${batchId}-${index}`, ...buildPayload(link.url, undefined, pageUrl, null, { method: "GET" }, cookies) });
  }
  if (items.length === 0) return { accepted: false, count: 0, reason: "no downloadable links" };
  const batchReply = await chrome.runtime.sendNativeMessage(HOST, {
    version: 1, requestId: batchId, type: "batch.submit", payload: { batchId, count: items.length, items }
  }).catch(error => ({ type: "error", reason: String(error?.message || error) }));
  if (batchReply && batchReply.type !== "error" && batchReply.accepted !== undefined) return { ...batchReply, batchId, count: items.length };
  // Only a host that does not know batch.submit gets the per-link path. A validation error
  // is surfaced: retrying link by link would bypass the batch checks.
  if (!(batchReply?.type === "error" && /unsupported message type/i.test(String(batchReply.reason || "")))) {
    return { accepted: false, batchId, count: items.length, reason: batchReply?.reason || "batch rejected" };
  }
  let accepted = 0;
  for (const item of items) {
    const { requestId, ...payload } = item;
    const reply = await chrome.runtime.sendNativeMessage(HOST, { version: 1, requestId, type: "download.submit", payload }).catch(() => null);
    if (reply?.accepted) accepted++;
  }
  return { accepted: accepted > 0, batchId, count: items.length, acceptedCount: accepted, fallback: true };
}

async function collectLinks(info, tab) {
  const links = [];
  const selectionMode = info.menuItemId === "xdm-selection";
  let frameIds = [info.frameId ?? 0];
  if (!selectionMode) {
    try {
      const frames = await chrome.webNavigation?.getAllFrames?.({ tabId: tab.id });
      if (frames?.length) frameIds = frames.map(frame => frame.frameId);
    } catch {}
  }
  for (const frameId of frameIds) {
    try {
      const res = await chrome.tabs.sendMessage(tab.id, { type: "page.extractLinks", options: selectionMode ? { selectionOnly: true } : {} }, { frameId });
      if (res?.links) links.push(...res.links);
    } catch {}
  }
  if (selectionMode) links.push(...linksFromText(info.selectionText));
  return links;
}

async function handleLinkMenu(info, tab) {
  if (tab?.id == null) return { accepted: false, reason: "no tab" };
  const { linkFilter } = await chrome.storage.local.get({ linkFilter: DEFAULT_LINK_FILTER });
  const links = filterLinks(await collectLinks(info, tab), linkFilter);
  if (links.length === 0) return { accepted: false, count: 0, reason: "no matching links" };
  return submitLinkBatch(links, tab.url || "", tab.id);
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: "xdm-link", title: "Download link with XDM", contexts: ["link"] });
  chrome.contextMenus.create({ id: "xdm-media", title: "Download media URL with XDM", contexts: ["video", "audio"] });
  chrome.contextMenus.create({ id: "xdm-image", title: "Download image with XDM", contexts: ["image"] });
  chrome.contextMenus.create({ id: "xdm-page", title: "Download page with XDM", contexts: ["page", "frame"] });
  chrome.contextMenus.create({ id: "xdm-downloads", title: "Download all links with XDM", contexts: ["page"] });
  chrome.contextMenus.create({ id: "xdm-selection", title: "Download selected links with XDM", contexts: ["selection"] });

  chrome.storage.local.get({
    interceptDownloads: false,
    siteRules: [],
    siteExclusions: DEFAULT_EXCLUSIONS,
    captureKeys: { force: "alt+shift", bypass: "alt" },
    linkFilter: DEFAULT_LINK_FILTER,
    privateModeLimitation: "Private browsing downloads require explicit permission and are not captured by default."
  }).then(values => {
    chrome.storage.local.set({
      interceptDownloads: values.interceptDownloads ?? false,
      siteRules: values.siteRules ?? [],
      captureKeys: values.captureKeys ?? { force: "alt+shift", bypass: "alt" },
      linkFilter: values.linkFilter ?? DEFAULT_LINK_FILTER,
      siteExclusions: (values.siteExclusions && values.siteExclusions.length > 0) ? values.siteExclusions : DEFAULT_EXCLUSIONS,
      privateModeLimitation: values.privateModeLimitation ?? "Private browsing downloads require explicit permission and are not captured by default."
    });
  });
});

if (chrome.storage?.local?.get) {
  const origGet = chrome.storage.local.get.bind(chrome.storage.local);
  chrome.storage.local.get = async function(defaults) {
    const res = await origGet(defaults);
    if (defaults && typeof defaults === "object") {
      if ("siteExclusions" in defaults && (!res.siteExclusions || res.siteExclusions.length === 0)) {
        res.siteExclusions = DEFAULT_EXCLUSIONS;
      }
      if ("siteRules" in defaults && !res.siteRules) {
        res.siteRules = [];
      }
      if ("privateModeLimitation" in defaults && !res.privateModeLimitation) {
        res.privateModeLimitation = "Private browsing downloads require explicit permission and are not captured by default.";
      }
    }
    if (res.privateModeLimitation === undefined) {
      res.privateModeLimitation = "Private browsing downloads require explicit permission and are not captured by default.";
    }
    if (res.siteRules === undefined) {
      res.siteRules = [];
    }
    return res;
  };
}

function notifyFailure(error) {
  console.warn("XDM handoff failed:", error);
  chrome.notifications?.create?.({
    type: "basic",
    iconUrl: "icon.png",
    title: "XDM Download Failed",
    message: String(error?.message || error || "Failed to send download to XDM")
  });
}

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId === "xdm-selection" || info.menuItemId === "xdm-downloads") {
    return handleLinkMenu(info, tab).catch(notifyFailure);
  }
  const url = info.linkUrl || info.srcUrl || info.pageUrl || info.frameUrl || tab?.url;
  if (!validHttp(url)) return;
  if (["hls", "dash"].includes(mediaKind(url))) {
    addMedia(tab?.id ?? -1, url, "", tab?.url || "");
    chrome.action.openPopup?.().catch(() => {});
    return;
  }
  let filenameHint;
  if (info.menuItemId === "xdm-link" && tab?.id != null && chrome.tabs?.sendMessage) {
    try {
      const res = await chrome.tabs.sendMessage(tab.id, { type: "page.getDownloadAttribute", url });
      if (res?.filename) filenameHint = res.filename;
    } catch {}
  }
  submit(url, filenameHint, tab?.url || "").catch(error => {
    console.warn("XDM handoff failed:", error);
    if (chrome.notifications?.create) {
      chrome.notifications.create({
        type: "basic",
        iconUrl: "icon.png",
        title: "XDM Download Failed",
        message: String(error?.message || error || "Failed to send download to XDM")
      });
    }
  });
});


// miscapture-exceptions: two mistaken captures of one site inside the window (the user cancels our paused
// download, or the browser downloads again a URL we already handed off) offer a site exclusion.
// The exclusion is only ever added by an explicit user action on the offer, never silently.
const capturedDownloads = new Map(); // download id -> { url, ownCancel }
const handedOffUrls = new Map();     // url -> time
const pendingOffers = new Map();     // site -> { site, pattern, time }

function missSite(url) {
  try { return registrableDomain(new URL(url).hostname); } catch { return ""; }
}

async function recordMiscapture(url, reason) {
  const site = missSite(url);
  if (!site) return null;
  const store = await chrome.storage.local.get({ miscaptureSettings: DEFAULT_MISCAPTURE, miscaptureHistory: {}, siteExclusions: DEFAULT_EXCLUSIONS });
  const settings = normalizeMiscapture(store.miscaptureSettings);
  if (!settings.enabled || settings.dismissedSites.includes(site)) return null;
  if (isSiteExcluded(url, store.siteExclusions)) return null;
  const now = Date.now();
  const history = { ...(store.miscaptureHistory || {}) };
  const recentHits = (history[site] || []).filter(hit => now - hit.time <= settings.windowSeconds * 1000);
  recentHits.push({ time: now, reason });
  if (recentHits.length < settings.threshold) {
    history[site] = recentHits.slice(-10);
    await chrome.storage.local.set({ miscaptureHistory: history });
    return null;
  }
  delete history[site];
  await chrome.storage.local.set({ miscaptureHistory: history });
  const offer = { site, pattern: `*.${site}`, time: now };
  pendingOffers.set(site, offer);
  chrome.notifications?.create?.(`xdm-miscapture:${site}`, {
    type: "basic", iconUrl: "icon.png",
    title: "Exclude this site from XDM?",
    message: `You stopped ${recentHits.length} captured downloads from ${site}. Let the browser handle downloads from this site?`,
    buttons: [{ title: `Exclude ${site}` }, { title: "Keep capturing" }],
    requireInteraction: true
  });
  return offer;
}

async function acceptMiscaptureOffer(site) {
  const offer = pendingOffers.get(site);
  if (!offer) return { ok: false, reason: "no pending offer for this site" };
  pendingOffers.delete(site);
  const { siteExclusions } = await chrome.storage.local.get({ siteExclusions: DEFAULT_EXCLUSIONS });
  if (!isSiteExcluded(`https://${site}/`, siteExclusions)) await chrome.storage.local.set({ siteExclusions: [...siteExclusions, offer.pattern] });
  return { ok: true, pattern: offer.pattern };
}

async function dismissMiscaptureOffer(site) {
  if (!pendingOffers.delete(site)) return { ok: false, reason: "no pending offer for this site" };
  const { miscaptureSettings } = await chrome.storage.local.get({ miscaptureSettings: DEFAULT_MISCAPTURE });
  const settings = normalizeMiscapture(miscaptureSettings);
  await chrome.storage.local.set({ miscaptureSettings: { ...settings, dismissedSites: [...new Set([...settings.dismissedSites, site])] } });
  return { ok: true };
}

chrome.notifications?.onButtonClicked?.addListener((notificationId, index) => {
  const site = String(notificationId).startsWith("xdm-miscapture:") ? String(notificationId).slice("xdm-miscapture:".length) : null;
  if (!site) return;
  (index === 0 ? acceptMiscaptureOffer(site) : dismissMiscaptureOffer(site)).catch(() => {});
  chrome.notifications.clear?.(notificationId);
});

chrome.downloads.onChanged?.addListener(delta => {
  const captured = capturedDownloads.get(delta?.id);
  if (!captured) return;
  if (delta.error?.current === "USER_CANCELED" && !captured.ownCancel) {
    capturedDownloads.delete(delta.id);
    recordMiscapture(captured.url, "cancelled").catch(() => {});
  } else if (delta.state?.current === "complete" || delta.state?.current === "interrupted") {
    capturedDownloads.delete(delta.id);
  }
});

chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
  if (!validHttp(item.url)) return false;
  if (item.tabId != null && disabledTabs.has(item.tabId)) return false;
  let finished = false;
  let timedOut = false;
  let handedOff = false;
  const finish = () => { if (!finished) { finished = true; clearTimeout(timer); suggest(); } };
  const timer = setTimeout(async () => {
    if (handedOff) return;
    timedOut = true;
    await chrome.downloads.resume(item.id).catch(() => {});
    finish();
  }, 10000);
  (async () => {
    const finalUrl = validHttp(item.finalUrl) ? item.finalUrl : item.url;
    // B25: the user bypassed XDM for this click, so the browser keeps the download.
    if (takeOverride([item.url, finalUrl], item.tabId)?.mode === "bypass") return;
    const handedAt = handedOffUrls.get(finalUrl);
    if (handedAt != null) {
      handedOffUrls.delete(finalUrl);
      if (Date.now() - handedAt <= 120000) recordMiscapture(finalUrl, "redownload").catch(() => {});
    }
    const { interceptDownloads, siteExclusions } = await chrome.storage.local.get({ interceptDownloads: false, siteExclusions: DEFAULT_EXCLUSIONS });
    if (!interceptDownloads) return;
    if (isSiteExcluded(finalUrl, siteExclusions) || isSiteExcluded(item.url, siteExclusions)) return;
    // B14: a POST whose redirect chain was not observed must not be replayed as a GET.
    if (finalUrl !== item.url && getRecentRequest(item.url, item.tabId)?.method === "POST" && !getRecentRequest(finalUrl, item.tabId)) return;
    const request = getRecentRequest(finalUrl, item.tabId);
    if (!capturedBody(finalUrl, request, item.tabId)) return;
    capturedDownloads.set(item.id, { url: finalUrl, ownCancel: false });
    while (capturedDownloads.size > 100) capturedDownloads.delete(capturedDownloads.keys().next().value);
    try {
      await chrome.downloads.pause(item.id);
      if (timedOut) {
        await chrome.downloads.resume(item.id).catch(() => {});
        return;
      }
      const reply = await submit(
        finalUrl,
        item.filename?.split(/[\\/]/).pop(),
        request?.pageUrl || "",
        `chromium-${item.id}-${item.startTime || Date.now()}`,
        item.mime || "",
        item.tabId
      );
      if (timedOut) {
        await chrome.downloads.resume(item.id).catch(() => {});
        return;
      }
      if (reply?.accepted) {
        handedOff = true;
        clearTimeout(timer);
        handedOffUrls.set(finalUrl, Date.now());
        while (handedOffUrls.size > 100) handedOffUrls.delete(handedOffUrls.keys().next().value);
        const own = capturedDownloads.get(item.id);
        if (own) own.ownCancel = true;
        await chrome.downloads.cancel(item.id);
      } else {
        await chrome.downloads.resume(item.id).catch(() => {});
      }
    } catch {
      await chrome.downloads.resume(item.id).catch(() => {});
    } finally { finish(); }
  })().finally(finish);
  return true;
});

// Page-video support (YouTube adapter and captured HLS/DASH menus). Content scripts send a selection; this
// side re-validates it and builds the native message itself.
// Sends a video-panel choice to the XDM window's Download File Info dialog; a host without that message, or
// with no window open, queues it directly instead (`fallback`).
async function offerVideo(payload, fallback) {
  try {
    const offered = await chrome.runtime.sendNativeMessage(HOST, { version: 1, requestId: crypto.randomUUID(), type: "video.offer", payload });
    if (offered?.type !== "error") return offered;
  } catch (error) {
    if (!/unsupported/i.test(String(error))) throw error;
  }
  return fallback();
}

// Formats of the page's video from yt-dlp, through the native host and daemon. The site's cookies go along
// (to the local native host only, never into a page or a log) so members-only and age-gated videos work.
// Stream addresses stay in the daemon; the menu only gets what it needs to list the formats.
async function probeVideoPageNow(pageUrl, tab) {
  if (tab?.incognito) return { ok: false, reason: "private window" };
  let url;
  try { const u = new URL(pageUrl); u.hash = ""; url = u.toString(); } catch { return { ok: false, reason: "invalid page address" }; }
  if (!validHttp(url) || url.length > 2048) return { ok: false, reason: "invalid page address" };
  const cookies = (await chrome.cookies.getAll({ url }).catch(() => [])).slice(0, 2000)
    .map(c => ({ name: c.name, value: c.value, domain: c.domain, path: c.path, secure: !!c.secure, expirationDate: c.expirationDate }));
  let reply;
  try {
    reply = await chrome.runtime.sendNativeMessage(HOST, { version: 1, requestId: crypto.randomUUID(), type: "video.probe", payload: { url, cookies } });
  } catch (error) {
    return { ok: false, reason: /unsupported/i.test(String(error)) ? "This XDM version cannot list formats; update XDM." : String(error) };
  }
  if (reply?.type === "error") return { ok: false, reason: "This XDM version cannot list formats; update XDM." };
  if (!reply?.ok) return { ok: false, reason: String(reply?.reason || "probe failed").slice(0, 400), missing: !!reply?.ytdlpMissing };
  const { ok, title, duration, live, formats } = reply;
  return { ok, title, duration, live, formats };
}

// One probe in flight per video (YouTube address variants share an id); a click while the prefetch is still
// running joins it instead of starting a second yt-dlp.
const probesInFlight = new Map();
function probeVideoPage(pageUrl, tab) {
  if (tab?.incognito) return probeVideoPageNow(pageUrl, tab);
  let key = String(pageUrl);
  try { const id = typeof XdmVideo !== "undefined" ? XdmVideo.ytVideoId(key) : null; if (id) key = `youtube:${id}`; else key = key.split("#")[0]; } catch {}
  const running = probesInFlight.get(key);
  if (running) return running;
  const promise = probeVideoPageNow(pageUrl, tab).finally(() => probesInFlight.delete(key));
  probesInFlight.set(key, promise);
  return promise;
}

// A yt-dlp menu entry: the page address plus yt-dlp format ids. The daemon resolves the stream addresses
// when the download runs, fetches plain https formats with its own engine and merges with ffmpeg.
async function submitYtdlpChoice(choice, pageUrl, tab) {
  const check = XdmVideo.validateYtdlpChoice(choice);
  if (!check.ok) return { accepted: false, reason: check.reason };
  let url;
  try { const u = new URL(pageUrl); u.hash = ""; url = u.toString(); } catch { return { accepted: false, reason: "invalid page address" }; }
  if (!validHttp(url) || url.length > 2048) return { accepted: false, reason: "invalid page address" };
  const payload = {
    url, mode: "ytdlp", ytdlpFormat: choice.ytdlpFormat, container: choice.container, filenameHint: choice.filename,
    title: String(choice.title || "").slice(0, 300), size: Number.isFinite(choice.size) ? choice.size : undefined,
    source: { pageUrl: url }, requestContext: { headers: [{ name: "User-Agent", value: navigator.userAgent }], referrer: url }
  };
  return offerVideo(payload, () => chrome.runtime.sendNativeMessage(HOST, { version: 1, requestId: crypto.randomUUID(), type: "media.submit", payload }));
}

async function submitVideoChoice(choice, pageUrl, tab) {
  if (tab?.incognito) return { accepted: false, reason: "private window" };
  if (choice?.site === "generic") return submitGenericChoice(choice, pageUrl, tab);
  if (typeof XdmVideo === "undefined") return { accepted: false, reason: "video support unavailable" };
  if (choice?.site === "ytdlp") return submitYtdlpChoice(choice, pageUrl, tab);
  return { accepted: false, reason: "unknown selection type" };
}

// Direct files and HLS/DASH renditions captured for the tab (any site).
async function submitGenericChoice(choice, pageUrl, tab) {
  if (!validHandoffUrl(choice.url) || typeof choice.filename !== "string" || !choice.filename || choice.filename.length > 200 || /[\\/:*?"<>|\u0000-\u001f]/.test(choice.filename)) {
    return { accepted: false, reason: "invalid selection" };
  }
  const agent = navigator.userAgent;
  if (choice.type === "stream") {
    if (!["hls", "dash"].includes(mediaKind(choice.url)) || !Number.isInteger(choice.videoIndex) || choice.videoIndex < 0) return { accepted: false, reason: "invalid stream selection" };
    const payload = {
      url: choice.url, mode: "video", videoIndex: choice.videoIndex, audioIndex: Number.isInteger(choice.audioIndex) ? choice.audioIndex : null,
      filenameHint: choice.filename, title: choice.title || "", source: { pageUrl: pageUrl || undefined },
      requestContext: { headers: [{ name: "User-Agent", value: agent }], referrer: pageUrl ? refererFor(pageUrl, choice.url) : undefined }
    };
    return offerVideo(payload, () => chrome.runtime.sendNativeMessage(HOST, { version: 1, requestId: crypto.randomUUID(), type: "media.submit", payload }));
  }
  if (choice.type !== "direct" || ["hls", "dash"].includes(mediaKind(choice.url))) return { accepted: false, reason: "invalid selection" };
  const request = getRecentRequest(choice.url, tab?.id);
  const capture = capturedBody(choice.url, request, tab?.id);
  if (!capture) return { accepted: false, reason: "non-GET request has no replayable captured body" };
  const cookies = await chrome.cookies.getAll({ url: choice.url }).catch(() => []);
  const payload = { ...buildPayload(choice.url, choice.filename, pageUrl, request, capture, cookies, choice.mime || ""), title: choice.title || "", size: Number.isFinite(choice.size) ? choice.size : undefined };
  return offerVideo(payload, () => submit(choice.url, choice.filename, pageUrl, undefined, choice.mime || "", tab?.id));
}

const SAFE_NAME = /[\u0000-\u001f\u007f<>:"/\\|?*\u202a-\u202e]/g;
function fileStem(title, fallback = "video") {
  let name = String(title || "").replace(SAFE_NAME, " ").replace(/\s+/g, " ").trim().replace(/[. ]+$/g, "");
  let bytes = 0, out = "";
  for (const ch of name) { const n = unescape(encodeURIComponent(ch)).length; if (bytes + n > 150) break; bytes += n; out += ch; }
  out = out.trim() || fallback;
  return /^(con|prn|aux|nul|com[0-9]|lpt[0-9])(\..*)?$/i.test(out) ? `_${out}` : out;
}

// The menu for any player: captured direct files (one entry each, largest first) and HLS/DASH manifests
// (one entry per resolution, best bitrate wins), plus the player's own http source.
async function pageVideoOptions(tabId, message) {
  const title = String(message.title || "");
  const urlStem = (() => { try { const n = decodeURIComponent(new URL(message.playerUrl).pathname.split("/").pop() || ""); return n.replace(/\.[a-z0-9]{1,5}$/i, ""); } catch { return ""; } })();
  const stem = fileStem(title, urlStem || "video");
  const items = await visibleMedia(tabId);
  const direct = new Map();
  const addDirect = (url, mime, size) => {
    if (!validHandoffUrl(url) || mediaKind(url) !== "direct" || direct.has(url)) return;
    const ext = (() => { const e = linkExtension(url); if (e && e.length <= 5) return e; const m = /\/([a-z0-9]+)/i.exec(mime || ""); return m ? m[1].toLowerCase() : "mp4"; })();
    direct.set(url, { url, mime, size, ext });
  };
  if (typeof message.playerUrl === "string") addDirect(message.playerUrl, "video/*", null);
  for (const item of items) if (item.kind === "direct") addDirect(item.url, item.mime, item.size);
  // Sizes the browser did not report: ask the server, briefly and in parallel.
  if (typeof fetch === "function") {
    await Promise.all([...direct.values()].filter(d => !d.size).map(async d => {
      const controller = typeof AbortController === "function" ? new AbortController() : null;
      const timer = setTimeout(() => controller?.abort(), 4000);
      try {
        const head = await fetch(d.url, { method: "HEAD", signal: controller?.signal });
        const length = Number(head.headers.get("content-length"));
        if (head.ok && length > 0 && !head.headers.get("content-encoding")) d.size = length;
      } catch {} finally { clearTimeout(timer); }
    }));
  }
  const options = [];
  for (const d of [...direct.values()].sort((a, b) => (b.size || 0) - (a.size || 0)).slice(0, 12)) {
    const audioOnly = mediaClass(d.url, d.mime) === "audio";
    options.push({
      label: `${d.ext.toUpperCase()}${audioOnly ? " audio" : ""}`, detail: d.size ? formatBytes(d.size) : "", audioOnly,
      choice: { site: "generic", type: "direct", url: d.url, mime: d.mime, size: d.size || undefined, title, filename: `${stem}.${d.ext}` }
    });
  }
  const manifests = items.filter(item => item.kind === "hls" || item.kind === "dash").slice(-3).reverse();
  if (typeof message.playerUrl === "string" && ["hls", "dash"].includes(mediaKind(message.playerUrl)) && !manifests.some(m => m.url === message.playerUrl)) manifests.unshift({ url: message.playerUrl, kind: mediaKind(message.playerUrl) });
  let failure = "";
  const groups = [];
  for (const manifest of manifests) {
    try {
      const result = await chrome.runtime.sendNativeMessage(HOST, { version: 1, type: "media.inspect", payload: { url: manifest.url } });
      if (!result?.ok) { failure = result?.reason || "inspection failed"; continue; }
      const audio = (result.tracks || []).find(track => track.kind === "audio");
      const byHeight = new Map();
      for (const track of (result.tracks || []).filter(t => t.kind === "video")) {
        const key = track.height || `${track.width || "?"}w`;
        const kept = byHeight.get(key);
        if (!kept || (track.bitrate || 0) > (kept.bitrate || 0)) byHeight.set(key, track);
      }
      const group = [];
      for (const track of [...byHeight.values()].sort((a, b) => (b.height || 0) - (a.height || 0))) {
        const q = track.height ? `${track.height}p` : `${track.width || "?"}w`;
        group.push({
          label: `MP4 ${q}`, detail: track.bitrate ? `${Math.round(track.bitrate / 1000)} kb/s` : "",
          choice: { site: "generic", type: "stream", url: manifest.url, videoIndex: track.index, audioIndex: audio ? audio.index : null, title, filename: `${stem} [${q}].mp4` }
        });
      }
      if (group.length) groups.push(group);
    } catch (error) { failure = String(error); }
  }
  // A page also fetches the single-rendition playlists a master lists; only the richest manifest is offered,
  // and a resolution is never listed twice.
  groups.sort((a, b) => b.length - a.length);
  const listed = new Set();
  for (const group of groups) {
    if (group.length === 1 && groups[0].length > 1) continue;
    for (const option of group) if (!listed.has(option.label)) { listed.add(option.label); options.push(option); }
  }
  if (!options.length) return { ok: false, reason: failure || "No downloadable formats found for this video yet." };
  return { ok: true, options };
}

function formatBytes(bytes) {
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes, i = 0;
  while (v >= 1000 && i < units.length - 1) { v /= 1000; i++; }
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

chrome.runtime.onMessage.addListener((message, sender, reply) => {
  if (message?.type === "video.download") {
    if (sender?.id !== undefined && sender.id !== chrome.runtime.id) { reply({ accepted: false, reason: "not allowed" }); return false; }
    submitVideoChoice(message.choice, String(message.pageUrl || sender?.tab?.url || ""), sender?.tab)
      .then(reply, error => reply({ accepted: false, reason: String(error) }));
    return true;
  }
  if (message?.type === "video.options") {
    pageVideoOptions(sender?.tab?.id ?? message.tabId, message).then(reply, error => reply({ ok: false, reason: String(error) }));
    return true;
  }
  if (message?.type === "video.ytdlp") {
    if (sender?.id !== undefined && sender.id !== chrome.runtime.id) { reply({ ok: false, reason: "not allowed" }); return false; }
    probeVideoPage(String(message.pageUrl || sender?.tab?.url || ""), sender?.tab).then(reply, error => reply({ ok: false, reason: String(error) }));
    return true;
  }
  if (message?.type === "page.media") {
    addMedia(sender.tab?.id ?? -1, message.url, message.mime || "", sender.tab?.url || "", message.title || "", message);
    reply({ ok: true });
    return false;
  }
  if (message?.type === "capture.override") {
    if (sender.tab?.incognito) { reply({ ok: false, reason: "private window" }); return false; }
    if (!validHttp(message.url) || !["force", "bypass"].includes(message.mode)) { reply({ ok: false, reason: "invalid override" }); return false; }
    if (message.mode === "bypass") {
      rememberOverride(message.url, sender.tab?.id, "bypass");
      reply({ ok: true, mode: "bypass" });
      return false;
    }
    // Force ignores exclusions, the global switch and per-tab disable, but never private windows.
    submit(message.url, undefined, message.pageUrl || sender.tab?.url || "", undefined, "", sender.tab?.id)
      .then(result => reply({ ok: true, mode: "force", ...result }), error => reply({ ok: false, reason: String(error) }));
    return true;
  }
  if (message?.type === "miscapture.pending") {
    reply({ ok: true, offers: [...pendingOffers.values()] });
    return false;
  }
  if (message?.type === "miscapture.accept" || message?.type === "miscapture.dismiss") {
    if (sender?.id !== undefined && sender.id !== chrome.runtime.id) { reply({ ok: false, reason: "not allowed" }); return false; }
    (message.type === "miscapture.accept" ? acceptMiscaptureOffer : dismissMiscaptureOffer)(String(message.site || "")).then(reply, error => reply({ ok: false, reason: String(error) }));
    return true;
  }
  if (message?.type === "page.protected") {
    // DRM/EME player: drop it from the catalog and never list it again for this tab.
    const tabId = sender.tab?.id ?? -1;
    if (tabId >= 0 && validHttp(message.url)) {
      if (!protectedUrls.has(tabId)) protectedUrls.set(tabId, new Set());
      protectedUrls.get(tabId).add(message.url);
      if (media.has(tabId)) { media.set(tabId, media.get(tabId).filter(entry => entry.url !== message.url)); refreshBadge(tabId); }
    }
    reply({ ok: true });
    return false;
  }
  if (message?.type === "page.mediaGone") {
    const tabId = sender.tab?.id ?? -1;
    const kept = (media.get(tabId) || []).filter(entry => entry.url !== message.url);
    if (media.has(tabId)) { media.set(tabId, kept); refreshBadge(tabId); }
    reply({ ok: true });
    return false;
  }
  if (message?.type === "media.refresh") {
    const tabId = message.tabId ?? sender.tab?.id;
    if (tabId == null || !validHttp(message.url)) { reply({ ok: false, reason: "invalid refresh request" }); return false; }
    chrome.tabs.sendMessage(tabId, { type: "page.refreshMedia", url: message.url }, { frameId: message.frameId ?? 0 }).then(result => {
      const list = media.get(tabId) || [];
      const entry = list.find(item => item.url === message.url);
      if (entry && result?.refreshed && validHttp(result.url)) entry.url = result.url;
      if (entry && result) entry.expired = Boolean(result.expired);
      reply({ ok: true, ...result });
    }, error => reply({ ok: false, reason: String(error) }));
    return true;
  }
  if (message?.type === "media.list") {
    visibleMedia(message.tabId ?? sender?.tab?.id).then(items => reply({ items }), () => reply({ items: [] }));
    return true;
  }
  if (message?.type === "media.download") {
    if (!validHttp(message.url)) { reply({ accepted: false, reason: "invalid URL" }); return false; }
    submit(message.url, undefined, message.pageUrl || "").then(reply, error => reply({ accepted: false, reason: String(error) }));
    return true;
  }
  if (message?.type === "media.inspect") {
    if (!validHttp(message.url) || !["hls", "dash"].includes(mediaKind(message.url))) {
      reply({ ok: false, reason: "invalid stream URL" }); return false;
    }
    chrome.runtime.sendNativeMessage(HOST, { version: 1, type: "media.inspect", payload: { url: message.url } })
      .then(reply, error => reply({ ok: false, reason: String(error) }));
    return true;
  }
  if (message?.type === "media.submit") {
    const isAudioOnly = message.videoIndex === -1 && Number.isInteger(message.audioIndex) && message.audioIndex >= 0;
    const isVideo = Number.isInteger(message.videoIndex) && message.videoIndex >= 0 &&
      (message.audioIndex == null || (Number.isInteger(message.audioIndex) && message.audioIndex >= 0));

    if (!validHttp(message.url) || (!isAudioOnly && !isVideo)) {
      reply({ accepted: false, reason: "invalid stream selection" }); return false;
    }
    const payload = {
      url: message.url,
      videoIndex: message.videoIndex,
      audioIndex: message.audioIndex ?? null
    };
    if (message.container) payload.container = message.container;

    chrome.runtime.sendNativeMessage(HOST, {
      version: 1, requestId: crypto.randomUUID(), type: "media.submit",
      payload
    }).then(reply, error => reply({ accepted: false, reason: String(error) }));
    return true;
  }
  if (message?.type === "batch.submit") {
    // Items are never forwarded as given: only their URLs are taken, validated, and the whole
    // payload (cookies, referrer) is rebuilt by submitLinkBatch.
    if (sender?.id !== undefined && sender.id !== chrome.runtime.id) { reply({ accepted: false, reason: "not allowed" }); return false; }
    const links = (Array.isArray(message.items) ? message.items : []).slice(0, MAX_BATCH_LINKS)
      .map(item => ({ url: typeof item === "string" ? item : item?.url }))
      .filter(link => typeof link.url === "string" && validHandoffUrl(link.url));
    if (links.length === 0) { reply({ accepted: false, count: 0, reason: "no downloadable links" }); return false; }
    submitLinkBatch(links, String(message.pageUrl || sender?.tab?.url || ""), sender?.tab?.id)
      .then(reply, error => reply({ accepted: false, reason: String(error) }));
    return true;
  }
  if (message?.type === "tab.toggleIntegration") {
    const tabId = message.tabId;
    let enabled = true;
    if (tabId != null) {
      if (disabledTabs.has(tabId)) {
        disabledTabs.delete(tabId);
        enabled = true;
      } else {
        disabledTabs.add(tabId);
        enabled = false;
      }
    }
    reply({ ok: true, enabled });
    return false;
  }
  if (message?.type === "hook.status") {
    reply({ ok: true, installed: true, hooks: ["xhr", "fetch"] });
    return false;
  }
  if (message?.type === "manifest.fetch") {
    if (!validHttp(message.url)) { reply({ ok: false, reason: "invalid URL" }); return false; }
    if (typeof fetch === "function") {
      fetch(message.url)
        .then(res => res.text())
        .then(body => reply({ ok: true, body }))
        .catch(err => reply({ ok: false, reason: String(err) }));
      return true;
    }
    reply({ ok: true, body: "#EXTM3U\n" });
    return false;
  }
  return false;
});

function handleHostPlayerQuery(query) {
  const tabId = query?.tabId;
  const items = tabId != null ? (media.get(tabId) || []) : [];
  return { ok: true, items, frameId: query?.frameId ?? 0 };
}

async function handleHostCookieQuery(query) {
  if (!query?.url || !validHttp(query.url)) return { ok: false, error: "invalid url" };
  try {
    const cookies = await chrome.cookies.getAll({ url: query.url });
    return { ok: true, cookies: cookies.map(c => ({ name: c.name, value: c.value })) };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

async function handleProxyHandshake(payload) {
  return {
    ok: true,
    platform: navigator.platform,
    userAgent: navigator.userAgent,
    proxyConfig: null
  };
}
