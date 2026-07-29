# 3차 종합 검증 보고서 — 팩 형제 import 가드 게이트

대상: `cysjavis-pack/bin/tests/test_import_guard.py` · `.github/workflows/ci-branch.yml`(레인 게이트) · `.github/workflows/windows-build.yml`(T6)
브랜치: `release/0.14-quality-line` (미커밋 작업트리) · 검증일 2026-07-29

---

## 0. 총판정: **BLOCK**

확증 35건 (BLOCK 12 · REVISE 19 · NIT 4) · 기각 3건.

BLOCK 판정의 결정적 근거는 개별 우회 검체가 아니라 **엔드투엔드 게이트 판정**이다. 아래 셋은 전부 실물 팩 사본에서 재현됐다.

| 조건 | 게이트 판정 | 실제 실행 |
|---|---|---|
| `kill(sys.path)` 1파일 투입 (D1-F1) | `=== 80/80 PASS ===` exit 0 | `ModuleNotFoundError: No module named 'javis_scrub'` |
| 데코레이터 형태 신규 스크립트 (D3-1) | `PASS ①` `PASS ②` `=== 80/80 PASS ===` exit 0 | rc=1 `ModuleNotFoundError` |
| **실물 3파일 가드에 한 토큰 오타** (T6-1) | `=== 78/78 PASS ===` exit 0 | 로드된 형제 모듈 **0개** |

세 번째가 가장 무겁다. 합성 검체가 아니라 **출하 중인 `javis_memory.py`·`javis_orchestra.py`·`javis_report.py` 세 파일**에서, `dirname(abspath(__file__))` → `dirname(dirname(abspath(__file__)))` 라는 흔한 오타 하나로 ①②③ 전 계층이 통과한다. 이 게이트가 존재하는 이유가 정확히 그 시나리오다.

**현재 살아있는 D1 인스턴스는 0건이다** — bin/ 76개 전부 중립 cwd·PYTHONPATH 제거 조건에서 rc=0 이다. 문제는 게이트가 지금 거짓말을 하고 있다는 게 아니라, **그 상태를 유지시키지 못한다**는 것이다.

---

## 1. 원리 판정 — 사례를 고쳤나, 원리를 닫았나

이번 라운드의 축이다. 2차 수정 7건 각각을 판정한다.

| # | 2차 수정 | 부류 판정 | 근거 |
|---|---|---|---|
| 1 | 파괴 allowlist 반전 | **미봉** | 메서드 축만 뒤집힘. 객체 동일성·문 형태 축은 여전히 구문 열거 + fail-open |
| 2 | `sys.path` 별칭 고정점 | **미봉** | 한 자료형에만 적용. 호출가능 객체엔 미적용. 스코프 무시 |
| 3 | 헬퍼 대칭 투영 | **미봉** | 가드↔철회 축만 대칭. 바인딩 축은 `def` 하나 |
| 4 | `first_call` 순서 | **미봉** | `Name(...)` 구문 1개만 셈 → 한 겹 간접이면 규칙 소멸 |
| 5 | TYPE_CHECKING / builtins opaque | **부분 성립** | 지정 표현은 버팀. 주변 fail-closed 원칙은 아는 이름 4개에만 |
| 6 | ③ rc=0 만 PASS | **반쪽** | 반대 방향(rc=0≠성공)이 실물 3/8 파일에서 이미 열림 |
| 7 | `_step_ranges()` | **미봉** | 표현 1개 수정. 판정기·결정자·레인 배선 전부 그대로 |

### 1-1. 파괴 allowlist 반전 → 부류 안 닫힘

반전이 **분석기가 이미 sys.path 라고 인식한 객체**에만 걸린다. 그 인식(`_path_aliases` 289-313, `_is_path_obj` 316-320)은 `from sys import path` + `Assign→Name` 두 형태뿐이다. 그 밖의 모든 바인딩 채널로 값이 새면 **비사건**이 된다 — walrus·튜플 언패킹·리스트 원소·함수 인자·for 타깃·컴프리헨션 변수·언바운드 메서드 수신자·`operator.*` 8/8 우회(D1-F1). 문 형태 축도 같다: `scan()`(370-378)은 `Assign`/`Delete` 두 문형만 보므로 `sys.path *= 0`·`setattr(sys,'path',[])`·`sys.__dict__['path']=[]` 가 통과한다(D1-F2).

같은 구멍이 **대칭으로 과탐도 만든다**: `sys.path += [_S]` 는 파이썬에서 가장 흔한 리스트 확장 idiom 인데 가드로 인식조차 안 돼 `ok_guard=False`(rc=0 정상 동작인데 FAIL). 이건 계약이 의도적으로 금지한 `pop(0)` 과탐과 **다른 건**이다 — 저건 계약 위반 코드를 벌하는 것이고 이건 **가드 자체를 못 알아보는 인식 누락**이다.

> **처방 P1 — 값 격자 + ⊤ fail-closed.**
> 추상 도메인 `{PATHOBJ, SELFDIR, OTHER, UNKNOWN(⊤)}` 을 도입하고 **모든** 바인딩 채널에서 값을 전파한다: `Assign`(Name/Tuple/List/Starred 타깃) · `AnnAssign` · `AugAssign` · `NamedExpr` · `For`/`AsyncFor` 타깃 · comprehension generator · `with ... as` · `except ... as` · 함수 파라미터(호출 지점 인자별) · `global`/`nonlocal`.
> 핵심 규칙 한 줄: **`PATHOBJ` 값이 분석기가 모델링하지 않는 위치(함수 인자·subscript store·컨테이너 리터럴 원소·컴프리헨션·언바운드 메서드 수신자)로 흐르는 순간 `opaque` 로 기록하고 파일을 금지한다.** 형태를 열거하지 않고 8/8 우회를 한 번에 막는다.
>
> **처방 P2 — 상태 전이 문형 allowlist.**
> `scan()` 에 `ast.AugAssign`·`ast.AnnAssign` 추가. `AugAssign` 은 `op` 가 `Add` 이고 피연산자가 `SELFDIR` 를 담으면 **가드**, 그 외 전부 철회 → `sys.path += [_S]` 과탐과 `sys.path *= 0` 미검출이 동시에 닫힌다. 베이스가 `sys` 모듈로 해석되는 모든 attribute-store·subscript-store(`setattr(sys,...)` · `sys.__dict__[...]` · `vars(sys)[...]` · `delattr`)는 철회. 그 밖에 `sys` 를 store/call 위치에서 만지는 형태는 `opaque`.

### 1-2. 별칭 고정점 → 한 자료형에만

고정점 전파 자체는 잘 작동한다(`r = q = sys.path` 추적). 문제는 **원리가 아니라 하나의 변수에 대한 사례로 구현됐다**는 것이다. 같은 문제 — "값이 아니라 이름에 걸려 있다" — 가 호출가능 객체에 그대로 남아 `e = exec` · `from importlib import import_module as im` · `im = importlib.import_module` · `_i = __import__` · `getattr(importlib,'import_module')` · `__builtins__["exec"]` · `globals()["__builtins__"]` 7형태가 전부 무검사다(D4-2). 게다가 `_path_aliases` 는 `ast.walk` 로 파일 전역 이름 집합을 만들어 **스코프를 무시**하므로, `path` 라는 파라미터 이름 하나로 `path.strip()`(str 메서드)이 sys.path 철회로 계상된다(D1-F6).

> **처방 P4a — 이름이 아니라 바인딩.** 스코프별 호출가능 바인딩 환경을 만든다: 이름 → `{def 노드}` | `{기타}` | `⊤`. `def` 이외의 재바인딩(Assign·AnnAssign·lambda·import-as·for 타깃·`globals()[...]=`·`setattr(mod,...)`·비항등 데코레이터)은 그 지점 이후 해당 이름을 `⊤` 로 만든다. `_path_aliases` 도 같은 환경 위에서 스코프 인식으로 재구현한다.

### 1-3. 헬퍼 대칭 투영 → 바인딩 축 미개봉

가드↔철회 축의 대칭은 이뤄졌으나 **호출 대상이 무엇인가**의 축이 `def` 문 하나로 남았다. 결과가 6종 우회(D2-1: 대입·람다·`globals()`·`functools.partial`·데코레이터 치환·`from X import f as ensure`) — 2차가 막은 것은 이 부류 중 `def` 이 두 번 나오는 **단 하나의 표현**이다.

동명 재정의 fail-closed 도 반쪽이다. `func_scope.setdefault` 가 **첫 def** 만 보존하므로 첫 정의가 무해하면 투영 루프의 두 분기를 모두 건너뛴다 — `func_dupes` 가 채워졌는데 완전히 무력하다(D2-2). 그리고 반대편에서는 그 fail-closed 를 '가드 불인정'이 아니라 **전역 철회로 격상**해서, `sys.path` 에 append 만 하는 정상 코드까지 벌한다(D2-5·D6-5 — `javis_org.py:14/18` 의 `try: fcntl / except ImportError: msvcrt` 형태가 실물에 출하 중).

> **처방 P4b — 효과 시그니처 기반 투영.**
> 이름의 후보 def 집합을 구하고:
> - 후보가 유일 → 그 def 의 효과를 투영(현행).
> - 후보가 복수인데 **효과 시그니처가 동일**(전부 guard-SELFDIR / 전부 destroy / 전부 neutral) → 어느 게 실행되든 결과가 같으므로 그 공통 효과를 투영. → 플랫폼 분기 정의(D2-5·D6-5) 과탐 소멸.
> - 후보들의 효과가 **다르다** → `opaque`(파일 금지). → D2-2 의 '첫 무해·둘째 철회'가 정확히 여기 걸린다.
> - 이름이 `⊤`(비-def 재바인딩) → 그 이름의 후보 def 중 sys.path 이벤트를 가진 것이 있으면 `opaque`, 없으면 무시. → D2-1 6종 일괄 차단.

### 1-4. `first_call` 순서 → 한 겹 간접이면 규칙 소멸

가장 취약하다. `module_calls` 적재 조건이 `isinstance(n.func, ast.Name) and len(chain)==1` 뿐이라, 모듈 로드 중 함수 본문을 실행시키는 나머지 경로 — 데코레이터 적용·바인드 메서드·전이 호출·별칭 호출·클래스 데코레이터 — 는 first_call 을 만들지 못한다. 그러면 `hi = inf` 가 되어 **바깥 스코프 가드가 위치와 무관하게 무조건 인정된다**(D3-1). ② 는 자기 출력에 `guard@20행 vs 첫 import@9행` 이라고 찍으면서 PASS 를 보고한다. 한 겹 감싸면 철회 투영도 같이 증발한다(D6-1).

