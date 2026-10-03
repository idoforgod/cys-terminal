// diag/cdp-update.mjs
// Drives the Tauri/WebView2 desktop app over the Chrome DevTools Protocol (WebView2 remote debugging port)
// and calls the same IPC commands the update button calls: check_update, then install_update {force:true}.
// Node >= 18 (global fetch). No external packages. The WebSocket client is a small built-in one (MiniWebSocket,
// RFC 6455 over node:http upgrade, NO extensions offered) because Node's global WebSocket (undici) offers
// 'permessage-deflate' and Chromium's DevTools server would accept it. The global WebSocket stays as a fallback.
//
//   node cdp-update.mjs --port 9333 --out <dir> [--prefix e2e] [--mode update|version|attach] [--max-wait-sec 720] [--ws mini|native]
//   mode update (default): attach, app version, screenshot, check_update, listeners, install_update {force:true}
//   mode version:          attach and read the app version only (<prefix>-cdp-after.json)
//   mode attach:           attach, app version, screenshot, check_update - and STOP (install_update is NOT called);
//                          used by sacreal-e2e.ps1 before Smart App Control is turned on
//
// Success path of install_update = the app exits by itself, so the socket closing is an expected result.
// The result file <out>/<prefix>-cdp.json is rewritten after every step so partial results survive any kill.
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import crypto from 'node:crypto';

function parseArgs(argv) {
  const o = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith('--')) {
      const k = a.slice(2);
      const nxt = argv[i + 1];
      if (nxt !== undefined && !nxt.startsWith('--')) {
        o[k] = nxt;
        i++;
      } else {
        o[k] = true;
      }
    }
  }
  return o;
}

const args = parseArgs(process.argv.slice(2));
const PORT = Number(args.port || 9333);
const OUT = String(args.out || '.');
const PREFIX = String(args.prefix || 'e2e');
const MODE = String(args.mode || 'update');
const MAX_WAIT_MS = Math.max(30, Number(args['max-wait-sec'] || 720)) * 1000;
const OUT_FILE = path.join(OUT, MODE === 'version' ? `${PREFIX}-cdp-after.json` : `${PREFIX}-cdp.json`);

const iso = () => new Date().toISOString();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const R = {
  script: 'cdp-update.mjs',
  node: process.version,
  mode: MODE,
  port: PORT,
  started: iso(),
  finished: null,
  has_native_websocket: typeof WebSocket !== 'undefined',
  ws_impl: null,
  has_fetch: typeof fetch !== 'undefined',
  targets: null,
  chosen_target: null,
  attached: false,
  tauri_global: null,
  page_info: null,
  app_version: null,
  check_update: null,
  check_update_error: null,
  listeners: null,
  install_update_started_at: null,
  install_update_outcome: null,
  socket_closed_at: null,
  events: { total: 0, progress_count: 0, progress_first: null, progress_last: null, non_progress: [] },
  steps: [],
  errors: [],
};

function save() {
  try {
    fs.mkdirSync(OUT, { recursive: true });
    fs.writeFileSync(OUT_FILE, JSON.stringify(R, null, 2));
  } catch (e) {
    // ignore: nothing else we can do
  }
}

function step(name, extra) {
  const rec = { t: iso(), name, ...(extra || {}) };
  R.steps.push(rec);
  console.log(JSON.stringify(rec).slice(0, 500));
  save();
}

function fail(where, e) {
  const m = `${where}: ${e && e.message ? e.message : String(e)}`;
  R.errors.push({ t: iso(), error: m });
  console.error(m);
  save();
}

// ---------------------------------------------------------------------------
// MiniWebSocket: minimal RFC 6455 client (text frames, fragmentation, ping/pong, close). No extensions.
// Same surface as the WHATWG WebSocket for what this script uses: addEventListener, send, close, readyState.
// ---------------------------------------------------------------------------
const WS_GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';
const WS_MAX_PAYLOAD = 256 * 1024 * 1024;

