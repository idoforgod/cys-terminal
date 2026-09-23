// ui/src/seatsig.ts — 사이드바 좌석 신호의 **정직한** 집계(0.14.41 · U12 빈 자리 표시 정직화 + U4 A5①).
//
// 무엇이 틀렸나(근거: _evidence/impl-9items-20260923/phase1/U12-hollow-seat-alive{,.refute}.md MRC-2·MRC-3·D4·D6·D11,
//   U4c-silentpass-pack-ui{,.refute}.md F7·D7):
//   ① 신호 없음 = 초록. 종전 buildTab 은 `dead ? error : idleN ? idle : working` 이라 org.status 가 한 번도
//      성공하지 않은 소켓(신호 0건)도, 응답이 끊긴 데몬(직전 값 영구 유지)도 **초록 점**이었다.
//   ② 이름표만 남은 빈 자리 = 초록. 데몬은 에이전트를 한 번도 못 본 좌석을 agent_alive=null(모름)로 내는데
//      사이드바는 `agent_alive === false` 만 셌다 → 셸만 남은 역할 자리가 working/idle 로 그려졌다.
//   ③ 낡은 자기보고 = 영구 working. status.state 를 나이와 무관하게 썼다(set_meta 는 status 를 지우지 않는다).
//
// 원칙 — **표시만** 바꾼다. 데몬·팩·부트·회수·편성·orchestra check 의 의미는 무변경이고, org.status 가 이미
//   싣는 필드(role·exited·agent_alive·seat·idle_secs·status.age_secs)만 읽는다. agent_alive 3상 계약(M1)은
//   그대로다 — null 을 false 로 접지 않는다(❌ '사망 확정' 축과 ○ '빈 자리' 축을 섞지 않는다 · AgentDead > Hollow).
//   빈 자리는 빨간 점이 아니고 토스트도 없다(반박 D4 — 사망 확정과 다른 사실이다). 빈 자리 판정 대상은 등록
//   에이전트가 있는 좌석뿐이다(isHollowSeat 머리주석 — 루트가 셸이 아닌 역할 pane 의 거짓 빈 자리 차단).
// 모듈 최상위 부수효과 0 · DOM/Tauri 무관(bun test 대상) · 구형 WKWebView 비호환 문법 0.

/** main.ts `setInterval(refreshSidebarStatus, 10000)` 과 같은 값 — 바꾸면 둘 다(seatsig.test 가 소스 핀). */
export const SIG_POLL_MS = 10_000;
/** 마지막 성공 조회 뒤 이 시간이 지나면 그 신호는 '미확인'이다(설계 §3 U4 A5① — 3×주기). 시간 기반이라
 *  in-flight 가드로 건너뛴 틱(실패로 세지 않는 틱)에도 늦지 않는다(반박 D7). */
export const SIG_STALE_MS = 3 * SIG_POLL_MS;
/** 자기보고 state 신선도 — cysjavis-pack/bin/javis_boot_node.py STATUS_FRESH_SECS(600)와 같은 값(테스트가 대조). */
export const STATUS_STATE_FRESH_SECS = 600;
/** 빈 자리로 그리기 전 무출력 유예. 데몬 seat 캐시는 5초 워치독 틱이라 기동 직후 한두 틱은 'empty' 로 남을 수
 *  있다 — 기동 중인 좌석은 출력(명령 에코·TUI)이 있어 idle_secs 가 작으므로 이 유예가 순간 오표시를 막는다. */
export const HOLLOW_QUIET_SECS = 20;
/** 좌석이 생긴 뒤(org.status created_at) 이 시간이 지나기 전에는 빈 자리로 그리지 않는다 — 무거운 셸 초기화
 *  (.zshrc·Windows 콜드부트)가 조용히 길어져 에이전트 기동 전 메타 좌석이 잠깐 '○빈자리'로 보이는 것을 막는다
 *  (리뷰1 #4 · 조사 H5). 구 데몬(created_at 부재)은 이 하한 없이 무출력 유예만 본다. */
export const HOLLOW_MIN_AGE_SECS = 60;

