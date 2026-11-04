#!/usr/bin/env python3
# javis_state_snapshot.py — 무손실 복원 Phase 0: 상태 파일 세대(generation) 보관기
#
# 설계 근거: _round/ZERO_LOSS_RESTORE_DESIGN.md §8.1-4
#   "Append-only 세대 보관 + 원본 불파괴 불변식 — 복원 시스템은 상태의 읽기 전용 소비자.
#    상태 파일은 덮어쓰지 않고 세대 보관(atomic rename + GC 정책)."
#
# 이 도구가 하는 일 / 안 하는 일:
#   - 한다: ~/.local/state/cys/topology.json 등 소형 L1 선언 상태 파일을 읽어
#           ~/.cys/state-generations/<timestamp>/ 에 원자적으로 세대 보관한다.
#   - 절대 안 한다: 원본 상태 파일을 수정/삭제하지 않는다(읽기 전용 소비자).
#                   state-generations/ 밖에는 아무것도 쓰지 않는다.
#
# 원자성 보장(성공기준 ①):
#   각 세대는 state-generations/.tmp-<pid>-<ts>/ 에 먼저 완성(파일 복사+fsync+manifest)한 뒤
#   os.rename 으로 최종 <timestamp>/ 로 '디렉터리 원자 승격'한다. 승격 이전에 프로세스가
#   죽으면 최종 세대는 존재하지 않고 .tmp-* 잔재만 남으며(다음 실행이 청소), 소비자는
#   반파 세대를 절대 보지 않는다. --self-test 가 SIGKILL 중단으로 이를 실증한다.
#
# GC 정책: 최근 48세대 + 일별 대표 14일 보관, 그 외 삭제(state-generations/ 내부만).
#
# 사용:
#   javis_state_snapshot.py snapshot [--dry-run]   # 세대 1건 생성 + GC
#   javis_state_snapshot.py list                   # 세대 목록
#   javis_state_snapshot.py verify [--gen <이름>]  # manifest 해시 무결성 검증
#   javis_state_snapshot.py gc [--dry-run]         # 보관정책 적용(삭제 대상 산출/실행)
#   javis_state_snapshot.py self-test              # 원자성/중단내성 실증(격리 임시폴더)

import argparse
import glob as _glob
import hashlib
import json
import os
import re
import shutil
import sys
import tempfile
import time
from datetime import datetime, timedelta, timezone

HOME = os.path.expanduser("~")

# 세대 보관 루트 (이 스크립트가 쓰기를 허용받은 유일한 출력 경로)
GEN_ROOT = os.path.join(HOME, ".cys", "state-generations")

# 세대 보관 대상 = 소형 L1 선언 상태(토폴로지/스케줄/자율주행/부서). 대형 DB(transcripts.db,
# analytics.db)·log 는 세대 보관이 아니라 3-2-1 백업의 몫이라 여기서 제외한다.
#
# ★Phase 7(자동 보호 상속): 소스는 정적 목록이 아니라 default_sources() 로 동적 산출한다 —
#   메인 데몬 선언상태 + 루트 레지스트리(depts.json) + ★발견되는 모든 부서의 선언상태를 자동 포함.
#   부서는 손 배선·부서명 하드코딩 없이 glob(cys-dept-*) ∪ depts.json 레지스트리로 발견된다
#   (모든 사용자 동일 적용 — 개인 경로/계정 무첨가, HOME 파생 상대경로만).
DECLARATIVE_BASENAMES = ["topology.json", "schedule_state.json", "autopilot.json", "event.seq"]

IS_WINDOWS = os.name == "nt"

# ── Windows named pipe → 상태 디렉터리 매핑 (Rust src/bin/cysd/state.rs::pipe_slug/state_dir 규칙의 단일 소스) ──
# ★javis_phoenix.py 도 이 두 함수를 재사용한다(중복 구현 금지) — 규칙은 여기 한 곳에만 둔다.
#   unix 는 소켓 부모가 곧 상태 dir 이지만 Windows 는 파이프에 파일시스템 부모가 없어 슬러그로 파생해야 한다:
#   기본 데몬(`\\.\pipe\cys`)=%LOCALAPPDATA%\cys · 부서(`\\.\pipe\cys-dept-<n>`)=그 하위 슬러그 디렉터리.

