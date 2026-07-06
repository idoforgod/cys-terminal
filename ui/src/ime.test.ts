// ime.ts 리듀서 프로필별 기록 시퀀스 테스트 (bun test — 신규 의존성 0).
//
// 각 프로필은 실기기 WebKit이 발화한 DOM 이벤트 순서다. 리듀서를 그 순서로 돌려
// PTY로 나가는 바이트(sends)와 잔여 조합 상태(pending)를 검증한다.
import { describe, it, expect } from "bun:test";
import { imeStep, initialImeState, isHangulText, isChoseongOf, type ImeEvent, type ImeState } from "./ime";

/** 이벤트 시퀀스를 리듀서에 흘려 전송 바이트·debug·armDefer 횟수·최종 state를 수집한다. */
function run(events: ImeEvent[], start?: Partial<ImeState>) {
  let state: ImeState = { ...initialImeState(), ...start };
  const sends: string[] = [];
  const debugs: string[] = [];
  let armDefers = 0;
  for (const ev of events) {
    const r = imeStep(state, ev);
    state = r.state;
    for (const a of r.actions) {
      if ("send" in a) sends.push(a.send);
      else if ("armDefer" in a) armDefers++;
      else debugs.push(a.debug);
    }
  }
  return { state, sends, debugs, armDefers, bytes: sends.join("") };
}

const input = (inputType: string, data: string | null): ImeEvent => ({ kind: "input", inputType, data });
const keydown = (keyCode: number, key: string): ImeEvent => ({ kind: "keydown", keyCode, key });
const onData = (data: string): ImeEvent => ({ kind: "onData", data });
/** macOS WKWebView 배선의 onData(wk=true) — 한글 defer 경로 활성. */
const onDataWk = (data: string): ImeEvent => ({ kind: "onData", data, wk: true });
const deferTimeout = (): ImeEvent => ({ kind: "deferTimeout" });

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

describe("3차 재발(2026-07-06) — 이중배달 defer 회귀: 초성 선유출·음절 복제", () => {
  // 주인님 실증 샘플(master pane 실타이핑): "ㅁ마스터다"·"ㅎ화"·"ㄷ되"(패턴 A 초성 선유출),
  // "다다"·"해해"(패턴 B 음절 복제). 0.12.20 업스트림 리듀서 재작성 때 1차 defer(017e20e)가
  // 소실되어 onData 즉시 send가 부활한 것 — 아래 시퀀스는 수리 전 리듀서에서 샘플과 동일한
  // 바이트열을 재생함을 확인했다(패턴A: "ㅁ"+"마"+DEL"마"→화면 "ㅁ마" · 패턴B: "다다").
  it("패턴 A: insertText 'ㅁ' → onData '마' → RT '마' ⇒ '마'만 (초성 선유출 차단)", () => {
    const r = run([
      input("insertText", "ㅁ"), // 머신이 초성 커밋 → pending "ㅁ"
      onDataWk("마"), // xterm 이중배달(완성 음절) — 수리 전: flush가 "ㅁ" 유출 + "마" 즉시 send
      input("insertReplacementText", "마"), // 머신이 같은 글자를 받아감 → 유예분 취소
      keydown(13, "Enter"),
    ]);
    expect(r.bytes).toBe("마"); // 수리 전: "ㅁ마\x7f마" (화면 "ㅁ마")
    expect(r.state.deferred).toBe("");
    expect(r.debugs).toContain('DEFER-cancel(insertReplacementText) "마"');
  });

  it("패턴 A 키마다 이중배달(1차 프로필): onData 'ㅁ'→insertText 'ㅁ'→onData '마'→RT '마' ⇒ '마'만", () => {
    const r = run([
      onDataWk("ㅁ"), // xterm이 조합 시작 자모도 배달 — 수리 전: 즉시 send(자모 유출)
      input("insertText", "ㅁ"), // 머신이 받아감 → 유예 취소, pending "ㅁ"
      onDataWk("마"),
      input("insertReplacementText", "마"),
      keydown(13, "Enter"),
    ]);
    expect(r.bytes).toBe("마"); // 수리 전: "ㅁㅁ마\x7f마"
    expect(r.debugs).toContain('DEFER-cancel(insertText) "ㅁ"');
  });

  it("패턴 B: insertText 'ㄷ' → RT '다' → onData '다' ⇒ '다'만 (음절 복제 차단)", () => {
    const r = run([
      input("insertText", "ㄷ"),
      input("insertReplacementText", "다"), // pending "다"
      onDataWk("다"), // xterm 이중배달 — 수리 전: flush "다" + send "다" = "다다"
      keydown(13, "Enter"),
    ]);
    expect(r.bytes).toBe("다"); // 수리 전: "다다"
    expect(r.debugs).toContain('DUP-drop(onData) "다" (== pending)');
  });

  it("실증 샘플 재생: '너는' 어절 — 이중배달 혼입에도 '너는' 정확 출력", () => {
    // 너(ㄴ+ㅓ) 는(ㄴ+ㅡ+ㄴ): 머신 체인 + 간헐 onData 이중배달 혼입 시퀀스
    const r = run([
      input("insertText", "ㄴ"), // pending "ㄴ"
      onDataWk("너"), // 이중배달
      input("insertReplacementText", "너"), // 취소·pending "너"
      input("insertText", "ㄴ"), // 다음 음절 초성 → flush "너", pending "ㄴ"
      input("insertReplacementText", "느"), // pending "느"
      onDataWk("는"), // 이중배달(완성)
      input("insertReplacementText", "는"), // 취소·pending "는"
      keydown(13, "Enter"),
    ]);
    expect(r.bytes).toBe("너는"); // 수리 전: "너너는..." 류 복제/유출
  });
});

