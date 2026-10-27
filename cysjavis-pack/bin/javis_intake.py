#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_intake.py — 접수-지불 대장 (오너 제정 2026-10-15 · CEO 구현)

【무엇을 고치는가】 오너 실측: 부서장이 지시받은 업무 중 **일부를 빠뜨린 채** 일을 끝냈다고
보고하는 치명적 오류. 기존 장치로는 보이지 않는다 — `javis_task.py` 는 **만들어진** 항목의
증거를 요구하지만(exit 5), **만들어지지 않은 항목**은 어떤 계수기도 볼 수 없다.
⇒ 빠진 것은 분자가 아니라 **분모**다. 이 도구는 접수 시점에 분모를 박는다.

【세 법이 이 설계를 정했다 — 전부 2026-10-15 측정에서 나왔다】
 ⒜ (오너) 관리감독은 항목을 **하나하나** 체크하는 것이다.
 ⒝ (CEO C52) 보류·완료의 단위는 번들이 아니라 **항목**이다. 번들 **이름**은 블로커를 선언하지
    못한다 — 이름 한 줄이 여섯 항목을 덮고 그 문장은 다섯에 대해 거짓이었다.
 ⒞ (dept-6 R23 · ★CEO 의 계수 기반 법을 반증한 측정) 침묵으로 해소한 쪽과 대가를 지불해
    해소한 쪽이 **같은 항 수**를 낸다. ⇒ **같은 수에 공짜로 닿는 경로는 침묵된 모순의 지문이다.
    몇 항 남았는지 묻지 말고 무엇을 지불했는지 물어라.**
 ⇒ 그래서 이 대장은 **계수기가 아니라 지불 검사기**다. 체크박스를 세지 않는다.

【결정론 계약 — exit code 가 사실이다】
  0  PAID      ★뜻이 서브커맨드마다 다르다 — 완료 보고 자격은 `status`·`audit` 의 0 뿐이다.
               · status : 판정한 대장(--id 하나 · 생략 시 이 좌석의 대장 · --all 은 팩 전체)의 항목 전부가 지불됐다
                          (증거 또는 선언된 보류). 완료 보고 가능.
               · audit  : 모든 루트의 모든 대장이 위와 같다.
               · pay·defer : 그 항목 하나가 지불·보류됐다(다른 항목은 열려 있을 수 있다).
               · add-item·stamp·selftest : 명령이 성공했다(add-item 은 열린 항목을 하나 늘린다).
               · open 은 0 을 내지 않는다 — 새 항목은 열려 있으므로 1, 항목이 없으면 2 다.
  1  OPEN      지불되지 않은 항목이 있다. 그 id 를 **전부** 출력한다. ⇒ 완료 보고 금지.
  2  NO_DENOM  지시는 기록됐는데 **항목 목록이 없다**(분모 부재). ★이것은 통과가 아니다 —
               「검사되지 않음」이 아니라 「검사 불가능」이다. 0 과 반드시 구별된다.
  3  NO_LEDGER 이 레인에 접수 기록이 아예 없다. ★빈 대장을 「열린 것 없음」으로 읽지 않는다.
  4  DRIFT     기록된 지시 원문의 다이제스트가 재계산과 다르다(지시가 사후 변조·드리프트).
  5  USAGE     사용법 오류 또는 writer lock busy/unavailable.
  9  UNRESOLVABLE  이 호스트에서 풀 수 없는 경로 이름 공간(파일 I/O 없음).
 10  EXEMPT    맨 문자열 증거: stat 으로 정규 파일 존재만 확인, 신원은 미확립. PAID 아님.
 11  NOT_COMMITTED  commit 전 저장 실패; 기존 대장은 그대로.
 12  COMMITTED/DURABILITY_UNCONFIRMED  교체 완료, 디렉터리 동기화 실패.
 13  COMMITTED/DURABILITY_UNVERIFIED  POSIX 동기화 미지원; 앞 단계의 미검증도 누적.
 14  CONFLICT  payments 와 v1 필드 공존; 수락 여부를 추정하지 않고 양쪽을 출력.
     비 POSIX 디렉터리 동기화 부재: commit rc 0 + 필수 stderr 고지(open 의 의미 exit 는 별도).
  6  SELFTEST  self-test 실패.
  8  UNREADABLE★artifact 가 **있는데 읽히지 않는다**(open/read 가 OSError). ★CHANGED(1) 와 다른 사실이다 —
               읽지 못한 것은 바뀐 것이 아니고 지불의 무효도 아니다. 이 코드는 **열린 항목이 없고** 판독 불가만
               남은 대장에서만 나온다(열림이 하나라도 있으면 1 이다 — 심각도는 숫자 크기가 아니라 `_SEVERITY` 다).
               ★완주 자격(0)은 아니다. 그러면 좌석은 무엇을 하는가 — **정당한 경로는 `defer`(사유 +
               release_condition)** 다: 「무엇이 이것을 막고 있는가」를 적어 항목을 닫는다(법 ⒝). ⛔읽히지 않는
               파일을 대신할 증거를 **지어내는 것**은 경로가 아니다(그 지시가 이 코드를 만든 이유다). 파일을
               내려받아 되살리는 것도 이 도구의 일이 아니다 — 오너 규칙이 실체화·복사를 금한다.

【지불의 정의 — 비어 있으면 거부한다】
 항목이 닫히는 길은 둘뿐이다.
  · done  : `evidence` 텍스트가 MIN_EVIDENCE_CHARS 이상 **그리고** `artifacts` 중 최소 1건이
            **디스크에 실존**해야 한다(경로를 적는 것으로는 지불이 아니다 — 기계가 stat 한다).
            ★지불 시점에 **정규 파일의 지문**을 얻은 artifact 만 지불을 싣는다(디렉터리·읽기 불가·FIFO·
            부재는 싣지 않는다). 지문 없이 실존만 보는 면제는 지문 칸 이전의 맨 문자열 기록뿐이다.
  · defer : `defer_reason` 이 MIN_REASON_CHARS 이상 **그리고** `release_condition` 이 비어 있지
            않아야 한다. ★「무엇이 이것을 막고 있는가」를 항목마다 적게 만든다(법 ⒝).
 그 밖의 모든 상태는 OPEN 이다. ⛔체크박스·true/false 플래그만으로는 어떤 항목도 닫히지 않는다.

【사용】
  intake open  --id <대장id> --instruction-file <파일> --item "<항목>" [--item ...] [--source owner|ceo]
  intake add-item   --id <대장id> --item "<항목>"
  intake pay        --id <대장id> --item-no <N> --evidence "<텍스트>" --artifact <경로> [--artifact ...]
  intake defer      --id <대장id> --item-no <N> --reason "<사유>" --release-condition "<조건>"
  intake status     [--id <대장id>] [--root <팩루트>] [--json]
  intake audit      --roots <루트1,루트2,...>        # CEO 제2검사자 — 레인 밖에서 읽는다
  intake selftest
