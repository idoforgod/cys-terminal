// diag/cdp-update.mjs
// Drives the Tauri/WebView2 desktop app over the Chrome DevTools Protocol (WebView2 remote debugging port)
// and calls the same IPC commands the update button calls: check_update, then install_update {force:true}.
// Node >= 18 (global fetch). No external packages. The WebSocket client is a small built-in one (MiniWebSocket,
// RFC 6455 over node:http upgrade, NO extensions offered) because Node's global WebSocket (undici) offers
// 'permessage-deflate' and Chromium's DevTools server would accept it. The global WebSocket stays as a fallback.
//
//   node cdp-update.mjs --port 9333 --out <dir> [--prefix e2e] [--mode update|version|attach|ui|uipre] [--max-wait-sec 720] [--ws mini|native]
//   mode update (default): attach, app version, screenshot, check_update, listeners, install_update {force:true}
//   mode version:          attach and read the app version only (<prefix>-cdp-after.json)
//   mode attach:           attach, app version, screenshot, check_update - and STOP (install_update is NOT called);
//                          used by sacreal-e2e.ps1 before Smart App Control is turned on
//   mode ui / uipre:       the REAL UI flow (Update button -> panel -> bin patch button -> confirm), see the block "4th task" below;
//                          used by app-e2e.ps1 (uipre = attach + wait for the UI, no click)
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
// 4th task (diag/app-e2e.ps1): the REAL UI flow of the app. Modes:
//   uipre  attach, wait for the Update button, install the in-page recorder, one screenshot - and STOP (nothing is clicked)
//   ui     uipre + click the Update button -> panel -> "bin patch install" button -> confirm "install", then observe for
//          --observe-sec (default 90) while the page is polled once a second (toasts, modals, panel) and screenshots are taken.
// Buttons are clicked with real mouse events (Input.dispatchMouseEvent at the centre of the element when a hit test says that the
// element itself is under that point), else with element.click() (marked "dom_click" in the click record).
// If a step of the flow cannot be completed the page state is saved and install_update {force:true} is invoked instead
// (R.ui.fallback_invoke = true: that is NOT a button click). Options: --observe-sec --ready-wait-sec --panel-wait-sec
// --modal-wait-sec --fallback-invoke 0|1.
// Selectors follow the product UI (ui/src/main.ts, 0.14.43): #btn-update, .upd-panel (.upd-headline .upd-row .upd-meta
// .modal-btns button), .confirm-overlay .modal (h3, p, .modal-btns .modal-yes), #toasts > .toast (.toast-name .toast-detail).
// Korean UI texts are written as \u escapes so that this source stays ASCII.
// ---------------------------------------------------------------------------
const UI_OBSERVE_MS = Math.max(5, Number(args['observe-sec'] || 90)) * 1000;
const UI_READY_MS = Math.max(20, Number(args['ready-wait-sec'] || 90)) * 1000;
const UI_PANEL_MS = Math.max(20, Number(args['panel-wait-sec'] || 180)) * 1000;
const UI_MODAL_MS = Math.max(10, Number(args['modal-wait-sec'] || 60)) * 1000;
const UI_FALLBACK = String(args['fallback-invoke'] === undefined ? '1' : args['fallback-invoke']) !== '0';
const K_UI = {
  update_btn: "document.getElementById('btn-update')",
  bin_label: '\uBCF8\uCCB4 \uD328\uCE58 \uC124\uCE58', // bin patch install button of the update panel
  checking_label: '\uD655\uC778 \uC911', // "checking" (the second button reads this while the check runs)
  recheck_label: '\uB2E4\uC2DC \uD655\uC778', // "check again"
  confirm_title_part: '\uD328\uCE58 \uC124\uCE58', // "patch install" (part of the confirm window title)
  yes_label: '\uC124\uCE58', // "install" (yes button of the confirm window)
  sac_word: '\uC2A4\uB9C8\uD2B8 \uC571 \uCEE8\uD2B8\uB864', // "Smart App Control"
  blocked_title: '\uC124\uCE58 \uD30C\uC77C \uC2E4\uD589\uC774 \uCC28\uB2E8\uB418\uC5C8\uC2B5\uB2C8\uB2E4', // "installer launch was blocked"
  kept_phrase: '\uC571\uC740 \uB2EB\uD788\uC9C0 \uC54A\uC558\uC2B5\uB2C8\uB2E4', // "the app was not closed" (body of that notification)
};

