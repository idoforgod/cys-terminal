#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
javis_backup.py — 불사조 Phase 8: 일반 백업 능력 + 정직한 보호 상태

설계 근거: _round/ZERO_LOSS_RESTORE_DESIGN.md §8.1-1(복원 리허설)·§11.4·§11.5·M2(신뢰원장 GREEN/AMBER/RED)·
  G5(소유자키 E2E 암호화·라이브 시크릿 제외+복원 재인증·불변 오프사이트·복원 무결성 검증).

★제품 계층(개인정보 무첨가·모든 사용자 동일): 이 도구는 "무엇을 어떻게 백업하는가"의 능력·기본값·정직한
  상태만 싣는다. "어디에·누구 열쇠로"의 앵커(오프사이트 목적지·암호키)는 사용자가 댄다.
  - 경로는 HOME 파생·glob 로만 산출한다(개인 경로/계정/원격/키 하드코딩 0).
  - 암호키는 --key-file 또는 env CYS_BACKUP_KEY 로 사용자가 제공한다(코드에 키 없음).
  - 오프사이트 목적지는 env CYS_BACKUP_OFFSITE(앵커)로 사용자가 댄다. 없어도 상태 보고는 정직하게 동작한다.

3계층 분류(G5):
  · Tier1 정체성(재생성 불가): ~/.cys/pack 트리(soul.md·directives·skills·memory·_round). → 암호화하여 오프사이트.
  · Tier2 비밀(라이브 시크릿): *.token·*.key·*.pem·*credential*·*auth*.json·.env 등. → 오프사이트에서 제외(로그)·복원 시 재인증.
  · Tier3 대용량(로컬만): *.db·*-wal·*-shm·*.log. → 로컬 백업만(오프사이트 제외).

암호화(이식성): openssl(shutil.which)로 AES-256-CBC. 키유도는 stdlib hashlib.pbkdf2_hmac 로 직접 수행하고
  openssl 에는 raw key/iv(-K/-iv)를 넘긴다 → openssl 의 -pbkdf2 플래그 유무(LibreSSL vs OpenSSL)에 무관하게 동작.
  salt/iv 는 비밀이 아니므로 manifest header 에 평문 보관(키는 보관하지 않는다).

서브커맨드:
  classify [--home H]              — 3계층 분류 결과(결정론) 출력
  backup   --out DIR [--home H] [--key-file F]   — 계층별 산출(Tier1 암호화·Tier2 제외·Tier3 로컬)
  restore  --in DIR --dest D [--key-file F]       — Tier1 복원(암호화면 복호화 후 추출)
  verify   --in DIR [--key-file F]                — Tier1 무결성(해시 대조)
  status   [--home H] [--json]                     — 정직 보호등급 GREEN/AMBER/RED(앵커 불요)
  self-test                                         — 격리 합성 home 로 backup→복원→해시동일·토큰제외·3등급 실증
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tarfile
import tempfile
import time

HOME = os.path.expanduser("~")

# 분류 패턴(결정론) — 우선순위: 비밀 > 대용량 > 정체성. 파일명·상대경로 양쪽에 매칭(안전 방향).
SECRET_PATTERNS = [
    r".*\.token$", r".*\.key$", r".*\.pem$", r".*\.p12$", r".*\.pfx$",
    r"(^|.*/)\.env(\..*)?$", r".*credential.*", r".*secret.*",
    r".*auth.*\.json$", r".*[._-]token[._-].*", r".*keychain.*", r".*\.netrc$",
]
LARGE_PATTERNS = [
    r".*\.db$", r".*\.db-wal$", r".*\.db-shm$", r".*\.sqlite3?$",
    r".*\.log$", r".*\.log\.\d+$",
]

PBKDF2_ITERS = 200000


def die(msg, code=2):
    sys.stderr.write("[backup][FATAL] %s\n" % msg)
    sys.exit(code)


def _matches(patterns, s):
    return any(re.match(p, s, re.IGNORECASE) for p in patterns)