def _win_pipe_slug(socket):
    """named pipe 경로에서 슬러그 추출 — 역/슬래시 마지막 컴포넌트의 안전문자(영숫자·-·_)만(state.rs::pipe_slug)."""
    s = str(socket)
    last = re.split(r"[\\/]", s)[-1] if s else ""
    return "".join(c for c in last if c.isalnum() or c in "-_")


def _win_state_dir_for_socket(socket, localappdata=None, home=HOME):
    """Windows 소켓(named pipe)→상태 디렉터리(state.rs::state_dir). localappdata 주입 가능(테스트·주입용)."""
    base = localappdata or os.environ.get("LOCALAPPDATA") or os.path.join(home, "AppData", "Local")
    root = os.path.join(base, "cys")
    slug = _win_pipe_slug(socket)
    if not slug or slug == "cys":
        return os.path.realpath(root)
    return os.path.realpath(os.path.join(root, slug))


def _dept_state_dirs(state_root, depts_json=None, windows=None, localappdata=None):
    """부서 상태 디렉터리를 동적 발견한다 — 파일시스템 glob(cys-dept-*) ∪ depts.json 소켓 매핑.
    ★부서명 하드코딩 없음. registry(depts.json)가 stale(미등록 부서 존재)여도 파일시스템 truth 로 커버한다
    (Phase7 실측: depts.json=dept-1 만인데 디스크엔 dept-1~5 존재 → glob 이 전부 잡는다).
    ★Windows(권고#1): 부서 디렉터리는 %LOCALAPPDATA%\\cys 하위(cys-dept-*)이고 registry 소켓은 named pipe 라
    부모 dirname 이 아니라 _win_state_dir_for_socket 슬러그 매핑으로 상태 dir 을 파생한다(unix=소켓 부모 그대로)."""
    win = IS_WINDOWS if windows is None else windows
    dirs = set()
    try:
        for name in os.listdir(state_root):
            if name.startswith("cys-dept-"):
                p = os.path.join(state_root, name)
                if os.path.isdir(p):
                    dirs.add(os.path.realpath(p))
    except OSError:
        pass
    dj = depts_json or os.path.join(os.path.dirname(os.path.dirname(state_root)), ".cys", "depts.json")
    if os.path.isfile(dj):
        try:
            reg = json.load(open(dj))
            for _name, meta in (reg.get("depts") or {}).items():
                sock = (meta or {}).get("socket")
                if sock:
                    if win:
                        dirs.add(os.path.realpath(_win_state_dir_for_socket(sock, localappdata=localappdata)))
                    else:
                        dirs.add(os.path.realpath(os.path.dirname(sock)))
        except Exception:
            pass
    return sorted(dirs)


