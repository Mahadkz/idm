const seen = new Set();
const api = typeof browser !== "undefined" ? browser : (typeof chrome !== "undefined" ? chrome : null);

function isAd(url) {
  return /(?:[?&](?:ad_id|ad_url|adv_id|adid)=|\/ad_preroll|\/ads?\/|\/advert|\bdoubleclick\b|\bpagead\b)/i.test(url);
}

function inspect(node) {
  if (!(node instanceof Element)) return;
  const players = node.matches("video,audio")
    ? [node]
    : (node.querySelectorAll ? [...node.querySelectorAll("video"), ...node.querySelectorAll("audio")] : []);

  for (const player of players) {
    if (typeof player.getBoundingClientRect === "function") {
      const rect = player.getBoundingClientRect();
      if (rect && rect.width <= 1 && rect.height <= 1) continue; // suppress 1x1 tracking pixel
    }
    const sources = player.querySelectorAll ? [...player.querySelectorAll("source")].map(s => s.src) : [];
    const urls = [player.currentSrc, player.src, ...sources].filter(Boolean);
    for (const url of urls) {
      if (!/^https?:/i.test(url) || seen.has(url) || isAd(url)) continue;
      seen.add(url);
      const title = player.title || (typeof document !== "undefined" ? document.title : "");
      api?.runtime?.sendMessage({
        type: "page.media",
        url,
        mime: player.tagName.toLowerCase() + "/*",
        title
      })?.catch(() => {});
    }
  }
}

if (typeof document !== "undefined" && document.documentElement) {
  inspect(document.documentElement);
  if (typeof MutationObserver !== "undefined") {
    new MutationObserver(records => records.forEach(record => (record.addedNodes || []).forEach(inspect)))
      .observe(document.documentElement, { childList: true, subtree: true });
  }
}

if (api?.runtime?.onMessage?.addListener) {
  api.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
    if (msg?.type === "page.getDownloadAttribute") {
      const anchors = typeof document !== "undefined" && document.querySelectorAll ? [...document.querySelectorAll("a")] : [];
      const anchor = anchors.find(a => (a.href || a.attributes?.href) === msg.url);
      const filename = (anchor?.getAttribute ? anchor.getAttribute("download") : anchor?.attributes?.download) || anchor?.innerText || anchor?.textContent || undefined;
      sendResponse({ filename });
      return false;
    }
  });
}
