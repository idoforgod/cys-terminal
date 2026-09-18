// deptlabel.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0). WP-10 잠금 픽스처 대응.
//
// '＋부서' 클릭 후 부서 데몬 준비 동안 탭 라벨이 진행 상태를 명시하는지(멈춘 줄 오해 방지),
// 확정 후엔 실제 표시명으로 바뀌는지 결정론으로 검증한다.
import { describe, it, expect } from "bun:test";
import {
  deptPlaceholderLabel,
  DEPT_PENDING_LABEL,
  deptSlugOfSocket,
  pickDeptWorkspace,
  isActiveDeptSocket,
  DEFAULT_SOCKET_KEY,
  deptNameFromSocket,
  deptLaunchName,
  restoreLaunchDecision,
  resolveDeptLaunchName,
  deptCloseAfterStop,
  tombReapErrorToast,
} from "./deptlabel";

// ★C1(2026-09-17 3라운드 · codex major) — 복원 경로의 묘비 검사가 launch 와 **다른 해소기**를 써서 묘비를 비켜 갔다.
// 종전 묘비 검사 = deptNameFromSocket(소켓 규약 파서)만 · launch = deptLaunchName(파서 → 레지스트리 키).
// 차집합(파서 null · 레지스트리 키 있음)에 묘비가 걸리지 않아 `cys-dept launch` 가 묘비를 지우고 지운 부서를 되살렸다.
// 이 핀은 **묘비가 있으면 launch=null** 을 세 소켓 표기(표준 · 대문자 PIPE · 레거시 파일경로형)에서 고정한다 —
// 해소기를 다시 deptNameFromSocket 으로 되돌리면 뒤 두 표기가 빨간불이 된다(3라운드 변이 대조).
describe("restoreLaunchDecision — 묘비 검사와 launch 는 같은 이름을 본다(C1)", () => {
  const REG = {
    "dept-1": { socket: "/Users/x/.local/state/cys-dept-dept-1/cys.sock" },
    "dept-2": { socket: "\\\\.\\pipe\\cys-dept-dept-2" },
    "dept-3": { socket: "C:\\Users\\x\\.local\\state\\cys-dept-dept-3\\cys.sock" },
  };
  const TOMBS = new Set(["dept-1", "dept-2", "dept-3"]);
  const STD = REG["dept-1"].socket; // 표준(unix 규약) — 파서가 읽는다
  const PIPE_UPPER = "\\\\.\\PIPE\\cys-dept-dept-2"; // 대문자 PIPE — 파서 null · sameSocket 은 같은 파이프
  const LEGACY = REG["dept-3"].socket; // 레거시 파일경로형 — 파서 null · 레지스트리 키로만 닿는다

  it("전제: 파서만으로는 대문자 PIPE·레거시 표기가 null 이다(= 옛 묘비 검사의 구멍)", () => {
    expect(deptNameFromSocket(STD)).toBe("dept-1");
    expect(deptNameFromSocket(PIPE_UPPER)).toBeNull();
    expect(deptNameFromSocket(LEGACY)).toBeNull();
    // 반면 launch 해소기는 셋 다 이름을 구한다 — 이 차집합이 묘비를 우회하던 경로다.
    expect(deptLaunchName(PIPE_UPPER, REG)).toBe("dept-2");
    expect(deptLaunchName(LEGACY, REG)).toBe("dept-3");
  });

  it("★묘비가 있으면 세 표기 모두 launch=null · reason=tombstone(지운 부서는 되살리지 않는다)", () => {
    for (const socket of [STD, PIPE_UPPER, LEGACY]) {
      expect(restoreLaunchDecision({ socket, regDepts: REG, tombstones: TOMBS })).toEqual({
        launch: null,
        reason: "tombstone",
      });
    }
  });

  it("★codex 반례 그대로: 등재 `\\\\.\\pipe\\cys-dept-dept-2` · 저장 탭 `\\\\.\\PIPE\\…` · 묘비 dept-2 → launch 없음", () => {
    // main.ts 의 regDepts 형태(display_name 동반)를 그대로 — 표시명 '연구부' 는 어떤 경로로도 launch 인자가 되지 않는다.
    const regWithDisplay = { "dept-2": { socket: "\\\\.\\pipe\\cys-dept-dept-2", display_name: "연구부" } };
    const d = restoreLaunchDecision({
      socket: "\\\\.\\PIPE\\cys-dept-dept-2",
      regDepts: regWithDisplay,
      tombstones: new Set(["dept-2"]),
    });
    expect(d.launch).toBeNull();
    expect(d.reason).toBe("tombstone");
  });

  it("묘비가 없으면(조회 성공 · 빈 집합) launch 는 deptLaunchName 과 같은 이름(등재 키 우선 · 파서 폴백)", () => {
    const none = new Set<string>();
    expect(restoreLaunchDecision({ socket: STD, regDepts: REG, tombstones: none })).toEqual({ launch: "dept-1", reason: "launch" });
    expect(restoreLaunchDecision({ socket: PIPE_UPPER, regDepts: REG, tombstones: none })).toEqual({ launch: "dept-2", reason: "launch" });
    expect(restoreLaunchDecision({ socket: LEGACY, regDepts: REG, tombstones: none })).toEqual({ launch: "dept-3", reason: "launch" });
    // 다른 부서의 묘비는 이 탭에 영향이 없다.
    expect(restoreLaunchDecision({ socket: STD, regDepts: REG, tombstones: new Set(["dept-9"]) }).launch).toBe("dept-1");
  });

  it("이름을 못 구하면 launch=null · reason=unnamed(묘비 집합과 무관 · 표시명은 받지도 않는다)", () => {
    const orphan = "C:\\Users\\x\\elsewhere\\cys.sock";
    expect(restoreLaunchDecision({ socket: orphan, regDepts: REG, tombstones: TOMBS })).toEqual({ launch: null, reason: "unnamed" });
    expect(restoreLaunchDecision({ socket: LEGACY, regDepts: null, tombstones: TOMBS })).toEqual({ launch: null, reason: "unnamed" }); // 레지스트리 미조회
    expect(restoreLaunchDecision({ socket: undefined, regDepts: REG, tombstones: TOMBS })).toEqual({ launch: null, reason: "unnamed" });
  });

  // ★D2(2026-09-17 4라운드 · codex 2차 ⑤): 3라운드까지는 이 자리가 fail-open("null = 묘비 없음 → 켠다")을 정답으로
  //   고정했다. 그 launch 는 성공 말미에 묘비를 지우므로 묘비 RPC timeout 한 번이 지운 부서를 되살렸다(비가역).
  //   Rust spawn_org_restore 와 같은 방향(죽은 부서 재기동 보류)으로 뒤집는다 — 되돌리면 이 핀이 red.
  it("★D2: 묘비 조회 실패(null/undefined)는 세 표기 모두 launch=null · reason=tombstone-unknown(죽은 부서 자동 launch 보류)", () => {
    for (const socket of [STD, PIPE_UPPER, LEGACY]) {
      expect(restoreLaunchDecision({ socket, regDepts: REG, tombstones: null })).toEqual({ launch: null, reason: "tombstone-unknown" });
      expect(restoreLaunchDecision({ socket, regDepts: REG, tombstones: undefined })).toEqual({ launch: null, reason: "tombstone-unknown" });
    }
    // 결측 ≠ 빈 집합: 조회에 성공했고 묘비가 0개면 켠다(위 케이스와 짝 — 둘을 뭉치면 fail-closed 가 '영구 idle' 이 된다).
    expect(restoreLaunchDecision({ socket: STD, regDepts: REG, tombstones: new Set() })).toEqual({ launch: "dept-1", reason: "launch" });
    // 이름조차 못 구하면 unnamed 가 앞선다 — 어차피 켤 수 없고 사용자가 할 일(소켓·등재 확인)이 다르다.
    expect(restoreLaunchDecision({ socket: "C:\\Users\\x\\elsewhere\\cys.sock", regDepts: REG, tombstones: null })).toEqual({ launch: null, reason: "unnamed" });
    // 묘비가 실제로 있으면 결측 여부와 무관하게 tombstone 이 앞선다(호출측이 탭 드롭·정리로 갈라 처리한다).
    expect(restoreLaunchDecision({ socket: STD, regDepts: REG, tombstones: TOMBS }).reason).toBe("tombstone");
  });
});

