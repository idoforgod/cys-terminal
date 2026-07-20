// cast.ts — browserd in-pane screencast 순수·테스트 가능부.
// lib.ts 관례를 따른다: 부작용 없는 로직·문자열 상수만(브라우저/네트워크 의존 없음).
// server.ts 가 이 모듈의 CAST_APP_HTML 을 서빙하고 castRoute·parseClientMsg·mapInput 을 소비한다.

// --- WS 클라이언트 → 서버 메시지 (검증 후 타입) ---
export type ClientMsg =
  | { type: "ack"; fid: number }
  | {
      type: "mouse";
      kind: "pressed" | "released" | "moved" | "wheel";
      x: number;
      y: number;
      cw: number;
      ch: number;
      button: "left" | "right" | "middle" | "none";
      clickCount: number;
      modifiers: number;
      deltaX: number;
      deltaY: number;
    }
  | { type: "key"; kind: "down" | "up"; key: string; code: string; text: string; modifiers: number }
  | { type: "insertText"; text: string }
  | { type: "navigate"; url: string }
  | { type: "control"; action: "release" };

const TEXT_LIMIT = 4096;
const URL_LIMIT = 2048;
const KEY_LIMIT = 64; // key/code 는 짧은 DOM 식별자 — 범위 방어

// 경로 파서: /<token>/cast/ → app, /<token>/cast/ws → ws, 그 외 null.
// 순수 함수(bun test). 세그먼트 수 불일치·빈 토큰은 null.
export function castRoute(pathname: string): { kind: "app" | "ws"; token: string } | null {
  const parts = pathname.split("/").filter(Boolean);
  if (parts.length === 2 && parts[1] === "cast" && parts[0]) {
    // 설계 계약은 끝 슬래시가 있는 `/<token>/cast/` 뿐이다. 끝 슬래시 없는 형태는 거부(엄격화)
    // — GUI 가 조립하는 URL(castAppUrl)은 항상 슬래시를 포함한다.
    if (!pathname.endsWith("/")) return null;
    return { kind: "app", token: parts[0] };
  }
  if (parts.length === 3 && parts[1] === "cast" && parts[2] === "ws" && parts[0]) {
    return { kind: "ws", token: parts[0] };
  }
  return null;
}

// WS 클라이언트 메시지 검증. 허용 type·필드 타입·범위만 통과. 미지 type·불량 = null.
// 순수 함수. 신뢰 경계: 이 값이 그대로 CDP Input 으로 흘러가므로 여기서 좁힌다.
export function parseClientMsg(raw: string): ClientMsg | null {
  let m: any;
  try {
    m = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!m || typeof m !== "object") return null;
  const fin = (v: any) => typeof v === "number" && Number.isFinite(v);

  switch (m.type) {
    case "ack": {
      // fid = 서버 부여 단조 프레임 id. CDP sessionId 는 세션 상수라 클라이언트에 노출하지 않는다.
      if (!Number.isInteger(m.fid)) return null;
      return { type: "ack", fid: m.fid };
    }
    case "mouse": {
      const kinds = ["pressed", "released", "moved", "wheel"];
      const buttons = ["left", "right", "middle", "none"];
      if (!kinds.includes(m.kind)) return null;
      if (!fin(m.x) || !fin(m.y) || !fin(m.cw) || !fin(m.ch)) return null;
      return {
        type: "mouse",
        kind: m.kind,
        x: m.x,
        y: m.y,
        cw: m.cw,
        ch: m.ch,
        button: buttons.includes(m.button) ? m.button : "none",
        clickCount: fin(m.clickCount) ? m.clickCount : 0,
        modifiers: fin(m.modifiers) ? m.modifiers : 0,
        deltaX: fin(m.deltaX) ? m.deltaX : 0,
        deltaY: fin(m.deltaY) ? m.deltaY : 0,
      };
    }
    case "key": {
      if (m.kind !== "down" && m.kind !== "up") return null;
      if (typeof m.key !== "string" || typeof m.code !== "string") return null;
      if (m.key.length > KEY_LIMIT || m.code.length > KEY_LIMIT) return null;
      const text = typeof m.text === "string" ? m.text : "";
      if (text.length > TEXT_LIMIT) return null;
      return { type: "key", kind: m.kind, key: m.key, code: m.code, text, modifiers: fin(m.modifiers) ? m.modifiers : 0 };
    }
    case "insertText": {
      if (typeof m.text !== "string" || m.text.length > TEXT_LIMIT) return null;
      return { type: "insertText", text: m.text };
    }
    case "navigate": {
      if (typeof m.url !== "string" || m.url.length > URL_LIMIT) return null;
      return { type: "navigate", url: m.url };
    }
    case "control": {
      if (m.action !== "release") return null;
      return { type: "control", action: "release" };
    }
    default:
      return null;
  }
}

