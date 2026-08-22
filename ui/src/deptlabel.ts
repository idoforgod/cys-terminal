// 부서 런칭 중(pending) 탭 라벨의 순수 계산 — main.ts의 buildTab이 이 함수에 배선만 한다(스피너 글리프·DOM은 호출측).
//
// WP-10: '＋부서' 클릭 후 부서 데몬 준비(~12초) 동안 라벨이 "…"만 보이면 사용자가 '멈춘 줄'로 오해한다.
// pending 탭엔 "부서 제작 중…"을 표시해 진행 중임을 명시한다. 순수 함수라 pending/확정 라벨을
// 결정론으로 회귀 테스트할 수 있다(deptlabel.test.ts).

// pending 부서 탭에 표시할 진행 라벨(스피너 글리프는 CSS가 담당 — 여기선 텍스트만).
export const DEPT_PENDING_LABEL = "부서 제작 중…";

// pending이면 진행 라벨, 확정되면 실제 부서 표시명.
export function deptPlaceholderLabel(ws: { pending?: boolean; name: string }): string {
  return ws.pending ? DEPT_PENDING_LABEL : ws.name;
}

// ★결함#4-b/F5(적대검증 2026-08-22) — 부서 소켓 경로에서 **사람이 읽는 슬러그**를 뽑는다.
// 승인 Feed 의 부서 행 부제(fi-meta)가 쓴다: 전체 경로는 길어 줄을 넘기므로(.fi-meta 에
// word-break 없음) 슬러그만 보이고 전체 경로는 title 로 남긴다.
//
//   unix : `…/cys-dept-sales/cysd.sock` → 부모 디렉터리 `cys-dept-sales`
//   win  : `\\.\pipe\cys-dept-sales`    → **마지막** 컴포넌트(파이프 이름 자체가 슬러그)
//
// 무엇이 깨졌었나: 초판은 win/unix 둘 다 받는다고 주석에 계약해 놓고 `slice(-2,-1)` 하나로
// 처리했다. named pipe 는 구분자 분해 후 `[".", "pipe", "cys-dept-sales"]` 라 -2 가 **항상
// `"pipe"`** — Windows 에서 모든 부서 행의 부제가 동일해져 식별이 죽었다(주석=계약 위반).
export function deptSlugOfSocket(socket: string): string {
  const parts = socket.split(/[\\/]/).filter(Boolean);
  // `\\.\pipe\NAME` / `\\?\pipe\NAME` — 접두로 판정하고, 폴백으로 컴포넌트도 본다
  // (경로가 정규화돼 접두가 달라져도 'pipe 바로 뒤가 이름'이라는 사실은 유지된다).
  const isPipe =
    /^\\\\[.?]\\pipe\\/i.test(socket) || parts[parts.length - 2]?.toLowerCase() === "pipe";
  return (isPipe ? parts[parts.length - 1] : parts[parts.length - 2]) ?? socket;
}

// ────────────────────────────────────────────────────────────────────────────
// ★F6①②(적대검증 2026-08-22) 승인 Feed 부서 행의 **이동 버튼 판정** — 순수부.
//
// 왜 여기로 옮겼나(2026-08-22 잔여 공백 마감): 판정 자체는 이미 옳았지만 `main.ts` 안의
// 모듈-private 함수라 **유닛 테스트가 닿지 못했다** — 구현은 정상인데 핀이 없어 green 이
// 무증거인 상태였다. 같은 라운드에서 억제 스캔(F4-③)이 정확히 그 상태로 649건 green 인 채
// 결함이 되살아나는 것을 부정 대조로 확인했으므로, 경미 등급이라고 예외를 두지 않는다.
// DOM·전환 부작용(activeWs 대입·render·setFocus)은 `main.ts` 에 그대로 남는다.
// ────────────────────────────────────────────────────────────────────────────

// `Workspace.socket === undefined`(기본 데몬)의 맵 키. 부서 소켓 비교의 단일 정규화 지점 —
// main.ts 의 대기수 맵 키(ccPendingBySocket)와 같은 값을 써야 행이 서로 어긋나지 않는다.
export const DEFAULT_SOCKET_KEY = "";

// 판정에 필요한 최소 필드만 받는다(트리·이름·그룹은 무관 — 좁은 입력이 테스트를 싸게 만든다).
export interface DeptWsRef {
  socket?: string;
  pending?: boolean;
}

//   "switched" = 그 워크스페이스가 활성이다(**이미 활성이었던 경우 포함** — 결과 상태가 같다)
//   "pending"  = 부서 데몬 기동 중인 placeholder 로 전환했다(★F6② — '닫힌 탭'이 **아니다**)
//   "missing"  = 그 socket 의 탭이 실제로 없다(레지스트리 잔재)
export type DeptSwitchOutcome = "switched" | "pending" | "missing";

export interface DeptWsPick {
  outcome: DeptSwitchOutcome;
  index: number; // "missing" 이면 -1
}

// 소켓 비교용 정규화 키(undefined = 기본 데몬).
export function deptWsSocketKey(ws: DeptWsRef): string {
  return ws.socket ?? DEFAULT_SOCKET_KEY;
}

// 대상 socket 의 워크스페이스를 고르고 결과를 **세 갈래로 사실대로** 돌려준다 —
// 조용히 아무 일도 안 하거나 틀린 사유를 말하면 '눌러도 안 되는 버튼'이 된다.
//
// ★F6②가 고친 것: 초판은 후보를 `!w.pending` 으로만 찾아 **연결 중** 부서를 못 찾고
// "missing"(= "탭이 이미 닫혔습니다")으로 오안내했다. 실제로는 탭이 있고 기동 중일 뿐이다.
// 그래서 비-pending 을 **우선** 채택하되(정상 탭이 있으면 그쪽), 없으면 pending 도 받는다.
export function pickDeptWorkspace(list: readonly DeptWsRef[], socket: string): DeptWsPick {
  const ready = list.findIndex((w) => !w.pending && deptWsSocketKey(w) === socket);
  if (ready >= 0) return { outcome: "switched", index: ready };
  const pending = list.findIndex((w) => deptWsSocketKey(w) === socket);
  return pending < 0 ? { outcome: "missing", index: -1 } : { outcome: "pending", index: pending };
}

// ★F6① 버튼 문구의 근거 — 이미 그 워크스페이스면 '이동'이 아니다("지금 이 부서 — 패널 닫기").
// 라벨이 실제 동작과 어긋나면 사용자는 눌러 보고 나서야 안다.
export function isActiveDeptSocket(
  list: readonly DeptWsRef[],
  activeIndex: number,
  socket: string,
): boolean {
  const ws = list[activeIndex];
  return ws ? deptWsSocketKey(ws) === socket : false;
}
