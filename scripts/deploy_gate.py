#!/usr/bin/env python3
"""deploy_gate.py — cys/cysd 게이트 배포 (정석복구 ④ · 영구 재발방지).

★실행 금지 — 오너 승인 후 master 감독 하에서만 `--execute`로 실행한다.
  scratch/deploy_*_swap.py(미서명 ad-hoc·빌드세대 스큐·Desktop SRC=iCloud 재발원)를 대체한다.

게이트 체인(순서 고정):
  1. 동시성 락(flock) 획득    — 다중 배포 스크립트 병렬 가동에 의한 파일 충돌 차단
  2. SRC=~/dev 강제          — Desktop(iCloud 동기화 경합·' 2' 충돌사본 재발원) 차단
  3. iCloud xattr 부재        — 빌드 dir·산출물에 com.apple.CloudDocs 있으면 중단
  4. 동세대 검증              — cys·cysd mtime 근접 + size 존재(빌드세대 스큐 차단)
  5. ★현재상태 백업(run-id)   — 매 실행 새 backup_dir에: 앱 번들 전체 ditto 복사(서명·xattr·nested 보존)
                                + 각 타깃 lstat/readlink/sha256 inventory + 개별 백업(symlink-aware)
  6. ★번들 원자 교체(ATOMIC-1) — 계약은 scripts/atomic_bundle.py(정본 Rust src/app_bundle.rs):
       ①같은 FS 스테이징에 현재 번들 완본 복사 → ②스테이징 **안에서만** 새 바이너리 교체·재서명
       → ③완본 검증(Info.plist·실행물 3종·codesign) → ④renamex_np(RENAME_SWAP) 1콜 교체
       → ⑤교체 후 재검증(**구조 완본성만** · 실패 시 자동 원복).
       ★⑤에서 봉인(codesign)을 빼는 이유는 swap_bundle_into_place 주석 — 스왑 직후의 설치본은
         살아있는 공유 객체라 구 데몬의 `.pyc` 한 건이 정상 업그레이드를 자동 원복시킨다(HIGH-2).
         설치될 비트의 봉인 판정은 ③(같은 비트·스왑 직전)에 그대로 있고, 최종 봉인 이탈은
         seal_status 로 **갈래를 구별**해(추가/변조/소실) 변조·소실만 hard fail 한다.
       ★2026-08-01 실사고 재발 차단: **살아있는 /Applications/cys.app 안에 개별 파일을 쓰지 않는다.**
         종전 방식(번들 안 cp + 개별 codesign)은 App Management(TCC)가 특정 파일만 막아도 나머지가
         교체되어 '세대 혼합 반쪽 번들'을 확정했다(cp -R·ditto 둘 다 트랜잭션이 아님 — 실측).
  7. 번들 밖 타깃 교체         — brew 심링크/파일은 개별 os.replace(같은 FS rename = 원자)
  8. cys --version 스모크     — ★cysd 직접 실행 금지(부팅 부작용)·cysd는 codesign -v 만
  9. 실패 시 자동 롤백         — 백업 번들을 같은 계약(스테이징→검증→단일 스왑)으로 되돌린다.
       ★합격 기준은 **구조 완본성**이다(봉인 제외 — 실행 이력이 있는 백업은 `.pyc` 때문에 봉인이
         정당하게 깨져 있고 롤백은 재서명을 하지 않는다). 봉인은 **백업 자신의 상태와 대조**해
         "백업은 유효했는데 복원본이 깨짐"만 hard fail 로 잡는다(HIGH-1).
  10. 데몬 재시작             — ★--execute 성공 시 자동 실행(2026-07-06 오너 지시 — 수동 kill·재가동 폐기):
       drain(저장 신호·best-effort) → system.identify 정확 PID로 종료 및 respawn 폴링. 실패 시 hard fail.
       구 데몬 종료 후 launchd KeepAlive가 새 번들의 cysd를 respawn하고, 새 cysd의 auto-restore
       (phoenix)가 노드를 복원한다. 단독 재시작은 기존대로 --restart.
"""
import hashlib
import json
import os
import shutil
import socket as _socket
import subprocess
import sys
import time
import fcntl

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import atomic_bundle as ab  # 번들 교체 계약(ATOMIC-1) — 이 파일은 정책, 저기가 기제.

# ★SRC는 반드시 ~/dev(iCloud 밖). $HOME 기반(범용 배포 이식성).
# CYS_DEPLOY_SRC로 오버라이드 가능 — 동시 빌드가 target/를 휘젓는 환경에서 검증된 격리 스냅샷
# 디렉토리에서 배포하기 위함(여전히 gate_src_path의 ~/dev 검증을 거친다).
SRC = os.path.expanduser(os.environ.get("CYS_DEPLOY_SRC", "~/dev/cys-terminal/target/release"))
APP_BUNDLE = "/Applications/cys.app"
APP_MACOS = "/Applications/cys.app/Contents/MacOS"
BREW = "/opt/homebrew/bin"
BACKUP_BASE = os.path.join(SRC, "deploy_backups")

