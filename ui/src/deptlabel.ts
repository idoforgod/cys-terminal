// 부서 런칭 중(pending) 탭 라벨의 순수 계산 — main.ts의 buildTab이 이 함수에 배선만 한다(스피너 글리프·DOM은 호출측).
//
// WP-10: '＋부서' 클릭 후 부서 데몬 준비(~12초) 동안 라벨이 "…"만 보이면 사용자가 '멈춘 줄'로 오해한다.
// pending 탭엔 "부서 제작 중…"을 표시해 진행 중임을 명시한다. 순수 함수라 pending/확정 라벨을
// 결정론으로 회귀 테스트할 수 있다(deptlabel.test.ts).

import { sameSocket } from "./wsreconcile";

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

// ────────────────────────────────────────────────────────────────────────────
// 부서 socket 경로 → **원래 부서명** 역산 (main.ts 에서 이관 · 2026-09-16).
//
// ★왜 옮겼나: 같은 규약의 파서가 main.ts·여기·Rust 세 벌로 갈려 있었고, main.ts 안의
// 모듈-private 함수라 **유닛 테스트가 닿지 못했다** — Windows named pipe 분기를 지키는 핀이
// 0개인 채로 'Windows 대응 완료'라고 적혀 있었다(deptSlugOfSocket 이 정확히 그 상태에서 깨졌던
// 전례가 이 파일 위쪽에 있다). 테스트가 **제품 함수 그 자체**를 부르게 하려고 여기로 옮긴다.
//
//   unix : `…/cys-dept-<name>/cys.sock`  → <name>
//   win  : `\\.\pipe\cys-dept-<name>`    → <name>
// 둘 다 아니면 null(= 부서 소켓이 아니다).
// ────────────────────────────────────────────────────────────────────────────
export function deptNameFromSocket(sock: string | undefined): string | null {
  const m = /\/cys-dept-(.+?)\/cys\.sock$/.exec(sock ?? "");
  if (m) return m[1];
  const w = /^\\\\\.\\pipe\\cys-dept-(.+)$/.exec(sock ?? "");
  return w ? w[1] : null;
}

// ────────────────────────────────────────────────────────────────────────────
// ★K2-05(2026-09-17 한글 사용자명 감사) — `cys-dept launch <name>` 에 넘길 **부서명 인자**의 단일 산출 지점.
//
// 종전 main.ts 두 곳('지금 켜기'·복원 재-launch)은 `deptNameFromSocket(ws.socket) ?? ws.name` 이었다.
// `ws.name` 은 create 표시명(한글 · main.ts addDeptWorkspace 의 display_name) 또는 사용자가 rename 한 탭
// 이름이라, 소켓 역산이 null 이면 그 한글이 부서명 인자로 흘러 cys-dept `validate_dept_name` 에서 exit 2
// ("부적격 부서명('영업부')") — 화면에는 "부서를 켜지 못했습니다" 만 남았다. 표시명은 라벨 레이어이지
// 정체가 아니다. 정체의 진실원은 소켓, 그 다음이 레지스트리(list_depts 의 **키** = cys-dept 가 검증해 등재한
// 부서명)다. 둘 다 실패하면 null 을 돌려주고 호출측이 사유를 말한다 — 이 함수는 표시명을 아예 받지 않으므로
// 표시명이 인자가 되는 경로는 타입 수준에서 없다.
//
//   ① depts 의 항목 중 sameSocket(e.socket, socket) 인 **등재 키** — cys-dept 가 검증해 기록한 이름 그 자체
//      (표기만 다른 같은 소켓 — Windows named pipe 대소문자 — 도 같은 항목으로 받는다)
//   ② deptNameFromSocket(socket) — 레지스트리를 못 읽었거나(미조회 · null) 일치 항목이 없을 때의 폴백
//      (unix `…/cys-dept-<n>/cys.sock` · win `\\.\pipe\cys-dept-<n>`)
//   ③ null
//
// ★D3(2026-09-17 4라운드 · codex 2차 ⑥): 순서를 **등재 키 우선**으로 뒤집었다. 종전은 파서 우선이라
//   등재 `dept-2` · 저장 탭 소켓 `\\.\pipe\cys-dept-DEPT-2`(named pipe 는 대소문자 무구분 = 같은 파이프)에서
//   파서가 `DEPT-2` 를 돌려줬다 — `cys-dept launch DEPT-2` 는 slug 충돌(exit 2)로 **매 기동 반복 실패**하고,
//   묘비 `dept-2` 도 `DEPT-2` 와 비교돼 놓쳤다. 등재 키는 정본이므로 파서보다 앞선다. unix 경로는 sameSocket 이
//   바이트 일치라 종전과 같은 답이고, 파서는 레지스트리 미조회(list_depts Err → null)·미등재에서만 쓴다.
// ────────────────────────────────────────────────────────────────────────────
export function deptLaunchName(
  socket: string | undefined,
  depts: Readonly<Record<string, { socket?: string } | undefined>> | null | undefined,
): string | null {
  if (!socket) return null;
  for (const [name, e] of Object.entries(depts ?? {})) {
    if (name && e?.socket && sameSocket(e.socket, socket)) return name;
  }
  return deptNameFromSocket(socket);
}

