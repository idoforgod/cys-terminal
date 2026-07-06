// ime.ts 리듀서 프로필별 기록 시퀀스 테스트 (bun test — 신규 의존성 0).
//
// 각 프로필은 실기기 WebKit이 발화한 DOM 이벤트 순서다. 리듀서를 그 순서로 돌려
// PTY로 나가는 바이트(sends)와 잔여 조합 상태(pending)를 검증한다.
import { describe, it, expect } from "bun:test";
import { imeStep, initialImeState, isHangulText, type ImeEvent, type ImeState } from "./ime";

/** 이벤트 시퀀스를 리듀서에 흘려 전송 바이트·debug·최종 state를 수집한다. */
function run(events: ImeEvent[], start: ImeState = initialImeState()) {
  let state = start;
  const sends: string[] = [];
  const debugs: string[] = [];
  for (const ev of events) {
    const r = imeStep(state, ev);
    state = r.state;
    for (const a of r.actions) {
      if ("send" in a) sends.push(a.send);
      else debugs.push(a.debug);
    }
  }
  return { state, sends, debugs, bytes: sends.join("") };
}

const input = (inputType: string, data: string | null): ImeEvent => ({ kind: "input", inputType, data });
const keydown = (keyCode: number, key: string): ImeEvent => ({ kind: "keydown", keyCode, key });
const onData = (data: string): ImeEvent => ({ kind: "onData", data });

describe("Profile C — 혼성(신규 버그): insertText 자모 커밋 후 표준 composition 진행", () => {
  it("insertText 'ㄴ' → compositionstart → onData '너' ⇒ 정확히 '너'만 전송(자모 유출 없음)", () => {
    const r = run([
      input("insertText", "ㄴ"), // 조합 첫 자모를 커밋 → pending "ㄴ"
      { kind: "compositionstart" }, // 이후 조합은 표준 composition으로 진행 → pending은 흡수됨
      onData("너"), // xterm이 완성 음절 1회 발화
    ]);
    expect(r.bytes).toBe("너"); // 수정 전에는 "ㄴ너" (자모 유출)
    expect(r.state.pending).toBe("");
    expect(r.debugs).toContain('DROP(composition-supersede) "ㄴ"');
  });

  it("어절 연쇄: 'ㄴ'+compositionstart+onData '너' 다음 음절 '는'은 유출 없음", () => {
    const r = run([
      input("insertText", "ㄴ"),
      { kind: "compositionstart" },
      onData("너"),
      { kind: "compositionstart" },
      onData("는"),
    ]);
    expect(r.bytes).toBe("너는");
  });

  it("composition inputType(insertCompositionText) 관측 시에도 pending drop", () => {
    const r = run([input("insertCompositionText", "너")], { pending: "ㄴ" });
    expect(r.state.pending).toBe("");
    expect(r.sends).toEqual([]); // 흡수된 자모는 폐기(전송 금지)
    expect(r.debugs).toContain('DROP(composition-supersede) "ㄴ"');
  });
});

describe("Profile B — 구형: 음절 단위 insertText → insertReplacementText 재조합 → keydown flush", () => {
  it("insertText '너' → insertReplacementText '넌' → keydown(Space) ⇒ '넌'", () => {
    const r = run([
      input("insertText", "너"), // pending "너"
      input("insertReplacementText", "넌"), // 조합 갱신 (pending → "넌")
      keydown(32, " "), // 비229 → flush
    ]);
    expect(r.bytes).toBe("넌");
    expect(r.state.pending).toBe("");
  });
});

describe("Profile A — 표준: composition 이벤트 + onData만", () => {
  it("compositionstart/update/end → onData '한' ⇒ onData 그대로 1회 전송, pending 무관여", () => {
    const r = run([
      { kind: "compositionstart" },
      { kind: "compositionupdate" },
      { kind: "compositionend" },
      onData("한"),
    ]);
    expect(r.sends).toEqual(["한"]);
    expect(r.bytes).toBe("한");
    expect(r.state.pending).toBe("");
  });
});

describe("병합 커밋 — 고속 입력 2음절 한 insertText", () => {
  it("insertText '안녕' ⇒ '안' 즉시 전송 + pending '녕', 이후 blur flush로 '녕'", () => {
    const r = run([
      input("insertText", "안녕"), // 앞 음절 "안" 즉시, 마지막 "녕" pending
      { kind: "blur" }, // 확정 flush
    ]);
    expect(r.sends).toEqual(["안", "녕"]);
    expect(r.bytes).toBe("안녕");
  });

  it("insertText '안녕' 직후 pending은 '녕'(flush 전)", () => {
    const r = run([input("insertText", "안녕")]);
    expect(r.state.pending).toBe("녕");
    expect(r.sends).toEqual(["안"]);
  });
});

