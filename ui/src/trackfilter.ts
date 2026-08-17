// PTY→xterm 출력 스트림에서 '마우스 트래킹 모드 전환' 시퀀스를 제거한다 — 근본 수리(B안)
// + 양방향 정합기(스펙 D4+A8, 2026-08-16): mac 에서 alt 화면 구간만 앱에 마우스를 돌려준다.
//
// 왜 필요한가(2026-08 현장 제보 — A안의 잔여 반쪽):
// Claude Code TUI가 DECSET 1003h/1006h 로 마우스 트래킹을 켜면 xterm.js 는 ①휠을 로컬 스크롤
// 대신 보고로 앱에 보내고 ②선택(selection)을 끈다. A안(mousefilter 휠 번역 + Option드래그)은
// 스크롤과 보조키 선택을 복구했지만, **일반 드래그 선택**은 xterm 이 트래킹 모드에 들어가는 한
// 구조적으로 불가능하다. 그래서 모드 전환 자체를 UI 계층(term.write 직전)에서 걷어낸다 —
// xterm 이 트래킹 모드에 아예 진입하지 않으면 휠·드래그 선택·복사 전부 기본 동작이다.
//
// 왜 정합기가 필요한가(스펙 D4 — 문제2: mac fullscreen 휠→프롬프트 히스토리 오염):
// 전면 스트리핑은 alt 화면(스크롤백 없는 전체화면 TUI)에서 역효과다 — 트래킹 미진입 xterm 은
// alt 의 휠을 방향키(CUU/CUD)로 합성해 앱에 보내고, Claude Code 는 그걸 프롬프트 히스토리
// 탐색으로 소비해 입력 히스토리가 오염된다. ∴ 장부(앱의 희망 트래킹 상태)를 기록해 두고,
// alt 진입 전이에서 장부를 재생 주입 + alt 구간의 명시 전환은 통과 + 복귀 전이에서 전 집합
// 상수 DECRST 로 소등 후 스트리핑 재개한다. primary 화면의 휠·드래그·복사 기본 동작(원 수리
// 목표)과 alt 화면의 앱 마우스 소비(문제2 수리)를 양립시키는 것이 이 정합기다.
//
// ★배치 계층 계약: 이 필터는 **UI 렌더 직전에만** 존재한다. 데몬 계층에 두면 read_text 가상
// 스크린을 소비하는 readiness 판정·거버넌스 프로브 전체가 파급권에 들어간다(부트 체인 접촉 금지).
//
// ★fail-open 계약(ABSOLUTE ANCHOR ④ — "터미널 백지화"류 결함 봉인):
//  · 홀드백(청크 경계에 걸린 미완성 후보)은 CARRY_CAP 바이트까지만 — 초과 시 원문 그대로 방류.
//  · 판정 불가·비정형 시퀀스는 전부 원문 통과. 이 필터가 바이트를 '지연'시킬 수 있는 유일한
//    경우는 (합법 DECSET 접두의 미완성 꼬리 ≤ CARRY_CAP)이고, 다음 청크가 오면 즉시 해소된다.
//  · 정합기의 전이 시 상수 주입은 **추가**일 뿐 지연이 아니다 — fail-open 불변식 무접촉.
//  · 킬스위치: localStorage.cysAllowAppMouse = "1" → 필터 전면 우회(앱이 마우스를 갖는다 —
//    vim mouse=a 등 TUI 마우스가 필요한 사용자용. 새 pane 부터 적용). 배선은 main.ts.
//  · 롤백 스위치: localStorage.cysMouseReconcilerOff = "1" → 정합기(소비)만 비활성 —
//    win 비활성 코드 경로 재사용(신규 로직 0). 배선은 main.ts(pane 생성 시 캡처).
//
// 트레이드오프(의도된 결정): primary 화면에서 TUI 앱은 마우스 이벤트를 받지 못하고, alt
// 트래킹 중 일반 드래그 선택은 Option+드래그로 대체된다(오너 버그 수정 지시·스펙 D4 명기).

// 제거 대상 DECSET/DECRST 파라미터 — 마우스 트래킹 계열 전부.
//   9=X10 · 1000=press/release · 1002=drag · 1003=any-motion · 1005=UTF-8 좌표 ·
//   1006=SGR 좌표 · 1015=urxvt 좌표 · 1016=SGR-Pixels 좌표.
// ★1004(focus in/out)·1049(alt screen)·2004(bracketed paste) 등은 마우스가 아니다 — 보존.
const MOUSE_PARAMS = new Set([9, 1000, 1002, 1003, 1005, 1006, 1015, 1016]);

