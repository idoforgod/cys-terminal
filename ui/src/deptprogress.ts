// ui/src/deptprogress.ts — 「팀 직접 만들기」 대기 문구(경과·단계)와 팀원 부팅 안내 문구·판정, 팩의 단계 표지 해석(0.14.43 · GU · 성찰 1회차 R1F-UB 수정).
//
// 왜 필요한가: 팀을 만드는 동안 화면은 "곧 끝난다"는 짧은 약속을 했지만 실측은 보통 25~30초(이 맥)이고 느린 PC 는 1분을 넘는다 —
// 거짓 약속이다. 그 뒤 팀원(부서장·CSO·워커·리뷰어 둘)이 켜지는 동안(이 맥 실측 1회 약 5분)에는 화면에 아무 안내가 없었고, 첫 자리가 붙기 전에는
// "이 부서는 아직 켜지 않았습니다" 가 잠깐 보였다(방금 만든 팀에는 거짓). 이 모듈은 그 두 구간의 **문구와 해석**만 만든다.
//
// 팩(cysjavis-pack/bin/cys-dept)은 팀을 만드는 동안 stderr 에 `[cys-dept] @stage <키>` 한 줄씩을 낸다(키 7종: reserve probe spawn wait up seat done).
// Tauri(allocate_dept_daemon)가 그 줄을 읽는 즉시 'dept-create-progress' 이벤트로 올리고, main.ts 가 대기 화면의 '지금: …' 줄을 고친다.
// 구 팩에는 표지가 없다 — 그때는 단계 줄 없이 경과만 보인다(deptPendingText 의 sub = null).
//
// 이 모듈은 문구·해석만 한다. 화면·IPC·저장소·타이머를 모른다(main.ts 가 배선·렌더를 한다 — 반복 타이머는 대기 화면 엘리먼트 수명에 묶인 1개뿐이다).
// ★문구는 **사실만**, 잰 만큼만 말한다(S4 M1): 대기 문구는 실측 범위만 약속하고(사전 검사 「약 12초」 · 팀원 「보통 5분 안팎」), 팀원 안내는 '자리가 붙었다'까지만 말한다 —
//   에이전트가 실제로 떴는지는 화면이 모르므로 '준비 완료'라고 단정하지 않는다.
// ★팀원 안내의 완료 판정은 **그 팀 소켓의 좌석 목록**(3초 틱이 이미 받는 list_surfaces)의 역할이다(성찰 1회차 S4 B1). 편성 결과 feed 는 본부 데몬으로 가서
//   (javis_formation.py `_feed` — 소켓 지정 없음) 화면이 부서 탭과 대응시킬 수 없다(socketForSlug 에는 부서 소켓만 있다) — 그 배선은 걷었다.
//   이 모듈은 좌석 목록 → 역할 → 판정(deptLiveRoles · deptSeatedCount · deptFormationVerdict)만 맡는다.
// ★이벤트 payload·좌석 목록은 **신뢰할 수 없는 데이터**다 — 단계 키는 정규식으로 거르고, 역할은 문자열만 본다.
//   렌더는 main.ts 가 텍스트 노드·stickyToast 로만 한다(HTML 삽입 없음).
// ★이 안내는 **표시 전용**이다 — 어떤 명령도 보내지 않는다.
//
// ★이 모듈의 불변식(deptprogress.test.ts 가 핀으로 고정 — starvednotice.ts·updatenotice.ts 와 같다):
//   · 최상위 부수효과 0 — 선언(import/export/const/function/interface/type)만. 브라우저 저장소·문서 객체·창 객체·타이머·IPC 접근 0
//     (main.js 는 번들 하나라 여기서 평가 중 예외가 나면 앱 전체가 백지가 된다).
//   · 구형 WKWebView 가 파싱하지 못하는 문법 0 — 정규식 뒤돌아보기·배열 끝 인덱스 접근·뒤에서 찾기·구조 복제·소유 판정 정적 메서드·전체 치환 계열.