/** 빈 자리 유예 한 벌. main.ts 는 hollowGraceScaled(winScaled) 로 Windows 에서 2배로 연다(낡음 창과 같은 관례). */
export type HollowGrace = { quietSecs: number; minAgeSecs: number };
export const HOLLOW_GRACE: HollowGrace = { quietSecs: HOLLOW_QUIET_SECS, minAgeSecs: HOLLOW_MIN_AGE_SECS };
/** 플랫폼 배율(ms→ms, main.ts winScaled)을 빈 자리 유예에 적용한다. 배율 함수가 이상한 값을 내면 기본값을 쓴다. */
export function hollowGraceScaled(scale: (ms: number) => number): HollowGrace {
  const sc = (secs: number): number => {
    const v = scale(secs * 1000) / 1000;
    return typeof v === "number" && Number.isFinite(v) && v >= secs ? v : secs;
  };
  return { quietSecs: sc(HOLLOW_QUIET_SECS), minAgeSecs: sc(HOLLOW_MIN_AGE_SECS) };
}

/** 사이드바 노드 신호 캐시 한 칸(main.ts nodeSig 값). */
export type SeatSig = {
  role: string | null;
  /** seatState(행) — 신선한 자기보고 또는 출력 기반 산식 */
  state: string;
  ctx_pct: number | null;
  idle_secs: number;
  agent_alive: boolean | null;
  /** isHollowSeat(행) — 이름표만 남은 빈 자리 */
  hollow: boolean;
  /** isUnregisteredRoleSeat(행) — 등록 에이전트 없는 역할 좌석이라 빈 자리 여부를 판정하지 않는 자리(중립 표식만) */
  unregistered?: boolean;
  /** 이 신호를 받은 시각(Date.now() ms) — **성공 조회에서만** 찍힌다. 낡음 판정의 유일한 근거. */
  at: number;
};

/** 자기보고(status)가 신선한가 — age_secs 가 숫자이고 STATUS_STATE_FRESH_SECS 이하. 나이 미상은 '방금'이 아니다. */
export function selfReportFresh(st: any): boolean {
  return st != null && typeof st.age_secs === "number" && st.age_secs <= STATUS_STATE_FRESH_SECS;
}

/** 자기보고 state 는 신선할 때만 쓴다. 낡았거나 나이 미상이면 종전 출력 기반 산식(idle_secs > 60 → idle). */
export function seatState(n: any): string {
  const st = n?.status;
  const age = st?.age_secs;
  if (st && typeof st.state === "string" && st.state !== "" && typeof age === "number" && age <= STATUS_STATE_FRESH_SECS)
    return st.state;
  return n?.idle_secs > 60 ? "idle" : "working";
}

/**
 * 이름표만 남은 빈 자리 — org.status 한 행에서 **모두 참**일 때만:
 *   role 있음 ∧ 등록 에이전트 있음(agent — launch-agent/restore 가 set_meta 로 적는 이름)
 *   ∧ pane 살아 있음(exited≠true) ∧ agent_alive 가 true 도 false 도 아님(null = 등록됐는데 한 번도 못 봄)
 *   ∧ seat==="empty"(셸 아래 프로세스 0 — 데몬 커널 사실) ∧ grace.quietSecs 이상 무출력
 *   ∧ 자기보고가 grace.quietSecs 보다 오래됨(방금 set-status 가 왔으면 산 에이전트 — 모순 신호는 종전 표시 · 리뷰1 #4 H6)
 *   ∧ 좌석 나이(now − created_at) ≥ grace.minAgeSecs(created_at 이 있을 때 · 느린 셸 초기화의 거짓 빈 자리 차단 · H5).
 * 유예는 호출자가 플랫폼 배율을 넣는다(main.ts HOLLOW_GRACE_UI = hollowGraceScaled(winScaled) — Windows 2배).
 * 한 항이라도 어긋나면 false = 종전 표시(구 데몬의 seat·agent 키 부재·콜드스타트 unknown 무해).
 * agent_alive=false 는 여기 들지 않는다 — 그것은 ❌ 사망 확정 축이다(반박 D6: 두 사실을 섞지 않는다).
 * ★등록 에이전트가 없는 역할 좌석은 **판정하지 않는다**(반박 D4 + 코드 실측): `cys new-surface --cmd <prog> --role r`
 *   는 `zsh -lc` 의 암묵 exec 로 pane 루트가 셸이 아닌 그 프로그램이 된다(예: cycle-autopilot 검증자 워처). 자손이 없으니
 *   seat=empty 이고 agent 도 없다 — 살아 일하는 pane 이 빈 자리로 보인다. 데몬 응답엔 '루트가 셸인가'가 없어 UI 가 가려낼 수
 *   없으므로 이 부류는 종전 표시로 두고 데몬측 설계(조사 보고서 H2)로 넘긴다. 온보딩 CLI 미설치 master 셸도 여기 든다.
 * ⚠이 판정은 **표시 전용**이다. 파괴·재기동·회수 판정에 쓰지 마라(설계 §2 U12 제외 — 회수 kill 경로 ④).
 */
