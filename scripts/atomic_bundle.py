#!/usr/bin/env python3
"""atomic_bundle.py — macOS 앱 번들 **원자 교체 계약(ATOMIC-1)** 의 파이썬 집행부.

★정본은 Rust `src/app_bundle.rs` 다. 이 파일은 같은 계약을 파이썬 도구(deploy_gate.py)가 쓸 수
있게 옮긴 것이고, **규칙이 갈라지면 안 된다**(순서·검증 항목·실패 시 처리 모두 동일).

────────────────────────────────────────────────────────────────────────────
왜 이 파일이 필요한가 — 2026-08-01 실사고

`/Applications/cys.app/Contents/Info.plist` 가 사라진 **반쪽 번들**이 남았다. 결과:
  · `codesign` → `bundle format unrecognized, invalid, or unsuitable`
  · `syspolicyd` → `Unable to open app for protection: 2`(ENOENT) · `bundle_id: (null)`
  · 커널이 번들 등록에 실패해 내부 `cys`·`python3` 를 미등록 단독 실행파일로 평가 → **859회 하드 거부**
사용자에게는 "손상되었기 때문에 열 수 없습니다" 한 줄만 보인다.

**기제(격리 재현)**: 살아있는 앱 번들 위에 `cp -R`·`ditto` 로 부분 복사를 하면, 어떤 파일에서
`Operation not permitted`(App Management/TCC)가 나도 **복사는 멈추지 않는다**. rc=1 한 줄만 남고
나머지는 전부 교체돼 세대 혼합 반쪽 번들이 확정된다. ⇒ `cp -R` 도 `ditto` 도 트랜잭션이 아니다.

────────────────────────────────────────────────────────────────────────────
계약 5단 (이 순서를 바꾸면 계약이 아니다)

  ① 같은 파일시스템의 임시 위치에 **완본**을 만든다. 비트랜잭션 복사는 **여기서만** 허용한다
     (실패해도 더럽혀지는 건 스크래치뿐이다).
  ② 그 완본을 **검증**한다(Info.plist + 필수 실행물 + codesign --verify --strict).
  ③ 통과했을 때만 교체한다. 교체는 `renamex_np(RENAME_SWAP)` **한 번의 시스템 콜**이다
     (실패하면 아무것도 바뀌지 않는다). 폴백은 rename 2단 + 실패 즉시 원복.
  ④ 교체 후 **다시 검증**한다.
  ⑤ 어느 단계든 실패하면 기존 번들을 그대로 둔 채 **큰 소리로 실패**한다.

금지 형태 둘(둘 다 실제 코드에 있었다):
  · `rm -rf <app> && mv <new> <app>` — 삭제가 먼저라 이동 실패 = 앱 전소.
  · 백업을 임시 디렉터리에 두고 최종 rename 실패 시 **되돌리지 않는** 경로(백업이 함께 증발).
"""
import ctypes
import os
import subprocess
import sys

# ── 완본의 정의 (Rust `VerifySpec::installed()` 와 동일 목록 — 어긋나면 둘 중 하나가 거짓말이다) ──
REQUIRED_EXECUTABLES = ("cys-app", "cysd", "cys")
REQUIRED_RESOURCES = (
    "Resources/runtime/python/bin/python3",
    "Resources/pack.tar.gz",
    "Resources/pack-manifest.json",
)

_libc = ctypes.CDLL("libc.dylib", use_errno=True)
_libc.renamex_np.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
_libc.renamex_np.restype = ctypes.c_int
RENAME_SWAP = 0x2


class BundleError(Exception):
    """교체 계약 위반·실패. 메시지에는 **기존 번들의 상태**가 반드시 들어간다."""


