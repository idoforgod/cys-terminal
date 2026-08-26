<!-- doc-command-pin: allow(unguardedLn, unguardedRm) — reason: 상위대체된 이력 문서.
     이 파일의 명령 블록은 "지금 실행하라"가 아니라 "2026-06-29 에 무엇을 했는가"의 기록이다.
     원문을 보존해야 이력으로서 값이 있으므로 명령 텍스트를 고치지 않는다. 실행 정본은
     docs/INSTALL.md §B 의 가드 블록이다. 이 예외를 다른 파일로 넓히지 마라. -->

# ✅ [완료 · 상위대체됨] "Install cys command in PATH" 구현 계획 (CLI PATH 통합결함 근본교정)

> # 🛑 이 문서를 실행 지시로 읽지 마라
>
> - **이 플랜은 2026-06-29 에 완료됐다** — 커밋 `1882a5b`
>   (`feat(cli-path): Control Center button to install cys CLI to PATH`).
>   아래 체크박스는 그때 전부 이행된 것이며, 남은 작업 목록이 아니다.
> - **2026-08-25 설계 정본 [`docs/plans/2026-08-25-shell-cli-restore-design.md`](2026-08-25-shell-cli-restore-design.md)
>   이 이 문서를 상위대체(supersede)한다.** 설계·가드·IPC 계약이 8라운드에 걸쳐 바뀌었고,
>   이 문서의 설계 결정 중 최소 2건은 **그 라운드들에서 결함으로 판정돼 폐기**됐다(바로 아래).
> - **여기의 명령을 그대로 복사하지 마라.** `docs/INSTALL.md` §B 의 **가드 블록이 정본**이다.
>
> **폐기된 설계 결정 (이 문서 안에 아직 옛 형태로 남아 있는 것)**
>
> | 이 문서가 말하는 것 | 지금 사실 | 근거 |
> |---|---|---|
> | 가드 5종 ⑤ "멱등·self-heal — `ln -sf` 로 재실행 시 stale 심볼릭 자동 교체"(이 파일 §설계 결정 5번) | **폐기.** `ln -sf` 는 그 자리의 **남의 실체 파일까지 말없이 파괴**한다. 지금은 `[ -e ] \|\| [ -L ]` → `readlink` 로 우리 번들 대조 → 아니면 `<원래 경로>.cys-backup-<epoch초>` 로 **백업**한 뒤 `ln -sfn`. | 설계 정본 §4.1 BLOCK-1 · §5 ② |
> | Task 5 Step 1 이 `docs/INSTALL.md` §B 에 써 넣으라고 지시하는 **맨 `sudo ln -sf` 두 줄**(이 파일 Task 5 Step 1 의 `INSTALL.md §B 교체안` 블록 안) | **폐기.** 그 두 줄은 2026-08-25 8라운드에 문서에서 제거됐고, 자리에는 백업 선행 가드 블록이 들어갔다. 이 지시를 그대로 따르면 **그 수리가 원상복구된다.** | 설계 정본 §4.11 · `docs/INSTALL.md` §B |
>
> **이 문서를 남겨 두는 이유**: 지우면 "왜 그때 그렇게 결정했는가"를 되물을 수 없다.
> 이력으로 읽되, 실행 지시로는 읽지 마라.

> ~~**For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.~~
> — **무효(2026-08-25).** 이 플랜은 이미 구현·완료됐다. 새로 착수할 대상이 아니다.
> 이 레인의 현행 지시는 설계 정본 §9(미결·인수인계)에 있다.

**Goal:** macOS DMG로 설치한 사용자가 GUI에서 1번 클릭하면(1회 관리자 승인) `cys`·`cysd`가 `/usr/local/bin`에 심볼릭으로 노출되어 외부 터미널·오케스트레이션에서 즉시 쓰이게 한다.

