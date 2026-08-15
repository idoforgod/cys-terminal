// trackfilter.ts 회귀 테스트 — 마우스 트래킹 DECSET 스트리핑의 3대 계약을 핀한다:
//  (1) 마우스 모드 전환(9·1000·1002·1003·1005·1006·1015·1016 h/l)은 반드시 걷어낸다.
//  (2) 그 외 바이트는 **단 하나도** 바꾸지 않는다(비마우스 DECSET·일반 CSI·텍스트·색상).
//  (3) 청크를 어디서 잘라도 결과가 통짜 처리와 바이트 동일하다(경계 전수 스윕 — 앵커④:
//      홀드백 결함=화면 백지화이므로 이 계약이 이 파일의 존재 이유다).
import { describe, it, expect } from "bun:test";
import { filterChunk, MouseTrackingFilter, MOUSE_ALL_OFF } from "./trackfilter";

const enc = new TextEncoder();
const bytes = (s: string) => enc.encode(s);
const runWhole = (s: string) => {
  // 스트림 의미론: feed 후 flush — 종단(exited)까지 포함한 전체 방출이 계약이다.
  const f = new MouseTrackingFilter();
  const a = f.feed(bytes(s));
  const b = f.flush();
  return Buffer.from(a).toString("binary") + Buffer.from(b).toString("binary");
};
// 문자열로 되돌릴 때 latin1(binary)로 — 바이트 정체성 비교용.
const asStr = (u: Uint8Array) => Buffer.from(u).toString("binary");

describe("마우스 트래킹 전환 제거", () => {
  for (const p of [9, 1000, 1002, 1003, 1005, 1006, 1015, 1016]) {
    it(`DECSET ?${p}h / DECRST ?${p}l 제거`, () => {
      expect(runWhole(`A\x1b[?${p}hB`)).toBe("AB");
      expect(runWhole(`A\x1b[?${p}lB`)).toBe("AB");
    });
  }
  it("복합 파라미터 전부 마우스 → 통째 제거", () => {
    expect(runWhole(`x\x1b[?1000;1006hy`)).toBe("xy");
    expect(runWhole(`x\x1b[?1002;1003;1015ly`)).toBe("xy");
  });
  it("혼합 파라미터 — 비마우스(1049·2004 등)는 순서 보존 재조립", () => {
    expect(runWhole(`x\x1b[?1049;1000hy`)).toBe("x\x1b[?1049hy");
    expect(runWhole(`x\x1b[?1000;2004;1006hy`)).toBe("x\x1b[?2004hy");
    expect(runWhole(`x\x1b[?25;1003;12ly`)).toBe("x\x1b[?25;12ly");
  });
  it("연속 전환(Claude Code 기동 시퀀스 형태)도 전부 제거 — 본문(UTF-8 한글) 무손상", () => {
    expect(runWhole(`\x1b[?1003h\x1b[?1006h프롬프트`)).toBe(asStr(bytes("프롬프트")));
  });
});

describe("무변경 보장 — 오탐은 렌더 파손이다", () => {
  const passthrough = [
    ["일반 텍스트", "hello 세계"],
    ["SGR 색상", "\x1b[31mred\x1b[0m"],
    ["커서 이동 CSI", "\x1b[2J\x1b[H\x1b[10;20H"],
    ["비마우스 DECSET(커서 표시)", "\x1b[?25h\x1b[?25l"],
    ["alt screen 진입/이탈", "\x1b[?1049h본문\x1b[?1049l"],
    ["bracketed paste 모드", "\x1b[?2004h\x1b[?2004l"],
    ["focus 보고(1004 — 마우스 아님)", "\x1b[?1004h\x1b[?1004l"],
    ["DECRQM 질의(h/l 아닌 최종 바이트)", "\x1b[?1000$p"],
    ["유사 숫자(비마우스 값)", "\x1b[?1001h\x1b[?1007h\x1b[?9999h"],
    ["ESC 단독·비CSI ESC", "\x1bZ\x1b(B\x1b"],
    ["OSC 제목 설정", "\x1b]0;title\x07"],
    ["파라미터 캡 초과(합법 후보 아님 — fail-open)", "\x1b[?" + "1".repeat(64) + "h"],
  ] as const;
  for (const [name, s] of passthrough) {
    it(`${name} → 바이트 동일`, () => {
      expect(runWhole(s)).toBe(asStr(bytes(s)));
    });
  }
});

