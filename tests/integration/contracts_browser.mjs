import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const chromiumDir = new URL('../../extensions/chromium/', import.meta.url);
const firefoxDir = new URL('../../extensions/firefox/', import.meta.url);

const chromiumManifest = JSON.parse(fs.readFileSync(new URL('manifest.json', chromiumDir), 'utf8'));
const firefoxManifest = JSON.parse(fs.readFileSync(new URL('manifest.json', firefoxDir), 'utf8'));

const chromiumBgSource = fs.readFileSync(new URL('background.js', chromiumDir), 'utf8');
const chromiumContentSource = fs.readFileSync(new URL('content.js', chromiumDir), 'utf8');

const firefoxCoreSource = fs.readFileSync(new URL('core.js', firefoxDir), 'utf8');
const firefoxBgSource = fs.readFileSync(new URL('background.js', firefoxDir), 'utf8');
const firefoxContentSource = fs.readFileSync(new URL('content.js', firefoxDir), 'utf8');

const tick = () => new Promise(resolve => setImmediate(resolve));

function makeChromiumHarness({
  nativeReply = async () => ({ accepted: true }),
  pause = async () => {},
  intercept = true,
  cookies = async () => [],
  platform = 'Linux x86_64',
  userAgent = 'Mozilla/5.0 (X11; Linux x86_64) XDM-Test',
} = {}) {
  const handlers = {};
  const calls = { native: 0, nativeHosts: [], resume: 0, cancel: 0, suggest: 0, messages: [], badges: [], menus: [], storageSets: [] };
  let timeout;
  const event = name => ({ addListener: fn => { handlers[name] = fn; } });
  const chrome = {
    webRequest: {
      onBeforeSendHeaders: event('request'),
      onBeforeRequest: event('before'),
      onHeadersReceived: event('headers'),
    },
    tabs: { onRemoved: event('removed') },
    action: {
      setBadgeText: value => { calls.badges.push(value); },
      openPopup: async () => {},
    },
    runtime: {
      onInstalled: event('installed'),
      onMessage: event('message'),
      sendNativeMessage: async (host, message) => {
        calls.native++;
        calls.nativeHosts.push(host);
        calls.messages.push(message);
        return nativeReply(message);
      },
    },
    contextMenus: {
      onClicked: event('menu'),
      create: item => { calls.menus.push(item); },
    },
    storage: {
      local: {
        get: async defaults => ({ ...defaults, interceptDownloads: intercept }),
        set: async value => { calls.storageSets.push(value); },
      },
    },
    cookies: { getAll: async query => cookies(query) },
    downloads: {
      onDeterminingFilename: event('download'),
      pause,
      resume: async () => { calls.resume++; },
      cancel: async () => { calls.cancel++; },
    },
  };

  const context = {
    chrome,
    URL,
    navigator: { userAgent, platform },
    crypto: { randomUUID: () => 'test-uuid' },
    setTimeout: fn => { timeout = fn; return 1; },
    clearTimeout: () => {},
    console: { ...console, warn: () => {} },
    Date,
    Map,
    Set,
    Number,
    String,
    Object,
    Array,
    encodeURIComponent,
  };

  vm.createContext(context);
  vm.runInContext(chromiumBgSource, context);
  return { calls, handlers, chrome, context, triggerTimeout: () => timeout?.() };
}

function makeFirefoxHarness({
  nativeReply = async () => ({ accepted: true }),
  pause = async () => {},
  intercept = true,
  cookies = async () => [],
  platform = 'Linux x86_64',
  userAgent = 'Mozilla/5.0 (X11; Linux x86_64) XDM-Test-Firefox',
} = {}) {
  const handlers = {};
  const calls = { native: 0, nativeHosts: [], resume: 0, cancel: 0, erase: 0, messages: [], cookieQueries: [], badges: [], menus: [], storageSets: [] };
  let timeout;
  const event = name => ({ addListener: fn => { handlers[name] = fn; } });
  const browser = {
    webRequest: {
      onBeforeSendHeaders: event('request'),
      onBeforeRequest: event('before'),
      onHeadersReceived: event('headers'),
    },
    tabs: {
      onRemoved: event('removed'),
      onUpdated: event('updated'),
    },
    browserAction: {
      setBadgeText: value => { calls.badges.push(value); },
      openPopup: async () => {},
    },
    runtime: {
      onInstalled: event('installed'),
      onMessage: event('message'),
      sendNativeMessage: async (host, message) => {
        calls.native++;
        calls.nativeHosts.push(host);
        calls.messages.push(message);
        return nativeReply(message);
      },
    },
    contextMenus: {
      onClicked: event('menu'),
      create: item => { calls.menus.push(item); },
    },
    storage: {
      local: {
        get: async defaults => ({ ...defaults, interceptDownloads: intercept }),
        set: async value => { calls.storageSets.push(value); },
      },
    },
    cookies: {
      getAll: async query => {
        calls.cookieQueries.push(query);
        return cookies(query);
      },
    },
    downloads: {
      onCreated: event('download'),
      pause,
      resume: async () => { calls.resume++; },
      cancel: async () => { calls.cancel++; },
      erase: async () => { calls.erase++; },
    },
  };

  const context = {
    browser,
    URL,
    navigator: { userAgent, platform },
    crypto: { randomUUID: () => 'test-uuid' },
    setTimeout: fn => { timeout = fn; return 1; },
    clearTimeout: () => {},
    console: { ...console, warn: () => {} },
    Date,
    Map,
    Set,
    Number,
    String,
    Object,
    Array,
    encodeURIComponent,
  };

  vm.createContext(context);
  vm.runInContext(firefoxCoreSource, context);
  vm.runInContext(firefoxBgSource, context);
  return { calls, handlers, browser, context, triggerTimeout: () => timeout?.() };
}