**Architecture:** VS Code "Install 'code' command in PATH"와 동형의 **명시 동의(explicit-consent) 메뉴 명령**. 새 Tauri 커맨드 `install_cli_to_path`가 ① 실행 중 번들 경로를 분류(정규/translocated/백업/비표준)하고 ② translocated·백업이면 거부+안내, ③ 정규/비표준이면 `osascript ... with administrator privileges` **1회 승격**으로 `/usr/local/bin`에 `cys`·`cysd` 심볼릭을 멱등 생성, ④ `which -a cys`로 실효 해석을 검증·그림자화를 보고한다. 첫 실행 자동화·dotfile 편집은 **하지 않는다**(오너 결정: 명시 메뉴 + `/usr/local/bin`). 가드 로직 전체를 순수함수 `plan_cli_install()`로 분리해 osascript 실행만 빼고 전량 단위테스트한다.

**Tech Stack:** Rust (Tauri 2, std only — 신규 crate 불요), TypeScript(UI), macOS `osascript`/`bash -lc`. 추가 의존성 없음(`Cargo.toml` 그대로).

**범위 경계(이 계획에 포함하지 않음):** ⓐ 첫 실행 무프롬프트 자동설치(오너이 명시 메뉴 선택) ⓑ `~/.local/bin` + dotfile 편집 ⓒ `.pkg`/brew tap 보조 채널 ⓓ 0.4.6/0.4.7 미공증 릴리스 위생(별개 트랙 — 부록 A 참조, 이 계획의 **선결 전제**이나 코드 작업 아님).

**근본원인 요약(이 계획이 메우는 통합 공백):** cys/cysd를 로그인 셸 PATH에 올리는 단계가 빌드·번들·CI·런타임 어디에도 없음. 오너 머신만 동작한 이유는 dev 전용 `scripts/deploy_gate.py`가 `/opt/homebrew/bin`에 바이너리를 **복사**(심볼릭 아님)했기 때문. GUI·데몬·내부 pane은 `current_exe().parent()` 형제 절대경로 해석(`main.rs:460-465`)·pane PATH 주입(`state.rs:759-771`)으로 정상 — **결함은 "사용자가 직접 연 외부 로그인 셸"의 cys/cysd 1점**에 국한.

---

## File Structure

| 파일 | 책임 | 작업 |
|---|---|---|
| `src-tauri/src/main.rs` | 순수 가드/스크립트 헬퍼 + `install_cli_to_path` 커맨드 + `generate_handler!` 등록 + 단위테스트 | Modify |
| `ui/index.html` | Control Center 헤더(`#cc-header`)에 "셸에 cys 설치" 버튼 | Modify |
| `ui/src/main.ts` | 버튼 클릭 핸들러 → `invoke("install_cli_to_path")` + 결과 알림 | Modify |
| `docs/INSTALL.md` | §B를 "1클릭 메뉴(권장) + sudo ln(폴백)"으로 개정, INST-DENY-02 주석 정합 | Modify |
| `README.md` | 설치 절에 "터미널에서 cys 쓰려면 1클릭 메뉴" 1줄(발견성 갭 제거) | Modify |

**설계 결정(가드 5종):**
1. **명시 동의** — 첫 실행 자동 아님, 사용자가 버튼 클릭 시에만 발동(INST-DENY-02 "사람 동의" 충족).
2. **translocation 거부** — 번들 경로에 `/AppTranslocation/` 포함 시 거부+"Finder로 Applications에 옮긴 뒤 재시도" 안내(Finder 이동=de-translocate).
3. **백업 번들 거부** — 번들명이 `cys.app.bak*`/`*.prev*`면 거부(stale 바이너리 심볼릭 차단).
4. **그림자화 보고(비차단)** — 설치 후 `which -a cys` 1순위가 `/usr/local/bin/cys`가 아니면 경고로 보고(오너 머신의 `/opt/homebrew/bin` 선행을 정직하게 표면화, 단 무손상이므로 차단하지 않음).
5. ~~**멱등·self-heal** — `ln -sf`로 재실행 시 stale 심볼릭 자동 교체.~~
   🛑 **폐기(2026-08-25).** `ln -sf` 는 남의 실체 파일도 말없이 파괴한다 — 지금은 백업 선행
   가드가 정본이다(설계 정본 §4.1 BLOCK-1 · `docs/INSTALL.md` §B).

