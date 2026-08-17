// 타입 게이트 **자기 방어** 핀 (2026-08-17 · 적대검증 2R major 수리)
//
// ★고친 결함: 릴리스 게이트 5(`cd ui && bunx tsc -p tsconfig.check.json` exit 0)가 파일 하나
//   (`ui/src/bun-env.d.ts`)에 통째로 의존하는데, **그 의존을 지키는 감시선이 0건**이었다.
//   그 파일이 커밋에서 빠지거나 "안 쓰는 파일"로 정리되면 타입 검증은 즉시 red 로 되돌아가는데
//   (실측 2026-08-17: 그 파일만 치우면 25 에러 · exit 1 — 되돌리면 0), 그 사실을 알려 주는
//   레인이 없었다:
//     · 타입 게이트는 **수동 게이트**다 — `bunx tsc` 를 돌리는 워크플로가 0건이다
//       (실측: `grep -n "bun test\|bunx tsc" .github/workflows/*.yml` → 히트는 release.yml
//        145·147 두 줄뿐이고 그 둘은 같은 `bun test` 스텝의 name·run 이다. `bunx tsc` 0건).
//     · 그래서 누락은 **다음 사람이 손으로 게이트를 돌릴 때**까지 조용하고, 그때는 이미 상시 red 다.
//       상시 red 인 게이트는 아무도 보지 않는다(=게이트가 아니다) — 두 파일의 `_why` 가 못박은 명제.
//   ∴ CI 에서 ui/ 의 **정확성을 검사하는 유일한 레인**(`bun test`)에 그 의존을 옮겨 심는다.
//   (ui/ 를 만지는 다른 CI 스텝은 release.yml:491 의 tauri build 뿐인데, 그것은 `ui/build.sh`
//    경유 **트랜스파일**이라 타입을 보지 않는다 — tsconfig.check.json 의 `_why` 가 못박은 사실.
//    게다가 `.d.ts` 는 어디서도 import 되지 않아 번들러의 시야에 아예 들어오지 않는다.)
//   여기서 깨지면 전체 스위트가 30ms 대에 끝나는 그 레인에서, 정확한 파일명과 복구 방법과
//   함께 즉시 드러난다.
//
// ★이 핀이 하지 **않는** 것(정직): 타입 검증을 대신하지 않는다. 여기 있는 것은 '게이트의 전제가
//   트리에 실존하는가'뿐이고, 타입이 실제로 0 에러인지는 여전히 `bunx tsc -p tsconfig.check.json`
//   만이 답한다. 특히 이 핀은 커밋에서 파일이 빠지는 것을 **막지 못한다** — 빠졌음을 **즉시 알릴**
//   뿐이다(막는 것은 `git add ui/src/bun-env.d.ts` 를 하는 사람의 몫이다).
//
// ★런타임 코드 0줄 계약: 이 파일도 bun-env.d.ts 처럼 본체에서 import 되지 않는다
//   (`bun build src/main.ts` 산출물 무영향 · `bun test` 만 실행).
import { describe, it, expect } from "bun:test";
import { existsSync, readFileSync } from "node:fs";

// 경로는 이 파일 기준 상대(URL) — cwd 비의존(레포 루트/ui 어디서 돌려도 동일. mousefilter.test.ts 관례).
const AMBIENT_URL = new URL("./bun-env.d.ts", import.meta.url);
const TSCONFIG_URL = new URL("../tsconfig.check.json", import.meta.url);

describe("타입 게이트 전제 — bun-env.d.ts 와 tsconfig.check.json 은 한 쌍이다", () => {
  it("ui/src/bun-env.d.ts 가 트리에 존재한다 (없으면 타입 게이트가 red 로 되돌아간다)", () => {
    expect(existsSync(AMBIENT_URL)).toBe(true);
    // ↑ 여기서 실패했다면: 커밋·리베이스·정리에서 그 파일이 빠졌다는 뜻이다.
    //   복구 — 파일이 워킹트리에 있으면 `git add ui/src/bun-env.d.ts` (미추적 상태로 남기지 마라).
    //   파일 자체가 사라졌으면 그 머리말의 채택 근거부터 읽어라(재작성 대상이지 삭제 대상이 아니다).
  });

  it("선언 3종(bun:test · node:fs · Buffer)이 살아 있다 — 그 에러를 없애는 것이 정확히 이 셋이다", () => {
    const src = readFileSync(AMBIENT_URL, "utf-8");
    // 구조 앵커만 본다(문안·matcher 목록은 자유롭게 늘어난다). 이 셋 중 하나라도 사라지면
    // 해당 부류의 에러가 통째로 되살아나므로, 여기 실패는 게이트 실패와 동치다
    // (실측 코드 분포 — 파일 전체를 치웠을 때 TS2307 20건 + TS2591 5건).
    expect(src).toContain('declare module "bun:test"');
    expect(src).toContain('declare module "node:fs"');
    expect(src).toContain("declare const Buffer");
  });

  it("tsconfig.check.json 의 include 가 여전히 그 파일을 덮는다 (앰비언트 공급의 유일한 기전)", () => {
    // `.d.ts` 는 **어디서도 import 되지 않는다** — include 범위 안에 있다는 것만으로 타입을 공급한다.
    // 그래서 include 를 좁히면(예: 테스트 파일만, 또는 개별 파일 나열) 파일이 그대로 있어도
    // 게이트는 red 로 되돌아간다. 그 축은 파일 존재 검사로는 절대 잡히지 않으므로 여기서 본다.
    const cfg = JSON.parse(readFileSync(TSCONFIG_URL, "utf-8")) as { include?: string[] };
    expect(cfg.include ?? []).toContain("src/**/*.ts");
  });
});
