import Cocoa
import WebKit

// 가상 시뮬레이션 1 — 설계 확정 코드(D1 스크립트 + D2 진단함수)의 전 경로를 동일 WebKit에서 구동.
// 시나리오: A=정상(arm 성공→ensure 성공) B=arm 실패(재생·enriched 진단) C=연타(dup 판정·이중 ensure 금지)
let scenario = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "A"

let app = NSApplication.shared
app.setActivationPolicy(.regular)

class Handler: NSObject, WKScriptMessageHandler {
    func userContentController(_ c: WKUserContentController, didReceive m: WKScriptMessage) {
        print("JS: \(m.body)"); fflush(stdout)
    }
}
let handler = Handler()
let config = WKWebViewConfiguration()
config.userContentController.add(handler, name: "log")

// ── 주입 순서는 계약이다 ─────────────────────────────────────────────────────
// tauri 2.11.2 실측 초기화 순서를 재현한다(이 순서를 틀리면 이번 결함=설치 시점 invoke
// 접촉을 시뮬이 놓친다): ① __TAURI_INTERNALS__={plugins:{}} 먼저 정의 → ② ipc-protocol
// 동형 스텁(postMessage만 정의, `})()\n`로 끝) + push_str 연결된 활성화 스크립트 → ③ **그
// 뒤** core.js 동형 스텁이 __TAURI_INTERNALS__.invoke를 정의한다. 즉 invoke는 활성화
// 스크립트 실행 이후에야 존재한다 — 활성화 스크립트가 설치 시점에 invoke를 잡으려 하면
// undefined.bind로 죽는다(v0.13.7 실사고). 설계 확정본은 invoke를 클릭 시점에 지연 조회한다.

let armBehavior = scenario == "B" ? "return Promise.reject('Browser bootstrap capability mismatch (simulated)');" : "S.pending = true; return Promise.resolve(null);"

// [a] __TAURI_INTERNALS__ 골격({plugins:{}}) — core.js 이전에 존재하나 invoke는 아직 없다.
let internalsBase = """
;(() => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: { plugins: {} }, configurable: true });
  window.webkit.messageHandlers.log.postMessage('internals-base-ran (invoke defined=' + (typeof window.__TAURI_INTERNALS__.invoke) + ')');
})()
"""

// [b] ipc-protocol.js 동형 말미(`})()\n`) — postMessage만 정의(invoke 없음). 실물처럼 식으로 끝난다.
let ipcProtocol = """
(function () {
  const log = (m) => window.webkit.messageHandlers.log.postMessage(m);
  Object.defineProperty(window.__TAURI_INTERNALS__, 'postMessage', { value: function (msg) { /* stub */ }, configurable: true });
  log('ipc-init-ran (invoke defined=' + (typeof window.__TAURI_INTERNALS__.invoke) + ')');
})()

"""

// [b'] D1 활성화 스크립트 — 실물(main.rs)에서 재추출(README '재추출 규칙': {{→{, {capability}→64hex).
// invoke를 설치 시점에 잡지 않고 클릭 핸들러 안에서 지연 조회한다.
let capability = String(repeating: "c", count: 64)
let d1 = """
;(() => {
  'use strict';
  const capability = '\(capability)';
  const mark = (s) => { try { window.__CYS_BROWSER_ACTIVATION__ = s; } catch (_) {} };
  mark('installed');
  const nativeClick = HTMLElement.prototype.click;
  document.addEventListener('click', (event) => {
    if (!event.isTrusted) return;
    const ua = navigator.userActivation;
    if (ua && ua.isActive === false) return;
    const raw = event.target;
    if (!(raw instanceof Element)) return;
    const target = raw.closest("#btn-browser, [data-browser-reconnect='true']");
    if (!target) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    const internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      mark('arm-failed: __TAURI_INTERNALS__.invoke unavailable at click time');
      return nativeClick.call(target);
    }
    internals.invoke('arm_browser_native_activation', { bootstrapCapability: capability })
      .then(() => { mark('armed-ok'); nativeClick.call(target); })
      .catch((e) => { mark('arm-failed: ' + e); nativeClick.call(target); });
  }, true);
})();
"""
let concatenated = ipcProtocol + d1  // append_invoke_initialization_script push_str 동형

// [c] core.js 동형 — **활성화 스크립트 뒤에** invoke를 정의한다(arm/ensure 레지스트리, TTL 제외).
let coreJs = """
;(() => {
  const log = (m) => window.webkit.messageHandlers.log.postMessage(m);
  const S = { pending: false };
  Object.defineProperty(window.__TAURI_INTERNALS__, 'invoke', {
    value: function (cmd, args) {
      log('invoke:' + cmd);
      if (cmd === 'arm_browser_native_activation') { \(armBehavior) }
      if (cmd === 'ensure_browserd_cast') {
        if (!S.pending) return Promise.reject('BROWSER_DISABLED_SAFE [NATIVE_ACTIVATION_REQUIRED]: trusted native Browser activation is required');
        S.pending = false; return Promise.resolve({ url: 'stub' });
      }
      return Promise.reject('unknown cmd');
    },
    configurable: true
  });
  log('core-init-ran (invoke defined=' + (typeof window.__TAURI_INTERNALS__.invoke) + ')');
})()
"""