def default_sources(home=HOME, state_root=None, depts_json=None, windows=None, localappdata=None):
    """세대 보관 소스를 동적 산출한다(정적 목록 대체). 메인 선언상태 + 루트 레지스트리 +
    ★발견되는 모든 부서의 선언상태를 자동 포함 = '태어날 때부터 보호'(손 배선 0).
    ★Windows(권고#1): 메인 상태=%LOCALAPPDATA%\\cys, 부서=그 하위 cys-dept-*(state.rs 레이아웃). unix 는 종전대로
    <state_root>/cys + <state_root>/cys-dept-*. windows/localappdata 주입으로 mac 에서도 win 분기 단위검증 가능."""
    win = IS_WINDOWS if windows is None else windows
    if win:
        base = localappdata or os.environ.get("LOCALAPPDATA") or os.path.join(home, "AppData", "Local")
        main_state = os.path.join(base, "cys")
        dept_scan_root = main_state            # win: 부서는 cys 하위(state.rs 슬러그 레이아웃)
    else:
        state_root = state_root or os.path.join(home, ".local", "state")
        main_state = os.path.join(state_root, "cys")
        dept_scan_root = state_root             # unix: 부서는 state_root 형제(cys-dept-*)
    srcs = [os.path.join(main_state, b) for b in DECLARATIVE_BASENAMES]
    depts_json = depts_json or os.path.join(home, ".cys", "depts.json")
    srcs.append(depts_json)
    for dd in _dept_state_dirs(dept_scan_root, depts_json=depts_json, windows=win, localappdata=localappdata):
        srcs.extend(os.path.join(dd, b) for b in DECLARATIVE_BASENAMES)
    # ★G1(d)(cokacdir 성찰 2026-07-04): 복원 SOT·노드 TODO·장기기억도 세대 보관에 포함 —
    #   SESSION_STATE.md 는 재부팅 복원의 단일 진실인데 세대 보관 대상에서 빠져 있었다.
    #   개인경로 하드코딩 금지(pack scan gate): 프로젝트는 $JAVIS_ROOT(env 또는 CWD),
    #   장기기억은 HOME glob 파생만(javis_wakeup.py 등과 동일 관례).
    proj_round = os.path.join(os.environ.get("JAVIS_ROOT") or os.getcwd(), "_round")
    srcs.append(os.path.join(proj_round, "SESSION_STATE.md"))
    srcs.extend(sorted(_glob.glob(os.path.join(proj_round, "*_TODO.md"))))
    #   ★복원 무결성: 위 $JAVIS_ROOT/CWD 기반 '_round' 는 스케줄러가 잡을 실행할 때의 CWD
    #   (pack/bin)에서 빗나가, 정작 SESSION_STATE·TODO 가 한 번도 세대에 담기지 않는다
    #   (실측: 50세대 전부 0건 — G1(d)의 목적이 조용히 배신됨). cys 표준 위치인
    #   ~/.cys/pack/round 를 HOME 파생으로 명시 추가한다(개인경로 하드코딩 아님 · 위
    #   장기기억 glob 과 동일 관례). 기존 라인은 부서/프로젝트 _round 하위호환 위해 존치.
    cys_round = os.path.join(home, ".cys", "pack", "round")
    srcs.append(os.path.join(cys_round, "SESSION_STATE.md"))
    srcs.extend(sorted(_glob.glob(os.path.join(cys_round, "*_TODO.md"))))
    srcs.extend(sorted(_glob.glob(os.path.join(
        home, ".claude*", "projects", "*", "memory", "*.md"))))
    return srcs

# GC 파라미터
KEEP_RECENT = 48        # 최근 N세대는 시간 무관 보관
KEEP_DAILY_DAYS = 14    # 최근 D일은 하루당 대표 1세대 보관

GEN_NAME_RE = re.compile(r"^(\d{8}T\d{6}Z)(?:-(\d+))?$")  # 20260703T085230Z 또는 ...-1
TMP_PREFIX = ".tmp-"


def _sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def _fsync_path(path):
    """파일 또는 디렉터리를 fsync 한다(디렉터리는 엔트리 durability 확보).
    ★Windows(S5): 디렉터리 fsync 는 건너뛴다 — Win32 는 디렉터리 핸들 fsync 를 거부(os.open(dir) PermissionError)한다.
    디렉터리 엔트리 durability 대신 os.rename(동일 볼륨 원자적 승격)에 의존한다(파일 fsync 는 그대로 수행)."""
    if os.name == "nt" and os.path.isdir(path):
        return
    flags = os.O_RDONLY
    if hasattr(os, "O_DIRECTORY") and os.path.isdir(path):
        flags |= os.O_DIRECTORY
    fd = os.open(path, flags)
    try:
        os.fsync(fd)
    except OSError:
        # 일부 파일시스템은 디렉터리 fsync를 거부할 수 있으나 rename 자체는 원자적이므로 치명 아님
        pass
    finally:
        os.close(fd)


def _now_utc():
    # 테스트 결정성을 위해 환경변수로 타임스탬프 주입 가능(self-test 용)
    override = os.environ.get("_CYS_SNAPSHOT_NOW")
    if override:
        return datetime.fromtimestamp(float(override), tz=timezone.utc)
    return datetime.now(timezone.utc)