// alt 화면 전이 감시 파라미터 — h=alt 진입, l=복귀 (xterm InputHandler 와 동일 집합).
// ★전이는 **소비 활성(mac) 경로에서** 장부를 변경하지 않는다 — 장부는 앱의 명시 마우스
// DECSET/DECRST 로만 갱신되는 '희망 상태'이고, 전이는 주입/소등 트리거일 뿐이다
// (suspend/resume 재진입에서 재생이 빈손이 되지 않게 — 스펙 D4 장부 의미론).
// ★예외 1건(소비 비활성 = Windows·정합기 롤백): **alt 이탈 파라미터를 '관측'하면 장부를
// 소거한다** — 내부 altActive 추적 상태와 무관하게(재부착 pane 은 진입을 못 봐 altActive 가
// false 로 시작하므로, 전이로 묶으면 그 pane 의 stale 이 영구 잔존한다). 그 경로엔 재생 주입이
// 없어 보존할 이유가 없고, 남겨 두면 다음 전체화면 앱이 이전 앱의 1003 을 물려받아 Windows
// 휠 가드가 오발한다(아래 [전이] 블록의 `!consume` 분기 주석이 정본).
// 출력 바이트는 무변경이라 win-parity 계약과 무충돌이다.
const ALT_PARAMS = new Set([47, 1047, 1049]);

// 복귀·리셋 소등용 **전 집합 상수** DECRST — 장부 부분집합이 아니라 상수로 고정한다:
// CARRY_CAP 초과 fail-open 으로 방류(유출)된 enable 까지 소등하는 안전망이다(멱등·무해).
// ★1049l 은 포함하지 않는다 — 앱/셸이 alt 에 그리는 중일 수 있어 강제 복귀는 화면 정합 파괴.
export const MOUSE_ALL_OFF = [...MOUSE_PARAMS].map((p) => `\x1b[?${p}l`).join("");

// 미완성 후보 홀드백 상한 — 합법 `ESC[?` + 파라미터 나열이 이 길이를 넘는 실전 사례는 없다.
// 넘으면 후보가 아니라고 보고 원문 방류(fail-open — 지연·소실 0).
const CARRY_CAP = 32;

const ESC = 0x1b;
const BRACKET = 0x5b; // '['
const QMARK = 0x3f; // '?'
const SEMI = 0x3b; // ';'
const RIS_FINAL = 0x63; // 'c' — ESC c (RIS)

function isDigit(b: number): boolean {
  return b >= 0x30 && b <= 0x39;
}

function pushStr(out: number[], s: string): void {
  for (let k = 0; k < s.length; k++) out.push(s.charCodeAt(k));
}

// 정합기 상태 — MouseTrackingFilter 가 소유하고 processChunk 가 갱신한다.
interface ReconcilerState {
  // 소비(주입·alt 통과·소등) 활성 = os==='mac' ∧ reconcile. false 면 장부 '기록만' 하고
  // 출력은 순수 filterChunk 와 byte-identical 이다(win-parity 계약 — 롤백 스위치도 이 경로).
  consume: boolean;
  // 스트림 관측 기준 alt 화면 여부. 전이({47,1047,1049} h/l)·RIS 로만 바뀐다 —
  // reset()/장부 소거는 건드리지 않는다(스트림 진실 보존).
  altActive: boolean;
  // 장부 = 앱 희망 트래킹 상태(파라미터별 최종 h/l). Map 삽입 순서 = 앱의 enable 시간순 —
  // 재생 주입이 프로토콜 우선순위(마지막 enable 승리)를 보존하도록 h 재기록 시 말미로 옮긴다.
  ledger: Map<number, boolean>;
  // 관측 세대 — 앱 명시 마우스 h/l·alt 전이 파라미터·RIS 관측마다 증가. agent.exited 리셋
  // 훅의 에포크 가드(main.ts)가 읽는다: 이벤트 수신 후 세대가 움직였으면 장부 소거 생략.
  generation: number;
}