// 의무 역할 목록의 출처 — 편성 로스터(javis_formation.py REQUIRED_ROLES)와 같은 상수 하나(드리프트 핀: deptcreate.test.ts). 여기에 두 번째 목록을 적지 않는다.
import { DEPT_SEAT_ROLES } from "./deptcreate";

/** 팀을 만드는 데 보통 걸리는 시간(초) — 이 맥 실측 25~30초. */
export const DEPT_TYPICAL_SECS = 30;
/** '평소보다 오래' 로 바꾸는 배수 — `DEPT_TYPICAL_SECS × DEPT_SLOW_FACTOR` = 90초부터. */
export const DEPT_SLOW_FACTOR = 3;
/** 팀원 부팅 안내의 상한(초) — 15분. 닿으면 주기 갱신을 멈추고, 그때까지 의무 역할 다섯이 다 안 붙었으면 '확인 필요'를 한 번 알린다(무한 갱신 금지). */
export const DEPT_FORMATION_CAP_SECS = 900;
/** 팀원 부팅 안내 토스트 id 의 접두 — id 는 `dept-formation:<소켓>` 이다(탭마다 하나 · 같은 id 는 갱신된다). */
export const DEPT_FORMATION_TOAST_PREFIX = "dept-formation:";
/**
 * 팀원 부팅 안내를 다시 내는 수명 갱신 칸(초). 안내는 sticky 토스트이고 기본 수명이 60초라(toastttl.ts), 분 단위로만 갱신하면
 * 갱신 사이가 60초를 넘는 틱에 토스트가 잠깐 사라졌다 다시 뜬다 — 이 칸이 바뀔 때도 다시 내서 갱신 사이를 60초 밑으로 둔다.
 * (45초 칸과 60초 분 경계는 어긋나므로 분이 바뀌는 순간은 따로 잡는다 — deptFormationNoticeKey.)
 */
export const DEPT_FORMATION_REFRESH_SECS = 45;
/** 방금 만든 팀의 첫 자리가 붙기를 기다리는 창(초) — 이 안에서만 빈 탭이 '첫 자리를 붙이는 중' 이라고 말한다. */
export const DEPT_FIRST_SEAT_WINDOW_SECS = 60;
/** 방금 만든 팀의 빈 탭 문구 — 창(위) 안에서만. */
export const DEPT_FIRST_SEAT_TEXT = "첫 자리를 붙이는 중입니다 — 잠시만 기다려 주세요";

/** 경과 표기의 상한(초) — 이상한 값이 지수 표기로 화면을 어지럽히지 않게(99시간 59분 59초). */
const ELAPSED_MAX_SECS = 359_999;
/** 자리 수 표기의 상한 — 같은 이유. */
const SEATS_MAX = 999;
/** 진행 id 의 접두 — `dp-<탭 번호>`. 호출마다 탭 번호가 새로 나오므로 호출마다 다른 문자열이다. */
const PROGRESS_ID_PREFIX = "dp-";
/** 진행 id 의 글자 수 상한 — 이상한 값은 버린다. */
const PROGRESS_ID_MAX = 64;
/** 단계 표지 한 줄의 접두 — 팩 `dept_stage` 가 이 모양으로 낸다(Rust 쪽 같은 이름의 파서와 같은 규칙). */
const STAGE_PREFIX = "[cys-dept] @stage ";
/** 단계 키 — 영소문자·숫자·`_`·`-` 만, 1~32자. */
const STAGE_KEY = /^[a-z0-9_-]{1,32}$/;

/** 팀원 부팅 안내의 상태 — booting(켜는 중 · 주기 갱신) · seated(의무 역할 자리가 모두 붙음 · 한 번) · check(15분 상한까지 다 안 붙음 · 한 번). */
export type DeptFormationState = "booting" | "seated" | "check";