// one JSON snapshot of everything the flow looks at
const EXPR_UI_SNAPSHOT = `(() => {
  const txt = (el) => (el && el.textContent ? String(el.textContent).replace(/\\s+/g, ' ').trim() : '');
  const q = (s, root) => (root || document).querySelector(s);
  const qa = (s, root) => Array.prototype.slice.call((root || document).querySelectorAll(s));
  const btn = (b) => ({ text: txt(b), cls: String(b.className || ''), disabled: !!b.disabled });
  const bu = document.getElementById('btn-update');
  const badge = document.getElementById('update-badge');
  const panel = q('.upd-panel');
  const confirm = q('.confirm-overlay .modal');
  const o = {
    t: Date.now(),
    ready: !!bu,
    update_button: bu ? { text: txt(bu), badge_text: badge ? txt(badge) : null, badge_hidden: badge ? !!badge.hidden : null, badge_title: badge ? String(badge.title || '') : null } : null,
    panel: panel ? { headline: txt(q('.upd-headline', panel)), rows: qa('.upd-row', panel).map((r) => ({ k: txt(q('.upd-k', r)), v: txt(q('.upd-v', r)) })), meta: txt(q('.upd-meta', panel)), buttons: qa('.modal-btns button', panel).map(btn) } : null,
    confirm: confirm ? { title: txt(q('h3', confirm)), body: String(q('p', confirm) ? q('p', confirm).textContent : '').slice(0, 6000), buttons: qa('.modal-btns button', confirm).map(btn) } : null,
    overlays: qa('.modal-overlay').map((e) => String(e.className || '')),
    toasts: qa('#toasts > *').map((e) => ({ cls: String(e.className || ''), name: txt(q('.toast-name', e)), detail: txt(q('.toast-detail', e)).slice(0, 4000) })),
    title: document.title,
  };
  return JSON.stringify(o);
})()`;

// in-page recorder: toasts (appear / detail change / gone) and modal windows (open / close). Installed once per page.
const EXPR_UI_RECORDER = `(() => {
  if (window.__diagUi) return 'already';
  const txt = (el) => (el && el.textContent ? String(el.textContent).replace(/\\s+/g, ' ').trim() : '');
  const q = (s, root) => (root || document).querySelector(s);
  const U = (window.__diagUi = { t0: Date.now(), events: [], names: {} });
  const push = (e) => { if (U.events.length < 400) { e.t = Date.now(); U.events.push(e); } };
  const scanToasts = () => {
    const box = document.getElementById('toasts');
    if (!box) return;
    const seen = {};
    Array.prototype.slice.call(box.children).forEach((el) => {
      const name = txt(q('.toast-name', el));
      const detail = txt(q('.toast-detail', el));
      const key = name || '(no name)';
      seen[key] = true;
      const rec = U.names[key];
      if (!rec) {
        U.names[key] = { first: Date.now(), last: Date.now(), n: 1, detail: detail, cls: String(el.className || ''), shown: true, detail_events: 0, last_detail_event: 0, reappears: 0 };
        push({ ev: 'toast_appear', name: name, detail: detail.slice(0, 4000), cls: String(el.className || '') });
      } else {
        rec.last = Date.now();
        rec.n++;
        if (!rec.shown) {
          rec.shown = true;
          rec.reappears++;
          if (rec.reappears <= 6) push({ ev: 'toast_reappear', name: name, detail: detail.slice(0, 4000) });
        }
        if (rec.detail !== detail) {
          rec.detail = detail;
          if (rec.detail_events < 12 && (Date.now() - rec.last_detail_event) > 2500) {
            rec.detail_events++;
            rec.last_detail_event = Date.now();
            push({ ev: 'toast_detail', name: name, detail: detail.slice(0, 4000) });
          }
        }
      }
    });
    Object.keys(U.names).forEach((k) => {
      if (!seen[k] && U.names[k].shown) { U.names[k].shown = false; push({ ev: 'toast_gone', name: k }); }
    });
  };
  const box = document.getElementById('toasts');
  if (box && typeof MutationObserver !== 'undefined') {
    new MutationObserver(scanToasts).observe(box, { childList: true, subtree: true, characterData: true });
  }
  if (typeof MutationObserver !== 'undefined') {
    new MutationObserver((muts) => {
      muts.forEach((m) => {
        Array.prototype.slice.call(m.addedNodes || []).forEach((n) => {
          if (n && n.className && String(n.className).indexOf('modal-overlay') >= 0) {
            push({ ev: 'modal_open', cls: String(n.className), title: txt(q('h3', n)), body: String(q('p', n) ? q('p', n).textContent : '').slice(0, 6000), buttons: Array.prototype.slice.call(n.querySelectorAll('button')).map(txt) });
          }
        });
        Array.prototype.slice.call(m.removedNodes || []).forEach((n) => {
          if (n && n.className && String(n.className).indexOf('modal-overlay') >= 0) {
            push({ ev: 'modal_close', cls: String(n.className), title: txt(q('h3', n)) });
          }
        });
      });
    }).observe(document.body, { childList: true });
  }
  scanToasts();
  return 'installed';
})()`;

