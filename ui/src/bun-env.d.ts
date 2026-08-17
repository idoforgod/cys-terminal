// 타입 검증 전용 앰비언트 선언 — **런타임 코드 0줄**(.d.ts 는 어디서도 import 되지 않으므로
// `bun build src/main.ts` 산출물에 들어가지 않고, `bun test` 는 *.test.ts 만 실행한다).
//
// ★왜 이 파일이 필요한가(고친 결함): 릴리스 게이트 문구는 "타입체크 0 에러"인데, 문서화된 명령
//   `cd ui && bunx tsc -p tsconfig.check.json` 은 **구조적으로 0 에러가 불가능**했다 — 30 에러 중
//   23건이 테스트 파일의 'bun:test / Buffer / node:fs' 타입 부재였고, 그 23건은 아래 두 제약이
//   동시에 걸려 패키지 설치로도 사라지지 않는다:
//     · tsconfig.check.json 의 `"types": []` 가 @types 자동 포함을 차단한다(명시 의도).
//     · 같은 파일의 `_why` 가 "typescript 를 devDependencies 에 넣지 않은 이유: bun.lock 을
//       건드리지 않고(오프라인 빌드 계약 보존) 검증만 온디맨드로 돌리기 위해서다"를 계약으로
//       못박고 있다 → @types/bun·@types/node 설치는 그 계약 위반이다(실측: 설치해도 여전히 30).
//   ∴ 의존성을 늘리지 않고 타입만 채우는 **손으로 적은 앰비언트 선언**이 이 제약들과 양립하는
//   유일한 수단이다. 게이트가 상시 red 면 아무도 안 보고, 안 보는 게이트는 게이트가 아니다.
//
// ★범위 계약(정직 — 이 파일은 bun-types 의 대체물이 아니다): 저장소의 테스트가 **실제로 쓰는
//   API 만** 선언한다. 새 API(mock·beforeEach·spyOn 등)를 쓰기 시작하면 여기 추가해야 한다 —
//   그때 "타입이 없다"는 에러가 나는 것이 정상이고, 그것이 이 파일을 최소로 유지하는 장치다.
// ★matcher 는 **의도적으로 느슨하다**: bun 의 진짜 matcher 는 기대값 타입을 실제값에 맞춰
//   검사하지만, 그 정밀도를 손으로 재현하면 오탐(있지도 않은 에러)이 나 게이트를 다시 못 쓰게
//   만든다. 여기 목적은 '테스트 파일이 컴파일되어 **본체 소스에 대한 호출**이 타입 검사를 받게
//   하는 것'이다 — 예: wheelgate.test.ts 가 shouldSuppressWheelWin 에 없는 필드를 넘기면 잡힌다.
//   matcher 인자 자체의 오타는 이 게이트가 아니라 `bun test` 실행이 잡는다.
//
// ★채택 판정 실측(2026-08-17 — 이 파일은 스톨한 에이전트의 잔여물이었고, 아래 수치로 '유지'가
//   확정됐다. 재현 명령은 전부 `cd ui` 기준):
//   · HEAD 기준선  `git archive HEAD ui` 사본 + node_modules 링크 → **30 에러**
//     (내역: 테스트 타입 부재 23 + main.ts 의 'Property … does not exist on type never' 7).
//   · 현 워킹트리에서 이 파일만 잠시 치우면 → **23 에러**〔당시 수치 — 아래 ★재실측이 갱신한다〕
//     (main.ts 7건은 같은 라운드의 main.ts 수리로 이미 사라졌다 — 이 파일과 무관).
//   · 이 파일을 되돌려 놓으면 → **0 에러 · exit 0**. 즉 그 23건을 정확히 이 파일이 없앴다.
//   · ★재실측(2026-08-17 2차 · 적대검증 2R 수리 후): 같은 치워 보기가 **25 에러 · exit 1** 이다.
//     늘어난 2건은 신규 `src/typegate.test.ts`(이 파일의 실존을 지키는 핀)가 쓰는 bun:test·node:fs 다.
//     ∴ **절대 건수는 계약이 아니다** — 테스트 파일이 늘면 함께 는다. 계약인 것은 관계다:
//     '치우면 전 테스트 파일의 bun:test·node:fs·Buffer 타입이 통째로 사라지고, 되돌리면 0 이 된다'.
//   · 런타임 부작용 0 확인: 저장소 어디에서도
//     "bun-env" 를 import 하지 않는다(grep 0건) · 비테스트 소스의 `Buffer` 사용 0건이라
//     전역 `declare const Buffer` 가 본체 코드의 타입을 느슨하게 만들 여지도 없다.
//   · 마스킹 없음: 종전엔 `bun:test` 가 **미해소 모듈**이라 import 심볼이 전부 any 였다 —
//     선언을 붙인 지금이 검사량이 더 많다(줄어들 수 없다).
//   · ★`bun test` 와의 관계 정정(2026-08-17 · 이 라운드): 종전 이 자리에는 "부작용 0 확인:
//     bun test 403 pass / 0 fail(**도입 전후 동일**)"이라고 적혀 있었는데, 그 문장은 지금
//     **두 겹으로 거짓**이다. ①건수가 맞지 않는다(현 실측 411 pass / 0 fail · 1599 expect).
//     ②`src/typegate.test.ts` 가 생긴 뒤로는 '전후 동일'이 아니다 — 이 파일을 치우면
//     `bun test` 가 **409 pass / 2 fail** 로 떨어진다(같은 트리 실측). 그게 결함이 아니라
//     **그 핀의 설계 목적**이다: 이 파일의 누락을 유일한 CI 레인(`bun test`)에서 즉시 울리게
//     하는 것. 그러니 이 줄을 '부작용이 생겼다'로 읽지 마라 — 런타임 부작용은 위 줄대로
//     여전히 0 이고(어디서도 import 되지 않는다), 여기서 바뀌는 것은 **감시선의 판정**뿐이다.
//
// ★★삭제·커밋 누락 금지: 이 파일이 사라지면 `cd ui && bunx tsc -p tsconfig.check.json` 이 **즉시
//   red** 로 되돌아간다(위 실측 — 2026-08-17 2차 기준 25 에러 · exit 1). 런타임 코드 0줄이라
//   "안 쓰는 파일" 로 보이는 것이 함정이다 — `.d.ts` 는 import 되지 않아도 include 범위
//   (`src/**/*.ts`)에 있는 것만으로 타입을 공급한다.
//   같은 경고가 `ui/tsconfig.check.json` 의 `_why` 에도 있다(둘은 한 쌍이다).
// ★그 경고를 **산문에만 두지 않는다**: `src/typegate.test.ts` 가 이 파일의 실존·선언 3종·include
//   범위를 `bun test` 에서 못박는다(타입 게이트 자체는 **어떤 워크플로도 돌리지 않는 수동
//   게이트**라, CI 에서 ui/ 의 정확성을 보는 레인은 `bun test` 하나뿐이다). 이 파일을 지우려면
//   그 핀도 함께 지워야 하고, 그 순간 '왜 지우면 안 되는가'가 코드 리뷰 표면으로 올라온다.

