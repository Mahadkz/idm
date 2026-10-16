# XDM user guide

XDM is an open-source download manager for Linux and Windows. It has a desktop window (`xdm-gui`), a background service that holds the queue (`xdm-daemon`), a command-line tool (`xdm-cli`), and optional browser extensions for Chromium-family browsers and Firefox. macOS is not supported in this release.

Status note: Linux is the primary tested platform. The Windows installer is built on Linux and has not been tested on a real Windows machine. Browser integration has been tested with Chrome for Testing on Linux only.

## 1. Install

### Linux

- **AppImage**: `XDM-<version>-x86_64.AppImage`. Make it executable and run it. It starts `xdm-gui` by default; the other tools are inside it.
- **Debian package**: `xdm_<version>_amd64.deb`. It installs system-wide into `/usr/bin` and registers the native messaging host for Chrome, Chromium, Edge and Firefox. It does not start the daemon for you.
- **Tarball and user installer**: extract `xdm-<version>-<arch>-linux.tar.gz`, then run `install/linux/install.sh`. It needs no root and installs into `~/.local/bin`, adds a desktop entry and icon, and writes a systemd user unit.
  - `./install/linux/install.sh --autostart` also enables and starts `xdm-daemon.service`.
  - `./install/linux/install.sh --dry-run` prints what would change.
  - `./install/linux/uninstall.sh` removes exactly what the installer recorded in `~/.local/state/xdm/install-manifest`. Add `--purge` to also delete `~/.local/share/xdm`. Downloads are never removed.

### Windows

- `XDM-<version>-setup.exe` is a per-user installer. It installs into `%LOCALAPPDATA%\Programs\XDM`, adds a Start menu entry, and registers the native messaging host for Chrome, Chromium, Edge, Brave and Firefox under the current user. It is not yet tested on Windows.
- `install/windows/Install-XDM.ps1` is the PowerShell alternative. `-Autostart` adds a Run entry. `Uninstall-XDM.ps1` reverses it.

### Browser extensions

- **Chromium-family** (Chrome, Chromium, Edge, Brave): `xdm-chromium-<version>.zip`. Unpack it, open `chrome://extensions`, turn on Developer mode, and choose Load unpacked. The packaged extension has the fixed ID `pilmmicffpeccbaaienmhdohpnnobmcf`.
- **Firefox**: `xdm-firefox-<version>.xpi`. The build script does not sign the package. Firefox release builds only accept signed add-ons, so load it through `about:debugging` as a temporary add-on, or use a signed build if one is available to you.
- If you run XDM from a checkout instead of a package, `install/linux/install.sh` derives the Chromium extension ID from `extensions/chromium` and registers it for you.

## 2. First run

1. Start the daemon. With `--autostart`, the systemd user unit already runs it. Otherwise run `xdm-daemon` in a terminal and leave it running.
2. Start the window with `xdm-gui`.

The GUI, the browser native host and every `xdm-cli daemon` command talk to the daemon. If the daemon is not running they cannot do anything. The window does not start the daemon for you.

Downloads go to `<data directory>/downloads` by default, with one folder per category unless a category has its own destination (see section 9). Change the destination in Add URL or Download File Info, or in the category settings.

## 3. Adding downloads

### Add URL

- **Tasks → Add new download**, or the **Add URL** toolbar button.
- The Add URL window asks for **Address** and **Save as**, with the buttons **OK / Download** and **Cancel**.
- After OK, the **Download File Info** dialog opens. It shows **URL**, **Category**, **Save As**, **Description** and a **remember this path** check box. The buttons are **Start Download**, **Download Later** and **Cancel**.
- Click the **...** button next to Save As to pick a folder.
- XDM asks the server for the file size with a HEAD request. This is done only for `http://` addresses. For `https://` the size is shown as unknown. The HEAD request never downloads the file, so one-time links are not used up.

### Browser handoff

- Install a browser extension (section 1), then open its toolbar popup.
- The Chromium popup has a switch titled **Send eligible browser downloads to XDM**. It is off by default; interception is opt-in. The Firefox popup has an equivalent download-interception toggle.
- When it is on, eligible browser downloads are sent to XDM instead of the browser. If the daemon is not reachable, the browser keeps the download.
- Downloads started in private or incognito windows are not captured by default.
- The extension options page has site exclusions, force and bypass modifier keys, a link filter for **Download selected links** and **Download all links**, and media panel settings.

### Download this video (browser panel)

