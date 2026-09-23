// U17 팀 직접 만들기 — 확인 창 문구·판정 회귀 핀(deptcreate.ts · DOM·Tauri 불요).
//
// 무엇을 지키는가(설계 §3 U17 + 반박 D2·D7·D10 반영):
//   · 확인 창은 무엇이 만들어지는지 말한다 — 이름 · 자리 **최대** 5 · 작업 폴더 · 첫 로그인 가능성 · CEO 전환(조건형).
//   · 이미 등재된 팀(mission_key 일치 → cys-dept REUSE)은 '다시 열기'이되, 꺼져 있으면 다시 켜고 자리를
//     다시 띄운다는 사실을 숨기지 않는다(REUSE_DEAD 는 NEW 와 같은 부작용 — 반박 D2).
//   · 카탈로그 부재(exit 3)로 번호 팀으로 바뀌면 **새 대상에 대한** 재확인 창이다(자동 생성 금지 — 설계 D5).
//   · 자리 역할 목록은 편성 도구(javis_formation.py REQUIRED_ROLES)와 어긋나지 않는다(드리프트 핀).
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import { DEPT_SEAT_ROLES, predictLegacyDeptName, buildDeptCreatePlan } from "./deptcreate";

const catalog = {
  departments: {
    sales: { display: "영업팀", mission_key: "mk-sales", cwd: "~/work/sales", account: "acct-b" },
    ops: { display: "운영팀", mission_key: "mk-ops", cwd: "$HOME/ops", account: "owner" },
  },
};

