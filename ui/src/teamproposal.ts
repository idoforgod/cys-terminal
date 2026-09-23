// U16(0.14.41) 말로 팀 만들기 1차 — 팀 제안 카드·확인 창의 **순수 판정자**(main.ts 는 배선만 한다).
//
// 흐름(설계 정본 §3 U16): 본부 대표가 `cys team-propose` 로 제안 1건을 올리면 데몬이 잠금(제안자·건수·
// 해소 권한)을 걸고 승인 피드에 kind=team-create-request 항목을 둔다. 카드의 [확인 창 열기] → 확인 창
// (이 모듈이 만든 문구 · 원문 그대로) → [만들기] 는 기존 생성 경로(addDeptWorkspace → allocate_dept_daemon)
// 에 이름·하는 일을 넘기고, **생성 성공 뒤에만** feed_reply allow 를 보낸다. [나중에]·창 바깥 클릭은 거부가
// 아니다(pending 유지) — 거부는 카드의 [만들지 않기] 버튼뿐이다. 자동 팝업은 없다(포커스 탈취·취소=거부
// 사고 · 반박 M6).
//
// 스키마·한도의 정본은 Rust `src/team_spec.rs` 다(teamproposal.test.ts 가 그 파일을 읽어 대조한다).
// 여기서 다시 검증하는 이유: 형식이 틀린 항목에는 생성 버튼을 열지 않는다(데몬이 이미 거르지만 구 데몬·
// 손상 항목 대비 fail-closed).
// 모듈 규약: 최상위 부수효과 0 · 구형 WKWebView 비호환 문법 0(lookbehind·.at·findLast·structuredClone 없음).

export const TEAM_CREATE_KIND = "team-create-request";
export const TEAM_DISPLAY_MAX = 40;
export const TEAM_PURPOSE_MAX = 2000;

export interface TeamSpec {
  id: string;
  display: string;
  purpose: string;
}

export type TeamParse = { ok: true; spec: TeamSpec } | { ok: false; reason: string };

// Rust INVISIBLE 과 같은 집합 — 보이지 않는 서식·양방향 제어(확인 창 스푸핑 차단).
const INVISIBLE = /[​-‏‪-‮⁠-⁤⁦-⁩﻿]/;
// Rust char::is_control() = 일반 범주 Cc(U+0000–001F · U+007F–009F).
const CONTROL = /[\u0000-\u001F\u007F-\u009F]/;
const CONTROL_EXCEPT_NL_TAB = /[\u0000-\u0008\u000B-\u001F\u007F-\u009F]/;
const ID_RE = /^tp-[A-Za-z0-9_-]{1,61}$/;

/** 코드포인트 수 — Rust chars().count() 와 같은 단위(서로게이트 쌍 = 1자). */
function cpLen(s: string): number {
  let n = 0;
  for (const _ of s) n++;
  return n;
}

export function parseTeamProposal(item: { kind: string; request_id: string; body: string }): TeamParse {
  if (item.kind !== TEAM_CREATE_KIND) return { ok: false, reason: "팀 만들기 제안 항목이 아닙니다" };
  let v: unknown;
  try {
    v = JSON.parse(item.body);
  } catch {
    return { ok: false, reason: "제안 본문이 JSON 이 아닙니다" };
  }
  if (typeof v !== "object" || v === null || Array.isArray(v)) return { ok: false, reason: "제안 본문 형식 오류" };
  const o = v as Record<string, unknown>;
  const keys = Object.keys(o).sort().join(",");
  if (keys !== "display,id,purpose,v") return { ok: false, reason: "제안 본문의 키가 v·id·display·purpose 넷이 아닙니다" };
  if (o.v !== 1) return { ok: false, reason: "제안 본문 버전이 1 이 아닙니다" };
  const { id, display, purpose } = o;
  if (typeof id !== "string" || typeof display !== "string" || typeof purpose !== "string")
    return { ok: false, reason: "제안 본문 값이 문자열이 아닙니다" };
  if (!ID_RE.test(id) || id !== item.request_id) return { ok: false, reason: "제안 id 가 맞지 않습니다" };
  const dn = cpLen(display);
  if (dn < 1 || dn > TEAM_DISPLAY_MAX || display.trim() !== display || CONTROL.test(display) || INVISIBLE.test(display))
    return { ok: false, reason: "팀 이름 형식 오류" };
  if (!purpose.trim() || cpLen(purpose) > TEAM_PURPOSE_MAX || CONTROL_EXCEPT_NL_TAB.test(purpose) || INVISIBLE.test(purpose))
    return { ok: false, reason: "하는 일 형식 오류" };
  return { ok: true, spec: { id, display, purpose } };
}