거기 더해 실행 모델 자체가 소박하다. 지연 스코프를 `def` 로만 정의해 `lambda`·제너레이터식이 현 스코프 실행으로 계상되고(D3-2), `_static_truth` 를 `ast.If.test` 에만 연결해 `while False:` / `for _ in ():` 속 죽은 가드가 유효로 인정되며(D3-3), 블록을 '전 문장 순서대로 실행'으로 평탄화해 `return`/`raise` 뒤 도달 불가 문과 `except` 본문 가드가 유효로 인정된다(D3-4).

반대편 창(window) 문제도 있다. `hi=inf` 는 형제 import **뒤**의 조작까지 소급 무효화한다 — 계약이 아예 다루지 않는 구간이다(D1-F5·D6-4). 실물 `javis_briefing.py` 의 `_speak_via_vm`(제3자 폴더를 빌렸다 반납)이 이미 그 상태이고, `deliver()` 에 형제 import 한 줄만 넣으면 게이트가 exit 1 을 낸다(대조군: 철회 없는 `javis_report.py` 에 동일 편집 → exit 0).

> **처방 P4c — 모듈 스코프 호출그래프 도달성.**
> `module_calls` 를 "모듈 로드 시 실행되는 스코프 집합"으로 재정의하고 고정점으로 계산한다. 진입점: 모듈 최상위 문 · 데코레이터 적용(정의 시점 실행) · 클래스 본문 · 모듈 최상위 컴프리헨션. 전이: 도달한 def 이 부르는 def. 호출 대상이 `⊤` 면 `opaque`.
>
> **처방 P5 — 실행 모델 정합.**
> (a) `_split_scope` 의 지연 스코프에 `ast.Lambda`·`ast.GeneratorExp`·comprehension 의 **element/condition** 식을 추가(최외곽 iterable 은 즉시 평가이므로 제외).
> (b) `_static_truth` 를 `While.test`(False→본문 폐기, orelse 유지)와 `For/AsyncFor.iter`(빈 리터럴→본문 폐기, orelse 유지)에 연결. 수정 3~5줄.
> (c) 블록 내 무조건 `return`/`raise`/`break`/`continue`/`sys.exit()` 이후 문장은 도달 불가로 폐기. `except` 본문·`for-else`·조건 분기는 **조건부 영역**으로 표시.
> (d) **전 분석기 관통 원칙**: 가드는 *must-execute* 에서만 인정, 철회는 *may-execute* 면 계상. 조건부 영역의 가드는 가드가 아니고, 조건부 영역의 철회는 철회다. 현행은 여러 지점에서 이 비대칭을 거꾸로 적용한다.
>
> **처방 P6 — 창 계약 정합(`inf` 제거).**
> `hi` 를 절대 `inf` 로 두지 않는다. 바깥 스코프 가드일 때 import 를 감싼 스코프의 **모듈 레벨 진입점 집합** `E`(P4c 산출)를 쓰고 판정을 `gline < min(E) and (gline, max(E)] 구간에 철회 없음` 으로 정의한다. 이것으로 D1-F4(재호출), D1-F5·D6-4(import 뒤 과탐)가 동시에 닫힌다.

### 1-5. TYPE_CHECKING / builtins opaque → 부분 성립

`_uses_builtins` 는 `import builtins as b`·`from builtins import exec as e` 를 정확히 True 로 잡는다(브리핑이 물은 두 형태 모두 방어됨). 이 축은 버텼다. 그러나 주변의 fail-closed 원칙은 **아는 이름 4개**(`_OPAQUE_FUNCS`·`_DYNAMIC_IMPORTERS`)에만 걸려 있고, 그 밖의 sys.path 의존 해석 API(`pkgutil.resolve_name`·`runpy.run_module`)를 쓰는 파일은 `analyze_dir` 526-527행의 `continue` 로 **verdicts 에서 통째로 사라진다** — 금지가 아니라 생략이 기본값이다(D4-1). `__builtins__["exec"]`·`globals()["__builtins__"]` 도 `_uses_builtins=False` 로 빠져나간다.

> **정정 기록**: D4-1 이 인용한 docstring 74-75행은 원문이 "정적 증명이 불가능한 형태**(`exec`/`eval`/비상수 동적 import)**는 분석 포기가 아니라 금지" 이며, 그 문단 제목은 "★이 게이트가 **증명하지 않는 것**(정직한 한계)" 이다. 문서는 무제한 fail-closed 를 선언한 적이 없다. "계약 위반" 프레이밍은 성립하지 않고, 남는 것은 **동작상의 fail-open 기본값**이다. 다음 라운드에서 이 인용을 다시 쓰지 마라.
>
> **처방 P7 — 해석 API 는 allowlist, 부작용/조회 구분.**
> 모듈을 **이름으로 해석해 바인딩하는**(부작용) 호출 — `importlib.import_module`·`__import__`·`pkgutil.resolve_name`·`runpy.run_module`·`exec`/`eval` — 는 `opaque`. 반대로 **조회만 하는** 형태 — `importlib.util.find_spec(x) is not None`·`pkgutil.get_loader(x) is not None` — 는 허용. 이 구분은 필수다: `javis_select.py:102` 의 `_module_ready()` 가 조회형이라, 구분 없이 opaque 로 잡으면 실물 팩이 거짓 FAIL 한다. 별칭 문제는 P4a(바인딩 환경)로 자동 해결되므로 D4-1·D4-2 가 한 수정으로 닫힌다.

### 1-6. ③ rc=0 만 PASS → 반쪽, 그리고 실물에서 이미 무력

닫힌 것은 "rc≠0 인데 통과시키던" 방향뿐이다. **"rc=0 이 성공을 뜻하지 않는"** 반대 방향이 열려 있고, 이건 가설이 아니라 팩의 확립된 관례(ADR-2 상호 전제 금지 폴백) 위에 이미 앉아 있다.

```
③ 대상 8종의 모듈레벨 형제 import (AST 실측)
  javis_memory.py       삼켜짐=[('javis_skillscan', 51, ['Exception'])]        노출=-   ← 신호 0
  javis_orchestra.py    삼켜짐=[(639,'javis_todo_decl'),(214,'javis_boot_node'),(1033,'javis_verdict')]  ← 신호 0
  javis_report.py       삼켜짐=[('javis_todo_decl', 36, ['ImportError'])]      노출=-   ← 신호 0
```

가드를 한 글자도 안 건드리고 **형제 모듈 파일 자체를 삭제**해도 이 3종은 ③=PASS 다(같은 조건에서 javis_replay·javis_wakeup 은 정상 FAIL — 프로브 자체는 살아있고 이 3종만 눈이 멀었다).

더해서 **층 독립성이 없다**. ③ 루프(800-806)는 `for fn in sorted(verdicts)` + `not v["toplevel"] → continue` 로 검사 대상을 ①②의 정적 출력에서 받는다(D4-3·D5-2). 그래서 정적 사각이 실행 사각으로 상속되고, 결과가 침묵이 아니라 **적극적 거짓 보증**이 된다. docstring 이 "이건 ③ 이 잡는다"고 자기 예제로 못박은 `sys.path.append((1/0,_S)[1])` 조차 형제 import 를 함수 안에 두면 `PASS ①` `PASS ②` `80/80 PASS` exit 0 이다.

실측 규모: bin/ **76개** → 게이트 분석 **15개** → ③ 도달 **8개** → 그중 3개 무신호 → **런타임 층이 실제로 신호를 낼 수 있는 파일 5개(6.6%)**.

> **처방 P8 — ③ 을 진짜 독립 층으로. (최우선·비용 0)**
> (a) **대상 확장**: 프로브 대상을 `verdicts ∩ toplevel` 이 아니라 게이트 표면의 모든 배송 .py 로. 실측 확인 완료 — 76개 전부 오늘 rc=0 이므로 확장 즉시 초록이다(추가 비용 0).
> (b) **rc 가 아니라 관측**: `_PROBE_CODE` 가 기대 형제 집합을 받아 `builtins.__import__` 를 감싸고, try/except 로 **삼켜지기 전에** 실패를 기록한다.
> ```python
> _PROBE_CODE = r'''
> import builtins, runpy, sys
> sibs = set(sys.argv[2].split(",")) if len(sys.argv) > 2 else set()
> failed = []
> _real = builtins.__import__
> def _hook(name, *a, **k):
>     try:
>         return _real(name, *a, **k)
>     except ModuleNotFoundError as e:
>         root = name.split(".")[0]
>         if root in sibs or (e.name or "") in sibs:
>             failed.append(e.name or name)
>         raise
> builtins.__import__ = _hook
> runpy.run_path(sys.argv[1], run_name="__guardprobe__")
> if failed:
>     sys.stderr.write("SIBLING_IMPORT_FAILED:" + ",".join(sorted(set(failed))) + "\n")
>     raise SystemExit(3)
> '''
> ```
> 추가로 정적으로 알려진 toplevel 형제가 `sys.modules` 에 실제 등재됐는지 확인. rc≠0 은 현행대로 FAIL 유지.
> (c) 체크 문구 `③ %s: 폴더 미등재 조건에서 형제 import 통과` 를 실제 측정한 것으로 교체.

### 1-7. `_step_ranges()` → 표현 1개

스텝 경계 인식을 `- name:` → `steps:` 리스트 항목으로 올린 것은 맞다. 그러나 (a) 판정기는 여전히 **줄단위 정규식**이라 `"if": false` · `if : false` · `if:` 뒤 값을 다음 줄에 두는 정당한 YAML 표기가 전부 사각이고(F7-3 — ③은 `if:\s*(.+)$` 가 콜론 뒤 1문자를 요구해 구조적으로 불가), (b) '이 스텝이 실제로 돌고 그 실패가 잡을 실패시키는가'의 결정자 6개(워크플로 `on:` ∧ job `if:` ∧ matrix ∧ step `if:` ∧ `continue-on-error` ∧ 스텝 내부 조기 exit) 중 **1개만** 보며(F7-2), (c) 4레인 중 **1레인(windows-build)** 에만 배선됐다(F7-1).

