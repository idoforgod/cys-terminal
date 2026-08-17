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
  // 한다. main.ts 는 Windows 에도 휠 핸들러를 등록하지만(별도 win 전용 술어
  // shouldSuppressWheelWin) 그것은 **입력 이벤트 측**이고, 이 핀은 **출력 바이트 측**을
  // 고정한다 — wheelgate.test.ts 가 술어 측을, 이 핀이 출력 측을 맡는 분업은 그대로다.
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

// ★정정 이력 — 위 win-parity 블록의 산문 한 줄이 한때 "휠 핸들러의 win 미등록은 main.ts 배선"
//   이라고 적혀 있었고 그것은 거짓이 됐다(main.ts 는 Windows 에도 등록한다 — 별도 win 전용 술어
//   wheelgate.shouldSuppressWheelWin). 이번 라운드에 **문장을 사실로 고쳤다**. 무수정이 계약인
//   것은 그 블록의 `expect` 식(출력 바이트 회귀 감시선)이지 그것을 설명하는 산문이 아니다 —
//   거짓이 된 산문을 남기는 것은 감시선 보존이 아니라 결함이다. 단언은 한 줄도 바뀌지 않았다.
//   ★오히려 이 핀이 win 휠 가드의 **전제**를 지킨다: 출력 소비가 계속 비활성이어야 xterm 이
//   Windows 에서 트래킹에 진입하지 못하고(mouseTrackingMode="none"), 그래야 win 술어의
//   `!xtermTracking` 항이 상수 true 로 유지된다. 이 핀이 깨지면 win 가드의 상태 모델도 함께 깨진다.
describe("ledgerWantsAnyMotion — 1003 단독 조회 (C-1: Windows 휠 억제 술어 입력)", () => {
  // 이 접근자는 ledgerWantsMouse('아무 마우스 파라미터나 하나')보다 **좁다**. 좁혀야 하는 이유가
  // 이 describe 의 존재 이유다: Claude Code 는 fullscreen 진입 시 1003 을 켜지만 vim `mouse=a`
  // 는 통상 켜지 않으므로, 1003 을 술어로 삼아야 Windows 휠 억제가 claude 에만 걸리고 vim 의
  // 방향키 합성 UX 는 그대로 보존된다. ★vim 의 실제 DECSET 캡처는 저장소에 0건 — 이 전제가
  // 이 설계의 최대 미확정이고, 아래 "vim 시나리오" 케이스는 그 **전제를 명문화한 핀**이다.
  it("초기 장부는 비어 있다 → false", () => {
    expect(mac().ledgerWantsAnyMotion()).toBe(false);
    expect(new MouseTrackingFilter().ledgerWantsAnyMotion()).toBe(false);
  });
  it("1003h 뒤 true / 명시 1003l 뒤 false", () => {
    const f = mac();
    feedS(f, "\x1b[?1003h");
    expect(f.ledgerWantsAnyMotion()).toBe(true);
    feedS(f, "\x1b[?1003l");
    expect(f.ledgerWantsAnyMotion()).toBe(false);
  });
  it("claude 시나리오(1000;1002;1003;1006h — 'full') → true", () => {
    const f = mac();
    feedS(f, "\x1b[?1000;1002;1003;1006h");
    expect(f.ledgerWantsAnyMotion()).toBe(true);
    expect(f.ledgerWantsMouse()).toBe(true);
  });
  it("★vim 시나리오(mouse=a: 1000/1002/1006 만 — 1003 부재) → false, 단 wantsMouse 는 true", () => {
    // 두 접근자가 갈리는 지점 = 억제/비억제가 갈리는 지점. wantsMouse 를 그대로 썼다면
    // vim 에서도 휠이 억제돼 현행 UX 가 회귀한다 — 그래서 좁은 술어가 필요하다.
    const f = mac();
    feedS(f, "\x1b[?1000h\x1b[?1002h\x1b[?1006h");
    expect(f.ledgerWantsAnyMotion()).toBe(false);
    expect(f.ledgerWantsMouse()).toBe(true);
  });
  it("1003 만 끈 뒤에도 다른 파라미터는 남는다 — 분리 입증(claude→부분 해제)", () => {
    const f = mac();
    feedS(f, "\x1b[?1000;1002;1003;1006h");
    feedS(f, "\x1b[?1003l");
    expect(f.ledgerWantsAnyMotion()).toBe(false);
    expect(f.ledgerWantsMouse()).toBe(true); // 1000/1002/1006 은 여전히 h
  });
  it("RIS(ESC c) 는 장부를 소거한다 → false", () => {
    const f = mac();
    feedS(f, "\x1b[?1003h");
    feedS(f, "\x1bc");
    expect(f.ledgerWantsAnyMotion()).toBe(false);
  });
  it("reset() 뒤 false (장부 소거 — 리셋 훅 경로)", () => {
    const f = mac();
    feedS(f, "\x1b[?1003h");
    f.reset();
    expect(f.ledgerWantsAnyMotion()).toBe(false);
  });
  it("clearLedgerIfGeneration 소거 뒤에도 false (에포크 가드 경로)", () => {
    const f = mac();
    feedS(f, "\x1b[?1003h");
    expect(f.clearLedgerIfGeneration(f.generation())).toBe(true);
    expect(f.ledgerWantsAnyMotion()).toBe(false);
  });
  it("소비 비활성(win)에서도 장부 기록은 공통이므로 조회된다 — 실사용 대상이 Windows 다", () => {
    // 술어의 유일한 소비자가 Windows 휠 게이트라, 소비 비활성 인스턴스에서 장부가 비면
    // 술어가 영구 false 가 돼 기능 자체가 죽는다. 장부 기록의 OS 무관성을 여기서 핀한다.
    const f = new MouseTrackingFilter({ os: "win", reconcile: true });
    expect(feedS(f, "\x1b[?1003h")).toBe(""); // 출력은 v1 스트리핑 그대로(win-parity 무접촉)
    expect(f.ledgerWantsAnyMotion()).toBe(true);
  });
  it("alt 구간에서 통과된 1003h 도 장부에 오른다(방출 경로와 무관하게 기록)", () => {
    const f = mac();
    feedS(f, "\x1b[?1049h");
    expect(feedS(f, "\x1b[?1003h")).toBe("\x1b[?1003h"); // alt 중 원문 통과
    expect(f.ledgerWantsAnyMotion()).toBe(true);
  });
});