// ────────────────────────────────────────────────────────────────────────────
// ★C1(2026-09-17 3라운드 · codex major "묘비 검사가 launch 와 다른 해소기를 쓴다") — 복원 경로의
// **부서 탭 1개에 대한 launch 판정**을 한 곳에서 낸다. main.ts 복원 루프는 이 판정에 배선만 한다.
//
// 무엇이 깨져 있었나: 2라운드가 launch 인자를 `deptLaunchName`(소켓 → 레지스트리 키)으로 넓혔는데,
// 그 직전의 묘비 검사는 여전히 `deptNameFromSocket`(소켓 규약 파서)만 썼다. 두 해소기의 **차집합**
// (파서는 null · 레지스트리 키는 있음)이 묘비를 우회한다 —
//   등재 소켓 `\\.\pipe\cys-dept-dept-2` · 저장 탭 소켓 `\\.\PIPE\cys-dept-dept-2`(대문자) · 묘비 dept-2
//   → 파서 null → 묘비 검사 통과 → launch 가 dept-2 를 기동 → cys-dept launch 말미가 **묘비를 지운다**
//   (= 사용자가 지운 부서가 되살아나고 그 LLM 팀까지 다시 뜬다). 레거시 파일경로형 소켓도 같은 구멍이다.
// 수리: 묘비 검사와 launch 가 **같은 이름**을 본다 — 이름을 먼저 한 번 구하고, 그 이름으로 묘비를 본다.
//
// 반환:
//   { launch: null, reason: "tombstone" }         — 삭제 의도 부서(호출측: 탭 드롭 + 잔존 데몬 정리 시도)
//   { launch: null, reason: "unnamed" }           — 소켓·레지스트리 어디서도 이름을 못 구함(호출측: 탭 보존 + 사유 토스트)
//   { launch: null, reason: "tombstone-unknown" } — 묘비 조회 실패(null/undefined): 이름은 있으나 **자동 launch 보류**
//                                                   (호출측: 탭 보존 · 살아 있는 pane 은 그대로 · 수동 '지금 켜기' 가능 · 토스트 1회)
//   { launch: name, reason: "launch" }            — 이 이름으로 `cys-dept launch` 가능(생존·등재 검사는 호출측이 이어서 한다)
//
// ★D2(2026-09-17 4라운드 · codex 2차 ⑤): 묘비 결측(tombstones === null)은 **fail-closed** 다. 종전은 '묘비 없음'으로
//   보고 켰는데(fail-open), 그 launch 는 성공 말미에 묘비를 지우므로(cysjavis-pack/bin/cys-dept) 삭제 묘비 기록 직후
//   저장 전에 앱이 죽어 옛 탭이 남고 다음 기동에서 묘비 RPC 가 timeout 이면 **지운 부서가 되살아나고 팀까지 다시 뜬다**
//   (비가역). Rust spawn_org_restore 는 같은 상황에서 죽은 부서 재기동을 보류한다 — 두 리바이버가 반대 방향이면 약한
//   쪽이 계약을 무효화하므로 여기도 같은 방향으로 맞춘다. 대가는 '그 기동에 죽은 부서 탭이 idle 로 남는다' 뿐이고
//   수동 '지금 켜기'(묘비를 보지 않는다)와 다음 기동이 회복한다. 살아 있는 부서는 호출측이 생존 검사로 먼저 걸러
//   이 판정의 launch 를 쓰지 않으므로 pane 은 보존된다(main.ts 복원 루프 · wswiring 핀).
//   3라운드가 fail-open 을 지킨 이유("구형 데몬 스큐에서 전 부서 영구 idle")는 '지금 켜기'가 있어 성립하지 않는다.
// ────────────────────────────────────────────────────────────────────────────
export type RestoreLaunchReason = "launch" | "tombstone" | "tombstone-unknown" | "unnamed";

