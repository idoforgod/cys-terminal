// 워크스페이스 복원 **배선** 회귀 핀 — main.ts 소스를 데이터로 읽어 계약을 단언한다
// (typegate.test.ts 관례: 런타임 코드 0줄 · 본체에서 import 되지 않는다 · DOM/Tauri 불요).
//
// ★왜 판정 테스트(wsreconcile.test.ts)만으로 부족한가 (2026-09-16 성찰 1회 지적):
// 이번 실사고에서 실제로 틀린 것은 **판정이 아니라 배선**이었다. 순수 함수는 옳아도
//   · main.ts 가 그 함수를 부르지 않거나
//   · 복원 체인의 await 하나가 시간 상한을 빠뜨리거나
//   · 되살아난 옛 술어 한 줄이 남아 있으면
// 화면은 그대로 사라진다. 실제로 성찰 1회가 찾은 blocker 는 '복원 체인의 마지막 두
// newSurface 만 상한을 빠뜨린 것'이었다 — 사람이 한 줄씩 세는 방식으로는 반드시 다시 빠진다.
// 그래서 배선을 기계가 센다. 여기 실패는 "고쳐 두었다고 믿는 것이 코드에 없다"는 뜻이다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
// 순수 판정 모듈(DOM·Tauri 무관)만 들여온다 — C1 의 동작 핀이 배선 핀과 같은 파일에서 한 번에 red 가 되게.
import {
  restoreLaunchDecision,
  deptLaunchName,
  resolveDeptLaunchName,
  deptCloseAfterStop,
} from "./deptlabel";
import { missingKnownWorkspaces, sameSocket } from "./wsreconcile";

const MAIN_URL = new URL("./main.ts", import.meta.url);
const src = readFileSync(MAIN_URL, "utf-8");
/** 주석을 걷어낸 코드 본문 — '코드에 있는가'를 묻는 핀은 주석에 속으면 안 된다(설명문이 핀을 깨뜨린다). */
const stripComments = (s: string): string =>
  s
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");
const code = stripComments(src);

/** start() 의 세션 복원 블록만 잘라낸다 — 트리 재구성 계약은 그 구간에만 적용된다. */
function restoreSlice(): string {
  const a = src.indexOf("// Session restore (멀티마스터 F4)");
  const b = src.indexOf("started = true;", a);
  expect(a).toBeGreaterThan(0); // 구간 앵커가 사라졌다 = 이 핀이 무엇도 지키지 않는다
  expect(b).toBeGreaterThan(a);
  return src.slice(a, b);
}

/**
 * start() **전체**(머리부터 `started = true` 까지). 시간 상한 계약은 여기에 적용된다.
 * ★왜 복원 블록만으로는 부족한가(2026-09-16 성찰 2회 blocker): 화면을 영구 백지로 만든 두 번째
 * 무상한 왕복은 복원 블록 **직전**의 기본 소켓 `daemon_status` 였다. 좁은 슬라이스는 그 자리를
 * 사정권 밖에 두었다 — 핀의 사정권이 결함의 사정권보다 좁으면 그 핀은 종이 방벽이다.
 */
function startSlice(): string {
  const a = src.indexOf("async function start() {");
  const b = src.indexOf("started = true;", a);
  expect(a).toBeGreaterThan(0);
  expect(b).toBeGreaterThan(a);
  return src.slice(a, b);
}

describe("복원 배선 — 판정 모듈이 실제로 꽂혀 있다", () => {
  it("★옛 차별 술어가 되살아나지 않았다 (`ws.socket != null && lb?.ok === true`)", () => {
    // 이 한 줄이 재부팅 후 본부 5팀을 화면에서 지웠다. 되살아나면 즉시 red.
    expect(code.includes("ws.socket != null && lb?.ok === true")).toBe(false);
  });

  it("복원 필터가 keepWorkspaceOnRestore 를 쓴다(판정 인라인 복제 금지)", () => {
    expect(restoreSlice().includes("keepWorkspaceOnRestore(")).toBe(true);
  });

  it("존재 진실원 보강이 missingKnownWorkspaces 를 쓴다", () => {
    expect(restoreSlice().includes("missingKnownWorkspaces(")).toBe(true);
  });

  it("3초 틱의 유령 수렴이 advanceGhostStrikes 를 쓴다(2연속 게이트 인라인 복제 금지)", () => {
    expect(src.includes("advanceGhostStrikes(")).toBe(true);
  });
});

