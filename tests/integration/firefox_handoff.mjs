import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const dir = new URL('../../extensions/firefox/', import.meta.url);
const manifest = JSON.parse(fs.readFileSync(new URL('manifest.json', dir), 'utf8'));
assert.equal(manifest.browser_specific_settings.gecko.id, 'xdm@xdm.local');
assert.equal(manifest.manifest_version, 2);
const sources = manifest.background.scripts.map(name => fs.readFileSync(new URL(name, dir), 'utf8'));

function harness({ nativeReply = async () => ({ accepted: true }), pause = async () => {}, intercept = true, cookies = async () => [] } = {}) {
  const handlers = {};
  const calls = { native: 0, resume: 0, cancel: 0, erase: 0, messages: [], cookieQueries: [], badges: [], menus: [] };
  let timeout;
  const event = name => ({ addListener: fn => { handlers[name] = fn; } });
  const browser = {
    webRequest: { onBeforeSendHeaders: event('request'), onBeforeRequest: event('before'), onHeadersReceived: event('headers') },
    tabs: { onRemoved: event('removed'), onUpdated: event('updated') },
    browserAction: { setBadgeText: value => calls.badges.push(value) },
    runtime: { onInstalled: event('installed'), onMessage: event('message'), sendNativeMessage: async (_host, message) => { calls.native++; calls.messages.push(message); return nativeReply(message); } },
    contextMenus: { onClicked: event('menu'), create: item => calls.menus.push(item) },
    storage: { local: { get: async () => ({ interceptDownloads: intercept }), set: async () => {} } },
    cookies: { getAll: async query => { calls.cookieQueries.push(query); return cookies(query); } },
    downloads: {
      onCreated: event('download'), pause,
      resume: async () => { calls.resume++; },
      cancel: async () => { calls.cancel++; },
      erase: async () => { calls.erase++; },
    },
  };
  const context = { browser, URL, navigator: { userAgent: 'XDM-test-UA' }, crypto: { randomUUID: () => 'test' },
    setTimeout: fn => { timeout = fn; return 1; }, clearTimeout: () => {}, console, Date };
  vm.createContext(context);
  for (const source of sources) vm.runInContext(source, context);
  return { calls, handlers, timeout: () => timeout() };
}

const tick = () => new Promise(resolve => setImmediate(resolve));
const URL_BIN = 'http://example.test/file.bin';
const seen = (h, url = URL_BIN, extra = {}) => h.handlers.request({ url, method: 'GET', tabId: 1, requestHeaders: [{ name: 'Referer', value: 'http://example.test/page' }, { name: 'User-Agent', value: 'Page-UA' }], ...extra });
const created = (h, url = URL_BIN, extra = {}) => h.handlers.download({ url, filename: '/tmp/file.bin', id: 7, ...extra });

// 1. Interception: pause -> native handoff with cookies/Referer/UA -> cancel + erase.
{
  const h = harness({ cookies: async () => [{ name: 'a', value: '1' }, { name: 'b', value: '2' }] });
  seen(h, URL_BIN, { cookieStoreId: 'firefox-container-3' });
  created(h);
  await tick(); await tick(); await tick();
  assert.equal(h.calls.native, 1);
  const message = h.calls.messages[0];
  assert.equal(message.type, 'download.submit');
  assert.equal(message.payload.filenameHint, 'file.bin');
  assert.equal(message.payload.requestContext.referrer, 'http://example.test/page');
  assert.deepEqual(JSON.parse(JSON.stringify(message.payload.requestContext.headers)), [{ name: 'User-Agent', value: 'Page-UA' }]);
  assert.deepEqual(JSON.parse(JSON.stringify(message.payload.requestContext.cookies)), [{ url: URL_BIN, header: 'a=1; b=2' }]);
  assert.equal(h.calls.cookieQueries[0].storeId, 'firefox-container-3', 'container cookie store must be queried');
  assert.equal(h.calls.cookieQueries[0].firstPartyDomain, null, 'first-party isolation must not hide cookies');
  assert.equal(h.calls.cancel, 1);
  assert.equal(h.calls.erase, 1);
  assert.equal(h.calls.resume, 0);
}