---

## Task 1: 순수 가드/스크립트 헬퍼 (TDD)

**Files:**
- Modify: `src-tauri/src/main.rs` — 헬퍼는 `resolve_sidecar`(`main.rs:460`) 근처에 추가, 테스트는 기존 `#[cfg(test)] mod tests`(파일 말미 `mod tests { use super::*; ... }`) 안에 추가.
- Test: 동일 파일 `mod tests`.

- [x] **Step 1: 실패 테스트 작성** — `src-tauri/src/main.rs`의 `mod tests { ... }` 안에 추가:

```rust
    // ── CLI PATH 설치 헬퍼 ──────────────────────────────────────────
    #[test]
    fn sh_squote_escapes_spaces_and_quotes() {
        assert_eq!(sh_squote("/usr/local/bin"), "'/usr/local/bin'");
        assert_eq!(sh_squote("/Users/x/a b/cys.app"), "'/Users/x/a b/cys.app'");
        // 단일따옴표는 '\'' 시퀀스로 안전 이스케이프
        assert_eq!(sh_squote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn build_install_script_emits_idempotent_symlinks() {
        let cys = std::path::Path::new("/Applications/cys.app/Contents/MacOS/cys");
        let cysd = std::path::Path::new("/Applications/cys.app/Contents/MacOS/cysd");
        let s = build_install_script(cys, cysd, "/usr/local/bin");
        assert_eq!(
            s,
            "mkdir -p '/usr/local/bin' && \
ln -sf '/Applications/cys.app/Contents/MacOS/cys' '/usr/local/bin/cys' && \
ln -sf '/Applications/cys.app/Contents/MacOS/cysd' '/usr/local/bin/cysd'"
        );
    }

    #[test]
    fn classify_bundle_dir_distinguishes_canonical_translocated_backup_nonstandard() {
        use std::path::Path;
        assert_eq!(
            classify_bundle_dir(Path::new("/Applications/cys.app/Contents/MacOS")),
            BundleKind::Canonical
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Users/x/Applications/cys.app/Contents/MacOS")),
            BundleKind::Canonical
        );
        assert_eq!(
            classify_bundle_dir(Path::new(
                "/private/var/folders/aa/bb/AppTranslocation/CCCC/d/cys.app/Contents/MacOS"
            )),
            BundleKind::Translocated
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Applications/cys.app.bak-044/Contents/MacOS")),
            BundleKind::Backup
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Applications/cys.app.prev-210050/Contents/MacOS")),
            BundleKind::Backup
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Users/x/Downloads/cys.app/Contents/MacOS")),
            BundleKind::NonStandard
        );
    }

    #[test]
    fn parse_which_a_returns_precedence_ordered_paths() {
        let out = "/Users/x/.local/bin/cys\n/opt/homebrew/bin/cys\n\n/usr/local/bin/cys\n";
        assert_eq!(
            parse_which_a(out),
            vec![
                "/Users/x/.local/bin/cys".to_string(),
                "/opt/homebrew/bin/cys".to_string(),
                "/usr/local/bin/cys".to_string(),
            ]
        );
    }

    #[test]
    fn plan_cli_install_refuses_translocated_and_backup() {
        // translocated → Err
        assert!(plan_cli_install(
            std::path::Path::new("/private/var/folders/x/AppTranslocation/Y/d/cys.app/Contents/MacOS"),
            "/usr/local/bin"
        ).is_err());
        // backup → Err
        assert!(plan_cli_install(
            std::path::Path::new("/Applications/cys.app.bak-044/Contents/MacOS"),
            "/usr/local/bin"
        ).is_err());
    }

    #[test]
    fn plan_cli_install_warns_on_nonstandard_but_proceeds() {
        let plan = plan_cli_install(
            std::path::Path::new("/Users/x/Downloads/cys.app/Contents/MacOS"),
            "/usr/local/bin"
        ).expect("nonstandard는 경고와 함께 진행");
        assert!(plan.osascript_arg.contains("with administrator privileges"));
        assert!(plan.warnings.iter().any(|w| w.contains("표준 위치")));
        assert_eq!(plan.cys_src, std::path::PathBuf::from("/Users/x/Downloads/cys.app/Contents/MacOS/cys"));
    }

    #[test]
    fn plan_cli_install_canonical_has_no_location_warning() {
        let plan = plan_cli_install(
            std::path::Path::new("/Applications/cys.app/Contents/MacOS"),
            "/usr/local/bin"
        ).expect("정규 번들은 진행");
        assert!(plan.warnings.iter().all(|w| !w.contains("표준 위치")));
        // osascript 인자는 do shell script + 승격 + 멱등 스크립트를 감싼다
        assert!(plan.osascript_arg.starts_with("do shell script '"));
        assert!(plan.osascript_arg.ends_with("' with administrator privileges"));
    }
```