# ★번들 안 실행물은 **스테이징 사본에만** 쓴다(ATOMIC-1). 실제 설치본은 단일 스왑으로만 바뀐다.
# cys-app = Tauri GUI(웹뷰). ui/dist를 generate_context!로 컴파일타임 임베드하므로 UI 변경은
# cys-app 재빌드로만 반영된다(brew엔 없음 — GUI 전용). UI 변경 배포 시 cargo build -p cys-app 선행 필수.
BUNDLE_EXECUTABLES = ["cys", "cysd", "cys-app"]
# 번들 **밖** 타깃 — 개별 파일이라 os.replace(같은 FS rename)가 그 자체로 원자적이다.
EXTERNAL_TARGETS = [
    ("cys", f"{BREW}/cys"),
    ("cysd", f"{BREW}/cysd"),
]
# inventory/백업/롤백은 두 갈래를 함께 본다(기존 계약 유지 — 백업 대상 축소 금지).
TARGETS = [(n, f"{APP_MACOS}/{n}") for n in BUNDLE_EXECUTABLES] + EXTERNAL_TARGETS

# ★교체 후 재검증(계약 ④)의 기준 — **구조 완본성만**(봉인 제외). 근거는 swap_bundle_into_place 주석.
#   스왑 직전 ②는 종전대로 봉인까지 본다 — 검사를 없앤 게 아니라 **경합이 없는 쪽으로 옮긴** 것이다.
POST_SWAP_VERIFY = {"codesign": False}
# ★롤백 복원의 기준도 **구조 완본성**이다(봉인 제외). 근거는 rollback() 주석 —
#   백업은 '실행 이력이 있는 설치본'의 사본이라 봉인이 정당하게 깨져 있을 수 있고, 롤백은 재서명을
#   하지 않는다(원본 서명 보존이 백업의 목적). 봉인은 판정 기준이 아니라 **대조 항목**으로 다룬다.
ROLLBACK_VERIFY = {"codesign": False}

# flock 핸들 전역 보유 (가비지 컬렉션 해제 방지)
LOCK_FILE = None

def die(msg, code=1):
    print(f"❌ {msg}")
    sys.exit(code)

def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True)

def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()

# ── 봉인(codesign) 이탈의 갈래 판정 ────────────────────────────────────────
#
# `codesign --verify --deep --strict --verbose` 는 이탈을 한 줄씩 이름으로 말한다(2026-08-01 실측):
#     file added:    <경로>   — 봉인 이후 **추가**된 파일
#     file modified: <경로>   — 봉인된 파일이 **변조**됨
#     file missing:  <경로>   — 봉인된 파일이 **사라짐**
# 위험도가 전혀 다른데 rc(0/1) 하나로 접으면 셋을 구별할 수 없다. 우리가 아는 **유일한 양성(benign)
# 갈래는 바이트코드 캐시 추가** 하나다: 동봉 python 이 번들 안에 `.pyc` 를 쓰면 그 한 건으로 봉인이
# 깨진다(SEAL-1 = 바로 그 예방책 · lib.rs `ENV_PY_NO_BYTECODE`). 나머지(변조·소실·중첩 서명 이상·
# 판독 불가 메시지)는 전부 **진짜 손상**으로 본다 — 의심스러우면 손상 쪽으로 접는다.
#
# ★이 판정은 게이트를 넓히는 장치가 아니라 **좁히는** 장치다: 종전에는 "봉인 실패 = 전부 롤백"이라
#   양성 한 건이 정상 배포를 통째로 되돌렸고(HIGH-2), 그 롤백은 원인(외부 프로세스의 쓰기)을 하나도
#   고치지 못한 채 옛 세대를 되살렸다. 이제 양성만 통과하고 **변조·소실은 종전대로 hard fail** 이다.
BYTECODE_SUFFIXES = (".pyc", ".pyo")
_DEVIATION_PREFIXES = ("file added:", "file modified:", "file missing:")


def _is_bytecode_cache(p):
    return "/__pycache__/" in p or p.endswith(BYTECODE_SUFFIXES)


def seal_status(path):
    """(ok, benign, detail) — ok=봉인 유효 / benign=이탈이 '바이트코드 캐시 추가'뿐 / detail=요약 원문."""
    r = run(["codesign", "--verify", "--deep", "--strict", "--verbose", path])
    if r.returncode == 0:
        return True, True, ""
    lines = [l.strip() for l in (r.stdout + r.stderr).splitlines() if l.strip()]
    detail = " | ".join(lines)[:600] or f"codesign exit {r.returncode}"
    deviations = [l for l in lines if l.startswith(_DEVIATION_PREFIXES)]
    # 이탈 줄이 하나도 없으면(예: "code has no resources but signature indicates…") 정체를 모르는
    # 실패다 → benign 으로 접지 않는다(관측 부재는 통과가 아니다).
    benign = bool(deviations) and all(
        l.startswith("file added:") and _is_bytecode_cache(l.split(":", 1)[1].strip())
        for l in deviations
    )
    return False, benign, detail

