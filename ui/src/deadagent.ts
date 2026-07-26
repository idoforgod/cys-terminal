// CU-6B2(사다리 2단) — agent 가 죽은 pane 의 **시각 상태**만 결정하는 순수 함수.
// main.ts 의 refreshPaneTitles(3초 폴링)가 이 판정에 배선만 하고, DOM 조작은 호출측이 한다.
//
// 왜 순수 분리인가: 이 판정은 "무엇을 화면에서 접을 것인가"를 정하는 술어라서, 잘못되면 살아 있는
// 노드를 접어 사용자에게 '죽은 것처럼' 보이게 만든다(오탐의 대가가 크다). 그래서 DOM 없이
// 결정론으로 회귀 테스트한다(deadagent.test.ts) — deptlabel.ts 와 동일한 관용구.
//
// 계약(설계 정본 §4 CU-6B2):
//   · 접기는 **표시**일 뿐이다 — PTY·스크롤백·데몬 상태 무접촉이고 close 는 절대 하지 않는다.
//     사다리 3단(오너 close)은 비범위이며, 기존 close 버튼(×)은 그대로 사람이 쓴다.
//   · `agent_alive === false` 엄격 — null/undefined(미상)는 절대 대상이 아니다. 데몬은 launch-agent 로
//     등록된 노드의 presence 만 답하므로, 수동 셸·구데몬·부팅 중 pane 이 null 로 온다.
//   · exited 된 pane 은 대상이 아니다 — 그건 '종료 표시([exited])'가 이미 말하는 다른 상태다.

export interface AgentLivenessLike {
  // 데몬 org.status/surface.list 봉투의 부분집합(추가 필드는 무시 — additive 스큐 안전).
  agent_alive?: boolean | null;
  exited?: boolean | null;
}

/** agent 는 죽었는데 셸(pane)은 살아있는가. 미상(null/undefined)은 살아있다고 본다(보수적). */
export function isDeadAgentPane(s: AgentLivenessLike | null | undefined): boolean {
  if (!s) return false;
  return s.agent_alive === false && s.exited !== true;
}

/** 접힌 pane 헤더에 남길 한 줄. 역할이 없으면(무역할 셸) 그 사실을 그대로 적는다. */
export function deadAgentHeaderText(role: string | null | undefined, collapsed: boolean): string {
  const r = role && role.trim() ? role.trim() : "역할 없음";
  return `${r} · agent✗ · ${collapsed ? "펼치기" : "접기"}`;
}
