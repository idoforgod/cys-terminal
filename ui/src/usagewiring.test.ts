// U1 사이드바 사용량 패널 **배선** 회귀 핀 — main.ts·index.html·style.css·새 모듈 소스를 데이터로 읽어
// 계약을 단언한다(wswiring.test.ts 관례: 런타임 코드 0줄 · DOM/Tauri 불요 · 주석 제거 본문 기준).
//
// 왜 배선을 기계로 세는가(설계 §3 U1 · 반박 D6·D7):
//   · 사용량 패널은 **표시 전용**이다. 새 타이머·새 폴링 루프·3초 입양 틱 개입·await 대기가 하나라도
//     생기면 자가치유 틱(③)과 승인 자동전환 판정(ceoIsActivelyGenerating 이 refreshSidebarStatus 를 await)
//     에 부하·지연이 번진다. 개수 핀(setInterval 9개)은 병렬 항목과 결합하므로 **의미 핀**으로 막는다.
//   · 새 DOM id 에 `!` 단언을 쓰면 index.html 과 번들이 어긋날 때 main.js 평가가 중단돼 전 pane 이
//     백지가 된다(④). 새 모듈의 구형 WKWebView 비호환 문법·최상위 부수효과도 같은 치명도다(반박 D6).
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf-8");
const stripComments = (s: string): string =>
  s
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");
const src = read("./main.ts");
const code = stripComments(src);
const html = read("../index.html");
const css = read("./style.css");

