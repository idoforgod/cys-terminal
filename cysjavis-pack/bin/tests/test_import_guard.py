#!/usr/bin/env python3
"""D1 회귀 잠금 — 팩 `bin/` 스크립트의 형제 모듈 import 경로 가드 검증 (2026-07-29).

★배경(실측): Windows 배포본의 번들 파이썬은 embeddable 형태(`python312._pth`)라
표준 경로 계산을 우회한다 → **스크립트 폴더가 sys.path 에 들어오지 않고** `cwd`·`PYTHONPATH` 도
무시된다. 그래서 `bin/` 형제 모듈의 bare import 가 조용히 실패한다.

  2026-07-29 Windows 0.14.4 실측:
    File "C:\\Users\\...\\.cys\\pack\\bin\\javis_event.py", line 19, in <module>
        import javis_scrub
    ModuleNotFoundError: No module named 'javis_scrub'

훅은 fail-open(무조건 exit 0)이라 이 실패는 **증상 없이** 산다 → 사람 눈이 아니라 게이트가 잡아야 한다.
이 테스트가 그 게이트다. 새 스크립트가 가드 없이 형제 모듈을 import 하면 **여기서 실패**한다.

검증 항목
  ① 형제 모듈 import 가 있는 모든 `bin/*.py` 는 경로 가드를 갖는다
  ② **모든** 형제 import 가 자기를 지배하는 유효한 가드 뒤에 온다
  ③ 실패 조건 재현: 스크립트 폴더가 sys.path 에 없는 상태에서 형제 import 가 통과한다
     (파일마다 **독립 자식 프로세스**로 실행 — 검체 간 오염 0)
  ④ 셀프테스트(`--selftest`): 스캐너 자신이 우회에 뚫리지 않는지 — 우회 검체를 임시 폴더에
     심고 **반드시 FAIL 하는지**, 정상 형태는 **반드시 PASS 하는지** 확인한다.

──────────────────────────────────────────────────────────────────────────────
★이력 1 — 2026-07-29 R1: 정규식 → `ast.parse` 전환.
  정규식판은 적대 검증에서 4가지 우회를 통과시켰다(대입만 앞·`append` 는 import 뒤 / 동적 import /
  콤마 결합 / 주석·죽은 코드 속 가드).

★이력 2 — 2026-07-29 적대 검증 2차(BLOCK 3 · REVISE 3) 반영. AST 1차판은
  "문법 노드 존재 + 소스 줄"을 런타임 지배로 오인했다. 실측으로 깨진 것과 처방:
    BLOCK-1 동적 import 6형태(`exec`/`eval`/변수·f-string `__import__`/변수 `import_module`/
            데코레이터 실행식)를 **검사 대상에서 통째로 누락** → 정적 증명 불가 형태를 **fail-closed**
            로 금지하고, 데코레이터·기본값·애노테이션·클래스 base 등 **정의 시점 실행식**도 방문한다.
    BLOCK-2 `if ():` 같은 확정 거짓 분기를 unknown 으로 흘리고, `try/finally` 되돌리기·
            `sys.path.clear()` 로 **철회된 가드**를 유효로 인정 → 리터럴 컨테이너 진릿값 처리 +
            **파괴적 조작(destroy) 이벤트**를 도입해 가드↔import 사이의 철회를 무효화한다.
    BLOCK-3 `③` 이 한 프로세스에서 순차 실행돼 앞 검체가 남긴 `sys.path`·`sys.modules` 로
            뒤 검체가 **무임승차** → 파일마다 **독립 자식 프로세스**로 격리한다.
    REVISE-1 정상 형태를 거짓 FAIL(`from sys import path` 별칭 · `q = sys.path` · 호출되는
            헬퍼 가드 · `if TYPE_CHECKING:` · 데코레이터 가드 · 함수 내 import 의 소스 줄 순서)
            → 별칭 추적 · 헬퍼 호출 인정 · TYPE_CHECKING 제외 · **스코프 교차 시 줄 순서 미적용**.
    REVISE-2 `run_name='__guardprobe__'` 는 `if __name__ == "__main__":` 을 실행하지 않는데
            정적 분류는 그 안의 import 를 최상위로 표시 → **main 전용 import 는 ③ 대상에서 제외**
            (①② 는 그대로 적용). 현행 팩 영향 0건이나 분류와 실행기의 의미를 일치시킨다.
    REVISE-3 셀프테스트가 이미 잡는 것만 확인 → 위 공격 검체를 **전부 편입**.
  ※ 클래스 본문은 `class` 문 실행 중 **실제로 실행된다**(1차판 주석의 "정의만 된다"는 거짓).
     따라서 클래스 본문은 별도 스코프가 아니라 **현 스코프의 실행 흐름에 인라인**한다.
──────────────────────────────────────────────────────────────────────────────

★이력 3 — 2026-07-29 적대 검증 **2차**(BLOCK 4 · REVISE 1) 반영.
  1차 지적을 **이름 그대로** 담은 검체는 통과했지만, 그 규칙을 동치 코드로 일반화하자 다시 뚫렸다.
    BLOCK-1 `sys.path[:] = []` · `__delitem__` · 2단 별칭(`r = q`) · **철회 함수 호출**
            → 파괴 판정을 열거에서 **allowlist 반전**으로(추가·읽기 전용 외 전부 철회),
              별칭은 고정점까지 전파, 헬퍼 투영을 가드·철회 **양쪽**으로 대칭화.
    BLOCK-2 같은 이름 함수 재정의(첫 정의만 보존해 실제로는 빈 함수가 불리는데 PASS) ·
            함수 호출이 모듈 가드보다 **앞서는** 경우
            → 동명 재정의는 이름 기반 투영 금지(fail-closed), 바깥 스코프 가드는
              **모듈 최상위 첫 호출보다 앞**일 것을 요구.
    BLOCK-3 `TYPE_CHECKING = True` 직접 바인딩 · `getattr(builtins,'ex'+'ec')`
            → TYPE_CHECKING 은 `typing` 에서 왔고 재대입이 없을 때만 신뢰, `builtins` 접촉은 금지.
    BLOCK-4 ★③ 프로브가 형제 `ModuleNotFoundError` 만 FAIL 로 보고 **그 밖의 비정상 종료·timeout 을
            통과**시켰다 → 가드 평가 도중 죽어 import 에 **도달조차 못 한 실행**이 "형제 import 통과"
            의 증거로 계상됐다. **rc=0 만 PASS**로 바꾸고 실패 사유를 구분해 보고한다.
    REVISE-1 `sys.path.pop(0)` 은 우리가 끝에 붙인 항목을 지우지 않으므로 실제로는 정상인데 FAIL —
            **의도적으로 고치지 않는다**. 아래 '계약' 참조.

★계약: **가드를 건 뒤 형제 import 전까지 `sys.path` 를 건드리지 마라.**
  인덱스·원소 위치를 추적해 "이 조작은 무해하다"를 증명하는 대신, 조작 자체를 금지한다.
  근거 ①실물 팩에 해당 형태 0건(측정) ②틀리는 방향이 안전하다 — 과탐은 CI 에서 **시끄럽게**
  드러나 즉시 고쳐지지만, 미검출은 Windows 에서 **조용히** 산다. 이 게이트의 존재 이유가 후자다.
  ③"무해함을 증명하는 분석"은 경계가 없다(2차 검증이 그 경계 없음을 실측으로 보였다).

★이 게이트가 증명하지 않는 것(정직한 한계)
  - 임의 dataflow. `sys.path` 를 여러 단계 거쳐 조작하면 추적하지 못한다. 그래서 정적 증명이
    불가능한 형태(`exec`/`eval`/비상수 동적 import)는 **분석 포기가 아니라 금지**로 처리한다.
  - 실행 순서. 같은 스코프 안에서만 소스 줄 순서를 실행 순서로 본다. 스코프를 넘으면
    "모듈 로드 시 실행된 가드가 이후의 함수 호출보다 앞선다"고 가정한다(가드 줄보다 앞에서
    그 함수를 호출하는 코드는 잡지 못한다).
  - ③ 은 모듈 레벨만 돈다. 함수 안에서만 하는 형제 import 는 ①② 정적 판정으로만 커버된다.
  - **가드 식이 실행 중 예외를 던지는 경우**(예: `sys.path.append((1/0, _S)[1])`)는 정적으로
    판정할 수 없다 — 식의 평가 결과를 정적으로 아는 것은 결정 불가능이다. 이건 ③ 이 잡는다
    (rc≠0 → FAIL). 즉 ①②와 ③ 은 서로를 대체하지 않는 **다른 층**이다.

★가드 형태 규약: `sys.path.append` + 중복 검사 (`insert(0)` 아님).
  목적은 **발견**이지 우선순위 강등이 아니다 — bin/ 을 stdlib 앞에 놓으면 미래에 `bin/secrets.py`
  같은 파일이 표준 라이브러리를 shadowing 한다. 근거 교리: `src/lib.rs` compose_unix_pane_path 주석
  MAJ#1 "append 인 이유 — 발견이 목적, 기존 항목의 precedence 를 강등하지 않는다".
  (기존 `insert(0)` 선례들은 무해하므로 강제 교정하지 않는다 — 신규 작성분의 권장 형태만 규정한다.)

실행: python3 cysjavis-pack/bin/tests/test_import_guard.py            (0=전건 PASS)
      python3 cysjavis-pack/bin/tests/test_import_guard.py --selftest (스캐너 자체 검증만)
"""
import ast
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.normpath(os.path.join(HERE, ".."))

