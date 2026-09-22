// OS 드롭 좌표·진단·경로 오류·배선 계약 (bun test — DOM/Tauri 불요).
// RED 먼저: droppoint.ts 및 ftdrop.ts 신규 export는 다음 구현 호출에서 추가한다.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  dropPlatformFromUserAgent,
  dropPointToCss,
  dropDebugDetail,
  isDropDebugEnabled,
  type DropPlatform,
} from "./droppoint";
import { classifyPathCheck, pathCheckToast, type PathCheck } from "./ftdrop";

describe("OS 드롭 플랫폼 — UA로 좌표 단위를 고른다", () => {
  test("Windows UA는 windows", () => {
    expect(dropPlatformFromUserAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe("windows");
  });
  test("Macintosh·Mac OS X UA는 macos", () => {
    expect(dropPlatformFromUserAgent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15")).toBe("macos");
    expect(dropPlatformFromUserAgent("Macintosh")).toBe("macos");
    expect(dropPlatformFromUserAgent("Mac OS X 10_15_7")).toBe("macos");
  });
  test("Linux UA와 그 외 문자열은 linux", () => {
    expect(dropPlatformFromUserAgent("Mozilla/5.0 (X11; Linux x86_64) …")).toBe("linux");
    expect(dropPlatformFromUserAgent("")).toBe("linux");
    expect(dropPlatformFromUserAgent("unknown")).toBe("linux");
  });
  test("Windows 토큰은 Mac 토큰보다 우선하고 대소문자를 가리지 않는다", () => {
    expect(dropPlatformFromUserAgent("Windows Macintosh Mac OS X")).toBe("windows");
    expect(dropPlatformFromUserAgent("windows macintosh")).toBe("windows");
    expect(dropPlatformFromUserAgent("macintosh")).toBe("macos");
  });
});

const PLATFORMS: DropPlatform[] = ["macos", "windows", "linux"];

describe("OS 드롭 좌표 — Windows만 물리 픽셀을 CSS px로 환산한다", () => {
  const raw = { x: 1014.8, y: 429.0 };
  for (const platform of PLATFORMS) {
    for (const dpr of [1, 2]) {
      test(`${platform} · dpr=${dpr} 좌표 매트릭스`, () => {
        const expected = platform === "windows"
          ? { x: raw.x / dpr, y: raw.y / dpr }
          : raw;
        expect(dropPointToCss(raw, dpr, platform)).toEqual(expected);
      });
    }
    for (const dpr of [0, NaN, Infinity, -Infinity, -2]) {
      test(`${platform} · 유한 양수가 아닌 dpr=${dpr}는 1로 취급한다`, () => {
        expect(dropPointToCss(raw, dpr, platform)).toEqual(raw);
      });
    }
  }
});

// V1 a08 실측 중심 좌표. viewportWidth/columns는 사이드바·간격도 포함하므로
// 실제 pane 폭의 보수적인 상한이다. r1c1은 대조군이며 반 폭 이상 오차 단언은
// 오배달이 확인된 오른쪽/가운데 열 세 표본에만 적용한다.
const A08_CENTERS = [
  { label: "1280x820 · 2x1 · r1c1", center: { x: 484.2, y: 429.0 }, viewportWidth: 1280, columns: 2, misdelivered: false },
  { label: "1280x820 · 2x1 · r1c2", center: { x: 1014.8, y: 429.0 }, viewportWidth: 1280, columns: 2, misdelivered: true },
  { label: "1728x1051 · 3x2 · r1c2", center: { x: 973.5, y: 291.2 }, viewportWidth: 1728, columns: 3, misdelivered: true },
  { label: "1728x1051 · 3x2 · r2c2", center: { x: 973.5, y: 797.8 }, viewportWidth: 1728, columns: 3, misdelivered: true },
];

describe("a08 실측 표본 — Retina 논리좌표의 이중 축소를 막는다", () => {
  for (const { label, center, viewportWidth, columns, misdelivered } of A08_CENTERS) {
    test(`${label} · macOS 중심을 보존하고 종전 /2 공식과 다르다`, () => {
      const css = dropPointToCss(center, 2, "macos");
      const oldCss = { x: center.x / 2, y: center.y / 2 };
      expect(css).toEqual(center);
      expect(css).not.toEqual(oldCss);
      if (misdelivered) {
        expect(Math.abs(css.x - oldCss.x)).toBeGreaterThanOrEqual(viewportWidth / columns / 2);
      }
    });
    test(`${label} · Windows 물리좌표(css×2)는 같은 중심으로 환산한다`, () => {
      expect(dropPointToCss({ x: center.x * 2, y: center.y * 2 }, 2, "windows")).toEqual(center);
    });
  }
});

describe("드롭 진단 — 좌표 근거를 담고 명시한 스위치에서만 켠다", () => {
  for (const platform of PLATFORMS) {
    test(`${platform} · raw x/y·dpr·플랫폼·CSS x/y를 모두 담는다`, () => {
      const raw = { x: 2029.6, y: 858 };
      // dpr 문자열이 좌표 숫자 안에 우연히 포함되면 누락을 잡지 못하므로 별도 값 사용.
      const dpr = 1.25;
      const css = { x: 1623.68, y: 686.4 };
      const detail = dropDebugDetail(raw, dpr, platform, css);
      for (const value of [raw.x, raw.y, dpr, platform, css.x, css.y]) {
        expect(detail).toContain(String(value));
      }
      expect(detail).toContain("dpr");
    });
  }
  test("localStorage 값이 정확히 1일 때만 진단을 켠다", () => {
    expect(isDropDebugEnabled("1")).toBe(true);
    for (const stored of [null, undefined, "0", "true", "", " 1", "1 ", "01"]) {
      expect(isDropDebugEnabled(stored)).toBe(false);
    }
  });
});

describe("경로 검사 — 조회 실패와 항목 부재를 구분한다", () => {
  test("invoke reject를 나타내는 null은 unreadable", () => {
    expect(classifyPathCheck(null, "a")).toBe("unreadable");
  });
  test("정상 목록에 이름이 없거나 빈 목록이면 missing", () => {
    expect(classifyPathCheck([{ name: "a" }], "b")).toBe("missing");
    expect(classifyPathCheck([], "a")).toBe("missing");
  });
  test("정상 목록에 이름이 있으면 ok", () => {
    expect(classifyPathCheck([{ name: "a" }], "a")).toBe("ok");
    expect(classifyPathCheck([{ name: "b" }, { name: "a" }], "a")).toBe("ok");
  });
});

describe("경로 오류 토스트 — 제목·경로·다중 순번·원인을 보존한다", () => {
  const p = "/Users/user/proj/gone.bin";
  const kinds: Exclude<PathCheck, "ok">[] = ["unreadable", "missing"];
  test("조회 실패와 항목 부재의 제목이 서로 다르다", () => {
    const unreadable = pathCheckToast("unreadable", p, 0, 1);
    const missing = pathCheckToast("missing", p, 0, 1);
    expect(unreadable.name).toBe("경로 확인 실패");
    expect(missing.name).toBe("경로가 더 이상 없음");
    expect(unreadable.name).not.toBe(missing.name);
  });
  for (const kind of kinds) {
    test(`${kind} · 다중 경로는 경로와 두 번째 항목 2/3을 담는다`, () => {
      // idx는 배열 인덱스(0부터), 사용자에게 표시하는 순번은 1부터다.
      const toast = pathCheckToast(kind, p, 1, 3);
      expect(toast.detail).toContain(p);
      expect(toast.detail).toContain("2/3");
    });
    test(`${kind} · 단일 경로에는 순번을 붙이지 않는다`, () => {
      const toast = pathCheckToast(kind, p, 0, 1);
      expect(toast.detail).toContain(p);
      expect(/\b\d+\/\d+\b/.test(toast.detail)).toBe(false);
    });
  }
  test("조회 실패의 원인 문자열이 있으면 본문에 보존한다", () => {
    const err = "EACCES: permission denied";
    const toast = pathCheckToast("unreadable", p, 1, 3, err);
    expect(toast.detail).toContain(p);
    expect(toast.detail).toContain("2/3");
    expect(toast.detail).toContain(err);
  });
});

const src = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
// wswiring.test.ts 관례: 주석 속 함수명·이벤트명으로 배선 핀을 통과시키지 않는다.
// tauri:// 문자열은 보존하고 블록 주석과 행 주석만 걷어낸다.
const code = src
  .replace(/\/\*[\s\S]*?\*\//g, "")
  .split("\n")
  .map((line) => line.trimStart().startsWith("//") ? "" : line.replace(/\s\/\/.*$/, ""))
  .join("\n");

function functionSlice(name: string): string {
  const start = code.indexOf(`function ${name}(`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = code.indexOf("\n}\n", start);
  expect(end).toBeGreaterThan(start);
  return code.slice(start, end + 3);
}

describe("OS 드롭 배선 — 순수 계약이 실제 UI에 연결된다", () => {
  test("paneAtPointStrict는 dropPointToCss를 쓰고 직접 /dpr 하지 않는다", () => {
    const body = functionSlice("paneAtPointStrict");
    expect(body).toContain("dropPointToCss(");
    expect(/\/\s*dpr\b/.test(body)).toBe(false);
  });
  for (const event of ["tauri://drag-enter", "tauri://drag-over", "tauri://drag-leave"]) {
    test(`${event} 리스너가 코드에 존재한다`, () => {
      expect(code).toContain(`"${event}"`);
    });
  }
  test("진단 localStorage 키 cysDropDebug가 연결된다", () => {
    expect(code).toContain("cysDropDebug");
  });
  test("injectPathsToPane은 경로 분류·문안을 쓰고 reject 사유를 삼키지 않는다", () => {
    const body = functionSlice("injectPathsToPane");
    expect(body).toContain("classifyPathCheck(");
    expect(body).toContain("pathCheckToast(");
    expect(/\.catch\(\s*\(\s*\)\s*=>\s*null\s*\)/.test(body)).toBe(false);
  });
  test("원문의 모든 플랫폼 물리 픽셀이라는 주석을 정정한다", () => {
    expect(src).not.toContain("payload.position=물리 픽셀");
    expect(src).not.toContain("드롭 물리좌표(디바이스 픽셀)");
  });
  test("기존 pane.drop-target 하이라이트 CSS 규칙을 재사용한다", () => {
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf-8")
      .replace(/\/\*[\s\S]*?\*\//g, "");
    expect(/\.pane\.drop-target\s*\{/.test(css)).toBe(true);
  });
});