// canvas 표시 좌표 → letterbox 역변환 → viewport CSS px.
// meta.deviceWidth/Height 를 신뢰(하드코딩 금지 — 시뮬 F4). letterbox 여백 클릭은 null(무시).
// 순수 함수(bun test).
export function mapInput(
  m: { x: number; y: number; cw: number; ch: number },
  meta: { deviceWidth: number; deviceHeight: number; offsetTop?: number; pageScaleFactor?: number }
): { x: number; y: number } | null {
  const dw = meta.deviceWidth;
  const dh = meta.deviceHeight;
  const { x, y, cw, ch } = m;
  if (!(dw > 0) || !(dh > 0) || !(cw > 0) || !(ch > 0)) return null; // metadata 미보유·불량
  // 비율 유지 fit — canvas 안에 프레임을 중앙 letterbox 배치할 때의 배율.
  const scale = Math.min(cw / dw, ch / dh);
  const dispW = dw * scale;
  const dispH = dh * scale;
  const offX = (cw - dispW) / 2;
  const offY = (ch - dispH) / 2;
  const ix = x - offX;
  const iy = y - offY;
  // 우/하 경계는 등호 포함 배제 — ix===dispW 는 마지막 픽셀 다음이라 뷰포트 밖으로 매핑된다.
  if (ix < 0 || iy < 0 || ix >= dispW || iy >= dispH) return null; // letterbox 여백
  // 이미지 px → DIP.
  const dipX = ix / scale;
  const dipY = iy / scale;
  // offsetTop=콘텐츠 상단 오프셋(DIP)·pageScaleFactor=핀치줌. 데스크톱 고정 뷰포트에선 0·1이라
  // 항등이지만, 후속 pane 크기 연동에서 필요하므로 값을 신뢰해 반영한다(하드코딩 금지).
  const psf = meta.pageScaleFactor && meta.pageScaleFactor > 0 ? meta.pageScaleFactor : 1;
  const top = Number.isFinite(meta.offsetTop as number) ? (meta.offsetTop as number) : 0;
  const cssY = (dipY - top) / psf;
  if (cssY < 0) return null; // 콘텐츠 영역 위쪽(브라우저 크롬 자리) 클릭은 무시
  return { x: dipX / psf, y: cssY };
}