export function isHollowSeat(n: any, nowMs: number = Date.now(), grace: HollowGrace = HOLLOW_GRACE): boolean {
  if (!n || typeof n !== "object") return false;
  if (typeof n.role !== "string" || n.role.trim() === "") return false;
  if (typeof n.agent !== "string" || n.agent.trim() === "") return false;
  if (n.exited === true) return false;
  if (n.agent_alive === true || n.agent_alive === false) return false;
  if (n.seat !== "empty") return false;
  const sa = n.status?.age_secs;
  if (typeof sa === "number" && sa < grace.quietSecs) return false;
  const c = n.created_at;
  if (typeof c === "number" && Number.isFinite(c) && nowMs / 1000 - c < grace.minAgeSecs) return false;
  const idle = n.idle_secs;
  return typeof idle === "number" && idle >= grace.quietSecs;
}

/**
 * 등록 에이전트가 없는 역할 좌석 중 셸 아래 프로세스가 보이지 않는 자리 — role 있음 ∧ agent 없음 ∧ exited≠true
 *   ∧ agent_alive≠true ∧ seat==="empty". isHollowSeat 가 **판정하지 않는** 부류(명령을 직접 띄운 워처 pane 은 살아 있고,
 *   formation 이 만든 맨 master 셸·온보딩 CLI 미설치 셸은 비어 있다 — 데몬 응답만으로는 가를 수 없다).
 * 표시는 점 색을 바꾸지 않는 중립 문구 '미등록 N' 뿐이다(양쪽 모두에게 사실인 말 · 리뷰1 #3 · 반박 D4).
 * ⚠표시 전용 — 파괴·재기동·회수 판정에 쓰지 마라.
 */
export function isUnregisteredRoleSeat(n: any): boolean {
  if (!n || typeof n !== "object") return false;
  if (typeof n.role !== "string" || n.role.trim() === "") return false;
  if (typeof n.agent === "string" && n.agent.trim() !== "") return false;
  if (n.exited === true) return false;
  if (n.agent_alive === true) return false;
  return n.seat === "empty";
}

/** 신호가 신선한가 — 수신 시각이 창 안. 시계가 뒤로 뛰어 at 이 창보다 먼 미래여도 신선으로 보지 않는다. */
function isFresh(s: any, now: number, staleMs: number): s is SeatSig {
  if (!s || typeof s !== "object" || typeof s.at !== "number" || !Number.isFinite(s.at)) return false;
  const age = now - s.at;
  return age <= staleMs && age >= -staleMs;
}
/** 빈 자리로 셀 좌석(사망 확정 false 는 제외 — AgentDead > Hollow). */
const hollowSig = (s: SeatSig): boolean => s.hollow === true && s.agent_alive !== false;

export type WsDot = "working" | "idle" | "error" | "hollow" | "unknown";
export type WsSigSummary = {
  dot: WsDot;
  /** pane 수(트리 기준 — 자리표시자 포함) */
  total: number;
  /** 신선한 신호 수 */
  fresh: number;
  /** 신호 없음 + 낡음 */
  unknownN: number;
  hollowN: number;
  hollowRoles: string[];
  /** 신선한 신호 중 등록 에이전트 없는 역할 좌석(빈 자리 판정 제외 · 중립 표식) */
  unregN: number;
  unregRoles: string[];
  dead: number;
  idleN: number;
  /** 신선한 산 좌석의 최대 CTX%(없으면 0 — 60% 미만은 표기하지 않는다) */
  worst: number;
  /** 이 작업공간 신호 중 가장 최근 수신 시각(낡은 것 포함) · 한 번도 없으면 null */
  lastOkAt: number | null;
  /** 사람용 툴팁(왜 이 색인가) */
  title: string;
};

/**
 * 작업공간 탭 한 줄의 신호 집계. 초록(working)은 **신선한 신호가 전부 산 좌석**일 때만이다.
 * 우선순위: 신선한 신호 0건 → unknown(회색) > 사망 확정 → error > 빈 자리 → hollow > 일부 미확인 → unknown
 *           > idle → idle > working.
 */
