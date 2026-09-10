// pane 전출(동일 socket 워크스페이스 간)의 순수 트리 변형 로직 (DOM 무접촉 — main.ts transferPaneToWs가 배선).
//
// 불변식: ①원자성 — 실패(null 반환) 시 호출측이 두 트리 모두 무변경 유지 ②sid 유일성 —
// src에서 제거한 노드만 dest에 붙이고, dest에 같은 sid가 이미 있으면 거부(유령 pane 차단).

export type TNode =
  | { type: "split"; dir: "row" | "col"; ratio?: number; a: TNode; b: TNode }
  | { type: "pane"; sid: number };

export function treeSids(node: TNode | null, out: number[] = []): number[] {
  if (!node) return out;
  if (node.type === "pane") out.push(node.sid);
  else {
    treeSids(node.a, out);
    treeSids(node.b, out);
  }
  return out;
}

// sid pane 노드를 제거하고 형제로 붕괴시킨다(main.ts replaceNode의 제거 특수형과 동일 의미).
function removeSid(node: TNode, sid: number): TNode | null {
  if (node.type === "pane") return node.sid === sid ? null : node;
  const a = removeSid(node.a, sid);
  const b = removeSid(node.b, sid);
  if (a && b) return { ...node, a, b };
  return a ?? b;
}

// 트리 말단(우측 row 분할)에 pane을 덧붙인다 — actionNew의 삽입 규칙과 동일.
export function appendPane(tree: TNode | null, sid: number): TNode {
  const moved: TNode = { type: "pane", sid };
  return tree ? { type: "split", dir: "row", a: tree, b: moved } : moved;
}

// src에서 sid를 떼어 dest 끝에 붙인 새 (src, dest) 트리 쌍. 원천 부재·대상 중복이면 null.
export function transferTrees(
  src: TNode | null,
  dest: TNode | null,
  sid: number,
): { src: TNode | null; dest: TNode } | null {
  if (!src || !treeSids(src).includes(sid)) return null;
  if (dest && treeSids(dest).includes(sid)) return null;
  return { src: removeSid(src, sid), dest: appendPane(dest, sid) };
}

// ── ★(0.14.31 · WP-4 R2) 전출 목적지 좌석 식별 ─────────────────────────────────
//
// 무엇을 고치는가: 크로스 부서 전출은 목적지에 **셸 pane** 을 만들고 그 셸에
// `cys launch-agent …` 를 주입한다. 그런데 `launch-agent` 는 그 셸 안에서 에이전트를 띄우는
// 것이 아니라 데몬에 **새 surface 를 만든다**. 종전 코드는 그 사실을 모른 채 런처 셸을 계속
// 목적지로 삼아, 준비 폴링·핸드오프 큐잉을 **전부 빈 셸에** 하고 원본을 닫았다 — 리뷰어는
// 핸드오프를 못 받고 작업 세션은 사라진다(리뷰어 blocking).
//
// R1 은 "런치 뒤에 나타난 같은 역할 좌석이 유일하면 그것"으로 좁혔지만, 그 조건은 **그 전출의
// 결과라는 증거가 아니다**(codex 적대검증 R2 blocking):
//   ⓐ `!!s.agent` 는 메타데이터 이름이 등록됐다는 뜻일 뿐 **생존 관측이 아니다** —
//      `launch-agent` 는 준비 판정 **전에** 메타를 세우고 `agent_seen=false` 로 출발한다.
//      즉 즉사한 CLI 도, 아직 뜨지도 않은 CLI 도 이 검사를 통과했다.
//   ⓑ 같은 시간대에 다른 경로가 같은 역할 좌석을 하나 띄우면 **무관한 좌석**이 목적지가 된다.
//   ⓒ agent 종류(claude/codex/gemini)를 보지 않아 다른 종류의 유일 후보도 선택됐다.
// 그래서 R2 는 **데몬이 기록한 생성자**(`created_by` — `surface.create` 호출자의 pane id를
// 데몬이 발신 pid 로 도출해 원장에 적은 값 · 호출자가 신고할 수 없다)를 축으로 삼는다:
//   · `created_by === 런처 셸 sid` — 이 좌석이 **내가 보낸 그 명령**의 산물이라는 증거
//   · `agent_alive === true` — 데몬 watchdog 이 그 프로세스를 **실제로 관측**했다(3값 축:
//     `null` 은 '말할 것이 없음'이고 `false` 는 '종료 통지됨'이다 — 둘 다 확정 근거가 아니다)
//   · `agent === <--agent 인자>` — 시킨 종류가 떴는가
//   · 역할 일치 · 미종료 · 런치 이후 생성 · 런치 전 집합에 없음 · **정확히 하나**
// 확정하지 못하면 **원본을 닫지 않는다**(보상 롤백) — 실패 방향은 언제나 '전출 안 함'이다.
export type SurfaceRow = {
  surface_id: number;
  role?: string | null;
  agent?: string | null;
  /** 3값 생존 관측(true=관측됨 · false=종료 통지 · null=말할 것 없음). `agent` 이름과 다르다. */
  agent_alive?: boolean | null;
  exited?: boolean | null;
  created_at?: number | null;
  /** 데몬이 기록한 **생성자 pane** — 호출자가 신고할 수 없는 값(surface.list `created_by`). */
  created_by?: number | null;
  /** 각성 래치(첫 자기보고 시각) — null 은 '아직 각성 증거 없음'. */
  awakened_at?: number | null;
};