# ── 게이트 0: 동시성 락 (flock) ─────────────────────────────────────────
def gate_concurrency_lock():
    global LOCK_FILE
    lock_path = "/tmp/cys_deploy_gate.lock"
    try:
        LOCK_FILE = open(lock_path, "w")
        fcntl.flock(LOCK_FILE, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        die("이미 다른 cys 배포 게이트 프로세스가 실행 중입니다 (flock 락 획득 실패).")
    print("  ✓ 동시성 락 획득(flock)")

# ── 게이트 1: SRC=~/dev 강제 ──────────────────────────────────────────
def gate_src_path():
    real = os.path.realpath(SRC)
    if "/Desktop/" in real or "/dev/" not in real:
        die(f"SRC가 ~/dev 밖 — iCloud 재발 위험: {real}")
    print(f"  ✓ SRC ~/dev 내부: {real}")

# ── 게이트 2: iCloud xattr 부재 ───────────────────────────────────────
def gate_no_icloud():
    for p in [SRC] + [os.path.join(SRC, n) for n, _ in TARGETS]:
        if "com.apple.CloudDocs" in run(["xattr", p]).stdout:
            die(f"iCloud 동기화 xattr 감지(빌드 경합 위험): {p}")
    print("  ✓ iCloud xattr 없음(빌드 dir·산출물)")

# ── 게이트 3: 동세대(cys·cysd) ────────────────────────────────────────
def gate_same_generation():
    cys, cysd = os.path.join(SRC, "cys"), os.path.join(SRC, "cysd")
    for f in (cys, cysd):
        if not os.path.isfile(f):
            die(f"빌드 산출물 누락: {f} — 재빌드 필요")
    dt = abs(os.path.getmtime(cys) - os.path.getmtime(cysd))
    if dt > 300:
        die(f"cys·cysd 빌드세대 스큐 {dt:.0f}s > 300s — 동시 빌드 필요(cargo build --bin cys --bin cysd)")
    print(f"  ✓ 동세대 mtimeΔ={dt:.0f}s cys={os.path.getsize(cys):,}B cysd={os.path.getsize(cysd):,}B")

# ── 타깃 inventory(symlink-aware) ────────────────────────────────────
def inventory_target(dst):
    existed = os.path.exists(dst) or os.path.islink(dst)
    d = {
        "dst": dst,
        "in_bundle": dst.startswith(APP_MACOS),
        "is_symlink": os.path.islink(dst),
        "existed": existed
    }
    if d["is_symlink"]:
        d["link_target"] = os.readlink(dst)
        d["size"] = d["sha256"] = None
    elif existed:
        d["link_target"] = None
        d["size"] = os.path.getsize(dst)
        d["sha256"] = sha256(dst)
    else:
        d["link_target"] = d["size"] = d["sha256"] = None
    return d

# ── 게이트 4: 현재상태 백업(run-id) + 번들 ditto + 서명 inventory ──────
def backup_current_state():
    run_id = str(int(time.time()))
    backup_dir = os.path.join(BACKUP_BASE, run_id)
    os.makedirs(backup_dir, exist_ok=False)  # run-id 고유 → stale 롤백 차단
    inv = {"run_id": run_id, "backup_dir": backup_dir, "bundle_sig": {}, "targets": []}

    # ★앱 번들 전체 ditto 복사(원본 서명·xattr·nested 보존 — BLOCKER2 가역성).
    # zip 아님이 디렉토리 복사(ditto --rsrc --extattr --acl)
    bundle_bak = os.path.join(backup_dir, "cys.app")
    r = run(["ditto", "--rsrc", "--extattr", "--acl", APP_BUNDLE, bundle_bak])
    if r.returncode != 0:
        die(f"앱 번들 ditto 백업 실패(가역성 미확보): {r.stderr.strip()}")
    inv["bundle_bak"] = bundle_bak
    inv["bundle_xattrs"] = run(["xattr", APP_BUNDLE]).stdout.splitlines()

    def sig(args):
        r = run(args)
        return r.stdout + r.stderr

    inv["bundle_sig"]["codesign_dvvv"] = sig(["codesign", "-dvvv", APP_BUNDLE])
    inv["bundle_sig"]["entitlements"] = sig(["codesign", "-d", "--entitlements", ":-", APP_BUNDLE])
    inv["bundle_sig"]["deep"] = sig(["codesign", "-dvvv", "--deep", APP_BUNDLE])

    for i, (_, dst) in enumerate(TARGETS):
        meta = inventory_target(dst)
        if not meta["is_symlink"] and meta["existed"]:
            bcopy = os.path.join(backup_dir, f"t{i}_{os.path.basename(dst)}.bak")
            shutil.copy2(dst, bcopy)
            meta["backup_copy"] = bcopy
        inv["targets"].append(meta)

    with open(os.path.join(backup_dir, "inventory.json"), "w") as f:
        json.dump(inv, f, indent=2, ensure_ascii=False)
    print(f"  ✓ 현재상태 백업(run-id={run_id}): 번들 ditto 복사 + symlink-aware inventory → {backup_dir}")
    return inv

# ── ★번들 원자 교체 (ATOMIC-1 계약 ①②③④⑤) ───────────────────────────
#
# 종전 구현은 `/Applications/cys.app/Contents/MacOS/*` 에 **직접** cp 하고 그 자리에서 codesign 했다.
# 그것이 2026-08-01 사고의 형태다: App Management(TCC)가 번들 안 특정 파일 쓰기를 막아도 `cp`·`ditto`
# 는 멈추지 않고 나머지를 전부 교체해, 에러 한 줄만 남긴 채 세대 혼합 반쪽 번들을 확정한다.
# 지금은 **살아있는 설치본에 개별 파일을 쓰지 않는다** — 모든 부분 쓰기는 스테이징 사본 안에서 끝나고,
# 설치본은 검증을 통과한 완본과 **한 번의 시스템 콜로** 자리를 바꾼다.
def stage_new_bundle():
    """① 현재 번들을 같은 FS 스테이징으로 완본 복사 → ② 그 사본 **안에서만** 새 바이너리 교체·재서명."""
    staged = ab.staging_path(APP_BUNDLE)
    ab.stage_copy(APP_BUNDLE, staged)          # rc 검사 포함(부분 복사 무음 통과 차단)
    print(f"  ✓ ① 스테이징 완본 복사 → {staged}")
    macos = os.path.join(staged, "Contents", "MacOS")
    for name in BUNDLE_EXECUTABLES:
        src = os.path.join(SRC, name)
        if not os.path.isfile(src):
            raise RuntimeError(f"빌드 산출물 누락: {src} — 재빌드 필요(스테이징 폐기·설치본 무접촉)")
        shutil.copy2(src, os.path.join(macos, name))
        print(f"  ✓ ② 스테이징 교체 {name} ({os.path.getsize(src):,}B)")
    # 재서명은 **스테이징 안에서** — ★xattr 제거를 서명 '前'에(신규 ad-hoc 봉인).
    for name in BUNDLE_EXECUTABLES:
        p = os.path.join(macos, name)
        run(["xattr", "-c", p])
        r = run(["codesign", "--force", "--sign", "-", p])
        if r.returncode != 0:
            raise RuntimeError(f"codesign {p}: {r.stderr.strip()}")
    run(["xattr", "-cr", staged])
    r = run(["codesign", "--force", "--deep", "--sign", "-", staged])
    if r.returncode != 0:
        raise RuntimeError(f"codesign --deep 스테이징: {r.stderr.strip()}")
    print("  ✓ ② 스테이징 재서명(xattr 제거 → codesign --force --deep)")
    return staged

def swap_bundle_into_place(staged):
    """③ 완본 검증 → ④ 단일 스왑 → ⑤ 교체 후 재검증(실패 시 자동 원복).
    실패는 전부 예외로 나가고, 그때 설치본은 **손대지 않은 옛 번들**이다.

    ★HIGH-2 수리(스왑 직후 재검증의 codesign 자충 · 실제 0.14.9→0.14.10 사용자 경로 결함):
      ②(스왑 직전 스테이징)는 종전대로 **봉인까지** 본다. ④(스왑 직후 설치 위치)는 **구조 완본성만**
      본다. 이유 —
        · 스왑은 `renamex_np(RENAME_SWAP)` 한 콜이라 **내용을 한 바이트도 바꾸지 않는다**. 방금 ②에서
          봉인이 유효하다고 확인한 그 비트가 그대로 자리를 옮긴다. 그러므로 ④의 봉인 검사는 ②의
          재확인이 아니라 **스왑~검증 사이에 누가 끼어들었는가**를 보는 경합 관측이다.
        · 그 '누군가'는 실재한다: 구 데몬(0.14.9 · SEAL-1 env 없음)이 `/Applications/cys.app/Contents/
          Resources/runtime/python/bin/python3` 를 **절대경로로** 부르면, 스왑이 끝난 그 순간부터 그
          경로는 **새 번들**을 가리키고 인터프리터는 거기에 `.pyc` 를 쓴다. 파일 추가 한 건으로
          `codesign --verify` 는 실패하고(실측: `a sealed resource is missing or invalid` +
          `file added: …/__pycache__/x.pyc`), 종전 코드는 그것을 ④ 실패로 읽어 **자동 원복**을 발동시켰다.
          = 정상 업그레이드가 스스로 되돌아간다.
        · 원복은 그 상황의 처방이 될 수 없다: 옛 세대를 되살려도 원인(외부 프로세스의 쓰기)은 그대로고,
          되살아난 옛 번들 역시 같은 이유로 봉인이 깨져 있다.
      **택하지 않은 대안 ⓑ(데몬 정지를 스왑보다 앞으로)**: 단독으로는 결함을 닫지 못한다 —
      launchd 는 `KeepAlive=true`(src/launchd.rs:45)로 등록돼 있어 죽인 데몬을 즉시 되살리고, 되살아난
      데몬이 같은 절대경로 python 을 다시 부른다. 게다가 스왑 전에 데몬을 내리면 KeepAlive 가
      **브루 타깃 교체·스모크 전에** 새 번들의 cysd 를 띄워 세대 혼합 창을 새로 만든다(현재는 배포가
      끝난 뒤 restart_daemon 이 순서를 통제한다). 그래서 ⓐ만 취하고, 봉인 이탈은 아래 최종 검증에서
      **갈래를 구별해** 보고한다(seal_status — 변조·소실은 종전대로 hard fail).
      ※ 봉인 게이트 자체는 사라지지 않는다: 설치될 비트에 대한 봉인 판정은 ②(같은 비트·스왑 직전)에
        그대로 있고, 스테이징 재서명 실패는 stage_new_bundle 이 이미 예외로 막는다."""
    previous = ab.install_atomically(staged, APP_BUNDLE, post_verify_kwargs=POST_SWAP_VERIFY)
    # 옛 번들은 **자동 삭제하지 않는다**(비가역 조작은 도구가 임의로 하지 않는다). 숨김 경로이고,
    # 다음 실행의 crash_recovery() 가 거둔다. 즉시 비우려면 그 경로를 직접 지우면 된다.
    print(f"  ✓ ③④⑤ 번들 원자 교체 완료 — 옛 번들 보관(다음 실행 시 자동 정리): {previous}")
    return previous

# ── 번들 밖 타깃(brew) 교체 — 개별 os.replace 가 그 자체로 원자적 ─────────
def replace_external_targets():
    for srcname, dst in EXTERNAL_TARGETS:
        tmp = dst + ".new-deploygate"
        if os.path.lexists(tmp):
            os.remove(tmp)
        shutil.copy2(os.path.join(SRC, srcname), tmp)
        run(["xattr", "-c", tmp])
        r = run(["codesign", "--force", "--sign", "-", tmp])
        if r.returncode != 0:
            raise RuntimeError(f"codesign {tmp}: {r.stderr.strip()}")
        # os.replace는 dst가 regular file이든 symlink이든 원자적으로 대체(rename-over) —
        # symlink일 때 os.remove 先을 두면 부재 창이 생긴다(MEDIUM·codex) → 직접 replace.
        # ★서명을 tmp 에 먼저 하는 이유: 자리에 앉힌 뒤 서명하면 '미서명으로 노출되는 창'이 생긴다.
        os.replace(tmp, dst)
        print(f"  ✓ 교체 {dst} ({os.path.getsize(dst):,}B)")

# ── 스모크: cys --version (★cysd 직접실행 금지) ──────────────────────
def smoke():
    for _, dst in TARGETS:
        if os.path.basename(dst) == "cys":
            r = run([dst, "--version"])
            if r.returncode != 0:
                raise RuntimeError(f"cys --version 스모크 실패 {dst}: {r.stderr.strip()}")
            print(f"  ✓ cys --version: {r.stdout.strip()} ({dst})")
        else:
            r = run(["codesign", "-v", dst])
            if r.returncode != 0:
                raise RuntimeError(f"cysd codesign -v 실패 {dst}: {r.stderr.strip()}")
            print(f"  ✓ cysd codesign -v PASS (직접 실행 안 함) {dst}")

# ── 롤백: 배포와 **같은 원자 계약**(스테이징 → 완본 검증 → 단일 스왑 → 재검증) ──
def rollback(inv):
    print("⏪ 롤백 — ATOMIC-1 계약으로 복원 개시")
    # ★종전 구현은 rename 2단 스왑이라 그 사이 번들이 존재하지 않는 창이 있었고(스스로 '완전 원자
    #   아님'을 주석에 명시), 두 번째 rename 이 실패해도 되돌리지 않았다. 이제 배포 경로와 **같은
    #   계약**을 쓴다: 백업을 스테이징으로 복원 → 완본 검증 → 단일 스왑 → 재검증(실패 시 자동 원복).
    # ★HIGH-1 수리(롤백 안전망이 스스로를 막던 결함):
    #   백업은 **그동안 실행된 설치본**의 ditto 사본이다. 살아 있던 번들에는 동봉 python 이 남긴
    #   `.pyc` 가 들어 있을 수 있고(SEAL-1 이전 세대에서는 사실상 항상), 파일 추가 한 건이면
    #   `codesign --verify` 는 실패한다 — 구조는 **완본인데** 봉인만 정당하게 깨진 상태다.
    #   그런데 롤백은 재서명을 하지 않는다(원본 서명 보존이 백업의 목적이다). 그래서 종전처럼
    #   봉인 포함 기본 검증으로 install_atomically 를 부르면 계약 ②에서 걸려 **되돌릴 수 있는
    #   상황에서 되돌리지 못한다** — 배포가 막 실패한, 안전망이 가장 절실한 그 순간에.
    #   ⇒ 롤백의 옳은 합격 기준은 **구조 완본성**이다(Info.plist·실행물 3종·필수 리소스).
    #     봉인은 기준에서 빼되 버리지는 않는다: 아래 최종 검증이 **백업 자신의 봉인 상태와 대조**해
    #     "백업은 멀쩡했는데 복원본이 깨졌다"(=복원 결함)만 hard fail 로 잡는다.
    bundle_bak = inv.get("bundle_bak")
    restored_from_backup = False
    if bundle_bak and os.path.exists(bundle_bak):
        staged_app = ab.staging_path(APP_BUNDLE)
        try:
            ab.stage_copy(bundle_bak, staged_app)
            ab.install_atomically(staged_app, APP_BUNDLE, ROLLBACK_VERIFY)
        except Exception as e:
            # 되돌리기 실패는 사람이 손으로 복구해야 한다 — 경로를 그대로 노출하고 hard fail.
            die(f"  [롤백] 번들 복원 실패: {e}\n  백업 번들: {bundle_bak}", 1)
        restored_from_backup = True
        print("  ⏪ 앱 번들 복원 완료(원본 서명 보존 · 단일 스왑 · 구조 완본성 재검증 통과)")
    else:
        print(f"  ⚠ 백업 번들 부재({bundle_bak}) — 번들 복원 생략(번들 밖 타깃만 복원)")

    # xattr 대조 출력
    orig_xattrs = inv.get("bundle_xattrs", [])
    curr_xattrs = run(["xattr", APP_BUNDLE]).stdout.splitlines()
    print(f"  ✓ xattr 대조 출력: 원본={orig_xattrs} -> 복원={curr_xattrs}")

    # brew 및 기타 타깃 복원
    for meta in inv["targets"]:
        if meta["in_bundle"]:
            continue
        dst = meta["dst"]
        
        # existed가 false면 제거 상태 유지(원래 부재 복원)
        if not meta["existed"]:
            if os.path.lexists(dst):
                os.remove(dst)
                print(f"  ⏪ 원래 존재하지 않던 파일 제거 유지: {dst}")
            continue

        if os.path.lexists(dst):
            os.remove(dst)  # existed 제거

        if meta["is_symlink"]:
            os.symlink(meta["link_target"], dst)
            print(f"  ⏪ symlink 복원 {dst} → {meta['link_target']}")
        elif meta.get("backup_copy"):
            shutil.copy2(meta["backup_copy"], dst)
            print(f"  ⏪ {dst}")

    # ★최종 검증 — 불일치를 출력만 말고 누적 → die hard fail(복원 실패 기계 감지·HIGH codex).
    errors = []
    seal_ok, _seal_benign, seal_detail = seal_status(APP_BUNDLE)
    if seal_ok:
        print("  롤백 후 codesign --verify --deep --strict: PASS")
    elif not restored_from_backup:
        # 백업이 없어 번들 복원 자체를 못 한 갈래 — 봉인 실패를 정당화할 기준선이 없다. hard fail.
        errors.append(f"번들 codesign --verify --deep --strict FAIL(백업 부재로 복원 미수행): {seal_detail}")
    elif seal_status(bundle_bak)[0]:
        # 백업은 봉인이 유효했는데 복원본이 깨졌다 = 복원 과정의 결함. 종전대로 hard fail.
        errors.append(f"번들 codesign --verify --deep --strict FAIL(백업은 유효 = 복원 결함): {seal_detail}")
    else:
        # 백업 자체가 이미 그 상태였다 → 복원은 **충실했다**. 이걸 실패로 적으면 롤백은 영원히 실패한다
        # (기준은 '봉인이 유효한가'가 아니라 '백업 상태로 정확히 되돌렸는가'다 — 아래 sha256 대조가 그 판정).
        print(f"  ⚠ 롤백 후 봉인 미유효 — 백업 자체가 이미 같은 상태였다(충실 복원): {seal_detail}")
    # 번들 xattr 대조(정렬 비교 — 나열 순서 무관).
    curr_bundle_x = sorted(run(["xattr", APP_BUNDLE]).stdout.split())
    if curr_bundle_x != sorted(inv.get("bundle_xattrs", [])):
        errors.append(f"번들 xattr 불일치: now={curr_bundle_x} orig={sorted(inv.get('bundle_xattrs', []))}")
    for meta in inv["targets"]:
        dst = meta["dst"]
        if not os.path.lexists(dst):
            if meta["existed"]:
                errors.append(f"복원 누락(존재해야 함): {dst}")
            continue
        if meta["sha256"] and not os.path.islink(dst) and sha256(dst) != meta["sha256"]:
            errors.append(f"sha256 불일치: {dst}")
    if errors:
        die("★롤백 복원 검증 실패(hard fail):\n  - " + "\n  - ".join(errors))
    # 문구는 실제로 통과한 기준만 말한다 — 봉인이 유효하지 않은데 "서명 일치"라고 적으면 그건 거짓말이다.
    print("  ✓ 롤백 복원 검증 PASS({}·sha256·번들 xattr 일치)".format(
        "서명" if seal_ok else "서명=백업과 같은 상태"))

# ── 데몬 self-pid: system.identify RPC ─────────────────────────────────
def _socket_path():
    return os.environ.get("CYS_SOCKET") or os.path.expanduser("~/.local/state/cys/cys.sock")

def daemon_identify(timeout=2.0):
    path = _socket_path()
    if not os.path.exists(path):
        return None
    try:
        s = _socket.socket(_socket.AF_UNIX, _socket.SOCK_STREAM)
        s.settimeout(timeout)
        s.connect(path)
        s.sendall(b'{"id":1,"method":"system.identify","params":{}}\n')
        buf = b""
        while b"\n" not in buf:
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
        s.close()
        return json.loads(buf.decode().splitlines()[0]).get("result", {}).get("daemon_pid")
    except Exception:
        return None

# ── drain: 재시작 前 살아있는 노드에 저장 신호 (best-effort·자체 watchdog 12s) ──
def drain_nodes():
    cys_cli = f"{BREW}/cys"
    if not os.path.isfile(cys_cli):
        cys_cli = f"{APP_MACOS}/cys"
    r = run([cys_cli, "drain"])
    print(f"  ✓ drain(저장 신호·best-effort): rc={r.returncode} {r.stdout.strip()}")

# ── 데몬 재시작 (실패 시 restart hard fail) ─────────────────────────────
def restart_daemon(inv):
    res = {"ts": int(time.time())}
    before = daemon_identify()
    res["before_pid"] = before
    if before:
        run(["kill", "-TERM", str(before)])
    
    down = False
    for _ in range(50):
        if daemon_identify() is None:
            down = True
            break
        time.sleep(0.1)
    res["down_confirmed"] = down
    
    # 데몬 다운 실패 시 hard fail
    if before and not down:
        die(f"기존 데몬(PID {before})이 5초 내에 종료되지 않았습니다. (restart hard fail)", 1)

    ready = False
    after = None
    for _ in range(50):
        after = daemon_identify()
        if after is not None and after != before:
            ready = True
            break
        time.sleep(0.1)
    res["after_pid"] = after
    res["socket_ready"] = ready
    res["note"] = "launchd KeepAlive 적재면 자동 respawn; 아니면 다음 launch-agent/claim-role autostart"
    
    rp = os.path.join(inv["backup_dir"], "restart_result.json")
    with open(rp, "w") as f:
        json.dump(res, f, indent=2, ensure_ascii=False)
    
    # 데몬 기동 실패 시 hard fail
    if not ready:
        die(f"신규 데몬 기동 및 socket-ready 감지 실패 (restart hard fail). 결과={rp}", 1)
        
    print(f"  ✓ restart: down={down} ready={ready} before={before} after={after} → {rp}")

# ── crash recovery: 이전 실행이 staging swap 중간에 죽었으면 잔존물에서 복구 ──
def crash_recovery():
    """이전 실행이 남긴 잔해 정리·복구.

    ★교체 자체는 이제 `renamex_np(RENAME_SWAP)` 한 콜이라 **중간 상태가 없다**(전부 아니면 전무) —
      종전의 rename 2단 스왑이 만들던 '번들이 존재하지 않는 창'과 그 회복 로직이 통째로 사라졌다.
      남는 건 스크래치 청소뿐이다. 구 버전이 남겼을 수 있는 `.rollback-*` 잔해도 함께 거둔다
      (설치본이 없고 백업만 남은 상태에서는 복귀시켜야 하므로 그 분기는 유지).
    """
    old = "/Applications/cys.app.rollback-old"
    staging = "/Applications/cys.app.rollback-staging"
    if os.path.exists(old):
        if not os.path.exists(APP_BUNDLE):
            os.rename(old, APP_BUNDLE)  # 구 버전 2단 스왑 중단분 → 번들 복귀
            print(f"  ⚠ crash recovery: {old} → {APP_BUNDLE} 복귀(구 버전 중단 스왑 회복)")
        else:
            shutil.rmtree(old)
            print(f"  ⚠ crash recovery: 잔존 {old} 정리")
    if os.path.exists(staging):
        shutil.rmtree(staging)
        print(f"  ⚠ crash recovery: 잔존 {staging} 정리")
    # 새 계약의 스테이징 잔해(= 실패한 스크래치 또는 직전 성공 배포가 밀어낸 옛 번들).
    # ★번들 전체 ditto 백업이 backup_dir 에 따로 있으므로 여기서 지워도 가역성이 유지된다.
    for p in ab.sweep_stale_staging(APP_BUNDLE):
        print(f"  ⚠ crash recovery: 스테이징 잔해 정리 {p}")


def main():
    print("=== deploy_gate.py (정석복구 ④) ===")
    gate_concurrency_lock()
    crash_recovery()  # ★락 직후·게이트 前: 이전 미완 스왑 잔존물 복구

    # --restart 진입경로 처리
    if "--restart" in sys.argv:
        if not os.path.exists(BACKUP_BASE):
            die("백업 디렉토리가 존재하지 않습니다. 먼저 전체 배포를 실행하십시오.")
        dirs = sorted([d for d in os.listdir(BACKUP_BASE) if d.isdigit()])
        if not dirs:
            die("생성된 백업 정보가 없습니다.")
        latest_dir = os.path.join(BACKUP_BASE, dirs[-1])
        with open(os.path.join(latest_dir, "inventory.json")) as f:
            inv = json.load(f)
        restart_daemon(inv)
        print("✅ 데몬 재시작 및 socket-ready 검증 성공.")
        return

    gate_src_path()
    gate_no_icloud()
    gate_same_generation()
    inv = backup_current_state()
    try:
        # ★ATOMIC-1: 부분 쓰기는 스테이징 사본 안에서만, 설치본은 단일 스왑으로만 바뀐다.
        #   ①②는 실패해도 설치본 무접촉이고, ③④⑤는 실패하면 스스로 원복한다.
        staged = stage_new_bundle()
        swap_bundle_into_place(staged)
        replace_external_targets()
        smoke()
        # ★최종 구조 완본성 — 봉인 갈래 판정에 앞서 **먼저** 못 박는다. 아래에서 봉인 이탈 한 갈래를
        #   양성으로 인정하므로, 그 관용이 구조 결손(=2026-08-01 사고의 형태)을 가리지 못하게
        #   구조는 여기서 무조건 hard fail 로 확인한다(종전에 없던 검사 — 순증).
        residual = ab.verify_bundle(APP_BUNDLE, codesign=False)
        if residual:
            raise RuntimeError("최종 구조 완본성 검증 실패:\n  - " + "\n  - ".join(residual))
        # ★최종 봉인 검증 — 갈래를 구별한다(HIGH-2 의 두 번째 자충 지점).
        #   여기까지 왔다는 건 스왑이 끝나고 스모크까지 통과했다는 뜻이다. 이 시점의 설치본은
        #   **살아 있는 공유 객체**라, 구 데몬이 `.pyc` 를 하나 쓰기만 해도 rc≠0 이 된다. 종전 코드는
        #   그걸 RuntimeError 로 올려 **성공한 배포를 통째로 롤백**했다(원인은 하나도 고치지 못한 채
        #   옛 세대로 되돌리는, 순수 손해인 처방). 이제 변조·소실은 종전대로 hard fail 이고,
        #   **바이트코드 캐시 추가만**인 경우에는 배포를 유지하고 큰 소리로 알린다(무증상 통과 아님).
        seal_ok, seal_benign, seal_detail = seal_status(APP_BUNDLE)
        if not seal_ok and not seal_benign:
            raise RuntimeError(f"최종 봉인 검증 실패(변조·소실 갈래): {seal_detail}")
        if not seal_ok:
            print(f"  ⚠ 최종 봉인 이탈이 **바이트코드 캐시 추가뿐** — 배포는 유지한다: {seal_detail}")
            print("     원인: 구 데몬(SEAL-1 이전)이 절대경로로 동봉 python 을 부름. 새 cysd 는 "
                  "PYTHONDONTWRITEBYTECODE 로 더 쓰지 않는다. 봉인 회복은 다음 배포의 스테이징 "
                  "재서명이 수행하며, 현 상태 확인은 `cys doctor app-seal`.")
        print(f"✅ 게이트 배포 완료 — 백업={inv['backup_dir']}")
    except Exception as e:
        print(f"❌ 배포 실패: {e}")
        rollback(inv)
        sys.exit(1)
    # ★스텝 10(2026-07-06 오너 지시): 배포 성공 확정 후 구 데몬 자동 교체 — drain(저장 신호)
    # → 구 데몬 SIGTERM → 신 데몬 socket-ready 폴링. 파일 교체는 이미 완료·검증됐으므로
    # 재시작 실패는 롤백하지 않고 hard fail로 알린다(restart_daemon 내 die).
    # 구 데몬이 아예 없으면 교체할 대상이 없다 — 건너뛴다(다음 기동이 새 바이너리·기존 성공 배포 불변).
    if daemon_identify() is None:
        print("  ✓ 실행 중인 데몬 없음 — 재시작 생략(다음 기동이 새 cysd)")
        return
    drain_nodes()
    restart_daemon(inv)
    print("✅ 데몬 교체 완료 — 구 데몬 종료·신 데몬 socket-ready 확인")

if __name__ == "__main__":
    # ★안전장치: --execute(배포) 또는 --restart(데몬 재시작) 없이는 실행 거부.
    if "--execute" in sys.argv or "--restart" in sys.argv:
        main()
    else:
        print(__doc__)
        print("배포(오너 승인 후): python3 scripts/deploy_gate.py --execute")
        print("데몬 재시작(오너 승인 후): python3 scripts/deploy_gate.py --restart")
        sys.exit(2)