class MiniWebSocket {
  constructor(url) {
    this.url = url;
    this.readyState = 0; // 0 connecting, 1 open, 2 closing, 3 closed
    this._l = { open: [], message: [], error: [], close: [] };
    this._buf = Buffer.alloc(0);
    this._frag = null;
    this._closeSent = false;
    this._closed = false;
    this._socket = null;
    this._req = null;
    this._start();
  }

  addEventListener(type, fn) {
    if (this._l[type]) this._l[type].push(fn);
  }

  _emit(type, ev) {
    for (const fn of this._l[type].slice()) {
      try {
        fn(ev);
      } catch (e) {
        // a listener error must not break the socket
      }
    }
  }

  _start() {
    let u;
    try {
      u = new URL(this.url);
    } catch (e) {
      setImmediate(() => this._fail(e));
      return;
    }
    const key = crypto.randomBytes(16).toString('base64');
    const req = http.request({
      host: u.hostname,
      port: Number(u.port || 80),
      path: u.pathname + u.search,
      method: 'GET',
      headers: { Host: u.host, Connection: 'Upgrade', Upgrade: 'websocket', 'Sec-WebSocket-Key': key, 'Sec-WebSocket-Version': '13' },
    });
    this._req = req;
    req.on('response', (res) => {
      res.resume();
      this._fail(new Error('handshake rejected: HTTP ' + res.statusCode));
    });
    req.on('upgrade', (res, socket, head) => {
      const want = crypto.createHash('sha1').update(key + WS_GUID).digest('base64');
      if (res.headers['sec-websocket-accept'] !== want) {
        socket.destroy();
        this._fail(new Error('bad Sec-WebSocket-Accept'));
        return;
      }
      this._socket = socket;
      this.readyState = 1;
      socket.setNoDelay(true);
      socket.on('data', (d) => this._onData(d));
      socket.on('error', (e) => this._emit('error', { message: String(e && e.message ? e.message : e) }));
      socket.on('close', () => this._finish());
      this._emit('open', {});
      if (head && head.length) this._onData(head);
    });
    req.on('error', (e) => this._fail(e));
    req.end();
  }

  _fail(e) {
    if (this._closed) return;
    this._emit('error', { message: String(e && e.message ? e.message : e) });
    this._finish();
  }

  _finish() {
    if (this._closed) return;
    this._closed = true;
    this.readyState = 3;
    this._emit('close', {});
  }

  _protocolError(msg) {
    this._emit('error', { message: 'websocket protocol error: ' + msg });
    try {
      this._socket.destroy();
    } catch (e) {
      // ignore
    }
    this._finish();
  }

