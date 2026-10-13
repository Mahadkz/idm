# IDM individual control review

Every extracted dialog/control individually classified. Static semantics and manually inferred layout bindings do not establish runtime behavior, defaults, callers or implemented parity. Source text is audit evidence, not XDM UI copy.

Inventory: 105 dialogs, 1055 controls; 275 manually bound input/collection/output controls; 1 interactive semantics unresolved.

Caption matches and reused tooltip IDs are contract candidates. Caller/default/visibility and action effects require separate runtime verification. Numeric Win32 styles are preserved; initial checked state is not inferred from checkbox type.

D236/C2 is capture-guidance text; the exact runtime producer remains unknown. D286/C11 is associated with the completed-file drag icon using shipped `CompleteDlg.htm` and its screenshot/layout; this is H/layout evidence, not tested event wiring. The JSON retains the help/screenshot hashes. D30721/C1 remains a generic framework-looking New-item collection with no established product caller.

## D100 — About Internet Download Manager

Target family: `app.about`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D100/C0 | 4294967295 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D100/C1 | 1180 | text-output-or-label | Internet Download Manager | S: explicit resource caption |
| D100/C2 | 1262 | text-output-or-label | Version: | S: explicit resource caption |
| D100/C3 | 1161 | action | Check for Update | S: explicit resource caption |
| D100/C4 | 1286 | group-container | This product is licensed to: | S: explicit resource caption |
| D100/C5 | 1181 | text-output-or-label | Unregistered Version | S: explicit resource caption |
| D100/C6 | 1085 | action | Purchase | S: explicit resource caption |
| D100/C7 | 1185 | action | Registration | S: explicit resource caption |
| D100/C8 | 1283 | read-only-output | Read-only about/license detail text | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D100/C9 | 1284 | text-output-or-label | Home Site: | S: explicit resource caption |
| D100/C10 | 1187 | text-output-or-label | http://www.internetdownloadmanager.com | S: explicit resource caption |
| D100/C11 | 1285 | text-output-or-label | Support Team: | S: explicit resource caption |
| D100/C12 | 1189 | text-output-or-label | internetdownloadmanager.com/contact_us.html | S: explicit resource caption |
| D100/C13 | 1079 | action | Close | S: explicit resource caption |
| D100/C14 | 1814 | text-output-or-label | Additional license information | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D100/C15 | 1818 | text-output-or-label | Additional license information | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D100/C16 | 4294967295 | text-output-or-label | Copyright © 1999-2026 Tonec FZE. All rights reserved. | S: explicit resource caption |

## D102 — Internet Download Manager 6.43

Target family: `gui.shell`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D102/C0 | 1002 | collection | Main download task collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D102/C1 | 1276 | collection | Category and queue navigation tree | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D102/C2 | 1279 | group-container | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D102/C3 | 1280 | group-container | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D102/C4 | 1278 | action | Close category navigation | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D102/C5 | 1281 | text-output-or-label | Categories | S: explicit resource caption |

## D103 — Download progress

Target family: `task.progress`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D103/C0 | 1146 | progress-output | Overall download progress | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D103/C1 | 1178 | action | << Hide details | S: explicit resource caption |
| D103/C2 | 1158 | action | Pause | S: explicit resource caption |
| D103/C3 | 2 | action | Cancel | S: explicit resource caption |
| D103/C4 | 1179 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D103/C5 | 1177 | collection | Per-connection start offsets and progress collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D103/C6 | 1305 | text-output-or-label | Start positions and download progress by connections | S: explicit resource caption |
| D103/C7 | 1760 | text-output-or-label | Download status detail surface | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D103/C8 | 1853 | text-output-or-label | Download status layout surface | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D105 — Tip of the Day

Target family: `app.tips`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D105/C0 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D105/C1 | 1864 | text-output-or-label | Do you know... | S: explicit resource caption |
| D105/C2 | 1005 | text-output-or-label | Current tip text | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D105/C3 | 1001 | boolean-choice | Show Tips on StartUp | S: explicit resource caption |
| D105/C4 | 1003 | action | Next Tip | S: explicit resource caption |
| D105/C5 | 1079 | action | Close | S: explicit resource caption |
| D105/C6 | 1000 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |

## D130 — Download File Info

Target family: `task.add`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D130/C0 | 1006 | action | Start Download | S: explicit resource caption |
| D130/C1 | 1004 | action | Download Later | S: explicit resource caption |
| D130/C2 | 2 | action | Cancel | S: explicit resource caption |
| D130/C3 | 1277 | text-output-or-label | URL | S: explicit resource caption |
| D130/C4 | 1007 | read-only-output | Download URL | S: manually reviewed label/layout binding; association inferred, not runtime observed D130/C3 |
| D130/C5 | 1297 | text-output-or-label | Category | S: explicit resource caption |
| D130/C6 | 1209 | choice-or-input | Destination category | S: manually reviewed label/layout binding; association inferred, not runtime observed D130/C5 |
| D130/C7 | 1697 | action | Create destination category | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D130/C8 | 1296 | text-output-or-label | Save As | S: explicit resource caption |
| D130/C9 | 1008 | choice-or-input | Output path/history | S: manually reviewed label/layout binding; association inferred, not runtime observed D130/C8 |
| D130/C10 | 1608 | action | Choose output path | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D130/C11 | 1817 | group-container | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D130/C12 | 1609 | boolean-choice | Remember this path for "General" category | S: explicit resource caption |
| D130/C13 | 1815 | read-only-output | Output filename within remembered category path | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D130/C14 | 1009 | input | Task description | S: manually reviewed label/layout binding; association inferred, not runtime observed D130/C17 |
| D130/C15 | 1173 | action | Preview | S: explicit resource caption |
| D130/C16 | 1101 | text-output-or-label | File information output | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D130/C17 | 1298 | text-output-or-label | Description | S: explicit resource caption |
| D130/C18 | 1094 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |

## D135 — File Properties

Target family: `task.properties`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D135/C0 | 1094 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D135/C1 | 1081 | text-output-or-label | Filename summary | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D135/C2 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D135/C3 | 1325 | text-output-or-label | Type: | S: explicit resource caption |
| D135/C4 | 1093 | text-output-or-label | File type value | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D135/C5 | 1299 | text-output-or-label | Status: | S: explicit resource caption |
| D135/C6 | 1098 | text-output-or-label | Task status value | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D135/C7 | 1324 | text-output-or-label | Size: | S: explicit resource caption |
| D135/C8 | 1095 | text-output-or-label | File size value | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D135/C9 | 1323 | text-output-or-label | Save To: | S: explicit resource caption |
| D135/C10 | 1096 | read-only-output | Output path | S: manually reviewed label/layout binding; association inferred, not runtime observed D135/C9 |
| D135/C11 | 1082 | action | Choose output path | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D135/C12 | 1619 | text-output-or-label | Address: | S: explicit resource caption |
| D135/C13 | 1097 | input | Source URL | S: manually reviewed label/layout binding; association inferred, not runtime observed D135/C12 |
| D135/C14 | 1298 | text-output-or-label | Description: | S: explicit resource caption |
| D135/C15 | 1099 | input | Task description | S: manually reviewed label/layout binding; association inferred, not runtime observed D135/C14 |
| D135/C16 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D135/C17 | 1741 | text-output-or-label | The web page from which this file was obtained: | S: explicit resource caption |
| D135/C18 | 1734 | text-output-or-label | Originating web page value | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D135/C19 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D135/C20 | 1570 | text-output-or-label | Referer: | S: explicit resource caption |
| D135/C21 | 1569 | input | Referer URL | S: manually reviewed label/layout binding; association inferred, not runtime observed D135/C20 |
| D135/C22 | 1322 | text-output-or-label | Login | S: explicit resource caption |
| D135/C23 | 1033 | input | HTTP username | S: manually reviewed label/layout binding; association inferred, not runtime observed D135/C22 |
| D135/C24 | 1289 | text-output-or-label | Password | S: explicit resource caption |
| D135/C25 | 1041 | input | HTTP password | S: manually reviewed label/layout binding; association inferred, not runtime observed D135/C24 |
| D135/C26 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D135/C27 | 1240 | text-output-or-label | Last try | S: explicit resource caption |
| D135/C28 | 1241 | text-output-or-label | Last attempt/result output | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D135/C29 | 1164 | action | Open | S: explicit resource caption |
| D135/C30 | 1092 | action | OK | S: explicit resource caption |

## D138 — Internet Download Manager Registration

Target family: `app.commercial`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D138/C0 | 1326 | text-output-or-label | Please enter your registration details in the corresponding fields and push OK button | S: explicit resource caption |
| D138/C1 | 65535 | group-container | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D138/C2 | 1327 | text-output-or-label | First Name | S: explicit resource caption |
| D138/C3 | 1200 | input | Registration first name | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D138/C4 | 1328 | text-output-or-label | Last name | S: explicit resource caption |
| D138/C5 | 1043 | input | Registration last name | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D138/C6 | 1329 | text-output-or-label | Email | S: explicit resource caption |
| D138/C7 | 1189 | input | Registration email | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D138/C8 | 1330 | text-output-or-label | Serial number | S: explicit resource caption |
| D138/C9 | 1194 | input | Registration serial number | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D138/C10 | 1755 | text-output-or-label | What is the "Seral number"? | S: explicit resource caption |
| D138/C11 | 1191 | action | OK | S: explicit resource caption |
| D138/C12 | 1201 | action | How to License | S: explicit resource caption |
| D138/C13 | 1080 | action | Cancel | S: explicit resource caption |

## D139 — General

Target family: `settings.general`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D139/C0 | 4294967295 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D139/C1 | 1527 | text-output-or-label | Browser/System Integration | S: explicit resource caption |
| D139/C2 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D139/C3 | 1056 | boolean-choice | Launch Internet Download Manager on startup | S: explicit resource caption |
| D139/C4 | 1061 | boolean-choice | Automatically start downloading of URLs placed to clipboard | S: explicit resource caption |
| D139/C5 | 1650 | boolean-choice | Run module for monitoring in IE-based browsers (AOL, MSN, Avant, etc) | S: explicit resource caption |
| D139/C6 | 1751 | group-container | Capture downloads from the following browsers: | S: explicit resource caption |
| D139/C7 | 1351 | collection | Integrated browser selection collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D139/C8 | 1633 | action | Add browser... | S: explicit resource caption |
| D139/C9 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D139/C10 | 1368 | text-output-or-label | Customize keys to prevent or force downloading with IDM | S: explicit resource caption |
| D139/C11 | 1367 | action | Configure browser capture modifier keys | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D139/C12 | 1689 | text-output-or-label | Customize IDM menu items in context menu of browsers | S: explicit resource caption |
| D139/C13 | 1688 | action | Configure browser context menu entries | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D139/C14 | 1721 | action | Configure browser download panels | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D139/C15 | 1748 | text-output-or-label | Customize IDM Download panels in browsers | S: explicit resource caption |
| D139/C16 | 1866 | action | Restart | S: explicit resource caption |
| D139/C17 | 1865 | text-output-or-label | Advanced browser integration is enabled | S: explicit resource caption |

## D140 — Proxy / Socks

Target family: `settings.proxy`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D140/C0 | 4294967295 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D140/C1 | 1528 | text-output-or-label | Proxy / socks configuration | S: explicit resource caption |
| D140/C2 | 1863 | action | Get System | S: explicit resource caption |
| D140/C3 | 1844 | boolean-choice | Use proxy/socks from a browser's request if there are download errors for the downloads intercepted from the browser | S: explicit resource caption |
| D140/C4 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D140/C5 | 1826 | exclusive-choice | No proxy/socks | S: explicit resource caption |
| D140/C6 | 1825 | exclusive-choice | Use system settings | S: explicit resource caption |
| D140/C7 | 1796 | exclusive-choice | Use automatic configuration script | S: explicit resource caption |
| D140/C8 | 1823 | exclusive-choice | Manual proxy/socks configuration | S: explicit resource caption |
| D140/C9 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D140/C10 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D140/C11 | 1617 | text-output-or-label | Address | S: explicit resource caption |
| D140/C12 | 1660 | input | Automatic proxy configuration script URL | S: manually reviewed label/layout binding; association inferred, not runtime observed D140/C11 |
| D140/C13 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D140/C14 | 1122 | text-output-or-label | Proxy server address | S: explicit resource caption |
| D140/C15 | 1123 | text-output-or-label | Port | S: explicit resource caption |
| D140/C16 | 1124 | text-output-or-label | UserName | S: explicit resource caption |
| D140/C17 | 1125 | text-output-or-label | Password | S: explicit resource caption |
| D140/C18 | 1103 | input | Proxy server address | S: manually reviewed label/layout binding; association inferred, not runtime observed D140/C14 |
| D140/C19 | 1104 | input | Proxy server port | S: manually reviewed label/layout binding; association inferred, not runtime observed D140/C15 |
| D140/C20 | 1105 | input | Proxy username | S: manually reviewed label/layout binding; association inferred, not runtime observed D140/C16 |
| D140/C21 | 1106 | input | Proxy password | S: manually reviewed label/layout binding; association inferred, not runtime observed D140/C17 |
| D140/C22 | 1785 | text-output-or-label | Use this proxy for the following protocols: | S: explicit resource caption |
| D140/C23 | 1782 | boolean-choice | http | S: explicit resource caption |
| D140/C24 | 1783 | boolean-choice | https | S: explicit resource caption |
| D140/C25 | 1784 | boolean-choice | ftp | S: explicit resource caption |
| D140/C26 | 1777 | action | Advanced / Socks... | S: explicit resource caption |
| D140/C27 | 1378 | group-container | Do not use proxy server for addresses beginning with: | S: explicit resource caption |
| D140/C28 | 1376 | input | Proxy bypass address prefixes | S: manually reviewed label/layout binding; association inferred, not runtime observed D140/C27 |
| D140/C29 | 1377 | text-output-or-label | Separate names by spaces | S: explicit resource caption |
| D140/C30 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D140/C31 | 1116 | boolean-choice | Use FTP in PASV mode | S: explicit resource caption |

