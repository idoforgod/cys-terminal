// wsreconcile.ts 회귀 핀 (bun test — 신규 의존성 0).
//
// ★이 파일이 존재하는 이유(2026-09-16 오너 실사고):
// 재부팅하면 본부(기본 데몬) 워크스페이스만 화면에서 사라지고, 앱을 다시 켜도 돌아오지 않았다.
// 원인은 복원 필터의 한 줄(`ws.socket != null && lb?.ok === true`)이 본부만 차별한 것이었고,
// **그 판정을 고정하는 테스트가 하나도 없어서 3개월간 살아남았다.** 그래서 판정을 순수 함수로
// 빼고 여기에 못을 박는다. 아래 각 테스트는 "이게 깨지면 화면에서 무엇이 사라지는가"를 이름에 적는다.
import { describe, it, expect } from "bun:test";
import {
  keepWorkspaceOnRestore,
  missingKnownWorkspaces,
  ghostSids,
  deadLiveSids,
  advanceGhostStrikes,
  sameSocket,
  scaleForPlatform,
  type ReconcileWs,
  type LiveProbe,
} from "./wsreconcile";
import { deptNameFromSocket } from "./deptlabel";

// ★개인 경로를 박지 않는다 — 시크릿/PII 스캐너가 `/Users/<name>` 류를 발행 차단으로 잡는다
// (scripts/secret-scan.sh PATH·PROFILE 규칙). 판정에 필요한 것은 `…/cys-dept-<name>/cys.sock`
// 라는 **형태**뿐이므로 중립 접두를 쓴다.
const STATE = "/state";
const DEPT = `${STATE}/cys-dept-dept-1/cys.sock`;
const DEPT2 = `${STATE}/cys-dept-dept-2/cys.sock`;
const WINPIPE = "\\\\.\\pipe\\cys-dept-dept-1";

// ★제품 함수를 그대로 쓴다(베껴 쓰지 않는다). 종전 이 파일은 같은 규칙을 **복사**해 뒀는데,
// 그러면 제품 파서가 바뀌어도 테스트는 계속 초록이라 'Windows 동형'이라는 이름표만 남는다.
const deptNameOf = (sock: string): string | null => deptNameFromSocket(sock);

const ok: LiveProbe = { ok: true };
const down: LiveProbe = { ok: false };
// 묘비 '조회 성공·0건'. ★null(조회 실패)과 구분하라 — 이 둘의 답은 의도적으로 다르다.
const NO_TOMBS: ReadonlySet<string> = new Set();
const base = (tree: unknown | null): ReconcileWs => ({ tree });
const dept = (tree: unknown | null, socket = DEPT): ReconcileWs => ({ tree, socket });

describe("keepWorkspaceOnRestore — 복원 시 탭 보존 판정", () => {
  it("★핵심 회귀: 본부(socket 없음) + 빈 트리 + 데몬 생존 → 보존한다", () => {
    // 깨지면: 재부팅 후 본부 5팀이 화면에서 통째로 사라진다(2026-09-16 실사고).
    expect(keepWorkspaceOnRestore(base(null), ok)).toBe(true);
  });

  it("★대칭 대조: 부서도 같은 조건에서 같은 답이다", () => {
    // 본부와 부서의 답이 갈리면 그 자체가 결함이다 — 이 쌍이 대칭을 고정한다.
    expect(keepWorkspaceOnRestore(dept(null), ok)).toBe(keepWorkspaceOnRestore(base(null), ok));
  });

  it("pane 이 살아 있으면 데몬 응답과 무관하게 보존한다", () => {
    for (const probe of [ok, down, undefined]) {
      expect(keepWorkspaceOnRestore(base({ type: "pane", sid: 1 }), probe)).toBe(true);
      expect(keepWorkspaceOnRestore(dept({ type: "pane", sid: 1 }), probe)).toBe(true);
    }
  });

  it("데몬 일시 미응답(ok:false)은 판정 보류 — 빈 트리라도 보존한다", () => {
    // 깨지면: 데몬이 잠깐 늦게 떴다는 이유로 탭이 영구 삭제된다.
    expect(keepWorkspaceOnRestore(base(null), down)).toBe(true);
    expect(keepWorkspaceOnRestore(dept(null), down)).toBe(true);
  });

  it("조회 자체가 없었던(undefined) 빈 트리는 드롭한다 — 유령 탭 증식 차단", () => {
    // ★결측은 값이 아니다: undefined(조회 안 함)와 {ok:false}(조회했으나 미응답)는 다른 상태다.
    expect(keepWorkspaceOnRestore(base(null), undefined)).toBe(false);
    expect(keepWorkspaceOnRestore(dept(null), undefined)).toBe(false);
  });
});

