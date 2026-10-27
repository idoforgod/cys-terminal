// WP13-TABTITLE-0.14.53 (owner fix F1 · 2026-10-24) — the pane/tab title the user READS.
//
// 【무엇을 고치는가】 오너 실측(OWNER-20261024-seven-fixes.txt §1): 같은 역할 좌석이 둘 생기면(10/24 surface 375·381)
// 탭 제목이 둘 다 `reviewer-codex-codex · cys` 로 똑같아 **어느 창을 닫아야 하는지 화면에서 알 수 없었다**.
// ⇒ 제목 앞에 좌석 번호를 붙이고(`[381] …`), 같은 역할이 둘 이상이면 그 수를 표시한다.
//
// ★표시 전용이다 — 데몬에 저장된 제목(`surface.title`)은 한 글자도 바뀌지 않는다. 이유는 측정된 것이 넷이다
// (DESIGN.md §2.1): ⒜ `cys.rs:18962 workflow_title` 의 고정 검체 14건이 깨진다 ⒝ `cys list`·`cys status` 의 칸이
// 바뀐다 ⒞ `recall.lines.title`(스크롤백 줄마다 붙는 귀속)이 오염된다 ⒟ 저장된 `[42] surface 42` 는
// `isAutoTitle` 에 걸리지 않아 그 pane 이 **현재 경로 표시를 잃는다**.
// ★에이전트가 쓰는 OSC 0/2 제목(탭 점 문법이 읽는 것)과는 **다른 칸**이다 — 이 모듈은 그것을 보지도 않는다.
//
// ⛔이 모듈의 모든 함수는 **순수·전체 함수**다(throw 없음). 호출처가 3초 갱신 루프 안이라(main.ts refreshPaneTitles)
// 여기서 예외가 터지면 그 틱의 남은 pane 제목이 갱신되지 않는다 — 오너 치명위험 ④('모든 pane 백지')의 입구다.
// 그래서 숫자·목록·개수는 전부 방어적으로 좁히고, 못 믿을 입력은 **표시를 줄이는 쪽**으로 떨어진다(표시가 사라질 뿐
// 어떤 경로도 멈추지 않는다).

/** 자동 제목인가 — 기본 자동 제목(`""`·`surface N`)이면 pane 은 제목 대신 현재 경로를 보여준다(종전 규칙 그대로). */
export const isAutoTitle = (t: string | null | undefined): boolean => !t || /^surface \d+$/.test(t);

/** `[381] ` — 쓸 수 있는 좌석 번호일 때만. 음수·소수·NaN·문자열·객체는 번호가 아니다(표시 없음). */
export const surfaceTag = (sid: unknown): string =>
  typeof sid === "number" && Number.isSafeInteger(sid) && sid >= 0 ? `[${sid}] ` : "";

/** 같은 소켓의 좌석 목록에서 역할별 **산 좌석** 수. 종료 좌석은 중복이 아니다(이미 ` [exited]` 가 붙는다). */
export const dupRoleCounts = (surfaces: unknown): Map<string, number> => {
  const out = new Map<string, number>();
  if (!Array.isArray(surfaces)) return out;
  for (const s of surfaces) {
    const row = s as { role?: unknown; exited?: unknown } | null | undefined;
    if (!row || typeof row !== "object") continue;
    if (row.exited === true) continue;
    const role = typeof row.role === "string" ? row.role : "";
    if (!role) continue;
    out.set(role, (out.get(role) ?? 0) + 1);
  }
  return out;
};

/**
 * 탭에 쓰는 문자열. `[번호] 제목` + (같은 역할이 둘 이상이면) ` (수)`.
 * ASCII 만 덧붙인다 — 비ASCII 글리프는 Windows 렌더링이 이 레인에서 미측정이다(헤드 결정 Q3).
 */
export const paneTitle = (
  sid: unknown,
  title: string | null | undefined,
  liveCwd?: string | null,
  dupCount?: unknown,
): string => {
  const base = isAutoTitle(title) ? liveCwd || "…" : (title as string);
  const dup =
    typeof dupCount === "number" && Number.isSafeInteger(dupCount) && dupCount >= 2 ? ` (${dupCount})` : "";
  return `${surfaceTag(sid)}${base}${dup}`;
};

/** Labels describe their evidence; missing observations never inherit launch metadata. */
export function paneDetails(value: unknown): string {
  const row = value && typeof value === "object" ? value as Record<string, unknown> : {};
  const text = (v: unknown) => typeof v === "string" && v ? v : "확인 불가";
  const lines = [`이름: ${text(row.title)}`, `시작 경로: ${text(row.cwd)}`, `관측된 셸 경로: ${text(row.effective_cwd)}`];
  const account = row.account && typeof row.account === "object" ? row.account as Record<string, unknown> : {};
  if (account.state === "mismatch") {
    lines.push("계정: 확인되지 않음 (관측 경로와 기록 경로 불일치)",
      `관측 프로필 경로 (계정 신원 아님): ${text(account.profile)}`,
      `기록 프로필 경로 (계정 신원 아님): ${text(account.recorded_profile)}`);
  }
  return lines.join("\n");
}