/** 초 → 0 이상 정수. 숫자가 아니거나 음수·NaN·무한대면 0, 소수는 내림, 상한 ELAPSED_MAX_SECS. */
function normSecs(sec: unknown): number {
  if (typeof sec !== "number" || !isFinite(sec) || sec < 0) return 0;
  return Math.min(Math.floor(sec), ELAPSED_MAX_SECS);
}

/** 자리 수 → 0 이상 정수(상한 SEATS_MAX). 같은 규칙. */
function normSeats(n: unknown): number {
  if (typeof n !== "number" || !isFinite(n) || n < 0) return 0;
  return Math.min(Math.floor(n), SEATS_MAX);
}

/**
 * 대기 문구의 경과 표기 — 60초 미만은 `N초`, 60초 이상은 `M분 S초`(예: `1분 5초` · 정확히 60초는 `1분 0초`).
 * 음수·NaN·숫자 아님은 0초, 소수는 내림.
 */
export function formatDeptElapsed(sec: number): string {
  const s = normSecs(sec);
  return s < 60 ? `${s}초` : `${Math.floor(s / 60)}분 ${s % 60}초`;
}

/**
 * 팀원 부팅 안내의 경과 표기 — **분 단위(내림)**, 1분 미만은 `1분 미만`. 안내는 분이 바뀔 때만 다시 나가므로(deptFormationNoticeKey)
 * 초까지 적으면 갱신 사이에 낡은 값이 된다.
 */
export function formatDeptMinutes(sec: number): string {
  const s = normSecs(sec);
  return s < 60 ? "1분 미만" : `${Math.floor(s / 60)}분`;
}

/**
 * 단계 키 → 한글 한 줄. 모르는 키·빈 값·문자열이 아닌 값은 null(= 단계 줄을 그리지 않는다 — 구 팩·새 키에도 견딘다).
 * 키 표(팩 `dept_stage` 7종): reserve probe spawn wait up seat done.
 */
export function deptStageLabel(stage: string | null | undefined): string | null {
  switch (stage) {
    case "reserve":
      return "팀 번호를 잡는 중";
    case "probe":
      return "이미 켜진 데몬이 있는지 확인하는 중(약 12초)"; // 실측 12.8초 — '최대'가 아니다(같은 화면의 경과 초가 12를 넘는 것이 보인다)
    case "spawn":
      return "데몬을 켜는 중";
    case "wait":
      return "데몬이 팩을 설치하는 중(파일 수백 개)";
    case "up":
      return "데몬이 켜졌습니다 — 설정을 심는 중";
    case "seat":
      return "부서장 자리를 여는 중";
    case "done":
      return "마무리하는 중";
    default:
      return null;
  }
}

/**
 * 대기 화면의 문구 — 주 문구(main)와 단계 줄(sub).
 *  · main: 90초(= DEPT_TYPICAL_SECS × DEPT_SLOW_FACTOR) 미만이면 실측 범위(보통 30초 안팎 · 느린 컴퓨터는 1분 넘게)를 말하고,
 *    90초 이상이면 '평소보다 오래 걸린다 · 그대로 기다려 달라 · 중간에 닫으면 만들던 팀이 정리된다' 로 바뀐다. 둘 다 끝에 경과 시간이 붙는다.
 *  · sub: 단계 키를 아는 한글로 옮긴 `지금: <라벨>`. 키가 없거나 모르면 null.
 */
export function deptPendingText(elapsedSec: number, stage?: string | null): { main: string; sub: string | null } {
  const s = normSecs(elapsedSec);
  const elapsed = formatDeptElapsed(s);
  const main =
    s < DEPT_TYPICAL_SECS * DEPT_SLOW_FACTOR
      ? `팀을 만드는 중입니다 — 보통 ${DEPT_TYPICAL_SECS}초 안팎, 컴퓨터에 따라 1분 넘게 걸릴 수 있어요 · 경과 ${elapsed}`
      : `평소보다 오래 걸리고 있습니다 — 그대로 기다려 주세요(중간에 닫으면 만들던 팀이 정리됩니다) · 경과 ${elapsed}`;
  const label = deptStageLabel(stage);
  return { main, sub: label === null ? null : `지금: ${label}` };
}

