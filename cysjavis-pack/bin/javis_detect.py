#!/usr/bin/env python3
"""javis_detect.py — 마스터 선언 감지기 **단일 소유** (T-0147-7 W1b · 재감사 §3 CS-8 · RC8 소멸).

문제(RC8): '마스터 선언인가'라는 의도 분류기가 role-bootstrap.sh 안에서 **순서의존 셸 grep
스택**으로 인라인 구현돼 있었다(구 :46-56). 그 구조 자체가 4개 결함의 공통 원인이었다:

  · **A4(P1)** 억제(의문·인용)를 선언 감지보다 **먼저** 돌려서, 선언과 무관한 위치의 물음표
    하나가 정당한 선언을 삼켰다 — "너는 마스터다. 오늘 뭐부터 할까?"가 미발화(실측 재현).
    셸 grep 은 매치 **스팬**을 모르므로 '선언이 속한 절'이라는 의미 경계를 만들 수 없다.
  · **P3-A-NEGA(P2)** 주어 어휘에 `네가/니가/당신이`가 없어 표준 정서법 선언이 전부 미발화.
  · **G9(P3)** `grep -E '.{0,15}'` 는 로케일이 C면 **바이트**를 세어 한글 filler 창이 1/3로
    축소 — LC_ALL=C 환경에서 선언 미탐.
  · **G25(P3)** `cut -c1-200` 은 GNU 에서 **바이트** 단위라 Windows/C 로케일에서 감지창이
    한글 약 66자로 축소.

해법(CS-8): 감지+억제 전체를 **이 파일의 단일 함수**(`detect`)로 이관한다. 파이썬 `re` 는
스팬을 주므로 순서를 역전(선언 먼저 → 그 선언이 **속한 절** 내부의 억제 마커만 평가)할 수
있고, 문자열 슬라이스가 문자 단위이며 로케일에 의존하지 않는다. 셸측 슬라이스(`cut -c`·
bashism `${x:0:200}`)는 훅에서 **제거**된다 — 그게 G25 의 소멸 방식이다.

★거리 튜닝 금지: 억제 판정은 "선언 매치와 마커 사이 N자"가 아니라 **절 경계 포함 여부**다.
  거리 상수는 재감사가 명시적으로 기각한 처방이다(A4 재검증: "문자거리 N보다 '선언 감지 후
  절 경계 스코프 억제'").

수치 스펙(§3 CS-8④ — 상수화·경계 corpus 박제 대상):
  WINDOW_CHARS=200(감지창 · **문자**) · FILLER_MAX=15(주어↔마스터 사이 filler)
  TERM_GAP_MAX=2(마스터↔종결어미 간격) · NEG_GAP_MAX=3(부정 인접 간격)

사용:
  · 라이브러리(권장 — corpus 회귀는 이 함수를 직접 때린다):
        import javis_detect; v = javis_detect.detect(prompt); v["fire"]
  · 훅 게이트(1왕복 · stdin=UserPromptSubmit hook JSON):
        printf '%s' "$INPUT" | python3 javis_detect.py hook-gate
        exit 0=FIRE / 1=judged-no(선언 없음·침묵) / 3=선언 검출 후 **억제**(stderr 1줄 로그)
        / 2=cannot-judge(입력 파싱 불가 — 호출측이 loud 분기)
    stdout 은 항상 1줄 verdict JSON 이다(훅은 `$(...)`로 회수 — 모델 컨텍스트로 새지 않는다).
  · `--self-test`: 밀폐 corpus 배터리(assert) — preflight/CI 관례.

★exit 1 과 3 의 분리가 계약의 핵심이다: '판정했고 아니다'(침묵 정당)와 '선언이었는데 억제'
  (로그 1줄 의무 — CS-8⑤ "억제 시 침묵 대신 로그 1줄")를 융합하면 왜 안 떴는지 영영 모른다.
"""
import json
import re
import sys

# ★로케일 비의존 I/O(G9 · 선례 javis_bootstrap.py R3/D-IMPL-3): LC_ALL=C·Windows cp949 파이프에서
#   한글 출력이 UnicodeEncodeError 로 크래시하면 '판정 불가'가 아니라 **훅 전체가 깨진다**. 입력은
#   아래 `hook-gate` 가 **바이트로 읽어 UTF-8 명시 디코드**하므로 stdin 인코딩 추론에도 의존하지 않는다.
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

