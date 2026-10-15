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

function addMedia(tabId, url, mime = "", pageUrl = "", title = "", extra = {}) {
  if (tabId < 0 || !validHttp(url) || isAdUrl(url)) return;
  const kind = mediaKind(url, mime);
  if (!kind) return;
  const entries = media.get(tabId) || [];
  if (!entries.some(entry => entry.url === url)) {
    const item = { url, mime, kind, pageUrl, title: deriveTitle(url, title), seenAt: Date.now() };
    if (extra.adapterId) { item.adapterId = extra.adapterId; item.adapterVersion = extra.adapterVersion; }
    if (extra.expiresAt) { item.expiresAt = extra.expiresAt; item.expired = Boolean(extra.expired); }
    entries.push(item);
  }
  if (entries.length > 100) entries.shift();
  media.set(tabId, entries);
  chrome.action.setBadgeText({ tabId, text: String(entries.length) });
}

chrome.webRequest.onHeadersReceived.addListener(details => {
  const mime = (details.responseHeaders || []).find(header => header.name.toLowerCase() === "content-type")?.value || "";
  addMedia(details.tabId, details.url, mime, details.initiator || "");
}, { urls: ["http://*/*", "https://*/*"] }, ["responseHeaders"]);

chrome.tabs.onRemoved.addListener(tabId => {
  media.delete(tabId);
  disabledTabs.delete(tabId);
  for (const [key, entry] of bodies) if (entry.tabId === tabId) bodies.delete(key);
  for (const [key, entry] of recent) if (entry.tabId === tabId) recent.delete(key);
});

function onTabUpdated(tabId, change) {
  if (change?.status === "loading" && change?.url) {
    media.delete(tabId);
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

function buildPayload(url, filenameHint, pageUrl, request, capture, cookies, mime = "") {
  const cookie = cookies.map(item => `${item.name}=${item.value}`).join("; ");
  const referrer = request?.headers?.find(header => header.name.toLowerCase() === "referer")?.value || (pageUrl || undefined);
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
    const cookies = await chrome.cookies.getAll({ url: link.url }).catch(() => []);
    items.push({ requestId: `${batchId}-${index}`, ...buildPayload(link.url, undefined, pageUrl, null, { method: "GET" }, cookies) });
  }
  if (items.length === 0) return { accepted: false, count: 0, reason: "no downloadable links" };
  const batchReply = await chrome.runtime.sendNativeMessage(HOST, {
    version: 1, requestId: batchId, type: "batch.submit", payload: { batchId, count: items.length, items }
  }).catch(error => ({ type: "error", reason: String(error?.message || error) }));
  if (batchReply && batchReply.type !== "error" && batchReply.accepted !== undefined) return { ...batchReply, batchId, count: items.length };
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
    const { interceptDownloads, siteExclusions } = await chrome.storage.local.get({ interceptDownloads: false, siteExclusions: DEFAULT_EXCLUSIONS });
    if (!interceptDownloads) return;
    if (isSiteExcluded(finalUrl, siteExclusions) || isSiteExcluded(item.url, siteExclusions)) return;
    // B14: a POST whose redirect chain was not observed must not be replayed as a GET.
    if (finalUrl !== item.url && getRecentRequest(item.url, item.tabId)?.method === "POST" && !getRecentRequest(finalUrl, item.tabId)) return;
    const request = getRecentRequest(finalUrl, item.tabId);
    if (!capturedBody(finalUrl, request, item.tabId)) return;
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

chrome.runtime.onMessage.addListener((message, sender, reply) => {
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
  if (message?.type === "page.mediaGone") {
    const tabId = sender.tab?.id ?? -1;
    const kept = (media.get(tabId) || []).filter(entry => entry.url !== message.url);
    if (media.has(tabId)) { media.set(tabId, kept); chrome.action.setBadgeText?.({ tabId, text: kept.length ? String(kept.length) : "" }); }
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
    reply({ items: media.get(message.tabId) || [] });
    return false;
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
    const items = Array.isArray(message.items) ? message.items : [];
    chrome.runtime.sendNativeMessage(HOST, {
      version: 1, requestId: crypto.randomUUID(), type: "batch.submit",
      payload: { items }
    }).then(reply, () => reply({ accepted: true, count: items.length }));
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
