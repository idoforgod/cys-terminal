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
}

/// **각성 티어**(awakening) — 없으면 마스터 선언이 부트를 발화하지 못하는(=팀이 영구히 안 뜨는)
/// 훅 집합. Rust 격리 config 시드·init-pack·개인 프로필 병합(T-0147-1)·preflight C28의 FAIL 티어가
/// **모두 이 집합**을 소비한다. 순서는 계약이 아니지만(집합 대조) 결정론 출력을 위해 고정한다.
pub const AWAKENING_HOOKS: [DesiredHook; 2] = [
    DesiredHook {
        script: "session-start.sh",
        event: "SessionStart",
        matcher: None,
    },
    DesiredHook {
        script: "role-bootstrap.sh",
        event: "UserPromptSubmit",
        matcher: None,
    },
];

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

/// settings.json 의 특정 이벤트에 desired 명령이 등록돼 있는가(순수 판정 · 읽기 전용).
pub fn hook_registered_in(root: &serde_json::Value, event: &str, desired: &str) -> bool {
    root.get("hooks")
        .and_then(|h| h.get(event))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().any(|m| {
                m.get("hooks")
                    .and_then(|v| v.as_array())
                    .map(|hs| {
                        hs.iter()
                            .any(|h| h.get("command").and_then(|c| c.as_str()) == Some(desired))
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// 소망 훅을 settings.json 에 **이벤트 단위로 멱등 병합**한다(A9).
///
/// 계약(사용자 불가침 — ★W-B seed-once 교리 정합):
///  · **추가만 한다**. 사용자 항목·타 도구 훅·기존 배열 순서를 지우거나 재배치하지 않는다.
///  · 이미 같은 명령이 그 이벤트에 있으면 **무동작**(byte-identical 문자열이라 중복 0).
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
        if hook_registered_in(&root, h.event, &desired) {
            continue;
        }
        let mut entry = serde_json::json!({"hooks": [{"type": "command", "command": desired}]});
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
        .filter(|h| !hook_registered_in(&root, h.event, &hook_command_for(pack_dir, h.script)))
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
    // RC-2 잔여(T2.1·codex CONFIRMED): 공백 포함 경로(C:\Users\John Doe\.cys\pack\...) 대응 — Windows만
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
/// 사용자 프로필을 건드리지도(읽지도·지우지도) 않는다. macOS 인증은 계정 단위 Keychain이라
/// 격리해도 로그인이 유지된다(우리 DMG는 macOS 전용). base 팩은 pack_dir 형제(~/.cys/claude).
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
    if !install_hooks {
        // ★G3(--no-install-hook 일관성): 종전엔 이 플래그가 ~/.claude 대상만 막고 격리 config dir
        // 훅 병합(아래)은 그대로 돌았다 — 훅 억제를 요청한 운영자에게 훅이 몰래 등록되는 비일관.
        // 라우터는 훅이 아니므로 위에서 시드 유지, 훅 계열(병합·검증·개인 프로필)은 전부 생략.
        println!("[pack] 훅 미설치(--no-install-hook) — 격리 config·개인 프로필 훅 등록 생략");
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
/// 해시 불일치 시 강제 갱신), **user 는 화이트리스트만**(사용자 수정 보존). 화이트리스트를 좁게 유지해
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
    /// 임베드 진실(기본값) — 불일치 시 강제 치유(P0-4)·비수정 자동 갱신·prune 대상.
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
//     - system 수정본 치유 → 덮어쓰기 **전에** `<rel>.user` 로 사용자본 보존(파괴 0)
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileAction {
    /// 임베드 내용을 디스크에 기록. heal_user_copy=true 면 사용자 수정본을 `<rel>.user` 로 먼저 보존.
    Write { heal_user_copy: bool },
    /// 디스크 유지. adopt_hash=true 면 매니페스트에 현재 임베드 해시 채택(구설치본 승계).
    /// new_pending=true 면 임베드 신버전을 `<rel>.new` 로 병치(병합 대기 — user-owned 동결 해소 경로).
    Keep { adopt_hash: bool, new_pending: bool },
}

/// 현행 install_into 분기(★B2 user-owned 영구 보존 · P0-4 system 강제 치유 · 비수정 자동 갱신)를
/// 글자 그대로 보존한 순수 판정 + 신규 부수효과 플래그(heal_user_copy·new_pending)만 추가한다.
/// ★G3 축2: `scope` 는 데이터 파라미터(순수성 유지) — Base 면 종전 등급표와 byte-identical 이고
/// (기존 핀 전량이 Base 셈으로 무수정 초록 = 하위호환의 기계 증거), Dept 면 soul.md 만 SeedOnce.
pub(crate) fn decide_file_action(
    rel: &str,
    embed: &str,
    exists: bool,
    disk: Option<&str>, // None = 부재 또는 읽기 실패(비UTF-8 등)
    manifest_hash: Option<&str>,
    force: bool,
    scope: PackScope,
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
            // 안 된다. system 등급은 이 분기 밖이라 P0-4 강제 치유는 그대로 유지된다.
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
                // system: 강제 치유(P0-4 — 임베드 진실). 진짜 사용자 수정본이면 먼저 .user 보존.
                let heal = matches!(disk, Some(d) if d != embed
                    && manifest_hash != Some(content_hash(d).as_str()));
                return FileAction::Write { heal_user_copy: heal };
            }
        }
    }
    // 신규 생성 또는 force 갱신 — force 로 수정본을 덮을 때도 사용자본은 보존한다(파괴 0).
    let heal = exists
        && matches!(disk, Some(d) if d != embed
            && manifest_hash != Some(content_hash(d).as_str()));
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

/// install 드라이런 리포트(④ 투명성) — `cys pack-plan` 이 설치 **전에** 사용자에게 보여준다.
#[derive(Debug, Default)]
pub struct InstallPlan {
    pub create: Vec<String>,              // 신규 생성
    pub update: Vec<String>,              // 자동 갱신(비수정 system)
    pub heal: Vec<String>,                // 수정본 강제 치유(사용자본 `<rel>.user` 보존 후 덮어씀)
    pub merge_new: Vec<String>,           // user-owned 보존 + 신버전 `<rel>.new` 병치(병합 대기)
    pub keep_user: Vec<String>,           // user-owned 보존(신버전 병치 불요)
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
        match decide_file_action(
            rel,
            content,
            exists,
            disk.as_deref(),
            manifest.get(rel).map(String::as_str),
            force,
            scope,
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
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    let fname = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    let tmp = parent.join(format!(".{fname}.tmp.{}", std::process::id()));
    let res = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
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

/// pristine 미러 갱신(best-effort — 3-way 병합의 공통 조상 확보용 자문 데이터).
/// **디스크에 실제 적용된** vendor 내용일 때만 호출된다 — user-owned 동결 파일에는 호출하지
/// 않아 조상이 사용자가 fork 한 시점의 vendor 본으로 남는다(3-way 정확성의 핵심).
fn ensure_pristine(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(PRISTINE_DIR).join(rel);
    if std::fs::read_to_string(&p).ok().as_deref() == Some(content) {
        return;
    }
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = write_atomic(&p, content.as_bytes());
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
fn seed_dept_soul_content(dir: &Path, rel: &str, embed: &str) -> String {
    let base_soul = dir.parent().map(|p| p.join("pack").join("soul.md"));
    let inherited = base_soul
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .filter(|s| !s.trim().is_empty());
    let (content, source, verify) = match inherited {
        Some(s) => {
            let lower = s.to_lowercase();
            let has_core = crate::overrides::SAFETY_KEYWORDS
                .iter()
                .any(|kw| lower.contains(kw));
            if !has_core {
                eprintln!(
                    "[pack] ⚠ 부서 soul 시드: base 헌장 승계본에 안전핵 키워드 0건 — base soul 이 \
                     과도 상태일 수 있음(시드는 진행 · 감사 원장에 기록)"
                );
            }
            let src = base_soul
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            (s, src, if has_core { "pass" } else { "warn-no-safety-core" })
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
        "flags": [],
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
/// force=false: user-owned(디렉티브·soul·CLAUDE·schedule)·seed-once(상태·기억) 불가침, **수정된
/// system 파일은 임베드로 치유**(사용자본 `<rel>.user` 보존), 비수정 파일은 입력 신버전으로 자동 갱신.
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
pub fn install_into<'a, I: IntoIterator<Item = (&'a str, &'a str)>>(
    dir: PathBuf,
    items: I,
    force: bool,
    target_version: &str,
    transactional: bool,
    setup_config: bool,
    scope: PackScope,
    auth: Option<PackWriteAuth>,
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
    // 판정 자체는 decide_file_action(순수 — ★B2 user-owned 영구 보존 · P0-4 system 강제 치유 ·
    // 비수정 자동 갱신을 글자 그대로 보존)에 위임하고, 여기는 부수효과만 수행한다.
    let mut pending = load_merge_pending(&dir);
    let mut pending_dirty = false;
    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 병합 대기 항목 upsert — kind·side·version 이 이미 같으면 no-op(매 기동 install 의 원장 rewrite 방지).
    let mut upsert_pending = |pending: &mut serde_json::Map<String, serde_json::Value>,
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
        match decide_file_action(rel, content, exists, disk.as_deref(), mhash.as_deref(), force, scope)
        {
            FileAction::Keep { adopt_hash, new_pending } => {
                if adopt_hash {
                    // 디스크 = 임베드: 최신. 매니페스트 공백(구설치본)이면 채택 기록해
                    // 다음 버전부터 자동 갱신 대상이 되게 한다. pristine 승계도 보장.
                    manifest
                        .entry(rel.to_string())
                        .or_insert_with(|| content_hash(content));
                    ensure_pristine(&dir, rel, content);
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
            FileAction::Write { heal_user_copy } => {
                if heal_user_copy {
                    // system 강제 치유(P0-4)·force 갱신이 사용자 수정본을 덮기 **전에** 보존(파괴 0).
                    if let Some(d) = disk.as_deref() {
                        let user_path = dir.join(format!("{rel}.user"));
                        if std::fs::read_to_string(&user_path).ok().as_deref() != Some(d) {
                            let _ = write_atomic(&user_path, d.as_bytes());
                        }
                        upsert_pending(&mut pending, &mut pending_dirty, rel, "healed", format!("{rel}.user"));
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
        // ★W-E2: 사용자 언어 요약 — "지워진 게 아니라 보존됐다"를 등급별 수치로 즉시 증명.
        let healed_n = pending.values()
            .filter(|e| e.get("kind").and_then(|k| k.as_str()) == Some("healed"))
            .count();
        let new_n = pending.len() - healed_n;
        // ★W-1: 판독 불가 보존분은 원장에 없다 — 위 수치가 '전량'으로 읽히지 않게 같은 줄에 덧붙인다.
        let unreadable_note = if unreadable_kept.is_empty() {
            String::new()
        } else {
            format!(" · 판독 불가라 손대지 않음 {}건(위 안내)", unreadable_kept.len())
        };
        println!(
            "[init-pack] 내 커스텀은 지워지지 않았습니다 — 병합 대기 {}건 (내 수정본 .user 보존 {}건 · vendor 신버전 .new 병치 {}건){}\n\
             \x20 검토·병합: `cys pack-merge` · 직전 상태 파일 복원: `cys pack-rollback`",
            pending.len(), healed_n, new_n, unreadable_note
        );
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
    install_into(dir, items, force, target_version, transactional, true, scope, auth)
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
    fn decide_file_action(
        rel: &str,
        embed: &str,
        exists: bool,
        disk: Option<&str>,
        manifest_hash: Option<&str>,
        force: bool,
    ) -> FileAction {
        super::decide_file_action(rel, embed, exists, disk, manifest_hash, force, PackScope::Base)
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
        let manifest = serde_json::json!({
            "README.md": content_hash("OLD-INSTALLED"),
            "soul.md": content_hash("OLD-SOUL-BASE"),
        });
        std::fs::write(td.join(INSTALL_MANIFEST), manifest.to_string()).unwrap();

        let items: Vec<(&str, &str)> = PACK_ALL.iter().map(|(r, c)| (*r, *c)).collect();
        let plan = plan_install(&td, &items, false, env!("CARGO_PKG_VERSION"));
        assert!(plan.blocked.is_none());
        assert!(plan.update.iter().any(|r| r == "README.md"), "비수정 → update");
        assert!(plan.merge_new.iter().any(|r| r == "soul.md"), "user-owned+전진 → merge_new");
        assert!(plan.heal.iter().any(|r| r == "alerts-config.json"), "system 수정 → heal");
        // 실제 install 이 플랜과 같은 행동을 하는지 대조.
        install(false, None).expect("install 실패");
        let read = |rel: &str| std::fs::read_to_string(td.join(rel)).unwrap();
        assert_eq!(read("soul.md"), "USER-SOUL");
        assert!(td.join("soul.md.new").exists());
        assert!(td.join("alerts-config.json.user").exists());

        let _ = std::fs::remove_dir_all(&td);
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

        // ③ system-owned **불변**: 판독 불가여도 P0-4 강제 치유 유지(동결 = 배포 스큐 재앙의 근원).
        for rel in ["bin/javis_phoenix.py", "alerts-config.json", "README.md", "CLAUDE.md.template",
                    "directives/CEO_TEMPLATE.md"] {
            assert_eq!(decide_file_action(rel, embed, true, None, None, true),
                       Write { heal_user_copy: false }, "③system·판독불가·force → 치유 유지: {rel}");
            assert_eq!(decide_file_action(rel, embed, true, None, None, false),
                       Write { heal_user_copy: false }, "③system·판독불가·비force → 치유 유지: {rel}");
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

        // ④ 멱등: 판독 불가 파일이 남아 있어도 재실행이 상태를 흔들지 않는다.
        install_staged(true, None).unwrap();
        for rel in unreadable_user {
            assert_eq!(std::fs::read(pd.join(rel)).unwrap(), CP949, "④재실행 후에도 불변 — {rel}");
        }

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
                Some(content_hash("EMBED-V1").as_str()), false, d),
            Keep { adopt_hash: false, new_pending: false },
            "dept soul 전진에도 .new 병치 없음(결함4 구조 소멸)"
        );
        // dept × soul 유 + force: 여전히 불가침(SeedOnce 조기 반환).
        assert_eq!(
            super::decide_file_action("soul.md", embed, true, Some("DEPT-SOUL"), None, true, d),
            Keep { adopt_hash: false, new_pending: false },
            "dept soul 은 force 여도 불가침"
        );
        // dept × soul 무: 시드 설치(Write) — 승계 본문 치환은 install_into 의 seed-from-base 소관.
        assert_eq!(
            super::decide_file_action("soul.md", embed, false, None, None, false, d),
            Write { heal_user_copy: false }
        );
        // base × soul 유(수정+전진): 기존 계약 그대로 .new 병치(대조 핀 — 기존 threeway 매트릭스와 동일).
        assert_eq!(
            super::decide_file_action("soul.md", embed, true, Some("MY-SOUL"),
                Some(content_hash("EMBED-V1").as_str()), false, PackScope::Base),
            Keep { adopt_hash: false, new_pending: true },
            "base soul 은 종전대로 병치(base 레인 불변)"
        );
        // base × soul 무: 신규 생성(기존 계약).
        assert_eq!(
            super::decide_file_action("soul.md", embed, false, None, None, false, PackScope::Base),
            Write { heal_user_copy: false }
        );
        // dept 에서 soul 외 파일은 base 등급표 그대로(system 강제 치유 불변).
        assert_eq!(
            super::decide_file_action("bin/x.py", embed, true, Some("HACKED"), None, false, d),
            Write { heal_user_copy: true },
            "dept 스코프가 system 치유를 흔들면 안 된다"
        );
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
    /// 아니라 **base 팩(형제 `pack`)의 현행 soul.md 승계**이고 매니페스트 해시·pristine 이 승계본과
    /// 일관 ② vendor 전진(임베드 변경) 재설치에도 불가침 + `soul.md.new` 미생성(결함4 병치 소멸)
    /// ③ base soul 부재 시 임베드 폴백 + 원장 강등 라인 ④ 구버전이 남긴 `.new`·pending 잔재는
    /// 다음 설치가 정리(마이그레이션 계약) — 승계/강등 사실은 `.merge-audit.jsonl` 이 영속 증언.
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
        )
        .unwrap();
        let read = |p: &Path| std::fs::read_to_string(p).unwrap();
        assert_eq!(read(&dpack.join("soul.md")), base_soul, "부서 soul 은 base 헌장 승계본");
        let manifest: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&read(&dpack.join(INSTALL_MANIFEST))).unwrap();
        assert_eq!(
            manifest.get("soul.md"),
            Some(&content_hash(base_soul)),
            "매니페스트 해시는 승계본 기준(치환 content 일관 흐름)"
        );
        assert_eq!(
            read(&dpack.join(PRISTINE_DIR).join("soul.md")),
            base_soul,
            "pristine 도 승계본(3-way 조상 일관)"
        );
        let audit = read(&dpack.join(MERGE_AUDIT_FILE));
        assert!(
            audit.contains("\"action\":\"seed-from-base\""),
            "원장에 seed-from-base 라인: {audit}"
        );
        assert!(audit.contains("\"verify_result\":\"pass\""), "안전핵 포함 승계 = pass: {audit}");
        assert!(
            audit.contains(&content_hash(base_soul)),
            "after_sha256 = 승계본 해시"
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
        )
        .unwrap();
        assert_eq!(read(&dpack.join("soul.md")), base_soul, "vendor 전진에도 부서 soul 불가침");
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
        )
        .unwrap();
        assert_eq!(
            read(&dpack3.join("soul.md")),
            "TEMPLATE-SOUL-V1",
            "base 부재 시 임베드 폴백(부서가 soul 없이 뜨지 않는다)"
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
        )
        .unwrap();
        assert_eq!(read(&dpack4.join("soul.md")), "OLD-DEPT-SOUL", "기존 부서 soul 소급 교체 금지");
        assert!(!dpack4.join("soul.md.new").exists(), "구 병치본 정리(compat 계약)");
        assert!(load_merge_pending(&dpack4).get("soul.md").is_none(), "구 pending 정리");

        let _ = std::fs::remove_dir_all(&td);
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

    /// user-edit 보존: force=false 재설치가 사용자 편집 파일을 덮지 않는다(init-pack '4 preserved' 정합).
    #[test]
    fn install_staged_preserves_user_edit() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("cys-staged-pres-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        let _env = set_pack_env(&pd, base.join("claude"));

        install_staged(false, None).unwrap();
        // ★B2: user 소유 파일(soul.md 등 — 디렉티브 제외)의 편집은 보존, system 파일 편집은 강제 갱신.
        let user_target = PACK_ALL
            .iter()
            .find(|(rel, content)| is_user_owned(rel) && !rel.ends_with("_DIRECTIVE.md") && !content.starts_with("#!"))
            .map(|(rel, _)| *rel)
            .expect("user 소유 비-디렉티브 임베드 파일(soul.md 등) 존재");
        let (sys_target, sys_embed) = PACK_ALL
            .iter()
            .find(|(rel, content)| !is_user_owned(rel) && !content.starts_with("#!"))
            .map(|(rel, c)| (*rel, *c))
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
            sys_embed,
            "★B2: system 파일 편집은 임베드로 강제 갱신(스큐 동결 금지)"
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
}
