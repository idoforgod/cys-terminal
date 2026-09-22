// 파일 트리→pane 경로 주입의 순수 판단 로직 (DOM 무접촉 — main.ts의 드롭·메뉴 핸들러가 배선만 한다).
//
// 형식 결정(D3): 에이전트 등록 pane(role/agent 필드 존재)에는 Claude 네이티브 `@경로` 멘션,
// 미등록 pane에는 셸 인용 경로(iTerm2 관행). 자동 Return은 어느 쪽에도 없다 — 전송은 사람 몫.

import { shellQuote, shellQuoteJoin } from "./shellquote";

// 경로 분해 — 트리 경로는 "/" 결합이지만 live_cwd 유래 경로(Windows)는 "\"일 수 있어 양쪽 인식.
export function splitPath(p: string): { parent: string; name: string } {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  if (i < 0) return { parent: "", name: p };
  let parent = p.slice(0, i);
  if (parent === "") parent = "/"; // "/file"
  if (/^[A-Za-z]:$/.test(parent)) parent += "\\"; // "C:\file" → 드라이브 루트
  return { parent, name: p.slice(i + 1) };
}

export const baseName = (p: string): string => splitPath(p).name;

// 부모 폴더 조회 실패(권한·IO)와 정상 조회 뒤 항목 부재를 구분한다.
export type PathCheck = "ok" | "unreadable" | "missing";

export function classifyPathCheck(entries: { name: string }[] | null, name: string): PathCheck {
  if (entries === null) return "unreadable";
  return entries.some((entry) => entry.name === name) ? "ok" : "missing";
}

export function pathCheckToast(
  kind: Exclude<PathCheck, "ok">,
  p: string,
  idx: number,
  total: number,
  err?: string,
): { name: string; detail: string } {
  const prefix = total > 1 ? `(${idx + 1}/${total}) ` : "";
  return kind === "unreadable"
    ? { name: "경로 확인 실패", detail: `${prefix}${p} — 폴더를 읽을 수 없음${err ? `: ${err}` : ""}` }
    : { name: "경로가 더 이상 없음", detail: `${prefix}${p} — 트리를 새로고침합니다` };
}

// cwd 기준 상대화 — cwd 바깥 경로는 절대경로 유지(../ 사슬은 가독성·정확성을 해친다).
export function relativize(abs: string, cwd: string | null): string {
  if (!cwd) return abs;
  if (abs === cwd) return ".";
  const base = cwd.endsWith("/") ? cwd : cwd + "/";
  return abs.startsWith(base) ? abs.slice(base.length) : abs;
}

// 주입 텍스트. 에이전트 pane=@멘션(공백 포함 경로는 멘션이 끊기므로 셸 인용 폴백),
// 그 외=셸 인용. 끝에 공백 1개(연속 삽입 구분), 개행 없음.
export function insertionText(
  paths: string[],
  opts: { agent: boolean; isWin: boolean; cwd: string | null },
): string {
  if (opts.agent) {
    return (
      paths
        .map((p) => {
          const rel = relativize(p, opts.cwd);
          return /\s/.test(rel) ? shellQuote(p, opts.isWin) : `@${rel}`;
        })
        .join(" ") + " "
    );
  }
  return shellQuoteJoin(paths, opts.isWin) + " ";
}

// 스트리밍 판정 — 마지막 PTY 출력이 threshold 안이면 에이전트 응답 중으로 본다(주입 확인 게이트).
export function isStreaming(lastOutputAt: number, now: number, thresholdMs = 3000): boolean {
  return lastOutputAt > 0 && now - lastOutputAt < thresholdMs;
}