def staging_path(dest):
    """목적지와 **같은 디렉터리**의 스테이징 경로. 같은 디렉터리면 같은 파일시스템이 자명하고,
    rename 계열의 원자성 전제가 무료로 성립한다. 점 접두라 Finder 에 보이지 않는다."""
    parent, name = os.path.dirname(dest.rstrip("/")), os.path.basename(dest.rstrip("/"))
    return os.path.join(parent, f".{name}.deploy-staging-{os.getpid()}")


def same_filesystem(a, b):
    """rename 원자성의 전제. 판정 불가는 보수적으로 False(=교체 거절)."""
    try:
        return os.stat(a).st_dev == os.stat(b).st_dev
    except OSError:
        return False


def swap_paths(a, b):
    """두 경로의 내용을 **한 번의 시스템 콜로 맞바꾼다**(APFS/HFS+ · macOS 10.12+).
    성공하면 a 에 옛 내용이, b 에 새 내용이 있다. 실패하면 아무것도 바뀌지 않는다.
    둘 중 하나라도 없으면 ENOENT."""
    rc = _libc.renamex_np(os.fsencode(a), os.fsencode(b), RENAME_SWAP)
    if rc != 0:
        err = ctypes.get_errno()
        raise OSError(err, os.strerror(err), a, None, b)


def verify_bundle(path, executables=REQUIRED_EXECUTABLES, resources=REQUIRED_RESOURCES,
                  codesign=True):
    """완본 검증 — **결손 목록**을 돌려준다(빈 리스트 = 완본).
    bool 로 접지 않는 이유: 무엇이 없는지 말해야 사람이 고칠 수 있다."""
    defects = []
    contents = os.path.join(path, "Contents")
    if not os.path.isdir(contents):
        return ["Contents 디렉터리 없음(번들 아님)"]
    plist = os.path.join(contents, "Info.plist")
    # ★사고의 최종 상태가 정확히 이것 — 첫 번째로 본다.
    if not os.path.isfile(plist):
        defects.append("Contents/Info.plist 없음 — macOS 가 앱으로 인식하지 못한다")
    elif os.path.getsize(plist) == 0:
        defects.append("Contents/Info.plist 0바이트(복사 중단)")
    for name in executables:
        p = os.path.join(contents, "MacOS", name)
        # os.path.isfile 은 심볼릭 링크를 따라간다 → 끊어진 링크는 '없음'으로 잡힌다(의도).
        if not os.path.isfile(p):
            defects.append(f"실행물 Contents/MacOS/{name} 없음")
        elif os.path.getsize(p) == 0:
            defects.append(f"실행물 Contents/MacOS/{name} 0바이트(복사 중단)")
        elif not os.access(p, os.X_OK):
            defects.append(f"실행물 Contents/MacOS/{name} 실행 권한 없음")
    for rel in resources:
        if not os.path.exists(os.path.join(contents, rel)):
            defects.append(f"필수 구성요소 Contents/{rel} 없음")
    if codesign:
        r = subprocess.run(["/usr/bin/codesign", "--verify", "--strict", "--verbose", path],
                           capture_output=True, text=True)
        if r.returncode != 0:
            msg = " | ".join(
                l.strip() for l in (r.stdout + r.stderr).splitlines()
                if l.strip() and not l.startswith(("--prepared:", "--validated:"))
            )
            defects.append(f"코드서명 봉인 실패: {msg[:400] or f'exit {r.returncode}'}")
    return defects


def stage_copy(src, staged):
    """계약 ① — `src` 를 빈 스크래치 `staged` 로 완본 복사(xattr·ACL·심볼릭링크 트리 보존).

    ★rc 를 반드시 본다: `ditto` 는 EPERM 한 건에도 나머지를 계속 복사하고 rc=1 만 남긴다(실측).
    rc 를 무시하면 반쪽 복사본이 '완본'으로 다음 단계에 넘어간다 — 그게 사고의 형태다.
    (그래도 rc 만 믿지 않는다. 계약 ②의 검증이 두 번째 그물이다.)"""
    if os.path.exists(staged):
        subprocess.run(["/bin/rm", "-rf", staged], check=True)
    r = subprocess.run(["/usr/bin/ditto", "--rsrc", "--extattr", "--acl", src, staged],
                       capture_output=True, text=True)
    if r.returncode != 0:
        raise BundleError(f"스테이징 복사 실패(rc={r.returncode}): {r.stderr.strip()} — 기존 번들 무접촉")