def _gen_stamp(dt):
    return dt.strftime("%Y%m%dT%H%M%SZ")


def list_generations(gen_root=GEN_ROOT):
    """유효 세대 디렉터리만(.tmp-* 제외) 이름 오름차순으로 반환."""
    if not os.path.isdir(gen_root):
        return []
    out = []
    for name in os.listdir(gen_root):
        if name.startswith(TMP_PREFIX):
            continue
        if GEN_NAME_RE.match(name) and os.path.isdir(os.path.join(gen_root, name)):
            out.append(name)
    out.sort()
    return out


def _cleanup_tmp(gen_root):
    """이전 실행이 승격 전에 죽어 남긴 .tmp-* 잔재를 청소한다(반파 세대 제거)."""
    removed = []
    if not os.path.isdir(gen_root):
        return removed
    for name in os.listdir(gen_root):
        if name.startswith(TMP_PREFIX):
            p = os.path.join(gen_root, name)
            try:
                shutil.rmtree(p)
                removed.append(name)
            except OSError:
                pass
    return removed


def do_snapshot(sources=None, gen_root=GEN_ROOT, dry_run=False, crash_hook=None):
    """상태 파일 세대 1건을 원자적으로 생성한다. 생성된 세대 이름 반환(dry_run이면 None)."""
    sources = sources if sources is not None else default_sources()
    os.makedirs(gen_root, exist_ok=True)

    # 승격 전 죽은 잔재 청소
    _cleanup_tmp(gen_root)

    present = [s for s in sources if os.path.isfile(s)]
    missing = [s for s in sources if not os.path.isfile(s)]

    if dry_run:
        print(f"[dry-run] 세대 보관 대상 {len(present)}건:")
        for s in present:
            print(f"  + {s}  ({os.path.getsize(s)}B)")
        for s in missing:
            print(f"  - (없음) {s}")
        return None

    if not present:
        print("[snapshot] 보관할 소스 파일이 하나도 없음 — 세대 미생성", file=sys.stderr)
        return None

    dt = _now_utc()
    stamp = _gen_stamp(dt)

    # 1) 임시 세대 디렉터리에 완성
    tmp_dir = tempfile.mkdtemp(prefix=f"{TMP_PREFIX}{os.getpid()}-", dir=gen_root)
    manifest = {
        "created_at": dt.timestamp(),
        "created_at_iso": dt.isoformat(),
        "generator": "javis_state_snapshot.py",
        "files": [],
    }
    try:
        for src in present:
            digest = _sha256(src)
            st = os.stat(src)
            base = os.path.basename(src)
            # 소스명 충돌 방지: 경로 해시 접두
            dest_name = base
            if any(os.path.basename(x) == base for x in present if x != src):
                tag = hashlib.sha256(src.encode()).hexdigest()[:8]
                dest_name = f"{tag}__{base}"
            dest = os.path.join(tmp_dir, dest_name)
            with open(src, "rb") as fsrc, open(dest, "wb") as fdst:
                shutil.copyfileobj(fsrc, fdst)
                fdst.flush()
                os.fsync(fdst.fileno())
            manifest["files"].append({
                "source": src,
                "stored_as": dest_name,
                "sha256": digest,
                "size": st.st_size,
                "src_mtime": st.st_mtime,
            })

        mpath = os.path.join(tmp_dir, "manifest.json")
        with open(mpath, "w") as mf:
            json.dump(manifest, mf, indent=2, ensure_ascii=False)
            mf.flush()
            os.fsync(mf.fileno())

        _fsync_path(tmp_dir)

        # 중단 시뮬레이션 훅(self-test 전용): 승격 직전 프로세스를 강제 종료
        if crash_hook == "before_rename" or os.environ.get("_CYS_SNAPSHOT_CRASH_BEFORE_RENAME"):
            os._exit(137)  # SIGKILL 유사 — atexit/flush 없이 즉사

        # 2) 최종 이름으로 디렉터리 원자 승격(os.rename). 동일 초 충돌 시 카운터 부여.
        final_dir = os.path.join(gen_root, stamp)
        counter = 1
        while os.path.exists(final_dir):
            final_dir = os.path.join(gen_root, f"{stamp}-{counter}")
            counter += 1
        os.rename(tmp_dir, final_dir)  # POSIX: 동일 FS 내 원자적
        tmp_dir = None
        _fsync_path(gen_root)
    finally:
        if tmp_dir is not None and os.path.isdir(tmp_dir):
            shutil.rmtree(tmp_dir, ignore_errors=True)

    gen_name = os.path.basename(final_dir)
    print(f"[snapshot] 세대 생성: {gen_name}  (파일 {len(present)}건, 누락 {len(missing)}건)")

    # 3) GC
    kept, deleted = do_gc(gen_root=gen_root, dry_run=False)
    if deleted:
        print(f"[gc] {len(deleted)}세대 삭제, {len(kept)}세대 보관")
    return gen_name


