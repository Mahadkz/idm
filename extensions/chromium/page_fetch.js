// Page-context fetch monitoring hook wrapper for streaming media manifests.
(function() {
  if (typeof window === "undefined" || window.__xdmFetchHookInstalled) return;
  window.__xdmFetchHookInstalled = true;

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
    const origFetch = window.fetch;
    if (origFetch) {
      window.fetch = function(input, init) {
        const url = typeof input === "string" ? input : (input && input.url ? input.url : null);
        if (url) checkMediaUrl(url);
        return origFetch.apply(this, arguments);
      };
    }
  } catch {}
})();