describe("defer 타임아웃 — 머신 미작동 변종(insertFromComposition만) 유실 0", () => {
  it("onData '히'(wk) → 타임아웃 ⇒ '히' 전송 (차단이 아닌 유예 — 유실 0)", () => {
    const r = run([onDataWk("히"), deferTimeout()]);
    expect(r.bytes).toBe("히");
    expect(r.armDefers).toBe(1); // 배선에 타이머 장전 지시
    expect(r.state.deferred).toBe("");
  });

  it("취소된 유예분은 타임아웃에 전송 없음 (복제 0)", () => {
    const r = run([onDataWk("ㅁ"), input("insertText", "ㅁ"), deferTimeout(), keydown(13, "Enter")]);
    expect(r.bytes).toBe("ㅁ"); // Enter가 pending "ㅁ" flush — 유예분 복제 없음
  });

  it("타임아웃 시 pending 초성이 유예 음절에 흡수된 상태면 폐기 (유출 0)", () => {
    // 머신이 insertText까지만 발화하고 멈춘 변종: pending "ㅁ" + deferred "마"
    const r = run([input("insertText", "ㅁ"), onDataWk("마"), deferTimeout()]);
    expect(r.bytes).toBe("마"); // "ㅁ마" 아님 — 초성은 그 음절의 조합중 상태
    expect(r.state.pending).toBe("");
    expect(r.debugs).toContain('DROP(absorbed-by-deferred) "ㅁ"');
  });

  it("keydown(Enter)이 타임아웃보다 먼저 와도 경계 유출 없음", () => {
    const r = run([input("insertText", "ㅁ"), onDataWk("마"), keydown(13, "Enter")]);
    expect(r.bytes).toBe("마"); // releaseDeferred(keydown)가 흡수 해소 후 방출
  });

  it("비한글 onData는 유예분 먼저 방출 후 데이터 전송 (순서 보존)", () => {
    const r = run([onDataWk("가"), onDataWk("\r")]);
    expect(r.sends).toEqual(["가", "\r"]);
  });
});

describe("wk 프로필 재생 — 기존 프로필 A·C의 defer 경로 등가", () => {
  it("Profile C(wk): insertText 'ㄴ' → compositionstart → onData '너' → 타임아웃 ⇒ '너'", () => {
    const r = run([
      input("insertText", "ㄴ"),
      { kind: "compositionstart" },
      onDataWk("너"),
      deferTimeout(),
    ]);
    expect(r.bytes).toBe("너");
    expect(r.state.pending).toBe("");
  });

  it("Profile A(wk): composition 3종 → onData '한' → 타임아웃 ⇒ '한' 1회", () => {
    const r = run([
      { kind: "compositionstart" },
      { kind: "compositionupdate" },
      { kind: "compositionend" },
      onDataWk("한"),
      deferTimeout(),
    ]);
    expect(r.sends).toEqual(["한"]);
  });

  it("영문·비한글(wk)은 유예 없이 즉시 전송 (지연 0·회귀 0)", () => {
    const r = run([onDataWk("a"), onDataWk("1")]);
    expect(r.sends).toEqual(["a", "1"]);
    expect(r.armDefers).toBe(0);
  });

  it("비wk 배선(Windows WebView2)은 한글 onData도 종전 즉시 send (회귀 0)", () => {
    const r = run([onData("너")]);
    expect(r.sends).toEqual(["너"]);
    expect(r.armDefers).toBe(0);
  });
});

