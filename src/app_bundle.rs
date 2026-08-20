//! app_bundle — macOS 앱 번들의 **원자 교체 계약(ATOMIC-1)** 과 **완본 검증**.
//!
//! ★실사고 2026-08-01(커널 로그·휴지통 잔해로 확정): `/Applications/cys.app` 에서
//! `Contents/Info.plist` 가 사라져 번들이 성립하지 않았다. 그 결과
//! ⓐ `codesign` 은 `bundle format unrecognized, invalid, or unsuitable`,
//! ⓑ `syspolicyd` 는 `Unable to open app for protection: 2`(ENOENT) + `bundle_id: (null)`,
//! ⓒ 커널은 번들 등록 실패로 내부 `cys`·`python3` 를 **미등록 단독 실행파일**로 평가해
//!    `Security policy would not allow process` 로 **859회 하드 거부**했다(헤드리스라 승인 프롬프트 불가).
//! 사용자에게 보이는 얼굴은 "**손상되었기 때문에 열 수 없습니다**" 하나뿐이고, 원인을 알 길이 없다.
//!
//! **기제(격리 재현 확인)**: 기존 앱 번들 위에 `cp -R`·`ditto` 로 **부분 복사**를 하면, 특정 파일에서
//! `Operation not permitted`(App Management/TCC · errno 1)가 나도 **복사는 멈추지 않는다**.
//! 에러 한 줄만 스크롤에 묻히고 나머지 파일은 전부 교체돼 **세대 혼합 반쪽 번들**이 확정된다
//! (`chflags uchg` 로 동일 errno 를 모사해 `cp -R`·`ditto` 양쪽에서 재현: rc=1인데 나머지는 전부 복사됨).
//! ⇒ **`cp -R` 도 `ditto` 도 트랜잭션이 아니다.**
//!
//! 그래서 이 모듈은 번들 교체를 다음 **5단 계약**으로 못 박는다(부분 상태가 남을 수 없는 형태):
//!   ① 임시 위치(같은 파일시스템)에 **완본**을 준비한다 — 비트랜잭션 복사(ditto)는 여기서만 허용한다.
//!      실패해도 더럽혀지는 건 스크래치뿐이다.
//!   ② 그 **완본 자체를 검증**한다(Info.plist 실재 + 필수 실행물 + 선택적 `codesign --verify`).
//!   ③ 검증을 통과했을 때만 교체한다. 교체는 **디렉터리 단위 원자 연산**이다
//!      (macOS `renamex_np(RENAME_SWAP)` 1회 · 폴백은 rename 2단 + 실패 시 자동 원복).
//!   ④ 교체 후 **다시 검증**한다.
//!   ⑤ 어느 단계든 실패하면 **기존 번들을 그대로 둔 채 큰 소리로 실패**한다
//!      (무증상 성공 금지 · 부분 상태 잔존 금지 · 되돌리기 실패는 사람이 읽을 수 있는 경로와 함께 보고).
//!
//! **이 계약이 금지하는 두 안티패턴**(둘 다 실제 코드에 있었다):
//! - `rm -rf <app> && mv <new> <app>` — 삭제가 먼저다. 이동이 실패하면 **앱이 전소**한다.
//!   (tauri-plugin-updater 2.10.1 `updater.rs:1273-1277` 의 권한상승 폴백이 정확히 이 형태다.)
//! - `rename(app → 임시백업)` 후 최종 rename 실패 시 **되돌리는 코드가 없는** 경로
//!   (같은 파일 `updater.rs:1255-1302` — 백업이 `TempDir` 이라 프로세스가 죽으면 같이 사라진다).
//!
//! **우리가 못 고치는 구간**(외부 도구: Finder 드래그·Tauri 업데이터)은 교체 자체를 계약화할 수 없다.
//! 그 구간은 **설치 후 검증**(`verify`)으로 덮는다 — 깨진 채 조용히 사는 대신, 앱이 스스로 알아채고
//! 사용자에게 정확한 복구 절차를 준다. `damaged_bundle_guidance` 가 그 문구의 단일 출처다.
//!
//! **인접 구현과의 경계**(중복 아님):
//! - `cys doctor` 의 `app-seal`(src/bin/cys.rs) = **코드서명 봉인** 진단(무엇이 봉인을 깼는가).
//! - 이 모듈 = **구조 완본성**(무엇이 빠졌는가) + **교체 계약**. 봉인 검사는 옵션으로만 얹는다.
//! - `SEAL-1`(lib.rs `ENV_PY_NO_BYTECODE`) = 번들이 **스스로** 봉인을 깨지 않게 하는 예방.

use std::path::{Path, PathBuf};

/// Tauri 메인 실행물(cargo bin 이름) — `Contents/MacOS/cys-app`.
pub const MAIN_EXECUTABLE: &str = "cys-app";
/// `tauri.conf.json` `bundle.externalBin` 으로 동봉되는 사이드카 — 데몬과 CLI.
/// 이 둘이 없으면 GUI 가 떠도 아무 일도 일어나지 않는다(데몬 기동 불가 = 제품 전체 무력).
pub const SIDECAR_EXECUTABLES: [&str; 2] = ["cysd", "cys"];
/// 동봉 Python 런타임 진입점. `scripts/build-macos-signed.sh` 가 이 파일의 실행 가능 여부를
/// 빌드 게이트로 쓴다(`[ ! -x src-tauri/runtime/python/bin/python3 ]` → 런타임 재준비).
/// 팩 스크립트·훅·오케스트레이션이 전부 이 인터프리터에 매달려 있다.
pub const PYTHON_RUNTIME: &str = "Resources/runtime/python/bin/python3";
/// 오프라인 자기완결 팩(`scripts/bundle-prep.sh` 가 결정론 tar 로 동봉).
pub const PACK_TARBALL: &str = "Resources/pack.tar.gz";
/// 팩 매니페스트(임베드 권위 SOT).
pub const PACK_MANIFEST: &str = "Resources/pack-manifest.json";

/// 번들 결손의 갈래. **"무엇이 없는가"를 파일 단위로 말할 수 있어야** 사용자에게 진짜 안내가 된다
/// ("손상되었습니다" 한 줄은 안내가 아니다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleDefect {
    /// `Contents` 자체가 없다 — 번들이라 부를 수 없는 상태.
    ContentsMissing,
    /// ★2026-08-01 실사고의 최종 상태. 이것 하나만 없어도 macOS 는 번들 등록에 실패하고
    /// 내부 실행물을 미등록 단독 바이너리로 취급해 실행을 거부한다.
    InfoPlistMissing,
    /// 존재하지만 0바이트 — 반쪽 복사·중단된 쓰기의 흔적. 파싱 이전에 이미 실격이다.
    InfoPlistEmpty,
    /// `Contents/MacOS` 디렉터리 부재.
    MacOsDirMissing,
    /// 필수 실행물 부재(`Contents/MacOS/<name>`). 끊어진 심볼릭 링크도 여기로 잡힌다
    /// (stat 이 실체를 못 찾으면 "없는 것"이다 — 실행 시점에 실패할 파일을 통과시키지 않는다).
    ExecutableMissing(String),
    /// 필수 실행물이 0바이트(복사 중단).
    ExecutableEmpty(String),
    /// 필수 실행물에 실행 비트가 없다(권한 유실 복사 — `cp` 옵션 실수·아카이브 왕복에서 발생).
    ExecutableNotExecutable(String),
    /// 필수 리소스 부재(`Contents/<rel>`). 실사고의 실패 번들은 `Resources/pack.tar.gz`·
    /// `runtime/{uv,node,git}`·`runtime/python/share` 를 잃은 채 남아 있었다.
    ResourceMissing(String),
    /// `codesign --verify --strict` 실패(요약 문구 동봉). 구조는 멀쩡한데 봉인만 깨진 경우도 있다.
    SealBroken(String),
}

impl BundleDefect {
    /// 사람이 읽는 한 줄. 로그·토스트·CLI 어디에 실려도 같은 문장이어야 한다(표현형 분기 금지).
    pub fn describe(&self) -> String {
        match self {
            BundleDefect::ContentsMissing => "Contents 디렉터리가 없습니다(번들 아님)".into(),
            BundleDefect::InfoPlistMissing => {
                "Contents/Info.plist 가 없습니다 — macOS 가 앱으로 인식하지 못합니다".into()
            }
            BundleDefect::InfoPlistEmpty => "Contents/Info.plist 가 비어 있습니다(복사 중단)".into(),
            BundleDefect::MacOsDirMissing => "Contents/MacOS 디렉터리가 없습니다".into(),
            BundleDefect::ExecutableMissing(n) => format!("실행 파일 Contents/MacOS/{n} 이 없습니다"),
            BundleDefect::ExecutableEmpty(n) => {
                format!("실행 파일 Contents/MacOS/{n} 이 비어 있습니다(복사 중단)")
            }
            BundleDefect::ExecutableNotExecutable(n) => {
                format!("Contents/MacOS/{n} 에 실행 권한이 없습니다")
            }
            BundleDefect::ResourceMissing(r) => format!("필수 구성요소 Contents/{r} 이 없습니다"),
            BundleDefect::SealBroken(s) => format!("코드서명 봉인이 깨졌습니다: {s}"),
        }
    }
}

