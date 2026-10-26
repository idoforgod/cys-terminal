#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""사본 드리프트 게이트 — `main`(기본 가지)과 발행 가능한 ref 의 **같은 파일이 같은지** 본다.

★무엇이 결함이었나 (실측 2026-10-26 · worker s84)
  `.github/workflows/release-publish.yml` 이 **다섯 판**으로 갈려 있었다
  (E1 main 2,306 B/66줄 · E2 8,416/144 · E3 15,161/234 · E4 21,092/298 · E5 태그 9,036/149).
  기전은 방치가 아니라 **부분 체리픽**이다: `d67d8c3`(2026-07-23 · PR #32 "register protected
  release publishing")이 `0418f17` 에서 **이 워크플로 1개만** 가져왔고, 그 파일이 호출하는
  `scripts/release-verify.py` 는 따라오지 않았다. main 의 blob 은 그때 이후 **95일** 동안
  한 번도 바뀌지 않았고(blob `9338fd0e`), 그 사본에는 다음 일곱이 없다:
    A `--tag "$TAG"`  B `--body-file`  C `--json body`  D 검증기 자기시험
    E `.release-tools` 2차 체크아웃  F 그 경로의 검증기 호출  G 승격 후 `tagName` 사후 단언
  A 가 없으면 승격이 draft 의 `tag_name` 을 `untagged-<sha>` 로 리셋할 수 있고(그 복구 가드가
  태그판 `:136-148` 에 흉터로 남아 있다), B·C 가 없으면 **윈도우 서명 고지(SAC) 본문 게이트가
  무력**해진다. 그리고 그 스텁은 사문이 아니다 — blob `9338fd0e` 는 2026-07-24 런
  `30073672719`(ref `feat/bootstrap-identity-hardening`)에서 **실제 공개 승격을 `--tag` 없이
  완주한 적이 있다**.

★무엇이 이 결함을 가리고 있었나 (그래서 이 파일이 있다)
  오늘 `--ref main` 으로 띄우면 `release-production` 환경의 **배포 분기 정책**이 ref 를 거부해
  step 전에 죽는다(실측: 런 `33981615346` = 2초 · steps 0 · failure). 그런데 그 정책은
  **저장소에 적혀 있지 않은 GitHub 설정**이고 `main` 가지에는 보호도 룰셋도 없다
  (`GET /branches/main/protection` → 404 · `GET /rulesets` → 0건). ⇒ 즉
    ⛔**있는 안전장치가 빠진 안전장치를 가려서 결함을 무해하게 보이게 한다 —
      그 가림은 설계가 아니라 우연이고, 우연은 규범이 아니다.**
  이 스크립트는 그 가림막을 **저장소 안으로 끌어온다**: 허용 ref 목록을 여기서 단언하므로
  정책에 `main` 이 추가되면 **빨강으로 드러난다**.

★왜 로직이 워크플로가 아니라 이 파일에 있는가
  이 게이트는 두 곳에서 불린다 — `release.yml`(태그 레인 하드게이트)과
  `copy-drift-watch.yml`(일일 조기경보). 같은 비교를 두 워크플로에 **복사해 넣으면
  고치려는 병을 고치는 중에 다시 만든다**. 단일 출처는 이 파일이다.

★비교 기전 — 내용주소(blob sha)
  `GET /repos/<repo>/contents/<path>?ref=main` 의 `.sha` 는 git blob sha 다. 로컬
  `git rev-parse --verify <ref>:<path>` 와 **같은 값**이다(실측 3/3 일치:
  `9338fd0e` · `d60dd507` · `b218d629`). ⇒ 다운로드·재해시·개행 정규화가 전부 불필요하고
  **1바이트 차이도 잡는다**.

★fail-closed — ⛔「0·빈값·404」를 「같다」로 접지 않는다
  부재(404)·API 오류·네트워크 실패는 전부 **실패**다. "재지 않았다"를 통과로 접는 것이
  이 저장소가 반복해 낸 사고의 형태다(`release.yml:59-108` windows-health-gate 와 같은 교리).

★권한 측정 완료 (2026-10-26 · run 40285752606)
  허용 ref 단언이 `GITHUB_TOKEN` 으로 ★도달해 `OK` 를 냈다 ⇒ 경고 유지 사유가 소멸했고
  같은 날 `--policy-mode fatal` 로 올렸다(치명화 집행일 2026-10-26).

★그리고 계기 자신의 함정 하나 (2026-10-26 실측 · 피해 0)
  ⛔`git rev-parse <ref>:<없는 경로>` 는 실패할 때 **인자를 그대로 stdout 에 되돌려 준다**
  (실측: stdout=`main:.github/workflows/release-postprocess.yml` · rc=128). 그래서 rc 를
  보지 않고 stdout 만 읽으면 **부재가 그럴듯한 blob 값으로 보인다**. `--verify` 를 붙이면
  stdout 은 비고 rc=128 이다(실측). ⇒ 이 파일은 ⒤`--verify` ⒥rc ⒦40자 16진수 **셋 다** 본다.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request

REPO_DEFAULT = "idoforgod/cys-terminal"
BASELINE_DEFAULT = "main"

# ★감시 경로 — 부서장 판정 2026-10-26 (HEAD-RULING-P0-1 §4) 으로 5종.
#   앞 셋 = 발행 레인의 **최소 기능 집합**(워크플로 + 검증기 + 그 검증기의 자기시험 대상).
#     ⇒ 워크플로만 맞추면 D 가 가리키는 파일이 main 에 없어 죽는다.
#   뒤 둘 = 「게이트 부재는 게이트가 알릴 수 없다」의 완화:
#     `release.yml` 은 이 게이트를 **담은** 파일이고,
#     `release-postprocess.yml` 은 main 에 없어서 2026-10-26 발행에서 `workflow_dispatch`
#     불가 ⇒ `push` 발화로 우회하는 **집행 비용을 이미 물린** 같은 결함이다.
#   ⛔`scripts/version-check.sh` 는 대상이 아니다 — 발행 레인은 그 파일을 **태그 트리**에서
#     읽으므로 main 의 사본은 이 레인에서 영원히 실행되지 않는다.
#   여섯째 = **이 파일 자신**(부서장 판정 2026-10-26 §2). 사유: `copy-drift-watch.yml`(일일 감시)는
#     **main 사본**의 이 스크립트를 쓴다 ⇒ main 쪽이 낡으면 ★엉뚱한 목록으로 초록을 낸다
#     = 조용한 실패. ⇒ 그래서 ★목록은 길이만이 아니라 **경로 문자열 전수**로 자기시험이 단언한다
#     (⛔`len == N` 만 보면 N 개가 **다른 N 개**로 바뀌어도 통과한다).
WATCHED_PATHS = [
    ".github/workflows/release-publish.yml",
    "scripts/release-verify.py",
    "scripts/tests/test_release_verify.py",
    ".github/workflows/release.yml",
    ".github/workflows/release-postprocess.yml",
    "scripts/copy-drift-check.py",
]

# ★`release-production` 환경의 배포 분기 정책 — 저장소 밖 config 를 저장소 안에서 단언한다.
#   실측 2026-10-26: total_count=2 · `feat/bootstrap-identity-hardening`(branch) · `v*`(tag).
#   ⇒ 여기에 `main` 이 추가되면(두 번 클릭으로 가능) 이 게이트가 빨강이 된다.
ENVIRONMENT_NAME = "release-production"
EXPECTED_POLICIES = {
    ("feat/bootstrap-identity-hardening", "branch"),
    ("v*", "tag"),
}

# ★정책 단언의 치명도 — ★**치명(fatal)이다.** 경고로 시작했고 ★2026-10-26 에 올렸다.
#
# ★왜 경고로 시작했나(기록으로 남긴다 — ⛔지우지 않는다):
#   `GITHUB_TOKEN`(job 권한 `contents: read`)이
#   `GET /environments/<env>/deployment-branch-policies` 에 ★도달하는지가 **미측정**이었다.
#   문서는 「Anyone with read access」라 하고 이 저장소는 public 이라 될 것으로 보였지만
#   ★그건 추론이었다 — 성공시켜 본 호출은 **호스트 PAT** 였고 그 사례는 GITHUB_TOKEN 의
#   도달을 증명하지 않는다. 403 이면 fail-closed 가 ★**모든 릴리스를 막고**, 그러면 위험이
#   제거되지 않고 ★「사람이 게이트를 끄는 것」으로 **이동한다** — 가장 나쁜 착지다.
#   ⇒ 그래서 ★「측정 불능은 통과도 실패도 아니다」에 따라 **경고**로 두었다.
#
# ★★그리고 ★측정됐다 ⇒ ★경고 유지 사유가 **소멸**했다:
#   ⒤계기: `copy-drift-watch.yml` 을 `workflow_dispatch` 로 1회 — ★**run 40285752606**
#     (2026-10-26T07:25:58Z · `head_sha=f27879cb` = 동기화가 착지한 main · actor idoforgod).
#   ⒥그 런의 로그가 적은 것(실측 인용):
#       `정책 단언 치명도: fatal  (치명 — 실패가 종료코드에 반영된다)`
#       `허용 ref 단언   : 환경 release-production · 기대 2건 {feat/bootstrap-identity-hardening(branch), v*(tag)}`
#       `                  실측 2건 {feat/bootstrap-identity-hardening(branch), v*(tag)}`
#       `… 허용 ref 단언 OK(fatal · 집계 기여 0) …`
#   ⒦⇒ ★**GITHUB_TOKEN 이 그 API 에 도달했고 정책이 기대와 일치했다**
#     ⇒ ⛔`UNMEASURED` 가 아니다 ⇒ ★경고로 둘 근거가 없다.
#
# ★★치명화 집행일: ★**2026-10-26**   (근거 = 위 run 40285752606 · ★같은 날 집행)
#   ⇒ `release.yml` 의 게이트 step 이 ★`--policy-mode fatal` 로 돈다(그 파일 주석에도 같은 날짜).
#   ⇒ 이 파일의 ★기본값도 `fatal` 이다 — ⛔기본값이 `warn` 으로 남으면 손으로 돌린 실행만
#     조용히 옛 상태를 쓰게 되고, 그게 이 티켓이 고치는 **바로 그 병**이다(사본·기본값 드리프트).
#
# ⚠되돌리는 조건(한 줄로 적어 둔다): 이 단언이 ★조회 실패(`UNMEASURED`)로 릴리스를 막기 시작하면
#   ⛔게이트를 끄지 말고 ★그 조회 실패를 먼저 측정하라 — `[조회 실패]` 와 `[정책 변경]` 은
#   아래에서 **다른 문장**으로 나온다. 원인이 권한 회수라면 그때 다시 `warn` 으로 내리고
#   ★그 날짜와 근거를 이 자리에 적는다.
POLICY_MODES = ("warn", "fatal")

SHA_RE = re.compile(r"^[0-9a-f]{40}$")

# 이쪽 사본을 읽는 출처 — argparse 의 choices 와 자기시험이 이 한 곳을 본다.
REF_SOURCES = ("git", "api")


# ─────────────────────────────────────────────────────────────────────────────
# 계기
# ─────────────────────────────────────────────────────────────────────────────
def is_blob_sha(value):
    """★40자 16진수만 blob sha 로 인정한다 — 에코형 실패를 값으로 받지 않는다."""
    return bool(value) and bool(SHA_RE.match(value))


def git_blob_sha(ref, path, cwd=None):
    """로컬 트리의 blob sha. 반환 (sha|None, 사유).

    ⛔`--verify` 없이 쓰지 마라: 실패 시 인자가 stdout 으로 돌아온다(위 docstring 실측).
    """
    spec = "%s:%s" % (ref, path)
    proc = subprocess.run(
        ["git", "rev-parse", "--verify", spec],
        cwd=cwd, capture_output=True, text=True,
    )
    out = proc.stdout.strip()
    if proc.returncode != 0:
        return None, "git rev-parse rc=%d (%s)" % (proc.returncode, proc.stderr.strip()[:120] or "부재")
    if not is_blob_sha(out):
        # 여기에 닿으면 git 거동이 바뀐 것이다 — 조용히 통과시키지 않는다.
        return None, "git 출력이 blob sha 형태가 아니다: %r" % out[:80]
    return out, ""


def _token():
    """토큰은 ⒤환경변수 ⒥git credential 순으로 **프로세스 안에서만** 얻는다.

    ⛔argv 에 넣지 않는다(같은 사용자 `ps` 에 노출된다) · ⛔파일에 쓰지 않는다 · ⛔출력하지 않는다.
    """
    for key in ("GH_TOKEN", "GITHUB_TOKEN"):
        value = os.environ.get(key)
        if value:
            return value
    proc = subprocess.run(
        ["git", "credential", "fill"],
        input="protocol=https\nhost=github.com\n\n",
        capture_output=True, text=True,
    )
    for line in proc.stdout.splitlines():
        if line.startswith("password="):
            return line[len("password="):]
    return None


def api_get(url_path):
    """GitHub REST GET. 반환 (status, payload|None, 사유). ⛔예외를 통과로 바꾸지 않는다."""
    req = urllib.request.Request(
        "https://api.github.com" + url_path,
        method="GET",
        headers={
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "cys-copy-drift-check",
        },
    )
    token = _token()
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            raw = resp.read().decode("utf-8")
            return resp.status, (json.loads(raw) if raw.strip() else {}), ""
    except urllib.error.HTTPError as exc:
        try:
            body = exc.read().decode("utf-8")
        except Exception:
            body = ""
        return exc.code, None, body[:200]
    except Exception as exc:                      # 네트워크·타임아웃·DNS
        return None, None, "%s: %s" % (type(exc).__name__, exc)


def remote_blob_sha(repo, path, ref):
    """기준선 ref 의 blob sha. 반환 (sha|None, 사유). ⛔404 를 「같다」로 접지 않는다."""
    status, payload, detail = api_get(
        "/repos/%s/contents/%s?ref=%s" % (repo, path, ref)
    )
    if status == 404:
        return None, "%s 에 **없다**(HTTP 404)" % ref
    if status != 200 or not isinstance(payload, dict):
        return None, "contents 조회 실패 status=%s %s" % (status, detail)
    sha = payload.get("sha")
    if not is_blob_sha(sha):
        return None, "contents 응답의 sha 가 blob sha 형태가 아니다: %r" % (sha,)
    return sha, ""


# ─────────────────────────────────────────────────────────────────────────────
# 판정
# ─────────────────────────────────────────────────────────────────────────────
def classify(here, here_why, there, there_why):
    """한 경로의 판정. 반환 ('OK'|'DRIFT'|'UNMEASURED', 사람이 읽을 사유)."""
    if here is None:
        return "UNMEASURED", "이쪽 사본을 재지 못했다 — %s" % here_why
    if there is None:
        return "UNMEASURED", "기준선 사본을 재지 못했다 — %s" % there_why
    if here == there:
        return "OK", ""
    return "DRIFT", "here=%s baseline=%s" % (here[:8], there[:8])


def classify_policies(status, payload, detail):
    """허용 ref 단언. 반환 ('OK'|'CHANGED'|'UNMEASURED', 사유, 실측집합|None).

    ★「조회 실패」와 「정책이 바뀌었다」를 ⛔같은 말로 적지 않는다 — 빨강의 원인을 오독하면
      사람이 엉뚱한 것을 고친다.
    """
    if status != 200 or not isinstance(payload, dict):
        return "UNMEASURED", "정책 조회 실패 status=%s %s" % (status, detail), None
    policies = payload.get("branch_policies")
    if not isinstance(policies, list):
        return "UNMEASURED", "응답에 branch_policies 배열이 없다", None
    actual = set()
    for item in policies:
        if not isinstance(item, dict):
            return "UNMEASURED", "branch_policies 항목이 객체가 아니다", None
        actual.add((item.get("name"), item.get("type")))
    if actual == EXPECTED_POLICIES:
        return "OK", "", actual
    added = sorted(actual - EXPECTED_POLICIES)
    removed = sorted(EXPECTED_POLICIES - actual)
    return "CHANGED", "추가 %s · 사라짐 %s" % (added or "없음", removed or "없음"), actual


def policy_failure_count(verdict, mode):
    """정책 단언이 ★실패 집계에 보태는 수. 경고 모드에서는 ⛔항상 0 이다(릴리스를 막지 않는다).

    ⚠경고 모드가 ⛔「검사하지 않음」이 아니라는 점이 중요하다 — 판정은 그대로 계산되고 그대로
      출력되며 `::warning::` 으로 밖에 나간다. 바뀌는 것은 **종료코드에 미치는 영향**뿐이다.
    """
    if mode not in POLICY_MODES:
        # ⛔모르는 모드를 조용히 「경고」로 흘리지 않는다.
        raise ValueError("알 수 없는 --policy-mode: %r (허용 %s)" % (mode, POLICY_MODES))
    if mode == "warn":
        return 0
    return 0 if verdict in ("OK", "SKIPPED") else 1


def fmt_set(items):
    return "{" + ", ".join("%s(%s)" % (n, t) for n, t in sorted(items, key=lambda x: (str(x[0]), str(x[1])))) + "}"


# ─────────────────────────────────────────────────────────────────────────────
# 본체
# ─────────────────────────────────────────────────────────────────────────────
def here_blob_sha(repo, ref, path, ref_source, cwd=None):
    """이쪽 사본의 blob sha. `git`(로컬 트리) 또는 `api`(원격 ref) 에서 가져온다.

    ★`api` 가 필요한 까닭: 일일 조기경보(`copy-drift-watch.yml`)는 **기본 가지에서** 돌므로
      로컬 트리가 main 이다. 그 레인이 비교해야 하는 이쪽은 **최신 릴리스 태그**이고, 태그를
      로컬로 끌어오려면 전체 fetch 가 필요하다. 같은 내용주소(blob sha)를 API 로 바로 읽는다.
    """
    if ref_source == "api":
        return remote_blob_sha(repo, path, ref)
    if ref_source == "git":
        return git_blob_sha(ref, path, cwd=cwd)
    # ⛔모르는 출처를 조용히 git 으로 흘리지 않는다 — 그 조용함이 이 저장소의 사고 형태다.
    raise ValueError("알 수 없는 --ref-source: %r (허용 %s)" % (ref_source, REF_SOURCES))


def run(repo, ref, baseline, paths, check_policy, cwd=None, ref_source="git",
        policy_mode="fatal", environment=ENVIRONMENT_NAME):
    print("=== 사본 드리프트 게이트 (copy-drift-check) ===")
    print("저장소          : %s" % repo)
    print("이쪽(비교 대상) : %s  (출처 %s)" % (ref, ref_source))
    print("저쪽(기준선)    : %s" % baseline)
    print("감시 경로 분모  : %d" % len(paths))     # ★분모를 먼저 선언한다
    print("정책 단언 치명도: %s%s" % (
        policy_mode,
        "  (★경고 — 릴리스를 막지 않는다 · ⛔2026-10-26 치명화 이후 이 모드는 예외 경로다)"
        if policy_mode == "warn"
        else "  (★치명 — 실패가 종료코드에 반영된다 · 2026-10-26 run 40285752606 으로 권한 도달 측정됨)"))
    if os.environ.get("GITHUB_REF_TYPE"):
        print("발화 ref        : ref_type=%s ref_name=%s"
              % (os.environ.get("GITHUB_REF_TYPE"), os.environ.get("GITHUB_REF_NAME")))

    tally = {"OK": 0, "DRIFT": 0, "UNMEASURED": 0}
    rows = []
    for index, path in enumerate(paths, 1):
        here, here_why = here_blob_sha(repo, ref, path, ref_source, cwd=cwd)
        there, there_why = remote_blob_sha(repo, path, baseline)
        verdict, why = classify(here, here_why, there, there_why)
        tally[verdict] += 1
        rows.append({"path": path, "verdict": verdict, "here": here, "baseline": there, "why": why})
        mark = {"OK": "일치", "DRIFT": "⛔드리프트", "UNMEASURED": "⛔측정 불능"}[verdict]
        print("  [%d/%d] %-52s %s" % (index, len(paths), path, mark))
        print("          이쪽=%s 저쪽=%s%s"
              % (here[:12] if here else "(없음)", there[:12] if there else "(없음)",
                 "  — " + why if why else ""))
        if verdict == "DRIFT":
            print("::error::사본 드리프트 — %s: tag=%s main=%s" % (path, here, there))
        elif verdict == "UNMEASURED":
            print("::error::측정 불능은 통과가 아니다 — %s: %s" % (path, why))

    policy_verdict = "SKIPPED"
    policy_why = ""
    if check_policy:
        status, payload, detail = api_get(
            "/repos/%s/environments/%s/deployment-branch-policies" % (repo, environment)
        )
        policy_verdict, policy_why, actual = classify_policies(status, payload, detail)
        # ★경고 모드에서도 판정은 그대로 계산·출력된다 — 바뀌는 것은 종료코드 영향뿐이다.
        lvl = "warning" if policy_mode == "warn" else "error"
        print("허용 ref 단언   : 환경 %s · 기대 %d건 %s"
              % (environment, len(EXPECTED_POLICIES), fmt_set(EXPECTED_POLICIES)))
        print("                  실측 %s %s"
              % ("%d건" % len(actual) if actual is not None else "(재지 못함)",
                 fmt_set(actual) if actual is not None else ""))
        if policy_verdict == "OK":
            print("                  ⇒ 일치")
        elif policy_verdict == "CHANGED":
            # ★원인① — 정책이 **바뀌었다**(조회는 됐다).
            print("::%s::[정책 변경] %s 환경의 허용 ref 가 바뀌었다 — %s"
                  % (lvl, environment, policy_why))
            print("::%s::[정책 변경] 이 목록은 저장소 밖 config 다. 의도한 변경이면 "
                  "scripts/copy-drift-check.py 의 EXPECTED_POLICIES 를 같은 커밋에서 고쳐라." % lvl)
        else:
            # ★원인② — **재지 못했다**(정책이 바뀐 것이 아니다). ⛔두 원인을 같은 말로 적지 않는다:
            #   빨강/노랑의 원인을 오독하면 사람이 엉뚱한 것을 고친다.
            print("::%s::[조회 실패] 허용 ref 를 재지 못했다 — ⛔이것은 ★정책 변경이 아니다 — %s"
                  % (lvl, policy_why))
            print("::%s::[조회 실패] 토큰 권한·네트워크·환경 이름을 본다. "
                  "⛔EXPECTED_POLICIES 를 고치지 마라(바뀐 증거가 없다)." % lvl)
        if policy_mode == "warn" and policy_verdict != "OK":
            print("::%s::[경고 유지] 이 칸은 ★종료코드에 반영되지 않는다(릴리스를 막지 않는다). "
                  "⛔이 모드는 2026-10-26(run 40285752606) 치명화 이후의 ★예외 경로다 — "
                  "왜 warn 으로 돌렸는지 근거와 날짜가 코드에 적혀 있어야 한다." % lvl)

    policy_failed = policy_failure_count(policy_verdict, policy_mode) if check_policy else 0
    failed = tally["DRIFT"] + tally["UNMEASURED"] + policy_failed
    print("=== 집계: 경로 %d 중 일치 %d · 드리프트 %d · 측정 불능 %d · "
          "허용 ref 단언 %s(%s · 집계 기여 %d) ⇒ 실패 %d건 ==="
          % (len(paths), tally["OK"], tally["DRIFT"], tally["UNMEASURED"],
             policy_verdict, policy_mode if check_policy else "skipped", policy_failed, failed))
    if failed:
        print("::error::사본 드리프트 게이트 실패 %d건 — 릴리스 자산 생산을 중단한다(fail-closed)." % failed)
        return 1, rows
    print("✅ 사본 일치 %d 중 %d · 허용 ref 단언 %s(%s)"
          % (len(paths), tally["OK"], policy_verdict, policy_mode if check_policy else "skipped"))
    return 0, rows


# ─────────────────────────────────────────────────────────────────────────────
# 자기시험 — ★관문을 믿기 전에 관문이 살아 있는지 본다 (네트워크 불요 · 1초 미만)
# ─────────────────────────────────────────────────────────────────────────────
def self_test():
    tally = {"pass": 0, "fail": 0}

    def ok(name, cond, detail=""):
        if cond:
            tally["pass"] += 1
        else:
            tally["fail"] += 1
        print(("PASS " if cond else "FAIL ") + name + (" | " + detail if detail else ""))

    good = "9338fd0ea95c094bc518dcd9cda65ba38de42bdc"
    other = "d60dd5073fa96348ca3ccefaf0a590cbd7327b9f"

    # ① blob sha 형태 판정 — ★에코형 실패를 값으로 받지 않는다
    ok("① 정상 blob sha 를 받아들인다", is_blob_sha(good))
    ok("② ⛔rev-parse 에코 문자열을 거부한다",
       not is_blob_sha("main:.github/workflows/release-postprocess.yml"))
    ok("③ ⛔빈 문자열을 거부한다", not is_blob_sha(""))
    ok("④ ⛔None 을 거부한다", not is_blob_sha(None))
    ok("⑤ ⛔39자·41자·대문자를 거부한다",
       not is_blob_sha(good[:39]) and not is_blob_sha(good + "0") and not is_blob_sha(good.upper()))

    # ② 경로 판정 — ⛔부재·오류를 「같다」로 접지 않는다
    ok("⑥ 같은 sha ⇒ OK", classify(good, "", good, "")[0] == "OK")
    ok("⑦ 다른 sha ⇒ DRIFT", classify(good, "", other, "")[0] == "DRIFT")
    ok("⑧ ★기준선 부재(404) ⇒ ⛔OK 가 아니라 UNMEASURED",
       classify(good, "", None, "main 에 **없다**(HTTP 404)")[0] == "UNMEASURED")
    ok("⑨ 이쪽 부재 ⇒ UNMEASURED", classify(None, "git rev-parse rc=128", good, "")[0] == "UNMEASURED")
    ok("⑩ 양쪽 부재 ⇒ ⛔OK 가 아니라 UNMEASURED",
       classify(None, "x", None, "y")[0] == "UNMEASURED")

    # ③ 허용 ref 단언
    base = [{"name": "feat/bootstrap-identity-hardening", "type": "branch"},
            {"name": "v*", "type": "tag"}]
    ok("⑪ 기대와 같은 정책 ⇒ OK",
       classify_policies(200, {"total_count": 2, "branch_policies": base}, "")[0] == "OK")
    ok("⑫ ★main 이 추가되면 ⇒ CHANGED",
       classify_policies(200, {"total_count": 3, "branch_policies":
                               base + [{"name": "main", "type": "branch"}]}, "")[0] == "CHANGED")
    ok("⑬ 항목이 사라지면 ⇒ CHANGED",
       classify_policies(200, {"total_count": 1, "branch_policies": base[:1]}, "")[0] == "CHANGED")
    ok("⑭ 이름이 바뀌면 ⇒ CHANGED",
       classify_policies(200, {"total_count": 2, "branch_policies":
                               [{"name": "feat/bootstrap-identity-hardening", "type": "branch"},
                                {"name": "v*", "type": "branch"}]}, "")[0] == "CHANGED")
    ok("⑮ ★조회 실패(403) ⇒ ⛔CHANGED 가 아니라 UNMEASURED",
       classify_policies(403, None, "Resource not accessible")[0] == "UNMEASURED")
    ok("⑯ ★조회 실패를 OK 로 접지 않는다",
       classify_policies(None, None, "URLError")[0] == "UNMEASURED")
    ok("⑰ branch_policies 가 배열이 아니면 ⇒ UNMEASURED",
       classify_policies(200, {"branch_policies": None}, "")[0] == "UNMEASURED")

    # ④ ★감시 목록 자체 — ⛔길이만 보지 않는다. 길이만 단언하면 **N 개가 다른 N 개로 바뀌어도**
    #    통과한다(부서장 판정 2026-10-26 §2 가 닫으라고 지목한 구멍이다). ⇒ ★경로 문자열을
    #    **전수·순서까지** 단언한다. 아래 리터럴은 ⛔WATCHED_PATHS 를 참조하지 않는다 —
    #    참조하면 계기가 자기를 재는 항진명제가 된다. 목록을 고치려면 ★두 곳을 같이 고쳐야 한다.
    expected_watched = (
        ".github/workflows/release-publish.yml",
        "scripts/release-verify.py",
        "scripts/tests/test_release_verify.py",
        ".github/workflows/release.yml",
        ".github/workflows/release-postprocess.yml",
        "scripts/copy-drift-check.py",
    )
    ok("⑱ 감시 경로가 6종이다(부서장 판정 2026-10-26 §2)", len(WATCHED_PATHS) == 6,
       "실측 %d" % len(WATCHED_PATHS))
    ok("⑲ ★경로 문자열이 전수·순서까지 기대와 같다(⛔길이만이 아니다)",
       tuple(WATCHED_PATHS) == expected_watched,
       "차집합 추가=%s 빠짐=%s" % (sorted(set(WATCHED_PATHS) - set(expected_watched)),
                                   sorted(set(expected_watched) - set(WATCHED_PATHS))))
    for idx, want in enumerate(expected_watched):
        got = WATCHED_PATHS[idx] if idx < len(WATCHED_PATHS) else "(없음)"
        ok("⑲-%d 경로[%d] == %s" % (idx + 1, idx, want), got == want, "실측 %s" % got)
    ok("⑳ 중복 경로가 없다", len(set(WATCHED_PATHS)) == len(WATCHED_PATHS))
    ok("㉑ ⛔version-check.sh 는 감시 대상이 아니다(태그 트리에서 읽는다)",
       "scripts/version-check.sh" not in WATCHED_PATHS)
    ok("㉒ ★이 스크립트 자신이 감시 대상이다(A2 가 main 사본을 쓴다)",
       "scripts/copy-drift-check.py" in WATCHED_PATHS)

    # ⑥ ★정책 단언의 치명도 — 경고 모드가 ⛔릴리스를 막지 않는가 / 치명 모드는 막는가
    ok("㉓ ★warn: OK 는 집계 기여 0", policy_failure_count("OK", "warn") == 0)
    ok("㉔ ★warn: ★CHANGED 도 집계 기여 ⛔0(릴리스를 막지 않는다)",
       policy_failure_count("CHANGED", "warn") == 0)
    ok("㉕ ★warn: ★UNMEASURED 도 집계 기여 ⛔0(권한 미측정이 릴리스를 막지 않는다)",
       policy_failure_count("UNMEASURED", "warn") == 0)
    ok("㉖ fatal: OK ⇒ 0 · SKIPPED ⇒ 0",
       policy_failure_count("OK", "fatal") == 0 and policy_failure_count("SKIPPED", "fatal") == 0)
    ok("㉗ ★fatal: CHANGED ⇒ 1 · UNMEASURED ⇒ 1",
       policy_failure_count("CHANGED", "fatal") == 1
       and policy_failure_count("UNMEASURED", "fatal") == 1)
    try:
        policy_failure_count("OK", "bogus-mode")
        mode_raised = False
    except ValueError:
        mode_raised = True
    ok("㉘ ★모르는 --policy-mode 는 ⛔조용히 warn 으로 흐르지 않고 터진다", mode_raised)
    ok("㉙ 허용 치명도는 warn·fatal 둘뿐이다(argparse 와 같은 상수)",
       POLICY_MODES == ("warn", "fatal"), "실측 %r" % (POLICY_MODES,))

    # ⑦ ★치명화(2026-10-26) 상태 단언 — ⛔기본값이 조용히 옛 상태로 남지 않는가
    import inspect
    default_mode = inspect.signature(run).parameters["policy_mode"].default
    ok("㉚ ★run() 의 기본 치명도가 fatal 이다(권한 측정 후 상태)",
       default_mode == "fatal", "실측 %r" % (default_mode,))
    ok("㉛ ★치명화 근거(측정 런 번호)가 ★코드에 적혀 있다",
       "40285752606" in (__doc__ or ""), "docstring 에 run 번호 기록")

    # ⑤ 이쪽 사본의 출처 분기 — ⛔항진명제로 세지 않는다(실제로 불러 본다 · 네트워크 불요)
    #   git 모드는 **로컬 git** 을 부른다: 없는 경로에 대해 rc 를 담은 사유를 돌려준다.
    missing_sha, missing_why = here_blob_sha(
        "unused/repo", "HEAD", "no/such/path/__copy_drift_selftest__", "git")
    ok("㉜ ★git 모드가 로컬 git 을 부른다(없는 경로 ⇒ rc 사유 · 네트워크 아님)",
       missing_sha is None and "rc=" in missing_why, missing_why[:60])

    #   ⛔모르는 출처가 조용히 git 으로 흐르면 안 된다 — 터져야 한다.
    try:
        here_blob_sha("unused/repo", "HEAD", "LICENSE", "bogus-source")
        raised = False
    except ValueError:
        raised = True
    ok("㉝ ★모르는 --ref-source 는 ⛔조용히 git 으로 흐르지 않고 터진다", raised)
    ok("㉞ 허용 출처는 git·api 둘뿐이다(argparse 와 같은 상수)",
       REF_SOURCES == ("git", "api"), "실측 %r" % (REF_SOURCES,))

    total = tally["pass"] + tally["fail"]
    print("\n=== self-test %d/%d PASS (실패 %d건) ===" % (tally["pass"], total, tally["fail"]))
    return 0 if tally["fail"] == 0 else 1


def main():
    parser = argparse.ArgumentParser(
        description="main 사본과 발행 가능한 ref 의 사본이 같은지 본다(내용주소 비교 · fail-closed)")
    parser.add_argument("--repo", default=REPO_DEFAULT, help="owner/name (기본 %s)" % REPO_DEFAULT)
    parser.add_argument("--ref", default="HEAD", help="이쪽 ref — 비교 대상(기본 HEAD)")
    parser.add_argument("--baseline", default=BASELINE_DEFAULT,
                        help="저쪽 ref — 기준선(기본 %s)" % BASELINE_DEFAULT)
    parser.add_argument("--ref-source", choices=REF_SOURCES, default="git",
                        help="이쪽 사본을 어디서 읽는가 — git=로컬 트리(태그 레인) · api=원격 ref(일일 감시)")
    parser.add_argument("--path", action="append", default=None,
                        help="감시 경로를 직접 지정(⛔시험용 · 기본은 WATCHED_PATHS 5종)")
    parser.add_argument("--policy-mode", choices=POLICY_MODES, default="fatal",
                        help="허용 ref 단언의 치명도 — fatal=실패가 종료코드에 반영된다(★현재 기본 · "
                             "2026-10-26 run 40285752606 으로 권한 도달이 측정돼 올렸다) · "
                             "warn=집계에 보태지 않는다(릴리스를 막지 않는다 · ⛔권한 회수가 측정된 경우에만)")
    parser.add_argument("--environment", default=ENVIRONMENT_NAME,
                        help="허용 ref 를 물어볼 환경 이름(기본 %s)" % ENVIRONMENT_NAME)
    parser.add_argument("--no-policy", action="store_true",
                        help="허용 ref 단언을 건너뛴다(⛔시험용 — 레인에서는 쓰지 않는다)")
    parser.add_argument("--json", dest="json_out", default=None, help="판정 결과를 JSON 으로 쓴다")
    parser.add_argument("--self-test", action="store_true",
                        help="합성 픽스처로 게이트 자신을 시험한다(네트워크 불요)")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    paths = args.path if args.path else WATCHED_PATHS
    code, rows = run(args.repo, args.ref, args.baseline, paths, not args.no_policy,
                     ref_source=args.ref_source, policy_mode=args.policy_mode,
                     environment=args.environment)
    if args.json_out:
        with open(args.json_out, "w", encoding="utf-8") as handle:
            json.dump({"exit": code, "rows": rows}, handle, ensure_ascii=False, indent=2)
    return code


if __name__ == "__main__":
    sys.exit(main())