function makeContentHarness(source, isFirefox = false) {
  const sentMessages = [];
  const observedNodes = [];

  class MockElement {
    constructor(tagName, attributes = {}) {
      this.tagName = tagName.toUpperCase();
      this.attributes = { ...attributes };
      this.src = attributes.src || '';
      this.currentSrc = attributes.currentSrc || this.src;
      this.title = attributes.title || '';
      this.children = [];
      this.parentNode = null;
      this.style = {};
      this.isConnected = true;
    }
    matches(sel) {
      const tags = sel.split(',').map(s => s.trim().toUpperCase());
      return tags.includes(this.tagName);
    }
    querySelectorAll(sel) {
      const results = [];
      const match = sel.toUpperCase();
      const traverse = node => {
        for (const child of node.children) {
          if (child.tagName === match || (sel.includes(',') && sel.split(',').map(s => s.trim().toUpperCase()).includes(child.tagName))) {
            results.push(child);
          }
          traverse(child);
        }
      };
      traverse(this);
      return results;
    }
    querySelector(sel) {
      return this.querySelectorAll(sel)[0] || null;
    }
    appendChild(child) {
      child.parentNode = this;
      this.children.push(child);
      return child;
    }
    removeChild(child) {
      const idx = this.children.indexOf(child);
      if (idx !== -1) {
        this.children.splice(idx, 1);
        child.parentNode = null;
      }
      return child;
    }
    getAttribute(name) {
      return this.attributes?.[name] ?? null;
    }
    setAttribute(name, val) {
      this.attributes = this.attributes || {};
      this.attributes[name] = val;
    }
  }

  const docEl = new MockElement('HTML');

  class MockMutationObserver {
    constructor(callback) {
      this.callback = callback;
    }
    observe(target, options) {
      observedNodes.push({ target, options, observer: this });
    }
    triggerAdd(node) {
      this.callback([{ addedNodes: [node], removedNodes: [] }]);
    }
    triggerRemove(node) {
      this.callback([{ addedNodes: [], removedNodes: [node] }]);
    }
  }

  let timeoutCb = null;
  const api = {
    runtime: {
      sendMessage: async msg => {
        sentMessages.push(msg);
        return { ok: true };
      },
      onMessage: {
        addListener: () => {},
      },
    },
  };

  const winListeners = {};
  const docListeners = {};

  const context = {
    Element: MockElement,
    MutationObserver: MockMutationObserver,
    document: {
      documentElement: docEl,
      createElement: tag => new MockElement(tag),
      querySelectorAll: sel => docEl.querySelectorAll(sel),
      querySelector: sel => docEl.querySelectorAll(sel)[0] || null,
      contains: node => {
        let cur = node;
        while (cur) {
          if (cur === docEl) return true;
          cur = cur.parentNode;
        }
        return false;
      },
      addEventListener: (type, fn) => { docListeners[type] = fn; },
      removeEventListener: (type, fn) => { delete docListeners[type]; },
      getSelection: () => null,
    },
    window: {
      addEventListener: (type, fn) => { winListeners[type] = fn; },
      removeEventListener: (type, fn) => { delete winListeners[type]; },
      getSelection: () => null,
    },
    [isFirefox ? 'browser' : 'chrome']: api,
    Set,
    Map,
    WeakMap,
    WeakRef,
    URL,
    console,
    Date,
    setTimeout: (fn, ms) => { timeoutCb = fn; return 1; },
    clearTimeout: () => { timeoutCb = null; },
  };

  vm.createContext(context);
  vm.runInContext(source, context);
  const result = {
    docEl,
    sentMessages,
    observedNodes,
    MockElement,
    docListeners,
    winListeners,
    triggerTimeout: () => { if (timeoutCb) { const cb = timeoutCb; timeoutCb = null; cb(); } }
  };
  Object.setPrototypeOf(result, context);
  return result;
}

// Map of expected failure reasons for contracts representing known gaps.
const EXPECTED_FAIL = {
  B07: 'Shelf suppression requires real browser installation with supported download shelf API',
  B10: 'Proprietary #command44 registration and licensing schemes are excluded by policy',
  B49: 'Window geometry, DPI, and desktop focus tracking require real OS and browser window manager',
};

const tests = {};

// B01: Linux and native Windows each register host and complete handoff; extension initialization must not reject non-Windows platforms.
tests.B01 = async function test_B01() {
  const ch = makeChromiumHarness({ platform: 'Linux x86_64', userAgent: 'XDM-Linux' });
  const ff = makeFirefoxHarness({ platform: 'Linux x86_64', userAgent: 'XDM-Linux' });
  assert.ok(ch.handlers.download, 'Chromium filename hook must initialize on Linux');
  assert.ok(ff.handlers.download, 'Firefox created hook must initialize on Linux');
  assert.ok(ch.handlers.request, 'Chromium webRequest must initialize on Linux');
  assert.ok(ff.handlers.request, 'Firefox webRequest must initialize on Linux');
};

// B02: Capture a media request before DOM readiness, reinjection, worker restart; content script runs at document_start with web-accessible page helper.
tests.B02 = async function test_B02() {
  assert.equal(chromiumManifest.content_scripts[0].run_at, 'document_start', 'content script must run at document_start');
  assert.ok(chromiumManifest.web_accessible_resources?.length > 0, 'manifest must declare web-accessible page helpers');
};

// B03: Persist settings across update; expose private-mode limitations and integration state without third-party tracking cookies.
tests.B03 = async function test_B03() {
  const ch = makeChromiumHarness();
  ch.handlers.installed();
  await tick();
  const stored = await ch.chrome.storage.local.get({});
  assert.ok('privateModeLimitation' in stored || 'siteRules' in stored, 'extension settings must expose private mode state or durable rules');
};

// B04: Host missing/outdated/daemon offline produces actionable diagnosis and usable browser fallback.
tests.B04 = async function test_B04() {
  const offlineError = new Error('host not found');
  const ch = makeChromiumHarness({ nativeReply: async () => { throw offlineError; } });
  ch.handlers.request({ url: 'http://example.test/file.bin', method: 'GET', tabId: 1, requestHeaders: [] });
  let suggested = false;
  ch.handlers.download({ url: 'http://example.test/file.bin', filename: 'file.bin', id: 10 }, () => { suggested = true; });
  await tick(); await tick();
  assert.equal(ch.calls.cancel, 0, 'download must not be canceled when daemon offline');
  assert.ok(ch.calls.resume >= 1, 'download must resume when daemon offline');
  assert.equal(suggested, true, 'suggest callback must be released');

  let rpcReply;
  ch.handlers.message({ type: 'media.download', url: 'http://example.test/a.mp4' }, {}, res => { rpcReply = res; });
  await tick();
  assert.equal(rpcReply.accepted, false);
  assert.match(rpcReply.reason, /host not found/);

  const ff = makeFirefoxHarness({ nativeReply: async () => { throw offlineError; } });
  ff.handlers.request({ url: 'http://example.test/file.bin', method: 'GET', tabId: 1, requestHeaders: [] });
  ff.handlers.download({ url: 'http://example.test/file.bin', filename: 'file.bin', id: 11 });
  await tick(); await tick(); await tick();
  assert.equal(ff.calls.cancel, 0);
  assert.ok(ff.calls.resume >= 1);
  const ffReply = await ff.handlers.message({ type: 'media.download', url: 'http://example.test/a.mp4' }, {});
  assert.equal(ffReply.accepted, false);
};

// B05: Version mismatch and malformed reply reject without browser cancellation.
tests.B05 = async function test_B05() {
  for (const badReply of [{ accepted: false }, { malformed: true }]) {
    const ch = makeChromiumHarness({ nativeReply: async () => badReply });
    ch.handlers.request({ url: 'http://example.test/f.bin', method: 'GET', tabId: 1, requestHeaders: [] });
    ch.handlers.download({ url: 'http://example.test/f.bin', filename: 'f.bin', id: 20 }, () => {});
    await tick(); await tick();
    assert.equal(ch.calls.cancel, 0);
    assert.ok(ch.calls.resume >= 1);

    const ff = makeFirefoxHarness({ nativeReply: async () => badReply });
    ff.handlers.request({ url: 'http://example.test/f.bin', method: 'GET', tabId: 1, requestHeaders: [] });
    ff.handlers.download({ url: 'http://example.test/f.bin', filename: 'f.bin', id: 21 });
    await tick(); await tick(); await tick();
    assert.equal(ff.calls.cancel, 0);
    assert.ok(ff.calls.resume >= 1);
  }
};

