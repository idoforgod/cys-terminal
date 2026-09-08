// pane 전출(동일 socket 워크스페이스 간)의 순수 트리 변형 로직 (DOM 무접촉 — main.ts transferPaneToWs가 배선).
//
// 불변식: ①원자성 — 실패(null 반환) 시 호출측이 두 트리 모두 무변경 유지 ②sid 유일성 —
// src에서 제거한 노드만 dest에 붙이고, dest에 같은 sid가 이미 있으면 거부(유령 pane 차단).

export type TNode =
  | { type: "split"; dir: "row" | "col"; ratio?: number; a: TNode; b: TNode }
  | { type: "pane"; sid: number };

export function treeSids(node: TNode | null, out: number[] = []): number[] {
  if (!node) return out;
  if (node.type === "pane") out.push(node.sid);
  else {
    treeSids(node.a, out);
    treeSids(node.b, out);
  }
  return out;
}

// sid pane 노드를 제거하고 형제로 붕괴시킨다(main.ts replaceNode의 제거 특수형과 동일 의미).
function removeSid(node: TNode, sid: number): TNode | null {
  if (node.type === "pane") return node.sid === sid ? null : node;
  const a = removeSid(node.a, sid);
  const b = removeSid(node.b, sid);
  if (a && b) return { ...node, a, b };
  return a ?? b;
}

// 트리 말단(우측 row 분할)에 pane을 덧붙인다 — actionNew의 삽입 규칙과 동일.
export function appendPane(tree: TNode | null, sid: number): TNode {
  const moved: TNode = { type: "pane", sid };
  return tree ? { type: "split", dir: "row", a: tree, b: moved } : moved;
}

// src에서 sid를 떼어 dest 끝에 붙인 새 (src, dest) 트리 쌍. 원천 부재·대상 중복이면 null.
export function transferTrees(
  src: TNode | null,
  dest: TNode | null,
  sid: number,
): { src: TNode | null; dest: TNode } | null {
  if (!src || !treeSids(src).includes(sid)) return null;
  if (dest && treeSids(dest).includes(sid)) return null;
  return { src: removeSid(src, sid), dest: appendPane(dest, sid) };
}

// ── ★(0.14.31 · WP-4 R2) 전출 목적지 좌석 식별 ─────────────────────────────────
//
// 무엇을 고치는가: 크로스 부서 전출은 목적지에 **셸 pane** 을 만들고 그 셸에
// `cys launch-agent …` 를 주입한다. 그런데 `launch-agent` 는 그 셸 안에서 에이전트를 띄우는
// 것이 아니라 데몬에 **새 surface 를 만든다**. 종전 코드는 그 사실을 모른 채 런처 셸을 계속
// 목적지로 삼아, 준비 폴링·핸드오프 큐잉을 **전부 빈 셸에** 하고 원본을 닫았다 — 리뷰어는
// 핸드오프를 못 받고 작업 세션은 사라진다(리뷰어 blocking).
//
// R1 은 "런치 뒤에 나타난 같은 역할 좌석이 유일하면 그것"으로 좁혔지만, 그 조건은 **그 전출의
// 결과라는 증거가 아니다**(codex 적대검증 R2 blocking):
//   ⓐ `!!s.agent` 는 메타데이터 이름이 등록됐다는 뜻일 뿐 **생존 관측이 아니다** —
//      `launch-agent` 는 준비 판정 **전에** 메타를 세우고 `agent_seen=false` 로 출발한다.
//      즉 즉사한 CLI 도, 아직 뜨지도 않은 CLI 도 이 검사를 통과했다.
//   ⓑ 같은 시간대에 다른 경로가 같은 역할 좌석을 하나 띄우면 **무관한 좌석**이 목적지가 된다.
//   ⓒ agent 종류(claude/codex/gemini)를 보지 않아 다른 종류의 유일 후보도 선택됐다.
// 그래서 R2 는 **데몬이 기록한 생성자**(`created_by` — `surface.create` 호출자의 pane id를
// 데몬이 발신 pid 로 도출해 원장에 적은 값 · 호출자가 신고할 수 없다)를 축으로 삼는다:
//   · `created_by === 런처 셸 sid` — 이 좌석이 **내가 보낸 그 명령**의 산물이라는 증거
//   · `agent_alive === true` — 데몬 watchdog 이 그 프로세스를 **실제로 관측**했다(3값 축:
//     `null` 은 '말할 것이 없음'이고 `false` 는 '종료 통지됨'이다 — 둘 다 확정 근거가 아니다)
//   · `agent === <--agent 인자>` — 시킨 종류가 떴는가
//   · 역할 일치 · 미종료 · 런치 이후 생성 · 런치 전 집합에 없음 · **정확히 하나**
// 확정하지 못하면 **원본을 닫지 않는다**(보상 롤백) — 실패 방향은 언제나 '전출 안 함'이다.
export type SurfaceRow = {
  surface_id: number;
  role?: string | null;
  agent?: string | null;
  /** 3값 생존 관측(true=관측됨 · false=종료 통지 · null=말할 것 없음). `agent` 이름과 다르다. */
  agent_alive?: boolean | null;
  exited?: boolean | null;
  created_at?: number | null;
  /** 데몬이 기록한 **생성자 pane** — 호출자가 신고할 수 없는 값(surface.list `created_by`). */
  created_by?: number | null;
  /** 각성 래치(첫 자기보고 시각) — null 은 '아직 각성 증거 없음'. */
  awakened_at?: number | null;
};