describe("청크 경계 — 어디서 잘라도 통짜와 동일 (앵커④ 봉인)", () => {
  const samples = [
    `앞\x1b[?1003h중간\x1b[?1006h뒤`,
    `\x1b[31m색\x1b[0m\x1b[?1000;1049h본문\x1b[?25h`,
    `plain\x1b[?1000$p텍스트\x1b[?1016l끝`,
    `\x1b[?2004h붙여넣기\x1b[?1002;1003l\x1b[?1049l`,
  ];
  for (const s of samples) {
    it(`전수 분할 스윕: ${JSON.stringify(s.slice(0, 18))}…`, () => {
      const whole = runWhole(s);
      const raw = bytes(s);
      for (let cut = 0; cut <= raw.length; cut++) {
        const f = new MouseTrackingFilter();
        const a = f.feed(raw.slice(0, cut));
        const b = f.feed(raw.slice(cut));
        expect(asStr(a) + asStr(b)).toBe(whole);
      }
    });
    it(`3분할 전수 스윕(이중 경계): ${JSON.stringify(s.slice(0, 12))}…`, () => {
      const whole = runWhole(s);
      const raw = bytes(s);
      for (let c1 = 0; c1 <= raw.length; c1 += 3) {
        for (let c2 = c1; c2 <= raw.length; c2 += 3) {
          const f = new MouseTrackingFilter();
          const parts = [raw.slice(0, c1), raw.slice(c1, c2), raw.slice(c2)];
          const got = parts.map((p) => asStr(f.feed(p))).join("");
          expect(got).toBe(whole);
        }
      }
    });
  }
  it("1바이트씩 흘려도 동일(최악 경계)", () => {
    const s = `x\x1b[?1000;1049hy\x1b[?1006lz`;
    const whole = runWhole(s);
    const raw = bytes(s);
    const f = new MouseTrackingFilter();
    let got = "";
    for (const b of raw) got += asStr(f.feed(new Uint8Array([b])));
    expect(got).toBe(whole);
  });
  it("미완성 꼬리는 다음 feed에서 해소된다(carry 상태 확인)", () => {
    const f = new MouseTrackingFilter();
    expect(asStr(f.feed(bytes("A\x1b[?10")))).toBe("A");
    expect(asStr(f.feed(bytes("03hB")))).toBe("B");
  });
  it("순수 filterChunk: carry 는 캡 이내의 합법 접두만", () => {
    const { out, carry } = filterChunk(bytes("text\x1b[?100"));
    expect(asStr(out)).toBe("text");
    expect(asStr(carry)).toBe("\x1b[?100");
  });
});

describe("ESC 중단 후보 직후의 마우스 enable — sticky 고착 봉인 (2026-08-12 R2 확정)", () => {
  // ★결함 기전: 종전 코드는 h/l 아닌 최종 바이트(fin)를 '소비'하고 j+1 로 건너뛰었다 —
  // fin==ESC(중단된 DECSET 후보 직후 새 시퀀스 시작)이면 뒤따르는 `?1003h` 가 ESC 없는 일반
  // 바이트로 흘러 필터를 통과했고, 이후 disable(`?1003l`)은 정상 스트리핑돼 pane 이 닫힐
  // 때까지 트래킹이 영구 고착됐다. fin 을 남겨 재스캔하면 후속 enable 도 정상 스트리핑된다.
  it("`ESC[?25` 중단 직후의 `ESC[?1003h` 는 스트리핑된다(통짜)", () => {
    const f = new MouseTrackingFilter();
    const got = asStr(f.feed(bytes("\x1b[?25\x1b[?1003hX")));
    expect(got).toBe("\x1b[?25X");
  });
  it("중단 접두 + enable/disable 쌍 — 어느 쪽도 유출되지 않는다", () => {
    const f = new MouseTrackingFilter();
    const got = asStr(f.feed(bytes("A\x1b[?25\x1b[?1003hmid\x1b[?1003lB")));
    expect(got).toBe("A\x1b[?25midB");
  });
  it("fin 이 일반 바이트(DECRQM `$p` 류)인 기존 경로는 출력 불변", () => {
    const f = new MouseTrackingFilter();
    const s = "q\x1b[?1003$pw";
    expect(asStr(f.feed(bytes(s)))).toBe(s);
  });
  it("청크 경계: 중단 접두와 enable 이 분할돼 와도 동일", () => {
    const whole = (() => {
      const f = new MouseTrackingFilter();
      return asStr(f.feed(bytes("\x1b[?25\x1b[?1003hX")));
    })();
    const raw = bytes("\x1b[?25\x1b[?1003hX");
    for (let cut = 0; cut <= raw.length; cut++) {
      const f = new MouseTrackingFilter();
      const got = asStr(f.feed(raw.slice(0, cut))) + asStr(f.feed(raw.slice(cut)));
      expect(got).toBe(whole);
    }
  });
});

