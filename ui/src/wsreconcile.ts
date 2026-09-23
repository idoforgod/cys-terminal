// 워크스페이스 복원 판정의 **순수부** — main.ts 는 여기에 배선만 한다
// (deptlabel.ts·reorder.ts 와 같은 관례: 순수 계산은 사이드 모듈 · DOM/부작용은 main.ts).
//
// ────────────────────────────────────────────────────────────────────────────
// ★무엇이 깨져 있었나 (2026-09-16 오너 실사고 · 재부팅 후 본부 워크스페이스 소실)
//
// 재부팅하면 launchd 가 GUI 보다 먼저 cysd 를 띄우고, cysd 의 콜드부트 auto-restore(phoenix)가
// 5노드를 **새 surface id 로** 되살린다(id 는 transcripts.db 기준 단조 발급이라 옛 id 는 영영
// 재등장하지 않는다 — src/bin/cysd/state.rs next_id · recall.rs max_surface_id).
// 그래서 저장본(localStorage)의 본부 pane id 는 전부 죽은 id 가 되고 트리가 null 로 접힌다.
//
// 종전 복원 필터는 그 상태의 **부서 ws 만** 보존하고 **본부 ws 는 버렸다**:
//
//     return ws.socket != null && lb?.ok === true;   // 본부는 socket=undefined → 항상 false
//
// 버린 직후 render()→saveLayout() 이 그 삭제를 디스크에 영속시키고, 3초 자동 입양 루프는
// `workspaces` 의 소켓만 순회하므로 본부 소켓을 **다시는 조회하지 않는다**. 즉 데몬 안에서는
// 5팀이 멀쩡히 살아 있는데 화면에서만 영구히 사라지고, 앱을 다시 켜도 돌아오지 않았다.
//
// 부서가 없던 시절에는 본부 ws 가 버려지면 `workspaces.length === 0` 이 되어 복원 말미의 폴백이
// 새 탭을 만들고 거기에 노드가 입양됐다 — 그래서 증상이 보이지 않았다. 부서 탭이 하나라도
// 살아남으면 length ≠ 0 이라 그 폴백이 죽는다. **부서를 만든 순간부터 결정론적으로 재현**된다.
//
// 수리는 두 가지다.
//   ① keepWorkspaceOnRestore  — 본부·부서 **대칭** 판정(소켓 유무로 차별하지 않는다).
//   ② missingKnownWorkspaces  — 저장본은 '배치 기억'이지 '존재 진실원'이 아니다. 본부와
//      레지스트리 등재 부서는 저장본에 없어도 탭을 만든다(유실·초기화·GUI 밖 생성까지 덮는다).
//
// ★경계(0.14.41 U2·U3): 이 모듈은 **탭의 존재·유령 sid** 만 판정한다. 탭 **안에서** 새 좌석을 어느 칸에
// 둘지(대표 1/3)와 재부팅 뒤 같은 역할의 새 창을 원래 칸에 다시 넣는 결속(역할 기억·구멍)은 seatlayout.ts
// 소관이다. 복원 순서: seatlayout.restoreTree(역할 칸 → 구멍) → 여기 deadLiveSids(나머지 죽은 칸 제거) →
// keepWorkspaceOnRestore → seatlayout.adoptSeat(결속 → 대표 1/3 배치) → seatlayout.anchorHeadSafe(S5).
// ────────────────────────────────────────────────────────────────────────────

/** 소켓별 live 조회 결과 중 이 판정에 필요한 최소 필드(main.ts 의 liveBySock 값). */
export interface LiveProbe {
  /** 데몬이 응답했는가. false = 일시 미응답(판정 보류) */
  ok: boolean;
}

/** 판정에 필요한 최소 워크스페이스 필드 — 좁은 입력이 테스트를 싸게 만든다. */
export interface ReconcileWs {
  socket?: string;
  tree: unknown | null;
  pending?: boolean;
}

