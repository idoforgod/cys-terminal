#!/usr/bin/env python3
"""릴리스 후처리 — CI 완주 후 배포 자산을 완성한다 (2026-07-29 신설).

★배경: 릴리스 CI(`release.yml`)는 DMG 2종 · setup.exe · 업데이터 자산 · 팩 3종까지 만들지만,
**홈페이지가 쓰는 나머지 2종은 만들지 않는다**:
  · `cys_<V>_x64-setup.zip`  — 4번째 다운로드 버튼(.exe 직다운 차단 환경용)
  · `SHA256SUMS.txt`         — 전 자산 무결성 목록
실측(v0.14.2·0.14.3·0.14.4): exe 업로드 03:06 → zip·SUMS 업로드 07:03. 즉 CI 밖의 손절차였다.
이 스크립트가 그 손절차를 **재현 가능하게 고정**한다.

하는 일 (기본은 dry-run — `--apply` 를 줘야 실제로 업로드한다)
  1. GitHub 릴리스에서 전 자산 다운로드 → `~/cys-release-backup/<tag>-assets/`
  2. `make-win-zip.py` 로 zip 변형 생성(기존 발행본 바이트 재현 확인됨)
  3. `SHA256SUMS.txt` 생성 — **자기 자신을 뺀 전 자산**(과거 관례: 13자산)
  4. 자기 검증: zip 왕복 · SUMS 전 줄 재계산 대조 · 누락 0
  5. ★Gatekeeper 게이트(F2 · 2026-08-20 신설): 백업 DMG 2종 = **발행될 실물 바이트**에
     `release-gate-gatekeeper.sh`(정적 실평가)를, 네이티브 아키텍처 DMG 에는 추가로
     `verify-gatekeeper-user-path.sh`(⑥ 봉인 자기파괴 재현 포함)를 돌린다.
     rc≠0(1=FAIL·2=판정 불가) 이면 **전체 비영 종료 — --apply 거부**(측정 불능≠통과).
     macOS 밖에서는 게이트가 못 돈다 = 판정 불가로 fail-closed(무음 skip 금지).
  6. `--apply` 면 zip·SUMS 를 릴리스에 업로드

인증: `git credential fill`(osxkeychain)에서 GitHub 토큰을 얻는다. gh CLI 불요.

★수신 규율 (2026-08-27 MF-C): 모든 GitHub 호출에 **timeout** 이 걸리고, 읽기(GET·자산
  다운로드)는 **유계 재시도**(429·5xx·연결 오류/타임아웃 · 상한 MAX_ATTEMPTS)를 탄다.
  4xx 는 확정 답이라 재시도하지 않고, 상태를 바꾸는 호출(POST 업로드·DELETE)은 멱등하지
  않아 timeout 만 건다. **크기 대조 fail-closed(1단계)는 그대로다** — 재시도 대상이 아니다.
  세목은 아래 '수신 규율' 절과 `with_retries()` 주석.

사용:
  python3 scripts/release-postprocess.py v0.14.19           # dry-run(다운로드·생성·검증만)
  python3 scripts/release-postprocess.py v0.14.19 --apply   # 업로드까지
  ※예시는 **SEAL 세대 태그**다 — 5단계 게이트의 ⑤ SEAL-2 정적 검사(선컴파일 커버리지)는
    SEAL 도입(2026-08-20) 이후 빌드에서만 성립한다. pre-SEAL 태그 재후처리는 아래
    비상 플래그 없이는 구조적으로 FAIL 한다(docs/RELEASE.md 복구 절차 참조).
  옵션: --unsafe-skip-gatekeeper  Gatekeeper 게이트 생략 — 비상 탈출구(LOUD 경고·평시 금지,
        `--force-no-verify` 선례와 동형: 어떤 자동 경로도 이 플래그를 실어서는 안 된다)
"""
import hashlib
import http.client
import json
import os
import platform
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

REPO = "idoforgod/cys-terminal"
BACKUP_ROOT = os.path.expanduser("~/cys-release-backup")
SUMS_NAME = "SHA256SUMS.txt"          # ★과거 관례 — `SHA256SUMS`(확장자 없음) 아님
HERE = os.path.dirname(os.path.abspath(__file__))