// B06: Separate global and tab/site controls, durable rule settings and status feedback.
tests.B06 = async function test_B06() {
  const ch = makeChromiumHarness();
  let handled = false;
  ch.handlers.message({ type: 'tab.toggleIntegration', tabId: 1 }, {}, res => { handled = res?.ok; });
  await tick();
  assert.equal(handled, true, 'must support per-tab integration toggle');
};

// B07: Toggle has immediate defined effect; original shelf visibility restored after integration disabled.
tests.B07 = async function test_B07() {
  const ch = makeChromiumHarness();
  assert.ok(typeof ch.chrome.downloads?.setShelfEnabled === 'function', 'shelf suppression must be implemented');
};

// B08: Install/update/offline/private mode explanations; explicit consent where needed; no repeated intrusive windows.
tests.B08 = async function test_B08() {
  assert.ok(
    chromiumManifest.options_ui || chromiumManifest.background?.page || chromiumManifest.action?.default_popup?.includes('onboarding'),
    'must provide onboarding or repair dialog surface'
  );
};

// B09: Detect own duplicate installation or registration conflict and explain it; leave unrelated extensions alone.
tests.B09 = async function test_B09() {
  assert.ok(chromiumManifest.permissions.includes('management'), 'extension must declare management permission to check duplicate installs');
};

// B10: Implement only independent supported capture routes; document unsupported proprietary schemes.
tests.B10 = async function test_B10() {
  const ch = makeChromiumHarness();
  let intercepted = false;
  ch.handlers.request({ url: 'http://example.test/#command44', method: 'GET', tabId: 1, requestHeaders: [] });
  await tick();
  assert.equal(intercepted, true, 'proprietary command44 registration URL must be handled');
};

// B11: One-shot/POST downloads remain browser-owned until a safe explicit design exists; never silently replay as GET.
tests.B11 = async function test_B11() {
  for (const harnessFn of [makeChromiumHarness, makeFirefoxHarness]) {
    const h = harnessFn();
    h.handlers.request({ url: 'http://example.test/upload.bin', method: 'POST', tabId: 1, requestHeaders: [] });
    h.handlers.before({ url: 'http://example.test/upload.bin', method: 'POST', tabId: 1, requestBody: { formData: { f: [{ file: 'doc.pdf' }] } } });
    if (h.handlers.download.length === 2) h.handlers.download({ url: 'http://example.test/upload.bin', filename: 'upload.bin', id: 30 }, () => {});
    else h.handlers.download({ url: 'http://example.test/upload.bin', filename: 'upload.bin', id: 30 });
    await tick(); await tick(); await tick();
    assert.equal(h.calls.native, 0, 'file uploads must not be submitted to native host');
    assert.equal(h.calls.cancel, 0, 'browser must keep file upload download');

    h.handlers.request({ url: 'http://example.test/raw.bin', method: 'POST', tabId: 1, requestHeaders: [] });
    h.handlers.before({ url: 'http://example.test/raw.bin', method: 'POST', tabId: 1, requestBody: { raw: [1, 2, 3] } });
    if (h.handlers.download.length === 2) h.handlers.download({ url: 'http://example.test/raw.bin', filename: 'raw.bin', id: 31 }, () => {});
    else h.handlers.download({ url: 'http://example.test/raw.bin', filename: 'raw.bin', id: 31 });
    await tick(); await tick(); await tick();
    assert.equal(h.calls.native, 0, 'raw byte requests must not be submitted');
    assert.equal(h.calls.cancel, 0, 'browser must keep raw body download');
  }
};

// B12: Concurrent same URL in different tabs/auth contexts and redirects retain request-ID-bound context; Authorization fixture passes.
tests.B12 = async function test_B12() {
  for (const harnessFn of [makeChromiumHarness, makeFirefoxHarness]) {
    const h = harnessFn();
    h.handlers.request({
      url: 'http://example.test/auth.bin', method: 'GET', tabId: 1,
      requestHeaders: [
        { name: 'Referer', value: 'http://example.test/source' },
        { name: 'User-Agent', value: 'Custom-Agent' },
        { name: 'Authorization', value: 'Bearer SUPER_SECRET' },
      ],
    });
    if (h.handlers.download.length === 2) h.handlers.download({ url: 'http://example.test/auth.bin', filename: 'auth.bin', id: 40 }, () => {});
    else h.handlers.download({ url: 'http://example.test/auth.bin', filename: 'auth.bin', id: 40 });
    await tick(); await tick(); await tick();
    assert.equal(h.calls.native, 1);
    const msg = h.calls.messages[0];
    const headerNames = (msg.payload.requestContext.headers || []).map(hdr => hdr.name.toLowerCase());
    assert.ok(headerNames.includes('user-agent'));
    assert.ok(!headerNames.includes('authorization'), 'Authorization header must be omitted from context');
    assert.equal(msg.payload.requestContext.referrer, 'http://example.test/source');
  }
};

// B13: Opaque signed URL with MIME/attachment and UTF-8 filename; failed and HTML responses not false downloads.
tests.B13 = async function test_B13() {
  for (const harnessFn of [makeChromiumHarness, makeFirefoxHarness]) {
    const h = harnessFn();
    const url = 'http://example.test/report?sig=123&exp=456';
    h.handlers.request({ url, method: 'GET', tabId: 1, requestHeaders: [] });
    h.handlers.headers({ tabId: 1, url, responseHeaders: [{ name: 'Content-Type', value: 'application/pdf' }] });
    if (h.handlers.download.length === 2) h.handlers.download({ url, filename: 'تقرير_2026.pdf', mime: 'application/pdf', id: 50 }, () => {});
    else h.handlers.download({ url, filename: 'تقرير_2026.pdf', mime: 'application/pdf', id: 50 });
    await tick(); await tick(); await tick();
    assert.equal(h.calls.native, 1);
    const msg = h.calls.messages[0];
    assert.equal(msg.payload.filenameHint, 'تقرير_2026.pdf');
    assert.equal(msg.payload.mime, 'application/pdf');
  }
};

// B14: Redirect method semantics, final URL, cookies and cross-origin header stripping verified together.
tests.B14 = async function test_B14() {
  const ch = makeChromiumHarness();
  assert.ok(ch.chrome.webRequest.onBeforeRedirect, 'webRequest.onBeforeRedirect must be hooked to track redirect chains');
};

// B15: Independent versioned site/type rules with priority, match preview, exemptions and force/bypass controls.
tests.B15 = async function test_B15() {
  const ch = makeChromiumHarness();
  const rules = await ch.chrome.storage.local.get({ siteExclusions: [] });
  assert.ok(Array.isArray(rules.siteExclusions) && rules.siteExclusions.length > 0, 'site exclusion rules must be configured');
};

// B16: Concurrent duplicate URLs, redirects and missing request records do not steal wrong download.
tests.B16 = async function test_B16() {
  const ch = makeChromiumHarness();
  const url = 'http://example.test/same.bin';
  ch.handlers.request({ url, method: 'GET', tabId: 1, requestHeaders: [{ name: 'Referer', value: 'http://example.test/tab1' }] });
  ch.handlers.request({ url, method: 'GET', tabId: 2, requestHeaders: [{ name: 'Referer', value: 'http://example.test/tab2' }] });
  ch.handlers.download({ url, filename: 'same.bin', id: 60, tabId: 1 }, () => {});
  await tick(); await tick();
  assert.equal(ch.calls.messages[0]?.payload.requestContext.referrer, 'http://example.test/tab1', 'tab 1 must retain tab 1 referrer');
};

