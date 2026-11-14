//! CYSJavis Pack: cys 터미널에 임베드된 멀티에이전트 운영체계 템플릿.

use std::path::{Path, PathBuf};

pub const ENV_PACK_DIR: &str = "CYS_PACK_DIR";
/// cys 전용 CLAUDE_CONFIG_DIR 오버라이드(주로 테스트 격리용). 미설정 시 pack_dir 형제(~/.cys/claude).
pub const ENV_CONFIG_DIR: &str = "CYS_CONFIG_DIR";

/// pack-update 명령의 pending/reinject 처리가 불완전하다. 이 실행의 디스크 반영 여부는 별도
/// terminal record로 판정한다(이미 최신인 팩에도 pending 실패가 가능하다). 0으로 숨기지 않고,
/// 커밋된 팩을 롤백하거나 UNKNOWN 입력을 맹목 재전송하지 않는다.
pub const EXIT_REINJECT_DEGRADED: i32 = 3;
/// run_pack_update가 reinject 집계를 stdout에 구조화 출력할 때 쓰는 줄 접두사. 호출자(Tauri
/// 브리지)가 failed/deferred를 정확히 파싱하도록 사람용 메시지와 별개의 안정 토큰으로 둔다.
pub const REINJECT_RESULT_PREFIX: &str = "PACK_UPDATE_RESULT";
/// U9 no-op 결과 줄 접두사. 순수 no-op의 종료코드 0과 confirmed 판정은 유지한다.
/// 아래 legacy helper는 gate/remote/disk/disk_parse를 출력한다. schema 2의 종결 레코드는
/// degraded/unknown 명령에도 이 접두사를 쓰되 구 소비자에게 확인된 최신 증거를 노출하지
/// 않는다(pack_update_result). 디스크 판독 실패를 최신으로 해석하면 안 된다.
pub const PACK_UPDATE_OUTCOME_PREFIX: &str = "PACK_UPDATE_OUTCOME";

/// 토큰 값 정규화 — 공백·비ASCII 가 섞이면 줄 파싱이 깨지므로 안전 문자만 그대로, 빈 값은 `-`, 그 외 `?`.
fn outcome_token_value(v: &str) -> String {
    let v = v.trim();
    if v.is_empty() {
        "-".to_string()
    } else if v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_')) {
        v.to_string()
    } else {
        "?".to_string()
    }
}

/// PACK_UPDATE_OUTCOME 한 줄을 만든다(순수). `disk_raw` = `.pack-version` 원문(읽기 실패면 None).
pub fn format_pack_update_outcome(gate: &str, remote: &str, disk_raw: Option<&str>) -> String {
    let disk = disk_raw.map(str::trim).unwrap_or("");
    let disk_parse = if parse_semver(disk).is_some() { "ok" } else { "fail" };
    format!(
        "{PACK_UPDATE_OUTCOME_PREFIX} gate={} remote={} disk={} disk_parse={disk_parse}",
        outcome_token_value(gate),
        outcome_token_value(remote),
        outcome_token_value(disk),
    )
}

/// `cys pack-update` UpToDate 분기가 찍는 결과 줄 — 디스크 `.pack-version` 을 **지금** 읽어 싣는다
/// (판정에 쓴 값과 같은 파일 · pack_update_from_dir 와 같은 pack_dir()). 읽기만 하고 쓰지 않는다.
pub fn pack_update_uptodate_line(remote: &str) -> String {
    let disk = std::fs::read_to_string(pack_dir().join(PACK_VERSION_FILE)).ok();
    format_pack_update_outcome("up-to-date", remote, disk.as_deref())
}

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
///
/// ★(0.14.42 · perf R3-6) **프로세스당 1회** 계산하고 재사용한다(`embedded_pack_hash_ref`). 입력 PACK_ALL 은
/// 컴파일 타임 상수라 값이 프로세스 수명 동안 바뀔 수 없다(결과 불변). 종전에는 org.status 가 불릴
/// 때마다 임베드 팩 전체를 소프트웨어 SHA-256 으로 다시 해시했다(aarch64 는 sha2 `asm` 피처 없이
/// 가속이 꺼진다). 실측 수치와 측정 시각·기준 sha 는 이 변경의 커밋 메시지에 있다 — 팩 크기가
/// 바뀌면 낡는 수치를 코드에 두지 않는다. 새 프로세스마다 한 번은 계산하므로 `cys phoenix-identity`
/// 같은 1회성 CLI 경로의 비용은 그대로다. 핀: `embedded_pack_hash_is_memoized_and_unchanged`.
pub fn embedded_pack_hash() -> String {
    embedded_pack_hash_ref().to_owned()
}

/// 메모 본체 — 첫 호출 때만 계산해 프로세스 수명 동안 보관한 값의 참조. 기동 때 미리 계산하지
/// 않는다(부트 체인에 비용을 얹지 않는다). 동시에 여러 스레드가 첫 호출을 하면 한 스레드만
/// 계산하고 나머지는 그 결과를 기다린다(`OnceLock`).
///
/// ★(0.14.42 통합 검증) `PACK_ALL` 은 `const` 라 참조하는 코드 생성 단위마다 팩 전체가 따로 실릴 수 있다.
/// 해시 루프를 `get_or_init` 의 제네릭 클로저 안에 두면 release 바이너리에 팩 사본이 하나 더 생긴다(실측 +≈11MB ·
/// 핀 `embedded_pack_hash_memo_closure_does_not_touch_pack_all`). 그래서 참조는 이 비제네릭 본체에서 뜨고 클로저는
/// 슬라이스만 받는다 — 값·메모 동작은 그대로다.
fn embedded_pack_hash_ref() -> &'static str {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let items: &'static [(&'static str, &'static str)] = PACK_ALL;
    HASH.get_or_init(|| pack_items_sha256(items)).as_str()
}

/// 임베드 팩 해시 계산부(rel\0content\0 스트리밍 SHA-256) — `embedded_pack_hash_ref` 의 메모 안에서만 부른다.
fn pack_items_sha256(items: &[(&str, &str)]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for (rel, content) in items {
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

/// 레인 가드 조기 종료 표식(`<pack>/state/lane-guard-tripped`)의 상대 경로.
/// 정의처 동형: `cysjavis-pack/bin/javis_preflight.py` LANE_GUARD_TRIPPED_REL.
pub const LANE_GUARD_TRIPPED_REL: &str = "state/lane-guard-tripped";
/// 표식 신선도 창(초). 정의처 동형: 같은 파일 LANE_GUARD_RECENT_S.
pub const LANE_GUARD_RECENT_SECS: u64 = 24 * 3600;

/// 이 레인(팩)의 훅이 **무발화 조기 종료**한 적이 있는가.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneGuardTrip {
    /// 표식의 `reason=` (absent·unreadable·no-redirect-line·already-redirected).
    pub reason: String,
    /// 표식의 `script=` (session-start.sh 등). 결측이면 빈 문자열.
    pub script: String,
    /// 표식이 있지만 판독 불가 — 결측은 값이 아니므로 **참(보수)**으로 접는다.
    pub unreadable: bool,
}

/// 파이썬 `lane_guard_tripped`의 `(recent, info)`를 같은 3분기로 판정한다.
/// 파일의 증명된 부재와 mtime 기준 24시간을 넘은 표식만 `None`이다.
/// 판독 불가(권한·비정규 파일·mtime 없음)는 `Some`의 `unreadable: true`로 막는다.
/// 본문은 Python `_read_text_tolerant`처럼 UTF-8 손상 바이트를 대체해 읽는다.
pub fn lane_guard_tripped(pack_dir: &Path) -> Option<LaneGuardTrip> {
    lane_guard_tripped_at(pack_dir, std::time::SystemTime::now())
}

/// 현재 시각만 주입해 신선도 경계와 시계 역전을 결정론적으로 검증한다.
fn lane_guard_tripped_at(pack_dir: &Path, now: std::time::SystemTime) -> Option<LaneGuardTrip> {
    use std::io::Read;

    let path = pack_dir.join(LANE_GUARD_TRIPPED_REL);
    let unreadable = || Some(LaneGuardTrip {
        reason: String::new(),
        script: String::new(),
        unreadable: true,
    });
    // Python `_lexists_strict`와 동형: 끊어진 심링크도 존재하며, 조회 실패는 부재가 아니다.
    if let Err(error) = std::fs::symlink_metadata(&path) {
        let absent = matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        ) || path.as_os_str().as_encoded_bytes().contains(&0);
        #[cfg(unix)]
        let absent = absent
            || error.raw_os_error().is_some_and(|code| {
                [libc::ENOENT, libc::ENOTDIR, libc::ELOOP, libc::ENAMETOOLONG].contains(&code)
            });
        return if absent { None } else { unreadable() };
    }

    // Python `_open_unblocking_ro`와 같은 O_NONBLOCK + 열린 fd 정규파일 확인:
    // writer 없는 FIFO와 lstat/open 사이 FIFO 교체도 cycle-agent를 멈추지 못한다.
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = match options.open(&path) {
        Ok(file) => file,
        Err(_) => return unreadable(),
    };
    match file.metadata() {
        Ok(metadata) if metadata.is_file() => {}
        _ => return unreadable(),
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return unreadable();
    }
    let mtime = match std::fs::metadata(&path).and_then(|metadata| metadata.modified()) {
        Ok(mtime) => mtime,
        Err(_) => return unreadable(),
    };
    let age = now.duration_since(mtime).unwrap_or_default();
    if age > std::time::Duration::from_secs(LANE_GUARD_RECENT_SECS) {
        return None;
    }

    let mut trip = LaneGuardTrip {
        reason: String::new(),
        script: String::new(),
        unreadable: false,
    };
    for line in String::from_utf8_lossy(&bytes).lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        match key {
            "reason" => trip.reason = value.to_owned(),
            "script" => trip.script = value.to_owned(),
            // Python의 나머지 인정 키도 읽되 현재 공개 결과에는 두 필드만 노출한다.
            "hook_root" | "lane_root" | "surface" | "ts" => {}
            _ => {}
        }
    }
    Some(trip)
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

/// ★U10(0.14.41) **관측 전용** hook 명령 비교 정규화(순수).
///
/// ★집행 경로(`merge_desired_hooks`·`verify_desired_hooks_registered`)에는 **절대 쓰지 않는다** —
/// 그쪽은 바이트 동등이 계약이다(느슨한 매칭이 '이미 있음'과 '없음'을 뒤섞으면 재등록이 중복 append
/// 폭주 방향으로 간다: 위 `hook_registered_in` 주석). 이 함수의 소비자는 기동 경고
/// (`cys launch-agent` 의 각성 훅 미등록 판정) 하나다.
///
/// 규칙(양쪽 문자열에 똑같이 적용 — 결정론):
///  ① 따옴표(`"`·`'`) 제거 ② `\` → `/` ③ 공백열 1칸으로 · 앞뒤 공백 제거
///  ④(windows 규칙만) 토큰 머리의 `//?/`(= `\\?\` 확장 경로 접두) 제거 · 토큰 머리의 MSYS 드라이브
///    `/c/…` → `c:/…` · 전체 소문자(NTFS 경로·`bash` 모두 대소문자 무시).
/// 윈도우 표기가 섞이는 근거: 쓰는 쪽(`hook_command_for` = `C:/…` · preflight `_cys_hook_cmd`)과 부서 팩 값이
/// 오는 쪽(cys-dept `$HOME` = Git Bash `/c/…` → MSYS 변환)이 다를 수 있다 — 저장소는 이미 `/c/rest` ↔ `C:/rest`
/// 를 같은 등록으로 인정한다(`hooks/_lib.sh cys_lane_registered`). 유닉스 규칙은 ①~③만(대소문자 구분 유지).
pub fn normalize_hook_command_for_compare(cmd: &str, windows_rules: bool) -> String {
    let unquoted: String = cmd
        .chars()
        .filter(|c| *c != '"' && *c != '\'')
        .map(|c| if c == '\\' { '/' } else { c })
        .collect();
    let tokens = unquoted.split_whitespace().map(|tok| {
        if !windows_rules {
            return tok.to_string();
        }
        let tok = tok.strip_prefix("//?/").unwrap_or(tok);
        let b = tok.as_bytes();
        let t = if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b'/' {
            format!("{}:{}", &tok[1..2], &tok[2..])
        } else {
            tok.to_string()
        };
        t.to_lowercase()
    });
    tokens.collect::<Vec<_>>().join(" ")
}

/// settings.json 의 특정 이벤트에 desired 명령이 **표기 정규화 후** 등록돼 있는가(관측 전용·순수).
/// command 축 단독(timeout 미고려) — [`hook_registered_in`] 과 같은 축이고 비교만 정규화한다.
pub fn hook_registered_normalized_in(
    root: &serde_json::Value,
    event: &str,
    desired: &str,
    windows_rules: bool,
) -> bool {
    let want = normalize_hook_command_for_compare(desired, windows_rules);
    root.get("hooks")
        .and_then(|h| h.get(event))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().any(|m| {
                m.get("hooks")
                    .and_then(|v| v.as_array())
                    .map(|hs| {
                        hs.iter().any(|h| {
                            h.get("command")
                                .and_then(|c| c.as_str())
                                .map(|c| normalize_hook_command_for_compare(c, windows_rules) == want)
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// ★U10(0.14.41) 각성 훅 누락 목록 — **레인 팩 하나**를 기대값으로 삼는다(관측 전용·순수).
///
/// 합집합(본부 팩 ∪ 레인 팩)을 쓰지 않는 이유: 부서 계정 settings 에 본부 팩 훅이 교차 등록된 경우를
/// '등록됨' 으로 통과시키면 레인 가드 위임 실패(훅이 조용히 조기 종료하는 형상)가 가려진다(U10 반박 DD1).
pub fn awakening_hooks_missing_in(
    root: &serde_json::Value,
    lane_pack: &Path,
    windows_rules: bool,
) -> Vec<&'static str> {
    AWAKENING_HOOKS
        .iter()
        .filter(|h| {
            !hook_registered_normalized_in(
                root,
                h.event,
                &hook_command_for(lane_pack, h.script),
                windows_rules,
            )
        })
        .map(|h| h.script)
        .collect()
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

/// `<settings>.cys-lock` 파일락 획득 — settings.json RMW 직렬화의 **단일 소유자**(G16 · H-CONC-3).
///
/// python preflight 는 `javis_lock.FileLock(<settings>.cys-lock)` 으로 RMW 를 직렬화한다
/// (`javis_preflight.py::_settings_rmw`). Rust 쪽 두 writer([`merge_desired_hooks`] ·
/// `factory_reset::strip_settings_matching`)가 락 없이 RMW 하면 preflight C28 재등록과 교차해
/// **lost-update**(한쪽 쓰기 증발)가 난다 — 원자 쓰기는 *반쪽 파일*만 막고 read-modify-write 의
/// 경합은 막지 못한다. 그래서 **같은 이름의 같은 락 파일**로 직렬화한다.
///  · unix: `flock(LOCK_EX)` **블로킹**(보유 창 = 파일 1개 RMW · 수 ms) · 락 파일 열기 실패는
///    `None`(직렬화만 포기하고 작업은 진행 — 락 실패가 치유를 막으면 잔존 훅이 영구화된다).
///  · windows: **미획득(None) — 감수 범위 명기**. python 백엔드가 msvcrt 바이트락이라 flock 과
///    상호 배제가 성립하지 않는다(이종 락). 파손은 원자 교체가 차단하고 최악은 lost-update 로
///    다음 preflight C28·부트 시드가 재수렴한다. 승격 조건은 `LockFileEx` 동형 배선이다.
///
/// ★반환 핸들이 **살아 있는 동안만** 락이 선다(drop = 해제). 호출부는 RMW 가 끝날 때까지 이름
/// 있는 바인딩으로 붙들어라 — `let _ = ...` 는 즉시 drop 이라 락이 아예 서지 않는다.
///
/// ★사본 금지: 종전엔 이 본문이 `src/bin/cys.rs` 에만 있었고 다른 두 writer 는 락이 없었다.
/// 소유자를 여기 하나로 모아 세 writer 가 **같은 락 파일**을 잡는다.
pub fn acquire_settings_lock(settings: &Path) -> Option<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        let lock_path = PathBuf::from(format!("{}.cys-lock", settings.display()));
        // ★T2i(RULINGS EC5): 막히지 않게 연다 — 그 자리가 FIFO(읽는 쪽 없음)면 열기가 영원히 멈췄다. 일반 파일만 잠근다(그
        //   밖 = 잠금 없음 · 종전 열기 실패와 같은 갈래).
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&lock_path)
            .ok()?;
        if !f.metadata().map(|m| m.is_file()).unwrap_or(false) {
            return None;
        }
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return None;
        }
        Some(f)
    }
    #[cfg(not(unix))]
    {
        let _ = settings;
        None
    }
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
/// ★T2i(RULINGS EC5): 설정 파일(`settings.json`)의 시작 경로 판독 — 막히지 않게 열고 일반 파일 · 상한 이하만(링크는 일반
/// 파일을 가리킬 때만 따른다 — 지금 동작). 부재 = `Ok(None)`(빈 설정으로 시작하는 호출부 규칙 그대로), 판독 불가 · UTF-8
/// 아님 = `Err`(종전 `read_to_string` 의 그 밖의 오류와 같은 갈래 — 덮어쓰지 않는다).
fn settings_read_bounded(settings_path: &Path) -> Result<Option<String>, String> {
    match read_start_live(settings_path) {
        StartRead::Missing => Ok(None),
        StartRead::Unreadable(e) => Err(format!("settings read error: {e}")),
        StartRead::Bytes(b) => {
            String::from_utf8(b).map(Some).map_err(|e| format!("settings read error: not UTF-8 ({e})"))
        }
    }
}

/// ★T2i(RULINGS EC5): `<settings>.bak-cys` 를 **이미 읽은 바이트**로 쓴다(종전 `std::fs::copy` 는 원본을 다시 열었다 — 그
/// 사이에 FIFO 로 바뀌면 막혔다). 원본의 권한 비트를 그대로(복사가 하던 일).
fn settings_backup_from(settings_path: &Path, body: &str) -> Result<(), String> {
    let backup = PathBuf::from(format!("{}.bak-cys", settings_path.display()));
    write_atomic_mode(&backup, body.as_bytes(), existing_file_mode(settings_path)).map_err(|e| e.to_string())
}

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
    // ★H-CONC-3: read → modify → write 전 구간을 공용 락으로 직렬화한다(python preflight 와
    //   **같은** `<settings>.cys-lock`). 원자 쓰기는 반쪽 파일만 막는다 — 두 writer 가 같은
    //   스냅샷을 읽어 각자 append 하면 나중 쓰기가 앞 append 를 통째로 지운다(lost update).
    //   바인딩 이름을 `_lock` 으로 두는 것이 계약이다(`_` 하나면 즉시 drop = 락 미성립).
    let _lock = acquire_settings_lock(settings_path);
    let existed = settings_path.exists();
    // ★T2i(RULINGS EC5): 시작 경로의 단일 판독기(막히지 않음 · 일반 파일 · 상한) — 백업도 이 바이트로 쓴다(다시 열지 않는다).
    let raw = settings_read_bounded(settings_path)?;
    let mut root: serde_json::Value = match raw.as_deref() {
        Some(s) if s.trim().is_empty() => serde_json::json!({}),
        Some(s) => serde_json::from_str(s).map_err(|e| format!("settings parse error: {e}"))?,
        // 파일 없음일 때만 빈 설정으로 시작 — 권한 등 다른 읽기 에러를 무시하면
        // 기존 settings.json이 hooks만 남은 JSON으로 대체될 수 있다.
        None => serde_json::json!({}),
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
    if let (true, Some(body)) = (existed, raw.as_deref()) {
        settings_backup_from(settings_path, body)?;
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
    // ★IG-23(2026-09-05): 이 함수도 **read-modify-write** 다 — 읽고 timeout 만 올려 다시 쓴다.
    //   H-CONC-3 수리에서 `merge_desired_hooks`·`strip_settings_matching` 둘만 잠그고 **여기를
    //   빠뜨렸다**(master 지시로 Rust writer 를 전수 색출하다 찾았다). 락이 없으면 preflight
    //   C28 재등록·병합기와 교차해 lost-update 가 난다 — 원자 쓰기는 반쪽 파일만 막고 RMW
    //   경합은 막지 못한다는, 다른 두 writer 와 똑같은 이유다.
    //   존재 검사 뒤에 잡는다: 파일이 없으면 만질 것도 없고, 그때 락 파일을 만들면 '없으면
    //   만들지 않는다' 경계(위 한 줄)를 잔재로 뚫는다.
    let _lock = acquire_settings_lock(settings_path);
    // ★T2i(RULINGS EC5): 시작 경로의 단일 판독기 — 백업도 이 바이트로 쓴다.
    let raw = settings_read_bounded(settings_path)?;
    let mut root: serde_json::Value = match raw.as_deref() {
        // 빈 파일 = 등록 0 — 만질 것이 없다(빈 객체로 덮어쓰지 않는다).
        Some(s) if s.trim().is_empty() => return Ok(vec![]),
        Some(s) => serde_json::from_str(s).map_err(|e| format!("settings parse error: {e}"))?,
        None => return Ok(vec![]),
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
    settings_backup_from(settings_path, raw.as_deref().unwrap_or_default())?;
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
    // ★T2i(RULINGS EC5): 시작 경로의 단일 판독기(판독 불가 = 빈 설정 = 전부 미등록으로 보고 — 지금과 같은 방향).
    let root: serde_json::Value = read_start_live(settings_path)
        .text()
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

/// ★0.14.42 agy(Antigravity CLI) 상태줄 자동 연결 — 설치·팩 병합 때 한 번 조정한다(계약 전문: `crate::agy_statusline`).
///
/// 개인 프로필 훅 병합([`merge_awakening_hooks_into_personal_profiles`])과 **같은 계층·같은 게이트**다:
///  · 테스트 빌드 무동작 · `CYS_NO_PERSONAL_HOOK_MERGE=1`(E2E 하네스의 개인 설정 무접촉 약속) 무동작
///  · **base 팩 전용** — 부서·임시·테스트 팩(`CYS_PACK_DIR` 지정)에서 도는 설치는 `~/.gemini` 를 건드리지 않는다
///    (CONTRIBUTING: 팩 변수만 격리하고 HOME 을 두고 온 E2E 가 라이브 프로필에 죽은 훅을 쌓은 사고의 같은 방어).
/// 노브가 꺼져 있으면(`CYS_AGY_STATUSLINE=0` · `~/.cys/agy-statusline-off`) cys 가 넣은(표지 달린) 연결만 뺀다.
/// best-effort — 무엇이 실패해도 팩 설치는 유효하고 설정 파일은 그대로다(실패 방향 = 쓰지 않음 · 로그 1줄).
pub fn reconcile_agy_statusline_at_install() {
    // ★(0.14.45 · B1 · 치명위험 ④) cysd 부트는 이 조정을 **소켓 바인드 뒤**로 미룬다 — 윈도우 실연 검사(≤5.2초)가 바인드 앞에서
    //   돌면 CLI 자동기동의 4초 폴링이 데몬을 못 보고 실패했다. 여기서는 '미뤄 둔 조정 있음' 표식만 세우고 돌아간다
    //   (`run_deferred_agy_statusline_reconcile` 이 바인드 뒤 별도 스레드에서 이 함수를 다시 부른다). 그 밖의 호출부
    //   (`cys init-pack` · `cys doctor --fix` · GUI 인앱 업데이트)는 표식을 세우지 않으므로 종전처럼 동기다.
    if AGY_PROBE_DEFERRED.load(std::sync::atomic::Ordering::Acquire) {
        AGY_RECONCILE_PENDING.store(true, std::sync::atomic::Ordering::Release);
        return;
    }
    reconcile_agy_statusline_now();
}

/// (B1) `cysd` 가 **소켓 바인드 전에** 세우는 표식 — 이 프로세스의 설치 경로는 agy 상태줄 조정(윈도우 실연 검사 포함)을 하지 않고
/// 미뤄 둔다. 다른 프로세스(init-pack·doctor)에는 아무 영향이 없다(프로세스 전역 원자 플래그).
static AGY_PROBE_DEFERRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// (B1) 미뤄 둔 조정이 있다 — 바인드 뒤 [`run_deferred_agy_statusline_reconcile`] 이 한 번 소비한다.
static AGY_RECONCILE_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// (B1) cysd 부트가 팩 설치 **앞에서** 부른다 — 이 프로세스의 설치 경로 agy 조정을 바인드 뒤로 미룬다.
pub fn defer_agy_statusline_probe() {
    AGY_PROBE_DEFERRED.store(true, std::sync::atomic::Ordering::Release);
}

/// (B1) cysd 가 소켓을 바인드한 직후 **동기로** 부른다 — 미루기 표식을 내려 그 뒤의 설치 경로(런타임 pack-update 등)가 종전처럼
/// 동기가 되게 한다(소비 스레드 생성이 실패해도 표식이 남아 뒤 설치가 영영 미뤄지는 경로 차단 — 리뷰 지적 #8).
pub fn end_agy_statusline_deferral() {
    AGY_PROBE_DEFERRED.store(false, std::sync::atomic::Ordering::Release);
}

/// (B1) cysd 가 소켓을 바인드한 뒤(별도 스레드 · 락 0) 부른다 — 조정을 **매 부트** 한 번 한다(미뤄 둔 설치가 있었든 없었든).
/// 설치 뒤 소비 전에 데몬이 죽으면 다음 부트는 팩이 최신이라 설치를 건너뛰므로, 소비를 설치에 묶지 않는다. 이미 연결된 기계·
/// 전에 연결했던 기계·같은 버전 실패 기록이 있는 기계는 파일 stat 몇 번으로 끝난다(실연 검사 없음). 반환 = 미뤄 둔 설치가 있었는가.
pub fn run_deferred_agy_statusline_reconcile() -> bool {
    end_agy_statusline_deferral();
    let pending = AGY_RECONCILE_PENDING.swap(false, std::sync::atomic::Ordering::AcqRel);
    reconcile_agy_statusline_now();
    pending
}

/// [`reconcile_agy_statusline_at_install`] 의 본체(미루기 표식 무관).
fn reconcile_agy_statusline_now() {
    use crate::agy_statusline as agy;
    if cfg!(test) {
        return; // 테스트 빌드는 실 HOME 을 절대 만지지 않는다
    }
    if std::env::var("CYS_NO_PERSONAL_HOOK_MERGE").map(|v| v == "1").unwrap_or(false) {
        return;
    }
    let pack = pack_dir();
    if pack != home_default_pack_dir() {
        return;
    }
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let settings = agy::settings_path_under(&home);
    let record = pack.join(agy::RECORD_REL);
    let off_file = home.join(".cys").join(agy::OFF_FILE);
    let version = env!("CARGO_PKG_VERSION");
    let outcome = if agy::knob_off(crate::env_compat(agy::ENV_KNOB).as_deref(), off_file.exists()) {
        agy::unlink(&settings, Some(&record), agy::Backup::Beside)
    } else {
        // ★(B1) 같은 버전에서 이미 실연 검사가 실패했으면 다시 검사하지 않는다(부트마다 5초 검사 반복 금지) — 버전이 바뀌거나
        //   사람이 `cys doctor --fix` 를 부르면(기록 무시 · 결과로 덮는다) 다시 검사한다. 유닉스에는 검사가 없어 기록도 없다.
        // ★(성찰 m2) 같은 버전이라도 24시간이 지나면 다시 검사한다(기계 환경이 바뀌었을 수 있다) — 시각 없는 옛 기록도 다시 검사.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if let Some(reason) = agy::probe_failure_deferring(&pack, version, now) {
            eprintln!(
                "[pack] agy 상태줄: 이 버전({version})의 실연 검사 실패 기록(24시간 이내)이 있어 다시 검사하지 않았습니다({reason}) — \
                 24시간 뒤 자동 재검사 · 지금 다시 시도: `cys doctor --fix`"
            );
            return;
        }
        let o = agy::ensure_linked(
            &agy::Ctx {
                settings: &settings,
                pack_dir: &pack,
                record: &record,
                windows: cfg!(windows),
                home: Some(&home),
                // 윈도우는 쓰기 직전 실연 검사(콘솔 창 없음 · 5초 상한) — 실패는 결과 한 줄일 뿐 설치는 계속된다.
                probe: Some(&agy::live_probe),
            },
            false,
        );
        match &o {
            agy::Outcome::WindowsProbeFailed { reason, .. } => agy::record_probe_failure(&pack, version, reason),
            agy::Outcome::Linked { .. } => agy::clear_probe_failure(&pack),
            _ => {}
        }
        o
    };
    if let Some(line) = agy::describe(&outcome, &settings) {
        eprintln!("[pack] agy 상태줄: {line}");
    }
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
    // ★0.14.42 agy 상태줄 자동 연결(오너 승인 2026-09-24) — 첫기동 시드처럼 `install_hooks` 조기 return **위**다.
    //   GUI 인앱 업데이트는 항상 `--no-install-hook` 으로 내려오므로 아래에 두면 업데이트로 올라온 사용자에게 영영
    //   닿지 않는다(U-19 와 같은 도달성). 이것은 Claude 훅이 아니고, 그 플래그가 막으려는 두 가지(정상 `.bak-cys`
    //   클로버 · 활성 프로필 재직렬화)를 하지 않는다: 칸이 비었을 때 **한 번** 외과 수술로 한 칸만 쓰고, 그 뒤로는
    //   무동작이다(연결 기록). 자기 노브(`CYS_AGY_STATUSLINE=0` · `~/.cys/agy-statusline-off`)가 따로 있다.
    reconcile_agy_statusline_at_install();
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
    // ★T2i(RULINGS EC5): 시작 경로의 단일 판독기(막히지 않음 · 일반 파일 · 상한) — 백업은 이 바이트로 쓴다(다시 열지 않는다).
    let mut original: Vec<u8> = Vec::new();
    let existing = if existed {
        match read_start_live(&path) {
            StartRead::Bytes(b) => {
                let parsed = std::str::from_utf8(&b)
                    .map_err(|e| e.to_string())
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).map_err(|e| e.to_string()));
                match parsed {
                    Ok(v) => {
                        original = b;
                        v
                    }
                    Err(e) => {
                        return SeedOutcome::Refused(format!("{} 파싱 불가 — 보존: {e}", path.display()))
                    }
                }
            }
            StartRead::Missing => {
                return SeedOutcome::Refused(format!("{} 읽기 불가 — 보존: 사라짐", path.display()))
            }
            StartRead::Unreadable(e) => {
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
        let mode = existing_file_mode(&path).or(Some(0o600));
        if let Err(e) = write_atomic_mode(&cfg.join(FIRST_RUN_SEED_BACKUP), &original, mode) {
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
    let verified = read_start_live(&path)
        .text()
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
    // ★T2h(RULINGS CW3): 시작 경로 판독 — 링크 비추종 · 막히지 않음 · 일반 파일 · 상한.
    let s = match read_start_meta(&path) {
        StartRead::Bytes(b) => match String::from_utf8(b) {
            Ok(s) => s,
            Err(e) => return PackStateRead::Corrupt(format!("읽기 실패: {e}")),
        },
        StartRead::Missing => return PackStateRead::Absent,
        StartRead::Unreadable(e) => return PackStateRead::Corrupt(format!("읽기 실패: {e}")),
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
    let Some(s) = read_start_meta(&dir.join(INSTALL_MANIFEST)).text() else {
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
            // ★T2i(RULINGS EC5): 시작 경로의 단일 판독기(막히지 않음 · 일반 파일 · 상한). 판독 불가(FIFO · 장치 · 폴더 · 상한
            //   초과)면 아래 증거 검사는 그대로 하되 자가치유(상태 쓰기)는 하지 않는다 — 모르는 값으로 기록을 고치지 않는다.
            let (disk_v, unreadable) = match read_start_meta(&dir.join(PACK_VERSION_FILE)) {
                StartRead::Missing => (String::new(), None),
                StartRead::Bytes(b) => (String::from_utf8(b).map(|s| s.trim().to_string()).unwrap_or_default(), None),
                StartRead::Unreadable(e) => (String::new(), Some(e)),
            };
            if unreadable.is_none() && st.base_version == disk_v {
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
            if let Some(e) = unreadable {
                eprintln!("[init-pack] ⚠ {PACK_VERSION_FILE} 판독 불가({e}) — state 자가치유 생략(다음 기동 재시도)");
                return None;
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

// ── F10 (0.14.50): hooks/ 드리프트 보고 — 순수 분류기 + 읽기 전용 수집 ─────────────────────────────
// 전파 장치는 이미 있다(소유권 등급 · kept-drift 제자리 보존 · 3-way 병합 · `.user` 사본). 이 블록은 아무것도 쓰지 않고
// 그 상태를 **보고**만 한다: 디스크 · 설치 매니페스트 · vendor(= 실행 중인 바이너리에 임베드된 팩) 세 해시의 관계.

/// `hooks/` 아래 파일 하나의 상태(일곱 가지). 판정 순서가 규칙이다: 판독 불가는 vendor 비교보다 먼저 정해진다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookState {
    /// 디스크 = vendor.
    Same,
    /// 사용자가 고쳤고 vendor 는 매니페스트 이후 전진하지 않았다(kept-drift — 다음 설치에서도 제자리 보존).
    Drift,
    /// 고치지 않았고 vendor 만 전진했다(다음 설치가 갱신한다).
    Behind,
    /// 고쳤고 vendor 도 전진했다(다음 설치가 병합하거나 `.user` 사본을 남기고 교체한다).
    DriftBehind,
    /// 매니페스트(또는 vendor)에 있는데 디스크에 없다.
    Missing,
    /// 디스크에 있는데 텍스트로 읽을 수 없다(비 UTF-8 · 권한).
    Unreadable,
    /// 디스크에만 있고 vendor 가 모른다(자작 파일).
    Extra,
}

impl HookState {
    pub fn as_str(self) -> &'static str {
        match self {
            HookState::Same => "same",
            HookState::Drift => "drift",
            HookState::Behind => "behind",
            HookState::DriftBehind => "drift+behind",
            HookState::Missing => "missing",
            HookState::Unreadable => "unreadable",
            HookState::Extra => "extra",
        }
    }
    /// WARN 으로 세는 상태 — `same` · `behind`(설치 대기) · `extra`(자작)는 세지 않는다.
    pub fn counted(self) -> bool {
        matches!(
            self,
            HookState::Drift | HookState::DriftBehind | HookState::Missing | HookState::Unreadable
        )
    }
}

/// 디스크 쪽 관측.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskRead {
    Missing,
    Unreadable,
    Hash(String),
}

/// 순수 분류기(F10 · 시뮬레이션 `drift_report.py` 의 규칙). `manifest`/`vendor` 는 해당 파일의 해시 문자열(없으면 None).
pub fn classify_hook(disk: &DiskRead, manifest: Option<&str>, vendor: Option<&str>) -> HookState {
    let disk_hash = match disk {
        DiskRead::Missing => return HookState::Missing,
        DiskRead::Unreadable => return HookState::Unreadable,
        DiskRead::Hash(h) => h.as_str(),
    };
    let Some(vendor) = vendor else {
        return HookState::Extra;
    };
    if disk_hash == vendor {
        return HookState::Same;
    }
    let edited = manifest.map_or(true, |m| disk_hash != m);
    let advanced = manifest.is_some_and(|m| vendor != m);
    match (edited, advanced) {
        (true, true) => HookState::DriftBehind,
        (true, false) => HookState::Drift,
        (false, _) => HookState::Behind,
    }
}

/// 보고 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRow {
    pub rel: String,
    pub state: HookState,
    pub disk: Option<String>,
    pub manifest: Option<String>,
    pub vendor: Option<String>,
}

/// 매니페스트 키 정규화(윈도우 역슬래시 키도 같은 행으로 — F10 시험 6).
pub fn normalize_manifest_key(key: &str) -> String {
    key.replace('\\', "/")
}

/// 세 해시 중 앞 8자(짧은 표기).
pub fn short_hash(h: &str) -> &str {
    h.get(..8).unwrap_or(h)
}

/// vendor 쪽: 이 바이너리에 임베드된 팩의 `hooks/` 파일들의 해시.
pub fn vendor_hook_hashes() -> std::collections::BTreeMap<String, String> {
    PACK_ALL
        .iter()
        .filter(|(rel, _)| rel.starts_with("hooks/"))
        .map(|(rel, body)| ((*rel).to_string(), content_hash(body)))
        .collect()
}

/// F10 M2(리뷰 1회차): hooks/ 보고가 읽는 파일 크기 상한 · 순회 깊이 · 항목 수 상한 — 병적인 폴더가 보고를 늘리거나 멈추게 하지 못한다.
pub const HOOKS_READ_CAP: u64 = 8 * 1024 * 1024;
const HOOKS_WALK_MAX_DEPTH: usize = 8;
const HOOKS_WALK_MAX_ENTRIES: usize = 4096;

/// 팩 파일 하나를 **블로킹 없이** 텍스트로 읽는다: unix 에서는 `O_NONBLOCK` 로 열어(쓰는 쪽이 없는 FIFO 도 즉시 열린다) 핸들의 stat 이
/// 일반 파일이고 상한 이하일 때만 읽는다. 일반 파일이 아닌 것(FIFO · 소켓 · 장치) · 상한 초과 · 심링크 순환 · 읽기 오류는 `Err`
/// (호출부는 '판독 불가/미지원'으로 보고한다). 심링크는 일반 파일을 가리킬 때만 따라간다. 부재는 `NotFound` 그대로.
pub fn read_pack_text_bounded(p: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NONBLOCK);
    }
    let f = opts.open(p)?;
    let m = f.metadata()?;
    if !m.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "비정규 파일(FIFO·소켓·장치 등)은 읽지 않는다"));
    }
    if m.len() > HOOKS_READ_CAP {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "크기 상한 초과"));
    }
    read_capped_utf8(f, HOOKS_READ_CAP)
}

/// ★T2h(RULINGS CW3 · 검토자 B3): 데몬 시작 경로(설치·업데이트 판정 — 바인드 전)가 읽는 파일 하나의 판독 결과. 부재 ·
/// 판독 불가(사유) · 바이트. 판독 불가는 호출부의 **보수적 기본값**이 된다(시작을 막지 않는다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartRead {
    Missing,
    Unreadable(String),
    Bytes(Vec<u8>),
}

impl StartRead {
    /// 바이트(부재·판독 불가 = None).
    pub fn bytes(self) -> Option<Vec<u8>> {
        match self {
            StartRead::Bytes(b) => Some(b),
            _ => None,
        }
    }
    /// UTF-8 텍스트(부재·판독 불가·UTF-8 아님 = None).
    pub fn text(self) -> Option<String> {
        self.bytes().and_then(|b| String::from_utf8(b).ok())
    }
}

/// ★T2h(RULINGS CW3): 장부·메타 파일(`.merge-audit.jsonl` · `.merge-pending.json` · 설치 기록 · `.pack-version` ·
/// `.pack-state.json` · `.pristine/<rel>` · `<rel>.user` · `<rel>.new` · `<rel>.base` · 스냅샷 `meta.json`) 하나의 상한.
pub const START_META_CAP: u64 = 64 * 1024 * 1024;
/// ★T2h(RULINGS CW3): 살아 있는 팩 파일 하나의 상한(내장 팩의 가장 큰 파일은 약 1.3MB).
pub const START_LIVE_CAP: u64 = 64 * 1024 * 1024;

/// ★T2h(RULINGS CW3): 장부·메타 판독 — 마지막 성분의 링크를 **따르지 않고**(unix `O_NOFOLLOW` · windows 재해석 지점을 그대로
/// 연다) **막히지 않게** 열어(unix `O_NONBLOCK` — 쓰는 쪽 없는 FIFO 도 즉시 열린다) 연 핸들이 일반 파일이고 상한 이하일 때만
/// 읽는다(상한 + 1 바이트까지 읽어 넘치면 거부 — 읽는 동안 자라도 성공이 아니다). FIFO · 소켓 · 장치 · 폴더 · 링크 · 상한 초과
/// = 판독 불가. 시작 경로가 이 파일들로 무기한 멈추지 않는다.
pub fn read_start_meta(p: &Path) -> StartRead {
    start_read(p, START_META_CAP, false)
}

/// ★T2h(RULINGS CW3): 살아 있는 팩 파일 판독 — 사용자가 둔 링크는 **일반 파일을 가리킬 때만** 따른다(지금 동작). 막히지 않게
/// 열고 일반 파일 · 상한 이하만 읽는다 — FIFO · 장치 · 상한 초과 = 판독 불가(그 파일은 '판독 불가' 경로를 탄다).
pub fn read_start_live(p: &Path) -> StartRead {
    start_read(p, START_LIVE_CAP, true)
}

fn start_read(p: &Path, cap: u64, follow: bool) -> StartRead {
    use std::io::Read;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NONBLOCK | if follow { 0 } else { libc::O_NOFOLLOW });
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        if !follow {
            opts.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
        }
    }
    let f = match opts.open(p) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return StartRead::Missing,
        Err(e) => return StartRead::Unreadable(format!("열기 실패: {e}")),
    };
    let m = match f.metadata() {
        Ok(m) => m,
        Err(e) => return StartRead::Unreadable(format!("상태 확인 실패: {e}")),
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if !follow && m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return StartRead::Unreadable("재해석 지점(링크)은 따르지 않는다".into());
        }
    }
    if !m.is_file() {
        return StartRead::Unreadable("일반 파일이 아니다(FIFO·소켓·장치·폴더·링크)".into());
    }
    if m.len() > cap {
        return StartRead::Unreadable(format!("크기 상한({cap}바이트) 초과"));
    }
    let mut bytes = Vec::new();
    if let Err(e) = f.take(cap + 1).read_to_end(&mut bytes) {
        return StartRead::Unreadable(format!("읽기 실패: {e}"));
    }
    if bytes.len() as u64 > cap {
        return StartRead::Unreadable("크기 상한 초과(읽는 동안 커졌다)".into());
    }
    StartRead::Bytes(bytes)
}

/// ★F10 R2-m3: 상한 + 1 바이트까지 읽어 넘치면 거부한다 — stat 뒤에 파일이 자라도 앞부분(상한까지)만 해시한 '성공'이 나오지 않는다.
fn read_capped_utf8<R: std::io::Read>(r: R, cap: u64) -> std::io::Result<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    r.take(cap + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "크기 상한 초과(읽는 동안 커졌다)"));
    }
    String::from_utf8(bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.utf8_error()))
}

/// ★F10 R2-m2: 순회가 끝까지 가지 못한 사유(보고는 이것을 '깨끗함'으로 읽지 않는다 — 진단은 WARN).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HooksIncomplete {
    /// 깊이 상한(`HOOKS_WALK_MAX_DEPTH`)에서 멈춘 디렉터리 수.
    pub depth_cut: usize,
    /// 방문 항목(파일 + 디렉터리) 수 상한(`HOOKS_WALK_MAX_ENTRIES`)에 닿아 멈췄는가.
    pub entries_cut: bool,
    /// 열 수 없던 디렉터리 수(최상위 `hooks/` 가 없는 경우는 세지 않는다 — 설치된 적 없는 팩).
    pub unreadable_dirs: usize,
}

impl HooksIncomplete {
    fn is_empty(&self) -> bool {
        self.depth_cut == 0 && !self.entries_cut && self.unreadable_dirs == 0
    }
    /// 한 줄 사유.
    pub fn describe(&self) -> String {
        let mut v = Vec::new();
        if self.depth_cut > 0 {
            v.push(format!("깊이 상한({}) 초과 폴더 {}개", HOOKS_WALK_MAX_DEPTH, self.depth_cut));
        }
        if self.entries_cut {
            v.push(format!("항목 수 상한({}) 도달", HOOKS_WALK_MAX_ENTRIES));
        }
        if self.unreadable_dirs > 0 {
            v.push(format!("열 수 없는 폴더 {}개", self.unreadable_dirs));
        }
        v.join(" · ")
    }
}

fn walk_hooks_dir(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let _ = walk_hooks_dir_checked(dir, base, out);
}

/// 순회 + 불완전 결과. 방문 수는 파일뿐 아니라 **디렉터리도** 센다(디렉터리만 있는 넓은 나무가 상한을 우회하지 못하게).
fn walk_hooks_dir_checked(dir: &Path, base: &Path, out: &mut Vec<String>) -> Option<HooksIncomplete> {
    let mut inc = HooksIncomplete { depth_cut: 0, entries_cut: false, unreadable_dirs: 0 };
    let mut visited = 0usize;
    walk_hooks_dir_bounded(dir, base, out, 0, &mut visited, &mut inc, true);
    (!inc.is_empty()).then_some(inc)
}

fn walk_hooks_dir_bounded(
    dir: &Path,
    base: &Path,
    out: &mut Vec<String>,
    depth: usize,
    visited: &mut usize,
    inc: &mut HooksIncomplete,
    top: bool,
) {
    if depth > HOOKS_WALK_MAX_DEPTH {
        inc.depth_cut += 1;
        return;
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            if !(top && e.kind() == std::io::ErrorKind::NotFound) {
                inc.unreadable_dirs += 1;
            }
            return;
        }
    };
    for e in rd.flatten() {
        if *visited >= HOOKS_WALK_MAX_ENTRIES {
            inc.entries_cut = true;
            return;
        }
        *visited += 1;
        let p = e.path();
        // 디렉터리 심링크는 따라가지 않는다(file_type 은 따라가지 않는다) — 순환 · 팩 밖으로의 이탈 없음. 나머지는 항목으로 올려 분류에 맡긴다.
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            walk_hooks_dir_bounded(&p, base, out, depth + 1, visited, inc, false);
        } else if let Ok(rel) = p.strip_prefix(base) {
            out.push(normalize_manifest_key(&rel.to_string_lossy()));
        }
    }
}

/// `hooks_report_scan` 의 결과 — 행과, 순회가 끝까지 가지 못했는가.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HooksScan {
    pub rows: Vec<HookRow>,
    pub incomplete: Option<HooksIncomplete>,
}

/// 읽기 전용 수집: `pack_dir/hooks/` 아래를 매니페스트 · vendor 와 대조해 `same` 이 아닌 행만 돌려준다(정렬).
/// 아무것도 쓰지 않고 디렉터리를 만들지 않으며 데몬에 닿지 않는다. `vendor` 는 주입 가능(시험) — 운영은 `vendor_hook_hashes()`.
pub fn hooks_report(
    pack_dir: &Path,
    manifest: &std::collections::BTreeMap<String, String>,
    vendor: &std::collections::BTreeMap<String, String>,
) -> Vec<HookRow> {
    hooks_report_scan(pack_dir, manifest, vendor).rows
}

/// `hooks_report` 와 같되 순회가 끝까지 갔는지도 알려 준다(R2-m2) — 진단(doctor)은 이쪽을 쓴다.
pub fn hooks_report_scan(
    pack_dir: &Path,
    manifest: &std::collections::BTreeMap<String, String>,
    vendor: &std::collections::BTreeMap<String, String>,
) -> HooksScan {
    let mut rels: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let man: std::collections::BTreeMap<String, &String> = manifest
        .iter()
        .map(|(k, v)| (normalize_manifest_key(k), v))
        .filter(|(k, _)| k.starts_with("hooks/"))
        .collect();
    rels.extend(man.keys().cloned());
    // vendor 에만 있고 매니페스트에도 디스크에도 없는 파일은 보고하지 않는다(아직 설치된 적 없음 — 다음 설치가 들여온다).
    let mut on_disk = Vec::new();
    let incomplete = walk_hooks_dir_checked(&pack_dir.join("hooks"), pack_dir, &mut on_disk);
    rels.extend(on_disk);
    let mut rows = Vec::new();
    for rel in rels {
        let p = pack_dir.join(&rel);
        let disk = match read_pack_text_bounded(&p) {
            Ok(text) => DiskRead::Hash(content_hash(&text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && std::fs::symlink_metadata(&p).is_err() => DiskRead::Missing,
            Err(_) => DiskRead::Unreadable,
        };
        let m = man.get(&rel).map(|s| s.as_str());
        let v = vendor.get(&rel).map(|s| s.as_str());
        let state = classify_hook(&disk, m, v);
        if state == HookState::Same {
            continue;
        }
        rows.push(HookRow {
            rel,
            state,
            disk: if let DiskRead::Hash(h) = &disk { Some(h.clone()) } else { None },
            manifest: m.map(str::to_string),
            vendor: v.map(str::to_string),
        });
    }
    HooksScan { rows, incomplete }
}

/// ★F10 C3 — 안내 문장의 **단일 출처**. 이 상수를 `cys doctor` 의 `hooks/` drift 행 아래와 `cys pack-ownership <hooks/…>` 가 찍는다.
/// `pack-guard.sh` 의 메시지는 같은 낱말을 쓴다(`<rel>` 만 경로로 치환). 0.15.0 M2: 밖으로 보내는 제안 경로 대신 수리 장부(`cys repair`)를 가리킨다.
pub const HOOKS_LOCAL_GUIDANCE: &str = "hooks/ 아래 vendor 파일의 수정은 제자리에 보존됩니다. 「스스로 해결」 수리라면 워커가 고치기 전에 cys repair begin --files <rel> --trigger <계기>(owner-order · grant · update 중 하나) 로 시작해 원본을 보관한 뒤 고치고 cys repair record 로 수리 장부에 남깁니다(장부가 업데이트 뒤 다시 입히기·되돌리기·감사의 근거) — 기동 필수·보호장치 관련 훅은 살아 있는 파일이 아니라 begin 이 알려 주는 작업 사본 repairs/R-<n>/staging/ 에서 고치고, 리뷰어1의 검증과 리뷰어2의 감사 서명을 받은 뒤 cys repair submit 으로 수정 최종 승인을 받습니다. 따로 두고 싶은 자기 훅은 ~/.cys/local/hooks/<이벤트>.d/ 에 둡니다(업데이트·치유 불가침).";

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
pub enum Ownership {
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
    /// healed 폴백). ★0.15.0 M11: 실패 폴백은 기동 필수 파일이면 `VendorFirst`, 아니면 `UserKept`.
    Merge3,
    /// ★0.15.0 M11 가(사용자 우선): 기동 필수가 아닌 시스템 파일의 충돌·기준본 없음·손상 의심 —
    /// **사용자 파일이 살아 있는 파일로 남고** vendor 신버전은 `<rel>.new` 로 보관, 원장 kind
    /// `user-kept`. 설치 기록(매니페스트)은 앞당기지 않는다(앞당기면 '고친 상태로 안정'으로
    /// 분류돼 다시는 알리지 않는다 — 설계 §4-11 가). `reason` 은 원장에 남는 판정 사유.
    UserKept { reason: &'static str },
    /// ★0.15.0 M11 나(기동 필수 충돌): vendor 본을 먼저 살리고 사용자 수정본은 `<rel>.user`
    /// (이전 세대는 `.user.prev-<시각>`)로 보관, 원장 kind `vendor-first` — 켜진 뒤 워커가 승인
    /// 절차(`cys repair reapply`)로 다시 입힌다. 보관이 불가능하면 덮지 않고 격리한다(부록 A-2 #4).
    VendorFirst { reason: &'static str },
    /// ★0.15.0 M11 마(묘비): 수리 장부에 기록된 삭제 — 부재를 유지한다(만들지 않는다). 생명줄 훅
    /// 여섯은 이 판정에 오지 않는다(묘비가 걸러진다). `repair` = 그 삭제를 기록한 수리 번호.
    KeepAbsent { repair: String },
}

/// ★0.15.0 M11(CONTRACTS O-C8-2): 설치 기록에서 본 이 파일의 기준 상태 — 있음(해시) · 없음 · 깨짐.
/// 깨짐은 매니페스트 파일이 있으나 읽을 수 없는 경우다(그때 지금 코드는 모든 파일을 '기준본
/// 없음'으로 본다 — 설계 §4-11 다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestBase<'a> {
    Some(&'a str),
    None,
    Corrupt,
}

/// ★0.15.0 M11(CONTRACTS O-C8-2) 순수 분류기의 결과 — 업데이트 규칙의 네 갈래.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateClass {
    /// 가: 사용자 파일 유지 + vendor 신버전 `.new` 보관(`park_new` 는 늘 참 — 계약 모양 보존).
    UserWins { park_new: bool },
    /// 나: vendor 본 우선 + 사용자 수정본 시각별 보관(`park_user_ts` 는 늘 참).
    VendorFirst { park_user_ts: bool },
    /// 사용자 수정이 아니다(설치 당시 해시 · 과거 vendor 본과 바이트 동일) — 새 버전으로 갱신.
    VendorUpdate,
    /// 이 규칙의 대상이 아니다 — 지금 동작 그대로(판독 불가 가-2 · 잠금 자산 L0 · vendor 미전진
    /// kept-drift · seed-once · 사용자가 고친 user-owned 의 `.new` 병치).
    KeepAsToday,
}

/// ★0.15.0 M11(CONTRACTS O-C8-2 · RULINGS A) **순수 분류기** — `decide_file_action` 옆에 둔다.
/// 입력은 호출자가 모은 사실이고(IO 0), 존재하며 vendor 본과 다른 파일 하나에 대해 업데이트
/// 규칙의 갈래를 정한다:
/// - 판독 불가(`encoding_ok=false` 또는 해시 없음) → `KeepAsToday`(가-2: 시스템 파일은 바이트 백업 뒤
///   vendor 본, 사용자 소유 파일은 강제 설치에서도 그대로 — 부록 A-2 #3).
/// - user-owned: 그 파일 자신의 설치 기록 해시 또는 그 파일의 과거 vendor 해시와 바이트가 같고
///   (가드 1) 병합 감사에 '내 것 유지'·'되돌림'이 없으면(가드 2) → `VendorUpdate`(바). 가드 3(규칙
///   문서 인수 검사)·가드 4(CEO 승격 중 MASTER)·부서 재작성 파일 제외는 호출자가 거부권으로 적용한다.
/// - system: 설치 기록 해시와 같음 → `VendorUpdate` · 기록 없음/깨짐 + 과거 vendor 본과 같음 →
///   `VendorUpdate`(다 — 사용자 수정으로 판정하기 **전에** 표부터, 부록 A-2 #1) · vendor 미전진 →
///   `KeepAsToday`(kept-drift) · 그 밖 → 기동 필수면 `VendorFirst`, 아니면 `UserWins`.
#[allow(clippy::too_many_arguments)]
pub fn classify_update(
    rel: &str,
    ownership: Ownership,
    boot_critical: bool,
    base: ManifestBase<'_>,
    disk_hash: Option<&str>,
    vendor_hash: &str,
    prev_vendor_match: bool,
    audit_keep: bool,
    encoding_ok: bool,
) -> UpdateClass {
    let Some(d) = disk_hash.filter(|_| encoding_ok) else {
        return UpdateClass::KeepAsToday;
    };
    if d == vendor_hash {
        return UpdateClass::KeepAsToday;
    }
    match ownership {
        Ownership::SeedOnce => UpdateClass::KeepAsToday,
        Ownership::User => {
            let unedited = matches!(base, ManifestBase::Some(h) if h == d) || prev_vendor_match;
            if unedited && !audit_keep {
                UpdateClass::VendorUpdate
            } else {
                UpdateClass::KeepAsToday
            }
        }
        Ownership::System => {
            if system_locked(rel) {
                return UpdateClass::KeepAsToday;
            }
            match base {
                ManifestBase::Some(h) if h == d => UpdateClass::VendorUpdate,
                ManifestBase::Some(h) if h == vendor_hash => UpdateClass::KeepAsToday,
                ManifestBase::None | ManifestBase::Corrupt if prev_vendor_match => {
                    UpdateClass::VendorUpdate
                }
                _ if boot_critical => UpdateClass::VendorFirst { park_user_ts: true },
                _ => UpdateClass::UserWins { park_new: true },
            }
        }
    }
}

/// ★0.15.0 M11: 한 파일의 판정 입력 중 **IO 로만 알 수 있는 사실** — 설치 실행부·드라이런(plan)·
/// 되살리기 모의 판정이 같은 수집기(`UpdateContext::facts`)로 만든다('계획 = 실제').
/// `Default` = 중립 사실(묘비 없음 · 기동 필수 아님 · 과거 vendor 일치 없음 · 감사 유지 없음 ·
/// 인수 검사 미통과 · 승격 아님) — 옛 시그니처 `decide_file_action` 이 이것으로 위임한다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateFacts {
    /// C4 질의(컴파일된 기동 필수 목록) — T5a 병합 전에는 목록이 비어 늘 거짓(계약의 시험 이음새).
    pub boot_critical: bool,
    /// 매니페스트 파일이 있으나 깨져 읽을 수 없음(다).
    pub manifest_corrupt: bool,
    /// 디스크 바이트가 이 파일의 과거 vendor 본 해시 중 하나와 같음(다 · 바 가드 1).
    pub prev_vendor_match: bool,
    /// 병합 감사 원장에 이 파일의 '내 것 유지'(keep-mine)·'되돌림'(rollback) 기록이 있음(바 가드 2 ·
    /// RULINGS C X2: `.merge-audit.jsonl` 을 읽는다).
    pub audit_keep: bool,
    /// 규칙 문서(헌법 파일)의 안전 조항 인수 검사 통과(바 가드 3 — 실패 시 `.new`).
    pub takeover_ok: bool,
    /// 본부 지침이 CEO 승격 상태(바 가드 4 — `cys.rs` A12 가드와 같은 조건).
    pub ceo_promoted: bool,
    /// 이 레인의 묘비가 이 파일의 삭제를 기록함 — 그 수리 번호(마). 생명줄은 결코 Some 이 아니다.
    pub tombstoned: Option<String>,
    /// 이미 `user-kept` 로 보관 중이고 `<rel>.new` 가 지금 vendor 본과 같음 — 다시 합치지 않는다(가).
    pub parked_same_vendor: bool,
}

/// 본부 지침 상대경로 — 바 가드 4 의 대상(`cys.rs` `MASTER_DIRECTIVE_REL` 과 같은 값).
pub const MASTER_DIRECTIVE_REL: &str = "directives/MASTER_DIRECTIVE.md";

/// 바 가드 4 의 조건 — `cys.rs:ceo_vendor_overwrite_rejection`(A12) 과 **같은 함수**(설계 §6-11 바 "가드 2개를
/// `src/pack.rs` 쪽에 재현"). ★T8(RULINGS CB1·CG2 · CCR-2 승인): '승격'의 한 뜻 —
/// `<pack>/directives/MASTER_DIRECTIVE.md.pre-ceo` 가 **사전적으로** 있다(끊긴 링크·읽을 수 없는 파일·폴더도 '있다' —
/// [`ceo_pre_ceo_present_in`]) **또는** 살아 있는 본부 지침에 생성기의 조립 구분선([`CEO_SEPARATOR`], LF·CRLF)이 있다.
/// 0.15.0 의 CEO 지침은 조립(생성된 머리말 + 구분선 + 사용자 본부 지침)이라 백업이 지워져도 내용이 승격을 말한다 —
/// 그 지침을 vendor 본으로 덮으면 CEO 머리말과 본문 수정이 함께 사라진다. 같은 술어: javis_preflight
/// `Preflight._c03_ceo_promoted`(파이썬 정본) · javis_org · cys-dept `ceo_py promoted` · 앱 `ceo_promoted` —
/// `cysjavis-pack/bin/tests/fixtures/ceo-promoted-cases.json` 한 벌이 여섯 판독자를 같은 답으로 묶는다
/// (시험 `ceo_promoted_in_matches_the_shared_fixture`).
pub fn ceo_promoted_in(dir: &Path) -> bool {
    ceo_pre_ceo_present_in(dir)
        || read_start_live(&dir.join(MASTER_DIRECTIVE_REL))
            .bytes()
            .map(|b| has_ceo_separator(&b))
            .unwrap_or(false)
}

/// `.pre-ceo` 의 실재 — 사전적(`symlink_metadata`: 항목이 있으면 '있다'). `exists()` 는 링크를 따라가 끊긴 링크를
/// '없다'고 본다(RULINGS CG2 — 지키는 방향의 한 판정).
pub fn ceo_pre_ceo_present_in(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir.join(format!("{MASTER_DIRECTIVE_REL}.pre-ceo"))).is_ok()
}

/// 조립 구분선 — `scripts/gen_ceo_template.py` 의 `SEPARATOR` 와 같은 바이트(`test_ceo_composition.py` 가 고정한다).
pub const CEO_SEPARATOR: &str = "\n---\n\n# [본문 — 표준 MASTER 운영 계약 전문]\n\n";

/// 지침에 조립 구분선이 있는가 — LF 철자 또는 같은 바이트의 CRLF 철자.
fn has_ceo_separator(b: &[u8]) -> bool {
    let crlf = CEO_SEPARATOR.replace('\n', "\r\n");
    let has = |needle: &[u8]| b.windows(needle.len()).any(|w| w == needle);
    has(CEO_SEPARATOR.as_bytes()) || has(crlf.as_bytes())
}

/// 바 가드 2 — `.merge-audit.jsonl` 에서 사용자가 일부러 옛 버전을 유지한 기록이 있는 파일 집합.
/// 행동 `keep-mine`(pack-merge)·`rollback`(pack-rollback)만 센다. 거부 기록(flags 에 `refused`)은
/// 사용자의 선택이 아니라 게이트의 거부이므로 세지 않는다. 판독 실패 행은 건너뛴다(파일 부재 = 빈 집합).
/// ★T2h(RULINGS CW3): `None` = 감사 원장을 읽을 수 없다(FIFO · 폴더 · 링크 · 상한 초과 · 읽기 오류) — 호출부는 **모든** user 소유
/// 파일을 '내 것 유지'로 본다(보수적 기본값: 자동 갱신하지 않는다). 부재 = 빈 집합(지금 동작).
pub fn merge_audit_keep_rels(dir: &Path) -> Option<std::collections::BTreeSet<String>> {
    let mut out = std::collections::BTreeSet::new();
    let text = match read_start_meta(&dir.join(MERGE_AUDIT_FILE)) {
        StartRead::Missing => return Some(out),
        StartRead::Unreadable(_) => return None,
        StartRead::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
    };
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let action = v.get("action").and_then(|a| a.as_str()).unwrap_or("");
        if action != "keep-mine" && action != "rollback" {
            continue;
        }
        let refused = v
            .get("flags")
            .and_then(|f| f.as_array())
            .is_some_and(|f| f.iter().any(|x| x.as_str() == Some("refused")));
        if refused {
            continue;
        }
        if let Some(file) = v.get("file").and_then(|f| f.as_str()) {
            out.insert(file.to_string());
        }
    }
    Some(out)
}

/// 생명줄 훅 파일인가 — `hooks/<여섯 이름>`(CONTRACTS 8.7 · `repair::boot_critical::LIFELINE_HOOKS`).
/// 묘비는 이 파일들에 결코 적용되지 않는다(언제나 되살린다 — 설계 §7 6행).
pub fn is_lifeline_rel(rel: &str) -> bool {
    rel.strip_prefix("hooks/")
        .is_some_and(|name| crate::repair::boot_critical::LIFELINE_HOOKS.contains(&name))
}

/// 셸 스크립트인가 — 라(CR 금지)의 대상. 확장자 `.sh`, 또는 첫 줄이 셸 인터프리터(`#!…sh`)인 파일(★FT1: 폴더는 따지지 않는다).
/// 판정은 vendor 본(임베드) 기준이다(사용자본의 첫 줄은 깨져 있을 수 있다).
pub fn is_shell_script(rel: &str, vendor: &str) -> bool {
    // ★T2i(RULINGS FT1): '셸 스크립트'는 셸 스크립트다 — 이름이 `.sh` 로 끝나거나 vendor 첫 줄이 sh / bash / zsh shebang 일 때만.
    //   폴더(`hooks/`)는 셸 스크립트를 만들지 않는다(종전: `hooks/` 아래 모든 파일 — README.md · inject_gate.py · Windows 배치
    //   cys-agy-statusline.cmd 의 CR 이 지워졌고 원본은 어디에도 남지 않았다 · TW1 CW7-ENC).
    if rel.ends_with(".sh") {
        return true;
    }
    let first = vendor.lines().next().unwrap_or("");
    first.starts_with("#!")
        && (first.ends_with("/sh")
            || first.ends_with("/bash")
            || first.ends_with("/zsh")
            || first.contains("env bash")
            || first.contains("env sh")
            || first.contains("env zsh"))
}

/// 팩 상대경로 하나에 대한 질의(기동 필수 · 목록 등재) — 입력원의 판정 함수 칸.
type RelPredicate = Box<dyn Fn(&str) -> bool + Send + Sync>;
/// 팩 상대경로 하나의 과거 출시 해시 목록 — 입력원의 표 칸.
type RelHashes = Box<dyn Fn(&str) -> Vec<String> + Send + Sync>;

/// ★T2f(RULINGS BD4 · BH6 · BC2) · T5a 재기반 2단계: **"두 컴파일 목록 중 하나에 있는가"** — 팩 쓰기 동사(되살리기 ·
/// 합치기 · 병합 되돌리기 · 되돌리기)가 출시본이 아닌 바이트를 넣지 못하는 파일의 단일 술어. 기동 필수 목록의 행
/// (`boot_critical::boot_critical_entry`) **또는** 보호 목록의 행(`boot_critical::protection_entry`)이면 참이다 — 두
/// 조회는 목록 자신의 대조 키(`rel_key`: `/` 구분자, 앞의 `./` 제거, T3c 의 `repair::types::fold_name` 으로 고정점까지
/// 접기 — RULINGS BN1 · CD1)로 맞추므로, 같은 파일을 여는 어느 철자든 같은 답이다(호출자가 이미 접은 키를 넘겨도 같다).
/// 업데이트의 충돌 처리는 이 술어를 쓰지 않는다: 보호 목록에만 있는 파일의 업데이트 충돌은 사용자 사본이 남는다(주인
/// 결정 20 · AQ12) — 업데이트의 기동 필수 판정은 `boot_critical_rel` 그대로다.
pub fn listed_in_compiled_lists(rel: &str) -> bool {
    crate::repair::boot_critical::boot_critical_entry(rel).is_some()
        || crate::repair::boot_critical::protection_entry(rel).is_some()
}

/// ★T2f(RULINGS BO3): 이 바이트가 **출시본**인가 — 이 바이너리의 내장 vendor 본 또는 과거 출시 해시 표 중 하나와 같은
/// UTF-8 바이트. 이 판정은 **바이너리 자신의 지식**으로만 한다: 설치 매니페스트·`.new`·pack.prev 의 매니페스트처럼
/// 일반 쓰기가 바꿀 수 있는 파일은 근거가 되지 않는다(매니페스트의 해시는 그 자체가 내장본·표의 해시일 때만 뜻이
/// 있는데, 그때는 이미 위 두 근거에 든다). 팩 쓰기 동사가 등재 파일에 쓸 수 있는 유일한 바이트다. `dir` 은 쓰지
/// 않는다(호출 모양을 바꾸지 않으려고 남겨 둔 인자 `_dir` — 판정에 디스크 파일을 읽지 않는다는 표시).
pub fn released_bytes(_dir: &Path, rel: &str, bytes: &[u8], sources: &UpdateSources) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else { return false };
    let h = content_hash(text);
    PACK_ALL.iter().any(|(r, c)| *r == rel && content_hash(c) == h)
        || sources.prev_vendor_for(rel).contains(&h)
}

/// ★T2f: 등재 파일에 출시본이 아닌 바이트를 넣으려는 팩 쓰기 동사의 거부 문장(동사마다 같은 문장 — 수리 흐름 안내).
pub fn listed_file_refusal(rel: &str, verb: &str) -> String {
    format!(
        "⛔ 거부: '{rel}' 은 기동 필수·보호 목록에 있는 파일입니다 — `{verb}` 는 이 파일에 출시본이 아닌 바이트를 넣지 \
         않습니다(아무것도 쓰지 않았습니다). 이 파일을 바꾸는 길은 수리 흐름뿐입니다: 워커가 `cys repair begin --files {rel} \
         --trigger …` → 예행·검사 → 주인 승인(업데이트가 보관한 내 사본 {rel}.user 을 다시 입히려면: {}).",
        parked_copy_routes(rel)
    )
}

/// ★T2g(RULINGS CN1 · CN2): 업데이트가 vendor 우선으로 보관한 사본(`<rel>.user` · 원장 vendor-first)을 다시 입히는 두 길 —
/// `cys repair reapply` 는 **수리**가 만든 수정만 받는다(수리 번호가 있어야 한다). 사용자가 손으로 고친 것은 수리 번호가 없으므로
/// 업데이트 계기의 보통 수리다. 팩 동사는 수리 장부를 읽지 않으므로(읽지 않아야 한다) 어느 쪽인지 모른다 — 조건과 함께 둘 다
/// 이름 댄다. `reapply` 혼자는 쓰지 않는다. `rel` 은 파일 이름(또는 `<파일>` 같은 자리 표시).
pub fn parked_copy_routes(rel: &str) -> String {
    format!(
        "수리로 고친 파일이면 `cys repair reapply <수리번호>`, 직접 고친 파일이면 `cys repair begin --files {rel} --trigger update`"
    )
}

/// ★T2f(RULINGS BV2): 등재 파일에 쓰려는 바이트가 이 바이너리가 아는 출시본(내장본·과거 출시 표)이 아닐 때의 거부
/// 문장 — `--take-new`(보관된 `.new`)와 되돌리기(스냅샷 · pack.prev)가 쓴다. 이 바이너리는 변조된 바이트와 자기보다 새
/// 서명 팩(팩만 업데이트)의 바이트를 믿을 만하게 가를 수 없으므로(그 서명 매니페스트를 보관·재검증하지 않는다 — 2차
/// 출시 후보) 둘을 함께 말하는 한 문장이다. 두 길만 알린다: 수리 흐름(이 사본을 후보로 · 검사·서명·주인 승인), 다음
/// 프로그램 업데이트. `cys pack-heal` 은 알리지 않는다(이 바이너리의 더 오래된 사본을 쓰기 때문이다).
pub fn unknown_release_refusal(rel: &str, what: &str) -> String {
    format!(
        "⛔ 거부: '{rel}' 은 기동 필수·보호 목록에 있는 파일이고, 이 바이너리는 {what} 의 바이트를 출시본으로 알지 \
         못합니다(이 바이너리의 내장본·과거 출시 해시 표에 없음 — 변조된 바이트일 수도, 이 바이너리보다 새 서명 팩의 \
         바이트일 수도 있습니다) — 아무것도 쓰지 않았습니다. 이 바이트를 살리는 길은 둘입니다: (1) 수리 흐름 — 워커가 \
         이 사본을 후보로 `cys repair begin --files {rel} --trigger …` → 예행·검사·서명 → 주인 승인, (2) 다음 프로그램 \
         업데이트(그 내장본·과거 출시 표가 이 바이트를 안다)."
    )
}

/// ★0.15.0 M11 설치 입력원 — 레인의 묘비(파일만 · 생명줄 제외), 기동 필수 질의, 과거 vendor 해시
/// 표, 업데이트 전 스냅샷 루트. 설치 실행부·plan·되살리기 모의가 같은 값을 쓴다. 환경 변수 노브 없음.
pub struct UpdateSources {
    /// 설치 대상 팩의 레인(`base` 또는 그 부서 소켓의 레인 키). None = 판정 불가 → 묘비 없음(지금 동작).
    pub lane: Option<crate::repair::types::LaneKey>,
    /// rel → 그 삭제를 기록한 수리 번호(파일 묘비만 · 생명줄 훅 파일 제외 — `with_tombstones` 가 거른다).
    tombstoned_files: std::collections::BTreeMap<String, String>,
    /// ★T2j(RULINGS HP4 (a)): 따르지 않은 파일 묘비 (rel, 수리 번호) — 보호 목록의 파일(설계 8-1 D⑥: 보호장치를 없애는 후보는
    /// 거부되므로 그 묘비는 정당할 수 없다). 묘비 파일은 그대로이고, 설치 보고가 한 번 이름 댄다.
    void_tombstones: Vec<(String, String)>,
    boot_critical: RelPredicate,
    /// ★T2f: 팩 쓰기 동사의 등재 술어(기본 = `listed_in_compiled_lists`) — 기동 필수 판정과 따로 둔다(보호 목록에만
    /// 있는 파일은 등재되지만 업데이트에서는 기동 필수가 아니다 — BC2).
    listed: RelPredicate,
    prev_vendor: RelHashes,
    /// 업데이트 전 스냅샷 루트(`~/.cys/local/update-snapshots`). None = 스냅샷을 만들지 않는다.
    pub snapshot_root: Option<PathBuf>,
}

impl std::fmt::Debug for UpdateSources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateSources")
            .field("lane", &self.lane)
            .field("tombstoned_files", &self.tombstoned_files)
            .field("void_tombstones", &self.void_tombstones)
            .field("snapshot_root", &self.snapshot_root)
            .finish_non_exhaustive()
    }
}

/// 컴파일된 기동 필수 목록(C4 · `repair::boot_critical::entries`)에 이 팩 상대경로가 있는가.
/// 설치기는 팩 상대경로를 이미 알고 있으므로 경로 정규화·레인 해소가 필요 없는 정확 일치로 묻는다
/// (staging 경로는 팩 루트가 아니라 경로 질의가 '판정 불가'가 된다 — INFERRED 선택, 보고서 기재).
/// T5a 병합 전에는 목록이 비어 늘 거짓 = '기동 필수 아님'(CONTRACTS C4 — 이음새: `with_boot_critical`).
pub fn boot_critical_rel(rel: &str) -> bool {
    let k = list_key(rel);
    crate::repair::boot_critical::entries()
        .iter()
        .any(|e| list_key(e.rel) == k)
}

/// ★T2f(RULINGS BO2 · BN1): 목록 대조용 키 — `/` 구분자, 앞의 `./` 제거, **전체 대소문자 접기**(유니코드
/// 대문자화 후 소문자화 — U+212A → `k`, U+017F → `s`, `ß` → `ss`). APFS·NTFS 는 기본적으로 대소문자를 가리지 않으므로
/// 그 철자들은 같은 파일을 연다 — 접기는 '목록에 있다' 쪽으로만 답을 바꾼다(보수 방향). T5a 의 목록 조회
/// (`boot_critical::boot_critical_entry` · `protection_entry` 의 `rel_key`)와 같은 규칙의 지역 사본이다 — T5a 의 마지막
/// 재기반 작업이 `listed_in_compiled_lists` 를 그 조회 함수로 바꿀 때 이 사본을 지운다(구현 하나).
pub fn list_key(rel: &str) -> String {
    let mut r = rel.replace('\\', "/");
    while let Some(t) = r.strip_prefix("./") {
        r = t.to_string();
    }
    r.trim_start_matches('/').to_uppercase().to_lowercase()
}

impl UpdateSources {
    /// 묘비 없음 · 기동 필수 없음 · 실제 과거 vendor 표 · 스냅샷 없음 — 옛 `install_into` 래퍼(시험 전용)가 쓴다.
    pub fn none() -> Self {
        UpdateSources {
            lane: None,
            tombstoned_files: std::collections::BTreeMap::new(),
            void_tombstones: Vec::new(),
            boot_critical: Box::new(boot_critical_rel),
            listed: Box::new(listed_in_compiled_lists),
            prev_vendor: Box::new(|rel: &str| {
                crate::pack_prev_vendor::prev_vendor_hashes(rel)
                    .iter()
                    .map(|h| h.to_string())
                    .collect()
            }),
            snapshot_root: None,
        }
    }

    /// 라이브 팩용 — 레인을 해소해 그 레인의 묘비만 읽고(`repair::tombstones::load`, 실패·부재 = 빈
    /// 집합 = 지금 동작 — CONTRACTS O-C8-3 · O-C1-6), 스냅샷 루트를 `local_dir()/update-snapshots` 로 둔다.
    /// ★테스트 빌드에서 팩 경로 환경 변수가 없으면 `local_dir()`→`pack_dir()` 가 격리 봉인으로 멈추므로
    /// (부록 A-1 #41), 그때는 묘비·스냅샷을 건너뛴다(빈 집합 — 라이브를 결코 읽지 않는다).
    pub fn for_live_pack(dir: &Path) -> Self {
        let mut s = UpdateSources::none();
        let resolvable = cfg!(not(test))
            || pack_dir_from_env().is_some()
            || crate::env_compat("CYS_LOCAL_DIR").is_some();
        if !resolvable {
            return s;
        }
        s.lane = lane_for_install(dir);
        if let Some(lane) = s.lane.clone() {
            match crate::repair::tombstones::load(&lane) {
                Ok(list) => s = s.with_tombstones(list),
                Err(crate::repair::types::RepairError::NotImplemented(_)) => {}
                Err(e) => eprintln!(
                    "[init-pack] ⚠ 레인 '{}' 의 묘비 기록을 읽지 못했습니다({}) — 지금 동작대로 지운 파일을 되살립니다",
                    lane.0,
                    e.code()
                ),
            }
            // 경로 = `repair::paths::update_snapshot_dir` 와 같은 배치(`local/update-snapshots/<lane>/<from>-<ts>`).
            // 테스트 빌드는 `CYS_LOCAL_DIR` 을 명시한 시험만 스냅샷을 만든다(공용 임시 디렉터리의 `local/` 오염 방지).
            if cfg!(not(test)) || crate::env_compat("CYS_LOCAL_DIR").is_some() {
                s.snapshot_root = Some(local_dir().join("update-snapshots"));
            }
        }
        s
    }

    /// 묘비 목록을 싣는다 — 파일 묘비만(훅 묘비는 종류로 건너뛴다 — RULINGS W3(b)), 생명줄 훅 파일은
    /// 버린다(CONTRACTS 8.7: 언제나 되살린다).
    pub fn with_tombstones(mut self, list: Vec<crate::repair::tombstones::Tombstone>) -> Self {
        for t in list {
            if t.kind != crate::repair::tombstones::TombstoneKind::File {
                continue;
            }
            if is_lifeline_rel(&t.rel) || t.rel.starts_with("settings:") {
                continue;
            }
            // ★T2j(RULINGS HP4 (a) · 설계 8-1 D⑥ "보호장치 넷을 없애는 후보가 거부됨"): 보호 목록의 파일은 승인된 수리로도 지울 수
            //   없다 — 그 묘비는 정당할 수 없으므로 생명줄처럼 따르지 않는다(파일은 되살아난다). 묘비 파일은 지우지 않고 설치 보고가
            //   한 번 이름 댄다. 기동 필수 목록에만 있는 파일의 묘비는 따른다 — 승인된 수리의 삭제가 업데이트 뒤에도 남는다
            //   (CONTRACTS 1.5 · 8.7: 생명줄 여섯만 묘비가 적용되지 않는다).
            if crate::repair::boot_critical::protection_entry(&t.rel).is_some() {
                self.void_tombstones.push((t.rel, t.repair.as_str().to_string()));
                continue;
            }
            self.tombstoned_files.insert(t.rel, t.repair.as_str().to_string());
        }
        self
    }

    /// 기동 필수 질의 이음새(시험).
    pub fn with_boot_critical(mut self, f: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        self.boot_critical = Box::new(f);
        self
    }

    /// 등재 술어 이음새(시험 — T5a 병합 전 컴파일 목록은 비어 있다).
    pub fn with_listed(mut self, f: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        self.listed = Box::new(f);
        self
    }

    /// ★T2f: 이 파일이 두 컴파일 목록 중 하나에 있는가(`listed_in_compiled_lists` — 팩 쓰기 동사의 단일 술어). 술어에는
    /// 목록 대조 키(`list_key` — 구분자·`./`·전체 대소문자 접기)로 접은 철자를 넘긴다(BO2: 같은 파일을 여는 어느 철자든).
    pub fn is_listed(&self, rel: &str) -> bool {
        (self.listed)(&list_key(rel))
    }

    /// 과거 vendor 해시 표 이음새(시험 — 과거 릴리스 본문은 런타임에 없다).
    pub fn with_prev_vendor(
        mut self,
        f: impl Fn(&str) -> Vec<String> + Send + Sync + 'static,
    ) -> Self {
        self.prev_vendor = Box::new(f);
        self
    }

    /// 스냅샷 루트·레인 이음새(시험).
    pub fn with_snapshot(mut self, lane: &str, root: PathBuf) -> Self {
        self.lane = Some(crate::repair::types::LaneKey(lane.to_string()));
        self.snapshot_root = Some(root);
        self
    }

    /// 이 파일의 삭제를 기록한 수리 번호(생명줄은 결코 None 이 아닌 값을 갖지 않는다).
    pub fn tombstone_for(&self, rel: &str) -> Option<&str> {
        self.tombstoned_files.get(rel).map(String::as_str)
    }

    /// ★T2j(RULINGS HP4 (a)): 따르지 않은 보호장치 파일의 묘비 (rel, 수리 번호) — 설치 보고용.
    pub fn void_tombstones(&self) -> &[(String, String)] {
        &self.void_tombstones
    }

    /// 허가된 부재 집합 — 설치 준비본 존재 검사(`verify_staging_with`)가 쓴다.
    pub fn authorised_absent(&self) -> std::collections::BTreeSet<String> {
        self.tombstoned_files.keys().cloned().collect()
    }

    pub fn is_boot_critical(&self, rel: &str) -> bool {
        (self.boot_critical)(rel)
    }

    /// 과거 출시 해시 표 조회 — 설치 판정(다 · 바 비수정 판정)과 스냅샷의 출시본 판정(RULINGS AC1)이 이 하나를
    /// 쓴다. 라이브·`none()` 입력원에서는 `pack_prev_vendor::prev_vendor_hashes(rel)` 그대로다(AC5 핀).
    pub fn prev_vendor_for(&self, rel: &str) -> Vec<String> {
        (self.prev_vendor)(rel)
    }
}

/// 설치 대상 팩의 레인 키(CONTRACTS O-C8-3: `~/.cys/pack` → `base`, `~/.cys/pack-dept-<name>` → 그 레인).
/// 부서 레인 키는 소켓 경로에서 나오므로(`lane::lane_key`), 이 프로세스의 `CYS_SOCKET` 이 그 부서의
/// 소켓일 때만(`lane_pack_for_socket` 이 같은 팩을 가리킬 때) 해소한다. 어긋나면 None = 묘비 없음(지금 동작).
pub fn lane_for_install(pack: &Path) -> Option<crate::repair::types::LaneKey> {
    if dept_scope_of(pack).is_none() {
        return Some(crate::repair::types::LaneKey("base".to_string()));
    }
    let sock = crate::socket_path();
    let lane_pack = lane_pack_for_socket(&sock)?;
    if lane_pack.file_name() == absolutize(pack).file_name() {
        Some(crate::repair::types::LaneKey::of_socket(&sock))
    } else {
        None
    }
}

/// 매니페스트 로드 + 깨짐 여부(파일이 있는데 JSON 으로 읽히지 않음). 깨지면 빈 표(지금 동작)와 true.
pub fn load_manifest_state(dir: &Path) -> (std::collections::BTreeMap<String, String>, bool) {
    // ★T2h(RULINGS CW3): 시작 경로 판독 — 판독 불가(FIFO · 폴더 · 링크 · 상한 초과)는 '깨짐'과 같다(보수 경로).
    match read_start_meta(&dir.join(INSTALL_MANIFEST)) {
        StartRead::Bytes(b) => match serde_json::from_slice(&b) {
            Ok(m) => (m, false),
            Err(_) => (std::collections::BTreeMap::new(), true),
        },
        StartRead::Missing => (std::collections::BTreeMap::new(), false),
        StartRead::Unreadable(_) => (std::collections::BTreeMap::new(), true),
    }
}

/// ★0.15.0 M11 사실 수집기 — 한 설치(또는 plan·모의 판정)당 하나. 팩 전체 사실(매니페스트 깨짐 ·
/// CEO 승격 · 병합 감사)은 한 번만 읽는다.
pub struct UpdateContext<'s> {
    dir: PathBuf,
    scope: PackScope,
    sources: &'s UpdateSources,
    manifest_corrupt: bool,
    ceo_promoted: bool,
    audit_keep: std::cell::OnceCell<Option<std::collections::BTreeSet<String>>>,
}

impl<'s> UpdateContext<'s> {
    /// `dir` = 사실을 읽을 물리 디렉터리(staging 이면 staging — 라이브 팩의 전량 사본이다).
    pub fn new(dir: &Path, scope: PackScope, sources: &'s UpdateSources, manifest_corrupt: bool) -> Self {
        UpdateContext {
            dir: dir.to_path_buf(),
            scope,
            sources,
            manifest_corrupt,
            ceo_promoted: ceo_promoted_in(dir),
            audit_keep: std::cell::OnceCell::new(),
        }
    }

    pub fn sources(&self) -> &UpdateSources {
        self.sources
    }

    /// 한 파일의 사실 — `decide_file_action_with` 의 입력.
    pub fn facts(
        &self,
        rel: &str,
        embed: &str,
        exists: bool,
        disk: Option<&str>,
        manifest_hash: Option<&str>,
        pending: &serde_json::Map<String, serde_json::Value>,
    ) -> UpdateFacts {
        let mut f = UpdateFacts { manifest_corrupt: self.manifest_corrupt, ..UpdateFacts::default() };
        if !exists {
            f.tombstoned = self.sources.tombstone_for(rel).map(str::to_string);
            return f;
        }
        f.boot_critical = self.sources.is_boot_critical(rel);
        let Some(d) = disk else { return f };
        if d == embed {
            return f;
        }
        let dh = content_hash(d);
        f.prev_vendor_match = self.sources.prev_vendor_for(rel).contains(&dh);
        match ownership_scoped(rel, self.scope) {
            Ownership::User => {
                f.audit_keep = self
                    .audit_keep
                    .get_or_init(|| merge_audit_keep_rels(&self.dir))
                    .as_ref()
                    .is_none_or(|kept| kept.contains(rel));
                f.ceo_promoted = rel == MASTER_DIRECTIVE_REL && self.ceo_promoted;
                let unedited = manifest_hash == Some(dh.as_str()) || f.prev_vendor_match;
                if unedited && is_constitution_file(rel) {
                    f.takeover_ok = crate::overrides::verify_constitution_takeover(d, embed).is_ok();
                }
            }
            Ownership::System => {
                // 재시도 금지는 '합쳐 봤는데 안 된' 보관에만 — 병합 전 사본조차 못 만들어 시도하지 못한 보관
                // (capture-failed)은 회복 뒤 다음 스윕이 다시 시도해야 수렴한다(부록 A-2 #4 회복 수렴).
                let e = pending.get(rel);
                let parked = e.and_then(|e| e.get("kind")).and_then(|k| k.as_str()) == Some("user-kept")
                    && e.and_then(|e| e.get("reason")).and_then(|r| r.as_str()) != Some("capture-failed");
                if parked {
                    f.parked_same_vendor = read_start_meta(&self.dir.join(format!("{rel}.new")))
                        .text()
                        .as_deref()
                        == Some(embed);
                }
            }
            Ownership::SeedOnce => {}
        }
        f
    }
}

/// 바: 한 번도 고치지 않은 사용자 소유 파일의 자동 갱신에 대한 거부권 — 가드 3·4 와 부서 재작성 파일.
/// (가드 1·2 는 `classify_update` 안에 있다.)
fn user_owned_auto_update_vetoed(rel: &str, scope: PackScope, facts: &UpdateFacts) -> bool {
    // 부서의 agents.json·acl.json·schedule.json 은 cys-dept 가 다시 쓴다 — 이 규칙에 걸리지 않는다.
    if scope == PackScope::Dept && matches!(rel, "agents.json" | "acl.json" | "schedule.json") {
        return true;
    }
    // 가드 3: 규칙 문서는 안전 조항 인수 검사를 통과해야 바꾼다(실패 시 `.new`).
    if is_constitution_file(rel) && !facts.takeover_ok {
        return true;
    }
    // 가드 4: 본부 지침이 CEO 승격 상태면 건너뛴다.
    rel == MASTER_DIRECTIVE_REL && facts.ceo_promoted
}

/// 순수 판정 SOT(옛 시그니처) — 중립 사실(`UpdateFacts::default()`)로 [`decide_file_action_with`] 에
/// 위임한다. 설치 실행부·plan·되살리기 모의는 `_with` 를 쓰고 사실은 `UpdateContext::facts` 하나로
/// 모은다('계획 = 실제'). 중립 사실 = 묘비 없음·기동 필수 아님·과거 vendor 일치 없음·감사 유지 없음·
/// 인수 검사 미통과(규칙 문서 자동 갱신 안 함)·승격 아님.
/// ★T4 CLI 동사 소비(pack-adopt 스윕 생존 판정 시뮬) — 재구현 금지.
#[allow(clippy::too_many_arguments)]
pub fn decide_file_action(
    rel: &str,
    embed: &str,
    exists: bool,
    disk: Option<&str>, // None = 부재 또는 읽기 실패(비UTF-8 등)
    manifest_hash: Option<&str>,
    force: bool,
    scope: PackScope,
    verified_base: Option<&str>,
) -> FileAction {
    decide_file_action_with(
        rel,
        embed,
        exists,
        disk,
        manifest_hash,
        force,
        scope,
        verified_base,
        &UpdateFacts::default(),
    )
}

/// ★0.15.0 M11 판정 SOT — 설치(프로그램·팩만 받는 업데이트·모든 `init-pack`·데몬 기동 설치)가 같이
/// 쓰는 판정(설계 §4-11 · CONTRACTS 8.1). 순서:
/// ⓪ 부재: 묘비가 있으면 `KeepAbsent`(생명줄은 묘비가 걸러져 오지 않는다), 아니면 새로 만든다(기록
///    없는 삭제는 지금처럼 돌아온다 — 문서화된 '지운 뒤 init-pack' 초기화).
/// ① seed-once 불가침 · user-owned: 판독 불가 보존(A-2 #3) · 같으면 채택 · 바(한 번도 고치지 않은
///    파일은 가드 넷과 함께 새 버전) · 그 밖은 지금처럼 보존 + `.new` 병치.
/// ② system: 같으면 채택 · 설치 당시 해시면 갱신 · 잠금 자산 L0 / 판독 불가 L1 은 지금 동작(가-2) ·
///    [`classify_update`] 로 다(과거 vendor 표)·kept-drift·가/나 를 가른다. 가/나 는 검증된 기준본이
///    있으면 먼저 3-way(`Merge3` — 실패 폴백이 가/나), 이미 같은 vendor 본으로 보관 중이면 다시
///    합치지 않는다(가 — 매번 합치기 전 사본이 쌓이는 것을 막는다).
/// `force` 는 사용자 수정의 판정을 바꾸지 않는다 — `cys init-pack --force` 는 사용자 수정을 버리지 않는다(자).
/// 강제의 효력은 디스크가 이미 vendor 본일 때 같은 내용을 다시 쓰는 것(종전 동일)과 호출자의 다운그레이드
/// 차단 우회·C-07 제자리 보존 우회뿐이다.
#[allow(clippy::too_many_arguments)]
pub fn decide_file_action_with(
    rel: &str,
    embed: &str,
    exists: bool,
    disk: Option<&str>,
    manifest_hash: Option<&str>,
    force: bool,
    scope: PackScope,
    verified_base: Option<&str>,
    facts: &UpdateFacts,
) -> FileAction {
    // ⓪ 부재 — 묘비만 부재를 유지한다.
    if !exists {
        if let Some(r) = &facts.tombstoned {
            return FileAction::KeepAbsent { repair: r.clone() };
        }
        return FileAction::Write { heal_user_copy: false };
    }
    let own = ownership_scoped(rel, scope);
    // ★B2-2 seed-once 상태: 존재하면 불가침(force 여도·읽기 실패여도).
    if own == Ownership::SeedOnce {
        return FileAction::Keep { adopt_hash: false, new_pending: false };
    }
    let base = match manifest_hash {
        Some(h) => ManifestBase::Some(h),
        None if facts.manifest_corrupt => ManifestBase::Corrupt,
        None => ManifestBase::None,
    };
    let Some(d) = disk else {
        // 판독 불가(비UTF-8·권한·잠금) — 가-2 는 지금 동작을 유지한다.
        if own == Ownership::User {
            // ★W-1(P0 · 2026-08-01): 판독 불가 user-owned 는 force 여도 무조건 보존(부록 A-2 #3).
            return FileAction::Keep { adopt_hash: false, new_pending: false };
        }
        // ★L0/L1(v2 §3): system — 바이트 백업 뒤 vendor 본(백업 실행부·격리는 install_into).
        return FileAction::Write { heal_user_copy: true };
    };
    if d == embed {
        // 종전 동일: force 는 같은 내용을 다시 쓴다(매니페스트·pristine 전진 — `w1_*` 핀 ②가 고정한다).
        if force {
            return FileAction::Write { heal_user_copy: false };
        }
        return FileAction::Keep { adopt_hash: true, new_pending: false };
    }
    let dh = content_hash(d);
    let vendor_hash = content_hash(embed);
    let class = classify_update(
        rel,
        own,
        facts.boot_critical,
        base,
        Some(dh.as_str()),
        vendor_hash.as_str(),
        facts.prev_vendor_match,
        facts.audit_keep,
        true,
    );
    if own == Ownership::User {
        if class == UpdateClass::VendorUpdate && !user_owned_auto_update_vetoed(rel, scope, facts) {
            return FileAction::Write { heal_user_copy: false };
        }
        // 사용자가 고친 파일(또는 가드가 막은 파일) — 영구 보존 + vendor 전진 시 `.new` 병치.
        let new_pending = manifest_hash != Some(vendor_hash.as_str());
        return FileAction::Keep { adopt_hash: false, new_pending };
    }
    // ── system ──
    // 설치-당시 해시 그대로(사용자 비수정) + 임베드가 더 새 버전 → 갱신.
    if manifest_hash == Some(dh.as_str()) {
        return FileAction::Write { heal_user_copy: false };
    }
    // ★L0(v2 §3·§7): 잠금 보안 자산 — 전 클래스 즉시 치유(현행 유지 — 설계 §7 5행 주인님 승인 예외).
    if system_locked(rel) {
        return FileAction::Write { heal_user_copy: true };
    }
    let base_reason = if manifest_hash.is_none() {
        if facts.manifest_corrupt {
            "manifest-corrupt"
        } else {
            "no-base"
        }
    } else {
        "base-unverified"
    };
    match class {
        UpdateClass::VendorUpdate => FileAction::Write { heal_user_copy: false },
        UpdateClass::KeepAsToday => FileAction::KeepDrift,
        UpdateClass::UserWins { .. } => {
            if facts.parked_same_vendor {
                return FileAction::UserKept { reason: "parked" };
            }
            if manifest_hash.is_some() && verified_base.is_some() {
                return FileAction::Merge3;
            }
            FileAction::UserKept { reason: base_reason }
        }
        UpdateClass::VendorFirst { .. } => {
            // ★RULINGS BO1(종전: 검증 base 가 있으면 먼저 `Merge3` — T2 가 유지했던 0.14.x 거동): 설치기는 기동 필수 파일에
            // 병합 결과를 **쓰지 않는다** — 고친 기동 필수 파일을 vendor 가 전진시키면 언제나 vendor 우선(출시본이 살아
            // 있고 고친 사본은 시각별 보관 · vendor-first 기록). 다시 입히기는 `cys repair reapply` — 워커가 합치고 예행·
            // 검사·서명·주인 승인을 거친다(D:911, D:913). 일반 파일은 위 UserWins 의 설치 시 병합을 그대로 쓴다.
            let reason = if manifest_hash.is_some() && verified_base.is_some() {
                "vendor-advanced"
            } else {
                base_reason
            };
            FileAction::VendorFirst { reason }
        }
    }
}

/// 병합 대기 원장 로드(부재·손상 = 빈 원장, 기동 차단 0).
pub fn load_merge_pending(dir: &Path) -> serde_json::Map<String, serde_json::Value> {
    // ★T2h(RULINGS CW3): 시작 경로 판독 — 판독 불가(FIFO · 폴더 · 링크 · 상한 초과)는 빈 원장(지금의 파싱 실패와 같다).
    read_start_meta(&dir.join(MERGE_PENDING_FILE))
        .bytes()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// 병합 대기 원장 저장. ★T2h(RULINGS CW5): 실패를 돌려준다 — vendor 우선 기록의 선저장(RULINGS AC7)은 이 결과를 보고,
/// 기록이 디스크에 남지 않으면 살아 있는 기동 필수 파일을 바꾸지 않는다(보관 실패와 같은 격리). 그 밖의 호출부는 자문
/// 메타데이터라 종전대로 실패가 설치를 막지 않는다(호출부가 `let _ =` 로 명시한다).
pub fn save_merge_pending(
    dir: &Path,
    pending: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&serde_json::Value::Object(pending.clone()))
        .map_err(|e| format!("병합 대기 원장 직렬화 실패: {e}"))?;
    let p = dir.join(MERGE_PENDING_FILE);
    // ★T2i(RULINGS EC7): 영속이 증명된 쓰기(본문 · 교체 · 폴더 fsync — 실패는 돌려준다). vendor 우선은 이 결과를 보고서만
    //   살아 있는 파일을 바꾼다.
    write_atomic_durable(&p, json.as_bytes()).map_err(|e| format!("병합 대기 원장 기록 실패 {}: {e}", p.display()))
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
    // ★T2h(RULINGS CW3 · CW1): 링크를 따르지 않고(살아 있는 파일의 별칭에 덧붙이지 않는다) 막히지 않게 연다(읽는 쪽 없는
    // FIFO 는 즉시 실패 — 설치가 멈추지 않는다) · 연 핸들이 일반 파일일 때만 쓴다.
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut f = opts.open(&path).map_err(|e| format!("감사 원장 열기 실패 {}: {e}", path.display()))?;
    let m = f.metadata().map_err(|e| format!("감사 원장 상태 확인 실패 {}: {e}", path.display()))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(format!("감사 원장 {} 이 재해석 지점(링크)입니다 — 쓰지 않습니다", path.display()));
        }
    }
    if !m.is_file() {
        return Err(format!("감사 원장 {} 이 일반 파일이 아닙니다 — 쓰지 않습니다", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.nlink() > 1 {
            return Err(format!("감사 원장 {} 이 다른 파일의 별칭(하드 링크)입니다 — 쓰지 않습니다", path.display()));
        }
    }
    // ★T2i(RULINGS EC4 (3)): 윈도우에서도 링크 수는 **연 핸들**의 사실이다(`GetFileInformationByHandle` 의 `nNumberOfLinks`) —
    //   하나가 아니면(보호 파일의 하드 링크) 쓰지 않는다. 그 질의가 실패해도 쓰지 않는다.
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) } == 0 {
            return Err(format!(
                "감사 원장 {} 의 링크 수를 핸들에서 확인할 수 없습니다({}) — 쓰지 않습니다",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        if info.nNumberOfLinks != 1 {
            return Err(format!("감사 원장 {} 이 다른 파일의 별칭(하드 링크)입니다 — 쓰지 않습니다", path.display()));
        }
    }
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
///
/// ★0.15.0 「스스로 해결」 M11 가·나 (CONTRACTS O-C8-1 · RULINGS A): 신 kind 2종을 위 절차로 등재한다.
/// `user-kept` = 기동 필수가 아닌 시스템 파일의 충돌·기준본 없음·손상 의심 — **사용자 파일이 살아 있고**
/// vendor 신버전은 `<rel>.new` 보관(합치기 과제 = 워커). `vendor-first` = 기동 필수 파일의 충돌 — vendor 본을
/// 먼저 살리고 사용자 수정본은 `<rel>.user`(이전 세대는 `.user.prev-<ts>`)로 보관(다시 입히기 = 워커 +
/// 주인 승인 · `cys repair reapply`). `conflicted` 는 재사용하지 않는다 — 그 kind 를 읽는 모든 곳이 뜻을
/// 거꾸로 읽는다(설계 §4-11 가). C68 기한: `user-kept` 는 **제외**(사용자가 이겼다 — 날마다 master 를 깨우지 않는다 ·
/// RULINGS AC7), `vendor-first` 는 기한을 **유지**한다(기동 필수 파일의 다시 입히기 = 승인 절차가 남았다).
pub const LEDGER_KINDS: [&str; 9] = [
    "healed",
    "new-pending",
    "kept-drift",
    "merged",
    "conflicted",
    "quarantined",
    "adopted",
    "user-kept",
    "vendor-first",
];

/// ★T3(D7): 계보 인지 원장 upsert — 기존 upsert_pending 클로저는 자구 불변(new-pending 현행
/// 경로 회귀 0)이며, 신 kind(kept-drift·merged·conflicted·quarantined)와 healed 의 계보 승계는
/// 전부 이 헬퍼를 경유한다.
/// - kept-drift 인입 + 기존 kind∈LINEAGE → kind 불변, `state:"at-rest"` 만 갱신(멱등 — 이미
///   at-rest 면 no-op·dirty 미발생). 정적 재스윕이 merged/conflicted 계보·캡처 포인터를 지우지
///   않는다(v2 §4 KeepDrift arm 쓰기 0 규칙 · merged_kind_not_clobbered 핀).
/// - ★T2g(RULINGS CQ1 · CR2): kept-drift 인입 + 기존 kind = `vendor-first` → **항목 그대로**(no-op · dirty 미발생). 그 기록은
///   "업데이트가 내 사본을 `<rel>.user` 에 보관했고 아직 다시 입히지 않았다"는 계보다 — 살아 있는 파일이 같은 버전에서
///   달라졌다고(손으로 고침) 사라지면 `cys repair reapply` 가 그 사본을 찾지 못한다. 장부만의 규칙이다: 팩 파일 쓰기 ·
///   스윕 순서 · 살아 있는 파일의 판정은 그대로다.
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
        // ★T2g(RULINGS CQ1 · CR2): 보관된 사본의 기록은 같은 버전의 드리프트가 바꾸지 않는다(항목 그대로 — rewrite 0).
        if ok == "vendor-first" && new_kind == "kept-drift" {
            return;
        }
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
    /// ★M11 가: 사용자 파일 유지 + vendor `.new` 보관(합치기 대기).
    pub user_kept: usize,
    /// ★M11 나: vendor 본 우선 + 사용자 수정본 보관(다시 입히기 대기).
    pub vendor_first: usize,
    pub unknown: usize,
}

impl PendingKindCounts {
    /// 조치 가능(actionable) 명시 합 — at-rest 보존 kind(kept-drift·merged)만 제외한 전 버킷
    /// 합(빼기 산식 금지 규율 준수). 미지 kind 는 안전측(가시)으로 포함하고, adopted 도 현행
    /// pack-plan 자구('kept-drift·merged 외 전부 검토 대상')를 보존해 포함한다 — pack-merge
    /// 목록이 '조치 불요(복권됨)' 안내로 해소하는 일시 상태라 과보고가 안전측이다.
    pub fn actionable(&self) -> usize {
        self.healed
            + self.new_pending
            + self.conflicted
            + self.quarantined
            + self.adopted
            + self.user_kept
            + self.vendor_first
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
            Some("user-kept") => c.user_kept += 1,
            Some("vendor-first") => c.vendor_first += 1,
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
    // ★0.15.0 M11('계획 = 실제' — 설계 §6-11 가 "결과 모델 5곳"): 신설 3버킷.
    pub user_kept: Vec<String>,           // ★가: 사용자 파일 유지 + vendor 신버전 <파일>.new 보관(합치기 = 워커)
    pub vendor_first: Vec<String>,        // ★나: 기동 필수 충돌 — vendor 본 우선 + 사용자본 시각별 보관(다시 입히기 = 워커+승인)
    pub keep_absent: Vec<String>,         // ★마: 수리 장부의 묘비 — 부재 유지(만들지 않음)
    pub unchanged: usize,                 // 최신(변화 없음)
    pub prune_delete: Vec<String>,        // 폐기 파일 제거(비수정)
    pub prune_keep_modified: Vec<String>, // 폐기됐지만 수정본이라 보존
    pub blocked: Option<String>,          // 다운그레이드 등 설치 차단 사유(파일 판정 무의미)
}

/// install_into 와 **같은 판정 함수**로 드라이런 리포트를 만든다(쓰기 0·드리프트 0).
/// ★0.15.0 M11: 입력원은 라이브 팩용(`UpdateSources::for_live_pack` — 그 레인의 묘비·기동 필수 질의)
/// 이다 — `cys pack-plan` 이 이 래퍼를 부르므로 설치 실행부와 같은 사실을 본다.
pub fn plan_install(
    dir: &Path,
    items: &[(&str, &str)],
    force: bool,
    target_version: &str,
) -> InstallPlan {
    let sources = UpdateSources::for_live_pack(dir);
    plan_install_with(dir, items, force, target_version, &sources)
}

/// [`plan_install`] 의 입력원 주입판 — 설치 실행부(`install_into_with`)와 같은 수집기·같은 판정.
pub fn plan_install_with(
    dir: &Path,
    items: &[(&str, &str)],
    force: bool,
    target_version: &str,
    sources: &UpdateSources,
) -> InstallPlan {
    // ★G3 축2: 스코프 1회 산출(plan 의 dir 는 실 팩 경로 — staging 우회 없음) → 전 판정에 데이터로 주입.
    plan_install_core(dir, items, force, target_version, pack_scope_of(dir), sources)
}

/// [`plan_install_with`] 의 스코프 주입판(같은 수집기 · 같은 판정) — 설치 실행부가 이번 설치가 덮거나 지울
/// 파일을 미리 고를 때(업데이트 전 스냅샷 · RULINGS AC4) 쓴다. 그 자리의 `dir` 는 staging 일 수 있어(팩 루트가
/// 아니다) 스코프를 호출자가 준다.
fn plan_install_core(
    dir: &Path,
    items: &[(&str, &str)],
    force: bool,
    target_version: &str,
    scope: PackScope,
    sources: &UpdateSources,
) -> InstallPlan {
    let mut plan = InstallPlan::default();
    // 다운그레이드 차단 미러(install_into 와 동일 판정).
    if !force {
        if let Some(dv) = read_start_meta(&dir.join(PACK_VERSION_FILE))
            .text()
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
    let (manifest, manifest_corrupt) = load_manifest_state(dir);
    // ★판정 좁힘의 입력 — 루프 밖에서 한 번만 읽는다(파일마다 읽으면 474회 IO 가 된다).
    let pending_now = load_merge_pending(dir);
    let ctx = UpdateContext::new(dir, scope, sources, manifest_corrupt);
    for (rel, content) in items.iter().copied() {
        let path = dir.join(rel);
        let exists = path.exists();
        let disk = if exists { read_start_live(&path).text() } else { None };
        let mh_raw = manifest.get(rel).map(String::as_str);
        // ★C-07(0.14.55 · P3): 분류 **직전** 낡은 항목 판정. decide_file_action 의 arm 은 무변경.
        let stale =
            classify_stale_entry(dir, rel, scope, content, disk.as_deref(), mh_raw, pending_now.contains_key(rel));
        let repaired = match &stale {
            StaleEntry::Repaired(h) => Some(h.clone()),
            _ => None,
        };
        let mh = repaired.as_deref().or(mh_raw);
        // ★`!force` 게이트(gemini-3 r1 major 1): 제자리 보존은 **보수 기본 경로**의 판정이다.
        //   `--force` 는 사용자가 명시적으로 vendor 본을 강제하는 동사이므로, 그 자리에서까지
        //   분류를 건너뛰면 강제가 조용히 무시된다(적용된 척 0 의 반대 방향 결함).
        if let (StaleEntry::KeepInPlace(_), false) = (&stale, force) {
            // ⛔제자리 보존 — 쓰지 않는다. 낡은 항목을 그대로 두면 L4 강제 치유로 떨어지므로
            //   분류를 건너뛰고 kept-drift 로 계상한다(설계 R3 section 3-2).
            plan.kept_drift.push(rel.to_string());
            continue;
        }
        // ★T3(D9) base lazy 로드 사전 필터 — 이 필터는 **과대포함만 허용**(과소포함=Merge3 침묵
        // 불발) · decide L3 가 최종 판정. 드리프트 파일에서만 pristine read 가 발생한다(전 파일
        // read+hash 2배화 회귀 방지 — 실측 통상 0~10건).
        let verified_base: Option<String> = match (exists, disk.as_deref(), mh) {
            (true, Some(d), Some(m)) if d != content => load_verified_base(dir, rel, m),
            _ => None,
        };
        let facts = ctx.facts(rel, content, exists, disk.as_deref(), mh, &pending_now);
        match decide_file_action_with(
            rel,
            content,
            exists,
            disk.as_deref(),
            mh,
            force,
            scope,
            verified_base.as_deref(),
            &facts,
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
            FileAction::UserKept { .. } => plan.user_kept.push(rel.to_string()),
            FileAction::VendorFirst { .. } => plan.vendor_first.push(rel.to_string()),
            FileAction::KeepAbsent { .. } => plan.keep_absent.push(rel.to_string()),
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
            match read_start_live(&dir.join(rel)).text().ok_or(()) {
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
    let Some(disk) = read_start_meta(&dir.join(PACK_VERSION_FILE)).text() else {
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
/// ★T2h(RULINGS CW1): 링크 · 재해석 지점인가(no-follow 메타데이터).
fn meta_is_link_like(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

/// ★T2h(RULINGS CW1 · 검토자 B1): 팩 동사의 **쓰기 자리 하나**의 경로 규칙 — `root` 아래 `rel_path` 의 이미 있는 구성 요소를
/// 루트에서부터 링크를 따르지 않고 본다: 어느 구성 요소든 링크(윈도우는 모든 재해석 지점)면 거부, 부모가 폴더가 아니면 거부,
/// 마지막 성분이 폴더면 거부. 없는 구성 요소에서 멈춘다(그 아래는 아무것도 없다 — 쓰기가 새로 만든다). 부수 자리(`.pristine/…` ·
/// `<rel>.user` · `<rel>.new` · `<rel>.base` · 장부)가 살아 있는 트리의 별칭이 되어 '살아 있는 바이트를 쓰지 않는' 예외가
/// 등재 파일의 쓰기로 바뀌는 길을 막는다.
pub fn sidecar_path_gate(root: &Path, rel_path: &str) -> Result<(), String> {
    let parts: Vec<&str> = rel_path.split(['/', '\\']).filter(|p| !p.is_empty() && *p != ".").collect();
    if parts.is_empty() || parts.contains(&"..") {
        return Err(format!("쓰기 자리 {rel_path} 가 정규형이 아닙니다"));
    }
    let mut at = root.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        at.push(part);
        let meta = match std::fs::symlink_metadata(&at) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("쓰기 자리 {} 를 확인할 수 없습니다({e})", parts[..=i].join("/"))),
        };
        if meta_is_link_like(&meta) {
            return Err(format!(
                "쓰기 자리 {} 가 링크(또는 연결 지점 · 재해석 지점)입니다 — 팩 동사는 링크를 거친 자리에 쓰지 않습니다",
                parts[..=i].join("/")
            ));
        }
        if i + 1 < parts.len() && !meta.is_dir() {
            return Err(format!("쓰기 자리 {} 의 상위 {} 가 폴더가 아닙니다", rel_path, parts[..=i].join("/")));
        }
        if i + 1 == parts.len() && meta.is_dir() {
            return Err(format!("쓰기 자리 {rel_path} 가 폴더입니다"));
        }
    }
    Ok(())
}

/// ★T2h(RULINGS CW1): `root` 아래 `rel_path` 에 **닻을 내린** 원자 쓰기(교체). 본문은 [`beneath_write`] 하나다(RULINGS EC4).
pub fn write_beneath(root: &Path, rel_path: &str, bytes: &[u8]) -> std::io::Result<()> {
    beneath_write(root, rel_path, bytes, None, false, Place::Replace, false)
}

/// ★T2i(RULINGS EC2 · EC4): 닻을 내린 쓰기가 마지막 이름에 놓는 방식 — `Replace` = 원자 교체(마지막 성분이 링크면 링크
/// 자체가 바뀐다), `NoReplace` = 그 이름이 비어 있을 때만(차 있으면 `AlreadyExists` · 아무것도 바뀌지 않음).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Replace,
    NoReplace,
}

fn rel_parts(rel_path: &str) -> std::io::Result<Vec<&str>> {
    let parts: Vec<&str> = rel_path.split(['/', '\\']).filter(|p| !p.is_empty() && *p != ".").collect();
    if parts.is_empty() || parts.contains(&"..") {
        return Err(std::io::Error::other(format!("쓰기 자리 {rel_path} 가 정규형이 아닙니다")));
    }
    Ok(parts)
}

#[cfg(unix)]
struct DirFd(libc::c_int);

#[cfg(unix)]
impl Drop for DirFd {
    fn drop(&mut self) {
        unsafe { libc::close(self.0) };
    }
}

#[cfg(unix)]
fn c_name(s: &str) -> std::io::Result<std::ffi::CString> {
    std::ffi::CString::new(s.as_bytes()).map_err(|e| std::io::Error::other(e.to_string()))
}

/// ★T2i(RULINGS EC4 (4)): unix — `root` 를 연 기술자에서 구성 요소마다 `openat(O_DIRECTORY | O_NOFOLLOW)` 로 내려간다(`mkdirs` 면
/// 없는 폴더를 `mkdirat`). 사실(링크 아님 · 폴더임)은 연 기술자에서 나온다 — 검사와 쓰기 사이에 구성 요소가 링크로 바뀌면 실패.
/// 없는 구성 요소(`mkdirs` 아님) = `NotFound`.
#[cfg(unix)]
fn anchor_dir(root: &Path, comps: &[&str], mkdirs: bool, rel_path: &str) -> std::io::Result<DirFd> {
    use std::os::unix::ffi::OsStrExt;
    let root_c = std::ffi::CString::new(root.as_os_str().as_bytes()).map_err(|e| std::io::Error::other(e.to_string()))?;
    let fd = unsafe { libc::open(root_c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut d = DirFd(fd);
    for comp in comps {
        let c = c_name(comp)?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let mut nfd = unsafe { libc::openat(d.0, c.as_ptr(), flags) };
        let mut err = std::io::Error::last_os_error();
        if nfd < 0 && mkdirs && err.raw_os_error() == Some(libc::ENOENT) {
            let made = unsafe { libc::mkdirat(d.0, c.as_ptr(), 0o755 as libc::mode_t) };
            if made != 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() != Some(libc::EEXIST) {
                    return Err(e);
                }
            }
            nfd = unsafe { libc::openat(d.0, c.as_ptr(), flags) };
            err = std::io::Error::last_os_error();
        }
        if nfd < 0 {
            if err.raw_os_error() == Some(libc::ENOENT) {
                return Err(err);
            }
            return Err(std::io::Error::other(format!(
                "쓰기 자리 {rel_path} 의 구성 요소 {comp} 가 링크이거나 폴더가 아닙니다({err})"
            )));
        }
        d = DirFd(nfd);
    }
    Ok(d)
}

/// ★T2i(RULINGS EC2): 같은 폴더 기술자 안의 **덮지 않는** 이름 바꾸기 — macOS `renameatx_np(RENAME_EXCL)`, 리눅스(glibc)·안드로이드
/// `renameat2(RENAME_NOREPLACE)`. 대상 이름이 차 있으면 `AlreadyExists`(아무것도 바뀌지 않음). 파일 시스템·운영체제가 지원하지
/// 않으면(`EINVAL` · `ENOTSUP` · `ENOSYS`) `Unsupported` — 호출부는 놓지 않는다(보관 불가 갈래).
#[cfg(unix)]
fn no_replace_renameat(fd: libc::c_int, from: &std::ffi::CStr, to: &std::ffi::CStr) -> std::io::Result<()> {
    #[cfg(test)]
    {
        if FAIL_NO_REPLACE_RENAME.with(|f| f.get()) {
            return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "injected: no-replace rename unsupported"));
        }
    }
    #[cfg(target_os = "macos")]
    let r = unsafe { libc::renameatx_np(fd, from.as_ptr(), fd, to.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(any(all(target_os = "linux", target_env = "gnu"), target_os = "android"))]
    let r = unsafe { libc::renameat2(fd, from.as_ptr(), fd, to.as_ptr(), libc::RENAME_NOREPLACE) };
    #[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"), target_os = "android")))]
    let r: libc::c_int = {
        let _ = (fd, from, to);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "이 운영체제에서는 덮지 않는 이름 바꾸기를 쓸 수 없습니다",
        ));
    };
    if r == 0 {
        return Ok(());
    }
    let e = std::io::Error::last_os_error();
    let c = e.raw_os_error();
    if c == Some(libc::EINVAL) || c == Some(libc::ENOTSUP) || c == Some(libc::EOPNOTSUPP) || c == Some(libc::ENOSYS) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("이 파일 시스템은 덮지 않는 이름 바꾸기를 지원하지 않습니다({e})"),
        ));
    }
    Err(e)
}

/// ★T2i(RULINGS FP1): 덮지 않는 놓기의 대체 길 — 닻 내린 폴더 기술자 안에서 마지막 이름을 배타 생성(O_CREAT | O_EXCL |
/// O_NOFOLLOW)해 쓰고 내린다. 이미 있으면 AlreadyExists(아무것도 바뀌지 않음). 쓰기 · 내림이 실패하면 그 반쪽 파일은 남는다 —
/// 자리의 사본으로 다음 보관이 옮긴다(지우지 않는다: 지우는 사이에 다른 쓰는 쪽이 같은 이름을 만들었을 수 있다).
#[cfg(unix)]
fn exclusive_create_at(d: &DirFd, leaf_c: &std::ffi::CStr, bytes: &[u8], mode: Option<u32>, exact: bool) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::io::FromRawFd;
    #[cfg(test)]
    {
        if FAIL_EXCLUSIVE_FALLBACK.with(|f| f.get()) {
            return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "injected: exclusive creation refused"));
        }
    }
    let fd = unsafe {
        libc::openat(
            d.0,
            leaf_c.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode.unwrap_or(0o644) as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
    f.write_all(bytes)?;
    if exact {
        if let Some(m) = mode {
            apply_mode_exact(&f, m)?;
        }
    }
    sync_file_checked(&f, &leaf_c.to_string_lossy())
}

/// ★T2i(RULINGS GB2): 자리에 이미 있는 같은 바이트의 사본을 '보관된 사본'으로 다시 쓸 때 — 그 **파일**을 내린다(닻 내린 판독
/// 기술자 · 링크 비추종 · 일반 파일만). 실패는 실패(내렸다고 가정하지 않는다). 윈도우: 내림(`FlushFileBuffers`)은 쓰기 권한
/// 핸들이 필요해 쓰기 권한으로 연다(바이트는 쓰지 않는다 · 재해석 지점 비추종 · 일반 파일만).
fn beneath_sync_file(root: &Path, rel_path: &str) -> std::io::Result<()> {
    let parts = rel_parts(rel_path)?;
    let leaf = parts[parts.len() - 1];
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        let d = anchor_dir(root, &parts[..parts.len() - 1], false, rel_path)?;
        let c = c_name(leaf)?;
        let fd = unsafe { libc::openat(d.0, c.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let f = unsafe { std::fs::File::from_raw_fd(fd) };
        if !f.metadata()?.is_file() {
            return Err(std::io::Error::other(format!("{rel_path} 가 일반 파일이 아닙니다")));
        }
        sync_file_checked(&f, leaf)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        sidecar_path_gate(root, rel_path).map_err(std::io::Error::other)?;
        let path = parts.iter().fold(root.to_path_buf(), |p, c| p.join(c));
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)?;
        if !f.metadata()?.is_file() {
            return Err(std::io::Error::other(format!("{rel_path} 가 일반 파일이 아닙니다")));
        }
        sync_file_checked(&f, leaf)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (root, leaf);
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "파일 내림을 이 운영체제에서 쓸 수 없습니다"))
    }
}

/// ★T2i(RULINGS EC7 · EC2): 닻 내린 폴더 기술자의 fsync — 시험 주입(`FAIL_DIR_SYNC_FOR`, 마지막 이름 단위) · '지원하지 않음'만 Ok.
#[cfg(unix)]
fn sync_dir_fd(d: &DirFd, leaf: &str) -> std::io::Result<()> {
    #[cfg(test)]
    {
        if FAIL_DIR_SYNC_FOR.with(|f| f.borrow().as_deref() == Some(leaf)) {
            return Err(std::io::Error::other("injected directory flush failure"));
        }
    }
    let _ = leaf;
    if unsafe { libc::fsync(d.0) } == 0 {
        return Ok(());
    }
    let e = std::io::Error::last_os_error();
    if dir_sync_unsupported(&e) {
        Ok(())
    } else {
        Err(e)
    }
}

/// ★T2i(RULINGS EC4 (4) · 윈도우): 배타적으로 만든 파일의 **핸들**이 뜻한 자리에 있는가 — `GetFinalPathNameByHandleW` 의 답이
/// `canonicalize(root)` + 구성 요소 + 이름과 같아야 한다(대소문자 무시). 중간 구성 요소가 연결 지점·재해석 지점이면 답이 달라져
/// 거부된다(루트 자체의 연결은 canonicalize 가 풀므로 허용). `NtCreateFile` 의 핸들 상대 생성은 쓰지 않는다(RULINGS EF).
#[cfg(windows)]
fn windows_handle_sits_at(f: &std::fs::File, root: &Path, comps: &[&str], name: &str) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let mut buf = vec![0u16; 1024];
    loop {
        let n = unsafe { GetFinalPathNameByHandleW(f.as_raw_handle() as _, buf.as_mut_ptr(), buf.len() as u32, 0) } as usize;
        if n == 0 {
            return Err(std::io::Error::last_os_error());
        }
        if n < buf.len() {
            buf.truncate(n);
            break;
        }
        buf.resize(n + 1, 0);
    }
    let got = String::from_utf16_lossy(&buf);
    let mut want = std::fs::canonicalize(root)?;
    for c in comps {
        want.push(c);
    }
    want.push(name);
    let norm = |s: &str| s.trim_start_matches(r"\\?\").replace('/', "\\").to_lowercase();
    if norm(&got) != norm(&want.to_string_lossy()) {
        return Err(std::io::Error::other(format!(
            "만든 임시 파일이 뜻한 자리에 있지 않습니다({got}) — 연결 지점·재해석 지점을 거친 자리에는 쓰지 않습니다"
        )));
    }
    Ok(())
}

/// ★T2i(RULINGS FP5 m6): 절대 경로를 verbatim(`\\?\` · UNC 는 `\\?\UNC\`) 넓은 문자열로 — 이미 붙었거나 상대 경로면 그대로 ·
/// `/` 는 `\` 로(verbatim 경로는 정규화되지 않는다 · 구성 요소는 `rel_parts` 가 이미 정규형으로 만들었다).
#[cfg(windows)]
fn windows_verbatim(p: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut w: Vec<u16> = p.as_os_str().encode_wide().map(|c| if c == u16::from(b'/') { u16::from(b'\\') } else { c }).collect();
    let bs = u16::from(b'\\');
    let starts = |w: &[u16], s: &str| w.len() >= s.len() && w.iter().zip(s.encode_utf16()).all(|(a, b)| *a == b);
    if !starts(&w, r"\\?\") && p.is_absolute() {
        if starts(&w, r"\\") {
            let rest: Vec<u16> = w[2..].to_vec();
            w = r"\\?\UNC\".encode_utf16().chain(rest).collect();
        } else {
            w = r"\\?\".encode_utf16().chain(w).collect();
        }
    }
    let _ = bs;
    w.push(0);
    w
}

/// ★T2i(RULINGS EC2 · 윈도우): `MoveFileExW` 덮지 않는 이동(`MOVEFILE_REPLACE_EXISTING` 없음 · `MOVEFILE_WRITE_THROUGH`) — 차 있으면
/// `AlreadyExists`(ERROR_FILE_EXISTS 80 · ERROR_ALREADY_EXISTS 183).
#[cfg(windows)]
fn windows_move_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    // ★T2i(RULINGS FP5 m6): 긴 경로 — std 의 이름 바꾸기처럼 절대 경로에 `\\?\` (UNC 는 `\\?\UNC\`) 를 붙인다.
    let (a, b) = (windows_verbatim(from), windows_verbatim(to));
    let ok = unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if ok != 0 {
        return Ok(());
    }
    let e = std::io::Error::last_os_error();
    // ★T2i(RULINGS FP5 m6): 대상 이름에 무엇이든(파일 · 폴더 · 재해석 지점) 있으면 '차 있음'(다음 세대 자리로).
    if matches!(e.raw_os_error(), Some(80) | Some(183)) || std::fs::symlink_metadata(to).is_ok() {
        return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
    }
    // ★T2i(RULINGS FP1): ERROR_INVALID_FUNCTION(1) · ERROR_NOT_SUPPORTED(50) · ERROR_INVALID_PARAMETER(87) = 이 볼륨은 지원하지 않음.
    if matches!(e.raw_os_error(), Some(1) | Some(50) | Some(87)) {
        return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, e));
    }
    Err(e)
}

/// ★T2i(RULINGS EC4 · EC2): **팩 동사의 쓰기 하나** — (1) 새 바이트는 이 쓰는 쪽이 **배타적으로 만든**(create-new · 링크 비추종)
/// 임시 파일에만 쓰고 마지막 이름에는 놓기로만 닿는다, (2) 이미 있는 임시 이름은 열지 않는다(다음 이름 · 고갈 = 거부), (4) 사실은
/// 연 핸들에서: unix = 닻 내린 폴더 기술자([`anchor_dir`]) 안의 `openat(O_CREAT | O_EXCL | O_NOFOLLOW)` · `renameat` /
/// [`no_replace_renameat`]; 윈도우 = 경로 규칙([`sidecar_path_gate`]) 뒤 배타 생성 → 핸들 증명([`windows_handle_sits_at`]) →
/// 놓기(`Replace` = std 이름 바꾸기 · `NoReplace` = [`windows_move_no_replace`]). `mode` = 만들 때의 권한(`exact` 면 `fchmod` + 되읽기
/// 검증 — H-08), `durable` = 놓은 뒤 부모 폴더 fsync 실패를 돌려준다(EC7). 실패하면 임시 파일을 지우고 아무것도 바꾸지 않는다.
pub fn beneath_write(
    root: &Path,
    rel_path: &str,
    bytes: &[u8],
    mode: Option<u32>,
    exact: bool,
    place: Place,
    durable: bool,
) -> std::io::Result<()> {
    let parts = rel_parts(rel_path)?;
    let leaf = parts[parts.len() - 1];
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::io::FromRawFd;
        let d = anchor_dir(root, &parts[..parts.len() - 1], true, rel_path)?;
        let leaf_c = c_name(leaf)?;
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let mut made: Option<(libc::c_int, std::ffi::CString)> = None;
        for _ in 0..crate::TMP_NAME_TRIES {
            let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let tmp_c = c_name(&format!(".{leaf}.tmp-{}-{n}", std::process::id()))?;
            let tfd = unsafe {
                libc::openat(
                    d.0,
                    tmp_c.as_ptr(),
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    mode.unwrap_or(0o644) as libc::c_uint,
                )
            };
            if tfd >= 0 {
                made = Some((tfd, tmp_c));
                break;
            }
            let e = std::io::Error::last_os_error();
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(e);
            }
        }
        let Some((tfd, tmp_c)) = made else {
            return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "임시 이름 고갈 — 아무것도 쓰지 않았습니다"));
        };
        let mut f = unsafe { std::fs::File::from_raw_fd(tfd) };
        let res = (|| -> std::io::Result<()> {
            f.write_all(bytes)?;
            if exact {
                if let Some(m) = mode {
                    apply_mode_exact(&f, m)?;
                }
            }
            f.sync_all()?;
            match place {
                Place::Replace => {
                    if unsafe { libc::renameat(d.0, tmp_c.as_ptr(), d.0, leaf_c.as_ptr()) } != 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                }
                Place::NoReplace => no_replace_renameat(d.0, &tmp_c, &leaf_c),
            }
        })();
        drop(f);
        match res {
            Ok(()) => {}
            // ★T2i(RULINGS FP1): 덮지 않는 이름 바꾸기를 이 파일 시스템이 지원하지 않으면 — 다른 길의 덮지 않는 생성: 마지막
            //   이름을 **배타적으로 만들고**(O_CREAT | O_EXCL | O_NOFOLLOW · 차 있으면 AlreadyExists) 그 자리에 쓰고 내린다. 쓰다 멈춘
            //   사본은 자리의 사본(occupant)이 되어 다음 보관이 세대 자리로 옮긴다(살아 있는 파일은 보관이 끝난 뒤에만 바뀐다).
            Err(e) if e.kind() == std::io::ErrorKind::Unsupported && place == Place::NoReplace => {
                unsafe { libc::unlinkat(d.0, tmp_c.as_ptr(), 0) };
                exclusive_create_at(&d, &leaf_c, bytes, mode, exact)?;
            }
            Err(e) => {
                unsafe { libc::unlinkat(d.0, tmp_c.as_ptr(), 0) };
                return Err(e);
            }
        }
        if durable {
            sync_dir_fd(&d, leaf)?;
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::io::Write;
        sidecar_path_gate(root, rel_path).map_err(std::io::Error::other)?;
        let dest = parts.iter().fold(root.to_path_buf(), |p, c| p.join(c));
        let parent = dest.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
        std::fs::create_dir_all(&parent)?;
        let mut made: Option<(PathBuf, std::fs::File)> = None;
        for _ in 0..crate::TMP_NAME_TRIES {
            let cand = crate::tmp_path_for(&parent, leaf);
            match create_tmp_with_mode(&cand, None) {
                Ok(f) => {
                    made = Some((cand, f));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        let Some((tmp, mut f)) = made else {
            return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "임시 이름 고갈 — 아무것도 쓰지 않았습니다"));
        };
        let tmp_name = tmp.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let res = (|| -> std::io::Result<()> {
            windows_handle_sits_at(&f, root, &parts[..parts.len() - 1], &tmp_name)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            Ok(())
        })();
        drop(f);
        let res = res.and_then(|_| match place {
            Place::Replace => std::fs::rename(&tmp, &dest),
            Place::NoReplace => windows_move_no_replace(&tmp, &dest),
        });
        match res {
            Ok(()) => {}
            // ★T2i(RULINGS FP1): 덮지 않는 이동을 지원하지 않는 볼륨 — 마지막 이름을 배타 생성(CREATE_NEW)해 쓰고 내린다(핸들 증명 포함).
            Err(e) if place == Place::NoReplace && e.kind() == std::io::ErrorKind::Unsupported => {
                let _ = std::fs::remove_file(&tmp);
                let mut g = create_tmp_with_mode(&dest, None)?;
                windows_handle_sits_at(&g, root, &parts[..parts.len() - 1], leaf)?;
                g.write_all(bytes)?;
                g.sync_all()?;
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        }
        let _ = (mode, exact, durable);
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (root, bytes, mode, exact, place, durable, leaf);
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "닻 내린 쓰기를 이 운영체제에서 쓸 수 없습니다"))
    }
}

/// ★T2i(RULINGS EC4 (4)): 팩 **대상** 파일의 닻 내린 쓰기 — [`write_pack_file`] 의 권한 규칙(H-08 `pack_target_mode` · 정확 적용)
/// 그대로, 쓰기는 [`beneath_write`](`place` — 동사가 고른다: 교체 · `--missing-only` 는 덮지 않음).
pub fn write_pack_file_beneath(
    root: &Path,
    rel_path: &str,
    bytes: &[u8],
    prior: Option<u32>,
    system_manifest_entry: bool,
    place: Place,
) -> std::io::Result<()> {
    let mode = pack_target_mode(prior, bytes, system_manifest_entry);
    beneath_write(root, rel_path, bytes, mode, true, place, false)
}

/// ★T2i(RULINGS EC2): 같은 폴더 안의 **덮지 않는** 이동(보관본을 세대 자리로) — unix 는 닻 내린 기술자 안의
/// [`no_replace_renameat`], 윈도우는 경로 규칙 뒤 [`windows_move_no_replace`]. 원본 없음 = `NotFound`, 대상 있음 = `AlreadyExists`.
pub fn beneath_move_no_replace(root: &Path, from_rel: &str, to_rel: &str) -> std::io::Result<()> {
    let fp = rel_parts(from_rel)?;
    let tp = rel_parts(to_rel)?;
    if fp[..fp.len() - 1] != tp[..tp.len() - 1] {
        return Err(std::io::Error::other(format!("{from_rel} → {to_rel}: 같은 폴더 안에서만 옮깁니다")));
    }
    #[cfg(unix)]
    {
        let d = anchor_dir(root, &fp[..fp.len() - 1], false, from_rel)?;
        no_replace_renameat(d.0, &c_name(fp[fp.len() - 1])?, &c_name(tp[tp.len() - 1])?)
    }
    #[cfg(windows)]
    {
        // 경로 규칙은 **이름 전체**에(부모 폴더만 재면 그 폴더 자체가 '폴더입니다'로 거부되고, 팩 루트의 파일은 빈 경로로 거부된다).
        sidecar_path_gate(root, from_rel).map_err(std::io::Error::other)?;
        sidecar_path_gate(root, to_rel).map_err(std::io::Error::other)?;
        let base = fp[..fp.len() - 1].iter().fold(root.to_path_buf(), |p, c| p.join(c));
        windows_move_no_replace(&base.join(fp[fp.len() - 1]), &base.join(tp[tp.len() - 1]))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (root, fp, tp);
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "덮지 않는 이동을 이 운영체제에서 쓸 수 없습니다"))
    }
}

/// ★T2i(RULINGS FP5 m7): 보관본(`<rel>.user` · `<rel>.new`)의 정체 — 링크를 따르지 않는 메타데이터의 (장치 · 아이노드 · 크기 ·
/// 수정 시각 · 바뀐 시각). 윈도우는 (크기 · 마지막 쓰기 · 만든 시각 · 속성). 지울 때 읽거나 확인한 그 사본인지 대조한다.
pub fn side_identity(path: &Path) -> Option<(u64, u64, u64, i64, i64)> {
    let m = std::fs::symlink_metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((m.dev(), m.ino(), m.size(), m.mtime() * 1_000_000_000 + m.mtime_nsec(), m.ctime() * 1_000_000_000 + m.ctime_nsec()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Some((u64::from(m.file_attributes()), 0, m.file_size(), m.last_write_time() as i64, m.creation_time() as i64))
    }
    #[cfg(not(any(unix, windows)))]
    {
        Some((0, 0, m.len(), 0, 0))
    }
}

/// ★T2i(RULINGS EC2): 보관 자리(`<rel>.user`)에 지금 있는 것 — 닻 내린 판독(링크 비추종 · 막히지 않음 · 일반 파일 · 상한).
#[derive(Debug)]
enum Occupant {
    Missing,
    /// 일반 파일 — 바이트(상한 초과 · 판독 실패 = None: 비교할 수 없으니 '다른 사본'으로 다룬다).
    File(Option<Vec<u8>>),
    /// 파일이 아닌 것(폴더 · 링크 · FIFO …) 또는 자리를 확인할 수 없음 — 세대가 아니라 장애물.
    Obstacle(String),
}

fn beneath_occupant(root: &Path, rel_path: &str) -> Occupant {
    let parts = match rel_parts(rel_path) {
        Ok(p) => p,
        Err(e) => return Occupant::Obstacle(e.to_string()),
    };
    #[cfg(unix)]
    {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        let d = match anchor_dir(root, &parts[..parts.len() - 1], false, rel_path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Occupant::Missing,
            Err(e) => return Occupant::Obstacle(e.to_string()),
        };
        let leaf_c = match c_name(parts[parts.len() - 1]) {
            Ok(c) => c,
            Err(e) => return Occupant::Obstacle(e.to_string()),
        };
        let fd = unsafe {
            libc::openat(d.0, leaf_c.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        };
        if fd < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::ENOENT) {
                return Occupant::Missing;
            }
            // ★T2i(RULINGS FP5 m8): 열지 못한 이유를 닻 내린 lstat 으로 가른다 — 읽을 수 없는 **일반 파일**은 '파일이 아님'이 아니라
            //   '판독 불가 사본'(비교할 수 없으니 다른 사본으로 다뤄 세대 자리로 옮긴다 · 옮기기는 읽기 권한이 필요 없다).
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::fstatat(d.0, leaf_c.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } == 0 {
                return match st.st_mode & libc::S_IFMT {
                    libc::S_IFREG => Occupant::File(None),
                    libc::S_IFLNK => Occupant::Obstacle("링크".into()),
                    libc::S_IFDIR => Occupant::Obstacle("폴더".into()),
                    _ => Occupant::Obstacle(format!("특수 파일({e})")),
                };
            }
            return Occupant::Obstacle(e.to_string());
        }
        let f = unsafe { std::fs::File::from_raw_fd(fd) };
        let Ok(m) = f.metadata() else { return Occupant::Obstacle("상태 확인 실패".into()) };
        if m.is_dir() {
            return Occupant::Obstacle("폴더".into());
        }
        if !m.is_file() {
            return Occupant::Obstacle("특수 파일(FIFO · 장치 · 소켓)".into());
        }
        if m.len() > START_META_CAP {
            return Occupant::File(None);
        }
        let mut b = Vec::new();
        match f.take(START_META_CAP + 1).read_to_end(&mut b) {
            Ok(_) => Occupant::File(Some(b)),
            Err(_) => Occupant::File(None),
        }
    }
    #[cfg(not(unix))]
    {
        // 경로 규칙은 이름 전체에(링크 · 폴더인 자리는 장애물 · 없으면 통과).
        if let Err(e) = sidecar_path_gate(root, rel_path) {
            return Occupant::Obstacle(e);
        }
        let p = parts.iter().fold(root.to_path_buf(), |p, c| p.join(c));
        match std::fs::symlink_metadata(&p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Occupant::Missing,
            Err(e) => Occupant::Obstacle(e.to_string()),
            Ok(m) if meta_is_link_like(&m) || !m.is_file() => Occupant::Obstacle("일반 파일이 아님".into()),
            Ok(_) => match read_start_meta(&p) {
                StartRead::Bytes(b) => Occupant::File(Some(b)),
                StartRead::Missing => Occupant::Missing,
                StartRead::Unreadable(_) => Occupant::File(None),
            },
        }
    }
}

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
    // 거동 불변: enforce=false 는 종전과 **바이트 동일**하다(mode 는 open(2) 인자로만 간다).
    write_atomic_mode_inner(path, bytes, mode, false, apply_mode_exact, false)
}

/// ★T2i(RULINGS EC7): **영속이 증명된** 원자 쓰기 — 본문 fsync → 원자 교체 → 부모 폴더 fsync, 그리고 그 셋의 실패를 **모두**
/// 돌려준다(종전 [`write_atomic`] 은 폴더 fsync 실패를 버렸다). 너그럽게 넘기는 것은 '폴더 fsync 를 지원하지 않음' 하나다
/// ([`dir_sync_unsupported`] — `Unsupported` · `EINVAL` · `ENOTSUP`/`EOPNOTSUPP`). vendor 우선의 선저장(보관본 `<rel>.user` 와
/// 원장)이 이것을 쓴다 — 살아 있는 기동 필수 파일은 이 둘이 디스크에 남았음이 증명된 뒤에만 바뀐다.
/// 윈도우: 본문은 `FlushFileBuffers`(`sync_all`)로 내리고, 폴더는 std 로 열어 내릴 수 없다(`FILE_FLAG_BACKUP_SEMANTICS` 필요) —
/// '지원하지 않음'으로 다룬다(교체의 영속은 NTFS 메타데이터 기록에 기댄다 — 추정).
pub fn write_atomic_durable(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_atomic_mode_inner(path, bytes, None, false, apply_mode_exact, true)
}

/// ★T2i(RULINGS EC7): 폴더 fsync 가 '지원하지 않음'으로 실패했는가(그때만 너그럽게 넘긴다).
fn dir_sync_unsupported(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::Unsupported {
        return true;
    }
    #[cfg(unix)]
    {
        let c = e.raw_os_error();
        c == Some(libc::EINVAL) || c == Some(libc::ENOTSUP) || c == Some(libc::EOPNOTSUPP)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

#[cfg(test)]
thread_local! {
    /// ★T2i(RULINGS EC7) 시험 주입: 이 파일 이름의 영속 쓰기에서 부모 폴더 fsync 가 실패한 것으로 한다(스레드 단위 — 다른 시험과
    /// 섞이지 않는다).
    static FAIL_DIR_SYNC_FOR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// ★T2i(RULINGS FP1) 시험 주입: 덮지 않는 이름 바꾸기가 '지원하지 않음'(EINVAL 같은 파일 시스템)인 것으로 한다.
    static FAIL_NO_REPLACE_RENAME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// ★T2i(RULINGS FP1) 시험 주입: 그 대체 길(마지막 이름의 배타 생성)도 실패한 것으로 한다.
    static FAIL_EXCLUSIVE_FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// ★T2i(RULINGS GB2) 시험 주입: 이 이름의 파일 내림(fsync)이 실패한 것으로 한다(배타 생성의 대체 길 · 자리의 같은 사본 다시 내림).
    static FAIL_FILE_SYNC_FOR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// ★T2i(RULINGS GB2): 파일 하나의 내림 — 시험 주입 · '지원하지 않음'만 Ok(EC7 의 규칙 그대로), 그 밖의 실패는 실패.
fn sync_file_checked(f: &std::fs::File, leaf: &str) -> std::io::Result<()> {
    #[cfg(test)]
    {
        if FAIL_FILE_SYNC_FOR.with(|x| x.borrow().as_deref() == Some(leaf)) {
            return Err(std::io::Error::other("injected file flush failure"));
        }
    }
    let _ = leaf;
    match f.sync_all() {
        Err(e) if e.kind() == std::io::ErrorKind::Unsupported => Ok(()),
        other => other,
    }
}

/// ★T2i(RULINGS EC7): 영속 쓰기의 부모 폴더 fsync — 실패를 돌려준다('지원하지 않음'만 Ok).
fn sync_parent_dir(path: &Path, parent: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        if FAIL_DIR_SYNC_FOR.with(|f| f.borrow().is_some() && *f.borrow() == name) {
            return Err(std::io::Error::other("injected directory flush failure"));
        }
    }
    let _ = path;
    #[cfg(unix)]
    {
        let d = std::fs::File::open(parent)?;
        match d.sync_all() {
            Err(e) if dir_sync_unsupported(&e) => Ok(()),
            other => other,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        Ok(())
    }
}

/// [`write_atomic_mode`] 의 **정확 적용판**(H-08).
///
/// 왜 따로 두는가: `write_atomic_mode` 는 mode 를 `OpenOptions::mode()` = **`open(2)` 의 mode 인자**로
/// 넘기고, 그 값은 **umask 로 걸러진다**(실측: umask 0111 에서 0755 요청 → 0644 · umask 0077 → 0700).
/// 그 거동은 원래 목적인 **좁히기**(`.claude.json` 0600)에는 해롭지 않다 — umask 는 비트를 **빼기만**
/// 하므로 좁히는 목표는 umask 와 충돌하지 않는다. 그러나 H-08 의 목표는 **실행비트를 주는 것**이고,
/// 주는 것은 umask 가 거부할 수 있다. 그래서 이 판은 tmp **디스크립터에 fchmod** 로 적용하고
/// (chmod(2) 는 umask 무관 — `install_into` 의 실행비트 루프가 이미 `set_permissions` 를 쓰는 이유다)
/// **rename 전에 fstat 으로 검증**한다. 검증 실패·적용 실패는 **대상을 교체하지 않고** 그대로 올린다.
pub fn write_atomic_mode_exact(path: &Path, bytes: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    write_atomic_mode_inner(path, bytes, mode, true, apply_mode_exact, false)
}

fn write_atomic_mode_inner(
    path: &Path,
    bytes: &[u8],
    mode: Option<u32>,
    enforce: bool,
    // ★주입 가능한 적용·검증 수단(기본은 `apply_mode_exact`). 제네릭이 아니라 **함수 포인터**인
    //   이유는 H-CONC-3 핀이 소유자를 `fn <이름>(` 문자열로 해소하기 때문이다 — 제네릭을 붙이면
    //   정의가 `fn write_atomic_mode_inner<A>(` 가 되어 핀이 소유자를 놓친다(실측된 사각).
    apply: fn(&std::fs::File, u32) -> std::io::Result<()>,
    // ★T2i(RULINGS EC7): true = 부모 폴더 fsync 실패를 돌려준다([`write_atomic_durable`]) · false = 종전(최선 노력).
    durable: bool,
) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    let fname = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    // ★H-CONC-3(2026-09-05): tmp 이름은 **호출마다 유일**해야 한다.
    //   종전 이름은 `.{fname}.tmp.{pid}` 로 **pid 당 하나로 고정**이었다. 프로세스가 다르면
    //   이름이 갈려 안전하므로 다중 **프로세스** 하네스(6 writer)는 파손 0 을 냈지만, 같은
    //   프로세스의 두 호출(데몬의 연결별 tokio task · GUI setup 의 병행 시드)은 **같은 tmp
    //   inode** 를 공유했다. `File::create` 는 O_TRUNC 라 뒤 호출이 앞 호출의 본문을 0 으로
    //   자르고 짧은 문서를 쓰며, 앞 호출이 자기 오프셋(=긴 문서 끝)에서 꼬리를 마저 쓰면
    //   **짧은 문서 + 구멍 + 긴 꼬리**가 rename 된다 → 소비자는 `Extra data: line 1 column NN`
    //   으로 죽는다(원자 교체는 *프로세스 간* 파손만 막지 이 축을 막지 못한다).
    //   `lib.rs::atomic_write_bytes`(A13)가 이미 같은 계약을 문서화해 두었으므로 이름 생성기
    //   `crate::tmp_path_for` 를 **공유**한다 — 사본을 만들지 않는다(이 저장소가 반복해서 맞은
    //   것이 사본 드리프트다).
    let (tmp, mut f) = {
        let mut last: Option<std::io::Error> = None;
        let mut got: Option<(PathBuf, std::fs::File)> = None;
        for _ in 0..crate::TMP_NAME_TRIES {
            let cand = crate::tmp_path_for(parent, fname);
            match create_tmp_with_mode(&cand, mode) {
                Ok(f) => {
                    got = Some((cand, f));
                    break;
                }
                // 이름 충돌은 **다음 후보로** — 남의 tmp 를 지우고 열지 않는다(진행 중인 남의
                // 쓰기를 파손하는 경로다 · A13 주석의 같은 금지).
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        match got {
            Some(v) => v,
            None => {
                return Err(last.unwrap_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::AlreadyExists, "tmp 이름 고갈")
                }))
            }
        }
    };
    let res = (|| -> std::io::Result<()> {
        f.write_all(bytes)?;
        f.sync_all()?; // 파일 본문 fsync (rename 전)
        // ★H-08: enforce 면 **rename 전에** 목표 mode 를 디스크립터에 적용·검증한다. 여기서 실패하면
        //   아래 Err 처리가 tmp 를 지우므로 **대상은 손대지 않은 채** 오류가 올라간다(비실행 스크립트를
        //   조용히 출하하는 것보다 시끄럽게 실패하는 쪽이 옳다).
        if enforce {
            if let Some(m) = mode {
                apply(&f, m)?;
            }
        }
        std::fs::rename(&tmp, path)?; // 원자 교체
        Ok(())
    })();
    match res {
        Ok(()) if durable => sync_parent_dir(path, parent),
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

/// tmp 파일 생성 — `mode=Some(m)` 은 `open(2)` 의 mode 인자로 가고 **umask 로 걸러진다**
/// (실효값 `m & !umask`). 그래서 이 인자만으로는 요청 비트가 선다고 말할 수 없다:
/// `umask 0111` 아래에서 `Some(0o755)` 는 `0o644` 로 생성된다.
/// ⇒ **좁히는 쪽**(예: `0o600` 으로 제한)에는 충분하지만, **실행비트를 umask 와 무관하게 세우는
/// 것**은 [`write_atomic_mode_exact`]/[`apply_mode_exact`](`fchmod` + 되읽기 검증)의 일이다.
/// `mode=None` 이면 종전과 같은 기본 퍼미션(umask 적용)이다.
/// <!-- 정정 근거: gemini-3 r1 minor — 종전 문안("생성 시점부터 그 퍼미션이다 · 사후 chmod 창 0")은
///      umask 필터링 아래에서 거짓이고 P3-DESIGN-R2 §1-1·§9-2, R3 §6 가 이미 정정을 약속했다. -->
///
/// ★H-CONC-3: 두 갈래 모두 `create_new`(O_EXCL)다 — 커널이 이름 유일성을 보증하므로 호출부가
/// 충돌을 **관측**해 다음 후보로 넘어갈 수 있다. `create`+truncate 는 충돌을 조용히 삼켜 두
/// writer 가 같은 inode 에 겹쳐 쓴다(그것이 이 결함의 기제였다). 잔여 tmp 를 지우고 여는 종전
/// 경로도 함께 사라진다 — 그것은 **남의 진행 중 쓰기**를 파손할 수 있었다.
#[cfg(unix)]
fn create_tmp_with_mode(tmp: &Path, mode: Option<u32>) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    if let Some(m) = mode {
        opts.mode(m);
    }
    opts.open(tmp)
}

#[cfg(not(unix))]
fn create_tmp_with_mode(tmp: &Path, _mode: Option<u32>) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().write(true).create_new(true).open(tmp)
}

/// tmp 디스크립터에 mode 를 **정확히** 적용하고 즉시 되읽어 검증한다(H-08).
/// `fchmod(2)` 는 umask 와 무관하므로 `open(2)` 의 mode 인자와 달리 요청값이 그대로 선다.
/// 되읽기는 mode 를 무시하는 파일시스템에서 **조용한 실패**를 막는다.
#[cfg(unix)]
fn apply_mode_exact(f: &std::fs::File, m: u32) -> std::io::Result<()> {
    apply_mode_exact_with(f, m, set_mode_fchmod, observe_mode_fstat)
}

/// 적용 수단 — `fchmod(2)`(umask 무관).
#[cfg(unix)]
fn set_mode_fchmod(f: &std::fs::File, m: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    f.set_permissions(std::fs::Permissions::from_mode(m))
}

/// 관측 수단 — `fstat(2)`(디스크립터 기준 · 경로 재해석 없음).
#[cfg(unix)]
fn observe_mode_fstat(f: &std::fs::File) -> std::io::Result<u32> {
    use std::os::unix::fs::PermissionsExt;
    Ok(f.metadata()?.permissions().mode() & 0o7777)
}

/// [`apply_mode_exact`] 의 **수단 주입판** — 적용·관측을 인자로 받아 검증 분기를 검체로 만든다.
///
/// 왜 분리하는가: 이 함수의 불일치 거부는 **mode 를 조용히 무시하는 파일시스템**에 대한 유일한
/// 방어인데, 실제 그런 파일시스템 없이는 그 분기를 밟을 수 없어 변이가 살아남았다(worker-15 자진
/// 신고 · codex r1 m1 · gemini-3 r1 major 2-a). 수단을 주입하면 검체가 "chmod 가 아무 일도 하지
/// 않는" 고장을 **주입**해 거부 경로를 끝까지(쓰기 전 Err · tmp 정리 · 대상 무변경) 밟는다.
#[cfg(unix)]
fn apply_mode_exact_with(
    f: &std::fs::File,
    m: u32,
    set: impl Fn(&std::fs::File, u32) -> std::io::Result<()>,
    observe: impl Fn(&std::fs::File) -> std::io::Result<u32>,
) -> std::io::Result<()> {
    set(f, m)?;
    let got = observe(f)?;
    if got != (m & 0o7777) {
        return Err(std::io::Error::other(format!(
            "mode 적용 검증 실패: 요청 {:04o} 실제 {:04o} — 대상 교체 안 함",
            m & 0o7777,
            got
        )));
    }
    Ok(())
}

/// Windows 에는 POSIX 실행비트가 없다 — `create_tmp_with_mode` 의 `cfg(not(unix))` 쌍과 같은 계약으로
/// mode 를 받아 **무시**한다(H-08 의 remedy 는 Windows 에서 구조적으로 no-op 이다).
#[cfg(not(unix))]
fn apply_mode_exact(_f: &std::fs::File, _m: u32) -> std::io::Result<()> {
    Ok(())
}

/// `install_into` 의 실행비트 규칙과 **같은 식**(pack.rs 의 shebang 루프) — 고정 목록은 드리프트한다.
pub fn exec_mode_for(bytes: &[u8]) -> Option<u32> {
    if bytes.starts_with(b"#!") {
        Some(0o755)
    } else {
        None
    }
}

/// 쓰기 **전** 대상의 현재 mode(없거나 읽을 수 없으면 None). 호출부는 반드시 쓰기 전에 읽어야 한다 —
/// 쓰기 뒤에 읽으면 tmp+rename 이 이미 mode 를 버린 뒤다.
pub fn current_mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// H-08 의 mode 규칙 **한 곳**(어느 호출부도 규칙을 반복하지 않는다).
///
/// - 선행 파일에 **실행비트가 하나라도** 있으면 그 mode 를 **그대로** 유지한다(의도된 0700 을 0755 로
///   넓히지 않는다).
/// - 선행 파일에 실행비트가 없고, **system 등급 매니페스트 항목**이며 내용이 `#!` 로 시작하면 **0755 로
///   복구**한다(`install_into` 와 같은 규칙 · 이전 치유가 0644 로 망가뜨린 대상을 되살린다).
/// - 그 밖의 선행 파일은 **그대로** 유지한다(사용자가 일부러 비실행으로 둔 파일은 설치기의 일이 아니다).
/// - 선행 파일이 **없으면** 내용에서 파생한다([`exec_mode_for`]).
pub fn pack_target_mode(
    prior: Option<u32>,
    bytes: &[u8],
    system_manifest_entry: bool,
) -> Option<u32> {
    match prior {
        Some(p) if p & 0o111 != 0 => Some(p),
        Some(_) if system_manifest_entry && bytes.starts_with(b"#!") => Some(0o755),
        Some(p) => Some(p),
        None => exec_mode_for(bytes),
    }
}

/// C-07 (0.14.55 · P3) 한 파일에 대한 낡은 매니페스트 항목 판정.
///
/// 이 판정은 **plan/install 분류 바로 앞**에 선다 — `decide_file_action` 의 7개 arm 은
/// **손대지 않는다**. 고치는 것은 그 함수에 들어가는 **manifest_hash 하나**이고(기준선 수리),
/// 판정 불가인 경우에만 호출부가 분류를 건너뛰고 제자리 보존을 고른다. arm 을 고치면 같은 사실을
/// 7곳에 흩뿌리게 되므로 그렇게 하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleEntry {
    /// 항목이 건전하거나 이 판정의 범위 밖 — 받은 해시 그대로 분류한다.
    Normal,
    /// 항목이 낡았고 **디스크 바이트가 표에 있는 과거 릴리스 본**이다 → 기준선을 그 해시로 수리.
    /// 그 뒤 Arm 2 가 맞아 `자동 갱신`(비수정 파일의 깨끗한 벤더 갱신)으로 분류된다.
    Repaired(String),
    /// 항목이 낡았고 디스크 바이트가 **무엇인지 말할 수 없다**(표에 행이 없거나 어느 행과도 불일치)
    /// → ⛔**제자리 보존 + 보고**. 쓰지 않고, `.user` 를 만들지 않고, 원장에 적지 않는다.
    KeepInPlace(&'static str),
}

/// [`StaleEntry`] 판정. 순수 — IO 는 호출부가 이미 읽은 값으로만 한다.
///
/// 규칙(설계 R3 section 3-2 · 보수 분기):
/// 1. system 등급이 아니면 `Normal` (user·seed-once 는 이 판정의 대상이 아니다)
/// 2. 매니페스트 항목이 없으면 `Normal` (레거시 경로 — 거동 불변)
/// 3. 항목이 디스크 해시와 같으면 `Normal` (건전)
/// 4. 표에 행이 없으면 `KeepInPlace("no-row")`
/// 5. 디스크 해시가 그 행에 있으면 `Repaired(디스크 해시)`
/// 6. 그 밖 → `KeepInPlace("unknown-disk")`
///
/// ⛔`.pristine` 조상을 근거로 3-way 병합을 고르는 확장은 **이 판에 없다**
/// (이관 D5H-20261027-P3-PRISTINE-MERGE): 디스크 바이트가 무엇인지 모르는 파일에 대해 조상이
/// 그 파일을 설명한다고 신뢰하는 것이기 때문이다.
pub fn classify_stale_entry(
    dir: &Path,
    rel: &str,
    scope: PackScope,
    embed: &str,
    disk: Option<&str>,
    manifest_hash: Option<&str>,
    has_pending_entry: bool,
) -> StaleEntry {
    // ★조상 해시는 **낡음 판정의 가드**다(이관된 pristine-Merge3 확장과 다르다): 항목이 조상과
    //   같으면 그 항목은 낡은 것이 아니라 **정상적인 드리프트/병합 상태의 기준선**이다.
    //   이 가드를 빼면 합법적인 로컬 수정(항목==조상 해시 · 건강한 3-way 경로)이 "출처 불명"으로
    //   오분류돼 3-way 병합이 사라진다(실측: lib 스위트 52건 적색).
    let pristine_hash = read_start_meta(&dir.join(PRISTINE_DIR).join(rel))
        .text()
        .map(|p| content_hash(&p));
    classify_stale_entry_with(
        rel,
        scope,
        disk,
        manifest_hash,
        pristine_hash.as_deref(),
        Some(content_hash(embed).as_str()),
        crate::pack_prev_vendor::prev_vendor_hashes(rel),
        has_pending_entry,
    )
}

/// [`classify_stale_entry`] 의 **표 주입판**. 표를 인자로 받아 완전히 순수해진다.
///
/// 왜 분리하는가: 정적 표만으로는 `Repaired` 분기를 검체로 만들 수 없다 — 그 분기를 밟으려면
/// **해시가 표의 한 행과 같은 내용**이 필요하고, 과거 릴리스의 내용은 런타임에 없다. 표를 주입하면
/// 검체가 `(내용, hash(내용))` 쌍을 스스로 만들어 여섯 분기를 전부 밟을 수 있다.
pub fn classify_stale_entry_with(
    rel: &str,
    scope: PackScope,
    disk: Option<&str>,
    manifest_hash: Option<&str>,
    pristine_hash: Option<&str>,
    embed_hash: Option<&str>,
    known: &[&str],
    has_pending_entry: bool,
) -> StaleEntry {
    if ownership_scoped(rel, scope) != Ownership::System {
        return StaleEntry::Normal;
    }
    let (Some(d), Some(mh)) = (disk, manifest_hash) else {
        return StaleEntry::Normal;
    };
    let dh = content_hash(d);
    if dh == mh {
        return StaleEntry::Normal;
    }
    // 항목이 **조상**과 같으면 낡은 것이 아니다 — 평범한 드리프트/3-way 입력이다.
    if pristine_hash == Some(mh) {
        return StaleEntry::Normal;
    }
    // ★항목이 **임베드**와 같으면 그것은 가능한 가장 새로운 값이다 = 정의상 낡지 않았다.
    //   이때 디스크가 다른 이유는 **사용자가 고쳤기 때문**이고(vendor 미전진 = L2 kept-drift),
    //   그 경로의 pinned 거동(제자리 보존 + 원장 kept-drift 계상)은 손대지 않는다.
    //   이 가드가 없으면 합법적인 L2 가 가로채여 원장 계상이 사라진다(실측: plan=actual 검체 적색).
    if embed_hash == Some(mh) {
        return StaleEntry::Normal;
    }
    // ★디스크가 **현재 임베드**와 같으면 그 바이트의 출처는 '불명'이 아니라 **이번 vendor 본**이다.
    //   표는 현재 릴리스의 blob 을 **구조적으로 제외**하므로(`scripts/gen_prev_vendor_hashes.py`
    //   `if o != oid_now:`) hash(embed) 는 어느 행에도 없고, 전진 없는 파일은 행 자체가 없다 ⇒
    //   아래 `no-row` 로 떨어져 제자리 보존이 되고, 그 결과 `decide_file_action` Arm 1
    //   (`Some(d) if d == embed` → `Keep { adopt_hash: true }`)과 그 뒤의 `manifest.insert` 가
    //   **도달 불가**가 돼 낡은 항목이 영구히 수리되지 않는다.
    //   ⇒ 여기서 `Normal` 을 돌려 Arm 1 이 설계대로 돌게 한다(codex r1 m2 · gemini-3 r1 major 1).
    //   ⛔C-07 좁힘은 그대로다: 제자리 보존은 여전히 '해시 불명 ∧ 병합 대기 없음' 일 때만이고,
    //      이 가드는 그 '불명' 집합에서 **알려진 현재 본**을 빼는 것이지 좁힘을 되돌리지 않는다.
    if embed_hash == Some(dh.as_str()) {
        return StaleEntry::Normal;
    }
    if known.contains(&dh.as_str()) {
        return StaleEntry::Repaired(dh);
    }
    // ★판정 승인(HEAD-RULING-P3-IMPL-C07-NARROW.md `9c1f081bab6f4c08`, 항목 1-2):
    //   제자리 보존은 **둘이 함께** 성립할 때만이다 — 디스크 해시가 표에 없고 **그리고** 그 경로에
    //   **병합 대기 원장 항목이 없다**. 항목이 있으면 디스크 바이트의 출처는 '불명'이 아니라
    //   **그 병합**이므로, pinned 크래시 창 수렴(치유 + 병합본 `.user` 보존)이 9dbea792 그대로 돈다.
    //   이 좁힘이 없으면 같은 상태 서명을 공유하는 merge3 크래시 창 복구 장치가 제거된다
    //   (치명위험 ③ 자가치유 전멸 · 실측 검체 merge3_crash_windows_converge [W3]).
    if has_pending_entry {
        return StaleEntry::Normal;
    }
    if known.is_empty() {
        return StaleEntry::KeepInPlace("no-row");
    }
    StaleEntry::KeepInPlace("unknown-disk")
}

/// H-08 의 "system 등급 **매니페스트 항목**인가" 판정 — [`pack_target_mode`] 의 0644→0755 복구 분기를
/// 그 집합으로만 좁히는 데 쓴다. 매니페스트 밖 파일·user 등급 파일은 사용자가 일부러 비실행으로 둘 수
/// 있으므로 복구 대상이 아니다(판독 실패 = 복구 안 함 = 안전측).
pub fn is_system_manifest_entry(dir: &Path, rel: &str) -> bool {
    if ownership_name_scoped(rel, dir) != "system" {
        return false;
    }
    let Some(s) = read_start_meta(&dir.join(INSTALL_MANIFEST)).text() else {
        return false;
    };
    let Ok(m) = serde_json::from_str::<std::collections::BTreeMap<String, String>>(&s) else {
        return false;
    };
    m.contains_key(rel)
}

/// 팩 **대상** 파일 쓰기(H-08). 사이드카(`<rel>.user`·`.base`·`.pristine`·`.vendor-base`)는 이 함수를
/// 쓰지 않는다 — 그쪽은 데이터이고 실행되지 않는다(설계의 명시 결정).
pub fn write_pack_file(
    path: &Path,
    bytes: &[u8],
    prior: Option<u32>,
    system_manifest_entry: bool,
) -> std::io::Result<()> {
    write_atomic_mode_exact(path, bytes, pack_target_mode(prior, bytes, system_manifest_entry))
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
    if read_start_meta(&p).text().as_deref() == Some(content) {
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
pub fn byte_backup_user(dir: &Path, rel: &str, src: &Path) -> Result<ParkOutcome, String> {
    // ★T2h(RULINGS CW3): 시작 경로에서 FIFO · 장치 · 상한 초과 파일을 읽지 않는다 — 막히지 않게 연 핸들이 일반 파일일 때만
    // 그 바이트를 백업한다(아니면 백업 실패 = 호출부의 격리 · 보안 자산은 종전 '무결성 우선' 예외 그대로).
    let bytes = match read_start_live(src) {
        StartRead::Bytes(b) => b,
        StartRead::Missing => return Err(format!("바이트 백업 원본 없음 {}", src.display())),
        StartRead::Unreadable(why) => return Err(format!("바이트 백업 원본 판독 불가 {}: {why}", src.display())),
    };
    // ★T2i(RULINGS EC4 (2) · EC2): 고정 임시 이름(`<rel>.user.tmp-<pid>` — 이미 있으면 따라가 자르고 썼다)을 쓰지 않는다 — 보관은
    //   [`park_copy`](배타 임시 파일 · 덮지 않는 놓기 · 차 있으면 세대 자리로).
    park_copy(dir, rel, &bytes, false)
}

/// ★T3(D9 · v2 §2 원칙 3): 검증 base lazy 로드 — `.pristine/<rel>` 을 읽어
/// `content_hash(pristine)==manifest_hash` 일 때만 Some. 불일치·부재·판독 실패 = None = 병합
/// 미시도(decide L4 종전 거동 낙하) — 크래시·원장 손상·연쇄 릴리스 스큐가 전부 이 게이트에서
/// "틀린 base 병합" 대신 손실 0 폴백으로 강등된다.
fn load_verified_base(dir: &Path, rel: &str, manifest_hash: &str) -> Option<String> {
    let pristine = read_start_meta(&dir.join(PRISTINE_DIR).join(rel)).text()?;
    if content_hash(&pristine) == manifest_hash {
        Some(pristine)
    } else {
        None
    }
}

/// ★0.15.0 M11 사(`.user` 한 칸 → 시각별 보관 · 설계 §4-11 사): `{rel}.user` 세대 보존 — 명령줄 도구에 이미
/// 있던 함수(`cys.rs` T4-fix R1-적대 #2)를 **그대로** 라이브러리로 옮겼다(세 번째 방식을 만들지 않는다).
/// 백업 덮어쓰기 직전, 기존 보존본이 있고 내용이 이번에 쓸 내용과 다르면 `{rel}.user.prev-<ts>[-n]` 으로
/// 원자 개명해 이전 세대를 지킨다. 동일 내용이면 무조치(멱등 재실행의 세대 증식 방지) · 개명 실패 = Err
/// (호출측 fail-closed 거부 의무). 설치 실행부(병합 폴백·판독 불가 바이트 백업·vendor 우선 보관)와
/// `cys pack-heal`·`revert-merge` 가 같이 쓴다.
pub fn preserve_prior_user_generation(
    dir: &Path,
    rel: &str,
    incoming: Option<&[u8]>,
) -> Result<Option<PathBuf>, String> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    preserve_prior_user_generation_at(dir, rel, incoming, ts, &mut |_| {})
}

/// ★T2h(RULINGS CW2 · 검토자 B2) → ★T2i(RULINGS EC2): 세대 자리는 **덮지 않는 이동**으로만 채운다 — `<rel>.user` 를 후보 이름으로
/// [`beneath_move_no_replace`](차 있으면 `AlreadyExists` → 다음 이름). 교체되는 세대 자리는 없다. 다른 쓰는 쪽이 이미 `.user` 를
/// 옮겼으면 보존할 것이 없다(Ok(None)). `before_reserve` = 시험의 끼어들기 지점(제품은 빈 함수).
fn preserve_prior_user_generation_at(
    dir: &Path,
    rel: &str,
    incoming: Option<&[u8]>,
    ts: u64,
    before_reserve: &mut dyn FnMut(&Path),
) -> Result<Option<PathBuf>, String> {
    match beneath_occupant(dir, &format!("{rel}.user")) {
        Occupant::Missing => return Ok(None),
        Occupant::File(Some(b)) if incoming == Some(b.as_slice()) => return Ok(None),
        _ => {}
    }
    Ok(move_occupant_to_slot(dir, rel, ts, before_reserve)?.map(|s| dir.join(s)))
}

/// ★T2i(RULINGS EC2): `<rel>.user` 에 있는 것을 새 세대 자리 `<rel>.user.prev-<ts>[-n]` 로 **덮지 않고** 옮긴다(팩 상대 경로 반환 ·
/// 옮길 것이 이미 없으면 None).
fn move_occupant_to_slot(
    dir: &Path,
    rel: &str,
    ts: u64,
    before_reserve: &mut dyn FnMut(&Path),
) -> Result<Option<String>, String> {
    let user_rel = format!("{rel}.user");
    for n in 1u32..=10_000 {
        let slot = if n == 1 { format!("{rel}.user.prev-{ts}") } else { format!("{rel}.user.prev-{ts}-{n}") };
        before_reserve(&dir.join(&slot));
        match beneath_move_no_replace(dir, &user_rel, &slot) {
            Ok(()) => return Ok(Some(slot)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::Unsupported => {
                return Err(format!("기존 {rel}.user 를 덮지 않고 옮기는 것을 이 파일 시스템이 지원하지 않습니다({e})"))
            }
            Err(e) => return Err(format!("기존 {rel}.user 세대 보존 실패({e})")),
        }
    }
    Err(format!("기존 {rel}.user 세대 자리를 찾지 못했습니다"))
}

/// ★T2i(RULINGS EC2): 보관 한 번의 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkOutcome {
    /// 이 쓰는 쪽의 사본이 있는 곳(팩 상대) — `<rel>.user`, 또는 자리가 끝내 비지 않았을 때 자기 세대 자리.
    pub at: String,
    /// 자리에 있던 다른 사본을 옮긴 세대 자리들(팩 상대).
    pub moved: Vec<String>,
    /// 자리가 끝내 비지 않아 자기 세대 자리에 놓았다(겹침).
    pub overlap: bool,
    /// ★T2i(RULINGS FH5): 겹침을 감사 원장에 적지 못한 사유 — 사본의 안전(위 `at`)과 겹침의 기록은 서로 다른 결과다. 겹침이
    /// 없거나 적었으면 None.
    pub overlap_record_error: Option<String>,
}

/// ★T2i(RULINGS EC2): 보관의 끼어들기 지점(시험 전용 — 제품은 빈 함수).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkStep {
    /// n 번째 놓기 직전.
    BeforePlace(u32),
    /// n 번째 놓기가 '차 있음'을 본 뒤, 그 자리의 사본을 세대 자리로 옮기기 직전.
    BeforeMove(u32),
}

/// ★T2i(RULINGS EC2): 덮지 않는 놓기를 다시 시도하는 횟수(그 뒤엔 자기 세대 자리 + 감사 원장의 겹침 기록).
pub const PARK_PLACE_TRIES: u32 = 3;

/// ★T2i(RULINGS EC2 — 규칙 하나를 보관본에): `<rel>.user` 에는 **아무것도 덮어쓰지 않는다**. 사본은 덮지 않는 놓기로만 그 이름에
/// 생기고([`beneath_write`] `NoReplace`), 그 이름에 있던 것은 덮지 않는 이동으로만 세대 자리로 간다. 놓을 때 자리가 차 있으면:
/// 같은 바이트면 끝 · 다르면 그 사본을 새 세대 자리로 옮기고 다시 놓는다(최대 [`PARK_PLACE_TRIES`]번). 그래도 차 있으면 자기
/// 사본을 자기 세대 자리에 두고 겹침을 감사 원장에 적는다(`park-overlap`). 그래서 두 쓰는 쪽이 어떻게 엇갈려도 모든 사본이 이름이나
/// 세대 자리에 남는다 — '막기가 아니라 알아채고 보관'. 덮지 않는 놓기를 지원하지 않는 파일 시스템 = Err(호출부의 보관 불가 갈래).
/// `durable` = vendor 우선의 선저장(EC7 — 놓은 뒤 폴더 fsync 실패를 돌려준다).
pub fn park_copy(dir: &Path, rel: &str, bytes: &[u8], durable: bool) -> Result<ParkOutcome, String> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    park_copy_at(dir, rel, bytes, durable, ts, &mut |_| {})
}

fn park_copy_at(
    dir: &Path,
    rel: &str,
    bytes: &[u8],
    durable: bool,
    ts: u64,
    hook: &mut dyn FnMut(ParkStep),
) -> Result<ParkOutcome, String> {
    let user_rel = format!("{rel}.user");
    let mut moved: Vec<String> = Vec::new();
    for attempt in 0..PARK_PLACE_TRIES {
        hook(ParkStep::BeforePlace(attempt));
        match beneath_write(dir, &user_rel, bytes, None, false, Place::NoReplace, durable) {
            Ok(()) => return Ok(ParkOutcome { at: user_rel, moved, overlap: false, overlap_record_error: None }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) if e.kind() == std::io::ErrorKind::Unsupported => {
                return Err(format!("{user_rel}: {e} — 덮지 않고 놓을 수 없어 보관하지 않았습니다"))
            }
            Err(e) => return Err(format!("{user_rel} 기록 실패: {e}")),
        }
        match beneath_occupant(dir, &user_rel) {
            Occupant::Missing => continue,
            Occupant::File(Some(b)) if b == bytes => {
                // ★T2i(RULINGS GB2): 자리에 이미 있는 같은 사본은 내려졌다고 가정하지 않는다 — 새로 쓴 사본과 같이 그 **파일**을 먼저
                //   내리고(언제나), 영속 증명이 필요하면 폴더도. 실패는 실패 — 호출부는 살아 있는 파일을 바꾸지 않는다.
                beneath_sync_file(dir, &user_rel).map_err(|e| format!("{user_rel} 기록 실패: {e}"))?;
                if durable {
                    beneath_sync_parent(dir, &user_rel).map_err(|e| format!("{user_rel} 기록 실패: {e}"))?;
                }
                return Ok(ParkOutcome { at: user_rel, moved, overlap: false, overlap_record_error: None });
            }
            Occupant::File(_) => {}
            Occupant::Obstacle(why) => {
                return Err(format!("{user_rel} 자리에 일반 파일이 아닌 항목({why})이 있어 세대 보존·백업을 할 수 없습니다"))
            }
        }
        // 마지막 시도에서도 차 있으면 그 사본은 이름에 두고(옮기지 않는다) 자기 사본을 세대 자리로.
        if attempt + 1 == PARK_PLACE_TRIES {
            break;
        }
        hook(ParkStep::BeforeMove(attempt));
        match move_occupant_to_slot(dir, rel, ts, &mut |_| {}) {
            Ok(Some(slot)) => moved.push(slot),
            Ok(None) => {}
            // ★T2i(RULINGS FP1): 덮지 않는 이동을 지원하지 않으면 자리의 사본은 그대로 두고, 이 사본을 자기 세대 자리에(배타 생성).
            Err(e) if e.contains("지원하지 않") => break,
            Err(e) => return Err(e),
        }
    }
    // 자리가 끝내 비지 않았다 — 자기 사본은 자기 세대 자리로, 겹침은 감사 원장에.
    let mut own: Option<String> = None;
    for n in 1u32..=10_000 {
        let slot = if n == 1 { format!("{rel}.user.prev-{ts}") } else { format!("{rel}.user.prev-{ts}-{n}") };
        match beneath_write(dir, &slot, bytes, None, false, Place::NoReplace, durable) {
            Ok(()) => {
                own = Some(slot);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{rel}: 보관 자리 겹침 — 자기 세대 자리 기록 실패({e})")),
        }
    }
    let Some(slot) = own else { return Err(format!("{rel}: 보관 자리 겹침 — 세대 자리를 찾지 못했습니다")) };
    let entry = serde_json::json!({
        "ts": ts, "file": rel, "action": "park-overlap", "slot": slot, "moved": moved, "flags": [],
    });
    let overlap_record_error = match append_merge_audit(dir, &entry) {
        Ok(()) => None,
        Err(e) => {
            eprintln!("[pack] ⚠ {rel}: 보관 자리 겹침을 감사 원장에 적지 못했습니다({e}) — 사본은 {slot} 에 있습니다");
            Some(e)
        }
    };
    Ok(ParkOutcome { at: slot, moved, overlap: true, overlap_record_error })
}

/// ★T2i(RULINGS EC7): `rel_path` 의 부모 폴더 fsync(닻 내린 기술자 — 윈도우는 '지원하지 않음').
fn beneath_sync_parent(root: &Path, rel_path: &str) -> std::io::Result<()> {
    let parts = rel_parts(rel_path)?;
    #[cfg(unix)]
    {
        let d = anchor_dir(root, &parts[..parts.len() - 1], false, rel_path)?;
        sync_dir_fd(&d, parts[parts.len() - 1])
    }
    #[cfg(not(unix))]
    {
        let _ = (root, parts);
        Ok(())
    }
}

/// ★M11 나: 사용자 수정본을 `{rel}.user` 로 보관(이전 세대 보존 선행). 이미 같은 내용이면 무조치.
/// Err = 보관 불가(호출자는 덮지 않고 격리한다 — 부록 A-2 #4).
/// ★T2i(RULINGS EC2 · EC7): [`park_copy`] 하나로(덮지 않는 놓기 · 영속 증명) — 사본이 있는 곳을 돌려준다(원장 side).
fn park_user_copy(dir: &Path, rel: &str, bytes: &[u8]) -> Result<ParkOutcome, String> {
    park_copy(dir, rel, bytes, true)
}

/// ★M11 가: 사용자 파일 유지 + vendor 신버전 `<rel>.new` 보관 + 원장 `user-kept`. 셸 스크립트는 살아 있는
/// 사용자본에서 CR 을 뺀다(라 — 보존 결과든 셸 스크립트는 LF). 같은 vendor 본으로 이미 보관 중(`parked`)
/// 이면 원장 항목을 다시 쓰지 않는다(첫 판정 사유 보존 · 매 스윕 rewrite 0).
#[allow(clippy::too_many_arguments)]
fn park_user_kept(
    dir: &Path,
    rel: &str,
    vendor: &str,
    live: Option<&str>,
    reason: &str,
    caution: Option<String>,
    capture: Option<String>,
    pending: &mut serde_json::Map<String, serde_json::Value>,
    dirty: &mut bool,
    version: &str,
    ts: u64,
) {
    let new_path = dir.join(format!("{rel}.new"));
    if read_start_meta(&new_path).text().as_deref() != Some(vendor) {
        if let Err(e) = write_atomic(&new_path, vendor.as_bytes()) {
            eprintln!("[init-pack] ⚠ {rel}: vendor 신버전 보관({rel}.new) 기록 실패({e}) — 사용자 파일은 그대로");
        }
    }
    let mut eol_fixed = false;
    if is_shell_script(rel, vendor) {
        if let Some(l) = live.filter(|l| l.contains('\r')) {
            let p = dir.join(rel);
            let fixed = l.replace('\r', "");
            match write_pack_file(&p, fixed.as_bytes(), current_mode(&p), true) {
                Ok(()) => eol_fixed = true,
                Err(e) => eprintln!("[init-pack] ⚠ {rel}: 셸 스크립트 CR 제거 실패({e})"),
            }
        }
    }
    let already = pending.get(rel).and_then(|e| e.get("kind")).and_then(|k| k.as_str())
        == Some("user-kept");
    if reason == "parked" && already && !eol_fixed {
        return;
    }
    let mut entry = serde_json::Map::new();
    entry.insert("kind".into(), serde_json::json!("user-kept"));
    entry.insert("side".into(), serde_json::json!(format!("{rel}.new")));
    entry.insert("reason".into(), serde_json::json!(reason));
    if let Some(c) = caution {
        entry.insert("caution".into(), serde_json::json!(c));
    }
    if let Some(c) = capture {
        entry.insert("capture".into(), serde_json::json!(c));
    }
    if eol_fixed {
        entry.insert("eol".into(), serde_json::json!("lf"));
    }
    entry.insert("version".into(), serde_json::json!(version));
    entry.insert("ts".into(), serde_json::json!(ts));
    if reason == "parked" && already {
        // CR 만 고친 재방문 — 첫 사유를 지키고 eol 표시만 더한다.
        if let Some(serde_json::Value::Object(old)) = pending.get(rel).cloned() {
            let mut e2 = old;
            e2.insert("eol".into(), serde_json::json!("lf"));
            upsert_pending_v2(pending, dirty, rel, e2);
            return;
        }
    }
    upsert_pending_v2(pending, dirty, rel, entry);
}

/// `root` 아래의 빈 디렉터리를 깊이 우선으로 지운다(파일이 있으면 그대로 — `remove_dir` 는 빈 것만 지운다).
fn remove_empty_dirs_under(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            remove_empty_dirs_under(&p);
            let _ = std::fs::remove_dir(&p);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ★0.15.0 M11 아 — 업데이트 전 스냅샷(설치기가 관리하는 파일만) + 복원 계약(CONTRACTS 8.10 · O-C8-4 (a))
// 위치 `~/.cys/local/update-snapshots/<lane_key>/<from-version>-<UTC 시각>/` (로컬 오버레이 — 공장 초기화
// 최상위 목록 무변경, `--purge-local` 이 함께 옮긴다). 팩 폴더 전체가 아니라 매니페스트 항목만 담는다
// (사용자 작업 자료·기억·세션 상태는 담지 않는다 — seed-once 제외). 복사로만 만든다(살아 있는 팩 폴더의
// 이름을 바꾸지 않는다 — 윈도우 파괴적 대체 동작 4753–4760 때문). 레인마다 최신 3세대만 둔다.
// ─────────────────────────────────────────────────────────────────────────────

/// 스냅샷 메타 스키마(정수 · 읽는 쪽은 더 높은 값을 읽을 수 없음으로 다룬다 — O-C1-7 과 같은 규칙).
pub const UPDATE_SNAPSHOT_SCHEMA: u32 = 1;
/// 레인당 보존 세대 수.
pub const UPDATE_SNAPSHOT_KEEP: usize = 3;
/// ★RULINGS AC4(부록 A-2 #17 — 데몬이 통신을 열기 전에는 느린 일을 하지 않는다): 파일 하나 / 스냅샷 하나의
/// 바이트 상한. 넘는 파일은 담지 않고 색인(`skipped`)에 이름을 남긴다 — 경고 1줄, 설치는 계속(실패로 만들지
/// 않는다). 설계 §4-11 아의 추정(설치기 관리 파일 12~25MB)과 이 팩의 실측(내장 약 479개 · 약 11.5MB · 가장 큰
/// 파일 약 1.3MB) 위에 둔다.
pub const UPDATE_SNAPSHOT_FILE_CAP: u64 = 4 * 1024 * 1024;
pub const UPDATE_SNAPSHOT_TOTAL_CAP: u64 = 32 * 1024 * 1024;
/// 스냅샷 메타 파일명 / 파일 사본 루트.
pub const UPDATE_SNAPSHOT_META: &str = "meta.json";
pub const UPDATE_SNAPSHOT_FILES: &str = "files";

/// 복원 계약의 파일 한 줄 — `{rel, sha256, mode, released, link_target?}`(필드 순서 = 계약 순서).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SnapshotFile {
    pub rel: String,
    /// 원본 바이트의 sha256(링크면 빈 문자열 — 링크는 `link_target` 이 정체다).
    pub sha256: String,
    /// 권한 비트(`st_mode & 0o7777` · 윈도우 0).
    pub mode: u32,
    /// ★RULINGS AC1(부록 A-2 #12 검증된 되돌리기): 이 바이트가 **출시본**인가 — 바꾸기 전 설치 기록(매니페스트
    /// 해시)과 같거나 과거 출시 해시 표에 있다. system 파일은 이것이 참일 때만 되돌린다. 링크·판독 불가·사용자
    /// 수정 = 거짓. 이 필드가 없는 메타는 거짓으로 읽는다(안전 방향).
    #[serde(default)]
    pub released: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<String>,
}

/// 상한 때문에 담지 않은 파일 — 색인에 이름·크기·사유를 남긴다(RULINGS AC4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SnapshotSkip {
    pub rel: String,
    pub bytes: u64,
    /// `file-cap` | `total-cap`
    pub reason: String,
}

/// 복원 계약 — `{schema, lane_key, pack_version, pro_revision, created_at, files, skipped?}`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SnapshotMeta {
    pub schema: u32,
    pub lane_key: String,
    pub pack_version: String,
    pub pro_revision: u32,
    pub created_at: String,
    pub files: Vec<SnapshotFile>,
    /// 상한 때문에 담지 않은 파일(없으면 생략).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SnapshotSkip>,
}

/// 스냅샷 하나를 만든 결과.
#[derive(Debug, Clone)]
pub struct SnapshotOutcome {
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub skipped: Vec<SnapshotSkip>,
}

/// 원본 바이트의 sha256(소문자 16진) — 스냅샷 계약의 `sha256`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn path_safe(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect()
}

/// 업데이트 전 스냅샷을 만든다 — `src_dir`(설치 대상 · staging 이면 라이브 팩의 전량 사본)에서 `rels`(이번
/// 설치가 덮거나 지울 파일 — 호출자가 같은 판정 함수로 고른다, RULINGS AC4)만, 권한·링크 대상·출시본 여부(AC1)와
/// 함께 복사한다. seed-once 는 담지 않는다. `caps` = (파일 하나, 전체) 바이트 상한 — 넘는 파일은 담지 않고
/// `skipped` 에 남긴다. 임시 디렉터리에 채운 뒤 한 번에 개명하고(반쪽 스냅샷이 최신으로 보이지 않게), 레인당 최신
/// [`UPDATE_SNAPSHOT_KEEP`] 세대만 남기되 이 전환(`from_version`)의 가장 오래된 스냅샷은 지우지 않는다(AC3).
#[allow(clippy::too_many_arguments)]
pub fn create_update_snapshot(
    src_dir: &Path,
    root: &Path,
    lane_key: &str,
    from_version: &str,
    manifest: &std::collections::BTreeMap<String, String>,
    rels: &[String],
    scope: PackScope,
    prev_vendor: &dyn Fn(&str) -> Vec<String>,
    caps: (u64, u64),
) -> Result<SnapshotOutcome, String> {
    let lane_dir = root.join(path_safe(lane_key));
    // ★T2h(RULINGS CW1): 스냅샷 보관소와 레인 폴더는 링크 · 재해석 지점이 아니다 — 링크를 거치면 사본 쓰기와 오래된 세대
    // 정리(폴더째 지우기)가 그 링크가 가리키는 곳(살아 있는 팩일 수도 있다)에서 일어난다.
    for p in [root, lane_dir.as_path()] {
        if std::fs::symlink_metadata(p).is_ok_and(|m| meta_is_link_like(&m)) {
            return Err(format!("스냅샷 자리 {} 가 링크(또는 재해석 지점)입니다 — 스냅샷을 만들지 않습니다", p.display()));
        }
    }
    std::fs::create_dir_all(&lane_dir)
        .map_err(|e| format!("스냅샷 레인 생성 실패 {}: {e}", lane_dir.display()))?;
    let now = chrono::Utc::now();
    let ts = now.format("%Y%m%dT%H%M%SZ").to_string();
    let tmp = lane_dir.join(format!(".tmp-{ts}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join(UPDATE_SNAPSHOT_FILES))
        .map_err(|e| format!("스냅샷 임시 디렉터리 생성 실패 {}: {e}", tmp.display()))?;
    let fill = || -> Result<(Vec<SnapshotFile>, u64, Vec<SnapshotSkip>), String> {
        let mut files = Vec::new();
        let mut skipped: Vec<SnapshotSkip> = Vec::new();
        let mut bytes_total = 0u64;
        for rel in rels {
            if ownership_scoped(rel, scope) == Ownership::SeedOnce
                || rel.contains("..")
                || rel.starts_with('/')
            {
                continue;
            }
            let src = src_dir.join(rel);
            let Ok(meta) = std::fs::symlink_metadata(&src) else { continue };
            if meta.is_file() {
                let len = meta.len();
                let reason = if len > caps.0 {
                    Some("file-cap")
                } else if bytes_total + len > caps.1 {
                    Some("total-cap")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    skipped.push(SnapshotSkip { rel: rel.clone(), bytes: len, reason: reason.into() });
                    continue;
                }
            }
            let dst = tmp.join(UPDATE_SNAPSHOT_FILES).join(rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("스냅샷 디렉터리 생성 실패 {}: {e}", parent.display()))?;
            }
            if meta.file_type().is_symlink() {
                let target = std::fs::read_link(&src)
                    .map_err(|e| format!("링크 대상 판독 실패 {}: {e}", src.display()))?;
                let target_s = target.to_string_lossy().to_string();
                #[cfg(unix)]
                std::os::unix::fs::symlink(&target, &dst)
                    .map_err(|e| format!("스냅샷 링크 생성 실패 {}: {e}", dst.display()))?;
                files.push(SnapshotFile {
                    rel: rel.clone(),
                    sha256: String::new(),
                    mode: 0,
                    released: false,
                    link_target: Some(target_s),
                });
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            // ★T2h(RULINGS CW3): 막히지 않는 판독 — lstat 뒤에 FIFO 로 바뀐 파일이 시작을 멈추지 못한다.
            let data = match read_start_live(&src) {
                StartRead::Bytes(b) => b,
                StartRead::Missing => continue,
                StartRead::Unreadable(why) => {
                    return Err(format!("스냅샷 원본 판독 실패 {}: {why}", src.display()))
                }
            };
            std::fs::write(&dst, &data)
                .map_err(|e| format!("스냅샷 사본 기록 실패 {}: {e}", dst.display()))?;
            bytes_total += data.len() as u64;
            // 출시본 = 바꾸기 전 설치 기록과 같은 바이트 ∨ 과거 출시 해시 표의 바이트(설치 판정 다와 같은 표).
            // 설치 기록의 해시(`content_hash`)는 같은 바이트의 sha256 이다 — 한 번만 계산한다.
            let sha = sha256_hex(&data);
            let released = std::str::from_utf8(&data).is_ok()
                && (manifest.get(rel.as_str()) == Some(&sha) || prev_vendor(rel).contains(&sha));
            files.push(SnapshotFile {
                rel: rel.clone(),
                sha256: sha,
                mode: current_mode(&src).unwrap_or(0),
                released,
                link_target: None,
            });
        }
        Ok((files, bytes_total, skipped))
    };
    let (files, bytes_total, skipped) = match fill() {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };
    let pro_revision = match read_pack_state(src_dir) {
        PackStateRead::Valid(st) => st.pro_revision,
        _ => 0,
    };
    let meta = SnapshotMeta {
        schema: UPDATE_SNAPSHOT_SCHEMA,
        lane_key: lane_key.to_string(),
        pack_version: from_version.to_string(),
        pro_revision,
        // 나노초 고정 폭(9자리 · `Z`) — 같은 초 안의 두 스냅샷도 문자열 순서 = 생성 순서(목록·정리·"최신"이 이 순서를 쓴다).
        created_at: now.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        files,
        skipped: skipped.clone(),
    };
    let n = meta.files.len();
    let json = serde_json::to_string_pretty(&meta).map_err(|e| format!("스냅샷 메타 직렬화 실패: {e}"))?;
    if let Err(e) = write_atomic(&tmp.join(UPDATE_SNAPSHOT_META), json.as_bytes()) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("스냅샷 메타 기록 실패: {e}"));
    }
    let base_name = format!("{}-{ts}", path_safe(from_version));
    let mut dest = lane_dir.join(&base_name);
    let mut k = 1u32;
    while dest.exists() {
        k += 1;
        dest = lane_dir.join(format!("{base_name}-{k}"));
    }
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("스냅샷 확정(개명) 실패 {}: {e}", dest.display()));
    }
    prune_update_snapshots(&lane_dir, UPDATE_SNAPSHOT_KEEP, Some(from_version));
    Ok(SnapshotOutcome { path: dest, files: n, bytes: bytes_total, skipped })
}

/// 레인 디렉터리의 스냅샷 목록(오래된 것 → 최신, `created_at`(나노초 고정 폭) 기준 · 메타 판독 불가는 빈 시각 = 맨 앞, 그 안에서 이름순).
pub fn list_update_snapshots(lane_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(lane_dir) else { return out };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        // ★T2h(RULINGS CW1): 링크는 스냅샷이 아니다(목록 · 정리 대상에서 뺀다 — 정리가 링크 너머를 지우지 않는다).
        if name.starts_with('.') || !std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir() && !meta_is_link_like(&m)) {
            continue;
        }
        let created = read_update_snapshot_meta(&p)
            .map(|m| m.created_at)
            .unwrap_or_default();
        out.push((format!("{created}\u{0}{name}"), p));
    }
    out.sort();
    out
}

/// 오래된 것부터 지워 `keep` 세대만 남긴다 — 단 `protect_version`(지금 진행 중인 전환의 출발 버전)을 담은 가장
/// 오래된 스냅샷은 지우지 않는다(RULINGS AC3: 실패를 거듭하는 업데이트가 진짜 업데이트 전 스냅샷을 밀어내지 않게).
fn prune_update_snapshots(lane_dir: &Path, keep: usize, protect_version: Option<&str>) {
    let list = list_update_snapshots(lane_dir);
    if list.len() <= keep {
        return;
    }
    let protected: Option<PathBuf> = protect_version.and_then(|v| {
        list.iter()
            .find(|(_, p)| read_update_snapshot_meta(p).is_ok_and(|m| m.pack_version == v))
            .map(|(_, p)| p.clone())
    });
    let mut excess = list.len() - keep;
    for (_, p) in &list {
        if excess == 0 {
            break;
        }
        if protected.as_ref() == Some(p) {
            continue;
        }
        let _ = std::fs::remove_dir_all(p);
        excess -= 1;
    }
}

/// 이 레인의 가장 최근 스냅샷(`root` = `update-snapshots` 루트).
pub fn newest_update_snapshot(root: &Path, lane_key: &str) -> Option<PathBuf> {
    list_update_snapshots(&root.join(path_safe(lane_key)))
        .pop()
        .map(|(_, p)| p)
}

/// 스냅샷 메타 판독(스키마가 더 높으면 Err — 읽을 수 없음).
pub fn read_update_snapshot_meta(snap: &Path) -> Result<SnapshotMeta, String> {
    let s = match read_start_meta(&snap.join(UPDATE_SNAPSHOT_META)) {
        StartRead::Bytes(b) => String::from_utf8(b)
            .map_err(|e| format!("스냅샷 메타 판독 실패 {}: {e}", snap.display()))?,
        StartRead::Missing => return Err(format!("스냅샷 메타 판독 실패 {}: 없음", snap.display())),
        StartRead::Unreadable(why) => return Err(format!("스냅샷 메타 판독 실패 {}: {why}", snap.display())),
    };
    let m: SnapshotMeta =
        serde_json::from_str(&s).map_err(|e| format!("스냅샷 메타 파싱 실패: {e}"))?;
    if m.schema > UPDATE_SNAPSHOT_SCHEMA {
        return Err(format!(
            "스냅샷 메타 스키마 {} 는 이 프로그램({UPDATE_SNAPSHOT_SCHEMA})보다 새롭습니다 — 읽지 않습니다",
            m.schema
        ));
    }
    Ok(m)
}

/// 스냅샷 한 항목의 보관본 검증 — 파일은 sha256, 링크는 링크 대상. Ok = 복원해도 되는 바이트(링크면 빈 벡터).
pub fn verify_snapshot_entry(snap: &Path, f: &SnapshotFile) -> Result<Vec<u8>, String> {
    let stored = snap.join(UPDATE_SNAPSHOT_FILES).join(&f.rel);
    if let Some(target) = &f.link_target {
        let got = std::fs::read_link(&stored)
            .map_err(|e| format!("'{}' 링크 보관본 판독 실패: {e}", f.rel))?;
        if got.to_string_lossy() != target.as_str() {
            return Err(format!(
                "'{}' 링크 보관본의 대상이 기록과 다릅니다(기록 {target} · 실제 {})",
                f.rel,
                got.display()
            ));
        }
        return Ok(Vec::new());
    }
    let data = std::fs::read(&stored).map_err(|e| format!("'{}' 보관본 판독 실패: {e}", f.rel))?;
    let got = sha256_hex(&data);
    if got != f.sha256 {
        return Err(format!(
            "'{}' 보관본이 기록과 다릅니다(기록 {} · 실제 {got})",
            f.rel, f.sha256
        ));
    }
    Ok(data)
}

/// 복원 계약(CONTRACTS O-C8-4): 선택한 항목을 **전부 검증한 뒤에만** 쓴다(하나라도 어긋나면 아무것도
/// 쓰지 않는다). `only=None` = 전체 복원 = system 파일 전부 + `.pack-version` + 팩 상태 revision(버전
/// 표시와 상태의 일관성) — user 소유 파일은 전체 복원에서 제외(업데이트 뒤 사용자 편집 보호 · 파일 단위
/// `only` 로만). seed-once 는 스냅샷에 없다. 권한은 기록값 그대로, 링크는 링크로. 반환 = 복원한 rel.
pub fn restore_update_snapshot(
    snap: &Path,
    pack_dir: &Path,
    only: Option<&[String]>,
    sources: &UpdateSources,
) -> Result<Vec<String>, String> {
    let meta = read_update_snapshot_meta(snap)?;
    let scope = pack_scope_of(pack_dir);
    let selected: Vec<&SnapshotFile> = match only {
        Some(list) => {
            let mut v = Vec::new();
            for rel in list {
                let f = meta
                    .files
                    .iter()
                    .find(|f| &f.rel == rel)
                    .ok_or_else(|| format!("'{rel}' 은 이 스냅샷에 없습니다"))?;
                v.push(f);
            }
            v
        }
        None => meta
            .files
            .iter()
            .filter(|f| ownership_scoped(&f.rel, scope) == Ownership::System)
            .collect(),
    };
    // ① 전부 검증(쓰기 0).
    let mut verified: Vec<(&SnapshotFile, Vec<u8>)> = Vec::with_capacity(selected.len());
    for f in selected {
        if f.rel.contains("..") || f.rel.starts_with('/') {
            return Err(format!("잘못된 경로가 스냅샷에 있습니다: {}", f.rel));
        }
        // ★RULINGS AC1(부록 A-2 #12): system 파일은 출시본 바이트일 때만 되돌린다. 파일 단위 요청이면 거부(아무것도
        // 쓰지 않는다), 전체 복원이면 그 항목만 건너뛴다(사용자 수정본은 그 파일의 병합 장부가 지킨다).
        if ownership_scoped(&f.rel, scope) == Ownership::System && !f.released {
            if only.is_some() {
                return Err(format!(
                    "'{}' 의 보관본은 출시본 바이트가 아닙니다(바꾸기 전 설치 기록·과거 출시 해시 표와 다름) — \
                     system 파일은 검증된 출시본으로만 되돌립니다",
                    f.rel
                ));
            }
            continue;
        }
        let bytes = verify_snapshot_entry(snap, f)?;
        // ★T2f(RULINGS BO5 · BD4): 기동 필수·보호 목록 파일은 쓰려는 바이트가 출시본일 때만(색인의 `released` 표시를
        // 믿지 않고 다시 잰다 · 링크는 출시본이 아니다). 파일 단위 요청이면 거부, 전체 복원이면 그 항목만 건너뛴다.
        if sources.is_listed(&f.rel)
            && !(f.link_target.is_none() && released_bytes(pack_dir, &f.rel, &bytes, sources))
        {
            if only.is_some() {
                return Err(listed_file_refusal(&f.rel, "update snapshot restore"));
            }
            continue;
        }
        verified.push((f, bytes));
    }
    // ② 쓰기.
    let mut restored = Vec::new();
    for (f, data) in verified {
        let dest = pack_dir.join(&f.rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("디렉터리 생성 실패 {}: {e}", parent.display()))?;
        }
        if let Some(target) = &f.link_target {
            #[cfg(unix)]
            {
                if std::fs::symlink_metadata(&dest).is_ok() {
                    std::fs::remove_file(&dest)
                        .map_err(|e| format!("'{}' 기존 항목 제거 실패: {e}", f.rel))?;
                }
                std::os::unix::fs::symlink(target, &dest)
                    .map_err(|e| format!("'{}' 링크 복원 실패: {e}", f.rel))?;
            }
            #[cfg(not(unix))]
            {
                let _ = target;
                return Err(format!(
                    "'{}' 는 링크입니다 — 이 운영체제에서는 링크를 만들 수 없어 복원하지 않았습니다",
                    f.rel
                ));
            }
        } else {
            let mode = if cfg!(unix) { Some(f.mode) } else { None };
            write_atomic_mode_exact(&dest, &data, mode)
                .map_err(|e| format!("'{}' 복원 기록 실패: {e}", f.rel))?;
        }
        restored.push(f.rel.clone());
    }
    if only.is_none() {
        write_atomic(&pack_dir.join(PACK_VERSION_FILE), meta.pack_version.as_bytes())
            .map_err(|e| format!(".pack-version 복원 실패: {e}"))?;
        if let PackStateRead::Valid(mut st) = read_pack_state(pack_dir) {
            st.base_version = meta.pack_version.clone();
            st.pro_revision = meta.pro_revision;
            write_pack_state(pack_dir, &st)?;
        }
    }
    Ok(restored)
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
        .and_then(|p| read_start_live(p).text())
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
/// 보존·벤더 전진=검증 base 3-way(★0.15.0 M11: 충돌·기준본 없음·손상 의심 = 기동 필수가 아니면 사용자 파일
/// 유지 + `<rel>.new`(user-kept) · 기동 필수면 vendor 본 우선 + `<rel>.user` 보관(vendor-first)), 비수정 파일은
/// 입력 신버전으로 자동 갱신. 묘비(장부에 기록된 삭제)는 부재 유지 · 생명줄은 언제나 되살린다.
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
#[allow(clippy::too_many_arguments)]
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
    // ★0.15.0 M11: 이 래퍼는 묘비·기동 필수·스냅샷이 없는 입력원(`UpdateSources::none()`)으로
    // 위임한다 — 운영 호출자(install_from_iter·install_staged)는 `install_into_with` 에 라이브 팩용
    // 입력원(`UpdateSources::for_live_pack`)을 넘긴다. 이 래퍼는 시험 전용이다(운영 호출 0 — grep).
    install_into_with(
        dir,
        items,
        force,
        target_version,
        transactional,
        setup_config,
        scope,
        auth,
        capture_ns,
        &UpdateSources::none(),
    )
}

/// ★0.15.0 M11 아 + RULINGS AC3·AC4: 이번 설치의 업데이트 전 스냅샷. None = 만들 일이 없다(입력원이 루트·레인을
/// 주지 않음 · 디스크 버전 없음 또는 = 대상 · 매니페스트 없음 · **이 전환의 스냅샷이 이미 있음** · 덮거나 지울 파일 0).
/// 대상 = 설치 실행부와 같은 판정(plan)이 덮는다(갱신·치유·병합·vendor 우선)·지운다(폐기 정리)로 고른 파일 중 디스크
/// 바이트가 새 바이트와 다른 것, 그리고 새로 만들 자리에 놓인 링크(쓰기가 링크를 파일로 바꾼다).
fn take_update_snapshot(
    dir: &Path,
    items: &[(&str, &str)],
    force: bool,
    target_version: &str,
    scope: PackScope,
    sources: &UpdateSources,
    manifest: &std::collections::BTreeMap<String, String>,
) -> Option<Result<SnapshotOutcome, String>> {
    let root = sources.snapshot_root.as_ref()?;
    let lane = sources.lane.as_ref()?;
    let dv = read_start_meta(&dir.join(PACK_VERSION_FILE))
        .text()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    if dv == target_version || manifest.is_empty() {
        return None;
    }
    // AC3: 이 전환의 스냅샷이 이미 있으면(실패를 거듭하는 기동 설치 등) 다시 복사하지 않는다 — 첫 시도 직전의
    // 상태가 진짜 업데이트 전 보존본이다(이후 시도는 이미 일부를 쓴 팩을 본다).
    if newest_update_snapshot(root, &lane.0)
        .and_then(|p| read_update_snapshot_meta(&p).ok())
        .is_some_and(|m| m.pack_version == dv)
    {
        return None;
    }
    let plan = plan_install_core(dir, items, force, target_version, scope, sources);
    if plan.blocked.is_some() {
        return None;
    }
    let embed: std::collections::HashMap<&str, &str> = items.iter().copied().collect();
    let is_link = |r: &str| {
        std::fs::symlink_metadata(dir.join(r)).is_ok_and(|m| m.file_type().is_symlink())
    };
    let mut rels: Vec<String> = Vec::new();
    for r in plan
        .update
        .iter()
        .chain(&plan.heal)
        .chain(&plan.merge3)
        .chain(&plan.vendor_first)
        .chain(&plan.prune_delete)
    {
        // 바이트가 그대로인 재기록(force · 디스크 = 새 바이트)은 되돌릴 것이 없다.
        let same = !is_link(r)
            && embed.get(r.as_str()).is_some_and(|c| {
                read_start_live(&dir.join(r)).bytes().is_some_and(|b| b == c.as_bytes())
            });
        if !same && !rels.contains(r) {
            rels.push(r.clone());
        }
    }
    for r in &plan.create {
        if is_link(r) && !rels.contains(r) {
            rels.push(r.clone());
        }
    }
    if rels.is_empty() {
        return None;
    }
    let prev = |rel: &str| sources.prev_vendor_for(rel);
    Some(create_update_snapshot(
        dir,
        root,
        &lane.0,
        &dv,
        manifest,
        &rels,
        scope,
        &prev,
        (UPDATE_SNAPSHOT_FILE_CAP, UPDATE_SNAPSHOT_TOTAL_CAP),
    ))
}

/// [`install_into`] 의 입력원 주입판 — 판정·실행의 본체(★0.15.0 M11 업데이트 규칙).
#[allow(clippy::too_many_arguments)]
pub fn install_into_with<'a, I: IntoIterator<Item = (&'a str, &'a str)>>(
    dir: PathBuf,
    items: I,
    force: bool,
    target_version: &str,
    transactional: bool,
    setup_config: bool,
    scope: PackScope,
    auth: Option<PackWriteAuth>,
    capture_ns: Option<&str>,
    sources: &UpdateSources,
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
    // ★0.15.0 M11 다: 깨진 매니페스트는 빈 표(지금 동작)이되 '깨짐'을 판정 입력으로 넘긴다 — 사용자
    // 수정으로 다루기 전에 과거 vendor 해시 표를 본다(부록 A-2 #1).
    let (mut manifest, manifest_corrupt) = load_manifest_state(&dir);
    if manifest_corrupt {
        eprintln!(
            "[init-pack] ⚠ 설치 기록({INSTALL_MANIFEST})이 깨져 읽을 수 없습니다 — 과거 버전 해시 표로 \
             옛 vendor 본을 먼저 가려 갱신하고, 그 밖의 수정본은 사용자 것으로 보존합니다"
        );
    }
    // 다운그레이드 차단: 디스크 팩 버전이 입력 버전(target_version)보다 새것이면(구버전 cys로 롤백/오설치)
    // 비강제 install이 비수정 파일·prune으로 신기능을 구 내용으로 후퇴시키는 사일런트 회귀를 막는다.
    // force(수동 init-pack --force)면 우회 — 의도적 재설치는 허용.
    if !force {
        if let Some(dv) = read_start_meta(&dir.join(PACK_VERSION_FILE))
            .text()
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
    // ★0.15.0 M11 아 + RULINGS AC3·AC4: 업데이트 전 스냅샷 — 이번 설치가 덮거나 지울 파일만(같은 판정 함수로 고른다),
    // 상한 안에서, 전환마다 한 번. 첫 쓰기보다 **먼저** 확정한다(제자리 쓰기인 기동 설치가 중간에 멈춰도 보존본은
    // 온전하다). 복사만 한다(살아 있는 팩의 이름을 바꾸지 않는다 — 윈도우 규칙). 실패는 설치를 막지 않는다(경고 1줄).
    match take_update_snapshot(&dir, &items, force, target_version, scope, sources, &manifest) {
        Some(Ok(o)) => {
            println!(
                "[init-pack] 업데이트 전 스냅샷: 이번 설치가 덮거나 지울 파일 {}건 · {:.1}MB → {}",
                o.files,
                o.bytes as f64 / 1_048_576.0,
                o.path.display()
            );
            if !o.skipped.is_empty() {
                eprintln!(
                    "[init-pack] ⚠ 스냅샷 상한(파일 하나 {}MB · 전체 {}MB)을 넘어 담지 않은 파일 {}건 — 스냅샷 색인에 기록했습니다(설치는 계속): {}",
                    UPDATE_SNAPSHOT_FILE_CAP / 1_048_576,
                    UPDATE_SNAPSHOT_TOTAL_CAP / 1_048_576,
                    o.skipped.len(),
                    o.skipped.iter().map(|k| k.rel.as_str()).collect::<Vec<_>>().join(", ")
                );
            }
        }
        Some(Err(e)) => eprintln!("[init-pack] ⚠ 업데이트 전 스냅샷 실패(설치는 계속): {e}"),
        None => {}
    }
    let update_ctx = UpdateContext::new(&dir, scope, sources, manifest_corrupt);
    // ★M11 마: 묘비로 부재를 유지한 파일(rel, 수리 번호) — 같은 실행의 출력에 사실대로 드러낸다.
    let mut absent_kept: Vec<String> = Vec::new();
    // ★M11 가: 사용자 우선 폴백으로 불필요해진 이번 스윕의 병합 전 사본(정리 대상 — 설계 §4-11 가).
    let mut capture_cleanup: Vec<PathBuf> = Vec::new();
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
        let disk = if exists { read_start_live(&path).text() } else { None };
        // 판정 입력과 동일한 사실(존재하나 읽기 실패) — 소유권 술어를 다시 쓰지 않는다(SOT 분산 금지).
        let unreadable = exists && disk.is_none();
        let mhash_raw: Option<String> = manifest.get(rel).cloned();
        // ★C-07: 분류 직전 판정 — 기준선 수리 또는 제자리 보존.
        let stale = classify_stale_entry(
            &dir,
            rel,
            scope,
            content,
            disk.as_deref(),
            mhash_raw.as_deref(),
            pending.contains_key(rel),
        );
        let mhash: Option<String> = match &stale {
            StaleEntry::Repaired(h) => {
                // 기준선을 **디스크에 실제로 있는 과거 릴리스 해시**로 수리한다 — 그러면 Arm 2 가
                // 맞아 깨끗한 벤더 갱신이 되고, 유령 `.user` 와 거짓 "healed" 원장 항목이 사라진다.
                manifest.insert(rel.to_string(), h.clone());
                Some(h.clone())
            }
            _ => mhash_raw.clone(),
        };
        // ★0.15.0 M11 사실 수집(plan_install_with 와 같은 수집기 — '계획 = 실제').
        let facts =
            update_ctx.facts(rel, content, exists, disk.as_deref(), mhash.as_deref(), &pending);
        // ★M11 마: 수리 장부에 기록된 삭제(묘비) — 만들지 않는다. 판정은 decide_file_action_with 의
        // ⓪과 같다(부재 ∧ 묘비 → KeepAbsent). 시드 승계(부서 soul)·병합보다 **먼저** 끊는다 — 승계는 감사
        // 원장에 한 줄을 남기는 부수효과가 있다. 매니페스트 항목은 지운다(설치되지 않은 파일이다 —
        // 묘비가 풀리면 다음 설치가 '기록 없는 부재'로 보고 지금처럼 되살린다).
        if !exists {
            if let Some(repair) = facts.tombstoned.as_deref() {
                manifest.remove(rel);
                absent_kept.push(format!("{rel} ({repair})"));
                kept += 1;
                continue;
            }
        }
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
        // ★`!force` 게이트(gemini-3 r1 major 1) — plan_install 과 같은 이유로 짝을 맞춘다.
        if let (StaleEntry::KeepInPlace(reason), false) = (&stale, force) {
            // ⛔C-07 보수 분기: 디스크 바이트가 과거 릴리스 본이라고 말할 근거가 없다 →
            //   **제자리 보존 + 보고**. 대상도 `.user` 도 원장도 쓰지 않는다(파일시스템 변화 0).
            //   종전(설계 R2)은 여기서 L4 강제 치유로 떨어져 운영자가 쓰지 않은 바이트의 `.user`
            //   백업과 거짓 "healed" 원장 항목을 만들었다(reviewer1 r2 M3).
            kept += 1;
            eprintln!(
                "📌 제자리 보존 '{rel}' — 매니페스트 항목이 낡았고 디스크 바이트의 출처를 \
                 확인할 수 없습니다({reason}) · 아무것도 쓰지 않았습니다"
            );
            continue;
        }
        match decide_file_action_with(
            rel, content, exists, disk.as_deref(), mhash.as_deref(), force, scope,
            verified_base.as_deref(), &facts,
        ) {
            FileAction::KeepAbsent { repair } => {
                // 위 조기 분기와 같은 판정(도달 불가 — 망라 일치용).
                manifest.remove(rel);
                absent_kept.push(format!("{rel} ({repair})"));
                kept += 1;
                continue;
            }
            FileAction::UserKept { reason } => {
                // ★M11 가(사용자 우선): 살아 있는 파일은 사용자 것 그대로 — vendor 신버전은 `.new`.
                // 매니페스트·pristine 은 앞당기지 않는다(재시도 금지와 짝 — 같은 vendor 본이면 다음
                // 스윕은 `parked` 로 곧장 여기 온다: 합치기 전 사본이 다시 생기지 않는다).
                park_user_kept(
                    &dir, rel, content, disk.as_deref(), reason, None, None,
                    &mut pending, &mut pending_dirty, target_version, now_ts,
                );
                kept += 1;
                continue;
            }
            FileAction::VendorFirst { reason } => {
                // ★M11 나(기동 필수 충돌): 사용자 수정본을 먼저 보관(이전 세대는 .user.prev-<시각>) —
                // 보관이 안 되면 덮지 않는다(격리 · 부록 A-2 #4). 보관되면 아래 공용 경로가 vendor 본을 쓴다.
                let user_bytes: Option<Vec<u8>> = read_start_live(&path).bytes();
                match user_bytes.as_deref().map(|b| park_user_copy(&dir, rel, b)) {
                    Some(Ok(parked)) => {
                        // 앞서 user-kept·new-pending 으로 보관하던 `.new` 는 이제 살아 있는 vendor 본과 같아진다 — 치운다.
                        if matches!(
                            pending.get(rel).and_then(|e| e.get("kind")).and_then(|k| k.as_str()),
                            Some("new-pending") | Some("user-kept")
                        ) {
                            let _ = std::fs::remove_file(dir.join(format!("{rel}.new")));
                        }
                        if let serde_json::Value::Object(mut entry) = serde_json::json!({
                            "kind": "vendor-first", "side": format!("{rel}.user"),
                            "reason": reason, "parked_at": now_ts,
                            "version": target_version, "ts": now_ts,
                        }) {
                            // ★T2i(RULINGS EC2): 자리가 끝내 비지 않아 이 사본이 세대 자리에 있으면 그 자리를 함께 적는다
                            //   (겹침은 감사 원장에도).
                            if parked.overlap {
                                entry.insert("overlap_slot".into(), serde_json::json!(parked.at));
                                // ★FH5: 감사 원장에 겹침을 적지 못했으면 그 사실도(사본은 위 자리에 안전하다).
                                if let Some(e) = parked.overlap_record_error.as_deref() {
                                    entry.insert("overlap_audit_error".into(), serde_json::json!(e));
                                }
                            }
                            upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                        }
                        // ★RULINGS AC7: 원장을 vendor 바이트보다 **먼저** 저장한다 — 그 사이에 멈춰도 `.user` 가 기록
                        // 없는 고아가 되지 않는다(다음 스윕은 같은 판정으로 수렴한다).
                        // ★T2h(RULINGS CW5): 저장이 **실패하면** 살아 있는 파일을 바꾸지 않는다 — 기록 없이 바뀐 기동 필수
                        // 파일은 남기지 않는다(보관 실패와 같은 격리 · 기록 가능 회복 후 재스윕이 같은 판정으로 수렴한다).
                        if let Err(e) = save_merge_pending(&dir, &pending) {
                            eprintln!("[init-pack] ⚠ 격리(quarantined): {rel} — 기동 필수 파일의 vendor 우선 기록을 남기지 못했습니다({e}). 파일 무접촉 보존 — 기록 가능 회복 후 재스윕 시 vendor 본 우선 적용.");
                            if let serde_json::Value::Object(entry) = serde_json::json!({
                                "kind": "quarantined", "reason": "ledger-write-failed", "side": rel,
                                "version": target_version, "ts": now_ts,
                            }) {
                                upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                            }
                            kept += 1;
                            continue;
                        }
                    }
                    other => {
                        let why = match other {
                            Some(Err(e)) => e,
                            _ => "사용자본 판독 실패".to_string(),
                        };
                        eprintln!("[init-pack] ⚠ 격리(quarantined): {rel} — 기동 필수 파일의 사용자 수정본을 보관하지 못했습니다({why}). 파일 무접촉 보존 — 보관 가능 회복 후 재스윕 시 vendor 본 우선 적용.");
                        if let serde_json::Value::Object(entry) = serde_json::json!({
                            "kind": "quarantined", "reason": "user-backup-failed", "side": rel,
                            "version": target_version, "ts": now_ts,
                        }) {
                            upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                        }
                        kept += 1;
                        continue;
                    }
                }
            }
            FileAction::Keep { adopt_hash, new_pending } => {
                if adopt_hash {
                    // 디스크 = 임베드: 최신. 매니페스트 공백(구설치본)이면 채택 기록해
                    // 다음 버전부터 자동 갱신 대상이 되게 한다. pristine 승계도 보장.
                    // ★C-07 예방: `or_insert_with` 는 **키가 없을 때만** 넣어 이미 있는 낡은 항목을
                    //   그대로 둔다 — 그것이 세 파일을 낡은 상태로 만든 기제다. 무조건 넣어 불변식을
                    //   세운다: *매니페스트 항목은 설치·채택된 내용의 해시와 항상 같다*
                    //   (쓰기 경로가 이미 하는 것과 같다).
                    manifest.insert(rel.to_string(), content_hash(content));
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
                    if read_start_meta(&new_path).text().as_deref() != Some(content) {
                        let _ = write_atomic(&new_path, content.as_bytes());
                    }
                    upsert_pending(&mut pending, &mut pending_dirty, rel, "new-pending", format!("{rel}.new"));
                } else if !unreadable
                    && matches!(
                        pending.get(rel).and_then(|e| e.get("kind")).and_then(|k| k.as_str()),
                        Some("new-pending") | Some("user-kept")
                    )
                {
                    // 병합 대기 해소(사용자가 vendor 본 채택 등) — 원장·.new 잔재 청소.
                    // ★M11 가: user-kept 도 같다 — 디스크가 vendor 본과 같아졌으면 보관할 것이 없다.
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
                let shell = is_shell_script(rel, content); // ★M11 라: 결과·보존본의 CR 제거 대상
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
                    // ★0.15.0 M11 라: 줄바꿈만 다른 사용자본(CRLF)은 맞춘 뒤 합친다 — 조상·vendor 가 LF 이고
                    //   사용자본만 CRLF 면 LF 로 맞춰 병합한다(merge3.rs 의 '섞인 줄바꿈 = 충돌' 규칙은 그대로).
                    //   결과는 사용자의 CRLF 로 되돌리되, 셸 스크립트는 LF 로 둔다(부록 A-2 #16 · CR 이 든 훅은
                    //   통째로 죽는다). 캡처(①)는 사용자 원본 바이트 그대로다.
                    let user_crlf =
                        ours.contains("\r\n") && !base.contains("\r\n") && !content.contains("\r\n");
                    let ours_norm: std::borrow::Cow<str> = if user_crlf {
                        std::borrow::Cow::Owned(ours.replace("\r\n", "\n"))
                    } else {
                        std::borrow::Cow::Borrowed(ours)
                    };
                    // ② 병합: diffy 순수 Rust(셸아웃 0) — 충돌 마커 = ⑤.
                    let merged = match crate::merge3::merge3(base, &ours_norm, content) {
                        crate::merge3::Merge3Outcome::Clean(m) => m,
                        crate::merge3::Merge3Outcome::Conflict(_) => {
                            return Err("merge-conflict".to_string())
                        }
                    };
                    // ③ 게이트: *.json 파스 + 손상 의심 휴리스틱(clean-but-wrong 차단 · 이식 ⑥).
                    if !crate::merge3::json_gate(rel, &merged) {
                        return Err("json-gate".to_string());
                    }
                    if let Some(rsn) = crate::merge3::suspect_damage(base, &ours_norm, &merged) {
                        return Err(format!("suspect:{rsn:?}"));
                    }
                    let merged = if shell {
                        merged.replace('\r', "")
                    } else if user_crlf {
                        merged.replace("\r\n", "\n").replace('\n', "\r\n")
                    } else {
                        merged
                    };
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
                                if matches!(
                                    pending.get(rel).and_then(|e| e.get("kind")).and_then(|k| k.as_str()),
                                    Some("new-pending") | Some("user-kept")
                                ) {
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
                // ★0.15.0 M11 가: 기동 필수가 아닌 파일 — 어느 실패든(충돌·손상 의심·json 게이트·캡처 실패·
                //   pristine 실패) **사용자 파일이 살아 있는 파일로 남는다**. vendor 신버전은 `.new`, 원장
                //   user-kept. 손상 의심은 덮는 이유가 아니라 합칠 때의 주의 표시(caution)다(설계 §7 2행). 이번
                //   시도의 병합 전 사본은 정리한다 — 사용자본이 살아 있어 그 사본이 지킬 것이 없고, 남기면 같은
                //   보관 파일을 다시 볼 때마다 사본이 쌓인다(설계 §4-11 가 · 정리 장치 부재 3507–3509). pristine
                //   전진 실패만은 디스크에 이미 병합본(사용자 수정 + vendor 변경)이 있으므로 그대로 두고 캡처를
                //   증거로 남긴다.
                if !facts.boot_critical {
                    let caution = fallback_reason
                        .strip_prefix("suspect:")
                        .map(|r| format!("suspect:{r}"));
                    let mut capture_ptr: Option<String> = None;
                    if fallback_reason == "pristine-write-failed" {
                        capture_ptr = capture_seg.as_ref().map(|(_, seg)| format!("{seg}/{rel}"));
                    } else if let Some((seg_path, _)) = capture_seg.as_ref() {
                        if fallback_reason != "capture-failed" {
                            capture_cleanup.push(seg_path.join(rel));
                        }
                    }
                    let live_now: Option<String> = fallback_user_src.clone().or_else(|| disk.clone());
                    park_user_kept(
                        &dir, rel, content, live_now.as_deref(), &fallback_reason, caution,
                        capture_ptr, &mut pending, &mut pending_dirty, target_version, now_ts,
                    );
                    kept += 1;
                    continue;
                }
                // ★RULINGS BO1: 판정 함수는 기동 필수 파일에 `Merge3` 를 내지 않는다 — 아래 갈래는 그 판정으로는 닿지 않는
                // 방어용 폴백이다(누가 기동 필수 사실과 함께 `Merge3` 를 넘겨도 사본 없이 덮지 않고 vendor 우선으로 끝난다).
                // ⑤ 나(기동 필수): vendor 본 우선 폴백 — 현행 healed 폴백 경로 동형 낙하(.user 보존 + vendor
                //   기록 + 조상 사이드카 <rel>.base)이되 원장 kind 는 vendor-first(다시 입히기 = 워커 + 승인)이고
                //   `.user` 를 쓰기 전에 이전 세대를 보존한다(사).
                //   .user 소스 = 그 시점의 디스크 실체(통상 ours=디스크 원본 · pristine-write-failed
                //   만 이미 기록된 merged — v2 §4 크래시표 2행 정합 · ours 원본은 ①캡처가 별도 보존).
                let user_src: Option<String> = fallback_user_src.or_else(|| disk.clone());
                if let Some(d) = user_src.as_deref() {
                    let user_path = dir.join(format!("{rel}.user"));
                    if read_start_meta(&user_path).text().as_deref() != Some(d) {
                        // ★T2i(RULINGS EC2 · FP5 m9 · FP1): 덮지 않는 보관 · vendor 우선이므로 영속 증명(EC7) · 보관이 안 되면
                        //   캡처가 있든 없든 덮지 않는다(기록은 사본이 있을 때만 그 사본을 이름 댄다).
                        let backup = park_copy(&dir, rel, d.as_bytes(), true).map(|_| ());
                        match backup {
                            Ok(()) => {}
                            Err(e) => {
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
                        }
                    }
                    // 충돌 조상 사이드카 <rel>.base — base 전문 기록(사후 3-way 재료 영구 보존 ·
                    // 이식 ① C안 요소 · backup_set side_paths 등재로 rollback 원자성 편입).
                    if let Some(b) = verified_base.as_deref() {
                        let base_path = dir.join(format!("{rel}.base"));
                        if read_start_meta(&base_path).text().as_deref() != Some(b) {
                            let _ = write_atomic(&base_path, b.as_bytes());
                        }
                    }
                    if let serde_json::Value::Object(entry) = serde_json::json!({
                        "kind": "vendor-first", "side": format!("{rel}.user"),
                        "base_side": format!("{rel}.base"),
                        "reason": fallback_reason, "parked_at": now_ts,
                        "version": target_version, "ts": now_ts,
                    }) {
                        upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                    }
                    // ★RULINGS AC7: 원장을 vendor 바이트보다 먼저 저장(위 vendor 우선 가지와 같은 이유).
                    // ★T2h(RULINGS CW5): 저장 실패 = 살아 있는 파일 무접촉(위 가지와 같은 격리).
                    if let Err(e) = save_merge_pending(&dir, &pending) {
                        eprintln!("[init-pack] ⚠ 격리(quarantined): {rel} — 기동 필수 파일의 vendor 우선 기록을 남기지 못했습니다({e}). 파일 무접촉 보존 — 기록 가능 회복 후 재스윕 시 vendor 본 우선 적용.");
                        if let serde_json::Value::Object(entry) = serde_json::json!({
                            "kind": "quarantined", "reason": "ledger-write-failed", "side": rel,
                            "version": target_version, "ts": now_ts,
                        }) {
                            upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                        }
                        kept += 1;
                        continue;
                    }
                }
            }
            FileAction::Write { heal_user_copy } => {
                if heal_user_copy {
                    // system 강제 치유(P0-4)·force 갱신이 사용자 수정본을 덮기 **전에** 보존(파괴 0).
                    // ★0.15.0: 이 가지는 L0(잠금 보안 자산)만 남는다 — 무결성 우선의 명시 예외라 보존은
                    // best-effort 그대로이되, 이전 세대를 덮지 않는다(사 — `.user.prev-<시각>`).
                    if let Some(d) = disk.as_deref() {
                        let user_path = dir.join(format!("{rel}.user"));
                        // ★T2i(RULINGS EC2): 덮지 않는 보관(이전 세대는 덮지 않는다) · 사본이 있는 곳을 안다.
                        let side: Result<String, String> = if read_start_meta(&user_path).text().as_deref() == Some(d) {
                            Ok(format!("{rel}.user"))
                        } else {
                            park_copy(&dir, rel, d.as_bytes(), false).map(|p| p.at)
                        };
                        match side {
                            // ★T3(D7): healed upsert 는 계보 인지 헬퍼 경유 — 크래시 창 재스윕 healed 가
                            // 영속된 merged 항목의 capture 포인터를 지우는 창의 봉인(v2 §4 공통 적용 규칙).
                            Ok(side) => {
                                if let serde_json::Value::Object(entry) = serde_json::json!({
                                    "kind": "healed", "side": side,
                                    "version": target_version, "ts": now_ts,
                                }) {
                                    upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                                }
                            }
                            // ★T2i(RULINGS GD1): 잠금 보안 자산(L0)은 무결성 우선 — 사본을 보관하지 못해도 치유는 간다(설계 §7 5행 ·
                            //   주인 결정 12 · 출시본 그대로). 기록은 참이다: 사본이 없으니 사본을 이름 대지 않고, 보관 실패와 그 사유를 적는다.
                            Err(e) if system_locked(rel) => {
                                eprintln!("[init-pack] ⚠ 보안 자산 {rel}: 사용자 수정본 보관 실패({e}) — 무결성 우선으로 치유 강행(v2 §2 원칙 4 예외)");
                                if let serde_json::Value::Object(entry) = serde_json::json!({
                                    "kind": "healed", "backup": "failed", "backup_error": e,
                                    "version": target_version, "ts": now_ts,
                                }) {
                                    upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                                }
                            }
                            // ★T2i(RULINGS FP1): 그 밖의 파일은 보관 없이는 덮지 않는다(이번 기동은 격리).
                            Err(e) => {
                                eprintln!("[init-pack] ⚠ 격리(quarantined): {rel} — 사용자 수정본을 보관하지 못했습니다({e}). 파일 무접촉 보존 — 보관 가능 회복 후 재스윕 시 정상 치유 대상.");
                                if let serde_json::Value::Object(entry) = serde_json::json!({
                                    "kind": "quarantined", "reason": "backup-failed", "side": rel,
                                    "version": target_version, "ts": now_ts,
                                }) {
                                    upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                                }
                                kept += 1;
                                continue;
                            }
                        }
                    } else if exists {
                        // ★T3 L0/L1 실행부(v2 §3 · §2 원칙 4 · D4·D11): 판독 불가(disk=None) —
                        // 문자열 백업이 구조적으로 못 뜨던 유일 무백업 파괴 edge 를 바이트 백업
                        // (tmp+copy+rename 원자화)으로 봉인한다.
                        // ★0.15.0 사: 이전 세대 보존을 먼저 — 두 번 밀려나도 첫 백업이 덮이지 않는다(CP949
                        // 파일 A·B). 세대 보존 실패는 백업 실패와 같다(fail-closed → 아래 격리).
                        // ★T2i(RULINGS EC2): 세대 보존과 백업이 하나다(`byte_backup_user` → `park_copy`).
                        let backup = byte_backup_user(&dir, rel, &path);
                        match backup {
                            Ok(parked) => {
                                if let serde_json::Value::Object(entry) = serde_json::json!({
                                    "kind": "healed", "side": parked.at,
                                    "version": target_version, "ts": now_ts,
                                }) {
                                    upsert_pending_v2(&mut pending, &mut pending_dirty, rel, entry);
                                }
                            }
                            // ★L0(v2 §3) · ★T2i(RULINGS GD1 — FP1 의 이 갈래를 거둠): 보안 자산은 백업 실패에도 치유 강행(무결성>보존 —
                            //   §2 원칙 4 의 명시 예외 · 주인 결정 12). 원장 side 는 실백업 성공 시에만 — 실패는 side 없이
                            //   backup:"failed" 와 그 사유(존재하지 않는 .user 를 '보존됨'으로 오보하지 않는다 · D11).
                            Err(e) if system_locked(rel) => {
                                eprintln!("[init-pack] ⚠ 보안 자산 {rel}: 바이트 백업 실패({e}) — 무결성 우선으로 치유 강행(v2 §2 원칙 4 예외)");
                                if let serde_json::Value::Object(entry) = serde_json::json!({
                                    "kind": "healed", "backup": "failed", "backup_error": e,
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
        // ★M11 가: user-kept 잔재도 같다(디스크가 vendor 본이 됐다). vendor-first 는 다시 입히기가 남아
        // 있으므로 지우지 않는다.
        if matches!(
            pending.get(rel).and_then(|e| e.get("kind")).and_then(|k| k.as_str()),
            Some("new-pending") | Some("user-kept")
        ) {
            pending.remove(rel);
            pending_dirty = true;
            let _ = std::fs::remove_file(dir.join(format!("{rel}.new")));
        }
        written += 1;
    }
    if pending_dirty {
        let _ = save_merge_pending(&dir, &pending);
    }
    // ★M11 가: 사용자 우선 폴백으로 불필요해진 이번 스윕의 병합 전 사본 정리(파일 → 빈 상위 디렉터리만).
    if !capture_cleanup.is_empty() {
        for f in &capture_cleanup {
            let _ = std::fs::remove_file(f);
        }
        if let Some((seg, _)) = capture_seg.as_ref() {
            remove_empty_dirs_under(seg);
            let _ = std::fs::remove_dir(seg);
        }
    }
    // ★M11 마: 묘비로 부재를 유지한 파일 — 사실대로 한 줄(어느 수리가 부재를 유지하는지).
    if !absent_kept.is_empty() {
        println!(
            "[init-pack] 수리 장부에 기록된 삭제 {}건은 만들지 않았습니다(부재 유지): {} — 되돌리려면 그 수리를 `cys repair revert <번호>`",
            absent_kept.len(),
            absent_kept.join(", ")
        );
    }
    // ★T2j(RULINGS HP4 (a)): 따르지 않은 보호장치 파일의 묘비 — 사실대로 한 번(묘비 파일은 지우지 않았다).
    if !sources.void_tombstones().is_empty() {
        println!(
            "[init-pack] 보호장치 파일의 삭제 표식(묘비) {}건은 따르지 않았습니다 — 보호장치는 승인된 수리로도 지울 수 없어 그 표식은 정당한 기록일 수 없습니다. 그 파일은 다른 파일처럼 설치했고(없으면 되살림) 표식은 지우지 않았습니다: {}",
            sources.void_tombstones().len(),
            sources.void_tombstones().iter().map(|(r, n)| format!("{r} ({n})")).collect::<Vec<_>>().join(", ")
        );
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
        if c.healed + c.merged + c.kept_drift + c.conflicted + c.quarantined + c.user_kept
            + c.vendor_first
            > 0
        {
            println!(
                "[init-pack] pack: healed={} merged={} kept-drift={} conflicted={} quarantined={} user-kept={} vendor-first={} — cys pack-merge 로 확인",
                c.healed, c.merged, c.kept_drift, c.conflicted, c.quarantined, c.user_kept,
                c.vendor_first
            );
        }
        // ★0.15.0 M11: 업데이트 뒤 보고의 설치기 몫(설계 §4-11 "보존된 수정 N건, 합쳐진 것 M건, …,
        // 합치기 대기 J건" — '원본에 반영되어 내린 것'은 수리 장부의 상태라 여기서 세지 않는다).
        if c.user_kept + c.vendor_first > 0 {
            println!(
                "[init-pack] 보존된 수정 {}건(사용자 파일 유지 {} · 제자리 보존 {}) · 합쳐진 것 {}건 · 합치기·다시 입히기 대기 {}건(사용자 파일 유지 + .new {} · 기동 필수 vendor 우선 + 사용자본 보관 {}) — 워커가 부서당 과제 하나로 처리",
                c.user_kept + c.kept_drift,
                c.user_kept,
                c.kept_drift,
                c.merged,
                c.user_kept + c.vendor_first,
                c.user_kept,
                c.vendor_first
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
                match read_start_live(&path).text().ok_or(()) {
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
    // ★0.15.0 M11: 라이브 팩용 입력원 — 그 레인의 묘비·기동 필수 질의·업데이트 전 스냅샷 루트.
    let sources = UpdateSources::for_live_pack(&dir);
    install_into_with(
        dir, items, force, target_version, transactional, true, scope, auth, None, &sources,
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// 팩 atomic swap (v3 §3.1) — init-pack의 파일별 in-place write(중단 시 반쯤 쓰인 팩 =
// stale-packfile 버그 클래스)를 staging 전개→검증→원자 rename 교체로 대체한다.
// ★pack-update는 이미 journal 트랜잭션(apply_pack_transactional)으로 all-or-nothing +
// minisign·sha256 검증을 수행하므로 이 경로를 타지 않는다(중복 래핑=heavily-reviewed 트랜잭션
// 재작성 위험 → 외과성 원칙 준수). run_init_pack(비원자 in-place write)만 이 경로로 승격한다.
// ★T9a (RULINGS GH1 · TW1 run 1 · design §4-15 라): 위 승격은 이제 **첫 설치(팩 폴더 없음)에만** 남는다. 살아 있는 팩을
// 폴더째 바꾸는 교체가 결함 라 자체였다(윈도우: 쥐어진 파일 하나가 rename 을 거부 → 복사 후 삭제가 팩을 반쯤 지웠다).
// 있는 팩은 제자리 파일별 설치로 갱신한다 — 파일별이므로 중단된 실행은 다음 실행이 마저 끝낸다.
// ─────────────────────────────────────────────────────────────────────────────

/// init-pack staging 디렉터리(pack_dir 형제·pid로 격리). pack-update의 고정 `.pack-staging`과
/// 이름을 분리해 동시 실행 충돌을 피한다(doctor가 `.pack-staging*` 잔재를 정리한다).
pub fn init_staging_dir(dir: &Path) -> PathBuf {
    let parent = dir.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!(".pack-staging-init-{}", std::process::id()))
}

/// pack_dir 형제 `<pack_dir>.prev` 의 이름. ★T9a (RULINGS GH1 · GJ1 · GJ3): 0.15.0 은 이 폴더를 만들지도, 읽지도, 옮기지도,
/// 지우지도 않는다 — 있다면 0.15.0 이전 버전의 통째 교체가 남긴 것이다(되돌리기의 원천은 업데이트 전 스냅샷뿐 — 설계 §7 13행).
pub fn pack_prev_dir(dir: &Path) -> PathBuf {
    PathBuf::from(format!("{}.prev", dir.display()))
}

/// 재귀 디렉터리 복사(파일=fs::copy로 권한 보존, 하위 dir 재귀). 팩 자체는 심링크를 배포하지 않는다(오너
/// 결정 — 심링크 마이그레이션 안 함). staging 전량 복사로 상태파일·user-edit·비임베드·디렉티브를 보존한다.
/// ★0.15.0 M11(설계 §7 20행 "링크를 링크로 보존한다"): 사용자가 만든 심볼릭 링크는 유닉스에서 **링크로**
/// 복사한다 — 종전 `fs::copy` 는 링크를 따라가 파일 링크를 사본으로 바꾸고, 대상이 없는 링크에서는
/// 실패해 `init-pack` 이 exit 1 로 멈췄다(조사 손실 시나리오 20). 윈도우는 개발자 모드 없이 링크를 만들 수
/// 없으므로 지금 동작 그대로다(운영체제 제약).
/// ★T9a (RULINGS GN1 · S3): 폴더와 파일을 전부 **배타적으로** 만든다 — 대상(또는 그 안의 무엇이든)이 이미 있으면
/// `AlreadyExists` 로 즉시 멈추고 아무것도 바꾸지 않는다(덮어쓰기 0). 호출자(첫 설치의 게시)는 그때 이미 있는 팩을
/// 제자리 설치로 갱신한다. `dst` 의 상위 폴더는 필요하면 만든다.
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        #[cfg(unix)]
        if ft.is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(&from)?, &to)?;
            continue;
        }
        if ft.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            copy_file_new(&from, &to)?;
        }
    }
    Ok(())
}

/// ★T9a (RULINGS GN1): 결코 바꾸지 않는 파일 복사 — 대상은 `create_new`(있으면 `AlreadyExists`)로 만들고, 원본의 권한을
/// 입힌다(`fs::copy` 가 하던 대로).
fn copy_file_new(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut input = std::fs::File::open(from)?;
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    std::io::copy(&mut input, &mut output)?;
    std::fs::set_permissions(to, input.metadata()?.permissions())
}

/// The rename every move of the pack swap goes through ([`publish_staging_with`]). Production passes `std::fs::rename`;
/// tests install a refusing one through [`swap_seam`] (RULINGS FS5).
type SwapRename = dyn Fn(&Path, &Path) -> std::io::Result<()>;

/// Test-only seam for [`atomic_swap`] (RULINGS FS5): a rename installed here replaces `std::fs::rename` for every move
/// of the swap on THIS thread until the guard drops — so a test can drive the real `install_staged` → `atomic_swap`
/// path with a refused rename. Thread-local: parallel tests never see each other's seam.
#[cfg(test)]
mod swap_seam {
    use std::cell::RefCell;
    use std::path::Path;
    use std::rc::Rc;

    thread_local! {
        static RENAME: RefCell<Option<Rc<super::SwapRename>>> = const { RefCell::new(None) };
    }

    pub(super) fn current() -> Option<Rc<super::SwapRename>> {
        RENAME.with(|r| r.borrow().clone())
    }

    /// Removes the installed rename when dropped (also on panic unwinding).
    pub(super) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            RENAME.with(|r| *r.borrow_mut() = None);
        }
    }

    #[must_use]
    pub(super) fn install(rename: impl Fn(&Path, &Path) -> std::io::Result<()> + 'static) -> Guard {
        RENAME.with(|r| *r.borrow_mut() = Some(Rc::new(rename)));
        Guard
    }
}

/// cross-device 대비 rename(§3.1-5) — 같은 볼륨이면 원자 rename, 실패 시 copy 후 원본 삭제
/// fallback(EXDEV 등). staging은 pack_dir 형제라 정상 경로는 원자 rename이다(Windows도 동일 볼륨 전제).
/// ★T9a (RULINGS FS2 · GH1): `src` is ONLY ever the staging folder this install itself just wrote (a first install's
/// publish) — the one place the copy-then-delete fallback remains. A live pack is never its source: an existing pack is
/// updated in place by [`install_staged`] and never moved as a folder.
/// ★T9a (RULINGS GN1 · S3): rename 이 실패했을 때 대상이 **그사이 생겼으면** 아무것도 복사하지 않고 `AlreadyExists` 를
/// 돌려준다(다른 설치기 — 데몬의 기동 설치·다른 init-pack — 가 만든 팩 위에 분류 없이 덮어쓰지 않는다). 복사 fallback 은
/// 대상이 아직 없을 때만이고, 그 복사도 배타적이다([`copy_dir_all`]).
fn rename_dir_or_move(src: &Path, dst: &Path, rename: &SwapRename) -> std::io::Result<()> {
    if rename(src, dst).is_ok() {
        return Ok(());
    }
    if std::fs::symlink_metadata(dst).is_ok() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} 가 그사이 생겼습니다 — 복사하지 않습니다", dst.display()),
        ));
    }
    // rename 불가(cross-device 등) — 아직 없는 대상에 배타적 copy 후 원본 삭제 fallback(src 부재면 여기서 loud Err).
    copy_dir_all(src, dst)?;
    std::fs::remove_dir_all(src)
}

/// 첫 설치의 게시(§3.1): staging → pack_dir, rename 하나(staging 의 copy fallback 만 허용).
///
/// ★T9a (RULINGS GH1 · design §4-15 라 "살아 있는 … 팩 폴더의 이름을 바꾸지 않고, 제자리에서 … 적용"): pack_dir 자리에
/// 무엇이든 이미 있으면 거부한다 — 살아 있는 팩은 폴더째 바꾸지 않는다(기존 팩의 갱신은 [`install_staged`] 의 제자리
/// 파일별 설치). 그래서 이 함수는 살아 있는 팩도 `pack.prev` 도 결코 건드리지 않는다(옮기기·지우기·만들기 0).
/// 반환 Err = 아무것도 바꾸지 않았거나(자리에 항목이 있음 · staging rename/copy 가 시작 전에 실패), staging 의 copy
/// fallback 이 도중에 멈춰 pack_dir 자리에 staging 의 일부 사본이 남았다 — 그 폴더는 다음 실행이 제자리 설치로 채운다.
pub fn atomic_swap(dir: &Path, staging: &Path) -> Result<(), String> {
    publish_staging(dir, staging).map_err(|r| match r {
        PublishRefused::Occupied => format!(
            "팩 자리에 이미 항목이 있습니다({}) — 살아 있는 팩은 폴더째 바꾸지 않습니다(기존 팩은 파일별 제자리 갱신) · \
             아무것도 바꾸지 않았습니다",
            dir.display()
        ),
        PublishRefused::Failed(e) => e,
    })
}

/// 첫 설치 게시가 일어나지 않은 이유.
enum PublishRefused {
    /// 팩 자리에 무엇이 있다(처음부터, 또는 rename 사이에 다른 설치기가 만들었다) — 아무것도 복사하지 않았다.
    Occupied,
    /// 그 밖의 실패(rename·배타 복사의 IO 오류).
    Failed(String),
}

/// [`atomic_swap`] 의 본체 — 테스트 이음새(swap_seam)를 거친 rename 으로 게시한다.
fn publish_staging(dir: &Path, staging: &Path) -> Result<(), PublishRefused> {
    #[cfg(test)]
    if let Some(rename) = swap_seam::current() {
        return publish_staging_with(dir, staging, &*rename);
    }
    publish_staging_with(dir, staging, &|src: &Path, dst: &Path| std::fs::rename(src, dst))
}

/// [`publish_staging`] with the rename it uses — the test seam of RULINGS FS5 / GH2 / GN1 (tests count the folder renames
/// and create the pack between the absence check and the rename).
fn publish_staging_with(dir: &Path, staging: &Path, rename: &SwapRename) -> Result<(), PublishRefused> {
    if std::fs::symlink_metadata(dir).is_ok() {
        return Err(PublishRefused::Occupied);
    }
    match rename_dir_or_move(staging, dir, rename) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(PublishRefused::Occupied),
        Err(e) => Err(PublishRefused::Failed(format!("staging→pack_dir rename 실패: {e}"))),
    }
}

/// staging 검증(§3.1-2): 임베드 전 파일이 staging에 실재하는가(파일 수·존재). pack-update의
/// sha256·minisign 검증은 pack-update 경로(packsig)가 이미 수행하므로, init-pack staging은
/// 존재·수 검증이다(디스크 오류로 반쯤 쓰인 staging을 교체 전에 차단하는 방어선).
pub fn verify_staging(staging: &Path, items: &[(&str, &str)]) -> Result<(), String> {
    verify_staging_with(staging, items, &std::collections::BTreeSet::new())
}

/// ★0.15.0 M11 마(설계 §4-11 "이 검사를 묘비와 함께 바꾸는 묶음에 넣는다"): **허가된 부재**(그 레인의 묘비가
/// 부재를 유지하는 파일 — 생명줄은 결코 들어오지 않는다)와 **덜 만들어진 준비본**을 구분한다. 허가 집합에
/// 없는 누락은 지금처럼 실패다(앱이 좌석 복원을 보류하는 근거 — 부록 A-2 #13). 링크는 링크로 보존되므로
/// (copy_dir_all) 대상 파일 존재 대신 항목 존재(`symlink_metadata`)도 인정한다(대상이 없는 링크 포함).
pub fn verify_staging_with(
    staging: &Path,
    items: &[(&str, &str)],
    authorised_absent: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let mut missing = 0usize;
    let mut first: Option<String> = None;
    for (rel, _) in items {
        let p = staging.join(rel);
        let present = p.is_file()
            || std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink());
        if !present && !authorised_absent.contains(*rel) {
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

/// init-pack 설치(§3.1). 반환: (written, kept).
/// ★T9a (RULINGS GH1 · design §4-15 라 "살아 있는 … 팩 폴더의 이름을 바꾸지 않고, 제자리에서 … 적용"):
/// - pack_dir 이 **있으면** 제자리 갱신 — 데몬의 기동 설치가 팩에 돌리는 것과 같은 파일별 설치(`install_into_with`:
///   같은 규칙·입력원·스코프·캡처 레인·버전 · 업데이트 전 스냅샷은 첫 쓰기 전)를 팩 자체에 돌리고, 같은 존재 검증과
///   같은 config 격리 셋업을 한다. 팩 사본·staging·팩 폴더 rename·`pack.prev` 없음. 파일별이지 전부-아니면-전무가
///   아니다: 바꿀 수 없는 파일 하나(윈도우에서 쥐어진 파일)가 그 자리에서 실행을 Err 로 멈추고(앞 파일은 갱신됨 ·
///   `.pack-version` 은 그대로라 데몬의 설치 관문도 열린 채), 다음 실행이 마저 끝낸다. 지우는 것은 없다.
/// - pack_dir 이 **없으면**(첫 설치) staging 전개 → install_into 반영 → 검증 → rename 하나로 게시([`atomic_swap`]) →
///   config 격리 셋업. 게시 전 중단은 pack_dir 을 만들지 않는다. ★RULINGS GN1: 그사이 다른 설치기가 팩을 만들었으면
///   (rename 이 거부되고 대상이 이제 있다) 그 위에 아무것도 복사하지 않고 staging 을 버린 뒤 위의 제자리 갱신을 돈다.
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
    // ★T9a (RULINGS GH1): 있는 팩 = 제자리 파일별 설치(데몬의 기동 설치와 같은 호출 — 차이는 force·config 셋업 시점·
    // 존재 검증뿐) → 같은 존재 검증 → 같은 config 셋업. 폴더째 교체는 없다.
    let in_place = || -> Result<(usize, usize), String> {
        let items: Vec<(&str, &str)> = PACK_ALL.iter().map(|(r, c)| (*r, *c)).collect();
        let sources = UpdateSources::for_live_pack(&dir);
        let (written, kept) = install_into_with(
            dir.clone(),
            items.iter().copied(),
            force,
            env!("CARGO_PKG_VERSION"),
            false,
            false,
            scope,
            auth, // 라이브 팩에 직접 쓴다 — 데몬의 기동 설치처럼 인가를 그대로 넘긴다.
            capture_ns.as_deref(),
            &sources,
        )?;
        verify_staging_with(&dir, &items, &sources.authorised_absent())
            .map_err(|e| format!("제자리 갱신 뒤 팩 검사 — {e}"))?;
        setup_isolated_config_dir(install_hooks);
        Ok((written, kept))
    };
    if dir.is_dir() {
        return in_place();
    }
    let staging = init_staging_dir(&dir);
    // 잔여 staging(같은 pid 재사용·직전 실패) 선정리.
    let _ = std::fs::remove_dir_all(&staging);
    // ① 첫 설치 — 빈 staging(있는 팩은 위에서 제자리 갱신했다 · 여기 오는 것은 팩 폴더가 없을 때뿐).
    std::fs::create_dir_all(&staging)
        .map_err(|e| format!("staging 생성 실패 {}: {e}", staging.display()))?;
    // ② 임베드 반영을 staging에(config 격리 셋업은 교체 후 실 dir에 — setup_config=false).
    let items: Vec<(&str, &str)> = PACK_ALL.iter().map(|(r, c)| (*r, *c)).collect();
    // ★0.15.0 M11: 입력원은 **논리 대상**(pack_dir)에서 — 레인(묘비)·스냅샷은 staging 이 아니라 그 팩의 것.
    let sources = UpdateSources::for_live_pack(&dir);
    let (written, kept) = match install_into_with(
        staging.clone(),
        items.iter().copied(),
        force,
        env!("CARGO_PKG_VERSION"),
        false,
        false,
        scope, // 논리 대상(pack_dir)의 스코프 — staging basename 재판정 금지(위 주석).
        None, // staging은 비라이브 형제 경로 — 여기 쓰기는 인가 불요(라이브 인가는 위 swap 게이트가 담당).
        capture_ns.as_deref(), // 캡처 레인 = 논리 대상 basename(스코프와 동일 주입 계약).
        &sources,
    ) {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    // ③ 검증(존재·수) — 실패 시 staging 폐기·교체 안 함(기존 팩 온전). ★M11 마: 묘비가 부재를 유지하는
    // 파일은 허가된 부재다(덜 만들어진 준비본과 구분 — 그 밖의 누락은 지금처럼 실패).
    if let Err(e) = verify_staging_with(&staging, &items, &sources.authorised_absent()) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    // ④ 게시(rename 하나 · 실패 시 staging 정리). ★T9a (RULINGS GN1 · S3): 그사이 다른 설치기가 팩을 만들었으면 그 위에
    //   아무것도 복사하지 않는다 — staging 을 버리고, 이제 있는 팩을 제자리 설치로 갱신한다(그 안의 모든 파일이 갱신
    //   규칙을 거친다).
    match publish_staging(&dir, &staging) {
        Ok(()) => {}
        Err(PublishRefused::Occupied) if dir.is_dir() => {
            let _ = std::fs::remove_dir_all(&staging);
            return in_place();
        }
        Err(refused) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(match refused {
                PublishRefused::Occupied => format!(
                    "팩 자리에 폴더가 아닌 항목이 있습니다({}) — 아무것도 바꾸지 않았습니다",
                    dir.display()
                ),
                PublishRefused::Failed(e) => e,
            });
        }
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

/// ★M15-다 / GX4 (0.15.0 「스스로 해결」 T9): the journal location of binaries BEFORE 0.15.0 — ONE folder shared by every
/// lane (`<pack parent>/.pack-journal`). A journal there names neither its lane nor its apply, so guessing its owner is
/// the defect itself (a department's start rolled the base's bytes into its own pack — stage-1 tests `m15_da_*`). No
/// lane reads it into a pack, moves it or deletes it; [`legacy_pack_journal_notice`] names it with the remedy.
const PACK_JOURNAL_DIR: &str = ".pack-journal";

/// ★GX1: the parent of every lane's OWN journal folder — `<pack parent>/.pack-journal.d/<lane>/`. A new name, so no
/// older binary (which reads only [`PACK_JOURNAL_DIR`]) ever sees a new journal.
pub const PACK_JOURNAL_ROOT: &str = ".pack-journal.d";

/// ★GX2: the transaction's LAST hard commit — a marker INSIDE the pack carrying THIS apply's id (+ target tuple + items
/// digest). Recovery calls an apply finished only when this marker carries the journal's apply id — never by
/// `.pack-version`, which a same-base-version revision (다-2) leaves unchanged.
pub const PACK_APPLIED_FILE: &str = ".pack-applied.json";

/// ★HE2 (T9 round 2): the lane's ONE journal slot `L/active/` — it exists exactly while a journal is unsettled or being
/// cleaned, and holds at most one entry named `j.<pid>.<birth|x>.<id>` (its writer), `r.<pid>.<birth|x>.<id>` (a taker),
/// `u.<id>` (given back, nobody's) or `d.<id>` (done — only removal left). `<id>` = the first 16 hex of the apply id.
/// Who wrote or holds a journal is read from the NAME (HB1 P4); nothing else is.
const JOURNAL_SLOT: &str = "active";

/// ★GX1: THIS lane's own journal folder (lane = the pack folder's name: `pack`, `pack-dept-<name>`).
pub fn pack_journal_dir() -> PathBuf {
    pack_journal_dir_for(&pack_dir())
}

/// ★GX1: the journal folder of the lane whose pack is `pack`.
pub fn pack_journal_dir_for(pack: &Path) -> PathBuf {
    let root = pack
        .parent()
        .map(|p| p.join(PACK_JOURNAL_ROOT))
        .unwrap_or_else(|| PathBuf::from(PACK_JOURNAL_ROOT));
    match pack.file_name() {
        Some(lane) => root.join(lane),
        None => root.join("pack"),
    }
}

/// ★GX4: the old shared location beside `pack` (named only — never read, moved or deleted).
pub fn legacy_pack_journal_dir_for(pack: &Path) -> PathBuf {
    pack.parent()
        .map(|p| p.join(PACK_JOURNAL_DIR))
        .unwrap_or_else(|| PathBuf::from(PACK_JOURNAL_DIR))
}

/// ★GX4: one line naming a legacy journal beside `pack` with the remedy; `None` when there is none. A stat only.
pub fn legacy_pack_journal_notice(pack: &Path) -> Option<String> {
    let p = legacy_pack_journal_dir_for(pack);
    std::fs::symlink_metadata(&p).ok()?;
    Some(format!(
        "이전 버전이 남긴 팩 업데이트 복구 기록 {} — 어느 레인의 것인지 적혀 있지 않아 어떤 레인도 되돌리거나 \
         옮기거나 지우지 않습니다(그대로 보존). 해결: 팩 업데이트를 다시 실행하십시오.",
        p.display()
    ))
}

/// The legacy notice is printed once per process (cysd's start, a pack update's two recovery calls).
static LEGACY_JOURNAL_NAMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// ★GX3: who wrote a journal — pid + the process's native start time (µs since the epoch, as
/// [`crate::process_observation`] reads it; `None` = it could not be read when the journal was written).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct JournalWriter {
    pid: u32,
    #[serde(default)]
    birth_us: Option<u64>,
}

impl JournalWriter {
    /// `<pid>.<birth|x>` — the identity part of a journal name (HE2).
    fn tag(&self) -> String {
        match self.birth_us {
            Some(b) => format!("{}.{b}", self.pid),
            None => format!("{}.x", self.pid),
        }
    }

    fn current() -> Self {
        let pid = std::process::id();
        let birth_us = crate::process_observation::observe(pid)
            .ok()
            .map(|o| birth_us_of(&o.identity));
        JournalWriter { pid, birth_us }
    }
}

fn birth_us_of(i: &crate::process_observation::Identity) -> u64 {
    i.birth_seconds
        .saturating_mul(1_000_000)
        .saturating_add(i.birth_microseconds)
}

/// ★GX3: the writer's state from ONE observation — nothing waits.
#[derive(Debug, PartialEq, Eq)]
enum WriterLife {
    /// The recorded process is gone (no such process, or the pid now belongs to a process born at another time).
    Gone,
    /// The recorded process is still running — its journal is not touched.
    Running,
    /// Liveness cannot be decided — the journal is not touched now; the next start looks again.
    Unknown(String),
}

/// ★GX3 (pure — the one decision rule, unit-tested): `seen` = the birth (µs) of the process now holding `w.pid`, or the
/// observation's failure.
fn writer_life(
    w: &JournalWriter,
    seen: Result<u64, crate::process_observation::Failure>,
) -> WriterLife {
    use crate::process_observation::Failure;
    match (seen, w.birth_us) {
        (Err(Failure::Dead), _) => WriterLife::Gone,
        (Ok(now), Some(recorded)) if now != recorded => WriterLife::Gone,
        (Ok(_), Some(_)) => WriterLife::Running,
        (Ok(_), None) => WriterLife::Unknown(format!(
            "작성 프로세스(pid {})의 시작 시각이 기록되지 않았고 그 번호의 프로세스가 살아 있음",
            w.pid
        )),
        (Err(e), _) => WriterLife::Unknown(format!("작성 프로세스(pid {}) 확인 불가: {e:?}", w.pid)),
    }
}

fn observe_writer(w: &JournalWriter) -> WriterLife {
    writer_life(
        w,
        crate::process_observation::observe(w.pid).map(|o| birth_us_of(&o.identity)),
    )
}

/// ★GX2: what the commit marker (and the journal's index) says about ONE apply.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct PackApplied {
    apply_id: String,
    /// The target tuple (base version, channel, revision).
    target: PackState,
    /// [`pack_items_sha256`] of the applied items.
    items_digest: String,
}

/// ★GX2: a fresh id for one apply (process, clock, sequence, content) — unique per apply, not a secret.
fn new_apply_id(items_digest: &str, target: &PackState) -> String {
    use sha2::{Digest, Sha256};
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = Sha256::new();
    h.update(format!(
        "{}\0{nanos}\0{}\0{items_digest}\0{}\0{}\0{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        target.base_version,
        target.channel,
        target.pro_revision
    ));
    format!("{:x}", h.finalize())[..32].to_string()
}

/// ★GX6 / HE7: the step boundaries of the transaction. The production entry passes a callback that always continues; the
/// crash proof stops the REAL transaction right after each boundary, as a dying process would (no rollback, no settle).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TxStep {
    /// ⓪ done (one recovery pass over the lane's slot).
    Recovered,
    /// The admission folder `L/t.<me>.<seq>/j.<me>.<id>/` prepared; the admission rename not yet done.
    Prepared,
    /// HE3: admitted — `L/active/j.<me>.<id>/` is the lane's one journal (empty).
    Admitted,
    /// ① every backup written, the index not yet.
    BackedUp,
    /// ① the index written (the journal is complete).
    Journaled,
    /// ② the files applied (install record included).
    Installed,
    /// ③ `.pack-state.json` written.
    StateWritten,
    /// ④ `.pack-version` written.
    VersionWritten,
    /// ④' the commit marker written — the apply is finished from here on.
    Committed,
    /// ⑤ the post-commit callback returned.
    PostCommitted,
    /// HE5: the writer's own journal renamed to `d.<id>` (settled); its removal not yet done.
    Settled,
}

fn stopped_at(s: TxStep) -> String {
    format!("[pack-journal] transaction stopped after {s:?} (crash proof seam)")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JournalEntry {
    rel: String,
    /// apply 전 파일이 존재했는가. false면 rollback 시 (신규 생성분) 삭제.
    existed: bool,
    /// M15-가 (0.15.0 「스스로 해결」 · design §2-11 가, §4-15 가): the file's permission mode before the
    /// apply ([`current_mode`], Unix only), restored exactly by [`rollback_journal`]. Before 0.15.0 the
    /// rollback rewrote every journaled file with the plain writer, so every script came back without
    /// its exec bit while `.pack-version` stayed old — no later install restored it, and the built-in
    /// jobs guarded by `[ -x … ] || exit 0` stopped silently.
    /// Optional on purpose, so the index stays readable in both directions: absent on Windows (no POSIX
    /// mode — the JSON is then byte-identical to before) and in journals written by older binaries
    /// (restore falls back to [`pack_target_mode`]); an older binary reading this field ignores it
    /// (serde's default for unknown fields — neither struct denies them) and restores bytes as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<u32>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JournalIndex {
    /// 이번 트랜잭션의 목표 pack_version(= 커밋 성공 시 `.pack-version`에 기록되는 값).
    /// recovery는 디스크 `.pack-version`이 이 값과 같은지로 커밋 완료를 판정한다.
    target_version: String,
    entries: Vec<JournalEntry>,
    /// ★GX2: THIS apply (id + target tuple + items digest). Optional only so that an index without it parses — such an
    /// index (not written by this version) is never acted on by [`recover_pack_journal`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    apply: Option<PackApplied>,
}

/// apply 전 backup journal 작성: backup_set의 각 파일 기존 bytes를 저널에 복사(+fsync)하고
/// 인덱스(목표 버전·existed 플래그)를 기록(+fsync)한다.
/// ★HE5 (T9 round 2): written ONLY into the apply's own admitted folder `own` = `L/active/j.<me>.<id>` (empty when it is
/// called — HE3 admitted it); the backups first, the index (with the apply record) LAST. Returns `Ok(false)` when the
/// step callback stopped it (the crash proof).
fn write_journal(
    own: &Path,
    target_version: &str,
    backup_set: &std::collections::BTreeSet<String>,
    apply: &PackApplied,
    step: &mut dyn FnMut(TxStep) -> bool,
) -> Result<bool, String> {
    let files_dir = own.join("files");
    std::fs::create_dir_all(&files_dir)
        .map_err(|e| format!("journal files dir 생성 실패 {}: {e}", files_dir.display()))?;
    let dir = pack_dir();
    let mut entries = Vec::new();
    for rel in backup_set {
        let src = dir.join(rel);
        if src.is_file() {
            // M15-가: the mode is read before any write (tmp + rename drops it).
            let mode = current_mode(&src);
            let bytes = std::fs::read(&src)
                .map_err(|e| format!("journal 백업 읽기 실패 {}: {e}", src.display()))?;
            let dst = files_dir.join(rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("journal 백업 dir 실패 {}: {e}", parent.display()))?;
            }
            write_atomic(&dst, &bytes)
                .map_err(|e| format!("journal 백업 쓰기 실패 {}: {e}", dst.display()))?;
            entries.push(JournalEntry {
                rel: rel.clone(),
                existed: true,
                mode,
            });
        } else {
            entries.push(JournalEntry {
                rel: rel.clone(),
                existed: false,
                mode: None,
            });
        }
    }
    if !step(TxStep::BackedUp) {
        return Ok(false);
    }
    let index = JournalIndex {
        target_version: target_version.to_string(),
        entries,
        apply: Some(apply.clone()),
    };
    let json =
        serde_json::to_vec_pretty(&index).map_err(|e| format!("journal 인덱스 직렬화 실패: {e}"))?;
    // 인덱스는 마지막에(원자) — 인덱스 부재 = '백업 미완 = 미커밋'(원본 미변경)을 의미.
    write_atomic(&own.join("index.json"), &json)
        .map_err(|e| format!("journal 인덱스 쓰기 실패: {e}"))?;
    Ok(true)
}

/// M15-가 (0.15.0 · design §4-15 가): put one journaled file back WITH its mode.
/// - recorded mode → exactly that mode, set on the tmp descriptor before the rename
///   ([`write_atomic_mode_exact`]: umask-independent, read back and verified);
/// - no recorded mode (a journal written by an older binary; every entry on Windows) → the installer's
///   single mode rule [`pack_target_mode`]`(current mode, bytes, true)`: an existing exec mode is kept,
///   a `#!` file gets 0755, any other mode is kept (the design's second method, "되돌린 뒤 실행 권한
///   처리를 다시 돈다").
///
/// The bytes are the rollback's first duty: if the filesystem refuses the mode, the bytes are still
/// restored (mode requested through `open(2)`, so never wider than asked) and the failure is printed —
/// a mode never aborts a rollback halfway.
fn restore_journaled_file(
    target: &Path,
    bytes: &[u8],
    recorded: Option<u32>,
) -> Result<(), String> {
    let mode = recorded.or_else(|| pack_target_mode(current_mode(target), bytes, true));
    match write_atomic_mode_exact(target, bytes, mode) {
        Ok(()) => Ok(()),
        Err(exact) if mode.is_some() => {
            write_atomic_mode(target, bytes, mode)
                .map_err(|e| format!("journal 복원 쓰기 실패 {}: {e}", target.display()))?;
            eprintln!(
                "[pack-journal] ⚠ mode {:04o} could not be set on {} ({exact}) — bytes restored, mode \
                 as far as the filesystem allows",
                mode.unwrap_or(0),
                target.display()
            );
            Ok(())
        }
        Err(e) => Err(format!("journal 복원 쓰기 실패 {}: {e}", target.display())),
    }
}

/// 저널에서 pre-state로 복원: existed=true는 백업 bytes를 원위치 atomic 복원, existed=false는 (신규 생성분) 삭제.
/// M15-가 (0.15.0): bytes AND mode — see [`restore_journaled_file`].
/// ★HE4 / HE5 (T9 round 2): rolls back FROM THE FOLDER IT IS GIVEN (the writer's own `j.` or a taker's held `r.`) — no
/// function reads "the lane's journal path" any more; `.pack-version` is in the backup set (GX2), so it is restored too.
/// It never removes the folder: removal happens only after SETTLE (rename to `d.<id>`). `restored(n)` is asked after
/// each restored entry (the crash proof; production always continues): `Ok(false)` = stopped there, `Ok(true)` = every
/// entry restored, `Err` = a failure on the way (the folder and all its backups are still there).
fn rollback_journal(held: &Path, restored: &mut dyn FnMut(usize) -> bool) -> Result<bool, String> {
    let index_path = held.join("index.json");
    // ★T2i(RULINGS FP2): 데몬이 듣기 전의 판독 — 시작 경로의 판독기(막히지 않음 · 일반 파일 · 상한). 판독 불가 = 종전 읽기 오류와
    //   같은 갈래(Err — 복구를 하지 않고 그대로 둔다).
    let s = journal_text(&index_path, "journal 인덱스 읽기 실패")?;
    let index: JournalIndex =
        serde_json::from_str(&s).map_err(|e| format!("journal 인덱스 파싱 실패: {e}"))?;
    let dir = pack_dir();
    let files_dir = held.join("files");
    for (n, entry) in index.entries.iter().enumerate() {
        let target = dir.join(&entry.rel);
        if entry.existed {
            let backup = files_dir.join(&entry.rel);
            let bytes = match read_start_meta(&backup) {
                StartRead::Bytes(b) => b,
                StartRead::Missing => return Err(format!("journal 백업 복원 읽기 실패 {}: 없음", backup.display())),
                StartRead::Unreadable(e) => {
                    return Err(format!("journal 백업 복원 읽기 실패 {}: {e}", backup.display()))
                }
            };
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            restore_journaled_file(&target, &bytes, entry.mode)?;
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
        if !restored(n + 1) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// ★T2i(RULINGS FP2): 저널 메타 파일의 텍스트 — 시작 경로의 판독기로(없음 · 판독 불가 · UTF-8 아님 = Err, 종전 읽기 오류와 같은 갈래).
fn journal_text(path: &Path, what: &str) -> Result<String, String> {
    match read_start_meta(path) {
        StartRead::Bytes(b) => String::from_utf8(b).map_err(|e| format!("{what} {}: UTF-8 아님({e})", path.display())),
        StartRead::Missing => Err(format!("{what} {}: 없음", path.display())),
        StartRead::Unreadable(e) => Err(format!("{what} {}: {e}", path.display())),
    }
}

/// ★HE2: one entry of the slot `L/active/`, parsed from its NAME only (strict; anything else is foreign).
#[derive(Clone, Debug, PartialEq, Eq)]
enum SlotEntry {
    /// `j.<pid>.<birth|x>.<id>` — owned by its writer (the apply).
    Writer { pid: u32, birth: Option<u64>, id: String },
    /// `r.<pid>.<birth|x>.<id>` — owned by a taker (a recovery).
    Taker { pid: u32, birth: Option<u64>, id: String },
    /// `u.<id>` — given back, nobody's; the outcome is not settled.
    GivenBack { id: String },
    /// `d.<id>` — done: the outcome is settled, only removal is left.
    Done { id: String },
}

fn is_journal_id(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn parse_slot_entry(name: &str) -> Option<SlotEntry> {
    let parts: Vec<&str> = name.split('.').collect();
    let digits = |s: &str, max: usize| !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_digit());
    match parts.as_slice() {
        [k @ ("j" | "r"), pid, birth, id] if digits(*pid, 10) && (*birth == "x" || digits(*birth, 20)) && is_journal_id(id) => {
            let pid: u32 = pid.parse().ok()?;
            let birth = if *birth == "x" { None } else { Some(birth.parse().ok()?) };
            let id = (*id).to_string();
            Some(if *k == "j" { SlotEntry::Writer { pid, birth, id } } else { SlotEntry::Taker { pid, birth, id } })
        }
        ["u", id] if is_journal_id(id) => Some(SlotEntry::GivenBack { id: (*id).to_string() }),
        ["d", id] if is_journal_id(id) => Some(SlotEntry::Done { id: (*id).to_string() }),
        _ => None,
    }
}

/// The 16-hex short form of an apply id (the names' `<id>`).
fn journal_id16(apply_id: &str) -> String {
    apply_id.chars().take(16).collect()
}

/// ★HG1: the protocol's ONE name-conditional move (admission, take, give-back, settle). Unix: `std::fs::rename` = one
/// `rename(2)` (refuses a non-empty directory target; replaces an empty one — proved safe, HG1). Windows: the existing
/// [`windows_move_no_replace`] = one `MoveFileExW` without replace (never replaces any existing target; not std's rename,
/// which may fall back to a handle-based rename and may replace an empty directory).
fn journal_move(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        windows_move_no_replace(from, to)
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
}

/// ★HE4 SETTLE: `held` → `L/active/d.<id>` FIRST (the atomic "nothing more will be written for this journal"), only then
/// the recursive removal of `d.<id>` BY ITS EXACT NAME and the non-recursive removal of the slot (HG2: the slot itself is
/// never removed recursively; no retry of our own — HG3). `after_rename` is the crash-proof seam (`false` = stop there).
/// `Ok(true)` = removed; `Ok(false)` = stopped after the rename; `Err` = the rename failed (the folder is left as it is)
/// or the removal failed (the next entry removes `d.<id>`).
fn settle_journal(held: &Path, id: &str, after_rename: &mut dyn FnMut() -> bool) -> Result<bool, String> {
    let slot = held.parent().map(Path::to_path_buf).unwrap_or_default();
    let done = slot.join(format!("d.{id}"));
    journal_move(held, &done)
        .map_err(|e| format!("journal {} 을 정리 표지 d.{id} 로 바꾸지 못했습니다({e}) — 그대로 둡니다", held.display()))?;
    if !after_rename() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&done)
        .map_err(|e| format!("정리된 journal {} 삭제 실패({e}) — 다음 기동이 이어서 지웁니다", done.display()))?;
    let _ = std::fs::remove_dir(&slot);
    Ok(true)
}

/// ★HE4 GIVE BACK: `L/active/r.<me>.<id>` → `L/active/u.<id>` (nobody's; the next entry takes it). Returns the line.
fn give_back_journal(held: &Path, id: &str, why: &str) -> String {
    let slot = held.parent().map(Path::to_path_buf).unwrap_or_default();
    match journal_move(held, &slot.join(format!("u.{id}"))) {
        Ok(()) => format!("journal u.{id} 로 돌려놓았습니다(아무것도 지우지 않음 — 다음 기동이 다시 맡습니다): {why}"),
        Err(e) => format!("journal {} 를 돌려놓지 못했습니다({e}) — 이 프로세스가 끝나면 다음 기동이 맡습니다: {why}", held.display()),
    }
}

/// The crash-proof seams of one recovery pass (production: every callback continues). `false` = the process dies there.
struct RecoverSeam<'a> {
    /// after the entry's holder was judged gone (or the entry is `u.`), BEFORE the take
    judged: &'a mut dyn FnMut() -> bool,
    /// after the take succeeded, BEFORE anything of the journal is read
    held: &'a mut dyn FnMut() -> bool,
    /// after each entry restored by the roll-back
    restored: &'a mut dyn FnMut(usize) -> bool,
    /// after SETTLE's rename to `d.<id>`, BEFORE its removal
    settled: &'a mut dyn FnMut() -> bool,
}

const RECOVERY_STOPPED: &str = "[pack-journal] recovery stopped (crash proof seam)";

fn settled_result(r: Result<bool, String>) -> Result<bool, String> {
    match r {
        Ok(true) => Ok(true),
        Ok(false) => Err(RECOVERY_STOPPED.to_string()),
        Err(e) => Err(e),
    }
}

/// crash recovery of THIS lane's journal — called by cysd before its boot install (`src/bin/cysd/main.rs`, separate
/// blocking pool, no lock — unchanged, OQ-H-11), by `cys pack-update` under its apply lock, and as the transaction's ⓪.
/// ★M15-다 / 다-2 (0.15.0 T9 — RULINGS GX1–GX5, HE1–HE6, HG1–HG3): the legacy notice (GX4), then ONE action on the
/// slot's entry (HE4); nothing waits, nothing locks, nothing retries. `Ok(true)` = an entry was settled / removed;
/// `Ok(false)` = nothing to do (or the entry was no longer there); `Err(<one line>)` = kept (cysd prints it and starts; a
/// pack update refuses). The T2g start-path readers are kept: every content read is bounded and never blocks.
pub fn recover_pack_journal() -> Result<bool, String> {
    recover_pack_journal_with(&mut || true)
}

/// ★T9 round 2 (RULINGS HB3 — test seam only): [`recover_pack_journal`] with `on_judged`, called right after the slot
/// entry's holder was judged gone (or the entry is `u.`) and BEFORE the take; `false` stops there as a dying process
/// would. Production passes `|| true`.
fn recover_pack_journal_with(on_judged: &mut dyn FnMut() -> bool) -> Result<bool, String> {
    recover_lane(&mut RecoverSeam {
        judged: on_judged,
        held: &mut || true,
        restored: &mut |_| true,
        settled: &mut || true,
    })
}

fn recover_lane(seam: &mut RecoverSeam) -> Result<bool, String> {
    let pack = pack_dir();
    if let Some(notice) = legacy_pack_journal_notice(&pack) {
        if !LEGACY_JOURNAL_NAMED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            eprintln!("[pack-journal] ⚠ {notice}");
        }
    }
    let slot = pack_journal_dir_for(&pack).join(JOURNAL_SLOT);
    // HE4: the listing is only a HINT for finding the name; every action below is conditional on that exact name.
    let mut names: Vec<std::ffi::OsString> = Vec::new();
    match std::fs::read_dir(&slot) {
        Ok(rd) => {
            for e in rd {
                match e {
                    Ok(e) => names.push(e.file_name()),
                    Err(e) => return Err(format!("journal 자리 {} 목록 판독 실패: {e}", slot.display())),
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("journal 자리 {} 목록 판독 실패: {e}", slot.display())),
    }
    if names.is_empty() {
        // HE4: nothing found → a non-recursive removal (a no-op unless the slot is really empty; failure ignored).
        let _ = std::fs::remove_dir(&slot);
        return Ok(false);
    }
    let shown = || names.iter().map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ");
    let entry = match (names.len(), names[0].to_str().and_then(parse_slot_entry)) {
        (1, Some(e)) => e,
        _ => {
            return Err(format!(
                "journal 자리 {} 에 알 수 없는 항목: {} — 손대지 않습니다",
                slot.display(),
                shown()
            ))
        }
    };
    let name = names[0].to_string_lossy().into_owned();
    let me = JournalWriter::current();
    let id = match &entry {
        SlotEntry::Done { .. } => {
            // HE4: `d.` → remove that folder by its exact name, then the slot (non-recursively). Repeats harmlessly.
            let done = slot.join(&name);
            std::fs::remove_dir_all(&done)
                .map_err(|e| format!("정리된 journal {} 삭제 실패({e}) — 다음 기동이 이어서 지웁니다", done.display()))?;
            let _ = std::fs::remove_dir(&slot);
            return Ok(true);
        }
        SlotEntry::Writer { pid, birth, id } | SlotEntry::Taker { pid, birth, id } => {
            let what = if matches!(entry, SlotEntry::Writer { .. }) {
                format!("다른 팩 업데이트가 실행 중입니다(pid {pid})")
            } else {
                format!("이 팩의 복구가 진행 중입니다(pid {pid})")
            };
            // HE4: a process never takes from a name carrying its OWN pid and start time (it is running).
            if *pid == me.pid && *birth == me.birth_us {
                return Err(format!("journal {name} 유지: {what}"));
            }
            match observe_writer(&JournalWriter { pid: *pid, birth_us: *birth }) {
                WriterLife::Gone => {}
                WriterLife::Running => return Err(format!("journal {name} 유지: {what}")),
                WriterLife::Unknown(why) => return Err(format!("journal {name} 유지: {why}")),
            }
            id.clone()
        }
        SlotEntry::GivenBack { id } => id.clone(),
    };
    if !(seam.judged)() {
        return Err(RECOVERY_STOPPED.to_string());
    }
    // HE4 TAKE: one name-conditional move from the exact listed name to my own name.
    let held = slot.join(format!("r.{}.{id}", me.tag()));
    match journal_move(&slot.join(&name), &held) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("journal {name} 을 맡지 못했습니다({e}) — 손대지 않습니다")),
    }
    if !(seam.held)() {
        return Err(RECOVERY_STOPPED.to_string());
    }
    // From here on ONLY the held folder is read (P1).
    let index_path = held.join("index.json");
    match std::fs::symlink_metadata(&index_path) {
        // no index → the backups were never completed, no pack file was written → SETTLE.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return settled_result(settle_journal(&held, &id, &mut *seam.settled)),
        Err(e) => return Err(give_back_journal(&held, &id, &format!("journal 인덱스 확인 불가: {e}"))),
        Ok(_) => {}
    }
    let index: JournalIndex = match journal_text(&index_path, "journal 인덱스 읽기 실패")
        .and_then(|s| serde_json::from_str(&s).map_err(|e| format!("journal 인덱스 파싱 실패(손상): {e}")))
    {
        Ok(ix) => ix,
        Err(e) => return Err(give_back_journal(&held, &id, &e)),
    };
    let apply_id = match index.apply.as_ref() {
        Some(a) if a.apply_id.starts_with(&id) => a.apply_id.clone(),
        _ => {
            return Err(give_back_journal(
                &held,
                &id,
                "인덱스의 적용 기록(apply id)이 이름의 id 로 시작하지 않음",
            ))
        }
    };
    let marker = pack.join(PACK_APPLIED_FILE);
    let committed = match read_start_meta(&marker) {
        StartRead::Missing => false,
        StartRead::Bytes(b) => match serde_json::from_slice::<PackApplied>(&b) {
            Ok(m) => m.apply_id == apply_id,
            Err(e) => {
                return Err(give_back_journal(&held, &id, &format!("커밋 표지 {} 파싱 실패: {e}", marker.display())))
            }
        },
        StartRead::Unreadable(e) => {
            return Err(give_back_journal(&held, &id, &format!("커밋 표지 {} 판독 불가: {e}", marker.display())))
        }
    };
    if committed {
        // 커밋 성공(표지 = 이 적용) → SETTLE(롤백 금지).
        return settled_result(settle_journal(&held, &id, &mut *seam.settled));
    }
    // 미커밋 → 맡은 폴더에서 롤백: 끝까지 되면 SETTLE, 도중 실패면 돌려놓는다(백업은 하나도 지우지 않았다).
    match rollback_journal(&held, &mut *seam.restored) {
        Ok(true) => settled_result(settle_journal(&held, &id, &mut *seam.settled)),
        Ok(false) => Err(RECOVERY_STOPPED.to_string()),
        Err(e) => Err(give_back_journal(&held, &id, &e)),
    }
}

/// ★HE5: the writer's own roll-back on a failure before the commit — from its own folder; complete → SETTLE, not → it
/// leaves `j.<me>.<id>` for a taker once this process has exited. Returns the honest outcome for the error line.
fn rollback_and_settle_own(own: &Path) -> Result<(), String> {
    let id = own
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(parse_slot_entry)
        .map(|e| match e {
            SlotEntry::Writer { id, .. } | SlotEntry::Taker { id, .. } | SlotEntry::GivenBack { id } | SlotEntry::Done { id } => id,
        })
        .ok_or_else(|| format!("journal 이름 판독 불가: {}", own.display()))?;
    match rollback_journal(own, &mut |_| true)? {
        true => settle_journal(own, &id, &mut || true).map(|_| ()),
        false => Err("rollback stopped".to_string()),
    }
}

/// The failure path of the writer before its commit (②③④④'): roll back from its own folder (HE5) and say what happened.
fn fail_before_commit(own: &Path, what: &str, e: &str) -> String {
    match rollback_and_settle_own(own) {
        Ok(()) => format!("{what}(rollback 완료): {e}"),
        Err(rb) => format!("{what}(rollback 미완 — 저널 {} 보존, 다음 기동이 이어받음: {rb}): {e}", own.display()),
    }
}

/// The line of a refused admission (HE3), from a hint listing of the slot.
fn admission_refused_line(slot: &Path, e: &std::io::Error) -> String {
    let what = std::fs::read_dir(slot)
        .ok()
        .and_then(|mut rd| rd.next())
        .and_then(|r| r.ok())
        .map(|d| d.file_name().to_string_lossy().into_owned());
    let why = match what.as_deref().and_then(parse_slot_entry) {
        Some(SlotEntry::Writer { pid, .. }) => format!("다른 팩 업데이트가 실행 중입니다(pid {pid})"),
        Some(SlotEntry::Taker { pid, .. }) => format!("이 팩의 복구가 진행 중입니다(pid {pid})"),
        Some(SlotEntry::GivenBack { id }) => format!("정리되지 않은 이전 업데이트 기록 u.{id} 이 있습니다 — 다음 기동이 맡습니다"),
        Some(SlotEntry::Done { id }) => format!("끝난 이전 업데이트 기록 d.{id} 을 지우는 중입니다 — 다시 실행하십시오"),
        None => match what {
            Some(n) => format!("journal 자리에 알 수 없는 항목 {n}"),
            None => "journal 자리가 비워지는 중입니다 — 다시 실행하십시오".to_string(),
        },
    };
    format!("[pack-journal] 팩 업데이트를 시작하지 않습니다({}): {why} ({e})", slot.display())
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
/// ★M15-다 / 다-2 (0.15.0 T9, RULINGS GX2 · GX5): the steps keep their order; ① journals `.pack-version` too and records
/// this apply (`apply_id` + target tuple = `state` + [`pack_items_sha256`] of `items`) and its writer; right after ④ the
/// commit marker [`PACK_APPLIED_FILE`] carrying the apply id is written — the LAST hard commit (④'). A failure up to and
/// including ④' rolls back. The signature is unchanged (the tuple and the digest come from `state` and `items`).
/// ★HE3 / HE5 (T9 round 2): after ⓪ the apply is ADMITTED by one name-conditional rename of its prepared folder
/// `L/t.<me>.<seq>/` (holding its empty `j.<me>.<id>/`) to the slot `L/active` — refused (another journal unsettled) →
/// its prepared folder is removed, one line, no second try, no wait. It then addresses its journal only as
/// `L/active/j.<me>.<id>`, rolls back from it on a failure before the commit, and SETTLES it after ④' and ⑤.
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
    apply_pack_transactional_steps(items, target_version, state, auth, post_commit, &mut |_| true)
}

/// [`apply_pack_transactional`] with a step callback (★GX6): `step(s)` is asked after each boundary; `false` stops the
/// transaction right there with NO rollback — the on-disk state a process dying at that point leaves. Production passes
/// a callback that always continues.
fn apply_pack_transactional_steps<F>(
    items: &[(&str, &str)],
    target_version: &str,
    state: &PackState,
    auth: Option<PackWriteAuth>,
    post_commit: F,
    step: &mut dyn FnMut(TxStep) -> bool,
) -> Result<(usize, usize, bool), String>
where
    F: FnOnce() -> Result<(), String>,
{
    // ⓪ 직전 crash로 남은 orphan 저널 자가치유(새 트랜잭션 전 pre-state 확정).
    // ★GX3: a journal it must keep (writer running / unknown / unreadable) makes this fail — never overwritten below.
    recover_pack_journal()?;
    if !step(TxStep::Recovered) {
        return Err(stopped_at(TxStep::Recovered));
    }
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
    // ★GX2: the version marker joins the backup set — a rollback before the commit marker restores it (a crash after ④
    //   no longer leaves "new label, old files").
    backup_set.insert(PACK_VERSION_FILE.to_string());
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
    // ★GX2: this apply's record.
    let items_digest = pack_items_sha256(items);
    let apply = PackApplied {
        apply_id: new_apply_id(&items_digest, state),
        target: state.clone(),
        items_digest,
    };
    let id = journal_id16(&apply.apply_id);
    // ★HE3 ADMISSION: prepare `L/t.<me>.<seq>/j.<me>.<id>/`, then ONE rename to `L/active` (HG1's no-replace move).
    let me = JournalWriter::current();
    let lane = pack_journal_dir();
    std::fs::create_dir_all(&lane)
        .map_err(|e| format!("[pack-journal] 레인 저널 폴더 {} 생성 실패: {e}", lane.display()))?;
    static PREP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let prep = lane.join(format!(
        "t.{}.{}",
        me.tag(),
        PREP_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let jname = format!("j.{}.{id}", me.tag());
    std::fs::create_dir(&prep)
        .map_err(|e| format!("[pack-journal] 준비 폴더 {} 생성 실패: {e}", prep.display()))?;
    if let Err(e) = std::fs::create_dir(prep.join(&jname)) {
        let _ = std::fs::remove_dir_all(&prep);
        return Err(format!("[pack-journal] 준비 폴더 {} 안의 journal 생성 실패: {e}", prep.display()));
    }
    if !step(TxStep::Prepared) {
        return Err(stopped_at(TxStep::Prepared));
    }
    let slot = lane.join(JOURNAL_SLOT);
    if let Err(e) = journal_move(&prep, &slot) {
        let line = admission_refused_line(&slot, &e);
        let _ = std::fs::remove_dir_all(&prep);
        return Err(line);
    }
    let own = slot.join(&jname);
    if !step(TxStep::Admitted) {
        return Err(stopped_at(TxStep::Admitted));
    }
    match write_journal(&own, target_version, &backup_set, &apply, step) {
        Ok(true) => {}
        Ok(false) => return Err(stopped_at(TxStep::BackedUp)),
        Err(e) => {
            // No index was written (it is the last write) → no pack file was touched: SETTLE (HE4's no-index rule).
            let _ = settle_journal(&own, &id, &mut || true);
            return Err(e);
        }
    }
    if !step(TxStep::Journaled) {
        return Err(stopped_at(TxStep::Journaled));
    }
    // ② 파일 반영(transactional=true) — .pack-version은 여기서 쓰지 않고(④에서 commit marker로),
    //    .install-manifest.json write 실패는 fail-closed로 Err가 되어 아래 rollback을 탄다.
    let (written, kept) =
        match install_from_iter(items.iter().copied(), false, target_version, true, auth) {
            Ok(v) => v,
            Err(e) => return Err(fail_before_commit(&own, "파일 반영 실패", &e)),
        };
    if !step(TxStep::Installed) {
        return Err(stopped_at(TxStep::Installed));
    }
    // ③ `.pack-state.json` 기록 — journal 백업 대상이므로 실패 시 rollback으로 전체 복원.
    if let Err(e) = write_pack_state(&dir, state) {
        return Err(fail_before_commit(&own, "state 기록 실패", &e));
    }
    if !step(TxStep::StateWritten) {
        return Err(stopped_at(TxStep::StateWritten));
    }
    // ④ .pack-version(결과 검사 — best-effort 금지). ★GX2: no longer the commit marker by itself.
    if let Err(e) = write_atomic(&dir.join(PACK_VERSION_FILE), target_version.as_bytes()) {
        return Err(fail_before_commit(&own, ".pack-version 커밋 실패", &e.to_string()));
    }
    if !step(TxStep::VersionWritten) {
        return Err(stopped_at(TxStep::VersionWritten));
    }
    // ④' ★GX2: the commit marker carrying THIS apply's id = the LAST hard commit (atomic). Failure → rollback.
    let marker = match serde_json::to_vec_pretty(&apply) {
        Ok(v) => v,
        Err(e) => return Err(fail_before_commit(&own, "커밋 표지 직렬화 실패", &e.to_string())),
    };
    if let Err(e) = write_atomic(&dir.join(PACK_APPLIED_FILE), &marker) {
        return Err(fail_before_commit(&own, "커밋 표지 기록 실패", &e.to_string()));
    }
    if !step(TxStep::Committed) {
        return Err(stopped_at(TxStep::Committed));
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
    if !step(TxStep::PostCommitted) {
        return Err(stopped_at(TxStep::PostCommitted));
    }
    // ⑥ ★HE5: SETTLE its own journal (rename to `d.<id>` first, then the removal). A failing rename leaves the folder as
    //   it is (the next recovery finds a gone writer and a marker carrying the id → settles).
    match settle_journal(&own, &id, &mut || step(TxStep::Settled)) {
        Ok(false) => return Err(stopped_at(TxStep::Settled)),
        Ok(true) => {}
        Err(e) => eprintln!("[pack-update] ⚠ {e}"),
    }
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

    // ───────────────────────── C-07 (0.14.55 · P3) ─────────────────────────
    /// C-07 낡은 항목 판정의 **여섯 분기 전부**. 표를 주입하므로 `Repaired` 도 밟는다.
    #[test]
    fn c07_classify_stale_entry_six_branches() {
        // system 등급 rel 하나를 임베드에서 고른다(등급 술어를 검체가 다시 쓰지 않는다).
        let sysrel = PACK_ALL
            .iter()
            .map(|(r, _)| *r)
            .find(|r| ownership_scoped(r, PackScope::Base) == Ownership::System)
            .expect("system rel");
        let userrel = PACK_ALL
            .iter()
            .map(|(r, _)| *r)
            .find(|r| ownership_scoped(r, PackScope::Base) == Ownership::User)
            .expect("user rel");

        let prev_vendor = "vendor bytes of a previous release\n";
        let hp = content_hash(prev_vendor);
        let stale = content_hash("a hash older than both\n");
        let table: Vec<&str> = vec![hp.as_str()];

        // 1) system 아님 → Normal (user 파일은 이 판정의 대상이 아니다)
        assert_eq!(
            classify_stale_entry_with(
                userrel, PackScope::Base, Some("x"), Some(&stale), None, None, &table, false
            ),
            StaleEntry::Normal
        );
        // 2) 매니페스트 항목 부재 → Normal (레거시 경로 거동 불변)
        assert_eq!(
            classify_stale_entry_with(sysrel, PackScope::Base, Some("x"), None, None, None, &table, false),
            StaleEntry::Normal
        );
        // 3) 항목이 디스크와 일치 → Normal (건전)
        let dh = content_hash("x");
        assert_eq!(
            classify_stale_entry_with(sysrel, PackScope::Base, Some("x"), Some(&dh), None, None, &table, false),
            StaleEntry::Normal
        );
        // 4) 표에 행이 없음 → KeepInPlace("no-row")
        assert_eq!(
            classify_stale_entry_with(sysrel, PackScope::Base, Some("x"), Some(&stale), None, None, &[], false),
            StaleEntry::KeepInPlace("no-row")
        );
        // 5) ★디스크가 표에 있는 과거 릴리스 본 → Repaired(디스크 해시)
        assert_eq!(
            classify_stale_entry_with(
                sysrel,
                PackScope::Base,
                Some(prev_vendor),
                Some(&stale),
                None,
                None,
                &table,
                false
            ),
            StaleEntry::Repaired(hp.clone())
        );
        // 6) 디스크가 어느 행과도 불일치 → KeepInPlace("unknown-disk") ⛔강제 치유 아님
        assert_eq!(
            classify_stale_entry_with(
                sysrel,
                PackScope::Base,
                Some("the operator wrote this\n"),
                Some(&stale),
                None,
                None,
                &table,
                false
            ),
            StaleEntry::KeepInPlace("unknown-disk")
        );
        // 7) ★판정 승인(C07-NARROW 항목 1-2): 같은 '출처 불명' 상태에 **병합 대기 원장 항목이 있으면**
        //    디스크 출처는 그 병합이다 → Normal → pinned 크래시 창 수렴이 그대로 돈다.
        assert_eq!(
            classify_stale_entry_with(
                sysrel,
                PackScope::Base,
                Some("the operator wrote this\n"),
                Some(&stale),
                None,
                None,
                &table,
                true
            ),
            StaleEntry::Normal,
            "병합 대기 항목이 있으면 제자리 보존이 아니다(크래시 창 수렴 보존)"
        );
        // 7b) ★항목이 임베드와 같으면 낡지 않았다(L2 kept-drift 경로 보존).
        let he = content_hash("embed-now\n");
        assert_eq!(
            classify_stale_entry_with(
                sysrel, PackScope::Base, Some("user edit\n"), Some(&he), None, Some(&he), &table, false
            ),
            StaleEntry::Normal,
            "항목==hash(embed) 는 가능한 가장 새로운 값 = 낡지 않았다(L2 pinned 거동 보존)"
        );
        // 8) 행이 없는 경로도 같다 — 항목이 있으면 Normal.
        assert_eq!(
            classify_stale_entry_with(sysrel, PackScope::Base, Some("x"), Some(&stale), None, None, &[], true),
            StaleEntry::Normal,
            "no-row + 병합 대기 항목 → Normal"
        );
    }

    /// ★T12 (gemini-3 r1 major 1 · codex r1 m2) — **디스크 == 현재 임베드**인데 매니페스트 항목이
    /// 낡은 상태는 '출처 불명'이 아니다. 표는 현재 릴리스 blob 을 구조적으로 제외하므로
    /// (`gen_prev_vendor_hashes.py` 의 `if o != oid_now:`) 이 상태는 `no-row` 로 떨어져 제자리
    /// 보존이 되고, 그러면 Arm 1(`Keep{adopt_hash:true}`) 과 그 뒤의 `manifest.insert` 가 영구히
    /// 도달 불가가 된다 — 낡은 항목이 **수리되지 않는다**. 분류기와 rig 양쪽에서 잰다.
    #[test]
    fn p3r1_m2_disk_equal_to_current_embed_reaches_adoption() {
        let sysrel = PACK_ALL
            .iter()
            .map(|(r, _)| *r)
            .find(|r| ownership_scoped(r, PackScope::Base) == Ownership::System)
            .expect("system rel");
        let embed = "current vendor bytes\n";
        let he = content_hash(embed);
        let stale = content_hash("an entry older than both\n");
        let prev_row = content_hash("some previous release\n");
        let table: Vec<&str> = vec![prev_row.as_str()];

        // ⓐ 조상 없음(pristine absent) + 낡은 항목 + 디스크==임베드 → Normal(=Arm 1 로 간다)
        assert_eq!(
            classify_stale_entry_with(
                sysrel, PackScope::Base, Some(embed), Some(&stale), None, Some(&he), &table, false
            ),
            StaleEntry::Normal,
            "디스크가 현재 임베드면 '불명' 이 아니다 — 채택 경로로 가야 한다"
        );
        // ⓑ 조상 == 현재 임베드 + 낡은 항목 + 디스크==임베드 → Normal
        assert_eq!(
            classify_stale_entry_with(
                sysrel, PackScope::Base, Some(embed), Some(&stale), Some(&he), Some(&he), &table, false
            ),
            StaleEntry::Normal
        );
        // ⓒ 대조군(유지돼야 하는 것 1) — 운영자 수정본은 여전히 제자리 보존이다.
        assert_eq!(
            classify_stale_entry_with(
                sysrel,
                PackScope::Base,
                Some("the operator wrote this\n"),
                Some(&stale),
                None,
                Some(&he),
                &table,
                false
            ),
            StaleEntry::KeepInPlace("unknown-disk"),
            "출처 불명 수정본의 제자리 보존(C-07 좁힘)은 그대로여야 한다"
        );
        // ⓓ 대조군(유지돼야 하는 것 2) — 병합 대기 항목이 있으면 Normal(크래시 창 수렴 보존).
        assert_eq!(
            classify_stale_entry_with(
                sysrel,
                PackScope::Base,
                Some("the operator wrote this\n"),
                Some(&stale),
                None,
                Some(&he),
                &table,
                true
            ),
            StaleEntry::Normal
        );

        // ── rig: 낡은 항목이 실제로 수리되는가(plan 과 install 양쪽) ──
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!(
            "cys-p3r1-m2-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&td);
        let pd = td.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        let _env = set_pack_env(&pd, td.join("cfg"));
        let items = [(sysrel, embed)];
        install_into(
            pd.clone(), items.iter().copied(), false, "1.0.0", false, false,
            pack_scope_of(&pd), None, None,
        )
        .unwrap();
        // 출하된 결함 상태를 만든다: 디스크·조상은 그대로 두고 **매니페스트 항목만** 낡게 한다.
        let mpath = pd.join(INSTALL_MANIFEST);
        let mut manifest: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(&mpath).unwrap()).unwrap();
        manifest.insert(sysrel.to_string(), stale.clone());
        std::fs::write(&mpath, serde_json::to_string(&manifest).unwrap()).unwrap();
        assert_eq!(
            std::fs::read_to_string(pd.join(sysrel)).unwrap(),
            embed,
            "전제: 디스크는 현재 임베드다"
        );

        let plan = plan_install(&pd, &items, false, "1.0.1");
        assert!(
            plan.kept_drift.is_empty(),
            "제자리 보존으로 떨어지면 안 된다(그러면 항목이 영구히 낡는다): {:?}",
            plan.kept_drift
        );
        assert_eq!(plan.unchanged, 1, "Arm 1 = 최신(변화 없음) 계상이어야 한다");

        install_into(
            pd.clone(), items.iter().copied(), false, "1.0.1", false, false,
            pack_scope_of(&pd), None, None,
        )
        .unwrap();
        let manifest2: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(&mpath).unwrap()).unwrap();
        assert_eq!(
            manifest2.get(sysrel),
            Some(&he),
            "낡은 항목이 hash(embed) 로 수리돼야 한다(manifest.insert 도달)"
        );
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★`!force` 게이트(gemini-3 r1 major 1 후반) — 제자리 보존 단축은 **보수 기본 경로**의 것이다.
    /// `--force` 는 사용자가 vendor 본을 명시적으로 강제하는 동사이므로 그 자리에서 분류를 건너뛰면
    /// 강제가 조용히 무시된다. plan 쪽에서 잰다(install 쪽은 같은 게이트의 짝).
    #[test]
    fn p3r1_major1_keep_in_place_short_circuit_is_gated_on_force() {
        let sysrel = PACK_ALL
            .iter()
            .map(|(r, _)| *r)
            .find(|r| ownership_scoped(r, PackScope::Base) == Ownership::System)
            .expect("system rel");
        let embed = "vendor v2\n";
        let _lock = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!(
            "cys-p3r1-force-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&td);
        let pd = td.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        let _env = set_pack_env(&pd, td.join("cfg"));
        install_into(
            pd.clone(), [(sysrel, "vendor v1\n")].iter().copied(), false, "1.0.0", false, false,
            pack_scope_of(&pd), None, None,
        )
        .unwrap();
        // 출처 불명 상태: 디스크는 운영자 바이트, 항목은 낡음, 병합 대기 없음.
        std::fs::write(pd.join(sysrel), "the operator wrote this\n").unwrap();
        let mpath = pd.join(INSTALL_MANIFEST);
        let mut manifest: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(&mpath).unwrap()).unwrap();
        manifest.insert(sysrel.to_string(), content_hash("an entry older than both\n"));
        std::fs::write(&mpath, serde_json::to_string(&manifest).unwrap()).unwrap();
        let items = [(sysrel, embed)];

        let plan_default = plan_install(&pd, &items, false, "1.0.1");
        assert!(
            plan_default.kept_drift.iter().any(|r| r == sysrel),
            "기본 경로(!force)는 제자리 보존이다: {:?}",
            plan_default.kept_drift
        );
        let plan_forced = plan_install(&pd, &items, true, "1.0.1");
        assert!(
            !plan_forced.kept_drift.iter().any(|r| r == sysrel),
            "--force 에서는 단축이 걸리지 않아야 한다(강제의 침묵 무시 금지): {:?}",
            plan_forced.kept_drift
        );
        // ★0.15.0 M11 자(H-RW 재작성 — 종전: "--force 는 분류로 내려가 치유/갱신으로 잡혀야 한다"): 강제는 여전히
        //   단축을 건너뛰어 분류로 내려가지만(침묵 무시 금지), 분류는 사용자 수정을 버리지 않는다 — 출처 불명의
        //   운영자 바이트는 사용자 우선(user-kept)이고 치유로 잡히지 않는다.
        assert!(
            plan_forced.user_kept.iter().any(|r| r == sysrel),
            "--force 는 분류로 내려가 사용자 우선으로 잡혀야 한다: user_kept={:?} heal={:?}",
            plan_forced.user_kept,
            plan_forced.heal
        );
        assert!(!plan_forced.heal.iter().any(|r| r == sysrel), "--force 가 사용자 수정을 치유로 덮지 않는다(자)");
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★T23 (codex r1 m1 · gemini-3 r1 major 2-a) — `apply_mode_exact` 의 **불일치 거부**가
    /// 실제로 부하를 진다: mode 적용이 조용히 무시되는 고장을 **주입**해 ①`Err` ②rename 전 중단
    /// (대상 바이트·mode 무변경) ③tmp 정리를 모두 잰다. 이 검체가 worker-15 가 자진 신고한
    /// 살아남은 변이(M-b = 검증 비교문 제거)를 죽인다.
    #[cfg(unix)]
    #[test]
    fn p3r1_m1_mode_verify_failure_refuses_before_rename() {
        use std::os::unix::fs::PermissionsExt;
        let td = std::env::temp_dir().join(format!(
            "cys-p3r1-t23-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let target = td.join("script.sh");
        std::fs::write(&target, "#!/bin/sh\nold\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();

        // 주입 고장: **mode 적용이 요청과 다른 결과를 낸다**(관측은 진짜 `fstat`). 되읽기 검증만이
        //   이것을 잡는다 — 이것이 "mode 를 조용히 무시하는 파일시스템"의 결정론 모형이다.
        // ⛔문자 그대로의 no-op(`|_f,_m| Ok(())`)은 **umask 에 의존해 거짓 녹색**이 된다: 실측 결과
        //   umask 022 아래에서는 `create_tmp_with_mode` 의 `open(2)` 가 이미 0755 를 만들어 두어
        //   검증이 통과해 버렸다(이 검체의 첫 판이 그렇게 녹색이었다). 그래서 적용 결과를 요청과
        //   **다르게** 만드는 쪽으로 고장을 모형화한다 — 같은 분기를 밟으면서 umask 와 무관하다.
        fn chmod_wrong_then_verify(f: &std::fs::File, m: u32) -> std::io::Result<()> {
            apply_mode_exact_with(f, m, |f, _m| set_mode_fchmod(f, 0o600), observe_mode_fstat)
        }
        let err = write_atomic_mode_inner(
            &target,
            b"#!/bin/sh\nnew\n",
            Some(0o755),
            true,
            chmod_wrong_then_verify,
            false,
        )
        .expect_err("mode 적용이 무시됐는데 성공을 돌려주면 조용한 실패다");
        assert!(
            err.to_string().contains("mode 적용 검증 실패"),
            "거부 사유가 mode 검증이어야 한다: {err}"
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "#!/bin/sh\nold\n",
            "rename 전에 멈췄으므로 대상 바이트는 그대로다"
        );
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
            0o644,
            "대상 mode 도 그대로다"
        );
        let leftovers: Vec<String> = std::fs::read_dir(&td)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "script.sh")
            .collect();
        assert!(leftovers.is_empty(), "실패 경로가 tmp 를 남겼다: {leftovers:?}");

        // 계측 타당성(양성 대조) — 진짜 수단으로는 같은 호출이 성공하고 요청 mode 가 정확히 선다.
        write_atomic_mode_inner(
            &target,
            b"#!/bin/sh\nnew\n",
            Some(0o755),
            true,
            apply_mode_exact,
            false,
        )
        .expect("진짜 fchmod 경로는 성공해야 한다");
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
            0o755
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "#!/bin/sh\nnew\n");
        let _ = std::fs::remove_dir_all(&td);
    }

    /// 생성된 표가 **실제로** 실려 있고 조회가 돈다(빈 표로 컴파일되면 모든 경로가 no-row 가 되어
    /// 조용히 "아무것도 안 함"이 되므로, 표의 실재 자체가 검체 대상이다).
    #[test]
    fn c07_generated_table_is_present_and_queryable() {
        let n = crate::pack_prev_vendor::PREV_VENDOR_HASHES.len();
        assert!(n >= 100, "표 행 수가 비정상적으로 적다: {n}");
        // 정렬 전제(이진 탐색) — 깨지면 조회가 조용히 빈 슬라이스를 돌려준다.
        let mut sorted = true;
        for w in crate::pack_prev_vendor::PREV_VENDOR_HASHES.windows(2) {
            if w[0].0 >= w[1].0 {
                sorted = false;
                break;
            }
        }
        assert!(sorted, "PREV_VENDOR_HASHES 는 rel 로 정렬돼 있어야 한다(이진 탐색 전제)");
        // 모든 행의 해시는 64자 소문자 hex 여야 한다.
        for (rel, hs) in crate::pack_prev_vendor::PREV_VENDOR_HASHES {
            assert!(!hs.is_empty(), "빈 행은 no-row 와 구별되지 않는다: {rel}");
            for h in *hs {
                assert!(
                    h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                    "해시 형식 위반 {rel}: {h}"
                );
            }
        }
        // 표에 있는 첫 행은 조회로 찾을 수 있고, 없는 경로는 빈 슬라이스다.
        let first = crate::pack_prev_vendor::PREV_VENDOR_HASHES[0].0;
        assert!(!crate::pack_prev_vendor::prev_vendor_hashes(first).is_empty());
        assert!(crate::pack_prev_vendor::prev_vendor_hashes("no/such/path.xyz").is_empty());
    }

    // ───────────────────────── H-08 (0.14.55 · P3) ─────────────────────────
    /// H-08 의 mode 규칙표(순수 함수 · umask 무관). 설계 R2 section 1-3 의 다섯 행 전부.
    #[test]
    fn h08_pack_target_mode_rule_table() {
        let sheb = b"#!/usr/bin/env python3\n";
        let plain = b"{\"k\": 1}\n";
        // A) 실행비트가 있으면 그대로 — 0755 는 0755
        assert_eq!(pack_target_mode(Some(0o755), sheb, true), Some(0o755));
        // B) 의도된 0700 을 0755 로 **넓히지 않는다**
        assert_eq!(pack_target_mode(Some(0o700), sheb, true), Some(0o700));
        // C) system 매니페스트 항목 + shebang + 실행비트 소실 → 0755 복구
        assert_eq!(pack_target_mode(Some(0o644), sheb, true), Some(0o755));
        // C') 같은 0644 shebang 이지만 system 매니페스트 항목이 **아니면** 손대지 않는다
        assert_eq!(pack_target_mode(Some(0o644), sheb, false), Some(0o644));
        // D) 선행 파일 없음 + shebang → 0755
        assert_eq!(pack_target_mode(None, sheb, true), Some(0o755));
        // E) 선행 파일 없음 + 비-shebang → 파생값 없음(종전 기본 퍼미션)
        assert_eq!(pack_target_mode(None, plain, true), None);
        // F) 선행 0644 비-shebang → 그대로
        assert_eq!(pack_target_mode(Some(0o644), plain, true), Some(0o644));
        // exec_mode_for 자체
        assert_eq!(exec_mode_for(sheb), Some(0o755));
        assert_eq!(exec_mode_for(plain), None);
    }

    /// ★H-08 의 기제: `write_atomic_mode` 는 mode 를 `open(2)` 인자로 넘겨 **umask 로 걸러지고**,
    /// `write_atomic_mode_exact` 는 tmp 디스크립터에 `fchmod` 로 적용해 **umask 와 무관**하다.
    ///
    /// ★격리(판정 C07-NARROW 항목 4-a): umask 는 **프로세스 전역**이고 cargo 는 검체를 같은 프로세스의
    /// 병렬 스레드로 돌린다 — 락으로는 그 락을 잡지 않는 검체를 막을 수 없다. 그래서 umask 창은
    /// **자식 프로세스**에서만 열린다: 부모가 자기 테스트 바이너리를 `--exact` 로 재실행하고
    /// (환경변수가 자식 분기를 고른다) 자식이 파일을 만든 뒤 종료하면, 부모가 그 파일의 mode 를
    /// 단언한다. 단언은 약화되지 않았다 — 두 umask 에서 정확히 같은 것을 본다.
    #[cfg(unix)]
    #[test]
    fn h08_exact_writer_is_umask_independent_and_plain_writer_is_not() {
        use std::os::unix::fs::PermissionsExt;
        const MARKER: &str = "CYS_H08_UMASK_CHILD";
        const SHEB: &[u8] = b"#!/usr/bin/env python3\n";
        const UMASKS: [u32; 2] = [0o111, 0o077];

        if let Ok(base) = std::env::var(MARKER) {
            // ── 자식: umask 창은 이 프로세스에만 존재한다 ──
            let base = std::path::PathBuf::from(base);
            for um in UMASKS {
                let prev = unsafe { libc::umask(um as libc::mode_t) };
                let exact = base.join(format!("exact-{um:04o}"));
                let plain = base.join(format!("plain-{um:04o}"));
                let a = write_atomic_mode_exact(&exact, SHEB, Some(0o755));
                let b = write_atomic_mode(&plain, SHEB, Some(0o755));
                unsafe { libc::umask(prev) };
                a.expect("exact 쓰기");
                b.expect("plain 쓰기");
            }
            // mode=None 회귀 대조(exact 가 기본 거동을 바꾸지 않는다).
            write_atomic_mode_exact(&base.join("none-exact"), b"x\n", None).expect("none exact");
            write_atomic_mode(&base.join("none-plain"), b"x\n", None).expect("none plain");
            return;
        }

        // ── 부모: 자식을 띄우고 자식이 만든 파일의 mode 를 단언한다 ──
        let base = std::env::temp_dir().join(format!("cys-h08-umask-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        // 디렉터리 실행비트는 자식의 umask 와 무관하게 세워 둔다(없으면 자식이 자기를 막는다).
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)).unwrap();
        let exe = std::env::current_exe().expect("test 바이너리 경로");
        let st = std::process::Command::new(exe)
            .args([
                "--exact",
                "pack::tests::h08_exact_writer_is_umask_independent_and_plain_writer_is_not",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(MARKER, &base)
            .status()
            .expect("자식 테스트 프로세스 기동");
        assert!(st.success(), "자식 프로세스가 실패했다: {st:?}");

        let mode_of = |p: &std::path::Path| {
            std::fs::metadata(p).unwrap().permissions().mode() & 0o7777
        };
        for um in UMASKS {
            let m_exact = mode_of(&base.join(format!("exact-{um:04o}")));
            let m_plain = mode_of(&base.join(format!("plain-{um:04o}")));
            assert_eq!(
                m_exact, 0o755,
                "umask {um:04o}: exact 판은 요청 mode 를 그대로 세워야 한다 — 실측 {m_exact:04o}"
            );
            assert_eq!(
                m_plain,
                0o755 & !um,
                "umask {um:04o}: 평문 판의 결과는 정확히 m & !umask 다(기제 대조) — 실측 {m_plain:04o}"
            );
            assert_ne!(m_plain, 0o755, "umask {um:04o}: 평문 판이 걸러지지 않았다면 대조가 성립하지 않는다");
        }
        assert_eq!(
            mode_of(&base.join("none-exact")),
            mode_of(&base.join("none-plain")),
            "mode=None 에서는 두 경로가 동일해야 한다(회귀 방지)"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★H-08 호출부 census(T9): 설계가 열거한 **8개 대상 쓰기**가 전부 새 헬퍼를 타고, 사이드카
    /// (`<rel>.user`·`.vendor-base`)와 `.pristine` 은 종전 writer 를 그대로 쓴다. 새 대상 쓰기를
    /// `write_atomic` 으로 추가하면 이 검체가 적색이 된다.
    /// ★T2i(RULINGS EC4 의 결과): 팩 동사의 대상 쓰기 8곳은 경로로 여는 [`write_pack_file`] 대신 닻 내린
    /// [`write_pack_file_beneath`] 를 탄다 — 권한 규칙은 같다(H-08 `pack_target_mode` · 정확 적용). `<rel>.user` 는
    /// (RULINGS EC2) [`park_copy`] 의 덮지 않는 놓기로만 생기고, 그 쓰기도 실행비트를 주지 않는다(권한 없음 = 데이터).
    /// 그래서 셈은 두 헬퍼의 합이 8 이고, 사이드카 검사는 지금의 writer 이름을 본다.
    #[test]
    fn h08_target_write_site_census() {
        let src = include_str!("bin/cys.rs");
        // 대상 쓰기 = H-08 헬퍼 호출(경로형 + 닻 내린 형). 설계의 8개 사이트와 수가 일치해야 한다.
        // 호출만 센다 — 주석 줄과 문자열 안의 언급(EC4 쓰기 자리 조사가 금지 목록으로 이 이름을 적는다)은 호출이 아니다.
        let calls = |pat: &str| -> usize {
            src.lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .map(|l| l.match_indices(pat).filter(|(i, _)| !l[..*i].ends_with('"')).count())
                .sum()
        };
        let path_writes = calls("cys::pack::write_pack_file(");
        let anchored_writes = calls("cys::pack::write_pack_file_beneath(");
        assert_eq!(
            path_writes + anchored_writes,
            8,
            "설계가 열거한 대상 쓰기 8곳이 전부 H-08 헬퍼를 타야 한다 — 실측 경로형 {path_writes} + 닻 내린 형 {anchored_writes}"
        );
        // 닻 내린 헬퍼도 같은 권한 규칙(pack_target_mode · 정확 적용)을 쓴다.
        let pack_src = include_str!("pack.rs");
        let body_at = pack_src.find("pub fn write_pack_file_beneath(").expect("닻 내린 대상 쓰기 헬퍼");
        let body = &pack_src[body_at..body_at + pack_src[body_at..].find("\n}\n").expect("헬퍼 끝")];
        assert!(
            body.contains("let mode = pack_target_mode(prior, bytes, system_manifest_entry);")
                && body.contains("beneath_write(root, rel_path, bytes, mode, true, place, false)"),
            "닻 내린 대상 쓰기가 H-08 권한 규칙을 정확 적용하지 않는다"
        );
        // 사이드카·메타데이터 쓰기는 실행비트를 주지 않는 writer(`<rel>.user` = park_copy · `.vendor-base` = write_atomic).
        for needle in ["cys::pack::park_copy(", ".vendor-base"] {
            assert!(
                src.contains(needle),
                "사이드카 경로가 사라졌다(설계는 무변경을 요구한다): {needle}"
            );
        }
        assert!(
            pack_src.contains("beneath_write(dir, &user_rel, bytes, None, false, Place::NoReplace, durable)"),
            "`<rel>.user` 의 놓기가 권한 없는(실행비트 없는) 쓰기가 아니다"
        );
    }

    /// ★(0.14.42 · perf R3-6) 임베드 팩 해시 **메모 핀**. PACK_ALL 은 컴파일 타임 상수라 값이 프로세스 수명
    /// 동안 바뀌지 않는다.
    /// ① 값: 종전 계산(여기 오라클)과 같다.
    /// ② 결정론: 두 번 받은 참조가 **같은 버퍼**다(`ptr::eq`). 호출마다 다시 계산하면 새 `String` 이라
    ///   주소가 갈린다. 부하·SHA 가속 유무와 무관하게 판별된다.
    /// ③ 보조(수정 전 적색 증거용): 공개 함수 5회 중 최소 소요가 오라클 계산 소요의 1/10 미만이다.
    ///   절대 임계(ms)를 두지 않는다 — SHA 가속이 켜지거나 팩이 작아져도 비율은 판별력을 잃지 않는다.
    /// 운영 구간에 `#[cfg(test)]` 계수기를 두지 않는 이유: 소스 핀들이 `find("#[cfg(test)]")` 로
    /// 운영 구간을 자른다.
    #[test]
    fn embedded_pack_hash_is_memoized_and_unchanged() {
        use sha2::{Digest, Sha256};
        let t = std::time::Instant::now();
        let mut h = Sha256::new();
        let mut bytes = 0usize;
        for (rel, content) in PACK_ALL.iter() {
            h.update(rel.as_bytes());
            h.update(b"\0");
            h.update(content.as_bytes());
            h.update(b"\0");
            bytes += rel.len() + content.len() + 2;
        }
        let oracle = format!("{:x}", h.finalize());
        let oracle_cost = t.elapsed();
        assert!(bytes > 0, "PACK_ALL 이 비었다 — 비교가 공허하다");
        assert_eq!(embedded_pack_hash(), oracle, "임베드 팩 해시 값이 종전 계산과 갈렸다");
        let a = embedded_pack_hash_ref();
        let b = embedded_pack_hash_ref();
        assert_eq!(a, oracle.as_str());
        assert!(std::ptr::eq(a, b), "embedded_pack_hash_ref 가 호출마다 새 값을 만든다(메모 아님)");
        let mut best = std::time::Duration::MAX;
        for _ in 0..5 {
            let t = std::time::Instant::now();
            let v = embedded_pack_hash();
            best = best.min(t.elapsed());
            assert_eq!(v, oracle);
        }
        assert!(
            best * 10 < oracle_cost,
            "embedded_pack_hash 가 호출마다 재계산된다: 5회 중 최소 {best:?} · 오라클 1회 {oracle_cost:?} (해시 입력 {bytes} B)"
        );
    }

    /// ★(0.14.42 통합 검증) 임베드 팩 **사본 수** 핀. `PACK_ALL` 은 build.rs 가 만든 `const` 라 그것을 참조하는
    /// 코드 생성 단위(CGU)마다 팩 전체(≈10MB 문자열)가 따로 실릴 수 있다. 메모 해시 루프를 `OnceLock::get_or_init`
    /// 의 제네릭 클로저 안에 두면 그 단형화가 다른 CGU 로 가서 release 바이너리에 팩 사본이 하나 더 생겼다
    /// (실측 2026-09-26 · a02aa79c 전후 release `cys` 30,183,808 → 41,148,624 B · 팩 문장 출현 2 → 3회 · cysd 도 +11MB).
    /// 그래서 `PACK_ALL` 참조는 비제네릭 메모 본체에서 한 번 뜨고, 클로저에는 슬라이스만 넘긴다 — 그 **모양**을 핀한다.
    /// (사본 수 자체는 release 산출물 측정으로만 보인다 — 이 핀은 그 회귀를 되부르는 소스 모양을 막는다.)
    #[test]
    fn embedded_pack_hash_memo_closure_does_not_touch_pack_all() {
        let src = include_str!("pack.rs");
        let prod = &src[..src.find("#[cfg(test)]\nmod tests").expect("테스트 모듈 경계")];
        let i = prod.find("fn embedded_pack_hash_ref()").expect("메모 본체 소실");
        let body = &prod[i..i + prod[i..].find("\n}\n").expect("메모 본체 끝")];
        let c = body.find(".get_or_init(").expect("OnceLock 메모 소실");
        assert!(
            !body[c..].contains("PACK_ALL"),
            "메모 클로저가 PACK_ALL 을 직접 참조한다 — release 바이너리에 임베드 팩 사본이 하나 더 실린다(+≈11MB/바이너리)"
        );
        assert!(body[..c].contains("PACK_ALL"), "메모 본체가 PACK_ALL 을 비제네릭 위치에서 참조하지 않는다");
    }

    // ── ★U10(0.14.41) 각성 훅 경고 — 레인 팩 단독 기대값 · 표기 정규화(관측 전용) ─────────────
    fn u10_settings(cmds: &[(&str, &str)]) -> serde_json::Value {
        let mut hooks = serde_json::Map::new();
        for (event, cmd) in cmds {
            let arr = hooks
                .entry(event.to_string())
                .or_insert_with(|| serde_json::json!([]));
            arr.as_array_mut()
                .unwrap()
                .push(serde_json::json!({"hooks": [{"type": "command", "command": cmd}]}));
        }
        serde_json::json!({ "hooks": hooks })
    }

    /// 부서 계정 settings 에 부서 팩 훅이 정상 등록 → 레인(부서) 팩 기대값으로 누락 0 (U10 RC2 오탐 제거).
    #[test]
    fn u10_awakening_missing_dept_lane_registered_is_clean() {
        let dept = Path::new("/h/.cys/pack-dept-a");
        let s = u10_settings(&[
            ("SessionStart", &hook_command_for(dept, "session-start.sh")),
            ("UserPromptSubmit", &hook_command_for(dept, "role-bootstrap.sh")),
        ]);
        assert!(awakening_hooks_missing_in(&s, dept, cfg!(windows)).is_empty());
    }

    /// 합집합 금지 — 본부 팩 훅만 교차 등록된 부서 settings 는 부서 레인 기대값에서 **둘 다 누락**이다.
    #[test]
    fn u10_awakening_missing_has_no_union_with_hq_pack() {
        let hq = Path::new("/h/.cys/pack");
        let dept = Path::new("/h/.cys/pack-dept-a");
        let s = u10_settings(&[
            ("SessionStart", &hook_command_for(hq, "session-start.sh")),
            ("UserPromptSubmit", &hook_command_for(hq, "role-bootstrap.sh")),
        ]);
        assert_eq!(
            awakening_hooks_missing_in(&s, dept, cfg!(windows)),
            vec!["session-start.sh", "role-bootstrap.sh"],
            "본부 팩 훅이 부서 레인에서 '등록됨' 으로 통과했다(합집합 금지 위반)"
        );
        // 대칭: 본부 레인 기대값에서는 등록됨.
        assert!(awakening_hooks_missing_in(&s, hq, cfg!(windows)).is_empty());
    }

    /// 윈도우 규칙: 드라이브 대소문자·역슬래시·MSYS(`/c/`)·`\\?\` 접두·따옴표 차이를 같은 경로로 본다.
    #[test]
    fn u10_hook_compare_windows_forms_are_equal() {
        // ★경로의 사용자명은 secret-scan 더미(`user`)만 쓴다(개인경로 오탐 차단 — scripts/secret-scan.sh dummy_names).
        let want = r#"bash "C:/Users/user/.cys/pack-dept-a/hooks/session-start.sh""#;
        for got in [
            r#"bash "c:\Users\user\.cys\pack-dept-a\hooks\session-start.sh""#,
            r#"bash "/c/Users/user/.cys/pack-dept-a/hooks/session-start.sh""#,
            r#"bash "\\?\C:\Users\user\.cys\pack-dept-a\hooks\session-start.sh""#,
            r#"bash C:/Users/user/.cys/pack-dept-a/hooks/session-start.sh"#,
            r#"BASH  "c:/USERS/user/.CYS/pack-dept-a/HOOKS/session-start.sh""#,
        ] {
            assert_eq!(
                normalize_hook_command_for_compare(got, true),
                normalize_hook_command_for_compare(want, true),
                "윈도우 표기 차이를 다른 명령으로 읽었다: {got}"
            );
            let s = u10_settings(&[("SessionStart", got)]);
            assert!(hook_registered_normalized_in(&s, "SessionStart", want, true), "{got}");
        }
        // 공백 포함 경로(따옴표 필수 형상)도 같은 규칙.
        assert_eq!(
            normalize_hook_command_for_compare(r#"bash "/c/Users/user/My Docs/.cys/pack/hooks/a.sh""#, true),
            normalize_hook_command_for_compare(r#"bash "C:\Users\user\my docs\.cys\pack\hooks\a.sh""#, true),
        );
    }

    /// 정규화가 **다른 명령을 같게 만들지 않는다**(느슨함의 상한) · 유닉스 규칙은 대소문자·MSYS 무변환.
    #[test]
    fn u10_hook_compare_does_not_merge_distinct_commands() {
        let w = |s: &str| normalize_hook_command_for_compare(s, true);
        assert_ne!(
            w(r#"bash "C:/Users/user/.cys/pack-dept-a/hooks/session-start.sh""#),
            w(r#"bash "C:/Users/user/.cys/pack/hooks/session-start.sh""#),
            "다른 팩"
        );
        assert_ne!(
            w(r#"bash "C:/Users/user/.cys/pack/hooks/session-start.sh""#),
            w(r#"bash "C:/Users/user/.cys/pack/hooks/role-bootstrap.sh""#),
            "다른 스크립트"
        );
        assert_ne!(
            w(r#"bash "C:/p/hooks/a.sh""#),
            w(r#"bash "D:/p/hooks/a.sh""#),
            "다른 드라이브"
        );
        let u = |s: &str| normalize_hook_command_for_compare(s, false);
        assert_ne!(u("sh /Users/user/.cys/Pack/hooks/a.sh"), u("sh /Users/user/.cys/pack/hooks/a.sh"), "유닉스는 대소문자 구분");
        assert_ne!(u("sh /c/x/hooks/a.sh"), u("sh c:/x/hooks/a.sh"), "유닉스는 MSYS 변환 없음");
        // 빈 문자열·이상 입력에서 패닉 0.
        for odd in ["", " ", "\"", "\\\\?\\", "/c", "/", "//?/", "bash \"\""] {
            let _ = w(odd);
            let _ = u(odd);
        }
    }

    /// U9(0.14.41): pack-update no-op 결과 토큰 — GUI 브리지(src-tauri parse_pack_update_outcome)와의 계약.
    #[test]
    fn pack_update_outcome_token_format() {
        assert_eq!(
            format_pack_update_outcome("up-to-date", "0.14.40", Some("0.14.40\n")),
            "PACK_UPDATE_OUTCOME gate=up-to-date remote=0.14.40 disk=0.14.40 disk_parse=ok"
        );
        // 디스크 판독 실패·빈 값·해석 불가 — CLI 는 UpToDate 지만 호출자가 '이미 적용됨'이라 말하면 안 된다.
        assert_eq!(
            format_pack_update_outcome("up-to-date", "0.14.40", None),
            "PACK_UPDATE_OUTCOME gate=up-to-date remote=0.14.40 disk=- disk_parse=fail"
        );
        assert_eq!(
            format_pack_update_outcome("up-to-date", "0.14.40", Some("garbage")),
            "PACK_UPDATE_OUTCOME gate=up-to-date remote=0.14.40 disk=garbage disk_parse=fail"
        );
        // 공백·비ASCII·BOM 이 섞인 값은 한 토큰으로 유지한다(줄 파싱 보호).
        let l = format_pack_update_outcome("up-to-date", "0.14.40", Some("\u{feff}0.14.40 x"));
        assert_eq!(l, "PACK_UPDATE_OUTCOME gate=up-to-date remote=0.14.40 disk=? disk_parse=fail");
        assert!(l.is_ascii());
        assert_eq!(l.split_whitespace().count(), 5);
    }

    /// ★리뷰1 F2(MR3 공허) — `pack_update_outcome_token_format`은 포맷 함수(`format_pack_update_outcome`)
    /// 만 본다. 실제로 디스크를 읽는 `pack_update_uptodate_line`은 어떤 lib 테스트도 부르지 않아서,
    /// 그 함수가 엉뚱한 파일을 읽거나(`.pack-state.json` 등) 항상 판독 실패를 반환해도 전 테스트가
    /// 녹색이었다. 임시 CYS_PACK_DIR 에 `.pack-version`을 직접 써 두고 그 값이 그대로 실리는지 본다
    /// (판정에 쓴 것과 같은 파일 · 같은 pack_dir()).
    #[test]
    fn pack_update_uptodate_line_reads_live_disk_pack_version() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let td = std::env::temp_dir().join(format!("cys-pack-uptodate-line-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let _env = EnvGuard::set("CYS_PACK_DIR", td.to_str().unwrap());

        // ① .pack-version 이 아직 없음 → 판독 실패(disk=- disk_parse=fail). 엉뚱한 파일을 읽어도
        //    이 값으로 우연히 맞을 수 있으므로 ②·③ 대조가 핵심이다.
        let line0 = pack_update_uptodate_line("0.14.41");
        assert_eq!(line0, "PACK_UPDATE_OUTCOME gate=up-to-date remote=0.14.41 disk=- disk_parse=fail");

        // ② .pack-version 에 쓴 값이 그대로 실린다 — 다른 파일을 읽으면(MR3) 여기서 불일치.
        std::fs::write(td.join(PACK_VERSION_FILE), "0.14.40").unwrap();
        let line1 = pack_update_uptodate_line("0.14.41");
        assert_eq!(line1, "PACK_UPDATE_OUTCOME gate=up-to-date remote=0.14.41 disk=0.14.40 disk_parse=ok");

        // ③ 다시 쓰면(CRLF 포함) 캐시된 옛 값이 아니라 **지금** 디스크 값을 다시 읽는다.
        std::fs::write(td.join(PACK_VERSION_FILE), "0.14.42\r\n").unwrap();
        let line2 = pack_update_uptodate_line("0.14.41");
        assert_eq!(line2, "PACK_UPDATE_OUTCOME gate=up-to-date remote=0.14.41 disk=0.14.42 disk_parse=ok");

        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★(0.14.39 라운드3 · 성찰2 notice) 레인 가드 표식의 경로·신선도 창은 러스트/파이썬 **사본 2벌**이다.
    /// 드리프트하면 `cys cycle-agent` 와 `javis_preflight` 가 서로 다른 레인 상태를 보고,
    /// 그 방향은 디렉티브 0회 주입(치명) 또는 상시 이중 주입(컨텍스트 급등) 어느 쪽으로도 갈 수 있다.
    #[test]
    fn lane_guard_constants_match_javis_preflight() {
        let preflight = include_str!("../cysjavis-pack/bin/javis_preflight.py");
        let rel = preflight
            .lines()
            .find_map(|line| line.strip_prefix("LANE_GUARD_TRIPPED_REL = "))
            .expect("파이썬 LANE_GUARD_TRIPPED_REL 줄 시작 정의");
        assert_eq!(
            rel.trim(),
            "os.path.join(\"state\", \"lane-guard-tripped\")",
            "표식 상대경로의 교차 언어 계약이 깨졌다(러스트: {LANE_GUARD_TRIPPED_REL})"
        );
        assert_eq!(
            LANE_GUARD_TRIPPED_REL,
            "state/lane-guard-tripped",
            "러스트 쪽 상수가 파이썬 os.path.join 조합과 다르다"
        );
        let recent = preflight
            .lines()
            .find_map(|line| line.strip_prefix("LANE_GUARD_RECENT_S = "))
            .expect("파이썬 LANE_GUARD_RECENT_S 줄 시작 정의");
        let secs: u64 = recent
            .trim()
            .split('*')
            .map(|term| term.trim().parse::<u64>().expect("정수 리터럴 곱"))
            .product();
        assert_eq!(secs, LANE_GUARD_RECENT_SECS, "표식 신선도 창(24h)의 교차 언어 계약이 깨졌다");
    }

    /// 레인 표식 검체마다 라이브 팩과 무관한 고유 임시 디렉터리를 만든다.
    fn lane_guard_test_pack(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let pack = std::env::temp_dir().join(format!(
            "cys-lane-guard-{name}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(pack.join("state")).unwrap();
        pack
    }

    /// 증명된 표식 부재만 레인 가드 무발화 기록 없음으로 판정한다.
    #[test]
    fn lane_guard_tripped_absent_marker_is_none() {
        let pack = lane_guard_test_pack("absent");
        assert_eq!(lane_guard_tripped(&pack), None);
        std::fs::remove_dir_all(pack).unwrap();
    }

    /// 방금 쓴 absent 표식은 판독 가능한 최근 조기 종료 기록이다.
    #[test]
    fn lane_guard_tripped_recent_marker_preserves_reason() {
        let pack = lane_guard_test_pack("recent");
        std::fs::write(pack.join(LANE_GUARD_TRIPPED_REL), "reason=absent\n").unwrap();
        let trip = lane_guard_tripped(&pack).expect("최근 표식을 부재로 접었다");
        assert_eq!(trip.reason, "absent");
        assert_eq!(trip.script, "");
        assert!(!trip.unreadable);
        std::fs::remove_dir_all(pack).unwrap();
    }

    /// mtime보다 25시간 뒤를 관측하면 낡은 표식은 오늘의 발화 판정에서 제외한다.
    #[test]
    fn lane_guard_tripped_stale_marker_is_none() {
        let pack = lane_guard_test_pack("stale");
        let marker = pack.join(LANE_GUARD_TRIPPED_REL);
        std::fs::write(&marker, "reason=absent\n").unwrap();
        let mtime = std::fs::metadata(&marker).unwrap().modified().unwrap();
        let now = mtime + std::time::Duration::from_secs(25 * 3600);
        assert_eq!(lane_guard_tripped_at(&pack, now), None);
        std::fs::remove_dir_all(pack).unwrap();
    }

    /// 위임 줄 부재의 사유와 SessionStart 스크립트를 원문 그대로 읽는다.
    #[test]
    fn lane_guard_tripped_parses_reason_and_script() {
        let pack = lane_guard_test_pack("fields");
        std::fs::write(
            pack.join(LANE_GUARD_TRIPPED_REL),
            "hook_root=/old\nlane_root=/lane\nreason=no-redirect-line\nscript=session-start.sh\nsurface=claude\nts=0\n",
        )
        .unwrap();
        let trip = lane_guard_tripped(&pack).unwrap();
        assert_eq!(trip.reason, "no-redirect-line");
        assert_eq!(trip.script, "session-start.sh");
        assert!(!trip.unreadable);
        std::fs::remove_dir_all(pack).unwrap();
    }

    /// 표식이 디렉터리여서 판독 불가하면 낡은 mtime으로도 무발화 기록을 지우지 않는다.
    #[test]
    fn lane_guard_tripped_unreadable_marker_is_conservative() {
        let pack = lane_guard_test_pack("unreadable");
        let marker = pack.join(LANE_GUARD_TRIPPED_REL);
        std::fs::create_dir(&marker).unwrap();
        let mtime = std::fs::metadata(&marker).unwrap().modified().unwrap();
        let trip = lane_guard_tripped_at(&pack, mtime + std::time::Duration::from_secs(25 * 3600))
            .expect("판독 불가를 부재나 낡은 표식으로 접었다");
        assert!(trip.unreadable);
        assert_eq!(trip.reason, "");
        assert_eq!(trip.script, "");
        std::fs::remove_dir_all(pack).unwrap();
    }

    /// 신선도는 정확히 24시간까지 포함하며 미래 mtime의 음수 나이는 0으로 접는다.
    #[test]
    fn lane_guard_tripped_recent_window_includes_boundary_and_future() {
        let pack = lane_guard_test_pack("clock");
        let marker = pack.join(LANE_GUARD_TRIPPED_REL);
        std::fs::write(&marker, "reason=already-redirected\n").unwrap();
        let mtime = std::fs::metadata(&marker).unwrap().modified().unwrap();
        assert!(lane_guard_tripped_at(
            &pack,
            mtime + std::time::Duration::from_secs(LANE_GUARD_RECENT_SECS)
        )
        .is_some());
        assert!(lane_guard_tripped_at(&pack, mtime - std::time::Duration::from_secs(1)).is_some());
        std::fs::remove_dir_all(pack).unwrap();
    }

    /// 빈 사유도 표식이며 미지 키는 무시하고 Python처럼 UTF-8 손상 바이트를 대체한다.
    #[test]
    fn lane_guard_tripped_empty_reason_and_lossy_text_remain_observed() {
        let pack = lane_guard_test_pack("text");
        std::fs::write(
            pack.join(LANE_GUARD_TRIPPED_REL),
            b"unknown=absent\n reason=not-a-key\nreason=\nscript=session-\xff.sh\n",
        )
        .unwrap();
        let trip = lane_guard_tripped(&pack).unwrap();
        assert_eq!(trip.reason, "");
        assert_eq!(trip.script, "session-\u{fffd}.sh");
        assert!(!trip.unreadable);
        std::fs::remove_dir_all(pack).unwrap();
    }

    /// 끊어진 심링크와 writer 없는 FIFO는 부재로 접거나 무한 대기하지 않고 판독 불가로 막는다.
    #[cfg(unix)]
    #[test]
    fn lane_guard_tripped_dangling_link_and_fifo_are_unreadable() {
        use std::os::unix::ffi::OsStrExt;
        let pack = lane_guard_test_pack("nonregular");
        let marker = pack.join(LANE_GUARD_TRIPPED_REL);
        std::os::unix::fs::symlink(pack.join("missing"), &marker).unwrap();
        assert!(lane_guard_tripped(&pack).unwrap().unreadable);
        std::fs::remove_file(&marker).unwrap();
        let fifo = std::ffi::CString::new(marker.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(lane_guard_tripped(&pack).unwrap().unreadable);
        std::fs::remove_dir_all(pack).unwrap();
    }

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
        // ★0.15.0 M11 가(H-RW 재작성 — 종전: "system 수정본(매니페스트 부재·상이) → 강제 치유(P0-4)하되
        //   사용자본 .user 보존"): 이제 사용자 파일이 살아 있는 파일로 남고 vendor 신버전은 `.new`(user-kept).
        //   P0-4(설치 기록 부재 ≠ 사용자 수정)는 과거 vendor 해시 표가 지킨다(아래 다 행 · 부록 A-2 #1).
        assert_eq!(decide_file_action("bin/x.py", embed, true, Some("HACKED"), None, false),
                   UserKept { reason: "no-base" });
        // ★0.15.0 M11 자(종전: "force 로 system 수정본을 덮을 때도 사용자본 보존"): force 는 사용자 수정을
        //   버리지 않는다 — 같은 판정(user-kept).
        assert_eq!(decide_file_action("bin/x.py", embed, true, Some("HACKED"), None, true),
                   UserKept { reason: "no-base" });
        // ★나: 기동 필수 파일이면 vendor 본 우선 + 사용자본 시각별 보관(vendor-first).
        let bc = UpdateFacts { boot_critical: true, ..UpdateFacts::default() };
        assert_eq!(super::decide_file_action_with("bin/x.py", embed, true, Some("HACKED"), None,
                       false, PackScope::Base, None, &bc),
                   VendorFirst { reason: "no-base" });
        // ★다(P0-4 계승): 매니페스트 부재여도 디스크가 이 파일의 과거 vendor 본이면 사용자 수정이 아니다 → 갱신.
        let prev = UpdateFacts { prev_vendor_match: true, ..UpdateFacts::default() };
        assert_eq!(super::decide_file_action_with("bin/x.py", embed, true, Some("HACKED"), None,
                       false, PackScope::Base, None, &prev),
                   Write { heal_user_copy: false });
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
        // ② ★0.15.0 M11 가(H-RW 재작성 — 종전: "system 수정본: 치유(임베드) + 사용자본 .user 보존 + 원장 healed"):
        //   기준본 없는 system 수정본은 사용자 파일이 살아 있고 vendor 신버전은 `.new`, 원장 user-kept(파괴 0 은 그대로).
        assert_eq!(read(sys_c), "SYS-DRIFT", "system 수정본 = 사용자 파일 유지(가)");
        assert_eq!(read(&format!("{sys_c}.new")), get(sys_c), "vendor 신버전 .new 보관");
        assert!(!td.join(format!("{sys_c}.user")).exists(), "사용자 우선 — .user 미생성");
        let pending = load_merge_pending(&td);
        assert_eq!(pending.get(user_b).and_then(|e| e["kind"].as_str()), Some("new-pending"));
        assert_eq!(pending.get(sys_c).and_then(|e| e["kind"].as_str()), Some("user-kept"));
        // ③ pristine 미러: 적용된 vendor 본만(sys_a), 동결 user-owned(user_b)·사용자 우선 system(sys_c)은 미기록
        //   (조상 보존 — sys_c 는 vendor 본이 디스크에 적용되지 않았다).
        assert_eq!(read(&format!("{PRISTINE_DIR}/{sys_a}")), get(sys_a));
        assert!(!td.join(PRISTINE_DIR).join(sys_c).exists(), "적용되지 않은 vendor 본은 조상으로 쓰지 않는다");
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

    /// D-04 ③: 옛 vendor agents.json 은 팩 갱신 시 보존되고, 마커만 읽기 시점에 승격된다.
    #[test]
    fn d04_pack_update_keeps_user_owned_agents_json_and_runtime_promotes_stale_marker() {
        use crate::agent_markers::{marker_candidates, promote_stale_vendor_defaults};
        use serde_json::json;

        let _g = PACK_ENV_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!("cys-pack-d04-marker-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let td = root.join("pack-dept-1");
        std::fs::create_dir_all(&td).unwrap();
        let _env = set_pack_env(&td, td.join("cysclaude"));
        // set_pack_env 는 경로만 격리한다. install 은 basename 으로 Dept 를 판정하며,
        // 부서 스코프도 설치를 막지 않고 agents.json 의 User 소유권을 그대로 유지한다.
        assert_eq!(pack_scope_of(&pack_dir()), PackScope::Dept);
        assert_eq!(ownership_scoped("agents.json", PackScope::Dept), Ownership::User);

        let embedded = PACK_ALL.iter().find(|(rel, _)| *rel == "agents.json")
            .map(|(_, content)| *content).expect("팩에 agents.json 부재");
        let embed: serde_json::Value = serde_json::from_str(embedded).unwrap();
        let mut stale = embed.clone();
        stale["codex"]["prompt_marker"] = json!("›");
        stale["gemini"].as_object_mut().unwrap().remove("prompt_marker");
        let old_vendor = serde_json::to_string_pretty(&stale).unwrap();
        std::fs::write(td.join("agents.json"), &old_vendor).unwrap();
        // 0.14.38 설치 형상 + 설치 당시 해시: 사용자가 수정하지 않은 옛 vendor 본이다.
        let manifest = json!({"agents.json": content_hash(&old_vendor)});
        std::fs::write(td.join(INSTALL_MANIFEST), manifest.to_string()).unwrap();

        install(false, None).expect("install 실패");
        let read = |rel: &str| std::fs::read(td.join(rel)).unwrap();
        assert_eq!(read("agents.json"), old_vendor.as_bytes(), "옛 vendor 본도 user-owned 로 보존");
        assert_eq!(read("agents.json.new"), embedded.as_bytes(), ".new = 임베드 신버전");
        let pending = load_merge_pending(&td);
        assert_eq!(pending.get("agents.json").and_then(|e| e["kind"].as_str()), Some("new-pending"));
        assert_eq!(pending.get("agents.json").and_then(|e| e["side"].as_str()), Some("agents.json.new"));
        assert_eq!(embed["codex"]["prompt_marker"], json!(["›", "»"]));
        assert_eq!(embed["gemini"]["prompt_marker"], json!([">"]));

        // 디스크 우선 읽기: 설치가 보존한 옛 codex 기본값은 메모리에서만 승격한다.
        let disk: serde_json::Value = serde_json::from_slice(&read("agents.json")).unwrap();
        let mut spec = disk["codex"].clone();
        assert_eq!(
            promote_stale_vendor_defaults("codex", &mut spec, Some(&embed["codex"])),
            vec!["prompt_marker"]
        );
        assert_eq!(spec["prompt_marker"], json!(["›", "»"]));
        // gemini 는 키 부재 → 후보 없음 → 데몬 merged_prompt_marker 순서상 임베드로 폴백.
        assert!(disk["gemini"].get("prompt_marker").is_none());
        assert!(marker_candidates(disk["gemini"].get("prompt_marker")).is_empty());
        assert_eq!(marker_candidates(embed["gemini"].get("prompt_marker")), vec![">"]);
        let mut custom = disk["codex"].clone();
        custom["prompt_marker"] = json!("▶");
        let custom_before = custom.clone();
        assert!(
            promote_stale_vendor_defaults("codex", &mut custom, Some(&embed["codex"])).is_empty()
        );
        assert_eq!(custom, custom_before, "사용자 커스텀은 승격하지 않는다");

        // 재설치도 디스크·병치본·new-pending 항목을 보존하고 원장을 중복 계상하지 않는다.
        install(false, None).expect("재실행 실패");
        assert_eq!(read("agents.json"), old_vendor.as_bytes());
        assert_eq!(read("agents.json.new"), embedded.as_bytes());
        let pending_after = load_merge_pending(&td);
        assert_eq!(pending_after.get("agents.json").and_then(|e| e["kind"].as_str()), Some("new-pending"));
        assert_eq!(pending_after.get("agents.json"), pending.get("agents.json"));
        assert_eq!(pending_after.len(), pending.len(), "원장 항목 수 불변");

        std::fs::remove_dir_all(&root).unwrap();
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
        // ★0.15.0 M11 가(H-RW-11 재작성 — 종전: "system 수정 → heal"): 기록 없는 system 수정본은 사용자 우선
        //   (user-kept) 버킷이다 — heal 로 오보하지 않는다.
        assert!(plan.user_kept.iter().any(|r| r == "alerts-config.json"), "system 수정(기준본 없음) → user_kept");
        assert!(!plan.heal.iter().any(|r| r == "alerts-config.json"), "user-kept 를 heal 로 오보 금지");
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
        // ★0.15.0 M11 가(plan=actual — 종전: alerts-config.json.user 존재): 사용자 파일 유지 + vendor `.new` + 원장 user-kept.
        assert_eq!(read("alerts-config.json"), "SYS-DRIFT", "user-kept: 사용자 파일 유지(plan=actual)");
        assert!(td.join("alerts-config.json.new").exists(), "user-kept: vendor 신버전 .new");
        assert!(!td.join("alerts-config.json.user").exists(), "user-kept: .user 미생성");
        assert_eq!(
            load_merge_pending(&td).get("alerts-config.json").and_then(|e| e["kind"].as_str()),
            Some("user-kept"),
            "user-kept 원장 계상(plan=actual)"
        );
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
        let ent2 = pend2.get(rel).expect("전환 후 항목 잔존");
        // ★0.15.0 M11 가(H-RW 재작성 — 종전: kind=healed · .user=병합본 · disk=vendor): L4 결말이 사용자 우선이라
        //   kind 는 user-kept 로 전환되고 병합본이 디스크에 남는다 — D7 계보 규칙(capture 승계)은 그대로 고정한다.
        assert_eq!(ent2["kind"].as_str(), Some("user-kept"), "★D7: kind 전환 허용(user-kept)");
        assert_eq!(ent2["capture"].as_str(), Some(capture_ptr),
                   "★D7: capture 승계(크래시 창 캡처 포인터 소실 금지)");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), merged_disk, "disk = 병합본 유지(손실 0)");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.new"))).unwrap(), e1, "vendor 본 .new");

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
                + c.adopted
                + c.user_kept
                + c.vendor_first;
            assert_eq!(named_sum, 1, "등재 kind {kind:?} 계상 오류(명시 버킷 합 {named_sum})");
        }
        // ①-b ★0.15.0 M11(O-C8-1): 신 kind 2종이 등재돼 있고 조치 대상(actionable)이다 — 둘 다 과제가
        // 남은 상태(user-kept = 워커의 합치기, vendor-first = 워커의 다시 입히기 + 주인 승인)라 at-rest 가 아니다.
        assert_eq!(LEDGER_KINDS.len(), 9, "등재 kind 수(0.15.0: 7 + user-kept·vendor-first)");
        for kind in ["user-kept", "vendor-first"] {
            assert!(LEDGER_KINDS.contains(&kind), "신 kind {kind} 미등재");
            let mut pending = serde_json::Map::new();
            pending.insert(
                "z".into(),
                serde_json::json!({"kind": kind, "side": "z", "version": "1.0.0", "ts": 1}),
            );
            assert_eq!(pending_kind_counts(&pending).actionable(), 1, "{kind} 는 조치 대상이어야 한다");
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
                user_kept: 1,
                vendor_first: 1,
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
            // ★0.15.0 M11 가(H-RW 재작성 — 종전: "[W3] disk=vendor 보증(healed) · .user=병합본 · kind=healed"):
            //   좁힘(병합 대기 항목 → 출처 '그 병합' — 부록 A-2 #5)은 그대로이고, base 검증 실패의 L4 결말이 사용자
            //   우선으로 바뀌었다 — 병합본(사용자 δ 포함)이 살아 있고 vendor 본은 `.new`, 계보 capture 는 승계.
            assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), merged,
                       "[W3] 가: 병합본(사용자 δ) 유지 — {rel}");
            assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.new"))).unwrap(), e1,
                       "[W3] 가: vendor 본 .new — {rel}");
            let e = pend3.get(rel).unwrap();
            assert_eq!(e["kind"].as_str(), Some("user-kept"), "[W3] kind=user-kept — {rel}");
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
        // ★0.15.0 M11 가(H-RW 재작성 — 종전: vendor 본 기록 + .user + .base + conflicted): 캡처 금지면 병합을
        //   시도하지 않는 것은 그대로이고(§2 원칙 4), 기동 필수가 아닌 파일은 사용자 파일이 살아 있다.
        assert_eq!(std::fs::read_to_string(pd2.join(wc)).unwrap(), ours, "[W-cap] 가: 사용자 파일 유지");
        assert_eq!(std::fs::read_to_string(pd2.join(format!("{wc}.new"))).unwrap(), e1,
                   "[W-cap] 가: vendor 본 .new");
        let pend_c = load_merge_pending(&pd2);
        let ec = pend_c.get(wc).unwrap();
        assert_eq!(ec["kind"].as_str(), Some("user-kept"), "[W-cap] 원장 user-kept");
        assert_eq!(ec["reason"].as_str(), Some("capture-failed"), "[W-cap] reason 토큰");
        // ★RULINGS BO1 재작성(종전: "나(기동 필수): 종전 폴백 — vendor 본 + .user + .base + 원장
        //   vendor-first{capture-failed}" — 설치기가 먼저 3-way 를 시도하다 캡처에 실패했다): 기동 필수 파일은 설치 때
        //   병합하지 않으므로 캡처할 일도 없다 — vendor 본 + .user(사용자본) + 원장 vendor-first{vendor-advanced} · .base 없음.
        let wb = "skills/wcb/SKILL.md";
        {
            let p = pd2.join(wb);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, ours).unwrap();
            let pp = pd2.join(PRISTINE_DIR).join(wb);
            std::fs::create_dir_all(pp.parent().unwrap()).unwrap();
            std::fs::write(&pp, e_old).unwrap();
        }
        std::fs::write(pd2.join(INSTALL_MANIFEST),
            serde_json::json!({ wb: content_hash(e_old) }).to_string()).unwrap();
        install_boot_critical(&pd2, &[(wb, e1)], "1.1.0");
        assert_eq!(std::fs::read_to_string(pd2.join(wb)).unwrap(), e1, "[W-cap] 나: vendor 본 기록");
        assert_eq!(std::fs::read_to_string(pd2.join(format!("{wb}.user"))).unwrap(), ours,
                   "[W-cap] 나: .user=사용자본(§2 원칙 4 보존)");
        assert!(!pd2.join(format!("{wb}.base")).exists(), "[W-cap] BO1: 병합을 시도하지 않으므로 .base 없음");
        let eb = load_merge_pending(&pd2).get(wb).cloned().unwrap();
        assert_eq!(eb["kind"].as_str(), Some("vendor-first"), "[W-cap] 나: 원장 vendor-first");
        assert_eq!(eb["reason"].as_str(), Some("vendor-advanced"), "[W-cap] BO1: reason = vendor 전진");
        assert!(eb.get("parked_at").is_some(), "[W-cap] 나: 보관 시각");

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
        // 자리 디렉터리 점유.
        std::fs::write(base.join("pack-captures"), "OCCUPIED").unwrap();
        std::fs::create_dir_all(pd.join(format!("{rel}.user"))).unwrap();

        // ★0.15.0 M11 가(H-RW 재작성 — 종전: 이중 실패 → quarantined): 기동 필수가 아닌 파일은 덮어쓸 일이
        //   없다 — 캡처 실패면 병합을 시도하지 않고 사용자 파일이 살아 있다(손실 경로 자체가 없다).
        install_from_iter([(rel, e1)], false, "1.1.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), ours,
                   "가: 파일 무접촉(사용자 우선)");
        let mani: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(pd.join(INSTALL_MANIFEST)).unwrap())
                .unwrap();
        assert_eq!(mani.get(rel), Some(&content_hash(e_old)), "manifest 미전진(재스윕 수렴 조건)");
        assert_eq!(std::fs::read_to_string(pd.join(PRISTINE_DIR).join(rel)).unwrap(), e_old,
                   "pristine 미전진");
        assert!(pd.join(format!("{rel}.user")).is_dir(), "가: .user 자리를 건드리지 않는다");
        let e = load_merge_pending(&pd).get(rel).cloned().unwrap();
        assert_eq!(e["kind"].as_str(), Some("user-kept"), "가: 원장 user-kept");
        assert_eq!(e["reason"].as_str(), Some("capture-failed"), "가: 사유 토큰");
        // 회복 후 재스윕 = 병합을 다시 시도한다(capture-failed 보관은 재시도 금지 대상이 아니다) → 정상 3-way 수렴.
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

        // 나(기동 필수 · 부록 A-2 #4 '백업이 없으면 덮지 않는다'): 캡처도 .user 백업도 불가능하면 vendor 기록·
        //   manifest·pristine 전진을 전부 멈추고 격리한다(종전 이중 실패 주입 그대로 — 캡처 루트 파일 점유 +
        //   `.user` 자리 디렉터리 점유; 설치기는 파일이 아닌 `.user` 를 세대로 옮기지 않는다).
        std::fs::write(pd.join(rel), ours).unwrap();
        std::fs::write(pd.join(PRISTINE_DIR).join(rel), e_old).unwrap();
        std::fs::write(pd.join(INSTALL_MANIFEST),
            serde_json::json!({ rel: content_hash(e_old) }).to_string()).unwrap();
        let _ = save_merge_pending(&pd, &serde_json::Map::new());
        let _ = std::fs::remove_dir_all(base.join("pack-captures")); // 가 회복 스윕이 만든 캡처 디렉터리
        std::fs::write(base.join("pack-captures"), "OCCUPIED").unwrap();
        std::fs::create_dir_all(pd.join(format!("{rel}.user"))).unwrap();
        install_boot_critical(&pd, &[(rel, e1)], "1.1.0");
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), ours,
                   "나: 이중 실패 = 파일 무접촉(vendor 기록 스킵 — 사용자 바이트 소실 경로 봉인)");
        let mani: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(pd.join(INSTALL_MANIFEST)).unwrap())
                .unwrap();
        assert_eq!(mani.get(rel), Some(&content_hash(e_old)), "나: manifest 미전진");
        assert_eq!(std::fs::read_to_string(pd.join(PRISTINE_DIR).join(rel)).unwrap(), e_old,
                   "나: pristine 미전진");
        let e = load_merge_pending(&pd).get(rel).cloned().unwrap();
        assert_eq!(e["kind"].as_str(), Some("quarantined"), "나: 원장 quarantined 전이(D11 동형)");
        assert_eq!(e["reason"].as_str(), Some("user-backup-failed"), "나: 이중 실패 사유 토큰");
        // ★RULINGS BO1 재작성(종전: "회복 후 재스윕 = 동일 Merge3 판정 재도달 → 정상 병합 수렴" — 병합본
        //   "head-new / common body / user-tail-delta" 이 살아 있는 파일이 됐다): 기동 필수 파일은 설치 때 병합하지 않는다 —
        //   회복 후 재스윕은 vendor 우선(vendor 본 + .user = 사용자본 + 원장 vendor-first) · 침묵 실패 0 · 손실 0.
        std::fs::remove_file(base.join("pack-captures")).unwrap();
        std::fs::remove_dir_all(pd.join(format!("{rel}.user"))).unwrap();
        install_boot_critical(&pd, &[(rel, e1)], "1.1.0");
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), e1, "나: 회복 후 vendor 본(병합본 아님 — BO1)");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.user"))).unwrap(), ours, "나: 사용자본 보관");
        assert_eq!(load_merge_pending(&pd).get(rel).and_then(|e| e["kind"].as_str()), Some("vendor-first"));
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

    /// ★0.15.0 M11: 기동 필수 이음새로 설치(나 — vendor 본 우선 폴백 기계를 그대로 고정하는 재작성 시험용).
    fn install_boot_critical(pd: &std::path::Path, items: &[(&str, &str)], version: &str) {
        let rels: Vec<String> = items.iter().map(|(r, _)| r.to_string()).collect();
        let src = UpdateSources::none().with_boot_critical(move |r: &str| rels.iter().any(|x| x == r));
        install_into_with(pd.to_path_buf(), items.iter().copied(), false, version, false, false,
                          PackScope::Base, None, None, &src).unwrap();
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
        // ★0.15.0 M11 가(H-RW 재작성 — 종전: json 게이트 → vendor 본 + .user + .base + conflicted): 게이트 배선은
        //   그대로 박제하되(손상 병합 차단), 기동 필수가 아닌 파일은 사용자 파일이 살아 있다.
        install_from_iter([(rel, theirs)], false, "1.1.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), ours,
            "[json-gate] 가: 파스 불능 병합본은 쓰지 않고 사용자 파일 유지");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.new"))).unwrap(), theirs,
            "[json-gate] 가: vendor 신버전 .new");
        assert_eq!(find_capture(&base_td.join("caps"), rel), None,
            "[json-gate] 가: 이번 시도의 병합 전 사본 정리");
        let pend = load_merge_pending(&pd);
        let e = pend.get(rel).unwrap();
        assert_eq!(e["kind"].as_str(), Some("user-kept"), "[json-gate] 원장 user-kept");
        assert_eq!(e["reason"].as_str(), Some("json-gate"), "[json-gate] reason 토큰");
        // ★RULINGS BO1 재작성(종전: "나(기동 필수): 종전 폴백 기계 그대로 — vendor 본 + .user + .base + 캡처 + 원장
        //   vendor-first{json-gate}" — 설치기가 병합을 시도한 뒤 JSON 관문에서 떨어졌다): 기동 필수 파일은 설치 때 병합하지
        //   않는다 — vendor 본 + .user(사용자본) + 원장 vendor-first{vendor-advanced} · 캡처·.base 없음.
        let _ = std::fs::remove_dir_all(&pd);
        std::fs::create_dir_all(&pd).unwrap();
        seed_merge3_shape(&pd, rel, ours, base);
        install_boot_critical(&pd, &[(rel, theirs)], "1.1.0");
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), theirs,
            "[json-gate] 나: disk=vendor 보증");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.user"))).unwrap(), ours,
            "[json-gate] 나: .user=사용자본 보존");
        assert!(!pd.join(format!("{rel}.base")).exists(), "[json-gate] BO1: .base 없음");
        assert_eq!(find_capture(&base_td.join("caps"), rel), None, "[json-gate] BO1: 병합 시도 없음 = 캡처 없음");
        let pend = load_merge_pending(&pd);
        let e = pend.get(rel).unwrap();
        assert_eq!(e["kind"].as_str(), Some("vendor-first"), "[json-gate] 나: 원장 vendor-first");
        assert_eq!(e["reason"].as_str(), Some("vendor-advanced"), "[json-gate] BO1: reason = vendor 전진");
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
        // ★0.15.0 M11 가(H-RW 재작성 — 종전: suspect → vendor 본 + .user + .base + conflicted): 세탁 clean 병합은
        //   여전히 받아들이지 않되(부록 A-2 #6), 의심은 '덮는 이유'가 아니라 합칠 때의 주의 표시다(설계 §7 2행).
        install_from_iter([(rel, theirs)], false, "1.1.0", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), ours,
            "[suspect] 가: 세탁 병합본을 쓰지 않고 사용자 파일 유지");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.new"))).unwrap(), theirs);
        let pend = load_merge_pending(&pd);
        let e = pend.get(rel).unwrap();
        assert_eq!(e["kind"].as_str(), Some("user-kept"), "[suspect] 가: 원장 user-kept");
        let reason = e["reason"].as_str().unwrap_or("");
        assert!(reason.starts_with("suspect:PureDeletionMajority"),
            "[suspect] reason=순삭제 휴리스틱 토큰(실측: {reason})");
        assert!(e["caution"].as_str().unwrap_or("").starts_with("suspect:PureDeletionMajority"),
            "[suspect] 가: 주의 표시(caution)");
        // ★RULINGS BO1 재작성(종전: "나(기동 필수): 종전 폴백 기계 — vendor 본 + .user + .base + 원장
        //   vendor-first{suspect}" — 병합을 시도한 뒤 손상 의심으로 떨어졌다): 기동 필수 파일은 설치 때 병합하지 않는다 —
        //   vendor 본 + .user(사용자본) + 원장 vendor-first{vendor-advanced} · .base 없음(세탁된 병합이 살아날 길 자체가 없다).
        let _ = std::fs::remove_dir_all(&pd);
        std::fs::create_dir_all(&pd).unwrap();
        seed_merge3_shape(&pd, rel, ours, base);
        install_boot_critical(&pd, &[(rel, theirs)], "1.1.0");
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), theirs,
            "[suspect] 나: disk=vendor 보증(세탁 clean 병합 차단)");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.user"))).unwrap(), ours,
            "[suspect] 나: .user=사용자본 보존");
        assert!(!pd.join(format!("{rel}.base")).exists(), "[suspect] BO1: .base 없음");
        let e = load_merge_pending(&pd).get(rel).cloned().unwrap();
        assert_eq!(e["kind"].as_str(), Some("vendor-first"), "[suspect] 나: 원장 vendor-first");
        assert_eq!(e["reason"].as_str(), Some("vendor-advanced"), "[suspect] BO1: reason = vendor 전진");
        let _ = std::fs::remove_dir_all(&base_td);
    }

    /// ★성찰 차단 수리 2R-4 핀 ⓒ(v2 §4 ④ pristine 검증형 승격): pristine 기록 실패 주입 시
    /// ④ 를 중단하고 ⑤ healed 폴백으로 강등 — .user=병합본(ours 원본은 캡처) ·
    /// conflicted{reason:pristine-write-failed} · pristine 미전진을 박제.
    ///
    /// ## ★주입 기제 교체(2026-09-05 · H-CONC-3)
    /// 종전 주입은 "`write_atomic` 의 tmp 슬롯 `.{fname}.tmp.{pid}` 자리를 디렉터리로 점유"였다.
    /// 그 주입은 **결함 자체에 기대고 있었다** — tmp 이름이 pid 고정이라 미리 알 수 있었던 것이
    /// 곧 H-CONC-3 의 근인이다. 이름이 호출마다 유일해지자 점유는 아무것도 막지 못하고 ④ 가
    /// 성공해 이 핀이 조용히 무력화됐다(실측: 이 검체가 유일하게 적색으로 그것을 알렸다).
    /// 그래서 **이름에 의존하지 않는** 기제로 바꾼다:
    ///  · unix — 부모 디렉터리를 `0o555` 로. tmp 생성 자체가 `EACCES` 라 이름과 무관하다.
    ///  · windows — 대상 파일을 읽기전용으로. `MoveFileEx(MOVEFILE_REPLACE_EXISTING)` 이
    ///    읽기전용 대상을 거부한다는 문서 근거로 배선했다. **이 갈래는 unix 기계에서 미검증**
    ///    이므로, Windows 러너에서 적색이 나면 그것은 결함이 아니라 이 주입의 재선택 신호다.
    /// 두 갈래 모두 **기존 pristine 파일을 읽을 수 있게** 남긴다(아래 '미전진' 단언 유지).
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
        // 주입(이름 비의존 · 위 머리말 참조) — 결정론으로 pristine 기록만 실패시킨다. bc = 기동 필수 이음새.
        let run_arm = |bc: bool| {
            let _ = std::fs::remove_dir_all(&pd);
            std::fs::create_dir_all(&pd).unwrap();
            seed_merge3_shape(&pd, rel, ours, base);
            #[cfg(unix)]
            let restore = {
                use std::os::unix::fs::PermissionsExt;
                let dir = pd.join(PRISTINE_DIR).join("skills/wpri");
                let prev = std::fs::metadata(&dir).unwrap().permissions();
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
                (dir, prev)
            };
            #[cfg(not(unix))]
            let restore = {
                let f = pd.join(PRISTINE_DIR).join(rel);
                let prev = std::fs::metadata(&f).unwrap().permissions();
                let mut ro = prev.clone();
                ro.set_readonly(true);
                std::fs::set_permissions(&f, ro).unwrap();
                (f, prev)
            };
            let installed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if bc {
                    install_boot_critical(&pd, &[(rel, theirs)], "1.1.0");
                } else {
                    install_from_iter([(rel, theirs)], false, "1.1.0", false, None).unwrap();
                }
            }));
            // 단언 **전에** 원복한다 — 패닉으로 빠져나가도 읽기전용 잔재가 다음 런을 막지 않게.
            std::fs::set_permissions(&restore.0, restore.1).unwrap();
            if let Err(p) = installed {
                std::panic::resume_unwind(p);
            }
        };
        // ★0.15.0 M11 가(H-RW 재작성 — 종전: "disk=vendor · .user=병합본 · conflicted"): 기동 필수가 아닌 파일은
        //   pristine 실패로 ④ 를 끝내지 못해도 vendor 본으로 덮지 않는다 — 디스크에는 이미 병합본(사용자 수정 +
        //   vendor 변경)이 있고 그대로 남는다. pristine 미전진 · 캡처 보존 · 원장 user-kept{pristine-write-failed}.
        run_arm(false);
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), merged,
            "[pristine-fail] 가: 디스크 = 병합본(사용자 수정 생존)");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.new"))).unwrap(), theirs);
        assert!(!pd.join(format!("{rel}.user")).exists(), "[pristine-fail] 가: .user 미생성");
        assert_eq!(std::fs::read_to_string(pd.join(PRISTINE_DIR).join(rel)).unwrap(), base,
            "[pristine-fail] pristine 미전진(실패 격리)");
        let e = load_merge_pending(&pd).get(rel).cloned().unwrap();
        assert_eq!(e["kind"].as_str(), Some("user-kept"), "[pristine-fail] 가: 원장 user-kept");
        assert_eq!(e["reason"].as_str(), Some("pristine-write-failed"), "[pristine-fail] reason 토큰");
        let cap = e["capture"].as_str().expect("[pristine-fail] 가: 캡처 포인터(ours 원본 증거)").to_string();
        assert_eq!(std::fs::read_to_string(base_td.join("caps").join(cap)).unwrap(), ours,
            "[pristine-fail] 가: 캡처=ours 원본");
        // ★RULINGS BO1 재작성(종전: "나(기동 필수): 종전 폴백 기계 — disk=vendor · .user=병합본 · 캡처=ours · pristine
        //   미전진 · vendor-first{pristine-write-failed}" — 병합본을 만든 뒤 pristine 실패로 떨어졌다): 기동 필수 파일은 설치 때
        //   병합하지 않는다 — 병합본은 어디에도 없다. disk=vendor · .user=사용자본(ours) · 캡처 없음 · 원장 vendor-first{vendor-advanced}.
        // 일반 파일 갈래가 남긴 캡처(증거)를 치우고 잰다 — 이 갈래가 새 캡처를 만들지 않는다는 것을 보기 위해.
        let _ = std::fs::remove_dir_all(base_td.join("caps"));
        run_arm(true);
        assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), theirs,
            "[pristine-fail] 나: disk=vendor 보증");
        assert_eq!(std::fs::read_to_string(pd.join(format!("{rel}.user"))).unwrap(), ours,
            "[pristine-fail] BO1: .user=사용자본(병합본이 아니다)");
        assert_eq!(find_capture(&base_td.join("caps"), rel), None, "[pristine-fail] BO1: 병합 시도 없음 = 캡처 없음");
        let pend = load_merge_pending(&pd);
        let e = pend.get(rel).unwrap();
        assert_eq!(e["kind"].as_str(), Some("vendor-first"), "[pristine-fail] 나: 원장 vendor-first");
        assert_eq!(e["reason"].as_str(), Some("vendor-advanced"), "[pristine-fail] BO1: reason = vendor 전진");
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
    /// 시에도 치유를 강행한다(무결성>보존 — §2 원칙 4 의 유일 예외 · ★T2i RULINGS GD1 로 되살림). 원장 side 는 **실백업 성공
    /// 시에만** 기록(실패 시 side 부재 + backup:"failed" + 그 사유 — 존재하지 않는 .user 를 '보존됨'으로 오보하지 않는다).
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
            // ★T2i(RULINGS GD1): 보관 실패의 사유도 적는다.
            assert!(p2.get(rel).and_then(|e| e["backup_error"].as_str()).is_some_and(|s| !s.is_empty()), "② 사유");
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
        // ★0.15.0 M11 가(H-RW 재작성 — 종전: "③★B2/P0-4: system 매니페스트부재·상이 → 강제 갱신(동결 금지)"):
        //   과거 vendor 본이 아닌 바이트는 사용자 수정이다 → 사용자 파일 유지 + vendor `.new` + 원장 user-kept.
        //   P0-4(동결 금지)는 과거 vendor 해시 표가 지킨다 — 옛 vendor 바이트는 갱신된다(m11_manifest_missing_* 가 고정).
        assert_eq!(read(sys_c), "SYS-DRIFT", "③system 매니페스트부재·상이(과거 vendor 아님) → 사용자 파일 유지");
        assert_eq!(read(&format!("{sys_c}.new")), sys_c_c, "③vendor 신버전은 .new 보관");
        assert_eq!(
            load_merge_pending(&td).get(sys_c).and_then(|e| e["kind"].as_str()),
            Some("user-kept"),
            "③원장 user-kept"
        );
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
        // ★0.15.0 M11 자(이 한 행만 재작성 — 종전: "③system 수정본은 .user 보존 후 치유"): `--force` 는 사용자
        //   수정을 버리지 않는다(설계 §4-11 자 · 판독 가능한 system 수정본은 user-kept). 위 판독 불가 행(A-2 #3·
        //   L1)과 ①②④ 는 무수정이다.
        assert_eq!(decide_file_action("alerts-config.json", embed, true, Some("SYS-DRIFT"), None, true),
                   UserKept { reason: "no-base" }, "③system 수정본 + force → 사용자 파일 유지(자)");

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
        // ★0.15.0 M11 자(이 두 행만 재작성 — 종전: "③system 수정본 → 임베드 치유" · "치유 전 사용자본 .user 보존"):
        //   `--force` 는 판독 가능한 사용자 수정을 버리지 않는다(설계 §4-11 자). README.md 는 vendor 미전진이라
        //   kept-drift 로 제자리 보존된다. 판독 불가 system(③ 아래 행 · ③-b)은 가-2 지금 동작 그대로다.
        assert_eq!(std::fs::read_to_string(pd.join(readable_sys)).unwrap(), "SYS-EDIT-XYZ",
                   "③system 수정본 + force → 버리지 않는다(자 · vendor 미전진 = kept-drift)");
        assert!(!pd.join(format!("{readable_sys}.user")).exists(),
                "③덮지 않았으므로 .user 도 없다");
        assert_eq!(load_merge_pending(&pd).get(readable_sys).and_then(|e| e["kind"].as_str()),
                   Some("kept-drift"), "③원장 kept-drift");
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
        // dept 에서 soul 외 파일은 base 등급표 그대로 — ★0.15.0 M11(H-RW 재작성 — 종전 기대값 "system 강제
        //   치유"): 그 등급표의 system 수정본 판정은 이제 user-kept(가). 스코프가 판정을 흔들지 않는다는 핀은
        //   Base 결과와의 동치로 그대로 남긴다.
        assert_eq!(
            super::decide_file_action("bin/x.py", embed, true, Some("HACKED"), None, false, d, None),
            super::decide_file_action("bin/x.py", embed, true, Some("HACKED"), None, false,
                PackScope::Base, None),
            "dept 스코프가 system 판정을 흔들면 안 된다"
        );
        assert_eq!(
            super::decide_file_action("bin/x.py", embed, true, Some("HACKED"), None, false, d, None),
            UserKept { reason: "no-base" },
            "dept·system 수정본 → 사용자 파일 유지(가)"
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
        // ★0.15.0 M11 가(H-RW 재작성 — 종전 L4 기대값 "Write{heal:true}"): L4 는 이제 사용자 우선(user-kept),
        //   기동 필수면 vendor 우선(vendor-first). 'Merge3 아님' 방어 핀은 그대로다.
        assert_eq!(super::decide_file_action("alerts-config.json", embed, true, Some("DRIFT"),
                       None, false, PackScope::Base, Some("B")),
                   UserKept { reason: "no-base" },
                   "★방어: manifest=None∧base=Some 계약위반 입력 → L4(Merge3 아님)");
        assert_eq!(super::decide_file_action("alerts-config.json", embed, true, Some("DRIFT"),
                       Some(e1h.as_str()), false, PackScope::Base, None),
                   UserKept { reason: "base-unverified" },
                   "★base=None: 벤더 전진+base 부재 → L4 사용자 우선");
        let bc = UpdateFacts { boot_critical: true, ..UpdateFacts::default() };
        assert_eq!(super::decide_file_action_with("alerts-config.json", embed, true, Some("DRIFT"),
                       Some(e1h.as_str()), false, PackScope::Base, None, &bc),
                   VendorFirst { reason: "base-unverified" },
                   "★나: 기동 필수 L4 → vendor 우선");
        // ★RULINGS BO1 재작성(종전: `Merge3` — "★나: 기동 필수도 검증 base 가 있으면 먼저 3-way(폴백이 vendor 우선)"):
        //   설치기는 기동 필수 파일에 병합 결과를 쓰지 않는다 — 검증 base 가 있어도 vendor 우선.
        assert_eq!(super::decide_file_action_with("alerts-config.json", embed, true, Some("DRIFT"),
                       Some(e1h.as_str()), false, PackScope::Base, Some("E1"), &bc),
                   VendorFirst { reason: "vendor-advanced" }, "★BO1: 기동 필수 + 검증 base → 병합 없이 vendor 우선");

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
        let journal_exists = pack_journal_dir().join(JOURNAL_SLOT).exists();
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
    ///
    /// REWRITTEN for M15-가 (0.15.0 「스스로 해결」 T2c · testplan H-RW-12 · design §6-15 가 "지금 시험은
    /// 바이트만 비교"): the pre-state now also holds a `#!` script at 0755 and a 0600 file, and the
    /// comparison covers bytes AND modes. Before the fix the rollback rewrote every journaled file with
    /// the plain writer, so the script came back 0644 (exec bit gone) and the 0600 file came back wider.
    /// The bytes-only assertions of the original test are all kept.
    #[test]
    fn mid_apply_fault_rolls_back_to_prestate() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate(
            "fault",
            &[
                ("README.md", "OLD-SOUL"),
                ("bin/tool.sh", "#!/bin/sh\necho old\n"),
                ("secret.txt", "OLD-SECRET"),
            ],
            "1.0.0",
        );
        #[cfg(unix)]
        {
            m15_set_mode(&pd.join("bin/tool.sh"), 0o755);
            m15_set_mode(&pd.join("secret.txt"), 0o600);
        }
        let pre_fp = fingerprint_dir(&pd);
        let pre_modes = fingerprint_dir_with_modes(&pd);

        // README.md 갱신(1번째 성공) → collide 파일(2번째 성공) → collide/child(3번째: 부모가
        // 파일이라 create_dir_all 실패) = mid-apply fault.
        let items: Vec<(&str, &str)> = vec![
            ("README.md", "NEW"),
            ("bin/tool.sh", "#!/bin/sh\necho new\n"),
            ("collide", "X"),
            ("collide/child", "Y"),
        ];
        let res = apply_pack_transactional(&items, "2.0.0", &test_free_state("2.0.0"), None, || Ok(()));

        let post_fp = fingerprint_dir(&pd);
        let post_modes = fingerprint_dir_with_modes(&pd);
        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let journal_exists = pack_journal_dir().join(JOURNAL_SLOT).exists();
        let _ = std::fs::remove_dir_all(&base);

        assert!(res.is_err(), "mid-apply fault인데 성공 반환");
        assert_eq!(pv.trim(), "1.0.0", ".pack-version 불변이어야(미커밋)");
        assert!(!journal_exists, "rollback 후 저널 잔존");
        assert_eq!(pre_fp, post_fp, "rollback이 pre-state로 복원 못함(부분적용 잔존)");
        #[cfg(unix)]
        {
            let m = |rel: &str| post_modes.get(rel).map(|v| v.1);
            assert_eq!(
                m("bin/tool.sh"),
                Some(0o755),
                "rollback stripped the exec bit of a #! script (M15-가)"
            );
            assert_eq!(
                m("secret.txt"),
                Some(0o600),
                "rollback widened a 0600 file (M15-가)"
            );
        }
        assert_eq!(
            pre_modes, post_modes,
            "rollback did not restore bytes AND modes of the pre-state (M15-가)"
        );
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
        let journal_exists = pack_journal_dir().join(JOURNAL_SLOT).exists();
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
    /// REWRITTEN for T9 stage 2 (RULINGS GX2/GX3, round 2 HE2): the hand-built journal gets this version's form — in the
    /// lane's slot under the name of a gone writer (`t9_hand_journal`) with an apply record; "not committed" = no commit
    /// marker carrying its apply id in the pack.
    /// Every original assertion is kept.
    #[test]
    fn orphan_journal_recovery_rolls_back() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        // crash 후 디스크: .pack-version 옛 1.0.0(미커밋) + README.md 부분반영 + new.txt 신규생성.
        let (base, pd, _env) = txn_prestate("orphan-rb", &[("README.md", "PARTIAL-NEW")], "1.0.0");
        std::fs::write(pd.join("new.txt"), "ORPHAN-NEW").unwrap();
        // 저널 수작업 조립: target 2.0.0, README.md(existed) backup=OLD-SOUL, new.txt(신규) existed=false.
        let jdir = t9_hand_journal();
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
        t9_add_apply_record(&jdir);

        let recovered = recover_pack_journal();

        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let new_exists = pd.join("new.txt").exists();
        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let journal_exists = pack_journal_dir().join(JOURNAL_SLOT).exists();
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(recovered.expect("recover 실패"), true, "orphan 미발견");
        assert_eq!(soul, "OLD-SOUL", "README.md rollback 안됨");
        assert!(!new_exists, "신규생성 new.txt 삭제 안됨");
        assert_eq!(pv.trim(), "1.0.0", ".pack-version 변경됨(미커밋인데)");
        assert!(!journal_exists, "recovery 후 저널 잔존");
    }

    /// orphan 저널 recovery: 디스크 .pack-version == 저널 target(커밋 성공·정리 중 crash)이면
    /// rollback 없이 저널만 삭제(커밋된 새 내용을 되돌리지 않는다).
    /// REWRITTEN for T9 stage 2 (RULINGS GX2): "committed" = the pack's commit marker `.pack-applied.json` carries the
    /// journal's apply id (was: `.pack-version == target_version`, which a same-base revision cannot tell — 다-2). The
    /// journal is in this version's form with a finished writer. Every original assertion is kept.
    #[test]
    fn orphan_journal_committed_only_cleaned() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        // 커밋 성공: .pack-version=2.0.0, README.md=NEW-SOUL(새 내용).
        let (base, pd, _env) = txn_prestate("orphan-commit", &[("README.md", "NEW-SOUL")], "2.0.0");
        let jdir = t9_hand_journal();
        let files_dir = jdir.join("files");
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::write(files_dir.join("README.md"), "OLD-SOUL").unwrap(); // 커밋 전 백업본
        let index = serde_json::json!({
            "target_version": "2.0.0",
            "entries": [{"rel": "README.md", "existed": true}]
        });
        std::fs::write(jdir.join("index.json"), index.to_string()).unwrap();
        t9_add_apply_record(&jdir);
        let marker = serde_json::json!({
            "apply_id": T9_TEST_APPLY,
            "target": {"channel": "free", "base_version": "2.0.0", "pro_revision": 0},
            "items_digest": ""
        });
        std::fs::write(pd.join(PACK_APPLIED_FILE), marker.to_string()).unwrap();

        let recovered = recover_pack_journal();

        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let journal_exists = pack_journal_dir().join(JOURNAL_SLOT).exists();
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(recovered.expect("recover 실패"), true, "orphan 미발견");
        assert_eq!(soul, "NEW-SOUL", "커밋된 내용을 잘못 rollback함");
        assert!(!journal_exists, "정리 후 저널 잔존");
    }

    // ── M15-가 (0.15.0 「스스로 해결」 T2c): the journal rollback restores file modes ──────────────────
    // Design §2-11 가 / §4-15 가 / §6-15 가, appendix A-2 #7; testplan I-F3-09, H-15ga-A2-07-01, H-RW-12.
    // Mutation that must turn these red: restore journaled files with the plain writer (mode `None`).

    /// Bytes AND permission mode per file (`fingerprint_dir` compares bytes only). Modes are Unix-only;
    /// elsewhere the mode column is 0, so the comparison stays a byte comparison there.
    fn fingerprint_dir_with_modes(
        root: &Path,
    ) -> std::collections::BTreeMap<String, (String, u32)> {
        fingerprint_dir(root)
            .into_iter()
            .map(|(rel, h)| {
                let m = current_mode(&root.join(&rel)).unwrap_or(0);
                (rel, (h, m))
            })
            .collect()
    }

    #[cfg(unix)]
    fn m15_set_mode(p: &Path, m: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
    }

    /// I-F3-09 arm 1 / H-15ga-A2-07-01: `write_journal` records each file's mode and `rollback_journal`
    /// restores it exactly — a `#!` script at 0755, an owner-only script at 0700, a 0600 file and a
    /// plain 0644 file. The partial apply is simulated the way the installer writes (tmp + rename =
    /// default mode). On v0.14.56 code every one of them came back with the default mode (0644 under
    /// umask 022): exec bits gone, 0600 widened.
    #[cfg(unix)]
    #[test]
    fn journal_rollback_restores_recorded_modes() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let files: [(&str, &str, u32); 4] = [
            ("bin/tool.sh", "#!/bin/sh\necho old\n", 0o755),
            ("bin/private.sh", "#!/bin/sh\necho private\n", 0o700),
            ("secret.txt", "OLD-SECRET", 0o600),
            ("README.md", "OLD-SOUL", 0o644),
        ];
        let seed: Vec<(&str, &str)> = files.iter().map(|(r, c, _)| (*r, *c)).collect();
        let (base, pd, _env) = txn_prestate("m15-modes", &seed, "1.0.0");
        for (rel, _, m) in &files {
            m15_set_mode(&pd.join(rel), *m);
        }
        let pre = fingerprint_dir_with_modes(&pd);
        let set: std::collections::BTreeSet<String> =
            files.iter().map(|(r, _, _)| (*r).to_string()).collect();
        let jd = t9_write_journal("2.0.0", &set).expect("write_journal");
        for (rel, _, _) in &files {
            write_atomic(&pd.join(rel), b"PARTIAL-APPLY").unwrap();
        }
        let rolled = rollback_and_settle_own(&jd);
        let post = fingerprint_dir_with_modes(&pd);
        let journal_exists = pack_journal_dir().join(JOURNAL_SLOT).exists();
        let _ = std::fs::remove_dir_all(&base);

        rolled.expect("rollback_journal");
        for (rel, _, m) in &files {
            let got = post.get(*rel).map(|v| v.1);
            let shown = got.map(|g| format!("{g:04o}"));
            assert_eq!(
                got,
                Some(*m),
                "{rel}: mode after rollback {shown:?}, pre-state {m:04o}"
            );
        }
        assert_eq!(pre, post, "rollback did not restore bytes AND modes");
        assert!(!journal_exists, "journal left behind after rollback");
    }

    /// RULINGS AE2: a recorded mode WITHOUT an exec bit is restored as it was, even on a `#!` file — the
    /// recorded mode wins over the shebang rule. The partial apply leaves the file at 0755 (as the
    /// installer's exec loop would), so a restore that adds an exec bit, or keeps the current exec mode,
    /// fails here.
    #[cfg(unix)]
    #[test]
    fn journal_rollback_keeps_a_recorded_non_exec_mode_on_a_shebang_file() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate(
            "m15-noexec",
            &[("bin/plain.sh", "#!/bin/sh\necho old\n")],
            "1.0.0",
        );
        m15_set_mode(&pd.join("bin/plain.sh"), 0o644);
        let set: std::collections::BTreeSet<String> =
            ["bin/plain.sh"].iter().map(|s| (*s).to_string()).collect();
        let jd = t9_write_journal("2.0.0", &set).expect("write_journal");
        write_atomic(&pd.join("bin/plain.sh"), b"#!/bin/sh\necho partial\n").unwrap();
        m15_set_mode(&pd.join("bin/plain.sh"), 0o755);
        let rolled = rollback_and_settle_own(&jd);
        let bytes = std::fs::read_to_string(pd.join("bin/plain.sh")).unwrap();
        let mode = current_mode(&pd.join("bin/plain.sh"));
        let _ = std::fs::remove_dir_all(&base);

        rolled.expect("rollback_journal");
        assert_eq!(bytes, "#!/bin/sh\necho old\n");
        assert_eq!(
            mode,
            Some(0o644),
            "a #! file recorded at 0644 must come back 0644 (the recorded mode wins)"
        );
    }

    /// I-F3-09 arm 2 (in-process form of the boot-time path): an orphan journal written by
    /// `write_journal` is rolled back by `recover_pack_journal` (no commit marker of its apply — T9 GX2; its writer
    /// marked finished — GX3) and
    /// `bin/cys-dept` keeps its exec bit — the `[ -x "$pk/bin/cys-dept" ] || exit 0` guards of the
    /// built-in `formation-heartbeat` / `ceo-promote-pending-tick` jobs (`schedule.rs` 407, 429) would
    /// otherwise stop silently (design §2-11 가, "주기 자가치유 전멸").
    #[cfg(unix)]
    #[test]
    fn orphan_journal_recovery_restores_modes() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate(
            "m15-orphan",
            &[
                ("bin/cys-dept", "#!/bin/bash\necho old\n"),
                ("README.md", "OLD-SOUL"),
            ],
            "1.0.0",
        );
        m15_set_mode(&pd.join("bin/cys-dept"), 0o755);
        let pre = fingerprint_dir_with_modes(&pd);
        let set: std::collections::BTreeSet<String> = ["bin/cys-dept", "README.md", "new.txt"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let jd = t9_write_journal("2.0.0", &set).expect("write_journal");
        // crash mid-apply: both files rewritten 0644, one new file created, marker still 1.0.0
        write_atomic(&pd.join("bin/cys-dept"), b"#!/bin/bash\necho partial\n").unwrap();
        write_atomic(&pd.join("README.md"), b"PARTIAL").unwrap();
        write_atomic(&pd.join("new.txt"), b"ORPHAN-NEW").unwrap();
        let _ = &jd;
        t9_writers_gone();

        let recovered = recover_pack_journal();
        let post = fingerprint_dir_with_modes(&pd);
        let journal_exists = pack_journal_dir().join(JOURNAL_SLOT).exists();
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(
            recovered.expect("recover_pack_journal"),
            true,
            "orphan journal not found"
        );
        let m = post.get("bin/cys-dept").map(|v| v.1).unwrap_or(0);
        assert!(
            m & 0o111 != 0,
            "bin/cys-dept lost its exec bit in the boot-time recovery ({m:04o})"
        );
        assert_eq!(
            pre, post,
            "recovery did not restore bytes AND modes (new.txt must be gone too)"
        );
        assert!(!journal_exists, "journal left behind after recovery");
    }

    /// The index carries `mode` as an integer for files that existed, and no `mode` key for entries
    /// that did not exist (nothing to restore). A path that is a directory is `existed: false` as before.
    #[cfg(unix)]
    #[test]
    fn journal_index_records_mode_only_for_existing_files() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate("m15-index", &[("bin/run.sh", "#!/bin/sh\n")], "1.0.0");
        m15_set_mode(&pd.join("bin/run.sh"), 0o750);
        let set: std::collections::BTreeSet<String> = ["bin/run.sh", "absent.txt"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let jd = t9_write_journal("2.0.0", &set).expect("write_journal");
        let raw = std::fs::read_to_string(jd.join("index.json")).unwrap();
        let _ = std::fs::remove_dir_all(&base);

        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let entries = v["entries"].as_array().expect("entries");
        let by_rel = |rel: &str| {
            entries
                .iter()
                .find(|e| e["rel"] == rel)
                .cloned()
                .expect(rel)
        };
        let run = by_rel("bin/run.sh");
        assert_eq!(run["existed"], true);
        assert_eq!(
            run["mode"].as_u64(),
            Some(0o750),
            "mode not recorded as an integer: {raw}"
        );
        let absent = by_rel("absent.txt");
        assert_eq!(absent["existed"], false);
        assert!(
            absent.get("mode").is_none(),
            "a non-existing entry must carry no mode: {raw}"
        );
    }

    /// Upgrade direction: a journal written by an older binary has no `mode`. It still parses, and the
    /// restore falls back to the installer's single mode rule (`pack_target_mode`): a `#!` file whose
    /// live copy lost the exec bit gets 0755 again, a non-`#!` file keeps its current mode (a 0600
    /// stays 0600). On v0.14.56 code both came back with the default mode.
    #[cfg(unix)]
    #[test]
    fn legacy_journal_without_mode_restores_exec_bit_by_the_shebang_rule() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) = txn_prestate(
            "m15-legacy",
            &[
                ("bin/tool.sh", "#!/bin/sh\necho partial\n"),
                ("secret.txt", "PARTIAL"),
            ],
            "1.0.0",
        );
        m15_set_mode(&pd.join("bin/tool.sh"), 0o644); // the partial apply wrote it without the bit
        m15_set_mode(&pd.join("secret.txt"), 0o600);
        let jdir = t9_hand_journal();
        let files_dir = jdir.join("files");
        std::fs::create_dir_all(files_dir.join("bin")).unwrap();
        std::fs::write(files_dir.join("bin/tool.sh"), "#!/bin/sh\necho old\n").unwrap();
        std::fs::write(files_dir.join("secret.txt"), "OLD-SECRET").unwrap();
        std::fs::write(files_dir.join("gone.txt"), "GONE-BEFORE").unwrap();
        let index = serde_json::json!({
            "target_version": "2.0.0",
            "entries": [
                {"rel": "bin/tool.sh", "existed": true},
                {"rel": "secret.txt", "existed": true},
                {"rel": "gone.txt", "existed": true}
            ]
        });
        std::fs::write(jdir.join("index.json"), index.to_string()).unwrap();
        // T9 stage 2: entries without `mode` now come only from Windows (a journal of an older binary lives at the old
        // shared location and is recovered by nobody — GX4); the restore rule for them is unchanged. Written by a gone
        // process (its name — RULINGS HE2).
        t9_add_apply_record(&jdir);

        let recovered = recover_pack_journal();
        let tool = std::fs::read_to_string(pd.join("bin/tool.sh")).unwrap();
        let tool_mode = current_mode(&pd.join("bin/tool.sh"));
        let secret = std::fs::read_to_string(pd.join("secret.txt")).unwrap();
        let secret_mode = current_mode(&pd.join("secret.txt"));
        let gone = std::fs::read_to_string(pd.join("gone.txt")).ok();
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(recovered.expect("recover_pack_journal"), true);
        assert_eq!(tool, "#!/bin/sh\necho old\n");
        assert_eq!(
            tool_mode,
            Some(0o755),
            "legacy entry: #! file must get its exec bit back"
        );
        assert_eq!(secret, "OLD-SECRET");
        assert_eq!(
            secret_mode,
            Some(0o600),
            "legacy entry: a non-#! file keeps its current mode"
        );
        assert_eq!(
            gone.as_deref(),
            Some("GONE-BEFORE"),
            "legacy entry whose live copy vanished"
        );
    }

    /// Downgrade direction: an index written by this binary (with `mode`) parses with the struct shape
    /// of v0.14.56 (`{rel, existed}` / `{target_version, entries}`, no `deny_unknown_fields`) — an
    /// older binary that recovers it ignores `mode` and restores bytes exactly as it always did.
    #[cfg(unix)]
    #[test]
    fn new_journal_index_parses_with_the_v0_14_56_struct_shape() {
        #[derive(serde::Deserialize)]
        struct OldEntry {
            rel: String,
            existed: bool,
        }
        #[derive(serde::Deserialize)]
        struct OldIndex {
            target_version: String,
            entries: Vec<OldEntry>,
        }
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let (base, pd, _env) =
            txn_prestate("m15-downgrade", &[("bin/tool.sh", "#!/bin/sh\n")], "1.0.0");
        m15_set_mode(&pd.join("bin/tool.sh"), 0o755);
        let set: std::collections::BTreeSet<String> = ["bin/tool.sh", "absent.txt"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let jd = t9_write_journal("2.0.0", &set).expect("write_journal");
        let raw = std::fs::read_to_string(jd.join("index.json")).unwrap();
        let _ = std::fs::remove_dir_all(&base);

        assert!(
            raw.contains("\"mode\""),
            "the new index must carry the mode field: {raw}"
        );
        let old: OldIndex =
            serde_json::from_str(&raw).expect("v0.14.56 shape must still parse the new index");
        assert_eq!(old.target_version, "2.0.0");
        let got: Vec<(String, bool)> = old
            .entries
            .into_iter()
            .map(|e| (e.rel, e.existed))
            .collect();
        assert_eq!(
            got,
            vec![
                ("absent.txt".to_string(), false),
                ("bin/tool.sh".to_string(), true)
            ]
        );
    }

    // ── M15-다 / M15-다-2 reproduction (0.15.0 「스스로 해결」 T9 stage 1 · ticket T9r) ─────────────────────
    // Design §2-11 다 / 다-2, §4-15 다, §6-15 다; appendix A-1 #3, A-2 #13, #14. Plan: impl/plans/T9.md.
    // Every test below asserts the CORRECT end state. They FAILED on the tip before the fix (stage 1, b729efac,
    // rebased 9d34f0bf — runs/T9-repro-b729efac.out); T9 stage 2 (RULINGS GX1–GX5) makes them pass and un-ignores
    // them. Stage-2 rewrite: the crash model marks the captured journal's writer as a finished process (GX3).
    // Synthetic lanes under a temporary root: `<root>/pack` (base) and `<root>/pack-dept-d1` (department) —
    // the same `<root>/pack` layout as `txn_prestate`, so both lanes' journal is `<root>/.pack-journal`
    // exactly as both live lanes' journal is `~/.cys/.pack-journal`. A daemon's boot recovery is the exact
    // library call cysd makes before it listens (`src/bin/cysd/main.rs`
    // `spawn_blocking(cys::pack::recover_pack_journal)`, no lock) with `CYS_PACK_DIR` naming that lane.
    // Interleavings are controlled, never timed: (a) the real `apply_pack_transactional`'s `post_commit`
    // seam (between ④ `.pack-version` and ⑥ journal deletion), or (b) step-wise: the on-disk state a crash
    // in step ② leaves, built from the REAL journal (captured from a twin run of the real transaction) and
    // the installer's own per-file calls.

    type M15daState = std::collections::BTreeMap<String, (String, u32)>;

    /// Every file under `root` → (text, mode). Mode is 0 off Unix.
    fn m15da_state(root: &Path) -> M15daState {
        fn walk(base: &Path, dir: &Path, out: &mut M15daState) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(base, &p, out);
                    } else if let Ok(bytes) = std::fs::read(&p) {
                        let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                        let mode = current_mode(&p).unwrap_or(0);
                        out.insert(rel, (String::from_utf8_lossy(&bytes).into_owned(), mode));
                    }
                }
            }
        }
        let mut out = M15daState::new();
        walk(root, root, &mut out);
        out
    }

    fn m15da_short(s: &str) -> String {
        let one = s.replace('\n', "⏎");
        if one.chars().count() > 72 {
            format!("{}…", one.chars().take(72).collect::<String>())
        } else {
            one
        }
    }

    /// One line per file that differs between two states of the same lane.
    fn m15da_diff(before: &M15daState, after: &M15daState) -> Vec<String> {
        let mut out = Vec::new();
        for (rel, (c0, m0)) in before {
            match after.get(rel) {
                None => out.push(format!(
                    "DELETED {rel} (was {:?} {m0:04o})",
                    m15da_short(c0)
                )),
                Some((c1, m1)) if c1 != c0 => out.push(format!(
                    "CHANGED {rel}: {:?} {m0:04o} -> {:?} {m1:04o}",
                    m15da_short(c0),
                    m15da_short(c1)
                )),
                Some((_, m1)) if m1 != m0 => {
                    out.push(format!("MODE    {rel}: {m0:04o} -> {m1:04o}"))
                }
                _ => {}
            }
        }
        for (rel, (c1, m1)) in after {
            if !before.contains_key(rel) {
                out.push(format!("ADDED   {rel}: {:?} {m1:04o}", m15da_short(c1)));
            }
        }
        out
    }

    fn m15da_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("cys-m15da-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A synthetic lane `<root>/<name>` as an install leaves it: `.pack-version`, the files (`#!` files
    /// 0755 on Unix), `.install-manifest.json` = their hashes, optional `.pack-state.json`, plus `extra`
    /// files that are NOT in the manifest (sidecars such as `<rel>.user`, `.merge-pending.json`).
    fn m15da_lane(
        root: &Path,
        name: &str,
        version: &str,
        files: &[(&str, &str)],
        extra: &[(&str, &str)],
        state: Option<PackState>,
    ) -> PathBuf {
        let pd = root.join(name);
        std::fs::create_dir_all(&pd).unwrap();
        std::fs::write(pd.join(PACK_VERSION_FILE), version).unwrap();
        let mut manifest = serde_json::Map::new();
        for (rel, content) in files.iter().chain(extra.iter()) {
            let p = pd.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
            #[cfg(unix)]
            m15_set_mode(
                &p,
                if content.starts_with("#!") {
                    0o755
                } else {
                    0o644
                },
            );
        }
        for (rel, content) in files {
            manifest.insert((*rel).to_string(), serde_json::json!(content_hash(content)));
        }
        std::fs::write(
            pd.join(INSTALL_MANIFEST),
            serde_json::Value::Object(manifest).to_string(),
        )
        .unwrap();
        if let Some(st) = state {
            write_pack_state(&pd, &st).unwrap();
        }
        pd
    }

    /// The base lane at 0.15.0. Vendor files (`bin/`, README) plus the lane's own user-owned and
    /// seed-once files (directive, soul, memory, schedule — `ownership()`), with the base's content.
    const M15DA_BASE: &[(&str, &str)] = &[
        ("bin/selfheal.sh", "#!/bin/sh\necho selfheal-v1\n"),
        ("README.md", "README v1\n"),
        (
            "directives/MASTER_DIRECTIVE.md",
            "BASE lane: CEO directive\n",
        ),
        ("soul.md", "BASE lane: soul (the owner's constitution)\n"),
        ("memory/MEMORY.md", "BASE lane: memory\n"),
        ("schedule.json", "{\"jobs\":[\"base-only-job\"]}\n"),
    ];
    /// The department lane d1 at the same vendor content, with ITS OWN user-owned / seed-once files.
    const M15DA_DEPT: &[(&str, &str)] = &[
        ("bin/selfheal.sh", "#!/bin/sh\necho selfheal-v1\n"),
        ("README.md", "README v1\n"),
        (
            "directives/MASTER_DIRECTIVE.md",
            "DEPT d1: department master directive\n",
        ),
        (
            "soul.md",
            "DEPT d1: soul (seeded from the base, then edited in the department)\n",
        ),
        ("memory/MEMORY.md", "DEPT d1: memory\n"),
        ("schedule.json", "{\"jobs\":[\"dept-d1-job\"]}\n"),
    ];
    /// Sidecars the booting lane holds that the updating lane does not: the user's kept copy and the
    /// merge ledger entry that points at it.
    const M15DA_SIDECARS: &[(&str, &str)] = &[
        ("README.md.user", "the user's own README, kept beside the vendor copy\n"),
        (
            MERGE_PENDING_FILE,
            "{\"README.md\":{\"kind\":\"user-kept\",\"side\":\"README.md.user\",\"version\":\"0.15.0\",\"ts\":1}}\n",
        ),
    ];

    /// The update's items for a lane: two vendor files change, one new script appears; the lane's own
    /// user-owned / seed-once files are in the bundle with unchanged bytes (the installer leaves them).
    /// Order = the installer's loop order (`install_into_with` iterates `items` as given).
    fn m15da_v2_items(lane: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut items: Vec<(&'static str, &'static str)> = vec![
            ("bin/selfheal.sh", "#!/bin/sh\necho selfheal-v2\n"),
            ("README.md", "README v2\n"),
            ("bin/new-in-v2.sh", "#!/bin/sh\necho new-in-v2\n"),
        ];
        for (rel, c) in lane {
            if !items.iter().any(|(r, _)| r == rel) {
                items.push((*rel, *c));
            }
        }
        items
    }

    struct M15daCross {
        root: PathBuf,
        journal_of_updater: PathBuf,
        journal_of_booter: PathBuf,
        apply: Result<(usize, usize, bool), String>,
        /// (journal index present when the booter started, its recovery result, journal still there after)
        boot: Option<(bool, Result<bool, String>, bool)>,
        booter_before: M15daState,
        booter_after: M15daState,
        updater_after: M15daState,
        booter_current_after: bool,
    }

    /// Lane `updater` runs the real pack-update transaction 0.15.0 → 0.15.1; inside its `post_commit`
    /// seam (after ④, before ⑥) the daemon of lane `booter` starts and runs its boot recovery.
    fn m15da_cross_lane_during_apply(
        tag: &str,
        updater: (&str, &'static [(&'static str, &'static str)]),
        booter: (&str, &'static [(&'static str, &'static str)]),
    ) -> M15daCross {
        let root = m15da_root(tag);
        let up = m15da_lane(&root, updater.0, "0.15.0", updater.1, &[], None);
        let bt = m15da_lane(&root, booter.0, "0.15.0", booter.1, M15DA_SIDECARS, None);
        let _env = set_pack_env(&up, root.join("cfg"));
        let journal_of_updater = pack_journal_dir();
        let journal_of_booter = {
            let _e = EnvGuard::set(ENV_PACK_DIR, &bt);
            pack_journal_dir()
        };
        let booter_before = m15da_state(&bt);
        let items = m15da_v2_items(updater.1);
        let boot = std::cell::RefCell::new(None);
        let apply =
            apply_pack_transactional(&items, "0.15.1", &test_free_state("0.15.1"), None, || {
                // The other lane's daemon starts now: cysd main.rs `recover_pack_journal`, no lock.
                let _e = EnvGuard::set(ENV_PACK_DIR, &bt);
                // The UPDATING lane's journal (stage 2: measured at its own path — with one shared folder in
                // stage 1 the booter's `pack_journal_dir()` was the same path).
                let present = t9_slot_has_index(&journal_of_updater);
                let rec = recover_pack_journal();
                let left = t9_slot_has_index(&journal_of_updater);
                *boot.borrow_mut() = Some((present, rec, left));
                Ok(())
            });
        M15daCross {
            booter_after: m15da_state(&bt),
            updater_after: m15da_state(&up),
            booter_current_after: pack_current_in(&bt, "0.15.0"),
            root,
            journal_of_updater,
            journal_of_booter,
            apply,
            boot: boot.into_inner(),
            booter_before,
        }
    }

    fn m15da_assert_cross(r: M15daCross, booter_name: &str) {
        let _ = std::fs::remove_dir_all(&r.root);
        // Preconditions (pass on the tip): the updater's own transaction completed, the booter's
        // recovery actually ran inside the seam.
        let (present, rec, left) = r
            .boot
            .expect("precondition: the post_commit seam did not run");
        r.apply
            .as_ref()
            .expect("precondition: the updating lane's transaction must succeed");
        assert_eq!(
            r.updater_after.get(".pack-version").map(|v| v.0.as_str()),
            Some("0.15.1"),
            "precondition: the updating lane must end at 0.15.1"
        );
        let diff = m15da_diff(&r.booter_before, &r.booter_after);
        let evidence = format!(
            "journal path, updating lane = {}\njournal path, booting lane  = {}\n\
             updating lane's journal index present when the {booter_name} daemon booted: {present}\n\
             {booter_name} boot recovery returned: {rec:?}\n\
             updating lane's journal index still present after that boot recovery: {left}\n\
             {booter_name} .pack-version after: {:?} · pack_current_in({booter_name}, \"0.15.0\") after = {} \
             (true = the next boot does NOT reinstall: the state stays)\n\
             {booter_name} pack, {} file(s) changed by ANOTHER lane's journal:\n  {}",
            r.journal_of_updater.display(),
            r.journal_of_booter.display(),
            r.booter_after.get(".pack-version").map(|v| v.0.as_str()),
            r.booter_current_after,
            diff.len(),
            diff.join("\n  ")
        );
        assert!(
            left,
            "M15-다: the {booter_name} daemon's boot recovery removed the updating lane's in-flight journal\n{evidence}"
        );
        assert!(
            diff.is_empty(),
            "M15-다: the {booter_name} daemon's boot recovery wrote another lane's pre-state into its own pack\n{evidence}"
        );
    }

    /// M15-다, base → department: the base lane's pack-update is in flight (journal written, commit marker
    /// written, journal not yet deleted) when a department daemon starts. Correct: the department pack
    /// is untouched and the base's journal is not taken.
    #[test]
    fn m15_da_dept_boot_recovery_during_base_update_writes_base_bytes_into_dept_pack() {
        // A failing (ignored-by-design) test must not poison the lock for the next one.
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let r = m15da_cross_lane_during_apply(
            "fwd",
            ("pack", M15DA_BASE),
            ("pack-dept-d1", M15DA_DEPT),
        );
        m15da_assert_cross(r, "department");
    }

    /// M15-다, the reverse: a `cys pack-update` run in a department seat (env `CYS_PACK_DIR` = the
    /// department pack; nothing refuses it) is in flight when the BASE daemon starts.
    #[test]
    fn m15_da_base_boot_recovery_during_dept_update_writes_dept_bytes_into_base_pack() {
        // A failing (ignored-by-design) test must not poison the lock for the next one.
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let r = m15da_cross_lane_during_apply(
            "rev",
            ("pack-dept-d1", M15DA_DEPT),
            ("pack", M15DA_BASE),
        );
        m15da_assert_cross(r, "base");
    }

    /// Builds lane `<root>/<name>` and leaves it in the on-disk state a crash of its pack-update leaves
    /// when the process dies in step ② right after the installer rewrote the first `k` changed items:
    /// 1. the lane is built and the REAL `apply_pack_transactional` runs on it; inside its `post_commit`
    ///    seam (after ④, before ⑥) the lane's REAL journal is copied out;
    /// 2. the lane is rebuilt from scratch with the same deterministic builder (= the exact pre-state the
    ///    journal recorded) and the captured journal is put back at the lane's `pack_journal_dir()`;
    /// 3. the first `k` changed items are written with the installer's own calls (`write_atomic` then
    ///    `ensure_pristine` — `install_into_with`), which leave the default file mode (exec bits are set
    ///    only after the loop); the install manifest (written after the loop), `.pack-state.json` (③) and
    ///    `.pack-version` (④) stay as they were.
    /// Returns (lane path, the end state of the complete real apply of step 1). Preconditions checked:
    /// the real apply wrote exactly these bytes and their `.pristine/` mirror (the plain write path).
    fn m15da_crash_in_step_2(
        root: &Path,
        name: &str,
        lane: (&str, &[(&str, &str)], &[(&str, &str)], Option<PackState>),
        items: &[(&str, &str)],
        target: &str,
        new_state: &PackState,
        k: usize,
    ) -> (PathBuf, M15daState) {
        let (version, files, extra, state) = lane;
        let pd = m15da_lane(root, name, version, files, extra, state.clone());
        let _e = EnvGuard::set(ENV_PACK_DIR, &pd);
        let journal = pack_journal_dir();
        let captured = root.join("captured-journal");
        let _ = std::fs::remove_dir_all(&captured);
        apply_pack_transactional(items, target, new_state, None, || {
            copy_dir_all(&journal, &captured).map_err(|e| e.to_string())
        })
        .expect("precondition: the real transaction must succeed");
        let complete = m15da_state(&pd);
        assert!(
            t9_slot_has_index(&captured),
            "precondition: the real journal was not captured"
        );
        std::fs::remove_dir_all(&pd).unwrap();
        assert_eq!(m15da_lane(root, name, version, files, extra, state), pd);
        let before = m15da_state(&pd);
        // the slot only: the lane folder itself persists (and `copy_dir_all` creates its destination new)
        copy_dir_all(&captured.join(JOURNAL_SLOT), &journal.join(JOURNAL_SLOT)).unwrap();
        // T9 stage 2 (GX3 / HE2): the writer of a crashed journal is gone; the in-process stop leaves this process alive —
        // its name is relabelled to a gone identity (what a dead writer of that pid leaves).
        t9_writers_gone();
        let _ = std::fs::remove_dir_all(&captured);
        let changed: Vec<(&str, &str)> = items
            .iter()
            .copied()
            .filter(|(rel, c)| before.get(*rel).map(|v| v.0.as_str()) != Some(*c))
            .take(k)
            .collect();
        assert_eq!(
            changed.len(),
            k,
            "precondition: fewer than {k} changed items"
        );
        for (rel, content) in &changed {
            let mirror = format!("{PRISTINE_DIR}/{rel}");
            assert!(
                complete.get(*rel).map(|v| v.0.as_str()) == Some(*content)
                    && complete.get(mirror.as_str()).map(|v| v.0.as_str()) == Some(*content),
                "precondition: the real apply did not write {rel} (+ {mirror}) as the crash model does"
            );
            let path = pd.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            write_atomic(&path, content.as_bytes()).unwrap();
            ensure_pristine(&pd, rel, content);
        }
        (pd, complete)
    }

    /// M15-다, crash variant: the base's pack-update 0.15.0 → 0.15.1 dies in step ② (power loss, the
    /// app killed with its sidecar). The orphan journal stays until a daemon boots. A department daemon
    /// (at 0.15.0) boots FIRST, then the base daemon. Correct: the department pack is untouched and the
    /// base pack is rolled back to its exact pre-state.
    /// Control arm (must pass on the tip): the same crash state with ONLY the base daemon booting is
    /// rolled back exactly — so any difference in the main arm is caused by the department's boot.
    #[test]
    fn m15_da_dept_boot_recovery_takes_base_crash_journal_and_base_stays_half_applied() {
        // A failing (ignored-by-design) test must not poison the lock for the next one.
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let items = m15da_v2_items(M15DA_BASE);
        let st = test_free_state("0.15.1");

        // Control arm: base only.
        let croot = m15da_root("crash-ctl");
        let _cenv = set_pack_env(croot.join("pack"), croot.join("cfg"));
        let lane = ("0.15.0", M15DA_BASE, &[][..], None);
        let (cbase, _) =
            m15da_crash_in_step_2(&croot, "pack", lane.clone(), &items, "0.15.1", &st, 1);
        let ccrash = m15da_state(&cbase);
        let cpre = {
            let pre_root = croot.join("pre");
            m15da_state(&m15da_lane(
                &pre_root,
                "pack",
                "0.15.0",
                M15DA_BASE,
                &[],
                None,
            ))
        };
        let crec = recover_pack_journal();
        let cdiff = m15da_diff(&cpre, &m15da_state(&cbase));
        drop(_cenv);
        let _ = std::fs::remove_dir_all(&croot);
        assert!(
            !m15da_diff(&cpre, &ccrash).is_empty(),
            "precondition: the crash model changed nothing"
        );
        assert!(
            cdiff.is_empty() && crec == Ok(true),
            "precondition (control arm): the base's own recovery must restore its pre-state on the tip; \
             recover={crec:?} diff={cdiff:?}"
        );

        // Main arm: the department daemon boots first.
        let root = m15da_root("crash-dept-first");
        let dept = m15da_lane(
            &root,
            "pack-dept-d1",
            "0.15.0",
            M15DA_DEPT,
            M15DA_SIDECARS,
            None,
        );
        let _env = set_pack_env(root.join("pack"), root.join("cfg"));
        let base_pre = m15da_state(&m15da_lane(
            &root.join("pre"),
            "pack",
            "0.15.0",
            M15DA_BASE,
            &[],
            None,
        ));
        let dept_pre = m15da_state(&dept);
        let (base, _) = m15da_crash_in_step_2(&root, "pack", lane, &items, "0.15.1", &st, 1);
        let base_crash = m15da_state(&base);
        let dept_rec = {
            let _e = EnvGuard::set(ENV_PACK_DIR, &dept);
            recover_pack_journal()
        };
        let journal_after_dept = t9_slot_has_index(&pack_journal_dir());
        let base_rec = recover_pack_journal();
        let base_end = m15da_state(&base);
        let dept_end = m15da_state(&dept);
        let base_current = pack_current_in(&base, "0.15.0");
        drop(_env);
        let _ = std::fs::remove_dir_all(&root);

        let dept_diff = m15da_diff(&dept_pre, &dept_end);
        let base_diff = m15da_diff(&base_pre, &base_end);
        let evidence = format!(
            "base after the crash (vs pre-state):\n  {}\n\
             department boot recovery returned {dept_rec:?}; journal still there after it: {journal_after_dept}\n\
             base boot recovery returned {base_rec:?}\n\
             department pack, {} file(s) changed by the base's journal:\n  {}\n\
             base pack vs its pre-state after both boots, {} difference(s):\n  {}\n\
             pack_current_in(base, \"0.15.0\") = {base_current} (true = no reinstall at the next boot)",
            m15da_diff(&base_pre, &base_crash).join("\n  "),
            dept_diff.len(),
            dept_diff.join("\n  "),
            base_diff.len(),
            base_diff.join("\n  ")
        );
        assert!(
            dept_diff.is_empty() && base_diff.is_empty(),
            "M15-다 (crash variant): wrong end state\n{evidence}"
        );
    }

    /// M15-다, the department is already at the base's TARGET version (e.g. its daemon was restarted
    /// on the newer binary and installed that version) — its boot recovery judges the base's orphan
    /// journal "committed" by ITS OWN `.pack-version` and deletes it without rolling anything back.
    /// Correct: the base pack is rolled back to its pre-state at the base's boot.
    #[test]
    fn m15_da_dept_at_target_version_deletes_base_crash_journal() {
        // A failing (ignored-by-design) test must not poison the lock for the next one.
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let items = m15da_v2_items(M15DA_BASE);
        let st = test_free_state("0.15.1");
        let root = m15da_root("crash-dept-at-target");
        let dept = m15da_lane(
            &root,
            "pack-dept-d1",
            "0.15.1",
            M15DA_DEPT,
            M15DA_SIDECARS,
            None,
        );
        let _env = set_pack_env(root.join("pack"), root.join("cfg"));
        let base_pre = m15da_state(&m15da_lane(
            &root.join("pre"),
            "pack",
            "0.15.0",
            M15DA_BASE,
            &[],
            None,
        ));
        let dept_pre = m15da_state(&dept);
        let lane = ("0.15.0", M15DA_BASE, &[][..], None);
        let (base, _) = m15da_crash_in_step_2(&root, "pack", lane, &items, "0.15.1", &st, 1);
        let dept_rec = {
            let _e = EnvGuard::set(ENV_PACK_DIR, &dept);
            recover_pack_journal()
        };
        let journal_after_dept = t9_slot_has_index(&pack_journal_dir());
        let base_rec = recover_pack_journal();
        let base_end = m15da_state(&base);
        let dept_end = m15da_state(&dept);
        drop(_env);
        let _ = std::fs::remove_dir_all(&root);

        let dept_diff = m15da_diff(&dept_pre, &dept_end);
        let base_diff = m15da_diff(&base_pre, &base_end);
        let evidence = format!(
            "department (.pack-version 0.15.1 = the base's target) boot recovery returned {dept_rec:?}; \
             journal still there after it: {journal_after_dept}\n\
             base boot recovery returned {base_rec:?}\n\
             department pack, {} file(s) changed:\n  {}\n\
             base pack vs its pre-state after both boots, {} difference(s):\n  {}",
            dept_diff.len(),
            dept_diff.join("\n  "),
            base_diff.len(),
            base_diff.join("\n  ")
        );
        assert!(
            dept_diff.is_empty() && base_diff.is_empty(),
            "M15-다 (delete variant): wrong end state\n{evidence}"
        );
    }

    /// M15-다-2: a pro-revision update inside the SAME base version — (0.15.0, pro 1) → (0.15.0, pro 2),
    /// allowed by the version gate (`cys.rs` test `version_gates_pro_revision_tuple_transitions`) —
    /// dies in step ②. `.pack-version` is "0.15.0" before and after, and `apply_pack_transactional` is
    /// called with target "0.15.0". Single lane, no department involved. Correct: the next boot's
    /// recovery rolls the uncommitted apply back to the exact pre-state.
    /// Also MEASURED (printed, not asserted): whether the next run of the same update heals the result.
    #[test]
    fn m15_da2_same_base_revision_crash_in_step_2_is_not_rolled_back() {
        // A failing (ignored-by-design) test must not poison the lock for the next one.
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let pro = |rev: u32| PackState {
            channel: "pro".to_string(),
            base_version: "0.15.0".to_string(),
            pro_revision: rev,
        };
        let items = m15da_v2_items(M15DA_BASE);
        let root = m15da_root("da2");
        let _env = set_pack_env(root.join("pack"), root.join("cfg"));
        let pre = m15da_state(&m15da_lane(
            &root.join("pre"),
            "pack",
            "0.15.0",
            M15DA_BASE,
            &[],
            Some(pro(1)),
        ));
        let lane = ("0.15.0", M15DA_BASE, &[][..], Some(pro(1)));
        let (base, clean) =
            m15da_crash_in_step_2(&root, "pack", lane, &items, "0.15.0", &pro(2), 1);
        let crash = m15da_state(&base);
        let target_in_journal = t9_slot_index_path(&pack_journal_dir())
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<JournalIndex>(&s).ok())
            .map(|i| i.target_version);
        let rec = recover_pack_journal();
        let journal_after = t9_slot_has_index(&pack_journal_dir());
        let after_boot = m15da_state(&base);
        let state_after_boot = match read_pack_state(&base) {
            PackStateRead::Valid(s) => {
                format!("{}/{}/pro {}", s.channel, s.base_version, s.pro_revision)
            }
            other => format!("{other:?}"),
        };
        // Measurement: the user runs the same update again (the gate allows it: disk tuple is still
        // (0.15.0, 1) < (0.15.0, 2)). Compared with the twin's clean, complete apply.
        let rerun = apply_pack_transactional(&items, "0.15.0", &pro(2), None, || Ok(()));
        let after_rerun = m15da_state(&base);
        drop(_env);
        let _ = std::fs::remove_dir_all(&root);

        let boot_diff = m15da_diff(&pre, &after_boot);
        let rerun_vs_clean = m15da_diff(&clean, &after_rerun);
        let evidence = format!(
            "journal target_version = {target_in_journal:?}; .pack-version before = {:?}\n\
             after the crash (vs pre-state):\n  {}\n\
             boot recovery returned {rec:?}; journal still there after it: {journal_after}\n\
             .pack-state.json after the boot: {state_after_boot}\n\
             pack vs its pre-state after the boot recovery, {} difference(s):\n  {}\n\
             MEASURED follow-up — the same update run again returned {:?}; its end state vs a clean \
             complete apply, {} difference(s):\n  {}",
            pre.get(".pack-version").map(|v| v.0.as_str()),
            m15da_diff(&pre, &crash).join("\n  "),
            boot_diff.len(),
            boot_diff.join("\n  "),
            rerun.as_ref().map(|(w, k, ok)| (w, k, ok)),
            rerun_vs_clean.len(),
            rerun_vs_clean.join("\n  ")
        );
        assert!(
            boot_diff.is_empty(),
            "M15-다-2: the boot recovery did not roll the uncommitted apply back\n{evidence}"
        );
    }

    // ── T9 test helpers on the journal layout of RULINGS HE2 (round 2) ──────────────────────────────────────────────
    /// The test apply id (hex); its first 16 hex are the names' `<id>`.
    const T9_TEST_APPLY: &str = "0123456789abcdef0123456789abcdef";

    fn t9_test_id16() -> String {
        journal_id16(T9_TEST_APPLY)
    }

    /// A writer that has finished: a short child process spawned and reaped (used by the GX3 rule test only).
    fn t9_finished_writer() -> serde_json::Value {
        #[cfg(windows)]
        let mut cmd = {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "exit 0"]);
            c
        };
        #[cfg(not(windows))]
        let mut cmd = std::process::Command::new("true");
        let mut child = cmd
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn a short child");
        let pid = child.id();
        child.wait().expect("reap the short child");
        serde_json::json!({"pid": pid, "birth_us": 0})
    }

    /// The one entry of a lane's slot `L/active/` (name, path), if there is one.
    pub(super) fn t9_slot_entry(lane: &Path) -> Option<(String, PathBuf)> {
        let e = std::fs::read_dir(lane.join(JOURNAL_SLOT)).ok()?.flatten().next()?;
        Some((e.file_name().to_string_lossy().into_owned(), e.path()))
    }

    /// The index of the slot's entry, when it has one.
    pub(super) fn t9_slot_index_path(lane: &Path) -> Option<PathBuf> {
        t9_slot_entry(lane)
            .map(|(_, p)| p.join("index.json"))
            .filter(|p| p.is_file())
    }

    pub(super) fn t9_slot_has_index(lane: &Path) -> bool {
        t9_slot_index_path(lane).is_some()
    }

    /// Relabels the current lane's slot entry when it carries THIS process's identity (`j.`/`r.` + my pid + my start
    /// time) to the same kind and pid with start time `birth` — "1" = a process born at another time = GONE (exactly the
    /// name a dead process of that pid leaves), "x" = start time not recorded = UNKNOWN. Nothing is spawned or killed.
    pub(super) fn t9_relabel(lane: &Path, birth: &str) {
        let me = JournalWriter::current();
        if let Some((name, path)) = t9_slot_entry(lane) {
            let (k, pid, b, id) = match parse_slot_entry(&name) {
                Some(SlotEntry::Writer { pid, birth, id }) => ("j", pid, birth, id),
                Some(SlotEntry::Taker { pid, birth, id }) => ("r", pid, birth, id),
                _ => return,
            };
            if pid == me.pid && b == me.birth_us {
                std::fs::rename(&path, path.with_file_name(format!("{k}.{pid}.{birth}.{id}"))).unwrap();
            }
        }
    }

    /// What a crash leaves: the writer / taker of the current lane's journal is gone (see [`t9_relabel`]).
    pub(super) fn t9_writers_gone() {
        t9_relabel(&pack_journal_dir(), "1");
    }

    /// A hand-built journal in the slot of `lane`, written by a GONE process: `L/active/j.<my pid>.1.<test id>/`.
    pub(super) fn t9_hand_journal_in(lane: &Path) -> PathBuf {
        let p = lane
            .join(JOURNAL_SLOT)
            .join(format!("j.{}.1.{}", std::process::id(), t9_test_id16()));
        std::fs::create_dir_all(p.join("files")).unwrap();
        p
    }

    fn t9_hand_journal() -> PathBuf {
        t9_hand_journal_in(&pack_journal_dir())
    }

    /// Adds the apply record (`T9_TEST_APPLY`) to a hand-built index that has none.
    pub(super) fn t9_add_apply_record(jdir: &Path) {
        let ip = jdir.join("index.json");
        if let Ok(s) = std::fs::read_to_string(&ip) {
            let mut v: serde_json::Value = serde_json::from_str(&s).unwrap();
            if v.get("apply").is_none() {
                let tv = v["target_version"].as_str().unwrap_or("").to_string();
                v["apply"] = serde_json::json!({
                    "apply_id": T9_TEST_APPLY,
                    "target": {"channel": "free", "base_version": tv, "pro_revision": 0},
                    "items_digest": ""
                });
            }
            std::fs::write(&ip, v.to_string()).unwrap();
        }
    }

    /// `write_journal` as the transaction calls it, into an admitted slot entry of THIS process
    /// (`L/active/j.<me>.<test id>/`); returns that folder. (Its callers are the unix-only mode tests — DEFERRED #131.)
    #[cfg(unix)]
    fn t9_write_journal(
        target: &str,
        set: &std::collections::BTreeSet<String>,
    ) -> Result<PathBuf, String> {
        let own = pack_journal_dir()
            .join(JOURNAL_SLOT)
            .join(format!("j.{}.{}", JournalWriter::current().tag(), t9_test_id16()));
        std::fs::create_dir_all(&own).map_err(|e| e.to_string())?;
        let apply = PackApplied {
            apply_id: T9_TEST_APPLY.to_string(),
            target: test_free_state(target),
            items_digest: String::new(),
        };
        write_journal(&own, target, set, &apply, &mut |_| true).map(|_| own)
    }

    // ── T9 stage 2 (0.15.0 「스스로 해결」 · RULINGS GX1–GX7): the fix of M15-다 / 다-2 ─────────────────────────────
    // GX6 proof besides the five `m15_da*` tests above: a crash after EVERY step of the REAL transaction, the writer rule,
    // and the legacy journal. A "crash" is the real transaction stopped at a step boundary through the step callback of
    // `apply_pack_transactional_steps` (no rollback runs — what a dying process leaves) and the journal's writer marked
    // as a finished process (a real crash leaves no live writer; an in-process stop leaves the test process alive).

    /// The commit marker of an EARLIER apply, already in the pack before the update (a later apply must not take it
    /// for its own).
    const T9_PREVIOUS_MARKER: &str = "{\"apply_id\":\"an-earlier-apply\",\"target\":{\"channel\":\"free\",\"base_version\":\"0.15.0\",\"pro_revision\":0},\"items_digest\":\"\"}";

    /// Every file of a state except the commit marker (whose apply id differs per run).
    fn t9_without_marker(s: &M15daState) -> M15daState {
        s.iter()
            .filter(|(rel, _)| rel.as_str() != PACK_APPLIED_FILE)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn t9_marker_of(s: &M15daState) -> Option<PackApplied> {
        s.get(PACK_APPLIED_FILE)
            .and_then(|v| serde_json::from_str::<PackApplied>(&v.0).ok())
    }

    const T9_STEPS: [TxStep; 11] = [
        TxStep::Recovered,
        TxStep::Prepared,
        TxStep::Admitted,
        TxStep::BackedUp,
        TxStep::Journaled,
        TxStep::Installed,
        TxStep::StateWritten,
        TxStep::VersionWritten,
        TxStep::Committed,
        TxStep::PostCommitted,
        TxStep::Settled,
    ];

    /// GX6 (one crash test, every step): for a version change (0.15.0 → 0.15.1, free) AND a same-base-version revision
    /// (0.15.0 pro 1 → 0.15.0 pro 2 — 다-2), the REAL transaction is stopped after each step boundary (plus the stage-1
    /// crash inside ② after the first rewritten file). Then, with the writer gone:
    /// 1. the DEPARTMENT lane's recovery runs first — its pack and the base's journal must be unchanged (OQ-H-12 e);
    /// 2. the base lane's own recovery runs — the base pack must be EXACTLY its pre-state (bytes + modes, the earlier
    ///    apply's marker included) for every stop before the commit marker, and EXACTLY the state of a complete apply
    ///    (every file; the marker carrying the stopped run's apply id) for a stop at or after it; the journal is gone.
    #[test]
    fn t9_crash_after_every_step_is_entirely_old_or_entirely_new() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let pro = |rev: u32| PackState {
            channel: "pro".to_string(),
            base_version: "0.15.0".to_string(),
            pro_revision: rev,
        };
        let marker_extra: [(&str, &str); 1] = [(PACK_APPLIED_FILE, T9_PREVIOUS_MARKER)];
        let items = m15da_v2_items(M15DA_BASE);
        // (name, lane state before, target label, target tuple)
        let scenarios: [(&str, Option<PackState>, &str, PackState); 2] = [
            ("version-change", None, "0.15.1", test_free_state("0.15.1")),
            ("same-base-revision", Some(pro(1)), "0.15.0", pro(2)),
        ];
        let mut failures: Vec<String> = Vec::new();
        for (name, before_state, target, new_state) in scenarios.iter() {
            // The complete apply, for "entirely new".
            let ref_root = m15da_root(&format!("t9-every-ref-{name}"));
            let ref_pd = m15da_lane(&ref_root, "pack", "0.15.0", M15DA_BASE, &marker_extra, before_state.clone());
            let complete = {
                let _e = set_pack_env(&ref_pd, ref_root.join("cfg"));
                apply_pack_transactional(&items, target, new_state, None, || Ok(()))
                    .expect("the complete reference apply");
                m15da_state(&ref_pd)
            };
            let _ = std::fs::remove_dir_all(&ref_root);
            let mut stops: Vec<Option<TxStep>> = T9_STEPS.iter().copied().map(Some).collect();
            stops.push(None); // None = the stage-1 crash inside ② (after the first rewritten file)
            for stop in stops {
                let label = format!("{name} / stop after {stop:?}");
                let root = m15da_root(&format!("t9-every-{name}-{}", stop.map(|s| format!("{s:?}")).unwrap_or_else(|| "mid-2".into())));
                let dept = m15da_lane(&root, "pack-dept-d1", "0.15.0", M15DA_DEPT, M15DA_SIDECARS, None);
                let _env = set_pack_env(root.join("pack"), root.join("cfg"));
                let pre = m15da_state(&m15da_lane(&root.join("pre"), "pack", "0.15.0", M15DA_BASE, &marker_extra, before_state.clone()));
                let base = match stop {
                    Some(s) => {
                        let pd = m15da_lane(&root, "pack", "0.15.0", M15DA_BASE, &marker_extra, before_state.clone());
                        let r = apply_pack_transactional_steps(&items, target, new_state, None, || Ok(()), &mut |at| at != s);
                        if r.is_ok() {
                            failures.push(format!("{label}: the step callback did not stop the transaction"));
                        }
                        t9_writers_gone();
                        pd
                    }
                    None => {
                        let lane = ("0.15.0", M15DA_BASE, &marker_extra[..], before_state.clone());
                        m15da_crash_in_step_2(&root, "pack", lane, &items, target, new_state, 1).0
                    }
                };
                let crashed = m15da_state(&base);
                let jdir = pack_journal_dir();
                let journal_before = m15da_state(&jdir);
                let entry_before = t9_slot_entry(&jdir).map(|e| e.0);
                let slot_before = jdir.join(JOURNAL_SLOT).exists();
                let stopped_id = t9_slot_index_path(&jdir)
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .and_then(|s| serde_json::from_str::<JournalIndex>(&s).ok())
                    .and_then(|i| i.apply)
                    .map(|a| a.apply_id);
                // 1. the department daemon starts first.
                let dept_pre = m15da_state(&dept);
                let dept_rec = {
                    let _e = EnvGuard::set(ENV_PACK_DIR, &dept);
                    recover_pack_journal()
                };
                let dept_diff = m15da_diff(&dept_pre, &m15da_state(&dept));
                let journal_diff = m15da_diff(&journal_before, &m15da_state(&jdir));
                let entry_after_dept = t9_slot_entry(&jdir).map(|e| e.0);
                if dept_rec != Ok(false) || !dept_diff.is_empty() || !journal_diff.is_empty() || entry_after_dept != entry_before {
                    failures.push(format!(
                        "{label}: the department's recovery touched something: rec={dept_rec:?} dept={dept_diff:?} base journal={journal_diff:?}"
                    ));
                }
                if m15da_diff(&crashed, &m15da_state(&base)).len() != 0 {
                    failures.push(format!("{label}: the department's recovery changed the base pack"));
                }
                // 2. the base daemon starts.
                let rec = recover_pack_journal();
                let after = m15da_state(&base);
                let journal_left = jdir.join(JOURNAL_SLOT).exists();
                let expect_new = matches!(
                    stop,
                    Some(TxStep::Committed) | Some(TxStep::PostCommitted) | Some(TxStep::Settled)
                );
                if expect_new {
                    let d = m15da_diff(&t9_without_marker(&complete), &t9_without_marker(&after));
                    let m = t9_marker_of(&after);
                    let ref_m = t9_marker_of(&complete);
                    let marker_ok = m.as_ref().map(|m| Some(&m.apply_id) == stopped_id.as_ref()).unwrap_or(false)
                        && m.as_ref().map(|m| (&m.target, &m.items_digest)) == ref_m.as_ref().map(|m| (&m.target, &m.items_digest));
                    if !d.is_empty() || !marker_ok || rec != Ok(true) {
                        failures.push(format!(
                            "{label}: expected ENTIRELY NEW; rec={rec:?} marker={m:?} journal id={stopped_id:?} diff vs complete apply={d:?}"
                        ));
                    }
                } else {
                    let d = m15da_diff(&pre, &after);
                    // a journal in the slot (whatever its content) is settled by this recovery → Ok(true); no slot → Ok(false)
                    let rec_ok = if slot_before { rec == Ok(true) } else { rec == Ok(false) };
                    if !d.is_empty() || !rec_ok {
                        failures.push(format!("{label}: expected ENTIRELY OLD; rec={rec:?} diff vs pre-state={d:?}"));
                    }
                }
                if journal_left {
                    failures.push(format!("{label}: the journal is still there after the base's recovery"));
                }
                drop(_env);
                let _ = std::fs::remove_dir_all(&root);
            }
        }
        assert!(failures.is_empty(), "GX6 crash proof:\n  {}", failures.join("\n  "));
    }

    /// GX3: a journal whose writer is still running — or whose liveness cannot be decided — is not touched (pack,
    /// journal), the recovery says so in one line (Err), and a pack update of that lane refuses instead of overwriting it.
    #[test]
    fn t9_journal_of_a_running_or_unknown_writer_is_not_touched() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let items = m15da_v2_items(M15DA_BASE);
        let st = test_free_state("0.15.1");
        let mut failures: Vec<String> = Vec::new();
        // (stop, how the writer is set)
        for (stop, writer) in [
            (TxStep::Installed, "running"),
            (TxStep::Admitted, "running"),
            (TxStep::Installed, "unknown"),
            (TxStep::Admitted, "unknown"),
        ] {
            let label = format!("stop after {stop:?}, writer {writer}");
            let root = m15da_root(&format!("t9-live-{stop:?}-{writer}"));
            let pd = m15da_lane(&root, "pack", "0.15.0", M15DA_BASE, &[], None);
            let _env = set_pack_env(&pd, root.join("cfg"));
            let _ = apply_pack_transactional_steps(&items, "0.15.1", &st, None, || Ok(()), &mut |at| at != stop);
            let jdir = pack_journal_dir();
            if writer == "unknown" {
                // this live process, but without a recorded start time = liveness cannot be decided (the name's `x`)
                t9_relabel(&jdir, "x");
            }
            let pack_before = m15da_state(&pd);
            let journal_before = m15da_state(&jdir);
            let entry_before = t9_slot_entry(&jdir).map(|e| e.0);
            let rec = recover_pack_journal();
            let again = apply_pack_transactional(&items, "0.15.1", &st, None, || Ok(()));
            let pack_diff = m15da_diff(&pack_before, &m15da_state(&pd));
            let journal_diff = m15da_diff(&journal_before, &m15da_state(&jdir));
            let entry_after = t9_slot_entry(&jdir).map(|e| e.0);
            let leftovers: Vec<String> = std::fs::read_dir(&jdir)
                .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("t.")).collect())
                .unwrap_or_default();
            if rec.is_ok() || again.is_ok() || !pack_diff.is_empty() || !journal_diff.is_empty() || entry_before.is_none()
                || entry_after != entry_before || !leftovers.is_empty()
            {
                failures.push(format!(
                    "{label}: rec={rec:?} update again={again:?} pack={pack_diff:?} journal={journal_diff:?} entry {entry_before:?} -> {entry_after:?} prepared leftovers={leftovers:?}"
                ));
            }
            drop(_env);
            let _ = std::fs::remove_dir_all(&root);
        }
        assert!(failures.is_empty(), "GX3:\n  {}", failures.join("\n  "));
    }

    /// GX3 (the decision rule, pure): one observation decides; only proof of death (no such process, or the pid now
    /// held by a process born at another time) lets a recovery act.
    #[test]
    fn t9_writer_life_is_decided_from_one_observation() {
        use crate::process_observation::Failure;
        let w = JournalWriter { pid: 4242, birth_us: Some(1_000) };
        let blind = JournalWriter { pid: 4242, birth_us: None };
        assert_eq!(writer_life(&w, Err(Failure::Dead)), WriterLife::Gone);
        assert_eq!(writer_life(&blind, Err(Failure::Dead)), WriterLife::Gone);
        assert_eq!(writer_life(&w, Ok(2_000)), WriterLife::Gone, "pid reused by a younger process");
        assert_eq!(writer_life(&w, Ok(1_000)), WriterLife::Running);
        for e in [
            Failure::Unknown { stage: "proc_pidinfo", os_error: Some(1) },
            Failure::Changed,
            Failure::InvalidPid,
            Failure::Malformed("x"),
            Failure::UnsupportedPlatform,
        ] {
            assert!(matches!(writer_life(&w, Err(e.clone())), WriterLife::Unknown(_)), "{e:?}");
        }
        assert!(matches!(writer_life(&blind, Ok(1_000)), WriterLife::Unknown(_)));
        // the real query on this live process
        let me = JournalWriter::current();
        assert!(me.birth_us.is_some(), "this process's start time must be readable here");
        assert_eq!(observe_writer(&me), WriterLife::Running);
        let gone = serde_json::from_value::<JournalWriter>(t9_finished_writer()).unwrap();
        assert_eq!(observe_writer(&gone), WriterLife::Gone);
    }

    /// GX4: a journal at the OLD shared location (an older binary's, no lane, no apply id) is touched by no lane — not
    /// by the base's or a department's recovery, not by a new pack update — and both lanes name it with the remedy.
    #[test]
    fn t9_legacy_shared_journal_is_touched_by_no_lane_and_named() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = m15da_root("t9-legacy");
        let base = m15da_lane(&root, "pack", "0.15.0", M15DA_BASE, &[], None);
        let dept = m15da_lane(&root, "pack-dept-d1", "0.15.0", M15DA_DEPT, M15DA_SIDECARS, None);
        let legacy = root.join(PACK_JOURNAL_DIR);
        std::fs::create_dir_all(legacy.join("files/bin")).unwrap();
        std::fs::write(legacy.join("files/bin/selfheal.sh"), "#!/bin/sh\necho selfheal-v0\n").unwrap();
        std::fs::write(
            legacy.join("index.json"),
            r#"{"target_version":"0.15.1","entries":[{"rel":"bin/selfheal.sh","existed":true},{"rel":"bin/new-in-v2.sh","existed":false}]}"#,
        )
        .unwrap();
        let _env = set_pack_env(&base, root.join("cfg"));
        let legacy_before = m15da_state(&legacy);
        let base_before = m15da_state(&base);
        let dept_before = m15da_state(&dept);
        let base_rec = recover_pack_journal();
        let dept_rec = {
            let _e = EnvGuard::set(ENV_PACK_DIR, &dept);
            recover_pack_journal()
        };
        let base_after_rec = m15da_state(&base);
        let update = apply_pack_transactional(&m15da_v2_items(M15DA_BASE), "0.15.1", &test_free_state("0.15.1"), None, || Ok(()));
        let legacy_after = m15da_state(&legacy);
        let dept_after = m15da_state(&dept);
        let notice_base = legacy_pack_journal_notice(&base);
        let notice_dept = legacy_pack_journal_notice(&dept);
        let base_new_journal = pack_journal_dir();
        drop(_env);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(base_rec, Ok(false), "the base's recovery must not act on the legacy journal");
        assert_eq!(dept_rec, Ok(false), "the department's recovery must not act on the legacy journal");
        assert!(m15da_diff(&base_before, &base_after_rec).is_empty(), "the base pack changed at recovery");
        assert!(m15da_diff(&dept_before, &dept_after).is_empty(), "the department pack changed");
        assert!(update.is_ok(), "a new pack update must still work beside a legacy journal: {update:?}");
        assert_eq!(legacy_before, legacy_after, "the legacy journal was touched");
        assert!(!legacy_after.is_empty());
        assert_ne!(base_new_journal, legacy, "the new journal must not be the legacy folder");
        for n in [&notice_base, &notice_dept] {
            let n = n.as_deref().expect("the legacy journal must be named");
            assert!(n.contains(&legacy.display().to_string()), "{n}");
            assert!(n.contains("팩 업데이트를 다시 실행"), "the remedy is missing: {n}");
        }
    }

    // ── T9 round 2 (RULINGS HB3 / HE7): reviewer1's sequence — red on 4afccd4a (0238b1fd), green with HE + HG ──────
    /// HB3 (1)/(2) — reviewer1's sequence (verdict-T9-r1-codex.json, blocking at src/pack.rs:8203 of 4afccd4a), run
    /// deterministically on two threads ordered by channels (no sleep): J = an earlier update of the lane that crashed
    /// (its writer gone) — before its commit marker (`j_committed == false`, the roll-back branch) or after it (the
    /// clean-up branch). D = the lane's daemon starting: its recovery judges J's writer gone and is PAUSED there (seam
    /// `recover_pack_journal_with`). K = a `cys pack-update` of the same lane: its ⓪ recovers J, then it writes its own
    /// journal and applies; it is PAUSED right after ② (`TxStep::Installed` — the nearest stop of the real transaction to
    /// "mid ②"). D resumes and finishes; then K resumes and finishes. Correct: D's resumption leaves K's journal and every
    /// pack file exactly as K left them and reports nothing as recovered; K ends with the pack ENTIRELY new.
    /// Returns the failures (empty = correct).
    fn t9_r2_reviewer_sequence(tag: &str, j_committed: bool) -> Vec<String> {
        use std::sync::mpsc::channel;
        use std::time::Duration;
        let mut failures: Vec<String> = Vec::new();
        let items_j = m15da_v2_items(M15DA_BASE);
        let st_j = test_free_state("0.15.1");
        let mut items_k: Vec<(&str, &str)> = items_j.clone();
        for it in items_k.iter_mut() {
            match it.0 {
                "bin/selfheal.sh" => it.1 = "#!/bin/sh\necho selfheal-v3\n",
                "README.md" => it.1 = "README v3\n",
                _ => {}
            }
        }
        let st_k = test_free_state("0.15.2");
        // The complete outcome: J rolled back (or committed), then K applied whole.
        let ref_root = m15da_root(&format!("t9r2-{tag}-ref"));
        let ref_pd = m15da_lane(&ref_root, "pack", "0.15.0", M15DA_BASE, &[], None);
        let complete = {
            let _e = set_pack_env(&ref_pd, ref_root.join("cfg"));
            if j_committed {
                apply_pack_transactional(&items_j, "0.15.1", &st_j, None, || Ok(())).expect("reference J");
            }
            apply_pack_transactional(&items_k, "0.15.2", &st_k, None, || Ok(())).expect("reference K");
            m15da_state(&ref_pd)
        };
        let _ = std::fs::remove_dir_all(&ref_root);

        let root = m15da_root(&format!("t9r2-{tag}"));
        let pd = m15da_lane(&root, "pack", "0.15.0", M15DA_BASE, &[], None);
        let _env = set_pack_env(&pd, root.join("cfg"));
        let j_stop = if j_committed { TxStep::Committed } else { TxStep::Installed };
        let _ = apply_pack_transactional_steps(&items_j, "0.15.1", &st_j, None, || Ok(()), &mut |s| s != j_stop);
        t9_writers_gone();
        let lane = pack_journal_dir();

        // D: the daemon's recovery, paused right after it judged J's writer gone.
        let (tx_judged, rx_judged) = channel::<()>();
        let (tx_go, rx_go) = channel::<()>();
        let daemon = std::thread::spawn(move || {
            let mut on_judged = move || {
                let _ = tx_judged.send(());
                rx_go.recv_timeout(Duration::from_secs(60)).is_ok()
            };
            recover_pack_journal_with(&mut on_judged)
        });
        if rx_judged.recv_timeout(Duration::from_secs(60)).is_err() {
            failures.push("the daemon's recovery never judged J's writer gone — the sequence was not exercised".into());
        }
        // K: the update of the same lane, paused right after ②; D resumes inside that pause.
        let mut daemon = Some(daemon);
        let mut tx_go = Some(tx_go);
        let mut seen: Option<(M15daState, M15daState, M15daState, M15daState, Result<bool, String>)> = None;
        let k = apply_pack_transactional_steps(&items_k, "0.15.2", &st_k, None, || Ok(()), &mut |s| {
            if s == TxStep::Installed && seen.is_none() {
                let lane_before = m15da_state(&lane);
                let pack_before = m15da_state(&pd);
                if let Some(tx) = tx_go.take() {
                    let _ = tx.send(());
                }
                let d = daemon
                    .take()
                    .map(|h| h.join().unwrap_or_else(|_| Err("the daemon thread panicked".into())))
                    .unwrap_or_else(|| Err("no daemon".into()));
                seen = Some((lane_before, m15da_state(&lane), pack_before, m15da_state(&pd), d));
            }
            true
        });
        if let Some(h) = daemon.take() {
            drop(tx_go.take());
            let _ = h.join();
        }
        let after = m15da_state(&pd);
        drop(_env);
        let _ = std::fs::remove_dir_all(&root);

        match seen {
            None => failures.push(format!("K never reached the pause after ② ({k:?})")),
            Some((lane_before, lane_after, pack_before, pack_after, d)) => {
                if d == Ok(true) {
                    failures.push(format!("the daemon reported a recovery after K had recovered J: {d:?}"));
                }
                let jd = m15da_diff(&lane_before, &lane_after);
                if !jd.is_empty() {
                    failures.push(format!("the daemon's resumption changed K's journal: {jd:?}"));
                }
                let pdiff = m15da_diff(&pack_before, &pack_after);
                if !pdiff.is_empty() {
                    failures.push(format!("the daemon's resumption changed the pack under the running K: {pdiff:?}"));
                }
                let k_id = lane_before
                    .iter()
                    .filter(|(rel, _)| rel.ends_with("index.json"))
                    .filter_map(|(_, v)| serde_json::from_str::<JournalIndex>(&v.0).ok())
                    .filter_map(|i| i.apply.map(|a| a.apply_id))
                    .next();
                if k.is_err() {
                    failures.push(format!("K did not finish: {k:?}"));
                }
                let d_final = m15da_diff(&t9_without_marker(&complete), &t9_without_marker(&after));
                let marker = t9_marker_of(&after).map(|m| m.apply_id);
                if !d_final.is_empty() || marker.is_none() || marker != k_id {
                    failures.push(format!(
                        "the pack is not ENTIRELY new after K: marker={marker:?} K's id={k_id:?} diff vs the complete outcome={d_final:?}"
                    ));
                }
            }
        }
        failures
    }

    #[test]
    fn t9_r2_daemon_judged_gone_then_update_reuses_the_path_rollback_branch() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = t9_r2_reviewer_sequence("rb", false);
        assert!(f.is_empty(), "HB3 (1):\n  {}", f.join("\n  "));
    }

    #[test]
    fn t9_r2_daemon_judged_gone_then_update_reuses_the_path_cleanup_branch() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = t9_r2_reviewer_sequence("cl", true);
        assert!(f.is_empty(), "HB3 (2):\n  {}", f.join("\n  "));
    }

    // ── T9 round 2 (RULINGS HE / HG): the protocol's proofs — HB3 (3)–(6), (8) and HE7 (a)–(d) ──────────────────────
    // One test process stands for other processes through NAMES (HE2): a gone process = my pid with a start time that is
    // not mine (`t9_relabel(.., "1")`), a running one = my own pid and start time, an unknown one = my pid + `x`. A
    // recoverer or writer that "dies" = its seam returns false (nothing after that point runs), then its name is
    // relabelled to a gone identity — the name a dead process of that pid leaves. Nothing is spawned or killed.

    fn t9r2_items() -> Vec<(&'static str, &'static str)> {
        m15da_v2_items(M15DA_BASE)
    }

    /// A lane at 0.15.0 under a fresh root; the env names it. Returns (root, pack, the pre-state, env guards).
    fn t9r2_lane(tag: &str) -> (PathBuf, PathBuf, M15daState, (EnvGuard, EnvGuard)) {
        let root = m15da_root(tag);
        let pre = m15da_state(&m15da_lane(&root.join("pre"), "pack", "0.15.0", M15DA_BASE, &[], None));
        let pd = m15da_lane(&root, "pack", "0.15.0", M15DA_BASE, &[], None);
        let env = set_pack_env(&pd, root.join("cfg"));
        (root, pd, pre, env)
    }

    /// The REAL transaction of the current lane stopped after `stop` (as a dying process would).
    fn t9r2_stopped_apply(stop: TxStep) -> Result<(usize, usize, bool), String> {
        apply_pack_transactional_steps(&t9r2_items(), "0.15.1", &test_free_state("0.15.1"), None, || Ok(()), &mut |s| s != stop)
    }

    /// One recovery pass with explicit seams (production = all true).
    fn t9r2_recover(
        judged: &mut dyn FnMut() -> bool,
        held: &mut dyn FnMut() -> bool,
        restored: &mut dyn FnMut(usize) -> bool,
        settled: &mut dyn FnMut() -> bool,
    ) -> Result<bool, String> {
        recover_lane(&mut RecoverSeam { judged, held, restored, settled })
    }

    /// Prepared admission folders `t.…` left in the current lane (an apply leaves none unless it died before admission).
    fn t9r2_prepared_left() -> Vec<String> {
        std::fs::read_dir(pack_journal_dir())
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.starts_with("t."))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn t9r2_me() -> JournalWriter {
        JournalWriter::current()
    }

    /// HB3 (3) / HE7 (d): two recoverers of ONE journal — exactly one acts. (i) R1 judges the gone writer, R2 recovers
    /// the journal completely inside R1's pause, R1 resumes: its take finds no such name → nothing touched, Ok(false).
    /// (ii) two takers, one name: R2 takes and dies holding it; R1's take finds nothing → exactly one holder. (iii) a
    /// second recoverer beside a taker mid roll-back (the holder's name is a running process) touches nothing.
    #[test]
    fn t9_r2_two_recoverers_one_name_exactly_one_acts() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut failures: Vec<String> = Vec::new();
        // (i)
        {
            let (root, pd, pre, _env) = t9r2_lane("t9r2-two-rec-i");
            let _ = t9r2_stopped_apply(TxStep::Installed);
            t9_writers_gone();
            let mut r2: Option<Result<bool, String>> = None;
            let r1 = recover_pack_journal_with(&mut || {
                r2 = Some(recover_pack_journal());
                true
            });
            let after = m15da_state(&pd);
            if r2 != Some(Ok(true)) || r1 != Ok(false) || !m15da_diff(&pre, &after).is_empty() || pack_journal_dir().join(JOURNAL_SLOT).exists() {
                failures.push(format!("(i) r1={r1:?} r2={r2:?} pack vs pre={:?}", m15da_diff(&pre, &after)));
            }
            drop(_env);
            let _ = std::fs::remove_dir_all(&root);
        }
        // (ii)
        {
            let (root, pd, _pre, _env) = t9r2_lane("t9r2-two-rec-ii");
            let _ = t9r2_stopped_apply(TxStep::Installed);
            t9_writers_gone();
            let crashed = m15da_state(&pd);
            let mut r2: Option<Result<bool, String>> = None;
            let r1 = recover_pack_journal_with(&mut || {
                r2 = Some(t9r2_recover(&mut || true, &mut || false, &mut |_| true, &mut || true));
                true
            });
            let holder = t9_slot_entry(&pack_journal_dir()).map(|e| e.0);
            let expected = format!("r.{}.{}", t9r2_me().tag(), "");
            let one_holder = holder.as_deref().map(|h| h.starts_with(&expected)).unwrap_or(false);
            if r1 != Ok(false) || !one_holder || !m15da_diff(&crashed, &m15da_state(&pd)).is_empty() {
                failures.push(format!("(ii) r1={r1:?} r2={r2:?} holder={holder:?}"));
            }
            drop(_env);
            let _ = std::fs::remove_dir_all(&root);
        }
        // (iii)
        {
            let (root, pd, pre, _env) = t9r2_lane("t9r2-two-rec-iii");
            let _ = t9r2_stopped_apply(TxStep::Installed);
            t9_writers_gone();
            let mut r2: Option<(Result<bool, String>, M15daState, Option<String>)> = None;
            let r1 = t9r2_recover(&mut || true, &mut || true, &mut |n| {
                if n == 1 && r2.is_none() {
                    let before = m15da_state(&pd);
                    let entry = t9_slot_entry(&pack_journal_dir()).map(|e| e.0);
                    let r = recover_pack_journal();
                    let d = m15da_diff(&before, &m15da_state(&pd));
                    r2 = Some((r, if d.is_empty() { M15daState::new() } else { m15da_state(&pd) }, entry));
                }
                true
            }, &mut || true);
            let ok_r2 = matches!(&r2, Some((Err(line), changed, Some(_))) if line.contains(&format!("pid {}", std::process::id())) && changed.is_empty());
            if r1 != Ok(true) || !ok_r2 || !m15da_diff(&pre, &m15da_state(&pd)).is_empty() {
                failures.push(format!("(iii) r1={r1:?} r2={:?}", r2.as_ref().map(|x| (&x.0, x.1.len(), &x.2))));
            }
            drop(_env);
            let _ = std::fs::remove_dir_all(&root);
        }
        assert!(failures.is_empty(), "two recoverers:\n  {}", failures.join("\n  "));
    }

    /// HB3 (4): a taker that dies mid roll-back leaves `r.<it>.<id>`; the next entry re-takes it from that exact name
    /// and finishes: the pack ends ENTIRELY old, the slot is gone.
    #[test]
    fn t9_r2_taker_gone_mid_rollback_is_retaken_and_the_pack_ends_old() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (root, pd, pre, _env) = t9r2_lane("t9r2-retake");
        let _ = t9r2_stopped_apply(TxStep::Installed);
        t9_writers_gone();
        let r1 = t9r2_recover(&mut || true, &mut || true, &mut |n| n < 2, &mut || true);
        let mid = m15da_state(&pd);
        let holder = t9_slot_entry(&pack_journal_dir()).map(|e| e.0);
        t9_writers_gone(); // the taker died
        let r2 = recover_pack_journal();
        let after = m15da_state(&pd);
        let slot_left = pack_journal_dir().join(JOURNAL_SLOT).exists();
        drop(_env);
        let _ = std::fs::remove_dir_all(&root);
        assert!(r1.is_err(), "the first taker must stop mid roll-back: {r1:?}");
        assert!(holder.as_deref().map(|h| h.starts_with("r.")).unwrap_or(false), "holder after the stop: {holder:?}");
        assert!(!m15da_diff(&pre, &mid).is_empty(), "precondition: the roll-back was only partly done");
        assert_eq!(r2, Ok(true), "the next entry must re-take and finish");
        assert!(m15da_diff(&pre, &after).is_empty(), "pack not entirely old: {:?}", m15da_diff(&pre, &after));
        assert!(!slot_left, "the slot must be gone");
    }

    /// HB3 (5): a starter beside a journal of a RUNNING writer, and beside a journal held by a RUNNING taker, is refused:
    /// nothing of the pack or the journal changes, no prepared folder is left, the line names the pid.
    #[test]
    fn t9_r2_starter_beside_a_running_writer_or_taker_is_refused() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut failures: Vec<String> = Vec::new();
        for case in ["running writer", "running taker"] {
            let (root, pd, _pre, _env) = t9r2_lane(&format!("t9r2-starter-{}", case.replace(' ', "-")));
            let _ = t9r2_stopped_apply(TxStep::Installed);
            if case == "running taker" {
                t9_writers_gone();
                let _ = t9r2_recover(&mut || true, &mut || false, &mut |_| true, &mut || true); // takes, then "pauses"
            }
            let lane = pack_journal_dir();
            let pack_before = m15da_state(&pd);
            let lane_before = m15da_state(&lane);
            let entry_before = t9_slot_entry(&lane).map(|e| e.0);
            let s = apply_pack_transactional(&t9r2_items(), "0.15.1", &test_free_state("0.15.1"), None, || Ok(()));
            let names_pid = s.as_ref().err().map(|l| l.contains(&format!("pid {}", std::process::id()))).unwrap_or(false);
            if s.is_ok() || !names_pid || !m15da_diff(&pack_before, &m15da_state(&pd)).is_empty()
                || !m15da_diff(&lane_before, &m15da_state(&lane)).is_empty()
                || t9_slot_entry(&lane).map(|e| e.0) != entry_before
                || !t9r2_prepared_left().is_empty()
            {
                failures.push(format!("{case}: starter={s:?} entry={entry_before:?} prepared={:?}", t9r2_prepared_left()));
            }
            drop(_env);
            let _ = std::fs::remove_dir_all(&root);
        }
        assert!(failures.is_empty(), "HB3 (5):\n  {}", failures.join("\n  "));
    }

    /// HB3 (6): two starters through the step seam — A is paused after preparing (before its admission rename) while B
    /// is admitted and running; A's admission then fails: exactly one admitted, A leaves nothing (no prepared folder,
    /// the slot holds only B's journal, the pack is as B left it).
    #[test]
    fn t9_r2_two_starters_exactly_one_admitted() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (root, pd, _pre, _env) = t9r2_lane("t9r2-two-starters");
        let mut b_state: Option<(Option<String>, M15daState)> = None;
        let a = apply_pack_transactional_steps(&t9r2_items(), "0.15.1", &test_free_state("0.15.1"), None, || Ok(()), &mut |s| {
            if s == TxStep::Prepared && b_state.is_none() {
                let _ = t9r2_stopped_apply(TxStep::Journaled); // B: admitted, journal written, still "running"
                b_state = Some((t9_slot_entry(&pack_journal_dir()).map(|e| e.0), m15da_state(&pd)));
            }
            true
        });
        let entry_after = t9_slot_entry(&pack_journal_dir()).map(|e| e.0);
        let pack_after = m15da_state(&pd);
        let prepared = t9r2_prepared_left();
        drop(_env);
        let _ = std::fs::remove_dir_all(&root);
        let (b_entry, b_pack) = b_state.expect("B ran inside A's pause");
        assert!(a.is_err(), "A must be refused: {a:?}");
        assert!(b_entry.as_deref().map(|e| e.starts_with("j.")).unwrap_or(false), "B must hold the slot: {b_entry:?}");
        assert_eq!(entry_after, b_entry, "the slot must still hold only B's journal");
        assert!(m15da_diff(&b_pack, &pack_after).is_empty(), "A changed the pack");
        assert!(prepared.is_empty(), "A left its prepared folder: {prepared:?}");
    }

    /// HB3 (8): a journal whose writer is gone and that has no index (the backups were never completed — no pack file
    /// was written) is SETTLED and removed; with a running writer it is kept.
    #[test]
    fn t9_r2_gone_writer_without_index_is_settled_running_is_kept() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut failures: Vec<String> = Vec::new();
        for stop in [TxStep::Admitted, TxStep::BackedUp] {
            for writer in ["gone", "running"] {
                let (root, pd, pre, _env) = t9r2_lane(&format!("t9r2-noindex-{stop:?}-{writer}"));
                let _ = t9r2_stopped_apply(stop);
                if writer == "gone" {
                    t9_writers_gone();
                }
                let entry_before = t9_slot_entry(&pack_journal_dir()).map(|e| e.0);
                let rec = recover_pack_journal();
                let slot_left = pack_journal_dir().join(JOURNAL_SLOT).exists();
                let entry_after = t9_slot_entry(&pack_journal_dir()).map(|e| e.0);
                let pack_same = m15da_diff(&pre, &m15da_state(&pd)).is_empty();
                let ok = match writer {
                    "gone" => rec == Ok(true) && !slot_left && pack_same,
                    _ => rec.is_err() && entry_after == entry_before && entry_before.is_some() && pack_same,
                };
                if !ok {
                    failures.push(format!("{stop:?}/{writer}: rec={rec:?} entry {entry_before:?} -> {entry_after:?} slot_left={slot_left} pack_same={pack_same}"));
                }
                drop(_env);
                let _ = std::fs::remove_dir_all(&root);
            }
        }
        assert!(failures.is_empty(), "HB3 (8):\n  {}", failures.join("\n  "));
    }

    /// HE4 GIVE BACK: a roll-back that fails on the way (a backup cannot be read) gives the journal back as `u.<id>` with
    /// every backup still there and says so; the next entry takes `u.<id>` and, once the backup is readable, finishes —
    /// the pack ends entirely old.
    #[test]
    fn t9_r2_failed_rollback_gives_back_and_keeps_every_backup() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (root, pd, pre, _env) = t9r2_lane("t9r2-giveback");
        let _ = t9r2_stopped_apply(TxStep::Installed);
        t9_writers_gone();
        let (_, entry) = t9_slot_entry(&pack_journal_dir()).expect("the crashed journal");
        let broken = entry.join("files").join("README.md");
        std::fs::remove_file(&broken).unwrap();
        std::fs::create_dir_all(&broken).unwrap(); // a directory where a backup file should be = unreadable
        let backups_before = m15da_state(&entry.join("files")).len();
        let rec = recover_pack_journal();
        let given = t9_slot_entry(&pack_journal_dir());
        let backups_after = given.as_ref().map(|(_, p)| m15da_state(&p.join("files")).len());
        // the backup becomes readable again (what the user's fix would do) → the next entry finishes
        if let Some((_, p)) = &given {
            let b = p.join("files").join("README.md");
            let _ = std::fs::remove_dir_all(&b);
            std::fs::write(&b, "README v1\n").unwrap();
        }
        let rec2 = recover_pack_journal();
        let after = m15da_state(&pd);
        let slot_left = pack_journal_dir().join(JOURNAL_SLOT).exists();
        drop(_env);
        let _ = std::fs::remove_dir_all(&root);
        assert!(rec.as_ref().err().map(|l| l.contains("u.")).unwrap_or(false), "the line must say it was given back: {rec:?}");
        assert!(given.as_ref().map(|(n, _)| n.starts_with("u.")).unwrap_or(false), "the journal must be given back as u.<id>: {given:?}");
        assert_eq!(backups_after, Some(backups_before), "a backup was removed by the failed roll-back");
        assert_eq!(rec2, Ok(true), "the next entry must take u.<id> and finish");
        assert!(m15da_diff(&pre, &after).is_empty(), "pack not entirely old: {:?}", m15da_diff(&pre, &after));
        assert!(!slot_left);
    }

    /// HE7 (a): a starter stopped anywhere around its admission (after ⓪, after preparing) while the lane's journal is
    /// taken by a recoverer that dies holding it and re-taken by another that dies mid roll-back: the starter is never
    /// admitted while the journal is unsettled and leaves nothing; the next entry settles the journal (pack entirely
    /// old) and the lane accepts an apply again.
    #[test]
    fn t9_r2_starter_never_admitted_while_the_journal_is_retaken_by_dying_recoverers() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut failures: Vec<String> = Vec::new();
        for pause_at in [TxStep::Recovered, TxStep::Prepared] {
            let (root, pd, pre, _env) = t9r2_lane(&format!("t9r2-he7a-{pause_at:?}"));
            let mut done = false;
            let s = apply_pack_transactional_steps(&t9r2_items(), "0.15.1", &test_free_state("0.15.1"), None, || Ok(()), &mut |at| {
                if at == pause_at && !done {
                    done = true;
                    let _ = t9r2_stopped_apply(TxStep::Installed); // W: admitted, crashed mid-way
                    t9_writers_gone();
                    let _ = t9r2_recover(&mut || true, &mut || false, &mut |_| true, &mut || true); // R1 takes, dies
                    t9_writers_gone();
                    let _ = t9r2_recover(&mut || true, &mut || true, &mut |n| n < 1, &mut || true); // R2 re-takes, dies mid roll-back
                    t9_writers_gone();
                }
                true
            });
            let entry = t9_slot_entry(&pack_journal_dir()).map(|e| e.0);
            let prepared = t9r2_prepared_left();
            let settle = recover_pack_journal();
            let pack_old = m15da_diff(&pre, &m15da_state(&pd)).is_empty();
            let again = apply_pack_transactional(&t9r2_items(), "0.15.1", &test_free_state("0.15.1"), None, || Ok(()));
            if s.is_ok() || !entry.as_deref().map(|e| e.starts_with("r.")).unwrap_or(false) || !prepared.is_empty()
                || settle != Ok(true) || !pack_old || again.is_err()
            {
                failures.push(format!(
                    "pause at {pause_at:?}: starter={s:?} entry={entry:?} prepared={prepared:?} settle={settle:?} pack_old={pack_old} again={again:?}"
                ));
            }
            drop(_env);
            let _ = std::fs::remove_dir_all(&root);
        }
        assert!(failures.is_empty(), "HE7 (a):\n  {}", failures.join("\n  "));
    }

    /// HE7 (b): a roll-back completed and SETTLED (renamed to `d.<id>`), then the removal stopped after one backup was
    /// removed with the index still present → the next entry REMOVES it (no second roll-back: a pack file changed since
    /// stays as it is) and the lane accepts an apply again.
    #[test]
    fn t9_r2_settled_journal_half_removed_is_removed_not_rolled_back() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (root, pd, _pre, _env) = t9r2_lane("t9r2-he7b");
        let _ = t9r2_stopped_apply(TxStep::Installed);
        t9_writers_gone();
        let r1 = t9r2_recover(&mut || true, &mut || true, &mut |_| true, &mut || false); // settled, dies before removal
        let (dname, dpath) = t9_slot_entry(&pack_journal_dir()).expect("the settled journal");
        let index_present = dpath.join("index.json").is_file();
        std::fs::remove_file(dpath.join("files").join("bin").join("selfheal.sh")).unwrap(); // half removed
        std::fs::write(pd.join("README.md"), "the user's edit after the recovery\n").unwrap();
        let r2 = recover_pack_journal();
        let readme = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let slot_left = pack_journal_dir().join(JOURNAL_SLOT).exists();
        let again = apply_pack_transactional(&t9r2_items(), "0.15.1", &test_free_state("0.15.1"), None, || Ok(()));
        drop(_env);
        let _ = std::fs::remove_dir_all(&root);
        assert!(r1.is_err(), "the taker must stop after settling: {r1:?}");
        assert!(dname.starts_with("d."), "the settled name: {dname}");
        assert!(index_present, "precondition: the index is still in the settled journal");
        assert_eq!(r2, Ok(true), "the next entry must remove the settled journal");
        assert_eq!(readme, "the user's edit after the recovery\n", "a second roll-back ran");
        assert!(!slot_left, "the slot must be gone");
        assert!(again.is_ok(), "the lane must accept an apply again: {again:?}");
    }

    /// HE7 (c): admission onto an existing NON-EMPTY slot fails (the apply leaves nothing, the slot's entry is untouched);
    /// onto an existing EMPTY slot it behaves as the platform does — unix `rename(2)` replaces an empty directory (the
    /// apply is admitted and completes); windows `windows_move_no_replace` never replaces (refused once, nothing
    /// touched). Both end safe.
    #[test]
    fn t9_r2_admission_onto_a_non_empty_slot_fails_onto_an_empty_slot_as_the_platform() {
        let _g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // non-empty
        let (root, pd, pre, _env) = t9r2_lane("t9r2-he7c-full");
        let foreign = format!("j.{}.ffffffffffffffff", t9r2_me().tag()); // a running writer's journal
        let a = apply_pack_transactional_steps(&t9r2_items(), "0.15.1", &test_free_state("0.15.1"), None, || Ok(()), &mut |s| {
            if s == TxStep::Prepared {
                std::fs::create_dir_all(pack_journal_dir().join(JOURNAL_SLOT).join(&foreign)).unwrap();
            }
            true
        });
        let entry = t9_slot_entry(&pack_journal_dir()).map(|e| e.0);
        let prepared = t9r2_prepared_left();
        let pack_same = m15da_diff(&pre, &m15da_state(&pd)).is_empty();
        drop(_env);
        let _ = std::fs::remove_dir_all(&root);
        assert!(a.is_err() && entry.as_deref() == Some(foreign.as_str()) && prepared.is_empty() && pack_same,
            "non-empty slot: a={a:?} entry={entry:?} prepared={prepared:?} pack_same={pack_same}");
        // empty
        let (root, pd, pre, _env) = t9r2_lane("t9r2-he7c-empty");
        let a = apply_pack_transactional_steps(&t9r2_items(), "0.15.1", &test_free_state("0.15.1"), None, || Ok(()), &mut |s| {
            if s == TxStep::Prepared {
                std::fs::create_dir_all(pack_journal_dir().join(JOURNAL_SLOT)).unwrap();
            }
            true
        });
        let slot_left = pack_journal_dir().join(JOURNAL_SLOT).exists();
        let prepared = t9r2_prepared_left();
        let after = m15da_state(&pd);
        drop(_env);
        let _ = std::fs::remove_dir_all(&root);
        #[cfg(not(windows))]
        assert!(a.is_ok() && !slot_left && prepared.is_empty() && !m15da_diff(&pre, &after).is_empty(),
            "empty slot (unix: admitted): a={a:?} slot_left={slot_left} prepared={prepared:?}");
        #[cfg(windows)]
        assert!(a.is_err() && prepared.is_empty() && m15da_diff(&pre, &after).is_empty(),
            "empty slot (windows: refused, nothing touched): a={a:?} prepared={prepared:?}");
    }

    /// GX1: every lane's journal is its own folder under `.pack-journal.d`, never the legacy shared folder.
    #[test]
    fn t9_each_lane_has_its_own_journal_folder() {
        let root = Path::new("/x/.cys");
        let base = pack_journal_dir_for(&root.join("pack"));
        let d1 = pack_journal_dir_for(&root.join("pack-dept-d1"));
        let d10 = pack_journal_dir_for(&root.join("pack-dept-d10"));
        assert_eq!(base, root.join(".pack-journal.d/pack"));
        assert_eq!(d1, root.join(".pack-journal.d/pack-dept-d1"));
        assert!(base != d1 && d1 != d10 && base != d10);
        assert_eq!(legacy_pack_journal_dir_for(&root.join("pack-dept-d1")), root.join(".pack-journal"));
        assert_eq!(PACK_JOURNAL_ROOT, ".pack-journal.d");
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
        let journal_exists = pack_journal_dir().join(JOURNAL_SLOT).exists();
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

    /// ★T9a (RULINGS GH1) 재작성(종전 `atomic_swap_success_creates_prev`: "성공 교체: staging→pack_dir, 기존 pack_dir→.prev
    /// (1세대 보존), staging 소진"): 게시는 pack_dir 이 없을 때만 — staging 이 rename 하나로 pack_dir 이 되고 .prev 는 생기지
    /// 않는다. 있는 pack_dir 로의 게시는 거부되고 아무것도 바뀌지 않는다(살아 있는 팩은 폴더째 바꾸지 않는다).
    #[test]
    fn atomic_swap_publishes_only_into_an_absent_pack() {
        let base = std::env::temp_dir().join(format!("cys-swap-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("pack");
        let staging = base.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("a.txt"), "new").unwrap();
        std::fs::write(staging.join("b.txt"), "b").unwrap();

        atomic_swap(&dir, &staging).unwrap();

        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "b");
        assert!(!pack_prev_dir(&dir).exists(), ".prev 는 만들지 않는다");
        assert!(!staging.exists(), "staging은 게시로 소진");

        // 있는 pack_dir 로의 게시 → 거부 · 무변경.
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("a.txt"), "newer").unwrap();
        assert!(atomic_swap(&dir, &staging).is_err(), "있는 팩으로의 게시는 거부");
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "new", "팩 무변경");
        assert_eq!(std::fs::read_to_string(staging.join("a.txt")).unwrap(), "newer", "staging 무변경");
        assert!(!pack_prev_dir(&dir).exists(), ".prev 없음");
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

    /// ─── T9a (RULINGS FS · GH1–GH5 · design §4-15 라 · TW1 run 1 report §10): an EXISTING pack is updated in place — no
    /// folder rename, no staging, no `pack.prev`; only a first install (no pack folder) is staged and published by one
    /// rename. The swap's folder renames go through `swap_seam`, so a test can count them.
    mod t9a {
        use super::*;
        use crate::pack::swap_seam;
        use std::cell::RefCell;
        use std::collections::BTreeMap;
        use std::rc::Rc;

        type CallLog = Rc<RefCell<Vec<(PathBuf, PathBuf)>>>;

        const LISTED: &str = "bin/javis_bootstrap.py";
        const USER_OWNED: &str = "agents.json";
        const RETIRED: &str = "bin/javis_retired_tool_t9a.py";
        const MINE: &str = "notes/my-own-notes.txt";
        const USER_BOOT: &str = "# the user's repair of the boot-critical file\n";
        const USER_AGENTS: &str = "{\"user\":\"my seats\"}\n";

        fn scratch(tag: &str) -> PathBuf {
            let base = std::env::temp_dir().join(format!(
                "cys-t9a-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&base).unwrap();
            base
        }

        fn write_tree(root: &Path, files: &[(&str, &str)]) {
            for (rel, body) in files {
                let p = root.join(rel);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, body).unwrap();
            }
        }

        fn embed_of(rel: &str) -> &'static str {
            PACK_ALL.iter().find(|(r, _)| *r == rel).map(|(_, c)| *c).unwrap()
        }

        /// Tree manifest: relative path → kind + sha256 of the bytes (+ permission bits on Unix). Absent root = empty.
        fn manifest(root: &Path) -> BTreeMap<String, String> {
            fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
                for e in std::fs::read_dir(dir).unwrap() {
                    let p = e.unwrap().path();
                    let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                    let meta = std::fs::symlink_metadata(&p).unwrap();
                    #[cfg(unix)]
                    let mode = {
                        use std::os::unix::fs::PermissionsExt;
                        format!(":{:o}", meta.permissions().mode() & 0o7777)
                    };
                    #[cfg(not(unix))]
                    let mode = String::new();
                    if meta.file_type().is_symlink() {
                        out.insert(rel, format!("link:{}", std::fs::read_link(&p).unwrap().display()));
                    } else if meta.is_dir() {
                        out.insert(rel, format!("dir{mode}"));
                        walk(root, &p, out);
                    } else {
                        out.insert(rel, format!("file:{}{mode}", sha256_hex(&std::fs::read(&p).unwrap())));
                    }
                }
            }
            let mut out = BTreeMap::new();
            if std::fs::symlink_metadata(root).is_ok_and(|m| m.is_dir()) {
                walk(root, root, &mut out);
            }
            out
        }

        /// [`manifest`] with the ledger compared by content minus its clock fields (`ts`, `parked_at` — seconds of the
        /// run that wrote the entry); every other byte of the tree is compared as is.
        fn tree(root: &Path) -> BTreeMap<String, String> {
            let mut m = manifest(root);
            if let Ok(s) = std::fs::read_to_string(root.join(MERGE_PENDING_FILE)) {
                let v: serde_json::Value = serde_json::from_str(&s).unwrap();
                let mut entries: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
                for (rel, e) in v.as_object().into_iter().flatten() {
                    let fields = e
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(k, _)| k.as_str() != "ts" && k.as_str() != "parked_at")
                        .map(|(k, x)| (k.clone(), x.to_string()))
                        .collect();
                    entries.insert(rel.clone(), fields);
                }
                m.insert(MERGE_PENDING_FILE.to_string(), format!("ledger:{entries:?}"));
            }
            m
        }

        /// Entry names directly under `base`, sorted — every leftover an install could leave beside the pack.
        fn entries(base: &Path) -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(base)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }

        /// Installs, on this thread, a rename for the swap that records every folder move and performs it.
        fn counting() -> (swap_seam::Guard, CallLog) {
            let log: CallLog = Rc::new(RefCell::new(Vec::new()));
            let seen = log.clone();
            let guard = swap_seam::install(move |src: &Path, dst: &Path| {
                seen.borrow_mut().push((src.to_path_buf(), dst.to_path_buf()));
                std::fs::rename(src, dst)
            });
            (guard, log)
        }

        /// Runs `f` with the pack env (pack dir, config dir, capture root) pointed at `pd` and its siblings.
        fn with_pack<T>(pd: &Path, f: impl FnOnce() -> T) -> T {
            let base = pd.parent().unwrap();
            let _env = set_pack_env(pd, base.join("claude"));
            let _cap = EnvGuard::set("CYS_PACK_CAPTURES_DIR", base.join("captures"));
            let _loc = EnvGuard::remove("CYS_LOCAL_DIR");
            f()
        }

        /// The state an older release leaves at the env's pack dir: a first install (staged, published), then an older
        /// `.pack-version`; six unedited system files spread over the item order whose recorded vendor base (manifest +
        /// `.pristine`) is an older vendor text that is still on disk (→ rewritten); the boot-critical listed file with an
        /// older vendor base and the user's edit (→ vendor-first); the user-owned `agents.json` edited (→ kept); one
        /// embedded file missing (→ re-created); a retired tool the manifest still lists, unedited (→ pruned); a file of
        /// the user's own that no manifest lists (→ untouched). Returns the six rewritten files and the missing one.
        fn older_pack(pd: &Path) -> (Vec<&'static str>, &'static str) {
            crate::pack::install_staged(false, None, false).unwrap();
            assert!(boot_critical_rel(LISTED), "premise: {LISTED} is on the compiled boot-critical list");
            assert!(is_user_owned(USER_OWNED), "premise: {USER_OWNED} is user-owned");
            std::fs::write(pd.join(PACK_VERSION_FILE), "0.1.0").unwrap();
            let mpath = pd.join(INSTALL_MANIFEST);
            let mut m: BTreeMap<String, String> =
                serde_json::from_str(&std::fs::read_to_string(&mpath).unwrap()).unwrap();
            let mut set_base = |rel: &str, old: &str| {
                m.insert(rel.to_string(), content_hash(old));
                let p = pd.join(PRISTINE_DIR).join(rel);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, old).unwrap();
            };
            let sys: Vec<&'static str> = PACK_ALL
                .iter()
                .map(|(r, _)| *r)
                .filter(|r| {
                    r.contains('/')
                        && ownership_name_scoped(r, pd) == "system"
                        && !system_locked(r)
                        && !boot_critical_rel(r)
                })
                .collect();
            assert!(sys.len() >= 12, "premise: enough unlisted system files ({})", sys.len());
            let updates: Vec<&'static str> = (0..6).map(|i| sys[i * (sys.len() - 1) / 5]).collect();
            for rel in &updates {
                let old = format!("# older vendor bytes of {rel}\n");
                std::fs::write(pd.join(rel), &old).unwrap();
                set_base(rel, &old);
            }
            set_base(LISTED, "# older vendor bytes of the boot-critical file\n");
            let missing = sys[sys.len() / 2 + 1];
            assert!(!updates.contains(&missing));
            drop(set_base);
            std::fs::write(pd.join(LISTED), USER_BOOT).unwrap();
            std::fs::write(pd.join(USER_OWNED), USER_AGENTS).unwrap();
            std::fs::remove_file(pd.join(missing)).unwrap();
            let retired = "# a tool an older release shipped\n";
            std::fs::write(pd.join(RETIRED), retired).unwrap();
            m.insert(RETIRED.to_string(), content_hash(retired));
            write_tree(pd, &[(MINE, "my own notes\n")]);
            std::fs::write(&mpath, serde_json::to_string(&m).unwrap()).unwrap();
            (updates, missing)
        }

        /// What the update rules must have produced from [`older_pack`] (the end state of one complete run).
        fn assert_updated(pd: &Path, updates: &[&str], missing: &str) {
            for rel in updates {
                assert_eq!(std::fs::read_to_string(pd.join(rel)).unwrap(), embed_of(rel), "{rel} rewritten");
            }
            assert_eq!(std::fs::read_to_string(pd.join(LISTED)).unwrap(), embed_of(LISTED), "vendor-first: vendor bytes live");
            assert_eq!(
                std::fs::read_to_string(pd.join(format!("{LISTED}.user"))).unwrap(),
                USER_BOOT,
                "vendor-first: the user's bytes parked in <rel>.user"
            );
            assert_eq!(
                load_merge_pending(pd).get(LISTED).and_then(|e| e["kind"].as_str()),
                Some("vendor-first"),
                "vendor-first: the record"
            );
            assert_eq!(std::fs::read_to_string(pd.join(USER_OWNED)).unwrap(), USER_AGENTS, "user-owned edit kept");
            assert_eq!(std::fs::read_to_string(pd.join(missing)).unwrap(), embed_of(missing), "missing file re-created");
            assert!(!pd.join(RETIRED).exists(), "retired unedited tool pruned");
            assert_eq!(std::fs::read_to_string(pd.join(MINE)).unwrap(), "my own notes\n", "the user's own file untouched");
            assert_eq!(
                std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap().trim(),
                env!("CARGO_PKG_VERSION")
            );
        }

        /// GH1 / GH2: `cys init-pack` on an EXISTING pack updates it in place — the swap's seam records ZERO folder
        /// renames (so none touches the pack or `pack.prev`), a `pack.prev` left by an older version stays byte-identical,
        /// no staging is left, and the update rules meet every user edit (the listed file parked vendor-first, the
        /// user-owned file kept). On d01d9188 the staged swap renamed the pack and replaced `pack.prev`.
        #[test]
        fn existing_pack_is_updated_in_place_with_no_folder_rename() {
            let _g = PACK_ENV_LOCK.lock().unwrap();
            let base = scratch("inplace");
            let pd = base.join("pack");
            with_pack(&pd, || {
                let (updates, missing) = older_pack(&pd);
                let prev = pack_prev_dir(&pd);
                write_tree(
                    &prev,
                    &[(LISTED, "# the only copy of an edit an older failed update displaced\n"), (".pack-version", "0.14.56\n")],
                );
                let m_prev = manifest(&prev);

                let (seam, log) = counting();
                let r = crate::pack::install_staged(false, None, false);
                drop(seam);

                r.expect("in-place update of an existing pack");
                assert!(log.borrow().is_empty(), "GH2: no folder rename for an existing pack: {:?}", log.borrow());
                assert_eq!(manifest(&prev), m_prev, "GH2: a pre-existing pack.prev is byte-identical");
                let left: Vec<String> =
                    entries(&base).into_iter().filter(|n| n.starts_with(".pack-staging")).collect();
                assert!(left.is_empty(), "no staging: {left:?}");
                assert_updated(&pd, &updates, missing);
            });
            let _ = std::fs::remove_dir_all(&base);
        }

        /// GH2 / GH4: an update of an existing pack is per file, not all-or-nothing. A run stopped part-way (the stop is
        /// the run's own failure exit — the first file of a folder cannot be created, the same `?` a held file takes on
        /// Windows) has updated exactly the files before that point, deleted nothing and left `.pack-version` old (so the
        /// daemon's install gate stays open); the next run completes it, and the final tree equals one uninterrupted run
        /// — for stops at several points of the item order. On d01d9188 a stopped staged run changed nothing in the pack.
        #[test]
        fn interrupted_in_place_run_is_completed_by_the_next() {
            let _g = PACK_ENV_LOCK.lock().unwrap();
            let base = scratch("interrupt");
            let input = base.join("input").join("pack");
            let (updates, missing) = with_pack(&input, || older_pack(&input));
            let input_paths: Vec<String> = manifest(&input).into_keys().collect();
            let pos = |rel: &str| PACK_ALL.iter().position(|(r, _)| *r == rel).unwrap();

            let reference = base.join("reference").join("pack");
            copy_dir_all(&input, &reference).unwrap();
            with_pack(&reference, || crate::pack::install_staged(false, None, false)).expect("uninterrupted run");
            assert_updated(&reference, &updates, missing);
            let want = tree(&reference);

            for (k, stop_at) in updates.iter().enumerate().skip(1) {
                let folder = Path::new(stop_at).parent().unwrap().to_string_lossy().replace('\\', "/");
                let first = PACK_ALL
                    .iter()
                    .position(|(r, _)| r.starts_with(&format!("{folder}/")))
                    .unwrap();
                let pd = base.join(format!("stop-{k}")).join("pack");
                copy_dir_all(&input, &pd).unwrap();
                // The stop: the folder is moved out of the pack and a plain file takes its name.
                let aside = base.join(format!("aside-{k}"));
                std::fs::rename(pd.join(&folder), &aside).unwrap();
                std::fs::write(pd.join(&folder), "obstacle\n").unwrap();
                let r = with_pack(&pd, || crate::pack::install_staged(false, None, false));
                let err = r.expect_err("the run stops at the first file of the blocked folder");
                assert!(err.contains("cannot create"), "[{folder}] {err}");
                std::fs::remove_file(pd.join(&folder)).unwrap();
                std::fs::rename(&aside, pd.join(&folder)).unwrap();
                assert_eq!(
                    std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap().trim(),
                    "0.1.0",
                    "[{folder}] .pack-version not advanced by a stopped run"
                );
                for rel in &updates {
                    let now = std::fs::read_to_string(pd.join(rel)).unwrap();
                    if pos(rel) < first {
                        assert_eq!(now, embed_of(rel), "[{folder}] {rel} before the stop is updated (in place)");
                    } else {
                        assert_eq!(now, format!("# older vendor bytes of {rel}\n"), "[{folder}] {rel} after the stop untouched");
                    }
                }
                for p in &input_paths {
                    assert!(std::fs::symlink_metadata(pd.join(p)).is_ok(), "[{folder}] nothing deleted by a stopped run: {p}");
                }
                with_pack(&pd, || crate::pack::install_staged(false, None, false)).expect("the next run completes it");
                assert_eq!(tree(&pd), want, "[{folder}] stopped + completed = one uninterrupted run");
            }
            let _ = std::fs::remove_dir_all(&base);
        }

        /// GH3 (1): for the same input, the in-place result equals the old staged result file for file. The old staged
        /// pass (d01d9188 `install_staged` ①–③) is reproduced here — copy of the pack, the per-file install on the copy
        /// with the staged arguments (no authorisation, no config set-up, the pack's capture lane, the pack's sources),
        /// the presence check — and its staging tree (what the swap published) is compared with the in-place tree.
        #[test]
        fn in_place_result_equals_the_old_staged_result() {
            let _g = PACK_ENV_LOCK.lock().unwrap();
            let base = scratch("equal");
            let input = base.join("input").join("pack");
            let (updates, missing) = with_pack(&input, || older_pack(&input));

            let staged_pack = base.join("staged").join("pack");
            copy_dir_all(&input, &staged_pack).unwrap();
            let staging = base.join("staged").join(".pack-staging-init-old");
            with_pack(&staged_pack, || {
                copy_dir_all(&staged_pack, &staging).unwrap();
                let items: Vec<(&str, &str)> = PACK_ALL.iter().map(|(r, c)| (*r, *c)).collect();
                let sources = UpdateSources::for_live_pack(&staged_pack);
                install_into_with(
                    staging.clone(),
                    items.iter().copied(),
                    false,
                    env!("CARGO_PKG_VERSION"),
                    false,
                    false,
                    pack_scope_of(&staged_pack),
                    None,
                    Some("pack"),
                    &sources,
                )
                .unwrap();
                verify_staging_with(&staging, &items, &sources.authorised_absent()).unwrap();
            });

            let in_place = base.join("inplace").join("pack");
            copy_dir_all(&input, &in_place).unwrap();
            with_pack(&in_place, || crate::pack::install_staged(false, None, false)).expect("in place");
            assert_updated(&in_place, &updates, missing);
            assert_eq!(tree(&in_place), tree(&staging), "in-place tree = old staged tree, file for file");
            let _ = std::fs::remove_dir_all(&base);
        }

        /// GH1: a first install (no pack folder) is staged and published by ONE rename staging → pack; no `pack.prev`.
        #[test]
        fn first_install_is_published_by_one_rename() {
            let _g = PACK_ENV_LOCK.lock().unwrap();
            let base = scratch("first");
            let pd = base.join("pack");
            with_pack(&pd, || {
                let (seam, log) = counting();
                let r = crate::pack::install_staged(false, None, false);
                drop(seam);
                r.expect("first install");
                let log = log.borrow();
                assert_eq!(log.len(), 1, "one folder rename: {log:?}");
                assert!(
                    log[0].0.file_name().unwrap().to_string_lossy().starts_with(".pack-staging-init-") && log[0].1 == pd,
                    "staging → pack: {log:?}"
                );
                assert!(pd.join(PACK_VERSION_FILE).is_file());
                assert!(!pack_prev_dir(&pd).exists(), "no pack.prev");
            });
            let _ = std::fs::remove_dir_all(&base);
        }

        /// GN1 (S3): a first install whose publishing rename fails because someone else created the pack meanwhile (the
        /// daemon's start install, another init-pack) copies NOTHING over it — it discards its staging and runs the
        /// in-place branch on the pack that now exists, so every file there meets the update rules: a user's edits made
        /// in it survive and are classified. On 4c75b42e the copy fallback overwrote them with the staging's bytes.
        #[test]
        fn first_install_meeting_a_pack_created_meanwhile_copies_nothing_over_it() {
            let _g = PACK_ENV_LOCK.lock().unwrap();
            let base = scratch("raced");
            let pd = base.join("pack");
            with_pack(&pd, || {
                let user_boot = "# edited in the pack another installer created\n";
                let target = pd.clone();
                let log: CallLog = Rc::new(RefCell::new(Vec::new()));
                let seen = log.clone();
                let seam = swap_seam::install(move |src: &Path, dst: &Path| {
                    seen.borrow_mut().push((src.to_path_buf(), dst.to_path_buf()));
                    if dst == target && std::fs::symlink_metadata(dst).is_err() {
                        // Between the absence check and the rename: another installer publishes the pack in place,
                        // and the user edits two files in it.
                        crate::pack::install(false, None).unwrap();
                        std::fs::write(dst.join(USER_OWNED), USER_AGENTS).unwrap();
                        std::fs::write(dst.join(LISTED), user_boot).unwrap();
                    }
                    std::fs::rename(src, dst)
                });
                let r = crate::pack::install_staged(false, None, false);
                drop(seam);
                r.expect("the raced first install ends as an in-place update of the pack that now exists");
                assert_eq!(std::fs::read_to_string(pd.join(USER_OWNED)).unwrap(), USER_AGENTS, "user-owned edit survives");
                assert_eq!(std::fs::read_to_string(pd.join(LISTED)).unwrap(), user_boot, "listed edit survives");
                assert_eq!(
                    load_merge_pending(&pd).get(LISTED).and_then(|e| e["kind"].as_str()),
                    Some("kept-drift"),
                    "the listed edit met the update rules (vendor not advanced → kept in place, recorded)"
                );
                assert_eq!(log.borrow().len(), 1, "one publishing rename attempt, nothing else: {:?}", log.borrow());
                let left: Vec<String> =
                    entries(&base).into_iter().filter(|n| n.starts_with(".pack-staging")).collect();
                assert!(left.is_empty(), "the staging was discarded: {left:?}");
                assert!(!pack_prev_dir(&pd).exists());
            });
            let _ = std::fs::remove_dir_all(&base);
        }

        /// GN1: the copy fallback (a cross-device staging) onto a destination that is still absent still publishes.
        #[test]
        fn first_install_cross_device_fallback_onto_an_absent_pack_still_publishes() {
            let _g = PACK_ENV_LOCK.lock().unwrap();
            let base = scratch("xdev");
            let pd = base.join("pack");
            with_pack(&pd, || {
                let target = pd.clone();
                let seam = swap_seam::install(move |src: &Path, dst: &Path| {
                    if dst == target {
                        return Err(std::io::Error::other("Invalid cross-device link (os error 18 — injected)"));
                    }
                    std::fs::rename(src, dst)
                });
                let r = crate::pack::install_staged(false, None, false);
                drop(seam);
                r.expect("the copy fallback publishes onto an absent pack");
                for (rel, content) in PACK_ALL.iter().take(40) {
                    assert_eq!(std::fs::read(pd.join(rel)).unwrap(), content.as_bytes(), "{rel}");
                }
                assert_eq!(
                    std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap().trim(),
                    env!("CARGO_PKG_VERSION")
                );
                let left: Vec<String> =
                    entries(&base).into_iter().filter(|n| n.starts_with(".pack-staging")).collect();
                assert!(left.is_empty(), "the staging was consumed: {left:?}");
            });
            let _ = std::fs::remove_dir_all(&base);
        }

        /// GN1: the copy creates every folder and file exclusively — onto an existing destination it stops at once
        /// with `AlreadyExists` and replaces nothing. On 4c75b42e it overwrote the existing file.
        #[test]
        fn the_publish_copy_never_replaces_an_existing_file() {
            let base = scratch("excl");
            let src = base.join("src");
            let dst = base.join("dst");
            write_tree(&src, &[("agents.json", "{\"vendor\":1}"), ("bin/x.py", "x")]);
            write_tree(&dst, &[("agents.json", "{\"user\":1}")]);
            let err = copy_dir_all(&src, &dst).expect_err("an existing destination stops the copy");
            assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists, "{err}");
            assert_eq!(std::fs::read_to_string(dst.join("agents.json")).unwrap(), "{\"user\":1}", "nothing replaced");
            assert!(!dst.join("bin").exists(), "nothing copied");
            let _ = std::fs::remove_dir_all(&base);
        }

        // ─── GN2 — M1, M2, M3: MEASURED properties of the shared per-file install (`install_into_with` — the daemon's
        // start install runs it in place at every update). OUT of T9a's scope (GN0); each test records the state after
        // the sequence, after ONE further install and after ONE further version (printed with `[M#]`), and asserts only
        // the safe outcome it measured: no user byte lost (live, parked or captured). What is not safe-but-wanted (a
        // lost record, a vendor change not applied) is described in the report, not pinned here.

        fn m_install(pd: &Path, items: &[(&str, &str)], version: &str, src: &UpdateSources) {
            install_into_with(
                pd.to_path_buf(), items.iter().copied(), false, version, false, false, PackScope::Base, None, None, src,
            )
            .unwrap();
        }

        fn read_opt(p: &Path) -> Option<String> {
            std::fs::read_to_string(p).ok()
        }

        fn ledger_kind(pd: &Path, rel: &str) -> Option<String> {
            load_merge_pending(pd).get(rel).and_then(|e| e["kind"].as_str()).map(str::to_string)
        }

        fn manifest_hash_of(pd: &Path, rel: &str) -> Option<String> {
            let m: BTreeMap<String, String> =
                serde_json::from_str(&std::fs::read_to_string(pd.join(INSTALL_MANIFEST)).ok()?).ok()?;
            m.get(rel).cloned()
        }

        /// M1: a stop between a merge's writes (the merged bytes on disk, `.pristine` advanced) and its record (ledger
        /// entry, manifest, `.pack-version` — saved after the loop). The stop is reproduced by putting the record files
        /// back as they were before the run (what a stop at that point leaves).
        #[test]
        fn m1_stop_between_a_merge_write_and_its_record_loses_no_user_byte() {
            let _g = PACK_ENV_LOCK.lock().unwrap();
            let base = scratch("m1");
            let pd = base.join("pack");
            std::fs::create_dir_all(&pd).unwrap();
            let cap_root = base.join("captures");
            let _cap = EnvGuard::set("CYS_PACK_CAPTURES_DIR", &cap_root);
            let src = UpdateSources::none();
            let rel = "skills/m1/SKILL.md";
            let e0 = "head-old\ncommon body\n";
            let e1 = "head-new\ncommon body\n";
            let e2 = "head-v2\ncommon body\n";
            let ours = "head-old\ncommon body\nuser-tail-delta\n";
            let merged = "head-new\ncommon body\nuser-tail-delta\n";
            m_install(&pd, &[(rel, e0)], "1.0.0", &src);
            std::fs::write(pd.join(rel), ours).unwrap();
            let record = [INSTALL_MANIFEST, MERGE_PENDING_FILE, PACK_VERSION_FILE];
            let before: Vec<Option<Vec<u8>>> = record.iter().map(|f| std::fs::read(pd.join(f)).ok()).collect();
            m_install(&pd, &[(rel, e1)], "1.1.0", &src);
            assert_eq!(read_opt(&pd.join(rel)).as_deref(), Some(merged), "premise: a clean merge");
            for (f, b) in record.iter().zip(&before) {
                match b {
                    Some(bytes) => std::fs::write(pd.join(f), bytes).unwrap(),
                    None => {
                        let _ = std::fs::remove_file(pd.join(f));
                    }
                }
            }
            let state = |stage: &str| {
                eprintln!(
                    "[M1] {stage}: disk={:?} .new={:?} .user={:?} pristine={:?} manifest_is_e0={} manifest_is_e1={} \
                     manifest_is_e2={} ledger={:?} capture={:?}",
                    read_opt(&pd.join(rel)),
                    read_opt(&pd.join(format!("{rel}.new"))),
                    read_opt(&pd.join(format!("{rel}.user"))),
                    read_opt(&pd.join(PRISTINE_DIR).join(rel)),
                    manifest_hash_of(&pd, rel) == Some(content_hash(e0)),
                    manifest_hash_of(&pd, rel) == Some(content_hash(e1)),
                    manifest_hash_of(&pd, rel) == Some(content_hash(e2)),
                    ledger_kind(&pd, rel),
                    find_capture(&cap_root, rel),
                );
                let live_or_parked = [rel.to_string(), format!("{rel}.user")]
                    .iter()
                    .any(|r| read_opt(&pd.join(r)).is_some_and(|s| s.contains("user-tail-delta")));
                assert!(live_or_parked, "[M1 {stage}] the user's change is live or parked");
                assert_eq!(find_capture(&cap_root, rel).as_deref(), Some(ours), "[M1 {stage}] pre-merge bytes captured");
            };
            state("after the stop");
            m_install(&pd, &[(rel, e1)], "1.1.0", &src);
            state("after one further install");
            m_install(&pd, &[(rel, e2)], "1.2.0", &src);
            state("after one further version");
            let _ = std::fs::remove_dir_all(&base);
        }

        /// M2: two installers with a stale pending map. A parks the user's copy of a boot-critical file (vendor-first)
        /// and records it; B loaded the ledger (and manifest) before A saved them, sees the vendor bytes A wrote, and
        /// saves its own map because another edited file dirtied it. The interleaving is reproduced by running B on
        /// A's files with the ledger and manifest put back as B loaded them.
        #[test]
        fn m2_a_stale_pending_map_keeps_the_parked_user_copy() {
            let _g = PACK_ENV_LOCK.lock().unwrap();
            let base = scratch("m2");
            let pd = base.join("pack");
            std::fs::create_dir_all(&pd).unwrap();
            let _cap = EnvGuard::set("CYS_PACK_CAPTURES_DIR", base.join("captures"));
            let x = "bin/m2-boot.py";
            let y = "skills/m2/y.md";
            let src = UpdateSources::none().with_boot_critical(move |r: &str| r == x);
            let (x0, x1, x2, y0) = ("# x vendor 0\n", "# x vendor 1\n", "# x vendor 2\n", "y vendor 0\n");
            let (ux, uy) = ("# x: the user's repair\n", "y: the user's edit\n");
            m_install(&pd, &[(x, x0), (y, y0)], "1.0.0", &src);
            std::fs::write(pd.join(x), ux).unwrap();
            std::fs::write(pd.join(y), uy).unwrap();
            let stale: Vec<Option<Vec<u8>>> =
                [INSTALL_MANIFEST, MERGE_PENDING_FILE].iter().map(|f| std::fs::read(pd.join(f)).ok()).collect();
            m_install(&pd, &[(x, x1), (y, y0)], "1.1.0", &src); // A
            assert_eq!(ledger_kind(&pd, x).as_deref(), Some("vendor-first"), "premise: A parked and recorded");
            for (f, b) in [INSTALL_MANIFEST, MERGE_PENDING_FILE].iter().zip(&stale) {
                match b {
                    Some(bytes) => std::fs::write(pd.join(f), bytes).unwrap(),
                    None => {
                        let _ = std::fs::remove_file(pd.join(f));
                    }
                }
            }
            m_install(&pd, &[(x, x1), (y, y0)], "1.1.0", &src); // B
            let state = |stage: &str| {
                eprintln!(
                    "[M2] {stage}: x={:?} x.user={:?} ledger(x)={:?} y={:?} ledger(y)={:?}",
                    read_opt(&pd.join(x)),
                    read_opt(&pd.join(format!("{x}.user"))),
                    ledger_kind(&pd, x),
                    read_opt(&pd.join(y)),
                    ledger_kind(&pd, y),
                );
                assert_eq!(read_opt(&pd.join(format!("{x}.user"))).as_deref(), Some(ux), "[M2 {stage}] parked copy on disk");
                assert_eq!(read_opt(&pd.join(y)).as_deref(), Some(uy), "[M2 {stage}] the other edit kept");
            };
            state("after the interleaving");
            m_install(&pd, &[(x, x1), (y, y0)], "1.1.0", &src);
            state("after one further install");
            m_install(&pd, &[(x, x2), (y, y0)], "1.2.0", &src);
            state("after one further version");
            let _ = std::fs::remove_dir_all(&base);
        }

        /// M3: a refused manifest write (Windows: a reader holds `.install-manifest.json`). The non-transactional install
        /// ignores it ("best effort … the next decision falls to PRESERVE") and still writes `.pack-version`. Reproduced by
        /// putting the manifest back as it was before the run. Then the user edits another updated file. Measured: one
        /// further install at the same version (what `cys init-pack` does — the daemon's gate is closed); one further
        /// version from that state (the daemon's path, no init-pack in between). The stale-entry rule reads this binary's
        /// static past-release table, which never lists the bytes this binary itself shipped — so the "next version" leg
        /// here measures the case where the next binary's table does not know them; the case where it does (`Repaired` →
        /// normal update) is the pure branch pinned by `c07_classify_stale_entry_six_branches`.
        #[test]
        fn m3_a_refused_manifest_write_preserves_or_converges() {
            let _g = PACK_ENV_LOCK.lock().unwrap();
            let base = scratch("m3");
            let pd = base.join("pack");
            std::fs::create_dir_all(&pd).unwrap();
            let _cap = EnvGuard::set("CYS_PACK_CAPTURES_DIR", base.join("captures"));
            let src = UpdateSources::none();
            let (a, b) = ("skills/m3/a.md", "skills/m3/b.md");
            let (a0, a1, a2, b0, b1, b2) = ("a0\n", "a1\n", "a2\n", "b0\n", "b1\n", "b2\n");
            let ub = "b: the user's edit after the update\n";
            m_install(&pd, &[(a, a0), (b, b0)], "1.0.0", &src);
            let manifest_before = std::fs::read(pd.join(INSTALL_MANIFEST)).unwrap();
            m_install(&pd, &[(a, a1), (b, b1)], "1.1.0", &src);
            std::fs::write(pd.join(INSTALL_MANIFEST), &manifest_before).unwrap(); // the refused write
            assert!(pack_current_in(&pd, "1.1.0"), "measured: the daemon's install gate is closed after the refusal");
            std::fs::write(pd.join(b), ub).unwrap();
            let after_stop = base.join("after-stop");
            copy_dir_all(&pd, &after_stop).unwrap();
            let state = |pd: &Path, stage: &str| {
                eprintln!(
                    "[M3] {stage}: a={:?} b={:?} b.new={:?} b.user={:?} manifest(a)=a0:{}/a1:{}/a2:{} ledger(b)={:?}",
                    read_opt(&pd.join(a)),
                    read_opt(&pd.join(b)),
                    read_opt(&pd.join(format!("{b}.new"))),
                    read_opt(&pd.join(format!("{b}.user"))),
                    manifest_hash_of(pd, a) == Some(content_hash(a0)),
                    manifest_hash_of(pd, a) == Some(content_hash(a1)),
                    manifest_hash_of(pd, a) == Some(content_hash(a2)),
                    ledger_kind(pd, b),
                );
                let kept = [b.to_string(), format!("{b}.user")].iter().any(|r| read_opt(&pd.join(r)).as_deref() == Some(ub));
                assert!(kept, "[M3 {stage}] the user's edit is live or parked");
            };
            state(&pd, "after the refused write");
            m_install(&pd, &[(a, a1), (b, b1)], "1.1.0", &src);
            state(&pd, "after one further install");
            let next = base.join("next-version");
            copy_dir_all(&after_stop, &next).unwrap();
            m_install(&next, &[(a, a2), (b, b2)], "1.2.0", &src);
            state(&next, "after one further version (from the refused state)");
            assert!(
                [Some(a1), Some(a2)].contains(&read_opt(&next.join(a)).as_deref()),
                "[M3] the unedited file holds a vendor text (kept in place or updated) — nothing else written over it"
            );
            let _ = std::fs::remove_dir_all(&base);
        }

        /// GH1: the swap refuses a destination that exists — nothing is renamed, moved or deleted (the pack, a
        /// `pack.prev` and the staging stay byte-identical). On d01d9188 it moved the pack to `pack.prev` and published.
        #[test]
        fn swap_refuses_an_existing_destination() {
            let base = scratch("refuse");
            let dir = base.join("pack");
            let prev = pack_prev_dir(&dir);
            let staging = base.join(".pack-staging-init-t9a");
            write_tree(&dir, &[("agents.json", "{\"u\":1}"), ("bin/javis_bootstrap.py", "# user\n")]);
            write_tree(&prev, &[("old.txt", "older generation")]);
            write_tree(&staging, &[("bin/javis_bootstrap.py", "# vendor\n")]);
            let (m_dir, m_prev, m_staging) = (manifest(&dir), manifest(&prev), manifest(&staging));
            let (seam, log) = counting();
            let r = atomic_swap(&dir, &staging);
            drop(seam);
            assert!(r.is_err(), "an existing destination is refused: {r:?}");
            assert!(log.borrow().is_empty(), "no rename: {:?}", log.borrow());
            assert_eq!((manifest(&dir), manifest(&prev), manifest(&staging)), (m_dir, m_prev, m_staging));
            let _ = std::fs::remove_dir_all(&base);
        }
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

    /// 신설 → written>0·임베드 반영·.prev 부재. 멱등 재설치 → written=0·pack 온전.
    /// ★T9a (RULINGS GH1) 재작성(종전 이름 `install_staged_fresh_then_idempotent_with_prev`, 종전 단언 "재설치는 .prev 1세대
    /// 보존"): 재설치는 있는 팩의 제자리 갱신이라 .prev 를 만들지 않는다.
    #[test]
    fn install_staged_fresh_then_idempotent_in_place() {
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
        assert!(!pack_prev_dir(&pd).exists(), "재설치는 제자리 갱신 — .prev 를 만들지 않는다(T9a GH1)");
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

    /// ★후계ⓐ(B2 전환 · v2 §6) 의 0.14.55 후속: 벤더 전진 + base 미검증 드리프트는 **출처 불명**이라
    /// 제자리 보존된다(C-07 · 종전 치유 불변을 이 모집단에서 대체).
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
        // ★0.14.55(C-07 · P3) 이 「치유 불변(B2 후계ⓐ · v2 §6)」을 **이 모집단에서 대체한다**.
        //   이 형상은 매니페스트 항목이 디스크와도·조상과도(pristine 제거)·임베드와도 다르고,
        //   디스크 바이트가 내장 과거벤더 표에 없으며, 병합 대기 항목도 없다 = **출처 불명**이다.
        //   근거: HEAD-DISPOSITION-P3-R3.md "What proceeds" 2 + HEAD-RULING-P3-IMPL-C07-NARROW.md
        //   항목 1 — "keep in place + report (no heal, no .user parking, no Merge3)".
        //   ⇒ 종전 치유는 운영자가 쓰지 않은 바이트의 `.user` 백업과 거짓 "healed" 원장 항목을
        //      만들었다(reviewer1 r2 M3). 쓰기 0 이 새 불변이다.
        //   ⛔병합 대기 항목이 있는 형상에서는 pinned 수렴이 그대로 돈다(판정 항목 2 · 별 검체).
        assert_eq!(
            std::fs::read_to_string(pd.join(sys)).unwrap(),
            "SYS-EDIT-XYZ",
            "★C-07(0.14.55): 출처 불명 → 제자리 보존(디스크 바이트 무변경)"
        );
        assert!(
            !pd.join(format!("{sys}.user")).exists(),
            "★C-07: 쓰지 않은 바이트의 .user 백업을 만들지 않는다"
        );
        assert_eq!(
            load_merge_pending(&pd).get(sys).and_then(|e| e["kind"].as_str()),
            None,
            "★C-07: 거짓 healed 원장 항목을 남기지 않는다"
        );
        let _ = sys_embed;

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
        // L4: 벤더 전진 + base 미검증(pristine 제거) — ★0.14.55 부터 제자리 보존(C-07 · 아래 단언 참조).
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
        // L3: Merge3 → 3자 상이 충돌 → ★0.15.0 M11 가(H-RW 재작성 — 종전: "vendor 본 + .user + .base 조상 +
        //   원장 conflicted"): 기동 필수가 아닌 파일이라 **사용자 파일이 살아 있고** vendor 신버전은 `.new`,
        //   원장 user-kept. 이번 시도의 병합 전 사본은 정리된다. (기동 필수 폴백 = vendor 본 + .user + .base +
        //   vendor-first 는 `m11_boot_critical_merge_conflict_vendor_first_keeps_fallback_mechanics` 가 고정.)
        assert_eq!(read(l3), l3_mine, "L3 충돌 폴백 = 사용자 파일 유지(가)");
        assert_eq!(read(&format!("{l3}.new")), embed_of(l3), "L3 vendor 신버전 .new 보관");
        assert!(!dpack.join(format!("{l3}.user")).exists(), "L3 사용자 우선 — .user 미생성");
        assert_eq!(kind(l3).as_deref(), Some("user-kept"), "L3 원장 user-kept");
        // L4: base 미검증 — ★0.14.55(C-07 · P3)부터 **제자리 보존 + 보고**다(종전 healed 가 아니다).
        //   근거: HEAD-DISPOSITION-P3-R3.md "What proceeds" 2 — "unknown disk hash -> keep in place +
        //   report (no heal, no .user parking, no Merge3)". 이 파일은 매니페스트 항목이 디스크와도
        //   조상과도 다르고 과거벤더 표에 행이 없다 = **출처를 말할 수 없다**. 종전 거동은 운영자가
        //   쓰지 않은 바이트의 `.user` 백업과 거짓 "healed" 원장 항목을 만들었다(reviewer1 r2 M3).
        //   ⇒ 이 검체의 기대값 변경은 거동 완화가 아니라 **구속력 있는 판정의 반영**이다.
        assert_eq!(read(l4), "DEPT-DRIFT-L4\n", "L4 제자리 보존(디스크 바이트 무변경)");
        assert!(
            !dpack.join(format!("{l4}.user")).exists(),
            "L4 .user 미생성 — 쓰지 않은 바이트의 백업을 만들지 않는다"
        );
        assert_eq!(kind(l4), None, "L4 원장 무기록 — 거짓 healed 항목을 남기지 않는다");
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
            // ★H-CONC-3 이후 계약 갱신: 종전엔 잔여 tmp 를 **지우고** 그 자리를 열어서 mode 를
            //   맞췄다. 지금은 이름이 호출마다 유일하고 O_EXCL 이라 그 자리를 아예 건드리지
            //   않는다 — 남의 tmp 를 지우는 것은 **진행 중인 남의 쓰기를 파손**하는 경로였다.
            //   그래서 mode 단언에 '남의 tmp 무접촉' 단언을 한 줄 더 건다.
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
            assert_eq!(
                std::fs::read(&tmp).unwrap(),
                b"junk",
                "남의 tmp 를 지우거나 덮어썼다 — 진행 중인 남의 쓰기를 파손하는 경로다"
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

    /// 리포 관례(위 검체들과 동일): temp_dir + 태그·pid — 시작 시 청소, 종료 후 잔존 허용.
    fn conc_dir(tag: &str) -> PathBuf {
        let td = std::env::temp_dir().join(format!("cys-conc3-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        td
    }

    /// ★H-CONC-3 ①(재현 프로브 · 2026-09-05): **같은 프로세스**의 두 writer 가 원자 쓰기를
    /// 겹쳐도 최종 파일은 언제나 **둘 중 하나의 온전한 문서**다.
    ///
    /// ## 무엇이 결함이었나
    /// [`write_atomic_mode`] 의 tmp 이름이 `.{fname}.tmp.{pid}` 로 **pid 당 하나**였다. 프로세스가
    /// 다르면 이름이 갈리므로 다중 **프로세스** 하네스(python 6 writer × 12 iter)는 파손 0 을
    /// 냈다 — 그래서 이 결함은 그 축에서 **보이지 않았다**. 같은 프로세스의 두 호출만
    /// `File::create`(O_TRUNC)로 **같은 inode** 를 공유해 서로를 잘랐고, 소비자가 본 증상이
    /// `Extra data: line 1 column NN` 이다.
    ///
    /// ## 왜 결정론이어야 하는가
    /// 재현을 스레드 스케줄에 맡기면 CI 에서 간헐 적색이 되고, 그 간헐성이 바로 IG-18 판정을
    /// 흐리던 것이다. 그래서 ⓐ는 기제를 **핸들 인터리브로 직접** 재현한다(매 실행 재현).
    #[test]
    fn h_conc_3_same_process_writers_never_tear_the_file() {
        let td = conc_dir("tear");
        let target = td.join("settings.json");
        let long = serde_json::to_vec(&serde_json::json!({"who": "long", "pad": "x".repeat(4000)}))
            .unwrap();
        let short = serde_json::to_vec(&serde_json::json!({"who": "short"})).unwrap();

        // ⓐ 음성 대조군(결정론) — 구 규칙(고정 tmp 이름 + O_TRUNC)의 기제를 그 자리에 재현한다.
        //    이것이 파손을 못 내면 아래 ⓑ의 GREEN 은 **검출력 미확인**이다(측정 실패).
        //
        //    ★일반식 `col = 먼저 착지한 짧은 문서의 길이 + 1` 을 두 길이로 건다. 이 식 자체는
        //    참이고 유용하지만, **CI 에서 관측된 col 44/52 의 출처는 이 코드가 아니다** — 하네스
        //    음성 대조군의 급수(`8N+11`)였다(2026-09-05 W-A 최종 특정 · 위 주석 참조).
        let tear_once = |name: &str, long: &[u8], short: &[u8]| -> serde_json::Error {
            use std::io::Write;
            let dst = td.join(name);
            let fixed = td.join(format!(".{name}.tmp.{}", std::process::id()));
            let mut a = std::fs::File::create(&fixed).unwrap(); // writer A 가 tmp 를 연다
            a.write_all(&long[..long.len() / 2]).unwrap(); //      A 가 앞 절반을 쓴다
            let mut b = std::fs::File::create(&fixed).unwrap(); // B 가 **같은 이름**을 연다(truncate)
            b.write_all(short).unwrap(); //                       B 의 짧은 문서가 앞을 덮는다
            a.write_all(&long[long.len() / 2..]).unwrap(); //      A 는 자기 오프셋에서 꼬리를 마저 쓴다
            std::fs::rename(&fixed, &dst).unwrap(); //             먼저 끝난 쪽이 교체한다
            let torn = std::fs::read(&dst).unwrap();
            assert!(torn.len() > short.len(), "짧은 문서 뒤 꼬리가 없다 — 기제 미재현");
            serde_json::from_slice::<serde_json::Value>(&torn)
                .expect_err("구 규칙 재현이 파손을 못 냈다 — 이 검체는 검출력이 없다(측정 실패)")
        };
        let err = tear_once("settings.json", &long, &short);
        assert!(
            err.to_string().contains("trailing characters"),
            "재현된 파손이 기대한 계급이 아니다(python 소비자의 Extra data 대응): {err}"
        );
        assert_eq!(
            err.column(),
            short.len() + 1,
            "col 이 '먼저 착지한 짧은 문서 길이 + 1' 이 아니다 — 기전 해석이 틀렸다"
        );
        // ★인과 정정(2026-09-05): 아래 43바이트 사례는 **CI 관측의 재현이 아니다.** CI 의 col
        //   44/52 는 하네스 음성 대조군(`naive.json`)의 급수 `8N+11` 에서 나온 것이고(N=4·N=5),
        //   settings.json 은 indent=2 라 그 서명이 원리적으로 불가능하다(W-A 최종 특정).
        //   그래서 이 사례는 '일반식 col = 짧은 문서 길이 + 1' 을 **길이를 바꿔 한 번 더** 거는
        //   경계 검사로만 남긴다 — 이 결함의 근거는 위 ⓑ 2스레드 프로브와 변이 검증이다.
        let short43 = format!("{{\"pad\":\"{}\"}}", "x".repeat(33)).into_bytes();
        assert_eq!(short43.len(), 43, "검체 자기검증 실패 — 43바이트가 아니다");
        assert_eq!(
            tear_once("len43.json", &long, &short43).column(),
            44,
            "43바이트 선착에서 col 이 44 가 아니다 — 일반식이 길이에 따라 깨진다"
        );

        // ⓑ 현행 계약 — 2스레드 경합 중 **어느 시점에 읽어도** 둘 중 하나의 온전한 문서다.
        //    오라클(파싱·동일성)의 검출력은 ⓐ가 이미 증명했다.
        std::fs::write(&target, &long).unwrap();
        let iters = 120usize;
        let reads = std::sync::atomic::AtomicUsize::new(0);
        let tears = std::sync::Mutex::new(Vec::<String>::new());
        std::thread::scope(|sc| {
            sc.spawn(|| {
                for _ in 0..iters {
                    write_atomic(&target, &long).unwrap();
                }
            });
            sc.spawn(|| {
                for _ in 0..iters {
                    write_atomic(&target, &short).unwrap();
                }
            });
            sc.spawn(|| {
                for _ in 0..iters * 4 {
                    let Ok(cur) = std::fs::read(&target) else {
                        continue;
                    };
                    reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if cur != long && cur != short {
                        tears.lock().unwrap().push(format!(
                            "{}B: {}",
                            cur.len(),
                            String::from_utf8_lossy(&cur[..cur.len().min(60)])
                        ));
                    }
                }
            });
        });
        let n = reads.load(std::sync::atomic::Ordering::Relaxed);
        assert!(n > 0, "판독 0회 — 측정 실패다(RAN=0 는 '미적발'이 아니다)");
        let t = tears.lock().unwrap();
        assert!(
            t.is_empty(),
            "원자 쓰기가 찢겼다 {}건 / 판독 {n}회: {:?}",
            t.len(),
            &t[..t.len().min(3)]
        );

        // ⓒ 잔재 0 — tmp 는 rename 으로 사라지고 pid 고정 이름은 더 이상 만들어지지 않는다.
        let left: Vec<String> = std::fs::read_dir(&td)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".tmp-") || n.contains(".tmp."))
            .collect();
        assert!(left.is_empty(), "tmp 잔재: {left:?}");
    }

    /// ★IG-23(2026-09-05 · W-C 23차 요청): **원자 교체와 tmp 잔재**를 플랫폼 중립으로 못박는다.
    ///
    /// 왜 이 검체가 생겼나: H-CONC-3 수리로 tmp 이름이 호출마다 유일해지고 `create_new`(O_EXCL)로
    /// 바뀌면서 "잔여 tmp 를 지우고 그 자리를 연다"는 종전 동작이 사라졌다. 그래서 W-C 가
    /// ①대상이 존재할 때 교체가 성립하는가 ②실패한 쓰기가 tmp 를 남기지 않는가 를 물었다.
    /// 둘 다 **계약**이므로 추정이 아니라 검체로 답한다.
    ///
    /// Windows 시맨틱은 std 소스로 확정했다(추정 아님):
    ///  · `rename` = `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` + ACCESS_DENIED 시
    ///    `FILE_RENAME_FLAG_REPLACE_IF_EXISTS|POSIX_SEMANTICS` 폴백 → 대상 존재·읽기전용 모두 교체.
    ///  · `OpenOptions` 기본 share_mode 에 `FILE_SHARE_DELETE` 포함 → **열린 핸들 상태에서도**
    ///    rename 이 성립하며, 이는 `File::create` 와 `create_new` **양쪽 동일**이라 이번 변경의
    ///    델타가 아니다.
    #[test]
    fn atomic_write_replaces_an_existing_target_and_leaves_no_tmp() {
        let td = conc_dir("residue");
        // ① 대상이 이미 있어도 교체된다(A13 rename 계약).
        let target = td.join("settings.json");
        std::fs::write(&target, b"old-content").unwrap();
        write_atomic(&target, b"new-content").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new-content", "대상 존재 시 교체가 안 됐다");
        let residue = |dir: &std::path::Path| -> Vec<String> {
            std::fs::read_dir(dir)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with(".tmp-") || n.contains(".tmp."))
                .collect()
        };
        assert!(residue(&td).is_empty(), "성공 후 tmp 잔재: {:?}", residue(&td));

        // ② **실패한 쓰기도** 자기 tmp 를 남기지 않는다 — 대상 자리를 디렉터리로 막아 rename 을
        //    결정론으로 실패시킨다(플랫폼 공통: 파일을 디렉터리 위로 rename 할 수 없다).
        let blocked = td.join("blocked.json");
        std::fs::create_dir_all(&blocked).unwrap();
        let err = write_atomic(&blocked, b"x").expect_err("디렉터리 위 교체가 성공했다");
        assert!(residue(&td).is_empty(), "실패 후 tmp 잔재: {:?} (err={err})", residue(&td));
    }

    /// ★H-CONC-3 ②(공용 락): [`merge_desired_hooks`] 가 python preflight 와 **같은**
    /// `<settings>.cys-lock` 을 잡아 read-modify-write 를 직렬화한다.
    ///
    /// 원자 쓰기는 *반쪽 파일*만 막는다 — 두 writer 가 같은 스냅샷을 읽어 각자 append 하면
    /// 나중 쓰기가 앞 append 를 통째로 지운다(lost update). 그 축은 락으로만 닫힌다.
    ///
    /// 오라클: 락을 **먼저 잡아 붙들고 있으면** 병합기는 해제 전에 끝날 수 없다. 같은 창에서
    /// 락을 안 잡는 RMW 를 먼저 재어 이 시간 오라클이 '락 없음'을 실제로 구별함을 보인다 —
    /// 구별 못 하는 오라클로 얻은 GREEN 은 아무것도 증명하지 않는다.
    #[test]
    fn h_conc_3_settings_writers_share_the_preflight_lock() {
        use std::time::{Duration, Instant};
        let td = conc_dir("lock");
        let pack = td.join("pack");
        let settings = td.join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{\"theme\":\"dark\"}").unwrap();
        let hold = Duration::from_millis(150);

        let guard = acquire_settings_lock(&settings).expect("공용 락을 잡지 못했다");
        let t0 = Instant::now();

        // 음성 대조군 — 락을 잡지 않는 RMW 는 보유 창 안에서 끝난다(오라클이 '락 없음'을 구별한다).
        let s2 = settings.clone();
        let naive_took = std::thread::spawn(move || {
            let raw = std::fs::read_to_string(&s2).unwrap();
            let mut v: serde_json::Value = serde_json::from_str(&raw).unwrap();
            v["naive"] = serde_json::json!(1);
            write_atomic(&s2, serde_json::to_string(&v).unwrap().as_bytes()).unwrap();
            t0.elapsed()
        })
        .join()
        .unwrap();
        assert!(
            naive_took < hold,
            "시간 오라클 무효 — 락 없는 RMW 도 보유 창을 넘겼다({naive_took:?} ≥ {hold:?})"
        );

        // 본 판정 — 병합기는 같은 락을 잡으므로 해제 전에 끝날 수 없다.
        let s3 = settings.clone();
        let p3 = pack.clone();
        let merged = std::thread::spawn(move || {
            merge_desired_hooks(&s3, &p3, &AWAKENING_HOOKS).unwrap();
            t0.elapsed()
        });
        std::thread::sleep(hold);
        drop(guard);
        let merge_took = merged.join().unwrap();
        assert!(
            merge_took >= hold,
            "병합기가 공용 락을 잡지 않았다 — 보유 중에 통과했다({merge_took:?} < {hold:?})"
        );

        // 대기만 하고 실패하면 무의미하다 — 해제 후 실제로 병합돼야 한다.
        assert!(
            verify_desired_hooks_registered(&settings, &pack, &AWAKENING_HOOKS).is_empty(),
            "락 대기 후 병합이 적용되지 않았다"
        );
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["theme"], "dark", "사용자 키 소실");
        // 락 보유 중 먼저 착지한 쓰기가 병합기의 RMW 에 삼켜지지 않았다(lost update 0 표식).
        assert_eq!(after["naive"], 1, "선행 쓰기가 병합기 RMW 에 지워졌다(lost update)");
    }
    /// ★(0.14.45 · B1) cysd 부트의 agy 상태줄 조정 미루기 — 표식을 세운 뒤의 설치 경로 호출은 '미뤄 둔 조정 있음' 만 남기고
    /// 돌아오며, 바인드 뒤 소비는 **한 번**만 돈다(두 번째는 false). 표식이 없으면 소비도 false(설치가 돌지 않은 부트).
    /// 테스트 빌드의 본체는 실 HOME 을 만지지 않으므로(`cfg!(test)` 조기 반환) 여기서 안전하게 흐름만 잰다.
    #[test]
    fn agy_statusline_reconcile_is_deferred_and_consumed_once() {
        assert!(!run_deferred_agy_statusline_reconcile(), "미뤄 둔 것이 없으면 false");
        defer_agy_statusline_probe();
        reconcile_agy_statusline_at_install(); // 설치 경로 — 표식만 남긴다
        reconcile_agy_statusline_at_install(); // 두 번 와도 표식은 하나
        assert!(run_deferred_agy_statusline_reconcile(), "바인드 뒤 소비 1회");
        assert!(!run_deferred_agy_statusline_reconcile(), "두 번째 소비는 없다");
        // 소비가 표식을 내렸으므로 그 뒤 설치 경로는 종전처럼 동기다(표식 미설정 → pending 도 서지 않는다).
        reconcile_agy_statusline_at_install();
        assert!(!run_deferred_agy_statusline_reconcile());
        // 표식만 내리는 동기 경로 — 그 뒤 설치는 pending 을 세우지 않는다(스레드 생성 실패에도 영영 미뤄지지 않는다).
        defer_agy_statusline_probe();
        end_agy_statusline_deferral();
        reconcile_agy_statusline_at_install();
        assert!(!run_deferred_agy_statusline_reconcile());
    }

    // ── F10 (0.14.50): hooks/ 드리프트 분류기 · 보고 (시뮬레이션 SIM-03 네 번의 실행을 표로 박제) ──
    fn f10_pack(tag: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("cys-f10-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (rel, body) in files {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        root
    }
    fn f10_hashes(files: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        files.iter().map(|(k, v)| (k.to_string(), content_hash(v))).collect()
    }
    fn f10_states(rows: &[HookRow]) -> Vec<(String, &'static str)> {
        rows.iter().map(|r| (r.rel.clone(), r.state.as_str())).collect()
    }

    #[test]
    fn f10_classifier_truth_table_and_unreadable_wins_over_vendor() {
        use DiskRead::*;
        let h = |s: &str| Hash(s.to_string());
        assert_eq!(classify_hook(&Missing, Some("m"), Some("v")), HookState::Missing);
        assert_eq!(classify_hook(&Unreadable, Some("m"), Some("v2")), HookState::Unreadable, "판독 불가가 vendor 전진보다 먼저");
        assert_eq!(classify_hook(&h("v"), Some("m"), Some("v")), HookState::Same);
        assert_eq!(classify_hook(&h("x"), Some("m"), None), HookState::Extra);
        assert_eq!(classify_hook(&h("x"), Some("m"), Some("m")), HookState::Drift, "고침 + vendor 미전진");
        assert_eq!(classify_hook(&h("x"), Some("m"), Some("v")), HookState::DriftBehind, "고침 + vendor 전진");
        assert_eq!(classify_hook(&h("m"), Some("m"), Some("v")), HookState::Behind, "안 고침 + vendor 전진");
        assert_eq!(classify_hook(&h("x"), None, Some("v")), HookState::Drift, "매니페스트 항목이 없으면 고친 것으로 보되(edited) 전진 여부는 판단 못 한다(advanced 는 항목이 있을 때만) — 모형 규칙");
        for st in [HookState::Same, HookState::Behind, HookState::Extra] {
            assert!(!st.counted(), "{st:?}");
        }
        for st in [HookState::Drift, HookState::DriftBehind, HookState::Missing, HookState::Unreadable] {
            assert!(st.counted(), "{st:?}");
        }
    }

    #[test]
    fn f10_planted_copy_four_runs_counts_0_3_3_4() {
        let vend_a = [("hooks/guard.sh", "g\n"), ("hooks/_lib.sh", "l\n"), ("hooks/inject-context.sh", "i1\n"), ("hooks/save-state.sh", "s\n")];
        let man = f10_hashes(&vend_a);
        let vendor_a = man.clone();
        // 대조군: 고치지 않은 사본 → 0행.
        let ctl = f10_pack("ctl", &[("hooks/guard.sh", b"g\n"), ("hooks/_lib.sh", b"l\n"), ("hooks/inject-context.sh", b"i1\n"), ("hooks/save-state.sh", b"s\n")]);
        assert!(hooks_report(&ctl, &man, &vendor_a).is_empty(), "대조군은 0행");
        // 심기: guard.sh 수정 · _lib.sh 삭제 · mine.sh 추가 · save-state.sh 를 비 UTF-8 로(어느 OS 에서도 만들 수 있는 '판독 불가').
        let pl = f10_pack("pl", &[("hooks/guard.sh", b"EDITED\n"), ("hooks/inject-context.sh", b"i1\n"), ("hooks/save-state.sh", &[0xffu8, 0xfe, 0x00]), ("hooks/mine.sh", b"m\n")]);
        let r2 = hooks_report(&pl, &man, &vendor_a);
        assert_eq!(
            f10_states(&r2),
            vec![
                ("hooks/_lib.sh".into(), "missing"),
                ("hooks/guard.sh".into(), "drift"),
                ("hooks/mine.sh".into(), "extra"),
                ("hooks/save-state.sh".into(), "unreadable"),
            ]
        );
        assert_eq!(r2.iter().filter(|r| r.state.counted()).count(), 3, "extra 는 세지 않는다");
        // 실행 3: 같은 사본, vendor B 에서 inject-context.sh 만 전진 → behind 행이 늘되 세지 않고, 판독 불가 행은 남는다.
        let mut vendor_b = vendor_a.clone();
        vendor_b.insert("hooks/inject-context.sh".into(), content_hash("i2\n"));
        let r3 = hooks_report(&pl, &man, &vendor_b);
        assert_eq!(r3.len(), 5);
        assert!(r3.iter().any(|r| r.rel == "hooks/inject-context.sh" && r.state == HookState::Behind));
        assert!(r3.iter().any(|r| r.rel == "hooks/save-state.sh" && r.state == HookState::Unreadable), "다른 파일의 vendor 변경으로 판독 불가 행이 사라지지 않는다(T10-7 a)");
        assert_eq!(r3.iter().filter(|r| r.state.counted()).count(), 3);
        // 실행 4: 합친 행 — inject-context.sh 도 판독 불가이면서 vendor 가 전진: 'behind' 가 아니라 'unreadable', 세어진다(T10-7 b).
        let pl2 = f10_pack("pl2", &[("hooks/guard.sh", b"EDITED\n"), ("hooks/inject-context.sh", &[0xffu8, 0xfe]), ("hooks/save-state.sh", &[0xffu8, 0xfe, 0x00]), ("hooks/mine.sh", b"m\n")]);
        let r4 = hooks_report(&pl2, &man, &vendor_b);
        assert!(r4.iter().any(|r| r.rel == "hooks/inject-context.sh" && r.state == HookState::Unreadable));
        assert_eq!(r4.iter().filter(|r| r.state.counted()).count(), 4);
        for d in [ctl, pl, pl2] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn f10_backslash_manifest_keys_give_the_same_rows_as_slash_keys() {
        let vend = [("hooks/guard.sh", "g\n")];
        let vendor = f10_hashes(&vend);
        let slash = vendor.clone();
        let back: std::collections::BTreeMap<String, String> = vendor.iter().map(|(k, v)| (k.replace('/', "\\"), v.clone())).collect();
        let pack = f10_pack("bs", &[("hooks/guard.sh", b"EDITED\n")]);
        let a = hooks_report(&pack, &slash, &vendor);
        let b = hooks_report(&pack, &back, &vendor);
        assert_eq!(f10_states(&a), f10_states(&b), "역슬래시 키도 같은 행");
        assert_eq!(f10_states(&a), vec![("hooks/guard.sh".to_string(), "drift")]);
        assert_eq!(normalize_manifest_key(r"hooks\sub\x.sh"), "hooks/sub/x.sh");
        let _ = std::fs::remove_dir_all(pack);
    }

    #[test]
    fn f10_report_writes_nothing_and_creates_no_directory() {
        let pack = f10_pack("ro", &[("hooks/guard.sh", b"EDITED\n")]);
        let before = f10_listing(&pack);
        let _ = hooks_report(&pack, &f10_hashes(&[("hooks/guard.sh", "g\n"), ("hooks/gone.sh", "x\n")]), &f10_hashes(&[("hooks/guard.sh", "g\n")]));
        assert_eq!(f10_listing(&pack), before);
        // 팩 폴더 자체가 없어도 만들지 않는다.
        let absent = std::env::temp_dir().join(format!("cys-f10-absent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&absent);
        let rows = hooks_report(&absent, &f10_hashes(&[("hooks/gone.sh", "x\n")]), &std::collections::BTreeMap::new());
        assert_eq!(f10_states(&rows), vec![("hooks/gone.sh".to_string(), "missing")]);
        assert!(!absent.exists());
        let _ = std::fs::remove_dir_all(pack);
    }
    fn f10_listing(root: &Path) -> Vec<String> {
        let mut v = Vec::new();
        walk_hooks_dir(root, root, &mut v);
        v.sort();
        v
    }

    /// (이름은 F10 당시의 것 — 0.15.0 M2 에서 계약이 바뀌었다: 안내는 밖으로 보내는 제안 경로 대신 수리 장부를 가리킨다.)
    #[test]
    fn f10_guidance_names_the_local_folder_and_the_existing_propose_route() {
        for needle in [
            "~/.cys/local/hooks/<이벤트>.d/",
            "cys repair begin --files <rel> --trigger <계기>(owner-order · grant · update 중 하나)",
            "리뷰어1의 검증과 리뷰어2의 감사 서명을 받은 뒤 cys repair submit",
            "cys repair record",
            "repairs/R-<n>/staging/",
            "cys repair submit",
        ] {
            assert!(HOOKS_LOCAL_GUIDANCE.contains(needle), "{needle}");
        }
        let (begin, record) = (
            HOOKS_LOCAL_GUIDANCE.find("cys repair begin").unwrap(),
            HOOKS_LOCAL_GUIDANCE.find("cys repair record").unwrap(),
        );
        assert!(
            begin < record,
            "begin 은 고치기 전(원본 보관) — record 보다 먼저 가르친다(RULINGS AH1)"
        );
        assert!(
            !HOOKS_LOCAL_GUIDANCE.contains('|'),
            "no shell pipe in a runnable line (AM2)"
        );
        for old in [["--pro", "pose"].concat(), ["제안", " 패치"].concat()] {
            assert!(
                !HOOKS_LOCAL_GUIDANCE.contains(&old),
                "옛 제안 경로가 남았다: {old}"
            );
        }
        assert!(!HOOKS_LOCAL_GUIDANCE.contains("pack-drift"), "만들지 않은 명령을 약속하지 않는다");
        assert!(!vendor_hook_hashes().is_empty(), "임베드 팩에 hooks/ 가 있다");
    }
    // ── F10 R2 minors (리뷰 2회차): r2-m2 불완전 순회 · r2-m3 읽는 동안 자란 파일 ──
    #[test]
    fn f10_r2m3_a_file_that_grows_past_the_cap_while_being_read_is_rejected_not_hashed_as_a_prefix() {
        // 정확히 상한: 통과. 상한 + 1: 거부(앞 상한 바이트만 해시한 '성공'이 아니다).
        let at_cap = vec![b'a'; 16];
        assert_eq!(read_capped_utf8(&at_cap[..], 16).unwrap().len(), 16);
        let over = vec![b'a'; 17];
        let e = read_capped_utf8(&over[..], 16).expect_err("상한 + 1 바이트는 거부");
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
        // 비 UTF-8 은 종전처럼 오류.
        assert!(read_capped_utf8(&[0xffu8, 0xfe][..], 16).is_err());
    }

    #[test]
    fn f10_r2m2_depth_cut_entry_cut_and_wide_directory_trees_are_reported_incomplete_never_clean() {
        let root = std::env::temp_dir().join(format!("cys-f10-r2m2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // 대조군: 얕고 작은 나무는 불완전이 아니다.
        let ctl = root.join("ctl");
        std::fs::create_dir_all(ctl.join("hooks/sub")).unwrap();
        std::fs::write(ctl.join("hooks/a.sh"), b"a").unwrap();
        std::fs::write(ctl.join("hooks/sub/b.sh"), b"b").unwrap();
        let mut out = Vec::new();
        assert_eq!(walk_hooks_dir_checked(&ctl.join("hooks"), &ctl, &mut out), None, "대조군은 완전");
        assert_eq!(out.len(), 2);
        // 팩에 hooks/ 자체가 없는 경우도 불완전이 아니다(설치된 적 없음).
        let mut out = Vec::new();
        assert_eq!(walk_hooks_dir_checked(&root.join("nope/hooks"), &root.join("nope"), &mut out), None);
        // 깊이 상한: 상한보다 깊은 폴더.
        let deep = root.join("deep");
        let mut d = deep.join("hooks");
        for i in 0..(HOOKS_WALK_MAX_DEPTH + 3) {
            d = d.join(format!("d{i}"));
        }
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("x.sh"), b"x").unwrap();
        let mut out = Vec::new();
        let inc = walk_hooks_dir_checked(&deep.join("hooks"), &deep, &mut out).expect("깊이 초과는 불완전");
        assert!(inc.depth_cut >= 1 && !inc.entries_cut, "{inc:?}");
        // 항목 수 상한: 디렉터리만으로도 센다(파일 0개).
        let wide = root.join("wide");
        std::fs::create_dir_all(wide.join("hooks")).unwrap();
        for i in 0..(HOOKS_WALK_MAX_ENTRIES + 5) {
            std::fs::create_dir(wide.join("hooks").join(format!("e{i}"))).unwrap();
        }
        let mut out = Vec::new();
        let inc = walk_hooks_dir_checked(&wide.join("hooks"), &wide, &mut out).expect("디렉터리만 많아도 불완전");
        assert!(inc.entries_cut, "{inc:?}");
        assert!(out.is_empty(), "파일은 하나도 없다 — 종전 셈(파일만)이면 상한에 닿지 않았다");
        // 보고 단계로 전파된다.
        let scan = hooks_report_scan(&wide, &std::collections::BTreeMap::new(), &std::collections::BTreeMap::new());
        assert!(scan.incomplete.as_ref().is_some_and(|i| i.entries_cut));
        assert!(scan.incomplete.unwrap().describe().contains("항목 수 상한"));
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── F10 M2 (리뷰 1회차): hooks/ 아래 비정규 파일(FIFO) · 심링크가 보고를 멈추게 하면 안 된다 ──
    #[cfg(unix)]
    fn f10_bounded<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(std::time::Duration::from_secs(secs)).ok()
    }
    #[cfg(unix)]
    fn f10_mkfifo(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let st = std::process::Command::new("/usr/bin/mkfifo").arg(p).status().expect("mkfifo");
        assert!(st.success());
    }
    /// 시한 안에 못 끝나면 실패한다 — 시험 자체는 멈추지 않는다(막힌 스레드는 쓰기 쪽을 열어 풀어 준다).
    #[cfg(unix)]
    fn f10_release_fifo(p: &Path) {
        let _ = std::fs::OpenOptions::new().write(true).custom_flags_nonblock().open(p);
    }
    #[cfg(unix)]
    trait NonBlockOpen { fn custom_flags_nonblock(&mut self) -> &mut Self; }
    #[cfg(unix)]
    impl NonBlockOpen for std::fs::OpenOptions {
        fn custom_flags_nonblock(&mut self) -> &mut Self {
            use std::os::unix::fs::OpenOptionsExt;
            self.custom_flags(libc::O_NONBLOCK) // 플랫폼 상수(macOS 0x4 · Linux 0x800): 읽는 쪽이 대기 중이면 열려서 풀어 준다
        }
    }

    #[cfg(unix)]
    #[test]
    fn f10_m2_unmanifested_fifo_under_hooks_is_reported_without_blocking() {
        let pack = f10_pack("fifo", &[("hooks/guard.sh", b"g\n")]);
        let fifo = pack.join("hooks/local.pipe");
        f10_mkfifo(&fifo);
        let man = f10_hashes(&[("hooks/guard.sh", "g\n")]);
        let (p2, m2) = (pack.clone(), man.clone());
        let got = f10_bounded(5, move || hooks_report(&p2, &m2, &m2));
        if got.is_none() {
            f10_release_fifo(&fifo);
        }
        let rows = got.expect("FIFO 에서 보고가 5초 안에 끝나지 않았다(블로킹 읽기)");
        assert_eq!(f10_states(&rows), vec![("hooks/local.pipe".to_string(), "unreadable")], "비정규 파일은 unreadable(미지원)로 보고한다");
        assert!(rows[0].state.counted());
        let _ = std::fs::remove_dir_all(pack);
    }

    #[cfg(unix)]
    #[test]
    fn f10_m2_manifested_fifo_symlink_to_fifo_and_symlink_loops_are_bounded() {
        let pack = f10_pack("fifo2", &[("hooks/guard.sh", b"g\n")]);
        let fifo = pack.join("hooks/listed.pipe");
        f10_mkfifo(&fifo);
        std::os::unix::fs::symlink(&fifo, pack.join("hooks/via-link")).unwrap();
        std::os::unix::fs::symlink(pack.join("hooks/loop-b"), pack.join("hooks/loop-a")).unwrap();
        std::os::unix::fs::symlink(pack.join("hooks/loop-a"), pack.join("hooks/loop-b")).unwrap();
        std::os::unix::fs::symlink(pack.join("hooks"), pack.join("hooks/dir-loop")).unwrap();
        let mut man = f10_hashes(&[("hooks/guard.sh", "g\n")]);
        man.insert("hooks/listed.pipe".into(), content_hash("x"));
        let (p2, m2) = (pack.clone(), man.clone());
        let got = f10_bounded(5, move || hooks_report(&p2, &m2, &m2));
        if got.is_none() {
            f10_release_fifo(&fifo);
        }
        let rows = got.expect("FIFO · 심링크 순환에서 보고가 5초 안에 끝나지 않았다");
        let st: std::collections::BTreeMap<_, _> = f10_states(&rows).into_iter().collect();
        assert_eq!(st.get("hooks/listed.pipe"), Some(&"unreadable"));
        assert_eq!(st.get("hooks/via-link"), Some(&"unreadable"), "FIFO 를 가리키는 심링크도 열지 않는다");
        assert_eq!(st.get("hooks/loop-a"), Some(&"unreadable"));
        assert!(!st.keys().any(|k| k.starts_with("hooks/dir-loop/")), "디렉터리 심링크는 따라가지 않는다(순환 없음): {st:?}");
        let _ = std::fs::remove_dir_all(pack);
    }

}

/// ★0.15.0 「스스로 해결」 M11 — 업데이트 규칙의 회귀 시험(설계 §4-11 · §6-11 · CONTRACTS C8 · testplan H §6-11).
/// 각 시험은 그 안전장치를 없애면 적색이 되도록 짰다(통과 조건 H). 임시 팩만 쓰고(라이브 무접촉), 묘비·
/// 기동 필수·과거 vendor 표는 입력원 이음새(`UpdateSources::with_*`)로 주입한다.
#[cfg(test)]
mod m11_tests {
    use super::*;
    use crate::repair::tombstones::{Tombstone, TombstoneKind};
    use crate::repair::types::RepairId;

    fn tomb(rel: &str, repair: &str) -> Tombstone {
        Tombstone {
            rel: rel.to_string(),
            kind: TombstoneKind::File,
            repair: RepairId::parse(repair).unwrap_or_else(|_| {
                serde_json::from_value(serde_json::json!(repair)).expect("repair id")
            }),
        }
    }

    fn hook_tomb(event: &str, script: &str, repair: &str) -> Tombstone {
        Tombstone {
            rel: format!("settings:{event}:{script}"),
            kind: TombstoneKind::Hook { event: event.into(), script: script.into() },
            repair: serde_json::from_value(serde_json::json!(repair)).expect("repair id"),
        }
    }

    /// 임시 팩 루트(`<base>/pack`) + 캡처 루트 격리(`CYS_PACK_CAPTURES_DIR`) — 반환된 가드가 살아 있는 동안만.
    struct Rig {
        base: PathBuf,
        pack: PathBuf,
        _g: std::sync::MutexGuard<'static, ()>,
        _cap: EnvGuard,
    }

    impl Rig {
        fn new(tag: &str) -> Rig {
            let g = PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let base = std::env::temp_dir().join(format!(
                "cys-m11-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&base);
            let pack = base.join("pack");
            std::fs::create_dir_all(&pack).unwrap();
            let cap = EnvGuard::set("CYS_PACK_CAPTURES_DIR", base.join("captures"));
            Rig { base, pack, _g: g, _cap: cap }
        }
        fn write(&self, rel: &str, content: &[u8]) {
            let p = self.pack.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        fn read(&self, rel: &str) -> Option<String> {
            std::fs::read_to_string(self.pack.join(rel)).ok()
        }
        fn manifest(&self, entries: &[(&str, &str)]) {
            let m: std::collections::BTreeMap<String, String> = entries
                .iter()
                .map(|(r, c)| (r.to_string(), content_hash(c)))
                .collect();
            std::fs::write(self.pack.join(INSTALL_MANIFEST), serde_json::to_string(&m).unwrap())
                .unwrap();
        }
        fn pristine(&self, rel: &str, content: &str) {
            let p = self.pack.join(PRISTINE_DIR).join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        fn kind(&self, rel: &str) -> Option<String> {
            load_merge_pending(&self.pack)
                .get(rel)
                .and_then(|e| e.get("kind"))
                .and_then(|k| k.as_str())
                .map(str::to_string)
        }
        fn entry(&self, rel: &str) -> Option<serde_json::Value> {
            load_merge_pending(&self.pack).get(rel).cloned()
        }
        fn install(&self, items: &[(&str, &str)], force: bool, sources: &UpdateSources) {
            install_into_with(
                self.pack.clone(),
                items.iter().copied(),
                force,
                "9.9.9",
                false,
                false,
                PackScope::Base,
                None,
                None,
                sources,
            )
            .expect("install_into_with");
        }
        fn plan(&self, items: &[(&str, &str)], force: bool, sources: &UpdateSources) -> InstallPlan {
            plan_install_with(&self.pack, items, force, "9.9.9", sources)
        }
        fn capture_files(&self) -> usize {
            let mut n = 0;
            let mut stack = vec![self.base.join("captures")];
            while let Some(d) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    if e.path().is_dir() {
                        stack.push(e.path());
                    } else {
                        n += 1;
                    }
                }
            }
            n
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            #[cfg(unix)]
            {
                // 권한 시험이 남긴 0555 디렉터리도 지울 수 있게.
                use std::os::unix::fs::PermissionsExt;
                let mut stack = vec![self.base.clone()];
                while let Some(d) = stack.pop() {
                    let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755));
                    if let Ok(rd) = std::fs::read_dir(&d) {
                        for e in rd.flatten() {
                            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                stack.push(e.path());
                            }
                        }
                    }
                }
            }
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    fn bc_only(rels: &'static [&'static str]) -> impl Fn(&str) -> bool + Send + Sync + 'static {
        move |r: &str| rels.contains(&r)
    }

    /// O-C8-2 순수 분류기 — 입력 갈래 전수(ruled 결과표). 각 행이 계약의 한 문장이다.
    #[test]
    fn m11_classify_update_matrix() {
        use UpdateClass::*;
        let v = "VENDOR";
        let d = "DISK";
        let s = Ownership::System;
        let u = Ownership::User;
        // 판독 불가 → 지금 동작(가-2).
        assert_eq!(classify_update("bin/x.py", s, false, ManifestBase::None, None, v, false, false, false), KeepAsToday);
        assert_eq!(classify_update("bin/x.py", s, true, ManifestBase::None, Some(d), v, false, false, false), KeepAsToday);
        // 이미 vendor 본.
        assert_eq!(classify_update("bin/x.py", s, true, ManifestBase::None, Some(v), v, false, false, true), KeepAsToday);
        // system: 설치 당시 해시 = 비수정 → 갱신.
        assert_eq!(classify_update("bin/x.py", s, true, ManifestBase::Some(d), Some(d), v, false, false, true), VendorUpdate);
        // system: vendor 미전진 → kept-drift(지금 동작).
        assert_eq!(classify_update("bin/x.py", s, true, ManifestBase::Some(v), Some(d), v, false, false, true), KeepAsToday);
        // system: 기록 없음/깨짐 + 과거 vendor 본 → 갱신(다 — 사용자 수정 판정 전에 표부터).
        assert_eq!(classify_update("bin/x.py", s, false, ManifestBase::None, Some(d), v, true, false, true), VendorUpdate);
        assert_eq!(classify_update("bin/x.py", s, true, ManifestBase::Corrupt, Some(d), v, true, false, true), VendorUpdate);
        // system: 기록이 있는데 다른 해시 + 과거 vendor 일치 → 표는 기록 부재일 때만(크래시 창 좁힘 보존).
        assert_eq!(classify_update("bin/x.py", s, false, ManifestBase::Some("OLD"), Some(d), v, true, false, true), UserWins { park_new: true });
        // system: 그 밖의 수정본 → 가/나.
        assert_eq!(classify_update("bin/x.py", s, false, ManifestBase::None, Some(d), v, false, false, true), UserWins { park_new: true });
        assert_eq!(classify_update("bin/x.py", s, true, ManifestBase::None, Some(d), v, false, false, true), VendorFirst { park_user_ts: true });
        assert_eq!(classify_update("bin/x.py", s, false, ManifestBase::Corrupt, Some(d), v, false, false, true), UserWins { park_new: true });
        assert_eq!(classify_update("bin/x.py", s, true, ManifestBase::Some("OLD"), Some(d), v, false, false, true), VendorFirst { park_user_ts: true });
        // 잠금 보안 자산은 이 규칙 밖(L0 지금 동작).
        assert_eq!(classify_update("trusted-keys.json", s, false, ManifestBase::None, Some(d), v, false, false, true), KeepAsToday);
        // user-owned: 비수정(기록 해시 일치) → 갱신(바 가드 1).
        assert_eq!(classify_update("acl.json", u, false, ManifestBase::Some(d), Some(d), v, false, false, true), VendorUpdate);
        // user-owned: 과거 vendor 본과 같음 → 갱신(가드 1 — 그 파일 자신의 과거 해시).
        assert_eq!(classify_update("acl.json", u, false, ManifestBase::None, Some(d), v, true, false, true), VendorUpdate);
        // user-owned: 감사 '내 것 유지'·'되돌림' → 건너뜀(가드 2).
        assert_eq!(classify_update("acl.json", u, false, ManifestBase::Some(d), Some(d), v, false, true, true), KeepAsToday);
        // user-owned: 고친 파일 → 지금 동작(.new).
        assert_eq!(classify_update("acl.json", u, false, ManifestBase::Some("OLD"), Some(d), v, false, false, true), KeepAsToday);
        // seed-once: 이 규칙 밖.
        assert_eq!(classify_update("memory/MEMORY.md", Ownership::SeedOnce, true, ManifestBase::None, Some(d), v, true, false, true), KeepAsToday);
    }

    /// '계획 = 실제'(설계 §6-11 가 결과 모델 5곳 · H-RW-11 의 신 kind 확장): 같은 입력원으로 plan 과 실제 설치가
    /// 한 파일도 다르지 않다 — 사용자 우선 보관, 기동 필수 vendor 우선, 묘비 부재 유지, 생명줄 복원, 과거 vendor
    /// 갱신, 바 자동 갱신과 그 가드, 판독 불가 보존·백업.
    #[test]
    fn m11_plan_equals_actual_for_update_rules() {
        let rig = Rig::new("plan");
        let items: Vec<(&str, &str)> = vec![
            ("bin/general.py", "VENDOR-GENERAL-2\n"),
            ("hooks/bootdep.sh", "#!/bin/bash\necho vendor-boot-2\n"),
            ("bin/tomb.py", "VENDOR-TOMB\n"),
            ("hooks/session-start.sh", "#!/bin/bash\necho session-start\n"),
            ("bin/prev.py", "VENDOR-PREV-2\n"),
            ("acl.json", "{\"rules\": [2]}\n"),
            ("agents.json", "{\"agents\": 2}\n"),
            ("bin/cp949.py", "VENDOR-CP949\n"),
            ("soul.md", "# soul vendor 2 — 헌법\n"),
            ("bin/new.py", "VENDOR-NEW\n"),
        ];
        rig.write("bin/general.py", b"USER-EDIT-GENERAL\n");
        rig.write("hooks/bootdep.sh", b"#!/bin/bash\necho USER-BOOT\n");
        rig.write("bin/prev.py", b"OLD-RELEASED-PREV\n");
        rig.write("acl.json", b"{\"rules\": [1]}\n");
        rig.write("agents.json", b"{\"agents\": 1}\n");
        rig.write("bin/cp949.py", &[0xC7, 0xD1, 0xB1, 0xDB, 0x0A]);
        rig.write("soul.md", &[0xC7, 0xD1, 0x0A]);
        // bin/tomb.py 와 hooks/session-start.sh 는 부재(둘 다 묘비 — 생명줄은 걸러진다).
        rig.manifest(&[("acl.json", "{\"rules\": [1]}\n"), ("agents.json", "{\"agents\": 1}\n")]);
        // 바 가드 2: agents.json 은 사용자가 일부러 옛 버전을 유지했다.
        append_merge_audit(
            &rig.pack,
            &serde_json::json!({"file": "agents.json", "action": "keep-mine", "flags": []}),
        )
        .unwrap();
        let sources = || {
            UpdateSources::none()
                .with_tombstones(vec![
                    tomb("bin/tomb.py", "R-1"),
                    tomb("hooks/session-start.sh", "R-2"),
                    hook_tomb("PostToolUse", "pack-guard.sh", "R-3"),
                ])
                .with_boot_critical(bc_only(&["hooks/bootdep.sh"]))
                .with_prev_vendor(|rel: &str| {
                    if rel == "bin/prev.py" {
                        vec![content_hash("OLD-RELEASED-PREV\n")]
                    } else {
                        Vec::new()
                    }
                })
        };
        let plan = rig.plan(&items, false, &sources());
        rig.install(&items, false, &sources());
        let set = |v: &Vec<String>| v.iter().cloned().collect::<std::collections::BTreeSet<_>>();
        let bs = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<std::collections::BTreeSet<_>>();
        // plan 버킷 = 기대 버킷
        assert_eq!(set(&plan.user_kept), bs(&["bin/general.py"]), "plan user_kept");
        assert_eq!(set(&plan.vendor_first), bs(&["hooks/bootdep.sh"]), "plan vendor_first");
        assert_eq!(set(&plan.keep_absent), bs(&["bin/tomb.py"]), "plan keep_absent");
        assert_eq!(set(&plan.update), bs(&["bin/prev.py", "acl.json"]), "plan update");
        assert_eq!(set(&plan.create), bs(&["hooks/session-start.sh", "bin/new.py"]), "plan create");
        assert_eq!(set(&plan.heal), bs(&["bin/cp949.py"]), "plan heal(판독 불가 system)");
        assert_eq!(set(&plan.merge_new), bs(&["agents.json"]), "plan merge_new(가드 2)");
        assert_eq!(set(&plan.keep_user), bs(&["soul.md"]), "plan keep_user(판독 불가 user-owned)");
        // 실제 = plan
        for r in &plan.user_kept {
            assert_eq!(rig.kind(r).as_deref(), Some("user-kept"), "{r} 실제 user-kept");
            assert_eq!(rig.read(&format!("{r}.new")).as_deref(),
                       items.iter().find(|(x, _)| x == r).map(|(_, c)| *c), "{r} .new = vendor");
        }
        assert_eq!(rig.read("bin/general.py").as_deref(), Some("USER-EDIT-GENERAL\n"), "사용자 파일 유지");
        for r in &plan.vendor_first {
            assert_eq!(rig.kind(r).as_deref(), Some("vendor-first"), "{r} 실제 vendor-first");
        }
        assert_eq!(rig.read("hooks/bootdep.sh").as_deref(), Some("#!/bin/bash\necho vendor-boot-2\n"), "기동 필수 = vendor 먼저");
        assert_eq!(rig.read("hooks/bootdep.sh.user").as_deref(), Some("#!/bin/bash\necho USER-BOOT\n"), "사용자본 보관");
        for r in &plan.keep_absent {
            assert!(!rig.pack.join(r).exists(), "{r} 부재 유지(묘비)");
        }
        for r in &plan.update {
            assert_eq!(rig.read(r).as_deref(), items.iter().find(|(x, _)| x == r).map(|(_, c)| *c), "{r} 갱신");
            assert!(!rig.pack.join(format!("{r}.user")).exists(), "{r} 사용자 수정이 아니다 — .user 없음");
        }
        for r in &plan.create {
            assert_eq!(rig.read(r).as_deref(), items.iter().find(|(x, _)| x == r).map(|(_, c)| *c), "{r} 생성(생명줄은 묘비 무시)");
        }
        assert_eq!(rig.read("bin/cp949.py").as_deref(), Some("VENDOR-CP949\n"), "판독 불가 system → vendor");
        assert_eq!(std::fs::read(rig.pack.join("bin/cp949.py.user")).unwrap(), vec![0xC7, 0xD1, 0xB1, 0xDB, 0x0A], "바이트 백업");
        assert_eq!(rig.read("agents.json").as_deref(), Some("{\"agents\": 1}\n"), "가드 2 — 갱신 안 함");
        assert_eq!(rig.read("agents.json.new").as_deref(), Some("{\"agents\": 2}\n"), "가드 2 — .new");
        assert_eq!(std::fs::read(rig.pack.join("soul.md")).unwrap(), vec![0xC7, 0xD1, 0x0A], "판독 불가 user-owned 바이트 그대로");
        // 묘비 파일의 매니페스트 항목은 설치되지 않은 파일이라 지운다.
        let (m, _) = load_manifest_state(&rig.pack);
        assert!(!m.contains_key("bin/tomb.py"), "묘비 rel 은 매니페스트에 없다");
        // 두 번째 스윕 — plan 이 변화 없음과 보관 유지(재시도 금지)를 말하고 실제도 같다.
        let plan2 = rig.plan(&items, false, &sources());
        assert_eq!(set(&plan2.user_kept), bs(&["bin/general.py"]), "두 번째도 user-kept(같은 vendor 본 — 다시 합치지 않음)");
        assert!(plan2.vendor_first.is_empty(), "vendor 우선은 한 번 — 이제 디스크=vendor");
        assert_eq!(set(&plan2.keep_absent), bs(&["bin/tomb.py"]));
        rig.install(&items, false, &sources());
        assert_eq!(rig.kind("hooks/bootdep.sh").as_deref(), Some("vendor-first"), "다시 입히기 대기는 지워지지 않는다");
    }

    /// 가: 보관 중인 항목은 매 업데이트·점검마다 다시 합치지 않는다 — 두 번째 스윕은 병합 전 사본을 만들지 않고,
    /// 첫 스윕의 사본은 정리된다(사용자본이 살아 있어 지킬 것이 없다). 설치 기록은 앞당기지 않는다.
    #[test]
    fn m11_user_kept_second_sweep_no_new_capture_and_manifest_not_advanced() {
        let rig = Rig::new("retry");
        let base = "line1\nline2\nline3\n";
        let mine = "line1\nMINE\nline3\n";
        let vendor = "line1\nVENDOR\nline3\n"; // 같은 줄 → 충돌
        let items = [("bin/conf.py", vendor)];
        rig.write("bin/conf.py", mine.as_bytes());
        rig.manifest(&[("bin/conf.py", base)]);
        rig.pristine("bin/conf.py", base);
        let src = UpdateSources::none();
        rig.install(&items, false, &src);
        assert_eq!(rig.read("bin/conf.py").as_deref(), Some(mine), "사용자 파일 유지");
        assert_eq!(rig.kind("bin/conf.py").as_deref(), Some("user-kept"));
        assert_eq!(rig.entry("bin/conf.py").unwrap()["reason"], "merge-conflict");
        assert_eq!(rig.capture_files(), 0, "병합 전 사본 정리");
        let (m, _) = load_manifest_state(&rig.pack);
        assert_eq!(m.get("bin/conf.py"), Some(&content_hash(base)), "설치 기록을 앞당기지 않는다(앞당기면 '안정'으로 분류돼 다시 알리지 않는다)");
        let before = std::fs::read_to_string(rig.pack.join(MERGE_PENDING_FILE)).unwrap();
        rig.install(&items, false, &src);
        assert_eq!(rig.capture_files(), 0, "두 번째 스윕은 병합 전 사본을 만들지 않는다");
        assert_eq!(std::fs::read_to_string(rig.pack.join(MERGE_PENDING_FILE)).unwrap(), before, "원장 무변경(재시도 없음 · rewrite 0)");
        assert_eq!(rig.read("bin/conf.py").as_deref(), Some(mine));
        // vendor 가 다시 전진하면 그 새 본으로 .new 갱신(새 vendor 본에 한 번은 다시 합쳐 본다).
        let vendor3 = "line1\nVENDOR3\nline3\n";
        rig.install(&[("bin/conf.py", vendor3)], false, &src);
        assert_eq!(rig.read("bin/conf.py.new").as_deref(), Some(vendor3));
        assert_eq!(rig.read("bin/conf.py").as_deref(), Some(mine));
        assert_eq!(rig.capture_files(), 0);
    }

    /// 나 + 사: 기동 필수 파일이 두 번 밀려나도 첫 수정본이 덮이지 않는다(.user + .user.prev-<시각>).
    #[test]
    fn m11_vendor_first_parks_user_with_generations() {
        let rig = Rig::new("gen");
        let src = || UpdateSources::none().with_boot_critical(bc_only(&["hooks/inject.sh"]));
        rig.write("hooks/inject.sh", b"#!/bin/bash\necho USER-A\n");
        rig.install(&[("hooks/inject.sh", "#!/bin/bash\necho V1\n")], false, &src());
        assert_eq!(rig.read("hooks/inject.sh").as_deref(), Some("#!/bin/bash\necho V1\n"));
        assert_eq!(rig.kind("hooks/inject.sh").as_deref(), Some("vendor-first"));
        assert!(rig.entry("hooks/inject.sh").unwrap().get("parked_at").is_some(), "보관 시각 기록");
        rig.write("hooks/inject.sh", b"#!/bin/bash\necho USER-B\n");
        rig.install(&[("hooks/inject.sh", "#!/bin/bash\necho V2\n")], false, &src());
        assert_eq!(rig.read("hooks/inject.sh").as_deref(), Some("#!/bin/bash\necho V2\n"));
        assert_eq!(rig.read("hooks/inject.sh.user").as_deref(), Some("#!/bin/bash\necho USER-B\n"));
        let prev: Vec<_> = std::fs::read_dir(rig.pack.join("hooks"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("inject.sh.user.prev-"))
            .collect();
        assert_eq!(prev.len(), 1, "첫 수정본 세대 1개");
        assert_eq!(std::fs::read_to_string(prev[0].path()).unwrap(), "#!/bin/bash\necho USER-A\n", "A 가 B 로 덮이지 않았다");
    }

    /// ★T2g(RULINGS CQ1 · CR2): vendor-first 기록은 같은 버전의 스윕이 **손으로 고친** 살아 있는 파일을 kept-drift 로 볼 때도
    /// 그대로 남는다(종전: kept-drift 가 그 기록을 바꿔 써서 `cys repair reapply` 가 보관본을 찾지 못했다). 그 상태에서 진짜
    /// 다음 업데이트는 **새로 고친 사본**을 `.user` 에 보관하고 앞서 보관한 사본을 `.user.prev-*` 로 옮긴다(오늘과 같다) — 세대마다
    /// 바이트와 기록의 모양(`kind: vendor-first` · `side` = 실재하는 `<rel>.user` — T6 의 `reapply` 가 읽는 모양)을 잰다.
    #[test]
    fn t2g_cq1_vendor_first_record_survives_a_same_version_sweep_after_a_hand_edit() {
        let rig = Rig::new("cq1");
        let rel = "hooks/inject.sh";
        let src = || UpdateSources::none().with_boot_critical(bc_only(&["hooks/inject.sh"]));
        let v1 = "#!/bin/bash\necho V1\n";
        let edit_a = "#!/bin/bash\necho USER-A\n";
        let edit_b = "#!/bin/bash\necho USER-B — a hand edit after the update\n";
        let v2 = "#!/bin/bash\necho V2\n";
        let shape = |want_user: &str| {
            let e = rig.entry(rel).expect("기록");
            assert_eq!(e["kind"], "vendor-first", "{e}");
            let side = e["side"].as_str().expect("side");
            assert_eq!(side, format!("{rel}.user"));
            assert!(rig.pack.join(side).is_file(), "side 는 실재하는 파일");
            assert_eq!(rig.read(side).as_deref(), Some(want_user), "side 의 바이트 = 내 사본");
        };
        // 업데이트 1: 내 수정 A → vendor 우선(살아 있는 파일 = V1, A 는 .user).
        rig.write(rel, edit_a.as_bytes());
        rig.install(&[(rel, v1)], false, &src());
        assert_eq!(rig.read(rel).as_deref(), Some(v1));
        shape(edit_a);
        // 손으로 고침 B → 같은 버전의 스윕(V1 그대로): KeepDrift — 살아 있는 파일 무접촉, 기록 그대로(rewrite 0).
        rig.write(rel, edit_b.as_bytes());
        let before = std::fs::read_to_string(rig.pack.join(MERGE_PENDING_FILE)).unwrap();
        assert_eq!(rig.plan(&[(rel, v1)], false, &src()).kept_drift, vec![rel.to_string()], "판정은 그대로 kept-drift");
        rig.install(&[(rel, v1)], false, &src());
        assert_eq!(rig.read(rel).as_deref(), Some(edit_b), "살아 있는 파일 무접촉");
        assert_eq!(std::fs::read_to_string(rig.pack.join(MERGE_PENDING_FILE)).unwrap(), before, "기록 그대로 — rewrite 0");
        shape(edit_a);
        assert!(load_merge_pending(&rig.pack).contains_key(rel), "병합 대기 목록이 이 파일을 보인다");
        // 진짜 다음 업데이트(V2): 새로 고친 B 를 .user 에, 앞서 보관한 A 는 .user.prev-* 로(오늘과 같다).
        rig.install(&[(rel, v2)], false, &src());
        assert_eq!(rig.read(rel).as_deref(), Some(v2), "기동 필수 = vendor 먼저");
        shape(edit_b);
        let prev: Vec<_> = std::fs::read_dir(rig.pack.join("hooks"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("inject.sh.user.prev-"))
            .collect();
        assert_eq!(prev.len(), 1, "앞선 세대 1개");
        assert_eq!(std::fs::read_to_string(prev[0].path()).unwrap(), edit_a, "A 는 한 세대 뒤에 그대로");
        // 대조: 같은 버전의 kept-drift 가 merged 계보에는 state 만 붙이고(종전 규칙 그대로), 기록 없는 파일에는 kept-drift 를 쓴다.
        let mut p = serde_json::Map::new();
        let mut dirty = false;
        let kd = |r: &str| match serde_json::json!({"kind": "kept-drift", "side": r, "version": "x", "ts": 1}) {
            serde_json::Value::Object(o) => o,
            _ => unreachable!(),
        };
        p.insert("a".into(), serde_json::json!({"kind": "vendor-first", "side": "a.user", "version": "x", "ts": 0}));
        upsert_pending_v2(&mut p, &mut dirty, "a", kd("a"));
        assert!(!dirty, "vendor-first: no-op");
        assert_eq!(p["a"], serde_json::json!({"kind": "vendor-first", "side": "a.user", "version": "x", "ts": 0}), "항목 그대로");
        p.insert("m".into(), serde_json::json!({"kind": "merged", "capture": "c", "version": "x", "ts": 0}));
        upsert_pending_v2(&mut p, &mut dirty, "m", kd("m"));
        assert_eq!((p["m"]["kind"].as_str(), p["m"]["state"].as_str()), (Some("merged"), Some("at-rest")));
        upsert_pending_v2(&mut p, &mut dirty, "n", kd("n"));
        assert_eq!(p["n"]["kind"], "kept-drift");
    }

    /// ★T2h(RULINGS CW5 · 검토자 M1): vendor 우선의 '원장 선저장'은 저장이 **디스크에 남았을 때만** 살아 있는 파일을 바꾼다 —
    /// `.merge-pending.json` 이 폴더(쓸 수 없는 원장)면 기동 필수 파일은 사용자 사본 그대로 남고(격리), 원장을 쓸 수 있게 되면
    /// 다음 스윕이 같은 판정으로 수렴한다(vendor 본 · `.user` · 기록). 종전: 저장 실패를 버리고 vendor 본을 써 기록 없는
    /// 보관본을 남겼다.
    #[test]
    fn t2h_cw5_vendor_first_never_replaces_the_live_file_without_its_record() {
        let rig = Rig::new("cw5");
        let rel = "hooks/inject.sh";
        let src = || UpdateSources::none().with_boot_critical(bc_only(&["hooks/inject.sh"]));
        let mine = "#!/bin/bash\necho USER-A\n";
        let v1 = "#!/bin/bash\necho V1\n";
        rig.write(rel, mine.as_bytes());
        std::fs::create_dir_all(rig.pack.join(MERGE_PENDING_FILE)).unwrap();
        assert!(save_merge_pending(&rig.pack, &serde_json::Map::new()).is_err(), "폴더인 원장 — 저장은 실패를 돌려준다");
        rig.install(&[(rel, v1)], false, &src());
        assert_eq!(rig.read(rel).as_deref(), Some(mine), "기록이 남지 않으면 살아 있는 파일을 바꾸지 않는다");
        assert!(rig.pack.join(MERGE_PENDING_FILE).is_dir(), "원장 자리는 그대로");
        // 원장을 쓸 수 있게 되면 다음 스윕이 수렴한다.
        std::fs::remove_dir_all(rig.pack.join(MERGE_PENDING_FILE)).unwrap();
        rig.install(&[(rel, v1)], false, &src());
        assert_eq!(rig.read(rel).as_deref(), Some(v1), "기동 필수 = vendor 먼저");
        assert_eq!(rig.read(&format!("{rel}.user")).as_deref(), Some(mine), "내 사본 보관");
        assert_eq!(rig.kind(rel).as_deref(), Some("vendor-first"), "기록과 함께");
    }

    /// ★T2i(RULINGS EC2): 두 쓰는 쪽이 각 경계에서 엇갈려도 **모든 사본이 남는다**(이름 또는 세대 자리) — U(먼저 보관돼 있던 사본) ·
    /// V(쓰는 쪽 B) · W(쓰는 쪽 A). 경계 = A 의 첫 놓기 직전 · A 가 '차 있음'을 보고 옮기기 직전 · A 의 다시 놓기 직전 — 그
    /// 지점에서 B 가 끝까지 돈다(끼어들기 지점으로 순서를 정한다 · 잠 없음). 자리가 끝내 비지 않으면(놓기마다 다른 쓰는 쪽의
    /// 사본이 먼저 생김) A 의 사본은 자기 세대 자리에, 겹침은 감사 원장에 남는다.
    #[test]
    fn ec2_two_writers_every_parked_copy_survives_at_each_boundary() {
        let rig = Rig::new("ec2");
        let rel = "hooks/x.sh";
        let copies = |rig: &Rig| -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(rig.pack.join("hooks"))
                .unwrap()
                .flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n == "x.sh.user" || n.starts_with("x.sh.user.prev-")
                })
                .map(|e| std::fs::read_to_string(e.path()).unwrap())
                .collect();
            v.sort();
            v
        };
        let reset = |rig: &Rig| {
            let _ = std::fs::remove_dir_all(rig.pack.join("hooks"));
            let _ = std::fs::remove_file(rig.pack.join(MERGE_AUDIT_FILE));
            rig.write(&format!("{rel}.user"), b"U\n");
        };
        for (case, at) in [
            ("before the first placement", ParkStep::BeforePlace(0)),
            ("before moving the occupant", ParkStep::BeforeMove(0)),
            ("before the retry", ParkStep::BeforePlace(1)),
        ] {
            reset(&rig);
            let pack = rig.pack.clone();
            let mut fired = false;
            let out = park_copy_at(&rig.pack, rel, b"W\n", false, 100, &mut |s| {
                if s == at && !fired {
                    fired = true;
                    park_copy_at(&pack, rel, b"V\n", false, 100, &mut |_| {}).unwrap();
                }
            })
            .unwrap();
            assert!(fired, "{case}: the interleaving happened");
            assert_eq!(copies(&rig), vec!["U\n", "V\n", "W\n"], "{case}: every copy exists");
            assert!(!out.overlap, "{case}");
            assert_eq!(rig.read(&format!("{rel}.user")).as_deref(), Some("W\n"), "{case}: A's copy holds the name");
        }
        // 같은 바이트가 이미 그 자리에 있으면 끝(사본이 늘지 않는다).
        reset(&rig);
        let out = park_copy_at(&rig.pack, rel, b"U\n", false, 100, &mut |_| {}).unwrap();
        assert_eq!((out.moved.len(), out.overlap), (0, false));
        assert_eq!(copies(&rig), vec!["U\n"]);
        // 자리가 끝내 비지 않음 → 자기 세대 자리 + 감사 원장의 겹침.
        reset(&rig);
        let pack = rig.pack.clone();
        let mut k = 0;
        let out = park_copy_at(&rig.pack, rel, b"W\n", false, 100, &mut |s| {
            if let ParkStep::BeforePlace(_) = s {
                k += 1;
                park_copy_at(&pack, rel, format!("X{k}\n").as_bytes(), false, 200, &mut |_| {}).unwrap();
            }
        })
        .unwrap();
        assert!(out.overlap, "{out:?}");
        assert_eq!(copies(&rig), vec!["U\n", "W\n", "X1\n", "X2\n", "X3\n"], "every copy exists");
        assert_eq!(rig.read(&format!("{rel}.user")).as_deref(), Some("X3\n"), "the last other copy stays at the name");
        assert_eq!(std::fs::read_to_string(rig.pack.join(&out.at)).unwrap(), "W\n");
        let audit = std::fs::read_to_string(rig.pack.join(MERGE_AUDIT_FILE)).unwrap();
        let e: serde_json::Value = serde_json::from_str(audit.lines().last().unwrap()).unwrap();
        assert_eq!((e["action"].as_str(), e["file"].as_str(), e["slot"].as_str()), (Some("park-overlap"), Some(rel), Some(out.at.as_str())));
        assert_eq!(out.overlap_record_error, None, "겹침을 적었다");
        // ★T2i(RULINGS FH5): 감사 원장을 쓸 수 없으면(폴더) 사본은 자기 세대 자리에 안전하지만 '기록 실패'가 따로 돌아온다.
        reset(&rig);
        std::fs::create_dir_all(rig.pack.join(MERGE_AUDIT_FILE)).unwrap();
        let pack = rig.pack.clone();
        let mut k = 0;
        let out = park_copy_at(&rig.pack, rel, b"W\n", false, 100, &mut |s| {
            if let ParkStep::BeforePlace(_) = s {
                k += 1;
                park_copy_at(&pack, rel, format!("Y{k}\n").as_bytes(), false, 300, &mut |_| {}).unwrap();
            }
        })
        .unwrap();
        assert!(out.overlap);
        assert!(out.overlap_record_error.as_deref().is_some_and(|e| e.contains("감사 원장")), "{out:?}");
        assert_eq!(std::fs::read_to_string(rig.pack.join(&out.at)).unwrap(), "W\n", "사본은 안전하다");
        std::fs::remove_dir_all(rig.pack.join(MERGE_AUDIT_FILE)).unwrap();
        // ★T2i(RULINGS FP5 m8): 읽을 수 없는 일반 파일인 사본 = '판독 불가 사본' — 장애물이 아니라 세대 자리로 옮기고 새 사본을 놓는다.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            reset(&rig);
            let up = rig.pack.join(format!("{rel}.user"));
            std::fs::set_permissions(&up, std::fs::Permissions::from_mode(0o000)).unwrap();
            let out = park_copy_at(&rig.pack, rel, b"W\n", false, 400, &mut |_| {}).unwrap();
            assert_eq!((out.at.as_str(), out.overlap, out.moved.len()), ("hooks/x.sh.user", false, 1), "{out:?}");
            let slot = rig.pack.join(&out.moved[0]);
            std::fs::set_permissions(&slot, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(std::fs::read_to_string(&slot).unwrap(), "U\n", "판독 불가였던 사본은 세대 자리에 그대로");
            assert_eq!(rig.read(&format!("{rel}.user")).as_deref(), Some("W\n"));
        }
        // 파일이 아닌 자리(폴더) = 장애물 → 보관하지 않는다(호출부의 격리 갈래).
        reset(&rig);
        std::fs::remove_file(rig.pack.join(format!("{rel}.user"))).unwrap();
        std::fs::create_dir_all(rig.pack.join(format!("{rel}.user"))).unwrap();
        let e = park_copy_at(&rig.pack, rel, b"W\n", false, 100, &mut |_| {}).unwrap_err();
        assert!(e.contains("일반 파일이 아닌 항목(폴더)"), "{e}");
    }

    /// ★T2i(RULINGS EC4): 팩 동사는 이미 있는 경로를 쓰려고 열지 않는다 — 검토자의 두 순서: (a) 미리 심은 `<rel>.user.tmp-<pid>`
    /// (보호 훅을 가리키는 링크 · 그 훅의 하드 링크) → 바이트 백업은 그 이름을 쓰지 않고(새 배타 임시 파일) 보호 훅은 그대로,
    /// (b) 감사 원장이 보호 파일의 하드 링크 → 덧붙이지 않는다. (4) 구성 요소가 링크로 바뀐 자리: 닻 내린 쓰기 · 보관 · 덮지 않는
    /// 이동은 거부하고 링크 너머에 아무것도 만들지 않는다. 윈도우: 같은 규칙을 핸들의 사실로(링크 수 = `GetFileInformationByHandle`,
    /// 자리 = `GetFinalPathNameByHandleW`) — 이 기계에서는 잴 수 없다(TW1).
    #[cfg(unix)]
    #[test]
    fn ec4_a_planted_temp_or_a_linked_audit_never_writes_a_protected_file() {
        let rig = Rig::new("ec4");
        let guard = "hooks/guard.sh";
        let guard_bytes = "#!/bin/sh\n# PROTECTED\n";
        rig.write(guard, guard_bytes.as_bytes());
        let rel = "hooks/x.sh";
        let src: &[u8] = b"\xc7\xd1\xb1\xdb CP949\n";
        rig.write(rel, src);
        for form in ["symlink", "hardlink"] {
            let planted = rig.pack.join(format!("{rel}.user.tmp-{}", std::process::id()));
            let _ = std::fs::remove_file(&planted);
            let _ = std::fs::remove_file(rig.pack.join(format!("{rel}.user")));
            if form == "symlink" {
                std::os::unix::fs::symlink(rig.pack.join(guard), &planted).unwrap();
            } else {
                std::fs::hard_link(rig.pack.join(guard), &planted).unwrap();
            }
            byte_backup_user(&rig.pack, rel, &rig.pack.join(rel)).unwrap();
            assert_eq!(rig.read(guard).as_deref(), Some(guard_bytes), "{form}: the protected hook is untouched");
            assert_eq!(std::fs::read(rig.pack.join(format!("{rel}.user"))).unwrap(), src, "{form}: the backup holds the source bytes");
            assert!(std::fs::symlink_metadata(&planted).is_ok(), "{form}: the planted name is left alone, not reused");
        }
        let _ = std::fs::remove_file(rig.pack.join(MERGE_AUDIT_FILE));
        std::fs::hard_link(rig.pack.join(guard), rig.pack.join(MERGE_AUDIT_FILE)).unwrap();
        assert!(append_merge_audit(&rig.pack, &serde_json::json!({"action": "x", "file": rel})).is_err(), "hard-linked audit refused");
        assert_eq!(rig.read(guard).as_deref(), Some(guard_bytes), "the protected file is untouched by the audit");
        std::fs::remove_file(rig.pack.join(MERGE_AUDIT_FILE)).unwrap();
        // (4) 구성 요소가 링크: `skills` → 팩 밖 폴더.
        let elsewhere = rig.base.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, rig.pack.join("skills")).unwrap();
        assert!(write_pack_file_beneath(&rig.pack, "skills/a.md", b"A", None, false, Place::Replace).is_err());
        assert!(park_copy(&rig.pack, "skills/a.md", b"A", false).is_err());
        std::fs::write(elsewhere.join("b.md.user"), "B").unwrap();
        assert!(beneath_move_no_replace(&rig.pack, "skills/b.md.user", "skills/b.md.user.prev-1").is_err());
        let left: Vec<String> = std::fs::read_dir(&elsewhere).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec!["b.md.user".to_string()], "nothing created or moved behind the link");
        // 덮지 않는 놓기: 차 있으면 AlreadyExists · 무변경.
        rig.write("hooks/n.sh", b"OLD");
        let e = beneath_write(&rig.pack, "hooks/n.sh", b"NEW", None, false, Place::NoReplace, false).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(rig.read("hooks/n.sh").as_deref(), Some("OLD"));
    }

    /// ★T2i(RULINGS FP1 · GD1 · GB5): 덮지 않는 이름 바꾸기를 파일 시스템이 지원하지 않아도(시험 주입) 사본은 다른 길로 보관된다 —
    /// 마지막 이름의 배타 생성. 자리에 다른 사본이 있으면 그것은 그대로 두고 이 사본은 자기 세대 자리에(겹침 기록). 그 다른 길마저
    /// 막히면: vendor 우선은 덮지 않는다(격리). 잠금 보안 자산(L0)은 무결성 우선 — 두 갈래(판독 가능 · 판독 불가) 모두 치유는 가고,
    /// 기록은 없는 사본을 이름 대지 않는다(backup: failed + 사유). `--missing-only` 의 쓰기(`write_pack_file_beneath` · 덮지 않음)도 그
    /// 길로 만든다.
    /// ★T2i(RULINGS GB5): unix 만 — 주입(`FAIL_NO_REPLACE_RENAME` · `FAIL_EXCLUSIVE_FALLBACK`)은 unix 의 `no_replace_renameat` ·
    /// `exclusive_create_at` 에만 걸린다. 윈도우의 같은 경우(덮지 않는 이동이 '지원하지 않음' → CREATE_NEW 로 만들기 · 그것도 막히면
    /// 살아 있는 파일 그대로 · 자리에 있는 사본 보존)는 Windows 측정(TW1)의 행이다 — 어느 Windows 작업도 이 시험을 돌리지 않는다
    /// (`.github/workflows`: 전체 `cargo test --lib` 는 macOS 레그만, Windows 는 필터 `factory_reset::` · `merge3::` 등).
    #[cfg(unix)]
    #[test]
    fn fp1_without_the_no_replace_rename_the_copy_is_still_parked_and_nothing_live_is_overwritten_without_it() {
        struct Inject;
        impl Drop for Inject {
            fn drop(&mut self) {
                FAIL_NO_REPLACE_RENAME.with(|f| f.set(false));
                FAIL_EXCLUSIVE_FALLBACK.with(|f| f.set(false));
            }
        }
        let _reset = Inject;
        let rig = Rig::new("fp1");
        let rel = "hooks/x.sh";
        // ① 이름 바꾸기 지원 없음 → 배타 생성으로 보관.
        FAIL_NO_REPLACE_RENAME.with(|f| f.set(true));
        let out = park_copy(&rig.pack, rel, b"W\n", false).unwrap();
        assert_eq!((out.at.as_str(), out.overlap), ("hooks/x.sh.user", false));
        assert_eq!(rig.read("hooks/x.sh.user").as_deref(), Some("W\n"));
        // 자리에 다른 사본(U) → U 는 그대로, W2 는 자기 세대 자리에 + 겹침 기록.
        let out = park_copy(&rig.pack, rel, b"W2\n", false).unwrap();
        assert!(out.overlap, "{out:?}");
        assert_eq!(rig.read("hooks/x.sh.user").as_deref(), Some("W\n"), "자리의 사본은 그대로");
        assert_eq!(std::fs::read_to_string(rig.pack.join(&out.at)).unwrap(), "W2\n");
        // `--missing-only` 의 쓰기: 없던 파일은 만들고, 있으면 차 있음(무변경).
        write_pack_file_beneath(&rig.pack, "bin/fresh.py", b"print(1)\n", None, true, Place::NoReplace).unwrap();
        assert_eq!(rig.read("bin/fresh.py").as_deref(), Some("print(1)\n"));
        let e = write_pack_file_beneath(&rig.pack, "bin/fresh.py", b"X", None, true, Place::NoReplace).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(rig.read("bin/fresh.py").as_deref(), Some("print(1)\n"));
        // ② 다른 길도 막힘 → 보관 실패.
        FAIL_EXCLUSIVE_FALLBACK.with(|f| f.set(true));
        assert!(park_copy(&rig.pack, "hooks/y.sh", b"Y\n", false).is_err());
        assert!(!rig.pack.join("hooks/y.sh.user").exists());
        // vendor 우선: 살아 있는 파일 무변경 · 격리 · 사본 없음.
        let vf = "hooks/inject.sh";
        rig.write(vf, b"#!/bin/bash\necho MINE\n");
        rig.install(&[(vf, "#!/bin/bash\necho V1\n")], false, &UpdateSources::none().with_boot_critical(bc_only(&["hooks/inject.sh"])));
        assert_eq!(rig.read(vf).as_deref(), Some("#!/bin/bash\necho MINE\n"), "vendor 우선: 보관 없이는 덮지 않는다");
        assert_eq!(rig.kind(vf).as_deref(), Some("quarantined"));
        assert!(!rig.pack.join(format!("{vf}.user")).exists(), "사본 없음");
        // ★T2i(RULINGS GD1): 잠금 파일(판독 가능) — 보관 실패여도 무결성 우선으로 치유하고, 기록은 없는 사본을 이름 대지 않는다.
        let locked = "trusted-keys.json";
        rig.write(locked, b"{\"keys\": [\"MINE\"]}\n");
        rig.manifest(&[(locked, "{\"keys\": []}\n")]);
        // 기준선이 맞아야(설치 기록 = .pristine) C-07 의 제자리 보존이 아니라 L0 치유 갈래에 닿는다.
        rig.pristine(locked, "{\"keys\": []}\n");
        rig.install(&[(locked, "{\"keys\": [\"VENDOR\"]}\n")], false, &UpdateSources::none());
        assert_eq!(rig.read(locked).as_deref(), Some("{\"keys\": [\"VENDOR\"]}\n"), "L0: 무결성 우선 — 치유는 간다");
        let e = rig.entry(locked).unwrap();
        assert_eq!((e["kind"].as_str(), e["backup"].as_str()), (Some("healed"), Some("failed")), "{e}");
        assert!(e.get("side").is_none(), "없는 사본을 이름 대지 않는다: {e}");
        assert!(!rig.pack.join(format!("{locked}.user")).exists());
        // 잠금 파일(판독 불가 — UTF-8 아님): 같은 규칙.
        rig.write(locked, b"\xff\xfe\x00keys\n");
        rig.install(&[(locked, "{\"keys\": [\"VENDOR\"]}\n")], false, &UpdateSources::none());
        assert_eq!(rig.read(locked).as_deref(), Some("{\"keys\": [\"VENDOR\"]}\n"), "판독 불가 L0 도 치유는 간다");
        let e = rig.entry(locked).unwrap();
        assert_eq!((e["kind"].as_str(), e["backup"].as_str()), (Some("healed"), Some("failed")), "{e}");
        assert!(e.get("side").is_none(), "{e}");
        rig.write(locked, b"\xff\xfe\x00keys\n");
        // 주입을 풀면 같은 스윕이 보관하고 치유한다(기록은 그 사본을 이름 댄다).
        FAIL_NO_REPLACE_RENAME.with(|f| f.set(false));
        FAIL_EXCLUSIVE_FALLBACK.with(|f| f.set(false));
        rig.install(&[(locked, "{\"keys\": [\"VENDOR\"]}\n")], false, &UpdateSources::none());
        assert_eq!(rig.read(locked).as_deref(), Some("{\"keys\": [\"VENDOR\"]}\n"));
        assert_eq!(std::fs::read(rig.pack.join(format!("{locked}.user"))).unwrap(), b"\xff\xfe\x00keys\n", "사본을 보관했다");
        let e = rig.entry(locked).unwrap();
        assert_eq!((e["kind"].as_str(), e["side"].as_str()), (Some("healed"), Some("trusted-keys.json.user")), "기록은 있는 사본을 이름 댄다");
        assert!(e.get("backup").is_none(), "{e}");
    }

    /// ★T2i(RULINGS GB2): 자리에 이미 있는 같은 사본은 다시 내린다 — 대체 길(배타 생성)의 파일 내림이 실패해 사본이 내려지지
    /// 않은 채 남으면, 다음 스윕의 같은 사본 재사용도 그 파일을 내려야 하고, 그 내림이 다시 실패하면 살아 있는 파일은 그대로다.
    /// 내림이 되면 그때 진행한다. (unix 만 — 주입은 unix 함수에 걸린다 · 윈도우 경우는 TW1 의 행, RULINGS GB5.)
    #[cfg(unix)]
    #[test]
    fn gb2_a_copy_found_in_place_is_flushed_again_before_live_changes() {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                FAIL_NO_REPLACE_RENAME.with(|f| f.set(false));
                FAIL_FILE_SYNC_FOR.with(|f| *f.borrow_mut() = None);
            }
        }
        let _reset = Reset;
        let rig = Rig::new("gb2");
        let rel = "hooks/inject.sh";
        let src = || UpdateSources::none().with_boot_critical(bc_only(&["hooks/inject.sh"]));
        let mine = "#!/bin/bash\necho MINE\n";
        let v1 = "#!/bin/bash\necho V1\n";
        rig.write(rel, mine.as_bytes());
        FAIL_NO_REPLACE_RENAME.with(|f| f.set(true));
        FAIL_FILE_SYNC_FOR.with(|f| *f.borrow_mut() = Some("inject.sh.user".into()));
        // ① 대체 길이 사본을 쓰지만 그 파일 내림이 실패 → 보관 실패 → 살아 있는 파일 그대로(사본은 내려지지 않은 채 남음).
        rig.install(&[(rel, v1)], false, &src());
        assert_eq!(rig.read(rel).as_deref(), Some(mine), "① 내림 실패 → 덮지 않음");
        assert_eq!(rig.read(&format!("{rel}.user")).as_deref(), Some(mine), "① 내려지지 않은 사본이 자리에 남음");
        // ② 다음 스윕: 같은 사본을 다시 쓰려면 그 파일을 내려야 한다 — 다시 실패 → 그대로.
        rig.install(&[(rel, v1)], false, &src());
        assert_eq!(rig.read(rel).as_deref(), Some(mine), "② 다시 내림 실패 → 덮지 않음");
        assert_eq!(rig.kind(rel).as_deref(), Some("quarantined"));
        // ③ 내림이 되면 진행.
        FAIL_FILE_SYNC_FOR.with(|f| *f.borrow_mut() = None);
        rig.install(&[(rel, v1)], false, &src());
        assert_eq!(rig.read(rel).as_deref(), Some(v1), "③ 내렸으면 vendor 우선");
        assert_eq!(rig.kind(rel).as_deref(), Some("vendor-first"));
        assert_eq!(rig.read(&format!("{rel}.user")).as_deref(), Some(mine));
    }

    /// ★T2i(RULINGS EC7): vendor 우선의 선저장은 **영속이 증명돼야** 한다 — 원장 쓰기의 부모 폴더 fsync 가 실패하면(시험 주입)
    /// 살아 있는 기동 필수 파일은 사용자 사본 그대로다(격리 · ledger-write-failed). 보관본 `<rel>.user` 쓰기의 폴더 fsync 가
    /// 실패해도 같다(user-backup-failed). 주입이 없으면 같은 스윕이 vendor 본 · `.user` · 기록으로 수렴한다. 영속 쓰기 자체는
    /// 폴더 fsync 실패를 돌려주고, '지원하지 않음'(EINVAL · ENOTSUP)만 너그럽게 넘긴다.
    #[test]
    fn ec7_vendor_first_needs_a_durable_record_before_the_live_file_changes() {
        let rig = Rig::new("ec7");
        let rel = "hooks/inject.sh";
        let src = || UpdateSources::none().with_boot_critical(bc_only(&["hooks/inject.sh"]));
        let mine = "#!/bin/bash\necho USER-A\n";
        let v1 = "#!/bin/bash\necho V1\n";
        let inject = |name: Option<&str>| FAIL_DIR_SYNC_FOR.with(|f| *f.borrow_mut() = name.map(str::to_string));
        // 영속 쓰기: 주입 → Err · 파일은 자리에 있다(교체는 됐으나 증명 안 됨) · 주입 해제 → Ok.
        let probe = rig.base.join("probe.json");
        inject(Some("probe.json"));
        assert!(write_atomic_durable(&probe, b"{}").is_err(), "폴더 fsync 실패를 돌려준다");
        inject(None);
        assert!(write_atomic_durable(&probe, b"{}").is_ok());
        #[cfg(unix)]
        {
            assert!(dir_sync_unsupported(&std::io::Error::from_raw_os_error(libc::EINVAL)));
            assert!(dir_sync_unsupported(&std::io::Error::from_raw_os_error(libc::ENOTSUP)));
            assert!(!dir_sync_unsupported(&std::io::Error::from_raw_os_error(libc::EIO)), "EIO 는 실패");
        }
        // ① 원장 쓰기의 폴더 fsync 실패 → 살아 있는 파일 무변경.
        rig.write(rel, mine.as_bytes());
        inject(Some(MERGE_PENDING_FILE));
        rig.install(&[(rel, v1)], false, &src());
        inject(None);
        assert_eq!(rig.read(rel).as_deref(), Some(mine), "원장이 영속하지 않으면 바꾸지 않는다");
        assert_eq!(rig.kind(rel).as_deref(), Some("quarantined"));
        assert_eq!(rig.entry(rel).unwrap()["reason"], "ledger-write-failed");
        // ② 보관본 쓰기의 폴더 fsync 실패 → 무변경. ★T2i(RULINGS FP5 m5): unix 만 — 윈도우에서는 폴더 내림이 '지원하지 않음'이라
        //   보관본 쓰기의 주입 지점에 닿지 않는다(TW1 에서 잴 것: 윈도우의 본문 내림 · NTFS 의 이름 영속).
        let _ = std::fs::remove_file(rig.pack.join(MERGE_PENDING_FILE));
        let _ = std::fs::remove_file(rig.pack.join(format!("{rel}.user")));
        #[cfg(unix)]
        {
            inject(Some("inject.sh.user"));
            rig.install(&[(rel, v1)], false, &src());
            inject(None);
            assert_eq!(rig.read(rel).as_deref(), Some(mine), "보관본이 영속하지 않으면 바꾸지 않는다");
            assert_eq!(rig.entry(rel).unwrap()["reason"], "user-backup-failed");
        }
        // ③ 주입 없음 → 수렴.
        rig.install(&[(rel, v1)], false, &src());
        assert_eq!(rig.read(rel).as_deref(), Some(v1), "기동 필수 = vendor 먼저");
        assert_eq!(rig.read(&format!("{rel}.user")).as_deref(), Some(mine), "내 사본 보관");
        assert_eq!(rig.kind(rel).as_deref(), Some("vendor-first"), "기록과 함께");
    }

    /// ★T2i(RULINGS EC5): 시작 경로의 남은 판독 — **정상 free 상태**(`.pack-state.json` = free · 기록 불일치라 자가치유 경로까지
    /// 간다)에서 `.pack-version` · 상위의 `.pack-accepted.json` · 설치 기록 · 상태 파일 자리에 FIFO · 폴더 · 상한 초과 · FIFO 를
    /// 가리키는 링크 · 장치(`/dev/zero`)를 가리키는 링크를 두어도 설치는 각자의 시한 안에 끝나고, 읽을 수 없는 `.pack-version` 으로
    /// 상태를 고치지 않는다(종전: 빈 값으로 자가치유). 설정 파일 판독기(`settings.json` 병합 · 재조정 · 검증, 첫기동 관문 시드의
    /// `.claude.json`, 잠금 파일 `<settings>.cys-lock`)도 같은 자리에서 시한 안에 끝나고 아무것도 덮지 않는다.
    #[test]
    fn ec5_start_path_free_state_and_settings_readers_never_block() {
        let rig = Rig::new("ec5");
        let old = "{\"agents\": 1}\n";
        let new = "{\"agents\": 2}\n";
        let items: Vec<(String, String)> = vec![("agents.json".into(), new.into())];
        let setup = |root: &Path| -> PathBuf {
            let pack = root.join("pack");
            std::fs::create_dir_all(&pack).unwrap();
            std::fs::write(pack.join("agents.json"), old).unwrap();
            std::fs::write(pack.join(PACK_VERSION_FILE), "1.0.0\n").unwrap();
            let st = PackState { channel: "free".into(), base_version: "0.9.0".into(), pro_revision: 0 };
            write_pack_state(&pack, &st).unwrap();
            let m: std::collections::BTreeMap<String, String> =
                [("agents.json".to_string(), content_hash(old))].into_iter().collect();
            std::fs::write(pack.join(INSTALL_MANIFEST), serde_json::to_string(&m).unwrap()).unwrap();
            pack
        };
        fn deadline<T: Send + 'static>(case: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(f());
            });
            rx.recv_timeout(std::time::Duration::from_secs(20))
                .unwrap_or_else(|_| panic!("{case}: did not finish within its deadline (blocked on a start-path read)"))
        }
        let run = |case: &str, pack: PathBuf| -> bool {
            let it = items.clone();
            deadline(case, move || {
                install_into_with(
                    pack,
                    it.iter().map(|(a, b)| (a.as_str(), b.as_str())),
                    false,
                    "9.9.9",
                    false,
                    false,
                    PackScope::Base,
                    None,
                    None,
                    &UpdateSources::none(),
                )
                .is_ok()
            })
        };
        let base_of = |pack: &Path| match read_pack_state(pack) {
            PackStateRead::Valid(st) => Some(st.base_version),
            _ => None,
        };
        // 대조: 정상 free 상태 · 기록 불일치 → 자가치유 → 설치 → 기록 = 설치한 판.
        let ctl = setup(&rig.base.join("control"));
        assert!(run("control", ctl.clone()), "control install");
        assert_eq!(std::fs::read_to_string(ctl.join("agents.json")).unwrap(), new, "control: 갱신");
        assert_eq!(base_of(&ctl).as_deref(), Some("9.9.9"), "control: 기록 수렴");
        let mut forms: Vec<&str> = vec!["dir", "oversized"];
        #[cfg(unix)]
        forms.extend(["fifo", "link-to-fifo", "link-to-device"]);
        let targets = [PACK_VERSION_FILE, "../.pack-accepted.json", INSTALL_MANIFEST, PACK_STATE_FILE];
        let mut cases = 0usize;
        for (ti, target) in targets.iter().enumerate() {
            for form in &forms {
                let case = format!("{target} as {form}");
                let root = rig.base.join(format!("e{ti}-{form}"));
                let pack = setup(&root);
                let p = match target.strip_prefix("../") {
                    Some(name) => root.join(name),
                    None => pack.join(target),
                };
                plant_special(&p, form, &root, &case);
                let _ = run(&case, pack.clone());
                if *target == PACK_VERSION_FILE {
                    assert_ne!(base_of(&pack).as_deref(), Some(""), "{case}: 읽을 수 없는 판 값으로 기록을 고치지 않는다");
                }
                cases += 1;
                let _ = std::fs::remove_dir_all(&root);
            }
        }
        assert_eq!(cases, targets.len() * forms.len());
        // 설정 파일 판독기 — 같은 형태를 `settings.json` · `.claude.json` · 잠금 파일 자리에.
        for form in &forms {
            let root = rig.base.join(format!("s-{form}"));
            let cfg = root.join("cfg");
            std::fs::create_dir_all(&cfg).unwrap();
            let settings = cfg.join("settings.json");
            plant_special(&settings, form, &root, &format!("settings.json as {form}"));
            let pk = root.join("pack");
            for which in ["merge", "retune", "verify"] {
                let (s, pk2) = (settings.clone(), pk.clone());
                let case = format!("{which}: settings.json as {form}");
                let r = deadline(&case, move || match which {
                    "merge" => merge_desired_hooks(&s, &pk2, &AWAKENING_HOOKS).map(|_| ()),
                    "retune" => retune_registered_hook_timeouts(&s, &pk2, &AWAKENING_HOOKS).map(|_| ()),
                    _ => {
                        let missing = verify_desired_hooks_registered(&s, &pk2, &AWAKENING_HOOKS);
                        if missing.len() == AWAKENING_HOOKS.len() { Err("all missing".into()) } else { Ok(()) }
                    }
                });
                // 읽을 수 없는 설정은 덮지 않는다(병합 · 재조정 = 오류, 검증 = 전부 미등록) — 폴더는 재조정의 '없으면 만들지 않는다'가
                // 아니라 판독 불가 갈래다.
                assert!(r.is_err(), "{case}: {r:?}");
                assert!(!cfg.join("settings.json.bak-cys").exists(), "{case}: 백업도 쓰지 않는다");
            }
            let claude = cfg.join(CLAUDE_CONFIG_FILE);
            plant_special(&claude, form, &root, &format!(".claude.json as {form}"));
            let c2 = cfg.clone();
            let out = deadline(&format!("seed: .claude.json as {form}"), move || {
                seed_first_run_gates_at(&c2, &["/w".to_string()], AuthPremise::Unproven)
            });
            assert!(matches!(out, SeedOutcome::Refused(_)), ".claude.json as {form}: {out:?}");
            assert!(!cfg.join(FIRST_RUN_SEED_BACKUP).exists(), ".claude.json as {form}: 백업 없음");
            let _ = std::fs::remove_dir_all(&root);
        }
        // ★T2i(RULINGS FP2): 데몬이 듣기 전의 저널 복구 — 정상 저널(인덱스 + 백업)에서 인덱스 · 백업 · `.pack-version` 자리에 같은
        //   형태를 두어도 복구는 시한 안에 돌아온다(복구의 순서 · 판정은 그대로 — 판독 불가 = 종전 읽기 오류 갈래).
        // ★T9 stage 2 (RULINGS GX1–GX3, round 2 HE2): the journal lives in the lane's slot under the NAME of a gone writer
        //   (`j.<my pid>.1.<id>`), so the recovery takes it and reaches every read of its path; the targets are the index,
        //   a backup, the pack's version and commit marker, and the slot ENTRY itself (a special file at the journal's name).
        for (ti, target) in [
            "index.json",
            "files/a.txt",
            "../../../../pack/.pack-version",
            "../../../../pack/.pack-applied.json",
            "<entry>",
        ]
        .iter()
        .enumerate()
        {
            for form in &forms {
                let root = rig.base.join(format!("j{ti}-{form}"));
                let pack = root.join("pack");
                std::fs::create_dir_all(&pack).unwrap();
                std::fs::write(pack.join("a.txt"), "new").unwrap();
                std::fs::write(pack.join(PACK_VERSION_FILE), "1.0.0
").unwrap();
                let jdir = super::tests::t9_hand_journal_in(&pack_journal_dir_for(&pack));
                std::fs::write(
                    jdir.join("index.json"),
                    r#"{"target_version":"2.0.0","entries":[{"rel":"a.txt","existed":true,"mode":null}]}"#,
                )
                .unwrap();
                std::fs::write(jdir.join("files").join("a.txt"), "old").unwrap();
                super::tests::t9_add_apply_record(&jdir);
                let p = if *target == "<entry>" {
                    std::fs::remove_dir_all(&jdir).unwrap();
                    jdir.clone()
                } else {
                    jdir.join(target)
                };
                plant_special(&p, form, &root, &format!("journal {target} as {form}"));
                let _env = EnvGuard::set("CYS_PACK_DIR", &pack);
                let _ = deadline(&format!("journal {target} as {form}"), recover_pack_journal);
                let _ = std::fs::remove_dir_all(&root);
            }
        }
        // 잠금 파일 자리의 FIFO → 잠금 없이 진행(종전 열기 실패와 같은 갈래) · 병합은 끝난다.
        #[cfg(unix)]
        {
            let root = rig.base.join("lock-fifo");
            let cfg = root.join("cfg");
            std::fs::create_dir_all(&cfg).unwrap();
            let settings = cfg.join("settings.json");
            std::fs::write(&settings, "{}").unwrap();
            plant_special(&cfg.join("settings.json.cys-lock"), &"fifo", &root, "lock as fifo");
            let (s, pk) = (settings.clone(), root.join("pack"));
            let r = deadline("merge: lock as fifo", move || merge_desired_hooks(&s, &pk, &AWAKENING_HOOKS));
            assert!(r.is_ok(), "{r:?}");
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// ★T2i(RULINGS FH2): 시작 경로의 판독기는 막히지 않고 상한 없이 읽지 않는다 — **최종 대상이 상한 이하의 일반 파일인 링크는
    /// 따른다**(출시본이 따르던 곳: `read_start_live` 를 쓰는 곳 — 설정 파일 판독 `settings_read_bounded` · `verify_desired_hooks_registered`
    /// · 첫기동 관문 시드의 `.claude.json`, 살아 있는 팩 파일의 계획 · 설치 · 스냅샷 · vendor 우선 판독, `ceo_promoted_in`, 부서 soul
    /// 시드, `byte_backup_user` 의 원본). 그 밖을 가리키는 링크(FIFO · 장치 · 폴더 · 상한 초과)는 '판독 불가' — 그 판독의 기존 안전값
    /// (설정 = 덮지 않음 · 검증 = 전부 미등록으로 보고 · 시드 = 거부). 끊긴 링크는 출시본과 같이 '없음'(그 판독의 기존 '없음' 갈래).
    /// `merge_desired_hooks` 는 출시본처럼 링크인 설정 파일을 쓰지 않는다(거부).
    #[cfg(unix)]
    #[test]
    fn fh2_a_link_to_a_regular_file_is_followed_and_a_link_to_anything_else_is_unreadable() {
        let rig = Rig::new("fh2");
        let root = rig.base.join("fh2");
        std::fs::create_dir_all(&root).unwrap();
        let pk = root.join("pack");
        // 판독기 자체.
        std::fs::write(root.join("real.json"), "{}").unwrap();
        std::os::unix::fs::symlink(root.join("real.json"), root.join("l-real")).unwrap();
        assert_eq!(read_start_live(&root.join("l-real")), StartRead::Bytes(b"{}".to_vec()), "일반 파일 링크는 따른다");
        assert!(matches!(read_start_meta(&root.join("l-real")), StartRead::Unreadable(_)), "메타 판독기는 링크를 따르지 않는다");
        let fifo = root.join("f.fifo");
        {
            use std::os::unix::ffi::OsStrExt;
            let c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        }
        std::fs::create_dir_all(root.join("adir")).unwrap();
        std::fs::File::create(root.join("big")).unwrap().set_len(START_LIVE_CAP + 1).unwrap();
        let others: [(&str, PathBuf); 4] = [
            ("fifo", fifo.clone()),
            ("device", PathBuf::from("/dev/zero")),
            ("dir", root.join("adir")),
            ("oversized", root.join("big")),
        ];
        for (what, target) in &others {
            let l = root.join(format!("l-{what}"));
            std::os::unix::fs::symlink(target, &l).unwrap();
            let l2 = l.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(read_start_live(&l2));
            });
            let got = rx
                .recv_timeout(std::time::Duration::from_secs(20))
                .unwrap_or_else(|_| panic!("link to {what}: the read did not return within its deadline"));
            assert!(matches!(got, StartRead::Unreadable(_)), "link to {what}: {got:?}");
        }
        std::os::unix::fs::symlink(root.join("nowhere"), root.join("l-dangling")).unwrap();
        assert_eq!(read_start_live(&root.join("l-dangling")), StartRead::Missing, "끊긴 링크 = 출시본과 같은 '없음'");
        // 설정 파일: 등록이 다 된 일반 파일을 가리키는 링크 → 검증은 그 내용을 읽는다(따름).
        let cfg = root.join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let full = root.join("full-settings.json");
        merge_desired_hooks(&full, &pk, &AWAKENING_HOOKS).unwrap();
        let settings = cfg.join("settings.json");
        std::os::unix::fs::symlink(&full, &settings).unwrap();
        assert!(verify_desired_hooks_registered(&settings, &pk, &AWAKENING_HOOKS).is_empty(), "링크를 따라 등록을 읽었다");
        assert!(merge_desired_hooks(&settings, &pk, &AWAKENING_HOOKS).is_err(), "링크인 설정 파일은 쓰지 않는다(출시본 그대로)");
        // 그 밖을 가리키는 링크 → 판독 불가의 안전값(검증 = 전부 미등록 · 재조정 = 오류 · 덮지 않음).
        for (what, target) in &others {
            std::fs::remove_file(&settings).unwrap();
            std::os::unix::fs::symlink(target, &settings).unwrap();
            let (s, pk2) = (settings.clone(), pk.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let v = verify_desired_hooks_registered(&s, &pk2, &AWAKENING_HOOKS).len();
                let r = retune_registered_hook_timeouts(&s, &pk2, &AWAKENING_HOOKS).is_err();
                let _ = tx.send((v, r));
            });
            let (missing, retune_err) = rx
                .recv_timeout(std::time::Duration::from_secs(20))
                .unwrap_or_else(|_| panic!("settings link to {what}: did not return within its deadline"));
            assert_eq!(missing, AWAKENING_HOOKS.len(), "settings link to {what}: 전부 미등록으로 보고");
            assert!(retune_err, "settings link to {what}: 재조정은 오류(덮지 않음)");
        }
        // 첫기동 관문 시드의 `.claude.json`: 일반 파일 링크는 읽고(판독 거부가 아님), FIFO 링크는 거부.
        let claude = cfg.join(CLAUDE_CONFIG_FILE);
        std::fs::write(root.join("claude-real.json"), "{}").unwrap();
        std::os::unix::fs::symlink(root.join("claude-real.json"), &claude).unwrap();
        let out = seed_first_run_gates_at(&cfg, &["/w".to_string()], AuthPremise::Unproven);
        assert!(!matches!(&out, SeedOutcome::Refused(why) if why.contains("읽기 불가")), "일반 파일 링크는 읽는다: {out:?}");
        let _ = std::fs::remove_file(&claude);
        let _ = std::fs::remove_file(cfg.join(FIRST_RUN_SEED_BACKUP));
        std::os::unix::fs::symlink(&fifo, &claude).unwrap();
        let c2 = cfg.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(seed_first_run_gates_at(&c2, &["/w".to_string()], AuthPremise::Unproven));
        });
        let out = rx.recv_timeout(std::time::Duration::from_secs(20)).expect(".claude.json link to fifo: deadline");
        assert!(matches!(&out, SeedOutcome::Refused(why) if why.contains("읽기 불가")), "{out:?}");
    }

    /// EC5 시험의 형태 심기 — 폴더 · 상한 초과(성긴 파일) · FIFO · FIFO 를 가리키는 링크 · 장치(`/dev/zero`)를 가리키는 링크.
    fn plant_special(p: &Path, form: &str, root: &Path, case: &str) {
        let _ = std::fs::remove_file(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        match form {
            "dir" => std::fs::create_dir_all(p).unwrap(),
            "oversized" => std::fs::File::create(p).unwrap().set_len(START_LIVE_CAP.max(START_META_CAP) + 1).unwrap(),
            #[cfg(unix)]
            "fifo" | "link-to-fifo" => {
                use std::os::unix::ffi::OsStrExt;
                let fifo = if form == "fifo" { p.to_path_buf() } else { root.join(format!("elsewhere-{}.fifo", p.file_name().unwrap().to_string_lossy())) };
                let c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0, "{case}: mkfifo");
                if form == "link-to-fifo" {
                    std::os::unix::fs::symlink(&fifo, p).unwrap();
                }
            }
            #[cfg(unix)]
            "link-to-device" => std::os::unix::fs::symlink("/dev/zero", p).unwrap(),
            _ => unreachable!("{form}"),
        }
    }

    /// ★T2i(RULINGS EC5 · FP2): 시작 경로 판독 조사(census) — 데몬이 듣기 **전**에 도는 팩 코드의 실제 뿌리(`cysd` main 의
    /// `recover_pack_journal` · `pack_current_for` · `install`)에서 **호출 그래프를 따라** 닿는 함수(이 파일과 `packsig.rs` 의
    /// 시험 밖 함수, 이름으로 해소 — 넘치게 잡는 쪽) 가운데 날것의 경로 판독(`std::fs::read(` · `std::fs::read_to_string(` ·
    /// `fs::read_to_string(` · `std::fs::copy(` · `File::open(`)이 있는 곳은 없어야 한다. 예외 둘: `write_atomic_mode_inner` 와
    /// `sync_parent_dir` 의 `File::open(parent)` = 폴더를 내리려고 여는 것(내용 판독 아님). 새 판독 함수 · 새 호출이 생기면 이 조사가
    /// 저절로 잡는다(이 Mac 에서 닿는 함수 165개 — 파이썬 이식으로 같은 규칙을 미리 잰 값, `impl/runs/T2-scratch/census_port.py`).
    #[test]
    fn ec5_start_path_read_census() {
        let pack_src = include_str!("pack.rs");
        let sig_src = include_str!("packsig.rs");
        // 문자열 · 문자 · 주석을 지운 사본(괄호 짝 맞추기용 — 길이 보존).
        fn blank(src: &str) -> String {
            let b = src.as_bytes();
            let mut out = b.to_vec();
            let mut i = 0;
            while i < b.len() {
                match b[i] {
                    b'/' if b.get(i + 1) == Some(&b'/') => {
                        while i < b.len() && b[i] != b'\n' {
                            out[i] = b' ';
                            i += 1;
                        }
                    }
                    b'/' if b.get(i + 1) == Some(&b'*') => {
                        while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                            if b[i] != b'\n' {
                                out[i] = b' ';
                            }
                            i += 1;
                        }
                        out[i] = b' ';
                        if i + 1 < b.len() {
                            out[i + 1] = b' ';
                        }
                        i += 2;
                    }
                    b'r' if (b.get(i + 1) == Some(&b'"') || b.get(i + 1) == Some(&b'#'))
                        && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) =>
                    {
                        let mut j = i + 1;
                        let mut hashes = 0;
                        while b.get(j) == Some(&b'#') {
                            hashes += 1;
                            j += 1;
                        }
                        if b.get(j) != Some(&b'"') {
                            i += 1;
                            continue;
                        }
                        j += 1;
                        loop {
                            if j >= b.len() {
                                break;
                            }
                            if b[j] == b'"' && (0..hashes).all(|h| b.get(j + 1 + h) == Some(&b'#')) {
                                j += 1 + hashes;
                                break;
                            }
                            j += 1;
                        }
                        for o in out.iter_mut().take(j).skip(i) {
                            if *o != b'\n' {
                                *o = b' ';
                            }
                        }
                        i = j;
                    }
                    b'"' => {
                        out[i] = b' ';
                        i += 1;
                        while i < b.len() && b[i] != b'"' {
                            if b[i] == b'\\' {
                                out[i] = b' ';
                                i += 1;
                            }
                            if i < b.len() && b[i] != b'\n' {
                                out[i] = b' ';
                            }
                            i += 1;
                        }
                        if i < b.len() {
                            out[i] = b' ';
                        }
                        i += 1;
                    }
                    b'\'' => {
                        // 문자 리터럴('x' · '\n' · '\u{..}')만 지운다 — 수명('a)은 그대로.
                        let end = if b.get(i + 1) == Some(&b'\\') {
                            (i + 2..b.len().min(i + 12)).find(|&k| b[k] == b'\'')
                        } else if b.get(i + 2) == Some(&b'\'') {
                            Some(i + 2)
                        } else {
                            std::str::from_utf8(&b[i + 1..b.len().min(i + 6)])
                                .ok()
                                .or_else(|| std::str::from_utf8(&b[i + 1..b.len().min(i + 5)]).ok())
                                .and_then(|s| s.chars().next())
                                .map(|c| i + 1 + c.len_utf8())
                                .filter(|&k| b.get(k) == Some(&b'\''))
                        };
                        match end {
                            Some(e) => {
                                for o in out.iter_mut().take(e + 1).skip(i) {
                                    *o = b' ';
                                }
                                i = e + 1;
                            }
                            None => i += 1,
                        }
                    }
                    _ => i += 1,
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        }
        // 시험 모듈(`#[cfg(test)] mod … { … }`)의 바이트 구간.
        fn test_regions(code: &str) -> Vec<(usize, usize)> {
            let mut out = Vec::new();
            let mut from = 0;
            while let Some(k) = code[from..].find("#[cfg(test)]") {
                let at = from + k;
                let rest = &code[at..];
                let Some(m) = rest.find("mod ") else { break };
                if rest[..m].contains(';') || rest[..m].contains("fn ") {
                    from = at + 12;
                    continue;
                }
                let Some(open) = rest.find('{') else { break };
                let mut depth = 0i32;
                let mut end = rest.len();
                for (i, ch) in rest[open..].char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = open + i + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                out.push((at, at + end));
                from = at + end;
            }
            out
        }
        // 이름 → 몸통(원문) 목록.
        fn functions(src: &str) -> Vec<(String, String)> {
            let code = blank(src);
            let tests = test_regions(&code);
            let mut out = Vec::new();
            let bytes = code.as_bytes();
            let mut pos = 0;
            while let Some(k) = code[pos..].find("fn ") {
                let at = pos + k;
                pos = at + 3;
                if at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_') {
                    continue;
                }
                if tests.iter().any(|&(s, e)| at >= s && at < e) {
                    continue;
                }
                let name: String = code[at + 3..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if name.is_empty() {
                    continue;
                }
                let rest = &code[at..];
                let (Some(open), semi) = (rest.find('{'), rest.find(';')) else { continue };
                if semi.is_some_and(|s| s < open) {
                    continue;
                }
                let mut depth = 0i32;
                let mut end = None;
                for (i, ch) in rest[open..].char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = Some(open + i + 1);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let Some(end) = end else { continue };
                out.push((name, src[at..at + end].to_string()));
            }
            out
        }
        let mut fns: Vec<(String, String, &str)> = Vec::new();
        for (n, b) in functions(pack_src) {
            fns.push((n, b, "pack.rs"));
        }
        for (n, b) in functions(sig_src) {
            fns.push((n, b, "packsig.rs"));
        }
        let names: std::collections::BTreeSet<String> = fns.iter().map(|(n, _, _)| n.clone()).collect();
        let calls_of = |body: &str| -> Vec<String> {
            let code = blank(body);
            let mut out = Vec::new();
            let b = code.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if b[i].is_ascii_alphabetic() || b[i] == b'_' {
                    let s = i;
                    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                        i += 1;
                    }
                    let mut j = i;
                    while j < b.len() && b[j] == b' ' {
                        j += 1;
                    }
                    if b.get(j) == Some(&b'(') {
                        out.push(code[s..i].to_string());
                    }
                } else {
                    i += 1;
                }
            }
            out
        };
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut stack: Vec<String> = ["recover_pack_journal", "pack_current_for", "install"].iter().map(|s| s.to_string()).collect();
        for r in &stack {
            assert!(names.contains(r), "census root {r} not found");
        }
        while let Some(n) = stack.pop() {
            if !seen.insert(n.clone()) {
                continue;
            }
            for (fname, body, _) in fns.iter().filter(|(f, _, _)| *f == n) {
                let _ = fname;
                for c in calls_of(body) {
                    if names.contains(&c) && !seen.contains(&c) {
                        stack.push(c);
                    }
                }
            }
        }
        for must in ["install_into_with", "channel_guard_and_heal", "read_accepted", "rollback_journal", "merge_desired_hooks", "park_copy_at"] {
            assert!(seen.contains(must), "census must reach {must} (reached {})", seen.len());
        }
        let raw = ["std::fs::read(", "std::fs::read_to_string(", "fs::read_to_string(", "std::fs::copy(", "File::open("];
        let mut bad = Vec::new();
        for (n, body, file) in fns.iter().filter(|(n, _, _)| seen.contains(n)) {
            let code = blank(body);
            for r in raw {
                let dir_open = r == "File::open(" && (n == "write_atomic_mode_inner" || n == "sync_parent_dir");
                if code.contains(r) && !dir_open {
                    bad.push(format!("{file}::{n} {r}"));
                }
            }
        }
        assert!(bad.is_empty(), "unbounded reads reachable before the daemon listens ({} functions): {bad:?}", seen.len());
    }

    /// ★T2h(RULINGS CW3 · 검토자 B3): 시작 경로(데몬의 기동 설치 — 바인드 전)는 장부·메타 파일 하나로 멈추지 않는다 — 감사 원장 ·
    /// 병합 대기 원장 · 설치 기록 · `.pack-version` · `.pack-state.json` · `.pristine/<rel>` · `<rel>.user` · `<rel>.new` 자리에
    /// FIFO(쓰는 쪽 없음) · 폴더 · FIFO 를 가리키는 링크 · 상한 초과 파일(성긴 파일)을 두어도 설치는 **각자의 시한 안에** 끝난다
    /// (시한을 넘기면 실패 — 회귀가 스위트를 멈추지 않는다). 감사 원장을 읽을 수 없으면 보수적 기본값: user 소유 파일을 자동
    /// 갱신하지 않는다(대조: 감사 원장이 없으면 갱신한다).
    #[test]
    fn t2h_cw3_start_path_never_blocks_on_a_special_or_oversized_ledger() {
        let rig = Rig::new("cw3");
        let agents_old = "{\"agents\": 1}\n";
        let agents_new = "{\"agents\": 2}\n";
        let items: Vec<(String, String)> = vec![
            ("agents.json".into(), agents_new.into()),
            ("hooks/inject.sh".into(), "#!/bin/bash\necho V1\n".into()),
            ("bin/drift.py".into(), "VENDOR-NEW\n".into()),
            ("bin/kept.py".into(), "line1\nVENDOR\nline3\n".into()),
        ];
        let setup = |pack: &Path| {
            let w = |rel: &str, c: &str| {
                let p = pack.join(rel);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, c).unwrap();
            };
            w("agents.json", agents_old);
            w("hooks/inject.sh", "#!/bin/bash\necho USER\n");
            w("bin/drift.py", "DRIFT\n");
            w(".pristine/bin/drift.py", "BASE\n");
            w("bin/kept.py", "line1\nMINE\nline3\n");
            w("bin/kept.py.new", "line1\nVENDOR\nline3\n");
            w(".pack-version", "1.0.0\n");
            let m: std::collections::BTreeMap<String, String> = [
                ("agents.json", content_hash(agents_old)),
                ("bin/drift.py", content_hash("BASE\n")),
                ("bin/kept.py", content_hash("line1\nline2\nline3\n")),
            ]
            .into_iter()
            .map(|(r, h)| (r.to_string(), h))
            .collect();
            std::fs::write(pack.join(INSTALL_MANIFEST), serde_json::to_string(&m).unwrap()).unwrap();
        };
        let run = |case: &str, pack: PathBuf| -> bool {
            let (tx, rx) = std::sync::mpsc::channel();
            let it = items.clone();
            std::thread::spawn(move || {
                let src = UpdateSources::none().with_boot_critical(bc_only(&["hooks/inject.sh"]));
                let r = install_into_with(
                    pack,
                    it.iter().map(|(a, b)| (a.as_str(), b.as_str())),
                    false,
                    "9.9.9",
                    false,
                    false,
                    PackScope::Base,
                    None,
                    None,
                    &src,
                );
                let _ = tx.send(r.is_ok());
            });
            rx.recv_timeout(std::time::Duration::from_secs(20))
                .unwrap_or_else(|_| panic!("{case}: the install did not finish within its deadline (blocked on a start-path read)"))
        };
        // 대조: 감사 원장이 없으면 user 소유 비수정 파일은 갱신된다.
        let ctl = rig.base.join("control");
        setup(&ctl);
        assert!(run("control", ctl.clone()), "control install");
        assert_eq!(std::fs::read_to_string(ctl.join("agents.json")).unwrap(), agents_new, "control: 갱신");
        let targets = [
            MERGE_AUDIT_FILE,
            MERGE_PENDING_FILE,
            INSTALL_MANIFEST,
            PACK_VERSION_FILE,
            PACK_STATE_FILE,
            ".pristine/bin/drift.py",
            "hooks/inject.sh.user",
            "bin/kept.py.new",
        ];
        let mut forms: Vec<&str> = vec!["dir", "oversized"];
        #[cfg(unix)]
        forms.extend(["fifo", "link-to-fifo"]);
        let mut cases = 0usize;
        for (ti, target) in targets.iter().enumerate() {
            for form in &forms {
                let case = format!("{target} as {form}");
                let pack = rig.base.join(format!("c{ti}-{form}"));
                setup(&pack);
                let p = pack.join(target);
                let _ = std::fs::remove_file(&p);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                match *form {
                    "dir" => std::fs::create_dir_all(&p).unwrap(),
                    "oversized" => std::fs::File::create(&p).unwrap().set_len(START_META_CAP + 1).unwrap(),
                    #[cfg(unix)]
                    "fifo" | "link-to-fifo" => {
                        use std::os::unix::ffi::OsStrExt;
                        let fifo = if *form == "fifo" { p.clone() } else { pack.join("elsewhere.fifo") };
                        let c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
                        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0, "{case}: mkfifo");
                        if *form == "link-to-fifo" {
                            std::os::unix::fs::symlink(&fifo, &p).unwrap();
                        }
                    }
                    _ => unreachable!(),
                }
                let _finished_ok = run(&case, pack.clone());
                if *target == MERGE_AUDIT_FILE {
                    assert_eq!(
                        std::fs::read_to_string(pack.join("agents.json")).unwrap(),
                        agents_old,
                        "{case}: 감사 원장을 읽을 수 없으면 user 소유 파일을 자동 갱신하지 않는다"
                    );
                }
                cases += 1;
                let _ = std::fs::remove_dir_all(&pack);
            }
        }
        assert_eq!(cases, targets.len() * forms.len());
        // 판독기 자체: 일반 파일만 · 링크 비추종(메타) · 링크 추종(살아 있는 파일) · 부재.
        let dir = rig.base.join("reader");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f"), "x").unwrap();
        assert_eq!(read_start_meta(&dir.join("f")), StartRead::Bytes(b"x".to_vec()));
        assert_eq!(read_start_meta(&dir.join("absent")), StartRead::Missing);
        assert!(matches!(read_start_meta(&dir), StartRead::Unreadable(_)), "폴더");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("f"), dir.join("l")).unwrap();
            assert!(matches!(read_start_meta(&dir.join("l")), StartRead::Unreadable(_)), "메타: 링크 비추종");
            assert_eq!(read_start_live(&dir.join("l")), StartRead::Bytes(b"x".to_vec()), "살아 있는 파일: 일반 파일 링크 추종");
        }
    }

    /// ★T2h(RULINGS CW2 · 검토자 B2): 보관된 세대는 결코 덮이지 않는다 — 두 쓰는 쪽을 결정적으로 끼워 넣는다: A 가 `.user`(U)를
    /// `prev-100` 에 보존하고 V 를 쓴 뒤, 같은 초의 B 가 고른 후보 자리를 **예약 직전에** 세 번째 쓰는 쪽(X)이 차지한다(종전 코드는
    /// 존재 확인 뒤 `rename` 으로 그 자리를 덮었다). B 는 다음 자리로 넘어가고 U · V · X 가 모두 남는다. 다른 쓰는 쪽이 이미 `.user`
    /// 를 옮겼으면 보존할 것이 없다.
    #[test]
    fn t2h_cw2_an_archived_generation_is_never_replaced() {
        let rig = Rig::new("cw2");
        let rel = "hooks/x.sh";
        let user = |c: &[u8]| rig.write(&format!("{rel}.user"), c);
        user(b"U\n");
        let a = preserve_prior_user_generation_at(&rig.pack, rel, Some(b"V\n"), 100, &mut |_| {}).unwrap().unwrap();
        assert_eq!(a, rig.pack.join(format!("{rel}.user.prev-100")));
        user(b"V\n");
        let mut taken: Vec<PathBuf> = Vec::new();
        let b = preserve_prior_user_generation_at(&rig.pack, rel, Some(b"W\n"), 100, &mut |cand| {
            if taken.is_empty() && !cand.exists() {
                std::fs::write(cand, "X\n").unwrap();
                taken.push(cand.to_path_buf());
            }
        })
        .unwrap()
        .unwrap();
        assert_eq!(taken.len(), 1, "끼어든 쓰는 쪽이 B 의 첫 후보를 차지했다");
        assert_ne!(b, taken[0], "B 는 차지된 자리를 쓰지 않는다");
        let read = |p: &Path| std::fs::read_to_string(p).unwrap();
        assert_eq!(read(&a), "U\n", "A 의 세대(U) 그대로");
        assert_eq!(read(&taken[0]), "X\n", "끼어든 세대(X) 그대로");
        assert_eq!(read(&b), "V\n", "B 의 세대(V)");
        let gens: Vec<String> = std::fs::read_dir(rig.pack.join("hooks"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("x.sh.user.prev-"))
            .map(|e| read(&e.path()))
            .collect();
        assert_eq!(gens.len(), 3, "{gens:?}");
        // 다른 쓰는 쪽이 이미 옮겼다 → 보존할 것 없음.
        assert_eq!(preserve_prior_user_generation_at(&rig.pack, rel, None, 100, &mut |_| {}).unwrap(), None);
    }

    /// ★T2h(RULINGS CW1): 닻 내린 쓰기와 경로 규칙 — 구성 요소가 링크면 쓰지 않는다(가리키는 곳에 아무것도 생기지 않는다),
    /// 마지막 성분이 링크면 그 링크만 교체된다(가리키는 파일 무접촉), 없는 구성 요소는 만든다. 스냅샷 레인이 링크면 스냅샷을
    /// 만들지 않는다(링크 너머의 정리 · 쓰기 없음).
    #[cfg(unix)]
    #[test]
    fn t2h_cw1_anchored_writes_never_follow_a_link() {
        let rig = Rig::new("cw1");
        let live = rig.pack.join("hooks");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::write(live.join("guard.sh"), "LIVE\n").unwrap();
        std::fs::create_dir_all(rig.pack.join(PRISTINE_DIR)).unwrap();
        std::os::unix::fs::symlink(&live, rig.pack.join(PRISTINE_DIR).join("hooks")).unwrap();
        assert!(sidecar_path_gate(&rig.pack, ".pristine/hooks/guard.sh").is_err(), "부모 링크 → 거부");
        assert!(write_beneath(&rig.pack, ".pristine/hooks/guard.sh", b"X\n").is_err(), "닻 내린 쓰기 → 실패");
        assert_eq!(std::fs::read_to_string(live.join("guard.sh")).unwrap(), "LIVE\n", "링크 너머 무접촉");
        assert_eq!(std::fs::read_dir(&live).unwrap().count(), 1, "링크 너머에 임시 파일도 없다");
        // 마지막 성분이 링크 → 그 링크만 교체.
        std::fs::remove_file(rig.pack.join(PRISTINE_DIR).join("hooks")).unwrap();
        std::fs::create_dir_all(rig.pack.join(PRISTINE_DIR).join("hooks")).unwrap();
        std::os::unix::fs::symlink(live.join("guard.sh"), rig.pack.join(PRISTINE_DIR).join("hooks/guard.sh")).unwrap();
        assert!(sidecar_path_gate(&rig.pack, ".pristine/hooks/guard.sh").is_err(), "마지막 성분 링크 → 관문 거부");
        write_beneath(&rig.pack, ".pristine/hooks/guard.sh", b"BASE\n").unwrap();
        assert_eq!(std::fs::read_to_string(live.join("guard.sh")).unwrap(), "LIVE\n", "가리키던 파일 무접촉");
        assert_eq!(std::fs::read_to_string(rig.pack.join(".pristine/hooks/guard.sh")).unwrap(), "BASE\n");
        // 없는 구성 요소는 만든다.
        write_beneath(&rig.pack, ".pristine/bin/new/x.py", b"Y\n").unwrap();
        assert_eq!(std::fs::read_to_string(rig.pack.join(".pristine/bin/new/x.py")).unwrap(), "Y\n");
        // 스냅샷 레인이 링크 → 만들지 않는다.
        let root = rig.base.join("snaps");
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(&live, root.join("base")).unwrap();
        std::fs::write(rig.pack.join("hooks/guard.sh"), "LIVE\n").unwrap();
        let none = |_: &str| Vec::<String>::new();
        let r = create_update_snapshot(&rig.pack, &root, "base", "1.0.0", &std::collections::BTreeMap::new(),
            &["hooks/guard.sh".to_string()], PackScope::Base, &none, (UPDATE_SNAPSHOT_FILE_CAP, UPDATE_SNAPSHOT_TOTAL_CAP));
        assert!(r.is_err(), "레인 링크 → 스냅샷 거부");
        assert_eq!(std::fs::read_dir(&live).unwrap().count(), 1, "링크 너머(살아 있는 hooks)에 아무것도 생기지 않았다");
    }

    /// 부록 A-2 #4(백업이 없으면 덮지 않는다): 기동 필수 파일의 사용자본을 보관할 수 없으면 vendor 본을 쓰지 않고 격리.
    #[cfg(unix)]
    #[test]
    fn m11_vendor_first_backup_failure_quarantines() {
        use std::os::unix::fs::PermissionsExt;
        let rig = Rig::new("bkfail");
        let src = UpdateSources::none().with_boot_critical(bc_only(&["hooks/only.sh"]));
        rig.write("hooks/only.sh", b"#!/bin/bash\necho USER\n");
        std::fs::set_permissions(rig.pack.join("hooks"), std::fs::Permissions::from_mode(0o555)).unwrap();
        rig.install(&[("hooks/only.sh", "#!/bin/bash\necho VENDOR\n")], false, &src);
        std::fs::set_permissions(rig.pack.join("hooks"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(rig.read("hooks/only.sh").as_deref(), Some("#!/bin/bash\necho USER\n"), "사본 없이 덮지 않는다");
        assert_eq!(rig.kind("hooks/only.sh").as_deref(), Some("quarantined"));
    }

    /// 마: 묘비가 있는 삭제만 유지되고, 묘비가 없으면(또는 풀리면) 지금처럼 돌아온다. 생명줄 여섯은 묘비를 무시하고
    /// 언제나 되살아난다(F2/F3). 훅 묘비는 파일 경로를 걷는 판정이 종류로 건너뛴다(W3(b)).
    #[test]
    fn m11_tombstones_files_only_lifelines_always_restored() {
        let rig = Rig::new("tomb");
        let mut items: Vec<(String, String)> = vec![("bin/gone.py".into(), "V\n".into())];
        for h in crate::repair::boot_critical::LIFELINE_HOOKS {
            items.push((format!("hooks/{h}"), format!("#!/bin/bash\necho {h}\n")));
        }
        let items_ref: Vec<(&str, &str)> = items.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let mut ts = vec![tomb("bin/gone.py", "R-7"), hook_tomb("PostToolUse", "pack-guard.sh", "R-8")];
        for h in crate::repair::boot_critical::LIFELINE_HOOKS {
            ts.push(tomb(&format!("hooks/{h}"), "R-9"));
            ts.push(hook_tomb("SessionStart", h, "R-9"));
        }
        let with = UpdateSources::none().with_tombstones(ts);
        assert_eq!(with.authorised_absent(), ["bin/gone.py".to_string()].into_iter().collect(),
                   "허가된 부재 = 생명줄이 아닌 파일 묘비뿐(훅 묘비·생명줄 묘비는 걸러진다)");
        rig.install(&items_ref, false, &with);
        assert!(!rig.pack.join("bin/gone.py").exists(), "기록된 삭제 유지");
        for h in crate::repair::boot_critical::LIFELINE_HOOKS {
            assert!(rig.pack.join("hooks").join(h).is_file(), "생명줄 {h} 은 묘비가 있어도 되살린다");
        }
        // 묘비가 없으면(되돌림 뒤) 지금처럼 되살아난다.
        rig.install(&items_ref, false, &UpdateSources::none());
        assert_eq!(rig.read("bin/gone.py").as_deref(), Some("V\n"), "묘비 없음 = 지금 동작(되살림)");
        // 묘비 없는 부재(편집기로 지움)는 돌아온다.
        std::fs::remove_file(rig.pack.join("bin/gone.py")).unwrap();
        rig.install(&items_ref, false, &UpdateSources::none());
        assert_eq!(rig.read("bin/gone.py").as_deref(), Some("V\n"), "기록 없는 삭제는 되살린다");
    }

    /// ★T2j(RULINGS HP4 (a) · 설계 8-1 D⑥ "보호장치 넷을 없애는 후보가 거부됨"): 보호장치 파일은 승인된 수리로도 지울 수 없다 — 그래서
    /// 그 파일의 묘비는 정당할 수 없고, 설치는 생명줄처럼 따르지 않는다(파일은 되살아난다). 기동 필수 파일(보호장치 아님 · 생명줄 아님)의
    /// 묘비는 따른다 — 승인된 수리의 삭제가 업데이트 뒤에도 남는다는 계약(CONTRACTS 1.5 · 8.7)을 깨지 않는다. 일반 파일도 종전 그대로.
    #[test]
    fn t2j_hp4_tombstone_of_a_protection_file_is_void_at_install() {
        use crate::repair::boot_critical::{boot_critical_entry, protection_entry};
        let rig = Rig::new("hp4");
        let prot = ["hooks/guard.sh", "trusted-keys.json", "bin/javis_guard_register.py"];
        let boot = "bin/javis_bootstrap.py";
        for r in prot {
            assert!(protection_entry(r).is_some() && !is_lifeline_rel(r), "{r}: 컴파일된 보호 목록의 행(생명줄 아님)");
        }
        assert!(boot_critical_entry(boot).is_some() && protection_entry(boot).is_none() && !is_lifeline_rel(boot), "{boot}: 기동 필수만");
        let mut items: Vec<(&str, &str)> = prot.iter().map(|r| (*r, "V\n")).collect();
        items.push((boot, "B\n"));
        items.push(("bin/gone.py", "G\n"));
        let mut ts: Vec<Tombstone> = prot.iter().map(|r| tomb(r, "R-5")).collect();
        ts.push(tomb(boot, "R-6"));
        ts.push(tomb("bin/gone.py", "R-7"));
        let src = UpdateSources::none().with_tombstones(ts);
        assert_eq!(
            src.void_tombstones(),
            prot.iter().map(|r| (r.to_string(), "R-5".to_string())).collect::<Vec<_>>().as_slice(),
            "따르지 않은 묘비 = 보호장치 파일의 것(설치 보고가 이름 댄다)"
        );
        assert_eq!(
            src.authorised_absent(),
            [boot, "bin/gone.py"].iter().map(|r| r.to_string()).collect(),
            "허가된 부재 = 기동 필수 · 일반 파일의 묘비"
        );
        rig.install(&items, false, &src);
        for r in prot {
            assert_eq!(rig.read(r).as_deref(), Some("V\n"), "{r}: 보호장치 파일은 묘비가 있어도 되살린다");
        }
        assert!(!rig.pack.join(boot).exists(), "{boot}: 기동 필수 파일의 묘비는 따른다(승인된 수리의 삭제 — CONTRACTS 1.5 · 8.7)");
        assert!(!rig.pack.join("bin/gone.py").exists(), "일반 파일의 묘비는 따른다(종전)");
    }

    /// 부록 A-2 #2: 문서화된 초기화('지운 뒤 init-pack')는 묘비 없이 그대로 동작하고, seed-once 는 덮지 않는다.
    #[test]
    fn m11_seed_once_documented_reset_still_reseeds() {
        let rig = Rig::new("seed");
        let items = [("memory/MEMORY.md", "SEED\n")];
        rig.write("memory/MEMORY.md", b"MY MEMORY\n");
        rig.install(&items, true, &UpdateSources::none());
        assert_eq!(rig.read("memory/MEMORY.md").as_deref(), Some("MY MEMORY\n"), "seed-once 불가침(force 여도)");
        std::fs::remove_file(rig.pack.join("memory/MEMORY.md")).unwrap();
        rig.install(&items, false, &UpdateSources::none());
        assert_eq!(rig.read("memory/MEMORY.md").as_deref(), Some("SEED\n"), "지운 뒤 init-pack = 다시 심김");
        std::fs::remove_file(rig.pack.join("memory/MEMORY.md")).unwrap();
        rig.install(&items, false, &UpdateSources::none().with_tombstones(vec![tomb("memory/MEMORY.md", "R-4")]));
        assert!(!rig.pack.join("memory/MEMORY.md").exists(), "묘비가 있으면 부재 유지(명시 기록만)");
    }

    /// 마 + 부록 A-2 #13: 준비본 존재 검사는 허가된 부재와 덜 만들어진 준비본을 구분한다.
    #[test]
    fn m11_verify_staging_authorised_absence_vs_half_built() {
        let rig = Rig::new("staging");
        rig.write("a.txt", b"A");
        let items = [("a.txt", "A"), ("b.txt", "B")];
        let none = std::collections::BTreeSet::new();
        assert!(verify_staging_with(&rig.pack, &items, &none).is_err(), "b.txt 누락 = 덜 만들어진 준비본");
        assert!(verify_staging(&rig.pack, &items).is_err(), "옛 이름도 같은 판정");
        let ok: std::collections::BTreeSet<String> = ["b.txt".to_string()].into_iter().collect();
        assert!(verify_staging_with(&rig.pack, &items, &ok).is_ok(), "묘비가 부재를 유지하는 b.txt = 허가된 부재");
        let other: std::collections::BTreeSet<String> = ["c.txt".to_string()].into_iter().collect();
        assert!(verify_staging_with(&rig.pack, &items, &other).is_err(), "다른 파일의 허가로 누락을 덮지 않는다");
    }

    /// 마(실행 권한은 언제나 되살린다 — 부록 A-2 #7): 사용자 우선으로 남은 셸 스크립트도, 설치기가 방금 만든 0644 도
    /// 사용자의 뜻으로 읽지 않는다.
    #[cfg(unix)]
    #[test]
    fn m11_exec_bits_restored_even_on_user_kept() {
        use std::os::unix::fs::PermissionsExt;
        let rig = Rig::new("exec");
        rig.write("bin/tool.sh", b"#!/bin/bash\necho MINE\n");
        std::fs::set_permissions(rig.pack.join("bin/tool.sh"), std::fs::Permissions::from_mode(0o644)).unwrap();
        rig.install(&[("bin/tool.sh", "#!/bin/bash\necho VENDOR\n"), ("bin/fresh.sh", "#!/bin/sh\n:\n")], false, &UpdateSources::none());
        assert_eq!(rig.kind("bin/tool.sh").as_deref(), Some("user-kept"));
        assert_eq!(current_mode(&rig.pack.join("bin/tool.sh")), Some(0o755), "사용자 우선 파일도 실행 권한 복원");
        assert_eq!(current_mode(&rig.pack.join("bin/fresh.sh")), Some(0o755), "새로 쓴 파일 0755");
    }

    /// 앱 온보딩 조건(`src-tauri needs_gui_onboard`)은 각성 훅만 본다 — 그 둘이 생명줄이라 묘비가 결코 닿지 않는다.
    #[test]
    fn m11_awakening_hooks_are_lifelines() {
        for h in AWAKENING_HOOKS {
            assert!(crate::repair::boot_critical::LIFELINE_HOOKS.contains(&h.script),
                    "각성 훅 {} 이 생명줄이 아니다 — 앱 온보딩이 묘비를 만날 수 있다", h.script);
            assert!(is_lifeline_rel(&format!("hooks/{}", h.script)));
        }
    }

    /// 자: `init-pack --force` 는 사용자 수정을 버리지 않는다(일반 = 사용자 우선, 기동 필수 = vendor 우선 + 보관).
    #[test]
    fn m11_force_never_discards_user_edit() {
        let rig = Rig::new("force");
        rig.write("bin/a.py", b"MINE-A\n");
        rig.write("hooks/b.sh", b"#!/bin/bash\necho MINE-B\n");
        let src = UpdateSources::none().with_boot_critical(bc_only(&["hooks/b.sh"]));
        rig.install(&[("bin/a.py", "VENDOR-A\n"), ("hooks/b.sh", "#!/bin/bash\necho VENDOR-B\n")], true, &src);
        assert_eq!(rig.read("bin/a.py").as_deref(), Some("MINE-A\n"), "force 여도 사용자 파일 유지");
        assert_eq!(rig.read("bin/a.py.new").as_deref(), Some("VENDOR-A\n"));
        assert_eq!(rig.read("hooks/b.sh.user").as_deref(), Some("#!/bin/bash\necho MINE-B\n"), "기동 필수도 사용자본 보관");
    }

    /// 가-2 + 사: 판독 불가(CP949) system 파일은 지금처럼 바이트 백업 뒤 vendor 본이되, 두 번 밀려나도 첫 백업이 남는다.
    #[test]
    fn m11_cp949_system_backup_generations() {
        let rig = Rig::new("cp949");
        let items = [("bin/doc.py", "VENDOR\n")];
        rig.write("bin/doc.py", &[0xB0, 0xA1, 0x41, 0x0A]);
        rig.install(&items, false, &UpdateSources::none());
        rig.write("bin/doc.py", &[0xB0, 0xA1, 0x42, 0x0A]);
        rig.install(&items, false, &UpdateSources::none());
        assert_eq!(std::fs::read(rig.pack.join("bin/doc.py.user")).unwrap(), vec![0xB0, 0xA1, 0x42, 0x0A], "B 백업");
        let prev: Vec<_> = std::fs::read_dir(rig.pack.join("bin")).unwrap().flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("doc.py.user.prev-")).collect();
        assert_eq!(prev.len(), 1);
        assert_eq!(std::fs::read(prev[0].path()).unwrap(), vec![0xB0, 0xA1, 0x41, 0x0A], "A 백업이 덮이지 않았다");
        assert_eq!(rig.read("bin/doc.py").as_deref(), Some("VENDOR\n"));
    }

    /// 다(부록 A-2 #1): 설치 기록이 없거나 깨져도, 과거 vendor 본과 같은 바이트는 사용자 수정이 아니다 → 갱신.
    /// 표에 없는 바이트만 사용자 우선으로 보존된다.
    #[test]
    fn m11_manifest_missing_or_corrupt_consults_prev_table_first() {
        for corrupt in [false, true] {
            let rig = Rig::new(if corrupt { "corrupt" } else { "missing" });
            rig.write("bin/old.py", b"RELEASED-0.14.9\n");
            rig.write("bin/mine.py", b"MY EDIT\n");
            if corrupt {
                std::fs::write(rig.pack.join(INSTALL_MANIFEST), b"{ not json").unwrap();
            }
            let src = UpdateSources::none().with_prev_vendor(|rel: &str| {
                if rel == "bin/old.py" { vec![content_hash("RELEASED-0.14.9\n")] } else { Vec::new() }
            });
            rig.install(&[("bin/old.py", "VENDOR-NOW\n"), ("bin/mine.py", "VENDOR-MINE\n")], false, &src);
            assert_eq!(rig.read("bin/old.py").as_deref(), Some("VENDOR-NOW\n"), "옛 vendor 본 → 갱신(corrupt={corrupt})");
            assert!(!rig.pack.join("bin/old.py.user").exists(), "사용자 수정이 아니다 — .user 없음");
            assert_eq!(rig.read("bin/mine.py").as_deref(), Some("MY EDIT\n"), "표에 없는 바이트 = 사용자 수정 보존");
            let reason = rig.entry("bin/mine.py").unwrap()["reason"].as_str().unwrap().to_string();
            assert_eq!(reason, if corrupt { "manifest-corrupt" } else { "no-base" });
        }
    }

    /// ★T8(CCR-2 · RULINGS CB1·CG2·CG3): 가드 4 는 '승격'의 한 뜻을 쓴다 — 조립된 본부 지침(생성된 머리말 + 조립
    /// 구분선 + 사용자 본부 지침)은 `.pre-ceo` 가 지워졌어도 자동 vendor 갱신이 덮지 않는다(LF·CRLF 두 철자).
    /// ① 가드 4 가 정하는 경로: 설치 기록이 이 바이트를 '한 번도 고치지 않은 것'으로 적었다고 둔다 — 제품의 어떤 명령도
    ///    조립 지침의 해시를 설치 기록에 적지 않지만(CG3), 그래야 가드 1–3 이 모두 통과하고 가드 4 하나만 남는다(사실로 확인).
    /// ② 실제 기록(조립 전 vendor 본의 해시): 조립 지침은 '고친 파일'이라 보존 + `.new`.
    /// ③ 대조: 구분선 없는 평범한 MASTER 는 같은 기록·같은 vendor 본에서 갱신된다(가드 4 가 아무것이나 막지 않는다).
    #[test]
    fn t8_composed_master_without_backup_is_not_replaced_by_an_update() {
        let header = "# CEO 머리말 (master of master · 단일소유 강제 · exit 7)\n";
        let body = "# M\nautopilot denylist\n";
        let vendor_new = "# M\nautopilot denylist v2\n";
        let none = serde_json::Map::new();
        for (tag, sep) in [
            ("lf", CEO_SEPARATOR.to_string()),
            ("crlf", CEO_SEPARATOR.replace('\n', "\r\n")),
        ] {
            let composed = format!("{header}{sep}{body}");
            let src = UpdateSources::none();
            // ①
            let rig = Rig::new(&format!("t8-g4-{tag}"));
            rig.write(MASTER_DIRECTIVE_REL, composed.as_bytes());
            rig.manifest(&[(MASTER_DIRECTIVE_REL, composed.as_str())]);
            assert!(
                !ceo_pre_ceo_present_in(&rig.pack),
                "이 시험은 백업이 없는 조립 지침을 잰다"
            );
            let ctx = UpdateContext::new(&rig.pack, PackScope::Base, &src, false);
            let f = ctx.facts(
                MASTER_DIRECTIVE_REL,
                vendor_new,
                true,
                Some(composed.as_str()),
                Some(content_hash(&composed).as_str()),
                &none,
            );
            assert!(
                f.ceo_promoted,
                "{tag}: 조립 지침이 승격으로 잡히지 않았다(가드 4 가 .pre-ceo 만 본다)"
            );
            assert!(
                f.takeover_ok,
                "{tag}: 가드 3 이 따로 막으면 이 시험은 가드 4 를 재지 못한다"
            );
            rig.install(&[(MASTER_DIRECTIVE_REL, vendor_new)], false, &src);
            assert_eq!(
                rig.read(MASTER_DIRECTIVE_REL).as_deref(),
                Some(composed.as_str()),
                "{tag}: 자동 갱신이 조립된 CEO 지침을 vendor 본으로 덮었다(CEO 머리말·본문 수정 소실)"
            );
            drop(rig);
            // ②
            let rig = Rig::new(&format!("t8-real-{tag}"));
            rig.write(MASTER_DIRECTIVE_REL, composed.as_bytes());
            rig.manifest(&[(MASTER_DIRECTIVE_REL, body)]);
            rig.install(&[(MASTER_DIRECTIVE_REL, vendor_new)], false, &src);
            assert_eq!(
                rig.read(MASTER_DIRECTIVE_REL).as_deref(),
                Some(composed.as_str()),
                "{tag}: 실제 기록에서도 보존"
            );
            assert_eq!(
                rig.read(&format!("{MASTER_DIRECTIVE_REL}.new")).as_deref(),
                Some(vendor_new),
                "{tag}: 신본은 .new 로 곁에 둔다"
            );
            drop(rig);
        }
        // ③
        let rig = Rig::new("t8-plain");
        rig.write(MASTER_DIRECTIVE_REL, body.as_bytes());
        rig.manifest(&[(MASTER_DIRECTIVE_REL, body)]);
        rig.install(
            &[(MASTER_DIRECTIVE_REL, vendor_new)],
            false,
            &UpdateSources::none(),
        );
        assert_eq!(
            rig.read(MASTER_DIRECTIVE_REL).as_deref(),
            Some(vendor_new),
            "평범한 미수정 MASTER 는 갱신된다"
        );
    }

    /// ★T8(CCR-2 · RULINGS CB1·CG2): `ceo_promoted_in` 은 '승격'의 한 뜻을 여섯 번째 판독자로 낸다 — 다른 다섯
    /// (javis_preflight·javis_org·cys-dept·A12·앱)과 같은 픽스처 한 벌, 실제 항목(파일·끊긴 링크·폴더·읽을 수 없는 파일).
    #[test]
    fn ceo_promoted_in_matches_the_shared_fixture() {
        let fx: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/cysjavis-pack/bin/tests/fixtures/ceo-promoted-cases.json"
        )))
        .unwrap();
        assert_eq!(
            fx["separator"].as_str(),
            Some(CEO_SEPARATOR),
            "팩 코어의 구분선 사본 ≠ 픽스처"
        );
        let mut ran = 0;
        for (i, c) in fx["cases"].as_array().unwrap().iter().enumerate() {
            let td = std::env::temp_dir().join(format!("cys-pack-cb1-{}-{i}", std::process::id()));
            let _ = std::fs::remove_dir_all(&td);
            std::fs::create_dir_all(td.join("directives")).unwrap();
            if let Some(md) = c["md"].as_str() {
                std::fs::write(td.join(MASTER_DIRECTIVE_REL), md).unwrap();
            }
            let pre = td.join(format!("{MASTER_DIRECTIVE_REL}.pre-ceo"));
            match c["pre_ceo"].as_str() {
                None => {}
                Some("file") => std::fs::write(&pre, "백업\n").unwrap(),
                Some("directory") => std::fs::create_dir(&pre).unwrap(),
                #[cfg(unix)]
                Some("dangling-link") => {
                    std::os::unix::fs::symlink(td.join("nowhere"), &pre).unwrap()
                }
                #[cfg(unix)]
                Some("unreadable") => {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::write(&pre, "백업\n").unwrap();
                    std::fs::set_permissions(&pre, std::fs::Permissions::from_mode(0o000)).unwrap();
                }
                Some(_) => {
                    let _ = std::fs::remove_dir_all(&td);
                    continue; // 이 플랫폼에서 만들 수 없는 항목
                }
            }
            assert_eq!(
                ceo_promoted_in(&td),
                c["promoted"].as_bool().unwrap(),
                "ceo_promoted_in ≠ 픽스처: {}",
                c["name"]
            );
            assert_eq!(
                ceo_pre_ceo_present_in(&td),
                !c["pre_ceo"].is_null(),
                "사전적 실재 ≠ 픽스처: {}",
                c["name"]
            );
            ran += 1;
            let _ = std::fs::remove_dir_all(&td);
        }
        assert!(ran >= 12, "픽스처 행을 거의 못 돌렸다: {ran}");
    }

    /// 바: 한 번도 고치지 않은 사용자 소유 파일은 새 버전으로 — 가드 넷(그 파일만 대조 · 감사 유지 · 규칙 문서 인수
    /// 검사 · CEO 승격 중 MASTER)과 부서 재작성 파일 제외. 각 가드를 빼면 해당 단언이 적색이 된다.
    #[test]
    fn m11_never_edited_user_owned_auto_update_and_four_guards() {
        let rig = Rig::new("bau");
        let old_acl = "{\"a\":1}\n";
        let old_sched = "{\"jobs\":[]}\n";
        let old_dir = "# W\nautopilot denylist 헌법\n";
        let new_dir_drops = "# W\n(안전 조항 없음)\n"; // 인수 검사 실패
        let old_master = "# M\nautopilot denylist\n";
        let old_sched_kept = "{\"jobs\":[1]}\n";
        rig.write("acl.json", old_acl.as_bytes());
        rig.write("agents.json", b"{\"x\":0}\n"); // 과거 vendor 본(표 일치)
        rig.write("schedule.json", old_sched_kept.as_bytes());
        rig.write("directives/WORKER_DIRECTIVE.md", old_dir.as_bytes());
        rig.write("directives/MASTER_DIRECTIVE.md", old_master.as_bytes());
        rig.write("directives/MASTER_DIRECTIVE.md.pre-ceo", b"backup");
        rig.manifest(&[
            ("acl.json", old_acl),
            ("schedule.json", old_sched_kept),
            ("directives/WORKER_DIRECTIVE.md", old_dir),
            ("directives/MASTER_DIRECTIVE.md", old_master),
        ]);
        append_merge_audit(&rig.pack, &serde_json::json!({"file": "schedule.json", "action": "rollback", "flags": []})).unwrap();
        // 거부 기록은 사용자의 선택이 아니다(가드 2 가 세지 않는다).
        append_merge_audit(&rig.pack, &serde_json::json!({"file": "acl.json", "action": "keep-mine", "flags": ["refused"]})).unwrap();
        let src = UpdateSources::none().with_prev_vendor(|rel: &str| {
            if rel == "agents.json" { vec![content_hash("{\"x\":0}\n")] } else { Vec::new() }
        });
        let items = [
            ("acl.json", "{\"a\":2}\n"),
            ("agents.json", "{\"x\":9}\n"),
            ("schedule.json", old_sched),
            ("directives/WORKER_DIRECTIVE.md", new_dir_drops),
            ("directives/MASTER_DIRECTIVE.md", "# M\nautopilot denylist v2\n"),
        ];
        rig.install(&items, false, &src);
        assert_eq!(rig.read("acl.json").as_deref(), Some("{\"a\":2}\n"), "가드 1: 설치 기록 해시 일치 → 갱신");
        assert_eq!(rig.read("agents.json").as_deref(), Some("{\"x\":9}\n"), "가드 1: 그 파일의 과거 vendor 본 → 갱신");
        assert_eq!(rig.read("schedule.json").as_deref(), Some(old_sched_kept), "가드 2: '되돌림' 기록 → 건너뜀");
        assert_eq!(rig.read("directives/WORKER_DIRECTIVE.md").as_deref(), Some(old_dir), "가드 3: 안전 조항을 잃는 새 규칙 문서는 바꾸지 않는다");
        assert_eq!(rig.read("directives/WORKER_DIRECTIVE.md.new").as_deref(), Some(new_dir_drops), "가드 3 실패 → .new");
        assert_eq!(rig.read("directives/MASTER_DIRECTIVE.md").as_deref(), Some(old_master), "가드 4: CEO 승격 중 MASTER 건너뜀");
        // 부서: agents.json·acl.json·schedule.json 은 cys-dept 가 다시 쓴다 — 이 규칙에 걸리지 않는다.
        let f = UpdateFacts { prev_vendor_match: true, ..UpdateFacts::default() };
        assert_eq!(
            decide_file_action_with("acl.json", "{\"a\":2}\n", true, Some(old_acl), Some(content_hash(old_acl).as_str()), false, PackScope::Dept, None, &f),
            FileAction::Keep { adopt_hash: false, new_pending: true },
            "부서 acl.json 은 자동 갱신하지 않는다"
        );
        // 규칙 문서도 인수 검사를 통과하면 갱신된다(가드 3 이 무조건 거부가 아님).
        let ok = UpdateFacts { takeover_ok: true, ..UpdateFacts::default() };
        assert_eq!(
            decide_file_action_with("directives/WORKER_DIRECTIVE.md", "new", true, Some(old_dir), Some(content_hash(old_dir).as_str()), false, PackScope::Base, None, &ok),
            FileAction::Write { heal_user_copy: false }
        );
    }

    /// 라 + TESTPLAN Q-C4: 줄바꿈만 다른 사용자본(CRLF)은 맞춘 뒤 합치고 결과는 사용자의 CRLF 를 지킨다. 셸 스크립트는
    /// 합친 결과·보존 결과 모두 CR 이 없다(부록 A-2 #16 · A-3 #9).
    #[test]
    fn m11_line_endings_crlf_text_and_lf_shell() {
        let rig = Rig::new("eol");
        let base = "a\nb\nc\nd\ne\n";
        let vendor = "a\nb\nc\nd\nE-VENDOR\n";
        rig.write("skills/x/SKILL.md", b"A-USER\r\nb\r\nc\r\nd\r\ne\r\n");
        rig.write("skills/x/run.sh", b"A-USER\r\nb\r\nc\r\nd\r\ne\r\n");
        rig.write("hooks/clash.sh", b"#!/bin/bash\r\necho MINE\r\n");
        rig.manifest(&[("skills/x/SKILL.md", base), ("skills/x/run.sh", base), ("hooks/clash.sh", "#!/bin/bash\necho old\n")]);
        rig.pristine("skills/x/SKILL.md", base);
        rig.pristine("skills/x/run.sh", base);
        rig.pristine("hooks/clash.sh", "#!/bin/bash\necho old\n");
        rig.install(&[("skills/x/SKILL.md", vendor), ("skills/x/run.sh", vendor), ("hooks/clash.sh", "#!/bin/bash\necho vendor\n")], false, &UpdateSources::none());
        assert_eq!(rig.read("skills/x/SKILL.md").as_deref(), Some("A-USER\r\nb\r\nc\r\nd\r\nE-VENDOR\r\n"), "텍스트: 두 수정 모두 + 사용자 CRLF 유지");
        assert_eq!(rig.read("skills/x/run.sh").as_deref(), Some("A-USER\nb\nc\nd\nE-VENDOR\n"), "셸: 합친 결과는 LF");
        assert_eq!(rig.kind("hooks/clash.sh").as_deref(), Some("user-kept"), "같은 줄 충돌 → 사용자 우선");
        assert_eq!(rig.read("hooks/clash.sh").as_deref(), Some("#!/bin/bash\necho MINE\n"), "셸 보존 결과에도 CR 없음");
        assert!(!std::fs::read(rig.pack.join("hooks/clash.sh")).unwrap().contains(&b'\r'));
    }

    /// ★T2i(RULINGS FT1): 보존된 파일의 CR 제거는 진짜 셸 스크립트에만 — `hooks/` 아래의 `.md` · `.py` · `.cmd` 는 사용자의 바이트
    /// 그대로(CRLF 유지)이고 원장에 "eol" 이 없다. `.sh` 와 shebang 파일(확장자 없음)은 종전대로 LF + "eol": "lf".
    #[test]
    fn ft1_only_a_real_shell_script_loses_its_cr_when_kept() {
        let rig = Rig::new("ft1");
        let cases: [(&str, &str, &[u8], bool); 5] = [
            ("hooks/zz_note.md", "# note\nvendor\n", b"# note\r\nmine\r\n", false),
            ("hooks/zz_gate.py", "#!/usr/bin/env python3\nprint('v')\n", b"#!/usr/bin/env python3\r\nprint('mine')\r\n", false),
            ("hooks/zz_tool.cmd", "@echo off\r\necho vendor\r\n", b"@echo off\r\necho mine\r\n", false),
            ("hooks/zz_run.sh", "#!/bin/bash\necho vendor\n", b"#!/bin/bash\r\necho mine\r\n", true),
            ("bin/zz_tool", "#!/bin/bash\necho vendor\n", b"#!/bin/bash\r\necho mine\r\n", true),
        ];
        for (rel, _v, mine, _s) in cases.iter() {
            rig.write(rel, mine);
        }
        let items: Vec<(&str, &str)> = cases.iter().map(|(r, v, _, _)| (*r, *v)).collect();
        rig.install(&items, false, &UpdateSources::none());
        for (rel, _v, mine, shell) in cases.iter() {
            let live = std::fs::read(rig.pack.join(rel)).unwrap();
            let e = rig.entry(rel);
            if *shell {
                let lf: Vec<u8> = mine.iter().copied().filter(|b| *b != b'\r').collect();
                assert_eq!(live, lf, "{rel}: 셸 스크립트 = LF");
                assert_eq!(e.as_ref().map(|e| e["eol"].clone()), Some(serde_json::json!("lf")), "{rel}: {e:?}");
            } else {
                assert_eq!(&live, mine, "{rel}: 사용자의 바이트 그대로(CR 포함)");
                assert!(e.as_ref().is_none_or(|e| e.get("eol").is_none()), "{rel}: eol 없음 {e:?}");
            }
        }
        assert!(!is_shell_script("hooks/README.md", "# x\n"));
        assert!(!is_shell_script("hooks/cys-agy-statusline.cmd", "@echo off\r\n"));
        assert!(!is_shell_script("hooks/inject_gate.py", "#!/usr/bin/env python3\n"));
        assert!(is_shell_script("hooks/session-start.sh", ""));
        assert!(is_shell_script("bin/cys-dept", "#!/usr/bin/env bash\n"));
    }

    /// 부록 A-2 #6(손상 의심 검사): 의심 병합은 받아들이지 않고 사용자 파일을 살린다 — 표시는 덮는 이유가 아니라
    /// 합칠 때의 주의(caution).
    #[test]
    fn m11_suspect_merge_keeps_user_file_with_caution() {
        let rig = Rig::new("suspect");
        let base: String = (0..20).map(|i| format!("line {i}\n")).collect();
        // 사용자본 = 앞 10줄만(뒤 10줄 삭제 — 삭제 위주 50%), vendor = 첫 줄만 변경 → 깨끗하게 합쳐지지만 의심.
        let mine: String = (0..10).map(|i| format!("line {i}\n")).collect();
        let mine = mine.as_str();
        let vendor = base.replacen("line 0\n", "line 0 vendor\n", 1);
        rig.write("bin/s.py", mine.as_bytes());
        rig.manifest(&[("bin/s.py", base.as_str())]);
        rig.pristine("bin/s.py", &base);
        rig.install(&[("bin/s.py", vendor.as_str())], false, &UpdateSources::none());
        assert_eq!(rig.read("bin/s.py").as_deref(), Some(mine), "의심 병합 → 사용자 파일 유지");
        let e = rig.entry("bin/s.py").unwrap();
        assert_eq!(e["kind"], "user-kept");
        assert!(e["caution"].as_str().unwrap_or("").starts_with("suspect:"), "caution 표시: {e}");
    }

    /// 나(기동 필수 + vendor 전진) — vendor 본 + `.user`(이전 세대 보존) + 원장 vendor-first.
    /// ★RULINGS BO1 재작성(종전: 병합을 먼저 시도해 충돌 → 폴백 기계 — `.base` 조상 사이드카 + reason `merge-conflict`):
    /// 설치기는 기동 필수 파일을 병합하지 않으므로 `.base` 도 없고 reason 은 `vendor-advanced` 다.
    #[test]
    fn m11_boot_critical_merge_conflict_vendor_first_keeps_fallback_mechanics() {
        let rig = Rig::new("bcmerge");
        let base = "#!/bin/bash\nx\n";
        let mine = "#!/bin/bash\nMINE\n";
        let vendor = "#!/bin/bash\nVENDOR\n";
        rig.write("hooks/h.sh", mine.as_bytes());
        rig.manifest(&[("hooks/h.sh", base)]);
        rig.pristine("hooks/h.sh", base);
        rig.install(&[("hooks/h.sh", vendor)], false, &UpdateSources::none().with_boot_critical(bc_only(&["hooks/h.sh"])));
        assert_eq!(rig.read("hooks/h.sh").as_deref(), Some(vendor), "vendor 본 먼저");
        assert_eq!(rig.read("hooks/h.sh.user").as_deref(), Some(mine));
        assert!(!rig.pack.join("hooks/h.sh.base").exists(), "BO1: 병합 시도 없음 = .base 없음");
        let e = rig.entry("hooks/h.sh").unwrap();
        assert_eq!(e["kind"], "vendor-first");
        assert_eq!(e["reason"], "vendor-advanced");
        assert!(e.get("parked_at").is_some());
    }

    /// RULINGS AC6 (T5a, approved test-only edit) — the pin on the REAL compiled list: the
    /// boot-critical list is not empty and holds every lifeline hook file the contract lists (the
    /// statusline script `cys-statusline.sh` is excluded from the list by O-C4-4 — a clear-procedure
    /// file — and stays off it), and under the production seam (`UpdateSources::none()` =
    /// `boot_critical_rel`, no stub) a conflicted lifeline hook file is `vendor-first`.
    #[test]
    fn ac6_real_list_makes_a_conflicted_lifeline_hook_vendor_first() {
        let list = crate::repair::boot_critical::entries();
        assert!(!list.is_empty(), "the compiled boot-critical list is empty");
        for name in crate::repair::boot_critical::LIFELINE_HOOKS {
            let rel = format!("hooks/{name}");
            if name == "cys-statusline.sh" {
                assert!(!boot_critical_rel(&rel), "{rel}: excluded by O-C4-4");
                continue;
            }
            let e = list.iter().find(|e| e.rel == rel);
            assert!(e.is_some_and(|e| e.lifeline), "{rel} is not a lifeline row of the compiled list");
            assert!(boot_critical_rel(&rel));
        }
        let rel = "hooks/inject-context.sh";
        let rig = Rig::new("ac6real");
        let base = "#!/bin/bash\nx\n";
        let mine = "#!/bin/bash\nMINE\n";
        let vendor = "#!/bin/bash\nVENDOR\n";
        rig.write(rel, mine.as_bytes());
        rig.manifest(&[(rel, base)]);
        rig.pristine(rel, base);
        rig.install(&[(rel, vendor)], false, &UpdateSources::none());
        assert_eq!(rig.read(rel).as_deref(), Some(vendor), "vendor first");
        assert_eq!(rig.read("hooks/inject-context.sh.user").as_deref(), Some(mine));
        assert_eq!(rig.entry(rel).unwrap()["kind"], "vendor-first");
    }

    /// 아 + RULINGS AC1·AC4: 업데이트 전 스냅샷 — 버전이 바뀔 때만, **이번 설치가 덮거나 지울 파일만**(변하지 않는 파일·
    /// 사용자 파일 유지·seed-once 제외), 각 항목에 출시본 여부를 기록하고, 복원은 전부 검증한 뒤에만 쓴다(하나라도
    /// 어긋나면 아무것도 쓰지 않는다). system 파일은 출시본일 때만 되돌리고, 전체 복원은 user 소유를 되돌리지 않는다.
    /// ★r2 재작성(종전: 매니페스트 항목 전부를 담는다고 단언 — RULINGS AC4 가 그 범위를 '덮거나 지울 파일'로 좁혔다).
    #[test]
    fn m11_update_snapshot_create_and_restore_contract() {
        let rig = Rig::new("snap");
        let root = rig.base.join("snapshots");
        let a1 = "#!/bin/bash\necho A1\n";
        rig.write("bin/a.sh", a1.as_bytes()); // system · 비수정 · vendor 전진 → 덮는다(출시본)
        rig.write("agents.json", b"{\"a\":1}\n"); // user · 비수정 · vendor 전진 → 자동 갱신으로 덮는다(출시본)
        rig.write("bin/b.py", b"B\n"); // system · 변화 없음 → 담지 않는다
        rig.write("bin/c.py", b"MINE\n"); // system · 사용자 수정 · 조상 없음 → 사용자 파일 유지(덮지 않음) → 담지 않는다
        rig.write("memory/MEMORY.md", b"STATE\n"); // seed-once → 담지 않는다
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(rig.pack.join("bin/a.sh"), std::fs::Permissions::from_mode(0o750)).unwrap();
            // 새로 만들 자리에 놓인 (끊긴) 링크 — 쓰기가 링크를 파일로 바꾸므로 담는다(출시본 아님).
            std::os::unix::fs::symlink("/nonexistent/target", rig.pack.join("bin/link")).unwrap();
        }
        let manifest_v1 = [("bin/a.sh", a1), ("agents.json", "{\"a\":1}\n"), ("bin/b.py", "B\n"),
                           ("bin/c.py", "C1\n"), ("memory/MEMORY.md", "STATE\n")];
        rig.manifest(&manifest_v1);
        let items = [("bin/a.sh", "#!/bin/bash\necho A2\n"), ("agents.json", "{\"a\":2}\n"), ("bin/b.py", "B\n"),
                     ("bin/c.py", "C2\n"), ("memory/MEMORY.md", "SEED\n"), ("bin/link", "#!/bin/sh\nnew\n")];
        let src = UpdateSources::none().with_snapshot("base", root.clone());
        std::fs::write(rig.pack.join(PACK_VERSION_FILE), "9.9.9").unwrap();
        rig.install(&items, false, &src);
        assert!(newest_update_snapshot(&root, "base").is_none(), "같은 버전이면 스냅샷을 만들지 않는다");
        // 업데이트 상황으로 되돌린다(버전 1.0.0 · 같은 디스크 바이트).
        std::fs::write(rig.pack.join(PACK_VERSION_FILE), "1.0.0").unwrap();
        rig.write("bin/a.sh", a1.as_bytes());
        rig.write("agents.json", b"{\"a\":1}\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(rig.pack.join("bin/a.sh"), std::fs::Permissions::from_mode(0o750)).unwrap();
            let _ = std::fs::remove_file(rig.pack.join("bin/link"));
            std::os::unix::fs::symlink("/nonexistent/target", rig.pack.join("bin/link")).unwrap();
        }
        rig.manifest(&manifest_v1);
        let _ = std::fs::remove_file(rig.pack.join(MERGE_PENDING_FILE));
        rig.install(&items, false, &src);
        let snap = newest_update_snapshot(&root, "base").expect("버전이 바뀌면 스냅샷");
        let meta = read_update_snapshot_meta(&snap).unwrap();
        assert_eq!(meta.pack_version, "1.0.0");
        assert_eq!(meta.lane_key, "base");
        let mut rels: Vec<&str> = meta.files.iter().map(|f| f.rel.as_str()).collect();
        rels.sort();
        #[cfg(unix)]
        assert_eq!(rels, vec!["agents.json", "bin/a.sh", "bin/link"], "덮거나 지울 파일만 — 변화 없음·사용자 파일 유지·seed-once 는 없다");
        #[cfg(not(unix))]
        assert_eq!(rels, vec!["agents.json", "bin/a.sh"]);
        let released = |r: &str| meta.files.iter().find(|f| f.rel == r).map(|f| f.released);
        assert_eq!(released("bin/a.sh"), Some(true), "설치 기록과 같은 바이트 = 출시본");
        assert_eq!(released("agents.json"), Some(true));
        #[cfg(unix)]
        assert_eq!(released("bin/link"), Some(false), "링크는 출시본이 아니다");
        assert!(meta.skipped.is_empty());
        assert_eq!(rig.read("bin/a.sh").as_deref(), Some("#!/bin/bash\necho A2\n"), "설치는 계속됐다");
        assert_eq!(rig.read("bin/c.py").as_deref(), Some("MINE\n"), "사용자 파일 유지");
        // 기억이 업데이트 뒤에 바뀌어도 복원은 상태를 되돌리지 않는다.
        rig.write("memory/MEMORY.md", b"NEW STATE AFTER UPDATE\n");
        // 보관본 하나가 깨지면 아무것도 쓰지 않는다.
        std::fs::write(snap.join(UPDATE_SNAPSHOT_FILES).join("agents.json"), b"TAMPERED").unwrap();
        rig.write("agents.json", b"{\"a\":\"after\"}\n");
        assert!(restore_update_snapshot(&snap, &rig.pack, Some(&["bin/a.sh".to_string(), "agents.json".to_string()]), &UpdateSources::none()).is_err());
        assert_eq!(rig.read("bin/a.sh").as_deref(), Some("#!/bin/bash\necho A2\n"), "검증 실패 → bin/a.sh 도 쓰지 않았다");
        std::fs::write(snap.join(UPDATE_SNAPSHOT_FILES).join("agents.json"), b"{\"a\":1}\n").unwrap();
        // 파일 단위 복원: 그 파일만.
        let done = restore_update_snapshot(&snap, &rig.pack, Some(&["bin/a.sh".to_string()]), &UpdateSources::none()).unwrap();
        assert_eq!(done, vec!["bin/a.sh".to_string()]);
        assert_eq!(rig.read("bin/a.sh").as_deref(), Some(a1));
        assert_eq!(rig.read("agents.json").as_deref(), Some("{\"a\":\"after\"}\n"), "다른 파일은 그대로");
        #[cfg(unix)]
        assert_eq!(current_mode(&rig.pack.join("bin/a.sh")), Some(0o750), "기록된 권한");
        // AC1: 출시본이 아닌 system 항목(링크)은 파일 단위로 되돌리지 않는다.
        #[cfg(unix)]
        assert!(restore_update_snapshot(&snap, &rig.pack, Some(&["bin/link".to_string()]), &UpdateSources::none()).is_err(), "출시본 아님 → 거부");
        // 전체 복원: 출시본 system 파일 + 버전 표시. 사용자 소유·상태·출시본 아닌 항목은 그대로.
        rig.write("bin/a.sh", b"#!/bin/bash\necho A2\n");
        let done = restore_update_snapshot(&snap, &rig.pack, None, &UpdateSources::none()).unwrap();
        assert_eq!(done, vec!["bin/a.sh".to_string()], "전체 복원 = 출시본 system 항목만");
        assert_eq!(rig.read("bin/a.sh").as_deref(), Some(a1));
        assert_eq!(std::fs::read_to_string(rig.pack.join(PACK_VERSION_FILE)).unwrap(), "1.0.0", "버전 표시 = 파일과 일치");
        assert_eq!(rig.read("memory/MEMORY.md").as_deref(), Some("NEW STATE AFTER UPDATE\n"), "상태 불퇴행");
        assert_eq!(rig.read("agents.json").as_deref(), Some("{\"a\":\"after\"}\n"), "전체 복원은 user 소유를 되돌리지 않는다");
        // 3세대 보존 — 잠 없이 연달아 만든다(같은 초 안이어도 생성 순서가 목록 순서다).
        for v in ["2.0.0", "3.0.0", "4.0.0"] {
            std::fs::write(rig.pack.join(PACK_VERSION_FILE), v).unwrap();
            rig.write("bin/a.sh", a1.as_bytes());
            rig.manifest(&manifest_v1);
            rig.install(&items, false, &src);
        }
        let kept = list_update_snapshots(&root.join("base"));
        assert_eq!(kept.len(), UPDATE_SNAPSHOT_KEEP, "레인당 최신 3세대");
        let kept_versions: Vec<String> = kept
            .iter()
            .map(|(_, p)| read_update_snapshot_meta(p).expect("메타").pack_version)
            .collect();
        assert_eq!(kept_versions, vec!["2.0.0", "3.0.0", "4.0.0"], "가장 오래된 1.0.0 세대가 정리되고 생성 순서가 유지된다");
        let newest = newest_update_snapshot(&root, "base").expect("최신");
        assert_eq!(read_update_snapshot_meta(&newest).unwrap().pack_version, "4.0.0", "최신 = 마지막에 만든 스냅샷");
    }

    /// ★RULINGS BO1: 고친 기동 필수 파일을 vendor 가 전진시키면 — 검증 base 가 있고 병합이 깨끗하게 될 경우라도 — 설치기는
    /// 병합하지 않는다: vendor 본이 살아 있고, 고친 사본은 시각과 함께 보관되고, vendor-first 기록이 남고, 병합본은 어디에도
    /// 살아 있지 않다. 같은 경우의 일반 파일은 종전대로 설치 때 병합된다. plan 버킷도 같은 답을 한다.
    #[test]
    fn m11_bo1_boot_critical_edit_is_never_merged_at_install() {
        let rig = Rig::new("bo1");
        let base = "#!/bin/bash\nline-1\nline-2\nline-3\n";
        let mine = "#!/bin/bash\nline-1 MINE\nline-2\nline-3\n";
        let vendor = "#!/bin/bash\nline-1\nline-2\nline-3 VENDOR\n";
        let merged = "#!/bin/bash\nline-1 MINE\nline-2\nline-3 VENDOR\n";
        for rel in ["hooks/boot.sh", "bin/general.sh"] {
            rig.write(rel, mine.as_bytes());
            rig.pristine(rel, base);
        }
        rig.manifest(&[("hooks/boot.sh", base), ("bin/general.sh", base)]);
        let src = || UpdateSources::none().with_boot_critical(|r| r == "hooks/boot.sh");
        let items = [("hooks/boot.sh", vendor), ("bin/general.sh", vendor)];
        let plan = rig.plan(&items, false, &src());
        assert_eq!(plan.vendor_first, vec!["hooks/boot.sh".to_string()], "plan: 기동 필수 = vendor 우선");
        assert_eq!(plan.merge3, vec!["bin/general.sh".to_string()], "plan: 일반 파일 = 설치 때 병합");
        rig.install(&items, false, &src());
        // 기동 필수: vendor 본 · 사용자본 보관 · vendor-first · 병합본 없음.
        assert_eq!(rig.read("hooks/boot.sh").as_deref(), Some(vendor), "기동 필수 = vendor 본(병합본 아님)");
        assert_eq!(rig.read("hooks/boot.sh.user").as_deref(), Some(mine), "고친 사본 보관");
        let e = rig.entry("hooks/boot.sh").unwrap();
        assert_eq!(e["kind"], "vendor-first");
        assert_eq!(e["reason"], "vendor-advanced");
        assert!(e.get("parked_at").is_some(), "보관 시각");
        assert!(!rig.pack.join("hooks/boot.sh.base").exists());
        for f in std::fs::read_dir(rig.pack.join("hooks")).unwrap().flatten() {
            assert_ne!(std::fs::read_to_string(f.path()).ok().as_deref(), Some(merged),
                       "병합본이 살아 있는 자리에 없다: {}", f.path().display());
        }
        // 일반 파일: 종전대로 병합.
        assert_eq!(rig.read("bin/general.sh").as_deref(), Some(merged), "일반 파일 = 설치 때 병합");
        assert_eq!(rig.kind("bin/general.sh").as_deref(), Some("merged"));
    }

    /// ★T2f(RULINGS BD4): 팩 쓰기 동사의 단일 술어 — 기동 필수 목록(`boot_critical::entries()`)과 보호 목록
    /// (`boot_critical::protection_entries()`)의 합집합이고(T5a 재기반 2단계가 넓혔다 — 첫 단언이 그 합집합), 제품 입력원
    /// (`none()` · `for_live_pack()`)은 이 함수를 그대로 쓴다(이음새로 바뀐 술어가 제품 경로에 남으면 빨갛다).
    #[test]
    fn t2f_one_predicate_for_every_pack_writer() {
        let rig = Rig::new("t2fpred");
        let _l = EnvGuard::set("CYS_LOCAL_DIR", rig.base.join("local"));
        let _p = EnvGuard::set(ENV_PACK_DIR, &rig.pack);
        let none = UpdateSources::none();
        let live = UpdateSources::for_live_pack(&rig.pack);
        let mut probes: Vec<&str> = PACK_ALL.iter().map(|(r, _)| *r).collect();
        probes.extend(crate::repair::boot_critical::entries().iter().map(|e| e.rel));
        probes.extend(crate::repair::boot_critical::protection_entries().iter().map(|e| e.rel));
        let protection_rel = |rel: &str| {
            crate::repair::boot_critical::protection_entries()
                .iter()
                .any(|e| list_key(e.rel) == list_key(rel))
        };
        for rel in probes {
            assert_eq!(listed_in_compiled_lists(rel), boot_critical_rel(rel) || protection_rel(rel), "{rel}");
            assert_eq!(none.is_listed(rel), listed_in_compiled_lists(rel), "none(): {rel}");
            assert_eq!(live.is_listed(rel), listed_in_compiled_lists(rel), "for_live_pack(): {rel}");
        }
        // 이음새는 기동 필수 판정과 따로다(보호 목록에만 있는 파일 = 등재 ∧ 업데이트에서는 기동 필수 아님 — BC2).
        let seam = UpdateSources::none().with_listed(|r| r == "hooks/guard.sh");
        assert!(seam.is_listed("hooks/guard.sh") && !seam.is_boot_critical("hooks/guard.sh"));
        // ★RULINGS BO2 · BO4: 술어에는 목록 대조 키로 접은 철자가 간다 — 같은 파일을 여는 철자(대소문자 · 앞의 `./` ·
        //   역슬래시 · KELVIN SIGN)는 모두 등재로 답한다. 등재되지 않은 파일은 그대로 아니다.
        for alias in ["hooks/guard.sh", "Hooks/Guard.SH", "./hooks/guard.sh", "hooks\\guard.sh", "hoo\u{212A}s/guard.sh"] {
            assert!(seam.is_listed(alias), "철자 {alias:?} 도 등재");
        }
        assert!(!seam.is_listed("hooks/other.sh"));
        assert_eq!(list_key("./Hooks\\X.SH"), "hooks/x.sh");
        assert_eq!(list_key("stra\u{DF}e"), "strasse", "전체 접기(ß → ss)");
    }

    /// ★T2f(RULINGS BV2): 등재 파일의 '출시본으로 모르는 바이트' 거부 문장 — 변조와 더 새 서명 팩을 함께 말하고, 두 길(수리
    /// 흐름 · 다음 프로그램 업데이트)만 알리며 `cys pack-heal` 은 알리지 않는다(이 바이너리의 더 오래된 사본을 쓰므로).
    #[test]
    fn t2f_unknown_release_sentence_names_the_two_routes_only() {
        let t = unknown_release_refusal("hooks/guard.sh", "보관된 hooks/guard.sh.new");
        assert!(t.contains("hooks/guard.sh") && t.contains("보관된 hooks/guard.sh.new"));
        assert!(t.contains("출시본으로 알지 못합니다"));
        assert!(t.contains("변조된 바이트") && t.contains("새 서명 팩"), "두 경우를 함께 말한다");
        assert!(t.contains("cys repair begin --files hooks/guard.sh"), "길 1: 수리 흐름");
        assert!(t.contains("다음 프로그램") && t.contains("업데이트"), "길 2: 다음 프로그램 업데이트");
        assert!(!t.contains("pack-heal"), "pack-heal 을 알리지 않는다");
    }

    /// ★T2f: 출시본 바이트 = 내장 vendor 본 · 이 설치의 매니페스트 · 과거 출시 해시 표 중 하나와 같은 UTF-8 바이트.
    #[test]
    fn t2f_released_bytes_are_embedded_manifest_or_table_bytes() {
        let rig = Rig::new("t2frel");
        let (rel, embed) = PACK_ALL.iter().find(|(r, _)| r.starts_with("bin/")).map(|(r, c)| (*r, *c)).unwrap();
        let none = UpdateSources::none();
        assert!(released_bytes(&rig.pack, rel, embed.as_bytes(), &none), "내장 vendor 본");
        assert!(!released_bytes(&rig.pack, rel, b"MY EDIT\n", &none), "사용자 수정본");
        assert!(!released_bytes(&rig.pack, rel, &[0xffu8, 0xfe], &none), "비 UTF-8");
        // ★RULINGS BO3 재작성(종전: "이 설치의 매니페스트" 의 해시도 출시본으로 셌다): 매니페스트는 일반 쓰기가 바꿀 수
        //   있는 파일이라 근거가 아니다 — 내장본·과거 출시 표에 없는 바이트는 매니페스트에 적혀 있어도 출시본이 아니다.
        rig.manifest(&[(rel, "LAST APPLIED VENDOR\n")]);
        assert!(!released_bytes(&rig.pack, rel, b"LAST APPLIED VENDOR\n", &none), "매니페스트만으로는 출시본이 아니다");
        let h = content_hash("OLD RELEASE\n");
        let table = UpdateSources::none().with_prev_vendor(move |_| vec![h.clone()]);
        assert!(released_bytes(&rig.pack, rel, b"OLD RELEASE\n", &table), "과거 출시 해시 표");
        assert!(!released_bytes(&rig.pack, rel, b"OLD RELEASE\n", &none));
    }

    /// RULINGS AJ3(복원 계약 O-C8-4 "링크는 링크로"): user 소유 파일 자리에 놓인 링크는 스냅샷에 링크로 기록되고, 파일 단위
    /// 복원은 그것을 **링크로** 되살린다(파일 사본이 아니다). user 파일이라 출시본 여부와 무관하게 되돌린다.
    #[cfg(unix)]
    #[test]
    fn m11_snapshot_restores_a_user_owned_link_as_a_link() {
        let rig = Rig::new("snaplink");
        let root = rig.base.join("snapshots");
        let rel = "agents.json";
        assert_eq!(ownership_scoped(rel, PackScope::Base), Ownership::User);
        rig.write("my-agents.json", b"{\"mine\":1}\n");
        std::os::unix::fs::symlink(rig.pack.join("my-agents.json"), rig.pack.join(rel)).unwrap();
        let none = |_: &str| Vec::new();
        let o = create_update_snapshot(&rig.pack, &root, "base", "1.0.0", &std::collections::BTreeMap::new(),
                                       &[rel.to_string()], PackScope::Base, &none,
                                       (UPDATE_SNAPSHOT_FILE_CAP, UPDATE_SNAPSHOT_TOTAL_CAP)).unwrap();
        let meta = read_update_snapshot_meta(&o.path).unwrap();
        assert_eq!(meta.files[0].link_target.as_deref(), Some(rig.pack.join("my-agents.json").to_str().unwrap()));
        assert!(!meta.files[0].released);
        // 업데이트가 링크를 파일로 바꿨다고 치고 되돌린다.
        std::fs::remove_file(rig.pack.join(rel)).unwrap();
        rig.write(rel, b"{\"vendor\":2}\n");
        // ★T5a 재기반 2단계: 이 시험은 '등재되지 않은 user 파일'의 링크 복원을 잰다 — T5a 의 실제 목록에서 `agents.json` 은
        //   기동 필수 행이라(등재 파일의 링크는 출시본이 아니어서 거부된다 — T2f BO2(c)) 이음새로 '등재 아님'을 명시한다.
        let unlisted = UpdateSources::none().with_listed(|_| false);
        let done = restore_update_snapshot(&o.path, &rig.pack, Some(&[rel.to_string()]), &unlisted).unwrap();
        assert_eq!(done, vec![rel.to_string()]);
        let m = std::fs::symlink_metadata(rig.pack.join(rel)).unwrap();
        assert!(m.file_type().is_symlink(), "링크로 되살아나야 한다(파일 사본 아님)");
        assert_eq!(std::fs::read_link(rig.pack.join(rel)).unwrap(), rig.pack.join("my-agents.json"));
        assert_eq!(rig.read(rel).as_deref(), Some("{\"mine\":1}\n"), "링크 대상 그대로");
    }

    /// RULINGS AJ3: 파이썬 점검의 다섯 생명줄 이름(`javis_preflight.EXPECTED_LIFELINES`)은 Rust 상수 `LIFELINE_HOOKS` 에서
    /// 상태줄(목록 밖 — O-C4-4)을 뺀 다섯과 **같다**. 한쪽만 바뀌면 빨갛다.
    #[test]
    fn m11_python_expected_lifelines_match_the_rust_constant() {
        let py = include_str!("../cysjavis-pack/bin/javis_preflight.py");
        let start = py.find("EXPECTED_LIFELINES = (").expect("EXPECTED_LIFELINES 정의");
        let body = &py[start..];
        let body = &body[..body.find(')').expect("튜플 끝")];
        let mut py_names: Vec<String> = body
            .split('"')
            .enumerate()
            .filter(|(i, _)| i % 2 == 1)
            .map(|(_, s)| s.to_string())
            .collect();
        py_names.sort();
        let mut rs_names: Vec<String> = crate::repair::boot_critical::LIFELINE_HOOKS
            .iter()
            .filter(|n| **n != "cys-statusline.sh")
            .map(|n| n.to_string())
            .collect();
        rs_names.sort();
        assert_eq!(py_names.len(), 5, "파이썬 쪽 이름 수: {py_names:?}");
        assert_eq!(py_names, rs_names, "생명줄 이름이 두 언어에서 갈렸다");
    }

    /// RULINGS AC3: 전환마다 스냅샷 하나 — 실패를 거듭하는 기동 설치(버전 표시가 전진하지 못함)는 다시 복사하지 않고,
    /// 정리는 지금 진행 중인 전환의 가장 오래된 스냅샷을 지우지 않는다(진짜 업데이트 전 보존본이 밀려나지 않는다).
    #[test]
    fn m11_snapshot_once_per_transition_and_its_oldest_survives_pruning() {
        let rig = Rig::new("snaponce");
        let root = rig.base.join("snapshots");
        let a1 = "#!/bin/bash\necho A1\n";
        let items = [("bin/a.sh", "#!/bin/bash\necho A2\n")];
        let src = UpdateSources::none().with_snapshot("base", root.clone());
        let attempt = |ver: &str, disk: &str| {
            std::fs::write(rig.pack.join(PACK_VERSION_FILE), ver).unwrap();
            rig.write("bin/a.sh", disk.as_bytes());
            rig.manifest(&[("bin/a.sh", disk)]);
            rig.install(&items, false, &src);
        };
        attempt("1.0.0", a1);
        assert_eq!(list_update_snapshots(&root.join("base")).len(), 1);
        let first = newest_update_snapshot(&root, "base").unwrap();
        // 같은 전환을 다시 시도(버전 표시가 1.0.0 그대로 · 팩은 이미 일부가 바뀐 상태) — 새 스냅샷 없음.
        for _ in 0..4 {
            attempt("1.0.0", "#!/bin/bash\necho HALF-WRITTEN\n");
        }
        let list = list_update_snapshots(&root.join("base"));
        assert_eq!(list.len(), 1, "같은 전환은 스냅샷 하나 — 매 기동마다 다시 복사하지 않는다");
        assert_eq!(list[0].1, first, "첫 스냅샷이 그대로다");
        assert_eq!(
            std::fs::read_to_string(first.join(UPDATE_SNAPSHOT_FILES).join("bin/a.sh")).unwrap(),
            a1,
            "보존본 = 첫 시도 직전의 바이트"
        );
        // 다른 전환 둘(2.0.0 · 3.0.0) 뒤 1.0.0 에서 다시 출발 — 정리는 1.0.0 의 가장 오래된 것(first)을 지키고 2.0.0 을 지운다.
        attempt("2.0.0", a1);
        attempt("3.0.0", a1);
        assert_eq!(list_update_snapshots(&root.join("base")).len(), 3);
        attempt("1.0.0", a1);
        let versions: Vec<String> = list_update_snapshots(&root.join("base"))
            .iter()
            .map(|(_, p)| read_update_snapshot_meta(p).unwrap().pack_version)
            .collect();
        assert_eq!(versions, vec!["1.0.0", "3.0.0", "1.0.0"], "진행 중 전환의 가장 오래된 스냅샷은 정리되지 않는다");
        assert!(first.is_dir(), "첫 스냅샷 생존");
    }

    /// RULINGS AC4(부록 A-2 #17): 스냅샷은 유한하다 — 이 팩의 실제 파일 수(내장 전부가 바뀌는 최악의 업데이트)에서
    /// 크기는 상한 안이고, 일의 양은 '이번 설치가 덮거나 지울 파일 수'를 넘지 않는다. 상한을 넘는 파일은 담지 않고
    /// 색인에 이름을 남기며 설치는 계속된다. 걸린 시간은 로그에 남기기만 한다 — 디버그 빌드·부하 걸린 실행기에서 시계
    /// 한계는 흔들린다(실측: 다른 트랙과 함께 돈 하네스에서 4.13초 — DEFERRED_MINOR #45 의 방향 "일의 양을 단언하고 시간은
    /// 기록만"; 기동 전 시간은 5단계에서 잰다 — RULINGS AB6(3)).
    /// ★r3 재작성(종전: `took < 4초` 단언 — 그 단언이 부하로 깨져 같은 잠금을 쓰는 시험 55개를 연쇄로 떨어뜨렸다).
    #[test]
    fn m11_snapshot_is_bounded_on_the_real_pack_file_count() {
        let rig = Rig::new("snapbound");
        let root = rig.base.join("snapshots");
        let v1: Vec<(&str, &str)> = PACK_ALL.iter().map(|(r, c)| (*r, *c)).collect();
        assert!(v1.len() >= 400, "실제 팩 파일 수 픽스처({}개)", v1.len());
        install_into_with(rig.pack.clone(), v1.iter().copied(), false, "1.0.0", false, false,
                          PackScope::Base, None, None, &UpdateSources::none()).unwrap();
        std::fs::write(rig.pack.join(PACK_VERSION_FILE), "1.0.0").unwrap();
        let (manifest, _) = load_manifest_state(&rig.pack);
        // 최악: 내장 파일 전부가 바뀐다.
        let v2_owned: Vec<(String, String)> =
            v1.iter().map(|(r, c)| (r.to_string(), format!("{c}\n# changed in 9.9.9\n"))).collect();
        let v2: Vec<(&str, &str)> = v2_owned.iter().map(|(r, c)| (r.as_str(), c.as_str())).collect();
        let src = UpdateSources::none().with_snapshot("base", root.clone());
        let t0 = std::time::Instant::now();
        let out = take_update_snapshot(&rig.pack, &v2, false, "9.9.9", PackScope::Base, &src, &manifest)
            .expect("스냅샷 대상 있음")
            .expect("스냅샷 성공");
        let took = t0.elapsed();
        // 실측 증거(로그): 파일 수 · 바이트 · 걸린 시간.
        eprintln!("[m11-snapshot-bound] files={} bytes={} skipped={} took={took:?}", out.files, out.bytes, out.skipped.len());
        assert!(out.files > 0 && out.files <= v1.len());
        assert!(out.bytes <= UPDATE_SNAPSHOT_TOTAL_CAP, "총량 {} > 상한", out.bytes);
        assert!(out.skipped.is_empty(), "실제 팩은 상한 안에 든다: {:?}", out.skipped);
        let mut stack = vec![out.path.join(UPDATE_SNAPSHOT_FILES)];
        let mut on_disk = 0u64;
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let m = std::fs::symlink_metadata(e.path()).unwrap();
                if m.is_dir() {
                    stack.push(e.path());
                } else {
                    assert!(m.len() <= UPDATE_SNAPSHOT_FILE_CAP);
                    on_disk += m.len();
                }
            }
        }
        assert_eq!(on_disk, out.bytes, "색인의 바이트 합 = 실제 사본");
        // 일의 양(결정론): 담은 파일 + 상한으로 뺀 파일 = 같은 판정이 '덮는다/지운다'로 고른 파일 수.
        let plan = plan_install_core(&rig.pack, &v2, false, "9.9.9", PackScope::Base, &src);
        let planned = plan.update.len()
            + plan.heal.len()
            + plan.merge3.len()
            + plan.vendor_first.len()
            + plan.prune_delete.len();
        assert_eq!(out.files + out.skipped.len(), planned, "스냅샷의 일 = 이번 설치가 덮거나 지울 파일 수");
        assert!(planned <= v1.len());
        // 상한: 파일 하나가 상한을 넘으면 그 파일만 빠지고 색인에 남는다 — 설치는 계속된다.
        let big = "x".repeat((UPDATE_SNAPSHOT_FILE_CAP + 1) as usize);
        rig.write("bin/huge.txt", big.as_bytes());
        rig.write("bin/small.txt", b"small v1\n");
        rig.manifest(&[("bin/huge.txt", big.as_str()), ("bin/small.txt", "small v1\n")]);
        std::fs::write(rig.pack.join(PACK_VERSION_FILE), "2.0.0").unwrap();
        rig.install(&[("bin/huge.txt", "y\n"), ("bin/small.txt", "small v2\n")], false, &src);
        let snap = newest_update_snapshot(&root, "base").unwrap();
        let meta = read_update_snapshot_meta(&snap).unwrap();
        assert_eq!(meta.pack_version, "2.0.0");
        assert_eq!(meta.files.iter().map(|f| f.rel.as_str()).collect::<Vec<_>>(), vec!["bin/small.txt"]);
        assert_eq!(meta.skipped.len(), 1);
        assert_eq!(meta.skipped[0].rel, "bin/huge.txt");
        assert_eq!(meta.skipped[0].reason, "file-cap");
        assert_eq!(rig.read("bin/huge.txt").as_deref(), Some("y\n"), "설치는 계속됐다");
        // 총량 상한(작은 상한으로 직접): 넘치는 파일은 total-cap 으로 빠진다.
        for (r, n) in [("t/a", 600usize), ("t/b", 900), ("t/c", 2000)] {
            rig.write(r, "z".repeat(n).as_bytes());
        }
        let rels: Vec<String> = ["t/a", "t/b", "t/c"].iter().map(|s| s.to_string()).collect();
        let none = |_: &str| Vec::new();
        let o = create_update_snapshot(&rig.pack, &root, "capped", "1.0.0", &std::collections::BTreeMap::new(),
                                       &rels, PackScope::Base, &none, (1000, 1600)).unwrap();
        assert_eq!(o.files, 2);
        assert_eq!(o.skipped, vec![SnapshotSkip { rel: "t/c".into(), bytes: 2000, reason: "file-cap".into() }]);
        let o = create_update_snapshot(&rig.pack, &root, "capped2", "1.0.0", &std::collections::BTreeMap::new(),
                                       &rels, PackScope::Base, &none, (5000, 1600)).unwrap();
        assert_eq!(o.skipped.iter().map(|k| (k.rel.as_str(), k.reason.as_str())).collect::<Vec<_>>(),
                   vec![("t/c", "total-cap")]);
    }

    /// RULINGS AC1: 스냅샷 항목의 출시본 여부 — 바꾸기 전 설치 기록과 같은 바이트, 또는 (기록이 없을 때) 과거 출시 해시
    /// 표의 바이트만 참이다. 사용자 수정본(병합·vendor 우선 대상)은 거짓이다.
    #[test]
    fn m11_snapshot_records_whether_bytes_are_released() {
        let rig = Rig::new("snaprel");
        let root = rig.base.join("snapshots");
        rig.write("bin/unedited.sh", b"#!/bin/sh\nv1\n");
        rig.write("bin/old.sh", b"#!/bin/sh\nold release\n");
        rig.write("bin/edited.sh", b"#!/bin/sh\nmine\n");
        rig.manifest(&[("bin/unedited.sh", "#!/bin/sh\nv1\n"), ("bin/edited.sh", "#!/bin/sh\nv1\n")]);
        rig.pristine("bin/edited.sh", "#!/bin/sh\nv1\n");
        std::fs::write(rig.pack.join(PACK_VERSION_FILE), "1.0.0").unwrap();
        let old_hash = content_hash("#!/bin/sh\nold release\n");
        let src = UpdateSources::none()
            .with_snapshot("base", root.clone())
            .with_prev_vendor(move |r| if r == "bin/old.sh" { vec![old_hash.clone()] } else { Vec::new() });
        rig.install(&[("bin/unedited.sh", "#!/bin/sh\nv2\n"), ("bin/old.sh", "#!/bin/sh\nv2\n"),
                      ("bin/edited.sh", "#!/bin/sh\nv2\n")], false, &src);
        let meta = read_update_snapshot_meta(&newest_update_snapshot(&root, "base").unwrap()).unwrap();
        let rel = |r: &str| meta.files.iter().find(|f| f.rel == r).map(|f| f.released);
        assert_eq!(rel("bin/unedited.sh"), Some(true), "설치 기록과 같은 바이트");
        assert_eq!(rel("bin/old.sh"), Some(true), "기록 없음 + 과거 출시 해시 표의 바이트");
        assert_eq!(rel("bin/edited.sh"), Some(false), "사용자 수정본(병합 대상)");
    }

    /// RULINGS AC5: 비수정 판정(다 · 바)과 스냅샷의 출시본 판정이 쓰는 과거 출시 해시 표 — 라이브(`for_live_pack`)와
    /// `none()` 입력원은 `pack_prev_vendor::prev_vendor_hashes(rel)` 를 **그대로** 돌려준다(이음새로 바뀐 표가 제품
    /// 경로에 남으면 빨갛다). 표의 내용 자체는 T2c 의 것(재생성 · CI 신선도 검사)이다.
    #[test]
    fn m11_prev_vendor_lookup_is_the_table_on_production_sources() {
        let rig = Rig::new("prevtable");
        let _l = EnvGuard::set("CYS_LOCAL_DIR", rig.base.join("local"));
        let _p = EnvGuard::set(ENV_PACK_DIR, &rig.pack);
        let none = UpdateSources::none();
        let live = UpdateSources::for_live_pack(&rig.pack);
        let mut with_rows = 0usize;
        for (rel, _) in PACK_ALL.iter() {
            let want: Vec<String> = crate::pack_prev_vendor::prev_vendor_hashes(rel)
                .iter()
                .map(|h| h.to_string())
                .collect();
            if !want.is_empty() {
                with_rows += 1;
            }
            assert_eq!(none.prev_vendor_for(rel), want, "none(): {rel}");
            assert_eq!(live.prev_vendor_for(rel), want, "for_live_pack(): {rel}");
        }
        assert!(with_rows > 0, "표에 행이 없다 — 이 핀이 아무것도 재지 않는다");
        // 판정 수집기도 같은 조회를 쓴다: 표의 해시와 같은 디스크 바이트는 '과거 vendor 본'이다(시험용 표로 확인).
        let probe = "#!/bin/sh\nprobe\n";
        let h = content_hash(probe);
        let seam = UpdateSources::none().with_prev_vendor(move |_| vec![h.clone()]);
        let ctx = UpdateContext::new(&rig.pack, PackScope::Base, &seam, false);
        let f = ctx.facts("bin/probe.sh", "#!/bin/sh\nv2\n", true, Some(probe), None, &serde_json::Map::new());
        assert!(f.prev_vendor_match);
    }

    /// RULINGS AC7: vendor-first 원장 항목은 vendor 바이트보다 **먼저** 저장된다 — vendor 쓰기가 실패해(여기서는 살아 있는
    /// 파일을 바꿀 수 없게 막아) 설치가 멈춰도 `.user` 는 기록된 보관본이다(기록 없는 고아가 아니다).
    #[cfg(target_os = "macos")]
    #[test]
    fn m11_vendor_first_record_is_saved_before_the_vendor_bytes() {
        let rig = Rig::new("vforder");
        let rel = "bin/boot.sh";
        rig.write(rel, b"#!/bin/sh\nmine\n");
        let src = UpdateSources::none().with_boot_critical(|r| r == "bin/boot.sh");
        let p = rig.pack.join(rel);
        let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::chflags(c.as_ptr(), libc::UF_IMMUTABLE) }, 0, "불변 표시");
        let r = install_into_with(rig.pack.clone(), [(rel, "#!/bin/sh\nvendor\n")], false, "9.9.9", false, false,
                                  PackScope::Base, None, None, &src);
        unsafe { libc::chflags(c.as_ptr(), 0) };
        assert!(r.is_err(), "vendor 쓰기가 실패해야 시험이 성립한다: {r:?}");
        assert_eq!(rig.read(rel).as_deref(), Some("#!/bin/sh\nmine\n"), "살아 있는 파일 무변경");
        assert_eq!(std::fs::read_to_string(rig.pack.join(format!("{rel}.user"))).unwrap(), "#!/bin/sh\nmine\n");
        assert_eq!(rig.kind(rel).as_deref(), Some("vendor-first"), "멈춘 자리에서도 .user 는 원장에 기록된 보관본");
    }

    /// O-C8-4 (a): 라이브 입력원의 스냅샷 위치가 T0 경로 도우미(`repair::paths::update_snapshot_dir`)와 같은 배치다.
    #[test]
    fn m11_snapshot_root_matches_t0_path_helper() {
        let rig = Rig::new("snaproot");
        let local = rig.base.join("local");
        let _l = EnvGuard::set("CYS_LOCAL_DIR", &local);
        let _p = EnvGuard::set(ENV_PACK_DIR, &rig.pack);
        let src = UpdateSources::for_live_pack(&rig.pack);
        let lane = src.lane.clone().expect("base 팩 = base 레인");
        assert_eq!(lane.0, "base");
        let root = src.snapshot_root.clone().expect("스냅샷 루트");
        assert_eq!(
            crate::repair::paths::update_snapshot_dir(&lane, "1.0.0", "20261113T000000Z"),
            root.join("base").join("1.0.0-20261113T000000Z"),
            "스냅샷 경로가 T0 계약 경로와 갈렸다"
        );
    }

    /// 설계 §7 20행: staged init-pack 의 전량 복사가 심볼릭 링크를 링크로 지킨다(대상 없는 링크 포함 — 종전엔
    /// 복사가 실패해 init-pack 이 멈췄다).
    #[cfg(unix)]
    #[test]
    fn m11_staged_copy_preserves_symlinks() {
        let rig = Rig::new("links");
        let src = rig.base.join("src");
        std::fs::create_dir_all(src.join("skills")).unwrap();
        std::fs::write(rig.base.join("outside.md"), "MARK").unwrap();
        std::os::unix::fs::symlink(rig.base.join("outside.md"), src.join("skills/file-link.md")).unwrap();
        std::os::unix::fs::symlink("/nonexistent/x", src.join("skills/dangling")).unwrap();
        std::os::unix::fs::symlink(rig.base.join("src/skills"), src.join("dir-link")).unwrap();
        let dst = rig.base.join("dst");
        copy_dir_all(&src, &dst).expect("대상 없는 링크가 있어도 복사가 끝난다");
        for l in ["skills/file-link.md", "skills/dangling", "dir-link"] {
            assert!(std::fs::symlink_metadata(dst.join(l)).unwrap().file_type().is_symlink(), "{l} 은 링크로 남는다");
        }
        assert_eq!(std::fs::read_link(dst.join("skills/dangling")).unwrap(), PathBuf::from("/nonexistent/x"));
    }
}
