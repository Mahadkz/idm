const toggle = document.querySelector("#intercept");
browser.storage.local.get({ interceptDownloads: false }).then(value => toggle.checked = value.interceptDownloads);
toggle.addEventListener("change", () => browser.storage.local.set({ interceptDownloads: toggle.checked }));
browser.tabs.query({ active: true, currentWindow: true }).then(async tabs => {
  const tab = tabs[0]; if (!tab) return;
  const { items } = await browser.runtime.sendMessage({ type: "media.list", tabId: tab.id });
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
        const reply = await browser.runtime.sendMessage({ type: "media.download", url: item.url, pageUrl: item.pageUrl });
        info.textContent = reply?.accepted ? "Added to XDM" : `Could not hand off: ${reply?.reason || "unknown error"}`;
        button.disabled = false;
      });
      row.append(button);
    } else {
      const inspect = document.createElement("button"); inspect.textContent = "Choose quality";
      inspect.addEventListener("click", async () => {
        inspect.disabled = true; info.textContent = "Inspecting stream…";
        try {
          const result = await browser.runtime.sendMessage({ type: "media.inspect", url: item.url });
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
              const result = await browser.runtime.sendMessage({ type: "media.submit", url: item.url,
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
