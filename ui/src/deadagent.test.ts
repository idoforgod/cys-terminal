// deadagent.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0). CU-6B2 렌더 분기 잠금.
//
// 잠그는 사실: 접기 대상은 `agent_alive === false && !exited` **하나뿐**이다. null(미상)을 접으면
// 수동 셸·부팅 중 pane 이 죽은 것처럼 보이고(오탐), exited 를 접으면 이미 [exited] 로 말하고 있는
// 상태를 이중 표기한다. 세 분기(null/false/exited)를 각각 못박는다.
import { describe, it, expect } from "bun:test";
import { isDeadAgentPane, deadAgentHeaderText } from "./deadagent";

describe("isDeadAgentPane — 접기 대상 판정(엄격)", () => {
  it("agent_alive=false + 미종료 → 대상(접는다)", () => {
    expect(isDeadAgentPane({ agent_alive: false, exited: false })).toBe(true);
  });
  it("agent_alive=null(미상) → 비대상(수동 셸·구데몬 오탐 차단)", () => {
    expect(isDeadAgentPane({ agent_alive: null, exited: false })).toBe(false);
  });
  it("agent_alive 키 부재(undefined) → 비대상", () => {
    expect(isDeadAgentPane({ exited: false })).toBe(false);
  });
  it("agent_alive=true → 비대상", () => {
    expect(isDeadAgentPane({ agent_alive: true, exited: false })).toBe(false);
  });
  it("exited=true 는 agent_alive=false 여도 비대상([exited] 표기 소관)", () => {
    expect(isDeadAgentPane({ agent_alive: false, exited: true })).toBe(false);
  });
  it("exited 키 부재는 미종료로 본다(구데몬 스큐 안전)", () => {
    expect(isDeadAgentPane({ agent_alive: false })).toBe(true);
  });
  it("엔트리 자체가 없으면 비대상(폴링 실패에 접지 않는다)", () => {
    expect(isDeadAgentPane(null)).toBe(false);
    expect(isDeadAgentPane(undefined)).toBe(false);
  });
});

describe("deadAgentHeaderText — 접힌 헤더 한 줄", () => {
  it("접힘 상태는 '펼치기'를 안내한다", () => {
    expect(deadAgentHeaderText("worker", true)).toBe("worker · agent✗ · 펼치기");
  });
  it("펼침 상태는 '접기'를 안내한다", () => {
    expect(deadAgentHeaderText("worker", false)).toBe("worker · agent✗ · 접기");
  });
  it("역할 없음(무역할 셸)도 사실대로 적는다", () => {
    expect(deadAgentHeaderText(null, true)).toBe("역할 없음 · agent✗ · 펼치기");
    expect(deadAgentHeaderText("   ", true)).toBe("역할 없음 · agent✗ · 펼치기");
  });
});