const EXPR_UI_RECORDER_POLL =
  'JSON.stringify({ events: (window.__diagUi && window.__diagUi.events) || [], names: (window.__diagUi && window.__diagUi.names) || {} })';

// what the page looks like when a step of the flow failed (selectors may have changed): top bar buttons, overlays, toasts, text head
const EXPR_UI_DOM_SUMMARY = `(() => {
  const txt = (el) => (el && el.textContent ? String(el.textContent).replace(/\\s+/g, ' ').trim() : '');
  const qa = (s) => Array.prototype.slice.call(document.querySelectorAll(s));
  return JSON.stringify({
    href: location.href,
    title: document.title,
    top_buttons: qa('#topbar button').map((b) => ({ id: b.id, text: txt(b).slice(0, 60), disabled: !!b.disabled })),
    all_button_ids: qa('button[id]').map((b) => b.id).slice(0, 80),
    overlays: qa('.modal-overlay').map((o) => ({ cls: String(o.className || ''), text: txt(o).slice(0, 600) })),
    toasts: qa('#toasts > *').map((e) => txt(e).slice(0, 600)),
    body_text_head: String(document.body ? document.body.innerText : '').slice(0, 3000),
  });
})()`;

function findButtonExpr(rootSel, label) {
  return (
    '(function () { var n = document.querySelectorAll(' + JSON.stringify(rootSel + ' button') + '); ' +
    'for (var i = 0; i < n.length; i++) { if ((n[i].textContent || "").trim() === ' + JSON.stringify(label) + ') return n[i]; } return null; })()'
  );
}

function exprFindRect(findExpr) {
  return `(() => {
  const el = ${findExpr};
  if (!el) return JSON.stringify({ found: false });
  try { el.scrollIntoView({ block: 'center', inline: 'center' }); } catch (e) {}
  const r = el.getBoundingClientRect();
  const x = r.left + r.width / 2;
  const y = r.top + r.height / 2;
  const top = document.elementFromPoint(x, y);
  const hit = !!top && (top === el || el.contains(top));
  const d = (n) => (n ? String(n.tagName || '') + (n.id ? '#' + n.id : '') + (n.className ? '.' + String(n.className).split(' ').join('.') : '') : null);
  return JSON.stringify({ found: true, x: x, y: y, w: r.width, h: r.height, hit: hit, top: d(top), el: d(el), text: String(el.textContent || '').replace(/\\s+/g, ' ').trim().slice(0, 80), disabled: !!el.disabled, vw: window.innerWidth, vh: window.innerHeight });
})()`;
}