가장 무거운 실증: **pack-release.yml 의 '배송 팩 계약 게이트' 스텝(스위트 7종을 한 스텝에서 전부 실행)에 `if: false` 한 줄** → PyYAML 로 `step['if'] = False`(절대 미실행)인데 게이트는 `비대칭 0 · 계약 충족` exit 0. 게이트 자신의 HINT 가 "배송 팩이 검증 0회로 tar·서명·봉인된다(C-1 계급)"라 부르는 상태다.

> **처방 P9 — YAML 파서 + 전 결정자 + 전 레인.**
> (a) `yaml.safe_load` 로 파싱. **ImportError 나 파싱 실패는 exit 1(fail-closed)** — F7-3 표기 공간이 통째로 닫힌다.
> (b) 결정자 전부 검사: 워크플로 `on:` · job `if:` · job/step `continue-on-error` · step `if:` · `needs`. 실행을 **넓히는** 조건은 화이트리스트(`always()`, `success() || failure()`) → F7-4 의 `if: always()` 과탐 소멸.
> (c) 4레인 전부 배선하되 판정을 레인별로: **"그 레인의 트리거 조건에서 무조건 실행되는 (job, step) 쌍이 최소 1개 존재"**. release.yml 의 build 잡 스텝은 `if: matrix.target == 'aarch64-apple-darwin'` 이 정당하고, pack-artifacts 잡(:535, ubuntu-latest, `if:` 없음)이 무조건 실행이므로 그 레인은 통과한다. 지금 검사를 단순 확장하면 release 가 즉시 거짓 실패한다(실측 확인).

---

## 2. 확증 건 — severity 순

### 2-1. BLOCK (12건)

| ID | 요지 | 위치 | 부류 |
|---|---|---|---|
| D1-F1 | sys.path 객체 인식이 2형태뿐 → 8가지 바인딩 채널로 우회 | `test_import_guard.py:289-320, 405-413` | 미검출 |
| D1-F2 | 문 형태가 `Assign`/`Delete` 뿐 → AugAssign·setattr·`__dict__` (+`+=` 가드 과탐) | `:370-378, 323-327` | 미검출+과탐 |
| D1-F3 | `_derives_from_file` 이 값이 아니라 오염을 봄 | `:275-286, 407` | 미검출 |
| D1-F4 | `first_call` 단일점 축약 + ③ 대상 선정이 구문 기반 | `:481-496, 530` | 미검출 |
| D2-1 | 헬퍼 투영이 `def` 문만 셈 → 6종 재바인딩 | `:435-458` | 미검출 |
| D2-2 | `setdefault` 로 첫 def 만 보존 → fail-closed 한 방향만 | `:438-440, 455-458` | 미검출 |
| D3-1 | `module_calls` 가 `Name(...)` 만 → first_call 소멸 | `:417-419, 481-496` | 미검출 |
| D4-3 | ③ 대상이 ①② 출력에 종속 → 깨진 파일에 적극적 PASS | `:800-806` | 계약구멍 |
| D5-1 | rc=0 이 형제 로드를 측정 안 함 (실물 3/8 무신호) | `:553-580, 572` | 미검출 |
| D6-1 | 한 겹 간접호출로 first_call·철회 투영 동시 증발 | `:417-419, 450-458, 481-485` | 미검출 |
| F7-1 | step `if:` 검사가 1레인에만 배선 (완전 레인 3개 무검사) | `ci-branch.yml:276` | 미검출 |
| T6-1 | ③ rc=0 → 실물 3파일 한 토큰 오타로 전층 통과 | `windows-build.yml:477` → `:546-580` | 미검출 |

**D1-F1** — `python3` 로 8검체 + 대조군 생성 후 `T.analyze_dir` vs 자식 프로세스 실행 대조:
```
f1_OK.py        ok_guard=True  ok_order=True 3대상=False | 실제: OK
f1_walrus.py    ok_guard=True  ok_order=True 3대상=False | 실제: ModuleNotFoundError: No module named 'javis_sib'
f1_funcarg.py / f1_unbound.py / f1_listbox.py / f1_tuple.py / f1_forloop.py / f1_comp.py / f1_operator.py  (동일)
```
증폭 요인 실측: `③ 미발동(함수내 import 만) = 7 ['javis_compete','javis_idempotency','javis_memory_inject','javis_phoenix','javis_phoenix_win_smoke','javis_purge_verify','javis_task']`. **처방 P1.**

**D1-F2** — 양방향:
```
f2_augGUARD.py   ok_guard=False ok_order=None  | 실제: OK    ← 정당한 `sys.path += [_S]` 과탐
f2_augassign.py  ok_guard=True  ok_order=True  | 실제: ModuleNotFoundError   ← 미검출
f2_setattr.py / f2_sysdict.py (동일 미검출)
```
`grep -rn "sys\.path *+=" cysjavis-pack/bin/*.py` → 0건 (금지가 아니라 우연). **처방 P2.**

**D1-F3** — `__file__` 파생이되 다른 폴더를 가리키는 4형태 전건 PASS:
```
f3_parentdir.py / f3_subdir.py / f3_dotdot.py / f3_ternary.py   ok_guard=True ok_order=True | 실제: ModuleNotFoundError
```
`__file__ if False else '/nonexistent'` 가 결정적 — `_static_truth` 가 `if False:` 를 처리할 줄 알면서 IfExp 안에서는 전혀 안 쓰인다. **비적대적 실수로 가장 일어나기 쉬운 부류다**(bin/tests 용 2단 dirname 을 bin/ 에 복붙). **처방 P3.**

**D1-F4** — `f6_recall.py`(조건부 조기 호출 + `sys.path.clear()` + 재호출): `①②PASS · ③스킵` 인데 **모듈 로드 자체**가 죽는다. 실물 팩 사본 투입 시 `80/80 PASS · exit=0`. 경계 실험으로 미검출 조건이 '첫 호출이 런타임에 안 타는 경우'로 한정됨을 확인(과장 없음). **처방 P6.**

**D2-1** — 대조: 셀프테스트 원본 `s_helper_redefined.py`(둘째 `def`)는 정적 FAIL, **둘째 def 를 대입으로 바꾼 동치**는 PASS.
```
collect dump: guards=[(4,<def스코프>),(8,0)]   ← 8행 ensure()(실제로는 _noop)에 모듈 스코프(0) 가드 투영
              destroys=[]  module_calls={'ensure':[8]}  is_protected=True
```
6종(대입·람다·`globals()`·`partial`·데코레이터·import-as) 전건 PASS + rc=1. **처방 P4a/P4b.**

**D2-2** — 순서만 뒤집으면 열린다:
```
guards=[(4,0)]  destroys=[(8, <둘째 def 스코프>)]   ← 호출지점 9행에 투영 안 됨
is_protected(line=10) = True
```
`func_dupes` 에 이름이 들어갔는데 **아무 효과가 없다**. **처방 P4b.**

**D3-1** — 5동치 전건 PASS, 대조군(bare `load()`)만 FAIL. 실물 팩 사본 엔드투엔드:
```
PASS ② javis_newfeature.py 모든 형제 import 가 유효 가드 뒤 | guard@20행 vs 첫 import@9행
=== 80/80 PASS ===   exit 0        ← 가드가 import 보다 11줄 뒤라고 스스로 출력하면서 PASS
```
③ 이 원리적으로 배제되는 유일한 BLOCK(`:802 not v["toplevel"] → continue`). 실물 실재성: 모듈 최상위 Attribute 호출 315건, 데코레이터 적용 1건(`javis_task.py:83`). **처방 P4c.**

**D4-3** — D4-1/D4-2 와 무관한 독립 재현(인식 구문만 사용, docstring 80-81행 자기 예제):
```
sys.path.append((1 / 0, _S)[1])    # 문서가 "이건 ③ 이 잡는다"고 명시한 형태
→ PASS ① / PASS ② / === 80/80 PASS === / exit 0     실제: rc=1 ZeroDivisionError
```
③ 사각 7파일이 **오늘도 살아 있다**. 처방 실행가능성 독립 측정: 76개 전부 rc=0 → 확장 비용 0. **처방 P8.**

**D5-1** — 변이 0으로 공허성 증명(형제 파일 삭제해도 3종 ③=PASS) + 게이트 exit 누수(javis_report 한 파일만 변이 → `78/78 PASS · exit 0`, 형제 미로드). 소스 82행 보상통제도 반례로 falsify(스위트 자신이 축복한 `s_good_try` 형태 안에서 성립). 완화 사실 기록: **원본 D1 형태(가드 전무)는 여전히 ①② 가 잡는다** — 무방비인 것은 '정적으로 유효하나 런타임에 틀린 가드'다. **처방 P8.**

**D6-1** — 깊이 1만 닫혔다:
```
a_indirect_call.py     PASS (ok_guard=True ok_order=True, ③대상=[])   실행 rc=1
ctl_direct_call.py     FAIL (naked=(4,'javis_sib'))                    ← outer() 한 겹만 제거한 대조군
>>> 실물 bin/ 사본 + 검체 1장:  GATE exit=0   REAL rc=1
```
보고가 든 `javis_phoenix.py:72` 는 실질 전조가 아님(가드·import 동일 함수). 실물 살아있는 결함 0건이나 트리거가 '평범한 한 겹 간접호출'. **처방 P4c.**

**F7-1** — `step_conditions()` 호출 지점이 `ci-branch.yml:276` 단 한 곳:
```
① ci-branch  팩 스위트 스텝 if: false  → [PyYAML] step.if=False, 게이트 EXIT=0
② pack-release 배송 게이트 스텝 if: false → suites=[7종 전부], 게이트 EXIT=0   ← 배송 레인
```
다른 층 없음(`test_import_guard.py` 817줄에 workflow/yaml 문자열 0건). **처방 P9.**

**T6-1** — 합성이 아닌 **실물 3파일** 오타:
```
$ (javis_memory/javis_orchestra/javis_report 의 첫 __file__ 파생 가드를 한 단계 위로)
SUITE_EXIT=0 / PASS ①②③ 전건 / === 78/78 PASS ===
--- 실제 --- javis_memory → 로드된 형제: 없음 / javis_orchestra → 없음 / javis_report → 없음
[대조] 같은 오타를 bare import 파일(javis_event)에 넣으면 → FAIL ③ / SUITE_EXIT=1
```
부류 경계가 정확히 'try/except 로 삼켜지는 형제 import' = 실물 3/8. **처방 P8.**

