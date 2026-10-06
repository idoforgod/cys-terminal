// diag/cdp-update.mjs
// Drives the Tauri/WebView2 desktop app over the Chrome DevTools Protocol (WebView2 remote debugging port)
// and calls the same IPC commands the update button calls: check_update, then install_update {force:true}.
// Node >= 18 (global fetch). No external packages. The WebSocket client is a small built-in one (MiniWebSocket,
// RFC 6455 over node:http upgrade, NO extensions offered) because Node's global WebSocket (undici) offers
// 'permessage-deflate' and Chromium's DevTools server would accept it. The global WebSocket stays as a fallback.
//
//   node cdp-update.mjs --port 9333 --out <dir> [--prefix e2e] [--mode update|version|attach|ui|uipre|uiteam|uiobs|w44] [--max-wait-sec 720] [--ws mini|native]
//   mode update (default): attach, app version, screenshot, check_update, listeners, install_update {force:true}
//   mode version:          attach and read the app version only (<prefix>-cdp-after.json)
//   mode attach:           attach, app version, screenshot, check_update - and STOP (install_update is NOT called);
//                          used by sacreal-e2e.ps1 before Smart App Control is turned on
//   mode ui / uipre:       the REAL UI flow (Update button -> panel -> bin patch button -> confirm), see the block "4th task" below;
//                          used by app-e2e.ps1 (uipre = attach + wait for the UI, no click)
//   mode uiteam:           the "create a team directly" flow of the real UI (scene TEAM of app-e2e.ps1), see the block "5th task" below;
//                          nothing is invoked instead of a click (no fallback call)
//   mode uiobs:            OBSERVE ONLY (scene UPGRADE of app-upgrade.ps1), see the block "7th task" below: the status bar, the version-skew
//                          badge, the toasts and the product's read-only daemon_status call, polled for --obs-sec; nothing is clicked
//
// Success path of install_update = the app exits by itself, so the socket closing is an expected result.
// The result file <out>/<prefix>-cdp.json is rewritten after every step so partial results survive any kill.
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';

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
// 5th task (diag/app-e2e.ps1, scene TEAM): the "create a team directly" flow of the REAL UI of the 0.14.43 app. Mode:
//   uiteam  attach, wait for the UI (a workspace tab AND a pane are drawn = the product's start() has finished), install the page
//           recorder and the event listeners, open the "expert" section of the sidebar (real mouse click on #btn-expert-toggle), press
//           the team button (#btn-ws-dept -> openTeamCreateFlow), answer the confirm window with its execute button (.modal-yes; when the
//           product shows a menu first, its LAST item = "new numbered team"), then observe for --team-observe-sec (default 360) while the
//           page is polled once a second: workspace tabs, the waiting screen (text + stage line), every toast (class, title, body), menus,
//           confirm windows, and the Tauri events 'dept-create-progress' (the pack's @stage markers) and 'daemon-event' (raw, capped).
//           At the end: the seat list of the new team (the product's own list_surfaces call), the alarm history of the Control Center
//           (the only place where the sticky toast ids are visible) and facts for the verdict.
// NOTHING IS INVOKED INSTEAD OF A BUTTON: if a click cannot be made the page state (DOM summary + screenshot) is saved and the mode ends
// (no fallback call of allocate_dept_daemon). The only invoke() calls are read-only observations the product itself makes
// (list_depts, read_dept_catalog, list_surfaces). The mode does not judge by screen texts: it records the texts verbatim and judges by
// structure (selectors / counts / classes). Files: <prefix>-cdp.json (everything), <prefix>-facts.json (small: facts + reference for the
// PowerShell verdict), <prefix>-live.json (tiny, rewritten every poll: the click time for the PowerShell side), <prefix>-ui-*.png.
// Options: --team-observe-sec 360 --team-ready-wait-sec 150 --team-settle-sec 6 --team-react-sec 60.
// Selectors follow the product UI (ui/src/main.ts, 0.14.43 snapshot 9650334f): #btn-expert-toggle / #wsbar-expert-body / #btn-ws-dept
// (mountExpertSection), #ws-tabs > .ws-tab[data-ws-id] > .ws-title-row > .ws-name + .ws-sub (buildTab; pending placeholders are NOT drawn as
// tabs), #root > .pane.dept-pending[aria-busy=true] > .dept-pending-box > .dept-pending-msg + .dept-pending-stage (renderDeptPending),
// .confirm-overlay > .modal > h3 + p + .modal-btns > .modal-no + .modal-yes (confirmModal), #ctx-menu > .ctx-item (showCtxMenu),
// #toasts > div.toast.<category> > .toast-name + .toast-detail (toast / stickyToast), #btn-cc + #cc-tabs .cc-tab[data-view=alarms] +
// #cc-alarm-list .alarm-item (> .al-meta .al-title .al-body) (renderAlarmHistory).
// ---------------------------------------------------------------------------
const teamNum = (v, d) => {
  const n = Number(v);
  return Number.isFinite(n) ? n : d;
};
const TEAM_OBSERVE_MS = Math.max(3, teamNum(args['team-observe-sec'], 360)) * 1000;
const TEAM_READY_MS = Math.max(5, teamNum(args['team-ready-wait-sec'], 150)) * 1000;
const TEAM_SETTLE_MS = Math.max(0, teamNum(args['team-settle-sec'], 6)) * 1000;
const TEAM_REACT_MS = Math.max(3, teamNum(args['team-react-sec'], 60)) * 1000;
const TEAM_FAIL_GRACE_MS = Math.max(2, teamNum(args['team-fail-grace-sec'], 20)) * 1000;
const K_TEAM = {
  toggle: "document.getElementById('btn-expert-toggle')",
  entry: "document.getElementById('btn-ws-dept')",
  menu_last: "(function () { var n = document.querySelectorAll('#ctx-menu .ctx-item'); return n.length ? n[n.length - 1] : null; })()",
  confirm_yes: "document.querySelector('.confirm-overlay .modal .modal-yes')",
  cc_btn: "document.getElementById('btn-cc')",
  cc_alarm_tab: "document.querySelector('#cc-tabs .cc-tab[data-view=\"alarms\"]')",
  cc_close: "document.getElementById('btn-cc-close')",
};

// ==== TEAM-PURE-BEGIN (self-contained: nothing in this block may use anything defined outside it; the local self-check evaluates exactly this text) ====
const TEAM_TIMELINE_MAX = 600;
const TEAM_DAEMON_EVENTS_MAX = 400;
const TEAM_SUB_EVENTS_MAX = 80;

function teamNorm(s) {
  return String(s === undefined || s === null ? '' : s).replace(/[0-9]+/g, 'N');
}
function teamR1(x) {
  return Math.round(Number(x) * 10) / 10;
}
function teamCut(s, n) {
  const t = String(s === undefined || s === null ? '' : s);
  return t.length > n ? t.slice(0, n) : t;
}
function teamHasToken(cls, token) {
  return (' ' + String(cls || '') + ' ').indexOf(' ' + token + ' ') >= 0;
}
// toast elements are "toast <category>": a failure is announced with watchdog (create failures) or health (blocked installer, ...)
function teamIsFailureCls(cls) {
  return teamHasToken(cls, 'watchdog') || teamHasToken(cls, 'health');
}
function teamToastKey(t) {
  return t && t.name ? String(t.name) : '(no name)';
}

function teamNewState() {
  return {
    stage: 'start',
    params: null,
    prev: null,
    last: null,
    polls: 0,
    timeline: [],
    timeline_dropped: 0,
    sub_events: 0,
    seen_before: {}, // toast keys seen in the page before the entry click = baseline (not caused by the team flow)
    entry_ms: null,
    entry_at: null,
    create_ms: null,
    create_at: null,
    create_click_ok: false,
    observe_end_ms: null,
    observe_end_reason: null,
    tab_ids_before: null,
    tab_names_before: null,
    tab_count_before: null,
    tab_ms: null,
    tab_id: null,
    tab_name: null,
    pending_seen: false,
    pending_first_ms: null,
    busy_seen: false,
    stage_texts: [],
    pending_samples: [],
    pending_norms: {},
    last_pending_sample_t: 0,
    stage_events: [],
    daemon_events: [],
    event_counts: {},
    ev_cursor: 0,
    confirm: null,
    menu: null,
    confirm_closed: false,
    ready: null,
    seats: null,
    alarms: null,
    flow_error: null,
  };
}

// what changed between two page snapshots (pageTeamSnapshot): [{ kind, ... }]; prev = null on the first snapshot
function teamDiffSnapshot(prev, cur) {
  const out = [];
  if (!cur) return out;
  const p = prev || null;
  const name = (t) => t.name;
  const pt = p && Array.isArray(p.tabs) ? p.tabs : [];
  const ct = Array.isArray(cur.tabs) ? cur.tabs : [];
  const pById = {};
  pt.forEach((t) => {
    pById[t.id] = t;
  });
  const cById = {};
  ct.forEach((t) => {
    cById[t.id] = t;
  });
  const added = ct.filter((t) => !pById[t.id]);
  const removed = pt.filter((t) => !cById[t.id]);
  const renamed = ct.filter((t) => pById[t.id] && pById[t.id].name !== t.name);
  const activeNow = ct.filter((t) => t.active).map(name).join('|');
  const activeBefore = pt.filter((t) => t.active).map(name).join('|');
  if (!p || added.length || removed.length || renamed.length || activeNow !== activeBefore) {
    out.push({ kind: 'tabs', count: ct.length, names: ct.map(name), added: added.map(name), removed: removed.map(name), active: activeNow });
  }
  ct.forEach((t) => {
    const o = pById[t.id];
    if (o && teamNorm(o.sub) !== teamNorm(t.sub)) out.push({ kind: 'tab_sub', id: t.id, name: t.name, sub: t.sub });
  });
  const rootKey = (r) => JSON.stringify(r ? [r.kind, teamNorm(r.msg), r.stage, r.idle_btn, r.pane_count, r.role_slots, r.pane_titles] : null);
  if (!p || rootKey(p.root) !== rootKey(cur.root)) {
    const r = cur.root || {};
    out.push({ kind: 'root', root: { kind: r.kind, msg: r.msg, stage: r.stage, idle_btn: r.idle_btn, pane_count: r.pane_count, role_slots: r.role_slots, pane_titles: r.pane_titles } });
  }
  const pe = p ? p.entry : null;
  const ce = cur.entry || {};
  if (!pe || pe.exists !== ce.exists || pe.visible !== ce.visible || pe.disabled !== ce.disabled || pe.text !== ce.text) out.push({ kind: 'entry', entry: ce });
  const pg = p ? p.toggle : null;
  const cg = cur.toggle || {};
  if (!pg || pg.exists !== cg.exists || pg.visible !== cg.visible || pg.expanded !== cg.expanded) out.push({ kind: 'toggle', toggle: cg });
  const pcf = p ? p.confirm : null;
  const ccf = cur.confirm || null;
  if (ccf && (!pcf || pcf.title !== ccf.title)) out.push({ kind: 'confirm_open', title: ccf.title, body: ccf.body, buttons: ccf.buttons });
  if (!ccf && pcf) out.push({ kind: 'confirm_close', title: pcf.title });
  const pm = p ? p.menu : null;
  const cm = cur.menu || null;
  if (cm && (!pm || JSON.stringify(pm) !== JSON.stringify(cm))) out.push({ kind: 'menu_open', items: cm.map((i) => i.text) });
  if (!cm && pm) out.push({ kind: 'menu_close' });
  const ptm = {};
  (p && Array.isArray(p.toasts) ? p.toasts : []).forEach((t) => {
    ptm[teamToastKey(t)] = t;
  });
  const ctm = {};
  (Array.isArray(cur.toasts) ? cur.toasts : []).forEach((t) => {
    ctm[teamToastKey(t)] = t;
  });
  (Array.isArray(cur.toasts) ? cur.toasts : []).forEach((t) => {
    const o = ptm[teamToastKey(t)];
    if (!o) out.push({ kind: 'toast_new', cls: t.cls, name: t.name, detail: t.detail });
    else if (o.detail !== t.detail || o.cls !== t.cls) out.push({ kind: 'toast_text', cls: t.cls, name: t.name, detail: t.detail, prev_detail: o.detail });
  });
  (p && Array.isArray(p.toasts) ? p.toasts : []).forEach((t) => {
    if (!ctm[teamToastKey(t)]) out.push({ kind: 'toast_gone', cls: t.cls, name: t.name });
  });
  if (!p || p.daemon_info !== cur.daemon_info) out.push({ kind: 'daemon_info', text: cur.daemon_info });
  if (p && p.cc_open !== cur.cc_open) out.push({ kind: 'cc', open: !!cur.cc_open });
  return out;
}