- [x] **Step 2: 테스트 실패 확인**

Run: `cd ~/dev/cys-terminal && cargo test --bin cys-app cli_install 2>&1 | tail -20` (또는 `sh_squote`/`classify_bundle`/`plan_cli`/`build_install`/`parse_which` 이름으로)
Expected: FAIL — `cannot find function 'sh_squote'`, `cannot find type 'BundleKind'` 등 미정의 오류.

- [x] **Step 3: 최소 구현** — `main.rs`의 `resolve_sidecar`(약 460행) 바로 위/아래에 추가:

```rust
// ── CLI PATH 설치(명시 메뉴) — 가드/스크립트 순수 헬퍼 ─────────────────
#[derive(PartialEq, Debug)]
enum BundleKind {
    Canonical,    // /Applications/cys.app 또는 ~/Applications/cys.app
    Translocated, // Gatekeeper AppTranslocation 휘발 경로
    Backup,       // cys.app.bak-*/*.prev*
    NonStandard,  // 그 외(Downloads 등) — 경고와 함께 진행
}

/// 셸 단일따옴표 이스케이프(경로의 공백·특수문자·따옴표 안전).
fn sh_squote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `<bundle>/Contents/MacOS` 디렉토리를 분류한다.
fn classify_bundle_dir(macos_dir: &std::path::Path) -> BundleKind {
    let s = macos_dir.to_string_lossy();
    if s.contains("/AppTranslocation/") {
        return BundleKind::Translocated;
    }
    // macos_dir = <bundle>.app/Contents/MacOS → bundle = parent.parent
    let bundle = macos_dir.parent().and_then(|p| p.parent());
    if let Some(b) = bundle {
        let name = b
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.starts_with("cys.app.bak") || name.starts_with("cys.app.prev") {
            return BundleKind::Backup;
        }
        if name == "cys.app" {
            let parent = b
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            if parent == "/Applications" || parent.ends_with("/Applications") {
                return BundleKind::Canonical;
            }
        }
    }
    BundleKind::NonStandard
}

/// `do shell script` 본문: target_dir 생성 + cys·cysd 심볼릭 멱등 생성(`ln -sf`).
fn build_install_script(
    cys: &std::path::Path,
    cysd: &std::path::Path,
    target_dir: &str,
) -> String {
    format!(
        "mkdir -p {td} && ln -sf {c} {tc} && ln -sf {d} {tdd}",
        td = sh_squote(target_dir),
        c = sh_squote(&cys.to_string_lossy()),
        tc = sh_squote(&format!("{target_dir}/cys")),
        d = sh_squote(&cysd.to_string_lossy()),
        tdd = sh_squote(&format!("{target_dir}/cysd")),
    )
}

/// `which -a cys` 출력 → precedence 순 경로 리스트(공백줄 제거).
fn parse_which_a(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// 설치 계획(순수): 가드 판정 + 소스 경로 + osascript 인자 + 경고. osascript 실행은 포함하지 않는다.
struct CliInstallPlan {
    cys_src: std::path::PathBuf,
    cysd_src: std::path::PathBuf,
    target_dir: String,
    osascript_arg: String, // `do shell script '...' with administrator privileges`
    warnings: Vec<String>,
}

fn plan_cli_install(
    macos_dir: &std::path::Path,
    target_dir: &str,
) -> Result<CliInstallPlan, String> {
    match classify_bundle_dir(macos_dir) {
        BundleKind::Translocated => {
            return Err("cys.app이 Gatekeeper에 의해 임시 위치에서 실행 중입니다. \
Finder에서 cys.app을 Applications 폴더로 옮긴 뒤 다시 열고 시도하세요."
                .into());
        }
        BundleKind::Backup => {
            return Err("백업 번들에서 실행 중입니다. \
정규 cys.app(Applications)에서 실행한 뒤 시도하세요."
                .into());
        }
        BundleKind::Canonical | BundleKind::NonStandard => {}
    }
    let mut warnings = vec![];
    if classify_bundle_dir(macos_dir) == BundleKind::NonStandard {
        warnings.push(
            "cys.app이 표준 위치(Applications)가 아닌 곳에서 실행 중입니다. \
앱을 옮기면 심볼릭이 깨지니 Applications로 이동을 권장합니다."
                .into(),
        );
    }
    let cys_src = macos_dir.join("cys");
    let cysd_src = macos_dir.join("cysd");
    let script = build_install_script(&cys_src, &cysd_src, target_dir);
    let osascript_arg = format!(
        "do shell script {} with administrator privileges",
        sh_squote(&script)
    );
    Ok(CliInstallPlan {
        cys_src,
        cysd_src,
        target_dir: target_dir.to_string(),
        osascript_arg,
        warnings,
    })
}
```

