// 0.14.44 C2·C3 — 승인 Feed 화면 배선 핀(main.ts · style.css 를 데이터로 읽는다). 판정은 feedclass-0144.test.ts 가 표로 잡는다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf-8");
const stripComments = (s: string): string =>
  s
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");
const code = stripComments(read("./main.ts"));
const css = read("./style.css");
function fnBody(name: string): string {
  const i = code.search(new RegExp(`(async )?function ${name}\\(`));
  expect({ 함수: name, 존재: i >= 0 }).toEqual({ 함수: name, 존재: true });
  const end = code.indexOf("\n}\n", i);
  return code.slice(i, end > i ? end + 3 : undefined);
}

describe("C2 — 목록을 그리는 한 곳만 새 명령 · 나머지 feed_list 호출처는 본부 전용 그대로", () => {
  it("refreshFeed 는 feed_list_all 을 부르고 feed_list 는 폴백에서만(호출 실패·본부 행 실패)", () => {
    const b = fnBody("refreshFeed");
    expect(b.split('invoke("feed_list_all"').length - 1).toBe(1);
    expect(b.split('invoke("feed_list"').length - 1).toBe(1); // 폴백 하나
    expect(b.includes("legacyMode")).toBe(true);
    expect(b.includes("if (legacyMode) renderOtherWorkspacePending(box);")).toBe(true); // 종전 띠는 폴백에서만
  });
  it("feed_list_all 호출은 refreshFeed 하나뿐 · feed_list 의 다른 호출처 셋(승인 전환·팔레트 게이트·수동 승인 게이트)은 그대로", () => {
    expect(code.split('invoke("feed_list_all"').length - 1).toBe(1);
    expect(code.split('feedList: () => invoke("feed_list", { status: null }),').length - 1).toBe(1); // 승인 전환 스케줄러(probefail.test.ts:450 이 같은 줄을 고정)
    expect(code.includes('invoke("feed_list", { status: "pending" })')).toBe(true);
  });
  it("패널이 닫혀 있으면 조회 전에 돌아간다 — 닫힌 채 본부+전 부서를 부르지 않는다(성찰 2회차 m-2) · 세대 번호는 먼저 올린다", () => {
    const b = fnBody("refreshFeed");
    const iGen = b.indexOf("++feedRefreshGen");
    const iClosed = b.indexOf('if (!(ccOpen && ccTab === "feed")) return;');
    const iAll = b.indexOf('invoke("feed_list_all"');
    expect(iGen >= 0 && iClosed > iGen && iAll > iClosed).toBe(true);
  });
  it("`refreshFeed();` 라는 호출 꼴이 남아 있다(소스를 이 글자로 자르는 시험 셋이 의존)", () => {
    expect(code.includes("refreshFeed();")).toBe(true);
    const i = code.indexOf('if (name === "feed.item.created") {');
    expect(i).toBeGreaterThan(0);
    expect(code.indexOf("refreshFeed();", i)).toBeGreaterThan(i);
  });
  it("부서 항목의 응답은 그 부서 소켓으로 — 치우기·Allow/Deny·모두 확인 모두 socket 을 넘긴다 · 열쇠는 (소켓, request_id) 로 클로저에 묶인다", () => {
    expect(code.includes('invoke("feed_reply", { requestId, decision: "dismissed", socket })')).toBe(true); // wireFeedDismiss
    const b = fnBody("refreshFeed");
    expect(b.includes('invoke("feed_reply", { requestId: item.request_id, decision, socket: sock })')).toBe(true);
    expect(fnBody("renderBulkConfirm").includes('invoke("feed_reply", { requestId: t.request_id, decision: "dismissed", socket })')).toBe(true);
    expect(b.includes("wireFeedDismiss(dismiss, item.request_id, sock)")).toBe(true);
    expect(b.includes("wireFeedDismiss(clear, item.request_id, sock)")).toBe(true);
    expect(b.includes("wireFeedDismiss(ok, item.request_id, sock)")).toBe(true);
  });
  it("부서별 묶음 머리글 · 응답 없는 부서 한 줄 · 「그 화면으로 이동」 · 「그 부서로 이동」", () => {
    const b = fnBody("refreshFeed");
    for (const needle of ["feed-dept-head", "대기 ${sPending.length}건", "이 부서는 지금 응답이 없습니다 — 자동으로 다시 확인합니다", "그 화면으로 이동", "그 부서로 이동"])
      expect({ 구현: needle, 있음: b.includes(needle) }).toEqual({ 구현: needle, 있음: true });
    expect(fnBody("jumpToDeptSurface").includes("switchToWorkspaceBySocket(socket)")).toBe(true);
    expect(fnBody("jumpToDeptSurface").includes("inject-flash")).toBe(true);
  });
  it("본부 전용 절차 두 종류는 부서 소켓에서 Allow 를 두지 않는다 — 분기가 team-create 분기보다 앞", () => {
    const b = fnBody("refreshFeed");
    const iHead = b.indexOf("sock && isHeadOnlyProcedureKind(item.kind)");
    const iTeam = b.indexOf('classifyPendingFeed(item) === "team-create"');
    expect(iHead >= 0 && iHead < iTeam).toBe(true);
  });
  it("응답할 수 없는 부서 소켓(replyable=false)은 단추를 내리고 이동만 · 거부 코드는 사람 말로", () => {
    expect(fnBody("refreshFeed").includes("sock && !origin.replyable")).toBe(true);
    expect(fnBody("feedReplyErrorText").includes("socket_not_allowed")).toBe(true);
  });
  it("토큰은 화면에 없다 — main.ts 에 operator_token 0건 · 화면이 소켓의 토큰 폴더를 읽지 않는다", () => {
    expect(code.includes("operator_token")).toBe(false);
    expect(code.includes("operator.token")).toBe(false);
  });
});

