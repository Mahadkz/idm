# Browser and media follow-up audit

This supplements [the desktop investigation](idm_reverse_engineering.md). It records the supplied Integration Module 6.43.1 and native IDM 6.43 build 15, before application implementation. Component tests, native static decompilation, and original-application execution under Wine are distinct evidence levels.

## Extension component experiments

[The reproducible probe](analysis/extension_contract_probe.mjs) executes the original background JavaScript inside a Node VM, with controlled Windows browser API mocks and simulated host replies. The controller is observed through the VM environment; the original file is not rewritten. Initialization and site configuration are mocked, so these experiments establish specific producer and acknowledgment paths, not successful browser integration or a complete negotiated configuration.

[Results](analysis/browser_media/extension_contract_results.json): 16 cases, 15 passed assertions and one bounded observation.

| Case | Result |
|---|---|
| User-requested context link | Type 14 message includes dummy cookies and source context |
| Accepted browser handoff | Type 13 request; acknowledgment status 3 cancels browser download |
| Rejected browser handoff | Browser resumes and filename callback is released exactly once |
| Host unavailable before sending | Browser resumes and filename callback is released |
| Unicode filename | Independent UTF-8 byte parser confirms the inner declared string lengths |
| Event registration | Downloads and network listeners registered against the mock APIs |
| MP4, opaque URL with WebM MIME, HLS, DASH, MP3 | Detection emits type 13; manifests use a different action discriminator from ordinary media |
| Configured MIME exclusion | Matching opaque WebM request is suppressed |
| HLS and DASH body forwarding | Simulated host request leads to content-side request and type 16 response containing field 115 |
| Blob context link | A blob URI itself is not submitted as an HTTP download |
| Disconnect after pending handoff | During a 30 ms observation, pending entry remained, with no resume or filename callback release |

The last row is not a passing recovery test and does not establish an indefinitely stuck browser download. Timer behavior, browser port semantics, and real event sequencing require a browser experiment. The new implementation should explicitly release any interception pending at disconnection or acknowledgment timeout.

### Manifest response-body contract

For synthetic request IDs 803 and 804, the detected manifest request was acknowledged by a simulated host reply with status 1. The background requested body retrieval through content-port message 19. Its observed tuple contained request ID, frame/context discriminator, URL, method, request headers, and body slot.

The content response path then emitted native inner message type 16, variant 1, flags 48, numeric arguments `[requestId, -1]`, and field 115 containing the manifest text. Both HLS and DASH round trips were asserted. These numbers have meaning within this observed source path; they do not establish all meanings of these flags or a full legacy protocol specification.

This is evidence for an important separation: media detection, obtaining response text in browser context, and desktop stream processing are different operations. Cookies and source headers are part of the context. Discovering an `.m3u8` URL alone is not evidence that the native engine has received all required playback context.

## Original application media experiments

A local HTTP fixture served six-second synthetic H.264/AAC media, a combined HLS playlist, a master HLS playlist with two resolutions and separate audio, and a DASH manifest with two video representations and separate audio. No external site or real account was used.

The media fixture producer was checked with FFprobe: the multi-quality manifests expose 640×360 and 320×180 video and AAC audio. This validates the fixtures, not IDM's ability to select or merge those tracks.

The original `IDMan.exe /d URL /p DESTINATION /f NAME /n` path was tested under Wine:

| Input | Observed output |
|---|---|
| Direct MP4 | 1,390,449 bytes; identical to source; FFprobe identified H.264 video and AAC audio |
| Combined HLS media playlist | Exactly the 197-byte manifest; no segment assembly through this command path |
| Multi-representation DASH manifest | Exactly the 2,349-byte XML manifest; no segment assembly through this command path |

MP4 SHA-256: `b313f4f483f84ec68e564b0b77e85806a8f65b89d1164a3c13b24ba237dceb89`.

These observations do **not** show that IDM lacks streaming support. They show that ordinary manual URL download and extension-driven streaming capture cannot be treated as the same path. [Output checks](analysis/browser_media/manual_media_downloads.json) preserve hashes and FFprobe results. [Request logs](analysis/browser_media/http_requests.jsonl) include both IDM traffic and fixture validation traffic; clients must be distinguished before attributing segment requests to IDM.

Language catalog entry 21130 instructs the user to select a desired quality in the website player before that quality appears in the download panel. This suggests that quality discovery can depend on played representations. Entries 21005 and 21006 expose audio-only and video-only tracks, while 20939 covers protected-stream refusal. These are catalog observations, not tested track selection or muxing behavior.

