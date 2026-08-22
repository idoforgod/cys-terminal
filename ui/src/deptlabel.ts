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