## D141 — Connection

Target family: `settings.connections`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D141/C0 | 1344 | action | New | S: explicit resource caption |
| D141/C1 | 1023 | collection | Per-server connection limit exception collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D141/C2 | 1345 | action | Delete | S: explicit resource caption |
| D141/C3 | 1346 | action | Edit | S: explicit resource caption |
| D141/C4 | 4294967295 | group-container | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D141/C5 | 1254 | boolean-choice | Download limits | S: explicit resource caption |
| D141/C6 | 1266 | text-output-or-label | Download no more than | S: explicit resource caption |
| D141/C7 | 1046 | input | Download quota in MBytes | S: manually reviewed label/layout binding; association inferred, not runtime observed D141/C6,D141/C8 |
| D141/C8 | 1264 | text-output-or-label | MBytes | S: explicit resource caption |
| D141/C9 | 1267 | text-output-or-label | every | S: explicit resource caption |
| D141/C10 | 1074 | input | Quota interval in hours | S: manually reviewed label/layout binding; association inferred, not runtime observed D141/C9,D141/C11 |
| D141/C11 | 1265 | text-output-or-label | hours | S: explicit resource caption |
| D141/C12 | 1268 | boolean-choice | Show warning before stopping downloads | S: explicit resource caption |
| D141/C13 | 1350 | text-output-or-label | Default max. conn. number | S: explicit resource caption |
| D141/C14 | 1206 | choice-or-input | Default maximum connections | S: manually reviewed label/layout binding; association inferred, not runtime observed D141/C13 |
| D141/C15 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D141/C16 | 1347 | text-output-or-label | Exceptions: | S: explicit resource caption |
| D141/C17 | 1343 | group-container | Max. connections number | S: explicit resource caption |
| D141/C18 | 4294967295 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D141/C19 | 1525 | text-output-or-label | Connections and Limits | S: explicit resource caption |
| D141/C20 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D141/C21 | 1842 | boolean-choice | Use TLS 1.3 from Microsoft (Experimental) | S: explicit resource caption |

## D143 — Save to

Target family: `settings.paths`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D143/C0 | 65535 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D143/C1 | 1626 | text-output-or-label | Categories, file types, folders | S: explicit resource caption |
| D143/C2 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D143/C3 | 1132 | group-container | Save To... | S: explicit resource caption |
| D143/C4 | 1344 | action | New | S: explicit resource caption |
| D143/C5 | 1346 | action | Edit... | S: explicit resource caption |
| D143/C6 | 1282 | choice-or-input | Category selection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D143/C7 | 1624 | text-output-or-label | Automatically put in "Geneal" category the following file types: | S: explicit resource caption |
| D143/C8 | 1038 | read-only-output | Category file extension patterns | S: manually reviewed label/layout binding; association inferred, not runtime observed D143/C7 |
| D143/C9 | 1136 | text-output-or-label | Default download directory for "General" category | S: explicit resource caption |
| D143/C10 | 1032 | read-only-output | Category destination directory | S: manually reviewed label/layout binding; association inferred, not runtime observed D143/C9 |
| D143/C11 | 1049 | action | Choose category directory | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D143/C12 | 1190 | boolean-choice | Change folder for "General" category on last selected | S: explicit resource caption |
| D143/C13 | 1795 | boolean-choice | Set file creation date as provided by the server | S: explicit resource caption |
| D143/C14 | 1138 | group-container | Temporary directory | S: explicit resource caption |
| D143/C15 | 1217 | read-only-output | Temporary partial-file directory | S: manually reviewed label/layout binding; association inferred, not runtime observed D143/C14 |
| D143/C16 | 1054 | action | Choose temporary directory | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D143/C17 | 1625 | text-output-or-label | Temporary directory is required for storing file parts during download. | S: explicit resource caption |
| D143/C18 | 1297 | text-output-or-label | Category | S: explicit resource caption |

## D144 — New version of Internet Download Manager is available

Target family: `app.update`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D144/C0 | 1162 | action | Update now | S: explicit resource caption |
| D144/C1 | 1086 | action | Cancel | S: explicit resource caption |
| D144/C2 | 1193 | input | Update release details | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D145 — Sites Logins

Target family: `auth.store`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D145/C0 | 65535 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D145/C1 | 1530 | text-output-or-label | User names and passwords for servers/sites | S: explicit resource caption |
| D145/C2 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D145/C3 | 1024 | collection | Site credential profile collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D145/C4 | 1087 | action | New | S: explicit resource caption |
| D145/C5 | 1196 | action | Edit | S: explicit resource caption |
| D145/C6 | 1050 | action | Remove | S: explicit resource caption |

## D146 — Site login

Target family: `auth.store`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D146/C0 | 1205 | choice-or-input | Site protocol choice | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D146/C1 | 1337 | text-output-or-label | Server/path | S: explicit resource caption |
| D146/C2 | 1034 | input | Server/path credential scope | S: manually reviewed label/layout binding; association inferred, not runtime observed D146/C1 |
| D146/C3 | 1287 | text-output-or-label | Note: Type the path only if you have different login names for different server directories. | S: explicit resource caption |
| D146/C4 | 1288 | text-output-or-label | User | S: explicit resource caption |
| D146/C5 | 1042 | input | Site username | S: manually reviewed label/layout binding; association inferred, not runtime observed D146/C4 |
| D146/C6 | 1289 | text-output-or-label | Password | S: explicit resource caption |
| D146/C7 | 1072 | input | Site password | S: manually reviewed label/layout binding; association inferred, not runtime observed D146/C6 |
| D146/C8 | 1 | action | OK | S: explicit resource caption |
| D146/C9 | 2 | action | Cancel | S: explicit resource caption |

## D147 — Dialog

Target family: `task.confirmation`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D147/C0 | 1163 | action | Yes | S: explicit resource caption |
| D147/C1 | 1088 | action | Yes to all | S: explicit resource caption |
| D147/C2 | 1202 | action | No | S: explicit resource caption |
| D147/C3 | 1051 | action | No to all | S: explicit resource caption |
| D147/C4 | 1203 | text-output-or-label | Dynamic confirmation message | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D148 — Enter new address to download

Target family: `task.add`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D148/C0 | 1617 | text-output-or-label | Address | S: explicit resource caption |
| D148/C1 | 1007 | choice-or-input | Download URL/history | S: manually reviewed label/layout binding; association inferred, not runtime observed D148/C0 |
| D148/C2 | 1834 | group-container | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D148/C3 | 1182 | boolean-choice | Use authorization | S: explicit resource caption |
| D148/C4 | 1184 | text-output-or-label | Login | S: explicit resource caption |
| D148/C5 | 1033 | input | HTTP username | S: manually reviewed label/layout binding; association inferred, not runtime observed D148/C4 |
| D148/C6 | 1186 | text-output-or-label | Password | S: explicit resource caption |
| D148/C7 | 1041 | input | HTTP password | S: manually reviewed label/layout binding; association inferred, not runtime observed D148/C6 |
| D148/C8 | 1022 | action | OK | S: explicit resource caption |
| D148/C9 | 1080 | action | Cancel | S: explicit resource caption |

## D150 — Shut down

Target family: `power.shutdown`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D150/C0 | 1263 | text-output-or-label | Internet Download Manager has finished queue processing. According to scheduler settings your computer is shutting down. | S: explicit resource caption |
| D150/C1 | 1211 | progress-output | Shutdown countdown progress | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D150/C2 | 2 | action | Cancel | S: explicit resource caption |
| D150/C3 | 1170 | action | Shut down now | S: explicit resource caption |

## D182 — Quick Update

Target family: `app.update`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D182/C0 | 1242 | progress-output | Update check progress | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D182/C1 | 1295 | text-output-or-label | Checking for new version | S: explicit resource caption |

## D184 — Dial up

Target family: `platform.dialup`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D184/C0 | 1167 | action | Redial | S: explicit resource caption |
| D184/C1 | 2 | action | Cancel | S: explicit resource caption |
| D184/C2 | 1247 | text-output-or-label | Connecting ... | S: explicit resource caption |

## D185 — Dial Up / VPN

Target family: `platform.dialup`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D185/C0 | 1058 | boolean-choice | Use Windows Dial Up / VPN Networking | S: explicit resource caption |
| D185/C1 | 1307 | group-container | Connection options | S: explicit resource caption |
| D185/C2 | 1313 | text-output-or-label | Connection: | S: explicit resource caption |
| D185/C3 | 1208 | choice-or-input | Windows dial-up/VPN connection selection | S: manually reviewed label/layout binding; association inferred, not runtime observed D185/C2 |
| D185/C4 | 1288 | text-output-or-label | User name: | S: explicit resource caption |
| D185/C5 | 1037 | input | Connection username | S: manually reviewed label/layout binding; association inferred, not runtime observed D185/C4 |
| D185/C6 | 1289 | text-output-or-label | Password: | S: explicit resource caption |
| D185/C7 | 1045 | input | Connection password | S: manually reviewed label/layout binding; association inferred, not runtime observed D185/C6 |
| D185/C8 | 1237 | boolean-choice | Save password | S: explicit resource caption |
| D185/C9 | 1166 | action | Apply | S: explicit resource caption |
| D185/C10 | 1308 | group-container | Redial options | S: explicit resource caption |
| D185/C11 | 1309 | text-output-or-label | Redial attempts (zero if endlessly): | S: explicit resource caption |
| D185/C12 | 1212 | input | Redial attempt count (zero indicates endless) | S: manually reviewed label/layout binding; association inferred, not runtime observed D185/C11,D185/C13 |
| D185/C13 | 1311 | text-output-or-label | times | S: explicit resource caption |
| D185/C14 | 1310 | text-output-or-label | Time between redial attempts: | S: explicit resource caption |
| D185/C15 | 1249 | input | Redial interval in seconds | S: manually reviewed label/layout binding; association inferred, not runtime observed D185/C14,D185/C16 |
| D185/C16 | 1312 | text-output-or-label | seconds | S: explicit resource caption |
| D185/C17 | 65535 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D185/C18 | 1526 | text-output-or-label | Dial up / VPN settings | S: explicit resource caption |
| D185/C19 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |

## D186 — Dial Up Options

Target family: `platform.dialup`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D186/C0 | 1288 | text-output-or-label | User name: | S: explicit resource caption |
| D186/C1 | 1037 | input | Connection username | S: manually reviewed label/layout binding; association inferred, not runtime observed D186/C0 |
| D186/C2 | 1289 | text-output-or-label | Password: | S: explicit resource caption |
| D186/C3 | 1045 | input | Connection password | S: manually reviewed label/layout binding; association inferred, not runtime observed D186/C2 |
| D186/C4 | 1237 | boolean-choice | Save password | S: explicit resource caption |
| D186/C5 | 1336 | action | Dial | S: explicit resource caption |
| D186/C6 | 2 | action | Cancel | S: explicit resource caption |

## D187 — IDM drop target. Drop web-links for downloading here

Target family: `gui.drop`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D187/C0 | 1250 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |

## D190 — Download IDM components

Target family: `app.update`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D190/C0 | 1 | action | OK | S: explicit resource caption |
| D190/C1 | 2 | action | Cancel | S: explicit resource caption |
| D190/C2 | 1258 | text-output-or-label | Download components | S: explicit resource caption |

## D191 — Export download list

Target family: `lists.export`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D191/C0 | 1259 | exclusive-choice | Export download queue | S: explicit resource caption |
| D191/C1 | 1260 | exclusive-choice | Export selected files | S: explicit resource caption |
| D191/C2 | 1261 | exclusive-choice | Export all files | S: explicit resource caption |
| D191/C3 | 1 | action | OK | S: explicit resource caption |
| D191/C4 | 2 | action | Cancel | S: explicit resource caption |

## D193 — Automatically start downloading the following extensions:

Target family: `browser.types`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D193/C0 | 1027 | collection | Automatically captured extension collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D193/C1 | 1 | action | OK | S: explicit resource caption |
| D193/C2 | 2 | action | Cancel | S: explicit resource caption |