export function pickLaunchedAgentSid(
  before: number[],
  launcherSid: number,
  after: SurfaceRow[],
  wantRole: string,
  wantAgent: string,
  launchedAt?: number,
): number | null {
  const known = new Set(before);
  const hits = after.filter(
    (s) =>
      s.surface_id !== launcherSid &&
      !known.has(s.surface_id) &&
      !s.exited &&
      !!s.role &&
      s.role === wantRole &&
      // ★생성자 증거 — 이 좌석이 그 런처 셸에서 태어났는가(부재=모름이지 아님이 아니다 →
      //   확정하지 않는다: fail-closed).
      s.created_by === launcherSid &&
      // ★생존 **관측**(메타 등록이 아니다). `=== true` 로 3값을 좁힌다.
      s.agent_alive === true &&
      // ★시킨 종류가 떴는가(다른 CLI 의 유일 후보를 목적지로 삼지 않는다).
      !!s.agent &&
      s.agent === wantAgent &&
      (launchedAt == null || s.created_at == null || s.created_at >= launchedAt),
  );
  return hits.length === 1 ? hits[0].surface_id : null;
}

/** 목적지가 **지금** 인계를 받을 수 있는 상태인가 — 선택 때 쓴 술어를 그대로 재평가한다.
 *
 * ★왜 생존 관측만으로 부족한가(codex 적대검증 R2 blocking): 프로세스가 살아 있어도 신뢰
 * 관문·인증 화면에 앉아 있을 수 있다. 그 상태에서 원본을 닫으면 인계는 아무도 읽지 않는다.
 *
 * ★(0.14.31 · 성찰 C2 · blocking) 종전 이 술어는 `awakened_at`(각성 래치)을 **필수**로 요구했다.
 * 그 래치의 유일한 write path 는 `status.set` 이고 그것은 LLM 이 `cys set-status` 를 실행할 때만
 * 난다 — 훅에는 호출이 0건이고 `REVIEWER_DIRECTIVE` 는 그 줄의 실행을 **금지**하며 gemini/codex 는
 * Claude Code 훅도 돌지 않는다. 즉 리뷰어 좌석은 **구조적으로** 각성을 신고할 수 없어 리뷰어 전출은
 * 영구 보류였고, 재시도마다 런처 셸 pane 이 하나씩 늘었다. 그래서 이 술어는 이제 **좌석 축**만 본다
 * (역할 보유 · 생존 관측 · 시킨 종류 · 미종료). "인계를 실제로 받았는가" 는 별도의 **수신자 인수
 * 확인**([`originCloseVerdict`] 의 `acked` — 목적지 에이전트가 쓰는 `<handoff>.received` 파일)이
 * 잰다. 각성과 인수는 다른 축이고, 인수 파일은 어떤 CLI 든 쓸 수 있다.
 *
 * `awakened_at` 은 행에 남아 있지만 이 술어는 읽지 않는다(null 은 여전히 '말할 것 없음'이다).
 * 모르면 거짓이다. */
export function destinationCanReceive(
  row: SurfaceRow | undefined,
  wantRole?: string,
  wantAgent?: string,
): boolean {
  if (!row || row.exited) return false;
  // 역할 축: 인계는 '그 역할' 앞으로 간다 — 역할을 잃은 좌석은 수신자가 아니다.
  if (!row.role) return false;
  if (wantRole != null && row.role !== wantRole) return false;
  // 생존 축: `null`(말할 것 없음)·`false`(종료 통지) 둘 다 근거가 아니다.
  if (row.agent_alive !== true) return false;
  // 종류 축: 시킨 CLI 가 아직 그 좌석의 것인가(호출부가 요구할 때만).
  if (wantAgent != null && row.agent !== wantAgent) return false;
  return true;
}

/** 목적지가 **확정적으로** 사라졌는가(종료 통지 · 좌석 소멸). `null` 생존은 여기서도 '모름'이다 —
 *  모름은 보류 사유이지 소멸 사유가 아니다(소멸로 읽으면 살아 있는 좌석 옆에 새 좌석을 띄운다). */