_results = []


def check(n, c, d=""):
    _results.append(bool(c))
    print(("PASS " if c else "FAIL ") + n + (" | " + d if d else ""))


# ── AST 유틸 ────────────────────────────────────────────────────────────────
_GUARD_METHODS = {"append", "insert", "extend"}
# `sys.path` 를 읽기만 하는 메서드. **이 목록과 _GUARD_METHODS 밖의 모든 조작은 철회로 본다**
# (2차 적대 검증 BLOCK-1: `__delitem__` 처럼 목록에 없는 동치 경로가 계속 나온다 → 열거가 아니라
#  allowlist 로 뒤집는다). 즉 계약은 "가드를 건 뒤 import 전까지 sys.path 를 건드리지 마라"다.
_PATH_READONLY = {"index", "count", "copy", "__contains__", "__len__", "__iter__", "__getitem__"}
# __file__ 이 아닌 경로원(源)으로도 가드가 성립하는 헬퍼 이름(기존 선례 보호)
_PATH_HELPERS = {"pack_dir", "bin_dir"}
# 정적 증명이 불가능해 **금지**하는 형태
_OPAQUE_FUNCS = {"exec", "eval"}
_DYNAMIC_IMPORTERS = {"import_module", "__import__"}


def _attr_chain(node):
    """`sys.path.append` → ('sys','path','append'). 이름 체인이 아니면 None."""
    parts = []
    while isinstance(node, ast.Attribute):
        parts.append(node.attr)
        node = node.value
    if isinstance(node, ast.Name):
        parts.append(node.id)
        return tuple(reversed(parts))
    return None


def _static_truth(node):
    """정적으로 확정되는 진릿값. 모르면 None.

    ★리터럴 컨테이너까지 본다 — `if ():` 는 확정 거짓인데 1차판은 unknown 으로 흘렸다(BLOCK-2).
    임의 식의 `eval` 은 하지 않는다(그 자체가 새 공격면).
    """
    if isinstance(node, ast.Constant):
        try:
            return bool(node.value)
        except Exception:
            return None
    if isinstance(node, (ast.Tuple, ast.List, ast.Set)):
        return bool(node.elts)
    if isinstance(node, ast.Dict):
        return bool(node.keys)
    if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.Not):
        inner = _static_truth(node.operand)
        return None if inner is None else (not inner)
    return None


def _is_main_guard(test):
    """`if __name__ == "__main__":` 인가 (그 안은 모듈 로드만으로는 실행되지 않는다)."""
    if not isinstance(test, ast.Compare) or len(test.ops) != 1:
        return False
    if not isinstance(test.ops[0], ast.Eq):
        return False
    left, right = test.left, test.comparators[0]
    def _is_name_dunder(n):
        return isinstance(n, ast.Name) and n.id == "__name__"
    def _is_main_str(n):
        return isinstance(n, ast.Constant) and n.value == "__main__"
    return (_is_name_dunder(left) and _is_main_str(right)) or \
           (_is_name_dunder(right) and _is_main_str(left))


def _is_type_checking(test):
    """`if TYPE_CHECKING:` 계열인가 (런타임에는 실행되지 않는다 — REVISE-1)."""
    for n in ast.walk(test):
        if isinstance(n, ast.Name) and n.id == "TYPE_CHECKING":
            return True
        if isinstance(n, ast.Attribute) and n.attr == "TYPE_CHECKING":
            return True
    return False


