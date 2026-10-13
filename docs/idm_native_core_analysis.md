# Native core follow-up

The supplied `IDMan.exe` was imported into portable Ghidra 12.1.4 on Linux. Automatic analysis reported success after 236 seconds. A string-reference script selected 19 functions for decompilation; all 19 returned decompiler output. This extends the runtime download experiments and native host/parser/muxer audit. It is not a complete reconstruction of the download engine.

No original PDB was available. Analysis reported an exception-metadata disassembly error at `00450052`, and some functions contain unresolved stack/register information. Function addresses identify this particular binary; inferred structures and prototypes require validation. Proprietary pseudocode remains in temporary private storage.

[Persisted inventory](analysis/core/native_core_inventory.json) contains binary identity, function addresses, call references, selected HTTP/path strings and analysis limitations. [The tracing script](analysis/TraceCoreStrings.java) can repeat the selection through Ghidra headless analysis.

## HTTP request and response paths

| Address | Observed native structure | Scope |
|---|---|---|
| `0058b320` | Constructs bounded, open-ended and zero-origin HTTP Range headers using 64-bit values | Request construction identified; internal worker fields remain unnamed |
| `005ae8e0` | Another substantial request-building path with the same Range forms and multiple branches | Purpose of separate path and full branch conditions remain unresolved |
| `00582780` | Iterates header lines; handles content length and Content-Range start/end/total values | Parser path identified; strict validation behavior requires caller tracing and malformed-response tests |
| `00581c80` | Handles Content-Range and ETag among response headers; writes parsed values into object fields | Stored ETag is not proof of resume validator enforcement |
| `0058c7f0` | Broad response processing, including conditional handling of Accept-Ranges and a search for `none` | No universal assumption that missing Accept-Ranges disables segmentation |
| `005dc710` | Parses captured request headers; branches consume Range and exclude several conditional headers, including If-Range, from a general copy path | An If-Range string reference does not establish that resumed engine requests send it |

This connects actual network behavior with native range-generation and response-parsing functions. The earlier fixture run began with GET and subsequently issued parallel Range requests; it did not send HEAD. A fresh engine can use HEAD as an optimization but needs a range-probe/fallback path when servers reject HEAD or provide incomplete metadata.

The existing request captures contain overlapping boundaries. These traces have not yet established the overlap purpose, worker splitting algorithm, connection ownership rules or stall-reallocation policy. Do not translate unnamed object fields into a purported complete segment scheduler.

## Partial-state and recovery paths

| Address | Observed native structure | Remaining work |
|---|---|---|
| `0046eca0` | Reads temporary-path configuration, obtains a user name, constructs a per-user DwnlData path and passes it to a helper | Resolve helper operation and all path-migration behavior |
| `0056fb90` | Builds a `.log` pathname using a filename-derived stem and numeric identifier | Recover metadata contents and lifecycle; filename alone is not a checkpoint schema |
| `005634de` | Recovery-related routine with old-download rebuild diagnostics, multiple file opens and file-position operations | Trace file record layout, validation, writes and transition to assembly |
| `00562030` | Related old-download rebuild path with file open/seek operations | Determine variant relationship and migration conditions |

Under Wine, actual pause inspection found separate segment files and a task `.log`. Forced process termination, restart and resumed transfer produced the exact expected 64 MiB hash. The native paths above support this storage/recovery interpretation but do not establish power-loss durability, journal ordering or the complete assembly algorithm.

The requested fresh sparse-output plus `.part.meta` design is an independent engineering choice. It is not an observed implementation detail of the tested IDM run.

## Closure and remaining validation

The later controlled fixture pass exercised one malformed Content-Range variant, transient 503, permanent 403, changed remote validators, one-file queue concurrency, and graceful settings/task persistence. The original recovered exact fixture hashes or entered an explicit error/restart path; see [final evidence](analysis/runtime_final/index.json). Exact worker ownership, retry policy, and recovery-log bytes remain unknown. Those are independent design and test responsibilities in the [architecture plan](architecture_spec.md), not reasons to copy proprietary internals. Actual native Windows monitor/driver and browser integration remain unverified.