# ── 수치 스펙(단일 진실원천 — 문서·주석·훅이 이 상수를 가리킨다) ──────────────
WINDOW_CHARS = 200   # 감지창: 프롬프트 앞 200 **문자**(긴 문서 본문 오발화 억제)
FILLER_MAX = 15      # 주어↔'마스터' 사이 허용 filler 문자수("너는 **지금부터** 마스터다")
TERM_GAP_MAX = 2     # '마스터'↔종결어미 사이 허용 간격("마스터**가 **되")
NEG_GAP_MAX = 3      # 부정 인접 억제의 비-한글 간격 허용치

# 절(문장) 경계 문자 — 이 문자에서 절이 끝난다(경계 문자는 **앞 절에 포함**한다:
# "…무슨 뜻?" 의 '?' 가 그 절의 억제 마커로 평가돼야 한다).
CLAUSE_BOUNDARY = ".!?;…。！？\n\r"

# ── 어휘 ─────────────────────────────────────────────────────────────────────
# 주어(P3-A-NEGA): 표준 정서법 `네가`·구어 `니가`·`당신이`를 추가한다.
# ★과확장 금지: `네`·`당신` **단독**은 넣지 않는다("네 마스터 브랜치"·"당신 마스터키" 오발화).
#   교착사(는/가/이)까지 포함한 형태만 주어로 인정한다. 단독 `너`는 구 계약 보존(맨 뒤 —
#   정규식 대안은 앞에서부터 시도되므로 긴 형태가 먼저 매치된다).
SUBJECT = r"(?:너는|넌|너가|네가|니가|당신은|당신이|너)"
MASTER = r"(?:마스터|master)"
# 종결: 서술격·명령형·'로 각성/승격'·'가 되/돼/된'
TERM = r"(?:다|야|이다|입니다|임|이야|여|로 *각성|로 *승격|가 *되|가 *돼|가 *된)"

DECL_KO = re.compile(SUBJECT + r".{0,%d}" % FILLER_MAX + MASTER
                     + r".{0,%d}" % TERM_GAP_MAX + TERM, re.IGNORECASE)
DECL_EN = re.compile(r"you\s+are\s+(?:the\s+|our\s+|now\s+)*master", re.IGNORECASE)

# 부정 인접 억제(adv#7): 선언 자리 자체가 부정인 경우 — "너는 마스터가 아니다/말고".
NEG = re.compile(MASTER + r"[^가-힣A-Za-z]{0,%d}(?:가|는|를)?[^가-힣A-Za-z]{0,%d}"
                 r"(?:아니|아냐|말고)" % (NEG_GAP_MAX, NEG_GAP_MAX), re.IGNORECASE)

# 의문·인용 억제 마커(adv#8): "'너는 마스터다'가 무슨 뜻?" / "…라고 말하지 마".
# ★구 셸 grep 과 **동일 어휘**를 유지한다(억제 범위만 절로 좁혔다 — 어휘 확장은 별 결함).
QUESTION = re.compile(r"(?:무슨|무엇|뜻|의미|가 뭐|가 무|\?|라고 (?:말|하지|입력)|처럼|예시|예를)")


def _flatten(text):
    """개행·탭 → 공백 **1:1 치환**(길이·오프셋 보존).

    구 훅은 `tr '\\n' ' '` 로 개행을 없앤 뒤 grep 했다. 여기서도 매칭은 평탄화 텍스트에서
    하되(선언이 줄바꿈을 걸쳐도 감지 — 구 동작 보존), **절 경계 계산은 원문**에서 한다
    (개행은 절 경계다). 두 문자열의 인덱스가 1:1로 정렬돼야 하므로 길이를 바꾸지 않는다.
    """
    return text.replace("\r", " ").replace("\n", " ").replace("\t", " ")