// 한 청크를 필터한다 — (출력 바이트, 다음 청크에 이어붙일 미완성 꼬리) 계산.
// st=null 이면 v1 스트리핑 전용(순수 filterChunk)과 완전 동일 동작이고, st 가 있으면 장부
// 기록(공통) + st.consume 일 때만 소비(주입·alt 통과·소등)가 더해진다. 단일 구현을 공유해
// '소비 비활성 = filterChunk 와 byte-identical' 을 구조적으로 보장한다(win-parity 핀).
function processChunk(
  input: Uint8Array,
  st: ReconcilerState | null,
): { out: Uint8Array; carry: Uint8Array } {
  const out: number[] = [];
  let i = 0;
  const n = input.length;
  while (i < n) {
    const b = input[i];
    if (b !== ESC) {
      out.push(b);
      i++;
      continue;
    }
    // ESC 발견 — DECSET/DECRST 후보(`ESC [ ? params h|l`)인지 전방 탐색.
    // 후보 확정 전에 청크가 끝나면 carry(캡 이내)로 보류한다.
    if (i + 1 >= n) {
      return { out: new Uint8Array(out), carry: input.slice(i) };
    }
    if (input[i + 1] !== BRACKET) {
      // ESC 이지만 CSI 아님. 정합기는 RIS(ESC c)만 전이로 본다 — xterm 의 fullReset 은 버퍼와
      // 마우스 프로토콜을 모두 리셋하므로 alt 복귀+장부 소거와 동일 처리한다(스펙 A8).
      if (st && input[i + 1] === RIS_FINAL) {
        out.push(input[i], input[i + 1]); // `ESC c` 원문 방출(바이트 무변경)
        st.altActive = false;
        st.ledger.clear();
        st.generation++;
        if (st.consume) pushStr(out, MOUSE_ALL_OFF); // 방출 직후 소등 주입(멱등) 후 스트리핑 재개
        i += 2;
        continue;
      }
      out.push(b); // 그 외 비CSI ESC — 원문 통과, 다음 바이트부터 재스캔
      i++;
      continue;
    }
    if (i + 2 >= n) {
      return { out: new Uint8Array(out), carry: input.slice(i) };
    }
    if (input[i + 2] !== QMARK) {
      // 일반 CSI — 손대지 않는다. `ESC[` 두 바이트를 통과시키고 이어서 스캔
      // (파라미터부에 ESC 는 올 수 없으므로 이어지는 바이트는 일반 루프가 그대로 통과시킨다).
      // ★DECSTR(`CSI ! p`)도 여기로 흘러 **무처리**다 — xterm 의 softReset 은 버퍼·마우스
      //   프로토콜을 건드리지 않으므로(벤더 InputHandler.ts:2692) 정합기도 전이로 보지 않는다.
      //   alt 복귀로 오판하면 정당한 트래킹 앱이 DECSTR 1회로 마우스를 잃는 원 결함 계열이
      //   부활한다(스펙 A8 — 역회귀 테스트 핀). crash-reset 동선은 RIS 가 전담한다.
      out.push(input[i], input[i + 1]);
      i += 2;
      continue;
    }
    // `ESC [ ?` 확정 — 파라미터(digits·';')를 소비하고 최종 바이트를 본다.
    let j = i + 3;
    while (j < n && (isDigit(input[j]) || input[j] === SEMI)) j++;
    if (j >= n) {
      // 미완성 — 캡 이내면 보류, 초과면 후보 아님(fail-open 원문 방류)
      if (n - i <= CARRY_CAP) {
        return { out: new Uint8Array(out), carry: input.slice(i) };
      }
      for (let k = i; k < n; k++) out.push(input[k]);
      i = n;
      continue;
    }
    const fin = input[j];
    if (fin !== 0x68 && fin !== 0x6c) {
      // 'h'/'l' 아님(DECRQM `$p` 등) — 스캔한 원문 통과. ★fin 은 소비하지 않고 그 위치에서
      // 재스캔한다(2026-08-12 R2 확정): 종전처럼 fin 까지 push·i=j+1 로 건너뛰면, fin==ESC
      // (중단된 후보 직후에 새 시퀀스가 시작하는 `\x1b[?25\x1b[?1003h` 류)에서 뒤따르는 마우스
      // enable 이 필터를 그대로 통과하고 — 이후 disable(`?1003l`)은 정상 스트리핑돼 걷히므로
      // pane 이 닫힐 때까지 트래킹이 영구 고착됐다(sticky). fin 을 남겨 두면 ESC 는 다음 루프의
      // 후보 스캔으로, 일반 바이트는 원문 통과로 각각 올바르게 처리된다(출력 바이트 불변).
      for (let k = i; k < j; k++) out.push(input[k]);
      i = j;
      continue;
    }
    // DECSET(h)/DECRST(l) — 정합기 훅(장부·세대) → 방출 → 전이 주입 순서.
    const paramText = String.fromCharCode(...input.slice(i + 3, j));
    const params = paramText.split(";").filter((p) => p !== "");
    const nums = params.map(Number);
    const isSet = fin === 0x68; // 'h'
    const hasAltParam = nums.some((p) => ALT_PARAMS.has(p));

    // [장부·세대] 앱의 명시 마우스 파라미터 h/l 만 장부를 갱신한다(전이 파라미터는 불변).
    if (st) {
      let observed = hasAltParam;
      for (const p of nums) {
        if (!MOUSE_PARAMS.has(p)) continue;
        observed = true;
        if (isSet) {
          st.ledger.delete(p); // 재-enable 은 말미로 — 재생 주입이 enable 시간순을 보존
          st.ledger.set(p, true);
        } else {
          st.ledger.set(p, false);
        }
      }
      if (observed) st.generation++;
    }

    // [방출] alt 구간(소비 활성)은 명시 전환 통과, 그 외(primary·소비 비활성)는 현행 스트리핑.
    if (st !== null && st.consume && st.altActive) {
      for (let k = i; k <= j; k++) out.push(input[k]); // alt 중 통과 — 앱이 마우스를 갖는다
    } else {
      const kept = params.filter((p) => !MOUSE_PARAMS.has(Number(p)));
      if (kept.length === params.length) {
        // 마우스 파라미터 전무 — 원문 그대로(바이트 무변경 계약)
        for (let k = i; k <= j; k++) out.push(input[k]);
      } else if (kept.length > 0) {
        // 부분 제거 — 비마우스 파라미터(1049·2004 등)는 순서 보존 재조립
        const rebuilt = `\x1b[?${kept.join(";")}${String.fromCharCode(fin)}`;
        pushStr(out, rebuilt);
      }
      // kept.length === 0 → 시퀀스 통째 제거(방출 없음)
    }

    // [전이] h=alt 진입 / l=복귀 — 주입 위치는 이 시퀀스(재조립분 포함) **방출 직후**다.
    // 청크 말미 일괄 주입은 금지: 같은 청크의 후속 바이트(앱의 즉시 DECRST 등)와 순서가
    // 역전된다(스펙 D4). ★주입 바이트는 out 직접 삽입 — feed 재진입 금지(주입분이 자기
    // 스트리핑돼 사라진다).
    if (st && hasAltParam) {
      if (isSet && !st.altActive) {
        st.altActive = true;
        if (st.consume) {
          // 장부상 희망 트래킹 재생 주입 — xterm 을 앱과 같은 트래킹 상태로 데려간다.
          for (const [p, on] of st.ledger) if (on) pushStr(out, `\x1b[?${p}h`);
        }
      } else if (!isSet && st.altActive) {
        st.altActive = false;
        if (st.consume) pushStr(out, MOUSE_ALL_OFF); // 전 집합 상수 소등 후 스트리핑 재개
      }
      // ★소비 비활성(=Windows · 정합기 롤백) 한정 장부 소거 — 휠 가드 stale 봉인
      //   (적대검증 2R major). **출력 바이트는 한 개도 바뀌지 않는다**(win-parity 불변).
      //
      // 왜 이 경로에서만 지우는가: 소비 활성(mac)에서 장부는 '재생 주입의 원본'이라 alt 이탈
      // 후에도 보존해야 한다(suspend/resume 재진입에서 재생이 빈손이 되지 않게 — 위 ALT_PARAMS
      // 주석의 장부 의미론). 그러나 **소비 비활성 경로에는 재생이 아예 없다** — 장부의 유일한
      // 독자는 wheelgate 의 Windows 술어가 읽는 ledgerWantsAnyMotion() 하나이고 그 술어는 alt
      // 화면일 때만 의미가 있다. ∴ 이 경로에서 alt 이탈 뒤의 장부는 '다음 전체화면 앱에게
      // 물려주는 오염'일 뿐이다.
      //
      // 무엇을 막는가: Windows 는 `!xtermTracking` 이 상수 true 라(wheelgate.ts (b) 의 단일
      // 방어선 고지) 억제를 푸는 해제항이 없다. 그래서 장부에 1003 이 남은 채 다음 앱이 alt 에
      // 들어가면 — 그 앱이 마우스를 요청하지 않는 less·man·vim 이라도 — 술어가 충족돼 그 pane 이
      // 닫힐 때까지 휠이 죽는다.
      //
      // ★위 else-if 안이 아니라 **밖**에 두는 이유(재부착 경로 봉인): 소거 조건을
      // `st.altActive` 가 참일 때로 묶으면 **재부착 pane 의 사각**이 남는다. 재부착이 보내는
      // 초기 화면은 셀 내용 스냅샷뿐이라 alt 진입 시퀀스가 재발행되지 않는다 → 이미 전체화면인
      // 앱에 붙어도 st.altActive 는 false 로 시작한다. 그 상태에서 앱이 마우스 DECSET 을 다시
      // 내면 장부에는 1003 이 오르는데, 이어지는 `?1049l` 은 (altActive=false 라) 전이로 잡히지
      // 않아 장부가 그대로 남는다 — 그 다음 앱부터 휠이 죽는다. 관측된 이탈 시퀀스는 추적
      // 상태와 무관하게 '전체화면 소유자가 끝났다'는 사실이므로, 여기서는 그것만 본다.
      // 오판 방향도 안전하다: 잘못 지우면 **비억제**(=가드 도입 전 HEAD 동작)로 떨어질 뿐이고,
      // 앱이 마우스 모드를 다시 선언하면 즉시 재무장한다.
      //
      // 남는 잔여(정직 고지): 앱이 **이탈 시퀀스조차 못 내고 즉사**하면(Windows 는 Job 오브젝트
      // 강제종료라 mac 보다 구조적으로 흔하다) 여기 도달하지 않는다. 그 경우 화면은 이미 alt
      // 버퍼에 갇혀 있어 사용자 처방이 'pane 을 새로 연다'로 같고(USER-MANUAL §4.6b 명기),
      // 에이전트 pane 은 main.ts 의 agent.exited 훅이 에포크 가드로 장부를 소거한다.
      if (!st.consume && !isSet) st.ledger.clear();
    }
    i = j + 1;
  }
  return { out: new Uint8Array(out), carry: new Uint8Array(0) };
}

