// Web-accessible page helper for media discovery before DOM readiness.
(function() {
  if (window.__xdmPageHelperInstalled) return;
  window.__xdmPageHelperInstalled = true;

  function reportMedia(url, mime, title) {
    if (!url || typeof url !== "string" || !/^https?:/i.test(url)) return;
    window.postMessage({ type: "XDM_PAGE_MEDIA", url, mime, title: title || document.title }, "*");
  }

  // Hook HTMLMediaElement src assignment if running in page context
  try {
    const proto = window.HTMLMediaElement?.prototype;
    if (proto) {
      const origPlay = proto.play;
      if (origPlay) {
        proto.play = function() {
          const url = this.currentSrc || this.src;
          if (url) reportMedia(url, (this.tagName || "video").toLowerCase() + "/*", this.title || document.title);
          return origPlay.apply(this, arguments);
        };
      }
    }
  } catch {}
})();