  _onData(d) {
    this._buf = this._buf.length ? Buffer.concat([this._buf, d]) : d;
    for (;;) {
      const b = this._buf;
      if (b.length < 2) return;
      const fin = (b[0] & 0x80) !== 0;
      const rsv = b[0] & 0x70;
      const op = b[0] & 0x0f;
      const masked = (b[1] & 0x80) !== 0;
      let len = b[1] & 0x7f;
      let off = 2;
      if (rsv !== 0) {
        this._protocolError('RSV bits set (an extension was negotiated?)');
        return;
      }
      if (len === 126) {
        if (b.length < 4) return;
        len = b.readUInt16BE(2);
        off = 4;
      } else if (len === 127) {
        if (b.length < 10) return;
        const big = b.readBigUInt64BE(2);
        if (big > BigInt(WS_MAX_PAYLOAD)) {
          this._protocolError('frame too large');
          return;
        }
        len = Number(big);
        off = 10;
      }
      let mask = null;
      if (masked) {
        if (b.length < off + 4) return;
        mask = b.subarray(off, off + 4);
        off += 4;
      }
      if (b.length < off + len) return;
      const payload = Buffer.from(b.subarray(off, off + len));
      if (mask) for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i & 3];
      this._buf = b.subarray(off + len);
      this._onFrame(fin, op, payload);
      if (this._closed) return;
    }
  }

  _onFrame(fin, op, payload) {
    if (op === 0x8) {
      if (!this._closeSent) this._sendFrame(0x8, payload.subarray(0, 2));
      this.readyState = 2;
      try {
        this._socket.end();
      } catch (e) {
        // ignore
      }
      setTimeout(() => {
        try {
          this._socket.destroy();
        } catch (e) {
          // ignore
        }
      }, 1000);
      return;
    }
    if (op === 0x9) {
      this._sendFrame(0xa, payload);
      return;
    }
    if (op === 0xa) return;
    if (op === 0x1 || op === 0x2) {
      this._frag = { op, parts: [payload] };
    } else if (op === 0x0) {
      if (!this._frag) return;
      this._frag.parts.push(payload);
    } else {
      return;
    }
    if (fin) {
      const all = Buffer.concat(this._frag.parts);
      const isText = this._frag.op === 0x1;
      this._frag = null;
      this._emit('message', { data: isText ? all.toString('utf8') : all });
    }
  }

  _sendFrame(op, payload) {
    if (!this._socket || this._socket.destroyed) return;
    const len = payload.length;
    let head;
    if (len < 126) {
      head = Buffer.alloc(2);
      head[1] = 0x80 | len;
    } else if (len < 65536) {
      head = Buffer.alloc(4);
      head[1] = 0x80 | 126;
      head.writeUInt16BE(len, 2);
    } else {
      head = Buffer.alloc(10);
      head[1] = 0x80 | 127;
      head.writeBigUInt64BE(BigInt(len), 2);
    }
    head[0] = 0x80 | op;
    const mask = crypto.randomBytes(4);
    const body = Buffer.from(payload);
    for (let i = 0; i < body.length; i++) body[i] ^= mask[i & 3];
    this._socket.write(Buffer.concat([head, mask, body]));
    if (op === 0x8) this._closeSent = true;
  }

  send(data) {
    if (this.readyState !== 1) throw new Error('socket not open');
    this._sendFrame(0x1, Buffer.from(String(data), 'utf8'));
  }

  close() {
    if (this.readyState >= 2) return;
    if (!this._socket) {
      try {
        this._req.destroy();
      } catch (e) {
        // ignore
      }
      this._finish();
      return;
    }
    this.readyState = 2;
    this._sendFrame(0x8, Buffer.alloc(0));
    setTimeout(() => {
      try {
        this._socket.destroy();
      } catch (e) {
        // ignore
      }
    }, 500);
  }
}

// ---------------------------------------------------------------------------
// minimal CDP client
// ---------------------------------------------------------------------------
let ws = null;
let nextId = 1;
const pending = new Map();
let closed = false;
let closedAt = null;

function onMessage(ev) {
  let m;
  try {
    const d = typeof ev.data === 'string' ? ev.data : Buffer.from(ev.data).toString('utf8');
    m = JSON.parse(d);
  } catch (e) {
    return;
  }
  if (m && m.id !== undefined && pending.has(m.id)) {
    const p = pending.get(m.id);
    pending.delete(m.id);
    clearTimeout(p.timer);
    if (m.error) {
      const e = new Error(`${p.method}: ${m.error.message || JSON.stringify(m.error)}`);
      e.cdp = m.error;
      p.reject(e);
    } else {
      p.resolve(m.result);
    }
  }
}

function onClose(sock) {
  if (sock !== ws) return;
  closed = true;
  closedAt = iso();
  for (const [, p] of pending) {
    clearTimeout(p.timer);
    const e = new Error('socket closed');
    e.closed = true;
    p.reject(e);
  }
  pending.clear();
}