function exprDomClick(findExpr) {
  return `(() => { const el = ${findExpr}; if (!el) return 'notfound'; el.click(); return 'clicked'; })()`;
}

async function uiSnapshot() {
  return parseJsonValue(await evalJs(EXPR_UI_SNAPSHOT, { awaitPromise: false, timeoutMs: 12000 }));
}

let uiLastStateKey = '';
function uiNoteState(snap) {
  if (!R.ui || !snap || snap.ok === false) return;
  const key = JSON.stringify([
    snap.ready,
    snap.panel ? [snap.panel.headline, (snap.panel.buttons || []).map((b) => b.text + (b.disabled ? '(off)' : ''))] : null,
    snap.confirm ? snap.confirm.title : null,
    (snap.toasts || []).map((t) => t.name),
    (snap.overlays || []).length,
  ]);
  if (key === uiLastStateKey) return;
  uiLastStateKey = key;
  if (R.ui.states.length < 250) {
    R.ui.states.push({
      t: iso(),
      ready: snap.ready,
      panel_headline: snap.panel ? snap.panel.headline : null,
      panel_buttons: snap.panel ? (snap.panel.buttons || []).map((b) => b.text + (b.disabled ? ' (disabled)' : '')) : null,
      confirm_title: snap.confirm ? snap.confirm.title : null,
      toasts: (snap.toasts || []).map((t) => t.name),
      overlays: (snap.overlays || []).length,
    });
  }
}

async function uiWait(pred, timeoutMs) {
  const t0 = Date.now();
  let last = null;
  while (Date.now() - t0 < timeoutMs && !closed) {
    try {
      last = await uiSnapshot();
      uiNoteState(last);
      if (last && last.ok !== false && pred(last)) return { ok: true, snap: last, waited_ms: Date.now() - t0 };
    } catch (e) {
      if (closed) break;
    }
    await sleep(500);
  }
  return { ok: false, snap: last, waited_ms: Date.now() - t0, closed: closed };
}

async function uiShot(name) {
  try {
    const shot = await send('Page.captureScreenshot', { format: 'png' }, 30000);
    const file = `${PREFIX}-ui-${name}.png`;
    fs.writeFileSync(path.join(OUT, file), Buffer.from(shot.data, 'base64'));
    if (R.ui) R.ui.shots.push({ name, file, t: iso(), bytes: shot.data.length });
    save();
    return file;
  } catch (e) {
    fail('screenshot ' + name, e);
    return null;
  }
}

// click one element: a real mouse click when the element itself is under its centre point, else element.click()
async function uiClick(findExpr, label, forceDom) {
  const rec = { label, t: iso(), method: null, ok: false };
  try {
    const info = parseJsonValue(await evalJs(exprFindRect(findExpr), { awaitPromise: false, timeoutMs: 15000 }));
    rec.find = info;
    if (!info || !info.found) {
      rec.error = 'element not found';
    } else if (info.disabled) {
      rec.error = 'element is disabled';
    } else if (!forceDom && info.hit && info.w > 0 && info.h > 0) {
      await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: info.x, y: info.y, button: 'none', buttons: 0 }, 10000);
      await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: info.x, y: info.y, button: 'left', buttons: 1, clickCount: 1 }, 10000);
      await sleep(60);
      await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: info.x, y: info.y, button: 'left', buttons: 0, clickCount: 1 }, 10000);
      rec.method = 'mouse';
      rec.ok = true;
    } else {
      const r = await evalJs(exprDomClick(findExpr), { awaitPromise: false, timeoutMs: 15000 });
      rec.method = 'dom_click';
      rec.dom_result = r;
      rec.ok = r === 'clicked';
      if (!forceDom && info && !info.hit) rec.why_dom = 'a different element is on top of the click point: ' + info.top;
    }
  } catch (e) {
    rec.error = String(e && e.message ? e.message : e);
    rec.closed = closed;
    // the mouse event path failed (a CDP call threw or timed out): the DOM click still works without it
    if (!closed && !forceDom && !rec.ok) {
      try {
        const r = await evalJs(exprDomClick(findExpr), { awaitPromise: false, timeoutMs: 15000 });
        rec.method = 'dom_click';
        rec.dom_result = r;
        rec.ok = r === 'clicked';
        rec.why_dom = 'the mouse event path failed: ' + rec.error;
      } catch (e2) {
        rec.error2 = String(e2 && e2.message ? e2.message : e2);
      }
    }
  }
  if (R.ui) R.ui.clicks.push(rec);
  save();
  return rec;
}