/** 카드 본문 — 원문 그대로(요약·가공 없음). */
export function teamCardText(spec: TeamSpec): string {
  return `팀 이름: ${spec.display}\n하는 일:\n${spec.purpose}`;
}

export interface TeamConfirm {
  title: string;
  body: string;
  yesLabel: string;
  noLabel: string;
}

/**
 * 확인 창 문구. firstTeam: true=등재된 팀 0개(첫 팀) · false=이미 팀 있음 · null=모름(명부 판독 실패).
 * 첫 팀이면 본부 대표가 CEO 로 바뀌고 대표 창에 지침이 다시 들어간다 — 그 순간 대표 창에 치던 글과
 * 섞이지 않게 입력을 멈춰 달라고 적는다(반박 M5 · 확인 창 고지로 처리 — 재주입 경로 변경은 별건).
 */
export function buildTeamConfirm(spec: TeamSpec, opts: { firstTeam: boolean | null }): TeamConfirm {
  const ceo =
    opts.firstTeam === true
      ? "· 첫 팀이므로 본부 대표가 CEO 역할로 바뀌고, 만든 직후 대표 창에 지침이 다시 들어갑니다 — 그동안 대표 창 입력을 멈춰 주세요.\n" +
        "  (본부 대표가 아직 시작 전이면 CEO 전환은 나중에 명령 팔레트에서 승인합니다.)\n"
      : opts.firstTeam === null
        ? "· 첫 팀이면 본부 대표가 CEO 역할로 바뀌고, 만든 직후 대표 창에 지침이 다시 들어갑니다 — 그동안 대표 창 입력을 멈춰 주세요.\n"
        : "";
  const body =
    `팀 이름: ${spec.display}\n\n` +
    `하는 일(대표가 오너와 정한 원문 그대로):\n${spec.purpose}\n\n` +
    "만들면 일어나는 일:\n" +
    "· 이 팀 전용 자리가 최대 5개(팀장 1 + 팀원 최대 4) 새로 뜹니다.\n" +
    "· 이 팀 전용 로그인 폴더를 새로 씁니다 — 처음 한 번 로그인이 필요할 수 있습니다.\n" +
    ceo +
    "· 위 '하는 일'은 팀 소개(참고 정보)로 이 팀의 모든 자리에 전달됩니다.\n\n" +
    "[만들기]를 누르면 만듭니다. [나중에]나 창 바깥을 누르면 제안은 그대로 남습니다 — " +
    "만들지 않으려면 카드의 [만들지 않기]를 누르세요.";
  return { title: "새 팀 만들기 — 대표가 제안했습니다", body, yesLabel: "만들기", noLabel: "나중에" };
}

/** 생성 실패 사유 — Tauri 가 `dept-create:<code>:<stderr>` 로 넘긴다(무음 실패 금지). */
export function teamCreateErrorText(e: unknown): string {
  const msg = String(e);
  const m = /^dept-create:(-?\d+):([\s\S]*)$/.exec(msg);
  if (!m) return msg.slice(0, 400);
  const code = parseInt(m[1], 10);
  const detail = m[2].trim().slice(0, 300);
  if (code === 8) return `팀 수 상한(8)에 도달했습니다 — 쓰지 않는 팀을 정리한 뒤 다시 시도하세요. (${detail})`;
  if (code === 2) return `제안 형식 오류이거나 제안이 이미 처리·변경됐습니다 — 카드를 다시 확인하세요. (${detail})`;
  if (code === 5) return `팀 전용 로그인 폴더를 준비하지 못했습니다(계정 격리 불가). (${detail})`;
  return `(${code}) ${detail}`;
}
