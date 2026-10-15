// Offers to exclude a site after repeated cancelled captures; nothing is excluded until the user clicks Exclude.
chrome.runtime.sendMessage({ type: "miscapture.pending" }).then(reply => {
  const root = document.querySelector("#offers");
  for (const offer of reply?.offers || []) {
    const box = document.createElement("div"); box.className = "offer";
    const text = document.createElement("div"); text.textContent = `You cancelled captured downloads from ${offer.site} more than once. Let the browser handle this site?`;
    const yes = document.createElement("button"); yes.textContent = `Exclude ${offer.site}`;
    const no = document.createElement("button"); no.textContent = "Keep capturing";
    const answer = type => async () => { await chrome.runtime.sendMessage({ type, site: offer.site }); box.remove(); };
    yes.addEventListener("click", answer("miscapture.accept")); no.addEventListener("click", answer("miscapture.dismiss"));
    box.append(text, yes, no); root.append(box);
  }
}).catch(() => {});
const toggle = document.querySelector("#intercept");
chrome.storage.local.get({ interceptDownloads: false }).then(value => toggle.checked = value.interceptDownloads);
toggle.addEventListener("change", () => chrome.storage.local.set({ interceptDownloads: toggle.checked }));
chrome.tabs.query({ active: true, currentWindow: true }).then(async tabs => {
  const tab = tabs[0]; if (!tab) return;
  const { items } = await chrome.runtime.sendMessage({ type: "media.list", tabId: tab.id });
  const root = document.querySelector("#items");
  if (!items.length) return;
  root.replaceChildren();
  for (const item of items) {
    const row = document.createElement("div"); row.className = "item";
    const label = document.createElement("div"); label.textContent = item.url;
    const info = document.createElement("small"); info.textContent = item.kind === "direct" ? "Direct media URL" : `${item.kind.toUpperCase()} clear stream · inspect available tracks`;
    row.append(label, info);
    if (item.kind === "direct") {
      const button = document.createElement("button"); button.textContent = "Download with XDM";
      button.addEventListener("click", async () => {
        button.disabled = true;
        const reply = await chrome.runtime.sendMessage({ type: "media.download", url: item.url, pageUrl: item.pageUrl });
        info.textContent = reply?.accepted ? "Added to XDM" : `Could not hand off: ${reply?.reason || "unknown error"}`;
        button.disabled = false;
      });
      row.append(button);
    } else {
      const inspect = document.createElement("button"); inspect.textContent = "Choose quality";
      inspect.addEventListener("click", async () => {
        inspect.disabled = true; info.textContent = "Inspecting stream…";
        try {
          const result = await chrome.runtime.sendMessage({ type: "media.inspect", url: item.url });
          if (!result?.ok) throw new Error(result?.reason || "inspection failed");
          const videos = result.tracks.filter(track => track.kind === "video")
            .sort((a, b) => (b.height || 0) - (a.height || 0) || (b.bitrate || 0) - (a.bitrate || 0));
          const audios = result.tracks.filter(track => track.kind === "audio");
          if (!videos.length) throw new Error("no video tracks found");
          const videoSelect = document.createElement("select");
          for (const track of videos) {
            const option = document.createElement("option"); option.value = String(track.index);
            option.textContent = `${track.width || "?"}×${track.height || "?"} · ${track.codec}${track.bitrate ? ` · ${Math.round(track.bitrate / 1000)} kb/s` : ""}`;
            videoSelect.append(option);
          }
          const audioSelect = document.createElement("select");
          const none = document.createElement("option"); none.value = ""; none.textContent = "No audio"; audioSelect.append(none);
          for (const track of audios) {
            const option = document.createElement("option"); option.value = String(track.index);
            option.textContent = `${track.language || "Audio"} · ${track.codec}${track.bitrate ? ` · ${Math.round(track.bitrate / 1000)} kb/s` : ""}`;
            audioSelect.append(option);
          }
          if (audios.length) audioSelect.value = String(audios[0].index);
          const download = document.createElement("button"); download.textContent = "Add to XDM queue";
          download.addEventListener("click", async () => {
            download.disabled = true; info.textContent = "Submitting stream…";
            try {
              const result = await chrome.runtime.sendMessage({ type: "media.submit", url: item.url,
                videoIndex: Number(videoSelect.value), audioIndex: audioSelect.value === "" ? null : Number(audioSelect.value) });
              info.textContent = result?.accepted ? `Queued as task #${result.taskId}` : `Could not queue: ${result?.reason || "unknown error"}`;
            } catch (error) { info.textContent = `Could not queue: ${error}`; }
            finally { download.disabled = false; }
          });
          row.append(videoSelect, audioSelect, download);
          inspect.remove();
          info.textContent = `${videos.length} video and ${audios.length} audio track(s) available`;
        } catch (error) {
          info.textContent = `Could not inspect: ${error}`;
          inspect.disabled = false;
        }
      });
      row.append(inspect);
    }
    root.append(row);
  }
});