function connectWith(kind, url, timeoutMs) {
  return new Promise((resolve, reject) => {
    let settled = false;
    let sock;
    try {
      sock = kind === 'native' ? new WebSocket(url) : new MiniWebSocket(url);
    } catch (e) {
      reject(e);
      return;
    }
    const timer = setTimeout(() => {
      if (!settled) {
        settled = true;
        try {
          sock.close();
        } catch (e) {
          // ignore
        }
        reject(new Error(`${kind} websocket open timeout`));
      }
    }, timeoutMs);
    sock.addEventListener('open', () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      ws = sock;
      closed = false;
      closedAt = null;
      resolve(sock);
    });
    sock.addEventListener('error', (ev) => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        reject(new Error(`${kind} websocket error before open: ` + (ev && ev.message ? ev.message : 'unknown')));
      }
    });
    sock.addEventListener('message', onMessage);
    sock.addEventListener('close', () => onClose(sock));
  });
}

// The debugging endpoint is always local: force 127.0.0.1 and our port whatever host the target list printed.
function localUrl(url) {
  const u = new URL(url);
  u.hostname = '127.0.0.1';
  u.port = String(PORT);
  return u.toString();
}

async function connect(url, timeoutMs = 15000) {
  const order = String(args.ws || 'mini') === 'native' ? ['native', 'mini'] : ['mini', 'native'];
  let lastErr = null;
  for (const kind of order) {
    if (kind === 'native' && typeof WebSocket === 'undefined') continue;
    try {
      await connectWith(kind, localUrl(url), timeoutMs);
      R.ws_impl = kind;
      return;
    } catch (e) {
      lastErr = e;
      fail(`connect (${kind}) ${url}`, e);
    }
  }
  throw lastErr || new Error('no websocket implementation');
}

function send(method, params = {}, timeoutMs = 30000) {
  return new Promise((resolve, reject) => {
    if (!ws || closed || ws.readyState !== 1) {
      const e = new Error('socket not open');
      e.closed = true;
      reject(e);
      return;
    }
    const id = nextId++;
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`timeout ${method} after ${timeoutMs} ms`));
    }, timeoutMs);
    pending.set(id, { resolve, reject, timer, method });
    try {
      ws.send(JSON.stringify({ id, method, params }));
    } catch (e) {
      clearTimeout(timer);
      pending.delete(id);
      reject(e);
    }
  });
}

async function evalJs(expression, { awaitPromise = true, timeoutMs = 30000 } = {}) {
  const res = await send('Runtime.evaluate', { expression, awaitPromise, returnByValue: true, userGesture: true }, timeoutMs);
  if (res && res.exceptionDetails) {
    const d = res.exceptionDetails;
    const text = (d.exception && (d.exception.description || d.exception.value)) || d.text || 'evaluate exception';
    throw new Error('page exception: ' + String(text).slice(0, 600));
  }
  return res && res.result ? res.result.value : undefined;
}

async function listTargets() {
  const r = await fetch(`http://127.0.0.1:${PORT}/json/list`, { signal: AbortSignal.timeout(5000) });
  return await r.json();
}

function rankTargets(list) {
  const pages = list.filter((t) => t.type === 'page' && t.webSocketDebuggerUrl);
  const score = (t) => (String(t.url).includes('tauri.localhost') ? 0 : String(t.url).startsWith('http') ? 1 : 2);
  return pages.sort((a, b) => score(a) - score(b));
}

// ---------------------------------------------------------------------------
// in-page snippets
// ---------------------------------------------------------------------------
const EXPR_HAS_TAURI =
  "(typeof window.__TAURI__ !== 'undefined' && window.__TAURI__ && window.__TAURI__.core) ? 'yes' : 'no'";

const EXPR_PAGE_INFO = `JSON.stringify({
  href: location.href,
  title: document.title,
  ua: navigator.userAgent,
  tauri_keys: Object.keys(window.__TAURI__ || {}),
  core_keys: Object.keys((window.__TAURI__ && window.__TAURI__.core) || {}),
  app_keys: Object.keys((window.__TAURI__ && window.__TAURI__.app) || {}),
  event_keys: Object.keys((window.__TAURI__ && window.__TAURI__.event) || {})
})`;