- [x] **Step 4: 테스트 통과 확인**

Run: `cd ~/dev/cys-terminal && cargo test --bin cys-app 2>&1 | tail -25`
Expected: PASS — 위 8개 테스트 통과, 기존 테스트 무회귀. (경고 `field is never read` 등은 Task 2에서 소비되므로 일시 허용 — `#[allow(dead_code)]`를 `CliInstallPlan`에 임시 부여해도 됨.)

- [x] **Step 5: 커밋**

```bash
cd ~/dev/cys-terminal
git add src-tauri/src/main.rs
git commit -m "feat(cli-path): add guarded pure helpers for /usr/local/bin symlink install"
```

---

## Task 2: `install_cli_to_path` Tauri 커맨드 + 등록

**Files:**
- Modify: `src-tauri/src/main.rs` — 커맨드는 Task 1 헬퍼 아래에 추가, 등록은 `tauri::generate_handler![` 리스트(`main.rs:1368`~`1413`, 마지막 항목 `read_dept_catalog,` 다음 줄).
- Test: 순수 로직은 Task 1에서 검증됨. 승격 경로는 수동(부록 B 체크리스트).

- [x] **Step 1: 커맨드 구현** — Task 1 헬퍼 아래에 추가:

```rust
#[derive(serde::Serialize)]
struct InstallCliReport {
    ok: bool,
    target_dir: String,
    cys_link: String,
    cysd_link: String,
    source_cys: String,
    effective_cys: Option<String>, // which -a cys 1순위
    shadowed_by: Option<String>,   // /usr/local/bin/cys 앞을 가리는 다른 cys
    warnings: Vec<String>,
}

/// 명시 메뉴 트리거. macOS에서 cys·cysd를 /usr/local/bin에 1회 승격으로 심볼릭한다.
#[tauri::command]
fn install_cli_to_path() -> Result<InstallCliReport, String> {
    #[cfg(not(target_os = "macos"))]
    {
        return Err("이 기능은 macOS 전용입니다.".into());
    }
    #[cfg(target_os = "macos")]
    {
        let target_dir = "/usr/local/bin";
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let macos_dir = exe
            .parent()
            .ok_or("번들 디렉토리 해석 실패")?
            .to_path_buf();

        let plan = plan_cli_install(&macos_dir, target_dir)?;
        if !plan.cys_src.exists() || !plan.cysd_src.exists() {
            return Err("번들 내 cys/cysd 바이너리를 찾지 못했습니다.".into());
        }

        // osascript 1회 승격(cys·cysd 동시 → 단일 프롬프트).
        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&plan.osascript_arg)
            .output()
            .map_err(|e| format!("osascript 실행 실패: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("-128") || err.contains("User canceled") {
                return Err("설치가 취소되었습니다.".into());
            }
            return Err(format!("심볼릭 생성 실패: {}", err.trim()));
        }

        // 검증: 로그인 PATH 기준 which -a cys.
        let which = std::process::Command::new("bash")
            .arg("-lc")
            .arg("which -a cys")
            .output()
            .ok();
        let entries = which
            .as_ref()
            .map(|o| parse_which_a(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default();
        let effective_cys = entries.first().cloned();
        let target_cys = format!("{target_dir}/cys");
        let shadowed_by = match &effective_cys {
            Some(p) if *p != target_cys => Some(p.clone()),
            _ => None,
        };

        let mut warnings = plan.warnings;
        if let Some(sh) = &shadowed_by {
            warnings.push(format!(
                "PATH 선행 위치의 다른 cys가 우선합니다: {sh} \
(예: dev deploy_gate의 /opt/homebrew/bin). 새로 설치한 {target_cys}는 그 뒤에 있습니다."
            ));
        }

        Ok(InstallCliReport {
            ok: true,
            target_dir: target_dir.to_string(),
            cys_link: target_cys,
            cysd_link: format!("{target_dir}/cysd"),
            source_cys: plan.cys_src.to_string_lossy().to_string(),
            effective_cys,
            shadowed_by,
            warnings,
        })
    }
}
```

