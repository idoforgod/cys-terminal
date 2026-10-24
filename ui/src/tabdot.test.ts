// ★WP11-TABDOT (0.14.53 · owner order 2026-10-23) — pane title dot:
//   WORKING = filled circle that BLINKS · IDLE (waiting) = HOLLOW circle, steady · DEAD = grey X, steady.
//   Every other state (input · stale · unknown · unobserved) has its own steady shape; colour is never the only cue.
// Namespace import on purpose: on the base tree (no deadReason / no dead view) each test fails on its own assertion
// instead of the whole file failing to link — that is how the RED-on-base run reads per-requirement results.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import * as RA from "./roleactivity";

// comments stripped first: a comment right before a rule would otherwise be read as part of its selector list
const CSS = readFileSync(new URL("./style.css", import.meta.url), "utf-8").replace(/\/\*[\s\S]*?\*\//g, "");
const MEDIA = "@media (prefers-reduced-motion: reduce)";
/** the dot rules live before the first reduced-motion block (style.css order) */
const PLAIN = CSS.slice(0, CSS.indexOf(MEDIA));
const VIEWS = ["working", "waiting", "dead", "input", "stale", "unknown", "unobserved"] as const;
const DOT_CLASS = RA.DOT_CLASS as Readonly<Record<string, string>>;

/** Declarations of every rule (outside media blocks) whose selector list contains exactly `sel`, in source order. */
function decls(sel: string, src = PLAIN): string {
  const out: string[] = [];
  for (const m of src.matchAll(/([^{}]+)\{([^}]*)\}/g)) {
    const sels = m[1].split(",").map((s) => s.trim().replace(/\s+/g, " "));
    if (sels.includes(sel)) out.push(m[2].trim());
  }
  return out.join(" ");
}
const dot = (suffix = ""): string => decls(`.pane .pane-role-dot${suffix}`);
/** The whole look of a view: its own rule plus its ::before/::after strokes. */
const look = (view: string): string => {
  const c = DOT_CLASS[view];
  return c === undefined ? "" : [dot(`.${c}`), dot(`.${c}::before`), dot(`.${c}::after`)].join(" | ");
};

describe("WP11 owner looks — the three states the owner named", () => {
  it("WORKING = a FILLED circle that BLINKS", () => {
    const base = dot();
    expect(base).toContain("border-radius: 50%");
    expect(base).toContain("background: var(--role-color");
    const w = look("working");
    expect(w).toContain("animation: cc-blink");
    expect(w).not.toContain("transparent"); // keeps the base fill
    expect(/@keyframes cc-blink \{[^}]*opacity/.test(CSS)).toBe(true);
  });
  it("IDLE (waiting) = a HOLLOW circle, steady", () => {
    const w = look("waiting");
    expect(w).toContain("background: transparent");
    expect(w).toContain("box-shadow: inset 0 0 0 1.5px var(--role-color");
    expect(w).not.toContain("animation");
    expect(w).not.toContain("dashed");
    expect(w).not.toContain("opacity");
  });
  it("DEAD = a grey X, steady, not in the role colour", () => {
    const d = look("dead");
    expect(dot(`.${DOT_CLASS.dead}`)).toContain("background: transparent");
    const before = dot(`.${DOT_CLASS.dead}::before`);
    const after = dot(`.${DOT_CLASS.dead}::after`);
    for (const stroke of [before, after]) {
      expect(stroke).toContain('content: ""');
      expect(stroke).toContain("background: #8b949e");
      expect(stroke).toContain("position: absolute");
    }
    expect(before).toContain("rotate(45deg)");
    expect(after).toContain("rotate(-45deg)");
    expect(d).not.toContain("--role-color");
    expect(d).not.toContain("animation");
  });
  it("Windows high contrast (forced colors) cannot erase the shapes: the dot opts out of forced colours", () => {
    // forced colors drop box-shadow and repaint background-color — without this the filled dot, the ring and the X disappear
    expect(dot()).toContain("forced-color-adjust: none");
  });
  it("only WORKING animates — anywhere in the stylesheet, for any dot class", () => {
    for (const v of VIEWS) {
      const c = DOT_CLASS[v];
      expect({ v, mapped: typeof c === "string" }).toEqual({ v, mapped: true });
      expect({ v, animates: look(v).includes("animation") }).toEqual({ v, animates: v === "working" });
    }
    expect(dot()).not.toContain("animation");
  });
});