// ════════════════════════════════════════════════════════════════════════════════
// 정합기(스펙 D4+A8 — 2026-08-16) 계약 핀. 위 v1 계약(스트리핑·무변경·경계)은 그대로 두고,
// mac(os:'mac', reconcile:true) 소비 활성 시의 장부·전이·주입·소등 의미론을 아래가 고정한다.
// ════════════════════════════════════════════════════════════════════════════════

const MOUSE_SET = [9, 1000, 1002, 1003, 1005, 1006, 1015, 1016] as const;
// 복귀·리셋 소등 상수의 기대 문안 — 전 집합·이 순서·1049l 미포함이 계약이다.
const ALL_OFF =
  "\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?1016l";
const mac = () => new MouseTrackingFilter({ os: "mac", reconcile: true });
const feedS = (f: MouseTrackingFilter, s: string) => asStr(f.feed(bytes(s)));

describe("정합기 — 장부 갱신 8종·재생 주입 (스펙 D4)", () => {
  it("MOUSE_ALL_OFF 상수 자체가 계약 문안과 일치(전 집합·1049l 미포함)", () => {
    expect(MOUSE_ALL_OFF).toBe(ALL_OFF);
    expect(MOUSE_ALL_OFF.includes("1049")).toBe(false);
  });
  for (const p of MOUSE_SET) {
    it(`?${p}h 장부 기록(primary 스트리핑 유지) → alt 진입 시 방출 직후 재생 주입`, () => {
      const f = mac();
      expect(feedS(f, `\x1b[?${p}h`)).toBe(""); // primary: 현행대로 스트리핑
      // 주입 위치 = 1049h 방출 **직후**(청크 말미 금지 — 후속 바이트 Y 앞이어야 한다)
      expect(feedS(f, `X\x1b[?1049hY`)).toBe(`X\x1b[?1049h\x1b[?${p}hY`);
    });
  }
  it("명시 DECRST 는 장부를 l 로 갱신 — alt 진입 시 무주입", () => {
    const f = mac();
    feedS(f, "\x1b[?1000h");
    feedS(f, "\x1b[?1000l");
    expect(feedS(f, "\x1b[?1049h")).toBe("\x1b[?1049h");
  });
  it("재생 주입은 앱의 enable 시간순 보존 — 재-enable 은 말미로(마지막 enable 승리)", () => {
    const f = mac();
    feedS(f, "\x1b[?1003h\x1b[?1006h\x1b[?1003h"); // 1003 재-enable → 1006 뒤로
    expect(feedS(f, "\x1b[?1049h")).toBe("\x1b[?1049h\x1b[?1006h\x1b[?1003h");
  });
});

describe("정합기 — 전이 감시 {47,1047,1049} h/l·장부 비변이 (스펙 A8)", () => {
  for (const t of [47, 1047, 1049]) {
    it(`?${t}h 진입 주입 / ?${t}l 복귀 8종 상수 소등 / 재진입 재생(전이는 장부 불변)`, () => {
      const f = mac();
      feedS(f, "\x1b[?1000h");
      expect(feedS(f, `\x1b[?${t}h`)).toBe(`\x1b[?${t}h\x1b[?1000h`);
      expect(feedS(f, `\x1b[?${t}l`)).toBe(`\x1b[?${t}l${ALL_OFF}`);
      // suspend/resume: 전이가 장부를 소거했다면 여기서 재생이 빈손이 된다 — 비변이 계약
      expect(feedS(f, `\x1b[?${t}h`)).toBe(`\x1b[?${t}h\x1b[?1000h`);
    });
  }
  it("alt 중 중복 진입(h 재수신)은 전이 아님 — 통과만·재주입 없음", () => {
    const f = mac();
    feedS(f, "\x1b[?1000h");
    feedS(f, "\x1b[?1049h");
    expect(feedS(f, "\x1b[?1049h")).toBe("\x1b[?1049h");
  });
  it("primary 에서의 l(전이 아님)은 소등 주입 없음 — 원문 통과", () => {
    const f = mac();
    feedS(f, "\x1b[?1000h");
    expect(feedS(f, "\x1b[?1049l")).toBe("\x1b[?1049l");
  });
});