### 2-2. REVISE (19건)

| ID | 요지 | 재현 핵심 출력 | 처방 |
|---|---|---|---|
| D1-F5 | 바깥 스코프 가드의 창이 import **뒤**까지 `inf` | `f4_after_import.py FAIL / 실제 OK` · 실물 `javis_briefing.py` naked=(151,'javis_scrub'), rc=0 | P6 |
| D2-3 | `func_scope` 평면 → 렉시컬 스코프 부재 | 순수검체(builtin `hash` 가림, 재바인딩 0): `PASS / rc=1` | P4a 회귀항목 |
| D2-4 | `main_only` 가 import 에만 전달, 호출엔 미전달 | `guards=[(4,def),(6,0)]` 인데 6행은 `if __name__` 안 · import 는 `main_only=False` 정확 | `:416-419` 에 `mo` 전달 (1줄) |
| D2-5 | dupe fail-closed 를 전역 철회로 격상 → 정상 가드 무효화 | `FAIL / rc=0` · `destroys=[(9,0)]` | P4b |
| D3-2 | 지연 스코프를 `def` 로만 정의(lambda·genexp) | `def→lambda` 한 토큰으로 FAIL→PASS 반전 | P5a |
| D3-3 | `_static_truth` 를 `If` 에만 연결 | `while False:` / `for _ in ():` → PASS / rc=1 | P5b (3~5줄) |
| D3-4 | 블록 평탄화(도달불가·except 본문) | `return None` 뒤 가드 / `except:` 본문 가드 → PASS / rc=1 | P5c/P5d |
| D4-1 | 미인식 해석 API → opaque 아닌 **파일 생략** | `78/78 PASS` + `newfeature 언급 = 0 회` / rc=1 | P7 |
| D4-2 | opaque·동적 import 가 이름 문자열에 걸림 | 인식 3형태 FAIL vs 동치 7형태 **누락(무검사)** / 전건 rc=1 | P4a+P7 |
| D4-4 | ④ 검체 25건이 부류를 표집 못 함 | 검체 3건 런타임 주입 → `38/41 PASS`, 하네스는 이미 있음 | P10 |
| D5-2 | ③ 대상 선정이 정적 근사(스코프 길이) | `①②PASS / toplevel=[] / ③건너뜀 / rc=1` · 실물 0건 | P8a |
| D6-2 | 가드가 '어느 폴더'인지 미검증 + ③ 미커버 7파일 | `c_wrong_dir_guard.py PASS / rc=1`, `GATE exit=0 / REAL rc=1` | P3 |
| D6-3 | 검사 표면(bin/)이 배송 표면보다 좁음 | `_naver_http`×3 · `holdout_eval`×1 검사 밖, 스위트 언급 0회 | 표면 확장 + `check_timesfm.py:166` 조회형 예외 |
| D6-4 | `hi=inf` 철회가 함수 **전체**를 오염 | `javis_briefing` GATE exit=1 / REAL rc=0 · 대조군 exit=0 | P6 |
| D6-5 | dupe → 철회 승격 (플랫폼 분기 idiom) | `GATE exit=1 / REAL rc=0` · 이름만 바꾼 대조군 PASS | P4b |
| F7-2 | job `if:` · `continue-on-error` · `on:` 무검사 | (a) `jobs.build.if=False` EXIT=0 (b) T6 `continue-on-error=True` EXIT=0 | P9b |
| F7-3 | YAML 동치 표기 3형태 사각 | `"if": false` / `if : false` / 다음줄 값 → 전부 EXIT=0 (대조군 EXIT=1) | P9a |
| T6-3 | 백스톱 300s < 콜리 워치독 960s | `8×120=960s vs 300s` · T6 만 `Bx` 없음 · **절단 시 로그 줄수 0** | 예산 상향/프로브 timeout 하향 + `flush=True` |
| T6-4 | T6 가 배송 경로에서 0회 실행 | `windows-build on.push=['feat/windows-x64-dist']` vs `release on.push tags v*` · T6 는 마지막 스텝·`if:` 없음 | release windows 레그 이식 또는 T3 앞으로 이동 |

**정정 기록 2건** (다음 라운드 중복 방지):
- **D2-3** — 보고서가 든 "현존 조건"(`javis_hud_bridge __init__×4` 등)은 `func_dupes` 조건이지 **누출 조건이 아니다**. 누출 트리거(모듈 최상위 bare 호출인데 모듈 바인딩 없음)는 76개 전수 **0건**. 제시 검체 2개도 `ensure = int` / `ensure = lambda` 를 써서 D2-1 하위 사례였다. 독립성은 내가 만든 순수 검체(builtin 이름 가림)로만 성립한다.
- **T6-4** — "release.yml 에서 test_import_guard 를 돌리는 스텝은 mac 전용"은 부정확하다. build 잡(:229)만 mac 전용이고 **pack-artifacts 잡(:535, ubuntu-latest, `if:` 없음)이 무조건 스위트를 돌린다**. 태그 레인에서 빠지는 것은 정적층 전체가 아니라 **'동봉 인터프리터로 돈다'는 층 하나**다.

### 2-3. NIT (4건)

| ID | 요지 | 상태 |
|---|---|---|
| D1-F6 | `_path_aliases` 스코프 무시 → 파라미터 `path` 의 `.strip()` 이 철회로 계상 | 실물 팩 별칭 형태 0건. P4a 로 자동 해소 |
| D2-6 | `calls` 가 `ast.Name` 만 → 정당한 간접 호출 과탐 | 기록 가치 큼: 같은 뿌리를 D2-1 은 미검출, D2-6 은 과탐으로 양방향 오판 → "이름 기반 투영을 넓히는" 방향은 회귀를 만든다 |
| D3-5 | `try: import X / except ImportError: 가드 후 재시도` 관용구 과탐 | rc=0 인데 FAIL. 실물 0건 |
| F7-4 | `steps:` 후행 주석 오진 + `if: always()` 금지 | 경계 판별 실패는 **의도된 fail-closed** 이고 메시지가 진짜 원인을 명시. `always()`/`success()||failure()` 화이트리스트만 |

---

## 3. 기각된 지적 (3건) — 재제안 금지

| ID | 요지 | 기각 사유 |
|---|---|---|
| **D5-3** | ③ 내부 timeout 예산(960s) > T6 스텝(300s) → 진단 경로 도달 불가 | 산술은 맞으나 **표제가 실행으로 반증**. 실제 매다는 검체로 정확히 120.0s 에 TimeoutExpired 분기 발동 확인 — 죽은 코드가 아니다. 1종 행=120.2s, 2종 행=240.2s 로 예산 내 완주. 3종 이상 동시 행이어도 최종 판정은 어느 경로든 fail-closed 로 동일하고, 달라지는 건 메시지 구체성뿐. 실물 총 소요 0.24s. ※ T6-3(설계 역전 + 로그 절단)은 별건으로 살아 있다. |
| **D5-4** | 자식 프로세스 격리가 env·HOME·cwd 엔 미성립 | 소스 556-557행의 실제 주장은 "sys.path·sys.modules **그 부류**"로 한정돼 있고 그건 참. env 상속은 격리 실패가 아니라 **의도된 충실도**(팩 스크립트가 `CYS_PACK_DIR` 등을 정당히 읽음; env 백지화는 재현 대상과 **덜** 같아진다). 모델링 대상의 유일한 오염원 PYTHONPATH 만 제거한 것은 설계대로. 공유 cwd 오염 실측 0건. 판정 영향 0. |
| **T6-2** | 인코딩 크래시가 'D1 재발'로 오귀인 | 근거 선례 2건이 **오독**. `javis_bootstrap.py:50-51` 주석은 "PYTHONUTF8 export 가 cys-dept 경로에만 있어 직접 실행을 못 지킨다" = 변수가 **안 걸린 경로**가 문제라는 뜻인데, T6 는 그 변수를 스텝 `env:` 로 명시 설정한다(:429). T5 도 `PYTHONUTF8: '1'` 을 설정하므로 `javis_phoenix_win_smoke.py:56-57` 의 reconfigure 는 실패 증거가 아니라 이중 방어. 재현이 CI 조건이 아님(`PYTHONIOENCODING=cp1252` 강제 — GH Actions 는 미설정). 핵심 전제(`._pth` 가 PYTHONUTF8 무효화)는 지적자도 미검증. 방향도 fail-closed(첫 실행 빨강). ※ `reconfigure` 한 줄 추가는 값싼 개선으로 NIT 잔여. |

---

## 4. 공격이 실패한 영역 — ACCEPT (신뢰의 근거)

8차원 총공세에서 **버틴 것들**이다. 이게 남길 가치가 있는 자산이다.

**a) 2차 수정이 자기 축에서 실제로 작동한다 (대조군이 전부 정상 차단)**
- `s_helper_redefined.py`(첫 def 가드 · 둘째 def 무해) → 정적 FAIL. D2-2 는 **순서를 뒤집었을 때**만 열린다.
- `s_outer_guard_after_call.py` 직접 `load()` 호출판 → FAIL. D3-1/D6-1 은 **한 겹 감쌌을 때**만 열린다.
- `s_func_guard.py`(def 만 되고 호출 안 됨) → FAIL. D3-2 는 **lambda 로 바꿨을 때**만 열린다.
- `s_false_tuple.py`(`if ():`) → FAIL. D3-3 은 **`while False:`/`for _ in ():`** 에서만 열린다.
- 레인 게이트: 평범한 `if: false` → 정확한 문구로 exit 1.
- ③ rc≠0 방향: 형제 파일 삭제 시 `javis_replay`·`javis_wakeup` 정상 FAIL. bare import 파일에 가드 오타 → `FAIL ③ ... (형제 모듈 — D1 재발)` exit 1.

**b) 브리핑이 물은 opaque 형태가 방어된다**
`import builtins as b` → `_uses_builtins=True`. `from builtins import exec as e` → `True`. 이 축은 뚫리지 않았다.

**c) `_path_aliases` 고정점 자체는 정확하다**
`from sys import path` · `q = sys.path` · `r = q` 3단 전파 모두 추적된다. D1-F6 는 전파가 아니라 **스코프**를 친 것이다.

