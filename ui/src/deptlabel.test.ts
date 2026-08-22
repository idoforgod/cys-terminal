// deptlabel.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0). WP-10 잠금 픽스처 대응.
//
// '＋부서' 클릭 후 부서 데몬 준비 동안 탭 라벨이 진행 상태를 명시하는지(멈춘 줄 오해 방지),
// 확정 후엔 실제 표시명으로 바뀌는지 결정론으로 검증한다.
import { describe, it, expect } from "bun:test";
import { deptPlaceholderLabel, DEPT_PENDING_LABEL, deptSlugOfSocket } from "./deptlabel";

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