- [x] **Step 2: `generate_handler!`에 등록** — `main.rs:1412`의 `read_dept_catalog,` 다음 줄에 추가:

```rust
            read_dept_catalog,
            install_cli_to_path,
        ])
```

- [x] **Step 3: 컴파일 확인**

Run: `cd ~/dev/cys-terminal && cargo check --bin cys-app 2>&1 | tail -15`
Expected: 0 errors. (Task 1의 `#[allow(dead_code)]` 임시 부여분이 있으면 제거 — 이제 `CliInstallPlan` 필드가 소비됨.)

- [x] **Step 4: 전체 테스트 무회귀**

Run: `cd ~/dev/cys-terminal && cargo test --bin cys-app 2>&1 | tail -15`
Expected: PASS, 무회귀.

- [x] **Step 5: 커밋**

```bash
cd ~/dev/cys-terminal
git add src-tauri/src/main.rs
git commit -m "feat(cli-path): install_cli_to_path command (osascript-elevated /usr/local/bin symlink)"
```

---

## Task 3: UI 버튼 + 핸들러

**Files:**
- Modify: `ui/index.html` — `#cc-header`(38~48행 근처) 내부에 버튼 추가.
- Modify: `ui/src/main.ts` — 클릭 핸들러(파일 내 다른 `getElementById(...).addEventListener` 패턴과 동일 위치군에).

- [x] **Step 1: 버튼 추가** — `ui/index.html`의 `<div id="cc-header">` 안, `#cc-clock` 근처에:

```html
        <button id="btn-install-cli" title="외부 터미널에서 cys 명령 쓰기(1회 관리자 승인)">셸에 cys 설치</button>
```

- [x] **Step 2: 핸들러 작성** — `ui/src/main.ts`에서 `invoke`가 정의된 이후(21행 이후) 적절한 초기화 블록에 추가:

```ts
document.getElementById("btn-install-cli")?.addEventListener("click", async () => {
  try {
    const r = (await invoke("install_cli_to_path")) as {
      cys_link: string; cysd_link: string; shadowed_by: string | null; warnings: string[];
    };
    let msg = `설치 완료:\n  ${r.cys_link}\n  ${r.cysd_link}\n\n새 터미널을 열면 'cys'를 바로 쓸 수 있습니다.`;
    if (r.warnings?.length) msg += `\n\n⚠ ${r.warnings.join("\n⚠ ")}`;
    alert(msg);
  } catch (e) {
    alert(`설치 실패: ${e}`);
  }
});
```

