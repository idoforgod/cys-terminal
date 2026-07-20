// cast-integration.test.ts — cast WS 통합(headless 실기동).
// HOME 을 임시 디렉토리로 격리한 서브프로세스로 server.ts 를 띄워 실 ~/.cys 오염을 막는다.
// 첫 frame 수신 → ack → 스킴 거부 → 마우스 입력 왕복(카운터) → http nav → 토큰/Origin 게이트 → 생존.
import { test, expect } from "bun:test";
import { mkdtempSync, rmSync, existsSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BROWSERD_DIR = join(import.meta.dir, "..");

// ★자식 프로세스 stderr 회수 — **실패했을 때만**, 그리고 **절대 무한 대기하지 않는다**.
// proc.kill() 로 bun 을 죽여도 playwright 가 띄운 Chrome 이 그 파이프를 상속해 살아있으면
// 스트림이 EOF 에 도달하지 않아 `.text()` 가 영원히 멈춘다(테스트 180초 타임아웃으로 발현 —
// 단언 실패가 아니라 정리 블록의 교착이었다). 진단 정보를 잃는 것보다 **게이트가 멈추는 것이
// 더 나쁘다** — 3초 안에 못 읽으면 그 사실만 기록하고 넘어간다.
async function readStderr(proc: any, ms = 3000): Promise<string> {
  try {
    return await Promise.race([
      new Response(proc.stderr).text(),
      new Promise<string>((r) => setTimeout(() => r("(stderr 수집 타임아웃 — 자식 프로세스가 파이프 보유 중)"), ms)),
    ]);
  } catch {
    return "(stderr 수집 실패)";
  }
}

function sleep(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

async function waitState(home: string, timeoutMs: number): Promise<any> {
  const p = join(home, ".cys", "browser", "state.json");
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (existsSync(p)) {
      try {
        const st = JSON.parse(readFileSync(p, "utf8"));
        if (st.port && st.token) return st;
      } catch {}
    }
    await sleep(200);
  }
  throw new Error("state.json 미생성(타임아웃)");
}

// 특정 조건 메시지를 명시 타임아웃으로 대기(프레임 등 다른 메시지는 무시).
function nextMsg(ws: WebSocket, pred: (m: any) => boolean, timeoutMs: number, label: string): Promise<any> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      ws.removeEventListener("message", onMsg);
      reject(new Error("타임아웃 대기: " + label));
    }, timeoutMs);
    function onMsg(ev: MessageEvent) {
      let m: any;
      try {
        m = JSON.parse(ev.data as string);
      } catch {
        return;
      }
      if (pred(m)) {
        clearTimeout(timer);
        ws.removeEventListener("message", onMsg);
        resolve(m);
      }
    }
    ws.addEventListener("message", onMsg);
  });
}

// open 성공 여부(실패=error/close before open)를 boolean 으로.
function wsOpen(ws: WebSocket, timeoutMs: number): Promise<boolean> {
  return new Promise((resolve) => {
    let done = false;
    const fin = (v: boolean) => {
      if (!done) {
        done = true;
        clearTimeout(timer);
        resolve(v);
      }
    };
    const timer = setTimeout(() => fin(false), timeoutMs);
    ws.addEventListener("open", () => fin(true));
    ws.addEventListener("error", () => fin(false));
    ws.addEventListener("close", () => fin(false));
  });
}

async function rpc(port: number, token: string, verb: string, args?: any): Promise<any> {
  const res = await fetch(`http://127.0.0.1:${port}/${token}/rpc`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ verb, args: args || {} }),
  });
  return res.json();
}

