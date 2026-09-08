#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_role.py — 좌석 역할 해소의 **단일 소유 모듈** (0.14.31 P6 · 감사 codex E).

정본 금지 조항(IMPLEMENTATION-PLAN.md §8): "`CYS_ROLE` env 를 권위로 쓰지 않는다(승계 후
stale). 데몬 조회 우선." — 그런데 팩의 파이썬 도구들은 각자 `os.environ.get("CYS_ROLE")` 를
직접 읽어 결정했다. 좌석이 승계되면(claim-role·takeover) env 는 **낡은 채로 살아남고**
데몬 roles 맵만 바뀌므로, 그 결정들은 전부 옛 신원으로 내려진다:
  · `javis_org.py require_cso()`  — 부서 lifecycle mutation 단일소유 게이트
  · `javis_snapshot.py is_master()` — BOOT_SNAPSHOT 생산 게이트
  · `javis_completion_guard.py _role()` — 이벤트 `agent` 귀속 라벨
이 모듈이 그 **한 곳**이다. 훅 셸의 짝은 `hooks/_lib.sh` 의 `cys_resolve_role()` 이며
**같은 캐시 디렉터리·같은 레코드 문법**을 공유한다 — 두 층이 갈리면
tests/test_role_authority.py 가 멈춘다.

────────────────────────────────────────────────────────────────────────────
계약(데몬 CLI `cys surface-role` = src/bin/cys.rs:10940 `run_surface_role` 3상)
  ⓐ rc=0 + 역할 문자열 → **권위 있는 역할**
  ⓑ rc=0 + 빈 줄       → **권위 있는 '역할 없음'**(내 surface 가 데몬 목록에 없을 때도 이것 ·
                          `CYS_SURFACE_ID` 부재/파싱 불가면 조회 없이 이것)
  ⓒ 그 밖(rc≠0·타임아웃·바이너리 부재·**표현 불가한 역할 문자열**) → **판정 불가**.
     '역할 없음'이 아니다.
ⓑ와 ⓒ를 뭉개면 데몬 사망이 '무역할'로 읽힌다 — Rust 가 그 둘을 rc 로 갈라 놓았으므로
여기서도 절대 합치지 않는다.

★신원 전제(R1 교정 — 이 모듈이 데몬에 묻는 조건): Rust 가 **실제로 읽는 그 값 전체**를
  검사한다. 종전 초안은 첫 줄만 떼어 검사하고 원본을 그대로 CLI 에 물려줬다 —
  `CYS_SURFACE_ID="7\njunk"` 는 여기선 신원 7 로 통과하는데 Rust `parse_surface_ref` 는
  파싱에 실패해 **조회 없이 rc0+빈 줄**(=권위 있는 무역할)을 낸다. 그러면 해소기가
  **유효한 surface 7 아래에 '권위 있는 무역할'을 캐시**해 이후 정상 CSO 호출이 거짓 거부된다
  (reviewer-codex R1). 그래서 지금은 값 **전체**를 다음 규칙으로 판정한다:
    ① 키 선택: `CYS_SURFACE_ID` → `JAVIS_SURFACE_ID` → `AITERM_SURFACE_ID` 중 **비어 있지 않은**
       첫 값(Rust `env_compat` src/lib.rs:351 의 `filter(|v| !v.is_empty())` 와 같다 —
       공백만 있는 값도 '비어 있지 않음'이라 다음 키로 **넘어가지 않는다**)
    ② 64자 초과면 즉시 '주소 없음'
    ③ ASCII 공백 트림(Rust `str::trim` 의 **부분집합** — 유니코드 공백은 트림하지 않는다)
    ④ 선두 `surface:` 1회 제거(Rust 와 같은 순서 — 제거 후 재트림 없음)
    ⑤ `^[0-9]{1,19}$` 만 수용. **Rust 가 받는 `+7` 은 일부러 거절한다**(더 엄격한 쪽) —
       거절의 귀결은 조회 없음 → env 폴백 = **이 WP 이전 동작**이라 새 허용이 없다.
  주소가 없으면 조회 자체를 하지 않는다. 이유: surface 없는 실행(일반 터미널 ·
  `CYS_ROLE=cso python3 javis_org.py apply …` 같은 정식 위임 경로 · 검체 하네스)에서
  Rust 는 ⓑ(빈 줄·rc 0)를 내는데, 그것을 '권위 있는 무역할'로 채택하면 **주소가 없다는
  사실이 역할이 없다는 판정으로 승격**된다. 그 승격은 정상 경로를 죽인다.

★실패 방향(§3-3 "막는 쪽으로만 틀린다"): 판정 불가(ⓒ)의 귀결은 **현행 동작 그대로**(env
  폴백)다. 즉 이 모듈이 추가하는 실패 경로는 없다 — 데몬이 답할 때만 판정이 더 정확해진다.

★비용(부트체인 ④ '전 pane 사망' 회피): `_role()` 은 한 런에 20+회 불린다
  (javis_completion_guard.py :534 :626 :962 :1368 …). 그래서
    ① **프로세스 메모** — 신원(surface·socket)마다 하나 · **만료 있음**(아래)
    ② **디스크 캐시 60s** — 훅이 초당 여러 번 떠도 왕복은 분당 1회
    ③ **실패 백오프 30s** — 데몬이 죽어 있을 때 매 호출 2s 정지가 전 pane 에 걸리지 않게
  세 겹으로 막는다. 승계 반영 지연 ≤60s 는 **명시적으로 수용**한다.

★메모의 만료(R1 교정 · reviewer-codex): 종전 메모는 **영구**였다 — 오래 사는 임포터가
  승계·데몬 재기동 뒤에도 옛 답을 무한히 재사용했다. 지금은
    · 키 = (surface env 원값, socket env 원값) — 매 호출 env 로 재계산(I/O 0)
    · 만료 = 권위 답이면 **그 답의 만료 시각**(데몬 답이면 now+60, 디스크 캐시 히트면
      **그 레코드의 ts+60** — 다 된 캐시를 다시 60초 살려 지연 상한을 두 배로 만들지 않는다
      · codex R1 지적) · 판정 불가/폴백이면 now+30(=실패 백오프와 같은 창)
  이라 승계 반영 상한은 디스크 캐시와 **같은 60s** 로 유지된다.

