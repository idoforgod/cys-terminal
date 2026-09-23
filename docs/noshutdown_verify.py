#!/usr/bin/env python3
"""무중단(재시작 0) 팩 업데이트 실측 E2E — DESIGN-noshutdown-pack-update §5-2·§7-⑥.

라이브 cysd에 `cys pack-update --from <테스트팩>`을 가해 **반영 전/후 스냅샷 동등성**을 검증한다:

  | 생존 대상 | RPC(실측)         | 읽는 필드                     | 합격 조건            |
  |-----------|-------------------|-------------------------------|----------------------|
  | cysd      | system.identify   | daemon_pid                    | 전/후 동일           |
  | 세션      | surface.list      | surfaces[].surface_id·exited  | 집합 불변·전부 false |
  | 팩 반영   | 파일              | <pack_dir>/.pack-version      | 새 버전으로 범프     |

★불합격(hard fail): daemon_pid 변동 = 데몬 재시작 = 무중단 위반 = FAIL.
  --app-pid 를 주면 cys-app(Tauri) OS 프로세스 pid·기동시각 동일성(재시작 0)도 검증한다.

★노드 각성 검증: pack-update 후 `system.topology`의 surface별 `pack_reinject` 마커가
  새 pack_version으로 갱신됐는지 확인한다(부분/낡은 각성 = FAIL). 디렉티브 해시 불변 시
  reinject는 스킵되므로(설계 §7-② step1) 평시엔 마커 변동 없음을 정상으로 본다.

★release 각성 hard gate(--require-awaken·--require-live): 평시 lenient 판정만으로는
  '모든 busy 노드가 deferred되어 실제로 아무도 새 지침을 못 받은' 무각성 상태가 pid/surface/
  .pack-version 통과로 새어나간다(codex R2 #2). 이를 막기 위해 게이트 모드에서는:
    ① pack-update stdout의 PACK_UPDATE_RESULT 토큰을 파싱해 failed==0 AND deferred==0 요구
       (deferred>0 또는 failed>0 → FAIL, 토큰 부재 → FAIL=각성 증명 불가).
    ② directive-changing 팩(테스트 팩 manifest의 directive_hash_changed≠false)은 최소 1개의
       pack_reinject 마커가 새 pack_version으로 각성됐는지 요구. skill-only(reinject 불요) 팩은
       manifest에 directive_hash_changed=false 명시 신호가 있어야 마커 무변경을 통과 허용한다
       (명시 신호 없는 무변경 통과 금지).

출력·exit code는 SKIP/PASS/FAIL을 명확히 구분한다:
  - PASS  → exit 0   (모든 검사 통과)
  - FAIL  → exit 1   (검사 1건 이상 실패 = 무중단/각성 위반)
  - SKIP  → exit 77  (라이브 데몬/테스트 팩 부재 — 평시 graceful skip, pass와 구분)

릴리스/승인용 hard gate 모드:
  --require-live   → 라이브 cysd 부재 = skip 아니라 FAIL(non-zero). 각성 hard gate도 켠다.
  --require-pack   → 테스트 팩(--from) 부재/불완전 = skip 아니라 FAIL(non-zero).
  --require-awaken → PACK_UPDATE_RESULT failed==0 AND deferred==0 + directive-changing 팩
                     마커 각성을 release hard gate로 요구(deferred-only/미각성 통과 차단).
  --self-test      → 라이브 데몬 없이 파싱·게이트 판정 로직만 단위 검증(exit 0/1).

실행:
  python3 docs/noshutdown_verify.py --from /path/to/testpack   # pack.tar.gz+manifest+.minisig
  python3 docs/noshutdown_verify.py --from ... --require-live --require-pack   # 릴리스 게이트
  python3 docs/noshutdown_verify.py --from ... --app-pid 12345                 # Tauri 앱 동일성도
  CYS_SOCKET=... CYS_PACK_DIR=... python3 docs/noshutdown_verify.py --from ...
"""
import argparse
import json
import os
import socket
import subprocess
import sys


