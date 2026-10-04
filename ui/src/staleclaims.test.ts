// 성찰 2회차 R2F-UI — 사실과 달라진 서술(낡은 주석·검체 이름의 단정)을 고친 자리의 소스 핀.
//
// 왜 필요한가: 주석·검체 이름은 코드가 바뀌어도 따라 바뀌지 않아 **사실과 다른 문장**으로 남는다 — 다음에 읽는 사람이 그 문장을 조건으로 쓰면(예: '키가 없으면 구버전 데몬' 을 안내 문구의 근거로)
// 새 동작이 그 분기를 탄다(A2 m-1). 이 파일은 고친 문장 각각에 대해 ① 옛 단정이 없고 ② 고친 사실이 코드에서도 성립함을 소스로 본다. 코드는 건드리지 않는다(읽기 전용).
//   A3 n9 — 낡은 주석 넷(formationDone 설명 · addDeptWorkspace 머리의 launch await 시간 · style.css headlineSev · T_SAC 설명)
//   A2 m-1 — current_profiles 키 부재를 '구버전 데몬' 이라 단정한 주석(usagebar.ts · src-tauri main.rs 병합 설명)
//   (7) deptcreate.test.ts 로스터 상수 핀 주석 한 줄
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (rel: string): string => readFileSync(new URL(rel, import.meta.url), "utf-8");
const MAIN = read("./main.ts");
const CODE = (s: string): string =>
  s
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");
const lineWith = (hay: string, needle: string): string => {
  const l = hay.split("\n").find((x) => x.includes(needle));
  expect({ 찾는_줄: needle, 존재: l !== undefined }).toEqual({ 찾는_줄: needle, 존재: true });
  return l as string;
};

describe("★R2F-UI(A3 n9) 낡은 주석 넷 — 사실대로", () => {
  it("① Workspace.formationDone 설명 — '닫힌 탭' 을 이 표식의 원인으로 말하지 않는다: 이 값은 자리 판정 분기(checkDeptFormationNotices) 한 곳에서만 선다", () => {
    const l = lineWith(MAIN, "formationDone?: boolean;");
    expect(l.includes("다섯 다 붙음 · 15분 상한 · 닫힌 탭)")).toBe(false); // 옛 단정
    expect(l.includes("한 곳에서만 선다")).toBe(true);
    expect(l.includes("탭이 목록에서 사라져 점검 대상에서 빠진다")).toBe(true); // 닫힌 탭의 실제 처리
    // 사실: formationDone 을 세우는 대입은 코드 전체에서 정확히 한 곳이고 자리 판정 분기 안이다
    const code = CODE(MAIN);
    expect(code.split("formationDone = true").length - 1).toBe(1);
    const at = code.indexOf("formationDone = true");
    const fn = code.lastIndexOf("function checkDeptFormationNotices(", at);
    expect(fn).toBeGreaterThan(0);
    expect(code.slice(fn, at).includes('v.verdict === "seated" || v.verdict === "check"')).toBe(true);
  });
  it("② addDeptWorkspace 머리 주석 — 「launch await(최대 ~12s)」 가 없다 · 실측(이 맥 25~30초 · 느린 PC 1분 넘게 · 윈도우 11 러너 약 43~44초)을 말한다", () => {
    expect(MAIN.includes("launch await(최대 ~12s)")).toBe(false);
    expect(MAIN.includes("최대 ~12s")).toBe(false);
    const l = lineWith(MAIN, "① 표시 지연(안 C): 무거운 launch await(");
    for (const fact of ["25~30초", "1분 넘게", "43~44초"]) expect({ 사실: fact, 있음: l.includes(fact) }).toEqual({ 사실: fact, 있음: true });
    // 대기 화면 문구가 말하는 값과 같은 출처다(deptprogress.ts 의 실측 상수 주석)
    expect(read("./deptprogress.ts").includes("DEPT_TYPICAL_SECS = 30")).toBe(true);
  });
  it("③ style.css headlineSev 주석 — '주 계정 창 최고 심각도' 라고만 말하지 않는다: 약식이면 요약에 실린 값(오래돼 `?` 가 붙은 값 제외) · 한 제공자뿐이면 주 계정의 두 창", () => {
    const css = read("./style.css");
    const l = lineWith(css, "headlineSev");
    expect(l.includes("headlineSev — 주 계정 창 최고 심각도(오래된 값이면 클래스 없음)")).toBe(false); // 옛 단정
    for (const fact of ["제공자별 약식", "요약에 실린 값", "제공자가 하나뿐이면 주 계정의 두 창"]) expect({ 사실: fact, 있음: l.includes(fact) }).toEqual({ 사실: fact, 있음: true });
    // 사실: usagebar.ts 의 headlineSev 는 sum(약식)이 있으면 sum.sev, 없으면 주 계정 창(pv)에서 계산한다
    const ub = CODE(read("./usagebar.ts"));
    const at = ub.indexOf("const headlineSev");
    expect(at).toBeGreaterThan(0);
    expect(ub.slice(at, at + 200).includes("sum")).toBe(true);
    expect(ub.slice(at, at + 400).includes("pv.some(")).toBe(true);
  });
  it("④ T_SAC 설명 — 이 상한을 쓰는 조회가 둘(smart_app_control · update_checked_launch_enabled)이고 최악의 대기는 상한의 2배라고 말한다", () => {
    const a = MAIN.indexOf("const T_SAC = winScaled(2_500);");
    expect(a).toBeGreaterThan(0);
    const head = MAIN.slice(Math.max(0, a - 1600), a);
    for (const fact of ["smart_app_control", "update_checked_launch_enabled", "조회 둘", "2배"]) expect({ 사실: fact, 있음: head.includes(fact) }).toEqual({ 사실: fact, 있음: true });
    // 사실: 그 상한을 쓰는 곳은 promptBinaryPatch 의 두 rpcT 호출뿐이다
    const code = CODE(MAIN);
    expect(code.split(", T_SAC)").length - 1).toBe(2);
    expect(code.includes('rpcT(invoke("smart_app_control"), T_SAC)')).toBe(true);
    expect(code.includes('rpcT(invoke("update_checked_launch_enabled"), T_SAC)')).toBe(true);
  });
});