def _clause_bounds(raw, start, end):
    """`raw` 에서 [start,end) 매치를 **완전히 포함하는 절**의 (lo, hi) 반환.

    lo = 매치 시작 왼쪽의 가장 가까운 경계 문자 다음 / hi = 매치 끝 오른쪽의 가장 가까운
    경계 문자 **포함**(없으면 문자열 끝). 매치 전체를 감싸는 이유: filler 안에 마침표가
    들어간 병리 입력("너는. 마스터다'가 무슨 뜻?")에서 절이 매치를 갈라 억제 마커를
    놓치는 것을 막는다(fail-closed 방향).
    """
    lo = 0
    for i in range(start - 1, -1, -1):
        if raw[i] in CLAUSE_BOUNDARY:
            lo = i + 1
            break
    hi = len(raw)
    for i in range(end, len(raw)):
        if raw[i] in CLAUSE_BOUNDARY:
            hi = i + 1
            break
    return lo, hi


def _matches(flat):
    """선언 후보 전량(한국어·영문)을 시작 오프셋 순으로 반환."""
    found = [(m.start(), m.end(), m.group(0), "ko") for m in DECL_KO.finditer(flat)]
    found += [(m.start(), m.end(), m.group(0), "en") for m in DECL_EN.finditer(flat)]
    found.sort(key=lambda t: (t[0], t[1]))
    return found


def detect(prompt, window_chars=WINDOW_CHARS):
    """마스터 선언 판정(순수 함수 · 로케일 비의존 · 부작용 0).

    반환 dict:
      fire(bool) · reason(str) · verdict('fire'|'no_declaration'|'suppressed')
      matched(선언 매치 문자열 또는 None) · marker(억제 마커 또는 None)
      clause(억제 판정에 쓰인 절) · span([start,end] 또는 None)
      window_chars · filler_max · term_gap_max · truncated(창 절단 여부)

    ★후보 여러 개면 **하나라도 억제되지 않은 선언**이 있으면 FIRE 한다(단조성):
      "'너는 마스터다'가 무슨 뜻? 아무튼 너는 마스터다." 는 발화가 정답이다.
    """
    text = prompt if isinstance(prompt, str) else ""
    raw = text[:window_chars]               # ★문자 단위 슬라이스(G25) — 바이트 아님
    flat = _flatten(raw)
    spec = {"window_chars": window_chars, "filler_max": FILLER_MAX,
            "term_gap_max": TERM_GAP_MAX, "truncated": len(text) > len(raw)}
    cands = _matches(flat)
    if not cands:
        return dict(spec, fire=False, verdict="no_declaration",
                    reason="감지창(%d자) 내 마스터 선언 없음" % window_chars,
                    matched=None, marker=None, clause=None, span=None)
    last = None
    for start, end, matched, lang in cands:
        lo, hi = _clause_bounds(raw, start, end)
        clause = flat[lo:hi]
        neg = NEG.search(clause)
        if neg:
            last = dict(spec, fire=False, verdict="suppressed",
                        reason="선언 인접 부정 — 절 내 부정 표현 %r" % neg.group(0),
                        matched=matched, marker=neg.group(0), clause=clause,
                        span=[start, end])
            continue
        q = QUESTION.search(clause)
        if q:
            last = dict(spec, fire=False, verdict="suppressed",
                        reason="선언이 속한 절 내부의 의문·인용 마커 %r — 선언 아님(질의/인용)"
                               % q.group(0),
                        matched=matched, marker=q.group(0), clause=clause,
                        span=[start, end])
            continue
        return dict(spec, fire=True, verdict="fire",
                    reason="마스터 선언 확정(%s · 절 내 억제 마커 없음)" % lang,
                    matched=matched, marker=None, clause=clause, span=[start, end])
    return last


# ══════════════════════════════════════════════════════════════════════════════
# CLI — 훅 게이트(1왕복)
# ══════════════════════════════════════════════════════════════════════════════
EXIT_FIRE = 0
EXIT_NO_DECL = 1        # judged-no — 침묵이 정당(로그 없음)
EXIT_CANNOT_JUDGE = 2   # 입력 파싱 불가 — 호출측 loud 분기(CS-2⑨)
EXIT_SUPPRESSED = 3     # 선언 검출 후 억제 — **stderr 1줄 로그 의무**(CS-8⑤)