describe("자리 역할 드리프트 핀 — 편성 도구와 같은 5석", () => {
  it("DEPT_SEAT_ROLES == javis_formation.py REQUIRED_ROLES", () => {
    const py = readFileSync(new URL("../../cysjavis-pack/bin/javis_formation.py", import.meta.url), "utf-8");
    const m = /^REQUIRED_ROLES\s*=\s*\(([^)]*)\)/m.exec(py);
    expect(m).not.toBeNull();
    const roles = [...m![1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
    expect([...DEPT_SEAT_ROLES]).toEqual(roles);
  });
});

describe("번호 팀 예상 이름 — 레지스트리의 비어 있는 가장 작은 dept-N", () => {
  it("빈 레지스트리 → dept-1", () => {
    expect(predictLegacyDeptName({ depts: {} })).toBe("dept-1");
    expect(predictLegacyDeptName({})).toBe("dept-1");
  });
  it("dept-1·dept-3 → dept-2 · 비정형 키는 무시", () => {
    expect(predictLegacyDeptName({ depts: { "dept-1": {}, "dept-3": {}, sales: {}, "dept-x": {} } })).toBe("dept-2");
  });
  it("레지스트리를 못 읽었으면 예측하지 않는다(null)", () => {
    expect(predictLegacyDeptName(null)).toBeNull();
  });
});

describe("확인 창 — 카탈로그 새 팀", () => {
  const p = buildDeptCreatePlan({ key: "sales", catalog, registry: { depts: { "dept-1": {} } } });
  it("제목·버튼: '팀 만들기 확인' · [만들기] · [취소]", () => {
    expect(p.title).toBe("팀 만들기 확인");
    expect(p.yesLabel).toBe("만들기");
    expect(p.noLabel).toBe("취소");
    expect(p.reuse).toBe(false);
    expect(p.legacy).toBe(false);
    expect(p.displayName).toBe("영업팀");
  });
  it("본문: 이름 · 자리 최대 5개 · 작업 폴더 · 첫 로그인 가능성", () => {
    expect(p.body).toContain("영업팀");
    expect(p.body).toContain("최대 5개");
    expect(p.body).toContain("~/work/sales");
    expect(p.body).toContain("로그인");
    expect(p.body).toContain("acct-b");
  });
  it("이미 팀이 있으면 CEO 전환 문구가 없다", () => {
    expect(p.body.includes("CEO")).toBe(false);
  });
});

describe("확인 창 — CEO 전환은 조건형(반박 D7)", () => {
  it("첫 팀(레지스트리 0개) → CEO 문구 + '시작 전이면' 보류 조건", () => {
    const p = buildDeptCreatePlan({ key: "sales", catalog, registry: { depts: {} } });
    expect(p.body).toContain("CEO");
    expect(p.body).toContain("시작 전이면");
  });
  it("레지스트리를 못 읽었으면 '첫 팀이면' 조건으로만 말한다", () => {
    const p = buildDeptCreatePlan({ key: "sales", catalog, registry: null });
    expect(p.body).toContain("첫 팀이면");
  });
});

describe("확인 창 — 이미 등재된 팀(REUSE) 은 '다시 열기' 이되 부작용을 숨기지 않는다(반박 D2)", () => {
  const reg = { depts: { "dept-2": { socket: "/tmp/s", mission_key: "mk-sales" } } };
  const p = buildDeptCreatePlan({ key: "sales", catalog, registry: reg });
  it("제목 '팀 다시 열기' · [열기]", () => {
    expect(p.reuse).toBe(true);
    expect(p.title).toBe("팀 다시 열기");
    expect(p.yesLabel).toBe("열기");
  });
  it("등재 이름과 '꺼져 있으면 다시 켜고 자리를 다시 띄운다'", () => {
    expect(p.body).toContain("dept-2");
    expect(p.body).toContain("다시 켜고");
    expect(p.body).toContain("최대 5개");
  });
});

describe("확인 창 — 번호 팀(레거시)", () => {
  it("예상 이름 + 홈 폴더 + 기본 계정의 팀 전용 폴더", () => {
    const p = buildDeptCreatePlan({ key: undefined, catalog: null, registry: { depts: { "dept-1": {} } } });
    expect(p.legacy).toBe(true);
    expect(p.title).toBe("팀 만들기 확인");
    expect(p.body).toContain("dept-2");
    expect(p.body).toContain("홈 폴더");
    expect(p.body).toContain("로그인");
  });
  it("카탈로그 팀이 모두 열려 있어 번호 팀으로 가는 사유를 첫 줄에", () => {
    const p = buildDeptCreatePlan({ key: undefined, catalog, registry: { depts: {} }, allRunning: true });
    expect(p.body.split("\n")[0]).toContain("모두");
  });
  it("카탈로그 판독 실패 사유를 말한다", () => {
    const p = buildDeptCreatePlan({ key: undefined, catalog: null, registry: { depts: {} }, catalogUnreadable: true });
    expect(p.body).toContain("읽지 못해");
  });
  it("레지스트리 미조회면 이름을 단정하지 않는다", () => {
    const p = buildDeptCreatePlan({ key: undefined, catalog: null, registry: null });
    expect(p.body).toContain("만들 때 정해집니다");
  });
});

describe("확인 창 — exit 3 재확인(대상이 바뀜 · 자동 생성 금지)", () => {
  it("고른 팀 이름을 밝히고 번호 팀으로 만들지 묻는다", () => {
    const p = buildDeptCreatePlan({ key: undefined, catalog: null, registry: { depts: {} }, fallbackFrom: "영업팀" });
    expect(p.title).toBe("번호 팀으로 만들까요?");
    expect(p.yesLabel).toBe("번호 팀 만들기");
    expect(p.noLabel).toBe("취소");
    expect(p.body).toContain("영업팀");
    expect(p.legacy).toBe(true);
  });
});

describe("방어 — 비정상 입력에 던지지 않는다", () => {
  it("카탈로그에 없는 키·필드 누락", () => {
    expect(() => buildDeptCreatePlan({ key: "ghost", catalog, registry: { depts: {} } })).not.toThrow();
    const p = buildDeptCreatePlan({ key: "ghost", catalog: { departments: { ghost: {} } }, registry: { depts: {} } });
    expect(p.displayName).toBe("ghost");
    expect(p.body).toContain("지정 없음");
  });
  it("레지스트리 값이 null 인 항목", () => {
    expect(() =>
      buildDeptCreatePlan({ key: "sales", catalog, registry: { depts: { "dept-1": null } } }),
    ).not.toThrow();
  });
});