describe("missingKnownWorkspaces — 저장본에 없어도 만들어야 하는 탭", () => {
  it("★핵심 회귀: 저장본에 부서만 있으면 본부 탭을 만든다", () => {
    // 깨지면: 한 번 깨진 저장본이 영원히 복구되지 않는다(앱을 다시 켜도 본부가 안 나온다).
    const out = missingKnownWorkspaces([dept(null)], [], null, deptNameOf);
    expect(out).toEqual([{}]);
  });

  it("본부 탭이 이미 있으면 추가하지 않는다(중복 탭 증식 차단)", () => {
    expect(missingKnownWorkspaces([base(null), dept(null)], [], null, deptNameOf)).toEqual([]);
  });

  it("저장본이 통째로 비어도 본부 탭 1개를 만든다(localStorage 유실·프로필 초기화)", () => {
    expect(missingKnownWorkspaces([], [], null, deptNameOf)).toEqual([{}]);
  });

  it("레지스트리에 있는데 저장본에 없는 부서 탭을 만든다(GUI 밖에서 만든 부서)", () => {
    const out = missingKnownWorkspaces([base(null)], [{ socket: DEPT, label: "영업부" }], NO_TOMBS, deptNameOf);
    expect(out).toEqual([{ socket: DEPT, name: "영업부" }]);
  });

  it("표시명이 없으면 소켓에서 역산한 부서명으로 폴백한다", () => {
    const out = missingKnownWorkspaces([base(null)], [{ socket: DEPT }], NO_TOMBS, deptNameOf);
    expect(out).toEqual([{ socket: DEPT, name: "dept-1" }]);
  });

  it("Windows named pipe 부서도 같은 결과를 낸다(맥/윈도 동형)", () => {
    const out = missingKnownWorkspaces([base(null)], [{ socket: WINPIPE }], NO_TOMBS, deptNameOf);
    expect(out).toEqual([{ socket: WINPIPE, name: "dept-1" }]);
  });

  it("★Windows 파이프 부서의 묘비도 존중한다(제품이 실제로 쓰는 경로)", () => {
    // 종전 Windows 테스트는 표시명 폴백(제품 배선상 도달 불가)만 밟아 '맥/윈도 동형'이라는
    // 이름표가 붙은 채 제품 경로를 한 줄도 통과하지 않았다. 묘비 대조는 실제로 지나가는 길이다.
    const out = missingKnownWorkspaces([base(null)], [{ socket: WINPIPE }], new Set(["dept-1"]), deptNameOf);
    expect(out).toEqual([]);
  });

  it("★묘비(삭제 의도) 부서는 되살리지 않는다", () => {
    // 깨지면: 사용자가 지운 부서가 재시작마다 부활한다(WP-3 묘비 계약 위반).
    const out = missingKnownWorkspaces([base(null)], [{ socket: DEPT }], new Set(["dept-1"]), deptNameOf);
    expect(out).toEqual([]);
  });

  it("★묘비 조회 실패(null)면 부서를 하나도 만들지 않는다 — 결측은 값이 아니다", () => {
    // 실패 방향이 재앙 쪽이라 뒤집었다(2026-09-16 성찰 1회): null 을 '제약 없음'으로 읽으면
    // 묘비 RPC 한 번의 실패가 → 삭제한 부서 탭 생성 → cys-dept launch → **묘비 영구 삭제** →
    // 그 부서 LLM 팀 재기동으로 번진다. 반대 방향의 대가는 '이번 기동에 탭이 한 박자 늦는다' 뿐이다.
    expect(missingKnownWorkspaces([base(null)], [{ socket: DEPT }], null, deptNameOf)).toEqual([]);
    // 본부는 묘비 개념이 없으므로 결측과 무관하게 계속 보장된다.
    expect(missingKnownWorkspaces([dept(null)], [{ socket: DEPT }], null, deptNameOf)).toEqual([{}]);
  });

  it("이미 열린 부서 탭은 다시 만들지 않는다", () => {
    expect(missingKnownWorkspaces([base(null), dept(null)], [{ socket: DEPT }], NO_TOMBS, deptNameOf)).toEqual([]);
  });

  it("레지스트리 중복 등재에도 같은 소켓 탭을 두 번 만들지 않는다", () => {
    const out = missingKnownWorkspaces([base(null)], [{ socket: DEPT }, { socket: DEPT }], NO_TOMBS, deptNameOf);
    expect(out).toHaveLength(1);
  });

  it("pending(부서 런칭 중 placeholder)은 본부 탭으로 세지 않는다", () => {
    // pending 은 socket 미정이라 socket==null 이다 — 이것을 본부로 세면 본부 탭이 안 생긴다.
    const out = missingKnownWorkspaces([{ tree: null, pending: true }], [], null, deptNameOf);
    expect(out).toEqual([{}]);
  });

  it("본부 + 부서 여러 개가 한 번에 빠져도 모두 돌려준다", () => {
    const out = missingKnownWorkspaces([], [{ socket: DEPT }, { socket: DEPT2 }], NO_TOMBS, deptNameOf);
    expect(out).toHaveLength(3);
    expect(out[0]).toEqual({}); // 본부가 먼저
    expect(out.map((s) => s.socket)).toEqual([undefined, DEPT, DEPT2]);
  });

  it("입력 배열을 변형하지 않는다(순수)", () => {
    const list: ReconcileWs[] = [dept(null)];
    const snapshot = JSON.stringify(list);
    missingKnownWorkspaces(list, [{ socket: DEPT2 }], NO_TOMBS, deptNameOf);
    expect(JSON.stringify(list)).toBe(snapshot);
  });
});