- With a browser extension installed, a **Download this video** button appears at the top right of a playing video. Click it for a list of the formats found: container and resolution, size where known, and audio-only entries. HLS and DASH streams list one entry per resolution; direct files list one entry each.
- Choosing an entry opens XDM's **Download File Info** dialog with the file name taken from the video title. If no XDM window is open, the download is queued directly instead.
- The extension toolbar popup shows the same list for the current tab.
- Videos with DRM protection never get the button. Live streams are not offered.
- YouTube and other video sites: XDM uses the optional [yt-dlp](https://github.com/yt-dlp/yt-dlp) program to list a page's formats (install it with `pipx install yt-dlp`, `brew install yt-dlp` or your package manager and keep it on your `PATH`; `xdm-cli daemon ytdlp-path ABSOLUTE_FILE` points XDM at a specific copy). Click the player's "Download this video" button, wait for "Getting formats…", and pick an entry such as "MP4 1080p 60fps" or "M4A 129 kbps". On sites with their own direct, HLS or DASH links those entries stay and a "More formats…" row asks yt-dlp. Separate video and audio streams are downloaded together and merged with ffmpeg; formats XDM cannot fetch itself are downloaded by yt-dlp. YouTube needs a JavaScript runtime that yt-dlp can use (deno, node, quickjs or bun); XDM passes the first one it finds. If yt-dlp is missing the menu says so and shows how to install it. Your browser's cookies for that site are given to yt-dlp through a private temporary file for the format lookup only (never in private windows).

### Batch download

- **Tasks → Add batch download…**
- Enter an address template with ranges, such as `https://example.com/img[001-100].jpg`. Supported forms are numeric ranges like `[1-10]` and zero-padded ranges like `[001-100]`, and letter ranges like `[a-z]`. Several groups can be combined.
- Choose the save folder and click **Add batch**. A single batch may expand to at most 10,000 addresses.

### Site grabber

- **Tasks → Run site grabber**. The window has **Start URL**, **Depth**, **Max pages**, **File types**, **Include** and **Exclude** patterns (URL globs such as `*.zip`), and a **Save folder**.
- **Crawl** starts the crawl. **Stop Crawl** stops it. Use **Save** to store the project under a name, and **Remove** or **Refresh** to manage saved projects.
- Saved projects also appear under **Grabber projects** in the category tree.

### Media download

- **Tasks → Add media download…** The window is titled **Add media** and has **Manifest URL** and **Headers (opt)** fields, then lists the available tracks.
- Browser extensions detect HLS and DASH manifests and media URLs on a page. Open the extension popup to see them and queue a chosen video and audio pair.
- Media jobs need `ffprobe` and `ffmpeg` on your `PATH`. XDM does not bundle them. Without them, media jobs fail.
- Video-page downloads (YouTube and similar sites) additionally use `yt-dlp` when it is installed; it is optional and XDM works without it.
- Media output is an MP4 copy of the selected streams, when the codecs allow it. Audio-only and subtitle outputs are also available from the CLI.

## 4. The main window

### Menu bar

- **Tasks**: Add new download, Add batch download…, Add media download…, Run site grabber, Import / Export…, Exit.
- **File**: Download now, Stop download, Remove (Del), Redownload, Restart, Open, Open folder, Move/Rename (Ctrl+M), Priority, Queue order, Properties.
- **Downloads**: Resume all, Stop all, Delete all completed, Find… (Ctrl+F), Scheduler, Queues…, Start queue, Stop queue, Schedule queue start…, Schedule queue stop…, Speed Limiter, Clean up partial files…, Categories…, Connection…, Options.
- **View**: Hide categories, Arrange files, Columns…, Large buttons, Small buttons, Refresh.
- **Help**: About XDM.

### Toolbar

Large or small buttons, chosen in **View**. The buttons are **Add URL**, **Resume**, **Stop**, **Stop All**, **Delete**, **Delete Completed**, **Options**, **Scheduler**, **Start Queue** and **Stop Queue**.

### Category tree

The left sidebar lists **All Downloads** (expandable to categories), **Unfinished**, **Finished**, **Grabber projects** and **Queues** (each named queue). Built-in categories are Compressed, Documents, Music, Programs and Video. Files that match no category go to the downloads folder directly.

### Columns

The task list has **File Name**, **Q**, **Size**, **Status**, **Time left**, **Transfer rate**, **Last Try Date**, **Description**, **Category** and **Queue**. Choose which are shown in **View → Columns…**, and drag the headers to reorder them. The layout is saved in `gui-layout.json`.

### Context menu

Right-click a download for Open, Open folder, Move/Rename (Ctrl-M), Redownload, Resume Download, Stop Download, Restart, Remove, Add to queue, Delete from queue, On Double click (Open file, Open folder or Properties) and Properties.

## 5. Queues and the scheduler

- **Downloads → Queues…** creates, renames and removes named queues, sets each queue's job limit, moves a selected stopped download into a queue and shows the queue's events.
- **Start queue** and **Stop queue** start and stop a queue. The **Start Queue** and **Stop Queue** toolbar buttons do the same for the global queue.
- **Schedule queue start…** and **Schedule queue stop…** take a one-time local time in `YYYY-MM-DD HH:MM` form or a Unix time.
- **Downloads → Scheduler** (or the toolbar **Scheduler**) opens queue settings for one queue:
  - start or stop the queue now;
  - retry policy (retries and delay in seconds);
  - a schedule (one-time, or daily between two times, optionally limited to chosen days);
  - the completion action.

Completion actions run when a queue finishes: **None**, **Notify**, **Sleep**, **Hibernate**, **Shutdown**, **Restart**, **Exit** or **Program** (an executable path and one argument per line). The countdown is 10 to 3600 seconds. Sleep, hibernate, shutdown and restart need a confirmation step. A queue stop affects downloads only; it never powers off the computer.

The CLI equivalents are `xdm-cli daemon queue ...` and `xdm-cli daemon completion status|cancel`. See section 10.

## 6. Speed limiter

- **Downloads → Speed Limiter → Turn on…** asks for a limit in KiB/s and applies it to the selected downloads. **Turn off** removes it. **Settings…** opens the bandwidth window.
- The bandwidth window has a **global limit** over all HTTP downloads, **per-host limits** (a pattern such as `example.com`, `*.example.com` or `*`), **per-host connection budgets**, and a **usage quota** in MiB. When the quota is used up, downloads wait in `quota_wait` until the period ends.
- Each progress window has a **Speed Limiter** tab with **Use Speed Limiter** and a KBytes/sec value.
- CLI: `daemon limit-global`, `daemon limit-host`, `daemon host-connections`, `daemon quota`, `daemon rate`, `daemon bandwidth`.

## 7. Proxies and site logins

### Proxies

**Downloads → Connection…** sets the mode to **System Proxy**, **Direct (No Proxy)** or **Manual Proxy**. For manual proxies, enter **HTTP Proxy**, **HTTPS Proxy** and a comma-separated **Bypass list**. The same window sets default connections per download (1 to 16) and the HTTP stall timeout (1 to 600 seconds).

Proxy passwords are not entered in the GUI. Store them from the CLI, with the password read from a file or from stdin:

```sh
xdm-cli daemon proxy-login USER PASSWORD_FILE
xdm-cli daemon proxy-login USER -
xdm-cli daemon proxy-login-clear
```

Proxy credentials are never accepted in a URL or on the command line.

### Site logins

Site logins (HTTP Basic and Digest for a URL pattern) are managed from the CLI. The GUI has no screen for them.

```sh
xdm-cli daemon login save PATTERN USER PASSWORD_FILE|-
xdm-cli daemon login list
xdm-cli daemon login remove ID
xdm-cli daemon auth TASK_ID USER PASSWORD_FILE|- [--remember]
```

`daemon auth` answers a download waiting on a 401 challenge.

## 8. Completion actions for a single download

- When a download finishes, the **Download complete** dialog opens with **Open**, **Open with...**, **Open folder**, **Close** and a **Don't show this dialog again** check box.
- The progress window's **Options on completion** tab controls whether that dialog is shown, whether to open the file, and whether to open its folder.
- Queue-level actions (section 5) apply to whole queues.

## 9. Categories

**Downloads → Categories…** lists built-in and custom categories. Each has a **Name**, **Extensions**, a **Destination folder** (empty means the default folder for the category) and **Sites**. Use **Save / Update**, **Remove** and **New / Clear**.

## 10. Command-line tool

`xdm-cli --help` prints the full usage. There is no separate help page for each subcommand. The usage has three forms.

**Direct download, no daemon needed:**

```sh
xdm-cli URL OUTPUT_PATH [--connections 1..16] [--header 'Name: value'] [--rate-kib N] [--restart] [--temp-dir ABSOLUTE_DIR] [--no-server-time]
xdm-cli URL OUTPUT_PATH --method POST --data-file BODY_FILE
xdm-cli URL OUTPUT_PATH [--stall-timeout 1..600] [--no-proxy | --proxy http|https|socks5|socks5h://HOST:PORT [--proxy-user USER --proxy-password-file FILE|-] [--proxy-bypass HOST,...]]
```

Press Ctrl-C to pause. Run the same command again to resume. `--restart` discards the partial data and starts from byte zero.

**Daemon commands** (`xdm-cli daemon ...`, daemon must be running). Main groups:

- Tasks: `list`, `submit`, `submit-post`, `info`, `pause`, `resume`, `remove`, `remove ID --delete-partial`, `redownload`, `restart`, `move`, `priority`, `rate`, `limit`, `bulk`, `move-file`, `partials`, `partials-delete`.
- Lists: `preview`, `import LIST ABSOLUTE_DIR [--start] [--allow-duplicates]`, `batch 'URL[001-100]' ABSOLUTE_DIR`, `playlist`, `export NEW_FILE json|urls`.
- Queues: `queue create|rename|remove|enable|disable|limit|assign|start|stop|start-one|stop-one|events|at|stop-at|retry|schedule|sync-interval|check|completion`, and `completion status|cancel`.
- Bandwidth and proxy: `limit-global`, `limit-host`, `host-connections`, `quota`, `bandwidth`, `proxy`, `proxy-login`, `proxy-login-clear`, `stall-timeout`, `temp-dir`, `ytdlp-path`.
- Grabber and categories: `grab`, `grab-status`, `grab-save`, `grab-projects`, `grab-remove`, `category list|save|remove`.
- Logins: `login list|save|remove|insecure`, `auth`.
- Support: `version`, `diagnostics OUT.zip|OUT_DIR` (section 11).

**Other top-level commands:**

- `xdm-cli grab PROJECT.json` runs a grabber project directly.
- `xdm-cli media inspect MANIFEST_URL`
- `xdm-cli media download MANIFEST_URL OUTPUT.mp4|OUTPUT.mkv VIDEO_INDEX [AUDIO_INDEX] [--subtitles 1,2]`
- `xdm-cli media record MANIFEST_URL OUTPUT VIDEO_INDEX [AUDIO_INDEX] [--max-seconds N]`
- `xdm-cli media audio MANIFEST_URL OUTPUT original|mp3:KBPS|m4a:KBPS|opus:KBPS`
- `xdm-cli media subtitles MANIFEST_URL OUTPUT.srt|.vtt TRACK`
- `xdm-cli media queue ...` and `xdm-cli daemon media-job ...` queue media jobs.

Every command that takes a password reads it from a file or from `-` (stdin). Passwords are never accepted as arguments.

`xdm-cli`, `xdm-daemon` and `xdm-gui` accept `--version`.

## 11. Where data, logs and crash reports live

Data directory:

- Linux: `$XDM_DATA_DIR`, else `$XDG_DATA_HOME/xdm`, else `~/.local/share/xdm`.
- Windows: `%LOCALAPPDATA%\XDM`.

Contents:

| Path | What it is |
|---|---|
| `tasks.sqlite3` | Downloads, queues, categories, settings, grabber projects |
| `credentials.json` | Site logins and proxy credentials (see privacy) |
| `gui-settings.json` | Window settings |
| `gui-layout.json` | Window layout and columns |
| `downloads/` | Default download folder |
| `logs/xdm-daemon.log`, `logs/xdm-gui.log` | Rotating logs, 5 MiB per file, 3 files kept |
| `crash/crash-*.txt` | Crash reports; the newest 10 are kept |
| `ipc-token`, `daemon.sock` | Local access token and socket (Linux). Windows uses a named pipe. |

Partial downloads sit next to their destination as `.part` and `.part.meta` files until they finish.

Log level is set with the `XDM_LOG` environment variable (`off`, `error`, `warn`, `info`, `debug`, `trace`; default `info`).

Support bundle: `xdm-cli daemon diagnostics OUT.zip` writes a zip, or `xdm-cli daemon diagnostics OUT_DIR` writes files into an empty folder. It contains version and OS information, a database integrity check, settings, a list of the latest 1,000 tasks with query strings removed, the logs and the crash reports. Headers, cookies and login data are not included.

## 12. Not available in this release

- No system tray icon. **Start minimized** minimizes the main window at startup.
- No GUI for site logins or proxy passwords (CLI only).
- No GUI switch for skipping the Download File Info dialog. The `show_file_info` value exists in `gui-settings.json`.
- No update checks, no automatic updates and no telemetry.
- No macOS build.
- Windows support is not tested on real Windows machines.
