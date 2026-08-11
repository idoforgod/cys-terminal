// trackfilter.ts 회귀 테스트 — 마우스 트래킹 DECSET 스트리핑의 3대 계약을 핀한다:
//  (1) 마우스 모드 전환(9·1000·1002·1003·1005·1006·1015·1016 h/l)은 반드시 걷어낸다.
//  (2) 그 외 바이트는 **단 하나도** 바꾸지 않는다(비마우스 DECSET·일반 CSI·텍스트·색상).
//  (3) 청크를 어디서 잘라도 결과가 통짜 처리와 바이트 동일하다(경계 전수 스윕 — 앵커④:
//      홀드백 결함=화면 백지화이므로 이 계약이 이 파일의 존재 이유다).
import { describe, it, expect } from "bun:test";
import { filterChunk, MouseTrackingFilter } from "./trackfilter";

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