const EXPR_APP_VERSION = `(async () => {
  try {
    if (window.__TAURI__.app && window.__TAURI__.app.getVersion) {
      return JSON.stringify({ ok: true, value: await window.__TAURI__.app.getVersion() });
    }
    return JSON.stringify({ ok: true, value: await window.__TAURI__.core.invoke('plugin:app|version') });
  } catch (e) {
    return JSON.stringify({ ok: false, error: (e && e.message) ? e.message : (typeof e === 'string' ? e : JSON.stringify(e)) });
  }
})()`;

const EXPR_CHECK_UPDATE = `(async () => {
  try {
    const v = await window.__TAURI__.core.invoke('check_update');
    return JSON.stringify({ ok: true, value: (v === undefined ? null : v) });
  } catch (e) {
    return JSON.stringify({ ok: false, error: (e && e.message) ? e.message : (typeof e === 'string' ? e : JSON.stringify(e)) });
  }
})()`;

// [guess] only update-progress and update-error are known event names; the others are harmless extras.
// update-progress arrives once per downloaded chunk (thousands of events): keep a counter plus the first and the last
// payload only; every other event goes into window.__diagEvents.
const EXPR_LISTENERS = `(async () => {
  window.__diagEvents = window.__diagEvents || [];
  window.__diagProgressCount = 0;
  window.__diagProgressFirst = null;
  window.__diagProgressLast = null;
  const names = ['update-progress', 'update-error', 'update-available', 'update-downloaded', 'update-finished', 'update-installing'];
  const out = {};
  for (const n of names) {
    try {
      await window.__TAURI__.event.listen(n, (e) => {
        try {
          const rec = { t: Date.now(), n: n, p: (e && e.payload !== undefined) ? e.payload : null };
          if (n === 'update-progress') {
            window.__diagProgressCount++;
            if (!window.__diagProgressFirst) window.__diagProgressFirst = rec;
            window.__diagProgressLast = rec;
          } else if (window.__diagEvents.length < 500) {
            window.__diagEvents.push(rec);
          }
        } catch (x) {}
      });
      out[n] = 'ok';
    } catch (x) {
      out[n] = 'fail: ' + String(x);
    }
  }
  return JSON.stringify(out);
})()`;

const EXPR_POLL_EVENTS =
  'JSON.stringify({ ev: window.__diagEvents || [], pc: window.__diagProgressCount || 0, pf: window.__diagProgressFirst || null, pl: window.__diagProgressLast || null })';

const EXPR_INSTALL_UPDATE = `(async () => {
  try {
    await window.__TAURI__.core.invoke('install_update', { force: true });
    return 'resolved';
  } catch (e) {
    return 'rejected:' + ((e && e.message) ? e.message : (typeof e === 'string' ? e : JSON.stringify(e)));
  }
})()`;

let lastSeenEvents = 0;
async function pollEvents() {
  try {
    const s = await evalJs(EXPR_POLL_EVENTS, { awaitPromise: false, timeoutMs: 8000 });
    const o = JSON.parse(s);
    const arr = Array.isArray(o.ev) ? o.ev : [];
    for (let i = lastSeenEvents; i < arr.length; i++) {
      R.events.total++;
      if (R.events.non_progress.length < 200) R.events.non_progress.push({ polled_at: iso(), ev: arr[i] });
    }
    lastSeenEvents = arr.length;
    if (o.pc > R.events.progress_count) {
      R.events.total += o.pc - R.events.progress_count;
      R.events.progress_count = o.pc;
      if (!R.events.progress_first && o.pf) R.events.progress_first = { polled_at: iso(), ev: o.pf };
      if (o.pl) R.events.progress_last = { polled_at: iso(), ev: o.pl };
    }
    return arr.length;
  } catch (e) {
    return -1;
  }
}