def _type_checking_trusted(tree):
    """이 파일의 `TYPE_CHECKING` 을 '런타임 거짓'으로 믿어도 되는가.

    ★2차 적대 검증 BLOCK-3: 이름만 보고 믿었더니 `TYPE_CHECKING = True` 로 두면 실제로 실행되는
      import 가 분석 대상에서 통째로 사라졌다. `typing` 에서 가져왔고 **이 파일에서 재대입하지
      않은** 경우에만 믿는다. 그 밖에는 보통 분기로 취급한다(fail-closed).
    """
    imported = assigned = False
    for n in ast.walk(tree):
        if isinstance(n, ast.ImportFrom) and n.module == "typing" and n.level == 0:
            if any(a.name == "TYPE_CHECKING" for a in n.names):
                imported = True
        elif isinstance(n, ast.Assign):
            for t in n.targets:
                if isinstance(t, ast.Name) and t.id == "TYPE_CHECKING":
                    assigned = True
        elif isinstance(n, ast.AnnAssign):
            if isinstance(n.target, ast.Name) and n.target.id == "TYPE_CHECKING":
                assigned = True
    return imported and not assigned


def _func_inline_exprs(st):
    """함수 정의문 중 **정의 시점에 실행되는** 식들(데코레이터·기본값·애노테이션).

    본문은 나중에 호출될 때 실행되지만 이것들은 지금 실행된다 — 1차판은 여기를 통째로
    건너뛰어 데코레이터 인자 속 import·가드를 놓쳤다(BLOCK-1 / REVISE-1).
    """
    out = list(st.decorator_list)
    a = st.args
    out += [d for d in list(a.defaults) + list(a.kw_defaults) if d is not None]
    for grp in (getattr(a, "posonlyargs", []), a.args, a.kwonlyargs):
        out += [x.annotation for x in grp if x.annotation is not None]
    for x in (a.vararg, a.kwarg):
        if x is not None and x.annotation is not None:
            out.append(x.annotation)
    if st.returns is not None:
        out.append(st.returns)
    return out


def _split_scope(body, tc_trusted):
    """스코프 본문 → (실행 단순문[(stmt, main_only)], 정의시점 실행식[(expr, main_only)],
                     지연 스코프[(FunctionDef, main_only)]).

    - 함수 본문만 **지연**된다. 클래스 본문은 `class` 문 실행 중 **실제로 실행**되므로 인라인한다.
    - `if False:` / `if ():` 등 정적 거짓 가지는 건너뛴다.
    - `if TYPE_CHECKING:` 본문은 런타임에 실행되지 않으므로 제외한다.
    - `if __name__ == "__main__":` 본문은 실행되지만 **모듈 로드만으로는 아니다** → main_only 표시.
    """
    simple, inline, funcs = [], [], []

    def rec(b, mo):
        for st in b:
            if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef)):
                inline.extend((e, mo) for e in _func_inline_exprs(st))
                funcs.append((st, mo))
            elif isinstance(st, ast.ClassDef):
                inline.extend((e, mo) for e in st.decorator_list)
                inline.extend((e, mo) for e in st.bases)
                inline.extend((kw.value, mo) for kw in st.keywords)
                rec(st.body, mo)          # ★클래스 본문은 지금 실행된다
            elif isinstance(st, ast.If):
                if tc_trusted and _is_type_checking(st.test):
                    rec(st.orelse, mo)     # 본문은 런타임 미실행
                elif _is_main_guard(st.test):
                    rec(st.body, True)
                    rec(st.orelse, mo)
                else:
                    t = _static_truth(st.test)
                    if t is not False:
                        rec(st.body, mo)
                    if t is not True:
                        rec(st.orelse, mo)
            elif isinstance(st, ast.Try):
                rec(st.body, mo)
                for h in st.handlers:
                    rec(h.body, mo)
                rec(st.orelse, mo)
                rec(st.finalbody, mo)      # finally 는 마지막에 — 철회 판정이 줄 순서와 맞는다
            elif isinstance(st, (ast.With, ast.AsyncWith)):
                rec(st.body, mo)
            elif isinstance(st, (ast.For, ast.AsyncFor, ast.While)):
                rec(st.body, mo)
                rec(st.orelse, mo)
            else:
                simple.append((st, mo))

    rec(body, False)
    return simple, inline, funcs


def _derives_from_file(expr, known):
    """이 식이 `__file__`(또는 그 파생 변수·경로 헬퍼)에서 온 경로인가."""
    for n in ast.walk(expr):
        if isinstance(n, ast.Name) and (n.id == "__file__" or n.id in known):
            return True
        if isinstance(n, ast.Call):
            ch = _attr_chain(n.func)
            if ch and ch[-1] in _PATH_HELPERS:
                return True
            if isinstance(n.func, ast.Name) and n.func.id in _PATH_HELPERS:
                return True
    return False


def _path_aliases(tree):
    """`sys.path` 를 가리키는 이름 집합. `from sys import path` · `q = sys.path` · **`r = q`**.

    ★2차 적대 검증 BLOCK-1: 1단 별칭만 잡았더니 `r = q` 로 한 번 더 넘기고 `r.clear()` 하면
      철회가 비사건이 됐다. 고정점까지 전파한다.
    """
    alias = set()
    changed = True
    while changed:
        changed = False
        for n in ast.walk(tree):
            if isinstance(n, ast.ImportFrom) and n.module == "sys" and n.level == 0:
                for a in n.names:
                    nm = a.asname or a.name
                    if a.name == "path" and nm not in alias:
                        alias.add(nm)
                        changed = True
            elif isinstance(n, ast.Assign):
                v = n.value
                if _attr_chain(v) == ("sys", "path") or (isinstance(v, ast.Name) and v.id in alias):
                    for t in n.targets:
                        if isinstance(t, ast.Name) and t.id not in alias:
                            alias.add(t.id)
                            changed = True
    return alias


def _is_path_obj(node, alias):
    """이 노드가 `sys.path` 또는 그 별칭인가."""
    if _attr_chain(node) == ("sys", "path"):
        return True
    return isinstance(node, ast.Name) and node.id in alias


def _assign_hits_path(target, alias):
    """대입 대상이 `sys.path` 자신이거나 그 슬라이스/요소인가(`sys.path[:] = []` 포함)."""
    if _is_path_obj(target, alias):
        return True
    return isinstance(target, ast.Subscript) and _is_path_obj(target.value, alias)