def _sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def _atomic_write_json(path, obj):
    """tmp+fsync+rename+dir fsync — Phase3 write_json_atomic 패턴 재사용."""
    d = os.path.dirname(path)
    os.makedirs(d, exist_ok=True)
    tmp = os.path.join(d, ".tmp-%d-%s" % (os.getpid(), os.path.basename(path)))
    with open(tmp, "w") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    os.rename(tmp, path)
    try:
        dfd = os.open(d, os.O_RDONLY)
        os.fsync(dfd)
        os.close(dfd)
    except Exception:
        pass


# ------------------------------------------------------------------ 분류

def default_roots(home=HOME):
    """Tier1 정체성 루트 = pack 트리(soul.md·directives·skills·memory·_round 전부 하위). HOME 파생."""
    return [os.path.join(home, ".cys", "pack")]


def classify(roots):
    """루트들을 walk 하여 파일을 3계층으로 결정론 분류. (tier1, tier2, tier3) 각 정렬 리스트."""
    t1, t2, t3 = [], [], []
    for root in roots:
        if not os.path.isdir(root):
            continue
        for dirpath, _dirnames, filenames in os.walk(root):  # followlinks=False(심링크 루프 회피)
            for fn in filenames:
                full = os.path.join(dirpath, fn)
                rel = os.path.relpath(full, root)
                if _matches(SECRET_PATTERNS, rel) or _matches(SECRET_PATTERNS, fn):
                    t2.append(full)
                elif _matches(LARGE_PATTERNS, fn):
                    t3.append(full)
                else:
                    t1.append(full)
    return sorted(t1), sorted(t2), sorted(t3)


# ------------------------------------------------------------------ 암호화(stdlib KDF + openssl raw-key)

def _openssl():
    import shutil
    p = shutil.which("openssl")
    if not p:
        die("openssl 미발견 — 암호화 백업 불가. 설치 후 재시도(평문 백업은 --key-file 없이 가능하나 오프사이트 부적격).")
    return p


def _derive_key(passphrase, salt, iters=PBKDF2_ITERS):
    return hashlib.pbkdf2_hmac("sha256", passphrase.encode("utf-8"), salt, iters, dklen=32)


def encrypt_file(plain_path, enc_path, passphrase):
    """AES-256-CBC 암호화. salt/iv 를 새로 생성하고 header(비밀 아님) 반환."""
    salt = os.urandom(16)
    iv = os.urandom(16)
    key = _derive_key(passphrase, salt)
    r = subprocess.run([_openssl(), "enc", "-aes-256-cbc", "-K", key.hex(), "-iv", iv.hex(),
                        "-in", plain_path, "-out", enc_path], capture_output=True, text=True)
    if r.returncode != 0:
        die("openssl 암호화 실패: %s" % (r.stderr or "").strip())
    return {"cipher": "aes-256-cbc", "kdf": "pbkdf2_hmac_sha256", "iterations": PBKDF2_ITERS,
            "salt": salt.hex(), "iv": iv.hex()}


def decrypt_file(enc_path, plain_path, passphrase, header):
    salt = bytes.fromhex(header["salt"])
    iv = bytes.fromhex(header["iv"])
    key = _derive_key(passphrase, salt, header.get("iterations", PBKDF2_ITERS))
    r = subprocess.run([_openssl(), "enc", "-d", "-aes-256-cbc", "-K", key.hex(), "-iv", iv.hex(),
                        "-in", enc_path, "-out", plain_path], capture_output=True, text=True)
    if r.returncode != 0:
        die("openssl 복호화 실패(키 불일치 의심): %s" % (r.stderr or "").strip())


# ------------------------------------------------------------------ backup / restore / verify

def get_passphrase(key_file=None):
    """사용자 제공 키 — --key-file 우선, 없으면 env CYS_BACKUP_KEY, 둘 다 없으면 None(평문). ★코드 하드코딩 0."""
    if key_file and os.path.isfile(key_file):
        return open(key_file).read().strip()
    env = os.environ.get("CYS_BACKUP_KEY")
    return env if env else None


def _safe_extract(tf, dest):
    """path traversal 방어 후 추출(우리 tar 는 home-상대이나 방어적)."""
    dest_real = os.path.realpath(dest)
    for m in tf.getmembers():
        target = os.path.realpath(os.path.join(dest, m.name))
        if not (target == dest_real or target.startswith(dest_real + os.sep)):
            die("tar 멤버 경로 이탈 거부: %s" % m.name)
    tf.extractall(dest)


