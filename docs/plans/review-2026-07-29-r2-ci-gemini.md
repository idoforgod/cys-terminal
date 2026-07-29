# R2 산출물(CI 배선) 적대적 검증 및 품질 감사 보고서 (reviewer2)

- **검증 대상 저장소**: `<repo>` (브랜치 `release/0.14-quality-line`)
- **검증 일시**: 2026-07-29 19:56:00 KST
- **검증 원칙**: `producer≠evaluator` (산출물 생산 노드와 독립된 적대적 감사 및 비판적 검증)

---

## 종합 판정 요약

| 공격 과제 | 대상 | 최종 판정 | 핵심 이유 |
|---|---|---|---|
| **공격 과제 A** | `windows-build.yml` T6 스텝 | **`REVISE`** | 음성 대조(`$probeCode`)가 non-zero exit code만 확인할 뿐, `ModuleNotFoundError: probe_sib`에 의해 실패했는지를 검증하지 않아 환경/구문 오류 시 오탐(false pass) 발생 리스크 존재 |
| **공격 과제 B** | `ci-branch.yml` `SUBSET_LANES` 레인 게이트 | **`REVISE`** | 텍스트 추출 방식의 한계로 인해 T6 스텝에 `if: false`가 설정되어 CI에서 실행되지 않더라도 게이트가 0건 차집하로 인식해 초록(exit 0)을 반환하는 사각지대 실증 |
| **공격 과제 C** | §11 보호 대상 및 불변식 감사 | **`ACCEPT`** | `src/lib.rs` 런타임 PATH 관련 4개 함수 및 `python312._pth` 3줄 바이트 불변 100% 무손상 확인 |

---

## 공격 과제 A — T6 스텝 (`windows-build.yml`) 상세 검증

### 1. PowerShell 구문·의미 오류 및 실행 환경 분석

- **[ACCEPT] 구문 및 런타임 변수 처리**:
  - `$ErrorActionPreference = 'Stop'` 설정 하에서 `& $py ...` 네이티브 명령 호출 시, stderr를 캡처(`2>&1`)하지 않고 셸 로그로 바로 흘리도록 설계되어 `NativeCommandError` 예외 발화를 회피함.
  - `$env:RUNNER_TEMP`는 GitHub Actions `windows-latest` 러너 환경에서 `D:\a\_temp`로 정상 제공되며 `Join-Path` 결합에 문제 없음.
  - `PYTHONUTF8: '1'` 환경변수가 명시되어 Windows 기본 cp1252 런타임에서의 한글 print UnicodeEncodeError를 차단함.

- **[REVISE / MAJOR] 음성 대조(`$probeCode`) 실패 원인 미검증 (false negative control pass)**:
  - **위험 증명**:
    ```powershell
    & $py (Join-Path $probe 'probe_main.py')
    $probeCode = $LASTEXITCODE
    if ($probeCode -eq 0) {
      throw "T6 전제 붕괴..."
    }
    ```
    위 로직은 `$probeCode`가 0이 아니기만 하면 음성 대조가 성공한 것으로 간주함. 만약 `probe_main.py` 경로 오류, Python CLI 인자 오류, DLL 부재 또는 다른 오타로 인해 Python이 exit 1로 종료되면, `sys.path`에 스크립트 폴더가 포함되어 있는지 여부와 상관없이 음성 대조를 무조건 통과(pass) 처리함.
  - **처방 권고**:
    음성 대조 실패 시 stderr 출력에 `ModuleNotFoundError` 및 `probe_sib`가 실제 포함되었는지를 파일 리다이렉션(`2> $errFile`) 또는 Python 내부 `-c` 래퍼로 직접 확인하도록 강화할 것.

- **[REVISE / MINOR] `$env:LOCALAPPDATA` 재귀 검색 비효율**:
  - `Get-ChildItem "$env:LOCALAPPDATA" -Recurse -Filter cysd.exe` 호출이 T3, T4, T5, T6에 걸쳐 반복 호출됨. `LOCALAPPDATA\cys`로 검색 범위를 좁히지 않고 전체 `$env:LOCALAPPDATA`를 검색하므로 디렉터리 수천 개를 탐색함. `timeout-minutes: 5` 이내에 완료는 되나 탐색 경로를 지정하는 것이 바람직함.

---

## 공격 과제 B — 레인 대조 게이트 (`ci-branch.yml`) 상세 검증

### 1. `SUBSET_LANES` (⊆ · ⊇) 계약 검증

