# Download File Info (IDM resource 4, 380x128 dlu)

References: docs/analysis/runtime/download-info.png (local analysis data, not shipped). Crop 356,359 570x208. Dialog units to pixels: x*1.5, y*1.625 (measured 570 px wide in the screenshot, so factor 1.5 confirmed).
Font: Wine Tahoma (assets/fonts/tahoma.ttf), 11 px.

| element | source | dlu x,y,w,h | px x,y,w,h (ours) | class | confidence |
|---|---|---|---|---|---|
| URL label | res 1277 | 5,5,54,10 | 7,5,77,16 right aligned | Observed | high |
| URL edit (grey, read-only look) | res 1007 / screenshot | 64,3,244,14 | 96,5,366,23 | Observed; ours is editable (Inferred) | high |
| Category combo | res 1209 | 64,22,116,64 | 96,34,174,23 | Observed | high |
| New category "+" | res 1697 | 183,21,16,14 | 274,34,24,23 | Observed | high |
| Save As combo | res 1008 | 64,39,223,86 | 96,63,334,23 (arrow inside) | Observed | high |
| Browse "..." | res 1608 | 292,39,16,14 | 438,63,24,23 | Observed | high |
| Remember group box + check | res 1817/1609 | 64,56,244,32 | 96,98,366,45 | Observed | high |
| Remembered path edit (grey) | res 1815 | 69,69,232,14 | 103,112,348,23 | Observed | high |
| Description | res 1009 | 64,91,244,14 | 96,148,366,23 | Observed | high |
| Download Later / Start Download / Cancel | res 1004/1006/2 | 64/149/234,110,74,14 | 96/223/351,179,111,23 | Observed | high |
| File glyph | res 1094 | 333,28,20,20 | 500,45,30,36 | Observed position, original art | medium |
| Size text | res 1101 | 311,57,64,10 | 466,93,96,16 centered | Observed | high |
| Preview button | res 1173 | 319,78,50,14 | not drawn (media only) | Unknown state | - |
| Error text column | - | - | 468,120,96,80 | Inferred | - |
| Duplicate prompt overlay | - | - | 40,40,490,140 | Inferred (IDM prompt not captured) | - |
| Default button focus rectangle | screenshot | - | not drawn | Observed, not reproduced | - |