describe("정합기 — 결합 파라미터 CSI = 재조립분 방출 직후 주입 (스펙 D4)", () => {
  it("?1049;1000h → ?1049h 재조립 + 직후 ?1000h 주입", () => {
    const f = mac();
    expect(feedS(f, "\x1b[?1049;1000h")).toBe("\x1b[?1049h\x1b[?1000h");
  });
  it("선행 장부(1006h) + 결합 진입 — 재생은 시간순(1006 먼저, 결합분 1000 은 말미)", () => {
    const f = mac();
    feedS(f, "\x1b[?1006h");
    expect(feedS(f, "X\x1b[?1049;1000hY")).toBe("X\x1b[?1049h\x1b[?1006h\x1b[?1000hY");
  });
  it("alt 중 결합 복귀(?1049;1002l) — 원문 통과 + 직후 8종 상수 소등", () => {
    const f = mac();
    feedS(f, "\x1b[?1000h");
    feedS(f, "\x1b[?1049h");
    expect(feedS(f, "\x1b[?1049;1002l")).toBe(`\x1b[?1049;1002l${ALL_OFF}`);
  });
});

describe("정합기 — alt 중 명시 전환 통과·복귀 후 스트리핑 재개 (스펙 D4)", () => {
  it("alt 구간의 마우스 DECSET/DECRST 는 원문 통과(앱이 마우스를 갖는다)", () => {
    const f = mac();
    feedS(f, "\x1b[?1049h");
    expect(feedS(f, "\x1b[?1002h\x1b[?1006h")).toBe("\x1b[?1002h\x1b[?1006h");
    expect(feedS(f, "\x1b[?1002l")).toBe("\x1b[?1002l");
  });
  it("복귀 전이 뒤에는 primary 스트리핑이 재개된다", () => {
    const f = mac();
    feedS(f, "\x1b[?1049h");
    feedS(f, "\x1b[?1003h"); // alt 중 통과(장부에도 기록)
    feedS(f, "\x1b[?1049l"); // 복귀 — 8종 소등
    expect(feedS(f, "\x1b[?1003h")).toBe(""); // 재개된 스트리핑
  });
  it("복귀 소등은 장부와 무관한 **전 집합 상수** — 빈 장부에서도 8종 전부(유출 enable 안전망)", () => {
    const f = mac();
    expect(feedS(f, "\x1b[?1049h")).toBe("\x1b[?1049h"); // 빈 장부 — 주입 없음
    expect(feedS(f, "\x1b[?1049l")).toBe(`\x1b[?1049l${ALL_OFF}`);
  });
});

describe("정합기 — RIS(ESC c) = alt 복귀 + 장부 소거 동일 처리 (스펙 A8)", () => {
  it("alt+트래킹 중 RIS: 원문 방출 직후 8종 소등·장부 소거·복귀 처리", () => {
    const f = mac();
    feedS(f, "\x1b[?1000h");
    feedS(f, "\x1b[?1049h");
    expect(feedS(f, "A\x1bcB")).toBe(`A\x1bc${ALL_OFF}B`);
    // 복귀 처리 입증: 이후 마우스 DECSET 는 primary 취급으로 스트리핑된다
    expect(feedS(f, "\x1b[?1003h")).toBe("");
    // 장부 소거 입증: 재진입 재생에 소거 전 1000h 는 없고 RIS 후 기록된 1003h 만 있다
    expect(feedS(f, "\x1b[?1049h")).toBe("\x1b[?1049h\x1b[?1003h");
  });
  it("primary 의 RIS 도 동일(소등 주입은 멱등·무해 — crash 후 셸 reset 동선)", () => {
    const f = mac();
    feedS(f, "\x1b[?1000h");
    expect(feedS(f, "\x1bc")).toBe(`\x1bc${ALL_OFF}`);
  });
  it("청크 경계에 걸린 RIS(ESC | c 분할)도 동일", () => {
    const f = mac();
    feedS(f, "\x1b[?1000h");
    feedS(f, "\x1b[?1049h");
    expect(feedS(f, "\x1b")).toBe(""); // ESC 단독 — carry 보류
    expect(feedS(f, "c")).toBe(`\x1bc${ALL_OFF}`);
  });
});