def _parse_stamp(name):
    m = GEN_NAME_RE.match(name)
    if not m:
        return None
    return datetime.strptime(m.group(1), "%Y%m%dT%H%M%SZ").replace(tzinfo=timezone.utc)


def _gen_effective_dt(gen_root, name):
    """★E3(P2-5) 세대 실효 시각 — 이름의 명목 타임스탬프와 디렉터리 mtime 중 **더 나중**을 취한다.
    시계 역행 시 실제로 더 최근에 만든 세대가 이름은 lexically 작아지는데, mtime 이 이를 바로잡아 '최근'으로
    인식되게 한다(lexical 오선택·GC 오삭제 방어). 둘 다 없으면 None."""
    nominal = _parse_stamp(name)
    mt = None
    try:
        mt = datetime.fromtimestamp(os.path.getmtime(os.path.join(gen_root, name)), tz=timezone.utc)
    except OSError:
        pass
    cands = [d for d in (nominal, mt) if d is not None]
    return max(cands) if cands else None


def compute_gc(generations, now=None, dt_of=None, dt_eff=None):
    """보관/삭제 결정. (keep_set, delete_list) 반환. 순수함수(테스트 가능).
    ★E3(P2-5 + gemini W6): 명목 시각(dt_of=이름)과 실효 시각(dt_eff=mtime 병행)을 **둘 다 보호**한다(union).
    - 명목-recent(이름 최신) KEEP_RECENT 는 항상 보존 → cp/touch 로 과거 세대 mtime 이 오염돼도 **진짜 최근 세대를
      밀어내지 못한다**(gemini 오염 왜곡 차단).
    - 실효-recent(mtime 최신) KEEP_RECENT 도 보존 → 시계 역행으로 이름이 작아진 실 최근 세대가 보존된다(P2-5).
    dt_eff 미주입 시 dt_of 와 동일(하위호환·명목만). 오염 세대가 실효 집합에 추가로 잔존할 수 있으나(무해·상한
    2×KEEP_RECENT), 진짜 최근 세대의 오삭제는 발생하지 않는다."""
    now = now or _now_utc()
    _E0 = datetime.min.replace(tzinfo=timezone.utc)
    dt_of = dt_of or _parse_stamp        # 명목(이름)
    dt_eff = dt_eff or dt_of             # 실효(mtime 병행) — 미주입 시 명목과 동일
    keep = set()

    # 규칙 A: 최근 KEEP_RECENT — 명목 기준 ∪ 실효 기준(union → 명목-recent 절대 evict 안 됨·실효-recent 도 보존)
    for keyfn in (dt_of, dt_eff):
        ordered = sorted(generations, key=lambda n: (keyfn(n) or _E0), reverse=True)
        for name in ordered[:KEEP_RECENT]:
            keep.add(name)

    # 규칙 B: 최근 14일, 하루당 대표 1건 — 명목 시각 기준(날짜 귀속은 이름이 정직·오염 mtime 미의존)
    cutoff = now - timedelta(days=KEEP_DAILY_DAYS)
    day_rep = {}
    for name in sorted(generations, key=lambda n: (dt_of(n) or _E0), reverse=True):
        dt = dt_of(name)
        if dt is None or dt < cutoff:
            continue
        day = dt.strftime("%Y%m%d")
        if day not in day_rep:
            day_rep[day] = name
            keep.add(name)

    delete = [n for n in generations if n not in keep]
    return keep, sorted(delete)