// click, then wait for the expected page state; when it does not show up within retryAfterMs the click is repeated once as a DOM click.
// A retry that finds no element any more (the first click already closed the window that held it) just keeps waiting.
async function uiClickThen(findExpr, label, pred, timeoutMs, retryAfterMs, onClicked) {
  let c1 = await uiClick(findExpr, label, false);
  if (!c1.ok && !closed && c1.error === 'element not found') {
    // the element may not be drawn yet: one more look after a second
    await sleep(1000);
    c1 = await uiClick(findExpr, label + ' (second look)', false);
  }
  if (!c1.ok) return { ok: false, click: c1 };
  if (onClicked) {
    try {
      onClicked(c1);
    } catch (e) {
      // ignore
    }
  }
  let w = await uiWait(pred, Math.min(timeoutMs, retryAfterMs));
  if (w.ok) return { ok: true, click: c1, wait: w };
  if (closed) return { ok: false, click: c1, wait: w };
  const c2 = await uiClick(findExpr, label + ' (retry as DOM click)', true);
  const gone = !c2.ok && c2.error === 'element not found';
  if (!c2.ok && !gone) return { ok: false, click: c2, wait: w };
  w = await uiWait(pred, Math.max(1000, timeoutMs - retryAfterMs));
  return { ok: w.ok, click: gone ? c1 : c2, wait: w };
}

const hasBinButton = (s) => !!(s.panel && (s.panel.buttons || []).some((b) => b.text === K_UI.bin_label && !b.disabled));
const checkRunning = (s) => !!(s.panel && (s.panel.buttons || []).some((b) => b.text.indexOf(K_UI.checking_label) >= 0));
const hasRecheck = (s) => !!(s.panel && (s.panel.buttons || []).some((b) => b.text === K_UI.recheck_label && !b.disabled));

async function uiDomSummary(tag) {
  try {
    const s = parseJsonValue(await evalJs(EXPR_UI_DOM_SUMMARY, { awaitPromise: false, timeoutMs: 15000 }));
    if (R.ui) R.ui['dom_summary_' + tag] = s;
    try {
      fs.writeFileSync(path.join(OUT, `${PREFIX}-ui-dom-${tag}.json`), JSON.stringify(s, null, 2));
    } catch (e) {
      // ignore
    }
    save();
  } catch (e) {
    fail('dom summary ' + tag, e);
  }
}