// B17: Single callback release, accepted cancellation, rejection/offline resumption, Unicode filename and restart idempotency.
tests.B17 = async function test_B17() {
  let releaseCalls = 0;
  const ch = makeChromiumHarness();
  ch.handlers.request({ url: 'http://example.test/u.bin', method: 'GET', tabId: 1, requestHeaders: [] });
  ch.handlers.download({ url: 'http://example.test/u.bin', filename: 'ملف.dat', id: 70 }, () => { releaseCalls++; });
  await tick(); await tick();
  assert.equal(releaseCalls, 1, 'suggest callback must be released exactly once');
  assert.equal(ch.calls.cancel, 1);
  assert.equal(ch.calls.messages[0]?.payload.filenameHint, 'ملف.dat');
};

// B18: Ensure canceled browser and durable daemon task agree; cancel API failure observable/recoverable.
tests.B18 = async function test_B18() {
  for (const harnessFn of [makeChromiumHarness, makeFirefoxHarness]) {
    const h = harnessFn({ nativeReply: async () => ({ accepted: true, taskId: 42 }) });
    h.handlers.request({ url: 'http://example.test/b.bin', method: 'GET', tabId: 1, requestHeaders: [] });
    if (h.handlers.download.length === 2) h.handlers.download({ url: 'http://example.test/b.bin', filename: 'b.bin', id: 80 }, () => {});
    else h.handlers.download({ url: 'http://example.test/b.bin', filename: 'b.bin', id: 80 });
    await tick(); await tick(); await tick();
    assert.equal(h.calls.cancel, 1, 'accepted handoff must cancel browser download');
    assert.equal(h.calls.resume, 0);
  }
};

// B19: Delayed host acceptance after timeout, worker termination, pause timeout, cancel rejection and disconnect each produce bounded recovery without hidden duplicate claims.
tests.B19 = async function test_B19() {
  let releasePause = () => {};
  const ch = makeChromiumHarness({ pause: () => new Promise(res => { releasePause = res; }) });
  ch.handlers.request({ url: 'http://example.test/slow.bin', method: 'GET', tabId: 1, requestHeaders: [] });
  ch.handlers.download({ url: 'http://example.test/slow.bin', filename: 'slow.bin', id: 90 }, () => {});
  await tick();
  ch.triggerTimeout();
  releasePause();
  await tick(); await tick();
  assert.equal(ch.calls.native, 0, 'timed-out pause must not submit download to daemon');
  assert.equal(ch.calls.cancel, 0, 'timed-out download must not be cancelled');
  assert.ok(ch.calls.resume >= 1, 'timed-out download must be resumed');
};

// B20: Define TTL, clear stale records, preserve in-flight correlation, worker restart behavior.
tests.B20 = async function test_B20() {
  const ch = makeChromiumHarness();
  ch.handlers.request({
    url: 'http://example.test/old.bin',
    method: 'GET',
    tabId: 1,
    requestHeaders: [{ name: 'Referer', value: 'http://example.test/old-page' }],
  });
  // Simulate download occurring after a long duration (e.g. 1 hour later)
  const origNow = Date.now;
  try {
    Date.now = () => origNow() + 3600 * 1000;
    ch.handlers.download({ url: 'http://example.test/old.bin', filename: 'old.bin', id: 200 }, () => {});
    await tick(); await tick();
    // Under contract B20, stale correlation (>60s) must be expired, so referrer from 1 hour ago must NOT be retained
    assert.equal(ch.calls.messages[0]?.payload.requestContext.referrer, undefined, 'stale request correlation must expire via TTL');
  } finally {
    Date.now = origNow;
  }
};

// B21: Container/private/partitioned-cookie fixtures; cookie matching obeys origin and store.
tests.B21 = async function test_B21() {
  const ff = makeFirefoxHarness({ cookies: async () => [{ name: 'sess', value: 'c2' }] });
  ff.handlers.request({ url: 'http://example.test/item.bin', method: 'GET', tabId: 1, cookieStoreId: 'container-2', requestHeaders: [] });
  ff.handlers.download({ url: 'http://example.test/item.bin', filename: 'item.bin', id: 100 });
  await tick(); await tick(); await tick();
  assert.equal(ff.calls.cookieQueries[0]?.storeId, 'container-2', 'must query cookies for container-2 storeId');
  assert.equal(ff.calls.cookieQueries[0]?.firstPartyDomain, null, 'must specify firstPartyDomain: null');

  // Private downloads must not be intercepted
  const ffPrivate = makeFirefoxHarness();
  ffPrivate.handlers.request({ url: 'http://example.test/priv.bin', method: 'GET', tabId: 2, incognito: true, requestHeaders: [] });
  ffPrivate.handlers.download({ url: 'http://example.test/priv.bin', filename: 'priv.bin', id: 101, incognito: true });
  await tick(); await tick(); await tick();
  assert.equal(ffPrivate.calls.native, 0, 'private downloads must remain in browser');
};

// B22: Raw binary and repeated form fields need byte-exact fixtures before supporting replay. Header/body size limits and redaction enforced.
tests.B22 = async function test_B22() {
  const ch = makeChromiumHarness();
  ch.handlers.request({ url: 'http://example.test/form.bin', method: 'POST', tabId: 1, requestHeaders: [] });
  ch.handlers.before({ url: 'http://example.test/form.bin', method: 'POST', tabId: 1, requestBody: { formData: { field: ['v1', 'v2'] } } });
  ch.handlers.download({ url: 'http://example.test/form.bin', filename: 'form.bin', id: 110 }, () => {});
  await tick(); await tick();
  assert.equal(ch.calls.native, 1);
  assert.equal(ch.calls.messages[0]?.payload.body, 'field=v1&field=v2', 'repeated form fields must be urlencoded');

  // Oversized body > 1MB rejected
  const hugeBody = 'x'.repeat(1024 * 1024 + 1);
  const chHuge = makeChromiumHarness();
  chHuge.handlers.request({ url: 'http://example.test/huge.bin', method: 'POST', tabId: 1, requestHeaders: [] });
  chHuge.handlers.before({ url: 'http://example.test/huge.bin', method: 'POST', tabId: 1, requestBody: { formData: { data: [hugeBody] } } });
  chHuge.handlers.download({ url: 'http://example.test/huge.bin', filename: 'huge.bin', id: 111 }, () => {});
  await tick(); await tick();
  assert.equal(chHuge.calls.native, 0, 'oversize body must not be submitted');
};

// B23: Nested cross-origin/about:blank frames; referrer policy and user agent faithfully represented.
tests.B23 = async function test_B23() {
  assert.ok(chromiumManifest.permissions.includes('webNavigation'), 'must declare webNavigation to walk frame ancestry');
};