def default_socket():
    """src/lib.rs::socket_path()와 동일 규칙 — CYS_SOCKET 우선, 없으면 ~/.local/state/cys/cys.sock."""
    for k in ("CYS_SOCKET", "JAVIS_SOCKET", "AITERM_SOCKET"):
        v = os.environ.get(k)
        if v:
            return v
    home = os.path.expanduser("~")
    return os.path.join(home, ".local", "state", "cys", "cys.sock")


def default_pack_dir():
    """src/pack.rs::pack_dir()와 동일 규칙 — CYS_PACK_DIR 우선, 없으면 ~/.cys/pack."""
    for k in ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_JARVIS_DIR"):
        v = os.environ.get(k)
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys", "pack")


def find_cys_bin():
    """cys 바이너리 — target/release > target/debug > PATH."""
    here = os.path.dirname(os.path.abspath(__file__))
    for cand in (
        os.path.join(here, "..", "target", "release", "cys"),
        os.path.join(here, "..", "target", "debug", "cys"),
    ):
        if os.path.isfile(cand) and os.access(cand, os.X_OK):
            return os.path.abspath(cand)
    return "cys"  # PATH 폴백


class DaemonUnavailable(Exception):
    pass


def rpc(sock_path, method, params, timeout=5.0):
    """Unix 소켓 1-shot JSON-RPC(개행 종단) — docs/*_e2e.py 패턴."""
    try:
        s = socket.socket(socket.AF_UNIX)
        s.settimeout(timeout)
        s.connect(sock_path)
    except OSError as e:
        raise DaemonUnavailable(f"소켓 연결 실패 {sock_path}: {e}")
    try:
        s.sendall((json.dumps({"id": 1, "method": method, "params": params}) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
    finally:
        s.close()
    if not buf:
        raise DaemonUnavailable(f"빈 응답({method})")
    resp = json.loads(buf)
    if "error" in resp and resp["error"]:
        raise RuntimeError(f"RPC 오류({method}): {resp['error']}")
    return resp.get("result", resp)


def read_pack_version(pack_dir):
    p = os.path.join(pack_dir, ".pack-version")
    try:
        with open(p) as f:
            return f.read().strip()
    except OSError:
        return ""


def snapshot(sock_path, pack_dir):
    """무중단 불변식 스냅샷: daemon_pid + {surface_id: exited} + .pack-version."""
    ident = rpc(sock_path, "system.identify", {})
    daemon_pid = ident.get("daemon_pid")
    surfaces = rpc(sock_path, "surface.list", {}).get("surfaces", [])
    surf = {int(s["surface_id"]): bool(s.get("exited", False)) for s in surfaces if "surface_id" in s}
    return {
        "daemon_pid": daemon_pid,
        "surfaces": surf,
        "pack_version": read_pack_version(pack_dir),
    }


def parse_semver(v):
    """src/pack.rs::parse_semver와 동일 규칙 — 'v' 접두 제거, '-'/'+' suffix 분리, major 결측=None."""
    v = (v or "").strip().lstrip("v")
    parts = v.split(".")
    if not parts or parts[0] == "":
        return None
    out = []
    for i in range(3):
        seg = parts[i] if i < len(parts) else "0"
        seg = seg.split("-")[0].split("+")[0]
        if not seg.isdigit():
            if i == 0:
                return None
            seg = "0"
        out.append(int(seg))
    return tuple(out)


def version_bumped(before, after):
    """after가 before보다 strictly-newer(semver). 둘 다 파싱되면 비교, 아니면 문자열 상이로 폴백."""
    b, a = parse_semver(before), parse_semver(after)
    if a is not None and b is not None:
        return a > b
    return bool(after) and after != before


SKIP_EXIT = 77  # skip을 pass(0)·fail(1)과 명확히 구분하는 종료코드.


def proc_alive(pid):
    """pid 프로세스 생존 여부 — signal 0(존재 검사, 미전달)."""
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True  # 존재하나 시그널 권한 없음 = 살아있음.
    except OSError:
        return False


def proc_starttime(pid):
    """프로세스 기동시각(ps lstart) — 동일 pid 재사용(재시작) 판별용. 실패 시 빈 문자열."""
    try:
        out = subprocess.run(
            ["ps", "-o", "lstart=", "-p", str(pid)],
            capture_output=True, text=True, timeout=5,
        )
        return out.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


def topology_markers(sock_path):
    """system.topology의 saved 엔트리에서 {키: pack_version} reinject 마커 맵 추출.

    키 = session_id 우선(없으면 role) — surface별 마커를 안정 식별. RPC 미지원/오류 시 None.
    """
    try:
        topo = rpc(sock_path, "system.topology", {})
    except (DaemonUnavailable, RuntimeError, OSError):
        return None
    markers = {}
    for e in topo.get("saved", []) or []:
        if not isinstance(e, dict):
            continue
        pr = e.get("pack_reinject")
        if isinstance(pr, dict) and pr.get("pack_version"):
            key = e.get("session_id") or e.get("role") or repr(sorted(e.items()))
            markers[key] = pr.get("pack_version")
    return markers


REINJECT_RESULT_PREFIX = "PACK_UPDATE_RESULT"  # src/pack.rs::REINJECT_RESULT_PREFIX와 동일.


def parse_pack_update_result(stdout):
    """pack-update stdout의 안정 토큰 1줄을 파싱 → counts dict 또는 None(토큰 부재).

    토큰(수치 팔): `PACK_UPDATE_RESULT pack_version=X injected=N skipped=N deferred=N failed=N`
    (src/pack.rs / src-tauri parse_reinject_counts와 동일 토큰).
    토큰(스킵 팔 · U4-B2③ · review1 M1 FIX): `PACK_UPDATE_RESULT pack_version=X reinject=skipped
    reason=Y` — 재주입 RPC 자체가 실패해 수치를 재지 못한 경우(`failed=`/`deferred=` 키가 없다).
    `reinject`/`reason` 키를 실어 반환하고, 수치 키는 종전대로 기본값 0으로 채운다 — 이 dict 만 보고
    "failed=0 deferred=0 이니 성공"으로 오판하지 않도록 `awaken_gate_failures`가 `reinject=="skipped"`를
    별도로 검사한다(수치 0 을 성공으로 읽지 않는다). 사람용 메시지와 독립한 안정 토큰만 신뢰한다 —
    카운트 파싱 실패 토큰은 0으로 보수 처리.
    """
    for line in stdout.splitlines():
        line = line.strip()
        if line.startswith(REINJECT_RESULT_PREFIX):
            rest = line[len(REINJECT_RESULT_PREFIX):]
            out = {
                "pack_version": None, "injected": 0, "skipped": 0, "deferred": 0, "failed": 0,
                "reinject": None, "reason": None,
            }
            for tok in rest.split():
                k, sep, v = tok.partition("=")
                if not sep:
                    continue
                if k == "pack_version":
                    out["pack_version"] = v
                elif k in ("reinject", "reason"):
                    out[k] = v
                elif k in ("injected", "skipped", "deferred", "failed"):
                    try:
                        out[k] = int(v)
                    except ValueError:
                        pass
            return out
    return None


def read_manifest_signal(from_dir):
    """테스트 팩 manifest의 directive_hash_changed 명시 신호 → True/False/None(미지정).

    skill-only(reinject 불요) 팩만 false를 명시해 '마커 무변경' 통과를 허용받는다. 신호 부재(None)는
    '명시 신호 없는 무변경 통과 금지'에 따라 디렉티브 변경 팩과 동일하게 마커 각성을 요구받는다.
    (PackManifest는 #[serde(default)]로 미지의 필드를 무시하므로 서명/검증과 무관하게 공존 가능.)
    """
    if not from_dir:
        return None
    p = os.path.join(from_dir, "pack-manifest.json")
    try:
        with open(p) as f:
            m = json.load(f)
    except (OSError, ValueError):
        return None
    v = m.get("directive_hash_changed")
    return v if isinstance(v, bool) else None


def awaken_gate_failures(pack_result, directive_changed, markers_available, bump_count, new_ver):
    """release 각성 hard gate 판정(순수 함수) — 실패 사유 리스트 반환(빈 리스트=통과).

    pack_result      : parse_pack_update_result 결과(None=토큰 부재 → 각성 증명 불가 → FAIL)
    directive_changed: manifest directive_hash_changed 신호(True=디렉티브 변경/False=skill-only/None=미지정)
    markers_available: system.topology 마커 관측 가능 여부(bool)
    bump_count       : 새 pack_version으로 각성한 pack_reinject 마커 수
    new_ver          : 새 pack_version 문자열(메시지용)
    """
    fails = []
    # ① PACK_UPDATE_RESULT: failed==0 AND deferred==0 — deferred-only/미각성 통과 차단.
    if pack_result is None:
        fails.append("PACK_UPDATE_RESULT 토큰 없음(구버전 사이드카·reinject 미실행 — 각성 증명 불가)")
    elif pack_result.get("reinject") == "skipped":
        # ★review1 M1 FIX(blocking): 재주입 RPC 자체가 실패한 B2 Err 팔은 `reinject=skipped`
        # 토큰을 찍되 `failed=`/`deferred=`는 싣지 않는다(재지 않은 수치를 0으로 위장하지 않음 —
        # cys.rs pack_update_reinject_skipped_line). 이 dict 의 failed/deferred 기본값 0 을 그대로
        # "완전 성공"으로 읽으면 v0.14.40 에서는 FAIL 이었던 것이 B2 에서 조용히 PASS 로 샌다
        # (실측: skill-only 팩이면 ② 축이 조기 반환해 게이트 전체가 PASS). reinject=skipped 는
        # failed/deferred 수치와 무관하게 그 자체로 "각성 증명 불가"다.
        reason = pack_result.get("reason") or "unknown"
        fails.append(f"reinject skipped(reason={reason}) — 재주입 RPC 실패로 각성 증명 불가(디스크만 갱신, 라이브 노드 미확인)")
    else:
        if pack_result["failed"] != 0:
            fails.append(f"reinject failed={pack_result['failed']}")
        if pack_result["deferred"] != 0:
            fails.append(f"reinject deferred={pack_result['deferred']} (busy 노드 미각성 = 릴리스 차단)")
    # ② directive-changing sentinel — skill-only(false 명시)만 마커 무변경 허용.
    if directive_changed is False:
        return fails  # skill-only 팩: 마커 무변경 정상.
    sig = "directive_hash_changed=true" if directive_changed else "directive_hash_changed 신호 없음"
    if not markers_available:
        fails.append(f"{sig}인데 system.topology 미응답으로 마커 각성 확인 불가")
    elif bump_count < 1:
        fails.append(f"{sig}인데 새 버전({new_ver!r}) 각성 마커 0개 — 미각성/무변경 통과 금지")
    return fails


def run_self_test():
    """라이브 데몬 없이 파싱·게이트 판정 로직 단위 검증(--self-test). exit 0=전건 통과."""
    failures = []

    def expect(name, cond):
        print(f"[{'PASS' if cond else 'FAIL'}] self-test: {name}")
        if not cond:
            failures.append(name)

    # PACK_UPDATE_RESULT 파싱.
    r = parse_pack_update_result(
        "noise\nPACK_UPDATE_RESULT pack_version=2.0.0 injected=2 skipped=1 deferred=3 failed=4\ntail")
    expect("parse pack_version=2.0.0", r is not None and r["pack_version"] == "2.0.0")
    expect("parse injected=2 skipped=1", r["injected"] == 2 and r["skipped"] == 1)
    expect("parse deferred=3 failed=4", r["deferred"] == 3 and r["failed"] == 4)
    expect("parse 토큰 부재 → None", parse_pack_update_result("no token here\nfoo=bar") is None)
    clean = parse_pack_update_result(
        "PACK_UPDATE_RESULT pack_version=1.2.3 injected=5 skipped=0 deferred=0 failed=0")
    expect("parse clean failed=0 deferred=0", clean["failed"] == 0 and clean["deferred"] == 0)

    # 각성 게이트 판정.
    expect("clean + skill-only(False) → PASS",
           awaken_gate_failures(clean, False, True, 0, "1.2.3") == [])
    deferred = parse_pack_update_result("PACK_UPDATE_RESULT pack_version=1.2.3 deferred=2 failed=0")
    expect("deferred>0 → FAIL",
           awaken_gate_failures(deferred, False, True, 1, "1.2.3") != [])
    failed = parse_pack_update_result("PACK_UPDATE_RESULT pack_version=1.2.3 deferred=0 failed=1")
    expect("failed>0 → FAIL",
           awaken_gate_failures(failed, False, True, 1, "1.2.3") != [])
    expect("토큰 부재 → FAIL", awaken_gate_failures(None, False, True, 0, "1.2.3") != [])
    # ★review1 M1 FIX(blocking) 회귀 핀: reinject=skipped 팔(재주입 RPC 실패 — cys.rs
    # pack_update_reinject_skipped_line)은 failed=/deferred= 없이 토큰만 찍는다. 파서가 그 결측을
    # 0으로 채워도(else 분기가 아니라 skipped 분기로 갈라져야) 게이트가 통과시키면 안 된다 — 개정 전
    # 코드에서는 이 자리가 PASS(적색이어야 할 자리에서 초록)였다.
    skipped_tok = parse_pack_update_result(
        "[pack-update] reinject 스킵(데몬 점검 필요): 연결 실패\n"
        "PACK_UPDATE_RESULT pack_version=1.2.3 reinject=skipped reason=daemon_rpc_failed")
    expect("parse reinject=skipped 토큰 부재 아님(값 존재)", skipped_tok is not None)
    expect("parse reinject=skipped 필드", skipped_tok["reinject"] == "skipped")
    expect("parse reason=daemon_rpc_failed 필드", skipped_tok["reason"] == "daemon_rpc_failed")
    expect("parse reinject=skipped 는 failed/deferred 를 0 그대로 둔다(재지 않음)",
           skipped_tok["failed"] == 0 and skipped_tok["deferred"] == 0)
    expect("reinject=skipped → FAIL(각성 증명 불가 — v0.14.40 은 토큰 부재로 FAIL 이었다)",
           awaken_gate_failures(skipped_tok, False, True, 0, "1.2.3") != [])
    expect("reinject=skipped + directive_changed=True 여도 FAIL(①이 먼저 걸린다)",
           awaken_gate_failures(skipped_tok, True, True, 0, "1.2.3") != [])
    expect("directive_changed=None + 마커 무변경(bump=0) → FAIL(명시 신호 없는 무변경 통과 금지)",
           awaken_gate_failures(clean, None, True, 0, "1.2.3") != [])
    expect("directive_changed=True + bump>=1 → PASS",
           awaken_gate_failures(clean, True, True, 1, "1.2.3") == [])
    expect("directive_changed=True + bump=0 → FAIL",
           awaken_gate_failures(clean, True, True, 0, "1.2.3") != [])
    expect("directive_changed=True + 마커 관측불가 → FAIL",
           awaken_gate_failures(clean, True, False, 0, "1.2.3") != [])

    # semver 범프 회귀.
    expect("version_bumped 1.0.0→1.0.1 True", version_bumped("1.0.0", "1.0.1"))
    expect("version_bumped 2.0.0→1.0.0 False", not version_bumped("2.0.0", "1.0.0"))

    if failures:
        print(f"\n[FAIL] self-test {len(failures)}건: {failures}")
        return 1
    print("\n[PASS] self-test 전건 통과.")
    return 0


def main():
    ap = argparse.ArgumentParser(description="무중단 팩 업데이트 실측 E2E")
    ap.add_argument("--from", dest="from_dir",
                    help="테스트 팩 디렉터리(pack.tar.gz + pack-manifest.json + pack-manifest.json.minisig)")
    ap.add_argument("--socket", default=default_socket(), help="cysd Unix 소켓 경로")
    ap.add_argument("--pack-dir", default=default_pack_dir(), help="설치 팩 디렉터리(.pack-version 위치)")
    ap.add_argument("--require-live", action="store_true",
                    help="라이브 cysd 부재 시 skip이 아니라 FAIL(릴리스/승인 게이트)")
    ap.add_argument("--require-pack", action="store_true",
                    help="테스트 팩(--from) 부재/불완전 시 skip이 아니라 FAIL(릴리스/승인 게이트)")
    ap.add_argument("--require-awaken", action="store_true",
                    help="PACK_UPDATE_RESULT failed==0 AND deferred==0 + directive-changing 팩 마커 각성을 "
                         "release hard gate로 요구(deferred-only/미각성 통과 차단)")
    ap.add_argument("--self-test", action="store_true",
                    help="라이브 데몬 없이 파싱·게이트 판정 로직만 단위 검증(exit 0/1)")
    ap.add_argument("--app-pid", type=int, default=None,
                    help="cys-app(Tauri) OS 프로세스 pid — 주면 pack-update 전후 동일성(재시작 0) 검증")
    args = ap.parse_args()

    if args.self_test:
        return run_self_test()

    # 각성 hard gate는 --require-awaken 또는 --require-live(릴리스 게이트) 시 켜진다.
    awaken_gate = args.require_awaken or args.require_live

    fails = []

    def check(name, cond, detail=""):
        tag = "PASS" if cond else "FAIL"
        print(f"[{tag}] {name}" + (f" — {detail}" if detail and not cond else ""))
        if not cond:
            fails.append(name)

    # 게이트 0: 라이브 데몬 — 평시 graceful skip(exit 77), --require-live면 FAIL(exit 1).
    try:
        rpc(args.socket, "system.ping", {})
    except (DaemonUnavailable, OSError) as e:
        msg = f"라이브 cysd 없음({args.socket}): {e}"
        if args.require_live:
            print(f"[FAIL] {msg} — --require-live 게이트(skip 불가).")
            return 1
        print(f"[SKIP] {msg} — 무중단 실측 생략(로직은 완성).")
        return SKIP_EXIT

    # 게이트 0': 서명된 테스트 팩 — 평시 graceful skip(exit 77), --require-pack면 FAIL(exit 1).
    def pack_gate_skip(reason):
        if args.require_pack:
            print(f"[FAIL] {reason} — --require-pack 게이트(skip 불가).")
            return 1
        print(f"[SKIP] {reason} 실측 생략(로직은 완성).")
        return SKIP_EXIT

    if not args.from_dir:
        return pack_gate_skip("--from 미지정 — 서명된 테스트 팩이 있어야 실측 가능.")
    needed = ["pack.tar.gz", "pack-manifest.json", "pack-manifest.json.minisig"]
    missing = [f for f in needed if not os.path.isfile(os.path.join(args.from_dir, f))]
    if missing:
        return pack_gate_skip(f"테스트 팩 불완전({args.from_dir}) — 누락 {missing}.")

    # cys-app(Tauri) 기동시각 — pack-update가 OS 앱 프로세스를 재시작하지 않음을 보장(재시작 0).
    app_before = None
    if args.app_pid is not None:
        if not proc_alive(args.app_pid):
            check(f"cys-app(pid {args.app_pid}) 반영 전 생존", False,
                  "프로세스 부재 — --app-pid 확인")
        else:
            app_before = proc_starttime(args.app_pid)
            print(f"[snap-before] app_pid={args.app_pid} starttime={app_before!r}")

    # ★노드 각성 — 반영 전 reinject 마커 베이스라인(session_id→pack_version).
    markers_before = topology_markers(args.socket) or {}

    # 반영 전 스냅샷.
    before = snapshot(args.socket, args.pack_dir)
    print(f"[snap-before] daemon_pid={before['daemon_pid']} "
          f"surfaces={sorted(before['surfaces'])} pack_version={before['pack_version']!r}")
    # 반영 전에도 모든 세션이 살아있어야(exited:false) 비교 기준이 유효하다.
    check("반영 전 전 세션 생존(exited:false)",
          all(not e for e in before["surfaces"].values()),
          f"exited 세션 존재: {[s for s, e in before['surfaces'].items() if e]}")

    # pack-update 실행.
    cys = find_cys_bin()
    print(f"[run] {cys} pack-update --from {args.from_dir}")
    proc = subprocess.run(
        [cys, "pack-update", "--from", args.from_dir],
        capture_output=True, text=True,
        env=dict(os.environ, CYS_SOCKET=args.socket, CYS_PACK_DIR=args.pack_dir),
    )
    sys.stdout.write(proc.stdout)
    sys.stderr.write(proc.stderr)
    check("pack-update 종료코드 0", proc.returncode == 0, f"exit={proc.returncode}")

    # ★PACK_UPDATE_RESULT 안정 토큰 파싱 — 각성 게이트(failed/deferred)의 1차 근거.
    pack_result = parse_pack_update_result(proc.stdout)
    if pack_result is not None:
        print(f"[result] PACK_UPDATE_RESULT injected={pack_result['injected']} "
              f"skipped={pack_result['skipped']} deferred={pack_result['deferred']} "
              f"failed={pack_result['failed']}")
    else:
        print("[result] PACK_UPDATE_RESULT 토큰 없음(구버전 사이드카·reinject 미실행 가능).")

    # 반영 후 스냅샷.
    after = snapshot(args.socket, args.pack_dir)
    print(f"[snap-after]  daemon_pid={after['daemon_pid']} "
          f"surfaces={sorted(after['surfaces'])} pack_version={after['pack_version']!r}")

    # ★무중단 불변식 검증.
    check("cysd 생존 — daemon_pid 동일(재시작 0)",
          before["daemon_pid"] is not None and before["daemon_pid"] == after["daemon_pid"],
          f"{before['daemon_pid']} != {after['daemon_pid']} (재시작 = 무중단 위반)")
    check("세션 집합 불변 — surface_id set 동일",
          set(before["surfaces"]) == set(after["surfaces"]),
          f"before={sorted(before['surfaces'])} after={sorted(after['surfaces'])}")
    check("반영 후 전 세션 생존(exited:false)",
          all(not e for e in after["surfaces"].values()),
          f"exited 세션 존재: {[s for s, e in after['surfaces'].items() if e]}")
    check("팩 반영 — .pack-version 범프",
          version_bumped(before["pack_version"], after["pack_version"]),
          f"{before['pack_version']!r} → {after['pack_version']!r} (범프 안 됨)")

    # ★cys-app(Tauri) 동일성 — 재시작 0(같은 프로세스 인스턴스 유지).
    if app_before is not None:
        app_after = proc_starttime(args.app_pid)
        check(f"cys-app(pid {args.app_pid}) 생존 — 재시작 0",
              proc_alive(args.app_pid) and app_after == app_before,
              f"기동시각 {app_before!r} → {app_after!r} (변동 = 앱 재시작 = 무중단 위반)")

    # ★노드 각성 검증 — pack_reinject 마커가 새 pack_version으로 갱신됐는지.
    new_ver = after["pack_version"]
    markers_after = topology_markers(args.socket)
    bump_count = 0
    if markers_after is None:
        print("[awaken] system.topology 미응답 — 라이브 마커 확인 생략(RPC 미노출 가능).")
    else:
        changed = {k: v for k, v in markers_after.items() if markers_before.get(k) != v}
        awakened = [k for k, v in changed.items() if v == new_ver]
        stale = {k: v for k, v in changed.items() if v != new_ver}
        bump_count = len(awakened)
        print(f"[awaken] reinject 마커: 새 버전({new_ver!r}) 각성 {len(awakened)}개, "
              f"마커 변동 없음 {len(markers_after) - len(changed)}개 "
              f"(디렉티브 해시 불변 시 reinject 스킵 — 설계 §7-② step1).")
        # 마커가 '변했다면' 반드시 새 버전이어야 한다(부분/낡은 각성 = FAIL · 평시·게이트 공통).
        check("노드 각성 — 변경된 reinject 마커는 새 pack_version",
              not stale,
              f"낡은/오류 마커: {stale} (기대 {new_ver!r})")

    # ★release 각성 hard gate — deferred-only/미각성 무변경 통과 차단(codex R2 #2).
    if awaken_gate:
        directive_changed = read_manifest_signal(args.from_dir)
        if directive_changed is False:
            print("[awaken] manifest directive_hash_changed=false — skill-only 팩, 마커 무변경 정상 허용.")
        gate_fails = awaken_gate_failures(
            pack_result, directive_changed, markers_after is not None, bump_count, new_ver)
        check("각성 hard gate — failed==0 AND deferred==0 + 디렉티브 변경 팩 마커 각성",
              not gate_fails, "; ".join(gate_fails))

    if fails:
        print(f"\n[FAIL] {len(fails)}건: {fails}")
        return 1
    print("\n[PASS] 무중단 검증 통과 — cysd·세션·앱 생존, 팩만 갱신(재시작 0), 노드 각성 정합.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
