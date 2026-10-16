// Real Firefox handoff: headless Firefox driven by geckodriver (WebDriver classic), the unpacked
// extension installed as a temporary add-on, a native-messaging manifest in a throwaway HOME, and an
// isolated xdm-daemon. Run after cargo build: XDM_FIREFOX=/path/to/firefox [XDM_GECKODRIVER=/path] node tests/integration/firefox_real.mjs
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const exe = process.platform === 'win32' ? '.exe' : '';
const onPath = name => {
  const found = spawnSync(process.platform === 'win32' ? 'where' : 'which', [name], { encoding: 'utf8' });
  return found.status === 0 ? found.stdout.split(/\r?\n/)[0] : '';
};
const firefoxBinary = process.env.XDM_FIREFOX;
const geckodriver = process.env.XDM_GECKODRIVER || onPath('geckodriver');
if (!firefoxBinary || !fs.existsSync(firefoxBinary)) throw new Error('Set XDM_FIREFOX to a Firefox executable');
if (!geckodriver) throw new Error('geckodriver not found: set XDM_GECKODRIVER or put it on PATH');

const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'xdm-firefox-real-'));
const home = path.join(temp, 'home');
const browserDownloads = path.join(temp, 'firefox-downloads');
for (const directory of [home, browserDownloads]) fs.mkdirSync(directory, { recursive: true });
const env = { ...process.env, HOME: home, USERPROFILE: home, XDG_CONFIG_HOME: path.join(home, '.config'), XDM_DATA_DIR: path.join(temp, 'state'), MOZ_HEADLESS: '1' };
const children = [];
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const EXTENSION_UUID = '5b1f0c1e-7d52-4b8e-9a0b-2f3c4d5e6f70';

async function until(fn, timeout = 20000, what = 'condition') {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const value = await fn();
    if (value) return value;
    await delay(100);
  }
  throw new Error(`Timed out waiting for ${what}`);
}
function child(command, args) {
  const proc = spawn(command, args, { env, stdio: ['ignore', 'pipe', 'pipe'] });
  proc.output = '';
  for (const channel of ['stdout', 'stderr']) proc[channel].on('data', chunk => { proc.output = (proc.output + chunk).slice(-8000); });
  children.push(proc);
  return proc;
}