/// 결손 목록을 사람이 읽는 블록으로.
pub fn describe_defects(defects: &[BundleDefect]) -> String {
    defects
        .iter()
        .map(|d| format!("  · {}", d.describe()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 무엇을 "완본"으로 볼 것인가의 명세. **검증 기준을 호출부마다 다르게 쓰면 계약이 아니다** —
/// 기준은 여기서만 정의하고, 호출부는 목적에 맞는 생성자를 고른다.
#[derive(Debug, Clone)]
pub struct VerifySpec {
    /// `Contents/MacOS/` 아래 반드시 있어야 하는 실행물.
    pub executables: Vec<String>,
    /// `Contents/` 기준 상대 경로로 반드시 있어야 하는 리소스.
    pub resources: Vec<String>,
    /// `codesign --verify --strict` 까지 볼 것인가(macOS + 도구 존재 시에만 실제 수행).
    pub codesign: bool,
}

impl VerifySpec {
    /// **설치본 자가검증**용(기동 자기점검·doctor). codesign 은 보지 않는다 —
    /// ⓐ 매 기동 서브프로세스 비용(실측 0.47s)을 물리지 않고
    /// ⓑ 봉인 파손 진단은 `cys doctor` 의 `app-seal` 항목이 이미 전담한다(중복 판정 금지).
    /// 여기서 잡는 건 **구조 결손**(반쪽 번들)이고, 그건 stat 몇 번이면 끝난다.
    pub fn installed() -> Self {
        let mut executables = vec![MAIN_EXECUTABLE.to_string()];
        executables.extend(SIDECAR_EXECUTABLES.iter().map(|s| s.to_string()));
        Self {
            executables,
            resources: vec![
                PYTHON_RUNTIME.to_string(),
                PACK_TARBALL.to_string(),
                PACK_MANIFEST.to_string(),
            ],
            codesign: false,
        }
    }

    /// **교체 게이트**용(계약 ②·④). 설치본 기준 + 코드서명 봉인까지. 새 번들을 자리에 앉히기
    /// 직전과 직후에는 비용을 아낄 이유가 없다 — 여기서 아끼면 사용자가 대신 낸다.
    /// (실측: `codesign --verify --strict` 는 디렉터리 이름이 `.app` 로 끝나지 않아도 동작한다 —
    ///  번들 판정 근거가 `Contents/Info.plist` 이기 때문. 그래서 스테이징 디렉터리도 그대로 검증된다.)
    pub fn release() -> Self {
        Self {
            codesign: true,
            ..Self::installed()
        }
    }

    /// 구조만(테스트·미서명 개발 번들). codesign=false, 리소스 요구 없음.
    pub fn structure_only(executables: &[&str]) -> Self {
        Self {
            executables: executables.iter().map(|s| s.to_string()).collect(),
            resources: Vec::new(),
            codesign: false,
        }
    }
}

/// 실행 파일 경로에서 **레이아웃만으로** 자기 번들 루트(`…/X.app`)를 찾는다.
///
/// ★`cys.rs` 의 `detect_app_bundle` 과 **의도적으로 다른 판정**이다(중복 아님 · 역할 분담):
/// 그쪽은 `Contents/Info.plist` 실재를 번들 확증 조건으로 요구한다 — 봉인 진단(app-seal)에는
/// 옳지만, **Info.plist 유실이 곧 사고인 이 검사에는 치명적**이다. 사고 상태에서 None 을 돌려주고
/// "검사 대상 없음"으로 강등되어, 가장 파괴적인 손상이 조용히 통과한다.
/// 그래서 여기서는 `X.app/Contents/MacOS/<exe>` **레이아웃**만 본다: 지금 이 프로세스의 실행물이
/// 그 자리에 있다는 사실 자체가 "여기는 번들이어야 한다"는 충분한 근거다. 그 다음에 무엇이
/// 빠졌는지는 `verify` 가 말한다.
pub fn enclosing_bundle(exe: &Path) -> Option<PathBuf> {
    // exe = <root>/Contents/MacOS/<name> → parent 3번이 <root>.
    let macos = exe.parent()?;
    if macos.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let root = contents.parent()?;
    if root.file_name()?.to_str()?.ends_with(".app") {
        Some(root.to_path_buf())
    } else {
        None
    }
}

/// 실체가 있고 0바이트가 아닌 파일인가. 심볼릭 링크는 **따라가서** 판정한다(끊어진 링크 = 없음).
fn file_state(p: &Path) -> Option<u64> {
    let md = std::fs::metadata(p).ok()?;
    if md.is_file() {
        Some(md.len())
    } else {
        None
    }
}

/// 번들 완본 검증. **결손 목록**을 돌려준다(빈 벡터 = 완본).
/// 실패를 bool 로 접지 않는 이유: 사용자에게 "무엇이" 없는지 말해야 복구 안내가 성립한다.
pub fn verify(bundle: &Path, spec: &VerifySpec) -> Vec<BundleDefect> {
    let mut defects = Vec::new();
    let contents = bundle.join("Contents");
    if !contents.is_dir() {
        // Contents 가 없으면 이하 전부 자명하게 없다 — 결손 목록을 소음으로 채우지 않는다.
        return vec![BundleDefect::ContentsMissing];
    }
    let plist = contents.join("Info.plist");
    match file_state(&plist) {
        None => defects.push(BundleDefect::InfoPlistMissing),
        Some(0) => defects.push(BundleDefect::InfoPlistEmpty),
        Some(_) => {}
    }
    let macos = contents.join("MacOS");
    if !macos.is_dir() {
        defects.push(BundleDefect::MacOsDirMissing);
    } else {
        for name in &spec.executables {
            let p = macos.join(name);
            match file_state(&p) {
                None => defects.push(BundleDefect::ExecutableMissing(name.clone())),
                Some(0) => defects.push(BundleDefect::ExecutableEmpty(name.clone())),
                Some(_) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let mode =
                            std::fs::metadata(&p).map(|m| m.permissions().mode()).unwrap_or(0);
                        if mode & 0o111 == 0 {
                            defects.push(BundleDefect::ExecutableNotExecutable(name.clone()));
                        }
                    }
                }
            }
        }
    }
    for rel in &spec.resources {
        // 심볼릭 링크도 실체가 있으면 통과다(dedup-git-core.sh 가 만드는 링크 트리가 정상 형태).
        if !contents.join(rel).exists() {
            defects.push(BundleDefect::ResourceMissing(rel.clone()));
        }
    }
    if spec.codesign {
        if let Some(msg) = codesign_failure(bundle) {
            defects.push(BundleDefect::SealBroken(msg));
        }
    }
    defects
}

/// `codesign --verify --strict` 를 돌려 실패 요약을 돌려준다(통과·수행불가면 None).
/// **수행 불가(도구 부재·비 macOS)를 실패로 적지 않는다** — 관측 부재는 판정이 아니다.
/// 반대로 통과로도 적지 않는다: 호출부는 None 을 "봉인에 대해 아무 말도 안 함"으로 읽는다.
fn codesign_failure(bundle: &Path) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    // PATH 하이재킹 회피 — stock macOS 상주 절대경로.
    let tool = Path::new("/usr/bin/codesign");
    if !tool.exists() {
        return None;
    }
    let out = std::process::Command::new(tool)
        .args(["--verify", "--strict", "--verbose"])
        .arg(bundle)
        .output()
        .ok()?;
    if out.status.success() {
        return None;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // 진행 로그(--prepared/--validated)는 버리고 진단 문장만 남긴다. 길이는 로그 폭 보호로 자른다.
    let msg: String = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("--prepared:") && !l.starts_with("--validated:"))
        .collect::<Vec<_>>()
        .join(" | ")
        .chars()
        .take(400)
        .collect();
    Some(if msg.is_empty() {
        format!("codesign exit {}", out.status.code().unwrap_or(-1))
    } else {
        msg
    })
}

/// 사용자가 그대로 따라 할 수 있는 복구 절차(반쪽 번들 = 구조 결손 전용 SOT).
///
/// ★왜 "번들 안의 파일만 다시 채워 넣기"를 안내하지 않는가 — 두 이유 모두 실측이다:
/// ① `/Applications` 안 앱 번들에 개별 파일을 쓰는 행위는 App Management(TCC) 보호에 막힌다
///    (`cp -R`·`ditto` 둘 다 내부 쓰기에서 `Operation not permitted` 실측).
/// ② 막히지 않더라도 세대 혼합 번들이 되어 코드서명 봉인은 여전히 깨진 채다.
/// ⇒ 통하는 길은 **번들 통째 교체** 하나뿐이고, 그 교체도 '스테이징 → 검증 → 단일 이동' 순서여야 한다.
///
/// ★왜 "기존 앱을 먼저 휴지통으로"가 1급 단계인가: Finder 드래그의 '바꾸기'는 기존 번들 **위에**
/// 쓰는 경로라 보호에 막힌 파일만 남는 반쪽 번들을 다시 만들 수 있다. 사용자가 같은 사고를
/// 재생산하지 않게 하는 것이 이 문구의 목적이다.
pub const RECOVERY_STEPS: &str = "복구 절차(순서대로):\n\
     1) 실행 중인 cys 를 완전히 종료합니다.\n\
     2) 최신 설치파일(DMG)을 내려받아 엽니다 — www.cysinsight.com\n\
     3) 응용 프로그램 폴더의 **기존 cys.app 을 먼저 휴지통으로 옮깁니다**.\n\
     \u{2003}   덮어쓰기('바꾸기')는 하지 마세요 — 일부 파일만 교체돼 같은 고장이 되풀이됩니다.\n\
     4) DMG 안의 cys.app 을 응용 프로그램(Applications) 폴더로 드래그합니다.\n\
     5) 응용 프로그램 폴더의 cys.app 을 엽니다.\n\
     그래도 열리지 않으면 터미널에서 `xattr -d com.apple.quarantine /Applications/cys.app` 를 한 번 실행한 뒤 다시 여세요.\n\
     설정·대화기록(~/.cys)은 앱 번들 밖에 있어 재설치로 지워지지 않습니다.";

/// 손상 안내 문구(순수 함수 — 테스트 대상). 원인 파일까지 이름으로 말하고, 복구 절차를 붙인다.
pub fn damaged_bundle_guidance(bundle: &Path, defects: &[BundleDefect]) -> String {
    damaged_bundle_guidance_with_previous(bundle, defects, None)
}

/// 같은 안내에 **교체 직전 번들이 대피해 있는 실제 경로**를 얹는다.
///
/// ★왜 이 경로가 1급 정보인가(2026-08-01 계열 사고의 마지막 갈래): 교체 후 검증이 실패했는데
/// 되돌리기까지 실패하면, 사용자의 멀쩡하던 옛 앱은 **사라진 게 아니라** 설치 위치 옆에
/// `cys.app.previous-12345`(폴백 경로) 또는 `.cys.app.staging-12345`(스왑 경로)라는 이름으로
/// 살아 있다. 그 사실을 말해주지 않으면 사용자는 "앱이 통째로 사라졌다"고 결론짓고 — 최악의 경우
/// 되살릴 수 있는 그 번들을 잔해로 알고 지운다. **재설치보다 빠르고 확실한 복구가 눈앞에 있는데
/// 안내가 그걸 숨기는** 셈이다. 그래서 경로와 함께 명령 두 줄까지 그대로 준다.
/// (파이썬 쌍둥이 `atomic_bundle.install_atomically` 는 이미 이 경로를 예외 메시지에 넣는다 —
///  여기서 빠지면 두 집행부의 사용자 안내가 갈라진다.)
pub fn damaged_bundle_guidance_with_previous(
    bundle: &Path,
    defects: &[BundleDefect],
    previous: Option<&Path>,
) -> String {
    let rescue = match previous {
        // ★`mv <previous> <bundle>` 한 줄로 안내하지 않는다: 설치 위치에는 지금 **손상된 새 번들**이
        //   있어서 그 한 줄은 실패한다(또는 엉뚱한 하위 경로로 들어간다). 실제로 통하는 순서를 준다.
        //   어느 단계도 삭제가 아니다 — 사용자가 스스로 앱을 지우게 만드는 안내를 두지 않는다.
        Some(prev) => format!(
            "★교체 직전의 정상 번들이 아직 남아 있습니다(사라지지 않았습니다):\n\
             \u{2003}{}\n\
             가장 빠른 복구는 이것을 제자리로 되돌리는 것입니다. 터미널에서 순서대로:\n\
             \u{2003}1) mv '{}' '{}.damaged'\n\
             \u{2003}2) mv '{}' '{}'\n\
             (이 번들을 지우지 마세요. 되돌린 뒤에도 열리지 않으면 아래 재설치 절차를 따르세요.)\n\n",
            prev.display(),
            bundle.display(),
            bundle.display(),
            prev.display(),
            bundle.display()
        ),
        None => String::new(),
    };
    format!(
        "cys 설치본이 온전하지 않습니다 — 설치 도중 일부 파일만 교체된 '반쪽 설치' 상태입니다.\n\
         위치: {}\n\
         빠지거나 깨진 구성요소:\n{}\n\n\
         이 상태에서는 macOS 가 \"손상되었기 때문에 열 수 없습니다\"로 앱을 차단할 수 있고, \
         백그라운드 서비스도 정상 동작하지 않습니다. 재설치 외의 우회는 없습니다.\n\n{}{}",
        bundle.display(),
        describe_defects(defects),
        rescue,
        RECOVERY_STEPS
    )
}

