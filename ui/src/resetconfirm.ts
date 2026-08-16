// 완전 초기화(팩토리 리셋) 확인 모달의 순수 판정·문구 조립 — main.ts factoryResetConfirmModal 이
// 배선만 한다. purgeconfirm.ts 규약 계승(실사고 2026-07-16: 비활성 버튼 무반응 오인·macOS 자동
// 대문자화 재교정·불일치 사유 무표시) — 판정을 순수 함수로 고정해 결정론 회귀 테스트로 잠근다.
// 입력 자동교정 차단은 PURGE_INPUT_GUARDS(purgeconfirm.ts)를 재사용한다(이원 정의 금지).

/// CLI `cys factory-reset` 의 FACTORY_RESET_PHRASE 와 동일 문자열 계약 — 갈리면 문서·안내가
/// 서로 다른 문구를 요구하게 된다.
export const RESET_PHRASE = "완전 초기화";

// ★P2-1: 확인 문구 정규화 — CLI `normalize_confirm_phrase` 와 **같은 규칙**이어야 한다
// (한쪽만 관용하면 같은 입력이 앱에선 되고 터미널에선 안 되는 계약 분열이 생긴다).
// 따옴표 동반 복사·NFD 자모(macOS)·NBSP/전각 공백·ZWSP 는 육안상 같으므로 같게 받는다.
export function normalizePhrase(input: string): string {
  let t = input.normalize("NFC").trim();
  // 양끝 짝지은 따옴표 제거(곡선 따옴표 포함) — 프롬프트·모달 안내문에서 그대로 복사한 경우.
  const pairs: [string, string][] = [
    ['"', '"'],
    ["'", "'"],
    ["\u201C", "\u201D"],
    ["\u2018", "\u2019"],
  ];
  let stripped = true;
  while (stripped) {
    stripped = false;
    for (const [a, b] of pairs) {
      if (t.length >= 2 && t.startsWith(a) && t.endsWith(b)) {
        t = t.slice(a.length, t.length - b.length);
        stripped = true;
        break;
      }
    }
  }
  return t
    .replace(/[\u200B\u200C\u200D\uFEFF]/g, "") // 폭 없는 문자 제거
    .split(/\s+/) // NBSP·전각 공백 포함 모든 공백류를 하나로
    .filter(Boolean)
    .join(" ");
}

// 확인 입력이 문구와 일치하는가(위 정규화 후 비교).
export function resetPhraseMatches(input: string): boolean {
  return normalizePhrase(input) === normalizePhrase(RESET_PHRASE);
}

// 불일치 시 실시간 힌트(빈 입력·일치 = 빈 문자열) — 무엇이 어긋났는지 보여준다.
export function resetMismatchHint(input: string): string {
  const v = normalizePhrase(input);
  if (!v || resetPhraseMatches(input)) return "";
  return `입력 "${v}" 는 "${RESET_PHRASE}" 와 다릅니다 — 띄어쓰기까지 그대로 입력하세요.`;
}

// 프리뷰 격리 총량 사람용 표기(purgeDept의 sizeHuman과 동일 임계) — 모달 고지용 순수 함수.
export function resetSizeHuman(bytes: number): string {
  return bytes >= 1e9
    ? (bytes / 1e9).toFixed(1) + " GB"
    : bytes >= 1e6
      ? (bytes / 1e6).toFixed(1) + " MB"
      : bytes >= 1e3
        ? (bytes / 1e3).toFixed(1) + " KB"
        : bytes + " B";
}

export type ResetPreviewItem = {
  path: string;
  label: string;
  size_bytes: number;
  outside_state?: boolean;
};

export type ResetPreview = {
  quarantineCount: number;
  totalBytes: number;
  keptCount: number;
  stripProfiles: number;
  trashDir: string;
  items?: ResetPreviewItem[];
  reportOnly?: string[];
  liveSessions?: number;
  deptCount?: number;
  interruptedPrior?: string[];
};

