#!/usr/bin/env python3
"""pre-tag-ci-check.py — 태그 전 점검(P2): 태그할 커밋의 브랜치 CI 가 **같은 SHA 에서 초록**인지 확인한다.

★왜 존재하는가(2026-09-11 · v0.14.34 윈도우 빌드 파손): 브랜치 push 10:49:46Z → 태그 push 10:50:44Z(58초 뒤).
  같은 커밋(88c1ca2)의 브랜치 windows-build 는 11:00:31Z 에 failure 였는데, 태그 레인(release.yml)은 브랜치
  런의 결과를 보지 않는다. 태그를 CI 결과 **뒤에** 두는 장치가 없어 파손본이 그대로 태그됐다.
  이 도구가 docs/RELEASE.md §0-C 태그 전 사전 게이트 3번이다 — 초록이 아니면 태그하지 않는다.

★gh 불필요: 공개 저장소의 Actions 런·잡 목록은 인증 없이 조회된다(익명 한도 시간당 60회 · --wait 는 60초 간격).
  실패한 런은 잡·스텝 이름과 check-run annotation(익명으로 읽히는 유일한 실패 사유 채널)을 함께 보여 준다.

사용: python3 scripts/pre-tag-ci-check.py [<SHA 또는 ref>] [--wait <분>] [--repo <owner/name>]
      python3 scripts/pre-tag-ci-check.py --self-test      # 판정 규칙 자기 검체(네트워크 0)
  SHA 기본 = HEAD · repo 기본 = git remote origin. --wait N 은 진행 중·런 없음 상태를 최대 N분 다시 본다.
exit 0 = 필수 워크플로(ci-branch · windows-build) 둘 다 같은 SHA 에서 success → 태그해도 된다
     1 = 하나라도 실패·취소·진행 중·런 없음 → 태그 금지(fail-closed)
     2 = 판정 불가(네트워크·API 한도·응답 형식·인자) → 태그 금지(통과가 아니다)
"""
import json
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

# 태그 전에 초록이어야 하는 브랜치 워크플로(파일 경로로 판정 — 이름은 바뀔 수 있다).
REQUIRED = (
    (".github/workflows/ci-branch.yml", "ci-branch"),
    (".github/workflows/windows-build.yml", "windows-build (feasibility)"),
)
API = "https://api.github.com"
POLL_S = 60


def decide(runs, required=REQUIRED):
    """순수 판정 → (코드, 행들, 기다릴 만한가). 워크플로마다 **가장 최근 런**(created_at·id 순)만 본다 —
    실패 뒤 재실행이 초록이면 초록, 초록 뒤 새 런이 적색이면 적색이다."""
    rows, code, waitable = [], 0, True
    for path, label in required:
        mine = [r for r in runs if r.get("path") == path]
        if not mine:
            rows.append((label, "런 없음 — 이 SHA 가 push 되지 않았거나 워크플로가 아직 트리거되지 않았다", None))
            code = 1
            continue
        last = max(mine, key=lambda r: (r.get("created_at") or "", r.get("id") or 0))
        st, cc = last.get("status"), last.get("conclusion")
        if st == "completed" and cc == "success":
            rows.append((label, "success", last))
        elif st != "completed":
            rows.append((label, "진행 중(%s) — 끝날 때까지 태그하지 않는다" % st, last))
            code = 1
        else:
            rows.append((label, "%s — 태그 금지" % cc, last))
            code, waitable = 1, False  # 끝난 적색은 기다려도 바뀌지 않는다
    return code, rows, waitable


def get(url):
    req = urllib.request.Request(url, headers={
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "cys-pre-tag-ci-check",
    })
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout.strip()


def origin_repo():
    url = git("remote", "get-url", "origin")
    m = re.search(r"github\.com[:/]+([^/]+)/([^/]+?)(?:\.git)?/?$", url)
    if not m:
        raise ValueError("origin 이 github 저장소가 아니다: %s" % url)
    return "%s/%s" % (m.group(1), m.group(2))


def explain_failure(run):
    """실패한 런의 잡·스텝·annotation — 로그가 익명 403 이어도 annotation 은 읽힌다."""
    try:
        jobs = get(run["jobs_url"] + "?per_page=100").get("jobs", [])
    except Exception as e:  # 설명은 보조다 — 판정을 바꾸지 않는다
        print("     (잡 목록 조회 실패: %s)" % e)
        return
    for j in jobs:
        if j.get("conclusion") not in ("failure", "cancelled", "timed_out"):
            continue
        steps = [s.get("name") for s in j.get("steps", []) if s.get("conclusion") == "failure"]
        print("     잡 %s: %s · 실패 스텝 %s" % (j.get("name"), j.get("conclusion"), steps or "-"))
        try:
            anns = get(j["check_run_url"] + "/annotations")
        except Exception:
            anns = []
        for a in anns[:5]:
            print("       · %s:%s %s" % (a.get("path"), a.get("start_line"), (a.get("message") or "").splitlines()[0][:200]))


