const HOST = "org.xdm.browser";
const MAX_RECENT = 300;
const HANDOFF_TIMEOUT_MS = 10000;
const { validHttp, mediaKind, isManifest, buildSubmit } = XdmCore;
const recent = new Map();
const bodies = new Map();
const media = new Map();

function remember(details) {
  if (!validHttp(details.url) || details.tabId < 0) return;
  const previous = recent.get(details.url) || {};
  const headers = (details.requestHeaders || []).filter(h => /^(referer|user-agent)$/i.test(h.name));
  recent.set(details.url, {
    ...previous, method: details.method || previous.method, tabId: details.tabId,
    cookieStoreId: details.cookieStoreId || previous.cookieStoreId,
    incognito: details.incognito ?? previous.incognito,
    pageUrl: details.originUrl || details.documentUrl || previous.pageUrl, headers, time: Date.now()
  });
  while (recent.size > MAX_RECENT) recent.delete(recent.keys().next().value);
}

// Observation only: non-blocking listeners, so Firefox never waits on this extension to load a page.
browser.webRequest.onBeforeSendHeaders.addListener(remember, { urls: ["http://*/*", "https://*/*"] }, ["requestHeaders"]);

// Captured POST bodies for main/sub-frame navigations are keyed by URL so a
// download triggered by the navigation can be replayed with the same request.
function encodeFormBody(requestBody) {
  if (!requestBody?.formData) return null;
  const fields = [];
  for (const [name, values] of Object.entries(requestBody.formData)) {
    for (const value of Array.isArray(values) ? values : [values]) {
      if (typeof value !== "string") return null; // file uploads cannot be replayed
      fields.push(`${encodeURIComponent(name)}=${encodeURIComponent(value)}`);
    }
  }
  return fields.join("&");
}

browser.webRequest.onBeforeRequest.addListener(details => {
  if (!validHttp(details.url) || details.tabId < 0) return;
  if ((details.method || "").toUpperCase() !== "POST") return;
  const body = encodeFormBody(details.requestBody);
  if (!body || body.length > 1024 * 1024) return; // uncaptured: the browser keeps the download
  bodies.set(details.url, { contentType: "application/x-www-form-urlencoded", body, tabId: details.tabId, time: Date.now() });
  while (bodies.size > MAX_RECENT) bodies.delete(bodies.keys().next().value);
}, { types: ["main_frame", "sub_frame"], urls: ["http://*/*", "https://*/*"] }, ["requestBody"]);

// Returns { method }, { method, body } or null; null means the request cannot
// be replayed by the daemon (browser keeps the download).
function capturedBody(url, request) {
  const method = (request?.method || "GET").toUpperCase();
  if (method === "GET") return { method: "GET" };
  const body = method === "POST" ? bodies.get(url) : undefined;
  return body ? { method, body } : null;
}

function addMedia(tabId, url, mime = "", pageUrl = "") {
  if (tabId < 0 || !validHttp(url)) return;
  const kind = mediaKind(url, mime);
  if (!kind) return;
  const entries = media.get(tabId) || [];
  if (!entries.some(entry => entry.url === url)) entries.push({ url, mime, kind, pageUrl, seenAt: Date.now() });
  if (entries.length > 100) entries.shift();
  media.set(tabId, entries);
  browser.browserAction.setBadgeText({ tabId, text: String(entries.length) });
}

browser.webRequest.onHeadersReceived.addListener(details => {
  const mime = (details.responseHeaders || []).find(header => header.name.toLowerCase() === "content-type")?.value || "";
  addMedia(details.tabId, details.url, mime, details.originUrl || details.documentUrl || "");
}, { urls: ["http://*/*", "https://*/*"] }, ["responseHeaders"]);

browser.tabs.onRemoved.addListener(tabId => {
  media.delete(tabId);
  for (const [url, entry] of bodies) if (entry.tabId === tabId) bodies.delete(url);
});
// Navigation to a new page replaces the tab's media catalog.
browser.tabs.onUpdated?.addListener((tabId, change) => {
  if (change.status === "loading" && change.url) { media.delete(tabId); browser.browserAction.setBadgeText({ tabId, text: "" }); }
});

// Containers need storeId; first-party isolation needs firstPartyDomain (null matches all domains).
async function cookiesFor(url, storeId) {
  const query = { url };
  if (storeId) query.storeId = storeId;
  try { return await browser.cookies.getAll({ ...query, firstPartyDomain: null }); }
  catch {
    try { return await browser.cookies.getAll(query); } catch { return []; }
  }
}

