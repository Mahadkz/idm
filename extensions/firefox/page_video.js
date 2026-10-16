// Page-context (MAIN world) half of the YouTube adapter. It only observes the page's own
// /youtubei/v1/player traffic and the player element so the content script knows which video is playing
// and whether it is downloadable at all (not live, DRM or blocked). It requests nothing itself: formats
// come from yt-dlp. Nothing leaves the page except postMessage to the content script. Other sites: this
// file does nothing.
(function () {
  if (typeof window === "undefined" || window.__xdmVideoHook) return;
  if (!/(^|\.)youtube(?:-nocookie)?\.com$/i.test(location.hostname)) return;
  window.__xdmVideoHook = true;

  const nativeFetch = window.fetch.bind(window);
  const post = (type, data) => { try { window.postMessage({ xdmVideo: 1, type, ...data }, location.origin); } catch {} };
  const idRe = /^[A-Za-z0-9_-]{11}$/;

  // Only what the content script needs; avoids posting multi-megabyte responses around.
  function slim(resp, client) {
    if (!resp || typeof resp !== "object") return null;
    const sd = resp.streamingData || {};
    const keepFormat = f => {
      const { itag, mimeType, url, signatureCipher, cipher, width, height, fps, bitrate, averageBitrate, contentLength, audioQuality,
        audioChannels, audioSampleRate, audioTrack, isDrc, approxDurationMs, drmFamilies, drmTrackType, qualityLabel } = f || {};
      return { itag, mimeType, url, signatureCipher: signatureCipher || cipher ? "1" : undefined, width, height, fps, bitrate, averageBitrate,
        contentLength, audioQuality, audioChannels, audioSampleRate, audioTrack: audioTrack && { audioIsDefault: audioTrack.audioIsDefault, displayName: audioTrack.displayName },
        isDrc, approxDurationMs, drmFamilies, drmTrackType, qualityLabel };
    };
    const d = resp.videoDetails || {};
    return {
      xdmClient: client,
      playabilityStatus: { status: resp.playabilityStatus?.status, reason: resp.playabilityStatus?.reason },
      videoDetails: { videoId: d.videoId, title: d.title, author: d.author, lengthSeconds: d.lengthSeconds, isLive: d.isLive, isLiveContent: d.isLiveContent, isUpcoming: d.isUpcoming, isPrivate: d.isPrivate },
      streamingData: {
        expiresInSeconds: sd.expiresInSeconds, drmFamilies: sd.drmFamilies, hlsManifestUrl: sd.hlsManifestUrl ? "1" : undefined,
        sabr: sd.serverAbrStreamingUrl ? true : undefined,
        formats: (sd.formats || []).map(keepFormat), adaptiveFormats: (sd.adaptiveFormats || []).map(keepFormat)
      }
    };
  }

  function currentVideoId() {
    try {
      const u = new URL(location.href);
      if (u.pathname === "/watch") return u.searchParams.get("v");
      const m = /^\/(?:shorts|embed|live|v)\/([^/?#]+)/.exec(u.pathname);
      return m ? m[1] : null;
    } catch { return null; }
  }

  const seen = new Set();
  function report(resp, source) {
    const s = slim(resp, null);
    const id = s?.videoDetails?.videoId;
    if (!s || !id) return;
    const sig = `${id}:${source}:${s.streamingData.formats.length}:${s.streamingData.adaptiveFormats.length}`;
    if (seen.has(sig)) return;
    seen.add(sig);
    post("player", { source, response: s });
  }

  // 1. The page's own player requests.
  window.fetch = function (input) {
    const result = nativeFetch.apply(this, arguments);
    try {
      const url = typeof input === "string" ? input : (input && input.url) || String(input);
      if (/\/youtubei\/v1\/player(?:[?#]|$)/.test(url)) result.then(r => r.clone().json()).then(j => report(j, "request")).catch(() => {});
    } catch {}
    return result;
  };
  try {
    const open = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function (_method, url) {
      try {
        if (/\/youtubei\/v1\/player(?:[?#]|$)/.test(String(url))) {
          this.addEventListener("load", () => { try { report(JSON.parse(this.responseText), "request"); } catch {} });
        }
      } catch {}
      return open.apply(this, arguments);
    };
  } catch {}

  // 2. State that already exists: the first-load response variable and the player element's own accessor
  // (which also follows single-page navigation, where the variable goes stale).
  function fromPage() {
    const id = currentVideoId();
    try {
      const mp = document.getElementById("movie_player") || document.querySelector(".html5-video-player");
      const r = typeof mp?.getPlayerResponse === "function" ? mp.getPlayerResponse() : null;
      if (r && (!id || r.videoDetails?.videoId === id)) return report(r, "player");
    } catch {}
    try {
      const r = window.ytInitialPlayerResponse;
      if (r && (!id || r.videoDetails?.videoId === id)) report(r, "initial");
    } catch {}
  }
  for (const type of ["yt-navigate-finish", "yt-page-data-updated", "DOMContentLoaded", "load"]) {
    window.addEventListener(type, () => { fromPage(); setTimeout(fromPage, 1500); }, true);
  }
  document.addEventListener("loadedmetadata", () => fromPage(), true);

  window.addEventListener("message", event => {
    const m = event.data;
    if (event.source !== window || !m || m.xdmVideo !== 1) return;
    if (m.type === "hello") fromPage();
  });
  post("ready", {});
})();