// 한 청크를 필터한다 — (출력 바이트, 다음 청크에 이어붙일 미완성 꼬리) 순수 계산(v1 계약).
// 호출측(MouseTrackingFilter)이 carry 를 앞에 이어붙여 재호출하는 것으로 상태를 만든다.
export function filterChunk(input: Uint8Array): { out: Uint8Array; carry: Uint8Array } {
  return processChunk(input, null);
}

// 정합기 생성 옵션 — pane 생성 시 캡처해 주입한다(라이브 재판독 금지: 수명 중 토글 시
// 필터/휠 억제 상태가 분열한다 — allowAppMouse 킬스위치와 동형 계약. 배선은 main.ts).
export interface TrackFilterOpts {
  os: "mac" | "win";
  // 롤백 스위치(localStorage.cysMouseReconcilerOff) off → false: 소비 전부 비활성 —
  // win 비활성 코드 경로 재사용(신규 로직 0), 출력은 현행 filterChunk 와 byte-identical.
  reconcile: boolean;
}

// pane 수명 전체를 관통하는 스테이트풀 래퍼 — attach 스냅샷 재생과 라이브 스트림이 같은
// 인스턴스를 지나야 한다(스냅샷 안의 1003h 가 살아남으면 재생 순간 트래킹 모드 진입).
export class MouseTrackingFilter {
  private carry: Uint8Array = new Uint8Array(0);
  private readonly st: ReconcilerState;

