# How IDM's browser extension captures video (and what XDM does instead)

Source studied: the unpacked IDM Integration Module 6.43.1 Chrome extension (read-only, treated as untrusted
data, never executed). The code is minified and the extension is a thin sensor: almost every decision is made
by the native IDM application on the other end of a native-messaging port. Everything below is described in
our own words; XDM reimplements the behavior, not the code (docs/ENGINEERING_RULES.md rule 19).

## 1. Pieces and who talks to whom

| Piece | Role |
| --- | --- |
| `background.js` (service worker) | Owns the native-messaging connection. Speaks a compact positional protocol: each message is an array whose first element is a numeric opcode. Registers `webRequest` listeners (before-request with request bodies, before-send-headers, headers-received, response-started, completed, error, before-redirect) for the URL patterns and resource types the native app asks for, plus `downloads`, `webNavigation`, `contextMenus`, `proxy` and declarativeNetRequest hooks. |
| `content.js` (all frames, document_start) | Finds media elements, tags each with a numeric id, reports their geometry and visibility, and relays messages between the page world and the background script. |
| `document.js` (web-accessible, injected into the page, main world) | Wraps `XMLHttpRequest.open/setRequestHeader/send`, `window.fetch`, and the `Response` readers (`text`, `json`, `arrayBuffer`, `blob`, the body stream) so that responses to selected URLs can be read by the extension. |
| `captured.html/js` | Landing page for natively registered download links (a main-frame navigation matching a native pattern is redirected there by a declarativeNetRequest session rule; the URL fragment is forwarded to the native app). It is not a media list. |
| `debug.js`, `welcome.js` | Diagnostics and first-run page. |

There is no site name in the extension (no "youtube" string): per-site rules live in the native app.

## 2. How the URL pattern reaches `document.js`

1. The native app tells the background script, over the port, which request URL patterns matter and which
   hooks to enable. The background script keeps these as the filters of its `webRequest` listeners (`O` for the
   observed patterns, a second list for media types).
2. The content script forwards a configuration to the page world with `window.postMessage`. The message is an
   array whose first element is the magic number `1229212978`. Its payload carries: a per-page name prefix, the
   attribute name used to tag elements, the names of the properties the hooks must patch (so the page script
   contains no recognizable API names), two flags (hook XHR, hook fetch), the URL regular expression `M`, and
   a second regular expression that extracts a short identifier from a captured value.
3. `document.js` only acts on messages whose `origin` equals the page origin. If the XHR flag is set it
   replaces the XHR methods; if the fetch flag is set it replaces `fetch` and the `Response` readers. Sending a
   configuration without the flags restores the originals. After applying it, it reports which hooks are active
   (`1229212979`).
4. Further control messages: `1229212981` (read a property or DOM path from the page and answer with
   `1229212982`; used to read player state such as a configuration object), `1229212983` (patch one more
   property path), and an initial "ready" message `1229212977`.

## 3. Message types seen on the page bridge

| Direction | First element | Meaning |
| --- | --- | --- |
| content to page | 1229212978 | install or remove hooks, with the URL regex and flags |
| content to page | 1229212981 | evaluate a path against `window` or an element, reply with the value |
| content to page | 1229212983 | install an extra property hook |
| page to content | 1229212977 | page script loaded |
| page to content | 1229212979 | which hooks are active |
| page to content | 1229212980 | captured response: `[id, short identifier or null]` |
| page to content | 1229212982 | answer to a property query |

The content script turns these into native opcodes (for example 46 for hook status and 36 for a captured
response); other opcodes carry element lists, page URL and referrer, video element counts, click coordinates and
window geometry.

## 4. Correlating page requests with `webRequest` events

A page script cannot see the headers the browser finally sends, and `webRequest` cannot see which script call
made a request. IDM joins them with a marker:

* When a hooked XHR is opened for a URL that matches the regex, the hook allocates an id (`prefix-counter`)
  and keeps it on the request object. At `send` time it appends one extra value to the request's `Accept`
  header: a vendor media type of the form `...int.pxr-<id>` (`pfm-<id>` for fetch). If the page set its own
  Accept header the marker is added after it; otherwise a neutral default value is set first.