/**
 * 복원 시 이 워크스페이스를 화면에 남길 것인가.
 *
 * 실패 방향(이 함수가 틀리면 무엇이 깨지는가):
 *   · false 를 과하게 주면 → 살아 있는 노드가 붙을 탭이 사라진다(이번 실사고).
 *   · true 를 과하게 주면 → 죽은 부서의 유령 탭이 쌓인다. 그래서 '데몬 생존'을 조건으로 남긴다.
 *
 * ★소켓 유무는 판정에 쓰지 않는다 — 본부(socket=undefined)와 부서는 같은 규칙이다.
 */
export function keepWorkspaceOnRestore(ws: ReconcileWs, lb: LiveProbe | undefined): boolean {
  if (ws.tree !== null) return true; // pane 이 하나라도 살아 있으면 무조건 보존
  if (lb?.ok === false) return true; // 데몬 일시 미응답 = 판정 보류(일시 미가동으로 영구 삭제 금지)
  return lb?.ok === true; // 데몬 생존 = 보존(빈 트리는 아래 입양·충전이 채운다)
}

/** 레지스트리(depts.json)가 아는 부서 한 줄. */
export interface KnownDept {
  socket: string;
  /** 표시명(없으면 부서명 폴백) */
  label?: string;
}

/** 새로 만들어야 하는 워크스페이스 1개의 명세. socket 없음 = 본부(기본 데몬). */
export interface MissingWsSpec {
  socket?: string;
  name?: string;
}

/**
 * 저장본에 없어서 **새로 만들어야 하는** 워크스페이스 목록을 돌려준다.
 *   · 본부(기본 데몬) 탭은 항상 1개 이상 있어야 한다.
 *   · 레지스트리에 등재됐고 묘비가 아닌 부서는 저장본에 없어도 탭이 있어야 한다.
 *
 * 이 함수가 덮는 것: ①본부 탭 소실 ②localStorage 유실·프로필 초기화 ③GUI 밖(CLI·에이전트)에서
 * 만든 부서. 셋 다 "데몬에는 있는데 화면에 없다"는 같은 병이다.
 *
 * 부작용 없음 — 호출측이 id 발급·push 를 한다(main.ts 배선).
 */
export function missingKnownWorkspaces(
  list: readonly ReconcileWs[],
  depts: readonly KnownDept[],
  tombs: ReadonlySet<string> | null,
  deptNameOf: (socket: string) => string | null,
): MissingWsSpec[] {
  const out: MissingWsSpec[] = [];
  // pending(부서 런칭 중 placeholder)은 socket 미정이라 본부로 세지 않는다.
  // 본부는 묘비 개념이 없다(기본 데몬은 삭제할 수 없다) — 아래 fail-closed 의 영향을 받지 않는다.
  if (!list.some((w) => !w.pending && w.socket == null)) out.push({});
  // ★결측은 값이 아니다(fail-closed): 묘비 조회에 **실패**했으면(null) 부서를 하나도 만들지 않는다.
  // 종전에는 null 을 '제약 없음'으로 읽었는데, 그 실패 방향이 재앙 쪽이다 —
  //   묘비 RPC 한 번의 지연·실패 → 삭제한 부서의 탭이 생김 → 그 탭이 데몬 확보 루프를 타
  //   `cys-dept launch` 를 부르고, launch 는 성공 말미에 **묘비를 지운다**(cysjavis-pack/bin/cys-dept).
  //   즉 일시적 결측 한 번이 "지운 부서는 되살아나지 않는다"는 사용자 계약을 **영구히** 깨고,
  //   그 부서의 LLM 팀까지 다시 기동시킨다(에이전트 폭주 축). 반대 방향의 대가는 "이번 기동에
  //   부서 탭이 한 박자 늦게 뜬다" 뿐이고, 저장본에 있던 부서 탭은 이 함수와 무관하게 그대로 뜬다.
  if (tombs === null) return out;
  for (const d of depts) {
    const dn = deptNameOf(d.socket);
    if (dn && tombs.has(dn)) continue; // 삭제-의도 부서는 되살리지 않는다(묘비 계약)
    if (list.some((w) => sameSocket(w.socket, d.socket))) continue;
    if (out.some((s) => sameSocket(s.socket, d.socket))) continue; // 레지스트리 중복 등재 방어
    out.push({ socket: d.socket, name: d.label || dn || undefined });
  }
  return out;
}

