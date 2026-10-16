# Main window reconstruction worksheet

Reference: IDM 6.43 screenshots captured under Wine (`docs/analysis/runtime/task-context-menu.png`, `idman-start.png`,
`runtime_final/search-bar.png`, `video-category-filter.png`). They are local analysis data and are not shipped.
Crop used for every comparison: window rectangle (241,252)-(1039,675) of the 1280x900 capture, i.e. 798x423 px
client area (the captures with a title bar are cropped below it). XDM is rendered at the same 798x423 size by
seeding `gui-layout.json` (`width 798, height 423`) and compared with `tools`-free PIL scripts (side-by-side,
50 % overlay, difference). Coordinates below are window-relative pixels.

Confidence: O = observed in a screenshot, I = inferred (implementation choice), U = unknown.

| Element | Source | x | y | w | h | Notes | Conf |
|---|---|---|---|---|---|---|---|
| Menu bar | task-context-menu.png | 0 | 0 | 798 | 21 | Items Tasks File Downloads View Help; text Tahoma 8 (11 px); item width = text + 14. "Registration" omitted (legal boundary). | O |
| Toolbar | same | 0 | 21 | 798 | 69 | Large buttons, 74 px pitch, icon 32 px centred 24 px below bar top, label baseline-centred at y=74, disabled = grey icon + grey label. Start/Stop Queue carry a 10 px dropdown arrow. | O |
| Toolbar order | same | | | | | Add URL, Resume, Stop, Stop All, Delete, Delete Completed, Options, Scheduler, Start Queue (v), Stop Queue (v). "Tell a Friend" omitted. | O |
| Categories pane | same | 7 | 89 | 142 | 332 | Frame, title "Categories" with a close X at right, tree rows 16 px, expander box 9 px, child indent 19 px. | O |
| Pane gap / margins | same | | | | | left 7, gap 5, right 8, bottom 3-6 px. | O |
| Table frame | same | 154 | 89 | 637 | 332 | Header 19 px, rows 17 px, 1 px grid lines (vertical and horizontal), white background. | O |
| Column widths | same | | | | | File Name 151, Q 20, Size 75, Status 65, Time left 75, Transfer rate 95, Last Try Date 65 (text truncated with an ellipsis like IDM), Description rest. | O |
| Selected row | same | | | | | IDM: blue fill, white text, thin orange top edge (not reproduced). | O |
| Context menu | task-context-menu.png | pointer | pointer | 175 | rows 17 / separators 9 | Open, Open with..., Open folder, Move/Rename (Ctrl-M), Redownload, Resume Download, Stop Download, Refresh download address, Remove, Add to queue >, Delete from queue, On Double click >, Properties. | O |
| Scroll bar | same | | | | | Horizontal scroll bar under the table in IDM; XDM shows Slint's own list scroll bar. | O |
| Status bar | not in IDM | 0 | 403 | 798 | 20 | Message + queue state. XDM needs a place for action results. | I |
| Window/layout persistence | n/a | | | | | `gui-layout.json`: size, position, sidebar width/visibility, toolbar style, column order and widths, expanded tree nodes. | I |

Dialog-unit geometry does not apply: the main window is not a dialog template in `idm_ui_resources.json`.
Hover colours, menu popups for the toolbar dropdowns, the cascade-on-hover behaviour and the drag-to-reorder
header gesture are Inferred (no screenshot shows them). Tray icon: Unknown/not built (see report).