describe("DECSTR(CSI ! p) 무처리 핀 — 역회귀 (스펙 A8·벤더 InputHandler.ts:2692)", () => {
  // xterm 의 softReset 은 버퍼·마우스 프로토콜을 건드리지 않는다 — DECSTR 를 alt 복귀로
  // 오판하면 정당한 트래킹 앱이 DECSTR 1회로 마우스를 잃는 원 결함 계열이 부활한다.
  it("alt+트래킹 중 CSI ! p — 바이트 불변·주입 0·상태(alt·장부·통과 모드) 불변", () => {
    const f = mac();
    feedS(f, "\x1b[?1000h");
    feedS(f, "\x1b[?1049h");
    expect(feedS(f, "\x1b[!p")).toBe("\x1b[!p"); // 무처리 — 소등 주입 없음
    expect(feedS(f, "\x1b[?1002h")).toBe("\x1b[?1002h"); // alt 유지 — 여전히 통과
    expect(feedS(f, "\x1b[?1049l")).toBe(`\x1b[?1049l${ALL_OFF}`); // 진짜 복귀는 l 이 한다
    expect(feedS(f, "\x1b[?1049h")).toBe("\x1b[?1049h\x1b[?1000h\x1b[?1002h"); // 장부 무손상
  });
});

describe("정합기 — reset() 의미론 (스펙 D4 ③)", () => {
  it("반환 = 8종 전 집합 상수 DECRST·1049l 미포함, 장부는 소거된다", () => {
    const f = mac();
    feedS(f, "\x1b[?1000h\x1b[?1006h");
    const r = f.reset();
    expect(r).toBe(ALL_OFF);
    expect(r.includes("1049")).toBe(false);
    expect(feedS(f, "\x1b[?1049h")).toBe("\x1b[?1049h"); // 소거 입증 — 재생 빈손
  });
  it("reset() 은 altActive 를 보존한다(스트림 관측 진실) — alt 중 통과 모드 유지", () => {
    const f = mac();
    feedS(f, "\x1b[?1049h");
    f.reset();
    expect(feedS(f, "\x1b[?1000h")).toBe("\x1b[?1000h"); // 여전히 alt 통과
  });
  it("ledgerWantsMouse: h 존재 시 true → 명시 l·reset 후 false", () => {
    const f = mac();
    expect(f.ledgerWantsMouse()).toBe(false);
    feedS(f, "\x1b[?1003h");
    expect(f.ledgerWantsMouse()).toBe(true);
    feedS(f, "\x1b[?1003l");
    expect(f.ledgerWantsMouse()).toBe(false);
    feedS(f, "\x1b[?1003h");
    f.reset();
    expect(f.ledgerWantsMouse()).toBe(false);
  });
});

describe("정합기 — 에포크 가드 세대 번호 (스펙 D4 ② agent.exited)", () => {
  it("세대는 명시 마우스 h/l·alt 전이 파라미터·RIS 에서만 증가", () => {
    const f = mac();
    const g0 = f.generation();
    feedS(f, "본문\x1b[31m색\x1b[?25h\x1b[!p"); // 텍스트·SGR·비마우스 DECSET·DECSTR — 불변
    expect(f.generation()).toBe(g0);
    feedS(f, "\x1b[?1000h");
    expect(f.generation()).toBe(g0 + 1);
    feedS(f, "\x1b[?1049h");
    expect(f.generation()).toBe(g0 + 2);
    feedS(f, "\x1bc");
    expect(f.generation()).toBe(g0 + 3);
  });
  it("clearLedgerIfGeneration: 세대 불변 → 소거 true / 그 사이 관측 → 보존 false", () => {
    const f = mac();
    feedS(f, "\x1b[?1000h");
    const g = f.generation();
    feedS(f, "\x1b[?1006h"); // 이벤트 수신~적용 사이 새 앱 관측을 모사(세대 이동)
    expect(f.clearLedgerIfGeneration(g)).toBe(false);
    expect(feedS(f, "\x1b[?1049h")).toBe("\x1b[?1049h\x1b[?1000h\x1b[?1006h"); // 장부 보존
    feedS(f, "\x1b[?1049l");
    expect(f.clearLedgerIfGeneration(f.generation())).toBe(true);
    expect(feedS(f, "\x1b[?1049h")).toBe("\x1b[?1049h"); // 소거 입증
  });
});