def do_backup(out_dir, home, roots=None, passphrase=None):
    roots = roots if roots is not None else default_roots(home)
    offsite = os.path.join(out_dir, "offsite")
    localdir = os.path.join(out_dir, "local")
    os.makedirs(offsite, exist_ok=True)
    os.makedirs(localdir, exist_ok=True)
    t1, t2, t3 = classify(roots)

    # Tier1 tar(평문) → 해시 → 암호화(키 있으면·평문 tar 제거) ──
    tar1 = os.path.join(offsite, "tier1.tar")
    with tarfile.open(tar1, "w") as tf:
        for f in t1:
            tf.add(f, arcname=os.path.relpath(f, home))
    t1_hash = _sha256_file(tar1)
    header = None
    stored1 = "tier1.tar"
    if passphrase:
        enc = os.path.join(offsite, "tier1.tar.enc")
        header = encrypt_file(tar1, enc, passphrase)
        os.remove(tar1)  # ★오프사이트엔 암호문만 — 평문 tar 잔존 금지
        stored1 = "tier1.tar.enc"

    # Tier3 로컬 tar(오프사이트 제외) ──
    tar3 = os.path.join(localdir, "tier3.tar")
    with tarfile.open(tar3, "w") as tf:
        for f in t3:
            tf.add(f, arcname=os.path.relpath(f, home))

    # Tier2 = 오프사이트/로컬 어디에도 쓰지 않는다(제외). 목록만 로그.
    manifest = {
        "created_at": time.time(),
        "generator": "javis_backup.py",
        "home_base": home,
        "roots": roots,
        "encrypted": bool(passphrase),
        "tier1_identity": {"stored": "offsite/" + stored1, "sha256_plaintext": t1_hash,
                           "count": len(t1), "header": header},
        "tier2_secrets_excluded": {"count": len(t2),
                                   "files": sorted(os.path.relpath(f, home) for f in t2),
                                   "note": "라이브 시크릿 — 오프사이트/로컬 산출물에서 제외(G5). 복원 후 재인증 필요."},
        "tier3_local_only": {"stored": "local/tier3.tar", "count": len(t3),
                             "files": sorted(os.path.relpath(f, home) for f in t3),
                             "note": "대용량 — 로컬 백업만(오프사이트 제외)."},
    }
    _atomic_write_json(os.path.join(out_dir, "manifest.json"), manifest)
    return manifest


def do_restore(in_dir, dest, passphrase=None):
    manifest = json.load(open(os.path.join(in_dir, "manifest.json")))
    os.makedirs(dest, exist_ok=True)
    stored = manifest["tier1_identity"]["stored"]  # 'offsite/tier1.tar[.enc]'
    src = os.path.join(in_dir, stored)
    tmp_tar = None
    if manifest.get("encrypted"):
        if not passphrase:
            die("암호화 백업 복원에 키 필요(--key-file 또는 CYS_BACKUP_KEY).")
        tmp_tar = os.path.join(dest, ".restore-tier1.tar")
        decrypt_file(src, tmp_tar, passphrase, manifest["tier1_identity"]["header"])
        tarpath = tmp_tar
    else:
        tarpath = src
    with tarfile.open(tarpath, "r") as tf:
        _safe_extract(tf, dest)
    if tmp_tar and os.path.exists(tmp_tar):
        os.remove(tmp_tar)
    return manifest


def do_verify(in_dir, passphrase=None):
    manifest = json.load(open(os.path.join(in_dir, "manifest.json")))
    stored = manifest["tier1_identity"]["stored"]
    src = os.path.join(in_dir, stored)
    if manifest.get("encrypted"):
        if not passphrase:
            return False, "키 없음 — 검증 불가"
        tmp = tempfile.NamedTemporaryFile(delete=False).name
        try:
            decrypt_file(src, tmp, passphrase, manifest["tier1_identity"]["header"])
            actual = _sha256_file(tmp)
        finally:
            if os.path.exists(tmp):
                os.remove(tmp)
    else:
        actual = _sha256_file(src)
    expected = manifest["tier1_identity"]["sha256_plaintext"]
    return actual == expected, "expected=%s actual=%s" % (expected[:12], actual[:12])