test("cast WS 통합: 프레임·입력 왕복·스킴/토큰/Origin 게이트·생존", async () => {
  const home = mkdtempSync(join(tmpdir(), "cast-it-"));
  const proc = Bun.spawn(["bun", "run", "server.ts", "--headless"], {
    cwd: BROWSERD_DIR,
    env: { ...process.env, HOME: home, CYS_BROWSER_HEADLESS: "1" },
    stdout: "pipe",
    stderr: "pipe",
  });
  let failed = false;
  try {
    const st = await waitState(home, 30000);
    const port: number = st.port;
    const token: string = st.token;
    const base = `ws://127.0.0.1:${port}/${token}/cast/ws?context=default`;

    // ① WS 접속(Origin 없음) → 첫 frame 수신 → ack
    const ws = new WebSocket(base);
    expect(await wsOpen(ws, 10000)).toBe(true);
    const frame = await nextMsg(ws, (m) => m.type === "frame", 20000, "first frame");
    expect(frame.metadata).toBeTruthy();
    const dw: number = frame.metadata.deviceWidth;
    const dh: number = frame.metadata.deviceHeight;
    expect(dw).toBeGreaterThan(0);
    expect(dh).toBeGreaterThan(0);
    expect(Number.isInteger(frame.fid)).toBe(true); // 서버 부여 fid
    expect(frame.sessionId).toBeUndefined(); // CDP sessionId 는 클라이언트에 노출 안 함
    ws.send(JSON.stringify({ type: "ack", fid: frame.fid }));

    // ② navigate file:// → err SCHEME_DENIED
    const errP = nextMsg(ws, (m) => m.type === "err" && m.code === "SCHEME_DENIED", 5000, "SCHEME_DENIED");
    ws.send(JSON.stringify({ type: "navigate", url: "file:///etc/hosts" }));
    await errP;

    // ②-b javascript: 도 거부(스킴 게이트 견고성)
    const errJs = nextMsg(ws, (m) => m.type === "err" && m.code === "SCHEME_DENIED", 5000, "SCHEME_DENIED(javascript:)");
    ws.send(JSON.stringify({ type: "navigate", url: "javascript:alert(1)" }));
    await errJs;

    // ④ 마우스 입력 왕복(about:blank) — 카운터 심고 클릭·release 후 판독(HUMAN_ACTIVE 회피)
    const seed = await rpc(port, token, "eval", {
      expression: "(function(){window.__t=0;document.addEventListener('mousedown',function(){window.__t++;});return 'seeded';})()",
    });
    expect(seed.ok).toBe(true);
    // cw/ch=metadata 값 → scale 1·letterbox 0 → 중앙 클릭이 뷰포트 중앙에 매핑.
    const cx = Math.floor(dw / 2);
    const cy = Math.floor(dh / 2);
    ws.send(JSON.stringify({ type: "mouse", kind: "pressed", x: cx, y: cy, cw: dw, ch: dh, button: "left", clickCount: 1, modifiers: 0, deltaX: 0, deltaY: 0 }));
    ws.send(JSON.stringify({ type: "mouse", kind: "released", x: cx, y: cy, cw: dw, ch: dh, button: "left", clickCount: 1, modifiers: 0, deltaX: 0, deltaY: 0 }));
    ws.send(JSON.stringify({ type: "control", action: "release" })); // control 반환 → eval 판독 가능
    let clicked = 0;
    for (let i = 0; i < 40; i++) {
      const r = await rpc(port, token, "eval", { expression: "window.__t" });
      if (r.ok && typeof r.result.result === "number" && r.result.result >= 1) {
        clicked = r.result.result;
        break;
      }
      await sleep(150);
    }
    expect(clicked).toBeGreaterThanOrEqual(1);

    // ③ http navigate(자기 앱 페이지) → nav 메시지
    // Phase 3: nav 는 **로딩 시작 시점에도** 온다(진행 바 근거) — 그때 url 은 아직 이전 주소다.
    // 따라서 "첫 nav" 가 아니라 **정착한 nav**(목적지 + loading=false)를 기다린다(핀 강화).
    const navLoading = nextMsg(ws, (m) => m.type === "nav" && m.loading === true, 15000, "로딩 시작 nav");
    const navP = nextMsg(ws, (m) => m.type === "nav" && String(m.url).includes("/cast/") && m.loading === false, 20000, "정착 nav");
    // 스킴 대소문자 변형(HtTp:)은 통과가 정상 — 스킴 판정이 대소문자 무관이라서. nav 도달로 단언.
    ws.send(JSON.stringify({ type: "navigate", url: `HtTp://127.0.0.1:${port}/${token}/cast/` }));
    await navLoading; // 로딩 시작이 사람에게 알려진다(pre-commit 구간 포함)
    const nav = await navP;
    expect(String(nav.url)).toContain("/cast/");

    ws.close();
    await sleep(300);

    // ⑤ 잘못된 토큰 WS → 실패
    const badTok = new WebSocket(`ws://127.0.0.1:${port}/deadbeefdeadbeefdeadbeefdeadbeef/cast/ws?context=default`);
    expect(await wsOpen(badTok, 5000)).toBe(false);

    // ⑥ 잘못된 Origin → 실패
    const badOrigin = new WebSocket(base, { headers: { Origin: "http://evil.example" } } as any);
    expect(await wsOpen(badOrigin, 5000)).toBe(false);

    // ⑦ RPC status → 프로세스 생존
    const status = await rpc(port, token, "status", {});
    expect(status.ok).toBe(true);
    expect(typeof status.result.pid).toBe("number");
  } catch (e) {
    failed = true;
    throw e;
  } finally {
    proc.kill();
    // 정상 경로에서는 stderr 를 아예 읽지 않는다(무한 대기 위험을 감수할 이유가 없다).
    if (failed) console.error("=== server stderr ===\n" + (await readStderr(proc)));
    try {
      rmSync(home, { recursive: true, force: true });
    } catch {}
  }
}, 90000);

// 연속 애니메이션 페이지 — 매 프레임 paint 를 만들어 스트림이 흐르는지 직접 관측한다.
const ANIM_URL =
  "data:text/html,<style>@keyframes s{from{transform:translateX(0)}to{transform:translateX(600px)}}" +
  "div{width:200px;height:200px;background:red;animation:s .8s linear infinite}</style><div></div>";

