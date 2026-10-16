// Manual headless Firefox check of the video panel on a real page (not CI):
//   XDM_FIREFOX=/path/firefox XDM_GECKODRIVER=/path/geckodriver node tests/manual/real_site_video_firefox.mjs <page-url>
// Installs the unpacked extension as a temporary add-on in a throwaway profile, opens the page, and prints the
// panel host and the formats the panel menu offers (through the extension's own page.videoMenu message).
import { spawn } from 'node:child_process';
import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const firefox = process.env.XDM_FIREFOX, geckodriver = process.env.XDM_GECKODRIVER;
if (!firefox || !geckodriver) throw new Error('Set XDM_FIREFOX and XDM_GECKODRIVER');
const pageUrl = process.argv[2] || 'https://www.youtube.com/watch?v=jNQXAC9IVRw';
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'xdm-ff-video-'));
const home = path.join(temp, 'home'); fs.mkdirSync(home, { recursive: true });
const env = { ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, '.config'), XDM_DATA_DIR: path.join(temp, 'state'), MOZ_HEADLESS: '1' };
const UUID = '5b1f0c1e-7d52-4b8e-9a0b-2f3c4d5e6f70';
const delay = ms => new Promise(r => setTimeout(r, ms));
const children = [];
const port = 4800 + Math.floor(Math.random() * 300);
const driver = spawn(geckodriver, ['--allow-system-access', '--port', String(port), '--host', '127.0.0.1'], { env, stdio: 'ignore' });
children.push(driver);
const wd = `http://127.0.0.1:${port}`;
let session;
const call = async (method, route, body) => { const r = await fetch(`${wd}${route}`, { method, headers: { 'content-type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body) }); const j = await r.json(); if (j.value?.error) throw new Error(`${route}: ${j.value.error}: ${j.value.message}`); return j.value; };
const s = (m, r, b) => call(m, `/session/${session}${r}`, b);
const exec = (script, ...args) => s('POST', '/execute/async', { script: `const done = arguments[arguments.length - 1]; (async () => { ${script} })().then(done, e => done({ __error: String(e) }));`, args }).then(v => { if (v?.__error) throw new Error(v.__error); return v; });
try {
  for (let i = 0; i < 100; i++) { if (await fetch(`${wd}/status`).then(r => r.ok, () => false)) break; await delay(100); }
  session = (await call('POST', '/session', { capabilities: { alwaysMatch: { browserName: 'firefox', 'moz:firefoxOptions': { binary: firefox, args: ['-headless'], prefs: {
    'extensions.webextensions.uuids': JSON.stringify({ 'xdm@xdm.local': UUID }), 'datareporting.policy.dataSubmissionEnabled': false } } } } })).sessionId;
  await s('POST', '/moz/addon/install', { path: path.join(root, 'extensions/firefox'), temporary: true });
  const pageTab = await s('GET', '/window');
  const before = await s('GET', '/window/handles');
  await s('POST', '/moz/context', { context: 'chrome' });
  await s('POST', '/execute/sync', { script: 'gBrowser.selectedTab = gBrowser.addTrustedTab(arguments[0]);', args: [`moz-extension://${UUID}/options.html`] });
  await s('POST', '/moz/context', { context: 'content' });
  const consoleTab = (await s('GET', '/window/handles')).find(h => !before.includes(h));
  await s('POST', '/window', { handle: pageTab });
  await s('POST', '/url', { url: pageUrl });
  await delay(Number(process.env.XDM_PAGE_WAIT || 20000));
  const host = await exec(`const e = document.querySelector('.xdm-player-panel'); if (!e) return null; const r = e.getBoundingClientRect(); return {x: r.x, y: r.y, w: r.width, display: getComputedStyle(e).display, label: null};`);
  console.log('panel host', JSON.stringify(host));
  await s('POST', '/window', { handle: consoleTab });
  const menu = await exec(`const tabs = await browser.tabs.query({}); const tab = tabs.find(t => !t.url.startsWith('moz-extension') && /^https?:/.test(t.url)); return await browser.tabs.sendMessage(tab.id, {type: 'page.videoMenu'}, {frameId: 0});`);
  console.log('menu', JSON.stringify(menu));
  if (!host || !menu?.options?.length) process.exitCode = 1;
} catch (e) { console.error(String(e)); process.exitCode = 1; }
finally { for (const c of children) c.kill('SIGTERM'); await delay(500); fs.rmSync(temp, { recursive: true, force: true }); }