// B24: Browser VPN/proxy mismatch shown; explicit supported proxy configuration, credentials and no direct accidental fallback.
tests.B24 = async function test_B24() {
  assert.ok(chromiumManifest.permissions.includes('proxy'), 'extension must declare proxy permission to query proxy context');
};

// B25: Modifier combinations and key release/blur reset in browser; no stuck override.
tests.B25 = async function test_B25() {
  for (const [source, isFf] of [[chromiumContentSource, false], [firefoxContentSource, true]]) {
    const content = makeContentHarness(source, isFf);
    assert.ok(content.observedNodes.some(n => n.options?.subtree && n.options?.childList), 'content script must observe mutations');
    assert.equal(typeof content.keydownHandler, 'function', 'content script must attach keyboard listeners for force/bypass capture');
    assert.equal(typeof content.keyupHandler, 'function', 'content script must attach keyup listeners');
    assert.equal(typeof content.resetModifiers, 'function', 'content script must provide modifier reset handler');

    content.keydownHandler({ key: 'Alt', altKey: true });
    assert.equal(content.modifierState.alt, true, 'Alt keydown must set alt modifier');
    content.keydownHandler({ key: 'Control', ctrlKey: true, altKey: true });
    assert.equal(content.modifierState.alt, true);
    assert.equal(content.modifierState.ctrl, true, 'Ctrl keydown must set ctrl modifier');

    content.keyupHandler({ key: 'Alt', altKey: false, ctrlKey: true });
    assert.equal(content.modifierState.alt, false, 'Alt release must clear alt modifier');
    assert.equal(content.modifierState.ctrl, true);

    content.resetModifiers();
    assert.equal(content.modifierState.alt, false, 'blur reset must clear alt');
    assert.equal(content.modifierState.ctrl, false, 'blur reset must clear ctrl');
    assert.equal(content.modifierState.shift, false, 'blur reset must clear shift');
  }
};

// B26: Any future DNR route bounded to exact intended rules; disable removes rules and page history restores correctly.
tests.B26 = async function test_B26() {
  assert.ok(chromiumManifest.permissions.includes('declarativeNetRequest'), 'manifest must declare declarativeNetRequest for DNR routes');
};

// B27: Report unsupported schemes explicitly; blob maps to underlying observable source, never treated as remote file.
tests.B27 = async function test_B27() {
  for (const scheme of ['ftp://example.test/file.zip', 'file:///etc/passwd', 'blob:http://example.test/uuid', 'data:text/plain;base64,abc']) {
    const ch = makeChromiumHarness();
    ch.handlers.download({ url: scheme, filename: 'item', id: 120 }, () => {});
    await tick(); await tick();
    assert.equal(ch.calls.native, 0, `${scheme} must not be submitted`);
    assert.equal(ch.calls.cancel, 0, `${scheme} must remain in browser`);
  }
};

// B28: Each configured context submits intended HTTP resource with label/page/referrer.
tests.B28 = async function test_B28() {
  const ch = makeChromiumHarness();
  ch.handlers.installed();
  const menuContexts = ch.calls.menus.flatMap(m => m.contexts || []);
  assert.ok(menuContexts.includes('image'), 'must support image context menu');
  assert.ok(menuContexts.includes('page'), 'must support page context menu');
};

// B29: Download attribute vs unsafe path and Unicode label handled independently.
tests.B29 = async function test_B29() {
  const ch = makeChromiumHarness();
  ch.handlers.installed();
  let requestedDOM = false;
  ch.chrome.tabs.sendMessage = async (_tabId, msg) => {
    if (msg.type === 'page.getDownloadAttribute') requestedDOM = true;
    return {};
  };
  ch.handlers.menu({ menuItemId: 'xdm-link', linkUrl: 'http://example.test/link.bin' }, { id: 1 });
  await tick();
  assert.equal(requestedDOM, true, 'context menu must query DOM for download attribute');
};

// B30: Count/preview/filter/choose destination/atomic enqueue; selected-text vs all-page extraction fixtures.
tests.B30 = async function test_B30() {
  for (const [source, isFf] of [[chromiumContentSource, false], [firefoxContentSource, true]]) {
    const content = makeContentHarness(source, isFf);
    assert.equal(typeof content.extractLinks, 'function', 'content script must support link extraction');

    const a1 = new content.MockElement('a', { href: 'http://example.test/file1.zip' });
    const a2 = new content.MockElement('a', { href: 'http://example.test/file2.zip' });
    const aDup = new content.MockElement('a', { href: 'http://example.test/file1.zip' });
    const img = new content.MockElement('img', { src: 'http://example.test/pic.png', alt: 'Pic' });
    const js = new content.MockElement('a', { href: 'javascript:void(0)' });
    content.docEl.appendChild(a1);
    content.docEl.appendChild(a2);
    content.docEl.appendChild(aDup);
    content.docEl.appendChild(img);
    content.docEl.appendChild(js);

    const extracted = content.extractLinks();
    assert.equal(extracted.length, 3, 'duplicate URLs and invalid schemes must be filtered');
    assert.equal(extracted[0].url, 'http://example.test/file1.zip');
    assert.equal(extracted[1].url, 'http://example.test/file2.zip');
    assert.equal(extracted[2].url, 'http://example.test/pic.png');

    const selExtracted = content.extractLinks({ selectedText: 'Links http://example.test/a.pdf and https://example.test/b.tar', selectionOnly: true });
    assert.equal(selExtracted.length, 2);
    assert.equal(selExtracted[0].url, 'http://example.test/a.pdf');
    assert.equal(selExtracted[1].url, 'https://example.test/b.tar');
  }
};

// B31: Each accepted batch has stable request ID, original order, per-entry context and all-or-defined-partial acceptance.
tests.B31 = async function test_B31() {
  const ch = makeChromiumHarness();
  let batchReply;
  ch.handlers.message({ type: 'batch.submit', items: [] }, {}, res => { batchReply = res; });
  await tick();
  assert.ok(batchReply?.accepted, 'must implement batch.submit message handler');
};

// B32: Selection rect/DPI/frame ancestry, no-selection removal, matching link counts.
tests.B32 = async function test_B32() {
  for (const [source, isFf] of [[chromiumContentSource, false], [firefoxContentSource, true]]) {
    const content = makeContentHarness(source, isFf);
    assert.equal(typeof content.selectionHandler, 'function', 'content script must track selected text and coordinates');
    assert.equal(content.selectionHandler(), null, 'no-selection must yield null');

    const mockSel = {
      isCollapsed: false,
      rangeCount: 1,
      toString: () => 'Get http://example.test/sel.zip',
      getRangeAt: () => ({
        getBoundingClientRect: () => ({ x: 10, y: 20, width: 100, height: 15, top: 20, left: 10, bottom: 35, right: 110 }),
      }),
    };
    const sel = content.selectionHandler(mockSel);
    assert.ok(sel !== null, 'active selection must be detected');
    assert.equal(sel.linkCount, 1, 'matching link count must be calculated');
    assert.equal(sel.rect.top, 20, 'coordinates must match selection rect');

    const emptySel = { isCollapsed: true, rangeCount: 0, toString: () => '' };
    assert.equal(content.selectionHandler(emptySel), null, 'cleared selection must remove selection state');
  }
};