// ─────────────────── 설치 후 봉인 자가진단(SEAL-DIAG · advisory) ───────────────────
//
// ★무엇을 하는 검사인가: 이 기계에 설치된 번들의 코드서명 봉인이 **이미 깨져 있는지**를
//   앱이 스스로 확인해 사용자에게 정직하게 말한다. SEAL-1(env)·SEAL-2(선컴파일)가 파손을
//   *예방*한다면, 이쪽은 그 둘을 뚫고 이미 벌어진 파손을 *발견*한다(예방과 치유는 다른 일이다).
//
// ★advisory 전용 — 어떤 판정에서도 기동을 막거나 지연시키지 않는다. 근거는
//   `bundle_integrity_guidance`(src-tauri) 주석과 동일: 봉인 파손은 '능동적으로 해로운' 상태가
//   아니라 '이 사본을 옮기면 열리지 않는' 상태다. 여기서 부트를 멈추면 오탐 1건이 정상 사용자의
//   앱을 통째로 무력화하고, 아직 멀쩡한 기능까지 뺏는다. 사고의 실제 피해는 "아무도 말해주지
//   않은 것"이었으므로 처방도 거기에 맞춘다.
//
// ★`cys doctor` 의 `app-seal` 항목과 중복이 아니라 **역할 분담**이다:
//   doctor = 사람이 물었을 때 답하는 대화형 진단(빠른 `--strict`, 파손 파일 전량 + 복구 명령).
//   이쪽    = 아무도 묻지 않아도 기동 때 한 번 스스로 보는 자가진단(느려도 되는 `--deep`,
//             문구는 일반 사용자용 한 문단). 판정 **어휘**는 아래 `parse_seal_failure` 하나를
//             양쪽이 공유해 갈리지 않게 한다(cys.rs 는 이 함수로 위임한다).

/// codesign `--verbose` 출력에서 봉인 파손의 **원인 파일**을 갈래별로 분류한다(순수 함수).
/// 반환 `(added, modified, missing, other)` — other 는 세 갈래에 안 잡힌 진단 문장(요약줄 포함).
///
/// ★`--verbose` 가 필수인 이유: 무-verbose 출력은 "a sealed resource is missing or invalid"
///   요약 한 줄뿐이라 *어떤 파일 때문인지*를 사용자에게 말할 수 없다(실측 확인).
/// ★`other` 를 파손으로 접지 않는 것이 이 분류의 핵심 계약이다 — 미서명 개발 빌드·중첩 서명
///   불만 같은 문장이 전부 여기로 떨어지고, 그것들은 "봉인이 깨졌다"와 **다른 사실**이다.
pub fn parse_seal_failure(out: &str) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut missing = Vec::new();
    let mut other = Vec::new();
    for line in out.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(p) = l.strip_prefix("file added: ") {
            added.push(p.trim().to_string());
        } else if let Some(p) = l.strip_prefix("file modified: ") {
            modified.push(p.trim().to_string());
        } else if let Some(p) = l.strip_prefix("file missing: ") {
            missing.push(p.trim().to_string());
        } else if l.starts_with("--prepared:") || l.starts_with("--validated:") {
            // verbose 진행 로그 — 진단이 아니다.
            continue;
        } else {
            other.push(l.to_string());
        }
    }
    (added, modified, missing, other)
}

/// 봉인 자가진단의 **3값** 판정. bool 로 접지 않는 이유가 곧 이 타입의 존재 이유다 —
/// "판정 불가"를 "파손"으로 접으면 미서명 개발 빌드·도구 부재가 사용자에게 오보로 나간다
/// (관측 부재는 판정이 아니다). 반대로 "통과"로 접으면 진짜 파손이 침묵한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealVerdict {
    /// 봉인 무결(codesign exit 0).
    Intact,
    /// 봉인 파손 확증 — codesign 이 원인 파일을 **이름으로** 지목했다.
    Broken {
        /// 원인 파일(added/modified/missing 합본, 번들 루트 상대경로).
        culprits: Vec<String>,
        /// 원인이 전부 `__pycache__` 인가 = 번들 안 Python 이 스스로 깬 자기유발 파손의 지문.
        self_inflicted: bool,
    },
    /// 판정 불가(도구 부재·실행 실패·번들 아님·파손 문장 미특정). **알리지 않는다.**
    Undetermined(String),
}

/// codesign 실행 결과 → 판정(순수 함수 = 회귀 핀 대상 · IO 없음).
///
/// ★`success == false` 를 곧바로 파손으로 읽지 않는 것이 이 함수의 전부다. `--deep` 은
/// 중첩 코드(동봉 python·node·git 안의 Mach-O)까지 훑으므로 "code object is not signed at
/// all" 같은 **봉인 파손이 아닌** 실패 문장을 낼 수 있다. 그런 문장은 `other` 로 떨어지고
/// 여기서 `Undetermined` 가 된다 — 그래야 미서명 개발 빌드·부분서명 사본에 "재설치하세요"를
/// 들이밀지 않는다(오보 금지 계약).
pub fn classify_seal_output(success: bool, exit_code: Option<i32>, text: &str) -> SealVerdict {
    if success {
        return SealVerdict::Intact;
    }
    let (added, modified, missing, other) = parse_seal_failure(text);
    let culprits: Vec<String> = added
        .into_iter()
        .chain(modified)
        .chain(missing)
        .collect();
    if culprits.is_empty() {
        let tail: String = other.join(" | ").chars().take(300).collect();
        return SealVerdict::Undetermined(format!(
            "codesign exit {} · 봉인 파손 문장 미특정: {}",
            exit_code.unwrap_or(-1),
            if tail.is_empty() { "(출력 없음)".into() } else { tail }
        ));
    }
    let self_inflicted = culprits.iter().all(|p| p.contains("__pycache__"));
    SealVerdict::Broken {
        culprits,
        self_inflicted,
    }
}

/// 번들 루트 접두를 떼어 사람이 읽을 수 있게 줄인다(로그 폭·개인 경로 노출 축소).
///
/// ★`codesign` 은 원인 파일을 **realpath 로** 보고한다 — 우리가 들고 있는 번들 경로가 심링크를
/// 거친 형태면(`/tmp/…` = `/private/tmp/…`, 홈이 심링크된 볼륨 등) 문자 그대로의 접두 비교는
/// 조용히 실패하고, 그러면 사용자 알림에 **전체 절대경로가 그대로 노출**된다. 그래서 원경로와
/// 정규화 경로를 **둘 다** 시도한다. (이 결함은 e2e 핀 `seal_deep_verify_detects_real_bundle_mutation`
/// 이 실제 codesign 출력으로 잡아냈다 — 순수 함수 핀만 있었으면 통과했을 자리다.)
pub fn seal_relative(bundle: &Path, p: &str) -> String {
    let mut roots = vec![bundle.to_path_buf()];
    if let Ok(real) = bundle.canonicalize() {
        if real != *bundle {
            roots.push(real);
        }
    }
    for root in &roots {
        if let Some(rel) = p.strip_prefix(root.to_string_lossy().as_ref()) {
            return rel.trim_start_matches('/').to_string();
        }
    }
    p.to_string()
}