  // 무인자 생성 = 현행(v1) 스트리핑 전용과 동일 출력(소비 비활성). 정합기는 opts 로 옵트인 —
  // 소비 활성 = os==='mac' ∧ reconcile(스펙 D4: Windows 는 장부 기록만 공통·소비 전부 비활성).
  constructor(opts?: TrackFilterOpts) {
    this.st = {
      consume: opts?.os === "mac" && opts?.reconcile === true,
      altActive: false,
      ledger: new Map(),
      generation: 0,
    };
  }

  feed(chunk: Uint8Array): Uint8Array {
    let input = chunk;
    if (this.carry.length > 0) {
      input = new Uint8Array(this.carry.length + chunk.length);
      input.set(this.carry, 0);
      input.set(chunk, this.carry.length);
    }
    const { out, carry } = processChunk(input, this.st);
    this.carry = carry;
    return out;
  }

  // 스트림 종료(surface exited) 시 잔여 carry 방류 — 앱이 시퀀스 중간에서 죽어도 바이트
  // 영구 소실 0 을 보장한다(미완성 후보는 어차피 렌더 무해한 접두다). 호출측: main.ts exited.
  flush(): Uint8Array {
    const rest = this.carry;
    this.carry = new Uint8Array(0);
    return rest;
  }

  // 장부에 희망 h(트래킹 요청) 파라미터가 있는가 — wheelgate 술어의 ledgerWantsMouse 입력.
  ledgerWantsMouse(): boolean {
    for (const on of this.st.ledger.values()) if (on) return true;
    return false;
  }