/** 열 0 의 `function name(` 부터 첫 열 0 닫는 중괄호까지(clipath.test.ts mainFnBody 와 같은 규칙). */
function fnBody(name: string): string {
  const i = code.indexOf(`function ${name}(`);
  expect({ 함수: name, 존재: i >= 0 }).toEqual({ 함수: name, 존재: true });
  const end = code.indexOf("\n}\n", i);
  return code.slice(i, end > i ? end + 3 : undefined);
}
/** 호출 지점마다 그 호출을 감싼 최상위 함수 이름(없으면 "<top>"). */
function enclosingFns(needle: string): string[] {
  const out: string[] = [];
  let at = code.indexOf(needle);
  while (at >= 0) {
    const head = code.lastIndexOf("\nfunction ", at);
    const headA = code.lastIndexOf("\nasync function ", at);
    const h = Math.max(head, headA);
    const closed = h >= 0 ? code.indexOf("\n}\n", h) : -1;
    if (h < 0 || (closed >= 0 && closed < at)) out.push("<top>");
    else {
      const m = /function\s+([A-Za-z0-9_$]+)\s*\(/.exec(code.slice(h, h + 200));
      out.push(m ? m[1] : "<?>");
    }
    at = code.indexOf(needle, at + needle.length);
  }
  return out;
}
const tagsOnly = (s: string) => s.replace(/>\s+</g, "><");

describe("사이드바 바닥 공용 꼬리 컨테이너(#wsbar-foot) — A2·A3 공통 마크업", () => {
  const h = tagsOnly(html);
  it("#ws-tabs 바로 다음 형제로 사용량 → 피드백 슬롯 → 전문가용 순서", () => {
    expect(
      h.includes(
        '<div id="ws-tabs"></div><div id="wsbar-foot"><section id="wsbar-usage" aria-label="사용량"></section><div id="wsbar-feedback-slot">',
      ),
    ).toBe(true);
    expect(h.includes('<section id="wsbar-expert"></section></div></nav>')).toBe(true);
    const u = h.indexOf('id="wsbar-usage"'), f = h.indexOf('id="wsbar-feedback-slot"'), e = h.indexOf('id="wsbar-expert"');
    expect(u >= 0 && u < f && f < e).toBe(true);
  });
  it("꼬리는 #wsbar 안에 있다", () => {
    const nav = h.indexOf('<nav id="wsbar">'), foot = h.indexOf('id="wsbar-foot"'), end = h.indexOf("</nav>", nav);
    expect(nav >= 0 && nav < foot && foot < end).toBe(true);
  });
  it("목록(#ws-tabs)에 최소 높이 · 꼬리 묶음에 상한 + 자체 스크롤 (목록이 0px 로 눌리지 않게 — 반박 U17 D5)", () => {
    const flat = css.replace(/\s+/g, " ");
    expect(/#ws-tabs \{[^}]*min-height:/.test(flat)).toBe(true);
    const foot = /#wsbar-foot \{([^}]*)\}/.exec(flat);
    expect(foot).not.toBeNull();
    expect(foot![1]).toContain("max-height:");
    expect(foot![1]).toContain("overflow-y: auto");
    expect(foot![1]).toContain("min-height: 0");
  });
  it("피드백 단추(A3)는 꼬리 여백 안에서 가로 여백을 걷는다 — 사용량·전문가용과 같은 선(리뷰1 M11 · 이중 들여쓰기 차단)", () => {
    const flat = css.replace(/\s+/g, " ");
    const r = /#wsbar-feedback-slot > #btn-feedback \{([^}]*)\}/.exec(flat);
    expect(r).not.toBeNull();
    for (const decl of ["margin-left: 0", "margin-right: 0", "width: 100%"]) expect(r![1]).toContain(decl);
  });
});

describe("사용량 조회 — 공유 fetcher 하나 · in-flight 가드 · 전용 상한", () => {
  it("usage_accounts_all 호출은 fetcher 본문 1곳뿐", () => {
    const n = code.split('invoke("usage_accounts_all"').length - 1;
    expect(n).toBe(1);
    expect(fnBody("refreshAccountsShared")).toContain('invoke("usage_accounts_all"');
  });
  it("fetcher 는 claimFlight + releaseFlightWhenSettled + rpcT(T_ACCT) 규약을 쓴다", () => {
    const b = fnBody("refreshAccountsShared");
    for (const needle of ["claimFlight(", "releaseFlightWhenSettled(", "rpcT(", "T_ACCT", "shouldFetchAccounts("])
      expect({ 배선: needle, 있음: b.includes(needle) }).toEqual({ 배선: needle, 있음: true });
    // ★순서(리뷰1 V1): 이름만 세면 `if (!go) return;` 을 지워도(부팅 유예·30초 간격·이벤트 폭주 방어가 통째로
    //   사라짐) 초록이었다. 판정 → 판정 결과로 반환 → in-flight 획득 → 시도 시각 기록 → 호출 순서를 못박는다.
    const at = (s: string) => b.indexOf(s);
    const seq = [
      "if (started && acctStartedAtMs === null) acctStartedAtMs = now;", // 유예 앵커 — 없으면 사이드바 조회 영구 0회
      "const go = shouldFetchAccounts(now,",
      "if (!go) return;",
      "if (!claimFlight(ACCT_FLIGHT_KEY)) return;",
      "acctLastAttemptAtMs = now;",
      'invoke("usage_accounts_all")',
    ];
    const pos = seq.map(at);
    expect({ 위치: seq.map((s, i) => [s, pos[i] >= 0]) }).toEqual({ 위치: seq.map((s) => [s, true]) });
    expect({ 순서대로: pos.every((p, i) => i === 0 || pos[i - 1] < p) }).toEqual({ 순서대로: true });
  });
  it("스로틀 판정에 실제 상태를 넘긴다(상수·null 로 바꿔치면 조회가 영구 0회 또는 무제한 — 리뷰1 V1)", () => {
    const b = fnBody("refreshAccountsShared");
    const gate = b.slice(b.indexOf("shouldFetchAccounts("), b.indexOf("});", b.indexOf("shouldFetchAccounts(")));
    for (const needle of ["started,", "startedAtMs: acctStartedAtMs,", "lastAttemptAtMs: acctLastAttemptAtMs,", "force,", "graceMs: ACCT_BOOT_GRACE_MS,", "minIntervalMs: ACCT_SIDEBAR_MIN_MS,"])
      expect({ 인자: needle, 있음: gate.includes(needle) }).toEqual({ 인자: needle, 있음: true });
  });
  it("성공하면 연속 실패를 0 으로, 실패하면 +1 — '데몬 응답 없음'이 한 번 뜨면 영영 남는 되돌림 차단", () => {
    const b = fnBody("refreshAccountsShared");
    const iRpc = b.indexOf("await rpcT(call, T_ACCT)");
    const iOk = b.indexOf("acctFailStreak = 0;");
    const iCatch = b.indexOf("catch", iRpc);
    const iInc = b.indexOf("acctFailStreak++", iCatch);
    expect({ 성공_리셋: iOk > iRpc && iOk < iCatch, 실패_증가: iInc > iCatch }).toEqual({ 성공_리셋: true, 실패_증가: true });
  });
  it("T_ACCT 는 winScaled 명명 상수(넘기면 무엇이 일어나는지 주석)", () => {
    expect(/const T_ACCT = winScaled\(\d[\d_]*\);/.test(code)).toBe(true);
    const line = src.split("\n").find((l) => l.includes("const T_ACCT = winScaled("))!;
    expect(line).toContain("넘기면");
  });
  it("호출 지점은 사이드바 10초 틱과 Control Center 두 곳뿐(반박 D7 의미 핀)", () => {
    const where = enclosingFns("refreshAccountsShared(").filter((f) => f !== "refreshAccountsShared");
    expect([...new Set(where)].sort()).toEqual(["refreshControlCenter", "refreshSidebarStatus"]);
  });
  it("사이드바 틱에서는 void(비대기)로 renderWsTabs() 뒤에 — await 금지(승인 자동전환 판정 지연 차단)", () => {
    const b = fnBody("refreshSidebarStatus");
    expect(b.includes("await refreshAccountsShared")).toBe(false);
    const v = b.indexOf("void refreshAccountsShared(false)");
    const r = b.indexOf("renderWsTabs();");
    expect(v > r && r >= 0).toBe(true);
  });
  it("Control Center Live 는 공유 fetcher 를 force 로 부른다(직접 invoke 제거 → 겹침 가드 획득)", () => {
    expect(fnBody("refreshControlCenter")).toContain("await refreshAccountsShared(true)");
  });
  it("3초 입양 틱(refreshPaneTitles)에는 아무것도 얹지 않는다", () => {
    const b = fnBody("refreshPaneTitles");
    for (const needle of ["refreshAccountsShared", "usage_accounts_all", "renderUsageBar"])
      expect({ 금지: needle, 있음: b.includes(needle) }).toEqual({ 금지: needle, 있음: false });
  });
  it("어떤 setInterval/setTimeout 도 사용량 조회·렌더를 직접 돌리지 않는다(새 타이머 0)", () => {
    const lines = code.split("\n");
    lines.forEach((l, i) => {
      if (!/set(Interval|Timeout)\(/.test(l)) return;
      const win = l + (lines[i + 1] ?? "");
      for (const needle of ["refreshAccountsShared", "usage_accounts_all", "renderUsageBar"])
        expect({ 줄: i + 1, 금지: needle, 있음: win.includes(needle) }).toEqual({ 줄: i + 1, 금지: needle, 있음: false });
    });
  });
  it("start() 복원 구간(머리 ~ started = true)에는 조회가 없다(부트 체인 비개입)", () => {
    const a = src.indexOf("async function start() {");
    const b = src.indexOf("started = true;", a);
    expect(a > 0 && b > a).toBe(true);
    expect(stripComments(src.slice(a, b)).includes("refreshAccountsShared")).toBe(false);
  });
  it("노드 신호(nodeSig) 폴백을 새로 만들지 않는다(설계 금지 — 제공자 혼합 최대값)", () => {
    expect(fnBody("refreshAccountsShared").includes("nodeSig")).toBe(false);
    expect(fnBody("renderUsageBar").includes("nodeSig")).toBe(false);
  });
});

describe("사용량 렌더 — 백지(④) 차단", () => {
  it("새 id 에 non-null 단언(`!`)을 쓰지 않는다", () => {
    for (const id of ["wsbar-usage", "wsbar-foot", "wsbar-expert", "wsbar-feedback-slot"])
      expect({ id, 단언: code.includes(`getElementById("${id}")!`) }).toEqual({ id, 단언: false });
  });
  it("렌더는 textContent 로만(innerHTML 금지 — 라벨은 로컬 폴더·파일에서 온다)", () => {
    expect(fnBody("renderUsageBar").includes("innerHTML")).toBe(false);
    expect(fnBody("renderUsageBar")).toContain("buildUsageBarModel(");
  });
  it("렌더는 스스로 오류를 삼킨다(표시 전용 — 호출측 틱으로 새지 않는다)", () => {
    expect(fnBody("renderUsageBar")).toContain("catch");
  });
  it("본문 모델이 직전과 같으면 본문을 다시 만들지 않는다(이벤트 구동 재렌더에 호버 툴팁이 사라지지 않게 — 리뷰1 M5)", () => {
    const b = fnBody("renderUsageBar");
    const iReset = b.indexOf('usageBodySig = "";'); // 머리·본문을 새로 만들면 반드시 다시 그린다
    const iSig = b.indexOf("const sig = JSON.stringify(model);");
    const iSkip = b.indexOf("if (sig === usageBodySig) return;");
    const iRebuild = b.indexOf("body.replaceChildren(");
    const iStore = b.indexOf("usageBodySig = sig;");
    expect({ 새칸_초기화: iReset >= 0 && iReset < iSig, 비교가_재구성보다_먼저: iSig >= 0 && iSig < iSkip && iSkip < iRebuild, 재구성_뒤_기록: iStore > iRebuild }).toEqual({
      새칸_초기화: true, 비교가_재구성보다_먼저: true, 재구성_뒤_기록: true,
    });
    // 선언은 초기 렌더(배선부 최상위 호출)보다 앞 — TDZ 면 renderUsageBar 가 catch 로 삼켜 패널이 빈 채로 남는다.
    const decl = code.indexOf('let usageBodySig = "";');
    const firstTop = code.indexOf("\nrenderUsageBar();");
    expect({ 선언: decl >= 0, 선언이_먼저: decl >= 0 && decl < firstTop }).toEqual({ 선언: true, 선언이_먼저: true });
  });
  it("최상위 접힘 판독(localStorage)은 try 안 — 저장소 차단이 main.js 평가를 끊지 않게(④)", () => {
    expect(/\ntry \{\n\s*usageCollapsed = localStorage\.getItem\(USAGE_COLLAPSED_KEY\) === "1";\n\} catch/.test(code)).toBe(true);
  });
  it("🔒 가림 상태를 모델에 넘긴다(툴팁의 설정 폴더 경로도 가린다 — 리뷰1 M9)", () => {
    const b = fnBody("renderUsageBar");
    const call = b.slice(b.indexOf("buildUsageBarModel("), b.indexOf(");", b.indexOf("ccAcctLabel,")) + 2);
    expect(call).toContain("ccAcctLabel,");
    expect(call).toContain("ccAcctRedact");
  });
  it("초기 렌더는 start() 와 무관한 배선부에서 1회(복원 중·start 실패에도 빈 섹션이 남지 않게 — 반박 D4·D5)", () => {
    const at = src.indexOf("// ---------- ui wiring ----------");
    expect(at).toBeGreaterThan(0);
    const wiring = stripComments(src.slice(at));
    expect(/\nrenderUsageBar\(\);/.test(wiring)).toBe(true);
  });
  it("🔒 계정 가림 토글이 사이드바도 다시 그린다(같은 키 공유)", () => {
    const i = code.indexOf('acctRedactBtn.addEventListener("click"');
    expect(i).toBeGreaterThan(0);
    expect(code.slice(i, code.indexOf("});", i))).toContain("renderUsageBar()");
  });
});

describe("새 순수 모듈 — 구형 WKWebView 파싱 실패·최상위 부수효과 0(반박 D6)", () => {
  // 블록 주석(/** … */)까지 걷는다 — 설명문이 금지 낱말을 언급해도 핀이 깨지지 않게(코드만 본다).
  const stripAll = (s: string) => stripComments(s.replace(/\/\*[\s\S]*?\*\//g, ""));
  for (const mod of ["./usagebar.ts", "./deptcreate.ts", "./feedclass.ts"]) {
    const m = stripAll(read(mod));
    it(`${mod}: 비호환 문법 0`, () => {
      for (const bad of ["(?<=", "(?<!", ".at(", "findLast", "structuredClone", "Object.hasOwn", "replaceAll("])
        expect({ 모듈: mod, 문법: bad, 있음: m.includes(bad) }).toEqual({ 모듈: mod, 문법: bad, 있음: false });
    });
    it(`${mod}: 전역 부수효과 표면(localStorage·document·window·navigator·타이머) 0`, () => {
      for (const bad of ["localStorage", "document.", "window.", "navigator", "setInterval", "setTimeout", "__TAURI__"])
        expect({ 모듈: mod, 표면: bad, 있음: m.includes(bad) }).toEqual({ 모듈: mod, 표면: bad, 있음: false });
    });
    it(`${mod}: 최상위 문장은 선언뿐`, () => {
      const bad = m
        .split("\n")
        .filter((l) => l.length > 0 && !/^\s/.test(l))
        .filter((l) => !/^(import |export |const |function |interface |type |\}|\)|\]|;)/.test(l));
      expect({ 모듈: mod, 최상위_비선언: bad }).toEqual({ 모듈: mod, 최상위_비선언: [] });
    });
  }
});

// ═════════ 0.14.43 (티켓 UI1) — 별명·'● 사용 중'·숨기기·접힘 줄·KPI 후보 배선 핀 ═════════
// 순수 판정(별명 라벨·주 계정 선정·오래됨·제공자 요약·숨김 제외·KPI 후보)은 usagebar.test.ts 가 표로 잡는다 — 여기는 main.ts·style.css 가
// 그 모델을 **어떻게 옮기는가**(textContent 만 · 저장소는 main.ts 만·try/catch 안 · CC 의 ccEsc · 위임 리스너)를 기계로 못박는다.
describe("0.14.43 UI1 — 사이드바 본문 배선(모델의 가산 필드를 화면으로)", () => {
  const body = () => fnBody("renderUsageBar");
  it("모델의 가산 필드를 전부 쓴다(쓰지 않으면 데몬·모델이 만든 표식이 화면에서 조용히 사라진다)", () => {
    const b = body();
    for (const needle of ["p.inUse", "o.inUse", "headlineSev", "headlineTitle", "unobservedFold", "moreTooltip", "hiddenCount"])
      expect({ 필드: needle, 사용: b.includes(needle) }).toEqual({ 필드: needle, 사용: true });
  });
  it("접힘 줄 '관측 없음 ${' 을 재도입하지 않는다 · 줄은 o.unobserved 를 반영한다(관측 전 계정 한 줄씩)", () => {
    const b = body();
    expect(code.includes("관측 없음 ${")).toBe(false);
    expect(b.includes("관측 없음")).toBe(false);
    expect(/"usage-other" \+ \(o\.dim[^;]*o\.unobserved/.test(b)).toBe(true);
  });
  it("'● 사용 중' 배지(.usage-inuse)·점(.usage-inuse-dot)은 textContent 로 · 같은 툴팁 · innerHTML 금지", () => {
    const b = body();
    expect(b.includes("innerHTML")).toBe(false);
    expect(b.includes('inUseMark("usage-inuse", "● 사용 중")')).toBe(true);
    expect(b.includes('inUseMark("usage-inuse-dot", "●")')).toBe(true);
    expect(code.includes('const USAGE_INUSE_TIP = "지금 로그인돼 쓰이고 있는 계정";')).toBe(true);
    const mark = b.slice(b.indexOf("const inUseMark ="), b.indexOf("const boxEl = "));
    for (const needle of ["s.className = cls;", "s.textContent = text;", "s.title = USAGE_INUSE_TIP;"])
      expect({ 구현: needle, 있음: mark.includes(needle) }).toEqual({ 구현: needle, 있음: true });
  });
  it("배지는 주 계정(p.inUse)에서만 · 점은 다른 줄(o.inUse)에서만 — 서로 바뀌지 않는다", () => {
    const b = body();
    expect(/if \(p\.inUse\) acctRow\.appendChild\(inUseMark\("usage-inuse", /.test(b)).toBe(true);
    expect(/if \(o\.inUse\) lab\.appendChild\(inUseMark\("usage-inuse-dot", /.test(b)).toBe(true);
  });
  it("잘려 나간 관측 전 접힘 줄(.usage-other.unobs.dim · title = tooltip)이 '외 N개' 줄 앞에 · 그 줄에는 title = moreTooltip", () => {
    const b = body();
    const fold = b.indexOf("if (model.unobservedFold) {");
    // 0.14.44(D1): 묶음 안의 '외 N개' 줄(g.moreCount)이 앞에 하나 더 있으므로 — 모델 전체의 moreCount 줄을 짚는다.
    const more = b.indexOf('el("usage-more", `외 ${model.moreCount}개');
    expect({ 접힘줄: fold >= 0, 외N개: more >= 0, 접힘줄이_앞: fold >= 0 && fold < more }).toEqual({ 접힘줄: true, 외N개: true, 접힘줄이_앞: true });
    expect(b.slice(fold, more).includes('el("usage-other unobs dim", "", model.unobservedFold.tooltip)')).toBe(true);
    expect(b.slice(more, b.indexOf(");", more)).includes("model.moreTooltip")).toBe(true);
    // 접힘 줄만 있어도(관측 줄이 상한 안) 줄 묶음 상자가 만들어진다
    expect(b.includes("if (model.others.length || model.moreCount || model.unobservedFold) {")).toBe(true);
  });
  it("숨김 안내(.usage-hidden)는 hiddenCount > 0 일 때만 · 본문 맨 끝(꼬리 경고 뒤)", () => {
    const b = body();
    const foot = b.indexOf('el("usage-foot"');
    const hid = b.indexOf('el("usage-hidden"');
    const rep = b.indexOf("body.replaceChildren(");
    expect({ 순서: foot >= 0 && foot < hid && hid < rep }).toEqual({ 순서: true });
    expect(b.slice(b.lastIndexOf("if (", hid), hid).includes("model.hiddenCount > 0")).toBe(true);
    expect(b.slice(hid, b.indexOf(");", hid)).includes("Control Center > Live 에서 다시 보이기")).toBe(true);
  });
  it("요약 줄: headlineSev 로 warn/crit 토글(응답 없음 경고와 같은 칸 · crit 가 이김) · headlineTitle 로 title 설정/제거", () => {
    const b = body();
    expect(b.includes('sumEl.classList.toggle("warn", !!model.footer || model.headlineSev === "warn");')).toBe(true);
    expect(b.includes('sumEl.classList.toggle("crit", model.headlineSev === "crit");')).toBe(true);
    expect(b.includes("sumEl.title = model.headlineTitle")).toBe(true);
    expect(b.includes('sumEl.removeAttribute("title")')).toBe(true);
  });
  it("모델 호출의 마지막 인자가 뷰어별 숨김 목록(usageHidden) — 🔒 가림 인자 뒤", () => {
    const b = body();
    const call = b.slice(b.indexOf("buildUsageBarModel("), b.indexOf(");", b.indexOf("ccAcctLabel,")) + 2);
    expect(call.indexOf("ccAcctLabel,") < call.indexOf("ccAcctRedact,") && call.indexOf("ccAcctRedact,") < call.indexOf("usageHidden,")).toBe(true);
  });
  it("🔒 가림 함수 ccHash6 는 usagebar.test.ts 의 겹침 꼬리표 대조용 사본과 같은 알고리즘(두 화면의 #hash6 가 같다)", () => {
    const b = fnBody("ccHash6");
    for (const needle of ["let h = 5381;", "h = ((h << 5) + h + s.charCodeAt(i)) >>> 0;", 'h.toString(16).padStart(8, "0").slice(0, 6)'])
      expect({ 알고리즘: needle, 있음: b.includes(needle) }).toEqual({ 알고리즘: needle, 있음: true });
  });
});

describe("0.14.43 UI1 — 숨김 저장소(main.ts 만 · try/catch 안 · 값 검증은 순수 함수)", () => {
  it("키는 cys-usage-hidden · JSON 문자열 배열로 저장", () => {
    expect(code.includes('const USAGE_HIDDEN_KEY = "cys-usage-hidden";')).toBe(true);
    expect(code.includes("JSON.stringify([...usageHidden])")).toBe(true);
  });
  it("저장소 접근은 정확히 읽기 1곳·쓰기 1곳이고 둘 다 try 안 — 차단·깨진 JSON 이어도 패널은 숨김 없음으로 정상", () => {
    expect(code.split("localStorage.getItem(USAGE_HIDDEN_KEY").length - 1).toBe(1);
    expect(code.split("localStorage.setItem(USAGE_HIDDEN_KEY").length - 1).toBe(1);
    expect(/\ntry \{\n\s*usageHidden = sanitizeHiddenKeys\(JSON\.parse\(localStorage\.getItem\(USAGE_HIDDEN_KEY\) \|\| "\[\]"\)\);\n\} catch/.test(code)).toBe(true);
    expect(/\n\s*try \{\n\s*localStorage\.setItem\(USAGE_HIDDEN_KEY, JSON\.stringify\(\[\.\.\.usageHidden\]\)\);\n\s*\} catch/.test(code)).toBe(true);
  });
  it("값 검증(배열·문자열·최대 개수)은 순수 함수 sanitizeHiddenKeys — 읽은 값을 그대로 쓰지 않는다", () => {
    expect(code.includes("sanitizeHiddenKeys(JSON.parse(")).toBe(true);
    expect(fnBody("toggleUsageAcctHidden")).toContain("USAGE_HIDDEN_MAX");
  });
  it("usageHidden 선언은 첫 최상위 renderUsageBar() 호출보다 앞(TDZ 면 renderUsageBar 가 catch 로 삼켜 패널이 빈 채로 남는다)", () => {
    const decl = code.indexOf("let usageHidden:");
    const firstTop = code.indexOf("\nrenderUsageBar();");
    expect({ 선언: decl >= 0, 선언이_먼저: decl >= 0 && decl < firstTop }).toEqual({ 선언: true, 선언이_먼저: true });
  });
  it("usagebar.ts 에는 저장소 접근 표면이 0 — 숨김 목록은 main.ts 가 읽어 인자로 넘긴다", () => {
    expect(read("./usagebar.ts").includes("localStorage")).toBe(false); // 주석 포함 원문 기준(티켓 문면: `localStorage` 문자열 0)
    const m = stripComments(read("./usagebar.ts").replace(/\/\*[\s\S]*?\*\//g, ""));
    for (const bad of ["localStorage", "sessionStorage", "indexedDB", "document.", "window."])
      expect({ 표면: bad, 있음: m.includes(bad) }).toEqual({ 표면: bad, 있음: false });
  });
});

describe("0.14.43 UI1 — Control Center Live 계정 섹션·KPI 배선", () => {
  it("KPI 후보는 순수 함수 kpiCandidates 로 거른다(숨김 목록 포함) — 계정을 직접 순회해 최댓값을 고르지 않는다", () => {
    const b = fnBody("ccAcctMax");
    expect(b.includes("for (const a of kpiCandidates(ccAccounts, label, Date.now() / 1000, usageHidden)) {")).toBe(true);
    expect(/of ccAccounts\b/.test(b)).toBe(false);
  });
  it("후보가 없으면 null — renderLiveBody 는 null 을 종전 '없음' 경로(ccAggRate 폴백 → 0%)로 그린다(경로 불변 핀)", () => {
    const b = fnBody("renderLiveBody");
    expect(b.includes("const used = m ? Math.round(m.used) : w ? Math.round(w.used) : 0;")).toBe(true);
    expect(b.includes("const sub = m ? ccAcctLabel(m.acct) : w ? ccReset(lab, w.reset) : \"\";")).toBe(true);
  });
  it("행 라벨 — 별명이 있으면 `별명 (이메일/해시)` · 둘 다 ccEsc · 이메일은 기존 ccAcctLabel 가림을 거친다", () => {
    const b = fnBody("renderAccounts");
    expect(b.includes("const who = ccEsc(ccAcctLabel(String(a.label ?? a.account_id ?? \"?\")));")).toBe(true);
    expect(b.includes("const alias = acctAlias(a);")).toBe(true);
    expect(b.includes("alias ? `${ccEsc(alias)} (${who})` : who")).toBe(true);
  });
  it("배지 문구 — '● 사용 중' · '이전 로그인' · '오래됨' · '관측 전'(종전 '관측 없음' 폐기 · 사이드바·설명서와 통일)", () => {
    const b = fnBody("renderAccounts");
    for (const needle of ["● 사용 중", "이전 로그인", ">오래됨<", ">관측 전<", "a.in_use === true", "isPreviousLogin(a)", "isOldObservation(a, nowSec)"])
      expect({ 문구: needle, 있음: b.includes(needle) }).toEqual({ 문구: needle, 있음: true });
    expect(b.includes(">관측 없음<")).toBe(false);
    expect(code.includes('<span class="cc-acct-badge">관측 없음</span>')).toBe(false);
    // 종전 'N분 전 관측' 배지는 유지
    expect(b.includes("분 전 관측")).toBe(true);
  });
  it("게이지는 windowView 규칙(리셋 지남 → 폭 0·'리셋됨'·경고색 없음) — 종전 sevClass(used, 70, 90) 직접 판정을 쓰지 않는다", () => {
    const b = fnBody("renderAccounts");
    expect(b.includes("windowView(a, lab, nowSec)")).toBe(true);
    expect(b.includes('v.state === "ok" ?')).toBe(true);
    expect(b.includes("sevClass(")).toBe(false);
    expect(b.includes("ccEsc(v.text)") && b.includes("ccEsc(v.resetText)")).toBe(true);
  });
  it("행 흐림 — 오래된 관측(30분 초과·스냅샷)·숨긴 계정 → .dim", () => {
    const b = fnBody("renderAccounts");
    expect(b.includes('class="cc-acct-row${old || hiddenNow ? " dim" : ""}"')).toBe(true);
  });
  it("숨기기/보이기 단추(.cc-acct-hide · data-acct-key) — 키는 ccEsc · 문구는 숨김 상태에 따라", () => {
    const b = fnBody("renderAccounts");
    expect(b.includes('class="cc-acct-hide" data-acct-key="${ccEsc(key)}"')).toBe(true);
    expect(b.includes('hiddenNow ? "보이기" : "숨기기"')).toBe(true);
    expect(b.includes("const key = acctKey(a);")).toBe(true);
  });
  it("클릭은 호스트 하나에 위임(행은 5초마다 innerHTML 로 다시 그려진다) — 저장소 갱신 뒤 사이드바·CC 를 다시 그린다", () => {
    expect(fnBody("renderAccounts").includes("addEventListener")).toBe(false); // 행마다 리스너를 달지 않는다
    const i = code.indexOf('ccAcctHost.addEventListener("click"');
    expect(i).toBeGreaterThan(0);
    const h = code.slice(i, code.indexOf("\n  });", i));
    for (const needle of ['.closest(".cc-acct-hide")', 'getAttribute("data-acct-key")', "toggleUsageAcctHidden(key);", "renderAccounts();", "renderUsageBar();", "void refreshControlCenter();"])
      expect({ 핸들러: needle, 있음: h.includes(needle) }).toEqual({ 핸들러: needle, 있음: true });
    expect(h.includes("refreshAccountsShared")).toBe(false); // 조회 호출 지점 핀(사이드바 틱·CC 두 곳뿐)을 건드리지 않는다
  });
  it("renderAccounts 의 데이터 보간은 ccEsc — 별명·이메일·키·source_error·plan 은 로컬 파일·IPC 에서 온다", () => {
    const b = fnBody("renderAccounts");
    for (const needle of ["ccEsc(alias)", "ccEsc(ccAcctLabel(", "ccEsc(key)", "ccEsc(a.source_error)", "ccEsc(String(a.plan))", "ccEsc(String(a.provider ?? \"?\"))"])
      expect({ 이스케이프: needle, 있음: b.includes(needle) }).toEqual({ 이스케이프: needle, 있음: true });
  });
});

describe("0.14.43 UI1 — style.css(기존 강조색 재사용 · 사이드바 줄 말줄임 보존)", () => {
  const flat = css.replace(/\s+/g, " ");
  const rule = (sel: string): string => {
    const i = flat.indexOf(`${sel} {`);
    expect({ 규칙: sel, 존재: i >= 0 }).toEqual({ 규칙: sel, 존재: true });
    return flat.slice(i, flat.indexOf("}", i) + 1);
  };
  it("새 규칙이 모두 있다(.usage-inuse · .usage-inuse-dot · .usage-hidden · .usage-sum.crit · .cc-acct-row.dim · .cc-acct-hide)", () => {
    for (const sel of ["#wsbar-usage .usage-inuse", "#wsbar-usage .usage-inuse-dot", "#wsbar-usage .usage-hidden", "#wsbar-usage .usage-sum.crit", ".cc-acct-row.dim", ".cc-acct-hide"]) rule(sel);
  });
  it("사용 중 배지·점의 색은 기존 --ok 재사용(새 색 정의 없음) · 크롬 표면이라 --canvas-text 금지", () => {
    expect(rule("#wsbar-usage .usage-inuse")).toContain("var(--ok)");
    expect(rule("#wsbar-usage .usage-inuse-dot")).toContain("var(--ok)");
    expect(/#[0-9a-fA-F]{3,6}\b/.test(rule("#wsbar-usage .usage-inuse") + rule("#wsbar-usage .usage-inuse-dot"))).toBe(false);
    for (const sel of ["#wsbar-usage .usage-inuse", "#wsbar-usage .usage-inuse-dot", "#wsbar-usage .usage-hidden"]) expect(rule(sel).includes("--canvas-text")).toBe(false);
  });
  it("사이드바 줄의 말줄임을 깨지 않는다 — 라벨 칸 max-width 45% · 이름은 줄어들고(말줄임) 배지는 flex:none", () => {
    const lab = rule("#wsbar-usage .usage-other-lab");
    for (const decl of ["flex: none", "max-width: 45%", "text-overflow: ellipsis", "overflow: hidden"]) expect({ 선언: decl, 있음: lab.includes(decl) }).toEqual({ 선언: decl, 있음: true });
    expect(rule("#wsbar-usage .usage-acct-name")).toContain("text-overflow: ellipsis");
    expect(rule("#wsbar-usage .usage-inuse")).toContain("flex: none");
  });
  it("요약 줄 crit 는 warn 규칙 뒤에(같은 특이도 — 응답 없음 warn 과 겹쳐도 crit 가 이긴다)", () => {
    expect(flat.indexOf("#wsbar-usage .usage-sum.crit {")).toBeGreaterThan(flat.indexOf("#wsbar-usage .usage-sum.warn {"));
  });
  it("흐린 행은 opacity 로(오래된 관측·숨긴 계정) · 단추는 flex:none", () => {
    expect(rule(".cc-acct-row.dim")).toContain("opacity:");
    expect(rule(".cc-acct-hide")).toContain("flex: none");
  });
});

// ═════════ 0.14.43 (티켓 UI2 · 추가 과제) — Control Center Live KPI 전 좌석 폴백 배선 ═════════
// 순수 집계(경보 부적격·리셋 지난 창 제외 · 오염값 무해 · 종전 동치)는 usagebar.test.ts 의 aggSeatRates 가 표로 잡는다 — 여기는 main.ts 의 ccAggRate 가
// 그것을 **실제로 부르는가**(옛 좌석 값을 되살리는 직접 순회를 되살리지 않았는가)만 기계로 못박는다.
describe("0.14.43 UI2 — Control Center KPI 전 좌석 폴백 배선(ccAggRate → aggSeatRates)", () => {
  it("ccAggRate 본문은 aggSeatRates(fleet, Date.now() / 1000) 를 돌려주는 한 줄 — 좌석 순회·usage.rate 직접 집계를 되살리지 않는다", () => {
    const b = fnBody("ccAggRate");
    expect(b.includes("return aggSeatRates(fleet, Date.now() / 1000);")).toBe(true);
    for (const old of ["for (", "usage?.rate", ".resets_at", ".used_pct"]) expect({ 금지: old, 있음: b.includes(old) }).toEqual({ 금지: old, 있음: false });
  });
  it("main.ts 가 aggSeatRates 를 ./usagebar 에서 가져온다", () => {
    const from = code.indexOf('} from "./usagebar"');
    const open = code.lastIndexOf("import {", from);
    expect(open >= 0 && from > open).toBe(true);
    expect(code.slice(open, from).includes("  aggSeatRates,\n")).toBe(true);
  });
  it("renderLiveBody 는 여전히 ccAggRate(fleet) 로 폴백 값을 얻고 계정 병합 값(ccAcctMax)이 우선이다 — 경로 불변", () => {
    const b = fnBody("renderLiveBody");
    expect(b.includes("const agg = ccAggRate(fleet);")).toBe(true);
    expect(b.includes("const m = ccAcctMax(lab);")).toBe(true);
    expect(b.includes("const used = m ? Math.round(m.used) : w ? Math.round(w.used) : 0;")).toBe(true);
  });
  it("ccAggRate 호출 지점은 renderLiveBody 하나뿐 — 다른 곳이 옛 좌석 값을 다시 쓰지 않는다", () => {
    expect(enclosingFns("ccAggRate(").filter((f) => f !== "ccAggRate")).toEqual(["renderLiveBody"]);
  });
});

// ═════════ 성찰 1회차 R1F-UB (S2 m-4 · m-1 ⓑ) — Control Center 계정 행의 문구·배지 배선 ═════════
describe("R1F-UB(S2 m-4) — 숨기기 단추 툴팁은 사실대로 말한다(동작은 그대로 · 문구만)", () => {
  const HIDE_TIP = "사이드바 사용량 패널과 위 KPI 의 계정 후보에서 이 계정을 뺍니다(계정 후보가 하나도 남지 않으면 KPI 는 좌석 값으로 표시됩니다)";
  const SHOW_TIP = "사이드바 사용량 패널과 위 KPI 에 이 계정을 다시 넣습니다";
  it("★숨기기 툴팁 전문 — 후보에서 뺀다는 것과 후보가 비면 좌석 값으로 간다는 것을 함께 적는다 · 옛 문구('위 KPI 에서 이 계정을 뺍니다')는 없다", () => {
    const b = fnBody("renderAccounts");
    expect(b.includes(`"${HIDE_TIP}"`)).toBe(true);
    expect(b.includes("위 KPI 에서 이 계정을 뺍니다")).toBe(false);
    expect(b.includes("이 표에는 흐리게 남습니다")).toBe(false);
  });
  it("보이기 툴팁은 종전 그대로 · 두 문구는 숨김 상태(hiddenNow)로 갈린다", () => {
    const b = fnBody("renderAccounts");
    expect(b.includes(`"${SHOW_TIP}"`)).toBe(true);
    const i = b.indexOf("const hideTip = hiddenNow");
    expect(i).toBeGreaterThan(0);
    const seg = b.slice(i, b.indexOf(";", b.indexOf(HIDE_TIP, i)) + 1);
    expect(seg.indexOf(SHOW_TIP)).toBeGreaterThan(0);
    expect(seg.indexOf(SHOW_TIP)).toBeLessThan(seg.indexOf(HIDE_TIP)); // hiddenNow ? 보이기 : 숨기기
    expect(b.includes('title="${hideTip}"')).toBe(true);
  });
});

describe("R1F-UB(S2 m-1 ⓑ) — Control Center '이전 로그인' 배지는 순수 판정(isPreviousLogin)만 따른다 · 요약 줄 색 주석은 사실대로", () => {
  it("배지는 isPreviousLogin(a) 로만 — 화면이 current_profiles 를 직접 보고 따로 판정하지 않는다(in_use 모순 방어가 한 곳에 있다)", () => {
    const b = fnBody("renderAccounts");
    expect(b.includes("if (isPreviousLogin(a)) badges.push(")).toBe(true);
    expect(b.includes("current_profiles")).toBe(false);
    expect(code.includes("current_profiles")).toBe(false);
  });
  it("요약 줄 색 주석은 약식 요약의 실제(요약에 실린 값들)를 말한다 — 주 계정 창들뿐이라고 적지 않는다", () => {
    expect(src.includes("제공자별 약식이면 요약에 실린 값들")).toBe(true);
    expect(src.includes("요약 줄 색 = 주 계정 창들의 최고 심각도(headlineSev")).toBe(false);
  });
});

// ═════════ 0.14.44 (WC · D1 · D2) — 상자 반복·제공자 묶음·전환 단추·저장 키 배선 ═════════
// 순수 판정(boxes · groups · 보기 방식 · 한 줄형 · 상자별 소진 예상)은 usagebar-modes.test.ts 가 표로 잡는다 — 여기는 main.ts·style.css 가 그 모델을 어떻게 옮기는가.
// 0.14.43 의 핀 중 `const p = model.primary;` 앵커(사용 중 표식 구현을 자르는 기준)는 boxes 반복에 맞게 `const boxEl = ` 앵커로 바뀌었다 — 같은 취지(사용 중 표지를 상자와 줄에 그린다)를 새 꼴로 다시 고정한다.
describe("0.14.44 D1·D2 — 상자는 boxes 반복으로(같은 DOM 꼴 · textContent 만)", () => {
  const body = () => fnBody("renderUsageBar");
  it("종전 `const p = model.primary;` 단일 상자 그리기는 없다 — boxes(또는 묶음의 대표) 를 반복한다", () => {
    const b = body();
    expect(b.includes("const p = model.primary;")).toBe(false);
    expect(b.includes("for (const p of model.boxes)")).toBe(true);
    expect(b.includes("for (const g of model.groups)")).toBe(true);
    expect(b.includes("model.boxes")).toBe(true);
  });
  it("상자 하나를 그리는 함수(boxEl)가 사용 중 배지 · 리셋 시각 · 소진 예상(상자마다)을 그린다 — 모두 textContent", () => {
    const b = body();
    const box = b.slice(b.indexOf("const boxEl = "), b.indexOf("const compactEl = "));
    for (const needle of ['inUseMark("usage-inuse", "● 사용 중")', "rs.textContent = w.resetText;", 'el("usage-exhaust", p.exhaust)', 'el("usage-note", p.fresh.note)', "fill.style.width"])
      expect({ 구현: needle, 있음: box.includes(needle) }).toEqual({ 구현: needle, 있음: true });
    expect(box.includes("innerHTML")).toBe(false);
  });
  it("한 줄형(compactEl)은 게이지 없이 이름·두 창 값 · 사용 중 점 — model.compact 일 때만 쓴다", () => {
    const b = body();
    const c = b.slice(b.indexOf("const compactEl = "), b.indexOf("const lineEl = "));
    expect(c.includes("usage-gauge")).toBe(false);
    expect(c.includes('inUseMark("usage-inuse-dot", "●")')).toBe(true);
    expect(b.includes("model.compact ? compactEl(")).toBe(true);
  });
  it("관측 전·관측 줄·묶음 줄은 한 함수(lineEl)로 — 줄마다 o.inUse 점 · o.unobserved 클래스", () => {
    const b = body();
    const l = b.slice(b.indexOf("const lineEl = "), b.indexOf("// ★0.14.44(D1) 상자들"));
    expect(l.includes("if (o.inUse) lab.appendChild(")).toBe(true);
    expect(l.includes('o.unobserved ? " unobs" : ""')).toBe(true);
    expect(b.includes("for (const o of model.others) box.appendChild(lineEl(o));")).toBe(true);
    expect(b.includes("for (const o of g.lines) gbox.appendChild(lineEl(o));")).toBe(true);
  });
  it("묶음 머리줄은 button · 접기는 g.key 로 toggleUsageFold · 접힌 묶음은 줄을 그리지 않는다", () => {
    const b = body();
    const g = b.slice(b.indexOf("for (const g of model.groups)"), b.indexOf('if (model.message) kids.push'));
    for (const needle of ['document.createElement("button")', 'gh.className = "usage-group-head";', "toggleUsageFold(g.key)", "if (!g.folded) {", 'gh.setAttribute("aria-expanded"'])
      expect({ 구현: needle, 있음: g.includes(needle) }).toEqual({ 구현: needle, 있음: true });
  });
  it("꼬리 경고 → 숨김 안내 → 본문 교체의 순서는 그대로(상자·묶음은 그 앞)", () => {
    const b = body();
    const iGroups = b.indexOf("for (const g of model.groups)");
    const iMsg = b.indexOf('el("usage-msg"');
    const iFoot = b.indexOf('el("usage-foot"');
    const iHid = b.indexOf('el("usage-hidden"');
    const iRep = b.indexOf("body.replaceChildren(");
    expect({ 순서: iGroups >= 0 && iGroups < iMsg && iMsg < iFoot && iFoot < iHid && iHid < iRep }).toEqual({ 순서: true });
  });
});

describe("0.14.44 D1 — 보기 방식 전환 단추(머리줄의 형제) · 저장 키 · 다시 그리기 판정", () => {
  const body = () => fnBody("renderUsageBar");
  it("전환 단추는 머리줄 단추(.usage-head)의 형제 — 같은 줄 래퍼(.usage-headrow) 안에 둘을 나란히 · 단추 안에 단추 없음", () => {
    const b = body();
    expect(b.includes("headRow.append(head, modeBtn);")).toBe(true);
    expect(b.includes("host.append(headRow, body);")).toBe(true);
    expect(b.includes("head.append(chev, title, sum);")).toBe(true); // 머리 단추의 자식에는 전환 단추가 없다
    expect(b.includes('modeBtn.className = "usage-mode";')).toBe(true);
    expect(b.includes("setUsageMode(nextUsageMode(usageMode))")).toBe(true);
  });
  it("저장 키 cys-usage-mode · cys-usage-fold — 읽기 각 1곳 · 쓰기 각 1곳 · 전부 try 안(저장소 차단·깨진 값이어도 기본값)", () => {
    expect(code.includes('const USAGE_MODE_KEY = "cys-usage-mode";')).toBe(true);
    expect(code.includes('const USAGE_FOLD_KEY = "cys-usage-fold";')).toBe(true);
    for (const k of ["USAGE_MODE_KEY", "USAGE_FOLD_KEY"]) {
      expect({ k, 읽기: code.split(`localStorage.getItem(${k}`).length - 1 }).toEqual({ k, 읽기: 1 });
      expect({ k, 쓰기: code.split(`localStorage.setItem(${k}`).length - 1 }).toEqual({ k, 쓰기: 1 });
    }
    expect(/\ntry \{\n\s*usageMode = sanitizeUsageMode\(localStorage\.getItem\(USAGE_MODE_KEY\)\);\n\} catch/.test(code)).toBe(true);
    expect(/\ntry \{\n\s*usageFolded = sanitizeFoldKeys\(JSON\.parse\(localStorage\.getItem\(USAGE_FOLD_KEY\) \|\| "\[\]"\)\);\n\} catch/.test(code)).toBe(true);
    expect(/\n\s*try \{\n\s*localStorage\.setItem\(USAGE_MODE_KEY, m\);\n\s*\} catch/.test(code)).toBe(true);
    expect(/\n\s*try \{\n\s*localStorage\.setItem\(USAGE_FOLD_KEY, JSON\.stringify\(\[\.\.\.usageFolded\]\)\);\n\s*\} catch/.test(code)).toBe(true);
  });
  it("값 검증은 순수 함수(sanitizeUsageMode · sanitizeFoldKeys) · 저장 값을 그대로 쓰지 않는다 · 기본은 자동", () => {
    expect(code.includes("usageMode = sanitizeUsageMode(localStorage.getItem(USAGE_MODE_KEY));")).toBe(true);
    expect(code.includes('let usageMode: UsageViewMode = "auto";')).toBe(true);
  });
  it("두 상태 선언은 첫 최상위 renderUsageBar() 호출보다 앞(TDZ 면 renderUsageBar 가 catch 로 삼켜 패널이 빈 채로 남는다)", () => {
    const firstTop = code.indexOf("\nrenderUsageBar();");
    for (const decl of ["let usageMode:", "let usageFolded:"]) {
      const at = code.indexOf(decl);
      expect({ decl, 선언: at >= 0, 선언이_먼저: at >= 0 && at < firstTop }).toEqual({ decl, 선언: true, 선언이_먼저: true });
    }
  });
  it("모델 호출이 보기 방식 · 글자 배율 · 접힘을 넘긴다 — 본문 시그니처는 모델 전체라 방식·접힘이 바뀌면 다시 그린다", () => {
    const b = body();
    const call = b.slice(b.indexOf("buildUsageBarModel("), b.indexOf(");", b.indexOf("ccAcctLabel,")) + 2);
    for (const needle of ["mode: usageMode", 'getPropertyValue("--wsbar-font")', "folded: usageFolded"])
      expect({ 인자: needle, 있음: call.includes(needle) }).toEqual({ 인자: needle, 있음: true });
    expect(b.includes("const sig = JSON.stringify(model);")).toBe(true);
  });
  it("글자 배율 단추가 사이드바 사용량을 다시 그린다(1.6 이상이면 자동 방식이 한 줄형으로 — 새 타이머 없음)", () => {
    const b = fnBody("applyWsbarFontStep");
    expect(b.includes("renderUsageBar();")).toBe(true);
  });
  it("전환 단추·묶음 머리줄 규칙이 style.css 에 있다 · 머리줄 래퍼는 같은 줄(flex) · 새 색 정의 없음", () => {
    const flat = css.replace(/\s+/g, " ");
    for (const sel of ["#wsbar-usage .usage-headrow", "#wsbar-usage .usage-mode", "#wsbar-usage .usage-group-head"]) expect({ 규칙: sel, 있음: flat.includes(`${sel} {`) }).toEqual({ 규칙: sel, 있음: true });
    const head = /#wsbar-usage \.usage-headrow \{([^}]*)\}/.exec(flat);
    expect(head).not.toBeNull();
    expect(head![1]).toContain("display: flex");
    expect(/#[0-9a-fA-F]{3,6}\b/.test(/#wsbar-usage \.usage-mode \{([^}]*)\}/.exec(flat)![1].replace("#8b949e", ""))).toBe(false);
  });
});
