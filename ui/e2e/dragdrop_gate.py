#!/usr/bin/env python3
"""OS 파일 드롭 좌표·하이라이트·오류 문구 회귀 게이트 (수동 실행 — CI 미배선).

검증 항목 (WP-A-ui · RED 먼저):
  1. dsf=2 macOS/Linux 논리 좌표 → 오른쪽 pane B(surfaceId=2); macOS A → 1.
  2. dsf=2 Windows 물리 좌표(CSS×2) → B; dsf=1 macOS 대조군 → B.
  3. drag-enter/over/leave/drop의 .pane.drop-target 이동·해제.
  4. list_dir reject와 항목 부재 토스트 분리·다중 경로 2/2 표시·전부 중단.
  5. cysDropDebug 기본 off / "1" on의 dpr 진단 토스트·정상 주입 유지.
  6. 모든 컨텍스트 pageerror 0건. UA와 device_scale_factor를 명시해 호스트 독립.

사전: sh ui/build.sh · pip install playwright · playwright install chromium
실행: python3 ui/e2e/dragdrop_gate.py   # exit 0 = PASS
지정된 검증 환경:
  PLAYWRIGHT_BROWSERS_PATH=/Users/cys/Desktop/CYSjavis/_evidence/bugverify-3problems-20260921/V2-P1-dragdrop-ui-sim/_sandbox/pw-browsers \
  /Users/cys/Desktop/CYSjavis/_evidence/bugverify-3problems-20260921/V2-P1-dragdrop-ui-sim/_sandbox/venv/bin/python ui/e2e/dragdrop_gate.py

ui/dist를 localhost로 서빙하고 __TAURI__ shim으로 두 pane을 부팅한다.
V2-P1 harness/dragdrop_sim.py의 SHIM·JS_STATE·JS_DROP·boot·pane_points를 이식했다.
실제 OS 드래그 대신 등록된 Tauri 이벤트 핸들러에 payload를 전달하는 UI 게이트다.
"""
import http.server
import sys
import threading
from pathlib import Path

DIST = Path(__file__).resolve().parent.parent / "dist"
UA_MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)"
UA_WIN = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
UA_LINUX = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
REPORT = "/Users/owner/proj/report.md"
GONE = "/Users/owner/proj/gone.bin"

SHIM = r"""
(() => {
  const calls = [];
  const listeners = {};
  const cfg = {
    listDir: 'ok',          // ok | reject | missing | nfd
    sendInput: 'ok',        // ok | reject
    listSurfaces: 'ok',     // ok | reject
    surfaces: [
      { surface_id: 1, title: 'ceo', exited: false, role: 'ceo', agent: 'claude', live_cwd: '/Users/owner/proj' },
      { surface_id: 2, title: 'zsh', exited: false, role: null, agent: null, live_cwd: '/Users/owner/proj' },
    ],
  };
  window.__sim = { calls, listeners, cfg };
  const respond = (cmd, a) => {
    switch (cmd) {
      case 'daemon_status': return { daemon_pid: 4242, socket_path: '/sandbox/cys.sock', version: '0.14.38', app_version: '0.14.38' };
      case 'app_version': return '0.14.38';
      case 'list_depts': return { depts: {} };
      case 'dept_tombstones': return [];
      case 'fresh_start_check': return { ask: false };
      case 'list_surfaces':
        if (cfg.listSurfaces === 'reject') throw 'sim: list_surfaces failed';
        return { surfaces: cfg.surfaces };
      case 'attach_surface': return { output_event: 'sim-out-' + a.surfaceId, exited_event: 'sim-exit-' + a.surfaceId };
      case 'list_dir': {
        if (cfg.listDir === 'reject') throw 'Operation not permitted (os error 1)';
        // missing에는 report.md도 gone.bin도 없다.
        if (cfg.listDir === 'missing') return [{ name: 'other.txt', is_dir: false }];
        // ok에는 report.md가 있고 gone.bin이 없어 두 번째 경로에서 실패한다.
        const names = ['report.md', 'my file.txt', 'notes.txt', '한글 보고서.md', '한글.md'];
        if (cfg.listDir === 'nfd') return names.map((n) => ({ name: n.normalize('NFD'), is_dir: false }));
        return names.map((n) => ({ name: n, is_dir: false }));
      }
      case 'send_input':
        if (cfg.sendInput === 'reject') throw 'sim: surface not found';
        return null;
      case 'home_dir_path': return '/Users/owner';
      default: return null;
    }
  };
  window.__TAURI__ = {
    core: {
      invoke: (cmd, args) => new Promise((res, rej) => {
        const rec = { cmd, args: args ?? null, t: Date.now() };
        calls.push(rec);
        try { const v = respond(cmd, args ?? {}); rec.ok = true; res(v); }
        catch (e) { rec.ok = false; rec.err = String(e); rej(e); }
      }),
    },
    event: {
      listen: (name, h) => { (listeners[name] ??= []).push(h); return Promise.resolve(() => {}); },
    },
  };
  window.__emit = (name, payload) => { for (const h of listeners[name] ?? []) h({ payload }); return (listeners[name] ?? []).length; };
})();
"""