  // 장부에 1003(any-motion)이 희망 h 로 올라 있는가 — Windows 전용 휠 억제 술어
  // (wheelgate.shouldSuppressWheelWin)의 ledgerWantsAnyMotion 입력. **읽기 전용**이다:
  // 장부는 processChunk 만 갱신하고 이 접근자는 조회만 한다(출력 바이트 무접촉 — win-parity 핀).
  //
  // ★왜 1003 인가(ledgerWantsMouse 의 '아무거나 하나' 보다 좁게 보는 이유):
  //  · Claude Code TUI 는 alt 진입 시 1000+1002+1003+1006("full" 마우스 모드)을 한꺼번에 켠다
  //    → 1003 이 반드시 장부에 오른다 → 술어 충족 → fullscreen 휠 억제 발동.
  //  · vim 의 `set mouse=a` 는 통상 1000/1002/1006 만 켜고 1003 은 켜지 않는다 → 술어 불충족
  //    → Windows 에서도 방향키 합성이 그대로 보존된다(현행 vim 휠 UX 무회귀).
  //  ∴ 1003 은 "앱이 버튼 안 눌린 상태의 이동까지 원한다" = 전면 마우스 장악 의사의 표지이고,
  //    그것이 휠을 앱에 넘겨야 하는(=xterm 의 방향키 합성을 막아야 하는) 경우와 겹친다.
  //
  // ★이 설계의 최대 미확정(정직 고지): 위 vim 서술은 이 저장소가 **실측으로 뒷받침하지 못한다** —
  //  vim 의 실제 DECSET 캡처가 저장소에 0건이다(Claude Code 쪽 "full" 집합만 근거가 있다).
  //  vim 빌드·설정에 따라 1003 을 켜는 경로가 있다면 ⓑ(vim 비억제) 전제가 무너진다.
  //  다음 조사자는 win/mac vim 의 실제 출력 바이트를 캡처해 이 주석을 갱신하라.
  //
  // ★수명(stale 봉인 — 적대검증 2R): 이 값이 참인 구간은 **1003h 관측 ~ 다음 alt 이탈 관측**
  //  까지다. 소비 비활성(=Windows) 인스턴스는 alt 이탈 파라미터(1049l·47l·1047l)를 관측하면
  //  내부 altActive 상태와 무관하게 장부를 소거한다(processChunk 의 [전이] `!consume` 분기 —
  //  재부착 pane 까지 덮기 위함). RIS(ESC c)·reset()·clearLedgerIfGeneration 도 같은 효과다.
  //  Windows 는 `!xtermTracking` 항이 상수 true 라 억제를 푸는 해제항이 없어, 이 소거가 없으면
  //  그 pane 이 닫힐 때까지 휠이 죽었다.
  ledgerWantsAnyMotion(): boolean {
    return this.st.ledger.get(1003) === true;
  }

  // 관측 세대 번호 — agent.exited 리셋 훅의 에포크 가드가 캡처한다(main.ts).
  generation(): number {
    return this.st.generation;
  }

  // 에포크 가드형 장부 소거: 세대가 gen 그대로일 때만 소거하고 true. 그 사이 앱의 명시
  // DECSET/alt 전이가 관측됐으면(새 앱이 이미 기동) 장부를 보존하고 false — 호출측은
  // 상수 소등 주입만 멱등 수행한다(스펙 D4 ② 에포크 가드).
  clearLedgerIfGeneration(gen: number): boolean {
    if (this.st.generation !== gen) return false;
    this.st.ledger.clear();
    return true;
  }

  // 리셋 훅(스펙 D4 ③): 장부 소거 + 8종 전 집합 상수 DECRST 반환 — **1049l 미포함**(앱/셸이
  // alt 에 그리는 중일 수 있어 강제 복귀는 화면 정합 파괴). 호출측이 term.write 로 **직접**
  // 기록한다 — feed 로 흘리면 주입분이 자기 스트리핑된다(필터 재진입 금지). altActive 는
  // 보존 — 스트림 관측 진실만 따른다(xterm 버퍼 상태와의 자의적 분열 금지).
  reset(): string {
    this.st.ledger.clear();
    return MOUSE_ALL_OFF;
  }
}