/**
 * 두 소켓 표기가 **같은 데몬**을 가리키는가.
 *
 * ★Windows named pipe 는 대소문자를 구분하지 않는다(`\\.\pipe\cys-dept-Sales` 와
 * `...\cys-dept-sales` 는 같은 파이프다). UI 의 소켓 비교는 전부 바이트 일치라, 레지스트리에
 * 표기가 다른 항목이 하나만 섞여도 **한 데몬이 두 탭**이 되고 같은 surface 가 두 런타임을 갖는다.
 * unix 경로는 대소문자를 구분하므로 그 축에서만 무시한다.
 */
export function sameSocket(a: string | undefined, b: string | undefined): boolean {
  if (a === b) return true;
  if (a == null || b == null) return false;
  const isPipe = (s: string) => /^\\\\[.?]\\pipe\\/i.test(s);
  if (!isPipe(a) || !isPipe(b)) return false;
  return a.toLowerCase() === b.toLowerCase();
}

/**
 * 데몬에 **기록 자체가 없는** sid — 데몬 재기동으로 소멸한 유령 pane 이다.
 *
 * ★`exited` 만 된 surface 는 여기에 넣지 않는다 — 종료 직후의 pane 은 `[exited]` 로 남겨 마지막
 * 출력을 읽게 하는 것이 기존 UX 다. 호출측은 데몬이 돌려준 **전체 목록**(exited 포함)의 id 집합을
 * knownIds 로 넘겨야 한다. live 만 넘기면 방금 끝난 pane 이 화면에서 즉시 증발한다.
 *
 * ⚠보존 기간을 정하는 것은 이 함수가 아니라 **데몬의 reap grace** 다. cysd 는 종료된 surface 를
 * 역할 노드 60초·비역할 10초 뒤 surfaces 맵에서 제거하므로(src/bin/cysd/governance.rs reap),
 * 그 뒤에는 이 함수에도 '기록 없음'으로 보여 정리 대상이 된다. 즉 계약은 "영구 보존"이 아니라
 * **"데몬이 기억하는 동안 보존"** 이다 — governance.rs 의 grace 상수가 이 UI 동작의 파라미터다.
 */
export function ghostSids(treeSids: readonly number[], knownIds: ReadonlySet<number>): number[] {
  return treeSids.filter((sid) => !knownIds.has(sid));
}

/**
 * 복원 시점(start)에서 **트리에 있으나 살아 있지 않은** sid.
 *
 * ★`ghostSids` 와 왜 다른가(둘 다 있어야 하는 이유):
 *   · 세션 중(3초 틱)에는 종료된 pane 에도 **xterm 런타임이 남아 있어** 마지막 출력을 보여 줄 수
 *     있다. 그래서 틱은 `exited` 를 살려 두고(`[exited]` 표기) **데몬이 기록조차 모르는** sid 만 친다.
 *   · 복원 시점에는 그 런타임이 없다(앱이 방금 떴다). 종료된 surface 를 트리에 남기면 보여 줄 내용이
 *     없는 `(없음)` 자리만 생긴다. 그래서 복원은 **살아 있지 않은 것**을 전부 뗀다.
 * 규칙이 갈리는 이유가 이것뿐이므로, 두 함수의 이름이 그 차이를 말하게 두고 술어를 한 곳에 모은다.
 */
export function deadLiveSids(treeSids: readonly number[], liveIds: ReadonlySet<number>): number[] {
  return treeSids.filter((sid) => !liveIds.has(sid));
}

