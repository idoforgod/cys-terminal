#!/usr/bin/env python3
"""javis_bootstrap.py — master 부트 시퀀스의 결정론 격상 (BOOTSTRAP_HARDENING WP-1).

"너는 마스터다" 이후의 기계적 절차 전부를 단일 exit-code 체인으로 수행한다.
LLM(master)의 역할은 이 스크립트 실행·출력 인용·이후 지휘뿐이다 — 산문 단계 수행 금지.

단계 체인 (실패 시 즉시 중단·단계명+원인을 stderr와 boot-last.json에 기록):
  ① preflight --fix READY        ② cys ping                ③ cys claim-role master
  ④ cys boot (결손>0에서만 — 결손 0=구성 충족이면 호출 생략·스폰 없음)
  ⑤ orchestra check (bounded retry 3s×10 — 노드 스폰은
  비동기·check는 무대기 스냅샷이므로 레이스 봉쇄)          ⑥ 완료 마커 write
  ⑦ cys-dept promote-if-pending --request-only (비대기 — 부트와 승격 동의의 분리)
  ⑧ 기계 요약 JSON 출력 (master는 이것을 인용해 보고한다)

완료 마커 ~/.cys/.master-bootstrapped 는 base 데몬 전용 단일-writer 마커다:
  - writer = 이 스크립트의 성공 경로(⑤ exit 0 후 ⑥) 유일. 삭제 주체 없음(버전 필드로 stale 판정).
  - ★소켓 격리: CYS_SOCKET이 base가 아니면(부서 pane 부트) write하지 않는다 — 부서장 부트가
    base 마커를 오염시키면 CEO 승격 게이트(cys-dept)가 오개방된다.

exit: 0=부트 완료(또는 부서장 단독 각성 — CEO 티켓 부재) / 2=preflight / 3=ping
      7=claim 거부(이 surface는 master 아님 — 지휘 중단·인계) / 4=boot / 6=check 최종 실패
      5=assert-ready 게이트 실패(하위 게이트 전용)
      8=레인↔팩 정합 실패(부서 소켓↔부서 팩 교차 오염 차단 — 팀 기동 전 중단)
      10=claim 신원 해석 실패(좌석 경합 아님 — 재시도 3회 소진·master 부재일 수 있음)
      9=자원 hard_block(결손 기준 자원 사전 게이트 — 팀 기동 전 착수 거부·CEO escalation)
안전밸브: CYS_BOOT_GATE=warn(assert-ready 실패를 경고로 강등)|off(게이트 무력).

부서 교리 게이트 (증분2 — D1 옵션 1'):
  ⓐ CEO 티켓 권한 게이트(P7): 부서 레인(CYS_SOCKET=부서 소켓)의 팀 기동은 CEO 발급 티켓 필수.
     티켓 부재/만료 → 실패가 아니라 '부서장 단독 각성'으로 강등(팀 기동만 생략·역할 등록/프리플라이트는
     정상·exit 0). 발급은 base 레인에서 `issue-ticket --dept <name>` 로만.
  ⓑ 결손 기준 자원 게이트: 팀 기동 직전 결손을 cys list 라이브 노드의 **구성 판정**으로 산출 —
     cso≥1 ∧ worker≥1 ∧ reviewer계열≥2 전부 충족 시에만 결손 0(구 총수 4 비교는 reviewer 4개+
     cso/worker 사망을 결손 0으로 오판 — 폐기). 결손 0(재선언)이면 게이트와 ④ cys boot 호출 자체를
     생략(스폰 없음·오탐 hard-block 방지 — "결손 0=스폰 없음"의 결정론화). 결손>0이면 자원 사전
     게이트 발동(hard=exit 9, 단 nodes 과계수 결함은 cys list 라이브 교차확인으로 무효화·1회 경고 후
     진행 / soft=매번 경고 후 진행 — 결손 0이면 게이트 자체를 생략하므로 soft 경고는 실팀기동
     시에만 발생, 소음 아니라 신호). 이 게이트는 base 포함 전 레인에 적용된다.

신뢰 모델: 티켓·마커·구성 게이트는 LLM 드리프트 차단용 결정론 가드이지 보안 경계가 아니다
(동일 $HOME 신뢰 도메인 — 파일 권한으로 악의 행위자를 막는 설계가 아님).
"""
import json
import os
import re
import subprocess
import sys
import tempfile
import time

# ★R3(D-IMPL-3): Windows 파이프 환경(cp949/cp1252)에서 한글 출력 UnicodeEncodeError 크래시 방어 —
# PYTHONUTF8 export는 cys-dept 경로에만 있어 이 스크립트의 직접 실행을 보호하지 못한다.
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

HOME = os.path.expanduser("~")
CYS_DIR = os.path.join(HOME, ".cys")
PACK = os.environ.get("CYS_PACK_DIR") or os.path.join(CYS_DIR, "pack")
MARKER = os.path.join(CYS_DIR, ".master-bootstrapped")
STATE_DIR = os.path.join(CYS_DIR, "state")
BOOT_LAST = os.path.join(STATE_DIR, "boot-last.json")
# ⑤ bounded retry — 무한 대기 금지(자원 거버넌스). env 오버라이드는 테스트 하네스 전용.
# ★예산 확대(오너 2026-07-15 적대검증 adv#4): 냉시작 claude는 모델 로드+MCP init로 30초 내
# agent_alive/set-status ack가 안 나 check가 조기 실패(팀은 아직 뜨는 중)했다. 노드 기동은 비동기라
# 넉넉히 기다린다 — 24×5s ≈ 120초 상한(무한 아님·자원 거버넌스 유지).
CHECK_RETRIES = max(1, int(os.environ.get("CYS_BOOT_CHECK_RETRIES", "24")))
CHECK_INTERVAL_S = float(os.environ.get("CYS_BOOT_CHECK_INTERVAL_S", "5"))  # 총 상한 ≈ 120초

# ── 증분2: 부서 교리 게이트 상태 ──
# CEO 티켓 저장소(base 레인이 발급·부서 레인이 소비) + 24h TTL + 1회성(소비 시 .used rename).
TICKET_DIR = os.path.join(STATE_DIR, "dept-boot-tickets")
TICKET_TTL_SECS = float(os.environ.get("CYS_DEPT_TICKET_TTL_SECS", str(24 * 3600)))