"""
import argparse
import copy
import functools
import importlib.util
import tempfile
import errno
import hashlib
import json
import ntpath
import os
import re
import stat
import sys
import time

MIN_EVIDENCE_CHARS = 40      # 지불로 인정하는 증거 최소 길이(문자) — "했다"·"ok" 불통과
MIN_REASON_CHARS = 20        # 보류 사유 최소 길이
SCHEMA = 2
ID_RE = re.compile(r"^[A-Za-z0-9._:-]{1,64}$")

EXIT_PAID, EXIT_OPEN, EXIT_NO_DENOM, EXIT_NO_LEDGER, EXIT_DRIFT, EXIT_USAGE, EXIT_SELFTEST = 0, 1, 2, 3, 4, 5, 6
# ★7 UNATTRIBUTED — 대장에 좌석 귀속이 없다(dept-1 가 재현 조건과 함께 올림 · 2026-10-15).
#   종전 `status` 는 --id 없이 그 팩의 **모든** 대장을 합산했고 대장에는 좌석 성분이 없었다 ⇒
#   ★항목 0건·지불 0건인 좌석이 **다른 좌석의 지불로 exit 0** 을 얻었다. A4 가 그 0 을 완료 보고
#   자격으로 쓰므로 이것은 구멍이다. 귀속이 없는 대장은 ★판정하지 않는다 — 7 로 거부한다.
#   (dept-5 의 법과 같은 뼈: 합계가 맞는 것은 각 항목이 지불됐다는 주장이 아니다.)
EXIT_UNATTRIB = 7
# ★8 UNREADABLE — artifact 가 **있는데 읽히지 않는다**(CEO 지시 2026-10-23 · WP10).
#   CEO 실측: 기록된 크기 그대로 present 인데 모든 읽기가 OSError errno 11(EDEADLK) 로 막혔다(macOS 의
#   dataless·compressed 플래그 — 내용이 클라우드로 비워진 파일이다. 901 참조 중 24건, 데스크톱 트리의 14.7%).
#   ⛔종전: 지문 재계산이 실패를 (None, None) 으로 삼켰고 재확인은 그것을 「★증거 바이트가 변경됐다 … 그 절만
#   떠낸 새 파일로 재지불하라」로 찍었다 ⇒ **읽을 수 없는 증거 대신 새 증거를 지어내라**는 지시였다.
#   ⇒ 판독 불가는 변경이 아니다. 지불은 무효가 아니다. 신원 재확인이 **불가능**할 뿐이고, 크기 일치는 정황이지
#   신원이 아니다(읽을 수 있는 sha 가 있어야 신원이다).
EXIT_UNREADABLE = 8

EXIT_UNRESOLVABLE, EXIT_EXEMPT = 9, 10
EXIT_NOT_COMMITTED, EXIT_COMMITTED_UNCONFIRMED, EXIT_COMMITTED_UNVERIFIED, EXIT_CONFLICT = 11, 12, 13, 14
_TAG = dict(enumerate(('PAID', 'OPEN', 'NO_DENOM', 'NO_LEDGER', 'DRIFT', 'USAGE', 'SELFTEST',
                      'UNATTRIBUTED', 'UNREADABLE', 'UNRESOLVABLE', 'EXEMPT', 'NOT_COMMITTED',
                      'COMMITTED/DURABILITY_UNCONFIRMED', 'COMMITTED/DURABILITY_UNVERIFIED', 'CONFLICT')))
_SEVERITY = {code: rank for rank, code in enumerate((0, 10, 8, 14, 9, 1, 2, 3, 4, 7))}


def _tag(rc):
    """exit code 의 이름. ★종전은 `[...][min(rc, 4)]` 였다 — 그 꼬리표는 7 도, 8 도 「DRIFT」로 찍는다."""
    return _TAG.get(rc, "rc=%d" % rc)


def _worse(a, b):
    """더 나쁜 쪽 — `_SEVERITY` 의 뜻 순서로 고른다(숫자 비교가 아니다)."""
    return b if _SEVERITY.get(b, 99) > _SEVERITY.get(a, 99) else a


def _root(explicit=None):
    """대장이 사는 자리 — 자기 레인의 팩 루트. ★base 경로 하드코딩은 교차 오염이다."""
    if explicit:
        return os.path.abspath(explicit)
    return os.path.abspath(os.environ.get("CYS_PACK_DIR") or os.path.expanduser("~/.cys/pack"))


def _dir(root):
    return os.path.join(root, "round", "_intake")


def _path(root, lid):
    return os.path.join(_dir(root), "%s.json" % lid)


def _digest(b):
    return hashlib.sha256(b).hexdigest()


def _seat():
    """(role, surface, authoritative) — ★좌석 신원은 **데몬**에서 받는다.

    ★2026-10-16 수리(dept-2 후보 1). 종전은 `CYS_SURFACE_ID`·`CYS_ROLE` env 만 읽었다.
    ⛔그런데 데몬 소스가 명문으로 적는다 — 「`CYS_ROLE` 은 **권위가 아니다**」(결합 판정에 쓰지
    않는다). 그리고 env 가 **없는 pane** 에서는 surface 비교가 **건너뛰어지고** 판정이 조용히
    **역할 대조로 떨어졌다** ⇒ 그 부서 실측: 「pane 불일치는 제외하지 않고 role 불일치는 제외한다」
    ⇒ ★`7 UNATTRIBUTED` 가 이름이 약속하는 만큼 보호하지 못했다.
    ⇒ 지금은 `cys identify`(데몬이 **커널 peer pid** 로 해소한 값)를 **1순위**로 쓴다.
    해소 실패면 env 를 **비권위 표지와 함께** 쓰고, 둘 다 없으면 ★판정하지 않는다(None).
    """
    role = os.environ.get("CYS_ROLE") or None
    sid = None
    auth = False
    try:
        import subprocess
        out = subprocess.run(["cys", "identify"], capture_output=True, text=True, timeout=10)
        if out.returncode == 0:
            j = json.loads(out.stdout)
            c = (j.get("caller") or {})
            if c.get("surface_id") is not None:
                sid, auth = str(c["surface_id"]), True
    except Exception:
        pass
    if sid is None:
        sid = os.environ.get("CYS_SURFACE_ID") or None
    return (role, sid, auth)


def _live_surfaces():
    """살아있는 surface id 집합 — ★함대 재번호 뒤 「없어진 좌석」을 가리기 위한 것.
    못 읽으면 None(= 판정에 쓰지 않는다 · 측정 불가를 통과로 바꾸지 않는다)."""
    import subprocess
    try:
        out = subprocess.run(["cys", "status", "--json"], capture_output=True, text=True, timeout=10)
        d = json.loads(out.stdout)
        return {str(x.get("surface_id")) for x in (d.get("surfaces") or []) if x.get("surface_id") is not None}
    except Exception:
        return None


def _mine(led):
    """이 대장이 이 좌석의 것인가 — (bool|None). None = 귀속 없음(판정 불가).

    ★2026-10-16 수리: 함대가 재구성되면 surface 번호가 바뀐다(실측: master 94 → 107).
    종전 판정은 기록된 surface 를 **문자 비교**만 해서, 번호가 바뀐 순간 그 대장이
    `OTHER_SEAT` 로 제외되고 좌석에 귀속분 0건 ⇒ ★`NO_LEDGER`(3) 가 나왔다 —
    ⛔**오너 절대규칙의 첫 행동 게이트가 재시작마다 깨진다.**
    ⇒ 기록된 surface 가 **살아있는 레지스트리에 없으면** 그 귀속은 ★낡은 좌표이므로
    **역할**로 대조한다(⛔번호를 지어내지 않는다 · 역할도 다르면 남의 것이다).
    """
    ob = led.get("opened_by")
    if not isinstance(ob, dict) or not (ob.get("surface") or ob.get("role")):
        return None
    role, sid, _auth = _seat()
    rec_sid = ob.get("surface")
    if rec_sid and not sid:
        # ★자기 좌석을 해소하지 못했다 — 기록에는 좌석이 있다 ⇒ **판정 불가**다.
        # ⛔역할로 떨어뜨려 통과시키지 않는다(측정 없음을 통과로 바꾸지 않는다).
        return None
    if rec_sid and sid:
        if str(rec_sid) == str(sid):
            return True
        live = _live_surfaces()
        if live is not None and str(rec_sid) not in live:
            # ★기록된 좌석이 사라졌다 — 번호가 아니라 역할로 대조한다.
            if ob.get("role") and role:
                return str(ob["role"]) == str(role)
            return None
        return False
    if ob.get("role") and role:
        return str(ob["role"]) == str(role)
    return None


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


SYNC_OK, SYNC_UNSUPPORTED, SYNC_FAILED, SYNC_NONPOSIX = 'SYNC_OK', 'SYNC_UNSUPPORTED', 'SYNC_FAILED', 'SYNC_NONPOSIX'
_AUTO_PRIOR = object()


def _sync_dir(path):
    if os.name != 'posix':
        return SYNC_NONPOSIX
    unverified = (errno.EACCES, errno.EPERM, errno.EINVAL, errno.ENOTSUP,
                  getattr(errno, 'EOPNOTSUPP', errno.ENOTSUP), errno.ENOSYS)
    try:
        fd = os.open(path, os.O_RDONLY | getattr(os, 'O_DIRECTORY', 0))
    except OSError as exc:
        return SYNC_UNSUPPORTED if exc.errno in unverified else SYNC_FAILED
    outcome = SYNC_OK
    try:
        os.fsync(fd)
    except OSError as exc:
        outcome = SYNC_UNSUPPORTED if exc.errno in unverified else SYNC_FAILED
    finally:
        try:
            os.close(fd)
        except OSError:
            outcome = SYNC_FAILED
    return outcome


def _make_dirs(path):
    """Return new directories in parent-first order; caller syncs each new entry's parent."""
    missing, cursor = [], os.path.abspath(path)
    while not os.path.isdir(cursor):
        missing.append(cursor)
        parent = os.path.dirname(cursor)
        if parent == cursor:
            raise OSError('no existing ancestor for '+path)
        cursor = parent
    created = []
    for d in reversed(missing):
        try:
            os.mkdir(d)
        except FileExistsError:
            if not os.path.isdir(d):
                raise
        else:
            created.append(d)
    return created


