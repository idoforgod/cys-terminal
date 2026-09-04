#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_learn_digest_queue.py — RSI 학습 자율추천이 feed 를 쏘지 않고 다이제스트 큐에만 쌓이는가 (0.14.30 · 오너 승인 2026-09-04).

무엇을 막는가: 종전에는 **라운드 종료마다**(그리고 eval ceiling 마다) `cys feed push --kind
learn_proposal` 로 건별 승인 요청이 자동 발행됐다. 승인권은 오너뿐인데 자동 생성분 6건(최고령
19h48m)이 적체했고, 그 소음이 **실제 승인 요청**(SSH 프로브 9.5h)을 덮었다.
RSI_LEARNING_DIRECTIVE §7-4 의 "접점 신설 시 다이제스트가 기본값(승인 피로 봉쇄 불변식)"이
이미 정본이므로, 코드가 그 정본을 따르도록 배달 채널을 바꿨다(§3 도 같은 날 함께 개정).

밀폐: `CYS_PACK_DIR` 을 임시 디렉터리로 덮고 `CYS_ROUND_DIR` 을 지운 뒤, **목 `cys`** 를 PATH
선두에 둔다. 목은 모든 호출을 calls.log 에 적으므로 "feed 를 쏘지 않았다"가 관측 가능해진다
(라이브 팩·라이브 feed 무접촉).

  ① 라운드 종료 경로(orchestra `_recommend_learn_once`) — **feed 호출 0** + 큐 1줄
  ② 같은 marker_key 재호출 — 큐 1줄 유지(중복 적재 0 · 마커 멱등)
  ③ eval ceiling 경로(rsi `_recommend_learn`) — feed 호출 0 + 큐 +1 · source=rsi.ceiling
  ④ 두 모듈의 큐 경로·레코드 키 집합 **동형**(같은 큐에 쓰므로 갈리면 다이제스트가 한쪽을 못 읽는다)
  ⑤ ★음성 대조 — **HEAD 의 구 javis_rsi**(개정 전)를 같은 목 하네스에 돌리면 `cys feed push`
     가 실제로 호출된다. 이 대조가 성립해야 ①③ 의 "feed 0" 이 무언가를 재는 것이 된다
     (git 이 없거나 구 파일을 못 얻으면 SKIP 을 명시 — 조용한 통과 금지).
  ⑥ 문서-코드 결박 — RSI_LEARNING_DIRECTIVE §3 이 가리키는 큐 파일명과 코드 상수가 같다.
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 LEARN-DIGEST-QUEUE-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_learn_digest_queue.py
"""
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PACK = os.path.dirname(BIN)
REPO = os.path.dirname(PACK)
sys.path.insert(0, BIN)

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def mock_cys(bindir, log):
    os.makedirs(bindir, exist_ok=True)
    p = os.path.join(bindir, "cys")
    with open(p, "w", encoding="utf-8", newline="\n") as f:
        f.write('#!/bin/sh\nprintf "%s\\n" "$*" >> "' + log + '"\nexit 0\n')
    os.chmod(p, 0o755)


def queue_lines(path):
    if not os.path.isfile(path):
        return []
    return [json.loads(l) for l in open(path, encoding="utf-8") if l.strip()]


root = tempfile.mkdtemp()
try:
    pack = os.path.join(root, "pack")
    os.makedirs(pack)
    log = os.path.join(root, "calls.log")
    binp = os.path.join(root, "mockbin")
    mock_cys(binp, log)
    os.environ["CYS_PACK_DIR"] = pack
    os.environ.pop("CYS_ROUND_DIR", None)
    for k in ("JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR"):
        os.environ.pop(k, None)
    os.environ["PATH"] = binp + os.pathsep + os.environ.get("PATH", "")

    import javis_orchestra as O          # noqa: E402
    import javis_rsi as R                # noqa: E402

    qpath = O.learn_digest_queue_path()

    # ① 라운드 종료 경로
    O._recommend_learn_once("gate", "T R1 종료 — 더 나은 방법론", "gate-T-1")
    rows = queue_lines(qpath)
    calls = open(log, encoding="utf-8").read() if os.path.isfile(log) else ""
    check("1a 큐에 1줄 적재", len(rows) == 1, repr(rows))
    check("1b feed 호출 0", "feed" not in calls, repr(calls))
    check("1c 레코드 필드", rows and rows[0]["reason"] == "gate"
          and rows[0]["source"] == "orchestra.gate-status"
          and rows[0]["status"] == "queued_for_weekly_digest", repr(rows[:1]))

    # ② 마커 멱등
    O._recommend_learn_once("gate", "T R1 종료 — 더 나은 방법론", "gate-T-1")
    check("2 같은 marker_key 재호출은 무적재", len(queue_lines(qpath)) == 1,
          repr(queue_lines(qpath)))

    # ③ eval ceiling 경로
    R._recommend_learn("ceiling", "R7 정체(ceiling) 돌파 방법론")
    rows = queue_lines(qpath)
    calls = open(log, encoding="utf-8").read() if os.path.isfile(log) else ""
    check("3a ceiling 도 큐로(누적 2줄)", len(rows) == 2, repr(rows))
    check("3b feed 호출 여전히 0", "feed" not in calls, repr(calls))
    check("3c source 구분", rows[-1]["source"] == "rsi.ceiling", repr(rows[-1]))

    # ④ 두 모듈 동형
    check("4a 큐 경로 동형(CYS_ROUND_DIR 미설정 생산 형상)",
          O.learn_digest_queue_path() == R.learn_digest_queue_path(),
          repr((O.learn_digest_queue_path(), R.learn_digest_queue_path())))
    check("4b 레코드 키 집합 동형", set(rows[0]) == set(rows[-1]), repr((sorted(rows[0]), sorted(rows[-1]))))
    check("4c 파일명 상수 동형", O.LEARN_DIGEST_QUEUE == R.LEARN_DIGEST_QUEUE)

    # ⑤ ★음성 대조 — 구 javis_rsi 는 같은 목에서 feed 를 쏜다
    #
    # ★기준점을 HEAD 에서 **이력 탐색**으로 옮겼다(2026-09-04 실측 수리). 두 결함이 겹쳐 있었다:
    #   ⓐ **이동하는 기준점**: 개정이 커밋되는 순간 HEAD 는 개정본이 된다 — 이 대조는 '개정 전'을
    #      필요로 하므로 HEAD 기준은 개정 커밋 **전에만** 유효했다(커밋과 동시에 자기 파괴).
    #   ⓑ **산문이 판별자를 통과했다**: 가드가 `"feed" not in old` 였는데, 개정본의 주석이 폐지
    #      사유를 설명하며 `cys feed push --kind learn_proposal` 을 **그대로 인용**한다. 그래서
    #      가드는 "구본이다"라고 오판했고, 개정본을 구본으로 실행해 ⑤·⑤b 가 함께 적색이 됐다.
    # 그래서 판별을 **호출 형상**(`"cys", "feed", "push"` — 산문에는 없고 argv 리터럴에만 있다)으로
    # 바꾸고, 그 형상을 가진 **가장 최근 블롭**을 이력에서 찾는다. 못 찾으면 조용히 통과하지 않고
    # SKIP 사유를 출력한다.
    CALL_PIN = '"cys", "feed", "push"'
    REL = "cysjavis-pack/bin/javis_rsi.py"
    old = old_sha = None
    if shutil.which("git") and os.path.exists(os.path.join(REPO, ".git")):
        lg = subprocess.run(["git", "-C", REPO, "log", "--format=%H", "-n", "80", "--", REL],
                            capture_output=True, text=True, timeout=60)
        for sha in (lg.stdout or "").split():
            b = subprocess.run(["git", "-C", REPO, "show", "%s:%s" % (sha, REL)],
                               capture_output=True, text=True, timeout=30)
            if b.returncode == 0 and CALL_PIN in b.stdout:
                old, old_sha = b.stdout, sha
                break
    if old is None:
        print("SKIP 5 구 코드 대조 — 이력에서 feed 호출 판(%s)을 못 찾았다(git 부재·얕은 클론·"
              "이력 절단). 조용한 통과 아님: 이 줄이 그 사실이다" % CALL_PIN)
    else:
        check("5-0 판별자가 **호출 형상**이다(산문 인용에 속지 않는다)",
              CALL_PIN in old and CALL_PIN not in open(
                  os.path.join(PACK, "bin", "javis_rsi.py"), encoding="utf-8").read(),
              "구본 %s 에는 있고 개정본에는 없어야 한다" % (old_sha or "")[:8])
        oldp = os.path.join(root, "old_javis_rsi.py")
        open(oldp, "w", encoding="utf-8", newline="\n").write(old)
        spec = importlib.util.spec_from_file_location("old_javis_rsi", oldp)
        mod = importlib.util.module_from_spec(spec)
        before = os.path.getsize(log) if os.path.isfile(log) else 0
        spec.loader.exec_module(mod)
        mod._recommend_learn("ceiling", "구 코드 대조")
        after = open(log, encoding="utf-8").read() if os.path.isfile(log) else ""
        check("5 구 코드는 같은 목에서 feed 를 쏜다(계측 타당성)",
              "feed" in after[before:], repr(after[before:][:200]))
        check("5b 구 코드는 큐에 쓰지 않는다", len(queue_lines(qpath)) == 2, repr(queue_lines(qpath)))

    # ⑥ 문서-코드 결박
    doc = open(os.path.join(PACK, "directives", "RSI_LEARNING_DIRECTIVE.md"),
               encoding="utf-8").read()
    check("6a 디렉티브가 큐 파일명을 명시", O.LEARN_DIGEST_QUEUE in doc)
    check("6b 디렉티브가 자동 트리거의 feed 발행 폐지를 명시",
          "건별 feed 승인 요청을 발행하지 않는다" in doc)
    src = open(os.path.join(BIN, "javis_orchestra.py"), encoding="utf-8").read() + \
        open(os.path.join(BIN, "javis_rsi.py"), encoding="utf-8").read()
    code_lines = [l for l in src.splitlines() if not l.strip().startswith("#")]
    check("6c 코드에 learn_proposal feed push 0(주석 제외)",
          not any("learn_proposal" in l for l in code_lines),
          repr([l.strip()[:80] for l in code_lines if "learn_proposal" in l]))
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("LEARN-DIGEST-QUEUE-OK")
sys.exit(0)