function parseJsonValue(s) {
  try {
    return JSON.parse(s);
  } catch (e) {
    return { ok: false, error: 'unparsable page result: ' + String(s).slice(0, 300) };
  }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------
async function main() {
  step('start', { argv: process.argv.slice(2).join(' '), native_websocket: R.has_native_websocket });

  // 1. wait for a page target
  let ranked = [];
  const tEnd = Date.now() + 60000;
  while (Date.now() < tEnd) {
    try {
      const l = await listTargets();
      R.targets = l.map((t) => ({ type: t.type, title: t.title, url: t.url, id: t.id }));
      ranked = rankTargets(l);
      if (ranked.length) break;
    } catch (e) {
      // not ready yet
    }
    await sleep(2000);
  }
  step('targets', { count: R.targets ? R.targets.length : 0, pages: ranked.length });
  if (!ranked.length) {
    R.error_summary = 'no page target on the debugging port';
    return;
  }

  // 2. attach and wait for window.__TAURI__
  let chosen = null;
  for (let i = 0; i < ranked.length && !chosen; i++) {
    const t = ranked[i];
    try {
      await connect(t.webSocketDebuggerUrl);
      step('connected', { url: t.url, title: t.title, ws_impl: R.ws_impl });
      const until = Date.now() + (i === 0 ? 90000 : 20000);
      let ok = false;
      while (Date.now() < until && !closed) {
        try {
          const v = await evalJs(EXPR_HAS_TAURI, { awaitPromise: false, timeoutMs: 10000 });
          if (v === 'yes') {
            ok = true;
            break;
          }
        } catch (e) {
          // page may still be loading
        }
        await sleep(2000);
      }
      if (ok) {
        chosen = t;
      } else {
        step('no __TAURI__ on target', { url: t.url });
        try {
          ws.close();
        } catch (e) {
          // ignore
        }
      }
    } catch (e) {
      fail('connect ' + t.url, e);
    }
  }
  if (!chosen) {
    R.error_summary = 'could not attach to a page with window.__TAURI__';
    return;
  }
  R.attached = true;
  R.chosen_target = { url: chosen.url, title: chosen.title, id: chosen.id };
  step('attached', R.chosen_target);

  try {
    R.page_info = parseJsonValue(await evalJs(EXPR_PAGE_INFO, { awaitPromise: false, timeoutMs: 15000 }));
    step('page_info', { href: R.page_info.href });
  } catch (e) {
    fail('page_info', e);
  }

  // 3. app version
  try {
    const av = parseJsonValue(await evalJs(EXPR_APP_VERSION, { timeoutMs: 20000 }));
    R.app_version = av.ok ? av.value : null;
    if (!av.ok) R.app_version_error = av.error;
    step('app_version', { value: R.app_version, error: av.error });
  } catch (e) {
    fail('app_version', e);
  }
  if (MODE === 'version') {
    return;
  }

  // screenshot before
  try {
    const shot = await send('Page.captureScreenshot', { format: 'png' }, 30000);
    fs.writeFileSync(path.join(OUT, `${PREFIX}-before.png`), Buffer.from(shot.data, 'base64'));
    step('screenshot', { file: `${PREFIX}-before.png`, bytes: shot.data.length });
  } catch (e) {
    fail('screenshot', e);
  }

  // 4. check_update
  try {
    const cu = parseJsonValue(await evalJs(EXPR_CHECK_UPDATE, { timeoutMs: 120000 }));
    if (cu.ok) R.check_update = cu.value;
    else R.check_update_error = cu.error;
    step('check_update', { ok: cu.ok, value: cu.ok ? JSON.stringify(cu.value).slice(0, 300) : cu.error });
  } catch (e) {
    R.check_update_error = String(e && e.message ? e.message : e);
    fail('check_update', e);
  }

  if (MODE === 'attach') {
    step('attach mode: stopping before listeners / install_update');
    return;
  }

  // 5. listeners
  try {
    R.listeners = parseJsonValue(await evalJs(EXPR_LISTENERS, { timeoutMs: 30000 }));
    step('listeners', R.listeners);
  } catch (e) {
    fail('listeners', e);
  }

  // 6. install_update (the call that the update button makes after its confirm dialog)
  armWatchdog(MAX_WAIT_MS + 3 * 60 * 1000);
  step('install_update invoking', { force: true, max_wait_ms: MAX_WAIT_MS });
  R.install_update_started_at = iso();
  const installP = send('Runtime.evaluate', { expression: EXPR_INSTALL_UPDATE, awaitPromise: true, returnByValue: true }, MAX_WAIT_MS + 15000).then(
    (res) => ({ kind: 'result', res }),
    (e) => ({ kind: e && e.closed ? 'closed' : 'error', e }),
  );
  const deadline = Date.now() + MAX_WAIT_MS;
  let outcome = null;
  for (;;) {
    const r = await Promise.race([installP, sleep(10000).then(() => ({ kind: 'tick' }))]);
    if (r.kind === 'tick') {
      await pollEvents();
      step('progress', {
        events_total: R.events.total,
        progress_count: R.events.progress_count,
        last_progress: R.events.progress_last ? R.events.progress_last.ev : null,
        socket_closed: closed,
      });
      if (Date.now() > deadline) {
        outcome = 'timeout';
        break;
      }
      continue;
    }
    if (r.kind === 'closed') {
      outcome = 'socket_closed_app_exit';
      break;
    }
    if (r.kind === 'error') {
      outcome = 'error:' + String(r.e && r.e.message ? r.e.message : r.e);
      break;
    }
    // kind === 'result'
    const exc = r.res && r.res.exceptionDetails;
    if (exc) {
      outcome = 'evaluate_exception:' + JSON.stringify(exc).slice(0, 500);
    } else {
      outcome = String(r.res && r.res.result ? r.res.result.value : undefined);
    }
    break;
  }
  R.install_update_outcome = outcome;
  step('install_update outcome', { outcome: outcome ? outcome.slice(0, 300) : outcome, socket_closed: closed });

  // after a rejection or an in-page resolve: keep watching events (and the socket) for a while
  if (outcome && (outcome === 'resolved' || outcome.startsWith('rejected:'))) {
    const extraUntil = Date.now() + (outcome === 'resolved' ? 120000 : 25000);
    while (Date.now() < extraUntil && !closed) {
      await pollEvents();
      await sleep(5000);
    }
    step('post-result watch ended', { socket_closed: closed, events_total: R.events.total });
  }
  if (closed) {
    R.socket_closed_at = closedAt;
  } else {
    await pollEvents();
  }
}

process.on('uncaughtException', (e) => {
  fail('uncaughtException', e);
  R.finished = iso();
  save();
  process.exit(0);
});
process.on('unhandledRejection', (e) => {
  fail('unhandledRejection', e);
});

// Hard stop so that this process can never hang. The pre-install steps can legally take up to ~6 minutes, so the
// watchdog is re-armed right before install_update (armWatchdog) to give that call its full MAX_WAIT_MS.
let watchdog = null;
function armWatchdog(ms) {
  if (watchdog) clearTimeout(watchdog);
  watchdog = setTimeout(() => {
    fail('watchdog', new Error('hard timeout reached'));
    R.finished = iso();
    save();
    process.exit(0);
  }, ms);
}
armWatchdog(MAX_WAIT_MS + 8 * 60 * 1000);

try {
  await main();
} catch (e) {
  fail('main', e);
}
if (closed && !R.socket_closed_at) R.socket_closed_at = closedAt;
R.finished = iso();
save();
if (watchdog) clearTimeout(watchdog);
try {
  if (ws) ws.close();
} catch (e) {
  // ignore
}
await sleep(200);
process.exit(0);