def _write_atomic(path, obj, prior=_AUTO_PRIOR, created_dirs=()):
    """One commit point; preimages are retained, never an automatic recovery promise."""
    trail, leftovers = [], []
    committed = False
    backup = None
    def sync(d):
        outcome = _sync_dir(d)
        trail.append((d, outcome))
    def temp_bytes(d, prefix, body):
        fd, tmp = tempfile.mkstemp(dir=d, prefix=prefix)
        leftovers.append(tmp)
        with os.fdopen(fd, 'wb') as f:
            f.write(body)
            f.flush()
            os.fsync(f.fileno())
        return tmp
    try:
        if prior is _AUTO_PRIOR:
            try:
                with open(path, 'rb') as f: prior = f.read()
            except FileNotFoundError:
                prior = None
        d = os.path.dirname(os.path.abspath(path))
        # Only this call's new directory entries need parent syncs. Pre-existing
        # ancestors may be search-only and are not prerequisites for publication.
        created = list(created_dirs) + _make_dirs(d)
        if prior is not None:
            backup_dir = os.path.join(d, '.preimage')
            created.extend(_make_dirs(backup_dir))
            stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())
            base = os.path.join(backup_dir, '%s.%s-%d' % (os.path.basename(path), stamp, os.getpid()))
            backup = None
            for n in range(1000):
                candidate = base + (('-'+str(n)) if n else '')
                try:
                    # Reserve exclusively: never replace someone else's preimage.
                    fd = os.open(candidate, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
                    os.close(fd)
                    backup = candidate
                    break
                except FileExistsError:
                    continue
            if backup is None:
                raise OSError('preimage name space exhausted')
            tmp = temp_bytes(backup_dir, '.preimage-', prior)
            os.replace(tmp, backup)
            leftovers.remove(tmp)
            with open(backup, 'rb') as f:
                if _digest(f.read()) != _digest(prior):
                    raise OSError('preimage mismatch; ledger untouched')
        body = (json.dumps(obj, ensure_ascii=False, indent=1)+'\n').encode('utf-8')
        tmp = temp_bytes(d, '.intake-', body)
        os.replace(tmp, path)
        leftovers.remove(tmp)
        committed = True
        # Directory durability is an outcome of the committed write, never rc11.
        targets = {d} | {os.path.dirname(x) for x in created}
        for target in sorted(targets):
            sync(target)
    except (OSError, ValueError, TypeError) as exc:
        rc = EXIT_COMMITTED_UNCONFIRMED if committed else EXIT_NOT_COMMITTED
        print('%s: %s; owned temporary paths=%s; preimage=%s; sync=%s' % (_tag(rc), exc, leftovers, backup, trail), file=sys.stderr)
        return rc
    if any(outcome == SYNC_FAILED for _, outcome in trail):
        print('COMMITTED/DURABILITY_UNCONFIRMED; sync=%s' % trail, file=sys.stderr)
        return EXIT_COMMITTED_UNCONFIRMED
    if any(outcome == SYNC_NONPOSIX for _, outcome in trail):
        print('COMMITTED/DURABILITY_UNVERIFIED (platform has no directory sync)', file=sys.stderr)
        return EXIT_PAID
    if any(outcome != SYNC_OK for _, outcome in trail):
        print('COMMITTED/DURABILITY_UNVERIFIED; sync=%s' % trail, file=sys.stderr)
        return EXIT_COMMITTED_UNVERIFIED
    return EXIT_PAID


def _lock_module():
    # -I omits the script directory and ignores the bytecode environment setting.
    # Seal before explicitly loading the sibling, including callers without -B.
    sys.dont_write_bytecode = True
    spec = importlib.util.spec_from_file_location('intake_writer_lock', os.path.join(os.path.dirname(__file__), 'javis_lock.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _writer(fn):
    @functools.wraps(fn)
    def call(a):
        if fn.__name__ == 'cmd_open' and not ID_RE.match(a.id or ''):
            print('USAGE invalid ledger id', file=sys.stderr)
            return EXIT_USAGE
        path = _path(_root(a.root), a.id)
        lk = None
        locked = False
        try:
            # Administrative lock metadata may be written; rejected payments never
            # write the ledger, its preimage, or a candidate temporary file.
            a._created_dirs = _make_dirs(os.path.dirname(path))
            try:
                locks = _lock_module()
            except (ImportError, OSError) as exc:
                print('USAGE writer lock unavailable: %s' % exc, file=sys.stderr)
                return EXIT_USAGE
            lk = locks.FileLock(path+'.write-lock', owner='intake:'+fn.__name__, blocking=False)
            result = lk.acquire()
            if result != locks.ACQUIRED:
                print('USAGE writer lock %s: %s holder=%s' % (result, lk.detail, lk.blocked_by), file=sys.stderr)
                return EXIT_USAGE
            locked = True
            try:
                with open(path, 'rb') as f: a._prior = f.read()
            except FileNotFoundError:
                a._prior = None
            if a._prior is not None and fn.__name__ != 'cmd_open':
                a._loaded = json.loads(a._prior.decode('utf-8'))
                if (not isinstance(a._loaded, dict) or not isinstance(a._loaded.get('items'), list)
                        or any(not isinstance(it, dict) for it in a._loaded['items'])):
                    raise ValueError('ledger object/items malformed')
            else:
                a._loaded = None
            return fn(a)
        except (OSError, ValueError, UnicodeError) as exc:
            rc = EXIT_DRIFT if locked else EXIT_USAGE
            print('%s ledger/lock read failed: %s' % (_tag(rc), exc), file=sys.stderr)
            return rc
        finally:
            if lk is not None:
                lk.release()
    return call


def _commit(a, path, led):
    return _write_atomic(path, led, prior=a._prior, created_dirs=a._created_dirs)


def _append_event(led, index, event):
    out = copy.deepcopy(led)
    item = out['items'][index]
    legacy = 'payments' not in item
    if legacy:
        item['payments'] = []
    if legacy or any(k in item for k in _MIRROR_KEYS):
        payload = {k: copy.deepcopy(v) for k, v in item.items() if k != 'payments'}
        item['payments'].append(dict(kind='legacy_snapshot', payload=payload, at=_now(),
                                    provenance='v1 fields, acceptance unproven'))
        for k in _MIRROR_KEYS:
            item.pop(k, None)
    item['payments'].append(event)
    out['schema'] = SCHEMA
    return out


def _load(path):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


_EPHEMERAL_ROOTS = ("/tmp/", "/private/tmp/", "/var/tmp/", "/private/var/folders/",
                    "/var/folders/")
# ★Windows 임시 루트(2026-10-20 수리 · B-328 #8 제품 승격 리뷰). 위 목록은 POSIX 경로뿐이라 Windows 에서는
#   %TEMP% 의 artifact 가 내구로 읽혀 **거짓 PAID** 가 났고 selftest ⒡' 가 6 을 냈다. 값은 env 에서 해소한다 —
#   TMPDIR·TEMP·TMP 는 파이썬 tempfile 이 같은 순서로 보는 이름이고, LOCALAPPDATA\Temp 는 TEMP 가 8.3 짧은
#   이름(RUNNER~1)일 때의 긴 이름 형태다. ⛔env 가 비었을 때 tempfile 이 쓰는 고정 폴백(c:\temp 등)은 담지 않았다.
_WIN_TEMP_ENV = ("TMPDIR", "TEMP", "TMP")


_OPEN = open
_STAT = os.stat
# ★`_OPEN`·`_STAT` 은 검체 주입구다 — 읽기와 stat 갈래를 실제 dataless 파일 없이 재기 위해 테스트가 대역으로
#   바꾼다(운영 경로는 builtin open·os.stat). 같은 수법을 Windows 경로 판정에서 `os` 대역으로 이미 쓴다.
#   ⛔이 도구는 파일을 **내려받거나 실체화하거나 옮기지 않는다**(오너 규칙) — stat 과 read 만 한다.

SF_DATALESS = 0x40000000      # macOS sys/stat.h — 내용이 클라우드로 비워졌다(stat 는 되고 read 는 막힌다)
UF_COMPRESSED = 0x20          # macOS — 압축 저장(APFS/HFS+ 압축)

PROBE_OK, PROBE_ABSENT, PROBE_UNREADABLE, PROBE_NOT_REGULAR = "OK", "ABSENT", "UNREADABLE", "NOT_REGULAR"


def _flags_note(flags, plat=None):
    """st_flags 를 사람 말로 — ★비트의 **뜻**은 darwin 에서만 붙인다.

    다른 OS 에서 `st_flags` 가 있어도 그 비트는 같은 뜻이 아니다 ⇒ 숫자만 찍는다(U5: 그 OS 는 errno 로 분류한다).
    `plat` 은 다른 OS 에서 darwin 판정을 재기 위한 주입구이고, 운영 호출은 생략한다.
    """
    if flags is None:
        return ""
    if (plat or sys.platform) != "darwin":
        return " · st_flags=0x%x" % flags
    names = [n for bit, n in ((SF_DATALESS, "dataless"), (UF_COMPRESSED, "compressed")) if flags & bit]
    return " · st_flags=0x%x(%s)" % (flags, ",".join(names) if names else "dataless 아님")


def _probe(path):
    """(kind, sha256_16, size, errno, st_flags) — 읽기 재확인의 **네 갈래**.

    ★왜 네 갈래인가(CEO 실측 2026-10-23): 종전 `_fp` 는 어떤 실패든 (None, None) 으로 삼켰고 재확인은 그것을
    「지문 불일치 = 변경」으로 읽었다 ⇒ ★**읽지 못한 것**과 **바뀐 것**이 같은 출력·같은 처방을 받았다. 둘은 다른
    사실이고 다른 처방을 부른다(변경 → 재지불 · 부재 → 재지불/보류 · 판독 불가 → ⛔재지불 금지).
      OK          : 열고 끝까지 읽었다 — 지문이 있다(신원을 말할 수 있는 유일한 갈래).
      ABSENT      : 경로가 없다.
      NOT_REGULAR : 있으나 정규 파일이 아니다(디렉터리·FIFO·장치) — 열지 않는다(FIFO 는 쓰는 쪽을 기다려 막힌다).
      UNREADABLE  : darwin SF_DATALESS 면 열지 않고 errno=None; 그 밖의 open/read OSError 는 실제 errno 를 싣는다.
                    st_flags 를 보존하고 크기는 stat 에서
                    얻는다(★stat 는 dataless 파일에서도 **논리 크기**를 준다 — 그래서 크기 대조는 정황이 된다).
    ⛔OSError 가 아닌 예외(경로가 문자열이 아닌 기록 등)는 여기서 분류하지 않는다 — 호출자가 종전처럼 지문 없음으로 본다.
    """
    try:
        stt = _STAT(path)
    except FileNotFoundError:
        return PROBE_ABSENT, None, None, None, None
    except OSError as e:
        # stat 자체가 막혔다 — 크기조차 모르는 판독 불가다.
        return PROBE_UNREADABLE, None, None, e.errno, None
    flags = getattr(stt, "st_flags", None)
    if not stat.S_ISREG(stt.st_mode):
        return PROBE_NOT_REGULAR, None, None, None, flags
    if sys.platform == "darwin" and flags is not None and flags & SF_DATALESS:
        # A failed open can still request hydration. Observe the flag without requesting bytes.
        return PROBE_UNREADABLE, None, stt.st_size, None, flags
    try:
        with _OPEN(path, "rb") as fh:
            b = fh.read()
    except OSError as e:
        return PROBE_UNREADABLE, None, stt.st_size, e.errno, flags
    return PROBE_OK, hashlib.sha256(b).hexdigest()[:16], len(b), None, flags


def _unreadable_note(eno, cur_size, rec_size, flags):
    """판독 불가 한 건을 사람 말로 — errno(이름) · st_flags · 크기 대조. ★크기 일치는 **정황**이고 신원이 아니다."""
    name = errno.errorcode.get(eno, "?") if eno is not None else "?"
    if (sys.platform == "darwin" and eno is None and cur_size is not None
            and flags is not None and flags & SF_DATALESS):
        out = "⛔열지 않았다 — stat 에서 SF_DATALESS 를 보고 바이트를 요청하지 않았다%s" % _flags_note(flags)
    else:
        out = "errno %s(%s)%s" % (eno, name, _flags_note(flags))
    if cur_size is None:
        out += " · 현재 크기 미측정(stat 도 막혔다)"
    elif rec_size is None:
        out += " · 현재 크기 %sB(기록된 크기가 없다)" % cur_size
    elif int(cur_size) == int(rec_size):
        out += " · 현재 크기 %sB = 기록과 일치(내용이 온전할 정황)" % cur_size
    else:
        out += " · 현재 크기 %sB 가 기록 %sB 와 ★다르다(변경 정황 — 읽지 못했으므로 sha 로 확정하지 못했다)" % (
            cur_size, rec_size)
    return out


def _fp(path):
    """(sha256_16, size) — 지불 시점의 지문. ★dept-6 이 2026-10-15 에 찾은 결함의 수리.

    종전 이 도구는 artifact 를 **맨 경로 문자열**로만 적고 `os.path.exists` 로 stat 했다 ⇒
    ★그 파일이 그 뒤 **덧붙여지거나 덮어써져도** 항목은 계속 `PAID` 로 읽혔고, 비교할 기준값이
    없으므로 ★「변경되지 않았다」와 「변경을 판정할 수 없다」를 **구별할 수 없었다.**
    (`javis_task.py` 는 `sha256_8`·`size`·`mtime` 을 적는다 — 같은 팩 안에서 갈렸다.)
    ⇒ 지금은 지불 시점에 지문을 박고 `status` 가 매번 재계산해 대조한다. ⛔덧붙임도 변경이다.

    ★정규 파일이 아니면 열지 않고 (None, None) 이다(2026-10-20 · B-328 #8 리뷰 R1) — FIFO 는 열면 쓰는 쪽이 올
    때까지 막히고 장치 파일은 끝없이 읽힌다. 확인과 열기 사이에 바뀌는 경로는 막지 못한다(어휘·시점의 한계).

    ★2026-10-23(WP10): 몸통을 `_probe` 에 위임한다 — 읽기 경로가 **하나**여야 지불 시점과 재확인이 같은
    사실을 본다. 계약은 그대로다: OK 가 아니면 (None, None) 이고, 그래서 **지불 시점에** 읽히지 않는 artifact 는
    종전처럼 지불을 싣지 못한다(그 문을 이 수리가 열지 않는다 — 열린 것은 재확인 쪽의 **구별**뿐이다).
    """
    try:
        kind, sha, size, _eno, _flags = _probe(path)
    except Exception:
        return None, None
    if kind != PROBE_OK:
        return None, None
    return sha, size


def _ephemeral(path, os_name=None, env=None):
    """★임시 경로인가 — 지불이 ★사라질 수 있는 자리.

    2026-10-15 측정: CEO 자신이 전파문 사본을 스크래치패드 경로로 지불 증거에 걸었다. 내구 경로가
    함께 있어 살아남았지만, ★임시 경로**만**으로 지불된 항목은 그 디렉터리가 청소되면 조용히 다시
    열린다 — dept-1 이 실측한 「이동은 지불을 푼다」와 같은 기제이고 이쪽은 **아무도 옮기지 않아도**
    일어난다. ⇒ 임시 경로만인 지불은 **지불로 세지 않는다**(내구 1건이라도 있으면 지불이다).

    ★Windows(os.name == "nt")는 `_win_temp_roots` 와 대소문자 무시로 대조한다 — POSIX 목록은 Windows 의
    abspath(드라이브 문자가 붙는다)와 맞을 수 없어 그 OS 에서는 원래 죽은 검사였다. `os_name`·`env` 는
    다른 OS 에서 Windows 판정을 재기 위한 주입구이고, 운영 호출은 둘 다 생략한다.
    """
    if (os_name or os.name) == "nt":
        pp = _nt_norm(path) + "\\"
        return any(pp.startswith(r) for r in _win_temp_roots(os.environ if env is None else env))
    pp = os.path.abspath(path)
    return any(pp.startswith(r) for r in _EPHEMERAL_ROOTS)


_NT_DRIVE_RE = re.compile(r"^[A-Za-z]:(\\|$)")


def _nt_norm(p):
    """Windows 경로의 비교 형태 — 절대·소문자·역슬래시. ★DOS 장치 경로 접두를 먼저 벗긴다(2026-10-20 · B-328 #8
    리뷰 R1): `\\\\?\\X:\\`·`\\\\.\\X:\\` 는 `X:\\` 로, `\\\\?\\UNC\\`·`\\\\.\\UNC\\` 는 `\\\\` 로 — 같은 파일의 다른 이름이라
    벗기지 않으면 어휘 비교가 빗나가 %TEMP% 파일이 내구로 읽힌다. 벗긴 뒤 abspath 하므로 `..` 처리가 파이썬 판마다
    같다(3.9 의 normpath 는 장치 경로를 손대지 않는다). ⛔드라이브·UNC 가 아닌 장치 이름(`\\\\?\\Volume{…}\\` 등)은
    어휘로 풀 수 없어 벗기지 않는다 — 그 형태의 임시 파일은 여전히 내구로 읽힌다(밝혀 둔 한계)."""
    s = p.replace("/", "\\")
    if s[:4] in ("\\\\?\\", "\\\\.\\"):
        rest = s[4:]
        if rest[:4].upper() == "UNC\\":
            s = "\\\\" + rest[4:]
        elif _NT_DRIVE_RE.match(rest):
            s = rest
    return ntpath.normcase(ntpath.abspath(s))


def _win_temp_roots(env):
    """Windows 임시 루트 접두 — 값이 있는 TMPDIR·TEMP·TMP 와 LOCALAPPDATA\\Temp 를 `_nt_norm` 으로 정규화하고
    끝에 구분자 하나를 붙인다(`C:\\Temp` 가 `C:\\Temp2` 를 덮지 않게)."""
    roots = [env.get(k) for k in _WIN_TEMP_ENV]
    if env.get("LOCALAPPDATA"):
        roots.append(ntpath.join(env["LOCALAPPDATA"], "Temp"))
    out = []
    for r in roots:
        if r:
            r = _nt_norm(r)
            out.append(r if r.endswith("\\") else r + "\\")
    return out


_MISSING = object()
_MIRROR_KEYS = ('state', 'evidence', 'artifacts', 'paid_at', 'defer_reason',
                'release_condition', 'deferred_at')


def _resolvable(path):
    """Reject foreign/ambiguous namespaces before stat, never resolve a legacy relative path."""
    p = path.replace('/', '\\')
    device = p.startswith(('\\\\?\\', '\\\\.\\'))
    drive = bool(re.match(r'^[A-Za-z]:\\', p))
    if os.name == 'nt':
        if device:
            rest = p[4:]
            return bool(re.match(r'^[A-Za-z]:\\', rest) or rest.upper().startswith('UNC\\'))
        return drive or p.startswith('\\\\')
    return not (device or drive or p.startswith('\\\\') or path.startswith('//')) and path.startswith('/')


def _artifact_valid(a, legacy):
    if isinstance(a, str):
        return legacy and bool(a.strip())
    if not isinstance(a, dict) or not isinstance(a.get('path'), str) or not a['path'].strip():
        return False
    sha, size = a.get('sha256_16'), a.get('size')
    if sha is not None and (not isinstance(sha, str) or re.fullmatch(r'[0-9a-fA-F]{16}', sha) is None):
        return False
    if size is not None and (type(size) is not int or size < 0):
        return False
    return legacy or (sha is not None and size is not None and type(a.get('ephemeral')) is bool)


def _arts(it, legacy=True):
    """Six-field projection; absent ephemeral stays distinct from null."""
    return [(a, None, None, True, _MISSING, legacy) if isinstance(a, str) else
            (a.get('path'), a.get('sha256_16'), a.get('size'), False,
             a.get('ephemeral', _MISSING), legacy) for a in it.get('artifacts', [])]


def _entry_valid(e):
    if not isinstance(e, dict):
        return False
    def text(k):
        return isinstance(e.get(k), str) and bool(e[k].strip())
    if e.get('kind') == 'legacy_snapshot':
        return isinstance(e.get('payload'), dict) and text('at') and text('provenance')
    if e.get('kind') == 'pay':
        return (e.get('state') == 'done' and isinstance(e.get('evidence'), str)
                and isinstance(e.get('artifacts'), list) and text('paid_at')
                and all(_artifact_valid(a, False) for a in e['artifacts']))
    if e.get('kind') == 'defer':
        return (e.get('state') == 'defer' and isinstance(e.get('defer_reason'), str)
                and isinstance(e.get('release_condition'), str) and text('deferred_at'))
    return False


def _journal_state(entries):
    if not isinstance(entries, list) or any(not _entry_valid(e) for e in entries):
        return 'DRIFT', 'malformed payments entry; no fallback'
    if not entries or entries[-1]['kind'] == 'legacy_snapshot':
        return 'OPEN', 'no proven settlement after legacy_snapshot'
    return _settlement_state(entries[-1], legacy=False)


def _item_state(it, root_for_rel=None):
    if not isinstance(it, dict):
        return 'DRIFT', 'item is not an object'
    if 'payments' not in it:
        return _settlement_state(it, legacy=True)
    entries = it['payments']
    if not isinstance(entries, list):
        return 'DRIFT', 'payments is not an array'
    journal_state, journal_why = _journal_state(entries)
    if journal_state == 'DRIFT':
        return journal_state, journal_why
    if any(k in it for k in _MIRROR_KEYS):
        mirror = {k: v for k, v in it.items() if k != 'payments'}
        current, why = _settlement_state(mirror, legacy=True)
        shape = (mirror.get('state') == 'done' and isinstance(mirror.get('evidence'), str)
                 and len(mirror['evidence'].strip()) >= MIN_EVIDENCE_CHARS
                 and isinstance(mirror.get('artifacts'), list)
                 and any(isinstance(a, dict) and _artifact_valid(a, True)
                         and a.get('sha256_16') and not _ephemeral(a['path']) for a in mirror['artifacts']))
        if mirror.get('state') == 'defer':
            shape = (isinstance(mirror.get('defer_reason'), str) and len(mirror['defer_reason'].strip()) >= MIN_REASON_CHARS
                     and isinstance(mirror.get('release_condition'), str) and bool(mirror['release_condition'].strip()))
        detail = dict(legacy=mirror, legacy_write_shape='accepted shape' if shape else 'not an accepted shape',
                      acceptance='unproven', legacy_current=dict(state=current, why=why),
                      journal=entries[-1] if entries else None,
                      journal_current=dict(state=journal_state, why=journal_why))
        return 'CONFLICT', json.dumps(detail, ensure_ascii=False, sort_keys=True)
    return journal_state, journal_why


def _settlement_state(it, legacy=True):
    st = it.get('state', 'open')
    if not isinstance(st, str):
        return 'DRIFT', 'state must be a string'
    if st == 'defer':
        rs, rc = it.get('defer_reason', ''), it.get('release_condition', '')
        if not isinstance(rs, str) or not isinstance(rc, str):
            return 'DRIFT', 'defer fields must be strings'
        if len(rs.strip()) < MIN_REASON_CHARS:
            return 'OPEN', 'defer_reason %d자 < 최소 %d자' % (len(rs.strip()), MIN_REASON_CHARS)
        if not rc.strip():
            return 'OPEN', 'release_condition 공백 — 항목마다 무엇이 막는지 대야 한다'
        return 'PAID', 'defer: 사유 %d자 + release_condition 선언' % len(rs.strip())
    if st != 'done':
        return 'OPEN', 'state=%s (지불 없음)' % st
    ev = it.get('evidence', '')
    if not isinstance(ev, str):
        return 'DRIFT', 'evidence must be a string'
    records = it.get('artifacts', [])
    if not isinstance(records, list) or any(not _artifact_valid(a, legacy) for a in records):
        return 'DRIFT', 'malformed artifact record'
    if len(ev.strip()) < MIN_EVIDENCE_CHARS:
        return 'OPEN', 'evidence %d자 < 최소 %d자' % (len(ev.strip()), MIN_EVIDENCE_CHARS)
    arts = _arts(it, legacy)
    classes, unreadable, drifted, vanished = [], [], [], []
    for path, rec_sha, rec_size, bare, ephemeral, old in arts:
        if not _resolvable(path):
            classes.append('UNRESOLVABLE'); continue
        eph = _ephemeral(path) if old else ephemeral
        if not bare and not rec_sha:
            classes.append('UNPROVEN'); continue
        try:
            if bare:
                stt = _STAT(path)
                kind = PROBE_OK if stat.S_ISREG(stt.st_mode) else PROBE_NOT_REGULAR
                cur_sha, cur_size, eno, flags = None, stt.st_size, None, getattr(stt, 'st_flags', None)
            else:
                kind, cur_sha, cur_size, eno, flags = _probe(path)
        except FileNotFoundError:
            kind, cur_sha, cur_size, eno, flags = PROBE_ABSENT, None, None, None, None
        except OSError as exc:
            kind, cur_sha, cur_size, eno, flags = PROBE_UNREADABLE, None, None, exc.errno, None
        except Exception:
            kind, cur_sha, cur_size, eno, flags = PROBE_NOT_REGULAR, None, None, None, None
        if kind == PROBE_UNREADABLE:
            classes.append('STAT_DENIED' if bare else 'UNREADABLE')
            unreadable.append((path, rec_sha, rec_size, cur_size, eno, flags))
        elif kind == PROBE_ABSENT:
            cl = 'EPHEMERAL_EXPIRED' if not old and rec_sha and ephemeral is True else 'LEGACY_TEMP_GONE' if old and eph else 'ABSENT'
            classes.append(cl)
            if cl != 'EPHEMERAL_EXPIRED': vanished.append((path, rec_sha, rec_size))
        elif kind == PROBE_NOT_REGULAR or (not bare and cur_sha != rec_sha):
            classes.append('CHANGED' if kind == PROBE_OK else 'NOT_REGULAR')
            drifted.append((path, rec_sha, cur_sha, rec_size, cur_size))
        else:
            classes.append(('EXEMPT_' if bare else 'VERIFIED_') + ('EPHEMERAL' if eph else 'DURABLE'))
    if not arts:
        return 'OPEN', 'artifacts 0건 — 경로를 적는 것으로는 지불이 아니다'
    if drifted:
        p0, rs, cs, rz, cz = drifted[0]
        return 'OPEN', ('★증거 바이트가 변경됐다(%d건) — %s: 기록 %s/%sB → 현재 %s/%sB. '
                        '⛔덧붙임도 변경이다 — 그 절만 떠낸 새 파일로 재지불하라%s' %
                       (len(drifted), os.path.basename(p0), rs, rz, cs, cz,
                        (' · (그 밖에 판독 불가 %d건 — 그쪽은 재지불 대상이 아니다)' % len(unreadable)) if unreadable else ''))
    if vanished:
        return 'OPEN', '★지불 뒤 artifact 가 사라졌다(%d건) — 디스크 부재 (⛔변경이 아니다 — 경로 부재다)' % len(vanished)
    if 'UNPROVEN' in classes:
        return 'OPEN', '지불 시점에 지문을 얻지 못했다 — 디스크 부재·디렉터리·읽기 불가·FIFO는 지불이 아니다'
    if 'UNRESOLVABLE' in classes:
        return 'UNRESOLVABLE', 'foreign or relative artifact namespace; no I/O'
    if unreadable:
        p0, rs, rz, cz, eno, flags = unreadable[0]
        return 'UNREADABLE', ('★판독 불가 %d건 — %s: %s. 기록 지문 %s. ⛔지불은 무효가 아니다 — 읽을 수 '
                             '있는 sha 없이는 신원이 증명되지 않을 뿐이고, 새 증거를 지어내 메울 일이 '
                             '아니다(이 줄은 어떤 처방도 요구하지 않는다).' %
                             (len(unreadable), os.path.basename(p0), _unreadable_note(eno, cz, rz, flags), rs))
    if 'EPHEMERAL_EXPIRED' in classes and 'VERIFIED_DURABLE' not in classes:
        return 'OPEN', 'expired ephemeral proof requires verified durable proof'
    if not any(c.endswith('_DURABLE') for c in classes):
        return 'OPEN', 'artifacts 전부 ★임시 경로다 — 내구 경로 1건 이상을 대라'
    counts = ', '.join('%s=%d' % (c, classes.count(c)) for c in sorted(set(classes)))
    if 'VERIFIED_DURABLE' in classes:
        return 'PAID', 'evidence %d자 + artifact 지문 대조 · %s' % (len(ev.strip()), counts)
    return 'EXEMPT', '지문 없음 — 신원 미확립(소급 검증 불가) · '+counts


def _evaluate(led):
    """(exit, lines) — 순수 함수. ★파일 I/O 는 artifact 실존 확인뿐이다."""
    lines = []
    instr = led.get("instruction") or {}
    text = instr.get("text")
    if text is None:
        return EXIT_NO_DENOM, ["NO_DENOM  지시 원문이 기록되지 않았다"]
    recorded = instr.get("sha256")
    actual = _digest(text.encode("utf-8"))
    if recorded and recorded != actual:
        return EXIT_DRIFT, ["DRIFT  지시 원문 다이제스트 불일치 — 기록 %s / 재계산 %s"
                            % (recorded[:16], actual[:16])]
    items = led.get("items") or []
    if not items:
        return EXIT_NO_DENOM, ["NO_DENOM  지시는 기록됐으나 항목 목록이 0건 — 분모가 없다"
                               " (★통과가 아니다: 검사 불가능이다)"]
    buckets = {name: [] for name in ('PAID', 'OPEN', 'UNREADABLE', 'UNRESOLVABLE', 'EXEMPT', 'CONFLICT', 'DRIFT')}
    codes = {name: code for code, name in _TAG.items()}
    worst = EXIT_PAID
    for i, it in enumerate(items, 1):
        state, why = _item_state(it)
        text = it.get('text', '') if isinstance(it, dict) else ''
        buckets[state].append("  %-5s #%-3d %s  [%s]" % (state, i, str(text)[:72], why))
        worst = _worse(worst, codes[state])
    lines.append("접수 %d · 지불 %d · 열림 %d · 판독 불가 %d · 면제 %d · 충돌 %d  (대장 %s · 지시 sha %s)" %
                 (len(items), len(buckets['PAID'])+len(buckets['UNREADABLE']), len(buckets['OPEN']),
                  len(buckets['UNREADABLE']),len(buckets['EXEMPT']),len(buckets['CONFLICT']),led.get('id'),actual[:16]))
    for state in ('DRIFT','OPEN','UNRESOLVABLE','CONFLICT','UNREADABLE','EXEMPT','PAID'):
        lines.extend(buckets[state])
    return worst, lines


@_writer
def cmd_open(a):
    root = _root(a.root)
    if not ID_RE.match(a.id or ""):
        print("USAGE  --id 는 [A-Za-z0-9._:-] 1~64자", file=sys.stderr)
        return EXIT_USAGE
    if not a.instruction_file or not os.path.exists(a.instruction_file):
        print("USAGE  --instruction-file 이 없다 — ★지시 원문을 기록하지 않고 대장을 열 수 없다",
              file=sys.stderr)
        return EXIT_USAGE
    with open(a.instruction_file, "rb") as fh:
        raw = fh.read()
    text = raw.decode("utf-8", "replace")
    p = _path(root, a.id)
    if os.path.exists(p):
        print("USAGE  대장 %s 가 이미 있다 — 덮지 않는다(add-item/pay/defer 를 쓰라)" % a.id,
              file=sys.stderr)
        return EXIT_USAGE
    led = {
        "schema": SCHEMA,
        "id": a.id,
        "source": a.source,
        "opened_at": _now(),
        "opened_by": {"role": _seat()[0], "surface": _seat()[1],
                      "pack": root},
        "instruction": {"text": text, "sha256": _digest(text.encode("utf-8")),
                        "bytes": len(raw), "from_file": os.path.abspath(a.instruction_file)},
        "items": [{"text": t, "state": "open", "added_at": _now()} for t in (a.item or [])],
    }
    commit_rc = _commit(a, p, led)
    if commit_rc:
        return commit_rc
    rc, lines = _evaluate(led)
    print("OPENED %s  항목 %d건  →  %s" % (p, len(led["items"]), _tag(rc)))
    for l in lines:
        print(l)
    return rc


@_writer
def cmd_add_item(a):
    root = _root(a.root)
    p = _path(root, a.id)
    if not os.path.exists(p):
        print("NO_LEDGER  %s 없다" % p, file=sys.stderr)
        return EXIT_NO_LEDGER
    led = copy.deepcopy(a._loaded)
    led.setdefault("items", []).append({"text": a.item, "state": "open", "added_at": _now()})
    commit_rc = _commit(a, p, led)
    if commit_rc:
        return commit_rc
    print("ADDED #%d  %s" % (len(led["items"]), a.item[:72]))
    return EXIT_PAID


def _get(a):
    root = _root(a.root)
    p = _path(root, a.id)
    if not os.path.exists(p):
        print("NO_LEDGER  %s 없다" % p, file=sys.stderr)
        return None, None, EXIT_NO_LEDGER
    led = copy.deepcopy(a._loaded)
    n = a.item_no
    if not (1 <= n <= len(led.get("items") or [])):
        print("USAGE  --item-no %d 는 1..%d 범위 밖" % (n, len(led.get("items") or [])), file=sys.stderr)
        return None, None, EXIT_USAGE
    return led, p, None


def _settle(a, event):
    led, p, err = _get(a)
    if err is not None:
        return err
    item = led['items'][a.item_no - 1]
    if 'payments' in item and _journal_state(item['payments'])[0] == 'DRIFT':
        print('DRIFT malformed existing journal', file=sys.stderr)
        return EXIT_DRIFT
    # A failed fingerprint is not a malformed persisted journal: it is a refused candidate.
    if event['kind'] == 'pay' and any(not _artifact_valid(r, False) for r in event['artifacts']):
        print('REFUSED 지불 시점에 지문을 얻지 못했다 — 디스크 부재·디렉터리·읽기 불가·FIFO', file=sys.stderr)
        return EXIT_OPEN
    state, why = _settlement_state(event, legacy=False)
    if state not in ('PAID', 'UNREADABLE'):
        print('REFUSED #%d — %s' % (a.item_no, why), file=sys.stderr)
        return EXIT_OPEN
    candidate = _append_event(led, a.item_no-1, event)
    rc = _commit(a, p, candidate)
    if rc:
        return rc
    print('%s #%d %s' % ('PAID' if event['kind']=='pay' else 'DEFERRED', a.item_no, why))
    return EXIT_PAID


@_writer
def cmd_pay(a):
    recs = []
    for path in (a.artifact or []):
        path = os.path.abspath(path)
        sha, size = _fp(path) if _resolvable(path) else (None, None)
        recs.append(dict(path=path, sha256_16=sha, size=size, ephemeral=_ephemeral(path)))
    return _settle(a, dict(kind='pay', state='done', evidence=a.evidence,
                           artifacts=recs, paid_at=_now()))


@_writer
def cmd_defer(a):
    return _settle(a, dict(kind='defer', state='defer', defer_reason=a.reason,
                           release_condition=a.release_condition, deferred_at=_now()))


@_writer
def cmd_stamp(a):
    """귀속 없는 대장에 이 좌석을 기입한다. ★다른 좌석의 대장을 가져가지 못하게 --id 를 요구한다."""
    root = _root(a.root)
    p = _path(root, a.id)
    if not os.path.exists(p):
        print("NO_LEDGER  %s 없다" % p, file=sys.stderr)
        return EXIT_NO_LEDGER
    led = copy.deepcopy(a._loaded)
    ob = led.get("opened_by")
    if isinstance(ob, dict) and (ob.get("surface") or ob.get("role")):
        print("USAGE  이미 귀속돼 있다(role=%s surface=%s) — 덮지 않는다"
              % (ob.get("role"), ob.get("surface")), file=sys.stderr)
        return EXIT_USAGE
    role, sid, _a = _seat()
    if not (role or sid):
        print("USAGE  CYS_ROLE·CYS_SURFACE_ID 가 둘 다 없다 — 좌석을 지어내지 않는다", file=sys.stderr)
        return EXIT_USAGE
    led["opened_by"] = {"role": role, "surface": sid, "pack": root, "stamped_at": _now()}
    commit_rc = _commit(a, p, led)
    if commit_rc:
        return commit_rc
    print("STAMPED  %s  role=%s surface=%s" % (a.id, role, sid))
    return EXIT_PAID


def _ledgers(root):
    d = _dir(root)
    if not os.path.isdir(d):
        return []
    return sorted(os.path.join(d, f) for f in os.listdir(d) if f.endswith(".json"))


def cmd_status(a):
    root = _root(a.root)
    paths = [_path(root, a.id)] if a.id else _ledgers(root)
    paths = [p for p in paths if os.path.exists(p)]
    if not paths:
        print("NO_LEDGER  %s 에 접수 기록이 없다 — ★빈 대장은 「열린 것 없음」이 아니다" % _dir(root))
        return EXIT_NO_LEDGER
    worst = EXIT_PAID
    out = []
    judged = 0
    for p in paths:
        try:
            led = _load(p)
        except Exception as e:
            out.append("DRIFT  %s 판독 불가: %s" % (p, e))
            worst = _worse(worst, EXIT_DRIFT)
            continue
        mine = _mine(led)
        if not a.id and not a.all:
            if mine is None:
                out.append("UNATTRIBUTED  %s — 좌석 성분 없음. `stamp --id %s` 로 귀속을 기입하라"
                           % (p, led.get("id")))
                worst = _worse(worst, EXIT_UNATTRIB)
                continue
            if mine is False:
                ob = led.get("opened_by") or {}
                out.append("OTHER_SEAT    %s (role=%s surface=%s) — 판정에서 제외"
                           % (os.path.basename(p), ob.get("role"), ob.get("surface")))
                continue
        rc, lines = _evaluate(led)
        judged += 1
        out.append("[%s] %s" % (_tag(rc), p))
        out.extend("  " + l for l in lines)
        worst = _worse(worst, rc)
    if not a.id and not a.all and judged == 0 and worst == EXIT_PAID:
        out.append("NO_LEDGER  이 좌석에 귀속된 대장이 0건이다 — ★다른 좌석의 지불로 통과하지 않는다")
        worst = EXIT_NO_LEDGER
    if a.json:
        print(json.dumps({"root": root, "worst_exit": worst, "ledgers": len(paths),
                          "judged": judged, "details": out}, ensure_ascii=False))
    else:
        print("\n".join(out))
    return worst


def cmd_audit(a):
    """CEO 제2검사자 — 레인 밖에서 읽는다. ★산출자는 자기 산출물의 종결을 판정하지 않는다."""
    roots = [r.strip() for r in (a.roots or "").split(",") if r.strip()]
    if not roots:
        print("USAGE  --roots 가 비었다", file=sys.stderr)
        return EXIT_USAGE
    worst = EXIT_PAID
    rows = []
    for r in roots:
        paths = _ledgers(r)
        if not paths:
            rows.append("  NO_LEDGER   %-34s 접수 기록 0건" % os.path.basename(r))
            worst = _worse(worst, EXIT_NO_LEDGER)
            continue
        for p in paths:
            try:
                led = _load(p)
            except Exception as e:
                rows.append("  DRIFT       %-34s %s" % (os.path.basename(r), e))
                worst = _worse(worst, EXIT_DRIFT)
                continue
            rc, lines = _evaluate(led)
            tag = _tag(rc)
            rows.append("  %-11s %-34s %s" % (tag, os.path.basename(r), lines[0]))
            if rc != EXIT_PAID:
                # ★판독 불가 줄도 떠낸다 — CEO 가 읽는 칸이 audit 이고, 그 줄이 errno·플래그·크기 대조를 싣는다.
                rows.extend("      " + l.strip() for l in lines[1:]
                            if l.strip().startswith(("OPEN", "UNREADABLE", "UNRESOLVABLE", "EXEMPT", "CONFLICT", "DRIFT")))
            worst = _worse(worst, rc)
    print("[intake audit] 루트 %d · 최악 exit %d (%s)"
          % (len(roots), worst, _tag(worst)))
    print("\n".join(rows))
    return worst


def cmd_selftest(_a):
    import tempfile
    fails = []

    def chk(cond, msg):
        if not cond:
            fails.append(msg)

    # ⒜ 분모 없음은 0 이 아니다
    rc, _ = _evaluate({"id": "t", "instruction": {"text": "x", "sha256": _digest(b"x")}, "items": []})
    chk(rc == EXIT_NO_DENOM, "항목 0건이 NO_DENOM 이 아니다(rc=%d)" % rc)
    # ⒝ 지시 원문 부재
    rc, _ = _evaluate({"id": "t", "instruction": {}})
    chk(rc == EXIT_NO_DENOM, "지시 원문 부재가 NO_DENOM 이 아니다")
    # ⒞ 드리프트
    rc, _ = _evaluate({"id": "t", "instruction": {"text": "x", "sha256": _digest(b"y")}, "items": [{"text": "a"}]})
    chk(rc == EXIT_DRIFT, "다이제스트 불일치가 DRIFT 가 아니다")
    # ⒟ 체크박스만으로는 닫히지 않는다 (★dept-6 R23 의 법)
    st, _ = _item_state({"text": "a", "state": "done", "done": True})
    chk(st == "OPEN", "증거 없는 done 이 PAID 로 읽혔다 — 침묵이 공짜로 통과한다")
    # ⒠ 경로만 적으면 지불이 아니다
    st, why = _item_state({"state": "done", "evidence": "가" * MIN_EVIDENCE_CHARS,
                           "artifacts": ["/nonexistent/path/zzz"]})
    chk(st == "OPEN", "실존하지 않는 artifact 가 지불로 읽혔다")
    # ⒡ 맨 문자열 내구 artifact + 증거 = EXEMPT (신원 미확립)
    #   ★검체를 고쳤다(2026-10-15): 종전은 `NamedTemporaryFile` 을 썼는데 그것은 **임시 경로**라
    #   새 규칙(⒡')과 충돌했다 — 자기검사가 그 충돌을 잡았다. 내구 경로 = 이 파일 자신.
    real = os.path.abspath(__file__)
    st, why = _item_state({"state": "done", "evidence": "가" * MIN_EVIDENCE_CHARS, "artifacts": [real]})
    chk(st == "EXEMPT", "맨 문자열 내구 artifact 가 EXEMPT 가 아니다(%s)" % why)
    # ⒡' ★임시 경로만인 지불은 지불이 아니다 (2026-10-15 · CEO 자기 적발)
    import tempfile as _tf
    _fh = _tf.NamedTemporaryFile(delete=False, dir=_tf.gettempdir())
    _tmp = _fh.name; _fh.close()
    try:
        st, why = _item_state({"state": "done", "evidence": "가" * MIN_EVIDENCE_CHARS,
                               "artifacts": [_tmp]})
        chk(st == "OPEN", "임시 경로만인 지불이 PAID 로 읽혔다(%s)" % why)
    finally:
        os.unlink(_tmp)
    # ⒡'' ★Windows 임시 루트(2026-10-20 수리) — 주입한 env 로 어느 OS 에서나 잰다(대소문자·슬래시 무시)
    _wenv = {"TEMP": r"C:\Users\x\AppData\Local\Temp"}
    chk(_ephemeral("c:/users/X/appdata/local/temp/a.log", os_name="nt", env=_wenv),
        "Windows %TEMP% 아래 artifact 가 임시 경로로 읽히지 않았다")
    chk(not _ephemeral(r"C:\Users\x\Documents\a.log", os_name="nt", env=_wenv),
        "Windows 내구 경로가 임시 경로로 읽혔다")

    # ⒢ 보류는 release_condition 없이 성립하지 않는다 (★C52 — 항목마다 블로커를 대라)
    st, _ = _item_state({"state": "defer", "defer_reason": "나" * MIN_REASON_CHARS, "release_condition": ""})
    chk(st == "OPEN", "release_condition 공백 보류가 PAID 로 읽혔다")
    st, _ = _item_state({"state": "defer", "defer_reason": "나" * MIN_REASON_CHARS,
                         "release_condition": "A5 착지 후"})
    chk(st == "PAID", "정당한 보류가 인정되지 않았다")
    # ⒣ 열림이 하나라도 있으면 OPEN
    rc, lines = _evaluate({"id": "t", "instruction": {"text": "x", "sha256": _digest(b"x")},
                           "items": [{"state": "defer", "defer_reason": "나" * 30, "release_condition": "c"},
                                     {"state": "open", "text": "미지불"}]})
    chk(rc == EXIT_OPEN, "열림 1건이 있는데 OPEN 이 아니다")
    chk(any("미지불" in l for l in lines), "열린 항목의 id/본문이 출력되지 않았다")
    # ⒤' ★exit 코드 충돌 — 사용법 오류는 측정 코드와 섞이지 않는다(dept-5 발견)
    import subprocess
    # -B: -I 는 환경변수의 캐시 금지 봉인을 무시하므로, 번들 python 이면 이 자식이 .pyc 를 번들 안에 쓸 수 있다
    #     (2026-10-20 수리 · 오너 규칙 — 번들 python 은 번들 안에 캐시를 쓰지 않는다).
    _r = subprocess.run([sys.executable, "-I", "-B", os.path.abspath(__file__), "audit"],
                        capture_output=True)
    chk(_r.returncode == EXIT_USAGE,
        "audit --roots 누락이 %d 를 냈다 — USAGE(%d) 여야 하고 NO_DENOM(%d) 과 겹치면 안 된다"
        % (_r.returncode, EXIT_USAGE, EXIT_NO_DENOM))
    _r2 = subprocess.run([sys.executable, "-I", "-B", os.path.abspath(__file__), "nosuchcmd"],
                         capture_output=True)
    chk(_r2.returncode == EXIT_USAGE, "미지 서브커맨드가 USAGE 가 아니다(rc=%d)" % _r2.returncode)

    # ⒤ 네 종결 상태가 서로 다른 코드다 (★양성 통제 — 갈래마다 한 번은 돌려 본다)
    chk(len({EXIT_PAID, EXIT_OPEN, EXIT_NO_DENOM, EXIT_NO_LEDGER, EXIT_DRIFT}) == 5, "exit 코드 충돌")

    # ⒥ ★판독 불가는 변경이 아니다 (CEO 지시 2026-10-23 · WP10). 상태마다 검체 하나 — iCloud 도, dataless 파일도
    #   필요하지 않다: `_OPEN` 대역이 OSError 를 던지고(=실측 기제와 같은 갈래), 나머지는 운영 경로 그대로다.
    def _oserr_open(no):
        def _op(*_a, **_k):
            raise OSError(no, os.strerror(no))
        return _op

    _rsha, _rsize = _fp(real)
    _rec = {"path": real, "sha256_16": _rsha, "size": _rsize}
    _saved = _OPEN
    try:
        globals()["_OPEN"] = _oserr_open(errno.EDEADLK)
        _st_u, _why_u = _item_state({"state": "done", "evidence": "가" * MIN_EVIDENCE_CHARS,
                                     "artifacts": [dict(_rec)]})
        _rc_u, _lines_u = _evaluate({"id": "t", "instruction": {"text": "x", "sha256": _digest(b"x")},
                                     "items": [{"state": "done", "evidence": "가" * MIN_EVIDENCE_CHARS,
                                                "artifacts": [dict(_rec)]}]})
        _rc_mix, _ = _evaluate({"id": "t", "instruction": {"text": "x", "sha256": _digest(b"x")},
                                "items": [{"state": "done", "evidence": "가" * MIN_EVIDENCE_CHARS,
                                           "artifacts": [dict(_rec)]},
                                          {"state": "open", "text": "미지불"}]})
        _pay_sha, _pay_size = _fp(real)
    finally:
        globals()["_OPEN"] = _saved
    chk(_st_u == "UNREADABLE", "판독 불가 artifact 가 UNREADABLE 이 아니다(%s · %s)" % (_st_u, _why_u))
    chk("재지불" not in _why_u, "판독 불가인데 재지불 처방을 찍었다 — ⛔증거를 지어내라는 지시다(%s)" % _why_u)
    chk("변경됐다" not in _why_u, "판독 불가를 「변경됐다」로 찍었다(%s)" % _why_u)
    chk("무효가 아니다" in _why_u, "지불이 무효가 아니라는 말이 없다(%s)" % _why_u)
    chk("errno" in _why_u, "errno 를 찍지 않았다(%s)" % _why_u)
    chk("정황" in _why_u, "크기 일치를 정황으로만 말하지 않았다(%s)" % _why_u)
    chk(_rc_u == EXIT_UNREADABLE, "판독 불가만 남은 대장이 %d 다 — %d 여야 한다" % (_rc_u, EXIT_UNREADABLE))
    chk(_rc_mix == EXIT_OPEN, "열린 항목 1건이 판독 불가에 가려졌다(rc=%d) — 열림이 이긴다" % _rc_mix)
    chk((_pay_sha, _pay_size) == (None, None), "지불 시점에 읽히지 않는 artifact 가 지문을 얻었다")
    # ⒥' 음성 대조 — 읽히는 파일의 진짜 지문 불일치는 종전대로 CHANGED(OPEN) + 재지불이다.
    _st_c, _why_c = _item_state({"state": "done", "evidence": "가" * MIN_EVIDENCE_CHARS,
                                 "artifacts": [{"path": real, "sha256_16": "0" * 16, "size": 1}]})
    chk(_st_c == "OPEN" and "재지불" in _why_c, "진짜 변경이 CHANGED(OPEN·재지불)로 읽히지 않았다(%s · %s)"
        % (_st_c, _why_c))
    # ⒦ 부재는 또 다른 사실이다 — 변경도, 판독 불가도 아니다.
    _st_a, _why_a = _item_state({"state": "done", "evidence": "가" * MIN_EVIDENCE_CHARS,
                                 "artifacts": [{"path": real + ".no-such", "sha256_16": "0" * 16, "size": 1}]})
    chk(_st_a == "OPEN" and "부재" in _why_a, "부재 artifact 가 부재로 읽히지 않았다(%s · %s)" % (_st_a, _why_a))
    chk(_probe(real + ".no-such")[0] == PROBE_ABSENT, "없는 경로가 ABSENT 가 아니다")
    chk(_probe(os.path.dirname(real))[0] == PROBE_NOT_REGULAR, "디렉터리가 NOT_REGULAR 가 아니다")
    chk(_probe(real)[0] == PROBE_OK, "읽히는 정규 파일이 OK 가 아니다")
    # ⒧ darwin 전용 비트 해석 — 다른 OS 는 숫자만(U5: 그 OS 는 errno 로 분류한다).
    chk("dataless" in _flags_note(SF_DATALESS, plat="darwin"), "darwin 에서 dataless 이름이 없다")
    chk("dataless" not in _flags_note(SF_DATALESS, plat="win32"), "darwin 아닌 OS 에서 비트를 해석했다")
    chk(_flags_note(None) == "", "st_flags 없음이 빈 말이 아니다")
    chk(len({EXIT_PAID, EXIT_OPEN, EXIT_NO_DENOM, EXIT_NO_LEDGER, EXIT_DRIFT, EXIT_UNATTRIB,
             EXIT_UNREADABLE}) == 7, "exit 코드 충돌(UNREADABLE 포함)")
    chk(_worse(EXIT_UNREADABLE, EXIT_OPEN) == EXIT_OPEN, "열림이 판독 불가를 이기지 못했다")
    chk(_worse(EXIT_PAID, EXIT_UNREADABLE) == EXIT_UNREADABLE, "판독 불가가 지불에 묻혔다")
    chk(_worse(EXIT_OPEN, EXIT_DRIFT) == EXIT_DRIFT, "기존 심각도 순서가 바뀌었다(OPEN < DRIFT)")

    if fails:
        for f in fails:
            print("SELFTEST FAIL  " + f, file=sys.stderr)
        return EXIT_SELFTEST
    print("SELFTEST OK  15군 · 지불 검사기 · 분모 부재는 통과가 아니다 · 체크박스는 닫지 못한다 · "
          "판독 불가는 변경이 아니다")
    return EXIT_PAID


class _Parser(argparse.ArgumentParser):
    """★exit 코드 충돌 제거(dept-5 가 첫 사용에서 발견 · 2026-10-15).

    argparse 기본값은 사용법 오류에 **exit 2** 를 쓴다. 이 도구의 2 는 `NO_DENOM`(분모 부재)이라
    ★같은 숫자가 두 사실을 뜻했다 — `audit` 를 `--roots` 없이 부르면 2 가 나오고, 그것을
    「항목 목록이 없다」로 읽을 수 있었다. 사용법 오류는 측정이 아니므로 측정 코드와 섞지 않는다.
    ⇒ 사용법 오류는 **5(USAGE)** 로만 나간다.
    """

    def error(self, message):
        self.print_usage(sys.stderr)
        print("USAGE  %s" % message, file=sys.stderr)
        sys.exit(EXIT_USAGE)

    def exit(self, status=0, message=None):
        if message:
            sys.stderr.write(message)
        sys.exit(EXIT_USAGE if status == 2 else status)


def main(argv=None):
    ap = _Parser(prog="javis_intake.py", add_help=True)
    ap.add_argument("--root", default=None, help="팩 루트(기본 CYS_PACK_DIR)")
    sub = ap.add_subparsers(dest="cmd", required=True, parser_class=_Parser)

    p = sub.add_parser("open"); p.add_argument("--id", required=True)
    p.add_argument("--instruction-file", required=True)
    p.add_argument("--item", action="append", default=[])
    p.add_argument("--source", default="owner", choices=["owner", "ceo", "other"])
    p.set_defaults(fn=cmd_open)

    p = sub.add_parser("add-item"); p.add_argument("--id", required=True)
    p.add_argument("--item", required=True); p.set_defaults(fn=cmd_add_item)

    p = sub.add_parser("pay"); p.add_argument("--id", required=True)
    p.add_argument("--item-no", type=int, required=True)
    p.add_argument("--evidence", required=True)
    p.add_argument("--artifact", action="append", default=[])
    p.set_defaults(fn=cmd_pay)

    p = sub.add_parser("defer"); p.add_argument("--id", required=True)
    p.add_argument("--item-no", type=int, required=True)
    p.add_argument("--reason", required=True)
    p.add_argument("--release-condition", required=True)
    p.set_defaults(fn=cmd_defer)

    p = sub.add_parser("status"); p.add_argument("--id", default=None)
    p.add_argument("--json", action="store_true")
    p.add_argument("--all", action="store_true",
                   help="★CEO 칸 — 좌석 범위를 풀고 그 팩의 모든 대장을 합산한다")
    p.set_defaults(fn=cmd_status)

    p = sub.add_parser("stamp"); p.add_argument("--id", required=True)
    p.set_defaults(fn=cmd_stamp)

    p = sub.add_parser("audit"); p.add_argument("--roots", required=True)
    p.set_defaults(fn=cmd_audit)

    p = sub.add_parser("selftest"); p.set_defaults(fn=cmd_selftest)

    a = ap.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