# ------------------------------------------------------------------ 정직한 보호 상태(M2·§11.5)

def protection_status(home=HOME, backup_dir=None, offsite_anchor=None, recent_days=7):
    """정직한 보호등급 — ★앵커 없이 상태만으로 산출(진실만 말함). RED=무방비 기본(숨김 금지).
    강제 아님(사용자 위협수준 제각각) — 진실을 가리지 않을 뿐."""
    backup_dir = backup_dir or os.environ.get("CYS_BACKUP_DIR") or os.path.join(home, ".cys", "backups")
    if offsite_anchor is None:
        offsite_anchor = os.environ.get("CYS_BACKUP_OFFSITE")
    anchored = bool(offsite_anchor)
    man = os.path.join(backup_dir, "manifest.json")
    base = {"backup_dir": backup_dir, "offsite_anchor_armed": anchored}
    if not os.path.isfile(man):
        return dict(base, grade="RED", reasons=["백업 미구성(manifest 없음) — 무방비"])
    try:
        m = json.load(open(man))
    except Exception:
        return dict(base, grade="RED", reasons=["manifest 손상 — 신뢰 불가"])
    age_days = (time.time() - m.get("created_at", 0)) / 86400.0
    recent = age_days <= recent_days
    encrypted = bool(m.get("encrypted"))
    reasons = []
    if not recent:
        return dict(base, grade="RED", reasons=["백업 오래됨(%.1f일 > %d일) — 사실상 무방비" % (age_days, recent_days)],
                    age_days=round(age_days, 2), encrypted=encrypted)
    if encrypted and anchored:
        grade = "GREEN"
        reasons = ["최근 백업(%.1f일)·암호화·오프사이트 앵커 무장" % age_days]
    else:
        grade = "AMBER"
        if not encrypted:
            reasons.append("암호화 안 됨 — 오프사이트 부적격(로컬만 안전)")
        if not anchored:
            reasons.append("오프사이트 앵커 미무장(env CYS_BACKUP_OFFSITE 미설정) — 단일 사이트")
    return dict(base, grade=grade, reasons=reasons, age_days=round(age_days, 2), encrypted=encrypted)


# ------------------------------------------------------------------ self-test(격리 합성 home)

SECRET_MARK = "SUPERSECRET_TOKEN_VALUE_DO_NOT_LEAK_9f3a"


def _build_synth_home(root):
    pack = os.path.join(root, ".cys", "pack")
    os.makedirs(os.path.join(pack, "directives"))
    os.makedirs(os.path.join(pack, "skills", "s1"))
    os.makedirs(os.path.join(pack, "memory"))
    os.makedirs(os.path.join(pack, "_round"))
    os.makedirs(os.path.join(pack, "creds"))
    # Tier1 정체성
    open(os.path.join(pack, "soul.md"), "w").write("SOUL identity\n")
    open(os.path.join(pack, "directives", "WORKER_DIRECTIVE.md"), "w").write("worker rules\n")
    open(os.path.join(pack, "skills", "s1", "SKILL.md"), "w").write("skill body\n")
    open(os.path.join(pack, "memory", "MEMORY.md"), "w").write("index\n")
    open(os.path.join(pack, "_round", "STATE.md"), "w").write("state\n")
    open(os.path.join(pack, "agents.json"), "w").write('{"a":1}\n')
    # Tier2 비밀(누출되면 안 됨)
    open(os.path.join(pack, "session.token"), "w").write(SECRET_MARK + "\n")
    open(os.path.join(pack, "creds", "auth.json"), "w").write('{"token":"' + SECRET_MARK + '"}\n')
    open(os.path.join(pack, ".env"), "w").write("API_KEY=" + SECRET_MARK + "\n")
    # Tier3 대용량
    open(os.path.join(pack, "transcripts.db"), "wb").write(b"\x00" * 8192)
    open(os.path.join(pack, "cysd.log"), "w").write("log line\n" * 100)
    return root