* The background script observes request headers in `onBeforeSendHeaders`, finds the marker, and thereby knows
  that the request with that id, carrying exactly those final headers (cookies, referer, user agent), is the
  one the page script is about to report.
* When the response body is captured, the page script posts `[1229212980, id, ...]`; the background
  script joins it to the request it saw by the same id.
* The content script also issues its own marked requests (it appends a media type `application/vnd.t1c.int-<n>`
  to Accept) when the native app wants a page re-fetched with the browser's credentials, and returns the body.

## 5. How bodies are captured and chunked

* XHR: a `load`-style listener registered at `open` time posts the response URL or a short identifier.
* fetch: the wrapper waits for the response, and if it is successful wraps each reader so that the first read
  of the body (text, json, arrayBuffer, blob) also posts a string form of the value. Binary buffers are turned
  into text through a streaming UTF-8 decoder; JSON is re-serialized.
* A response consumed as a stream is split with `tee()`: one branch goes to the page, the other is read to
  the end by the hook, which then posts the decoded text.
* The data posted to the content script is not the whole body. Each captured value goes through the second
  regular expression, and only the first capture group (a short identifier extracted by native-side rules) or
  the value itself leaves the page. Large manifests are therefore not chunked into pieces by the extension;
  the extension reports what the native-side rule extracts, and the native app re-fetches the media itself with
  the correlated request headers. Request ids are hashed from the URL plus selected header values so identical
  requests deduplicate.

## 6. What `background.js` forwards

For each observed request it collects: URL, frame and tab ids, whether the request is a main-frame
navigation, the request method, request headers (only when asked, in `extraHeaders` mode on Chromium), POST
`requestBody` (only when it is raw bytes or form data, never when it contains a file upload), response headers
and status, redirects (a POST turned into GET by a 301/302/303 is recorded as GET), and completion or error.
It also tracks frames (the parent chain), per-tab media lists, DRM-free status, the proxy configuration, and
the download events of the browser itself (`onCreated`, `onDeterminingFilename`), which it can cancel after
transferring the download.

## 7. The "Download this video" panel is drawn by the native app

The content script contains no panel UI. It keeps, per media element, a numeric id (stored in an attribute with
a random name and, in some browsers, a visible marker), observes the element with `IntersectionObserver`,
`ResizeObserver` and a `MutationObserver` on style and parent changes, and reports to the native app: the
element's bounding rectangle in CSS pixels, visibility and scale (`devicePixelRatio`, or a ratio derived from
the document width), its source address (or none for `blob:` sources), its title, and later changes
(opcodes 22 to 24). It also forwards mouse clicks and the page selection state. The native application
positions its own always-on-top panel over the reported rectangle and shows the format list, so the panel
follows resize, scroll and fullscreen only as fast as these reports arrive, and it works on any site for which
the native side holds rules.

## 8. What this means for YouTube

For YouTube the native rules match the page's own `/youtubei/v1/player` call and the `googlevideo.com`
`videoplayback` requests. The page's player JavaScript has already produced signed addresses (and, historically,
the `n` throttling parameter transformation), so IDM receives usable addresses without running site code of its
own.

Empirical check done for this project (headless Chrome for Testing 155, logged out, 2026-10-16, a public
Creative Commons video): the **web client no longer receives plain addresses**. The player response contains
formats with sizes and itags but no `url` and no `signatureCipher`; the player streams over **SABR**: `POST
/videoplayback` with a protobuf body and a UMP response. A hook of the IDM kind therefore sees the format list
(title, duration, itag, quality, contentLength) but there is no address to replay. IDM's current native rules
cannot be inspected from the extension, so how IDM obtains streams today is unknown to us.

## 9. XDM's equivalent

* Format list: the page's own player response, captured by `page_video.js` (fetch/XHR hook on
  `/youtubei/v1/player`, `movie_player.getPlayerResponse()` and `ytInitialPlayerResponse`), the same sensor idea
  as `document.js` but limited to the one endpoint and with no marker header.
