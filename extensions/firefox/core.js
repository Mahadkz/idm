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

  function buildSubmit({ url, filenameHint, mime = "", pageUrl = "", requestId, request, cookies, userAgent, method, body, contentType, container }) {
    const cookie = cookieHeader(cookies);
    const referrer = headerValue(request?.headers, "referer") || (pageUrl ? refererFor(pageUrl, url) : undefined);
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
    registrableDomain, sameSite, batchCookies, refererFor, validHandoffUrl,
    DEFAULT_LINK_FILTER, normalizeLinkFilter, filterLinks, linksFromText, linkExtension, redirectMethod
  };
})();