def cmd_hook_gate(argv):
    """stdin(UserPromptSubmit hook JSON) → verdict. 프롬프트 추출도 여기서 한다(왕복 1회)."""
    try:
        # ★바이트로 읽고 UTF-8 명시 디코드: LC_ALL=C 환경에서 sys.stdin 이 ASCII 로 추론돼
        #   한글 프롬프트가 UnicodeDecodeError 로 죽는 것을 원천 차단한다(G9 로케일 비의존).
        raw = sys.stdin.buffer.read() if hasattr(sys.stdin, "buffer") else sys.stdin.read()
        payload = raw.decode("utf-8", "replace") if isinstance(raw, bytes) else raw
    except Exception as e:                      # stdin 소실 — 판정 불가
        sys.stderr.write("[detect] cannot-judge: stdin 읽기 실패(%s)\n" % e)
        print(json.dumps({"fire": False, "verdict": "cannot_judge",
                          "reason": "stdin 읽기 실패"}, ensure_ascii=False))
        return EXIT_CANNOT_JUDGE
    if not (payload or "").strip():
        print(json.dumps({"fire": False, "verdict": "no_declaration",
                          "reason": "빈 stdin"}, ensure_ascii=False))
        return EXIT_NO_DECL
    try:
        obj = json.loads(payload)
        prompt = obj.get("prompt", "") if isinstance(obj, dict) else ""
    except Exception as e:
        sys.stderr.write("[detect] cannot-judge: hook JSON 파싱 실패(%s) — 선언 감지 불가\n" % e)
        print(json.dumps({"fire": False, "verdict": "cannot_judge",
                          "reason": "hook JSON 파싱 실패: %s" % e}, ensure_ascii=False))
        return EXIT_CANNOT_JUDGE
    if not isinstance(prompt, str) or not prompt:
        print(json.dumps({"fire": False, "verdict": "no_declaration",
                          "reason": "prompt 필드 부재/빈값"}, ensure_ascii=False))
        return EXIT_NO_DECL
    v = detect(prompt)
    print(json.dumps(v, ensure_ascii=False))
    if v["fire"]:
        return EXIT_FIRE
    if v["verdict"] == "suppressed":
        # ★CS-8⑤: 억제는 침묵하지 않는다 — 왜 안 떴는지 1줄로 남긴다.
        sys.stderr.write("[detect] SKIP(억제): %s | 선언=%r 마커=%r\n"
                         % (v["reason"], v["matched"], v["marker"]))
        return EXIT_SUPPRESSED
    return EXIT_NO_DECL


# ── corpus 배터리(밀폐 self-test) ────────────────────────────────────────────
# ★producer≠evaluator 경계: 이 배터리는 **정규식 작성자가 역산한 케이스가 아니라** 재감사가
#   실측으로 확정한 결함 재현 케이스(A4·P3-A-NEGA·P3-A-FILLER·G9·G25)와 기존 회귀 corpus
#   (test_role_bootstrap_hook.py 원본 15건)를 그대로 옮긴 것이다. 확장 corpus 는 오너 실발화
#   채집으로 늘린다(§3 CS-8⑤).
FIRE_CORPUS = [
    # ── 기존 corpus(구 훅 시대부터 통과해야 했던 것 — 전량 보존) ──
    "너는 마스터다", "너는 이제 마스터다", "너는 지금부터 마스터다", "너가 마스터야",
    "당신은 우리의 마스터입니다", "너는 마스터로 각성하라", "you are the master",
    "지금부터 너는 마스터가 된다",
    # ── A4: 혼합 의도(선언 + 후속 질문) — 절 경계 스코프의 핵심 케이스 ──
    "너는 마스터다. 오늘 뭐부터 할까?",
    "너는 이제 마스터다! 무슨 일부터 시작할까?",
    "너는 마스터다.\n오늘 작업 목록을 뭐로 잡을까?",
    # ── P3-A-NEGA: 표준 정서법·구어 주어 ──
    "네가 마스터다", "니가 마스터다", "당신이 마스터다", "네가 이제 마스터야",
    # ── 억제 마커가 **다른 절**에 있으면 발화 ──
    "'설계 문서'를 예시로 보여줘. 그리고 너는 마스터다.",
]
SKIP_CORPUS = [
    # ── 기존 corpus ──
    "너는 마스터가 아니다", "'너는 마스터다'가 무슨 뜻이야?", "너는 마스터다라고 말하지 마",
    "오늘 작업 지시해줘", "너는 워커다", "마스터 브랜치를 확인해줘", "너는 오늘 마스터 브랜치 봐",
    # ── A4 반대편: 같은 절 안의 의문·인용은 여전히 억제 ──
    "'네가 마스터다'가 무슨 뜻?", "\"당신이 마스터다\"라고 입력하면 어떻게 되나요?",
    "너는 마스터다 처럼 들리는 문장을 만들어줘",
    # ── P3-A-NEGA 과확장 금지: 주어 아닌 `네`·`당신` 단독 ──
    "네 마스터 브랜치를 봐줘", "당신 마스터키 어디 뒀어",
]