# ── ★수신 규율 (2026-08-27 MF-C · 무방비 urllib 봉합) ────────────────────────
# ★배경: `api()`·`download()` 는 `urllib.request.urlopen(req)` 를 **timeout 없이·재시도 없이**
#   불렀다. 남은 두 축이 그대로였다:
#     · 일시 5xx/429 가 잡히지 않고 **트레이스백으로 죽는다** — main() 의 except 는 릴리스
#       조회 한 곳(404 폴백)만 감싼다. CI 직후 GitHub 5xx 블립 한 번이면 후처리 전체가 죽고,
#       사람은 파이썬 스택을 보고 원인을 뒤진다.
#     · 멈춘 연결은 timeout 부재로 **무기한 대기**한다(소켓 기본값 = None). 릴리스 절차가
#       hang 하면 그 자체가 사고다(`retry-loop-needs-stop-condition` 의 반대편 — 정지 조건 부재).
#
# ★규율은 이 레인의 다른 두 도구(verify-release-remote.py · deploy-homepage.py)와 같다:
#     · 재시도 대상 = **429·5xx·연결 오류/타임아웃**('지금 대답을 못 한다')
#     · 4xx(429 제외) = 원격의 **확정 답** → 재시도하지 않는다(같은 답이 올 뿐)
#     · 상한은 range() 가 강제 · 재시도 사유는 **반드시 로그로** 남긴다(침묵 금지)
#     · ★크기·해시 대조 실패는 재시도 대상이 **아니다** — 같은 바이트에 시간만 쓴다.
#       기존 fail-closed(4단계 앞 크기 대조 → `::error::` + return 1)는 **그대로 보존**한다.
API_TIMEOUT = 60                 # 릴리스 조회·삭제 1회 상한(초)
DOWNLOAD_TIMEOUT = 600           # 자산 본문 1회 상한(초) — 269MB DMG 기준
UPLOAD_TIMEOUT = 900             # 자산 업로드 1회 상한(초)
MAX_ATTEMPTS = 3                 # ★유계 — 최초 1회 + 재시도 2회
RETRY_BACKOFF_S = (2.0, 5.0)     # len ≥ MAX_ATTEMPTS-1 (부족하면 마지막 값 반복)

# 재시도 대상 예외 — 연결이 안 되거나(URLError) 멈추거나(timeout) 응답이 끊긴(IncompleteRead)
# 경우. ★HTTPError 는 URLError 의 하위형이라 **먼저** 걸러 상태코드로 가른다.
_TRANSIENT_EXC = (urllib.error.URLError, socket.timeout, TimeoutError,
                  ConnectionError, http.client.HTTPException)


def is_transient_status(status):
    """유계 재시도 대상인가 — **429 와 5xx**. 4xx(429 제외)는 원격의 확정 답이다."""
    return status == 429 or status >= 500


def with_retries(what, fn, attempts=MAX_ATTEMPTS, _sleep=None):
    """`fn()` 을 유계 재시도로 부른다 → fn 의 반환값.

    ★상한은 `range(attempts)` 가 구조적으로 강제한다(무한 루프 불가).
    ★소진하면 조용히 넘어가지 않고 `SystemExit("::error::…")` 로 **loud 하게** 죽는다 —
      후처리가 반쯤 된 상태로 '성공'을 보고하는 경로를 만들지 않기 위해서다.
    """
    sleeper = _sleep or time.sleep
    attempts = max(1, int(attempts))
    last = "?"
    for i in range(attempts):                      # ← 유계
        if i:
            delay = RETRY_BACKOFF_S[min(i - 1, len(RETRY_BACKOFF_S) - 1)]
            print("  ↻ 재시도 %d/%d (%.0fs 후) ← %s | %s"
                  % (i, attempts - 1, delay, last, what), flush=True)   # 침묵 금지
            sleeper(delay)
        try:
            return fn()
        except urllib.error.HTTPError as ex:
            if not is_transient_status(ex.code):
                raise                              # 4xx = 확정 답 — 호출부가 판단한다
            last = "HTTP %d" % ex.code
        except _TRANSIENT_EXC as ex:
            last = "%s: %s" % (type(ex).__name__, ex)
    raise SystemExit("::error::%s — 유계 재시도 %d회를 소진했다(마지막: %s)"
                     % (what, attempts, last))


def token():
    """osxkeychain 의 git 자격에서 GitHub 토큰을 얻는다(값은 출력하지 않는다)."""
    p = subprocess.run(["git", "credential", "fill"], input="protocol=https\nhost=github.com\n\n",
                       capture_output=True, text=True)
    for line in p.stdout.splitlines():
        if line.startswith("password="):
            return line[len("password="):]
    raise SystemExit("::error::GitHub 토큰을 얻지 못했다(git credential fill)")