// Fixture: page (sets a cookie), an attachment, a media response; every request is recorded.
const SIZE = 256 * 1024;
const payload = Buffer.alloc(SIZE, 0x5a);
const requests = [];
const server = http.createServer((req, res) => {
  requests.push({ path: req.url, headers: req.headers });
  if (req.url === '/page.html') {
    res.writeHead(200, { 'Content-Type': 'text/html', 'Set-Cookie': 'sid=firefox-cookie-1; Path=/' });
    res.end(`<!doctype html><title>fixture</title>
<a id="dl" href="/files/doc.bin">download</a>
<a id="dl2" href="/files/fallback.bin">fallback</a>
<script>fetch('/media/clip.mp4').then(r => r.arrayBuffer());</script>`);
  } else if (req.url.startsWith('/files/')) {
    const name = req.url.split('/').pop();
    res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Length': SIZE, 'Content-Disposition': `attachment; filename="${name}"`, 'Accept-Ranges': 'bytes' });
    res.end(payload);
  } else if (req.url.startsWith('/media/')) {
    res.writeHead(200, { 'Content-Type': 'video/mp4', 'Content-Length': 4096 });
    res.end(Buffer.alloc(4096));
  } else { res.writeHead(404); res.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const base = `http://127.0.0.1:${server.address().port}`;

let session = null; let wd = '';
async function call(method, route, body) {
  const reply = await fetch(`${wd}${route}`, { method, headers: { 'content-type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body) });
  const json = await reply.json();
  if (json.value?.error) throw new Error(`${method} ${route}: ${json.value.error}: ${json.value.message}`);
  return json.value;
}
const s = (method, route, body) => call(method, `/session/${session}${route}`, body);
const exec = (script, ...args) => s('POST', '/execute/async', { script: `const done = arguments[arguments.length - 1]; (async () => { ${script} })().then(done, e => done({ __error: String(e) }));`, args })
  .then(value => { if (value?.__error) throw new Error(value.__error); return value; });

try {
  const daemon = child(path.join(root, `target/debug/xdm-daemon${exe}`), []);
  await until(() => /listening locally/.test(daemon.output), 15000, 'daemon');

  // Native messaging manifest (Linux layout under the throwaway HOME).
  const manifest = { name: 'org.xdm.browser', description: 'XDM test host', path: path.join(root, `target/debug/xdm-native-host${exe}`), type: 'stdio', allowed_extensions: ['xdm@xdm.local'] };
  const hosts = path.join(home, '.mozilla', 'native-messaging-hosts');
  fs.mkdirSync(hosts, { recursive: true });
  fs.writeFileSync(path.join(hosts, 'org.xdm.browser.json'), JSON.stringify(manifest));

  const driverPort = 4400 + Math.floor(Math.random() * 400);
  const driver = child(geckodriver, ['--allow-system-access', '--port', String(driverPort), '--host', '127.0.0.1']);
  wd = `http://127.0.0.1:${driverPort}`;
  await until(() => fetch(`${wd}/status`).then(r => r.ok, () => false), 15000, 'geckodriver');
  const created = await call('POST', '/session', { capabilities: { alwaysMatch: {
    browserName: 'firefox', acceptInsecureCerts: true,
    'moz:firefoxOptions': { binary: firefoxBinary, args: ['-headless'], prefs: {
      'extensions.webextensions.uuids': JSON.stringify({ 'xdm@xdm.local': EXTENSION_UUID }),
      'browser.download.folderList': 2, 'browser.download.dir': browserDownloads, 'browser.download.useDownloadDir': true,
      'browser.download.always_ask_before_handling_new_types': false, 'browser.download.panel.shown': true,
      'network.proxy.type': 0, 'datareporting.policy.dataSubmissionEnabled': false,
    } },
  } } });
  session = created.sessionId;
  const addon = await s('POST', '/moz/addon/install', { path: path.join(root, 'extensions/firefox'), temporary: true });
  assert.equal(addon, 'xdm@xdm.local');

  // Extension page used as a console (privileged: runtime.sendNativeMessage, storage, tabs).
  const pageTab = await s('GET', '/window');
  // WebDriver refuses to navigate to moz-extension: URLs, so the browser chrome opens the tab.
  const handlesBefore = await s('GET', '/window/handles');
  await s('POST', '/moz/context', { context: 'chrome' });
  await s('POST', '/execute/sync', { script: 'gBrowser.selectedTab = gBrowser.addTrustedTab(arguments[0]);', args: [`moz-extension://${EXTENSION_UUID}/options.html`] });
  await s('POST', '/moz/context', { context: 'content' });
  const consoleTab = await until(async () => (await s('GET', '/window/handles')).find(h => !handlesBefore.includes(h)), 10000, 'extension page tab');
  globalThis.consoleTabHandle = consoleTab;
  await s('POST', '/window', { handle: consoleTab });
  const hello = await exec(`return await browser.runtime.sendNativeMessage('org.xdm.browser', {version:1,type:'hello'});`);
  assert.equal(hello.type, 'hello.result', JSON.stringify(hello));
  await exec(`await browser.storage.local.set({interceptDownloads:true}); return true;`);
  // The background page reads interceptDownloads from storage on every download, so no restart is needed.

  await s('POST', '/window', { handle: pageTab });
  await s('POST', '/url', { url: `${base}/page.html` });
  await until(() => requests.some(r => r.path === '/media/clip.mp4'), 10000, 'page media fetch');

  // Media capture: the background page must have seen the video response for this tab.
  await s('POST', '/window', { handle: consoleTab });
  const media = await until(async () => {
    const tabs = await exec(`return (await browser.tabs.query({})).map(t => ({id: t.id, url: t.url}));`);
    const tab = tabs.find(t => t.url.endsWith('/page.html'));
    if (!tab) return null;
    const list = await exec(`return await browser.runtime.sendMessage({type:'media.list', tabId: arguments[0]});`, tab.id);
    return list.items.length ? { tab, items: list.items } : null;
  }, 10000, 'media.list entry');
  const clip = media.items.find(item => item.url === `${base}/media/clip.mp4`);
  assert.ok(clip, JSON.stringify(media.items));
  assert.equal(clip.kind, 'direct');
  assert.equal(clip.mime, 'video/mp4');
  assert.equal(clip.size, 4096);

  // Media handoff from the popup path (media.download) creates a daemon task through the native host.
  const mediaReply = await exec(`return await browser.runtime.sendMessage({type:'media.download', url: arguments[0], pageUrl: arguments[1]});`, `${base}/media/clip.mp4`, `${base}/page.html`);
  assert.equal(mediaReply.accepted, true, JSON.stringify(mediaReply));
  // The daemon files downloads under category subfolders (Downloads/Video/...).
  const downloads = path.join(home, 'Downloads');
  const located = (name, size) => {
    const walk = dir => fs.existsSync(dir) ? fs.readdirSync(dir, { withFileTypes: true }).flatMap(e => e.isDirectory() ? walk(path.join(dir, e.name)) : [path.join(dir, e.name)]) : [];
    return walk(downloads).find(f => path.basename(f) === name && fs.statSync(f).size === size);
  };
  await until(() => located('clip.mp4', 4096), 20000, 'daemon media download');

  // Download link: click it in the page. XDM must take it; Firefox must not save it.
  await s('POST', '/window', { handle: pageTab });
  const before = requests.length;
  const link = await s('POST', '/element', { using: 'css selector', value: '#dl' });
  await s('POST', `/element/${Object.values(link)[0]}/click`, {});
  const output = await until(() => located('doc.bin', SIZE), 20000, 'daemon download of doc.bin');
  assert.ok(fs.readFileSync(output).equals(payload));
  const mine = requests.slice(before).filter(r => r.path === '/files/doc.bin');
  assert.ok(mine.length >= 2, 'browser request plus the daemon fetch expected');
  // Every request for the file (browser and daemon) carries the page referrer; the daemon fetch carries the cookie.
  for (const r of mine) assert.equal(r.headers.referer, `${base}/page.html`, JSON.stringify(r.headers));
  assert.ok(mine.filter(r => r.headers.cookie === 'sid=firefox-cookie-1').length >= 2, `cookie must be forwarded: ${JSON.stringify(mine.map(r => r.headers))}`);
  assert.match(mine[mine.length - 1].headers['user-agent'] || '', /Firefox/);
  await delay(1500);
  assert.deepEqual(fs.readdirSync(browserDownloads), [], 'Firefox must not also save the file');
  const tasks = spawnSync(path.join(root, `target/debug/xdm-cli${exe}`), ['daemon', 'list'], { env, encoding: 'utf8' });
  assert.match(tasks.stdout, /\/files\/doc\.bin/, tasks.stdout + tasks.stderr);

  // Daemon unavailable: the handoff fails and Firefox keeps the download itself.
  daemon.kill('SIGTERM');
  await until(() => daemon.exitCode !== null || daemon.signalCode !== null, 10000, 'daemon exit');
  const link2 = await s('POST', '/element', { using: 'css selector', value: '#dl2' });
  await s('POST', `/element/${Object.values(link2)[0]}/click`, {});
  const kept = path.join(browserDownloads, 'fallback.bin');
  await until(() => fs.existsSync(kept) && fs.statSync(kept).size === SIZE, 30000, 'Firefox fallback download');
  assert.ok(!located('fallback.bin', SIZE), 'daemon must not have fetched the fallback file');

  await s('DELETE', '');
  session = null;
  console.log('real Firefox: temporary add-on install, native hello, media capture + media handoff, download handoff (URL, referrer, cookie, no browser copy) and daemon-unavailable fallback passed');
} catch (error) {
  for (const proc of children) console.error(`--- ${path.basename(proc.spawnfile)} output ---\n${proc.output}`);
  if (session) {
    try {
      await s('POST', '/window', { handle: globalThis.consoleTabHandle });
      console.error('Extension recent requests:', JSON.stringify(await exec(`const bg = await browser.runtime.getBackgroundPage(); return bg.eval('JSON.stringify([...recent.entries()])');`)));
      console.error('Browser downloads:', JSON.stringify(await exec(`return await browser.downloads.search({});`)));
    } catch (e) { console.error('could not query downloads:', String(e)); }
  }
  console.error('Browser download dir:', JSON.stringify(fs.readdirSync(browserDownloads)));
  console.error('Fixture requests:', JSON.stringify(requests.map(r => [r.path, r.headers.referer, r.headers.cookie, r.headers.range])));
  console.error(error); process.exitCode = 1;
} finally {
  if (session) await s('DELETE', '').catch(() => {});
  server.close();
  for (const proc of children.reverse()) if (proc.exitCode === null && proc.signalCode === null) proc.kill('SIGTERM');
  await delay(500);
  if (process.env.XDM_KEEP_TEST_TEMP) console.error('Kept test files:', temp);
  else fs.rmSync(temp, { recursive: true, force: true });
  process.exit(process.exitCode || 0);
}
