import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const chromiumDir = new URL('../../extensions/chromium/', import.meta.url);
const firefoxDir = new URL('../../extensions/firefox/', import.meta.url);

const chromiumManifest = JSON.parse(fs.readFileSync(new URL('manifest.json', chromiumDir), 'utf8'));
const firefoxManifest = JSON.parse(fs.readFileSync(new URL('manifest.json', firefoxDir), 'utf8'));

const chromiumBgSource = fs.readFileSync(new URL('background.js', chromiumDir), 'utf8');
const videoCoreSource = fs.readFileSync(new URL('video_core.js', chromiumDir), 'utf8');
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
  storage = {},
  tabSend = async () => undefined,
  frames = [],
} = {}) {
  const handlers = {};
  const store = { ...storage };
  const calls = { notifications: [], native: 0, nativeHosts: [], resume: 0, cancel: 0, suggest: 0, messages: [], badges: [], menus: [], storageSets: [], tabMessages: [], cookieQueries: [] };
  let timeout;
  const event = name => ({ addListener: fn => { handlers[name] = fn; } });
  const chrome = {
    webRequest: {
      onBeforeSendHeaders: event('request'),
      onBeforeRequest: event('before'),
      onHeadersReceived: event('headers'),
      onBeforeRedirect: event('redirect'),
    },
    tabs: {
      onRemoved: event('removed'),
      sendMessage: async (tabId, msg, opts) => { calls.tabMessages.push({ tabId, msg, opts }); return tabSend(tabId, msg, opts); },
    },
    webNavigation: { getAllFrames: async () => frames },
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
        get: async defaults => ({ ...defaults, interceptDownloads: intercept, ...store }),
        set: async value => { calls.storageSets.push(value); Object.assign(store, value); },
      },
    },
    notifications: {
      create: (id, opts) => { calls.notifications.push({ id, opts }); },
      clear: () => {},
      onButtonClicked: event('notifButton'),
    },
    cookies: { getAll: async query => { calls.cookieQueries.push(query); return cookies(query); } },
    downloads: {
      onChanged: event('downloadChanged'),
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
  vm.runInContext(videoCoreSource, context);
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
  storage = {},
  tabSend = async () => undefined,
  frames = [],
} = {}) {
  const handlers = {};
  const store = { ...storage };
  const calls = { notifications: [], native: 0, nativeHosts: [], resume: 0, cancel: 0, erase: 0, messages: [], cookieQueries: [], badges: [], menus: [], storageSets: [], tabMessages: [] };
  let timeout;
  const event = name => ({ addListener: fn => { handlers[name] = fn; } });
  const browser = {
    webRequest: {
      onBeforeSendHeaders: event('request'),
      onBeforeRequest: event('before'),
      onHeadersReceived: event('headers'),
      onBeforeRedirect: event('redirect'),
    },
    tabs: {
      onRemoved: event('removed'),
      onUpdated: event('updated'),
      sendMessage: async (tabId, msg, opts) => { calls.tabMessages.push({ tabId, msg, opts }); return tabSend(tabId, msg, opts); },
    },
    webNavigation: { getAllFrames: async () => frames },
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
        get: async defaults => ({ ...defaults, interceptDownloads: intercept, ...store }),
        set: async value => { calls.storageSets.push(value); Object.assign(store, value); },
      },
    },
    notifications: {
      create: (id, opts) => { calls.notifications.push({ id, opts }); },
      clear: () => {},
      onClicked: event('notifClick'),
      onClosed: event('notifClosed'),
    },
    cookies: {
      getAll: async query => {
        calls.cookieQueries.push(query);
        return cookies(query);
      },
    },
    downloads: {
      onChanged: event('downloadChanged'),
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
  vm.runInContext(videoCoreSource, context);
  vm.runInContext(firefoxBgSource, context);
  return { calls, handlers, browser, context, triggerTimeout: () => timeout?.() };
}

