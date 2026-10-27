// WP13-TABTITLE-0.14.53 (owner fix F1) — tab title = [surface number] + title + duplicate-role mark.
//
// Owner's ask (OWNER-20261024-seven-fixes.txt §1, verbatim): 「탭 제목 앞에 번호 표시. 예: [381] reviewer-codex-codex · cys」
// and 「같은 역할 좌석이 둘 이상이면 탭에 중복 표시」. Two seats of the same role showed the identical title, so the
// owner could not tell which window to close (10/24: surfaces 375 and 381, both "reviewer-codex-codex · cys").
//
// ★Display only. The stored daemon title is NOT touched: `cys list`/`cys status` columns, `recall.lines.title` and the
// `isAutoTitle` auto-title rule all keep reading exactly what they read at f0ac3b0c (DESIGN.md §2.1 has the four
// measured reasons). The agent's OSC 0/2 title and the tab-dot grammar are a different field entirely and are untouched.
//
// Two layers, the same reason the *wiring* tests in this folder exist: ① the pure module, ② a source pin on main.ts, so a
// variant that keeps the module correct while never passing the surface id (or writing the prefix into the stored title)
// is still red.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import { isAutoTitle, surfaceTag, dupRoleCounts, paneTitle } from "./panetitle";

const MAIN = readFileSync(new URL("./main.ts", import.meta.url), "utf8");

describe("WP13 ① surfaceTag — the number, and nothing when there is no usable number", () => {
  it("prefixes the surface number in the owner's shape", () => {
    expect(surfaceTag(381)).toBe("[381] ");
    expect(paneTitle(381, "reviewer-codex-codex · cys")).toBe("[381] reviewer-codex-codex · cys");
  });
  it("is total: no throw and no tag for a missing or unusable id", () => {
    // ★the only fatal risk this change can carry is a throw inside the 3 s refresh loop (all panes blank),
    //   so every one of these must return a string.
    for (const bad of [null, undefined, NaN, Infinity, -Infinity, -1, 1.5, "381", {}, []] as unknown[]) {
      expect(typeof surfaceTag(bad as never)).toBe("string");
      expect(surfaceTag(bad as never)).toBe("");
      expect(typeof paneTitle(bad as never, "t")).toBe("string");
    }
    expect(paneTitle(null as never, "reviewer-codex-codex · cys")).toBe("reviewer-codex-codex · cys");
  });
  it("keeps a huge but real id", () => {
    expect(surfaceTag(999999)).toBe("[999999] ");
    expect(surfaceTag(0)).toBe("[0] ");
  });
});

describe("WP13 ② the auto-title rule is unchanged", () => {
  it("still treats an empty or 'surface N' title as automatic", () => {
    expect(isAutoTitle(null)).toBe(true);
    expect(isAutoTitle(undefined)).toBe(true);
    expect(isAutoTitle("")).toBe(true);
    expect(isAutoTitle("surface 42")).toBe(true);
    expect(isAutoTitle("worker-claude · cys")).toBe(false);
    expect(isAutoTitle("[42] surface 42")).toBe(false); // the exact reason the prefix is display-only
  });
  it("an auto-titled pane shows the number and the live cwd", () => {
    expect(paneTitle(42, "surface 42", "/Users/x/dev")).toBe("[42] /Users/x/dev");
    expect(paneTitle(42, "", null)).toBe("[42] …");
  });
});