def api(path, tok, method="GET", data=None, ctype="application/json",
        timeout=API_TIMEOUT, retry=None, _opener=None, _sleep=None):
    """GitHub API 1건. ★timeout 필수 · 재시도는 **읽기(GET)만**.

    ★왜 GET 만 재시도하는가 — DELETE/POST 는 멱등하지 않다. 서버가 처리했는데 응답만
      유실된 경우 재시도는 두 번째 호출에서 404 를 받고 그걸 확정 오류로 올린다(=없는 실패를
      만든다). 그래서 상태를 바꾸는 호출은 **timeout 만** 걸고 재시도는 걸지 않는다.
      `retry=True/False` 로 호출부가 명시적으로 뒤집을 수 있다.
    """
    if retry is None:
        retry = method in ("GET", "HEAD")
    opener = _opener or urllib.request.urlopen

    def once():
        req = urllib.request.Request("https://api.github.com" + path, method=method, data=data)
        req.add_header("Authorization", "Bearer " + tok)
        req.add_header("Accept", "application/vnd.github+json")
        if data is not None:
            req.add_header("Content-Type", ctype)
        with opener(req, timeout=timeout) as r:
            body = r.read()
        return json.loads(body) if body else {}

    if not retry:
        return once()
    return with_retries("%s %s" % (method, path), once, _sleep=_sleep)