- **[REVISE / MAJOR] 게이트 텍스트 스캔의 사각지대 실증 (`if: false` 무력화)**:
  - **재현 시험**: `windows-build.yml`의 T6 스텝에 `if: false` 조건을 주입하여 실제로 스텝이 실행되지 않도록 설정한 후 레인 대조 게이트를 실행함.
  - **실행 결과**:
    ```
    [레인 대조] ci-branch=7종 · pack-release=7종 · release=8종 · 3레인 공통=7종 · 허용 비대칭=2종
    [부분 레인] windows-build=1종 · 필수 보유 1/1
    [레인 대조] 비대칭 0 — 3완전 레인 목록 일치(허용 항목 제외) · 부분 레인 계약 충족
    (Exit Code: 0)
    ```
  - **원인 및 영향**: 게이트는 YAML의 AST/실행 조건(`if:`)을 해석하지 않고 텍스트에서 `test_[a-z0-9_]+` 이름만을 정규식으로 추출함. 따라서 `windows-build.yml` 내에 `test_import_guard` 문자열이 남아 있는 한, 해당 스텝이 `if: false`로 비활성화되어 있거나 주석 처리되어 있더라도 부분 레인 계약 통과(exit 0) 판정을 내림.
  - **평가**: 이는 주석 문맥에 명시된 게이트의 정직한 한계(1)와 일치하나, `SUBSET_LANES`가 "실제 Windows 런타임에서의 1회 이상 실행"을 보장하는 목적을 가진 만큼, 스텝 비활성화에 대한 주의가 필요함.

- **[ACCEPT] fail-closed 및 차집합 억제 수용**:
  - `SUBSET_LANES`에 등재되지 않은 신규 팩 테스트가 `windows-build.yml`에만 추가될 경우 `have - union`에 의해 `부분 레인 계약 위반` 에러(exit 1)가 정확히 발화함.
  - `REQUIRED` 집합(`{"test_import_guard"}`)의 스위트 이름이 `windows-build.yml`에서 완전히 지워지면 `names()`가 0건 추출 또는 `required - have`에 의해 즉시 fail-closed(exit 1)됨을 확인함.

---

## 공격 과제 C — §11 감사 체크리스트 검증

독립적 검증 결과, 생산자 산출물이 핵심 불변식을 완벽히 준수하였음을 확인함:

1. **`src/lib.rs` 핵심 런타임 PATH 함수 4종 무손상**:
   - `runtime_bin_dirs`, `runtime_prefixed_path`, `compose_pane_path`, `compose_unix_pane_path` 소스 변경 0건.
2. **`python312._pth` 3줄 내용 무변경**:
   - `.github/workflows/windows-build.yml` 및 `release.yml`의 `printf 'python312.zip\n.\nimport site\n'` 변경 0건(100% 바이트 동일).
3. **get-pip 부트스트랩 및 cys.rs/governance.rs 무변경**:
   - `src/bin/cys.rs`, `src/bin/cysd/governance.rs` 파일 변경 0건.
4. **회귀 테스트 `test_import_guard.py` 실행 검증**:
   - `python3 cysjavis-pack/bin/tests/test_import_guard.py --selftest`: 12/12 PASS
   - `python3 cysjavis-pack/bin/tests/test_import_guard.py`: 52/52 PASS
5. **작업트리 오염 여부 검증**:
   - `git status --porcelain` 확인 결과 지정된 미커밋 파일 외의 무관 오염 없음.

---

## 재현 명령 및 검증 기록

```bash
# 1. test_import_guard.py 셀프테스트 및 전체 52종 검증
python3 cysjavis-pack/bin/tests/test_import_guard.py --selftest && python3 cysjavis-pack/bin/tests/test_import_guard.py
# 출력: 12/12 PASS (selftest) -> 52/52 PASS

# 2. 레인 대조 게이트 실행
python3 - <<'EOF' > /tmp/lane_gate.py
import re
src = open(".github/workflows/ci-branch.yml", encoding="utf-8").read()
m = re.search(r"LANE-GATE-SELF-BEGIN.*?python3 - <<'PY'\n(.*?)\n\s*PY\n", src, re.S)
print("\n".join(l[10:] if l.startswith(" "*10) else l for l in m.group(1).splitlines()))
EOF
python3 /tmp/lane_gate.py
# 출력: [레인 대조] 비대칭 0 — 3완전 레인 목록 일치(허용 항목 제외) · 부분 레인 계약 충족

# 3. Rust 및 보호 대상 파일 변경 0건 확인
git diff src/lib.rs src/bin/cys.rs src/bin/cysd/governance.rs
# 출력: (빈 출력 - 0줄 변경)
```

---
*보고서 작성 완료: `<repo>/docs/plans/review-2026-07-29-r2-ci-gemini.md`*