describe("복원 배선 — 체인의 모든 대기에 시간 상한이 있다", () => {
  // 먹통(accept 후 무응답) 데몬 하나가 start() 를 붙잡으면 **탭이 하나도 렌더되지 않는다**.
  // 상한이 빠진 await 가 한 곳만 있어도 그 결함이 통째로 되살아난다.
  it("★복원 구간의 newSurface 호출은 전부 상한 인자(T_NEW)를 넘긴다", () => {
    // 줄 단위로 본다 — `newSurface(null, current().socket, T_NEW)` 처럼 인자에 괄호가 들어가면
    // 괄호 균형을 안 맞추는 정규식이 첫 `)` 에서 끊겨 T_NEW 를 놓친다(핀이 조용히 무력해진다).
    const lines = restoreSlice().split("\n").filter((l) => l.includes("newSurface("));
    expect(lines.length).toBeGreaterThan(0); // 호출이 사라졌다면 이 핀은 무의미해진다
    for (const l of lines) expect(l).toContain("T_NEW");
  });

  // 기동 구간이 직접 부르는 **데몬 왕복** 목록(이름이 계약이다 — 늘어나면 여기 추가).
  const DAEMON_CMDS = [
    "list_depts",
    "dept_tombstones",
    "daemon_status",
    "launch_dept_daemon",
    "list_surfaces",
    "create_surface",
    "org_status",
    "factory_reset_preview",
    // ★F3(8라운드)→★G1(10라운드): 복원의 묘비 정리는 다시 **fire-and-forget** 이다. 커맨드는 목록에 남긴다 —
    // 이 자리에 다시 `await` 판이 들어오면 상한 없이 통과하지 못하게(그리고 아래 G1 핀이 await 자체를 막는다).
    "stop_dept_daemon_by_socket",
  ];
  // 상한 대신 **다른 방식으로 유계**임이 감사된 예외. 새로 추가하려면 여기에 이유와 함께 적어야 한다
  // (= 사람이 한 번은 의식적으로 판단하게 만드는 것이 이 목록의 목적이다).
  const AUDITED_UNBOUNDED = [
    // 300ms 데몬 대기 프로브: rpcT 로 감싸지 않는 대신 claimFlight 재진입 가드로 동시 1건을
    // 보장하고, 대기가 어떤 경로로 풀리든 stop() 이 인터벌을 회수한다(무계 적체 없음).
    'const call = invoke("daemon_status");',
    // ★G1(2026-09-17 10라운드): 복원 루프의 묘비 정리는 **await 하지 않는다**(HEAD 의미론). 상한이
    // 필요한 이유는 '복원 체인이 그 호출을 기다리기 때문'인데 여기서는 기다리지 않으므로 화면·예산에
    // 어떤 지연도 얹지 않는다(=다른 방식으로 유계다). 기다리는 판이 되살아나면 아래 G1 핀이 red 다.
    'invoke("stop_dept_daemon_by_socket", { socket: ws.socket }).catch((e) => {',
  ];

  it("★기동 구간의 데몬 invoke 는 **전수** 검사한다 — 맨몸 호출이 새로 들어와도 잡힌다", () => {
    // ⚠종전 핀은 `(\w+)\(invoke\("cmd"` 로 **이미 감싸인 것만** 셌다. 래퍼가 아예 없는 줄은
    // 매치 0건이라 배열에 들어가지도 않아, 같은 커맨드의 rpcT 판이 하나라도 있으면 초록이었다
    // — 실제 blocker 가 정확히 그 모양으로 통과했다(돌연변이로 실증됨). 그래서 전수로 뒤집는다.
    const slice = startSlice();
    const bare: string[] = [];
    for (const m of slice.matchAll(/invoke\("(\w+)"/g)) {
      if (!DAEMON_CMDS.includes(m[1])) continue;
      const before = slice.slice(Math.max(0, m.index! - 6), m.index!);
      if (before.endsWith("rpcT(")) continue; // 상한 통과
      const lineStart = slice.lastIndexOf("\n", m.index!) + 1;
      const line = slice.slice(lineStart, slice.indexOf("\n", m.index!)).trim();
      if (AUDITED_UNBOUNDED.some((a) => line.includes(a))) continue; // 감사된 예외
      bare.push(line);
    }
    // 여기서 실패했다면: 그 줄을 `rpcT(invoke(...), T_*)` 로 감싸거나, 다른 방식으로 유계임을
    // 증명하고 AUDITED_UNBOUNDED 에 이유와 함께 등재하라. 조용히 목록에 넣지 마라.
    expect(bare).toEqual([]);
  });

  it("전수 검사가 실제로 무언가를 세고 있다(핀 자체의 계측 타당성)", () => {
    const slice = startSlice();
    const all = [...slice.matchAll(/invoke\("(\w+)"/g)].filter((m) => DAEMON_CMDS.includes(m[1]));
    expect(all.length).toBeGreaterThanOrEqual(6); // 0건이면 위 핀은 공회전이다
  });

  it("상한 값은 명명 상수다(리터럴 흩뿌리기 금지 · Windows 배율이 한 곳에서 걸린다)", () => {
    for (const k of ["T_REG", "T_LIST", "T_STATUS", "T_LAUNCH", "T_NEW", "winScaled"]) {
      expect(src.includes(`const ${k}`)).toBe(true);
    }
  });
});

describe("복원 배선 — 무응답과 사망을 구분한다", () => {
  it("★daemon_status 타임아웃은 재기동 사유가 아니다(중복 launch 폭주 차단)", () => {
    // 살아 있는(느린·먹통인) 데몬에 rival launch 를 걸면 중복 데몬·좌석 탈취로 번진다.
    const slice = restoreSlice();
    expect(slice.includes("statusUnknown")).toBe(true);
    expect(slice.includes("if (statusUnknown) continue;")).toBe(true);
  });
});

describe("복원 배선 — 같은 소켓에 요청을 쌓지 않는다(폭주 축)", () => {
  // JS 상한은 **JS 쪽 포기**일 뿐 Rust/데몬 왕복을 취소하지 못한다. 상한만 두고 3초마다 같은
  // 소켓에 새 요청을 보내면 대기열이 자란다 — 가드의 전부는 '언제 푸느냐'다.
  it("★in-flight 해제는 원 호출이 실제로 끝났을 때다(rpcT 포기 시점 아님)", () => {
    // rpcT 가 포기하는 순간 풀면 다음 틱이 또 얹는다 = 가드가 있으나 마나.
    expect(src.includes("function releaseFlightWhenSettled(")).toBe(true);
    // 두 주기 폴러(3초 틱 list_surfaces · 10초 사이드바 org_status) 모두에 걸려 있어야 한다.
    expect((src.match(/releaseFlightWhenSettled\(/g) ?? []).length).toBeGreaterThanOrEqual(3);
    // 조기 해제(finally 에서 지우기)가 되살아나면 red.
    expect(code.includes("inFlight.delete(flightKey)")).toBe(false);
    expect(code.includes("inFlight.delete(sbKey)")).toBe(false);
  });

  it("★영구 skip 방지 — 끝나지 않는 호출도 재시도 상한이 있다(자가치유 전멸 차단)", () => {
    expect(src.includes("const INFLIGHT_RETRY_MS")).toBe(true);
    expect(src.includes("Date.now() - since < INFLIGHT_RETRY_MS")).toBe(true);
  });
});

// ★K2-05(2026-09-17 한글 사용자명 감사) — 표시명(ws.name)이 `cys-dept launch` 인자로 흐르는 배선이 다시 생기지 않게.
// ★C1(2026-09-17 3라운드 · codex minor): 종전 census 는 invoke **뒤쪽** 240자만 보고 금지 문자열도 옛
//   `deptNameFromSocket(...) ?? ws.name` 만 찾아, 변이 `deptLaunchName(ws.socket, regDepts) ?? ws.name` 이 살아 통과했다
//   (검증자 메모리 실행으로 실증). 이제 (1) invoke 인자 **전체**(괄호 균형 — 창 길이 의존 금지)에서 `ws.name`·`??` 를
//   금지하고 (2) `launchName` 대입의 **우변 전부**를 센다 — 우변은 deptLaunchName(…) 또는 restoreLaunchDecision 의
//   `.launch` 뿐이어야 하며 폴백 연산자가 어디에 붙어도 red 다.
describe("부서 launch 배선 — 표시명은 절대 부서명 인자가 되지 않는다(K2-05 · C1 census)", () => {
  /** `invoke("launch_dept_daemon", …)` 호출 표현을 괄호 균형으로 통째로 잘라낸다. */
  function launchInvokeCalls(): string[] {
    const out: string[] = [];
    for (const m of code.matchAll(/invoke\("launch_dept_daemon"/g)) {
      const start = m.index!;
      let depth = 0;
      let i = start + "invoke".length; // 여는 '(' 위치
      for (; i < code.length; i++) {
        const c = code[i];
        if (c === "(" || c === "{" || c === "[") depth++;
        else if (c === ")" || c === "}" || c === "]") {
          depth--;
          if (depth === 0) break;
        }
      }
      out.push(code.slice(start, i + 1));
    }
    return out;
  }
  it("★launch_dept_daemon invoke 인자 **전체**에 ws.name·`??` 폴백이 없다(240자 창 아님)", () => {
    const calls = launchInvokeCalls();
    expect(calls.length).toBeGreaterThanOrEqual(2); // '지금 켜기' + 복원 재-launch
    for (const c of calls) {
      expect(c).not.toContain("ws.name");
      expect(c).not.toContain("??");
      expect(c).toContain("name: launchName");
    }
  });
  it("★`launchName` 대입의 우변은 deptLaunchName(…)·decision.launch·resolved.name 뿐 — `?? ws.name` 변이는 여기서 red", () => {
    const rhs = [...code.matchAll(/\blaunchName\s*=(?!=)\s*([^;]+);/g)].map((m) => m[1].trim());
    expect(rhs.length).toBeGreaterThanOrEqual(2); // '지금 켜기'(resolved.name) + 복원 루프(decision.launch)
    for (const r of rhs) {
      expect(r).not.toContain("??");
      expect(r).not.toContain("ws.name");
      // ★E3: '지금 켜기' 는 이제 순수부 resolveDeptLaunchName 의 산출을 쓴다(그 안에서 등재 키 우선 규칙이 강제된다).
      expect(/^(deptLaunchName\(|decision\.launch$|resolved\.name$)/.test(r)).toBe(true);
    }
    // main.ts 는 소켓 파서(deptNameFromSocket)를 직접 부르지 않는다 — 해소기는 deptLaunchName 계열 하나뿐이어야
    // 묘비 검사와 launch 가 서로 다른 이름을 볼 수 없다(C1 의 병인).
    expect(code).not.toContain("deptNameFromSocket(");
  });
  it("자동 복원은 restoreLaunchDecision(레지스트리 넘김)의 산출값을 쓴다", () => {
    expect(restoreSlice().includes("restoreLaunchDecision({ socket: ws.socket, regDepts, tombstones: deptTombs })")).toBe(true);
    expect(restoreSlice().includes("unnamedLaunch += 1")).toBe(true); // 무음 생략 금지 — 사유 집계
  });
});

// ★E3(2026-09-17 6라운드 · codex 3차 ④) — 수동 '지금 켜기' 도 **등재 키 우선** 규칙을 지킨다.
//
// 종전 핀은 `deptLaunchName(ws.socket, null)` 이라는 **파서 우선 구조 자체**를 고정하고 있었다(= 결함을
// 박제한 핀). 등재 `dept-2` · 저장 소켓 `\\.\pipe\cys-dept-DEPT-2` 에서 그 구조는 `DEPT-2` 를 내고
// `cys-dept launch DEPT-2` 는 slug 충돌(exit 2)로 실패한다 — 자동 복원만 고쳐졌고 수동 경로는 그대로였다.
// 이제 구조가 아니라 **동작**을 고정한다: 조회를 먼저 1회 하고, 그 결과로 등재 키를 낸다.
describe("'지금 켜기' — 등재 키 우선(조회 먼저 1회 · 실패 때만 파서 폴백)(E3)", () => {
  const REG = { "dept-2": { socket: "\\\\.\\pipe\\cys-dept-dept-2" } };
  const STORED = "\\\\.\\pipe\\cys-dept-DEPT-2"; // 저장된 탭 소켓(대문자 표기 — named pipe 는 대소문자 무구분)

  it("★반례 검체: 등재 dept-2 · 저장 pipe DEPT-2 → 'dept-2'(조회 1회) — 파서 우선이면 'DEPT-2' 로 red", async () => {
    let calls = 0;
    const r = await resolveDeptLaunchName(STORED, async () => {
      calls += 1;
      return REG;
    });
    expect(r).toEqual({ name: "dept-2", regQueried: true });
    expect(calls).toBe(1); // ★조회를 건너뛰면(파서 우선) 여기서 red — 종전 클릭 동작이 정확히 calls===0 이었다
    // 대조: 레지스트리를 넘기지 않으면(종전 구조) slug 충돌을 부르는 이름이 나온다.
    expect(deptLaunchName(STORED, null)).toBe("DEPT-2");
  });

  it("조회 실패에서만 파서 폴백 — 사유 문구를 가르는 regQueried 가 false 다(미등재가 아니다)", async () => {
    const unix = "/Users/x/.local/state/cys-dept-dept-1/cys.sock";
    const boom = await resolveDeptLaunchName(unix, async () => {
      throw new Error("depts.json 판독 실패(cp949)");
    });
    expect(boom).toEqual({ name: "dept-1", regQueried: false }); // 파서 폴백
    const empty = await resolveDeptLaunchName(unix, async () => ({}));
    expect(empty).toEqual({ name: "dept-1", regQueried: true }); // 조회 성공·미등재 — 폴백은 같아도 사유가 다르다
    expect(await resolveDeptLaunchName(undefined, async () => REG)).toEqual({ name: null, regQueried: false });
  });

  it("★배선: 클릭 블록은 resolveDeptLaunchName 에 list_depts 로더를 주입하고, 파서 우선 호출은 남아 있지 않다", () => {
    const a = code.indexOf('btn.textContent = "여는 중…";');
    const b = code.indexOf('invoke("launch_dept_daemon", { name: launchName })', a);
    expect(a).toBeGreaterThan(0);
    expect(b).toBeGreaterThan(a);
    const seg = code.slice(a, b);
    expect(seg).toContain("resolveDeptLaunchName(ws.socket,");
    expect(seg).toContain('invoke("list_depts")'); // 조회가 **먼저** 들어간다(로더)
    expect(seg).not.toContain("deptLaunchName(ws.socket, null)"); // 파서 우선 구조 복귀 = red
  });
});

// ★C1(2026-09-17 3라운드 · codex major) — 묘비 검사와 launch 가 **같은 해소기**를 쓴다. 파서(deptNameFromSocket)만
// 쓰던 묘비 검사는 대문자 PIPE·레거시 파일경로형 소켓에서 null 이 되어 묘비를 비켜 갔고, launch 는 레지스트리 키로
// 그 부서를 기동해 `cys-dept launch` 가 묘비를 지웠다(지운 부서 부활). 순수 판정은 deptlabel.test.ts 가 잠그고,
// 여기서는 그 판정이 **실제로 꽂혀 있는지**와 순서(묘비 → 드롭이 launch 보다 앞)를 센다.
describe("복원 배선 — 묘비 검사는 launch 와 같은 해소기를 쓴다(C1)", () => {
  it("★새 탭 생성(missingKnownWorkspaces)의 이름 해소기는 deptLaunchName(레지스트리 키까지) — 파서 단독 금지", () => {
    const s = restoreSlice();
    const i = s.indexOf("missingKnownWorkspaces(");
    expect(i).toBeGreaterThan(0);
    expect(s.slice(i, i + 400)).toContain("(socket) => deptLaunchName(socket, regDepts)");
  });
  it("★복원 루프의 묘비 판정은 restoreLaunchDecision 이고, 묘비 → 드롭(ghosts.add) → continue 가 launch 보다 앞이다", () => {
    // ★D5-f(2026-09-17 4라운드 · codex 2차): 종전 이 핀은 제목과 달리 ghosts.add·continue 를 세지 않았다(판정 위치만).
    //   묘비 분기의 **내용**(탭 드롭 + 루프 탈출)이 빠지면 묘비 부서가 launch 로 흘러가므로 셋을 순서대로 센다.
    const s = restoreSlice();
    const d = s.indexOf("restoreLaunchDecision({ socket: ws.socket, regDepts, tombstones: deptTombs })");
    const t = s.indexOf('if (decision.reason === "tombstone") {', d);
    const g = s.indexOf("ghosts.add(ws.id);", t);
    const c = s.indexOf("continue;", g);
    const l = s.indexOf('invoke("launch_dept_daemon"', d);
    expect(d).toBeGreaterThan(0);
    expect(t).toBeGreaterThan(d);
    expect(g).toBeGreaterThan(t);
    expect(c).toBeGreaterThan(g);
    expect(l).toBeGreaterThan(c);
    // 드롭·continue 는 묘비 블록 **안**이다 — 블록 닫힘(다음 `let alive`) 보다 앞.
    const blockEnd = s.indexOf("let alive = false;", t);
    expect(blockEnd).toBeGreaterThan(c);
  });
  it("★순수 판정 동작 핀(변이 대조 대상): 대문자 PIPE·레거시 표기에서 묘비가 있으면 launch=null", () => {
    // 해소기를 옛 deptNameFromSocket 으로 되돌리면 두 표기 모두 이름이 null → 묘비 검사를 통과 → 이 핀이 red.
    const reg = {
      "dept-2": { socket: "\\\\.\\pipe\\cys-dept-dept-2" },
      "dept-3": { socket: "C:\\Users\\x\\.local\\state\\cys-dept-dept-3\\cys.sock" },
    };
    const tombs = new Set(["dept-2", "dept-3"]);
    expect(restoreLaunchDecision({ socket: "\\\\.\\PIPE\\cys-dept-dept-2", regDepts: reg, tombstones: tombs })).toEqual({
      launch: null,
      reason: "tombstone",
    });
    expect(restoreLaunchDecision({ socket: reg["dept-3"].socket, regDepts: reg, tombstones: tombs })).toEqual({
      launch: null,
      reason: "tombstone",
    });
    expect(restoreLaunchDecision({ socket: "/Users/x/.local/state/cys-dept-dept-1/cys.sock", regDepts: reg, tombstones: new Set(["dept-1"]) }).launch).toBeNull();
  });
  it("C4-b: 복원 launch 실패 사유는 삼키지 않고 루프 뒤 **1회** 토스트로 낸다(부서마다 토스트 금지 · 재시도 없음)", () => {
    const s = restoreSlice();
    expect(s.includes("failedLaunch += 1")).toBe(true);
    expect(s.includes("firstLaunchErr")).toBe(true);
    const c = s.indexOf("failedLaunch += 1");
    expect(s.slice(c - 700, c).includes("toast(")).toBe(false); // catch 블록 안에서 토스트를 내면 스팸
    expect(s.includes("if (failedLaunch > 0)")).toBe(true);
    expect(code.includes("if (failedLaunch > 0)")).toBe(true);
  });
});

describe("복원 배선 — 묘비는 결측이면 닫는다(fail-closed)", () => {
  it("★묘비 조회 실패(null)에 부서를 만들지 않는다 — 순수부에 그 조항이 있다", () => {
    const ws = readFileSync(new URL("./wsreconcile.ts", import.meta.url), "utf-8");
    expect(ws.includes("if (tombs === null) return out;")).toBe(true);
  });
  it("그룹 삭제도 teardown 전에 삭제 의도를 남긴다(부활 경로 차단)", () => {
    const a = src.indexOf("async function confirmDeleteGroup(");
    const b = src.indexOf("stop_dept_daemon_by_socket", a);
    expect(a).toBeGreaterThan(0);
    expect(src.slice(a, b).includes("dept_tombstone_by_socket")).toBe(true); // 묘비가 teardown 앞이다
  });
});

describe("복원 배선 — 승인 대기 배지는 단일 진실원에서 낸다", () => {
  it("★건너뛴 소켓이 배지 합계를 깎지 않는다(지역 누산기 금지)", () => {
    // in-flight 가드로 건너뛴 소켓을 0으로 세면, 고장 난 데몬이 하나도 없는 정상 동시 호출에서도
    // 승인 대기 ⚠ 배지가 사라진다 — CEO·마스터가 워커의 승인 대기를 화면에서 놓친다.
    expect(code.includes("pendingApprovals = [...pendingBySocket.values()].reduce(")).toBe(true);
    expect(code.includes("pendingApprovals = pend;")).toBe(false);
  });
});

describe("복원 배선 — 저장본을 읽기 전에는 저장하지 않는다", () => {
  it("★saveLayout 은 layoutLoaded 게이트를 먼저 통과한다(배치 영구 파괴 차단)", () => {
    // start() 가 저장본 로드 **전에** 죽고 복구 경로가 render()→saveLayout() 을 부르면,
    // 빈 workspaces 가 사용자의 모든 탭·그룹·분할을 단일 키에 덮어쓴다(백업 없음).
    const i = code.indexOf("function saveLayout()");
    expect(i).toBeGreaterThan(0);
    const head = code.slice(i, i + 200);
    expect(head.includes("if (!layoutLoaded) return;")).toBe(true);
    // 게이트를 여는 곳은 저장본을 읽은 직후 한 곳뿐이어야 한다.
    expect((code.match(/layoutLoaded = true/g) ?? []).length).toBe(1);
  });

  it("★기동 실패 복구는 `started = true` 를 render() 보다 먼저 세운다(자가치유가 그리기에 의존 금지)", () => {
    const i = code.indexOf("startup failed:");
    expect(i).toBeGreaterThan(0);
    const tail = code.slice(i, i + 900);
    expect(tail.indexOf("started = true")).toBeLessThan(tail.indexOf("render()"));
  });
});

describe("복원 배선 — 부팅 1회의 부서 데몬 팬아웃에 상한이 있다", () => {
  it("★등재만 된 부서까지 한꺼번에 띄우지 않는다(A① 폭주)", () => {
    // `cys-dept launch` 한 번은 CEO 승격·티켓·5역할 편성 기동까지 부수효과로 착수한다.
    expect(code.includes("MAX_DEPT_LAUNCH_PER_START")).toBe(true);
    // ★면제 판정은 '저장본에 있었나'가 아니라 '사용자가 실제로 쓰는 탭인가'여야 한다 —
    // 자동 생성된 탭도 곧바로 저장본에 기록되므로, 저장본 기준이면 상한이 1회짜리로 소멸한다.
    expect(code.includes("ws.autoCreated === true && launched >= MAX_DEPT_LAUNCH_PER_START")).toBe(true);
    expect(code.includes("savedSockets")).toBe(false);
  });
  it("★예산 검사가 가장 비싼 루프(부서 데몬 확보)에도 있다", () => {
    // daemon_status(최대 10s·Win 20s) + launch(최대 60s·Win 120s)가 부서마다 직렬이다.
    // 이 루프에 예산이 없으면 죽은 부서 여럿에서 분 단위로 화면이 비고 자가치유도 꺼진 채다.
    const slice = restoreSlice();
    const i = slice.indexOf("const deptWsList");
    expect(i).toBeGreaterThan(0);
    expect(slice.slice(i, i + 700).includes("Date.now() > restoreDeadline")).toBe(true);
  });
  it("복원 체인에 총량 데드라인이 있다(직렬 상한 합만큼 백지로 두지 않는다)", () => {
    expect(code.includes("const START_BUDGET")).toBe(true);
    expect((code.match(/Date\.now\(\) > restoreDeadline/g) ?? []).length).toBeGreaterThanOrEqual(2);
  });
  it("★pane 0개 워크스페이스는 백지가 아니라 안내+손잡이를 그린다(본부·부서 공통)", () => {
    // 팬아웃 상한·예산 소진·launch/셸 생성 실패·데몬 무응답이 전부 tree:null 로 떨어진다.
    expect(code.includes("function renderIdleWorkspace(")).toBe(true);
    expect(code.includes("root.appendChild(renderIdleWorkspace(ws))")).toBe(true);
  });
  it("미룬 사유를 하나로 뭉치지 않는다(제품의 조절 vs 이 기계가 느림)", () => {
    expect(code.includes("cappedLaunch")).toBe(true);
    expect(code.includes("budgetLaunch")).toBe(true);
  });
});


// ★D2(2026-09-17 4라운드 · codex 2차 ⑤) — 묘비 미조회(tombstones=null)는 **죽은 부서의 자동 launch 만** 보류한다.
//   순수 판정은 deptlabel.test.ts 가 잠그고, 여기서는 배선을 센다: 판정이 생존 검사 **뒤**에서만 launch 를 막고(살아 있는
//   pane 보존), 탭을 드롭하는 ghosts 경로에는 들어가지 않으며, 사유는 sticky 토스트 1개(id 하나)로만 나간다.
describe("복원 배선 — 묘비 미조회는 죽은 부서의 자동 launch 만 보류한다(D2 fail-closed)", () => {
  it("★tombstone-unknown 분기는 생존 검사(`if (alive) continue;`) 뒤에 있고, **트리 보존 · 탭 유지 · launch 보류** 다", () => {
    const s = restoreSlice();
    const alive = s.indexOf("if (alive) continue;");
    const held = s.indexOf('decision.reason === "tombstone-unknown"');
    expect(alive).toBeGreaterThan(0);
    expect(held).toBeGreaterThan(alive); // 살아 있는 부서는 이 판정에 닿기 전에 continue — pane 보존
    // 주석에 속지 않게 본문만 본다 — '무엇을 하지 않는다'는 핀은 그 문자열이 설명문에 있어도 red 가 된다.
    const seg = stripComments(s.slice(held, s.indexOf("} else unnamedLaunch += 1;", held)));
    // ★G2(10라운드 · codex 5차 ②): 보류는 **저장 트리를 비우지 않는다.** 8라운드의 `ws.tree = null` 은
    //   "생존 검사 실패 = 사망 확정" 이라는 틀린 전제 위에 있었다 — Windows ERROR_PIPE_BUSY(231) 는 살아 있는
    //   데몬의 혼잡인데 rpc timeout 이 아니라서 사망 분기로 떨어진다. 그때 트리를 비우면 render()→saveLayout()
    //   이 **살아 있는 pane 의 배치를 저장본에서 지운다**(비가역). 이 줄을 되살리는 변이는 여기서 red 다.
    expect(seg).not.toContain("ws.tree = null");
    expect(seg).not.toContain("ghosts.add("); // 보류는 '켜지 않음'이지 '지움'이 아니다
    expect(seg).not.toContain("workspaces.splice("); // 탭도 지우지 않는다
    expect(s.includes("tombUnknownLaunch += 1")).toBe(true);
  });
  it("★사유 토스트는 1회 — sticky id 'tomb-unknown' 하나를 갱신하고(새 탭 미생성 고지와 공유) 루프 안에서는 내지 않는다", () => {
    const s = restoreSlice();
    expect((s.match(/stickyToast\(\s*"tomb-unknown"/g) ?? []).length).toBe(2); // 새 탭 미생성 + 자동 launch 보류
    const loopStart = s.indexOf("for (let di = 0; di < deptWsList.length; di++) {");
    const loopEnd = s.indexOf("if (ghosts.size) workspaces = workspaces.filter(", loopStart);
    expect(loopStart).toBeGreaterThan(0);
    expect(loopEnd).toBeGreaterThan(loopStart);
    expect(s.slice(loopStart, loopEnd)).not.toContain('"tomb-unknown"');
    expect(s.indexOf("if (tombUnknownLaunch > 0)")).toBeGreaterThan(loopEnd);
  });
  it("수동 [지금 켜기] 는 묘비를 보지 않는다(보류의 탈출구) — 이름 해소만 · restoreLaunchDecision 미사용", () => {
    // ★E3 이후 해소기는 resolveDeptLaunchName(등재 키 우선)이다. 묘비를 보지 않는다는 성질은 그대로여야
    //   보류(tombstone-unknown)의 탈출구가 남는다. 주석에 속지 않도록 comment-stripped 본문으로 센다.
    const a = code.indexOf('btn.textContent = "여는 중…";');
    const b = code.indexOf('invoke("launch_dept_daemon", { name: launchName })', a);
    expect(a).toBeGreaterThan(0);
    expect(b).toBeGreaterThan(a);
    const seg = code.slice(a, b);
    expect(seg).toContain("resolveDeptLaunchName(ws.socket,");
    expect(seg).not.toContain("restoreLaunchDecision(");
    expect(seg).not.toContain("dept_tombstones");
  });
});

// ★D1(2026-09-17 4라운드 · codex 2차 ③) — Rust stop_dept_daemon(_by_socket) 이 비0 rc 를 Err 로 주게 됐다. UI 쪽 계약:
//   거짓 문구("부활은 차단됨(삭제 의도 기록됨)")가 되살아나지 않고, 실패를 의도적으로 무시하는 `.catch(() => {})` 는
//   이유 주석을 단 두 곳(생성 롤백)뿐이다.
describe("부서 teardown 배선 — Err 를 삼키는 곳은 이유가 적힌 두 곳뿐이다(D1)", () => {
  it("★rc 10 에서 거짓이던 문구가 없다", () => {
    expect(code).not.toContain("부활은 차단됨(삭제 의도 기록됨)");
  });
  it("★탭 닫기·그룹 삭제·복원 묘비 정리는 실패를 토스트로 낸다(문구는 순수부가 낸다)", () => {
    // 탭 닫기: 묘비 기록 성공 여부로 뒷문장을 가르는 판정을 순수부에 넘겼다(deptCloseAfterStop).
    const close = code.indexOf("let tombRecorded = false;");
    expect(close).toBeGreaterThan(0);
    const closeSeg = code.slice(close, code.indexOf("return tab;", close));
    expect(closeSeg).toContain("deptCloseAfterStop({ stop: stopped, tombRecorded, error: stopErr })");
    expect(closeSeg).toContain('toast("watchdog", "부서 데몬 종료 실패", verdict.detail ?? "")');
    // 그룹 삭제: 모아서 1회.
    const grp = code.indexOf("async function confirmDeleteGroup(");
    const grpSeg = code.slice(grp, code.indexOf("const UNTITLED", grp));
    expect(grpSeg).toContain("stopFailed.push(");
    expect(grpSeg).toContain("if (stopFailed.length) {");
    // 복원 묘비 정리(★G1 이후 fire-and-forget): 실패는 `.catch` 안에서 sticky id 하나로 고지한다 —
    // 기동당 화면에 1개(부서마다 새 토스트가 생기면 폭주 · pushAlarm 도 같은 id 를 대체한다).
    expect(restoreSlice()).toContain("tombReapErrCount += 1;");
    expect(restoreSlice()).toContain('"tomb-reap-err"');
  });
  it("★`.catch(() => {})` 로 무시하는 stop_dept_daemon_by_socket 호출은 정확히 2곳이고 각각 ★D1 이유 주석이 앞에 있다", () => {
    const re = /invoke\("stop_dept_daemon_by_socket"[^\n]*\.catch\(\(\) => \{\}\)/g;
    const hits = [...src.matchAll(re)];
    expect(hits.length).toBe(2);
    for (const m of hits) {
      const before = src.slice(Math.max(0, m.index! - 600), m.index!);
      expect(before).toContain("★D1: "); // 이유 없이 삼키는 새 호출이 생기면 여기서 red
      expect(before).toContain("missingKnownWorkspaces"); // 무음 소실이 아닌 근거(다음 시작의 레지스트리 대조)
    }
  });
});

// ★D4(2026-09-17 4라운드 · codex 2차 ⑦) — `{depts:{"dept-1":{display_name:"영업부"}}}` 처럼 socket 필드가 없는 정상 등재.
//   UI 는 등재의 socket 으로 탭을 만들고(missingKnownWorkspaces) 지키며(registered · sameSocket) 켠다(restoreLaunchDecision) —
//   socket 이 비면 세 곳 모두 그 부서를 모른 채 '미등재 유령'으로 탭을 드롭했다. 수리는 Rust list_depts 가 canonical
//   `dept_socket_path(name)` 을 채우는 것(Rust 복원기와 해석 통일)이고, 여기서는 (1) 그 계약이 main.rs 에 배선돼 있는지와
//   (2) 채워진 등재를 UI 순수부가 실제로 탭 보존·생성·복원으로 잇는지를 센다.
describe("복원 배선 — socket 없는 등재는 Rust 가 canonical socket 을 채우고 UI 는 그것으로 탭을 지킨다(D4)", () => {
  const rs = readFileSync(new URL("../../src-tauri/src/main.rs", import.meta.url), "utf-8");
  const rsBody = rs.slice(0, rs.indexOf("#[cfg(test)]\nmod tests {"));
  it("★main.rs list_depts 는 read_json_or_empty 뒤에 fill_canonical_dept_sockets 를 지나고, 그 함수는 dept_socket_path(name) 을 쓴다", () => {
    const l = rsBody.indexOf("fn list_depts()");
    expect(l).toBeGreaterThan(0);
    const seg = rsBody.slice(l, l + 900);
    expect(seg).toContain("read_json_or_empty(");
    expect(seg).toContain("fill_canonical_dept_sockets(");
    const f = rsBody.indexOf("fn fill_canonical_dept_sockets(");
    expect(f).toBeGreaterThan(0);
    expect(rsBody.slice(f, f + 1200)).toContain("dept_socket_path(name)");
  });
  it("UI 는 등재의 socket 에만 의존한다(표시명 유무 무관) — 그래서 Rust 의 canonical 채움이 탭 보존의 전제다", () => {
    const s = restoreSlice();
    expect(s).toContain(".map((v) => v?.socket)"); // registered
    expect(s).toContain("if (e?.socket) displayBySocket.set(e.socket, e.display_name ?? dname);"); // 새 탭 원천
  });
  it("★검체: display_name 만 있던 등재에 canonical socket 이 채워지면 탭 생성 · 보존 · 복원 launch 가 전부 dept-1 로 잇는다", () => {
    // Rust 가 채운 뒤의 모습(unix canonical) — 채워지기 전(socket 없음)과 대조한다.
    const canonical = "/Users/x/.local/state/cys-dept-dept-1/cys.sock";
    const filled = { "dept-1": { display_name: "영업부", socket: canonical } };
    const unfilled: Record<string, { display_name: string; socket?: string }> = { "dept-1": { display_name: "영업부" } };
    const known = (reg: Record<string, { display_name?: string; socket?: string }>) =>
      Object.entries(reg)
        .filter(([, e]) => !!e.socket)
        .map(([n, e]) => ({ socket: e.socket!, label: e.display_name ?? n }));
    // (1) 새 탭 생성 — 저장본에 없어도 등재 부서는 탭이 생긴다(채워지기 전엔 known 이 비어 아무 탭도 못 만든다).
    expect(missingKnownWorkspaces([{ socket: undefined, tree: null }], known(unfilled), new Set(), (s) => deptLaunchName(s, unfilled))).toEqual([]);
    expect(missingKnownWorkspaces([{ socket: undefined, tree: null }], known(filled), new Set(), (s) => deptLaunchName(s, filled))).toEqual([
      { socket: canonical, name: "영업부" },
    ]);
    // (2) 탭 보존 — 저장 탭의 소켓이 registered(등재 socket 집합)에 있어야 '미등재 유령'으로 드롭되지 않는다.
    const registered = (reg: Record<string, { socket?: string }>) =>
      new Set(Object.values(reg).map((v) => v?.socket).filter((x): x is string => !!x));
    expect([...registered(unfilled)].some((r) => sameSocket(r, canonical))).toBe(false); // 채워지기 전: 드롭 경로
    expect([...registered(filled)].some((r) => sameSocket(r, canonical))).toBe(true); // 채워진 뒤: 보존
    // (3) 복원 launch — 표시명 '영업부' 가 아니라 등재 키 dept-1 로 켠다.
    expect(restoreLaunchDecision({ socket: canonical, regDepts: filled, tombstones: new Set() })).toEqual({ launch: "dept-1", reason: "launch" });
    // Windows named pipe canonical 도 같은 흐름(대소문자 표기 차이 흡수).
    const pipe = "\\\\.\\pipe\\cys-dept-dept-1";
    const filledWin = { "dept-1": { display_name: "영업부", socket: pipe } };
    expect(missingKnownWorkspaces([{ socket: undefined, tree: null }], known(filledWin), new Set(), (s) => deptLaunchName(s, filledWin))).toEqual([
      { socket: pipe, name: "영업부" },
    ]);
    expect(restoreLaunchDecision({ socket: "\\\\.\\PIPE\\cys-dept-dept-1", regDepts: filledWin, tombstones: new Set() }).launch).toBe("dept-1");
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★F3(2026-09-17 8라운드 · 단순화) — 삭제에서 **탭은 종료가 확인된 뒤에** 지운다.
//
// 반례(codex 3차 ②): 삭제 → 묘비 기록 → 탭 splice·저장 → `cys-dept down` rc 10(kill 0 · 등재 제거 0) →
// 데몬은 살아 있는데 탭이 없고, 묘비가 탭 재생성을 막으므로 그 데몬을 다시 볼 주체가 없다.
// 6라운드의 답은 '레지스트리 ∩ 묘비' reaper 였고, 그 reaper 는 탭 없는(CLI 로 만든) 부서까지 매 기동
// 종료 대상으로 삼았다(codex 4차 major 1·2). 8라운드는 상태를 더하지 않고 **순서**를 고친다 —
// 종료 실패면 탭이 남고, 탭이 남으면 다음 기동의 **탭 기반** 복원 정리가 그대로 재시도 주체다.
// ────────────────────────────────────────────────────────────────────────────
describe("부서 삭제 — 탭은 종료가 확인된 뒤에 지운다(F3)", () => {
  it("★검체: 종료 실패면 탭을 남긴다(= 재시도 손잡이) · 성공·불요면 지운다", () => {
    expect(deptCloseAfterStop({ stop: "failed", tombRecorded: true, error: "rc=10" }).removeTab).toBe(false);
    expect(deptCloseAfterStop({ stop: "ok", tombRecorded: true }).removeTab).toBe(true);
    expect(deptCloseAfterStop({ stop: "not-needed", tombRecorded: false }).removeTab).toBe(true);
  });

  it("★탭 닫기 배선: stop 을 await 한 **뒤에** splice 한다(낙관적 제거를 되살리면 red)", () => {
    const a = code.indexOf("let tombRecorded = false;");
    expect(a).toBeGreaterThan(0);
    const seg = code.slice(a, code.indexOf("return tab;", a));
    const stop = seg.indexOf('rpcT(invoke("stop_dept_daemon_by_socket", { socket: ws.socket }), T_STOP)');
    const verdict = seg.indexOf("deptCloseAfterStop({ stop: stopped, tombRecorded, error: stopErr })");
    const splice = seg.indexOf("workspaces.splice(i, 1);");
    expect(stop).toBeGreaterThan(0);
    expect(verdict).toBeGreaterThan(stop); // 판정은 종료 결과를 보고 내린다
    expect(splice).toBeGreaterThan(verdict); // ★순서가 곧 계약이다
    expect((seg.match(/workspaces\.splice\(/g) ?? []).length).toBe(1); // 낙관적 splice 가 되살아나면 red
    expect(seg).toContain("if (!verdict.removeTab) {"); // 실패 → 탭 유지
    expect(seg).toContain("ws.tree = null;"); // 닫아 버린 pane 의 죽은 트리는 남기지 않는다
    expect(seg).toContain("(w) => w !== ws && w.socket === ws.socket"); // 자기 자신은 아직 목록에 있다
  });

  it("★그룹 삭제도 같은 계약 — 실패한 부서만 탭을 남기고 사유는 모아서 1회", () => {
    const g = code.indexOf("async function confirmDeleteGroup(");
    expect(g).toBeGreaterThan(0);
    const seg = code.slice(g, code.indexOf("const UNTITLED", g));
    const stop = seg.indexOf('rpcT(invoke("stop_dept_daemon_by_socket", { socket: ws.socket }), T_STOP)');
    const splice = seg.indexOf("workspaces.splice(i, 1);");
    expect(stop).toBeGreaterThan(0);
    expect(splice).toBeGreaterThan(stop);
    expect(seg).toContain("deptCloseAfterStop({");
    expect(seg).toContain("tombRecordedIds.has(ws.id)"); // 부서별 묘비 기록 여부로 문구를 가른다
    expect(seg).toContain("stopFailed.push(");
  });

  it("★복원의 묘비 정리는 삭제 흐름이 아니다 — 재시도 손잡이는 탭 닫기·그룹 삭제뿐이다(G1 이후)", () => {
    // 8라운드는 복원 루프도 '종료 확인 후 탭 드롭'으로 만들었다가 경쟁 창·기아를 낳았다(codex 5차 ①③).
    // 10라운드는 그 계약을 **삭제 경로에만** 둔다 — 복원은 묘비(=확정된 삭제 의도)를 보고 탭을 즉시 지운다.
    const close = code.indexOf("let tombRecorded = false;");
    const grp = code.indexOf("async function confirmDeleteGroup(");
    expect(close).toBeGreaterThan(0);
    expect(grp).toBeGreaterThan(0);
    for (const seg of [code.slice(close, code.indexOf("return tab;", close)), code.slice(grp, code.indexOf("const UNTITLED", grp))]) {
      expect(seg).toContain('rpcT(invoke("stop_dept_daemon_by_socket", { socket: ws.socket }), T_STOP)');
      expect(seg).toContain("deptCloseAfterStop({");
    }
  });

  it("★8라운드에서 걷어낸 상태기계의 잔재가 0이다(라운드마다 새 결함을 만들던 코드)", () => {
    for (const gone of [
      "reapTombstonedDepts",
      "tombstonedRegisteredDepts",
      "tombReapRan",
      "tombRetryGate",
      "runTombRetry",
      "scheduleTombRetry",
      "restoreHeld",
      "restoreSurfaceView",
    ]) {
      expect(src.includes(gone)).toBe(false);
    }
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★F2(2026-09-17 8라운드 · 단순화) — 묘비 RPC 실패(tombstone-unknown) 보류 탭의 최소 안전 동작.
// 탭 유지 + 트리 비움 + 사유 토스트 1회. 자동 재조회·재시도 없음(수동 [지금 켜기] 또는 앱 재기동).
// ────────────────────────────────────────────────────────────────────────────
describe("복원 보류 — 탭은 유지하고 트리만 비운다(F2)", () => {
  it("★자동 재조회 타이머가 없다 — 이 세션에서 다시 묘비를 묻지 않는다(폭주·상한 우회 제거)", () => {
    for (const gone of ["tombRetryGate", "TOMB_RETRY", "runTombRetry", "scheduleTombRetry"]) {
      expect(src.includes(gone)).toBe(false);
    }
    // 묘비 조회는 복원 블록의 1회뿐이다(타이머·인터벌에 얹히지 않는다).
    expect((code.match(/invoke\("dept_tombstones"\)/g) ?? []).length).toBe(1);
    for (const m of code.matchAll(/setTimeout\(([^\n]*)/g)) expect(m[1]).not.toContain("dept_tombstones");
    for (const m of code.matchAll(/setInterval\(([^\n]*)/g)) expect(m[1]).not.toContain("dept_tombstones");
  });

  it("★빈 트리 탭은 백지가 아니라 idle 패널('지금 켜기')이 받는다", () => {
    expect(code).toContain("root.appendChild(renderIdleWorkspace(ws))");
    expect(code).toContain('btn.textContent = isDept ? "지금 켜기" : "새 셸 열기";');
  });

  it("★기본 데몬 폴백은 본부 탭에만 — 부서 탭에 붙인 셸은 pane 키가 어긋나 보이지 않는다", () => {
    const i = code.indexOf("if (!current().tree) {");
    expect(i).toBeGreaterThan(0);
    const seg = code.slice(i, code.indexOf("\n  render();", i));
    expect(seg).toContain("newSurface(null, current().socket, T_NEW)");
    expect(seg).toContain("if (!current().socket) {");
  });

  it("★수동 [지금 켜기] 의 등재 조회에도 상한이 있다(무기한 disabled 금지)", () => {
    const a = code.indexOf('btn.textContent = "여는 중…";');
    const b = code.indexOf('invoke("launch_dept_daemon", { name: launchName })', a);
    expect(a).toBeGreaterThan(0);
    expect(b).toBeGreaterThan(a);
    expect(code.slice(a, b)).toContain('rpcT(invoke("list_depts"), T_REG)');
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★G1(2026-09-17 10라운드 · codex 5차 ①③ · opus 9R medium) — 복원 루프의 묘비 정리는 **HEAD 의미론**이다:
// fire-and-forget · 탭 즉시 드롭 · START_BUDGET 무소모 · 실패는 기동당 1개의 토스트.
//
// 8라운드가 이 자리에 넣은 `await rpcT(…, T_STOP)` 한 줄이 만든 것:
//   ① 경쟁 창 — 대기하는 T_STOP(win 40s) 동안 사용자가 [지금 켜기]로 되살린 부서를 뒤늦은 stop 이 죽였다.
//   ② 기아 — 그 대기가 복원 예산에서 빠져 묘비 탭 3개면 살아 있는 부서가 한 곳도 켜지지 않았다.
// 그래서 이 핀은 **await 가 없다**를 직접 센다(변이 = await 복원 → red).
// ────────────────────────────────────────────────────────────────────────────
describe("복원 묘비 정리 — fire-and-forget 으로 되돌렸다(G1)", () => {
  /** 복원 루프의 묘비 분기 **본문**(판정 ~ 생존 검사 직전 · 주석 제거 — 설명문이 핀을 깨뜨리지 않게). */
  function tombSeg(): string {
    const s = restoreSlice();
    const i = s.indexOf('if (decision.reason === "tombstone") {');
    expect(i).toBeGreaterThan(0);
    const j = s.indexOf("let alive = false;", i);
    expect(j).toBeGreaterThan(i);
    return stripComments(s.slice(i, j));
  }

  it("★정리는 await 하지 않는다 — 이 분기에 await·rpcT·T_STOP 이 없다(8R 직렬화 복원 시 red)", () => {
    const seg = tombSeg();
    expect(seg).toContain('invoke("stop_dept_daemon_by_socket", { socket: ws.socket }).catch(');
    expect(seg).not.toContain("await "); // 어떤 형태의 대기도 허용하지 않는다(경쟁 창·예산 소모의 근원)
    expect(seg).not.toContain("rpcT(");
    expect(seg).not.toContain("T_STOP");
  });

  it("★복원 루프 **전체**에 T_STOP 이 없다 — 예산(START_BUDGET)은 살아 있는 부서 확보에만 쓴다", () => {
    const s = restoreSlice();
    const loop = stripComments(
      s.slice(
        s.indexOf("for (let di = 0; di < deptWsList.length; di++) {"),
        s.indexOf("if (ghosts.size) workspaces = workspaces.filter("),
      ),
    );
    expect(loop.length).toBeGreaterThan(200);
    expect(loop).not.toContain("T_STOP");
  });

  it("★묘비 탭은 **즉시** 드롭한다(삭제 의도 확정) — 종료 확인을 기다리는 판이 되살아나면 red", () => {
    const seg = tombSeg();
    const drop = seg.indexOf("ghosts.add(ws.id);");
    const stop = seg.indexOf('invoke("stop_dept_daemon_by_socket"');
    expect(drop).toBeGreaterThan(0);
    expect(drop).toBeLessThan(stop); // HEAD 와 같은 순서: 드롭 먼저, 정리는 뒤에 걸어만 둔다
    expect(seg).not.toContain("deptCloseAfterStop("); // 삭제 경로의 판정을 복원이 흉내 내지 않는다
  });

  it("★실패 고지는 기동당 1개 — sticky id 하나를 갱신하고 회수 절차는 **해소된 부서명**으로 말한다(G1+I1)", () => {
    const seg = tombSeg();
    expect(seg).toContain('stickyToast("tomb-reap-err", "watchdog", reap.title, reap.body);');
    expect(seg).toContain("tombReapErrCount += 1;");
    // ★I1(2026-09-17 13라운드 · opus 11R major): 회수 손잡이의 이름은 표시명이 아니라 **부서명**이다 —
    //   `ws.name`(= `display_name ?? dname`)은 한글일 수 있고 `cys-dept down <표시명>` 은
    //   validate_dept_name(^[A-Za-z0-9][A-Za-z0-9_-]*$)에서 exit 2 로 거부된다(= 안내가 실행 불가능).
    //   이 분기는 판정을 만든 그 입력으로 이름을 해소해 넘기기만 한다(문구는 순수부 tombReapErrorToast).
    expect(seg).toContain("const reapName = deptLaunchName(ws.socket, regDepts);");
    expect(seg).toContain("tombReapErrorToast({ count: tombReapErrCount, deptName: reapName, error: String(e) })");
    expect(seg).not.toContain("ws.name"); // 표시명 복원 변이 → red
    expect(seg).not.toContain("UNTITLED");
    expect(seg).not.toContain("toast("); // 휘발 토스트(부서마다 1개)가 되살아나면 red
    // 같은 id 는 이 한 곳뿐이다(다른 경로가 같은 자리를 덮어쓰지 않는다).
    expect((code.match(/"tomb-reap-err"/g) ?? []).length).toBe(1);
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★G3-b(2026-09-17 10라운드 · codex 5차 ④) — 묘비 **기록자와 판독기가 같은 식별자**를 쓴다.
// 프론트가 해소한 이름(등재 키 우선)을 dept_tombstone.set 에 넘기고, Rust 는 미지정일 때만 소켓에서 파생한다.
// ────────────────────────────────────────────────────────────────────────────
describe("묘비 기록 — 소켓 파서가 아니라 해소된 이름을 넘긴다(G3-b)", () => {
  it("★탭 닫기: dept_tombstone_by_socket 호출이 resolveDeptLaunchName 의 산출을 name 으로 넘긴다", () => {
    const a = code.indexOf("let tombRecorded = false;");
    expect(a).toBeGreaterThan(0);
    const seg = code.slice(a, code.indexOf("for (const sid of collectSids(ws.tree))", a));
    expect(seg).toContain("resolveDeptLaunchName(ws.socket,");
    expect(seg).toContain('invoke("dept_tombstone_by_socket", { socket: ws.socket, name: tombName.name })');
    // 해소 없이 소켓만 넘기는 옛 호출이 되살아나면 red.
    expect(seg).not.toContain('invoke("dept_tombstone_by_socket", { socket: ws.socket })');
  });

  it("★그룹 삭제: 등재 조회는 그룹당 1회이고 부서마다 deptLaunchName 으로 이름을 해소한다", () => {
    const g = code.indexOf("async function confirmDeleteGroup(");
    const seg = code.slice(g, code.indexOf("const stopFailed: string[] = []", g));
    expect(seg).toContain('rpcT(invoke("list_depts"), T_REG)');
    expect((seg.match(/invoke\("list_depts"\)/g) ?? []).length).toBe(1); // 부서마다 왕복하면 삭제가 N배 느려진다
    expect(seg).toContain('invoke("dept_tombstone_by_socket", { socket: ws.socket, name: deptLaunchName(ws.socket, grpReg) })');
    expect(seg).not.toContain('invoke("dept_tombstone_by_socket", { socket: ws.socket })');
  });

  it("★Rust 기록자는 인자 이름을 우선하고, 없을 때만 소켓 파서로 폴백한다(계약 후퇴 없음)", () => {
    const rs = readFileSync(new URL("../../src-tauri/src/main.rs", import.meta.url), "utf-8");
    const i = rs.indexOf("async fn dept_tombstone_by_socket(");
    expect(i).toBeGreaterThan(0);
    const seg = rs.slice(i, rs.indexOf("\n}", i));
    expect(seg).toContain("socket: String, name: Option<String>");
    expect(seg).toContain(".or_else(|| dept_name_from_socket(&socket))");
    expect(seg).toContain('"dept_tombstone.set"');
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★G4(2026-09-17 10라운드 · opus 9R minor) — 삭제 중(stop 대기) 탭은 입양 대상이 아니다.
// F3 이 splice 를 stop 뒤로 미루면서 생긴 창(탭 닫기 T_STOP · 그룹 삭제 N×T_STOP)에서 3초 틱이
// 잔여 role surface 를 붙이면, 곧 splice 되는 탭의 pane 런타임이 회수 대상에서 빠져 누수된다.
// ────────────────────────────────────────────────────────────────────────────
describe("삭제 중 탭 — 입양 금지 + 진행 표시(G4)", () => {
  it("★3초 입양 틱의 가드에 `!w.deleting` 이 있다(가드를 빼면 red)", () => {
    const i = code.indexOf("for (const s of [...r.surfaces].sort(");
    expect(i).toBeGreaterThan(0);
    const seg = code.slice(i, code.indexOf("adopted = true;", i));
    expect(seg).toContain("!w.pending && !w.deleting");
  });

  // ★I2(2026-09-17 13라운드 · opus 11R minor m1) — 가드는 **확인 직후**에 선다. 10라운드 판은 stop
  //   직전에만 세웠는데, 회수 대상 sid 스냅샷(collectSids)은 그보다 앞에서 떠지고 그 뒤 close_surface
  //   루프가 sid 마다 await 로 이벤트 루프를 놓는다 — 그 창에서 입양된 surface 는 스냅샷에 없어
  //   destroyPaneRuntime 을 못 거치고 탭이 splice 될 때 런타임만 남는다(입양 1건당 누수 1건).
  //   묘비 기록 왕복(list_depts · T_REG · mac 8s/win 16s)도 같은 창이었다.
  it("★탭 닫기: 확인 직후에 `deleting` 을 세워 스냅샷·close_surface·묘비 왕복·stop 을 모두 덮고 finally 에서만 해제한다(I2)", () => {
    const c = code.indexOf("const wsName = ws.name || UNTITLED;");
    expect(c).toBeGreaterThan(0);
    const seg = code.slice(c, code.indexOf("return tab;", c));
    const ok = seg.indexOf("if (!ok) return;");
    const set = seg.indexOf("ws.deleting = true;");
    const sids = seg.indexOf("for (const sid of collectSids(ws.tree))");
    const tomb = seg.indexOf('invoke("dept_tombstone_by_socket"');
    const stop = seg.indexOf('invoke("stop_dept_daemon_by_socket"');
    const fin = seg.indexOf("} finally {");
    const clear = seg.indexOf("ws.deleting = undefined;");
    expect(ok).toBeGreaterThan(0);
    expect(set).toBeGreaterThan(ok); // 거부(`!ok`)는 플래그를 세우지 않는다
    expect(tomb).toBeGreaterThan(set); // 묘비 기록 왕복(T_REG)이 가드 안이다
    expect(sids).toBeGreaterThan(set); // ★sid 스냅샷이 가드 안이다(이 줄이 m1 의 핵심)
    expect(stop).toBeGreaterThan(set); // 표식이 종료보다 먼저 선다(창이 열리기 전에)
    expect(fin).toBeGreaterThan(stop);
    expect(clear).toBeGreaterThan(fin); // 해제는 finally 한 곳 — 성공·실패·timeout·예외를 모두 덮는다
    expect((seg.match(/ws\.deleting = true;/g) ?? []).length).toBe(1);
    expect((seg.match(/ws\.deleting = undefined;/g) ?? []).length).toBe(1);
  });

  it("★그룹 삭제도 동형 — 확인 직후 멤버 전원에 세우고 finally 에서 전원 해제한다(I2)", () => {
    const g = code.indexOf("async function confirmDeleteGroup(");
    expect(g).toBeGreaterThan(0);
    const seg = code.slice(g, code.indexOf("const UNTITLED", g));
    const mem = seg.indexOf("const members = workspaces.filter((w) => w.groupId === g.id);");
    const set = seg.indexOf("for (const ws of members) ws.deleting = true;");
    const sids = seg.indexOf("for (const sid of collectSids(ws.tree))");
    const tomb = seg.indexOf('invoke("dept_tombstone_by_socket"');
    const stop = seg.indexOf('invoke("stop_dept_daemon_by_socket"');
    const fin = seg.indexOf("} finally {");
    const clear = seg.indexOf("for (const ws of members) ws.deleting = undefined;");
    expect(mem).toBeGreaterThan(0);
    expect(set).toBeGreaterThan(mem); // 2-클릭 확인(groupDeleteArm) 통과 직후다
    expect(tomb).toBeGreaterThan(set);
    expect(sids).toBeGreaterThan(set);
    expect(stop).toBeGreaterThan(set);
    expect(fin).toBeGreaterThan(stop);
    expect(clear).toBeGreaterThan(fin);
    // 그룹 삭제는 탭을 다시 그려야 표식이 보인다(탭 닫기는 자기 엘리먼트를 직접 바꾼다).
    expect(seg).toContain("renderWsTabs();");
    // 탭 렌더러가 그 축을 실제로 그린다.
    expect(code).toContain("if (ws.deleting) {");
  });

  it("★런타임 전용 — 저장본에는 `deleting`·`stopFailed` 가 실리지 않는다(다음 기동 오염 차단)", () => {
    expect(code).toContain("norm.map(({ deleting: _d, stopFailed: _s, ...w }) => w)");
    expect(code).toContain("JSON.stringify({ workspaces: persisted,");
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★G5(2026-09-17 10라운드 · codex 5차 ⑤) — 삭제 실패 탭의 안내가 실제 동작과 같다.
// "앱을 다시 켜면 준비됩니다"는 이 상태에서 거짓이다(재시작은 '준비'가 아니라 종료 재시도).
// ────────────────────────────────────────────────────────────────────────────
describe("삭제 실패 탭 — 안내가 실제 동작과 같다(G5)", () => {
  it("★문구: '종료 실패 — 탭을 다시 닫아 재시도 / 지금 켜기를 누르면 삭제가 취소됩니다'", () => {
    expect(code).toContain("종료 실패 — 탭을 다시 닫아 재시도 / 지금 켜기를 누르면 삭제가 취소됩니다");
    const i = code.indexOf("function renderIdleWorkspace(");
    expect(i).toBeGreaterThan(0);
    const seg = code.slice(i, code.indexOf("box.append(msg, btn);", i));
    expect(seg).toContain("ws.stopFailed");
  });

  it("★표식은 종료 실패 두 경로가 세우고, 켜기 성공이 내린다(삭제 취소 = 묘비 해소)", () => {
    expect((code.match(/ws\.stopFailed = true;/g) ?? []).length).toBe(2); // 탭 닫기 · 그룹 삭제
    expect(code).toContain("ws.stopFailed = undefined;");
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★U2+U3(0.14.41 · WP-A1) — 좌석 배치는 seatlayout.ts **한 모듈**을 거친다.
//   U3: 새 좌석을 트리 전체 오른쪽에 반반으로 붙여 대표 칸이 1/2→1/4→1/8 로 줄던 규칙을 없앤다.
//   U2: 칸에 역할을 적어 두고, 재부팅으로 sid 가 바뀌어도 같은 역할의 새 창을 원래 칸에 결속한다.
//   판정은 seatlayout.test.ts·seatbind.test.ts 가 잠그고, 여기서는 **배선**을 센다(순수 함수가 옳아도
//   main.ts 가 부르지 않으면 화면은 그대로 틀린다 — 이 파일 머리말의 교훈).
// ────────────────────────────────────────────────────────────────────────────
describe("U2+U3 좌석 배치 배선 — 대표 1/3 · 역할 자리 기억(seatlayout.ts)", () => {
  // actionSplit 의 경합 폴백은 `dir` 변수라서 걸리지 않는다(사용자가 지정한 분할 방향 — 의도된 동작).
  const LEGACY_APPEND = /type: "split", dir: "row", a: [\w.()]+, b: \{ type: "pane"/;

  it("★옛 '오른쪽 반반 부착' 리터럴이 main.ts 코드에 0회(3초 입양·기동 병합·+New·전출 런처 셸 — 반박 U3 D9)", () => {
    expect(LEGACY_APPEND.test(code)).toBe(false);
    // 종전 규칙이 필요한 폴백은 seatlayout.ts 의 legacyAppend 한 곳에만 있다.
    expect(code).toContain("placeSeatSafe(");
  });

  it("3초 입양 틱: 종료 좌석은 역할 맵에서 null(반박 U3 D4) · 결속 우선 입양(adoptSeat) · 입양 가드 유지", () => {
    const i = code.indexOf("for (const s of [...r.surfaces].sort(");
    expect(i).toBeGreaterThan(0);
    const seg = code.slice(i, code.indexOf("adopted = true;", i));
    expect(seg).toContain("seatPriority(");
    expect(seg).toContain("adoptSeat(");
    expect(seg).toContain("!w.pending && !w.deleting");
    expect(code).toContain("s.exited ? null : s.role");
  });

  it("3초 틱: 역할 기억은 바뀐 틱에만 저장(annotateRoles) · 유령 집행은 역할 칸을 구멍으로(holdRolePane) · 구멍 위생(tidyHoles)", () => {
    const a = code.indexOf("async function refreshPaneTitles(");
    const seg = code.slice(a, code.indexOf("setInterval(refreshPaneTitles, 3000);", a));
    expect(seg).toContain("annotateRoles(");
    expect(seg).toContain("holdRolePane(sid, sk,");
    expect(seg).toContain("tidyHoles(");
    expect(seg).toContain("holeUntil.delete(");
  });

  it("★기동 복원: 역할 결속(restoreTree)이 죽은 칸 제거(deadLiveSids) **앞** · 병합은 adoptSeat · S5(anchorHeadSafe)는 병합 뒤·빈 탭 충전 앞", () => {
    const s = restoreSlice();
    const rt = s.indexOf("restoreTree(");
    const dl = s.indexOf("deadLiveSids(");
    expect(rt).toBeGreaterThan(0);
    expect(dl).toBeGreaterThan(rt);
    const merge = s.indexOf("adoptSeat(", dl);
    const s5 = s.indexOf("anchorHeadSafe(", merge);
    const fill = s.indexOf("newSurface(null, ws.socket, T_NEW)", s5);
    expect(merge).toBeGreaterThan(dl);
    expect(s5).toBeGreaterThan(merge);
    expect(fill).toBeGreaterThan(s5);
  });

  it("기동: 데몬 세대는 **이미 부르는** daemon_status 응답에서 읽는다(새 왕복 0 · daemonIdentOf)", () => {
    const s = startSlice();
    expect(s).toContain("daemonIdentOf(");
    expect((s.match(/invoke\("daemon_status"/g) ?? []).length).toBe(3); // 300ms 프로브 · 본부 · 부서 생존 — 늘면 red
  });

  it("수동 표식: 구버전 저장본 1회 추정(looksManual) · 3px 이상 분할선 드래그·pane 이동에서만 켜짐 · 정렬이 끈다", () => {
    expect(code).toContain("looksManual(");
    const d = code.indexOf("function attachDividerDrag(");
    expect(d).toBeGreaterThan(0);
    const dseg = code.slice(d, code.indexOf("\n}\n", d));
    expect(dseg).toContain("layoutManual = true");
    expect(dseg).toMatch(/>= 3/); // 클릭·합성 mousemove 로는 켜지지 않는다(반박 U3 D8)
    const mv = code.slice(code.indexOf("function movePane("), code.indexOf("function setFocus("));
    expect(mv).toContain("layoutManual = true");
    const eq = code.slice(code.indexOf("async function actionEqualize("), code.indexOf("// ---------- workspace tabs"));
    expect(eq).toContain("roleLayout(");
    expect(eq).toContain("layoutManual = false");
  });

  it("정렬·빗질 정의는 seatlayout.ts 한 곳뿐(main.ts 이중 정의 금지 · 규칙이 다시 갈라지지 않게)", () => {
    expect(code).not.toContain("function roleLayout(");
    expect(code).not.toContain("function evenComb(");
    expect(code).not.toContain("function firstWithRole(");
    expect(code).not.toContain("const rolePri =");
  });

  it("종료 이벤트: exited·reaped 는 역할 칸을 구멍으로 보류 · closed(의도된 닫기·전출 원본)는 종전대로 뗀다", () => {
    expect(code).toContain('removeDeadPane(Number(sid), sock, name !== "surface.closed")');
  });

  it("구멍(음수 sid)은 collectSids 에 나오지 않는다 — 닫기·포커스·RPC 경로가 구멍을 보지 않는다", () => {
    const i = code.indexOf("function collectSids(");
    expect(i).toBeGreaterThan(0);
    const seg = code.slice(i, code.indexOf("\n}\n", i));
    expect(seg).toContain("node.sid > 0");
  });

  it("렌더: 보이는 칸이 없으면(구멍만) idle 패널 · 보류 칸은 [새 셸 열기]/[칸 비우기] 손잡이(④ 백지 금지 · 반박 U2 major ⓐ)", () => {
    expect(code).toContain("nodeShown(");
    const i = code.indexOf("function renderRoleSlot(");
    expect(i).toBeGreaterThan(0);
    const seg = code.slice(i, code.indexOf("\n}\n", i));
    expect(seg).toContain('"새 셸 열기"');
    expect(seg).toContain('"칸 비우기"');
    expect(seg).toContain("newSurface(null, ws.socket, T_NEW)");
    expect(seg).toContain("roleSlotText(");
  });

  it("보류 기한은 winScaled(Windows ×2 = 480s) 한 곳", () => {
    expect(code).toContain("const ROLE_SLOT_GRACE = winScaled(ROLE_SLOT_GRACE_MS)");
  });
});

describe("seatlayout.ts — 순수 모듈 격리(import 0 · 부작용 식별자 0 · 구형 WKWebView 문법 0 · 최상위 부수효과 0)", () => {
  const sl = readFileSync(new URL("./seatlayout.ts", import.meta.url), "utf-8");
  // 주석과 문자열 리터럴을 걷어낸 코드 본문 — 설명문·문구가 핀을 깨거나 속이지 않게.
  const body = stripComments(sl).replace(/`[^`]*`|"(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'/g, '""');

  it("import 문이 없다 — invoke·Tauri·DOM 을 끌어올 경로 자체가 없다(문자열 대신 import 그래프로 핀 · 반박 U2 minor)", () => {
    expect(/^\s*import\s/m.test(sl)).toBe(false);
    expect(/\brequire\s*\(/.test(body)).toBe(false);
  });
  it("부작용·생성·전송 식별자 0(UI 는 claim·launch·newSurface·send 를 새로 부르지 않는다)", () => {
    for (const id of ["invoke", "__TAURI__", "window", "document", "localStorage", "setTimeout", "setInterval", "fetch", "newSurface", "claim", "launch", "send"])
      expect(new RegExp(`\\b${id}\\b`).test(body)).toBe(false);
  });
  it("구형 WKWebView 비호환 문법 0", () => {
    for (const bad of ["(?<=", "(?<!", ".at(", "findLast", "structuredClone", "Object.hasOwn", "replaceAll("]) expect(sl.includes(bad)).toBe(false);
  });
  it("최상위 문장은 선언뿐(export/const/function/type/interface) — 모듈 로드만으로 아무 일도 일어나지 않는다", () => {
    const tops = sl.split("\n").filter((l) => /^[A-Za-z]/.test(l));
    expect(tops.length).toBeGreaterThan(10);
    for (const l of tops) expect(/^(export (const|function|type|interface) |const |function |type |interface )/.test(l)).toBe(true);
  });
});