def cmd_self_test():
    fails = []
    for p in FIRE_CORPUS:
        v = detect(p)
        if not v["fire"]:
            fails.append("FALSE-NEGATIVE %r → %s" % (p, v["reason"]))
    for p in SKIP_CORPUS:
        v = detect(p)
        if v["fire"]:
            fails.append("FALSE-POSITIVE %r → %s" % (p, v["reason"]))
    # 경계 스펙 — filler 15=발화 / 16=미발화(P3-A-FILLER: 주석 12 ≠ 코드 15 불일치 해소본)
    if not detect("너는" + "가" * FILLER_MAX + "마스터다")["fire"]:
        fails.append("filler %d자 경계에서 미발화" % FILLER_MAX)
    if detect("너는" + "가" * (FILLER_MAX + 1) + "마스터다")["fire"]:
        fails.append("filler %d자에서 발화(창 초과 오발화)" % (FILLER_MAX + 1))
    # 감지창 — 한글 **문자** 200자 기준(G25: 바이트면 약 66자에서 잘린다)
    pad = "가" * (WINDOW_CHARS - len("너는 마스터다"))
    if not detect(pad + "너는 마스터다")["fire"]:
        fails.append("감지창 끝(문자 %d) 선언 미발화 — 바이트 슬라이스 회귀 의심" % WINDOW_CHARS)
    if detect("가" * WINDOW_CHARS + "너는 마스터다")["fire"]:
        fails.append("감지창 밖 선언이 발화(창 미적용)")
    # 억제/미검출 분리(exit 1 vs 3)
    if detect("'너는 마스터다'가 무슨 뜻?")["verdict"] != "suppressed":
        fails.append("인용·의문 케이스가 suppressed 로 분류되지 않음")
    if detect("오늘 작업 지시해줘")["verdict"] != "no_declaration":
        fails.append("무선언 케이스가 no_declaration 으로 분류되지 않음")
    if fails:
        print("javis_detect self-test FAIL (%d):" % len(fails), file=sys.stderr)
        for f in fails:
            print("  -", f, file=sys.stderr)
        return 1
    print("javis_detect self-test OK (FIRE %d · SKIP %d · filler 15/16 경계 · 감지창 %d자 경계 · "
          "억제/미검출 분리)" % (len(FIRE_CORPUS), len(SKIP_CORPUS), WINDOW_CHARS))
    return 0


def main(argv):
    if "--self-test" in argv:
        return cmd_self_test()
    cmd = argv[1] if len(argv) > 1 else "hook-gate"
    if cmd == "hook-gate":
        return cmd_hook_gate(argv[2:])
    if cmd == "text":                     # 진단용: 인자 텍스트 1건 판정(개발자 편의)
        v = detect(" ".join(argv[2:]))
        print(json.dumps(v, ensure_ascii=False, indent=1))
        return EXIT_FIRE if v["fire"] else EXIT_NO_DECL
    sys.stderr.write("usage: javis_detect.py [hook-gate|text <문장>|--self-test]\n"
                     "  hook-gate: stdin=UserPromptSubmit hook JSON → "
                     "exit 0=FIRE 1=선언없음 2=판정불가 3=억제\n")
    return 64  # EX_USAGE — 미지 서브커맨드는 거부(fail-closed · A14 동형)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