export interface RestoreLaunchDecision {
  /** `cys-dept launch` 에 넘길 부서명. null 이면 켜지 않는다(reason 이 사유). */
  launch: string | null;
  reason: RestoreLaunchReason;
}

// ────────────────────────────────────────────────────────────────────────────
// ★H1(2026-09-17 12라운드 · codex 6차 major) — 묘비 대조는 **정확 일치 2회** 다(fold 폐기).
//
// 10라운드는 묘비 대조를 팩의 `dept_name_fold`(소문자화 + '.' 제거)로 넓혔다. 그것이 만든 회귀:
//   `launch Sales` → `down Sales`(묘비 `Sales` 기록) → `launch sales`(팩의 slug 충돌 검사는 **현재 등재만**
//   보므로 통과) → cysd 의 묘비 해제는 **정확 일치** `dt.remove(&name)` 라 `Sales` 묘비가 그대로 남는다
//   (src/bin/cysd/handlers.rs:5102). 다음 기동에서 fold 판독이 살아 있는 `sales` 를 `Sales` 묘비와 같다고
//   보고 **탭을 지우고 종료를 요청**했다 — 정상 재생성한 부서를 자동으로 죽인 것이다.
// 즉 fold 는 '같은 부서' 가 아니라 '팩이 동시 등재를 거부하는 축' 일 뿐이고, 묘비는 시간축으로 남으므로
// 그 축을 묘비 대조에 쓰면 **다른 시점의 다른 부서**까지 삼킨다. 그래서 fold 는 걷어낸다.
//
// 그 대신 5차 리뷰가 잡았던 원 반례(별칭 묘비 미검출)는 **축이 아니라 후보를 늘려** 막는다. 한 탭에는
// 그 부서를 가리키는 식별자가 둘 있다:
//   (i)  해소된 이름  = `deptLaunchName(socket, regDepts)` — 등재 키 우선(= launch 인자 · 지금의 기록자가 남기는 이름)
//   (ii) 소켓 파생 이름 = `deptNameFromSocket(socket)`     — 옛 기록자(main.rs 의 소켓 파생 폴백)가 남겼을 이름
// 둘 **각각을 정확 일치**로 검사하고, 어느 하나라도 묘비에 있으면 켜지 않는다.
//
// 왜 이것이 과잉 차단이 아닌가: (ii) 는 이 탭의 소켓 문자열에서만 나온다. 부서 소켓은 부서마다 유일하므로
// (unix `…/cys-dept-<n>/cys.sock` · win `\\.\pipe\cys-dept-<n>`) **서로 다른 부서가 같은 소켓 파생 이름을
// 가질 수 없다**. 따라서 (ii) 가 묘비에 있다는 것은 '같은 부서의 옛 기록자 식별자가 묘비에 있다' 는 뜻이다.
// 표기만 다른 별칭(`DEPT-2` vs `dept-2`)은 이 두 후보로 덮이고, 위 `Sales`/`sales` 회귀는 두 후보 모두
// 정확 일치에 걸리지 않으므로 그대로 켜진다.
// (기록자 쪽은 10라운드대로 **해소된 이름**을 남긴다 — main.ts 탭 닫기·그룹 삭제 → `dept_tombstone.set`.)
// ────────────────────────────────────────────────────────────────────────────

/** 이 탭의 두 식별자 중 하나라도 묘비에 **정확 일치**로 있는가. */
function tombstoneHasDept(
  tombs: ReadonlySet<string>,
  name: string | null,
  socketName: string | null,
): boolean {
  return (!!name && tombs.has(name)) || (!!socketName && tombs.has(socketName));
}

export function restoreLaunchDecision(args: {
  socket: string | undefined;
  regDepts: Readonly<Record<string, { socket?: string } | undefined>> | null | undefined;
  tombstones: ReadonlySet<string> | null | undefined;
}): RestoreLaunchDecision {
  const name = deptLaunchName(args.socket, args.regDepts);
  // 소켓 파생 이름은 `name` 과 갈릴 수 있다(등재 키 `dept-2` · 저장 소켓 `\\.\pipe\cys-dept-DEPT-2`).
  // 옛 기록자가 남긴 묘비는 이쪽 표기이므로, 두 후보를 각각 본다(위 H1 주석).
  const socketName = deptNameFromSocket(args.socket);
  if (args.tombstones && tombstoneHasDept(args.tombstones, name, socketName)) {
    return { launch: null, reason: "tombstone" };
  }
  if (!name) return { launch: null, reason: "unnamed" };
  // ★D2 fail-closed: 결측은 값이 아니다 — 빈 집합(조회 성공 · 묘비 0)만 '묘비 없음'이다.
  if (args.tombstones == null) return { launch: null, reason: "tombstone-unknown" };
  return { launch: name, reason: "launch" };
}