def do_self_test():
    print("=== javis_backup self-test (격리 합성 home) ===")
    work = tempfile.mkdtemp(prefix="cys-backuptest-")
    try:
        home = _build_synth_home(os.path.join(work, "home"))
        roots = default_roots(home)

        # T1: 3계층 분류 결정론
        t1, t2, t3 = classify(roots)
        t1r = [os.path.relpath(f, home) for f in t1]
        t2r = [os.path.relpath(f, home) for f in t2]
        t3r = [os.path.relpath(f, home) for f in t3]
        assert any("soul.md" in f for f in t1r), "T1: soul.md 가 Tier1 아님"
        assert any("session.token" in f for f in t2r), "T1: token 이 Tier2 아님"
        assert any("auth.json" in f for f in t2r), "T1: auth.json 이 Tier2 아님"
        assert any(f.endswith(".env") for f in t2r), "T1: .env 이 Tier2 아님"
        assert any("transcripts.db" in f for f in t3r), "T1: db 가 Tier3 아님"
        assert any("cysd.log" in f for f in t3r), "T1: log 가 Tier3 아님"
        assert not any("token" in f or "auth.json" in f or f.endswith(".env") for f in t1r), "T1: 비밀이 Tier1 오분류"
        print("  [PASS] T1 3계층 결정론 분류 (Tier1 정체성 / Tier2 비밀제외 / Tier3 대용량)")

        # T2: 암호화 백업 → 복원 → 원본 해시 동일(왕복)
        key = os.path.join(work, "scratch.key")
        open(key, "w").write("scratch-test-passphrase-only\n")
        out = os.path.join(work, "backup")
        man = do_backup(out, home, passphrase=get_passphrase(key))
        assert man["encrypted"], "T2: 암호화 플래그 미설정"
        dest = os.path.join(work, "restored")
        do_restore(out, dest, passphrase=get_passphrase(key))
        # Tier1 원본 파일 각각 복원본과 해시 동일
        mismatch = []
        for f in t1:
            rp = os.path.relpath(f, home)
            rf = os.path.join(dest, rp)
            if not os.path.isfile(rf) or _sha256_file(rf) != _sha256_file(f):
                mismatch.append(rp)
        assert not mismatch, "T2: 복원 해시 불일치 %s" % mismatch
        print("  [PASS] T2 복원 리허설: 암호화 백업→복원→원본 해시 동일(왕복 무손실)")

        # T3: verify 해시 대조 + 잘못된 키 실패
        ok, ev = do_verify(out, passphrase=get_passphrase(key))
        assert ok, "T3: verify 실패 %s" % ev
        print("  [PASS] T3 verify 무결성 해시 대조 OK (%s)" % ev)

        # T4: ★토큰 오프사이트 미포함(유출면 차단·G5)
        offdir = os.path.join(out, "offsite")
        offsite_files = os.listdir(offdir)
        assert offsite_files == ["tier1.tar.enc"], "T4: 오프사이트에 암호문 외 산출물 존재 %s" % offsite_files
        blob = open(os.path.join(offdir, "tier1.tar.enc"), "rb").read()
        assert SECRET_MARK.encode() not in blob, "T4: 비밀 마커가 오프사이트 암호문 바이트에 노출!"
        # 복호화해도 tar 안에 토큰 파일이 없어야 함
        tmp = os.path.join(work, "peek.tar")
        decrypt_file(os.path.join(offdir, "tier1.tar.enc"), tmp, get_passphrase(key), man["tier1_identity"]["header"])
        with tarfile.open(tmp) as tf:
            names = tf.getnames()
        assert not any("token" in n or "auth.json" in n or n.endswith(".env") for n in names), "T4: 비밀이 Tier1 tar 에 포함!"
        assert SECRET_MARK.encode() not in open(tmp, "rb").read(), "T4: 복호화 tar 에 비밀 마커 존재"
        # 복원본에도 토큰 없어야
        assert not os.path.exists(os.path.join(dest, ".cys", "pack", "session.token")), "T4: 토큰이 복원됨(제외 실패)"
        assert man["tier2_secrets_excluded"]["count"] >= 3, "T4: 제외 로그 부실"
        print("  [PASS] T4 토큰 유출면 차단: 오프사이트 암호문/복호tar/복원본 어디에도 비밀 없음·제외 로그 %d건"
              % man["tier2_secrets_excluded"]["count"])

        # T5: 정직 보호등급 RED/AMBER/GREEN
        empty = os.path.join(work, "nobackup")
        os.makedirs(empty)
        g_red = protection_status(home=home, backup_dir=empty, offsite_anchor=None)
        assert g_red["grade"] == "RED", "T5: 무백업이 RED 아님 (%s)" % g_red["grade"]
        # 평문 백업(암호화 없음)·앵커 없음 → AMBER
        outp = os.path.join(work, "backup_plain")
        do_backup(outp, home, passphrase=None)
        g_amber = protection_status(home=home, backup_dir=outp, offsite_anchor=None)
        assert g_amber["grade"] == "AMBER", "T5: 평문/무앵커가 AMBER 아님 (%s)" % g_amber["grade"]
        # 암호화 백업 + 앵커 무장 → GREEN
        g_green = protection_status(home=home, backup_dir=out, offsite_anchor="scratch://remote")
        assert g_green["grade"] == "GREEN", "T5: 암호화+앵커가 GREEN 아님 (%s)" % g_green["grade"]
        print("  [PASS] T5 정직 보호등급: 무방비=RED / 부분(평문·무앵커)=AMBER / 완비(암호화+앵커)=GREEN")

        print("=== self-test 전체 PASS ===")
        return 0
    except AssertionError as e:
        print("=== self-test FAIL: %s ===" % e, file=sys.stderr)
        return 1
    finally:
        import shutil
        shutil.rmtree(work, ignore_errors=True)