// ★K2-05(2026-09-17 한글 사용자명 감사) — `cys-dept launch <name>` 인자의 단일 산출 지점 핀.
// 종전 `deptNameFromSocket(ws.socket) ?? ws.name` 은 소켓 역산이 null 이면 한글 표시명을 부서명으로
// 넘겨 exit 2("부적격 부서명") 를 냈다. 이 함수는 표시명을 **받지 않는다** — 소켓 → 레지스트리 키 → null.
describe("deptLaunchName — 표시명은 어떤 경우에도 launch 인자가 되지 않는다", () => {
  const REG = {
    "dept-1": { socket: "/Users/x/.local/state/cys-dept-dept-1/cys.sock", display_name: "영업부(한국)" },
    "dept-2": { socket: "\\\\.\\pipe\\cys-dept-dept-2", display_name: "미래연구부" },
    // ★2라운드 검증 적발: 역산 불가 소켓 항목에도 display_name(한글)을 둔다 — 변이 `return e.display_name ?? name` 이
    //   이 픽스처 없이는 22/22 를 살아 통과했다(핀이 표제 주장을 실제로 잠그지 못했다).
    "dept-3": { socket: "C:\\Users\\x\\.local\\state\\cys-dept-dept-3\\cys.sock", display_name: "총무부" },
  };
  it("① 규약 소켓은 레지스트리 없이도 소켓에서 역산(종전 동작 보존)", () => {
    expect(deptLaunchName("/Users/x/.local/state/cys-dept-dept-1/cys.sock", null)).toBe("dept-1");
    expect(deptLaunchName("\\\\.\\pipe\\cys-dept-sales", undefined)).toBe("sales");
  });
  it("① 한글 홈(NFC·NFD)·공백 경로에서도 부서명은 소켓 슬러그 그대로", () => {
    const nfc = "/Users/x/Desktop/홍길동/.local/state/cys-dept-dept-1/cys.sock";
    const nfd = "/Users/x/Desktop/" + "홍길동".normalize("NFD") + "/.local/state/cys-dept-dept-1/cys.sock";
    expect(nfd).not.toBe(nfc); // 픽스처가 실제로 NFD 인지
    expect(deptLaunchName(nfc, null)).toBe("dept-1");
    expect(deptLaunchName(nfd, null)).toBe("dept-1");
    expect(deptLaunchName("/Users/x/영업 팀/.local/state/cys-dept-sales-kr/cys.sock", null)).toBe("sales-kr");
  });
  it("② 역산 불가 소켓(Windows 파일경로형)은 레지스트리 **키** 로 — display_name 이 아니다", () => {
    const sock = "C:\\Users\\x\\.local\\state\\cys-dept-dept-3\\cys.sock";
    expect(deptNameFromSocket(sock)).toBeNull(); // 전제: 파서가 못 읽는 형태
    expect(deptLaunchName(sock, REG)).toBe("dept-3");
    expect(deptLaunchName(sock, REG)).not.toBe(REG["dept-3"].display_name); // 레지스트리 경로에서도 표시명은 돌아오지 않는다
  });
  it("★② D3: 표기만 다른 같은 named pipe(대소문자)는 **등재 키가 파서보다 앞선다**(codex 2차 ⑥ 반례)", () => {
    // 등재 dept-2 · 저장 탭 소켓 `\\.\pipe\cys-dept-DEPT-2` — 3라운드까지는 파서 우선이라 'DEPT-2' 를 돌려줬고
    // `cys-dept launch DEPT-2` 는 slug 충돌(exit 2)로 매 기동 반복 실패 · 묘비 dept-2 도 놓쳤다. 정본은 등재 키다.
    const reg = { "dept-2": { socket: "\\\\.\\pipe\\cys-dept-dept-2" } };
    expect(deptLaunchName("\\\\.\\pipe\\cys-dept-DEPT-2", reg)).toBe("dept-2");
    expect(deptLaunchName("\\\\.\\PIPE\\cys-dept-dept-2", reg)).toBe("dept-2");
    // 같은 반례에서 묘비 dept-2 는 이제 걸린다(3라운드 판정은 'DEPT-2' 를 보고 놓쳤다).
    expect(restoreLaunchDecision({ socket: "\\\\.\\pipe\\cys-dept-DEPT-2", regDepts: reg, tombstones: new Set(["dept-2"]) })).toEqual({
      launch: null,
      reason: "tombstone",
    });
    // 파서는 폴백이다 — 레지스트리 미조회(null)·미등재(다른 부서만 등재)에서만 쓴다.
    expect(deptLaunchName("\\\\.\\pipe\\cys-dept-DEPT-2", null)).toBe("DEPT-2");
    expect(deptLaunchName("\\\\.\\pipe\\cys-dept-DEPT-2", { "dept-9": { socket: "\\\\.\\pipe\\cys-dept-dept-9" } })).toBe("DEPT-2");
    // `\\?\pipe\` 접두는 파서가 못 읽고 sameSocket 도 다르게 보므로 null — 대소문자 차이만 흡수한다.
    expect(deptLaunchName("\\\\?\\pipe\\cys-dept-dept-2", reg)).toBeNull();
  });
  it("② unix 소켓은 등재 키와 파서가 같은 답이다(바이트 일치) — 등재가 정본이면 등재 키, 없으면 파서", () => {
    const sock = "/Users/x/.local/state/cys-dept-dept-1/cys.sock";
    expect(deptLaunchName(sock, REG)).toBe("dept-1");
    expect(deptLaunchName(sock, null)).toBe("dept-1");
    expect(deptLaunchName(sock, { "dept-9": { socket: "/Users/x/.local/state/cys-dept-dept-9/cys.sock" } })).toBe("dept-1");
  });
  it("③ 어디에도 없으면 null — 한글 display_name 은 결코 돌아오지 않는다", () => {
    const orphan = "C:\\Users\\x\\elsewhere\\cys.sock";
    expect(deptLaunchName(orphan, REG)).toBeNull();
    expect(deptLaunchName(undefined, REG)).toBeNull();
    expect(deptLaunchName("", REG)).toBeNull();
    for (const s of [orphan, REG["dept-1"].socket, REG["dept-2"].socket, REG["dept-3"].socket]) {
      const n = deptLaunchName(s, REG);
      expect(n === null || /^[A-Za-z0-9][A-Za-z0-9_-]{0,39}$/.test(n)).toBe(true); // cys-dept dept_name_ok 집합
    }
  });
  it("레지스트리 항목의 socket 이 비었거나 키가 비어도 매칭하지 않는다", () => {
    expect(deptLaunchName("C:\\x\\cys.sock", { "": { socket: "C:\\x\\cys.sock" }, d: {}, e: undefined })).toBeNull();
  });
});