export function summarizeWsSigs(
  sigs: ReadonlyArray<SeatSig | null | undefined>,
  now: number,
  staleMs: number = SIG_STALE_MS,
): WsSigSummary {
  const total = sigs.length;
  let lastOkAt: number | null = null;
  const fresh: SeatSig[] = [];
  for (const s of sigs) {
    if (s && typeof s === "object" && typeof s.at === "number" && Number.isFinite(s.at))
      lastOkAt = lastOkAt === null ? s.at : Math.max(lastOkAt, s.at);
    if (isFresh(s, now, staleMs)) fresh.push(s);
  }
  const unknownN = total - fresh.length;
  const hollow = fresh.filter(hollowSig);
  const live = fresh.filter((s) => !hollowSig(s)); // 빈 자리의 동결 CTX·idle 은 집계에 넣지 않는다
  const dead = live.filter((s) => s.agent_alive === false).length;
  const idleN = live.filter((s) => s.state === "idle" || s.idle_secs > 60).length;
  const worst = live.reduce((acc, s) => (typeof s.ctx_pct === "number" ? Math.max(acc, s.ctx_pct) : acc), 0);
  const hollowRoles = hollow.map((s) => String(s.role ?? "?"));
  const unreg = live.filter((s) => s.unregistered === true);
  const unregRoles = unreg.map((s) => String(s.role ?? "?"));

  let dot: WsDot;
  if (fresh.length === 0) dot = "unknown";
  else if (dead) dot = "error";
  else if (hollow.length) dot = "hollow";
  else if (unknownN) dot = "unknown";
  else if (idleN) dot = "idle";
  else dot = "working";

  const lines: string[] = [];
  if (total === 0) {
    lines.push("좌석 없음 — 이 작업공간에 pane 이 없습니다");
  } else if (fresh.length === 0) {
    lines.push(
      lastOkAt === null
        ? "상태 수집 전 — 이 작업공간의 데몬 응답을 아직 받지 못했습니다(초록으로 그리지 않습니다)"
        : `상태 미확인 — 데몬 응답 없음(마지막 성공 ${Math.max(0, Math.round((now - lastOkAt) / 1000))}초 전) · 낡은 값은 표시하지 않습니다`,
    );
  } else {
    if (dead) lines.push(`❌ 에이전트 종료 확인 ${dead}`);
    if (hollow.length)
      lines.push(
        `○ 빈 자리 ${hollow.length}: ${hollowRoles.join(", ")} — 역할 이름표는 있으나 에이전트 프로세스가 보이지 않습니다`,
      );
    if (unknownN)
      lines.push(`? 미확인 ${unknownN} — 상태 신호가 없거나 ${Math.round(staleMs / 1000)}초 넘게 갱신되지 않았습니다`);
    if (unreg.length)
      lines.push(
        `· 미등록 ${unreg.length}: ${unregRoles.join(", ")} — 등록 에이전트가 없고 셸 아래 프로세스도 보이지 않아 빈 자리인지 판정하지 않습니다(명령을 직접 띄운 pane 이면 정상)`,
      );
  }
  return {
    dot,
    total,
    fresh: fresh.length,
    unknownN,
    hollowN: hollow.length,
    hollowRoles,
    unregN: unreg.length,
    unregRoles,
    dead,
    idleN,
    worst,
    lastOkAt,
    title: lines.join("\n"),
  };
}

/** 렌더 경로용 — 판정 예외가 탭 바 전체를 멈추지 않게 회색 요약으로 접는다(④ 방어 · 반박 D7).
 *  staleMs 는 **필수**다 — 호출자(main.ts buildTab)가 플랫폼 배율 창(SIG_STALE_MS_UI)을 빠뜨리면 타입 검사가 잡는다. */
export function summarizeWsSigsSafe(
  sigs: ReadonlyArray<SeatSig | null | undefined>,
  now: number,
  staleMs: number,
): WsSigSummary {
  try {
    return summarizeWsSigs(sigs, now, staleMs);
  } catch {
    const total = Array.isArray(sigs) ? sigs.length : 0;
    return {
      dot: "unknown",
      total,
      fresh: 0,
      unknownN: total,
      hollowN: 0,
      hollowRoles: [],
      unregN: 0,
      unregRoles: [],
      dead: 0,
      idleN: 0,
      worst: 0,
      lastOkAt: null,
      title: "상태 미확인 — 신호 집계 실패",
    };
  }
}