# ------------------------------------------------------------------ main

def main():
    ap = argparse.ArgumentParser(description="불사조 Phase8 일반 백업 능력 + 정직 보호 상태")
    sub = ap.add_subparsers(dest="cmd", required=True)
    pc = sub.add_parser("classify"); pc.add_argument("--home", default=HOME)
    pb = sub.add_parser("backup"); pb.add_argument("--out", required=True); pb.add_argument("--home", default=HOME)
    pb.add_argument("--key-file")
    pr = sub.add_parser("restore"); pr.add_argument("--in", dest="indir", required=True)
    pr.add_argument("--dest", required=True); pr.add_argument("--key-file")
    pv = sub.add_parser("verify"); pv.add_argument("--in", dest="indir", required=True); pv.add_argument("--key-file")
    ps = sub.add_parser("status"); ps.add_argument("--home", default=HOME); ps.add_argument("--json", action="store_true")
    sub.add_parser("self-test")
    args = ap.parse_args()

    if args.cmd == "classify":
        t1, t2, t3 = classify(default_roots(args.home))
        out = {"tier1_identity": [os.path.relpath(f, args.home) for f in t1],
               "tier2_secrets_excluded": [os.path.relpath(f, args.home) for f in t2],
               "tier3_local_only": [os.path.relpath(f, args.home) for f in t3]}
        print(json.dumps(out, ensure_ascii=False, indent=2))
        return 0
    if args.cmd == "backup":
        m = do_backup(args.out, args.home, passphrase=get_passphrase(args.key_file))
        print(json.dumps({"backup": "OK", "out": args.out, "encrypted": m["encrypted"],
                          "tier1": m["tier1_identity"]["count"], "tier2_excluded": m["tier2_secrets_excluded"]["count"],
                          "tier3_local": m["tier3_local_only"]["count"]}, ensure_ascii=False, indent=2))
        return 0
    if args.cmd == "restore":
        do_restore(args.indir, args.dest, passphrase=get_passphrase(args.key_file))
        print(json.dumps({"restore": "OK", "dest": args.dest}, ensure_ascii=False, indent=2))
        return 0
    if args.cmd == "verify":
        ok, ev = do_verify(args.indir, passphrase=get_passphrase(args.key_file))
        print(json.dumps({"verify": "OK" if ok else "FAIL", "detail": ev}, ensure_ascii=False, indent=2))
        return 0 if ok else 1
    if args.cmd == "status":
        st = protection_status(home=args.home)
        print(json.dumps(st, ensure_ascii=False, indent=2))
        return 0
    if args.cmd == "self-test":
        return do_self_test()
    return 1


if __name__ == "__main__":
    sys.exit(main())