def _path_escapes(tree, alias):
    """`sys.path`(또는 별칭)가 **모델링되지 않은 위치**로 새는 지점 [(줄, 사유)].

    ★3차 적대 검증 BLOCK(D1-F1) 반영. 2차의 allowlist 반전은 "이미 sys.path 라고 인식한 객체"의
      **메서드 이름** 축만 뒤집었다. 그 위층인 "무엇이 sys.path 인가"는 여전히 구문 열거라,
      값이 추적 밖으로 새면(함수 인자·컨테이너 원소·튜플/for/컴프리헨션 타깃·walrus·
      언바운드 메서드 수신자·operator 인자) 그 뒤의 조작이 **비사건**이 됐다 — 8/8 우회 실증.

    처방은 열거를 더 늘리는 게 아니라 **기본 처분을 뒤집는 것**이다: 우리가 뜻을 아는 위치
    (메서드 수신자 · 대입/del 타깃 · 멤버십 테스트 · 첨자)가 아니면 **금지(opaque)**.
    도크스트링이 선언한 원칙("정적 증명 불가는 분석 포기가 아니라 금지")을 destroy 축에도
    일관 적용하는 것이다. 실물 팩 76개 파일 측정 결과 이런 위치 **0건** — 비용 없이 부류가 닫힌다.
    """
    parent = {}
    for p in ast.walk(tree):
        for c in ast.iter_child_nodes(p):
            parent[c] = p

    def _modeled(p):
        return isinstance(p, (ast.Attribute, ast.Compare, ast.Subscript,
                              ast.Assign, ast.AugAssign, ast.Delete))

    out = []
    for n in ast.walk(tree):
        ref = None
        if _attr_chain(n) == ("sys", "path"):
            ref = "sys.path"
        elif isinstance(n, ast.Name) and n.id in alias and isinstance(n.ctx, ast.Load):
            ref = "sys.path 별칭 '%s'" % n.id
        if ref is None:
            continue
        p = parent.get(n)
        if not _modeled(p):
            out.append((n.lineno, "%s 가 추적 불가 위치(%s)로 샌다 — 이후 조작을 증명할 수 없다"
                        % (ref, type(p).__name__ if p else "?")))
    # `setattr(sys, 'path', ...)` · `sys.__dict__` — sys.path 를 **거치지 않는** 상태 전이(D1-F2)
    for n in ast.walk(tree):
        if isinstance(n, ast.Call) and isinstance(n.func, ast.Name) and n.func.id == "setattr":
            if n.args and isinstance(n.args[0], ast.Name) and n.args[0].id == "sys":
                out.append((n.lineno, "setattr(sys, ...) — sys.path 를 우회한 상태 전이"))
        if _attr_chain(n) == ("sys", "__dict__"):
            out.append((n.lineno, "sys.__dict__ 접근 — sys.path 를 우회한 상태 전이"))
    return out


def _uses_builtins(tree):
    """`builtins` 를 건드리는가 — `getattr(builtins, 'ex'+'ec')` 류로 금지 판정을 우회할 수 있다."""
    for n in ast.walk(tree):
        if isinstance(n, ast.Name) and n.id == "builtins":
            return True
        if isinstance(n, ast.Import) and any(a.name.split(".")[0] == "builtins" for a in n.names):
            return True
        if isinstance(n, ast.ImportFrom) and n.module and n.module.split(".")[0] == "builtins":
            return True
    return False


def collect(tree, siblings, selfname):
    """가드·철회·형제 import·금지형태를 **스코프 정보와 함께** 수집.

    반환 = {"guards": [(줄, 스코프키)], "destroys": [(줄, 스코프키)],
            "imports": [(줄, 모듈, 스코프체인집합, main_only)], "opaque": [(줄, 사유)]}
    """
    alias = _path_aliases(tree)
    tc_trusted = _type_checking_trusted(tree)
    guards, destroys, imports, opaque = [], [], [], []
    calls = []                 # (줄, 함수이름, 스코프체인) — 헬퍼 호출 후보
    func_scope = {}            # 함수이름 → 그 함수의 스코프키
    func_dupes = set()         # 같은 이름으로 두 번 이상 정의된 함수(이름 기반 투영 금지)
    scope_name = {}            # 스코프키 → 함수이름
    scope_has_guard = set()
    scope_has_destroy = set()
    module_calls = {}          # 함수이름 → [모듈 최상위에서 호출된 줄...]

    if _uses_builtins(tree):
        opaque.append((1, "builtins 를 건드린다 — getattr(builtins,'ex'+'ec') 류로 금지 판정을 우회할 수 있다"))
    opaque.extend(_path_escapes(tree, alias))

    def take_import(lineno, dotted, chain, mo):
        root = dotted.split(".")[0]
        if root in siblings and root != selfname:
            imports.append((lineno, dotted, set(chain), mo))

    def scan(node, chain, known, mo):
        """단순문 또는 정의시점 실행식 하나를 훑어 이벤트를 뽑는다(중첩 문 본문 없음)."""
        # `sys.path = ...` / `sys.path[:] = []` / `del sys.path[...]` = 통째 파괴
        if isinstance(node, ast.Assign) and any(_assign_hits_path(t, alias) for t in node.targets):
            destroys.append((node.lineno, chain[-1]))
            scope_has_destroy.add(chain[-1])
        if isinstance(node, ast.Delete):
            for t in node.targets:
                tgt = t.value if isinstance(t, ast.Subscript) else t
                if _is_path_obj(tgt, alias):
                    destroys.append((node.lineno, chain[-1]))
                    scope_has_destroy.add(chain[-1])
        # AugAssign — `+=` 만 순수 추가(가드 형태)이고 그 외(`*= 0` 등)는 제자리 철회다.
        # ★3차 감사 지적: 이 축이 양방향으로 열려 있었다(`sys.path *= 0` 미검출 실증).
        if isinstance(node, ast.AugAssign) and _assign_hits_path(node.target, alias):
            if isinstance(node.op, ast.Add):
                if _derives_from_file(node.value, known):
                    guards.append((node.lineno, chain[-1]))
                    scope_has_guard.add(chain[-1])
            else:
                destroys.append((node.lineno, chain[-1]))
                scope_has_destroy.add(chain[-1])

        for n in ast.walk(node):
            if isinstance(n, ast.Import):
                for a in n.names:                      # 콤마 결합 포함
                    take_import(n.lineno, a.name, chain, mo)
            elif isinstance(n, ast.ImportFrom):
                if n.level == 0 and n.module:          # 상대 import 는 대상 아님
                    take_import(n.lineno, n.module, chain, mo)
            elif isinstance(n, ast.Call):
                fname = n.func.id if isinstance(n.func, ast.Name) else None
                ch = _attr_chain(n.func)
                meth = ch[-1] if ch else None

                # ── 금지 형태: 정적 증명 불가 ──
                if fname in _OPAQUE_FUNCS:
                    opaque.append((n.lineno, "%s() 는 import 를 숨길 수 있어 정적 증명이 불가하다" % fname))
                if (fname in _DYNAMIC_IMPORTERS) or (meth in _DYNAMIC_IMPORTERS):
                    a0 = n.args[0] if n.args else None
                    if isinstance(a0, ast.Constant) and isinstance(a0.value, str):
                        take_import(n.lineno, a0.value, chain, mo)
                    else:
                        opaque.append((n.lineno, "동적 import 의 인자가 상수 문자열이 아니다"))

                # ── 가드/철회 ──
                # ★allowlist 로 뒤집는다: 추가 계열(append/insert/extend)과 읽기 전용을 뺀
                #   **모든** sys.path 조작은 철회다(`__delitem__` 등 동치 경로가 계속 나온다).
                if isinstance(n.func, ast.Attribute) and _is_path_obj(n.func.value, alias):
                    if meth in _GUARD_METHODS:
                        if any(_derives_from_file(a, known) for a in n.args):
                            guards.append((n.lineno, chain[-1]))
                            scope_has_guard.add(chain[-1])
                        # 파일 파생이 아닌 추가는 무관 — 철회도 아니다
                    elif meth not in _PATH_READONLY:
                        destroys.append((n.lineno, chain[-1]))
                        scope_has_destroy.add(chain[-1])

                # ── 헬퍼 호출 후보(가드/철회를 품은 함수를 실제로 부르는 형태) ──
                if fname:
                    calls.append((n.lineno, fname, set(chain)))
                    if len(chain) == 1:                     # 모듈 최상위 호출 시점
                        module_calls.setdefault(fname, []).append(n.lineno)

    def visit(body, chain, known):
        simple, inline, funcs = _split_scope(body, tc_trusted)
        local = set(known)
        for st, mo in simple:
            if isinstance(st, ast.Assign) and _derives_from_file(st.value, local):
                for t in st.targets:
                    if isinstance(t, ast.Name):
                        local.add(t.id)
            elif isinstance(st, ast.AnnAssign) and st.value is not None and _derives_from_file(st.value, local):
                if isinstance(st.target, ast.Name):
                    local.add(st.target.id)
            scan(st, chain, local, mo)
        for ex, mo in inline:
            scan(ex, chain, local, mo)
        for fn_node, _mo in funcs:
            key = id(fn_node)
            scope_name[key] = fn_node.name
            if fn_node.name in func_scope:
                func_dupes.add(fn_node.name)   # 같은 이름 재정의 → 이름 기반 투영 금지
            func_scope.setdefault(fn_node.name, key)
            visit(fn_node.body, chain + (key,), local)

    visit(tree.body, (0,), set())

    # 헬퍼 호출 투영: 가드를 품은 함수를 부르면 그 호출 지점이 가드고,
    # **철회를 품은 함수를 부르면 그 호출 지점이 철회다**(2차 BLOCK-1 — 비대칭이었다).
    # ★같은 이름이 두 번 정의된 함수는 어느 쪽이 불리는지 증명할 수 없어 **투영하지 않는다**
    #   (2차 BLOCK-2: 가드 있는 첫 정의만 보존해 실제로는 빈 함수가 불리는데 PASS 했다).
    #   가드 쪽은 인정하지 않고, 철회 쪽은 fail-closed 로 인정한다.
    for lineno, fname, chain in calls:
        key = func_scope.get(fname)
        if key is None:
            continue
        here = sorted(chain)[-1] if chain else 0
        if key in scope_has_destroy or (fname in func_dupes and key in scope_has_guard):
            destroys.append((lineno, here))
        elif key in scope_has_guard and fname not in func_dupes:
            guards.append((lineno, here))

    guards.sort()
    destroys.sort()
    imports.sort(key=lambda h: h[0])
    opaque.sort()
    return {"guards": guards, "destroys": destroys, "imports": imports, "opaque": opaque,
            "scope_name": scope_name, "module_calls": module_calls}