// steps 1-4 of the click sequence: Update button -> panel -> bin patch button -> confirm window. Returns { ok, error, retryable }.
async function uiReachConfirm(U, attempt) {
  U.stage = 'click_update_button';
  step('ui: click the Update button', { attempt });
  const c1 = await uiClickThen(K_UI.update_btn, 'update button', (s) => !!s.panel, 20000, 6000);
  if (!c1.ok) return { ok: false, error: 'the update panel did not open after clicking #btn-update', retryable: false };
  U.stage = 'wait_bin_button';
  step('ui: panel open, waiting for the bin patch button', { attempt });
  const panelOpenedAt = Date.now();
  let retried = !!U.panel_recheck;
  let w = { ok: false, snap: null };
  const until = Date.now() + UI_PANEL_MS;
  while (Date.now() < until && !closed) {
    // "fresh" = the bin patch button is there AND the re-check has finished: the panel redraws its buttons when the check ends, and a click
    // that straddles the redraw is lost. A check that hangs for 40 s is given up on: the button of the previous check is used.
    w = await uiWait(
      (s) => !!s.panel && ((hasBinButton(s) && (!checkRunning(s) || Date.now() - panelOpenedAt > 40000)) || (!hasBinButton(s) && hasRecheck(s) && !checkRunning(s) && Date.now() - panelOpenedAt > 20000)),
      5000,
    );
    if (!w.ok) continue;
    if (hasBinButton(w.snap)) break;
    // the check finished and there is no bin patch button: look again once ("check again"), then give up
    if (!retried) {
      retried = true;
      U.panel_recheck = true;
      step('ui: check finished without a bin patch button, clicking "check again" once');
      await uiClick(findButtonExpr('.upd-panel .modal-btns', K_UI.recheck_label), 'check again', false);
      await sleep(1500);
      continue;
    }
    break;
  }
  const snap = w.snap || (await uiSnapshot().catch(() => null));
  U.panel = snap && snap.panel ? snap.panel : null;
  U.update_button_state = snap ? snap.update_button : null;
  if (!snap || !hasBinButton(snap)) return { ok: false, error: 'no enabled bin patch button in the update panel', retryable: false };
  await uiShot('panel');
  U.stage = 'click_bin_button';
  step('ui: click the bin patch button', { panel_headline: snap.panel.headline, attempt });
  const confirmPred = (s) => !!(s.confirm && (s.confirm.title || '').indexOf(K_UI.confirm_title_part) >= 0);
  // the first attempt gives the confirm window 30 s (it normally needs a few seconds: one registry query with a 5 s cap); then it is tried again once
  const modalWait = attempt === 1 ? Math.min(UI_MODAL_MS, 30000) : UI_MODAL_MS;
  const c2 = await uiClickThen(findButtonExpr('.upd-panel .modal-btns', K_UI.bin_label), 'bin patch button', confirmPred, modalWait, 10000);
  let cs = c2.ok && c2.wait && c2.wait.snap ? c2.wait.snap.confirm : null;
  if (!cs && !closed) {
    const late = await uiSnapshot().catch(() => null);
    if (late && late.ok !== false && confirmPred(late)) cs = late.confirm;
  }
  if (!cs) return { ok: false, error: 'the confirm window did not appear after clicking the bin patch button', retryable: attempt === 1 };
  U.confirm = { title: cs.title, body: cs.body, buttons: cs.buttons, sac_note_in_body: String(cs.body || '').indexOf(K_UI.sac_word) >= 0, seen_at: iso() };
  step('ui: confirm window', { title: cs.title, sac_note: U.confirm.sac_note_in_body });
  await uiShot('confirm');
  return { ok: true };
}

// the click sequence; returns true when the confirm window was answered with "install"
async function uiButtonFlow(U) {
  let r = await uiReachConfirm(U, 1);
  if (!r.ok && r.retryable && !closed) {
    U.retry_from_update_button = true;
    step('ui: no confirm window - trying again from the Update button', { why: r.error });
    r = await uiReachConfirm(U, 2);
  }
  if (!r.ok) {
    U.flow_error = r.error;
    return false;
  }
  U.stage = 'click_install';
  // the time of the click is written (and saved) right after the click itself: the app may exit within seconds of it
  const c3 = await uiClickThen(findButtonExpr('.confirm-overlay .modal-btns', K_UI.yes_label), 'confirm install button', (s) => !s.confirm, 15000, 6000, () => {
    U.clicked_install_at = iso();
    U.clicked_install_ms = Date.now();
    save();
  });
  U.install_click = c3.click;
  if (!U.clicked_install_at) {
    U.flow_error = 'the install button of the confirm window could not be clicked';
    return false;
  }
  if (!c3.ok && !closed) {
    U.flow_error = 'the confirm window stayed open after clicking its install button';
    return false;
  }
  U.button_flow_complete = true;
  return true;
}