★자식에게 `CYS_NO_AUTOSTART=1` 을 건다: 소켓 파일이 없으면 `cys` 는 autostart 경로를 탄다
  (src/bin/cys.rs:2258). **역할을 묻는 행위가 데몬을 낳아서는 안 된다** — 특히 `cys-dept`
  가드에서 부르는 경로는 아직 데몬이 없을 때 도는 경로다.

★캐시 기질(R1 전면 교체 — 두 리뷰어의 심링크·FIFO·별칭·고아 지적 일괄 봉인):
  ⓐ **전용 디렉터리** `<tmpbase>/cys-role-authority.d`(0700 · 소유자 자신 · group/other 쓰기 0).
     검증에 실패하면 **캐시를 통째로 끈다**(데몬에 매번 묻는다) — 남의 디렉터리를 신뢰하느니
     왕복 한 번이 낫다. tmp 루트에 파일을 흩뿌리지 않으므로 '예측 가능한 경로에 심어 둔
     심링크·FIFO' 부류가 구조적으로 닫힌다.
  ⓑ **유계 레코드 문법** `"<ts> <role> <epoch> <sockid>"` 1줄(판독 상한 4KB).
     ts=선두 0 없는 1~12자리(셸 산술이 8진수·오버플로로 죽던 길 차단) · role/epoch=`-` 또는
     `[A-Za-z0-9._:+-]{1,64}`(**공백 불가** — 같은 바이트가 두 층에서 `cso`/`cso ` 로 갈리던 길
     차단) · sockid=줄 나머지(정확 일치 비교).
  ⓒ boot-epoch 는 **파일명에서 레코드로** 옮겼다 — 재기동마다 고아 파일이 생기던 것을 없애고
     (파일은 (surface, socket 슬러그)당 정확히 하나), 슬러그 충돌은 **캐시 미스**로 강등된다
     (종전엔 다른 데몬의 역할이 권위로 읽힐 수 있었다 · reviewer-codex).
  ⓓ 모든 판독은 `O_NOFOLLOW|O_NONBLOCK` 로 연 뒤 fd 를 `fstat` 해 정규 파일·소유자 확인
     (FIFO 가 열기에서 훅을 영원히 붙잡던 길 차단 — 종전엔 검사 **전에** 블로킹 open 이었다).

stdlib 만 사용. import 부작용 0(파일 생성·env 변경·프로세스 스폰 전무 — 첫 `resolve_role()`
호출에서만 조회가 일어난다).

★출하 전제: `build.rs` 는 `git ls-files cysjavis-pack`(추적 파일 전용)으로 팩을 임베드한다.
  이 파일이 git 에 추적되지 않으면 설치본에 존재하지 않는다(javis_lane.py 헤더와 같은 경고).
