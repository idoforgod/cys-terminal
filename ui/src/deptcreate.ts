// U17 팀 직접 만들기 — 확인 창 문구·판정 순수 모듈 (DOM·Tauri·저장소 무관).
//
// 오너 지시(2026-09-23): "팀을 직접 만드는 메뉴는 전문가용 칸 안으로. 그 메뉴로 만들 때는 만들기 전에
// 확인 창이 한 번 뜨게." 종전 ＋부서 버튼은 확인 없이 곧바로 새 데몬을 띄웠고, 카탈로그가 없는 설치본에서는
// **클릭 1회 = 즉시 생성**(CEO 승격·티켓·편성 최대 5석 포함)이었다. main.ts 는 배선(조회·확인 창·생성)만 하고,
// 확인 창이 **무엇을 말하는지**는 여기서 정한다(purgeconfirm.ts·resetconfirm.ts 와 같은 분리 관례).
//
// ★확인 창이 말해야 하는 것(설계 §3 U17 · 반박 D2·D7·D10) — 근거는 cysjavis-pack/bin/cys-dept:
//   · 이름 — 카탈로그 표시명, 번호 팀은 예상 이름(레지스트리의 빈 가장 작은 dept-N; 확정은 백엔드).
//   · 자리 **최대** 5개 — 편성 로스터(javis_formation.py REQUIRED_ROLES). 외부 세션 역할은 env 로 빠질
//     수 있으므로(effective_required_roles) "최대" 다(반박 D10). 드리프트 핀이 두 목록을 대조한다.
//   · 작업 폴더 — 카탈로그 cwd 원문, 번호 팀은 홈 폴더(CYS_DEPT_CWD 미지정 시).
//   · 첫 로그인 — 팀 전용 설정 폴더를 새로 쓰면 macOS Keychain 이 폴더 경로 단위라 /login 1회가 필요할 수 있다.
//   · CEO 전환 — 첫 팀이면 본부 대표가 CEO 로 바뀐다. 본부 대표가 아직 부트 전이면 보류(PENDING)되므로
//     **조건형**으로만 말한다(반박 D7). 레지스트리를 못 읽었으면 "첫 팀이면" 으로만.
//   · 이미 등재된 팀(mission_key 일치 → cys-dept REUSE)은 "다시 열기" 이되, 꺼져 있으면 데몬을 다시 켜고
//     자리를 다시 띄운다는 사실을 숨기지 않는다(REUSE_DEAD 는 NEW 와 같은 부작용 — 반박 D2).
//
// ★이 모듈의 불변식(usagewiring.test.ts 가 핀으로 고정): 최상위 부수효과 0 · 구형 WKWebView 비호환 문법 0.

/** 부서 상비 편성 로스터 — javis_formation.py REQUIRED_ROLES 와 같아야 한다(deptcreate.test.ts 드리프트 핀). */
export const DEPT_SEAT_ROLES = ["master", "cso", "worker", "reviewer-gemini", "reviewer-codex"] as const;

export interface DeptCatalogEntry {
  display?: string;
  mission_key?: string;
  cwd?: string;
  account?: string;
}
export interface DeptCatalog {
  departments?: Record<string, DeptCatalogEntry | null | undefined>;
}
export interface DeptRegistryEntry {
  socket?: string;
  mission_key?: string;
}
export interface DeptRegistry {
  depts?: Record<string, DeptRegistryEntry | null | undefined>;
}

export interface DeptCreatePlanInput {
  /** 카탈로그 키. undefined = 번호 팀(레거시 `cys-dept allocate`). */
  key: string | undefined;
  catalog: DeptCatalog | null;
  /** list_depts 결과. null = 조회 실패(REUSE·첫 팀·예상 이름을 단정하지 않는다). */
  registry: DeptRegistry | null;
  /** 카탈로그 팀이 모두 이미 열려 있어 번호 팀으로 가는가. */
  allRunning?: boolean;
  /** 카탈로그 파일이 있는데 읽지 못해 번호 팀으로 가는가. */
  catalogUnreadable?: boolean;
  /** exit 3(카탈로그 부재) 재확인 — 원래 고른 팀 이름. 있으면 제목·버튼이 재확인용으로 바뀐다. */
  fallbackFrom?: string;
}
export interface DeptCreatePlan {
  title: string;
  body: string;
  yesLabel: string;
  noLabel: string;
  reuse: boolean;
  legacy: boolean;
  /** 확인 창·후속 안내에 쓰는 팀 이름(카탈로그 표시명 · 키 · 번호 팀 예상 이름). */
  displayName: string;
}

const NAME_MAX = 80; // 표시명이 비정상적으로 길면 자른다(확인 창 레이아웃 보호 · 내용은 textContent 라 안전)
const clip = (s: string): string => (s.length > NAME_MAX ? s.slice(0, NAME_MAX - 1) + "…" : s);
const str = (v: unknown): string => (typeof v === "string" ? v : "");

/** 레지스트리의 비어 있는 가장 작은 `dept-N`(백엔드 allocate 의 lowest-unused 예측 — 확정은 백엔드). */
export function predictLegacyDeptName(registry: DeptRegistry | null): string | null {
  if (!registry) return null;
  const used = new Set<number>();
  for (const k of Object.keys(registry.depts ?? {})) {
    const m = /^dept-(\d+)$/.exec(k);
    if (m) used.add(parseInt(m[1], 10));
  }
  for (let n = 1; n <= used.size + 1; n++) if (!used.has(n)) return `dept-${n}`;
  return `dept-${used.size + 1}`;
}

