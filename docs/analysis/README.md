# Local investigation evidence

These files describe the supplied IDM build. They are investigation artifacts, not source material or assets to embed in the open-source alternative. Runtime observations are specific to Wine on Linux; executable resources may include inactive or historical features.

The static inventory and limits are explained in [the investigation report](../idm_reverse_engineering.md). The complete dialog catalog is [here](idm_dialog_catalog.md). Runtime captures and request logs are in `runtime/`.

`build_feature_coverage.py` emits the complete resource-identity ledger. `build_behavior_review.py` adds manually reviewed semantic sections for every shipped language entry and detailed state-transition requirements. Both accept `--check` to validate current artifacts. The behavior review pins its catalog digest and rejects section gaps/overlaps or missing cited keys; semantic association does not establish a runtime caller or a passing acceptance test.

`build_control_review.py` emits individual control semantics, including explicit manual bindings, source styles/layout, contract candidates and unresolved caller/default conditions. `check_feature_mapping.py` cross-checks all generated mappings, desktop identities, browser inventory counts/source hashes, detailed contract IDs and evidence links. [Mapping status](../feature_mapping_status.md) defines the completed inventory scope and remaining runtime/platform work.

To repeat static extraction into an empty temporary directory, with Python and `pefile` available:

```bash
python3 docs/analysis/extract_idm_installer.py IDM/idman643build15.exe /tmp/idm-repeat
python3 docs/analysis/inspect_pe_resources.py /tmp/idm-repeat
```

Do not put recovered proprietary binaries, images, help files, or scripts into the new implementation. The extraction tool leaves them in the requested analysis directory.

The local HTTP fixture serves deterministic 64 MiB content, with an endpoint honoring byte ranges and another deliberately returning a full response regardless of Range. Its logs identify what the original application requested:

```bash
python3 docs/analysis/http_fixture.py /tmp/idm-http --delay 0.25
```

The fixture prints its selected port. `--port` can preserve a port during server-restart experiments. `--delay` controls the delay after each 32 KiB body block. Request logs and output hashes must be inspected before assigning a test result.

Screenshots made before the private display gained an Openbox window manager lack title bars and may have incomplete repaints. They are retained for event evidence; captures after that point provide better UI observations. Neither establishes exact native Windows pixels or Windows driver behavior.

Browser/media results are explained in [the follow-up](../idm_browser_media_analysis.md) and indexed in `browser_media/index.json`. To repeat the component probe against a temporary extraction of the supplied original extension:

```bash
node docs/analysis/extension_contract_probe.mjs /tmp/EXTENSION_SOURCE /tmp/idm-component-results
```

This uses mocked browser APIs and simulated host replies. It does not prove working browser or OS integration. The output separates assertions from the bounded pending-disconnect observation.

The media HTTP fixture accepts a directory of synthetic media assets and an output directory for request logs:

```bash
python3 docs/analysis/media_fixture.py /tmp/SYNTHETIC_MEDIA /tmp/idm-media-http
```

It serves only on loopback and supports byte ranges. `/auth/` requires the dummy cookie `audit_cookie=local_fixture`; no real credentials are used. The saved asset inventory identifies the tested fixtures. Fixtures and downloaded proprietary components remain outside the implementation.

The [final runtime evidence index](runtime_final/index.json) contains queue, retry, malformed-range, changed-validator, import/export, restart-persistence and installed-browser-package results. `fault_fixture.py` serves disposable 8 MiB range fixtures with 503, 403, malformed `Content-Range`, and switchable validator/content variants. Its `/control/version2` endpoint is for this isolated local audit only. `win_ui_probe.c` is a Wine/Win32 UI inspection helper; `native_media_probe.c` is an inconclusive standalone export probe. Neither is application implementation code. The [audit completion report](../idm_audit_completion.md) states which observations are verified and which require platform tests.

`ExportNativeAudit.java` is a Ghidra headless script accepting a private output directory and function-name filters, or `ALL`. It exports selected pseudocode and metadata for investigation. The persisted native inventory omits pseudocode files; do not copy decompiler output into application source. Inferred signatures are not a validated ABI.

`TraceCoreStrings.java` accepts a private output directory and selects native main-executable functions referencing HTTP range/recovery strings. The persisted structural inventory is `core/native_core_inventory.json`; [the core follow-up](../idm_native_core_analysis.md) distinguishes identified paths from unresolved algorithms. All original string-reference records and pseudocode stay in private temporary storage.
