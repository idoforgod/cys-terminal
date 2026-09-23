// U6(0.14.41) 「피드백 보내기」 — 사이드바 단추 + 작성 창(DOM 배선 · 의존성 주입 · 최상위 부수효과 0).
//
// 1단계(설계 §3 U6): 설명(필수)·첨부(끌어다 놓기·붙여넣기·파일 고르기)·진단 동봉(가림 후 미리보기)·
// 개인정보 고지 → [메일로 보내기] = 이 컴퓨터에 묶음(~/.cys/feedback/<id>/) → 묶음 폴더 열기 →
// 기본 메일 앱을 받는 주소·제목·본문 자동으로 열기(mailto). 서버 전송은 없다(2단계).
//
// 안전 계약(반박 반영):
//  · 창은 `.modal-overlay` 기본층(z 1000) — 전역 단축키 차단(main.ts keydown)을 물려받고, 위에 뜨는
//    확인 창·자동 모달이 밑에 깔리지 않는다(D4). 결과·오류는 **창 안**에 보인다(토스트는 창 밑).
//  · 창이 떠 있는 동안 setFocus 는 xterm 에 포커스를 주지 않는다(main.ts · modalguard — D2).
//    여기서는 이중 방어로 focusin 을 지켜 모달 층 밖으로 나간 포커스를 되찾는다.
//  · keydown 캡처·focusin·드롭 구독은 **finally** 에서 반드시 걷힌다 — 남으면 pane 의 Esc(vim·claude
//    중단)가 전역으로 삼켜진다(D13).
//  · 이 창은 feedback_* 커맨드만 부른다 — 피드백 원문이 데몬·에이전트 큐·PTY 로 가는 경로 0.
//  · 다른 모달·팔레트가 떠 있으면 열지 않는다(중첩 금지). 재진입 가드는 첫 await 앞.
import {
  FEEDBACK_ALLOWED_EXT,
  FEEDBACK_CHUNK_BYTES,
  FEEDBACK_MAX_FILES,
  FEEDBACK_PRIVACY_NOTICE,
  attachmentKind,
  canSubmit,
  captureTips,
  chunkRanges,
  feedbackImageExt,
  formatBytes,
  mailSizeNote,
  shouldConfirmDiscard,
  submitBlockReason,
  validateAttachment,
} from "./feedback";
import { baseName } from "./ftdrop";
import { modalLayerOpen, shouldReclaimFocus } from "./modalguard";

/** 진단에 넣을 앱 안 경량 사실(키는 Rust DiagFacts 필드명 그대로). */
export interface FeedbackFacts {
  user_agent: string;
  daemon: string;
  daemon_version: string | null;
  workspaces: number;
  panes: number;
}

export interface FeedbackDeps {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  /** 원시 바이트 IPC(본문 = 바이트, 헤더로 메타) — 파일 고르기·붙여넣기 조각 전송. */
  invokeRaw: (cmd: string, bytes: Uint8Array, headers: Record<string, string>) => Promise<unknown>;
  listen: (name: string, handler: (e: { payload: unknown }) => void) => Promise<() => void>;
  facts: () => Promise<FeedbackFacts>;
  /** 창을 닫은 뒤 원래 pane 으로 포커스 복귀. */
  restoreFocus: () => void;
  platform: "mac" | "win" | "other";
}

interface Attachment {
  name: string;
  original: string;
  size: number;
  kind: string;
}

interface BundleReport {
  id: string;
  folder: string;
  to: string;
  subject: string;
  attachments: Attachment[];
  total_bytes: number;
  include_diag: boolean;
  mail_app: boolean;
}

let feedbackOpen = false;

const BUTTON_HTML =
  '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/><path d="M8 9h8M8 13h5"/></svg>' +
  "<span>피드백 보내기</span>";