// ★P0-2: 사용자 폴더(=cys 상태 루트 밖) 안에서 사라지는 항목은 **따로 맨 앞에** 보여준다.
// 종전 모달은 "34건 · 2.2GB" 숫자 한 줄뿐이라, 프로젝트 폴더 안 작업기억 279MB가 이름 없이
// 사라졌다. 같은 기능의 CLI `--plan` 은 전 경로를 찍는데 GUI 사용자만 눈이 가려진 비대칭이었다.
export function resetOutsideItems(info: ResetPreview): ResetPreviewItem[] {
  return (info.items ?? [])
    .filter((i) => i.outside_state)
    .sort((a, b) => b.size_bytes - a.size_bytes);
}

// 큰 항목부터 상위 N건(모달 본문용) — 나머지는 "…외 N건"으로 접는다.
export function resetTopItems(info: ResetPreview, n = 6): ResetPreviewItem[] {
  return [...(info.items ?? [])].sort((a, b) => b.size_bytes - a.size_bytes).slice(0, n);
}

// 프리뷰 → 모달 고지문(순수 조립 — 테스트로 문구 계약을 잠근다). 정직 고지 규약:
// 무엇이 사라지고 무엇이 보존되며 복구 경로가 뭔지 명시(비가역처럼 보이는 오도 금지).
export function resetNoticeLines(info: ResetPreview): string[] {
  const lines: string[] = [];

  // ① 되돌릴 수 없는 즉시 피해를 맨 위에 — '↻ 재시작' 툴팁도 drain·미저장 손실을 고지한다.
  const live = info.liveSessions ?? 0;
  const depts = info.deptCount ?? 0;
  if (live > 0 || depts > 0) {
    lines.push(
      `지금 실행 중인 세션 ${live}개·부서 ${depts}개가 저장(drain) 신호 없이 즉시 종료됩니다 — 중요한 작업은 먼저 마무리하세요.`,
    );
  } else {
    lines.push("실행 중인 세션이 저장 신호 없이 즉시 종료됩니다 — 중요한 작업은 먼저 마무리하세요.");
  }

  lines.push(
    `격리 대상 ${info.quarantineCount}건 · 총 ${resetSizeHuman(info.totalBytes)} — 모든 부서·세션·대화기억·작업기억·설정이 초기화됩니다.`,
  );

  // ② 사용자 폴더 안에서 사라지는 것은 경로를 그대로 노출한다(승인의 전제).
  const outside = resetOutsideItems(info);
  if (outside.length > 0) {
    lines.push(
      "⚠ 내 폴더 안에서 사라지는 항목:\n" +
        outside.map((i) => `   • ${i.path} (${resetSizeHuman(i.size_bytes)})`).join("\n"),
    );
  }

  // ③ 큰 항목 미리보기 — 무엇이 큰 비중인지 승인 전에 보이게.
  const top = resetTopItems(info);
  if (top.length > 0) {
    const rest = (info.items ?? []).length - top.length;
    lines.push(
      "주요 격리 항목:\n" +
        top.map((i) => `   • ${i.path} (${resetSizeHuman(i.size_bytes)})`).join("\n") +
        (rest > 0 ? `\n   …외 ${rest}건` : ""),
    );
  }

  lines.push(
    `에이전트 전용 계정 로그인(~/.cys/claude*)이 격리되어 재로그인이 필요합니다. Claude Code 연결 고리(훅)는 ${info.stripProfiles}개 프로필에서 해제됩니다.`,
  );

  lines.push(
    info.keptCount > 0
      ? `보존: 라이선스·직접 만든 설정(~/.cys/local)·직접 넣은 파일 등 ${info.keptCount}건은 삭제되지 않습니다.`
      : "보존: 라이선스·직접 만든 설정·직접 넣은 파일은 삭제되지 않습니다.",
  );

  // ④ 자동으로 정리하지 않는 것 — 사용자가 직접 판단해야 하는 잔재.
  const ro = info.reportOnly ?? [];
  if (ro.length > 0) {
    lines.push("자동 정리하지 않음 — 직접 확인하세요:\n" + ro.map((r) => `   • ${r}`).join("\n"));
  }

  // ⑤ 이전 중단 흔적 — 격리 폴더가 둘이 되는 혼선의 예방.
  const prior = info.interruptedPrior ?? [];
  if (prior.length > 0) {
    lines.push(
      "이전에 중단된 초기화 흔적이 있습니다(복구 지도는 각 폴더의 journal.ndjson):\n" +
        prior.map((p) => `   • ${p}`).join("\n"),
    );
  }

  lines.push(
    "복구: 즉시 삭제가 아니라 격리 보관되어 되돌릴 수 있습니다 — 완료 후 안내되는 폴더에서 `cys factory-reset --undo <폴더>`. 격리본은 약 14일 뒤 정리 작업에서 소거될 수 있습니다.",
  );
  lines.push("완료 후 앱을 다시 실행하면 설치 온보딩이 처음부터 시작됩니다.");
  lines.push(`계속하려면 아래에 "${RESET_PHRASE}" 를 정확히 입력하세요.`);
  return lines;
}