describe("ghostSids — 데몬이 기록조차 모르는 pane", () => {
  it("데몬 재기동으로 소멸한 옛 sid 를 집어낸다", () => {
    expect(ghostSids([2, 3, 7], new Set([12, 13]))).toEqual([2, 3, 7]);
  });

  it("★exited 만 된 surface 는 유령이 아니다(호출측이 exited 포함 목록을 넘기는 계약)", () => {
    // 깨지면: 방금 끝난 pane 이 화면에서 즉시 증발해 마지막 출력을 읽을 수 없다.
    expect(ghostSids([12, 13], new Set([12, 13]))).toEqual([]);
  });

  it("빈 트리·빈 목록에서 빈 배열", () => {
    expect(ghostSids([], new Set([1]))).toEqual([]);
    expect(ghostSids([1], new Set([1]))).toEqual([]);
  });

  it("살아있는 것과 유령이 섞이면 유령만 돌려준다", () => {
    expect(ghostSids([2, 12, 7, 13], new Set([12, 13]))).toEqual([2, 7]);
  });
});

describe("deadLiveSids — 복원 시점의 죽은 pane", () => {
  it("live 에 없으면 exited 든 미상이든 전부 뗀다(복원 시점엔 보여 줄 런타임이 없다)", () => {
    expect(deadLiveSids([2, 12, 13], new Set([12]))).toEqual([2, 13]);
  });
  it("★ghostSids 와 답이 갈리는 지점을 고정한다 — exited 만 된 sid", () => {
    // 같은 sid 14 에 대해: 틱(ghostSids·전체 목록)은 남기고, 복원(deadLiveSids·live 목록)은 뗀다.
    // 이 비대칭이 의도임을 못박는다 — 한쪽을 다른 쪽에 맞추려다 UX 가 깨진 적이 있다.
    expect(ghostSids([14], new Set([14]))).toEqual([]); // 데몬이 기억함 → 유지
    expect(deadLiveSids([14], new Set())).toEqual([14]); // live 아님 → 복원에선 제거
  });
});

