// Browser-agnostic helpers (no browser.* access) used by background.js.
// Kept per-extension rather than shared across directories so each extension package is self-contained.
var XdmCore = (() => {
  function validHttp(url) {
    try { return ["http:", "https:"].includes(new URL(url).protocol); } catch { return false; }
  }

  function mediaKind(url, mime = "") {
    const path = new URL(url).pathname.toLowerCase();
    if (/\.(m3u8)(?:$)/.test(path) || /mpegurl/i.test(mime)) return "hls";
    if (/\.(mpd)(?:$)/.test(path) || /dash\+xml/i.test(mime)) return "dash";
    if (/^(video|audio)\//i.test(mime) || /\.(mp4|webm|mp3|m4a|aac|ogg)(?:$)/.test(path)) return "direct";
    return null;
  }

  function isManifest(url) { return ["hls", "dash"].includes(mediaKind(url)); }

  function headerValue(headers, name) {
    return headers?.find(header => header.name.toLowerCase() === name)?.value;
  }

  function cookieHeader(cookies) {
    return (cookies || []).map(item => `${item.name}=${item.value}`).join("; ");
  }

  function buildSubmit({ url, filenameHint, mime = "", pageUrl = "", requestId, request, cookies, userAgent, method, body, contentType, container }) {
    const cookie = cookieHeader(cookies);
    const referrer = headerValue(request?.headers, "referer") || (pageUrl || undefined);
    const agent = headerValue(request?.headers, "user-agent") || userAgent;
    const headers = [{ name: "User-Agent", value: agent }];
    if (body) headers.push({ name: "Content-Type", value: contentType });
    const payload = {
      url, filenameHint, mime, source: { pageUrl: pageUrl || undefined },
      requestContext: { headers, referrer, cookies: cookie ? [{ url, header: cookie }] : [] },
      method: body ? (method || "POST") : undefined,
      body: body || undefined,
    };
    if (container) payload.container = container;
    return {
      version: 1, requestId, type: "download.submit",
      payload
    };
  }

  return { validHttp, mediaKind, isManifest, headerValue, cookieHeader, buildSubmit };
})();