describe("deptPlaceholderLabel — 부서 제작 중 표시", () => {
  it("pending 부서 탭은 '부서 제작 중' 표시", () => {
    expect(deptPlaceholderLabel({ pending: true, name: "…" })).toContain("부서 제작 중");
  });
  it("확정된 부서는 실제 표시명", () => {
    expect(deptPlaceholderLabel({ pending: false, name: "리서치부" })).toBe("리서치부");
  });
  it("pending 미지정(undefined)은 실제 이름 취급(확정 탭 회귀)", () => {
    expect(deptPlaceholderLabel({ name: "dept-1" })).toBe("dept-1");
  });
  it("pending 라벨은 상수 DEPT_PENDING_LABEL 과 일치", () => {
    expect(deptPlaceholderLabel({ pending: true, name: "무엇이든" })).toBe(DEPT_PENDING_LABEL);
  });
});

// ★결함#4-b/F5 회귀 — 승인 Feed 부서 행 부제. Windows named pipe 에서 모든 부서가 같은
// 부제("pipe")로 접혀 식별이 죽던 결함의 잠금 픽스처.
describe("deptSlugOfSocket — 부서 소켓 → 사람이 읽는 슬러그", () => {
  // 픽스처 경로의 username 은 반드시 **더미 허용 목록**을 쓴다(scripts/secret-scan.sh 의
  // `dummy_user_re` = user|x|youruser|USERNAME|runner|home). 그 외 `/Users/<name>` 은
  // 개인경로로 판정돼 PUBLIC 발행 하드 게이트가 fail-closed 로 막는다(태그 레인 red).
  // 리포 관례는 `/Users/x/` 다(handlers.rs·src-tauri/main.rs 픽스처 동일).
  it("unix 소켓은 부모 디렉터리(부서 슬러그)", () => {
    expect(deptSlugOfSocket("/Users/x/.cys/cys-dept-sales/cysd.sock")).toBe("cys-dept-sales");
  });
  it("★win named pipe 는 마지막 컴포넌트 — 'pipe' 가 아니다(F5 회귀)", () => {
    expect(deptSlugOfSocket("\\\\.\\pipe\\cys-dept-sales")).toBe("cys-dept-sales");
  });
  it("★win named pipe 서로 다른 부서가 서로 다른 부제를 갖는다(식별 복원)", () => {
    const a = deptSlugOfSocket("\\\\.\\pipe\\cys-dept-1");
    const b = deptSlugOfSocket("\\\\.\\pipe\\cys-dept-2");
    expect(a).not.toBe(b);
    expect([a, b]).not.toContain("pipe");
  });
  it("`\\\\?\\pipe\\` 접두 형태도 동일 처리", () => {
    expect(deptSlugOfSocket("\\\\?\\pipe\\cys-dept-hr")).toBe("cys-dept-hr");
  });
  it("컴포넌트가 부족하면 원본을 그대로(정보 은폐 금지)", () => {
    expect(deptSlugOfSocket("cysd.sock")).toBe("cysd.sock");
  });
});