**d) 2층 설계가 조건이 갖춰지면 실제로 작동한다**
정적층이 놓친 다수 케이스를 ③ 이 실제로 잡았다 — D1-F3 모듈레벨판(`FAIL ③ javis_twodir_top.py ... D1 재발`), D2-1·D2-2 모듈레벨판, D3-2 c1/c2, D3-3 b1/b2, D3-4 b4/b5/b7, D6-2 c2. **③ 이 안 도는 것이 문제이지 ③ 자체가 무력한 게 아니다.** 그래서 P8(대상 확장)이 가장 값싸고 효과가 크다.

**e) 실물 팩은 지금 깨끗하다**
bin/ 76개 전부 중립 cwd·PYTHONPATH 제거 조건에서 rc=0. 살아있는 D1 인스턴스 0건. `javis_report.py:33-35` 는 가드를 `try` **앞**에 둔 모범 배치다. 위험은 현재가 아니라 회귀에 있다.

**f) 게이트의 fail-closed 본능이 일부 자리에서는 옳게 작동한다**
스텝 경계 판별 실패 시 조용한 통과가 아니라 exit 1(F7-4) — 이건 결함이 아니라 설계다. 진단 문구도 진짜 원인을 명시한다.

**g) 3건이 독립 반증으로 기각됐다**
D5-3·D5-4·T6-2. 특히 D5-4(env 상속)와 T6-2(인코딩)는 **의도된 설계가 지적보다 옳았다**.

---

## 5. 남은 미검증 영역과 위험

