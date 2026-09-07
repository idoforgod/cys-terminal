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

// ── ★(0.14.31 · WP-4 R1) 전출 목적지 좌석 식별 ─────────────────────────────────
//
// 무엇을 고치는가: 크로스 부서 전출은 목적지에 **셸 pane** 을 만들고 그 셸에
// `cys launch-agent …` 를 주입한다. 그런데 `launch-agent` 는 그 셸 안에서 에이전트를 띄우는
// 것이 아니라 데몬에 **새 surface 를 만든다**. 종전 코드는 그 사실을 모른 채 런처 셸을 계속
// 목적지로 삼아, 준비 폴링·핸드오프 큐잉을 **전부 빈 셸에** 하고 원본을 닫았다 — 리뷰어는
// 핸드오프를 못 받고 작업 세션은 사라진다(리뷰어 blocking).
//
// 어떻게 고치는가: 런치 **전** surface id 집합을 스냅샷하고, 그 뒤 나타난 좌석 중
//   · 런처 셸이 아니고 · 종료되지 않았고 · **역할이 정확히 우리가 시킨 그 역할**이고
//   · 데몬이 에이전트를 **실제로 관측**했고(`agent` 비어 있지 않음)
//   · 런치 시각 이후에 생겼고(`created_at`)
// 그런 좌석이 **정확히 하나**일 때만 그것을 목적지로 확정한다.
//
// 왜 이렇게 좁은가(codex 적대검증 R1 blocking 2종):
//   ⓐ "새로 생겼고 역할이 같다"만으로는 **그 전출의 결과라는 증거가 아니다** — 같은 시간대에
//      다른 경로가 같은 역할 좌석을 하나 더 띄우면 무관한 좌석에 핸드오프하고 원본을 닫는다.
//      그래서 후보가 2 이상이면 **확정하지 않는다**(모호는 승계의 근거가 아니다 — reclaim 과 같은 규율).
//   ⓑ 역할 등록만으로는 각성 증거가 아니다(신뢰 관문 보류 상태로 앉아 있을 수 있다).
//      `agent` 관측까지 요구해 "CLI 가 실제로 돈다"를 최소한으로 확인한다.
// 확정하지 못하면 **원본을 닫지 않는다**(보상 롤백) — 실패 방향은 언제나 '전출 안 함'이다.
export type SurfaceRow = {
  surface_id: number;
  role?: string | null;
  agent?: string | null;
  exited?: boolean | null;
  created_at?: number | null;
};

export function pickLaunchedAgentSid(
  before: number[],
  launcherSid: number,
  after: SurfaceRow[],
  wantRole: string,
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
      !!s.agent &&
      (launchedAt == null || s.created_at == null || s.created_at >= launchedAt),
  );
  return hits.length === 1 ? hits[0].surface_id : null;
}
