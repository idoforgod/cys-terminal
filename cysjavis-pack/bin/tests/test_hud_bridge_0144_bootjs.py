#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_hud_bridge_0144_bootjs.py — office-boot.js(B5 ㉠㉡㉢)를 가짜 화면 환경에서 돌려 보는 시험(node 가 있을 때만).

  ㉠ 앱 안(iframe: window.top !== window)에서도 배너 · 문구에 터미널 명령 없음 · 세부 줄 「세부: three.module.js — 응답 404」
  ㉡ 다시 불러오기 3→6→12→24→48초 · 정확히 5회에서 멈춤(횟수는 주소의 ob_retry)
  ㉢ /health 기동 식별자: 둘 다 있고 다를 때만 새로 고침 · 응답 없음/옛 브리지(404)는 가만히 · 세션 10회 상한
"""
import json
import os
import shutil
import subprocess
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
JS = os.path.join(os.path.dirname(os.path.dirname(SELF)), "web", "office-boot.js")
NODE = shutil.which("node")

HARNESS = r"""
const fs = require('fs'), vm = require('vm');
const src = fs.readFileSync(process.argv[1], 'utf8');
const scenario = JSON.parse(process.argv[2]);

function makeEnv(opt) {
  let now = 0, seq = 0; const timers = [];
  const hrefs = [];
  const store = {};
  const body = { children: [], appendChild(c) { c.parentNode = this; this.children.push(c); return c; },
                removeChild(c) { const i = this.children.indexOf(c); if (i >= 0) this.children.splice(i, 1); c.parentNode = null; return c; } };
  function el() { return { style: {}, children: [], _t: '', setAttribute() {}, appendChild(c) { this.children.push(c); return c; },
                    set textContent(v) { this._t = v; }, get textContent() { return this._t; } }; }
  const doc = { body, getElementById(id) { if (id === 'scene') return (opt.down && !(opt.upAt != null && now >= opt.upAt)) ? null : { style: { width: '800px' }, width: 800 }; return id === 'office-boot-banner' ? (body.children.find(c => c.id === id) || null) : null; },
                querySelector() { return null; }, createElement() { return el(); }, addEventListener() {} };
  const listeners = {};
  const win = {
    top: opt.iframe ? {} : null, location: { pathname: '/', search: opt.search || '', hash: '' },
    sessionStorage: opt.noStorage ? { getItem() { throw new Error('x'); }, setItem() { throw new Error('x'); } }
      : { getItem(k) { return k in store ? store[k] : null; }, setItem(k, v) { store[k] = String(v); } },
    history: { replaceState() {} },
    addEventListener(t, f) { listeners[t] = f; },
  };
  win.top = opt.iframe ? {} : win;
  Object.defineProperty(win.location, 'href', { set(v) { hrefs.push([now, v]); }, get() { return '/'; } });
  const sandbox = {
    window: win, document: doc, console,
    setTimeout(f, ms) { timers.push({ at: now + ms, f, id: ++seq }); return seq; },
    clearTimeout(id) { const i = timers.findIndex(t => t.id === id); if (i >= 0) timers.splice(i, 1); },
    setInterval(f, ms) { const id = ++seq; const tick = () => { timers.push({ at: now + ms, f: () => { f(); tick(); }, id }); }; tick(); return id; },
    fetch: opt.fetch, AbortController: undefined,
  };
  sandbox.window.AbortController = undefined;
  async function advance(ms) {
    const end = now + ms;
    for (;;) {
      timers.sort((a, b) => a.at - b.at);
      if (!timers.length || timers[0].at > end) break;
      const t = timers.shift(); now = t.at; t.f();
      for (let i = 0; i < 5; i++) await Promise.resolve();
      await new Promise(r => setImmediate(r));
    }
    now = end;
  }
  vm.runInNewContext(src, sandbox);
  return { advance, hrefs, body, store, listeners, win };
}