| 영역 | 상태 | 위험 |
|---|---|---|
| **T6 는 Windows 에서 0회 실행** | `windows-build.yml` 은 `push: branches: ['feat/windows-x64-dist']` + `workflow_dispatch` 에서만 실행. 현재 브랜치 `release/0.14-quality-line`. 태그 릴리스(`v*`)로 나가는 setup.exe 는 T6 를 **0회** 통과 | **높음**. T6 의 유일한 고유 가치(동봉 embeddable 로 실행)가 배송 경로에 도달 0회. 이 라운드의 T6 관련 판정(T6-1·T6-3·T6-4)은 전부 mac 에서 유도한 것이고, **실제 embeddable 런타임에서의 동작은 아무도 본 적이 없다** |
| **음성 대조(`probe_sib.py`/`probe_main.py`)** | `windows-build.yml:443-472`. 실행 이력 0 | 중간. 이 대조가 실제로 `._pth` 성질을 증명하는지 미확인 |
| **Windows 경로 의미론** | 이번 라운드 전건 미검증 | 중간. 실물 가드는 `if _S not in sys.path` 문자열 비교. Windows 대소문자 무시·백슬래시·8.3 단축명 하에서 이 멤버십 검사가 어떻게 되는지 아무도 안 봤다. ③ 프로브의 자식 프로세스도 `._pth` 를 상속하므로 mac 과 `sys.path` 초기값이 다르다 |
| **verdicts 에서 빠진 61개 파일** | 실측 rc=0 이나 **게이트 대상 아님** | 중간. 오늘 초록인 것은 확인됐지만 회귀 보호는 0. P8a 로 즉시 해소 가능(비용 0) |
| **skills/*/scripts 4건** | `_naver_http`×3 · `holdout_eval`×1. 같은 embeddable 로 돌지만 검사 밖 | 중간(D6-3). 현재 넷 다 가드 보유 — 회귀 보호만 0 |
| **③ 미도달 7파일** | compete·idempotency·memory_inject·phoenix·phoenix_win_smoke·purge_verify·task | 높음. ①② 가 유일 게이트인데 그 ①② 가 이 보고서의 BLOCK 대부분이 뚫는 층이다 |
| **PowerShell 인코딩(T6-2 잔여)** | 기각됐으나 `reconfigure` 미적용 | 낮음. 최악이어도 첫 실행 빨강 |
| **T6 로그 절단** | 파이프 블록 버퍼(8KB)에 78줄(~5KB)이 갇혀 절단 시 **0줄** | 중간. hang 이 아니라 **어떤 이유로든** 스텝이 절단되면 ①②③ 판정이 한 줄도 안 남는다 |

---

## 6. 처방 우선순위 — BLOCK 해제 조건

증거가 말하는 것은 명확하다. **증분 패치는 3라운드 연속 같은 방식으로 실패했다.** 4번째 표현 열거는 처방이 아니다. 다만 전면 재작성 전에 **비용 0이고 효과가 즉시 나는 것**이 하나 있다.

**0단계 — 지금 당장 (비용 0, 회귀 위험 0)**
`P8a`: ③ 프로브 대상을 `verdicts ∩ toplevel` → 배송 표면 전체로. 실측으로 76개 전부 오늘 rc=0 이므로 확장 즉시 초록이다. 이것만으로 D5-2·D4-3 의 사각 7파일과 61개 미검사 파일이 커버되고, 정적층이 놓친 다수 케이스에 백스톱이 생긴다.

**1단계 — 층 독립성 회복 (BLOCK 해제 필수)**
`P8b/c`: `builtins.__import__` 훅으로 **삼켜지기 전에** 형제 import 실패를 관측. rc 대리지표 폐기. → D5-1·T6-1·D4-3 이 닫힌다. 이 하나가 실물 팩의 가장 무거운 구멍(3/8 무신호)을 없앤다.

**2단계 — 값 판정 (BLOCK 해제 필수)**
`P3`: `_derives_from_file` 을 오염 walk 에서 **심볼릭 경로 폴딩**으로 교체, `SELFDIR` 정확 일치만 가드, `UNKNOWN` 은 opaque. → D1-F3·D6-2 + D4-3 의 IfExp 케이스.

**3단계 — 추상해석으로 전환 (BLOCK 해제 필수)**
`P1`+`P2`+`P4a/b/c`+`P5`+`P6`+`P7`. 이건 개별 패치의 합이 아니라 **분석기 코어의 재작성**이다. 값 격자 · 바인딩 환경 · 모듈 스코프 호출그래프 도달성 · ⊤=opaque 의 4요소가 서로를 필요로 한다. 단일 관통 원칙: **가드는 must-execute, 철회는 may-execute.**

**4단계 — CI 층**
`P9`: 레인 게이트를 PyYAML 파서로(파싱 실패=exit 1), 결정자 전부 검사, 4레인 배선(레인별 "무조건 실행 (job,step) 쌍 ≥1"). `T6-4`: T6 를 release.yml windows 레그에 이식하거나 최소한 T3 앞으로 이동. `T6-3`: 예산 정합 + `flush=True`.

**5단계 — 회귀 잠금**
`P10`: `_SPECIMENS` 에 각 부류당 **"수정이 커버해야 하지만 실증 케이스가 아니었던"** 검체 2건 이상. ④ 하네스(738-742)는 이미 `v is None + must_fail` 을 "스캐너가 놓쳤다"로 FAIL 처리하므로 **검체만 추가하면 된다**. 단 반드시 분석기 수정과 **같은 커밋**에 — 검체만 넣으면 스위트가 영구 빨강이다.

### BLOCK 해제 기준

1. 12건 BLOCK 각각에 대해, 그 건의 재현 검체 **그리고 같은 부류의 서로 다른 표현 최소 2개**가 게이트 exit≠0 을 만들 것.
2. 실물 팩 3파일(`javis_memory`·`javis_orchestra`·`javis_report`) 가드에 한 토큰 오타를 냈을 때 게이트가 exit≠0 을 낼 것 (T6-1 의 재현을 그대로 통과 기준으로).
3. `pack-release.yml` 배송 게이트 스텝에 `if: false` 를 달았을 때 레인 게이트가 exit≠0 을 낼 것.
4. 위 전부 만족한 상태에서 무수정 실물 팩이 여전히 초록일 것(과탐 0 확인 — 특히 `javis_briefing.py`·`javis_org.py`·`javis_report.py` 의 현존 idiom).

---

## 7. 검증 환경 원상 확인

모든 검체·사본은 `mktemp -d` 임시 폴더에만 생성했다. 작업트리 무수정.

```
$ python3 cysjavis-pack/bin/tests/test_import_guard.py
SUITE_EXIT=0
=== 78/78 PASS ===

$ python3 cysjavis-pack/bin/tests/test_import_guard.py --selftest
SELFTEST_EXIT=0
=== 38/38 PASS (selftest only) ===

$ git status --porcelain
 M .github/workflows/ci-branch.yml
 M .github/workflows/pack-release.yml
 M .github/workflows/release.yml
 M .github/workflows/windows-build.yml
 M cysjavis-pack/bin/javis_event.py
 M cysjavis-pack/bin/javis_memory.py
 M cysjavis-pack/bin/javis_memory_inject.py
 M cysjavis-pack/bin/javis_orchestra.py
 M cysjavis-pack/bin/javis_phoenix_win_smoke.py
 M cysjavis-pack/bin/javis_task.py
 M cysjavis-pack/bin/javis_wakeup.py
AM cysjavis-pack/bin/tests/test_import_guard.py
?? docs/plans/2026-07-29-win-evidence-runbook.md
?? docs/plans/2026-07-29-win-two-defects-plan.md
?? docs/plans/review-2026-07-29-r1-adversarial-codex.md
?? docs/plans/review-2026-07-29-r2-ci-gemini.md
?? docs/plans/review-2026-07-29-round2-codex.md
?? docs/plans/review-2026-07-29-round2-gemini.md

$ git branch --show-current
release/0.14-quality-line
```

기준선과 동일 — 변경 0.

---

## 8. 한 문단 요약

이 스위트는 **좋은 게이트가 되려는 정직한 시도**이고 실제로 여러 축에서 버틴다(§4). 그러나 3라운드 연속 같은 방식으로 뚫렸고, 이번 라운드의 증거는 그 실패가 놓친 표현들의 목록이 아니라 **설계**임을 말한다 — 분석기는 이름·구문 술어의 집합이며 증명 불가에 대한 기본 처분이 금지가 아니라 생략이다. 게다가 유일한 안전망이어야 할 실행층 ③ 은 검사 대상을 정적층에서 받고(층 독립성 없음), 판정을 rc 라는 대리지표로 하며(실물 3/8 무신호), 결과적으로 깨진 파일에 침묵이 아니라 **적극적 PASS 두 줄**을 찍는다. 다행히 가장 값싼 수정(③ 대상 확장 + import 훅 관측)이 가장 큰 효과를 내고 오늘 즉시 초록임이 실측됐다. 그것부터 하고, 정적층은 추상해석으로 다시 세워라.


---

# 부록 A — 완결성 비평 (검증 자체의 사각)
## 종합

3차 검증 8차원은 **분석기 안쪽**을 8방향에서 훑었다. 사각은 전부 **분석기 바깥**에 있다 — 이 게이트가 존재해야 하는지, 게이트가 검증한 조건이 생산 조건인지, 게이트를 만난 사람이 무엇을 하는지, 그리고 게이트가 초록인 동안 D1 이 다른 문으로 들어오는지. 8차원은 "분석기가 부류를 닫았나"를 물었고, 아무도 "**분석기가 있어야 하는 층이 여기가 맞나**"를 묻지 않았다.

가장 무거운 것부터.

**(1) 근본 수리 층이 한 번도 재개봉되지 않았다.** D1 의 원인인 `python312._pth` 는 **이 저장소가 직접 쓴다**(`windows-build.yml:62`·`release.yml:277`: `printf 'python312.zip\n.\nimport site\n'`). 그 3줄에 `import site` 가 있고 get-pip 로 `Lib/site-packages` 가 만들어지므로 **`.pth`·`sitecustomize` 경로가 살아 있다**. 계획서 §7(610행)은 `.pth` 대안을 "정적 경로는 한 레인에 고착 → 부서 격리 위반"으로 철회했는데, `.pth` 의 `import` 로 시작하는 줄은 **실행되는 코드**라 정적일 이유가 없다. 실증(`python3 -I`, embeddable 과 동치):
```
== 대조: .pth 없이 (=오늘의 조건) ==   ModuleNotFoundError: No module named 'javis_scrub'
== .pth 활성 ==                        가드 0줄 · bare import 성공 -> 42
```
`.pth` 한 줄(`sys.path.append(os.environ.get("CYS_PACK_DIR", ...)+"/bin")`)이 `CYS_PACK_DIR`·부서 팩을 그대로 지원하고 `append` 라 shadowing 교리(docstring 84-88행)도 지킨다. 즉 817줄 분석기 + 3라운드 적대검증 + 76개 파일 영구 규율은 **메커니즘에 대한 사실오류에 기댄 철회 결정 하나** 위에 서 있고, 그 결정은 3라운드 어디서도 다시 열리지 않았다. 이건 "게이트를 더 잘 만들라"가 아니라 "게이트가 지키는 불변식을 없앨 수 있나"라는 질문이고, 아무도 안 했다.

**(2) ③ 은 생산이 스크립트를 부르는 방식으로 부른 적이 없다.** `run_name='__guardprobe__'` 라서 `__main__` 이 안 돈다. 그런데 생산은 훅이 `python3 <file>` 로 부른다 — 전부 `__main__` 이다. 실측: 형제 import 20건 중 모듈레벨 8건(40%)만 ③ 이 실행하고 함수내 12건(60%)은 **어느 실행층도 닿지 않는다**. 그리고 조건을 그대로 유지하면서 생산 진입형을 쓰는 방법이 한 플래그로 있다 — `python3 -I script.py`(실측: 스크립트 폴더가 sys.path 에서 빠지는 embeddable 과 동치). T6 도 같은 스위트를 돌리므로 "실물 런타임 레인"조차 팩 스크립트를 훅처럼 부르지 않는다. (D5-2/D6-2 는 *어느 파일을* 고르는가의 문제였다. 이건 *어떤 형태로* 부르는가의 문제이고, 파일 선정을 완벽히 고쳐도 남는다.)

**(3) ④ 셀프테스트에 실행 접지가 0이다.** 검체 37건 중 **단 하나도 실행되지 않는다** — `runtime_probe` 가 같은 파일 안에 있는데도. 내가 37건을 전부 실행해 보니 오늘의 라벨은 (내 픽스처 결함 1건 제외) 전부 옳다. 문제는 그게 **우연**이라는 것이다. `must_fail=False` 라벨은 사실상 "분석기야 이건 잡지 마라"는 지시이므로, 라벨이 틀리면 사각이 **회귀 잠금으로 승격**된다. 실증 — `bin/tests/` 용 2단 dirname 을 `bin/` 에 복붙한 검체:
```
analyzer  ok_guard=True ok_order=True   execution runs_ok=False (ModuleNotFoundError)
must_fail=False 로 등재하면 ④ 는 '오탐 없음 PASS' 를 찍는다 → PASS
```
3라운드 동안 사람이 손으로 20+건을 찾아낸 방법이 정확히 "정적 vs 실행 대조"인데, 스위트는 그 방법을 기계화하지 않았다. (D4-4 는 *어떤 검체가 없는가*였고, 이건 *있는 검체의 라벨이 검증되지 않는다*이다.)

**(4) 과탐이 미검출을 가르친다 — 계약의 명시적 정당화가 실측으로 반박된다.** docstring 69-70행은 "과탐은 CI 에서 시끄럽게 드러나 즉시 고쳐진다"를 미수정 근거로 든다. 그런데 게이트에는 억제 수단(noqa·file allowlist)이 없고, D4-1 이 확인한 대로 인식 불가 형태는 **파일 통째 생략**이다. 실측 2단계:
```
[1] 정당한 빌림/반납 2줄 추가        → 게이트 FAIL | naked=(47,'javis_scrub')
[2] find_spec 로 한 줄 교체          → verdicts 등재: False  (①②③ 어느 층도 이 파일을 보지 않는다)
```
즉 소음의 **가장 싼 해소법이 미검출 생산**이다. "시끄러움 → 즉시 수정"은 수정 경로가 하나일 때만 성립하는데 여기선 두 번째 경로가 더 싸다.

**(5)(6)(7)** 은 각각: 생산 인터프리터가 PATH 해소 결과라 T6 가 검증한 인터프리터와 같다는 보장이 없다는 것 / 훅 fail-open 이 그대로라 생산에 D1 탐지기가 0개라는 것(D1 이 릴리스 하나를 산 원인은 게이트 부재가 아니라 무증상성인데 3라운드 전체가 사전 게이트만 봤다) / 게이트가 배송 산출물이 아니라 checkout 소스를 검사한다는 것.

**증거가 약한 확증 건.** `D1-F5`·`D6-4`(바깥 스코프 `hi=inf` 과탐)는 "javis_briefing.py 에 이미 그 형태가 있다"를 근거로 드는데, 실측 기준선은 그 파일이 78/78 안에서 PASS 다 — 즉 **현재 실물 영향은 0이고 가정 편집 위의 예측**이다. BLOCK 급 확증과 같은 무게로 읽히면 안 된다. 반대로 `D5-1`/`T6-1`(rc=0 이 성공을 뜻하지 않음)은 실물 3/8 파일에서 구조적으로 성립하므로 증거 등급이 확연히 다르다.

**작업트리 원상 확인** — 시작 시점과 동일(검체는 전부 `mktemp -d` 임시 폴더):
```
 M .github/workflows/ci-branch.yml
 M .github/workflows/pack-release.yml
 M .github/workflows/release.yml
 M .github/workflows/windows-build.yml
 M cysjavis-pack/bin/javis_event.py
 M cysjavis-pack/bin/javis_memory.py
 M cysjavis-pack/bin/javis_memory_inject.py
 M cysjavis-pack/bin/javis_orchestra.py
 M cysjavis-pack/bin/javis_phoenix_win_smoke.py
 M cysjavis-pack/bin/javis_task.py
 M cysjavis-pack/bin/javis_wakeup.py
AM cysjavis-pack/bin/tests/test_import_guard.py
?? docs/plans/2026-07-29-win-evidence-runbook.md
?? docs/plans/2026-07-29-win-two-defects-plan.md
?? docs/plans/review-2026-07-29-r1-adversarial-codex.md
?? docs/plans/review-2026-07-29-r2-ci-gemini.md
?? docs/plans/review-2026-07-29-round2-codex.md
?? docs/plans/review-2026-07-29-round2-gemini.md
```
기준선 재확인: `78/78 PASS` · `38/38 PASS (selftest only)`.

**권고.** 4라운드를 분석기 안에서 돌리기 전에 (1)을 먼저 판정하라. `.pth`/`sitecustomize` 가 채택되면 분석기는 **defense-in-depth 의 2선**으로 강등되고 D1-F1~D6-5 의 BLOCK 등급이 전부 재산정된다 — 즉 지금 착수하려는 추상해석 재작성의 필요 규모 자체가 이 판정에 종속된다. (2)(3)은 각각 한 플래그·다섯 줄이고 3라운드가 손으로 한 노동을 기계화하므로, (1) 결과와 무관하게 즉시 넣을 값어치가 있다.

## 목적 적합성 자체 — '이 게이트가 존재해야 하는가'를 8차원 중 어느 것도 묻지 않았다. D1 의 원인인 `python312._pth` 는 이 저장소가 직접 쓰고(`windows-build.yml:62`·`release.yml:277`), 그 3줄에 `import site` 가 있어 `.pth`/`sitecustomize` 경로가 살아 있다. 계획서 §7(610행)은 `.pth` 대안을 '정적 경로는 한 레인에 고착 → 부서 격리 위반'으로 철회했는데, `.pth` 의 `import` 줄은 실행되는 코드라 정적일 이유가 없다. 실증: `python3 -I` 격리(embeddable 동치)에서 `.pth` 한 줄이 `CYS_PACK_DIR` 를 런타임에 읽어 append 하면 **가드 0줄로 bare import 가 성공한다**(`가드 0줄 · bare import 성공 -> 42`; 대조군은 `ModuleNotFoundError`). 철회 근거가 메커니즘에 대한 사실오류다.

**왜 중요한가**: 817줄 분석기·3라운드 적대검증·76개 파일에 대한 영구 규율이 전부 이 철회 결정 하나 위에 서 있다. 그 결정이 틀렸다면 3라운드가 찾은 BLOCK 20여 건은 '고쳐야 할 결함'이 아니라 '없어도 되는 층의 결함'이고, 4라운드 추상해석 재작성은 착수 자체가 오분류다. 반대로 철회가 옳다면 그 이유는 지금 문서에 적힌 것이 아니라 다른 것(예: 그 런타임을 쓰는 모든 프로세스에 pack bin 이 보이는 것의 부작용)이어야 하는데, 그 논증이 어디에도 없다. 또한 T6 는 런타임이 스크립트 폴더를 sys.path 에 넣으면 '전제 붕괴'로 throw 하도록 설계돼 있어, 게이트가 결함을 **영구 전제로 고정**하는 방향으로 작동한다.

**닫는 방법**: (a) 계획서 §7 610행 철회를 재개봉하고, `.pth`/`sitecustomize` 를 **정적 경로 형태가 아니라 실행 코드 형태**로 재평가하라 — 위 실증이 '부서 격리 위반' 논거를 무효화한다. (b) 재평가 축은 새 것이어야 한다: 이 런타임을 쓰는 모든 프로세스(pip 설치 CLI 포함)에 pack bin 이 sys.path 에 들어가는 것의 부작용, `append` 유지 시 shadowing 위험, 팩 미설치 상태에서의 동작. (c) 채택되면 분석기는 2선(defense-in-depth)으로 강등하고 BLOCK 등급을 전부 재산정하라. 기각되면 **기각 사유를 코드 주석과 §7 에 실측 기반으로 다시 쓰라** — 지금 적힌 사유는 반증됐다.

## ③ 실행층이 생산 진입 경로(`__main__`)를 한 번도 실행하지 않는다. `runtime_probe` 는 `run_name='__guardprobe__'` 로 돌아 `if __name__ == '__main__':` 을 건너뛰는데, 생산에서 훅은 `python3 <file>` 로 부르므로 전부 `__main__` 이다. 실측: 팩의 형제 import 20건 중 모듈레벨 8건(40%)만 ③ 이 실행하고 함수내 12건(60%)은 어느 실행층에도 닿지 않는다. 그리고 조건을 유지하면서 생산 진입형을 쓰는 방법이 플래그 하나로 존재한다 — 실측 `python3 -I script.py` 는 스크립트 폴더를 sys.path 에서 빼면서(embeddable `._pth` 와 동치) `__main__` 으로 실행한다.

**왜 중요한가**: ③ 은 '정적 판정을 믿지 않는 층'으로 선언됐는데, 실제로는 생산에서 도는 코드의 40% 만, 그것도 생산과 다른 진입형으로 돈다. 실측상 8개 프로브 대상 파일의 모듈 레벨은 사실상 불활성(빈 HOME 에서도 전부 rc=0, 부작용 0)이라 ③ 의 rc=0 은 '거의 아무것도 실행하지 않고 끝났다'와 구별되지 않는다. D5-1 의 'rc=0 이 성공을 뜻하지 않는다'와 곱해지면 ③ 의 신호값은 거의 0에 수렴한다. T6 도 같은 스위트를 돌리므로 '유일한 실물 런타임 레인'조차 팩 스크립트를 훅이 부르는 방식으로 부른 적이 없다 — T6 가 3레인 대비 더한다고 주장하는 값이 여기서 다시 깎인다.

**닫는 방법**: (a) `runtime_probe` 를 `runpy.run_path(..., run_name='__guardprobe__')` 대신 **`[sys.executable, '-I', path]` 자식 프로세스**로 바꿔 생산 진입형(`__main__`)을 그대로 재현하라 — mac/ubuntu 3레인에서도 embeddable 과 동치가 되어 T6 의존이 줄어든다. (b) 부작용이 있는 스크립트는 `--help`/no-op 인자 등 **부작용 없는 생산 진입점**을 각 스크립트에 규약으로 두고 그것으로 돌려라. (c) ③ 대상 선정을 `toplevel` 이 아니라 '이 파일이 생산에서 어떻게 기동되는가'(훅 등록 표) 기준으로 바꿔 함수내 12건을 커버 안으로 끌어들여라.

## ④ 셀프테스트가 검체를 하나도 실행하지 않는다 — 오라클이 '저자의 믿음'이고, 잘못된 `must_fail=False` 라벨은 분석기의 사각을 **회귀 잠금으로 승격**시킨다. 검체 37건 전부에 대해 `runtime_probe` 를 직접 돌려 보니 오늘의 라벨은 옳지만(내 픽스처 결함 1건 제외) 그 사실을 스위트는 확인하지 않는다. 실증 — `bin/tests/` 용 2단 dirname 을 `bin/` 파일에 복붙한 검체: `analyzer ok_guard=True ok_order=True` / `execution runs_ok=False (ModuleNotFoundError)`, 그런데 `must_fail=False` 로 등재하면 ④ 는 '오탐 없음 PASS' 를 찍는다.

**왜 중요한가**: `must_fail=False` 라벨은 사실상 '분석기야 이건 잡지 마라'는 지시다. 라벨이 한 번 틀리면 그 사각은 이후 모든 라운드에서 '수정하면 셀프테스트가 깨지는' 보호 대상이 되어 고칠 수 없게 된다 — 4라운드에서 추상해석으로 올릴 때 정확히 이 방향으로 회귀가 걸린다(D1-F3 이 지적한 `_derives_from_file` 완화가 바로 이 라벨 부류다). 더 근본적으로, 3라운드 동안 사람이 손으로 20+건을 찾아낸 방법이 '정적 판정 vs 실제 실행 대조'인데 스위트는 그 방법을 기계화하지 않았다. 그래서 매 라운드 같은 노동이 사람에게 되돌아온다. `runtime_probe` 는 이미 같은 파일 안에 있다.

**닫는 방법**: `selftest()` 에서 각 검체에 `runtime_probe(<검체>, {'javis_sib'}, <중립 cwd>)` 를 돌려 **라벨 자체를 검증하는 체크를 추가하라**: `must_fail=True` → `runs_ok` 는 반드시 False(아니면 그 검체는 과탐 박제다), `must_fail=False` → `runs_ok` 는 반드시 True. 그리고 세 번째 체크를 넣어라 — `static_ok != runs_ok` 인 검체가 생기면 즉시 FAIL(= 분석기와 실행의 불일치를 스위트가 스스로 잡는다). 5줄이면 되고, 이 라운드가 손으로 한 대조가 이후 자동으로 돈다.

## 게이트를 만난 개발자의 실제 행동(우회 유인)을 아무도 모델링하지 않았다. 게이트에는 억제 수단(`# noqa`·파일 allowlist·예외 등록)이 전혀 없고, D4-1 이 확인한 대로 인식 불가 형태의 기본 처분은 파일 **통째 생략**이다. 실측 2단계: ①실물 `javis_wakeup.py` 에 정당한 '빌렸다 반납' 2줄(`sys.path.insert(0,'/tmp/vendor')` … `remove`)을 넣으면 → `게이트 FAIL | naked=(47,'javis_scrub')`. ②그 소음을 없애는 한 줄(`import javis_scrub` → `importlib.util.find_spec` 경유)로 바꾸면 → `verdicts 등재: False` — ①②③ **어느 층도 그 파일을 보지 않는다**.

**왜 중요한가**: docstring 69-70행은 미수정 결정의 명시적 근거로 '과탐은 CI 에서 시끄럽게 드러나 즉시 고쳐지지만 미검출은 조용히 산다'를 든다. 그 논증은 소음의 해소 경로가 '코드를 올바르게 고친다' 하나일 때만 성립한다. 실측상 두 번째 경로(분석기에게 안 보이게 만들기)가 더 싸고, 그 경로의 결과물이 정확히 '조용히 사는 미검출'이다. 즉 게이트의 과탐 정책이 자기가 막으려는 실패 형태를 **생산한다**. 3라운드는 과탐(D1-F5·D2-5·D6-4·D6-5)과 파일 소멸(D4-1·D4-3)을 각각 확증했지만 둘을 하나의 인과로 잇지 않았고, 그래서 '과탐은 안전한 방향'이라는 전제가 살아남았다.

**닫는 방법**: (a) 계약 문서의 '과탐은 안전하다' 논증을 폐기하거나, 우회가 불가능함을 보장하는 조건(fail-closed 기본값)을 먼저 세운 뒤에만 유지하라. (b) **명시적 억제 채널을 만들어라** — 한 줄 주석 pragma + 사유 필수 + 그 목록을 스위트가 세어 상한을 걸고 리뷰에 노출. 억제가 없으면 억제는 코드 난독화로 발생한다. (c) D4-1 을 fail-closed 로 뒤집어 '인식 못 하는 형태 = opaque = FAIL' 로 만들면 두 번째 경로가 막힌다 — 이것이 (b)보다 우선이다. (d) 게이트 실패 메시지에 '이 게이트를 우회하지 말고 이렇게 고쳐라'의 정답 형태를 직접 박아 넣어라.

## 게이트가 검증한 인터프리터와 생산이 실제로 쓰는 인터프리터가 같다는 것을 아무도 확인하지 않았다. 훅은 인터프리터를 **PATH 로 해소**한다 — `cysjavis-pack/hooks/role-bootstrap.sh:28`·`save-state.sh:12`: `CYS_PY="$(command -v python3 || command -v python || command -v py || echo python3)"`, `hooks/grill-gate.sh:29,36` 은 bare `python3`. 반면 T6 은 `$dir\runtime\python\python3.exe` 를 **하드코딩**해 검증한다(windows-build.yml:435). 두 값이 일치하는 근거는 cysd 의 PATH 주입 순서뿐이고, 게이트는 그 순서를 검증하지 않는다.

**왜 중요한가**: 양방향 모두 게이트의 의미를 무효화한다. ⑴사용자 PATH 에 자기 Python(스토어판·Anaconda·python.org)이 앞서면 스크립트 폴더가 sys.path 에 들어가 D1 이 나타나지 않는다 — 게이트가 지키는 불변식이 그 기계에선 무의미하고, 반대로 그 기계에서 초록인 코드가 embeddable 기계에서 깨진다(개발자가 로컬에서 재현할 수 없는 상태). ⑵PATH 주입이 실패하거나 순서가 바뀌면 T6 이 본 적 없는 인터프리터가 팩 스크립트를 돌린다. T6 의 음성 대조는 `$RUNNER_TEMP\guard_probe` 에 검체를 심고 **하드코딩된 그 exe** 로만 확인하므로 이 축을 전혀 다루지 않는다. 즉 '문제의 런타임에서 실제로 돈다'는 T6 의 존재 이유가 전제 미검증 위에 있다.

**닫는 방법**: (a) T6 음성 대조를 **하드코딩 경로가 아니라 훅과 동일한 해소 경로**(`command -v python3` 를 cysd 가 주입한 PATH 아래에서)로 한 번 더 돌려, 두 결과가 같은 실행파일을 가리키는지 확인하라(다르면 fail-closed). (b) 훅의 `CYS_PY` 해소를 PATH 탐색이 아니라 **번들 런타임 절대경로 우선**으로 고정하고(부재 시에만 폴백), 그 규칙을 게이트가 검사하라. (c) 그렇게 고정할 수 없다면, 게이트의 계약 문서에 '이 게이트는 번들 인터프리터로 실행될 때만 유효하다'를 명시하고 어느 비율의 실행이 그 조건인지 측정하라.

## 생산에 D1 탐지기가 여전히 0개다 — 훅 fail-open 은 그대로다. D1 이 릴리스 하나를 조용히 산 원인은 '정적 게이트가 없었다'가 아니라 '실패가 무증상이었다'인데, 3라운드 24건 전부가 사전(pre-merge) 게이트만 봤다. 실측: 훅은 광범위하게 fail-open(`hooks/appbuild-gate.sh:12` `command -v python3 ... || exit 0  # fail-open`, `hooks/actprobe-kill-gate.sh:151-186` 다중 `exit 0 # 인프라 fail-open`, `hooks/cys-hook.sh:10` '절대 차단 금지·항상 exit 0'). 그리고 게이트가 닿지 않는 유입 경로가 실재한다 — 사용자·에이전트의 로컬 편집, `CYS_PACK_DIR` 부서 팩, `pack-update` 후 `~/.cys/pack/bin` 에 남는 구버전 파일, `skills/*/scripts`(D6-3).

**왜 중요한가**: 게이트는 '이 저장소를 통과하는 변경'만 막는다. 그 밖의 모든 유입 경로로 들어온 D1 은 오늘도 정확히 2026-07-29 이전과 같은 방식으로 — 훅이 exit 0 을 내고, 아무 로그도 남지 않고, 사람 눈에만 의존해 — 산다. 8차원은 '스위트가 초록인데 Windows 에서 D1 이 재발하는 경로'를 분석기 내부의 우회 형태로만 찾았고, **분석기를 아예 거치지 않는 경로**는 목록에 없다. 탐지가 없는 한 다음 D1 의 발견 지연은 다시 릴리스 단위가 된다. 사전 게이트를 4라운드까지 벼려도 이 값은 개선되지 않는다.

**닫는 방법**: (a) 팩 스크립트의 형제 import 실패를 생산에서 **보이게** 만들어라 — 훅 래퍼가 자식의 stderr 에서 `ModuleNotFoundError: No module named 'javis_*'` 를 감지하면 exit 0 을 유지하되(차단 금지 교리 보존) `cys` 이벤트/알림으로 1회 방출. 무증상성만 깨면 된다. (b) 부팅 1회 셀프체크: `javis_preflight.py` 에 '설치된 `$CYS_PACK_DIR/bin` 전 파일을 번들 인터프리터로 import-only 스모크' 를 추가해 유입 경로와 무관하게 잡아라 — 이건 정적 분석 없이 부류 전체를 닫는다. (c) 그 셀프체크가 생기면 3라운드의 BLOCK 다수가 등급 재산정 대상이 된다(사전 게이트 단독 의존이 끝나므로).

## 게이트가 **배송 산출물이 아니라 checkout 소스**를 검사한다. T6 은 인터프리터만 설치본에서 가져오고 파일은 checkout 에서 가져온다 — `windows-build.yml:475`: `$suite = Join-Path (Get-Location) 'cysjavis-pack\bin\tests\test_import_guard.py'`, 그리고 스위트 내부 `BIN = os.path.join(HERE, "..")` → checkout 의 `cysjavis-pack/bin/`. 설치된 팩(`$dir` 아래 / `~/.cys/pack/bin`)은 어느 층도 분석하지 않는다. 한편 배송 집합은 디스크 목록이 아니라 **git 추적 목록**에서 파생된다(`build.rs:48-60`: `git ls-files` 결과에서 dot·`tests`·`__pycache__` 컴포넌트 제외 → 바이너리 임베드 PACK). 즉 '분석 대상 = `os.listdir(bin)`' 과 '배송 대상 = 추적 파일' 은 서로 다른 두 집합이고 둘이 같음을 검증하는 것이 없다.

**왜 중요한가**: 게이트가 세우는 명제는 '소스에 가드가 있다'이지 '배송되어 설치된 파일에 가드가 있다'가 아니다. 그 사이에는 임베드 PACK 코드젠, `pack.tar.gz`, `cys pack-update --from` 세 개의 배송 메커니즘이 있고 어느 것의 출력도 재분석되지 않는다. 특히 부분 업그레이드로 `~/.cys/pack/bin` 에 남는 구버전 파일은 정의상 소스에 없으므로 게이트가 원리적으로 볼 수 없다. 이 저장소가 반복해 당한 실패 형태가 '레인 하나를 빼먹음'(ci-branch.yml:39 자인)인데, 여기서는 '층 하나를 빼먹음'이 같은 구조로 있다 — 검사하는 것과 배송되는 것이 다른 객체다.

**닫는 방법**: (a) T6 에 한 스텝을 더해 **설치된 팩 폴더**를 대상으로 같은 분석을 돌려라 — 스위트가 `--bin <경로>` 인자를 받게 하고 T6 가 `$dir` 아래 실제 pack bin 을 넘긴다(1회 실행 추가). (b) 배송 집합과 분석 집합의 일치를 기계로 세워라: `git ls-files cysjavis-pack/bin/*.py` 와 `os.listdir(BIN)` 의 차집합이 비지 않으면 FAIL(양방향). (c) `pack-update`/설치 경로가 구버전 `.py` 를 제거하는지(prune) 확인하고, 제거하지 않는다면 그 잔류 파일이 게이트 밖이라는 사실을 계약 문서의 '증명하지 않는 것' 절에 명시하라.

---

# 부록 B — 기각된 지적

- **D5-3** ③ 의 내부 timeout 예산(8×120s=960s)이 T6 스텝 예산(timeout-minutes: 5 = 300s)의 3.2배 — 설계된 진단 경로가 도달 불가능하다
  - 기각 사유: 기각. 산술(8×120s=960s vs 300s)은 내 실측으로도 그대로 재현되지만, **지적의 표제 주장 '설계된 진단 경로가 도달 불가능하다 / 죽은 코드가 된다'가 실행으로 반증된다.**

실제로 매달리는 검체(time.sleep(9999))를 만들어 runtime_probe 를 돌렸더니 정확히 120.0s 에 TimeoutExpired 분기가 발동하고 의도한 진단 문구를 그대로 반환했다. 나머지 7종 실측 합계가 0.24s 이므로 1종 행 = 총 120.2s, 2종 행 = 240.2s 로 둘 다 T6 예산 300s 안에서 완주한다. 즉 그 분기는 죽은 코드가 아니라 정상 도달 가능한 코드이며, 지적자 자신도 본문에서 '최대 2회분까지 발동 가능'이라고 적어 표제와 모순된다.

3종 이상이 동시에 매달려야 스텝이 먼저 죽는데, ① 판정에 미치는 영향이 0이다 — 첫 행 하나만으로 이미 ③ 이 FAIL 이라 스위트는 exit 1 이고, 스텝이 GitHub 에 먼저 죽어도 스텝 실패=잡 실패로 동일하게 fail-closed 다. 어느 경로든 최종 판정은 같고, 달라지는 것은 에러 메시지의 구체성뿐이다.
② 촉발 조건의 실물 근거가 0이다 — ③ 대상 8종의
- **D5-4** 자식 프로세스 격리는 sys.path·sys.modules 에만 성립한다 — 환경변수·HOME·cwd 는 8종 프로브가 그대로 공유한다
  - 기각 사유: 기각. 관측 사실 자체는 재현되지만(자식이 부모 env·HOME 을 상속, 8종이 중립 cwd 하나를 공유), 그로부터 '격리 계약 위반'이라는 결론이 따라나오지 않는다.

① 소스가 실제로 한 주장은 지적이 인용한 것보다 좁고, 좁은 쪽이 참이다. 556-557행은 'sys.path·sys.modules 로 뒤 검체가 무임승차한다 … 프로세스를 나누면 **그 부류**가 통째로 사라진다' 로, '그 부류'가 바로 앞 문장의 sys.path/sys.modules 를 가리키도록 명시적으로 한정돼 있다. 그 한정된 주장은 참이다. 무한정 표현은 20행 한 줄('검체 간 오염 0')뿐이고, 이건 주석 문구 다듬기 수준이다.

② env 상속은 격리 실패가 아니라 의도된 충실도다. 프로브의 목적은 '팩 스크립트를 Windows 배송 조건에서 실제로 돌린다'이고, 팩 스크립트는 CYS_PACK_DIR 등 환경변수를 정당하게 읽는다. env 를 백지로 만들면 재현하려는 실런타임과 **덜** 같아진다. 모델링해야 할 조건(._pth 로 스크립트 폴더 미등재)의 유일한 오염원이 PYTHONPATH 이고 그것만 제거한 것은 설계대로다. 재현 코드가 보인 D5_SECRET 누출
- **T6-2** 본 검증에는 음성 대조가 가진 '실패의 이유' 구분이 없다 — 스위트의 인코딩 크래시가 'D1 재발'로 오귀인된다 (PLAUSIBLE · 실행 검증 필요)
  - 기각 사유: 기각 — 근거로 든 '저장소 선례 두 건'이 **오독**이고, 핵심 전제(embeddable 에서 PYTHONUTF8 무효)는 지적자 본인도 미검증인 채 남았으며, 내가 재현 가능한 범위에서는 방어가 실제로 작동한다.

(a) 오독: javis_bootstrap.py:50-51 주석은 'PYTHONUTF8 이 부족하다'가 아니라 "PYTHONUTF8 **export는 cys-dept 경로에만 있어** 이 스크립트의 **직접 실행**을 보호하지 못한다" 다 — 환경변수가 **안 걸려 있는 경로**가 문제라는 뜻이다. T6 스텝은 그 변수를 스텝 `env:` 로 **명시 설정**한다(:429). 선례가 지목한 결함 조건이 T6 에는 성립하지 않는다.
(b) 오독 2: T5 스텝도 `PYTHONUTF8: '1'` 을 설정한다(windows-build.yml:382). 즉 javis_phoenix_win_smoke.py:56-57 의 reconfigure 는 PYTHONUTF8 실패의 증거가 아니라 **CI 밖 직접 실행용 이중 방어**다. "T5 의 초록은 embeddable 이 한글을 뱉는다를 증명하지 않는다"는 문장은 성립하지만, 그로부터 'PYTHONUTF8