/** 사이드바 바닥 공용 칸의 피드백 자리(#wsbar-feedback-slot · 칸 자체는 A2 소유)에 단추를 단다. 멱등·null 안전. */
export function mountFeedbackButton(slot: HTMLElement | null, onClick: () => void): void {
  if (!slot || slot.querySelector("#btn-feedback")) return;
  const b = document.createElement("button");
  b.id = "btn-feedback";
  b.type = "button";
  b.title = "피드백 보내기 — 불편한 점이나 바라는 점을 사진·영상과 함께 알려 주세요";
  b.setAttribute("aria-label", "피드백 보내기");
  b.innerHTML = BUTTON_HTML;
  b.addEventListener("click", onClick);
  slot.appendChild(b);
}

const MODAL_HTML =
  '<div class="modal feedback-modal" role="dialog" aria-modal="true" aria-labelledby="fb-title">' +
  '<div class="fb-head"><h3 id="fb-title">피드백 보내기</h3><button type="button" class="fb-x" aria-label="닫기">×</button></div>' +
  '<div class="fb-compose">' +
  '<p class="fb-intro">불편한 점이나 바라는 점을 알려 주세요. 사진·영상을 함께 넣어 주시면 더 빨리 고칠 수 있습니다.</p>' +
  '<textarea class="modal-input fb-desc" rows="6" maxlength="10000" placeholder="무엇을 하다가 어떤 일이 있었나요? (필수)"></textarea>' +
  '<div class="fb-drop"><div class="fb-drop-row"><span class="fb-drop-hint"></span>' +
  '<button type="button" class="fb-btn fb-pick">파일 고르기</button></div>' +
  '<input type="file" class="fb-file" multiple hidden />' +
  '<ul class="fb-list"></ul><p class="fb-size-note" hidden></p></div>' +
  '<p class="fb-tip"></p>' +
  '<p class="fb-warn">⚠ 화면에 비밀번호·API 키·개인 정보가 보이지 않는지 확인해 주세요.</p>' +
  '<label class="fb-diag-row"><input type="checkbox" class="fb-diag" checked />' +
  "<span>진단 정보 함께 넣기 (앱·운영체제 버전, 데몬 응답 여부, 화면 수 — 개인 폴더 경로는 가립니다)</span></label>" +
  '<button type="button" class="fb-btn fb-diag-view">넣을 진단 내용 보기</button>' +
  '<pre class="fb-diag-pre" hidden></pre>' +
  '<details class="fb-privacy"><summary>개인정보 안내</summary><p class="fb-privacy-body"></p></details>' +
  '<div class="fb-confirm" hidden><span>작성 중인 내용을 버릴까요?</span>' +
  '<button type="button" class="fb-btn fb-keep">계속 쓰기</button>' +
  '<button type="button" class="fb-btn fb-discard">버리기</button></div>' +
  "</div>" +
  '<div class="fb-done" hidden><p class="fb-done-msg"></p><p class="fb-done-addr"></p><p class="fb-done-note"></p></div>' +
  '<p class="fb-status" role="status" aria-live="polite"></p>' +
  '<p class="modal-hint fb-hint"></p>' +
  '<div class="modal-btns fb-compose-btns"><button type="button" class="modal-no fb-cancel">취소</button>' +
  '<button type="button" class="modal-yes fb-send" disabled>메일로 보내기</button></div>' +
  '<div class="modal-btns fb-done-btns" hidden>' +
  '<button type="button" class="fb-copy">주소 복사</button>' +
  '<button type="button" class="fb-folder">폴더 열기</button>' +
  '<button type="button" class="fb-mailapp" hidden>Mail 앱에 첨부해 열기</button>' +
  '<button type="button" class="fb-remail">메일 다시 열기</button>' +
  '<button type="button" class="modal-yes fb-close">닫기</button></div>' +
  "</div>";

function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

const kindLabel = (k: string): string => (k === "video" ? "영상" : k === "text" ? "텍스트" : "사진");

/** 피드백 창을 연다. 이미 열렸거나 다른 모달·팔레트가 떠 있으면 아무것도 하지 않는다. */
export async function openFeedbackModal(deps: FeedbackDeps): Promise<void> {
  if (feedbackOpen || modalLayerOpen(document)) return;
  feedbackOpen = true;
  try {
    await runFeedbackModal(deps);
  } finally {
    feedbackOpen = false;
  }
}

