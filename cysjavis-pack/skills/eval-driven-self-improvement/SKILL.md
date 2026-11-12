---
name: eval-driven-self-improvement
description: Use when iteratively improving any artifact (site, codebase, doc, model output) over multiple rounds and you must keep ONLY genuine verified gains — especially recursive self-improvement (RSI) where the same agent both makes the changes and judges them, where self-scoring risks fooling yourself, metric gaming, reward-hacking, content-deletion-for-score, or never knowing when to stop.
---

# Eval-Driven Self-Improvement (RSI loop)

## Overview
Iteratively improve an artifact and keep only changes a **locked, objective, adversarially game-proofed evaluation** confirms are real. Core principle: **when the same agent improves AND judges, you WILL fool yourself unless the judge is enforced by code and run independently — self-discipline ("I won't edit the judge") is not enforcement.**

A capable agent already reaches the *obvious* scaffolding on its own (freeze a rubric as code, use objective tools, branch per round, pre-commit a keep-rule, reset against the kept baseline, stop at plateau). **This skill is the non-obvious 40% that even a smart agent misses** — the anti-self-fooling, anti-reward-hacking enforcement. If you're not doing the 7 rules below, your loop is gameable.

## When to Use
- Any "improve X and prove each round is really better" loop, run many times.
- Especially when **producer = evaluator** (you write changes and score them).
- Symptoms it's needed: "I'll review and judge if it's better" · self-reported scores · a rubric you *could* edit mid-loop · metrics that go up while quality doesn't · score rose because content/features were deleted.

**Not for:** one-off changes (just review once); subjective work with no measurable proxy (the loop has nothing to lock — escalate to a human judge instead).

## The loop
1. **Build the eval** = hard gates (pass/fail) + a composite score (0..1). Lock it.
2. **Adversarially verify the eval** (Rule 2) → **LOCK** with cryptographic pins (Rule 1).
3. **Baseline**: run the locked eval N≥3× → median + spread (noise floor).
4. **Round**: producer changes ONLY the artifact → **trust-gate** (Rule 3) → evaluator runs the locked eval → **keep-or-discard** (Rule 7).
5. At ceiling → **evolve the eval** (Rule 7), don't pad or stop blindly.

## Mechanical enforcement — rsi-gate + EFEC 일가 (2026-06-07 RSI ×Karpathy 업그레이드)
The 7 rules below are now **mechanically enforced**, not merely disciplined. Before any auto-Elevate, pipe the change descriptor (JSON) to **`_round/autopilot/rsi-gate.sh`** (stdin JSON → exit 0 = auto-Elevate licensed / exit 2 = demote to proposal + stderr reason). It is the **Tier-2 enforcer** of the **Eval-First Experiment Contract (EFEC)** — full contract in `RSI_PROTOCOL.md §4.2~4.2.9`:
- **EFEC** (§4.2): cheap **mechanical** eval = the autonomy license; expensive **linguistic** review (gemini/codex) is a scarce resource reserved for the unmechanizable. 4 requirements: pre-fixed eval · bounded change-space · rollback · independent evaluator. Karpathy thesis — *"verification is cheap, search is expensive"* → mechanical verification is what licenses untrusted-producer autonomy.
- **Non-code eval taxonomy E0–E5** (§4.2.1): structural lint (E0) alone never licenses; require behavioral/replay (E1/E2) **+ mutation-kill (E3)**. Eval must measure *observable behavior*, not the improvement claim. Unmeasurable wisdom → honest Human-Critical proposal.
- **Bounded Change-Space + 불변식3** (§4.2.2): per-layer [Auto]/[Human] whitelist (default-deny); ★**no loop modifies its own evaluator / rollback / denylist in the same loop** (meta-circular safety = the deepest reward-hack vector; rsi-gate DENYs these targets unconditionally).
- **Diversity-Locked Verification** (§4.2.3): producer ≠ evaluator must be **different model families** (DLE); evaluator **independently re-derives** then checksum-compares (IR) — extends Rule 1/2.
- **Adversarial Metric Integrity** (§4.2.5): Sentinel-Pareto (no regression in pytest/SOUL_SMOKE/store-clean), hidden held-out (Rule 6/env-lock), **meta-mutation of the eval itself** (Rule 2 applied to the judge).
- **Unified Autonomous Boundary** (§4.2.6): RSI Auto-Elevate ⊆ autopilot denylist (Absolute Hard-Stop: 외부발행·비가역·헌법). RSI may ratchet conservatism **up** (human-approved), never down.

Implementation of the enforcer is **Human-Critical** (R9 §4.2.8); once built, the RSI loop itself never edits rsi-gate/guard/hooks (the loop cannot weaken its own gate) — fixing a broken hook is ordinary self-repair work through the repair flow (`cys repair`), never an RSI round. guard.sh = command irreversibility (kill-switch); rsi-gate = RSI-change auto-Elevate license — two defense lines.

## The 7 hard-won rules (the part agents skip)

1. **Producer ≠ evaluator, enforced by CODE not discipline.** SHA256-pin every eval file in a manifest; the manifest pins its own digest. An **evaluator-owned launcher** extracts the eval from a *locked git ref* (ground truth), independently verifies the candidate's protected paths byte-match, and only then runs — so a tampered candidate copy of the judge can't grade itself. The orchestrator holds 3 pins (locked ref · manifest digest · launcher sha256) and re-derives them. "I froze the rubric and won't touch it" is the self-trust hole — it does not hold under pressure.

2. **Adversarially verify the EVAL before locking.** An *independent* agent tries to break the eval itself, hunting reward-hack holes: metric-failure → silent exclusion, self-verifying mutable judge, environment not gated, one-axis-sacrifice via arithmetic mean, content-deletion paths. Eval authors cannot see their own holes. A bare "looks good (YES)" is not verification — demand file:line findings. On reviewer disagreement, decide by independent re-derivation, not majority/average.

