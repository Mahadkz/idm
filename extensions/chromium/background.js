const HOST = "org.xdm.browser";
const MAX_RECENT = 300;
const recent = new Map();
const bodies = new Map();
const media = new Map();

function validHttp(url) {
  try { return ["http:", "https:"].includes(new URL(url).protocol); } catch { return false; }
}

function remember(details) {
  if (!validHttp(details.url) || details.tabId < 0) return;
  const previous = recent.get(details.url) || {};
  const headers = (details.requestHeaders || []).filter(h => /^(referer|user-agent)$/i.test(h.name));
  recent.set(details.url, { ...previous, method: details.method || previous.method, tabId: details.tabId, pageUrl: details.initiator || details.documentUrl || previous.pageUrl, headers, time: Date.now() });
  while (recent.size > MAX_RECENT) recent.delete(recent.keys().next().value);
}

chrome.webRequest.onBeforeSendHeaders.addListener(remember, { urls: ["http://*/*", "https://*/*"] }, ["requestHeaders"]);

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

chrome.webRequest.onBeforeRequest.addListener(details => {
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

function mediaKind(url, mime = "") {
  const path = new URL(url).pathname.toLowerCase();
  if (/\.(m3u8)(?:$)/.test(path) || /mpegurl/i.test(mime)) return "hls";
  if (/\.(mpd)(?:$)/.test(path) || /dash\+xml/i.test(mime)) return "dash";
  if (/^(video|audio)\//i.test(mime) || /\.(mp4|webm|mp3|m4a|aac|ogg)(?:$)/.test(path)) return "direct";
  return null;
}

function addMedia(tabId, url, mime = "", pageUrl = "") {
  if (tabId < 0 || !validHttp(url)) return;
  const kind = mediaKind(url, mime);
  if (!kind) return;
  const entries = media.get(tabId) || [];
  if (!entries.some(entry => entry.url === url)) entries.push({ url, mime, kind, pageUrl, seenAt: Date.now() });
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
  for (const [url, entry] of bodies) if (entry.tabId === tabId) bodies.delete(url);
});

async function submit(url, filenameHint, pageUrl = "", requestId = crypto.randomUUID(), mime = "") {
  if (["hls", "dash"].includes(mediaKind(url))) return { accepted: false, reason: "choose a stream in the XDM popup" };
  const request = recent.get(url);
  const capture = capturedBody(url, request);
  if (!capture) return { accepted: false, reason: "non-GET request has no replayable captured body" };
  const cookies = await chrome.cookies.getAll({ url }).catch(() => []);
  const cookie = cookies.map(item => `${item.name}=${item.value}`).join("; ");
  const referrer = request?.headers?.find(header => header.name.toLowerCase() === "referer")?.value || pageUrl;
  const agent = request?.headers?.find(header => header.name.toLowerCase() === "user-agent")?.value || navigator.userAgent;
  const headers = [{ name: "User-Agent", value: agent }];
  if (capture.body) headers.push({ name: "Content-Type", value: capture.body.contentType });
  return chrome.runtime.sendNativeMessage(HOST, {
    version: 1, requestId, type: "download.submit",
    payload: {
      url, filenameHint, mime, source: { pageUrl },
      requestContext: { headers, referrer, cookies: cookie ? [{ url, header: cookie }] : [] },
      method: capture.body ? capture.method : undefined,
      body: capture.body ? capture.body.body : undefined,
    }
  });
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: "xdm-link", title: "Download link with XDM", contexts: ["link"] });
  chrome.contextMenus.create({ id: "xdm-media", title: "Download media URL with XDM", contexts: ["video", "audio"] });
  chrome.storage.local.get({ interceptDownloads: false }).then(values => {
    if (values.interceptDownloads === undefined) chrome.storage.local.set({ interceptDownloads: false });
  });
});

chrome.contextMenus.onClicked.addListener((info, tab) => {
  const url = info.menuItemId === "xdm-link" ? info.linkUrl : info.srcUrl;
  if (!validHttp(url)) return;
  if (["hls", "dash"].includes(mediaKind(url))) {
    addMedia(tab?.id ?? -1, url, "", tab?.url || "");
    chrome.action.openPopup?.().catch(() => {});
    return;
  }
  submit(url, undefined, tab?.url || "").catch(error => console.warn("XDM handoff failed:", error));
});

chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
  if (!validHttp(item.url)) return false;
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
    const { interceptDownloads } = await chrome.storage.local.get({ interceptDownloads: false });
    if (!interceptDownloads) return;
    const request = recent.get(item.url);
    if (!capturedBody(item.url, request)) return;
    try {
      await chrome.downloads.pause(item.id);
      if (timedOut) {
        await chrome.downloads.resume(item.id).catch(() => {});
        return;
      }
      const reply = await submit(item.url, item.filename?.split(/[\\/]/).pop(), request.pageUrl || "", `chromium-${item.id}-${item.startTime || Date.now()}`, item.mime || "");
      if (timedOut) {
        await chrome.downloads.resume(item.id).catch(() => {});
        return;
      }
      if (reply?.accepted) {
        handedOff = true;
        clearTimeout(timer);
        await chrome.downloads.cancel(item.id);
      }
      else await chrome.downloads.resume(item.id).catch(() => {});
    } catch {
      await chrome.downloads.resume(item.id).catch(() => {});
    } finally { finish(); }
  })().finally(finish);
  return true;
});

chrome.runtime.onMessage.addListener((message, sender, reply) => {
  if (message?.type === "page.media") {
    addMedia(sender.tab?.id ?? -1, message.url, message.mime || "", sender.tab?.url || "");
    reply({ ok: true });
    return false;
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
    if (!validHttp(message.url) || !Number.isInteger(message.videoIndex) || message.videoIndex < 0 ||
        (message.audioIndex != null && (!Number.isInteger(message.audioIndex) || message.audioIndex < 0))) {
      reply({ accepted: false, reason: "invalid stream selection" }); return false;
    }
    chrome.runtime.sendNativeMessage(HOST, {
      version: 1, requestId: crypto.randomUUID(), type: "media.submit",
      payload: { url: message.url, videoIndex: message.videoIndex, audioIndex: message.audioIndex ?? null }
    }).then(reply, error => reply({ accepted: false, reason: String(error) }));
    return true;
  }
  return false;
});