function makeContentHarness(source, isFirefox = false, { storage, location, reply } = {}) {
  const sentMessages = [];
  const observedNodes = [];
  const intersectionObservers = [];
  const resizeObservers = [];
  let messageListener = null;
  const storageListeners = [];

  class MockIntersectionObserver {
    constructor(callback) { this.callback = callback; this.targets = []; this.disconnected = false; intersectionObservers.push(this); }
    observe(target) { this.targets.push(target); }
    disconnect() { this.disconnected = true; }
    trigger(target, isIntersecting) { this.callback([{ target, isIntersecting }]); }
  }
  class MockResizeObserver {
    constructor(callback) { this.callback = callback; this.targets = []; this.disconnected = false; resizeObservers.push(this); }
    observe(target) { this.targets.push(target); }
    disconnect() { this.disconnected = true; }
    trigger() { this.callback([]); }
  }

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
      Object.defineProperty(this.style, 'setProperty', { value: (name, value, priority) => { (this.important ||= {})[name] = [value, priority]; } });
      this.isConnected = true;
      this.listeners = {};
    }
    addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
    removeEventListener(type, fn) { this.listeners[type] = (this.listeners[type] || []).filter(f => f !== fn); }
    dispatch(type, event = {}) { for (const fn of [...(this.listeners[type] || [])]) fn({ type, stopPropagation() { event.stopped = true; }, preventDefault() { event.prevented = true; }, ...event }); return event; }
    focus() { this.focused = true; }
    attachShadow(init) {
      const root = new MockElement('#shadow-root');
      root.mode = init.mode;
      if (init.mode !== 'closed') this.shadowRoot = root;
      this.shadow = root;
      return root;
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
        return reply ? reply(msg) : { ok: true };
      },
      onMessage: {
        addListener: fn => { messageListener = fn; },
      },
    },
    ...(storage ? {
      storage: {
        local: { get: async defaults => ({ ...defaults, ...storage }) },
        onChanged: { addListener: fn => { storageListeners.push(fn); } },
      },
    } : {}),
  };

  const winListeners = {};
  const docListeners = {};

  const context = {
    ...(location ? { location } : {}),
    Element: MockElement,
    MutationObserver: MockMutationObserver,
    IntersectionObserver: MockIntersectionObserver,
    ResizeObserver: MockResizeObserver,
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
      innerWidth: 1280,
      innerHeight: 720,
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
  vm.runInContext(videoCoreSource, context);
  vm.runInContext(source, context);
  const result = {
    docEl,
    sentMessages,
    observedNodes,
    MockElement,
    docListeners,
    winListeners,
    intersectionObservers,
    resizeObservers,
    onMessage: (msg, sender = {}) => new Promise(resolve => { messageListener(msg, sender, resolve); }),
    fireStorageChange: (changes, area = 'local') => storageListeners.forEach(fn => fn(changes, area)),
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

// ---- helpers shared by the B14/B25/B32/B42/B44/B47/B48 tests ----
const bothHarnesses = [['chromium', makeChromiumHarness], ['firefox', makeFirefoxHarness]];
const bothContents = [[chromiumContentSource, false], [firefoxContentSource, true]];

async function sendRuntime(h, message, sender = {}) {
  if ('browser' in h) return h.handlers.message(message, sender);
  let result;
  h.handlers.message(message, sender, value => { result = value; });
  await tick(); await tick(); await tick();
  return result;
}

async function startDownload(h, item) {
  if ('browser' in h) h.handlers.download(item);
  else h.handlers.download(item, () => {});
  for (let i = 0; i < 6; i++) await tick();
}

async function clickMenu(h, info, tab) {
  await h.handlers.menu(info, tab);
  for (let i = 0; i < 4; i++) await tick();
}

function makeAnchor(c, href, text = '') {
  const a = new c.MockElement('a', { href });
  a.href = href;
  a.textContent = text;
  return a;
}

function makeClick(target, flags = {}) {
  const event = { target, button: 0, preventDefaultCalled: false, preventDefault() { this.preventDefaultCalled = true; }, ...flags };
  return event;
}

const plain = value => JSON.parse(JSON.stringify(value));
const rect = (left, top, width, height) => ({ left, top, width, height, right: left + width, bottom: top + height });

function appendVideo(c, src, box, parent = c.docEl) {
  const v = new c.MockElement('video', { src });
  v.getBoundingClientRect = () => v.box;
  v.box = box;
  parent.appendChild(v);
  return v;
}

// The overlay button lives in a closed shadow root; the mock keeps a handle on it for assertions.
function btnOf(panel) {
  return panel.shadow.children.find(ch => ch.tagName === 'BUTTON');
}

function panelsIn(c) {
  return c.docEl.children.filter(ch => ch.className === 'xdm-player-panel');
}

// B14: Redirect method semantics, final URL, cookies and cross-origin header stripping verified together.
tests.B14 = async function test_B14() {
  const A = 'http://origin.test/form';
  const B = 'http://mid.test/step';
  const C = 'http://cdn.test/final.bin';
  const referer = [{ name: 'Referer', value: 'http://origin.test/page' }, { name: 'Authorization', value: 'Bearer secret' }, { name: 'Cookie', value: 'origin=1' }];
  const cookies = async q => q.url.startsWith('http://cdn.test') ? [{ name: 's', value: '1' }] : [{ name: 'other', value: 'x' }];

  for (const [name, make] of bothHarnesses) {
    const label = `${name}: `;
    const post = h => {
      h.handlers.request({ url: A, method: 'POST', tabId: 1, requestHeaders: referer });
      h.handlers.before({ url: A, method: 'POST', tabId: 1, requestBody: { formData: { f: ['1'] } } });
    };

    // 307 then 308 keep the POST, its body and the original Referer; cookies and URL are those of the final hop.
    const keep = make({ cookies });
    post(keep);
    keep.handlers.redirect({ url: A, redirectUrl: B, statusCode: 307, method: 'POST', tabId: 1 });
    keep.handlers.request({ url: B, method: 'POST', tabId: 1, requestHeaders: [] });
    keep.handlers.redirect({ url: B, redirectUrl: C, statusCode: 308, method: 'POST', tabId: 1 });
    keep.handlers.request({ url: C, method: 'POST', tabId: 1, requestHeaders: [] });
    await startDownload(keep, { url: A, finalUrl: C, filename: 'final.bin', id: 1, tabId: 1 });
    let msg = keep.calls.messages[0];
    assert.equal(msg?.payload.url, C, `${label}final URL is submitted`);
    assert.equal(msg.payload.method, 'POST', `${label}307/308 keep POST`);
    assert.equal(msg.payload.body, 'f=1', `${label}307/308 keep the body`);
    assert.equal(msg.payload.requestContext.referrer, 'http://origin.test/page', `${label}original Referer survives two hops`);
    assert.equal(msg.payload.requestContext.cookies[0]?.header, 's=1', `${label}cookies come from the final origin`);
    assert.ok(keep.calls.cookieQueries.every(q => q.url === C), `${label}no cookies are read for intermediate origins`);
    assert.deepEqual(plain(msg.payload.requestContext.headers.map(h => h.name)), ['User-Agent', 'Content-Type'], `${label}Authorization and Cookie from the first hop are not forwarded across origins`);

    // 302 on POST becomes GET: no body or method is replayed, Referer is still the original one.
    const toGet = make({ cookies });
    post(toGet);
    toGet.handlers.redirect({ url: A, redirectUrl: C, statusCode: 302, method: 'POST', tabId: 1 });
    toGet.handlers.request({ url: C, method: 'GET', tabId: 1, requestHeaders: [] });
    await startDownload(toGet, { url: A, finalUrl: C, filename: 'final.bin', id: 2, tabId: 1 });
    msg = toGet.calls.messages[0];
    assert.equal(msg?.payload.url, C);
    assert.equal(msg.payload.method, undefined, `${label}302 on POST is a GET, never replayed as POST`);
    assert.equal(msg.payload.body, undefined, `${label}no body on the GET hop`);
    assert.equal(msg.payload.requestContext.referrer, 'http://origin.test/page', `${label}Referer kept across the method change`);

    // 303 drops the body even when the redirected request was never observed.
    const see = make();
    post(see);
    see.handlers.redirect({ url: A, redirectUrl: C, statusCode: 303, method: 'POST', tabId: 1 });
    await startDownload(see, { url: A, finalUrl: C, filename: 'final.bin', id: 3, tabId: 1 });
    assert.equal(see.calls.messages[0]?.payload.body, undefined, `${label}303 never replays the POST body`);
    assert.equal(see.calls.messages[0]?.payload.method, undefined);

    // A POST whose chain is unknown must stay in the browser instead of being resubmitted as GET.
    const unknown = make();
    post(unknown);
    await startDownload(unknown, { url: A, finalUrl: 'http://elsewhere.test/x.bin', filename: 'x.bin', id: 4, tabId: 1 });
    assert.equal(unknown.calls.native, 0, `${label}unobserved POST redirect is not converted to GET`);
  }
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
  for (const [source, isFf] of bothContents) {
    const c = makeContentHarness(source, isFf);
    assert.ok(c.observedNodes.some(n => n.options?.subtree && n.options?.childList), 'content script must observe mutations');
    assert.equal(typeof c.docListeners.click, 'function', 'content script must listen to link clicks');
    const a = makeAnchor(c, 'http://example.test/file.zip');
    const inner = new c.MockElement('span');
    a.appendChild(inner);
    const sent = () => c.sentMessages.filter(m => m.type === 'capture.override');

    // Default combinations: Alt = bypass, Alt+Shift = force; anything else is ignored.
    const bypass = makeClick(inner, { altKey: true, shiftKey: false, ctrlKey: false, metaKey: false });
    c.docListeners.click(bypass);
    assert.deepEqual(sent().map(m => [m.mode, m.url]), [['bypass', 'http://example.test/file.zip']], 'Alt-click inside a link is bypass');
    assert.equal(bypass.preventDefaultCalled, false, 'bypass must leave the click to the browser');
    const force = makeClick(a, { altKey: true, shiftKey: true, ctrlKey: false, metaKey: false });
    c.docListeners.click(force);
    assert.equal(sent()[1]?.mode, 'force', 'Alt+Shift-click is force');
    assert.equal(force.preventDefaultCalled, true, 'force must stop the browser from handling the click');
    c.docListeners.click(makeClick(a, { altKey: false, shiftKey: false, ctrlKey: true, metaKey: false }));
    c.docListeners.click(makeClick(a, { altKey: false, shiftKey: false, ctrlKey: false, metaKey: false }));
    c.docListeners.click(makeClick(a, { altKey: true, button: 2 }));
    c.docListeners.click(makeClick(makeAnchor(c, 'javascript:void(0)'), { altKey: true }));
    c.docListeners.click(makeClick(new c.MockElement('div'), { altKey: true }));
    assert.equal(sent().length, 2, 'unmapped modifiers, plain clicks, other buttons, non-http links and non-links are ignored');

    // Keys are user-configurable; ambiguous configuration falls back to the defaults.
    c.setCaptureKeys({ force: 'ctrl', bypass: 'meta' });
    c.docListeners.click(makeClick(a, { altKey: false, shiftKey: false, ctrlKey: true, metaKey: false }));
    c.docListeners.click(makeClick(a, { altKey: false, shiftKey: false, ctrlKey: false, metaKey: true }));
    c.docListeners.click(makeClick(a, { altKey: true, shiftKey: false, ctrlKey: false, metaKey: false }));
    assert.deepEqual(sent().slice(2).map(m => m.mode), ['force', 'bypass'], 'configured keys replace the defaults');
    c.setCaptureKeys({ force: 'ctrl', bypass: 'Control' });
    assert.deepEqual({ ...c.captureKeys }, { force: 'alt+shift', bypass: 'alt' }, 'identical combinations are rejected');

    // The tracked state is read for synthetic events without flags, and blur clears it (no stuck override).
    c.setCaptureKeys({ force: 'alt+shift', bypass: 'alt' });
    c.keydownHandler({ key: 'Alt', altKey: true });
    assert.equal(c.modifierState.alt, true);
    const before = sent().length;
    c.docListeners.click(makeClick(a));
    assert.equal(sent().length, before + 1, 'held Alt without event flags still counts');
    c.winListeners.blur();
    assert.equal(c.modifierState.alt, false, 'blur clears held modifiers');
    c.docListeners.click(makeClick(a));
    assert.equal(sent().length, before + 1, 'no override remains after blur');
    c.keydownHandler({ key: 'Control', ctrlKey: true, altKey: true });
    c.keyupHandler({ key: 'Alt', altKey: false, ctrlKey: true });
    assert.equal(c.modifierState.alt, false, 'Alt release clears alt');
    assert.equal(c.modifierState.ctrl, true);
    c.resetModifiers();
    assert.equal(c.modifierState.ctrl, false);

    const detached = new c.MockElement('video', { src: 'http://example.test/d.mp4' });
    detached.isConnected = false;
    assert.equal(c.queryElementMetadata(detached), null, 'detached element must not attach metadata');

    // Stored configuration is loaded and live updates apply.
    const stored = makeContentHarness(source, isFf, { storage: { captureKeys: { force: 'ctrl+shift', bypass: 'ctrl' } } });
    await tick(); await tick();
    assert.equal(stored.captureKeys.bypass, 'ctrl', 'saved captureKeys must be loaded');
    stored.fireStorageChange({ captureKeys: { newValue: { force: 'meta', bypass: 'shift' } } });
    assert.equal(stored.captureKeys.force, 'meta', 'option changes apply without reload');
  }

  // Background: bypass keeps the browser download, force hands over even when filtered, never in private windows.
  const page = 'http://example.test/page';
  for (const [name, make] of bothHarnesses) {
    const url = 'http://example.test/keep.zip';
    const control = make();
    control.handlers.request({ url, method: 'GET', tabId: 1, requestHeaders: [] });
    await startDownload(control, { url, filename: 'keep.zip', id: 1, tabId: 1 });
    assert.equal(control.calls.native, 1, `${name}: without an override the download is handed to XDM`);

    const bypass = make();
    bypass.handlers.request({ url, method: 'GET', tabId: 1, requestHeaders: [] });
    const reply = await sendRuntime(bypass, { type: 'capture.override', mode: 'bypass', url, pageUrl: page }, { tab: { id: 1, url: page } });
    assert.equal(reply.ok, true);
    await startDownload(bypass, { url, filename: 'keep.zip', id: 1, tabId: 1 });
    assert.equal(bypass.calls.native, 0, `${name}: bypass must not submit to XDM`);
    assert.equal(bypass.calls.cancel, 0, `${name}: bypass must not cancel the browser download`);
    await startDownload(bypass, { url, filename: 'keep.zip', id: 2, tabId: 1 });
    assert.equal(bypass.calls.native, 1, `${name}: bypass applies to one click only`);

    const excluded = 'http://www.bank.example/statement.pdf';
    const filtered = make({ intercept: false });
    filtered.handlers.request({ url: excluded, method: 'GET', tabId: 1, requestHeaders: [] });
    await startDownload(filtered, { url: excluded, filename: 'statement.pdf', id: 3, tabId: 1 });
    assert.equal(filtered.calls.native, 0, `${name}: filtered download stays in the browser`);
    const forced = await sendRuntime(filtered, { type: 'capture.override', mode: 'force', url: excluded, pageUrl: page }, { tab: { id: 1, url: page } });
    assert.equal(forced.ok, true);
    assert.equal(filtered.calls.messages[0]?.type, 'download.submit', `${name}: force submits even when excluded and interception is off`);
    assert.equal(filtered.calls.messages[0]?.payload.url, excluded);
    assert.equal(filtered.calls.messages[0]?.payload.requestContext.referrer, new URL(page).origin + '/', 'cross-origin page referrer is origin-only');

    const priv = make({ intercept: false });
    const denied = await sendRuntime(priv, { type: 'capture.override', mode: 'force', url: excluded, pageUrl: page }, { tab: { id: 9, url: page, incognito: true } });
    assert.equal(denied.ok, false, `${name}: private windows are never overridden`);
    assert.equal(priv.calls.native, 0);
    const bad = await sendRuntime(priv, { type: 'capture.override', mode: 'force', url: 'file:///etc/passwd' }, { tab: { id: 1 } });
    assert.equal(bad.ok, false);
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
  ch.handlers.message({ type: 'batch.submit', items: [{ url: 'http://example.test/a.zip' }] }, {}, res => { batchReply = res; });
  await tick(); await tick(); await tick();
  assert.ok(batchReply?.accepted, 'must implement batch.submit message handler');
};

// B32: Selection rect/DPI/frame ancestry, no-selection removal, matching link counts.
tests.B32 = async function test_B32() {
  for (const [source, isFf] of bothContents) {
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

    // The selection consumer: links inside the selection (not other page links) are collected for the batch flow.
    const inside = makeAnchor(content, 'http://example.test/a.zip', 'A');
    const outside = makeAnchor(content, 'http://example.test/b.zip', 'B');
    content.docEl.appendChild(inside);
    content.docEl.appendChild(outside);
    const liveSel = { ...mockSel, toString: () => 'pick these http://example.test/typed.pdf', containsNode: n => n === inside };
    content.window.getSelection = () => liveSel;
    const reply = await content.onMessage({ type: 'page.extractLinks', options: { selectionOnly: true } });
    assert.deepEqual(plain(reply.links.map(l => l.url).sort()), ['http://example.test/a.zip', 'http://example.test/typed.pdf'], 'only selected anchors and typed URLs are returned');
    assert.equal(content.selectionHandler(liveSel).linkCount, 2, 'link count includes selected anchors');
    const all = await content.onMessage({ type: 'page.extractLinks', options: {} });
    assert.ok(all.links.some(l => l.url === 'http://example.test/b.zip'), 'all-links mode includes unselected anchors');
  }

  // Link filter: dedupe, extension list, hide HTML, hide images.
  const sample = [
    { url: 'http://x.test/a.zip#top', type: 'anchor' },
    { url: 'http://x.test/a.zip', type: 'anchor' },
    { url: 'http://x.test/doc.pdf', type: 'anchor' },
    { url: 'http://x.test/page.html', type: 'anchor' },
    { url: 'http://x.test/dir/', type: 'anchor' },
    { url: 'http://x.test/pic.png', type: 'image' },
    { url: 'http://x.test/photo.jpg', type: 'anchor' },
    { url: 'ftp://x.test/a.zip', type: 'anchor' },
    { url: 'http://x.test/get?id=3', type: 'anchor' },
  ];
  const ff = makeFirefoxHarness();
  const ch = makeChromiumHarness();
  for (const filterLinks of [ff.context.XdmCore.filterLinks, ch.context.filterLinks]) {
    const urls = f => plain(filterLinks(sample, f).map(l => l.url));
    assert.deepEqual(urls(undefined), ['http://x.test/a.zip', 'http://x.test/doc.pdf', 'http://x.test/get?id=3'], 'defaults dedupe and hide HTML and images');
    assert.deepEqual(urls({ extensions: ['.ZIP', 'pdf'] }), ['http://x.test/a.zip', 'http://x.test/doc.pdf'], 'extension list is case/dot insensitive');
    assert.deepEqual(urls({ extensions: 'pdf, zip' }), ['http://x.test/a.zip', 'http://x.test/doc.pdf']);
    assert.ok(urls({ hideHtml: false }).includes('http://x.test/page.html'), 'hideHtml:false keeps pages');
    assert.ok(urls({ hideImages: false }).includes('http://x.test/pic.png'), 'hideImages:false keeps images');
  }

  // Background flows: one batch handoff per menu click.
  const page = 'http://example.test/page';
  const frameLinks = {
    0: [{ url: 'http://example.test/a.zip', type: 'anchor' }, { url: 'http://example.test/page2.html', type: 'anchor' }, { url: 'http://example.test/i.png', type: 'image' }],
    7: [{ url: 'http://example.test/b.zip', type: 'anchor' }, { url: 'http://example.test/a.zip', type: 'anchor' }],
  };
  const cookies = async q => [{ name: 'k', value: new URL(q.url).pathname.slice(1) }];
  for (const [name, make] of bothHarnesses) {
    const mk = (extra = {}) => make({
      cookies,
      frames: [{ frameId: 0 }, { frameId: 7 }],
      tabSend: async (_tab, _msg, opts) => ({ links: frameLinks[opts.frameId] || [] }),
      nativeReply: async m => m.type === 'batch.submit' ? { version: 1, type: 'batch.result', accepted: true, count: m.payload.items.length } : { accepted: true },
      ...extra,
    });
    const inst = mk();
    inst.handlers.installed();
    assert.ok(inst.calls.menus.some(m => m.id === 'xdm-selection' && m.contexts.includes('selection')), `${name}: selection context menu`);
    assert.ok(inst.calls.menus.some(m => m.id === 'xdm-downloads' && /all links/i.test(m.title)), `${name}: all-links context menu`);

    // Download all links: every frame is queried, results merged, filtered and sent as one batch.
    const all = mk();
    await clickMenu(all, { menuItemId: 'xdm-downloads' }, { id: 3, url: page });
    assert.deepEqual(all.calls.tabMessages.map(m => m.opts.frameId), [0, 7], `${name}: all frames are queried`);
    assert.equal(all.calls.messages.length, 1, `${name}: one native message for the whole batch`);
    const batch = all.calls.messages[0];
    assert.equal(batch.type, 'batch.submit');
    assert.equal(batch.version, 1);
    assert.equal(batch.payload.count, 2);
    assert.equal(batch.payload.batchId, batch.requestId);
    assert.deepEqual(plain(batch.payload.items.map(i => i.url)), ['http://example.test/a.zip', 'http://example.test/b.zip'], `${name}: deduped, HTML and images hidden, order kept`);
    assert.ok(batch.payload.items.every(i => i.requestContext.referrer === page && i.requestId.startsWith(batch.payload.batchId)), `${name}: each item carries page context and a batch-scoped id`);
    assert.deepEqual(plain(batch.payload.items.map(i => i.requestContext.cookies[0].header)), ['k=a.zip', 'k=b.zip'], `${name}: per-link cookies`);

    // Extension list from storage plus HTML shown.
    const extFilter = mk({ storage: { linkFilter: { extensions: ['html'], hideHtml: false, hideImages: true } } });
    await clickMenu(extFilter, { menuItemId: 'xdm-downloads' }, { id: 3, url: page });
    assert.deepEqual(plain(extFilter.calls.messages[0]?.payload.items.map(i => i.url)), ['http://example.test/page2.html'], `${name}: stored filter is applied`);

    // Selected links: only the frame that owns the selection is asked; selectionText URLs are included.
    const sel = mk({ tabSend: async (_t, msg, opts) => ({ links: msg.options.selectionOnly && opts.frameId === 7 ? [{ url: 'http://example.test/b.zip', type: 'anchor' }] : [] }) });
    await clickMenu(sel, { menuItemId: 'xdm-selection', frameId: 7, selectionText: 'also http://example.test/c.iso' }, { id: 3, url: page });
    assert.deepEqual(sel.calls.tabMessages.map(m => m.opts.frameId), [7]);
    assert.equal(sel.calls.tabMessages[0].msg.options.selectionOnly, true);
    assert.deepEqual(plain(sel.calls.messages[0].payload.items.map(i => i.url)), ['http://example.test/b.zip', 'http://example.test/c.iso']);

    // Host without batch.submit: the same items are handed over one by one.
    const fallback = mk({ nativeReply: async m => m.type === 'batch.submit' ? { version: 1, type: 'error', reason: 'unsupported message type' } : { accepted: true } });
    await clickMenu(fallback, { menuItemId: 'xdm-downloads' }, { id: 3, url: page });
    const types = fallback.calls.messages.map(m => m.type);
    assert.deepEqual(types, ['batch.submit', 'download.submit', 'download.submit'], `${name}: falls back per item`);
    assert.deepEqual(fallback.calls.messages.slice(1).map(m => m.payload.url), ['http://example.test/a.zip', 'http://example.test/b.zip']);

    // Cookies: everything for same-site links, only SameSite=None for cross-site links.
    const jar = async () => [{ name: 's', value: '1', sameSite: 'lax' }, { name: 'st', value: '2', sameSite: 'strict' }, { name: 'u', value: '3', sameSite: 'unspecified' }, { name: 'n', value: '4', sameSite: 'no_restriction' }];
    const sites = mk({
      cookies: jar,
      tabSend: async () => ({ links: [
        { url: 'http://dl.example.test/same.zip', type: 'anchor' },
        { url: 'http://cdn.other.test/cross.zip', type: 'anchor' },
        { url: 'http://shop.example.co.uk/uk.zip', type: 'anchor' },
      ] }),
    });
    await clickMenu(sites, { menuItemId: 'xdm-downloads' }, { id: 3, url: 'http://example.test/page?q=1#frag' });
    const byUrl = Object.fromEntries(sites.calls.messages[0].payload.items.map(i => [i.url, i.requestContext]));
    assert.equal(byUrl['http://dl.example.test/same.zip'].cookies[0].header, 's=1; st=2; u=3; n=4', `${name}: same-site links keep the session`);
    assert.equal(byUrl['http://cdn.other.test/cross.zip'].cookies[0].header, 'n=4', `${name}: cross-site links get only SameSite=None cookies`);
    assert.deepEqual(plain(byUrl['http://shop.example.co.uk/uk.zip'].cookies.map(c => c.header)), ['n=4'], `${name}: example.co.uk is not same-site with example.test`);
    // Referer: origin only across origins, full (without fragment) for the same origin.
    assert.equal(byUrl['http://cdn.other.test/cross.zip'].referrer, 'http://example.test/', `${name}: cross-origin referrer is origin-only`);
    assert.equal(byUrl['http://dl.example.test/same.zip'].referrer, 'http://example.test/', `${name}: sibling origin is cross-origin`);
    const same = mk({ tabSend: async () => ({ links: [{ url: 'http://example.test/o.zip', type: 'anchor' }] }) });
    await clickMenu(same, { menuItemId: 'xdm-downloads' }, { id: 3, url: 'http://example.test/page?q=1#frag' });
    assert.equal(same.calls.messages[0].payload.items[0].requestContext.referrer, 'http://example.test/page?q=1', `${name}: same-origin referrer keeps path and query, drops fragment`);

    // A host validation error is surfaced; it must not trigger per-link submission.
    const rejecting = mk({ nativeReply: async m => m.type === 'batch.submit' ? { version: 1, type: 'error', reason: 'item 0: invalid URL' } : { accepted: true } });
    await clickMenu(rejecting, { menuItemId: 'xdm-downloads' }, { id: 3, url: page });
    assert.deepEqual(rejecting.calls.messages.map(m => m.type), ['batch.submit'], `${name}: validation errors are not retried per link`);

    // The runtime passthrough rebuilds items: only validated URLs survive, foreign context is discarded.
    const pass = mk();
    const passReply = await sendRuntime(pass, { type: 'batch.submit', pageUrl: page, items: [
      { url: 'http://example.test/a.zip', requestContext: { headers: [{ name: 'Host', value: 'evil' }], cookies: [{ url: 'http://example.test/', header: 'forged=1' }] } },
      { url: 'ftp://example.test/y.zip' }, { url: 'http://u:p@example.test/z.zip' }, { url: 'file:///etc/passwd' }, 'javascript:1',
    ] });
    assert.ok(passReply?.accepted, `${name}: passthrough accepted`);
    assert.equal(pass.calls.messages.length, 1);
    const forwarded = pass.calls.messages[0].payload.items;
    assert.deepEqual(plain(forwarded.map(i => i.url)), ['http://example.test/a.zip'], `${name}: only valid web links are forwarded`);
    assert.ok(!JSON.stringify(forwarded).includes('forged') && !JSON.stringify(forwarded).includes('evil'), `${name}: caller-supplied context is discarded`);
    const foreign = mk();
    const foreignReply = await sendRuntime(foreign, { type: 'batch.submit', items: [{ url: 'http://example.test/a.zip' }] }, { id: 'someone-else' });
    assert.equal(foreignReply?.accepted, false);
    assert.equal(foreign.calls.native, 0, `${name}: messages from other extensions are ignored`);

    // Single handoffs enforce the host's URL rule too.
    const single = mk();
    for (const bad of ['http://user:pw@example.test/a.zip', 'ftp://example.test/a.zip', 'file:///etc/passwd']) {
      const r = await single.context.submit(bad, 'a.zip');
      assert.equal(r.accepted, false, `${name}: ${bad} refused`);
    }
    assert.equal(single.calls.native, 0);

    // Nothing matching: nothing is sent.
    const none = mk({ tabSend: async () => ({ links: [{ url: 'http://example.test/x.html', type: 'anchor' }] }) });
    await clickMenu(none, { menuItemId: 'xdm-downloads' }, { id: 3, url: page });
    assert.equal(none.calls.native, 0, `${name}: empty filtered list is not sent`);
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
  const past = Math.floor(Date.now() / 1000) - 3600;
  const future = Math.floor(Date.now() / 1000) + 3600;
  for (const [source, isFf] of bothContents) {
    const content = makeContentHarness(source, isFf);
    assert.ok(Array.isArray(content.siteAdapters), 'siteAdapters must be an array');
    assert.ok(content.siteAdapters.length >= 2, 'must provide multiple site adapters');
    for (const a of content.siteAdapters) {
      assert.ok(typeof a.id === 'string', 'adapter must have string ID');
      assert.ok(typeof a.version === 'number', 'adapter must declare version');
      assert.ok(typeof a.minHostVersion === 'number', 'adapter must declare the host protocol version it needs');
      assert.ok(typeof a.matches === 'function', 'adapter must have matches predicate');
      assert.ok(typeof a.extractMedia === 'function', 'adapter must have extractMedia function');
      assert.ok(typeof a.refreshUrl === 'function', 'adapter must have refreshUrl handler');
    }
    assert.equal(content.getSiteAdapter('http://example.test/stream').id, 'embedded-stream', 'specific adapter wins over the generic one');
    assert.equal(content.getSiteAdapter('http://example.test/video').id, 'generic-html5', 'generic adapter is the fallback');

    // Host-version gating: an adapter that needs a newer host is not used.
    content.siteAdapters.unshift({ id: 'future', version: 1, minHostVersion: 2, matches: () => true, extractMedia: () => [], refreshUrl: async m => m });
    assert.ok(!content.enabledAdapters(1).some(a => a.id === 'future'), 'adapter requiring host 2 is disabled on host 1');
    assert.equal(content.getSiteAdapter('http://example.test/video', 1).id, 'generic-html5');
    assert.equal(content.getSiteAdapter('http://example.test/video', 2).id, 'future', 'adapter is enabled once the host is new enough');
    content.siteAdapters.shift();

    // Expiry detection from signed-URL parameters only.
    assert.equal(content.urlExpired(`http://x.test/v.mp4?expires=${past}`), true);
    assert.equal(content.urlExpired(`http://x.test/v.mp4?expires=${future}`), false);
    assert.equal(content.urlExpired('http://x.test/v.mp4'), false);
    assert.equal(content.urlExpired('http://x.test/v.mp4?X-Amz-Date=20200101T000000Z&X-Amz-Expires=60'), true);

    // The inspection path goes through the adapters and reports adapter identity and auth expiry.
    const plain = appendVideo(content, 'http://example.test/plain.mp4', rect(0, 0, 640, 360));
    const signed = appendVideo(content, `http://example.test/signed.mp4?expires=${past}`, rect(0, 400, 640, 360));
    const hls = new content.MockElement('video');
    const src = new content.MockElement('source', { src: 'http://example.test/stream/live.m3u8' });
    src.src = 'http://example.test/stream/live.m3u8';
    hls.appendChild(src);
    hls.getBoundingClientRect = () => rect(0, 800, 640, 360);
    content.docEl.appendChild(hls);
    content.observedNodes[0].observer.triggerAdd(content.docEl);
    const msgs = Object.fromEntries(content.sentMessages.filter(m => m.type === 'page.media').map(m => [m.url, m]));
    assert.equal(msgs['http://example.test/plain.mp4'].adapterId, 'generic-html5');
    assert.equal(msgs['http://example.test/plain.mp4'].adapterVersion, 2);
    assert.equal(msgs['http://example.test/plain.mp4'].expired, undefined, 'non-expiring URL is not flagged');
    assert.equal(msgs[`http://example.test/signed.mp4?expires=${past}`].expired, true, 'expired signed URL is reported as expired');
    assert.equal(msgs['http://example.test/stream/live.m3u8'].adapterId, 'embedded-stream', 'stream source is attributed to the embedded-stream adapter');

    // refreshUrl really re-reads the player: a rotated URL replaces the stale one.
    const adapter = content.getSiteAdapter('http://example.test/plain.mp4');
    let refreshed = await adapter.refreshUrl({ url: 'http://example.test/plain.mp4' });
    assert.deepEqual({ url: refreshed.url, refreshed: refreshed.refreshed, expired: refreshed.expired }, { url: 'http://example.test/plain.mp4', refreshed: false, expired: false });
    plain.src = plain.currentSrc = 'http://example.test/plain-rotated.mp4';
    refreshed = await adapter.refreshUrl({ url: 'http://example.test/plain.mp4' });
    assert.equal(refreshed.url, 'http://example.test/plain-rotated.mp4', 'rotated player URL is returned');
    assert.equal(refreshed.refreshed, true);
    const gone = await adapter.refreshUrl({ url: `http://example.test/gone.mp4?expires=${past}` });
    assert.equal(gone.expired, true, 'a vanished player with an expired URL is reported expired');
    assert.equal(gone.refreshed, false);
    const viaMessage = await content.onMessage({ type: 'page.refreshMedia', url: 'http://example.test/plain.mp4' });
    assert.equal(viaMessage.url, 'http://example.test/plain-rotated.mp4', 'page.refreshMedia exposes the refresh to the background');
    assert.equal(viaMessage.adapterId, 'generic-html5');
    void signed;
  }

  // Background: media.refresh updates the catalog entry; page.mediaGone drops it.
  for (const [name, make] of bothHarnesses) {
    const h = make({ tabSend: async () => ({ refreshed: true, url: 'http://example.test/new.mp4', expired: false }) });
    h.handlers.headers({ tabId: 5, url: 'http://example.test/old.mp4', responseHeaders: [{ name: 'Content-Type', value: 'video/mp4' }] });
    const r = await sendRuntime(h, { type: 'media.refresh', tabId: 5, url: 'http://example.test/old.mp4' });
    assert.equal(r.ok, true);
    assert.equal(r.url, 'http://example.test/new.mp4');
    let list = await sendRuntime(h, { type: 'media.list', tabId: 5 });
    assert.deepEqual(plain(list.items.map(i => i.url)), ['http://example.test/new.mp4'], `${name}: catalog entry carries the refreshed URL`);
    await sendRuntime(h, { type: 'page.mediaGone', url: 'http://example.test/new.mp4' }, { tab: { id: 5 } });
    list = await sendRuntime(h, { type: 'media.list', tabId: 5 });
    assert.equal(list.items.length, 0, `${name}: removed player leaves the catalog`);
    const bad = await sendRuntime(h, { type: 'media.refresh', tabId: 5, url: 'file:///x' });
    assert.equal(bad.ok, false);
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
  for (const [source, isFf] of bothContents) {
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
    assert.equal(panelsIn(content).length, 0, 'explicit unregister removes the overlay from the page');

    // Removal through the MutationObserver, including a player nested in a removed container.
    const box = new content.MockElement('div');
    const nested = appendVideo(content, 'http://example.test/nested.mp4', rect(0, 0, 640, 360), box);
    content.docEl.appendChild(box);
    content.observedNodes[0].observer.triggerAdd(box);
    assert.equal(content.playerRegistry.size(), 1, 'player is registered by inspection');
    assert.equal(panelsIn(content).length, 1, 'overlay is attached for the detected player');
    const io = content.intersectionObservers[0];
    const ro = content.resizeObservers[0];
    assert.ok(io && ro, 'per-player intersection and resize observers exist');

    content.sentMessages.length = 0;
    content.docEl.removeChild(box);
    content.observedNodes[0].observer.triggerRemove(box);
    assert.equal(content.playerRegistry.size(), 0, 'removed container drops its player from the registry');
    assert.equal(panelsIn(content).length, 0, 'removed player leaves no overlay');
    assert.equal(io.disconnected && ro.disconnected, true, 'observers are disconnected on removal');
    assert.ok(content.sentMessages.some(m => m.type === 'page.mediaGone' && m.url === 'http://example.test/nested.mp4'), 'removal is reported to the background');

    // A moved player (removed and re-added in one batch) keeps its overlay.
    const mover = appendVideo(content, 'http://example.test/move.mp4', rect(0, 0, 640, 360));
    content.observedNodes[0].observer.triggerAdd(mover);
    content.observedNodes[0].observer.callback([{ addedNodes: [], removedNodes: [mover] }]);
    assert.equal(content.playerRegistry.size(), 1, 'a player still in the document is not dropped');

    // Replacement player showing the same URL gets its own overlay; the old one is gone.
    const old = mover;
    content.docEl.removeChild(old);
    content.observedNodes[0].observer.triggerRemove(old);
    assert.equal(panelsIn(content).length, 0);
    const replacement = appendVideo(content, 'http://example.test/move.mp4', rect(0, 0, 640, 360));
    content.observedNodes[0].observer.triggerAdd(replacement);
    assert.equal(content.playerRegistry.size(), 1, 'replacement is registered');
    assert.equal(panelsIn(content).length, 1, 'replacement has exactly one overlay');
    void nested;

    // Source identity: the same element with a new source keeps its id and the button downloads the current URL.
    const idBefore = [...content.playerRegistry.entries.keys()][0];
    replacement.src = replacement.currentSrc = 'http://example.test/quality-720.mp4';
    content.observedNodes[0].observer.triggerAdd(replacement);
    assert.deepEqual(plain([...content.playerRegistry.entries.keys()]), [idBefore], 'element keeps its stable id');
    assert.equal(content.playerRegistry.get(idBefore).url, 'http://example.test/quality-720.mp4');
    assert.equal(panelsIn(content).length, 1, 'source change does not duplicate overlays');
    content.sentMessages.length = 0;
    btnOf(panelsIn(content)[0]).onclick();
    await tick();
    assert.equal(content.sentMessages[0]?.playerUrl, 'http://example.test/quality-720.mp4', 'overlay menu is built for the current source');
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
  for (const [source, isFf] of bothContents) {
    const content = makeContentHarness(source, isFf);
    assert.ok(typeof content.debounceTimer !== 'undefined', 'DOM inspection must be debounced with 200ms timer');
    assert.equal(content.DEBOUNCE_MS, 200, 'debounce delay must be 200ms');
    assert.equal(content.debounceTimer, null, 'idle page keeps no pending timer');

    const v1 = new content.MockElement('video', { src: 'http://example.test/v1.mp4' });
    content.observedNodes[0]?.observer.triggerAdd(v1);
    assert.ok(content.debounceTimer !== null, 'debounce timer must activate on mutation');
    content.triggerTimeout();
    assert.equal(content.debounceTimer, null, 'debounce timer clears once it fires');

    // Attribute change on a live player re-inspects after the debounce and updates the source.
    const video = appendVideo(content, 'http://example.test/a.mp4', rect(100, 100, 640, 360));
    content.observedNodes[0].observer.triggerAdd(video);
    content.triggerTimeout();
    const id = [...content.playerRegistry.entries.keys()][0];
    video.src = video.currentSrc = 'http://example.test/b.mp4';
    content.observedNodes[0].observer.callback([{ target: video, attributeName: 'src', addedNodes: [], removedNodes: [] }]);
    assert.ok(content.debounceTimer !== null, 'attribute mutation schedules a debounced inspection');
    content.triggerTimeout();
    assert.equal(content.playerRegistry.get(id).url, 'http://example.test/b.mp4', 'src attribute change is picked up');
    assert.ok(content.sentMessages.some(m => m.type === 'page.media' && m.url === 'http://example.test/b.mp4'));

    // Own overlay restyling must not trigger inspection (no self-feedback loop).
    const panel = panelsIn(content)[0];
    content.observedNodes[0].observer.callback([{ target: panel, attributeName: 'style', addedNodes: [], removedNodes: [] }]);
    assert.equal(content.debounceTimer, null, 'overlay mutations are ignored');

    // Visibility: intersection hides and shows the overlay.
    const io = content.intersectionObservers.find(o => o.targets.includes(video));
    const ro = content.resizeObservers.find(o => o.targets.includes(video));
    assert.equal(panel.style.display, 'block', 'visible player has a visible overlay');
    io.trigger(video, false);
    assert.equal(panel.style.display, 'none', 'player scrolled out of view hides the overlay');
    io.trigger(video, true);
    assert.equal(panel.style.display, 'block');

    // Resize: overlay follows the player box and hides when the player becomes tiny.
    assert.equal(panel.style.top, '104px');
    assert.equal(panel.style.left, `${100 + 640 - 144 - 4}px`);
    video.box = rect(200, 50, 800, 450);
    ro.trigger();
    assert.equal(panel.style.top, '54px', 'resize repositions the overlay');
    assert.equal(panel.style.left, `${200 + 800 - 144 - 4}px`);
    video.box = rect(200, 50, 20, 20);
    ro.trigger();
    assert.equal(panel.style.display, 'none', 'tiny player has no overlay');

    // Scroll re-aligns without any timer; a player scrolled off-screen hides.
    video.box = rect(10, 30, 640, 360);
    content.winListeners.scroll();
    assert.equal(panel.style.display, 'block');
    assert.equal(panel.style.top, '34px', 'scroll repositions the overlay');
    video.box = rect(10, -500, 640, 360);
    content.winListeners.scroll();
    assert.equal(panel.style.display, 'none', 'off-screen player hides the overlay on scroll');
    content.winListeners.resize();
    assert.equal(content.debounceTimer, null, 'scroll and resize never start inspection timers');

    // Navigation drops every overlay and observer.
    video.box = rect(10, 30, 640, 360);
    content.winListeners.pagehide();
    assert.equal(content.playerRegistry.size(), 0, 'pagehide clears the registry');
    assert.equal(panelsIn(content).length, 0, 'pagehide leaves no overlay');
    assert.ok(content.intersectionObservers.every(o => o.disconnected) && content.resizeObservers.every(o => o.disconnected), 'all observers are disconnected');

    // A player detached without a mutation record is purged by popstate cleanup.
    const late = appendVideo(content, 'http://example.test/late.mp4', rect(0, 0, 640, 360));
    content.observedNodes[0].observer.triggerAdd(late);
    assert.equal(content.playerRegistry.size(), 1);
    content.docEl.removeChild(late);
    content.winListeners.popstate();
    assert.equal(content.playerRegistry.size(), 0, 'SPA navigation purges detached players');
    assert.equal(panelsIn(content).length, 0);
  }
};

// B48: Independent original-looking panel with defined desktop/DOM ownership, focus, hide/show and no intrusion on unrelated windows.
tests.B48 = async function test_B48() {
  for (const [source, isFf] of bothContents) {
    const content = makeContentHarness(source, isFf);
    assert.ok(typeof content.playerPanel !== 'undefined', 'content script must inject floating player panel');

    const panel0 = content.playerPanel.create('p1', { url: 'http://example.test/vid.mp4', title: 'Vid' });
    assert.ok(panel0, 'player panel must be created');
    assert.equal(content.playerPanel.get('p1'), panel0);
    content.playerPanel.hide('p1');
    assert.equal(panel0.visible === false || panel0.style?.display === 'none', true, 'panel must hide');
    content.playerPanel.show('p1');
    assert.equal(panel0.visible === true || panel0.style?.display === 'block', true, 'panel must show');
    content.playerPanel.remove('p1');
    assert.equal(content.playerPanel.get('p1'), null, 'panel must be cleaned up on remove');

    // A detected player gets an overlay that is really attached to the document.
    const video = appendVideo(content, 'http://example.test/movie.mp4', rect(50, 100, 640, 360));
    video.title = 'Movie';
    content.observedNodes[0].observer.triggerAdd(video);
    const attached = panelsIn(content);
    assert.equal(attached.length, 1, 'overlay is attached to the page document');
    const panel = attached[0];
    assert.equal(panel.parentNode, content.docEl, 'overlay is owned by the document element');
    assert.equal(panel.style.position, 'fixed', 'overlay is viewport-anchored');
    assert.equal(panel.style.display, 'block', 'overlay shows for a large visible player');
    assert.equal(panel.style.top, '104px');
    assert.equal(panel.style.left, `${50 + 640 - 144 - 4}px`, 'overlay sits at the player top-right corner');
    assert.equal(panel.shadow.mode, 'closed', 'overlay UI lives in a closed shadow root');
    assert.equal(panel.shadowRoot, undefined, 'closed shadow root is not reachable from the page');
    assert.equal(panel.children.length, 0, 'no overlay UI is exposed as light-DOM children page CSS could style');
    assert.equal(btnOf(panel).textContent, 'Download this video');

    content.sentMessages.length = 0;
    btnOf(panel).onclick({ stopPropagation() {}, preventDefault() {} });
    await tick();
    assert.deepEqual(content.sentMessages.map(m => [m.type, m.playerUrl, m.title]), [['video.options', 'http://example.test/movie.mp4', 'Movie']], 'button asks for the formats of the player URL');

    // No intrusion: tiny players and 1x1 pixels never show an overlay.
    const tiny = appendVideo(content, 'http://example.test/tiny.mp4', rect(0, 0, 20, 20));
    content.observedNodes[0].observer.triggerAdd(tiny);
    const tinyPanel = panelsIn(content).find(p => p !== panel);
    assert.equal(tinyPanel?.style.display, 'none', 'tiny player overlay stays hidden');

    // Removing the player removes the overlay.
    content.docEl.removeChild(video);
    content.observedNodes[0].observer.triggerRemove(video);
    assert.ok(!panelsIn(content).includes(panel), 'overlay is detached with its player');
  }
};


// ---------------------------------------------------------------------------
// Media-panel policy tests (extra IDs, run by default and by filter: node contracts_browser.mjs MEDIA_SIZE)
// ---------------------------------------------------------------------------

const respHeaders = (mime, extra = []) => [{ name: 'Content-Type', value: mime }, ...extra];
const cl = n => ({ name: 'Content-Length', value: String(n) });

async function mediaItems(h, tabId) {
  return plain((await sendRuntime(h, { type: 'media.list', tabId })).items.map(i => i.url.split('/').pop()));
}

function headersEvent(h, tabId, url, headers, statusCode = 200, origin = 'http://page.test') {
  h.handlers.headers({ tabId, url, responseHeaders: headers, statusCode, initiator: origin, originUrl: origin });
}

// MEDIA_SIZE (minimum-media-size): separate audio/video thresholds, only for a known total size; site exclusions.
tests.MEDIA_SIZE = async function test_MEDIA_SIZE() {
  const settings = { version: 1, minAudioBytes: 1_000_000, minVideoBytes: 5_000_000, panelExcludedSites: ['*.blocked.example'] };
  for (const [name, make] of bothHarnesses) {
    const h = make({ storage: { mediaSettings: settings } });
    const T = 7;
    headersEvent(h, T, 'http://m.test/small-audio.mp3', respHeaders('audio/mpeg', [cl(500_000)]));
    headersEvent(h, T, 'http://m.test/big-audio.mp3', respHeaders('audio/mpeg', [cl(2_000_000)]));
    headersEvent(h, T, 'http://m.test/small-video.mp4', respHeaders('video/mp4', [cl(2_000_000)]));
    headersEvent(h, T, 'http://m.test/big-video.mp4', respHeaders('video/mp4', [cl(6_000_000)]));
    headersEvent(h, T, 'http://m.test/unknown-video.mp4', respHeaders('video/mp4'));
    headersEvent(h, T, 'http://m.test/unknown-audio.mp3', respHeaders('audio/mpeg'));
    headersEvent(h, T, 'http://m.test/range-small.mp4', respHeaders('video/mp4', [cl(100), { name: 'Content-Range', value: 'bytes 0-99/4000' }]), 206);
    headersEvent(h, T, 'http://m.test/range-big.mp4', respHeaders('video/mp4', [cl(100), { name: 'Content-Range', value: 'bytes 0-99/9000000' }]), 206);
    headersEvent(h, T, 'http://m.test/partial-no-range.mp4', respHeaders('video/mp4', [cl(10)]), 206);
    headersEvent(h, T, 'http://m.test/gzip.mp4', respHeaders('video/mp4', [cl(10), { name: 'Content-Encoding', value: 'gzip' }]));
    headersEvent(h, T, 'http://m.test/tiny-manifest.m3u8', respHeaders('application/vnd.apple.mpegurl', [cl(120)]));
    headersEvent(h, T, 'http://m.test/blocked-site.mp4', respHeaders('video/mp4', [cl(9_000_000)]), 200, 'http://www.blocked.example');
    const listed = await mediaItems(h, T);
    assert.deepEqual(listed.sort(), [
      'big-audio.mp3', 'big-video.mp4', 'gzip.mp4', 'partial-no-range.mp4', 'range-big.mp4', 'tiny-manifest.m3u8', 'unknown-audio.mp3', 'unknown-video.mp4',
    ].sort(), `${name}: audio and video thresholds differ; unknown-size, partial-without-range, compressed and manifest items are kept; excluded site is hidden`);
    // A later full response for the unknown-size item reveals its size and the threshold then applies.
    headersEvent(h, T, 'http://m.test/unknown-video.mp4', respHeaders('video/mp4', [cl(1_000)]));
    assert.ok(!(await mediaItems(h, T)).includes('unknown-video.mp4'), `${name}: once the known size is below the minimum the item is hidden`);
    // Raising/lowering the setting applies without reload.
    const api = h.chrome || h.browser;
    await api.storage.local.set({ mediaSettings: { version: 1, minAudioBytes: 0, minVideoBytes: 0 } });
    const all = await mediaItems(h, T);
    assert.ok(all.includes('small-audio.mp3') && all.includes('small-video.mp4') && all.includes('blocked-site.mp4'), `${name}: zero thresholds and no exclusions show everything`);
    // Settings are normalized and versioned: garbage degrades to defaults instead of hiding media.
    await api.storage.local.set({ mediaSettings: { minAudioBytes: 'x', minVideoBytes: -5, overlayMode: 'weird' } });
    assert.equal((await mediaItems(h, T)).length, all.length, `${name}: invalid thresholds mean no threshold`);
  }
};

async function fireNotification(h, id, accept) {
  if ('browser' in h) {
    if (accept) h.handlers.notifClick(id); else h.handlers.notifClosed(id, true);
  } else h.handlers.notifButton(id, accept ? 0 : 1);
  for (let i = 0; i < 6; i++) await tick();
}

async function cancelCaptured(h, id, url, handoff = false) {
  await startDownload(h, { url, finalUrl: url, filename: 'f.bin', id, tabId: 1, startTime: String(id) });
  if (!handoff) h.handlers.downloadChanged({ id, error: { current: 'USER_CANCELED' } });
  for (let i = 0; i < 6; i++) await tick();
}

const exclusionWrites = h => h.calls.storageSets.filter(v => 'siteExclusions' in v);
const offers = h => h.calls.notifications.filter(n => String(n.id).startsWith('xdm-miscapture:'));

// MISCAPTURE (miscapture-exceptions): repeated cancels/re-downloads of one site offer, never silently apply, an exclusion.
tests.MISCAPTURE = async function test_MISCAPTURE() {
  const U = 'http://dl.example.test/a.bin';
  for (const [name, make] of bothHarnesses) {
    // Two user cancels of captured downloads of the same site -> one offer, nothing written yet.
    let h = make({ nativeReply: async () => ({ accepted: false }) });
    await cancelCaptured(h, 1, U);
    assert.equal(offers(h).length, 0, `${name}: a single cancel offers nothing`);
    await cancelCaptured(h, 2, 'http://other.example.test/b.bin');
    assert.equal(offers(h).length, 1, `${name}: second cancel on the same site (subdomains alike) offers an exclusion`);
    assert.equal(offers(h)[0].id, 'xdm-miscapture:example.test');
    assert.equal(exclusionWrites(h).length, 0, `${name}: the exclusion is never added silently`);
    const pending = await sendRuntime(h, { type: 'miscapture.pending' });
    assert.deepEqual(plain(pending.offers.map(o => o.pattern)), ['*.example.test'], `${name}: popup can list the pending offer`);
    assert.equal((await sendRuntime(h, { type: 'miscapture.accept', site: 'attacker.test' })).ok, false, `${name}: only a pending offer can be accepted`);
    assert.equal(exclusionWrites(h).length, 0);
    await fireNotification(h, 'xdm-miscapture:example.test', true);
    assert.ok(exclusionWrites(h)[0].siteExclusions.includes('*.example.test'), `${name}: user acceptance adds the site exclusion`);
    const before = h.calls.native;
    await startDownload(h, { url: 'http://dl.example.test/c.bin', filename: 'c.bin', id: 9, tabId: 1 });
    assert.equal(h.calls.native, before, `${name}: excluded site is no longer captured`);

    // Dismissing suppresses further offers for that site.
    h = make({ nativeReply: async () => ({ accepted: false }) });
    await cancelCaptured(h, 1, U); await cancelCaptured(h, 2, U);
    await fireNotification(h, 'xdm-miscapture:example.test', false);
    assert.equal(exclusionWrites(h).length, 0, `${name}: declining adds nothing`);
    await cancelCaptured(h, 3, U); await cancelCaptured(h, 4, U);
    assert.equal(offers(h).length, 1, `${name}: a declined site is not offered again`);

    // Our own cancel after a successful handoff is not a user mistake.
    h = make();
    for (const id of [1, 2, 3]) {
      await cancelCaptured(h, id, `http://own.example.test/${id}.bin`, true);
      h.handlers.downloadChanged({ id, error: { current: 'USER_CANCELED' } });
    }
    await tick();
    assert.equal(offers(h).length, 0, `${name}: the extension's own cancel of a handed-off download never counts`);

    // The browser re-downloading a handed-off URL twice counts as miscapture.
    h = make();
    await cancelCaptured(h, 1, U, true);
    await cancelCaptured(h, 2, U, true);
    assert.equal(offers(h).length, 0, `${name}: first re-download is only one hit`);
    await cancelCaptured(h, 3, U, true);
    assert.equal(offers(h).length, 1, `${name}: second browser re-download after handoff offers an exclusion`);

    // Hits outside the window do not count; the feature can be disabled; the threshold is configurable.
    const stale = { 'example.test': [{ time: Date.now() - 10 * 60_000, reason: 'cancelled' }] };
    h = make({ storage: { miscaptureHistory: stale }, nativeReply: async () => ({ accepted: false }) });
    await cancelCaptured(h, 1, U);
    assert.equal(offers(h).length, 0, `${name}: a hit older than the window is forgotten`);
    h = make({ storage: { miscaptureSettings: { version: 1, enabled: false } }, nativeReply: async () => ({ accepted: false }) });
    await cancelCaptured(h, 1, U); await cancelCaptured(h, 2, U); await cancelCaptured(h, 3, U);
    assert.equal(offers(h).length, 0, `${name}: disabled setting never offers`);
    h = make({ storage: { miscaptureSettings: { version: 1, threshold: 3 } }, nativeReply: async () => ({ accepted: false }) });
    await cancelCaptured(h, 1, U); await cancelCaptured(h, 2, U);
    assert.equal(offers(h).length, 0, `${name}: configured threshold of 3 needs three cancels`);
    await cancelCaptured(h, 3, U);
    assert.equal(offers(h).length, 1);
  }
};

const policyPage = hostname => ({ hostname, href: `http://${hostname}/watch` });
async function overlayHarness(source, isFf, mediaSettings, hostname = 'example.test') {
  const c = makeContentHarness(source, isFf, { storage: { mediaSettings }, location: policyPage(hostname) });
  await tick(); await tick();
  return c;
}

// PANEL_POLICY (player-panel-policy): full/mini mode, per-site disable, never an offer for DRM media.
tests.PANEL_POLICY = async function test_PANEL_POLICY() {
  for (const [source, isFf] of bothContents) {
    const label = isFf ? 'firefox' : 'chromium';
    // Mini mode: compact button, still labelled for assistive tech, anchored by its own width.
    let c = await overlayHarness(source, isFf, { overlayMode: 'mini' });
    let v = appendVideo(c, 'http://example.test/a.mp4', rect(50, 100, 640, 360));
    c.observedNodes[0].observer.triggerAdd(v);
    let panel = panelsIn(c)[0];
    assert.equal(btnOf(panel).textContent, 'XDM', `${label}: mini overlay shows a compact label`);
    assert.equal(btnOf(panel).getAttribute('aria-label'), 'Download this video');
    assert.equal(panel.style.left, `${50 + 640 - 44 - 4}px`, `${label}: mini overlay uses its smaller width`);
    // Live switch back to full mode.
    c.fireStorageChange({ mediaSettings: { newValue: { overlayMode: 'full' } } });
    assert.equal(btnOf(panel).textContent, 'Download this video', `${label}: mode change applies without reload`);
    assert.equal(panel.style.left, `${50 + 640 - 144 - 4}px`);

    // Per-site disable (exact and wildcard); other sites unaffected; live toggle.
    for (const pattern of ['example.test', '*.example.test']) {
      c = await overlayHarness(source, isFf, { overlayDisabledSites: [pattern] }, pattern === 'example.test' ? 'example.test' : 'www.example.test');
      v = appendVideo(c, 'http://example.test/a.mp4', rect(0, 0, 640, 360));
      c.observedNodes[0].observer.triggerAdd(v);
      assert.equal(panelsIn(c)[0].style.display, 'none', `${label}: overlay is disabled on ${pattern}`);
      c.fireStorageChange({ mediaSettings: { newValue: { overlayDisabledSites: [] } } });
      assert.equal(panelsIn(c)[0].style.display, 'block', `${label}: re-enabling shows it again`);
    }
    c = await overlayHarness(source, isFf, { overlayDisabledSites: ['example.test'] }, 'other.test');
    v = appendVideo(c, 'http://other.test/a.mp4', rect(0, 0, 640, 360));
    c.observedNodes[0].observer.triggerAdd(v);
    assert.equal(panelsIn(c)[0].style.display, 'block', `${label}: another site keeps its overlay`);
    c = await overlayHarness(source, isFf, { panelExcludedSites: ['example.test'] });
    v = appendVideo(c, 'http://example.test/a.mp4', rect(0, 0, 640, 360));
    c.observedNodes[0].observer.triggerAdd(v);
    assert.equal(panelsIn(c)[0].style.display, 'none', `${label}: a media-panel site exclusion also hides the overlay`);

    // DRM: encrypted event after playback starts removes the overlay and the catalog offer.
    c = await overlayHarness(source, isFf, {});
    v = appendVideo(c, 'http://example.test/drm.mp4', rect(0, 0, 640, 360));
    c.observedNodes[0].observer.triggerAdd(v);
    assert.equal(panelsIn(c).length, 1);
    c.sentMessages.length = 0;
    v.dispatch('encrypted');
    assert.equal(panelsIn(c).length, 0, `${label}: encrypted event removes the overlay`);
    assert.ok(c.sentMessages.some(m => m.type === 'page.protected' && m.url === 'http://example.test/drm.mp4'), `${label}: the background is told the player is protected`);
    c.observedNodes[0].observer.triggerAdd(v);
    c.triggerTimeout();
    assert.equal(panelsIn(c).length, 0, `${label}: re-inspection never brings the overlay back`);
    assert.equal(c.playerRegistry.isProtected(v), true);
    // MediaKeys already attached before detection: never an overlay, never a media report.
    c = await overlayHarness(source, isFf, {});
    v = appendVideo(c, 'http://example.test/keys.mp4', rect(0, 0, 640, 360));
    v.mediaKeys = {};
    c.sentMessages.length = 0;
    c.observedNodes[0].observer.triggerAdd(v);
    assert.equal(panelsIn(c).length, 0, `${label}: element with MediaKeys gets no overlay`);
    assert.ok(!c.sentMessages.some(m => m.type === 'page.media'), `${label}: protected media is not offered to the catalog`);
  }
  // Background: protected URLs leave the catalog and stay out.
  for (const [name, make] of bothHarnesses) {
    const h = make();
    headersEvent(h, 4, 'http://m.test/drm.mp4', respHeaders('video/mp4', [cl(9_000_000)]));
    assert.deepEqual(await mediaItems(h, 4), ['drm.mp4']);
    await sendRuntime(h, { type: 'page.protected', url: 'http://m.test/drm.mp4' }, { tab: { id: 4 } });
    assert.deepEqual(await mediaItems(h, 4), [], `${name}: protected media is removed from the catalog`);
    headersEvent(h, 4, 'http://m.test/drm.mp4', respHeaders('video/mp4', [cl(9_000_000)]));
    assert.deepEqual(await mediaItems(h, 4), [], `${name}: and not re-added by later network sightings`);
  }
};

// OVERLAY_SHADOW (B48 gap): closed shadow root, important inline host styles, keyboard focus and Escape.
tests.OVERLAY_SHADOW = async function test_OVERLAY_SHADOW() {
  for (const [source, isFf] of bothContents) {
    const label = isFf ? 'firefox' : 'chromium';
    const c = await overlayHarness(source, isFf, {});
    const v = appendVideo(c, 'http://example.test/k.mp4', rect(0, 0, 640, 360));
    c.observedNodes[0].observer.triggerAdd(v);
    const panel = panelsIn(c)[0];
    assert.equal(panel.shadow.mode, 'closed', `${label}: closed shadow root`);
    assert.ok(btnOf(panel), `${label}: button is inside the shadow root`);
    assert.equal(panel.important?.display?.[1], 'important', `${label}: host display is an !important inline declaration page CSS cannot override`);
    assert.equal(panel.important?.position?.[0], 'fixed');
    assert.ok(panel.shadow.children.some(ch => ch.tagName === 'STYLE'), `${label}: overlay styles are scoped inside the shadow root`);
    // Keyboard: keys inside the overlay never reach page shortcuts.
    const space = panel.shadow.dispatch('keydown', { key: ' ' });
    assert.equal(space.stopped, true, `${label}: key events stop at the overlay`);
    assert.equal(panel.style.display, 'block');
    // Escape dismisses and gives focus back to the player.
    const esc = panel.shadow.dispatch('keydown', { key: 'Escape' });
    assert.equal(esc.stopped, true);
    assert.equal(panel.style.display, 'none', `${label}: Escape hides the overlay`);
    assert.equal(v.focused, true, `${label}: Escape returns focus to the player`);
    c.resizeObservers[0].trigger();
    c.intersectionObservers[0].trigger(v, true);
    assert.equal(panel.style.display, 'none', `${label}: layout changes do not resurrect a dismissed overlay`);
    // A new source on the same player brings it back.
    v.src = v.currentSrc = 'http://example.test/next.mp4';
    c.observedNodes[0].observer.triggerAdd(v);
    assert.equal(panelsIn(c)[0].style.display, 'block', `${label}: a new source shows the overlay again`);
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


// ---- Video panel: YouTube adapter core, panel menu and background hand-off (synthetic fixtures only) ----
const FAR = Math.floor(Date.now() / 1000) + 6 * 3600;
const gv = (itag, extra = '') => `https://rr1---sn-synthetic.googlevideo.com/videoplayback?expire=${FAR}&itag=${itag}&sig=SYNTHETIC${extra}`;
const fmt = (itag, mimeType, extra = {}) => ({ itag, mimeType, url: gv(itag), ...extra });
function playerFixture({ withUrls = true, live = false, drm = false, status = 'OK', title = 'A: Test/Video?' } = {}) {
  const u = f => withUrls ? f : { ...f, url: undefined };
  const adaptive = [
    u(fmt(313, 'video/webm; codecs="vp9"', { width: 3840, height: 2160, fps: 30, bitrate: 20e6, contentLength: '900000000' })),
    u(fmt(137, 'video/mp4; codecs="avc1.640028"', { width: 1920, height: 1080, fps: 30, bitrate: 4e6, contentLength: '250000000' })),
    u(fmt(248, 'video/webm; codecs="vp9"', { width: 1920, height: 1080, fps: 30, bitrate: 2.5e6, contentLength: '150000000' })),
    u(fmt(136, 'video/mp4; codecs="avc1.4d401f"', { width: 1280, height: 720, fps: 30, bitrate: 2e6, contentLength: '90000000' })),
    u(fmt(298, 'video/mp4; codecs="avc1.4d4020"', { width: 1280, height: 720, fps: 60, bitrate: 3e6, contentLength: '120000000' })),
    u(fmt(140, 'audio/mp4; codecs="mp4a.40.2"', { bitrate: 130000, averageBitrate: 128000, audioChannels: 2, contentLength: '10000000', audioQuality: 'AUDIO_QUALITY_MEDIUM' })),
    u(fmt(251, 'audio/webm; codecs="opus"', { bitrate: 140000, averageBitrate: 130000, audioChannels: 2, contentLength: '9000000' })),
    u({ ...fmt(140, 'audio/mp4; codecs="mp4a.40.2"', { bitrate: 130000, audioChannels: 2, isDrc: true }), itag: 141 }),
    u(fmt(256, 'audio/mp4; codecs="mp4a.40.5"', { bitrate: 380000, audioChannels: 6, contentLength: '30000000' })),
    { itag: 399, mimeType: 'video/mp4; codecs="av01.0.08M.08"', width: 1920, height: 1080, fps: 30, signatureCipher: 's=x&url=y' },
  ];
  const formats = [u(fmt(18, 'video/mp4; codecs="avc1.42001E, mp4a.40.2"', { width: 640, height: 360, fps: 30, bitrate: 600000 }))];
  if (drm) adaptive[0].drmFamilies = ['WIDEVINE'];
  return {
    playabilityStatus: { status, reason: status === 'OK' ? undefined : 'Sign in to confirm' },
    videoDetails: { videoId: 'aaaaaaaaaaa', title, author: 'Someone', lengthSeconds: '600', isLive: live },
    streamingData: { formats, adaptiveFormats: adaptive },
  };
}

tests.VIDEO_CORE = async function test_VIDEO_CORE() {
  const ctx = { URL, Date, Math };
  vm.createContext(ctx);
  vm.runInContext(videoCoreSource, ctx);
  const V = ctx.XdmVideo;
  assert.equal(V.ytVideoId('https://www.youtube.com/watch?v=aaaaaaaaaaa&t=3'), 'aaaaaaaaaaa');
  assert.equal(V.ytVideoId('https://youtu.be/bbbbbbbbbbb?si=x'), 'bbbbbbbbbbb');
  assert.equal(V.ytVideoId('https://www.youtube.com/shorts/ccccccccccc'), 'ccccccccccc');
  assert.equal(V.ytVideoId('https://www.youtube.com/embed/ddddddddddd'), 'ddddddddddd');
  assert.equal(V.ytVideoId('https://m.youtube.com/watch?v=eeeeeeeeeee'), 'eeeeeeeeeee');
  for (const bad of ['https://www.youtube.com/', 'https://www.youtube.com/watch?v=short', 'https://evil.test/watch?v=aaaaaaaaaaa', 'https://www.youtube.com.evil.test/watch?v=aaaaaaaaaaa', 'nope']) assert.equal(V.ytVideoId(bad), null, bad);

  const model = V.normalizePlayerResponse(plain(playerFixture()));
  assert.equal(model.playable, true);
  assert.equal(model.title, 'A: Test/Video?');
  const menu = V.buildMenu(model);
  const labels = menu.map(o => o.label);
  // Best first; MP4 (H.264 + AAC) and WEBM (VP9 + Opus) per height; 60 fps named; audio-only last.
  assert.deepEqual(plain(labels.slice(0, 6)), ['WEBM 2160p', 'MP4 1080p', 'WEBM 1080p', 'MP4 720p60', 'MP4 720p', 'MP4 360p']);
  assert.ok(labels.includes('M4A 128 kbps') && labels.includes('WEBM 130 kbps'), labels.join(','));
  assert.ok(!labels.some(l => /AV1|399/.test(l)), 'a ciphered format without an address is never offered');
  const mp4 = menu.find(o => o.label === 'MP4 1080p');
  assert.equal(mp4.type, 'adaptive');
  assert.equal(mp4.container, 'mp4');
  assert.deepEqual([mp4.video.itag, mp4.audio.itag], [137, 140], 'MP4 pairs H.264 with AAC');
  assert.equal(mp4.size, 260000000);
  assert.equal(mp4.detail, '260 MB');
  const web = menu.find(o => o.label === 'WEBM 2160p');
  assert.deepEqual([web.video.itag, web.audio.itag, web.container], [313, 251, 'webm'], 'WEBM pairs VP9 with Opus');
  const muxed = menu.find(o => o.label === 'MP4 360p');
  assert.equal(muxed.type, 'direct');
  assert.equal(muxed.audio, null);
  assert.ok(!menu.some(o => o.audio?.itag === 141 || o.audio?.itag === 256), 'DRC and surround variants are not paired');
  assert.equal(menu.find(o => o.audioOnly && o.ext === 'm4a').filename, 'A Test Video [128k].m4a');
  assert.equal(mp4.filename, 'A Test Video [1080p].mp4', 'title is made a safe file name');
  assert.ok(menu.every(o => V.validateChoice({ ...o, title: 't' }).ok), 'every offered choice passes validation');

  // SABR-only response (no addresses): nothing to offer until addresses are resolved and merged by itag.
  const sabr = V.normalizePlayerResponse(plain(playerFixture({ withUrls: false })));
  assert.equal(sabr.playable, true);
  assert.equal(sabr.hasUrls, false);
  assert.deepEqual(plain(V.buildMenu(sabr)), []);
  const resolved = V.normalizePlayerResponse(plain(playerFixture()));
  const merged = V.mergeUrls(sabr, resolved.formats);
  assert.ok(merged.hasUrls && V.buildMenu(merged).length >= 8, 'addresses resolved by itag complete the menu');
  const only = V.mergeUrls(sabr, [{ itag: 9999, url: gv(9999), kind: 'video' }], { addUnknown: false });
  assert.ok(!only.formats.some(f => f.itag === 9999), 'captured addresses only fill itags the page knows');

  // Never offered: live, DRM, blocked videos; expired addresses.
  for (const odd of [{ live: true }, { drm: true }, { status: 'LOGIN_REQUIRED' }]) {
    const m = V.normalizePlayerResponse(plain(playerFixture(odd)));
    assert.equal(m.playable, false, JSON.stringify(odd));
    assert.deepEqual(plain(V.buildMenu(m)), []);
  }
  assert.deepEqual(plain(V.buildMenu(model, (FAR + 60) * 1000)), [], 'expired addresses are not offered');

  // The background accepts only googlevideo https addresses and sane names.
  const good = { type: 'direct', video: { url: gv(18) }, audio: null, filename: 'a.mp4' };
  assert.equal(V.validateChoice(good).ok, true);
  for (const bad of [
    { ...good, video: { url: 'https://evil.test/videoplayback' } },
    { ...good, video: { url: 'http://rr1---x.googlevideo.com/videoplayback' } },
    { ...good, video: { url: 'https://googlevideo.com.evil.test/x' } },
    { ...good, filename: '../x.mp4' },
    { ...good, type: 'adaptive', container: 'mp4' },
    { type: 'adaptive', container: 'avi', video: { url: gv(1) }, audio: { url: gv(2) }, filename: 'a.avi' },
  ]) assert.equal(V.validateChoice(bad).ok, false, JSON.stringify(bad));

  // Chunk parameters are stripped so a captured player request addresses the whole stream.
  const whole = V.wholeStreamUrl(gv(22, '&range=0-9999&rn=3&rbuf=100&ump=1'));
  assert.ok(!/[?&](range|rn|rbuf|ump)=/.test(whole) && /itag=22/.test(whole), whole);
  assert.equal(V.wholeStreamUrl('https://example.test/videoplayback?itag=1'), null);
  assert.equal(V.safeName('CON'), '_CON');
  assert.equal(V.safeName('   '), 'video');
  assert.ok(new TextEncoder().encode(V.safeName('é'.repeat(300))).length <= 240);
};

async function youtubeContent(source, isFf, replyOverride) {
  const location = { href: 'https://www.youtube.com/watch?v=aaaaaaaaaaa', hostname: 'www.youtube.com', origin: 'https://www.youtube.com' };
  const posted = [];
  const c = makeContentHarness(source, isFf, { location, reply: async msg => {
    if (replyOverride) { const r = await replyOverride(msg); if (r !== undefined) return r; }
    if (msg.type === 'video.captured') return { formats: [] };
    if (msg.type === 'video.verify') return { ok: true, results: Object.fromEntries(msg.items.map(i => [i.url, true])) };
    return { ok: true };
  } });
  c.window.postMessage = (data) => posted.push(data);
  return { c, posted, send: data => c.winListeners.message({ data: { xdmVideo: 1, ...data }, source: c.window }) };
}

tests.VIDEO_PANEL = async function test_VIDEO_PANEL() {
  for (const [source, isFf] of bothContents) {
    const label = isFf ? 'firefox' : 'chromium';
    const h = await youtubeContent(source, isFf);
    const { c, posted, send } = h;
    const v = appendVideo(c, 'blob:https://www.youtube.com/1234', rect(10, 80, 800, 450));
    v.src = v.currentSrc = 'blob:https://www.youtube.com/1234';
    c.observedNodes[0].observer.triggerAdd(v);
    assert.equal(panelsIn(c).length, 0, `${label}: no panel before the page's player response is known`);
    // The page script reports the page's own player response (no addresses: SABR).
    send({ type: 'player', response: plain(playerFixture({ withUrls: false })) });
    c.triggerTimeout?.();
    c.observedNodes[0].observer.triggerAdd(v);
    const panel = panelsIn(c)[0];
    assert.ok(panel, `${label}: blob: video gets a panel identified by the page video, not a URL`);
    assert.equal(btnOf(panel).textContent, 'Download this video');
    assert.equal(c.playerRegistry.get([...c.playerRegistry.entries.keys()][0]).kind, 'page-video');
    // Opening the menu asks the page script to resolve addresses; the menu lists qualities.
    btnOf(panel).onclick({ stopPropagation() {}, preventDefault() {} });
    await tick();
    const request = posted.find(m => m.type === 'resolve');
    assert.ok(request && request.videoId === 'aaaaaaaaaaa', `${label}: resolution is requested from the page script`);
    send({ type: 'resolved', requestId: request.requestId, ok: true, response: plain(playerFixture()) });
    await tick(); await tick(); await tick();
    const menu = panel.shadow.children.find(ch => ch.className === 'menu');
    assert.ok(menu, `${label}: menu opened inside the shadow root`);
    const rows = menu.children.filter(ch => ch.className === 'opt');
    assert.equal(rows[0].children[0].textContent, 'WEBM 2160p');
    assert.ok(rows.length >= 8, `${label}: ${rows.length} rows`);
    // Choosing one hands the full selection to the background script.
    c.sentMessages.length = 0;
    rows[1].onclick({ stopPropagation() {}, preventDefault() {} });
    await tick();
    const chosen = c.sentMessages.find(m => m.type === 'video.download');
    assert.ok(chosen, `${label}: choice is sent`);
    assert.equal(chosen.choice.label, 'MP4 1080p');
    assert.equal(chosen.choice.site, 'youtube');
    assert.equal(chosen.choice.title, 'A: Test/Video?');
    assert.equal(chosen.choice.type, 'adaptive');
    // Escape closes the menu first, then the panel.
    panel.shadow.dispatch('keydown', { key: 'Escape' });
    assert.ok(!panel.shadow.children.some(ch => ch.className === 'menu'), `${label}: Escape closes the menu`);
    assert.equal(panel.style.display, 'block');
    // Single-page navigation to another video: stale model never shows for the new page.
    c.location = undefined;
  }
};

tests.VIDEO_PANEL_DRM = async function test_VIDEO_PANEL_DRM() {
  for (const [source, isFf] of bothContents) {
    const h = await youtubeContent(source, isFf);
    const { c, send } = h;
    const v = appendVideo(c, 'blob:https://www.youtube.com/9', rect(0, 0, 800, 450));
    send({ type: 'player', response: plain(playerFixture({ drm: true, withUrls: false })) });
    c.observedNodes[0].observer.triggerAdd(v);
    assert.equal(panelsIn(c).length, 0, 'DRM-protected video never gets a panel');
    const live = await youtubeContent(source, isFf);
    const lv = appendVideo(live.c, 'blob:https://www.youtube.com/8', rect(0, 0, 800, 450));
    live.send({ type: 'player', response: plain(playerFixture({ live: true, withUrls: false })) });
    live.c.observedNodes[0].observer.triggerAdd(lv);
    assert.equal(panelsIn(live.c).length, 0, 'live streams get no panel');
    // A response for a different video than the page shows is ignored.
    const other = await youtubeContent(source, isFf);
    const ov = appendVideo(other.c, 'blob:https://www.youtube.com/7', rect(0, 0, 800, 450));
    const r = playerFixture({ withUrls: false });
    r.videoDetails.videoId = 'zzzzzzzzzzz';
    other.send({ type: 'player', response: plain(r) });
    other.c.observedNodes[0].observer.triggerAdd(ov);
    assert.equal(panelsIn(other.c).length, 0, 'stale response of another video is ignored');
  }
};

tests.VIDEO_PANEL_GENERIC = async function test_VIDEO_PANEL_GENERIC() {
  for (const [source, isFf] of bothContents) {
    // A blob: player on an ordinary site is offered the manifest the background script captured.
    let asked = 0;
    const c = makeContentHarness(source, isFf, { location: { href: 'https://news.example/story', hostname: 'news.example', origin: 'https://news.example' }, reply: async msg => {
      if (msg.type === 'media.list') { asked++; return { items: [{ url: 'https://cdn.example/v/master.m3u8', kind: 'hls', title: 'Story clip' }] }; }
      if (msg.type === 'video.options') return { ok: true, options: [{ label: 'MP4 720p', detail: '2000 kb/s', choice: { site: 'generic', type: 'stream', url: msg.playerUrl, videoIndex: 1, audioIndex: 3, filename: 'Story [720p].mp4' } }] };
      return { ok: true };
    } });
    const v = appendVideo(c, '', rect(0, 0, 640, 360));
    v.src = v.currentSrc = 'blob:https://news.example/abc';
    c.observedNodes[0].observer.triggerAdd(v);
    await tick(); await tick();
    assert.ok(asked >= 1, 'blob: player asks the background script for captured streams');
    const panel = panelsIn(c)[0];
    assert.ok(panel, 'panel shows for a captured HLS stream');
    btnOf(panel).onclick({ stopPropagation() {}, preventDefault() {} });
    await tick(); await tick();
    const options = panel.shadow.children.find(ch => ch.className === 'menu').children.filter(ch => ch.className === 'opt');
    assert.equal(options.length, 1);
    assert.equal(c.sentMessages.find(m => m.type === 'video.options').playerUrl, 'https://cdn.example/v/master.m3u8');
    c.sentMessages.length = 0;
    options[0].onclick({ stopPropagation() {}, preventDefault() {} });
    await tick();
    const sent = c.sentMessages.find(m => m.type === 'video.download');
    assert.deepEqual(plain(sent.choice), { site: 'generic', type: 'stream', url: 'https://cdn.example/v/master.m3u8', videoIndex: 1, audioIndex: 3, filename: 'Story [720p].mp4' });
    // Not shown for a DRM player: an element with mediaKeys is never registered.
    const protectedPlayer = appendVideo(c, 'https://news.example/p.mp4', rect(0, 400, 640, 360));
    protectedPlayer.mediaKeys = {};
    c.observedNodes[0].observer.triggerAdd(protectedPlayer);
    assert.equal(panelsIn(c).length, 1, 'DRM player gets no panel');
  }
};

async function callMessage(h, message, sender = {}) {
  if (h.handlers.message.length === 3) return await new Promise(resolve => { h.handlers.message(message, sender, resolve); });
  return await h.handlers.message(message, sender);
}

tests.VIDEO_BACKGROUND = async function test_VIDEO_BACKGROUND() {
  const choice = (type, extra = {}) => ({
    site: 'youtube', type, title: 'A Test', label: 'x', filename: 'A Test [1080p].mp4', container: 'mp4', size: 123,
    video: { itag: 137, url: gv(137), mime: 'video/mp4' }, audio: type === 'adaptive' ? { itag: 140, url: gv(140), mime: 'audio/mp4' } : null, ...extra,
  });
  for (const [name, make] of bothHarnesses) {
    // The host shows the choice in the XDM window.
    let h = make({ nativeReply: async m => ({ version: 1, type: 'download.result', accepted: true, offered: true, requestId: m.requestId }) });
    let reply = await callMessage(h, { type: 'video.download', choice: choice('adaptive'), pageUrl: 'https://www.youtube.com/watch?v=aaaaaaaaaaa' }, { tab: { id: 1 } });
    assert.equal(reply.accepted, true, `${name}: ${JSON.stringify(reply)}`);
    const offer = h.calls.messages[0];
    assert.equal(offer.type, 'video.offer');
    assert.equal(offer.payload.mode, 'adaptive');
    assert.equal(offer.payload.url, gv(137));
    assert.equal(offer.payload.audioUrl, gv(140));
    assert.equal(offer.payload.container, 'mp4');
    assert.equal(offer.payload.filenameHint, 'A Test [1080p].mp4');
    assert.equal(offer.payload.requestContext.referrer, 'https://www.youtube.com/');
    assert.ok(offer.payload.requestContext.headers.some(x => x.name === 'User-Agent'));
    assert.equal(JSON.stringify(offer).includes('cookie'), false, 'no cookies travel with an adaptive offer');

    // An older host without video.offer: the choice is queued directly through media.submit.
    h = make({ nativeReply: async m => m.type === 'video.offer' ? { version: 1, type: 'error', reason: 'unsupported message type' } : { version: 1, type: 'media.result', accepted: true, taskId: 4 } });
    reply = await callMessage(h, { type: 'video.download', choice: choice('adaptive') }, { tab: { id: 1 } });
    assert.equal(reply.accepted, true);
    assert.deepEqual(h.calls.messages.map(m => m.type), ['video.offer', 'media.submit']);
    assert.equal(h.calls.messages[1].payload.mode, 'adaptive');

    // A single stream (muxed or audio-only) falls back to the ordinary handoff.
    h = make({ nativeReply: async m => m.type === 'video.offer' ? { version: 1, type: 'error', reason: 'unsupported message type' } : { version: 1, type: 'download.result', accepted: true } });
    reply = await callMessage(h, { type: 'video.download', choice: choice('direct', { video: null, audio: { itag: 140, url: gv(140), mime: 'audio/mp4' }, filename: 'A Test [128k].m4a' }) }, { tab: { id: 1 } });
    assert.equal(reply.accepted, true);
    assert.deepEqual(h.calls.messages.map(m => m.type), ['video.offer', 'download.submit']);
    assert.equal(h.calls.messages[1].payload.filenameHint, 'A Test [128k].m4a');

    // Rejected without contacting the host.
    h = make();
    for (const [what, message, sender] of [
      ['foreign host', { choice: choice('direct', { video: { url: 'https://evil.test/x', mime: 'video/mp4' } }) }, { tab: { id: 1 } }],
      ['expired address', { choice: choice('direct', { video: { url: `https://rr1---x.googlevideo.com/videoplayback?expire=${Math.floor(Date.now() / 1000) - 5}&itag=18`, mime: 'video/mp4' } }) }, { tab: { id: 1 } }],
      ['private window', { choice: choice('adaptive') }, { tab: { id: 1, incognito: true } }],
      ['bad container', { choice: choice('adaptive', { container: 'avi' }) }, { tab: { id: 1 } }],
    ]) {
      reply = await callMessage(h, { type: 'video.download', ...message }, sender);
      assert.equal(reply.accepted, false, `${name}: ${what}`);
    }
    assert.equal(h.calls.native, 0, `${name}: rejected choices never reach the host`);
    if (name === 'chromium') {
      const foreign = await callMessage(h, { type: 'video.download', choice: choice('adaptive') }, { id: 'someone-else', tab: { id: 1 } });
      assert.equal(foreign.accepted, false);
    }

    // Plain GET videoplayback requests of the page's player are remembered per tab, whole-stream.
    h = make();
    h.handlers.request({ url: gv(22, '&mime=video%2Fmp4&clen=5000&range=0-999&rn=2&rbuf=1'), method: 'GET', tabId: 7, requestHeaders: [] });
    h.handlers.request({ url: gv(23, '&mime=video%2Fmp4'), method: 'POST', tabId: 7, requestHeaders: [] });
    const captured = await callMessage(h, { type: 'video.captured' }, { tab: { id: 7 } });
    assert.equal(captured.formats.length, 1);
    assert.equal(captured.formats[0].itag, 22);
    assert.ok(!/[?&](range|rn|rbuf)=/.test(captured.formats[0].url));
    assert.equal(captured.formats[0].contentLength, '5000');
    assert.equal((await callMessage(h, { type: 'video.captured' }, { tab: { id: 8 } })).formats.length, 0);
  }

  // Generic sites: direct files one entry each, HLS variants one entry per resolution.
  for (const [name, make] of bothHarnesses) {
    const h = make({ nativeReply: async m => {
      if (m.type === 'media.inspect') return { ok: true, tracks: [
        { index: 0, kind: 'video', codec: 'h264', width: 1280, height: 720, bitrate: 2000000 },
        { index: 1, kind: 'video', codec: 'h264', width: 1280, height: 720, bitrate: 1500000 },
        { index: 2, kind: 'video', codec: 'h264', width: 640, height: 360, bitrate: 700000 },
        { index: 3, kind: 'audio', codec: 'aac', bitrate: 128000 }] };
      return { version: 1, type: m.type === 'media.submit' ? 'media.result' : 'download.result', accepted: true, offered: true };
    } });
    h.handlers.headers({ tabId: 3, url: 'https://cdn.example/clip.mp4', responseHeaders: [{ name: 'Content-Type', value: 'video/mp4' }, { name: 'Content-Length', value: '5000000' }] });
    h.handlers.headers({ tabId: 3, url: 'https://cdn.example/low.webm', responseHeaders: [{ name: 'Content-Type', value: 'video/webm' }, { name: 'Content-Length', value: '900000' }] });
    h.handlers.headers({ tabId: 3, url: 'https://cdn.example/master.m3u8', responseHeaders: [{ name: 'Content-Type', value: 'application/vnd.apple.mpegurl' }] });
    const options = (await callMessage(h, { type: 'video.options', title: 'Story: clip', playerUrl: 'blob:x' }, { tab: { id: 3 } }));
    assert.equal(options.ok, true, `${name}: ${JSON.stringify(options)}`);
    assert.deepEqual(plain(options.options.map(o => o.label)), ['MP4', 'WEBM', 'MP4 720p', 'MP4 360p'], 'direct files first (largest first), then one entry per resolution');
    assert.equal(options.options[0].detail, '5.0 MB');
    const stream = options.options[2].choice;
    assert.deepEqual([stream.videoIndex, stream.audioIndex], [0, 3], 'the best bitrate of a resolution wins, paired with the audio rendition');
    assert.equal(stream.filename, 'Story clip [720p].mp4');
    const sent = await callMessage(h, { type: 'video.download', choice: stream, pageUrl: 'https://cdn.example/page' }, { tab: { id: 3 } });
    assert.equal(sent.accepted, true);
    const last = h.calls.messages.at(-1);
    assert.equal(last.type, 'video.offer');
    assert.equal(last.payload.mode, 'video');
    assert.equal(last.payload.videoIndex, 0);
    const direct = await callMessage(h, { type: 'video.download', choice: options.options[0].choice, pageUrl: 'https://cdn.example/page' }, { tab: { id: 3 } });
    assert.equal(direct.accepted, true);
    assert.equal(h.calls.messages.at(-1).type, 'video.offer');
    assert.equal(h.calls.messages.at(-1).payload.filenameHint, 'Story clip.mp4');
    const forged = await callMessage(h, { type: 'video.download', choice: { site: 'generic', type: 'direct', url: 'file:///etc/passwd', filename: 'x.mp4' } }, { tab: { id: 3 } });
    assert.equal(forged.accepted, false);
  }
};


tests.VIDEO_VERIFY = async function test_VIDEO_VERIFY() {
  // YouTube cuts addresses obtained without its integrity proof after about a minute of media. Formats that are
  // not served to the end are never offered; the muxed stream that is stays, with a note about the rest.
  for (const [source, isFf] of bothContents) {
    const label = isFf ? 'firefox' : 'chromium';
    const probes = [];
    const h = await youtubeContent(source, isFf, async msg => {
      if (msg.type !== 'video.verify') return undefined;
      probes.push(...msg.items);
      return { ok: true, results: Object.fromEntries(msg.items.map(i => [i.url, /itag=18(&|$)/.test(i.url)])) };
    });
    const { c, posted, send } = h;
    const v = appendVideo(c, 'blob:https://www.youtube.com/5', rect(10, 80, 800, 450));
    send({ type: 'player', response: plain(playerFixture({ withUrls: false })) });
    c.observedNodes[0].observer.triggerAdd(v);
    const panel = panelsIn(c)[0];
    btnOf(panel).onclick({ stopPropagation() {}, preventDefault() {} });
    await tick();
    send({ type: 'resolved', requestId: posted.find(m => m.type === 'resolve').requestId, ok: true, response: plain(playerFixture()) });
    await tick(); await tick(); await tick();
    const menu = panel.shadow.children.find(ch => ch.className === 'menu');
    const rows = menu.children.filter(ch => ch.className === 'opt').map(r => r.children[0].textContent);
    assert.deepEqual(rows, ['MP4 360p'], `${label}: only the stream served to the end is offered`);
    assert.ok(menu.children.some(ch => ch.className === 'note' && /Higher qualities are hidden/.test(ch.textContent)), `${label}: the user is told why`);
    assert.ok(probes.every(p => Number.isInteger(p.at) && p.at >= 0) && probes.some(p => p.at > 100000000), `${label}: probes address deep into the file`);
    // When nothing verifies, the menu says so instead of listing dead entries.
    const none = await youtubeContent(source, isFf, async msg => msg.type === 'video.verify' ? { ok: true, results: Object.fromEntries(msg.items.map(i => [i.url, false])) } : undefined);
    const v2 = appendVideo(none.c, 'blob:https://www.youtube.com/6', rect(10, 80, 800, 450));
    none.send({ type: 'player', response: plain(playerFixture({ withUrls: false })) });
    none.c.observedNodes[0].observer.triggerAdd(v2);
    const p2 = panelsIn(none.c)[0];
    btnOf(p2).onclick({ stopPropagation() {}, preventDefault() {} });
    await tick();
    none.send({ type: 'resolved', requestId: none.posted.find(m => m.type === 'resolve').requestId, ok: true, response: plain(playerFixture()) });
    await tick(); await tick(); await tick();
    const m2 = p2.shadow.children.find(ch => ch.className === 'menu');
    assert.equal(m2.children.filter(ch => ch.className === 'opt').length, 0);
    assert.ok(m2.children.some(ch => /integrity check/.test(ch.textContent)));
  }
  // Background: one-byte ranged probes; 206 means served, 403 means cut, anything else is unknown.
  for (const [name, make] of bothHarnesses) {
    const h = make();
    const seen = [];
    h.context.fetch = async (url, init) => { seen.push([url, init.headers.Range]); return { status: /itag=137/.test(url) ? 403 : (/itag=140/.test(url) ? 416 : 206) }; };
    h.context.AbortController = AbortController;
    const items = [137, 136, 140].map(itag => ({ url: gv(itag), at: 123456 }));
    const reply = await callMessage(h, { type: 'video.verify', items: [...items, { url: 'https://evil.test/x', at: 1 }] }, { tab: { id: 1 } });
    assert.equal(reply.ok, true, name);
    assert.deepEqual(plain(reply.results), { [gv(137)]: false, [gv(136)]: true, [gv(140)]: false }, `${name}: only a 206 counts as served (416 too), foreign hosts are never fetched`);
    assert.deepEqual(plain(seen.map(x => x[1])), ['bytes=123456-123456', 'bytes=123456-123456', 'bytes=123456-123456']);
  }
};

tests.VIDEO_PAGE_SCRIPT = async function test_VIDEO_PAGE_SCRIPT() {
  // The page-world script is shipped as an identical copy in both extensions and does nothing off YouTube.
  const a = fs.readFileSync(new URL('page_video.js', chromiumDir), 'utf8');
  assert.equal(a, fs.readFileSync(new URL('page_video.js', firefoxDir), 'utf8'));
  assert.equal(videoCoreSource, fs.readFileSync(new URL('video_core.js', firefoxDir), 'utf8'));
  const other = { window: { fetch: () => {}, addEventListener() {}, postMessage() { throw new Error('must stay silent'); } }, location: { hostname: 'example.test' }, document: {} };
  other.window.window = other.window;
  vm.createContext(other);
  vm.runInContext(a, other);
  assert.equal(other.window.__xdmVideoHook, undefined, 'inactive on other sites');
  // On YouTube it answers a resolve request with the first client profile that yields addresses.
  const posted = [];
  const calls = [];
  const listeners = {};
  const win = {
    ytcfg: { get: key => ({ INNERTUBE_API_KEY: 'KEY', VISITOR_DATA: 'VD' })[key] },
    addEventListener: (type, fn) => { listeners[type] = fn; },
    postMessage: data => posted.push(data),
    fetch: async (url, init) => {
      const body = JSON.parse(init.body);
      calls.push(body.context.client.clientName);
      const blocked = body.context.client.clientName === 'ANDROID';
      return { json: async () => blocked ? { playabilityStatus: { status: 'LOGIN_REQUIRED', reason: 'Sign in' } } : { ...playerFixture(), playabilityStatus: { status: 'OK' } } };
    },
  };
  win.window = win;
  const yt = { window: win, location: { hostname: 'www.youtube.com', origin: 'https://www.youtube.com', href: 'https://www.youtube.com/watch?v=aaaaaaaaaaa' },
    document: { addEventListener() {}, getElementById: () => null, querySelector: () => null }, XMLHttpRequest: function () {}, setTimeout, encodeURIComponent, JSON };
  yt.XMLHttpRequest.prototype.open = function () {};
  vm.createContext(yt);
  vm.runInContext(a, yt);
  listeners.message({ source: win, data: { xdmVideo: 1, type: 'resolve', videoId: 'aaaaaaaaaaa', requestId: 'r1' } });
  await new Promise(r => setTimeout(r, 20));
  const reply = posted.find(m => m.type === 'resolved');
  assert.ok(reply?.ok, JSON.stringify(reply));
  assert.deepEqual(calls, ['ANDROID', 'IOS'], 'falls through to the next client profile when one is refused');
  assert.equal(reply.response.xdmClient, 'IOS');
  assert.ok(reply.response.streamingData.adaptiveFormats.length > 5);
  listeners.message({ source: win, data: { xdmVideo: 1, type: 'resolve', videoId: '../bad', requestId: 'r2' } });
  await new Promise(r => setTimeout(r, 5));
  assert.equal(posted.find(m => m.requestId === 'r2')?.ok, false, 'a malformed video id is refused');
};

// Test Runner
async function run() {
  console.log('Running IDM Browser Behavior Contract Tests (B01-B62 plus media-policy checks)...\n');
  const only = process.argv.slice(2);
  const baseIds = Array.from({ length: 62 }, (_, i) => `B${String(i + 1).padStart(2, '0')}`);
  const extraIds = ['MEDIA_SIZE', 'MISCAPTURE', 'PANEL_POLICY', 'OVERLAY_SHADOW', 'VIDEO_CORE', 'VIDEO_PANEL', 'VIDEO_PANEL_DRM', 'VIDEO_PANEL_GENERIC', 'VIDEO_BACKGROUND', 'VIDEO_VERIFY', 'VIDEO_PAGE_SCRIPT'];
  const allIds = [...baseIds, ...extraIds].filter(id => only.length === 0 || only.includes(id));
  const unknown = only.filter(id => ![...baseIds, ...extraIds].includes(id));
  if (unknown.length) { console.error(`unknown test id(s): ${unknown.join(', ')}`); process.exit(2); }

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