describe("WP13 ③ duplicate-role mark — ASCII ' (n)', per socket, live seats only", () => {
  const live = (surface_id: number, role: string | null, exited = false) => ({ surface_id, role, exited });

  it("counts live seats per role", () => {
    const c = dupRoleCounts([live(375, "reviewer-codex"), live(381, "reviewer-codex"), live(7, "master")]);
    expect(c.get("reviewer-codex")).toBe(2);
    expect(c.get("master")).toBe(1);
  });
  it("marks both duplicates and leaves a unique role unmarked", () => {
    const c = dupRoleCounts([live(375, "reviewer-codex"), live(381, "reviewer-codex"), live(7, "master")]);
    expect(paneTitle(381, "reviewer-codex-codex · cys", null, c.get("reviewer-codex"))).toBe(
      "[381] reviewer-codex-codex · cys (2)",
    );
    expect(paneTitle(375, "reviewer-codex-codex · cys", null, c.get("reviewer-codex"))).toBe(
      "[375] reviewer-codex-codex · cys (2)",
    );
    expect(paneTitle(7, "master-claude · cys", null, c.get("master"))).toBe("[7] master-claude · cys");
  });
  it("excludes exited seats — a dead twin is not a duplicate", () => {
    const c = dupRoleCounts([live(375, "reviewer-codex", true), live(381, "reviewer-codex")]);
    expect(c.get("reviewer-codex")).toBe(1);
    expect(paneTitle(381, "reviewer-codex-codex · cys", null, c.get("reviewer-codex"))).toBe(
      "[381] reviewer-codex-codex · cys",
    );
  });
  it("ignores role-less shells", () => {
    const c = dupRoleCounts([live(1, null), live(2, null), live(3, "")]);
    expect(c.size).toBe(0);
  });
  it("is total against a broken list", () => {
    expect(dupRoleCounts(undefined as never).size).toBe(0);
    expect(dupRoleCounts(null as never).size).toBe(0);
    expect(dupRoleCounts([null, undefined, {}] as never).size).toBe(0);
  });
  it("marks nothing for a count of 0, 1 or a broken count", () => {
    for (const n of [undefined, null, 0, 1, NaN, -3, "2"] as unknown[]) {
      expect(paneTitle(5, "t", null, n as never)).toBe("[5] t");
    }
  });
  it("ASCII only — no non-ASCII glyph (Windows rendering is unmeasured in this lane)", () => {
    const out = paneTitle(381, "reviewer-codex-codex · cys", null, 2);
    const added = out.replace("reviewer-codex-codex · cys", "");
    expect(/^[\x20-\x7e]*$/.test(added)).toBe(true);
  });
});

describe("WP13 ④ main.ts wiring — the display path really passes the number (source pin)", () => {
  it("imports the display module instead of defining the old formatter inline", () => {
    expect(MAIN).toContain('from "./panetitle"');
    expect(MAIN).not.toContain("const isAutoTitle = (t: string | null | undefined)");
    expect(MAIN).not.toContain("const paneTitle = (title: string | null | undefined");
  });
  it("passes the surface id at the refresh write and at pane creation", () => {
    expect(MAIN).toContain("paneTitle(s.surface_id, s.title, s.live_cwd");
    expect(MAIN).toContain("paneTitle(sid, title)");
  });
  it("computes the duplicate counts from the per-socket surface list", () => {
    expect(MAIN).toContain("dupRoleCounts(r.surfaces)");
  });
  it("still skips the write while the title is being edited in place", () => {
    expect(MAIN).toContain("if (rt.titleEl.isContentEditable) continue;");
  });
  it("does not send the prefix back to the daemon (display only)", () => {
    // the rename call must carry the user's text, never a rendered title
    expect(MAIN).toContain('invoke("rename_surface", { socket, surfaceId: sid, title: name })');
    expect(MAIN).not.toContain("surfaceTag(sid) + name");
  });
  it("★R1: the in-place edit starts from the STORED title, not from the rendered one", () => {
    // Found by R1, not by the first green run: `commit()` reads `titleEl.textContent`, and this change made that
    // text a RENDERED string ("[381] foo (2)"). Committing without retyping would therefore save the prefix and
    // the duplicate mark INTO the daemon's stored title, and the next render would show "[381] [381] foo (2) (2)" —
    // the display-only promise broken through the edit path. So both write sites keep the raw title beside the node
    // and the edit seeds the editable text from it.
    expect(MAIN).toContain('titleEl.dataset.stored = title ?? ""');          // pane creation
    expect(MAIN).toContain('rt.titleEl.dataset.stored = s.title ?? ""');     // the 3 s refresh
    expect(MAIN).toContain('titleEl.textContent = titleEl.dataset.stored ?? ""'); // entering edit mode
  });
});

import * as P7Titles from "./panetitle";
describe("P7 metadata evidence", () => {
  it("separates launch and observed paths without attributing an account mismatch", () => {
    expect(typeof P7Titles.paneDetails).toBe("function");
    const out=P7Titles.paneDetails({title:"my title",cwd:"/fixture/old",effective_cwd:"/fixture/new",account:{state:"mismatch",account_id:"wrong",profile:"observed-profile",recorded_profile:"recorded-profile"}});
    expect(out).toContain("/fixture/old");expect(out).toContain("/fixture/new");expect(out).not.toContain("wrong");
    expect(out).toContain("observed-profile");expect(out).toContain("recorded-profile");
    expect(P7Titles.paneDetails({cwd:"/fixture/old",effective_cwd:null})).toContain("관측된 셸 경로: 확인 불가");
    for (const bad of [null,undefined,42,[],{}]) expect(typeof P7Titles.paneDetails(bad)).toBe("string");
    expect(MAIN).toContain("rt.titleEl.title = paneDetails(s)");
  });
});