// 회귀 핀: CDP sessionId 를 dedup 키로 쓰면 첫 ack 이후 전부 차단되어 스트림이 영구 스톨한다(0fps).
// fid dedup 이어야 계속 흐른다. 동시에 30fps 상한이 실제로 걸리는지도 단언한다.
test("cast 지속 스트림: fid ack 로 계속 흐르고 30fps 상한이 걸린다 + 다중 클라이언트 dedup", async () => {
  const home = mkdtempSync(join(tmpdir(), "cast-stream-"));
  const proc = Bun.spawn(["bun", "run", "server.ts", "--headless"], {
    cwd: BROWSERD_DIR,
    env: { ...process.env, HOME: home, CYS_BROWSER_HEADLESS: "1" },
    stdout: "pipe",
    stderr: "pipe",
  });
  let failed = false;
  const sockets: WebSocket[] = [];
  try {
    const st = await waitState(home, 30000);
    const port: number = st.port;
    const token: string = st.token;
    const wsUrl = `ws://127.0.0.1:${port}/${token}/cast/ws?context=default`;

    // 애니메이션 페이지를 먼저 로드(RPC open — 이 시점 control=agent).
    const opened = await rpc(port, token, "open", { url: ANIM_URL });
    expect(opened.ok).toBe(true);

    // --- 1단계: 단일 클라이언트 지속 스트림 ---
    const a = new WebSocket(wsUrl);
    sockets.push(a);
    expect(await wsOpen(a, 10000)).toBe(true);

    let countA = 0;
    let counting = false;
    const fidsA: number[] = [];
    a.addEventListener("message", (ev) => {
      let m: any;
      try {
        m = JSON.parse(ev.data as string);
      } catch {
        return;
      }
      if (m.type !== "frame") return;
      a.send(JSON.stringify({ type: "ack", fid: m.fid })); // 매 프레임 ack
      if (counting) {
        countA++;
        fidsA.push(m.fid);
      }
    });

    await sleep(2000); // 워밍업 제외
    // [신뢰 경계] 서버가 발급하지 않은 fid 를 대량 선행 ack — 서버 상태가 오염되면 스트림이
    // 스톨한다. 서버는 자기가 발급한 미확인 fid 만 유효로 보므로 전부 무시되어야 한다.
    for (let i = 0; i < 200; i++) a.send(JSON.stringify({ type: "ack", fid: 900000 + i }));
    const statBefore = await castStat(port, token);
    counting = true;
    await sleep(4000); // 측정 4초
    counting = false;

    const fps = countA / 4;
    console.log(`[지속 스트림] 단일 클라이언트 4초간 ${countA}프레임 = ${fps.toFixed(1)} fps`);
    expect(countA).toBeGreaterThanOrEqual(20); // sessionId dedup 결함이면 0
    expect(countA).toBeLessThanOrEqual(140); // 4초×35fps — 상한 부재면 ~92fps(≈368)로 초과
    for (let i = 1; i < fidsA.length; i++) expect(fidsA[i]).toBeGreaterThan(fidsA[i - 1]); // fid 단조 증가

    // --- 2단계: 클라이언트 2개가 같은 fid 를 각각 ack 해도 스트림 유지(dedup) ---
    const b = new WebSocket(wsUrl);
    sockets.push(b);
    expect(await wsOpen(b, 10000)).toBe(true);

    let count2A = 0;
    let count2B = 0;
    let counting2 = false;
    const seenA: number[] = [];
    const seenB: number[] = [];
    b.addEventListener("message", (ev) => {
      let m: any;
      try {
        m = JSON.parse(ev.data as string);
      } catch {
        return;
      }
      if (m.type !== "frame") return;
      b.send(JSON.stringify({ type: "ack", fid: m.fid })); // B 도 같은 fid 를 ack — 중복 ack 유발
      if (counting2) {
        count2B++;
        seenB.push(m.fid);
      }
    });
    a.addEventListener("message", (ev) => {
      let m: any;
      try {
        m = JSON.parse(ev.data as string);
      } catch {
        return;
      }
      if (m.type === "frame" && counting2) {
        count2A++;
        seenA.push(m.fid);
      }
    });

    await sleep(1000); // B 합류 안정화
    const dedupBefore = await castStat(port, token);
    counting2 = true;
    await sleep(3000); // 측정 3초
    counting2 = false;
    const dedupAfter = await castStat(port, token);

    console.log(`[다중 클라이언트] 3초간 A=${count2A} B=${count2B} 프레임`);
    expect(count2A).toBeGreaterThanOrEqual(15); // 중복 ack 로 스트림이 죽지 않아야 한다
    expect(count2B).toBeGreaterThanOrEqual(15);
    // 두 클라이언트는 동일 브로드캐스트를 본다 — 공통 fid 가 존재해야 한다(같은 프레임을 각각 ack 했다는 증거).
    const common = seenA.filter((f) => seenB.includes(f));
    expect(common.length).toBeGreaterThan(0);

    // ★ack dedup 계약: 클라이언트 2개가 같은 fid 를 각각 ack 해도 CDP 릴레이는 fid당 1회다.
    // 따라서 릴레이 증가분은 push 증가분을 넘을 수 없다(dedup 제거 시 ≈2배가 되어 실패).
    const pushedDelta = dedupAfter.framesPushed - dedupBefore.framesPushed;
    const relayDelta = dedupAfter.ackRelayed - dedupBefore.ackRelayed;
    console.log(`[ack dedup] push=${pushedDelta} relay=${relayDelta} (relay ≤ push+1 이어야 함)`);
    expect(pushedDelta).toBeGreaterThan(0);
    // ★경계 표본 오차 +1 허용: before/after 두 status 스냅샷은 서로 다른 순간에 찍히므로,
    // before 직전에 push 된 프레임이 before 직후에 ack 되면 relay 가 push 보다 1 커 보인다
    // (제품 결함이 아니라 계측 경계 현상 — 부하가 있을 때 ~5회 중 1회 재현).
    // dedup 이 없으면 relay 는 push 의 **약 2배**가 되므로 +1 여유는 계약을 전혀 무르게 하지 않는다.
    expect(relayDelta).toBeLessThanOrEqual(pushedDelta + 1);
    expect(relayDelta).toBeLessThan(pushedDelta * 2 - 5); // dedup 제거 시 ≈2배 → 그때는 반드시 실패
    // 선행 ack(미발급 fid 200건)가 서버 상태를 오염시키지 않았음 — 측정 구간 내내 스트림이 살아있었다.
    expect(statBefore.framesPushed).toBeGreaterThan(0);

    // --- 3단계: WS 전부 close → stopScreencast → 재접속 시 자기회복(다시 흐른다) ---
    // 한계 정직 고지: stopScreencast 호출 자체는 외부에서 직접 관측할 수 없다. 여기서 단언하는 것은
    // "전 클라이언트 종료 후 재접속하면 스트림이 정상 재개된다"는 stop→start 사이클의 관측 가능한 결과다.
    a.close();
    b.close();
    await sleep(800); // 마지막 close 처리(stop+detach) 여유

    const c = new WebSocket(wsUrl);
    sockets.push(c);
    expect(await wsOpen(c, 10000)).toBe(true);
    let count3 = 0;
    let counting3 = false;
    c.addEventListener("message", (ev) => {
      let m: any;
      try {
        m = JSON.parse(ev.data as string);
      } catch {
        return;
      }
      if (m.type !== "frame") return;
      c.send(JSON.stringify({ type: "ack", fid: m.fid }));
      if (counting3) count3++;
    });
    await sleep(1000); // 재기동 워밍업
    counting3 = true;
    await sleep(2000);
    counting3 = false;
    console.log(`[재접속 자기회복] 2초간 ${count3}프레임`);
    expect(count3).toBeGreaterThanOrEqual(10); // 재개 실패면 0
  } catch (e) {
    failed = true;
    throw e;
  } finally {
    for (const s of sockets) {
      try {
        s.close();
      } catch {}
    }
    proc.kill();
    // 정상 경로에서는 stderr 를 아예 읽지 않는다(무한 대기 위험을 감수할 이유가 없다).
    if (failed) console.error("=== server stderr ===\n" + (await readStderr(proc)));
    try {
      rmSync(home, { recursive: true, force: true });
    } catch {}
  }
}, 120000);