export function destinationGone(row: SurfaceRow | undefined, rowsObserved: boolean): boolean {
  if (!rowsObserved) return false;
  if (!row) return true;
  return row.exited === true || row.agent_alive === false;
}

// ── ★(0.14.31 · 성찰 C1·C2) 인계 적재 영수증 · 인수 확인 · 원본 종료 결정 ─────────────────────
//
// 【C1 · blocking】 종전 GUI 는 `send_input(queued)` 의 데몬 응답을 **버렸다**(tauri 가 `()` 로 접었다).
// 데몬은 그 응답에 `queue_entry_id` 와 `durable`(큐 WAL 치환 성공 여부)을 싣는다 — `durable:false` 는
// "이 항목은 아직 메모리에만 있다(데몬이 죽으면 사라진다)" 는 뜻인데, GUI 는 그 사실을 모른 채 원본을
// 닫았다. 목적지 데몬이 그 사이 죽으면 인계는 **복구 불가**로 유실된다.
//
// 【C2 · blocking】 원본 종료의 관문이 `awakened_at` 하나였다(위 `destinationCanReceive` doc).
//
// 【설계 · codex 설계 검토(2026-09-10) 반영】 처음 설계는 데몬의 `queue.delivered` 이벤트를 인수 사실로
// 쓰려 했다. codex 가 지적한 대로 그것은 **PTY writer 의 claim**(첫 바이트 전에 claim · 쓰기 실패는
// 루프만 종료)이지 에이전트의 소비 확인이 아니다(governance.rs 의 배달자 · state.rs 의 writer). 그래서
// 종료 조건은 **수신자 자신의 인수 확인**으로 바꿨다: 큐로 보내는 지시문이 "읽었으면 `<handoff>.received`
// 파일을 써라" 를 포함하고, GUI 는 그 파일의 실존(비어 있지 않음)을 관측한다. 원본 쪽 5필드 파일 검증과
// 대칭인 결정론 신호이고, `cys set-status` 와 달리 어떤 CLI 든 쓸 수 있다(리뷰어 포함). 인계 문서 경로가
// 전출마다 유일하므로 파일은 그 전출에 결속된다.
//
// 【실패 방향】 어느 갈래든 '원본 보존'이다. 인수 확인 없음 · 목적지 미수신 · 관측 실패 전부 보류.
// 보류는 **인계를 다시 적재하지 않는다**(중복 enqueue 0) — 재시도는 [`transferRetryAction`] 이 같은
// 원본의 전출 기록으로 **관측만** 재개한다(런처 셸·좌석·인계 문서를 다시 만들지 않는다 = pane 증식 0).

/** 데몬 `surface.send_text`(queued) 응답에서 GUI 가 소비하는 두 사실. 결측은 값이 아니다(`null`). */
export type EnqueueReceipt = {
  /** 큐 항목 id — 이후 관측의 조준점. `null` = 데몬이 돌려주지 않았다(구 데몬 · 적재 미확인). */
  entryId: string | null;
  /** WAL 치환 성공 여부. `false` = 메모리에만 있다. `null` = 데몬이 이 축을 말하지 않는다. */
  durable: boolean | null;
};

export function parseEnqueueReceipt(v: unknown): EnqueueReceipt {
  if (!v || typeof v !== "object") return { entryId: null, durable: null };
  const o = v as Record<string, unknown>;
  const id = o.queue_entry_id;
  const durable = o.durable;
  return {
    entryId: typeof id === "string" && id.length > 0 ? id : typeof id === "number" ? String(id) : null,
    durable: typeof durable === "boolean" ? durable : null,
  };
}

/** 수신자 인수 확인 파일 — 인계 문서 옆, 같은 이름 + `.received`. 전출마다 유일(문서 경로가 유일). */
export function handoffAckPath(handoffPath: string): string {
  return `${handoffPath}.received`;
}

/** 목적지 좌석에 큐로 보내는 지시문 — 인수 확인 파일 쓰기를 **요구**한다(이 파일이 원본 종료의 유일한
 *  신호다). 문안은 여기 하나다(테스트가 인수 요구를 핀한다). */
export function handoffInstruction(role: string, handoffPath: string): string {
  const ack = handoffAckPath(handoffPath);
  return (
    `너는 전출된 ${role} 다. ${handoffPath} 를 읽고 작업을 이어가라. ` +
    `읽었으면 가장 먼저 ${ack} 파일에 "received" 한 줄을 써서 인수를 알려라` +
    `(이 파일이 원본 pane 을 닫아도 된다는 유일한 신호다 — 쓰지 않으면 원본은 보존된다).`
  );
}