* Addresses: resolved in the page, same-origin, with the page's own session data, by calling YouTube's player
  endpoint with a client profile whose response carries plain addresses (tried in order: ANDROID, IOS, MWEB).
  Measured result: the ANDROID profile returned 40 formats (muxed 360p, every adaptive video up to 2160p,
  audio), all with plain `googlevideo.com` addresses, no `n`, no `pot`, no signature, valid about six hours
  (IOS: 24 formats; MWEB and TVHTML5_SIMPLY return addresses that 403 outside the browser; ANDROID_VR,
  TVHTML5, creator and music profiles refused with LOGIN_REQUIRED or UNPLAYABLE).
  No site JavaScript runs outside the page and nothing runs in the daemon.
* Transfer constraints measured on those addresses (curl and the browser, logged out, same machine):
  `HEAD` returns 403; an open-ended `Range: bytes=0-` returns 403; a bounded range is accepted only up to a
  size limit (10,000,001 bytes worked, 33 MB did not; for a low-bitrate stream even 4 MB failed); the redirect
  from the signed address lands on a different `googlevideo.com` cache host per request. The HTTP engine
  therefore has an opt-in per-request cap, a one-byte bounded probe, and a same-site redirect allowance
  (daemon `range_cap_for`, active for `googlevideo.com`).
* **The decisive limit**: adaptive addresses obtained this way are served only for roughly the first minute
  of media. Range requests deeper into a file return 403 (itag 299, 258 MB: offset 20 MB served, 50 MB not;
  itag 140 audio: 1 MB served, 5 MB not; itag 135: 2.5 MB served, 3 MB not). Only the muxed 360p stream
  (itag 18) is served to the end. Appending the PO token that a headed browser's player sends in later
  SABR requests (an 85-byte value in the streamer context) to the ANDROID/IOS addresses did not change this;
  the MWEB and TV profiles' addresses return 403 from outside the browser. Headless Chrome's own player stalls
  after the first minute for the same reason (no attestation); a headed Chrome under Xvfb plays at any
  position because its player attaches the integrity token to its SABR requests.
* What XDM does about it: before listing a quality, the extension's background script sends a one-byte ranged
  request deep into each stream (its last byte, or 90% of the size implied by bitrate and duration) and offers
  only streams answered with 206. Result for a 10-minute video: muxed MP4 360p; for a video shorter than about
  a minute: every format. A full-length high-quality download would need a client for SABR (protobuf requests
  and UMP responses) fed with the integrity token the page's player produces; that is not implemented and was
  not attempted beyond the measurements above. A second possible route, teeing the page's own
  `MediaSource` buffers while the player runs fast, was not tried either.
* The panel is drawn inside the page (closed shadow root) instead of by the native app, so it is positioned and
  hidden by the content script itself; see `extensions/chromium/content.js`.

## 10. User-visible IDM video features the static inventory may have missed

Checked against the extension package; items marked "native" cannot be confirmed from the extension alone.

* **Panel owned by the native app** (section 7): placement, appearance and the format dialog are native; the
  extension only reports geometry. XDM draws the panel in-page.
* **Per-site rules and context menus are data**: the context menu entries (title, contexts, document URL
  patterns) are created from a list sent by the native app, so "Download with IDM" for links, "Download all
  links", "Download selected links" and "Transfer download to IDM" (localized strings in the package) exist, plus
  whatever the native side adds per site. XDM has link/selection/all-links menus; per-site menu items have no
  equivalent.
* **`captured.html`** is a registration landing page for natively registered `#command44` links, not a list of
  captured media. IDM's toolbar action here only toggles integration; the captured-media list a user sees is
  native. XDM's popup lists captured media for the tab and now also the page video's formats.
* **Refresh download address** is a native feature (the extension re-reads the page and answers with the
  current address on request). XDM refreshes addresses before handing a choice over, not after a failed
  transfer (see PRODUCTION_READINESS).
* **Hook configuration is remote-controlled** (regex, flags): sites can be added without an extension update.
  XDM's adapters are versioned code inside the extension.
* **Proxy awareness**: the extension reports the browser proxy configuration to the native app; XDM has a
  proxy handshake already.
