# Download progress (IDM resource 2, 353x250 dlu) and Speed Limiter (resource 91)

References: range-progress-before-pause.png, crop 370,260 535x408. The screenshot is a partial Wine render: labels and tabs are not visible. Measured horizontal factor 1.515 (535 px) vs 1.5 for the other dialogs; vertical 1.625.

| element | source | dlu | px (ours) | class |
|---|---|---|---|---|
| Info panel | res 1760 | 7,5,338,101 | 10,29,512,143 | Observed bounds |
| Tab strip (Download status / Speed Limiter / Options on completion) | not visible in screenshot, real IDM shows tabs | - | 10,8 | Inferred |
| URL line | - | - | 14,34 | Inferred |
| Status text (blue) | screenshot | - | 94,54 | Observed |
| Values column | screenshot (Downloaded/rate/time text start x=497) | - | x=127, rows y=73,90,106,122 | Observed |
| Resume value | screenshot ("Yes") | - | 203,140 | Observed |
| Left labels File size/Downloaded/Transfer rate/Time left/Resume capability | brief | - | x=14 | Inferred (invisible in screenshot) |
| Progress bar | res 1146 | 7,111,338,10 | 10,180,512,16 | Observed |
| Hide details / Pause / Cancel | res 1178/1158/2 | 15/203/277,126 | 22/309/420,206, h 24 | Observed |
| Caption "Start positions..." | res 1305 | 21,143,306,9 | 31,232,473,16 | Observed |
| Segment bar | res 1179 | 7,153,338,10 | 10,249,512,16 | Observed |
| Connections table | res 1177 | 7,168,338,79 | 11,274,511,126 | Observed; mauve row colour sampled |
| Speed Limiter tab controls | res 91 | see json | shifted 30 px by tab strip | Observed geometry, tab placement Inferred |
| Options on completion tab | - | - | - | Inferred (only XDM-supported actions) |