// status 의 cast 계측(hubs·clients·started·stopped·framesPushed·ackRelayed) 관측.
async function castStat(port: number, token: string): Promise<any> {
  const r = await rpc(port, token, "status", {});
  return r?.result?.cast;
}

// status 동사로 컨텍스트의 control 을 직접 관측(status 는 control 게이트가 없다).
async function controlOf(port: number, token: string, cid = "default"): Promise<string | undefined> {
  const r = await rpc(port, token, "status", {});
  const found = (r?.result?.contexts || []).find((x: any) => x.id === cid);
  return found?.control;
}

const STATIC_URL = "data:text/html,<h1>static</h1>";

// 회귀 핀: ①control 초기 상태를 첫 클라이언트에도 전송 ②hover(mousemove)로 조작권 탈취 금지
// ③전 클라이언트 close 시 control 복구 ④정적 페이지에서 두 번째 클라이언트가 캐시 프레임 즉시 수신.
test("cast control 경계·복구 + 신규 클라이언트 캐시 프레임", async () => {
  const home = mkdtempSync(join(tmpdir(), "cast-ctrl-"));
  const proc = Bun.spawn(["bun", "run", "server.ts", "--headless"], {
    cwd: BROWSERD_DIR,
    env: { ...process.env, HOME: home, CYS_BROWSER_HEADLESS: "1" },
    stdout: "pipe",
    stderr: "pipe",
  });
  let failed = false;
  const sockets: WebSocket[] = [];
  try {
    const st = await waitState(home, 30000);
    const port: number = st.port;
    const token: string = st.token;
    const wsUrl = `ws://127.0.0.1:${port}/${token}/cast/ws?context=default`;

    expect((await rpc(port, token, "open", { url: STATIC_URL })).ok).toBe(true);
    // cast 클라이언트가 붙기 전에 이미 human 인 상태를 만든다(에이전트 CLI 가 조작권을 잡은 상황).
    expect((await rpc(port, token, "control", { action: "acquire", actor: "human" })).ok).toBe(true);
    expect(await controlOf(port, token)).toBe("human");

    // ① 첫 클라이언트도 현재 control 을 받아야 한다(툴바 stale 표시 방지).
    const a = new WebSocket(wsUrl);
    sockets.push(a);
    expect(await wsOpen(a, 10000)).toBe(true);
    const ctlMsg = await nextMsg(a, (m) => m.type === "control", 20000, "첫 클라이언트 control 초기 상태");
    expect(ctlMsg.control).toBe("human");

    // ③ 마지막 클라이언트가 나가면 control 은 agent 로 복구된다(고착 방지).
    a.close();
    await sleep(800);
    expect(await controlOf(port, token)).toBe("agent");

    // 재접속해서 입력 경계 검증.
    const a2 = new WebSocket(wsUrl);
    sockets.push(a2);
    expect(await wsOpen(a2, 10000)).toBe(true);
    let lastFidA2 = -1;
    a2.addEventListener("message", (ev) => {
      let m: any;
      try {
        m = JSON.parse(ev.data as string);
      } catch {
        return;
      }
      if (m.type !== "frame") return;
      lastFidA2 = m.fid;
      a2.send(JSON.stringify({ type: "ack", fid: m.fid }));
    });
    const f = await nextMsg(a2, (m) => m.type === "frame", 20000, "정적 페이지 첫 프레임");
    const dw: number = f.metadata.deviceWidth;
    const dh: number = f.metadata.deviceHeight;

    // ② hover(mousemove)만으로는 조작권을 뺏지 않는다 — cast 앱이 30ms 스로틀로 상시 발신하므로 치명적.
    for (let i = 0; i < 3; i++) {
      a2.send(
        JSON.stringify({ type: "mouse", kind: "moved", x: 10 + i, y: 10 + i, cw: dw, ch: dh, button: "none", clickCount: 0, modifiers: 0, deltaX: 0, deltaY: 0 })
      );
    }
    await sleep(600);
    expect(await controlOf(port, token)).toBe("agent"); // 여전히 agent

    // 명시적 클릭(pressed)은 조작 의사 → human 획득.
    a2.send(
      JSON.stringify({ type: "mouse", kind: "pressed", x: Math.floor(dw / 2), y: Math.floor(dh / 2), cw: dw, ch: dh, button: "left", clickCount: 1, modifiers: 0, deltaX: 0, deltaY: 0 })
    );
    await sleep(600);
    expect(await controlOf(port, token)).toBe("human");

    // ④ 정적 페이지 — 두 번째 클라이언트는 새 프레임이 영영 안 오므로 캐시 프레임을 즉시 받아야 한다.
    await sleep(500);
    const fidBefore = lastFidA2;
    const b = new WebSocket(wsUrl);
    sockets.push(b);
    expect(await wsOpen(b, 10000)).toBe(true);
    const bFrame = await nextMsg(b, (m) => m.type === "frame", 3000, "두 번째 클라이언트 캐시 프레임");
    // 캐시본이므로 새로 생성된 프레임이 아니다(fid 가 앞서 본 최신 fid 를 넘지 않는다).
    expect(bFrame.fid).toBeLessThanOrEqual(fidBefore);

    // ③' 클라이언트가 2개여도 전부 나가면 복구된다.
    a2.close();
    b.close();
    await sleep(800);
    expect(await controlOf(port, token)).toBe("agent");
  } catch (e) {
    failed = true;
    throw e;
  } finally {
    for (const s of sockets) {
      try {
        s.close();
      } catch {}
    }
    proc.kill();
    // 정상 경로에서는 stderr 를 아예 읽지 않는다(무한 대기 위험을 감수할 이유가 없다).
    if (failed) console.error("=== server stderr ===\n" + (await readStderr(proc)));
    try {
      rmSync(home, { recursive: true, force: true });
    } catch {}
  }
}, 120000);

