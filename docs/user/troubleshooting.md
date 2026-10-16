# XDM troubleshooting

Start with the task's details (select the download and open **Properties**). The error text and the failure class are shown there. Failure classes are: DNS lookup failed, connection refused, connection timed out, TLS/certificate failure, read timed out (stalled connection), server returned an error status, authentication required, disk error, unusable proxy configuration, network error and error.

## The daemon is not reachable

The GUI, browser handoff and `xdm-cli daemon` commands all fail or do nothing.

- Check that `xdm-daemon` is running. On Linux, `systemctl --user status xdm-daemon` shows the systemd unit if you used `--autostart`.
- Read `logs/xdm-daemon.log` in the data directory (see the [guide](guide.md#11-where-data-logs-and-crash-reports-live)).
- An older daemon left running after an upgrade answers the version handshake with an error. Restart it.

## Resume is not supported by the server

Two situations look similar to the user.

1. **The server ignores byte ranges.** The file is downloaded as one stream, and the partial file is replaced from the first byte each time. There is nothing to fix. Pausing and resuming start the file again.
2. **The server stopped honouring ranges, or the file changed.** The task stops with the status `restart_required`, and the error says "server stopped honoring byte ranges" or "remote validators changed". Partial data is kept, but it cannot be combined with the new file. Choose **Restart** from the File menu, or run `xdm-cli daemon restart TASK_ID`. The direct CLI equivalent is to run the same command with `--restart`.

XDM will never splice bytes from a changed remote file. That is intentional.

## One-time links

Some links work once. XDM uses a HEAD request for the file-size check, which never uses up such a link. The download itself is a normal GET, so the first attempt uses the link. If the transfer fails and the link is spent, a retry will get an HTTP error. Get a new link from the site and add it again.

## Not enough disk space

Before writing data, XDM on Linux checks the free space on the destination volume. If the file cannot fit, the task fails with "not enough disk space (or quota) for the download" and the partial file is kept. Free some space and resume the task; the task keeps its partial data. Downloads to a partials folder on another volume are checked on the final volume too.

## Proxy authentication (407)

A proxy that requires a login returns 407. The task then waits for credentials and the status shows authentication required.

```sh
xdm-cli daemon proxy-login USER PASSWORD_FILE
```

Use `-` in place of the file to type the password on standard input. Storing credentials requeues tasks that were waiting on a 407. Clear them with `xdm-cli daemon proxy-login-clear`. Do not put the password in the proxy URL; XDM refuses such URLs.

A server login (401) is handled separately with `xdm-cli daemon login save ...` or `xdm-cli daemon auth TASK_ID USER PASSWORD_FILE|- [--remember]`.

## The browser extension is not connecting

The extension sends downloads to XDM through a native messaging host named `org.xdm.browser`. If it cannot reach the host, the browser keeps the download.

1. Make sure the daemon is running (section above).
2. Check that the host manifest is registered for your browser.
   - Linux, from a checkout or tarball: `install/linux/install.sh --browser chrome` (or chromium, edge, brave). Re-run it after moving the extension.
   - Linux, Firefox: `install/linux/register_firefox_native_host.py` writes `~/.mozilla/native-messaging-hosts/org.xdm.browser.json`.
   - Windows: the installer writes the registry entries for Chrome, Chromium, Edge, Brave and Firefox under `HKCU`.
   - Debian package: installs the Firefox host manifest under `/usr/lib/mozilla/native-messaging-hosts` and Chromium-family manifests under `/etc` (Chromium, Chrome and Edge), using the packaged extension ID.
3. For an unpacked Chromium extension, the extension ID depends on its folder path. If you move the folder, register the host again.
4. The Chromium extension ID must match the manifest's allowed origins. The packaged zip has the fixed ID `pilmmicffpeccbaaienmhdohpnnobmcf`.
5. The Chromium popup switch **Send eligible browser downloads to XDM** must be on. It is off by default.

Once the host is registered, `xdm-cli daemon version` should work from the terminal, which confirms the daemon is reachable.

## Media jobs fail because ffmpeg is missing

Media jobs need `ffprobe` and `ffmpeg` on `PATH`. XDM does not bundle them. If either is missing, the job fails with the operating system's "program not found" error. Install an ffmpeg build, confirm `ffmpeg -version` and `ffprobe -version` work in the same terminal, and start the daemon or GUI from that environment.

## Windows: "connection refused"

The task shows the failure class "connection refused". This means the server or a proxy actively refused the connection. It is not a timeout.

- Confirm the address and port in the browser.
- If you use a proxy, check **Downloads → Connection…**. A wrong proxy address produces the same error. Also check the system proxy setting.
- On Windows, a full connection backlog on the server can also appear as refused after XDM's retries.

## Logs

- `logs/xdm-daemon.log` and `logs/xdm-gui.log` in the data directory. Each file is rotated at 5 MiB, and three files are kept.
- Raise detail for a single session with `XDM_LOG=debug` before starting the daemon or GUI.
- Crash reports are in `crash/crash-*.txt`. The newest 10 are kept.
- Logs and crash reports are redacted: URL user names and passwords, query strings, cookies, authorization headers and secret-looking names are replaced with `<redacted>`.

## Support bundle

```sh
xdm-cli daemon diagnostics bundle.zip
xdm-cli daemon diagnostics ./bundle-dir
```

The output path must not exist, or, for a directory, must be empty. The bundle contains:

- `version.txt`: XDM version, operating system, and the data directory path;
- `database.txt`: database integrity check, schema version, and backups;
- `settings.txt`: stored settings;
- `tasks.txt`: the 1,000 most recent tasks, with URL query strings and user information removed;
- `logs/` and `crash/`: the redacted log and crash files.

Request headers, cookies, login data and proxy passwords are not included. Review the bundle before you share it.