// B33: Actual browser permits intended page context; history removal opt-in and original output retained.
tests.B33 = async function test_B33() {
  const ch = makeChromiumHarness();
  ch.handlers.installed();
  assert.ok(ch.calls.menus.some(m => m.id === 'xdm-downloads'), 'must register downloads page menu integration');
};

// B34: Context action failure visible without opening developer tools; no false success.
tests.B34 = async function test_B34() {
  let notified = false;
  const ch = makeChromiumHarness({ nativeReply: async () => { throw new Error('daemon dead'); } });
  ch.handlers.installed();
  ch.chrome.notifications = { create: () => { notified = true; } };
  ch.handlers.menu({ menuItemId: 'xdm-link', linkUrl: 'http://example.test/link.bin' }, { id: 1, url: 'http://example.test/' });
  await tick(); await tick();
  assert.equal(notified, true, 'must show notification when context menu handoff fails');
};

// B35: Explicit supported profiles; nonmedia/404/ad and opaque MIME fixtures; no claim every classified type can be processed.
tests.B35 = async function test_B35() {
  const ch = makeChromiumHarness();
  // Ad tracking video URLs should be excluded
  ch.handlers.headers({ tabId: 1, url: 'http://example.test/ad_preroll.mp4?ad_id=123', responseHeaders: [{ name: 'Content-Type', value: 'video/mp4' }] });
  let listReply;
  ch.handlers.message({ type: 'media.list', tabId: 1 }, {}, res => { listReply = res; });
  await tick();
  assert.equal(listReply?.items.length, 0, 'ad tracking URLs must be excluded from media detection');
};

// B36: Separate detection/inspection/replay with stable identity and error feedback.
tests.B36 = async function test_B36() {
  for (const harnessFn of [makeChromiumHarness, makeFirefoxHarness]) {
    const h = harnessFn();
    const manifestUrl = 'http://example.test/stream.m3u8';
    h.handlers.request({ url: manifestUrl, method: 'GET', tabId: 1, requestHeaders: [] });
    if (h.handlers.download.length === 2) h.handlers.download({ url: manifestUrl, filename: 'stream.m3u8', id: 130 }, () => {});
    else h.handlers.download({ url: manifestUrl, filename: 'stream.m3u8', id: 130 });
    await tick(); await tick(); await tick();
    assert.equal(h.calls.native, 0, 'manifest must not be submitted as plain file download');
    assert.equal(h.calls.cancel, 0, 'manifest download must not be cancelled by plain downloader');
    assert.ok(h.calls.resume >= 1, 'manifest download must be resumed in browser');
  }
};

// B37: Cached/session-bound/one-shot manifests and non200/CORS responses; size/time limits and captured-body provenance.
tests.B37 = async function test_B37() {
  const ch = makeChromiumHarness();
  let manifestBodyReply;
  ch.handlers.message({ type: 'manifest.fetch', url: 'http://example.test/m.m3u8' }, {}, res => { manifestBodyReply = res; });
  await tick();
  assert.ok(manifestBodyReply?.body, 'extension must fetch manifest in page/browser context');
};

// B38: Avoid breaking page requests; marker removal where allowed; repeated requests, exceptions and replacement-hook coexistence.
tests.B38 = async function test_B38() {
  assert.ok(
    chromiumManifest.content_scripts.some(cs => cs.js.some(f => f.includes('xhr'))),
    'manifest must include page-context XHR hooks'
  );
};

// B39: Page observes unchanged promise/value/body semantics, single capture, large/binary/stream content bounds, failed and nonmatching responses untouched.
tests.B39 = async function test_B39() {
  assert.ok(
    chromiumManifest.content_scripts.some(cs => cs.js.some(f => f.includes('fetch'))),
    'manifest must include page-context fetch hook wrappers'
  );
};

// B40: Explicit hook capability result and restore; CSP/world isolation/page override tests.
tests.B40 = async function test_B40() {
  const ch = makeChromiumHarness();
  let hookReply;
  ch.handlers.message({ type: 'hook.status' }, {}, res => { hookReply = res; });
  await tick();
  assert.ok(hookReply?.installed, 'must report hook capabilities and status');
};

// B41: Define minimal declarative extraction scope and authentication; stale/detached elements cannot attach metadata to replacement player.
tests.B41 = async function test_B41() {
  for (const [source, isFf] of [[chromiumContentSource, false], [firefoxContentSource, true]]) {
    const content = makeContentHarness(source, isFf);
    assert.equal(typeof content.queryElementMetadata, 'function', 'content script must support element metadata queries');

    const v = new content.MockElement('video', { src: 'http://example.test/v.mp4', title: 'Video Title' });
    v.videoWidth = 1280;
    v.videoHeight = 720;
    content.docEl.appendChild(v);

    const meta = content.queryElementMetadata(v);
    assert.equal(meta.src, 'http://example.test/v.mp4');
    assert.equal(meta.title, 'Video Title');
    assert.equal(meta.videoWidth, 1280);

    const detached = new content.MockElement('video', { src: 'http://example.test/d.mp4' });
    detached.isConnected = false;
    assert.equal(content.queryElementMetadata(detached), null, 'detached element must not attach metadata');
  }
};

// B42: Independent adapters with per-site tests/versioning; player-quality changes and auth expiry reported; do not reuse proprietary decipher code.
tests.B42 = async function test_B42() {
  for (const [source, isFf] of [[chromiumContentSource, false], [firefoxContentSource, true]]) {
    const content = makeContentHarness(source, isFf);
    assert.ok(Array.isArray(content.siteAdapters), 'siteAdapters must be an array');
    assert.ok(content.siteAdapters.length >= 2, 'must provide multiple site adapters');
    for (const a of content.siteAdapters) {
      assert.ok(typeof a.id === 'string', 'adapter must have string ID');
      assert.ok(typeof a.version === 'number', 'adapter must declare version');
      assert.ok(typeof a.matches === 'function', 'adapter must have matches predicate');
      assert.ok(typeof a.extractMedia === 'function', 'adapter must have extractMedia function');
      assert.ok(typeof a.refreshUrl === 'function', 'adapter must have refreshUrl handler');
    }
    const adapter = content.getSiteAdapter('http://example.test/stream');
    assert.ok(adapter, 'matching adapter must be found');
    const refreshed = await adapter.refreshUrl({ url: 'http://example.test/stream.mp4' });
    assert.equal(refreshed.expired, false);
  }
};

// B43: Dynamic src/source/load events, multiple players, blob/MSE relation, detached elements, same URL reuse.
tests.B43 = async function test_B43() {
  for (const [source, isFf] of [[chromiumContentSource, false], [firefoxContentSource, true]]) {
    const c = makeContentHarness(source, isFf);
    const video = new c.MockElement('video', { src: 'http://example.test/direct.mp4' });
    c.docEl.appendChild(video);
    const dynVideo = new c.MockElement('video', { src: 'http://example.test/dyn.webm' });
    c.observedNodes[0]?.observer.triggerAdd(dynVideo);
    await tick();
    const urls = c.sentMessages.filter(m => m.type === 'page.media').map(m => m.url);
    assert.ok(urls.includes('http://example.test/dyn.webm'));
  }
};