// 보안 계약 핀: human 프로필 컨텍스트는 cast 로 노출되지 않는다(로그인 SOT 화면·입력 비노출).
// RPC open 은 args.approved 로 결재를 표현하므로 테스트가 human 컨텍스트를 직접 만들 수 있다
// (CLI 의 cys feed 결재 게이트는 이 경로 바깥 — 서버 계약만 검증한다).
test("cast: human 프로필 컨텍스트는 HUMAN_PROFILE_PROTECTED 로 거부", async () => {
  const home = mkdtempSync(join(tmpdir(), "cast-human-"));
  const proc = Bun.spawn(["bun", "run", "server.ts", "--headless"], {
    cwd: BROWSERD_DIR,
    env: { ...process.env, HOME: home, CYS_BROWSER_HEADLESS: "1" },
    stdout: "pipe",
    stderr: "pipe",
  });
  let failed = false;
  const sockets: WebSocket[] = [];
  try {
    const st = await waitState(home, 30000);
    const port: number = st.port;
    const token: string = st.token;

    // ★PRE-2: human 프로필 요청은 context 인자와 무관하게 cid "human" 으로 강제 분리된다.
    // (요청은 "humanctx" 로 보내지만 서버가 "human" 으로 낙착시킨다 — sot/observe 가 context 를
    //  안 실어 "default" 로 떨어지던 경로를 구조적으로 막는다.)
    const opened = await rpc(port, token, "open", {
      url: STATIC_URL,
      profile: "human",
      approved: true,
      context: "humanctx",
    });
    expect(opened.ok).toBe(true);
    expect(opened.result.profile).toBe("human");
    expect(opened.result.context).toBe("human");

    const ws = new WebSocket(`ws://127.0.0.1:${port}/${token}/cast/ws?context=human`);
    sockets.push(ws);
    expect(await wsOpen(ws, 10000)).toBe(true);
    const err = await nextMsg(ws, (m) => m.type === "err", 15000, "HUMAN_PROFILE_PROTECTED");
    expect(err.code).toBe("HUMAN_PROFILE_PROTECTED");

    // agent 프로필 컨텍스트는 정상 통과해야 한다(거부가 무차별이 아님을 대조 확인).
    const okWs = new WebSocket(`ws://127.0.0.1:${port}/${token}/cast/ws?context=default`);
    sockets.push(okWs);
    expect(await wsOpen(okWs, 10000)).toBe(true);
    const frame = await nextMsg(okWs, (m) => m.type === "frame", 20000, "agent 프로필 프레임");
    expect(Number.isInteger(frame.fid)).toBe(true);
  } catch (e) {
    failed = true;
    throw e;
  } finally {
    for (const s of sockets) {
      try {
        s.close();
      } catch {}
    }
    proc.kill();
    // 정상 경로에서는 stderr 를 아예 읽지 않는다(무한 대기 위험을 감수할 이유가 없다).
    if (failed) console.error("=== server stderr ===\n" + (await readStderr(proc)));
    try {
      rmSync(home, { recursive: true, force: true });
    } catch {}
  }
}, 120000);