// ────────────────────────────────────────────────────────────────────────────
// ★E3(2026-09-17 6라운드 · codex 3차 ④) — 수동 '지금 켜기' 의 **부서명 해소 1지점**.
//
// 무엇이 깨져 있었나: 클릭 핸들러가 `deptLaunchName(ws.socket, null)`(= 레지스트리 없이 파서만)을
// **먼저** 부르고, 그것이 이름을 내면 레지스트리를 아예 조회하지 않았다. deptLaunchName 의 계약은
// '등재 키 우선 · 파서는 폴백' 인데, 인자를 null 로 주면 그 우선순위가 호출부에서 무효화된다 —
// 등재 `dept-2` · 저장 소켓 `\\.\pipe\cys-dept-DEPT-2`(named pipe 는 대소문자 무구분 = 같은 파이프)에서
// 파서가 `DEPT-2` 를 돌려주고 `cys-dept launch DEPT-2` 는 slug 충돌(exit 2)로 **매 클릭 실패**했다.
// 자동 복원 경로(restoreLaunchDecision)는 이미 레지스트리를 넘기므로 수동 경로만 규칙 밖이었다.
//
// 수리: 조회를 **먼저 한 번** 한다(등재 키 우선). 조회가 실패하면(=null) 그때만 파서 폴백이다.
// 로더는 주입받는다 — 이 판정이 Tauri·DOM 없이 실행 가능해야 '조회 0회' 변이가 검체에서 red 가 된다
// (main.ts 는 `invoke("list_depts")` 배선만).
// ────────────────────────────────────────────────────────────────────────────
export interface DeptLaunchResolution {
  /** `cys-dept launch` 에 넘길 이름. null 이면 켜지 않는다. */
  name: string | null;
  /** 레지스트리 조회가 성공했는가. false = list_depts 실패(판독 오류·타임아웃) → 파서 폴백만 썼다(미등재가 아니다). */
  regQueried: boolean;
}

export async function resolveDeptLaunchName(
  socket: string | undefined,
  loadDepts: () => Promise<Readonly<Record<string, { socket?: string } | undefined>> | null | undefined>,
): Promise<DeptLaunchResolution> {
  if (!socket) return { name: null, regQueried: false };
  let reg: Readonly<Record<string, { socket?: string } | undefined>> | null = null;
  try {
    reg = (await loadDepts()) ?? null;
  } catch {
    reg = null; // 조회 실패 시에만 파서 폴백 — 실패를 빈 레지스트리로 접으면 사유가 '미등재'로 둔갑한다
  }
  return { name: deptLaunchName(socket, reg), regQueried: reg !== null };
}

// ────────────────────────────────────────────────────────────────────────────
// ★F3(2026-09-17 8라운드 · 단순화) — 부서 삭제에서 **탭은 종료가 확인된 뒤에** 지운다.
//
// 무엇이 깨져 있었나: 삭제 흐름이 ①묘비 기록 → ②탭 splice·저장 → ③`cys-dept down` 순서였다.
// ③이 rc 10(depts.json 판독 실패 — kill 0 · 등재 제거 0)으로 끝나면 데몬은 살아 있는데 탭만 없다.
// 묘비가 탭 재생성을 막으므로(missingKnownWorkspaces) 그 데몬을 다시 볼 주체가 하나도 없다 — 영구 잔존.
// 6라운드는 이 구멍을 '레지스트리 ∩ 묘비' 를 훑는 별도 reaper 로 메웠는데, 그 열거원은 탭이 없는
// (= GUI 가 모르는) 부서까지 매 기동 종료 대상으로 삼아 새 결함을 만들었다(codex 4차 major 1·2).
//
// 수리는 상태를 더하는 것이 아니라 **순서를 바꾸는 것**이다: 종료가 성공해야 탭을 지운다.
// 실패하면 탭이 남고, 탭이 남으면 다음 기동의 **탭 기반** 복원 정리 루프(묘비 판정 → down 재시도)가
// 같은 부서를 다시 본다. 재시도의 손잡이가 탭 자체이므로 별도 reaper 도 별도 상태기계도 필요 없다.
// ────────────────────────────────────────────────────────────────────────────

/** 종료 시도의 결과. "not-needed" = 부서가 아니거나 같은 소켓을 쓰는 다른 탭이 남아 있다(종료하지 않는다). */
export type DeptStopResult = "ok" | "failed" | "not-needed";