// 1b. A top-level navigation download has no Referer in webRequest and no originUrl; Firefox's DownloadItem.referrer supplies it.
{
  const h = harness();
  h.handlers.request({ url: URL_BIN, method: 'GET', tabId: 1, requestHeaders: [{ name: 'User-Agent', value: 'Page-UA' }] });
  created(h, URL_BIN, { referrer: 'http://example.test/page' });
  await tick(); await tick(); await tick();
  assert.equal(h.calls.messages[0].payload.requestContext.referrer, 'http://example.test/page');
}

// 2. Opt-in off, non-GET, private windows, extension-initiated: browser keeps the download.
for (const [name, h, run] of [
  ['opt-out', harness({ intercept: false }), h => { seen(h); created(h); }],
  ['POST', harness(), h => { seen(h, URL_BIN, { method: 'POST' }); created(h); }],
  ['private download', harness(), h => { seen(h); created(h, URL_BIN, { incognito: true }); }],
  ['private request', harness(), h => { seen(h, URL_BIN, { incognito: true }); created(h); }],
  ['extension download', harness(), h => { seen(h); created(h, URL_BIN, { byExtensionId: 'other@x' }); }],
  ['manifest', harness(), h => { seen(h, 'http://example.test/a.m3u8'); created(h, 'http://example.test/a.m3u8'); }],
]) {
  run(h);
  await tick(); await tick(); await tick();
  assert.equal(h.calls.cancel, 0, `${name}: download must survive`);
  assert.equal(h.calls.native, 0, `${name}: no handoff`);
}

// 3. Daemon unavailable (reject) and declined reply: download resumes, never cancelled.
for (const nativeReply of [async () => { throw new Error('Attempt to postMessage on disconnected port'); }, async () => ({ accepted: false })]) {
  const h = harness({ nativeReply });
  seen(h); created(h);
  await tick(); await tick(); await tick();
  assert.equal(h.calls.native, 1);
  assert.equal(h.calls.cancel, 0);
  assert.equal(h.calls.erase, 0);
  assert.ok(h.calls.resume >= 1, 'download must resume when handoff fails');
}

// 4. Slow pause that times out: no submission, download resumes.
{
  let release;
  const h = harness({ pause: () => new Promise(resolve => { release = resolve; }) });
  seen(h); created(h);
  await tick();
  await h.timeout();
  release();
  await tick(); await tick();
  assert.equal(h.calls.native, 0);
  assert.equal(h.calls.cancel, 0);
  assert.ok(h.calls.resume >= 1);
}

// 4b. Captured main/sub-frame POST form bodies replay through the daemon as POST.
{
  const h = harness();
  h.handlers.request({ url: 'http://example.test/post.bin', method: 'POST', tabId: 1, requestHeaders: [{ name: 'User-Agent', value: 'Page-UA' }] });
  h.handlers.before({ url: 'http://example.test/post.bin', method: 'POST', tabId: 1, requestBody: { formData: { token: ['abc'] } } });
  created(h, 'http://example.test/post.bin');
  await tick(); await tick(); await tick();
  assert.equal(h.calls.native, 1);
  const message = h.calls.messages[0];
  assert.equal(message.type, 'download.submit');
  assert.equal(message.payload.method, 'POST');
  assert.equal(message.payload.body, 'token=abc');
  assert.deepEqual(JSON.parse(JSON.stringify(message.payload.requestContext.headers)),
    [{ name: 'User-Agent', value: 'Page-UA' }, { name: 'Content-Type', value: 'application/x-www-form-urlencoded' }]);
  assert.equal(h.calls.cancel, 1);
  assert.equal(h.calls.erase, 1);
}
// Raw byte and file-upload bodies cannot be replayed: the browser keeps the download.
for (const [name, requestBody] of [
  ['raw body', { raw: [1] }],
  ['file upload', { formData: { file: [{ file: 'a.bin' }] } }],
]) {
  const h = harness();
  h.handlers.request({ url: 'http://example.test/post.bin', method: 'POST', tabId: 1, requestHeaders: [] });
  h.handlers.before({ url: 'http://example.test/post.bin', method: 'POST', tabId: 1, requestBody });
  created(h, 'http://example.test/post.bin');
  await tick(); await tick(); await tick();
  assert.equal(h.calls.native, 0, `${name}: no handoff`);
  assert.equal(h.calls.cancel, 0, `${name}: the browser keeps the download`);
}