## Native decompilation on Linux

Portable Ghidra 12.1.4 and a private JDK 21 were used for actual native-code analysis. No Windows installation was needed for this static stage. Hermes was not used: these are x86 PE binaries, and the extension contains ordinary JavaScript rather than Hermes bytecode. REA's local checkout was inspected earlier but its runtime was not executed.

[Native function inventory](analysis/browser_media/native_function_inventory.json) records 83 host functions, nine selected mux/conversion functions, and three selected parser functions. These are selected native subsystems, not a full decompilation of IDMan.exe. Recovered proprietary pseudocode is kept outside the project in temporary analysis storage.

### Native messaging host

The host main routine at `00402830` supports this structural interpretation:

1. Resolve pipe process-identification APIs, with a process-image fallback.
2. Check the `Local\IDMEventMonitor` mutex. A branch can inspect installation status and attempt a COM launch.
3. Determine the Windows session ID and construct `\\.\pipe\IDMNetworkMonitor.<session-id>`.
4. Open standard input/output and the session-specific named pipe.
5. Set up events, semaphores, and worker threads for asynchronous forwarding.
6. Send an internal startup record containing an identified browser/parent process ID; function `00403530` traces the identity construction.
7. Convert incoming browser JSON-string content into the internal pipe representation, and escape internal responses back into browser JSON strings.

Functions `00402420` and `00402550` participate in JSON string escaping/unescaping. Functions `00403410` and `004034a0` handle I/O. The internal representation has framing adjustments beyond the browser's standard four-byte length prefix; the entire desktop pipe protocol is not yet specified or interoperability-tested.

This is more specific than merely observing `ReadFile` and `WriteFile` imports: the browser-facing host and desktop-facing pipe are separate protocol boundaries. The standard browser framing does not by itself describe what the Windows monitor expects.

The original host was also executed under Wine using a controlled Python framing client, rather than a browser. It received a stock extension initialization string and exited with no stdout response. [Probe result](analysis/browser_media/native_host_result.json) records this failed integration attempt. Static code reveals multiple early exits and environment dependencies, but the exact branch taken in that execution has not been traced. Do not label the native integration working or attribute its failure to a single cause.

### Parser and muxer

`IDMVMPrs.dll`'s exported `Parse1` entry at `10002fc0` allocates a parser object, advances a supplied text pointer toward an XML opening delimiter, invokes an internal parser, and returns an object through an output pointer on success. `GetMPDParserInfo` returns two stored words. Their exact types and interpretation require further tracing.

`idmvconv.dll` has distinct TS-to-MP4, TS-plus-AAC-to-MP4, TS/TS-to-MP4 or MKV, FLV-to-MP4, and WebM audio normalization paths. The selected TS/TS-to-MP4 wrapper reads three zero-terminated 16-bit strings. Decompiler prototypes and parameter ordering are inferred; they have not been validated as a callable ABI. A later standalone call with guessed signatures under Wine was inconclusive: conversion faulted; TS+TS mux emitted a file with H.264/AAC stream headers but decoding errors and zero video duration. This says nothing definitive about the integrated application pipeline. [Probe results](analysis/runtime_final/native-media-results.json).

The evidence supports separate parser and muxing subsystems. It does not establish all supported manifest profiles, arbitrary MP3 conversion, every quality from every site, or live-stream behavior.

## Final bounded audit and platform validation

- Multiple queue jobs, a one-file queue limit, retry/malformed-range/changed-validator behavior, text import/export, dynamic search/batch/limiter dialogs, and ordinary restart persistence were exercised in the [final runtime pass](analysis/runtime_final/index.json).
- The installed payload contains distinct Chromium MV3, Edge MV2, and Firefox MV2 packages; exact hashes, manifests, and hook counts are in [browser_packages.json](analysis/runtime_final/browser_packages.json). Firefox does not register `downloads.onDeterminingFilename` in its background script; its `webRequestBlocking` path requires separate browser tests.
- Real browser interception, native-host negotiation, media selection, overlay behavior, muxed output and Windows monitor/driver behavior remain platform/product validation, not established behavior of this Linux audit.
- Full queue/worker internals and recovery-file formats are not needed for a clean-room design. The [completed audit](idm_audit_completion.md) bounds these uncertainties and the [architecture plan](architecture_spec.md) defines independent implementation tests.

Rust remains the proposed implementation language. Application code remains behind the user's requested confirmation step.