/** 원본 종료 결정의 재료 — 전부 **관측된 사실**이다(추정 없음). */
export type HandoffFacts = {
  /** 목적지 좌석 행(`surface.list`). 관측 실패면 `undefined` + `rowsObserved:false`. */
  row: SurfaceRow | undefined;
  /** `surface.list` 응답을 받았는가(행 부재와 관측 실패를 가른다). */
  rowsObserved: boolean;
  wantRole?: string;
  wantAgent?: string;
  entryId: string | null;
  durable: boolean | null;
  /** 수신자 인수 확인 파일이 존재하고 비어 있지 않다. */
  acked: boolean;
};

export type CloseVerdict =
  | { close: true }
  | {
      close: false;
      /** `destination` = 좌석이 수신 가능 상태가 아니다 · `not-acked` = 인수 확인이 아직 없다. */
      hold: "destination" | "not-acked";
      /** 목적지가 확정적으로 사라졌다(전출 기록을 지워도 되는 유일한 경우). */
      gone: boolean;
      note: string;
    };

/** 원본을 **지금** 닫아도 되는가. 닫는 조건은 인수 확인 ∧ 좌석 수신 가능 — 둘 다 관측된 사실이다.
 *
 * `durable`·`entryId` 는 결정을 바꾸지 않고 **보류 사유에 실린다**: 인수 확인은 배달·내구보다 강한
 * 사실(소비)이므로 인수가 있으면 `durable:false` 여도 닫는다. 인수가 없으면 `durable:false` 는 "인계가
 * 데몬 재기동에 유실될 수 있다" 는 경고로, `entryId:null` 은 "적재 자체가 미확인" 으로 문안에 남는다. */
export function originCloseVerdict(f: HandoffFacts): CloseVerdict {
  if (!destinationCanReceive(f.row, f.wantRole, f.wantAgent)) {
    const gone = destinationGone(f.row, f.rowsObserved);
    const why = !f.rowsObserved
      ? "목적지 좌석을 관측하지 못했다(RPC 실패)"
      : !f.row
        ? "목적지 좌석이 사라졌다"
        : f.row.exited
          ? "목적지 좌석이 종료됐다"
          : f.row.agent_alive === false
            ? "목적지 에이전트가 죽었다(셸만 남음)"
            : f.row.agent_alive == null
              ? "목적지 에이전트의 생존이 관측되지 않았다"
              : !f.row.role || (f.wantRole != null && f.row.role !== f.wantRole)
                ? "목적지 좌석이 역할을 잃었다"
                : "목적지 좌석의 에이전트 종류가 시킨 것과 다르다";
    return { close: false, hold: "destination", gone, note: why };
  }
  if (!f.acked) {
    let note = "목적지가 인수 확인 파일을 아직 쓰지 않았다";
    if (f.durable === false) note += " · 큐 항목이 디스크에 확정되지 않았다(durable:false — 데몬 재기동 시 유실 가능)";
    if (f.entryId == null) note += " · 데몬이 큐 항목 id 를 돌려주지 않았다(구 데몬 — 적재 미확인)";
    return { close: false, hold: "not-acked", gone: false, note };
  }
  return { close: true };
}

/** 같은 원본 pane 의 진행 중 전출 기록 — 재시도는 이것으로 **관측만** 재개한다. */
export type TransferRecord = {
  state: "in-progress" | "awaiting-ack";
  /** 목적지 부서 socket(부서 식별). 기본 데몬은 `undefined`. */
  destSocket: string | undefined;
  /** 목적지 에이전트 좌석(런처 셸이 아니다). */
  destSid: number;
  /** 런처 셸(기동이 진행 중일 수 있어 남긴다). */
  launcherSid: number | null;
  handoffPath: string;
  role: string;
  wantAgent: string;
  entryId: string | null;
  durable: boolean | null;
  sinceMs: number;
};

export type RetryAction = "fresh" | "busy" | "resume" | "other-destination";

/** 재시도 계획(순수). `fresh` 만 새 좌석을 만든다 — 나머지는 pane 을 하나도 늘리지 않는다.
 *  · `busy`: 같은 원본의 전출이 아직 진행 중이다(중복 클릭) → 무동작.
 *  · `resume`: 인계는 이미 적재됐다 → 인수 확인 관측만 다시 한다(적재 0 · 기동 0).
 *  · `other-destination`: 앞선 전출이 다른 부서에 인수 대기 중이다 → 그쪽을 먼저 매듭짓는다. */
export function transferRetryAction(
  rec: TransferRecord | undefined,
  destSocket: string | undefined,
): RetryAction {
  if (!rec) return "fresh";
  if (rec.state === "in-progress") return "busy";
  return rec.destSocket === destSocket ? "resume" : "other-destination";
}
