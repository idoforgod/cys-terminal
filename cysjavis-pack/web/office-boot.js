/* office-boot.js — 오피스 HUD 조용한 실패 제거 (W1-c · classic script, 모듈 아님)
 *
 * office3d.html의 ES 모듈 첫 줄 `import * as THREE from '/vendor/three.module.js'`가
 * 404면 모듈 전체가 죽어 "연결 중…" 정적 텍스트만 남는 침묵 실패가 됐다(실사고).
 * 이 스크립트는 브리지가 </head> 직전에 주입하는 classic script로, 그 침묵을 깨고
 * 복구 안내 배너를 띄운다. 외부 의존 0 · office3d.html 본문 무접촉.
 *
 * THREE 성공 신호(본문 무접촉): office3d.html은 <canvas id="scene">를 정적으로
 * 항상 포함하므로(본문 118행) '캔버스 존재'는 성공 신호가 될 수 없다. 대신
 * renderer.setSize(innerWidth,innerHeight)(본문 325·329행)가 성공 시 캔버스의
 * 인라인 style.width를 '<px>'로, 백버퍼 width를 화면폭*pixelRatio(>300)로 채운다 —
 * 둘 다 비파괴 판독이 가능한 성공 지문이다.
 *
 * 0.14.44 (B5): ㉠ 앱 안(iframe)에서도 배너가 보인다 · 문구에 명령어 없음 + 세부 한 줄
 *   ㉡ 자동 다시 불러오기 3→6→12→24→48초 5회(횟수는 주소의 ob_retry 인자 — 새로 고침을 넘어 기억)
 *   ㉢ 15초마다 /health 의 기동 식별자(boot_id)를 비교 — 둘 다 있고 다를 때만 새로 고침(세션당 10회 상한).
 *   구형 WKWebView 문법 주의: 새 문법(뒤돌아보기 정규식 · 배열 at · 문자열 전체치환)을 쓰지 않는다.
 */
