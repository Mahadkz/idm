// Pure helpers for page-video adapters (no DOM, no browser APIs): turn a site's player response into
// a quality menu and validate what the menu hands to the background script. The same file ships in the
// Chromium and Firefox extensions; tests/integration/contracts_browser.mjs keeps the copies identical.
var XdmVideo = (() => {
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

  const kbps = f => Math.round(f.abr || f.tbr || 0);
  const sizeOf = (...parts) => {
    if (parts.some(p => !p || !p.filesize)) return { size: null, approx: false };
    return { size: parts.reduce((n, p) => n + p.filesize, 0), approx: parts.some(p => p.filesizeApprox) };
  };
  const FAMILY = { avc: /^(avc1|h264)/i, vp9: /^vp0?[89]/i, av1: /^av01/i };
  const familyOf = f => FAMILY.avc.test(f.vcodec) ? "avc" : FAMILY.vp9.test(f.vcodec) ? "vp9" : FAMILY.av1.test(f.vcodec) ? "av1" : "other";
  const direct = f => f.protocol === "https" || f.protocol === "http";
  const byQuality = (a, b) => (direct(b) - direct(a)) || (b.tbr || 0) - (a.tbr || 0) || (b.filesize || 0) - (a.filesize || 0);
  const audioCodecName = f => /opus/i.test(f.acodec) ? "Opus" : /mp4a|aac/i.test(f.acodec) ? (f.ext === "m4a" ? "M4A" : "AAC") : /mp3/i.test(f.acodec) ? "MP3" : /vorbis/i.test(f.acodec) ? "Vorbis" : f.ext.toUpperCase();
  const FORMAT_ID = /^[A-Za-z0-9_][A-Za-z0-9_.-]{0,63}$/;
  const CONTAINERS = ["mp4", "mkv", "webm", "m4a", "opus", "mp3", "ogg", "aac"];

  // The IDM-style list from a yt-dlp probe: best combined options first ("MP4 1080p 60fps" ...), each
  // naming the yt-dlp format ids it needs, then audio-only entries. Video-only formats pair with the best
  // audio whose codec fits the container (H.264/AV1 with AAC into MP4, VP9 with Opus into WEBM); anything
  // else falls back to MKV. Live streams get no menu.
  function buildYtdlpMenu(probe) {
    if (!probe || probe.live || !Array.isArray(probe.formats)) return [];
    const usable = probe.formats.filter(f => f && FORMAT_ID.test(String(f.id)) && !/drc/i.test(`${f.id} ${f.note || ""}`));
    const stem = safeName(probe.title);
    const audios = usable.filter(f => f.kind === "audio").sort((a, b) => byQuality(a, b) || (b.abr || 0) - (a.abr || 0));
    const bestAudio = (re) => audios.filter(a => re.test(a.acodec)).sort((x, y) => (!!x.language - !!y.language) || byQuality(x, y))[0] || null;
    const m4a = bestAudio(/mp4a|aac/i), opus = bestAudio(/opus|vorbis/i);
    const out = [];
    const seen = new Set();
    const push = o => {
      if (seen.has(o.key) || out.length >= 40) return;
      seen.add(o.key);
      const quality = o.height ? `${o.height}p${o.fps > 30 ? ` ${Math.round(o.fps)}fps` : ""}` : (o.note || "video");
      const tag = `${o.height ? o.height + "p" : (o.note || "video")}${o.fps > 30 ? Math.round(o.fps) : ""}`;
      o.label = o.audioOnly ? o.audioLabel : `${o.container.toUpperCase()} ${quality}`;
      o.detail = o.size ? `${o.approx ? "~" : ""}${fmtSize(o.size)}` : "";
      o.filename = `${stem} [${o.audioOnly ? o.audioTag : tag}].${o.container}`;
      out.push(o);
    };
    for (const f of usable.filter(f => f.kind === "muxed").sort(byQuality)) {
      const s = sizeOf(f);
      push({ key: `m:${f.ext}:${f.height}:${f.fps > 30 ? f.fps : 30}`, ytdlpFormat: f.id, container: f.ext, ext: f.ext, height: f.height || 0, fps: f.fps || 0,
        note: f.note, size: s.size, approx: s.approx });
    }
    const groups = new Map();
    for (const v of usable.filter(f => f.kind === "video")) {
      const k = `${v.height || v.note || v.id}:${v.fps > 30 ? v.fps : 30}`;
      if (!groups.has(k)) groups.set(k, { height: v.height || 0, fps: v.fps || 0, formats: [] });
      groups.get(k).formats.push(v);
    }
    for (const [k, g] of groups) {
      const best = {};
      for (const v of g.formats.sort(byQuality)) { const fam = familyOf(v); if (!best[fam]) best[fam] = v; }
      const picks = [];
      if (best.avc) picks.push(best.avc);
      if (best.vp9) picks.push(best.vp9);
      if (!best.avc && best.av1) picks.push(best.av1);
      if (!picks.length) for (const v of Object.values(best)) { picks.push(v); break; }
      for (const v of picks) {
        const webmFirst = familyOf(v) === "vp9";
        const a = webmFirst ? (opus || m4a) : (m4a || opus);
        if (!a) continue;
        const container = webmFirst && a === opus && /webm/i.test(v.ext) ? "webm" : (!webmFirst && a === m4a && /mp4|m4v/i.test(v.ext) ? "mp4" : "mkv");
        const s = sizeOf(v, a);
        push({ key: `a:${container}:${k}`, ytdlpFormat: `${v.id}+${a.id}`, container, ext: container, height: g.height, fps: g.fps, note: v.note, size: s.size, approx: s.approx });
      }
    }
    out.sort((a, b) => b.height - a.height || b.fps - a.fps);
    const seenAudio = new Set();
    for (const a of audios) {
      const label = `${audioCodecName(a)} ${kbps(a) ? kbps(a) + " kbps" : "audio"}`;
      if (seenAudio.has(label) || seenAudio.size >= 4) continue;
      seenAudio.add(label);
      const s = sizeOf(a);
      push({ key: `o:${a.id}`, ytdlpFormat: a.id, container: a.ext, ext: a.ext, height: 0, fps: 0, audioOnly: true, audioLabel: label,
        audioTag: kbps(a) ? `${kbps(a)}k` : "audio", size: s.size, approx: s.approx });
    }
    return out.map(o => ({
      key: o.key, label: o.label, detail: o.detail, audioOnly: !!o.audioOnly, height: o.height, container: o.container,
      choice: { site: "ytdlp", type: "ytdlp", ytdlpFormat: o.ytdlpFormat, container: o.container, filename: o.filename, title: String(probe.title || "").slice(0, 300), size: o.size || undefined }
    }));
  }

  // What the background script accepts from a content script for a yt-dlp download: one or two plain format
  // ids, a container the daemon accepts and a plain file name. The page address is checked separately.
  function validateYtdlpChoice(choice) {
    const bad = reason => ({ ok: false, reason });
    if (!choice || typeof choice !== "object" || choice.site !== "ytdlp") return bad("no selection");
    const ids = String(choice.ytdlpFormat || "").split("+");
    if (ids.length < 1 || ids.length > 2 || !ids.every(id => FORMAT_ID.test(id))) return bad("invalid format selection");
    if (!CONTAINERS.includes(choice.container)) return bad("unsupported container");
    const name = String(choice.filename || "");
    if (!name || name.length > 200 || /[\\/:*?"<>|\u0000-\u001f]/.test(name)) return bad("invalid file name");
    return { ok: true };
  }

  return { ytVideoId, parseMime, normalizeFormat, normalizePlayerResponse, buildYtdlpMenu, validateYtdlpChoice, safeName, fmtSize };
})();
if (typeof globalThis !== "undefined") globalThis.XdmVideo = XdmVideo;