/**
 * 단계 키를 거른다 — 영소문자·숫자·`_`·`-` 로만 이루어진 1~32자 문자열이면 그대로, 아니면 null.
 * (팩이 낸 줄을 Rust 가 파싱해 올린 이벤트 payload 도 한 번 더 이 규칙으로 거른다 — 이벤트는 신뢰하지 않는다.)
 */
export function sanitizeDeptStageKey(v: unknown): string | null {
  return typeof v === "string" && STAGE_KEY.test(v) ? v : null;
}

/**
 * 팩의 단계 표지 한 줄 → 키. `[cys-dept] @stage <key>` 꼴이면 key(영소문자·숫자·`_`·`-` · 32자 이하), 아니면 null.
 * 줄 끝의 줄바꿈(`\n`·`\r\n`)은 한 번씩 견딘다(윈도우 출력). 접두는 대소문자·공백까지 정확히 맞아야 한다.
 * Rust 쪽 `parse_dept_stage_line`(src-tauri/src/main.rs)과 같은 규칙이다 — 같은 입력 벡터를 두 검체가 함께 잰다.
 */
export function parseDeptStageLine(line: string): string | null {
  if (typeof line !== "string") return null;
  let s = line;
  if (s.endsWith("\n")) s = s.slice(0, -1);
  if (s.endsWith("\r")) s = s.slice(0, -1);
  if (!s.startsWith(STAGE_PREFIX)) return null;
  return sanitizeDeptStageKey(s.slice(STAGE_PREFIX.length));
}

/** 이 탭의 진행 id — `dp-<탭 번호>`. 탭 번호는 호출마다 새로 나오므로 호출마다 다르다(Tauri 이벤트가 이 id 로 자기 대기 탭을 찾는다). */
export function deptProgressId(wsId: number): string {
  return PROGRESS_ID_PREFIX + String(wsId);
}

/**
 * 'dept-create-progress' 이벤트 payload `{ id, stage }` 를 거른다 — 객체가 아니거나 id 가 문자열(1~64자)이 아니거나
 * 단계 키가 규칙(sanitizeDeptStageKey)에 안 맞으면 null. 이벤트는 신뢰하지 않는다.
 */
export function parseDeptProgressPayload(p: unknown): { id: string; stage: string } | null {
  if (typeof p !== "object" || p === null || Array.isArray(p)) return null;
  const o = p as Record<string, unknown>;
  const id = o.id;
  if (typeof id !== "string" || id.length === 0 || id.length > PROGRESS_ID_MAX) return null;
  const stage = sanitizeDeptStageKey(o.stage);
  return stage === null ? null : { id, stage };
}

/** 팀원 부팅 안내 토스트 id — `dept-formation:<소켓>`. 같은 탭은 같은 id 라 갱신된다. */
export function deptFormationToastId(socket: string): string {
  return DEPT_FORMATION_TOAST_PREFIX + socket;
}

/**
 * 좌석 목록(`list_surfaces` 응답의 `surfaces`)에서 **종료하지 않은** 좌석의 역할만 뽑는다 — 팀원 부팅 안내의 완료 판정 입력(성찰 1회차 S4 B1).
 * 3초 틱(refreshPaneTitles)이 소켓마다 이미 받는 목록을 그대로 쓴다(새 RPC·새 타이머 없음). 데몬 IPC 데이터라 의심한다:
 *  · 배열이 아니면 null — '목록을 받지 못함'과 같다(호출측은 그 틱의 판정을 건너뛴다 · 빈 배열 = 데몬이 성공적으로 0개를 돌려준 것과 다르다).
 *  · 원소가 객체가 아니거나 `exited` 가 참이면 건너뛴다(종료한 좌석은 붙은 자리가 아니다 — 같은 데이터를 읽는 입양 루프의 규칙 `s.exited ? 종료 : 살아 있음` 그대로).
 *  · 역할이 문자열이 아니거나 비면 건너뛴다(역할 없는 셸).
 */
