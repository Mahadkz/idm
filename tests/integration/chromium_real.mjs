// Real Linux Chromium-family handoff using Chrome for Testing and an isolated profile.
// Run after cargo build --workspace: XDM_CHROME_FOR_TESTING=/path/to/chrome node tests/integration/chromium_real.mjs
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const chromeBinary = process.env.XDM_CHROME_FOR_TESTING;
if (!chromeBinary || !fs.existsSync(chromeBinary)) throw new Error('Set XDM_CHROME_FOR_TESTING to a Chrome for Testing executable');
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'xdm-chromium-real-'));
const home = path.join(temp, 'home');
const profile = path.join(temp, 'profile');
for (const directory of [home, profile]) fs.mkdirSync(directory, { recursive: true });
const env = { ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, '.config'), XDM_DATA_DIR: path.join(temp, 'state') };
const children = [];
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

function child(command, args, options = {}) {
  const process = spawn(command, args, { env, stdio: ['ignore', 'pipe', 'pipe'], ...options });
  children.push(process);
  return process;
}
async function lineFrom(process, channel = 'stdout', timeout = 10000) {
  return await Promise.race([
    new Promise((resolve, reject) => {
      let buffer = '';
      process[channel].on('data', chunk => {
        buffer += chunk.toString();
        if (buffer.includes('\n')) resolve(buffer.slice(0, buffer.indexOf('\n')));
      });
      process.once('exit', code => reject(new Error(`${process.spawnfile} exited ${code}: ${buffer}`)));
    }),
    delay(timeout).then(() => { throw new Error(`Timed out waiting for ${process.spawnfile}`); }),
  ]);
}
async function until(fn, timeout = 15000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const value = await fn();
    if (value) return value;
    await delay(100);
  }
  throw new Error('Timed out waiting for condition');
}
async function targets(port) {
  return await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
}
class Cdp {
  constructor(url) {
    this.socket = new WebSocket(url);
    this.sequence = 0;
    this.pending = new Map();
    this.socket.onmessage = event => {
      const packet = JSON.parse(event.data);
      if (!packet.id) return;
      const pending = this.pending.get(packet.id);
      if (!pending) return;
      this.pending.delete(packet.id);
      packet.error ? pending.reject(new Error(JSON.stringify(packet.error))) : pending.resolve(packet.result);
    };
  }
  async open() {
    await new Promise((resolve, reject) => {
      this.socket.onopen = resolve;
      this.socket.onerror = reject;
    });
    return this;
  }
  call(method, params = {}) {
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }
  async eval(expression) {
    const reply = await this.call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (reply.exceptionDetails) throw new Error(JSON.stringify(reply.exceptionDetails));
    return reply.result.value;
  }
  close() { this.socket.close(); }
}