// the persistent notification of a blocked installer launch: from the recorder events first, else from the last snapshot
function findBlockedToast(U) {
  const evs = (U.recorder && U.recorder.events) || [];
  for (let i = 0; i < evs.length; i++) {
    const e = evs[i];
    if ((e.ev === 'toast_appear' || e.ev === 'toast_reappear' || e.ev === 'toast_detail') && String(e.name || '').indexOf(K_UI.blocked_title) >= 0) {
      const d = String(e.detail || '');
      return { found: true, from: 'recorder', event: e.ev, t: e.t, name: String(e.name), detail: d, has_4551: d.indexOf('4551') >= 0, has_kept_phrase: d.indexOf(K_UI.kept_phrase) >= 0 };
    }
  }
  const ts = (U.final_snapshot && U.final_snapshot.toasts) || [];
  for (let i = 0; i < ts.length; i++) {
    if (String(ts[i].name || '').indexOf(K_UI.blocked_title) >= 0) {
      const d = String(ts[i].detail || '');
      return { found: true, from: 'snapshot', name: String(ts[i].name), detail: d, has_4551: d.indexOf('4551') >= 0, has_kept_phrase: d.indexOf(K_UI.kept_phrase) >= 0 };
    }
  }
  return { found: false };
}

async function uiObserve(U) {
  U.stage = 'observe';
  const t0 = U.clicked_install_ms || Date.now();
  step('ui: observing', { observe_ms: UI_OBSERVE_MS, fallback_invoke: !!U.fallback_invoke });
  let polls = 0;
  let lastSnap = null;
  let lastRec = 0;
  let lastEv = 0;
  let shot2 = false;
  let shot10 = false;
  let shotBlocked = false;
  let shotLate = false;
  let blockedSeenAt = null;
  while (Date.now() - t0 < UI_OBSERVE_MS && !closed) {
    polls++;
    try {
      lastSnap = await uiSnapshot();
      uiNoteState(lastSnap);
    } catch (e) {
      if (closed) break;
    }
    const el = Date.now() - t0;
    if (U.fallback_promise) {
      const o = await Promise.race([U.fallback_promise, sleep(1).then(() => null)]);
      if (o && !U.fallback_outcome) {
        U.fallback_outcome = o.kind === 'result' ? String(o.res && o.res.result ? o.res.result.value : 'no value') : o.kind + (o.e ? ':' + String(o.e.message || o.e) : '');
        step('ui: fallback install_update outcome', { outcome: String(U.fallback_outcome).slice(0, 300) });
      }
    }
    if (lastSnap && lastSnap.toasts && !blockedSeenAt) {
      const bt0 = lastSnap.toasts.find((t) => String(t.name || '').indexOf(K_UI.blocked_title) >= 0);
      if (bt0) {
        blockedSeenAt = iso();
        U.blocked_toast_seen_at = blockedSeenAt;
        const d0 = String(bt0.detail || '');
        U.blocked_toast = { found: true, from: 'snapshot', name: String(bt0.name), detail: d0, has_4551: d0.indexOf('4551') >= 0, has_kept_phrase: d0.indexOf(K_UI.kept_phrase) >= 0 };
        step('ui: the "installer launch blocked" notification is on the screen', { has_4551: U.blocked_toast.has_4551 });
      }
    }
    if (!shot2 && el >= 2000) { shot2 = true; await uiShot('after-click-2s'); }
    if (!shot10 && el >= 10000) { shot10 = true; await uiShot('after-click-10s'); }
    if (blockedSeenAt && !shotBlocked) { shotBlocked = true; await sleep(700); await uiShot('blocked-toast'); }
    if (!shotLate && el >= Math.max(30000, UI_OBSERVE_MS - 8000)) { shotLate = true; await uiShot('observe-late'); }
    if (Date.now() - lastRec >= 2000) {
      lastRec = Date.now();
      try {
        const rp = parseJsonValue(await evalJs(EXPR_UI_RECORDER_POLL, { awaitPromise: false, timeoutMs: 10000 }));
        if (rp && rp.events) { U.recorder = rp; }
      } catch (e) {
        // ignore
      }
    }
    if (Date.now() - lastEv >= 2000) {
      lastEv = Date.now();
      await pollEvents();
    }
    save();
    await sleep(1000);
  }
  U.observe = { polls, ms: Date.now() - t0, socket_closed: closed, socket_closed_at: closed ? closedAt : null };
  if (!closed) {
    try {
      lastSnap = await uiSnapshot();
      uiNoteState(lastSnap);
      const rp = parseJsonValue(await evalJs(EXPR_UI_RECORDER_POLL, { awaitPromise: false, timeoutMs: 10000 }));
      if (rp && rp.events) U.recorder = rp;
      await pollEvents();
      await uiShot('observe-end');
    } catch (e) {
      fail('observe end', e);
    }
  }
  U.final_snapshot = lastSnap;
  U.blocked_toast_final = !!(lastSnap && lastSnap.toasts && lastSnap.toasts.some((t) => String(t.name || '').indexOf(K_UI.blocked_title) >= 0));
  if (!U.blocked_toast || !U.blocked_toast.found) U.blocked_toast = findBlockedToast(U);
  if (!closed) {
    try {
      U.body_text_head = String(await evalJs("String(document.body ? document.body.innerText : '').slice(0, 4000)", { awaitPromise: false, timeoutMs: 10000 }));
    } catch (e) {
      // ignore
    }
  }
}

