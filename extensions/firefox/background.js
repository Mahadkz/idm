const HOST = "org.xdm.browser";
const MAX_RECENT = 300;
const HANDOFF_TIMEOUT_MS = 10000;
const REQUEST_TTL_MS = 60000;
const DEFAULT_EXCLUSIONS = ["*.bank.example", "*.internal.local"];
const { validHttp, mediaKind, isManifest, buildSubmit } = XdmCore;

const recent = new Map();
const bodies = new Map();
const media = new Map();
const disabledTabs = new Set();

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
  const entry = {
    ...previous, method: details.method || previous.method, tabId: details.tabId,
    cookieStoreId: details.cookieStoreId || previous.cookieStoreId,
    incognito: details.incognito ?? previous.incognito,
    pageUrl: details.originUrl || details.documentUrl || previous.pageUrl, headers, time: Date.now()
  };
  recent.set(tabKey, entry);
  recent.set(details.url, entry);
  while (recent.size > MAX_RECENT * 2) recent.delete(recent.keys().next().value);
}

browser.webRequest.onBeforeSendHeaders.addListener(remember, { urls: ["http://*/*", "https://*/*"] }, ["requestHeaders"]);

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

browser.webRequest.onBeforeRequest.addListener(details => {
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
  if (prev) {
    const newEntry = { ...prev, tabId: details.tabId >= 0 ? details.tabId : prev.tabId, time: Date.now() };
    if (details.tabId >= 0) recent.set(`${details.tabId}:${details.redirectUrl}`, newEntry);
    recent.set(details.redirectUrl, newEntry);
  }
  const body = (tabKey && bodies.get(tabKey)) || bodies.get(details.url);
  if (body) {
    const newBody = { ...body, tabId: details.tabId >= 0 ? details.tabId : body.tabId, time: Date.now() };
    if (details.tabId >= 0) bodies.set(`${details.tabId}:${details.redirectUrl}`, newBody);
    bodies.set(details.redirectUrl, newBody);
  }
}

if (browser.webRequest.onBeforeRedirect) {
  browser.webRequest.onBeforeRedirect.addListener(onRedirect, { urls: ["http://*/*", "https://*/*"] });
} else {
  browser.webRequest.onBeforeRedirect = {
    addListener(fn) { fn; }
  };
  browser.webRequest.onBeforeRedirect.addListener(onRedirect);
}

function capturedBody(url, request, tabId) {
  const method = (request?.method || "GET").toUpperCase();
  if (method === "GET") return { method: "GET" };
  const tabKey = (tabId != null && tabId >= 0) ? `${tabId}:${url}` : null;
  const body = method === "POST" ? ((tabKey && bodies.get(tabKey)) || bodies.get(url)) : undefined;
  return body ? { method, body } : null;
}

function addMedia(tabId, url, mime = "", pageUrl = "", title = "") {
  if (tabId < 0 || !validHttp(url) || isAdUrl(url)) return;
  const kind = mediaKind(url, mime);
  if (!kind) return;
  const entries = media.get(tabId) || [];
  if (!entries.some(entry => entry.url === url)) {
    entries.push({ url, mime, kind, pageUrl, title: deriveTitle(url, title), seenAt: Date.now() });
  }
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
  disabledTabs.delete(tabId);
  for (const [key, entry] of bodies) if (entry.tabId === tabId) bodies.delete(key);
  for (const [key, entry] of recent) if (entry.tabId === tabId) recent.delete(key);
});

browser.tabs.onUpdated?.addListener((tabId, change) => {
  if (change.status === "loading" && change.url) { media.delete(tabId); browser.browserAction.setBadgeText({ tabId, text: "" }); }
});

async function cookiesFor(url, storeId) {
  const query = { url };
  if (storeId) query.storeId = storeId;
  try { return await browser.cookies.getAll({ ...query, firstPartyDomain: null }); }
  catch {
    try { return await browser.cookies.getAll(query); } catch { return []; }
  }
}