describe("repl-sync — pending 없이 이미 전송된 음절 교정", () => {
  it("insertReplacementText '한' (pending 비어있음) ⇒ '\\x7f한' 전송", () => {
    const r = run([input("insertReplacementText", "한")]);
    expect(r.bytes).toBe("\x7f한");
    expect(r.sends).toEqual(["\x7f한"]);
  });
});

describe("deleteContentBackward — pending 감소", () => {
  it("멀티 pending '간다' → deleteContentBackward ⇒ '간'", () => {
    const r = run([input("deleteContentBackward", null)], { pending: "간다" });
    expect(r.state.pending).toBe("간");
  });
});

describe("keydown/blur flush · 229 무전송 · onData 순서 보존", () => {
  it("Enter(비229) keydown ⇒ pending flush", () => {
    const r = run([keydown(13, "Enter")], { pending: "안" });
    expect(r.bytes).toBe("안");
    expect(r.state.pending).toBe("");
  });

  it("keyCode 229 keydown ⇒ flush 안 함(조합 유지)", () => {
    const r = run([keydown(229, "Process")], { pending: "안" });
    expect(r.sends).toEqual([]);
    expect(r.state.pending).toBe("안");
  });

  it("blur ⇒ pending flush", () => {
    const r = run([{ kind: "blur" }], { pending: "안" });
    expect(r.bytes).toBe("안");
    expect(r.state.pending).toBe("");
  });

  it("onData 시 잔여 pending 먼저 전송 후 data (순서 보존)", () => {
    const r = run([onData("녕")], { pending: "안" });
    expect(r.sends).toEqual(["안", "녕"]); // pending 먼저, 그다음 data
    expect(r.bytes).toBe("안녕");
  });
});

describe("빈 onData 유출 가드 — 조합중 홑자모 (2026-07-06 실기기 계측 회귀)", () => {
  // 실캡처($TMPDIR/cys-ime.log · 오늘 세션): 사용자가 조합 중 홑자모까지 되돌린 상태에서
  // 데이터 없는 onData("")가 도착 → 수정 전에는 flush("onData")가 그 자모를 완성음절 앞으로
  // 유출(실 PTY 유출 5건: ㅇㅈㅇㅎㄱ). 수정 후에는 유출 0·유실 0.
  it("pending 'ㅎ'(조합중) + onData('') ⇒ 전송 0, pending 'ㅎ' 유지(유출 차단)", () => {
    const r = run([onData("")], { pending: "ㅎ" });
    expect(r.sends).toEqual([]); // 수정 전에는 ["ㅎ"] (자모 유출)
    expect(r.state.pending).toBe("ㅎ"); // 조합 지속 — 후속 이벤트가 흡수/완성
    expect(r.debugs).toContain('onData(empty) keep composing jamo "ㅎ"');
  });

  it("실캡처 재생: insertReplacementText로 홑자모 복귀 후 빈 onData ⇒ 자모 미유출", () => {
    // 로그: ...RT '하'←'합'←'하'←'ㅎ' 후 onData recv "" pending="ㅎ" → FLUSH(onData) "ㅎ"(버그)
    const r = run([
      input("insertReplacementText", "합"), // pending "하" → "합"  (seed pending "하")
      input("insertReplacementText", "하"), // 재조합 "합" → "하"
      input("insertReplacementText", "ㅎ"), // 홑자모까지 되돌림 pending "ㅎ"
      onData(""), // 빈 onData — 유출 지점
    ], { pending: "하" });
    expect(r.bytes).toBe(""); // 조합중 자모는 새어나가지 않는다
    expect(r.state.pending).toBe("ㅎ");
  });

  it("완성형 pending은 빈 onData에도 종전대로 flush(유실 방지 — 가드는 조합중 자모 한정)", () => {
    const r = run([onData("")], { pending: "안" });
    // 완성음절은 진짜 직전 음절 → 순서 보존 flush 유지(뒤따르는 빈 data send는 무해 no-op).
    expect(r.bytes).toBe("안"); // 유실 0 — "안"은 PTY로 나간다
    expect(r.sends).toContain("안");
    expect(r.state.pending).toBe("");
  });
});

describe("isHangulText — 자모·완성형만 참", () => {
  it("자모/완성형 참, 그 외 거짓", () => {
    expect(isHangulText("ㄴ")).toBe(true);
    expect(isHangulText("너")).toBe(true);
    expect(isHangulText("안녕")).toBe(true);
    expect(isHangulText("a")).toBe(false);
    expect(isHangulText("1")).toBe(false);
    expect(isHangulText("")).toBe(false);
  });
});