## D194 — Download limits exceeded!

Target family: `settings.quota`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D194/C0 | 1 | action | OK | S: explicit resource caption |
| D194/C1 | 1269 | text-output-or-label | Quota warning message | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D195 — Sounds

Target family: `settings.sounds`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D195/C0 | 65535 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D195/C1 | 1531 | text-output-or-label | Sound settings | S: explicit resource caption |
| D195/C2 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D195/C3 | 1320 | group-container | Select sounds for Internet Download Manager events | S: explicit resource caption |
| D195/C4 | 1028 | collection | Event sound configuration collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D195/C5 | 1172 | action | Choose event sound file | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D195/C6 | 1091 | action | Preview event sound | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D196 — Zip preview

Target family: `task.archive`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D196/C0 | 1 | action | OK | S: explicit resource caption |
| D196/C1 | 1029 | collection | ZIP archive member collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D196/C2 | 1275 | text-output-or-label | Name | S: explicit resource caption |

## D204 — IDM categories

Target family: `settings.categories`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D204/C0 | 1291 | text-output-or-label | Category name | S: explicit resource caption |
| D204/C1 | 1075 | input | Category name | S: manually reviewed label/layout binding; association inferred, not runtime observed D204/C0 |
| D204/C2 | 1292 | text-output-or-label | Automatically put in this category the following file types: | S: explicit resource caption |
| D204/C3 | 1038 | input | Category extension patterns | S: manually reviewed label/layout binding; association inferred, not runtime observed D204/C2 |
| D204/C4 | 1293 | text-output-or-label | Note: type file extensions separated by space (e.g. avi mpg mpeg) | S: explicit resource caption |
| D204/C5 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D204/C6 | 1790 | boolean-choice | Automatically put in this category the files from the following sites only: | S: explicit resource caption |
| D204/C7 | 1792 | input | Category site wildcard patterns | S: manually reviewed label/layout binding; association inferred, not runtime observed D204/C6 |
| D204/C8 | 1793 | text-output-or-label | Separate sites by spaces. You may use asterisk as a wildcard pattern | S: explicit resource caption |
| D204/C9 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D204/C10 | 1294 | text-output-or-label | Save future downloads of this category to the following folder: | S: explicit resource caption |
| D204/C11 | 1047 | input | Category destination folder | S: manually reviewed label/layout binding; association inferred, not runtime observed D204/C10 |
| D204/C12 | 1239 | boolean-choice | Remember last save path | S: explicit resource caption |
| D204/C13 | 1174 | action | Choose category folder | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D204/C14 | 1 | action | OK | S: explicit resource caption |
| D204/C15 | 2 | action | Cancel | S: explicit resource caption |

## D208 — IDM language

Target family: `gui.language`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D208/C0 | 1 | action | OK | S: explicit resource caption |
| D208/C1 | 1204 | choice-or-input | Interface language selection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D208/C2 | 65535 | text-output-or-label | Please select the language for IDM user interface (You may change it later using "View->Language" menu item) | S: explicit resource caption |
| D208/C3 | 1338 | text-output-or-label | Language information | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D209 — Columns

Target family: `gui.columns`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D209/C0 | 1339 | text-output-or-label | Check the columns that you would like visible in this list. Use the Move Up and Move Down buttons to reorder the columns however you like. | S: explicit resource caption |
| D209/C1 | 1023 | collection | Column visibility/order selection collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D209/C2 | 1160 | action | Move Up | S: explicit resource caption |
| D209/C3 | 1083 | action | Move Down | S: explicit resource caption |
| D209/C4 | 1340 | action | Show | S: explicit resource caption |
| D209/C5 | 1048 | action | Hide | S: explicit resource caption |
| D209/C6 | 1053 | action | Reset | S: explicit resource caption |
| D209/C7 | 1341 | text-output-or-label | The selected column should be | S: explicit resource caption |
| D209/C8 | 1031 | input | Selected column width in pixels | S: manually reviewed label/layout binding; association inferred, not runtime observed D209/C7,D209/C9 |
| D209/C9 | 1342 | text-output-or-label | pixels wide | S: explicit resource caption |
| D209/C10 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D209/C11 | 1 | action | OK | S: explicit resource caption |
| D209/C12 | 2 | action | Cancel | S: explicit resource caption |

## D226 — Max. connections number for a server

Target family: `settings.host_connections`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D226/C0 | 1348 | text-output-or-label | Server | S: explicit resource caption |
| D226/C1 | 1204 | choice-or-input | Server protocol selection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D226/C2 | 1031 | input | Server wildcard pattern | S: manually reviewed label/layout binding; association inferred, not runtime observed D226/C0 |
| D226/C3 | 1618 | text-output-or-label | You may use asterisk as a wildcard pattern | S: explicit resource caption |
| D226/C4 | 1306 | text-output-or-label | Max. connections number | S: explicit resource caption |
| D226/C5 | 1349 | choice-or-input | Maximum connections for matching server | S: manually reviewed label/layout binding; association inferred, not runtime observed D226/C4 |
| D226/C6 | 1 | action | OK | S: explicit resource caption |
| D226/C7 | 2 | action | Cancel | S: explicit resource caption |

## D228 — Download All Links with IDM

Target family: `browser.all_links`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D228/C0 | 1355 | text-output-or-label | Please check the links, which you want to add to the download list of IDM, and click OK button.  You may wait until IDM checks and fills all file types. | S: explicit resource caption |
| D228/C1 | 1392 | collection | Discovered page link selection collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D228/C2 | 1356 | group-container | Save To | S: explicit resource caption |
| D228/C3 | 1357 | exclusive-choice | Every file to the directory according to the category of the file | S: explicit resource caption |
| D228/C4 | 1358 | exclusive-choice | All files to one category | S: explicit resource caption |
| D228/C5 | 1359 | exclusive-choice | All files to one directory | S: explicit resource caption |
| D228/C6 | 1204 | choice-or-input | Single destination category | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D228/C7 | 1031 | input | Single destination directory | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D228/C8 | 1360 | action | Choose single directory | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D228/C9 | 1234 | boolean-choice | Use link texts as download descriptions | S: explicit resource caption |
| D228/C10 | 1055 | boolean-choice | Hide HTML files | S: explicit resource caption |
| D228/C11 | 1385 | boolean-choice | Hide images located on this web page | S: explicit resource caption |
| D228/C12 | 1060 | boolean-choice | Hide repeated files | S: manually reviewed label/layout binding; association inferred, not runtime observed D228/C20 |
| D228/C13 | 1040 | input | Replacement filename wildcard pattern | S: manually reviewed label/layout binding; association inferred, not runtime observed D228/C19 |
| D228/C14 | 1353 | action | Check All | S: explicit resource caption |
| D228/C15 | 1354 | action | Uncheck All | S: explicit resource caption |
| D228/C16 | 1352 | action | Edit... | S: explicit resource caption |
| D228/C17 | 1 | action | OK | S: explicit resource caption |
| D228/C18 | 2 | action | Cancel | S: explicit resource caption |
| D228/C19 | 1552 | text-output-or-label | Replace file names using asterisk wildcard for the file name pattern: | S: explicit resource caption |
| D228/C20 | 1849 | text-output-or-label | Hide repeated files | S: explicit resource caption |
| D228/C21 | 1850 | text-output-or-label | Every file to the directory according to the category of the file | S: explicit resource caption |
| D228/C22 | 1851 | text-output-or-label | All files to one category | S: explicit resource caption |
| D228/C23 | 1852 | text-output-or-label | All files to one directory | S: explicit resource caption |

## D229 — Using special keys

Target family: `browser.shortcuts`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D229/C0 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D229/C1 | 1361 | boolean-choice | Use the following key(s) to prevent downloading with IDM for any links: | S: explicit resource caption |
| D229/C2 | 1362 | boolean-choice | Alt | S: explicit resource caption |
| D229/C3 | 1363 | boolean-choice | Shift | S: explicit resource caption |
| D229/C4 | 1064 | boolean-choice | Ctrl | S: explicit resource caption |
| D229/C5 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D229/C6 | 1067 | boolean-choice | Use the following key(s) to force downloading with IDM for any links: | S: explicit resource caption |
| D229/C7 | 1364 | boolean-choice | Alt | S: explicit resource caption |
| D229/C8 | 1365 | boolean-choice | Shift | S: explicit resource caption |
| D229/C9 | 1373 | boolean-choice | Ctrl | S: explicit resource caption |
| D229/C10 | 1252 | boolean-choice | Ins | S: explicit resource caption |
| D229/C11 | 1374 | text-output-or-label | Note: Sometimes after you click on a download link, a web page will load telling that the download starts in X seconds. In this case you will need to uncheck the following option to force downloading with IDM. | S: explicit resource caption |
| D229/C12 | 1366 | boolean-choice | Check for left mouse button clicked on a link along with the special key(s) pressed to force downloading the link | S: explicit resource caption |
| D229/C13 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D229/C14 | 1 | action | OK | S: explicit resource caption |
| D229/C15 | 2 | action | Cancel | S: explicit resource caption |
| D229/C16 | 1754 | text-output-or-label | Note 2: | S: explicit resource caption |
| D229/C17 | 1753 | boolean-choice | While holding down a special key DO NOT take over the downloads   which are web-pages, pictures, scripts and etc. | S: explicit resource caption |
| D229/C18 | 1835 | boolean-choice | Del | S: explicit resource caption |

## D230 — Checking Internet Connection

Target family: `network.check`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D230/C0 | 1380 | text-output-or-label | Status | S: explicit resource caption |
| D230/C1 | 1382 | action | Check | S: explicit resource caption |
| D230/C2 | 2 | action | Cancel | S: explicit resource caption |

## D231 — General

Target family: `settings.general`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D231/C0 | 4294967295 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D231/C1 | 1527 | text-output-or-label | Browser/System Integration | S: explicit resource caption |
| D231/C2 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D231/C3 | 1056 | boolean-choice | Launch Internet Download Manager on startup | S: explicit resource caption |
| D231/C4 | 1650 | boolean-choice | Run module for monitoring in IE-based browsers (AOL, MSN, Avant, etc) | S: explicit resource caption |
| D231/C5 | 1061 | boolean-choice | Automatically start downloading of URLs placed to clipboard | S: explicit resource caption |
| D231/C6 | 1381 | boolean-choice | Use advanced browser integration | S: explicit resource caption |
| D231/C7 | 1751 | group-container | Capture downloads from the following browsers: | S: explicit resource caption |
| D231/C8 | 1351 | collection | Integrated browser selection collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D231/C9 | 1633 | action | Add browser... | S: explicit resource caption |
| D231/C10 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D231/C11 | 1368 | text-output-or-label | Customize keys to prevent or force downloading with IDM | S: explicit resource caption |
| D231/C12 | 1367 | action | Configure browser capture modifier keys | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D231/C13 | 1689 | text-output-or-label | Customize IDM menu items in context menu of browsers | S: explicit resource caption |
| D231/C14 | 1688 | action | Configure browser context menu entries | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D231/C15 | 1721 | action | Configure browser download panels | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D231/C16 | 1748 | text-output-or-label | Customize IDM Download panels in browsers | S: explicit resource caption |

## D232 — Internet Download Manager Problem Report

Target family: `app.report`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D232/C0 | 1383 | text-output-or-label | We have created a configuration report that you can send us. | S: explicit resource caption |
| D232/C1 | 1384 | read-only-output | Generated configuration report | S: manually reviewed label/layout binding; association inferred, not runtime observed D232/C0 |
| D232/C2 | 1386 | text-output-or-label | Add any comments and requests below if you wish. For example, if you know the application that conflicts with IDM advanced integration, please give its name. | S: explicit resource caption |
| D232/C3 | 1387 | input | User report comments | S: manually reviewed label/layout binding; association inferred, not runtime observed D232/C2 |
| D232/C4 | 1391 | text-output-or-label | If you want the notification when the problem will be resolved, please add your e-mail below | S: explicit resource caption |
| D232/C5 | 1070 | input | Notification email address | S: manually reviewed label/layout binding; association inferred, not runtime observed D232/C4 |
| D232/C6 | 1390 | action | Send | S: explicit resource caption |
| D232/C7 | 2 | action | Cancel | S: explicit resource caption |

## D236 — IDM tip: Automatic link capture

Target family: `app.tips`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D236/C0 | 1 | action | OK | S: explicit resource caption |
| D236/C1 | 1393 | boolean-choice | Don't show this tip again | S: explicit resource caption |
| D236/C2 | 1394 | text-output-or-label | Automatic link capture guidance text; exact runtime text producer remains unverified | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D236/C3 | 1800 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D236/C4 | 1397 | text-output-or-label | IDM has a flexible mechanism to take over or ignore downloads from any Internet Applications. Would you like to read more? | S: explicit resource caption |
| D236/C5 | 1396 | action | Read more | S: explicit resource caption |

## D239 — IDM Site Grabber

Target family: `grabber.wizard`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D239/C0 | 1487 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D239/C1 | 1429 | action | < Back | S: explicit resource caption |
| D239/C2 | 1430 | action | Next > | S: explicit resource caption |
| D239/C3 | 1079 | action | Close | S: explicit resource caption |