// 5. Context menu handoff (link in a container tab), manifest URL goes to popup instead.
{
  const h = harness();
  h.handlers.installed();
  assert.deepEqual(h.calls.menus.map(menu => menu.id), ['xdm-link', 'xdm-media', 'xdm-image', 'xdm-page', 'xdm-downloads', 'xdm-selection']);
  h.handlers.menu({ menuItemId: 'xdm-link', linkUrl: 'http://example.test/x.zip', cookieStoreId: 'firefox-container-1' }, { id: 2, url: 'http://example.test/page' });
  await tick(); await tick();
  assert.equal(h.calls.native, 1);
  assert.equal(h.calls.messages[0].payload.source.pageUrl, 'http://example.test/page');
  assert.equal(h.calls.messages[0].payload.requestContext.referrer, 'http://example.test/page');
  assert.equal(h.calls.messages[0].payload.requestContext.headers[0].value, 'XDM-test-UA');
  assert.equal(h.calls.cookieQueries[0].storeId, 'firefox-container-1');
  h.handlers.menu({ menuItemId: 'xdm-media', srcUrl: 'http://example.test/m.m3u8' }, { id: 2, url: 'http://example.test/page' });
  await tick();
  assert.equal(h.calls.native, 1, 'HLS manifest must not be sent as a plain file');
  const list = await h.handlers.message({ type: 'media.list', tabId: 2 }, {});
  assert.equal(list.items[0].kind, 'hls');
  h.handlers.menu({ menuItemId: 'xdm-link', linkUrl: 'file:///etc/passwd' }, { id: 2 });
  await tick();
  assert.equal(h.calls.native, 1);
}

// 6. Media detection via response headers, page content script, popup RPCs, tab cleanup.
{
  const h = harness({ nativeReply: async message =>
    message.type === 'media.inspect' ? { ok: true, tracks: [{ index: 0, kind: 'video', height: 360 }] } : { accepted: true, taskId: 9 } });
  h.handlers.headers({ tabId: 5, url: 'http://example.test/stream', responseHeaders: [{ name: 'Content-Type', value: 'video/mp4' }] });
  h.handlers.headers({ tabId: 5, url: 'http://example.test/page.html', responseHeaders: [{ name: 'Content-Type', value: 'text/html' }] });
  h.handlers.headers({ tabId: 5, url: 'http://example.test/master.m3u8', responseHeaders: [{ name: 'Content-Type', value: 'application/vnd.apple.mpegurl' }] });
  h.handlers.headers({ tabId: -1, url: 'http://example.test/bg.mp4', responseHeaders: [] });
  assert.equal((await h.handlers.message({ type: 'media.list', tabId: 5 }, {})).items.map(item => item.kind).join(), 'direct,hls');
  assert.equal(h.calls.badges.at(-1).text, '2');
  assert.equal((await h.handlers.message({ type: 'page.media', url: 'http://example.test/v.webm', mime: 'video/*' }, { tab: { id: 5, url: 'http://example.test/p' } })).ok, true);
  assert.equal((await h.handlers.message({ type: 'media.list', tabId: 5 }, {})).items.length, 3);
  assert.equal((await h.handlers.message({ type: 'media.inspect', url: 'http://example.test/master.m3u8' }, {})).tracks[0].height, 360);
  assert.equal((await h.handlers.message({ type: 'media.submit', url: 'http://example.test/master.m3u8', videoIndex: 0, audioIndex: null }, {})).taskId, 9);
  assert.equal((await h.handlers.message({ type: 'media.submit', url: 'http://example.test/master.m3u8', videoIndex: -1 }, {})).accepted, false);
  assert.equal((await h.handlers.message({ type: 'media.download', url: 'http://example.test/stream', pageUrl: 'http://example.test/p' }, {})).accepted, true);
  h.handlers.updated(5, { status: 'loading', url: 'http://example.test/other' });
  assert.equal((await h.handlers.message({ type: 'media.list', tabId: 5 }, {})).items.length, 0);
}

// 7. Popup/media handoff with daemon down reports failure instead of throwing.
{
  const h = harness({ nativeReply: async () => { throw new Error('host not found'); } });
  const reply = await h.handlers.message({ type: 'media.download', url: 'http://example.test/a.mp4' }, {});
  assert.equal(reply.accepted, false);
  assert.match(reply.reason, /host not found/);
  assert.equal((await h.handlers.message({ type: 'media.inspect', url: 'http://example.test/a.m3u8' }, {})).ok, false);
}

console.log('firefox handoff, context menu, media detection, cookie context, POST form replay, and daemon-down fallback passed');