// B44: Player replacement/removal cleans catalog and overlays without losing current source identity.
tests.B44 = async function test_B44() {
  for (const [source, isFf] of [[chromiumContentSource, false], [firefoxContentSource, true]]) {
    const content = makeContentHarness(source, isFf);
    assert.ok(typeof content.playerRegistry !== 'undefined', 'must track player elements via WeakRef / registry');

    const v = new content.MockElement('video', { src: 'http://example.test/reg.mp4', title: 'Reg Vid' });
    content.docEl.appendChild(v);
    const id = content.playerRegistry.register(v, { url: 'http://example.test/reg.mp4', title: 'Reg Vid' });
    assert.ok(typeof id === 'string');
    assert.equal(content.playerRegistry.size(), 1);
    assert.equal(content.playerRegistry.get(id).url, 'http://example.test/reg.mp4');

    content.playerRegistry.unregister(v);
    assert.equal(content.playerRegistry.size(), 0);
    assert.equal(content.playerRegistry.get(id), null);
  }
};

// B45: Scroll/zoom/DPI/clipping/occlusion/tiny audio widgets with explicit bounding-rect expectations.
tests.B45 = async function test_B45() {
  const content = makeContentHarness(chromiumContentSource);
  const tinyVideo = new content.MockElement('video', { src: 'http://example.test/1x1.mp4' });
  tinyVideo.getBoundingClientRect = () => ({ width: 1, height: 1 });
  content.observedNodes[0]?.observer.triggerAdd(tinyVideo);
  await tick();
  const tinySent = content.sentMessages.some(m => m.url === 'http://example.test/1x1.mp4');
  assert.equal(tinySent, false, '1x1 tracking pixel video must be suppressed');
};

// B46: Cross-origin iframe nesting and about:blank player coords; top-level fullscreen transform.
tests.B46 = async function test_B46() {
  assert.equal(chromiumManifest.content_scripts[0].all_frames, true, 'Chromium content script must declare all_frames');
  assert.equal(firefoxManifest.content_scripts[0].all_frames, true, 'Firefox content script must declare all_frames');
};

// B47: Bound idle CPU; cover attributes, removals, visibility changes and debounce without stale overlay.
tests.B47 = async function test_B47() {
  for (const [source, isFf] of [[chromiumContentSource, false], [firefoxContentSource, true]]) {
    const content = makeContentHarness(source, isFf);
    assert.ok(typeof content.debounceTimer !== 'undefined', 'DOM inspection must be debounced with 200ms timer');
    assert.equal(content.DEBOUNCE_MS, 200, 'debounce delay must be 200ms');

    const v1 = new content.MockElement('video', { src: 'http://example.test/v1.mp4' });
    content.observedNodes[0]?.observer.triggerAdd(v1);
    assert.ok(content.debounceTimer !== null, 'debounce timer must activate on mutation');
  }
};

// B48: Independent original-looking panel with defined desktop/DOM ownership, focus, hide/show and no intrusion on unrelated windows.
tests.B48 = async function test_B48() {
  for (const [source, isFf] of [[chromiumContentSource, false], [firefoxContentSource, true]]) {
    const content = makeContentHarness(source, isFf);
    assert.ok(typeof content.playerPanel !== 'undefined', 'content script must inject floating player panel');

    const panel = content.playerPanel.create('p1', { url: 'http://example.test/vid.mp4', title: 'Vid' });
    assert.ok(panel, 'player panel must be created');
    assert.equal(content.playerPanel.get('p1'), panel);

    content.playerPanel.hide('p1');
    assert.equal(panel.visible === false || panel.style?.display === 'none', true, 'panel must hide');
    content.playerPanel.show('p1');
    assert.equal(panel.visible === true || panel.style?.display === 'block', true, 'panel must show');

    content.playerPanel.remove('p1');
    assert.equal(content.playerPanel.get('p1'), null, 'panel must be cleaned up on remove');
  }
};

// B49: Multiple monitors, mixed DPI, fullscreen/exit, minimized window, tab detach and background tab hide.
tests.B49 = async function test_B49() {
  const ch = makeChromiumHarness();
  assert.ok(ch.chrome.windows?.onFocusChanged, 'must register window listeners for DPI/focus/geometry');
};

// B50: Navigation/SPA/history/tab-close cleanup, no stale media shown as current.
tests.B50 = async function test_B50() {
  const ch = makeChromiumHarness();
  ch.handlers.headers({ tabId: 5, url: 'http://example.test/video1.mp4', responseHeaders: [{ name: 'Content-Type', value: 'video/mp4' }] });
  let listBefore;
  ch.handlers.message({ type: 'media.list', tabId: 5 }, {}, res => { listBefore = res; });
  await tick();
  assert.equal(listBefore?.items.length, 1);

  // Tab navigates to new URL
  if (ch.handlers.updated) ch.handlers.updated(5, { status: 'loading', url: 'http://example.test/other' });
  let listAfter;
  ch.handlers.message({ type: 'media.list', tabId: 5 }, {}, res => { listAfter = res; });
  await tick();
  assert.equal(listAfter?.items.length, 0, 'navigation must clear stale media catalog');
};

// B51: Safe text title, source page, quality/format/audio labels, multiple similar videos distinguishable.
tests.B51 = async function test_B51() {
  const ch = makeChromiumHarness();
  ch.handlers.headers({ tabId: 1, url: 'http://example.test/video.mp4', responseHeaders: [{ name: 'Content-Type', value: 'video/mp4' }] });
  let listReply;
  ch.handlers.message({ type: 'media.list', tabId: 1 }, {}, res => { listReply = res; });
  await tick();
  assert.ok(listReply?.items[0]?.title && listReply.items[0].title.length > 0, 'media item must contain human-readable title');
};

// B52: List available manifest qualities and separately observed-only qualities; resolution choice cannot invent tracks.
tests.B52 = async function test_B52() {
  for (const harnessFn of [makeChromiumHarness, makeFirefoxHarness]) {
    const h = harnessFn({
      nativeReply: async msg => {
        if (msg.type === 'media.inspect') {
          return {
            ok: true,
            tracks: [
              { index: 0, kind: 'video', height: 720, bitrate: 2000000, codec: 'h264' },
              { index: 1, kind: 'video', height: 1080, bitrate: 4000000, codec: 'h264' },
            ],
          };
        }
        return { accepted: true };
      },
    });
    let inspectReply;
    if (h.handlers.message.length === 3) {
      h.handlers.message({ type: 'media.inspect', url: 'http://example.test/master.m3u8' }, {}, res => { inspectReply = res; });
      await tick();
    } else {
      inspectReply = await h.handlers.message({ type: 'media.inspect', url: 'http://example.test/master.m3u8' }, {});
    }
    assert.equal(inspectReply.ok, true);
    assert.equal(inspectReply.tracks.length, 2);
    assert.equal(inspectReply.tracks[1].height, 1080);
  }
};

