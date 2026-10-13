import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const source = fs.readFileSync(new URL('../../extensions/chromium/background.js', import.meta.url), 'utf8');

function harness(pause, nativeReply = async () => ({ accepted: true })) {
  const handlers = {};
  const calls = { native: 0, resume: 0, cancel: 0, suggest: 0, messages: [] };
  let timeout;
  const event = name => ({ addListener: fn => { handlers[name] = fn; } });
  const chrome = {
    webRequest: { onBeforeSendHeaders: event('request'), onBeforeRequest: event('before'), onHeadersReceived: event('headers') },
    tabs: { onRemoved: event('removed') },
    action: { setBadgeText() {} },
    runtime: { onInstalled: event('installed'), onMessage: event('message'), sendNativeMessage: async (_host, message) => { calls.native++; calls.messages.push(message); return nativeReply(message); } },
    contextMenus: { onClicked: event('menu'), create() {} },
    storage: { local: { get: async () => ({ interceptDownloads: true }) } },
    cookies: { getAll: async () => [] },
    downloads: {
      onDeterminingFilename: event('download'),
      pause,
      resume: async () => { calls.resume++; },
      cancel: async () => { calls.cancel++; },
    },
  };
  vm.runInNewContext(source, {
    chrome, URL, navigator: { userAgent: 'XDM-test' }, crypto: { randomUUID: () => 'test' },
    setTimeout: fn => { timeout = fn; return 1; }, clearTimeout: () => {},
    console, Date,
  });
  handlers.request({ url: 'http://example.test/file.bin', method: 'GET', tabId: 1, requestHeaders: [] });
  handlers.download({ url: 'http://example.test/file.bin', filename: 'file.bin', id: 7 }, () => { calls.suggest++; });
  return { calls, handlers, timeout: () => timeout() };
}

const tick = () => new Promise(resolve => setImmediate(resolve));

let releasePause;
const slow = harness(() => new Promise(resolve => { releasePause = resolve; }));
await tick();
await slow.timeout();
releasePause();
await tick();
assert.equal(slow.calls.native, 0, 'timed-out pause must not submit to native host');
assert.equal(slow.calls.cancel, 0, 'timed-out browser download must survive');
assert.equal(slow.calls.suggest, 1);
assert.ok(slow.calls.resume >= 1);

const accepted = harness(async () => {});
await tick();
await tick();
assert.equal(accepted.calls.native, 1);
assert.equal(accepted.calls.cancel, 1);
assert.equal(accepted.calls.resume, 0);
assert.equal(accepted.calls.suggest, 1);

const media = harness(async () => {}, async message =>
  message.type === 'media.inspect' ? { ok: true, tracks: [{ index: 0, kind: 'video', height: 360 }] } : { accepted: true, taskId: 9 });
let inspection;
assert.equal(media.handlers.message({ type: 'media.inspect', url: 'http://example.test/master.m3u8' }, {}, value => { inspection = value; }), true);
await tick();
assert.equal(inspection.tracks[0].height, 360);
let submission;
assert.equal(media.handlers.message({ type: 'media.submit', url: 'http://example.test/master.m3u8', videoIndex: 0, audioIndex: null }, {}, value => { submission = value; }), true);
await tick();
assert.equal(submission.taskId, 9);
assert.deepEqual(media.calls.messages.filter(message => message.type.startsWith('media.')).map(message => message.type), ['media.inspect', 'media.submit']);

const manifest = harness(async () => {});
await tick();
const nativeBefore = manifest.calls.native;
const canceledBefore = manifest.calls.cancel;
manifest.handlers.request({ url: 'http://example.test/master.m3u8', method: 'GET', tabId: 1, requestHeaders: [] });
manifest.handlers.download({ url: 'http://example.test/master.m3u8', filename: 'master.m3u8', id: 8 }, () => { manifest.calls.suggest++; });
await tick();
await tick();
assert.equal(manifest.calls.native, nativeBefore, 'a manifest must not be submitted as a plain file');
assert.equal(manifest.calls.cancel, canceledBefore);
assert.ok(manifest.calls.resume >= 1);

// Captured main/sub-frame POST form bodies replay through the daemon as POST.
{
  const post = harness(async () => {});
  await tick();
  post.handlers.request({ url: 'http://example.test/form.bin', method: 'POST', tabId: 1, requestHeaders: [] });
  post.handlers.before({ url: 'http://example.test/form.bin', method: 'POST', tabId: 1, requestBody: { formData: { token: ['abc'] } } });
  const nativeBeforePost = post.calls.native;
  const canceledBeforePost = post.calls.cancel;
  post.handlers.download({ url: 'http://example.test/form.bin', filename: 'form.bin', id: 9 }, () => {});
  await tick(); await tick();
  assert.equal(post.calls.native - nativeBeforePost, 1);
  const message = post.calls.messages.at(-1);
  assert.equal(message.type, 'download.submit');
  assert.equal(message.payload.method, 'POST');
  assert.equal(message.payload.body, 'token=abc');
  assert.deepEqual(JSON.parse(JSON.stringify(message.payload.requestContext.headers)),
    [{ name: 'User-Agent', value: 'XDM-test' }, { name: 'Content-Type', value: 'application/x-www-form-urlencoded' }]);
  assert.equal(post.calls.cancel - canceledBeforePost, 1);
}
{
  const form = harness(async () => {});
  await tick();
  form.handlers.request({ url: 'http://example.test/form.bin', method: 'POST', tabId: 1, requestHeaders: [] });
  form.handlers.before({ url: 'http://example.test/form.bin', method: 'POST', tabId: 1, requestBody: { formData: { file: [{ file: 'a.bin' }] } } });
  const nativeBeforeForm = form.calls.native;
  const canceledBeforeForm = form.calls.cancel;
  form.handlers.download({ url: 'http://example.test/form.bin', filename: 'form.bin', id: 10 }, () => {});
  await tick(); await tick();
  assert.equal(form.calls.native, nativeBeforeForm, 'file-upload bodies must not be replayed');
  assert.equal(form.calls.cancel, canceledBeforeForm, 'the browser must keep an unreplayable download');
}
{
  const raw = harness(async () => {});
  await tick();
  raw.handlers.request({ url: 'http://example.test/form.bin', method: 'POST', tabId: 1, requestHeaders: [] });
  raw.handlers.before({ url: 'http://example.test/form.bin', method: 'POST', tabId: 1, requestBody: { raw: [1] } });
  const nativeBeforeRaw = raw.calls.native;
  const canceledBeforeRaw = raw.calls.cancel;
  raw.handlers.download({ url: 'http://example.test/form.bin', filename: 'form.bin', id: 11 }, () => {});
  await tick(); await tick();
  assert.equal(raw.calls.native, nativeBeforeRaw, 'raw byte bodies must not be replayed');
  assert.equal(raw.calls.cancel, canceledBeforeRaw, 'the browser must keep the raw-body download');
}
console.log('extension handoff, media track RPC, manifest guard, POST form replay, and timeout fallback passed');