/** 카탈로그 키의 mission_key 가 이미 레지스트리에 있으면 그 등재 이름(cys-dept create 의 REUSE 술어와 같다). */
export function reuseRegistryName(key: string, catalog: DeptCatalog | null, registry: DeptRegistry | null): string | null {
  const mk = str(catalog?.departments?.[key]?.mission_key);
  if (!mk || !registry) return null;
  for (const [name, e] of Object.entries(registry.depts ?? {})) if (e && e.mission_key === mk) return name;
  return null;
}

const SEATS_LINE =
  `자리: 최대 ${DEPT_SEAT_ROLES.length}개 — 부서장 1 + 팀원 최대 ${DEPT_SEAT_ROLES.length - 1}(CSO·워커·검토 2). ` +
  "부서장 자리가 먼저 뜨고, 팀원 자리는 필요한 프로그램(claude·agy·codex)이 설치돼 있으면 자동으로 채워집니다.";
const CEO_COND = "(본부 대표가 아직 시작 전이면 나중에 명령 팔레트에서 승인합니다. 팀이 0개가 되면 되돌아갑니다.)";
const CANCEL_LINE = "[취소]를 누르면 아무것도 만들지 않습니다.";

function ceoLine(registry: DeptRegistry | null): string | null {
  if (!registry) return `본부 대표: 첫 팀이면 CEO 역할로 바뀝니다 ${CEO_COND}`;
  return Object.keys(registry.depts ?? {}).length === 0 ? `본부 대표: CEO 역할로 바뀝니다 ${CEO_COND}` : null;
}

/** 확인 창 한 벌. 문구는 사람이 읽는 말로 — 내부 용어(allocate·REUSE·mission_key)는 쓰지 않는다. */
export function buildDeptCreatePlan(inp: DeptCreatePlanInput): DeptCreatePlan {
  const lines: string[] = [];
  const registry = inp.registry;

  // ── 번호 팀(레거시) ──
  if (inp.key === undefined) {
    const predicted = predictLegacyDeptName(registry);
    if (inp.fallbackFrom)
      lines.push(`고른 팀 '${clip(inp.fallbackFrom)}'의 카탈로그를 찾지 못했습니다. 대신 번호로 새 팀을 만들까요?`, "");
    else if (inp.catalogUnreadable) lines.push("팀 목록 파일(카탈로그)을 읽지 못해 번호로 새 팀을 만듭니다.", "");
    else if (inp.allRunning) lines.push("카탈로그의 팀이 모두 이미 열려 있어 번호로 새 팀을 만듭니다.", "");
    lines.push(
      predicted ? `만들 팀: 새 번호 팀 — 예상 이름 ${predicted} (만들 때 확정)` : "만들 팀: 새 번호 팀 (이름은 만들 때 정해집니다)",
      SEATS_LINE,
      "작업 폴더: 홈 폴더(따로 지정하지 않은 경우)",
      "계정: 기본 계정의 팀 전용 설정 폴더를 새로 씁니다 — 처음 한 번 로그인이 필요할 수 있습니다.",
    );
    const ceo = ceoLine(registry);
    if (ceo) lines.push(ceo);
    lines.push("", CANCEL_LINE);
    return {
      title: inp.fallbackFrom ? "번호 팀으로 만들까요?" : "팀 만들기 확인",
      body: lines.join("\n"),
      yesLabel: inp.fallbackFrom ? "번호 팀 만들기" : "만들기",
      noLabel: "취소",
      reuse: false,
      legacy: true,
      displayName: predicted ?? "새 번호 팀",
    };
  }

  // ── 카탈로그 팀 ──
  const key = inp.key;
  const d = inp.catalog?.departments?.[key] ?? null;
  const name = clip(str(d?.display) || key);
  const cwd = str(d?.cwd);
  const account = str(d?.account);
  const reuseName = reuseRegistryName(key, inp.catalog, registry);
  if (reuseName) {
    lines.push(
      `이미 있는 팀 '${name}'(${reuseName})을 엽니다.`,
      `꺼져 있으면 다시 켜고 자리(최대 ${DEPT_SEAT_ROLES.length}개)를 다시 띄웁니다 — 팀을 새로 하나 더 만들지는 않습니다.`,
      `작업 폴더: ${cwd || "(카탈로그에 지정 없음)"}`,
      "",
      "[취소]를 누르면 아무것도 바꾸지 않습니다.",
    );
    return { title: "팀 다시 열기", body: lines.join("\n"), yesLabel: "열기", noLabel: "취소", reuse: true, legacy: false, displayName: name };
  }
  lines.push(
    `만들 팀: ${name}`,
    SEATS_LINE,
    `작업 폴더: ${cwd || "(카탈로그에 지정 없음)"}`,
    account
      ? `계정: ${account} — 이 팀 전용 설정 폴더를 새로 쓰는 경우 처음 한 번 로그인이 필요할 수 있습니다.`
      : "계정: (카탈로그에 지정 없음) — 팀 전용 설정 폴더를 새로 쓰는 경우 처음 한 번 로그인이 필요할 수 있습니다.",
  );
  const ceo = ceoLine(registry);
  if (ceo) lines.push(ceo);
  lines.push("", CANCEL_LINE);
  return { title: "팀 만들기 확인", body: lines.join("\n"), yesLabel: "만들기", noLabel: "취소", reuse: false, legacy: false, displayName: name };
}