3. **Trust-gate before EVERY scoring.** Before running the eval on a candidate, verify: (a) launcher/eval hashes == pinned, (b) baseline reference unchanged, (c) candidate touched only the artifact, not the eval. **Never trust the producer's self-reported score** — re-run under the locked launcher and use that number.

4. **Measurement failure = hard fail, never silent exclusion.** Retry once for flake; if a metric stays unmeasurable, that's a FAIL, not "drop it from the average." Excluding an unmeasurable metric hands the producer a free pass to break the hard-to-optimize metric.

5. **Lock a content/function-retention hard gate.** Size/perf/budget metrics reward deleting content. Pin critical-content retention (token/sentence/asset/section floors, rendered-text presence) as a HARD gate, or "improvement" becomes deletion.

6. **Environment lock.** Pin Node/Chrome/OS (or equivalent). A candidate measured in a different environment is not comparable — mismatch invalidates the round.

7. **Keep-rule = close a fraction of the residual gap; at ceiling, evolve don't pad.** Keep iff: all hard gates pass AND `C_new ≥ C_old + g·(1−C_old)` (e.g. g=0.30) AND no axis regresses > ε AND gain > baseline noise. Composite = **weighted geometric mean** (arithmetic mean lets one maxed axis hide a tanked one). At ceiling (C_old ≥ ~0.98): **stop claiming gains; evolve the eval by adding metrics with GENUINE headroom** (verify the artifact actually scores low on them — adding already-maxed metrics is padding/gaming), as an explicit approved meta-step. This eval-evolution is the deeper recursion; it requires re-verify + re-lock.

## Rationalization table
| Excuse | Reality |
|--------|---------|
| "I froze the rubric, I won't edit it" | Self-trust = the hole. Enforce with pins + an evaluator-owned launcher (Rule 1). |
| "My gate run already passed, keep it" | You ran the candidate's copy of the judge. Re-run under the locked launcher (Rule 3). |
| "The metric couldn't be measured, skip it" | Skip = reward-hack. Measurement failure is a hard fail (Rule 4). |
| "Score went up, ship it" | Up via deletion? Retention hard gate must pass first (Rule 5). |
| "One reviewer said it's fine" | Bare YES ≠ verification. Demand file:line; re-derive on disagreement (Rule 2). |
| "We're at ceiling, just keep grinding" | Grinding a maxed metric = gaming. Evolve the eval with real-headroom metrics (Rule 7). |
| "Add metrics to keep the loop going" | Only metrics the artifact scores LOW on. Padding with maxed metrics is fake headroom. |
| "벤더가 GIFT-Eval 1위라니 그 수치 인용" | 자가보고치(벤더/외부 리더보드)는 본 코드 재현 아님. **self-reported vs hold-out 재현을 분리표기**하고 '1위'는 시점 라벨 필수(예: TimesFM-2.5 2025-09 1위 → 2026 Chronos-2 추월). keep-rule 근거는 **재현 수치만** [P12]. |

## Red flags — STOP
- 외부 모델(TimesFM 등) 성능을 벤더/리더보드 **자가보고치**로 인용하는데 '본 코드 재현(hold-out)' 라벨·시점 라벨이 없다 — self-reported와 재현 수치를 분리표기하고, 재현 수치만 keep-rule 근거로 쓴다 [P12] (TimesFM 사례: `_research/_timesfm_impl/DUEL_FINDINGS.md`).
- You're about to accept a score the producer computed.
- The judge code is editable in the same tree as the changes, with no pin check.
- A metric is being averaged-out / excluded because it's null.
- A round "improved" while content/features shrank.
- Nobody adversarially tried to break the eval before locking.
- At ceiling, you're adding weight to metrics that are already ~1.0.

## Real-world impact
On a static site RSI run: a single reviewer rubber-stamped the locked eval ("YES"), but an independent adversarial reviewer found **5 reward-hack holes** (null-exclusion, self-trusting SHA, un-gated env, weak audit, CWV null) — each at file:line. After hardening + re-lock, R1 produced a trust-gated, master-run gain of **C 0.8947 → 0.9888** (a11y 0.78→1.0, visual 0.77→1.0) with content retention provably unchanged — then honestly hit the ceiling after one round, triggering eval-evolution rather than fake grinding.


## Tournament & termination procedure (W2-7 · OMC self-improve 클린룸 흡수 · 2026-07-06)

1. **Worktree 토너먼트**: 개선 시도가 N개 접근으로 갈릴 때, 각 producer는 **각자 git worktree**에서 병렬 실행한다(approach family를 서로 다르게 강제 — 같은 계열 N개는 토너먼트가 아니다). 채점은 전원 동일 LOCKED eval로.
2. **승자 merge 후 재benchmark 회귀검증**: 승자 병합 직후 **같은 eval을 한 번 더** 실행한다 — 병합 자체가 회귀를 만들 수 있다. 회귀 검출 시 즉시 롤백(reset --hard 급 원복)하고 병합을 무효로 한다. "병합 전 점수"를 결과로 보고하는 것은 금지.
3. **종료조건 구분 — plateau vs circuit-breaker**: plateau 카운터는 **정체된 승리**(개선폭 < ε인 PASS)만 세고, circuit-breaker는 **실패**만 센다(공통상수 3 — 3연속 실패=변형 반복 금지·아키텍처 의심·에스컬레이션). 두 카운터를 섞으면 "실패 연속"이 plateau로 위장되어 낭비 루프가 은폐된다. 종료 사유는 반드시 둘 중 무엇인지 명시 보고한다.