// feed one page snapshot into the state: time table (only changes), baseline toasts, waiting-screen facts, new-tab detection.
// Returns { new_tab, first_stage_text, changes } so that the caller knows when to take a screenshot.
function teamNote(S, snap) {
  const res = { new_tab: false, first_stage_text: false, changes: 0 };
  if (!snap) return res;
  S.polls++;
  const t = typeof snap.t === 'number' ? snap.t : Date.now();
  const changes = teamDiffSnapshot(S.prev, snap);
  for (const ch of changes) {
    if (ch.kind === 'tab_sub') {
      if (S.sub_events >= TEAM_SUB_EVENTS_MAX) continue;
      S.sub_events++;
    }
    if (S.timeline.length >= TEAM_TIMELINE_MAX) {
      S.timeline_dropped++;
      continue;
    }
    const rec = { t, ms_entry: S.entry_ms === null ? null : t - S.entry_ms, ms_create: S.create_ms === null ? null : t - S.create_ms };
    for (const k of Object.keys(ch)) rec[k] = ch[k];
    S.timeline.push(rec);
    res.changes++;
  }
  if (S.entry_ms === null) {
    for (const x of Array.isArray(snap.toasts) ? snap.toasts : []) S.seen_before[teamToastKey(x)] = true;
  }
  const root = snap.root || {};
  if (S.create_ms !== null) {
    if (root.kind === 'pending' && !S.pending_seen) {
      S.pending_seen = true;
      S.pending_first_ms = t - S.create_ms;
    }
    if (snap.entry && snap.entry.disabled && !S.busy_seen) S.busy_seen = true;
    if (root.stage && (S.stage_texts.length === 0 || S.stage_texts[S.stage_texts.length - 1].text !== root.stage)) {
      S.stage_texts.push({ ms: t - S.create_ms, text: String(root.stage) });
      if (S.stage_texts.length === 1) res.first_stage_text = true;
    }
    if (S.tab_ids_before !== null && S.tab_ms === null) {
      const fresh = (Array.isArray(snap.tabs) ? snap.tabs : []).filter((x) => S.tab_ids_before.indexOf(x.id) < 0)[0];
      if (fresh) {
        S.tab_ms = t;
        S.tab_id = fresh.id;
        S.tab_name = fresh.name;
        res.new_tab = true;
      }
    }
    if (root.kind === 'pending') {
      S.pending_norms[teamNorm(root.msg)] = true;
      if (t - S.last_pending_sample_t >= 15000 && S.pending_samples.length < 40) {
        S.last_pending_sample_t = t;
        S.pending_samples.push({ ms_create: t - S.create_ms, msg: root.msg, stage: root.stage });
      }
    }
  }
  S.prev = snap;
  S.last = snap;
  return res;
}

// events pulled from the page (pagePollTeamEvents): 'dept-create-progress' (the pack's stage markers) and 'daemon-event' (raw, capped)
function teamNoteEvents(S, evs) {
  for (const e of Array.isArray(evs) ? evs : []) {
    if (!e || typeof e !== 'object') continue;
    const ms = S.create_ms === null ? null : e.t - S.create_ms;
    if (e.n === 'dept-create-progress') {
      const p = e.p && typeof e.p === 'object' ? e.p : {};
      const rec = {
        t: e.t,
        ms_create: ms,
        id: p.id === undefined || p.id === null ? null : String(p.id),
        stage: p.stage === undefined || p.stage === null ? null : String(p.stage),
        raw: teamCut(JSON.stringify(e.p === undefined ? null : e.p), 300),
      };
      S.stage_events.push(rec);
      if (S.timeline.length < TEAM_TIMELINE_MAX) S.timeline.push({ t: e.t, ms_entry: S.entry_ms === null ? null : e.t - S.entry_ms, ms_create: ms, kind: 'stage_event', stage: rec.stage, id: rec.id, raw: rec.raw });
    } else if (e.n === 'daemon-event') {
      if (S.daemon_events.length < TEAM_DAEMON_EVENTS_MAX) {
        S.daemon_events.push({ t: e.t, ms_create: ms, name: e.name, category: e.category, slug: e.slug, sid: e.sid, kind: e.kind, title: e.title, body: e.body });
      }
      if (e.kind && S.timeline.length < TEAM_TIMELINE_MAX) {
        S.timeline.push({ t: e.t, ms_entry: S.entry_ms === null ? null : e.t - S.entry_ms, ms_create: ms, kind: 'daemon_event', name: e.name, category: e.category, event_kind: e.kind, title: e.title, body: e.body });
      }
    }
  }
}

// ordered stage list with the seconds each stage lasted (until the next marker; the last one until the real tab stood)
function teamStageSummary(S) {
  const evs = S.stage_events.filter((e) => e.ms_create !== null).slice().sort((a, b) => a.t - b.t);
  const endMs = S.tab_ms !== null && S.create_ms !== null ? S.tab_ms - S.create_ms : null;
  const out = [];
  for (let i = 0; i < evs.length; i++) {
    const cur = evs[i];
    let next = null;
    if (i + 1 < evs.length) next = evs[i + 1].ms_create;
    else if (endMs !== null && endMs >= cur.ms_create) next = endMs;
    out.push({ stage: cur.stage, at_s: teamR1(cur.ms_create / 1000), dur_s: next === null ? null : teamR1((next - cur.ms_create) / 1000) });
  }
  return out;
}

function teamStageLine(stages) {
  if (!stages.length) return 'no stage events';
  return stages.map((s) => s.stage + ' at ' + s.at_s + 's' + (s.dur_s === null ? '' : ' (' + s.dur_s + 's)')).join(' > ');
}

// new notifications since the entry click, sorted: failure-like (watchdog / health) before the real tab stood = failure alerts (they count
// against PASS); failure-like after the tab stood = post-creation notices (reference: e.g. "members could not be started");
// everything else = other. Toasts that were already on the screen before the click (baseline, by title) never count.
function teamClassifyAlerts(S) {
  const failure = [];
  const post = [];
  const other = [];
  const end = S.observe_end_ms;
  for (const r of S.timeline) {
    if (r.kind !== 'toast_new') continue;
    if (S.entry_ms === null || r.t < S.entry_ms) continue;
    if (end !== null && r.t > end) continue;
    if (S.seen_before[r.name ? String(r.name) : '(no name)']) continue;
    const item = { t: r.t, ms_create: r.ms_create, cls: r.cls, name: r.name, detail: r.detail };
    if (!teamIsFailureCls(r.cls)) other.push(item);
    else if (S.tab_ms !== null && r.t >= S.tab_ms) post.push(item);
    else failure.push(item);
  }
  return { failure, post_creation: post, other };
}

// the time table as text: one line per entry, sorted by time (seconds after the execute click, seconds after the entry click, kind, details).
// Texts are verbatim; a line cuts them at 400 characters (the JSON records have them whole).
function teamTimelineText(S) {
  const rows = S.timeline.slice().sort((a, b) => a.t - b.t);
  const sec = (ms) => (ms === null || ms === undefined ? '      ---' : (ms < 0 ? '-' : '+') + String(teamR1(Math.abs(ms) / 1000)).padStart(7) + 's');
  const one = (v) => teamCut(String(v === undefined || v === null ? '' : v).replace(/\s+/g, ' '), 400);
  const lines = ['# TEAM scene time table (changes only). columns: seconds after the execute click | seconds after the entry click | kind | details (texts verbatim, cut at 400 characters here)'];
  for (const r of rows) {
    let d = '';
    if (r.kind === 'tabs') d = 'count=' + r.count + ' names=[' + r.names.join('|') + '] added=[' + r.added.join('|') + '] removed=[' + r.removed.join('|') + '] active=' + r.active;
    else if (r.kind === 'tab_sub') d = r.name + ' :: ' + one(r.sub);
    else if (r.kind === 'root') d = 'kind=' + r.root.kind + ' msg="' + one(r.root.msg) + '" stage="' + one(r.root.stage) + '" idle_btn="' + one(r.root.idle_btn) + '" panes=' + r.root.pane_count + ' role_slots=' + r.root.role_slots + ' titles=[' + (r.root.pane_titles || []).join('|') + ']';
    else if (r.kind === 'entry') d = 'exists=' + r.entry.exists + ' visible=' + r.entry.visible + ' disabled=' + r.entry.disabled + ' text="' + one(r.entry.text) + '"';
    else if (r.kind === 'toggle') d = 'exists=' + r.toggle.exists + ' visible=' + r.toggle.visible + ' expanded=' + r.toggle.expanded;
    else if (r.kind === 'confirm_open') d = 'title="' + one(r.title) + '" buttons=[' + (r.buttons || []).map((b) => b.text).join('|') + '] body="' + one(r.body) + '"';
    else if (r.kind === 'confirm_close') d = 'title="' + one(r.title) + '"';
    else if (r.kind === 'menu_open') d = 'items=[' + (r.items || []).join(' | ') + ']';
    else if (r.kind === 'toast_new' || r.kind === 'toast_text') d = '[' + r.cls + '] ' + one(r.name) + ' :: ' + one(r.detail);
    else if (r.kind === 'toast_gone') d = '[' + r.cls + '] ' + one(r.name);
    else if (r.kind === 'stage_event') d = 'stage=' + r.stage + ' id=' + r.id + ' raw=' + r.raw;
    else if (r.kind === 'daemon_event') d = r.name + ' [' + r.category + '] kind=' + r.event_kind + ' title="' + one(r.title) + '" body="' + one(r.body) + '"';
    else if (r.kind === 'daemon_info') d = one(r.text);
    else if (r.kind === 'cc') d = 'open=' + r.open;
    lines.push(sec(r.ms_create) + ' ' + sec(r.ms_entry) + ' ' + r.kind.padEnd(13) + ' ' + d);
  }
  if (S.timeline_dropped > 0) lines.push('# ' + S.timeline_dropped + ' more entries were dropped (cap ' + TEAM_TIMELINE_MAX + ')');
  return lines.join('\r\n') + '\r\n';
}

function teamSeatsFrom(ls) {
  if (!ls || ls.ok !== true) return { ok: false, error: ls && ls.error ? teamCut(ls.error, 300) : 'no result', count: null, roles: [], surfaces: [] };
  const v = ls.value;
  const arr = Array.isArray(v) ? v : v && Array.isArray(v.surfaces) ? v.surfaces : null;
  if (!arr) return { ok: false, error: 'unexpected shape: ' + teamCut(JSON.stringify(v), 300), count: null, roles: [], surfaces: [] };
  const surfaces = arr.slice(0, 30).map((s) => {
    const o = {};
    if (s && typeof s === 'object') {
      for (const k of Object.keys(s)) {
        const x = s[k];
        if (typeof x === 'string') o[k] = teamCut(x, 120);
        else if (typeof x === 'number' || typeof x === 'boolean' || x === null) o[k] = x;
      }
    }
    return o;
  });
  const roles = arr.map((s) => (s && typeof s === 'object' && s.role !== undefined && s.role !== null ? String(s.role) : '(none)'));
  return { ok: true, count: arr.length, roles, surfaces };
}

function teamCompactValue(v, max) {
  let s;
  try {
    s = JSON.stringify(v);
  } catch (e) {
    s = String(v);
  }
  if (s === undefined) return null;
  return s.length > max ? { truncated: true, length: s.length, head: s.slice(0, max) } : v;
}

// the registry key of the team that was just created: a key that was not there before (the tab name when it is one of them)
function teamPickNewDept(beforeKeys, depts, tabName) {
  const d = depts && typeof depts === 'object' ? depts : {};
  const before = Array.isArray(beforeKeys) ? beforeKeys : [];
  const fresh = Object.keys(d).filter((k) => before.indexOf(k) < 0);
  if (tabName && fresh.indexOf(tabName) >= 0) return tabName;
  return fresh.length ? fresh[0] : null;
}

function teamParseAlarms(items) {
  return (Array.isArray(items) ? items : []).map((it) => {
    const parts = String((it && it.meta) || '').split(' \u00b7 ');
    return { time: parts[0] || null, category: parts[1] || null, id: parts.length > 2 ? parts.slice(2).join(' \u00b7 ') : null, title: it ? it.title : null, body: it ? it.body : null, cls: it ? it.cls : null };
  });
}

