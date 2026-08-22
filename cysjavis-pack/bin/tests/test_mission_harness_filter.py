#!/usr/bin/env python3
"""test_mission_harness_filter.py — 임무 게이트 층0(harness 내부 알림) 회귀 핀.

## 재현 대상 (2026-08-22 06:30:06 실사고)
부서 레인 임무 대장이 harness 가 합성한 백그라운드 작업 완료 알림으로 **덮였다** —

    {"mission": "<task-notification> <task-id>bacbyhtv8</task-id> <tool-use-id>…</tool-use-id>
                 <output-file>…</output-file> <status>completed</status>
                 <summary>Background command … completed (exit code 0)</summary>
                 </task-notification>",
     "source": "prompt", "reason": "잔여문 395자 — 오너 임무로 인정"}

즉 **오너의 진짜 임무를 기계 산출물이 덮었다**. 층1(배달 원장 대조)은 데몬이 pane stdin 에
주입한 것만 원장에 남기므로 harness 가 프로세스 **내부에서** 합성한 알림을 볼 수 없고,
층2(라벨)도 `[` 로 시작하지 않아 못 잡는다 — 그래서 층0(병렬 축)이 필요했다.

## 판정 계약 (2026-08-22 master 판정 — 규칙은 **잔여문 하나**다)
마커 블록(여는 태그~닫는 태그 · **본문 포함**)을 걷어낸 잔여문이 최소 길이 미만이면 기계다.
초안에 있던 '시작부 지배'(첫 글자부터 마커 태그면 기계) 규칙은 **기각·제거**됐다 — harness 는
마커를 오너 문장 **앞에** 붙이는 것이 평시 동작이라(선행 `<system-reminder>` · 슬래시 명령
`<command-name>` 블록 뒤의 "계속하라"), 그 규칙은 **이 사고의 거울상**(기계가 오너 임무를 못
들어오게 막음)을 만든다. 아래 6·7번 핀이 그 회귀를 잡는다.

## 이 파일이 못박는 것 (밀폐: CYS_STATE_DIR 격리 · 실 데몬·실 사용자 대장 무접촉)
  1) 실측 사고 문자열 → `mission=null` · `source=harness_notification` · 게이트 무개방
  2) 정상 오너 임무 → 그대로 임무로 기록(과잉 차단 회귀 0)
  3) 마커 **단순 언급**(태그 아님)·오너 임무 뒤에 붙은 리마인더 → 통과
  4) 진행 중 오너 임무를 harness 알림이 **덮지 않는다**(반대 방향 사고 차단)
  5) 다른 harness 알림 형태(리마인더·캐비앳·명령 연쇄) → 접힘
  6) ★마커가 오너 문장 **앞**·**앞뒤 동시**여도 → 통과 + 잔여문이 정확히 오너 문장
  7) ★닫는 태그 없는 **잘린 알림** → 접힘(미종결 폴백) · 짝 맞는 블록은 과잉 절단 없음
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
MISSION = os.path.join(SELF, "..", "javis_mission.py")
PY = sys.executable or "python3"

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


# ★실측 사고 문자열(요약·재작성 금지 — 재현 검체의 가치는 원문 형태에 있다)
REAL = ("<task-notification> <task-id>bacbyhtv8</task-id> "
        "<tool-use-id>toolu_01ABCdefGHIjklMNOpqrs</tool-use-id> "
        "<output-file>/tmp/claude-501/bg-out.txt</output-file> "
        "<status>completed</status> "
        "<summary>Background command `python3 javis_orchestra.py check` completed "
        "(exit code 0)</summary> </task-notification>")


def make_env(tmp):
    env = dict(os.environ)
    env["CYS_STATE_DIR"] = os.path.join(tmp, "state")     # 밀폐 — 실 사용자 대장 무접촉
    env["CYS_SURFACE_ID"] = "7"
    env.pop("CYS_MISSION", None)                          # env 임무는 게이트를 열어 버린다
    env.pop("AITERM_SURFACE_ID", None)
    return env


def record(env, prompt):
    """훅과 같은 소비면 — stdin=UserPromptSubmit hook JSON. (exit, stderr)"""
    r = subprocess.run([PY, MISSION, "record"], input=json.dumps({"prompt": prompt},
                                                                 ensure_ascii=False),
                       capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
    return r.returncode, r.stderr


def ledger(env):
    r = subprocess.run([PY, MISSION, "path"], capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=30)
    p = (r.stdout or "").strip()
    if not p or not os.path.isfile(p):
        return None, p
    with open(p, encoding="utf-8") as f:
        return json.load(f), p


def status(env):
    r = subprocess.run([PY, MISSION, "status"], capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=30)
    return r.returncode


# ── 1. 실측 사고 문자열 → mission=null · 판정 근거 기록 · 게이트 무개방 ──
tmp = tempfile.mkdtemp(prefix="mission-h1-")
env = make_env(tmp)
record(env, REAL)
rec, path = ledger(env)
check("1a 대장 생성(판정 근거를 남긴다)", isinstance(rec, dict), "path=%s" % path)
check("1b ★mission=null(기계 산출이 임무가 되지 않는다)",
      isinstance(rec, dict) and rec.get("mission") is None,
      "mission=%r" % (rec or {}).get("mission"))
check("1c source=harness_notification(판정 근거가 대장에 남는다)",
      (rec or {}).get("source") == "harness_notification", "source=%r" % (rec or {}).get("source"))
check("1d reason 비어 있지 않음", bool((rec or {}).get("reason")))
check("1e 게이트 무개방(자율 착수 금지)", status(env) != 0)
shutil.rmtree(tmp)

# ── 2. 정상 오너 임무는 그대로 통과 ──
tmp = tempfile.mkdtemp(prefix="mission-h2-")
env = make_env(tmp)
record(env, "부서 문서 정리 착수해줘")
rec, _ = ledger(env)
check("2a 오너 평문 임무 기록", (rec or {}).get("mission") == "부서 문서 정리 착수해줘",
      "mission=%r" % (rec or {}).get("mission"))
check("2b source=prompt", (rec or {}).get("source") == "prompt")
shutil.rmtree(tmp)

# ── 3. 마커 단순 언급·후행 리마인더는 통과(과잉 차단 금지) ──
#    ★판정만 본다(임무로 인정되는가) — 층0 은 **접느냐 마느냐**의 축이지 임무 문자열을 다듬는
#      정규화기가 아니다. 후행 리마인더가 mission 문자열에 남는 것은 종전과 같은 거동이며,
#      오너 문장이 **앞머리에 보존**되는지만 못박는다(잘려 나가면 그것은 회귀다).
for i, (prompt, head, why) in enumerate((
    ("task-notification 이 왜 임무로 기록되는지 조사해줘",
     "task-notification 이 왜 임무로 기록되는지 조사해줘", "태그 아닌 단순 언급"),
    ("system-reminder 훅 동작을 정리해서 보고해라",
     "system-reminder 훅 동작을 정리해서 보고해라", "태그 아닌 단순 언급"),
    ("릴리스 노트 초안을 만들어줘\n<system-reminder>Memory contents …</system-reminder>",
     "릴리스 노트 초안을 만들어줘", "오너 임무 뒤 harness 리마인더(평시 경로)"),
), start=1):
    tmp = tempfile.mkdtemp(prefix="mission-h3-%d-" % i)
    env = make_env(tmp)
    record(env, prompt)
    rec, _ = ledger(env)
    got = (rec or {}).get("mission")
    check("3%d 통과(%s)" % (i, why),
          isinstance(got, str) and got.startswith(head), "mission=%r head=%r" % (got, head))
    shutil.rmtree(tmp)

# ── 4. 진행 중 오너 임무를 harness 알림이 덮지 않는다(2026-08-22 사고의 본체) ──
tmp = tempfile.mkdtemp(prefix="mission-h4-")
env = make_env(tmp)
record(env, "Wave2 릴리스 노트 초안 만들어줘")
rec0, _ = ledger(env)
check("4a 선행 오너 임무 기록", (rec0 or {}).get("mission") == "Wave2 릴리스 노트 초안 만들어줘")
record(env, REAL)
rec1, _ = ledger(env)
check("4b ★오너 임무 무덮어쓰기(사고 본체 재현 0)",
      (rec1 or {}).get("mission") == "Wave2 릴리스 노트 초안 만들어줘",
      "mission=%r" % (rec1 or {}).get("mission"))
shutil.rmtree(tmp)

# ── 5. 마커 corpus — 다른 harness 알림 형태도 전부 접힌다 ──
for i, (prompt, why) in enumerate((
    ("<system-reminder>Codebase instructions …</system-reminder>", "시스템 리마인더 단독"),
    ("<local-command-caveat>Caveat: 아래는 슬래시 명령 출력입니다</local-command-caveat>",
     "슬래시 명령 캐비앳"),
    ("<command-name>/clear</command-name><command-message>clear</command-message>",
     "명령 알림 연쇄"),
), start=1):
    tmp = tempfile.mkdtemp(prefix="mission-h5-%d-" % i)
    env = make_env(tmp)
    record(env, prompt)
    rec, _ = ledger(env)
    check("5%d 접힘(%s)" % (i, why), (rec or {}).get("mission") is None,
          "mission=%r" % (rec or {}).get("mission"))
    shutil.rmtree(tmp)

# ── 6. ★마커가 오너 문장 **앞**·**앞뒤 동시**여도 통과 + 잔여문 = 오너 문장 ──
#    (2026-08-22 master 반려 사유 그대로의 회귀 핀 — '시작부 지배' 규칙을 되살리면 즉시 FAIL)
for i, (prompt, want, why) in enumerate((
    ("<system-reminder>Codebase instructions …</system-reminder>"
     "부서 플로우 6결함을 고치고 배포하라.",
     "부서 플로우 6결함을 고치고 배포하라.", "선행 리마인더 + 오너 임무"),
    ("<command-name>/model</command-name>\n<command-message>model</command-message>\n"
     "<command-args></command-args>\n계속하라",
     "계속하라", "슬래시 명령 블록 뒤의 오너 지시(실측 형태)"),
    ("<system-reminder>Memory …</system-reminder>\n부서 플로우 6결함을 고치고 배포하라.\n"
     "<task-notification><status>completed</status></task-notification>",
     "부서 플로우 6결함을 고치고 배포하라.", "선행 + 후행 동시"),
), start=1):
    tmp = tempfile.mkdtemp(prefix="mission-h6-%d-" % i)
    env = make_env(tmp)
    record(env, prompt)
    rec, _ = ledger(env)
    got = (rec or {}).get("mission")
    check("6%d-a ★임무로 통과(%s)" % (i, why), got is not None,
          "mission=%r · 접히면 사고의 거울상이다" % got)
    # 잔여문(=오너 문장) 정확 일치는 판정 소유자의 순수 함수로 대조한다(사본 금지)
    r = subprocess.run(
        [PY, "-c",
         "import sys; sys.path.insert(0, %r); import javis_mission as m; "
         "sys.stdout.write(m.strip_harness_blocks(sys.stdin.read()))"
         % os.path.abspath(os.path.join(SELF, "..")),
        ], input=prompt, capture_output=True, text=True, encoding="utf-8", timeout=30)
    check("6%d-b 잔여문 = 오너 문장" % i, (r.stdout or "") == want,
          "residual=%r want=%r" % (r.stdout, want))
    shutil.rmtree(tmp)

# ── 7. ★닫는 태그 없는 잘린 알림 → 접힘(미종결 폴백) + 사유 흔적 ──
TRUNC = ("<task-notification> <task-id>bacbyhtv8</task-id> "
         "<tool-use-id>toolu_01ABCdefGHIjklMNOpqrs</tool-use-id> "
         "<summary>Background command")
tmp = tempfile.mkdtemp(prefix="mission-h7-")
env = make_env(tmp)
record(env, TRUNC)
rec, _ = ledger(env)
check("7a ★잘린 알림도 접힘(미종결 폴백)", (rec or {}).get("mission") is None,
      "mission=%r" % (rec or {}).get("mission"))
check("7b 폴백 적용 사실이 사유에 남는다(감사 가능성)",
      "미종결" in ((rec or {}).get("reason") or ""), "reason=%r" % (rec or {}).get("reason"))
shutil.rmtree(tmp)

# ── 8. ★적대검증 치명① — 미종결 폴백이 오너 지시를 삼키는가 (리뷰어 재현 입력 그대로) ──
#    초판 폴백은 여는 태그부터 **문자열 끝까지** 잘라 '왜'·'##'·'>' 만 남기고 오너 지시를
#    통째로 삼켰다. record 훅 e2e 로도 record-exit=1(임무 미기록·게이트 폐쇄)이 확인됐다.
for i, (prompt, why) in enumerate((
    ("왜 <summary> 때문에 임무가 안 잡혀? 원인 찾아서 고쳐라", "접두 1자 + 미종결 summary"),
    ("이 <summary> 태그 버그 전부 고치고 배포해라", "접두 1자 + 미종결 summary"),
    ("## <command-name> 처리 로직 전면 재작성해라", "접두 2자(구두점) + 미종결 알림 마커"),
    ("> <task-id> 필드 추가하고 릴리스해라", "접두 1자(구두점) + 미종결 컨텍스트 마커"),
), start=1):
    tmp = tempfile.mkdtemp(prefix="mission-h8-%d-" % i)
    env = make_env(tmp)
    rc, _err = record(env, prompt)
    rec, _ = ledger(env)
    got = (rec or {}).get("mission")
    check("8%d ★오너 지시 무삼킴(%s)" % (i, why), got is not None,
          "mission=%r record-exit=%d — 치명① 재발" % (got, rc))
    shutil.rmtree(tmp)

# ── 9. ★적대검증 치명② — 슬래시 명령 합성 turn 이 그대로 통과하는가 ──
#    `local-command-caveat` 는 있는데 같은 가족의 `local-command-stdout` 이 없어, `/cost` 형태가
#    "source":"prompt" 로 기록되고 게이트가 열렸다(2026-08-22 사고 원문과 동형 재현).
COST = ("<command-name>/cost</command-name>\n<command-message>cost</command-message>\n"
        "<command-args></command-args>\n<local-command-stdout>Total cost: $12.34\n"
        "Total duration (API): 4m 5.6s\nTotal duration (wall): 12m 3.4s\n"
        "Total code changes: 120 lines added, 8 lines removed\n"
        "Usage by model:\n    claude-opus: 1.2k input, 3.4k output\n</local-command-stdout>")
# 목록에 **없는** 태그 — 이름 축으로는 못 잡고 이름 독립 구조 축이 잡아야 한다.
UNKNOWN_TAG = ("<command-name>/newthing</command-name>\n"
               "<brand-new-harness-tag>목록에 없는 미래 harness 태그 출력</brand-new-harness-tag>")
for i, (prompt, why) in enumerate(((COST, "★/cost 실측 형태"),
                                   (UNKNOWN_TAG, "★목록에 없는 새 태그(구조 축)")), start=1):
    tmp = tempfile.mkdtemp(prefix="mission-h9-%d-" % i)
    env = make_env(tmp)
    record(env, prompt)
    rec, _ = ledger(env)
    check("9%d-a ★기계로 접힘(%s)" % (i, why), (rec or {}).get("mission") is None,
          "mission=%r — 층0 을 넣고도 사고가 살아 있다" % str((rec or {}).get("mission"))[:60])
    check("9%d-b 게이트 무개방" % i, status(env) != 0)
    shutil.rmtree(tmp)

# ── 10. ★구조 축이 오너를 삼키지 않는가 (master 지적 함정 1·2) ──
for i, (prompt, why) in enumerate((
    ("이 컴포넌트 고쳐줘\n```jsx\n<div><span>hi</span></div>\n```", "코드펜스 + 지시"),
    ("```html\n<html><body><h1>Hi</h1></body></html>\n```", "★펜스만 — 보호 구간"),
    ("이 XML 파싱 버그 고쳐줘: <root><item>1</item></root>", "XML 붙여넣기 + 지시"),
    ("<details><summary>접기</summary></details> 이 마크다운 렌더링 고쳐줘", "일반 HTML + 지시"),
), start=1):
    tmp = tempfile.mkdtemp(prefix="mission-h10-%d-" % i)
    env = make_env(tmp)
    record(env, prompt)
    rec, _ = ledger(env)
    check("10%d 오너 통과(%s)" % (i, why), (rec or {}).get("mission") is not None,
          "mission=%r — 구조 축 과잉 차단" % (rec or {}).get("mission"))
    shutil.rmtree(tmp)

# ── 11. ★적대검증 ⑤ — 훅 지연(성능). 매 프롬프트가 이만큼 느려진다. ──
import time as _time
PERF = [("미종결 마커 10,000개", "<summary>" * 10000),
        ("미종결 일반 태그 10,000개", "<x-tag>" * 10000),
        ("짝맞는 블록 5,000개", "<x-tag>a</x-tag>" * 5000)]
r = subprocess.run(
    [PY, "-c",
     "import sys,time,json; sys.path.insert(0,%r); import javis_mission as m;\n"
     "cases=json.load(sys.stdin)\n"
     "print(json.dumps([[n, round(( (lambda t0: (m.harness_origin(s), time.time()-t0)[1])(time.time()) ),4)] for n,s in cases]))"
     % os.path.abspath(os.path.join(SELF, "..")),
    ], input=json.dumps(PERF), capture_output=True, text=True, encoding="utf-8", timeout=120)
try:
    timings = json.loads(r.stdout)
except Exception:
    timings = []
check("11a 성능 측정 성공", bool(timings), r.stderr[-200:])
for name, secs in timings:
    check("11b %s < 0.5s" % name, secs < 0.5, "%.4fs" % secs)

print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
