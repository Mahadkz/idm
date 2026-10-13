# IDM feature mapping: scope, results and remaining verification

Reviewed 2026-10-12. The supplied-artifact inventory and feature mapping now cover every extracted dialog, control, static menu command and catalog entry. This closes the inventory/semantic-mapping milestone for the supplied build. It does **not** close implementation parity, recover every native caller, or verify every original runtime transition. One generic framework-looking control retains an unresolved product meaning.

| Evidence surface | Mapping result | Review artifact |
|---|---|---|
| Desktop resources | 105 dialogs, 1,055 individual control records, including 275 manual field/action/collection bindings | [Individual control review](idm_control_review.md), [JSON](idm_control_review.json) |
| Static menu trees | Two trees, 112 nodes including containers/separators, 75 command occurrences | [Feature coverage ledger](feature_coverage.md), [JSON](feature_coverage.json) |
| Dynamic labels and messages | All 1,604 language entries manually read and associated with 128 semantic sections; 156 PE string references retained | [Dynamic behavior review](idm_behavior_review.md), [JSON](idm_behavior_review.json) |
| Dynamic state transitions | 39 detailed original-text → independent requirement → acceptance contracts | [Dynamic behavior review](idm_behavior_review.md) |
| Browser/native/media integration | 62 detailed lifecycle, hook, message, context, player and package contracts | [Browser contracts](idm_browser_behavior_contracts.md), [source inventory](analysis/idm_browser_source_inventory.json) |
| Network, acceleration, storage, queues and scheduler | 69 detailed contracts: 48 network/storage/throughput and 21 queue/scheduler | [Core and queue contracts](idm_core_queue_contracts.md) |

There are **170 detailed behavioral contracts**, plus the individual control and menu records. Contract counts are specification coverage, not passing tests or unique product-feature counts: some contracts intentionally examine different facets of the same workflow. The resource ledger retains 84 feature families and 26 non-UI paths.

## Original behavior and replacement boundaries

Each contract identifies original evidence, the current independently written Rust/extension behavior or gap, and an acceptance outcome. Evidence stays distinguished as resource/source text, shipped help, original Wine observations, extension mocks, or XDM tests. New UI assets, translations, schemas and implementations are independent; no original binaries, images or recovered pseudocode are application dependencies.

Some behaviors merit independent substitutes: commercial registration becomes open-source project/license information; Windows monitor drivers and legacy IE plugins become supported browser/native-host integration; proprietary task-list/project/update formats and endpoints require explicit compatibility or independently documented replacements. These are visible differences, not silently counted as equivalent functionality. DRM decryption remains outside scope, consistent with the original protected-stream refusal surface.

The original quality catalog can depend on which representation the player requested (L21130). XDM's clear-manifest enumeration is an independent improvement, with public fixture coverage; it does not prove every site's resolutions or audio formats are available. Authentication, request lifetime, browser routing, codecs and source protection constrain available downloads.

## Precise remaining unknowns

- **D30721/C1** is a ListBox in a generic New template. Its identity, class, dimensions and neighboring OK/Cancel/Help controls are known; no reachable IDM product caller or item population is established. It remains `legacy.generic`, rather than becoming an invented product feature.
- **D236/C2** is automatic-capture guidance text. The role is clear from the dialog and catalog, but its exact string producer and triggering path remain untraced.
- **D286/C11** matches the completed-file drag icon through shipped `CompleteDlg.htm` and the screenshot/layout. The review retains source hashes and H/layout evidence; current-build visibility and drag event wiring are untested.
- **1,136 dynamic references** have no caption/command/control-ID association in the static resource ledger. They now have reviewed semantic sections, but section classification does not identify their native runtime callers.
- Original defaults, enable/visibility dependencies, pause-versus-stop distinctions, retry/queue-completion policies, dynamic host configuration and numeric native-protocol fields remain unverified where the detailed contracts say so. Historical/empty templates are not proof of a reachable feature.
- No native Windows system was available. Browser registration/ACLs, installed Chromium/Edge/Brave/Firefox capture, tray, shell actions, wake/power behavior and native GUI performance still need platform validation. Wine and cross-compilation evidence remain labeled as such.

## Implementation order derived from the mapping

1. **Transfer and ownership correctness:** single-connection range resume, safe response reuse/one-shot handling, method/body/context replay, authenticated media, no-clobber publication, and late-acceptance/cancel reconciliation. Browser acceptance must preserve exactly one owner; file recovery must never mix representations.
2. **Queue equivalence:** independent named-queue start semantics, persistent manual ordering, distinct network/task retry policies, synchronization, per-queue recurrence and stop times, quota windows and completion precedence. OS actions use fake adapters in tests before native validation.
3. **Browser breadth and media workflows:** Firefox adapter, profile health, page-context body capture, player/frame association and overlay lifecycle, navigation cleanup, selected/all-page links, quality/audio/subtitle selection, live stop-and-save, codec/container and separate-track fallback.
4. **Desktop workflows and preferences:** virtualized columns/tree, multi-selection and contextual commands, move/rename/open/drag, configurable search, categories/site/path rules, batch generation, clipboard/drop target, settings, tray, translations, sounds and scanner hooks.
5. **Grabber and distribution:** bounded authenticated/offline crawl, project/template recovery and scheduling; native Windows/Linux installation/update diagnostics, signed independent distribution, and measured release-build CPU/RAM with large task lists. macOS adapters follow later.

Each stage uses the contract acceptance scenarios; existing prototype tests only cover their documented subset. [Implementation status](implementation_status.md) records the current build and these gaps.

## Reproducible mapping verification

Run:

```sh
python3 docs/analysis/build_feature_coverage.py --check
python3 docs/analysis/build_behavior_review.py --check
python3 docs/analysis/build_control_review.py --check
python3 docs/analysis/check_feature_mapping.py
```

The combined checker verifies generated artifacts, exact source control IDs/classes/styles/bounds/captions, matching dynamic-reference identities, contiguous detailed contract IDs, browser inventory counts and linked evidence files. When the private extracted browser files are present it also verifies all five recorded source hashes. These are documentation/provenance checks; they do not count as functional application tests.

All work in this mapping pass was read-only investigation or documentation generation. No GUI was launched on desktop 1 and no power action was executed. Claude Code Haiku and Sonnet provided bounded read-only consistency/control reviews; source evidence and conclusions were independently checked before incorporation.