// facts for the verdict (node part) + reference facts (never used for the verdict) from the recorded state and the click records
function teamBuildFacts(S, clicks) {
  const find = (label) => {
    const l = (clicks || []).filter((c) => c && c.label === label);
    return l.length ? l[l.length - 1] : null;
  };
  const entry = find('team entry button');
  const toggle = find('expert toggle');
  const menu = find('team menu item');
  const yes = find('team confirm yes');
  const al = teamClassifyAlerts(S);
  const stages = teamStageSummary(S);
  const last = S.last || {};
  const entryOk = !!(entry && entry.ok);
  const confirmSeen = !!S.confirm;
  const yesOk = !!(yes && yes.ok);
  const tabCreated = S.tab_ms !== null;
  const flowStarted = yesOk && (S.pending_seen || tabCreated || S.busy_seen);
  const rolledBack = S.pending_seen && !tabCreated && !!last.root && last.root.kind !== 'pending';
  const why = S.flow_error ? ' (' + S.flow_error + ')' : '';
  const missing = [];
  if (!entryOk) missing.push('the team button could not be clicked' + (entry && entry.error ? ' (' + entry.error + ')' : why));
  if (entryOk && !confirmSeen) missing.push('no confirm window appeared after the click' + why);
  if (confirmSeen && !yesOk) missing.push('the execute button of the confirm window could not be clicked' + (yes && yes.error ? ' (' + yes.error + ')' : why));
  if (yesOk && !flowStarted) missing.push('the flow did not start: no waiting screen, no busy button and no new tab after the execute click');
  if (flowStarted && !tabCreated) missing.push('no new team tab stood' + (rolledBack ? ' (the waiting screen went away without one: the creation failed)' : ' within the observation window'));
  for (const a of al.failure) missing.push('failure notification [' + a.cls + '] ' + a.name + ' :: ' + teamCut(a.detail, 300));
  const tabsAfter = Array.isArray(last.tabs) ? last.tabs : [];
  const facts = {
    ready_ok: !!(S.ready && S.ready.ok),
    entry_found: !!(entry && entry.find && entry.find.found),
    entry_click_ok: entryOk,
    entry_click_method: entry ? entry.method : null,
    toggle_click_method: toggle ? toggle.method : null,
    menu_seen: !!S.menu,
    menu_picked: S.menu ? S.menu.picked_text : null,
    menu_click_ok: !!(menu && menu.ok),
    confirm_seen: confirmSeen,
    confirm_title: S.confirm ? S.confirm.title : null,
    confirm_click_ok: yesOk,
    confirm_click_method: yes ? yes.method : null,
    confirm_closed: !!S.confirm_closed,
    any_dom_click: [toggle, entry, menu, yes].some((c) => !!c && c.method === 'dom_click'),
    flow_started: flowStarted,
    pending_seen: S.pending_seen,
    pending_first_ms: S.pending_first_ms,
    busy_button_seen: S.busy_seen,
    tab_created: tabCreated,
    tab_ms: S.tab_ms !== null && S.create_ms !== null ? S.tab_ms - S.create_ms : null,
    tab_name: S.tab_name,
    tab_id: S.tab_id,
    tab_count_before: S.tab_count_before,
    tab_names_before: S.tab_names_before,
    tab_count_after: tabsAfter.length,
    tab_names_after: tabsAfter.map((t) => t.name),
    rolled_back: rolledBack,
    failure_alert_count: al.failure.length,
    failure_alerts: al.failure,
    post_creation_alert_count: al.post_creation.length,
    post_creation_alerts: al.post_creation,
    other_new_toast_count: al.other.length,
    stage_event_count: S.stage_events.length,
    stage_source: S.stage_events.length ? 'event' : S.stage_texts.length ? 'text' : 'none',
    observe_end_reason: S.observe_end_reason,
    flow_error: S.flow_error,
  };
  // reference: what the screen / the pack said
  const post = S.timeline.filter((r) => (r.kind === 'toast_new' || r.kind === 'toast_text') && S.tab_ms !== null && r.t >= S.tab_ms && (S.observe_end_ms === null || r.t <= S.observe_end_ms) && !S.seen_before[r.name ? String(r.name) : '(no name)']);
  const formationLine = post.length
    ? post.slice(0, 8).map((r) => (r.ms_create === null ? '?' : teamR1(r.ms_create / 1000)) + 's [' + r.cls + '] ' + teamCut(r.name, 80) + ' :: ' + teamCut(r.detail, 200)).join(' | ')
    : 'no notification after the tab stood';
  const seats = S.seats || null;
  const seatsLine = seats ? (seats.ok ? 'count=' + seats.count + ' roles=[' + seats.roles.join(',') + ']' : 'not available: ' + seats.error) : 'not read';
  const ps = S.pending_samples;
  const tabLine = tabCreated
    ? 'tab ' + JSON.stringify(S.tab_name) + ' stood ' + teamR1(facts.tab_ms / 1000) + ' s after the execute click (tabs ' + facts.tab_count_before + ' -> ' + facts.tab_count_after + ')'
    : 'no new tab (tabs ' + facts.tab_count_before + ' -> ' + facts.tab_count_after + ')';
  const reference = {
    stages,
    stage_line: teamStageLine(stages),
    stage_source: facts.stage_source,
    stage_texts: S.stage_texts.slice(0, 20),
    tab_line: tabLine,
    confirm: S.confirm ? { title: S.confirm.title, body_head: teamCut(S.confirm.body, 600), buttons: (S.confirm.buttons || []).map((b) => b.text) } : null,
    menu: S.menu,
    pending_first_text: ps.length ? ps[0].msg : null,
    pending_last_text: ps.length ? ps[ps.length - 1].msg : null,
    pending_text_variants: Object.keys(S.pending_norms).length,
    post_creation_notices: post.slice(0, 20).map((r) => ({ ms_create: r.ms_create, kind: r.kind, cls: r.cls, name: r.name, detail: r.detail })),
    formation_line: formationLine,
    seats,
    seats_line: seatsLine,
    event_counts: S.event_counts,
    alarm_count: S.alarms && S.alarms.items ? S.alarms.items.length : null,
    alarm_id_count: S.alarms && S.alarms.items ? S.alarms.items.filter((a) => a.id).length : null,
    alarm_ids: S.alarms && S.alarms.items ? S.alarms.items.filter((a) => a.id).slice(0, 40).map((a) => ({ id: a.id, category: a.category, title: a.title })) : [],
    final_daemon_info: last.daemon_info === undefined ? null : last.daemon_info,
    final_root: last.root || null,
  };
  return { facts, reference, node_ok: missing.length === 0, node_missing: missing };
}
// ==== TEAM-PURE-END ====