def install_atomically(staged, dest, verify_kwargs=None, post_verify_kwargs=None):
    """계약 ②③④⑤ 집행. 성공하면 옛 번들이 있는 경로를 돌려준다(신규 설치면 None).

    **불변식**: 어떻게 끝나든 `dest` 는 검증을 통과한 새 번들이거나 손대지 않은 기존 번들이다.
    반쪽 상태는 만들어지지 않는다. 되돌리기까지 실패한 경우에만 예외를 던지되, 그때는 옛 번들의
    **실제 경로**를 메시지에 넣어 사람이 손으로 되돌릴 수 있게 한다(조용한 소실 금지).

    ★`post_verify_kwargs`(④의 기준)를 ②와 **다르게 좁힐 수 있다**. 생략하면 종전과 똑같이 ②와
    같은 기준을 쓴다(기본 동작 불변). 왜 이 구멍이 필요한가 — ②와 ④는 같은 이름의 검사지만
    **대상의 성질이 다르다**:
      · ② 는 아직 아무도 모르는 숨김 스크래치를 본다 → 우리가 만든 그대로다. 봉인(codesign)까지 봐야 한다.
      · ④ 는 **살아 있는 설치 위치**를 본다 → 스왑이 끝난 그 순간부터 제3자(구 데몬·CLI·훅)가
        합법적으로 쓸 수 있다. 실측된 대표 사례가 동봉 python 의 `.pyc` 생성이고(SEAL-1 의 존재 이유),
        그 한 건이 봉인 검사를 깨뜨려 **성공한 교체를 자동 원복시킨다**. 스왑은 내용을 바꾸지 않으므로
        ④에서 봉인을 다시 보는 것은 ②의 재확인이 아니라 **경합 조건 관측**이다.
    그래서 호출부는 ④를 '구조 완본성만'으로 좁힐 수 있다. 봉인 게이트는 ②(스왑 직전·같은 비트)에
    그대로 남으므로 "봉인 깨진 번들이 설치된다"는 경로는 열리지 않는다(계약 약화 아님).
    ★정본 Rust `install_bundle_atomically_verifying(staged, dest, spec, post_spec)` 와 같은 구멍이다."""
    vk = verify_kwargs or {}
    pvk = vk if post_verify_kwargs is None else post_verify_kwargs
    # ② 완본 검증 — 통과하지 못하면 목적지를 건드리지도 않는다.
    defects = verify_bundle(staged, **vk)
    if defects:
        raise BundleError(
            "새 번들이 완본이 아니라 교체를 중단했다(기존 번들 무접촉):\n  - " + "\n  - ".join(defects))
    if not same_filesystem(os.path.dirname(staged) or "/", os.path.dirname(dest) or "/"):
        # ★복사 폴백을 두지 않는다 — 복사는 트랜잭션이 아니고, 그 폴백이 곧 사고의 기제다.
        raise BundleError(
            f"스테이징({staged})과 목적지({dest})가 다른 파일시스템 — 원자 교체 불가(기존 번들 무접촉)")

    # ③ 교체.
    previous = None
    if not os.path.lexists(dest):
        os.rename(staged, dest)          # 신규 설치 — 단일 rename 이 곧 원자 교체
        atomic = False
    else:
        try:
            swap_paths(staged, dest)     # 전부 아니면 전무
            previous, atomic = staged, True
        except OSError:
            previous, atomic = _two_step_replace(staged, dest), False

    # ④ 교체 후 재검증 → 실패하면 ⑤에 따라 완전히 되돌린다(기준은 pvk — 함수 주석 참조).
    post = verify_bundle(dest, **pvk)
    if post:
        restored = _undo(staged, dest, previous, atomic)
        raise BundleError(
            "교체 후 검증 실패 — {}:\n  - {}".format(
                "기존 상태로 되돌렸다" if restored
                else f"★되돌리지 못했다. 옛 번들: {previous} · 설치 위치: {dest}",
                "\n  - ".join(post)))
    return previous