## D240 — (untitled template)

Target family: `grabber.files`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D240/C0 | 1404 | choice-or-input | Included file type filter | S: manually reviewed label/layout binding; association inferred, not runtime observed D240/C23 |
| D240/C1 | 1405 | action | Add Filter | S: explicit resource caption |
| D240/C2 | 1420 | choice-or-input | Excluded file type filter | S: manually reviewed label/layout binding; association inferred, not runtime observed D240/C24 |
| D240/C3 | 1421 | action | Add Filter | S: explicit resource caption |
| D240/C4 | 1437 | boolean-choice | Search files on this site only | S: explicit resource caption |
| D240/C5 | 1571 | boolean-choice | Hide duplicate files found in different locations | S: explicit resource caption |
| D240/C6 | 1459 | boolean-choice | Start downloading all matched files at once | S: explicit resource caption |
| D240/C7 | 1471 | text-output-or-label | You can explore the site at first, check the files that you need, and download them in the main Grabber window, or add them to the main list of IDM. This option downloads all explored files immediately. | S: explicit resource caption |
| D240/C8 | 1485 | action | Advanced >> | S: explicit resource caption |
| D240/C9 | 1484 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D240/C10 | 1517 | group-container | Download if file size is | S: explicit resource caption |
| D240/C11 | 1480 | boolean-choice | Not less than | S: explicit resource caption |
| D240/C12 | 1479 | input | Minimum allowed file size | S: manually reviewed label/layout binding; association inferred, not runtime observed D240/C11 |
| D240/C13 | 1443 | choice-or-input | Minimum file size unit | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D240/C14 | 1482 | boolean-choice | Not more than | S: explicit resource caption |
| D240/C15 | 1481 | input | Maximum allowed file size | S: manually reviewed label/layout binding; association inferred, not runtime observed D240/C14 |
| D240/C16 | 1444 | choice-or-input | Maximum file size unit | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D240/C17 | 1483 | boolean-choice | Download the files located within the following paths/domains only : | S: explicit resource caption |
| D240/C18 | 1040 | input | Allowed file path/domain patterns | S: manually reviewed label/layout binding; association inferred, not runtime observed D240/C17 |
| D240/C19 | 1486 | boolean-choice | Don't download the files located within the following paths/domains: | S: explicit resource caption |
| D240/C20 | 1041 | input | Excluded file path/domain patterns | S: manually reviewed label/layout binding; association inferred, not runtime observed D240/C19 |
| D240/C21 | 1488 | text-output-or-label | Use semicolons ( ; ) to separate entries. | S: explicit resource caption |
| D240/C22 | 1801 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D240/C23 | 1515 | text-output-or-label | Download the following files (file types) | S: explicit resource caption |
| D240/C24 | 1516 | text-output-or-label | Don't download the following files (file types) | S: explicit resource caption |

## D241 — IDM Site Grabber

Target family: `grabber.wizard`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D241/C0 | 1431 | collection | Grabber explored page hierarchy | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D241/C1 | 1023 | collection | Discovered grabber file collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D241/C2 | 1429 | action | < Back | S: explicit resource caption |
| D241/C3 | 1430 | action | Next > | S: explicit resource caption |
| D241/C4 | 1079 | action | Close | S: explicit resource caption |
| D241/C5 | 1504 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D241/C6 | 1501 | text-output-or-label | Dynamic grabber status indicator | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D241/C7 | 1502 | text-output-or-label | Dynamic grabber status indicator | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D241/C8 | 1503 | text-output-or-label | Dynamic grabber status indicator | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D241/C9 | 1505 | text-output-or-label | Dynamic grabber status indicator | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D241/C10 | 1506 | text-output-or-label | Dynamic grabber status indicator | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D242 — (untitled template)

Target family: `legacy.empty`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|

## D244 — Filters

Target family: `grabber.filters`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D244/C0 | 1023 | collection | Named file filter collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D244/C1 | 1423 | action | Add | S: explicit resource caption |
| D244/C2 | 1424 | action | Edit | S: explicit resource caption |
| D244/C3 | 1425 | action | Delete | S: explicit resource caption |
| D244/C4 | 1 | action | OK | S: explicit resource caption |

## D245 — Filter

Target family: `grabber.filters`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D245/C0 | 1426 | text-output-or-label | Name | S: explicit resource caption |
| D245/C1 | 1428 | input | Filter name | S: manually reviewed label/layout binding; association inferred, not runtime observed D245/C0 |
| D245/C2 | 1427 | text-output-or-label | Mask | S: explicit resource caption |
| D245/C3 | 1040 | input | Comma-separated wildcard masks | S: manually reviewed label/layout binding; association inferred, not runtime observed D245/C2 |
| D245/C4 | 1568 | text-output-or-label | Use asterisk as a wildcard pattern. Use commas without spaces to separate entries | S: explicit resource caption |
| D245/C5 | 1 | action | OK | S: explicit resource caption |
| D245/C6 | 2 | action | Cancel | S: explicit resource caption |

## D257 — (untitled template)

Target family: `grabber.project`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D257/C0 | 1414 | text-output-or-label | Grabber Project Name | S: explicit resource caption |
| D257/C1 | 1411 | input | Grabber project name | S: manually reviewed label/layout binding; association inferred, not runtime observed D257/C0 |
| D257/C2 | 1510 | text-output-or-label | Start page/address | S: explicit resource caption |
| D257/C3 | 1402 | input | Grabber seed URL | S: manually reviewed label/layout binding; association inferred, not runtime observed D257/C2 |
| D257/C4 | 1511 | text-output-or-label | Project template | S: explicit resource caption |
| D257/C5 | 1443 | choice-or-input | Grabber project template choice | S: manually reviewed label/layout binding; association inferred, not runtime observed D257/C4 |
| D257/C6 | 1470 | text-output-or-label | If you select a project template, the wizard will make the required project settings for the selected template on the next steps. Note that you may always change any settings manually. | S: explicit resource caption |
| D257/C7 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D257/C8 | 1439 | boolean-choice | Use authorization | S: explicit resource caption |
| D257/C9 | 1184 | text-output-or-label | Login | S: explicit resource caption |
| D257/C10 | 1033 | input | Grabber HTTP username | S: manually reviewed label/layout binding; association inferred, not runtime observed D257/C9 |
| D257/C11 | 1186 | text-output-or-label | Password | S: explicit resource caption |
| D257/C12 | 1041 | input | Grabber HTTP password | S: manually reviewed label/layout binding; association inferred, not runtime observed D257/C11 |
| D257/C13 | 1563 | text-output-or-label | Press Advanced button to enable manual login or to disable a logout page | S: explicit resource caption |
| D257/C14 | 1485 | action | Advanced >> | S: explicit resource caption |
| D257/C15 | 1441 | boolean-choice | Enter login and password manually at the following web page: | S: explicit resource caption |
| D257/C16 | 1070 | input | Manual login page URL | S: manually reviewed label/layout binding; association inferred, not runtime observed D257/C15 |
| D257/C17 | 1561 | boolean-choice | Don't open the logout page: | S: explicit resource caption |
| D257/C18 | 1562 | input | Logout page exclusion wildcard | S: manually reviewed label/layout binding; association inferred, not runtime observed D257/C17 |
| D257/C19 | 1564 | text-output-or-label | Many sites have a logout page(s) that the Grabber should not open.  You may use asterisk wildcard here to specify a pattern for different logout pages. | S: explicit resource caption |
| D257/C20 | 1801 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D257/C21 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |

## D259 — (untitled template)

Target family: `grabber.destination`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D259/C0 | 1356 | group-container | Save To | S: explicit resource caption |
| D259/C1 | 1357 | exclusive-choice | Every file to a folder according to IDM category of the file | S: explicit resource caption |
| D259/C2 | 1358 | exclusive-choice | All files to the folder associated with the following IDM category: | S: explicit resource caption |
| D259/C3 | 1204 | choice-or-input | Grabber destination category | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D259/C4 | 1492 | read-only-output | Resolved category folder | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D259/C5 | 1359 | exclusive-choice | All files to the following folder | S: explicit resource caption |
| D259/C6 | 1031 | input | Grabber single destination folder | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D259/C7 | 1514 | boolean-choice | Use original relative subfolders | S: explicit resource caption |
| D259/C8 | 1360 | action | Choose grabber destination folder | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D259/C9 | 1468 | boolean-choice | Convert the links in downloaded html files to the local files for offline browsing | S: explicit resource caption |
| D259/C10 | 1469 | boolean-choice | Overwrite existing files | S: explicit resource caption |
| D259/C11 | 1436 | boolean-choice | Add checked files to IDM main list and to download queue on closing the grabber | S: explicit resource caption |
| D259/C12 | 1801 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |

## D262 — Settings of IDM Grabber

Target family: `grabber.settings`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D262/C0 | 1465 | input | Parallel grabber exploration files (1 to 10) | S: manually reviewed label/layout binding; association inferred, not runtime observed D262/C1 |
| D262/C1 | 1466 | text-output-or-label | files to explore at the same time (1 to 10) | S: explicit resource caption |
| D262/C2 | 1040 | input | Parallel grabber download files (1 to 10) | S: manually reviewed label/layout binding; association inferred, not runtime observed D262/C3 |
| D262/C3 | 1467 | text-output-or-label | files to download at the same time (1 to 10) | S: explicit resource caption |
| D262/C4 | 1509 | text-output-or-label | Please note that a web server may reject requests if you set a large number of files to explore (download) at the same time. | S: explicit resource caption |
| D262/C5 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D262/C6 | 1573 | boolean-choice | Look up files in IE cache before downloading | S: explicit resource caption |
| D262/C7 | 1518 | boolean-choice | Use link texts as download descriptions when adding files to IDM main list | S: explicit resource caption |
| D262/C8 | 1 | action | OK | S: explicit resource caption |
| D262/C9 | 2 | action | Cancel | S: explicit resource caption |
| D262/C10 | 1813 | boolean-choice | Use Advanced web page processing | S: explicit resource caption |

## D264 — (untitled template)

Target family: `grabber.crawl`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D264/C0 | 1512 | text-output-or-label | At this step you should specify what web pages to explore to find the required files. At the next step, you will be able to set file types, location, and other filters. | S: explicit resource caption |
| D264/C1 | 1435 | exclusive-choice | Explore the whole site | S: explicit resource caption |
| D264/C2 | 1445 | exclusive-choice | Explore the specified number of link levels: | S: explicit resource caption |
| D264/C3 | 1406 | input | Crawl depth within base site | S: manually reviewed label/layout binding; association inferred, not runtime observed D264/C4 |
| D264/C4 | 1490 | text-output-or-label | levels within the base site | S: explicit resource caption |
| D264/C5 | 1472 | input | Crawl depth on other sites | S: manually reviewed label/layout binding; association inferred, not runtime observed D264/C6 |
| D264/C6 | 1491 | text-output-or-label | levels on other sites | S: explicit resource caption |
| D264/C7 | 1489 | text-output-or-label | What is the number of link levels? | S: explicit resource caption |
| D264/C8 | 1494 | boolean-choice | Ignore popup windows | S: explicit resource caption |
| D264/C9 | 1416 | boolean-choice | Don't explore parent directories | S: explicit resource caption |
| D264/C10 | 1493 | boolean-choice | Explore all sites within the main domain | S: explicit resource caption |
| D264/C11 | 1565 | boolean-choice | Process JavaScript | S: explicit resource caption |
| D264/C12 | 1567 | text-output-or-label | Use this option carefully. What is the risk? | S: explicit resource caption |
| D264/C13 | 1485 | action | Advanced >> | S: explicit resource caption |
| D264/C14 | 1474 | boolean-choice | Explore web pages within the following paths/domains only: | S: explicit resource caption |
| D264/C15 | 1040 | input | Allowed exploration path/domain patterns | S: manually reviewed label/layout binding; association inferred, not runtime observed D264/C14 |
| D264/C16 | 1476 | boolean-choice | Don't explore web pages within the following paths/domains: | S: explicit resource caption |
| D264/C17 | 1070 | input | Excluded exploration path/domain patterns | S: manually reviewed label/layout binding; association inferred, not runtime observed D264/C16 |
| D264/C18 | 1488 | text-output-or-label | Use semicolons ( ; ) to separate entries. | S: explicit resource caption |
| D264/C19 | 1801 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D264/C20 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |

## D268 — Template Name

Target family: `grabber.templates`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D268/C0 | 1495 | input | Grabber template name | S: manually reviewed label/layout binding; association inferred, not runtime observed D268/C3 |
| D268/C1 | 1 | action | OK | S: explicit resource caption |
| D268/C2 | 2 | action | Cancel | S: explicit resource caption |
| D268/C3 | 1508 | text-output-or-label | Please enter a name for the grabber template | S: explicit resource caption |

## D269 — Saving the project of IDM Grabber