// ==== TEAM-PAGE-BEGIN (functions that run INSIDE the page: they are stringified into Runtime.evaluate expressions, so each must be self-contained) ====
// one JSON snapshot of everything the team scene looks at (selectors: see the block comment above)
function pageTeamSnapshot() {
  var txt = function (el) {
    return el && el.textContent ? String(el.textContent).replace(/\s+/g, ' ').trim() : '';
  };
  var q = function (s, root) {
    return (root || document).querySelector(s);
  };
  var qa = function (s, root) {
    return Array.prototype.slice.call((root || document).querySelectorAll(s));
  };
  var clsOf = function (el) {
    return String(el && el.className ? el.className : '');
  };
  var hasCls = function (el, c) {
    return (' ' + clsOf(el) + ' ').indexOf(' ' + c + ' ') >= 0;
  };
  var vis = function (el) {
    if (!el) return false;
    var r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  var cut = function (s, n) {
    var t = String(s === undefined || s === null ? '' : s);
    return t.length > n ? t.slice(0, n) : t;
  };
  var btn = function (b) {
    return { text: txt(b), cls: clsOf(b), disabled: !!b.disabled };
  };
  var o = { t: Date.now() };
  o.title = document.title;
  o.daemon_info = cut(txt(document.getElementById('daemon-info')), 200);
  var tg = document.getElementById('btn-expert-toggle');
  o.toggle = tg ? { exists: true, visible: vis(tg), expanded: tg.getAttribute('aria-expanded') } : { exists: false, visible: false, expanded: null };
  var en = document.getElementById('btn-ws-dept');
  o.entry = en ? { exists: true, visible: vis(en), disabled: !!en.disabled, text: cut(txt(en), 80) } : { exists: false, visible: false, disabled: false, text: '' };
  o.tabs = qa('#ws-tabs .ws-tab').map(function (tb) {
    return {
      id: tb.getAttribute('data-ws-id'),
      name: cut(txt(q('.ws-name', tb)), 120),
      sub: cut(txt(q('.ws-sub', tb)), 160),
      active: hasCls(tb, 'active'),
      busy: tb.getAttribute('aria-busy') === 'true',
    };
  });
  o.tab_count = o.tabs.length;
  var rootEl = document.getElementById('root');
  var panes = rootEl ? qa('.pane', rootEl) : [];
  var big = panes.filter(function (p) {
    return hasCls(p, 'dept-pending') && !hasCls(p, 'role-slot');
  })[0] || null;
  var seatPanes = panes.filter(function (p) {
    return p.getAttribute('data-sid') !== null && !hasCls(p, 'dept-pending');
  });
  var kind = 'empty';
  if (big) kind = big.getAttribute('aria-busy') === 'true' ? 'pending' : 'idle';
  else if (panes.length) kind = 'panes';
  o.root = {
    kind: kind,
    msg: big ? cut(txt(q('.dept-pending-msg', big)), 600) : '',
    stage: big ? cut(txt(q('.dept-pending-stage', big)), 300) : '',
    idle_btn: big ? cut(txt(q('.dept-idle-btn', big)), 80) : '',
    pane_count: seatPanes.length,
    role_slots: panes.filter(function (p) {
      return hasCls(p, 'role-slot');
    }).length,
    pane_titles: seatPanes.slice(0, 8).map(function (p) {
      return cut(txt(q('.pane-title-text', p)), 80);
    }),
  };
  var cm = q('.confirm-overlay .modal');
  o.confirm = cm
    ? { title: cut(txt(q('h3', cm)), 200), body: cut(q('p', cm) ? q('p', cm).textContent : '', 6000), buttons: qa('.modal-btns button', cm).map(btn) }
    : null;
  var mn = document.getElementById('ctx-menu');
  o.menu = mn
    ? qa('.ctx-item', mn).map(function (it) {
        return { text: cut(txt(it), 160), disabled: hasCls(it, 'disabled') };
      })
    : null;
  o.overlays = qa('.modal-overlay').length;
  o.toasts = qa('#toasts > *').map(function (e) {
    return { cls: clsOf(e), name: cut(txt(q('.toast-name', e)), 400), detail: cut(txt(q('.toast-detail', e)), 4000) };
  });
  var cc = document.getElementById('cc-panel');
  o.cc_open = !!cc && !cc.hidden;
  return JSON.stringify(o);
}

// what the page looks like when a step failed (selectors may have changed): the sidebar and the waiting area as HTML heads
function pageTeamDomSummary() {
  var txt = function (el) {
    return el && el.textContent ? String(el.textContent).replace(/\s+/g, ' ').trim() : '';
  };
  var qa = function (s) {
    return Array.prototype.slice.call(document.querySelectorAll(s));
  };
  var cut = function (s, n) {
    var t = String(s === undefined || s === null ? '' : s);
    return t.length > n ? t.slice(0, n) : t;
  };
  var one = function (sel, n) {
    var e = document.querySelector(sel);
    return e ? cut(e.outerHTML, n) : null;
  };
  var rootEl = document.getElementById('root');
  var menu = document.getElementById('ctx-menu');
  return JSON.stringify({
    href: location.href,
    title: document.title,
    daemon_info: cut(txt(document.getElementById('daemon-info')), 200),
    wsbar_head: one('#wsbar-head', 1500),
    ws_tabs: one('#ws-tabs', 4000),
    wsbar_expert: one('#wsbar-expert', 3000),
    root_head: rootEl ? cut(rootEl.innerHTML, 3000) : null,
    top_buttons: qa('#topbar button').map(function (b) {
      return { id: b.id, text: cut(txt(b), 60), disabled: !!b.disabled };
    }),
    all_button_ids: qa('button[id]').map(function (b) {
      return b.id;
    }).slice(0, 80),
    overlays: qa('.modal-overlay').map(function (e) {
      return { cls: String(e.className || ''), text: cut(txt(e), 600) };
    }),
    menu: menu ? cut(txt(menu), 600) : null,
    toasts: qa('#toasts > *').map(function (e) {
      return cut(txt(e), 600);
    }),
    body_text_head: cut(document.body ? document.body.innerText : '', 3000),
  });
}

// listeners for the Tauri events of the team flow: 'dept-create-progress' (payload {id, stage}) and 'daemon-event' (raw, compact)
async function pageTeamListeners() {
  window.__diagTeam = window.__diagTeam || { events: [], counts: {}, listen: {} };
  var D = window.__diagTeam;
  var reg = async function (n) {
    if (D.listen[n]) return;
    try {
      await window.__TAURI__.event.listen(n, function (e) {
        try {
          var p = e && e.payload !== undefined ? e.payload : null;
          var r = { t: Date.now(), n: n };
          if (n === 'dept-create-progress') {
            r.p = p;
          } else {
            var o = p && typeof p === 'object' ? p : {};
            var pl = o.payload && typeof o.payload === 'object' ? o.payload : {};
            r.name = String(o.name === undefined || o.name === null ? '' : o.name).slice(0, 120);
            r.category = String(o.category === undefined || o.category === null ? '' : o.category).slice(0, 60);
            r.slug = o.socket_slug === undefined || o.socket_slug === null ? null : String(o.socket_slug).slice(0, 120);
            r.sid = o.surface_id === undefined ? null : o.surface_id;
            r.kind = pl.kind === undefined || pl.kind === null ? null : String(pl.kind).slice(0, 80);
            r.title = pl.title === undefined || pl.title === null ? null : String(pl.title).slice(0, 300);
            r.body = pl.body === undefined || pl.body === null ? null : String(pl.body).slice(0, 1200);
          }
          var ck = n === 'daemon-event' ? 'daemon-event:' + r.name : n;
          D.counts[ck] = (D.counts[ck] || 0) + 1;
          if (n === 'dept-create-progress' || D.events.length < 700) D.events.push(r);
        } catch (x) {}
      });
      D.listen[n] = 'ok';
    } catch (x) {
      D.listen[n] = 'fail: ' + String(x);
    }
  };
  await reg('dept-create-progress');
  await reg('daemon-event');
  return JSON.stringify(D.listen);
}

function pagePollTeamEvents(from) {
  var D = window.__diagTeam;
  if (!D) return JSON.stringify({ n: 0, ev: [], counts: {} });
  return JSON.stringify({ n: D.events.length, ev: D.events.slice(from || 0), counts: D.counts });
}

// one read-only call of the product's own commands (list_depts, read_dept_catalog, list_surfaces)
async function pageInvoke(cmd, args) {
  try {
    var v = await window.__TAURI__.core.invoke(cmd, args);
    return JSON.stringify({ ok: true, value: v === undefined ? null : v });
  } catch (e) {
    return JSON.stringify({ ok: false, error: e && e.message ? e.message : typeof e === 'string' ? e : JSON.stringify(e) });
  }
}

// the alarm history list of the Control Center ("category + id" per entry is the only place where the sticky toast ids are visible)
function pageTeamAlarms() {
  var txt = function (el) {
    return el && el.textContent ? String(el.textContent).replace(/\s+/g, ' ').trim() : '';
  };
  var q = function (s, root) {
    return (root || document).querySelector(s);
  };
  var items = Array.prototype.slice.call(document.querySelectorAll('#cc-alarm-list .alarm-item')).slice(0, 200);
  return JSON.stringify(
    items.map(function (it) {
      var b = txt(q('.al-body', it));
      return { cls: String(it.className || ''), meta: txt(q('.al-meta', it)), title: txt(q('.al-title', it)), body: b.length > 2000 ? b.slice(0, 2000) : b };
    }),
  );
}
// ==== TEAM-PAGE-END ====
const EXPR_TEAM_SNAPSHOT = '(' + pageTeamSnapshot.toString() + ')()';
const EXPR_TEAM_DOM_SUMMARY = '(' + pageTeamDomSummary.toString() + ')()';
const EXPR_TEAM_LISTENERS = '(' + pageTeamListeners.toString() + ')()';
const EXPR_TEAM_ALARMS = '(' + pageTeamAlarms.toString() + ')()';
const exprTeamEvents = (from) => '(' + pagePollTeamEvents.toString() + ')(' + (Number(from) || 0) + ')';
const exprTeamInvoke = (cmd, a) => '(' + pageInvoke.toString() + ')(' + JSON.stringify(String(cmd)) + ', ' + JSON.stringify(a || {}) + ')';

// ---- team scene: the runner (uses uiClick / uiShot / evalJs / save / step / fail of the blocks above) ----
function teamWriteLive(S) {
  try {
    const o = {
      t: iso(),
      stage: S.stage,
      entry_at: S.entry_at,
      team_clicked_at: S.create_click_ok ? S.create_at : null,
      tab_created: S.tab_ms !== null,
      tab_count: S.last && typeof S.last.tab_count === 'number' ? S.last.tab_count : null,
      pending_seen: S.pending_seen,
      polls: S.polls,
    };
    fs.writeFileSync(path.join(OUT, `${PREFIX}-live.json`), JSON.stringify(o));
  } catch (e) {
    // ignore: only a convenience for the PowerShell watch loop
  }
}

function teamWriteFacts(S, partial) {
  try {
    const fa = teamBuildFacts(S, R.ui ? R.ui.clicks : []);
    S.facts = fa.facts;
    S.reference = fa.reference;
    S.node_ok = fa.node_ok;
    S.node_missing = fa.node_missing;
    const o = {
      script: 'cdp-update.mjs',
      mode: MODE,
      written: iso(),
      partial: !!partial,
      attached: R.attached,
      app_version: R.app_version,
      team_clicked_at: S.create_click_ok ? S.create_at : null,
      node_ok: fa.node_ok,
      node_missing: fa.node_missing,
      facts: fa.facts,
      reference: fa.reference,
    };
    fs.writeFileSync(path.join(OUT, `${PREFIX}-facts.json`), JSON.stringify(o, null, 1));
    fs.writeFileSync(path.join(OUT, `${PREFIX}-ui-timeline.txt`), teamTimelineText(S));
  } catch (e) {
    fail('team facts', e);
  }
}

// one poll: page snapshot -> state (time table, baseline, new tab ...), then the events that arrived in the page
async function teamPoll(S) {
  const snap = parseJsonValue(await evalJs(EXPR_TEAM_SNAPSHOT, { awaitPromise: false, timeoutMs: 12000 }));
  if (!snap || snap.ok === false) return null;
  const r = teamNote(S, snap);
  try {
    const o = parseJsonValue(await evalJs(exprTeamEvents(S.ev_cursor), { awaitPromise: false, timeoutMs: 8000 }));
    if (o && Array.isArray(o.ev)) {
      S.ev_cursor = typeof o.n === 'number' ? o.n : S.ev_cursor;
      teamNoteEvents(S, o.ev);
      if (o.counts && typeof o.counts === 'object') S.event_counts = o.counts;
    }
  } catch (e) {
    // the events are an extra: a failed pull never stops the scene
  }
  teamWriteLive(S);
  return { snap, r };
}

async function teamWait(S, pred, timeoutMs) {
  const t0 = Date.now();
  let snap = S.last;
  while (Date.now() - t0 < timeoutMs && !closed) {
    try {
      const r = await teamPoll(S);
      if (r) {
        snap = r.snap;
        if (pred(snap)) return { ok: true, snap, waited_ms: Date.now() - t0 };
      }
    } catch (e) {
      if (closed) break;
    }
    await sleep(500);
  }
  return { ok: false, snap, waited_ms: Date.now() - t0, closed };
}

// after the entry click: a confirm window (or the menu that comes first); a failure-like notification that stays 6 s without either = blocked
async function teamWaitReaction(S, timeoutMs) {
  const t0 = Date.now();
  let toastSince = null;
  while (Date.now() - t0 < timeoutMs && !closed) {
    let r = null;
    try {
      r = await teamPoll(S);
    } catch (e) {
      if (closed) break;
    }
    if (r) {
      const s = r.snap;
      if (s.confirm) return { ok: true, kind: 'confirm', snap: s, waited_ms: Date.now() - t0 };
      if (s.menu) return { ok: true, kind: 'menu', snap: s, waited_ms: Date.now() - t0 };
      const fresh = (Array.isArray(s.toasts) ? s.toasts : []).filter((x) => !S.seen_before[teamToastKey(x)] && teamIsFailureCls(x.cls));
      if (fresh.length && toastSince === null) toastSince = Date.now();
      if (toastSince !== null && Date.now() - toastSince >= 6000) return { ok: false, kind: 'notification', snap: s, waited_ms: Date.now() - t0 };
    }
    await sleep(500);
  }
  return { ok: false, kind: 'none', snap: S.last, waited_ms: Date.now() - t0, closed };
}

async function teamInvoke(cmd, a) {
  try {
    return parseJsonValue(await evalJs(exprTeamInvoke(cmd, a), { timeoutMs: 30000 }));
  } catch (e) {
    return { ok: false, error: String(e && e.message ? e.message : e) };
  }
}

async function teamDomSummary(S, tag) {
  try {
    const s = parseJsonValue(await evalJs(EXPR_TEAM_DOM_SUMMARY, { awaitPromise: false, timeoutMs: 15000 }));
    const file = `${PREFIX}-team-dom-${tag}.json`;
    fs.writeFileSync(path.join(OUT, file), JSON.stringify(s, null, 2));
    S.dom_summaries = S.dom_summaries || [];
    S.dom_summaries.push({ tag, file, t: iso() });
    save();
  } catch (e) {
    fail('team dom summary ' + tag, e);
  }
}

async function teamSeats(S) {
  S.stage = 'seats';
  try {
    const reg = await teamInvoke('list_depts', {});
    const depts = reg.ok && reg.value && typeof reg.value.depts === 'object' && reg.value.depts ? reg.value.depts : null;
    const key = depts ? teamPickNewDept(S.before_dept_keys, depts, S.tab_name) : null;
    const entry = key && depts[key] ? depts[key] : null;
    const sock = entry && typeof entry.socket === 'string' ? entry.socket : null;
    S.seats_dept = { registry_ok: !!reg.ok, registry_error: reg.ok ? null : reg.error, key, socket: sock, entry: entry ? teamCompactValue(entry, 1500) : null, dept_keys_after: depts ? Object.keys(depts) : null };
    if (sock) {
      const ls = await teamInvoke('list_surfaces', { socket: sock });
      S.seats = teamSeatsFrom(ls);
      S.seats.raw = ls.ok ? teamCompactValue(ls.value, 6000) : null;
    } else {
      S.seats = { ok: false, error: key ? 'the registry entry of ' + key + ' has no socket' : 'no new team in the registry', count: null, roles: [], surfaces: [] };
    }
  } catch (e) {
    S.seats = { ok: false, error: 'exception: ' + String(e && e.message ? e.message : e), count: null, roles: [], surfaces: [] };
  }
  step('team: seats', { ok: S.seats.ok, count: S.seats.count, roles: S.seats.roles });
}

// the alarm history of the Control Center: the only place where the ids of the sticky notifications are visible (best effort)
async function teamAlarms(S) {
  S.stage = 'alarms';
  S.alarms = { ok: false, note: null, items: [] };
  try {
    const o1 = await uiClick(K_TEAM.cc_btn, 'cc button', false);
    if (!o1.ok) {
      S.alarms.note = 'the Control Center button could not be clicked: ' + (o1.error || 'unknown');
      return;
    }
    const w = await teamWait(S, (s) => !!s.cc_open, 8000);
    if (!w.ok) {
      S.alarms.note = 'the Control Center did not open';
      return;
    }
    const o2 = await uiClick(K_TEAM.cc_alarm_tab, 'cc alarms tab', false);
    if (!o2.ok) {
      S.alarms.note = 'the alarms tab could not be clicked: ' + (o2.error || 'unknown');
    } else {
      await sleep(800);
      const raw = parseJsonValue(await evalJs(EXPR_TEAM_ALARMS, { awaitPromise: false, timeoutMs: 12000 }));
      S.alarms.items = teamParseAlarms(Array.isArray(raw) ? raw : []);
      S.alarms.ok = Array.isArray(raw);
      if (!S.alarms.ok) S.alarms.note = 'the alarm list could not be read: ' + teamCut(JSON.stringify(raw), 200);
      await uiShot('alarms');
    }
    await uiClick(K_TEAM.cc_close, 'cc close', false);
  } catch (e) {
    S.alarms.note = 'exception: ' + String(e && e.message ? e.message : e);
  }
  step('team: alarms', { ok: S.alarms.ok, count: S.alarms.items.length, note: S.alarms.note });
}

async function teamObserve(S) {
  S.stage = 'observe';
  const t0 = S.create_ms;
  step('team: observing', { observe_ms: TEAM_OBSERVE_MS });
  let shotAfter = false;
  let shotStage = false;
  let shotTab = false;
  let failSince = null;
  let lastFacts = Date.now();
  while (Date.now() - t0 < TEAM_OBSERVE_MS && !closed) {
    try {
      await teamPoll(S);
    } catch (e) {
      if (closed) break;
    }
    const el = Date.now() - t0;
    if (!shotAfter && el >= 1500) {
      shotAfter = true;
      await uiShot('after-click');
    }
    if (!shotStage && (S.stage_texts.length > 0 || S.stage_events.length > 0)) {
      shotStage = true;
      await uiShot('first-stage');
    }
    if (!shotTab && S.tab_ms !== null) {
      shotTab = true;
      await sleep(700);
      await uiShot('new-tab');
    }
    const root = S.last && S.last.root ? S.last.root : {};
    const started = S.pending_seen || S.busy_seen || S.tab_ms !== null;
    if (!started && el >= 30000) {
      S.observe_end_reason = 'the flow did not start within 30 s of the execute click';
      break;
    }
    if (S.pending_seen && S.tab_ms === null && root.kind !== 'pending') {
      if (failSince === null) failSince = Date.now();
      if (Date.now() - failSince >= TEAM_FAIL_GRACE_MS) {
        S.observe_end_reason = 'creation failed: the waiting screen went away and no tab stood';
        break;
      }
    } else {
      failSince = null;
    }
    if (Date.now() - lastFacts >= 30000) {
      lastFacts = Date.now();
      teamWriteFacts(S, true);
    }
    save();
    await sleep(1000);
  }
  if (!S.observe_end_reason) S.observe_end_reason = closed ? 'the CDP socket closed (the app exited)' : 'observation window elapsed (' + Math.round(TEAM_OBSERVE_MS / 1000) + ' s)';
  S.observe_end_ms = Date.now();
  S.observe = { polls: S.polls, ms: S.observe_end_ms - t0, socket_closed: closed, socket_closed_at: closed ? closedAt : null };
  step('team: observation ended', { reason: S.observe_end_reason, tab_created: S.tab_ms !== null });
}

async function teamRun(S, U) {
  S.stage = 'wait_ready';
  step('team: wait for the UI (a workspace tab and a pane drawn)', { ready_ms: TEAM_READY_MS });
  const t0 = Date.now();
  let ready = false;
  let okSince = null;
  while (Date.now() - t0 < TEAM_READY_MS && !closed) {
    let r = null;
    try {
      r = await teamPoll(S);
    } catch (e) {
      if (closed) break;
    }
    const s = r ? r.snap : null;
    const good = !!(s && s.toggle && s.toggle.exists && s.tab_count >= 1 && s.root && s.root.pane_count >= 1);
    if (good) {
      if (okSince === null) okSince = Date.now();
      if (Date.now() - okSince >= TEAM_SETTLE_MS) {
        ready = true;
        break;
      }
    } else {
      okSince = null;
    }
    await sleep(1000);
  }
  S.ready = { ok: ready, waited_ms: Date.now() - t0, snapshot: S.last };
  U.ready = { ok: ready, waited_ms: S.ready.waited_ms };
  if (!ready) {
    S.ready_note = 'the UI was not complete within ' + Math.round(TEAM_READY_MS / 1000) + ' s (a workspace tab and a pane were not both drawn): the click is tried anyway';
    await teamDomSummary(S, 'not-ready');
    if (!S.last || !S.last.toggle || !S.last.toggle.exists) {
      S.flow_error = 'the team controls (#btn-expert-toggle / #btn-ws-dept) are not in the page';
      await uiShot('no-controls');
      return;
    }
  }
  try {
    S.recorder_installed = await evalJs(EXPR_UI_RECORDER, { awaitPromise: false, timeoutMs: 15000 });
  } catch (e) {
    fail('team recorder', e);
  }
  try {
    S.listeners = parseJsonValue(await evalJs(EXPR_TEAM_LISTENERS, { timeoutMs: 30000 }));
    step('team: listeners', S.listeners);
  } catch (e) {
    fail('team listeners', e);
  }
  // the registry and the catalog as the product reads them at the entry (read-only calls the product itself makes)
  const regBefore = await teamInvoke('list_depts', {});
  const catBefore = await teamInvoke('read_dept_catalog', {});
  S.before_registry = regBefore.ok ? teamCompactValue(regBefore.value, 6000) : { error: regBefore.error };
  S.before_catalog = catBefore.ok ? teamCompactValue(catBefore.value, 6000) : { error: catBefore.error };
  S.before_dept_keys = regBefore.ok && regBefore.value && typeof regBefore.value.depts === 'object' && regBefore.value.depts ? Object.keys(regBefore.value.depts) : null;
  await uiShot('ready');

  // the entry button sits in the collapsed "expert" section of the sidebar: open it with a real click first
  let snap = S.last || {};
  if (!(snap.entry && snap.entry.visible) && snap.toggle && snap.toggle.exists) {
    S.stage = 'open_expert';
    step('team: open the expert section (click on its toggle)');
    const tr = await uiClick(K_TEAM.toggle, 'expert toggle', false);
    S.toggle_click = { ok: tr.ok, method: tr.method, error: tr.error || null };
    const w = await teamWait(S, (s) => !!(s.entry && s.entry.visible), 15000);
    snap = w.snap || S.last || {};
  }
  if (!(snap.entry && snap.entry.exists)) {
    S.flow_error = 'the team button (#btn-ws-dept) is not in the page';
    await teamDomSummary(S, 'no-entry');
    await uiShot('no-entry');
    return;
  }

  S.stage = 'click_entry';
  step('team: click the team button');
  S.entry_ms = Date.now();
  S.entry_at = iso();
  const er = await uiClick(K_TEAM.entry, 'team entry button', false);
  S.entry_click = { ok: er.ok, method: er.method, error: er.error || null };
  if (!er.ok) {
    S.flow_error = 'the team button could not be clicked: ' + (er.error || 'unknown');
    await teamDomSummary(S, 'entry-click-failed');
    await uiShot('entry-click-failed');
    return;
  }
  const rw = await teamWaitReaction(S, TEAM_REACT_MS);
  S.reaction = { ok: rw.ok, kind: rw.kind, waited_ms: rw.waited_ms };
  if (!rw.ok) {
    S.flow_error = rw.kind === 'notification' ? 'a failure-like notification appeared and no confirm window or menu followed' : 'no confirm window or menu within ' + Math.round(TEAM_REACT_MS / 1000) + ' s of the click';
    await teamDomSummary(S, 'no-reaction');
    await uiShot('no-reaction');
    return;
  }
  let confirmSnap = rw.snap;
  if (rw.kind === 'menu') {
    const items = (Array.isArray(rw.snap.menu) ? rw.snap.menu : []).map((i) => i.text);
    S.menu = { items, seen_at: iso(), picked_index: items.length - 1, picked_text: items.length ? items[items.length - 1] : null };
    step('team: a menu comes first - choosing its last item', { items });
    await uiShot('menu');
    const mr = await uiClick(K_TEAM.menu_last, 'team menu item', false);
    if (!mr.ok) {
      S.flow_error = 'the menu item could not be clicked: ' + (mr.error || 'unknown');
      await teamDomSummary(S, 'menu-click-failed');
      return;
    }
    const mw = await teamWait(S, (s) => !!s.confirm, 30000);
    if (!mw.ok) {
      S.flow_error = 'no confirm window within 30 s of choosing the menu item';
      await teamDomSummary(S, 'no-confirm-after-menu');
      await uiShot('no-confirm-after-menu');
      return;
    }
    confirmSnap = mw.snap;
  }
  const cs = confirmSnap && confirmSnap.confirm ? confirmSnap.confirm : null;
  if (!cs) {
    S.flow_error = 'the confirm window was gone before it could be read';
    await teamDomSummary(S, 'confirm-gone');
    return;
  }
  S.confirm = { title: cs.title, body: cs.body, buttons: cs.buttons, seen_at: iso() };
  step('team: confirm window', { title: cs.title, buttons: (cs.buttons || []).map((b) => b.text) });
  await uiShot('confirm');

  // the tabs as they are right before the execute click: the baseline of the "a new tab stood" test
  const base = S.last || confirmSnap;
  S.tab_ids_before = (Array.isArray(base.tabs) ? base.tabs : []).map((t) => t.id);
  S.tab_names_before = (Array.isArray(base.tabs) ? base.tabs : []).map((t) => t.name);
  S.tab_count_before = S.tab_ids_before.length;
  S.stage = 'click_create';
  S.create_ms = Date.now();
  S.create_at = iso();
  const yr = await uiClick(K_TEAM.confirm_yes, 'team confirm yes', false);
  S.create_click = { ok: yr.ok, method: yr.method, error: yr.error || null };
  if (!yr.ok) {
    S.create_ms = null;
    S.flow_error = 'the execute button of the confirm window could not be clicked: ' + (yr.error || 'unknown');
    await teamDomSummary(S, 'confirm-click-failed');
    await uiShot('confirm-click-failed');
    return;
  }
  S.create_click_ok = true;
  teamWriteLive(S);
  step('team: execute button clicked', { method: yr.method, at: S.create_at });
  const cw = await teamWait(S, (s) => !s.confirm, 10000);
  S.confirm_closed = cw.ok;

  await teamObserve(S);

  S.stage = 'tail';
  if (!closed) {
    try {
      await teamPoll(S);
    } catch (e) {
      // ignore
    }
    await uiShot('end');
    try {
      const rp = parseJsonValue(await evalJs(EXPR_UI_RECORDER_POLL, { awaitPromise: false, timeoutMs: 10000 }));
      if (rp && rp.events) S.recorder = rp;
    } catch (e) {
      // the recorder is an extra source (it also sees toasts that lived less than one poll)
    }
    teamWriteFacts(S, true);
    await teamSeats(S);
    await teamAlarms(S);
  }
}

async function runTeamMode() {
  armWatchdog(TEAM_READY_MS + TEAM_OBSERVE_MS + 8 * 60 * 1000);
  const U = (R.ui = { mode: MODE, stage: 'start', ready: null, clicks: [], states: [], shots: [], button_flow_complete: false, fallback_invoke: false, observe: null });
  const S = (R.team = teamNewState());
  S.params = { observe_ms: TEAM_OBSERVE_MS, ready_ms: TEAM_READY_MS, settle_ms: TEAM_SETTLE_MS, react_ms: TEAM_REACT_MS, fail_grace_ms: TEAM_FAIL_GRACE_MS };
  try {
    await teamRun(S, U);
  } catch (e) {
    fail('team flow', e);
    S.flow_error = S.flow_error || 'exception in the team flow: ' + String(e && e.message ? e.message : e);
  }
  if (S.observe_end_ms === null) S.observe_end_ms = Date.now();
  S.prev = undefined;
  S.timeline.sort((a, b) => a.t - b.t);
  teamWriteFacts(S, false);
  U.stage = 'done';
  S.stage = 'done';
  save();
}

// ---------------------------------------------------------------------------
// 7th task (diag/app-upgrade.ps1, scene UPGRADE): mode uiobs = OBSERVE ONLY. Nothing is clicked, nothing that changes state is invoked.
//   uiobs   attach, then every --obs-interval-sec (default 5) for --obs-sec (default 150; 0 = exactly one poll): the status bar text
//           (#daemon-info), the version-skew badge inside it (.ver-skew-badge: presence, text, title), every toast (class, title, body),
//           open windows - and the product's own READ-ONLY calls invoke('daemon_status') and invoke('app_version'). The mode ends early
//           when the daemon reports --obs-expect-daemon in two polls in a row.
// Why daemon_status and not the status bar: #daemon-info only shows "daemon pid=<pid> sock=<path>" - ui/src/main.ts start():
//   info.textContent = `daemon pid=${status.daemon_pid} sock=${status.socket_path}` - written once in start() (the only other writes are
//   error / "blocked" texts). It carries no version, and no code path rewrites it after a daemon rotation. The daemon version that the UI
//   itself compares with the app version is daemon_status().version (detectSkew(); daemon_status = RPC system.identify -> socket_path,
//   daemon_pid, version, started_at, surface_count: a pure read). checkVersionSkew() runs once at start and every 5 minutes: with no
//   session to keep it rotates the daemon by itself, else it appends the badge (its text names both versions) to #daemon-info and
//   shows one notice toast.
// Files: <prefix>-cdp.json (everything, R.obs), <prefix>-obs-facts.json (small, rewritten after every poll: what the PowerShell side
// reads), <prefix>-obs-timeline.txt (one line per poll), <prefix>-ui-obs-start.png / <prefix>-ui-obs-end.png.
// Options: --obs-sec 150 --obs-interval-sec 5 --obs-expect-daemon <version> (the readiness wait is --ready-wait-sec capped at 30 s),
// --obs-min-sec 45 (UPG-2: no early end before that; every toast is filed with first seen / gone / there at the start / at the end).
// Selectors follow the product UI (ui/index.html + ui/src/main.ts, the same in 0.14.42 and 0.14.43): #daemon-info, .ver-skew-badge
// (showSkewBadge), #toasts > .toast (.toast-name .toast-detail), .modal-overlay, #btn-update, #ws-tabs .ws-tab, #root .pane.
// ---------------------------------------------------------------------------
const OBS_MS = Math.max(0, teamNum(args['obs-sec'], 150)) * 1000;
const OBS_INTERVAL_MS = Math.max(1, teamNum(args['obs-interval-sec'], 5)) * 1000;
const OBS_EXPECT = args['obs-expect-daemon'] === undefined || args['obs-expect-daemon'] === true ? '' : String(args['obs-expect-daemon']);
const OBS_READY_MS = Math.min(UI_READY_MS, 30000);
const OBS_TICKS_MAX = 240;
// UPG-2: the early end (the daemon reports the expected version twice) is only taken after this many seconds, so that notifications
// which come and go right after the start are all seen (--obs-min-sec, default 45; never longer than --obs-sec)
const OBS_MIN_MS = Math.min(Math.max(0, teamNum(args['obs-min-sec'], 45)) * 1000, OBS_MS);
const K_OBS = {
  notice_title: '\uC0C8 \uBC84\uC804 \uC900\uBE44', // title of the one-time notice toast of checkVersionSkew() ("new version ready")
  // UPG-2: two notifications of the first start after an update (ui/src/main.ts): toast("watchdog", "<restore done title>", ...) after
  // the node restore, and toast("health", ..., <payload of the backend event update-error>) - that payload names init-pack
  restore_title: '\uC9C1\uC6D0 \uBCF5\uADC0 \uC644\uB8CC',
  update_error_text: 'init-pack',
};

// ==== OBS-PURE-BEGIN (self-contained: nothing in this block may use anything defined outside it; the local self-check evaluates exactly this text) ====
function obsNewState(expect) {
  return {
    expect: expect || '',
    polls: 0,
    daemon_ok: 0,
    daemon_fail: 0,
    last_error: null,
    version_first: null,
    version_last: null,
    versions: [],
    pid_first: null,
    pid_last: null,
    pids: [],
    started_first: null,
    started_last: null,
    surface_count_last: null,
    reached_at: null,
    reached_el_ms: null,
    reached_streak: 0,
    badge_seen: false,
    badge_first_at: null,
    badge_first_el_ms: null,
    badge_polls: 0,
    badge_last_present: false,
    badge_text: '',
    badge_title: '',
    notice_seen: false,
    notice_detail: '',
    info_first: null,
    info_last: null,
    app_version_cmd: null,
    update_error_seen: false,
    update_error_detail: '',
    update_error_first_el_ms: null,
    restore_done_seen: false,
    restore_done_detail: '',
    restore_done_first_el_ms: null,
    toasts: [], // every distinct toast (class + title) that was on the screen in a poll
  };
}

// one poll { at, el_ms, snap, daemon } -> the running state
function obsNote(F, tick, noticeTitle, marks) {
  F.polls++;
  const d = tick && tick.daemon ? tick.daemon : null;
  if (d && d.ok === true) {
    F.daemon_ok++;
    const v = d.version === undefined || d.version === null ? null : String(d.version);
    if (v !== null) {
      if (F.version_first === null) F.version_first = v;
      F.version_last = v;
      if (F.versions.indexOf(v) < 0) F.versions.push(v);
    }
    if (d.daemon_pid !== undefined && d.daemon_pid !== null) {
      if (F.pid_first === null) F.pid_first = d.daemon_pid;
      F.pid_last = d.daemon_pid;
      if (F.pids.indexOf(d.daemon_pid) < 0) F.pids.push(d.daemon_pid);
    }
    if (d.started_at !== undefined && d.started_at !== null) {
      if (F.started_first === null) F.started_first = d.started_at;
      F.started_last = d.started_at;
    }
    if (d.surface_count !== undefined && d.surface_count !== null) F.surface_count_last = d.surface_count;
    if (d.app_version_cmd) F.app_version_cmd = String(d.app_version_cmd);
    if (F.expect && v === F.expect) {
      F.reached_streak++;
      if (F.reached_at === null) {
        F.reached_at = tick.at;
        F.reached_el_ms = tick.el_ms;
      }
    } else {
      F.reached_streak = 0;
    }
  } else {
    F.daemon_fail++;
    F.reached_streak = 0;
    F.last_error = d && d.error ? String(d.error) : 'no daemon_status result';
  }
  const s = tick && tick.snap ? tick.snap : null;
  if (s) {
    if (s.daemon_info_present) {
      const own = String(s.daemon_info_own || s.daemon_info || '');
      if (F.info_first === null) F.info_first = own;
      F.info_last = own;
    }
    const b = s.badge || {};
    F.badge_last_present = b.present === true;
    if (b.present === true) {
      F.badge_polls++;
      if (!F.badge_seen) {
        F.badge_seen = true;
        F.badge_first_at = tick.at;
        F.badge_first_el_ms = tick.el_ms;
      }
      F.badge_text = String(b.text || '');
      F.badge_title = String(b.title || '');
    }
    const ts = Array.isArray(s.toasts) ? s.toasts : [];
    const seenNow = [];
    for (let i = 0; i < ts.length; i++) {
      const t = ts[i] || {};
      const name = String(t.name || '');
      const detail = String(t.detail || '');
      const cls = String(t.cls || '');
      let rec = null;
      for (let k = 0; k < F.toasts.length; k++) {
        if (F.toasts[k].name === name && F.toasts[k].cls === cls) {
          rec = F.toasts[k];
          break;
        }
      }
      if (!rec && F.toasts.length < 60) {
        rec = { cls: cls, name: name, detail: detail, first_el_ms: tick.el_ms, last_el_ms: tick.el_ms, polls: 0, first_poll: F.polls, gone_el_ms: null };
        F.toasts.push(rec);
      }
      if (rec) {
        rec.polls++;
        rec.last_el_ms = tick.el_ms;
        rec.detail = detail;
        rec.gone_el_ms = null;
        seenNow.push(rec);
      }
      const m = marks || {};
      if (m.update_error_text && (' ' + cls + ' ').indexOf(' health ') >= 0 && detail.indexOf(m.update_error_text) >= 0 && !F.update_error_seen) {
        F.update_error_seen = true;
        F.update_error_detail = name + ' :: ' + detail;
        F.update_error_first_el_ms = tick.el_ms;
      }
      if (m.restore_title && name.indexOf(m.restore_title) >= 0 && !F.restore_done_seen) {
        F.restore_done_seen = true;
        F.restore_done_detail = name + ' :: ' + detail;
        F.restore_done_first_el_ms = tick.el_ms;
      }
      if (noticeTitle && name.indexOf(noticeTitle) >= 0) {
        F.notice_seen = true;
        F.notice_detail = detail;
      }
    }
    // a toast that was seen before and is not in this poll went away between the two polls
    for (let k = 0; k < F.toasts.length; k++) {
      if (seenNow.indexOf(F.toasts[k]) < 0 && F.toasts[k].gone_el_ms === null) F.toasts[k].gone_el_ms = tick.el_ms;
    }
  }
  return F;
}

// the small result the PowerShell side reads (fixed key names only: Windows PowerShell 5.1 cannot read arbitrary texts as JSON keys)
function obsFacts(F) {
  const hasToken = function (cls, token) {
    return (' ' + String(cls || '') + ' ').indexOf(' ' + token + ' ') >= 0;
  };
  // the pid the status bar shows ("daemon pid=<pid> sock=..."), to compare with the pid daemon_status answers
  const pm = /pid=(\d+)/.exec(String(F.info_last || ''));
  const barPid = pm ? Number(pm[1]) : null;
  const dsPid = F.pid_last === null || F.pid_last === undefined ? null : Number(F.pid_last);
  return {
    polls: F.polls,
    daemon_reads_ok: F.daemon_ok,
    daemon_reads_failed: F.daemon_fail,
    daemon_last_error: F.last_error,
    expect_daemon: F.expect || null,
    daemon_version_first: F.version_first,
    daemon_version_last: F.version_last,
    daemon_versions_seen: F.versions.slice(),
    daemon_pid_first: F.pid_first,
    daemon_pid_last: F.pid_last,
    daemon_pids_seen: F.pids.slice(),
    daemon_started_at_first: F.started_first,
    daemon_started_at_last: F.started_last,
    daemon_surface_count_last: F.surface_count_last,
    daemon_changed: F.pids.length > 1 || F.versions.length > 1,
    daemon_reached_expect: F.expect ? F.version_last === F.expect : null,
    daemon_reached_expect_at: F.reached_at,
    daemon_reached_expect_el_ms: F.reached_el_ms,
    skew_badge_seen: F.badge_seen,
    skew_badge_first_at: F.badge_first_at,
    skew_badge_first_el_ms: F.badge_first_el_ms,
    skew_badge_polls: F.badge_polls,
    skew_badge_present_at_end: F.badge_last_present,
    skew_badge_text: F.badge_text,
    skew_badge_title: F.badge_title,
    skew_notice_seen: F.notice_seen,
    skew_notice_detail: F.notice_detail,
    daemon_info_first: F.info_first,
    daemon_info_last: F.info_last,
    app_version_cmd: F.app_version_cmd,
    status_bar_pid: barPid,
    status_bar_pid_differs: barPid === null || dsPid === null || isNaN(dsPid) ? null : barPid !== dsPid,
    update_error_toast_seen: F.update_error_seen,
    update_error_toast_detail: F.update_error_detail,
    update_error_toast_first_el_ms: F.update_error_first_el_ms,
    restore_done_toast_seen: F.restore_done_seen,
    restore_done_toast_detail: F.restore_done_detail,
    restore_done_toast_first_el_ms: F.restore_done_first_el_ms,
    toasts_seen: F.toasts.map(function (t) {
      return { cls: t.cls, name: t.name, detail: t.detail, first_el_ms: t.first_el_ms, last_el_ms: t.last_el_ms, polls: t.polls, failure_like: hasToken(t.cls, 'watchdog') || hasToken(t.cls, 'health'), present_at_start: t.first_poll === 1, gone_el_ms: t.gone_el_ms === undefined ? null : t.gone_el_ms, present_at_end: t.gone_el_ms === null || t.gone_el_ms === undefined };
    }),
  };
}

// one line of <prefix>-obs-timeline.txt
function obsLine(tick) {
  const d = tick && tick.daemon ? tick.daemon : {};
  const s = tick && tick.snap ? tick.snap : {};
  const b = s.badge || {};
  const ts = Array.isArray(s.toasts) ? s.toasts : [];
  const el = (Math.round((Number(tick && tick.el_ms) || 0) / 100) / 10).toFixed(1);
  let dm;
  if (d.ok === true) dm = 'daemon v' + d.version + ' pid=' + d.daemon_pid + ' surfaces=' + d.surface_count + ' started_at=' + d.started_at;
  else dm = 'daemon_status FAILED: ' + String(d.error || (tick && tick.snap_error) || 'no result').slice(0, 160);
  return (
    '[+' + el + 's ' + String(tick && tick.at) + '] ' + dm +
    ' | status bar: ' + JSON.stringify(s.daemon_info_own === undefined ? null : s.daemon_info_own) +
    ' | badge: ' + (b.present === true ? JSON.stringify(String(b.text || '')) : 'none') +
    ' | toasts(' + ts.length + '): ' +
    ts
      .map(function (t) {
        return '[' + String(t.cls || '') + '] ' + String(t.name || '') + ' :: ' + String(t.detail || '').slice(0, 200);
      })
      .join(' || ') +
    ' | windows: ' + (Array.isArray(s.overlays) ? s.overlays.length : 0)
  );
}
// ==== OBS-PURE-END ====

// ==== OBS-PAGE-BEGIN (functions that run INSIDE the page: they are stringified into Runtime.evaluate expressions, so each must be self-contained) ====
function pageObsSnapshot() {
  var txt = function (el) {
    return el && el.textContent ? String(el.textContent).replace(/\s+/g, ' ').trim() : '';
  };
  var cut = function (s, n) {
    var t = String(s === undefined || s === null ? '' : s);
    return t.length > n ? t.slice(0, n) : t;
  };
  var qa = function (s) {
    return Array.prototype.slice.call(document.querySelectorAll(s));
  };
  var info = document.getElementById('daemon-info');
  var badge = document.querySelector('.ver-skew-badge');
  // the status bar's own text, without the badge element that the product appends to it
  var own = '';
  if (info) {
    var kids = info.childNodes || [];
    for (var i = 0; i < kids.length; i++) {
      if (kids[i].nodeType === 3) own += String(kids[i].textContent || '');
    }
  }
  var o = { t: Date.now(), title: document.title };
  o.daemon_info_present = !!info;
  o.daemon_info = cut(txt(info), 600);
  o.daemon_info_own = cut(own.replace(/\s+/g, ' ').trim(), 400);
  o.badge = badge
    ? { present: true, text: cut(txt(badge), 400), title: cut(badge.title || badge.getAttribute('title') || '', 1000), in_daemon_info: !!(info && info.contains(badge)) }
    : { present: false, text: '', title: '', in_daemon_info: false };
  o.toasts = qa('#toasts > *').map(function (e) {
    return { cls: String(e.className || ''), name: cut(txt(e.querySelector('.toast-name')), 400), detail: cut(txt(e.querySelector('.toast-detail')), 2000) };
  });
  o.overlays = qa('.modal-overlay').map(function (e) {
    return { cls: String(e.className || ''), title: cut(txt(e.querySelector('h3')), 200) };
  });
  o.update_button = !!document.getElementById('btn-update');
  o.tab_count = qa('#ws-tabs .ws-tab').length;
  o.pane_count = qa('#root .pane').length;
  return JSON.stringify(o);
}

// the two read-only calls the product's own detectSkew() / checkVersionSkew() make: daemon_status (RPC system.identify) and app_version
async function pageObsDaemon() {
  var o = { t: Date.now(), ok: false, version: null, daemon_pid: null, socket_path: null, started_at: null, surface_count: null, keys: [], error: null, app_version_cmd: null };
  var es = function (e) {
    return e && e.message ? e.message : typeof e === 'string' ? e : JSON.stringify(e);
  };
  try {
    var st = await window.__TAURI__.core.invoke('daemon_status');
    o.ok = true;
    if (st && typeof st === 'object') {
      o.keys = Object.keys(st).slice(0, 40);
      o.version = st.version === undefined || st.version === null ? null : String(st.version);
      o.daemon_pid = st.daemon_pid === undefined ? null : st.daemon_pid;
      o.socket_path = st.socket_path === undefined || st.socket_path === null ? null : String(st.socket_path).slice(0, 300);
      o.started_at = st.started_at === undefined ? null : st.started_at;
      o.surface_count = st.surface_count === undefined ? null : st.surface_count;
    }
  } catch (e) {
    o.error = String(es(e)).slice(0, 400);
  }
  try {
    o.app_version_cmd = String(await window.__TAURI__.core.invoke('app_version'));
  } catch (e2) {
    o.app_version_cmd = null;
  }
  return JSON.stringify(o);
}
// ==== OBS-PAGE-END ====
const EXPR_OBS_SNAPSHOT = '(' + pageObsSnapshot.toString() + ')()';
const EXPR_OBS_DAEMON = '(' + pageObsDaemon.toString() + ')()';

function obsWriteFacts(O, F, partial) {
  try {
    const o = {
      script: 'cdp-update.mjs',
      mode: MODE,
      written: iso(),
      partial: !!partial,
      attached: R.attached,
      app_version: R.app_version,
      ready_ok: O.ready ? O.ready.ok : null,
      end_reason: O.end_reason,
      socket_closed: closed,
      obs_ms: OBS_MS,
      obs_min_ms: OBS_MIN_MS,
      interval_ms: OBS_INTERVAL_MS,
      facts: obsFacts(F),
    };
    fs.writeFileSync(path.join(OUT, `${PREFIX}-obs-facts.json`), JSON.stringify(o, null, 1));
  } catch (e) {
    fail('obs facts', e);
  }
}

async function runObsMode() {
  armWatchdog(OBS_READY_MS + OBS_MS + 4 * 60 * 1000);
  // R.ui exists so that uiShot() files its screenshots like the other UI modes do (R.ui.shots); this mode makes no click
  R.ui = { mode: MODE, stage: 'start', ready: null, clicks: [], states: [], shots: [], button_flow_complete: false, fallback_invoke: false, observe: null };
  const O = (R.obs = { params: { obs_ms: OBS_MS, interval_ms: OBS_INTERVAL_MS, expect_daemon: OBS_EXPECT || null }, started: iso(), ended: null, end_reason: null, ready: null, ticks: [], ticks_dropped: 0, facts: null });
  const F = obsNewState(OBS_EXPECT);
  const tl = path.join(OUT, `${PREFIX}-obs-timeline.txt`);
  const line = (text) => {
    try {
      fs.appendFileSync(tl, text + '\n');
    } catch (e) {
      // ignore: the same facts are in the JSON files
    }
  };
  line(`# uiobs start ${iso()} obs_sec=${Math.round(OBS_MS / 1000)} interval_sec=${Math.round(OBS_INTERVAL_MS / 1000)} expect_daemon=${OBS_EXPECT || '-'} app_version=${R.app_version}`);
  step('obs: wait for the status bar (#daemon-info)');
  const tr = Date.now();
  let readyOk = false;
  while (Date.now() - tr < OBS_READY_MS && !closed) {
    try {
      const s0 = parseJsonValue(await evalJs(EXPR_OBS_SNAPSHOT, { awaitPromise: false, timeoutMs: 12000 }));
      if (s0 && s0.ok !== false && s0.daemon_info_present) {
        readyOk = true;
        break;
      }
    } catch (e) {
      if (closed) break;
    }
    await sleep(500);
  }
  O.ready = { ok: readyOk, waited_ms: Date.now() - tr };
  R.ui.ready = O.ready;
  if (!closed) await uiShot('obs-start');
  R.ui.stage = 'observe';
  const t0 = Date.now();
  step('obs: observing', { obs_ms: OBS_MS, interval_ms: OBS_INTERVAL_MS, expect_daemon: OBS_EXPECT || null, ready: readyOk });
  for (;;) {
    if (closed) {
      O.end_reason = 'the CDP socket closed (the app exited)';
      break;
    }
    const tick = { at: iso(), el_ms: Date.now() - t0, snap: null, daemon: null };
    try {
      const s = parseJsonValue(await evalJs(EXPR_OBS_SNAPSHOT, { awaitPromise: false, timeoutMs: 12000 }));
      if (s && s.ok !== false) tick.snap = s;
      else tick.snap_error = s && s.error ? String(s.error) : 'no snapshot';
    } catch (e) {
      tick.snap_error = String(e && e.message ? e.message : e);
    }
    if (!closed) {
      try {
        tick.daemon = parseJsonValue(await evalJs(EXPR_OBS_DAEMON, { timeoutMs: 15000 }));
      } catch (e) {
        tick.daemon = { ok: false, error: String(e && e.message ? e.message : e) };
      }
    }
    obsNote(F, tick, K_OBS.notice_title, K_OBS);
    if (O.ticks.length < OBS_TICKS_MAX) O.ticks.push(tick);
    else O.ticks_dropped++;
    line(obsLine(tick));
    O.facts = obsFacts(F);
    obsWriteFacts(O, F, true);
    save();
    if (closed) {
      O.end_reason = 'the CDP socket closed (the app exited)';
      break;
    }
    if (OBS_EXPECT && F.reached_streak >= 2 && Date.now() - t0 >= OBS_MIN_MS) {
      O.end_reason = 'the daemon reports the expected version ' + OBS_EXPECT + ' (two polls in a row)' + (OBS_MIN_MS > 0 ? ' and the minimum observation of ' + Math.round(OBS_MIN_MS / 1000) + ' s is over' : '');
      break;
    }
    const left = OBS_MS - (Date.now() - t0);
    if (left <= 0) {
      O.end_reason = 'observation window elapsed (' + Math.round(OBS_MS / 1000) + ' s)';
      break;
    }
    await sleep(Math.max(200, Math.min(OBS_INTERVAL_MS, left)));
  }
  O.ended = iso();
  if (!closed) await uiShot('obs-end');
  O.facts = obsFacts(F);
  line('# end: ' + O.end_reason);
  obsWriteFacts(O, F, false);
  R.ui.stage = 'done';
  step('obs: ended', { reason: O.end_reason, polls: F.polls, daemon_version_last: F.version_last, badge_seen: F.badge_seen });
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
  if (MODE === 'uiteam') {
    await runTeamMode();
    return;
  }
  if (MODE === 'uiobs') {
    await runObsMode();
    return;
  }
  if (MODE === 'w44') {
    await runW44Mode();
    return;
  }
  if (MODE === 'w44office') {
    await runW44OfficeMode();
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

// ---------------------------------------------------------------------------
// w44 task (diag/w44-scene.ps1): mode w44 = the 0.14.44 screens on a real Windows 11, driven over CDP.
//   usage     the view-mode button of the usage box (#wsbar-usage .usage-mode): its text, the box text, 3 clicks (auto -> all -> one -> auto)
//   feed      Control Center > approval Feed: the tab, a pushed HQ item shows up, Allow on it is received by the pushing process
//             (`cys feed push --wait` exits 0 = allow), and the same for an item pushed to a department daemon (--w44-dept-pipes)
//   office    Control Center > office tab: does the page load (hint hidden, iframe src), the targets, the bridge /health and /world
// Nothing outside the app is touched except the feed items pushed here (`cys feed push` of the product) and the bridge GETs.
// Options: --w44-cys <cys.exe> --w44-dept-pipes <pipe,pipe> --w44-office-wait-sec 120 --w44-feed-wait-sec 60
// Files: <prefix>-cdp.json (R.w44), <prefix>-ui-w44-*.png
// ---------------------------------------------------------------------------
const W44_CYS = args['w44-cys'] === undefined || args['w44-cys'] === true ? '' : String(args['w44-cys']);
const W44_DEPT_PIPES = args['w44-dept-pipes'] === undefined || args['w44-dept-pipes'] === true ? [] : String(args['w44-dept-pipes']).split(',').map((s) => s.trim()).filter(Boolean);
const W44_NO_OFFICE = args['w44-no-office'] === true;
const W44_ONLY_OFFICE = args['w44-only-office'] === true;
const W44_LEAVE_OFFICE = args['w44-leave-office'] === true;
const W44_OFFICE_WAIT_MS = Math.max(10, Number(args['w44-office-wait-sec'] || 120)) * 1000;
const W44_FEED_WAIT_MS = Math.max(10, Number(args['w44-feed-wait-sec'] || 60)) * 1000;

const w44Expr = (fn, ...a) => '(' + fn.toString() + ')(' + a.map((x) => JSON.stringify(x)).join(',') + ')';

function pageW44Usage() {
  const host = document.getElementById('wsbar-usage');
  const btn = host ? host.querySelector('.usage-mode') : null;
  const body = host ? host.querySelector('.usage-body') : null;
  const sum = host ? host.querySelector('.usage-sum') : null;
  let stored = null;
  try { stored = localStorage.getItem('cys-usage-mode'); } catch (e) { stored = 'blocked'; }
  return {
    host: !!host,
    button: btn ? btn.textContent : null,
    button_title: btn ? btn.title : null,
    summary: sum ? sum.textContent : null,
    body_text: body ? String(body.innerText || '').slice(0, 700) : null,
    body_boxes: body ? body.querySelectorAll('.usage-box, .ub-box, [class*="box"]').length : null,
    body_children: body ? body.children.length : null,
    stored_mode: stored,
  };
}
function pageW44ClickUsageMode() {
  const btn = document.querySelector('#wsbar-usage .usage-mode');
  if (!btn) return false;
  btn.click();
  return true;
}
function pageW44Click(sel) {
  const el = document.querySelector(sel);
  if (!el) return false;
  el.click();
  return true;
}
function pageW44Feed() {
  const box = document.getElementById('cc-feed-items');
  const items = box ? Array.from(box.querySelectorAll('.feed-item')) : [];
  const badge = document.getElementById('cc-feed-tabbadge');
  const pend = document.getElementById('cc-pending-badge');
  return {
    box: !!box,
    visible: box ? !!(box.offsetParent) : null,
    item_count: items.length,
    items: items.slice(0, 12).map((e) => ({
      cls: e.className,
      title: (e.querySelector('.fi-title') || {}).textContent || '',
      meta: (e.querySelector('.fi-meta') || {}).textContent || '',
      buttons: Array.from(e.querySelectorAll('button')).map((b) => (b.textContent || '') + (b.disabled ? '(off)' : '')),
    })),
    tab_badge: badge ? { text: badge.textContent, hidden: !!badge.hidden } : null,
    cc_badge: pend ? { text: pend.textContent, hidden: !!pend.hidden } : null,
    text_head: box ? String(box.innerText || '').slice(0, 500) : null,
  };
}
function pageW44FeedClickAllow(title) {
  const box = document.getElementById('cc-feed-items');
  if (!box) return { ok: false, why: 'no #cc-feed-items' };
  const items = Array.from(box.querySelectorAll('.feed-item'));
  const it = items.find((e) => String((e.querySelector('.fi-title') || {}).textContent || '').indexOf(title) >= 0);
  if (!it) return { ok: false, why: 'item not in the list', count: items.length };
  const b = it.querySelector('.fi-actions button.allow');
  if (!b) return { ok: false, why: 'no Allow button', buttons: Array.from(it.querySelectorAll('button')).map((x) => x.textContent), cls: it.className };
  b.click();
  return { ok: true, cls: it.className };
}
function pageW44Office() {
  const fr = document.getElementById('cc-office-frame');
  const hint = document.getElementById('cc-office-hint');
  const txt = document.getElementById('cc-office-hint-text');
  const view = document.getElementById('cc-view-office');
  let doc = 'n/a';
  try { doc = fr && fr.contentDocument ? ('title=' + fr.contentDocument.title) : 'no contentDocument (cross-origin or not loaded)'; } catch (e) { doc = 'blocked: ' + e.name; }
  return {
    view_visible: view ? !view.hidden : null,
    frame_src: fr ? fr.getAttribute('src') : null,
    frame_w: fr ? fr.clientWidth : null,
    frame_h: fr ? fr.clientHeight : null,
    hint_hidden: hint ? !!hint.hidden : null,
    hint_text: txt ? txt.textContent : null,
    repair_button_hidden: (document.getElementById('cc-office-repair') || {}).hidden,
    frame_document: doc,
  };
}

function w44SpawnFeed(title, body, pipe) {
  const rec = { title, pipe: pipe || '(default)', started: iso(), pid: null, exit_code: null, exited_at: null, stdout: '', stderr: '', error: null, child: null };
  try {
    const env = { ...process.env };
    if (pipe) env.CYS_SOCKET = pipe;
    const child = spawn(W44_CYS, ['feed', 'push', '--kind', 'permission', '--title', title, '--body', body, '--wait', '--timeout-secs', '170'], { env, windowsHide: true });
    rec.pid = child.pid;
    child.stdout.on('data', (d) => { rec.stdout = (rec.stdout + d).slice(-600); });
    child.stderr.on('data', (d) => { rec.stderr = (rec.stderr + d).slice(-600); });
    child.on('exit', (c) => { rec.exit_code = c; rec.exited_at = iso(); });
    child.on('error', (e) => { rec.error = String(e && e.message ? e.message : e); });
    rec.child = child;
  } catch (e) { rec.error = String(e && e.message ? e.message : e); }
  return rec;
}

async function w44HttpGet(url, ms) {
  try {
    const r = await fetch(url, { signal: AbortSignal.timeout(ms || 5000) });
    const t = await r.text();
    return { url, status: r.status, bytes: t.length, head: t.slice(0, 600), text: t };
  } catch (e) { return { url, error: String(e && e.message ? e.message : e) }; }
}

async function w44Feed(W, label, title, pipe) {
  const F = { label, title, pipe: pipe || '(default)', pushed: null, seen_after_ms: null, seen_item: null, clicked: null, exit_code: null, exit_after_click_ms: null, verdict: null };
  W.feed.push(F);
  save();
  const sp = w44SpawnFeed(title, 'w44 probe: the harness pushed this item and presses Allow on it', pipe);
  F.pushed = { pid: sp.pid, error: sp.error };
  const t0 = Date.now();
  let seen = null;
  while (Date.now() - t0 < W44_FEED_WAIT_MS && !closed) {
    if (sp.exit_code !== null || sp.error) break;
    try {
      const f = await evalJs(w44Expr(pageW44Feed), { awaitPromise: false, timeoutMs: 10000 });
      const it = (f.items || []).find((x) => String(x.title).indexOf(title) >= 0);
      if (it) { seen = it; F.seen_after_ms = Date.now() - t0; break; }
    } catch (e) { /* page busy */ }
    await sleep(1000);
  }
  F.seen_item = seen;
  if (seen) {
    await uiShot('w44-feed-' + label + '-seen');
    try {
      F.clicked = await evalJs(w44Expr(pageW44FeedClickAllow, title), { awaitPromise: false, timeoutMs: 10000 });
    } catch (e) { F.clicked = { ok: false, why: String(e && e.message ? e.message : e) }; }
    const tc = Date.now();
    while (Date.now() - tc < 25000 && sp.exit_code === null && !sp.error) await sleep(500);
    F.exit_after_click_ms = Date.now() - tc;
  }
  F.exit_code = sp.exit_code;
  F.stdout = sp.stdout; F.stderr = sp.stderr; F.spawn_error = sp.error;
  if (sp.exit_code === null) { try { sp.child && sp.child.kill(); } catch (e) { /* ignore */ } }
  F.verdict = !seen ? 'NOT SEEN in the Feed list' : (F.clicked && F.clicked.ok ? (sp.exit_code === 0 ? 'PASS (seen, Allow clicked, the pushing process got allow = exit 0)' : ('Allow clicked but exit code ' + sp.exit_code)) : 'seen but no Allow button');
  save();
}

async function runW44Mode() {
  armWatchdog(12 * 60 * 1000);
  const U = (R.ui = { mode: MODE, stage: 'start', ready: null, clicks: [], states: [], shots: [] });
  const W = (R.w44 = { started: iso(), cys: W44_CYS, dept_pipes: W44_DEPT_PIPES, ready: null, usage: null, feed: [], office: null, errors: [] });
  try {
    // ready: the sidebar usage box and the Control Center button exist
    const tr = Date.now();
    while (Date.now() - tr < 90000 && !closed) {
      try {
        const ok = await evalJs("(!!document.getElementById('btn-cc') && !!document.getElementById('wsbar-usage')) ? 'yes' : 'no'", { awaitPromise: false, timeoutMs: 8000 });
        if (ok === 'yes') { W.ready = Date.now() - tr; break; }
      } catch (e) { /* loading */ }
      await sleep(1500);
    }
    await sleep(3000);
    await uiShot('w44-start');

    if (W44_LEAVE_OFFICE) {
      // only: go back from the office tab to the Live tab (the 3D page stops being the visible tab)
      await evalJs(w44Expr(pageW44Click, '#btn-cc'), { awaitPromise: false, timeoutMs: 8000 }).catch(() => false);
      await sleep(1500);
      W.left_office = await evalJs(w44Expr(pageW44Click, '.cc-tab[data-view="live"]'), { awaitPromise: false, timeoutMs: 8000 }).catch(() => false);
      await sleep(3000);
      await uiShot('w44-left-office');
      W.finished = iso(); U.stage = 'done'; save();
      return;
    }
    // ---- usage view mode ----
    const US = (W.usage = { states: [], clicks: 0 });
    const snap = async (tag) => { try { const s = await evalJs(w44Expr(pageW44Usage), { awaitPromise: false, timeoutMs: 8000 }); s.tag = tag; US.states.push(s); return s; } catch (e) { US.states.push({ tag, error: String(e && e.message ? e.message : e) }); return null; } };
    const first = await snap('initial');
    for (let i = 0; i < (W44_ONLY_OFFICE ? 0 : 3); i++) {
      const c = await evalJs(w44Expr(pageW44ClickUsageMode), { awaitPromise: false, timeoutMs: 8000 }).catch(() => false);
      US.clicks += c ? 1 : 0;
      await sleep(1500);
      await snap('after-click-' + (i + 1));
      await uiShot('w44-usage-' + (i + 1));
    }
    const labels = US.states.map((s) => s.button);
    US.button_labels = labels;
    US.cycle_ok = !!(first && labels.length === 4 && labels[1] !== labels[0] && labels[2] !== labels[1] && labels[2] !== labels[0] && labels[3] === labels[0]);
    save();

    // ---- Control Center > Feed ----
    await evalJs(w44Expr(pageW44Click, '#btn-cc'), { awaitPromise: false, timeoutMs: 8000 }).catch(() => false);
    await sleep(2000);
    await evalJs(w44Expr(pageW44Click, '.cc-tab[data-view="feed"]'), { awaitPromise: false, timeoutMs: 8000 }).catch(() => false);
    await sleep(2500);
    W.feed_tab = await evalJs(w44Expr(pageW44Feed), { awaitPromise: false, timeoutMs: 10000 }).catch((e) => ({ error: String(e) }));
    await uiShot('w44-feed-tab');
    if (W44_CYS && !W44_ONLY_OFFICE) {
      await w44Feed(W, 'hq', 'w44 probe approval HQ ' + Date.now(), '');
      for (let i = 0; i < Math.min(W44_DEPT_PIPES.length, 2); i++) {
        await w44Feed(W, 'dept' + (i + 1), 'w44 probe approval DEPT' + (i + 1) + ' ' + Date.now(), W44_DEPT_PIPES[i]);
      }
    } else {
      W.errors.push('no --w44-cys: the feed push probes were not run');
    }

    // ---- Control Center > office ----
    if (W44_NO_OFFICE) { W.finished = iso(); U.stage = 'done'; save(); return; }
    await evalJs(w44Expr(pageW44Click, '.cc-tab[data-view="office"]'), { awaitPromise: false, timeoutMs: 8000 }).catch(() => false);
    const OF = (W.office = { samples: [], bridge_health: null, bridge_world: null, targets: null });
    const to = Date.now();
    let loaded = false;
    while (Date.now() - to < W44_OFFICE_WAIT_MS && !closed) {
      let o = null;
      try { o = await evalJs(w44Expr(pageW44Office), { awaitPromise: false, timeoutMs: 8000 }); } catch (e) { o = { error: String(e && e.message ? e.message : e) }; }
      o.t_ms = Date.now() - to;
      if (OF.samples.length < 40) OF.samples.push(o);
      if (o && o.frame_src && o.hint_hidden === true) { loaded = true; OF.loaded_after_ms = o.t_ms; break; }
      await sleep(3000);
    }
    OF.loaded_signal = loaded;
    await sleep(4000);
    await uiShot('w44-office');
    try { const l = await listTargets(); OF.targets = l.map((t) => ({ type: t.type, title: t.title, url: String(t.url).slice(0, 160) })); } catch (e) { OF.targets = String(e); }
    const h = await w44HttpGet('http://127.0.0.1:8642/health', 5000);
    OF.bridge_health = { status: h.status, bytes: h.bytes, head: h.head, error: h.error };
    const w = await w44HttpGet('http://127.0.0.1:8642/world', 8000);
    OF.bridge_world = { status: w.status, bytes: w.bytes, error: w.error };
    try {
      const j = JSON.parse(w.text);
      OF.bridge_world.top_keys = Object.keys(j).slice(0, 30);
      const pick = (o, k) => (o && o[k] !== undefined ? o[k] : undefined);
      const seatsOf = (x) => (Array.isArray(x) ? x.length : (x && typeof x === 'object' ? Object.keys(x).length : null));
      OF.bridge_world.counts = { seats: seatsOf(pick(j, 'seats')), surfaces: seatsOf(pick(j, 'surfaces')), depts: seatsOf(pick(j, 'depts')), agents: seatsOf(pick(j, 'agents')), hq: seatsOf(pick(j, 'hq')) };
      OF.bridge_world.compact = JSON.stringify(j).slice(0, 1500);
    } catch (e) { OF.bridge_world.parse_error = String(e && e.message ? e.message : e); OF.bridge_world.head = String(w.head || '').slice(0, 300); }
    const f2 = await w44HttpGet('http://127.0.0.1:8642/office-boot.js', 5000);
    OF.office_boot_js = { status: f2.status, bytes: f2.bytes };
    await uiShot('w44-office-end');
  } catch (e) {
    fail('w44 flow', e);
    W.errors.push(String(e && e.message ? e.message : e));
  }
  W.finished = iso();
  U.stage = 'done';
  save();
}


// w44office: open Control Center > Live then > office again and watch the office tab: hint text, repair button, frame src/size, until the
// screen is "loaded" (hint hidden + frame src) or the wait ends; then --w44-settle-sec more and a screenshot (--w44-shot <name>).
async function runW44OfficeMode() {
  armWatchdog(8 * 60 * 1000);
  const U = (R.ui = { mode: MODE, stage: 'start', shots: [] });
  const W = (R.w44office = { started: iso(), samples: [], loaded: false, loaded_after_ms: null, final: null, shot: null });
  const shotName = args['w44-shot'] === undefined || args['w44-shot'] === true ? 'w44office' : String(args['w44-shot']);
  const settleMs = Math.max(0, Number(args['w44-settle-sec'] || 15)) * 1000;
  try {
    const tr = Date.now();
    while (Date.now() - tr < 60000 && !closed) {
      try { if ((await evalJs("(!!document.getElementById('btn-cc')) ? 'yes' : 'no'", { awaitPromise: false, timeoutMs: 8000 })) === 'yes') break; } catch (e) { /* loading */ }
      await sleep(1500);
    }
    await evalJs(w44Expr(pageW44Click, '#btn-cc'), { awaitPromise: false, timeoutMs: 8000 }).catch(() => false);
    await sleep(1500);
    await evalJs(w44Expr(pageW44Click, '.cc-tab[data-view="live"]'), { awaitPromise: false, timeoutMs: 8000 }).catch(() => false);
    await sleep(1500);
    await evalJs(w44Expr(pageW44Click, '.cc-tab[data-view="office"]'), { awaitPromise: false, timeoutMs: 8000 }).catch(() => false);
    const t0 = Date.now();
    while (Date.now() - t0 < W44_OFFICE_WAIT_MS && !closed) {
      let o = null;
      try { o = await evalJs(w44Expr(pageW44Office), { awaitPromise: false, timeoutMs: 8000 }); } catch (e) { o = { error: String(e && e.message ? e.message : e) }; }
      o.t_ms = Date.now() - t0;
      if (W.samples.length < 60) W.samples.push(o);
      if (o && o.frame_src && o.hint_hidden === true) { W.loaded = true; W.loaded_after_ms = o.t_ms; break; }
      await sleep(2000);
    }
    await sleep(settleMs);
    try { W.final = await evalJs(w44Expr(pageW44Office), { awaitPromise: false, timeoutMs: 8000 }); } catch (e) { W.final = { error: String(e) }; }
    W.shot = await uiShot(shotName);
    try { const l = await listTargets(); W.targets = l.map((t) => ({ type: t.type, title: t.title, url: String(t.url).slice(0, 120) })); } catch (e) { W.targets = String(e); }
  } catch (e) { fail('w44office', e); }
  W.finished = iso(); U.stage = 'done'; save();
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
