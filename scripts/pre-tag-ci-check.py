#!/usr/bin/env python3
"""pre-tag-ci-check.py — 태그 전 점검(P2): 태그할 커밋의 브랜치 CI 가 **같은 SHA 에서 초록**인지 확인한다.

★왜 존재하는가(2026-09-11 · v0.14.34 윈도우 빌드 파손): 브랜치 push 10:49:46Z → 태그 push 10:50:44Z(58초 뒤).
  같은 커밋(88c1ca2)의 브랜치 windows-build 는 11:00:31Z 에 failure 였는데, 태그 레인(release.yml)은 브랜치
  런의 결과를 보지 않는다. 태그를 CI 결과 **뒤에** 두는 장치가 없어 파손본이 그대로 태그됐다.
  이 도구가 docs/RELEASE.md §0-C 태그 전 사전 게이트 3번이다 — 초록이 아니면 태그하지 않는다.

★gh 불필요: 공개 저장소의 Actions 런·잡 목록은 인증 없이 조회된다(익명 한도 시간당 60회 · --wait 는 60초 간격).
  실패한 런은 잡·스텝 이름과 check-run annotation(익명으로 읽히는 유일한 실패 사유 채널)을 함께 보여 준다.
★성공한 런도 그 잡의 check-run annotation 가운데 `failure`·`warning` 수준이 있으면 **제목을 요약해 보여 준다**(0.14.43 성찰 R2F-PK · A4 m1):
  `continue-on-error` 스텝(예 windows-health 의 cysd 전량 · hwmon 스텝)의 실패는 런·잡 결론이 success 라 이 판정이 볼 수 없고, 진짜 결과는 annotation 에만 있다 —
  요약은 **종료 코드를 바꾸지 않는다**(비차단 실패가 태그 전에 사람 눈에 들어오게 할 뿐). 못 읽으면 "annotation 을 읽지 못했다" 한 줄(판정은 그대로).
  비용: 성공한 런마다 잡 목록 1회 + 잡마다 annotation 1회를 더 읽는다(익명 한도 시간당 60회 안).

사용: python3 scripts/pre-tag-ci-check.py [<SHA 또는 ref>] [--wait <분>] [--repo <owner/name>]
      python3 scripts/pre-tag-ci-check.py --self-test      # 판정 규칙 자기 검체(네트워크 0)
  SHA 기본 = HEAD · repo 기본 = git remote origin. --wait N 은 진행 중·런 없음 상태를 최대 N분 다시 본다.
exit 0 = 필수 워크플로(ci-branch · windows-build · windows-health) 셋 다 같은 SHA 에서 success → 태그해도 된다
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
# ★windows-health(R2F-PK · A4 m1): 태그 레인(release.yml `windows-health-gate`)이 **같은 SHA 의 windows-health 완주 런이 전부 success** 를 요구한다 —
#   이 사전 게이트가 그 워크플로를 안 보면 사전 게이트 4종이 전부 rc 0 인데도 태그 레인이 pack-artifacts 를 만들지 않고 죽는다(태그는 불변 → 버전 소모). 같은 판정을 태그 **전에** 한다.
#   ※ 알려진 차이(고치지 않았다 — 범위 밖): 태그 레인은 같은 SHA 의 완주 런 **전부**가 success 여야 하지만 이 판정은 워크플로마다 **가장 최근 런**만 본다. 같은 SHA 에 windows-health 런이 둘 이상(별도 dispatch)이고
#   옛 런이 적색이면 여기서는 초록·태그 레인에서는 적색이다(같은 런의 재실행 attempt 는 런 id 가 같아 최신 결론만 남으므로 해당 없다).
REQUIRED = (
    (".github/workflows/ci-branch.yml", "ci-branch"),
    (".github/workflows/windows-build.yml", "windows-build (feasibility)"),
    (".github/workflows/windows-health.yml", "windows-health (H-WIN 실기)"),
)
SUMMARY_MAX = 15   # 성공한 런의 failure·warning annotation 요약에 보이는 최대 묶음 수(넘으면 '외 N묶음')
ANN_ORDER = {"failure": 0, "warning": 1}   # 요약 순서 — failure 가 먼저(요약에 세는 수준은 이 둘뿐이다)
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


def annotation_summary(anns):
    """순수 요약(네트워크 0) — failure·warning 수준 annotation 을 (수준, 제목) 로 묶어 [(수준, 제목, 건수)] 로 돌려준다(failure 먼저 · 같은 수준 안에서는 제목순).
    제목이 없는 annotation(러너 경고 등)은 메시지 첫 줄로 대신한다. notice 와 모르는 수준은 센 적 없다."""
    seen = {}
    for a in anns or []:
        if not isinstance(a, dict):
            continue
        lvl = a.get("annotation_level")
        if lvl not in ANN_ORDER:
            continue
        title = (a.get("title") or "").strip()
        if not title:
            first = (a.get("message") or "").strip().splitlines()
            title = "(제목 없음) " + (first[0].strip() if first else "")
        key = (lvl, title[:120] + ("…" if len(title) > 120 else ""))
        seen[key] = seen.get(key, 0) + 1
    return sorted(((l, t, n) for (l, t), n in seen.items()), key=lambda r: (ANN_ORDER[r[0]], r[1]))


def explain_warnings(run, getter=None, out=print):
    """성공한 런의 잡마다 check-run annotation 을 읽어 failure·warning 수준이 있으면 제목을 요약해 보인다 — **종료 코드는 바꾸지 않는다**(보조 설명).
    API 접근은 explain_failure 와 같은 방식(잡 목록 `jobs_url` → 잡의 `check_run_url` + `/annotations`)이다. 어디서든 못 읽으면 "annotation 을 읽지 못했다" 한 줄을 남기고 끝낸다(예외를 밖으로 내지 않는다)."""
    getter = getter or get
    try:
        jobs = getter(run["jobs_url"] + "?per_page=100").get("jobs", [])
    except Exception as e:
        out("     (annotation 을 읽지 못했다 — 잡 목록 조회 실패: %s)" % e)
        return
    groups, unread = {}, 0   # (수준, 제목) → [건수, [그 annotation 이 있는 잡 이름들]] — 같은 경고가 잡마다 반복돼도(예 러너의 Node.js 안내) 한 줄로 접는다
    for j in jobs:
        try:
            anns = getter(j["check_run_url"] + "/annotations?per_page=100")
        except Exception:
            unread += 1
            continue
        for lvl, title, n in annotation_summary(anns):
            g = groups.setdefault((lvl, title), [0, []])
            g[0] += n
            if j.get("name") not in g[1]:
                g[1].append(j.get("name"))
    found = sorted(groups.items(), key=lambda kv: (ANN_ORDER[kv[0][0]], kv[0][1]))
    if unread:
        out("     (annotation 을 읽지 못했다 — 잡 %d/%d개 · 아래는 읽은 잡만의 요약이다)" % (unread, len(jobs)))
    if found:
        out("     주의: 성공(success)한 런이지만 failure·warning annotation 이 있다 — 비차단 스텝의 실패일 수 있다(종료 코드는 바뀌지 않는다 · 태그 전에 눈으로 확인하라):")
        for (lvl, title), (n, names) in found[:SUMMARY_MAX]:
            out("       · [%s ×%d] %s — 잡 %s" % (lvl, n, title, ", ".join(str(x) for x in names)))
        if len(found) > SUMMARY_MAX:
            out("       · … 외 %d묶음" % (len(found) - SUMMARY_MAX))
    elif not unread:
        out("     annotation 점검: failure·warning 없음(잡 %d개)" % len(jobs))


def check(repo, sha):
    data = get("%s/repos/%s/actions/runs?head_sha=%s&per_page=100" % (API, repo, sha))
    runs = data.get("workflow_runs")
    if not isinstance(runs, list):
        raise ValueError("응답 형식이 예상과 다르다(workflow_runs 없음)")
    return decide(runs)


def self_test():
    def run(path, st, cc, t, i):
        return {"path": path, "status": st, "conclusion": cc, "created_at": t, "id": i}
    cb, wb, wh = (p for p, _ in REQUIRED)
    ok = lambda p, t="1", i=1: run(p, "completed", "success", t, i)
    cases = [
        ("셋 다 success", [ok(cb), ok(wb, i=2), ok(wh, i=3)], 0),
        ("windows-build failure — v0.14.34 형태", [ok(cb), run(wb, "completed", "failure", "1", 2), ok(wh, i=3)], 1),
        ("진행 중은 초록이 아니다", [ok(cb), run(wb, "in_progress", None, "1", 2), ok(wh, i=3)], 1),
        ("런 없음은 초록이 아니다", [ok(cb), ok(wh, i=3)], 1),
        ("옛 실패 뒤 재실행 success = 초록", [ok(cb), run(wb, "completed", "failure", "1", 2), ok(wb, "2", 3), ok(wh, i=4)], 0),
        ("옛 success 뒤 새 런 failure = 적색", [ok(cb), ok(wb, i=2), run(wb, "completed", "failure", "2", 3), ok(wh, i=4)], 1),
        ("cancelled 는 초록이 아니다", [run(cb, "completed", "cancelled", "1", 1), ok(wb, i=2), ok(wh, i=3)], 1),
        # 필수가 아닌 워크플로의 초록은 세지 않는다 — windows-health 는 이제 필수라서 다른 워크플로(release·pack-release)로 이 행의 뜻을 지킨다(R2F-PK)
        ("다른 워크플로의 초록은 세지 않는다", [ok(".github/workflows/release.yml"), ok(".github/workflows/pack-release.yml", i=2), ok(cb, i=3)], 1),
        # ── windows-health(R2F-PK · A4 m1) — 태그 레인이 같은 SHA 의 windows-health 를 요구한다 ──
        ("windows-health 런 없음 = 태그 금지(ci-branch·windows-build 가 초록이어도)", [ok(cb), ok(wb, i=2)], 1),
        ("windows-health failure = 태그 금지(다른 둘이 초록이어도)", [ok(cb), ok(wb, i=2), run(wh, "completed", "failure", "1", 3)], 1),
        ("windows-health 진행 중 = 초록이 아니다", [ok(cb), ok(wb, i=2), run(wh, "in_progress", None, "1", 3)], 1),
        ("windows-health cancelled = 초록이 아니다", [ok(cb), ok(wb, i=2), run(wh, "completed", "cancelled", "1", 3)], 1),
    ]
    bad = 0
    for name, runs, want in cases:
        got = decide(runs)[0]
        print("%s | %s | 기대 %d · 실제 %d" % ("PASS" if got == want else "FAIL", name, want, got))
        bad += got != want
    # 필수 워크플로 목록 자체도 못박는다 — 하나가 조용히 빠지면(예 windows-health) 위 행들이 붉어지지만, 이름·경로 오타는 여기서 잡는다.
    req_ok = [p for p, _ in REQUIRED] == [".github/workflows/ci-branch.yml", ".github/workflows/windows-build.yml", ".github/workflows/windows-health.yml"]
    print("%s | 필수 워크플로 = ci-branch · windows-build · windows-health(태그 레인 windows-health-gate 와 같은 SHA 요구)" % ("PASS" if req_ok else "FAIL"))
    bad += not req_ok

    # ── 성공한 런의 failure·warning annotation 요약(R2F-PK · A4 m1) — 네트워크 0 · 가짜 getter ──
    def fake(table):
        def g(url):
            if url not in table:
                raise OSError("HTTP 403 (가짜)")
            v = table[url]
            if isinstance(v, Exception):
                raise v
            return v
        return g

    def lines_of(table, run_obj=None):
        buf = []
        explain_warnings(run_obj or {"jobs_url": "J"}, getter=fake(table), out=buf.append)
        return buf

    J = "J?per_page=100"
    jobs2 = {"jobs": [{"name": "win-health", "check_run_url": "C1"}, {"name": "ci-misc", "check_run_url": "C2"}]}
    anns = [
        {"annotation_level": "failure", "title": "cysd-win 완주·실패(비차단)", "message": "rc=101"},
        {"annotation_level": "failure", "title": "cysd-win 완주·실패(비차단)", "message": "again"},
        {"annotation_level": "notice", "title": "hwmon-win 완주·통과(비차단)", "message": "ok"},
        {"annotation_level": "warning", "title": None, "message": "Node.js 20 actions are deprecated.\n두 번째 줄"},
        {"annotation_level": "mystery", "title": "모르는 수준", "message": "x"},
    ]
    checks = []
    got = lines_of({J: jobs2, "C1/annotations?per_page=100": anns, "C2/annotations?per_page=100": []})
    txt = "\n".join(got)
    checks.append(("failure 제목이 건수·잡 이름과 함께 요약에 든다", "[failure ×2] cysd-win 완주·실패(비차단) — 잡 win-health" in txt))
    checks.append(("제목 없는 warning 은 메시지 첫 줄로 대신한다", "[warning ×1] (제목 없음) Node.js 20 actions are deprecated. — 잡 win-health" in txt))
    checks.append(("notice·모르는 수준은 요약에 없다", "hwmon-win" not in txt and "모르는 수준" not in txt))
    checks.append(("failure 가 warning 보다 먼저 나온다", txt.index("[failure") < txt.index("[warning")))
    checks.append(("요약은 '종료 코드는 바뀌지 않는다'를 밝힌다", "종료 코드는 바뀌지 않는다" in txt))
    got = lines_of({J: jobs2, "C1/annotations?per_page=100": [{"annotation_level": "notice", "title": "t", "message": "m"}], "C2/annotations?per_page=100": []})
    checks.append(("notice 만 있으면 '없음' 한 줄", got == ["     annotation 점검: failure·warning 없음(잡 2개)"]))
    got = lines_of({})
    checks.append(("잡 목록을 못 읽으면 '읽지 못했다' 한 줄", len(got) == 1 and "annotation 을 읽지 못했다" in got[0]))
    got = lines_of({J: jobs2, "C1/annotations?per_page=100": anns})   # C2 는 못 읽는다
    checks.append(("일부 잡만 못 읽으면 읽은 요약 + 읽지 못했다 한 줄", any("annotation 을 읽지 못했다 — 잡 1/2개" in l for l in got) and any("[failure ×2]" in l for l in got)))
    same = [{"annotation_level": "warning", "title": "러너 경고", "message": "m"}]
    got = lines_of({J: jobs2, "C1/annotations?per_page=100": same, "C2/annotations?per_page=100": same})
    checks.append(("같은 경고가 여러 잡에 있으면 한 줄로 접고 잡 이름을 모두 적는다", len([l for l in got if "러너 경고" in l]) == 1 and any("[warning ×2] 러너 경고 — 잡 win-health, ci-misc" in l for l in got)))
    big = [{"annotation_level": "failure", "title": "t%02d" % i, "message": "m"} for i in range(SUMMARY_MAX + 4)]
    got = lines_of({J: {"jobs": [{"name": "j", "check_run_url": "C1"}]}, "C1/annotations?per_page=100": big})
    checks.append(("묶음이 많으면 최대 %d 개 + '외 N묶음'" % SUMMARY_MAX, len([l for l in got if l.strip().startswith("· [failure")]) == SUMMARY_MAX and any("외 4묶음" in l for l in got)))
    checks.append(("failure 가 warning 보다 먼저 나온다(잡이 달라도 전역 순서)", (lambda g: g.index(next(l for l in g if "[failure" in l)) < g.index(next(l for l in g if "[warning" in l)))(lines_of({J: jobs2, "C1/annotations?per_page=100": same, "C2/annotations?per_page=100": [{"annotation_level": "failure", "title": "f", "message": "m"}]}))))
    try:
        explain_warnings({}, getter=fake({}), out=lambda s: None)   # run 에 jobs_url 이 없어도(KeyError) 예외를 내지 않는다
        checks.append(("잘못된 run 객체에도 예외를 밖으로 내지 않는다", True))
    except Exception:
        checks.append(("잘못된 run 객체에도 예외를 밖으로 내지 않는다", False))
    for name, okk in checks:
        print("%s | %s" % ("PASS" if okk else "FAIL", name))
        bad += not okk
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
            elif run.get("status") == "completed":
                explain_warnings(run)   # 성공한 런의 failure·warning annotation — 비차단 실패가 태그 전에 눈에 들어오게(종료 코드 불변)
    if code == 0:
        print("판정: 0 — 같은 SHA 의 필수 워크플로가 모두 success 다(태그해도 된다 · 위 annotation 요약에 failure 가 있으면 비차단 실패이니 눈으로 확인하라)")
    else:
        print("판정: 1 — 태그 금지(초록이 아니다 · 진행 중이면 --wait 로 기다렸다 다시 확인하라)")
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