def _two_step_replace(staged, dest):
    """원자 스왑을 못 쓰는 파일시스템용 폴백. 대피처를 **목적지와 같은 부모**에 둔다 —
    임시 디렉터리에 두면 프로세스가 죽는 순간 되돌릴 대상 자체가 증발한다.

    ★모든 실패는 `BundleError` 로만 나간다(계약 예외). 종전에는 **대피 단계**(`rm -rf` 잔해 정리와
    `rename(dest, parked)`)가 try 밖에 있어 raw `OSError`·`CalledProcessError` 가 그대로 샜다.
    그 갈래는 이론이 아니라 실 `/Applications` 에서 가장 흔한 형태다 — 원자 스왑이 거부되고(1차 실패)
    이어서 대피 rename 마저 App Management(TCC)·권한에 막히는(2차 실패) 이중 실패 경로.
    거기서 계약 밖 예외가 나가면 호출부는 "기존 번들이 어떤 상태인지"를 타입으로 알 수 없고,
    정본 Rust(`InstallError::SwapFailed{...}` · `destination_intact()==true`)와도 갈라진다.
    대피가 실패했다는 것은 `dest` 를 **아직 건드리지 못했다**는 뜻이므로 기존 번들은 무접촉이다."""
    parked = f"{dest}.previous-{os.getpid()}"
    try:
        if os.path.exists(parked):
            subprocess.run(["/bin/rm", "-rf", parked], check=True)
        os.rename(dest, parked)
    except (OSError, subprocess.SubprocessError) as e:
        raise BundleError(
            f"기존 번들 대피 실패({e}) — 교체를 시작하지 못했다(기존 번들 무접촉). 설치 위치: {dest}") from e
    try:
        os.rename(staged, dest)
    except OSError as e:
        try:
            os.rename(parked, dest)
        except OSError as e2:
            raise BundleError(
                f"★수동 복구 필요 — 새 번들 안착 실패({e}) 후 원복도 실패({e2}). "
                f"옛 번들: {parked} · 설치 위치: {dest}") from e
        raise BundleError(f"새 번들 안착 실패({e}) — 기존 번들 원복 완료(무접촉과 동일 결과)") from e
    return parked


def _undo(staged, dest, previous, atomic):
    """교체를 완전히 되돌린다. 성공하면 설치 위치는 교체 이전과 글자 그대로 같다.
    ★설치 위치에 `*.failed-*` 같은 잔해를 남기지 않는다 — 사용자 눈에 보이는 곳에 반쪽을 두지 않는다."""
    try:
        if previous and atomic:
            swap_paths(previous, dest)          # 대칭 연산 1콜
        elif previous:
            subprocess.run(["/bin/rm", "-rf", staged], check=False)
            os.rename(dest, staged)
            os.rename(previous, dest)
        else:
            subprocess.run(["/bin/rm", "-rf", staged], check=False)
            os.rename(dest, staged)             # 신규 설치 → 설치 전 '부재' 상태로 복원
        return True
    except OSError:
        return False


def sweep_stale_staging(dest):
    """이전 실행이 남긴 스테이징 잔해 정리(같은 부모의 `.{name}.deploy-staging-*`).
    스왑 성공 후 남는 옛 번들도 여기 있으므로, **백업이 따로 있는 도구**에서만 호출한다."""
    parent = os.path.dirname(dest.rstrip("/")) or "/"
    name = os.path.basename(dest.rstrip("/"))
    swept = []
    prefix = f".{name}.deploy-staging-"
    for entry in os.listdir(parent):
        if entry.startswith(prefix):
            p = os.path.join(parent, entry)
            subprocess.run(["/bin/rm", "-rf", p], check=False)
            swept.append(p)
    return swept