export interface DeptCloseDecision {
  /** 탭을 목록에서 제거하는가. ★false 면 탭이 재시도 손잡이다 — 지우면 잔존 데몬을 볼 주체가 사라진다. */
  removeTab: boolean;
  /** 실패 안내 본문(성공·불요면 null). 호출측이 토스트로 낸다(그룹 삭제는 모아서 1회). */
  detail: string | null;
}

export function deptCloseAfterStop(a: {
  stop: DeptStopResult;
  /** 이번 삭제에서 묘비(삭제 의도)를 기록했는가 — 뒷문장이 사실이 되려면 갈라야 한다. */
  tombRecorded: boolean;
  error?: string;
}): DeptCloseDecision {
  if (a.stop !== "failed") return { removeTab: true, detail: null };
  const why = (a.error ?? "").slice(0, 300);
  return {
    removeTab: false,
    detail:
      `부서 데몬 종료 실패 — ${why}. 탭을 다시 닫아 재시도하세요` +
      (a.tombRecorded
        ? "(삭제 의도는 기록돼 있어 재시작해도 부서가 되살아나지는 않습니다 · 탭은 잔존 데몬을 다시 종료하기 위한 손잡이입니다)."
        : "(삭제 의도 기록도 실패했습니다 — 레지스트리를 복구한 뒤 다시 삭제해 주세요)."),
  };
}

// ────────────────────────────────────────────────────────────────────────────
// ★I1(2026-09-17 13라운드 · opus 11R major) — 잔존 데몬 **회수 절차의 식별자**는 실행 가능해야 한다.
//
// 무엇이 깨져 있었나: G1 이 복원 루프의 묘비 정리를 fire-and-forget 으로 되돌리면서 "탭은 즉시 지우고,
// 실패하면 토스트가 CLI 절차를 준다"를 그 설계의 **유일한 회수 손잡이**로 삼았다. 그런데 그 토스트가
// 부서를 가리키던 이름이 `ws.name` = **표시명**이었다. 복원이 만드는 탭 이름은 `display_name ?? dname`
// 이라 한글 표시명(`영업부(한국)`)이 그대로 들어오고, 사용자가 안내대로 `cys-dept down 영업부(한국)` 을
// 치면 `validate_dept_name`(`^[A-Za-z0-9][A-Za-z0-9_-]*$` · cysjavis-pack/bin/cys-dept)이 **exit 2 로
// 거부**한다 — 이 패치 묶음이 존재하는 이유인 K2-05(표시명 ≠ 부서명)가 회수 문구에서 되살아난 것이다.
//
// 수리: 문구 생성기는 **표시명을 아예 받지 않는다**(deptLaunchName 과 같은 원칙 — 표시명이 인자가 되는
// 경로가 타입 수준에서 없다). 호출측은 그 분기에서 이미 손에 있는 **해소된 이름**
// (`deptLaunchName(ws.socket, regDepts)` — 판정을 만든 바로 그 입력)을 넘기고, 못 구하면 null 을 넘겨
// `cys-dept list` 로 이름을 먼저 확인하는 절차로 갈라진다(실행 불가능한 명령을 주느니 절차를 준다).
// ────────────────────────────────────────────────────────────────────────────
export interface TombReapErrorToast {
  title: string;
  body: string;
}

export function tombReapErrorToast(a: {
  /** 이번 기동에서 정리에 실패한 묘비 부서 누계(토스트는 sticky id 하나를 갱신한다). */
  count: number;
  /** 해소된 부서명 = `cys-dept down` 에 그대로 넣을 수 있는 인자. null = 해소 실패(표시명으로 대체하지 않는다). */
  deptName: string | null;
  /** 가장 최근 실패 사유(원문 — 자르는 것은 여기서 한다). */
  error: string;
}): TombReapErrorToast {
  const how = a.deptName
    ? `다음 명령으로 정리해 주세요: \`cys-dept down ${a.deptName}\``
    : "다음 절차로 정리해 주세요: `cys-dept list` 로 부서명을 확인한 뒤 `cys-dept down <부서명>`";
  return {
    title: `삭제된 부서 ${a.count}곳의 종료가 실패했습니다`,
    body:
      "탭은 이미 지웠습니다(삭제 의도는 묘비에 남아 있어 부활하지 않습니다). " +
      `잔존 데몬은 레지스트리 복구 후 ${how}. ` +
      `가장 최근 사유 — ${a.deptName ?? "(부서명 미상)"}: ${String(a.error).slice(0, 300)}`,
  };
}