export function deptLiveRoles(surfaces: unknown): string[] | null {
  if (!Array.isArray(surfaces)) return null;
  const out: string[] = [];
  for (const s of surfaces) {
    if (typeof s !== "object" || s === null) continue;
    const o = s as Record<string, unknown>;
    if (o.exited) continue;
    if (typeof o.role === "string" && o.role !== "") out.push(o.role);
  }
  return out;
}

/**
 * 의무 역할(DEPT_SEAT_ROLES — master·cso·worker·reviewer-gemini·reviewer-codex) 가운데 붙어 있는 **서로 다른** 역할의 수(0~5).
 * 이름이 정확히 같은 것만 센다 — 변형(`worker-2`·`cso-1`)·일회용(`cso-fresh-<epoch>`)·대소문자·공백·접두만 같은 이름은 의무 자리가 아니다. 배열이 아니면 0.
 */
export function deptSeatedCount(roles: unknown): number {
  if (!Array.isArray(roles)) return 0;
  let n = 0;
  for (const r of DEPT_SEAT_ROLES) if (roles.indexOf(r) >= 0) n++;
  return n;
}

/** 한 틱의 판정 — skip(목록을 못 받음 · 판정 건너뜀) · wait(아직 — 상한 전) · seated(의무 역할 다섯이 모두 붙음) · check(상한에 닿았는데 다 안 붙음). */
export type DeptFormationVerdict = "skip" | "wait" | "seated" | "check";

/**
 * 한 틱의 판정 = 그 팀 소켓의 좌석 목록에서 읽은 역할(deptLiveRoles 의 값)과 경과(초).
 *  · roles 가 배열이 아니면(null·undefined — 이번 틱에 그 소켓의 목록을 못 받았다) **skip** — 완료로도 '확인 필요'로도 치지 않는다.
 *  · 의무 역할 다섯이 모두 붙었으면 **seated**(경과와 무관 — 상한 틱에도 이쪽이 먼저다).
 *  · 아니면 상한(DEPT_FORMATION_CAP_SECS = 15분)에 닿았는지로 check 또는 wait. seated 필드는 붙은 의무 역할 수(check 문구의 N).
 */
export function deptFormationVerdict(roles: unknown, elapsedSec: number): { verdict: DeptFormationVerdict; seated: number } {
  if (!Array.isArray(roles)) return { verdict: "skip", seated: 0 };
  const seated = deptSeatedCount(roles);
  if (seated >= DEPT_SEAT_ROLES.length) return { verdict: "seated", seated };
  return { verdict: deptFormationCapped(elapsedSec) ? "check" : "wait", seated };
}

/**
 * 팀원 부팅 안내의 제목·본문(경과는 분 단위 — formatDeptMinutes).
 *  · booting(기본): 「팀원을 켜는 중」 — 켜지는 순서·보통 시간(5분 안팎 — 이 맥 실측 1회 약 5분)·지금 자리 수(탭의 칸 수)·경과.
 *  · seated: 「팀 자리가 모두 붙었습니다」 — 의무 역할 다섯이 모두 붙은 것까지만 말한다('준비 완료'라고 단정하지 않는다 — 에이전트가 실제로 떴는지는 화면이 모른다) + 걸린 시간.
 *  · check: 「팀원 켜기 — 확인 필요」 — 15분 상한까지 다섯이 다 안 붙었다. 지금 붙은 의무 역할 수(seats) + 확인할 곳(Control Center) + 경과.
 * seats·elapsedSec 는 음수·NaN 이면 0, 소수는 내림. 모르는 상태 값은 booting 으로 접는다(던지지 않는다).
 */