describe("advanceGhostStrikes — 2연속 관측 게이트", () => {
  const key = (sid: number) => `S#${sid}`;
  const PFX = "S#";

  it("1회차는 집행하지 않고 관측만 한다(단발 오판 차단)", () => {
    const r = advanceGhostStrikes(new Map(), [2, 3], new Set([9]), key, PFX);
    expect(r.evict).toEqual([]);
    expect(r.next.get("S#2")).toBe(1);
  });

  it("2연속이면 집행한다", () => {
    const r1 = advanceGhostStrikes(new Map(), [2], new Set([9]), key, PFX);
    const r2 = advanceGhostStrikes(r1.next, [2], new Set([9]), key, PFX);
    expect(r2.evict).toEqual([2]);
    expect(r2.next.has("S#2")).toBe(false); // 집행 후 누적 해제
  });

  it("중간에 살아 돌아오면 누적이 해제된다", () => {
    const r1 = advanceGhostStrikes(new Map(), [2], new Set([9]), key, PFX);
    const r2 = advanceGhostStrikes(r1.next, [2], new Set([2]), key, PFX);
    expect(r2.next.has("S#2")).toBe(false);
    const r3 = advanceGhostStrikes(r2.next, [2], new Set([9]), key, PFX);
    expect(r3.evict).toEqual([]); // 다시 1회차부터
  });

  it("★데몬이 빈 목록을 돌려주면 아무것도 집행하지 않는다(복원 중 화면 전멸 차단)", () => {
    // cysd 는 소켓 accept 를 연 뒤 auto-restore 를 비동기로 돈다 — 그 창에서 surface.list 는
    // '성공적으로 0개'다. 이걸 '전부 죽었다'로 읽으면 트리가 통째로 지워지고 디스크에 영속된다.
    let m = new Map<string, number>();
    for (let i = 0; i < 5; i++) {
      const r = advanceGhostStrikes(m, [2, 3, 4], new Set(), key, PFX);
      expect(r.evict).toEqual([]);
      m = r.next;
    }
    expect(m.size).toBe(0); // 누적조차 쌓지 않는다
  });

  it("트리에서 사라진 sid 의 누적은 회수한다(맵 무한 증가 차단)", () => {
    const r1 = advanceGhostStrikes(new Map(), [2, 3], new Set([9]), key, PFX);
    expect(r1.next.size).toBe(2);
    const r2 = advanceGhostStrikes(r1.next, [3], new Set([9]), key, PFX); // 2번 탭이 닫힘
    expect(r2.next.has("S#2")).toBe(false);
  });

  it("다른 소켓(접두가 다른) 누적은 건드리지 않는다", () => {
    const prev = new Map([["OTHER#7", 1]]);
    const r = advanceGhostStrikes(prev, [2], new Set([9]), key, PFX);
    expect(r.next.get("OTHER#7")).toBe(1);
  });

  it("입력 맵을 변형하지 않는다(순수)", () => {
    const prev = new Map([["S#2", 1]]);
    advanceGhostStrikes(prev, [2], new Set([9]), key, PFX);
    expect(prev.get("S#2")).toBe(1);
  });
});

describe("sameSocket — Windows named pipe 는 대소문자를 구분하지 않는다", () => {
  const P_UP = "\\\\.\\pipe\\cys-dept-Sales";
  const P_LO = "\\\\.\\pipe\\cys-dept-sales";
  it("★표기만 다른 같은 파이프를 같은 데몬으로 본다(한 데몬 두 탭 차단)", () => {
    expect(sameSocket(P_UP, P_LO)).toBe(true);
  });
  it("그래서 레지스트리 표기가 달라도 탭을 또 만들지 않는다", () => {
    const out = missingKnownWorkspaces(
      [base(null), { tree: null, socket: P_UP }],
      [{ socket: P_LO }],
      NO_TOMBS,
      deptNameOf,
    );
    expect(out).toEqual([]);
  });
  it("unix 경로는 대소문자를 구분한다(그 축에서는 무시하면 안 된다)", () => {
    expect(sameSocket("/a/cys-dept-x/cys.sock", "/a/cys-dept-X/cys.sock")).toBe(false);
  });
  it("같은 문자열·양쪽 undefined 는 같고, 한쪽만 undefined 면 다르다", () => {
    expect(sameSocket(DEPT, DEPT)).toBe(true);
    expect(sameSocket(undefined, undefined)).toBe(true);
    expect(sameSocket(undefined, DEPT)).toBe(false);
  });
});

describe("deptNameFromSocket — 제품 파서(맥/윈도 양쪽)", () => {
  it("unix 소켓 경로에서 부서명을 뽑는다", () => {
    expect(deptNameFromSocket(DEPT)).toBe("dept-1");
  });
  it("★Windows named pipe 에서도 부서명을 뽑는다", () => {
    expect(deptNameFromSocket(WINPIPE)).toBe("dept-1");
  });
  it("부서 소켓이 아니면 null(본부 소켓·미정의 포함)", () => {
    expect(deptNameFromSocket(`${STATE}/cys/cys.sock`)).toBeNull();
    expect(deptNameFromSocket(undefined)).toBeNull();
  });
  it("하이픈이 든 부서명도 끝까지 가져온다", () => {
    expect(deptNameFromSocket("/x/cys-dept-sales-eu/cys.sock")).toBe("sales-eu");
  });
});

describe("scaleForPlatform — 이 판의 유일한 Windows 분기", () => {
  it("★Windows 에서 2배(배율을 지우는 변이를 잡는다)", () => {
    expect(scaleForPlatform(8_000, true)).toBe(16_000);
    expect(scaleForPlatform(10_000, true)).toBe(20_000);
  });
  it("맥·리눅스는 그대로", () => {
    expect(scaleForPlatform(8_000, false)).toBe(8_000);
  });
  it("0 과 큰 값에서도 단조", () => {
    expect(scaleForPlatform(0, true)).toBe(0);
    expect(scaleForPlatform(60_000, true)).toBeGreaterThan(scaleForPlatform(60_000, false));
  });
});