// ★F6①② 회귀 — 승인 Feed 부서 행 '이 부서로 이동' 버튼의 판정.
//
// 왜 이 핀이 생겼나: 판정 자체는 옳게 구현돼 있었지만 `main.ts` 모듈-private 함수라
// 테스트가 닿지 못했다 — 같은 라운드에서 억제 스캔(F4-③)이 바로 그 상태로 649건 green 인 채
// 결함이 되살아나는 것을 부정 대조로 확인했으므로, 순수부를 여기로 빼고 박제한다.
describe("pickDeptWorkspace — 부서 이동 버튼 3갈래 판정", () => {
  const SALES = "/Users/x/.cys/cys-dept-sales/cysd.sock";
  const HR = "/Users/x/.cys/cys-dept-hr/cysd.sock";

  it("ⓐ 정상 부서 탭 → switched(+대상 인덱스)", () => {
    const list = [{ socket: undefined }, { socket: SALES }];
    expect(pickDeptWorkspace(list, SALES)).toEqual({ outcome: "switched", index: 1 });
  });

  it("ⓑ 이미 활성인 워크스페이스도 switched — 결과 상태가 같다(문구만 호출측이 가른다)", () => {
    // 판정은 활성 여부를 보지 않는다(부작용만 생략됨) — '이미 그 부서'여도 성공이 사실이다.
    const list = [{ socket: SALES }, { socket: HR }];
    expect(pickDeptWorkspace(list, SALES).outcome).toBe("switched");
    // 버튼 문구의 근거는 별도 순수 판정이다.
    expect(isActiveDeptSocket(list, 0, SALES)).toBe(true);
    expect(isActiveDeptSocket(list, 1, SALES)).toBe(false);
  });

  it("ⓒ ★연결 중(pending) 부서 → pending — '탭이 이미 닫혔습니다' 오안내 재발 금지", () => {
    // 초판은 후보를 `!w.pending` 으로만 찾아 이 경우를 missing 으로 접었다. 탭은 있고
    // 부서 데몬이 기동 중일 뿐이므로 '닫힌 탭'이라는 안내는 사실이 아니다.
    const list = [{ socket: undefined }, { socket: SALES, pending: true }];
    expect(pickDeptWorkspace(list, SALES)).toEqual({ outcome: "pending", index: 1 });
  });

  it("ⓓ 목록에 없는 socket → missing(index -1)", () => {
    const list = [{ socket: undefined }, { socket: HR }];
    expect(pickDeptWorkspace(list, SALES)).toEqual({ outcome: "missing", index: -1 });
  });

  it("ⓔ 같은 socket 에 pending 과 정상이 둘 다면 **정상 탭을 우선** 채택", () => {
    const list = [{ socket: SALES, pending: true }, { socket: SALES }];
    expect(pickDeptWorkspace(list, SALES)).toEqual({ outcome: "switched", index: 1 });
  });

  it("socket 미지정(기본 데몬)은 DEFAULT_SOCKET_KEY 로 정규화돼 매칭된다", () => {
    const list = [{ socket: HR }, { socket: undefined }];
    expect(pickDeptWorkspace(list, DEFAULT_SOCKET_KEY)).toEqual({
      outcome: "switched",
      index: 1,
    });
  });

  it("빈 목록·범위 밖 활성 인덱스에서도 던지지 않는다(레지스트리 잔재 방어)", () => {
    expect(pickDeptWorkspace([], SALES)).toEqual({ outcome: "missing", index: -1 });
    expect(isActiveDeptSocket([], 0, SALES)).toBe(false);
    expect(isActiveDeptSocket([{ socket: SALES }], 9, SALES)).toBe(false);
    expect(isActiveDeptSocket([{ socket: SALES }], -1, SALES)).toBe(false);
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★F3(2026-09-17 8라운드 · 단순화) — 삭제에서 **탭은 종료가 확인된 뒤에** 지운다.
//
// 반례(codex 3차 ②): 삭제 → 묘비 기록 → 탭 splice·저장 → `cys-dept down` rc 10(kill 0 · 등재 제거 0).
// 데몬은 살아 있는데 탭이 없고, 묘비가 탭 재생성을 막으므로 그 데몬을 다시 볼 주체가 하나도 없다.
// 6라운드는 '레지스트리 ∩ 묘비' 를 훑는 별도 reaper 로 메웠고, 그 reaper 가 탭 없는(CLI 로 만든) 부서까지
// 매 기동 종료 대상으로 삼았다(codex 4차 major 1·2). 8라운드는 상태를 더하지 않고 **순서**를 고친다:
// 종료 실패면 탭을 남긴다 = 다음 기동의 탭 기반 복원 정리가 곧 재시도 주체다.
// ────────────────────────────────────────────────────────────────────────────
describe("deptCloseAfterStop — 종료 실패면 탭을 지우지 않는다(F3)", () => {
  it("★반례: down 이 rc 10(Err)로 끝나면 탭은 남는다 — 지우면 잔존 데몬을 볼 주체가 사라진다", () => {
    const d = deptCloseAfterStop({
      stop: "failed",
      tombRecorded: true,
      error: "cys-dept down-sock rc=10 — 레지스트리 판독 실패로 종료 완료 미확인",
    });
    expect(d.removeTab).toBe(false);
    expect(d.detail).toContain("종료 완료 미확인"); // 사유를 그대로 싣는다(무음 삼킴 금지)
    expect(d.detail).toContain("탭을 다시 닫아 재시도하세요");
    expect(d.detail).toContain("삭제 의도는 기록"); // 묘비가 있으면 부서가 되살아나지는 않는다
  });

  it("종료가 성공하거나 불요(같은 소켓의 다른 탭·본부 탭)면 탭을 지운다 — 안내는 없다", () => {
    expect(deptCloseAfterStop({ stop: "ok", tombRecorded: true })).toEqual({ removeTab: true, detail: null });
    expect(deptCloseAfterStop({ stop: "not-needed", tombRecorded: false })).toEqual({ removeTab: true, detail: null });
  });

  it("★묘비 기록까지 실패했으면 뒷문장이 달라진다(다음 시작에 탭이 돌아온다 — 그때 다시 삭제)", () => {
    const d = deptCloseAfterStop({ stop: "failed", tombRecorded: false, error: "boom" });
    expect(d.removeTab).toBe(false);
    expect(d.detail).toContain("삭제 의도 기록도 실패");
    expect(d.detail).not.toContain("삭제 의도는 기록돼 있어");
  });

  it("사유는 길어도 잘라 싣는다(토스트 폭 보호) · 사유가 없어도 던지지 않는다", () => {
    const d = deptCloseAfterStop({ stop: "failed", tombRecorded: true, error: "x".repeat(1000) });
    expect(d.detail!.includes("x".repeat(300))).toBe(true);
    expect(d.detail!.includes("x".repeat(301))).toBe(false);
    expect(deptCloseAfterStop({ stop: "failed", tombRecorded: true }).removeTab).toBe(false);
  });
});

// ★E3 순수부 — 클릭 동작 검체는 wswiring.test.ts 가 배선과 함께 잠근다. 여기서는 계약(조회 1회·폴백)만.
describe("resolveDeptLaunchName — 등재 키 우선(E3)", () => {
  it("조회는 파서보다 **먼저** 한 번 — 조회 0회면 등재 키 우선 규칙이 무효화된다", async () => {
    let calls = 0;
    const r = await resolveDeptLaunchName("/Users/x/.local/state/cys-dept-dept-1/cys.sock", async () => {
      calls += 1;
      return { "dept-1": { socket: "/Users/x/.local/state/cys-dept-dept-1/cys.sock" } };
    });
    expect(calls).toBe(1);
    expect(r).toEqual({ name: "dept-1", regQueried: true });
  });
  it("socket 이 없으면 조회하지 않는다(본부 탭)", async () => {
    let calls = 0;
    const r = await resolveDeptLaunchName(undefined, async () => {
      calls += 1;
      return {};
    });
    expect(calls).toBe(0);
    expect(r).toEqual({ name: null, regQueried: false });
  });
  it("로더가 null 을 주면(조회 실패) regQueried=false · 파서 폴백", async () => {
    const r = await resolveDeptLaunchName("\\\\.\\pipe\\cys-dept-hr", async () => null);
    expect(r).toEqual({ name: "hr", regQueried: false });
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★H1(2026-09-17 12라운드 · codex 6차 major) — 묘비 대조는 **두 식별자의 정확 일치** 다.
//
// 10라운드는 이 자리를 fold(소문자화 + '.' 제거) 비교로 넓혔다가 정상 재생성 부서를 죽였다:
//   `launch Sales` → `down Sales`(묘비 `Sales`) → `launch sales`(팩의 slug 충돌 검사는 현재 등재만 본다) 에서
//   cysd 의 묘비 해제가 정확 일치(handlers.rs `dt.remove(&name)`)라 `Sales` 묘비가 남고, fold 판독이 살아 있는
//   `sales` 를 삭제 대상으로 오인해 탭을 지우고 종료를 요청했다.
// 대신 후보를 둘로 늘린다 — (i) 해소된 이름(등재 키 우선) · (ii) 소켓 파생 이름 — 각각 정확 일치.
//
// 변이 대조: (i) 만 검사하도록 소켓 파생 후보를 걷어내면 (b) 의 `DEPT-2` 묘비 핀이 red · fold 를 되살리면
//   (a) 의 재생성 핀이 red 다.
// ────────────────────────────────────────────────────────────────────────────
describe("restoreLaunchDecision — 묘비 대조는 두 식별자의 정확 일치다(H1)", () => {
  // (a) 삭제 → 표기 변경 재생성 — `Sales` 묘비가 남은 채 살아 있는 `sales` 를 켜야 한다.
  const REG_SALES = { sales: { socket: "/Users/x/.local/state/cys-dept-sales/cys.sock" } };
  const SALES_SOCK = REG_SALES.sales.socket;

  it("★(a) `down Sales` 뒤 재생성한 `sales` 는 그대로 켠다 — 묘비 {Sales} 는 다른 표기다(fold 복원 시 red)", () => {
    expect(restoreLaunchDecision({ socket: SALES_SOCK, regDepts: REG_SALES, tombstones: new Set(["Sales"]) })).toEqual({
      launch: "sales",
      reason: "launch",
    });
    // 전제: 두 식별자 모두 `sales` 이고, 묘비의 `Sales` 와 바이트가 다르다(fold 로 접으면 같아진다).
    expect(deptLaunchName(SALES_SOCK, REG_SALES)).toBe("sales");
    expect(deptNameFromSocket(SALES_SOCK)).toBe("sales");
    // named pipe 표기도 같다(win 재생성).
    const winReg = { sales: { socket: "\\\\.\\pipe\\cys-dept-sales" } };
    expect(restoreLaunchDecision({ socket: winReg.sales.socket, regDepts: winReg, tombstones: new Set(["Sales"]) })).toEqual({
      launch: "sales",
      reason: "launch",
    });
    // 같은 표기의 묘비는 당연히 막는다(대조군 — 정확 일치가 죽지 않았다).
    expect(restoreLaunchDecision({ socket: SALES_SOCK, regDepts: REG_SALES, tombstones: new Set(["sales"]) })).toEqual({
      launch: null,
      reason: "tombstone",
    });
  });

  // (b) 별칭 묘비 — 등재 키와 소켓 파생 이름이 갈리는 탭(named pipe 는 대소문자 무구분 = 같은 파이프).
  const REG = { "dept-2": { socket: "\\\\.\\pipe\\cys-dept-dept-2", display_name: "미래연구부" } };
  const STORED = "\\\\.\\pipe\\cys-dept-DEPT-2";

  it("★전제: 이 소켓의 해소된 이름은 등재 키 `dept-2` 이고, 소켓 파생 이름은 `DEPT-2` 다(두 이름이 갈린다)", () => {
    expect(deptLaunchName(STORED, REG)).toBe("dept-2");
    expect(deptNameFromSocket(STORED)).toBe("DEPT-2");
  });

  it("★(b) 묘비가 'DEPT-2'(옛 기록자 = 소켓 파생)면 launch=null — 소켓 파생 후보를 빼면 red", () => {
    expect(restoreLaunchDecision({ socket: STORED, regDepts: REG, tombstones: new Set(["DEPT-2"]) })).toEqual({
      launch: null,
      reason: "tombstone",
    });
  });

  it("★(b) 묘비가 'dept-2'(등재 키 = 현행 기록자)면 launch=null", () => {
    expect(restoreLaunchDecision({ socket: STORED, regDepts: REG, tombstones: new Set(["dept-2"]) })).toEqual({
      launch: null,
      reason: "tombstone",
    });
  });

  it("★(c) 무관 묘비는 켜는 것을 막지 않는다 — 접으면 같아지는 표기도 **다른 이름**이다", () => {
    // `de.pt-2` 는 fold 로는 `dept-2` 와 같지만, 묘비는 시간축 기록이라 그 축으로 삼키면 (a) 의 재생성이 죽는다.
    expect(restoreLaunchDecision({ socket: STORED, regDepts: REG, tombstones: new Set(["de.pt-2"]) })).toEqual({
      launch: "dept-2",
      reason: "launch",
    });
    expect(restoreLaunchDecision({ socket: STORED, regDepts: REG, tombstones: new Set(["dept-20", "dept_2", "dept-3"]) })).toEqual({
      launch: "dept-2",
      reason: "launch",
    });
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★I1(2026-09-17 13라운드 · opus 11R major) — 복원 루프의 묘비 정리 실패 토스트는 **실행 가능한
// 회수 절차**를 준다. G1 이후 그 토스트가 잔존 데몬의 **유일한 손잡이**인데(탭은 이미 지웠다),
// 10라운드 판은 부서를 `ws.name` = 표시명으로 가리켰다. 복원이 만드는 탭 이름은 `display_name ?? dname`
// 이라 한글 표시명이 그대로 들어오고(격리 체인 실측: 등재 키 `dept-2` ↔ 표시명 `영업부(한국)`),
// `cys-dept down 영업부(한국)` 은 validate_dept_name(`^[A-Za-z0-9][A-Za-z0-9_-]*$`)에서 **exit 2** 다.
// 이 핀은 문구에 **부서명이 들어가고 표시명이 들어가지 않는다**를 고정한다(표시명 복원 변이 → red).
// ────────────────────────────────────────────────────────────────────────────
describe("tombReapErrorToast — 회수 손잡이는 표시명이 아니라 부서명이다(I1)", () => {
  // 격리 체인이 실제로 만드는 조합: 등재 키 `dept-2` · 표시명 `영업부(한국)`.
  const DISPLAY = "영업부(한국)";
  const DEPT = "dept-2";

  it("★해소된 부서명이 명령에 그대로 들어가고 표시명은 제목·본문 어디에도 없다", () => {
    const t = tombReapErrorToast({ count: 1, deptName: DEPT, error: "rc 10" });
    expect(t.body).toContain(`cys-dept down ${DEPT}`);
    expect(t.body).not.toContain(DISPLAY);
    expect(t.title).not.toContain(DISPLAY);
    expect(t.title).toContain("1곳");
    // 생성기는 표시명을 **받지 않는다**(타입 수준) — 같은 인자면 문구가 결정론으로 같다.
    expect(tombReapErrorToast({ count: 1, deptName: DEPT, error: "rc 10" })).toEqual(t);
  });

  it("★해소 실패(null)면 거짓 명령 대신 `cys-dept list` 로 이름을 확인하는 절차를 준다", () => {
    const t = tombReapErrorToast({ count: 2, deptName: null, error: "rc 10" });
    expect(t.body).toContain("`cys-dept list` 로 부서명을 확인한 뒤 `cys-dept down <부서명>`");
    expect(t.body).not.toContain(DISPLAY);
    expect(t.title).toContain("2곳");
    // 이름을 모르면서 아는 척하지 않는다(가짜 인자를 박아 넣으면 red).
    expect(t.body).toContain("(부서명 미상)");
  });

  it("★체인 핀: 표시명 `영업부(한국)` 탭이라도 문구에는 **등재 키 dept-2** 만 나간다", () => {
    // 격리 체인(한글 HOME) 실측 조합 — 등재 `dept-2` · 표시명 `영업부(한국)` · 소켓은 한글 경로.
    const REG = { "dept-2": { socket: "/Users/홍길동/.local/state/cys-dept-dept-2/cys.sock" } };
    const ws = { name: DISPLAY, socket: REG["dept-2"].socket }; // 복원이 만드는 탭(name = display_name ?? dname)
    const resolved = deptLaunchName(ws.socket, REG); // main.ts 묘비 분기가 생성기에 넘기는 바로 그 값
    expect(resolved).toBe(DEPT);
    const t = tombReapErrorToast({ count: 1, deptName: resolved, error: "rc 10" });
    expect(t.body).toContain(`cys-dept down ${DEPT}`);
    expect(t.body).not.toContain(ws.name); // 표시명으로 안내하면 `cys-dept down` 이 exit 2 다
  });

  it("가장 최근 사유는 해소된 이름으로 적고 원문은 300자에서 자른다(토스트 폭 보호)", () => {
    const t = tombReapErrorToast({ count: 3, deptName: DEPT, error: "x".repeat(400) });
    expect(t.body).toContain(`가장 최근 사유 — ${DEPT}: `);
    expect(t.body).toContain("x".repeat(300));
    expect(t.body).not.toContain("x".repeat(301));
  });
});
