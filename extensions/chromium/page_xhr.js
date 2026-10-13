// Page-context XHR monitoring hook for streaming manifests (.m3u8, .mpd) and media.
(function() {
  if (typeof window === "undefined" || window.__xdmXhrHookInstalled) return;
  window.__xdmXhrHookInstalled = true;

  const api = typeof chrome !== "undefined" && chrome.runtime?.sendMessage ? chrome : (typeof browser !== "undefined" ? browser : null);
  if (!api) return;

  function checkMediaUrl(url) {
    if (!url || typeof url !== "string") return;
    if (/\.(m3u8|mpd)(?:[?#]|$)/i.test(url)) {
      api.runtime.sendMessage({
        type: "page.media",
        url,
        mime: /\.m3u8/i.test(url) ? "application/vnd.apple.mpegurl" : "application/dash+xml",
        title: typeof document !== "undefined" ? document.title : ""
      }).catch(() => {});
    }
  }

  try {
    const origOpen = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function(_method, url) {
      if (typeof url === "string") checkMediaUrl(url);
      return origOpen.apply(this, arguments);
    };
  } catch {}
})();
