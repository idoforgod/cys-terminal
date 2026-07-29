# R1 적대 검증 — `test_import_guard.py` AST 재작성

## 총판정: BLOCK

이 상태로 커밋하면 안 된다. 현재 52/52와 셀프테스트 12/12는 초록이지만, 임시 검체를 실제 실행한 결과 형제 bare import가 `ModuleNotFoundError`로 죽는 9개 형태를 스캐너가 누락 또는 PASS 처리했다. 반대로 같은 런타임에서 정상 동작하는 6개 형태를 FAIL 처리했다. 이 스위트가 Windows embeddable의 조용한 fail-open을 막는 유일한 게이트라는 전제에서, 검출 누락은 차단 결함이다.

검증 대상은 지시서가 지정한 `docs/plans/2026-07-29-win-two-defects-plan.md` §18과 `cysjavis-pack/bin/tests/test_import_guard.py`로 한정했다. 모든 공격 검체는 `tempfile.mkdtemp()` 아래에 만들고 종료 시 삭제했다. 대상 스크립트 및 팩 파일은 수정하지 않았다.

## 기준선

실행:

```text
python3 cysjavis-pack/bin/tests/test_import_guard.py
python3 cysjavis-pack/bin/tests/test_import_guard.py --selftest
```

실제 출력의 종결부:

```text
PASS ③ 실런타임 재현 대상이 존재한다 | probed=8

=== 52/52 PASS ===
BASE_EXIT=0
...
=== 12/12 PASS (selftest only) ===
SELF_EXIT=0
```

즉, 아래 공격은 기존 초록 상태에서 재현됐다.

## BLOCK-1 — 동적 import 6형태를 검사 대상에서 통째로 누락한다

근거 코드: 동적 import는 첫 인자가 문자열 `ast.Constant`일 때만 수집한다(`test_import_guard.py:184-191`). 함수·클래스 정의의 decorator/default 표현식은 `_split_scope()`가 정의 노드 전체를 별도 scope로 빼고 body만 방문하므로 검사하지 않는다(`:104-105`, `:193-195`). `exec`와 `eval`은 별도 규칙이 없다.

재현 명령은 임시 폴더에 `javis_sib.py`와 아래 검체를 쓴 뒤 `T.analyze_dir(tmp)` 및 `runpy.run_path(..., run_name="__main__")`를 각각 호출했다.

실제 출력:

```text
bypass_exec.py | SCAN OMITTED | RUNTIME ModuleNotFoundError:javis_sib:No module named 'javis_sib'
bypass_eval.py | SCAN OMITTED | RUNTIME ModuleNotFoundError:javis_sib:No module named 'javis_sib'
bypass_dunder_var.py | SCAN OMITTED | RUNTIME ModuleNotFoundError:javis_sib:No module named 'javis_sib'
bypass_dunder_fstring.py | SCAN OMITTED | RUNTIME ModuleNotFoundError:javis_sib:No module named 'javis_sib'
bypass_importlib_var.py | SCAN OMITTED | RUNTIME ModuleNotFoundError:javis_sib:No module named 'javis_sib'
bypass_decorator.py | SCAN OMITTED | RUNTIME ModuleNotFoundError:javis_sib:No module named 'javis_sib'
```

검체의 핵심:

```python
exec("import javis_sib")
eval("__import__('javis_sib')")
name = "javis_sib"; __import__(name)
stem = "javis"; __import__(f"{stem}_sib")
name = "javis_sib"; importlib.import_module(name)

@__import__("javis_sib").dec
def target(): ...
```

`analyze_dir()`는 imports가 비면 파일을 결과에서 제외한다(`test_import_guard.py:242-244`). 따라서 이들은 “FAIL”이 아니라 검사 대상 0건으로 조용히 사라진다.

필수 수정:

- `exec`/`eval` 문자열의 import를 지원하거나, 팩 `bin/*.py`에서 import를 포함할 수 있는 `exec`/`eval`을 fail-closed로 금지한다.
- `__import__`/`import_module`의 비상수 첫 인자는 안전함을 증명할 수 없으므로 최소한 fail-closed 판정한다.
- decorator, 함수 기본값, annotation, class base/metaclass 등 정의 시 실행되는 식을 body와 별도로 방문한다.
- “형제 import가 없는 파일은 결과 제외”와 별도로 “해석 불가능한 동적 import가 있는 파일”을 실패 결과에 넣는다.

## BLOCK-2 — 확정 거짓 분기와 경로 가드 철회를 PASS 처리한다

근거 코드:

- `_static_truth()`는 `ast.Constant`만 평가한다(`test_import_guard.py:83-90`). 빈 tuple `()`은 런타임에서 확정 거짓이지만 `ast.Tuple`이므로 unknown이 되고, `_split_scope()`는 거짓 body와 else를 모두 실행 가능 경로로 합친다(`:106-111`).
- `try`, handler, else, finally를 제어 흐름 없이 한 목록으로 평탄화한다(`:112-117`).
- `is_protected()`는 같은/외부 scope의 앞줄 가드 존재만 보고, 이후 `remove`/`clear`/재대입으로 가드가 철회됐는지 보지 않는다(`:209-216`).

실제 출력:

```text
bypass_false_tuple.py | SCAN guard=True order=True naked=None top=[(5, 'javis_sib')] err=None | RUNTIME ModuleNotFoundError:javis_sib:No module named 'javis_sib'
bypass_finally_undo.py | SCAN guard=True order=True naked=None top=[(7, 'javis_sib')] err=None | RUNTIME ModuleNotFoundError:javis_sib:No module named 'javis_sib'
bypass_clear.py | SCAN guard=True order=True naked=None top=[(5, 'javis_sib')] err=None | RUNTIME ModuleNotFoundError:javis_sib:No module named 'javis_sib'
```

검체의 핵심:

```python
if ():
    sys.path.append(path_from_file)
import javis_sib

try:
    sys.path.append(path_from_file)
finally:
    sys.path.remove(path_from_file)
import javis_sib

sys.path.append(path_from_file)
sys.path.clear()
import javis_sib
```

필수 수정:

- 리터럴 container 및 안전하게 평가 가능한 unary/bool 표현식의 정적 진릿값을 처리한다. 임의 `eval`은 금지한다.
- 단순 “앞줄+scope”가 아니라 import까지 모든 도달 경로에서 가드가 살아 있는지 보는 최소 CFG/dataflow가 필요하다.
- 최소한 `sys.path.remove/pop/clear`, `sys.path =`, alias를 통한 destructive mutation, `finally` 철회를 import 전 invalidate해야 한다.

## REVISE-1 — 정상 가드 및 비실행 import를 거짓 FAIL 처리한다

실제 출력:

```text
fp_from_path_alias.py | SCAN guard=False order=None naked=(5, 'javis_sib') top=[(5, 'javis_sib')] err=None | RUNTIME OK
fp_path_object_alias.py | SCAN guard=False order=None naked=(5, 'javis_sib') top=[(5, 'javis_sib')] err=None | RUNTIME OK
fp_helper_called.py | SCAN guard=True order=False naked=(5, 'javis_sib') top=[(5, 'javis_sib')] err=None | RUNTIME OK
fp_import_source_before_execution.py | SCAN guard=True order=False naked=(3, 'javis_sib') top=[] err=None | RUNTIME OK
fp_type_checking.py | SCAN guard=False order=None naked=(3, 'javis_sib') top=[(3, 'javis_sib')] err=None | RUNTIME OK
fp_decorator_guard.py | SCAN guard=False order=None naked=(6, 'javis_sib') top=[] err=None | RUNTIME OK
```

반례는 각각 다음 계약을 깨뜨린다.

- `from sys import path; path.append(...)`
- `q = sys.path; q.append(...)`
- `_ensure_path()` 정의 후 모듈 최상위에서 즉시 호출
- 함수 본문의 import 소스 줄은 가드보다 위지만, 호출은 가드 뒤
- `if TYPE_CHECKING:`의 런타임 비실행 import
- decorator 인자에서 경로를 추가한 뒤 실행되는 class body import

`is_protected()`의 소스 줄 비교는 실행 순서가 아니다. helper 호출과 decorator 평가 순서를 모델링하지 않는 한 정상 코드를 막는다. 정책상 alias/helper를 금지할 수는 있지만, 그 경우 “무가드 형제 import”로 오진하지 말고 금지 문법을 별도 오류로 보고해야 한다. `TYPE_CHECKING`은 런타임 게이트 대상에서 제외해야 한다.

## BLOCK-3 — 실런타임 재현은 검체 간 격리되지 않아 우연 통과한다

근거 코드:

- 각 probe 전에는 형제 모듈 캐시를 비우지 않고, probe 후에만 `sys.modules`를 정리한다.
- `sys.path.remove(BIN)`은 동일 경로가 여러 번 있으면 한 건만 제거한다.
- 다음 probe 전 `BIN not in sys.path`를 다시 단언하지 않는다.

동일한 finally 로직을 임시 폴더에 적용한 실제 출력:

```text
a_double.py RESULT OK TMP_PATH_COUNT_AFTER_FINALLY 1
b_deadguard.py RESULT OK TMP_PATH_COUNT_AFTER_FINALLY 0
b_deadguard.py ISOLATED_RESULT ModuleNotFoundError:No module named 'javis_sib'
c_noguard.py PRESEEDED_MODULE_RESULT OK
```