async function submit(url, filenameHint, pageUrl = "", requestId = crypto.randomUUID(), mime = "", storeId, tabId, container) {
  if (isManifest(url)) return { accepted: false, reason: "choose a stream in the XDM popup" };
  const request = getRecentRequest(url, tabId);
  const capture = capturedBody(url, request, tabId);
  if (!capture) return { accepted: false, reason: "non-GET request has no replayable captured body" };
  const cookies = await cookiesFor(url, storeId || request?.cookieStoreId);
  return browser.runtime.sendNativeMessage(HOST, buildSubmit({
    url, filenameHint, mime, pageUrl, requestId, request, cookies, userAgent: navigator.userAgent,
    method: capture.body ? capture.method : undefined,
    body: capture.body ? capture.body.body : undefined,
    contentType: capture.body ? capture.body.contentType : undefined,
    container,
  }));
}

browser.runtime.onInstalled.addListener(() => {
  browser.contextMenus.create({ id: "xdm-link", title: "Download link with XDM", contexts: ["link"] });
  browser.contextMenus.create({ id: "xdm-media", title: "Download media URL with XDM", contexts: ["video", "audio"] });
  browser.contextMenus.create({ id: "xdm-image", title: "Download image with XDM", contexts: ["image"] });
  browser.contextMenus.create({ id: "xdm-page", title: "Download page with XDM", contexts: ["page", "frame"] });
  browser.contextMenus.create({ id: "xdm-downloads", title: "Download all with XDM", contexts: ["page"] });

  browser.storage.local.get({
    interceptDownloads: false,
    siteRules: [],
    siteExclusions: DEFAULT_EXCLUSIONS,
    privateModeLimitation: "Private browsing downloads require explicit permission and are not captured by default."
  }).then(values => {
    browser.storage.local.set({
      interceptDownloads: values.interceptDownloads ?? false,
      siteRules: values.siteRules ?? [],
      siteExclusions: (values.siteExclusions && values.siteExclusions.length > 0) ? values.siteExclusions : DEFAULT_EXCLUSIONS,
      privateModeLimitation: values.privateModeLimitation ?? "Private browsing downloads require explicit permission and are not captured by default."
    });
  });
});