describe("isChoseongOf — 초성 흡수 판별", () => {
  it("초성 매칭만 참", () => {
    expect(isChoseongOf("ㅁ", "마")).toBe(true);
    expect(isChoseongOf("ㅎ", "화")).toBe(true);
    expect(isChoseongOf("ㄴ", "마")).toBe(false);
    expect(isChoseongOf("ㅁㅁ", "마")).toBe(false);
    expect(isChoseongOf("ㅁ", "")).toBe(false);
    expect(isChoseongOf("ㅁ", "m")).toBe(false);
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

describe("R1 리뷰 보강 회귀(2026-07-06) — 누적·역전·U+1100 초성", () => {
  it("R1-①: 다중 onData 누적 'ㅁ'+'마' → insertText 'ㅁ' 부분취소 → RT '마' 취소 ⇒ '마' 1회", () => {
    const r = run([
      onDataWk("ㅁ"), // deferred "ㅁ"
      onDataWk("마"), // deferred "ㅁ마" (누적 — 타이머 미만료 고속 입력)
      input("insertText", "ㅁ"), // 앞부분만 받아감 → 부분취소, 잔여 "마"
      input("insertReplacementText", "마"), // 잔여도 받아감 → 전체 취소
      keydown(32, " "), // 경계
    ]);
    expect(r.bytes).toBe("마"); // 수정 전에는 "마ㅁ마"류 복제
    expect(r.state.deferred).toBe("");
  });

  it("R1-①b: 유예 홑자모가 머신 커밋 음절의 초성이면 흡수 취소 (유예 'ㅁ' ⊂ 커밋 '마')", () => {
    const r = run([
      onDataWk("ㅁ"),
      input("insertText", "마"), // 머신이 조합 완성분을 직접 커밋
      keydown(13, "Enter"),
    ]);
    expect(r.bytes).toBe("마"); // 수정 전에는 타임아웃/경계에서 "ㅁ" 유출
  });

  it("R1-②: pending(구분) '나' + 유예(신분) '가' → keydown ⇒ '나가' (시간순 — 역전 차단)", () => {
    const r = run([onDataWk("가"), keydown(13, "Enter")], { pending: "나" });
    expect(r.bytes).toBe("나가"); // 수정 전에는 "가나" (유예분 선전송 역전)
  });

  it("R1-②b: 유예(구분) '가' 잔존 중 insertText '나' 신규 커밋 ⇒ '가' 먼저 방출 (가나)", () => {
    const r = run([
      onDataWk("가"), // 조합전용 프로필의 직전 음절 — 머신이 안 받아감
      input("insertText", "나"), // 다음 음절 신규 커밋 (유예분보다 신분)
      keydown(13, "Enter"),
    ]);
    expect(r.bytes).toBe("가나"); // 시간순: 가(구) → 나(신)
    expect(r.state.deferred).toBe("");
  });

  it("R1-②c: deferTimeout도 pending(구분) 우선 — pending '나' + 유예 '가' ⇒ '나가'", () => {
    const r = run([onDataWk("가"), deferTimeout()], { pending: "나" });
    expect(r.bytes).toBe("나가");
  });

  it("R1-③: U+1100 조합자모 초성도 흡수 판별 — pending 'ᄆ'(U+1106) + 유예 '마' ⇒ '마'만", () => {
    const r = run([onDataWk("마"), deferTimeout()], { pending: "ᄆ" });
    expect(r.bytes).toBe("마"); // 수정 전에는 "마ᄆ"류 (흡수 실패 → 자모 유출)
    expect(isChoseongOf("ᄆ", "마")).toBe(true);
    expect(isChoseongOf("ᄂ", "마")).toBe(false); // ᄂ은 마의 초성 아님
  });

  it("기존 불변 재확인: 정상 페어링(onData 자모→insertText 동일)은 종전과 동일 경로", () => {
    const r = run([
      onDataWk("ㄹ"),
      input("insertText", "ㄹ"),
      input("insertReplacementText", "료"),
      keydown(32, " "),
    ], { pending: "완" });
    expect(r.bytes).toBe("완료");
  });
});