def is_protected(imp, info):
    """이 import 가 유효한 가드로 보호되는가.

    - 가드 스코프가 import 를 감싸야 한다(모듈 또는 그 함수 자신·바깥).
    - **같은 스코프**면 소스 줄 순서를 실행 순서로 본다(가드가 앞이어야 한다).
    - **바깥 스코프**(모듈 가드 ↔ 함수 안 import)면 소스 줄 순서를 따지지 않는다. 다만 그 함수가
      **모듈 최상위에서 호출**된다면 그 호출이 실제 실행 시점이므로 **가드가 첫 호출보다 앞**이어야
      한다(2차 BLOCK-2: `load()` 를 먼저 부르고 가드를 뒤에 두면 실제로는 실패하는데 PASS 했다).
    - 가드와 import 사이에 그 가드를 **철회**하는 조작이 있으면 무효다.
    """
    lineno, _mod, chain, _mo = imp
    own = sorted(chain)[-1] if chain else 0
    guards, destroys = info["guards"], info["destroys"]
    # 이 import 를 담은 함수가 모듈 최상위에서 불리는 가장 이른 시점
    first_call = None
    nm = info["scope_name"].get(own)
    if nm and info["module_calls"].get(nm):
        first_call = min(info["module_calls"][nm])
    for gline, gscope in guards:
        if gscope not in chain:
            continue
        if gscope == own:
            if not (gline < lineno):
                continue
            lo, hi = gline, lineno
        else:
            if first_call is not None and not (gline < first_call):
                continue
            lo, hi = gline, (first_call if first_call is not None else float("inf"))
        if not any(lo < dline < hi and dscope in chain for dline, dscope in destroys):
            return True
    return False


def analyze_dir(dirpath):
    """폴더의 `*.py` 를 검사해 {파일명: 판정} 을 돌려준다.

    판정 = {"sib", "guard", "ok_guard", "ok_order", "naked", "toplevel", "opaque", "error"}
      ok_guard = 유효한 가드가 하나라도 있는가 (+ 금지 형태가 없는가)
      ok_order = **모든** 형제 import 가 자기를 지배하는 유효한 가드 뒤에 오는가
      toplevel = 모듈 로드만으로 실행되는 형제 import(= ③ 실런타임 재현 대상)
    형제 import 도 금지 형태도 없는 파일은 결과에 넣지 않는다(가드 불요).
    """
    files = sorted(f for f in os.listdir(dirpath) if f.endswith(".py"))
    siblings = {f[:-3] for f in files}
    out = {}
    for fn in files:
        path = os.path.join(dirpath, fn)
        try:
            with open(path, "rb") as fh:
                src = fh.read().decode("utf-8")
            tree = ast.parse(src, filename=path)
        except (OSError, SyntaxError, UnicodeDecodeError) as e:
            out[fn] = {"sib": None, "guard": None, "ok_guard": False, "ok_order": None,
                       "naked": None, "toplevel": [], "opaque": [],
                       "error": "%s: %s" % (type(e).__name__, e)}
            continue
        r = collect(tree, siblings, fn[:-3])
        if not r["imports"] and not r["opaque"]:
            continue
        unprotected = [i for i in r["imports"] if not is_protected(i, r)]
        # main 전용 import 는 모듈 로드만으로는 실행되지 않는다 → ③ 대상 아님(REVISE-2)
        top = [i for i in r["imports"] if len(i[2]) == 1 and not i[3]]
        first = r["imports"][0] if r["imports"] else None
        out[fn] = {
            "sib": (first[0], first[1]) if first else None,
            "guard": r["guards"][0][0] if r["guards"] else None,
            "ok_guard": bool(r["guards"]) and not r["opaque"],
            "ok_order": (not unprotected) if r["guards"] else None,
            "naked": (unprotected[0][0], unprotected[0][1]) if unprotected else None,
            "toplevel": [(i[0], i[1]) for i in top],
            "opaque": r["opaque"],
            "error": None,
        }
    return out