/** 탭 서브라인 조각. 신선한 신호가 0건이면 낡은 ❌·CTX·💤 대신 '상태 수집 전/상태 미확인' 한 마디만. */
export function wsSubBits(s: WsSigSummary, firstTitle: string): string[] {
  const bits = [`${s.total} pane`];
  if (firstTitle) bits.push(firstTitle);
  if (s.total === 0) {
    bits.push("좌석 없음");
    return bits;
  }
  if (s.fresh === 0) {
    bits.push(s.lastOkAt === null ? "상태 수집 전" : "상태 미확인");
    return bits;
  }
  if (s.worst >= 60) bits.push(`CTX ${s.worst}%`);
  if (s.idleN) bits.push(`💤${s.idleN}`);
  if (s.dead) bits.push(`❌${s.dead}`);
  if (s.hollowN) bits.push(`○빈자리 ${s.hollowN}`);
  if (s.unknownN) bits.push(`?미확인 ${s.unknownN}`);
  if (s.unregN) bits.push(`미등록 ${s.unregN}`);
  return bits;
}

/**
 * CEO 좌석이 '살아서 생성 중'인가(승인 자동 전환 유예 판정 · main.ts ceoIsActivelyGenerating).
 * 반환 null = 낡은 신호라 판정에 쓰지 않는다(호출자는 건너뛴다). 빈 자리 CEO 는 활성이 아니다 — 유예를 끌지
 * 않고 사람에게 보여 주는 쪽으로 실패한다. agent_alive=null 자체는 종전대로 '모름'이지 사망이 아니다(M1).
 */
export function ceoSigActivity(sig: SeatSig, now: number, staleMs: number = SIG_STALE_MS): boolean | null {
  if (!isFresh(sig, now, staleMs)) return null;
  const alive = sig.agent_alive !== false && !hollowSig(sig);
  const working = sig.state === "working" || sig.idle_secs < 30;
  return alive && working;
}

/**
 * 승인 전환 유예의 CEO 활성 판정 한 벌(main.ts ceoIsActivelyGenerating 은 신호 재갱신 뒤 이것만 부른다).
 * 첫 번째 **신선한** CEO 신호의 판정을 쓴다 — 낡은 CEO 신호(null)는 건너뛰고, 활성으로 치지 않는다(리뷰1 #1 M24:
 * 낡은 신호가 유예를 끌면 사람 소환이 최대 300초 늦어진다). CEO 신호가 없거나 전부 낡았으면 false(즉시 사람 소환).
 * staleMs 는 필수(플랫폼 배율 창을 호출자가 넣는다).
 */
export function ceoActiveFromSigs(sigs: Iterable<SeatSig | null | undefined>, now: number, staleMs: number): boolean {
  for (const sig of sigs) {
    if (!sig || sig.role !== "ceo") continue;
    const active = ceoSigActivity(sig, now, staleMs);
    if (active === null) continue;
    return active;
  }
  return false;
}

/** Control Center Tasks 행의 좌석·자기보고 판정(리뷰1 #2 — 사이드바와 같은 U12 규칙을 Tasks 탭에도).
 *  seat: offline(pane 종료) > hollow(빈 자리) > live · report: fresh(≤600초) | stale(낡음·나이 미상) | none. */
export type TaskSeatView = { seat: "offline" | "hollow" | "live"; report: "fresh" | "stale" | "none" };
export function taskSeatView(s: any, nowMs: number, grace: HollowGrace): TaskSeatView {
  const st = s?.status;
  const report: TaskSeatView["report"] = st == null ? "none" : selfReportFresh(st) ? "fresh" : "stale";
  if (s?.exited) return { seat: "offline", report };
  if (isHollowSeat(s, nowMs, grace)) return { seat: "hollow", report };
  return { seat: "live", report };
}
/** Tasks 부서 머리의 '작업중' 계수 — 산 좌석만, 신선한 자기보고면 그 state, 아니면 출력 기반(60초 이하 = 활동). */
export function taskSeatIsWorking(s: any, v: TaskSeatView): boolean {
  if (v.seat !== "live") return false;
  if (v.report === "fresh") return s?.status?.state === "working";
  return (s?.idle_secs ?? 999) <= 60;
}