async function submit(url, filenameHint, pageUrl = "", requestId = crypto.randomUUID(), mime = "", storeId) {
  if (isManifest(url)) return { accepted: false, reason: "choose a stream in the XDM popup" };
  const request = recent.get(url);
  const capture = capturedBody(url, request);
  if (!capture) return { accepted: false, reason: "non-GET request has no replayable captured body" };
  const cookies = await cookiesFor(url, storeId || request?.cookieStoreId);
  return browser.runtime.sendNativeMessage(HOST, buildSubmit({
    url, filenameHint, mime, pageUrl, requestId, request, cookies, userAgent: navigator.userAgent,
    method: capture.body ? capture.method : undefined,
    body: capture.body ? capture.body.body : undefined,
    contentType: capture.body ? capture.body.contentType : undefined,
  }));
}

browser.runtime.onInstalled.addListener(() => {
  browser.contextMenus.create({ id: "xdm-link", title: "Download link with XDM", contexts: ["link"] });
  browser.contextMenus.create({ id: "xdm-media", title: "Download media URL with XDM", contexts: ["video", "audio"] });
  browser.storage.local.get({ interceptDownloads: false }).then(values => {
    if (values.interceptDownloads === undefined) browser.storage.local.set({ interceptDownloads: false });
  });
});

browser.contextMenus.onClicked.addListener((info, tab) => {
  const url = info.menuItemId === "xdm-link" ? info.linkUrl : info.srcUrl;
  if (!validHttp(url)) return;
  if (isManifest(url)) {
    addMedia(tab?.id ?? -1, url, "", tab?.url || "");
    browser.browserAction.openPopup?.().catch(() => {});
    return;
  }
  submit(url, undefined, tab?.url || "", undefined, "", info.cookieStoreId || tab?.cookieStoreId)
    .catch(error => console.warn("XDM handoff failed:", error));
});

// Firefox has no onDeterminingFilename: the download already exists in onCreated. Pause it, hand it
// off, then cancel+erase on acceptance; on any failure or timeout resume so the browser finishes it.
browser.downloads.onCreated.addListener(item => {
  if (!validHttp(item.url) || item.incognito || item.byExtensionId) return;
  let timedOut = false;
  let handedOff = false;
  const resume = () => browser.downloads.resume(item.id).catch(() => {});
  const timer = setTimeout(async () => {
    if (handedOff) return;
    timedOut = true;
    await resume();
  }, HANDOFF_TIMEOUT_MS);
  (async () => {
    const { interceptDownloads } = await browser.storage.local.get({ interceptDownloads: false });
    if (!interceptDownloads) return;
    const request = recent.get(item.url);
    if (!capturedBody(item.url, request) || request?.incognito) return;
    try {
      await browser.downloads.pause(item.id);
      if (timedOut) { await resume(); return; }
      const name = item.filename?.split(/[\\/]/).pop() || undefined;
      const reply = await submit(item.url, name, request.pageUrl || "", `firefox-${item.id}-${item.startTime || Date.now()}`, item.mime || "", request.cookieStoreId);
      if (timedOut) { await resume(); return; }
      if (reply?.accepted) {
        handedOff = true;
        clearTimeout(timer);
        await browser.downloads.cancel(item.id);
        await browser.downloads.erase({ id: item.id }).catch(() => {});
      } else await resume();
    } catch {
      await resume();
    }
  })().finally(() => { if (!timedOut) clearTimeout(timer); });
});

browser.runtime.onMessage.addListener((message, sender) => {
  if (message?.type === "page.media") {
    addMedia(sender.tab?.id ?? -1, message.url, message.mime || "", sender.tab?.url || "");
    return Promise.resolve({ ok: true });
  }
  if (message?.type === "media.list") return Promise.resolve({ items: media.get(message.tabId) || [] });
  if (message?.type === "media.download") {
    if (!validHttp(message.url)) return Promise.resolve({ accepted: false, reason: "invalid URL" });
    return submit(message.url, undefined, message.pageUrl || "").catch(error => ({ accepted: false, reason: String(error) }));
  }
  if (message?.type === "media.inspect") {
    if (!validHttp(message.url) || !isManifest(message.url)) return Promise.resolve({ ok: false, reason: "invalid stream URL" });
    return browser.runtime.sendNativeMessage(HOST, { version: 1, type: "media.inspect", payload: { url: message.url } })
      .catch(error => ({ ok: false, reason: String(error) }));
  }
  if (message?.type === "media.submit") {
    if (!validHttp(message.url) || !Number.isInteger(message.videoIndex) || message.videoIndex < 0 ||
        (message.audioIndex != null && (!Number.isInteger(message.audioIndex) || message.audioIndex < 0))) {
      return Promise.resolve({ accepted: false, reason: "invalid stream selection" });
    }
    return browser.runtime.sendNativeMessage(HOST, {
      version: 1, requestId: crypto.randomUUID(), type: "media.submit",
      payload: { url: message.url, videoIndex: message.videoIndex, audioIndex: message.audioIndex ?? null }
    }).catch(error => ({ accepted: false, reason: String(error) }));
  }
  return false;
});