# ── ③ 실런타임 재현 (자식 프로세스 격리) ────────────────────────────────────
_PROBE_CODE = (
    "import runpy, sys\n"
    "runpy.run_path(sys.argv[1], run_name='__guardprobe__')\n"
    # ★모듈 레벨 실행 후 **형제 모듈이 실제로 적재됐는지**를 보고한다. 종료코드는 이걸 증명하지
    #   못한다 — 팩 관례가 형제 import 를 try/except 로 감싸 실패를 삼키기 때문이다(실측 3/8).
    "want = [w for w in sys.argv[2].split(',') if w]\n"
    "sys.stdout.write('\\nGUARDPROBE_LOADED=' + ','.join(w for w in want if w in sys.modules) + '\\n')\n"
)
_MNF = re.compile(r"ModuleNotFoundError: No module named '([^']+)'")
_LOADED = re.compile(r"^GUARDPROBE_LOADED=(.*)$", re.M)


def runtime_probe(path, siblings, neutral_cwd, expect=()):
    """모듈 레벨만 **독립 자식 프로세스**로 실행. (ok, 상세) 반환.

    ★한 프로세스에서 순차 실행하면 앞 검체가 남긴 `sys.path`·`sys.modules` 로 뒤 검체가
      무임승차한다(적대 검증 BLOCK-3 실증). 프로세스를 나누면 그 부류가 통째로 사라진다.
    ★`cwd` 를 중립 폴더로, `PYTHONPATH` 를 제거해 Windows(`._pth`) 조건과 동치로 만든다.
    ★**정상 종료(rc=0)만 PASS 다**(2차 적대 검증 BLOCK-4). 초안은 형제 `ModuleNotFoundError` 만
      FAIL 로 보고 나머지 비정상 종료·timeout 을 "무관 실패"로 **통과**시켰다. 그러면 가드 평가
      도중 죽어 import 에 **도달조차 못 한 실행**이 "형제 import 통과"의 증거로 계상된다.
      실패 사유는 구분해 보고하되, 판정은 둘 다 FAIL 이다.
    ★★**rc=0 은 성공을 뜻하지 않는다**(3차 적대 검증 BLOCK — 실측 확증). 팩 관례가 형제 import 를
      `try/except` 로 감싸 실패를 삼킨다(③ 대상 8종 중 **3종**: javis_memory·javis_orchestra·
      javis_report). 그래서 가드가 **틀린 폴더**를 가리켜 형제가 하나도 안 실려도 rc=0 이 나오고,
      스위트는 `PASS ③ ... 형제 import 통과` 라는 **거짓 문장**을 찍었다(재현 완료).
      → `expect` 에 이 파일이 모듈 레벨에서 import 해야 할 형제를 넘기면, 자식이 실행 후
        `sys.modules` 를 보고하고 **실제 적재 여부로** 판정한다. 프록시가 아니라 측정이다.
    """
    env = dict(os.environ)
    env.pop("PYTHONPATH", None)
    expect = sorted(set(expect))
    try:
        r = subprocess.run([sys.executable, "-c", _PROBE_CODE, path, ",".join(expect)],
                           cwd=neutral_cwd, env=env, stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, timeout=120)
    except subprocess.TimeoutExpired:
        return False, "timeout — 모듈 레벨이 끝나지 않아 형제 import 도달 여부를 증명할 수 없다"
    if r.returncode != 0:
        err = r.stderr.decode("utf-8", "replace")
        m = _MNF.search(err)
        if m and m.group(1).split(".")[0] in siblings:
            return False, "No module named '%s' (형제 모듈 — D1 재발)" % m.group(1)
        tail = [l for l in err.strip().splitlines() if l.strip()]
        return False, "모듈 레벨이 rc=%d 로 실패 — 형제 import 도달 여부 증명 불가: %s" % (
            r.returncode, tail[-1][:100] if tail else "(stderr 없음)")
    # rc=0 이어도 **실제 적재**를 확인한다
    if expect:
        out = r.stdout.decode("utf-8", "replace")
        m = _LOADED.search(out)
        if not m:
            return False, "적재 보고 마커 부재 — 프로브가 끝까지 실행되지 않았다(판정 불가·fail-closed)"
        loaded = {x for x in m.group(1).split(",") if x}
        missing = [e for e in expect if e not in loaded]
        if missing:
            return False, ("rc=0 이지만 형제 %s 가 실제로 적재되지 않았다 — 예외를 삼켰을 뿐 "
                           "가드는 무효다" % ", ".join(missing))
    return True, ""


# ── ④ 셀프테스트 검체 ───────────────────────────────────────────────────────
_G = ("import os\nimport sys\n"
      "_SELF_DIR = os.path.dirname(os.path.abspath(__file__))\n"
      "if _SELF_DIR not in sys.path:\n"
      "    sys.path.append(_SELF_DIR)\n")