async function runFeedbackModal(deps: FeedbackDeps): Promise<void> {
  const ov = document.createElement("div");
  ov.className = "modal-overlay feedback-overlay";
  ov.innerHTML = MODAL_HTML;
  const q = <T extends HTMLElement>(sel: string): T => ov.querySelector(sel) as T;
  const box = q<HTMLElement>(".feedback-modal");
  const compose = q<HTMLElement>(".fb-compose");
  const doneView = q<HTMLElement>(".fb-done");
  const desc = q<HTMLTextAreaElement>(".fb-desc");
  const drop = q<HTMLElement>(".fb-drop");
  const fileInput = q<HTMLInputElement>(".fb-file");
  const list = q<HTMLElement>(".fb-list");
  const sizeNote = q<HTMLElement>(".fb-size-note");
  const diagChk = q<HTMLInputElement>(".fb-diag");
  const diagView = q<HTMLButtonElement>(".fb-diag-view");
  const diagPre = q<HTMLElement>(".fb-diag-pre");
  const confirmBox = q<HTMLElement>(".fb-confirm");
  const status = q<HTMLElement>(".fb-status");
  const hint = q<HTMLElement>(".fb-hint");
  const sendBtn = q<HTMLButtonElement>(".fb-send");
  const composeBtns = q<HTMLElement>(".fb-compose-btns");
  const doneBtns = q<HTMLElement>(".fb-done-btns");
  const closeBtn = q<HTMLButtonElement>(".fb-close");
  const mailAppBtn = q<HTMLButtonElement>(".fb-mailapp");
  const folderBtn = q<HTMLButtonElement>(".fb-folder");

  // 고정 틀 밖의 문구는 textContent 로만 넣는다(사용자·플랫폼 값이 innerHTML 에 닿지 않게).
  const pasteKey = deps.platform === "mac" ? "⌘V" : "Ctrl+V";
  q<HTMLElement>(".fb-drop-hint").textContent = `여기에 사진·영상을 끌어다 놓거나, 캡처한 뒤 ${pasteKey} 로 붙여넣으세요. (최대 ${FEEDBACK_MAX_FILES}개)`;
  q<HTMLElement>(".fb-tip").textContent = captureTips(deps.platform);
  q<HTMLElement>(".fb-privacy-body").textContent = FEEDBACK_PRIVACY_NOTICE;
  fileInput.accept = FEEDBACK_ALLOWED_EXT.map((e) => "." + e).join(",");

  let atts: Attachment[] = [];
  let pending = 0;
  let submitting = false;
  let done = false;
  let closedFlag = false;
  let pasteSeq = 0;
  let report: BundleReport | null = null;
  let draftP: Promise<string> | null = null;
  let factsP: Promise<FeedbackFacts> | null = null;
  let chain: Promise<void> = Promise.resolve();
  let resolveClose: () => void = () => {};
  const closed = new Promise<void>((r) => {
    resolveClose = r;
  });
  const unlisteners: Array<() => void> = [];
  let disposed = false;

  const setStatus = (msg: string, tone: "" | "ok" | "error" = "") => {
    status.textContent = msg;
    status.className = "fb-status" + (tone ? " " + tone : "");
  };
  const totalBytes = () => atts.reduce((s, a) => s + a.size, 0);

  const refresh = () => {
    const st = { desc: desc.value, pending, submitting };
    sendBtn.disabled = done || !canSubmit(st);
    hint.textContent = done ? "" : (submitBlockReason(st) ?? "");
  };

  /** 초안 폴더는 첫 첨부(또는 보내기) 때 한 번만 만든다. 실패하면 다음 행위에서 다시 시도. */
  const ensureDraft = (): Promise<string> => {
    if (!draftP) {
      draftP = deps.invoke("feedback_draft_new").then(String);
      draftP.catch(() => {
        draftP = null;
      });
    }
    return draftP;
  };

  /** 첨부 작업은 한 줄로 세운다(칸 번호 배정·합계 상한이 어긋나지 않게 — Rust 잠금과 이중). */
  const enqueue = (label: string, job: () => Promise<void>) => {
    if (done || submitting) {
      setStatus("묶음을 이미 만들었습니다 — 새 첨부는 새 피드백으로 보내 주세요.", "error");
      return;
    }
    pending++;
    refresh();
    chain = chain.then(async () => {
      try {
        if (!closedFlag) await job();
      } catch (e) {
        setStatus(`${label} — ${errText(e)}`, "error");
      } finally {
        pending--;
        refresh();
      }
    });
  };

  const renderList = () => {
    list.textContent = "";
    for (const a of atts) {
      const li = document.createElement("li");
      const n = document.createElement("span");
      n.className = "fb-att-name";
      n.textContent = a.original;
      n.title = a.original;
      const s = document.createElement("span");
      s.className = "fb-att-size";
      s.textContent = `${kindLabel(a.kind)} · ${formatBytes(a.size)}`;
      const x = document.createElement("button");
      x.type = "button";
      x.className = "fb-att-x";
      x.textContent = "×";
      x.title = "첨부 빼기";
      x.setAttribute("aria-label", `${a.original} 빼기`);
      x.addEventListener("click", () =>
        enqueue("첨부 빼기", async () => {
          const id = await ensureDraft();
          await deps.invoke("feedback_detach", { id, name: a.name });
          atts = atts.filter((t) => t.name !== a.name);
          renderList();
          setStatus("");
        }),
      );
      li.append(n, s, x);
      list.appendChild(li);
    }
    const note = mailSizeNote(totalBytes());
    sizeNote.textContent = note ?? "";
    sizeNote.hidden = !note;
  };

  const attachPath = (path: string) => {
    const name = baseName(path);
    enqueue(name, async () => {
      if (!attachmentKind(path)) {
        const chk = validateAttachment(path, 1, { count: 0, total: 0 });
        throw new Error(chk.ok ? "첨부할 수 없는 파일입니다." : chk.reason);
      }
      if (atts.length >= FEEDBACK_MAX_FILES) throw new Error(`첨부는 ${FEEDBACK_MAX_FILES}개까지입니다.`);
      setStatus(`${name} 넣는 중…`);
      const id = await ensureDraft();
      const info = (await deps.invoke("feedback_attach_path", { id, path })) as Attachment;
      atts.push(info);
      renderList();
      setStatus(`${name} 넣었습니다.`, "ok");
    });
  };

  const attachFile = (file: File, original: string) => {
    enqueue(original, async () => {
      const chk = validateAttachment(original, file.size, { count: atts.length, total: totalBytes() });
      if (!chk.ok) throw new Error(chk.reason);
      const id = await ensureDraft();
      const slot = String(await deps.invoke("feedback_attach_begin", { id, original, size: file.size }));
      let ok = false;
      try {
        for (const [a, b] of chunkRanges(file.size, FEEDBACK_CHUNK_BYTES)) {
          if (closedFlag) throw new Error("창을 닫아 넣기를 멈췄습니다.");
          const bytes = new Uint8Array(await file.slice(a, b).arrayBuffer());
          await deps.invokeRaw("feedback_attach_chunk", bytes, {
            "x-fb-id": id,
            "x-fb-slot": slot,
            "x-fb-offset": String(a),
          });
          setStatus(`${original} 넣는 중… ${Math.floor((b / file.size) * 100)}%`);
        }
        const info = (await deps.invoke("feedback_attach_commit", {
          id,
          slot,
          size: file.size,
          original,
        })) as Attachment;
        atts.push(info);
        ok = true;
        renderList();
        setStatus(`${original} 넣었습니다.`, "ok");
      } finally {
        if (!ok) void deps.invoke("feedback_detach", { id, name: slot + ".part" }).catch(() => {});
      }
    });
  };

  const getFacts = (): Promise<FeedbackFacts> => {
    if (!factsP) {
      factsP = deps.facts().catch(() => ({
        user_agent: "",
        daemon: "확인 못 함",
        daemon_version: null,
        workspaces: 0,
        panes: 0,
      }));
    }
    return factsP;
  };

  const showConfirm = () => {
    confirmBox.hidden = false;
    q<HTMLButtonElement>(".fb-keep").focus();
  };
  const hideConfirm = () => {
    confirmBox.hidden = true;
    desc.focus();
  };

  const finish = () => {
    if (closedFlag) return;
    closedFlag = true;
    // 묶음을 만들기 전에 닫으면 초안(첨부 사본)을 지운다 — 줄 선 첨부가 끝난 뒤에(Rust 잠금과 이중).
    if (!done && draftP) {
      const p = draftP;
      chain = chain.then(async () => {
        try {
          const id = await p;
          await deps.invoke("feedback_discard", { id });
        } catch {
          /* 초안이 없거나 이미 지워졌다 — 완전 초기화 인벤토리가 마지막 그물 */
        }
      });
    }
    resolveClose();
  };

  const requestClose = () => {
    if (submitting) return; // 묶음을 쓰는 동안은 닫지 않는다(반쪽 묶음 방지 · 짧다)
    if (!confirmBox.hidden) {
      hideConfirm();
      return;
    }
    if (shouldConfirmDiscard({ desc: desc.value, attachments: atts.length, done })) {
      showConfirm();
      return;
    }
    finish();
  };

  const tryInvoke = async (cmd: string, args: Record<string, unknown>): Promise<string | null> => {
    try {
      await deps.invoke(cmd, args);
      return null;
    } catch (e) {
      return errText(e);
    }
  };

  const copyText = (s: string) => {
    const fallback = () => {
      // 임시 입력칸은 **창 안**에 둔다 — 창 밖이면 focusin 되찾기가 선택을 빼앗아 복사가 실패한다.
      const ta = document.createElement("textarea");
      ta.value = s;
      ta.className = "fb-copy-buf";
      box.appendChild(ta);
      ta.select();
      let ok = false;
      try {
        ok = document.execCommand("copy");
      } catch {
        ok = false;
      }
      ta.remove();
      closeBtn.focus();
      setStatus(ok ? `주소를 복사했습니다: ${s}` : `복사하지 못했습니다 — 주소를 직접 적어 주세요: ${s}`, ok ? "ok" : "error");
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(s).then(() => setStatus(`주소를 복사했습니다: ${s}`, "ok"), fallback);
    } else fallback();
  };

  const showDone = (rep: BundleReport, mailErr: string | null, folderErr: string | null) => {
    done = true;
    compose.hidden = true;
    composeBtns.hidden = true;
    doneView.hidden = false;
    doneBtns.hidden = false;
    const files = rep.attachments.length + (rep.include_diag ? 1 : 0);
    q<HTMLElement>(".fb-done-msg").textContent =
      `피드백 묶음을 만들었습니다 (번호 ${rep.id}).\n` +
      (mailErr
        ? `메일 앱을 열지 못했습니다(${mailErr}). 아래 주소로 직접 보내 주세요.`
        : "메일 앱에 받는 주소·제목·내용을 채워 열었습니다. 확인하시고 메일에서 [보내기]를 눌러 주세요.");
    q<HTMLElement>(".fb-done-addr").textContent = `받는 주소: ${rep.to}\n제목: ${rep.subject}`;
    const lines: string[] = [];
    if (files > 0) {
      const what: string[] = [];
      if (rep.attachments.length) what.push(`첨부 ${rep.attachments.length}개(${formatBytes(rep.total_bytes)})`);
      if (rep.include_diag) what.push("진단 파일 diag.json");
      lines.push(
        `${what.join(" · ")}는 ${folderErr ? "묶음 폴더" : "함께 열린 폴더"}(${rep.folder})에서 메일 창으로 끌어다 넣어 주세요.` +
          (folderErr ? ` 폴더를 열지 못했습니다(${folderErr}).` : ""),
      );
      const note = mailSizeNote(rep.total_bytes);
      if (note) lines.push(note);
    }
    lines.push("cys 는 자동으로 아무것도 보내지 않습니다 — 메일에서 직접 [보내기]를 누르실 때만 전달됩니다.");
    q<HTMLElement>(".fb-done-note").textContent = lines.join("\n");
    folderBtn.hidden = false;
    mailAppBtn.hidden = !(deps.platform === "mac" && rep.mail_app && files > 0);
    setStatus("");
    refresh();
    closeBtn.focus();
  };

  const submit = async () => {
    if (submitting || done || !canSubmit({ desc: desc.value, pending, submitting })) return;
    submitting = true;
    refresh();
    setStatus("묶음을 만드는 중…");
    try {
      const id = await ensureDraft();
      const includeDiag = diagChk.checked;
      const facts = includeDiag ? await getFacts() : null;
      const rep = (await deps.invoke("feedback_submit", {
        id,
        description: desc.value,
        includeDiag,
        facts,
        originals: atts.map((a) => ({ name: a.name, original: a.original })),
      })) as BundleReport;
      report = rep;
      // 폴더를 먼저, 메일 창을 나중에 연다 — 나중에 뜬 창이 앞에 와서 사용자가 곧장 메일을 본다.
      const hasFiles = rep.attachments.length > 0 || rep.include_diag;
      const folderErr = hasFiles ? await tryInvoke("feedback_reveal", { id: rep.id }) : null;
      const mailErr = await tryInvoke("feedback_open_mail", { id: rep.id });
      submitting = false;
      showDone(rep, mailErr, folderErr);
    } catch (e) {
      setStatus(`묶음을 만들지 못했습니다 — ${errText(e)}`, "error");
    } finally {
      submitting = false;
      refresh();
    }
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.isComposing || e.keyCode === 229) return; // IME 조합 중 Esc 는 조합 취소다
    if (e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    requestClose();
  };

  const onFocusIn = (e: FocusEvent) => {
    if (closedFlag) return;
    // 포커스가 모달 층 밖(뒤 pane 의 xterm 등)으로 나가면 즉시 되찾는다 — 키 입력이 PTY 로 새지 않게.
    if (shouldReclaimFocus(e.target)) (done ? closeBtn : desc).focus();
  };

  /** OS 드롭 구독 — 창이 떠 있는 동안만. 해제 전에 풀린 구독은 즉시 되돌린다. */
  const sub = (name: string, h: (e: { payload: unknown }) => void) => {
    deps
      .listen(name, h)
      .then((un) => {
        if (disposed) {
          try {
            un();
          } catch {
            /* 이미 해제됨 */
          }
        } else unlisteners.push(un);
      })
      .catch(() => {
        /* 구독 실패 — 파일 고르기·붙여넣기는 그대로 쓸 수 있다 */
      });
  };
  const unlistenAll = () => {
    disposed = true;
    for (const un of unlisteners.splice(0)) {
      try {
        un();
      } catch {
        /* 이미 해제됨 */
      }
    }
  };

  // ── 배선 ──
  desc.addEventListener("input", refresh);
  sendBtn.addEventListener("click", () => void submit());
  q<HTMLButtonElement>(".fb-cancel").addEventListener("click", requestClose);
  q<HTMLButtonElement>(".fb-x").addEventListener("click", requestClose);
  q<HTMLButtonElement>(".fb-keep").addEventListener("click", hideConfirm);
  q<HTMLButtonElement>(".fb-discard").addEventListener("click", finish);
  closeBtn.addEventListener("click", finish);
  ov.addEventListener("click", (e) => {
    if (e.target === ov) requestClose();
  });
  q<HTMLButtonElement>(".fb-pick").addEventListener("click", () => fileInput.click());
  fileInput.addEventListener("change", () => {
    for (const f of Array.from(fileInput.files ?? [])) attachFile(f, f.name);
    fileInput.value = "";
  });
  ov.addEventListener("paste", (e: ClipboardEvent) => {
    const imgs = Array.from(e.clipboardData?.items ?? []).filter(
      (it) => it.kind === "file" && feedbackImageExt(it.type) != null,
    );
    if (!imgs.length) return; // 글자 붙여넣기는 기본 동작(설명칸)
    e.preventDefault();
    for (const it of imgs) {
      const f = it.getAsFile();
      const ext = feedbackImageExt(it.type);
      if (f && ext) {
        pasteSeq++;
        attachFile(f, `붙여넣은 이미지 ${pasteSeq}.${ext}`);
      }
    }
  });
  diagView.addEventListener("click", async () => {
    if (!diagPre.hidden) {
      diagPre.hidden = true;
      diagView.textContent = "넣을 진단 내용 보기";
      return;
    }
    try {
      diagPre.textContent = String(await deps.invoke("feedback_diag_preview", { facts: await getFacts() }));
      diagPre.hidden = false;
      diagView.textContent = "진단 내용 접기";
    } catch (e) {
      setStatus(`진단 내용을 만들지 못했습니다 — ${errText(e)}`, "error");
    }
  });
  q<HTMLButtonElement>(".fb-copy").addEventListener("click", () => {
    if (report) copyText(report.to);
  });
  folderBtn.addEventListener("click", async () => {
    if (!report) return;
    const err = await tryInvoke("feedback_reveal", { id: report.id });
    setStatus(err ? `폴더를 열지 못했습니다 — ${err}` : "묶음 폴더를 열었습니다.", err ? "error" : "ok");
  });
  q<HTMLButtonElement>(".fb-remail").addEventListener("click", async () => {
    if (!report) return;
    const err = await tryInvoke("feedback_open_mail", { id: report.id });
    setStatus(err ? `메일 앱을 열지 못했습니다 — ${err}` : "메일 앱을 다시 열었습니다.", err ? "error" : "ok");
  });
  mailAppBtn.addEventListener("click", async () => {
    if (!report) return;
    const err = await tryInvoke("feedback_open_mail_app", { id: report.id });
    if (!err) {
      setStatus(`Mail 앱 새 메시지에 파일을 넣었습니다. 받는 사람(${report.to})과 제목을 적어 보내 주세요.`, "ok");
      return;
    }
    // 실패하면 폴더 열기로 떨어진다(설계 §3 U6).
    const ferr = await tryInvoke("feedback_reveal", { id: report.id });
    setStatus(
      `Mail 앱으로 열지 못했습니다(${err}). ` + (ferr ? `폴더도 열지 못했습니다(${ferr}).` : "묶음 폴더를 열었습니다."),
      "error",
    );
  });

  try {
    window.addEventListener("keydown", onKey, true);
    document.addEventListener("focusin", onFocusIn, true);
    document.body.appendChild(ov);
    sub("tauri://drag-enter", () => drop.classList.add("over"));
    sub("tauri://drag-leave", () => drop.classList.remove("over"));
    sub("tauri://drag-drop", (e) => {
      drop.classList.remove("over");
      if (closedFlag) return;
      // 좌표는 보지 않는다 — 창이 떠 있으면 어디에 놓든 첨부다(윈도우 드롭 좌표 오프셋 무관).
      const p = (e.payload ?? {}) as { paths?: unknown };
      const paths = Array.isArray(p.paths) ? p.paths.filter((x): x is string => typeof x === "string") : [];
      for (const path of paths) attachPath(path);
    });
    refresh();
    desc.focus();
    await closed;
  } finally {
    window.removeEventListener("keydown", onKey, true);
    document.removeEventListener("focusin", onFocusIn, true);
    unlistenAll();
    ov.remove();
    // 포커스 복귀는 한 박자 뒤 — 창을 닫은 그 키(Enter·Space)의 남은 이벤트가 pane 으로 가지 않게.
    // 그사이 다른 모달이 떴다면 setFocus 가드가 xterm 포커스를 막는다.
    setTimeout(() => {
      try {
        deps.restoreFocus();
      } catch {
        /* 포커스 복귀는 편의 — 실패해도 창은 닫혔다 */
      }
    }, 0);
  }
}