- [x] **Step 3: UI 빌드 확인**

Run: `cd ~/dev/cys-terminal && sh ui/build.sh 2>&1 | tail -10`
Expected: 빌드 성공, `ui/dist` 갱신, TS 타입오류 0.

- [x] **Step 4: 커밋**

```bash
cd ~/dev/cys-terminal
git add ui/index.html ui/src/main.ts
git commit -m "feat(cli-path): Control Center button to install cys CLI to PATH"
```

---

## Task 4: 문서 개정 (발견성 + 경계 정합)

**Files:**
- Modify: `docs/INSTALL.md` — §B(80~87행)와 INST-DENY-02(36행) 정합.
- Modify: `README.md` — 설치 절에 1줄.

> 🛑 **아래 Step 1 의 블록을 그대로 옮겨 쓰지 마라 — 그 안의 맨 `sudo ln -sf` 두 줄은
> 2026-08-25 에 결함으로 판정돼 문서에서 제거됐다.** `ln -sf` 는 그 자리에 있는 것이 우리
> 심볼릭인지 **남의 실체 파일**인지 보지 않고 갈아 끼운다(백업 0 · 경고 0 · 복구 불가).
> 지금 `docs/INSTALL.md` §B 에 들어 있는 것은 `[ -e ] || [ -L ]` → `readlink` 번들 대조 →
> `<원래 경로>.cys-backup-<epoch초>` 백업 → `ln -sfn` 순서로 도는 **가드 블록**이며 그것이 정본이다.
> 아래 텍스트는 2026-06-29 당시의 원안 기록으로만 읽어라.

- [x] **Step 1: INSTALL.md §B 개정** — 기존 §B(80~87행)를 아래로 교체:

```markdown
### 🧑 B. CLI도 외부 터미널에서 쓰려면 (선택)
앱 번들 안의 cys·cysd를 PATH(`/usr/local/bin`)에 노출합니다. **권장: 앱 안에서 1클릭.**

1. **권장 — GUI 1클릭(1회 관리자 승인):** Control Center 헤더 → **"셸에 cys 설치"** 클릭 →
   macOS 비밀번호 1회 입력. `/usr/local/bin/cys`·`/usr/local/bin/cysd` 심볼릭이 생기고,
   새 터미널에서 `cys`가 바로 동작합니다. (앱 업데이트에도 경로 유지 — 심볼릭이라 자동 추종.)
2. **폴백 — 수동 sudo(에이전트 자율 금지):** GUI를 못 쓰는 환경에서만.
```sh
# 🧑 [HUMAN] 🚧 [BOUNDARY INST-DENY-02] sudo 심링크 — 사람이 직접
sudo ln -sf /Applications/cys.app/Contents/MacOS/cys  /usr/local/bin/cys
sudo ln -sf /Applications/cys.app/Contents/MacOS/cysd /usr/local/bin/cysd
```
(pane *안*에서는 PATH가 자동 주입되므로 이 단계는 **앱 밖 터미널**에서 `cys`를 칠 때만 필요)
```

- [x] **Step 2: INST-DENY-02 주석 정합** — `docs/INSTALL.md:36`의 INST-DENY-02 행에 다음 문장을 보강(경계 표현형 충돌 방지):

```markdown
| INST-DENY-02 | `sudo ln -sf …` (심링크 덮어쓰기) | `sudo` = 오너 권한 단계 + `-f`로 기존 파일을 묻지 않고 덮어씀. | 사람(🧑)이 직접 실행 → 워커는 위임. **단, GUI "셸에 cys 설치" 버튼은 사용자 명시 클릭 + osascript 1회 승격이라 이 경계를 위반하지 않음**(워커가 그 버튼을 자율 클릭하는 것은 여전히 금지). |
```

- [x] **Step 3: README 발견성 1줄** — `README.md` 설치 안내("드래그=끝" 부근)에 추가:

```markdown
> 외부 터미널에서 `cys` 명령을 쓰려면: 앱 Control Center → **"셸에 cys 설치"** 1클릭(1회 관리자 승인). 자세히는 docs/INSTALL.md §B.
```

- [x] **Step 4: 커밋**

```bash
cd ~/dev/cys-terminal
git add docs/INSTALL.md README.md
git commit -m "docs(cli-path): document 1-click 'Install cys command' + reconcile INST-DENY-02"
```

---

## 부록 A — 선결 전제(별개 트랙, 이 계획의 코드 작업 아님): 0.4.6/0.4.7 공증

이 메뉴 수정은 **앱이 열려야** 도달 가능하다. 실측: 현재 `dist-mac/` 공증판 최신 = **0.4.5**, 현 버전 = **0.4.7** → 0.4.6/0.4.7은 미공증(`target/.../cys_0.4.7_aarch64.dmg` = Developer ID 서명·**미공증**·spctl rejected). 신규 소비자가 받을 진짜 산출물은 `build-macos-signed.sh`가 공증·staple하는 `dist-mac/cys-<ver>-macos-arm64.dmg`이므로, **배포 전 0.4.7을 `build-macos-signed.sh`로 공증**해야 한다(메모리 `cys-terminal-apple-notarization.md` 절차). 이는 오너 게이트(서명·발행). 이 계획과 독립적으로 선행/병행.

## 부록 B — 승격 경로 수동 테스트 체크리스트(자동화 불가 부분)

osascript 관리자 프롬프트는 비대화형 자동테스트 불가. 빌드 후 수동 검증:

- [x] cys.app을 `/Applications`에 두고 정상 실행 → "셸에 cys 설치" 클릭 → 비번 1회 → 성공 알림.
- [x] 새 Terminal에서 `which -a cys` → `/usr/local/bin/cys` 출현, `cys --version`(또는 `cys list`) 동작.
- [x] `ls -l /usr/local/bin/cys` → 심볼릭이 `/Applications/cys.app/Contents/MacOS/cys` 가리킴.
- [x] 재클릭(멱등) → 오류 없이 재성공(self-heal).
- [x] 프롬프트에서 취소 → "설치가 취소되었습니다" 반환, 부분상태 없음.
- [x] (그림자화 보고) `/opt/homebrew/bin/cys`가 이미 있는 dev 머신 → 성공하되 `shadowed_by`/경고에 `/opt/homebrew/bin` 표면화, 기존 동작 무손상.
- [x] (translocation 거부) DMG 마운트 상태로 바로 실행 후 클릭 → "Applications로 옮긴 뒤 재시도" 오류(심볼릭 미생성).

---

## Self-Review (계획 작성자 자체 점검)

- **Spec 커버리지:** 오너 결정(명시 메뉴 + /usr/local/bin) → Task 2·3. 가드5종 → translocation/backup 거부(Task1 `plan_cli_install`), 그림자화 보고(Task2), 멱등(`ln -sf`), 명시 동의(Task3 버튼). 발견성 갭 → Task4 README. INST-DENY-02 충돌 → Task4 정합. 공증 선결 → 부록 A. ✅
- **Placeholder 스캔:** 모든 코드 스텝에 실제 코드·정확 경로·예상 출력 명시, TBD 없음. ✅
- **타입 일관성:** `BundleKind`(Task1) ↔ `classify_bundle_dir`/`plan_cli_install`(Task1) ↔ `install_cli_to_path`(Task2) 시그니처 일치. `CliInstallPlan{cys_src,cysd_src,target_dir,osascript_arg,warnings}` ↔ Task2 소비 일치. `InstallCliReport` ↔ UI 핸들러 필드(`cys_link,cysd_link,shadowed_by,warnings`) 일치. ✅
- **미해결(다음 라운드 오너 게이트):** ⓐ `/opt/homebrew/bin` 실파일 3채널 단일화(이 계획은 `/usr/local/bin` 신설만·기존 무손상) ⓑ 부록 A 공증 ⓒ 보조 채널(.pkg/brew) 채택 여부.