# (파일명, 소스, 반드시_FAIL해야_하는가, 설명)
_SPECIMENS = [
    # ── 1차 적대 검증에서 정규식판이 뚫렸던 우회 ──
    ("s_late_append.py",
     "import os\nimport sys\n_SELF_DIR = os.path.dirname(os.path.abspath(__file__))\n"
     "import javis_sib\n" + "# 채움\n" * 200 +
     "if _SELF_DIR not in sys.path:\n    sys.path.append(_SELF_DIR)\n",
     True, "대입만 앞·append 는 import 200줄 뒤"),
    ("s_comma.py", "import os, javis_sib\n", True, "콤마 결합 import"),
    ("s_dead_guard.py",
     "import os\nimport sys\nif False:\n    _S = os.path.dirname(os.path.abspath(__file__))\n"
     "    sys.path.append(_S)\nimport javis_sib\n",
     True, "if False: 죽은 가드"),
    ("s_comment_guard.py",
     "import os\nimport sys\n# sys.path.append(os.path.dirname(os.path.abspath(__file__)))\n"
     "DOC = '''sys.path.append(os.path.dirname(os.path.abspath(__file__)))'''\n"
     "import javis_sib\n",
     True, "주석·문자열 속 가드"),
    ("s_func_guard.py",
     "import os\nimport sys\ndef _ensure():\n"
     "    sys.path.append(os.path.dirname(os.path.abspath(__file__)))\nimport javis_sib\n",
     True, "정의만 되고 호출 안 되는 함수 속 가드"),
    ("s_mutant.py", "import os\nimport sys\nimport javis_sib\n", True, "가드 전면 제거"),

    # ── 2차 적대 검증 BLOCK-1: 정적 증명 불가 형태(금지) ──
    ("s_exec.py", "exec(\"import javis_sib\")\n", True, "exec 문자열 import"),
    ("s_eval.py", "eval(\"__import__('javis_sib')\")\n", True, "eval 문자열 import"),
    ("s_dunder_var.py", "name = 'javis_sib'\n__import__(name)\n", True, "__import__ 변수 인자"),
    ("s_dunder_fstring.py", "stem = 'javis'\n__import__(f'{stem}_sib')\n", True, "__import__ f-string 인자"),
    ("s_importlib_var.py", "import importlib\nname = 'javis_sib'\nimportlib.import_module(name)\n",
     True, "import_module 변수 인자"),
    ("s_importlib_const.py", "import importlib\nimportlib.import_module('javis_sib')\n",
     True, "import_module 상수 인자(가드 없음)"),
    ("s_decorator_import.py",
     "import os\nimport sys\n@__import__('javis_sib').dec\ndef target():\n    pass\n",
     True, "데코레이터 실행식 속 동적 import"),

    # ── 2차 적대 검증 BLOCK-2: 죽은/철회된 가드 ──
    ("s_false_tuple.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "if ():\n    sys.path.append(_S)\nimport javis_sib\n",
     True, "if (): 확정 거짓 분기 속 가드"),
    ("s_finally_undo.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "try:\n    sys.path.append(_S)\nfinally:\n    sys.path.remove(_S)\nimport javis_sib\n",
     True, "try/finally 로 가드 철회"),
    ("s_clear.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\nsys.path.clear()\nimport javis_sib\n",
     True, "sys.path.clear() 로 철회"),
    ("s_reassign.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\nsys.path = []\nimport javis_sib\n",
     True, "sys.path 재대입으로 철회"),

    # ── 2차 적대 검증에서 새로 뚫린 형태(전부 실제 ModuleNotFoundError 실증됨) ──
    ("s_slice_delete.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\nsys.path[:] = []\nimport javis_sib\n",
     True, "sys.path[:] = [] 슬라이스 대입 철회"),
    ("s_dunder_delitem.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\nsys.path.__delitem__(slice(None))\nimport javis_sib\n",
     True, "__delitem__ 철회(메서드 열거 우회)"),
    ("s_alias_chain_revoke.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "q = sys.path\nr = q\nq.append(_S)\nr.clear()\nimport javis_sib\n",
     True, "2단 별칭(r = q)으로 철회"),
    ("s_called_revoke_helper.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\ndef revoke():\n    sys.path.clear()\nrevoke()\nimport javis_sib\n",
     True, "철회 함수를 호출(가드만 투영하던 비대칭)"),
    ("s_helper_redefined.py",
     "import os\nimport sys\ndef ensure():\n"
     "    sys.path.append(os.path.dirname(os.path.abspath(__file__)))\n"
     "def ensure():\n    return None\nensure()\nimport javis_sib\n",
     True, "같은 이름 재정의 — 실제로 불리는 건 빈 함수"),
    ("s_outer_guard_after_call.py",
     "import os\nimport sys\ndef load():\n    import javis_sib\n    return javis_sib\n"
     "load()\nsys.path.append(os.path.dirname(os.path.abspath(__file__)))\n",
     True, "함수 호출이 모듈 가드보다 앞선다"),
    ("s_type_checking_true.py",
     "TYPE_CHECKING = True\nif TYPE_CHECKING:\n    import javis_sib\n",
     True, "TYPE_CHECKING 을 직접 True 로 바인딩"),
    ("s_getattr_exec.py",
     "import builtins\ngetattr(builtins, 'ex' + 'ec')(\"import javis_sib\")\n",
     True, "getattr(builtins,...) 로 exec 금지 판정 우회"),

    # ── 3차 적대 검증: sys.path 값이 추적 밖으로 새는 부류(전부 실제 ModuleNotFoundError 실증) ──
    ("s_esc_funcarg.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\ndef kill(p):\n    p.clear()\nkill(sys.path)\nimport javis_sib\n",
     True, "sys.path 를 함수 인자로 넘겨 철회(비적대적으로도 나올 수 있는 형태)"),
    ("s_esc_unbound.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\nlist.clear(sys.path)\nimport javis_sib\n",
     True, "언바운드 메서드 수신자로 철회"),
    ("s_esc_container.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\nbox = [sys.path]\nbox[0].clear()\nimport javis_sib\n",
     True, "컨테이너 원소로 담아 철회"),
    ("s_esc_walrus.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\n(q := sys.path).clear()\nimport javis_sib\n",
     True, "walrus 바인딩으로 철회"),
    ("s_esc_forloop.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\nfor p in (sys.path,):\n    p.clear()\nimport javis_sib\n",
     True, "for 타깃으로 철회"),
    ("s_esc_operator.py",
     "import os\nimport sys\nimport operator\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\noperator.setitem(sys.path, slice(None), [])\nimport javis_sib\n",
     True, "operator 인자로 철회"),
    ("s_esc_setattr.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\nsetattr(sys, 'path', [])\nimport javis_sib\n",
     True, "setattr(sys,...) — sys.path 를 거치지 않는 상태 전이"),
    ("s_esc_sysdict.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\nsys.__dict__['path'] = []\nimport javis_sib\n",
     True, "sys.__dict__ 우회 상태 전이"),
    ("s_esc_augassign.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path.append(_S)\nsys.path *= 0\nimport javis_sib\n",
     True, "AugAssign(`*= 0`)으로 제자리 비우기"),
    ("s_good_augadd.py",
     "import os\nimport sys\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "sys.path += [_S]\nimport javis_sib\n",
     False, "`sys.path += [...]` 는 순수 추가 — 철회 아님(양성 대조)"),

    # ── 양성 대조: 정상인데 FAIL 나면 과탐 ──
    ("s_good.py", _G + "import javis_sib\n", False, "정상 가드"),
    ("s_good_from.py", _G + "from javis_sib import thing\n", False, "정상 가드 + from-import"),
    ("s_good_infunc.py",
     "import os\nimport sys\ndef load():\n"
     "    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))\n"
     "    import javis_sib\n    return javis_sib\n",
     False, "함수 안 가드 + 같은 함수 안 import(실재: javis_compete·javis_phoenix)"),
    ("s_good_try.py",
     "import os\nimport sys\ntry:\n    _S = os.path.dirname(os.path.abspath(__file__))\n"
     "    if _S not in sys.path:\n        sys.path.append(_S)\nexcept Exception:\n    pass\n"
     "import javis_sib\n",
     False, "try 블록 안 가드는 실행된다"),
    ("s_good_path_alias.py",
     "import os\nfrom sys import path\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "path.append(_S)\nimport javis_sib\n",
     False, "from sys import path 별칭 가드"),
    ("s_good_path_obj.py",
     "import os\nimport sys\nq = sys.path\n_S = os.path.dirname(os.path.abspath(__file__))\n"
     "q.append(_S)\nimport javis_sib\n",
     False, "q = sys.path 별칭 가드"),
    ("s_good_helper_called.py",
     "import os\nimport sys\ndef _ensure():\n"
     "    sys.path.append(os.path.dirname(os.path.abspath(__file__)))\n"
     "_ensure()\nimport javis_sib\n",
     False, "가드 헬퍼를 실제로 호출"),
    ("s_good_import_above_guard.py",
     "import os\nimport sys\ndef load():\n    import javis_sib\n    return javis_sib\n"
     "sys.path.append(os.path.dirname(os.path.abspath(__file__)))\nload()\n",
     False, "함수 내 import 소스 줄이 가드보다 위(호출은 가드 뒤)"),
    ("s_good_type_checking.py",
     "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    import javis_sib\n",
     False, "TYPE_CHECKING 은 런타임 미실행"),
    ("s_good_class_body.py",
     "import os\nimport sys\nclass C:\n"
     "    sys.path.append(os.path.dirname(os.path.abspath(__file__)))\n"
     "    import javis_sib\n",
     False, "클래스 본문은 실제로 실행된다"),
    ("s_good_class_guard_then_import.py",
     "import os\nimport sys\nclass C:\n"
     "    sys.path.append(os.path.dirname(os.path.abspath(__file__)))\n"
     "import javis_sib\n",
     False, "클래스 본문 가드가 이후 모듈 import 를 보호한다"),
    ("s_good_main_only.py",
     _G + "if __name__ == '__main__':\n    import javis_sib\n",
     False, "main 전용 import(가드 있음)"),
]