Target family: `grabber.save`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D269/C0 | 1507 | text-output-or-label | Save this project with the following name | S: explicit resource caption |
| D269/C1 | 1505 | input | Saved grabber project name | S: manually reviewed label/layout binding; association inferred, not runtime observed D269/C0 |
| D269/C2 | 1513 | boolean-choice | Don't show this dialog if the name is unique | S: explicit resource caption |
| D269/C3 | 1 | action | OK | S: explicit resource caption |
| D269/C4 | 2 | action | Cancel | S: explicit resource caption |

## D270 — Load project

Target family: `grabber.load`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D270/C0 | 1 | action | OK | S: explicit resource caption |
| D270/C1 | 2 | action | Cancel | S: explicit resource caption |
| D270/C2 | 1023 | collection | Saved grabber project selection | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D271 — IDM Grabber Statistics

Target family: `grabber.statistics`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D271/C0 | 1579 | text-output-or-label | Total | S: explicit resource caption |
| D271/C1 | 1519 | text-output-or-label | Total explored progress value | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D271/C2 | 1576 | text-output-or-label | Simple | S: explicit resource caption |
| D271/C3 | 1575 | text-output-or-label | Web pages processed | S: explicit resource caption |
| D271/C4 | 1578 | text-output-or-label | Files | S: explicit resource caption |
| D271/C5 | 1574 | text-output-or-label | Status: | S: explicit resource caption |
| D271/C6 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D271/C7 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D271/C8 | 1577 | text-output-or-label | Advanced | S: explicit resource caption |
| D271/C9 | 1580 | text-output-or-label | Explored | S: explicit resource caption |
| D271/C10 | 1581 | text-output-or-label | Matched | S: explicit resource caption |
| D271/C11 | 1582 | text-output-or-label | Downloaded | S: explicit resource caption |
| D271/C12 | 1524 | text-output-or-label | Dynamic grabber statistic value; metric binding not runtime verified | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D271/C13 | 1521 | text-output-or-label | Dynamic grabber statistic value; metric binding not runtime verified | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D271/C14 | 1523 | text-output-or-label | Dynamic grabber statistic value; metric binding not runtime verified | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D271/C15 | 1520 | text-output-or-label | Dynamic grabber statistic value; metric binding not runtime verified | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D271/C16 | 1522 | text-output-or-label | Dynamic grabber statistic value; metric binding not runtime verified | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D271/C17 | 1453 | text-output-or-label | Grabber status value | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D279 — Connecting to the site

Target family: `auth.challenge`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D279/C0 | 1536 | text-output-or-label | Type your user name and password. | S: explicit resource caption |
| D279/C1 | 1535 | text-output-or-label | Realm: | S: explicit resource caption |
| D279/C2 | 1537 | text-output-or-label | User name: | S: explicit resource caption |
| D279/C3 | 1532 | choice-or-input | Authentication challenge username/history | S: manually reviewed label/layout binding; association inferred, not runtime observed D279/C2 |
| D279/C4 | 1538 | text-output-or-label | Password: | S: explicit resource caption |
| D279/C5 | 1533 | input | Authentication challenge password | S: manually reviewed label/layout binding; association inferred, not runtime observed D279/C4 |
| D279/C6 | 1534 | boolean-choice | Save this password in IDM password list | S: explicit resource caption |
| D279/C7 | 1 | action | OK | S: explicit resource caption |
| D279/C8 | 2 | action | Cancel | S: explicit resource caption |
| D279/C9 | 4294967295 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |

## D280 — Batch download

Target family: `batch.generated`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D280/C0 | 1553 | text-output-or-label | It's possible to add a group of sequential file names like img001.jpg, img002.jpg, etc., img100.jpg to IDM download queue. Use the asterisk wildcard for the file name pattern.  For example:   http://www.internetdownloadmanager.com/pictures/img*.jpg | S: explicit resource caption |
| D280/C1 | 1617 | text-output-or-label | Address | S: explicit resource caption |
| D280/C2 | 1007 | choice-or-input | Batch URL wildcard pattern/history | S: manually reviewed label/layout binding; association inferred, not runtime observed D280/C1 |
| D280/C3 | 1555 | group-container | Replace asterisk to | S: explicit resource caption |
| D280/C4 | 1540 | exclusive-choice | Numbers | S: explicit resource caption |
| D280/C5 | 1541 | exclusive-choice | Letters | S: explicit resource caption |
| D280/C6 | 1556 | text-output-or-label | From: | S: explicit resource caption |
| D280/C7 | 1542 | input | Numeric sequence first value | S: manually reviewed label/layout binding; association inferred, not runtime observed D280/C6 |
| D280/C8 | 1543 | numeric-adjustment | Adjust numeric sequence first value | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D280/C9 | 1557 | text-output-or-label | To: | S: explicit resource caption |
| D280/C10 | 1040 | input | Numeric sequence last value | S: manually reviewed label/layout binding; association inferred, not runtime observed D280/C9 |
| D280/C11 | 1544 | numeric-adjustment | Adjust numeric sequence last value | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D280/C12 | 1547 | text-output-or-label | Wildcard size | S: explicit resource caption |
| D280/C13 | 1070 | input | Numeric wildcard padding width | S: manually reviewed label/layout binding; association inferred, not runtime observed D280/C12 |
| D280/C14 | 1545 | numeric-adjustment | Adjust wildcard padding width | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D280/C15 | 65535 | group-container | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D280/C16 | 1182 | boolean-choice | Use authorization | S: explicit resource caption |
| D280/C17 | 1184 | text-output-or-label | Login | S: explicit resource caption |
| D280/C18 | 1033 | input | Batch HTTP username | S: manually reviewed label/layout binding; association inferred, not runtime observed D280/C17 |
| D280/C19 | 1186 | text-output-or-label | Password | S: explicit resource caption |
| D280/C20 | 1041 | input | Batch HTTP password | S: manually reviewed label/layout binding; association inferred, not runtime observed D280/C19 |
| D280/C21 | 1558 | text-output-or-label | First file: | S: explicit resource caption |
| D280/C22 | 1548 | text-output-or-label | First generated URL preview | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D280/C23 | 1559 | text-output-or-label | Second file: | S: explicit resource caption |
| D280/C24 | 1549 | text-output-or-label | Second generated URL preview | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D280/C25 | 65535 | text-output-or-label | ... | S: explicit resource caption |
| D280/C26 | 1560 | text-output-or-label | Last file: | S: explicit resource caption |
| D280/C27 | 1550 | text-output-or-label | Last generated URL preview | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D280/C28 | 1 | action | OK | S: explicit resource caption |
| D280/C29 | 2 | action | Cancel | S: explicit resource caption |
| D280/C30 | 1546 | input | Letter sequence first value | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D280/C31 | 1210 | input | Letter sequence last value | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D281 — IDM Site Grabber

Target family: `grabber.cleanup`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D281/C0 | 1501 | text-output-or-label | Dynamic grabber cleanup indicator | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D281/C1 | 1502 | text-output-or-label | Dynamic grabber cleanup indicator | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D281/C2 | 1503 | text-output-or-label | Dynamic grabber cleanup indicator | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D281/C3 | 1505 | text-output-or-label | Dynamic grabber cleanup indicator | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D281/C4 | 1506 | text-output-or-label | Dynamic grabber cleanup indicator | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D281/C5 | 1566 | text-output-or-label | Deleting temp files. Please wait... | S: explicit resource caption |
| D281/C6 | 1572 | text-output-or-label | 100 % | S: explicit resource caption |

## D284 — Schedule the grabber project

Target family: `grabber.schedule`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D284/C0 | 65535 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D284/C1 | 1593 | text-output-or-label | Project: | S: explicit resource caption |
| D284/C2 | 1594 | text-output-or-label | <project name> | S: explicit resource caption |
| D284/C3 | 1600 | group-container | Step 1. Select type | S: explicit resource caption |
| D284/C4 | 1583 | exclusive-choice | One-time exploring/downloading | S: explicit resource caption |
| D284/C5 | 1584 | exclusive-choice | Periodic synchronization | S: explicit resource caption |
| D284/C6 | 1601 | group-container | Step 2. Select action | S: explicit resource caption |
| D284/C7 | 1585 | exclusive-choice | Explore only | S: explicit resource caption |
| D284/C8 | 1586 | exclusive-choice | Explore site and download matched files | S: explicit resource caption |
| D284/C9 | 1587 | exclusive-choice | Download checked files | S: explicit resource caption |
| D284/C10 | 1588 | group-container | Step 3. Schedule | S: explicit resource caption |
| D284/C11 | 1218 | boolean-choice | Start download at | S: explicit resource caption |
| D284/C12 | 1216 | date-time-input | Grabber scheduled start time | S: manually reviewed label/layout binding; association inferred, not runtime observed D284/C11 |
| D284/C13 | 1595 | boolean-choice | Start again every | S: explicit resource caption |
| D284/C14 | 1219 | exclusive-choice | Once at | S: explicit resource caption |
| D284/C15 | 1221 | exclusive-choice | Daily | S: explicit resource caption |
| D284/C16 | 1599 | text-output-or-label | Daily | S: explicit resource caption |
| D284/C17 | 1220 | date-time-input | Grabber one-time start date | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D284/C18 | 1596 | input | Grabber repeat interval hours | S: manually reviewed label/layout binding; association inferred, not runtime observed D284/C19 |
| D284/C19 | 1597 | text-output-or-label | hours | S: explicit resource caption |
| D284/C20 | 1040 | input | Grabber repeat interval minutes | S: manually reviewed label/layout binding; association inferred, not runtime observed D284/C21 |
| D284/C21 | 1598 | text-output-or-label | min | S: explicit resource caption |
| D284/C22 | 1062 | boolean-choice | Sunday | S: explicit resource caption |
| D284/C23 | 1057 | boolean-choice | Monday | S: explicit resource caption |
| D284/C24 | 1066 | boolean-choice | Tuesday | S: explicit resource caption |
| D284/C25 | 1069 | boolean-choice | Wednesday | S: explicit resource caption |
| D284/C26 | 1222 | boolean-choice | Thursday | S: explicit resource caption |
| D284/C27 | 1223 | boolean-choice | Friday | S: explicit resource caption |
| D284/C28 | 1224 | boolean-choice | Saturday | S: explicit resource caption |
| D284/C29 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D284/C30 | 1225 | boolean-choice | Stop download at | S: explicit resource caption |
| D284/C31 | 1226 | date-time-input | Grabber scheduled stop time | S: manually reviewed label/layout binding; association inferred, not runtime observed D284/C30 |
| D284/C32 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D284/C33 | 1602 | text-output-or-label | Note: IDM should be running in system tray at the specified time to start a scheduled project | S: explicit resource caption |
| D284/C34 | 1227 | boolean-choice | Hang up modem when done | S: explicit resource caption |
| D284/C35 | 1271 | boolean-choice | Exit Internet Download Manager when done | S: explicit resource caption |
| D284/C36 | 1228 | boolean-choice | Turn off computer when done | S: explicit resource caption |
| D284/C37 | 1235 | boolean-choice | Force processes to terminate | S: explicit resource caption |
| D284/C38 | 1590 | boolean-choice | Open the following file when done: | S: explicit resource caption |
| D284/C39 | 1591 | input | Grabber completion command/file | S: manually reviewed label/layout binding; association inferred, not runtime observed D284/C38 |
| D284/C40 | 1592 | action | Choose grabber completion command/file | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D284/C41 | 1 | action | OK | S: explicit resource caption |
| D284/C42 | 2 | action | Cancel | S: explicit resource caption |
| D284/C43 | 1443 | choice-or-input | Grabber completion power action selection | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D286 — Download complete

Target family: `task.completion`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D286/C0 | 1164 | action | Open | S: explicit resource caption |
| D286/C1 | 1639 | action | Open with... | S: explicit resource caption |
| D286/C2 | 1606 | action | Open folder | S: explicit resource caption |
| D286/C3 | 1079 | action | Close | S: explicit resource caption |
| D286/C4 | 1607 | boolean-choice | Don't show this dialog again | S: explicit resource caption |
| D286/C5 | 65535 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D286/C6 | 1301 | text-output-or-label | Downloaded xx bytes. | S: explicit resource caption |
| D286/C7 | 1610 | text-output-or-label | Address | S: explicit resource caption |
| D286/C8 | 1603 | read-only-output | Completed download source URL | S: manually reviewed label/layout binding; association inferred, not runtime observed D286/C7 |
| D286/C9 | 1611 | text-output-or-label | The file saved as | S: explicit resource caption |
| D286/C10 | 1040 | read-only-output | Completed output filename | S: manually reviewed label/layout binding; association inferred, not runtime observed D286/C9 |
| D286/C11 | 1023 | collection | Completed-file drag icon collection; help and screenshot/layout association, runtime wiring unverified | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D290 — IDM update notice

Target family: `app.update`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D290/C0 | 1079 | action | Close | S: explicit resource caption |
| D290/C1 | 65535 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D290/C2 | 1613 | text-output-or-label | Read more | S: explicit resource caption |
| D290/C3 | 1614 | text-output-or-label | IDM checks for a new release every 2 weeks by requesting an update page from its home site. | S: explicit resource caption |
| D290/C4 | 1621 | text-output-or-label | Update notice | S: explicit resource caption |