export type ResetResult = {
  ok?: boolean;
  trash_dir?: string;
  moved?: number;
  failed?: { path: string; error: string }[];
  deferred?: { path: string; error: string }[];
  revived_warning?: string | null;
  skipped_absent?: number;
  manifest_written?: boolean;
  report_path?: string;
  interrupted_prior?: string[];
};

// ★P0-4: 부분 실패인데 제목이 "완전 초기화 완료"라 실패가 모달 뒤에 가려지고, 앱을 끄면
// 60초 토스트와 함께 영영 사라졌다. 제목·본문을 결과에서 파생시켜 실패를 정면에 세운다.
export function resetResultTitle(rep: ResetResult): string {
  const partial = rep.ok === false || (rep.failed ?? []).length > 0 || !!rep.revived_warning;
  return partial ? "완전 초기화 부분 완료 — 정리되지 않은 항목이 있습니다" : "완전 초기화 완료";
}

export function resetResultBody(rep: ResetResult): string {
  const parts: string[] = [];
  const failed = rep.failed ?? [];
  const deferred = rep.deferred ?? [];

  if (rep.revived_warning) parts.push(`⚠ ${rep.revived_warning}`);
  if (failed.length > 0) {
    parts.push(
      "정리되지 않은 항목 — 직접 확인하세요:\n" +
        failed.map((f) => `   • ${f.path}: ${f.error}`).join("\n"),
    );
  }
  // 예고 건수와 완료 건수의 차이를 분해해 설명한다(설명 없는 불일치 금지).
  parts.push(
    `이동 ${rep.moved ?? 0}건 · 이미 없음 ${rep.skipped_absent ?? 0}건 · 이연 ${deferred.length}건 · 실패 ${failed.length}건`,
  );
  if (deferred.length > 0) {
    parts.push(
      `앱이 사용 중이라 ${deferred.length}건(화면 저장값)은 옮기지 못했습니다 — 앱을 종료한 뒤 외부 터미널에서 \`cys factory-reset\` 을 한 번 더 실행하면 정리됩니다.`,
    );
  }
  if (rep.manifest_written === false) {
    parts.push("⚠ 복구 지도(manifest.json)를 쓰지 못했습니다 — journal.ndjson 이 유일한 지도입니다.");
  }
  if (rep.trash_dir) parts.push(`격리 위치: ${rep.trash_dir}`);
  if (rep.report_path) parts.push(`결과 요약 파일: ${rep.report_path} (이 창을 닫아도 남습니다)`);
  if (rep.trash_dir) parts.push(`되돌리려면: cys factory-reset --undo ${rep.trash_dir}`);
  parts.push(
    "지금 앱을 종료하세요. 다시 실행하면 설치 온보딩이 처음부터 시작됩니다.\n(종료 전까지는 데몬·부서 생성이 차단됩니다 — 반쪽 상태에서의 재생성 방지)",
  );
  return parts.join("\n\n");
}