def selftest():
    """검체를 임시 폴더에 심고 스캐너가 기대대로 판정하는지 확인한다."""
    tmp = tempfile.mkdtemp(prefix="import_guard_selftest_")
    try:
        with open(os.path.join(tmp, "javis_sib.py"), "w") as fh:
            fh.write("VALUE = 1\ndef dec(f):\n    return f\n")
        for fn, src, _mf, _d in _SPECIMENS:
            with open(os.path.join(tmp, fn), "w") as fh:
                fh.write(src)
        verdicts = analyze_dir(tmp)
        for fn, _src, must_fail, desc in _SPECIMENS:
            v = verdicts.get(fn)
            if v is None:
                # 검사 대상에서 아예 빠졌다 = 가장 나쁜 우회(조용히 사라진다)
                check("④ %s" % fn, not must_fail,
                      "%s → 스캐너가 검사 대상에서 **놓쳤다**" % desc if must_fail
                      else "%s (형제 import 없음 — 정상)" % desc)
                continue
            failed = not (v["ok_guard"] and v["ok_order"])
            detail = "%s | guard=%s sib=%s opaque=%d" % (
                desc, v["guard"], v["sib"][0] if v["sib"] else None, len(v["opaque"]))
            if must_fail:
                check("④ %s 우회 차단" % fn, failed, detail)
            else:
                check("④ %s 오탐 없음" % fn, not failed, detail)
        # main 전용 import 는 ③ 대상에서 빠져야 한다(REVISE-2)
        v = verdicts.get("s_good_main_only.py")
        check("④ main 전용 import 는 ③ 대상 제외", bool(v) and v["toplevel"] == [],
              "toplevel=%s" % (v and v["toplevel"]))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main(argv):
    only_self = "--selftest" in argv[1:]

    selftest()  # ④ 스캐너 자체가 먼저 신뢰돼야 ①②의 PASS 가 의미를 갖는다
    if only_self:
        npass = sum(1 for c in _results if c)
        print("\n=== %d/%d PASS (selftest only) ===" % (npass, len(_results)))
        return 0 if npass == len(_results) else 1

    verdicts = analyze_dir(BIN)
    for fn in sorted(verdicts):
        v = verdicts[fn]
        if v["error"]:
            check("① %s 파싱" % fn, False, v["error"])
            continue
        if v["opaque"]:
            check("① %s 정적 증명 가능 형태" % fn, False,
                  "%d행: %s" % (v["opaque"][0][0], v["opaque"][0][1]))
            continue
        sline, smod = v["sib"]
        check("① %s 경로 가드 존재" % fn, v["ok_guard"], "형제=%s(%d행)" % (smod, sline))
        if not v["ok_guard"]:
            continue
        check(
            "② %s 모든 형제 import 가 유효 가드 뒤" % fn,
            v["ok_order"],
            ("guard@%d행 vs 첫 import@%d행" % (v["guard"], sline)) if v["ok_order"]
            else ("무방비 import: %s@%d행 (가드 최선 %d행)" % (v["naked"][1], v["naked"][0], v["guard"])),
        )

    check("검사 대상이 존재한다(스캐너 자체 회귀 방지)", len(verdicts) >= 10,
          "checked=%d" % len(verdicts))

    # ③ 실패 조건 재현 — 스크립트 폴더가 sys.path 에 없는 상태에서 모듈 최상위 형제 import 실행.
    #    ★파일마다 독립 자식 프로세스 · 중립 cwd · PYTHONPATH 제거(BLOCK-3).
    #    ★이 스크립트를 Windows 잡에서 **동봉 embeddable python 으로** 그대로 돌리면(R2)
    #      이 ③ 이 모사가 아니라 실물 런타임 재현이 된다.
    sib_names = {f[:-3] for f in os.listdir(BIN) if f.endswith(".py")}
    neutral = tempfile.mkdtemp(prefix="import_guard_probe_cwd_")
    probed = 0
    try:
        for fn in sorted(verdicts):
            v = verdicts[fn]
            if v["error"] or v["opaque"] or not v["toplevel"]:
                continue  # 함수 안·main 전용 import 만 있는 파일은 로드만으로 재현되지 않는다
            probed += 1
            expect = {mod.split(".")[0] for _ln, mod in v["toplevel"]}
            ok, detail = runtime_probe(os.path.join(BIN, fn), sib_names, neutral, expect)
            check("③ %s: 폴더 미등재 조건에서 형제 %s 실제 적재" % (fn, ",".join(sorted(expect))),
                  ok, detail)
    finally:
        shutil.rmtree(neutral, ignore_errors=True)
    check("③ 실런타임 재현 대상이 존재한다", probed >= 5, "probed=%d" % probed)

    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