def _atomic_write_json(path, obj):
    """CRLF 함정 회피(newline='\\n')·원자 교체 — Windows 재직렬화 원복 교훈."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), prefix=".tmp-boot-")
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as f:
            json.dump(obj, f, ensure_ascii=False, indent=1)
            f.write("\n")
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            try:
                os.unlink(tmp)
            except OSError:
                pass


def _read_json(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return None


def _progress(msg):
    """★R12: 단계 시작 신호(stderr 1줄) — 진행 중 무출력이면 최악 수 분의 침묵 창이 생겨
    관찰자(초보·master)가 '멈춤'으로 오인한다(실사고 증상②의 형태적 재생산 방지).
    기계 계약(exit code·⑧ JSON)과 별개의 인간 관찰자용 인터페이스."""
    sys.stderr.write("[bootstrap] %s\n" % msg)
    try:
        sys.stderr.flush()
    except Exception:
        pass


def _run(cmd, timeout=120):
    """서브프로세스 실행 — (exit, stdout+stderr 병합 텍스트). shell 미사용(경로 quoting 안전)."""
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                           encoding="utf-8", errors="replace")
        return r.returncode, (r.stdout or "") + (r.stderr or "")
    except FileNotFoundError:
        return 127, "명령 없음: %s" % cmd[0]
    except subprocess.TimeoutExpired:
        return 124, "timeout(%ss): %s" % (timeout, " ".join(cmd))


def _socket_is_base(sock):
    """순수 판정: 소켓 경로 문자열 → base 여부(§4.1 소켓 격리). CYS_SOCKET 미설정('')=base.
    ★보수성(아키텍트 성찰): base = (미설정) 또는 (basename이 cys/cys.sock **AND** cys-dept- 성분
    없음). 커스텀 소켓(/tmp/whatever.sock)은 구코드처럼 **비-base·비-dept**다 — base 마커 무접촉·
    issue-ticket 불허·티켓 게이트 비적용(구동작 보존). "cys-dept- 성분 없으면 전부 base"는 미지
    소켓에 base 특권(마커 write·티켓 발급)을 주던 과관용이었다.
    ★경로 기반 dept 판정(basename 아님): 부서 소켓 ~/.local/state/cys-dept-<name>/cys.sock 은
    basename이 본부와 동일한 'cys.sock'이라 basename 단독 판정이 부서를 base로 오판했다
    (마커 오염·ceo_promote 오개방) — cys-dept- 성분이 있으면 무조건 비-base.
    Windows named pipe(백슬래시.백슬래시 pipe 형식)는 성분 분해가 부적합하므로 기존 basename 동작을 보존한다."""
    sock = (sock or "").strip()
    if not sock:
        return True
    norm = sock.replace("\\", "/")
    if sock.startswith("\\\\") or norm.lower().startswith("//./pipe/"):  # win named pipe — 기존 동작 보존
        return os.path.basename(norm) in ("cys", "cys.sock")
    for part in norm.split("/"):
        if part.startswith("cys-dept-"):
            return False
    return os.path.basename(norm) in ("cys", "cys.sock")


def _is_base_socket():
    """CYS_SOCKET env 래퍼(호출부 하위호환)."""
    return _socket_is_base(os.environ.get("CYS_SOCKET", ""))


def _sanitize_sock_key(sock):
    """소켓 전체 경로 → 파일명 안전 락 키(레인마다 유일). 부서 소켓은 basename(cys.sock)이 동일해
    basename 키를 쓰면 모든 레인이 같은 락 파일을 공유했다 — 전체 경로 새니타이즈로 레인 유일화.
    경로 구분자(os.sep·'/'·'\\')·':'를 '_'로 치환. 파일명 길이 상한(255) 여유 — 과길면 앞부분+경로
    해시로 유일성 보존(절단만 하면 서로 다른 긴 경로가 같은 키로 충돌)."""
    raw = (sock or "").strip() or "base"
    for ch in (os.sep, "/", "\\", ":"):
        raw = raw.replace(ch, "_")
    raw = raw.strip("_") or "base"
    if len(raw) > 160:
        import hashlib
        raw = raw[:120] + "-" + hashlib.sha1(raw.encode("utf-8")).hexdigest()[:16]
    return raw


def _socket_dept(sock=None):
    """순수 판정: 소켓 경로 → 부서명(cys-dept-<name> 성분) 또는 None(비-부서). 기본값은 CYS_SOCKET
    env. ※ dept None ≠ base: 커스텀 소켓(/tmp/whatever.sock)은 비-base·비-dept(dept None)다 —
    base 판정은 _socket_is_base가 별도로 한다. 빈 suffix(cys-dept-/)는 _socket_malformed_dept가
    불량 레인으로 걸러 cmd_run이 명시 실패한다."""
    sock = os.environ.get("CYS_SOCKET", "") if sock is None else sock
    sock = (sock or "").strip()
    if not sock:
        return None
    for part in sock.replace("\\", "/").split("/"):
        if part.startswith("cys-dept-"):
            return part[len("cys-dept-"):] or None
    return None


def _socket_malformed_dept(sock=None):
    """순수 판정: 빈 부서명 레인(경로 성분이 정확히 'cys-dept-' — suffix 없음) 감지(R1-LOW-2).
    이 소켓은 _socket_is_base=False인데 _socket_dept=None이라 어느 레인 계약에도 못 들어가는
    불량 레인이다 — cmd_run이 레인↔팩 가드 계열(exit 8)로 시끄럽게 명시 실패한다."""
    sock = os.environ.get("CYS_SOCKET", "") if sock is None else sock
    sock = (sock or "").strip()
    if not sock:
        return False
    return any(part == "cys-dept-" for part in sock.replace("\\", "/").split("/"))


def _pack_dept(pack=None):
    """순수 판정: 팩 경로 마지막 성분 pack-dept-<name> → name, 아니면 None(메인 팩). 기본값은 PACK."""
    pack = PACK if pack is None else pack
    base = os.path.basename((pack or "").replace("\\", "/").rstrip("/"))
    if base.startswith("pack-dept-"):
        return base[len("pack-dept-"):] or None
    return None


def _lane_pack_mismatch(sock=None, pack=None):
    """레인(소켓 부서)↔팩(부서 팩) 정합 판정. 정합이면 None, 불일치면 (sock_dept, pack_dept).
    교차 오염(UT-14): dept-X 레인이 메인/다른 부서 팩을 쓰거나 base 레인이 부서 팩을 쓰면 위험."""
    sd = _socket_dept(sock)
    pd = _pack_dept(pack)
    return None if sd == pd else (sd, pd)


def _notify_loud(title, body):
    """실패를 시끄럽게 알림 — feed push(승인 채널) 우선, 실패 시 cys send --queued --to master 폴백.
    둘 다 best-effort·짧은 timeout(데몬 부재 시 행 금지·graceful). 성공 채널명 또는 'none' 반환(흔적)."""
    for name, cmd in (
        ("feed", ["cys", "feed", "push", "--kind", "bootstrap-fail", "--title", title, "--body", body]),
        ("send", ["cys", "send", "--queued", "--to", "master", "[부트 중단] %s — %s" % (title, body)]),
    ):
        try:
            r = subprocess.run(cmd, capture_output=True, timeout=10)
            if r.returncode == 0:
                return name
        except Exception:
            continue
    return "none(데몬 부재 등 — 비제로 exit·boot-last.json이 최종 증거)"


def _pack_version():
    v = None
    for cand in (os.path.join(CYS_DIR, ".pack-version"), os.path.join(PACK, ".pack-version")):
        try:
            with open(cand, encoding="utf-8") as f:
                v = f.read().strip() or None
            if v:
                break
        except OSError:
            continue
    return v or "unknown"


def _binary_version():
    code, out = _run(["cys", "--version"], timeout=10)
    return out.strip().splitlines()[0] if code == 0 and out.strip() else "unknown"


class _Log:
    """단계 결과를 boot-last.json에 누적(진단 가시성 — 각 retry 시도 포함)."""

    def __init__(self):
        self.data = {"started": time.strftime("%Y-%m-%dT%H:%M:%S"), "steps": [],
                     "socket": os.environ.get("CYS_SOCKET", ""), "base_socket": _is_base_socket()}

    def step(self, name, code, detail=""):
        self.data["steps"].append({"step": name, "exit": code,
                                   "detail": detail.strip()[-2000:]})
        _atomic_write_json(BOOT_LAST, self.data)

    def fail(self, name, code, detail, exit_code):
        self.step(name, code, detail)
        self.data["result"] = {"ok": False, "failed_step": name, "exit": exit_code}
        _atomic_write_json(BOOT_LAST, self.data)
        sys.stderr.write("[bootstrap] 단계 실패: %s (exit %d)\n%s\n" % (name, code, detail.strip()))
        # ★실패 가시화(오너 2026-07-15 적대검증 adv#5): 훅이 배경 실행이라 stderr가 화면에 안 보인다.
        # 훅 NOTE는 "팀이 뜬다"고 알렸는데 부트가 조용히 실패하면 사용자는 원인을 모른다 — feed 알림으로
        # 승격(best-effort·데몬 다운 등 실패 무해). ②ping 실패(데몬 자체 부재)는 feed도 불가라 skip.
        if name != "②ping":
            # ★결함수리(2026-07-23): ③claim-role 힌트가 exit 코드와 무관하게 "다른 pane이 이미
            # master"로 고정돼 있어, 신원 해석 실패(exit 10)에도 존재하지 않는 master를 찾으라는
            # 거짓 안내가 Feed로 나갔다. 사유별로 분기한다.
            if name == "③claim-role":
                hint = ("이 pane의 신원(소속 pane 계보)을 데몬이 확정하지 못해 master 등록에 실패했습니다 — "
                        "master 좌석 경합이 아닙니다. cys list로 master 부재를 확인하고, pane 안에서 "
                        "재선언하거나 cys launch-agent --role master --agent claude 로 다시 띄우세요."
                        ) if exit_code == 10 else (
                        "다른 pane이 이미 master입니다 — 기존 master 탭을 쓰세요(조직당 master 1명).")
            else:
                hint = {"④boot": "팀(CSO·워커·리뷰어) 기동 실패 — claude CLI 설치를 확인하세요.",
                        "⑤check": "팀 노드가 제 시간에 안 떴습니다 — cys list로 확인하고 필요시 재선언하세요."
                        }.get(name, "부트스트랩이 %s 단계에서 실패했습니다 — cys list·boot-last.json 확인." % name)
            try:
                subprocess.run(["cys", "feed", "push", "--kind", "bootstrap-fail",
                                "--title", "부트스트랩 미완(%s)" % name, "--body", hint],
                               capture_output=True, timeout=10)
            except Exception:
                pass
        return exit_code


def _singleflight_key(sock):
    """순수 판정: 소켓 → 싱글플라이트 락 키(R1-LOW-4). base 레인은 env 미설정·base 경로 명시
    어느 쪽이든 단일 'base' 키로 정규화한다 — 같은 base 데몬에 서로 다른 락을 주던 선재결함 교정.
    비-base(부서·커스텀)는 전체 경로 새니타이즈로 레인마다 유일."""
    return "base" if _socket_is_base(sock) else _sanitize_sock_key(sock)


def _acquire_singleflight():
    """부트스트랩 전체 단일 실행 락(오너 2026-07-15 적대검증·아키텍트: preflight 300s는 boot 락으로
    직렬화되지 않아 중복 fire가 settings.json read-modify-write를 경쟁하고 300s 프리플라이트를 중복
    실행했다). 소켓별 flock 비차단 — 이미 진행 중이면 None 반환(호출부가 no-op 종료). unix 전용
    실효(windows는 항상 락 획득=직렬화 없음, boot 락이 최종 방어). 반환 fd를 프로세스 수명동안 보유."""
    key = _singleflight_key(os.environ.get("CYS_SOCKET", ""))  # ★base 정규화 + 비-base 전체 경로 유일화
    lock_path = os.path.join(STATE_DIR, "bootstrap-%s.lock" % key)
    try:
        os.makedirs(STATE_DIR, exist_ok=True)
        fd = os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o644)
    except OSError:
        return True  # 락 못 열면 직렬화 없이 진행(보수적 허용)
    if os.name == "posix":
        import fcntl
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            os.close(fd)
            return None  # 다른 부트스트랩 진행 중 — no-op
    _acquire_singleflight._fd = fd  # GC로 fd 닫혀 락 풀리지 않게 프로세스 수명동안 보유
    return True


# ── 증분2 ⓐ: CEO 티켓 권한 게이트(P7) ──
# 이 게이트는 LLM 드리프트 차단용 결정론 가드이지 보안 경계가 아니다(동일 $HOME 신뢰 도메인).
def _ticket_path(dept):
    return os.path.join(TICKET_DIR, "%s.ticket" % dept)


def _parse_ticket_json(text, dept, now):
    """순수 판정: 티켓 파일 텍스트 → (유효 bool, 사유). now(epoch) 주입으로 TTL 결정론 검증.
    계약: JSON 객체 · dept 일치 · issued_at(epoch 숫자) 존재 · 0<=경과<=TTL. 위반=강등(단독 각성)."""
    try:
        d = json.loads(text)
    except (ValueError, TypeError):
        return False, "티켓 JSON 파싱 실패"
    if not isinstance(d, dict):
        return False, "티켓 루트가 객체 아님"
    if d.get("dept") != dept:
        return False, "티켓 dept 불일치(%r≠%r)" % (d.get("dept"), dept)
    ts = d.get("issued_at")
    if not isinstance(ts, (int, float)) or isinstance(ts, bool):
        return False, "issued_at 없음/형식오류"
    age = now - ts
    if age < 0:
        return False, "issued_at 미래(시계 이상 %ds)" % int(-age)
    if age > TICKET_TTL_SECS:
        return False, "티켓 만료(%dh 경과 > TTL %dh)" % (age / 3600, TICKET_TTL_SECS / 3600)
    return True, "유효(발급 %dm 전 · issuer=%s)" % (age / 60, d.get("issuer", "?"))


def _peek_dept_ticket(dept):
    """부서 티켓 유효성 조회(소비하지 않음). → (유효 bool, 사유, path)."""
    path = _ticket_path(dept)
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return False, "티켓 파일 부재(%s)" % path, path
    ok, why = _parse_ticket_json(text, dept, time.time())
    return ok, why, path


def _consume_dept_ticket(path):
    """티켓을 .used 로 rename(1회성 소비). 실패해도 부트는 계속(best-effort) — 흔적만 반환."""
    try:
        os.replace(path, path + ".used")
        return "소비(.used)"
    except OSError as e:
        return "소비 실패(%s — 이미 rename됐거나 권한): 계속" % e


# ── 증분2 ⓑ: 결손 기준 자원 사전 게이트 ──
# 의무 구성(cso 1·worker 1·리뷰어계열 2 — grok 선택 제외). ★총수 비교가 아니라 역할별 구성 판정:
# 총수 4 비교는 reviewer 4개 생존+cso/worker 사망을 결손 0으로 오판했다(R1-MED-1).
_REQUIRED_COMPOSITION = (("cso", 1), ("worker", 1), ("reviewer", 2))


def _team_composition_deficit(counts):
    """순수 판정: 역할별 라이브 카운트 dict → (결손 bool, 사유). cso≥1 ∧ worker≥1 ∧
    reviewer계열≥2 **전부 충족** 시에만 결손 0(R1-MED-1 — 구 총수 4 비교 폐기)."""
    missing = ["%s %d<%d" % (role, counts.get(role, 0), need)
               for role, need in _REQUIRED_COMPOSITION if counts.get(role, 0) < need]
    if missing:
        return True, "구성 결손(%s) — 결손 존재" % ", ".join(missing)
    return False, "구성 충족(cso=%d worker=%d reviewer=%d) — 결손 0(재선언·전 구성 생존)" % (
        counts.get("cso", 0), counts.get("worker", 0), counts.get("reviewer", 0))


def _team_has_deficit():
    """팀 결손 여부를 cys list 라이브 노드의 **구성 판정**으로 산출(orchestra check의 판정 로직과
    동일 신호 — role surface 생존). ★orchestra check 서브프로세스를 쓰지 않는다: ④-b(boot-reviewers)
    →⑤(check) orchestra 호출 순서·검증 계약을 오염시키지 않기 위함(deficit용 별도 check가 그 순서에
    끼면 안 됨). 반환: (결손 bool, 사유). cys list 실패 → 보수적으로 결손 가정(게이트 진행)."""
    counts = _live_role_counts()
    if counts is None:
        return True, "cys list 실패 — 결손 가정(게이트 진행·보수)"
    return _team_composition_deficit(counts)


def _live_role_counts():
    """cys list 로 라이브(미exited) 노드의 역할별 카운트 산출 → {'cso','worker','reviewer'} dict.
    변형 역할(cso-1·worker-2·reviewer-*)은 접두로 귀속(디렉티브 주입 dir_file 접두 규약과 정합).
    파싱 불가/데몬 부재 → None(호출부 보수 판정)."""
    code, out = _run(["cys", "list"], timeout=15)
    if code != 0:
        return None
    counts = {"cso": 0, "worker": 0, "reviewer": 0}
    for line in out.splitlines():
        f = line.rstrip("\n").split("\t")
        if len(f) < 4:
            continue
        role = f[1][5:] if f[1].startswith("role=") else ""
        if f[3].strip().endswith("true"):  # exited surface 무시
            continue
        if role == "cso" or role.startswith("cso-"):
            counts["cso"] += 1
        elif role == "worker" or role.startswith("worker-"):
            counts["worker"] += 1
        elif role.startswith("reviewer"):
            counts["reviewer"] += 1
    return counts


def _live_node_count():
    """cys list 로 라이브(미exited) 노드 role surface 수 산출(ps 과계수 결함 교차확인용).
    파싱 불가/데몬 부재 → None(호출부는 교차확인 불가 시 genuine hard-block로 보수 판정)."""
    code, out = _run(["cys", "list"], timeout=15)
    if code != 0:
        return None
    n = 0
    for line in out.splitlines():
        f = line.rstrip("\n").split("\t")
        if len(f) < 4:
            continue
        role = f[1][5:] if f[1].startswith("role=") else ""
        if f[3].strip().endswith("true"):  # exited surface 무시
            continue
        if role in ("cso", "worker") or role.startswith("worker-") or role.startswith("reviewer"):
            n += 1
    return n


def _resource_gate_decision(gate_exit, gate_json, live_node_count):
    """순수 판정: 자원 게이트 exit·json·라이브 노드 수 → (verdict, 사유).
    verdict ∈ allow|soft|hard-overcount|hard-block.
      exit 0=allow · 1=soft · 2=hard(단, nodes-only hard이고 라이브 노드<유효임계면 과계수→overcount로 무효화).
      기타 exit(내부오류)=allow(보수적 — 게이트 내부오류로 부트를 막지 않되 진행)."""
    if gate_exit == 0:
        return "allow", "자원 게이트 allow"
    if gate_exit == 1:
        return "soft", "자원 게이트 soft_warn"
    if gate_exit == 2:
        trips = (gate_json or {}).get("trips") or []
        hard = [t for t in trips if t.get("level") == "hard"]
        nodes_hard = [t for t in hard if t.get("metric") == "nodes"]
        other_hard = [t for t in hard if t.get("metric") != "nodes"]
        if nodes_hard and not other_hard and isinstance(live_node_count, (int, float)):
            eff = ((gate_json or {}).get("measured") or {}).get("nodes_hard_effective")
            if isinstance(eff, (int, float)) and live_node_count < eff:
                return "hard-overcount", (
                    "nodes hard(ps=%s)이나 라이브 노드 %d < 유효임계 %s — ps 과계수 결함으로 판단, "
                    "1회 경고 후 진행" % (nodes_hard[0].get("value"), live_node_count, eff))
        detail = ", ".join("%s=%s" % (t.get("metric"), t.get("value")) for t in hard) or "미상"
        return "hard-block", "자원 hard_block(트립: %s) — 팀 기동 거부" % detail
    return "allow", "자원 게이트 미지 exit %s(내부오류) — 보수적 진행" % gate_exit


def _run_resource_gate(py, log):
    """결손>0 확정 후의 자원 사전 게이트(호출부가 결손 0이면 이 함수를 호출하지 않는다).
    반환: None=진행 / 9=hard-block(팀 기동 0·CEO escalation)."""
    gate = os.path.join(PACK, "bin", "javis_resource_gate.py")
    if not os.path.isfile(gate):
        log.step("④′resource-gate", 0, "결손>0이나 resource_gate 부재 — 게이트 생략(계속)")
        return None
    code, out = _run([py, gate, "check", "--json"], timeout=30)
    try:
        gate_json = json.loads(out.strip())
    except (ValueError, TypeError):
        gate_json = None
    live = _live_node_count() if code == 2 else None
    verdict, why = _resource_gate_decision(code, gate_json, live)
    log.step("④′resource-gate", code, "결손>0 · verdict=%s · %s" % (verdict, why))
    if verdict == "hard-block":
        _progress("✗ 자원 hard_block — 팀 기동 0·CEO escalation: " + why)
        notified = _notify_loud("자원 hard_block(부트 중단)",
                                "%s. 자원 정리(서버 kill·/clear·노드 회수) 후 재선언하라." % why)
        log.step("④′resource-gate-notify", 0, "알림 채널: %s" % notified)
        log.data["result"] = {"ok": False, "failed_step": "resource-gate", "exit": 9}
        _atomic_write_json(BOOT_LAST, log.data)
        return 9
    if verdict == "hard-overcount":
        _progress("⚠ 자원 nodes hard(과계수 결함으로 판단) — cys list 교차확인 후 1회 경고·진행: " + why)
        _notify_loud("자원 게이트 nodes 과계수 경고", why)
    elif verdict == "soft":
        # 매번 경고 후 진행(디바운스 없음) — 결손 0이면 게이트 자체를 생략하므로
        # 이 경고는 실팀기동 시에만 발생한다: 소음 아니라 신호(설계 v0.3 판정).
        _progress("⚠ 자원 soft_warn — 경고 push 후 진행: " + why)
        _notify_loud("자원 soft_warn", why)
    return None


def cmd_issue_ticket(argv):
    """CEO 티켓 발급 — base 레인 전용. 사용: issue-ticket --dept <name>.
    exit: 0=발급(경로 stdout) / 2=base 레인 아님 또는 --dept 형식 위반.
    이 게이트는 LLM 드리프트 차단용 결정론 가드이지 보안 경계가 아니다(동일 $HOME 신뢰 도메인)."""
    dept = None
    for i, a in enumerate(argv):
        if a == "--dept" and i + 1 < len(argv):
            dept = argv[i + 1]
        elif a.startswith("--dept="):
            dept = a.split("=", 1)[1]
    if not dept or not re.fullmatch(r"[a-z0-9][a-z0-9-]*", dept):
        sys.stderr.write("[issue-ticket] --dept <name>(kebab-case a-z0-9-) 필수: %r\n" % dept)
        return 2
    if not _is_base_socket():
        sys.stderr.write("[issue-ticket] base 레인에서만 티켓 발급 허용 — 현재 소켓은 부서 레인(%s). "
                         "본부(base) master에서 발급하라.\n" % os.environ.get("CYS_SOCKET", ""))
        return 2
    now = time.time()
    ticket = {"dept": dept, "issued_at": now,
              "issued_at_iso": time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(now)),
              "issuer": os.environ.get("CYS_SURFACE_ID", "") or "base-master"}
    path = _ticket_path(dept)
    _atomic_write_json(path, ticket)
    print(json.dumps({"ok": True, "dept": dept, "ticket": path,
                      "ttl_hours": round(TICKET_TTL_SECS / 3600, 1)}, ensure_ascii=False))
    return 0


def cmd_run():
    # ★단일 실행 게이트 — 진행 중이면 즉시 성공 반환(중복 preflight/boot 방지·pile-up 차단).
    if _acquire_singleflight() is None:
        _progress("부트스트랩 이미 진행 중(단일 실행 락) — 중복 실행 skip. 진행은 cys list로 확인.")
        return 0
    log = _Log()
    py = sys.executable or "python3"

    # ★불량 레인 가드(R1-LOW-2): 빈 부서명(cys-dept-/ — suffix 없음) 소켓은 base도 부서도 아닌
    # 불량 레인 — 어느 레인 계약(마커·티켓·팩 페어링)에도 못 들어가므로 시끄럽게 명시 실패한다
    # (레인↔팩 가드 계열·exit 8).
    if _socket_malformed_dept():
        detail = ("불량 레인(빈 부서명): CYS_SOCKET=%s 의 'cys-dept-' 성분에 부서명이 없다 — "
                  "base도 부서도 아닌 소켓으로는 부트 불가. 부서를 정규 이름(cys-dept-<name>)으로 "
                  "재생성한 뒤 재선언하라." % os.environ.get("CYS_SOCKET", ""))
        log.step("③′lane-pack", 1, detail)
        _progress("⚠ " + detail)
        notified = _notify_loud("불량 레인(빈 부서명 — 부트 중단)", detail)
        log.step("③′lane-pack-notify", 0, "알림 채널: %s" % notified)
        log.data["result"] = {"ok": False, "failed_step": "lane-pack", "exit": 8}
        _atomic_write_json(BOOT_LAST, log.data)
        return 8

    # ★레인↔팩 정합 가드(증분1 · UT-14 교차 오염 차단): 부서 소켓 레인은 그 부서 팩(pack-dept-X)을,
    # base 레인은 메인 팩을 써야 한다. 불일치면 잘못된 데몬/팩 조합이 마커·승격·디렉티브를 오염시키므로
    # 팀 기동(④) 전에 시끄럽게 실패한다(조용한 진행이 최악 — adv#5 실패 가시화 계열).
    mismatch = _lane_pack_mismatch()
    if mismatch is not None:
        sd, pd = mismatch
        detail = ("레인↔팩 불일치(교차 오염·UT-14): 소켓 부서=%s · 팩 부서=%s. CYS_SOCKET과 "
                  "CYS_PACK_DIR이 같은 부서를 가리켜야 한다(base↔메인팩 / dept-X↔pack-dept-X). "
                  "팀 기동 중단." % (sd or "base", pd or "메인"))
        log.step("③′lane-pack", 1, detail)
        _progress("⚠ " + detail)
        notified = _notify_loud("레인↔팩 불일치(부트 중단)", detail)
        log.step("③′lane-pack-notify", 0, "알림 채널: %s" % notified)
        log.data["result"] = {"ok": False, "failed_step": "lane-pack", "exit": 8}
        _atomic_write_json(BOOT_LAST, log.data)
        return 8

    # ★TCC 보조 경고(오너 2026-07-15): macOS 폴더 권한 리셋(서명 변경 업그레이드) 시 pane 자식이
    # EPERM으로 죽는 실사고 — 부트가 살아있는 세션에서라도 조기 경고(주 안내는 GUI perm-warning).
    if sys.platform == "darwin":
        try:
            os.listdir(os.path.join(HOME, "Desktop"))
        except PermissionError:
            _progress("⚠ macOS 데스크탑 폴더 접근 거부 — 시스템 설정→개인정보 보호 및 보안→"
                      "파일 및 폴더에서 cys 허용 후 앱 재시작(미허용 시 pane의 claude가 EPERM으로 꺼짐)")
        except OSError:
            pass

    # ① preflight --fix — ★비치명화(오너 2026-07-15 적대검증 adv#1 CRITICAL): 종전엔 preflight가
    # 완전-green(exit 0)이 아니면 여기서 abort해 ④ 팀 부팅이 영영 안 됐다. preflight는 60+ 체크
    # 표면이라 자동수리 불가 FAIL 하나(구 hook·수동 디렉티브 핀·git 부재)만 있어도 팀 0개 — "5노드
    # 100%" 요구와 정면 충돌(이 기계도 잔여 FAIL 존재). 팀 부팅의 진짜 게이트는 ⑤ check다. 따라서
    # preflight FAIL은 경고로 강등하고 ④로 계속한다. 부팅-치명 전제(데몬·claude)는 ②ping·cys boot가
    # 각자 검증하므로 preflight와 분리해도 안전. 마커가 현재 pack_version이면 300s preflight 자체를
    # 생략(재선언 fast path — pile-up·재실행 비용 제거).
    preflight = os.path.join(PACK, "bin", "javis_preflight.py")
    _marker = _read_json(MARKER) or {}
    _marker_fresh = (_is_base_socket() and _marker.get("pack_version") == _pack_version()
                     and _marker.get("pack_version") not in (None, "unknown"))
    if _marker_fresh:
        log.step("①preflight", 0, "base 마커가 현재 pack_version — preflight 생략(fast path)")
    elif os.path.isfile(preflight):
        _progress("① preflight --fix 실행 중(최대 300s · 비치명 — FAIL이어도 팀 부팅 계속)…")
        code, out = _run([py, preflight, "--fix"], timeout=300)
        log.step("①preflight", code, out)
        if code != 0:
            _progress("⚠ preflight 잔여 FAIL(비치명) — 팀 부팅 계속·진짜 게이트는 ⑤ check. 상세 boot-last.json")
    else:
        log.step("①preflight", 0, "preflight 부재 — 생략(팩 불완전 가능·계속)")

    # ② 데몬 생존 — 이후 ③의 비정상 exit를 '거부'로 해석하는 전제(데몬 생존 보증)
    _progress("② 데몬 생존 확인…")
    code, out = _run(["cys", "ping"], timeout=15)
    log.step("②ping", code, out)
    if code != 0:
        return log.fail("②ping", code, out, 3)

    # ③ claim-role master — 거부=exit 7(유령 master 차단: 이 surface는 master가 아니다)
    # ★SEAT(2026-07-17 실사고): 보유자가 '빈 좌석'(role 만 쥔 agent 없는 셸 — cys-dept 가 부서 생성 시
    #   띄우는 그 셸)이면 종전엔 여기서 영구 거부돼 부트가 데드엔드에 빠졌다(부서장이 영영 못 뜸).
    #   --takeover-empty-seat 는 **요청**일 뿐이다: 데몬이 커널 사실(자손 프로세스 0·agent 메타 없음·
    #   최근 입력 없음)로 재판정해 정말 빈 좌석일 때만 승계를 허용하고, agent 가 붙은 정당한 master 는
    #   종전대로 거부한다(유령 master 차단 규칙 불변 — 살아있는 master 가 있으면 여전히 exit 7).
    # ★결함수리(2026-07-23 master 실사고): 종전엔 claim-role 의 **모든** 실패를 exit 7("살아있는
    #   master 가 존재 — 지휘 중단·인계")로 단일 분류했고 재시도가 없었다. 실제 사고에서는 데몬이
    #   호출자 신원을 해석하지 못한 거부("claim_denied: caller (surface None) may only claim its own
    #   surface, not 6")였는데 이것이 좌석 경합으로 오분류돼 (a) 1차 부트가 즉사하고 (b) 레지스트리에
    #   master 가 하나도 없는 상태(cys list 실측 role=- )에서 진짜 master 에게 "너는 master 가 아니다,
    #   기존 master 에게 인계하라"는 **거짓 지시**가 전달됐다. 오분류는 자체로 치명이다 — 사용자는
    #   존재하지 않는 master 를 찾아 헤매고 팀은 영영 안 뜬다.
    # 수리: 거부 **사유를 판별**한다. 신원 미해석은 일시 상태일 수 있으므로 백오프 재시도하고, 소진 시
    #   exit 10(신원 해석 실패)으로 **분리 보고**한다. 살아있는 보유자에 의한 거부만 종전대로 exit 7 —
    #   유령 master 차단 규칙은 불변이며 재시도도 하지 않는다(즉시 판정).
    _progress("③ master 역할 등록…")
    _IDENTITY_HINTS = ("surface None", "CYS_SURFACE_ID", "no --surface", "unknown surface")
    code, out = 1, ""
    for _attempt in (1, 2, 3):
        code, out = _run(["cys", "claim-role", "master", "--takeover-empty-seat"], timeout=15)
        if code == 0:
            break
        if not any(h in out for h in _IDENTITY_HINTS):
            break  # 좌석 경합 등 — 재시도해도 결과가 바뀌지 않는다
        if _attempt < 3:
            _progress("③ 신원 해석 실패 — 재시도 %d/2 (데몬 레지스트리 준비 대기)…" % _attempt)
            time.sleep(1.5 * _attempt)
    log.step("③claim-role", code, out)
    if code != 0:
        if any(h in out for h in _IDENTITY_HINTS):
            msg = ("claim 거부 사유 = **신원 해석 실패**(좌석 경합 아님). 데몬이 이 호출자의 소속 pane "
                   "계보를 확정하지 못했다 — 주원인은 호출이 pane 프로세스 계보 밖(고아화·외부 셸)에서 "
                   "실행된 것. 재시도 3회 소진.\n"
                   "※ 이것은 '이미 master가 있다'는 뜻이 **아니다** — cys list 로 실측하라. pane 안에서 "
                   "직접 재선언하거나, 이 pane 을 cys 에서 다시 띄워라(cys launch-agent --role master).\n%s" % out)
            return log.fail("③claim-role", code, msg, 10)
        msg = ("이 surface는 master가 아님(claim 거부). 살아있는 master가 레지스트리에 존재한다 — "
               "선언을 중단하고 기존 master에 인계하라.\n%s" % out)
        return log.fail("③claim-role", code, msg, 7)

    # ── 증분2 ⓐ CEO 티켓 권한 게이트(P7) — 부서 레인 팀 기동 전. 티켓 부재/만료=단독 각성 강등(exit 0) ──
    dept = _socket_dept()
    ticket_path = None
    if dept is not None:
        _progress("③″ CEO 티켓 권한 게이트(부서 레인=%s)…" % dept)
        ok, why, ticket_path = _peek_dept_ticket(dept)
        if not ok:
            note = ("CEO 티켓 부재 — 부서장 단독 각성(팀 기동은 CEO 티켓 발급 후). "
                    "발급: base master에서 `javis_bootstrap.py issue-ticket --dept %s`. 사유: %s"
                    % (dept, why))
            # R1-LOW-3 검증 비대칭 경고: 발급(issue-ticket)은 정규식을 강제하나 소켓 쪽 부서명은
            # 자유 형식이라, 불일치 부서는 티켓을 영영 못 받는 비대칭이 침묵으로 남는다 — 명시.
            if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", dept):
                note += (" ★주의: 부서명 %r 은 발급 정규식([a-z0-9][a-z0-9-]*) 불일치 — "
                         "이 부서명은 티켓 발급 불가 형식이다(부서 재생성 필요)." % dept)
            _progress(note)
            log.step("③″ceo-ticket", 0, note)
            summary = {"ok": True, "marker": "부서장 단독 각성(CEO 티켓 부재)",
                       "solo_awakening": True, "dept": dept,
                       "steps": [(s["step"], s["exit"]) for s in log.data["steps"]],
                       "boot_last": BOOT_LAST}
            log.data["result"] = {"ok": True, "solo_awakening": True, "reason": why}
            _atomic_write_json(BOOT_LAST, log.data)
            print(json.dumps(summary, ensure_ascii=False))
            return 0
        log.step("③″ceo-ticket", 0, "CEO 티켓 유효 — 부서 팀 기동 진행. %s" % why)

    # ── 증분2 ⓑ 결손 기준 자원 사전 게이트 — 팀 기동(④) 직전 ──
    # 결손을 cys list 라이브 노드의 구성 판정(cso≥1∧worker≥1∧reviewer≥2)으로 산출(R1-MED-1):
    # 결손 0(재선언·전 구성 생존) → 게이트와 ④ cys boot 호출 자체를 생략("결손 0=스폰 없음" 결정론화).
    orchestra = os.path.join(PACK, "bin", "javis_orchestra.py")
    has_deficit, deficit_why = _team_has_deficit()
    if has_deficit:
        gate_rc = _run_resource_gate(py, log)
        if gate_rc is not None:
            return gate_rc  # 9 = 자원 hard_block(팀 기동 0·escalation)

        # ④ 4종 의무 노드 기동 — 결손>0에서만 호출(결손 0=스폰 경로 미진입)
        _progress("④ 4종 의무 노드 기동 중(최대 300s)…")
        code, out = _run(["cys", "boot"], timeout=300)
        log.step("④boot", code, out)
        if code != 0:
            # 티켓은 아직 미소비 — boot 실패(exit 4)면 보존돼 재시도 가능(R2-LOW-C)
            return log.fail("④boot", code, out, 4)

        # 부서 레인 CEO 티켓 소비 — ④ boot **성공 직후**(실스폰 발생)에만 1회성 소비(R2-LOW-C):
        # "1회성 티켓 ⟺ 실스폰" 불변식. 착수 전 소각은 boot 실패 시 재시도 티켓까지 태웠다.
        if ticket_path is not None:
            log.step("③″ceo-ticket-consume", 0, _consume_dept_ticket(ticket_path))

        # ④-b 리뷰어 감지·무구독 폴백(R1·D-IMPL-1 — 산문 §0 ④-b의 코드 전사): cys boot는 미설치
        # CLI를 건너뛰므로 agy/codex 부재 기계(초보 전원)에서 대체 리뷰어(reviewer-claude-*)를 기동할
        # 주체가 없으면 ⑤ check가 영영 실패한다. 실패=기록만(best-effort) — 최종 게이트는 ⑤ check.
        _progress("④-b 리뷰어 감지·폴백 기동 중(최대 320s — 대체 리뷰어 2슬롯 순차)…")
        code, out = _run([py, orchestra, "boot-reviewers"], timeout=320)
        log.step("④b-boot-reviewers", code, out)
    else:
        log.step("④′resource-gate", 0,
                 "결손 0(%s) — 자원 게이트 생략(재선언 오탐 hard-block 방지)" % deficit_why)
        # 결손 0 재선언은 스폰이 없으므로 ④ cys boot·④-b 폴백을 호출하지 않고 티켓도 태우지
        # 않는다(향후 실 기동에 재사용). ⑤ check는 유지(생존 재확인).
        log.step("④boot-skip", 0, "결손 0(구성 충족) — cys boot·④-b 생략(스폰 없음·티켓 미소비)")
        _progress("④ 결손 0(전 구성 생존) — 팀 기동 생략, ⑤ 생존 재확인으로 진행")

    # ⑤ orchestra check — bounded retry(노드 ready는 비동기·check는 스냅샷)
    _progress("⑤ 노드 생존 결정론 확인(check · 최대 %d회×%.0fs 재시도)…" % (CHECK_RETRIES, CHECK_INTERVAL_S))
    code, out = 1, "orchestra 부재"
    for attempt in range(1, CHECK_RETRIES + 1):
        code, out = _run([py, orchestra, "check"], timeout=60)
        log.step("⑤check#%d" % attempt, code, out)
        if code == 0:
            break
        if attempt < CHECK_RETRIES:
            time.sleep(CHECK_INTERVAL_S)
    if code != 0:
        return log.fail("⑤check", code,
                        "%d회 재시도 후에도 의무 노드 미기동:\n%s" % (CHECK_RETRIES, out), 6)

    # ⑥ 완료 마커 — ⑤ exit 0에서만 도달. base 소켓 가드(부서 부트는 base 마커 무접촉).
    if _is_base_socket():
        _atomic_write_json(MARKER, {
            "pack_version": _pack_version(), "binary_version": _binary_version(),
            "ts": time.strftime("%Y-%m-%dT%H:%M:%S"),
            "surface_ref": os.environ.get("CYS_SURFACE_ID", ""),
            "socket": os.environ.get("CYS_SOCKET", ""), "orchestra_check": "exit 0"})
        log.step("⑥marker", 0, MARKER)
        marker_note = "base 마커 기록"
    else:
        log.step("⑥marker", 0, "부서 소켓 컨텍스트 — base 마커 무접촉(부서장 부트 완료)")
        marker_note = "부서장 부트 — base 마커 무접촉"

    # ⑦ 승격 pending 해소 요청(비대기) — 동의·실제 승격은 부트 밖(배지/feed·차기 lifecycle)
    if _is_base_socket():
        dept = os.path.join(PACK, "bin", "cys-dept")
        if os.path.isfile(dept):
            code, out = _run(["bash", dept, "promote-if-pending", "--request-only"], timeout=30)
            log.step("⑦promote-request", code, out)  # best-effort — 실패해도 부트는 성공
        else:
            log.step("⑦promote-request", 0, "cys-dept 부재 — 생략")
    else:
        log.step("⑦promote-request", 0, "부서 컨텍스트 — 생략")

    # ⑧ 기계 요약 — master는 이 JSON을 인용해 '기동 완료'를 보고한다(다른 근거 인용 금지)
    summary = {"ok": True, "marker": marker_note,
               "steps": [(s["step"], s["exit"]) for s in log.data["steps"]],
               "boot_last": BOOT_LAST}
    log.data["result"] = {"ok": True}
    _atomic_write_json(BOOT_LAST, log.data)
    print(json.dumps(summary, ensure_ascii=False))
    return 0


def cmd_status():
    print(json.dumps({"marker": _read_json(MARKER), "boot_last": _read_json(BOOT_LAST),
                      "base_socket": _is_base_socket()}, ensure_ascii=False, indent=1))
    return 0


def cmd_assert_ready():
    """하위 게이트 전용: 마커 부재/pack_version 불일치 → exit 5.
    stale 정책(설계 §4.1): assert-ready는 버전 대조(ceo_promote 게이트는 존재만 — cys-dept 측)."""
    gate = os.environ.get("CYS_BOOT_GATE", "").strip().lower()
    if gate == "off":
        return 0
    m = _read_json(MARKER)
    ok = bool(m) and m.get("pack_version") == _pack_version()
    if ok:
        return 0
    why = "마커 부재" if not m else "pack_version 불일치(%s≠%s) — 재부트 필요" % (
        m.get("pack_version"), _pack_version())
    sys.stderr.write("[bootstrap assert-ready] %s\n" % why)
    return 0 if gate == "warn" else 5


def cmd_self_test():
    """레인 격리 3종 순수 판정 자체검증(orchestra 관례 — assert 배터리 → OK/FAIL).
    결정론·밀폐: env·데몬·파일 무접촉(순수 함수만 호출)."""
    try:
        # ── t1: base/dept 판정 매트릭스(unix base·unix dept·win pipe) ──
        assert _socket_is_base("") is True, "unset=base"
        assert _socket_is_base("/Users/x/.local/state/cys/cys.sock") is True, "unix base"
        assert _socket_is_base("/Users/x/.local/state/cys-dept-dept-1/cys.sock") is False, \
            "unix dept 소켓이 base로 오판(원 버그 — basename cys.sock 동일)"
        assert _socket_is_base("/Users/x/.local/state/cys-dept-ceo/cys.sock") is False, "unix dept(ceo)"
        assert _socket_is_base("\\\\.\\pipe\\cys") is True, "win base pipe(basename 보존)"
        assert _socket_is_base("\\\\.\\pipe\\cys-dept-foo") is False, "win dept pipe"
        # ★보수성 복원(아키텍트 성찰): 커스텀 소켓은 비-base·비-dept — base 특권(마커·티켓 발급) 없음
        assert _socket_is_base("/tmp/whatever.sock") is False, \
            "커스텀 소켓이 base로 오판(과관용 — 마커 write·티켓 발급 특권 오부여)"
        assert _socket_dept("/tmp/whatever.sock") is None, "커스텀 소켓은 비-dept(구동작 보존)"
        assert _socket_is_base("/Users/x/.local/state/cys/cys") is True, "basename cys(무확장)도 base"
        # ★불량 레인(R1-LOW-2): 빈 부서명(cys-dept-/)은 비-base·dept None — malformed로 명시 검출
        assert _socket_is_base("/s/cys-dept-/cys.sock") is False, "빈 부서명이 base로 오판"
        assert _socket_dept("/s/cys-dept-/cys.sock") is None, "빈 부서명 dept가 None 아님"
        assert _socket_malformed_dept("/s/cys-dept-/cys.sock") is True, "빈 부서명 malformed 미검출"
        assert _socket_malformed_dept("/s/cys-dept-dept-1/cys.sock") is False, "정규 부서가 malformed 오판"
        assert _socket_malformed_dept("") is False, "미설정이 malformed 오판"
        assert _socket_malformed_dept("/tmp/whatever.sock") is False, "커스텀 소켓이 malformed 오판"
        assert _socket_malformed_dept("\\\\.\\pipe\\cys-dept-") is True, "win 빈 부서명 pipe 미검출"

        # ── t2: 락 키 유일성(부서 basename 동일 → 전체 경로 유일화) ──
        k1 = _sanitize_sock_key("/Users/x/.local/state/cys-dept-dept-1/cys.sock")
        k2 = _sanitize_sock_key("/Users/x/.local/state/cys-dept-dept-2/cys.sock")
        kb = _sanitize_sock_key("")
        assert k1 != k2, "동일 basename 두 부서 소켓이 같은 락 키(원 버그)"
        assert kb == _sanitize_sock_key("base") == "base", "미설정=base 키"
        assert k1 != kb and k2 != kb, "부서 키가 base 키와 충돌"
        for k in (k1, k2, kb):
            assert k and "/" not in k and os.sep not in k and ":" not in k and "\\" not in k, \
                "락 키에 경로 구분자/공백 잔존: %r" % k
        klong = _sanitize_sock_key("/" + "a" * 400 + "/cys-dept-z/cys.sock")
        assert len(klong) <= 180, "과길이 소켓 키 미절단: %d" % len(klong)
        assert klong == _sanitize_sock_key("/" + "a" * 400 + "/cys-dept-z/cys.sock"), "새니타이즈 비결정론"
        assert klong != _sanitize_sock_key("/" + "b" * 400 + "/cys-dept-z/cys.sock"), "과길이 경로 해시 충돌"
        # ★락 키 base 정규화(R1-LOW-4): env 미설정과 base 경로 명시가 같은 base 데몬에 다른 락을
        # 주던 선재결함 — 싱글플라이트 키는 base면 단일 'base'로 수렴해야 한다.
        assert _singleflight_key("") == "base", "미설정 락 키≠base"
        assert _singleflight_key("/Users/x/.local/state/cys/cys.sock") == "base", \
            "base 경로 명시가 'base' 키로 정규화되지 않음(같은 데몬에 다른 락)"
        assert _singleflight_key("/s/cys-dept-dept-1/cys.sock") != "base", "부서 락 키가 base로 수렴"
        assert _singleflight_key("/tmp/whatever.sock") != "base", "커스텀 소켓 락 키가 base로 수렴"
        assert _singleflight_key("/s/cys-dept-dept-1/cys.sock") != \
            _singleflight_key("/s/cys-dept-dept-2/cys.sock"), "부서 간 락 키 충돌"

        # ── t3: 레인↔팩 정합(부서명 추출 + 불일치 판정) ──
        assert _socket_dept("") is None, "base 소켓 dept=None"
        assert _socket_dept("/s/cys-dept-dept-1/cys.sock") == "dept-1", "부서명 추출"
        assert _socket_dept("/s/cys/cys.sock") is None, "본부 소켓 dept=None"
        assert _pack_dept("/h/.cys/pack") is None, "메인 팩 dept=None"
        assert _pack_dept("/h/.cys/pack-dept-dept-1") == "dept-1", "부서 팩명 추출"
        assert _pack_dept("/h/.cys/pack-dept-dept-1/") == "dept-1", "trailing slash 관용"
        assert _lane_pack_mismatch("", "/h/.cys/pack") is None, "base+메인팩=정합"
        assert _lane_pack_mismatch("/s/cys-dept-dept-1/cys.sock", "/h/.cys/pack-dept-dept-1") is None, \
            "dept-X+pack-dept-X=정합"
        assert _lane_pack_mismatch("/s/cys-dept-dept-1/cys.sock", "/h/.cys/pack") == ("dept-1", None), \
            "dept 소켓+메인 팩=불일치(UT-14)"
        assert _lane_pack_mismatch("", "/h/.cys/pack-dept-dept-2") == (None, "dept-2"), \
            "base 소켓+부서 팩=불일치"
        assert _lane_pack_mismatch("/s/cys-dept-dept-1/cys.sock", "/h/.cys/pack-dept-dept-2") \
            == ("dept-1", "dept-2"), "교차 부서=불일치"

        # ── t4: CEO 티켓 파싱·TTL(증분2 ⓐ 순수 로직) ──
        now = 1_000_000.0
        good = json.dumps({"dept": "dept-1", "issued_at": now - 60, "issuer": "base-master"})
        ok, _ = _parse_ticket_json(good, "dept-1", now)
        assert ok, "유효 티켓(60s 전)이 거부됨"
        expired = json.dumps({"dept": "dept-1", "issued_at": now - TICKET_TTL_SECS - 1})
        ok, why = _parse_ticket_json(expired, "dept-1", now)
        assert not ok and "만료" in why, "TTL 초과 티켓이 유효로 통과: %s" % why
        # TTL 경계(정확히 TTL 경과)는 유효(<=)
        edge = json.dumps({"dept": "dept-1", "issued_at": now - TICKET_TTL_SECS})
        assert _parse_ticket_json(edge, "dept-1", now)[0], "TTL 경계(정확히 24h)가 만료 처리됨"
        wrong_dept = json.dumps({"dept": "dept-2", "issued_at": now})
        assert not _parse_ticket_json(wrong_dept, "dept-1", now)[0], "dept 불일치 티켓이 통과"
        future = json.dumps({"dept": "dept-1", "issued_at": now + 100})
        assert not _parse_ticket_json(future, "dept-1", now)[0], "미래 issued_at(시계 이상)이 통과"
        assert not _parse_ticket_json("{not json", "dept-1", now)[0], "손상 JSON이 유효로 통과"
        assert not _parse_ticket_json(json.dumps({"dept": "dept-1"}), "dept-1", now)[0], \
            "issued_at 부재 티켓이 통과"
        assert not _parse_ticket_json(json.dumps(
            {"dept": "dept-1", "issued_at": True}), "dept-1", now)[0], "bool issued_at이 숫자로 통과"

        # ── t5: 자원 게이트 결정 순수 로직(증분2 ⓑ) ──
        assert _resource_gate_decision(0, None, None)[0] == "allow", "exit 0=allow"
        assert _resource_gate_decision(1, None, None)[0] == "soft", "exit 1=soft"
        srv_hard = {"trips": [{"metric": "servers", "level": "hard", "value": 5}]}
        assert _resource_gate_decision(2, srv_hard, 0)[0] == "hard-block", "servers hard=block"
        # nodes-only hard + 라이브 노드<유효임계 → 과계수 무효화
        nodes_hard = {"trips": [{"metric": "nodes", "level": "hard", "value": 22}],
                      "measured": {"nodes_hard_effective": 18}}
        assert _resource_gate_decision(2, nodes_hard, 5)[0] == "hard-overcount", \
            "nodes 과계수(라이브 5<18) 미무효화"
        # nodes hard인데 라이브 노드가 임계 이상 → genuine block(과계수 아님)
        assert _resource_gate_decision(2, nodes_hard, 20)[0] == "hard-block", \
            "라이브 노드 20>=18인데 과계수로 오무효화"
        # 라이브 노드 측정 불가(None) → 교차확인 불가 → genuine block(보수)
        assert _resource_gate_decision(2, nodes_hard, None)[0] == "hard-block", \
            "라이브 측정 불가 시 보수적 block 아님"
        # nodes+servers 복합 hard → nodes 과계수 무효화 불가(servers는 실자원) → block
        mixed = {"trips": [{"metric": "nodes", "level": "hard", "value": 22},
                           {"metric": "servers", "level": "hard", "value": 5}],
                 "measured": {"nodes_hard_effective": 18}}
        assert _resource_gate_decision(2, mixed, 5)[0] == "hard-block", "복합 hard가 과계수로 오무효화"
        assert _resource_gate_decision(3, None, None)[0] == "allow", "미지 exit=보수적 allow"

        # ── t6: 결손 구성 판정(R1-MED-1 순수 로직 — 총수 비교 폐기) ──
        full = {"cso": 1, "worker": 1, "reviewer": 2}
        assert _team_composition_deficit(full)[0] is False, "완전 구성이 결손으로 오판"
        assert _team_composition_deficit({"cso": 2, "worker": 3, "reviewer": 5})[0] is False, \
            "초과 구성이 결손으로 오판"
        # ★네거티브(원 결함): reviewer만 4 — 구 총수 4 비교는 결손 0으로 오판했다
        half, why = _team_composition_deficit({"cso": 0, "worker": 0, "reviewer": 4})
        assert half is True, "반쪽 팀(reviewer만 4)이 결손 0으로 오판(총수 비교 잔재)"
        assert "cso" in why and "worker" in why, "결손 사유에 결손 역할 미명시: %s" % why
        assert _team_composition_deficit({"cso": 1, "worker": 1, "reviewer": 1})[0] is True, \
            "reviewer 1(<2)이 결손 0으로 오판"
        assert _team_composition_deficit({})[0] is True, "빈 카운트가 결손 0으로 오판"
    except AssertionError as e:
        print("javis_bootstrap self-test FAIL: %s" % e, file=sys.stderr)
        return 1
    print("javis_bootstrap self-test OK (레인 격리 3종 + 부서 교리 게이트 2종 + 결손 구성 판정 — "
          "base/dept 판정·불량 레인·락 키·레인↔팩·CEO 티켓 TTL·자원 게이트 결정·구성 결손)")
    return 0


def main(argv):
    # preflight/CI 호환: `--self-test`는 subcommand 없이도 동작(가로채기).
    if "--self-test" in argv:
        return cmd_self_test()
    cmd = argv[1] if len(argv) > 1 else "run"
    if cmd == "issue-ticket":
        return cmd_issue_ticket(argv[2:])
    return {"run": cmd_run, "status": cmd_status,
            "assert-ready": cmd_assert_ready}.get(cmd, cmd_run)()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