describe("C3 — 표지 · 「확인」 · 「모두 확인」 배선", () => {
  it("표지는 단추를 바꾸지 않는다 — Allow·Deny 를 그린 뒤에 「치우기」를 더한다(isEndedSeatRequest 판정)", () => {
    const b = fnBody("refreshFeed");
    const iAllow = b.indexOf('[["Allow", "allow", "allow"], ["Deny", "deny", "deny"]]');
    const iMark = b.indexOf("if (isEndedSeatRequest(item)) {");
    expect(iAllow >= 0 && iMark > iAllow).toBe(true);
    const seg = b.slice(iMark, b.indexOf("el.appendChild(actions);", iMark));
    expect(seg.includes("ENDED_SEAT_MARK")).toBe(true);
    expect(seg.includes('"치우기"')).toBe(true);
    expect(seg.includes("btns")).toBe(false); // Allow·Deny 단추 배열을 건드리지 않는다(비활성화·제거 0)
    expect(seg.includes("remove")).toBe(false);
  });
  it("정보성 알림 「확인」 — isConfirmableNotice 일 때만 Allow·Deny 대신 · 응답은 dismissed(wireFeedDismiss)", () => {
    const b = fnBody("refreshFeed");
    const i = b.indexOf("isConfirmableNotice(item)");
    expect(i).toBeGreaterThan(0);
    const seg = b.slice(i, b.indexOf("} else if (item.status === \"pending\") {", i));
    expect(seg.includes("NOTICE_CONFIRM_LABEL")).toBe(true);
    expect(seg.includes("wireFeedDismiss(ok, item.request_id, sock)")).toBe(true);
    expect(seg.includes('"allow"') || seg.includes('"deny"')).toBe(false);
  });
  it("「모두 확인」 — isBulkConfirmable 만 · 최대 BULK_CONFIRM_MAX · 하나씩 순서대로(동시 폭주 금지) · dismissed · 소켓별", () => {
    const b = fnBody("renderBulkConfirm");
    expect(b.includes("items.filter((i) => isBulkConfirmable(i)).slice(0, BULK_CONFIRM_MAX)")).toBe(true);
    expect(b.includes("for (const t of targets) {")).toBe(true);
    expect(b.includes("await invoke(")).toBe(true);
    expect(b.includes("Promise.all")).toBe(false);
    expect(b.includes('decision: "dismissed"')).toBe(true);
    expect(b.includes('"allow"')).toBe(false);
    const r = fnBody("refreshFeed");
    expect(r.includes("renderBulkConfirm(box, pendingItems);")).toBe(true); // 본부
    expect(r.includes("renderBulkConfirm(box, sPending, sec.socket);")).toBe(true); // 부서(소켓별)
  });
  it("style.css 에 부서 머리글 · 모두 확인 줄 · 표지 규칙", () => {
    for (const sel of [".feed-dept-head", ".feed-bulk", ".feed-item .fi-ended-mark"]) expect({ 규칙: sel, 있음: css.includes(`${sel} {`) }).toEqual({ 규칙: sel, 있음: true });
  });
});

describe("리뷰 반영 — 세대 가드 · 부서 더 보기 · 응답 불가 부서의 모두 확인 · waiter 모름", () => {
  it("refreshFeed 는 세대 가드를 둔다 — 응답을 기다린 뒤 더 새 갱신이 있으면 그리지 않는다", () => {
    const b = fnBody("refreshFeed");
    expect(b.includes("const myGen = ++feedRefreshGen;")).toBe(true);
    expect(b.indexOf("if (myGen !== feedRefreshGen) return;") > b.indexOf('invoke("feed_list"')).toBe(true);
  });
  it("부서 묶음에도 '더 보기'가 있고(소켓 키로 펼침 기억) 응답 불가 부서에는 「모두 확인」이 없다", () => {
    const b = fnBody("refreshFeed");
    expect(b.includes("feedDeptExpanded.has(sec.socket)")).toBe(true);
    expect(b.includes("if (sec.replyable) renderBulkConfirm(box, sPending, sec.socket);")).toBe(true);
  });
  it("정보성 알림 「확인」은 waiter === false 일 때만 — true 이거나 모름(칸 없음)이면 종전 Allow·Deny", () => {
    const fc = read("./feedclass.ts");
    expect(fc.includes("i.waiter === false")).toBe(true);
    expect(fc.includes("i.waiter !== true")).toBe(false);
  });
});