(function () {
  "use strict";

  var BANNER_ID = "office-boot-banner";
  var MSG_RETRYING = "오피스 화면을 불러오지 못했습니다. 자동으로 다시 시도합니다.";
  var MSG_GAVE_UP = "오피스 화면을 불러오지 못했습니다.";
  var RETRY_DELAYS = [3, 6, 12, 24, 48];       // 초 — 5회까지만
  var RETRY_PARAM = "ob_retry";
  var HEALTH_EVERY_MS = 15000;
  var REFRESH_CAP = 10;                         // 앱 세션당 health 새로 고침 상한
  var REFRESH_KEY = "office-boot-refresh";
  var REFRESH_PARAM = "ob_ref";                 // 저장소를 못 쓸 때의 대체(주소 인자)

  var failedUrl = null;                         // 마지막으로 실패한 자원 주소(세부 줄용)
  var reloadTimer = null;

  // THREE 렌더 성공 여부 — setSize가 남기는 비파괴 지문으로 판정(본문 무접촉)
  function threeIsUp() {
    var c = document.getElementById("scene") ||
            (document.querySelector ? document.querySelector("canvas") : null);
    if (!c) return false;
    var styled = c.style && c.style.width !== "" && c.style.width != null;
    var buffered = typeof c.width === "number" && c.width > 300;
    return !!(styled || buffered);
  }

  function getParamNum(name) {
    try {
      var m = new RegExp("[?&]" + name + "=(\\d+)").exec(window.location.search || "");
      return m ? parseInt(m[1], 10) : 0;
    } catch (e) { return 0; }
  }

  // 주소에서 인자를 지우거나(value==null) 바꾼 새 주소 — 다른 인자·해시는 그대로
  function urlWith(name, value) {
    var loc = window.location;
    var q = (loc.search || "").replace(new RegExp("([?&])" + name + "=\\d+&?"), "$1");
    q = q.replace(/[?&]$/, "");
    if (value != null) q += (q.indexOf("?") === -1 ? "?" : "&") + name + "=" + value;
    return loc.pathname + q + (loc.hash || "");
  }

  function fileName(u) {
    try { return String(u).split("?")[0].split("/").pop() || String(u); } catch (e) { return String(u); }
  }

  // 세부 줄 — 「세부: three.module.js — 응답 404」 (피드백을 보낼 때 쓰인다)
  function describe(url, done) {
    var probe = url || "/vendor/three.module.js";
    var name = fileName(probe);
    if (typeof fetch !== "function") { done(url ? "세부: " + name + " — 불러오지 못함" : ""); return; }
    try {
      fetch(probe, { cache: "no-store" }).then(function (r) {
        if (r.status !== 200) done("세부: " + name + " — 응답 " + r.status);
        else if (url) done("세부: " + name + " — 불러오지 못함");
        else done("세부: 화면 그리기가 시작되지 않았습니다");
      }, function () {
        done("세부: " + name + " — 연결 실패");
      });
    } catch (e) { done(""); }
  }

  function scheduleReload() {
    var n = getParamNum(RETRY_PARAM);
    if (n >= RETRY_DELAYS.length) return false;     // 5회에서 멈춘다
    if (reloadTimer) return true;
    reloadTimer = setTimeout(function () {
      reloadTimer = null;
      if (threeIsUp()) return;                       // 그새 떴으면 취소
      window.location.href = urlWith(RETRY_PARAM, n + 1);
    }, RETRY_DELAYS[n] * 1000);
    return true;
  }

  function showBanner() {
    if (!document.body) {  // <head> 실행 시점엔 body 미파싱 — DOM 준비 후 재시도
      document.addEventListener("DOMContentLoaded", showBanner, { once: true });
      return;
    }
    if (document.getElementById(BANNER_ID)) return;   // 중복 생성 가드
    if (threeIsUp()) return;                           // 그새 렌더됐으면 취소

    var willRetry = scheduleReload();
    var b = document.createElement("div");
    b.id = BANNER_ID;
    b.setAttribute("role", "alert");
    var main = document.createElement("div");
    main.textContent = willRetry ? MSG_RETRYING : MSG_GAVE_UP;
    var detail = document.createElement("div");
    detail.style.cssText = "font-weight:400;font-size:11px;opacity:.8;margin-top:4px";
    b.appendChild(main);
    b.appendChild(detail);
    var s = b.style;
    s.position = "fixed";
    s.top = "0";
    s.left = "0";
    s.right = "0";
    s.zIndex = "2147483647";
    s.padding = "12px 16px";
    s.background = "#7a1420";
    s.color = "#ffe8ea";
    s.font = "600 14px/1.4 'SF Mono',Menlo,monospace";
    s.textAlign = "center";
    s.letterSpacing = "0.02em";
    s.boxShadow = "0 2px 12px rgba(0,0,0,.5)";
    s.borderBottom = "1px solid #b0303f";
    document.body.appendChild(b);
    describe(failedUrl, function (line) { detail.textContent = line; });
  }

  // 화면이 뒤늦게라도 정상으로 뜨면 배너를 지운다(느린 PC·부하 중에 3초를 넘겨 뜬 경우 정상 화면 위에 실패 문구가 남지 않게).
  function clearBannerIfUp() {
    if (!threeIsUp()) return;
    var b = document.getElementById(BANNER_ID);
    if (b && b.parentNode) b.parentNode.removeChild(b);
  }
  setInterval(clearBannerIfUp, 1000);

  // 빠른 경로: 모듈/자산 로드 실패 포착. 리소스 로드 에러는 window.onerror로
  // 버블하지 않으므로 캡처 단계 리스너로 잡는다(three.module.js 404 포함).
  // 리소스 로드 에러는 message가 없다 — 런타임 예외(message 有)와 구별해
  // 자산 로드 실패만 즉시 배너 트리거하고, 실제 렌더 성공 시엔 무시한다.
  window.addEventListener("error", function (e) {
    var isResourceError = e && !e.message &&
      e.target && e.target !== window &&
      (e.target.tagName === "SCRIPT" || e.target.tagName === "LINK" ||
       e.target.src || e.target.href);
    if (isResourceError && !threeIsUp()) {
      failedUrl = e.target.src || e.target.href || failedUrl;
      showBanner();
    }
  }, true);

  // 백스톱: 3초 내 THREE 미렌더면(에러 이벤트 무발생 침묵 실패 포함) 배너.
  setTimeout(function () {
    if (!threeIsUp()) showBanner();
  }, 3000);

  // ㉢ 브리지 교체 감지 — /health 의 boot_id 가 앞서 읽은 값과 둘 다 있고 다를 때만 한 번 새로 고친다.
  //   응답이 없거나 /health 가 없는 옛 브리지면 아무것도 하지 않는다(브리지가 내려가 있는 동안 되풀이 방지).
  function refreshCount() {
    try {
      var v = parseInt(window.sessionStorage.getItem(REFRESH_KEY) || "", 10);
      if (!isNaN(v)) return v;
      return 0;
    } catch (e) { return getParamNum(REFRESH_PARAM); }
  }

  function bumpRefreshAndReload(n) {
    var next = n + 1;
    var stored = false;
    try { window.sessionStorage.setItem(REFRESH_KEY, String(next)); stored = true; } catch (e) { stored = false; }
    var u = urlWith(RETRY_PARAM, null);                  // 새 브리지에서는 재시도 횟수를 처음부터
    if (!stored) {
      var loc = window.location;
      var q = u.indexOf("?") === -1 ? "" : u.slice(u.indexOf("?"));
      var base = u.indexOf("?") === -1 ? u : u.slice(0, u.indexOf("?"));
      q = q.replace(new RegExp("([?&])" + REFRESH_PARAM + "=\\d+&?"), "$1").replace(/[?&]$/, "");
      u = base + q + (q.indexOf("?") === -1 ? "?" : "&") + REFRESH_PARAM + "=" + next + (loc.hash || "");
    }
    window.location.href = u;
  }

  var baseId = null;
  var pending = false;
  function pollHealth() {
    if (pending || typeof fetch !== "function") return;
    pending = true;
    var ctl = null;
    var guard = null;
    try {
      if (typeof AbortController === "function") {
        ctl = new AbortController();
        guard = setTimeout(function () { try { ctl.abort(); } catch (e) { /* 무시 */ } }, 5000);
      }
    } catch (e) { ctl = null; }
    var opts = { cache: "no-store" };
    if (ctl) opts.signal = ctl.signal;
    fetch("/health", opts).then(function (r) {
      if (r.status !== 200) return null;
      return r.json();
    }).then(function (j) {
      pending = false;
      if (guard) clearTimeout(guard);
      var id = j && typeof j.boot_id === "string" && j.boot_id ? j.boot_id : null;
      if (!id) return;                                   // 응답 없음·옛 브리지 — 아무것도 하지 않는다
      if (threeIsUp() && getParamNum(RETRY_PARAM) > 0) {
        try { window.history.replaceState(null, "", urlWith(RETRY_PARAM, null)); } catch (e) { /* 무시 */ }
      }
      if (baseId === null) { baseId = id; return; }
      if (id !== baseId) {
        var n = refreshCount();
        if (n >= REFRESH_CAP) return;                    // 세션당 10회 상한
        bumpRefreshAndReload(n);
      }
    }).then(null, function () {
      pending = false;
      if (guard) clearTimeout(guard);
    });
  }
  pollHealth();
  setInterval(pollHealth, HEALTH_EVERY_MS);
})();