// ★CRITICAL 회귀 핀 — 고아 CastHub 레이스.
// WS open() 이 Chromium cold-start 를 await 하는 동안 클라이언트가 끊기면, hub 등록이 await 뒤에
// 있을 때 close() 가 아무것도 못 치우고 죽은 ws 를 담은 고아 hub 가 남는다 → clients.size 가 영원히
// 0 이 안 되어 stop/detach 미실행 + 이후 모든 접속이 프레임 0. 한 번 발생하면 재기동 전까지 불능.
// 함께 검증: 전부 닫힌 뒤 hub 0 · stopScreencast 가 start 횟수만큼 실제 호출됨(자원 누수 계약).
test("cast 고아 hub 레이스: cold-start 중 즉시 close 후에도 신규 접속이 정상 수신", async () => {
  const home = mkdtempSync(join(tmpdir(), "cast-race-"));
  const proc = Bun.spawn(["bun", "run", "server.ts", "--headless"], {
    cwd: BROWSERD_DIR,
    env: { ...process.env, HOME: home, CYS_BROWSER_HEADLESS: "1" },
    stdout: "pipe",
    stderr: "pipe",
  });
  let failed = false;
  const sockets: WebSocket[] = [];
  try {
    const st = await waitState(home, 30000);
    const port: number = st.port;
    const token: string = st.token;
    const wsUrl = `ws://127.0.0.1:${port}/${token}/cast/ws?context=default`;

    // 컨텍스트가 아직 없는 상태에서 접속 → open() 이 Chromium cold-start 를 await 한다.
    // 그 창에서 즉시 끊는다(pane 닫기·GUI 종료·iframe 재로드와 동일 상황).
    const doomed = new WebSocket(wsUrl);
    sockets.push(doomed);
    expect(await wsOpen(doomed, 10000)).toBe(true);
    doomed.close();

    // ★핵심 단언 — cold-start 가 끝나 open() 이 재개된 직후, 다른 클라이언트가 붙기 **전에** 본다.
    // 이 시점에 고아 hub 가 있으면 hubs≥1·clients≥1 로 드러난다(뒤에 새 클라이언트가 붙으면
    // 맵 엔트리가 덮여 고아가 계수에서 사라지고, 캐시 프레임이 '프레임 0' 증상까지 가린다).
    let ctxReady = false;
    for (let i = 0; i < 60; i++) {
      const s = await rpc(port, token, "status", {});
      if (s.ok && (s.result.contexts || []).some((x: any) => x.id === "default")) {
        ctxReady = true;
        break;
      }
      await sleep(500);
    }
    expect(ctxReady).toBe(true); // cold-start 는 완료됐다(= open() 이 재개됨)
    await sleep(700); // 핸들러 정리 여유
    const orphan = await castStat(port, token);
    console.log(`[고아 검사] hubs=${orphan.hubs} clients=${orphan.clients} started=${orphan.started} stopped=${orphan.stopped}`);
    expect(orphan.hubs).toBe(0); // 죽은 ws 를 담은 hub 가 남으면 안 된다
    expect(orphan.clients).toBe(0);
    // ★이 단언이 증명하는 것과 못 하는 것(거짓 커버리지 금지 — 정직 고지)
    //   증명함: started/stopped **자기보고 계측**의 균형. 고아 hub 가 남으면 started>stopped 로 갈린다.
    //   증명 못 함: `Page.stopScreencast` 가 실제로 호출됐다는 사실. master 변이 검증 실측 결과
    //     ①stop 을 다른 CDP 메서드로 치환 ②정리 블록 전면 제거 — 양쪽 다 이 단언을 통과했다.
    //   효과 기반 검증은 **원리적으로 불가**: ack 배압이 이미 프레임 생산을 멈추므로, 전 클라이언트
    //     종료 후 프레임 증가는 정리 유무와 무관하게 0 이다(정리 제거 시에도 증가 0 실측).
    //   잔여 위험은 CDP 세션 핸들·리스너 누수뿐이고 15분 idle 종료가 상한을 건다.
    expect(orphan.started).toBe(orphan.stopped);

    // 이후 신규 접속이 정상적으로 프레임을 받아야 한다(고아 hub 면 영구 0).
    for (let i = 1; i <= 2; i++) {
      const ws = new WebSocket(wsUrl);
      sockets.push(ws);
      expect(await wsOpen(ws, 10000)).toBe(true);
      const f = await nextMsg(ws, (m) => m.type === "frame", 25000, `재접속 ${i} 프레임`);
      expect(Number.isInteger(f.fid)).toBe(true);
      ws.send(JSON.stringify({ type: "ack", fid: f.fid }));
      ws.close();
      await sleep(600);
    }

    // 전부 닫힌 뒤: hub 잔여 0 + stop 이 start 만큼 실제로 호출됐어야 한다(자원 누수 차단 계약).
    await sleep(800);
    const cs = await castStat(port, token);
    console.log(`[고아 hub] hubs=${cs.hubs} clients=${cs.clients} started=${cs.started} stopped=${cs.stopped}`);
    expect(cs.hubs).toBe(0);
    expect(cs.clients).toBe(0);
    expect(cs.started).toBeGreaterThanOrEqual(1);
    expect(cs.stopped).toBe(cs.started); // 위 '고아 검사'의 계측 한계 주석이 이 단언에도 그대로 적용된다
  } catch (e) {
    failed = true;
    throw e;
  } finally {
    for (const s of sockets) {
      try {
        s.close();
      } catch {}
    }
    proc.kill();
    // 정상 경로에서는 stderr 를 아예 읽지 않는다(무한 대기 위험을 감수할 이유가 없다).
    if (failed) console.error("=== server stderr ===\n" + (await readStderr(proc)));
    try {
      rmSync(home, { recursive: true, force: true });
    } catch {}
  }
}, 120000);