def do_gc(gen_root=GEN_ROOT, dry_run=False):
    gens = list_generations(gen_root)
    keep, delete = compute_gc(gens, dt_of=_parse_stamp,
                              dt_eff=lambda n: _gen_effective_dt(gen_root, n))
    if dry_run:
        print(f"[gc dry-run] 총 {len(gens)}세대 → 보관 {len(keep)} / 삭제 {len(delete)}")
        for n in delete:
            print(f"  DELETE {n}")
        return sorted(keep), delete
    for n in delete:
        shutil.rmtree(os.path.join(gen_root, n), ignore_errors=True)
    if delete:
        _fsync_path(gen_root)
    return sorted(keep), delete


def do_verify(gen_root=GEN_ROOT, gen=None):
    """세대의 manifest 해시가 저장된 파일과 일치하는지 검증. 실패 시 exit 1."""
    gens = [gen] if gen else list_generations(gen_root)
    if not gens:
        print("[verify] 검증할 세대 없음", file=sys.stderr)
        return 1
    all_ok = True
    for name in gens:
        gdir = os.path.join(gen_root, name)
        mpath = os.path.join(gdir, "manifest.json")
        if not os.path.isfile(mpath):
            print(f"[verify] {name}: manifest.json 없음 — 반파 의심 FAIL")
            all_ok = False
            continue
        try:
            with open(mpath) as f:
                manifest = json.load(f)
        except (OSError, json.JSONDecodeError) as e:
            print(f"[verify] {name}: manifest 파싱 실패 {e} FAIL")
            all_ok = False
            continue
        ok = True
        for entry in manifest.get("files", []):
            stored = os.path.join(gdir, entry["stored_as"])
            if not os.path.isfile(stored):
                print(f"[verify] {name}: 저장파일 없음 {entry['stored_as']} FAIL")
                ok = False
                continue
            actual = _sha256(stored)
            if actual != entry["sha256"]:
                print(f"[verify] {name}: 해시 불일치 {entry['stored_as']} FAIL")
                ok = False
        if ok:
            print(f"[verify] {name}: OK ({len(manifest.get('files', []))}파일)")
        else:
            all_ok = False
    return 0 if all_ok else 1


