// Pure helpers for page-video adapters (no DOM, no browser APIs): turn a site's player response into
// a quality menu and validate what the menu hands to the background script. The same file ships in the
// Chromium and Firefox extensions; tests/integration/contracts_browser.mjs keeps the copies identical.
var XdmVideo = (() => {
  const GOOGLEVIDEO = /(^|\.)googlevideo\.com$/i;
  const WATCH_HOSTS = /^(?:www\.|m\.|music\.)?youtube(?:-nocookie)?\.com$/i;
  const MAX_FORMATS = 200;
  const RESERVED = /^(con|prn|aux|nul|com[0-9]|lpt[0-9])(\..*)?$/i;

  const num = value => { const n = Number(value); return Number.isFinite(n) && n > 0 ? n : null; };

  // Watch pages, shorts, embeds, live pages and youtu.be short links; anything else is not a video page.
  function ytVideoId(url) {
    let u;
    try { u = new URL(url); } catch { return null; }
    const id = /^[A-Za-z0-9_-]{11}$/;
    if (u.hostname.toLowerCase() === "youtu.be") {
      const seg = u.pathname.split("/")[1];
      return id.test(seg || "") ? seg : null;
    }
    if (!WATCH_HOSTS.test(u.hostname)) return null;
    if (u.pathname === "/watch") { const v = u.searchParams.get("v"); return id.test(v || "") ? v : null; }
    const m = /^\/(?:shorts|embed|live|v)\/([A-Za-z0-9_-]{11})(?:[/?#]|$)/.exec(u.pathname);
    return m ? m[1] : null;
  }

  function parseMime(mimeType) {
    const m = /^\s*(video|audio)\/([a-z0-9.+-]+)\s*(?:;\s*codecs="?([^"]*)"?)?/i.exec(String(mimeType || ""));
    if (!m) return null;
    const codecs = (m[3] || "").split(",").map(c => c.trim()).filter(Boolean);
    return { type: m[1].toLowerCase(), subtype: m[2].toLowerCase(), codecs };
  }

  function expiryOf(url) {
    try {
      const n = Number(new URL(url).searchParams.get("expire"));
      return Number.isFinite(n) && n > 0 ? n * 1000 : null;
    } catch { return null; }
  }

  // One entry of streamingData.formats / adaptiveFormats. Ciphered formats (no plain URL) are kept but
  // flagged: this extension never runs a site's signature code outside the page.
  function normalizeFormat(raw, muxedList) {
    if (!raw || typeof raw !== "object") return null;
    const mime = parseMime(raw.mimeType);
    const itag = Number(raw.itag);
    if (!mime || !Number.isInteger(itag)) return null;
    const url = typeof raw.url === "string" ? raw.url : null;
    const vcodec = mime.type === "video" ? (mime.codecs[0] || "") : "";
    const acodec = mime.type === "audio" ? (mime.codecs[0] || "") : (mime.codecs[1] || "");
    const kind = mime.type === "audio" ? "audio" : (muxedList || acodec ? "muxed" : "video");
    const ext = mime.type === "audio" ? (mime.subtype === "mp4" ? "m4a" : mime.subtype) : mime.subtype;
    let n = false;
    try { n = !!url && new URL(url).searchParams.has("n"); } catch {}
    return {
      itag, kind, mime: `${mime.type}/${mime.subtype}`, ext, vcodec, acodec,
      width: num(raw.width), height: num(raw.height), fps: num(raw.fps),
      bitrate: num(raw.averageBitrate) || num(raw.bitrate), size: num(raw.contentLength),
      channels: num(raw.audioChannels), audioQuality: raw.audioQuality || null,
      isDrc: !!raw.isDrc || /[?&]xtags=[^&]*drc/i.test(url || ""),
      audioDefault: !raw.audioTrack || raw.audioTrack.audioIsDefault !== false,
      lang: raw.audioTrack?.displayName || null,
      approxMs: num(raw.approxDurationMs),
      url, ciphered: !url && !!(raw.signatureCipher || raw.cipher),
      throttled: n, expiresAt: url ? expiryOf(url) : null,
      drm: !!(raw.drmFamilies || raw.drmTrackType)
    };
  }

  // Reduces a /youtubei/v1/player response to a model. `playable` is false for live, DRM, private and
  // blocked videos: those never get a download panel.
  function normalizePlayerResponse(resp) {
    if (!resp || typeof resp !== "object") return null;
    const d = resp.videoDetails || {};
    const status = resp.playabilityStatus || {};
    const sd = resp.streamingData || {};
    const formats = [];
    for (const [list, muxed] of [[sd.formats, true], [sd.adaptiveFormats, false]]) {
      for (const raw of Array.isArray(list) ? list.slice(0, MAX_FORMATS) : []) {
        const f = normalizeFormat(raw, muxed);
        if (f) formats.push(f);
      }
    }
    const drm = formats.some(f => f.drm) || !!sd.drmFamilies || !!resp.drmFamilies;
    const live = !!(d.isLive || d.isUpcoming || (sd.hlsManifestUrl && !formats.length));
    const videoId = typeof d.videoId === "string" && /^[A-Za-z0-9_-]{11}$/.test(d.videoId) ? d.videoId : null;
    const playable = status.status === "OK" && !!videoId && !drm && !live && !d.isPrivate;
    return {
      videoId, title: String(d.title || "").slice(0, 300), author: String(d.author || "").slice(0, 100),
      durationSec: num(d.lengthSeconds), live, drm, playable,
      reason: status.status === "OK" ? (drm ? "protected" : live ? "live" : "") : String(status.reason || status.status || "unavailable").slice(0, 200),
      formats, hasUrls: formats.some(f => f.url), client: typeof resp.xdmClient === "string" ? resp.xdmClient : null
    };
  }

  // Adds URLs from a later source (another client's response, or URLs the player itself requested) to the
  // formats of a model that has none for them. Existing URLs are kept.
  function mergeUrls(model, extra, { addUnknown = true } = {}) {
    if (!model) return model;
    const byItag = new Map();
    for (const f of extra || []) if (f && f.url && !byItag.has(f.itag)) byItag.set(f.itag, f);
    const formats = model.formats.map(f => {
      if (f.url) return f;
      const e = byItag.get(f.itag);
      return e ? { ...f, url: e.url, ciphered: false, throttled: e.throttled, expiresAt: e.expiresAt, size: e.size || f.size, approxMs: e.approxMs || f.approxMs } : f;
    });
    const known = new Set(formats.map(f => f.itag));
    if (addUnknown) for (const e of byItag.values()) if (!known.has(e.itag)) formats.push(e);
    return { ...model, formats, hasUrls: formats.some(f => f.url) };
  }

  function safeName(title, fallback = "video") {
    let name = String(title || "").replace(/[\u0000-\u001f\u007f<>:"/\\|?*‪-‮]/g, " ").replace(/\s+/g, " ").trim();
    name = name.replace(/[. ]+$/g, "").slice(0, 120).trim();
    if (!name) name = fallback;
    if (RESERVED.test(name)) name = `_${name}`;
    return name;
  }

  function fmtSize(bytes) {
    if (!bytes) return "";
    const units = ["B", "KB", "MB", "GB"];
    let v = bytes, i = 0;
    while (v >= 1000 && i < units.length - 1) { v /= 1000; i++; }
    return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
  }

  const family = f => /^avc1|^h264/i.test(f.vcodec) ? "mp4" : /^vp0?9|^vp8/i.test(f.vcodec) ? "webm" : /^av01/i.test(f.vcodec) ? "av1" : "other";
  const better = (a, b) => (b.bitrate || 0) - (a.bitrate || 0) || (b.size || 0) - (a.size || 0);

  function bestAudio(audios, ext) {
    return audios.filter(a => a.ext === ext).sort(better)[0] || null;
  }

  // The IDM-style list: video qualities (largest first) as MP4/WEBM, then audio-only. Video-only adaptive
  // formats are paired with the best stereo audio of the matching codec family so the merge needs no
  // transcoding; muxed formats download as one file. Formats without a plain URL are omitted.
  function buildMenu(model, now = Date.now()) {
    if (!model || !model.playable) return [];
    const usable = model.formats.filter(f => f.url && (!f.expiresAt || f.expiresAt > now));
    const audios = usable.filter(f => f.kind === "audio" && !f.isDrc && f.audioDefault && (f.channels || 2) <= 2);
    const stem = safeName(model.title);
    for (const f of usable) f.durationSec = model.durationSec;
    const out = [];
    const seen = new Set();
    const push = o => { if (!seen.has(o.key)) { seen.add(o.key); out.push(o); } };

    for (const f of usable.filter(f => f.kind === "muxed")) {
      const ext = f.ext === "mp4" ? "mp4" : f.ext;
      const q = `${f.height || "?"}p${f.fps > 30 ? f.fps : ""}`;
      push({ key: `${ext}:${f.height}:${f.fps > 30 ? f.fps : 30}`, type: "direct", ext, container: ext, height: f.height || 0, fps: f.fps || 0,
        label: `${ext.toUpperCase()} ${q}`, size: f.size, video: pick(f), audio: null, filename: `${stem} [${q}].${ext}` });
    }
    const videos = usable.filter(f => f.kind === "video").sort((a, b) => (b.height || 0) - (a.height || 0) || (b.fps || 0) - (a.fps || 0) || better(a, b));
    const heights = new Map();
    for (const v of videos) {
      const k = `${v.height}:${v.fps > 30 ? v.fps : 30}`;
      if (!heights.has(k)) heights.set(k, []);
      heights.get(k).push(v);
    }
    for (const [k, group] of heights) {
      const byFamily = {};
      for (const v of group) { const fam = family(v); if (!byFamily[fam]) byFamily[fam] = v; }
      const picks = [];
      if (byFamily.mp4) picks.push(byFamily.mp4);
      if (byFamily.webm) picks.push(byFamily.webm);
      if (!picks.length && byFamily.av1) picks.push(byFamily.av1);
      if (!picks.length && byFamily.other) picks.push(byFamily.other);
      for (const v of picks) {
        const fam = family(v);
        const m4a = bestAudio(audios, "m4a"), webm = bestAudio(audios, "webm");
        const webmFirst = fam === "webm";
        const a = webmFirst ? (webm || m4a) : (m4a || webm);
        if (!a) continue;
        const container = fam === "webm" && a.ext === "webm" ? "webm" : (fam !== "webm" && a.ext === "m4a" ? "mp4" : "mkv");
        const q = `${v.height}p${v.fps > 30 ? v.fps : ""}`;
        push({ key: `${container}:${k}`, type: "adaptive", ext: container, container, height: v.height || 0, fps: v.fps || 0,
          label: `${container.toUpperCase()} ${q}`, size: v.size && a.size ? v.size + a.size : null,
          video: pick(v), audio: pick(a), filename: `${stem} [${q}].${container}` });
      }
    }
    out.sort((a, b) => b.height - a.height || b.fps - a.fps || (a.type === "direct" ? -1 : 1) - (b.type === "direct" ? -1 : 1));
    const seenAudio = new Set();
    for (const a of audios.slice().sort(better)) {
      const kbps = Math.round((a.bitrate || 0) / 1000);
      const key = `${a.ext}:${a.itag}`;
      if (seenAudio.has(key) || seenAudio.size >= 4) continue;
      seenAudio.add(key);
      const label = `${a.ext.toUpperCase()} ${kbps ? kbps + " kbps" : "audio"}`;
      out.push({ key: `a:${key}`, type: "direct", ext: a.ext, container: a.ext, height: 0, fps: 0, audioOnly: true,
        label, size: a.size, video: null, audio: pick(a), filename: `${stem} [${kbps || "audio"}k].${a.ext}`, url: a.url });
    }
    for (const o of out) {
      o.detail = fmtSize(o.size);
      o.id = o.key;
    }
    return out;
  }

  // Where to test whether a stream is served to the end: its last byte when the length is known, otherwise
  // 90% of the way through the size its bitrate and duration imply. YouTube cuts addresses obtained without
  // its integrity proof after about a minute of media, which shows up as a 403 deep into the file.
  function probeOffset(f, durationSec) {
    if (f.size) return f.size - 1;
    const seconds = f.approxMs ? f.approxMs / 1000 : durationSec;
    return f.bitrate && seconds ? Math.floor(f.bitrate / 8 * seconds * 0.9) : 0;
  }

  function pick(f) { return { itag: f.itag, url: f.url, size: f.size, ext: f.ext, mime: f.mime, expiresAt: f.expiresAt, probeAt: probeOffset(f, f.durationSec) }; }

  // What the background script accepts from a content script for a page-video download: every URL must be
  // an https googlevideo URL (the only hosts a YouTube adapter may hand to the daemon), the container must be
  // one the daemon merges, and the file name must be a plain name.
  function validateChoice(choice) {
    const bad = reason => ({ ok: false, reason });
    if (!choice || typeof choice !== "object") return bad("no selection");
    const okUrl = u => {
      try { const p = new URL(u); return p.protocol === "https:" && !p.username && !p.password && GOOGLEVIDEO.test(p.hostname) && u.length <= 8192; } catch { return false; }
    };
    const parts = [choice.video, choice.audio].filter(Boolean);
    if (!parts.length || !parts.every(p => okUrl(p.url))) return bad("stream address is not a YouTube media address");
    if (choice.type === "adaptive" && !(choice.video && choice.audio)) return bad("adaptive selection needs video and audio");
    if (choice.type === "adaptive" && !["mp4", "webm", "mkv"].includes(choice.container)) return bad("unsupported container");
    if (choice.type !== "adaptive" && choice.type !== "direct") return bad("unknown selection type");
    const name = String(choice.filename || "");
    if (!name || name.length > 200 || /[\\/:*?"<>|\u0000-\u001f]/.test(name)) return bad("invalid file name");
    return { ok: true };
  }

  // URLs the page's own player requested (plain GET videoplayback): strip the parameters that address one
  // chunk so the URL addresses the whole stream.
  const CHUNK_PARAMS = ["range", "rn", "rbuf", "ump", "sabr", "srfvp"];
  function wholeStreamUrl(url) {
    let u;
    try { u = new URL(url); } catch { return null; }
    if (!GOOGLEVIDEO.test(u.hostname) || u.pathname !== "/videoplayback") return null;
    for (const p of CHUNK_PARAMS) u.searchParams.delete(p);
    return u.toString();
  }

  return { ytVideoId, parseMime, normalizeFormat, normalizePlayerResponse, mergeUrls, buildMenu, validateChoice,
    wholeStreamUrl, safeName, fmtSize, expiryOf, googlevideo: GOOGLEVIDEO };
})();
if (typeof globalThis !== "undefined") globalThis.XdmVideo = XdmVideo;