async function runUiMode() {
  // the hard stop of this process covers all waits of the flow plus the observation
  armWatchdog(UI_READY_MS + UI_PANEL_MS + UI_MODAL_MS + UI_OBSERVE_MS + 6 * 60 * 1000);
  const U = (R.ui = { mode: MODE, stage: 'start', ready: null, clicks: [], states: [], shots: [], button_flow_complete: false, fallback_invoke: false, observe: null });
  step('ui: wait for the Update button');
  U.stage = 'wait_ui_ready';
  const ready = await uiWait((s) => !!s.ready, UI_READY_MS);
  U.ready = { ok: ready.ok, waited_ms: ready.waited_ms, snapshot: ready.snap };
  if (!ready.ok) {
    U.flow_error = 'the Update button (#btn-update) did not appear within ' + Math.round(UI_READY_MS / 1000) + ' s';
    await uiDomSummary('not-ready');
  } else {
    try {
      U.recorder_installed = await evalJs(EXPR_UI_RECORDER, { awaitPromise: false, timeoutMs: 15000 });
    } catch (e) {
      fail('recorder', e);
    }
    await uiShot('ready');
  }
  if (MODE === 'uipre') {
    U.stage = 'done';
    save();
    return;
  }
  // listeners for the progress events of the download (same as the old driver)
  try {
    R.listeners = parseJsonValue(await evalJs(EXPR_LISTENERS, { timeoutMs: 30000 }));
    step('listeners', R.listeners);
  } catch (e) {
    fail('listeners', e);
  }
  let flowOk = false;
  if (ready.ok && !closed) {
    try {
      flowOk = await uiButtonFlow(U);
    } catch (e) {
      fail('ui button flow', e);
      U.flow_error = 'exception in the button flow: ' + String(e && e.message ? e.message : e);
    }
  }
  if (!flowOk && !closed) {
    await uiDomSummary('flow-failed');
    if (UI_FALLBACK) {
      U.stage = 'fallback_invoke';
      U.fallback_invoke = true;
      step('ui: the button flow could not be completed - invoking install_update {force:true} instead (NOT a button click)', { why: U.flow_error });
      U.clicked_install_at = iso();
      U.clicked_install_ms = Date.now();
      U.fallback_promise = send('Runtime.evaluate', { expression: EXPR_INSTALL_UPDATE, awaitPromise: true, returnByValue: true }, UI_OBSERVE_MS + 15000).then(
        (res) => ({ kind: 'result', res }),
        (e) => ({ kind: e && e.closed ? 'closed' : 'error', e }),
      );
    }
  }
  if (flowOk || U.fallback_invoke) {
    await uiObserve(U);
  }
  U.stage = 'done';
  save();
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
  if (MODE === 'ui' || MODE === 'uipre') {
    await runUiMode();
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
