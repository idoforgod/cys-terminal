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
} from "./deptlabel";

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
  it("② 표기만 다른 같은 named pipe(대소문자)도 레지스트리 키로 맞춘다(sameSocket 술어)", () => {
    // `\\?\pipe\` 접두는 파서가 못 읽지만 sameSocket 도 다르게 보므로 null — 대소문자 차이만 흡수한다.
    const reg = { "dept-2": { socket: "\\\\.\\pipe\\cys-dept-dept-2" } };
    expect(deptLaunchName("\\\\.\\pipe\\cys-dept-DEPT-2", reg)).toBe("DEPT-2"); // 파서 우선(종전 순서 보존)
    expect(deptLaunchName("\\\\?\\pipe\\cys-dept-dept-2", reg)).toBeNull();
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