describe("정합기 — 전이 걸친 carry 경계 스윕 (앵커④ 불변)", () => {
  const sample = "\x1b[?1000h안\x1b[?1049;1006h본\x1b[?1002h문\x1b[?1049l끝\x1bc\x1b[?1003h";
  const wholeOf = (s: string) => {
    const f = mac();
    return asStr(f.feed(bytes(s))) + asStr(f.flush());
  };
  it("전수 분할 스윕(2분할): 어디서 잘라도 통짜와 동일 — 주입 포함", () => {
    const whole = wholeOf(sample);
    const raw = bytes(sample);
    for (let cut = 0; cut <= raw.length; cut++) {
      const f = mac();
      const got = asStr(f.feed(raw.slice(0, cut))) + asStr(f.feed(raw.slice(cut))) + asStr(f.flush());
      expect(got).toBe(whole);
    }
  });
  it("3분할 스텝 스윕(이중 경계)", () => {
    const whole = wholeOf(sample);
    const raw = bytes(sample);
    for (let c1 = 0; c1 <= raw.length; c1 += 3) {
      for (let c2 = c1; c2 <= raw.length; c2 += 3) {
        const f = mac();
        const got =
          asStr(f.feed(raw.slice(0, c1))) +
          asStr(f.feed(raw.slice(c1, c2))) +
          asStr(f.feed(raw.slice(c2))) +
          asStr(f.flush());
        expect(got).toBe(whole);
      }
    }
  });
});

describe("정합기 — CARRY_CAP fail-open 불변 (소비 활성에서도)", () => {
  it("캡 초과 후보는 원문 방류(주입·지연 0) — primary/alt 양쪽", () => {
    const long = "\x1b[?" + "1".repeat(64) + "h";
    const f = mac();
    expect(feedS(f, long)).toBe(long);
    feedS(f, "\x1b[?1049h");
    expect(feedS(f, long)).toBe(long);
  });
});

describe("win-parity — 소비 비활성 출력 = 현행 filterChunk 와 byte-identical (스펙 D4)", () => {
  // 소비 비활성 3형(무인자·os:'win'·reconcile:false)은 장부를 기록하되 출력은 v1 그대로여야
  // 한다. 휠 핸들러의 win 미등록은 main.ts 배선(IS_WINDOWS 게이트) — wheelgate.test.ts 가
  // 술어 측을, 이 핀이 출력 측을 고정한다.
  const parityOpts = [
    undefined,
    { os: "win", reconcile: true },
    { os: "win", reconcile: false },
    { os: "mac", reconcile: false }, // 롤백 스위치 — win 비활성 경로 재사용(신규 로직 0)
  ] as const;
  const corpus = [
    "앞\x1b[?1003h중간\x1b[?1006h뒤",
    "x\x1b[?1049;1000hy\x1b[?1049l",
    "\x1b[?47h\x1b[?1002h\x1b[?47l\x1b[?1047h\x1b[?1047l",
    "A\x1bcB\x1b[!p\x1b[?1000$p",
    "\x1b[?2004h붙\x1b[?1015;1016l\x1b[?25h",
    "\x1b[?" + "9".repeat(64) + "h말미\x1b[?1000h",
  ];
  const pureWhole = (s: string) => {
    const r = filterChunk(bytes(s));
    return asStr(r.out) + asStr(r.carry); // v1 순수 계산 + 종단 방류(flush 동형)
  };
  for (const [oi, opts] of parityOpts.entries()) {
    it(`opts#${oi}(${JSON.stringify(opts ?? "무인자")}) — 코퍼스 전건 통짜 동일`, () => {
      for (const s of corpus) {
        const f = new MouseTrackingFilter(opts as ConstructorParameters<typeof MouseTrackingFilter>[0]);
        const got = asStr(f.feed(bytes(s))) + asStr(f.flush());
        expect(got).toBe(pureWhole(s));
      }
    });
  }
  it("경계 스윕까지 동일(win 소비 비활성 — 대표 샘플 전수 2분할)", () => {
    const s = "x\x1b[?1049;1000hy\x1bc\x1b[?1006lz";
    const whole = pureWhole(s);
    const raw = bytes(s);
    for (let cut = 0; cut <= raw.length; cut++) {
      const f = new MouseTrackingFilter({ os: "win", reconcile: true });
      const got = asStr(f.feed(raw.slice(0, cut))) + asStr(f.feed(raw.slice(cut))) + asStr(f.flush());
      expect(got).toBe(whole);
    }
  });
});