// 테스트 전용 정적 서버 — 팝업(window.open)은 동일 출처 http 여야 제약 없이 검증된다
// (data: URL 은 불투명 출처라 opener→popup 조작이 막힌다).
function startFixtureServer() {
  return Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(req) {
      const path = new URL(req.url).pathname;
      const html = (b: string) => new Response(`<!doctype html><meta charset=utf-8>${b}`, { headers: { "content-type": "text/html; charset=utf-8" } });
      if (path === "/orig") return html("<h1 id=t>ORIGINAL-PAGE</h1>");
      if (path === "/popup") return html("<h1 id=p>POPUP-PAGE</h1>");
      if (path === "/console") return html("<h1>CONSOLE</h1><script>console.log('HELLO-LOG');setTimeout(function(){throw new Error('BOOM-ERR')},0);</script>");
      if (path === "/flood") return html("<h1>FLOOD</h1><script>for(var i=0;i<250;i++)console.log('LINE-'+i);</script>");
      return new Response("nope", { status: 404 });
    },
  });
}

// ★HIGH 회귀 핀 — 팝업·새 탭 채택.
// 채택이 없으면 사람은 "클릭했는데 아무 일도 안 일어났다"고 보고(cast 는 옛 페이지만 그림),
// 에이전트도 새 페이지를 조작하지 못한다(eval 이 원본만 봄). target=_blank 는 흔하므로 치명적.
test("cast 팝업 채택: window.open → 새 페이지 프레임·nav 수신 + 에이전트 eval 이 새 페이지를 봄", async () => {
  const home = mkdtempSync(join(tmpdir(), "cast-popup-"));
  const fixture = startFixtureServer();
  const fx = `http://127.0.0.1:${fixture.port}`;
  const proc = Bun.spawn(["bun", "run", "server.ts", "--headless"], {
    cwd: BROWSERD_DIR,
    env: { ...process.env, HOME: home, CYS_BROWSER_HEADLESS: "1" },
    stdout: "pipe",
    stderr: "pipe",
  });
  let failed = false;
  const sockets: WebSocket[] = [];
  try {
    const st = await waitState(home, 30000);
    const port: number = st.port;
    const token: string = st.token;

    expect((await rpc(port, token, "open", { url: `${fx}/orig` })).ok).toBe(true);

    const ws = new WebSocket(`ws://127.0.0.1:${port}/${token}/cast/ws?context=default`);
    sockets.push(ws);
    expect(await wsOpen(ws, 10000)).toBe(true);
    ws.addEventListener("message", (ev) => {
      let m: any;
      try {
        m = JSON.parse(ev.data as string);
      } catch {
        return;
      }
      if (m.type === "frame") ws.send(JSON.stringify({ type: "ack", fid: m.fid }));
    });
    await nextMsg(ws, (m) => m.type === "frame", 20000, "원본 페이지 첫 프레임");

    // ① 팝업 유발 → nav 수신(채택 증거) + 새 프레임
    const navP = nextMsg(ws, (m) => m.type === "nav" && String(m.url).includes("/popup"), 20000, "팝업 nav");
    const framesBefore = (await castStat(port, token)).framesPushed;
    expect((await rpc(port, token, "eval", { expression: `window.open('${fx}/popup','_blank')&&'opened'` })).ok).toBe(true);
    const nav = await navP;
    expect(String(nav.url)).toContain("/popup");
    await sleep(1200);
    const framesAfter = (await castStat(port, token)).framesPushed;
    console.log(`[팝업 채택] nav=${nav.url} 프레임증가=${framesAfter - framesBefore}`);
    expect(framesAfter).toBeGreaterThan(framesBefore); // 채택 후 새 페이지 화면이 실제로 흐른다

    // ② 에이전트 eval 이 새 페이지를 본다
    const seen = await rpc(port, token, "eval", { expression: "document.body.innerText" });
    expect(seen.ok).toBe(true);
    expect(String(seen.result.result)).toContain("POPUP-PAGE");

    // ③ 팝업을 닫으면 원본으로 복귀하고 에이전트도 원본을 다시 본다
    await rpc(port, token, "eval", { expression: "(function(){window.close();return 'closing'})()" }).catch(() => {});
    await sleep(1500);
    const back = await rpc(port, token, "eval", { expression: "document.body.innerText" });
    expect(back.ok).toBe(true);
    console.log(`[팝업 복귀] eval=${String(back.result.result).trim().slice(0, 40)}`);
    expect(String(back.result.result)).toContain("ORIGINAL-PAGE");

    // ④ 페이지 상한 — 팝업을 여러 개 열어도 cid 보유 페이지는 상한(4) 이하로 유지된다
    for (let i = 0; i < 5; i++) {
      await rpc(port, token, "eval", { expression: `window.open('${fx}/popup?n=${i}','_blank')&&'ok'` });
      await sleep(500);
    }
    await sleep(800);
    const status = await rpc(port, token, "status", {});
    const ctx = (status.result.contexts || []).find((x: any) => x.id === "default");
    console.log(`[페이지 상한] pages=${ctx.pages}`);
    expect(ctx.pages).toBeLessThanOrEqual(4);
  } catch (e) {
    failed = true;
    throw e;
  } finally {
    for (const s of sockets) {
      try {
        s.close();
      } catch {}
    }
    proc.kill();
    fixture.stop(true);
    // 정상 경로에서는 stderr 를 아예 읽지 않는다(무한 대기 위험을 감수할 이유가 없다).
    if (failed) console.error("=== server stderr ===\n" + (await readStderr(proc)));
    try {
      rmSync(home, { recursive: true, force: true });
    } catch {}
  }
}, 120000);