// screencast 앱 — 단일 HTML(인라인 CSS/JS, 외부 자산 0). browserd 가 자기 origin 에서 서빙한다.
// canvas letterbox 렌더 + 주소창 + control 상태 + WS 클라이언트 + 입력 캡처(마우스·키·한글 IME).
// CSP(default-src 'none'; script-src 'unsafe-inline' …)는 server.ts 응답 헤더가 강제.
export const CAST_APP_HTML = `<!doctype html>
<html lang="ko">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>cys 브라우저</title>
<style>
  html,body{margin:0;height:100%;background:#1a1a1a;color:#ddd;font:13px -apple-system,BlinkMacSystemFont,"Apple SD Gothic Neo",sans-serif;}
  body{display:flex;flex-direction:column;overflow:hidden;}
  #bar{display:flex;align-items:center;gap:6px;height:34px;padding:0 8px;background:#2a2a2a;border-bottom:1px solid #000;flex:0 0 auto;}
  #addr{flex:1;height:22px;border:1px solid #444;border-radius:4px;background:#111;color:#eee;padding:0 8px;outline:none;}
  #bar button{height:24px;border:1px solid #444;border-radius:4px;background:#333;color:#eee;cursor:pointer;padding:0 8px;}
  #bar button:hover{background:#3d3d3d;}
  #ctrl{font-size:12px;color:#8ab;min-width:60px;text-align:center;}
  #stage{flex:1 1 auto;position:relative;overflow:hidden;}
  #screen{position:absolute;inset:0;width:100%;height:100%;display:block;background:#1a1a1a;outline:none;}
  #overlay{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;text-align:center;padding:16px;background:rgba(20,20,20,0.85);color:#ccc;}
  #proxy{position:absolute;left:-9999px;top:0;width:1px;height:1px;opacity:0;}
</style>
</head>
<body>
<div id="bar">
  <input id="addr" type="text" placeholder="주소 입력 후 Enter" spellcheck="false" autocomplete="off">
  <button id="go">이동</button>
  <span id="ctrl">에이전트</span>
  <button id="hand">에이전트에게 넘기기</button>
</div>
<div id="stage">
  <canvas id="screen" tabindex="0"></canvas>
  <div id="overlay">연결 중...</div>
</div>
<input id="proxy" type="text" tabindex="-1" aria-hidden="true">
<script>
(function(){
  var seg = location.pathname.split('/').filter(Boolean);
  var token = seg[0] || '';
  var params = new URLSearchParams(location.search);
  var context = params.get('context') || 'default';
  var wsUrl = (location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/' + token + '/cast/ws?context=' + encodeURIComponent(context);

  var canvas = document.getElementById('screen');
  var g = canvas.getContext('2d');
  var addr = document.getElementById('addr');
  var goBtn = document.getElementById('go');
  var handBtn = document.getElementById('hand');
  var ctrlLabel = document.getElementById('ctrl');
  var overlay = document.getElementById('overlay');
  var proxy = document.getElementById('proxy');

  var ws = null;
  var hasFrame = false;   // 첫 frame(metadata) 전에는 입력 무시(설계 D3)
  var lastMoveSent = 0;

  function mods(e){ return (e.altKey?1:0)|(e.ctrlKey?2:0)|(e.metaKey?4:0)|(e.shiftKey?8:0); }
  function btn(e){ return e.button===0?'left':(e.button===1?'middle':(e.button===2?'right':'none')); }
  function send(o){ if (ws && ws.readyState === 1) ws.send(JSON.stringify(o)); }

  function showOverlay(text, clickable){
    overlay.textContent = text;
    overlay.style.display = 'flex';
    overlay.style.cursor = clickable ? 'pointer' : 'default';
    overlay._clickable = !!clickable;
  }
  function hideOverlay(){ overlay.style.display = 'none'; overlay._clickable = false; }

  overlay.addEventListener('click', function(){
    if (overlay._clickable) {
      // GUI 가 ensure 재발동·iframe 재로드하도록 부모에 알린다.
      try { window.parent.postMessage({ type: 'cys-cast-reconnect' }, '*'); } catch(e){}
    }
  });

  function fitCanvas(){ canvas.width = canvas.clientWidth; canvas.height = canvas.clientHeight; }

  function drawFrame(img){
    fitCanvas();
    var cw = canvas.width, ch = canvas.height;
    g.fillStyle = '#1a1a1a';
    g.fillRect(0, 0, cw, ch);
    var iw = img.naturalWidth || img.width, ih = img.naturalHeight || img.height;
    if (!iw || !ih) return;
    var scale = Math.min(cw / iw, ch / ih);
    var dw = iw * scale, dh = ih * scale;
    g.drawImage(img, (cw - dw) / 2, (ch - dh) / 2, dw, dh);
  }

  function onFrame(msg){
    var img = new Image();
    img.onload = function(){
      drawFrame(img);
      hasFrame = true;
      hideOverlay();
      send({ type: 'ack', fid: msg.fid }); // 그린 뒤 ack — e2e 배압. 서버가 원본 sessionId 로 CDP 릴레이.
    };
    // 디코드 실패해도 ack 는 보낸다 — 안 보내면 CDP 스트림이 영구 스톨한다.
    img.onerror = function(){ send({ type: 'ack', fid: msg.fid }); };
    img.src = 'data:image/jpeg;base64,' + msg.data;
  }

  function connect(){
    showOverlay('연결 중...', false);
    try { ws = new WebSocket(wsUrl); } catch(e){ showOverlay('연결 실패 — 클릭해 재연결', true); return; }
    ws.onopen = function(){ showOverlay('화면 대기 중...', false); };
    ws.onmessage = function(ev){
      var msg; try { msg = JSON.parse(ev.data); } catch(e){ return; }
      if (msg.type === 'frame') onFrame(msg);
      else if (msg.type === 'nav') {
        addr.value = msg.url || '';
        if (msg.title) document.title = msg.title;
        // 부모(cys GUI)의 pane 헤더 갱신용 — 수신·반영은 GUI 측 담당.
        try { window.parent.postMessage({ type: 'cys-cast-title', title: msg.title || '', url: msg.url || '' }, '*'); } catch(e){}
      }
      else if (msg.type === 'control') { ctrlLabel.textContent = (msg.control === 'human') ? '사람 조작 중' : '에이전트'; }
      else if (msg.type === 'err') { showOverlay('오류 [' + msg.code + '] ' + (msg.message || ''), false); }
    };
    ws.onclose = function(){ hasFrame = false; showOverlay('연결 끊김 — 클릭해 재연결', true); };
    ws.onerror = function(){ /* onclose 가 뒤따른다 */ };
  }

  // --- 마우스 입력(canvas) ---
  canvas.addEventListener('mousedown', function(e){
    proxy.focus(); // 키보드·IME 는 숨김 프록시로 라우팅
    if (!hasFrame) return;
    send({ type:'mouse', kind:'pressed', x:e.offsetX, y:e.offsetY, cw:canvas.clientWidth, ch:canvas.clientHeight, button:btn(e), clickCount:e.detail||1, modifiers:mods(e), deltaX:0, deltaY:0 });
  });
  canvas.addEventListener('mouseup', function(e){
    if (!hasFrame) return;
    send({ type:'mouse', kind:'released', x:e.offsetX, y:e.offsetY, cw:canvas.clientWidth, ch:canvas.clientHeight, button:btn(e), clickCount:e.detail||1, modifiers:mods(e), deltaX:0, deltaY:0 });
  });
  canvas.addEventListener('mousemove', function(e){
    if (!hasFrame) return;
    var now = Date.now();
    if (now - lastMoveSent < 30) return; // ~30ms 스로틀
    lastMoveSent = now;
    send({ type:'mouse', kind:'moved', x:e.offsetX, y:e.offsetY, cw:canvas.clientWidth, ch:canvas.clientHeight, button:'none', clickCount:0, modifiers:mods(e), deltaX:0, deltaY:0 });
  });
  canvas.addEventListener('wheel', function(e){
    if (!hasFrame) return;
    e.preventDefault();
    send({ type:'mouse', kind:'wheel', x:e.offsetX, y:e.offsetY, cw:canvas.clientWidth, ch:canvas.clientHeight, button:'none', clickCount:0, modifiers:mods(e), deltaX:e.deltaX, deltaY:e.deltaY });
  }, { passive: false });
  canvas.addEventListener('contextmenu', function(e){ e.preventDefault(); });

  // --- 키보드·한글 IME(숨김 프록시 input) ---
  // WKWebView 는 canvas 에 composition 이벤트를 주지 않는다 → off-screen input 을 포커스 프록시로
  // 삼아 compositionend 로 확정 텍스트를 받고, 조합 아닌 일반 키는 keydown 에서 잡아 프록시를 매번 비운다.
  proxy.addEventListener('keydown', function(e){
    if (!hasFrame) return;
    if (e.isComposing || e.keyCode === 229) return; // IME 조합 중 — 미전송(이중입력 방지)
    e.preventDefault();
    var text = (e.key && e.key.length === 1) ? e.key : ''; // 인쇄 가능 단일 문자만 text
    send({ type:'key', kind:'down', key:e.key, code:e.code, text:text, modifiers:mods(e) });
    proxy.value = '';
  });
  proxy.addEventListener('keyup', function(e){
    if (!hasFrame) return;
    if (e.isComposing || e.keyCode === 229) return;
    e.preventDefault();
    send({ type:'key', kind:'up', key:e.key, code:e.code, text:'', modifiers:mods(e) });
    proxy.value = '';
  });
  proxy.addEventListener('compositionend', function(e){
    if (!hasFrame) return;
    if (e.data) send({ type:'insertText', text:e.data });
    proxy.value = '';
  });

  // --- 주소창 이동 ---
  function navigate(){
    var u = (addr.value || '').trim();
    if (!u) return;
    if (u.indexOf('://') === -1) u = 'https://' + u; // 스킴 없으면 보정
    send({ type:'navigate', url:u });
  }
  goBtn.addEventListener('click', navigate);
  addr.addEventListener('keydown', function(e){ if (e.key === 'Enter') { e.preventDefault(); navigate(); } });

  // --- control: 에이전트에게 넘기기 ---
  handBtn.addEventListener('click', function(){ send({ type:'control', action:'release' }); });

  connect();
})();
</script>
</body>
</html>`;