export function pickLaunchedAgentSid(
  before: number[],
  launcherSid: number,
  after: SurfaceRow[],
  wantRole: string,
  wantAgent: string,
  launchedAt?: number,
): number | null {
  const known = new Set(before);
  const hits = after.filter(
    (s) =>
      s.surface_id !== launcherSid &&
      !known.has(s.surface_id) &&
      !s.exited &&
      !!s.role &&
      s.role === wantRole &&
      // ★생성자 증거 — 이 좌석이 그 런처 셸에서 태어났는가(부재=모름이지 아님이 아니다 →
      //   확정하지 않는다: fail-closed).
      s.created_by === launcherSid &&
      // ★생존 **관측**(메타 등록이 아니다). `=== true` 로 3값을 좁힌다.
      s.agent_alive === true &&
      // ★시킨 종류가 떴는가(다른 CLI 의 유일 후보를 목적지로 삼지 않는다).
      !!s.agent &&
      s.agent === wantAgent &&
      (launchedAt == null || s.created_at == null || s.created_at >= launchedAt),
  );
  return hits.length === 1 ? hits[0].surface_id : null;
}

/** 목적지가 **지금** 인계를 받을 수 있는 상태인가 — 선택 때 쓴 술어를 그대로 재평가한다.
 *
 * ★왜 생존 관측만으로 부족한가(codex 적대검증 R2 blocking): 프로세스가 살아 있어도 신뢰
 * 관문·인증 화면에 앉아 있을 수 있다. 그 상태에서 원본을 닫으면 인계는 아무도 읽지 않는다.
 * 래치는 "노드가 지침을 읽고 스스로 신고했다"는 데몬의 단방향 사실이다(status.set 이 유일
 * write path). 서지 않으면 **원본을 닫지 않는다** — 목적지는 살아 있으니 사람이 판단한다.
 *
 * ★그리고 래치는 **필요조건이지 충분조건이 아니다**(독립 재유도 · codex blocking #3):
 * `awakened_at` 은 한 번 서면 내려가지 않는 **과거 사실**인데, 원본 종료의 유일한 관문이
 * 그것 하나였다. 각성 뒤에 에이전트가 죽거나(`agent_alive=false` — 셸만 남아 `exited=false`)
 * 역할을 잃은 좌석도 그대로 통과해, 그 상태에서 원본을 닫으면 **좌석 사망 + 인계 유실**이다.
 * 그래서 종료 직전 이 술어가 선택 때 쓴 축을 다시 본다: 역할 보유(요구하면 동일 역할) ·
 * 생존 **관측**(3값을 `true` 로만 좁힌다) · 시킨 종류 · 미종료 · 래치. 모르면 거짓이다. */
export function destinationLooksAwake(
  row: SurfaceRow | undefined,
  wantRole?: string,
  wantAgent?: string,
): boolean {
  if (!row || row.exited) return false;
  // 역할 축: 인계는 '그 역할' 앞으로 간다 — 역할을 잃은 좌석은 수신자가 아니다.
  if (!row.role) return false;
  if (wantRole != null && row.role !== wantRole) return false;
  // 생존 축: `null`(말할 것 없음)·`false`(종료 통지) 둘 다 근거가 아니다.
  if (row.agent_alive !== true) return false;
  // 종류 축: 시킨 CLI 가 아직 그 좌석의 것인가(호출부가 요구할 때만).
  if (wantAgent != null && row.agent !== wantAgent) return false;
  return row.awakened_at != null;
}
