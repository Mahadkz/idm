// Manual, headless acceptance run of the video panel against a real site. NOT part of CI (needs the network
// and a site that still behaves the same); run before a release:
//   XDM_CHROME_FOR_TESTING=/path/to/chrome node tests/manual/real_site_video.mjs <page-url> [quality-label] [--shots DIR]
// Starts a throwaway daemon, a throwaway Chrome profile with the extension, opens the page, opens the panel
// menu, chooses the requested entry (default: the 480p MP4 / smallest video) and waits for the finished file.
// Screenshots of the panel and the open menu go to --shots (default /tmp/xdm-video-shots).
import { spawn } from 'node:child_process';
import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const chromeBinary = process.env.XDM_CHROME_FOR_TESTING;
if (!chromeBinary || !fs.existsSync(chromeBinary)) throw new Error('Set XDM_CHROME_FOR_TESTING');
const args = process.argv.slice(2);
const shotsAt = args.includes('--shots') ? args[args.indexOf('--shots') + 1] : '/tmp/xdm-video-shots';
const positional = args.filter((a, i) => !a.startsWith('--') && args[i - 1] !== '--shots');
const pageUrl = positional[0] || 'https://www.youtube.com/watch?v=aqz-KE-bpKQ';
const wanted = positional[1] || 'MP4 480p';
const tag = process.env.XDM_SHOT_TAG || new URL(pageUrl).hostname.replace(/\W/g, '_');
fs.mkdirSync(shotsAt, { recursive: true });
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'xdm-real-video-'));
const home = path.join(temp, 'home'); const profile = path.join(temp, 'profile');
for (const d of [home, profile]) fs.mkdirSync(d, { recursive: true });
const env = { ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, '.config'), XDG_DOWNLOAD_DIR: path.join(home, 'Downloads'), XDM_DATA_DIR: path.join(temp, 'state') };
const children = [];
const delay = ms => new Promise(r => setTimeout(r, ms));
const child = (cmd, a, o = {}) => { const p = spawn(cmd, a, { env, stdio: ['ignore', 'pipe', 'pipe'], ...o }); children.push(p); return p; };
async function until(fn, timeout = 20000) { const end = Date.now() + timeout; while (Date.now() < end) { const v = await fn(); if (v) return v; await delay(200); } throw new Error('timeout'); }
class Cdp {
  constructor(url) { this.ws = new WebSocket(url); this.id = 0; this.pend = new Map(); this.ws.onmessage = e => { const m = JSON.parse(e.data); if (m.id && this.pend.has(m.id)) { this.pend.get(m.id)(m); this.pend.delete(m.id); } }; }
  async open() { await new Promise(r => this.ws.onopen = r); return this; }
  call(method, params = {}) { return new Promise(r => { const i = ++this.id; this.pend.set(i, r); this.ws.send(JSON.stringify({ id: i, method, params })); }).then(m => { if (m.error) throw new Error(JSON.stringify(m.error)); return m.result; }); }
  async eval(ex) { const r = await this.call('Runtime.evaluate', { expression: ex, returnByValue: true, awaitPromise: true }); if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 400)); return r.result.value; }
}
const find = (n, pred, out = []) => { if (pred(n)) out.push(n); for (const c of [...(n.children || []), ...(n.shadowRoots || []), ...(n.contentDocument ? [n.contentDocument] : [])]) find(c, pred, out); return out; };
const cls = n => (n.attributes || []).join(' ');
const report = { pageUrl, wanted };
try {
  const daemon = child(path.join(root, 'target/debug/xdm-daemon'), []);
  await new Promise((res, rej) => { let b = ''; daemon.stderr.on('data', c => { b += c; if (/listening locally/.test(b)) res(); }); setTimeout(() => rej(new Error('daemon start')), 10000); });
  const chrome = child(chromeBinary, ['--headless=new', '--no-sandbox', '--disable-gpu', '--no-first-run', '--remote-allow-origins=*', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
    `--disable-extensions-except=${path.join(root, 'extensions/chromium')}`, `--load-extension=${path.join(root, 'extensions/chromium')}`,
    '--autoplay-policy=no-user-gesture-required', '--mute-audio', '--window-size=1280,800', 'about:blank']);
  const portFile = path.join(profile, 'DevToolsActivePort');
  const port = await until(() => fs.existsSync(portFile) && Number(fs.readFileSync(portFile, 'utf8').split('\n')[0]));
  const list = async () => (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const sw = await until(async () => {
    for (const t of (await list()).filter(x => x.type === 'service_worker' && x.url.endsWith('/background.js'))) {
      const c = await new Cdp(t.webSocketDebuggerUrl).open();
      const name = await c.eval('chrome.runtime.getManifest().name').catch(() => '');
      c.ws.close();
      if (name === 'XDM Browser Bridge') return t;
    }
    return null;
  });
  const extId = new URL(sw.url).hostname;
  const manifest = { name: 'org.xdm.browser', description: 'XDM test host', path: path.join(root, 'target/debug/xdm-native-host'), type: 'stdio', allowed_origins: [`chrome-extension://${extId}/`] };
  for (const dir of [path.join(profile, 'NativeMessagingHosts'), ...['google-chrome', 'chromium', 'google-chrome-for-testing'].map(b => path.join(home, '.config', b, 'NativeMessagingHosts'))]) { fs.mkdirSync(dir, { recursive: true }); fs.writeFileSync(path.join(dir, 'org.xdm.browser.json'), JSON.stringify(manifest)); }
  const worker = await new Cdp(sw.webSocketDebuggerUrl).open();
  report.hello = await worker.eval("chrome.runtime.sendNativeMessage('org.xdm.browser', {version:1,type:'hello'}).then(r=>r,e=>String(e))");
  report.extId = extId;
  const page = await new Cdp((await list()).find(t => t.type === 'page').webSocketDebuggerUrl).open();
  await page.call('Page.enable');
  await page.call('Page.navigate', { url: pageUrl });
  await delay(Number(process.env.XDM_PAGE_WAIT || 20000));
  if (process.env.XDM_SCROLL_VIDEO) { await page.eval("document.querySelector('video')?.scrollIntoView({block:'center'})"); await delay(4000); }
  const doc = async () => (await page.call('DOM.getDocument', { depth: -1, pierce: true })).root;
  report.panelHosts = find(await doc(), n => n.nodeName === 'DIV' && cls(n).includes('xdm-player-panel')).length;
  const shot = async name => { const { data } = await page.call('Page.captureScreenshot', { format: 'png' }); const f = path.join(shotsAt, `${tag}-${name}.png`); fs.writeFileSync(f, Buffer.from(data, 'base64')); report[`shot_${name}`] = f; };
  await shot('1-panel');
  const click = async node => { const box = await page.call('DOM.getBoxModel', { nodeId: node.nodeId }); const q = box.model.content; const x = (q[0] + q[2]) / 2, y = (q[1] + q[5]) / 2; for (const type of ['mousePressed', 'mouseReleased']) await page.call('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1 }); };
  const button = find(await doc(), n => n.nodeName === 'BUTTON' && cls(n).includes('xdm-download-btn'))[0];
  if (!button) throw new Error('no panel button: ' + JSON.stringify(report));
  await click(button);
  const menuNodes = await until(async () => { const n = find(await doc(), x => x.nodeName === 'BUTTON' && cls(x).includes('opt')); return n.length ? n : null; }, 40000);
  const labels = [];
  for (const n of menuNodes) labels.push((await page.call('DOM.getOuterHTML', { nodeId: n.nodeId })).outerHTML.replace(/<[^>]+>/g, '|').replace(/\|+/g, ' ').trim());
  report.menu = labels;
  await shot('2-menu');
  const index = labels.findIndex(l => l.startsWith(wanted));
  if (index < 0) throw new Error(`no menu entry "${wanted}": ${labels.join(' / ')}`);
  await click(menuNodes[index]);
  await delay(3000);
  await shot('3-after-choice');
  const note = find(await doc(), n => n.nodeName === 'DIV' && cls(n).includes('note')).pop();
  report.note = note ? (await page.call('DOM.getOuterHTML', { nodeId: note.nodeId })).outerHTML.replace(/<[^>]+>/g, '') : null;
  const downloads = path.join(home, 'Downloads');
  const finished = await until(() => { const dir = path.join(downloads, 'Video'); if (!fs.existsSync(dir)) return null; const files = fs.readdirSync(dir).filter(f => !/\.(part|xdm-)/.test(f)); return files.length ? path.join(dir, files[0]) : null; }, Number(process.env.XDM_DOWNLOAD_WAIT || 600000));
  report.file = finished; report.bytes = fs.statSync(finished).size;
  console.log(JSON.stringify(report, null, 2));
  worker.ws.close(); page.ws.close();
} catch (error) {
  console.error(String(error)); console.error(JSON.stringify(report, null, 2)); process.exitCode = 1;
} finally {
  for (const p of children.reverse()) if (p.exitCode === null) p.kill('SIGTERM');
  await delay(500);
  if (process.env.XDM_KEEP_TEST_TEMP) console.error('kept', temp); else fs.rmSync(temp, { recursive: true, force: true });
}