## D291 — Downloads

Target family: `settings.downloads`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D291/C0 | 4294967295 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D291/C1 | 1529 | text-output-or-label | Default download settings | S: explicit resource caption |
| D291/C2 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D291/C3 | 1767 | text-output-or-label | Customize "Download progress" dialog | S: explicit resource caption |
| D291/C4 | 1688 | action | Configure progress dialog | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D291/C5 | 1059 | boolean-choice | Show start download dialog | S: explicit resource caption |
| D291/C6 | 1862 | boolean-choice | Do not start downloading, only add files to the queue | S: explicit resource caption |
| D291/C7 | 1236 | boolean-choice | Show download complete dialog | S: explicit resource caption |
| D291/C8 | 1622 | text-output-or-label | Note: These settings don't relate to queue processing | S: explicit resource caption |
| D291/C9 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D291/C10 | 1623 | boolean-choice | Start downloading immediately while displaying "Download File Info" dialog | S: explicit resource caption |
| D291/C11 | 1739 | boolean-choice | Show queue selection panel on pressing "Download Later" button | S: explicit resource caption |
| D291/C12 | 1738 | boolean-choice | Show queue selection panel on closing batch download dialogs | S: explicit resource caption |
| D291/C13 | 1615 | boolean-choice | Ignore file modification time changes when resuming a download | S: explicit resource caption |
| D291/C14 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D291/C15 | 1646 | text-output-or-label | If a duplicate download link is added: | S: explicit resource caption |
| D291/C16 | 1443 | choice-or-input | Duplicate URL action selection | S: manually reviewed label/layout binding; association inferred, not runtime observed D291/C15 |
| D291/C17 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D291/C18 | 1839 | text-output-or-label | User-Agent for manually added downloads: | S: explicit resource caption |
| D291/C19 | 1840 | input | Manual download User-Agent | S: manually reviewed label/layout binding; association inferred, not runtime observed D291/C18 |
| D291/C20 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D291/C21 | 1841 | action | Configure antivirus completion check | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D291/C22 | 1838 | text-output-or-label | Virus checking settings | S: explicit resource caption |

## D292 — Rebooting computer

Target family: `power.restart`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D292/C0 | 1 | action | Now | S: explicit resource caption |
| D292/C1 | 1211 | progress-output | Reboot countdown progress | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D292/C2 | 1627 | action | Stop | S: explicit resource caption |
| D292/C3 | 1628 | text-output-or-label | Rebooting in 15 seconds | S: explicit resource caption |

## D294 — File types

Target family: `browser.types`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D294/C0 | 4294967295 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D294/C1 | 1630 | text-output-or-label | Downloaded file types | S: explicit resource caption |
| D294/C2 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D294/C3 | 1270 | text-output-or-label | Automatically start downloading the following file types: | S: explicit resource caption |
| D294/C4 | 1071 | input | Automatically captured file extension patterns | S: manually reviewed label/layout binding; association inferred, not runtime observed D294/C3 |
| D294/C5 | 1077 | action | Default | S: explicit resource caption |
| D294/C6 | 1667 | text-output-or-label | Don't start downloading automatically from the following sites: | S: explicit resource caption |
| D294/C7 | 1031 | input | Automatic capture excluded sites | S: manually reviewed label/layout binding; association inferred, not runtime observed D294/C6 |
| D294/C8 | 1662 | text-output-or-label | (separate names by spaces) | S: explicit resource caption |
| D294/C9 | 1629 | action | Default | S: explicit resource caption |
| D294/C10 | 1661 | text-output-or-label | Don't start downloading automatically from the following addresses: | S: explicit resource caption |
| D294/C11 | 1656 | action | Edit automatic capture address exclusions | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D296 — Browsers integration

Target family: `browser.integration`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D296/C0 | 1 | action | OK | S: explicit resource caption |
| D296/C1 | 2 | action | Cancel | S: explicit resource caption |
| D296/C2 | 1023 | collection | Discovered browser integration selection collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D296/C3 | 1635 | text-output-or-label | IDM has detected the following additional browsers on your computer. Please select browsers that you want to integrate with IDM. | S: explicit resource caption |

## D297 — Browser details

Target family: `browser.integration`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D297/C0 | 1023 | collection | Browser version integration collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D297/C1 | 1636 | action | Add version... | S: explicit resource caption |
| D297/C2 | 1079 | action | Close | S: explicit resource caption |

## D298 — Duplicate download link

Target family: `task.duplicate`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D298/C0 | 1640 | read-only-output | Duplicate source URL | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D298/C1 | 1641 | text-output-or-label | This file already exists in your download list. You may choose one of the following options, or press Cancel button to skip the download of this file | S: explicit resource caption |
| D298/C2 | 1642 | exclusive-choice | Add the duplicate with a numbered file name | S: explicit resource caption |
| D298/C3 | 1643 | exclusive-choice | Add the duplicate and overwrite the existing file | S: explicit resource caption |
| D298/C4 | 1644 | exclusive-choice | If existing file is complete, show download complete dialog; otherwise resume it. | S: explicit resource caption |
| D298/C5 | 1 | action | OK | S: explicit resource caption |
| D298/C6 | 2 | action | Cancel | S: explicit resource caption |
| D298/C7 | 1645 | boolean-choice | Remember my selection and don't show this dialog again.  You may change it in IDM Options -> Downloads at a later time. | S: explicit resource caption |

## D300 — Internet Download Manager

Target family: `task.confirmation`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D300/C0 | 1647 | action | Continue | S: explicit resource caption |
| D300/C1 | 2 | action | Cancel | S: explicit resource caption |
| D300/C2 | 1649 | text-output-or-label | Dynamic continue/cancel message | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D300/C3 | 1648 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |

## D302 — Cancelling an automatically started download

Target family: `browser.exceptions`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D302/C0 | 1651 | text-output-or-label | You have cancelled this download twice.  If IDM always takes over this download by mistake, you may add this address or this site to IDM list of exceptions: | S: explicit resource caption |
| D302/C1 | 1652 | boolean-choice | Don't start downloading automatically from this site: | S: explicit resource caption |
| D302/C2 | 1668 | text-output-or-label | site | S: explicit resource caption |
| D302/C3 | 1653 | boolean-choice | Don't start downloading automatically from this address: | S: explicit resource caption |
| D302/C4 | 1669 | text-output-or-label | address | S: explicit resource caption |
| D302/C5 | 1655 | text-output-or-label | Also you may edit exception lists on "Options->File Types" tab of the options dialog | S: explicit resource caption |
| D302/C6 | 1 | action | OK | S: explicit resource caption |
| D302/C7 | 2 | action | Cancel | S: explicit resource caption |
| D302/C8 | 1654 | boolean-choice | Don't show this dialog again | S: explicit resource caption |

## D303 — The list of address exceptions

Target family: `browser.exceptions`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D303/C0 | 1659 | boolean-choice | Show the dialog to add an address to the list of exceptions for a twice-cancelled download | S: explicit resource caption |
| D303/C1 | 1661 | text-output-or-label | Don't start downloading automatically from the following addresses: | S: explicit resource caption |
| D303/C2 | 1657 | collection | Automatic capture address exception collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D303/C3 | 1423 | action | Add | S: explicit resource caption |
| D303/C4 | 1425 | action | Delete | S: explicit resource caption |
| D303/C5 | 1 | action | OK | S: explicit resource caption |
| D303/C6 | 1703 | text-output-or-label | You may use the asterisk wildcard for the address pattern. | S: explicit resource caption |

## D304 — Add an address to exceptions list

Target family: `browser.exceptions`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D304/C0 | 1 | action | OK | S: explicit resource caption |
| D304/C1 | 2 | action | Cancel | S: explicit resource caption |
| D304/C2 | 1660 | input | Capture exception address pattern | S: manually reviewed label/layout binding; association inferred, not runtime observed D304/C3 |
| D304/C3 | 1617 | text-output-or-label | Address | S: explicit resource caption |

## D305 — IDM could not take over the download

Target family: `browser.fallback`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D305/C0 | 1 | action | OK | S: explicit resource caption |
| D305/C1 | 1670 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D305/C2 | 1671 | text-output-or-label | Takeover failure message | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D305/C3 | 1654 | boolean-choice | Don't show this dialog again | S: explicit resource caption |

## D306 — Schedule

Target family: `queue.schedule`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D306/C0 | 1679 | exclusive-choice | One-time downloading | S: explicit resource caption |
| D306/C1 | 1584 | exclusive-choice | Periodic synchronization | S: explicit resource caption |
| D306/C2 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D306/C3 | 1218 | boolean-choice | Start download at | S: explicit resource caption |
| D306/C4 | 1216 | date-time-input | Queue scheduled start time | S: manually reviewed label/layout binding; association inferred, not runtime observed D306/C3 |
| D306/C5 | 1595 | boolean-choice | Start again every | S: explicit resource caption |
| D306/C6 | 1219 | exclusive-choice | Once at | S: explicit resource caption |
| D306/C7 | 1221 | exclusive-choice | Daily | S: explicit resource caption |
| D306/C8 | 1599 | text-output-or-label | Daily | S: explicit resource caption |
| D306/C9 | 1220 | date-time-input | Queue one-time start date | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D306/C10 | 1596 | input | Queue repeat interval hours | S: manually reviewed label/layout binding; association inferred, not runtime observed D306/C11 |
| D306/C11 | 1597 | text-output-or-label | hours | S: explicit resource caption |
| D306/C12 | 1040 | input | Queue repeat interval minutes | S: manually reviewed label/layout binding; association inferred, not runtime observed D306/C13 |
| D306/C13 | 1598 | text-output-or-label | min | S: explicit resource caption |
| D306/C14 | 1062 | boolean-choice | Sunday | S: explicit resource caption |
| D306/C15 | 1057 | boolean-choice | Monday | S: explicit resource caption |
| D306/C16 | 1066 | boolean-choice | Tuesday | S: explicit resource caption |
| D306/C17 | 1069 | boolean-choice | Wednesday | S: explicit resource caption |
| D306/C18 | 1222 | boolean-choice | Thursday | S: explicit resource caption |
| D306/C19 | 1223 | boolean-choice | Friday | S: explicit resource caption |
| D306/C20 | 1224 | boolean-choice | Saturday | S: explicit resource caption |
| D306/C21 | 1225 | boolean-choice | Stop download at | S: explicit resource caption |
| D306/C22 | 1226 | date-time-input | Queue scheduled stop time | S: manually reviewed label/layout binding; association inferred, not runtime observed D306/C21 |
| D306/C23 | 1680 | boolean-choice | Number of retries for each file if downloading failed : | S: explicit resource caption |
| D306/C24 | 1681 | input | Retry count per failed queue file | S: manually reviewed label/layout binding; association inferred, not runtime observed D306/C23 |
| D306/C25 | 1590 | boolean-choice | Open the following file when done: | S: explicit resource caption |
| D306/C26 | 1591 | input | Queue completion command/file | S: manually reviewed label/layout binding; association inferred, not runtime observed D306/C25 |
| D306/C27 | 1592 | action | Choose queue completion command/file | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D306/C28 | 1227 | boolean-choice | Hang up modem when done | S: explicit resource caption |
| D306/C29 | 1271 | boolean-choice | Exit Internet Download Manager when done | S: explicit resource caption |
| D306/C30 | 1228 | boolean-choice | Turn off computer when done | S: explicit resource caption |
| D306/C31 | 1235 | boolean-choice | Force processes to terminate | S: explicit resource caption |
| D306/C32 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D306/C33 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D306/C34 | 1729 | boolean-choice | Start download on IDM startup | S: explicit resource caption |
| D306/C35 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D306/C36 | 1443 | choice-or-input | Queue completion power action selection | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D307 — Files in the queue

Target family: `queue.files`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D307/C0 | 1333 | text-output-or-label | Download | S: explicit resource caption |
| D307/C1 | 1036 | input | Concurrent files in this queue | S: manually reviewed label/layout binding; association inferred, not runtime observed D307/C0,D307/C3 |
| D307/C2 | 1214 | numeric-adjustment | Adjust queue concurrency | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D307/C3 | 1335 | text-output-or-label | files at the same time | S: explicit resource caption |
| D307/C4 | 1025 | collection | Ordered files assigned to queue | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D307/C5 | 1229 | action | Move item down | S: shipped tooltip; association by same control ID is a candidate until caller verification |
| D307/C6 | 1230 | action | Move item up | S: shipped tooltip; association by same control ID is a candidate until caller verification |
| D307/C7 | 1231 | action | Remove item from download queue | S: shipped tooltip; association by same control ID is a candidate until caller verification |

## D308 — Scheduler