/// `codesign --verify --deep --strict --verbose` 로 설치본 봉인을 확인한다(macOS 전용 IO).
///
/// ★`--deep` 을 쓰는 이유(doctor 의 `--strict` 단독과 의도적으로 다르다): 이 검사는 사용자를
/// 기다리게 하지 않는 **백그라운드 스레드**에서 버전당 한 번만 돌기 때문에 느려도 되고, 그
/// 대가로 중첩 코드(동봉 python/node/git)의 파손까지 본다. doctor 는 사람이 답을 기다리는
/// 대화형 경로라 빠른 최상위 판정을 고른다 — 둘 다 옳고, 판정 어휘는 위에서 공유한다.
/// `--deep` 이 늘리는 오탐 표면은 `classify_seal_output` 의 Undetermined 강등이 막는다.
///
/// 비 macOS·`/usr/bin/codesign` 부재·실행 실패는 전부 `Undetermined`(무음 skip 대상).
pub fn verify_seal_deep(bundle: &Path) -> SealVerdict {
    if !cfg!(target_os = "macos") {
        return SealVerdict::Undetermined("macOS 아님".into());
    }
    if !bundle.exists() {
        return SealVerdict::Undetermined(format!("번들 경로 소멸({})", bundle.display()));
    }
    // PATH 하이재킹 회피 — stock macOS 상주 절대경로.
    let tool = Path::new("/usr/bin/codesign");
    if !tool.exists() {
        return SealVerdict::Undetermined("/usr/bin/codesign 부재".into());
    }
    let out = match std::process::Command::new(tool)
        .args(["--verify", "--deep", "--strict", "--verbose"])
        .arg(bundle)
        .output()
    {
        Ok(o) => o,
        Err(e) => return SealVerdict::Undetermined(format!("codesign 실행 실패({e})")),
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    match classify_seal_output(out.status.success(), out.status.code(), &text) {
        SealVerdict::Broken {
            culprits,
            self_inflicted,
        } => SealVerdict::Broken {
            culprits: culprits.iter().map(|p| seal_relative(bundle, p)).collect(),
            self_inflicted,
        },
        v => v,
    }
}

/// 봉인 파손을 사용자에게 알리는 문구(순수 함수 = 회귀 핀 대상).
///
/// ★문구 계약(과장 금지): ⓐ 지금 당장 앱이 못 쓰게 된 게 **아니라는** 사실을 먼저 말한다
/// (실제로 지금 돌고 있으니까 — 겁주면 오히려 신뢰를 잃는다) ⓑ 실제 위험은 "이 사본을 옮기거나
/// 다시 받아서 열 때 macOS 가 차단할 수 있다"는 조건부라는 것을 그대로 말한다 ⓒ 해결은 재설치
/// 하나뿐임을 링크와 함께 준다. 번들 안 파일을 지우라는 '부분 수리'는 **절대 안내하지 않는다**
/// (App Management 보호에 막히고, 막히지 않아도 added 가 missing 으로 바뀔 뿐이다 — `RECOVERY_STEPS` 참조).
pub fn seal_broken_notice(bundle: &Path, culprits: &[String], self_inflicted: bool) -> String {
    let sample: Vec<&str> = culprits.iter().take(3).map(|s| s.as_str()).collect();
    let more = culprits.len().saturating_sub(sample.len());
    let sample_line = if more > 0 {
        format!("{} 외 {}건", sample.join(", "), more)
    } else {
        sample.join(", ")
    };
    let cause = if self_inflicted {
        "\n원인 파일이 전부 파이썬 캐시(__pycache__)입니다 — 앱 안의 파이썬이 실행 중에 스스로 만든 파일이며, \
         최신 버전에서는 이 현상이 차단되어 있습니다."
    } else {
        ""
    };
    format!(
        "cys 설치본의 코드서명 봉인이 깨져 있습니다(자가진단 결과).\n\
         위치: {}\n\
         봉인과 어긋난 파일 {}건: {}{}\n\n\
         지금 실행 중인 이 앱은 계속 사용할 수 있습니다. 다만 이 상태의 사본을 다른 맥으로 옮기거나 \
         내려받아 처음 열면, macOS Gatekeeper 가 \"손상되었기 때문에 열 수 없습니다\"로 차단할 수 있습니다.\n\n\
         해결 방법은 최신 버전 재설치 하나뿐입니다 — www.cysinsight.com/downloads 에서 내려받아 \
         응용 프로그램 폴더의 기존 cys.app 을 휴지통으로 옮긴 뒤 새로 설치해 주세요.\n\
         (설정·대화기록은 앱 밖 ~/.cys 에 있어 재설치로 지워지지 않습니다.)\n\
         재설치 후 이 앱을 완전히 종료하고 새로 열어 주세요 — 자가진단이 다시 돌아 정상이면 \
         이 알림은 나타나지 않습니다.",
        bundle.display(),
        culprits.len(),
        sample_line,
        cause
    )
}

// ───────────────────────── 원자 교체(ATOMIC-1) ─────────────────────────

/// 교체 실패의 갈래. **어느 갈래든 "기존 번들이 어떤 상태인지"를 타입이 말하게** 한다 —
/// 호출부가 로그만 보고 추측하게 두면 무증상 성공과 구별할 수 없다.
#[derive(Debug)]
pub enum InstallError {
    /// ① 스테이징 완본이 검증에 실패 — **기존 번들 무접촉**(가장 흔하고 가장 안전한 실패).
    StagedIncomplete {
        staged: PathBuf,
        defects: Vec<BundleDefect>,
    },
    /// ② 스테이징과 목적지가 다른 파일시스템 — rename 이 원자적이지 않다. **기존 번들 무접촉**.
    ///    ★복사 폴백을 두지 않는다: 복사는 트랜잭션이 아니고, 그 폴백이 곧 이 사고의 기제다.
    CrossFilesystem { staged: PathBuf, dest: PathBuf },
    /// ③ 교체 연산 실패 — **기존 번들 무접촉**(원자 스왑은 전부 아니면 전무 · 폴백은 즉시 원복).
    SwapFailed { dest: PathBuf, io: String },
    /// ④ 교체 후 검증 실패. `restored=true` 면 기존 번들로 되돌렸다(사용자 피해 0).
    PostVerifyFailed {
        dest: PathBuf,
        defects: Vec<BundleDefect>,
        restored: bool,
        /// 교체 직전 번들이 **실제로 놓여 있는 경로**(신규 설치라 옛 번들이 없었으면 None).
        /// ★`restored == false` 일 때 이 값이 사용자의 유일한 복구 실마리다 — 옛 앱은 사라진 게
        /// 아니라 여기 있다. 담지 않으면 "앱이 사라졌다"로 읽히고, 그 오해가 되살릴 수 있는
        /// 번들을 지우게 만든다. `restored == true` 일 때는 이미 제자리로 돌아갔으므로 안내에 쓰지 않는다.
        previous: Option<PathBuf>,
    },
    /// ★최악의 갈래 — 되돌리기까지 실패했다. 사람이 손으로 옮길 수 있게 **경로를 그대로 노출**한다.
    ///    조용히 삼키면 사용자는 앱이 사라진 이유를 영원히 알 수 없다.
    RestoreFailed {
        dest: PathBuf,
        previous: PathBuf,
        io: String,
    },
    /// 스테이징 준비(복사) 실패 — 스크래치만 더럽고 목적지는 무접촉.
    StagingFailed { staged: PathBuf, io: String },
}

impl InstallError {
    /// 기존 설치본이 **그대로 남아 있는가**. false 면 사람 개입이 필요하다.
    pub fn destination_intact(&self) -> bool {
        match self {
            InstallError::RestoreFailed { .. } => false,
            InstallError::PostVerifyFailed { restored, .. } => *restored,
            _ => true,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            InstallError::StagedIncomplete { staged, defects } => format!(
                "설치를 중단했습니다 — 새 번들이 완본이 아닙니다({}).\n{}\n기존 설치본은 그대로입니다.",
                staged.display(),
                describe_defects(defects)
            ),
            InstallError::CrossFilesystem { staged, dest } => format!(
                "설치를 중단했습니다 — 임시 위치({})와 설치 위치({})가 서로 다른 디스크라 \
                 원자적 교체가 불가능합니다. 설치 위치와 같은 디스크에 스테이징해야 합니다. \
                 기존 설치본은 그대로입니다.",
                staged.display(),
                dest.display()
            ),
            InstallError::SwapFailed { dest, io } => format!(
                "설치를 중단했습니다 — 교체 실패({io}). 기존 설치본({})은 그대로입니다.",
                dest.display()
            ),
            // ★되돌린 경우와 못 되돌린 경우는 **사용자가 할 일이 정반대**라 문장을 나눈다.
            //   되돌렸으면 할 일이 없다(짧게 사실만). 못 되돌렸으면 지금 화면의 이 문장이
            //   사용자가 받는 **유일한 복구 안내**이므로, 결손·옛 번들 실제 경로·명령까지 전부 준다.
            InstallError::PostVerifyFailed {
                dest,
                defects,
                restored: true,
                ..
            } => format!(
                "설치 후 검증에 실패했습니다({}) — 기존 상태로 되돌렸습니다.\n{}",
                dest.display(),
                describe_defects(defects)
            ),
            InstallError::PostVerifyFailed {
                dest,
                defects,
                restored: false,
                previous,
            } => format!(
                "설치 후 검증에 실패했고 **되돌리지 못했습니다**({}).\n{}",
                dest.display(),
                damaged_bundle_guidance_with_previous(dest, defects, previous.as_deref())
            ),
            InstallError::RestoreFailed {
                dest,
                previous,
                io,
            } => format!(
                "★수동 복구 필요 — 되돌리기 실패({io}).\n\
                 기존 번들은 여기 있습니다: {}\n설치 위치: {}\n\
                 Finder 에서 기존 번들을 설치 위치로 옮기거나, 최신 DMG 로 재설치하세요.",
                previous.display(),
                dest.display()
            ),
            InstallError::StagingFailed { staged, io } => format!(
                "새 번들 준비(복사) 실패({io}) — {}. 기존 설치본은 그대로입니다.",
                staged.display()
            ),
        }
    }
}

/// 교체 성공 보고. `previous` 는 **자동 삭제하지 않는다** — 비가역 삭제는 호출부의 결정이다
/// (남겨도 다음 설치가 새 스테이징 경로를 쓰므로 재발 위험이 없고, 되돌릴 여지를 남긴다).
#[derive(Debug)]
pub struct InstallReport {
    pub dest: PathBuf,
    /// 교체 전 번들이 옮겨 간 자리(신규 설치라 기존 번들이 없었으면 None).
    pub previous: Option<PathBuf>,
    /// 원자 스왑(`renamex_np(RENAME_SWAP)`) 1회로 끝났는가. false 면 rename 폴백 경로였다.
    pub atomic_swap: bool,
}

/// 목적지와 **반드시 같은 파일시스템**인 스테이징 디렉터리 경로(생성은 `stage_full_copy`).
/// 목적지의 부모 디렉터리 안에 만든다 — 같은 디렉터리면 같은 파일시스템이 자명하다.
/// 이름은 점으로 시작해 Finder 에 보이지 않게 하고, pid 로 동시 실행을 격리한다.
pub fn staging_dir_for(dest: &Path) -> PathBuf {
    let parent = dest.parent().unwrap_or_else(|| Path::new("/"));
    let name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("bundle.app");
    parent.join(format!(".{}.staging-{}", name, std::process::id()))
}

/// 두 경로가 같은 파일시스템인가(rename 원자성의 전제). 판정 불가면 보수적으로 false.
#[cfg(unix)]
fn same_filesystem(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

/// macOS 원자 스왑: 두 경로의 내용을 **한 번의 시스템 콜로 맞바꾼다**(APFS/HFS+).
/// 성공 후 `a` 에는 옛 번들이, `b` 에는 새 번들이 있다. 실패하면 **아무것도 바뀌지 않는다** —
/// 이 "전부 아니면 전무"가 rename 2단(중간에 목적지가 없는 창이 생긴다)보다 나은 유일한 이유다.
/// 대상 중 하나라도 없으면 ENOENT(신규 설치 경로에서는 이 함수를 부르지 않는다).
#[cfg(target_os = "macos")]
fn swap_paths(a: &Path, b: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let ca = std::ffi::CString::new(a.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let cb = std::ffi::CString::new(b.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    // SAFETY: 두 인자 모두 널종료 C 문자열 포인터이고, 호출 동안 살아 있다. 커널은 이 호출로
    // 우리 주소공간에 쓰지 않는다. 실패는 반환값 + errno 로만 보고된다.
    let rc = unsafe { libc::renamex_np(ca.as_ptr(), cb.as_ptr(), libc::RENAME_SWAP) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// RENAME_SWAP 은 macOS 전용(리눅스의 `RENAME_EXCHANGE` 는 별도 API) — 폴백 2단을 타게 한다.
#[cfg(all(unix, not(target_os = "macos")))]
fn swap_paths(_a: &Path, _b: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "RENAME_SWAP unsupported",
    ))
}

/// 폴백 교체(원자 스왑을 못 쓰는 파일시스템): `dest` 를 스테이징 자리로 대피시키고 새 번들을 앉힌다.
/// 성공 시 옛 번들이 있는 경로(=`staged`)를 돌려준다 — 스왑 경로와 **사후 형태가 같아져** 되돌림
/// 로직이 갈라지지 않는다.
///
/// ★대피처를 `staged`(목적지와 같은 파일시스템·같은 부모)로 잡는 이유: 임시 디렉터리에 두면
/// 프로세스가 죽는 순간 되돌릴 대상 자체가 증발한다(tauri-plugin-updater 가 밟은 함정).
#[cfg(unix)]
fn two_step_replace(staged: &Path, dest: &Path) -> Result<PathBuf, InstallError> {
    let parked = dest.with_file_name(format!(
        "{}.previous-{}",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("bundle.app"),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&parked);
    std::fs::rename(dest, &parked).map_err(|e| InstallError::SwapFailed {
        dest: dest.to_path_buf(),
        io: format!("기존 번들 대피 실패: {e}"),
    })?;
    if let Err(e) = std::fs::rename(staged, dest) {
        // 새 번들을 못 앉혔다 → 즉시 원복. 원복까지 실패하면 경로를 노출해 보고한다.
        return Err(match std::fs::rename(&parked, dest) {
            Ok(()) => InstallError::SwapFailed {
                dest: dest.to_path_buf(),
                io: format!("{e} (기존 번들 원복 완료)"),
            },
            Err(e2) => InstallError::RestoreFailed {
                dest: dest.to_path_buf(),
                previous: parked,
                io: format!("{e} → 원복 실패: {e2}"),
            },
        });
    }
    Ok(parked)
}

/// 교체를 **완전히 되돌린다**(계약 ⑤). 성공하면 설치 위치는 교체 이전과 글자 그대로 같다.
///
/// - 원자 스왑이었으면 같은 스왑을 한 번 더 한다(대칭 연산 · 1콜).
/// - 폴백이었으면 새 번들을 스테이징 자리로 물리고 대피본을 도로 앉힌다.
/// - 신규 설치였으면(되돌릴 옛 번들 없음) 새 번들을 스테이징 자리로 물린다 = 설치 전 부재 상태 복원.
///   ★설치 위치에 `*.failed-*` 같은 잔해를 남기지 않는다 — 사용자 눈에 보이는 곳에 반쪽을 두지 않는다.
#[cfg(unix)]
fn undo_replace(staged: &Path, dest: &Path, previous: Option<&Path>, atomic_swap: bool) -> bool {
    match previous {
        Some(prev) if atomic_swap => swap_paths(prev, dest).is_ok(),
        Some(prev) => {
            let _ = std::fs::remove_dir_all(staged);
            std::fs::rename(dest, staged).is_ok() && std::fs::rename(prev, dest).is_ok()
        }
        None => {
            let _ = std::fs::remove_dir_all(staged);
            std::fs::rename(dest, staged).is_ok()
        }
    }
}

/// `src` 번들을 `staged` 로 **완본 복사**한다(계약 ①). macOS 는 `ditto` 로 xattr·ACL·리소스포크·
/// 심볼릭링크 트리를 보존한다(`dedup-git-core.sh` 가 만든 링크 구조가 깨지면 서명이 깨진다).
///
/// ★비트랜잭션 복사를 여기서만 허용하는 이유: 목적지가 **빈 스크래치**다. 중간에 실패하면 반쪽짜리
/// 스크래치가 남을 뿐이고, 계약 ②의 검증이 그걸 반드시 거른다. 절대 하면 안 되는 건 이 복사를
/// **살아있는 설치본 위에** 하는 것이다(그게 2026-08-01 사고).
#[cfg(unix)]
pub fn stage_full_copy(src: &Path, staged: &Path) -> Result<(), InstallError> {
    let fail = |io: String| InstallError::StagingFailed {
        staged: staged.to_path_buf(),
        io,
    };
    if staged.exists() {
        std::fs::remove_dir_all(staged).map_err(|e| fail(format!("이전 스테이징 정리 실패: {e}")))?;
    }
    if let Some(parent) = staged.parent() {
        std::fs::create_dir_all(parent).map_err(|e| fail(e.to_string()))?;
    }
    let out = if cfg!(target_os = "macos") {
        std::process::Command::new("/usr/bin/ditto")
            .args(["--rsrc", "--extattr", "--acl"])
            .arg(src)
            .arg(staged)
            .output()
    } else {
        std::process::Command::new("cp")
            .arg("-a")
            .arg(src)
            .arg(staged)
            .output()
    };
    match out {
        Ok(o) if o.status.success() => Ok(()),
        // ★rc 를 반드시 본다: `ditto`·`cp -R` 은 EPERM 한 건에도 나머지를 계속 복사하고 rc=1 만
        //   남긴다(실측). rc 를 무시하면 반쪽 복사본이 '완본'으로 다음 단계에 넘어간다.
        Ok(o) => Err(fail(format!(
            "exit {} {}",
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stderr).trim()
        ))),
        Err(e) => Err(fail(e.to_string())),
    }
}

/// **원자 교체 계약(ATOMIC-1)의 집행부** — 계약 ②③④⑤.
///
/// 사전조건: `staged` 는 이미 준비된 **완본**이고 `dest` 와 같은 파일시스템에 있다
/// (`staging_dir_for` + `stage_full_copy` 를 쓰면 자동으로 만족한다).
///
/// 사후조건(불변식): 이 함수가 어떻게 끝나든 `dest` 는 **검증을 통과한 새 번들**이거나
/// **손대지 않은 기존 번들**(신규 설치였으면 부재)이다. 반쪽 상태는 만들어지지 않는다.
/// 유일한 예외는 `RestoreFailed`·`PostVerifyFailed{restored:false}` 이고, 그때조차 옛 번들의
/// 실제 경로와 결손 목록을 오류에 담아 사람이 손으로 되돌릴 수 있게 한다(조용한 소실 금지).
#[cfg(unix)]
pub fn install_bundle_atomically(
    staged: &Path,
    dest: &Path,
    spec: &VerifySpec,
) -> Result<InstallReport, InstallError> {
    install_bundle_atomically_verifying(staged, dest, spec, spec)
}

/// 같은 집행부이되 **④(교체 후 재검증)의 기준을 ②와 다르게 좁힐 수 있는** 형태.
/// `install_bundle_atomically` 는 둘을 같은 spec 으로 부르는 얇은 위임이다(종전 동작 불변).
///
/// ★왜 이 구멍이 필요한가 — ②와 ④는 같은 이름의 검사지만 **대상의 성질이 다르다**:
/// - ② 는 아직 아무도 모르는 숨김 스크래치를 본다 → 우리가 만든 그대로다. 봉인까지 봐야 한다.
/// - ④ 는 **살아 있는 설치 위치**를 본다 → 스왑이 끝난 그 순간부터 제3자(구 데몬·CLI·훅)가 합법적으로
///   쓸 수 있다. 실측된 대표 사례가 동봉 python 의 `.pyc` 생성이고(SEAL-1 의 존재 이유),
///   그 파일 추가 한 건이 `codesign --verify` 를 깨뜨려 **성공한 교체를 자동 원복시킨다**
///   (실제 0.14.9→0.14.10 업그레이드 경로의 결함 · 파이썬 쌍둥이 `deploy_gate.swap_bundle_into_place`
///   주석에 재현 조건 기록). 스왑은 내용을 한 바이트도 바꾸지 않으므로, ④의 봉인 검사는 ②의
///   재확인이 아니라 **경합 조건 관측**이다.
/// 그래서 호출부는 ④를 `VerifySpec::installed()`(구조 완본성) 같은 좁은 기준으로 줄 수 있다.
/// 봉인 게이트는 ②(스왑 직전·같은 비트)에 그대로 남으므로 계약이 약해지지 않는다.
#[cfg(unix)]
pub fn install_bundle_atomically_verifying(
    staged: &Path,
    dest: &Path,
    spec: &VerifySpec,
    post_spec: &VerifySpec,
) -> Result<InstallReport, InstallError> {
    // ② 완본 검증 — 통과하지 못하면 목적지를 **건드리지도 않는다**.
    let defects = verify(staged, spec);
    if !defects.is_empty() {
        return Err(InstallError::StagedIncomplete {
            staged: staged.to_path_buf(),
            defects,
        });
    }
    // 파일시스템 동일성: rename 계열 원자성의 전제. 다르면 복사 폴백 대신 **거절**한다.
    let dest_parent = dest.parent().unwrap_or_else(|| Path::new("/"));
    let staged_parent = staged.parent().unwrap_or_else(|| Path::new("/"));
    if !same_filesystem(staged_parent, dest_parent) {
        return Err(InstallError::CrossFilesystem {
            staged: staged.to_path_buf(),
            dest: dest.to_path_buf(),
        });
    }

    // ③ 교체.
    let dest_exists = dest.symlink_metadata().is_ok();
    let mut previous: Option<PathBuf> = None;
    let mut atomic_swap = false;
    if !dest_exists {
        // 신규 설치 — 옮길 옛 번들이 없다. 단일 rename 이 곧 원자 교체다.
        std::fs::rename(staged, dest).map_err(|e| InstallError::SwapFailed {
            dest: dest.to_path_buf(),
            io: e.to_string(),
        })?;
    } else {
        match swap_paths(staged, dest) {
            Ok(()) => {
                // 스왑 후 staged 경로에 **옛 번들**이 들어 있다.
                atomic_swap = true;
                previous = Some(staged.to_path_buf());
            }
            Err(_) => previous = Some(two_step_replace(staged, dest)?),
        }
    }

    // ④ 교체 후 재검증 — 실패하면 ⑤에 따라 완전히 되돌린다(기준은 post_spec — 함수 주석 참조).
    let post = verify(dest, post_spec);
    if !post.is_empty() {
        let restored = undo_replace(staged, dest, previous.as_deref(), atomic_swap);
        return Err(InstallError::PostVerifyFailed {
            dest: dest.to_path_buf(),
            defects: post,
            restored,
            // ★되돌리기에 실패했을 때 사용자의 유일한 실마리 — 옛 번들이 **실제로 있는 곳**.
            //   되돌렸다면 옛 번들은 제자리(dest)에 있으므로 안내에 쓰지 않는다(None 으로 접는다).
            previous: if restored { None } else { previous },
        });
    }
    Ok(InstallReport {
        dest: dest.to_path_buf(),
        previous,
        atomic_swap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★SEAL-DIAG 회귀 핀 ①: 판정이 **3값**으로 갈리는가. 이 핀이 지키는 것은 "판정 불가를
    /// 파손으로 오보하지 않는다"는 계약 하나다 — 그게 무너지면 미서명 개발 빌드와 중첩 서명
    /// 불만(=`--deep` 이 새로 여는 오탐 표면)이 전부 사용자에게 "재설치하세요"로 나간다.
    #[test]
    fn seal_verdict_never_reports_undetermined_as_broken() {
        // ⓐ 통과.
        assert_eq!(classify_seal_output(true, Some(0), ""), SealVerdict::Intact);

        // ⓑ 실제 사고 출력(2026-08-01) — 원인 파일이 전부 __pycache__ = 자기유발 지문.
        let real = "/Applications/cys.app: a sealed resource is missing or invalid\n\
                    file added: /Applications/cys.app/Contents/Resources/runtime/python/lib/python3.12/__pycache__/_compression.cpython-312.pyc\n\
                    file modified: /Applications/cys.app/Contents/Resources/runtime/python/lib/python3.12/encodings/__pycache__/utf_8.cpython-312.pyc\n\
                    --prepared:/Applications/cys.app/Contents/MacOS/cys\n";
        match classify_seal_output(false, Some(1), real) {
            SealVerdict::Broken {
                culprits,
                self_inflicted,
            } => {
                assert_eq!(culprits.len(), 2, "added+modified 둘 다 원인으로 셈해야 한다");
                assert!(self_inflicted, "전부 __pycache__ = 번들 안 python 자기유발");
                assert!(
                    !culprits.iter().any(|c| c.starts_with("--prepared")),
                    "verbose 진행 로그는 진단이 아니다"
                );
            }
            v => panic!("실사고 출력은 파손으로 확증돼야 한다: {v:?}"),
        }

        // ⓒ ★`--deep` 이 새로 여는 오탐 표면: 중첩 코드 미서명은 **봉인 파손이 아니다**.
        //    doctor 의 `--strict` 단독에는 없던 갈래라 이 핀이 SEAL-DIAG 전용 방어다.
        let nested = "/Applications/cys.app/Contents/Resources/runtime/node/bin/node: \
                      code object is not signed at all\nIn subcomponent: /Applications/cys.app/…\n";
        assert!(
            matches!(
                classify_seal_output(false, Some(1), nested),
                SealVerdict::Undetermined(_)
            ),
            "중첩 서명 불만을 파손으로 접으면 정상 사용자에게 오보가 나간다"
        );

        // ⓓ 미서명 개발 빌드(가장 흔한 오탐 후보)도 판정 불가다.
        assert!(matches!(
            classify_seal_output(false, Some(1), "test.app: code object is not signed at all\n"),
            SealVerdict::Undetermined(_)
        ));
    }

    /// ★SEAL-DIAG 회귀 핀 ②: 사용자 문구가 **정직**한가(과장 금지·부분 수리 금지).
    /// 문구는 이 기능의 유일한 산출물이라 내용 계약을 코드로 잠근다.
    #[test]
    fn seal_broken_notice_is_honest_and_never_suggests_partial_repair() {
        let bundle = Path::new("/Applications/cys.app");
        let culprits = vec![
            "Contents/Resources/runtime/python/lib/python3.12/__pycache__/a.pyc".to_string(),
            "Contents/Resources/runtime/python/lib/python3.12/__pycache__/b.pyc".to_string(),
            "Contents/Resources/runtime/python/lib/python3.12/__pycache__/c.pyc".to_string(),
            "Contents/Resources/runtime/python/lib/python3.12/__pycache__/d.pyc".to_string(),
        ];
        let msg = seal_broken_notice(bundle, &culprits, true);
        assert!(msg.contains("www.cysinsight.com/downloads"), "재설치 경로를 줘야 한다");
        assert!(msg.contains("계속 사용할 수 있습니다"), "지금 못 쓴다고 겁주지 않는다");
        assert!(msg.contains("차단할 수 있습니다"), "위험은 조건부임을 그대로 말한다");
        assert!(msg.contains("외 1건"), "원인 파일은 샘플 3건 + 나머지 개수로 줄인다");
        // [g] 재시작 안내 — 재설치 후에도 구 프로세스가 살아 있으면 알림이 남아 "고쳐지지
        // 않았다"로 오독된다. 완전 종료→재실행 시 자가진단이 다시 돌아 스스로 해소됨을 말한다.
        assert!(
            msg.contains("완전히 종료하고 새로 열어 주세요"),
            "재설치 후 재시작 안내가 있어야 한다(알림 잔존 오독 방지)"
        );
        // ★번들 안 파일을 지우라는 안내는 절대 나가면 안 된다(App Management 에 막히고,
        //   막히지 않아도 added 가 missing 으로 바뀔 뿐 봉인은 그대로 깨진 채다).
        for forbidden in ["rm -rf", "__pycache__ 를 삭제", "codesign --force"] {
            assert!(!msg.contains(forbidden), "부분 수리 안내 금지: {forbidden}");
        }
        // 자기유발이 아니면 python 원인 문장이 붙지 않는다(없는 사실을 말하지 않는다).
        let other = seal_broken_notice(bundle, &["Contents/Resources/x".into()], false);
        assert!(!other.contains("__pycache__"), "지문이 없으면 원인을 단정하지 않는다");
    }

    /// ★SEAL-DIAG 회귀 핀 ③(e2e · 실 codesign): 위 두 핀은 **문자열을 가정**한다 — 그 가정이
    /// 실제 `codesign --deep` 출력과 어긋나면 진단이 통째로 무음이 된다(가장 위험한 고장 모양).
    /// 그래서 진짜로 서명하고, 실사고와 같은 모양으로 `__pycache__` 파일 하나를 번들 안에 심어,
    /// `verify_seal_deep` 이 파손을 잡아내는지 본다. 서명 불가 환경은 **거짓 실패를 만들지 않고** skip
    /// (측정 불능은 실패가 아니다 — 그 갈래가 곧 `Undetermined` 의 존재 이유이기도 하다).
    #[test]
    fn seal_deep_verify_detects_real_bundle_mutation() {
        if !cfg!(target_os = "macos") || !Path::new("/usr/bin/codesign").exists() {
            return;
        }
        let root = tmp_root("sealdeep");
        let app = root.join("Probe.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>\
             <key>CFBundleExecutable</key><string>probe</string>\
             <key>CFBundleIdentifier</key><string>com.cys.probe</string></dict></plist>",
        )
        .unwrap();
        // 주 실행파일은 실제 Mach-O 여야 ad-hoc 서명이 성립한다.
        std::fs::copy("/bin/echo", app.join("Contents/MacOS/probe")).unwrap();
        let signed = std::process::Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(&app)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !signed {
            let _ = std::fs::remove_dir_all(&root);
            return; // 서명 불가 환경 — 판정 불가
        }
        assert_eq!(
            verify_seal_deep(&app),
            SealVerdict::Intact,
            "갓 서명한 번들은 --deep 에서도 무결이어야 한다(오탐 0 확인)"
        );

        let pyc = app.join("Contents/Resources/runtime/python/lib/python3.12/__pycache__");
        std::fs::create_dir_all(&pyc).unwrap();
        std::fs::write(pyc.join("_compression.cpython-312.pyc"), b"x").unwrap();
        match verify_seal_deep(&app) {
            SealVerdict::Broken {
                culprits,
                self_inflicted,
            } => {
                assert!(self_inflicted, "전부 __pycache__ = 자기유발 지문");
                // 번들 루트 접두가 떨어져 사용자에게 읽히는 형태여야 한다(개인 경로 노출 축소).
                assert!(
                    culprits.iter().all(|c| c.starts_with("Contents/")),
                    "번들 상대경로로 줄여야 한다: {culprits:?}"
                );
                let msg = seal_broken_notice(&app, &culprits, self_inflicted);
                assert!(msg.contains("www.cysinsight.com/downloads"));
            }
            v => panic!("파일 1개 추가로 봉인이 깨져야 한다: {v:?}"),
        }
        // 진단은 읽기 전용 — 변이 파일을 지우지 않는다(부분 수리 금지).
        assert!(pyc.join("_compression.cpython-312.pyc").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn tmp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("cys-atomic-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// 최소 완본 번들 픽스처(실행 비트 포함). `marker` 로 세대를 구분한다.
    fn make_bundle(root: &Path, name: &str, execs: &[&str], marker: &str) -> PathBuf {
        let app = root.join(name);
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            format!("<plist><dict><key>Marker</key><string>{marker}</string></dict></plist>"),
        )
        .unwrap();
        for e in execs {
            let p = app.join("Contents/MacOS").join(e);
            std::fs::write(&p, format!("#!/bin/sh\necho {marker}\n")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        app
    }

    fn marker_of(app: &Path) -> String {
        std::fs::read_to_string(app.join("Contents/Info.plist")).unwrap_or_default()
    }

    /// 설치 위치(부모 디렉터리)에 남은 우리 잔해 목록 — "부분 상태 잔존 금지"의 기계 판정.
    fn residue(root: &Path, dest_name: &str) -> Vec<String> {
        std::fs::read_dir(root)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(dest_name) && n != dest_name)
            .collect()
    }

    #[test]
    fn enclosing_bundle_survives_missing_info_plist() {
        // ★회귀 핀: 이것이 2026-08-01 사고의 정확한 상태다. Info.plist 를 번들 확증 조건으로
        // 요구하면 여기서 None 이 되어 "검사 대상 없음"으로 조용히 통과한다 — 그 실수를 못 하게 고정한다.
        let root = tmp_root("locate");
        let app = make_bundle(&root, "cys.app", &["cys-app"], "v1");
        std::fs::remove_file(app.join("Contents/Info.plist")).unwrap();
        let exe = app.join("Contents/MacOS/cys-app");
        assert_eq!(
            enclosing_bundle(&exe).as_deref(),
            Some(app.as_path()),
            "Info.plist 가 없어도 레이아웃만으로 번들을 지목해야 한다"
        );
        // 번들 밖(개발 빌드 target/release/cys) → None
        let plain = root.join("target/release/cys");
        std::fs::create_dir_all(plain.parent().unwrap()).unwrap();
        std::fs::write(&plain, b"x").unwrap();
        assert!(enclosing_bundle(&plain).is_none(), "번들 레이아웃 밖은 None");
        // .app 이 아닌 이름 → None(Contents/MacOS 만 흉내낸 디렉터리에 속지 않는다)
        let notapp = root.join("plain/Contents/MacOS/x");
        std::fs::create_dir_all(notapp.parent().unwrap()).unwrap();
        std::fs::write(&notapp, b"x").unwrap();
        assert!(enclosing_bundle(&notapp).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn verify_names_every_missing_part() {
        let root = tmp_root("verify");
        let spec = VerifySpec::structure_only(&["cys-app", "cysd", "cys"]);
        let app = make_bundle(&root, "ok.app", &["cys-app", "cysd", "cys"], "v1");
        assert!(verify(&app, &spec).is_empty(), "완본은 결손 0건");

        // 사고 재현: Info.plist 만 사라진 번들.
        let broken = make_bundle(&root, "broken.app", &["cys-app", "cysd", "cys"], "v1");
        std::fs::remove_file(broken.join("Contents/Info.plist")).unwrap();
        assert_eq!(verify(&broken, &spec), vec![BundleDefect::InfoPlistMissing]);

        // 사이드카 유실·0바이트·끊어진 링크가 각각 잡혀야 안내가 정확해진다.
        let partial = make_bundle(&root, "partial.app", &["cys-app", "cysd", "cys"], "v1");
        std::fs::remove_file(partial.join("Contents/MacOS/cysd")).unwrap();
        std::fs::write(partial.join("Contents/MacOS/cys"), b"").unwrap();
        let d = verify(&partial, &spec);
        assert!(d.contains(&BundleDefect::ExecutableMissing("cysd".into())), "{d:?}");
        assert!(d.contains(&BundleDefect::ExecutableEmpty("cys".into())), "{d:?}");

        #[cfg(unix)]
        {
            // 끊어진 심볼릭 링크 = 없는 것(실행 시점에 실패할 파일을 통과시키지 않는다).
            let dangling = make_bundle(&root, "dangling.app", &["cys-app"], "v1");
            std::fs::remove_file(dangling.join("Contents/MacOS/cys-app")).unwrap();
            std::os::unix::fs::symlink("/nonexistent/x", dangling.join("Contents/MacOS/cys-app"))
                .unwrap();
            assert_eq!(
                verify(&dangling, &VerifySpec::structure_only(&["cys-app"])),
                vec![BundleDefect::ExecutableMissing("cys-app".into())]
            );
            // 실행 비트 유실도 별도 결손(아카이브 왕복·cp 옵션 실수).
            use std::os::unix::fs::PermissionsExt;
            let nox = make_bundle(&root, "nox.app", &["cys-app"], "v1");
            std::fs::set_permissions(
                nox.join("Contents/MacOS/cys-app"),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
            assert_eq!(
                verify(&nox, &VerifySpec::structure_only(&["cys-app"])),
                vec![BundleDefect::ExecutableNotExecutable("cys-app".into())]
            );
        }

        // 리소스 결손(실패 번들이 잃었던 pack.tar.gz 계열)도 이름으로 말해야 한다.
        let res_spec = VerifySpec {
            executables: vec!["cys-app".into()],
            resources: vec![PACK_TARBALL.into()],
            codesign: false,
        };
        assert_eq!(
            verify(&app, &res_spec),
            vec![BundleDefect::ResourceMissing(PACK_TARBALL.into())]
        );

        // Contents 자체가 없으면 한 줄로 끝낸다(결손 목록 소음 방지).
        let empty = root.join("empty.app");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(verify(&empty, &spec), vec![BundleDefect::ContentsMissing]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn installed_spec_pins_the_parts_a_release_bundle_must_have() {
        // 규약 고정: 이 목록이 흔들리면 '완본'의 정의가 흔들린다(빌드 산출물과 동기).
        let s = VerifySpec::installed();
        assert!(s.executables.contains(&"cys-app".to_string()), "Tauri 메인");
        assert!(s.executables.contains(&"cysd".to_string()), "데몬 사이드카");
        assert!(s.executables.contains(&"cys".to_string()), "CLI 사이드카");
        assert!(s.resources.contains(&PYTHON_RUNTIME.to_string()), "동봉 python");
        assert!(!s.codesign, "기동 자기점검은 codesign 을 돌리지 않는다(doctor app-seal 담당)");
        assert!(VerifySpec::release().codesign, "교체 게이트는 봉인까지 본다");
    }

    #[test]
    fn guidance_names_cause_and_never_suggests_partial_repair() {
        let g = damaged_bundle_guidance(
            Path::new("/Applications/cys.app"),
            &[
                BundleDefect::InfoPlistMissing,
                BundleDefect::ExecutableMissing("cysd".into()),
            ],
        );
        assert!(g.contains("Info.plist"), "원인 파일을 이름으로 말해야 한다");
        assert!(g.contains("cysd"));
        assert!(g.contains("휴지통"), "기존 번들 제거 단계가 있어야 한다");
        assert!(g.contains("덮어쓰기"), "덮어쓰기 금지 경고가 있어야 한다");
        assert!(!g.contains("codesign --force"), "사용자에게 서명 조작을 시키지 않는다");
        assert!(g.contains("~/.cys"), "데이터가 지워지지 않는다는 안심 문구");
    }

    // ─── ATOMIC-1 계약 ───

    /// ①정상 교체 성공 — 목적지는 새 세대, 옛 번들은 온전히 보존, 설치 위치에 잔해 0.
    #[cfg(unix)]
    #[test]
    fn atomic_install_replaces_bundle_and_preserves_the_old_one() {
        let root = tmp_root("swap-ok");
        let execs = ["cys-app", "cysd", "cys"];
        let dest = make_bundle(&root, "cys.app", &execs, "OLD");
        let staged = staging_dir_for(&dest);
        let new = make_bundle(&root, "new-src.app", &execs, "NEW");
        stage_full_copy(&new, &staged).expect("스테이징 복사");

        let spec = VerifySpec::structure_only(&execs);
        let rep = install_bundle_atomically(&staged, &dest, &spec).expect("교체 성공");
        assert!(marker_of(&dest).contains("NEW"), "목적지는 새 세대여야 한다");
        assert!(verify(&dest, &spec).is_empty(), "교체 후 완본");
        let prev = rep.previous.expect("옛 번들 경로가 보고돼야 한다");
        assert!(marker_of(&prev).contains("OLD"), "옛 번들이 온전히 보존돼야 한다");
        assert!(residue(&root, "cys.app").is_empty(), "설치 위치에 잔해 0");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ②완본 검증 실패 → 기존 번들 보존(목적지를 **건드리지도 않는다**).
    #[cfg(unix)]
    #[test]
    fn atomic_install_keeps_old_bundle_when_new_one_is_incomplete() {
        let root = tmp_root("swap-reject");
        let execs = ["cys-app", "cysd", "cys"];
        let dest = make_bundle(&root, "cys.app", &execs, "OLD");
        let staged = staging_dir_for(&dest);
        let new = make_bundle(&root, "new-src.app", &execs, "NEW");
        stage_full_copy(&new, &staged).unwrap();
        // 사고 재현: 스테이징 완본에서 Info.plist 가 빠졌다.
        std::fs::remove_file(staged.join("Contents/Info.plist")).unwrap();

        let spec = VerifySpec::structure_only(&execs);
        let err = install_bundle_atomically(&staged, &dest, &spec).unwrap_err();
        match &err {
            InstallError::StagedIncomplete { defects, .. } => {
                assert!(defects.contains(&BundleDefect::InfoPlistMissing), "{defects:?}")
            }
            other => panic!("StagedIncomplete 여야 한다: {other:?}"),
        }
        assert!(err.destination_intact());
        assert!(marker_of(&dest).contains("OLD"), "기존 번들이 그대로여야 한다");
        assert!(verify(&dest, &spec).is_empty(), "기존 번들은 여전히 완본");
        assert!(residue(&root, "cys.app").is_empty(), "설치 위치에 잔해 0");
        // 사용자에게 나가는 문구가 '무증상'이 아니어야 한다.
        assert!(err.describe().contains("기존 설치본은 그대로"), "{}", err.describe());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ③교체 도중 실패 → 반쪽 상태 미잔존.
    /// 설치 위치의 부모 디렉터리를 쓰기 불가로 만들어 **실제 EACCES** 로 교체를 실패시킨다
    /// (스왑도 rename 도 부모 디렉터리 쓰기 권한을 요구한다 — 모킹이 아니라 진짜 실패다).
    #[cfg(unix)]
    #[test]
    fn atomic_install_leaves_no_half_bundle_when_replace_fails() {
        use std::os::unix::fs::PermissionsExt;
        let root = tmp_root("swap-eacces");
        let execs = ["cys-app", "cysd", "cys"];
        let install_dir = root.join("Applications");
        std::fs::create_dir_all(&install_dir).unwrap();
        let dest = make_bundle(&install_dir, "cys.app", &execs, "OLD");
        let staged = staging_dir_for(&dest);
        let new = make_bundle(&root, "new-src.app", &execs, "NEW");
        stage_full_copy(&new, &staged).unwrap();
        let spec = VerifySpec::structure_only(&execs);
        assert!(verify(&staged, &spec).is_empty(), "스테이징은 완본(②는 통과한다)");

        // 부모 디렉터리 쓰기 금지 → rename/renamex_np 가 EACCES 로 실패한다.
        std::fs::set_permissions(&install_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let err = install_bundle_atomically(&staged, &dest, &spec).unwrap_err();
        std::fs::set_permissions(&install_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        match &err {
            InstallError::SwapFailed { .. } => {}
            other => panic!("SwapFailed 여야 한다: {other:?}"),
        }
        assert!(err.destination_intact(), "교체 실패는 기존 번들을 건드리지 않는다");
        assert!(marker_of(&dest).contains("OLD"), "기존 번들이 옛 세대 그대로여야 한다");
        assert!(verify(&dest, &spec).is_empty(), "반쪽이 아니라 완본이어야 한다");
        assert!(
            residue(&install_dir, "cys.app").is_empty(),
            "설치 위치에 .previous-*/.failed-* 잔해가 남으면 안 된다: {:?}",
            residue(&install_dir, "cys.app")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ③-b 폴백(rename 2단) 경로의 실패·자동 원복. 원자 스왑을 못 쓰는 파일시스템에서 도는 코드라
    /// 단위로 직접 친다: 새 번들이 사라진 상태로 부르면 대피 → 앉히기 실패 → **원복**이 돌아야 한다.
    #[cfg(unix)]
    #[test]
    fn two_step_fallback_restores_old_bundle_when_it_cannot_seat_the_new_one() {
        let root = tmp_root("two-step");
        let execs = ["cys-app"];
        let dest = make_bundle(&root, "cys.app", &execs, "OLD");
        let ghost = root.join(".cys.app.staging-ghost"); // 존재하지 않는 스테이징
        let err = two_step_replace(&ghost, &dest).unwrap_err();
        match &err {
            InstallError::SwapFailed { io, .. } => {
                assert!(io.contains("원복 완료"), "원복 사실이 보고돼야 한다: {io}")
            }
            other => panic!("SwapFailed(원복 완료) 여야 한다: {other:?}"),
        }
        assert!(err.destination_intact());
        assert!(marker_of(&dest).contains("OLD"), "옛 번들이 제자리로 돌아와야 한다");
        assert!(residue(&root, "cys.app").is_empty(), "대피본 잔해가 남으면 안 된다");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ④교체 후 검증 실패 → 완전 원복. 스테이징 안을 **절대 경로 심볼릭 링크**로 만들어,
    /// 교체 전에는 실체가 있고(②통과) 교체 후에는 끊어지도록(④실패) 한다 — 스왑이 끝나면
    /// 그 절대 경로에는 옛 번들이 들어와 있어 링크 대상이 사라지기 때문이다.
    /// (모킹 없이 '교체 후에만 결손'을 만드는 유일한 결정론 수단)
    #[cfg(unix)]
    #[test]
    fn atomic_install_rolls_back_when_post_swap_verification_fails() {
        let root = tmp_root("post-verify");
        let execs = ["cys-app", "cysd"];
        let dest = make_bundle(&root, "cys.app", &execs, "OLD");
        let staged = staging_dir_for(&dest);
        let new = make_bundle(&root, "new-src.app", &execs, "NEW");
        stage_full_copy(&new, &staged).unwrap();
        // cysd 를 '스테이징 경로 안의 실제 파일'을 가리키는 절대 링크로 바꾼다.
        let real = staged.join("Contents/Resources/real-cysd");
        std::fs::write(&real, b"#!/bin/sh\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let link = staged.join("Contents/MacOS/cysd");
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let spec = VerifySpec::structure_only(&execs);
        assert!(verify(&staged, &spec).is_empty(), "교체 전에는 완본으로 보인다(②통과)");

        let err = install_bundle_atomically(&staged, &dest, &spec).unwrap_err();
        match &err {
            InstallError::PostVerifyFailed { defects, restored, .. } => {
                assert!(*restored, "되돌렸어야 한다");
                assert!(
                    defects.contains(&BundleDefect::ExecutableMissing("cysd".into())),
                    "{defects:?}"
                );
            }
            other => panic!("PostVerifyFailed 여야 한다: {other:?}"),
        }
        assert!(err.destination_intact(), "되돌렸으면 목적지는 온전하다");
        assert!(marker_of(&dest).contains("OLD"), "옛 번들이 제자리로 돌아와야 한다");
        assert!(verify(&dest, &spec).is_empty(), "돌아온 번들은 완본");
        assert!(residue(&root, "cys.app").is_empty(), "설치 위치에 잔해 0");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ★MEDIUM-2 회귀 핀: **되돌리지 못한 갈래**의 안내가 옛 번들의 실제 경로를 담아야 한다.
    /// 이 갈래에서 사용자가 받는 문장은 이것 하나뿐이고, 옛 앱은 사라진 게 아니라 옆자리에 있다.
    /// 경로가 빠지면 사용자는 "앱이 사라졌다"고 읽고 되살릴 수 있는 번들을 잔해로 알고 지운다.
    /// (되돌리기 실패 자체는 스왑의 대칭 연산이라 모킹 없이 만들 수 없다 — 그래서 사용자에게
    ///  나가는 **문구 계약**을 값으로 직접 고정한다. `damaged_bundle_guidance` 를 순수 함수로
    ///  테스트하는 이 파일의 기존 관례와 같은 층위다.)
    #[test]
    fn unrecoverable_post_verify_error_hands_the_user_their_old_bundle_path() {
        let dest = PathBuf::from("/Applications/cys.app");
        let parked = PathBuf::from("/Applications/cys.app.previous-4242");
        let err = InstallError::PostVerifyFailed {
            dest: dest.clone(),
            defects: vec![BundleDefect::InfoPlistMissing],
            restored: false,
            previous: Some(parked.clone()),
        };
        assert!(!err.destination_intact(), "되돌리지 못했으면 목적지는 온전하지 않다");
        let m = err.describe();
        assert!(
            m.contains("/Applications/cys.app.previous-4242"),
            "옛 번들의 실제 경로가 없으면 사용자는 자기 앱을 찾을 수 없다: {m}"
        );
        assert!(m.contains("Info.plist"), "무엇이 빠졌는지도 함께 말해야 한다: {m}");
        // 통하는 복구 순서여야 한다: 설치 위치에는 손상된 새 번들이 있으므로 mv 한 줄로는 실패한다.
        assert!(m.contains("'/Applications/cys.app' '/Applications/cys.app.damaged'"), "{m}");
        assert!(m.contains("'/Applications/cys.app.previous-4242' '/Applications/cys.app'"), "{m}");
        assert!(m.contains("지우지 마세요"), "되살릴 수 있는 번들을 버리게 하면 안 된다: {m}");
        assert!(m.contains("휴지통"), "그래도 안 되면 쓸 재설치 절차는 남아 있어야 한다");

        // 되돌린 갈래는 반대다 — 사용자가 할 일이 없으므로 대피 경로를 노출하지 않는다(소음 금지).
        let ok = InstallError::PostVerifyFailed {
            dest,
            defects: vec![BundleDefect::InfoPlistMissing],
            restored: true,
            previous: Some(parked),
        };
        assert!(ok.destination_intact());
        assert!(ok.describe().contains("되돌렸습니다"));
        assert!(
            !ok.describe().contains("previous-4242"),
            "되돌렸는데 대피 경로를 안내하면 사용자가 옛 번들을 다시 앉히려 한다"
        );
    }

    /// ★HIGH-2 수리의 뼈대: ④(교체 후 재검증)의 기준을 ②와 **다르게 좁힐 수 있어야** 한다.
    /// 실제 동기는 봉인(codesign)이지만(스왑 직후 제3자가 `.pyc` 를 쓰면 봉인이 깨진다), 여기서는
    /// 모킹 없이 결정론으로 같은 형태를 만든다: `cysd` 를 **스테이징 경로 안을 가리키는 절대 링크**로
    /// 두면 교체 전에는 실체가 있고(②통과) 교체 후에는 끊어진다(④에서만 결손).
    /// 같은 픽스처를 두 벌 만들어 대조한다 — 좁힌 기준은 성공, 종전처럼 같은 기준이면 원복.
    #[cfg(unix)]
    #[test]
    fn post_spec_narrows_only_the_after_swap_check() {
        fn fixture(root: &Path) -> (PathBuf, PathBuf) {
            let execs = ["cys-app", "cysd"];
            let dest = make_bundle(root, "cys.app", &execs, "OLD");
            let staged = staging_dir_for(&dest);
            let new = make_bundle(root, "new-src.app", &execs, "NEW");
            stage_full_copy(&new, &staged).unwrap();
            let real = staged.join("Contents/Resources/real-cysd");
            std::fs::write(&real, b"#!/bin/sh\n").unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let link = staged.join("Contents/MacOS/cysd");
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink(&real, &link).unwrap();
            (dest, staged)
        }
        let full = VerifySpec::structure_only(&["cys-app", "cysd"]);
        let narrow = VerifySpec::structure_only(&["cys-app"]); // ④에서만 뺀 항목

        // ⓐ 좁힌 ④ — 교체는 성공하고 설치본은 새 세대다(정상 업그레이드가 스스로 되돌아가지 않는다).
        let root_a = tmp_root("post-spec-narrow");
        let (dest_a, staged_a) = fixture(&root_a);
        assert!(verify(&staged_a, &full).is_empty(), "②는 넓은 기준으로도 통과한다");
        let rep = install_bundle_atomically_verifying(&staged_a, &dest_a, &full, &narrow)
            .expect("④에서 뺀 항목 때문에 원복되면 안 된다");
        assert!(marker_of(&dest_a).contains("NEW"), "설치본은 새 세대");
        assert!(rep.previous.is_some(), "옛 번들은 보존된다");
        let _ = std::fs::remove_dir_all(&root_a);

        // ⓑ 대조군 — ②와 ④가 같은 기준이면 종전대로 ④에서 걸려 원복된다(게이트는 살아 있다).
        let root_b = tmp_root("post-spec-same");
        let (dest_b, staged_b) = fixture(&root_b);
        let err = install_bundle_atomically(&staged_b, &dest_b, &full).unwrap_err();
        match &err {
            InstallError::PostVerifyFailed { restored, previous, .. } => {
                assert!(*restored, "되돌렸어야 한다");
                assert!(previous.is_none(), "되돌렸으면 대피 경로를 안내에 싣지 않는다");
            }
            other => panic!("PostVerifyFailed 여야 한다: {other:?}"),
        }
        assert!(marker_of(&dest_b).contains("OLD"), "옛 번들이 제자리로");
        let _ = std::fs::remove_dir_all(&root_b);
    }

    /// 신규 설치(옛 번들 없음) — 단일 rename 경로.
    #[cfg(unix)]
    #[test]
    fn atomic_install_handles_first_time_install() {
        let root = tmp_root("fresh");
        let dest = root.join("cys.app");
        let staged = staging_dir_for(&dest);
        let new = make_bundle(&root, "new-src.app", &["cys-app"], "NEW");
        stage_full_copy(&new, &staged).unwrap();
        let spec = VerifySpec::structure_only(&["cys-app"]);
        let rep = install_bundle_atomically(&staged, &dest, &spec).expect("신규 설치 성공");
        assert!(rep.previous.is_none(), "기존 번들이 없었으므로 previous 는 없다");
        assert!(!rep.atomic_swap, "신규 설치는 단일 rename 경로");
        assert!(marker_of(&dest).contains("NEW"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ⑤다른 파일시스템이면 복사 폴백 없이 거절한다(복사 폴백이 곧 사고의 기제).
    /// 같은 FS 인 환경에서 이 분기를 자연 재현할 수 없으므로 판정 함수 자체를 고정한다.
    #[cfg(unix)]
    #[test]
    fn cross_filesystem_staging_is_refused_not_copied() {
        let root = tmp_root("xdev");
        let a = root.join("a");
        std::fs::create_dir_all(&a).unwrap();
        assert!(same_filesystem(&a, &root), "같은 디렉터리 트리는 같은 FS");
        assert!(
            !same_filesystem(&root, Path::new("/nonexistent-volume-xyz")),
            "판정 불가는 보수적으로 false(= 교체 거절)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ★실물 릴리스 번들 게이트(수동 실행 · 기본 미실행).
    /// `VerifySpec::installed()`/`release()` 의 필수 목록이 **실제 출하 번들과 어긋나면**
    /// 기동 자기점검이 정상 설치본을 손상으로 오판한다(거짓 경보 = 최악의 회귀).
    /// 그래서 새 릴리스 아티팩트가 나올 때마다 이 게이트로 대조한다:
    ///   `CYS_VERIFY_BUNDLE=/Volumes/cys/cys.app cargo test --lib -- --ignored real_release_bundle`
    /// (2026-08-01 실행 결과: 0.14.9 정품 DMG 번들에서 결손 0건 · codesign 통과.)
    #[cfg(unix)]
    #[test]
    #[ignore = "실물 번들 경로가 필요하다 — CYS_VERIFY_BUNDLE 로 지정해 수동 실행"]
    fn real_release_bundle_satisfies_release_spec() {
        let Ok(p) = std::env::var("CYS_VERIFY_BUNDLE") else {
            panic!("CYS_VERIFY_BUNDLE=<...cys.app> 을 지정하라");
        };
        let bundle = Path::new(&p);
        assert!(bundle.is_dir(), "번들 경로가 없다: {p}");
        let defects = verify(bundle, &VerifySpec::release());
        assert!(
            defects.is_empty(),
            "정품 릴리스 번들이 우리 '완본' 정의를 만족하지 못한다 — 필수 목록이 출하물과 어긋났다:\n{}",
            describe_defects(&defects)
        );
    }

    /// `stage_full_copy` 는 복사 도구의 rc 를 반드시 본다 — 없는 원본으로 부르면 실패해야 한다
    /// (rc 무시가 곧 '반쪽 복사본을 완본으로 넘기는' 사고 경로다).
    #[cfg(unix)]
    #[test]
    fn stage_full_copy_reports_copy_failure() {
        let root = tmp_root("stage-fail");
        let staged = root.join(".stage");
        let err = stage_full_copy(&root.join("nonexistent.app"), &staged).unwrap_err();
        assert!(matches!(err, InstallError::StagingFailed { .. }), "{err:?}");
        assert!(err.destination_intact());
        let _ = std::fs::remove_dir_all(&root);
    }
}
