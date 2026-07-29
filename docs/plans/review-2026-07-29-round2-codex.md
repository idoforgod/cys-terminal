# 0.14.4 에이전트 매처 — 스캐너 2차 적대 재검증 (codex)

```json
{
  "verdict": "BLOCK",
  "justification": "기준선 70/70 및 selftest 30/30은 통과하지만, 새 규칙을 일반화한 임시 검체에서 형제 import가 실제 실패하는데 스캐너가 PASS/OMIT하는 미검출 8종과 실제 성공 코드를 FAIL시키는 오탐 1종을 재현했다. 또한 runtime_probe가 가드 실행 자체의 예외를 성공으로 반환해 ③의 실런타임 증거가 거짓 양성이 된다.",
  "evidence": [
    {
      "claim": "slice 대입·__delitem__·2단 별칭·호출된 철회 함수가 가드를 제거해 실제 ModuleNotFoundError가 나지만 정적 판정은 PASS다.",
      "ref": "cysjavis-pack/bin/tests/test_import_guard.py:238",
      "verified": true
    },
    {
      "claim": "동일 이름 헬퍼 재정의와 바깥 스코프의 사후 가드가 실제 실패 import를 보호된 것으로 오인시킨다.",
      "ref": "cysjavis-pack/bin/tests/test_import_guard.py:342",
      "verified": true
    },
    {
      "claim": "TYPE_CHECKING=True 및 getattr(builtins, 'ex'+'ec') 경로는 실제 형제 import 실패를 실행하지만 분석 결과에서 파일 자체가 OMIT된다.",
      "ref": "cysjavis-pack/bin/tests/test_import_guard.py:144",
      "verified": true
    },
    {
      "claim": "sys.path.pop(0)은 추가한 끝 원소를 보존해 실제 import가 성공하지만 모든 pop을 철회로 보아 FAIL한다.",
      "ref": "cysjavis-pack/bin/tests/test_import_guard.py:315",
      "verified": true
    },
    {
      "claim": "정적으로 가드로 인정된 식이 ZeroDivisionError로 실행되지 않아도 runtime_probe는 rc=1을 무관 실패로 무시하고 true를 반환한다.",
      "ref": "cysjavis-pack/bin/tests/test_import_guard.py:448",
      "verified": true
    }
  ],
  "issues": [
    "BLOCK-1: destroy 및 별칭 추적이 구문 형태별 부분 목록이라 동치인 경로 철회를 놓친다.",
    "BLOCK-2: 헬퍼 이름 단일 매핑과 스코프 교차 줄 순서 완화가 실행 순서와 실제 바인딩을 모델링하지 못한다.",
    "BLOCK-3: TYPE_CHECKING 및 opaque 제외가 값·호출 대상을 확인하지 않아 실행 가능한 import를 분석 대상에서 제거한다.",
    "BLOCK-4: runtime_probe가 형제 ModuleNotFoundError 이외의 비정상 종료와 timeout을 PASS로 반환한다.",
    "REVISE-1: 모든 pop을 철회로 간주해 정상 코드를 실패시킨다."
  ],
  "missing": [
    "각 미검출·오탐을 고정한 회귀 검체",
    "별칭/재정의/호출 순서를 반영하는 흐름 민감 분석 또는 허용 구문을 좁힌 명시적 fail-closed 계약",
    "runtime_probe의 정상 종료 필수 조건과 비정상 종료 별도 FAIL 판정"
  ]
}
```

## 1. 기준선

실행:

```text
python3 cysjavis-pack/bin/tests/test_import_guard.py
python3 cysjavis-pack/bin/tests/test_import_guard.py --selftest
```

실제 마지막 출력:

```text
=== 70/70 PASS ===
BASE_EXIT=0
=== 30/30 PASS (selftest only) ===
SELFTEST_EXIT=0
```

즉 1차 지적을 이름 그대로 담은 검체는 통과한다. 아래 결과는 그 수정 규칙을 동치 코드로 일반화했을 때 새로 드러난 구멍이다.

## 2. 재현 방법