Target family: `queue.manage`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D308/C0 | 1675 | action | New queue | S: explicit resource caption |
| D308/C1 | 1678 | action | Delete | S: explicit resource caption |
| D308/C2 | 1232 | action | Start now | S: explicit resource caption |
| D308/C3 | 1233 | action | Stop | S: explicit resource caption |
| D308/C4 | 1171 | action | Help | S: explicit resource caption |
| D308/C5 | 1089 | action | Apply | S: explicit resource caption |
| D308/C6 | 1079 | action | Close | S: explicit resource caption |
| D308/C7 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D308/C8 | 1683 | text-output-or-label | Queue name | S: explicit resource caption |
| D308/C9 | 1676 | text-output-or-label | Selected queue settings content surface | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D308/C10 | 1682 | collection | Named queue navigation tree | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D308/C11 | 65535 | icon-or-image | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D308/C12 | 1684 | text-output-or-label | Queues | S: explicit resource caption |

## D309 — (untitled template)

Target family: `settings.quota`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D309/C0 | 1687 | boolean-choice | Download limits | S: explicit resource caption |
| D309/C1 | 1266 | text-output-or-label | Download no more than | S: explicit resource caption |
| D309/C2 | 1046 | input | Download quota in MBytes | S: manually reviewed label/layout binding; association inferred, not runtime observed D309/C1,D309/C3 |
| D309/C3 | 1264 | text-output-or-label | MBytes | S: explicit resource caption |
| D309/C4 | 1267 | text-output-or-label | every | S: explicit resource caption |
| D309/C5 | 1074 | input | Quota interval in hours | S: manually reviewed label/layout binding; association inferred, not runtime observed D309/C4,D309/C6 |
| D309/C6 | 1265 | text-output-or-label | hours | S: explicit resource caption |
| D309/C7 | 1268 | boolean-choice | Show warning before stopping downloads | S: explicit resource caption |

## D310 — Enter queue name

Target family: `queue.manage`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D310/C0 | 1660 | input | New queue name | S: manually reviewed label/layout binding; association inferred, not runtime observed D310/C1 |
| D310/C1 | 1686 | text-output-or-label | Enter a name for the new queue that will be displayed in the list of queues | S: explicit resource caption |
| D310/C2 | 1 | action | OK | S: explicit resource caption |
| D310/C3 | 2 | action | Cancel | S: explicit resource caption |

## D332 — IDM Queues

Target family: `queue.schedule`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D332/C0 | 1 | action | OK | S: explicit resource caption |
| D332/C1 | 1685 | text-output-or-label | You have scheduled the start time of queue processing.  Please note that in order to start the queue at the specified time, it's necessary to have IDM running at that time at least in the taskbar notification area - tray (see the picture below) | S: explicit resource caption |
| D332/C2 | 1802 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |

## D334 —  Chrome, Edge, IE 

Target family: `browser.integration`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D334/C0 | 1691 | boolean-choice | Download with IDM | S: explicit resource caption |
| D334/C1 | 1694 | boolean-choice | Download last requested FLV video with IDM | S: explicit resource caption |
| D334/C2 | 1695 | boolean-choice | Download FLV videos with IDM from 10 last requested | S: explicit resource caption |
| D334/C3 | 1693 | boolean-choice | Download all links with IDM | S: explicit resource caption |
| D334/C4 | 1696 | text-output-or-label | Selected menu items will appear in browser context menu: | S: explicit resource caption |

## D335 — Firefox and other Mozilla based

Target family: `browser.firefox`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D335/C0 | 1691 | boolean-choice | Download with IDM | S: explicit resource caption |
| D335/C1 | 1694 | boolean-choice | Download last requested FLV video with IDM | S: explicit resource caption |
| D335/C2 | 1695 | boolean-choice | Download FLV videos with IDM from 10 last requested | S: explicit resource caption |
| D335/C3 | 1693 | boolean-choice | Download all links with IDM | S: explicit resource caption |
| D335/C4 | 1696 | text-output-or-label | Selected menu items will appear in browser context menu: | S: explicit resource caption |

## D336 — Report for IDM developers

Target family: `app.report`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D336/C0 | 1390 | action | Send | S: explicit resource caption |
| D336/C1 | 2 | action | Cancel | S: explicit resource caption |
| D336/C2 | 1699 | read-only-output | Reported capture exception URL | S: manually reviewed label/layout binding; association inferred, not runtime observed D336/C5 |
| D336/C3 | 1701 | read-only-output | Reported capture origin/referrer URL | S: manually reviewed label/layout binding; association inferred, not runtime observed D336/C6 |
| D336/C4 | 1702 | text-output-or-label | You can send a report to IDM developers about the downloads that were taken over by mistake to make a workaround for this site in the future versions of IDM | S: explicit resource caption |
| D336/C5 | 1698 | text-output-or-label | Added the following exception: | S: explicit resource caption |
| D336/C6 | 1700 | text-output-or-label | The referrer URL from which the download was taken over by mistake: | S: explicit resource caption |

## D337 — Speed Limiter settings

Target family: `speed.task`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D337/C0 | 1 | action | OK | S: explicit resource caption |
| D337/C1 | 2 | action | Cancel | S: explicit resource caption |
| D337/C2 | 1660 | input | Maximum speed for one file in KBytes/sec | S: manually reviewed label/layout binding; association inferred, not runtime observed D337/C4,D337/C5 |
| D337/C3 | 1704 | boolean-choice | Always turn on Speed Limiter on IDM startup | S: explicit resource caption |
| D337/C4 | 1708 | text-output-or-label | Maximum download speed for one file | S: explicit resource caption |
| D337/C5 | 1709 | text-output-or-label | KBytes/sec | S: explicit resource caption |
| D337/C6 | 1710 | text-output-or-label | Note: Internet servers may break connection when the speed is too limited! Thus it's not recommened to use Speed Limiter to download from servers that do not support "resume" feature. | S: explicit resource caption |

## D338 — Confirm deletion of downloads

Target family: `task.remove`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D338/C0 | 1715 | text-output-or-label | Are you sure you want to delete selected downloads from IDM list of downloads? | S: explicit resource caption |
| D338/C1 | 1713 | boolean-choice | Delete completely downloaded files from your hard disk as well. Be careful ! | S: explicit resource caption |
| D338/C2 | 1163 | action | Yes | S: explicit resource caption |
| D338/C3 | 1202 | action | No | S: explicit resource caption |
| D338/C4 | 1654 | boolean-choice | Don't show this dialog again | S: explicit resource caption |

## D339 — Confirm deletion of downloads

Target family: `task.remove`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D339/C0 | 1654 | boolean-choice | Don't show this dialog again | S: explicit resource caption |
| D339/C1 | 1163 | action | Yes | S: explicit resource caption |
| D339/C2 | 1202 | action | No | S: explicit resource caption |
| D339/C3 | 1716 | text-output-or-label | The selected downloads are not complete. Are you sure you want to delete them from IDM list of downloads? | S: explicit resource caption |

## D340 — (untitled template)

Target family: `media.overlay`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D340/C0 | 1727 | text-output-or-label | IDM can show its Download panel on a web-player in a browser when IDM detects a multimedia request from the web-player | S: explicit resource caption |
| D340/C1 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D340/C2 | 1728 | text-output-or-label | Panel view: | S: explicit resource caption |
| D340/C3 | 1717 | exclusive-choice | Full mode | S: explicit resource caption |
| D340/C4 | 1722 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D340/C5 | 1718 | exclusive-choice | Mini mode | S: explicit resource caption |
| D340/C6 | 1723 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D340/C7 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D340/C8 | 1726 | text-output-or-label | Show IDM download panel for the following file types: | S: explicit resource caption |
| D340/C9 | 1023 | collection | Media panel captured file type collection | S: manually reviewed label/layout binding; association inferred, not runtime observed D340/C8 |
| D340/C10 | 1423 | action | Add | S: explicit resource caption |
| D340/C11 | 1822 | action | Minimum size | S: explicit resource caption |
| D340/C12 | 1742 | action | Check All | S: explicit resource caption |
| D340/C13 | 1743 | action | Clear All | S: explicit resource caption |
| D340/C14 | 1787 | action | Exceptions | S: explicit resource caption |
| D340/C15 | 1725 | text-output-or-label | Note: | S: explicit resource caption |
| D340/C16 | 1720 | boolean-choice | Don't capture downloads from web-players automatically | S: explicit resource caption |
| D340/C17 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D340/C18 | 1762 | boolean-choice | Show download panel for protected content   which IDM may not download | S: explicit resource caption |

## D341 — Add file type

Target family: `browser.types`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D341/C0 | 1730 | input | Media panel file extension | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D341/C1 | 1 | action | OK | S: explicit resource caption |
| D341/C2 | 2 | action | Cancel | S: explicit resource caption |

## D342 — IDM

Target family: `platform.privilege`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D342/C0 | 1 | action | OK | S: explicit resource caption |
| D342/C1 | 1731 | text-output-or-label | Warning: You are running IDM with Administrator privileges.   IDM won't be able to take over downloads from browsers in this mode. | S: explicit resource caption |
| D342/C2 | 1654 | boolean-choice | Don't show this dialog again | S: explicit resource caption |

## D344 — IDM

Target family: `task.wait`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D344/C0 | 1732 | action | Stop waiting | S: explicit resource caption |
| D344/C1 | 1733 | text-output-or-label | Waiting/reconnect status message | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D345 — (untitled template)

Target family: `queue.assignment`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D345/C0 | 1 | action | OK | S: explicit resource caption |
| D345/C1 | 1443 | choice-or-input | Destination queue selection | S: manually reviewed label/layout binding; association inferred, not runtime observed D345/C2 |
| D345/C2 | 1735 | text-output-or-label | Add files to the queue: | S: explicit resource caption |
| D345/C3 | 1736 | boolean-choice | Start queue processing | S: explicit resource caption |
| D345/C4 | 1737 | boolean-choice | Don't ask me again | S: explicit resource caption |
| D345/C5 | 1740 | action | Create destination queue | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D348 — Dialog

Target family: `browser.selection_panel`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D348/C0 | 1749 | text-output-or-label | IDM can show Download panel on a web-page when you select a text that contains download links. | S: explicit resource caption |
| D348/C1 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D348/C2 | 1744 | exclusive-choice | Show Download panel for any selected links | S: explicit resource caption |
| D348/C3 | 1745 | exclusive-choice | Don't show Download panel for selected links | S: explicit resource caption |
| D348/C4 | 1746 | exclusive-choice | Show Download panel for selected links for the following sites only: | S: explicit resource caption |
| D348/C5 | 1747 | collection | Selected-link panel enabled site collection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D348/C6 | 1345 | action | Delete | S: explicit resource caption |
| D348/C7 | 1423 | action | Add | S: explicit resource caption |
| D348/C8 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D348/C9 | 1728 | text-output-or-label | Panel view: | S: explicit resource caption |
| D348/C10 | 1717 | exclusive-choice | Full mode | S: explicit resource caption |
| D348/C11 | 1718 | exclusive-choice | Mini mode | S: explicit resource caption |
| D348/C12 | 1722 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D348/C13 | 1723 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |

## D349 — Enter site name

Target family: `browser.selection_panel`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D349/C0 | 1 | action | OK | S: explicit resource caption |
| D349/C1 | 2 | action | Cancel | S: explicit resource caption |
| D349/C2 | 1750 | input | Selected-link panel site name | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D352 — Exit IDM

Target family: `app.exit`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D352/C0 | 1752 | text-output-or-label | Saving settings... | S: explicit resource caption |

## D363 — Download status

Target family: `task.progress`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D363/C0 | 1156 | text-output-or-label | Download source URL | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D363/C1 | 1299 | text-output-or-label | Status | S: explicit resource caption |
| D363/C2 | 1030 | text-output-or-label | Connection/download status value | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D363/C3 | 1300 | text-output-or-label | File size | S: explicit resource caption |
| D363/C4 | 1154 | text-output-or-label | File size value | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D363/C5 | 1301 | text-output-or-label | Downloaded | S: explicit resource caption |
| D363/C6 | 1147 | text-output-or-label | Downloaded byte count | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D363/C7 | 1302 | text-output-or-label | Transfer rate | S: explicit resource caption |
| D363/C8 | 1151 | text-output-or-label | Transfer rate value | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D363/C9 | 1303 | text-output-or-label | Time left | S: explicit resource caption |
| D363/C10 | 1149 | text-output-or-label | Time remaining value | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D363/C11 | 1304 | text-output-or-label | Resume capability | S: explicit resource caption |
| D363/C12 | 1175 | text-output-or-label | Resume capability value | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D364 — Options on completion