"""
import os
import re
import subprocess
import sys
import tempfile
import time

__all__ = ["resolve_role", "resolve_role_detail", "reset_cache", "surface_id",
           "is_authoritative", "is_authoritative_none",
           "QUERY_TIMEOUT_S", "CACHE_TTL_S", "FAIL_BACKOFF_S"]

# ★상수는 **고정**이다(R1 교정): 종전 셸 짝은 `CYS_ROLE_CACHE_TTL` 류 env 로 덮을 수 있었고
#   파이썬은 못 덮어서 "두 층 규칙 동일"이 거짓이었다(reviewer-claude). 노브를 없애는 쪽으로
#   통일한다 — 정본 §3-4 "게이트를 끄는 노브 없음"과도 같은 방향이다.
QUERY_TIMEOUT_S = 2.0      # 초 · 자식 `cys surface-role` 데드라인(Rust 내장 10s 보다 짧게)
CACHE_TTL_S = 60           # 초 · 디스크 캐시 수명(= 승계 반영 지연 상한 · 명시적 수용)
FAIL_BACKOFF_S = 30        # 초 · 조회 실패 후 재조회 유예(데몬 사망 시 폭주·정지 차단)

CACHE_DIR_NAME = "cys-role-authority.d"
CACHE_READ_CAP = 4096      # 바이트 · 캐시 판독 상한
IDENT_MAX = 64             # surface env 원값 상한(그 이상은 신원이 아니다)
SOCKID_MAX = 512           # 소켓 신원 문자열 상한
SLUG_MAX = 80              # 파일명 성분 상한

# 레코드 토큰 문법 — 셸 짝 `cys_role_token_ok` 와 **글자 그대로 같은 집합**.
_TOKEN_RE = re.compile(r"^[A-Za-z0-9._:+-]{1,64}$")
# 타임스탬프 — 선두 0 금지 · 1~12자리(POSIX sh 산술이 8진수 해석·int64 초과로 죽지 않게).
_TS_RE = re.compile(r"^[1-9][0-9]{0,11}$")
# 신원 — Rust `parse_surface_ref`(src/lib.rs:2263)에 자릿수 상한 19 를 더한 **부분집합**.
_SID_RE = re.compile(r"^[0-9]{1,19}$")
# ASCII 공백만 트림한다(Rust `str::trim` 은 유니코드 공백까지 트림 — 우리는 더 엄격한 쪽).
_ASCII_WS = " \t\n\r\v\f"

# 판정 출처(진단·검체용). daemon/daemon-none 만 권위다.
SOURCE_DAEMON = "daemon"            # ⓐ 데몬이 구체 역할을 줬다
SOURCE_DAEMON_NONE = "daemon-none"  # ⓑ 데몬이 '역할 없음'을 확정했다
SOURCE_CACHE = "cache"              # 신선한 디스크 캐시(구체 역할)
SOURCE_CACHE_NONE = "cache-none"    # 신선한 디스크 캐시('역할 없음')
SOURCE_ENV_ROLE = "env-cys-role"          # 폴백: CYS_ROLE(판정 불가 · 주소 없음)
SOURCE_NONE = "none"                # 아무 근거도 없다(빈 역할)

# **권위 있는 답**의 집합. 소비처의 단조-거부(monotone deny) 합성은 이 술어로 갈린다:
#   권위 있는 답이면 그 답으로 **추가 거부**하고, 아니면 종전 env 판정을 그대로 쓴다.
#   → 이 WP 는 어떤 소비처에서도 **종전에 없던 허용을 만들지 않는다**(반파괴 half-op 봉인).
_AUTHORITATIVE = (SOURCE_DAEMON, SOURCE_DAEMON_NONE, SOURCE_CACHE, SOURCE_CACHE_NONE)
_AUTHORITATIVE_NONE = (SOURCE_DAEMON_NONE, SOURCE_CACHE_NONE)

SURFACE_ENV_KEYS = ("CYS_SURFACE_ID", "JAVIS_SURFACE_ID", "AITERM_SURFACE_ID")
SOCKET_ENV_KEYS = ("CYS_SOCKET", "JAVIS_SOCKET", "AITERM_SOCKET")


def is_authoritative(source):
    """`source` 가 데몬 권위(직접 응답 또는 그 응답의 신선한 캐시)인가."""
    return source in _AUTHORITATIVE


def is_authoritative_none(source):
    """데몬이 **역할 없음**을 확정했는가(판정 불가와 구별)."""
    return source in _AUTHORITATIVE_NONE


# (key, mono_expiry, wall_created, wall_expiry, (role, source)) — 두 시계로 만료하는 메모.
# 단조시계는 벽시계 조작에, 벽시계는 서스펜드를 세지 않는 단조시계에 각각 면역이다(R2).
_MEMO = None            # 일반 해소(`resolve_role_detail`)
_MEMO_CONFIRM = None    # 직접 확인(`confirm_role_detail` · 디스크 역할 캐시를 권위로 쓰지 않는다)


def reset_cache():
    """프로세스 메모 초기화 — 검체 전용(같은 프로세스에서 여러 env 를 재실측할 때)."""
    global _MEMO, _MEMO_CONFIRM
    _MEMO = None
    _MEMO_CONFIRM = None


# ── 문자열 규율(셸 짝과 글자 그대로 같은 규칙) ────────────────────────────────
def _first_line(value):
    """첫 줄만(CR·LF 어느 쪽이든 거기서 끊는다). 길이 자르기는 하지 않는다."""
    if not isinstance(value, str):
        return ""
    return value.replace("\r", "\n").split("\n", 1)[0]


def _line(value):
    """첫 줄 + ASCII 공백 트림 — 역할 문자열의 **단일 정규화**(두 층 동일)."""
    return _first_line(value).strip(_ASCII_WS)


def _env_compat(keys):
    """Rust `env_compat`(src/lib.rs:351) 미러 — **비어 있지 않은** 첫 값.

    ★공백만 있는 값도 '비어 있지 않음'이라 다음 키로 넘어가지 않는다(Rust 와 동일).
    """
    for k in keys:
        v = os.environ.get(k)
        if v:
            return v
    return ""


def _token_ok(s):
    return bool(_TOKEN_RE.match(s or ""))


def surface_id():
    """Rust 가 조회할 신원의 **정규화된 숫자부** 또는 ""(주소 없음).

    파일 헤더 ★신원 전제의 ①~⑤ 를 그대로 집행한다. 반환값은 선두 0 을 제거한 10진수라
    캐시 키가 `007`/`7` 로 갈리지 않는다(셸 짝도 같은 정규화를 한다).
    """
    raw = _env_compat(SURFACE_ENV_KEYS)
    if not raw or len(raw) > IDENT_MAX:
        return ""
    t = raw.strip(_ASCII_WS)
    if t.startswith("surface:"):
        t = t[len("surface:"):]
    if not _SID_RE.match(t):
        return ""
    return t.lstrip("0") or "0"


def _has_break(*vals):
    """어느 값에든 LF·CR 이 있는가 — 레코드는 1줄 문법이라 그 순간 신원이 표현 불가다."""
    for v in vals:
        if "\n" in (v or "") or "\r" in (v or ""):
            return True
    return False


# Windows 정규 종단점 접두 — `cys-dept:48`(MINGW/MSYS/CYGWIN)과 `src/lib.rs:385` 가 만든다.
# 셸 짝 `cys_role_sock_id_init` 의 `case` 패턴과 **글자 그대로** 같은 집합이다.
_PIPE_PREFIXES = ("\\\\.\\pipe\\", "\\\\?\\pipe\\")


def _is_abs_endpoint(v):
    r"""소켓 값이 **cwd 에 매달리지 않는 종단점**인가(유닉스 절대 경로 또는 Windows named pipe).

    ★`/` 가 든 값은 pipe 로 인정하지 않는다 — 정규 named pipe 경로에는 `/` 가 없고, 그 배제가
      유닉스의 백슬래시 상대 파일명을 pipe 로 오인할 여지를 좁힌다(codex 설계 비평 (k)).
    ★접두만으로는 부족하다: 이름이 비어 있는 `\\.\pipe\` 는 종단점이 아니다(codex (j)).
    """
    if not v:
        return False
    if v.startswith("/"):
        return True
    if "/" in v:
        return False
    for pre in _PIPE_PREFIXES:
        if v.startswith(pre) and len(v) > len(pre):
            return True
    return False


def _pct_esc(s):
    """기본 신원 성분의 **단사** 인코딩 — `%`→`%25` 를 먼저, 그 다음 `:`→`%3A`.

    순서가 뒤바뀌면 단사가 아니다(`:` 를 먼저 바꾸면 원문의 `%3A` 와 구별되지 않는다).
    셸 짝 `cys_role_pct_esc` 와 **글자 그대로** 같은 규칙이며, 바꾸는 것이 ASCII 두 글자뿐이라
    비-ASCII 는 원문 바이트 그대로 복사된다(두 층의 문자/바이트 셈 차이가 결과에 닿지 않는다).
    """
    return (s or "").replace("%", "%25").replace(":", "%3A")


def _sock_id():
    r"""데몬 신원 문자열 또는 **""(표현 불가 → 디스크 캐시 끔)**.

    캐시 **레코드에 그대로 실려** 정확 비교된다.
    ★왜 슬러그가 아니라 원문인가(reviewer-codex R1): `tr -c` 슬러그는 손실 치환이라
      `/tmp/a/b.sock` 과 `/tmp/a_b.sock` 이 **같은 파일명**을 만들었다 — 그러면 A 데몬의
      역할·실패표식이 B 데몬의 권위가 된다. 지금은 파일명이 겹쳐도 레코드의 sockid 가
      다르면 **캐시 미스**다(권위가 넘어가지 않는다).
    ★소켓 지정이 없을 때: Rust 기본 소켓은 상태 디렉터리에서 유도된다(src/lib.rs:379) →
      `default:<XDG_STATE_HOME>:<HOME>` 로 문맥을 구분한다(다른 HOME 이 키를 공유하지 않게).

    ★R2(major · reviewer-codex) — **자르지 않는다. 표현할 수 없으면 캐시를 끈다.**
      종전 규칙은 서로 다른 종단점을 하나의 신원으로 접었다:
        ⓐ 상대 경로(`CYS_SOCKET=cys.sock`)는 cwd 마다 다른 유닉스 소켓인데 같은 신원이었다.
           cwd 를 붙이는 방법은 두 층 파리티를 깨뜨린다(셸 `$(pwd -P)` 는 말미 개행을 먹고,
           Windows 의 `/foo`·`C:foo` 는 드라이브별 cwd 에 매달리며, 파이썬 `C:\\work` 와
           Git Bash `/c/work` 는 같은 자리를 다른 바이트로 적는다 · codex R2). 그래서
           **절대 경로가 아니면 신원 미지**로 간주하고 디스크 캐시를 끈다(데몬에 매번 묻는다).
        ⓑ `default:<XDG>:<HOME>` 은 값에 `:` 가 있으면 서로 다른 문맥을 같은 문자열로 접는다
           (`XDG=/x:state,HOME=/h` 와 `XDG=/x,HOME=state:/h`) → 그때도 신원 미지였다.
        ⓒ 512 초과 절단은 서로 다른 긴 경로를 같은 신원으로 만들었다 → 절단 대신 신원 미지.
    ★I7 수렴(판정관 T5 · 2026-09-08) — ⓐ·ⓑ 를 **거절이 아니라 표현**으로 바꾼다.
      Windows 의 정규 종단점은 named pipe 다(`bin/cys-dept:48` 이 `\\.\pipe\cys-dept-<n>` 을
      만들고 `src/lib.rs:385` 의 기본 소켓이 `\\.\pipe\cys` 다). 종전 규칙은 그것을 통째로
      '신원 미지'로 접어 **디스크 캐시도 `.fail` 백오프도 둘 다** 껐다 — 데몬이 죽어 있으면
      훅·도구 프로세스마다 `CYS_ROLE_QUERY_TIMEOUT`(2s)를 온전히 문다(§7 ④ 방향이 Windows 에서만
      사라진다). 드라이브 지정 `HOME=C:\Users\x` 도 `:` 규칙에 걸려 기본 신원조차 없었다.
        · ⓐ `\\.\pipe\<이름>` · `\\?\pipe\<이름>` 을 **명시적 절대 종단점**으로 인정한다
          (`_is_abs_endpoint`). 이름이 비어 있으면·`/` 가 섞이면 인정하지 않는다.
        · ⓑ `:` 를 거절하는 대신 성분을 **퍼센트 이스케이프**해 단사로 만든다(`%`→`%25` 를 먼저,
          그 다음 `:`→`%3A`). 구분자와 이스케이프 문자만 바꾸므로 비-ASCII 는 원문 그대로
          복사되고, 두 층이 `${#}`(셸마다 바이트/글자가 갈린다)에 의존하지 않는다 —
          길이 접두 인코딩을 버린 이유가 이것이다(codex 설계 비평 (i)).
      **남는 정직한 한계**: 유닉스에서 `\\.\pipe\x` 는 백슬래시가 든 **상대 파일명**일 수도
      있다. 그 형상을 절대 종단점으로 인정하면 cwd 가 다른 두 좌석이 한 신원을 공유한다.
      플랫폼으로 가르면 두 층(Git Bash sh vs 네이티브 파이썬)이 갈리므로 규칙은 순수하게 두고,
      대신 `/` 가 없는 정규 pipe 형상만 인정한다(실제 유닉스 소켓 경로는 `/` 를 포함한다).
      "신원 미지"의 귀결은 **디스크 캐시 없음 = 매번 데몬 조회**이고, 조회가 실패하면 이 WP
      이전 동작(env 폴백)이다 — 새 허용은 0 이지만 캐시가 실어 나르던 *거부*도 함께 사라진다는
      점은 정직하게 적어 둔다(코덱스 R2 지적 · 그 상태는 P6 이전 기준선과 같다).
    """
    v = _env_compat(SOCKET_ENV_KEYS)
    if v:
        if not _is_abs_endpoint(v):
            return ""                       # ⓐ 상대·드라이브 상대 = 신원 미지
    else:
        v = "default:%s:%s" % (_pct_esc(os.environ.get("XDG_STATE_HOME", "")),
                               _pct_esc(os.environ.get("HOME", "")))
    if len(v) > SOCKID_MAX or _has_break(v):
        return ""                           # ⓒ 절단하지 않는다
    return v


def _slug(s):
    """파일명 성분 — 셸 짝 `tr -cs 'A-Za-z0-9.-' '_'` 와 **글자 그대로** 같다.

    ★R2(codex 위임 차분 프로브 실측): R1 의 "파이썬도 UTF-8 바이트로 치환하니 셸(`tr`=바이트)과
      갈리지 않는다"는 **틀린 주장이었다**. macOS `tr` 는 `env -i`(로케일 없음)에서도 멀티바이트를
      **한 글자로** 세어 `/tmp/한글/소켓.sock` 을 `_tmp______.sock`(밑줄 6)로, 파이썬은
      `_tmp______________.sock`(밑줄 14)로 만들었다 — 두 층이 **다른 파일**을 쓴다.
      귀결은 권위 오판이 아니라 캐시 미스지만(레코드의 sockid 를 원문 대조하므로), 두 층이
      캐시를 공유한다는 계약 자체가 거짓이 된다.
    ★수정: **연속된 치환은 하나로 접는다**(`-s`). 허용 문자 집합이 순수 ASCII 라 두 층은 언제나
      **같은 구간**을 치환하고 길이만 달랐다 — 접고 나면 어떤 입력에서도 결과가 같아지며,
      결과가 ASCII 라 80자 절단의 단위(코드포인트/바이트) 문제도 함께 사라진다.
    ★I2 수렴(판정관 T2 · 2026-09-08): 접기만으로는 파리티가 서지 않았다 — `_` 가 **허용 문자**라
      입력에 원래 있던 `_` 는 파이썬이 그대로 흘리는데 `tr -s` 는 출력의 연속 `_` 를 **출처를
      가리지 않고** 접었다(`/tmp/a__b.sock` → py `_tmp_a__b.sock` vs sh `_tmp_a_b.sock`).
      규칙을 하나로 만든다: **허용 집합에서 `_` 를 뺀다**(`A-Za-z0-9.-`). 그러면 출력의 모든
      `_` 가 치환 산물이라 `-s` 와 아래 접기가 **언제나 같은 구간**을 접는다.
      대가는 `a_b` 와 `a-b`… 가 아니라 `a_b` 와 `a b` 가 같은 **파일명**이 되는 것뿐인데,
      권위는 파일명이 아니라 레코드의 `sockid` 원문 대조가 가른다(겹침의 귀결은 캐시 미스).
    """
    out = []
    for c in (s or "").encode("utf-8", "replace"):
        if (48 <= c <= 57) or (65 <= c <= 90) or (97 <= c <= 122) or c in (46, 45):
            out.append(chr(c))
        elif not out or out[-1] != "_":
            out.append("_")
    return "".join(out)[:SLUG_MAX]


# ── 캐시 기질 ────────────────────────────────────────────────────────────────
_O_NOFOLLOW = getattr(os, "O_NOFOLLOW", 0)   # Windows 에는 없다(0 = 무효과)
_O_NONBLOCK = getattr(os, "O_NONBLOCK", 0)   # FIFO 에서 open 이 매달리지 않게
_O_BINARY = getattr(os, "O_BINARY", 0)


def _euid():
    f = getattr(os, "geteuid", None) or getattr(os, "getuid", None)
    return f() if f is not None else None


def _cache_dir():
    """0700 전용 디렉터리 경로 또는 ""(캐시 사용 불가).

    셸 짝과 **같은 규칙**: `TMPDIR` → `TEMP` → `TMP` 중 비어 있지 않은 첫 값(없으면
    파이썬은 `tempfile.gettempdir()`, 셸은 `/tmp`) 아래의 `cys-role-authority.d`.
    ★정직한 한계: 네이티브 Windows 파이썬은 `TEMP`(예 `C:\\…\\Temp`)를 집고 Git Bash 셸은
      MSYS `/tmp` 를 집을 수 있다 — 그 플랫폼에서 두 층은 **캐시를 공유하지 못한다**.
      비용은 데몬 왕복이 층마다 한 번씩 더 도는 것뿐이고(권위는 캐시가 아니라 데몬),
      각 층의 캐시는 자기 안에서 정합하다. 검체는 이 해소 규칙 자체를 잰다.
    """
    base = ""
    for k in ("TMPDIR", "TEMP", "TMP"):
        v = os.environ.get(k)
        if v:
            base = v
            break
    if not base or not os.path.isdir(base):
        try:
            base = tempfile.gettempdir()
        except Exception:
            return ""
    d = os.path.join(base, CACHE_DIR_NAME)
    try:
        os.mkdir(d, 0o700)
    except FileExistsError:
        pass
    except Exception:
        return ""
    import stat as _stat
    try:
        st = os.lstat(d)
    except Exception:
        return ""
    if not _stat.S_ISDIR(st.st_mode):
        return ""                       # 심링크·정규 파일 등 = 신뢰 불가
    uid = _euid()
    if uid is not None:
        if st.st_uid != uid:
            return ""
        if st.st_mode & 0o022:          # group/other 쓰기 가능 = 신뢰 불가
            return ""
    return d


def _cache_path(sid):
    """(surface, socket 슬러그)당 정확히 하나. 캐시 불가면 "".

    ★R2(codex 위임 차분 프로브 실측): 종전에는 **신원 미지(`_sock_id()==""`)에서도 경로를 만들어**
      `role-<sid>-` 를 냈다 — 셸 짝 `cys_role_cache_path` 는 같은 상황에서 rc 1(경로 없음)이라
      두 층이 갈렸다. 호출측(`_resolve_uncached`)이 막고 있어 실동작은 옳았지만, 규칙을 이 함수가
      스스로 지키게 한다(다음 호출자·검체가 같은 함정에 빠지지 않게).
    """
    if not _sock_id():
        return ""
    d = _cache_dir()
    if not d:
        return ""
    return os.path.join(d, "role-%s-%s" % (_slug(sid or "none"), _slug(_sock_id())))


def _open_trusted(path):
    """캐시 파일을 **연 뒤** 그 fd 로 검증한다 — 검사 후 교체(TOCTOU)를 닫는다.

    ⓐ `O_NOFOLLOW` 로 심링크는 열리지 않는다(POSIX). ⓑ `O_NONBLOCK` 이라 FIFO 여도 open 이
    매달리지 않는다(R1 교정: 종전엔 **검사 전에** 블로킹 open 이라 훅이 영원히 멈출 수 있었다).
    ⓒ 연 fd 를 `fstat` 해 정규 파일·소유자 자신을 확인한다. ⓓ 실패하면 신뢰하지 않는다.
    ★Windows: `O_NOFOLLOW`·`st_uid` 가 없거나 무의미하다 → 그 축만 비고, 전용 디렉터리와
      TMPDIR 이 사용자별이라는 OS 계약에 기댄다. 캐시는 **권한 증명이 아니다**.
    """
    import stat as _stat
    try:
        fd = os.open(path, os.O_RDONLY | _O_NOFOLLOW | _O_NONBLOCK | _O_BINARY)
    except Exception:
        return None
    try:
        st = os.fstat(fd)
        if not _stat.S_ISREG(st.st_mode):
            os.close(fd)
            return None
        uid = _euid()
        if uid is not None and st.st_uid != uid:
            os.close(fd)
            return None
        return fd
    except Exception:
        try:
            os.close(fd)
        except Exception:
            pass
        return None


def _read_first_line(path):
    """신뢰 가능한 파일의 첫 줄(상한 4KB). 못 읽으면 None."""
    if not path:
        return None
    fd = _open_trusted(path)
    if fd is None:
        return None
    try:
        raw = os.read(fd, CACHE_READ_CAP)
    except Exception:
        raw = b""
    finally:
        try:
            os.close(fd)
        except Exception:
            pass
    try:
        text = raw.decode("utf-8", "replace")
    except Exception:
        return None
    return _first_line(text)


def _boot_epoch():
    """`dirname($CYS_SOCKET)/boot-epoch` 첫 줄 토큰 또는 `-`(모름).

    데몬이 부트마다 bump 한다(boot_supervisor.rs:856). 캐시 **레코드**에 넣어
    '데몬 재기동 = 캐시 무효'로 만든다(파일명에 넣던 종전 방식은 재기동마다 고아를 남겼다).

    ★정직한 한계(과장 금지): 이것은 **권위가 아니라 캐시의 세대 표식**이다.
      ⓐ Windows 의 소켓은 named pipe 라 `dirname` 이 상태 디렉터리가 아니다 → `-`.
      ⓑ 소켓 지정이 없거나 감독자 비활성·쓰기 실패면 → `-`.
      그 경우 무효화는 **TTL 60s 하나만** 남는다. 셸 짝도 같은 규칙이라 두 층이 갈리지 않는다.
    ★I7 수렴(codex 설계 비평 (j)): `/` 로 시작하지 않는 종단점에서는 **아예 읽지 않는다**.
      유닉스 `os.path.dirname("\\\\.\\pipe\\cys")` 는 `""` 라 상대 경로 `boot-epoch` 를 열었다
      — cwd 마다 다른 세대 표식이 레코드에 실려 같은 종단점의 캐시가 서로를 무효화한다.
      Windows 에서 이미 `-` 인 것과 같은 결과로 두 층·두 플랫폼을 한 규칙에 모은다.
    """
    sock = _env_compat(SOCKET_ENV_KEYS)
    if not sock or not sock.startswith("/"):
        return "-"
    try:
        line = _read_first_line(os.path.join(os.path.dirname(sock), "boot-epoch"))
    except Exception:
        return "-"
    if line is None:
        return "-"
    t = line.strip(_ASCII_WS)
    return t if _token_ok(t) else "-"


def _record(now, role, epoch, sockid):
    return "%d %s %s %s\n" % (now, role if role else "-", epoch or "-", sockid)


def _parse_record(line, sockid, epoch):
    """(ts, role) 또는 None. 문법 위반·신원 불일치·세대 불일치는 전부 '캐시 없음'이다."""
    if not line:
        return None
    parts = line.split(" ", 3)
    if len(parts) != 4:
        return None
    ts_s, role, ep, sk = parts
    if not _TS_RE.match(ts_s):
        return None
    if not _token_ok(role) or not _token_ok(ep):
        return None
    if sk != sockid or ep != epoch:
        return None
    return int(ts_s), role


def _cache_write(path, text):
    """0600 · 같은 디렉터리 원자 교체. 실패는 조용히 무시한다(캐시는 최적화지 사실이 아니다).

    ★`O_CREAT|O_EXCL` 로 임시 이름을 **새로** 만든다 — 기존 파일에 바로 쓰면 그 자리에 심어 둔
      심링크의 목적지를 truncate 하는 길이 된다. 최종 배치는 `os.replace`(원자 교체)다.
    """
    if not path:
        return
    tmp = "%s.%d.tmp" % (path, os.getpid())
    try:
        try:
            os.unlink(tmp)
        except Exception:
            pass
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | _O_BINARY, 0o600)
        try:
            os.write(fd, text.encode("utf-8"))
        finally:
            os.close(fd)
        os.replace(tmp, path)
    except Exception:
        try:
            os.unlink(tmp)
        except Exception:
            pass


def _query_daemon():
    """(role, ok) — ok=False 는 **판정 불가**(ⓒ)다. role="" + ok=True 는 권위 있는 무역할(ⓑ).

    ★자식 env: `CYS_NO_AUTOSTART=1` 강제(역할 조회가 데몬을 낳지 않게) ·
      `PYTHONDONTWRITEBYTECODE`/`PYTHONUTF8` 은 부모 값을 그대로 상속한다.
    ★표현 불가한 역할(공백 포함·64자 초과·문법 밖)은 **판정 불가**로 낸다 — 잘라 쓰면 없는
      역할을 지어내는 것이고, 그 자리에서 캐시 문법도 깨진다. 판정 불가의 귀결은 종전 동작이다.
    """
    exe = _line(os.environ.get("CYS_BIN", "")) or "cys"
    env = dict(os.environ)
    env["CYS_NO_AUTOSTART"] = "1"
    try:
        p = subprocess.run([exe, "surface-role"], capture_output=True, text=True,
                           encoding="utf-8", errors="replace",
                           timeout=QUERY_TIMEOUT_S, env=env)
    except Exception:
        # FileNotFoundError(cys 부재) · TimeoutExpired · PermissionError … 전부 판정 불가.
        return "", False
    if p.returncode != 0:
        return "", False
    role = _line(p.stdout)
    if role and not _token_ok(role):
        return "", False
    return role, True


def _env_fallback():
    """폴백은 **`CYS_ROLE` 하나뿐**이다 — 현행 결정 지점들이 읽는 바로 그 키.

    ★`CYS_SURFACE_ROLE` 을 일반 폴백에 넣지 않는 이유(codex R1): 그 변수는
      `role-capability-gate.sh` 가 **해소 결과로 export 하는 산출물**이지 신원 입력이 아니다.
      폴백에 넣으면 `CYS_SURFACE_ROLE=cso` + `CYS_ROLE=worker` 에서 **현행이 거부하던 것을
      새로 허용**하게 된다 — "판정 불가면 현행 그대로"라는 이 WP 의 실패 방향 약속이 거짓이 된다.
      그 두 키를 함께 보는 소비처(`hooks/inject-context.sh`)는 자기 계약으로 직접 본다.
    """
    v = _line(os.environ.get("CYS_ROLE", ""))
    if v:
        return v, SOURCE_ENV_ROLE
    return "", SOURCE_NONE


def _resolve_uncached(now, trust_cache=True):
    """(role, source, expires_at) — 만료 시각은 프로세스 메모의 상한이 된다.

    ★`trust_cache=False`(I5 수렴 · codex 설계 비평 (g)): **디스크 역할 캐시를 권위로 읽지 않는다**.
      새 허용(=stale env 를 뒤집는 판정)의 근거는 살아 있는 데몬의 직접 응답뿐이어야 한다 —
      캐시 레코드는 같은 uid 의 아무 프로세스나 쓸 수 있으므로 그것을 통과 근거로 삼으면
      위조 한 줄이 부서 lifecycle mutation 을 연다. `.fail` 백오프는 **그대로 존중한다**
      (데몬 사망 시 매 호출 2s 정지가 전 pane 에 걸리는 것이 §7 ④ 방향이다) — 백오프에 걸리면
      판정 불가로 강등되고 그 귀결은 종전 env 동작이다(거부 방향).
    """
    fb_exp = now + FAIL_BACKOFF_S
    sid = surface_id()
    if not sid:
        # 주소가 없다 = 데몬에게 '나'를 물을 수 없다. 무역할이라는 **판정이 아니다**.
        role, src = _env_fallback()
        return role, src, fb_exp
    sockid = _sock_id()
    epoch = _boot_epoch()
    # 신원을 표현할 수 없으면(상대 경로·모호한 기본 인코딩·512 초과) 디스크 캐시를 쓰지 않는다
    # — 자르거나 뭉개서 **남의 데몬 역할을 권위로 읽는 것**보다 왕복 한 번이 낫다(R2).
    cpath = _cache_path(sid) if sockid else ""
    if cpath and trust_cache:
        rec = _parse_record(_read_first_line(cpath), sockid, epoch)
        if rec:
            ts, role = rec
            if 0 < ts <= now and (now - ts) < CACHE_TTL_S:
                exp = ts + CACHE_TTL_S      # ★다 된 캐시를 메모가 되살리지 않는다(codex R1)
                if role == "-":
                    return "", SOURCE_CACHE_NONE, exp
                return role, SOURCE_CACHE, exp
    fpath = (cpath + ".fail") if cpath else ""
    skip = False
    if fpath:
        rec = _parse_record(_read_first_line(fpath), sockid, epoch)
        if rec:
            ts, _r = rec
            skip = (0 < ts <= now and (now - ts) < FAIL_BACKOFF_S)
    if not skip:
        role, ok = _query_daemon()
        if ok:
            _cache_write(cpath, _record(now, role, epoch, sockid))
            if fpath:
                try:
                    os.unlink(fpath)
                except Exception:
                    pass
            if role:
                return role, SOURCE_DAEMON, now + CACHE_TTL_S
            return "", SOURCE_DAEMON_NONE, now + CACHE_TTL_S
        _cache_write(fpath, _record(now, "-", epoch, sockid))   # 실패 표식(형식 공유)
    # ⓒ 판정 불가 — 신선하지 않은 캐시는 쓰지 않는다(옛 역할이 무기한 사는 길). env 폴백.
    role, src = _env_fallback()
    return role, src, fb_exp


def _memo_key():
    """프로세스 메모의 신원 키 — **표현 불가한 신원도 문맥을 잃지 않는다**(R2 · codex).

    `_sock_id()` 는 상대 경로에서 ""(신원 미지)를 내는데, 그 하나로 키를 잡으면 cwd 만 바꾼
    **다른 소켓**이 같은 메모를 재사용한다. 그래서 원값 두 개에 더해, 소켓 값이 절대 종단점이
    아닐 때만 cwd 를 키에 싣는다(절대 종단점에서는 cwd 가 판정에 영향을 주지 않으므로 부르지도
    않는다 — 호출당 getcwd 1회를 아낀다).

    ★I6 수렴(판정관 T4 · 2026-09-08): 종전 키는 `(raw_surface, raw_socket, cwd)` 뿐이라
      **기본 종단점의 입력을 통째로 빠뜨렸다**. 소켓 지정이 없으면 신원은
      `default:<XDG_STATE_HOME>:<HOME>` 인데(`_sock_id`), 그 둘이 키에 없으니 한 프로세스가
      HOME 을 A→B 로 바꿔 다시 해소하면 **디스크 신원이 달라졌는데도** A 데몬의 답이 재사용됐다.
      `_sock_id()` 의 **산출**을 키에 실으면 그 산출이 곧 디스크 신원이라 정의상 빠짐이 없다
      (I7 의 퍼센트 이스케이프로 기본 신원이 단사가 됐으므로 `:` 가 든 값도 서로 구별된다).
    """
    raw_s = _env_compat(SURFACE_ENV_KEYS)
    raw_k = _env_compat(SOCKET_ENV_KEYS)
    cwd = ""
    if raw_k and not _is_abs_endpoint(raw_k):
        try:
            cwd = os.getcwd()
        except Exception:
            cwd = "\x00unknown"      # 알 수 없는 cwd 는 어떤 실제 cwd 와도 같지 않다
    return (raw_s, raw_k, _sock_id(), cwd)


def _resolve_memoized(trust_cache):
    """`resolve_role_detail`/`confirm_role_detail` 공통 몸통 — 메모 슬롯만 갈린다.

    소비처는 `source` 로 '권위 있는 무역할'(daemon-none/cache-none)과 '판정 불가 후 env 폴백'
    을 구분할 수 있다 — `cys-dept` 단일소유 가드가 그 구분을 쓴다.

    ★프로세스 메모의 수명(R2 · reviewer-codex): 종전은 벽시계 하나였다 — 시계를 되돌리면
      `now < expires_at` 이 계속 참이라 권위 있는 옛 답이 60초를 훌쩍 넘겨 살아남았다.
      지금은 **두 시계를 동시에** 만족해야 재사용한다:
        ⓐ `time.monotonic()` 이 만료 전(벽시계 조작에 면역)
        ⓑ 벽시계가 생성 시각 이후이고(역행 검출) 벽시계 만료 전(단조시계가 서스펜드를 세지
           않는 플랫폼 — macOS 가 그렇다 — 에서 절전 1시간 뒤 메모가 살아남던 길 차단)
      수명은 **답의 나이에 앵커**한다: 디스크 캐시 히트는 `레코드ts+60`, 데몬 답은 `now+60`,
      폴백은 `now+30`. 두 시계 모두 해소 **시작 시각**에 고정하므로 해소가 오래 걸려도 수명이
      늘어나지 않는다. 남은 수명이 0 이하면 **메모하지 않는다**(종전의 `now+1` 갱신은 만료된
      답을 1초 되살리는 길이었다 — 삭제).
    """
    global _MEMO, _MEMO_CONFIRM
    try:
        now = int(time.time())
        mono = time.monotonic()
        key = _memo_key()
        memo = _MEMO if trust_cache else _MEMO_CONFIRM
        if memo is not None and memo[0] == key:
            _mexp, _wcreated, _wexp, _val = memo[1], memo[2], memo[3], memo[4]
            if mono < _mexp and _wcreated <= now < _wexp:
                return _val
        role, src, exp = _resolve_uncached(now, trust_cache)
        remaining = exp - now
        fresh = (key, mono + remaining, now, exp, (role, src)) if remaining > 0 else None
        if trust_cache:
            _MEMO = fresh
        else:
            _MEMO_CONFIRM = fresh
        return role, src
    except Exception:
        # 이 모듈이 소비처를 죽이는 경로는 없다(§3-3) — 최악이 현행(env) 동작이다.
        try:
            return _env_fallback()
        except Exception:
            return "", SOURCE_NONE


def resolve_role_detail():
    """(role, source) — 데몬 권위 우선 · 캐시 · env 폴백. **예외를 내지 않는다**.

    (아래 서술은 `_resolve_memoized` 의 계약이며 이 함수가 그 기본 진입점이다.)
    """
    return _resolve_memoized(True)


def confirm_role_detail():
    """(role, source) — **디스크 역할 캐시를 권위로 쓰지 않는** 직접 확인(I5 · codex 비평 (g)).

    새 허용(=stale `CYS_ROLE` env 를 뒤집어 통과시키는 판정)의 유일한 근거다. 반환 `source` 가
    `daemon`/`daemon-none` 이면 살아 있는 데몬이 **방금** 답한 것이고, 그 밖이면(백오프·데몬
    사망·주소 없음) 판정 불가라 소비처는 **종전 env 판정 그대로**로 강등한다(거부 방향).

    ★왜 캐시를 통과 근거에서 빼는가: 캐시 레코드는 같은 uid 의 아무 프로세스나 쓸 수 있다
      (게이트는 도구 호출만 본다). 캐시를 통과 근거로 삼으면 위조 한 줄이 부서 lifecycle
      mutation 을 여는 **새 허용**이 된다 — 거부 방향으로만 틀린다는 §3-3 약속이 깨진다.
      거부 방향(캐시가 '비-cso'라고 말할 때 더 막는 것)에는 종전대로 캐시를 쓴다.
    """
    return _resolve_memoized(False)


def resolve_role(default=""):
    """역할 문자열(없으면 `default`). 결정 지점의 **유일한 입구**."""
    role, _src = resolve_role_detail()
    return role or default


def _self_test():
    """밀폐 자기검증 — 데몬 3상 x 폴백을 스텁 `cys` 로 실측(팩 `--self-test` 관례).

    ★Windows-safe: 스텁은 `os.name` 으로 `.bat`/`sh` 를 갈라 쓴다(Git Bash 없이도 돈다).
    """
    import shutil
    fails = []

    def check(name, cond, detail=""):
        print("%s %s%s" % ("PASS" if cond else "FAIL", name,
                           (" - " + detail) if detail else ""))
        if not cond:
            fails.append(name)

    keys = ("CYS_SURFACE_ID", "JAVIS_SURFACE_ID", "AITERM_SURFACE_ID", "CYS_ROLE",
            "CYS_SURFACE_ROLE", "CYS_SOCKET", "JAVIS_SOCKET", "AITERM_SOCKET",
            "CYS_BIN", "TMPDIR", "TEMP", "TMP")
    saved = {k: os.environ.get(k) for k in keys}
    td = tempfile.mkdtemp(prefix="javis-role-st-")
    n = [0]
    try:
        def stub(rc, out):
            """rc·stdout 을 고정하는 `cys` 스텁을 만들고 CYS_BIN 으로 가리킨다."""
            n[0] += 1
            if os.name == "nt":
                p = os.path.join(td, "cys-stub-%d.bat" % n[0])
                body = "@echo off\r\n"
                body += ("echo(%s\r\n" % out) if out else "echo(\r\n"
                body += "exit /b %d\r\n" % rc
            else:
                p = os.path.join(td, "cys-stub-%d.sh" % n[0])
                body = "#!/bin/sh\n"
                body += ("printf '%s\\n' " + _sh_quote(out) + "\n") if out else "printf '\\n'\n"
                body += "exit %d\n" % rc
            with open(p, "w", encoding="utf-8", newline="") as f:
                f.write(body)
            os.chmod(p, 0o755)
            os.environ["CYS_BIN"] = p

        def fresh(**env):
            for k in keys:
                os.environ.pop(k, None)
            os.environ["TMPDIR"] = tempfile.mkdtemp(dir=td)
            os.environ.update({k: v for k, v in env.items() if v is not None})
            reset_cache()

        fresh(CYS_SURFACE_ID="7", CYS_ROLE="master")
        stub(0, "cso")
        check("데몬 답 우선(env=master 인데 데몬=cso)",
              resolve_role_detail() == ("cso", SOURCE_DAEMON), repr(resolve_role_detail()))

        fresh(CYS_SURFACE_ID="7", CYS_ROLE="master")
        stub(2, "")
        check("판정 불가(rc=2) -> env 폴백",
              resolve_role_detail() == ("master", SOURCE_ENV_ROLE), repr(resolve_role_detail()))

        fresh(CYS_SURFACE_ID="7")
        stub(2, "")
        check("둘 다 없음 -> 빈 역할",
              resolve_role_detail() == ("", SOURCE_NONE), repr(resolve_role_detail()))

        fresh(CYS_SURFACE_ID="7", CYS_ROLE="cso")
        stub(0, "")
        check("권위 있는 무역할(rc0+빈줄)이 env 를 덮는다",
              resolve_role_detail() == ("", SOURCE_DAEMON_NONE), repr(resolve_role_detail()))

        fresh(CYS_ROLE="cso")
        stub(0, "")
        check("surface 없음 -> 조회 없이 env(주소 부재는 무역할 판정이 아니다)",
              resolve_role_detail() == ("cso", SOURCE_ENV_ROLE), repr(resolve_role_detail()))

        fresh(CYS_SURFACE_ID="7\njunk", CYS_ROLE="cso")
        stub(0, "")
        check("★여러 줄 신원은 Rust 도 거절한다 -> 조회 없이 env(권위 무역할 캐시 오염 차단)",
              resolve_role_detail() == ("cso", SOURCE_ENV_ROLE), repr(resolve_role_detail()))

        fresh(CYS_SURFACE_ID="7", CYS_ROLE="master")
        stub(0, "cso")
        resolve_role_detail()
        reset_cache()
        stub(2, "")                       # 데몬 사망 — 신선 캐시가 답한다
        check("신선 캐시가 판정 불가를 메운다",
              resolve_role_detail() == ("cso", SOURCE_CACHE), repr(resolve_role_detail()))
    finally:
        shutil.rmtree(td, ignore_errors=True)
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        reset_cache()
    if fails:
        print("javis_role self-test FAIL (%d): %s" % (len(fails), fails))
        return 1
    print("javis_role self-test PASS")
    return 0


def _sh_quote(s):
    """POSIX sh 단일따옴표 인용(self-test 스텁 생성 전용 · shlex 의존 회피)."""
    return "'" + str(s).replace("'", "'\\''") + "'"


def main(argv):
    if "--self-test" in argv:
        return _self_test()
    role, src = resolve_role_detail()
    print("%s\t%s" % (role, src))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