config.userContentController.addUserScript(WKUserScript(source: """
window.addEventListener('error', (e) => window.webkit.messageHandlers.log.postMessage('WINDOW-ERROR: ' + e.message));
""", injectionTime: .atDocumentStart, forMainFrameOnly: true))
// 실물 tauri 순서: (a) __TAURI_INTERNALS__ 골격 → (b) ipc-protocol+활성화 스크립트 → (c) core.js가 invoke 정의
config.userContentController.addUserScript(WKUserScript(source: internalsBase, injectionTime: .atDocumentStart, forMainFrameOnly: true))
config.userContentController.addUserScript(WKUserScript(source: concatenated, injectionTime: .atDocumentStart, forMainFrameOnly: true))
config.userContentController.addUserScript(WKUserScript(source: coreJs, injectionTime: .atDocumentStart, forMainFrameOnly: true))

// [3] 페이지 — openCastPane dup 판정 + castActiveEnsure + D2 진단(설계 확정 순수함수) 재현
let html = """
<html><body style="margin:0">
<button id="btn-browser" style="position:fixed;left:0;top:0;width:400px;height:300px;font-size:30px">browser</button>
<script>
const log = (m) => window.webkit.messageHandlers.log.postMessage(m);
log('interceptor-marker=' + window.__CYS_BROWSER_ACTIVATION__);
// D2 순수함수(설계 확정본)
function castActivationDiagnostics(err, marker) {
  const raw = err === null || err === undefined ? "" : String(err);
  if (!raw.includes("NATIVE_ACTIVATION_REQUIRED")) return raw;
  if (marker === undefined || marker === null)
    return `활성화 스크립트 미설치(빌드 결함 — 재설치 필요) · ${raw}`;
  const m = String(marker);
  if (m.startsWith("arm-failed")) return `${m} · ${raw}`;
  return raw;
}
function castFailureReason(err, maxLen = 120) {
  const one = (err === null || err === undefined ? "" : String(err)).replace(/\\s+/g, " ").trim();
  return one.length <= maxLen ? one : `${one.slice(0, maxLen)}…`;
}
let paneExists = false;
let ensureCalls = 0;
document.getElementById('btn-browser').addEventListener('click', (e) => {
  log('HANDLER: click isTrusted=' + e.isTrusted + ' marker=' + window.__CYS_BROWSER_ACTIVATION__);
  if (paneExists) { log('openCastPane: dup -> focus-only'); return; }
  paneExists = true;
  ensureCalls++;
  window.__TAURI_INTERNALS__.invoke('ensure_browserd_cast', {})
    .then(() => log('ensure OK (calls=' + ensureCalls + ')'))
    .catch((err) => {
      const enriched = castActivationDiagnostics(err, window.__CYS_BROWSER_ACTIVATION__);
      log('ensure FAIL enriched=' + enriched);
      log('placeholder(' + castFailureReason(enriched).length + 'ch)=' + castFailureReason(enriched));
    });
});
log('page-ready');
</script>
</body></html>
"""

let rect = NSRect(x: 200, y: 200, width: 500, height: 400)
let window = NSWindow(contentRect: rect, styleMask: [.titled], backing: .buffered, defer: false)
let webView = WKWebView(frame: NSRect(x: 0, y: 0, width: 500, height: 400), configuration: config)
window.contentView = webView
window.makeKeyAndOrderFront(nil)
app.activate(ignoringOtherApps: true)
webView.loadHTMLString(html, baseURL: nil)

func click() {
    let loc = NSPoint(x: 100, y: webView.frame.height - 100)
    let t = ProcessInfo.processInfo.systemUptime
    let down = NSEvent.mouseEvent(with: .leftMouseDown, location: loc, modifierFlags: [], timestamp: t, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1)!
    window.sendEvent(down)
    let up = NSEvent.mouseEvent(with: .leftMouseUp, location: loc, modifierFlags: [], timestamp: t + 0.05, windowNumber: window.windowNumber, context: nil, eventNumber: 2, clickCount: 1, pressure: 0)!
    window.sendEvent(up)
}

DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { print("CLICK-1"); click() }
if scenario == "C" {
    DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) { print("CLICK-2"); click() }
}
DispatchQueue.main.asyncAfter(deadline: .now() + 4.0) { print("DONE scenario=\(scenario)"); app.terminate(nil) }
app.run()