def check(repo, sha):
    data = get("%s/repos/%s/actions/runs?head_sha=%s&per_page=100" % (API, repo, sha))
    runs = data.get("workflow_runs")
    if not isinstance(runs, list):
        raise ValueError("응답 형식이 예상과 다르다(workflow_runs 없음)")
    return decide(runs)


def self_test():
    def run(path, st, cc, t, i):
        return {"path": path, "status": st, "conclusion": cc, "created_at": t, "id": i}
    cb, wb = REQUIRED[0][0], REQUIRED[1][0]
    ok = lambda p, t="1", i=1: run(p, "completed", "success", t, i)
    cases = [
        ("둘 다 success", [ok(cb), ok(wb, i=2)], 0),
        ("windows-build failure — v0.14.34 형태", [ok(cb), run(wb, "completed", "failure", "1", 2)], 1),
        ("진행 중은 초록이 아니다", [ok(cb), run(wb, "in_progress", None, "1", 2)], 1),
        ("런 없음은 초록이 아니다", [ok(cb)], 1),
        ("옛 실패 뒤 재실행 success = 초록", [ok(cb), run(wb, "completed", "failure", "1", 2), ok(wb, "2", 3)], 0),
        ("옛 success 뒤 새 런 failure = 적색", [ok(cb), ok(wb, i=2), run(wb, "completed", "failure", "2", 3)], 1),
        ("cancelled 는 초록이 아니다", [run(cb, "completed", "cancelled", "1", 1), ok(wb, i=2)], 1),
        ("다른 워크플로의 초록은 세지 않는다", [ok(".github/workflows/windows-health.yml"), ok(cb, i=2)], 1),
    ]
    bad = 0
    for name, runs, want in cases:
        got = decide(runs)[0]
        print("%s | %s | 기대 %d · 실제 %d" % ("PASS" if got == want else "FAIL", name, want, got))
        bad += got != want
    print("== 자기 검체: FAIL %d건" % bad)
    return 1 if bad else 0


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    ref, repo, wait_min = "HEAD", None, 0
    it = iter(argv)
    try:
        for a in it:
            if a == "--wait":
                wait_min = float(next(it))
            elif a == "--repo":
                repo = next(it)
            elif a.startswith("-"):
                raise ValueError("모르는 옵션: %s" % a)
            else:
                ref = a
        sha = git("rev-parse", "--verify", ref + "^{commit}")
        repo = repo or origin_repo()
    except (StopIteration, ValueError, subprocess.CalledProcessError) as e:
        print("판정 불가(2) — 인자·저장소: %s" % e)
        return 2
    print("태그 전 CI 점검: %s @ %s (%s)" % (repo, sha, ref))
    deadline = time.time() + wait_min * 60
    while True:
        try:
            code, rows, waitable = check(repo, sha)
        except urllib.error.HTTPError as e:
            left = e.headers.get("X-RateLimit-Remaining") if e.headers else None
            print("판정 불가(2) — GitHub API HTTP %s%s" % (e.code, " · 남은 익명 한도 %s" % left if left is not None else ""))
            return 2
        except (urllib.error.URLError, TimeoutError, ValueError, OSError) as e:
            print("판정 불가(2) — %s" % e)
            return 2
        if code == 0 or not waitable or time.time() >= deadline:
            break
        pending = [r[0] for r in rows if r[1] != "success"]
        print("  … %s 기다리는 중(%d초 뒤 다시 본다)" % (", ".join(pending), POLL_S))
        time.sleep(POLL_S)
    for label, verdict, run in rows:
        print("  %-30s %s" % (label, verdict))
        if run:
            print("     %s · %s" % (run.get("html_url"), run.get("created_at")))
            if run.get("status") == "completed" and run.get("conclusion") != "success":
                explain_failure(run)
    if code == 0:
        print("판정: 0 — 같은 SHA 의 필수 워크플로가 모두 success 다(태그해도 된다)")
    else:
        print("판정: 1 — 태그 금지(초록이 아니다 · 진행 중이면 --wait 로 기다렸다 다시 확인하라)")
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