# ── 설치 도우미(Install cys.app) 전용 얇은 CLI 래퍼 ────────────────────────────
# ★로직 이중구현 금지 — 아래는 stage_copy + install_atomically 를 **부르는 배선일 뿐**이다.
#   설치 도우미의 install-core.sh 는 `/usr/bin/python3 atomic_bundle.py install <src> <dst>` 로 이 경로만
#   탄다(번들 내부에 봉인된 사본). 계약(순서·검증·실패 처리)은 위 함수들이 곧 정본이다.
def _cli_install(src, dest):
    """소스 cys.app 을 목적지에 **원자적으로** 설치한다(신규=rename · 업그레이드=renamex_np SWAP).

    ② 스왑 직전 게이트는 **봉인(codesign)까지** 본다 — 스테이플된 소스의 무결성이 ditto 복사로
       그대로 승계됐는지 설치 직전에 확인한다(DEMO3: Gatekeeper 승계). 봉인이 깨진 반쪽/변조 소스는
       목적지를 건드리지 않고 거부된다.
    ④ 스왑 직후 재검증은 **구조 완본성만**(codesign=False) 본다 — 스왑은 비트를 한 바이트도 바꾸지
       않으므로 봉인 판정은 ②(같은 비트·스왑 직전)에 이미 있고, 업그레이드 시 살아있는 구 데몬이 새
       경로에 `.pyc` 를 써 봉인 검사를 깨뜨려 **성공한 교체를 자동 원복**시키는 실측 경합을 피한다
       (deploy_gate.POST_SWAP_VERIFY 와 동형 · 근거는 deploy_gate.swap_bundle_into_place 주석).

    ★소스 진위 게이트(서명·Team ID Q43YA2NMF9·Apple 공증)는 여기가 아니라 **위층**
      installer-app/install-core.sh 에 있다 — 소스의 동봉 python 을 실행하기 **전에** 시스템
      codesign·spctl 로 검증해야 공격자 소스 코드를 root 로 돌리는 LPE 경로를 닫는다(순서 불변).
      이 공유 함수에 공증 게이트를 넣지 않는 이유: deploy_gate.py 는 아직 공증 전인 자기 빌드
      산출물을 이 경로로 설치하므로, 여기서 공증을 강제하면 정상 배포 파이프라인이 깨진다."""
    src = src.rstrip("/")
    dest = dest.rstrip("/")
    if not os.path.isdir(os.path.join(src, "Contents")):
        print(f"✗ 소스가 .app 번들이 아님: {src}", file=sys.stderr)
        return 2
    staged = staging_path(dest)
    try:
        print(f"① 스테이징 완본 복사 → {staged}")
        stage_copy(src, staged)
        print("②③④⑤ 완본 검증 → 원자 교체 → 재검증")
        previous = install_atomically(staged, dest, post_verify_kwargs={"codesign": False})
    except BundleError as e:
        subprocess.run(["/bin/rm", "-rf", staged], check=False)  # 실패 시 스테이징 잔해 정리(설치본 무접촉)
        print(f"✗ 설치 실패 — {e}", file=sys.stderr)
        return 1
    if previous:  # 업그레이드: 스왑으로 밀려난 옛 번들(숨김 스테이징)을 정리
        subprocess.run(["/bin/rm", "-rf", previous], check=False)
    print(f"✓ 원자 설치 완료: {dest}")
    return 0


if __name__ == "__main__":
    _argv = sys.argv[1:]
    if len(_argv) == 3 and _argv[0] == "install":
        raise SystemExit(_cli_install(_argv[1], _argv[2]))
    print("사용: atomic_bundle.py install <SRC cys.app> <DEST cys.app>", file=sys.stderr)
    raise SystemExit(64)