검체는 작업트리 밖 임시 디렉터리에 `sibling.py`와 각 표본을 만들었다. 정적 판정과 중립 `cwd`·`PYTHONPATH` 제거 자식 프로세스의 직접 실행, 제품 `runtime_probe()` 결과를 같은 실행에서 수집했다.

```text
python3 /private/tmp/r2_import_guard_adversarial.py
```

핵심 실제 출력:

```text
slice_delete         scanner=PASS direct_rc=1 direct_tail=ModuleNotFoundError: No module named 'sibling' runtime_probe_ok=false
dunder_delitem       scanner=PASS direct_rc=1 direct_tail=ModuleNotFoundError: No module named 'sibling' runtime_probe_ok=false
alias_chain_revoke   scanner=PASS direct_rc=1 direct_tail=ModuleNotFoundError: No module named 'sibling' runtime_probe_ok=false
called_revoke_helper scanner=PASS direct_rc=1 direct_tail=ModuleNotFoundError: No module named 'sibling' runtime_probe_ok=false
helper_redefined     scanner=PASS direct_rc=1 direct_tail=ModuleNotFoundError: No module named 'sibling' runtime_probe_ok=false
outer_guard_after_call scanner=PASS direct_rc=1 direct_tail=ModuleNotFoundError: No module named 'sibling' runtime_probe_ok=false
type_checking_true   scanner=OMIT direct_rc=1 direct_tail=ModuleNotFoundError: No module named 'sibling' runtime_probe_ok=false
getattr_exec         scanner=OMIT direct_rc=1 direct_tail=ModuleNotFoundError: No module named 'sibling' runtime_probe_ok=false
normal_pop_zero      scanner=FAIL direct_rc=0 runtime_probe_ok=true
probe_guard_crashes  scanner=PASS direct_rc=1 direct_tail=ZeroDivisionError: division by zero runtime_probe_ok=true
runtime_probe_detail=(무관 실패 무시: rc=1 ZeroDivisionError: division by zero)
```

위 텍스트는 JSON 원출력의 필드를 한 줄씩 옮긴 것이다. 원출력에서 각 PASS 건은 `ok_guard:true, ok_order:true`, OMIT 건은 `analysis:null`, 오탐은 `ok_order:false`였다.

## 3. BLOCK — destroy/별칭 철회 미검출

### 3.1 slice 대입 및 `__delitem__`

두 검체 모두 먼저 `sys.path.append(P)`하고 각각 다음 동작으로 목록을 비운 뒤 `import sibling`을 수행했다.

```python
sys.path[:] = []
sys.path.__delitem__(slice(None))
```

직접 실행은 모두 형제 `ModuleNotFoundError`, 정적 분석은 모두 PASS였다. 대입 철회는 target 자체가 `sys.path`인 경우만 검사한다(`test_import_guard.py:279-286`). 메서드 철회도 고정된 `_DESTROY_METHODS` 호출만 처리한다(`:310-320`). 두 형태 모두 같은 상태 전이를 만들지만 이벤트가 없다.

### 3.2 2단 별칭과 호출된 철회 함수

```python
q = sys.path
r = q
q.append(P)
r.clear()
import sibling
```

`q = sys.path`만 별칭으로 수집하고 `r = q`는 전파하지 않는다(`:238-250`). 따라서 `q.append`는 가드, `r.clear`는 비사건이 되어 PASS한다.

다음 검체도 PASS 후 실제 실패했다.

```python
sys.path.append(P)
def revoke():
    sys.path.clear()
revoke()
import sibling
```

가드 헬퍼 호출만 호출 지점으로 투영하고 철회 헬퍼 호출은 투영하지 않는다(`:347-351`). 정적 이벤트와 실제 실행 효과가 비대칭이다.

## 4. BLOCK — 이름 기반 헬퍼와 스코프 완화가 실제 실행 순서를 뒤집음

### 4.1 같은 이름 재정의

```python
def ensure():
    sys.path.append(P)
def ensure():
    return None
ensure()
import sibling
```

