//! CYSJavis Pack: cys 터미널에 임베드된 멀티에이전트 운영체계 템플릿.

use std::path::{Path, PathBuf};

pub const ENV_PACK_DIR: &str = "CYS_PACK_DIR";
/// cys 전용 CLAUDE_CONFIG_DIR 오버라이드(주로 테스트 격리용). 미설정 시 pack_dir 형제(~/.cys/claude).
pub const ENV_CONFIG_DIR: &str = "CYS_CONFIG_DIR";

/// pack-update 종료코드: 디스크 팩은 반영됐으나 라이브 노드 reinject에 실패가 있어 일부 노드가
/// 미각성 상태(이전 지침으로 동작)임을 의미한다. 디스크 반영 자체는 성공이라 롤백하지 않되,
/// 성공으로 침묵 포장하지 않도록 0/일반실패(1)와 구분되는 신호다. Tauri install_pack_update
/// 브리지가 이 코드를 보고 pack-updated(디스크 갱신)+update-warning(라이브 미각성)을 함께 emit한다.
pub const EXIT_REINJECT_DEGRADED: i32 = 3;
/// run_pack_update가 reinject 집계를 stdout에 구조화 출력할 때 쓰는 줄 접두사. 호출자(Tauri
/// 브리지)가 failed/deferred를 정확히 파싱하도록 사람용 메시지와 별개의 안정 토큰으로 둔다.
pub const REINJECT_RESULT_PREFIX: &str = "PACK_UPDATE_RESULT";

// cysjavis-pack의 git-추적 전체 트리는 build.rs가 `git ls-files cysjavis-pack` 소싱으로
// 컴파일 타임 자동 임베드한다(PACK_ALL — README·directives·bin·hooks·schemas·skills 등 전체). 새
// 파일은 cysjavis-pack/ 아래에 두고 git add 하면 재빌드 시 자동 통합 — 수동 목록 갱신 불필요. 추적
// 집합이 SOT이므로 gitignore(개인정보) 경계가 구조적으로 강제되고 untracked 개인파일은 임베드되지 않는다.
include!(concat!(env!("OUT_DIR"), "/pack_all.rs"));

/// 하위호환 별칭 — 전체 트리는 PACK_ALL 단일 소스다. 외부 호출처(src/bin/cys.rs의 pack-manifest
/// 산출)가 `PACK.iter().chain(PACK_SKILLS.iter())`로 참조하므로 심볼을 보존한다: PACK은 PACK_ALL
/// 그대로, 옛 skills 전용 PACK_SKILLS는 전체 트리에 흡수돼 빈 슬라이스다(이중 카운트 0).
pub const PACK: &[(&str, &str)] = PACK_ALL;
pub const PACK_SKILLS: &[(&str, &str)] = &[];

/// ★W1 identity(3중 대조): phoenix ↔ cysd/cys 실행 신뢰원이 같은 빌드인지 교차대조하는 3필드 단일 SOT.
/// 폴백 cys 채택 시 python 이 이 3필드를 self-report(cys) vs daemon(cysd status) 로 대조한다(§5-1②).
/// ① build_id = git HEAD SHA(build.rs 임베드) ② embedded_pack_hash = 임베드 팩 트리 해시 ③ protocol version.
pub const PHOENIX_PROTOCOL_VERSION: &str = "1";

/// 빌드 식별자(git HEAD 짧은 SHA · build.rs 가 CYS_BUILD_ID 로 주입). 같은 빌드의 cys·cysd 동일.
pub fn build_id() -> &'static str {
    option_env!("CYS_BUILD_ID").unwrap_or("unknown")
}

/// 임베드 팩 매니페스트 해시 — PACK_ALL(rel+content, build.rs 가 이미 정렬)을 sha256 스트리밍 해시.
/// 같은 소스로 빌드된 cys·cysd 는 동일 값(둘 다 동일 PACK_ALL 임베드). 팩 내용이 다르면 값이 갈린다.
pub fn embedded_pack_hash() -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for (rel, content) in PACK_ALL.iter() {
        h.update(rel.as_bytes());
        h.update(b"\0");
        h.update(content.as_bytes());
        h.update(b"\0");
    }
    format!("{:x}", h.finalize())
}

/// ★팩 경로 env 키의 **우선순위 목록 정본**(W14 S19 · 2026-07-26).
///
/// 이 순서가 계약이다 — 먼저 발견되는 비어있지 않은 값이 이긴다. Python 팩의 세 구현
/// (`javis_report.pack_dir` · `javis_orchestra.pack_dir` · `javis_todo_stamp.pack_dir`)이
/// **같은 목록을 같은 순서로** 갖고, `cysjavis-pack/bin/tests/test_todo_shared_constants.py`가
/// 이 상수를 읽어 4자 기계 대조한다.
///
/// **왜 상수로 뽑았나**: 같은 개념이 5곳에 적혀 있었고(2언어 5구현) env 목록이 **3종**으로
/// 갈려 있었다. 실제 피해 — `cys todo-path`가 `AITERM_JARVIS_DIR`를 인식하지 못해, 레거시 env
/// 환경에서 **생성 위치(`~/.cys/pack/round`)와 스캔 위치(`$AITERM_JARVIS_DIR/round`)가 갈려
/// 파일이 보고기에 영영 보이지 않았다.** 설계 §14-4 4번: 한 개념이 두 곳에 적히면 그 자체가
/// 결함이다 — 재사용하거나, 못 하면 기계 대조를 박아라.
///
/// `AITERM_PACK_DIR`는 종전 `env_compat` 기계적 개명 규칙이 실제로 인정하던 키라 목록에
/// 명시적으로 남긴다(암묵을 명시로 바꾼 것이지 동작 변경이 아니다).
pub const PACK_DIR_ENV_KEYS: [&str; 4] = [
    ENV_PACK_DIR,        // "CYS_PACK_DIR"
    "JAVIS_PACK_DIR",
    "AITERM_PACK_DIR",
    "AITERM_JARVIS_DIR",
];

/// pack_dir 해석의 순수 env 층(W0-a): `PACK_DIR_ENV_KEYS` 순서대로 → 명시 경로,
/// 아무 override도 없으면 `None`(= 홈 기본 폴백 대상). panic·부수효과 없음 — 폴백-불변식
/// 회귀 테스트(pack_dir_env_precedence_and_legacy_fallbacks)가 이 순수함수를 직접 호출한다.
fn pack_dir_from_env() -> Option<PathBuf> {
    PACK_DIR_ENV_KEYS
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .map(PathBuf::from)
}

/// 홈 기반 라이브 기본 pack 경로(~/.cys/pack) — env override를 **의도적으로 무시**하는 원천 경로.
/// pack_dir()의 최종 폴백이자 W0-d 인가 게이트가 보호하는 대상(테스트가 이 경로를 가리키면 인가 필요).
fn home_default_pack_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cys/pack")
}

/// 설치 위치: $CYS_PACK_DIR (구 JAVIS_PACK_DIR·AITERM_JARVIS_DIR 폴백) 또는 ~/.cys/pack.
/// ★W0-a fail-closed: **테스트 빌드(cfg!(test))** 에서 어떤 pack env도 설정돼 있지 않으면 panic한다.
/// 테스트가 CYS_PACK_DIR 격리를 잃은 채(teardown 창 등) 라이브 `~/.cys/pack`을 만지는 재오염 벡터를
/// 조용한 폴백이 아니라 큰 소리 실패로 봉인한다. panic은 pack_dir() **본체가 아니라 이 진입점**에만
/// 있으므로, 폴백 4단 자체를 검증하는 회귀 테스트는 순수함수(pack_dir_from_env·home_default_pack_dir)를
/// 직접 호출해 영향을 받지 않는다. 프로덕션(cfg!(test) 아님)은 종전대로 홈 기본으로 폴백한다.
pub fn pack_dir() -> PathBuf {
    if let Some(p) = pack_dir_from_env() {
        return p;
    }
    #[cfg(test)]
    {
        panic!(
            "W0-a 테스트 격리 봉인 위반 — CYS_PACK_DIR 미설정 상태에서 pack_dir() 호출. \
             테스트는 반드시 CYS_PACK_DIR을 temp 샌드박스로 설정하고(권장: EnvGuard) 라이브 \
             ~/.cys/pack 을 만지지 않아야 한다."
        );
    }
    #[cfg(not(test))]
    {
        home_default_pack_dir()
    }
}

/// 소켓 경로 → 그 레인의 팩 경로(결정론 유도 · G34).
///
/// 규약: 부서 소켓은 경로 성분(unix 부모 디렉터리 / windows 파이프명)에 `cys-dept-<name>` 을 갖고,
/// 그 부서의 팩은 `~/.cys/pack-dept-<name>` 이다 — `cys-dept` 의 `dept_sock`/`dept_pack`
/// 명명 규약과 **동일 소스 규칙**(cysjavis-pack/bin/cys-dept:40-44). 부서명이 비면(`cys-dept-`)
/// 불량 레인이므로 None(javis_bootstrap `_socket_malformed_dept` 와 동일 판정).
///
/// ★W4 에서 `cys.rs` 로컬 함수에서 **lib 로 승격**했다(중복 구현 금지). 소비자가 둘이 됐기
/// 때문이다: ⓐCLI autostart 의 (소켓,팩) 쌍 보증(`cys.rs::ensure_daemon_lane_pack`) ⓑGUI 의
/// 부서장 기동(`src-tauri/main.rs::start_dept_master` — CYS_SOCKET 만 주입하면 데몬이 **본부 팩**을
/// 물려받아 그 부서 부트가 레인↔팩 가드에 exit 8 로 영구 차단된다: G34 의 GUI 지점).
/// 같은 규칙을 두 크레이트에 두 번 쓰면 그것이 RC1(사본 드리프트)의 새 인스턴스가 된다.
pub fn lane_pack_for_socket(socket: &Path) -> Option<PathBuf> {
    let name = socket
        .to_string_lossy()
        .replace('\\', "/")
        .split('/')
        .find_map(|c| c.strip_prefix("cys-dept-").map(|s| s.to_string()))
        .filter(|s| !s.is_empty())?;
    Some(
        dirs::home_dir()?
            .join(".cys")
            .join(format!("pack-dept-{name}")),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// 선언 기반 todo 판정(Declared State)의 팩 정체성 — `todo_decl::classify`가 요구하는
// `my_scope`와 `scope_exists`를 여기 한 곳에서 산출한다.
//
// **여기 두는 이유**: 소비자가 데몬(`cysd/governance.rs check_todo`)과 CLI(`cys.rs` cycle
// 저장검증) **둘**이라, 각자 구현하면 즉시 drift가 난다(파서를 lib 계층에 둔 것과 같은 이유).
// 그리고 이 판정의 정본은 팩 디렉터리 이름 자체다 — 파서에 파일시스템을 넣지 않는 설계(픽스처가
// 계약·ADR-2)라 디스크 조회는 소비자 쪽인 여기서 콜러블로 주입한다.
//
// Python 정본(`cysjavis-pack/bin/javis_report.py`의 `my_scope`·`scope_exists`)과 **같은 규칙**
// 이어야 2언어 판정이 갈리지 않는다.
// ─────────────────────────────────────────────────────────────────────────────

/// 내 팩 식별자 = `pack_dir()`의 basename. **하드코딩 금지** — 본사는 `pack`, 부서는
/// `pack-dept-dept-2` 등으로 팩 이름 자체가 정체성이다(설계 §4-1 `scope` 필드).
pub fn scope_id() -> String {
    scope_id_of(&pack_dir())
}

/// 선언된 scope가 **디스크에 실재하는 팩**인가 — 팩의 형제 디렉터리 존재로만 판정한다.
///
/// 이 판정이 필요한 이유(§4-2 R2 교정): scope가 남의 팩이라고 **무조건 조용히 배제**하면 부서
/// teardown·재생성·팩 개명 시 살아있는 파일이 통째로 사라져 07-11 유령 사고를 거울상으로
/// 재현한다. 실재하면 정상(조용한 배제 = foreign-scope), 실재하지 않으면 orphan-scope로
/// 시끄럽게 알린다. 입력이 "디렉터리 존재"라 시간 의존이 없고 결정론이다.
pub fn scope_exists(scope: &str) -> bool {
    scope_exists_in(&pack_dir(), scope)
}

/// `scope_id`의 순수 부분 — pack_dir() 봉인(W0-a)에 걸리지 않고 테스트가 직접 검증한다.
/// 상대 경로는 cwd 기준으로 절대화한 뒤 basename을 뽑는다(Python `basename(normpath(abspath(…)))` 등가).
fn scope_id_of(dir: &Path) -> String {
    absolutize(dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 부서 팩 판정(순수 · G3 축1) — basename 의 `pack-dept-` 접두를 벗겨 부서명을 얻는다
/// (빈 이름 = 불량 레인 = None · `lane_pack_for_socket` 의 `_socket_malformed_dept` 판정과 동형).
///
/// 명명 규칙의 등재소는 **하나**다: `cys-dept` 의 `dept_pack`(cysjavis-pack/bin/cys-dept:52
/// `~/.cys/pack-dept-<name>`)·위 `lane_pack_for_socket` 과 동일 규칙이며, 세 번째 소비자
/// (config 시드 표적 판정·hooks-prune 게이트·init-pack 부서 게이트)가 생기면서 순수 함수로
/// 승격했다 — 같은 규칙을 소비처마다 다시 쓰면 RC1(사본 드리프트)의 새 인스턴스가 된다.
///
/// ★부서 판정 술어 통일표(2언어 — H-SEED-6 파리티 핀이 기계 대조 · G3 축1 확정):
///   Rust   ① `pack::dept_scope_of`(이 함수 — basename `pack-dept-` 접두)
///          ② `factory_reset::command_points_into_pack`(`<base>/pack` 경계 + `-dept-` 꼬리 —
///             "이 설치의 팩 전체" 소유 판정 · factory reset 전용 광의)
///   Python ③ `javis_preflight._discover_isolation_block` 의 `_pack_is_dept`(basename
///             'pack-dept-' 접두 · 2026-06-30 실재) — **C28 부서 게이트를 새로 만들지 않는다**
///             (같은 함수에 제3 술어 중복 = 드리프트 원천)
///          ④ `javis_preflight.is_dept_pack`(pack_dir ≠ 기본 — CEO·임시 팩 포함 광의 · C03 면제용)
///          ⑤ `javis_preflight` C56 `_dept_hooks_in`('/pack-dept-' 경로 앵커 — 글로벌 누수
///             invariant **탐지**)
///   **제거 엔진은 `cys hooks-prune`(`factory_reset::strip_hooks_pointing_into_pack`) 단일**이다 —
///   C56/C57 은 탐지 invariant(+기존 레거시 청소) 레인이며 파이썬에 신규 제거 로직을 늘리지 않는다.
pub fn dept_scope_of(pack: &Path) -> Option<String> {
    absolutize(pack)
        .file_name()
        .and_then(|n| n.to_str().map(str::to_owned))
        .and_then(|n| n.strip_prefix("pack-dept-").map(str::to_owned))
        .filter(|s| !s.is_empty())
}

/// `scope_exists`의 순수 부분(팩 경로 주입형).
fn scope_exists_in(pack: &Path, scope: &str) -> bool {
    // 경로 탈출 방어 — G4 값 문법(`[A-Za-z0-9._:-]+`)상 정상 선언엔 나올 수 없는 형태지만,
    // 깨진 선언이 파서를 통과하는 미래 변경에 대비해 소비자 쪽에서도 막는다.
    if scope.is_empty()
        || scope == "."
        || scope == ".."
        || scope.contains('/')
        || scope.contains('\\')
    {
        return false;
    }
    match absolutize(pack).parent() {
        Some(parent) => parent.join(scope).is_dir(),
        None => false,
    }
}

/// 상대 경로를 cwd 기준으로 절대화한다(cwd 조회 실패는 원본 그대로 — 패닉 0).
fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(p),
        Err(_) => p.to_path_buf(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// W0-d 양성 인가 게이트 — 팩 쓰기 경로는 대상이 **라이브 기본 경로**(~/.cys/pack)일 때
// 양성 인가(PackWriteAuth) 없이는 하드 거부(Err)한다. 인가는 프로덕션 진입점(cys init-pack
// CLI 핸들러·pack-update·pack-downgrade·cysd 부팅 자동설치)만 `PackWriteAuth::production()`으로
// 부여한다(스코프 토큰 타입 — env 아님). 비라이브 대상(테스트 샌드박스·CYS_PACK_DIR temp) 쓰기는
// 인가가 필요 없다. 원칙: 부정 감지(테스트 감지)가 아니라 양성 인가(프로덕션만 라이브 쓰기 허가).
// ─────────────────────────────────────────────────────────────────────────────

/// 팩 쓰기 인가 증표(W0-d) — 라이브 기본 pack_dir에 쓰기를 허가하는 스코프 토큰.
/// 필드가 private이라 crate 외부는 `production()`으로만 생성할 수 있고, crate 내부 테스트는
/// `for_test()`로 명시 인가를 만든다. **env가 아니라 값**으로 흐르므로 teardown 창·잔류 env에
/// 영향받지 않는다. Copy라 여러 쓰기 단계로 부담 없이 전달된다.
#[derive(Clone, Copy)]
pub struct PackWriteAuth {
    _seal: (),
}

impl PackWriteAuth {
    /// 프로덕션 진입점 전용 라이브 쓰기 인가. 호출처 = cys init-pack·pack-update·pack-downgrade
    /// CLI 핸들러 + cysd 부팅 자동설치. (테스트에서 라이브를 실제로 쓰려 이걸 부르면 안 된다 —
    /// 테스트는 temp 대상이라 인가 자체가 불필요하다.)
    ///
    /// 실경계 고지: `pub`은 바이너리 crate(cys·cysd)가 접근해야 하는 구조적 필요이며, 언어
    /// 차원에서 "프로덕션 4곳만 생성 가능"을 강제하지는 못한다. 실효 방어는 W0-a(cfg-test
    /// fail-closed)+W0-c(.cargo env 샌드박스)+W0-d(라이브 경로 인가 게이트)의 조합이고,
    /// 테스트 코드가 이 토큰을 의도적으로 위조해 라이브 경로를 직접 겨냥하는 벡터까지는
    /// 막지 않는다 — 그 벡터는 코드리뷰 게이트 소관이다.
    pub fn production() -> Self {
        PackWriteAuth { _seal: () }
    }

    /// crate 내부 테스트 전용 명시 인가(라이브 경로 대상 쓰기 로직을 의도적으로 검증할 때만).
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        PackWriteAuth { _seal: () }
    }
}

/// 경로를 비교 가능한 정규형으로 해소: 존재하는 최장 접두를 canonicalize(심링크·`..`·상대경로
/// 해소)하고 남은 **미존재 꼬리**를 그대로 이어 붙인다. macOS `/tmp`→`/private/tmp` 심링크와
/// 아직 생성되지 않은 pack 디렉터리(rename 대상)를 둘 다 정확히 비교하기 위함.
pub(crate) fn resolve_for_compare(p: &Path) -> PathBuf {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(p)
    };
    let mut existing = abs.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(c) = std::fs::canonicalize(&existing) {
            let mut out = c;
            for comp in tail.iter().rev() {
                out.push(comp);
            }
            return out;
        }
        match existing.file_name() {
            Some(name) => {
                tail.push(name.to_os_string());
                match existing.parent() {
                    Some(par) => existing = par.to_path_buf(),
                    None => return abs,
                }
            }
            None => return abs,
        }
    }
}

/// dir이 (심링크·상대경로 해소 후) 라이브 기본 pack 경로(~/.cys/pack)를 가리키는가.
/// env override와 무관하게 홈 기반 원천 경로만 본다 — 보호 대상은 실제 라이브 팩이다.
fn targets_live_default(dir: &Path) -> bool {
    resolve_for_compare(dir) == resolve_for_compare(&home_default_pack_dir())
}

/// W0-d 게이트: 라이브 기본 경로 대상 쓰기인데 인가 토큰이 없으면 하드 거부(Err).
/// 비라이브 대상이거나 인가가 있으면 Ok — 아무 부수효과도 없는 순수 검사(쓰기 전에 선차단).
fn authorize_pack_write(dir: &Path, auth: Option<PackWriteAuth>) -> Result<(), String> {
    if auth.is_none() && targets_live_default(dir) {
        return Err(format!(
            "W0-d 인가 없는 라이브 팩 쓰기 거부 — 대상 {} 은 라이브 기본 경로(~/.cys/pack)이며 \
             PackWriteAuth(프로덕션 진입점 전용) 없이는 쓸 수 없다. 테스트는 CYS_PACK_DIR temp \
             대상으로 쓰거나 명시 인가를 부여하라.",
            dir.display()
        ));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 소망상태 훅 매니페스트 (T-0147-7 W3 · A9 · 재감사 §3 CS-7③)
// ─────────────────────────────────────────────────────────────────────────────
/// "이 config 계급에는 어떤 훅이 등록돼 있어야 하는가"의 **단일 데이터 소스**.
///
/// ★왜 필요한가(A9): 종전엔 소망상태가 **집행자마다 흩어져** 있었다 —
///   ① Rust 시드(`setup_isolated_config_dir`)는 `settings.json` **파일이 없을 때만** 2훅을 썼고,
///   ② init-pack(`cys.rs::install_claude_hook`)은 SessionStart **하나만** 등록했고,
///   ③ 파이썬 preflight C28(`SELFCORR_HOOKS`)만 UserPromptSubmit(role-bootstrap)을 등록했는데
///      그 C28의 유일한 트리거가 **결손된 그 훅 자신**이었다(결정론 층 닭·달걀).
///   결과: `settings.json`이 이미 있는 기계(=거의 전부)는 각성 훅이 영구 미등록일 수 있었다.
///
/// 그래서 소망 집합은 데이터로 한 곳에 적고(1 데이터 × N 집행자), 집행은 **파일 단위가 아니라
/// 이벤트 단위 멱등 병합**(`merge_desired_hooks`)으로 한다. 파이썬 측 사본(`javis_preflight.
/// SELFCORR_HOOKS` + C08 session-start)은 `bin/tests/run_bootstrap_health.py` H-SEED-1이 이
/// 상수를 파싱해 **집합 대조**한다(test_todo_shared_constants 선례 — 언어 경계는 기계 대조로 결박).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesiredHook {
    /// pack `hooks/` 아래 스크립트 basename.
    pub script: &'static str,
    /// Claude Code hook 이벤트명.
    pub event: &'static str,
    /// matcher(툴 필터). None = matcher 키 미기록(전체).
    pub matcher: Option<&'static str>,
    /// ★U-21 · **선언 timeout(초)**. `None` = 키 미기록(하네스 기본값에 맡김).
    ///
    /// 필드 위치는 계약이다 — **반드시 `matcher` 뒤**. `bin/tests/run_bootstrap_health.py`
    /// `_rust_awakening_hooks()` 가 이 리터럴을 `script → event → matcher → timeout` 순서로
    /// 파싱해 파이썬 매니페스트와 4-튜플 대조한다(순서가 바뀌면 대조가 조용히 짝을 잃는다).
    pub timeout: Option<u64>,
}

/// ★Claude Code 훅 계약 실측 — `UserPromptSubmit` 의 **기본 timeout 은 30초**다(다른 이벤트는 600초).
///
/// 그리고 timeout 은 지연이 아니라 **취소 + 출력 폐기**다. role-bootstrap 훅의 stdout 은
/// `additionalContext`(마스터 선언 → 팀 기동 사실 + 착수 규율)라서, 30초를 넘긴 순간 부트 체인은
/// 에러도 남기지 않고 **조용히 사라진다**.
///
/// 그 30초가 얼마나 얇은지는 훅 자신의 데드라인 합으로 계산된다(`cysjavis-pack/hooks/role-bootstrap.sh`):
/// 역할 게이트 2s + 임무 record 5s + 임무 path 5s + machine-origin 5s + 선행 claim 10s = **27s**.
/// 여기에 인터프리터 냉시작(Windows python 은 회당 수 초)이 4회 얹히면 상한을 넘는다 —
/// mac 에서는 멀쩡하고 Windows 설치본에서만 부트가 사라지는 그 계열이다.
pub const HOOK_TIMEOUT_PLATFORM_DEFAULT_UPS_S: u64 = 30;

/// `UserPromptSubmit` 각성 훅의 **선언 timeout**.
///
/// 값 선정 — 새 숫자를 발명하지 않고 **하네스가 다른 모든 이벤트에 쓰는 기본값(600s)** 에 맞춘다.
/// "UserPromptSubmit 만 특별히 얇은" 비대칭을 없애는 것이 이 단위의 목적이고, 그보다 큰 값은
/// 근거가 없고 그보다 작은 값은 위 27s 여유를 다시 깎는다.
///
/// ★이 값이 부트 체인 최악치(`javis_budget.bootstrap_chain_worst_s()` ≈ 3020s)보다 **작다**는
/// 사실은 결함이 아니라 **관측 결과**다 — 훅 자식 안에서 부트를 완주시킬 수 없다는 뜻이고,
/// 그것이 곧 "부트를 훅 자식에서 떼어내라"(U-23)의 기계적 증명이다. 여기서 값을 3020 으로
/// 올려 역전을 지우는 것은 수리가 아니라 **증거 인멸**이다.
pub const HOOK_TIMEOUT_ROLE_BOOTSTRAP_S: u64 = 600;

/// **각성 티어**(awakening) — 없으면 마스터 선언이 부트를 발화하지 못하는(=팀이 영구히 안 뜨는)
/// 훅 집합. Rust 격리 config 시드·init-pack·개인 프로필 병합(T-0147-1)·preflight C28의 FAIL 티어가
/// **모두 이 집합**을 소비한다. 순서는 계약이 아니지만(집합 대조) 결정론 출력을 위해 고정한다.
pub const AWAKENING_HOOKS: [DesiredHook; 2] = [
    DesiredHook {
        script: "session-start.sh",
        event: "SessionStart",
        matcher: None,
        // SessionStart 의 하네스 기본값은 이미 600s 다 — 선언하지 않는다.
        // ★무선언은 "아무것도 단언하지 않는다"는 뜻이지 "0 을 강제한다"가 아니다(아래 판정 참조).
        timeout: None,
    },
    DesiredHook {
        script: "role-bootstrap.sh",
        event: "UserPromptSubmit",
        matcher: None,
        timeout: Some(HOOK_TIMEOUT_ROLE_BOOTSTRAP_S),
    },
];

// ═════════════════════════════════════════════════════════════════════════════
// U-21 · 선언 timeout 축의 **롤백 스위치**
// ═════════════════════════════════════════════════════════════════════════════

/// 이 축 전용 노브. `"1"` 만 끈다(형제 축과 같은 엄격 비교).
pub const ENV_HOOK_TIMEOUT_V1: &str = "CYS_HOOK_TIMEOUT_V1";

/// 축 전용 노브의 **순수 코어**.
pub fn hook_timeout_v1_from(env_val: Option<&str>) -> bool {
    env_val == Some("1")
}

/// 축의 **최종 유효값**(순수) — 마스터 `CYS_BOOT_GATES=0` 에 접힌다.
///
/// ★새 판정 축은 태어날 때 마스터에 접는다([`crate::gate_axes_from`] 의 규율). 사고 순간에
/// 사람이 노브 조합을 기억할 수는 없다 — 손잡이는 하나여야 한다.
pub fn hook_timeout_axis_legacy_from(master_env: Option<&str>, axis_env: Option<&str>) -> bool {
    crate::boot_gates_master_off_from(master_env) || hook_timeout_v1_from(axis_env)
}

/// 위의 단일 진입점(env 2회 판독 — 축별 1지점 규약).
pub fn hook_timeout_axis_legacy() -> bool {
    hook_timeout_axis_legacy_from(
        std::env::var(crate::ENV_BOOT_GATES).ok().as_deref(),
        std::env::var(ENV_HOOK_TIMEOUT_V1).ok().as_deref(),
    )
}

/// 소망 훅의 **유효 선언 timeout** — 축이 종전이면 `None`(= 선언 없음).
///
/// 롤백이 이 한 지점으로 끝나는 이유: 선언이 `None` 이면 판정은 `command` 만 보고(종전과 동일),
/// 병합기는 `timeout` 키를 쓰지도 고치지도 않는다. 코드 revert 없이 거동이 종전으로 되돌아간다.
pub fn declared_timeout_for(h: &DesiredHook) -> Option<u64> {
    if hook_timeout_axis_legacy() {
        None
    } else {
        h.timeout
    }
}

/// hook 등록 명령 문자열(단일 OS 규약) — `session_start_hook_command`·
/// `role_bootstrap_hook_command`의 일반화. `javis_preflight._cys_hook_cmd(script)`와
/// **byte-identical**이어야 한다(두 writer가 같은 문자열을 내야 중복 append 0·matcher 일치).
pub fn hook_command_for(pack_dir: &Path, script: &str) -> String {
    let path = pack_dir.join("hooks").join(script);
    if cfg!(windows) {
        format!("bash \"{}\"", path.display().to_string().replace('\\', "/"))
    } else {
        format!("sh {}", path.display())
    }
}

/// settings.json hook 객체의 `timeout` 을 수로 읽는다. 수가 아니면 `None`(= 미기록 취급).
/// 사용자 파일에 `900.0` 같은 실수가 들어 있을 수 있어 정수·실수 둘 다 본다.
fn hook_entry_timeout(h: &serde_json::Value) -> Option<u64> {
    let v = h.get("timeout")?;
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)
}

/// **선언 timeout 충족** 판정(순수 · 진리표 대상).
///
/// 규약 — 선언(`declared`)은 **하한**이다.
///  · `declared == None` → 아무것도 단언하지 않는다(종전 판정과 동일). 우리가 선언하지 않은 축을
///    "0 이어야 한다"로 읽으면 사용자가 손으로 넣은 값이 전부 불일치가 되어 **우리가 그것을 지운다**.
///  · `declared == Some(t)` → 엔트리가 **`t` 이상**이어야 충족. `==`(동등)을 기각한 이유를 남긴다:
///    우리보다 **큰** 값을 우리 값으로 내리는 것은 취소 시각을 앞당기는 **오살 방향**이다
///    (살아서 완주하던 부트가 우리 손에 잘린다). 이 축은 **한 방향으로만 연다**.
///    ★이것은 판정 완화가 아니다 — 종전 판정은 timeout 을 **아예 보지 않았고**(무조건 충족),
///    이 술어는 그보다 **엄격**하다(미기록·저값은 불충족). 완화가 아니라 새 하한의 도입이다.
///  · 엔트리에 키가 **없으면 불충족**이다. 하네스 기본값(UserPromptSubmit=30s)을 우리가 읽을 길이
///    없으므로 "없음 = 안전"으로 접으면 원 결함(무음 취소)이 그대로 남는다.
pub fn hook_timeout_satisfied(entry_timeout: Option<u64>, declared: Option<u64>) -> bool {
    match declared {
        None => true,
        Some(want) => matches!(entry_timeout, Some(v) if v >= want),
    }
}

/// settings.json 의 특정 이벤트에 desired 명령이 **선언 timeout 을 충족한 채** 등록돼 있는가
/// (순수 판정 · 읽기 전용). 판정 = `command 동등 ∧ 선언 timeout 충족`.
pub fn hook_registered_with_timeout_in(
    root: &serde_json::Value,
    event: &str,
    desired: &str,
    declared_timeout: Option<u64>,
) -> bool {
    root.get("hooks")
        .and_then(|h| h.get(event))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().any(|m| {
                m.get("hooks")
                    .and_then(|v| v.as_array())
                    .map(|hs| {
                        hs.iter().any(|h| {
                            h.get("command").and_then(|c| c.as_str()) == Some(desired)
                                && hook_timeout_satisfied(hook_entry_timeout(h), declared_timeout)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// settings.json 의 특정 이벤트에 desired 명령이 등록돼 있는가(순수 판정 · 읽기 전용).
///
/// ★U-21 이후 이 함수는 **command 축 단독** 판정이다(선언 timeout 미고려 = `declared: None`).
/// 집행 경로(`merge_desired_hooks`·`verify_desired_hooks_registered`)는
/// [`hook_registered_with_timeout_in`] 을 쓴다. 관측 전용 소비처(`cys doctor`·부트 경고)는
/// 이 얇은 형태를 유지한다 — timeout 스큐만으로 "훅이 없습니다" 경고를 내는 것은 오탐이고,
/// 그 오탐이 유도하는 재등록은 **중복 append 폭주** 방향이기 때문이다.
pub fn hook_registered_in(root: &serde_json::Value, event: &str, desired: &str) -> bool {
    hook_registered_with_timeout_in(root, event, desired, None)
}

/// **불일치 엔트리 교체 경로**(U-21) — 이미 등록된 우리 hook 객체의 `timeout` 만 선언값으로 올린다.
///
/// 안전 계약(오살 금지 — 이 함수가 이 단위에서 가장 위험한 코드다):
///  · `command` 가 우리 것과 **바이트 동등**인 hook 객체만 만진다. 사용자 항목·타 도구 훅·
///    같은 이벤트의 다른 엔트리는 읽지도 않는다.
///  · 건드리는 키는 `timeout` **하나**다. 삭제·재배치·순서 변경 0.
///  · `max(기존, 선언)` — 사용자가 더 크게 잡아둔 값은 **내리지 않는다**.
///  · 반환: 실제로 값이 바뀐 객체가 하나라도 있었는가.
fn raise_hook_timeout_in(
    root: &mut serde_json::Value,
    event: &str,
    desired: &str,
    want: u64,
) -> bool {
    let Some(arr) = root
        .get_mut("hooks")
        .and_then(|h| h.get_mut(event))
        .and_then(|v| v.as_array_mut())
    else {
        return false;
    };
    let mut changed = false;
    for entry in arr.iter_mut() {
        let Some(hs) = entry.get_mut("hooks").and_then(|v| v.as_array_mut()) else {
            continue;
        };
        for h in hs.iter_mut() {
            if h.get("command").and_then(|c| c.as_str()) != Some(desired) {
                continue;
            }
            let cur = hook_entry_timeout(h);
            let next = cur.map(|c| c.max(want)).unwrap_or(want);
            if cur == Some(next) {
                continue;
            }
            if let Some(obj) = h.as_object_mut() {
                obj.insert("timeout".into(), serde_json::json!(next));
                changed = true;
            }
        }
    }
    changed
}

/// 소망 훅을 settings.json 에 **이벤트 단위로 멱등 병합**한다(A9).
///
/// 계약(사용자 불가침 — ★W-B seed-once 교리 정합):
///  · **추가만 한다**. 사용자 항목·타 도구 훅·기존 배열 순서를 지우거나 재배치하지 않는다.
///  · 이미 같은 명령이 그 이벤트에 있으면 **무동작**(byte-identical 문자열이라 중복 0).
///  · ★U-21 **불일치 엔트리 교체 경로**: 명령은 같은데 **선언 timeout 만 미달**이면 append 가
///    아니라 그 hook 객체의 `timeout` 하나만 올린다(`raise_hook_timeout_in`). append 로 처리하면
///    같은 명령이 두 번 실려 **매 프롬프트마다 훅이 2회 발화**한다 — 큐 폭주 방향이다.
///  · 추가할 것이 없으면 파일을 **쓰지 않는다**(백업도 건드리지 않는다 — 정상 백업 클로버 차단).
///  · symlink 거부 / 파싱 실패 시 **거부**(빈 객체로 덮어쓰기 금지 — 침묵 데이터 소실 차단).
///  · 쓰기는 `write_atomic`(tmp+rename+fsync) — 반쪽 파일이 굳으면 훅 등록부 전체가 사라진다.
///
/// 반환: 실제로 추가한 항목의 사람용 라벨(`"SessionStart←session-start.sh"`). 빈 벡터 = 이미 충족.
pub fn merge_desired_hooks(
    settings_path: &Path,
    pack_dir: &Path,
    hooks: &[DesiredHook],
) -> Result<Vec<String>, String> {
    if std::fs::symlink_metadata(settings_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(format!(
            "{} is a symlink — refusing to write",
            settings_path.display()
        ));
    }
    let existed = settings_path.exists();
    let mut root: serde_json::Value = match std::fs::read_to_string(settings_path) {
        Ok(s) if s.trim().is_empty() => serde_json::json!({}),
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("settings parse error: {e}"))?,
        // 파일 없음일 때만 빈 설정으로 시작 — 권한 등 다른 읽기 에러를 무시하면
        // 기존 settings.json이 hooks만 남은 JSON으로 대체될 수 있다.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(format!("settings read error: {e}")),
    };
    if !root.is_object() {
        return Err("settings root is not an object".into());
    }
    let mut added: Vec<String> = Vec::new();
    for h in hooks {
        let desired = hook_command_for(pack_dir, h.script);
        let declared = declared_timeout_for(h);
        if hook_registered_with_timeout_in(&root, h.event, &desired, declared) {
            continue;
        }
        // ★교체 경로 — 명령은 이미 있고 선언 timeout 만 미달인 경우. 여기서 append 로 가면
        //   동일 명령이 중복 등재된다(발화 2배 = 폭주 방향).
        if let Some(want) = declared {
            if hook_registered_in(&root, h.event, &desired)
                && raise_hook_timeout_in(&mut root, h.event, &desired, want)
            {
                added.push(format!("{}←{} (timeout={want}s 교정)", h.event, h.script));
                continue;
            }
        }
        let mut entry = serde_json::json!({"hooks": [{"type": "command", "command": desired}]});
        if let Some(t) = declared {
            entry["hooks"][0]["timeout"] = serde_json::json!(t);
        }
        if let Some(m) = h.matcher {
            entry["matcher"] = serde_json::Value::String(m.to_string());
        }
        let arr = root
            .as_object_mut()
            .ok_or("settings root is not an object")?
            .entry("hooks")
            .or_insert(serde_json::json!({}))
            .as_object_mut()
            .ok_or("hooks is not an object")?
            .entry(h.event.to_string())
            .or_insert(serde_json::json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("hooks.{} is not an array", h.event))?;
        arr.push(entry);
        added.push(format!("{}←{}", h.event, h.script));
    }
    if added.is_empty() {
        return Ok(added);
    }
    // backup — 실제 write가 발생할 때만(멱등 재실행이 정상 백업을 클로버하지 않게).
    if existed {
        let backup = format!("{}.bak-cys", settings_path.display());
        std::fs::copy(settings_path, &backup).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    write_atomic(settings_path, body.as_bytes()).map_err(|e| e.to_string())?;
    Ok(added)
}

/// **이미 등록된 우리 훅의 `timeout` 만 재조정**한다 — 훅을 새로 설치하지 않는다(U-29 · M-09-a).
///
/// ## 무엇이 결함이었나 (2026-08-24 · 기준선 v0.14.24 ↔ 현재 격리 실주행 대조)
///
/// GUI 인앱 업데이트는 **항상** `cys init-pack --no-install-hook` 으로 내려온다
/// (`src-tauri/src/main.rs` `maybe_apply_pending_update`). 그 플래그는 [`setup_isolated_config_dir`]
/// 의 훅 병합을 통째로 건너뛰므로 **팩 파일은 새 훅으로 교체되는데 훅 *등록*(settings.json 의
/// `timeout` 필드)은 갱신되지 않는다**(격리 실주행 실측: 업데이트 전후 등록부 바이트 동일).
///
/// 그런데 새 훅은 종전보다 **느리다**(같은 대조의 실측): python 프로세스 기동 14→16회 ·
/// `cys` 호출 3→4회 · 응답 지연 1초 환경에서 14.83→17.12초, 2초에서 27.80→32.15초,
/// 3초에서 40.77→47.15초. 즉 `timeout` 이 [`HOOK_TIMEOUT_ROLE_BOOTSTRAP_S`] 로 등록된 기계는
/// 무해하지만, **옛 하네스 기본 상한([`HOOK_TIMEOUT_PLATFORM_DEFAULT_UPS_S`])이 남은 기계는
/// 절단 확률이 올라간다** — 그리고 인앱 업데이트 사용자가 정확히 그 상태가 된다. U-21 이 만든
/// 축이 **도달하지 못해서** 나빠지는 자리이므로, 수리는 값이 아니라 **도달성**이다.
///
/// ## 왜 이것이 `--no-install-hook` 의 존재 이유를 깨지 않는가
///
/// 그 플래그가 지키는 것은 둘이다 — ①사용자 프로필(`~/.claude*`) 불가침 ②훅을 새로 **설치**하지
/// 않는다(활성 프로필 재직렬화·정상 `.bak-cys` 클로버 방지). 이 함수는 둘 다 지킨다:
///  · **대상이 격리 config dir 한 곳**이다. 호출부가 개인 프로필 경로를 넘기지 않는다 — 사용자
///    프로필은 이 경로에서 읽지도 쓰지도 않는다(그쪽은 종전대로 생략된 채 남는다).
///  · **파일이 없으면 만들지 않고 즉시 반환**한다. 등록이 없다는 뜻이고, 여기서 파일을 만드는
///    것이 곧 '훅 설치'다(회귀 핀 `no_install_hook_consistency` 가 그 부재를 단언한다).
///  · **이벤트·엔트리를 추가하지 않는다.** 집행은 U-21 이 이미 만든 교체 경로
///    [`raise_hook_timeout_in`] 하나이고, 그것은 `command` 가 바이트 동등인 기존 hook 객체의
///    `timeout` 키만 만지므로 구조적으로 append 가 불가능하다(새 기구를 만들지 않는다).
///  · **값이 실제로 바뀌지 않으면 쓰지 않는다.** 한 기계에서 이 함수가 디스크를 만지는 것은
///    스큐가 남아 있던 **1회뿐**이고, 그 뒤의 모든 업데이트는 재직렬화 0·백업 무접촉이다.
///  · `max(기존, 선언)` — 사용자가 더 크게 잡아둔 값은 **내리지 않는다**(오살 금지 · 그 술어도
///    [`raise_hook_timeout_in`] 소유다).
///
/// ## 롤백
/// 선언값은 [`declared_timeout_for`] 를 통과한다 — 축 노브(`CYS_HOOK_TIMEOUT_V1=1`)나 마스터
/// (`CYS_BOOT_GATES=0`)를 누르면 선언이 `None` 이 되고 이 함수는 **아무 파일도 만지지 않는다**.
///
/// 반환: 실제로 값이 오른 항목의 사람용 라벨. 빈 벡터 = 만질 것이 없었다(정상·멱등).
pub fn retune_registered_hook_timeouts(
    settings_path: &Path,
    pack_dir: &Path,
    hooks: &[DesiredHook],
) -> Result<Vec<String>, String> {
    // ★'없으면 만들지 않는다' — 이 한 줄이 '재조정'과 '설치'를 가르는 경계다.
    if !settings_path.exists() {
        return Ok(vec![]);
    }
    if std::fs::symlink_metadata(settings_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(format!(
            "{} is a symlink — refusing to write",
            settings_path.display()
        ));
    }
    let mut root: serde_json::Value = match std::fs::read_to_string(settings_path) {
        // 빈 파일 = 등록 0 — 만질 것이 없다(빈 객체로 덮어쓰지 않는다).
        Ok(s) if s.trim().is_empty() => return Ok(vec![]),
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("settings parse error: {e}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("settings read error: {e}")),
    };
    if !root.is_object() {
        return Err("settings root is not an object".into());
    }
    let mut raised: Vec<String> = Vec::new();
    for h in hooks {
        // 선언이 없는 훅(SessionStart)은 축 자체가 없다 — 만지지 않는다.
        let Some(want) = declared_timeout_for(h) else {
            continue;
        };
        let desired = hook_command_for(pack_dir, h.script);
        if raise_hook_timeout_in(&mut root, h.event, &desired, want) {
            raised.push(format!("{}←{} (timeout={want}s 재조정)", h.event, h.script));
        }
    }
    if raised.is_empty() {
        return Ok(raised);
    }
    let backup = format!("{}.bak-cys", settings_path.display());
    std::fs::copy(settings_path, &backup).map_err(|e| e.to_string())?;
    let body = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    write_atomic(settings_path, body.as_bytes()).map_err(|e| e.to_string())?;
    Ok(raised)
}

/// 설치 직후 **'등록 집합 ⊇ 소망 집합'** 검증(W3 게이트) — 미충족 항목을 반환한다(빈 벡터=충족).
///
/// 왜 병합 뒤에 다시 읽는가: 병합기는 '내가 쓴 것'을 알지만 **디스크가 실제로 그 상태인지**는
/// 다른 사실이다(권한·경합·부분 쓰기·심링크 거부). 보고를 실측에서 파생시키는 것이 CS-3 계약이고,
/// 이 검증이 없으면 "시드했다"는 주장만 남는다(A9 가 정확히 그 자리에서 났다).
pub fn verify_desired_hooks_registered(
    settings_path: &Path,
    pack_dir: &Path,
    hooks: &[DesiredHook],
) -> Vec<String> {
    let root: serde_json::Value = std::fs::read_to_string(settings_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    hooks
        .iter()
        .filter(|h| {
            // ★U-21: ⊇ 게이트는 **집행 축과 같은 판정**을 써야 한다. 병합기가 timeout 을 보고
            //   ⊇ 검증이 안 보면, timeout 스큐가 남은 디스크를 "충족"으로 보고하게 된다.
            !hook_registered_with_timeout_in(
                &root,
                h.event,
                &hook_command_for(pack_dir, h.script),
                declared_timeout_for(h),
            )
        })
        .map(|h| format!("{}←{}", h.event, h.script))
        .collect()
}

/// 홈 직하 개인 Claude 프로필 디렉터리(`.claude` / `.claude-*`)의 settings.json 경로들.
/// ★G7: **디렉터리 존재** 기준이다(settings.json 부재 프로필도 후보 — 병합기가 생성한다).
/// 결정론: 사전순. 홈 미해소·읽기 실패는 빈 목록(부트 게이트라 crash 금지).
pub fn personal_profile_settings_paths() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return vec![];
    };
    personal_profile_dirs_under(&home)
        .into_iter()
        .map(|d| d.join("settings.json"))
        .collect()
}

/// 홈 직하 개인 Claude 프로필 **디렉토리**(`.claude` / `.claude-*`) 열거 — 홈 주입판.
/// 소비자: 위 settings 경로 파생 + factory_reset(훅 strip·스킬 심링크 제거 대상 열거).
/// 판별 규칙이 두 곳에 갈리면 등록과 해제가 서로 다른 프로필을 보게 되므로 여기 한 곳에만 둔다.
pub fn personal_profile_dirs_under(home: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(home) else {
        return vec![];
    };
    let mut dirs_found: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n == ".claude" || n.starts_with(".claude-"))
                .unwrap_or(false)
                && e.path().is_dir()
        })
        .map(|e| e.path())
        .collect();
    dirs_found.sort();
    dirs_found
}

/// ★T-0147-1 레거시 각성 경로 — 개인 프로필(`~/.claude*`)에 **각성 훅만** 멱등 병합한다.
///
/// 왜(오너 명시 요구 · P0): preflight C28의 home-glob 등록은 이미 있었지만 **닭·달걀**이었다 —
/// 첫 훅이 없으면 preflight가 실행되지 않고, preflight가 실행되지 않으면 훅이 등록되지 않는다.
/// 데몬 pack install(=기동마다 도는 유일한 결정론 집행자)이 이 병합을 하면 그 고리가 끊긴다.
///
/// 격리 교리와의 관계: `~/.claude` 불가침은 **오너가 훅 병합에 한해 의식적으로 완화**한 결정이다
/// (memory `awakening-dual-path-requirement`). 병합은 추가만 하며 사용자 항목을 절대 건드리지
/// 않는다(`merge_desired_hooks` 계약).
/// 안전 전제(W1a A2): 훅 최선두에 surface 이중 게이트(CYS_SURFACE_ID∥AITERM_SURFACE_ID)가 있는
/// 바이너리와 **같은 릴리스**다 — 비-cys 세션에서 이 훅이 등록돼 있어도 부트를 발화하지 않는다
/// (하드 제약 2 충족). 그 게이트가 없다면 이 병합은 오발화 표면을 넓히는 결함이 된다.
///
/// 범위 게이트: **base 팩 전용**. 부서 팩(`pack-dept-*`)·임시 팩(테스트·스냅샷)에서 도는 install은
/// 개인 프로필을 건드리지 않는다(F1 레인 격리·테스트 부작용 0). 옵트아웃: `CYS_NO_PERSONAL_HOOK_MERGE=1`.
/// best-effort — 실패해도 팩 설치 자체는 유효(로그만 남긴다).
pub fn merge_awakening_hooks_into_personal_profiles() -> Vec<(String, Vec<String>)> {
    if cfg!(test) {
        return vec![]; // 테스트 빌드는 실 HOME 을 절대 만지지 않는다
    }
    if std::env::var("CYS_NO_PERSONAL_HOOK_MERGE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return vec![];
    }
    let pack = pack_dir();
    if pack != home_default_pack_dir() {
        return vec![]; // 부서/임시/테스트 팩 — 개인 프로필 무접촉
    }
    let mut out = vec![];
    for settings in personal_profile_settings_paths() {
        match merge_desired_hooks(&settings, &pack, &AWAKENING_HOOKS) {
            Ok(added) if added.is_empty() => {}
            Ok(added) => {
                eprintln!(
                    "[pack] 개인 프로필 각성 훅 병합: {} ({})",
                    settings.display(),
                    added.join(", ")
                );
                out.push((settings.display().to_string(), added));
            }
            Err(e) => eprintln!(
                "[pack] ⚠ 개인 프로필 훅 병합 건너뜀: {} — {e}",
                settings.display()
            ),
        }
    }
    out
}

/// SessionStart hook 등록 명령을 OS별로 조립하는 **공용 함수**(RC-2 · 순수 함수·회귀 핀).
/// Windows: 바닐라 셸(cmd/PowerShell)은 `.sh`를 인터프리터 없이 못 실행하고 "open with" 대화상자를
///   띄운다(anthropics/claude-code #21847·#24097). Claude Code가 Windows에서 찾는 인터프리터는
///   Git Bash의 `bash`이므로 `bash`로 명시 호출한다(맨 이름 `sh`는 Git Bash가 `bash.exe`만 보장 → 회피).
/// Unix: 기존과 동일 `sh <path>`(제로 회귀).
/// cys.rs::hook_command(init-pack 경로)와 setup_isolated_config_dir(격리 config dir 경로)가 **둘 다**
/// 이 함수를 써서 두 경로의 인터프리터가 일치한다(구: 격리 경로만 `sh` 하드코딩 → Windows 불일치).
/// ★W3(A9): 구현은 `hook_command_for` 단일 규약에 위임한다 — 종전엔 이 함수와
/// `role_bootstrap_hook_command` 가 같은 OS 분기를 **두 벌** 갖고 있었다(RC1 사본).
pub fn session_start_hook_command(pack_dir: &Path) -> String {
    // RC-2 잔여(T2.1·codex CONFIRMED): 공백 포함 경로(C:\Users\x y\.cys\pack\...) 대응 — Windows만
    // quote로 감싼다. unix는 **무변경**(기존 install에 등록된 미quote 문자열과 already-매칭 유지 —
    // quote 추가 시 불일치→매 기동 중복 append 회귀). 역슬래시→정슬래시 정규화(RC-3)도 그 함수 소관.
    hook_command_for(pack_dir, "session-start.sh")
}

/// UserPromptSubmit hook(role-bootstrap.sh) 등록 명령 — session_start_hook_command 와 동일 OS 규약
/// (unix `sh <abs>` / win `bash "<정슬래시>"`). ★javis_preflight._cys_hook_cmd("role-bootstrap.sh")
/// 와 **byte-identical** 이어야 한다 — 격리 config 초기 시드(이 함수)와 preflight C28 재등록이 같은
/// 문자열을 방출해야 중복 append 0(_prune_stale_hook_entries·_event_hook_registered matcher 일치·RC1).
pub fn role_bootstrap_hook_command(pack_dir: &Path) -> String {
    hook_command_for(pack_dir, "role-bootstrap.sh")
}

/// cys 전용 CLAUDE_CONFIG_DIR — 사용자 ~/.claude(외부 터미널 체계·구 지침 오염 가능)와 **격리**한다.
/// cys가 띄우는 claude는 이 디렉터리만 읽으므로, 사용자 프로필이 오염돼 있어도 영향받지 않고
/// 사용자 프로필을 건드리지도(읽지도·지우지도) 않는다. base 팩은 pack_dir 형제(~/.cys/claude).
///
/// ★정정(2026-08-23 실측 · 종전 서술 "macOS 인증은 계정 단위 Keychain이라 격리해도 로그인이
/// 유지된다" 는 반증됐다): macOS Keychain 항목은 **계정 단위가 아니라 config dir 경로 단위**로
/// 갈린다 — 서비스명이 `Claude Code-credentials-<sha256(CLAUDE_CONFIG_DIR 절대경로)[..8]>` 다.
/// 측정: 이 맥의 `security dump-keychain` 서비스명 9개를 열거해 경로 sha256 앞 8자리와 대조,
/// `~/.claude`=bdf68cc2 · `~/.cys/claude`=c45eaec5 · `~/.claude-3`=8e88d2ce 등 7개가 정확 일치.
/// 파급: ① 격리 config dir 는 **자체 `/login` 1회**가 필요하다(사용자 ~/.claude 의 로그인이
/// 따라오지 않는다) ② 프로필을 다른 경로로 복사·이동하면 인증이 따라가지 않는다 ③ 새 부서
/// dir(`~/.cys/claude-<key>`)마다 재발한다. 우회 수단 없음 — 사람 1회 로그인이 필요하다.
/// Windows 는 반대다: 토큰이 config dir 안의 `.credentials.json` **파일**이라 복사가 성립한다
/// (Windows 11 실기 확인 — `~/.cys/claude/.credentials.json` 509B · `cmdkey /list` 에 항목 없음).
///
/// ★G3 축1(부서 인식 · 확정 재설계): 부서 팩(`pack-dept-*`) 스코프에서는 공용 ~/.cys/claude 가
/// **아니라** 그 부서 claude 가 실제로 읽는 dir 를 표적한다 — 실소비 SOT 는
/// `${CYS_ACCOUNT_DIR:-~/.cys/claude}`(lib.rs `resolve_claude_config_dir`·agents.json 템플릿·
/// cys-dept 3자 일치)이며, 부서 스코프에서 CYS_ACCOUNT_DIR 이 없으면 **None**(시드 표적 없음)이다.
/// 레거시 폴백 dir(claude-dept-<name>)은 만들지 않는다 — 그 위치의 판독자는 생태계에 전무해
/// "아무도 안 읽는 dir에 쓰는" 사각 디렉터리가 된다(fail-closed·성찰 BLOCKER 확정).
pub fn config_dir() -> Option<PathBuf> {
    let pack = pack_dir();
    config_dir_for(
        crate::env_compat(ENV_CONFIG_DIR).as_deref(),
        dept_scope_of(&pack).as_deref(),
        std::env::var("CYS_ACCOUNT_DIR").ok().as_deref(),
        &pack,
    )
}

/// `config_dir` 의 순수부(env 주입형 — 전 OS 단위 테스트 가능). 우선순위는 계약이다:
///  ① CYS_CONFIG_DIR(호환 접두 포함) — 항상 최우선(기존 1순위 불변 핀)
///  ② 부서 스코프: CYS_ACCOUNT_DIR 비어있지 않으면 그 값 / 없으면 **None**(시드 생략 신호 —
///     호출부가 loud WARN + doctor anomaly 로 처리한다. 공용 claude 로 폴백하면 결함2 재발)
///  ③ base 스코프: pack 부모/"claude"(기존 거동 byte-identical — base 레인 무변경 보증)
pub fn config_dir_for(
    cfg_env: Option<&str>,
    dept_scope: Option<&str>,
    acct_env: Option<&str>,
    pack: &Path,
) -> Option<PathBuf> {
    if let Some(d) = cfg_env.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(d));
    }
    if dept_scope.is_some() {
        return acct_env.filter(|s| !s.is_empty()).map(PathBuf::from);
    }
    Some(
        pack.parent()
            .map(|p| p.join("claude"))
            .unwrap_or_else(|| PathBuf::from(".cys/claude")),
    )
}

/// ★★M5(2026-08-24 자기성찰 3회전) — **설치 표적 ≠ 실소비 SOT** 진단(순수).
///
/// ## 무엇이 어긋나는가
///
/// base 레인의 설치 표적은 [`config_dir_for`] ③ = `pack.parent()/claude` 로 **팩 위치에서
/// 파생**된다. 그런데 에이전트가 실제로 읽는 값은 `agents.json` 의
/// `${CYS_ACCOUNT_DIR:-$HOME/.cys/claude}` = [`crate::resolve_claude_config_dir`] 이고, 그쪽은
/// **팩 위치를 보지 않는다**. 두 값은 팩이 `~/.cys/pack` 일 때만 **우연히** 일치한다.
///
/// 그래서 팩이 다른 곳에 있으면(개발 트리 · 임시 팩 · 이주 중 · `CYS_PACK_DIR` 지정) 각성 훅은
/// 아무도 읽지 않는 폴더에 설치되고, 노드는 정상 기동하지만 `/clear` 후 지침 재주입(SessionStart)
/// 과 마스터 선언 부트 발화(UserPromptSubmit)가 **영구히 발동하지 않는다**(팀 미기동).
/// 회전2 격리 주행에서 "각성 훅 미등록" 경고가 났고, 처방된 `cys init-pack` 과
/// `javis_preflight.py --fix` 를 **완주시켰는데도 같은 경고가 재현**됐다 — 둘 다 어긋난 표적에
/// 쓰기 때문이다(BLOCK-2 와 같은 부류: "사고 순간에 사람이 읽는 유일한 문서가 듣지 않는 손잡이를
/// 안내한다").
///
/// ## 왜 여기서는 **진단만** 하는가
///
/// 표적 통일(설치처를 실소비 SOT 로 옮기는 것)은 부서 레인·마이그레이션·기존 설치본의 훅
/// 잔존까지 건드리는 부작용 반경이 크다. 태그 전에는 **어긋남을 보이게** 만들고(기동 시 loud
/// WARN + `cys doctor` 항목), 어긋난 상태에서 안내되는 명령이 듣지 않는다는 사실을 문안에
/// 정직하게 적는다.
///
/// ## 왜 검체가 스스로는 절대 못 잡는가
///
/// **기본 경로에서는 두 값이 우연히 일치하므로 검체가 영원히 초록**이다. 그래서 이 함수의
/// 진리표는 경로를 **주입**받아 어긋난 조합을 직접 만든다(실기 의존 0).
///
/// 반환: `None` = 일치(또는 판정 대상 아님) · `Some((설치표적, 실소비))` = 어긋남.
pub fn config_target_mismatch(
    install_target: Option<&Path>,
    consumed: &Path,
) -> Option<(PathBuf, PathBuf)> {
    let target = install_target?;
    // 경로 비교는 **정규화 후**에 한다 — `~/.cys/pack/../claude` 와 `~/.cys/claude` 가 다른
    // 문자열이라는 이유로 거짓 경보를 내면, 진짜 어긋남이 소음에 묻힌다.
    let norm = |p: &Path| -> PathBuf {
        p.canonicalize().unwrap_or_else(|_| lexical_normalize(p))
    };
    let (a, b) = (norm(target), norm(consumed));
    (a != b).then(|| (target.to_path_buf(), consumed.to_path_buf()))
}

/// `canonicalize` 가 실패할 때(아직 만들어지지 않은 dir — 정확히 이 결함의 상태다)의 폴백:
/// `.`/`..` 만 어휘적으로 접는다. 심볼릭 링크는 풀지 않는다(그건 canonicalize 의 몫이고,
/// 없는 경로에는 애초에 링크가 없다).
fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 격리 config dir 셋업: cys 라우터(CLAUDE.md)와 SessionStart hook(settings.json)을 설치한다.
/// ★보존 모드 — 기존 파일은 덮지 않는다(사용자 커스터마이즈 불가침). best-effort(실패해도
/// pack 설치 자체는 유효). 사용자 ~/.claude 는 절대 건드리지 않는다(격리의 핵심).
/// ★G3: `install_hooks=false` 면 라우터(CLAUDE.md — 훅이 아님) 시드는 유지하되 훅 병합·검증·
/// 개인 프로필 병합을 전부 생략한다(--no-install-hook 의미론 통일 — 모든 계급의 훅 등록 억제).
fn setup_isolated_config_dir(install_hooks: bool) {
    let Some(cfg) = config_dir() else {
        // ★G3 축1(확정 재설계): 부서 팩 스코프 + CYS_ACCOUNT_DIR 부재 = 실소비 SOT 없음 →
        //   **시드 생략**(fail-closed). 종전엔 config_dir 가 공용 ~/.cys/claude 로 접혀 부서 경로
        //   훅이 공용 프로필에 병합됐다(결함2 — 공용 프로필 무변조 기본 계약 위반). 레거시 폴백
        //   dir(claude-dept-<name>)은 판독자 전무라 만들지 않는다("아무도 안 읽는 dir에 쓰지
        //   않는다"). 정상 부서 부트(cys-dept launch/rotate/create/allocate)는 CYS_ACCOUNT_DIR 을
        //   주입하므로 이 분기는 이상 기동 신호다 — loud WARN + doctor 가시화(부서 레인=hook 진단
        //   부서 arm · base 레인=dept-awakening-seed anomaly — 포인터 정합: 리뷰 BLOCK-2).
        eprintln!(
            "[pack] ⚠ 부서 팩({}) 컨텍스트인데 CYS_ACCOUNT_DIR 미설정 — 격리 config 시드 생략(fail-closed). \
             부서 재기동(cys-dept launch/rotate)이 계정 dir 주입 후 재시드한다 · 진단: cys doctor(hook·dept-awakening-seed)",
            pack_dir().display()
        );
        return;
    };
    if std::fs::create_dir_all(&cfg).is_err() {
        return;
    }
    // 라우터: 임베드 CLAUDE.md.template → <cfg>/CLAUDE.md (없을 때만 — 역할선언→~/.cys/pack 라우팅)
    let claude_md = cfg.join("CLAUDE.md");
    if !claude_md.exists() {
        if let Some((_, tmpl)) = PACK_ALL.iter().find(|(rel, _)| *rel == "CLAUDE.md.template") {
            // ★(W2 · A8rs) 비원자 `fs::write` → `write_atomic`(pid-suffixed tmp + rename + fsync).
            // 비원자 쓰기는 부분 파일(torn write)을 남길 수 있고, 이 파일들은 **존재만으로 시드
            // 완료로 판정**되므로(`!exists()` 가드) 반쪽 파일이 영구히 굳는다 — 그 뒤 어떤 재실행도
            // 고치지 않는다(보존 모드가 덮지 않으므로). 원자 교체는 '옛 완본 또는 새 완본'만 남긴다.
            let _ = write_atomic(&claude_md, tmpl.as_bytes());
        }
    }
    // ★U-19 · 첫기동 관문 시드 — **훅 병합과 분리된 별도 단계**이며 `install_hooks` 조기 return
    //   **위**에 둔다. GUI 인앱 업데이트는 항상 `cys init-pack --no-install-hook` 으로 내려오므로
    //   (`src-tauri/src/main.rs` `maybe_apply_pending_update`) 아래에 두면 **업데이트로 올라온
    //   사용자 전원에게 영영 시드되지 않는다**(도달성 결함 — K-1 과 같은 계열).
    //   시드 자체는 자기 스위치(`CYS_FIRST_RUN_SEED`)로 제어되며 **기본은 꺼짐**이다.
    match seed_first_run_gates(&cfg, INSTALL_TIME_SEED_WORKSPACES, AuthPremise::Unproven) {
        SeedOutcome::Disabled | SeedOutcome::NothingToDo => {}
        other => eprintln!("[pack] 첫기동 관문 시드: {other:?}"),
    }
    if !install_hooks {
        // ★G3(--no-install-hook 일관성): 종전엔 이 플래그가 ~/.claude 대상만 막고 격리 config dir
        // 훅 병합(아래)은 그대로 돌았다 — 훅 억제를 요청한 운영자에게 훅이 몰래 등록되는 비일관.
        // 라우터는 훅이 아니므로 위에서 시드 유지, 훅 계열(병합·검증·개인 프로필)은 전부 생략.
        //
        // ★U-29(M-09-a · 2026-08-24): 그 억제가 **이미 등록된 우리 훅의 timeout 재조정까지**
        //   삼키면, GUI 인앱 업데이트로 올라온 기계는 **새(더 느린) 훅을 옛 상한 아래에서** 돌린다
        //   — U-21 이 고친 것보다 오히려 나빠지는 유일한 축이다. '설치'와 '재조정'은 다른 행위이며,
        //   아래 함수는 파일을 만들지도·엔트리를 추가하지도·개인 프로필을 만지지도 않는다
        //   (계약 전문과 실측 근거는 그 함수 문서에 있다). 개인 프로필은 종전대로 무접촉이다.
        let settings = cfg.join("settings.json");
        match retune_registered_hook_timeouts(&settings, &pack_dir(), &AWAKENING_HOOKS) {
            Ok(raised) if !raised.is_empty() => eprintln!(
                "[pack] 등록된 각성 훅 timeout 재조정: {} ({})",
                settings.display(),
                raised.join(", ")
            ),
            Ok(_) => {}
            Err(e) => eprintln!(
                "[pack] ⚠ 각성 훅 timeout 재조정 건너뜀: {} — {e}",
                settings.display()
            ),
        }
        println!(
            "[pack] 훅 미설치(--no-install-hook) — 격리 config·개인 프로필 훅 등록 생략\
             (등록된 우리 훅의 timeout 재조정만 수행 · 신규 설치 0)"
        );
        return;
    }
    // hook: <cfg>/settings.json 에 **소망 훅 집합(AWAKENING_HOOKS)** 을 이벤트 단위 멱등 병합.
    //
    // ★W3 A9: 종전엔 `if !settings.exists()` — **파일 단위 시드**였다. settings.json 이 이미 있는
    //   기계(사용자가 한 번이라도 Claude 설정을 만졌거나 구버전이 SessionStart 만 등록한 기계)에서는
    //   UserPromptSubmit(role-bootstrap) 이 **영구히 등록되지 않았다**. 그 훅이 preflight C28 의
    //   유일한 자동 트리거라 닭·달걀이 완성됐다(A9 재검증: v0.12.70 이전 설치 기계 전체가 모집단).
    //   이제 파일이 있어도 **없는 이벤트만 추가**한다 — 문자열이 byte-identical 이라 중복 0이고,
    //   사용자 항목은 병합기 계약상 불가침이다.
    let settings = cfg.join("settings.json");
    match merge_desired_hooks(&settings, &pack_dir(), &AWAKENING_HOOKS) {
        Ok(added) if !added.is_empty() => eprintln!(
            "[pack] 격리 config 각성 훅 병합: {} ({})",
            settings.display(),
            added.join(", ")
        ),
        Ok(_) => {}
        Err(e) => eprintln!("[pack] ⚠ 격리 config 훅 병합 건너뜀: {} — {e}", settings.display()),
    }
    // ★W3 게이트: 설치 직후 '등록 집합 ⊇ 소망 집합' 실측 검증(주장 아닌 파생 보고).
    let missing = verify_desired_hooks_registered(&settings, &pack_dir(), &AWAKENING_HOOKS);
    if !missing.is_empty() {
        eprintln!(
            "[pack] ⚠ 각성 훅 등록 미충족(등록 집합 ⊅ 소망 집합): {} — {} \
             (조치: `cys doctor --fix` 또는 preflight C28)",
            settings.display(),
            missing.join(", ")
        );
    }
    // ★T-0147-1: 개인 프로필(~/.claude*)의 레거시 각성 경로 — base 팩에서만·추가만(위 함수 계약).
    let _ = merge_awakening_hooks_into_personal_profiles();
}

// ═════════════════════════════════════════════════════════════════════════════
// U-19 · 첫기동 관문 시드 (C-4) — **훅 병합과 분리된 별도 단계**
// ═════════════════════════════════════════════════════════════════════════════
//
// ## ★V-h 실측 (2026-08-24 · macOS · Claude Code 2.1.241 · 격리 `CLAUDE_CONFIG_DIR` · PTY 관측)
//
// 신규 프로필로 `claude` 를 띄워 화면과 `.claude.json` 을 4조합으로 대조했다.
//
// | 시드한 것 | 첫 화면 | 디스크에 남은 키 |
// |---|---|---|
// | (없음) | **테마 선택 관문** | `theme` 없음 · `hasCompletedOnboarding` 없음 |
// | `theme:"dark"` | **테마 선택 관문**(변화 0) | ★`theme` 가 **지워졌다** |
// | `hasCompletedOnboarding:true` | **폴더신뢰 관문**(테마·로그인 관문이 사라짐) | 키 유지 |
// | 위 + `projects[<abs cwd>].hasTrustDialogAccepted:true` | **프롬프트**(관문 0) | 둘 다 유지 |
//
// 이 측정이 확정한 것 셋 —
//   ① **효능이 있는 키는 둘뿐이다**: 전역 `hasCompletedOnboarding` 과 워크스페이스별
//      `projects[<절대경로>].hasTrustDialogAccepted`. 관문 6종이 전부 사라진다.
//   ② **`theme` 시드는 무효다.** claude 가 기동 시 `.claude.json` 을 자기 스키마로 다시 쓰면서
//      모르는 키를 **버린다**(실측: 시드한 `theme` 가 소멸). 그래서 "관문마다 키를 하나씩"
//      같은 모델링은 이 버전에서 거짓이다 — 측정한 두 키만 쓴다.
//   ③ ★**`hasCompletedOnboarding` 은 로그인 관문까지 지운다.** 자격증명이 하나도 없는
//      프로필에서도 좌석이 프롬프트에 도달했고 **65초 뒤에도 살아 있었다**(상태줄
//      `Not logged in · Run /login`). 즉 이 키 하나가 **살아 있지만 아무 일도 못 하는 좌석**을
//      만든다 — `profile_gate.rs`(U-17) 가 경고한 **허위 READY 영구화**가 바로 이 형태다.
//
// ## 그래서 이 단위의 안전 계약 — 로그인 관문은 **인증 증명 없이는 지우지 않는다**
//
// [`AuthPremise::Unproven`] 이면 [`SEED_KEY_ONBOARDING`] 을 **넣지 않는다**. 폴더신뢰만 시드하는
// 것은 안전하다(그 창의 실측 기본 포커스가 이미 `Yes, I trust this folder` 이고, 대상 폴더는
// 우리가 고른 것이다). 반대로 로그인 관문을 지우는 것은 **오살 방향**이다 — 관문 앞에서 멈춘
// 좌석은 사람이 한 번 만지면 살아나지만, 관문이 지워진 미인증 좌석은 영원히 READY 로 보인다.
// 이것은 판정 완화가 아니라 **한 방향으로만 여는 축**을 하나 더 세운 것이다.
//
// ## 왜 `install_hooks` 조기 return **위**인가 (도달성)
//
// GUI 인앱 업데이트는 항상 `cys init-pack --no-install-hook` 으로 내려온다
// (`src-tauri/src/main.rs` `maybe_apply_pending_update`). 시드를 그 조기 return **아래**에 두면
// **업데이트로 올라온 사용자 전원에게 영영 시드되지 않는다** — `agents.json` 값 수정이 기존
// 기계에 닿지 않는 K-1 과 같은 계열의 도달성 결함이다. 그래서 시드는 훅과 **독립 플래그**로
// 제어하고 조기 return 위에 둔다.
//
// ## 롤백 스위치 — 마스터 하나 + 축 노브 하나
//
// | 스위치 | 값 | 되돌아가는 범위 |
// |---|---|---|
// | **`CYS_BOOT_GATES`** | `0` | 이 캠페인이 추가한 축 전부(이 시드 포함) |
// | `CYS_FIRST_RUN_SEED` | 미설정·`1` 이외 | **이 시드만 꺼짐 = 기본값** |
//
// ★극성이 형제 게이트(`기본 켜짐 · "0" 만 끔`)와 **반대**인 이유: 형제들은 *판정* 축이라 잊으면
// 안전장치가 켜져 있는 방향이지만, 이 축은 **사용자 파일에 쓰고 안전 화면을 지우는** 축이다.
// 잊어서 켜져 있으면 위 ③의 허위 READY 좌석이 생긴다. 두 경우 모두 규율은 같다 —
// **잊어도 오늘과 같은 방향**으로 배치한다. 그 방향이 여기서는 '꺼짐'이다.
// env 를 읽는 곳은 [`first_run_seed_enabled`] 하나뿐이고 판정은 순수
// [`first_run_seed_enabled_from`] 에 있다.
//
// ## 롤백 경로(파일)
// 쓰기 직전 원본을 옆자리 [`FIRST_RUN_SEED_BACKUP`] 로 복사한다. 되돌림은 그 파일을 제자리에
// 옮기는 것 하나뿐이다.

/// ★롤백 스위치의 env 이름(1지점). **기본은 꺼짐** — 정확히 `"1"` 만 켠다.
pub const ENV_FIRST_RUN_SEED: &str = "CYS_FIRST_RUN_SEED";

/// 이 시드 키 집합을 실측한 claude 바이너리 버전(V-h · 2026-08-24 · macOS).
/// 벤더가 관문을 늘리면 이 값과 실기 버전이 갈린다 — 그때 재측정이 필요하다는 표식이다.
pub const FIRST_RUN_SEED_MEASURED_ON: &str = "2.1.241";

/// claude 프로필 config 파일 이름(격리 config dir 안).
pub const CLAUDE_CONFIG_FILE: &str = ".claude.json";

/// 시드 직전 원본 보관 이름 — **롤백 경로**(제자리로 옮기면 종전 상태).
pub const FIRST_RUN_SEED_BACKUP: &str = ".claude.json.cys-seed.bak";

/// 전역 온보딩 완료 키. ★테마 관문과 **로그인 관문**을 동시에 지운다(V-h 실측 ③).
pub const SEED_KEY_ONBOARDING: &str = "hasCompletedOnboarding";

/// 워크스페이스별 봉투 키(절대경로 → 설정).
pub const SEED_KEY_PROJECTS: &str = "projects";

/// 워크스페이스별 폴더신뢰 키.
pub const SEED_KEY_TRUST: &str = "hasTrustDialogAccepted";

/// 설치 시점 호출부가 넘기는 워크스페이스 목록 — **비어 있다**.
///
/// 좌석의 cwd 는 `cys launch-agent` 가 호출 폴더로 정한다(`run_launch_agent_opts`) — 팩 설치
/// 시점에는 알 수 없고, 임의 폴더에 신뢰를 미리 박는 것은 이 저장소가 금지한 '임의 워크스페이스
/// 접촉'이다. 실 워크스페이스 시드는 **좌석을 만드는 쪽**이 자기가 아는 경로로 이 함수를
/// 부르는 것이 옳다(그 배선은 이 단위의 파일 반경 밖 — 인계 사항).
const INSTALL_TIME_SEED_WORKSPACES: &[String] = &[];

/// 이 프로필이 **인증되어 있음이 증명됐는가** — 로그인 관문을 지워도 되는지의 유일한 입력.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPremise {
    /// 인증 오라클(`claude auth status --json`)이 통과를 냈다 → 로그인 관문 제거 허용.
    Verified,
    /// 미측정·미인증 → **로그인 관문을 지우지 않는다**(허위 READY 영구화 차단).
    Unproven,
}

/// 시드 한 번의 결과. `Debug` 문자열이 그대로 운영 로그로 나간다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedOutcome {
    /// 스위치가 꺼져 있다(기본값) — 파일을 열지도 않았다.
    Disabled,
    /// 넣을 것이 없다(이미 전부 있거나, 대상 키가 하나도 성립하지 않는다).
    NothingToDo,
    /// **보존 방향 거부** — 파손·미지 형태·백업 실패. 원본을 건드리지 않았다.
    Refused(String),
    /// 썼다. 되읽기까지 확인된 추가분.
    Seeded(Vec<String>),
    /// 썼으나 되읽기에서 확인되지 않았다(동시 writer 경합 등) — 주장으로 덮지 않는다.
    Unverified(Vec<String>),
    /// 쓰기 자체가 실패했다.
    Failed(String),
}

/// 시드 스위치의 **순수 코어**. 마스터 접기값과 축 노브를 함께 받는다.
///
/// 마스터(`CYS_BOOT_GATES=0`)가 눌리면 축 노브가 `"1"` 이어도 꺼진다 — 사고 순간에 사람이
/// 노브를 조합할 수 없다는 규율(BLOCK-3)이 여기에도 그대로 적용된다.
pub fn first_run_seed_enabled_from(env_val: Option<&str>, master_legacy: bool) -> bool {
    !master_legacy && env_val == Some("1")
}

/// 위 판정의 **유일한 env 판독 지점**(부작용 있음).
pub fn first_run_seed_enabled() -> bool {
    first_run_seed_enabled_from(
        std::env::var(ENV_FIRST_RUN_SEED).ok().as_deref(),
        crate::gate_axes_forced_legacy(),
    )
}

// ── 한글 NFC/NFD (macOS 경로 정규화) ────────────────────────────────────────
//
// macOS 파일시스템은 경로를 **NFD**로 돌려주고 사람이 손으로 적은 config 는 **NFC**다. 같은
// 폴더가 두 형태로 갈리면 `projects` 봉투에 **중복 항목**이 생기고 신뢰 시드가 엉뚱한 쪽에
// 붙는다(= 관문이 그대로 남는다). 한글 음절의 조합·분해는 **표 없이 산술로** 정확히 계산되는
// 유일한 스크립트라(Unicode Hangul Syllable Composition Algorithm) 외부 크레이트 없이 정본을
// 구현한다. ★적용 범위는 한글로 한정한다 — 라틴 결합문자 등은 표가 필요하고, 그 경우 조회는
// **정확 일치로 폴백**한다(모르는 것을 아는 척 접지 않는다).

const HANGUL_S_BASE: u32 = 0xAC00;
const HANGUL_L_BASE: u32 = 0x1100;
const HANGUL_V_BASE: u32 = 0x1161;
const HANGUL_T_BASE: u32 = 0x11A7;
const HANGUL_L_COUNT: u32 = 19;
const HANGUL_V_COUNT: u32 = 21;
const HANGUL_T_COUNT: u32 = 28;
const HANGUL_N_COUNT: u32 = HANGUL_V_COUNT * HANGUL_T_COUNT;
const HANGUL_S_COUNT: u32 = HANGUL_L_COUNT * HANGUL_N_COUNT;

/// 조합형 한글 음절 → 자모 분해(NFD). 그 밖의 문자는 그대로 둔다.
pub fn hangul_nfd(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 2);
    for ch in s.chars() {
        let c = ch as u32;
        if (HANGUL_S_BASE..HANGUL_S_BASE + HANGUL_S_COUNT).contains(&c) {
            let si = c - HANGUL_S_BASE;
            let (l, v, t) = (
                HANGUL_L_BASE + si / HANGUL_N_COUNT,
                HANGUL_V_BASE + (si % HANGUL_N_COUNT) / HANGUL_T_COUNT,
                si % HANGUL_T_COUNT,
            );
            match (char::from_u32(l), char::from_u32(v)) {
                (Some(lc), Some(vc)) => {
                    out.push(lc);
                    out.push(vc);
                    if t != 0 {
                        if let Some(tc) = char::from_u32(HANGUL_T_BASE + t) {
                            out.push(tc);
                        }
                    }
                }
                // 산술상 도달 불가 — 도달하면 원문 보존(손실 금지).
                _ => out.push(ch),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// 자모 시퀀스 → 조합형 한글 음절(NFC). 그 밖의 문자는 그대로 둔다.
pub fn hangul_nfc(s: &str) -> String {
    let src: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < src.len() {
        let l = src[i] as u32;
        if (HANGUL_L_BASE..HANGUL_L_BASE + HANGUL_L_COUNT).contains(&l) && i + 1 < src.len() {
            let v = src[i + 1] as u32;
            if (HANGUL_V_BASE..HANGUL_V_BASE + HANGUL_V_COUNT).contains(&v) {
                let mut si = (l - HANGUL_L_BASE) * HANGUL_N_COUNT
                    + (v - HANGUL_V_BASE) * HANGUL_T_COUNT;
                let mut used = 2usize;
                if i + 2 < src.len() {
                    let t = src[i + 2] as u32;
                    if (HANGUL_T_BASE + 1..HANGUL_T_BASE + HANGUL_T_COUNT).contains(&t) {
                        si += t - HANGUL_T_BASE;
                        used = 3;
                    }
                }
                if let Some(sy) = char::from_u32(HANGUL_S_BASE + si) {
                    out.push(sy);
                    i += used;
                    continue;
                }
            }
        }
        out.push(src[i]);
        i += 1;
    }
    out
}

/// `projects` 봉투에서 이 워크스페이스가 **이미 어떤 형태로 들어 있는가**를 찾는다.
///
/// 규약(설계 U-19): 조회는 **양쪽 형태 모두**, 쓰기는 '이미 존재하는 형태가 있으면 그것,
/// 없으면 NFC'. 새 항목을 NFD 로 만들면 사람이 손으로 적은 config 와 영원히 갈린다.
fn resolve_project_key(projects: &serde_json::Map<String, serde_json::Value>, ws: &str) -> String {
    if projects.contains_key(ws) {
        return ws.to_string();
    }
    let folded = hangul_nfd(ws);
    for k in projects.keys() {
        if hangul_nfd(k) == folded {
            return k.clone();
        }
    }
    hangul_nfc(ws)
}

/// 시드 계획 — [`plan_first_run_seed`] 의 산출.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedPlan {
    /// 쓸 값(추가분이 없으면 입력과 같다).
    pub next: serde_json::Value,
    /// 추가되는 키 경로의 사람용 라벨.
    pub added: Vec<String>,
    /// 보존 방향 거부 사유(있으면 아무것도 쓰지 않는다).
    pub refused: Option<String>,
}

/// **순수 계획기** — IO 도 env 도 없다. 판정은 전부 여기 있고 아래 IO 함수는 배선만 한다.
///
/// 계약 넷:
///   ① **부재 키만 채운다.** 이미 있는 값은 `false` 여도 덮지 않는다(사용자 의사 불가침).
///   ② **다른 키는 전부 보존한다** — `oauthAccount` 포함(회귀 핀이 집행).
///   ③ **모르는 형태는 거부한다.** 최상위가 객체가 아니거나 `projects` 가 객체가 아니면
///      한 글자도 쓰지 않는다(파손 파일을 우리 스키마로 덮는 것이 더 위험하다).
///   ④ [`AuthPremise::Unproven`] 이면 [`SEED_KEY_ONBOARDING`] 을 넣지 않는다(V-h ③).
pub fn plan_first_run_seed(
    existing: &serde_json::Value,
    workspaces: &[String],
    premise: AuthPremise,
) -> SeedPlan {
    let refuse = |why: &str| SeedPlan {
        next: existing.clone(),
        added: Vec::new(),
        refused: Some(why.to_string()),
    };
    let Some(root) = existing.as_object() else {
        return refuse("최상위가 JSON 객체가 아니다 — 덮지 않는다");
    };
    let mut next = root.clone();
    let mut added: Vec<String> = Vec::new();

    // ④ 로그인 관문 제거는 인증 증명이 있을 때만.
    if premise == AuthPremise::Verified && !next.contains_key(SEED_KEY_ONBOARDING) {
        next.insert(SEED_KEY_ONBOARDING.to_string(), serde_json::Value::Bool(true));
        added.push(SEED_KEY_ONBOARDING.to_string());
    }

    if !workspaces.is_empty() {
        let mut projects = match next.get(SEED_KEY_PROJECTS) {
            None => serde_json::Map::new(),
            Some(serde_json::Value::Object(m)) => m.clone(),
            Some(_) => return refuse("`projects` 가 객체가 아니다 — 덮지 않는다"),
        };
        let mut touched = false;
        for ws in workspaces {
            if ws.is_empty() {
                continue;
            }
            let key = resolve_project_key(&projects, ws);
            let entry = projects
                .entry(key.clone())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            // 남이 만든 형태(객체 아님)는 보존하고 건너뛴다 — 관문이 남는 쪽이 안전하다.
            let Some(obj) = entry.as_object_mut() else {
                continue;
            };
            if !obj.contains_key(SEED_KEY_TRUST) {
                obj.insert(SEED_KEY_TRUST.to_string(), serde_json::Value::Bool(true));
                added.push(format!("{SEED_KEY_PROJECTS}[{key}].{SEED_KEY_TRUST}"));
                touched = true;
            }
        }
        // 추가분이 있을 때만 봉투를 얹는다(빈 `projects` 를 새로 만들어 쓰기를 유발하지 않는다).
        if touched {
            next.insert(
                SEED_KEY_PROJECTS.to_string(),
                serde_json::Value::Object(projects),
            );
        }
    }

    SeedPlan {
        next: serde_json::Value::Object(next),
        added,
        refused: None,
    }
}

/// 첫기동 관문 시드 — **스위치를 읽는 바깥 껍데기**(설치 경로가 부르는 것은 이쪽).
pub fn seed_first_run_gates(
    cfg: &Path,
    workspaces: &[String],
    premise: AuthPremise,
) -> SeedOutcome {
    if !first_run_seed_enabled() {
        return SeedOutcome::Disabled;
    }
    seed_first_run_gates_at(cfg, workspaces, premise)
}

/// 시드의 IO 부(스위치 무관 — 검체가 env 를 건드리지 않고 거동을 시험할 수 있게 분리).
///
/// 쓰는 파일은 **`<cfg>/.claude.json` 과 그 백업 하나뿐**이다. 개인 프로필(`~/.claude*`)·
/// 임의 워크스페이스는 이 함수의 어떤 경로에서도 열리지 않는다(검체가 단언).
pub fn seed_first_run_gates_at(
    cfg: &Path,
    workspaces: &[String],
    premise: AuthPremise,
) -> SeedOutcome {
    let path = cfg.join(CLAUDE_CONFIG_FILE);
    let existed = path.exists();
    let existing = if existed {
        match std::fs::read_to_string(&path) {
            Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => v,
                Err(e) => {
                    return SeedOutcome::Refused(format!("{} 파싱 불가 — 보존: {e}", path.display()))
                }
            },
            Err(e) => {
                return SeedOutcome::Refused(format!("{} 읽기 불가 — 보존: {e}", path.display()))
            }
        }
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    let plan = plan_first_run_seed(&existing, workspaces, premise);
    if let Some(why) = plan.refused {
        return SeedOutcome::Refused(why);
    }
    if plan.added.is_empty() {
        return SeedOutcome::NothingToDo;
    }

    // 롤백 경로 — 원본을 옆자리에 보관한다. 실패하면 **시드하지 않는다**(되돌릴 수 없는 쓰기 금지).
    // ★파일이 애초에 없었으면 백업도 없다 — 그 경우의 되돌림은 `.claude.json` 삭제 하나다
    //   (claude 가 다음 기동에 종전과 같은 신규 프로필을 다시 만든다). 백업 부재는 곧
    //   "시드 이전에는 이 파일이 존재하지 않았다" 의 기록이므로 정보가 사라지지 않는다.
    if existed {
        if let Err(e) = std::fs::copy(&path, cfg.join(FIRST_RUN_SEED_BACKUP)) {
            return SeedOutcome::Refused(format!("백업 실패 — 시드 보류: {e}"));
        }
    }

    let body = match serde_json::to_vec_pretty(&plan.next) {
        Ok(b) => b,
        Err(e) => return SeedOutcome::Failed(e.to_string()),
    };
    // ★권한 보존: `write_atomic` 은 tmp+rename 이라 원본 mode 가 사라진다(factory_reset.rs 의
    //   같은 관찰). `.claude.json` 은 실측 0600 이다 — 시드가 0644 로 넓히면 안 된다.
    let mode = existing_file_mode(&path).or(Some(0o600));
    if let Err(e) = write_atomic_mode(&path, &body, mode) {
        return SeedOutcome::Failed(format!("{}: {e}", path.display()));
    }

    // 사후 검증 — '썼다'는 주장이 아니라 **되읽기**로 확인한다. 같은 순수 계획기를 다시 돌려
    // "이제 추가할 것이 없다" 를 오라클로 쓴다(판정 사본 0).
    let verified = std::fs::read_to_string(&path)
        .ok()
        .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok())
        .map(|v| {
            let re = plan_first_run_seed(&v, workspaces, premise);
            re.refused.is_none() && re.added.is_empty()
        })
        .unwrap_or(false);
    if verified {
        SeedOutcome::Seeded(plan.added)
    } else {
        SeedOutcome::Unverified(plan.added)
    }
}

/// 기존 파일의 퍼미션 비트(unix). 파일이 없거나 unix 가 아니면 `None`.
#[cfg(unix)]
fn existing_file_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.mode() & 0o7777)
}

#[cfg(not(unix))]
fn existing_file_mode(_path: &Path) -> Option<u32> {
    None
}

/// 설치 매니페스트: rel → 설치 당시 내용의 sha256. "지금 디스크에 있는 파일이 우리가
/// 설치한 그대로인가(=사용자 비수정)"를 판정하는 유일한 근거다.
pub const INSTALL_MANIFEST: &str = ".install-manifest.json";
const PACK_VERSION_FILE: &str = ".pack-version";

// ─────────────────────────────────────────────────────────────────────────────
// free/pro 채널 상태 계약 (DESIGN-free-pro-distribution.md v6 §3·§5)
// ─────────────────────────────────────────────────────────────────────────────

/// 디스크 측 채널·튜플 SOT — pack_dir/.pack-state.json. `.pack-version`(최종 커밋 마커)과
/// 별개 파일이되, 트랜잭션 journal 편입 + 정합 검사(base_version ↔ .pack-version)로
/// 원자성을 보장한다(v4 — agy 병합안 변형 수용: 검증된 복구 기계 보존).
pub const PACK_STATE_FILE: &str = ".pack-state.json";

/// post-commit accepted 기록 실패 시 전용 경고 exit code(v5 §3 — EXIT_REINJECT_DEGRADED 동형:
/// 디스크 반영은 성공이라 롤백하지 않되 성공으로 침묵 포장하지 않는 구분 신호).
pub const EXIT_ACCEPTED_DEGRADED: i32 = 4;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PackState {
    /// "free" | "pro" 단 둘. 미지 값 = 손상 취급(보존 방향).
    #[serde(default = "crate::packsig::default_channel")]
    pub channel: String,
    /// 이 팩의 base semver — `.pack-version`과 일치해야 정상(불일치 = 손상 간주·§3 정합 검사).
    #[serde(default)]
    pub base_version: String,
    /// pro 채널 단조 증분(free = 0).
    #[serde(default)]
    pub pro_revision: u32,
}

/// 상태 판독 3상 — 부재(구 설치 = free/0 자연 마이그레이션) / 정상 / 손상(보존 방향).
#[derive(Debug)]
pub enum PackStateRead {
    Absent,
    Valid(PackState),
    /// 파싱 불가·미지 channel 값 — pro 간주(보존: 무음 파괴 차단이 최우선) + loud 진단 대상.
    Corrupt(String),
}

pub fn read_pack_state(dir: &Path) -> PackStateRead {
    let path = dir.join(PACK_STATE_FILE);
    let s = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return PackStateRead::Absent,
        Err(e) => return PackStateRead::Corrupt(format!("읽기 실패: {e}")),
    };
    match serde_json::from_str::<PackState>(&s) {
        Ok(st) if st.channel == "free" || st.channel == "pro" => PackStateRead::Valid(st),
        Ok(st) => PackStateRead::Corrupt(format!("미지 channel 값: {}", st.channel)),
        Err(e) => PackStateRead::Corrupt(format!("파싱 실패: {e}")),
    }
}

pub fn write_pack_state(dir: &Path, st: &PackState) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(st).map_err(|e| format!("state 직렬화 실패: {e}"))?;
    write_atomic(&dir.join(PACK_STATE_FILE), &json)
        .map_err(|e| format!("state 기록 실패: {e}"))
}

/// 무중단 채널 반영 판정의 튜플 확장(v6 §3): (base semver, pro_revision) 튜플로 strictly-newer.
/// fail-CLOSED — 어느 한쪽 base 파싱 실패 = false(반영 거부). free 경로는 rev=0 동치 무회귀.
pub fn remote_is_newer_tuple(remote: (&str, u32), disk: (&str, u32)) -> bool {
    match (parse_semver(remote.0), parse_semver(disk.0)) {
        (Some(r), Some(d)) => (r, remote.1) > (d, disk.1),
        _ => false,
    }
}

/// pro 전용 파일 실재 증거(v6 §5 음성 증거 검사의 ②축): install-manifest에 기록된 설치 파일 중
/// 임베드 트리(PACK_ALL)에 없는 것이 있으면 pro overlay 실재로 본다. (판독 실패 = 증거 없음 —
/// ①축 accepted.channel이 1차 권위라 이 축은 보조 휴리스틱이다.)
pub fn pro_file_evidence(dir: &Path) -> bool {
    let Ok(s) = std::fs::read_to_string(dir.join(INSTALL_MANIFEST)) else {
        return false;
    };
    let Ok(m) = serde_json::from_str::<std::collections::BTreeMap<String, String>>(&s) else {
        return false;
    };
    let embedded: std::collections::HashSet<&str> = PACK_ALL.iter().map(|(rel, _)| *rel).collect();
    m.keys().any(|k| !embedded.contains(k.as_str()))
}

/// 내장(비트랜잭션) install 경로의 채널 가드 + 제한적 자가치유(v6 §5).
/// 반환 Some(사유) = 내장 install 전체 생략(쓰기 0 + prune 0 — 보존). None = 진행.
fn channel_guard_and_heal(dir: &Path) -> Option<String> {
    match read_pack_state(dir) {
        PackStateRead::Absent => None, // 부재 = free/0 (구 설치 자연 마이그레이션)
        PackStateRead::Corrupt(e) => Some(format!(
            "PACK_CHANNEL_PRESERVED ⚠ .pack-state.json 손상({e}) → 보존 모드(pro 간주)·내장 팩 미반영. \
             복구: cys pack-repair-channel"
        )),
        PackStateRead::Valid(st) if st.channel == "pro" => Some(
            "PACK_CHANNEL_PRESERVED channel=pro — 내장 팩 미반영(pro 팩 보존). \
             free 복귀는 cys pack-downgrade-to-free 전용"
                .to_string(),
        ),
        PackStateRead::Valid(st) => {
            // channel=free — 정합 검사(state.base ↔ .pack-version).
            let disk_v = std::fs::read_to_string(dir.join(PACK_VERSION_FILE))
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            if st.base_version == disk_v {
                return None;
            }
            // 불일치 — 음성 pro 증거 검사(v6·R5 codex 결착: "정상 JSON이지만 거짓 free" 차단).
            // ①accepted.channel=pro ②pro 전용 파일 실재 → 자가치유 금지·보존+repair 유도.
            let accepted = dir
                .parent()
                .map(|p| p.join(".pack-accepted.json"))
                .unwrap_or_else(|| PathBuf::from(".pack-accepted.json"));
            match crate::packsig::read_accepted_evidence(&accepted) {
                Ok(Some((channel, _, _))) if channel == "pro" => {
                    return Some(
                        "PACK_CHANNEL_PRESERVED state=free이나 accepted 기록=pro(거짓 free 의심) \
                         → 보존·내장 팩 미반영. 복구: cys pack-repair-channel"
                            .to_string(),
                    )
                }
                Err(_) => {
                    // accepted 손상 = 증거 판독 불가 → fail-closed(보존).
                    return Some(
                        "PACK_CHANNEL_PRESERVED accepted 기록 손상 — 증거 판독 불가 → 보존. \
                         복구: cys pack-repair-channel"
                            .to_string(),
                    );
                }
                _ => {}
            }
            if pro_file_evidence(dir) {
                return Some(
                    "PACK_CHANNEL_PRESERVED state=free이나 pro 전용 파일 실재(거짓 free 의심) \
                     → 보존·내장 팩 미반영. 복구: cys pack-repair-channel"
                        .to_string(),
                );
            }
            // 증거 없음 → 제한적 자가치유: base_version만 동기화(loud) 후 진행.
            let mut healed = st;
            let old = std::mem::replace(&mut healed.base_version, disk_v);
            healed.pro_revision = 0;
            match write_pack_state(dir, &healed) {
                Ok(()) => eprintln!(
                    "[init-pack] state 자가치유: base {old:?} → {:?} (channel=free·pro 증거 없음)",
                    healed.base_version
                ),
                Err(e) => eprintln!("[init-pack] ⚠ state 자가치유 기록 실패(다음 기동 재시도): {e}"),
            }
            None
        }
    }
}

/// semver(major.minor.patch) 비교 — a > b. 'v' 접두·prerelease/build suffix('-rc','+build') 분리,
/// major 결측·비숫자는 파싱 실패로 본다. ★fail-CLOSED: 디스크 버전(a) 파싱 실패 시 보수적으로
/// true(=다운그레이드로 간주, 보존)를 반환해 사일런트 회귀를 막는다(0 폴백의 fail-OPEN 방지).
fn version_gt(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> Option<(u32, u32, u32)> {
        let mut it = v.trim().trim_start_matches('v').split('.').map(|p| {
            // prerelease/build suffix 분리: '10-rc' → '10', '0+build' → '0'
            p.split(|c| c == '-' || c == '+')
                .next()
                .unwrap_or("")
                .parse::<u32>()
                .ok()
        });
        let major = it.next().flatten()?; // major 결측·비숫자 → 파싱 실패
        Some((
            major,
            it.next().flatten().unwrap_or(0),
            it.next().flatten().unwrap_or(0),
        ))
    }
    match (parts(a), parts(b)) {
        (Some(pa), Some(pb)) => pa > pb,
        (None, _) => true,        // 디스크 버전 비정상 → 안전측(보존/차단)
        (Some(_), None) => false, // embed 비정상(env! 상수라 사실상 불가) → 차단 안 함
    }
}

fn content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

/// ★B1: 외부(cysd)가 디스크 폴백 phoenix 의 stale 여부(임베드 해시 대조)를 판정할 때 쓰는 공개 래퍼.
pub fn content_hash_pub(content: &str) -> String {
    content_hash(content)
}

/// ★B2(§2 축B 소유권 매니페스트): 팩 파일의 system|user 소유권 축. **기본값=system**(임베드 진실 —
/// 벤더 전진 시 갱신·병합, 벤더 미전진 드리프트는 kept-drift 제자리 보존 · v2 §3 L0-L4),
/// **user 는 화이트리스트만**(사용자 수정 보존). 화이트리스트를 좁게 유지해
/// '조용한 탈락'(system 인데 user 로 오분류돼 스큐가 동결)을 방지한다.
///   user(preserve)  = 디렉티브(*_DIRECTIVE.md)·헌법(soul.md)·CLAUDE.md — CEO/사용자 커스텀 대상.
///   system(update)  = bin/*.py·hooks/*·skills·schemas·templates 등 그 외 전부(cysd 소유·스큐 금지).
/// P0-4 수리: 과거 `_ =>` catch-all 이 매니페스트 부재·읽기 실패까지 'user 수정'으로 오판해 phoenix(system)를
/// 영구 동결시켜 배포 스큐를 냈다 — 이 분류가 그 근원을 대체한다(CLAUDE.md.template 은 .template 이라 system).
/// ★G3 축2 이후: 런타임 소비처는 전부 스코프 인지판(ownership_scoped != System 등)으로 이동했고,
/// 이 술어 래퍼 2종은 Base 등급표 회귀 핀(테스트)의 소비 대상으로 남는다 — 삭제 금지(핀 약화).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn is_user_owned(rel: &str) -> bool {
    ownership(rel) == Ownership::User
}

/// ★B2-2(치유 원복 사고 시정 · 2026-07-12): **seed-once 상태 파일** — 팩이 부재 시에만 시드로
/// 설치하고, 존재하면 force 여도 불가침. memory/(장기기억 색인·시드 본문)·round/SESSION_STATE.md
/// (복원 단일 진실)·round/RECOVERY.md 는 설치 후 노드가 계속 갱신하는 런타임 상태라 임베드와 항상
/// 달라지는데, system 등급이면 init-pack 전량 스윕(부트 ⓪ preflight --fix 가 결손 1건에도 호출)마다
/// vendor 골격으로 원복돼 기억·상태가 주기 소실된다(실측: 로컬 원장 healed 0.12.46~47 3건 + 배포
/// 사용자 기계 동일 사고 4회차). round/ 의 정적 계약(TOOL_RESULT_VOCAB·catalog·video-archetypes)은
/// 상태가 아니므로 system 유지 — 상태 파일만 좁게 열거한다. 의도적 초기화 = 파일 삭제 후 init-pack.
#[cfg_attr(not(test), allow(dead_code))] // 위 is_user_owned 와 동일 사유(Base 등급표 핀 소비).
pub(crate) fn is_seed_once(rel: &str) -> bool {
    ownership(rel) == Ownership::SeedOnce
}

/// 팩 파일 소유권 3등급 — 분류 SOT는 아래 `ownership()` 단일 함수다(성찰 후속 2026-07-12).
/// 화이트리스트가 술어 2개(is_user_owned/is_seed_once)로 분산되면 교집합(이중 등급) 시 동작이
/// 호출 순서에 숨는다 — 단일 match 가 배타 등급 하나만 반환하게 구조로 보장한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ownership {
    /// 임베드 진실(기본값) — 벤더 전진 불일치 시 치유/3-way 병합(P0-4 계승) · 벤더 미전진
    /// 드리프트는 kept-drift 보존 · 비수정 자동 갱신·prune 대상.
    System,
    /// 사용자 주권(디렉티브·soul·CLAUDE·schedule) — 영구 보존, 임베드 신버전은 `.new` 병치.
    User,
    /// 런타임 상태(memory/·round 상태) — 부재 시에만 시드, 존재하면 force 여도 불가침.
    SeedOnce,
}

/// 헌법 파일(디렉티브·soul·CLAUDE) 판정 — ownership() User 등급의 부분집합이며, 병합 시
/// 추가 안전 게이트(안전핵 존재 검증·자동 병합 금지)가 붙는 대상. ownership() 이 이 함수를
/// 호출해 패턴을 한 곳에 유지한다(SOT 분산 금지 — 술어 조건 직접 중복 금지 규칙과 동일 취지).
pub fn is_constitution_file(rel: &str) -> bool {
    rel.ends_with("_DIRECTIVE.md")
        || rel == "soul.md"
        || rel.ends_with("/soul.md")
        || rel == "CLAUDE.md"
        || rel.ends_with("/CLAUDE.md")
}

/// 소유권 분류 단일 SOT. 우선순위 **SeedOnce > User > System** — 상태 경로 밑에 user 패턴
/// 이름이 오는 가상 케이스(예: memory/CLAUDE.md)에서 '상태 보존(불가침)'이 '병합 병치(.new)'보다
/// 안전측이며, decide_file_action 의 기존 검사 순서(seed-once 조기 반환)와 동형이다.
pub(crate) fn ownership(rel: &str) -> Ownership {
    if rel.starts_with("memory/")
        || rel == "round/SESSION_STATE.md"
        || rel == "round/RECOVERY.md"
    {
        return Ownership::SeedOnce;
    }
    if is_constitution_file(rel)
        // ★B2-1(W3): schedule.json 은 사용자가 `cys schedule add` 로 편집하는 혼합 파일 — 팩 강제갱신이 덮으면
        // 사용자 잡이 소실(비가역 데이터 손실)된다. user 소유로 보존하고, built-in 잡(phoenix-*)은 데몬 부트 시
        // 코드가 idempotent ensure 한다(cysd schedule::ensure_builtin_jobs). 기본 잡 드리프트(복구 가능) < 사용자 잡 소실.
        || rel == "schedule.json"
        // ★W-B(커스텀 생존 설계 2026-07-17): agents.json 은 파일 자신의 _doc 이 "사용자 환경에 맞게
        // 수정 가능"이라 선언하는 혼합 설정인데 system 등급이라 매 스윕 치유돼 사용자 어댑터 수정이
        // 소실됐다(실측: 병합 원장 healed 0.12.64). schedule.json 전례를 따라 user 승격 — vendor 신
        // 어댑터는 .new 병치+병합으로 전달되고, 소비자(launch-agent·cysd)는 어댑터 결손 시 해당
        // 어댑터만 비활성(전체 파손 아님)이라 동결 리스크 < 사용자 수정 소실.
        || rel == "agents.json"
        // ★W-ACL(오너 승인 2026-08-01): acl.json 은 "이 설치본에서 누가 누구에게 stdin 을 넣을 수
        // 있는가"를 정하는 **설치별 운영 정책**이다(부서 편성·오너가 확정한 송신 규칙). 성격이 같은
        // schedule.json·agents.json 은 이미 user 로 보존되는데 acl.json 만 system 이라, 매 설치
        // 스윕이 vendor 기본 정책으로 강제 치유해 확정 정책이 되돌아갔다(2026-08-01 실증).
        // 전례 그대로 user 승격 — vendor 신규 규칙은 `.new` 병치+pack-merge 로 전달되고, 데몬의
        // ACL 평가는 파일 부재·파싱 실패 시 fail-OPEN(cysd check_send_acl)이라 동결 리스크 <
        // 정책 원복. ★is_constitution_file 에는 넣지 않는다 — 헌법 문서가 아니라 정책 파일이라
        // pack-merge 대화형 강제(--yes 무시·안전핵 검증)가 아니라 일반 user-owned 병합 경로를 탄다.
        || rel == "acl.json"
    {
        return Ownership::User;
    }
    Ownership::System
}

/// 잠금 보안 자산(v2 §7 — enum 분할 기각: 암묵 else 침묵 미구현 위험 R7). 디스크 사본은 현행
/// 런타임 비소비(서명 키링은 컴파일타임 임베드 — packsig.rs embedded_keyring)이나 ①팩 배포 신뢰
/// 자산의 드리프트는 정당한 사용자 확장 사례 0(수정=변조 신호) ②미래 키 회전·오신뢰 표면 차단 —
/// 방어적 무결성 계약(v2 §2 원칙 4의 예외).
pub(crate) const SYSTEM_LOCKED: &[&str] = &["trusted-keys.json"];
pub(crate) fn system_locked(rel: &str) -> bool {
    SYSTEM_LOCKED.contains(&rel)
}

/// 소유권 분류의 CLI 노출용 이름(외부 crate 인 bin/cys.rs 는 pub(crate) ownership() 을 못 본다).
/// pack-guard hook·pack-ownership 서브커맨드가 소비 — 분류 자체는 위 단일 SOT 를 그대로 통과.
pub fn ownership_name(rel: &str) -> &'static str {
    match ownership(rel) {
        Ownership::System => "system",
        Ownership::User => "user",
        Ownership::SeedOnce => "seed-once",
    }
}

/// ★G3 축2(2026-08-21 확정): 팩 스코프 — 같은 rel 이라도 base 팩과 부서 팩(`pack-dept-*`)에서
/// 소유권 등급이 다를 수 있다(현행 차등은 soul.md 하나). 스코프는 **데이터**로 주입한다 —
/// decide_file_action 의 순수성(부수효과 0·env 무참조)을 유지하고, install_staged 의 staging
/// (`.pack-staging-init-*` — basename 에 부서 정보가 없다)에서도 논리 대상의 스코프가 흐르게 한다.
/// pub 인 이유: install_into(pub) 시그니처에 흐르는 타입은 private 일 수 없다(E0446).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackScope {
    /// 공용 base 팩(~/.cys/pack) — 기존 등급표 그대로(byte-identical 거동 보증).
    Base,
    /// 부서 팩(pack-dept-*) — soul.md 만 SeedOnce 로 승격(base 헌장 승계 후 불가침).
    Dept,
}

/// 디렉터리 → 스코프. 부서 판정 규칙의 등재소는 `dept_scope_of` 한 곳이다(이 함수는 래퍼 —
/// RC1 사본 드리프트 금지). 빈 부서명(`pack-dept-`)은 dept_scope_of 가 None 이라 Base 취급.
pub fn pack_scope_of(dir: &Path) -> PackScope {
    if dept_scope_of(dir).is_some() {
        PackScope::Dept
    } else {
        PackScope::Base
    }
}

/// 스코프 인지 소유권 — Base 는 기존 `ownership()` 그대로(`ownership(rel) ==
/// ownership_scoped(rel, Base)` 항등이 계약이며 ownership_scoped_matrix 가 PACK_ALL 전량으로
/// 봉인한다). Dept 는 soul.md(및 */soul.md)만 SeedOnce 로 승격 — 부서 soul 은 최초 1회 base
/// 헌장을 승계해 시드되고(설치 코어 seed-from-base), 존재하면 force 여도 불가침이며, vendor
/// 전진 시 `.new` 병치 노이즈가 구조적으로 소멸한다(결함4 해소 — decide_file_action 의
/// seed-once 조기 반환이 병치 판정보다 먼저다).
pub(crate) fn ownership_scoped(rel: &str, scope: PackScope) -> Ownership {
    if scope == PackScope::Dept && (rel == "soul.md" || rel.ends_with("/soul.md")) {
        return Ownership::SeedOnce;
    }
    ownership(rel)
}

/// `ownership_name` 의 스코프 인지판 — CLI(pack-ownership·pack-rollback 가드)가 팩 경로로
/// 스코프를 산출해 소비한다. 출력 어휘는 기존 3종({system,user,seed-once}) 그대로다 — dept
/// 스코프에서 soul.md 가 "seed-once" 로 나오는 것은 정당한 신규 값이며, pack-guard.sh 는
/// `= "system"` 정확 비교만 하므로 user→seed-once 전이는 훅 거동 무변(하위호환 계약).
pub fn ownership_name_scoped(rel: &str, pack: &Path) -> &'static str {
    match ownership_scoped(rel, pack_scope_of(pack)) {
        Ownership::System => "system",
        Ownership::User => "user",
        Ownership::SeedOnce => "seed-once",
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ★사용자 커스터마이즈 절충 계층 (2026-07-07 오너 승인 6층 로드맵의 ②③④ 코어)
//   문제: system 파일은 매 install 강제 치유(P0-4)로 사용자 수정이 소실, user-owned 는
//   보존되지만 영구 동결(병합 경로 없음) — 업데이트가 커스텀을 무효화한다는 사용자 항의의 실체.
//   절충(dpkg conffile/rpmnew·rpmsave 패턴):
//     - user-owned 수정본 + 임베드 신버전 변경 → 보존 유지 + `<rel>.new` 병치(병합 대기)
//     - system 수정본: 벤더 미전진=제자리 보존(kept-drift) · 벤더 전진=검증 base 3-way(실패
//       전부 치유 폴백) · 치유 시 덮어쓰기 전에 <rel>.user 보존(파괴 0)
//     - `.pristine/<rel>` = 마지막으로 디스크에 적용된 vendor 원본(3-way 병합의 공통 조상)
//     - `.merge-pending.json` = 병합 대기 원장 — `cys pack-merge` 가 소비
//   판정은 decide_file_action(순수)로 추출해 install_into(쓰기)와 plan_install(드라이런)이
//   같은 논리를 공유한다(플랜≠실제 드리프트 차단).

/// 병합 대기 원장 파일명 (pack_dir 루트 · install-manifest 형제 · 매니페스트 비등재라 prune 불가침).
pub const MERGE_PENDING_FILE: &str = ".merge-pending.json";
/// ★G3-축3 병합 감사 원장(append-only) — take-new/keep-mine/force 해소·거부의 사후 추적.
/// 매니페스트 비등재 dotfile 이라 prune 불가침이며, apply_pack_transactional backup_set 에도
/// **의도적 비등재**다: 감사는 rollback 을 생존해야 한다(MERGE_PENDING_FILE 은 상태라 저널
/// 편입이 맞지만, 원장은 되감기면 거부·강제 라인이 소거돼 원장이 아니게 된다).
pub const MERGE_AUDIT_FILE: &str = ".merge-audit.jsonl";
/// pristine 미러 디렉터리 — 마지막 적용 vendor 원본(3-way base). 매니페스트 비등재.
pub const PRISTINE_DIR: &str = ".pristine";

/// 사용자 로컬 오버레이 루트(⑤①) — 업데이터·치유·prune 이 **존재 자체를 모르는** 사용자 전용 영역.
/// directives/*_DIRECTIVE.local.md(디렉티브 append)·skills/(동명 shadowing)·hooks/<event>.d/(후행 실행)·notes/.
/// 테스트 오버라이드: CYS_LOCAL_DIR. 기본 = pack_dir 형제 `local`(~/.cys/local).
pub fn local_dir() -> PathBuf {
    if let Some(d) = crate::env_compat("CYS_LOCAL_DIR") {
        return PathBuf::from(d);
    }
    let pd = pack_dir();
    match pd.parent() {
        Some(parent) => parent.join("local"),
        None => PathBuf::from("local"),
    }
}

/// 파일 1건의 설치 판정(순수·부수효과 0) — install_into 와 plan_install 공용.
/// ★T4 CLI 동사 소비(pack-adopt 판정 시뮬 결과 타입) — 재구현 금지.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAction {
    /// 임베드 내용을 디스크에 기록. heal_user_copy=true 면 사용자 수정본을 `<rel>.user` 로 먼저 보존
    /// — 판독 가능이면 `<rel>.user` · 판독 불가(L0·L1)는 바이트 백업(실행부 T3 — T2 인터림은 현행
    /// 실행부 그대로 백업 미실행).
    Write { heal_user_copy: bool },
    /// 디스크 유지. adopt_hash=true 면 매니페스트에 현재 임베드 해시 채택(구설치본 승계).
    /// new_pending=true 면 임베드 신버전을 `<rel>.new` 로 병치(병합 대기 — user-owned 동결 해소 경로).
    Keep { adopt_hash: bool, new_pending: bool },
    /// ★NEW(v2 §3 L2): 벤더 미전진 + system 드리프트 — 제자리 보존. 원장 kept-drift 계상·pristine
    /// 백필 가드는 T3.
    KeepDrift,
    /// ★NEW(v2 §3 L3): 벤더 전진 + 드리프트 + 검증된 base — 3-way 병합 시도(실행부 T3 · 실패 전부
    /// healed 폴백).
    Merge3,
}

/// 순수 판정 SOT — User·SeedOnce 조기 분기 불변, system은 L0(locked 즉시 치유)/L1(판독불가 백업 후
/// 치유)/L2(벤더 미전진 kept-drift)/L3(벤더 전진 검증 base 3-way)/L4(레거시 종전 거동) 판정표(v2 §3).
/// ★T4 CLI 동사 소비(pack-adopt 스윕 생존 판정 시뮬) — 재구현 금지.
pub fn decide_file_action(
    rel: &str,
    embed: &str,
    exists: bool,
    disk: Option<&str>, // None = 부재 또는 읽기 실패(비UTF-8 등)
    manifest_hash: Option<&str>,
    force: bool,
    scope: PackScope,
    // 호출자가 content_hash(pristine)==manifest_hash 검증을 마친 base(v2 §2 원칙 3). 미검증·부재·
    // manifest 부재면 None — IO·해시 검증은 호출자 몫(순수성 유지·scope 주입과 동형). T2 호출자
    // 전원 None(L3 도달 불가·배선은 T3).
    verified_base: Option<&str>,
) -> FileAction {
    // ★B2-2 seed-once 상태: 존재하면 불가침(force 여도·읽기 실패여도) — 부재 시에만 아래 시드 설치.
    // (dept 스코프의 soul.md 도 이 조기 반환을 탄다 — 병치 판정보다 먼저라 결함4 가 구조 소멸.)
    if exists && ownership_scoped(rel, scope) == Ownership::SeedOnce {
        return FileAction::Keep { adopt_hash: false, new_pending: false };
    }
    // ★B2 user-owned 영구 보존 (force 여도) — 읽기 성공 + 내용 상이일 때.
    if exists && ownership_scoped(rel, scope) == Ownership::User {
        if let Some(d) = disk {
            if d != embed {
                // 임베드가 마지막 적용본(매니페스트 해시)에서 전진했으면 신버전 병치(병합 대기).
                // 매니페스트 부재(구설치본)도 안전측으로 병치해 가시화한다(base 없는 2-way 병합).
                let new_pending = manifest_hash != Some(content_hash(embed).as_str());
                return FileAction::Keep { adopt_hash: false, new_pending };
            }
        } else {
            // ★W-1 수리(P0 · 2026-08-01): **판독 불가(disk=None)면 force 여도 무조건 보존.**
            // read_to_string 실패(ko-KR Windows 의 CP949/ANSI 저장·권한·잠금)는 "사용자 수정이
            // 없다"는 증거가 아니라 **비교 자체가 불가능하다**는 뜻이다. 종전엔 위 Some(d) 가드와
            // 아래 `exists && !force` 가드를 둘 다 통과해 최종 폴백의 Write{heal_user_copy:false}로
            // 떨어졌다 — 헌법 파일(*_DIRECTIVE.md·soul.md·CLAUDE.md·schedule.json·agents.json)이
            // **무경고·무백업**으로 임베드에 덮여 비가역 소실된다(발화 경로: preflight C03 가 CP949
            // 파일의 한국어 핀을 전부 '소실'로 오판 → 화면이 `cys init-pack --force` 를 권고).
            // ★백업 후 교체가 아니라 Keep 이 기본인 이유: 백업 실행부(install_into 의 heal_user_copy)
            // 자체가 `disk.as_deref()` 의 Some 에 갇혀 있어 disk=None 에선 어떤 백업도 뜨지 못한다
            // — 플래그만 켜는 수리는 실효 0(측정 완료). 읽을 수 없는 사용자 파일은 더더욱 덮으면
            // 안 된다. system 등급은 이 분기 밖 — L0/L1이 치유+백업 의무로 처리(v2 §3).
            return FileAction::Keep { adopt_hash: false, new_pending: false };
        }
    }
    if exists && !force {
        match disk {
            Some(d) if d == embed => {
                return FileAction::Keep { adopt_hash: true, new_pending: false };
            }
            Some(d) if manifest_hash == Some(content_hash(d).as_str()) => {
                // 설치-당시 해시 그대로(사용자 비수정) + 임베드가 더 새 버전 → 갱신.
                return FileAction::Write { heal_user_copy: false };
            }
            _ => {
                // 사용자 수정본·매니페스트 부재·읽기 실패.
                if ownership_scoped(rel, scope) == Ownership::User {
                    // ★W-1 이후 이 가지는 도달 불가다(읽기 실패는 위 else 가 force 무관하게 먼저
                    // 잡고, 내용 상이는 첫 블록이 잡는다). 다중 방어로 남겨둔다 — 위 분기가 훗날
                    // 리팩터링으로 흔들려도 force=false 경로의 보존은 여기서 한 번 더 성립한다.
                    return FileAction::Keep { adopt_hash: false, new_pending: false };
                }
                // ★L0(v2 §3·§7): 잠금 보안 자산 — 전 클래스 즉시 치유(벤더 미전진·판독 불가 포함).
                //   disk=Some(d)면 이 arm 도달 조건상 d != embed ∧ manifest != hash(d)라 현행 heal 식과 동치(항상 true).
                //   disk=None은 L0 하위 케이스 명문(v2 §3) — 바이트 백업 의무 신호(실행부 T3 · T2 런타임 쓰기는 현행 동형).
                if system_locked(rel) {
                    return FileAction::Write { heal_user_copy: true };
                }
                // ★L1(v2 §3): 판독 불가(비UTF-8·권한·잠금) — 치유 유지 + 백업 의무 신호(현행 heal:false→true 전환).
                if disk.is_none() {
                    return FileAction::Write { heal_user_copy: true };
                }
                // ★L2(v2 §3 — v1 Patch 1 흡수): 벤더 미전진 + 드리프트 → 제자리 보존(kept-drift).
                //   이 arm 도달 조건상 disk=Some(d) ∧ d != embed (arm1·fast-path 선점) — matches! 가드는 리팩터링 내성용.
                let vendor_advanced = manifest_hash != Some(content_hash(embed).as_str());
                if !vendor_advanced && matches!(disk, Some(d) if d != embed) {
                    return FileAction::KeepDrift;
                }
                // ★L3(v2 §3): 벤더 전진 + 드리프트 + 검증된 base → 3-way(실행부 T3 — T2는 호출자 verified_base=None 고정이라 도달 불가).
                //   manifest_hash.is_some() 방어 conjunct(master 결정): base 검증 계약이 manifest 부재에선 불성립 — 계약 위반 입력은 L4로.
                if manifest_hash.is_some() && vendor_advanced && verified_base.is_some() {
                    return FileAction::Merge3;
                }
                // L4: 레거시(manifest 부재)·base 미검증 — 종전 P0-4 거동(치유 + .user 보존).
                let heal = matches!(disk, Some(d) if d != embed
                    && manifest_hash != Some(content_hash(d).as_str()));
                return FileAction::Write { heal_user_copy: heal };
            }
        }
    }
    // 신규 생성 또는 force 갱신 — force 로 수정본을 덮을 때도 사용자본은 보존한다(파괴 0).
    // ★v2 §3(이식 ⑤): 판독 불가(disk=None)도 백업 의무 신호(heal:true) — 이 지점 도달 ∧ disk=None ⇒ system 전용
    //   (User disk=None은 W-1 조기 분기가, SeedOnce는 조기 반환이 force 무관 선행). !exists는 exists&& 가드로 heal=false 불변.
    //   T2 런타임 FS 파급 0: 백업 실행부가 Some(d) 게이트(install_into)라 disk=None에선 플래그 무력 — w1 bytewise 핀 무수정 초록이 기계 증거.
    let heal = exists
        && (disk.is_none()
            || matches!(disk, Some(d) if d != embed
                && manifest_hash != Some(content_hash(d).as_str())));
    FileAction::Write { heal_user_copy: heal }
}

/// 병합 대기 원장 로드(부재·손상 = 빈 원장, 기동 차단 0).
pub fn load_merge_pending(dir: &Path) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(dir.join(MERGE_PENDING_FILE))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// 병합 대기 원장 저장(best-effort — 자문 메타데이터라 실패가 설치를 막지 않는다).
pub fn save_merge_pending(dir: &Path, pending: &serde_json::Map<String, serde_json::Value>) {
    if let Ok(json) = serde_json::to_string_pretty(&serde_json::Value::Object(pending.clone())) {
        let _ = write_atomic(&dir.join(MERGE_PENDING_FILE), json.as_bytes());
    }
}

/// ★G3-축3 감사 원장 1줄 append. 다중 프로세스 appender 전제라 O_APPEND + **라인당 단일
/// write_all** 규율 — 커널 append 원자성으로 라인 교차를 막는다(프로세스 내 락으로는 부족).
/// serde 직렬화가 문자열 내 개행을 이스케이프하므로 엔트리 1건 = 물리 1줄이 보장된다.
/// write_atomic(임시파일 rename)을 쓰지 않는 이유: 교체 쓰기는 동시 appender 의 라인을
/// 유실시킨다 — append-only 원장은 append 로만 자란다.
pub fn append_merge_audit(dir: &Path, entry: &serde_json::Value) -> Result<(), String> {
    use std::io::Write;
    let mut line =
        serde_json::to_string(entry).map_err(|e| format!("감사 엔트리 직렬화 실패: {e}"))?;
    line.push('\n');
    let path = dir.join(MERGE_AUDIT_FILE);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("감사 원장 열기 실패 {}: {e}", path.display()))?;
    f.write_all(line.as_bytes())
        .map_err(|e| format!("감사 원장 쓰기 실패 {}: {e}", path.display()))
}

/// ★T3(D7 · v2 §4 이식 ①): 계보 kind — merged·conflicted 원장 항목은 계보 포인터(capture·
/// base_side)를 갖는 영속 kind 다. 무규칙 upsert 는 rel 당 1항목 계약(upsert_pending)상 이
/// 포인터를 지운다 — 아래 upsert_pending_v2 가 유일한 보존 경로다.
pub(crate) const LINEAGE_KINDS: [&str; 2] = ["merged", "conflicted"];

/// ★성찰 차단 수리(계상 SOT 3분산): 병합 대기 원장(.merge-pending.json) kind 문자열 전체
/// 목록의 **유일 등재소**. 종전에는 같은 목록이 ①pending_kind_counts ②doctor pack-drift
/// count_kind 클로저 ③pack-plan n_kind 클로저 + 파이썬 javis_preflight C62/C68 에 자구
/// 재구현으로 4분산돼, 신 kind 1종(실례: T4 adopted — 부트 요약 무계상·doctor 만 계상·C68
/// 분류 누락)마다 동시 수정을 사람이 기억해야 했다. 이제 ②③은 pending_kind_counts 를 위임
/// 소비하고(계상기 단일화 = 상호 일치가 구조로 박제), 파이썬 미러(MERGE_LEDGER_KINDS·
/// C68_EXEMPT_KINDS)는 test_todo_shared_constants 의 census 핀이 이 상수와 대조한다.
///
/// **신 kind 추가 절차(기계가 강제)**: 여기 등재 → pending_kind_counts 버킷 추가(census 핀
/// `ledger_kinds_census_bijective_with_counter` 의 전필드 struct 리터럴이 컴파일로 강제) →
/// javis_preflight MERGE_LEDGER_KINDS 등재 + C62/C68 분류 결정(2언어 census 핀 + 행동 census
/// `test_kind_census_c62_c68_classification` 이 강제). 주의: 감사 원장(.merge-audit)의
/// "pre-merge-capture" 는 다른 파일의 다른 계약이다 — 여기 등재 대상이 아니다.
pub const LEDGER_KINDS: [&str; 7] =
    ["healed", "new-pending", "kept-drift", "merged", "conflicted", "quarantined", "adopted"];

/// ★T3(D7): 계보 인지 원장 upsert — 기존 upsert_pending 클로저는 자구 불변(new-pending 현행
/// 경로 회귀 0)이며, 신 kind(kept-drift·merged·conflicted·quarantined)와 healed 의 계보 승계는
/// 전부 이 헬퍼를 경유한다.
/// - kept-drift 인입 + 기존 kind∈LINEAGE → kind 불변, `state:"at-rest"` 만 갱신(멱등 — 이미
///   at-rest 면 no-op·dirty 미발생). 정적 재스윕이 merged/conflicted 계보·캡처 포인터를 지우지
///   않는다(v2 §4 KeepDrift arm 쓰기 0 규칙 · merged_kind_not_clobbered 핀).
/// - 그 외 kind 인입 = kind 전환 허용하되 **capture·base_side 는 신 항목에 승계**(신 항목이 자체
///   값을 갖지 않는 필드만 — 소실 금지. 크래시 창 재스윕 healed 가 영속된 merged 항목의 캡처
///   포인터를 지우는 창의 봉인 · v2 §4). 승계는 old kind 무관 — 한 번 실린 증거 포인터는 어떤
///   후속 upsert 도 떨어뜨리지 않는다.
/// - 전 필드(ts 제외) same 비교 후 upsert — 같으면 no-op(매 기동 install 의 원장 rewrite 방지
///   계약 유지 · ts 는 최초 기록 시각 보존).
/// ★T4 CLI 동사 소비(pack-heal·pack-adopt·revert-merge 원장 계보 전환) — 재구현 금지.
pub fn upsert_pending_v2(
    pending: &mut serde_json::Map<String, serde_json::Value>,
    dirty: &mut bool,
    rel: &str,
    mut entry: serde_json::Map<String, serde_json::Value>,
) {
    let old = pending.get(rel).and_then(|e| e.as_object()).cloned();
    let old_kind = old.as_ref().and_then(|o| o.get("kind")).and_then(|k| k.as_str());
    let new_kind = entry.get("kind").and_then(|k| k.as_str()).unwrap_or("");
    // ★성찰 차단 수리(계상 SOT): 이 바이너리가 **쓰는** kind 는 등재 필수 — LEDGER_KINDS 에
    // 없는 kind 를 쓰면 부트 요약·doctor·pack-plan·preflight 분류가 전부 '미지'로 빠진다.
    // debug 빌드(cargo test 포함)에서만 검사 — 신 바이너리가 남긴 원장을 구 바이너리가 읽는
    // forward-compat 관용(읽기·보존)은 불변이다(승계 항목이 아니라 신규 entry 만 본다).
    debug_assert!(
        LEDGER_KINDS.contains(&new_kind),
        "미등재 원장 kind 쓰기: {new_kind:?} — src/pack.rs LEDGER_KINDS 에 등재 후 사용(계상 SOT)"
    );
    if let Some(ok) = old_kind {
        if LINEAGE_KINDS.contains(&ok) && new_kind == "kept-drift" {
            // kind 불변 — state:"at-rest" 만(멱등 · no-op 시 dirty 미발생 = 원장 rewrite 0).
            let mut e = old.unwrap();
            if e.get("state").and_then(|s| s.as_str()) != Some("at-rest") {
                e.insert("state".to_string(), serde_json::json!("at-rest"));
                pending.insert(rel.to_string(), serde_json::Value::Object(e));
                *dirty = true;
            }
            return;
        }
    }
    // 계보 필드 승계(capture·base_side) — 신 항목이 자체 값을 갖지 않을 때만 구 항목에서 복사.
    for k in ["capture", "base_side"] {
        if !entry.contains_key(k) {
            if let Some(v) = old.as_ref().and_then(|o| o.get(k)) {
                entry.insert(k.to_string(), v.clone());
            }
        }
    }
    let same = old.as_ref().is_some_and(|o| {
        let strip = |m: &serde_json::Map<String, serde_json::Value>| {
            let mut c = m.clone();
            c.remove("ts");
            c
        };
        strip(o) == strip(&entry)
    });
    if !same {
        pending.insert(rel.to_string(), serde_json::Value::Object(entry));
        *dirty = true;
    }
}

/// ★T3(D13): 병합 원장 kind 별 명시 계상 — **빼기 산식 금지**(구 `new_n = len - healed_n` 산식은
/// 신 kind 4종을 전부 '.new 병치'로 오보했다 — W-E2 오계상·성찰 4렌즈 공통 실측). 부트 요약
/// 1줄(v2 §5 채널 1)과 W-E2 사용자 언어 요약이 같은 산식을 공유한다.
/// ★성찰 차단 수리(계상 SOT 3분산): pub 승격 — doctor pack-drift·pack-plan(bin)이 자구 동형
/// 클로저 재구현 대신 이 함수를 위임 소비한다(계상기 3벌 → 1벌 · 상호 일치 = 구조 보장).
/// 버킷은 LEDGER_KINDS 와 1:1 + unknown(미지 kind 명시 계상 — 기존 버킷 오계상 금지는
/// 유지하되, 종전 `_ => {}` 무계상과 달리 안전측 가시로 센다). adopted 는 여기서 세지만 부트
/// 요약 1줄 조건·문구에는 넣지 않는다(복권 확정 = 다음 스윕 정규화 대기 — 설계 의도의 명문화,
/// doctor 는 표시).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PendingKindCounts {
    pub healed: usize,
    pub new_pending: usize,
    pub kept_drift: usize,
    pub merged: usize,
    pub conflicted: usize,
    pub quarantined: usize,
    pub adopted: usize,
    pub unknown: usize,
}

impl PendingKindCounts {
    /// 조치 가능(actionable) 명시 합 — at-rest 보존 kind(kept-drift·merged)만 제외한 전 버킷
    /// 합(빼기 산식 금지 규율 준수). 미지 kind 는 안전측(가시)으로 포함하고, adopted 도 현행
    /// pack-plan 자구('kept-drift·merged 외 전부 검토 대상')를 보존해 포함한다 — pack-merge
    /// 목록이 '조치 불요(복권됨)' 안내로 해소하는 일시 상태라 과보고가 안전측이다.
    pub fn actionable(&self) -> usize {
        self.healed + self.new_pending + self.conflicted + self.quarantined + self.adopted
            + self.unknown
    }
}

pub fn pending_kind_counts(
    pending: &serde_json::Map<String, serde_json::Value>,
) -> PendingKindCounts {
    let mut c = PendingKindCounts::default();
    for e in pending.values() {
        match e.get("kind").and_then(|k| k.as_str()) {
            Some("healed") => c.healed += 1,
            Some("new-pending") => c.new_pending += 1,
            Some("kept-drift") => c.kept_drift += 1,
            Some("merged") => c.merged += 1,
            Some("conflicted") => c.conflicted += 1,
            Some("quarantined") => c.quarantined += 1,
            Some("adopted") => c.adopted += 1,
            _ => c.unknown += 1, // 미지 kind — 기존 버킷 오계상 금지(명시 unknown 계상)
        }
    }
    c
}

/// install 드라이런 리포트(④ 투명성) — `cys pack-plan` 이 설치 **전에** 사용자에게 보여준다.
#[derive(Debug, Default)]
pub struct InstallPlan {
    pub create: Vec<String>,              // 신규 생성
    pub update: Vec<String>,              // 자동 갱신(비수정 system)
    pub heal: Vec<String>,                // 수정본 강제 치유(사용자본 `<rel>.user` 보존 후 덮어씀)
    pub merge_new: Vec<String>,           // user-owned 보존 + 신버전 `<rel>.new` 병치(병합 대기)
    pub keep_user: Vec<String>,           // user-owned 보존(신버전 병치 불요)
    // ★T3(v2 §4 plan_install · v1 감사 D6): 신설 2버킷 — heal 버킷 재사용 금지(재검증 R7:
    // 재사용은 컴파일은 통과하나 pack-plan 이 병합을 치유로 오보하는 의미 드리프트).
    pub kept_drift: Vec<String>,          // ★L2: system 수정본 + vendor 미전진 — 제자리 보존(kept-drift)
    pub merge3: Vec<String>,              // ★L3: 수정본 + vendor 전진 + 검증 base — 자동 3-way 병합
    pub unchanged: usize,                 // 최신(변화 없음)
    pub prune_delete: Vec<String>,        // 폐기 파일 제거(비수정)
    pub prune_keep_modified: Vec<String>, // 폐기됐지만 수정본이라 보존
    pub blocked: Option<String>,          // 다운그레이드 등 설치 차단 사유(파일 판정 무의미)
}

/// install_into 와 **같은 판정 함수**로 드라이런 리포트를 만든다(쓰기 0·드리프트 0).
pub fn plan_install(
    dir: &Path,
    items: &[(&str, &str)],
    force: bool,
    target_version: &str,
) -> InstallPlan {
    let mut plan = InstallPlan::default();
    // ★G3 축2: 스코프 1회 산출(plan 의 dir 는 실 팩 경로 — staging 우회 없음) → 전 판정에 데이터로 주입.
    let scope = pack_scope_of(dir);
    // 다운그레이드 차단 미러(install_into 와 동일 판정).
    if !force {
        if let Some(dv) = std::fs::read_to_string(dir.join(PACK_VERSION_FILE))
            .ok()
            .map(|s| s.trim().to_string())
        {
            if version_gt(&dv, target_version) {
                plan.blocked = Some(format!(
                    "다운그레이드 차단 — 디스크 {dv} > 대상 {target_version}"
                ));
                return plan;
            }
        }
    }
    let manifest: std::collections::BTreeMap<String, String> =
        std::fs::read_to_string(dir.join(INSTALL_MANIFEST))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
    for (rel, content) in items.iter().copied() {
        let path = dir.join(rel);
        let exists = path.exists();
        let disk = if exists { std::fs::read_to_string(&path).ok() } else { None };
        let mh = manifest.get(rel).map(String::as_str);
        // ★T3(D9) base lazy 로드 사전 필터 — 이 필터는 **과대포함만 허용**(과소포함=Merge3 침묵
        // 불발) · decide L3 가 최종 판정. 드리프트 파일에서만 pristine read 가 발생한다(전 파일
        // read+hash 2배화 회귀 방지 — 실측 통상 0~10건).
        let verified_base: Option<String> = match (exists, disk.as_deref(), mh) {
            (true, Some(d), Some(m)) if d != content => load_verified_base(dir, rel, m),
            _ => None,
        };
        match decide_file_action(
            rel,
            content,
            exists,
            disk.as_deref(),
            mh,
            force,
            scope,
            verified_base.as_deref(),
        ) {
            FileAction::Write { heal_user_copy: true } => plan.heal.push(rel.to_string()),
            FileAction::Write { heal_user_copy: false } => {
                if exists {
                    plan.update.push(rel.to_string());
                } else {
                    plan.create.push(rel.to_string());
                }
            }
            FileAction::Keep { new_pending: true, .. } => plan.merge_new.push(rel.to_string()),
            FileAction::Keep { .. } => {
                // 비-System(user·seed-once) — 스코프 인지 판정(dept soul 은 SeedOnce 라도 동일 분류).
                if ownership_scoped(rel, scope) != Ownership::System
                    && disk.as_deref() != Some(content)
                {
                    plan.keep_user.push(rel.to_string());
                } else {
                    plan.unchanged += 1;
                }
            }
            FileAction::KeepDrift => plan.kept_drift.push(rel.to_string()), // ★T3(D14): 전용 버킷 계상
            FileAction::Merge3 => plan.merge3.push(rel.to_string()),        // ★T3(D14): 전용 버킷 계상
        }
    }
    // prune 프리뷰(install_into prune 블록과 동일 판정).
    let embedded: std::collections::HashSet<&str> = items.iter().map(|(rel, _)| *rel).collect();
    if !embedded.is_empty() {
        for (rel, mh) in manifest.iter() {
            if embedded.contains(rel.as_str())
                || ownership_scoped(rel, scope) != Ownership::System
            {
                continue;
            }
            match std::fs::read_to_string(dir.join(rel)) {
                Ok(existing) if mh.as_str() == content_hash(&existing).as_str() => {
                    plan.prune_delete.push(rel.clone());
                }
                Ok(_) => plan.prune_keep_modified.push(rel.clone()),
                Err(_) => {} // 파일 이미 없음 — 매니페스트 정리만(사용자 표시 불요)
            }
        }
    }
    plan
}

/// semver(major.minor.patch) 파싱 — version_gt 내부 parts와 동일 규칙('v' 접두 제거,
/// prerelease/build suffix('-rc','+build') 분리, major 결측·비숫자는 None). ★version_gt와 달리
/// 파싱 실패를 안전측 bool로 흡수하지 않고 Option으로 노출한다 — remote 비교(§7-④)는 실패=거부
/// (fail-CLOSED 반영거부) 방향이라 보존 방향인 version_gt와 묶으면 안 된다.
pub fn parse_semver(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.trim().trim_start_matches('v').split('.').map(|p| {
        // prerelease/build suffix 분리: '10-rc' → '10', '0+build' → '0'
        p.split(|c| c == '-' || c == '+')
            .next()
            .unwrap_or("")
            .parse::<u32>()
            .ok()
    });
    let major = it.next().flatten()?; // major 결측·비숫자 → 파싱 실패
    Some((
        major,
        it.next().flatten().unwrap_or(0),
        it.next().flatten().unwrap_or(0),
    ))
}

/// 무중단 채널 반영 판정(§7-④): remote 팩 버전이 디스크 버전보다 새것인가.
/// ★fail-CLOSED 반영거부: **둘 다 파싱 성공 AND remote > disk**일 때만 true. 어느 한쪽이라도
/// 파싱 실패면 false(반영 거부)다 — version_gt(disk-vs-embed 보존용, 파싱 실패=보존=true)와 안전
/// 방향이 반대다. P4 `cys pack-update`의 version_gates(반영 판정 축)가 호출한다.
pub fn remote_is_newer(remote: &str, disk: &str) -> bool {
    match (parse_semver(remote), parse_semver(disk)) {
        (Some(r), Some(d)) => r > d,
        _ => false, // 파싱 실패 = 신버전 아님 = 반영 거부(fail-CLOSED)
    }
}

/// 부트 스윕 조기 반환 게이트 — 디스크 팩이 `binary_version` 이상으로 커밋됐고(.pack-version)
/// 매니페스트가 실재하면 true(스윕 불요). 사용처 = **cysd 부트**(cysd/main.rs 온보딩②) 단독.
/// (v3에서 GUI 온보딩도 이 술어를 썼으나, cysd가 GUI보다 먼저 돈 머신에서 게이트가 선점돼
/// hook 미설치 회귀(0.12.52 cys-neo) → GUI는 자체 완료 마커(.gui-onboarded·main.rs)로 분리(v4).
/// GUI 게이트로 재사용하지 마라 — 팩 최신 ≠ GUI 온보딩(hook·schtasks) 완료.)
/// - 디스크>바이너리(무중단 pack-update 전진)도 true — 스윕해봐야 install의 다운그레이드
///   차단에 막히므로 스킵이 동치·저렴하다(lame-duck 스큐의 매 부트 차단 로그 소음도 제거).
/// - ★안전 방향이 remote_is_newer(fail-CLOSED=반영 거부)와 반대다: 마커 부재·파싱 실패·
///   매니페스트 부재 = false = **스윕(치유) 실행**. 게이트는 "확실히 최신"일 때만 닫힌다.
/// - 매니페스트는 존재 stat만 검사한다 — 깊은 파싱 검증은 doctor 소관(매 부트 파싱 = 비용 재유입).
pub fn pack_current_in(dir: &Path, binary_version: &str) -> bool {
    if !dir.join(INSTALL_MANIFEST).exists() {
        return false;
    }
    let Ok(disk) = std::fs::read_to_string(dir.join(PACK_VERSION_FILE)) else {
        return false;
    };
    match (parse_semver(disk.trim()), parse_semver(binary_version)) {
        (Some(d), Some(b)) => d >= b,
        _ => false,
    }
}

/// pack_current_in의 실경로 래퍼 — 호출부(GUI setup·cysd main)가 pack_dir 해석에 재결합하지 않게 한다.
/// ★게이트는 반드시 **부트 호출부**에 두고 install() 내부에 넣지 마라 — 내부에 넣으면 수동
/// `cys init-pack`·pack-update·pack-downgrade(치유의 정식 경로)까지 게이트되어 치유가 불구가 된다.
pub fn pack_current_for(binary_version: &str) -> bool {
    pack_current_in(&pack_dir(), binary_version)
}

/// 원자적 파일 쓰기(§7-⑤): 같은 디렉터리 temp 파일에 쓰고 fsync → rename으로 원자 교체 →
/// 디렉터리 fsync(best-effort). 쓰는 도중 crash 시 부분 파일이 최종 경로에 남지 않는다
/// (std::fs::write는 비원자라 부분 쓰기 노출). cysd governance의 write_json_atomic과 동형.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_atomic_mode(path, bytes, None)
}

/// [`write_atomic`] 의 **퍼미션 보존판**. `mode=None` 이면 종전과 완전히 같다(위임 관계라
/// 사본이 아니다 — 이 저장소가 반복해서 맞은 것이 사본 드리프트다).
///
/// 왜 필요한가: tmp+rename 은 **원본 mode 를 버린다**(`factory_reset.rs` 의 같은 관찰).
/// `.claude.json` 은 실측 0600 이라, 시드가 그것을 0644 로 넓히면 안 된다. 되돌려 chmod 하는
/// 방식은 넓은 창이 잠깐 열리므로, **처음부터** 목표 mode 로 만든다.
pub fn write_atomic_mode(path: &Path, bytes: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    let fname = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    let tmp = parent.join(format!(".{fname}.tmp.{}", std::process::id()));
    let res = (|| -> std::io::Result<()> {
        let mut f = create_tmp_with_mode(&tmp, mode)?;
        f.write_all(bytes)?;
        f.sync_all()?; // 파일 본문 fsync (rename 전)
        std::fs::rename(&tmp, path)?; // 원자 교체
        Ok(())
    })();
    match res {
        Ok(()) => {
            // 디렉터리 엔트리 영속화 — best-effort(실패 무시).
            if let Ok(d) = std::fs::File::open(parent) {
                let _ = d.sync_all();
            }
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// tmp 파일 생성 — `mode=Some(m)` 이면 **생성 시점부터** 그 퍼미션이다(사후 chmod 창 0).
/// `mode=None` 이면 `File::create` 와 동치(종전 경로 무변경).
#[cfg(unix)]
fn create_tmp_with_mode(tmp: &Path, mode: Option<u32>) -> std::io::Result<std::fs::File> {
    let Some(m) = mode else {
        return std::fs::File::create(tmp);
    };
    use std::os::unix::fs::OpenOptionsExt;
    // 잔여 tmp 가 있으면 `.mode()` 는 적용되지 않는다(생성 시에만 유효) — 먼저 치운다.
    let _ = std::fs::remove_file(tmp);
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(m)
        .open(tmp)
}

#[cfg(not(unix))]
fn create_tmp_with_mode(tmp: &Path, _mode: Option<u32>) -> std::io::Result<std::fs::File> {
    std::fs::File::create(tmp)
}

/// pristine 미러 갱신(best-effort — 3-way 병합의 공통 조상 확보용 자문 데이터).
/// **디스크에 실제 적용된** vendor 내용일 때만 호출된다 — user-owned 동결 파일에는 호출하지
/// 않아 조상이 사용자가 fork 한 시점의 vendor 본으로 남는다(3-way 정확성의 핵심).
/// ★T3(D8): 본문은 ensure_pristine_checked 로 승격 이관 — 이 래퍼가 기존 호출 2곳(Keep adopt·
/// 공용 write)과 KeepDrift 백필의 best-effort 계약을 자구 그대로 보존한다.
fn ensure_pristine(dir: &Path, rel: &str, content: &str) {
    let _ = ensure_pristine_checked(dir, rel, content);
}

/// ★T3(D8): ensure_pristine 의 검증형 — 동일 본문·에러 승격. 소비자는 Merge3 성공 경로(④)
/// 하나다: pristine 전진 실패는 다음 릴리스의 base 소실이므로 침묵 불가 — 실패 시 ④를 중단하고
/// healed 폴백으로 강등한다(손실 0 · v2 §4).
fn ensure_pristine_checked(dir: &Path, rel: &str, content: &str) -> Result<(), String> {
    let p = dir.join(PRISTINE_DIR).join(rel);
    if std::fs::read_to_string(&p).ok().as_deref() == Some(content) {
        return Ok(());
    }
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("pristine 디렉터리 생성 실패 {}: {e}", parent.display()))?;
    }
    write_atomic(&p, content.as_bytes())
        .map_err(|e| format!("pristine 기록 실패 {}: {e}", p.display()))
}

/// ★T3(D5 · v2 §4 ①): 캡처 루트 — env_compat("CYS_PACK_CAPTURES_DIR") 오버라이드 우선, 기본 =
/// 설치 대상 dir 의 형제 `pack-captures`(라이브 = ~/.cys/pack-captures — local_dir 전례 동형).
/// ★pack_dir() 직접 사용 금지(W0-a): 테스트 빌드 env 미설정 panic + staged 설치(dir=staging)
/// 비수렴 — 반드시 install_into 의 dir 에서 유도한다(staging.parent()==실팩.parent() 실측).
/// 팩 밖 경로라 write_journal·rollback·prune·pack.prev 전 기계 무접촉 = **트랜잭션 롤백을
/// 생존하는 증거 저장소**(MERGE_AUDIT 비저널 전례 동형). 캡처 GC 없음(0.14.29 · D15 — 릴리스
/// 노트 이월 기재).
/// ★T4 CLI 동사 소비(revert-merge 캡처 상대경로 해소) — 재구현 금지.
pub fn pack_captures_dir(dir: &Path) -> PathBuf {
    if let Some(d) = crate::env_compat("CYS_PACK_CAPTURES_DIR") {
        return PathBuf::from(d);
    }
    match dir.parent() {
        Some(parent) => parent.join("pack-captures"),
        None => PathBuf::from("pack-captures"),
    }
}

/// ★T3(D5): 스윕당 배타 캡처 세그먼트 — `<root>/<pack-basename>/<unix_secs>-<pid>[-n]`.
/// 배타 fs::create_dir 로 같은 초 재스윕(크래시 직후 재기동)이 최초 사용자본 캡처를 덮는 창을
/// 봉인한다 — AlreadyExists 는 -1,-2… 접미 루프(결정론·플랫폼 무관 · 콜론 없는 Windows 안전
/// 명명). 반환 = (절대 경로, 캡처 루트 상대 세그먼트). Err = 캡처 불가(호출자가 healed 폴백 —
/// §2 원칙 4: 캡처 없이는 병합하지 않는다).
fn create_capture_segment(
    root: &Path,
    pack_basename: &str,
    now_ts: u64,
) -> Result<(PathBuf, String), String> {
    let lane = root.join(pack_basename);
    std::fs::create_dir_all(&lane)
        .map_err(|e| format!("캡처 레인 생성 실패 {}: {e}", lane.display()))?;
    let base_name = format!("{}-{}", now_ts, std::process::id());
    for n in 0..=999u32 {
        let name = if n == 0 { base_name.clone() } else { format!("{base_name}-{n}") };
        let seg = lane.join(&name);
        match std::fs::create_dir(&seg) {
            Ok(()) => return Ok((seg, format!("{pack_basename}/{name}"))),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("캡처 세그먼트 생성 실패 {}: {e}", seg.display())),
        }
    }
    Err(format!("캡처 세그먼트 접미 소진(-999): {}", lane.join(&base_name).display()))
}

/// ★T3(D4): 판독 불가(disk=None) 파일의 바이트 백업 — 기존 `<rel>.user` 슬롯 재사용.
/// fs::copy 직접 덮어쓰기가 아니라 tmp+copy+rename 원자화(복사 중 크래시의 반파 .user 창 봉인 —
/// 단일 슬롯 의미론 자체는 유지 · 세대 백업은 0.14.30 회부). 문자열 경로의 read_to_string 멱등
/// 가드는 비UTF-8 .user 에서 항상 불일치 판정이라 재사용 불가 — 바이트 경로는 무조건 복사한다
/// (치유 성공 후 L1 재도달 없음 — 재복사는 재손상 시에만).
/// ★T4 CLI 동사 소비(pack-heal 판독 불가 파일 바이트 백업) — 재구현 금지.
pub fn byte_backup_user(dir: &Path, rel: &str, src: &Path) -> Result<(), String> {
    let dst = dir.join(format!("{rel}.user"));
    let tmp = dir.join(format!("{rel}.user.tmp-{}", std::process::id()));
    std::fs::copy(src, &tmp).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("바이트 백업 복사 실패 {}: {e}", tmp.display())
    })?;
    std::fs::rename(&tmp, &dst).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("바이트 백업 rename 실패 {}: {e}", dst.display())
    })
}

/// ★T3(D9 · v2 §2 원칙 3): 검증 base lazy 로드 — `.pristine/<rel>` 을 읽어
/// `content_hash(pristine)==manifest_hash` 일 때만 Some. 불일치·부재·판독 실패 = None = 병합
/// 미시도(decide L4 종전 거동 낙하) — 크래시·원장 손상·연쇄 릴리스 스큐가 전부 이 게이트에서
/// "틀린 base 병합" 대신 손실 0 폴백으로 강등된다.
fn load_verified_base(dir: &Path, rel: &str, manifest_hash: &str) -> Option<String> {
    let pristine = std::fs::read_to_string(dir.join(PRISTINE_DIR).join(rel)).ok()?;
    if content_hash(&pristine) == manifest_hash {
        Some(pristine)
    } else {
        None
    }
}

/// PACK 템플릿 설치 (CLI init-pack과 데몬 첫 기동 자동 설치의 공용 코어).
/// force=false: 사용자 수정 파일 불가침 + **비수정 파일은 임베드 신버전으로 자동 갱신**
/// (설치 매니페스트의 설치-당시 해시와 현재 파일 해시가 일치 = 비수정). 매니페스트가
/// 없는 구설치본 파일은 종전대로 보존한다(안전측). 반환: (written, kept).
pub fn install(force: bool, auth: Option<PackWriteAuth>) -> Result<(usize, usize), String> {
    // 얇은 래퍼: embed PACK_ALL(git-추적 전체 트리)를 입력원으로 install_from_iter에 위임한다.
    // ★외부 동작(반환값·디스크 결과·부수효과)은 완전 불변 — C/D/E 호출처 무영향(§3 하위호환).
    // auth는 라이브 대상 쓰기 인가(W0-d) — install_from_iter가 pack_dir()에 쓰므로 그대로 전달한다.
    install_from_iter(
        PACK_ALL.iter().map(|(r, c)| (*r, *c)),
        force,
        env!("CARGO_PKG_VERSION"),
        false, // embed/cysd 경로(비트랜잭션): .pack-version 직접 기록 + 매니페스트 best-effort(외부 동작 불변).
        auth,
    )
}

/// ★결함3(b) **승계 금지 가드 마커 규약 v2** — 2026-08-22 적대 리뷰 BLOCK(F2 치명·F3 중대) 시정본.
///
/// v1 은 마커를 `contains` 로 잡았다. 그래서 **평문 한국어**("승계 금지" 같은 일상 어휘)가 마커가
/// 됐고, 출하 `cysjavis-pack/soul.md` 의 **안내 문장**이 문서 루트(H1) 직속 본문에 있던 탓에
/// **base soul 전량이 드롭**됐다(실측: DROPPED=["# soul.md — 운영 헌장 (최소 골격)"] · KEPT 0 bytes
/// → 조용히 벤더 스켈레톤으로 강등). 오너가 헌장을 채워 넣는 가장 자연스러운 형태에서
/// seed-from-base 가 결정론적으로 무력화되는 치명 결함이었다.
///
/// v3(2026-08-22 2차 BLOCK 시정 — 오너 결정: **레거시 평문 쌍 규칙 삭제**)은 마커를 **줄 전체가
/// 기계 토큰**인 경우로만 좁힌다. v2 가 하위호환으로 남겼던 "인용문 줄에 `본부(base) 레인 전용`
/// 과 `승계 금지` 가 함께 있으면 마커" 규칙은 **자연문 문서를 계속 삭제**했다(리뷰어 실측 4종:
/// 규약을 설명하는 인용문·중첩 인용·들여쓴 인용이 전부 마커로 인정 → 그 절과 하위 절 소실).
/// 평문 한국어를 마커로 인정하는 한 이 오탐은 구조적으로 막을 수 없으므로 규칙 자체를 없앴다.
///
/// **삭제의 결과(정직 고지)**: 옛 평문 표기(`> ★**본부(base) 레인 전용 절 — 승계 금지 가드**: …`)
/// 만 붙어 있는 절은 이제 **부서로 그대로 승계된다**(차단되지 않는다). 그런 절이 있는 base soul
/// 은 정본 마커 한 줄을 절 안에 넣어 주어야 한다. 이것은 **과삭제보다 안전한 실패 방향**이며
/// (승계된 문안은 사람이 지울 수 있지만 삭제된 헌장은 되살릴 수 없다), 미탐은 아래 '인식 실패
/// 감사 흔적'(`unrecognized`)이 조용하지 않게 만든다.
///
///   · 정본 : 줄 전체가 `<!-- cys:no-inherit -->` — 주석 안쪽 공백 유무·대소문자 무관
///            (`<!--cys:no-inherit-->` · `<!-- CYS:NO-INHERIT -->` 모두 인정 · 기계 토큰이므로)
///   · 동치 : 줄 전체가 `cys:no-inherit`(주석 래퍼 없는 맨 토큰 · 대소문자 무관)
///   · 그 외 **어떤 자연문도 마커가 아니다**. 인정 범위는 앞으로도 **좁히는 방향으로만** 바꾼다.
const SOUL_NO_INHERIT_MARKER_LINE: &str = "<!-- cys:no-inherit -->";
/// 위 정본 마커의 주석 래퍼를 벗긴 맨 토큰(줄 전체 일치일 때만 마커 · 대소문자 무관).
const SOUL_NO_INHERIT_TOKEN: &str = "cys:no-inherit";

/// 한 줄이 승계 금지 가드 마커인가 — 규약 v3(위 상수 doc). 줄 전체 일치만 인정한다.
/// HTML 주석 래퍼(`<!--` … `-->`)는 벗겨서 안쪽을 보고, 안쪽 공백과 대소문자는 무시한다.
fn is_no_inherit_marker(line: &str) -> bool {
    let t = line.trim();
    // `<!--x-->` 최소 길이 7 — 접두·접미가 겹치지 않을 때만 래퍼로 인정(`<!---->` 는 빈 본문).
    let inner = match (t.starts_with("<!--"), t.ends_with("-->"), t.len() >= 7) {
        (true, true, true) => &t[4..t.len() - 3],
        _ => t,
    };
    inner.trim().eq_ignore_ascii_case(SOUL_NO_INHERIT_TOKEN)
}

/// ATX heading 레벨(1..=6) — `#` 1~6개 + 공백(또는 줄 끝). setext(`===`·`---`)는 **보수적으로
/// 무시**한다(절 경계 오판이 곧 오삭제라 인식 범위를 좁게 잡는다).
fn atx_heading_level(line: &str) -> Option<usize> {
    let t = line.trim_start();
    let hashes = t.len() - t.trim_start_matches('#').len();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &t[hashes..];
    if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
        Some(hashes)
    } else {
        None
    }
}

/// 여는 코드펜스 판정 — (펜스 문자, 길이). ``` 와 ~~~ 는 **다른 종류**다(F3-c).
fn fence_open(line: &str) -> Option<(char, usize)> {
    let t = line.trim_start();
    for ch in ['`', '~'] {
        let n = t.len() - t.trim_start_matches(ch).len();
        if n >= 3 {
            return Some((ch, n));
        }
    }
    None
}

/// 닫는 코드펜스 판정 — **같은 문자**로 같거나 더 길게, 정보 문자열 없이 그 문자만인 줄.
fn fence_closes(line: &str, ch: char, open_len: usize) -> bool {
    let t = line.trim();
    let n = t.len() - t.trim_start_matches(ch).len();
    n >= open_len && t.len() == n
}

/// 줄별 "코드펜스 내부" 마스크 — **닫힌 펜스만 펜스로 인정**한다(F3-b).
/// 미닫힌 펜스를 펜스로 치면 그 뒤 heading 이 전부 목록에서 빠져 드롭 범위가 EOF 까지 번진다
/// (실측: `## marked` 하나 드롭이 after1·after2 까지 소멸시켰다). 열린 채 EOF 면 펜스가 아니었던
/// 것으로 간주하고 그 다음 줄부터 계속 스캔한다 — 미탐(펜스 안 `#` 을 heading 으로 봄) 방향으로
/// 틀리는 편이 과삭제보다 안전하다.
fn fenced_line_mask(lines: &[&str]) -> Vec<bool> {
    let mut mask = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if let Some((ch, n)) = fence_open(lines[i]) {
            if let Some(j) = (i + 1..lines.len()).find(|&j| fence_closes(lines[j], ch, n)) {
                mask[i..=j].iter_mut().for_each(|m| *m = true);
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    mask
}

/// 원문을 줄 단위로 쪼개되 **줄 종결자를 원본 그대로** 들고 다닌다 — (내용, 종결자 포함 원본 슬라이스).
/// 드롭 후 재조립이 살아남은 줄의 CRLF/LF·마지막 개행 유무를 **바이트 그대로** 유지하게 한다(F3-d:
/// v1 은 `lines()` + `join("\n")` 이라 드롭이 1건이라도 나면 문서 전체 CRLF 가 LF 로 바뀌었다).
fn lines_with_endings(src: &str) -> Vec<(&str, &str)> {
    src.split_inclusive('\n')
        .map(|raw| {
            let content = raw.strip_suffix('\n').unwrap_or(raw);
            let content = content.strip_suffix('\r').unwrap_or(content);
            (content, raw)
        })
        .collect()
}

/// 가드 절 제거 결과 — 남은 본문 · 드롭된 절 heading · **거부 사유**(드롭하려다 안전 규칙에
/// 걸려 취소한 건) · **인식 실패 흔적**(마커 토큰이 있는데 효력이 없던 줄). 둘 다 조용히 삼키지
/// 않는다 — 호출처가 loud 고지 + 원장 flag 로 남긴다(조용한 미탐이 가장 나쁘다).
struct GuardStrip {
    kept: String,
    dropped: Vec<String>,
    refused: Vec<String>,
    unrecognized: Vec<String>,
}

/// ★결함3(b): 승계 금지 가드 마커가 붙은 절을 **heading 단위**로 제거한다.
///
/// 규칙(전부 보수적 방향 — 과삭제 0 이 최우선):
///   · 마커 = `is_no_inherit_marker`(줄 전체가 기계 토큰 · 규약 v3). 평문 어휘는 마커가 아니다.
///   · **코드펜스 안의 마커는 마커가 아니다** — 마커 표기법을 코드블록으로 문서화한 절이
///     통째로 삭제되던 결함 시정(리뷰어 실측). heading 판정과 **같은 마스크**를 쓴다.
///   · 마커 탐색 범위 = 그 절의 **직속 본문**(heading ~ 다음 heading 직전, 레벨 무관) —
///     하위 절의 마커가 상위 절을 끌어내리는 과삭제를 막는다.
///   · 드롭 범위 = 그 절 + **하위 절 전체**(같거나 얕은 레벨의 다음 heading 직전까지).
///   · **문서의 첫 최상위 절 불가침(F2)**: 문서 최소 heading 레벨의 **첫 절**은 뒤에 무엇이
///     오든 드롭하지 않는다. (구 조건 `k==0 && end==EOF` 는 뒤에 H1 이 하나만 더 있으면
///     우회돼 헌장 본문이 통째로 날아갔다 — 리뷰어 실측.) 본부 전용 문안은 하위 절로 표시하는
///     것이 규약이며, 이 규칙이 USER-MANUAL 의 "문서 전체는 어떤 경우에도 제외되지 않는다"를
///     참으로 만든다.
///   · **빈 결과 거부(F2 2선)**: 드롭 결과가 공백이면 전량 취소하고 원문을 유지한다.
///   · 닫힌 코드펜스 내부의 `#` 는 heading 이 아니다(미닫힘은 펜스 아님 · 종류 구분).
///   · 첫 heading 이전(preamble)은 절이 아니므로 **절대 드롭하지 않는다**.
///   · 드롭 0건이면 입력을 **바이트 그대로** 돌려준다(개행 정규화조차 없음).
///   · **인식 실패는 감사에 남긴다**: 펜스 밖에 마커 토큰이 있는데 그 줄이 실제 드롭으로
///     이어지지 않았으면(표기 불일치·preamble 위치·BOM 선행으로 heading 미인식·CR-only
///     개행으로 줄 분리 실패·setext heading 문서 등) `unrecognized` 에 적는다. 인정 범위를
///     넓히는 대신 **왜 안 먹었는지 보이게** 하는 선택이다(조용한 무동작 금지).
fn strip_no_inherit_sections(src: &str) -> GuardStrip {
    let rows = lines_with_endings(src);
    let contents: Vec<&str> = rows.iter().map(|(c, _)| *c).collect();
    let fenced = fenced_line_mask(&contents);
    // 마커는 **펜스 밖**에서만 유효하다(문서화용 코드블록 안의 예시는 마커가 아니다).
    let marker_line: Vec<bool> = contents
        .iter()
        .enumerate()
        .map(|(i, l)| !fenced[i] && is_no_inherit_marker(l))
        .collect();
    // (줄 index, heading level) — 닫힌 코드펜스 밖의 ATX heading 만.
    let heads: Vec<(usize, usize)> = contents
        .iter()
        .enumerate()
        .filter(|(i, _)| !fenced[*i])
        .filter_map(|(i, l)| atx_heading_level(l).map(|lv| (i, lv)))
        .collect();
    // 문서의 첫 최상위 절(= 최소 레벨 heading 중 첫 번째)의 인덱스 — 불가침 대상.
    let root_k = heads
        .iter()
        .map(|&(_, lv)| lv)
        .min()
        .and_then(|min_lv| heads.iter().position(|&(_, lv)| lv == min_lv));
    let mut drop_line = vec![false; rows.len()];
    let mut dropped: Vec<String> = Vec::new();
    let mut refused: Vec<String> = Vec::new();
    for (k, &(start, level)) in heads.iter().enumerate() {
        let own_end = heads.get(k + 1).map(|&(i, _)| i).unwrap_or(rows.len());
        if !marker_line[start..own_end].iter().any(|m| *m) {
            continue;
        }
        if root_k == Some(k) {
            // 문서의 첫 최상위 절 = 문서의 몸통. 마커가 있어도 드롭하지 않는다(F2 재발 차단).
            refused.push(format!(
                "문서의 첫 최상위 절 드롭 거부(문서 몸통 보호): {}",
                contents[start].trim()
            ));
            continue;
        }
        let end = heads[k + 1..]
            .iter()
            .find(|&&(_, lv)| lv <= level)
            .map(|&(i, _)| i)
            .unwrap_or(rows.len());
        drop_line[start..end].iter_mut().for_each(|d| *d = true);
        dropped.push(contents[start].trim().to_string());
    }
    let kept: String = if dropped.is_empty() {
        src.to_string()
    } else {
        rows.iter()
            .zip(drop_line.iter())
            .filter(|(_, d)| !**d)
            .map(|((_, raw), _)| *raw)
            .collect()
    };
    if !dropped.is_empty() && kept.trim().is_empty() {
        // 남는 게 없다 = 승계 의도가 아니라 파서 과삭제로 봐야 한다 — 전량 취소하고 원문 유지.
        refused.push(format!(
            "드롭 결과가 빈 문서 — 전량 취소(원문 유지) · 취소된 드롭 {}건: {}",
            dropped.len(),
            dropped.join(" | ")
        ));
        let unrecognized = unrecognized_marker_notes(&contents, &fenced, &vec![false; rows.len()]);
        return GuardStrip {
            kept: src.to_string(),
            dropped: Vec::new(),
            refused,
            unrecognized,
        };
    }
    let unrecognized = unrecognized_marker_notes(&contents, &fenced, &drop_line);
    GuardStrip { kept, dropped, refused, unrecognized }
}

/// 인식 실패 탐지용 **느슨한** 바늘 — 마커 의도를 알아보되 인정은 하지 않는 신호.
/// (`<!-- cys : no-inherit -->` 처럼 정본 토큰조차 깨진 표기를 잡아야 하므로 토큰보다 짧다.
///  이 바늘은 **경고에만** 쓰이며 어떤 줄도 삭제하지 않는다 — 넓혀도 안전한 유일한 자리다.)
const SOUL_NO_INHERIT_NEAR_MISS: &str = "no-inherit";

/// 옛 평문 표기(v2 가 마커로 인정하던 쌍) — v3 에서 **더 이상 마커가 아니다**. 삭제 판정에는
/// 절대 쓰지 않고 **경고 전용 바늘**로만 남긴다: 이 표기만 붙은 절은 이제 그대로 승계되므로,
/// 거동 변화가 조용히 지나가지 않도록(오너의 base soul 이 실제로 이 상태다) 흔적을 남긴다.
/// 둘을 **한 줄에 모두** 담은 줄만 본다 — 경고조차 자연문에 남발하지 않기 위해서다.
const SOUL_NO_INHERIT_LEGACY_HINT: [&str; 2] = ["본부(base) 레인 전용", "승계 금지"];

/// 마커 의도가 **펜스 밖에** 있는데 실제 드롭으로 이어지지 않은 줄을 감사 흔적으로 모은다.
/// 인정 범위를 넓히지 않고 **보이게** 하는 장치다 — 표기 오류(`<!-- cys : no-inherit -->` 류)·
/// 절 밖 위치(preamble)·BOM 선행으로 heading 미인식·CR-only 개행(문서 전체가 한 줄)·setext
/// heading 문서가 전부 여기 걸린다. 정상 동작한 마커(드롭된 줄 안의 마커)는 잡히지 않는다.
fn unrecognized_marker_notes(contents: &[&str], fenced: &[bool], dropped_line: &[bool]) -> Vec<String> {
    let needle = SOUL_NO_INHERIT_NEAR_MISS.to_ascii_lowercase();
    contents
        .iter()
        .enumerate()
        .filter(|(i, _)| !fenced[*i] && !dropped_line[*i])
        .filter_map(|(i, l)| {
            let head = l.chars().take(60).collect::<String>();
            if l.to_ascii_lowercase().contains(&needle) {
                Some(format!(
                    "{}행: 마커 토큰이 있으나 효력 없음(줄 전체 표기 아님·절 밖·구조 미인식): {head}",
                    i + 1
                ))
            } else if SOUL_NO_INHERIT_LEGACY_HINT.iter().all(|h| l.contains(h)) {
                Some(format!(
                    "{}행: 옛 평문 표기는 v0.14.23부터 마커가 아니다(이 절은 그대로 승계된다): {head}",
                    i + 1
                ))
            } else {
                None
            }
        })
        .collect()
}

/// 시드 대상 **부서명** 산출 — 판정 규칙의 등재소는 `dept_scope_of` 하나다(사본 드리프트 금지).
///   ① `dir` 이 부서 팩(`pack-dept-<name>`)이면 그대로 — cysd 자동설치·install_from_iter 경로.
///   ② `dir` 이 staging(`.pack-staging-init-*` — basename 에 부서 정보가 없다)이면 **논리 대상**
///      에서 같은 규칙으로 다시 뽑는다. install_staged 가 스코프를 Dept 로 판정한 근거가 바로 그
///      `pack_dir()` 이므로 **같은 SOT** 를 보는 셈이고 스코프 판정과 어긋날 수 없다.
///      순수 env 층(`pack_dir_from_env`)을 쓰므로 W0-a 테스트 빌드 panic 을 유발하지 않는다.
/// 둘 다 실패하면 None — 호출처가 '(부서명 미상)' 스탬프 + loud 고지로 강등한다(정체 없는 부팅 방지).
fn dept_name_for_seed(dir: &Path) -> Option<String> {
    dept_scope_of(dir).or_else(|| pack_dir_from_env().as_deref().and_then(dept_scope_of))
}

/// ★결함3(a) **부서장 정체 스탬프** — 부서 soul 최초 시드 본문 맨 앞에 박히는 정체 정의처.
///
/// 실측 사고(2026-08-22): 오너가 GUI 로 부서를 만들고 그 창에서 "너는 마스터다"라고 선언했을 때
/// 부서 마스터가 **자신을 부서장으로 인식·호칭하지 못했다** — 표준 디렉티브에도, 빈 soul 템플릿
/// 에도 "너는 <부서명> 부서장 마스터다"라고 말해 주는 정의처가 없었기 때문이다. 승계본이든
/// 폴백 템플릿이든 **이 스탬프는 항상 앞에 붙는다**(정체 없이 뜨는 부서장 0).
fn dept_identity_stamp(dept: Option<&str>) -> String {
    let name = dept.unwrap_or("(부서명 미상)");
    format!(
        "# 부서 정체 — {name} 부서장 마스터 (부서 팩 최초 시드 자동 스탬프)\n\
         \n\
         > 이 절은 부서 팩 시드가 박은 **부서장 정체의 정의처**다. 아래 본문은 본부(base) 헌장\n\
         > 승계본이며, 정체에 관해서는 이 절이 우선한다 — 이 데몬은 본부가 아니라 부서 레인이다.\n\
         \n\
         - 이 데몬은 부서 **`{name}`** 이고, 이 데몬의 master 는 **\"{name} 부서장 마스터\"** 다 —\n\
         \u{20}\u{20}자신을 그렇게 인식하고 그렇게 호칭한다.\n\
         - **각성 보고 첫 문장에 부서장 정체를 명시하라** — 예: \"저는 {name} 부서장 마스터입니다.\"\n\
         - 본부(base) 데몬의 master 는 **CEO** 다. 부서장은 CEO 에게 보고하고 CEO 의 지시를 받는다 —\n\
         \u{20}\u{20}단 **오너의 직접 지시가 항상 최우선**이며, CEO 지시와 충돌하면 오너 지시가 이긴다.\n\
         - 이 데몬의 워커·CSO·리뷰어는 {name} 부서장 마스터가 지휘한다.\n\
         \n\
         ---\n\
         \n"
    )
}

/// ★G3 축2(seed-from-base): 부서 soul.md 최초 시드 본문 산출. base 팩 위치는 **형제 규약**
/// `<dir 부모>/pack`(프로덕션 ~/.cys/pack 과 동일) — install_staged 의 staging(`.pack-staging-init-*`)
/// 도 pack_dir 형제라 같은 부모를 공유해 동일하게 성립한다(경로 규약 등재소는 이 함수 한 곳).
/// 판독 실패·공백이면 임베드 템플릿 폴백 + stderr 강등 고지 — fail-open 이 아니라 "부서가 soul
/// 없이 뜨는" 더 큰 결함의 방지다(시드 자체는 항상 진행·성찰 위험③ 확정). 승계본이 안전핵
/// 키워드(overrides::SAFETY_KEYWORDS)를 하나도 포함하지 않으면 WARN(시드는 진행 — base 가
/// 과도 상태일 수 있음을 감사 가능하게). 승계/강등 사실은 `.merge-audit.jsonl` 에
/// action="seed-from-base" 라인으로 영속한다(Wave1 원장 기계 재사용 — 필드명은 cys.rs
/// merge_audit_entry 스키마 정합: ts/file/action/actor_os_user/before·after_sha256/verify_result/
/// flags + additive "source". 기록 실패는 loud 후 불차단 — 감사는 관측이지 게이트가 아니다).
///
/// ★결함3(2026-08-22 실측 사고) 두 겹이 여기 얹힌다 — 둘 다 **부서 스코프 시드 경로 전용**이라
/// base 레인 거동은 byte-identical 이다(호출 자체가 `scope == Dept && !exists` 에서만 난다):
///   (a) **부서장 정체 스탬프**(`dept_identity_stamp`)를 승계본·폴백 템플릿 **양쪽 모두** 앞에 삽입 —
///       부서장이 자신을 "부서장"으로 인식할 정의처를 팩이 직접 제공한다.
///   (b) **본부 전용 절 승계 차단**(`strip_no_inherit_sections`) — seed-from-base 는 base soul 을
///       통째로 물려주므로, 가드 마커가 붙은 본부 전용 절(예: "이 데몬의 master 는 CEO 다")이
///       그대로 승계되면 부서장이 자신을 **CEO 로 오인**한다. 마커 규약(줄 앵커 기계 토큰)의
///       등재소는 `SOUL_NO_INHERIT_MARKER_LINE` doc 이다.
/// 안전핵 키워드 판정은 **드롭 이후 본문** 기준이다(드롭된 절에만 안전핵이 있었다면 WARN 이 떠야
/// 정직하다). 드롭·드롭 거부·부서명 미판정은 전부 stderr + 원장 flags 로 감사 가능하게 남긴다.
fn seed_dept_soul_content(dir: &Path, rel: &str, embed: &str) -> String {
    let base_soul = dir.parent().map(|p| p.join("pack").join("soul.md"));
    let inherited = base_soul
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .filter(|s| !s.trim().is_empty());
    // (a) 부서명 — 등재소 `dept_scope_of` 재사용(사본 드리프트 금지). 미판정도 시드는 진행한다.
    let dept = dept_name_for_seed(dir);
    if dept.is_none() {
        eprintln!(
            "[pack] ⚠ 부서 soul 시드: 부서명 판정 실패({} · staging 이면 pack env 도 미설정) — \
             정체 스탬프를 '(부서명 미상)'으로 박는다(정체 없이 뜨는 부서장 방지)",
            dir.display()
        );
    }
    let mut flags: Vec<&'static str> = Vec::new();
    if dept.is_none() {
        flags.push("dept-name-unresolved");
    }
    let mut dropped_sections: Vec<String> = Vec::new();
    let (body, source, verify) = match inherited {
        Some(s) => {
            // (b) 본부 전용 절 승계 차단 — 마커가 명확한 절만, heading 단위로 통째로 드롭.
            //     드롭 거부(문서 루트·빈 결과)는 **조용히 삼키지 않는다** — 원문이 그대로 승계되므로
            //     본부 전용 문안이 남아 있을 수 있다는 사실을 loud 고지 + 원장 flag 로 남긴다(F2).
            let strip = strip_no_inherit_sections(&s);
            if !strip.dropped.is_empty() {
                eprintln!(
                    "[pack] 부서 soul 시드: 본부(base) 전용 절 {}건 승계 차단 — {}",
                    strip.dropped.len(),
                    strip.dropped.join(" | ")
                );
                flags.push("guard-dropped");
            }
            for r in &strip.refused {
                eprintln!(
                    "[pack] ⚠ 부서 soul 시드: 가드 절 드롭을 안전 규칙으로 **거부**했다 — {r} \
                     (원문 승계 유지 · 본부 전용 문안이 남아 있을 수 있으니 부서 soul 을 확인하라)"
                );
            }
            if !strip.refused.is_empty() {
                flags.push("guard-drop-refused");
            }
            // 조용한 무동작 금지 — 마커를 붙였는데 안 먹은 줄은 반드시 보이게 한다.
            for u in &strip.unrecognized {
                eprintln!(
                    "[pack] ⚠ 부서 soul 시드: 승계 금지 마커가 **인식되지 않았다** — {u} \
                     · 정본 표기는 줄 전체가 `{SOUL_NO_INHERIT_MARKER_LINE}` 이며 제외하려는 \
                     절(heading) 안에 넣어야 한다"
                );
            }
            if !strip.unrecognized.is_empty() {
                flags.push("guard-marker-unrecognized");
            }
            dropped_sections = strip.dropped;
            let src = base_soul
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let lower = strip.kept.to_lowercase();
            let has_core = crate::overrides::SAFETY_KEYWORDS
                .iter()
                .any(|kw| lower.contains(kw));
            if !has_core {
                eprintln!(
                    "[pack] ⚠ 부서 soul 시드: base 헌장 승계본에 안전핵 키워드 0건 — base soul 이 \
                     과도 상태일 수 있음(시드는 진행 · 감사 원장에 기록)"
                );
            }
            (strip.kept, src, if has_core { "pass" } else { "warn-no-safety-core" })
        }
        None => {
            eprintln!(
                "[pack] ⚠ 부서 soul 시드: base 헌장({}) 승계 실패(부재·판독 불가·공백) — 임베드 \
                 템플릿 시드로 강등",
                base_soul
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<부모 없음>".into())
            );
            (embed.to_string(), "embed-template".to_string(), "degraded-template-fallback")
        }
    };
    // 정체 스탬프는 **어느 분기에서도** 앞에 붙는다(승계 성공·강등 폴백 무관 — 결함3 (a)의 목적).
    let content = format!("{}{}", dept_identity_stamp(dept.as_deref()), body);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let os_user = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" })
        .unwrap_or_else(|_| "unknown".into());
    let entry = serde_json::json!({
        "ts": ts,
        "file": rel,
        "action": "seed-from-base",
        "actor_os_user": os_user,
        "before_sha256": null, // 시드 전 부재(!exists 게이트) — 전신 없음
        "after_sha256": content_hash(&content),
        "verify_result": verify,
        "flags": flags,
        // ★결함3 additive 필드 — 정체 스탬프의 부서명과 승계 차단된 절 목록(사후 추적 가능성).
        "dept": dept,
        "dropped_sections": dropped_sections,
        "source": source,
    });
    // 신설 팩 첫 시드는 파일 쓰기(부모 생성)보다 먼저 돈다 — 원장 append 가 dir 부재로 죽지 않게.
    let _ = std::fs::create_dir_all(dir);
    if let Err(e) = append_merge_audit(dir, &entry) {
        eprintln!("⚠ 감사 원장 기록 실패(시드는 계속): {e}");
    }
    content
}

/// install의 **파일 반영 코어**(§7-⑤): `(rel, content)` 이터레이터를 입력원으로 받아 preserve-gate·
/// prune·매니페스트·다운그레이드 차단·.pack-version 기록·격리 config·exec bit를 수행한다.
/// embed PACK_ALL iter(기존 경로)와 staged-tree iter(무중단 채널)가 같은 로직을 공유한다(중복 0·회귀 0).
/// 다운그레이드 가드 비교 기준은 `target_version`(env! 직접 참조 제거 — staged 입력은 자기 버전을 넘김).
/// force=false: user-owned·seed-once 불가침, 수정된 system 파일은 벤더 미전진=kept-drift 제자리
/// 보존·벤더 전진=검증 base 3-way(충돌=치유+`<rel>.user` 보존), 비수정 파일은 입력 신버전으로 자동 갱신.
/// ("사용자 수정 파일 불가침" 구 문구는 user-owned 등급에만 참인데 전체로 읽혀 배포 현장의 오판을
/// 낳았다 — 2026-07-12 치유 원복 사고 시정.) 반환: (written, kept).
/// `transactional`: false면 embed/cysd/init-pack 경로 — 종전대로 마지막에 `.pack-version`을
/// best-effort 기록하고 `.install-manifest.json` 영속도 best-effort(외부 동작 불변). true면
/// 무중단 pack-update 트랜잭션(apply_pack_transactional) 경로 — ⓐ`.pack-version`을 여기서
/// 기록하지 않는다(record_accepted **이후** apply_pack_transactional이 마지막 hard commit
/// marker로 직접·검사 기록·R2CODE HIGH #1), ⓑ`.install-manifest.json` write 실패를 **fail-closed**로
/// Err 반환해 apply_pack_transactional이 rollback_journal를 타게 한다 — 매니페스트가 손상/구상태로
/// 남으면 다음 update preserve-gate가 새 파일을 사용자 수정본으로 오판(자동갱신·prune 차단)하는
/// 부분커밋을 차단(R2CODE2 HIGH #1).
/// ★G3 축2: `scope` 는 **호출자가 논리 대상에서 산출**해 넘긴다(install_from_iter=pack_dir,
/// install_staged=pack_dir — staging 의 basename `.pack-staging-init-*` 에는 부서 정보가 없어
/// 여기서 dir 재판정하면 부서 팩이 Base 로 오판된다). Base 스코프 거동은 byte-identical.
/// ★캡처 레인 명명도 같은 계약(v2 §4 ① — 성찰 차단 수리): `capture_ns` = 캡처 레인 basename.
/// None = dir 자신에서 유도(직설치 — dir 이 곧 논리 대상). install_staged 는 물리 staging
/// basename(`.pack-staging-init-<pid>` — ls 비표시 dot·pid 휘발 명명)이 아니라 **논리 대상
/// (pack_dir) basename** 을 데이터로 주입한다 — 스코프의 "staging basename 재판정 금지" 와
/// 자기모순이던 내부 계약의 정합화(캡처 실파일·원장 capture 포인터가 은닉 레인에 흩어지는 것 차단).
pub fn install_into<'a, I: IntoIterator<Item = (&'a str, &'a str)>>(
    dir: PathBuf,
    items: I,
    force: bool,
    target_version: &str,
    transactional: bool,
    setup_config: bool,
    scope: PackScope,
    auth: Option<PackWriteAuth>,
    capture_ns: Option<&str>,
) -> Result<(usize, usize), String> {
    // ★W0-d 양성 인가 게이트(최후 방어) — 어떤 부수효과보다 먼저. 대상이 라이브 기본 경로면
    // 인가 없이는 하드 거부한다(테스트 재오염 벡터 구조적 봉인). 비라이브·인가 보유는 통과.
    authorize_pack_write(&dir, auth)?;
    // items를 한 번 Vec로 고정 — 쓰기 루프·prune embedded-set·exec bit 루프 세 곳이 같은 집합을 본다.
    let items: Vec<(&str, &str)> = items.into_iter().collect();
    // ★채널 가드(v6 §5 — 내장/비트랜잭션 경로만): state=pro·손상이면 쓰기+prune **전체 생략**
    // (내장 free 팩이 pro 팩을 파괴하는 R1 실증 재앙 차단). pack-update(transactional=true)는
    // 자체 채널·버전 게이트를 통과한 서명 팩이므로 이 가드를 타지 않는다.
    if !transactional {
        if let Some(reason) = channel_guard_and_heal(&dir) {
            println!("[init-pack] {reason}");
            return Ok((0, 0));
        }
    }
    let manifest_path = dir.join(INSTALL_MANIFEST);
    let mut manifest: std::collections::BTreeMap<String, String> = std::fs::read_to_string(
        &manifest_path,
    )
    .ok()
    .and_then(|s| serde_json::from_str(&s).ok())
    .unwrap_or_default();
    // 다운그레이드 차단: 디스크 팩 버전이 입력 버전(target_version)보다 새것이면(구버전 cys로 롤백/오설치)
    // 비강제 install이 비수정 파일·prune으로 신기능을 구 내용으로 후퇴시키는 사일런트 회귀를 막는다.
    // force(수동 init-pack --force)면 우회 — 의도적 재설치는 허용.
    if !force {
        if let Some(dv) = std::fs::read_to_string(dir.join(PACK_VERSION_FILE))
            .ok()
            .map(|s| s.trim().to_string())
        {
            if version_gt(&dv, target_version) {
                // stdout 명시 — 정상 멱등 설치(0 written)와 구분되도록 호출처/UI가 차단을 인지하게 한다.
                println!(
                    "[init-pack] 다운그레이드 차단 — 팩 미반영 (디스크 {dv} > 바이너리 {target_version}). 의도적 재설치는 force로."
                );
                return Ok((0, 0));
            }
        }
    }
    let mut written = 0;
    let mut kept = 0;
    // ★커스터마이즈 절충 원장(②): .new/.user 병치·pristine 미러·병합 대기 기록.
    // 판정 자체는 decide_file_action(순수 — ★B2 user-owned 영구 보존 · system 은 L0-L4 판정표
    // (v2 §3) · 비수정 자동 갱신)에 위임하고, 여기는 부수효과만 수행한다.
    let mut pending = load_merge_pending(&dir);
    let mut pending_dirty = false;
    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 병합 대기 항목 upsert — kind·side·version 이 이미 같으면 no-op(매 기동 install 의 원장 rewrite 방지).
    let upsert_pending = |pending: &mut serde_json::Map<String, serde_json::Value>,
                              dirty: &mut bool,
                              rel: &str,
                              kind: &str,
                              side: String| {
        let same = pending.get(rel).is_some_and(|e| {
            e.get("kind").and_then(|v| v.as_str()) == Some(kind)
                && e.get("side").and_then(|v| v.as_str()) == Some(side.as_str())
                && e.get("version").and_then(|v| v.as_str()) == Some(target_version)
        });
        if !same {
            pending.insert(
                rel.to_string(),
                serde_json::json!({"kind": kind, "side": side, "version": target_version, "ts": now_ts}),
            );
            *dirty = true;
        }
    };
    // ★W-1: 존재하지만 판독 불가(비UTF-8·권한·잠금)라 **덮지 않고 그대로 둔** 파일 목록.
    // 병합 대기 원장(.user/.new)에는 잡히지 않는 등급이라, 여기서 따로 세지 않으면 같은 실행의
    // 사용자 보고("내 커스텀은 지워지지 않았습니다")가 이 파일들을 침묵으로 빠뜨린다.
    let mut unreadable_kept: Vec<String> = Vec::new();
    // ★T3(D5) Merge3 캡처 상태(스윕당 1세그먼트 · 지연 생성): 첫 병합 시 배타 생성 — 같은 스윕의
    // 후속 병합 파일은 같은 세그먼트 아래 <rel> 로 쌓인다. 원장 capture = 캡처 루트 상대 경로.
    let captures_root = pack_captures_dir(&dir);
    // 레인 basename = 논리 대상(capture_ns 주입 시) > 물리 dir basename(직설치 — 동일 값) 순.
    let pack_basename = capture_ns
        .map(str::to_string)
        .or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "pack".to_string());
    let mut capture_seg: Option<(PathBuf, String)> = None;
    for (rel, content) in items.iter().copied() {
        let path = dir.join(rel);
        let exists = path.exists();
        let disk = if exists { std::fs::read_to_string(&path).ok() } else { None };
        // 판정 입력과 동일한 사실(존재하나 읽기 실패) — 소유권 술어를 다시 쓰지 않는다(SOT 분산 금지).
        let unreadable = exists && disk.is_none();
        let mhash: Option<String> = manifest.get(rel).cloned();
        // ★G3 축2(seed-from-base): 부서 soul.md 의 최초 시드는 임베드 템플릿이 아니라 **base 팩의
        // 현행 soul.md 승계**다(부서는 base 헌장 아래에서 태어난다 — 2026-08-21 확정). `!exists`
        // 에만 발동하므로 이후 SeedOnce 불가침과 정합하고, 치환된 content 가 아래 판정·write·
        // 매니페스트 해시·pristine 까지 한 흐름으로 흘러 스큐가 없다.
        let seed_override: Option<String> = if scope == PackScope::Dept
            && !exists
            && (rel == "soul.md" || rel.ends_with("/soul.md"))
        {
            Some(seed_dept_soul_content(&dir, rel, content))
        } else {
            None
        };
        let content: &str = seed_override.as_deref().unwrap_or(content);
        // ★T3(D9) base lazy 로드 사전 필터 — 과대포함만 허용(과소포함=Merge3 침묵 불발) ·
        // decide L3 가 최종 판정. 드리프트 파일 한정 IO(전 파일 pristine read 회귀 방지).
        let verified_base: Option<String> = match (exists, disk.as_deref(), mhash.as_deref()) {
            (true, Some(d), Some(m)) if d != content => load_verified_base(&dir, rel, m),
            _ => None,
        };
        match decide_file_action(
            rel, content, exists, disk.as_deref(), mhash.as_deref(), force, scope,
            verified_base.as_deref(),
        ) {
            FileAction::Keep { adopt_hash, new_pending } => {
                if adopt_hash {
                    // 디스크 = 임베드: 최신. 매니페스트 공백(구설치본)이면 채택 기록해
                    // 다음 버전부터 자동 갱신 대상이 되게 한다. pristine 승계도 보장.
                    manifest
                        .entry(rel.to_string())
                        .or_insert_with(|| content_hash(content));
                    ensure_pristine(&dir, rel, content);
                    // ★T3(D6) 원장 수명 — disk==embed 재일치 합류점: kept-drift·quarantined 는
                    // 제거(new-pending 청소 동형 — 방치 시 유령 계상 영구화 실측), merged·conflicted
                    // 는 보존하되 state:"at-rest" 만 갱신(capture·base_side 계보 가치 — v2 §4).
                    match pending.get(rel).and_then(|e| e.get("kind")).and_then(|k| k.as_str()) {
                        Some("kept-drift") | Some("quarantined") => {
                            pending.remove(rel);
                            pending_dirty = true;
                        }
                        Some(k) if LINEAGE_KINDS.contains(&k) => {
                            if let Some(serde_json::Value::Object(e)) = pending.get_mut(rel) {
                                if e.get("state").and_then(|s| s.as_str()) != Some("at-rest") {
                                    e.insert("state".to_string(), serde_json::json!("at-rest"));
                                    pending_dirty = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if new_pending {
                    // user-owned 보존 + 임베드 신버전 병치(idempotent) — '영구 동결'을 '보이는 병합 대기'로.
                    let new_path = dir.join(format!("{rel}.new"));
                    if std::fs::read_to_string(&new_path).ok().as_deref() != Some(content) {
                        let _ = write_atomic(&new_path, content.as_bytes());
                    }
                    upsert_pending(&mut pending, &mut pending_dirty, rel, "new-pending", format!("{rel}.new"));
                } else if !unreadable
                    && pending
                        .get(rel)
                        .and_then(|e| e.get("kind"))
                        .and_then(|k| k.as_str())
                        == Some("new-pending")
                {
                    // 병합 대기 해소(사용자가 vendor 본 채택 등) — 원장·.new 잔재 청소.
                    // ★W-1: 판독 불가로 보존된 건은 '해소'가 아니라 '비교 불가'다 — 대기 상태를
                    // 그대로 둬야 한다(읽지 못했다는 이유로 vendor 신버전 사본을 지우면 정보 손실).
                    pending.remove(rel);
                    pending_dirty = true;
                    let _ = std::fs::remove_file(dir.join(format!("{rel}.new")));
                }
                if unreadable {
                    unreadable_kept.push(rel.to_string());
                }
                kept += 1;
                continue;
            }
            FileAction::KeepDrift => {
                // ★T3(v2 §3 L2 · v1 Patch 1 흡수): 벤더 미전진 + system 드리프트 — 제자리 보존.
                // 파일 쓰기 0(Keep 동형) + 원장 kept-drift 계상(가시화 4채널의 원천 · D1 인터림 해소).
                if !unreadable
                    && pending.get(rel).and_then(|e| e.get("kind")).and_then(|k| k.as_str())
                        == Some("new-pending")
                {
                    pending.remove(rel);
                    pending_dirty = true;
                    let _ = std::fs::remove_file(dir.join(format!("{rel}.new")));
                }
                // 원장 계상(kind:"kept-drift", side:rel) — 기존 kind∈LINEAGE(merged·conflicted)면
                // kind 불변·state:"at-rest" 만(upsert_pending_v2 계보 규칙 · no-op 시 rewrite 0).
                if let serde_json::Value::Object(entry) = serde_json::json!({
                    "kind": "kept-drift", "side": rel,
                    "version": target_version, "ts": now_ts,
                }) {
                    upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                }
                // ★T3 pristine 백필 가드(v2 §4 · v1 감사 D3 — best-effort): .pristine/<rel> 부재 ∧
                // manifest[rel]==hash(embed)(L2 도달 조건이 이미 증명) → embed 로 백필. 해시
                // 등식이 embed==마지막 적용 vendor 임을 바이트 수준으로 증명하므로 안전(다음
                // 릴리스 L3 base 재료 — pristine 부재 레거시 코호트의 병합 진입로).
                if !dir.join(PRISTINE_DIR).join(rel).exists() {
                    ensure_pristine(&dir, rel, content);
                }
                if unreadable {
                    unreadable_kept.push(rel.to_string());
                }
                kept += 1;
                continue;
            }
            FileAction::Merge3 => {
                // ★T3 실행부(v2 §4 · D10): 검증된 조상 위 자동 3-way 병합. 성공(④)은 자체 continue
                // 종결 — 공용 write 낙하 재사용 불가(성공 경로는 disk←merged·pristine←embed·
                // manifest←hash(embed) 로 세 값이 갈라진다 · 성찰 실측). 실패 5경로(캡처·충돌·
                // json 게이트·손상 의심·pristine 검증형)는 전부 ⑤ healed 폴백 = 현행 Write{heal:
                // true} 경로 동형 낙하(.user 보존 + vendor 기록 + 원장 conflicted{base_side} +
                // <rel>.base 조상 사이드카) — 어떤 실패도 "0.14.28 처럼 동작"으로 강등될 뿐 손실 0.
                let fallback_reason: String; // ⑤ 원장 reason 토큰(T4 doctor 소비) — 전 ⑤ 경로 확정 대입
                let mut fallback_user_src: Option<String> = None; // pristine-write-failed 만 merged
                let gates: Result<String, String> = (|| {
                    let (Some(base), Some(ours)) = (verified_base.as_deref(), disk.as_deref())
                    else {
                        // 방어 낙하(도달 불가 — decide L3 가 base·disk 실재를 보장).
                        return Err("merge-fallback".to_string());
                    };
                    // ① 캡처: 사용자본 전문 → 팩 밖 캡처 세그먼트. 실패 = 병합 미시도 → ⑤
                    //   (§2 원칙 4 — 캡처 없이는 병합하지 않는다) · 감사 append 만 best-effort.
                    if capture_seg.is_none() {
                        match create_capture_segment(&captures_root, &pack_basename, now_ts) {
                            Ok(seg) => capture_seg = Some(seg),
                            Err(e) => {
                                eprintln!("[init-pack] ⚠ 병합 캡처 불가({rel}): {e} — 병합 미시도·healed 폴백");
                                return Err("capture-failed".to_string());
                            }
                        }
                    }
                    let cap_path = capture_seg.as_ref().unwrap().0.join(rel);
                    if let Some(parent) = cap_path.parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            eprintln!("[init-pack] ⚠ 병합 캡처 불가({rel}): {e} — 병합 미시도·healed 폴백");
                            return Err("capture-failed".to_string());
                        }
                    }
                    if let Err(e) = std::fs::write(&cap_path, ours.as_bytes()) {
                        eprintln!("[init-pack] ⚠ 병합 캡처 불가({rel}): {e} — 병합 미시도·healed 폴백");
                        return Err("capture-failed".to_string());
                    }
                    if let Err(e) = append_merge_audit(
                        &dir,
                        &serde_json::json!({
                            "kind": "pre-merge-capture", "rel": rel,
                            "sha": content_hash(ours), "base_sha": content_hash(base),
                            "theirs_sha": content_hash(content),
                        }),
                    ) {
                        // best-effort 전례 동형(cys.rs audit closure) — loud 경고 후 계속.
                        eprintln!("[init-pack] 감사 원장 기록 실패(계속 진행): {e}");
                    }
                    // ② 병합: diffy 순수 Rust(셸아웃 0) — 충돌 마커 = ⑤.
                    let merged = match crate::merge3::merge3(base, ours, content) {
                        crate::merge3::Merge3Outcome::Clean(m) => m,
                        crate::merge3::Merge3Outcome::Conflict(_) => {
                            return Err("merge-conflict".to_string())
                        }
                    };
                    // ③ 게이트: *.json 파스 + 손상 의심 휴리스틱(clean-but-wrong 차단 · 이식 ⑥).
                    if !crate::merge3::json_gate(rel, &merged) {
                        return Err("json-gate".to_string());
                    }
                    if let Some(rsn) = crate::merge3::suspect_damage(base, ours, &merged) {
                        return Err(format!("suspect:{rsn:?}"));
                    }
                    Ok(merged)
                })();
                match gates {
                    Ok(merged) => {
                        // ④ 기록(성공): disk←merged → pristine←embed(검증형 — 실패는 다음 릴리스
                        // base 소실이므로 ⑤ 강등) → manifest←hash(embed) → 원장 merged → continue.
                        if let Some(parent) = path.parent() {
                            std::fs::create_dir_all(parent)
                                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                        }
                        write_atomic(&path, merged.as_bytes())
                            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
                        match ensure_pristine_checked(&dir, rel, content) {
                            Ok(()) => {
                                manifest.insert(rel.to_string(), content_hash(content));
                                if pending
                                    .get(rel)
                                    .and_then(|e| e.get("kind"))
                                    .and_then(|k| k.as_str())
                                    == Some("new-pending")
                                {
                                    pending.remove(rel);
                                    pending_dirty = true;
                                    let _ = std::fs::remove_file(dir.join(format!("{rel}.new")));
                                }
                                let capture_rel =
                                    format!("{}/{rel}", capture_seg.as_ref().unwrap().1);
                                if let serde_json::Value::Object(entry) = serde_json::json!({
                                    "kind": "merged", "side": rel, "capture": capture_rel,
                                    "version": target_version, "ts": now_ts,
                                }) {
                                    upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                                }
                                // merged 도 이 스윕이 쓴 파일이다(written+kept=처리 파일 수 불변식).
                                // 병합 건수 가시화는 원장 kind 계상(W-E2/부트 요약)이 담당.
                                written += 1;
                                continue;
                            }
                            Err(e) => {
                                eprintln!(
                                    "[init-pack] ⚠ pristine 전진 실패({rel}): {e} — healed 폴백 강등(디스크에는 병합본 — .user 소스=병합본 · ours 원본은 캡처 보존)"
                                );
                                fallback_reason = "pristine-write-failed".to_string();
                                fallback_user_src = Some(merged);
                            }
                        }
                    }
                    Err(reason) => fallback_reason = reason,
                }
                // ⑤ healed 폴백 — 현행 Write{heal:true} 경로 동형 낙하 + 충돌 계보 사이드카.
                //   .user 소스 = 그 시점의 디스크 실체(통상 ours=디스크 원본 · pristine-write-failed
                //   만 이미 기록된 merged — v2 §4 크래시표 2행 정합 · ours 원본은 ①캡처가 별도 보존).
                let user_src: Option<String> = fallback_user_src.or_else(|| disk.clone());
                if let Some(d) = user_src.as_deref() {
                    let user_path = dir.join(format!("{rel}.user"));
                    if std::fs::read_to_string(&user_path).ok().as_deref() != Some(d) {
                        match write_atomic(&user_path, d.as_bytes()) {
                            Ok(()) => {}
                            Err(e) if fallback_reason == "capture-failed" => {
                                // ★성찰 차단 수리 2R-5(v2 §2 원칙 4 — 백업 불가면 덮지 않는다):
                                // capture-failed 레인은 ①캡처가 없어 .user 가 유일 보존본인데 그
                                // 쓰기 실패가 best-effort 로 침묵 통과되면 아래 공용 레인의 vendor
                                // 기록이 사용자 바이트를 완전 소실시킨다(이중 실패 = 유일한 무캡처
                                // 손실 경로). L1 바이트 백업과 동일하게 fail-closed: vendor 기록·
                                // manifest 전진·pristine 전진 전부 스킵 + quarantined 전이(D11
                                // 수렴 기전 재사용 — manifest 미전진이라 백업 가능 회복 후 재스윕
                                // 시 동일 Merge3 판정 재도달 = 수렴 · 회복 전까지 파일 무접촉).
                                eprintln!("[init-pack] ⚠ 격리(quarantined): {rel} — 병합 캡처·.user 백업 이중 실패({e}). 파일 무접촉 보존 — 백업 가능 회복 후 재스윕 시 정상 병합 대상.");
                                if let serde_json::Value::Object(entry) = serde_json::json!({
                                    "kind": "quarantined", "reason": "user-backup-failed", "side": rel,
                                    "version": target_version, "ts": now_ts,
                                }) {
                                    upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                                }
                                kept += 1;
                                continue;
                            }
                            Err(e) => {
                                // 캡처 실재 레인(충돌·게이트·pristine 실패): 사용자 바이트는 ①캡처가
                                // 이미 보존하므로 .user 는 이차 사본 — 종전 best-effort 유지하되
                                // 침묵은 금지(v2 §2 원칙 5 · loud 경고 — 원장 side 는 기록되므로
                                // doctor·사용자가 .user 부재를 관측·추적 가능).
                                eprintln!("[init-pack] ⚠ {rel}: .user 백업 기록 실패({e}) — 사용자본은 병합 캡처에 보존(원장 capture 경로 참조)");
                            }
                        }
                    }
                    // 충돌 조상 사이드카 <rel>.base — base 전문 기록(사후 3-way 재료 영구 보존 ·
                    // 이식 ① C안 요소 · backup_set side_paths 등재로 rollback 원자성 편입).
                    if let Some(b) = verified_base.as_deref() {
                        let base_path = dir.join(format!("{rel}.base"));
                        if std::fs::read_to_string(&base_path).ok().as_deref() != Some(b) {
                            let _ = write_atomic(&base_path, b.as_bytes());
                        }
                    }
                    if let serde_json::Value::Object(entry) = serde_json::json!({
                        "kind": "conflicted", "side": format!("{rel}.user"),
                        "base_side": format!("{rel}.base"),
                        "reason": fallback_reason,
                        "version": target_version, "ts": now_ts,
                    }) {
                        upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                    }
                }
            }
            FileAction::Write { heal_user_copy } => {
                if heal_user_copy {
                    // system 강제 치유(P0-4)·force 갱신이 사용자 수정본을 덮기 **전에** 보존(파괴 0).
                    if let Some(d) = disk.as_deref() {
                        let user_path = dir.join(format!("{rel}.user"));
                        if std::fs::read_to_string(&user_path).ok().as_deref() != Some(d) {
                            let _ = write_atomic(&user_path, d.as_bytes());
                        }
                        // ★T3(D7): healed upsert 는 계보 인지 헬퍼 경유 — 크래시 창 재스윕 healed 가
                        // 영속된 merged 항목의 capture 포인터를 지우는 창의 봉인(v2 §4 공통 적용 규칙).
                        if let serde_json::Value::Object(entry) = serde_json::json!({
                            "kind": "healed", "side": format!("{rel}.user"),
                            "version": target_version, "ts": now_ts,
                        }) {
                            upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                        }
                    } else if exists {
                        // ★T3 L0/L1 실행부(v2 §3 · §2 원칙 4 · D4·D11): 판독 불가(disk=None) —
                        // 문자열 백업이 구조적으로 못 뜨던 유일 무백업 파괴 edge 를 바이트 백업
                        // (tmp+copy+rename 원자화)으로 봉인한다.
                        match byte_backup_user(&dir, rel, &path) {
                            Ok(()) => {
                                if let serde_json::Value::Object(entry) = serde_json::json!({
                                    "kind": "healed", "side": format!("{rel}.user"),
                                    "version": target_version, "ts": now_ts,
                                }) {
                                    upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                                }
                            }
                            Err(e) if system_locked(rel) => {
                                // ★L0(v2 §3): 보안 자산은 백업 실패에도 치유 강행(무결성>보존 —
                                // §2 원칙 4 의 명시 예외). 원장 side 는 **실백업 성공 시에만**
                                // "{rel}.user" — 실패는 side 없이 backup:"failed" 기록(존재하지
                                // 않는 .user 를 '보존됨'으로 오보하지 않는다 · D11).
                                eprintln!("[init-pack] ⚠ 보안 자산 {rel}: 바이트 백업 실패({e}) — 무결성 우선으로 치유 강행(v2 §2 원칙 4 예외)");
                                if let serde_json::Value::Object(entry) = serde_json::json!({
                                    "kind": "healed", "backup": "failed",
                                    "version": target_version, "ts": now_ts,
                                }) {
                                    upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                                }
                            }
                            Err(e) => {
                                // ★D11 quarantined 전이(fail-closed): 백업 없이는 덮지 않는다 —
                                // write/manifest.insert/ensure_pristine/written 증가 전부 스킵.
                                // manifest 미전진 = 재스윕 L1 재도달(수렴 조건 — 전진시키면 자동갱신
                                // fast-path 가 손상본을 비수정으로 오판할 수 있다).
                                eprintln!("[init-pack] ⚠ 격리(quarantined): {rel} — 판독 불가 + 백업 실패({e}). 파일 무접촉 보존 — UTF-8 회복(또는 백업 가능 회복) 후 재스윕 시 정상 치유 대상.");
                                if let serde_json::Value::Object(entry) = serde_json::json!({
                                    "kind": "quarantined", "reason": "backup-failed", "side": rel,
                                    "version": target_version, "ts": now_ts,
                                }) {
                                    upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                                }
                                unreadable_kept.push(rel.to_string());
                                kept += 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        write_atomic(&path, content.as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        manifest.insert(rel.to_string(), content_hash(content));
        ensure_pristine(&dir, rel, content);
        // 정상 갱신으로 합류(비수정 update·신규 생성) — 남은 new-pending 잔재는 무의미하므로 청소.
        if pending
            .get(rel)
            .and_then(|e| e.get("kind"))
            .and_then(|k| k.as_str())
            == Some("new-pending")
        {
            pending.remove(rel);
            pending_dirty = true;
            let _ = std::fs::remove_file(dir.join(format!("{rel}.new")));
        }
        written += 1;
    }
    if pending_dirty {
        save_merge_pending(&dir, &pending);
    }
    // ★W-1(P0): 판독 불가라 손대지 않은 파일은 **같은 실행의 출력에 반드시 드러난다.** 침묵하면
    // 사용자는 "덮어썼는지 아닌지"를 알 길이 없고, 아래 병합 대기 수치를 전량으로 오독한다.
    if !unreadable_kept.is_empty() {
        println!(
            "[init-pack] 읽을 수 없어 손대지 않은 파일 {}건 — 내용이 UTF-8 이 아니거나(한국어 Windows 의 \
             CP949/ANSI 저장 등) 권한·잠금으로 읽기 실패. 덮어쓰기·백업 **둘 다 하지 않았고** 파일은 그대로입니다: {}\n\
             \x20 조치: 해당 파일을 UTF-8 로 다시 저장하면 다음 설치부터 정상 비교·병합 대상이 됩니다.",
            unreadable_kept.len(),
            unreadable_kept.join(", ")
        );
    }
    if !pending.is_empty() {
        // ★W-E2 · ★T3(D13): 사용자 언어 요약 — kind 별 **명시** 계상(빼기 산식 전면 개정: 구
        // `new_n = len - healed_n` 은 kept-drift/merged/conflicted/quarantined 를 전부 '.new 병치'
        // 로 오보했다 — 성찰 4렌즈 공통 실측). at-rest kind(kept-drift·merged)는 '병합 대기'가
        // 아니라 보존 상태다 — 아래 부트 요약 1줄(v2 §5 채널 1)이 분리 보고한다.
        let c = pending_kind_counts(&pending);
        // ★W-1: 판독 불가 보존분은 원장에 없다 — 위 수치가 '전량'으로 읽히지 않게 같은 줄에 덧붙인다.
        let unreadable_note = if unreadable_kept.is_empty() {
            String::new()
        } else {
            format!(" · 판독 불가라 손대지 않음 {}건(위 안내)", unreadable_kept.len())
        };
        if c.healed + c.new_pending > 0 {
            println!(
                "[init-pack] 내 커스텀은 지워지지 않았습니다 — 병합 대기 {}건 (내 수정본 .user 보존 {}건 · vendor 신버전 .new 병치 {}건){}\n\
                 \x20 검토·병합: `cys pack-merge` · 직전 상태 파일 복원: `cys pack-rollback`",
                c.healed + c.new_pending, c.healed, c.new_pending, unreadable_note
            );
        }
        // ★T3(D13 · v2 §5 가시성 채널 1): 부트 요약 1줄 — 어느 하나라도 0 이 아닐 때만.
        if c.healed + c.merged + c.kept_drift + c.conflicted + c.quarantined > 0 {
            println!(
                "[init-pack] pack: healed={} merged={} kept-drift={} conflicted={} quarantined={} — cys pack-merge 로 확인",
                c.healed, c.merged, c.kept_drift, c.conflicted, c.quarantined
            );
        }
    }
    // prune: 임베드에서 사라진 옛 파일(폐기 스킬·디렉티브)을 제거해 '기능 제거 배포'를 가능케 한다.
    // 비수정(설치-당시 해시 == 현재 디스크 해시)만 삭제하고, 사용자 수정본·*_DIRECTIVE.md는 보존(안전측).
    // embed 목록이 비정상적으로 비면(빌드 이상) 전량 삭제 재앙을 막기 위해 prune을 건너뛴다.
    {
        let embedded: std::collections::HashSet<&str> =
            items.iter().map(|(rel, _)| *rel).collect();
        if !embedded.is_empty() {
            let stale: Vec<String> = manifest
                .keys()
                .filter(|rel| !embedded.contains(rel.as_str()))
                .cloned()
                .collect();
            let mut pruned = 0;
            for rel in stale {
                if ownership_scoped(&rel, scope) != Ownership::System {
                    continue; // ★B2: user 소유·seed-once 상태는 영구 보존 — prune 대상 제외(스코프 인지)
                }
                let path = dir.join(&rel);
                match std::fs::read_to_string(&path) {
                    // 비수정(설치-당시 해시 == 디스크 해시) → 제거 + 매니페스트에서 삭제.
                    Ok(existing)
                        if manifest.get(&rel).map(String::as_str)
                            == Some(content_hash(&existing).as_str()) =>
                    {
                        if std::fs::remove_file(&path).is_ok() {
                            manifest.remove(&rel);
                            pruned += 1;
                        }
                    }
                    Ok(_) => {} // 사용자 수정본 → 보존(매니페스트 유지)
                    Err(_) => {
                        manifest.remove(&rel); // 파일 이미 없음 → 매니페스트만 정리
                    }
                }
            }
            if pruned > 0 {
                eprintln!("[init-pack] pruned {pruned} stale (removed) file(s)");
            }
        }
    }
    // 매니페스트 영속:
    // - transactional=false(embed/cysd/init-pack): 최선노력 — 직렬화·write 실패해도 설치 자체는
    //   유효하고 다음 판정은 보존(안전측)으로 떨어진다(외부 동작 불변).
    // - transactional=true(pack-update): fail-closed — write 실패를 Err로 승격해
    //   apply_pack_transactional이 rollback_journal를 타게 한다. 매니페스트가 손상/구상태로 남으면
    //   다음 update preserve-gate가 새 파일을 사용자 수정본으로 오판(자동갱신·prune 차단)하기 때문
    //   (R2CODE2 HIGH #1). 매니페스트 bytes는 apply_pack_transactional backup_set에 포함돼 rollback
    //   대상이다.
    match serde_json::to_string_pretty(&manifest) {
        Ok(json) => {
            let res = write_atomic(&manifest_path, json.as_bytes());
            if transactional {
                res.map_err(|e| format!("cannot write {}: {e}", manifest_path.display()))?;
            } else {
                let _ = res;
            }
        }
        Err(e) => {
            if transactional {
                return Err(format!("cannot serialize manifest: {e}"));
            }
        }
    }
    // 팩 버전 기록 — 다음 install의 다운그레이드 판정 기준(target_version으로 갱신).
    // ★pack-update 트랜잭션(transactional=true)은 여기서 쓰지 않는다 — apply_pack_transactional이
    // 마지막 hard commit marker로 직접(검사) 기록한다.
    // v5 checked 쓰기 순서(R4 codex 결착 — 구 best-effort는 state 동기와 결합 시 불일치 유발):
    // `.pack-version` checked 먼저(실패 = loud + state 미갱신 = 불일치 미생성) → 성공 후에만
    // `.pack-state.json` 동기(존재 시 {free, target, 0} — v4 자체 발견: 오탐 동결 차단).
    if !transactional {
        match write_atomic(&dir.join(PACK_VERSION_FILE), target_version.as_bytes()) {
            Ok(()) => {
                if let PackStateRead::Valid(mut st) = read_pack_state(&dir) {
                    if st.channel == "free" && st.base_version != target_version {
                        st.base_version = target_version.to_string();
                        st.pro_revision = 0;
                        if let Err(e) = write_pack_state(&dir, &st) {
                            eprintln!(
                                "[init-pack] ⚠ .pack-state.json 동기 실패(다음 기동 자가치유로 수렴): {e}"
                            );
                        }
                    }
                }
            }
            Err(e) => eprintln!(
                "[init-pack] ⚠ .pack-version 기록 실패 — state 미갱신(불일치 미생성): {e}"
            ),
        }
    }
    // cys 전용 CLAUDE_CONFIG_DIR 격리 셋업(오너 2026-06-15) — 사용자 ~/.claude 오염으로부터
    // cys 마스터를 분리한다. best-effort·보존 모드라 깨끗한 환경에서도 회귀 0.
    // ★staging 경로(install_staged)는 setup_config=false로 여기서 건너뛰고, atomic swap 후 실
    // pack_dir에 대해 한 번 셋업한다(격리 config는 pack_dir 형제라 staging 대상이 아님).
    if setup_config {
        // 훅 등록 억제(--no-install-hook)는 staged 경로(install_staged)만의 관심사 — 이 인라인
        // 경로(cysd 자동설치·install_from_iter)는 항상 완전 시드다(거동 불변·설계 확정).
        setup_isolated_config_dir(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 실행권한은 임베드 내용의 shebang으로 결정한다 — 고정 목록은 스킬 스크립트
        // 추가 시 드리프트(fs::write가 exec 비트를 만들지 않아 직접 실행 스킬·hook
        // 등록이 신규 머신에서 깨짐)의 원천이었다. kept 파일에도 적용해 기존 설치본을
        // 복구한다.
        for (rel, content) in items.iter().copied() {
            if !content.starts_with("#!") {
                continue;
            }
            let p = dir.join(rel);
            if p.exists() {
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755));
            }
        }
    }
    Ok((written, kept))
}

/// install_into의 공개 얇은 래퍼 — 실 pack_dir() 대상, config 격리 셋업 포함(외부 동작 완전 불변).
/// C/D/E 호출처(install·apply_pack_transactional)의 기존 시그니처를 보존한다(§3 하위호환).
pub fn install_from_iter<'a, I: IntoIterator<Item = (&'a str, &'a str)>>(
    items: I,
    force: bool,
    target_version: &str,
    transactional: bool,
    auth: Option<PackWriteAuth>,
) -> Result<(usize, usize), String> {
    let dir = pack_dir();
    // ★G3 축2: 스코프는 실 pack_dir 에서 산출(부서 데몬 자동설치가 이 경로 — dept soul 승계·불가침).
    let scope = pack_scope_of(&dir);
    install_into(dir, items, force, target_version, transactional, true, scope, auth, None)
}

// ─────────────────────────────────────────────────────────────────────────────
// 팩 atomic swap (v3 §3.1) — init-pack의 파일별 in-place write(중단 시 반쯤 쓰인 팩 =
// stale-packfile 버그 클래스)를 staging 전개→검증→원자 rename 교체로 대체한다.
// ★pack-update는 이미 journal 트랜잭션(apply_pack_transactional)으로 all-or-nothing +
// minisign·sha256 검증을 수행하므로 이 경로를 타지 않는다(중복 래핑=heavily-reviewed 트랜잭션
// 재작성 위험 → 외과성 원칙 준수). run_init_pack(비원자 in-place write)만 이 경로로 승격한다.
// ─────────────────────────────────────────────────────────────────────────────

/// init-pack staging 디렉터리(pack_dir 형제·pid로 격리). pack-update의 고정 `.pack-staging`과
/// 이름을 분리해 동시 실행 충돌을 피한다(doctor가 `.pack-staging*` 잔재를 정리한다).
pub fn init_staging_dir(dir: &Path) -> PathBuf {
    let parent = dir.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!(".pack-staging-init-{}", std::process::id()))
}

/// 1세대 롤백 보존 디렉터리(pack_dir 형제 `<pack_dir>.prev` — 즉시 롤백 근거).
pub fn pack_prev_dir(dir: &Path) -> PathBuf {
    PathBuf::from(format!("{}.prev", dir.display()))
}

/// 재귀 디렉터리 복사(파일=fs::copy로 권한 보존, 하위 dir 재귀). 팩엔 심링크가 없다(오너 결정 —
/// 심링크 마이그레이션 안 함). staging 전량 복사로 상태파일·user-edit·비임베드·디렉티브를 보존한다.
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// cross-device 대비 rename(§3.1-5) — 같은 볼륨이면 원자 rename, 실패 시 copy 후 원본 삭제
/// fallback(EXDEV 등). staging은 pack_dir 형제라 정상 경로는 원자 rename이다(Windows도 동일 볼륨 전제).
fn rename_dir_or_move(src: &Path, dst: &Path) -> std::io::Result<()> {
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    // rename 불가(cross-device 등) — copy 후 원본 삭제 fallback(src 부재면 여기서 loud Err).
    copy_dir_all(src, dst)?;
    std::fs::remove_dir_all(src)
}

/// 원자 교체(§3.1-3): pack_dir→pack_dir.prev, staging→pack_dir. 2번째 rename 실패 시 역rename으로
/// pre-state 복구. pack_dir.prev는 1세대만 보존. 반환 Err = 교체 안 됨(기존 팩 온전).
///
/// L6 전제 명문화: 두 rename 사이엔 pack_dir가 잠깐 **부재하는 창**이 있다(원자 교체지만 순간 공백).
/// 이는 **데몬 미가동/init 시점**(팩을 읽는 상주 소비자가 없는 때)을 전제로 안전하다 — 무중단
/// 업데이트 경로(deploy_gate --execute)는 이 함수를 데몬이 팩을 읽지 않는 시점에만 호출한다.
/// 상주 데몬이 그 창에 팩을 읽으면 일시적 not-found가 날 수 있으므로, 라이브 교체는 이 전제를
/// 지키는 호출자 책임이다(코드 변경 불요·전제 고지).
pub fn atomic_swap(dir: &Path, staging: &Path) -> Result<(), String> {
    let prev = pack_prev_dir(dir);
    // 직전 세대 정리(1세대 보존).
    let _ = std::fs::remove_dir_all(&prev);
    let had_old = dir.exists();
    if had_old {
        rename_dir_or_move(dir, &prev)
            .map_err(|e| format!("pack_dir→prev rename 실패(교체 안 함): {e}"))?;
    }
    match rename_dir_or_move(staging, dir) {
        Ok(()) => Ok(()),
        Err(e) => {
            // 역rename 복구: (실패한 fallback이 만든 부분/빈 dir 정리 후) prev→pack_dir 복원.
            if had_old {
                let _ = std::fs::remove_dir_all(dir);
                let _ = rename_dir_or_move(&prev, dir);
            }
            Err(format!("staging→pack_dir rename 실패(pre-state 복구 시도): {e}"))
        }
    }
}

/// staging 검증(§3.1-2): 임베드 전 파일이 staging에 실재하는가(파일 수·존재). pack-update의
/// sha256·minisign 검증은 pack-update 경로(packsig)가 이미 수행하므로, init-pack staging은
/// 존재·수 검증이다(디스크 오류로 반쯤 쓰인 staging을 교체 전에 차단하는 방어선).
pub fn verify_staging(staging: &Path, items: &[(&str, &str)]) -> Result<(), String> {
    let mut missing = 0usize;
    let mut first: Option<String> = None;
    for (rel, _) in items {
        if !staging.join(rel).is_file() {
            missing += 1;
            if first.is_none() {
                first = Some((*rel).to_string());
            }
        }
    }
    if missing == 0 {
        Ok(())
    } else {
        Err(format!(
            "staging 검증 실패: 임베드 {}개 중 {}개 누락(예: {}) — 교체 중단",
            items.len(),
            missing,
            first.unwrap_or_default()
        ))
    }
}

/// 원자 교체 기반 init-pack 설치(§3.1). 현재 pack_dir을 staging에 전량 복사→install_into로 임베드
/// 반영(preserve-gate·prune·.pack-version)→검증→원자 rename 교체→실 pack_dir에 config 격리 셋업.
/// 중단(카피·반영·검증 중 abort)은 기존 pack_dir을 건드리지 않는다(원자성). 반환: (written, kept).
/// ★G3: `install_hooks=false`(init-pack --no-install-hook)면 마지막 config 격리 셋업에서 훅
/// 병합·검증·개인 프로필 병합을 생략한다(라우터 시드는 유지) — 훅 억제의 의미론을 '모든 계급의
/// 훅 등록 억제'로 통일(종전엔 ~/.claude 만 막고 격리 config dir 병합은 못 막던 비일관 해소).
pub fn install_staged(
    force: bool,
    auth: Option<PackWriteAuth>,
    install_hooks: bool,
) -> Result<(usize, usize), String> {
    let dir = pack_dir();
    // ★G3 축2: 스코프는 staging basename(`.pack-staging-init-*` — 부서 정보 없음)이 아니라
    // **논리 대상**(pack_dir)에서 산출해 데이터로 주입한다 — 부서 팩의 init-pack 도 staged 경로를
    // 타므로, 여기서 산출하지 않으면 dept soul 이 Base(User) 로 오판돼 결함4(.new 병치)가 재발한다.
    let scope = pack_scope_of(&dir);
    // ★캡처 레인도 스코프와 동일 계약(v2 §4 ①): 물리 staging basename 이 아니라 논리 대상
    // (pack_dir) basename 을 데이터로 주입 — 캡처가 `.pack-staging-init-<pid>` 은닉·휘발 레인에
    // 생성되는 것을 차단한다(원장 capture 포인터·실파일·doctor 계상 전부 논리 레인으로 수렴).
    let capture_ns = dir.file_name().map(|n| n.to_string_lossy().to_string());
    // ★W0-d: 최종 commit/rename(atomic_swap)이 라이브 기본 경로를 원자 교체하므로, 어떤 staging
    // 작업보다 먼저 인가를 검사한다(비라이브 대상·인가 보유는 통과 — 테스트는 temp 대상이라 무영향).
    authorize_pack_write(&dir, auth)?;
    let staging = init_staging_dir(&dir);
    // 잔여 staging(같은 pid 재사용·직전 실패) 선정리.
    let _ = std::fs::remove_dir_all(&staging);
    // ① 기존 팩 전량을 staging에 복사(상태파일·user-edit·비임베드·디렉티브 전부 보존 — 완전 교체 대상).
    if dir.exists() {
        copy_dir_all(&dir, &staging)
            .map_err(|e| format!("staging 복사 실패 {}: {e}", staging.display()))?;
    } else {
        std::fs::create_dir_all(&staging)
            .map_err(|e| format!("staging 생성 실패 {}: {e}", staging.display()))?;
    }
    // ② 임베드 반영을 staging에(config 격리 셋업은 교체 후 실 dir에 — setup_config=false).
    let items: Vec<(&str, &str)> = PACK_ALL.iter().map(|(r, c)| (*r, *c)).collect();
    let (written, kept) = match install_into(
        staging.clone(),
        items.iter().copied(),
        force,
        env!("CARGO_PKG_VERSION"),
        false,
        false,
        scope, // 논리 대상(pack_dir)의 스코프 — staging basename 재판정 금지(위 주석).
        None, // staging은 비라이브 형제 경로 — 여기 쓰기는 인가 불요(라이브 인가는 위 swap 게이트가 담당).
        capture_ns.as_deref(), // 캡처 레인 = 논리 대상 basename(스코프와 동일 주입 계약).
    ) {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    // ③ 검증(존재·수) — 실패 시 staging 폐기·교체 안 함(기존 팩 온전).
    if let Err(e) = verify_staging(&staging, &items) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    // ④ 원자 교체(실패 시 pre-state 복구·staging 정리).
    if let Err(e) = atomic_swap(&dir, &staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    // ⑤ 교체 후 실 pack_dir 기준 config 격리 셋업(pack_dir 형제 — staging 대상이 아니었다).
    setup_isolated_config_dir(install_hooks);
    Ok((written, kept))
}

// ─────────────────────────────────────────────────────────────────────────────
// 무중단 pack-update 적용 트랜잭션(§7-⑤ 옵션 b — 오너 결정 ⑤ 확정: 심링크 마이그레이션 안 함).
// backup journal + rollback + `.pack-version` = 마지막 hard commit marker로 전체 팩 적용에
// all-or-nothing(부분적용 0)을 부여한다. ★install()/cysd 자동설치·init-pack 경로는 이 트랜잭션을
// 거치지 않는다(install_from_iter를 transactional=false로 직접 호출 — 외부 동작 불변).
// pack-update만 apply_pack_transactional로 감싼다. R2CODE HIGH #1 해소.
// ─────────────────────────────────────────────────────────────────────────────

const PACK_JOURNAL_DIR: &str = ".pack-journal";

/// 백업 저널 디렉터리(~/.cys/.pack-journal) — pack_dir 형제(staging·lock·accepted와 동일 루트).
pub fn pack_journal_dir() -> PathBuf {
    pack_dir()
        .parent()
        .map(|p| p.join(PACK_JOURNAL_DIR))
        .unwrap_or_else(|| PathBuf::from(PACK_JOURNAL_DIR))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JournalEntry {
    rel: String,
    /// apply 전 파일이 존재했는가. false면 rollback 시 (신규 생성분) 삭제.
    existed: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JournalIndex {
    /// 이번 트랜잭션의 목표 pack_version(= 커밋 성공 시 `.pack-version`에 기록되는 값).
    /// recovery는 디스크 `.pack-version`이 이 값과 같은지로 커밋 완료를 판정한다.
    target_version: String,
    entries: Vec<JournalEntry>,
}

/// apply 전 backup journal 작성: backup_set의 각 파일 기존 bytes를 저널에 복사(+fsync)하고
/// 인덱스(목표 버전·existed 플래그)를 기록(+fsync)한다. 잔존 저널은 먼저 비운다.
fn write_journal(
    target_version: &str,
    backup_set: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let jdir = pack_journal_dir();
    let _ = std::fs::remove_dir_all(&jdir);
    let files_dir = jdir.join("files");
    std::fs::create_dir_all(&files_dir)
        .map_err(|e| format!("journal files dir 생성 실패 {}: {e}", files_dir.display()))?;
    let dir = pack_dir();
    let mut entries = Vec::new();
    for rel in backup_set {
        let src = dir.join(rel);
        if src.is_file() {
            let bytes = std::fs::read(&src)
                .map_err(|e| format!("journal 백업 읽기 실패 {}: {e}", src.display()))?;
            let dst = files_dir.join(rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("journal 백업 dir 실패 {}: {e}", parent.display()))?;
            }
            write_atomic(&dst, &bytes)
                .map_err(|e| format!("journal 백업 쓰기 실패 {}: {e}", dst.display()))?;
            entries.push(JournalEntry { rel: rel.clone(), existed: true });
        } else {
            entries.push(JournalEntry { rel: rel.clone(), existed: false });
        }
    }
    let index = JournalIndex {
        target_version: target_version.to_string(),
        entries,
    };
    let json =
        serde_json::to_vec_pretty(&index).map_err(|e| format!("journal 인덱스 직렬화 실패: {e}"))?;
    // 인덱스는 마지막에(원자) — 인덱스 부재 = '백업 미완 = 미커밋'(원본 미변경)을 의미.
    write_atomic(&jdir.join("index.json"), &json)
        .map_err(|e| format!("journal 인덱스 쓰기 실패: {e}"))?;
    Ok(())
}

/// 저널에서 pre-state로 복원: existed=true는 백업 bytes를 원위치 atomic 복원, existed=false는
/// (신규 생성분) 삭제. `.pack-version`은 저널에 없으므로 손대지 않는다(미커밋 = old 유지). 복원
/// 후 저널 삭제. ★커밋 마커(.pack-version==target)가 아닐 때만 호출(recover_pack_journal이 판정).
pub fn rollback_journal() -> Result<(), String> {
    let jdir = pack_journal_dir();
    let index_path = jdir.join("index.json");
    let s = std::fs::read_to_string(&index_path)
        .map_err(|e| format!("journal 인덱스 읽기 실패 {}: {e}", index_path.display()))?;
    let index: JournalIndex =
        serde_json::from_str(&s).map_err(|e| format!("journal 인덱스 파싱 실패: {e}"))?;
    let dir = pack_dir();
    let files_dir = jdir.join("files");
    for entry in &index.entries {
        let target = dir.join(&entry.rel);
        if entry.existed {
            let backup = files_dir.join(&entry.rel);
            let bytes = std::fs::read(&backup)
                .map_err(|e| format!("journal 백업 복원 읽기 실패 {}: {e}", backup.display()))?;
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            write_atomic(&target, &bytes)
                .map_err(|e| format!("journal 복원 쓰기 실패 {}: {e}", target.display()))?;
        } else {
            match std::fs::remove_file(&target) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                // 백업 시 파일이 아니라 디렉터리였던 경로(예: 손상돼 디렉터리가 된
                // .install-manifest.json)는 bytes 백업 불가라 existed=false로 기록된다. remove_file은
                // 디렉터리에 실패하므로 remove_dir_all로 손상물을 정리해 rollback이 중단 없이
                // pre-state(손상물 부재=안전측)로 수렴하게 한다(R2CODE2 HIGH #1 fail-closed 경로).
                Err(_) if target.is_dir() => {
                    std::fs::remove_dir_all(&target).map_err(|e| {
                        format!("journal 신규 디렉터리 삭제 실패 {}: {e}", target.display())
                    })?;
                }
                Err(e) => {
                    return Err(format!("journal 신규파일 삭제 실패 {}: {e}", target.display()))
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&jdir);
    Ok(())
}

/// crash recovery(§7-⑤): orphan 저널을 발견하면 `.pack-version`(= hard commit marker)을 저널의
/// 목표 버전과 대조한다. 같으면 커밋은 성공했고 저널 정리 중 crash였으므로 저널만 삭제(롤백 금지).
/// 다르면 미커밋(부분적용)이므로 rollback으로 pre-state 자가치유. 인덱스 부재(백업 도중 crash)는
/// 원본 미변경이므로 잔존 저널만 폐기. 저널 완전 부재면 no-op. 반환: 복구를 수행했으면 true.
/// ★pack-update 착수 시·cysd 기동 시(install(false) 전)에 호출해 부분적용을 선치유한다.
pub fn recover_pack_journal() -> Result<bool, String> {
    let jdir = pack_journal_dir();
    let index_path = jdir.join("index.json");
    if !index_path.is_file() {
        // 인덱스 없는 잔존 디렉터리 = 백업 미완(원본 미변경) → 통째 폐기.
        if jdir.exists() {
            let _ = std::fs::remove_dir_all(&jdir);
            return Ok(true);
        }
        return Ok(false);
    }
    let s = std::fs::read_to_string(&index_path)
        .map_err(|e| format!("journal 인덱스 읽기 실패 {}: {e}", index_path.display()))?;
    let index: JournalIndex =
        serde_json::from_str(&s).map_err(|e| format!("journal 인덱스 파싱 실패(손상): {e}"))?;
    let disk_version = std::fs::read_to_string(pack_dir().join(PACK_VERSION_FILE))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if !disk_version.is_empty() && disk_version == index.target_version {
        // 커밋 성공(.pack-version == target) → 저널 정리만(롤백 금지).
        let _ = std::fs::remove_dir_all(&jdir);
    } else {
        // 미커밋 → 롤백(pre-state 복원 + 저널 삭제).
        rollback_journal()?;
    }
    Ok(true)
}

/// 무중단 pack-update 적용 트랜잭션(§7-⑤ 옵션 b + free/pro v4 §3 상태 계약). 호출 전제:
/// apply-lock 보유(writer 배타).
/// 순서: ⓪orphan 저널 자가치유 → ①backup journal(변경·삭제 대상 + `.pack-state.json` 포함) →
/// ②install_from_iter(파일 반영, `.pack-version` 미기록) → ③`.pack-state.json` 기록(journal
/// 편입 — 실패 시 rollback) → ④`.pack-version` = 마지막 hard commit marker(결과 검사) →
/// ⑤post_commit(record_accepted — ★커밋 **이후**: R3 codex blocking 결착. 실패해도 rollback
/// 없음 — 낡은 accepted는 안전 방향(버전 게이트·신선도 창이 방어)이며 self-heal이 수렴.
/// loud + 반환 bool로 구분 보고) → ⑥저널 삭제.
/// ③까지 실패 시 rollback(pre-state 복원·부분적용 0). 반환: (written, kept, post_commit_ok).
pub fn apply_pack_transactional<F>(
    items: &[(&str, &str)],
    target_version: &str,
    state: &PackState,
    auth: Option<PackWriteAuth>,
    post_commit: F,
) -> Result<(usize, usize, bool), String>
where
    F: FnOnce() -> Result<(), String>,
{
    // ⓪ 직전 crash로 남은 orphan 저널 자가치유(새 트랜잭션 전 pre-state 확정).
    recover_pack_journal()?;
    let dir = pack_dir();
    // ★W0-d: 저널·파일 반영 어떤 부수효과보다 먼저 라이브 인가 검사(비라이브·인가 보유는 통과).
    authorize_pack_write(&dir, auth)?;
    // ① backup set = 새 manifest.files(=items) ∪ 현재 install-manifest 키(prune·overwrite 대상)
    //    ∪ .install-manifest.json ∪ .pack-state.json(v4 — rollback이 state도 pre-state로 복원).
    let mut backup_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (rel, _) in items {
        backup_set.insert((*rel).to_string());
    }
    if let Ok(s) = std::fs::read_to_string(dir.join(INSTALL_MANIFEST)) {
        if let Ok(m) = serde_json::from_str::<std::collections::BTreeMap<String, String>>(&s) {
            for k in m.keys() {
                backup_set.insert(k.clone());
            }
        }
    }
    backup_set.insert(INSTALL_MANIFEST.to_string());
    backup_set.insert(PACK_STATE_FILE.to_string());
    // ★커스터마이즈 절충 부수효과(.pristine/·.new·.user·병합 원장)도 저널 편입 — rollback이
    // pre-state 를 **글자 단위로** 복원한다는 기존 계약(mid_apply_fault 테스트)을 부수효과까지 확장.
    // 대부분 부재 경로라 저널 증가는 미미하다(write_journal 은 존재/부재를 그대로 스냅샷).
    let side_paths: Vec<String> = backup_set
        .iter()
        .filter(|rel| !rel.starts_with('.')) // 마커·매니페스트·state 파일 자신은 제외
        .flat_map(|rel| {
            [
                format!("{}/{rel}", PRISTINE_DIR),
                format!("{rel}.new"),
                format!("{rel}.user"),
                format!("{rel}.base"), // ★T3(v2 §4 ⑤): 충돌 조상 사이드카 — rollback 원자성 편입
            ]
        })
        .collect();
    backup_set.extend(side_paths);
    backup_set.insert(MERGE_PENDING_FILE.to_string());
    // ★MERGE_AUDIT_FILE 은 의도적 비등재 — append-only 감사 원장은 rollback 을 생존한다(G3-축3).
    write_journal(target_version, &backup_set)?;
    // ② 파일 반영(transactional=true) — .pack-version은 여기서 쓰지 않고(④에서 commit marker로),
    //    .install-manifest.json write 실패는 fail-closed로 Err가 되어 아래 rollback을 탄다.
    let (written, kept) =
        match install_from_iter(items.iter().copied(), false, target_version, true, auth) {
            Ok(v) => v,
            Err(e) => {
                let _ = rollback_journal();
                return Err(format!("파일 반영 실패(rollback 완료): {e}"));
            }
        };
    // ③ `.pack-state.json` 기록 — journal 백업 대상이므로 실패 시 rollback으로 전체 복원.
    if let Err(e) = write_pack_state(&dir, state) {
        let _ = rollback_journal();
        return Err(format!("state 기록 실패(rollback 완료): {e}"));
    }
    // ④ .pack-version = 마지막 hard commit marker(결과 검사 — best-effort 금지).
    if let Err(e) = write_atomic(&dir.join(PACK_VERSION_FILE), target_version.as_bytes()) {
        let _ = rollback_journal();
        return Err(format!(".pack-version 커밋 실패(rollback 완료): {e}"));
    }
    // ⑤ post-commit(record_accepted) — 커밋은 이미 유효. 실패 = loud + false 반환(침묵 포장 금지).
    let post_commit_ok = match post_commit() {
        Ok(()) => true,
        Err(e) => {
            eprintln!(
                "[pack-update] ⚠ post-commit accepted 기록 실패 — 디스크 반영은 성공(롤백 없음). \
                 replay 기준선이 낡음(안전 방향) → 다음 pack-update self-heal이 수렴: {e}"
            );
            false
        }
    };
    // ⑥ 커밋 성공 → 저널 삭제.
    let _ = std::fs::remove_dir_all(pack_journal_dir());
    Ok((written, kept, post_commit_ok))
}

pub fn role_directive_path(role: &str) -> Option<PathBuf> {
    // 접두 일치: reviewer-gemini / worker-2 같은 변형 역할도 표준 지침을 받는다
    let file = match role {
        "master" => "MASTER_DIRECTIVE.md",
        r if r.starts_with("worker") => "WORKER_DIRECTIVE.md",
        r if r.starts_with("cso") => "CSO_DIRECTIVE.md",
        r if r.starts_with("reviewer") => "REVIEWER_DIRECTIVE.md",
        _ => return None,
    };
    Some(pack_dir().join("directives").join(file))
}

/// ★W0-b RAII EnvGuard(테스트 지원) — 환경변수를 **이전 값 복원형**으로 임시 설정/제거한다.
/// drop 시(정상 종료·**패닉 언와인딩 포함**) 이전 값으로 되돌리므로, 구 teardown(`remove_var`)이
/// 남기던 "env가 비는 창"(다음 테스트가 라이브 pack_dir을 오판·오염하던 벡터)을 구조적으로 없앤다.
/// 프로덕션 코드는 이걸 쓰지 않는다(테스트 전용). lib·bin 테스트가 공유하도록 `pub`이다.
#[doc(hidden)]
pub struct EnvGuard {
    key: String,
    prev: Option<String>,
}

impl EnvGuard {
    /// key를 val로 설정하고 이전 값을 기억한다(drop 시 복원).
    pub fn set(key: &str, val: impl AsRef<std::ffi::OsStr>) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, val);
        EnvGuard { key: key.to_string(), prev }
    }
    /// key를 제거하고 이전 값을 기억한다(drop 시 복원).
    pub fn remove(key: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        EnvGuard { key: key.to_string(), prev }
    }
    /// Option<String>(예: 다른 곳에서 캡처한 이전 값)으로 설정/제거한다.
    pub fn set_opt(key: &str, val: Option<impl AsRef<std::ffi::OsStr>>) -> Self {
        match val {
            Some(v) => EnvGuard::set(key, v),
            None => EnvGuard::remove(key),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(&self.key, v),
            None => std::env::remove_var(&self.key),
        }
    }
}

/// pack_dir()이 읽는 전역 env 키(ENV_PACK_DIR)의 set/remove 윈도를 직렬화하는 테스트 락.
/// pack.rs·overrides.rs 테스트가 같은 lib 테스트 바이너리에서 ENV_PACK_DIR을 공유하므로
/// 한 락으로 직렬화해야 프로세스 전역 env 경합(flaky)을 막는다 (R4 패턴의 모듈 간 공유).
#[cfg(test)]
pub(crate) static PACK_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// 역할 → 디렉티브 파일명만 검증 (pack_dir 절대경로는 env 의존이라 비교하지 않음).
    fn dir_file(role: &str) -> Option<String> {
        role_directive_path(role)
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
    }

    /// [G3 축2 회귀 핀 셈] 기존 판정 핀 전량을 **무수정 초록**으로 유지하는 Base 스코프 셈 —
    /// 로컬 fn 이 glob import(super::decide_file_action)를 가리므로 기존 6-인자 호출이 전부 이
    /// 셈을 통해 Base 등급표로 돈다 = `ownership(rel)==ownership_scoped(rel, Base)` 항등과
    /// "base 레인 거동 byte-identical" 의 기계 증거. dept 스코프 검증은 super:: 경로로 명시 호출.
    /// ★T2: Base+무base 셈 — 종전 6인자 핀 전량이 L4/조기분기 등가로 무수정 관통(verified_base=None).
    fn decide_file_action(
        rel: &str,
        embed: &str,
        exists: bool,
        disk: Option<&str>,
        manifest_hash: Option<&str>,
        force: bool,
    ) -> FileAction {
        super::decide_file_action(rel, embed, exists, disk, manifest_hash, force, PackScope::Base, None)
    }

    /// [G3 회귀 핀 셈] 기존 install_staged 핀 전량을 무수정 초록으로 유지 — install_hooks=true
    /// 가 종전 거동(훅 병합 포함)의 박제다. --no-install-hook 경로는 no_install_hook_consistency
    /// 가 super:: 경로로 명시 검증.
    fn install_staged(force: bool, auth: Option<PackWriteAuth>) -> Result<(usize, usize), String> {
        super::install_staged(force, auth, true)
    }

    /// ★T-0147-1 실증 검체(W3) — **목 개인 프로필**에 각성 훅을 병합해 3단 단언한다:
    ///   ①사용자 기존 항목 **보존** ②우리 훅 **존재** ③재실행 **멱등**(중복 0·백업 무접촉).
    ///
    /// 왜 이 검체가 필수인가: 오너는 `~/.claude` 불가침을 **훅 병합에 한해** 의식적으로 완화했다.
    /// 그 완화가 안전한 이유는 병합기가 '추가만' 한다는 계약 하나뿐이므로, 그 계약이 깨지면
    /// 사용자 설정 파괴로 직결된다 — 계약을 코드가 아니라 **테스트가** 지킨다.
    #[test]
    fn awakening_hook_merge_preserves_user_entries_and_is_idempotent() {
        let td = std::env::temp_dir().join(format!("cys-t01471-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let pack = td.join("pack");
        let settings = td.join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        // 사용자 기존 설정: 우리와 무관한 훅 + 우리와 같은 이벤트의 남의 훅 + 비-훅 키
        let user = serde_json::json!({
            "theme": "dark",
            "permissions": {"allow": ["Bash(ls)"]},
            "hooks": {
                "SessionStart": [{"hooks": [{"type": "command", "command": "sh /home/u/myhooks/session-start.sh"}]}],
                "Stop": [{"hooks": [{"type": "command", "command": "sh /home/u/mytool/stop.sh"}]}]
            }
        });
        std::fs::write(&settings, serde_json::to_string_pretty(&user).unwrap()).unwrap();

        // ── 1차 병합 ──
        let added = merge_desired_hooks(&settings, &pack, &AWAKENING_HOOKS).unwrap();
        assert_eq!(added.len(), 2, "각성 2훅이 추가되지 않았다: {added:?}");
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        // ① 사용자 항목 보존(비-훅 키·남의 훅·다른 이벤트 전부)
        assert_eq!(after["theme"], "dark", "사용자 비-훅 키 소실");
        assert_eq!(after["permissions"]["allow"][0], "Bash(ls)", "사용자 권한 설정 소실");
        let ss = after["hooks"]["SessionStart"].as_array().unwrap();
        assert!(
            ss.iter().any(|e| e["hooks"][0]["command"] == "sh /home/u/myhooks/session-start.sh"),
            "같은 이벤트의 사용자 훅이 삭제됐다(추가만 계약 위반)"
        );
        assert_eq!(
            after["hooks"]["Stop"][0]["hooks"][0]["command"],
            "sh /home/u/mytool/stop.sh",
            "다른 이벤트의 사용자 훅 소실"
        );
        // ② 우리 훅 존재(등록 집합 ⊇ 소망 집합)
        assert!(
            verify_desired_hooks_registered(&settings, &pack, &AWAKENING_HOOKS).is_empty(),
            "병합 후에도 소망 집합이 충족되지 않았다"
        );
        // ③ 재실행 멱등 — 추가 0 · 중복 0 · 백업 무접촉(정상 백업 클로버 금지)
        let backup = format!("{}.bak-cys", settings.display());
        std::fs::write(&backup, "{\"_sentinel\":\"keep\"}").unwrap();
        let again = merge_desired_hooks(&settings, &pack, &AWAKENING_HOOKS).unwrap();
        assert!(again.is_empty(), "멱등 위반 — 재실행이 또 추가했다: {again:?}");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "{\"_sentinel\":\"keep\"}",
            "무변경 재실행이 정상 백업을 클로버했다"
        );
        let after2: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        for h in AWAKENING_HOOKS.iter() {
            let want = hook_command_for(&pack, h.script);
            let n = after2["hooks"][h.event]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|e| {
                            e["hooks"]
                                .as_array()
                                .map(|hs| hs.iter().any(|x| x["command"] == want.as_str()))
                                .unwrap_or(false)
                        })
                        .count()
                })
                .unwrap_or(0);
            assert_eq!(n, 1, "{} 훅이 {n}회 등록됐다(중복 append)", h.script);
        }
        // ④ 파싱 불가 파일은 **거부**(빈 객체로 덮어쓰지 않는다 — 침묵 데이터 소실 차단)
        let broken = td.join(".claude-broken").join("settings.json");
        std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
        std::fs::write(&broken, "{ not json").unwrap();
        assert!(
            merge_desired_hooks(&broken, &pack, &AWAKENING_HOOKS).is_err(),
            "파싱 실패 파일을 덮어썼다(사용자 설정 소실 경로)"
        );
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), "{ not json", "거부인데 파일이 변경됐다");
        // ⑤ 부재 파일은 생성한다(G7 빈 프로필 배선의 전제)
        let fresh = td.join(".claude-fresh").join("settings.json");
        std::fs::create_dir_all(fresh.parent().unwrap()).unwrap();
        assert_eq!(
            merge_desired_hooks(&fresh, &pack, &AWAKENING_HOOKS).unwrap().len(),
            2,
            "부재 파일에 각성 2훅이 생성되지 않았다"
        );
        assert!(verify_desired_hooks_registered(&fresh, &pack, &AWAKENING_HOOKS).is_empty());
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★U-21 · 선언 timeout 축의 **순수 진리표**.
    ///
    /// 이 표가 지키는 것은 하나다 — "선언은 하한이지 동등이 아니다". 동등으로 읽으면 사용자가
    /// 더 크게 잡아둔 값을 우리가 **내리고**, 그 순간 살아서 완주하던 부트가 잘린다(오살).
    #[test]
    fn hook_timeout_declaration_is_a_floor_not_an_equality() {
        // 선언 없음 = 아무것도 단언하지 않는다(종전 판정과 동일).
        assert!(hook_timeout_satisfied(None, None));
        assert!(hook_timeout_satisfied(Some(1), None));
        // 선언 있음: 미기록은 **불충족**(하네스 기본 30s 를 우리가 읽을 길이 없다).
        assert!(!hook_timeout_satisfied(None, Some(600)));
        // 저값 불충족 · 동값 충족 · **고값도 충족**(내리지 않는다).
        assert!(!hook_timeout_satisfied(Some(30), Some(600)));
        assert!(hook_timeout_satisfied(Some(600), Some(600)));
        assert!(hook_timeout_satisfied(Some(900), Some(600)));
        // 선언값이 하네스 기본을 실제로 넘는가 — 넘지 못하면 이 단위는 아무것도 고치지 않은 것이다.
        assert!(
            HOOK_TIMEOUT_ROLE_BOOTSTRAP_S > HOOK_TIMEOUT_PLATFORM_DEFAULT_UPS_S,
            "선언 timeout 이 하네스 기본값 이하다 — 부트 체인 무음 취소가 그대로 남는다"
        );
        // 각성 매니페스트가 실제로 그 선언을 싣고 있는가(리터럴 회귀 핀).
        let ups = AWAKENING_HOOKS
            .iter()
            .find(|h| h.event == "UserPromptSubmit")
            .expect("UserPromptSubmit 각성 훅이 사라졌다");
        assert_eq!(ups.timeout, Some(HOOK_TIMEOUT_ROLE_BOOTSTRAP_S));
    }

    /// ★U-21 · 롤백 스위치는 **마스터 한 손잡이**로 닫힌다(사고 순간 조합 금지).
    #[test]
    fn hook_timeout_axis_folds_into_master_switch() {
        // 기본(미설정) = 신동작.
        assert!(!hook_timeout_axis_legacy_from(None, None));
        // 축 전용 노브.
        assert!(hook_timeout_axis_legacy_from(None, Some("1")));
        assert!(!hook_timeout_axis_legacy_from(None, Some("0")));
        // ★마스터 하나로 축이 종전으로 돌아간다.
        assert!(hook_timeout_axis_legacy_from(Some("0"), None));
        // 엄격 비교 — 형제 축과 같은 규약("false"·"" 같은 값으로 꺼지지 않는다).
        assert!(!hook_timeout_axis_legacy_from(Some("false"), Some("yes")));
        assert_eq!(ENV_HOOK_TIMEOUT_V1, "CYS_HOOK_TIMEOUT_V1");
    }

    /// ★U-21 · **불일치 엔트리 교체 경로** — 명령은 같고 timeout 만 미달일 때.
    ///
    /// 이 테스트가 잡는 실패 두 가지가 정확히 이 제품이 낸 사고다:
    ///   ① append 로 처리 → 같은 명령 2회 등재 → **매 프롬프트 훅 2회 발화**(큐 폭주 방향)
    ///   ② 엔트리 통째 교체 → 같은 배열의 **사용자 항목 소실**(오살)
    #[test]
    fn timeout_mismatch_is_reconciled_in_place_not_appended() {
        let td = std::env::temp_dir().join(format!("cys-u21-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let pack = td.join("pack");
        let settings = td.join("settings.json");
        let ups = AWAKENING_HOOKS
            .iter()
            .find(|h| h.event == "UserPromptSubmit")
            .unwrap();
        let ours = hook_command_for(&pack, ups.script);

        // 기존 디스크 상태 = 우리 훅이 **timeout 없이** 실려 있고, 옆에 사용자 훅이 있다.
        std::fs::write(
            &settings,
            serde_json::to_string_pretty(&serde_json::json!({
                "hooks": {
                    "UserPromptSubmit": [
                        {"hooks": [{"type": "command", "command": "sh /home/u/mine.sh"}]},
                        {"hooks": [{"type": "command", "command": ours}]}
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let added = merge_desired_hooks(&settings, &pack, &AWAKENING_HOOKS).unwrap();
        assert!(
            added.iter().any(|a| a.contains("timeout")),
            "timeout 미달을 교정하지 않았다: {added:?}"
        );
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let arr = after["hooks"]["UserPromptSubmit"].as_array().unwrap();
        // ① 중복 append 0 — 우리 명령은 여전히 정확히 1회.
        let n = arr
            .iter()
            .filter(|e| {
                e["hooks"]
                    .as_array()
                    .map(|hs| hs.iter().any(|x| x["command"] == ours.as_str()))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(n, 1, "교체가 아니라 중복 append 됐다(훅 2회 발화 = 폭주 방향)");
        // ② 사용자 항목 보존.
        assert!(
            arr.iter().any(|e| {
                e["hooks"]
                    .as_array()
                    .map(|hs| hs.iter().any(|x| x["command"] == "sh /home/u/mine.sh"))
                    .unwrap_or(false)
            }),
            "사용자 훅이 사라졌다(오살)"
        );
        // ③ 값이 실제로 선언값으로 올라갔고, 재실행은 멱등이다.
        assert!(verify_desired_hooks_registered(&settings, &pack, &AWAKENING_HOOKS).is_empty());
        assert!(
            merge_desired_hooks(&settings, &pack, &AWAKENING_HOOKS)
                .unwrap()
                .is_empty(),
            "교정 후 재실행이 또 썼다(멱등 위반)"
        );

        // ④ ★사용자가 **더 크게** 잡아둔 값은 내리지 않는다 — 한 방향으로만 여는 축.
        let hi = td.join("settings-hi.json");
        std::fs::write(
            &hi,
            serde_json::to_string_pretty(&serde_json::json!({
                "hooks": {"UserPromptSubmit": [
                    {"hooks": [{"type": "command", "command": ours, "timeout": 900}]}
                ]}
            }))
            .unwrap(),
        )
        .unwrap();
        let added_hi = merge_desired_hooks(&hi, &pack, &AWAKENING_HOOKS).unwrap();
        assert!(
            !added_hi.iter().any(|a| a.contains("UserPromptSubmit")),
            "더 큰 사용자 값을 우리 값으로 내렸다(오살 방향): {added_hi:?}"
        );
        let after_hi: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&hi).unwrap()).unwrap();
        assert_eq!(
            after_hi["hooks"]["UserPromptSubmit"][0]["hooks"][0]["timeout"],
            serde_json::json!(900)
        );
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★U-29(M-09-a) · **재조정은 설치가 아니다** — `--no-install-hook` 경로가 쓰는 얇은 집행기.
    ///
    /// 이 테스트가 잡는 실패 넷이 곧 이 함수가 넘지 말아야 할 선이다:
    ///   ① 파일이 없는데 **만든다**(= 훅 설치 · 회귀 핀 `no_install_hook_consistency` 파괴)
    ///   ② 등록이 없는 이벤트에 **엔트리를 추가한다**(= 훅 설치)
    ///   ③ 사용자가 더 크게 잡은 값을 **내린다**(오살 방향)
    ///   ④ 만질 것이 없는데 **쓴다**(업데이트마다 재직렬화·정상 백업 클로버)
    #[test]
    fn retune_raises_registered_timeouts_without_ever_installing() {
        let td = std::env::temp_dir().join(format!("cys-u29-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let pack = td.join("pack");
        let ups = AWAKENING_HOOKS
            .iter()
            .find(|h| h.event == "UserPromptSubmit")
            .unwrap();
        let ours = hook_command_for(&pack, ups.script);
        let want = ups.timeout.unwrap();

        // ① 파일 부재 = 무동작·무생성.
        let absent = td.join("absent.json");
        assert!(retune_registered_hook_timeouts(&absent, &pack, &AWAKENING_HOOKS)
            .unwrap()
            .is_empty());
        assert!(!absent.exists(), "등록부가 없는데 파일을 만들었다(= 훅 설치)");

        // ② 우리 훅이 **아닌** 것만 있는 등록부 = 무동작·무추가·무쓰기.
        let foreign = td.join("foreign.json");
        let foreign_body = serde_json::to_string_pretty(&serde_json::json!({
            "hooks": {"UserPromptSubmit": [
                {"hooks": [{"type": "command", "command": "sh /home/u/mine.sh"}]}
            ]}
        }))
        .unwrap();
        std::fs::write(&foreign, &foreign_body).unwrap();
        assert!(retune_registered_hook_timeouts(&foreign, &pack, &AWAKENING_HOOKS)
            .unwrap()
            .is_empty());
        assert_eq!(
            std::fs::read_to_string(&foreign).unwrap(),
            foreign_body,
            "우리 훅이 없는 등록부를 재직렬화했다(무변경이어야 한다)"
        );
        assert!(
            !td.join("foreign.json.bak-cys").exists(),
            "만질 것이 없는데 백업을 만들었다(정상 백업 클로버 경로)"
        );

        // ③ 우리 훅이 timeout 없이 등록된 등록부 = 그 키 하나만 오른다(사용자 항목 보존).
        let skew = td.join("skew.json");
        std::fs::write(
            &skew,
            serde_json::to_string_pretty(&serde_json::json!({
                "theme": "dark",
                "hooks": {
                    "UserPromptSubmit": [
                        {"hooks": [{"type": "command", "command": "sh /home/u/mine.sh"}]},
                        {"hooks": [{"type": "command", "command": ours}]}
                    ],
                    "Stop": [{"hooks": [{"type": "command", "command": "sh /home/u/stop.sh"}]}]
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let raised = retune_registered_hook_timeouts(&skew, &pack, &AWAKENING_HOOKS).unwrap();
        assert_eq!(raised.len(), 1, "재조정 항목이 1건이 아니다: {raised:?}");
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&skew).unwrap()).unwrap();
        let arr = after["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "엔트리를 추가했다(= 훅 설치): {arr:?}");
        assert_eq!(after["theme"], "dark", "사용자 비-훅 키 소실");
        assert_eq!(
            after["hooks"]["Stop"][0]["hooks"][0]["command"],
            "sh /home/u/stop.sh",
            "다른 이벤트의 사용자 훅 소실"
        );
        assert_eq!(arr[0]["hooks"][0]["command"], "sh /home/u/mine.sh");
        assert!(
            arr[0]["hooks"][0].get("timeout").is_none(),
            "남의 훅에 timeout 을 심었다(계약 위반)"
        );
        assert_eq!(arr[1]["hooks"][0]["timeout"], serde_json::json!(want));
        // ⓐ 그 훅 **하나**는 이제 집행 축의 판정을 충족한다(같은 술어로 확인).
        assert!(
            verify_desired_hooks_registered(&skew, &pack, std::slice::from_ref(ups)).is_empty(),
            "재조정 후에도 집행 축 판정이 미충족이다"
        );
        // ⓐ′ ★그러나 **소망 집합 전체를 충족시키지는 않는다** — 등록이 없던 SessionStart 는
        //     여전히 없다. 이것이 '재조정'과 '설치'의 경계이며, 여기서 미충족이 사라지면
        //     이 함수는 몰래 훅을 설치하고 있는 것이다.
        assert!(
            after["hooks"].get("SessionStart").is_none(),
            "등록이 없던 이벤트를 만들었다(= 훅 설치): {after}"
        );
        assert_eq!(
            verify_desired_hooks_registered(&skew, &pack, &AWAKENING_HOOKS).len(),
            1,
            "재조정이 소망 집합 전체를 채웠다 — 설치를 하고 있다"
        );
        // ⓑ 멱등 — 재실행은 만질 것이 없으므로 쓰지 않는다(백업 sentinel 무접촉으로 실측).
        let backup = format!("{}.bak-cys", skew.display());
        std::fs::write(&backup, "{\"_sentinel\":\"keep\"}").unwrap();
        assert!(retune_registered_hook_timeouts(&skew, &pack, &AWAKENING_HOOKS)
            .unwrap()
            .is_empty());
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "{\"_sentinel\":\"keep\"}",
            "멱등 재실행이 정상 백업을 덮었다"
        );

        // ④ 사용자가 더 크게 잡은 값은 내리지 않는다(한 방향으로만 여는 축).
        let hi = td.join("hi.json");
        std::fs::write(
            &hi,
            serde_json::to_string_pretty(&serde_json::json!({
                "hooks": {"UserPromptSubmit": [
                    {"hooks": [{"type": "command", "command": ours, "timeout": 900}]}
                ]}
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(retune_registered_hook_timeouts(&hi, &pack, &AWAKENING_HOOKS)
            .unwrap()
            .is_empty());
        let after_hi: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&hi).unwrap()).unwrap();
        assert_eq!(
            after_hi["hooks"]["UserPromptSubmit"][0]["hooks"][0]["timeout"],
            serde_json::json!(900),
            "더 큰 사용자 값을 내렸다(오살 방향)"
        );

        // ⑤ 파손 등록부는 **거부**한다(빈 객체로 덮어쓰기 = 훅 등록부 전소).
        let broken = td.join("broken.json");
        std::fs::write(&broken, "{ this is not json").unwrap();
        assert!(retune_registered_hook_timeouts(&broken, &pack, &AWAKENING_HOOKS).is_err());
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), "{ this is not json");

        let _ = std::fs::remove_dir_all(&td);
    }

    /// 개인 프로필 병합의 **범위 게이트** — 부서/임시 팩에서는 개인 프로필을 건드리지 않는다.
    /// (테스트 빌드는 항상 no-op 이라 실 HOME 무접촉이 구조적으로 보장된다.)
    #[test]
    fn personal_profile_merge_is_scoped_and_test_safe() {
        assert!(
            merge_awakening_hooks_into_personal_profiles().is_empty(),
            "테스트 빌드에서 개인 프로필 병합이 동작했다(실 HOME 오염 위험)"
        );
    }

    #[test]
    fn role_directive_exact_master() {
        // master는 정확 일치만 — 'masterful' 같은 변형은 매핑 없음.
        // ★W0-a: dir_file("master")는 pack_dir()을 경유하므로, env를 전역 제거하는 테스트
        // (pack_dir_env_precedence·w0a panic)와 직렬화해 미설정 창에서의 오탐 panic을 막는다.
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        assert_eq!(dir_file("master").as_deref(), Some("MASTER_DIRECTIVE.md"));
        assert_eq!(dir_file("masterful"), None);
    }

    /// ★부트 스윕 게이트 회귀 핀 — 게이트는 "확실히 최신"일 때만 닫힌다(치유 방향 fail-open).
    /// 안전 방향이 remote_is_newer(fail-CLOSED)와 반대임을 박제한다: 마커 부재·파싱 실패·
    /// 매니페스트 부재는 전부 false(=스윕 실행)여야 한다.
    #[test]
    fn pack_current_gate_only_closes_when_provably_current() {
        let td = std::env::temp_dir().join(format!("cys-pack-current-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();

        // ① 완전 부재(신선 머신) → 스윕
        assert!(!pack_current_in(&td, "0.12.51"), "마커·매니페스트 부재 = 스윕");
        // ② 마커만 있고 매니페스트 부재(부분 손상) → 스윕
        std::fs::write(td.join(PACK_VERSION_FILE), "0.12.51").unwrap();
        assert!(!pack_current_in(&td, "0.12.51"), "매니페스트 부재 = 스윕");
        // ③ 동일 버전 + 매니페스트 실재 → 게이트 닫힘(평시 부트)
        std::fs::write(td.join(INSTALL_MANIFEST), "{}").unwrap();
        assert!(pack_current_in(&td, "0.12.51"), "동일 버전 = 스킵");
        // ④ 개행·공백 trim(python 도구 CRLF 재직렬화 대비 — 7-12 원복 사고 계열)
        std::fs::write(td.join(PACK_VERSION_FILE), "0.12.51\r\n").unwrap();
        assert!(pack_current_in(&td, "0.12.51"), "CRLF trim 후 동일 버전 = 스킵");
        // ⑤ 디스크 구버전(바이너리 업그레이드 직후) → 스윕
        std::fs::write(td.join(PACK_VERSION_FILE), "0.12.50").unwrap();
        assert!(!pack_current_in(&td, "0.12.51"), "디스크 구버전 = 스윕");
        // ⑥ 디스크 전진(pack-update 스큐) → 스킵(스윕해봐야 다운그레이드 차단 = 동치·저렴)
        std::fs::write(td.join(PACK_VERSION_FILE), "0.12.52").unwrap();
        assert!(pack_current_in(&td, "0.12.51"), "디스크 전진 = 스킵");
        // ⑦ 마커 손상(비semver) → 스윕(fail-open 치유)
        std::fs::write(td.join(PACK_VERSION_FILE), "garbage").unwrap();
        assert!(!pack_current_in(&td, "0.12.51"), "마커 손상 = 스윕");

        let _ = std::fs::remove_dir_all(&td);
    }

    #[test]
    fn session_start_hook_command_is_os_aware_shared() {
        // RC-2 회귀 핀: 격리 config dir·init-pack 두 경로가 공유하는 공용 함수.
        let cmd = session_start_hook_command(Path::new("/pack"));
        assert!(
            cmd.contains("hooks/session-start.sh") || cmd.contains("hooks\\session-start.sh"),
            "must target bundled hook: {cmd:?}"
        );
        let interp = cmd.split_whitespace().next().unwrap_or("");
        assert!(interp == "sh" || interp == "bash", "shell interpreter only: {interp:?}");
        #[cfg(unix)]
        assert_eq!(cmd, "sh /pack/hooks/session-start.sh", "unix 제로 회귀");
        #[cfg(windows)]
        {
            assert!(cmd.starts_with("bash \""), "windows must use quoted bash: {cmd:?}");
            assert!(!cmd.contains('\\'), "windows 경로 정슬래시 정규화(RC-3 회귀 핀): {cmd:?}");
        }
    }

    #[test]
    fn session_start_hook_command_quotes_windows_space_path() {
        // RC-2 잔여(T2.1): 공백 포함 pack 경로 — Windows는 quote(공백 깨짐 방지), unix는 무변경
        // (기존 등록 문자열과 already 매칭 유지 → 중복 등록 방지).
        let cmd = session_start_hook_command(Path::new("/pack dir/x"));
        assert!(cmd.contains("session-start.sh"), "hook 스크립트 대상: {cmd:?}");
        #[cfg(not(windows))]
        assert_eq!(cmd, "sh /pack dir/x/hooks/session-start.sh", "unix 무변경(quote 없음)");
        #[cfg(windows)]
        {
            assert!(cmd.starts_with("bash \""), "windows 공백경로 quote 시작: {cmd:?}");
            assert!(cmd.ends_with('"'), "windows quote 종료: {cmd:?}");
            assert!(cmd.contains("pack dir"), "공백 경로 보존: {cmd:?}");
        }
    }

    #[test]
    fn role_bootstrap_hook_command_is_os_aware_shared() {
        // R2-LOW-B 회귀 핀(session_start 동형): 격리 config 시드와 preflight C28(_cys_hook_cmd)이
        // 공유하는 문자열 — byte-identical이 깨지면 매 기동 중복 append(RC1 matcher 불일치)가 재발한다.
        let cmd = role_bootstrap_hook_command(Path::new("/pack"));
        assert!(
            cmd.contains("hooks/role-bootstrap.sh") || cmd.contains("hooks\\role-bootstrap.sh"),
            "must target bundled hook: {cmd:?}"
        );
        let interp = cmd.split_whitespace().next().unwrap_or("");
        assert!(interp == "sh" || interp == "bash", "shell interpreter only: {interp:?}");
        #[cfg(unix)]
        assert_eq!(cmd, "sh /pack/hooks/role-bootstrap.sh", "unix byte-pin(제로 회귀)");
        #[cfg(windows)]
        {
            assert!(cmd.starts_with("bash \""), "windows must use quoted bash: {cmd:?}");
            assert!(!cmd.contains('\\'), "windows 경로 정슬래시 정규화: {cmd:?}");
        }
    }

    #[test]
    fn role_directive_prefix_variants_map_to_standard() {
        // 접두 일치: 변형 역할(worker-2·reviewer-gemini·cso-1)도 표준 지침을 받는다
        // — 디렉티브 주입(각성)이 변형 역할에서 누락되지 않게 하는 핵심 불변식.
        // ★W0-a: dir_file는 pack_dir() 경유 — env 전역 제거 테스트와 직렬화(미설정 창 오탐 panic 방지).
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        for (role, file) in [
            ("worker", "WORKER_DIRECTIVE.md"),
            ("worker-2", "WORKER_DIRECTIVE.md"),
            ("workerbee", "WORKER_DIRECTIVE.md"),
            ("cso", "CSO_DIRECTIVE.md"),
            ("cso-1", "CSO_DIRECTIVE.md"),
            ("reviewer", "REVIEWER_DIRECTIVE.md"),
            ("reviewer-gemini", "REVIEWER_DIRECTIVE.md"),
            ("reviewer-codex", "REVIEWER_DIRECTIVE.md"),
        ] {
            assert_eq!(dir_file(role).as_deref(), Some(file), "role={role}");
        }
    }

    #[test]
    fn role_directive_unknown_and_empty_are_none() {
        // 미지의 역할·빈 문자열은 None (잘못된 지침 주입 방지)
        assert_eq!(dir_file(""), None);
        assert_eq!(dir_file("gemini"), None);
        assert_eq!(dir_file("admin"), None);
        // 대소문자 민감 — 'Worker'는 'worker' 접두에 불일치
        assert_eq!(dir_file("Worker"), None);
    }

    #[test]
    fn role_directive_path_is_under_directives_dir() {
        // 경로 구조: <pack_dir>/directives/<FILE> — 부모 디렉터리가 'directives'
        // ★W0-a: pack_dir() 경유 — env 전역 제거 테스트와 직렬화(미설정 창 오탐 panic 방지).
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        let p = role_directive_path("master").unwrap();
        assert_eq!(
            p.parent().and_then(|d| d.file_name()).map(|f| f.to_string_lossy().into_owned()),
            Some("directives".to_string())
        );
    }

    // PACK_ENV_LOCK은 모듈 스코프(pub(crate))로 이동 — overrides.rs 테스트와 공유해
    // 같은 lib 바이너리 내 ENV_PACK_DIR 경합을 막는다. `use super::*`로 가시.

    /// ★불변식 박제: build.rs 자동 임베드가 오너 채택 스킬 14종(2026-06-12 k-skill 감사)
    /// + 기본 2종 + harness-creator + work management 2종(절대지침 5차 앵커 4규칙 b·c:
    /// hallucination-guard·grill-me) + 출처 고지를 전부 포함하고, 모든 SKILL.md가
    /// compose_directive의 색인 파서(첫 10줄 name:)에 잡히는 형식이어야 한다 —
    /// 어긋나면 노드 색인에서 누락된다.
    #[test]
    fn pack_skills_embed_adopted_set_and_indexable() {
        let names: Vec<&str> = PACK_ALL.iter().map(|(p, _)| *p).collect();
        for skill in [
            "korean-humanizer", "korean-spell-check", "korean-character-count",
            "naver-blog-research", "kosis-stats", "hwp", "rhwp-edit",
            "joseon-sillok-search", "geeknews-search", "k-dart", "korean-patent-search",
            "korean-stock-search", "daishin-report-search", "library-book-search",
            "skill-writing", "self-correction-loops", "harness-creator",
            "hallucination-guard", "grill-me",
            // superpowers A+B 9종 (2026-06-12 오너 채택 · 핀 6fd4507)
            "systematic-debugging", "test-driven-development",
            "subagent-driven-development", "dispatching-parallel-agents",
            "verification-before-completion", "brainstorming",
            "receiving-code-review", "writing-plans", "using-git-worktrees",
            // mattpocock A+B+집필3 9종 (2026-06-12 오너 채택 · 핀 694fa30)
            "git-guardrails-claude-code", "grill-with-docs", "prototype",
            "improve-codebase-architecture", "zoom-out", "handoff",
            "writing-fragments", "writing-beats", "writing-shape",
        ] {
            let want = format!("skills/{skill}/SKILL.md");
            assert!(names.iter().any(|p| *p == want), "임베드 누락: {skill}");
        }
        // cys-video-creator 영상 자동제작 스킬 32종(오너 제작 · preflight C26 VIDEO_SKILLS와
        // 동기) — pack 임베드로 기본 배포됨을 박제. 새 스킬 추가 시 양쪽을 함께 갱신한다.
        for skill in [
            "youtube-video-pipeline", "suite-runtime-keys", "cost-preview-confirm",
            "script-writer", "script-writer-research", "script-writer-structure",
            "script-writer-factcheck", "script-writer-voice-prep",
            "voice-clone-elevenlabs", "voice-clone-elevenlabs-chunk",
            "voice-clone-elevenlabs-synth-qc",
            "heygen-avatar-render", "heygen-avatar-render-api", "heygen-avatar-render-gate",
            "media-gen", "media-gen-image", "media-gen-edit", "media-gen-video",
            "media-gen-upscale", "media-gen-thumbnail",
            "video-stitch", "video-stitch-compositing", "video-stitch-broll",
            "video-stitch-captions",
            "audio-post", "audio-post-music", "audio-post-mix",
            "video-verify", "video-verify-visual", "video-verify-timing",
            "video-verify-audio-sync", "video-verify-final-gate",
        ] {
            let want = format!("skills/{skill}/SKILL.md");
            assert!(names.iter().any(|p| *p == want), "영상 스킬 임베드 누락: {skill}");
        }
        // appbuild 웹/앱 빌드 스킬 20종(오너 제작 · 워커 필수 · preflight C27 APPBUILD_SKILLS와
        // 동기) — 스펙 기반 기획→감독관 검증→자율빌드. pack 임베드 기본 배포 박제.
        for skill in [
            "appbuild", "appbuild-plan", "appbuild-plan-interview",
            "appbuild-plan-debate", "appbuild-plan-quick",
            "appbuild-screen-spec", "appbuild-screen-spec-flow", "appbuild-screen-spec-detail",
            "appbuild-tasks", "appbuild-tasks-slice", "appbuild-tasks-order",
            "appbuild-supervisor", "appbuild-supervisor-collect", "appbuild-supervisor-verify",
            "appbuild-supervisor-fix", "appbuild-supervisor-gate",
            "appbuild-orchestrate", "appbuild-orchestrate-delegate",
            "appbuild-orchestrate-verify", "appbuild-orchestrate-route",
        ] {
            let want = format!("skills/{skill}/SKILL.md");
            assert!(names.iter().any(|p| *p == want), "appbuild 스킬 임베드 누락: {skill}");
        }
        // appbuild 코드선행 금지 hook이 임베드돼야 C27이 설치·등록할 수 있다.
        let pack_names: Vec<&str> = PACK_ALL.iter().map(|(p, _)| *p).collect();
        assert!(pack_names.contains(&"hooks/appbuild-gate.sh"), "appbuild-gate hook 임베드 누락");
        assert!(names.contains(&"skills/THIRD_PARTY.md"), "외부 유래 출처 고지(MIT) 누락");
        for (path, content) in PACK_ALL.iter() {
            if path.ends_with("/SKILL.md") {
                // 실파서(compose_directive)는 name 값이 비어있으면 색인에서 제외한다 —
                // 존재만 보면 빈 name이 거짓 통과한다(적대 검증 R1).
                let indexable = content
                    .lines()
                    .take(10)
                    .any(|l| l.strip_prefix("name:").is_some_and(|v| !v.trim().is_empty()));
                assert!(indexable, "{path}: 첫 10줄에 유효한 name: 부재 — 스킬 색인에서 누락된다");
            }
        }
    }

    /// ★불변식 박제: 빈 디렉터리(신규 머신)에 install()만으로 코어 pack + 채택 스킬이
    /// 전부 설치된다 — "cysjavis 설치 = 기본 스킬 자동 설치" 계약의 기계 검증.
    #[test]
    fn install_writes_core_and_skills_to_fresh_dir() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!("cys-pack-install-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let cfgdir = td.join("cysclaude"); // 격리 config dir(테스트 밀폐 — td와 함께 정리)
        let _env = set_pack_env(&td, &cfgdir);
        let result = install(false, None);
        let (written, kept) = result.expect("install 실패");
        assert_eq!(kept, 0, "빈 디렉터리인데 kept>0");
        assert_eq!(written, PACK_ALL.len(), "임베드 전수 설치 아님");
        // ★격리 config dir 셋업(오너 2026-06-15): cys 라우터+hook이 전용 dir에 설치되고,
        // 사용자 ~/.claude 와 분리된다. 라우터는 ~/.cys/pack 디렉티브로 라우팅해야 한다.
        let router = std::fs::read_to_string(cfgdir.join("CLAUDE.md")).expect("격리 CLAUDE.md 미설치");
        assert!(router.contains("~/.cys/pack/directives"), "격리 라우터가 pack 디렉티브로 안 보냄");
        assert!(router.contains("cys 터미널 전용"), "격리 라우터에 cys 환경선언 부재");
        let cfg_settings = std::fs::read_to_string(cfgdir.join("settings.json")).expect("격리 settings.json 미설치");
        assert!(cfg_settings.contains("SessionStart") && cfg_settings.contains("session-start.sh"),
                "격리 settings.json에 SessionStart hook 부재");
        // ★RC1(증분2): 초기 시드에 UserPromptSubmit→role-bootstrap.sh 등록(마스터 선언 결정론 발화의
        // preflight-전 초기 창을 닫음). preflight _cys_hook_cmd 와 동일 문자열이라 재등록 시 중복 0.
        assert!(cfg_settings.contains("UserPromptSubmit") && cfg_settings.contains("role-bootstrap.sh"),
                "격리 settings.json에 UserPromptSubmit(role-bootstrap.sh) hook 부재");
        for probe in [
            "skills/korean-humanizer/SKILL.md",
            "skills/kosis-stats/scripts/run_kosis_stats.py",
            "skills/THIRD_PARTY.md",
            "bin/javis_route.py",
            "directives/MASTER_DIRECTIVE.md",
        ] {
            assert!(td.join(probe).is_file(), "설치 누락: {probe}");
        }
        // ★불변식 박제: shebang 임베드 파일은 설치 직후 실행 가능해야 한다 —
        // 스킬이 scripts/x.sh 직접 실행·hook 등록을 전제하므로 exec 비트 소실은
        // 신규 머신에서 해당 기능 전체가 깨지는 결함이다(전수조사 발견 A).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut shebang_seen = 0;
            for (rel, content) in PACK_ALL.iter() {
                if !content.starts_with("#!") {
                    continue;
                }
                shebang_seen += 1;
                let mode = std::fs::metadata(td.join(rel))
                    .unwrap_or_else(|_| panic!("설치 누락: {rel}"))
                    .permissions()
                    .mode();
                assert!(mode & 0o111 != 0, "{rel}: shebang인데 실행권한 없음 (mode={mode:o})");
            }
            // 회귀 가드: 스킬 스크립트가 규칙에 실제로 잡히는지 (bin 6종 + 스킬 7종 이상)
            assert!(shebang_seen >= 13, "shebang 파일이 {shebang_seen}개뿐 — 임베드 누락 의심");
        }
        let _ = std::fs::remove_dir_all(&td);
    }

    /// version_gt: 자릿수 비교·prerelease suffix 분리·fail-CLOSED(파싱 실패 시 보수적 차단).
    #[test]
    fn version_gt_basic_prerelease_and_fail_closed() {
        assert!(version_gt("0.10.0", "0.4.1"), "minor 자릿수");
        assert!(version_gt("0.4.10", "0.4.9"), "patch 자릿수(문자열 비교면 실패)");
        assert!(!version_gt("0.4.1", "0.4.1"), "동일 → false");
        assert!(!version_gt("0.4.0", "0.4.1"), "낮음 → false");
        assert!(version_gt("v0.5.0", "0.4.9"), "'v' 접두");
        // prerelease/build suffix 분리 — 이전 fail-OPEN(10-rc→0)이 뚫렸던 회귀 케이스
        assert!(version_gt("0.4.10-rc", "0.4.9"), "patch 10-rc → 10 > 9");
        assert!(version_gt("0.5.0-rc1", "0.4.9"));
        assert!(version_gt("0.4.0+build", "0.3.9"));
        assert!(!version_gt("0.4.9", "0.4.10-rc"), "역방향");
        // ★fail-CLOSED: 디스크 버전(a) 파싱 실패 → true(보존/차단)
        assert!(version_gt("garbage", "0.4.1"), "비숫자 major → fail-CLOSED");
        assert!(version_gt("", "0.4.1"), "빈 문자열 → fail-CLOSED");
    }

    /// 다운그레이드 차단: 디스크 .pack-version이 embed보다 새것이면 비강제 install이 (0,0)으로
    /// 차단하고 디스크 버전을 보존한다. force는 우회한다.
    #[test]
    fn install_blocks_downgrade_when_disk_version_newer() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!("cys-pack-downgrade-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let _env = set_pack_env(&td, td.join("cysclaude"));

        let embed = env!("CARGO_PKG_VERSION");
        // 1) 정상 설치 → .pack-version = embed 기록
        install(false, None).expect("최초 install 실패");
        let disk_v1 = std::fs::read_to_string(td.join(PACK_VERSION_FILE)).unwrap();
        // 2) 디스크 .pack-version을 더 새 버전으로 위조(구버전 cys 롤백/오설치 시뮬)
        std::fs::write(td.join(PACK_VERSION_FILE), "99.0.0").unwrap();
        // 3) install(false) → 다운그레이드 차단 → (0,0), .pack-version 유지(embed로 안 덮음)
        let blocked = install(false, None).expect("install 실패");
        let disk_after = std::fs::read_to_string(td.join(PACK_VERSION_FILE)).unwrap();
        // 4) force는 우회 → 갱신
        install(true, None).expect("force install 실패");
        let disk_forced = std::fs::read_to_string(td.join(PACK_VERSION_FILE)).unwrap();

        // env 복원(assert 전 — 패닉해도 전역 env 누수 없게)
        let _ = std::fs::remove_dir_all(&td);

        assert_eq!(disk_v1.trim(), embed, "최초 install이 .pack-version을 embed로 기록");
        assert_eq!(blocked, (0, 0), "다운그레이드는 차단되어 (0,0) 반환");
        assert_eq!(disk_after.trim(), "99.0.0", "차단 시 디스크 버전 유지");
        assert_eq!(disk_forced.trim(), embed, "force는 다운그레이드 우회해 embed로 갱신");
    }

    /// ★불변식 박제 + B2 소유권 매니페스트: force=false 업그레이드 의미론.
    /// ① system 비수정 파일(설치-당시 해시 일치) → 임베드 신버전으로 자동 갱신
    /// ② user 파일(soul.md) 수정 → 불가침 보존
    /// ③ ★B2/P0-4: system 파일 매니페스트 부재 + 내용 상이 → **강제 갱신**(과거 보존 동결이 배포 스큐 근원)
    /// ④ user 파일 매니페스트 부재 + 내용 상이 → 보존(안전측)
    /// ⑤ 디스크=임베드인 구설치본 → 매니페스트 채택 기록 + 멱등
    /// ★커스터마이즈 절충(②) 순수 판정: decide_file_action 이 기존 B2/P0-4 분기를 보존하면서
    /// heal_user_copy(치유 전 사용자본 보존)·new_pending(user-owned 신버전 병치)만 추가하는지 박제.
    #[test]
    fn decide_file_action_threeway_matrix() {
        use super::FileAction::*;
        let embed = "EMBED-V2";
        let eh = content_hash(embed);
        // 부재 → 신규 생성(보존 대상 없음).
        assert_eq!(decide_file_action("bin/x.py", embed, false, None, None, false),
                   Write { heal_user_copy: false });
        // 디스크=임베드 → 최신 채택.
        assert_eq!(decide_file_action("bin/x.py", embed, true, Some(embed), None, false),
                   Keep { adopt_hash: true, new_pending: false });
        // system 비수정(매니페스트 해시=디스크) → 자동 갱신(사용자본 보존 불요).
        assert_eq!(decide_file_action("bin/x.py", embed, true, Some("OLD"),
                       Some(content_hash("OLD").as_str()), false),
                   Write { heal_user_copy: false });
        // system 수정본(매니페스트 부재·상이) → 강제 치유(P0-4)하되 사용자본 .user 보존.
        assert_eq!(decide_file_action("bin/x.py", embed, true, Some("HACKED"), None, false),
                   Write { heal_user_copy: true });
        // force 로 system 수정본을 덮을 때도 사용자본 보존.
        assert_eq!(decide_file_action("bin/x.py", embed, true, Some("HACKED"), None, true),
                   Write { heal_user_copy: true });
        // user-owned 수정 + 임베드가 마지막 적용본에서 전진 → 보존 + 신버전 병치(병합 대기).
        assert_eq!(decide_file_action("soul.md", embed, true, Some("MY-SOUL"),
                       Some(content_hash("EMBED-V1").as_str()), false),
                   Keep { adopt_hash: false, new_pending: true });
        // user-owned 수정 + 임베드=마지막 적용본(vendor 무변경) → 보존만(dpkg 동형 — 병치 불요).
        assert_eq!(decide_file_action("soul.md", embed, true, Some("MY-SOUL"),
                       Some(eh.as_str()), false),
                   Keep { adopt_hash: false, new_pending: false });
        // user-owned 는 force 여도 보존(기존 ★B2 계약 불변).
        assert_eq!(decide_file_action("soul.md", embed, true, Some("MY-SOUL"),
                       Some(eh.as_str()), true),
                   Keep { adopt_hash: false, new_pending: false });
    }

    /// ★커스터마이즈 절충(②③④) 통합: .new 병치·.user 보존·.pristine 미러·병합 원장·해소 경로 박제.
    #[test]
    fn install_threeway_sides_pristine_and_pending_lifecycle() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!("cys-pack-threeway-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let _env = set_pack_env(&td, td.join("cysclaude"));

        let get = |rel: &str| PACK_ALL.iter().find(|(r, _)| *r == rel).map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("팩에 {rel} 부재"));
        let sys_a = "README.md";  // system·비수정 → 갱신 + pristine 미러
        let user_b = "soul.md";   // user-owned·수정 + vendor 전진 → 보존 + .new + pending
        // ★W-ACL 이후 acl.json 은 user 등급이라 system 픽스처가 될 수 없다 — 성격이 같은
        // 루트 JSON 설정 중 system 으로 남은 alerts-config.json 으로 대조군을 옮긴다.
        let sys_c = "alerts-config.json"; // system·수정 → 치유 + .user + pending
        std::fs::create_dir_all(&td).unwrap();
        for (rel, stale) in [(sys_a, "OLD-INSTALLED"), (user_b, "USER-SOUL"), (sys_c, "SYS-DRIFT")] {
            std::fs::write(td.join(rel), stale).unwrap();
        }
        // sys_a=비수정 증명(설치-당시 해시), user_b=마지막 적용본이 embed 와 다름(vendor 전진) 증명.
        let manifest = serde_json::json!({
            sys_a: content_hash("OLD-INSTALLED"),
            user_b: content_hash("OLD-SOUL-BASE"),
        });
        std::fs::write(td.join(INSTALL_MANIFEST), manifest.to_string()).unwrap();

        install(false, None).expect("install 실패");
        let read = |rel: &str| std::fs::read_to_string(td.join(rel)).unwrap();

        // ① user-owned: 보존 + 신버전 .new 병치 + 원장 new-pending.
        assert_eq!(read(user_b), "USER-SOUL", "user-owned 보존 불변");
        assert_eq!(read("soul.md.new"), get(user_b), ".new = 임베드 신버전");
        // ② system 수정본: 치유(임베드) + 사용자본 .user 보존 + 원장 healed.
        assert_eq!(read(sys_c), get(sys_c), "system 치유(P0-4 불변)");
        assert_eq!(read(&format!("{sys_c}.user")), "SYS-DRIFT", "치유 전 사용자본 보존(파괴 0)");
        let pending = load_merge_pending(&td);
        assert_eq!(pending.get(user_b).and_then(|e| e["kind"].as_str()), Some("new-pending"));
        assert_eq!(pending.get(sys_c).and_then(|e| e["kind"].as_str()), Some("healed"));
        // ③ pristine 미러: 적용된 vendor 본만(sys_a·sys_c), 동결 user-owned(user_b)는 미기록(조상 보존).
        assert_eq!(read(&format!("{PRISTINE_DIR}/{sys_a}")), get(sys_a));
        assert_eq!(read(&format!("{PRISTINE_DIR}/{sys_c}")), get(sys_c));
        assert!(!td.join(PRISTINE_DIR).join(user_b).exists(), "동결 파일 조상은 미갱신");
        // ④ 멱등: 재실행해도 상태 동일(원장 중복 기록·불필요 rewrite 없음).
        install(false, None).expect("재실행 실패");
        assert_eq!(read(user_b), "USER-SOUL");
        assert_eq!(load_merge_pending(&td).len(), 2);
        // ⑤ 해소: 사용자가 vendor 본 채택(디스크=임베드) → .new·원장 항목 자동 청소.
        std::fs::write(td.join(user_b), get(user_b)).unwrap();
        install(false, None).expect("3차 실행 실패");
        assert!(!td.join("soul.md.new").exists(), "채택 후 .new 청소");
        assert!(load_merge_pending(&td).get(user_b).is_none(), "채택 후 원장 소거");

        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★플랜=실제 무드리프트(④): plan_install 분류가 같은 픽스처의 install 실행 결과와 일치.
    #[test]
    fn plan_install_matches_actual_install_actions() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!("cys-pack-plan-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let _env = set_pack_env(&td, td.join("cysclaude"));

        std::fs::create_dir_all(&td).unwrap();
        std::fs::write(td.join("README.md"), "OLD-INSTALLED").unwrap();
        std::fs::write(td.join("soul.md"), "USER-SOUL").unwrap();
        std::fs::write(td.join("alerts-config.json"), "SYS-DRIFT").unwrap(); // system 대조군(W-ACL 이후)
        // ★T3(커밋① · D14): kept-drift 픽스처 — manifest[rel]==hash(embed)(벤더 미전진 증명) +
        // 드리프트 → plan.kept_drift 버킷(unchanged 오계상·heal 버킷 재사용 금지의 박제).
        let kd_rel = "CLAUDE.md.template";
        let kd_embed = PACK_ALL.iter().find(|(r, _)| *r == kd_rel).map(|(_, c)| *c)
            .expect("팩에 CLAUDE.md.template 부재(system 픽스처)");
        std::fs::write(td.join(kd_rel), "KD-DRIFT-EDIT").unwrap();
        // ★T3(커밋② · D9·D10): 병합 성공 경로 plan=actual — `.pristine` 실재+검증 통과 픽스처
        // (현행 픽스처는 pristine 부재라 병합 경로가 무검증이던 사각의 봉인 · v1 감사 D6).
        // base = 임베드 머리에 LEGACY 헤더를 얹은 구본 · ours = base⊕꼬리 δ · theirs = 임베드
        // (머리 제거) — 머리/꼬리 분리라 clean 병합 = 임베드⊕δ.
        let m3_rel = "directives/CEO_TEMPLATE.md";
        let m3_embed = PACK_ALL.iter().find(|(r, _)| *r == m3_rel).map(|(_, c)| *c)
            .expect("팩에 CEO_TEMPLATE.md 부재(system 픽스처)");
        let m3_base = format!("LEGACY-HEADER\n{m3_embed}");
        let m3_ours = format!("LEGACY-HEADER\n{m3_embed}M3-USER-TAIL\n");
        {
            let p = td.join(m3_rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, &m3_ours).unwrap();
            let pp = td.join(PRISTINE_DIR).join(m3_rel);
            std::fs::create_dir_all(pp.parent().unwrap()).unwrap();
            std::fs::write(&pp, &m3_base).unwrap();
        }
        // 캡처는 td 내부로 격리(공유 temp 오염 방지 — env 오버라이드 경로 검증 겸용).
        let _cap = EnvGuard::set("CYS_PACK_CAPTURES_DIR", td.join("cap-root"));
        let manifest = serde_json::json!({
            "README.md": content_hash("OLD-INSTALLED"),
            "soul.md": content_hash("OLD-SOUL-BASE"),
            kd_rel: content_hash(kd_embed),
            m3_rel: content_hash(&m3_base),
        });
        std::fs::write(td.join(INSTALL_MANIFEST), manifest.to_string()).unwrap();

        let items: Vec<(&str, &str)> = PACK_ALL.iter().map(|(r, c)| (*r, *c)).collect();
        let plan = plan_install(&td, &items, false, env!("CARGO_PKG_VERSION"));
        assert!(plan.blocked.is_none());
        assert!(plan.update.iter().any(|r| r == "README.md"), "비수정 → update");
        assert!(plan.merge_new.iter().any(|r| r == "soul.md"), "user-owned+전진 → merge_new");
        assert!(plan.heal.iter().any(|r| r == "alerts-config.json"), "system 수정 → heal");
        assert!(plan.kept_drift.iter().any(|r| r == kd_rel),
                "★T3: system 수정+vendor 미전진 → kept_drift 버킷");
        assert!(!plan.heal.iter().any(|r| r == kd_rel), "kept-drift 를 heal 로 오보 금지(R7)");
        assert!(plan.merge3.iter().any(|r| r == m3_rel),
                "★T3(커밋②): 수정+vendor 전진+검증 base → merge3 버킷");
        assert!(!plan.heal.iter().any(|r| r == m3_rel), "merge3 를 heal 로 오보 금지(R7)");
        // 실제 install 이 플랜과 같은 행동을 하는지 대조.
        install(false, None).expect("install 실패");
        let read = |rel: &str| std::fs::read_to_string(td.join(rel)).unwrap();
        assert_eq!(read("soul.md"), "USER-SOUL");
        assert!(td.join("soul.md.new").exists());
        assert!(td.join("alerts-config.json.user").exists());
        // ★T3 plan=actual 확장: kept-drift 는 제자리 보존 + 원장 계상(파일 쓰기 0).
        assert_eq!(read(kd_rel), "KD-DRIFT-EDIT", "kept-drift 제자리 보존(plan=actual)");
        let pend = load_merge_pending(&td);
        assert_eq!(
            pend.get(kd_rel).and_then(|e| e["kind"].as_str()),
            Some("kept-drift"),
            "kept-drift 원장 계상(plan=actual)"
        );
        // ★T3(커밋②) plan=actual 병합 확장: install 후 disk=병합본 · 원장 merged · pristine 전진.
        assert_eq!(read(m3_rel), format!("{m3_embed}M3-USER-TAIL\n"),
                   "병합 성공: disk = 임베드⊕δ (plan=actual)");
        assert_eq!(pend.get(m3_rel).and_then(|e| e["kind"].as_str()), Some("merged"),
                   "merged 원장 계상(plan=actual)");
        assert_eq!(read(&format!("{PRISTINE_DIR}/{m3_rel}")), m3_embed, "pristine = 임베드 전진");

        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★T3(D7 · v2 §4 이식 ① — 원장 계보 보존 핀): merged 항목 위에 후속 upsert 가 와도
    /// 계보(kind·capture)가 소실되지 않는다. 커밋① = KeepDrift 쪽(정적 재스윕 → kind 불변·
    /// state:"at-rest" 만 · no-op 멱등). healed 쪽(크래시 창 재스윕 → kind 전환·capture 승계)은
    /// 커밋②에서 확장.
    #[test]
    fn merged_kind_not_clobbered() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-t3-lineage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        let _env = set_pack_env(&pd, base.join("claude"));

        let rel = "skills/t3-lineage/SKILL.md";
        let e1 = "VENDOR-V1\ncommon\n";
        let merged_disk = "VENDOR-V1\ncommon\nUSER-DELTA\n";
        let capture_ptr = "pack/100-1/skills/t3-lineage/SKILL.md";
        // 병합 직후 at-rest 상태 구성: disk=E1⊕δ · manifest=hash(E1) · 원장 merged{capture}.
        let p = pd.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, merged_disk).unwrap();
        std::fs::write(
            pd.join(INSTALL_MANIFEST),
            serde_json::json!({ rel: content_hash(e1) }).to_string(),
        )
        .unwrap();
        std::fs::write(
            pd.join(MERGE_PENDING_FILE),
            serde_json::json!({
                rel: {"kind": "merged", "side": rel, "capture": capture_ptr,
                      "version": "1.1.0", "ts": 100}
            })
            .to_string(),
        )
        .unwrap();

        // 정적 재스윕(벤더 미전진 — 같은 E1) → decide=KeepDrift.
        install_from_iter([(rel, e1)], false, "1.1.0", false, None).unwrap();
        let pend = load_merge_pending(&pd);
        let e = pend.get(rel).expect("원장 항목 잔존");
        assert_eq!(e["kind"].as_str(), Some("merged"),
                   "★D7: KeepDrift upsert 가 merged kind 를 덮었다(계보 소실)");
        assert_eq!(e["capture"].as_str(), Some(capture_ptr), "capture 포인터 소실");
        assert_eq!(e["state"].as_str(), Some("at-rest"), "state:\"at-rest\" 갱신 누락");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), merged_disk, "at-rest 본문 불가침");
        // 멱등: 재스윕 no-op(원장 바이트 불변 — dirty 미발생 계약).
        let bytes = std::fs::read(pd.join(MERGE_PENDING_FILE)).unwrap();
        install_from_iter([(rel, e1)], false, "1.1.0", false, None).unwrap();
        assert_eq!(std::fs::read(pd.join(MERGE_PENDING_FILE)).unwrap(), bytes,
                   "at-rest 재스윕이 원장을 rewrite(멱등 위반)");

        // ★T3(커밋②) healed 쪽: 크래시 광폭 창(disk=병합본·pristine=E1·manifest=hash(E0)) 재스윕
        // → base 검증 실패 → L4 healed — kind 는 전환되지만 capture 는 승계된다(소실 금지 · D7).
        let e0 = "VENDOR-V0\ncommon\n";
        {
            let pp = pd.join(PRISTINE_DIR).join(rel);
            std::fs::create_dir_all(pp.parent().unwrap()).unwrap();
            std::fs::write(&pp, e1).unwrap();
        }
        std::fs::write(
            pd.join(INSTALL_MANIFEST),
            serde_json::json!({ rel: content_hash(e0) }).to_string(),
        )
        .unwrap();
        install_from_iter([(rel, e1)], false, "1.1.0", false, None).unwrap();
        let pend2 = load_merge_pending(&pd);
        let ent2 = pend2.get(rel).expect("healed 전환 후 항목 잔존");
        assert_eq!(ent2["kind"].as_str(), Some("healed"), "★D7 healed 쪽: kind 전환 허용");
        assert_eq!(ent2["capture"].as_str(), Some(capture_ptr),
                   "★D7 healed 쪽: capture 승계(크래시 창 캡처 포인터 소실 금지)");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.user"))).unwrap(), merged_disk,
                   ".user = 병합본 바이트 보존(손실 0)");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), e1, "disk = vendor 보증(healed)");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★T3(D13): W-E2/부트 요약 계상 산식 — kind 별 **명시** count(빼기 산식 금지). 미지 kind 는
    /// 어느 버킷에도 오계상되지 않는다(구 산식은 신 kind 전부를 '.new 병치'로 오보 — 성찰 실측).
    #[test]
    fn w_e2_summary_counts_by_kind_explicit() {
        let mk = |kind: &str| serde_json::json!({"kind": kind, "side": "x", "version": "1.0.0", "ts": 1});
        let mut pending = serde_json::Map::new();
        pending.insert("a".into(), mk("healed"));
        pending.insert("b".into(), mk("new-pending"));
        pending.insert("c".into(), mk("kept-drift"));
        pending.insert("d".into(), mk("merged"));
        pending.insert("e".into(), mk("conflicted"));
        pending.insert("f".into(), mk("quarantined"));
        pending.insert("g".into(), mk("future-unknown-kind"));
        pending.insert("h".into(), mk("healed"));
        pending.insert("i".into(), mk("adopted"));
        let c = pending_kind_counts(&pending);
        assert_eq!(
            (c.healed, c.new_pending, c.kept_drift, c.merged, c.conflicted, c.quarantined),
            (2, 1, 1, 1, 1, 1),
            "kind 별 명시 계상 위반 — 빼기 산식이면 미지 kind 가 기존 버킷에 오계상된다"
        );
        // ★성찰 차단 수리(계상 SOT): adopted 명시 버킷 + 미지 kind 안전측 가시(unknown) 계상.
        assert_eq!(
            (c.adopted, c.unknown),
            (1, 1),
            "adopted 는 명시 버킷, 미지 kind 는 unknown 버킷 — 무계상 침묵 금지"
        );
        assert_eq!(
            c.actionable(),
            pending
                .values()
                .filter(|e| {
                    !matches!(
                        e.get("kind").and_then(|v| v.as_str()),
                        Some("kept-drift") | Some("merged")
                    )
                })
                .count(),
            "actionable() ≠ pack-plan 현행 자구(kept-drift·merged 외 전부) — 위임 소비 등가성 파괴"
        );
    }

    /// ★성찰 차단 수리 census 핀(계상 SOT 3분산): LEDGER_KINDS(유일 등재소) ↔
    /// pending_kind_counts 버킷의 전단사 박제. ①등재 kind 각 1건 → 대응 명시 버킷 정확 1 ·
    /// unknown 0 (등재만 하고 버킷을 안 만들면 unknown 으로 새서 실패) ②전필드 struct 리터럴
    /// 대조(`..Default` 금지) — 버킷 필드를 추가하고 여기를 안 고치면 **컴파일이 거부**한다
    /// (등재 없는 유령 버킷의 역방향 봉인) ③미지 kind → unknown(안전측 가시).
    #[test]
    fn ledger_kinds_census_bijective_with_counter() {
        // ① 등재 kind 각각 단독 1건 — 명시 버킷 정확 1 + unknown 0.
        for kind in LEDGER_KINDS {
            let mut pending = serde_json::Map::new();
            pending.insert(
                "x".into(),
                serde_json::json!({"kind": kind, "side": "x", "version": "1.0.0", "ts": 1}),
            );
            let c = pending_kind_counts(&pending);
            assert_eq!(c.unknown, 0, "등재 kind {kind:?} 가 unknown 으로 샜다 — 버킷 누락");
            let named_sum = c.healed
                + c.new_pending
                + c.kept_drift
                + c.merged
                + c.conflicted
                + c.quarantined
                + c.adopted;
            assert_eq!(named_sum, 1, "등재 kind {kind:?} 계상 오류(명시 버킷 합 {named_sum})");
        }
        // ② 전 kind 1건씩 — 전필드 struct 리터럴(컴파일 강제: 필드 추가 시 여기 미갱신 = 거부).
        let mut pending = serde_json::Map::new();
        for kind in LEDGER_KINDS {
            pending.insert(
                kind.to_string(),
                serde_json::json!({"kind": kind, "side": kind, "version": "1.0.0", "ts": 1}),
            );
        }
        assert_eq!(
            pending_kind_counts(&pending),
            PendingKindCounts {
                healed: 1,
                new_pending: 1,
                kept_drift: 1,
                merged: 1,
                conflicted: 1,
                quarantined: 1,
                adopted: 1,
                unknown: 0,
            },
            "LEDGER_KINDS ↔ 버킷 전단사 파괴"
        );
        // ③ 미등재 kind → unknown 안전측 가시(리터럴 대신 변수 경유 — 등재 강제 대상 아님).
        let ghost = format!("future-{}", "kind");
        let mut pending = serde_json::Map::new();
        pending.insert(
            "y".into(),
            serde_json::json!({"kind": ghost, "side": "y", "version": "1.0.0", "ts": 1}),
        );
        let c = pending_kind_counts(&pending);
        assert_eq!((c.unknown, c.actionable()), (1, 1), "미지 kind 는 unknown + actionable 가시");
    }

    /// ★T3 통합 핀 ①(v2 §4 크래시 수렴 · 출하 차단): 크래시 직후 상태를 직접 구성해(txn_prestate
    /// 전례 동형 — 프로세스 킬 불요) 재스윕이 "재병합 수렴" 또는 "healed(.user)" 로만 낙하함을
    /// 증명한다 — 침묵 실패 0 · 바이트 손실 0. 레인: [W1 캡처 후 병합 전] [W2 disk 기록 후
    /// pristine 전] [W3 광폭 창(★다중 파일 3본 · capture 승계)] [배타 캡처 세그먼트]
    /// [W-cap 캡처 금지(루트 파일 점유 → 병합 미시도 → conflicted 폴백)].
    #[test]
    fn merge3_crash_windows_converge() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base_td = std::env::temp_dir().join(format!("cys-t3-crash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base_td);
        let pd = base_td.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        let _env = set_pack_env(&pd, base_td.join("claude"));
        let _cap_env = EnvGuard::remove("CYS_PACK_CAPTURES_DIR"); // 기본 유도(dir 형제) 검증

        let e_old = "head-old\ncommon body\n";
        let e1 = "head-new\ncommon body\n";
        let ours = "head-old\ncommon body\nuser-tail-delta\n"; // δ=꼬리(머리/꼬리 분리 → clean)
        let merged = "head-new\ncommon body\nuser-tail-delta\n";
        let seed = |rel: &str, disk: &str, pristine: &str| {
            let p = pd.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, disk).unwrap();
            let pp = pd.join(PRISTINE_DIR).join(rel);
            std::fs::create_dir_all(pp.parent().unwrap()).unwrap();
            std::fs::write(&pp, pristine).unwrap();
        };

        // ── [W1] 캡처 후 병합 전 크래시: disk=ours·pristine=E_old·manifest=hash(E_old)
        //    → 동일 판정 재도달·clean 병합(캡처는 ts 세그먼트라 중복 무해). ──
        let w1 = "skills/w1/SKILL.md";
        seed(w1, ours, e_old);
        std::fs::write(pd.join(INSTALL_MANIFEST),
            serde_json::json!({ w1: content_hash(e_old) }).to_string()).unwrap();
        install_from_iter([(w1, e1)], false, "1.1.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd.join(w1)).unwrap(), merged, "[W1] clean 재병합 수렴");
        let pend1 = load_merge_pending(&pd);
        assert_eq!(pend1.get(w1).and_then(|e| e["kind"].as_str()), Some("merged"), "[W1] 원장 merged");
        let cap_rel = pend1.get(w1).and_then(|e| e["capture"].as_str())
            .expect("[W1] capture 필드").to_string();
        let cap_root = base_td.join("pack-captures");
        assert_eq!(std::fs::read_to_string(cap_root.join(&cap_rel)).unwrap(), ours,
                   "[W1] 캡처 = 사용자본 전문(팩 밖 증거)");
        assert!(cap_rel.starts_with("pack/") && cap_rel.ends_with(w1),
                "[W1] capture = 캡처 루트 상대 <pack-basename>/<seg>/<rel>: {cap_rel}");
        // 세그먼트 디렉터리명 규약 = <unix_secs>-<pid>[-n](콜론 0 — Windows 안전).
        let seg_name = cap_rel.split('/').nth(1).unwrap().to_string();
        let parts: Vec<&str> = seg_name.split('-').collect();
        assert!((2..=3).contains(&parts.len()) && parts.iter().all(|p| p.parse::<u64>().is_ok()),
                "[W1] 캡처 세그먼트 명명 규약 위반: {seg_name}");
        assert_eq!(std::fs::read_to_string(pd.join(PRISTINE_DIR).join(w1)).unwrap(), e1,
                   "[W1] pristine=E1 전진");
        let audit = std::fs::read_to_string(pd.join(MERGE_AUDIT_FILE)).unwrap();
        assert!(audit.contains("pre-merge-capture") && audit.contains(w1),
                "[W1] 감사 원장 pre-merge-capture 라인(원장 save 전 크래시 창의 증거)");

        // ── [W2] disk 기록 후 pristine 전 크래시: disk=merged·pristine=E_old·manifest=hash(E_old)
        //    → base(E_old) 검증 통과 → 재병합(ours⊇theirs) clean 수렴 · .user 미생성. ──
        let w2 = "skills/w2/SKILL.md";
        seed(w2, merged, e_old);
        std::fs::write(pd.join(INSTALL_MANIFEST),
            serde_json::json!({ w1: content_hash(e1), w2: content_hash(e_old) }).to_string()).unwrap();
        install_from_iter([(w1, e1), (w2, e1)], false, "1.1.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd.join(w2)).unwrap(), merged,
                   "[W2] 재병합 clean 수렴 — δ·Δvendor 동시 잔존");
        assert!(!pd.join(format!("{w2}.user")).exists(), "[W2] clean 수렴 — .user 미생성");
        assert_eq!(load_merge_pending(&pd).get(w2).and_then(|e| e["kind"].as_str()),
                   Some("merged"), "[W2] 원장 merged");

        // ── [W3] 광폭 창(★다중 파일 3본): disk=merged·pristine=E1·manifest=hash(E_old)
        //    → base 검증 실패(hash(E1)≠hash(E_old)) → L4 healed — 시끄러운 손실 0 폴백.
        //    사전 시드된 merged 항목의 capture 는 healed 전환 후에도 승계된다(★D7). ──
        let w3s = ["skills/w3a/SKILL.md", "skills/w3b/SKILL.md", "skills/w3c/SKILL.md"];
        let mut mani = serde_json::Map::new();
        mani.insert(w1.to_string(), serde_json::json!(content_hash(e1)));
        mani.insert(w2.to_string(), serde_json::json!(content_hash(e1)));
        let mut pend_seed = load_merge_pending(&pd);
        for rel in w3s {
            seed(rel, merged, e1);
            mani.insert(rel.to_string(), serde_json::json!(content_hash(e_old)));
            pend_seed.insert(rel.to_string(), serde_json::json!({
                "kind": "merged", "side": rel, "capture": format!("pack/1-1/{rel}"),
                "version": "1.1.0", "ts": 1
            }));
        }
        std::fs::write(pd.join(INSTALL_MANIFEST),
            serde_json::Value::Object(mani).to_string()).unwrap();
        std::fs::write(pd.join(MERGE_PENDING_FILE),
            serde_json::to_string_pretty(&serde_json::Value::Object(pend_seed)).unwrap()).unwrap();
        let items3: Vec<(&str, &str)> =
            vec![(w1, e1), (w2, e1), (w3s[0], e1), (w3s[1], e1), (w3s[2], e1)];
        install_from_iter(items3.iter().copied(), false, "1.1.0", false, None).unwrap();
        let pend3 = load_merge_pending(&pd);
        for rel in w3s {
            assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), e1,
                       "[W3] disk=vendor 보증(healed) — {rel}");
            assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.user"))).unwrap(), merged,
                       "[W3] .user=병합본 바이트 보존(손실 0) — {rel}");
            let e = pend3.get(rel).unwrap();
            assert_eq!(e["kind"].as_str(), Some("healed"), "[W3] kind=healed — {rel}");
            assert_eq!(e["capture"].as_str(), Some(format!("pack/1-1/{rel}").as_str()),
                       "[W3] ★D7: merged 항목의 capture 승계 — {rel}");
        }
        // 침묵 0: 변경 3파일 전원이 원장에 계상됐다(계상 합 = 변경 파일 수).
        assert!(w3s.iter().all(|r| pend3.contains_key(*r)), "[W3] 침묵 파일 존재");

        // ── [배타 세그먼트] 새 vendor E2 재스윕 → 두 번째 캡처 — 최초 캡처 바이트 불변
        //    (같은 초 재기동이어도 배타 create_dir + 접미 루프가 덮어쓰기를 봉인). ──
        let e2 = "head-v2\ncommon body\n";
        install_from_iter([(w1, e2), (w2, e2)], false, "1.2.0", false, None).unwrap();
        let lane = cap_root.join("pack");
        let segs: Vec<String> = std::fs::read_dir(&lane).unwrap()
            .filter_map(|d| d.ok().map(|e| e.file_name().to_string_lossy().to_string()))
            .collect();
        assert!(segs.len() >= 2, "배타 세그먼트 — 스윕별 캡처 디렉터리 실재(2본+): {segs:?}");
        assert_eq!(std::fs::read_to_string(cap_root.join(&cap_rel)).unwrap(), ours,
                   "최초 캡처 바이트 불변(증거의 자기 소실 금지)");

        // ── [W-cap] 캡처 금지: 캡처 루트 자리를 파일이 점유 → create_dir_all 결정론 실패 →
        //    병합 미시도 → healed 폴백(원장 conflicted{reason:capture-failed} + .base 조상). ──
        let base2 = std::env::temp_dir().join(format!("cys-t3-crash-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base2);
        let pd2 = base2.join("pack");
        std::fs::create_dir_all(&pd2).unwrap();
        let _env2 = set_pack_env(&pd2, base2.join("claude"));
        std::fs::write(base2.join("pack-captures"), "OCCUPIED").unwrap();
        let wc = "skills/wc/SKILL.md";
        {
            let p = pd2.join(wc);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, ours).unwrap();
            let pp = pd2.join(PRISTINE_DIR).join(wc);
            std::fs::create_dir_all(pp.parent().unwrap()).unwrap();
            std::fs::write(&pp, e_old).unwrap();
        }
        std::fs::write(pd2.join(INSTALL_MANIFEST),
            serde_json::json!({ wc: content_hash(e_old) }).to_string()).unwrap();
        install_from_iter([(wc, e1)], false, "1.1.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd2.join(wc)).unwrap(), e1, "[W-cap] vendor 본 기록");
        assert_eq!(std::fs::read_to_string(pd2.join(format!("{wc}.user"))).unwrap(), ours,
                   "[W-cap] .user=사용자본(캡처 실패 시에도 §2 원칙 4 보존)");
        assert_eq!(std::fs::read_to_string(pd2.join(format!("{wc}.base"))).unwrap(), e_old,
                   "[W-cap] .base=조상 전문(사후 3-way 재료)");
        let pend_c = load_merge_pending(&pd2);
        let ec = pend_c.get(wc).unwrap();
        assert_eq!(ec["kind"].as_str(), Some("conflicted"), "[W-cap] 원장 conflicted");
        assert_eq!(ec["reason"].as_str(), Some("capture-failed"), "[W-cap] reason 토큰");
        assert_eq!(ec["base_side"].as_str(), Some(format!("{wc}.base").as_str()),
                   "[W-cap] base_side 포인터");

        let _ = std::fs::remove_dir_all(&base_td);
        let _ = std::fs::remove_dir_all(&base2);
    }

    /// ★성찰 차단 수리 2R-5 핀(v2 §2 원칙 4 — 손실 0 계약): capture-failed 레인의 .user 백업
    /// 이중 실패(캡처 루트 파일 점유 + <rel>.user 자리 디렉터리 점유 — 양쪽 다 이 스위트의 기존
    /// 픽스처 프리미티브)에서 vendor 기록·manifest·pristine 전진을 전부 중단하고 quarantined
    /// 전이(D11 수렴 기전)함을 박제. 수리 전: .user 쓰기 실패가 `let _ =` 로 침묵 통과된 뒤
    /// 공용 레인이 vendor 본을 무조건 기록 — 무캡처 상태의 사용자 바이트 완전 소실 경로.
    /// 회복(점유 해제) 후 재스윕이 정상 Merge3 로 수렴함까지 함께 잰다.
    #[test]
    fn merge3_capture_failed_user_backup_double_failure_quarantines() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-2r5-dbl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        let _env = set_pack_env(&pd, base.join("claude"));
        let _cap_env = EnvGuard::remove("CYS_PACK_CAPTURES_DIR"); // 기본 유도(dir 형제)

        let e_old = "head-old\ncommon body\n";
        let e1 = "head-new\ncommon body\n";
        let ours = "head-old\ncommon body\nuser-tail-delta\n";
        let rel = "skills/wdbl/SKILL.md";
        {
            let p = pd.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, ours).unwrap();
            let pp = pd.join(PRISTINE_DIR).join(rel);
            std::fs::create_dir_all(pp.parent().unwrap()).unwrap();
            std::fs::write(&pp, e_old).unwrap();
        }
        std::fs::write(
            pd.join(INSTALL_MANIFEST),
            serde_json::json!({ rel: content_hash(e_old) }).to_string(),
        )
        .unwrap();
        // 이중 실패 주입: ①캡처 루트 자리 파일 점유(W-cap 프리미티브 — 캡처 불가) ②<rel>.user
        // 자리 디렉터리 점유(rename 불가 — .user 백업 불가).
        std::fs::write(base.join("pack-captures"), "OCCUPIED").unwrap();
        std::fs::create_dir_all(pd.join(format!("{rel}.user"))).unwrap();

        install_from_iter([(rel, e1)], false, "1.1.0", false, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(pd.join(rel)).unwrap(),
            ours,
            "이중 실패 = 파일 무접촉(vendor 기록 스킵 — 사용자 바이트 소실 경로 봉인)"
        );
        let mani: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(pd.join(INSTALL_MANIFEST)).unwrap())
                .unwrap();
        assert_eq!(mani.get(rel), Some(&content_hash(e_old)), "manifest 미전진(재스윕 수렴 조건)");
        assert_eq!(
            std::fs::read_to_string(pd.join(PRISTINE_DIR).join(rel)).unwrap(),
            e_old,
            "pristine 미전진"
        );
        let pend = load_merge_pending(&pd);
        let e = pend.get(rel).unwrap();
        assert_eq!(e["kind"].as_str(), Some("quarantined"), "원장 quarantined 전이(D11 동형)");
        assert_eq!(e["reason"].as_str(), Some("user-backup-failed"), "이중 실패 사유 토큰");

        // 회복 후 재스윕 = 동일 Merge3 판정 재도달 → 정상 병합 수렴(침묵 실패 0 · 손실 0).
        std::fs::remove_file(base.join("pack-captures")).unwrap();
        std::fs::remove_dir_all(pd.join(format!("{rel}.user"))).unwrap();
        install_from_iter([(rel, e1)], false, "1.1.0", false, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(pd.join(rel)).unwrap(),
            "head-new\ncommon body\nuser-tail-delta\n",
            "회복 후 재스윕 = 정상 3-way 수렴(δ 생존)"
        );
        assert_eq!(
            load_merge_pending(&pd).get(rel).and_then(|e| e["kind"].as_str()),
            Some("merged"),
            "원장 merged 정규화"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★성찰 차단 수리 2R-4 공통 픽스처: Merge3 유발 형상 시드(disk=ours · pristine=base ·
    /// manifest=hash(base)) — 캡처 루트는 base/caps 로 고정해 캡처 실물 검증 가능.
    fn seed_merge3_shape(pd: &std::path::Path, rel: &str, ours: &str, base: &str) {
        let p = pd.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, ours).unwrap();
        let pp = pd.join(PRISTINE_DIR).join(rel);
        std::fs::create_dir_all(pp.parent().unwrap()).unwrap();
        std::fs::write(&pp, base).unwrap();
        std::fs::write(
            pd.join(INSTALL_MANIFEST),
            serde_json::json!({ rel: content_hash(base) }).to_string(),
        )
        .unwrap();
    }

    /// 캡처 루트 아래에서 rel 캡처 실물을 찾아 내용 반환(세그먼트 ts-pid 동적 명명 대응).
    fn find_capture(cap_root: &std::path::Path, rel: &str) -> Option<String> {
        let lane = cap_root.join("pack");
        for seg in std::fs::read_dir(lane).ok()?.filter_map(|e| e.ok()) {
            let cand = seg.path().join(rel);
            if cand.is_file() {
                return std::fs::read_to_string(cand).ok();
            }
        }
        None
    }

    /// ★성찰 차단 수리 2R-4 핀 ⓐ(뮤테이션 생존 봉인 — v2 §4 clean-but-wrong 차단): install_into
    /// Merge3 실행부의 *.json 게이트 배선을 스윕 경로 통합으로 박제. 픽스처: base/theirs 는 머리
    /// 값 전진, ours 는 꼬리 닫는 중괄호 삭제(50% 미만 — suspect 비발동 격리) → clean 병합이나
    /// 결과는 파스 불능 → conflicted{reason:json-gate} + vendor 보증 + .user/.base/캡처 보존.
    /// (M8 실측: 이 배선을 꺼도 종전 스위트는 전 초록 — merge3.rs 단위 핀만으로는 실행부 회귀를
    /// 못 잡는다.)
    #[test]
    fn merge3_json_gate_wired_in_install_sweep() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base_td = std::env::temp_dir().join(format!("cys-2r4-json-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base_td);
        let pd = base_td.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        let _env = set_pack_env(&pd, base_td.join("claude"));
        let _cap = EnvGuard::set("CYS_PACK_CAPTURES_DIR", base_td.join("caps"));

        let rel = "skills/wjson/config.json";
        let base = "{\n  \"alpha\": 1,\n  \"beta\": 2,\n  \"gamma\": 3,\n  \"delta\": 4,\n  \"epsilon\": 5,\n  \"zeta\": 6\n}\n";
        let theirs = "{\n  \"alpha\": 10,\n  \"beta\": 2,\n  \"gamma\": 3,\n  \"delta\": 4,\n  \"epsilon\": 5,\n  \"zeta\": 6\n}\n";
        let ours = "{\n  \"alpha\": 1,\n  \"beta\": 2,\n  \"gamma\": 3,\n  \"delta\": 4,\n  \"epsilon\": 5,\n  \"zeta\": 6\n";
        // 전제 자가검증: 이 3자는 clean 병합되며 그 결과가 파스 불능이어야 한다(픽스처 유효성).
        match crate::merge3::merge3(base, ours, theirs) {
            crate::merge3::Merge3Outcome::Clean(m) => {
                assert!(serde_json::from_str::<serde_json::Value>(&m).is_err(),
                    "픽스처 전제: clean-but-wrong(파스 불능)");
            }
            crate::merge3::Merge3Outcome::Conflict(_) => panic!("픽스처 전제: clean 병합이어야 함"),
        }
        seed_merge3_shape(&pd, rel, ours, base);
        install_from_iter([(rel, theirs)], false, "1.1.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), theirs,
            "[json-gate] disk=vendor 보증(healed 폴백)");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.user"))).unwrap(), ours,
            "[json-gate] .user=사용자본 보존");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.base"))).unwrap(), base,
            "[json-gate] .base=조상 보존(사후 3-way 재료)");
        assert_eq!(find_capture(&base_td.join("caps"), rel).as_deref(), Some(ours),
            "[json-gate] 캡처 보존(병합 시도 전 원본)");
        let pend = load_merge_pending(&pd);
        let e = pend.get(rel).unwrap();
        assert_eq!(e["kind"].as_str(), Some("conflicted"), "[json-gate] 원장 conflicted");
        assert_eq!(e["reason"].as_str(), Some("json-gate"), "[json-gate] reason 토큰");
        let _ = std::fs::remove_dir_all(&base_td);
    }

    /// ★성찰 차단 수리 2R-4 핀 ⓑ(v2 §6 계층2 순삭제 세탁 조준 방어): base 10줄 · ours 꼬리
    /// 5줄 순삭제(정확 50%) · theirs 머리만 전진(비접촉) → clean 병합으로 손상이 "사용자 삭제
    /// δ" 로 세탁되는 시나리오 — 스윕 경로에서 suspect_damage 배선이 이를 잡아
    /// conflicted{reason:suspect:PureDeletionMajority…} 로 강등함을 박제.
    #[test]
    fn merge3_suspect_deletion_laundering_wired_in_install_sweep() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base_td = std::env::temp_dir().join(format!("cys-2r4-del-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base_td);
        let pd = base_td.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        let _env = set_pack_env(&pd, base_td.join("claude"));
        let _cap = EnvGuard::set("CYS_PACK_CAPTURES_DIR", base_td.join("caps"));

        let rel = "skills/wdel/SKILL.md";
        let base = "head-old\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";
        let theirs = "head-new\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";
        let ours = "head-old\nl2\nl3\nl4\nl5\n"; // 꼬리 5/10 순삭제(잘림 손상과 문자적 동형)
        seed_merge3_shape(&pd, rel, ours, base);
        install_from_iter([(rel, theirs)], false, "1.1.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), theirs,
            "[suspect] disk=vendor 보증(세탁 clean 병합 차단)");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.user"))).unwrap(), ours,
            "[suspect] .user=사용자본 보존");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.base"))).unwrap(), base,
            "[suspect] .base=조상 보존");
        let pend = load_merge_pending(&pd);
        let e = pend.get(rel).unwrap();
        assert_eq!(e["kind"].as_str(), Some("conflicted"), "[suspect] 원장 conflicted");
        let reason = e["reason"].as_str().unwrap_or("");
        assert!(reason.starts_with("suspect:PureDeletionMajority"),
            "[suspect] reason=순삭제 휴리스틱 토큰(실측: {reason})");
        let _ = std::fs::remove_dir_all(&base_td);
    }

    /// ★성찰 차단 수리 2R-4 핀 ⓒ(v2 §4 ④ pristine 검증형 승격): pristine 기록 실패 주입
    /// (write_atomic 의 tmp 슬롯 `.{fname}.tmp.{pid}` 자리 디렉터리 점유 — 플랫폼 공통 실패)
    /// 시 ④ 를 중단하고 ⑤ healed 폴백으로 강등 — .user=병합본(ours 원본은 캡처) ·
    /// conflicted{reason:pristine-write-failed} · pristine 미전진을 박제.
    #[test]
    fn merge3_pristine_write_failure_downgrades_to_conflicted() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base_td = std::env::temp_dir().join(format!("cys-2r4-pri-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base_td);
        let pd = base_td.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        let _env = set_pack_env(&pd, base_td.join("claude"));
        let _cap = EnvGuard::set("CYS_PACK_CAPTURES_DIR", base_td.join("caps"));

        let rel = "skills/wpri/SKILL.md";
        let base = "head-old\ncommon body\n";
        let theirs = "head-new\ncommon body\n";
        let ours = "head-old\ncommon body\nuser-tail-delta\n";
        let merged = "head-new\ncommon body\nuser-tail-delta\n";
        seed_merge3_shape(&pd, rel, ours, base);
        // 주입: pristine write_atomic 의 tmp 경로를 디렉터리로 점유(생성 실패 결정론).
        let tmp_slot = pd
            .join(PRISTINE_DIR)
            .join("skills/wpri")
            .join(format!(".SKILL.md.tmp.{}", std::process::id()));
        std::fs::create_dir_all(&tmp_slot).unwrap();
        install_from_iter([(rel, theirs)], false, "1.1.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), theirs,
            "[pristine-fail] disk=vendor 보증(⑤ 강등 후 공용 레인)");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.user"))).unwrap(), merged,
            "[pristine-fail] .user=병합본(크래시표 2행 정합 — ours 원본은 캡처 보존)");
        assert_eq!(find_capture(&base_td.join("caps"), rel).as_deref(), Some(ours),
            "[pristine-fail] 캡처=ours 원본");
        assert_eq!(std::fs::read_to_string(pd.join(PRISTINE_DIR).join(rel)).unwrap(), base,
            "[pristine-fail] pristine 미전진(실패 격리)");
        let pend = load_merge_pending(&pd);
        let e = pend.get(rel).unwrap();
        assert_eq!(e["kind"].as_str(), Some("conflicted"), "[pristine-fail] 원장 conflicted");
        assert_eq!(e["reason"].as_str(), Some("pristine-write-failed"), "[pristine-fail] reason 토큰");
        assert_eq!(e["base_side"].as_str(), Some(format!("{rel}.base").as_str()),
            "[pristine-fail] base_side 포인터");
        let _ = std::fs::remove_dir_all(&base_td);
    }

    /// ★T3 통합 핀 ②(v2 §3 연쇄 릴리스 · 출하 차단): E0⊕δ → E1 → E2 2연쇄에서 δ 생존 —
    /// keep-mine 없이도 병합 기점(pristine·manifest)이 자동 전진함의 박제. δ 는 머리/꼬리 분리
    /// 배치(vendor 는 머리만·사용자 δ 는 꼬리만 — diffy 보수 충돌 회피).
    #[test]
    fn chained_release_double_merge() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-t3-chain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        let _env = set_pack_env(&pd, base.join("claude"));
        let _cap_env = EnvGuard::remove("CYS_PACK_CAPTURES_DIR");

        let rel = "skills/chain/SKILL.md";
        let e0 = "v0-head\ncommon body\n";
        let e1 = "v1-head\ncommon body\n";
        let e2 = "v2-head\ncommon body\n";
        let with_delta = |head: &str| format!("{head}user-tail-delta\n");
        // E0 설치(v1.0.0) — manifest·pristine 기준선.
        install_from_iter([(rel, e0)], false, "1.0.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd.join(PRISTINE_DIR).join(rel)).unwrap(), e0);
        // δ 주입(사용자 편집 — 규율 요구 0: 그냥 파일을 고친다).
        std::fs::write(pd.join(rel), with_delta(e0)).unwrap();
        // E1 릴리스(v1.1.0) — 자동 3-way: δ 가 E1 위에 재적용.
        install_from_iter([(rel, e1)], false, "1.1.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), with_delta(e1),
                   "1연쇄: disk=E1⊕δ(δ 생존 + vendor 전진 수용)");
        assert_eq!(std::fs::read_to_string(pd.join(PRISTINE_DIR).join(rel)).unwrap(), e1,
                   "1연쇄: pristine=E1");
        let mani1: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(pd.join(INSTALL_MANIFEST)).unwrap())
                .unwrap();
        assert_eq!(mani1.get(rel), Some(&content_hash(e1)), "1연쇄: manifest=hash(E1)");
        assert_eq!(load_merge_pending(&pd).get(rel).and_then(|e| e["kind"].as_str()),
                   Some("merged"), "1연쇄: 원장 merged");
        assert!(!pd.join(format!("{rel}.user")).exists(), "clean 병합 — .user 미생성");
        // E2 릴리스(v1.2.0) — base=E1 검증 통과 재병합: δ 재생존 + E2 마커(연쇄 자연 처리 §3).
        install_from_iter([(rel, e2)], false, "1.2.0", false, None).unwrap();
        let disk = std::fs::read_to_string(pd.join(rel)).unwrap();
        assert_eq!(disk, with_delta(e2), "2연쇄: disk=E2⊕δ(δ 생존)");
        assert!(disk.contains("v2-head") && disk.contains("user-tail-delta"),
                "2연쇄: E2 마커·δ 동시 잔존");
        assert_eq!(std::fs::read_to_string(pd.join(PRISTINE_DIR).join(rel)).unwrap(), e2,
                   "2연쇄: pristine=E2(병합 기점 자동 전진)");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★T3 통합 핀 ③(v2 §3 자연 보존 · 출하 차단): 병합 at-rest 본문은 ⓐ결손 스윕(정적)에
    /// KeepDrift 로 불가침(kind 불덮힘·state=at-rest·.user 미생성·쓰기 0) ⓑprune 에 수정본
    /// 분류로 생존한다 — manifest 의미론 무변("마지막 적용 vendor 해시")의 구조 보존 박제.
    #[test]
    fn at_rest_merged_survives_static_sweep_and_prune() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-t3-atrest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        let _env = set_pack_env(&pd, base.join("claude"));
        let _cap_env = EnvGuard::remove("CYS_PACK_CAPTURES_DIR");

        let rel = "skills/atrest/SKILL.md";
        let e0 = "v0-head\ncommon body\n";
        let e1 = "v1-head\ncommon body\n";
        install_from_iter([(rel, e0)], false, "1.0.0", false, None).unwrap();
        std::fs::write(pd.join(rel), format!("{e0}user-tail-delta\n")).unwrap();
        install_from_iter([(rel, e1)], false, "1.1.0", false, None).unwrap();
        let merged = format!("{e1}user-tail-delta\n");
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), merged, "전제: 병합 완료");
        let capture0 = load_merge_pending(&pd).get(rel)
            .and_then(|e| e["capture"].as_str()).expect("전제: capture 실재").to_string();

        // ⓐ 같은 릴리스 재스윕(정적) → KeepDrift: 본문·계보 불가침 + state=at-rest.
        let (w, _k) = install_from_iter([(rel, e1)], false, "1.1.0", false, None).unwrap();
        assert_eq!(w, 0, "ⓐ정적 재스윕 쓰기 0(병합본 치유 금지)");
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), merged, "ⓐ본문 바이트 불변");
        let pend = load_merge_pending(&pd);
        let entry = pend.get(rel).unwrap();
        assert_eq!(entry["kind"].as_str(), Some("merged"), "ⓐkind 불덮힘(★D7)");
        assert_eq!(entry["capture"].as_str(), Some(capture0.as_str()), "ⓐcapture 보존");
        assert_eq!(entry["state"].as_str(), Some("at-rest"), "ⓐstate=at-rest");
        assert!(!pd.join(format!("{rel}.user")).exists(), "ⓐ.user 미생성");

        // ⓑ 해당 rel 제외 차기 릴리스(폐기) — prune: manifest(hash E1)≠hash(disk=E1⊕δ) → 보존.
        let plan = plan_install(&pd, &[("other.txt", "OTHER")], false, "1.3.0");
        assert!(plan.prune_keep_modified.iter().any(|r| r == rel),
                "ⓑplan: 폐기지만 수정본 보존 분류");
        install_from_iter([("other.txt", "OTHER")], false, "1.3.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), merged,
                   "ⓑprune 생존(§3 자연 보존)");
        let mani: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(pd.join(INSTALL_MANIFEST)).unwrap())
                .unwrap();
        assert!(mani.contains_key(rel), "ⓑ매니페스트 유지(수정본 보존 arm)");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★T3(D4·D11 · v2 §3 L1): 판독 불가 system — 바이트 백업 성공 시 치유(.user=원본 바이트),
    /// 백업 실패 시 quarantined 전이(무접촉·manifest 미전진 = fail-closed 수렴 조건) 후 백업
    /// 가능 회복 시 치유로 수렴한다.
    #[test]
    fn unreadable_system_byte_backup() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-t3-quar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        let _env = set_pack_env(&pd, base.join("claude"));

        const CP949: &[u8] = b"\x23\x20\xb8\xb6\xbd\xba\xc5\xcd\x0a";
        assert!(String::from_utf8(CP949.to_vec()).is_err(), "픽스처 전제: 비UTF-8");
        let rel = "alerts-config.json";
        let vendor = "{\"alerts\":\"v2\"}\n";
        std::fs::write(pd.join(rel), CP949).unwrap();
        std::fs::write(pd.join(INSTALL_MANIFEST),
            serde_json::json!({ rel: content_hash("OLD-VENDOR-BASE") }).to_string()).unwrap();
        // 결정론 실패 주입: `<rel>.user` 자리에 디렉터리 — tmp 복사는 성공·rename 이 실패한다.
        std::fs::create_dir_all(pd.join(format!("{rel}.user"))).unwrap();

        install_from_iter([(rel, vendor)], false, "1.0.0", false, None).unwrap();
        assert_eq!(std::fs::read(pd.join(rel)).unwrap(), CP949,
                   "★D11: 백업 실패 시 덮지 않는다(무접촉 — §2 원칙 4)");
        let mani: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(pd.join(INSTALL_MANIFEST)).unwrap())
                .unwrap();
        assert_eq!(mani.get(rel), Some(&content_hash("OLD-VENDOR-BASE")),
                   "★D11: manifest 미전진(재스윕 L1 재도달 = 수렴 조건)");
        let pend = load_merge_pending(&pd);
        assert_eq!(pend.get(rel).and_then(|e| e["kind"].as_str()), Some("quarantined"));
        assert_eq!(pend.get(rel).and_then(|e| e["reason"].as_str()), Some("backup-failed"));
        // 멱등: 격리 재스윕 동일 상태·원장 no-op.
        let bytes = std::fs::read(pd.join(MERGE_PENDING_FILE)).unwrap();
        install_from_iter([(rel, vendor)], false, "1.0.0", false, None).unwrap();
        assert_eq!(std::fs::read(pd.join(MERGE_PENDING_FILE)).unwrap(), bytes,
                   "격리 재스윕 원장 no-op(rewrite 방지)");
        assert_eq!(std::fs::read(pd.join(rel)).unwrap(), CP949, "격리 재스윕 무접촉");
        // 백업 가능 회복(디렉터리 제거) → 재스윕: 바이트 백업 후 치유로 수렴.
        std::fs::remove_dir_all(pd.join(format!("{rel}.user"))).unwrap();
        install_from_iter([(rel, vendor)], false, "1.0.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), vendor, "회복 후 치유");
        assert_eq!(std::fs::read(pd.join(format!("{rel}.user"))).unwrap(), CP949,
                   "바이트 백업 = CP949 원본 왕복");
        let pend2 = load_merge_pending(&pd);
        assert_eq!(pend2.get(rel).and_then(|e| e["kind"].as_str()), Some("healed"),
                   "격리 해소 — healed 전환");
        assert_eq!(pend2.get(rel).and_then(|e| e["side"].as_str()),
                   Some(format!("{rel}.user").as_str()), "side=실백업 경로");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★T3(v2 §3 L0 · D11): 잠금 보안 자산(trusted-keys.json) — 백업 선행 시도하되, 백업 실패
    /// 시에도 치유를 강행한다(무결성>보존 — §2 원칙 4 의 유일 예외). 원장 side 는 **실백업 성공
    /// 시에만** 기록(실패 시 side 부재 + backup:"failed" — 존재하지 않는 .user 를 '보존됨'으로
    /// 오보하지 않는다).
    #[test]
    fn unreadable_locked_byte_backup() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        const CP949: &[u8] = b"\x23\x20\xb8\xb6\xbd\xba\xc5\xcd\x0a";
        let vendor = "{\"keys\":[\"v2\"]}\n";
        let rel = "trusted-keys.json";

        // ① 백업 성공 레인: 바이트 백업 선행 + 치유 + side=실백업 경로.
        let base1 = std::env::temp_dir().join(format!("cys-t3-lock1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base1);
        let pd1 = base1.join("pack");
        std::fs::create_dir_all(&pd1).unwrap();
        {
            let _env = set_pack_env(&pd1, base1.join("claude"));
            std::fs::write(pd1.join(rel), CP949).unwrap();
            install_from_iter([(rel, vendor)], false, "1.0.0", false, None).unwrap();
            assert_eq!(std::fs::read_to_string(pd1.join(rel)).unwrap(), vendor, "①L0 치유");
            assert_eq!(std::fs::read(pd1.join(format!("{rel}.user"))).unwrap(), CP949,
                       "①바이트 백업 선행(무백업 edge 봉인)");
            let p1 = load_merge_pending(&pd1);
            assert_eq!(p1.get(rel).and_then(|e| e["kind"].as_str()), Some("healed"));
            assert_eq!(p1.get(rel).and_then(|e| e["side"].as_str()),
                       Some(format!("{rel}.user").as_str()), "①side=실백업 경로");
        }

        // ② 백업 실패 레인: 치유 강행 + side 부재 + backup:"failed".
        let base2 = std::env::temp_dir().join(format!("cys-t3-lock2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base2);
        let pd2 = base2.join("pack");
        std::fs::create_dir_all(&pd2).unwrap();
        {
            let _env = set_pack_env(&pd2, base2.join("claude"));
            std::fs::write(pd2.join(rel), CP949).unwrap();
            std::fs::create_dir_all(pd2.join(format!("{rel}.user"))).unwrap(); // rename 결정론 실패
            install_from_iter([(rel, vendor)], false, "1.0.0", false, None).unwrap();
            assert_eq!(std::fs::read_to_string(pd2.join(rel)).unwrap(), vendor,
                       "②★L0: 백업 실패에도 치유 강행(무결성>보존)");
            let p2 = load_merge_pending(&pd2);
            assert_eq!(p2.get(rel).and_then(|e| e["kind"].as_str()), Some("healed"));
            assert!(p2.get(rel).and_then(|e| e.get("side")).is_none(),
                    "②side 부재 — 실패한 백업을 '보존됨'으로 오보 금지");
            assert_eq!(p2.get(rel).and_then(|e| e["backup"].as_str()), Some("failed"));
        }
        let _ = std::fs::remove_dir_all(&base1);
        let _ = std::fs::remove_dir_all(&base2);
    }

    #[test]
    fn install_ownership_system_forced_user_preserved() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!("cys-pack-ownership-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let _env = set_pack_env(&td, td.join("cysclaude")); // 격리(밀폐)

        let get = |rel: &str| PACK_ALL.iter().find(|(r, _)| *r == rel).map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("팩에 {rel} 부재"));
        let sys_a = "README.md";       // system·비수정(manifest 일치) → 갱신
        let user_b = "soul.md";        // user·수정 → 보존
        let sys_c = "alerts-config.json"; // system·manifest 부재·상이 → 강제 갱신(B2/P0-4)
        let user_d = "directives/MASTER_DIRECTIVE.md"; // user·manifest 부재·상이 → 보존
        let (sys_a_c, user_b_c, sys_c_c, user_d_c) =
            (get(sys_a), get(user_b), get(sys_c), get(user_d));
        // 임베드 4파일과 상이한 값이어야 함(내용 상이 조건).
        for c in [sys_a_c, user_b_c, sys_c_c, user_d_c] {
            assert_ne!(c, "OLD-INSTALLED"); assert_ne!(c, "USER-MODIFIED");
            assert_ne!(c, "SYS-DRIFT"); assert_ne!(c, "USER-CUSTOM");
        }
        std::fs::create_dir_all(&td).unwrap();
        for (rel, stale) in [(sys_a, "OLD-INSTALLED"), (user_b, "USER-MODIFIED"),
                             (sys_c, "SYS-DRIFT"), (user_d, "USER-CUSTOM")] {
            let p = td.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, stale).unwrap();
        }
        // 매니페스트: sys_a 는 설치-당시 해시=현재 디스크 해시(비수정 증명). 나머지는 항목 없음(부재).
        let manifest = serde_json::json!({ sys_a: content_hash("OLD-INSTALLED") });
        std::fs::write(td.join(INSTALL_MANIFEST), manifest.to_string()).unwrap();

        install(false, None).expect("install 실패");
        let read = |rel: &str| std::fs::read_to_string(td.join(rel)).unwrap();

        assert_eq!(read(sys_a), sys_a_c, "①system 비수정 → 임베드로 갱신");
        assert_eq!(read(user_b), "USER-MODIFIED", "②user 수정본 불가침");
        assert_eq!(read(sys_c), sys_c_c, "③★B2/P0-4: system 매니페스트부재·상이 → 강제 갱신(동결 금지)");
        assert_eq!(read(user_d), "USER-CUSTOM", "④user 매니페스트부재·상이 → 보존");

        // ⑤ 채택 기록 + 멱등: 재실행이 아무것도 다시 쓰지 않고 user 보존 유지.
        let m: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&read(INSTALL_MANIFEST)).unwrap();
        assert_eq!(m.get(sys_a), Some(&content_hash(sys_a_c)), "갱신 후 매니페스트 미반영");
        let (w2, _) = install(false, None).unwrap();
        assert_eq!(w2, 0, "멱등 위반: 재실행이 {w2}개를 다시 씀");
        assert_eq!(std::fs::read_to_string(td.join(user_b)).unwrap(), "USER-MODIFIED");
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★B2-1(W3): schedule.json 은 user-owned — 팩 **강제갱신(force)** 후에도 사용자 잡이 소실되지 않는다.
    /// (built-in phoenix 잡은 데몬 부트 ensure_builtin_jobs 가 별도로 upsert — 이 테스트는 사용자 잡 보존만 검증.)
    #[test]
    fn install_force_preserves_user_schedule_jobs() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!("cys-sched-owner-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let _env = set_pack_env(&td, td.join("cysclaude"));
        std::fs::create_dir_all(&td).unwrap();

        // 사용자가 `cys schedule add` 로 넣은 잡이 담긴 schedule.json(임베드와 상이).
        let user_schedule = r#"{"jobs":[{"id":"my-daily-brief","every_minutes":1440,"action":"push","to":"master","text":"USER JOB"}]}"#;
        std::fs::write(td.join("schedule.json"), user_schedule).unwrap();

        // force=true 강제갱신 — user-owned schedule.json 은 보존돼야 한다.
        install(true, None).expect("install(force) 실패");
        let after = std::fs::read_to_string(td.join("schedule.json")).unwrap();

        assert!(
            after.contains("my-daily-brief") && after.contains("USER JOB"),
            "강제갱신이 사용자 schedule.json 잡을 소실시켰다 — B2-1 위반. after={after}"
        );
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★W-ACL(오너 승인 2026-08-01): acl.json 은 user-owned — **force 설치에도 오너가 확정한 송신
    /// 정책이 그대로 남고**, vendor 신버전은 `<rel>.new` 로 주차된다(schedule.json·agents.json 동형).
    ///
    /// 무엇이 깨졌었나 — acl.json 만 system 등급이라 매 설치 스윕(P0-4 강제 치유)이 vendor 기본
    /// 정책으로 덮어써, "external→worker 차단" 같은 **설치별 운영 정책이 매 설치마다 원복**됐다
    /// (2026-08-01 실증). 이 테스트는 보존(등급)뿐 아니라 **동결이 아님**(vendor 신규 규칙이 .new +
    /// 병합 원장으로 도달)까지 함께 고정한다 — 보존만 하고 전달 경로가 없으면 정책이 영구 동결된다.
    #[test]
    fn install_force_preserves_user_acl_and_parks_vendor_new() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!("cys-acl-owner-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let _env = set_pack_env(&td, td.join("cysclaude"));
        std::fs::create_dir_all(&td).unwrap();

        // 오너가 이 설치본에서 확정한 송신 정책(임베드 기본값과 상이).
        let user_acl = r#"{"default":"deny","rules":[{"from":"external","to":"worker*","allow":false}]}"#;
        std::fs::write(td.join("acl.json"), user_acl).unwrap();
        let embed = PACK_ALL.iter().find(|(r, _)| *r == "acl.json").map(|(_, c)| *c)
            .expect("팩에 acl.json 부재");
        assert_ne!(embed, user_acl, "픽스처 전제: 오너 정책 ≠ vendor 기본 정책");

        // ★가장 공격적인 스윕: force 재설치(= `cys init-pack --force`).
        install(true, None).expect("install(force) 실패");
        let read = |rel: &str| std::fs::read_to_string(td.join(rel)).unwrap();

        // ① 보존: 오너 확정 정책이 vendor 기본으로 원복되지 않는다.
        assert_eq!(read("acl.json"), user_acl,
                   "강제갱신이 오너 확정 ACL 정책을 vendor 기본으로 원복시켰다 — W-ACL 위반");
        // ①-b Keep 은 '백업 후 교체'가 아니라 '무접촉' — 치유 사이드카(.user)가 생기면 안 된다.
        assert!(!td.join("acl.json.user").exists(), "Keep 인데 .user 사이드카가 생겼다(치유 경로 오진입)");

        // ② 동결 아님: vendor 신버전은 .new 로 주차 + 병합 원장 등재(pack-merge 가 소비).
        assert_eq!(read("acl.json.new"), embed, ".new = vendor 신버전 주차");
        assert_eq!(load_merge_pending(&td).get("acl.json").and_then(|e| e["kind"].as_str()),
                   Some("new-pending"), "병합 원장 미등재 — 신규 vendor 규칙 전달 경로 소실");

        // ③ 헌법 파일이 **아니다**: 정책 파일이므로 pack-merge 대화형 강제(--yes 무시·안전핵 검증)
        //    대상에 들어가면 안 된다(일반 user-owned 병합 경로).
        assert!(!is_constitution_file("acl.json"), "acl.json 이 헌법 특례에 들어갔다");

        // ④ 멱등: 재실행이 보존·주차 상태를 흔들지 않는다(원장 중복 기록 없음).
        install(true, None).expect("재실행 실패");
        assert_eq!(read("acl.json"), user_acl, "④재실행 후에도 보존 불변");
        assert_eq!(read("acl.json.new"), embed, "④재실행 후에도 .new 유지");

        // ⑤ 해소: 오너가 vendor 본을 채택(디스크=임베드)하면 .new·원장이 자동 청소된다.
        std::fs::write(td.join("acl.json"), embed).unwrap();
        install(false, None).expect("3차 실행 실패");
        assert!(!td.join("acl.json.new").exists(), "채택 후 .new 청소");
        assert!(load_merge_pending(&td).get("acl.json").is_none(), "채택 후 원장 소거");

        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★W-1 회귀 핀 ①(순수 판정 · P0 · 2026-08-01): **판독 불가(disk=None) user-owned 는 force 여도 Keep.**
    ///
    /// 무엇이 깨졌었나 — ko-KR Windows 에서 헌법 파일을 CP949/ANSI 로 저장하면 `read_to_string` 이
    /// 실패해 disk=None 이 된다. 종전 코드는 ⓐuser-owned 보존 블록의 `if let Some(d) = disk` 와
    /// ⓑ`exists && !force` 블록을 **둘 다 통과**해 최종 폴백 `Write { heal_user_copy: false }` 로
    /// 떨어졌다 — 무경고·무백업 덮어쓰기. 폭발 반경 = *_DIRECTIVE.md·soul.md·CLAUDE.md·
    /// schedule.json·agents.json. 이 테스트는 그 한 칸(교집합)만이 아니라 **주변 칸이 변하지 않았음**도
    /// 함께 고정해, 수리가 Keep 범위를 넓혀 system 강제 치유(P0-4)를 마비시키지 않았음을 증명한다.
    #[test]
    fn w1_unreadable_user_owned_kept_even_under_force() {
        use super::FileAction::*;
        let embed = "EMBED-V2";
        let eh = content_hash(embed);
        let keep = Keep { adopt_hash: false, new_pending: false };

        // ① ★수리 본체: user-owned + 존재 + 판독 불가 → force 여도 Keep(덮어쓰기 금지).
        //    매니페스트 유무 두 갈래 모두 — 구설치본(None)이 오히려 흔한 실사용 형상이다.
        for rel in ["soul.md", "directives/MASTER_DIRECTIVE.md", "CLAUDE.md",
                    "schedule.json", "agents.json", "acl.json", "sub/dir/CSO_DIRECTIVE.md"] {
            assert_eq!(decide_file_action(rel, embed, true, None, None, true), keep,
                       "①force=true·판독불가·매니페스트부재 → Keep 이어야: {rel}");
            assert_eq!(decide_file_action(rel, embed, true, None, Some(eh.as_str()), true), keep,
                       "①force=true·판독불가·매니페스트有 → Keep 이어야: {rel}");
            // force=false 도 동일(종전에도 보존됐던 칸 — 회귀 금지).
            assert_eq!(decide_file_action(rel, embed, true, None, None, false), keep,
                       "①force=false·판독불가 → Keep 유지: {rel}");
        }

        // ② 정상 UTF-8 user-owned 경로 **무변화**(수리가 판독 가능한 쪽 의미론을 건드리지 않았다).
        assert_eq!(decide_file_action("soul.md", embed, true, Some("MY-SOUL"), Some(eh.as_str()), true),
                   keep, "②읽기성공·상이·vendor 무전진 → 보존만(종전 동일)");
        assert_eq!(decide_file_action("soul.md", embed, true, Some("MY-SOUL"),
                       Some(content_hash("EMBED-V1").as_str()), true),
                   Keep { adopt_hash: false, new_pending: true },
                   "②읽기성공·상이·vendor 전진 → 보존 + .new 병치(종전 동일)");
        assert_eq!(decide_file_action("soul.md", embed, true, Some(embed), Some(eh.as_str()), true),
                   Write { heal_user_copy: false },
                   "②디스크=임베드 + force → 동일 내용 재기록(종전 동일 — Keep 으로 넓히지 않았다)");
        // ②-b 부재(신규 설치)는 판독 불가 개념 자체가 없다 — 시드 설치가 막히면 안 된다.
        assert_eq!(decide_file_action("soul.md", embed, false, None, None, true),
                   Write { heal_user_copy: false }, "②-b 부재 → 신규 설치(수리가 막지 않는다)");

        // ③ system-owned: 판독 불가여도 치유 유지 + ★L1/force 백업 의무 플래그(바이트 백업
        //    실행부는 T3 — T2 런타임 쓰기는 현행 byte-identical).
        for rel in ["bin/javis_phoenix.py", "alerts-config.json", "README.md", "CLAUDE.md.template",
                    "directives/CEO_TEMPLATE.md"] {
            assert_eq!(decide_file_action(rel, embed, true, None, None, true),
                       Write { heal_user_copy: true }, "③system·판독불가·force → 치유 유지: {rel}");
            assert_eq!(decide_file_action(rel, embed, true, None, None, false),
                       Write { heal_user_copy: true }, "③system·판독불가·비force → 치유 유지: {rel}");
        }
        assert_eq!(decide_file_action("alerts-config.json", embed, true, Some("SYS-DRIFT"), None, true),
                   Write { heal_user_copy: true }, "③system 수정본은 .user 보존 후 치유(종전 동일)");

        // ④ seed-once 는 종전부터 판독 불가에도 불가침 — 우선순위가 뒤집히지 않았음을 재확인.
        assert_eq!(decide_file_action("memory/MEMORY.md", embed, true, None, None, true), keep,
                   "④seed-once 판독불가·force → 불가침(종전 동일)");
    }

    /// ★W-1 회귀 핀 ②(실파일 · 바이트 단위): 프로덕션 `cys init-pack --force` 와 **같은 경로**
    /// (`install_staged` → staging 복사 → `install_into` → 원자 교체)로 CP949 바이트 디렉티브가
    /// 살아남는지 검증한다. 순수 판정만 보는 핀 ①과 달리, staging 복사·prune·atomic swap 이
    /// 중간에서 파일을 갈아치울 여지까지 닫는다. 대조군(system 등급·정상 UTF-8 user)이 종전대로
    /// 동작함을 같은 실행에서 함께 증명해, 보존이 "설치가 아무것도 안 한" 우연이 아님을 고정한다.
    #[test]
    fn w1_force_install_preserves_unreadable_user_files_bytewise() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-w1-cp949-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        let _env = set_pack_env(&pd, base.join("claude"));

        // "# 마스터 절대지침\n주인님 호칭 유지. 내 커스텀 규칙 3개.\n" 의 CP949 인코딩 —
        // 0xB8 이 UTF-8 시작 바이트로 불법이라 read_to_string 이 반드시 실패한다(disk=None 재현).
        const CP949: &[u8] = b"\x23\x20\xb8\xb6\xbd\xba\xc5\xcd\x20\xc0\xfd\xb4\xeb\xc1\xf6\xc4\xa7\x0a\
\xc1\xd6\xc0\xce\xb4\xd4\x20\xc8\xa3\xc4\xaa\x20\xc0\xaf\xc1\xf6\x2e\x20\xb3\xbb\x20\
\xc4\xbf\xbd\xba\xc5\xd2\x20\xb1\xd4\xc4\xa2\x20\x33\xb0\xb3\x2e\x0a";
        assert!(String::from_utf8(CP949.to_vec()).is_err(), "픽스처 전제: CP949 바이트는 비UTF-8");

        // ⓪ 정상 설치 1회(매니페스트·pristine 기준선 확보 — 실사용 형상과 동일).
        install_staged(false, None).unwrap();
        let embed_of = |rel: &str| PACK_ALL.iter().find(|(r, _)| *r == rel).map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("팩에 {rel} 부재"));

        let unreadable_user = ["soul.md", "directives/MASTER_DIRECTIVE.md"]; // 헌법 파일
        let unreadable_sys = "alerts-config.json"; // 대조군: system 등급 + 판독 불가
        let readable_user = "agents.json";    // 대조군: user 등급 + 정상 UTF-8 수정
        let readable_sys = "README.md";       // 대조군: system 등급 + 정상 UTF-8 수정

        for rel in unreadable_user {
            std::fs::write(pd.join(rel), CP949).unwrap();
        }
        std::fs::write(pd.join(unreadable_sys), CP949).unwrap();
        std::fs::write(pd.join(readable_user), "{\"adapters\":{\"my-cli\":\"MINE\"}}").unwrap();
        std::fs::write(pd.join(readable_sys), "SYS-EDIT-XYZ").unwrap();
        // readable_user 의 마지막 적용본을 과거로 되돌려 vendor 전진(.new 병치) 갈래까지 태운다.
        let mpath = pd.join(INSTALL_MANIFEST);
        let mut manifest: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(&mpath).unwrap()).unwrap();
        manifest.insert(readable_user.to_string(), content_hash("OLD-AGENTS-BASE"));
        std::fs::write(&mpath, serde_json::to_string(&manifest).unwrap()).unwrap();

        // ★가장 공격적인 스윕: force 재설치(= `cys init-pack --force`).
        install_staged(true, None).unwrap();

        // ① 판독 불가 헌법 파일 = **바이트 단위 불변**(내용 비교가 아니라 원본 바이트 그대로).
        for rel in unreadable_user {
            assert_eq!(
                std::fs::read(pd.join(rel)).unwrap(), CP949,
                "★W-1: force 가 판독 불가 user-owned 를 덮었다(무백업 소실 재발) — {rel}"
            );
            // 백업 사이드카가 없다는 사실도 박제 — Keep 은 '백업 후 교체'가 아니라 '무접촉'이다.
            assert!(!pd.join(format!("{rel}.user")).exists(), "Keep 인데 .user 가 생겼다 — {rel}");
        }
        // ② 정상 UTF-8 user-owned: 종전 계약 그대로(보존 + vendor 신버전 .new 병치).
        assert_eq!(
            std::fs::read_to_string(pd.join(readable_user)).unwrap(),
            "{\"adapters\":{\"my-cli\":\"MINE\"}}",
            "②user-owned 정상 UTF-8 수정본 보존(종전 동작 불변)"
        );
        assert_eq!(std::fs::read_to_string(pd.join(format!("{readable_user}.new"))).unwrap(),
                   embed_of(readable_user), "②vendor 신버전 .new 병치(종전 동작 불변)");
        // ③ system-owned: 판독 가능/불가 모두 강제 치유 유지(P0-4 마비 없음).
        assert_eq!(std::fs::read_to_string(pd.join(readable_sys)).unwrap(), embed_of(readable_sys),
                   "③system 수정본 → 임베드 치유(종전 동작 불변)");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{readable_sys}.user"))).unwrap(),
                   "SYS-EDIT-XYZ", "③치유 전 사용자본 .user 보존(종전 동작 불변)");
        assert_eq!(std::fs::read(pd.join(unreadable_sys)).unwrap(), embed_of(unreadable_sys).as_bytes(),
                   "③system 은 판독 불가여도 치유 — 수리가 Keep 범위를 넓히지 않았다");
        // ★T3(커밋② · D4·D11) ③-b: L1 바이트 백업 사이드카 — CP949 원본 **바이트 왕복**(.user)
        //   + 원장 정합(kind=healed · side=실백업 경로).
        assert_eq!(std::fs::read(pd.join(format!("{unreadable_sys}.user"))).unwrap(), CP949,
                   "③-b 판독 불가 system 치유 전 바이트 백업(.user=CP949 원본 왕복)");
        let pend_b = load_merge_pending(&pd);
        assert_eq!(pend_b.get(unreadable_sys).and_then(|e| e["kind"].as_str()), Some("healed"),
                   "③-b 원장 kind=healed");
        assert_eq!(pend_b.get(unreadable_sys).and_then(|e| e["side"].as_str()),
                   Some(format!("{unreadable_sys}.user").as_str()),
                   "③-b 원장 side=실백업 경로(.user)");

        // ④ 멱등: 판독 불가 파일이 남아 있어도 재실행이 상태를 흔들지 않는다.
        install_staged(true, None).unwrap();
        for rel in unreadable_user {
            assert_eq!(std::fs::read(pd.join(rel)).unwrap(), CP949, "④재실행 후에도 불변 — {rel}");
        }
        // ★T3 ③-b 재스윕 생존: 치유된 disk 는 판독 가능이라 재백업 비발동 — .user 바이트 불변.
        assert_eq!(std::fs::read(pd.join(format!("{unreadable_sys}.user"))).unwrap(), CP949,
                   "④재스윕 후에도 바이트 백업 생존(단일 슬롯 재복사 없음)");

        // ⑤ 병합 대기 중이던 파일이 판독 불가가 되면 대기를 **해소로 오인하지 않는다**.
        //    (읽지 못한 것은 '사용자가 vendor 본을 채택했다'는 증거가 아니다 — .new·원장 유지.)
        assert!(load_merge_pending(&pd).get(readable_user).is_some(), "⑤전제: 대기 항목 존재");
        std::fs::write(pd.join(readable_user), CP949).unwrap();
        install_staged(true, None).unwrap();
        assert_eq!(std::fs::read(pd.join(readable_user)).unwrap(), CP949, "⑤판독 불가 전환 후에도 불변");
        assert!(pd.join(format!("{readable_user}.new")).exists(),
                "⑤판독 불가를 '병합 해소'로 오인해 vendor 신버전 사본을 지웠다");
        assert!(load_merge_pending(&pd).get(readable_user).is_some(), "⑤원장 대기 항목도 유지");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★B2 분류 순수 함수: user 화이트리스트(디렉티브·헌법·CLAUDE.md·혼합 설정)만 preserve, 나머지=system.
    #[test]
    fn is_user_owned_classification() {
        // agents.json: ★W-B 승격 핀 — _doc 이 "사용자 환경에 맞게 수정 가능"이라 선언하는 혼합
        // 설정(schedule.json 동형). system 으로 되돌리면 사용자 어댑터 수정이 매 스윕 소실된다.
        // acl.json: ★W-ACL 승격 핀 — 설치별 운영 정책(누가 누구에게 stdin 을 넣는가)이라
        // schedule.json·agents.json 동형. system 으로 되돌리면 오너가 확정한 송신 정책이 매
        // 설치 스윕마다 vendor 기본으로 원복된다(2026-08-01 실증).
        for u in ["soul.md", "directives/MASTER_DIRECTIVE.md", "CLAUDE.md",
                  "sub/dir/CSO_DIRECTIVE.md", "some/soul.md", "schedule.json", "agents.json",
                  "acl.json"] {
            assert!(is_user_owned(u), "user 여야: {u}");
        }
        for s in ["bin/javis_phoenix.py", "hooks/session_start.sh", "README.md",
                  "alerts-config.json", "CLAUDE.md.template", "skills/x/SKILL.md",
                  "directives/CEO_TEMPLATE.md", "sub/schedule.json", "sub/agents.json",
                  "sub/acl.json"] {
            assert!(!is_user_owned(s), "system 여야: {s}");
        }
    }

    #[test]
    fn is_seed_once_classification_and_behavior() {
        // ★B2-2 분류: 런타임 상태(기억·복원 진실)만 seed-once — round/ 정적 계약은 system 유지.
        for st in ["memory/MEMORY.md", "memory/feedback_autonomous-pilot-mandate.md",
                   "round/SESSION_STATE.md", "round/RECOVERY.md"] {
            assert!(is_seed_once(st), "seed-once 여야: {st}");
        }
        for s in ["round/TOOL_RESULT_VOCAB.md", "round/capability_catalog.json",
                  "round/video-archetypes/cinematic/workflow.json", "bin/javis_memory.py"] {
            assert!(!is_seed_once(s), "system 여야: {s}");
        }
        // ★행동 박제(치유 원복 사고 회귀 핀): 존재하는 seed-once 상태는
        //   ① 수정돼 있어도 치유(Write) 금지, ② force 여도 불가침, ③ 읽기 실패(disk=None)여도 불가침.
        let embed = "SKELETON";
        let keep = FileAction::Keep { adopt_hash: false, new_pending: false };
        assert_eq!(decide_file_action("memory/MEMORY.md", embed, true, Some("LIVE-STATE"),
                       Some(content_hash(embed).as_str()), false), keep, "① 수정본 원복 금지");
        // ①-w(Windows 최다 발생 실측 2026-07-12): 팩 python 도구가 상태 파일을 텍스트 모드
        // (open("w")·newline 미지정)로 쓰면 Windows 에서 LF→CRLF 자동 변환 — 논리 내용이 같아도
        // 바이트가 달라져 구버전에서는 매 스윕 '수정됨'→치유(원복)였다. seed-once 는 내용·해시
        // 비교 **이전**의 조기 반환이라 개행 변형에도 불가침이다.
        assert_eq!(decide_file_action("memory/MEMORY.md", "A\nB\n", true, Some("A\r\nB\r\n"),
                       Some(content_hash("A\nB\n").as_str()), false), keep, "①-w CRLF 재직렬화 원복 금지");
        assert_eq!(decide_file_action("round/SESSION_STATE.md", embed, true, Some("LIVE-STATE"),
                       None, true), keep, "② force 여도 불가침");
        assert_eq!(decide_file_action("round/RECOVERY.md", embed, true, None,
                       Some(content_hash(embed).as_str()), false), keep, "③ 읽기 실패도 불가침");
        // ④ 부재 시에는 시드 설치(Write) — seed-once 가 신규 설치까지 막으면 안 된다.
        assert_eq!(decide_file_action("memory/MEMORY.md", embed, false, None, None, false),
                   FileAction::Write { heal_user_copy: false }, "④ 부재 시 시드 설치");
    }

    #[test]
    fn ownership_single_sot_priority_and_exclusivity() {
        // 우선순위 핀(SeedOnce > User): 상태 경로 밑에 user 패턴 이름이 오는 가상 케이스는
        // '상태 보존(불가침)'이 '병합 병치(.new)'보다 안전측 — ownership() 문서의 규정을 박제.
        assert_eq!(ownership("memory/CLAUDE.md"), Ownership::SeedOnce);
        assert_eq!(ownership("memory/X_DIRECTIVE.md"), Ownership::SeedOnce);
        // 등급 대표 핀.
        assert_eq!(ownership("soul.md"), Ownership::User);
        assert_eq!(ownership("round/SESSION_STATE.md"), Ownership::SeedOnce);
        assert_eq!(ownership("bin/javis_phoenix.py"), Ownership::System);
        // 술어 래퍼 배타성: 단일 SOT 의 배타 등급이므로 어떤 rel 에서도 동시 참 불가.
        for rel in ["memory/CLAUDE.md", "memory/MEMORY.md", "soul.md", "CLAUDE.md",
                    "round/SESSION_STATE.md", "round/capability_catalog.json",
                    "schedule.json", "acl.json", "bin/javis_phoenix.py"] {
            assert!(!(is_user_owned(rel) && is_seed_once(rel)), "이중 등급: {rel}");
        }
    }

    /// [회귀 핀·G3 축2] 스코프 인지 소유권 매트릭스 — ① dept 차등은 soul.md 하나(SeedOnce 승격)
    /// ② `ownership(rel)==ownership_scoped(rel, Base)` 항등을 PACK_ALL 전량 스냅샷으로 봉인(Base
    /// 등급표 불변 = base 레인 거동 byte-identical 보증) ③ dept 도 soul 외 전 rel 은 base 와 동일
    /// (차등 최소 원칙 — 조용한 광역 회귀 금지).
    #[test]
    fn ownership_scoped_matrix() {
        // dept soul = SeedOnce(승계 후 불가침) · base soul = User(기존 핀).
        assert_eq!(ownership_scoped("soul.md", PackScope::Dept), Ownership::SeedOnce);
        assert_eq!(ownership_scoped("soul.md", PackScope::Base), Ownership::User);
        assert_eq!(ownership_scoped("sub/soul.md", PackScope::Dept), Ownership::SeedOnce);
        // 상태 경로는 양 스코프 SeedOnce(기존 등급 유지).
        assert_eq!(ownership_scoped("memory/MEMORY.md", PackScope::Base), Ownership::SeedOnce);
        assert_eq!(ownership_scoped("memory/MEMORY.md", PackScope::Dept), Ownership::SeedOnce);
        // ② Base 항등: PACK_ALL 전 rel 전량 대조.
        for (rel, _) in PACK_ALL.iter() {
            assert_eq!(
                ownership(rel),
                ownership_scoped(rel, PackScope::Base),
                "Base 항등 위반: {rel}"
            );
        }
        // ③ dept 는 soul.md 외 전 rel 에서 base 와 동일.
        for (rel, _) in PACK_ALL.iter() {
            if *rel == "soul.md" || rel.ends_with("/soul.md") {
                continue;
            }
            assert_eq!(
                ownership(rel),
                ownership_scoped(rel, PackScope::Dept),
                "dept 차등 과잉(soul 외 등급 변조 금지): {rel}"
            );
        }
        // pack_scope_of: 부서 판정 규칙 등재소(dept_scope_of) 재사용 핀.
        assert_eq!(pack_scope_of(Path::new("/h/.cys/pack")), PackScope::Base);
        assert_eq!(pack_scope_of(Path::new("/h/.cys/pack-dept-sales")), PackScope::Dept);
        assert_eq!(
            pack_scope_of(Path::new("/h/.cys/pack-dept-")),
            PackScope::Base,
            "빈 부서명 = 불량 레인 = base 취급(dept_scope_of None 정합)"
        );
        // CLI 노출 어휘: dept soul 은 "seed-once"(정당한 신규 값), base soul 은 "user"(불변).
        assert_eq!(ownership_name_scoped("soul.md", Path::new("/h/.cys/pack-dept-2")), "seed-once");
        assert_eq!(ownership_name_scoped("soul.md", Path::new("/h/.cys/pack")), "user");
        assert_eq!(
            ownership_name_scoped("bin/javis_phoenix.py", Path::new("/h/.cys/pack-dept-2")),
            "system"
        );
    }

    /// [회귀 핀·G3 축2] decide_file_action 의 dept 스코프 4조합(dept×soul 유/무 + base 대조) —
    /// dept soul 은 존재 시 force·vendor 전진 불문 불가침이고 `.new` 병치가 구조 소멸한다(결함4).
    #[test]
    fn decide_file_action_dept_soul_seed_once() {
        use super::FileAction::*;
        let embed = "EMBED-V2";
        let d = PackScope::Dept;
        // dept × soul 유: vendor 전진(매니페스트 구해시)이어도 병치 없이 불가침.
        assert_eq!(
            super::decide_file_action("soul.md", embed, true, Some("DEPT-SOUL"),
                Some(content_hash("EMBED-V1").as_str()), false, d, None),
            Keep { adopt_hash: false, new_pending: false },
            "dept soul 전진에도 .new 병치 없음(결함4 구조 소멸)"
        );
        // dept × soul 유 + force: 여전히 불가침(SeedOnce 조기 반환).
        assert_eq!(
            super::decide_file_action("soul.md", embed, true, Some("DEPT-SOUL"), None, true, d, None),
            Keep { adopt_hash: false, new_pending: false },
            "dept soul 은 force 여도 불가침"
        );
        // dept × soul 무: 시드 설치(Write) — 승계 본문 치환은 install_into 의 seed-from-base 소관.
        assert_eq!(
            super::decide_file_action("soul.md", embed, false, None, None, false, d, None),
            Write { heal_user_copy: false }
        );
        // base × soul 유(수정+전진): 기존 계약 그대로 .new 병치(대조 핀 — 기존 threeway 매트릭스와 동일).
        assert_eq!(
            super::decide_file_action("soul.md", embed, true, Some("MY-SOUL"),
                Some(content_hash("EMBED-V1").as_str()), false, PackScope::Base, None),
            Keep { adopt_hash: false, new_pending: true },
            "base soul 은 종전대로 병치(base 레인 불변)"
        );
        // base × soul 무: 신규 생성(기존 계약).
        assert_eq!(
            super::decide_file_action("soul.md", embed, false, None, None, false, PackScope::Base, None),
            Write { heal_user_copy: false }
        );
        // dept 에서 soul 외 파일은 base 등급표 그대로(system 강제 치유 불변).
        assert_eq!(
            super::decide_file_action("bin/x.py", embed, true, Some("HACKED"), None, false, d, None),
            Write { heal_user_copy: true },
            "dept 스코프가 system 치유를 흔들면 안 된다"
        );
    }

    /// [T2 회귀 핀·v2 §3] L0-L4 판정표 전수 — super:: 8인자 직호출(embed="E2", eh=hash("E2")).
    #[test]
    fn decide_file_action_l0_l4_matrix() {
        use super::FileAction::*;
        let embed = "E2";
        let eh = content_hash(embed);
        let e1h = content_hash("E1");

        // L0: 잠금 보안 자산 — 전 클래스 즉시 치유.
        assert_eq!(super::decide_file_action("trusted-keys.json", embed, true, Some("HACK"),
                       Some(eh.as_str()), false, PackScope::Base, None),
                   Write { heal_user_copy: true },
                   "★L0>L2: locked 는 벤더 미전진 드리프트도 치유");
        assert_eq!(super::decide_file_action("trusted-keys.json", embed, true, None,
                       None, false, PackScope::Base, None),
                   Write { heal_user_copy: true }, "L0: 판독 불가도 즉시 치유+백업 의무");
        assert_eq!(super::decide_file_action("trusted-keys.json", embed, true, Some("HACK"),
                       Some(e1h.as_str()), false, PackScope::Base, Some("E1")),
                   Write { heal_user_copy: true }, "★L0>L3: locked 병합 금지");

        // L1: 판독 불가 — 치유 유지 + 백업 의무 신호.
        assert_eq!(super::decide_file_action("alerts-config.json", embed, true, None,
                       None, false, PackScope::Base, None),
                   Write { heal_user_copy: true }, "L1: 판독 불가 → 치유+백업 의무");
        assert_eq!(super::decide_file_action("bin/x.py", embed, true, None,
                       Some(eh.as_str()), false, PackScope::Base, None),
                   Write { heal_user_copy: true }, "★L1>L2: 판독 불가는 벤더 미전진이어도 치유");

        // L2: 벤더 미전진 + 드리프트 → 제자리 보존(kept-drift).
        assert_eq!(super::decide_file_action("alerts-config.json", embed, true, Some("DRIFT"),
                       Some(eh.as_str()), false, PackScope::Base, None),
                   KeepDrift, "L2: 벤더 미전진 드리프트 → kept-drift");
        assert_eq!(super::decide_file_action("bin/x.py", embed, true, Some("DRIFT"),
                       Some(eh.as_str()), false, PackScope::Base, Some("E1")),
                   KeepDrift, "★L2>L3: 벤더 미전진이면 base 있어도 보존");

        // L3: 벤더 전진 + 드리프트 + 검증된 base → 3-way.
        assert_eq!(super::decide_file_action("alerts-config.json", embed, true, Some("DRIFT"),
                       Some(e1h.as_str()), false, PackScope::Base, Some("E1")),
                   Merge3, "L3: 벤더 전진+검증 base → Merge3");
        assert_eq!(super::decide_file_action("alerts-config.json", embed, true, Some("DRIFT"),
                       None, false, PackScope::Base, Some("B")),
                   Write { heal_user_copy: true },
                   "★방어: manifest=None∧base=Some 계약위반 입력 → L4(Merge3 아님)");
        assert_eq!(super::decide_file_action("alerts-config.json", embed, true, Some("DRIFT"),
                       Some(e1h.as_str()), false, PackScope::Base, None),
                   Write { heal_user_copy: true }, "★base=None: 벤더 전진+base 부재 → L4 치유");

        // force 폴백: 판독 불가 heal 개정 + !exists 불변.
        assert_eq!(super::decide_file_action("alerts-config.json", embed, true, None,
                       Some(eh.as_str()), true, PackScope::Base, None),
                   Write { heal_user_copy: true }, "force·판독불가 → 백업 의무(폴백 개정)");
        assert_eq!(super::decide_file_action("bin/x.py", embed, false, None,
                       None, true, PackScope::Base, None),
                   Write { heal_user_copy: false }, "force·부재 → 신규 설치(!exists 불변)");

        // fast-path 대조군: 디스크=임베드 → Keep{adopt:true} — verified_base 미참조 증명.
        assert_eq!(super::decide_file_action("bin/x.py", embed, true, Some(embed),
                       None, false, PackScope::Base, Some("E1")),
                   Keep { adopt_hash: true, new_pending: false },
                   "fast-path: base=Some 주입에도 Keep{{adopt:true}} 불변(verified_base 미참조)");
    }

    /// [T2 봉인·v2 §7] SYSTEM_LOCKED 3단 census — ①전 항목 팩 실재 ②전 항목 양 스코프 System
    /// (등급 이탈 = User/SeedOnce 조기 분기가 L0 를 그림자화 — 기계 봉인) ③스냅샷 항등 + 파일명
    /// 패턴 스윕(trusted-keys|.pub|minisign 매치 ⊆ SYSTEM_LOCKED).
    #[test]
    fn system_locked_census() {
        // ① SYSTEM_LOCKED 전 항목이 임베드 팩(PACK_ALL)에 실재.
        for &rel in SYSTEM_LOCKED {
            assert!(PACK_ALL.iter().any(|(r, _)| *r == rel), "SYSTEM_LOCKED 항목 팩 부재: {rel}");
        }
        // ② 전 항목 양 스코프 System — 조기 분기의 L0 그림자화 봉인.
        for &rel in SYSTEM_LOCKED {
            assert_eq!(ownership_scoped(rel, PackScope::Base), Ownership::System,
                       "②Base 스코프 System 이탈: {rel}");
            assert_eq!(ownership_scoped(rel, PackScope::Dept), Ownership::System,
                       "②Dept 스코프 System 이탈: {rel}");
        }
        // ③ 스냅샷 항등 + 패턴 스윕(신규 신뢰 자산이 잠금 등재 없이 팩에 들어오면 여기서 걸린다).
        let locked: Vec<&str> = PACK_ALL.iter().map(|(r, _)| *r)
            .filter(|&r| system_locked(r)).collect();
        assert_eq!(locked, vec!["trusted-keys.json"], "③system_locked 스냅샷 항등");
        for &(rel, _) in PACK_ALL.iter() {
            if rel.contains("trusted-keys") || rel.ends_with(".pub") || rel.contains("minisign") {
                assert!(system_locked(rel), "③패턴 스윕: 신뢰 자산 후보가 SYSTEM_LOCKED 밖: {rel}");
            }
        }
    }

    #[test]
    fn pack_dir_env_precedence_and_legacy_fallbacks() {
        // ★불변식 박제: pack_dir의 4단 폴백 우선순위.
        //   1) CYS_PACK_DIR (env_compat: CYS_ → JAVIS_ → AITERM_PACK_DIR 까지 본다)
        //   2) JAVIS_PACK_DIR (명시 레거시 루프)
        //   3) AITERM_JARVIS_DIR (명시 레거시 루프 — env_compat은 AITERM_PACK_DIR를
        //      만들지 AITERM_JARVIS_DIR가 아니므로 '오직 이 루프'로만 도달 가능)
        //   4) ~/.cys/pack (기본)
        // 마이그레이션 경로라 순서가 뒤집히면 구 설치본을 조용히 못 찾는다.
        // ★W0-a: pack_dir()은 테스트 빌드에서 전 override 미설정 시 panic(격리 봉인)하므로, 4단 폴백
        // 자체를 검증하는 이 테스트는 순수함수(pack_dir_from_env·home_default_pack_dir)를 직접 호출한다
        // (설계 W0 명시 예외). EnvGuard로 각 키를 이전 값 복원형으로 격리한다(패닉 시에도 복원).
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        let _g_cys = EnvGuard::remove("CYS_PACK_DIR");
        let _g_javis = EnvGuard::remove("JAVIS_PACK_DIR");
        let _g_aiterm_pack = EnvGuard::remove("AITERM_PACK_DIR");
        let _g_aiterm_jarvis = EnvGuard::remove("AITERM_JARVIS_DIR");

        // 셋 다 없으면 순수 해석은 None(= 홈 기본 폴백 대상), 홈 기본 경로 끝 2요소가 .cys/pack
        assert!(pack_dir_from_env().is_none(), "override 부재 = None(홈 기본 폴백)");
        assert!(
            home_default_pack_dir().ends_with(".cys/pack"),
            "기본 경로는 .cys/pack: {:?}",
            home_default_pack_dir()
        );

        // AITERM_JARVIS_DIR만 → 3순위로 도달 (env_compat이 못 만드는 키, 루프 전용 경로)
        std::env::set_var("AITERM_JARVIS_DIR", "/legacy/aiterm");
        assert_eq!(pack_dir_from_env(), Some(PathBuf::from("/legacy/aiterm")));

        // JAVIS_PACK_DIR 추가 → AITERM_JARVIS_DIR보다 우선 (2순위)
        std::env::set_var("JAVIS_PACK_DIR", "/legacy/javis");
        assert_eq!(pack_dir_from_env(), Some(PathBuf::from("/legacy/javis")));

        // CYS_PACK_DIR 추가(env_compat primary) → 최우선 (1순위)
        std::env::set_var("CYS_PACK_DIR", "/modern/cys");
        assert_eq!(pack_dir_from_env(), Some(PathBuf::from("/modern/cys")));

        // env_compat 폴백: CYS_PACK_DIR 비우면 JAVIS_PACK_DIR로(=2순위와 동일 키지만
        // env_compat 경로) — 빈 문자열은 미설정 취급이라 다음 후보로 넘어간다
        std::env::set_var("CYS_PACK_DIR", "");
        assert_eq!(pack_dir_from_env(), Some(PathBuf::from("/legacy/javis")));
        // guards drop → 각 키 원값 복원(중간 set_var는 덮어써짐)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // 선언 기반 todo 판정의 팩 정체성(scope_id·scope_exists)
    // ★W0-a: pack_dir()은 테스트 빌드에서 env 미설정 시 panic(격리 봉인)하므로, 여기서도
    // 순수함수(scope_id_of·scope_exists_in)를 직접 호출한다(pack_dir_env_precedence와 동일 예외).
    // ─────────────────────────────────────────────────────────────────────────

    /// scope는 **팩 이름 자체**다. 본사 `pack`·부서 `pack-dept-dept-2` 어느 쪽도 하드코딩하지
    /// 않고 basename으로 산출해야, 부서 팩이 늘어나도 판정이 자동으로 따라온다.
    #[test]
    fn scope_id_is_pack_dir_basename() {
        assert_eq!(scope_id_of(Path::new("/home/u/.cys/pack")), "pack");
        assert_eq!(
            scope_id_of(Path::new("/home/u/.cys/pack-dept-dept-2")),
            "pack-dept-dept-2"
        );
        // 후행 슬래시·`.` 요소는 basename 판정에 영향을 주지 않는다(Python normpath 등가).
        assert_eq!(scope_id_of(Path::new("/home/u/.cys/pack/")), "pack");
        assert_eq!(scope_id_of(Path::new("/home/u/.cys/pack/.")), "pack");
    }

    /// 실재 판정은 **팩의 형제 디렉터리 존재**로만 한다 — 시간 의존 0·결정론.
    /// 부재(orphan-scope)와 실재(foreign-scope)를 가르는 이 한 번의 stat이, 부서 teardown·개명
    /// 시 살아있는 파일이 통째로 조용히 사라지는 것(07-11 사고의 거울상)을 막는다.
    #[test]
    fn scope_exists_checks_sibling_pack_dirs_only() {
        let root = std::env::temp_dir().join(format!(
            "cys-scope-exists-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let me = root.join("pack-dept-dept-2");
        std::fs::create_dir_all(&me).unwrap();
        std::fs::create_dir_all(root.join("pack-dept-dept-1")).unwrap();
        std::fs::write(root.join("pack-dept-dept-7"), b"file, not dir").unwrap();

        assert!(scope_exists_in(&me, "pack-dept-dept-1"), "형제 팩 실재");
        assert!(scope_exists_in(&me, "pack-dept-dept-2"), "자기 자신도 실재");
        assert!(!scope_exists_in(&me, "pack-dept-dept-9"), "부재 = orphan-scope 근거");
        assert!(!scope_exists_in(&me, "pack-dept-dept-7"), "파일은 팩이 아니다");
        // 경로 탈출 방어 — G4 값 문법상 정상 선언엔 나올 수 없지만 소비자 쪽에서도 막는다.
        for bad in ["", ".", "..", "../pack-dept-dept-1", "a/b", "a\\b"] {
            assert!(!scope_exists_in(&me, bad), "경로 탈출 후보가 통과했다: {bad:?}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// G3 축1: 부서 판정 순수 함수 진리표 — `pack-dept-` 접두(cys-dept `dept_pack`·
    /// preflight `_pack_is_dept` 와 동일 규칙 · H-SEED-6 파리티 핀의 Rust 측).
    #[test]
    fn dept_scope_of_matrix() {
        assert_eq!(dept_scope_of(Path::new("/h/.cys/pack")), None, "base 팩은 부서 아님");
        assert_eq!(
            dept_scope_of(Path::new("/h/.cys/pack-dept-dept-2")),
            Some("dept-2".to_string())
        );
        assert_eq!(dept_scope_of(Path::new("/h/.cys/pack-dept-sales")), Some("sales".to_string()));
        assert_eq!(dept_scope_of(Path::new("/h/.cys/pack-dept-")), None, "빈 부서명 = 불량 레인");
        assert_eq!(dept_scope_of(Path::new("/h/.cys/pack-notes")), None, "접두 유사 비부서");
        assert_eq!(dept_scope_of(Path::new("/h/.cys/claude")), None);
    }

    /// [회귀 핀·G3 축1] base 레인 거동 박제 — 팩이 `pack`(비부서)이면 시드 표적은 종전과
    /// byte-identical 하게 `<부모>/claude` 다. **CYS_ACCOUNT_DIR 이 잔류해 있어도** base 레인은
    /// 영향 0(성찰 위험④ 핀 — account-dir 참조는 부서 스코프 분기 안에만 있다).
    #[test]
    fn config_dir_base_unchanged() {
        let pack = Path::new("/t/.cys/pack");
        let scope = dept_scope_of(pack);
        assert_eq!(
            config_dir_for(None, scope.as_deref(), None, pack),
            Some(PathBuf::from("/t/.cys/claude"))
        );
        assert_eq!(
            config_dir_for(None, scope.as_deref(), Some("/t/acct"), pack),
            Some(PathBuf::from("/t/.cys/claude")),
            "base 레인에 CYS_ACCOUNT_DIR 우발 발효 금지"
        );
        // CYS_CONFIG_DIR 1순위 불변 핀
        assert_eq!(
            config_dir_for(Some("/cfg"), scope.as_deref(), Some("/t/acct"), pack),
            Some(PathBuf::from("/cfg"))
        );
    }

    /// G3 축1(확정 재설계): 부서 스코프 시드 표적 — CYS_ACCOUNT_DIR 있으면 그 dir(실소비 SOT),
    /// 없으면 **None**(시드 생략 신호). 레거시 폴백 dir(claude-dept-<name>)은 만들지 않는다.
    #[test]
    fn config_dir_dept_scope() {
        let pack = Path::new("/t/.cys/pack-dept-2");
        let scope = dept_scope_of(pack);
        assert_eq!(scope.as_deref(), Some("2"));
        assert_eq!(
            config_dir_for(Some("/cfg"), scope.as_deref(), Some("/acct"), pack),
            Some(PathBuf::from("/cfg")),
            "CYS_CONFIG_DIR 1순위는 부서 스코프에서도 불변"
        );
        assert_eq!(
            config_dir_for(None, scope.as_deref(), Some("/acct"), pack),
            Some(PathBuf::from("/acct"))
        );
        assert_eq!(config_dir_for(None, scope.as_deref(), Some(""), pack), None, "빈 값 = 부재");
        assert_eq!(
            config_dir_for(None, scope.as_deref(), None, pack),
            None,
            "부서+무acct = 시드 생략(공용 claude 폴백 금지 — 결함2 재발 방지)"
        );
    }

    /// ★★M5 검체(2026-08-24 자기성찰 3회전) — **설치 표적 ≠ 실소비 SOT** 를 잡는다.
    ///
    /// 【고치는 결함】 base 레인 설치 표적은 `pack.parent()/claude`(팩 위치 파생)이고 에이전트의
    /// 실소비는 `${CYS_ACCOUNT_DIR:-$HOME/.cys/claude}`(팩 위치 무관)다. 두 값은 팩이
    /// `~/.cys/pack` 일 때만 **우연히** 일치한다. 어긋나면 각성 훅이 아무도 읽지 않는 폴더에
    /// 설치되고, 노드는 떠도 `/clear` 후 지침 재주입·마스터 선언 부트 발화가 영구히 죽는다.
    /// 회전2 격리 주행에서 처방된 `cys init-pack`·`javis_preflight.py --fix` 를 **완주시켰는데도
    /// 같은 경고가 재현**된 원인이 이것이다(BLOCK-2 와 같은 부류의 재발).
    ///
    /// 【왜 이 검체가 필요한가 — 스스로는 절대 발견되지 않는다】 **기본 경로에서는 두 값이
    /// 우연히 일치하므로 어떤 통합 검체도 영원히 초록**이다. 그래서 여기서는 경로를 주입해
    /// 어긋난 조합을 **직접 만든다**(실기·env 의존 0).
    #[test]
    fn config_target_mismatch_detects_the_accidentally_aligned_default() {
        let home = Path::new("/h");
        let consumed = home.join(".cys").join("claude"); // ${CYS_ACCOUNT_DIR:-$HOME/.cys/claude}

        // ⓐ ★기본 경로 — 팩이 `~/.cys/pack` 이라 **우연히** 일치한다(그래서 검체가 안 잡힌다).
        let default_pack = home.join(".cys").join("pack");
        let target = config_dir_for(None, dept_scope_of(&default_pack).as_deref(), None, &default_pack);
        assert_eq!(target.as_deref(), Some(consumed.as_path()), "드릴 전제: 기본은 일치한다");
        assert_eq!(
            config_target_mismatch(target.as_deref(), &consumed),
            None,
            "정상 기계에서 거짓 경보 — 소음이 진짜 신호를 묻는다"
        );

        // ⓑ ★핵심 — 팩이 다른 곳에 있으면(개발 트리·임시 팩·CYS_PACK_DIR 지정·이주 중)
        //    설치 표적만 따라 움직이고 실소비는 그대로다. 이때 훅은 아무도 안 읽는 곳에 앉는다.
        for pack in [
            Path::new("/h/dev/cys-terminal-rel/pack"),
            Path::new("/tmp/cys-pack-XXXX/pack"),
            Path::new("/h/.cys-alt/pack"),
        ] {
            let t = config_dir_for(None, dept_scope_of(pack).as_deref(), None, pack);
            let hit = config_target_mismatch(t.as_deref(), &consumed);
            assert!(
                hit.is_some(),
                "어긋난 팩({})에서 표적 불일치를 놓쳤다 — 각성 훅이 침묵으로 죽는다",
                pack.display()
            );
            let (install, consume) = hit.unwrap();
            assert_ne!(install, consume, "불일치 보고가 같은 경로를 낸다");
        }

        // ⓒ `.`/`..` 만 다른 같은 경로는 **거짓 경보가 아니다**(정규화 후 비교).
        assert_eq!(
            config_target_mismatch(Some(Path::new("/h/.cys/pack/../claude")), &consumed),
            None,
            "어휘적 차이 하나로 거짓 경보 — 소음"
        );
        assert_eq!(
            config_target_mismatch(Some(Path::new("/h/./.cys/claude")), &consumed),
            None
        );

        // ⓓ 설치 표적 자체가 없으면(부서 스코프 + CYS_ACCOUNT_DIR 부재) **대조 불가**다 —
        //    없는 것을 불일치로 보고하면 그 좌석의 진짜 진단(dept-awakening-seed)이 묻힌다.
        let dept = Path::new("/h/.cys/pack-dept-sales");
        let t = config_dir_for(None, dept_scope_of(dept).as_deref(), None, dept);
        assert_eq!(t, None, "드릴 전제: 부서+무acct 는 시드 표적이 없다");
        assert_eq!(config_target_mismatch(t.as_deref(), &consumed), None);

        // ⓔ 부서 레인에서 CYS_ACCOUNT_DIR 이 주입되면 표적 = 실소비 = 그 dir 다(일치).
        let acct = Path::new("/h/.cys/claude-sales");
        let t = config_dir_for(None, dept_scope_of(dept).as_deref(), acct.to_str(), dept);
        assert_eq!(config_target_mismatch(t.as_deref(), acct), None);
        // 그런데 실소비가 그 값을 **안 물려받으면**(부서 부트 밖 기동) 다시 어긋난다.
        assert!(config_target_mismatch(t.as_deref(), &consumed).is_some());
    }

    /// [무변조 계약 핀·G3 축1] 부서 팩 설치는 ①공용 <base>/claude 를 **byte-identical** 로 두고
    /// ②CYS_ACCOUNT_DIR(acctdir)에 각성 훅 2종을 결정론 시드하며 ③acct 부재 시엔 시드를 통째로
    /// 생략한다(레거시 폴백 dir 미생성). 결함2(공용 프로필 오염)의 기계 증거.
    #[test]
    fn dept_install_never_touches_shared_claude() {
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!(
            "cys-dept-install-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&td);
        let dpack = td.join("pack-dept-2");
        std::fs::create_dir_all(&dpack).unwrap();
        let shared = td.join("claude");
        std::fs::create_dir_all(&shared).unwrap();
        let shared_settings = shared.join("settings.json");
        let shared_body = r#"{"theme":"dark"}"#;
        std::fs::write(&shared_settings, shared_body).unwrap();
        let acct = td.join("acct");
        let items = [("CLAUDE.md.template", "router\n")];

        // ① acct 주입 설치: 공용 무변조 + acctdir 시드
        {
            let _g1 = EnvGuard::set(ENV_PACK_DIR, &dpack);
            let _g2 = EnvGuard::remove(ENV_CONFIG_DIR);
            let _g3 = EnvGuard::set("CYS_ACCOUNT_DIR", &acct);
            install_into(
                dpack.clone(),
                items.iter().copied(),
                false,
                "1.0.0",
                false,
                true,
                pack_scope_of(&dpack),
                None,
                None,
            )
            .unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&shared_settings).unwrap(),
            shared_body,
            "부서 설치가 공용 claude settings 를 변조했다(결함2 재발)"
        );
        assert!(
            verify_desired_hooks_registered(
                &acct.join("settings.json"),
                &dpack,
                &AWAKENING_HOOKS
            )
            .is_empty(),
            "부서 acctdir 에 각성 훅 2종이 시드되지 않았다"
        );

        // ② acct 부재 설치: 시드 생략(공용 무변조 유지 + 폴백 dir 미생성)
        let dpack3 = td.join("pack-dept-3");
        std::fs::create_dir_all(&dpack3).unwrap();
        {
            let _g1 = EnvGuard::set(ENV_PACK_DIR, &dpack3);
            let _g2 = EnvGuard::remove(ENV_CONFIG_DIR);
            let _g3 = EnvGuard::remove("CYS_ACCOUNT_DIR");
            install_into(
                dpack3.clone(),
                items.iter().copied(),
                false,
                "1.0.0",
                false,
                true,
                pack_scope_of(&dpack3),
                None,
                None,
            )
            .unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&shared_settings).unwrap(),
            shared_body,
            "acct 부재 부서 설치가 공용 claude 로 폴백했다(fail-closed 위반)"
        );
        // (H-SEED-6 파리티 핀이 코드라인의 폴백 dir 리터럴 부재를 감시하므로 조립해 쓴다)
        assert!(
            !td.join(format!("{}-{}", "claude-dept", 3)).exists(),
            "레거시 폴백 dir 가 생성됐다(아무도 안 읽는 사각 디렉터리 금지 — BLOCKER 확정 위반)"
        );
        let _ = std::fs::remove_dir_all(&td);
    }

    /// [회귀 핀·G3 축2] 부서 soul seed-from-base 전 수명주기 — ① 최초 시드는 임베드 템플릿이
    /// 아니라 **base 팩(형제 `pack`)의 현행 soul.md 승계**이고 매니페스트 해시·pristine 이 시드본과
    /// 일관 ② vendor 전진(임베드 변경) 재설치에도 불가침 + `soul.md.new` 미생성(결함4 병치 소멸)
    /// ③ base soul 부재 시 임베드 폴백 + 원장 강등 라인 ④ 구버전이 남긴 `.new`·pending 잔재는
    /// 다음 설치가 정리(마이그레이션 계약) — 승계/강등 사실은 `.merge-audit.jsonl` 이 영속 증언.
    /// ★결함3(2026-08-22) 이후: 시드본은 `정체 스탬프 + 승계본`이다 — 등가 비교는 "승계본을 꼬리로
    /// 그대로 담는가"로 바뀌었을 뿐, 해시 일관(매니페스트·pristine·원장)의 불변식은 그대로다.
    #[test]
    fn dept_soul_seeds_from_base() {
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!(
            "cys-dept-soulseed-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&td);
        let base_pack = td.join("pack");
        std::fs::create_dir_all(&base_pack).unwrap();
        let base_soul = "# CEO-앵커-헌장\n안전핵 denylist 불변 · kill-switch 즉시 정지\n";
        std::fs::write(base_pack.join("soul.md"), base_soul).unwrap();
        let dpack = td.join("pack-dept-2");
        std::fs::create_dir_all(&dpack).unwrap();
        let _env = set_pack_env(&dpack, td.join("cfg"));
        let items = [("soul.md", "TEMPLATE-SOUL-V1"), ("README.md", "R1")];

        // ① 최초 설치: 부서 soul == base 승계본(템플릿 아님) + 매니페스트·pristine 일관 + 원장 pass.
        install_into(
            dpack.clone(),
            items.iter().copied(),
            false,
            "1.0.0",
            false,
            false,
            pack_scope_of(&dpack),
            None,
            None,
        )
        .unwrap();
        let read = |p: &Path| std::fs::read_to_string(p).unwrap();
        let seeded = read(&dpack.join("soul.md"));
        assert!(
            seeded.ends_with(base_soul),
            "부서 soul 은 base 헌장 승계본을 꼬리로 그대로 담는다: {seeded}"
        );
        assert!(
            seeded.starts_with("# 부서 정체 — 2 부서장 마스터"),
            "결함3(a): 승계본 **앞**에 부서장 정체 스탬프가 박힌다: {seeded}"
        );
        let manifest: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&read(&dpack.join(INSTALL_MANIFEST))).unwrap();
        assert_eq!(
            manifest.get("soul.md"),
            Some(&content_hash(&seeded)),
            "매니페스트 해시는 시드본 기준(치환 content 일관 흐름)"
        );
        assert_eq!(
            read(&dpack.join(PRISTINE_DIR).join("soul.md")),
            seeded,
            "pristine 도 시드본(3-way 조상 일관)"
        );
        let audit = read(&dpack.join(MERGE_AUDIT_FILE));
        assert!(
            audit.contains("\"action\":\"seed-from-base\""),
            "원장에 seed-from-base 라인: {audit}"
        );
        assert!(audit.contains("\"verify_result\":\"pass\""), "안전핵 포함 승계 = pass: {audit}");
        assert!(
            audit.contains(&content_hash(&seeded)),
            "after_sha256 = 시드본 해시"
        );

        // ② vendor 전진 가장(임베드 변경) 재설치: 불가침 + .new 미생성(결함4 핀) + 원장 재발화 없음.
        let items2 = [("soul.md", "TEMPLATE-SOUL-V2"), ("README.md", "R2")];
        install_into(
            dpack.clone(),
            items2.iter().copied(),
            false,
            "1.0.1",
            false,
            false,
            pack_scope_of(&dpack),
            None,
            None,
        )
        .unwrap();
        assert_eq!(read(&dpack.join("soul.md")), seeded, "vendor 전진에도 부서 soul 불가침");
        assert!(!dpack.join("soul.md.new").exists(), "결함4: dept soul 의 .new 병치 구조 소멸");
        assert!(load_merge_pending(&dpack).get("soul.md").is_none(), "병합 대기 미생성");
        let audit2 = read(&dpack.join(MERGE_AUDIT_FILE));
        assert_eq!(
            audit2.matches("seed-from-base").count(),
            1,
            "시드는 최초 1회만(존재 시 재발화 금지)"
        );

        // ③ base soul 부재: 임베드 템플릿 폴백 + 원장 강등 라인(fail-open 아님 — soul 없는 부팅 방지).
        std::fs::remove_file(base_pack.join("soul.md")).unwrap();
        let dpack3 = td.join("pack-dept-3");
        std::fs::create_dir_all(&dpack3).unwrap();
        install_into(
            dpack3.clone(),
            items.iter().copied(),
            false,
            "1.0.0",
            false,
            false,
            pack_scope_of(&dpack3),
            None,
            None,
        )
        .unwrap();
        let seeded3 = read(&dpack3.join("soul.md"));
        assert!(
            seeded3.ends_with("TEMPLATE-SOUL-V1"),
            "base 부재 시 임베드 폴백(부서가 soul 없이 뜨지 않는다): {seeded3}"
        );
        assert!(
            seeded3.starts_with("# 부서 정체 — 3 부서장 마스터"),
            "결함3(a): 폴백 경로에도 정체 스탬프가 붙는다(정체 없이 뜨는 부서장 0): {seeded3}"
        );
        assert!(
            read(&dpack3.join(MERGE_AUDIT_FILE)).contains("degraded-template-fallback"),
            "강등 사실 원장 기록"
        );

        // ④ 마이그레이션: 구버전이 남긴 dept soul 의 .new 병치·pending 잔재를 다음 설치가 정리.
        let dpack4 = td.join("pack-dept-4");
        std::fs::create_dir_all(&dpack4).unwrap();
        std::fs::write(dpack4.join("soul.md"), "OLD-DEPT-SOUL").unwrap();
        std::fs::write(dpack4.join("soul.md.new"), "STALE-VENDOR").unwrap();
        std::fs::write(
            dpack4.join(MERGE_PENDING_FILE),
            serde_json::json!({
                "soul.md": {"kind": "new-pending", "side": "soul.md.new", "version": "0.9.0", "ts": 1}
            })
            .to_string(),
        )
        .unwrap();
        install_into(
            dpack4.clone(),
            items.iter().copied(),
            false,
            "1.0.0",
            false,
            false,
            pack_scope_of(&dpack4),
            None,
            None,
        )
        .unwrap();
        assert_eq!(read(&dpack4.join("soul.md")), "OLD-DEPT-SOUL", "기존 부서 soul 소급 교체 금지");
        assert!(!dpack4.join("soul.md.new").exists(), "구 병치본 정리(compat 계약)");
        assert!(load_merge_pending(&dpack4).get("soul.md").is_none(), "구 pending 정리");

        let _ = std::fs::remove_dir_all(&td);
    }

    /// [회귀 핀·결함3 · 2026-08-22 오너 기계 실측 사고] 부서장이 자신을 '부서장'으로 인식할
    /// **정의처**가 없어 부서 마스터가 정체 없이 깨어난 사고 + seed-from-base 가 본부 전용 절을
    /// 통째로 물려줘 부서장이 자신을 CEO 로 오인할 수 있는 결함의 회귀 핀.
    ///   ① 부서 스코프 시드 결과에 **부서명 + "부서장"** 문자열과 각성 보고 지시가 실재
    ///   ② 가드 마커 절(오너 기계 base soul 의 실제 문안)이 승계본에서 **하위 절까지 통째로** 제거
    ///      되고, 마커 없는 이웃 절은 온전(보수적 드롭)
    ///   ③ base 스코프는 스탬프 0·시드 원장 0 — 임베드 그대로(byte-identical 계약)
    ///   ④ 드롭·부서명이 `.merge-audit.jsonl` 에 감사 가능하게 남는다
    ///   ⑤ staging(`.pack-staging-init-*`) 대상에서도 부서명이 논리 대상 env 로 복원된다
    #[test]
    fn dept_soul_stamps_identity_and_drops_base_only_sections() {
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!(
            "cys-dept-soulstamp-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&td);
        let base_pack = td.join("pack");
        std::fs::create_dir_all(&base_pack).unwrap();
        // 오너 기계의 실제 base soul 형태 — 가드 절은 **정본 마커 한 줄**(규약 v3)로 표시하고,
        // 옛 평문 안내(인용문)는 그대로 둔다: 평문은 더 이상 마커가 아니지만 그 절에 정본 마커가
        // 있으므로 드롭돼야 한다(평문만으로 드롭되지 않는 것은 legacy_plaintext 핀이 따로 못박음).
        let base_soul = "# soul.md — 운영 헌장 (최소 골격)\n\
                         \n\
                         ## 정체\n\
                         \n\
                         - 이 시스템의 주인(이하 \"오너\"): 홍길동\n\
                         - 안전핵 denylist 불변 · kill-switch 즉시 정지\n\
                         \n\
                         ## CEO 정체·부서 티켓 (오너 명령 집행 2026-08-22)\n\
                         \n\
                         <!-- cys:no-inherit -->\n\
                         \n\
                         > ★**본부(base) 레인 전용 절 — 승계 금지 가드**: 부서 팩의 soul.md 는 이\n\
                         > 파일을 승계해 시드된다. 이 절이 부서에 복사되면 부서장이 자신을 CEO 로 오인한다.\n\
                         \n\
                         - 이 데몬의 master 는 **CEO(master of master)** 다.\n\
                         \n\
                         ### 부서장 티켓 자동 발급\n\
                         \n\
                         - 부서장 요청을 받으면 즉시 발급한다.\n\
                         \n\
                         ## 금지선 (denylist)\n\
                         \n\
                         - 외부 발행 — 비가역\n";
        std::fs::write(base_pack.join("soul.md"), base_soul).unwrap();
        let dpack = td.join("pack-dept-영업");
        std::fs::create_dir_all(&dpack).unwrap();
        let _env = set_pack_env(&dpack, td.join("cfg"));
        let items = [("soul.md", "TEMPLATE-SOUL-V1"), ("README.md", "R1")];
        install_into(
            dpack.clone(),
            items.iter().copied(),
            false,
            "1.0.0",
            false,
            false,
            pack_scope_of(&dpack),
            None,
            None,
        )
        .unwrap();
        let read = |p: &Path| std::fs::read_to_string(p).unwrap();
        let seeded = read(&dpack.join("soul.md"));

        // ① 정체 스탬프 — 부서명이 박힌 구체 문장 + 각성 보고 지시 + CEO 보고선.
        assert!(seeded.contains("영업 부서장 마스터"), "부서명+부서장 정체 문장 부재: {seeded}");
        assert!(seeded.contains("부서장"), "'부서장' 문자열 부재");
        assert!(
            seeded.contains("각성 보고 첫 문장에 부서장 정체를 명시하라"),
            "각성 보고 첫 문장 지시 부재: {seeded}"
        );
        assert!(
            seeded.contains("본부(base) 데몬의 master 는 **CEO** 다"),
            "CEO 보고선(본부=CEO · 부서장→CEO) 부재: {seeded}"
        );
        assert!(
            seeded.contains("오너의 직접 지시가 항상 최우선"),
            "오너 직접 지시 최우선 조항 부재: {seeded}"
        );

        // ② 본부 전용 절 승계 차단 — 가드 절 + 하위 절까지 통째로, 이웃 절은 온전.
        assert!(!seeded.contains("승계 금지 가드"), "가드 마커 줄이 승계됐다: {seeded}");
        assert!(
            !seeded.contains("CEO(master of master)"),
            "본부 전용 CEO 정체 절이 부서로 승계됐다(부서장이 CEO 로 오인): {seeded}"
        );
        assert!(
            !seeded.contains("부서장 티켓 자동 발급"),
            "가드 절의 **하위 절**이 남았다(반쪽 삭제): {seeded}"
        );
        assert!(seeded.contains("## 금지선 (denylist)"), "마커 없는 이웃 절은 온전해야 한다");
        assert!(seeded.contains("홍길동"), "마커 없는 상위 절(정체)도 온전해야 한다");
        assert!(
            seeded.contains("- 외부 발행 — 비가역"),
            "가드 절 뒤 이웃 절 본문이 잘렸다: {seeded}"
        );

        // ④ 감사 원장 — 드롭 flag·부서명·드롭된 heading.
        let audit = read(&dpack.join(MERGE_AUDIT_FILE));
        assert!(audit.contains("\"guard-dropped\""), "드롭 flag 원장 기록 부재: {audit}");
        assert!(audit.contains("\"dept\":\"영업\""), "부서명 원장 기록 부재: {audit}");
        assert!(
            audit.contains("## CEO 정체·부서 티켓"),
            "드롭된 절 heading 이 원장에 없다(감사 불가): {audit}"
        );

        // ③ base 스코프 불변 — 스탬프 0·시드 원장 0(임베드 그대로).
        let bpack = td.join("pack-base");
        std::fs::create_dir_all(&bpack).unwrap();
        assert_eq!(pack_scope_of(&bpack), PackScope::Base, "계측 타당성: base 스코프여야 한다");
        install_into(
            bpack.clone(),
            items.iter().copied(),
            false,
            "1.0.0",
            false,
            false,
            pack_scope_of(&bpack),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            read(&bpack.join("soul.md")),
            "TEMPLATE-SOUL-V1",
            "base 레인 거동 불변 — 정체 스탬프·승계 없음(byte-identical 계약)"
        );
        assert!(
            !bpack.join(MERGE_AUDIT_FILE).exists(),
            "base 레인에서 seed-from-base 원장이 생겼다(부서 전용 경로 누수)"
        );

        // ⑤ staging 대상(부서명이 basename 에 없음) — 논리 대상(pack env)에서 부서명 복원.
        let staging = init_staging_dir(&dpack);
        assert_eq!(dept_scope_of(&staging), None, "계측 타당성: staging basename 엔 부서 정보가 없다");
        assert_eq!(
            dept_name_for_seed(&staging).as_deref(),
            Some("영업"),
            "staging 경로에서 부서명이 유실되면 정체 스탬프가 '(부서명 미상)'으로 강등된다"
        );

        let _ = std::fs::remove_dir_all(&td);
    }

    /// [회귀 핀·★2차 BLOCK 부수3 · **조용한 미탐 차단**] v3 가 레거시 평문 규칙을 삭제한 결과
    /// **옛 표기만 가진 base soul 은 본부 전용 문안까지 그대로 승계된다**. 이것은 과삭제보다 안전한
    /// 실패 방향이라는 오너 결정이지만, **조용하면 안 된다** — 판정(드롭)은 그대로 두고 **관측만**
    /// 얹는 것이 계약이다. 이 핀은 그 관측이 실제 시드 경로에서 stderr 뿐 아니라 **원장 flag**
    /// (`guard-marker-unrecognized`)까지 도달하는지를 못 박는다. 단위 테스트는 `GuardStrip.unrecognized`
    /// 필드까지만 보므로, 이 핀이 없으면 flags 배선을 지워도 전부 green 이다(무증거 green 재발).
    #[test]
    fn legacy_only_base_soul_inherits_loudly_not_silently() {
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!(
            "cys-dept-legacyonly-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&td);
        let base_pack = td.join("pack");
        std::fs::create_dir_all(&base_pack).unwrap();
        // 정본 마커가 **하나도 없는** base soul — 옛 평문 표기만 붙어 있다(이주 전 오너 기계 상태).
        let base_soul = "# soul.md — 운영 헌장 (최소 골격)\n\
                         \n\
                         ## 정체\n\
                         \n\
                         - 이 시스템의 주인(이하 \"오너\"): 홍길동\n\
                         - 안전핵 denylist 불변 · kill-switch 즉시 정지\n\
                         \n\
                         ## CEO 정체·부서 티켓\n\
                         \n\
                         > ★**본부(base) 레인 전용 절 — 승계 금지 가드**: 이 절이 부서에 복사되면\n\
                         > 부서장이 자신을 CEO 로 오인한다.\n\
                         \n\
                         - 이 데몬의 master 는 **CEO(master of master)** 다.\n\
                         \n\
                         ## 금지선 (denylist)\n\
                         \n\
                         - 외부 발행 — 비가역\n";
        std::fs::write(base_pack.join("soul.md"), base_soul).unwrap();
        let dpack = td.join("pack-dept-총무");
        std::fs::create_dir_all(&dpack).unwrap();
        let _env = set_pack_env(&dpack, td.join("cfg"));
        install_into(
            dpack.clone(),
            [("soul.md", "TEMPLATE-SOUL-V1")].iter().copied(),
            false,
            "1.0.0",
            false,
            false,
            pack_scope_of(&dpack),
            None,
            None,
        )
        .unwrap();
        let seeded = std::fs::read_to_string(dpack.join("soul.md")).unwrap();

        // ① 판정은 바뀌지 않는다 — 평문만으로는 드롭하지 않는다(과삭제 0 원칙 유지).
        assert!(
            seeded.contains("CEO(master of master)"),
            "평문 표기를 마커로 되살려 절을 삭제했다(2차 BLOCK 재발): {seeded}"
        );
        // ② 그래도 정체 스탬프는 앞에 붙어 부서장이 자기 정체를 먼저 읽는다(미탐의 실피해 완충).
        assert!(seeded.contains("총무 부서장 마스터"), "정체 스탬프가 빠졌다: {seeded}");

        // ③ ★관측 — 원장에 미탐 flag 가 남아야 한다(조용한 승계 금지).
        let audit = std::fs::read_to_string(dpack.join(MERGE_AUDIT_FILE)).unwrap();
        assert!(
            audit.contains("\"guard-marker-unrecognized\""),
            "옛 표기만 가진 base soul 의 미탐이 **원장에 남지 않았다**(조용한 미탐): {audit}"
        );
        assert!(
            !audit.contains("\"guard-dropped\""),
            "드롭이 없는데 드롭 flag 가 찍혔다(원장 오보): {audit}"
        );

        let _ = std::fs::remove_dir_all(&td);
    }

    /// [회귀 핀·결함3(b)] 승계 금지 가드 마커 파서는 **보수적**이다 — 마커가 명확할 때만, 그
    /// 절(+하위 절)만 드롭한다. 오탐 삭제가 미탐 승계보다 위험하므로 부정 케이스를 함께 못 박는다.
    #[test]
    fn strip_no_inherit_sections_drops_only_marked_sections() {
        // ⓐ 마커 0건 = 입력 바이트 그대로(개행 정규화조차 없음).
        let plain = "# T\r\n\r\n## A\r\n본문\r\n## B\r\n끝";
        let r = strip_no_inherit_sections(plain);
        assert_eq!(r.kept, plain, "마커 없으면 승계본은 바이트 불변(CRLF 포함)");
        assert!(r.dropped.is_empty());

        // ⓑ 마커 절 + 하위 절 드롭 · 이웃 절 온전 · 정본 마커(HTML 주석 한 줄) 인식.
        let src = "# 제목\n\n## 유지\nU\n\n## 본부 전용\n<!-- cys:no-inherit -->\nX\n\n### 하위\nY\n\n## 뒤\nZ\n";
        let r = strip_no_inherit_sections(src);
        assert_eq!(r.kept, "# 제목\n\n## 유지\nU\n\n## 뒤\nZ\n", "드롭 결과: {}", r.kept);
        assert_eq!(r.dropped, vec!["## 본부 전용".to_string()]);
        assert!(r.refused.is_empty());

        // ⓒ 하위 절의 마커는 **하위 절만** 드롭(상위 절을 끌어내리지 않는다).
        let src = "## 상위\nP\n\n### 하위\n<!-- cys:no-inherit -->\nQ\n\n## 다음\nR\n";
        let r = strip_no_inherit_sections(src);
        assert_eq!(r.kept, "## 상위\nP\n\n## 다음\nR\n", "상위 절 과삭제: {}", r.kept);
        assert_eq!(r.dropped, vec!["### 하위".to_string()]);

        // ⓓ 닫힌 코드펜스 안의 '#' 는 heading 이 아니다 — 절 경계 오판으로 반쪽 삭제되지 않는다.
        //   (문서 몸통 보호 규칙에 걸리지 않도록 H1 아래의 하위 절을 대상으로 잡는다 — 실제 규약
        //    "본부 전용 문안은 하위 절로 표시한다"와 같은 형태다.)
        let src = "# T\n\n## 본부 전용\ncys:no-inherit\n```md\n## 가짜 heading\n```\n꼬리\n\n## 유지\nK\n";
        let r = strip_no_inherit_sections(src);
        assert_eq!(r.kept, "# T\n\n## 유지\nK\n", "펜스 안 '#' 를 heading 으로 오판했다: {}", r.kept);
        assert_eq!(r.dropped.len(), 1);

        // ⓔ 첫 heading 이전(preamble)의 마커는 절이 아니므로 아무것도 드롭하지 않는다.
        //    단 **조용하지 않다** — 인식 실패 흔적이 남는다(마커를 붙였는데 무동작 금지).
        let src = "<!-- cys:no-inherit -->\n\n## 유지\nK\n";
        let r = strip_no_inherit_sections(src);
        assert_eq!(r.kept, src, "preamble 마커로 문서가 잘렸다");
        assert!(r.dropped.is_empty());
        assert_eq!(r.unrecognized.len(), 1, "preamble 마커 무동작이 조용히 지나갔다");

        // ⓕ 계측 타당성: 마커 표기법을 **코드블록으로 문서화한** 절은 삭제되지 않는다(리뷰어 실측).
        let src = "# T\n\n## 마커 사용법\n다음처럼 적는다:\n```md\n<!-- cys:no-inherit -->\n```\n설명 계속\n\n## 유지\nK\n";
        let r = strip_no_inherit_sections(src);
        assert!(
            r.dropped.is_empty(),
            "코드펜스 안의 마커 예시로 문서화 절이 삭제됐다: {:?}",
            r.dropped
        );
        assert_eq!(r.kept, src, "바이트 불변");
        assert!(r.unrecognized.is_empty(), "펜스 안 예시는 인식 실패로도 시끄럽지 않아야 한다");
    }

    /// [회귀 핀·★F2 치명 · 2026-08-22 적대 리뷰 BLOCK] **실제 배포본**(`PACK_ALL` 임베드
    /// `soul.md`)으로 도는 핀. v1 파서는 이 파일의 안내 문장에 들어 있던 평문("승계 금지" 등)을
    /// 마커로 오인하고, 그것이 문서 루트(H1) 직속 본문이라 **문서 전량**을 드롭했다 —
    /// 실측 `DROPPED=["# soul.md — 운영 헌장 (최소 골격)"] · KEPT 0 bytes`. 합성 문자열 핀은
    /// 그때도 전부 green 이었다(**green 이 무증거**였다). 그래서 이 핀만이 재발을 막는다.
    #[test]
    fn shipped_soul_md_is_inherited_bytewise() {
        let shipped = PACK_ALL
            .iter()
            .find(|(rel, _)| *rel == "soul.md")
            .map(|(_, c)| *c)
            .expect("PACK_ALL 에 soul.md 가 없다(계측 타당성 실패)");
        let r = strip_no_inherit_sections(shipped);
        assert!(
            r.dropped.is_empty(),
            "출하 soul.md 가 자기 가드에 걸려 드롭됐다(F2 재발): {:?}",
            r.dropped
        );
        assert!(r.refused.is_empty(), "출하 soul.md 에서 드롭 거부가 발생했다: {:?}", r.refused);
        assert!(
            r.unrecognized.is_empty(),
            "출하 soul.md 에 효력 없는 마커 토큰이 있다(표기 오류): {:?}",
            r.unrecognized
        );
        assert_eq!(r.kept, shipped, "출하 soul.md 승계본은 바이트 불변이어야 한다");
        assert!(!r.kept.trim().is_empty(), "승계본이 공백(강등 분기 유발)");
    }

    /// [회귀 핀·★2차 BLOCK · **픽스처 회피 재발 방지**] 위 출하본 핀이 통과하는 이유가 "규칙이
    /// 좁아서"임을 증명하는 핀 — 옛 **평문 표기**(`본부(base) 레인 전용` · `승계 금지`)를 인용문·
    /// 중첩 인용·들여쓴 인용·규약 설명문으로 **가득 채운** 합성 base soul 을 넣어도 드롭 0 이어야
    /// 한다. v2 의 레거시 쌍 규칙은 이 입력에서 절을 통째로 삭제했다(리뷰어 실측 4종).
    /// ※ 규칙 삭제의 결과: 평문만 붙은 절은 이제 **승계된다**(차단 안 됨). 과삭제보다 안전한
    ///    실패 방향이라는 오너 결정이며, 정본 마커 한 줄을 넣는 것이 차단 방법이다.
    #[test]
    fn legacy_plaintext_wording_is_never_a_marker() {
        let base_soul = "# 헌장\n\
                         \n\
                         > 절 본문에 `cys:no-inherit`(권장) 또는 `승계 금지` / `본부(base) 레인 전용` 중 하나가\n\
                         > 있으면 제외된다고 설명하는 문장\n\
                         \n\
                         ## 정체\n\
                         오너는 주인님\n\
                         \n\
                         # 부록\n\
                         > 본부(base) 레인 전용 절은 승계 금지 대상이다\n\
                         >> 중첩 인용: 본부(base) 레인 전용 절은 승계 금지 대상\n\
                         \u{20} > 들여쓴 인용: 본부(base) 레인 전용 · 승계 금지\n\
                         \n\
                         ## 세부\n\
                         안전핵: 비가역 금지\n";
        let r = strip_no_inherit_sections(base_soul);
        assert!(
            r.dropped.is_empty(),
            "옛 평문 표기가 아직도 절을 삭제한다(2차 BLOCK 재발): {:?}",
            r.dropped
        );
        assert_eq!(r.kept, base_soul, "평문 문서는 바이트 불변으로 승계된다");
        assert!(
            r.kept.contains("안전핵: 비가역 금지"),
            "리뷰어 실측에서 소실됐던 줄이 또 사라졌다"
        );
        // 거동 변화(옛 표기 절이 이제 승계됨)는 **조용하지 않다** — 감사 흔적이 남는다.
        assert!(
            !r.unrecognized.is_empty(),
            "옛 평문 표기의 무력화가 조용히 지나갔다(마이그레이션 미고지)"
        );
        // 리뷰어가 마커로 인정된다고 실측한 4줄 — 이제 전부 마커가 아니어야 한다.
        for line in [
            "> 절 본문에 `cys:no-inherit`(권장) 또는 `승계 금지` / `본부(base) 레인 전용` 중 하나가",
            "> 본부(base) 레인 전용 절에는 승계 금지 마커를 붙여라",
            ">> 중첩 인용: 본부(base) 레인 전용 절은 승계 금지 대상",
            "  > 들여쓴 인용: 본부(base) 레인 전용 · 승계 금지",
        ] {
            assert!(!is_no_inherit_marker(line), "레거시 평문 규칙이 살아 있다: {line}");
        }
    }

    /// [회귀 핀·★F2/F3 · 리뷰어 실측 입력 3종을 그대로] 파서가 "보수적"이라는 주장의 반례들.
    ///   ⓐ 미닫힌 코드펜스 → 이후 절까지 EOF 과삭제(F3-b)
    ///   ⓑ 펜스 종류 미구분(``` 안의 `~~~` 가 패리티를 뒤집음 · F3-c)
    ///   ⓒ 드롭 1건이 문서 전체 CRLF 를 LF 로 바꿈(F3-d) + 평문 한국어 오탐(F3-a)
    ///   ⓓ 문서 루트 드롭 거부 + 빈 결과 취소(F2 두 방어선)
    #[test]
    fn guard_strip_survives_adversarial_inputs() {
        // ⓐ-0 리뷰어 실측 입력 **그대로** — `## marked` 가 문서의 첫 최상위 절이므로 이제는
        //     문서 몸통 보호에 걸려 아무것도 드롭되지 않는다(과삭제 벡터 자체가 닫힌다).
        let raw = "## marked\ncys:no-inherit\n```\ncode\n\n## after1\nA\n\n## after2\nB\n";
        let r = strip_no_inherit_sections(raw);
        assert!(r.dropped.is_empty(), "문서 첫 최상위 절이 드롭됐다: {:?}", r.dropped);
        assert_eq!(r.kept, raw, "바이트 불변");
        assert_eq!(r.refused.len(), 1, "보호 사유 미기록");
        // ⓐ 같은 구조를 H1 아래 하위 절로 옮겨 **미닫힌 펜스**만 검증 — after1·after2 생존.
        let src = "# 문서\n\n## marked\ncys:no-inherit\n```\ncode\n\n## after1\nA\n\n## after2\nB\n";
        let r = strip_no_inherit_sections(src);
        assert_eq!(r.dropped, vec!["## marked".to_string()]);
        assert_eq!(
            r.kept, "# 문서\n\n## after1\nA\n\n## after2\nB\n",
            "미닫힌 펜스가 뒤 절까지 삼켰다(F3-b 재발): {:?}",
            r.kept
        );

        // ⓑ 펜스 종류 구분 — ``` 블록 안의 `~~~` 줄은 닫는 펜스가 아니다.
        let src = "# 문서\n\n## marked\ncys:no-inherit\n```\n~~~\n## 펜스 안 가짜\n```\n\n## after\nA\n";
        let r = strip_no_inherit_sections(src);
        assert_eq!(r.dropped, vec!["## marked".to_string()]);
        assert_eq!(
            r.kept, "# 문서\n\n## after\nA\n",
            "펜스 종류 미구분으로 경계가 깨졌다(F3-c): {:?}",
            r.kept
        );

        // ⓒ-1 리뷰어 실측 CRLF 입력 **그대로** — 평문 "승계 금지"는 v2 에서 마커가 아니다(F3-a).
        let crlf = "# T\r\n\r\n## A\r\n승계 금지\r\n## B\r\nkeep\r\n";
        let r = strip_no_inherit_sections(crlf);
        assert!(r.dropped.is_empty(), "평문 한국어를 마커로 오인했다(F3-a 재발): {:?}", r.dropped);
        assert_eq!(r.kept, crlf, "바이트 불변이어야 한다");
        // ⓒ-2 실제로 드롭이 나는 CRLF 문서 — 살아남은 절의 CRLF 가 보존돼야 한다(F3-d).
        let crlf = "# T\r\n\r\n## A\r\n<!-- cys:no-inherit -->\r\n## B\r\nkeep\r\n";
        let r = strip_no_inherit_sections(crlf);
        assert_eq!(r.dropped, vec!["## A".to_string()]);
        assert_eq!(
            r.kept, "# T\r\n\r\n## B\r\nkeep\r\n",
            "드롭이 살아남은 줄의 CRLF 를 LF 로 바꿨다(F3-d 재발): {:?}",
            r.kept
        );

        // ⓓ-1 문서 루트(H1 하나 + 그 아래 ## 들) 드롭 거부 — 문서가 통째로 사라지지 않는다.
        let src = "# 헌장\n<!-- cys:no-inherit -->\n서두\n\n## 정체\n오너\n\n## 금지선\n비가역\n";
        let r = strip_no_inherit_sections(src);
        assert!(r.dropped.is_empty(), "문서 루트를 드롭했다(F2 재발): {:?}", r.dropped);
        assert_eq!(r.kept, src, "루트 드롭 거부 시 원문 유지");
        assert_eq!(r.refused.len(), 1, "거부는 조용히 삼키지 않는다: {:?}", r.refused);
        // ⓓ-1′ **루트 거부 우회**(리뷰어 실측): 뒤에 H1 이 하나만 더 있으면 몸통이 통째로 날아갔다.
        //      → '문서의 첫 최상위 절'은 뒤에 무엇이 오든 불가침이어야 한다.
        let src = "# 헌장\n<!-- cys:no-inherit -->\n…\n## 정체\n…\n# 부록\nx\n";
        let r = strip_no_inherit_sections(src);
        assert!(r.dropped.is_empty(), "루트 거부가 후속 H1 로 우회됐다(F2 재발): {:?}", r.dropped);
        assert_eq!(r.kept, src, "문서 몸통이 소실됐다");
        assert_eq!(r.refused.len(), 1, "거부 사유 미기록");
        // ⓓ-2 빈 결과 취소 — 마커 절이 문서의 전부일 때 벤더 스켈레톤 강등 대신 원문 유지.
        let src = "## 전부\n<!-- cys:no-inherit -->\n본문\n";
        let r = strip_no_inherit_sections(src);
        assert!(r.dropped.is_empty(), "빈 결과 드롭이 통과했다: {:?}", r.dropped);
        assert_eq!(r.kept, src, "빈 결과면 전량 취소하고 원문 유지");
        assert!(!r.refused.is_empty(), "취소 사유가 기록되지 않았다");
    }

    /// [회귀 핀·★2차 BLOCK 부수3] 반대 방향 — **마커를 붙였는데 무동작**. 표기 흔들림(공백·대소
    /// 문자)은 인정하고, 구조적 미인식(BOM 선행·CR-only 개행·setext heading)은 **인정 범위를
    /// 넓히는 대신 감사 흔적(`unrecognized`)으로 남긴다**(조용한 미탐이 가장 나쁘다).
    #[test]
    fn guard_marker_variants_are_honored_or_audited() {
        // ⓐ 공백 유무·대소문자는 인정한다(기계 토큰).
        for m in ["<!--cys:no-inherit-->", "<!-- CYS:NO-INHERIT -->", "<!--   cys:no-inherit   -->", "CYS:NO-INHERIT"] {
            assert!(is_no_inherit_marker(m), "표기 흔들림을 못 받았다: {m}");
            let src = format!("# T\n\n## 유지\nK\n\n## 대상\n{m}\nX\n");
            let r = strip_no_inherit_sections(&src);
            assert_eq!(r.dropped, vec!["## 대상".to_string()], "드롭 안 됨: {m}");
            assert_eq!(r.kept, "# T\n\n## 유지\nK\n\n", "잔여 본문: {}", r.kept);
            assert!(r.unrecognized.is_empty(), "정상 마커가 인식 실패로 잡혔다: {m}");
        }

        // ⓑ BOM 선행 — 첫 줄이 heading 으로 인식되지 않아 마커가 절 밖(preamble)에 놓인다.
        //    드롭 0 이되 **감사 흔적**이 남아야 한다.
        let src = "\u{feff}# T\n<!-- cys:no-inherit -->\n본문\n";
        let r = strip_no_inherit_sections(src);
        assert!(r.dropped.is_empty(), "BOM 문서에서 드롭이 났다: {:?}", r.dropped);
        assert_eq!(r.kept, src, "바이트 불변");
        assert_eq!(r.unrecognized.len(), 1, "BOM 미인식이 조용히 지나갔다: {:?}", r.unrecognized);

        // ⓒ CR-only 개행 — 문서 전체가 한 줄이라 줄 앵커가 성립하지 않는다.
        let src = "# T\r<!-- cys:no-inherit -->\r본문\r";
        let r = strip_no_inherit_sections(src);
        assert!(r.dropped.is_empty(), "CR-only 문서에서 드롭이 났다: {:?}", r.dropped);
        assert_eq!(r.kept, src, "바이트 불변");
        assert_eq!(r.unrecognized.len(), 1, "CR-only 미인식이 조용히 지나갔다: {:?}", r.unrecognized);

        // ⓓ setext heading 문서 — ATX 가 없어 절 경계가 없다(마커는 preamble 취급).
        let src = "제목\n====\n<!-- cys:no-inherit -->\n본문\n";
        let r = strip_no_inherit_sections(src);
        assert!(r.dropped.is_empty(), "setext 문서에서 드롭이 났다: {:?}", r.dropped);
        assert_eq!(r.kept, src, "바이트 불변");
        assert_eq!(r.unrecognized.len(), 1, "setext 미인식이 조용히 지나갔다: {:?}", r.unrecognized);

        // ⓔ 표기 오류(토큰 주위에 다른 글자) — 마커가 아니고, 흔적은 남는다.
        let src = "# T\n\n## 대상\n<!-- cys : no-inherit -->\nX\n";
        let r = strip_no_inherit_sections(src);
        assert!(r.dropped.is_empty(), "깨진 표기를 마커로 받았다: {:?}", r.dropped);
        assert_eq!(r.unrecognized.len(), 1, "표기 오류가 조용히 지나갔다: {:?}", r.unrecognized);
    }

    /// [회귀 핀·F3-a] 마커 인정 범위 — 줄 앵커 기계 토큰만. 자연문 어휘는 마커가 아니다.
    #[test]
    fn no_inherit_marker_is_line_anchored_only() {
        for yes in [
            "<!-- cys:no-inherit -->",
            "   <!-- cys:no-inherit -->  ",
            "<!--cys:no-inherit-->",
            "<!-- CYS:NO-INHERIT -->",
            "cys:no-inherit",
        ] {
            assert!(is_no_inherit_marker(yes), "정본 마커를 놓쳤다: {yes}");
        }
        for no in [
            // 출하 soul.md 의 안내 문장류 — v1 이 여기 걸려 문서 전량을 지웠다.
            "> 부서장이 그것을 물려받아 자기 정체를 오인한다. 그런 절에는 **승계 금지 마커 주석**을",
            "- 승계 금지 절은 이렇게 표시한다",
            "본부(base) 레인 전용 절이라는 개념을 설명하는 평문",
            "> 본부(base) 레인 전용 이라고만 적힌 인용문",
            "> 승계 금지 라고만 적힌 인용문",
            // v2 레거시 쌍 규칙이 마커로 인정하던 4종(리뷰어 실측) — v3 에서 전부 마커 아님.
            "> ★**본부(base) 레인 전용 절 — 승계 금지 가드**: v0.14.22부터 …",
            "> 절 본문에 `cys:no-inherit`(권장) 또는 `승계 금지` / `본부(base) 레인 전용` 중 하나가",
            ">> 중첩 인용: 본부(base) 레인 전용 절은 승계 금지 대상",
            "  > 들여쓴 인용: 본부(base) 레인 전용 · 승계 금지",
            "코드 안 문자열 \"cys:no-inherit\" 언급",           // 줄 전체가 아님
            "## cys:no-inherit",                                 // heading 은 마커가 아니다
            "<!-- cys:no-inherit --> 뒤에 설명이 붙은 줄",       // 줄 전체가 아님
            "<!---->",                                           // 빈 주석
        ] {
            assert!(!is_no_inherit_marker(no), "자연문을 마커로 오인했다(F3-a 재발): {no}");
        }
    }

    /// [회귀 핀·결함3(a)] 부서명 판정 실패에서도 정체 스탬프는 붙는다 — '정체 없이 뜨는 부서장 0'
    /// 이 목적이므로 강등은 **부서명 자리**에서만 일어나고 정체 선언 자체는 사라지지 않는다.
    #[test]
    fn dept_identity_stamp_degrades_name_but_never_identity() {
        let s = dept_identity_stamp(None);
        assert!(s.contains("(부서명 미상) 부서장 마스터"), "부서명 미상 강등 문안 부재: {s}");
        assert!(s.contains("각성 보고 첫 문장에 부서장 정체를 명시하라"), "정체 지시가 사라졌다: {s}");
        let s = dept_identity_stamp(Some("영업"));
        assert!(s.starts_with("# 부서 정체 — 영업 부서장 마스터"), "스탬프 제목 문안: {s}");
        assert!(!s.contains("(부서명 미상)"), "정상 판정에 강등 문구가 섞였다");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ★W0 부정 케이스 3종(설계 합격 판정) — verifier를 부정 케이스로 검증한다.
    // ─────────────────────────────────────────────────────────────────────────

    /// 부정①(W0-a): CYS_PACK_DIR(및 레거시) 미설정 상태에서 pack_dir() 호출은 **panic**한다.
    /// catch_unwind로 패닉을 잡아 PACK_ENV_LOCK 오염(다른 테스트의 .lock().unwrap() 연쇄 실패)을 막고,
    /// EnvGuard로 각 키를 복원한다(패닉 언와인딩에도 drop 보장).
    #[test]
    fn w0a_pack_dir_panics_when_all_env_unset_in_test_build() {
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        let _g_cys = EnvGuard::remove("CYS_PACK_DIR");
        let _g_javis = EnvGuard::remove("JAVIS_PACK_DIR");
        let _g_aiterm_pack = EnvGuard::remove("AITERM_PACK_DIR");
        let _g_aiterm_jarvis = EnvGuard::remove("AITERM_JARVIS_DIR");
        // 패닉 메시지 소음을 잠시 죽인다(테스트 stderr 노이즈 방지).
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(pack_dir);
        std::panic::set_hook(prev_hook);
        assert!(
            result.is_err(),
            "W0-a: 전 pack env 미설정 시 pack_dir()은 panic해야 한다(라이브 재오염 봉인)"
        );
    }

    /// 부정②(W0-d): 인가 토큰 없이 **라이브 기본 경로** 대상 쓰기는 하드 거부(Err)된다.
    /// HOME을 가짜 temp로 override해 home_default_pack_dir()가 실 라이브를 가리키지 않게 한다
    /// (테스트가 실제 ~/.cys/pack을 만질 위험 0). CYS_PACK_DIR 샌드박스는 유지(W0-a panic 회피 —
    /// 게이트는 target env가 아니라 dir 인자로 판정한다). 대조로 명시 인가 시 동일 쓰기가 성공한다.
    #[test]
    fn w0d_rejects_unauthorized_write_to_live_default_path() {
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-w0d-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let _gh = EnvGuard::set("HOME", &base);
        let live = home_default_pack_dir(); // = <fake_home>/.cys/pack
        assert!(
            targets_live_default(&live),
            "설정 오류: fake HOME 기준 live 경로가 라이브 기본으로 판정돼야 한다"
        );
        let items = vec![("probe.txt", "hello")];

        // 인가(None) 없이 라이브 기본 경로 쓰기 → Err, 아무것도 쓰이지 않음.
        let res = install_into(
            live.clone(),
            items.iter().copied(),
            false,
            "2.0.0",
            false,
            false,
            pack_scope_of(&live),
            None,
            None,
        );
        assert!(res.is_err(), "인가 없는 라이브 쓰기는 Err여야 한다: {res:?}");
        assert!(!live.join("probe.txt").exists(), "거부 후 파일이 쓰이면 안 된다");

        // 대조: 명시 인가(for_test) 부여 시 동일 대상 쓰기는 성공(게이트가 인가를 존중).
        let ok = install_into(
            live.clone(),
            items.iter().copied(),
            false,
            "2.0.0",
            false,
            false,
            pack_scope_of(&live),
            Some(PackWriteAuth::for_test()),
            None,
        );
        assert!(ok.is_ok(), "인가 부여 시 라이브 쓰기는 성공해야 한다: {ok:?}");
        assert!(live.join("probe.txt").exists(), "인가 쓰기 후 파일이 존재해야 한다");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 부정③(W0-d): **심링크**를 경유해 라이브 기본 경로를 가리키는 쓰기도 canonicalize 비교로 거부된다.
    /// fake HOME 아래 실제 라이브 경로를 만들고, 별도 위치의 심링크가 그 부모를 가리키게 한 뒤
    /// 심링크 경유 경로로 인가 없이 쓰기를 시도한다 → Err.
    #[cfg(unix)]
    #[test]
    fn w0d_rejects_symlinked_path_to_live_default() {
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-w0d-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let _gh = EnvGuard::set("HOME", &base);
        // 라이브 기본 경로의 부모(<fake_home>/.cys)를 실재화.
        let live = home_default_pack_dir();
        let live_parent = live.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&live_parent).unwrap();
        // 별도 위치의 심링크가 <fake_home>/.cys 를 가리킨다 → link/pack == 라이브 기본 경로.
        let link = base.join("cys-link");
        std::os::unix::fs::symlink(&live_parent, &link).unwrap();
        let via_symlink = link.join("pack"); // canonicalize 시 <fake_home>/.cys/pack 로 해소
        assert!(
            targets_live_default(&via_symlink),
            "심링크 경유 경로가 canonicalize 후 라이브 기본으로 판정돼야 한다"
        );

        let items = vec![("probe.txt", "hello")];
        let res = install_into(
            via_symlink.clone(),
            items.iter().copied(),
            false,
            "2.0.0",
            false,
            false,
            pack_scope_of(&via_symlink),
            None,
            None,
        );
        assert!(res.is_err(), "심링크 경유 라이브 쓰기는 거부돼야 한다: {res:?}");
        assert!(!live.join("probe.txt").exists(), "거부 후 라이브에 파일이 쓰이면 안 된다");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 빈 임시 dir에서 디스크 산출물을 핑거프린트(rel → sha256)로 채집한다 —
    /// install vs install_from_iter 등가성 비교용. 매니페스트·pack-version도 포함.
    fn fingerprint_dir(root: &Path) -> std::collections::BTreeMap<String, String> {
        fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, String>) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(base, &p, out);
                    } else if let Ok(bytes) = std::fs::read(&p) {
                        use sha2::{Digest, Sha256};
                        let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                        out.insert(rel, format!("{:x}", Sha256::digest(&bytes)));
                    }
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(root, root, &mut out);
        out
    }

    /// ★등가성 박제(§7-⑤): install(false)의 디스크 결과 == install_from_iter(PACK+SKILLS, false,
    /// CARGO_PKG_VERSION). 얇은 래퍼가 외부 동작을 완전 보존하는지(written/kept·전 파일 핑거프린트).
    #[test]
    fn install_from_iter_equivalent_to_install() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base =
            std::env::temp_dir().join(format!("cys-pack-equiv-test-{}", std::process::id()));
        let td_a = base.join("a"); // install(false)
        let td_b = base.join("b"); // install_from_iter
        let _ = std::fs::remove_dir_all(&base);

        // 격리 config dir은 pack dir **밖**에 둔다 — settings.json이 pack_dir 절대경로
        // (hooks/session-start.sh)를 박으므로 td 안에 두면 td_a≠td_b 경로 차이가 핑거프린트를
        // 오염시킨다. pack dir 콘텐츠 자체는 경로 무관 결정론이라 이 분리로 순수 등가 비교가 된다.
        // A: 기존 래퍼
        let _env_a = set_pack_env(&td_a, base.join("cfg-a"));
        let res_a = install(false, None);
        let fp_a = fingerprint_dir(&td_a);

        // B: 추출 코어 직접 호출(동일 입력원·동일 버전)
        let _env_b = set_pack_env(&td_b, base.join("cfg-b"));
        let res_b = install_from_iter(
            PACK_ALL.iter().map(|(r, c)| (*r, *c)),
            false,
            env!("CARGO_PKG_VERSION"),
            false,
            None,
        );
        let fp_b = fingerprint_dir(&td_b);

        let _ = std::fs::remove_dir_all(&base);

        let (wa, ka) = res_a.expect("install 실패");
        let (wb, kb) = res_b.expect("install_from_iter 실패");
        assert_eq!((wa, ka), (wb, kb), "written/kept 불일치");
        assert_eq!(wa, PACK_ALL.len(), "전수 설치 아님");
        // 핵심 파일 존재 + 전 파일 핑거프린트 동등
        for probe in [
            "skills/korean-humanizer/SKILL.md",
            "bin/javis_route.py",
            "directives/MASTER_DIRECTIVE.md",
            PACK_VERSION_FILE,
            INSTALL_MANIFEST,
        ] {
            assert!(fp_a.contains_key(probe), "A 산출물에 {probe} 부재");
        }
        assert_eq!(fp_a, fp_b, "install vs install_from_iter 디스크 산출물 불일치");
    }

    /// parse_semver: 자릿수·v접두·-rc/+build suffix 분리·실패=None.
    #[test]
    fn parse_semver_cases() {
        assert_eq!(parse_semver("0.4.1"), Some((0, 4, 1)));
        assert_eq!(parse_semver("0.4.10"), Some((0, 4, 10)), "patch 자릿수");
        assert_eq!(parse_semver("v0.5.0"), Some((0, 5, 0)), "'v' 접두");
        assert_eq!(parse_semver("0.4.10-rc"), Some((0, 4, 10)), "-rc suffix 분리");
        assert_eq!(parse_semver("0.4.0+build"), Some((0, 4, 0)), "+build suffix 분리");
        assert_eq!(parse_semver("1"), Some((1, 0, 0)), "minor/patch 결측=0");
        assert_eq!(parse_semver("garbage"), None, "비숫자 major=실패");
        assert_eq!(parse_semver(""), None, "빈 문자열=실패");
    }

    /// remote_is_newer: fail-CLOSED 반영거부 — malformed=false·정상 newer=true·동일=false.
    #[test]
    fn remote_is_newer_fail_closed() {
        assert!(remote_is_newer("0.4.2", "0.4.1"), "정상 newer=true");
        assert!(remote_is_newer("0.5.0", "0.4.9"), "minor newer=true");
        assert!(!remote_is_newer("0.4.1", "0.4.1"), "동일=false");
        assert!(!remote_is_newer("0.4.0", "0.4.1"), "낮음=false");
        // ★fail-CLOSED: 한쪽이라도 파싱 실패 → false(반영 거부) — version_gt(보존=true)와 반대
        assert!(!remote_is_newer("garbage", "0.4.1"), "malformed remote=false");
        assert!(!remote_is_newer("0.5.0", "garbage"), "malformed disk=false");
        assert!(!remote_is_newer("", "0.4.1"), "빈 remote=false");
    }

    /// write_atomic: 쓰고 읽어 일치 + 기존 파일 원자 교체.
    #[test]
    fn write_atomic_roundtrip_and_replace() {
        let td =
            std::env::temp_dir().join(format!("cys-write-atomic-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let p = td.join("sub").join("file.txt");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();

        write_atomic(&p, b"first").expect("write 실패");
        assert_eq!(std::fs::read(&p).unwrap(), b"first", "roundtrip 불일치");

        // 기존 파일 교체
        write_atomic(&p, b"second-longer-content").expect("replace 실패");
        assert_eq!(
            std::fs::read(&p).unwrap(),
            b"second-longer-content",
            "교체 후 내용 불일치"
        );
        // temp 잔존 없음(rename으로 소비)
        let leftovers: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temp 파일 잔존: {leftovers:?}");

        let _ = std::fs::remove_dir_all(&td);
    }

    // ── pack-update 적용 트랜잭션(§7-⑤ 옵션 b — R2CODE HIGH #1/MED #2) ────────────────
    // 모든 트랜잭션 테스트는 PACK_ENV_LOCK으로 직렬화한다(ENV_PACK_DIR 프로세스 전역 + 저널은
    // pack_dir 형제라 격리 base/pack 구조로 저널을 base 안에 가둔다).

    /// pre-state(.pack-version·README.md·.install-manifest)를 base/pack에 깔고 env를 세팅한다.
    /// 반환: (base, pd). 정리는 호출처가 remove_dir_all(base).
    /// 트랜잭션 테스트 공용 free 상태(v6 §3 — 시그니처 확장에 따른 헬퍼).
    fn test_free_state(base: &str) -> PackState {
        PackState {
            channel: "free".to_string(),
            base_version: base.to_string(),
            pro_revision: 0,
        }
    }

    /// ★W0-b: 테스트 env 격리 헬퍼 — ENV_PACK_DIR·ENV_CONFIG_DIR을 RAII 가드로 설정한다(이전 값
    /// 복원형). 반환 가드가 drop될 때(정상·**패닉 언와인딩 포함**) 복원되므로 수동 teardown이 불필요하고
    /// "env가 비는 창"이 생기지 않는다. PACK_ENV_LOCK 하에서 호출하는 것을 전제로 한다.
    #[must_use]
    fn set_pack_env(pack: impl AsRef<std::ffi::OsStr>, cfg: impl AsRef<std::ffi::OsStr>) -> (EnvGuard, EnvGuard) {
        (EnvGuard::set(ENV_PACK_DIR, pack), EnvGuard::set(ENV_CONFIG_DIR, cfg))
    }

    fn txn_prestate(
        tag: &str,
        files: &[(&str, &str)],
        version: &str,
    ) -> (PathBuf, PathBuf, (EnvGuard, EnvGuard)) {
        let base = std::env::temp_dir().join(format!("cys-journal-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        // ★W0-b: env 격리 가드 — 호출 테스트가 이 가드를 보유해 스코프 종료(패닉 포함) 시 복원.
        let env_guards = set_pack_env(&pd, base.join("cfg"));
        std::fs::write(pd.join(PACK_VERSION_FILE), version).unwrap();
        let mut manifest = serde_json::Map::new();
        for (rel, content) in files {
            let p = pd.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
            manifest.insert((*rel).to_string(), serde_json::json!(content_hash(content)));
        }
        std::fs::write(
            pd.join(INSTALL_MANIFEST),
            serde_json::Value::Object(manifest).to_string(),
        )
        .unwrap();
        (base, pd, env_guards)
    }

    /// 정상 경로: 파일 반영·prune·record_accepted(closure)·.pack-version commit marker 기록 후
    /// 저널이 삭제된다.
    #[test]
    fn apply_transactional_commit_then_journal_removed() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate(
            "commit",
            &[("README.md", "OLD-SOUL"), ("stale.txt", "STALE")],
            "1.0.0",
        );

        // README.md 갱신 + new.txt 추가, stale.txt는 items 부재 → prune.
        let items: Vec<(&str, &str)> = vec![("README.md", "NEW-SOUL"), ("new.txt", "NEW")];
        let committed = std::cell::Cell::new(false);
        let res = apply_pack_transactional(&items, "2.0.0", &test_free_state("2.0.0"), None, || {
            committed.set(true);
            Ok(())
        });

        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let newf = std::fs::read_to_string(pd.join("new.txt")).unwrap();
        let stale_exists = pd.join("stale.txt").exists();
        let journal_exists = pack_journal_dir().exists();
        let _ = std::fs::remove_dir_all(&base);

        let (w, _k, post_ok) = res.expect("commit 실패");
        assert!(committed.get(), "post_commit(record_accepted) 미호출");
        assert!(post_ok, "post_commit 성공인데 false 보고");
        assert_eq!(pv.trim(), "2.0.0", ".pack-version commit marker 미기록");
        assert_eq!(soul, "NEW-SOUL", "README.md 갱신 안됨");
        assert_eq!(newf, "NEW", "new.txt 추가 안됨");
        assert!(!stale_exists, "stale.txt prune 안됨");
        assert!(!journal_exists, "commit 성공 후 저널 미삭제");
        assert!(w >= 2, "written={w}");
    }

    /// ★핵심(codex missing): apply 도중 N번째 쓰기에서 실패를 주입(디렉터리 충돌: 파일 'collide'
    /// 직후 'collide/child' 쓰기가 create_dir_all 실패)하면 트리가 pre-state와 동일(전부 rollback)
    /// 이고 .pack-version 불변임을 증명한다(부분적용 0).
    #[test]
    fn mid_apply_fault_rolls_back_to_prestate() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate("fault", &[("README.md", "OLD-SOUL")], "1.0.0");
        let pre_fp = fingerprint_dir(&pd);

        // README.md 갱신(1번째 성공) → collide 파일(2번째 성공) → collide/child(3번째: 부모가
        // 파일이라 create_dir_all 실패) = mid-apply fault.
        let items: Vec<(&str, &str)> =
            vec![("README.md", "NEW"), ("collide", "X"), ("collide/child", "Y")];
        let res = apply_pack_transactional(&items, "2.0.0", &test_free_state("2.0.0"), None, || Ok(()));

        let post_fp = fingerprint_dir(&pd);
        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let journal_exists = pack_journal_dir().exists();
        let _ = std::fs::remove_dir_all(&base);

        assert!(res.is_err(), "mid-apply fault인데 성공 반환");
        assert_eq!(pv.trim(), "1.0.0", ".pack-version 불변이어야(미커밋)");
        assert!(!journal_exists, "rollback 후 저널 잔존");
        assert_eq!(pre_fp, post_fp, "rollback이 pre-state로 복원 못함(부분적용 잔존)");
    }

    /// ★G3-축3 감사 원장 수명주기 핀: append-only(누적·트렁케이트 0)·라인당 유효 JSON 1건 +
    /// 성공 트랜잭션(prune 스윕)·실패 트랜잭션(rollback) 양쪽 생존. backup_set 에 MERGE_AUDIT_FILE
    /// 을 등재하면 rollback 이 거부·강제 라인을 되감아 소거한다 — 이 핀이 그 회귀를 막는다.
    #[test]
    fn merge_audit_appends_and_survives_transactions() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate("audit", &[("README.md", "OLD")], "1.0.0");
        let e1 = serde_json::json!({"ts": 1, "file": "soul.md", "action": "take-new"});
        append_merge_audit(&pd, &e1).unwrap();
        // ① 성공 트랜잭션 후에도 원장 잔존 — 매니페스트 비등재 dotfile 은 prune 불가침.
        let items: Vec<(&str, &str)> = vec![("README.md", "NEW")];
        apply_pack_transactional(&items, "2.0.0", &test_free_state("2.0.0"), None, || Ok(()))
            .expect("정상 트랜잭션 실패");
        let after_ok = std::fs::read_to_string(pd.join(MERGE_AUDIT_FILE)).unwrap();
        assert_eq!(after_ok.lines().count(), 1, "성공 트랜잭션이 감사 원장을 건드렸다");
        // ② append 누적(truncate 0).
        let e2 = serde_json::json!({"ts": 2, "file": "soul.md", "action": "take-new", "flags": ["refused"]});
        append_merge_audit(&pd, &e2).unwrap();
        // ③ mid-apply fault 로 rollback 을 타도 원장은 생존(backup_set 비등재 계약).
        let bad: Vec<(&str, &str)> = vec![("collide", "X"), ("collide/child", "Y")];
        assert!(
            apply_pack_transactional(&bad, "3.0.0", &test_free_state("3.0.0"), None, || Ok(()))
                .is_err(),
            "fault 주입이 성공으로 둔갑"
        );
        let after_rb = std::fs::read_to_string(pd.join(MERGE_AUDIT_FILE)).unwrap();
        let lines: Vec<&str> = after_rb.lines().collect();
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(lines.len(), 2, "rollback 이 감사 라인을 되감았다(backup_set 등재 금지)");
        for l in &lines {
            let v: serde_json::Value = serde_json::from_str(l).expect("라인당 유효 JSON 1건");
            assert!(v.get("action").is_some(), "action 필드 부재: {l}");
        }
    }

    /// v4 §3 재배치 핀(R3 codex blocking 결착): record_accepted는 post-commit — 실패해도
    /// 커밋(파일·state·.pack-version)은 유효하게 남고 rollback하지 않으며, 성공으로 침묵
    /// 포장하지 않고 post_ok=false로 구분 보고한다. (구 동작: pre-commit이라 실패 시 전체
    /// rollback → 낡은 accepted가 정품 번들 재시도를 replay 거부하는 crash 교착의 원천이었다.)
    #[test]
    fn post_commit_failure_keeps_commit_and_reports() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate(
            "recordfail",
            &[("README.md", "OLD-SOUL"), ("stale.txt", "STALE")],
            "1.0.0",
        );

        // 파일 반영·prune·커밋은 성공, post-commit record_accepted만 실패.
        let items: Vec<(&str, &str)> = vec![("README.md", "NEW-SOUL"), ("new.txt", "NEW")];
        let res = apply_pack_transactional(&items, "2.0.0", &test_free_state("2.0.0"), None, || {
            Err("record_accepted boom".into())
        });

        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let stale_exists = pd.join("stale.txt").exists();
        let journal_exists = pack_journal_dir().exists();
        let _ = std::fs::remove_dir_all(&base);

        let (_w, _k, post_ok) = res.expect("post-commit 실패가 Err로 승격되면 안됨(커밋은 유효)");
        assert!(!post_ok, "post_commit 실패인데 true 보고(침묵 포장)");
        assert_eq!(pv.trim(), "2.0.0", "커밋 마커는 유효해야 함(rollback 금지)");
        assert_eq!(soul, "NEW-SOUL", "파일 반영은 유지돼야 함");
        assert!(!stale_exists, "prune 결과도 유지돼야 함");
        assert!(!journal_exists, "커밋 성공 경로 — 저널 정리돼야 함");
    }

    /// orphan 저널 recovery: 디스크 .pack-version != 저널 target(미커밋)이면 rollback으로
    /// pre-state 자가치유. crash로 남은 부분적용(README.md=PARTIAL·new.txt 생성)을 되돌린다.
    #[test]
    fn orphan_journal_recovery_rolls_back() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        // crash 후 디스크: .pack-version 옛 1.0.0(미커밋) + README.md 부분반영 + new.txt 신규생성.
        let (base, pd, _env) = txn_prestate("orphan-rb", &[("README.md", "PARTIAL-NEW")], "1.0.0");
        std::fs::write(pd.join("new.txt"), "ORPHAN-NEW").unwrap();
        // 저널 수작업 조립: target 2.0.0, README.md(existed) backup=OLD-SOUL, new.txt(신규) existed=false.
        let jdir = pack_journal_dir();
        let files_dir = jdir.join("files");
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::write(files_dir.join("README.md"), "OLD-SOUL").unwrap();
        let index = serde_json::json!({
            "target_version": "2.0.0",
            "entries": [
                {"rel": "README.md", "existed": true},
                {"rel": "new.txt", "existed": false}
            ]
        });
        std::fs::write(jdir.join("index.json"), index.to_string()).unwrap();

        let recovered = recover_pack_journal();

        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let new_exists = pd.join("new.txt").exists();
        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let journal_exists = pack_journal_dir().exists();
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(recovered.expect("recover 실패"), true, "orphan 미발견");
        assert_eq!(soul, "OLD-SOUL", "README.md rollback 안됨");
        assert!(!new_exists, "신규생성 new.txt 삭제 안됨");
        assert_eq!(pv.trim(), "1.0.0", ".pack-version 변경됨(미커밋인데)");
        assert!(!journal_exists, "recovery 후 저널 잔존");
    }

    /// orphan 저널 recovery: 디스크 .pack-version == 저널 target(커밋 성공·정리 중 crash)이면
    /// rollback 없이 저널만 삭제(커밋된 새 내용을 되돌리지 않는다).
    #[test]
    fn orphan_journal_committed_only_cleaned() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        // 커밋 성공: .pack-version=2.0.0, README.md=NEW-SOUL(새 내용).
        let (base, pd, _env) = txn_prestate("orphan-commit", &[("README.md", "NEW-SOUL")], "2.0.0");
        let jdir = pack_journal_dir();
        let files_dir = jdir.join("files");
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::write(files_dir.join("README.md"), "OLD-SOUL").unwrap(); // 커밋 전 백업본
        let index = serde_json::json!({
            "target_version": "2.0.0",
            "entries": [{"rel": "README.md", "existed": true}]
        });
        std::fs::write(jdir.join("index.json"), index.to_string()).unwrap();

        let recovered = recover_pack_journal();

        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let journal_exists = pack_journal_dir().exists();
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(recovered.expect("recover 실패"), true, "orphan 미발견");
        assert_eq!(soul, "NEW-SOUL", "커밋된 내용을 잘못 rollback함");
        assert!(!journal_exists, "정리 후 저널 잔존");
    }

    /// ★핵심(R2CODE2 HIGH #1): pack-update 트랜잭션에서 .install-manifest.json write_atomic 실패
    /// (경로를 디렉터리로 만들어 rename 실패 유발)는 fail-closed로 Err가 되어 apply_pack_transactional이
    /// rollback을 타야 한다. 트리 pre-state 복원(README.md=OLD·new.txt 제거)·.pack-version 불변(미커밋)을
    /// assert해 부분커밋 0을 증명한다.
    #[test]
    fn manifest_write_failure_transactional_rolls_back() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate("manifest-fail", &[("README.md", "OLD-SOUL")], "1.0.0");
        // .install-manifest.json을 디렉터리로 치환 → write_atomic(rename) 실패 유발(IO fault 주입).
        let mp = pd.join(INSTALL_MANIFEST);
        std::fs::remove_file(&mp).unwrap();
        std::fs::create_dir_all(mp.join("child")).unwrap();

        let items: Vec<(&str, &str)> = vec![("README.md", "NEW-SOUL"), ("new.txt", "NEW")];
        let committed = std::cell::Cell::new(false);
        let res = apply_pack_transactional(&items, "2.0.0", &test_free_state("2.0.0"), None, || {
            committed.set(true);
            Ok(())
        });

        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let new_exists = pd.join("new.txt").exists();
        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let journal_exists = pack_journal_dir().exists();
        let _ = std::fs::remove_dir_all(&base);

        assert!(res.is_err(), "매니페스트 write 실패인데 성공 반환(best-effort 흡수)");
        assert!(!committed.get(), "파일 반영 실패 전에 commit_extra가 호출되면 안됨");
        assert_eq!(soul, "OLD-SOUL", "rollback이 README.md를 pre-state로 복원 못함");
        assert!(!new_exists, "rollback이 신규 new.txt를 제거 못함(부분적용 잔존)");
        assert_eq!(pv.trim(), "1.0.0", ".pack-version 불변이어야(미커밋)");
        assert!(!journal_exists, "rollback 후 저널 잔존");
    }

    /// ★대조(외부 동작 불변): embed/cysd/init-pack 경로(transactional=false)는 .install-manifest.json이
    /// 디렉터리여도 매니페스트 영속을 best-effort로 무시하고 설치를 진행한다 — 파일 반영·.pack-version
    /// 기록이 종전대로 일어난다(fail-closed는 pack-update 트랜잭션 전용).
    #[test]
    fn manifest_write_failure_embed_best_effort_proceeds() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate("manifest-embed", &[("README.md", "OLD-SOUL")], "1.0.0");
        let mp = pd.join(INSTALL_MANIFEST);
        std::fs::remove_file(&mp).unwrap();
        std::fs::create_dir_all(mp.join("child")).unwrap();

        // new.txt는 신규(preserve-gate 충돌 없음)라 반영된다. README.md는 매니페스트 불가독으로
        // preserve-gate가 안전측 보존(OLD 유지) — 이는 manifest 손상의 정상 부작용이며 embed
        // best-effort 분기와 무관하다. 핵심: 매니페스트 write 실패에도 Err 없이 진행 + 버전 마커 기록.
        let items: Vec<(&str, &str)> = vec![("README.md", "NEW-SOUL"), ("new.txt", "NEW")];
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false, None);

        let new_exists = pd.join("new.txt").exists();
        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let _ = std::fs::remove_dir_all(&base);

        assert!(res.is_ok(), "embed 경로(best-effort)인데 매니페스트 실패로 Err 반환");
        assert!(new_exists, "embed 경로 신규 파일(new.txt) 반영 안됨");
        assert_eq!(pv.trim(), "2.0.0", "embed 경로 .pack-version 기록 안됨(외부 동작 변경)");
    }

    // ── free/pro 채널 상태 계약(v6 §3·§5) ──────────────────────────────────────

    #[test]
    fn pack_state_read_three_way() {
        let base = std::env::temp_dir().join(format!("cys-state3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        // 부재 = Absent (구 설치 자연 마이그레이션 = free/0)
        assert!(matches!(read_pack_state(&base), PackStateRead::Absent));
        // 정상 free
        write_pack_state(&base, &test_free_state("1.0.0")).unwrap();
        assert!(matches!(read_pack_state(&base), PackStateRead::Valid(st) if st.channel == "free"));
        // 손상(파싱 불가) = Corrupt(보존 방향)
        std::fs::write(base.join(PACK_STATE_FILE), b"{garbage").unwrap();
        assert!(matches!(read_pack_state(&base), PackStateRead::Corrupt(_)));
        // 미지 channel 값 = Corrupt(fail-closed)
        std::fs::write(
            base.join(PACK_STATE_FILE),
            br#"{"channel":"enterprise","base_version":"1.0.0","pro_revision":0}"#,
        )
        .unwrap();
        assert!(matches!(read_pack_state(&base), PackStateRead::Corrupt(_)));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// v6 §3 튜플 비교기 — 전이 케이스(설계 의무: free→pro/pro.N+1/역행/rebase/fail-closed).
    #[test]
    fn remote_is_newer_tuple_transitions() {
        assert!(remote_is_newer_tuple(("0.8.0", 1), ("0.8.0", 0)), "free→pro 전환");
        assert!(remote_is_newer_tuple(("0.8.0", 2), ("0.8.0", 1)), "pro.N→pro.N+1 증분");
        assert!(!remote_is_newer_tuple(("0.8.0", 1), ("0.8.0", 2)), "pro 역행 거부");
        assert!(remote_is_newer_tuple(("0.9.0", 1), ("0.8.0", 5)), "base rebase(base 우선)");
        assert!(!remote_is_newer_tuple(("0.8.0", 1), ("0.8.0", 1)), "동일 튜플 = 반영 아님");
        assert!(!remote_is_newer_tuple(("garbage", 9), ("0.8.0", 0)), "파싱 실패 fail-closed");
        // 기존 free 경로 무회귀(rev 0 동치).
        assert!(remote_is_newer_tuple(("0.4.2", 0), ("0.4.1", 0)));
        assert!(!remote_is_newer_tuple(("0.4.1", 0), ("0.4.1", 0)));
    }

    /// ★회귀 핀(v6 §5 의무): 앱 업데이트(내장 install 신버전)가 marker=pro 설치에서
    /// **쓰기 0 + prune 0** — pro 전용 파일 전수 생존.
    #[test]
    fn embed_guard_pro_state_preserves_pro_files() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate(
            "proguard",
            &[("README.md", "OLD-SOUL"), ("pro-only/skill.md", "PRO-SKILL")],
            "1.0.0",
        );
        write_pack_state(
            &pd,
            &PackState { channel: "pro".into(), base_version: "1.0.0".into(), pro_revision: 1 },
        )
        .unwrap();

        // 내장 install 시뮬레이션: 신버전 2.0.0, items에 pro-only 파일 부재(=구현 전이라면 prune 대상).
        let items: Vec<(&str, &str)> = vec![("README.md", "NEW-SOUL")];
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false, None);

        let pro_file = std::fs::read_to_string(pd.join("pro-only/skill.md")).unwrap_or_default();
        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let st_after = read_pack_state(&pd);
        let _ = std::fs::remove_dir_all(&base);

        let (w, k) = res.expect("가드 경로는 Ok((0,0))이어야 함");
        assert_eq!((w, k), (0, 0), "marker=pro인데 내장 install이 뭔가를 썼다");
        assert_eq!(pro_file, "PRO-SKILL", "★pro 전용 파일이 prune됨(R1 재앙 재현)");
        assert_eq!(soul, "OLD-SOUL", "pro 팩 파일이 내장본으로 덮임");
        assert_eq!(pv.trim(), "1.0.0", ".pack-version이 변경됨(가드 위반)");
        assert!(
            matches!(st_after, PackStateRead::Valid(st) if st.channel == "pro" && st.pro_revision == 1),
            "state가 변경됨(가드 위반)"
        );
    }

    /// 손상 state = 보존 모드(pro 간주) — 내장 install 전체 생략(v6 §5 fail-closed 방향).
    #[test]
    fn embed_guard_corrupt_state_preserves() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate("corruptguard", &[("README.md", "OLD")], "1.0.0");
        std::fs::write(pd.join(PACK_STATE_FILE), b"{not json").unwrap();

        let items: Vec<(&str, &str)> = vec![("README.md", "NEW")];
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false, None);

        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(res.expect("가드 경로 Ok"), (0, 0));
        assert_eq!(soul, "OLD", "손상 state인데 파일이 변경됨(보존 위반)");
    }

    /// channel=free 정합 불일치 + 음성 pro 증거 없음 → 제한적 자가치유 후 install 진행(v6 §5).
    #[test]
    fn embed_free_mismatch_heals_without_evidence() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        // install-manifest 키는 임베드 트리에 실재하는 rel만(README.md) — pro 파일 증거 없음.
        let (base, pd, _env) = txn_prestate("healok", &[("README.md", "OLD")], "1.0.0");
        write_pack_state(&pd, &test_free_state("0.9.0")).unwrap(); // base 불일치(0.9.0 ≠ 1.0.0)

        let items: Vec<(&str, &str)> = vec![("README.md", "NEW")];
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false, None);

        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let st_after = read_pack_state(&pd);
        let _ = std::fs::remove_dir_all(&base);

        let (w, _k) = res.expect("자가치유 후 install 진행돼야 함");
        assert!(w >= 1, "install이 진행되지 않음(자가치유 미발동?)");
        assert_eq!(soul, "NEW", "비수정 파일 자동 갱신 안됨");
        // checked 쓰기 순서: .pack-version(2.0.0) 성공 후 state 동기까지 수렴.
        assert!(
            matches!(st_after, PackStateRead::Valid(st) if st.channel == "free" && st.base_version == "2.0.0"),
            "state 동기 갱신 실패"
        );
    }

    /// v6 음성 증거 ①: state=free이나 accepted 기록=pro(거짓 free) → 자가치유 금지·보존.
    /// (R5 codex major 회귀 핀: pro 설치에서 state만 valid free로 오염 → prune 미수행)
    #[test]
    fn embed_free_mismatch_with_accepted_pro_preserved() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate(
            "falsefree",
            &[("README.md", "OLD"), ("pro-only/skill.md", "PRO")],
            "1.0.0",
        );
        write_pack_state(&pd, &test_free_state("0.9.0")).unwrap(); // 거짓 free + 불일치
        // parent(.pack-accepted.json)에 pro 수용 이력.
        std::fs::write(
            base.join(".pack-accepted.json"),
            br#"{"pack_version":"1.0.0","signed_at":1000,"channel":"pro","pro_revision":1}"#,
        )
        .unwrap();

        let items: Vec<(&str, &str)> = vec![("README.md", "NEW")];
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false, None);

        let pro_file = std::fs::read_to_string(pd.join("pro-only/skill.md")).unwrap_or_default();
        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(res.expect("보존 경로 Ok"), (0, 0), "거짓 free인데 install 진행됨");
        assert_eq!(pro_file, "PRO", "★거짓 free 자가치유가 pro 파일을 prune(R5 재앙 재현)");
        assert_eq!(soul, "OLD", "거짓 free인데 파일 덮임");
    }

    /// v6 음성 증거 ②: accepted 부재여도 pro 전용 파일 실재(임베드 외 설치 기록) → 자가치유 금지.
    #[test]
    fn embed_free_mismatch_with_pro_file_evidence_preserved() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        // install-manifest에 임베드 트리 밖 rel(pro-only/skill.md) = pro 파일 증거.
        let (base, pd, _env) = txn_prestate(
            "proevidence",
            &[("README.md", "OLD"), ("pro-only/skill.md", "PRO")],
            "1.0.0",
        );
        write_pack_state(&pd, &test_free_state("0.9.0")).unwrap();

        let items: Vec<(&str, &str)> = vec![("README.md", "NEW")];
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false, None);

        let pro_file = std::fs::read_to_string(pd.join("pro-only/skill.md")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(res.expect("보존 경로 Ok"), (0, 0));
        assert_eq!(pro_file, "PRO", "pro 파일 증거 무시하고 진행됨");
    }

    /// v5 checked 쓰기 순서 fault-injection: `.pack-version` 쓰기 실패(경로가 디렉터리) 시
    /// loud 처리 + state 미생성(불일치 미생성) + install 자체는 Ok(기존 best-effort 외부 동작).
    #[test]
    fn embed_version_write_failure_creates_no_state_mismatch() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-vfault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        let _env = set_pack_env(&pd, base.join("cfg"));
        // .pack-version 경로를 디렉터리로 만들어 write_atomic(rename) 실패 주입.
        std::fs::create_dir_all(pd.join(PACK_VERSION_FILE).join("child")).unwrap();

        let items: Vec<(&str, &str)> = vec![("README.md", "NEW")];
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false, None);

        let state_exists = pd.join(PACK_STATE_FILE).exists();
        let soul_exists = pd.join("README.md").exists();
        let _ = std::fs::remove_dir_all(&base);

        assert!(res.is_ok(), "version 쓰기 실패는 loud 경고일 뿐 Err 아님(기존 외부 동작)");
        assert!(soul_exists, "파일 반영 자체는 수행돼야 함");
        assert!(!state_exists, "version 실패인데 state가 생성됨(불일치 생성 = v5 위반)");
    }

    /// write_pack_state 실패(경로가 디렉터리)가 Err로 표면화됨을 핀 — 내장 경로의 state 동기
    /// 실패 loud 분기(Err 수신)가 실재 오류를 받는다는 보장.
    #[test]
    fn write_pack_state_failure_is_reported() {
        let base = std::env::temp_dir().join(format!("cys-sfault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join(PACK_STATE_FILE).join("child")).unwrap();
        assert!(write_pack_state(&base, &test_free_state("1.0.0")).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    // ─── §3.1 팩 atomic swap ───

    /// 성공 교체: staging→pack_dir, 기존 pack_dir→.prev(1세대 보존), staging 소진.
    #[test]
    fn atomic_swap_success_creates_prev() {
        let base = std::env::temp_dir().join(format!("cys-swap-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("pack");
        let staging = base.join("staging");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "old").unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("a.txt"), "new").unwrap();
        std::fs::write(staging.join("b.txt"), "b").unwrap();

        atomic_swap(&dir, &staging).unwrap();

        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "b");
        let prev = pack_prev_dir(&dir);
        assert!(prev.exists(), ".prev 1세대 보존");
        assert_eq!(std::fs::read_to_string(prev.join("a.txt")).unwrap(), "old");
        assert!(!staging.exists(), "staging은 교체로 소진");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 교체 전 abort(2번째 rename 실패: staging 부재) → 역rename으로 기존 팩 온전 복구.
    #[test]
    fn atomic_swap_reverses_on_failure_keeps_old_pack() {
        let base = std::env::temp_dir().join(format!("cys-swap-rev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("pack");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "old").unwrap();
        let staging = base.join("does-not-exist");

        let r = atomic_swap(&dir, &staging);

        assert!(r.is_err(), "staging 부재는 교체 실패");
        assert!(dir.exists(), "역rename으로 pack_dir 복구");
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "old",
            "pre-state 온전(반쯤 쓰인 팩 없음)"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 검증 실패 = 임베드 파일 누락 시 Err(교체 전 차단 방어선).
    #[test]
    fn verify_staging_detects_missing_file() {
        let base = std::env::temp_dir().join(format!("cys-verify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("present.txt"), "x").unwrap();
        let items = [("present.txt", "x"), ("missing.txt", "y")];

        let r = verify_staging(&base, &items);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("missing.txt"));

        std::fs::write(base.join("missing.txt"), "y").unwrap();
        assert!(verify_staging(&base, &items).is_ok(), "전부 존재 → Ok");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 신설 → written>0·임베드 반영·.prev 부재. 멱등 재설치 → written=0·pack 온전·.prev 1세대 생성.
    #[test]
    fn install_staged_fresh_then_idempotent_with_prev() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-staged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        let _env = set_pack_env(&pd, base.join("claude"));

        let (rel0, _) = PACK_ALL[0];

        let (w1, _k1) = install_staged(false, None).unwrap();
        assert!(w1 > 0, "신설은 written>0");
        assert!(pd.join(".pack-version").is_file(), ".pack-version 기록");
        assert!(pd.join(rel0).is_file(), "임베드 파일 반영");
        assert!(!pack_prev_dir(&pd).exists(), "첫 설치는 .prev 없음");

        let (w2, _k2) = install_staged(false, None).unwrap();
        assert_eq!(w2, 0, "멱등 재설치 written=0");
        assert!(pack_prev_dir(&pd).exists(), "재설치는 .prev 1세대 보존");
        assert!(pd.join(rel0).is_file(), "재설치 후 임베드 온전");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★성찰 차단 수리 핀(v2 §4 ① — staged 프로덕션 형상): init-pack(install_staged) 경유
    /// Merge3 의 캡처 레인은 물리 staging basename(`.pack-staging-init-<pid>` — ls 비표시 dot·
    /// pid 마다 레인 분산)이 아니라 **논리 대상(pack_dir) basename** 아래 생성된다 — 스코프의
    /// "staging basename 재판정 금지" 계약과의 정합(원장 capture 포인터·실파일 동시 검증).
    /// 비스테이징 형상은 merge3_crash_windows_converge 가 박제 — 이 핀이 staged 무핀 사각을 봉인.
    #[test]
    fn install_staged_capture_lane_uses_logical_pack_basename() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-staged-caplane-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        let _env = set_pack_env(&pd, base.join("claude"));
        let _cap_env = EnvGuard::remove("CYS_PACK_CAPTURES_DIR"); // 기본 유도(dir 형제) 검증

        install_staged(false, None).unwrap();

        // Merge3 픽스처(크래시 핀 [W1] 동형 — 머리/꼬리 분리 δ = clean): 조상 = 임베드 앞에
        // 구 헤더 1줄, ours = 조상 + 사용자 꼬리 1줄, theirs = 현 임베드(헤더 제거 = 벤더 전진).
        let (rel, embed) = PACK_ALL
            .iter()
            .find(|(r, c)| r.starts_with("skills/") && r.ends_with("/SKILL.md") && c.ends_with('\n'))
            .map(|(r, c)| (*r, *c))
            .expect("skills/*/SKILL.md 임베드 실재");
        let e_old = format!("LEGACY-HEAD-v0\n{embed}");
        let ours = format!("{e_old}USER-TAIL-DELTA\n");
        std::fs::write(pd.join(rel), &ours).unwrap();
        let mpath = pd.join(INSTALL_MANIFEST);
        let mut manifest: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(&mpath).unwrap()).unwrap();
        manifest.insert(rel.to_string(), content_hash(&e_old));
        std::fs::write(&mpath, serde_json::to_string(&manifest).unwrap()).unwrap();
        let pp = pd.join(PRISTINE_DIR).join(rel);
        std::fs::create_dir_all(pp.parent().unwrap()).unwrap();
        std::fs::write(&pp, &e_old).unwrap();

        install_staged(false, None).unwrap(); // 프로덕션 staged 스윕 → Merge3

        let pend = load_merge_pending(&pd);
        let entry = pend.get(rel).expect("원장 등재");
        assert_eq!(entry["kind"].as_str(), Some("merged"), "clean 병합: {entry}");
        let cap_rel = entry["capture"].as_str().expect("capture 필드").to_string();
        assert!(
            cap_rel.starts_with("pack/"),
            "캡처 레인 = 논리 팩 basename(스코프 주입 계약 동형): {cap_rel}"
        );
        assert!(
            !cap_rel.contains(".pack-staging"),
            "물리 staging basename 은닉·휘발 레인 금지: {cap_rel}"
        );
        // 실파일도 논리 레인 아래(원장 포인터로 해소 가능 = revert-merge 재료).
        let cap_path = base.join("pack-captures").join(&cap_rel);
        assert_eq!(
            std::fs::read_to_string(&cap_path).unwrap(),
            ours,
            "캡처 = 사용자본 전문(팩 밖 증거): {}",
            cap_path.display()
        );
        assert_eq!(
            std::fs::read_to_string(pd.join(rel)).unwrap(),
            format!("{embed}USER-TAIL-DELTA\n"),
            "병합 결과 = 벤더 전진 + 사용자 δ 생존"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★후계ⓐ(B2 전환 · v2 §6): 벤더 전진 + base 미검증 드리프트 → 치유 불변(healed).
    /// 픽스처는 의도적으로 base 검증 실패 형상(manifest≠hash(pristine)) — T3 이후에도 L4 healed
    /// 레인 핀으로 영속(병합 성공 레인은 T3 통합 핀이 별도 담당 — 이 핀을 Merge3 기대로 오개정 금지).
    #[test]
    fn system_edit_healed_when_vendor_advanced() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-staged-healA-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        let _env = set_pack_env(&pd, base.join("claude"));

        install_staged(false, None).unwrap();
        let sys = "alerts-config.json";
        let sys_embed = PACK_ALL.iter().find(|(r, _)| *r == sys).map(|(_, c)| *c)
            .expect("팩에 alerts-config.json 실재");
        std::fs::write(pd.join(sys), "SYS-EDIT-XYZ").unwrap();
        // 벤더 전진 시뮬(w1 bytewise 전례 동형): 마지막 적용본 해시를 과거로 되감는다.
        let mpath = pd.join(INSTALL_MANIFEST);
        let mut manifest: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(&mpath).unwrap()).unwrap();
        manifest.insert(sys.to_string(), content_hash("OLD-VENDOR-BASE"));
        std::fs::write(&mpath, serde_json::to_string(&manifest).unwrap()).unwrap();
        // base 미검증 고정: pristine 사본 제거(manifest≠hash(pristine) 형상 — L3 아닌 L4 레인).
        let _ = std::fs::remove_file(pd.join(PRISTINE_DIR).join(sys));

        install_staged(false, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(pd.join(sys)).unwrap(),
            sys_embed,
            "★후계ⓐ(B2 전환 · v2 §6): 벤더 전진+base 미검증 드리프트 → 치유 불변(healed)"
        );
        assert_eq!(
            std::fs::read_to_string(pd.join(format!("{sys}.user"))).unwrap(),
            "SYS-EDIT-XYZ",
            "치유 전 사용자본 보존 파괴 0 불변"
        );
        assert_eq!(
            load_merge_pending(&pd).get(sys).and_then(|e| e["kind"].as_str()),
            Some("healed"),
            "원장 kind==healed"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★후계ⓑ(B2 전환 · v2 §6): 벤더 미전진 드리프트 → kept-drift 제자리 보존.
    /// 현행 픽스처 그대로(1차 설치 후 sys 편집 → manifest 는 자연히 hash(embed)) — user_target
    /// 보존 assert 는 원 핀(install_staged_preserves_user_edit)에서 승계.
    #[test]
    fn system_edit_kept_when_vendor_static() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-staged-keptB-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        let _env = set_pack_env(&pd, base.join("claude"));

        install_staged(false, None).unwrap();
        // ★B2: user 소유 파일(soul.md 등 — 디렉티브 제외) 편집 보존 — 원 핀 픽스처 그대로.
        let user_target = PACK_ALL
            .iter()
            .find(|(rel, content)| is_user_owned(rel) && !rel.ends_with("_DIRECTIVE.md") && !content.starts_with("#!"))
            .map(|(rel, _)| *rel)
            .expect("user 소유 비-디렉티브 임베드 파일(soul.md 등) 존재");
        let sys_target = PACK_ALL
            .iter()
            .find(|(rel, content)| !is_user_owned(rel) && !content.starts_with("#!"))
            .map(|(rel, _)| *rel)
            .expect("system 비-shebang 임베드 파일 존재");
        std::fs::write(pd.join(user_target), "USER-EDIT-XYZ").unwrap();
        std::fs::write(pd.join(sys_target), "SYS-EDIT-XYZ").unwrap();

        install_staged(false, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(pd.join(user_target)).unwrap(),
            "USER-EDIT-XYZ",
            "★B2: user 소유 파일 편집 보존(force=false)"
        );
        assert_eq!(
            std::fs::read_to_string(pd.join(sys_target)).unwrap(),
            "SYS-EDIT-XYZ",
            "★후계ⓑ(B2 전환 · v2 §6): 벤더 미전진 드리프트 → kept-drift 제자리 보존"
        );
        assert!(!pd.join(format!("{sys_target}.user")).exists(), "kept-drift 는 .user 미생성");
        // ★T3(커밋①) 원장 계상: kind:"kept-drift" · side=rel — 가시화 4채널의 원천.
        let pend = load_merge_pending(&pd);
        assert_eq!(
            pend.get(sys_target).and_then(|e| e["kind"].as_str()),
            Some("kept-drift"),
            "★T3: kept-drift 원장 계상 부재 — D1 인터림 침묵 재발"
        );
        assert_eq!(
            pend.get(sys_target).and_then(|e| e["side"].as_str()),
            Some(sys_target),
            "kept-drift side=rel(제자리 보존 — 사이드카 아님)"
        );
        let ledger_bytes = std::fs::read(pd.join(MERGE_PENDING_FILE)).unwrap();
        // 재실행 멱등: kept-drift 가 상태를 흔들지 않는다.
        install_staged(false, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(pd.join(sys_target)).unwrap(),
            "SYS-EDIT-XYZ",
            "재실행 멱등"
        );
        // ★T3: 재실행 원장 no-op(ts 포함 바이트 불변) — 매 기동 원장 rewrite 방지 계약.
        assert_eq!(
            std::fs::read(pd.join(MERGE_PENDING_FILE)).unwrap(),
            ledger_bytes,
            "재실행이 원장을 rewrite 했다(upsert same-check 위반)"
        );
        // ★T3(D6) 수명: 드리프트 소멸(disk==embed 재일치)이면 kept-drift 항목 자동 제거.
        let sys_embed = PACK_ALL.iter().find(|(r, _)| *r == sys_target).map(|(_, c)| *c).unwrap();
        std::fs::write(pd.join(sys_target), sys_embed).unwrap();
        install_staged(false, None).unwrap();
        assert!(
            load_merge_pending(&pd).get(sys_target).is_none(),
            "★T3(D6): disk==embed 재일치 후에도 kept-drift 원장 유령 잔존"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★T6(R9 · v2 §7): 0.14.27 잔류 dept 스코프 팩의 릴리스 경계 스윕 — L0-L4 판정 레인이
    /// base 팩과 동일하게 흐른다(dept 분기 = soul 계열 SeedOnce 조기 반환뿐 · 판정·실행부는
    /// scope 파라미터 동일 적용). 픽스처: 실설치 dept 팩(pack-dept-2)의 .pack-version 을
    /// 0.14.27 로 되감은 잔류 형상(구 릴리스가 남긴 팩·구 manifest) → 현행 바이너리 스윕 1회.
    /// 검체 5종 — L0 잠금 자산 즉시 치유(벤더 미전진에도) · L1 판독 불가 바이트 백업 후 치유 ·
    /// L2 벤더 미전진 kept-drift 제자리 보존 · L3 벤더 전진+검증 base → Merge3(3자 상이 충돌
    /// → conflicted 폴백·.base 조상 보존) · L4 벤더 전진+base 미검증 → 종전 healed.
    /// soul.md 는 dept 스코프 SeedOnce 조기 반환이라 검체로 쓰지 않는다(함정 명문).
    #[test]
    fn dept_residual_pack_release_boundary_rides_l0_l4() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!(
            "cys-dept-l0l4-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let dpack = base.join("pack-dept-2");
        std::fs::create_dir_all(&dpack).unwrap();
        let _env = set_pack_env(&dpack, base.join("cfg"));
        let _cap = EnvGuard::set("CYS_PACK_CAPTURES_DIR", base.join("captures"));
        assert_eq!(pack_scope_of(&dpack), PackScope::Dept, "dept 스코프 전제");

        install_staged(false, None).unwrap();
        // 0.14.27 잔류 형상: 구 릴리스가 남긴 팩(.pack-version 되감기 — manifest·pristine 완비).
        std::fs::write(dpack.join(PACK_VERSION_FILE), "0.14.27").unwrap();

        // 검체 선별(동적·잠금 제외·상호 상이) — L1~L4 용 system rel 4종.
        let picks: Vec<&str> = PACK_ALL
            .iter()
            .filter(|(r, _)| ownership_name_scoped(r, &dpack) == "system" && !system_locked(r))
            .take(4)
            .map(|(r, _)| *r)
            .collect();
        assert_eq!(picks.len(), 4, "system 검체 4종 실재");
        let (l1, l2, l3, l4) = (picks[0], picks[1], picks[2], picks[3]);
        let embed_of =
            |rel: &str| PACK_ALL.iter().find(|(r, _)| *r == rel).map(|(_, c)| *c).unwrap();
        let manifest_rewind = |rel: &str, base_content: &str| {
            let mpath = dpack.join(INSTALL_MANIFEST);
            let mut m: std::collections::BTreeMap<String, String> =
                serde_json::from_str(&std::fs::read_to_string(&mpath).unwrap()).unwrap();
            m.insert(rel.to_string(), content_hash(base_content));
            std::fs::write(&mpath, serde_json::to_string(&m).unwrap()).unwrap();
        };

        // L0: 잠금 자산 드리프트(벤더 미전진) — 그래도 즉시 치유 대상.
        let l0 = "trusted-keys.json";
        std::fs::write(dpack.join(l0), "TAMPERED-KEYRING\n").unwrap();
        // L1: 판독 불가(CP949 바이트) 드리프트(벤더 미전진) — 바이트 백업 후 치유.
        let l1_raw: &[u8] = &[0xC7, 0xD1, 0xB1, 0xDB, 0x0A];
        std::fs::write(dpack.join(l1), l1_raw).unwrap();
        // L2: 가독 드리프트(벤더 미전진) — kept-drift 제자리 보존.
        std::fs::write(dpack.join(l2), "DEPT-DRIFT-L2\n").unwrap();
        // L3: 벤더 전진 + 검증 base(manifest==hash(pristine)) + 3자 상이 → 충돌 폴백.
        let l3_base = "L3-OLD-VENDOR-BASE\n";
        let l3_mine = "L3-MY-DEPT-EDIT\n";
        assert_ne!(embed_of(l3), l3_base, "L3 전제: 벤더 전진(embed≠base)");
        std::fs::write(dpack.join(l3), l3_mine).unwrap();
        manifest_rewind(l3, l3_base);
        let bp = dpack.join(PRISTINE_DIR).join(l3);
        std::fs::create_dir_all(bp.parent().unwrap()).unwrap();
        std::fs::write(&bp, l3_base).unwrap();
        // L4: 벤더 전진 + base 미검증(pristine 제거) — 종전 healed 거동(B2 후계ⓐ 동형).
        std::fs::write(dpack.join(l4), "DEPT-DRIFT-L4\n").unwrap();
        manifest_rewind(l4, "OLD-VENDOR-BASE");
        let _ = std::fs::remove_file(dpack.join(PRISTINE_DIR).join(l4));

        install_staged(false, None).unwrap(); // 릴리스 경계 스윕(0.14.27 → 현행)

        let read = |rel: &str| std::fs::read_to_string(dpack.join(rel)).unwrap();
        let kind = |rel: &str| {
            load_merge_pending(&dpack)
                .get(rel)
                .and_then(|e| e["kind"].as_str())
                .map(str::to_string)
        };
        // L0: 즉시 치유 + .user 백업(벤더 미전진에도 — kept-drift 로 새지 않는다).
        assert_eq!(read(l0), embed_of(l0), "L0 잠금 자산 즉시 치유");
        assert_eq!(read(&format!("{l0}.user")), "TAMPERED-KEYRING\n", "L0 .user 백업");
        assert_eq!(kind(l0).as_deref(), Some("healed"), "L0 원장 healed");
        // L1: 치유 + 바이트 백업(무백업 파괴 edge 봉인).
        assert_eq!(read(l1), embed_of(l1), "L1 판독 불가 치유");
        assert_eq!(
            std::fs::read(dpack.join(format!("{l1}.user"))).unwrap(),
            l1_raw,
            "L1 바이트 백업"
        );
        assert_eq!(kind(l1).as_deref(), Some("healed"), "L1 원장 healed");
        // L2: kept-drift 제자리 보존(.user 미생성).
        assert_eq!(read(l2), "DEPT-DRIFT-L2\n", "L2 제자리 보존");
        assert!(!dpack.join(format!("{l2}.user")).exists(), "L2 .user 미생성");
        assert_eq!(kind(l2).as_deref(), Some("kept-drift"), "L2 원장 kept-drift");
        // L3: Merge3 → 3자 상이 충돌 → vendor 본 + .user + .base 조상 + 원장 conflicted.
        assert_eq!(read(l3), embed_of(l3), "L3 충돌 폴백 = vendor 본");
        assert_eq!(read(&format!("{l3}.user")), l3_mine, "L3 .user 보존");
        assert_eq!(read(&format!("{l3}.base")), l3_base, "L3 .base 조상 보존");
        assert_eq!(kind(l3).as_deref(), Some("conflicted"), "L3 원장 conflicted");
        // L4: base 미검증 — 종전 healed(치유 + .user 보존).
        assert_eq!(read(l4), embed_of(l4), "L4 종전 healed 거동");
        assert_eq!(read(&format!("{l4}.user")), "DEPT-DRIFT-L4\n", "L4 .user 보존");
        assert_eq!(kind(l4).as_deref(), Some("healed"), "L4 원장 healed");
        // 릴리스 경계 완주: .pack-version 현행 재기록(잔류 해소).
        assert_eq!(
            std::fs::read_to_string(dpack.join(PACK_VERSION_FILE)).unwrap(),
            env!("CARGO_PKG_VERSION"),
            "릴리스 경계 스윕 후 .pack-version 현행"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// [회귀 핀·G3] --no-install-hook 일관성 — ① install_hooks=false 는 격리 config 의 라우터
    /// (CLAUDE.md — 훅 아님)는 시드하되 **훅 계열은 0**(settings.json 자체 미생성 = hooks 키 부재)
    /// ② install_hooks=true 는 종전 거동 그대로 소망 훅 집합 완비(기존 계약 핀). 종전엔 이
    /// 플래그가 ~/.claude 대상만 막고 격리 config dir 병합은 못 막았다(비일관 — 설계 원문 (b)).
    #[test]
    fn no_install_hook_consistency() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!(
            "cys-staged-nohook-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        let cfg = base.join("cysclaude");
        let _env = set_pack_env(&pd, &cfg);

        // ① 훅 억제 설치: 라우터 시드는 유지, 훅은 어느 계급에도 0.
        super::install_staged(false, None, false).unwrap();
        assert!(cfg.join("CLAUDE.md").is_file(), "라우터(CLAUDE.md)는 훅이 아니므로 시드 유지");
        assert!(
            !cfg.join("settings.json").exists(),
            "--no-install-hook 인데 격리 config 에 훅이 병합됐다(비일관 재발)"
        );

        // ② 기존 거동 핀: install_hooks=true 면 소망 훅 집합(AWAKENING_HOOKS) 등록 완비.
        super::install_staged(false, None, true).unwrap();
        assert!(
            verify_desired_hooks_registered(&cfg.join("settings.json"), &pd, &AWAKENING_HOOKS)
                .is_empty(),
            "install_hooks=true 종전 거동(소망 훅 완비) 회귀"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★W-A1(커스텀 생존 계약 2026-07-17): 비임베드 신규 파일(사용자 자작 도구·스킬 등 출하물이
    /// 아닌 파일)은 재설치 스윕·원자 교체·prune 어디서도 소실되지 않는다 — "자작 신규 파일은
    /// 절대 안전"을 제품 약속으로 박제하는 회귀 핀. 대조군으로 진짜 prune 대상(매니페스트에
    /// 등재됐지만 임베드에서 사라진 stale vendor 파일)은 정리됨을 함께 증명해, 이 보증이
    /// "prune 이 아예 안 도는" 우연이 아니라 판정(매니페스트 등재 여부)에 의한 것임을 고정한다.
    #[test]
    fn user_created_noembed_files_survive_install_and_prune() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-staged-noembed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        let _env = set_pack_env(&pd, base.join("claude"));

        install_staged(false, None).unwrap();

        // 자작 파일 3종(루트·bin·중첩 스킬) — 임베드·매니페스트 어디에도 없다.
        let customs = [
            ("USER-NOTES-ROOT.md", "내 메모"),
            ("bin/my_custom_tool.py", "#!/usr/bin/env python3\nprint('mine')\n"),
            ("skills/my-jarvis-skill/SKILL.md", "---\nname: my-jarvis-skill\n---\n"),
        ];
        for (rel, content) in customs {
            let p = pd.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
        }
        // 대조군: 임베드에서 제거된 stale vendor 파일(매니페스트 등재 + 해시 일치 → prune 대상).
        let stale_rel = "bin/legacy_gone_tool.py";
        std::fs::write(pd.join(stale_rel), "OLD-VENDOR").unwrap();
        let mpath = pd.join(INSTALL_MANIFEST);
        let mut manifest: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(&mpath).unwrap()).unwrap();
        manifest.insert(stale_rel.to_string(), content_hash("OLD-VENDOR"));
        std::fs::write(&mpath, serde_json::to_string(&manifest).unwrap()).unwrap();

        // force 재설치(가장 공격적인 스윕) + 원자 교체 경로 통과.
        install_staged(true, None).unwrap();

        for (rel, content) in customs {
            assert_eq!(
                std::fs::read_to_string(pd.join(rel)).unwrap(),
                content,
                "★생존 계약: 비임베드 자작 파일은 force 스윕·prune·원자 교체에서 불가침 — {rel}"
            );
        }
        assert!(
            !pd.join(stale_rel).exists(),
            "대조군: 매니페스트 등재 stale vendor 파일은 prune 됨(판정이 살아있음의 증명)"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    // ═════════════════════════════════════════════════════════════════════════
    // U-19 · 첫기동 관문 시드 (C-4) — V-h 실측을 계약으로 박제한다
    // ═════════════════════════════════════════════════════════════════════════

    fn u19_tmp(tag: &str) -> PathBuf {
        let td = std::env::temp_dir().join(format!("cys-u19-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        td
    }

    fn u19_json(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    /// 롤백 스위치 진리표 — **기본은 꺼짐**이고 마스터가 축 노브를 이긴다.
    ///
    /// 극성이 형제 게이트와 반대인 이유는 모듈 문서에 있다(이 축은 사용자 파일에 쓰고 안전
    /// 화면을 지운다 → 잊으면 꺼져 있어야 오늘과 같다).
    #[test]
    fn u19_seed_switch_defaults_off_and_master_switch_wins() {
        // 축 노브만 볼 때: "1" 만 켠다(느슨한 truthy 금지 — 형제 게이트와 같은 엄격 비교).
        for v in [None, Some(""), Some("0"), Some("true"), Some("yes"), Some("on"), Some("2")] {
            assert!(
                !first_run_seed_enabled_from(v, false),
                "축 노브 {v:?} 가 시드를 켰다 — 기본 꺼짐 계약 위반"
            );
        }
        assert!(first_run_seed_enabled_from(Some("1"), false), "명시 1 이 켜지 않는다");
        // 마스터(`CYS_BOOT_GATES=0`)가 눌리면 축 노브가 "1" 이어도 꺼진다.
        assert!(
            !first_run_seed_enabled_from(Some("1"), true),
            "마스터 스위치가 이 축에 닿지 않는다 — BLOCK-3 형태의 반쪽 롤백"
        );
    }

    /// ★V-h ③ 박제 — **인증 증명 없이 로그인 관문을 지우지 않는다.**
    ///
    /// 실측(2026-08-24 · 2.1.241 · macOS): 자격증명이 하나도 없는 프로필에
    /// `hasCompletedOnboarding:true` 만 넣으면 테마·로그인 관문이 **둘 다 사라지고** 좌석이
    /// 프롬프트에 도달해 65초 뒤에도 살아 있었다(상태줄 `Not logged in`). 그 좌석은 영원히
    /// READY 로 보이지만 아무 일도 못 한다 — 관문 앞에서 멈춘 좌석보다 나쁘다.
    ///
    /// ★계측 타당성(in-band): 수리 전 술어("프리미스와 무관하게 온보딩 키를 넣는다")를 아래에
    /// 재현해, 그 술어라면 미인증 프로필에 키가 들어갔음을 같은 검체 안에서 보인다.
    #[test]
    fn u19_plan_refuses_login_gate_erasure_without_auth_premise() {
        let fresh = u19_json("{}");

        let unproven = plan_first_run_seed(&fresh, &[], AuthPremise::Unproven);
        assert!(unproven.refused.is_none(), "정상 입력을 거부했다: {unproven:?}");
        assert!(
            unproven.added.is_empty(),
            "미인증 프로필에 무언가를 시드했다 — 허위 READY 좌석 제조: {:?}",
            unproven.added
        );
        assert!(
            unproven.next.get(SEED_KEY_ONBOARDING).is_none(),
            "미인증인데 로그인 관문이 지워졌다"
        );

        let verified = plan_first_run_seed(&fresh, &[], AuthPremise::Verified);
        assert_eq!(
            verified.next.get(SEED_KEY_ONBOARDING),
            Some(&serde_json::Value::Bool(true)),
            "인증 증명이 있는데도 온보딩 키가 들어가지 않았다"
        );
        assert_eq!(verified.added, vec![SEED_KEY_ONBOARDING.to_string()]);

        // ── 계측 타당성: 수리 전 술어 재현 ──
        let legacy_predicate = |_: AuthPremise| true; // 구 설계: 프리미스 개념 자체가 없었다
        assert!(
            legacy_predicate(AuthPremise::Unproven),
            "구 술어 재현이 실패했다 — 이 검체는 무엇도 증명하지 못한다"
        );
        // 구 술어였다면 미인증 프로필에 키가 들어갔을 것이다. 지금 판정과 갈린다는 것이
        // 이 수리가 실재한다는 증거다.
        assert_ne!(
            legacy_predicate(AuthPremise::Unproven),
            !unproven.added.is_empty(),
            "신·구 술어가 같은 답을 낸다 — 로그인 관문 보호가 실재하지 않는다"
        );
    }

    /// 부재 키만 채운다 · 다른 키는 **전부** 보존한다(`oauthAccount` 포함).
    ///
    /// `oauthAccount` 보존은 U-19 게이트 4항 중 하나다. 시드가 신원 봉투를 갈아치우면
    /// `accounts.rs` 의 신원 판정(=`oauthAccount.accountUuid` 존재)이 통째로 흔들린다.
    #[test]
    fn u19_plan_is_additive_and_preserves_every_other_key() {
        let before = u19_json(
            r#"{
                "oauthAccount": {"accountUuid": "c45eaec5-0000-0000-0000-000000000000",
                                 "emailAddress": "x@example.com"},
                "userID": "u", "machineID": "m", "migrationVersion": 13,
                "hasCompletedOnboarding": false,
                "projects": {"/keep": {"hasTrustDialogAccepted": false, "allowedTools": []}}
            }"#,
        );
        // ★`/keep` 을 **시드 대상에 포함**한다 — 대상이 아닌 항목이 안 바뀌는 것은 자명하고,
        //   진짜 계약은 "시드하러 간 항목이 이미 값을 갖고 있으면 덮지 않는다" 다.
        let ws = vec!["/keep".to_string(), "/new".to_string()];
        let plan = plan_first_run_seed(&before, &ws, AuthPremise::Verified);
        assert!(plan.refused.is_none(), "정상 입력을 거부했다: {plan:?}");

        // ① 이미 있는 값은 `false` 여도 덮지 않는다.
        assert_eq!(
            plan.next.get(SEED_KEY_ONBOARDING),
            Some(&serde_json::Value::Bool(false)),
            "사용자가 false 로 둔 키를 덮었다 — 부재 키만 채운다는 계약 위반"
        );
        assert_eq!(
            plan.next
                .pointer("/projects/~1keep/hasTrustDialogAccepted"),
            Some(&serde_json::Value::Bool(false)),
            "기존 워크스페이스의 false 신뢰를 덮었다"
        );

        // ② oauthAccount 는 비트 단위로 그대로다.
        assert_eq!(
            plan.next.get("oauthAccount"),
            before.get("oauthAccount"),
            "★oauthAccount 가 변조됐다 — 신원 봉투 보존 계약 위반"
        );
        for k in ["userID", "machineID", "migrationVersion"] {
            assert_eq!(plan.next.get(k), before.get(k), "무관한 키 {k} 가 바뀌었다");
        }
        assert_eq!(
            plan.next.pointer("/projects/~1keep/allowedTools"),
            before.pointer("/projects/~1keep/allowedTools"),
            "기존 워크스페이스의 다른 필드가 사라졌다"
        );

        // ③ 새 워크스페이스에만 신뢰가 붙는다.
        assert_eq!(
            plan.next.pointer("/projects/~1new/hasTrustDialogAccepted"),
            Some(&serde_json::Value::Bool(true))
        );
        assert_eq!(
            plan.added,
            vec![format!("{SEED_KEY_PROJECTS}[/new].{SEED_KEY_TRUST}")]
        );
    }

    /// 한글 경로 픽스처 — NFC/NFD 어느 형태로 들어와도 **기존 항목을 찾아** 붙고,
    /// 새로 만들 때는 NFC 로 쓴다(중복 항목 0).
    #[test]
    fn u19_project_key_lookup_spans_nfc_and_nfd() {
        let nfc = "/Users/x/바탕화면/작업"; // 사람이 손으로 적은 형태
        let nfd = hangul_nfd(nfc); // macOS 파일시스템이 돌려주는 형태
        assert_ne!(nfc, nfd.as_str(), "픽스처가 두 형태로 갈리지 않는다 — 검체 무효");

        // ⓐ 디스크에 NFD 로 있고 요청이 NFC → **기존 항목**에 붙는다(중복 생성 0).
        let disk_nfd = serde_json::json!({ "projects": { nfd.clone(): {} } });
        let p = plan_first_run_seed(&disk_nfd, &[nfc.to_string()], AuthPremise::Unproven);
        let projects = p.next.get(SEED_KEY_PROJECTS).unwrap().as_object().unwrap();
        assert_eq!(projects.len(), 1, "NFC/NFD 중복 항목이 생겼다: {:?}", projects.keys());
        assert_eq!(
            projects.get(nfd.as_str()).unwrap().get(SEED_KEY_TRUST),
            Some(&serde_json::Value::Bool(true)),
            "기존 NFD 항목에 신뢰가 붙지 않았다(엉뚱한 항목에 붙었다 = 관문 그대로)"
        );

        // ⓑ 디스크에 NFC 로 있고 요청이 NFD → 역방향도 같다.
        let disk_nfc = serde_json::json!({ "projects": { nfc: {} } });
        let q = plan_first_run_seed(&disk_nfc, &[nfd.clone()], AuthPremise::Unproven);
        let qp = q.next.get(SEED_KEY_PROJECTS).unwrap().as_object().unwrap();
        assert_eq!(qp.len(), 1, "역방향에서 중복 항목이 생겼다");
        assert!(qp.contains_key(nfc), "역방향에서 기존 NFC 항목을 못 찾았다");

        // ⓒ 아무것도 없으면 **NFC** 로 만든다(사람이 적는 형태와 갈리지 않게).
        let empty = u19_json("{}");
        let r = plan_first_run_seed(&empty, &[nfd.clone()], AuthPremise::Unproven);
        let rp = r.next.get(SEED_KEY_PROJECTS).unwrap().as_object().unwrap();
        assert!(
            rp.contains_key(nfc),
            "새 항목이 NFC 가 아니다: {:?}",
            rp.keys().collect::<Vec<_>>()
        );
    }

    /// 한글 조합·분해가 산술적으로 정확한가(외부 크레이트 없이 구현한 정본의 자기검증).
    #[test]
    fn u19_hangul_normalization_is_exact_and_scoped() {
        // 받침 있음/없음 · 비한글 혼재 · 왕복
        for s in ["한글", "가", "값", "뷁", "/a/한/b-1_c.txt", "ascii only", "日本語", ""] {
            let d = hangul_nfd(s);
            assert_eq!(hangul_nfc(&d), s, "왕복이 깨졌다: {s:?} → {d:?}");
        }
        // 분해 결과는 자모 영역이다(조합형이 남아 있으면 폴딩이 헛돈다).
        assert!(
            hangul_nfd("값").chars().all(|c| (0x1100..0x1200).contains(&(c as u32))),
            "분해 결과에 조합형이 남았다"
        );
        assert_eq!(hangul_nfd("값").chars().count(), 3, "종성이 분리되지 않았다");
        assert_eq!(hangul_nfd("가").chars().count(), 2, "종성 없는 음절 분해가 틀렸다");
        // 비한글은 손대지 않는다(범위 한정 — 모르는 것을 아는 척 접지 않는다).
        for s in ["café", "Ω", "e\u{0301}"] {
            assert_eq!(hangul_nfd(s), s, "비한글을 건드렸다: {s:?}");
            assert_eq!(hangul_nfc(s), s, "비한글을 건드렸다: {s:?}");
        }
    }

    /// 모르는 형태는 **거부**한다 — 파손 파일을 우리 스키마로 덮는 것이 더 위험하다.
    #[test]
    fn u19_plan_refuses_unknown_shapes_without_writing() {
        for bad in ["[]", "\"str\"", "3", "null"] {
            let v = u19_json(bad);
            let p = plan_first_run_seed(&v, &[], AuthPremise::Verified);
            assert!(p.refused.is_some(), "최상위 {bad} 를 덮으려 했다");
            assert!(p.added.is_empty());
            assert_eq!(p.next, v, "거부했는데 값이 바뀌었다");
        }
        let bad_projects = u19_json(r#"{"projects": "not-an-object"}"#);
        let p = plan_first_run_seed(&bad_projects, &["/x".to_string()], AuthPremise::Verified);
        assert!(p.refused.is_some(), "projects 가 문자열인데 덮으려 했다");
        assert_eq!(p.next, bad_projects, "거부했는데 값이 바뀌었다");

        // 워크스페이스 항목이 객체가 아니면 **그 항목만** 건너뛰고 보존한다.
        let odd = u19_json(r#"{"projects": {"/x": 7}}"#);
        let q = plan_first_run_seed(&odd, &["/x".to_string()], AuthPremise::Unproven);
        assert!(q.refused.is_none());
        assert!(q.added.is_empty(), "객체가 아닌 항목에 키를 밀어 넣었다");
        assert_eq!(q.next, odd, "보존 대상이 바뀌었다");
    }

    /// IO — 백업 생성 · 원자 교체 · **퍼미션 보존(0600)** · 되읽기 검증.
    #[test]
    fn u19_seed_writes_backup_and_keeps_permissions() {
        let td = u19_tmp("io");
        let cfg = td.join("iso");
        std::fs::create_dir_all(&cfg).unwrap();
        let target = cfg.join(CLAUDE_CONFIG_FILE);
        let before = r#"{"userID":"u","oauthAccount":{"accountUuid":"a"}}"#;
        std::fs::write(&target, before).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        let ws = vec![cfg.to_string_lossy().into_owned()];
        let out = seed_first_run_gates_at(&cfg, &ws, AuthPremise::Verified);
        match &out {
            SeedOutcome::Seeded(added) => assert_eq!(added.len(), 2, "추가분 셈이 틀렸다: {added:?}"),
            other => panic!("시드가 성립하지 않았다: {other:?}"),
        }

        // 롤백 경로: 원본이 옆자리에 그대로.
        let bak = std::fs::read_to_string(cfg.join(FIRST_RUN_SEED_BACKUP)).unwrap();
        assert_eq!(bak, before, "백업이 원본과 다르다 — 롤백 경로가 거짓이다");

        // 결과: 부재 키만 추가 · oauthAccount 보존.
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(after.get("userID"), Some(&serde_json::json!("u")));
        assert_eq!(
            after.pointer("/oauthAccount/accountUuid"),
            Some(&serde_json::json!("a")),
            "★oauthAccount 가 시드로 변조됐다"
        );
        assert_eq!(after.get(SEED_KEY_ONBOARDING), Some(&serde_json::json!(true)));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                m, 0o600,
                "★퍼미션이 넓어졌다(0{m:o}) — tmp+rename 이 mode 를 버렸다"
            );
        }

        // 멱등: 다시 돌리면 넣을 것이 없다.
        assert_eq!(
            seed_first_run_gates_at(&cfg, &ws, AuthPremise::Verified),
            SeedOutcome::NothingToDo,
            "두 번째 시드가 다시 썼다(멱등 아님)"
        );

        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★개인 프로필 무접촉 — 목 HOME 의 `.claude`·`.claude-2` 가 **한 바이트도** 바뀌지 않는다.
    ///
    /// `merge_awakening_hooks_into_personal_profiles` 와 이 단계를 **절대 공유하지 않는다**는
    /// 계약의 기계 집행자다. 시드가 개인 프로필로 번지면 격리의 의미가 사라진다.
    #[test]
    fn u19_seed_never_touches_personal_profiles() {
        let td = u19_tmp("personal");
        let home = td.join("home");
        let mut fixtures = Vec::new();
        for p in [".claude", ".claude-2"] {
            let d = home.join(p);
            std::fs::create_dir_all(&d).unwrap();
            let f = d.join(CLAUDE_CONFIG_FILE);
            let body = format!("{{\"personal\":\"{p}\",\"hasCompletedOnboarding\":false}}");
            std::fs::write(&f, &body).unwrap();
            fixtures.push((f, body));
        }
        // 홈 직하 파일도 함께 동결한다(`~/.claude.json` 은 실재하는 경로다).
        let home_json = home.join(CLAUDE_CONFIG_FILE);
        std::fs::write(&home_json, "{\"personal\":\"home\"}").unwrap();
        fixtures.push((home_json, "{\"personal\":\"home\"}".to_string()));

        let cfg = td.join("iso");
        std::fs::create_dir_all(&cfg).unwrap();
        let out = seed_first_run_gates_at(
            &cfg,
            &[home.to_string_lossy().into_owned()],
            AuthPremise::Verified,
        );
        assert!(matches!(out, SeedOutcome::Seeded(_)), "시드가 성립하지 않았다: {out:?}");

        for (f, want) in &fixtures {
            assert_eq!(
                &std::fs::read_to_string(f).unwrap(),
                want,
                "★개인 프로필이 변조됐다: {}",
                f.display()
            );
        }
        // 개인 프로필 dir 에 백업·tmp 같은 부산물도 생기지 않았다.
        for p in [".claude", ".claude-2"] {
            let names: Vec<String> = std::fs::read_dir(home.join(p))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            assert_eq!(
                names,
                vec![CLAUDE_CONFIG_FILE.to_string()],
                "개인 프로필 dir 에 부산물이 생겼다: {names:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&td);
    }

    /// 파손 config 는 **읽고 거부**한다 — 한 바이트도 쓰지 않는다.
    #[test]
    fn u19_seed_refuses_corrupt_config_untouched() {
        let td = u19_tmp("corrupt");
        let cfg = td.join("iso");
        std::fs::create_dir_all(&cfg).unwrap();
        let target = cfg.join(CLAUDE_CONFIG_FILE);
        let junk = "{ this is not json";
        std::fs::write(&target, junk).unwrap();

        let out = seed_first_run_gates_at(&cfg, &["/w".to_string()], AuthPremise::Verified);
        assert!(matches!(out, SeedOutcome::Refused(_)), "파손 파일을 덮으려 했다: {out:?}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), junk, "파손 파일이 바뀌었다");
        assert!(
            !cfg.join(FIRST_RUN_SEED_BACKUP).exists(),
            "쓰지도 않았는데 백업이 생겼다"
        );
        let _ = std::fs::remove_dir_all(&td);
    }

    /// `write_atomic` 의 종전 경로(`mode=None`)는 무변경이고, `Some(m)` 은 **생성 시점부터**
    /// 그 퍼미션이다(사후 chmod 창 0).
    #[test]
    fn u19_write_atomic_mode_applies_at_creation() {
        let td = u19_tmp("mode");
        let a = td.join("none.txt");
        write_atomic(&a, b"x").unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "x");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let b = td.join("mode.txt");
            write_atomic_mode(&b, b"y", Some(0o600)).unwrap();
            assert_eq!(
                std::fs::metadata(&b).unwrap().permissions().mode() & 0o777,
                0o600
            );
            // 잔여 tmp 가 있어도 목표 mode 로 만들어진다(`.mode()` 는 생성 시에만 유효).
            let c = td.join("stale.txt");
            let tmp = td.join(format!(".stale.txt.tmp.{}", std::process::id()));
            std::fs::write(&tmp, b"junk").unwrap();
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o666)).unwrap();
            write_atomic_mode(&c, b"z", Some(0o600)).unwrap();
            assert_eq!(
                std::fs::metadata(&c).unwrap().permissions().mode() & 0o777,
                0o600,
                "잔여 tmp 때문에 퍼미션이 넓어졌다"
            );
        }
        let _ = std::fs::remove_dir_all(&td);
    }

    /// 설치 경로 호출부는 **오늘 아무것도 하지 않는다** — 스위치 기본 꺼짐 ∧ 프리미스 미증명
    /// ∧ 워크스페이스 0. 세 이유가 **독립적으로** 성립한다(하나가 풀려도 나머지가 막는다).
    #[test]
    fn u19_install_time_call_site_is_inert_by_three_independent_reasons() {
        assert!(
            INSTALL_TIME_SEED_WORKSPACES.is_empty(),
            "설치 시점 호출부가 임의 워크스페이스를 신뢰하려 한다"
        );
        // 프리미스 미증명 + 워크스페이스 0 → 계획기 산출이 비어 있다(스위치와 무관하게).
        let p = plan_first_run_seed(
            &u19_json("{}"),
            INSTALL_TIME_SEED_WORKSPACES,
            AuthPremise::Unproven,
        );
        assert!(p.refused.is_none());
        assert!(p.added.is_empty(), "설치 시점 호출부가 무언가를 쓰려 한다: {:?}", p.added);
        // 스위치 기본값도 꺼짐이다.
        assert!(!first_run_seed_enabled_from(None, false));
    }
}
