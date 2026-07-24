import Cocoa
import WebKit

// 실증 2 — tauri push_str 연결을 그대로 재현: ipc-protocol.js 동형(})()\n로 끝) + cys 인터셉터 IIFE.
// 기대: 첫 IIFE는 실행되지만 반환값(undefined) 호출로 TypeError → 인터셉터 미설치.

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

class Handler: NSObject, WKScriptMessageHandler {
    func userContentController(_ c: WKUserContentController, didReceive m: WKScriptMessage) {
        print("JS: \(m.body)"); fflush(stdout)
    }
}

let handler = Handler()
let config = WKWebViewConfiguration()
config.userContentController.add(handler, name: "log")

// tauri 기본 invoke init 스크립트의 구조적 동형(마지막 바이트가 `})()` + 개행)
let defaultInit = """
(function () {
  window.webkit.messageHandlers.log.postMessage('default-init-ran');
  Object.defineProperty(window, '__X__', { value: { ok: true } })
})()

"""
// cys-terminal browser_activation_initialization_script 동형(IIFE로 시작)
let appended = """
(() => {
  'use strict';
  window.webkit.messageHandlers.log.postMessage('interceptor-installed');
  document.addEventListener('click', () => {}, true);
})();
"""
let concatenated = defaultInit + appended  // Builder::append_invoke_initialization_script = push_str

let errTrap = """
window.addEventListener('error', (e) => window.webkit.messageHandlers.log.postMessage('ERROR: ' + e.message));
"""
config.userContentController.addUserScript(WKUserScript(source: errTrap, injectionTime: .atDocumentStart, forMainFrameOnly: true))
config.userContentController.addUserScript(WKUserScript(source: concatenated, injectionTime: .atDocumentStart, forMainFrameOnly: true))

let webView = WKWebView(frame: NSRect(x: 0, y: 0, width: 300, height: 200), configuration: config)
webView.loadHTMLString("<html><body>x</body></html>", baseURL: nil)

DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) {
    webView.evaluateJavaScript("typeof window.__X__") { r, _ in
        print("EVAL: __X__ typeof = \(r ?? "nil")")
        app.terminate(nil)
    }
}
app.run()
