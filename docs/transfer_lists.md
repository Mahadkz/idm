# Portable download lists

The CLI and desktop support newline URL lists and version 1 JSON task lists. A list is bounded to 500 entries and 512 KiB; larger lists are rejected rather than silently truncated. Export currently has the same 500-task limit.

```sh
xdm-cli daemon preview urls.txt
xdm-cli daemon import urls.txt /absolute/download/folder
xdm-cli daemon import tasks.json /absolute/download/folder --start
xdm-cli daemon export tasks.json json
xdm-cli daemon export urls.txt urls
```

Preview does not need a running daemon and creates no tasks. Plain text uses one HTTP(S) URL per line. Empty lines and lines starting with `#` are ignored. Repeated URLs are skipped. Filenames come from URL paths; names unusable on Windows or Linux get a safe fallback, and filename collisions receive numbered suffixes. URL-list entries are ordinary HTTP file downloads; use a JSON media entry to preserve a selected HLS/DASH video/audio combination.

Import defaults to paused tasks. `--start` places them in the queue, subject to the queue's current running/stopped state and concurrency limit. The GUI's **Import / Export…** dialog previews names and destination paths and offers **Import paused** or **Add to queue**. The destination must be absolute and contain no `..` components. An existing output, checkpoint, assigned task destination, malformed entry, or unsafe filename rejects the entire batch before any task is inserted. Task registration and its retry receipt commit together in SQLite. Repeating the same import request ID with the same payload returns the original task IDs; changed payloads or a subsequently removed task reject that retry.

JSON preserves reusable configuration, not execution state or downloaded bytes:

```json
{
  "version": 1,
  "entries": [
    {
      "url": "https://example.com/file.zip",
      "filename": "file.zip",
      "kind": "http",
      "priority": 1,
      "connections": 8,
      "rate_limit": null,
      "video": null,
      "audio": null,
      "category": "Compressed"
    }
  ]
}
```

Priority is 0 (Low), 1 (Normal), or 2 (Immediate). HTTP connections range from 1 to 16. The rate cap is bytes per second or null for unlimited. Categories are Compressed, Documents, Music, Programs, Video, and Other. Media entries use `kind: "media"`, a `.m3u8`/`.mpd` URL, an `.mp4` filename, one connection, a null rate cap, and the chosen video/audio indices. Their URLs must still offer those tracks when downloaded.

Exports omit cookies and all stored request headers, absolute directories, browser submission IDs, and task errors. URL usernames/passwords are stripped. Query strings—including signed access parameters—remain part of the URL. Such URLs can expire or convey access, so an exported list is not inherently public. Imported authenticated downloads may need new browser capture because their request headers are not in this format. Unknown JSON fields, including header fields, are rejected. Export creates a new file and refuses to overwrite an existing one.

Validation is in `tests/integration/transfer_lists.py`, the shared parser unit tests, and the dedicated-Xvfb `tests/integration/gui_actions.py` flow.
