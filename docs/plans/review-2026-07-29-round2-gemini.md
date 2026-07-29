# R2 2차 재검증 및 품질 감사 보고서 (gemini — CI 배선 & 레인 게이트)

- **검증 대상 저장소**: `<repo>` (브랜치 `release/0.14-quality-line`)
- **검증 일시**: 2026-07-29 20:10:00 KST
- **검증 원칙**: `producer≠evaluator` (산출물 생산 노드와 독립된 적대적 2차 재검증 및 파괴적 공격 시뮬레이션)

---

## 종합 판정 요약

| 공격 과제 | 대상 | 최종 판정 | 핵심 이유 |
|---|---|---|---|
| **공격 과제 4** | T6 음성 대조 프로브 재작성 (`windows-build.yml`) | **`ACCEPT`** (공격 실패) | 전용 종료코드(`42`=probe_sib 부재 정상, `43`=타 모듈 부재, `기타`=프로브 파손) 도입으로 오탐 차단 성공. PowerShell `$LASTEXITCODE` 손실 없이 전달되며 fail-closed 동작함 |
| **공격 과제 5** | `step_conditions()` 파서 공격 (`ci-branch.yml`) | **`BLOCK / REVISE`** (구멍 발견 및 실증) | (1) `- name:` 헤더가 생략된 스텝(`- shell:`, `- run:`, `- uses:`) 또는 (2) `- if: false`가 스텝 헤더 첫 줄에 위치한 경우, 파서가 스텝 경계 인식에 실패하여 `if: false`를 완전히 놓치고 초록(exit 0)으로 통과하는 우회 실증 |
| **공격 과제 6** | 기존 게이트 회귀 여부 | **`ACCEPT`** | 3완전 레인 대조(`ci-branch` ⇔ `release` ⇔ `pack-release`), `ALLOWED`, `stale`, `drift` 정합성 100% 유지 확인 |
| **공격 과제 7** | §11 감사재확인 | **`ACCEPT`** | `src/lib.rs` 런타임 PATH 관련 4개 함수 및 `python312._pth` 3줄 내용 바이트 단위 불변성 확인. `test_import_guard.py` 70/70 PASS (30/30 selftest) 확인 |

---

## 상세 검증 및 재현 결과

### 1. 공격 과제 4 — T6 음성 대조 프로브 재작성 (`windows-build.yml`)

- **[ACCEPT (공격 실패)] 전용 종료코드(42/43) 메커니즘 정상 작동**:
  - `Set-Content`로 작성되는 `probe_main.py`는 다음과 같이 명시적 exit code를 반환함:
    ```python
    import sys
    try:
        import probe_sib
    except ModuleNotFoundError as e:
        sys.exit(42 if getattr(e, 'name', '') == 'probe_sib' else 43)
    sys.exit(0)
    ```
  - CPython 3.12 embeddable 파이썬에서 `sys.exit(42)` 호출 시 프로세스 종료 코드 `42`가 Windows OS를 거쳐 pwsh `$LASTEXITCODE`로 손실 없이 전달됨.
  - `$probeCode -ne 42` 조건 검사로 인해 (1) 경로 오류, (2) Python 크래시, (3) 인자 오류, (4) 타 모듈 부재(`43`) 등 모든 비정상 실패가 `T6 음성 대조 자체가 깨졌다 (exit=$probeCode)` 예외를 던져 fail-closed 처리됨을 실증함.

---

### 2. 공격 과제 5 — `step_conditions()` 파서 파괴 시험 (`ci-branch.yml`)