/** advanceGhostStrikes 의 결과 — 다음 누적표와 이번에 집행할 sid 목록. */
export interface GhostStrikeStep {
  next: Map<string, number>;
  evict: number[];
}

/**
 * 유령 pane 누적 판정(순수). **2연속 관측일 때만 집행**한다.
 *
 * ★왜 1회로 집행하지 않는가: 데몬이 한 틱만 부분·빈 목록을 돌려줘도 트리가 통째로 증발하면
 * "터미널에 글자가 하나도 안 보이는" 최악이 된다. 한 번은 관측만 하고, 그 사이 목록이 정상으로
 * 돌아오면 누적을 해제한다.
 *
 * ★`knownIds` 가 **비어 있으면 아무것도 집행하지 않는다.** 데몬이 성공적으로 '0개'를 돌려주는
 * 상태는 '전부 죽었다' 와 '아직 복원 중이다' 를 구분할 수 없다(cysd 는 소켓 accept 를 연 뒤
 * auto-restore 를 비동기로 돌린다). 둘을 구분할 수 없을 때 화면을 지우는 쪽으로 무너지면 안 된다 —
 * 남겨 두는 비용은 `(없음)` 자리 몇 개고, 지우는 비용은 사용자의 화면 전체다.
 *
 * 부작용 없음 — 호출측이 evict 를 받아 트리·런타임을 정리하고 next 로 누적표를 교체한다.
 */
export function advanceGhostStrikes(
  prev: ReadonlyMap<string, number>,
  treeSids: readonly number[],
  knownIds: ReadonlySet<number>,
  keyOf: (sid: number) => string,
  keyPrefix: string,
): GhostStrikeStep {
  const next = new Map(prev);
  const evict: number[] = [];
  if (knownIds.size === 0) return { next, evict }; // 판정 보류(위 설명)
  const inTree = new Set(treeSids.map(keyOf));
  for (const sid of treeSids) {
    const k = keyOf(sid);
    if (knownIds.has(sid)) {
      next.delete(k); // 살아 돌아왔다 — 누적 해제
      continue;
    }
    const n = (next.get(k) ?? 0) + 1;
    if (n < 2) {
      next.set(k, n); // 1회차는 관측만
      continue;
    }
    next.delete(k);
    evict.push(sid);
  }
  // 트리에서 사라진 sid(탭 닫힘 등)의 누적은 회수한다 — 위 두 해제 경로에 다시 오지 않으므로
  // 두지 않으면 맵이 단조 증가한다. `keyPrefix` 범위(=한 소켓)만 훑는다.
  // ⚠호출 계약: `treeSids` 는 **그 소켓의 전 워크스페이스 트리 합집합**이어야 한다. 워크스페이스
  // 하나씩 부르면, 같은 소켓의 다른 탭에 있는 sid 가 '트리에서 사라졌다'로 오판돼 누적이 지워진다.
  for (const k of [...next.keys()]) {
    if (k.startsWith(keyPrefix) && !inTree.has(k)) next.delete(k);
  }
  return { next, evict };
}

/**
 * 복원 경로 시간 상한의 **플랫폼 배율**.
 *
 * ★왜 Windows 만 키우나: 재부팅 직후 Windows 는 Defender 스캔·콜드 디스크·ConPTY 기동이 겹치고,
 * named pipe 는 혼잡 시 `ERROR_PIPE_BUSY` 재시도로 연결 수립에만 수 초를 먹는다(src/lib.rs).
 * 맥 기준값을 그대로 쓰면 **살아 있는 데몬을 죽었다고 오판**해 불필요한 재기동을 부른다.
 *
 * ★순수 함수로 둔 이유: 이 diff 의 유일한 Windows 분기인데, main.ts 안에 상수로 있으면 그 동작을
 * 재는 테스트를 쓸 수 없다(이름의 존재만 확인하는 핀은 배율을 지우는 변이를 못 잡는다 — 실측 확인).
 */
export function scaleForPlatform(ms: number, isWindows: boolean): number {
  return isWindows ? ms * 2 : ms;
}