JS_STATE = r"""
() => {
  const rect = (el) => { if (!el) return null; const r = el.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; };
  const panes = [...document.querySelectorAll('.pane')].map((el) => ({ rect: rect(el), title: el.querySelector('.pane-title, .title')?.textContent ?? null, cls: el.className }));
  return {
    dpr: window.devicePixelRatio, inner: { w: innerWidth, h: innerHeight },
    panes, wsbar: rect(document.getElementById('wsbar')), toolbar: rect(document.getElementById('toolbar') ?? document.querySelector('header')),
    dragDropListeners: (window.__sim.listeners['tauri://drag-drop'] ?? []).length,
    listenerNames: Object.keys(window.__sim.listeners),
    cmds: window.__sim.calls.map((c) => c.cmd),
  };
}
"""

JS_DROP = r"""
async ({ pos, css, paths, settle }) => {
  const sim = window.__sim;
  const n0 = sim.calls.length;
  document.querySelectorAll('.toast').forEach((t) => t.remove());
  const n = window.__emit('tauri://drag-drop', { paths, position: pos });
  await new Promise((r) => setTimeout(r, settle ?? 250));
  const newCalls = sim.calls.slice(n0).filter((c) => ['send_input', 'list_dir', 'list_surfaces'].includes(c.cmd));
  const toasts = [...document.querySelectorAll('.toast')].map((t) => t.textContent.replace(/\s+/g, ' ').trim());
  const modal = document.querySelector('.modal-overlay');
  // 표적의 CSS 좌표는 호출자가 전달한다. 제품의 환산식을 판정 오라클로 쓰지 않는다.
  const hit = document.elementFromPoint(css.x, css.y);
  return {
    handlers: n,
    send_input: newCalls.filter((c) => c.cmd === 'send_input').map((c) => ({ surfaceId: c.args.surfaceId, data: c.args.data, machineOrigin: c.args.machineOrigin, ok: c.ok, err: c.err ?? null })),
    list_dir: newCalls.filter((c) => c.cmd === 'list_dir').map((c) => ({ path: c.args.path, ok: c.ok })),
    toasts,
    modal: modal ? modal.textContent.replace(/\s+/g, ' ').trim().slice(0, 120) : null,
    hitTestEl: hit ? (hit.closest('.pane') ? 'pane' : (hit.closest('#wsbar') ? '#wsbar' : (hit.id ? '#' + hit.id : hit.tagName + '.' + hit.className))) : null,
  };
}
"""

JS_HIGHLIGHTS = """
() => [...document.querySelectorAll('.pane')]
  .sort((a, b) => a.getBoundingClientRect().x - b.getBoundingClientRect().x)
  .map((el) => el.classList.contains('drop-target'))
"""

failures: list[str] = []


def check(cond: bool, msg: str) -> None:
    print(f"  [{'PASS' if cond else 'FAIL'}] {msg}")
    if not cond:
        failures.append(msg)


def serve():
    handler = lambda *a, **kw: http.server.SimpleHTTPRequestHandler(*a, directory=str(DIST), **kw)
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd, httpd.server_address[1]


