const api = chrome;
const DEFAULT_KEYS = { force: "alt+shift", bypass: "alt" };
const field = id => document.getElementById(id);

api.storage.local.get({ captureKeys: DEFAULT_KEYS, linkFilter: { extensions: [], hideHtml: true, hideImages: true } }).then(values => {
  field("forceKeys").value = values.captureKeys.force;
  field("bypassKeys").value = values.captureKeys.bypass;
  field("linkExtensions").value = (values.linkFilter.extensions || []).join(", ");
  field("hideHtml").checked = values.linkFilter.hideHtml !== false;
  field("hideImages").checked = values.linkFilter.hideImages !== false;
});

field("saveOptions").addEventListener("click", async () => {
  const captureKeys = { force: field("forceKeys").value.trim(), bypass: field("bypassKeys").value.trim() };
  const linkFilter = {
    extensions: field("linkExtensions").value.split(/[\s,;]+/).filter(Boolean),
    hideHtml: field("hideHtml").checked,
    hideImages: field("hideImages").checked
  };
  await api.storage.local.set({ captureKeys, linkFilter });
  field("saveStatus").textContent = "Saved";
});

// Versioned settings objects; the background and content scripts normalise them again on read.
const list = value => value.split(/[\s,;]+/).map(item => item.trim().toLowerCase()).filter(Boolean);
api.storage.local.get({ mediaSettings: {}, miscaptureSettings: {} }).then(({ mediaSettings: m, miscaptureSettings: x }) => {
  field("minAudioKb").value = Math.round((m.minAudioBytes || 0) / 1024);
  field("minVideoKb").value = Math.round((m.minVideoBytes || 0) / 1024);
  field("panelExcludedSites").value = (m.panelExcludedSites || []).join(", ");
  field("overlayMode").value = m.overlayMode === "mini" ? "mini" : "full";
  field("overlayDisabledSites").value = (m.overlayDisabledSites || []).join(", ");
  field("miscaptureEnabled").checked = x.enabled !== false;
  field("miscaptureThreshold").value = x.threshold || 2;
  field("miscaptureWindow").value = x.windowSeconds || 120;
  field("miscaptureDismissed").value = (x.dismissedSites || []).join(", ");
});

field("saveMedia").addEventListener("click", async () => {
  const kb = id => Math.max(0, Math.floor(Number(field(id).value) || 0)) * 1024;
  const clamp = (id, min, max, fallback) => Math.min(max, Math.max(min, Math.floor(Number(field(id).value)) || fallback));
  await api.storage.local.set({
    mediaSettings: {
      version: 1, minAudioBytes: kb("minAudioKb"), minVideoBytes: kb("minVideoKb"),
      panelExcludedSites: list(field("panelExcludedSites").value),
      overlayMode: field("overlayMode").value === "mini" ? "mini" : "full",
      overlayDisabledSites: list(field("overlayDisabledSites").value)
    },
    miscaptureSettings: {
      version: 1, enabled: field("miscaptureEnabled").checked,
      threshold: clamp("miscaptureThreshold", 2, 10, 2), windowSeconds: clamp("miscaptureWindow", 5, 3600, 120),
      dismissedSites: list(field("miscaptureDismissed").value)
    }
  });
  field("saveMediaStatus").textContent = "Saved";
});
