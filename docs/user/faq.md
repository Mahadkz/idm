# XDM FAQ

**Does XDM need the daemon running?**
Yes. The window, the browser native host and every `xdm-cli daemon` command talk to `xdm-daemon`. Start it first, or enable it with `install/linux/install.sh --autostart`. Direct `xdm-cli URL OUTPUT_PATH` downloads do not need the daemon.

**Can a paused download resume?**
Only if the server supports byte ranges and the file has not changed. XDM checks the server's validators (ETag, Last-Modified and length). If the server ignores range requests, XDM downloads the file again from the first byte when you resume. If the file changed on the server, the task stops with `restart_required`; use **Restart** in the File menu or `xdm-cli daemon restart ID`. See [troubleshooting](troubleshooting.md#resume-is-not-supported-by-the-server).

**Does XDM work with one-time links?**
The file-size check before download uses a HEAD request and never a GET, so it does not use up the link. The download itself is a GET. If the link is single-use, a failed attempt cannot be retried with the same link. The size check is only done for `http://` addresses; `https://` addresses show the size as unknown.

**Which browsers are supported?**
Chromium-family browsers (Chrome, Chromium, Edge, Brave) through `xdm-chromium-<version>.zip`, and Firefox through `xdm-firefox-<version>.xpi`. The Chromium extension has been tested with Chrome for Testing on Linux. Installed Chrome, Edge, Brave and Windows browser integration have not been tested.

**Does XDM send anything to the internet on its own?**
No telemetry, analytics, update checks or crash uploads exist in the code. Network traffic goes only to the addresses you download from, and to media or grabber addresses you choose. See [privacy](privacy.md).

**Can I use proxies?**
Yes. **Downloads → Connection…** supports System Proxy, Direct and Manual Proxy. Proxy usernames and passwords are stored with `xdm-cli daemon proxy-login`. HTTP, HTTPS, SOCKS4, SOCKS5 and SOCKS5h proxy schemes are accepted by the CLI.

**Where are passwords stored?**
Site logins and proxy passwords are kept in `credentials.json` in the data directory. On Linux the file is not encrypted; it has owner-only permissions (0600). On Windows the file is sealed with DPAPI, which has not been tested on a real Windows machine. Neither is returned by any command, export or log.

**Can XDM shut down or sleep my computer when a queue finishes?**
Yes, if you choose **Sleep**, **Hibernate**, **Shutdown** or **Restart** as the queue's completion action and confirm it. The countdown is 10 to 3600 seconds and can be cancelled with `xdm-cli daemon completion cancel QUEUE_ID`. Stopping a queue never powers off the computer.

**Where do downloads go?**
To `<data directory>/downloads`, with one folder per built-in category (Compressed, Documents, Music, Programs, Video), unless a category has its own destination in **Downloads → Categories…**. You can set the save path for each download in Download File Info.

**Does XDM have a tray icon?**
No. **Start minimized** minimizes the main window at startup. There is no tray icon in this release.

**Can I import or export download lists?**
Yes. **Tasks → Import / Export…**, or `xdm-cli daemon preview|import|export`. Lists are limited to 500 entries and 512 KiB. Exports omit request headers and URL usernames and passwords; query strings remain.

**Does XDM support macOS?**
No. Linux is the primary platform. The Windows installer is built on Linux and has not been run on a real Windows machine.

**How do I report a bug?**
Run `xdm-cli daemon diagnostics bundle.zip`, read the file to check it contains nothing you consider private, and attach it to your report. See [troubleshooting](troubleshooting.md#support-bundle).