// B53: Audio-only and language selection, pairing compatibility, FFprobe/decode output; explicit MP3 transcoding feature remains pending.
tests.B53 = async function test_B53() {
  const ch = makeChromiumHarness();
  let submitReply;
  ch.handlers.message({ type: 'media.submit', url: 'http://example.test/stream.m3u8', videoIndex: -1, audioIndex: 0 }, {}, res => { submitReply = res; });
  await tick();
  assert.equal(submitReply?.accepted, true, 'audio-only submission with videoIndex: -1 must be accepted');
};

// B54: Container selection/error, compatible audio mux, separate output fallback and no misleading MP4 extension.
tests.B54 = async function test_B54() {
  const ch = makeChromiumHarness();
  let submitReply;
  ch.handlers.message({ type: 'media.submit', url: 'http://example.test/stream.m3u8', videoIndex: 0, audioIndex: 0, container: 'mkv' }, {}, res => { submitReply = res; });
  await tick();
  assert.equal(ch.calls.messages[0]?.payload.container, 'mkv', 'container format option must be passed');
};

// B55: Stop live yields playable output; distinguish restart from resume, subtitle policy, refresh expiry, refuse DRM/encryption without decrypting.
tests.B55 = async function test_B55() {
  for (const harnessFn of [makeChromiumHarness, makeFirefoxHarness]) {
    const h = harnessFn({ nativeReply: async () => ({ accepted: true, taskId: 99 }) });
    let submitReply;
    if (h.handlers.message.length === 3) {
      h.handlers.message({ type: 'media.submit', url: 'http://example.test/clear.m3u8', videoIndex: 0, audioIndex: null }, {}, res => { submitReply = res; });
      await tick();
    } else {
      submitReply = await h.handlers.message({ type: 'media.submit', url: 'http://example.test/clear.m3u8', videoIndex: 0, audioIndex: null }, {});
    }
    assert.equal(submitReply.accepted, true);
    assert.equal(submitReply.taskId, 99);
  }
};

// B56: Request scoped to sender/browser session; fallback not wrong tab/player.
tests.B56 = async function test_B56() {
  const ch = makeChromiumHarness();
  assert.ok(typeof ch.context.handleHostPlayerQuery === 'function', 'must handle host queries to locate player/frame');
};

// B57: Domain-scoped cookie request with consent/store context and no unrestricted desktop cookie export.
tests.B57 = async function test_B57() {
  const ch = makeChromiumHarness();
  assert.ok(typeof ch.context.handleHostCookieQuery === 'function', 'must handle host requests for URL cookies');
};

// B58: Browser/daemon transport compatibility diagnostic without exposing proxy credentials to logs.
tests.B58 = async function test_B58() {
  const ch = makeChromiumHarness();
  assert.ok(typeof ch.context.handleProxyHandshake === 'function', 'must handle proxy comparison handshake with daemon');
};

// B59: Rate-limit and validate notifications, user control, platform notification tests.
tests.B59 = async function test_B59() {
  assert.ok(chromiumManifest.permissions.includes('notifications'), 'must declare notifications permission for desktop bridge');
};

// B60: Unknown/malformed operation produces structured error; never implement opaque native command echo.
tests.B60 = async function test_B60() {
  for (const harnessFn of [makeChromiumHarness, makeFirefoxHarness]) {
    const h = harnessFn();
    h.handlers.request({ url: 'http://example.test/file.bin', method: 'GET', tabId: 1, requestHeaders: [] });
    if (h.handlers.download.length === 2) h.handlers.download({ url: 'http://example.test/file.bin', filename: 'file.bin', id: 140 }, () => {});
    else h.handlers.download({ url: 'http://example.test/file.bin', filename: 'file.bin', id: 140 });
    await tick(); await tick(); await tick();
    assert.equal(h.calls.native, 1);
    const msg = h.calls.messages[0];
    assert.equal(msg.version, 1);
    assert.equal(typeof msg.type, 'string');
    assert.ok(msg.type.startsWith('download.') || msg.type.startsWith('media.'));
  }
};

// B61: Native Windows browser/process registration, peer ACL/auth and stdout isolation; complete original pipe schema not claimed.
tests.B61 = async function test_B61() {
  for (const harnessFn of [makeChromiumHarness, makeFirefoxHarness]) {
    const h = harnessFn();
    h.handlers.request({ url: 'http://example.test/iso.bin', method: 'GET', tabId: 1, requestHeaders: [] });
    if (h.handlers.download.length === 2) h.handlers.download({ url: 'http://example.test/iso.bin', filename: 'iso.bin', id: 150 }, () => {});
    else h.handlers.download({ url: 'http://example.test/iso.bin', filename: 'iso.bin', id: 150 });
    await tick(); await tick(); await tick();
    assert.equal(h.calls.nativeHosts[0], 'org.xdm.browser');
  }
};

// B62: Partial I/O, malformed UTF-8/JSON, Unicode and oversize frames, no diagnostics on stdout.
tests.B62 = async function test_B62() {
  for (const harnessFn of [makeChromiumHarness, makeFirefoxHarness]) {
    const h = harnessFn();
    let submitReply;
    if (h.handlers.message.length === 3) {
      h.handlers.message({ type: 'media.submit', url: 'http://example.test/stream.m3u8', videoIndex: 'invalid' }, {}, res => { submitReply = res; });
      await tick();
    } else {
      submitReply = await h.handlers.message({ type: 'media.submit', url: 'http://example.test/stream.m3u8', videoIndex: 'invalid' }, {});
    }
    assert.equal(submitReply.accepted, false);
    assert.equal(h.calls.native, 0, 'malformed payload must not reach native messaging');
  }
};

// Test Runner
async function run() {
  console.log('Running IDM Browser Behavior Contract Tests (B01-B62)...\n');
  const allIds = Array.from({ length: 62 }, (_, i) => `B${String(i + 1).padStart(2, '0')}`);

  let passed = 0;
  let xfailed = 0;
  let unexpectedPasses = 0;
  let unexpectedFailures = 0;

  for (const id of allIds) {
    const testFn = tests[id];
    if (!testFn) {
      console.error(`MISSING TEST: No test function implemented for ${id}`);
      unexpectedFailures++;
      continue;
    }

    const expectedFailReason = EXPECTED_FAIL[id];
    let testError = null;

    try {
      await testFn();
    } catch (err) {
      testError = err;
    }

    if (!testError) {
      if (expectedFailReason) {
        console.error(`UNEXPECTED PASS: ${id} passed but was expected to fail: ${expectedFailReason}`);
        unexpectedPasses++;
      } else {
        console.log(`PASS:  ${id}`);
        passed++;
      }
    } else {
      // Only a failed assertion documents a gap; any other error is a harness error.
      if (expectedFailReason && testError.name === 'AssertionError') {
        console.log(`XFAIL: ${id} - ${expectedFailReason}`);
        xfailed++;
      } else {
        console.error(`FAIL:  ${id} unexpected failure: ${testError.message}`);
        unexpectedFailures++;
      }
    }
  }

  console.log('\n============================================================');
  console.log(`Results: ${passed} passed, ${xfailed} expected failures (XFAIL), ${unexpectedPasses} unexpected passes, ${unexpectedFailures} unexpected failures.`);
  console.log('============================================================');

  if (unexpectedPasses > 0 || unexpectedFailures > 0) {
    process.exit(1);
  }
}

await run();