// ★MED 회귀 핀 — console·pageerror 캡처(에이전트 진단 근거) + 링버퍼 상한.
test("browserd console 캡처: console.log·pageerror 를 snapshot 에 UNTRUSTED 라벨 아래로 노출", async () => {
  const home = mkdtempSync(join(tmpdir(), "cast-console-"));
  const fixture = startFixtureServer();
  const fx = `http://127.0.0.1:${fixture.port}`;
  const proc = Bun.spawn(["bun", "run", "server.ts", "--headless"], {
    cwd: BROWSERD_DIR,
    env: { ...process.env, HOME: home, CYS_BROWSER_HEADLESS: "1" },
    stdout: "pipe",
    stderr: "pipe",
  });
  let failed = false;
  try {
    const st = await waitState(home, 30000);
    const port: number = st.port;
    const token: string = st.token;

    expect((await rpc(port, token, "open", { url: `${fx}/console` })).ok).toBe(true);
    await sleep(800); // console.log + setTimeout throw 수집 여유

    const snap = await rpc(port, token, "snapshot", {});
    expect(snap.ok).toBe(true);
    const text: string = snap.result.text;
    console.log(`[console 캡처] HELLO-LOG=${text.includes("HELLO-LOG")} BOOM-ERR=${text.includes("BOOM-ERR")}`);
    expect(text).toContain("HELLO-LOG"); // console.log
    expect(text).toContain("BOOM-ERR"); // uncaught exception(pageerror)
    // 비신뢰 경계: 콘솔 절은 UNTRUSTED 헤더 **뒤에** 온다(지시가 아님을 라벨이 덮는다).
    expect(text.indexOf("[UNTRUSTED WEB CONTENT]")).toBeLessThan(text.indexOf("--- CONSOLE"));

    const status1 = await rpc(port, token, "status", {});
    const c1 = (status1.result.contexts || []).find((x: any) => x.id === "default");
    expect(c1.console_lines).toBeGreaterThanOrEqual(2);

    // 링버퍼 상한 — 250줄을 쏟아도 200 을 넘지 않고 오래된 것부터 폐기된다.
    expect((await rpc(port, token, "open", { url: `${fx}/flood` })).ok).toBe(true);
    await sleep(1500);
    const status2 = await rpc(port, token, "status", {});
    const c2 = (status2.result.contexts || []).find((x: any) => x.id === "default");
    console.log(`[링버퍼 상한] console_lines=${c2.console_lines}`);
    expect(c2.console_lines).toBe(200);
    const snap2 = await rpc(port, token, "snapshot", {});
    expect(snap2.result.text).toContain("LINE-249"); // 최신은 남고
    expect(snap2.result.text).not.toContain("HELLO-LOG"); // 가장 오래된 것은 폐기됐다
  } catch (e) {
    failed = true;
    throw e;
  } finally {
    proc.kill();
    fixture.stop(true);
    // 정상 경로에서는 stderr 를 아예 읽지 않는다(무한 대기 위험을 감수할 이유가 없다).
    if (failed) console.error("=== server stderr ===\n" + (await readStderr(proc)));
    try {
      rmSync(home, { recursive: true, force: true });
    } catch {}
  }
}, 120000);