(async () => {
  const out = {};
  if (scenario === 'retry') {
    const delays = [];
    const fetch404 = async () => ({ status: 404 });
    for (let n = 0; n <= 5; n++) {
      const env = makeEnv({ down: true, iframe: true, search: n ? '?ob_retry=' + n : '', fetch: async (u) => u === '/health' ? { status: 404 } : { status: 404 } });
      await env.advance(3100);                           // 백스톱 3초 → 배너
      const banner = env.body.children[0];
      if (n === 0) { out.banner = banner ? banner.children.map(c => c.textContent) : null; }
      if (n === 5) { out.finalBanner = banner ? banner.children.map(c => c.textContent) : null; }
      await env.advance(60000);
      delays.push(env.hrefs.length ? env.hrefs[0] : null);
    }
    out.delays = delays;
  }
  if (scenario === 'late') {
    // 지연 로드: 4초에 화면이 정상으로 뜬다 → 3초 백스톱이 배너를 띄웠다가 1초 안에 지운다
    const env = makeEnv({ down: true, upAt: 4000, iframe: true, fetch: async () => ({ status: 404 }) });
    await env.advance(3100);
    out.shown = env.body.children.length;
    await env.advance(1500);                             // 4.6초 — 떴으니 배너가 사라져야 한다
    out.afterUp = env.body.children.length;
    await env.advance(60000);
    out.hrefs = env.hrefs.length;                        // 떴으니 다시 불러오기도 없다
    // 끝까지 안 뜨는 경우는 배너가 그대로 남는다(지우는 코드가 정상 신호에만 반응)
    const env2 = makeEnv({ down: true, iframe: true, fetch: async () => ({ status: 404 }) });
    await env2.advance(5000);
    out.stillDown = env2.body.children.length;
  }
  if (scenario === 'health') {
    let ids = ['aaa', 'aaa', 'bbb'];
    let i = 0;
    const env = makeEnv({ search: '?ob_retry=3', fetch: async (u) => ({ status: 200, json: async () => ({ boot_id: ids[Math.min(i++, ids.length - 1)] }) }) });
    await env.advance(1);        // 첫 읽기 = 기준
    await env.advance(15000);    // 같음
    out.afterSame = env.hrefs.length;
    await env.advance(15000);    // 다름
    out.afterDiff = env.hrefs.map(h => h[1]);
    out.store = env.store;
  }
  if (scenario === 'health-missing') {
    const seqs = [() => ({ status: 404 }), () => { throw new Error('down'); }, () => ({ status: 200, json: async () => ({ ok: true }) })];
    let i = 0;
    const env = makeEnv({ fetch: async () => seqs[i++ % 3]() });
    await env.advance(100000);
    out.hrefs = env.hrefs.length;
    // 기준만 있고 이후 응답이 없음 → 새로 고침 없음
    let k = 0;
    const env2 = makeEnv({ fetch: async () => { if (k++ === 0) return { status: 200, json: async () => ({ boot_id: 'a' }) }; throw new Error('down'); } });
    await env2.advance(100000);
    out.hrefs2 = env2.hrefs.length;
  }
  if (scenario === 'cap') {
    let n = 0;
    const env = makeEnv({ fetch: async () => ({ status: 200, json: async () => ({ boot_id: 'id' + (n++) }) }) });
    env.store['office-boot-refresh'] = '9';
    await env.advance(1); await env.advance(15000);
    out.at9 = env.hrefs.length; out.store9 = env.store['office-boot-refresh'];
    await env.advance(15000);
    out.at10 = env.hrefs.length;
  }
  if (scenario === 'nostorage') {
    let n = 0;
    const env = makeEnv({ noStorage: true, search: '?x=1&ob_ref=2', fetch: async () => ({ status: 200, json: async () => ({ boot_id: 'id' + (n++) }) }) });
    await env.advance(1); await env.advance(15000);
    out.hrefs = env.hrefs.map(h => h[1]);
  }
  console.log(JSON.stringify(out));
})();
"""


def run(scn):
    r = subprocess.run([NODE, "-e", HARNESS, JS, json.dumps(scn)], capture_output=True, text=True, timeout=60)
    assert r.returncode == 0, r.stderr
    return json.loads(r.stdout.strip().splitlines()[-1])


@unittest.skipUnless(NODE, "node 없음")
class BootJs(unittest.TestCase):
    def test_banner_in_iframe_without_terminal_command_and_retry_schedule(self):
        o = run("retry")
        self.assertIsNotNone(o["banner"], "iframe 안에서 배너가 안 뜬다")
        self.assertEqual(o["banner"][0], "오피스 화면을 불러오지 못했습니다. 자동으로 다시 시도합니다.")
        self.assertRegex(o["banner"][1], r"^세부: three\.module\.js — 응답 404$")
        for ln in o["banner"] + o["finalBanner"]:
            for bad in ("cys ", "init-pack", "터미널", "python3", "--force"):
                self.assertNotIn(bad, ln)
        want = [[3000, "/?ob_retry=1"], [6000, "/?ob_retry=2"], [12000, "/?ob_retry=3"],
                [24000, "/?ob_retry=4"], [48000, "/?ob_retry=5"]]
        got = [[d[0] - 3000, d[1]] for d in o["delays"][:5]]
        self.assertEqual(got, want)
        self.assertIsNone(o["delays"][5], "여섯 번째(ob_retry=5)부터는 새로 고침이 없다 — 정확히 5회에서 멈춤")
        self.assertEqual(o["finalBanner"][0], "오피스 화면을 불러오지 못했습니다.", "멈춘 뒤에는 '자동으로 다시 시도' 문구가 없다")

    def test_late_success_clears_banner(self):
        o = run("late")
        self.assertEqual(o["shown"], 1, "3초 백스톱이 배너를 띄운다")
        self.assertEqual(o["afterUp"], 0, "4초에 화면이 정상으로 뜨면 배너가 사라진다(리뷰 M3)")
        self.assertEqual(o["hrefs"], 0, "뜬 화면은 다시 불러오지 않는다")
        self.assertEqual(o["stillDown"], 1, "끝까지 안 뜨면 배너는 남는다")

    def test_health_boot_id_change_refreshes_once(self):
        o = run("health")
        self.assertEqual(o["afterSame"], 0)
        self.assertEqual(o["afterDiff"], ["/"], "다를 때 한 번 · 재시도 인자는 지우고 새로 고침")
        self.assertEqual(o["store"].get("office-boot-refresh"), "1")

    def test_health_missing_or_old_bridge_does_nothing(self):
        o = run("health-missing")
        self.assertEqual(o["hrefs"], 0)
        self.assertEqual(o["hrefs2"], 0, "응답이 끊긴 동안 새로 고침이 되풀이되면 안 된다")

    def test_session_cap_10(self):
        o = run("cap")
        self.assertEqual(o["at9"], 1)
        self.assertEqual(o["store9"], "10")
        self.assertEqual(o["at10"], 1, "10회 뒤에는 더 새로 고치지 않는다")

    def test_no_storage_falls_back_to_url_param(self):
        o = run("nostorage")
        self.assertEqual(o["hrefs"], ["/?x=1&ob_ref=3"])

    def test_source_has_no_new_syntax(self):
        src = open(JS, encoding="utf-8").read()
        import re
        code = re.sub(r"/\*.*?\*/", "", src, flags=re.S)
        for bad in ("(?<=", "(?<!", ".at(", "replaceAll", "?.", "??", "=>"):
            self.assertNotIn(bad, code, bad)
        self.assertNotIn("window.top !== window", code, "iframe 가드는 제거(앱 안에서도 배너)")


if __name__ == "__main__":
    unittest.main(verbosity=2)