`a_double.py`가 경로를 두 번 추가하자 finally가 한 번만 제거했고, 독립 실행하면 실패하는 `b_deadguard.py`가 앞 검체의 잔여 경로로 통과했다. 또한 첫 probe 전에 `sys.modules["javis_sib"]`를 심으면 무가드 import가 성공했다.

필수 수정:

- 각 probe **전후**에 관련 모듈 캐시를 정리한다.
- 각 probe 전후 `sys.path` 전체를 원본 snapshot으로 복원한다. 단일 `remove`는 격리가 아니다.
- 각 probe 직전에 `BIN not in sys.path`를 검증한다.
- 가능하면 probe마다 새 subprocess를 사용해 `sys.modules`, import hooks, `sys.meta_path`, `sys.path_importer_cache`까지 격리한다.

## REVISE-2 — `run_name="__guardprobe__"`는 `__main__` 블록을 실행하지 않는다

임시 검체:

```python
if __name__ == "__main__":
    sys.path.append(path_from_file)
    import javis_sib
```

실제 출력:

```text
main_only.py | SCAN guard=True order=True naked=None top=[(5, 'javis_sib')] err=None | RUNTIME OK
```

정적 수집기는 `if __name__ == "__main__":`을 unknown 분기로 평탄화해 import를 `toplevel`로 표시하지만, ③은 `__guardprobe__`로 실행하므로 그 블록을 전혀 실행하지 않는다. 따라서 “toplevel이면 로드만 해도 실행된다”는 `test_import_guard.py:246-248` 설명과 실제 probe가 불일치한다.

현재 팩에 실제 영향 파일이 있는지 AST로 전수 집계한 출력:

```text
MAIN_ONLY_SIBLING_BLOCKS []
COUNT 0
```

따라서 현행 15개 파일의 커버리지 손실은 0건이지만, `toplevel` 분류와 실행기가 서로 다른 의미를 쓰는 잠복 결함이다. `__main__` 분기를 runtime probe 대상에서 제외해 정적 게이트만 적용하거나, 부작용을 격리한 subprocess에서 `run_name="__main__"` 대조를 별도로 수행해야 한다.

## REVISE-3 — 셀프테스트는 이미 처리하는 12종만 확인한다

`_SPECIMENS`는 상수 문자열 동적 import, `if False`, 미호출 함수, 정상 직접 `sys.path.append/insert` 중심이다(`test_import_guard.py:271` 이후). 이번 공격에서 깨진 다음 축이 없다.

- `exec`/`eval`
- 변수 및 f-string 동적 import
- decorator/default 실행식
- truthy/falsy 상수 표현식
- `try/finally` 가드 철회
- `sys.path.clear/remove/reassign`
- alias와 호출된 helper의 정상 가드
- `TYPE_CHECKING`
- probe 간 `sys.path`/`sys.modules` 오염
- `__main__` 실행 의미 차이

그러므로 12/12는 스캐너가 이미 잡도록 작성된 검체의 자기 확인일 뿐, 공격 과제의 대표 표본이 아니다.

## ACCEPT — 공격했지만 버틴 범위

### 클래스 본문 내부 직접 가드

실제 출력:

```text
ok_class_body.py | SCAN guard=True order=True naked=None top=[] err=None | RUNTIME OK
```

클래스 body를 별도 scope로 처리한 현재 규칙은 클래스 내부에서 직접 경로를 추가하고 형제를 import하는 형태를 올바르게 인정했다. 단, 주석의 “클래스 본문은 정의 시 실행되지 않는다”(`test_import_guard.py:96`)는 Python 의미상 거짓이다. 클래스 body는 class statement 실행 중 실행된다.

### `spec_from_file_location`

실제 출력:

```text
ok_spec_location.py | SCAN OMITTED | RUNTIME OK
```

명시적 파일 경로로 형제를 로드하는 이 형태는 스크립트 폴더의 `sys.path` 등재가 필요 없으므로, 검사 대상에서 빠져도 본 결함을 재현하지 않았다. 이 후보는 공격 실패다.

## 결론

정규식판의 네 우회는 막았지만, AST판은 “문법 노드 존재+소스 줄”을 런타임 지배로 오인한다. 특히 동적 import 누락과 죽은 가드/철회된 가드 PASS는 Windows embeddable에서 다시 조용한 `ModuleNotFoundError`를 허용한다. BLOCK-1~3을 수정하고 위 검체를 셀프테스트에 추가한 뒤 재검증해야 한다.