// ★alt 이탈 장부 소거 — Windows 휠 가드 stale 봉인 (적대검증 2R major).
//
// 왜 이 핀이 필요한가: Windows 는 `!xtermTracking` 항이 상수 true 라(wheelgate.ts (b))
// 억제를 푸는 해제항이 없다. 그래서 장부에 1003 이 남으면 **다음** 전체화면 앱(less·man·vim)
// 이 alt 에 들어가는 순간 술어가 충족돼 그 pane 이 닫힐 때까지 휠이 죽었다. 소비 비활성
// 경로에는 재생 주입이 없어 장부를 alt 이탈 후까지 보존할 이유가 하나도 없다 — 그래서 지운다.
// mac(소비 활성)은 그 장부가 재생 주입의 원본이므로 **보존이 계약**이고, 위 :188 블록의
// suspend/resume 핀이 그것을 이미 고정하고 있다. 이 블록은 두 경로가 갈리는 지점을 못박는다.
describe("alt 이탈 장부 소거 — 소비 비활성(win) 한정 (휠 가드 stale 봉인)", () => {
  for (const t of [47, 1047, 1049]) {
    it(`win: ?${t}l 이탈 시 1003 이 내려간다 → 다음 alt 앱에 새지 않는다`, () => {
      const f = new MouseTrackingFilter({ os: "win", reconcile: true });
      feedS(f, "\x1b[?1000;1002;1003;1006h"); // claude 'full'
      feedS(f, `\x1b[?${t}h`);
      expect(f.ledgerWantsAnyMotion()).toBe(true); // alt 중에는 억제가 걸려야 한다
      feedS(f, `\x1b[?${t}l`); // 앱이 alt 를 떠난다(1003l 을 보내지 않은 채)
      expect(f.ledgerWantsAnyMotion()).toBe(false); // ← 이것이 없으면 휠 영구 사망
      expect(f.ledgerWantsMouse()).toBe(false);
      // 다음 앱(less: 트래킹 무요청)이 alt 에 들어가도 술어는 불충족이어야 한다.
      feedS(f, `\x1b[?${t}h`);
      expect(f.ledgerWantsAnyMotion()).toBe(false);
    });
  }
  it("★win: alt 진입을 못 본 상태(재부착)에서도 이탈 관측이면 소거 — 재부착 사각 봉인", () => {
    // 재부착이 보내는 초기 화면은 셀 내용 스냅샷뿐이라 alt 진입 시퀀스가 재발행되지 않는다
    // → 이미 전체화면인 앱에 붙어도 내부 altActive 는 false 로 시작한다. 그 상태에서 앱이
    // 마우스 DECSET 을 다시 내면 장부에 1003 이 오르는데, 이어지는 ?1049l 을 '전이 아님'으로
    // 흘려보내면 장부가 영구 잔존해 **다음** 앱부터 휠이 죽는다. 소거 조건을 altActive 에
    // 묶지 않는 이유가 이것이다(processChunk 의 [전이] 블록 주석).
    // 오판 방향은 안전하다: 잘못 지우면 비억제(=가드 도입 전 동작)로 떨어질 뿐이고, 앱이
    // 마우스 모드를 다시 선언하면 즉시 재무장한다.
    const f = new MouseTrackingFilter({ os: "win", reconcile: true });
    feedS(f, "\x1b[?1003h"); // 재부착 후 앱이 다시 선언(진입 시퀀스는 못 봤다)
    expect(f.ledgerWantsAnyMotion()).toBe(true);
    expect(feedS(f, "\x1b[?1049l")).toBe("\x1b[?1049l"); // 출력은 v1 그대로
    expect(f.ledgerWantsAnyMotion()).toBe(false);
    // 재무장 확인 — 앱이 다시 선언하면 술어가 되살아난다(영구 무력화가 아니다).
    feedS(f, "\x1b[?1049h\x1b[?1003h");
    expect(f.ledgerWantsAnyMotion()).toBe(true);
  });
  it("★mac(소비 활성)은 이탈에도 장부를 보존한다 — 재생 주입 원본(비대칭 확정)", () => {
    const f = mac();
    feedS(f, "\x1b[?1003h");
    feedS(f, "\x1b[?1049h");
    feedS(f, "\x1b[?1049l");
    expect(f.ledgerWantsAnyMotion()).toBe(true); // mac 은 suspend/resume 재생을 위해 유지
    expect(feedS(f, "\x1b[?1049h")).toBe("\x1b[?1049h\x1b[?1003h"); // 재생이 빈손이 아님
  });
  it("★출력 바이트 무변경 — 소거는 win-parity 계약에 닿지 않는다", () => {
    // 이 소거가 출력에 한 바이트라도 보태면 win-parity(스펙 D4)가 깨진다. 위 :386 블록의
    // 코퍼스가 이미 통짜로 고정하지만, 소거를 넣은 이 전이 자체를 여기서 직접 못박는다.
    const f = new MouseTrackingFilter({ os: "win", reconcile: true });
    feedS(f, "\x1b[?1003h\x1b[?1049h");
    expect(feedS(f, "\x1b[?1049l")).toBe("\x1b[?1049l"); // 소등 주입 없음(=v1 그대로)
  });
  it("정합기 롤백(mac·reconcile=false)도 같은 비소비 경로 — 신규 분기 0", () => {
    const f = new MouseTrackingFilter({ os: "mac", reconcile: false });
    feedS(f, "\x1b[?1003h\x1b[?1049h");
    expect(feedS(f, "\x1b[?1049l")).toBe("\x1b[?1049l");
    expect(f.ledgerWantsAnyMotion()).toBe(false);
  });
});
