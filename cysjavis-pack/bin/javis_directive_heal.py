#!/usr/bin/env python3
"""javis_directive_heal.py — W5 결정론 디렉티브 치유 (설계 DD-5 · Sim S3-1 · 오너 2026-07-24).

반쪽마스터(라이브 MASTER_DIRECTIVE == CEO 스텁으로 변조) 를 **결정론**(해시 대장 정확 일치)으로 탐지·치유한다.
MASTER_DIRECTIVE 는 User-owned(T0 A1) 이라 pack-update 가 변조 스텁을 '사용자 수정본'으로 보존 → 릴리스만으론
정상화되지 않는다. 이 스크립트가 결정론 치유의 정식 경로다.

파이프라인:
  1) 감지: sha256(MASTER_DIRECTIVE) ∈ heal_hashes.json(역대 스텁 대장) — **정확 일치만·퍼지 금지**(사용자
     정당 수정본 오폭 0·안전핵 ADR-D5). sentinel 합성물이면 base 부분만 추출해 대조(T1 독립행 앵커 파서 동형).
  2) base 복원 우선순위: .pristine/directives/MASTER_DIRECTIVE.md(C03 핀 통과 확인) → .pre-ceo(base_hashes
     대장 일치 시) → 팩 소스. 복원 후 depts.json 부서 수≥1 이면 overlay append(cys-dept recompose 재사용).
  3) 부서 팩 페이로드 전체 sync(S3-1 치명): depts.json 순회 — 각 pack_dir 에 CYS_PACK_DIR 재지정으로
     `cys init-pack`(무 --force·보존 시맨틱, T0 A4 실측 경로). cys 부재/실패 폴백 = System-소유 파일만 복사
     (소유권 판정 `cys pack-ownership`, 부재 시 보수적 bin/·hooks/). 각 부서 디렉티브도 동일 치유.
     죽은 부서(소켓 무응답)는 파일만·reinject 생략.
  4) 라이브 reinject: 소켓 응답 시 `cys reinject --role master`. 실패 graceful.
  5) 실행: 전 단계 멱등·로그·graceful. 부트 락 선점 시 대기→타임아웃 보고(R10 — 부트 비봉쇄).

배선: ①단독 CLI(needs/heal/sync/heal-all/doctor) ②javis_bootstrap 프리플라이트 가드 호출 ③cys doctor --fix.

라이브 무접촉 계약(test_heal.py): 해시 대장은 이 스크립트와 **동봉**(PACK_SELF 상대) — 치유 대상 팩(CYS_PACK_DIR)
과 분리되므로 CYS_PACK_DIR=temp 테스트에서도 대장 로드가 유효하다. import 시 부작용 0(전 로직 함수 내).

실행: python3 javis_directive_heal.py {needs|heal|sync|heal-all|doctor} [--pack DIR] [--json]
exit: 0=치유 불요/성공 · 1=치유 발생(needs) 또는 부분 실패 · 2=내부 오류
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time

# ── 경로 앵커 ─────────────────────────────────────────────────────────────
# PACK_SELF = 이 스크립트가 동봉된 팩(대장·overlay 소스의 권위). 치유 '대상' 팩과 구분(핵심).
SELF = os.path.dirname(os.path.abspath(__file__))
PACK_SELF = os.path.normpath(os.path.join(SELF, ".."))
HEAL_LEDGER = os.path.join(PACK_SELF, "heal_hashes.json")   # 스텁 대장(정확 일치 탐지)
BASE_LEDGER = os.path.join(PACK_SELF, "base_hashes.json")   # 표준 base 대장(사용자 수정본 보호)

MD_REL = os.path.join("directives", "MASTER_DIRECTIVE.md")
OVERLAY_RELS = (os.path.join("directives", "CEO_OVERLAY.md"),
                os.path.join("directives", "CEO_TEMPLATE.md"))   # 전환기 하위호환

# ★T1 독립행 앵커 sentinel(산문·백틱 리터럴 배제) — compose/strip/recompose 와 동형(중복 재구현 아님·공용 규약).
SENT = re.compile(
    r'(?ms)^<!-- CEO-OVERLAY BEGIN v1 sha256:[0-9a-f]{64} -->$\n.*?^<!-- CEO-OVERLAY END -->$\n?')

CYS = os.environ.get("CYS_BIN", "cys")


def log(msg):
    sys.stderr.write("[heal] %s\n" % msg)


# ── 해시·파싱 유틸 ────────────────────────────────────────────────────────
def _sha256_text(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _sha256_file(path):
    try:
        with open(path, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()
    except OSError:
        return None


def _read(path):
    try:
        with open(path, encoding="utf-8") as f:
            return f.read()
    except OSError:
        return None


def _load_ledger(path):
    try:
        with open(path, encoding="utf-8") as f:
            return {h["sha256"] for h in json.load(f).get("hashes", []) if h.get("sha256")}
    except (OSError, ValueError, KeyError, TypeError):
        return set()


def _stub_hashes():
    return _load_ledger(HEAL_LEDGER)


def _base_hashes():
    return _load_ledger(BASE_LEDGER)


def extract_base(text):
    """sentinel 합성물에서 base 부분만 추출(치유 전처리). sentinel 부재면 원문 그대로."""
    if text is None:
        return text
    m = SENT.search(text)
    return (text[:m.start()] + text[m.end():]) if m else text


# ── 1) 감지 ──────────────────────────────────────────────────────────────
def needs_heal(md_sha):
    """md 해시가 스텁 대장에 **정확 일치**할 때만 True(퍼지 금지·오폭 0). 인자 = sha256 hex 문자열."""
    return md_sha in _stub_hashes()


def needs_heal_path(path):
    """파일 경로 기준 치유 필요 판정. 원문 sha 또는 sentinel base 추출 sha 중 하나라도 스텁 대장이면 True."""
    text = _read(path)
    if text is None:
        return False
    stubs = _stub_hashes()
    if _sha256_text(text) in stubs:
        return True
    base = extract_base(text)
    return base is not None and base != text and _sha256_text(base) in stubs


# ── 2) base 복원 ─────────────────────────────────────────────────────────
# ★리뷰어1 fix#1(정확일치 불변식·감지↔복원 대칭): 복원 소스 신뢰는 **정확일치·배포신뢰 루트만**.
#   감지가 "스텁 대장 sha256 정확일치(퍼지 금지)"인 것과 대칭으로, 복원도 사용자 수정본을 임의 base 로
#   덮지 않는다. 종전 `len>=8000` 크기 퍼지 승인은 폐지 — 크기·핀은 **진단 경고로만**(신뢰 판정 불사용).
def _size_diag(text):
    """진단 경고 전용(신뢰 판정 불사용): 표준 base≈28~36KB·스텁≈4.5KB. 소재 크기 이상 시 경고 문자열/None."""
    if text is None:
        return "소스 없음"
    return None if len(text) >= 8000 else "소스 크기 이상(<8KB — 스텁/절단 의심)"


def _base_trusted(text, source_name):
    """복원 소스 수용 규칙(퍼지 금지·감지↔복원 대칭):
      (a) sha256(base) ∈ base_hashes.json 대장 — 정확일치 표준 base(최고 신뢰), 또는
      (b) source == 'pack-self' — heal 코드와 **동봉 배포**된 팩 자신의 표준 base(identity by
          distribution: 서명·배포된 팩 루트라 퍼지 아님).
    스텁은 어느 소스·어느 규칙이든 **절대 거부**(치유 자기부정 차단·불변식 — 스텁 pack-self 도 거부).
    .pristine·.pre-ceo·pack-target 은 (a) 통과 시에만 수용(크기·핀은 신뢰 판정에 불사용)."""
    if text is None:
        return False
    sha = _sha256_text(text)
    if sha in _stub_hashes():          # 스텁을 base 로 되쓰기 금지(감지↔복원 대칭 불변식·최우선)
        return False
    if sha in _base_hashes():          # (a) 정확일치 표준 base
        return True
    if source_name == "pack-self":     # (b) 배포 신뢰 루트(동봉 팩 자신 — 스텁 아님이 위에서 보장됨)
        return True
    return False                       # .pristine/.pre-ceo/pack-target = (a) 통과 필수(퍼지 승인 없음)


def _candidate_bases(pack):
    """복원 소스 우선순위 생성기: (이름, 경로). .pristine → .pre-ceo → 팩 소스(PACK_SELF) → 대상 자신."""
    md = os.path.join(pack, MD_REL)
    yield (".pristine", os.path.join(pack, ".pristine", MD_REL))
    yield (".pre-ceo", md + ".pre-ceo")
    # 팩 자기 소스(배포 신뢰 루트): 대상 팩이 스텁이어도 PACK_SELF(동봉 권위) 의 표준 base 로 복원.
    yield ("pack-self", os.path.join(PACK_SELF, MD_REL))
    yield ("pack-target", md)


def _atomic_write(path, text):
    if text and not text.endswith("\n"):
        text += "\n"
    tmp = "%s.heal.tmp.%d" % (path, os.getpid())
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


def restore_base(pack):
    """우선순위 소스에서 첫 **신뢰**(정확일치 OR pack-self 배포루트) base 로 MASTER_DIRECTIVE 원자 교체.
    신뢰 소스 부재 = **자동 변경 0**·merge-pending 회부(사용자 수정본 보호). 반환: (ok, source, detail)."""
    md = os.path.join(pack, MD_REL)
    warns = []
    for name, src in _candidate_bases(pack):
        if name == "pack-target":
            # 대상 자신 = 감지된 스텁(restore 는 needs_heal True 시만 호출) — 신뢰 소스 아님, 스킵.
            continue
        text = _read(src)
        if text is None:
            continue
        base = extract_base(text)      # 소스가 합성물이어도 base 만 취함
        diag = _size_diag(base)
        if _base_trusted(base, name):
            _atomic_write(md, base)
            wd = (" · 진단경고:%s" % diag) if diag else ""
            return True, name, "%s 에서 표준 base 복원(%s)%s" % (name, _sha256_text(base)[:16], wd)
        if diag:
            warns.append("%s:%s" % (name, diag))
    detail = "신뢰 복원 소스 없음(정확일치/배포루트 부재) — 자동 변경 0·merge-pending 회부(사용자 수정본 보호)"
    if warns:
        detail += " [진단:%s]" % "; ".join(warns)
    return False, None, detail


# ── recompose 재사용(cys-dept — 직접 재구현 금지) ──────────────────────────
def recompose(pack=None, no_overlay=False):
    """cys-dept recompose 재사용 — base 최신표준 재병합 + 부서≥1 이면 overlay append. CYS_PACK_DIR 재지정.
    반환: (rc, out). cys-dept 부재/실패는 graceful(rc, 메시지)."""
    pack = pack or target_pack()
    dept = os.path.join(PACK_SELF, "bin", "cys-dept")
    if not os.path.isfile(dept):
        dept = os.path.join(pack, "bin", "cys-dept")
    if not os.path.isfile(dept):
        return 127, "cys-dept 부재 — recompose 스킵(base 복원만 반영)"
    env = dict(os.environ)
    env["CYS_PACK_DIR"] = pack
    argv = ["bash", dept, "recompose"] + (["--no-overlay"] if no_overlay else [])
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=60, env=env)
        return r.returncode, (r.stdout or r.stderr or "").strip()[:300]
    except Exception as e:
        return 1, "recompose 예외: %s" % e


# ── 3) 부서 팩 sync ───────────────────────────────────────────────────────
def _depts_path():
    return os.environ.get("CYS_DEPTS_JSON") or os.path.expanduser(
        os.path.join("~", ".cys", "depts.json"))


def _load_depts():
    try:
        with open(_depts_path(), encoding="utf-8") as f:
            return json.load(f).get("depts", {}) or {}
    except (OSError, ValueError, AttributeError):
        return {}


def _sock_alive(socket):
    if not socket:
        return False
    env = dict(os.environ)
    env["CYS_SOCKET"] = socket
    try:
        return subprocess.run([CYS, "ping"], env=env, capture_output=True,
                              timeout=3).returncode == 0
    except (subprocess.SubprocessError, OSError):
        return False


def _cys_available():
    try:
        return subprocess.run([CYS, "--version"], capture_output=True,
                              timeout=5).returncode == 0
    except (subprocess.SubprocessError, OSError):
        return False


def _pack_ownership(rel):
    """cys pack-ownership <rel> --quiet → 'system'|'user'|'seed-once'. 실패 시 None."""
    try:
        r = subprocess.run([CYS, "pack-ownership", rel, "--quiet"],
                           capture_output=True, text=True, timeout=5)
        return r.stdout.strip().lower() if r.returncode == 0 else None
    except (subprocess.SubprocessError, OSError):
        return None


def _copy_system_files(dst_pack):
    """폴백: base 팩(PACK_SELF)에서 System-소유 파일만 dst_pack 으로 복사. 소유권 판정 부재 시 보수적으로
    bin/·hooks/ 만(directives 는 heal 로직으로만 — 사용자 소유 헌법 파일 무손상). 반환: 복사 파일 수."""
    copied = 0
    have_ownership = _cys_available()
    for sub in ("bin", "hooks"):
        root = os.path.join(PACK_SELF, sub)
        if not os.path.isdir(root):
            continue
        for base, _dirs, files in os.walk(root):
            for fn in files:
                srcf = os.path.join(base, fn)
                rel = os.path.relpath(srcf, PACK_SELF)
                if have_ownership:
                    own = _pack_ownership(rel)
                    if own is not None and own != "system":
                        continue   # user·seed-once 는 보존(덮지 않음)
                # 소유권 조회 불가 = 보수적으로 bin/·hooks/ 만(이 루프 범위) 복사
                dstf = os.path.join(dst_pack, rel)
                try:
                    os.makedirs(os.path.dirname(dstf), exist_ok=True)
                    shutil.copy2(srcf, dstf)
                    copied += 1
                except OSError as e:
                    log("폴백 복사 실패 %s: %s" % (rel, e))
    return copied


def _valid_pack_dir(pack_dir):
    """★리뷰어1 fix#5: 부서 pack_dir 검증 — realpath·is_dir·홈 하위 봉쇄(경로 탈출 차단). 이상 엔트리는
    경고 후 skip(임의 경로에 System 파일 대량 복사·apply 방지). 반환: (ok, real_or_reason)."""
    if not pack_dir:
        return False, "no-pack_dir"
    try:
        real = os.path.realpath(pack_dir)
        home = os.path.realpath(os.path.expanduser("~"))
    except OSError as e:
        return False, "realpath 실패: %s" % e
    if not os.path.isdir(real):
        return False, "존재하지 않는 디렉토리(is_dir False)"
    try:
        contained = os.path.commonpath([real, home]) == home
    except ValueError:
        contained = False          # 다른 드라이브/혼합 경로 = 미봉쇄로 간주(보수적 거부)
    if not contained:
        return False, "홈 하위 아님(경로 탈출 차단): %s" % real
    return True, real


def _sync_one_dept(name, entry):
    """부서 1개 sync: pack_dir 검증 → init-pack(보존) → 폴백 복사 → 디렉티브 치유 → alive 면 reinject."""
    pack_dir = entry.get("pack_dir")
    socket = entry.get("socket")
    res = {"dept": name, "pack_dir": pack_dir, "alive": False,
           "method": None, "healed": False, "reinject": "skip"}
    ok, real = _valid_pack_dir(pack_dir)
    if not ok:
        res["method"] = "이상 pack_dir skip(%s)" % real
        log("부서 %s pack_dir 검증 실패 — skip: %s" % (name, real))
        return res
    pack_dir = real
    res["pack_dir"] = pack_dir
    alive = _sock_alive(socket)
    res["alive"] = alive

    # ① CYS_PACK_DIR 재지정 init-pack(무 --force·보존 시맨틱, T0 A4)
    used_fallback = False
    if _cys_available():
        env = dict(os.environ)
        env["CYS_PACK_DIR"] = pack_dir
        try:
            r = subprocess.run([CYS, "init-pack"], env=env,
                               capture_output=True, text=True, timeout=120)
            if r.returncode == 0:
                res["method"] = "cys init-pack(보존)"
            else:
                used_fallback = True
                log("부서 %s init-pack 실패(rc=%s) → 폴백" % (name, r.returncode))
        except (subprocess.SubprocessError, OSError) as e:
            used_fallback = True
            log("부서 %s init-pack 예외 → 폴백: %s" % (name, e))
    else:
        used_fallback = True

    # ② 폴백: System-소유 파일만 복사(cys 부재/실패)
    if used_fallback:
        n = _copy_system_files(pack_dir)
        res["method"] = "폴백 System 복사(%d파일)" % n

    # ③ 부서 디렉티브 치유(base heal — 사용자 소유 헌법 파일도 스텁이면 결정론 복원).
    #   do_recompose=False: cys-dept recompose 는 PACK_DEFAULT($HOME/.cys/pack)만 대상이라 부서 팩에
    #   대해 호출하면 엉뚱한 팩을 건드린다. 부서 디렉티브는 base 복원만(CEO 오버레이는 base-master 전용).
    try:
        hb = heal_base(pack_dir, do_recompose=False)
        res["healed"] = hb.get("healed", False)
        res["heal_detail"] = hb.get("detail")
    except Exception as e:
        res["heal_detail"] = "heal 예외: %s" % e

    # ④ alive 면 reinject, 죽은 부서는 파일만·생략(S3-1)
    if alive:
        res["reinject"] = "ok" if reinject_live(socket) else "fail"
    return res


def sync_dept_packs():
    """depts.json 순회 부서 팩 전체 sync(S3-1 치명 확장). 반환: 결과 리스트."""
    depts = _load_depts()
    out = []
    for name, entry in depts.items():
        if not isinstance(entry, dict):
            continue
        try:
            out.append(_sync_one_dept(name, entry))
        except Exception as e:
            out.append({"dept": name, "error": str(e)})
    return out


# ── 4) 라이브 reinject ────────────────────────────────────────────────────
def reinject_live(socket=None):
    """소켓 응답 시 cys reinject --role master. 실패 graceful(False). socket None = 기본(base) 데몬."""
    if socket is not None and not _sock_alive(socket):
        return False
    env = dict(os.environ)
    if socket:
        env["CYS_SOCKET"] = socket
    try:
        r = subprocess.run([CYS, "reinject", "--role", "master"], env=env,
                           capture_output=True, text=True, timeout=30)
        return r.returncode == 0
    except (subprocess.SubprocessError, OSError):
        return False


# ── 5) 부트 락(R10 — 부트 비봉쇄) ─────────────────────────────────────────
def _state_dir():
    return os.environ.get("CYS_STATE_DIR") or os.path.expanduser(
        os.path.join("~", ".cys", "state"))


# ★리뷰어1 fix#3: DEMOTE_INCOMPLETE 마커 수명 완결. cys-dept ceo_demote 가 strip post-verify 불완전 시
#   기록하는 마커(경로 규약 동일) — 치유가 성공하면 해소(제거)하고, needs(진단) 출력이 존재를 표면화한다.
def _demote_marker_path():
    return os.path.join(_state_dir(), "ceo-demote-incomplete")


def _clear_demote_marker():
    """demote 불완전 마커 제거(치유=해소). 존재해서 지웠으면 True, 없거나 실패면 False."""
    p = _demote_marker_path()
    try:
        if os.path.exists(p):
            os.remove(p)
            return True
    except OSError:
        pass
    return False


def _boot_lock_present():
    """부트 진행 중 락 존재 여부(bootstrap-*.lock / cys-boot.lock 양 규약). 존재=부트 중."""
    import glob
    sd = _state_dir()
    if os.path.exists(os.path.join(sd, "cys-boot.lock")):
        return True
    return bool(glob.glob(os.path.join(sd, "bootstrap-*.lock")))


def wait_boot_lock(timeout=30, interval=1.0):
    """부트 락 선점 시 대기 → 타임아웃이면 (False, 사유). 락 없으면 즉시 (True, ''). 부트 비봉쇄:
    치유가 부트와 락 순서 경합하지 않도록 부트를 양보(치유는 부트 후 사후 단계가 정석)."""
    deadline = time.time() + timeout
    waited = False
    while _boot_lock_present():
        waited = True
        if time.time() >= deadline:
            return False, "부트 락 %ss 대기 타임아웃 — 치유 연기(부트 비봉쇄·R10)" % timeout
        time.sleep(interval)
    return True, ("부트 락 해제 대기 후 진행" if waited else "")


# ── base 치유(감지+복원+recompose) ────────────────────────────────────────
def heal_base(pack=None, do_recompose=True):
    """단일 팩 base 치유(멱등). 감지→복원→(부서≥1) recompose. 반환: dict(needs/healed/source/detail)."""
    pack = pack or target_pack()
    md = os.path.join(pack, MD_REL)
    text = _read(md)
    if text is None:
        return {"pack": pack, "needs": False, "healed": False, "detail": "MASTER_DIRECTIVE 부재(skip)"}
    need = needs_heal_path(md)
    if not need:
        return {"pack": pack, "needs": False, "healed": False, "detail": "base 무결(치유 불요·멱등)"}
    log("반쪽마스터 감지: %s (sha=%s)" % (md, _sha256_file(md)[:16]))
    ok, source, detail = restore_base(pack)
    if not ok:
        return {"pack": pack, "needs": True, "healed": False, "detail": detail}
    rc_detail = ""
    if do_recompose:
        rc, out = recompose(pack)
        rc_detail = " · recompose(rc=%s): %s" % (rc, out)
    # ★fix#3: 치유 성공 = demote 불완전 마커 해소(존재 시 제거).
    cleared = _clear_demote_marker()
    return {"pack": pack, "needs": True, "healed": True, "source": source,
            "demote_marker_cleared": cleared, "detail": detail + rc_detail}


# ── 전체 치유 오케스트레이션 ──────────────────────────────────────────────
def target_pack():
    """치유 대상 팩. CYS_PACK_DIR 우선(부서 데몬이 자식에 전파), 없으면 ~/.cys/pack."""
    return os.environ.get("CYS_PACK_DIR") or os.path.expanduser(os.path.join("~", ".cys", "pack"))


def heal_all(boot_wait=30):
    """전체 치유: 부트 락 양보 → base 치유 → 라이브 reinject → 부서 팩 전체 sync. 멱등·graceful.
    반환: 요약 dict."""
    proceed, why = wait_boot_lock(timeout=boot_wait)
    summary = {"boot_lock": why or "clear", "proceeded": proceed}
    if not proceed:
        summary["skipped"] = True
        log(why)
        return summary
    base = heal_base(target_pack(), do_recompose=True)
    summary["base"] = base
    # base 데몬 라이브 reinject(치유 반영) — 소켓 미지정=기본 데몬. 실패 graceful.
    summary["base_reinject"] = "ok" if reinject_live(None) else "skip/fail"
    summary["depts"] = sync_dept_packs()
    return summary


# ── CLI ───────────────────────────────────────────────────────────────────
def _emit(obj, as_json):
    if as_json:
        print(json.dumps(obj, ensure_ascii=False, indent=1))
    else:
        print(json.dumps(obj, ensure_ascii=False))


def main(argv=None):
    p = argparse.ArgumentParser(description="W5 결정론 디렉티브 치유(DD-5)")
    sub = p.add_subparsers(dest="cmd", required=True)

    def add_common(c):
        c.add_argument("--pack", default=None, help="치유 대상 팩(기본: CYS_PACK_DIR 또는 ~/.cys/pack)")
        c.add_argument("--json", action="store_true")

    cn = sub.add_parser("needs", help="치유 필요 감지만(쓰기 0) — exit 1=치유 필요")
    add_common(cn)
    ch = sub.add_parser("heal", help="base 치유(감지+복원+recompose)")
    add_common(ch)
    cs = sub.add_parser("sync", help="부서 팩 전체 sync(depts.json 순회)")
    cs.add_argument("--json", action="store_true")
    ca = sub.add_parser("heal-all", help="부트 락 양보→base 치유→reinject→부서 sync")
    ca.add_argument("--json", action="store_true")
    ca.add_argument("--boot-wait", type=int, default=30)
    cd = sub.add_parser("doctor", help="cys doctor --fix 연계 진입점(heal-all 별칭·비치명)")
    cd.add_argument("--json", action="store_true")
    cd.add_argument("--boot-wait", type=int, default=30)

    a = p.parse_args(argv)
    try:
        if a.cmd == "needs":
            pack = a.pack or target_pack()
            md = os.path.join(pack, MD_REL)
            need = needs_heal_path(md)
            # ★fix#3 소비자: demote 불완전 마커 존재를 진단 출력에 표면화(마커의 단일 소비자 확보).
            mk = _demote_marker_path()
            _emit({"pack": pack, "needs_heal": need, "sha": _sha256_file(md),
                   "demote_incomplete_marker": mk if os.path.exists(mk) else None}, a.json)
            return 1 if need else 0
        if a.cmd == "heal":
            r = heal_base(a.pack or target_pack(), do_recompose=True)
            _emit(r, a.json)
            return 1 if (r.get("needs") and not r.get("healed")) else 0
        if a.cmd == "sync":
            r = sync_dept_packs()
            _emit({"depts": r}, a.json)
            return 0
        if a.cmd in ("heal-all", "doctor"):
            r = heal_all(boot_wait=a.boot_wait)
            _emit(r, a.json)
            base = r.get("base") or {}
            if base.get("needs") and not base.get("healed"):
                return 1
            return 0
    except Exception as e:      # 부트체인 무해성(R10): 어떤 예외도 비치명(exit 2)·부트 계속
        log("치유 내부 오류(비치명): %s" % e)
        _emit({"error": str(e)}, getattr(a, "json", False))
        return 2
    return 2


if __name__ == "__main__":
    sys.exit(main())