def boot(browser, port, dsf, errs, ua):
    ctx = browser.new_context(
        viewport={"width": 1400, "height": 900},
        user_agent=ua,
        device_scale_factor=dsf,
    )
    try:
        pg = ctx.new_page()
        pg.add_init_script(SHIM)
        pg.on("pageerror", lambda e: errs.append(str(e)[:200]))
        pg.goto(f"http://127.0.0.1:{port}/index.html")
        pg.wait_for_function(
            "document.querySelectorAll('.pane').length >= 2 && (window.__sim.listeners['tauri://drag-drop']||[]).length > 0",
            timeout=15000,
        )
        pg.wait_for_timeout(400)
        return ctx, pg
    except Exception:
        ctx.close()
        raise


def pane_points(st):
    ps = sorted(st["panes"], key=lambda p: p["rect"]["x"])
    a, b = ps[0]["rect"], ps[1]["rect"]
    center = lambda r: (r["x"] + r["w"] / 2, r["y"] + r["h"] / 2)
    return {
        "paneA_center": center(a),
        "paneB_center": center(b),
        "sidebar": (st["wsbar"]["x"] + st["wsbar"]["w"] / 2, st["wsbar"]["y"] + st["wsbar"]["h"] / 2) if st["wsbar"] else (20, 400),
        "pane_boundary": ((a["x"] + a["w"] + b["x"]) / 2, a["y"] + a["h"] / 2),
        "paneA_topleft_inset": (a["x"] + 12, a["y"] + 40),
        "paneB_bottomright_inset": (b["x"] + b["w"] - 12, b["y"] + b["h"] - 12),
    }


def position(point, scale=1):
    x, y = point
    return {"x": x * scale, "y": y * scale}


def drop(pg, point, paths=None, scale=1):
    return pg.evaluate(JS_DROP, {
        "pos": position(point, scale),
        "css": position(point),
        "paths": [REPORT] if paths is None else paths,
        "settle": 250,
    })


def check_delivery(result, sid, msg):
    sends = result["send_input"]
    check(
        len(sends) == 1 and sends[0]["surfaceId"] == sid and sends[0]["ok"],
        f"{msg} (send_input={sends}, toasts={result['toasts']})",
    )


def emit(pg, name, payload=None):
    # drag-leave는 payload 속성/값 없이 호출한다.
    if payload is None:
        pg.evaluate("name => window.__emit(name)", name)
    else:
        pg.evaluate("({ name, payload }) => window.__emit(name, payload)", {"name": name, "payload": payload})
    pg.wait_for_timeout(100)


def check_highlights(pg, expected, msg):
    actual = pg.evaluate(JS_HIGHLIGHTS)
    check(actual == expected, f"{msg} (A/B={actual})")


def mac_retina(pg, points):
    check_delivery(drop(pg, points["paneB_center"]), 2, "macOS dsf=2 논리좌표 B → surfaceId 2")
    check_delivery(drop(pg, points["paneA_center"]), 1, "macOS dsf=2 논리좌표 A → surfaceId 1")


def highlights(pg, points):
    a, b = position(points["paneA_center"]), position(points["paneB_center"])
    emit(pg, "tauri://drag-enter", {"paths": [REPORT], "position": b})
    check_highlights(pg, [False, True], "drag-enter B → B만 drop-target")
    emit(pg, "tauri://drag-over", {"position": a})
    check_highlights(pg, [True, False], "drag-over A → A만 drop-target")
    emit(pg, "tauri://drag-leave")
    check_highlights(pg, [False, False], "payload 없는 drag-leave → 하이라이트 0개")
    emit(pg, "tauri://drag-enter", {"paths": [REPORT], "position": b})
    check_highlights(pg, [False, True], "재진입 B → B만 drop-target")
    result = drop(pg, points["paneB_center"])
    check_delivery(result, 2, "하이라이트 뒤 B 드롭 → surfaceId 2")
    check_highlights(pg, [False, False], "drag-drop 뒤 하이라이트 0개")