def do_self_test():
    """원자성/중단내성 실증. 격리 임시폴더에서만 수행(라이브 상태 불변)."""
    print("=== javis_state_snapshot self-test (격리 임시폴더) ===")
    # ★Windows(S5): 중단내성 T2 만 os.fork(POSIX 전용)에 의존하므로 그 케이스만 [SKIP] 하고 T1·T3~T6 은 실 수행한다
    # (블랜킷 skip 은 false-green 이라 폐기 — 원자성·GC·부서 커버리지는 Windows 에서도 실측 검증). 상세는 T2 블록 참조.
    workdir = tempfile.mkdtemp(prefix="cys-snaptest-")
    try:
        src_dir = os.path.join(workdir, "src")
        gen_root = os.path.join(workdir, "gen")
        os.makedirs(src_dir)
        src_file = os.path.join(src_dir, "topology.json")
        payload = json.dumps({"entries": [{"role": "worker"}], "n": 1}).encode()
        with open(src_file, "wb") as f:
            f.write(payload)
        src_hash = _sha256(src_file)
        sources = [src_file]

        # T1: 정상 세대 생성 + 무결성
        gen = do_snapshot(sources=sources, gen_root=gen_root)
        assert gen is not None, "T1: 세대 생성 실패"
        gens = list_generations(gen_root)
        assert len(gens) == 1, f"T1: 세대 수 {len(gens)} != 1"
        rc = do_verify(gen_root=gen_root)
        assert rc == 0, "T1: verify 실패"
        print("  [PASS] T1 정상 세대 생성 + manifest 해시 검증")

        # T2: 중단 시뮬레이션 — 승격 직전 os._exit(자식 프로세스에서).
        # ★Windows(S5): os.fork 는 POSIX 전용 → 이 케이스만 [SKIP](PASS 아님·나머지는 실행). 원자성 자체는 T1/T3 가
        #   os.rename(동일 볼륨 원자 승격)으로 커버하고, 승격 전 크래시의 반파 회피는 POSIX drill(mac CI)로 확증한다.
        if os.name == "nt":
            print("  [SKIP] T2 승격전-중단 내성은 os.fork(POSIX) 필요 — Windows 미지원(mac CI 가 확증). 나머지 케이스는 실행.")
        else:
            pid = os.fork()
            if pid == 0:
                # 자식: 크래시 훅으로 승격 직전 즉사
                try:
                    do_snapshot(sources=sources, gen_root=gen_root, crash_hook="before_rename")
                except BaseException:
                    pass
                os._exit(0)
            _, status = os.waitpid(pid, 0)
            gens_after = list_generations(gen_root)
            assert len(gens_after) == 1, f"T2: 중단인데 세대 증가 {len(gens_after)} (반파 승격 발생!)"
            tmp_leftover = [n for n in os.listdir(gen_root) if n.startswith(TMP_PREFIX)]
            print(f"  [PASS] T2 승격 직전 중단 → 최종 세대 미증가(반파 없음), .tmp 잔재 {len(tmp_leftover)}건")

        # T3: 잔재 청소 — 다음 스냅샷이 .tmp-* 잔재 청소
        do_snapshot(sources=sources, gen_root=gen_root)
        tmp_leftover2 = [n for n in os.listdir(gen_root) if n.startswith(TMP_PREFIX)]
        assert len(tmp_leftover2) == 0, "T3: .tmp 잔재 미청소"
        assert len(list_generations(gen_root)) == 2, "T3: 세대 수 이상"
        print("  [PASS] T3 다음 실행이 반파 .tmp 잔재 청소 + 정상 세대 추가")

        # T4: 원본 불파괴 불변식
        assert _sha256(src_file) == src_hash, "T4: 원본 변조됨(불변식 위반!)"
        print("  [PASS] T4 원본 상태 파일 불파괴(읽기 전용 소비자)")

        # T5: GC 순수함수 — 60세대 합성, 최근48 + 일별14 보관 검증
        base = datetime(2026, 1, 1, tzinfo=timezone.utc)
        synth = []
        for i in range(60):
            synth.append(_gen_stamp(base + timedelta(hours=i)))
        now = base + timedelta(hours=59)
        keep, delete = compute_gc(synth, now=now)
        # 최근 48은 반드시 보관
        recent48 = sorted(synth, reverse=True)[:48]
        for n in recent48:
            assert n in keep, f"T5: 최근48 누락 {n}"
        assert len(keep) + len(delete) == 60, "T5: 보관+삭제 != 총량"
        print(f"  [PASS] T5 GC 정책(최근{KEEP_RECENT}+일별{KEEP_DAILY_DAYS}일): 60세대→보관{len(keep)}/삭제{len(delete)}")

        # T6: ★Phase7 부서 자동 커버리지 — 합성 state_root 에 메인 + 부서 2개(registry엔 1개만) 두고
        #     default_sources()가 손 배선/부서명 하드코딩 없이 전 부서 선언상태를 포함하는지, 그리고
        #     세대 스냅샷이 부서 선언파일을 실제로 담는지 실증(대형 .db/.log 은 제외됨도 확인).
        home6 = os.path.join(workdir, "home6")
        state_root6 = os.path.join(home6, ".local", "state")
        main6 = os.path.join(state_root6, "cys")
        os.makedirs(main6)
        with open(os.path.join(main6, "topology.json"), "w") as f:
            f.write('{"entries":[{"role":"worker"}]}')
        os.makedirs(os.path.join(home6, ".cys"))
        # registry 는 dept-A 만 등록(stale) — 하지만 디스크엔 dept-A·dept-B 둘 다 존재
        with open(os.path.join(home6, ".cys", "depts.json"), "w") as f:
            json.dump({"depts": {"dept-A": {"socket": os.path.join(state_root6, "cys-dept-dept-A", "cys.sock")}}}, f)
        for dep in ("cys-dept-dept-A", "cys-dept-dept-B"):
            dd = os.path.join(state_root6, dep)
            os.makedirs(dd)
            with open(os.path.join(dd, "schedule_state.json"), "w") as f:
                f.write('{"jobs":[]}')
            # 대형 산출물(포함되면 안 됨)
            with open(os.path.join(dd, "analytics.db"), "wb") as f:
                f.write(b"\x00" * 4096)
        srcs6 = default_sources(home=home6, state_root=state_root6)
        # 부서명 하드코딩 없이 stale-registry 밖의 dept-B 까지 발견돼야 함(파일시스템 truth)
        assert any("cys-dept-dept-A" in s and s.endswith("schedule_state.json") for s in srcs6), "T6: dept-A 선언상태 누락"
        assert any("cys-dept-dept-B" in s and s.endswith("schedule_state.json") for s in srcs6), "T6: dept-B(미등록) 누락 — 파일시스템 truth 실패"
        assert not any(s.endswith("analytics.db") for s in srcs6), "T6: 대형 .db 가 소스에 포함됨(제외 위반)"
        gen6_root = os.path.join(workdir, "gen6")
        do_snapshot(sources=srcs6, gen_root=gen6_root)
        gdir6 = os.path.join(gen6_root, list_generations(gen6_root)[0])
        man6 = json.load(open(os.path.join(gdir6, "manifest.json")))
        stored_srcs = [e["source"] for e in man6["files"]]
        assert any("cys-dept-dept-B" in s for s in stored_srcs), "T6: 스냅샷에 부서 선언상태 미포함"
        assert do_verify(gen_root=gen6_root) == 0, "T6: 부서 포함 세대 무결성 실패"
        print("  [PASS] T6 부서 자동 커버리지: stale-registry 밖 부서까지 glob 발견·스냅샷 포함·대형 .db 제외·무결성 OK")

        print("=== self-test 전체 PASS ===")
        return 0
    except AssertionError as e:
        print(f"=== self-test FAIL: {e} ===", file=sys.stderr)
        return 1
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