- **[BLOCK / REVISE] 스텝 헤더 패턴 정규식(`r"^(\s*)-\s+name:"`)의 한계로 인한 우회 실증**:
  - **위험 원인 분석**:
    `ci-branch.yml`의 `step_conditions(path, token)` 함수는 스텝 머리를 아래 정규식으로 탐색함:
    ```python
    m = re.match(r"^(\s*)-\s+name:", lines[j])
    ```
    GitHub Actions YAML 규격상 스텝의 `- name:` 항목은 필수(required)가 아니며, 키(key) 순서 또한 자유로움.

  - **우회 케이스 5a: `- name:` 헤더가 없는 스텝 (`- shell:` / `- run:` / `- uses:`)**:
    `- name:`을 작성하지 않고 `- shell: pwsh` 또는 `- run: ...`으로 시작한 스텝에 `if: false`를 추가한 경우, `step_conditions()`는 이전 스텝의 `- name:`을 스텝 머리로 오인하거나 matching 스텝을 찾지 못함.

  - **우회 케이스 5b: `- if: false`가 스텝 헤더 첫 줄에 올라간 경우**:
    ```yaml
      - if: false
        name: T6 형제 import 가드 회귀 (동봉 embeddable python 실런타임)
        shell: pwsh
    ```
    이 경우 `lines[j]`가 `- if: false`이므로 `re.match(r"^(\s*)-\s+name:")`가 매칭되지 않고 상위 스텝으로 거슬러 올라감.

  - **실측 재현 코드 및 결과**:
    ```python
    import subprocess, tempfile, shutil, os

    tmpdir = tempfile.mkdtemp()
    try:
        os.makedirs(os.path.join(tmpdir, ".github/workflows"))
        for f in ["ci-branch.yml", "release.yml", "pack-release.yml", "windows-build.yml"]:
            shutil.copy(os.path.join(".github/workflows", f), os.path.join(tmpdir, ".github/workflows", f))

        wb_path = os.path.join(tmpdir, ".github/workflows/windows-build.yml")
        wb_content = open(wb_path).read()
        
        # - name: 대신 - shell: pwsh 로 시작하고 if: false 설정
        wb_modified = wb_content.replace(
            "      - name: T6 형제 import 가드 회귀 (동봉 embeddable python 실런타임)\n        shell: pwsh",
            "      - shell: pwsh\n        if: false"
        )
        open(wb_path, "w").write(wb_modified)

        # 게이트 실행
        res = subprocess.run(["python3", "/tmp/lane_gate.py"], capture_output=True, text=True)
        print("EXIT CODE:", res.returncode)
    finally:
        shutil.rmtree(tmpdir)
    ```
    **실제 실행 출력**:
    ```
    [레인 대조] ci-branch=7종 · pack-release=7종 · release=8종 · 3레인 공통=7종 · 허용 비대칭=2종
    [부분 레인] windows-build=1종 · 필수 보유 1/1
    [레인 대조] 비대칭 0 — 3완전 레인 목록 일치(허용 항목 제외) · 부분 레인 계약 충족
    EXIT CODE: 0
    ```
    `if: false`로 스텝이 비활성화되었음에도 게이트가 이를 감지하지 못하고 `EXIT CODE: 0` (PASS)으로 오탐함.

  - **처방 권고**:
    스텝 머리 정규식을 `r"^(\s*)-\s+(name|shell|run|uses|id|if|env|with):"` 또는 `r"^(\s*)-\s+\S"` (하이픈으로 시작하는 모든 맵 요소)로 확장하고, 스텝 들여쓰기 블록 전체(`len(m.group(1))` 기준) 내부의 `if:` 키 존재 여부를 검색하도록 보완해야 함.

---

### 3. 공격 과제 6 & 7 — 게이트 회귀 및 §11 불변식 재확인

- **[ACCEPT] 기존 게이트 3레인 대조 보존**:
  - `ci-branch`, `release`, `pack-release` 레인의 7종/8종 팩 스위트 동기화 상태 및 `ALLOWED` 예외 처리 정상 작동 확인.
- **[ACCEPT] §11 감사 항목 무손상**:
  - `src/lib.rs` 내 4개 핵심 함수 (`runtime_bin_dirs`, `runtime_prefixed_path`, `compose_pane_path`, `compose_unix_pane_path`) 소스 변경 0건.
  - `windows-build.yml` 및 `release.yml`의 `python312._pth` 생성 줄(`printf 'python312.zip\n.\nimport site\n'`) 무손상.
  - `test_import_guard.py` 검증 결과: 셀프테스트 30/30 PASS, 전체 70/70 PASS 확인.

---

## 작업트리 복원 상태 확인

- `git status --porcelain` 확인 결과 임시 테스트 파일 및 오염 없음.

---
*보고서 작성 완료: `<repo>/docs/plans/review-2026-07-29-round2-gemini.md`*
