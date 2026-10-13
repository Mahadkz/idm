const seen = new Set();
function inspect(node) {
  if (!(node instanceof Element)) return;
  const players = node.matches("video,audio") ? [node] : [...node.querySelectorAll("video,audio")];
  for (const player of players) {
    const urls = [player.currentSrc, player.src, ...[...player.querySelectorAll("source")].map(source => source.src)].filter(Boolean);
    for (const url of urls) {
      if (!/^https?:/i.test(url) || seen.has(url)) continue;
      seen.add(url);
      browser.runtime.sendMessage({ type: "page.media", url, mime: player.tagName.toLowerCase() + "/*" }).catch(() => {});
    }
  }
}
inspect(document.documentElement);
new MutationObserver(records => records.forEach(record => record.addedNodes.forEach(inspect))).observe(document.documentElement, { childList: true, subtree: true });
