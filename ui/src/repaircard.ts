// 「스스로 해결」(0.15.0 · T5b · C5) — 수리 최종 승인 카드. 승인(Feed) 탭 안에서만 그린다 — 아무것도 저절로 뜨지 않는다
// (팀 만들기 제안 확인 창의 '자동으로 띄우지 않는다' 규칙과 같은 동작 · 설계 §4-7 5단계).
// ★카드는 데몬이 계산한 사실(대상 파일 · 원본 바이트 지문 · 예행 결과 · 실패하면 어떻게 돌아오는지)과 워커가 쓴 글을
//   **나누어** 보여 준다(워커의 글에는 숨은 지시가 섞일 수 있다 — 팀 제안 카드의 선례). 모든 글은 textContent 로만 넣는다.
// ★[수정 최종 승인]은 이 화면에서 한 번 더 확인한 뒤에만 데몬에 '시작'을 보낸다 — 승인을 만드는 것은 데몬이 띄운
//   도우미의 운영체제 본인 확인(Touch ID · 로그인 암호)이고, 앱은 결과를 기다리지 않는다(결과는 사건으로 온다).
// 순수 모듈 — DOM 문서와 결정 콜백을 받는다(Tauri 불요 · repaircard.test.ts 가 핀).

export const REPAIR_CARD_TITLE = "수리 최종 승인 요청";
export const REPAIR_APPROVE_LABEL = "수정 최종 승인";
export const REPAIR_REJECT_LABEL = "거절";
export const REPAIR_CONFIRM_TEXT =
  "승인하면 운영체제 본인 확인 창(Touch ID 또는 로그인 암호)이 뜹니다. 본인 확인을 마쳐야 승인되고, 그 뒤 데몬이 이 후보를 적용합니다.";
export const REPAIR_CONFIRM_YES = "본인 확인 시작";
export const REPAIR_CONFIRM_NO = "취소";
export const REPAIR_WORKER_TEXT_HEADING = "워커가 쓴 설명(데몬이 확인한 사실이 아닙니다)";
export const REPAIR_FACTS_HEADING = "데몬이 확인한 사실";
export const REPAIR_TOAST_TITLE = "수리 최종 승인 요청 1건";
export const REPAIR_TOAST_BODY = "Control Center 의 '승인 Feed' 탭에서 카드를 여세요 — 창은 저절로 뜨지 않습니다.";

export interface RepairCardFacts {
  classification_label?: string;
  targets?: string[];
  fingerprint_sha256?: string;
  rehearsal?: { ok: boolean; summary: string } | null;
  prerehearsal_possible?: boolean;
  prerehearsal_note?: string | null;
  on_failure?: string;
  screen_notes?: string[];
  other_lanes?: string[];
  identical_bytes_note?: string | null;
  resubmits_used?: number;
}

export interface RepairCard {
  request_id: string;
  repair: string;
  state: string;
  state_label?: string;
  expires_at?: number;
  facts?: RepairCardFacts;
  worker_text?: string;
  reason?: string | null;
}

/** 남은 시간(분) — 만료 시각(초)과 지금(밀리초). 음수는 0. */
export function minutesLeft(expiresAt: number | undefined, nowMs: number): number {
  if (typeof expiresAt !== "number") return 0;
  return Math.max(0, Math.floor((expiresAt * 1000 - nowMs) / 60000));
}

function line(doc: Document, cls: string, text: string): HTMLElement {
  const d = doc.createElement("div");
  d.className = cls;
  d.textContent = text;
  return d;
}

/**
 * 카드 하나. `decide(request_id, "approve"|"reject")` 는 확인을 마친 뒤에만 불린다.
 * 대기(pending) 카드만 단추를 갖고, 본인 확인 중(verifying)이면 단추 없이 그 상태만 적는다.
 */
export function buildRepairCard(
  doc: Document,
  card: RepairCard,
  nowMs: number,
  decide: (requestId: string, decision: "approve" | "reject") => void,
): HTMLElement {
  const box = doc.createElement("div");
  box.className = "cc-feed-item repair-card";
  box.dataset.requestId = card.request_id;
  const f = card.facts ?? {};
  box.appendChild(line(doc, "repair-card-title", `${REPAIR_CARD_TITLE} — ${card.repair} (${card.state_label ?? card.state})`));
  const facts = doc.createElement("div");
  facts.className = "repair-card-facts";
  facts.appendChild(line(doc, "repair-card-heading", REPAIR_FACTS_HEADING));
  if (f.classification_label) facts.appendChild(line(doc, "repair-card-fact", `종류: ${f.classification_label}`));
  facts.appendChild(line(doc, "repair-card-fact", `대상 파일: ${(f.targets ?? []).join(", ")}`));
  if (f.fingerprint_sha256) facts.appendChild(line(doc, "repair-card-fact", `원본 바이트 지문: ${f.fingerprint_sha256}`));
  if (f.rehearsal) facts.appendChild(line(doc, "repair-card-fact", `예행 결과: ${f.rehearsal.ok ? "통과" : "실패"} — ${f.rehearsal.summary}`));
  if (f.prerehearsal_possible === false && f.prerehearsal_note) facts.appendChild(line(doc, "repair-card-fact repair-card-warn", f.prerehearsal_note));
  if (f.on_failure) facts.appendChild(line(doc, "repair-card-fact", `실패하면: ${f.on_failure}`));
  for (const n of f.screen_notes ?? []) facts.appendChild(line(doc, "repair-card-fact", `보호장치 심사: ${n}`));
  if (f.identical_bytes_note && (f.other_lanes ?? []).length > 0)
    facts.appendChild(line(doc, "repair-card-fact", `${f.identical_bytes_note} (부서: ${(f.other_lanes ?? []).join(", ")})`));
  facts.appendChild(line(doc, "repair-card-fact", `남은 시간: ${minutesLeft(card.expires_at, nowMs)}분`));
  box.appendChild(facts);
  const worker = doc.createElement("div");
  worker.className = "repair-card-worker";
  worker.appendChild(line(doc, "repair-card-heading", REPAIR_WORKER_TEXT_HEADING));
  worker.appendChild(line(doc, "repair-card-worker-text", card.worker_text ?? ""));
  box.appendChild(worker);
  if (card.state !== "pending") return box;
  const actions = doc.createElement("div");
  actions.className = "cc-feed-actions";
  const approve = doc.createElement("button");
  approve.textContent = REPAIR_APPROVE_LABEL;
  const reject = doc.createElement("button");
  reject.textContent = REPAIR_REJECT_LABEL;
  reject.onclick = () => decide(card.request_id, "reject");
  approve.onclick = () => {
    // 한 번 더 확인 — 이 확인은 화면 안의 줄일 뿐 창을 띄우지 않는다
    actions.replaceChildren();
    actions.appendChild(line(doc, "repair-card-confirm", REPAIR_CONFIRM_TEXT));
    const yes = doc.createElement("button");
    yes.textContent = REPAIR_CONFIRM_YES;
    yes.onclick = () => {
      yes.disabled = true;
      decide(card.request_id, "approve");
    };
    const no = doc.createElement("button");
    no.textContent = REPAIR_CONFIRM_NO;
    no.onclick = () => {
      actions.replaceChildren(approve, reject);
    };
    actions.append(yes, no);
  };
  actions.append(approve, reject);
  box.appendChild(actions);
  return box;
}