describe("★R2F-UI(A2 m-1) current_profiles 키 부재 = '구버전 데몬' 단정 주석 — 신 데몬이 이번에 읽지 못한 폴더가 낀 행도 키를 뺀다", () => {
  const QUALIFIER = "신 데몬이 이번에 읽지 못한 폴더";
  it("usagebar.ts — 옛 단정 다섯 문장이 없고, 고친 문장마다 '신 데몬이 이번에 읽지 못한 폴더' 가 있다", () => {
    const ub = read("./usagebar.ts");
    for (const old of [
      "키가 없으면 구버전 데몬 — profiles 로 폴백한다.",
      "키가 없는 구버전 데몬은 판정하지 않는다(false).",
      "· 키가 없으면(구버전 데몬) 종전처럼 profiles 로 만든다.",
      "키가 없으면(구버전 데몬) 종전 profiles.",
      "가산 키가 없으면(구버전 데몬) 종전 규칙으로 폴백한다",
    ])
      expect({ 옛_단정: old, 있음: ub.includes(old) }).toEqual({ 옛_단정: old, 있음: false });
    expect(ub.split(QUALIFIER).length - 1).toBeGreaterThanOrEqual(5);
    // 사실: 소비 코드는 키 부재를 '구버전' 으로 따로 분기하지 않고 배열 여부만 본다 — 키가 없으면 profiles 로 폴백(구 데몬과 읽지 못한 행이 같은 길)
    const code = CODE(ub.replace(/\/\*[\s\S]*?\*\//g, "")); // 블록 주석(JSDoc)까지 걷은 코드 줄만
    expect(code.includes("Array.isArray(a.current_profiles)")).toBe(true);
    expect(/구버전|legacy/i.test(code)).toBe(false);
  });
  it("src-tauri/src/main.rs — 병합 설명의 옛 단정('전부 구버전')이 없고 신 데몬 케이스를 말한다 · 그 사양을 박는 검체가 있다", () => {
    const rs = read("../../src-tauri/src/main.rs");
    const doc = rs.slice(rs.indexOf("/// ★0.14.43 가산 키 셋("), rs.indexOf("fn merge_account_rows("));
    expect(doc.length).toBeGreaterThan(100);
    expect(doc.includes("어느 응답에도 이 키(배열)가 없으면(전부 구버전) 결과에도 만들지 않는다")).toBe(false); // 옛 단정
    expect(doc.includes(QUALIFIER)).toBe(true);
    expect(doc.includes("키 없는 신 데몬 행 + 빈 배열 신 데몬 행 → 빈 배열")).toBe(true);
    expect(rs.includes("fn r2fui_merge_new_daemon_missing_key_plus_new_daemon_empty_array_is_empty_array()")).toBe(true);
    // 사실의 출처: 신 데몬도 읽지 못한 폴더가 낀 행에서는 키를 뺀다(데몬 소스)
    const acc = read("../../src/bin/cysd/accounts.rs");
    expect(acc.includes("fn current_profiles_for(")).toBe(true);
  });
});

describe("★R2F-UI(7) deptcreate.test.ts 로스터 상수 핀 — '상수 동일성만 본다' 주석 한 줄", () => {
  it("로스터 드리프트 핀 위에 「상수 동일성만 본다 — 실제 로스터는 설치된 CLI 에 달렸다」 가 있다(그 핀이 '화면과 팩이 같은 로스터를 본다' 는 인상을 주지 않게)", () => {
    const t = read("./deptcreate.test.ts");
    const a = t.indexOf('describe("자리 역할 드리프트 핀');
    const b = t.indexOf('describe("번호 팀 예상 이름');
    expect(a).toBeGreaterThan(0);
    expect(b).toBeGreaterThan(a);
    expect(t.slice(a, b).includes("상수 동일성만 본다 — 실제 로스터는 설치된 CLI 에 달렸다")).toBe(true);
  });
});
