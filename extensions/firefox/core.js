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

  return {
    validHttp, mediaKind, isManifest, headerValue, cookieHeader, buildSubmit,
    DEFAULT_LINK_FILTER, normalizeLinkFilter, filterLinks, linksFromText, linkExtension, redirectMethod
  };
})();
