# XDM privacy

This page describes what XDM stores on your computer, what it sends over the network, and what the browser extension can see. It was written from the source code in this repository.

## What is sent over the network

Checked by searching `crates/` for telemetry, analytics, update checks, crash uploads and hard-coded addresses, the search found none. No analytics, telemetry, automatic update or crash-report upload code exists in the program. The only outbound connections are the ones that serve a download you asked for:

- **Your downloads.** The HTTP, HTTPS and FTP addresses you add, import, batch-create or hand over from the browser. This includes any cookies or headers the browser sent with that address.
- **Size check before download.** Add URL and Download File Info send a HEAD request to `http://` addresses to learn the file size. This is a direct connection that does not use the configured proxy. `https://` addresses are not checked.
- **Queue sync checks.** A sync queue re-checks the remote files it manages with a lightweight request, and fetches again only changed files.
- **Site grabber.** Crawling a start URL requests the pages and links you specify, within your depth and page limits.
- **Media.** Manifests and media segments you choose, plus manifests the browser extension fetches from the page you are viewing. ffmpeg and ffprobe reach these addresses only through a local loopback relay that XDM starts on `127.0.0.1`.

XDM does not contact any other host. Local network listeners are limited to the loopback addresses used by the media relay and the daemon's local socket.

## What is stored locally

Data directory: `$XDM_DATA_DIR`, else `~/.local/share/xdm` on Linux, or `%LOCALAPPDATA%\XDM` on Windows. The directory is created with mode 0700 on Linux.

| Item | Contents | Protection |
|---|---|---|
| `tasks.sqlite3` | Download URLs, output paths, status and history, queues, categories, grabber projects, and the **request headers** stored per download (for example a `Cookie` header passed over from the browser) | Not encrypted. Protected only by the 0700 directory on Linux. |
| `credentials.json` | Site login passwords and proxy passwords | Linux: plaintext JSON with mode 0600. Windows: sealed with DPAPI for the current user. The Windows path has not been tested on real Windows. |
| `ipc-token` | Token for local control of the daemon | Mode 0600 on Linux |
| `gui-settings.json`, `gui-layout.json` | Window settings and layout | Plain files |
| `downloads/` and your chosen folders | Downloaded files | Your files, not encrypted by XDM |
| `logs/` | Daemon and GUI logs, 5 MiB per file, three files | Redacted (see below) |
| `crash/` | Crash reports, newest ten | Redacted (see below) |

Partial downloads sit next to their destination as `.part` and `.part.meta` files until they finish.

### Redaction in logs, crash reports and exports

- Logs and crash reports pass through the same redaction routine. It replaces the user name and password in URLs, the query string of URLs, values of `Authorization`, `Proxy-Authorization`, `Cookie`, `Set-Cookie` and `X-API-Key` headers, and values of names such as password, token, secret, session, signature and body.
- The support bundle (`xdm-cli daemon diagnostics`) contains the same redacted logs and crash reports, and the task list with URL query strings and user names removed. Request headers, cookies, logins and proxy passwords are not included.
- Exports (`xdm-cli daemon export`) omit request headers and URL user names and passwords. URL query strings remain in exports.

Passwords are read from files or standard input and never accepted on the command line or in URLs.

## What the browser extension can see

The extension is optional and off for downloads until you turn on interception in its popup. Its manifest requests these permissions: downloads, native messaging, context menus, storage, cookies, webRequest, tabs, notifications, management, webNavigation, proxy and declarative network requests. It also runs on every `http://` and `https://` page, including in every frame, and observes page requests.

In practice:

- **Page content.** A content script runs on all pages and hooks page `XMLHttpRequest` and `fetch` calls to detect media URLs. Detected media is listed in the popup.
- **Requests.** A `webRequest` listener records the `Referer` and `User-Agent` headers of recent requests in memory for a short time. For form submissions it also records the request body, so a POST download can be replayed. These records are not written to disk.
- **Cookies.** When the extension hands a download to XDM, it reads the cookies for that address through the browser's cookie API and sends them, along with the headers above, to the local native host. The native host keeps a cookie only when it belongs to the download's own origin. The cookie is then stored in the task's request headers in `tasks.sqlite3`.
- **Manifests.** Media manifests are fetched by the extension from the page's own addresses.
- **Settings.** Site exclusions, key bindings, link filters and media settings are stored in the extension's local storage.
- **Private windows.** Downloads from private or incognito windows are not captured unless you allow the extension to run there.

The extension talks only to the local native messaging host, and that host talks only to the local daemon. The extension itself does not send data to any other service.

## Summary

- XDM sends no usage data and checks for no updates.
- Downloads and their saved request headers, including cookies handed over from the browser, are stored in `tasks.sqlite3` without encryption.
- Passwords are kept in `credentials.json`: unencrypted with restricted permissions on Linux, DPAPI-sealed on Windows.
- Logs, crash reports and support bundles are redacted, and you should still read a bundle before sharing it.