def sha256_file(p):
    h = hashlib.sha256()
    with open(p, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def download(url, dest, tok, timeout=DOWNLOAD_TIMEOUT, _opener=None, _sleep=None):
    """릴리스 자산 1건을 내려받는다. ★timeout + 유계 재시도(429·5xx·연결 오류/타임아웃).

    ★재시도는 **처음부터 다시 받는다**(Range 이어받기 없음) — `dest` 를 매번 'wb' 로 열어
      부분 파일이 남지 않게 한다. 이어받기를 하면 '끊긴 지점이 진짜였는지'를 증명해야 하는데
      이 스크립트에는 그럴 근거가 없다.
    ★크기 대조는 **여기서 하지 않는다** — 호출부(main 1단계)의 fail-closed 가 정본이고,
      그 대조 실패는 재시도 대상이 아니다(같은 바이트에 시간만 쓴다). 그 계약을 바꾸지 않는다.
    """
    opener = _opener or urllib.request.urlopen

    def once():
        req = urllib.request.Request(url)
        req.add_header("Authorization", "Bearer " + tok)
        req.add_header("Accept", "application/octet-stream")
        with opener(req, timeout=timeout) as r, open(dest, "wb") as fh:
            while True:
                chunk = r.read(1 << 20)
                if not chunk:
                    break
                fh.write(chunk)

    return with_retries("GET %s" % url, once, _sleep=_sleep)


GATE_SCRIPT = os.path.join(HERE, "release-gate-gatekeeper.sh")
USER_PATH_GATE = os.path.join(HERE, "verify-gatekeeper-user-path.sh")


def gatekeeper_gate(outdir, version, unsafe_skip=False,
                    gate_script=GATE_SCRIPT, user_path_script=USER_PATH_GATE,
                    sys_platform=None, machine=None, run=subprocess.run):
    """발행될 실물 바이트(draft 백업 DMG 2종)에 대한 Gatekeeper 실평가 게이트 (F2).

    ★왜 여기인가: CI 게이트(release.yml:429 부근)는 **빌드 산출물**을 업로드 전에 본다.
    그러나 발행 판정의 대상은 이 스크립트가 방금 받은 **백업 자산 = 실제로 발행될 바이트**다.
    그 바이트에 대한 발행 전 로컬 실평가 지점이 없었다 — 4단계(자기 검증) 직후에 세운다.

    fail-closed 계약 (측정 불능은 통과가 아니다 — verify-gatekeeper-user-path.sh 관례):
      · 게이트 rc 1(FAIL)·2(판정 불가) 모두 그대로 비영 반환 → main 이 그 값으로 종료한다.
      · macOS 가 아니면 게이트 자체가 못 돈다(hdiutil·spctl·codesign 부재) = 판정 불가 → 2.
      · 대상 DMG 부재도 판정 불가 = 2. 무음 skip 경로는 없다.

    반환: 0=통과 · 비영=차단(--apply 거부). 키워드 인자(gate_script·user_path_script·
    sys_platform·machine·run)는 테스트 주입 전용이다 — test_release_postprocess_gate.py 가
    페이크 게이트(exit 0/1/2)로 이 계약을 박제한다.
    """
    if unsafe_skip:
        # `--force-no-verify` 선례 동형의 비상 탈출구 — 조용히 열리면 상시 우회가 되므로 LOUD.
        print("!!!! [UNSAFE] --unsafe-skip-gatekeeper — Gatekeeper 게이트를 건너뛴다: 발행될 "
              "바이트가 사용자 머신에서 열리는지 이 실행은 아무것도 증명하지 않는다.", file=sys.stderr)
        print("!!!! 비상 탈출구다(평시 금지 · 측정 불능≠통과) — 사용 사유를 릴리스 기록"
              "(SESSION_STATE·릴리스 노트)에 남겨라.", file=sys.stderr)
        return 0
    if sys_platform is None:
        sys_platform = sys.platform
    if machine is None:
        machine = platform.machine()
    if sys_platform != "darwin":
        # macOS 밖 = 게이트 불가. skip 은 곧 무검증 발행이므로 판정 불가(2)로 fail-closed 한다
        # (release-gate-gatekeeper.sh 의 exit 2 계약과 동일 — 통과가 아니다).
        print("::error::Gatekeeper 게이트는 macOS 에서만 돈다(현재 platform=%s) — 판정 불가"
              "=통과 아님. macOS 에서 다시 돌려라(비상시에만 --unsafe-skip-gatekeeper)."
              % sys_platform, file=sys.stderr)
        return 2

    # 상위판(⑥ 봉인 자기파괴 재현 = 동봉 python 실제 스폰)은 **네이티브 아키텍처 DMG 에만**.
    # 비네이티브 쪽을 정적판만으로 두는 근거: verify-gatekeeper-user-path.sh 는 대상 아키텍처
    # python 을 실행하므로 arm64 맥에서 x64 DMG 는 Rosetta 2 의존이 생기고, 없으면 구조적
    # FAIL 이다. 정적판은 실행 0으로 양쪽 결정론 — release-gate-gatekeeper.sh 머리 주석
    # 「scripts/verify-gatekeeper-user-path.sh 와의 관계」(행번호는 병렬 수정으로 유동).
    native_arch = "aarch64" if machine == "arm64" else "x64"
    print("\n═══ Gatekeeper 게이트 — 발행될 실물 바이트(draft 백업 DMG 2종) 실평가 ═══", flush=True)
    for arch in ("aarch64", "x64"):
        dmg = os.path.join(outdir, "cys_%s_%s.dmg" % (version, arch))
        if not os.path.exists(dmg):
            print("::error::게이트 대상 DMG 없음: %s — 판정 불가=통과 아님" % dmg, file=sys.stderr)
            return 2
        cmds = [["bash", gate_script, dmg]]
        if arch == native_arch:
            cmds.append(["bash", user_path_script, dmg])
        for cmd in cmds:
            print("  → %s %s" % (os.path.basename(cmd[1]), os.path.basename(dmg)), flush=True)
            rc = run(cmd).returncode
            if rc != 0:
                print("::error::Gatekeeper 게이트 차단 rc=%d (%s · %s) — 1=FAIL·2=판정 불가, "
                      "둘 다 발행 금지(--apply 거부)"
                      % (rc, os.path.basename(cmd[1]), os.path.basename(dmg)), file=sys.stderr)
                return rc
    print("  ✓ Gatekeeper 게이트 통과 — DMG 2종(네이티브 %s 는 사용자 경로 재현 ⑥ 포함)" % native_arch)
    return 0


def main(argv):
    if len(argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    tag = argv[1]
    apply_ = "--apply" in argv[2:]
    unsafe_skip = "--unsafe-skip-gatekeeper" in argv[2:]
    version = tag.lstrip("v")
    tok = token()

    # ★draft 릴리스는 `/releases/tags/<tag>` 로 조회되지 않는다(404 — 실측).
    #   목록 조회로 폴백해야 CI 직후(draft 상태)에도 후처리가 돈다.
    try:
        rel = api("/repos/%s/releases/tags/%s" % (REPO, tag), tok)
    except urllib.error.HTTPError as e:
        if e.code != 404:
            raise
        rel = None
        for page in (1, 2):
            for r in api("/repos/%s/releases?per_page=50&page=%d" % (REPO, page), tok):
                if r.get("tag_name") == tag:
                    rel = r
                    break
            if rel:
                break
        if rel is None:
            raise SystemExit("::error::릴리스 %s 를 찾지 못했다(draft 포함 조회 실패)" % tag)
        print("  (draft — 목록 조회로 찾음)")
    print("릴리스 %s — draft=%s · 자산 %d종" % (tag, rel.get("draft"), len(rel.get("assets", []))))

    outdir = os.path.join(BACKUP_ROOT, tag + "-assets")
    os.makedirs(outdir, exist_ok=True)

    # ── 1. 전 자산 다운로드 ──
    by_name = {}
    for a in rel.get("assets", []):
        dest = os.path.join(outdir, a["name"])
        if os.path.exists(dest) and os.path.getsize(dest) == a["size"]:
            print("  (캐시) %-34s %12d" % (a["name"], a["size"]))
        else:
            print("  받는 중 %-34s %12d …" % (a["name"], a["size"]), flush=True)
            download(a["url"], dest, tok)
            got = os.path.getsize(dest)
            if got != a["size"]:
                print("::error::크기 불일치 %s: %d != %d" % (a["name"], got, a["size"]), file=sys.stderr)
                return 1
        by_name[a["name"]] = dest

    # ── 2. zip 변형 (없으면 생성) ──
    exe = "cys_%s_x64-setup.exe" % version
    zipname = "cys_%s_x64-setup.zip" % version
    if exe not in by_name:
        print("::error::%s 가 릴리스에 없다 — CI 완주를 먼저 확인하라" % exe, file=sys.stderr)
        return 1
    zippath = os.path.join(outdir, zipname)
    if zipname in by_name:
        print("  zip 이미 릴리스에 있음 — 재생성 생략")
    else:
        r = subprocess.run([sys.executable, os.path.join(HERE, "make-win-zip.py"),
                            by_name[exe], zippath])
        if r.returncode != 0:
            return 1
        by_name[zipname] = zippath

    # ── 3. SHA256SUMS.txt — 자기 자신 제외 전 자산 ──
    names = sorted(n for n in by_name if n != SUMS_NAME)
    lines = ["%s  %s\n" % (sha256_file(by_name[n]), n) for n in names]
    sums_path = os.path.join(outdir, SUMS_NAME)
    with open(sums_path, "w") as fh:
        fh.writelines(lines)
    print("  %s 생성 — %d자산" % (SUMS_NAME, len(names)))

    # ── 4. 자기 검증 ──
    bad = 0
    for line in lines:
        want, name = line.split("  ", 1)
        name = name.strip()
        if sha256_file(by_name[name]) != want:
            print("::error::SUMS 불일치: %s" % name, file=sys.stderr)
            bad += 1
    if bad:
        return 1
    # 홈페이지 4종이 전부 들어 있는가(누락 0 — 오너 지시 ⓑ)
    want4 = ["cys_%s_aarch64.dmg" % version, "cys_%s_x64.dmg" % version, exe, zipname]
    missing = [w for w in want4 if w not in by_name]
    if missing:
        print("::error::배포 4종 누락: %s" % ", ".join(missing), file=sys.stderr)
        return 1
    print("  ✓ 자기 검증 통과 — 전 줄 재계산 일치 · 배포 4종 누락 0")

    # ── 5. Gatekeeper 게이트 (F2) — 발행될 실물 바이트를 dry-run·--apply 공통으로 실평가 ──
    #    ★업로드(6단계)·dry-run 성공 판정보다 반드시 앞: rc≠0 이면 여기서 전체가 비영 종료라
    #      --apply 는 게이트를 우회할 수 없다(fail-closed · 측정 불능≠통과).
    gate_rc = gatekeeper_gate(outdir, version, unsafe_skip=unsafe_skip)
    if gate_rc:
        return gate_rc

    # ── 6. 업로드 ──
    if not apply_:
        print("\n[dry-run] 업로드하지 않았다. 실제 업로드는 --apply")
        print("산출물: %s" % outdir)
        return 0

    up_base = rel["upload_url"].split("{")[0]
    existing = {a["name"]: a["id"] for a in rel.get("assets", [])}
    for name, ctype in ((zipname, "application/zip"), (SUMS_NAME, "text/plain")):
        if name in existing:      # --clobber 동등: 기존 자산 삭제 후 재업로드
            api("/repos/%s/releases/assets/%d" % (REPO, existing[name]), tok, method="DELETE")
        with open(os.path.join(outdir, name), "rb") as fh:
            body = fh.read()
        req = urllib.request.Request(up_base + "?name=" + name, method="POST", data=body)
        req.add_header("Authorization", "Bearer " + tok)
        req.add_header("Content-Type", ctype)
        # ★timeout 은 걸고 재시도는 걸지 않는다 — 업로드 POST 는 멱등하지 않다(서버가 받았는데
        #   응답만 유실되면 재시도가 중복 자산·422 를 만든다). 멈춘 연결로 후처리가 무기한
        #   hang 하는 것만 막는다. 실패는 예외로 올라가 loud 하게 죽는다(조용한 성공 금지).
        with urllib.request.urlopen(req, timeout=UPLOAD_TIMEOUT) as r:
            r.read()
        print("  ✓ 업로드 %s (%d bytes)" % (name, len(body)))
    print("\n✅ 후처리 완료 — %s" % outdir)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