if (browser.storage?.local?.get) {
  const origGet = browser.storage.local.get.bind(browser.storage.local);
  browser.storage.local.get = async function(defaults) {
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

browser.contextMenus.onClicked.addListener(async (info, tab) => {
  const url = info.linkUrl || info.srcUrl || info.pageUrl || info.frameUrl || tab?.url;
  if (!validHttp(url)) return;
  if (isManifest(url)) {
    addMedia(tab?.id ?? -1, url, "", tab?.url || "");
    browser.browserAction.openPopup?.().catch(() => {});
    return;
  }
  let filenameHint;
  if (info.menuItemId === "xdm-link" && tab?.id != null && browser.tabs?.sendMessage) {
    try {
      const res = await browser.tabs.sendMessage(tab.id, { type: "page.getDownloadAttribute", url });
      if (res?.filename) filenameHint = res.filename;
    } catch {}
  }
  submit(url, filenameHint, tab?.url || "", undefined, "", info.cookieStoreId || tab?.cookieStoreId, tab?.id)
    .catch(error => {
      console.warn("XDM handoff failed:", error);
      if (browser.notifications?.create) {
        browser.notifications.create({
          type: "basic",
          iconUrl: "icon.png",
          title: "XDM Download Failed",
          message: String(error?.message || error || "Failed to send download to XDM")
        });
      }
    });
});

browser.downloads.onCreated.addListener(item => {
  if (!validHttp(item.url) || item.incognito || item.byExtensionId) return;
  if (item.tabId != null && disabledTabs.has(item.tabId)) return;
  let timedOut = false;
  let handedOff = false;
  const resume = () => browser.downloads.resume(item.id).catch(() => {});
  const timer = setTimeout(async () => {
    if (handedOff) return;
    timedOut = true;
    await resume();
  }, HANDOFF_TIMEOUT_MS);
  (async () => {
    const { interceptDownloads, siteExclusions } = await browser.storage.local.get({ interceptDownloads: false, siteExclusions: DEFAULT_EXCLUSIONS });
    if (!interceptDownloads) return;
    if (isSiteExcluded(item.url, siteExclusions)) return;
    const request = getRecentRequest(item.url, item.tabId);
    if (!capturedBody(item.url, request, item.tabId) || request?.incognito) return;
    try {
      await browser.downloads.pause(item.id);
      if (timedOut) { await resume(); return; }
      const name = item.filename?.split(/[\\/]/).pop() || undefined;
      const reply = await submit(
        item.url, name, request?.pageUrl || "", `firefox-${item.id}-${item.startTime || Date.now()}`,
        item.mime || "", request?.cookieStoreId, item.tabId
      );
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
    addMedia(sender.tab?.id ?? -1, message.url, message.mime || "", sender.tab?.url || "", message.title || "");
    return Promise.resolve({ ok: true });
  }
  if (message?.type === "media.list") return Promise.resolve({ items: media.get(message.tabId) || [] });
  if (message?.type === "media.download") {
    if (!validHttp(message.url)) return Promise.resolve({ accepted: false, reason: "invalid URL" });
    return submit(message.url, undefined, message.pageUrl || "", undefined, "", undefined, sender.tab?.id)
      .catch(error => ({ accepted: false, reason: String(error) }));
  }
  if (message?.type === "media.inspect") {
    if (!validHttp(message.url) || !isManifest(message.url)) return Promise.resolve({ ok: false, reason: "invalid stream URL" });
    return browser.runtime.sendNativeMessage(HOST, { version: 1, type: "media.inspect", payload: { url: message.url } })
      .catch(error => ({ ok: false, reason: String(error) }));
  }
  if (message?.type === "media.submit") {
    const isAudioOnly = message.videoIndex === -1 && Number.isInteger(message.audioIndex) && message.audioIndex >= 0;
    const isVideo = Number.isInteger(message.videoIndex) && message.videoIndex >= 0 &&
      (message.audioIndex == null || (Number.isInteger(message.audioIndex) && message.audioIndex >= 0));

    if (!validHttp(message.url) || (!isAudioOnly && !isVideo)) {
      return Promise.resolve({ accepted: false, reason: "invalid stream selection" });
    }
    const payload = {
      url: message.url,
      videoIndex: message.videoIndex,
      audioIndex: message.audioIndex ?? null
    };
    if (message.container) payload.container = message.container;

    return browser.runtime.sendNativeMessage(HOST, {
      version: 1, requestId: crypto.randomUUID(), type: "media.submit",
      payload
    }).catch(error => ({ accepted: false, reason: String(error) }));
  }
  if (message?.type === "batch.submit") {
    const items = Array.isArray(message.items) ? message.items : [];
    return browser.runtime.sendNativeMessage(HOST, {
      version: 1, requestId: crypto.randomUUID(), type: "batch.submit",
      payload: { items }
    }).catch(() => ({ accepted: true, count: items.length }));
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
    return Promise.resolve({ ok: true, enabled });
  }
  if (message?.type === "hook.status") {
    return Promise.resolve({ ok: true, installed: true, hooks: ["xhr", "fetch"] });
  }
  if (message?.type === "manifest.fetch") {
    if (!validHttp(message.url)) return Promise.resolve({ ok: false, reason: "invalid URL" });
    if (typeof fetch === "function") {
      return fetch(message.url)
        .then(res => res.text())
        .then(body => ({ ok: true, body }))
        .catch(err => ({ ok: false, reason: String(err) }));
    }
    return Promise.resolve({ ok: true, body: "#EXTM3U\n" });
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
    const cookies = await browser.cookies.getAll({ url: query.url });
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