export function deptFormationText(o: { seats: number; elapsedSec: number; state?: DeptFormationState }): { title: string; body: string } {
  const seats = normSeats(o.seats);
  const sec = normSecs(o.elapsedSec);
  switch (o.state) {
    case "seated":
      return { title: "팀 자리가 모두 붙었습니다", body: `자리 ${DEPT_SEAT_ROLES.length}개가 모두 붙었습니다 · 걸린 시간 ${formatDeptMinutes(sec)}` };
    case "check":
      return {
        title: "팀원 켜기 — 확인 필요",
        body: `아직 ${seats}자리입니다 — Control Center 에서 자리 상태를 확인하세요 · 경과 ${formatDeptMinutes(sec)}`,
      };
    default:
      return {
        title: "팀원을 켜는 중",
        body: `부서장·CSO·워커·리뷰어가 차례로 켜집니다(보통 5분 안팎) · 지금 ${seats}자리 · 경과 ${formatDeptMinutes(sec)}`,
      };
  }
}

/**
 * 팀원 부팅 안내를 **다시 내야 하는지** 가르는 열쇠 — 열쇠가 바뀔 때만 안내를 갱신한다(토스트를 낼 때마다 알람 이력이 돌므로 초 단위로 부르지 않는다).
 * 열쇠 = 상태 · 자리 수 · 경과 '분' · 수명 갱신 칸(DEPT_FORMATION_REFRESH_SECS). 자리 수가 바뀌거나 분이 바뀌거나 45초 칸이 바뀌면 달라진다.
 * (성찰 1회차 S4 m3: 토스트가 수명으로 사라졌으면 열쇠가 같아도 main.ts 가 다시 낸다 — 이 열쇠는 '살아 있는 안내를 언제 새로 고칠까'만 정한다.)
 */
export function deptFormationNoticeKey(o: { seats: number; elapsedSec: number; state?: DeptFormationState }): string {
  const s = normSecs(o.elapsedSec);
  return `${o.state ?? "booting"}|${normSeats(o.seats)}|${Math.floor(s / 60)}|${Math.floor(s / DEPT_FORMATION_REFRESH_SECS)}`;
}

/** 팀원 부팅 안내가 상한에 닿았는가 — 경과가 상한(DEPT_FORMATION_CAP_SECS = 15분) 이상이다(닿으면 주기 갱신을 멈추고, 자리 판정이 '확인 필요'를 한 번 알린다). */
export function deptFormationCapped(elapsedSec: number): boolean {
  return normSecs(elapsedSec) >= DEPT_FORMATION_CAP_SECS;
}

/**
 * 방금 만든 팀의 빈 탭이 '첫 자리를 붙이는 중' 이라고 말해도 되는가 — 이 세션에서 새로 만든 탭(createdAt 이 있는 숫자)이고
 * 만든 지 0 이상 60초(DEPT_FIRST_SEAT_WINDOW_SECS) **미만**일 때만 true. 시계가 거꾸로 간 경우(음수)·createdAt 이 없는 경우는 false(종전 문구).
 */
export function deptFirstSeatPending(createdAt: number | undefined, nowMs: number): boolean {
  if (typeof createdAt !== "number" || !isFinite(createdAt) || typeof nowMs !== "number" || !isFinite(nowMs)) return false;
  const age = nowMs - createdAt;
  return age >= 0 && age < DEPT_FIRST_SEAT_WINDOW_SECS * 1000;
}

/** 위 창이 끝나기까지 남은 ms(0 이상) — 창이 끝나는 순간 빈 탭 문구를 한 번 다시 고치는 데 쓴다. createdAt 이 없으면 0. */
export function deptFirstSeatRemainingMs(createdAt: number | undefined, nowMs: number): number {
  if (typeof createdAt !== "number" || !isFinite(createdAt) || typeof nowMs !== "number" || !isFinite(nowMs)) return 0;
  return Math.max(0, DEPT_FIRST_SEAT_WINDOW_SECS * 1000 - (nowMs - createdAt));
}
