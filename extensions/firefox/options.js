const api = browser;
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