def main():
    # ★Windows 패리티(S5): 과거 os.name=="nt" hard-gate 는 제거됐다 — 디렉터리 fsync 를 Windows 에서 건너뛰고
    # (_fsync_path) os.rename(동일 볼륨 원자 승격)에 의존하므로 snapshot/list/gc/verify 가 Windows 에서 동작한다.
    ap = argparse.ArgumentParser(description="cys 상태 파일 세대 보관기 (무손실 복원 Phase 0)")
    sub = ap.add_subparsers(dest="cmd")

    p_snap = sub.add_parser("snapshot", help="세대 1건 생성 + GC")
    p_snap.add_argument("--dry-run", action="store_true")

    sub.add_parser("list", help="세대 목록")

    p_gc = sub.add_parser("gc", help="보관정책 적용")
    p_gc.add_argument("--dry-run", action="store_true")

    p_ver = sub.add_parser("verify", help="manifest 해시 무결성 검증")
    p_ver.add_argument("--gen", default=None)

    sub.add_parser("self-test", help="원자성/중단내성 실증(격리)")

    args = ap.parse_args()
    cmd = args.cmd or "snapshot"

    if cmd == "snapshot":
        do_snapshot(dry_run=getattr(args, "dry_run", False))
        return 0
    if cmd == "list":
        gens = list_generations()
        print(f"세대 {len(gens)}건 @ {GEN_ROOT}")
        for n in gens:
            gdir = os.path.join(GEN_ROOT, n)
            nf = len([x for x in os.listdir(gdir) if x != "manifest.json"]) if os.path.isdir(gdir) else 0
            print(f"  {n}  ({nf}파일)")
        return 0
    if cmd == "gc":
        do_gc(dry_run=getattr(args, "dry_run", False))
        return 0
    if cmd == "verify":
        return do_verify(gen=getattr(args, "gen", None))
    if cmd == "self-test":
        return do_self_test()
    ap.print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