declare module "bun:test" {
  /** 실제 사용 중인 matcher 만 — 미선언 matcher 를 쓰면 컴파일 에러로 드러난다(의도). */
  interface Matchers {
    toBe(expected: unknown): void;
    toEqual(expected: unknown): void;
    toContain(expected: unknown): void;
    toHaveLength(expected: number): void;
    toBeNull(): void;
    toBeUndefined(): void;
    toBeGreaterThan(expected: number): void;
    toBeLessThan(expected: number): void;
    toBeCloseTo(expected: number, numDigits?: number): void;
    toThrow(expected?: unknown): void;
    /** 부정 체이닝 — `expect(x).not.toBe(y)`. */
    readonly not: Matchers;
  }

  type TestBody = () => void | Promise<void>;
  interface TestFn {
    (label: string, body: TestBody): void;
    skip(label: string, body: TestBody): void;
    only(label: string, body: TestBody): void;
    todo(label: string, body?: TestBody): void;
  }

  export function expect(actual: unknown): Matchers;
  export const describe: TestFn;
  export const it: TestFn;
  export const test: TestFn;
}

declare module "node:fs" {
  /** mousefilter.test.ts 의 코퍼스 로딩(URL 경로 — cwd 비의존)만 쓴다. */
  export function existsSync(path: string | URL): boolean;
  export function readFileSync(path: string | URL, encoding: "utf8" | "utf-8"): string;
}

/**
 * trackfilter.test.ts 가 바이트 정체성 비교를 위해 latin1 왕복에만 쓴다
 * (`Buffer.from(u8).toString("binary")`). 그 두 연산 외에는 선언하지 않는다.
 */
declare const Buffer: {
  from(data: Uint8Array | ArrayBuffer | readonly number[]): { toString(encoding: "binary"): string };
};