def error_messages(pg, points):
    point = points["paneB_center"]
    pg.evaluate("window.__sim.cfg.listDir='reject'")
    rejected = drop(pg, point)
    check(any("경로 확인 실패" in t for t in rejected["toasts"]), f"list_dir reject → 경로 확인 실패 ({rejected['toasts']})")
    pg.evaluate("window.__sim.cfg.listDir='missing'")
    missing = drop(pg, point)
    check(any("경로가 더 이상 없음" in t for t in missing["toasts"]), f"항목 부재 → 경로가 더 이상 없음 ({missing['toasts']})")
    rejected_first = rejected["toasts"][0] if rejected["toasts"] else ""
    missing_first = missing["toasts"][0] if missing["toasts"] else ""
    check(bool(rejected_first and missing_first and rejected_first != missing_first), "reject와 missing의 첫 토스트 문구가 다름")
    pg.evaluate("window.__sim.cfg.listDir='ok'")
    multi = drop(pg, point, paths=[REPORT, GONE])
    check(any("gone.bin" in t and "2/2" in t for t in multi["toasts"]), f"다중 경로 실패 → gone.bin·2/2 표시 ({multi['toasts']})")
    check(not multi["send_input"], f"다중 경로 중 하나 실패 → send_input 0건 ({multi['send_input']})")


def debug_switch(pg, points):
    check(pg.evaluate("localStorage.getItem('cysDropDebug')") is None, "새 컨텍스트 cysDropDebug 값 없음")
    default = drop(pg, points["paneB_center"])
    check(not any("dpr" in t.lower() for t in default["toasts"]), f"기본 설정 → dpr 진단 토스트 없음 ({default['toasts']})")
    check_delivery(default, 2, "진단 off → B 정상 주입")
    pg.evaluate("localStorage.setItem('cysDropDebug','1')")
    enabled = drop(pg, points["paneB_center"])
    check(any("dpr" in t.lower() for t in enabled["toasts"]), f"cysDropDebug=1 → dpr 진단 토스트 1건 이상 ({enabled['toasts']})")
    check_delivery(enabled, 2, "진단 on → B 정상 주입 유지")


def run_case(browser, port, label, ua, dsf, scenario):
    print(f"\n{label}")
    errs = []
    ctx = None
    try:
        ctx, pg = boot(browser, port, dsf, errs, ua)
        state = pg.evaluate(JS_STATE)
        check(state["dpr"] == dsf, f"{label}: devicePixelRatio={dsf} (got {state['dpr']})")
        check(pg.evaluate("navigator.userAgent") == ua, f"{label}: UA 고정")
        check(len(state["panes"]) == 2, f"{label}: pane A/B 2개")
        scenario(pg, pane_points(state))
    except Exception as exc:
        check(False, f"{label}: 게이트 실행 오류 {type(exc).__name__}: {exc}")
    finally:
        if ctx is not None:
            ctx.close()
        check(not errs, f"{label}: pageerror 0건 (got {errs})")


def main() -> int:
    if not (DIST / "index.html").exists():
        print("FAIL: ui/dist 없음 — 먼저 `sh ui/build.sh`")
        return 2
    from playwright.sync_api import sync_playwright

    httpd, port = serve()
    try:
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            try:
                run_case(browser, port, "1. macOS Retina 논리좌표", UA_MAC, 2, mac_retina)
                run_case(browser, port, "2. Windows Retina 물리좌표", UA_WIN, 2,
                         lambda pg, pts: check_delivery(drop(pg, pts["paneB_center"], scale=2), 2, "Windows dsf=2 CSS×2 → B"))
                run_case(browser, port, "3. Linux Retina 논리좌표", UA_LINUX, 2,
                         lambda pg, pts: check_delivery(drop(pg, pts["paneB_center"]), 2, "Linux dsf=2 논리좌표 → B"))
                run_case(browser, port, "4. macOS dsf=1 대조군", UA_MAC, 1,
                         lambda pg, pts: check_delivery(drop(pg, pts["paneB_center"]), 2, "macOS dsf=1 → B"))
                run_case(browser, port, "5. macOS Retina 하이라이트", UA_MAC, 2, highlights)
                run_case(browser, port, "6. 오류 문구·다중 경로 전부 중단", UA_MAC, 1, error_messages)
                run_case(browser, port, "7. macOS Retina 진단 스위치", UA_MAC, 2, debug_switch)
            finally:
                browser.close()
    finally:
        httpd.shutdown()
        httpd.server_close()
    print(f"\n{'PASS' if not failures else 'FAIL'} — 실패 {len(failures)}건")
    for failure in failures:
        print(f"  ✗ {failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