실제 호출 대상은 두 번째 함수라 import가 실패한다. 그러나 `func_scope.setdefault(fn_node.name, key)`가 첫 정의만 보존하고(`:340-343`), 이름만 같은 호출을 가드로 승격해(`:347-351`) PASS했다. 조건부 재정의·클래스 메서드 이름 충돌도 같은 모델 결함의 변형이다.

### 4.2 함수 호출 뒤에 실행되는 모듈 가드

```python
def load():
    import sibling
load()
sys.path.append(P)
```

직접 실행은 가드 도달 전 실패하지만 스캐너는 PASS했다. 바깥 스코프 가드에 줄 순서를 적용하지 않고(`:363-377`) “모듈 가드는 함수 호출보다 먼저 실행된다”는 가정을 무조건 사용하기 때문이다. 함수 호출이 가드보다 앞설 수 있으므로 이 완화는 제어 흐름 증명 없이 안전하지 않다.

## 5. BLOCK — 실행 가능한 코드를 분석 대상에서 OMIT

### 5.1 `TYPE_CHECKING = True`

```python
TYPE_CHECKING = True
if TYPE_CHECKING:
    import sibling
```

직접 실행은 형제 import 실패, 분석 결과는 `null`이었다. `_is_type_checking()`은 바인딩이나 정적 값을 보지 않고 이름/attribute 문자열만 찾는다(`:144-151`). 런타임 거짓이라는 전제가 증명되지 않았다.

### 5.2 `getattr`로 얻은 `exec`

```python
import builtins
getattr(builtins, "ex" + "ec")("import sibling")
```

직접 실행은 형제 import 실패, 분석 결과는 `null`이었다. opaque 판정은 호출 대상이 직접 `Name`인 `exec/eval` 또는 알려진 importer 이름인 경우만 다룬다(`:295-308`). 이 결과가 OMIT되므로 fail-closed 계약이 우회된다.

## 6. REVISE — 정상 `pop(0)` 오탐

```python
sys.path.append(P)
sys.path.pop(0)
import sibling
```

추가한 `P`는 목록 끝에 있고 `pop(0)`은 기존 첫 항목을 제거하므로 직접 실행 `rc=0`이었다. 스캐너는 인덱스를 보지 않고 모든 `pop`을 철회로 처리해 `ok_order:false`를 반환했다(`:315-320`). “정상 팩 코드 형태”를 허용해야 한다면 인덱스·추가 위치를 추적하거나, 지원 구문 계약을 명확히 좁혀야 한다.

## 7. BLOCK — ③ 런타임 프로브의 거짓 PASS

```python
sys.path.append((1 / 0, P)[1])
import sibling
```

정적 `_derives_from_file` 판정은 이를 가드로 인정해 PASS했다. 실제 자식 프로세스는 가드 평가 중 `ZeroDivisionError`, `rc=1`이다. 그런데 제품 `runtime_probe()` 결과는 다음과 같았다.

```text
runtime_probe_ok=true
runtime_probe_detail=(무관 실패 무시: rc=1 ZeroDivisionError: division by zero)
```

현재 구현은 형제 이름이 정규식으로 잡힌 `ModuleNotFoundError`만 FAIL하고, timeout과 그 밖의 모든 비정상 종료를 true로 반환한다(`:448-457`). 따라서 “폴더 미등재 조건에서 형제 import 통과”라는 ③의 PASS가 실제로는 import에 도달하지 못한 실행을 포함한다. 최소한 `rc != 0`은 ③ PASS가 될 수 없으며, 필요하면 “형제 경로 실패”와 “검체 자체 실패”를 서로 다른 실패 사유로 보고해야 한다.

## 8. 판정

**BLOCK.** 1차의 구체 검체는 회귀 목록에 편입됐지만, 새 분석 모델은 상태 전이·바인딩·호출 순서를 충분히 증명하지 않고 PASS한다. 특히 실제 `ModuleNotFoundError`와 런타임 프로브 거짓 PASS가 함께 남아 있어 이 게이트를 Windows embeddable import 보장의 출고 증거로 사용할 수 없다.