describe("WP11 every state is distinguishable — by shape, not only by colour", () => {
  it("DOT_CLASS maps all seven views to seven different classes", () => {
    const classes = VIEWS.map((v) => DOT_CLASS[v]);
    expect(classes.every((c) => typeof c === "string" && c.startsWith("act-"))).toBe(true);
    expect(new Set(classes).size).toBe(VIEWS.length);
  });
  it("the seven looks are pairwise different even with every colour removed", () => {
    const shapeOnly = (s: string): string =>
      s.replace(/var\(--role-color[^)]*\)/g, "C").replace(/#[0-9a-fA-F]{3,8}\b/g, "C").replace(/\s+/g, " ").trim();
    const sigs = VIEWS.map((v) => shapeOnly(look(v)));
    for (let i = 0; i < VIEWS.length; i++)
      for (let j = i + 1; j < VIEWS.length; j++) expect({ a: VIEWS[i], b: VIEWS[j], same: sigs[i] === sigs[j] }).toEqual({ a: VIEWS[i], b: VIEWS[j], same: false });
  });
  it("the other states keep their own shapes: input diamond · stale dim · unknown dashed ring · unobserved bar", () => {
    expect(look("input")).toContain("rotate(45deg)");
    expect(look("stale")).toContain("opacity: .4");
    expect(look("unknown")).toContain("border: 1.5px dashed var(--role-color");
    expect(look("unobserved")).toContain("height: 2px");
  });
  it("reduced motion: WORKING stops blinking but keeps a static outer ring that the hollow IDLE dot does not have", () => {
    const blocks = CSS.split(MEDIA).slice(1);
    const mine = blocks.find((b) => b.includes(`.pane .pane-role-dot.${DOT_CLASS.working}`)) ?? "";
    const body = mine.slice(mine.indexOf("{") + 1, mine.indexOf("\n}")); // inside the @media braces only
    const w = decls(`.pane .pane-role-dot.${DOT_CLASS.working}`, body);
    expect(w).toContain("animation: none");
    expect(w).toContain("box-shadow: 0 0 0 1.5px var(--bar), 0 0 0 3px var(--role-color");
    expect(look("waiting")).not.toContain("box-shadow: 0 0 0"); // idle's ring is inset only
  });
});

describe("WP11 state inputs — dead predicate and the no-grammar CLI", () => {
  const deadReason = (RA as unknown as Record<string, unknown>).deadReason as ((r: Record<string, unknown>) => string | null) | undefined;
  it("deadReason: exited === true or agent_alive === false; null/absent/true and non-boolean values are not dead", () => {
    expect(typeof deadReason).toBe("function");
    const f = deadReason as (r: Record<string, unknown>) => string | null;
    expect(f({ exited: true })).toBe("창 종료(exited)");
    expect(f({ exited: true, agent_alive: false })).toBe("창 종료(exited)");
    expect(f({ exited: false, agent_alive: false })).toBe("에이전트 프로세스 없음");
    for (const r of [{}, { exited: false }, { agent_alive: null }, { agent_alive: true }, { exited: "true" }, { agent_alive: 0 }, { agent_alive: "false" }])
      expect({ r, dead: f(r) }).toEqual({ r, dead: null });
  });
  const at = (wire: Record<string, unknown>) => {
    const s = new RA.RoleActivityStore(31_000);
    expect(s.accept("k", 1, 0, 5, wire)).toBe("accepted");
    return RA.projectActivity(s.get("k"), 1_000, 31_000);
  };
  // the exact display_activity measured on the live dept-5 daemon for gemini seat 72 and claude seat 77 (MEASURE.md, probe1.jsonl)
  const LIVE = (over: Record<string, unknown>): Record<string, unknown> => ({
    consecutive_rejections: 0, display_epoch: "ef2ae0917babc52c", family: "claude", grammar_revision: "claude-2.1.301-r1",
    last_positive_age_secs: null, observation_seq: 20767, observed_age_secs: 0.526, raw: "unknown", reason: "incomplete frame",
    retained: false, stable_age_secs: 20551.521, state: "unknown", ...over,
  });
  it("a CLI the daemon has no grammar for (reason 'unsupported family', gemini today) gets its own 'unobserved' view", () => {
    expect((RA as unknown as Record<string, unknown>).UNSUPPORTED_FAMILY_REASON).toBe("unsupported family");
    const p = at(LIVE({ family: "gemini", grammar_revision: null, reason: "unsupported family" }));
    expect(p.view).toBe("unobserved");
    expect(p.text).toContain("활동 관측 미지원");
    expect(p.text).toContain("gemini");
  });
  it("the measured live claude value (unknown · incomplete frame) stays 'unknown' — the UI shows what the daemon sends", () => {
    expect(at(LIVE({})).view).toBe("unknown");
    expect(at(LIVE({ state: "working", raw: "active", reason: "active frame", last_positive_age_secs: 0.5 })).view).toBe("working");
    expect(at(LIVE({ state: "waiting", raw: "idle", reason: "idle frame" })).view).toBe("waiting");
  });
});