Target family: `task.completion`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D364/C0 | 1759 | text-output-or-label | Save To: | S: explicit resource caption |
| D364/C1 | 1236 | boolean-choice | Show download complete dialog | S: explicit resource caption |
| D364/C2 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D364/C3 | 1761 | text-output-or-label | Power options | S: explicit resource caption |
| D364/C4 | 1227 | boolean-choice | Hang up modem when done | S: manually reviewed label/layout binding; association inferred, not runtime observed D364/C10 |
| D364/C5 | 1271 | boolean-choice | Exit application when done | S: manually reviewed label/layout binding; association inferred, not runtime observed D364/C11 |
| D364/C6 | 1228 | boolean-choice | Turn off computer when done | S: manually reviewed label/layout binding; association inferred, not runtime observed D364/C12 |
| D364/C7 | 1443 | choice-or-input | Completion power action selection | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D364/C8 | 1235 | boolean-choice | Use this option in an emergency. This can cause the applications to lose data. | S: shipped tooltip; association by same control ID is a candidate until caller verification D364/C13 |
| D364/C9 | 1772 | action | Hide tab | S: explicit resource caption |
| D364/C10 | 1846 | text-output-or-label | Hang up modem when done | S: explicit resource caption |
| D364/C11 | 1847 | text-output-or-label | Exit Internet Download Manager when done | S: explicit resource caption |
| D364/C12 | 1848 | text-output-or-label | Turn off computer when done | S: explicit resource caption |
| D364/C13 | 1845 | text-output-or-label | Force processes to terminate | S: explicit resource caption |

## D365 — Speed Limiter

Target family: `speed.task`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D365/C0 | 1707 | boolean-choice | Use Speed Limiter | S: explicit resource caption |
| D365/C1 | 1764 | input | Maximum task speed in KBytes/sec | S: manually reviewed label/layout binding; association inferred, not runtime observed D365/C2,D365/C3 |
| D365/C2 | 1763 | text-output-or-label | Maximum download speed: | S: explicit resource caption |
| D365/C3 | 1709 | text-output-or-label | KBytes/sec | S: explicit resource caption |
| D365/C4 | 1302 | text-output-or-label | Transfer rate | S: explicit resource caption |
| D365/C5 | 1151 | text-output-or-label | Current task transfer rate | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D365/C6 | 1765 | boolean-choice | Remember Speed Limiter settings for this file  on download stop/resume | S: explicit resource caption |
| D365/C7 | 1772 | action | Hide tab | S: explicit resource caption |

## D367 — Customize download progress dialog

Target family: `task.progress`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D367/C0 | 1 | action | OK | S: explicit resource caption |
| D367/C1 | 2 | action | Cancel | S: explicit resource caption |
| D367/C2 | 1768 | boolean-choice | Show "Speed Limiter" tab | S: explicit resource caption |
| D367/C3 | 1769 | boolean-choice | Show "Options on completion" tab | S: explicit resource caption |
| D367/C4 | 1773 | boolean-choice | Show "Hide tab" buttons | S: explicit resource caption |
| D367/C5 | 1207 | choice-or-input | Initial progress window view | S: manually reviewed label/layout binding; association inferred, not runtime observed D367/C6 |
| D367/C6 | 1775 | text-output-or-label | Start view: | S: explicit resource caption |
| D367/C7 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D367/C8 | 1774 | text-output-or-label | Note: "Start view" option doesn't relate to queue processing. When IDM is processing queues,  "Download progress" dialog always starts minimized in system tray | S: explicit resource caption |

## D369 — Advanced proxy settings

Target family: `settings.proxy`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D369/C0 | 1114 | group-container | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D369/C1 | 1102 | boolean-choice | Use HTTP Proxy | S: explicit resource caption |
| D369/C2 | 1122 | text-output-or-label | Proxy | S: explicit resource caption |
| D369/C3 | 1123 | text-output-or-label | Port | S: explicit resource caption |
| D369/C4 | 1124 | text-output-or-label | UserName | S: explicit resource caption |
| D369/C5 | 1125 | text-output-or-label | Password | S: explicit resource caption |
| D369/C6 | 1103 | input | HTTP proxy address | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C2 |
| D369/C7 | 1104 | input | HTTP proxy port | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C3 |
| D369/C8 | 1105 | input | HTTP proxy username | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C4 |
| D369/C9 | 1106 | input | HTTP proxy password | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C5 |
| D369/C10 | 1142 | group-container | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D369/C11 | 1118 | boolean-choice | Use HTTPS Proxy | S: explicit resource caption |
| D369/C12 | 1133 | text-output-or-label | Proxy | S: explicit resource caption |
| D369/C13 | 1135 | text-output-or-label | Port | S: explicit resource caption |
| D369/C14 | 1137 | text-output-or-label | UserName | S: explicit resource caption |
| D369/C15 | 1139 | text-output-or-label | Password | S: explicit resource caption |
| D369/C16 | 1119 | input | HTTPS proxy address | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C12 |
| D369/C17 | 1121 | input | HTTPS proxy port | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C13 |
| D369/C18 | 1130 | input | HTTPS proxy username | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C14 |
| D369/C19 | 1131 | input | HTTPS proxy password | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C15 |
| D369/C20 | 1115 | group-container | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D369/C21 | 1108 | boolean-choice | Use FTP Proxy | S: explicit resource caption |
| D369/C22 | 1126 | text-output-or-label | Proxy | S: explicit resource caption |
| D369/C23 | 1127 | text-output-or-label | Port | S: explicit resource caption |
| D369/C24 | 1128 | text-output-or-label | UserName | S: explicit resource caption |
| D369/C25 | 1129 | text-output-or-label | Password | S: explicit resource caption |
| D369/C26 | 1109 | input | FTP proxy address | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C22 |
| D369/C27 | 1110 | input | FTP proxy port | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C23 |
| D369/C28 | 1111 | input | FTP proxy username | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C24 |
| D369/C29 | 1112 | input | FTP proxy password | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C25 |
| D369/C30 | 1832 | group-container | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D369/C31 | 1779 | boolean-choice | Use socks | S: explicit resource caption |
| D369/C32 | 1786 | text-output-or-label | Socks server address | S: explicit resource caption |
| D369/C33 | 1829 | text-output-or-label | Port | S: explicit resource caption |
| D369/C34 | 1830 | text-output-or-label | UserName | S: explicit resource caption |
| D369/C35 | 1831 | text-output-or-label | Password | S: explicit resource caption |
| D369/C36 | 1803 | input | SOCKS address | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C32 |
| D369/C37 | 1804 | input | SOCKS port | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C33 |
| D369/C38 | 1805 | input | SOCKS username | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C34 |
| D369/C39 | 1806 | input | SOCKS password | S: manually reviewed label/layout binding; association inferred, not runtime observed D369/C35 |
| D369/C40 | 1781 | exclusive-choice | Socks 5 | S: explicit resource caption |
| D369/C41 | 1780 | exclusive-choice | Socks 4 | S: explicit resource caption |
| D369/C42 | 1833 | boolean-choice | Proxy DNS | S: explicit resource caption |
| D369/C43 | 1863 | action | Get system | S: explicit resource caption |
| D369/C44 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D369/C45 | 1 | action | OK | S: explicit resource caption |
| D369/C46 | 2 | action | Cancel | S: explicit resource caption |
| D369/C47 | 1843 | text-output-or-label | Note:  IDM will use socks only for the protocols that are NOT checked above | S: explicit resource caption |

## D370 — Download panel exceptions

Target family: `media.overlay`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D370/C0 | 1788 | text-output-or-label | Don't show IDM download panel for the following sites: | S: explicit resource caption |
| D370/C1 | 1031 | input | Media panel excluded site wildcard patterns | S: manually reviewed label/layout binding; association inferred, not runtime observed D370/C0 |
| D370/C2 | 1789 | text-output-or-label | Separate names by spaces. You may use asterisk as a wildcard pattern | S: explicit resource caption |
| D370/C3 | 1 | action | OK | S: explicit resource caption |
| D370/C4 | 2 | action | Cancel | S: explicit resource caption |

## D381 — Google Chrome Integration

Target family: `browser.integration`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D381/C0 | 1 | action | Close | S: explicit resource caption |
| D381/C1 | 1797 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D381/C2 | 1798 | text-output-or-label | Chrome integration instruction text | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D381/C3 | 1799 | text-output-or-label | Chrome integration instruction text | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D487 — Find file

Target family: `gui.search`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D487/C0 | 1807 | text-output-or-label | File name or part of the name: | S: explicit resource caption |
| D487/C1 | 1808 | input | Filename search expression | S: manually reviewed label/layout binding; association inferred, not runtime observed D487/C0 |
| D487/C2 | 1809 | text-output-or-label | Description or part of the description: | S: explicit resource caption |
| D487/C3 | 1040 | input | Description search expression | S: manually reviewed label/layout binding; association inferred, not runtime observed D487/C2 |
| D487/C4 | 65535 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D487/C5 | 1811 | boolean-choice | Match case | S: explicit resource caption |
| D487/C6 | 1812 | boolean-choice | Match whole string only | S: explicit resource caption |
| D487/C7 | 1673 | action | Find | S: explicit resource caption |
| D487/C8 | 2 | action | Cancel | S: explicit resource caption |

## D488 — Minimum size

Target family: `media.minimum`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D488/C0 | 1 | action | OK | S: explicit resource caption |
| D488/C1 | 2 | action | Cancel | S: explicit resource caption |
| D488/C2 | 1819 | text-output-or-label | Minimum size for media file | S: explicit resource caption |
| D488/C3 | 1820 | text-output-or-label | Minimum media size explanation | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D488/C4 | 1660 | input | Minimum detected media size in KB | S: manually reviewed label/layout binding; association inferred, not runtime observed D488/C2,D488/C5 |
| D488/C5 | 65535 | text-output-or-label | KB | S: explicit resource caption |
| D488/C6 | 1821 | text-output-or-label | Note: | S: explicit resource caption |

## D489 — Integration into Microsoft Edge browser

Target family: `browser.integration`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D489/C0 | 1 | action | OK | S: explicit resource caption |
| D489/C1 | 1797 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D489/C2 | 1827 | text-output-or-label | Edge integration instruction text | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D489/C3 | 1828 | text-output-or-label | Then please wait for the installation to finish and run Edge browser. In a a few seconds Edge will show a notification of the new extension.  Please confirm by pressing on "Turn it on" button. See image below. | S: explicit resource caption |
| D489/C4 | 1798 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |

## D494 — Virus checking

Target family: `security.scanner`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D494/C0 | 1 | action | OK | S: explicit resource caption |
| D494/C1 | 2 | action | Cancel | S: explicit resource caption |
| D494/C2 | 1318 | text-output-or-label | Virus scanner program | S: explicit resource caption |
| D494/C3 | 1044 | input | Antivirus scanner executable | S: manually reviewed label/layout binding; association inferred, not runtime observed D494/C2 |
| D494/C4 | 1165 | action | Choose antivirus scanner executable | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D494/C5 | 1319 | text-output-or-label | Command line parameters | S: explicit resource caption |
| D494/C6 | 1073 | input | Antivirus command arguments | S: manually reviewed label/layout binding; association inferred, not runtime observed D494/C5 |
| D494/C7 | 1837 | text-output-or-label | Note: | S: explicit resource caption |

## D504 — (untitled template)

Target family: `legacy.empty`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|

## D505 — (untitled template)

Target family: `gui.search`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D505/C0 | 1856 | text-output-or-label | Find: | S: explicit resource caption |
| D505/C1 | 1808 | input | Quick filename search expression | S: manually reviewed label/layout binding; association inferred, not runtime observed D505/C0 |
| D505/C2 | 1673 | action | Find | S: explicit resource caption |
| D505/C3 | 1076 | action | Settings... | S: explicit resource caption |
| D505/C4 | 1854 | action | Close quick search bar | S: manually reviewed label/layout binding; association inferred, not runtime observed |

## D506 — Find file

Target family: `gui.search`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D506/C0 | 1860 | text-output-or-label | Find: | S: explicit resource caption |
| D506/C1 | 1808 | input | Multi-field search expression | S: manually reviewed label/layout binding; association inferred, not runtime observed D506/C0 |
| D506/C2 | 1861 | boolean-choice | File name or part of the name | S: explicit resource caption |
| D506/C3 | 1858 | boolean-choice | Description or part of the description | S: explicit resource caption |
| D506/C4 | 1859 | boolean-choice | Site name/Download link/Parent web page/Referer | S: explicit resource caption |
| D506/C5 | 4294967295 | decoration | Resource layout/container/image; no separate executable action established | S: resource class/style |
| D506/C6 | 1811 | boolean-choice | Match case | S: explicit resource caption |
| D506/C7 | 1812 | boolean-choice | Match whole string only | S: explicit resource caption |
| D506/C8 | 1673 | action | Find | S: explicit resource caption |
| D506/C9 | 2 | action | Cancel | S: explicit resource caption |

## D30721 — New

Target family: `legacy.generic`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|
| D30721/C0 | 65535 | text-output-or-label | New | S: explicit resource caption |
| D30721/C1 | 100 | collection | Generic framework new-item list; actual caller unresolved | S: manually reviewed label/layout binding; association inferred, not runtime observed |
| D30721/C2 | 1 | action | OK | S: explicit resource caption |
| D30721/C3 | 2 | action | Cancel | S: explicit resource caption |
| D30721/C4 | 57670 | action | Help | S: explicit resource caption |

## D30734 — (untitled template)

Target family: `legacy.empty`. Reachability: resource-presence; not every template proven reachable.

| Control | Source ID | Role | Semantic binding | Evidence / labels |
|---|---:|---|---|---|