try {
  const fixture = child('python3', [path.join(root, 'docs/analysis/fault_fixture.py'), path.join(temp, 'http'), '--delay', '0.01']);
  const { port: httpPort } = JSON.parse(await lineFrom(fixture));
  const daemon = child(path.join(root, 'target/debug/xdm-daemon'), []);
  assert.match(await lineFrom(daemon, 'stderr'), /listening locally/);
  const chrome = child(chromeBinary, [
    ...(!process.env.XDM_CHROME_HEADED ? ['--headless=new'] : []), ...(process.env.XDM_CHROME_NO_SANDBOX ? ['--no-sandbox'] : []), '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
    `--user-data-dir=${profile}`, '--remote-debugging-port=0', '--remote-allow-origins=*',
    `--disable-extensions-except=${path.join(root, 'extensions/chromium')}`,
    `--load-extension=${path.join(root, 'extensions/chromium')}`, 'about:blank',
  ]);
  let chromeError = '';
  chrome.stderr.on('data', chunk => { chromeError += chunk.toString(); });
  const portFile = path.join(profile, 'DevToolsActivePort');
  const devtoolsPort = await until(() => fs.existsSync(portFile) && Number(fs.readFileSync(portFile, 'utf8').split('\n')[0]), 15000);
  const workerTarget = await until(async () => {
    const available = await targets(devtoolsPort);
    for (const target of available.filter(item => item.type === 'service_worker' && item.url.endsWith('/background.js'))) {
      const cdp = await new Cdp(target.webSocketDebuggerUrl).open();
      const name = await cdp.eval('chrome.runtime.getManifest().name').catch(() => '');
      cdp.close();
      if (name === 'XDM Browser Bridge') return target;
    }
    return null;
  });
  const extensionId = new URL(workerTarget.url).hostname;
  const manifest = {
    name: 'org.xdm.browser', description: 'XDM test host', path: path.join(root, 'target/debug/xdm-native-host'),
    type: 'stdio', allowed_origins: [`chrome-extension://${extensionId}/`],
  };
  for (const browser of ['google-chrome', 'chromium', 'google-chrome-for-testing']) {
    const directory = path.join(home, '.config', browser, 'NativeMessagingHosts');
    fs.mkdirSync(directory, { recursive: true });
    fs.writeFileSync(path.join(directory, 'org.xdm.browser.json'), JSON.stringify(manifest));
  }
  const profileHosts = path.join(profile, 'NativeMessagingHosts');
  fs.mkdirSync(profileHosts, { recursive: true });
  fs.writeFileSync(path.join(profileHosts, 'org.xdm.browser.json'), JSON.stringify(manifest));
  const worker = await new Cdp(workerTarget.webSocketDebuggerUrl).open();
  const hello = await worker.eval(`chrome.runtime.sendNativeMessage('org.xdm.browser', {version:1,type:'hello'})`);
  assert.equal(hello.type, 'hello.result', JSON.stringify(hello));
  await worker.eval('chrome.storage.local.set({interceptDownloads:true})');
  assert.equal((await worker.eval('chrome.storage.local.get({interceptDownloads:false})')).interceptDownloads, true);
  const pageTarget = (await targets(devtoolsPort)).find(item => item.type === 'page');
  const page = await new Cdp(pageTarget.webSocketDebuggerUrl).open();
  await page.call('Page.navigate', { url: `http://127.0.0.1:${httpPort}/browser.bin` });
  const output = path.join(home, 'Downloads', 'browser.bin');
  try {
    await until(() => fs.existsSync(output) && fs.statSync(output).size === 8 * 1024 * 1024, 20000);
  } catch (error) {
    console.error('Browser downloads:', JSON.stringify(await worker.eval('chrome.downloads.search({})')));
    console.error('Extension state:', JSON.stringify(await worker.eval('(async()=>({setting:await chrome.storage.local.get({interceptDownloads:false}), recent:[...recent.entries()] }))()')));
    console.error('Daemon tasks:', fs.existsSync(path.join(env.XDM_DATA_DIR, 'tasks.sqlite3')) ? 'database exists' : 'no database');
    console.error('Chrome log:', chromeError.slice(-3000));
    console.error('Fixture requests:', fs.existsSync(path.join(temp, 'http/requests.jsonl')) ? fs.readFileSync(path.join(temp, 'http/requests.jsonl'), 'utf8').slice(-3000) : 'none');
    throw error;
  }
  const expected = Buffer.alloc(8 * 1024 * 1024);
  for (let index = 0; index < expected.length; index++) expected[index] = index % 256;
  const expectedHash = createHash('sha256').update(expected).digest('hex');
  assert.equal(createHash('sha256').update(fs.readFileSync(output)).digest('hex'), expectedHash);
  const downloads = await worker.eval('chrome.downloads.search({})');
  assert.ok(downloads.some(item => item.url.endsWith('/browser.bin') && item.state === 'interrupted'), JSON.stringify(downloads));
  daemon.kill('SIGTERM');
  await until(() => daemon.exitCode !== null || daemon.signalCode !== null);
  await page.call('Page.navigate', { url: `http://127.0.0.1:${httpPort}/fallback.bin` });
  const fallback = await until(async () => {
    const items = await worker.eval('chrome.downloads.search({})');
    return items.find(item => item.url.endsWith('/fallback.bin') && item.state === 'complete');
  }, 20000);
  assert.equal(createHash('sha256').update(fs.readFileSync(fallback.filename)).digest('hex'), expectedHash);
  page.close(); worker.close();
  console.log('real Chrome for Testing accepted handoff and daemon-unavailable browser fallback passed');
} finally {
  for (const process of children.reverse()) if (process.exitCode === null && process.signalCode === null) process.kill('SIGTERM');
  await delay(500);
  if (process.env.XDM_KEEP_TEST_TEMP) console.error('Kept test files:', temp);
  else fs.rmSync(temp, { recursive: true, force: true });
}
