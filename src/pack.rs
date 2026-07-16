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

/// 설치 위치: $CYS_PACK_DIR (구 JAVIS_PACK_DIR·AITERM_JARVIS_DIR 폴백) 또는 ~/.cys/pack
pub fn pack_dir() -> PathBuf {
    if let Some(d) = crate::env_compat(ENV_PACK_DIR) {
        return PathBuf::from(d);
    }
    for legacy in ["JAVIS_PACK_DIR", "AITERM_JARVIS_DIR"] {
        if let Ok(d) = std::env::var(legacy) {
            if !d.is_empty() {
                return PathBuf::from(d);
            }
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cys/pack")
}

/// SessionStart hook 등록 명령을 OS별로 조립하는 **공용 함수**(RC-2 · 순수 함수·회귀 핀).
/// Windows: 바닐라 셸(cmd/PowerShell)은 `.sh`를 인터프리터 없이 못 실행하고 "open with" 대화상자를
///   띄운다(anthropics/claude-code #21847·#24097). Claude Code가 Windows에서 찾는 인터프리터는
///   Git Bash의 `bash`이므로 `bash`로 명시 호출한다(맨 이름 `sh`는 Git Bash가 `bash.exe`만 보장 → 회피).
/// Unix: 기존과 동일 `sh <path>`(제로 회귀).
/// cys.rs::hook_command(init-pack 경로)와 setup_isolated_config_dir(격리 config dir 경로)가 **둘 다**
/// 이 함수를 써서 두 경로의 인터프리터가 일치한다(구: 격리 경로만 `sh` 하드코딩 → Windows 불일치).
pub fn session_start_hook_command(pack_dir: &Path) -> String {
    let script = pack_dir.join("hooks/session-start.sh");
    if cfg!(windows) {
        // RC-2 잔여(T2.1·codex CONFIRMED): 공백 포함 경로(C:\Users\John Doe\.cys\pack\...) 대응 — Windows만
        // quote로 감싼다. unix는 **무변경**(기존 install에 등록된 미quote 문자열과 install_claude_hook의
        // already-매칭이 유지돼야 중복 등록이 안 생긴다 — quote 추가 시 불일치→매 기동 중복 append 회귀).
        // ★역슬래시→정슬래시 정규화(RC-3): git-bash가 C:\ 역슬래시를 escape/미해석해 경로를 파괴하는
        // 것(C:\Users\...→C:Users...→No such file) 방지. javis_preflight._cys_hook_cmd 의 Windows 형태와
        // **동일 문자열**을 방출 → 두 writer 간 중복 등록 0(matcher 일치).
        format!("bash \"{}\"", script.display().to_string().replace('\\', "/"))
    } else {
        format!("sh {}", script.display())
    }
}

/// cys 전용 CLAUDE_CONFIG_DIR — 사용자 ~/.claude(외부 터미널 체계·구 지침 오염 가능)와 **격리**한다.
/// cys가 띄우는 claude는 이 디렉터리만 읽으므로, 사용자 프로필이 오염돼 있어도 영향받지 않고
/// 사용자 프로필을 건드리지도(읽지도·지우지도) 않는다. macOS 인증은 계정 단위 Keychain이라
/// 격리해도 로그인이 유지된다(우리 DMG는 macOS 전용). pack_dir 형제(~/.cys/claude).
pub fn config_dir() -> PathBuf {
    config_dir_for(&pack_dir())
}

/// config_dir의 pack-dir 파라미터 코어 — 트랜잭션 진입점이 1회 캡처한 pack_dir을 스레딩하는
/// config 셋업 경로가 쓴다(전역 pack_dir() 재해석 금지). CYS_CONFIG_DIR 명시값은 종전대로 최우선
/// (별개 env — 외부 동작 불변), 부재 시 **주입된** pack_dir의 형제(~/.cys/claude)로 파생한다.
///
/// ★계약 명문화(codex R5 issue3): `CYS_CONFIG_DIR`는 **의도적·독립적인 config 목적지 override**이며
/// pack dir이 아니다. 이 값은 pack prune/journal을 절대 retarget할 수 없다(별개 env·config 전용).
/// config/hook 부수효과는 pack-path 콘텐츠에는 주입된 pack_dir을 쓰되, config **목적지**만 이 별개
/// override를 존중한다. 코드 동작 불변 — 계약 문서화 목적.
fn config_dir_for(pack_dir: &Path) -> PathBuf {
    if let Some(d) = crate::env_compat(ENV_CONFIG_DIR) {
        return PathBuf::from(d);
    }
    pack_dir
        .parent()
        .map(|p| p.join("claude"))
        .unwrap_or_else(|| PathBuf::from(".cys/claude"))
}

/// 격리 config dir 셋업: cys 라우터(CLAUDE.md)와 SessionStart hook(settings.json)을 설치한다.
/// ★보존 모드 — 기존 파일은 덮지 않는다(사용자 커스터마이즈 불가침). best-effort(실패해도
/// pack 설치 자체는 유효). 사용자 ~/.claude 는 절대 건드리지 않는다(격리의 핵심).
fn setup_isolated_config_dir(pack_dir: &Path) {
    let cfg = config_dir_for(pack_dir);
    if std::fs::create_dir_all(&cfg).is_err() {
        return;
    }
    // 라우터: 임베드 CLAUDE.md.template → <cfg>/CLAUDE.md (없을 때만 — 역할선언→~/.cys/pack 라우팅)
    let claude_md = cfg.join("CLAUDE.md");
    if !claude_md.exists() {
        if let Some((_, tmpl)) = PACK_ALL.iter().find(|(rel, _)| *rel == "CLAUDE.md.template") {
            let _ = std::fs::write(&claude_md, tmpl);
        }
    }
    // hook: <cfg>/settings.json 에 SessionStart → session-start.sh (없을 때만)
    let settings = cfg.join("settings.json");
    if !settings.exists() {
        // RC-2: OS-aware 공용 함수 사용 — Windows는 bash(Git Bash가 bash.exe만 보장·cmd/PowerShell은
        // .sh를 인터프리터 없이 못 실행). 구 `sh` 하드코딩 제거(격리 config dir·init-pack 경로 일치).
        let hook = session_start_hook_command(pack_dir);
        let json = serde_json::json!({
            "hooks": { "SessionStart": [ { "hooks": [ { "type": "command", "command": hook } ] } ] }
        });
        if let Ok(s) = serde_json::to_string_pretty(&json) {
            let _ = std::fs::write(&settings, s);
        }
    }
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
    if rel.ends_with("_DIRECTIVE.md")
        || rel == "soul.md"
        || rel.ends_with("/soul.md")
        || rel == "CLAUDE.md"
        || rel.ends_with("/CLAUDE.md")
        // ★B2-1(W3): schedule.json 은 사용자가 `cys schedule add` 로 편집하는 혼합 파일 — 팩 강제갱신이 덮으면
        // 사용자 잡이 소실(비가역 데이터 손실)된다. user 소유로 보존하고, built-in 잡(phoenix-*)은 데몬 부트 시
        // 코드가 idempotent ensure 한다(cysd schedule::ensure_builtin_jobs). 기본 잡 드리프트(복구 가능) < 사용자 잡 소실.
        || rel == "schedule.json"
    {
        return Ownership::User;
    }
    Ownership::System
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
pub(crate) fn decide_file_action(
    rel: &str,
    embed: &str,
    exists: bool,
    disk: Option<&str>, // None = 부재 또는 읽기 실패(비UTF-8 등)
    manifest_hash: Option<&str>,
    force: bool,
) -> FileAction {
    // ★B2-2 seed-once 상태: 존재하면 불가침(force 여도·읽기 실패여도) — 부재 시에만 아래 시드 설치.
    if exists && is_seed_once(rel) {
        return FileAction::Keep { adopt_hash: false, new_pending: false };
    }
    // ★B2 user-owned 영구 보존 (force 여도) — 읽기 성공 + 내용 상이일 때.
    if exists && is_user_owned(rel) {
        if let Some(d) = disk {
            if d != embed {
                // 임베드가 마지막 적용본(매니페스트 해시)에서 전진했으면 신버전 병치(병합 대기).
                // 매니페스트 부재(구설치본)도 안전측으로 병치해 가시화한다(base 없는 2-way 병합).
                let new_pending = manifest_hash != Some(content_hash(embed).as_str());
                return FileAction::Keep { adopt_hash: false, new_pending };
            }
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
                if is_user_owned(rel) {
                    // (여기 도달 = 읽기 실패 케이스 — 내용 상이는 위 첫 블록이 잡는다) 보존.
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
                if (is_user_owned(rel) || is_seed_once(rel)) && disk.as_deref() != Some(content) {
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
            if embedded.contains(rel.as_str()) || is_user_owned(rel) || is_seed_once(rel) {
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
pub fn install(force: bool) -> Result<(usize, usize), String> {
    // 얇은 래퍼: embed PACK_ALL(git-추적 전체 트리)를 입력원으로 install_from_iter에 위임한다.
    // ★외부 동작(반환값·디스크 결과·부수효과)은 완전 불변 — C/D/E 호출처 무영향(§3 하위호환).
    install_from_iter(
        PACK_ALL.iter().map(|(r, c)| (*r, *c)),
        force,
        env!("CARGO_PKG_VERSION"),
        false, // embed/cysd 경로(비트랜잭션): .pack-version 직접 기록 + 매니페스트 best-effort(외부 동작 불변).
    )
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
pub fn install_into<'a, I: IntoIterator<Item = (&'a str, &'a str)>>(
    dir: PathBuf,
    items: I,
    force: bool,
    target_version: &str,
    transactional: bool,
    setup_config: bool,
) -> Result<(usize, usize), String> {
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
    for (rel, content) in items.iter().copied() {
        let path = dir.join(rel);
        let exists = path.exists();
        let disk = if exists { std::fs::read_to_string(&path).ok() } else { None };
        let mhash: Option<String> = manifest.get(rel).cloned();
        match decide_file_action(rel, content, exists, disk.as_deref(), mhash.as_deref(), force) {
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
                } else if pending
                    .get(rel)
                    .and_then(|e| e.get("kind"))
                    .and_then(|k| k.as_str())
                    == Some("new-pending")
                {
                    // 병합 대기 해소(사용자가 vendor 본 채택 등) — 원장·.new 잔재 청소.
                    pending.remove(rel);
                    pending_dirty = true;
                    let _ = std::fs::remove_file(dir.join(format!("{rel}.new")));
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
    if !pending.is_empty() {
        println!(
            "[init-pack] 커스터마이즈 병합 대기 {}건 — `cys pack-merge` 로 검토 (신버전 .new 병치 / 치유 전 사용자본 .user 보존)",
            pending.len()
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
                if is_user_owned(&rel) || is_seed_once(&rel) {
                    continue; // ★B2: user 소유·seed-once 상태는 영구 보존 — prune 대상 제외
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
        // ★캡처한 dir을 config 셋업에 스레딩 — 트랜잭션 경로(apply_pack_transactional_in)에서
        // 전역 pack_dir()/config_dir() 재해석으로 잘못된 pack 경로를 참조하는 hook·config 기록을
        // 구조적으로 차단한다(issue2). 비트랜잭션 경로(install_from_iter, dir==pack_dir())는 동일 값이라 무변경.
        setup_isolated_config_dir(&dir);
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
) -> Result<(usize, usize), String> {
    install_into(pack_dir(), items, force, target_version, transactional, true)
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
pub fn install_staged(force: bool) -> Result<(usize, usize), String> {
    let dir = pack_dir();
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
    //    1회 캡처한 dir(=pack_dir())을 스레딩(setup_isolated_config_dir 시그니처 정합 — 값 동일·무변경).
    setup_isolated_config_dir(&dir);
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
/// pack_dir을 명시 인자로 받는다(트랜잭션 진입점이 1회 캡처한 dir을 스레딩 — 전역 env 재해석 금지).
///
/// ★API 계약(codex R5 issue1): 이 함수는 **crate 내부 트랜잭션 헬퍼**이며 지원되는 공개 API가
/// 아니다(구 `pub` → `pub(crate)` 강등은 의도적 — 외부 호출처 0 검증됨, 우발적 breaking change 아님).
/// 지원되는 공개 표면은 3개 래퍼뿐: `apply_pack_transactional`·`recover_pack_journal`·`license::install`.
pub(crate) fn pack_journal_dir(dir: &Path) -> PathBuf {
    dir.parent()
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
    dir: &Path,
    target_version: &str,
    backup_set: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let jdir = pack_journal_dir(dir);
    let _ = std::fs::remove_dir_all(&jdir);
    let files_dir = jdir.join("files");
    std::fs::create_dir_all(&files_dir)
        .map_err(|e| format!("journal files dir 생성 실패 {}: {e}", files_dir.display()))?;
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
///
/// ★API 계약(codex R5 issue1): 이 함수는 **crate 내부 트랜잭션 헬퍼**이며 지원되는 공개 API가
/// 아니다(구 `pub` → `pub(crate)` 강등은 의도적 — 외부 호출처 0 검증됨, 우발적 breaking change 아님).
/// 지원되는 공개 표면은 3개 래퍼뿐: `apply_pack_transactional`·`recover_pack_journal`·`license::install`.
pub(crate) fn rollback_journal(dir: &Path) -> Result<(), String> {
    let jdir = pack_journal_dir(dir);
    let index_path = jdir.join("index.json");
    let s = std::fs::read_to_string(&index_path)
        .map_err(|e| format!("journal 인덱스 읽기 실패 {}: {e}", index_path.display()))?;
    let index: JournalIndex =
        serde_json::from_str(&s).map_err(|e| format!("journal 인덱스 파싱 실패: {e}"))?;
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
    recover_pack_journal_in(&pack_dir())
}

/// recover_pack_journal의 dir-파라미터 코어 — pack_dir을 1회 캡처해 스레딩하는 트랜잭션 진입점
/// (apply_pack_transactional_in)과 외부 standalone 호출(공개 래퍼)이 공유한다. 내부에서 pack_dir()을
/// 재해석하지 않는다(전역 env 재해석으로 인한 실 pack_dir 오탐·mass-prune 방지 — 구조적 차단).
pub fn recover_pack_journal_in(dir: &Path) -> Result<bool, String> {
    let jdir = pack_journal_dir(dir);
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
    let disk_version = std::fs::read_to_string(dir.join(PACK_VERSION_FILE))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if !disk_version.is_empty() && disk_version == index.target_version {
        // 커밋 성공(.pack-version == target) → 저널 정리만(롤백 금지).
        let _ = std::fs::remove_dir_all(&jdir);
    } else {
        // 미커밋 → 롤백(pre-state 복원 + 저널 삭제).
        rollback_journal(dir)?;
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
    post_commit: F,
) -> Result<(usize, usize, bool), String>
where
    F: FnOnce() -> Result<(), String>,
{
    // 공개 래퍼: pack_dir을 **여기서 1회** 캡처해 dir-파라미터 코어에 스레딩한다. 코어 안의 어떤
    // 하위 함수도 pack_dir()을 재해석하지 않으므로, 트랜잭션 도중 CYS_PACK_DIR가 바뀌어도 이 트랜잭션이
    // 실 pack_dir을 mass-prune하는 버그 클래스가 구조적으로 불가능하다(외부 호출처 시그니처 불변).
    apply_pack_transactional_in(&pack_dir(), items, target_version, state, post_commit)
}

/// apply_pack_transactional의 dir-파라미터 코어. `dir`은 트랜잭션 진입점이 1회 캡처한 pack_dir이며
/// journal·rollback·install(prune)·state·commit marker 전 계층이 **이 한 값만** 사용한다(재해석 0).
pub fn apply_pack_transactional_in<F>(
    dir: &Path,
    items: &[(&str, &str)],
    target_version: &str,
    state: &PackState,
    post_commit: F,
) -> Result<(usize, usize, bool), String>
where
    F: FnOnce() -> Result<(), String>,
{
    // ⓪ 직전 crash로 남은 orphan 저널 자가치유(새 트랜잭션 전 pre-state 확정).
    recover_pack_journal_in(dir)?;
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
    write_journal(dir, target_version, &backup_set)?;
    // ② 파일 반영(transactional=true) — .pack-version은 여기서 쓰지 않고(④에서 commit marker로),
    //    .install-manifest.json write 실패는 fail-closed로 Err가 되어 아래 rollback을 탄다.
    //    캡처한 dir을 install_into에 직접 넘긴다(install_from_iter의 pack_dir() 재해석 우회 —
    //    setup_config=true는 install_from_iter와 동일: 외부 동작 불변).
    let (written, kept) = match install_into(
        dir.to_path_buf(),
        items.iter().copied(),
        false,
        target_version,
        true,
        true,
    ) {
        Ok(v) => v,
        Err(e) => {
            let _ = rollback_journal(dir);
            return Err(format!("파일 반영 실패(rollback 완료): {e}"));
        }
    };
    // ③ `.pack-state.json` 기록 — journal 백업 대상이므로 실패 시 rollback으로 전체 복원.
    if let Err(e) = write_pack_state(dir, state) {
        let _ = rollback_journal(dir);
        return Err(format!("state 기록 실패(rollback 완료): {e}"));
    }
    // ④ .pack-version = 마지막 hard commit marker(결과 검사 — best-effort 금지).
    if let Err(e) = write_atomic(&dir.join(PACK_VERSION_FILE), target_version.as_bytes()) {
        let _ = rollback_journal(dir);
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
    let _ = std::fs::remove_dir_all(pack_journal_dir(dir));
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

    #[test]
    fn role_directive_exact_master() {
        // master는 정확 일치만 — 'masterful' 같은 변형은 매핑 없음
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
    fn role_directive_prefix_variants_map_to_standard() {
        // 접두 일치: 변형 역할(worker-2·reviewer-gemini·cso-1)도 표준 지침을 받는다
        // — 디렉티브 주입(각성)이 변형 역할에서 누락되지 않게 하는 핵심 불변식.
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
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-pack-install-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let cfgdir = td.join("cysclaude"); // 격리 config dir(테스트 밀폐 — td와 함께 정리)
        std::env::set_var(ENV_PACK_DIR, &td);
        std::env::set_var(ENV_CONFIG_DIR, &cfgdir);
        let result = install(false);
        match saved {
            Some(v) => std::env::set_var(ENV_PACK_DIR, v),
            None => std::env::remove_var(ENV_PACK_DIR),
        }
        match saved_cfg {
            Some(v) => std::env::set_var(ENV_CONFIG_DIR, v),
            None => std::env::remove_var(ENV_CONFIG_DIR),
        }
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
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-pack-downgrade-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::env::set_var(ENV_PACK_DIR, &td);
        std::env::set_var(ENV_CONFIG_DIR, td.join("cysclaude"));

        let embed = env!("CARGO_PKG_VERSION");
        // 1) 정상 설치 → .pack-version = embed 기록
        install(false).expect("최초 install 실패");
        let disk_v1 = std::fs::read_to_string(td.join(PACK_VERSION_FILE)).unwrap();
        // 2) 디스크 .pack-version을 더 새 버전으로 위조(구버전 cys 롤백/오설치 시뮬)
        std::fs::write(td.join(PACK_VERSION_FILE), "99.0.0").unwrap();
        // 3) install(false) → 다운그레이드 차단 → (0,0), .pack-version 유지(embed로 안 덮음)
        let blocked = install(false).expect("install 실패");
        let disk_after = std::fs::read_to_string(td.join(PACK_VERSION_FILE)).unwrap();
        // 4) force는 우회 → 갱신
        install(true).expect("force install 실패");
        let disk_forced = std::fs::read_to_string(td.join(PACK_VERSION_FILE)).unwrap();

        // env 복원(assert 전 — 패닉해도 전역 env 누수 없게)
        match saved {
            Some(v) => std::env::set_var(ENV_PACK_DIR, v),
            None => std::env::remove_var(ENV_PACK_DIR),
        }
        match saved_cfg {
            Some(v) => std::env::set_var(ENV_CONFIG_DIR, v),
            None => std::env::remove_var(ENV_CONFIG_DIR),
        }
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
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-pack-threeway-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::env::set_var(ENV_PACK_DIR, &td);
        std::env::set_var(ENV_CONFIG_DIR, td.join("cysclaude"));

        let get = |rel: &str| PACK_ALL.iter().find(|(r, _)| *r == rel).map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("팩에 {rel} 부재"));
        let sys_a = "README.md";  // system·비수정 → 갱신 + pristine 미러
        let user_b = "soul.md";   // user-owned·수정 + vendor 전진 → 보존 + .new + pending
        let sys_c = "acl.json";   // system·수정 → 치유 + .user + pending
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

        install(false).expect("install 실패");
        let read = |rel: &str| std::fs::read_to_string(td.join(rel)).unwrap();

        // ① user-owned: 보존 + 신버전 .new 병치 + 원장 new-pending.
        assert_eq!(read(user_b), "USER-SOUL", "user-owned 보존 불변");
        assert_eq!(read("soul.md.new"), get(user_b), ".new = 임베드 신버전");
        // ② system 수정본: 치유(임베드) + 사용자본 .user 보존 + 원장 healed.
        assert_eq!(read(sys_c), get(sys_c), "system 치유(P0-4 불변)");
        assert_eq!(read("acl.json.user"), "SYS-DRIFT", "치유 전 사용자본 보존(파괴 0)");
        let pending = load_merge_pending(&td);
        assert_eq!(pending.get(user_b).and_then(|e| e["kind"].as_str()), Some("new-pending"));
        assert_eq!(pending.get(sys_c).and_then(|e| e["kind"].as_str()), Some("healed"));
        // ③ pristine 미러: 적용된 vendor 본만(sys_a·sys_c), 동결 user-owned(user_b)는 미기록(조상 보존).
        assert_eq!(read(&format!("{PRISTINE_DIR}/{sys_a}")), get(sys_a));
        assert_eq!(read(&format!("{PRISTINE_DIR}/{sys_c}")), get(sys_c));
        assert!(!td.join(PRISTINE_DIR).join(user_b).exists(), "동결 파일 조상은 미갱신");
        // ④ 멱등: 재실행해도 상태 동일(원장 중복 기록·불필요 rewrite 없음).
        install(false).expect("재실행 실패");
        assert_eq!(read(user_b), "USER-SOUL");
        assert_eq!(load_merge_pending(&td).len(), 2);
        // ⑤ 해소: 사용자가 vendor 본 채택(디스크=임베드) → .new·원장 항목 자동 청소.
        std::fs::write(td.join(user_b), get(user_b)).unwrap();
        install(false).expect("3차 실행 실패");
        assert!(!td.join("soul.md.new").exists(), "채택 후 .new 청소");
        assert!(load_merge_pending(&td).get(user_b).is_none(), "채택 후 원장 소거");

        let _ = std::fs::remove_dir_all(&td);
        match saved { Some(v) => std::env::set_var(ENV_PACK_DIR, v), None => std::env::remove_var(ENV_PACK_DIR) }
        match saved_cfg { Some(v) => std::env::set_var(ENV_CONFIG_DIR, v), None => std::env::remove_var(ENV_CONFIG_DIR) }
    }

    /// ★플랜=실제 무드리프트(④): plan_install 분류가 같은 픽스처의 install 실행 결과와 일치.
    #[test]
    fn plan_install_matches_actual_install_actions() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-pack-plan-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::env::set_var(ENV_PACK_DIR, &td);
        std::env::set_var(ENV_CONFIG_DIR, td.join("cysclaude"));

        std::fs::create_dir_all(&td).unwrap();
        std::fs::write(td.join("README.md"), "OLD-INSTALLED").unwrap();
        std::fs::write(td.join("soul.md"), "USER-SOUL").unwrap();
        std::fs::write(td.join("acl.json"), "SYS-DRIFT").unwrap();
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
        assert!(plan.heal.iter().any(|r| r == "acl.json"), "system 수정 → heal");
        // 실제 install 이 플랜과 같은 행동을 하는지 대조.
        install(false).expect("install 실패");
        let read = |rel: &str| std::fs::read_to_string(td.join(rel)).unwrap();
        assert_eq!(read("soul.md"), "USER-SOUL");
        assert!(td.join("soul.md.new").exists());
        assert!(td.join("acl.json.user").exists());

        let _ = std::fs::remove_dir_all(&td);
        match saved { Some(v) => std::env::set_var(ENV_PACK_DIR, v), None => std::env::remove_var(ENV_PACK_DIR) }
        match saved_cfg { Some(v) => std::env::set_var(ENV_CONFIG_DIR, v), None => std::env::remove_var(ENV_CONFIG_DIR) }
    }

    #[test]
    fn install_ownership_system_forced_user_preserved() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-pack-ownership-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::env::set_var(ENV_PACK_DIR, &td);
        std::env::set_var(ENV_CONFIG_DIR, td.join("cysclaude")); // 격리(밀폐)

        let get = |rel: &str| PACK_ALL.iter().find(|(r, _)| *r == rel).map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("팩에 {rel} 부재"));
        let sys_a = "README.md";       // system·비수정(manifest 일치) → 갱신
        let user_b = "soul.md";        // user·수정 → 보존
        let sys_c = "acl.json";        // system·manifest 부재·상이 → 강제 갱신(B2/P0-4)
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

        install(false).expect("install 실패");
        let read = |rel: &str| std::fs::read_to_string(td.join(rel)).unwrap();

        assert_eq!(read(sys_a), sys_a_c, "①system 비수정 → 임베드로 갱신");
        assert_eq!(read(user_b), "USER-MODIFIED", "②user 수정본 불가침");
        assert_eq!(read(sys_c), sys_c_c, "③★B2/P0-4: system 매니페스트부재·상이 → 강제 갱신(동결 금지)");
        assert_eq!(read(user_d), "USER-CUSTOM", "④user 매니페스트부재·상이 → 보존");

        // ⑤ 채택 기록 + 멱등: 재실행이 아무것도 다시 쓰지 않고 user 보존 유지.
        let m: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&read(INSTALL_MANIFEST)).unwrap();
        assert_eq!(m.get(sys_a), Some(&content_hash(sys_a_c)), "갱신 후 매니페스트 미반영");
        let (w2, _) = install(false).unwrap();
        match saved {
            Some(v) => std::env::set_var(ENV_PACK_DIR, v),
            None => std::env::remove_var(ENV_PACK_DIR),
        }
        match saved_cfg {
            Some(v) => std::env::set_var(ENV_CONFIG_DIR, v),
            None => std::env::remove_var(ENV_CONFIG_DIR),
        }
        assert_eq!(w2, 0, "멱등 위반: 재실행이 {w2}개를 다시 씀");
        assert_eq!(std::fs::read_to_string(td.join(user_b)).unwrap(), "USER-MODIFIED");
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★B2-1(W3): schedule.json 은 user-owned — 팩 **강제갱신(force)** 후에도 사용자 잡이 소실되지 않는다.
    /// (built-in phoenix 잡은 데몬 부트 ensure_builtin_jobs 가 별도로 upsert — 이 테스트는 사용자 잡 보존만 검증.)
    #[test]
    fn install_force_preserves_user_schedule_jobs() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-sched-owner-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::env::set_var(ENV_PACK_DIR, &td);
        std::env::set_var(ENV_CONFIG_DIR, td.join("cysclaude"));
        std::fs::create_dir_all(&td).unwrap();

        // 사용자가 `cys schedule add` 로 넣은 잡이 담긴 schedule.json(임베드와 상이).
        let user_schedule = r#"{"jobs":[{"id":"my-daily-brief","every_minutes":1440,"action":"push","to":"master","text":"USER JOB"}]}"#;
        std::fs::write(td.join("schedule.json"), user_schedule).unwrap();

        // force=true 강제갱신 — user-owned schedule.json 은 보존돼야 한다.
        install(true).expect("install(force) 실패");
        let after = std::fs::read_to_string(td.join("schedule.json")).unwrap();

        match saved {
            Some(v) => std::env::set_var(ENV_PACK_DIR, v),
            None => std::env::remove_var(ENV_PACK_DIR),
        }
        match saved_cfg {
            Some(v) => std::env::set_var(ENV_CONFIG_DIR, v),
            None => std::env::remove_var(ENV_CONFIG_DIR),
        }
        assert!(
            after.contains("my-daily-brief") && after.contains("USER JOB"),
            "강제갱신이 사용자 schedule.json 잡을 소실시켰다 — B2-1 위반. after={after}"
        );
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★B2 분류 순수 함수: user 화이트리스트(디렉티브·헌법·CLAUDE.md)만 preserve, 나머지=system.
    #[test]
    fn is_user_owned_classification() {
        for u in ["soul.md", "directives/MASTER_DIRECTIVE.md", "CLAUDE.md",
                  "sub/dir/CSO_DIRECTIVE.md", "some/soul.md", "schedule.json"] {
            assert!(is_user_owned(u), "user 여야: {u}");
        }
        for s in ["bin/javis_phoenix.py", "hooks/session_start.sh", "README.md",
                  "acl.json", "CLAUDE.md.template", "skills/x/SKILL.md",
                  "directives/CEO_TEMPLATE.md", "sub/schedule.json"] {
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
                    "schedule.json", "bin/javis_phoenix.py"] {
            assert!(!(is_user_owned(rel) && is_seed_once(rel)), "이중 등급: {rel}");
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
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let keys = [
            "CYS_PACK_DIR",
            "JAVIS_PACK_DIR",
            "AITERM_PACK_DIR",
            "AITERM_JARVIS_DIR",
        ];
        let saved: Vec<(&str, Option<String>)> =
            keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
        for k in keys {
            std::env::remove_var(k);
        }

        // 셋 다 없으면 기본 ~/.cys/pack (홈 끝 2요소가 .cys/pack)
        let def = pack_dir();
        assert!(
            def.ends_with(".cys/pack"),
            "기본 경로는 .cys/pack: {def:?}"
        );

        // AITERM_JARVIS_DIR만 → 3순위로 도달 (env_compat이 못 만드는 키, 루프 전용 경로)
        std::env::set_var("AITERM_JARVIS_DIR", "/legacy/aiterm");
        assert_eq!(pack_dir(), PathBuf::from("/legacy/aiterm"));

        // JAVIS_PACK_DIR 추가 → AITERM_JARVIS_DIR보다 우선 (2순위)
        std::env::set_var("JAVIS_PACK_DIR", "/legacy/javis");
        assert_eq!(pack_dir(), PathBuf::from("/legacy/javis"));

        // CYS_PACK_DIR 추가(env_compat primary) → 최우선 (1순위)
        std::env::set_var("CYS_PACK_DIR", "/modern/cys");
        assert_eq!(pack_dir(), PathBuf::from("/modern/cys"));

        // env_compat 폴백: CYS_PACK_DIR 비우면 JAVIS_PACK_DIR로(=2순위와 동일 키지만
        // env_compat 경로) — 빈 문자열은 미설정 취급이라 다음 후보로 넘어간다
        std::env::set_var("CYS_PACK_DIR", "");
        assert_eq!(pack_dir(), PathBuf::from("/legacy/javis"));

        // 복원
        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
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
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let base =
            std::env::temp_dir().join(format!("cys-pack-equiv-test-{}", std::process::id()));
        let td_a = base.join("a"); // install(false)
        let td_b = base.join("b"); // install_from_iter
        let _ = std::fs::remove_dir_all(&base);

        // 격리 config dir은 pack dir **밖**에 둔다 — settings.json이 pack_dir 절대경로
        // (hooks/session-start.sh)를 박으므로 td 안에 두면 td_a≠td_b 경로 차이가 핑거프린트를
        // 오염시킨다. pack dir 콘텐츠 자체는 경로 무관 결정론이라 이 분리로 순수 등가 비교가 된다.
        // A: 기존 래퍼
        std::env::set_var(ENV_PACK_DIR, &td_a);
        std::env::set_var(ENV_CONFIG_DIR, base.join("cfg-a"));
        let res_a = install(false);
        let fp_a = fingerprint_dir(&td_a);

        // B: 추출 코어 직접 호출(동일 입력원·동일 버전)
        std::env::set_var(ENV_PACK_DIR, &td_b);
        std::env::set_var(ENV_CONFIG_DIR, base.join("cfg-b"));
        let res_b = install_from_iter(
            PACK_ALL.iter().map(|(r, c)| (*r, *c)),
            false,
            env!("CARGO_PKG_VERSION"),
            false,
        );
        let fp_b = fingerprint_dir(&td_b);

        match saved {
            Some(v) => std::env::set_var(ENV_PACK_DIR, v),
            None => std::env::remove_var(ENV_PACK_DIR),
        }
        match saved_cfg {
            Some(v) => std::env::set_var(ENV_CONFIG_DIR, v),
            None => std::env::remove_var(ENV_CONFIG_DIR),
        }
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

    fn txn_prestate(tag: &str, files: &[(&str, &str)], version: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("cys-journal-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        // ENV_PACK_DIR은 install_from_iter/install_staged 계열 테스트(이 fixture 공유·미변경 경로)가
        // pack_dir()을 env로 해석하므로 계속 설정한다. txn 테스트(_in 코어 호출)는 이 값에 의존하지 않고
        // pd를 dir 인자로 직접 주입해 파괴 경로(journal·prune·rollback)를 격리한다.
        std::env::set_var(ENV_PACK_DIR, &pd);
        std::env::set_var(ENV_CONFIG_DIR, base.join("cfg"));
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
        (base, pd)
    }

    fn restore_env(saved: Option<String>, saved_cfg: Option<String>) {
        match saved {
            Some(v) => std::env::set_var(ENV_PACK_DIR, v),
            None => std::env::remove_var(ENV_PACK_DIR),
        }
        match saved_cfg {
            Some(v) => std::env::set_var(ENV_CONFIG_DIR, v),
            None => std::env::remove_var(ENV_CONFIG_DIR),
        }
    }

    /// 정상 경로: 파일 반영·prune·record_accepted(closure)·.pack-version commit marker 기록 후
    /// 저널이 삭제된다.
    #[test]
    fn apply_transactional_commit_then_journal_removed() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let (base, pd) = txn_prestate(
            "commit",
            &[("README.md", "OLD-SOUL"), ("stale.txt", "STALE")],
            "1.0.0",
        );

        // README.md 갱신 + new.txt 추가, stale.txt는 items 부재 → prune.
        let items: Vec<(&str, &str)> = vec![("README.md", "NEW-SOUL"), ("new.txt", "NEW")];
        let committed = std::cell::Cell::new(false);
        let res = apply_pack_transactional_in(&pd, &items, "2.0.0", &test_free_state("2.0.0"), || {
            committed.set(true);
            Ok(())
        });

        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let newf = std::fs::read_to_string(pd.join("new.txt")).unwrap();
        let stale_exists = pd.join("stale.txt").exists();
        let journal_exists = pack_journal_dir(&pd).exists();
        restore_env(saved, saved_cfg);
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
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let (base, pd) = txn_prestate("fault", &[("README.md", "OLD-SOUL")], "1.0.0");
        let pre_fp = fingerprint_dir(&pd);

        // README.md 갱신(1번째 성공) → collide 파일(2번째 성공) → collide/child(3번째: 부모가
        // 파일이라 create_dir_all 실패) = mid-apply fault.
        let items: Vec<(&str, &str)> =
            vec![("README.md", "NEW"), ("collide", "X"), ("collide/child", "Y")];
        let res = apply_pack_transactional_in(&pd, &items, "2.0.0", &test_free_state("2.0.0"), || Ok(()));

        let post_fp = fingerprint_dir(&pd);
        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let journal_exists = pack_journal_dir(&pd).exists();
        restore_env(saved, saved_cfg);
        let _ = std::fs::remove_dir_all(&base);

        assert!(res.is_err(), "mid-apply fault인데 성공 반환");
        assert_eq!(pv.trim(), "1.0.0", ".pack-version 불변이어야(미커밋)");
        assert!(!journal_exists, "rollback 후 저널 잔존");
        assert_eq!(pre_fp, post_fp, "rollback이 pre-state로 복원 못함(부분적용 잔존)");
    }

    /// v4 §3 재배치 핀(R3 codex blocking 결착): record_accepted는 post-commit — 실패해도
    /// 커밋(파일·state·.pack-version)은 유효하게 남고 rollback하지 않으며, 성공으로 침묵
    /// 포장하지 않고 post_ok=false로 구분 보고한다. (구 동작: pre-commit이라 실패 시 전체
    /// rollback → 낡은 accepted가 정품 번들 재시도를 replay 거부하는 crash 교착의 원천이었다.)
    #[test]
    fn post_commit_failure_keeps_commit_and_reports() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let (base, pd) = txn_prestate(
            "recordfail",
            &[("README.md", "OLD-SOUL"), ("stale.txt", "STALE")],
            "1.0.0",
        );

        // 파일 반영·prune·커밋은 성공, post-commit record_accepted만 실패.
        let items: Vec<(&str, &str)> = vec![("README.md", "NEW-SOUL"), ("new.txt", "NEW")];
        let res = apply_pack_transactional_in(&pd, &items, "2.0.0", &test_free_state("2.0.0"), || {
            Err("record_accepted boom".into())
        });

        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let stale_exists = pd.join("stale.txt").exists();
        let journal_exists = pack_journal_dir(&pd).exists();
        restore_env(saved, saved_cfg);
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
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        // crash 후 디스크: .pack-version 옛 1.0.0(미커밋) + README.md 부분반영 + new.txt 신규생성.
        let (base, pd) = txn_prestate("orphan-rb", &[("README.md", "PARTIAL-NEW")], "1.0.0");
        std::fs::write(pd.join("new.txt"), "ORPHAN-NEW").unwrap();
        // 저널 수작업 조립: target 2.0.0, README.md(existed) backup=OLD-SOUL, new.txt(신규) existed=false.
        let jdir = pack_journal_dir(&pd);
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

        let recovered = recover_pack_journal_in(&pd);

        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let new_exists = pd.join("new.txt").exists();
        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let journal_exists = pack_journal_dir(&pd).exists();
        restore_env(saved, saved_cfg);
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
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        // 커밋 성공: .pack-version=2.0.0, README.md=NEW-SOUL(새 내용).
        let (base, pd) = txn_prestate("orphan-commit", &[("README.md", "NEW-SOUL")], "2.0.0");
        let jdir = pack_journal_dir(&pd);
        let files_dir = jdir.join("files");
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::write(files_dir.join("README.md"), "OLD-SOUL").unwrap(); // 커밋 전 백업본
        let index = serde_json::json!({
            "target_version": "2.0.0",
            "entries": [{"rel": "README.md", "existed": true}]
        });
        std::fs::write(jdir.join("index.json"), index.to_string()).unwrap();

        let recovered = recover_pack_journal_in(&pd);

        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let journal_exists = pack_journal_dir(&pd).exists();
        restore_env(saved, saved_cfg);
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
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let (base, pd) = txn_prestate("manifest-fail", &[("README.md", "OLD-SOUL")], "1.0.0");
        // .install-manifest.json을 디렉터리로 치환 → write_atomic(rename) 실패 유발(IO fault 주입).
        let mp = pd.join(INSTALL_MANIFEST);
        std::fs::remove_file(&mp).unwrap();
        std::fs::create_dir_all(mp.join("child")).unwrap();

        let items: Vec<(&str, &str)> = vec![("README.md", "NEW-SOUL"), ("new.txt", "NEW")];
        let committed = std::cell::Cell::new(false);
        let res = apply_pack_transactional_in(&pd, &items, "2.0.0", &test_free_state("2.0.0"), || {
            committed.set(true);
            Ok(())
        });

        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let new_exists = pd.join("new.txt").exists();
        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let journal_exists = pack_journal_dir(&pd).exists();
        restore_env(saved, saved_cfg);
        let _ = std::fs::remove_dir_all(&base);

        assert!(res.is_err(), "매니페스트 write 실패인데 성공 반환(best-effort 흡수)");
        assert!(!committed.get(), "파일 반영 실패 전에 commit_extra가 호출되면 안됨");
        assert_eq!(soul, "OLD-SOUL", "rollback이 README.md를 pre-state로 복원 못함");
        assert!(!new_exists, "rollback이 신규 new.txt를 제거 못함(부분적용 잔존)");
        assert_eq!(pv.trim(), "1.0.0", ".pack-version 불변이어야(미커밋)");
        assert!(!journal_exists, "rollback 후 저널 잔존");
    }

    /// ★격리 회귀(issue4 — 2026-07-15 팩소실 재발 방지의 결정적 증거): 트랜잭션은 **주입된 dir**만
    /// 파괴 대상으로 삼고, 전역 CYS_PACK_DIR(env)이 가리키는 실 pack은 절대 건드리지 않음을 증명한다.
    /// 기존 txn 테스트는 env와 주입 dir을 같은 pd로 두어(txn_prestate가 env=pd 설정), 트랜잭션이
    /// pack_dir()을 재해석해도 같은 pd로 수렴해 통과하는 false-green이었다. 여기서는 env를 **POISON**
    /// (주입 dir과 다른 별도 temp)으로 돌려, 회귀(내부 pack_dir() 재해석)가 있으면 POISON이 prune/overwrite
    /// 당해 아래 assert가 실패하도록 만든다. 이 테스트가 있었다면 2026-07-15 재앙을 잡았다.
    #[test]
    fn apply_transactional_ignores_env_uses_injected_dir() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();

        // 주입 대상(pd): 트랜잭션이 실제로 작업해야 하는 격리 pack dir. txn_prestate는 마지막에
        // env(ENV_PACK_DIR)=pd로 설정하므로, 아래에서 POISON으로 덮어써 env≠주입 dir을 만든다.
        let (base, pd) = txn_prestate(
            "env-inject",
            &[("README.md", "OLD-SOUL"), ("stale.txt", "STALE")],
            "1.0.0",
        );

        // POISON: 전역 env가 가리키는 "실 pack dir" 스탠드인. 별도 base라 parent(.pack-journal 위치)도
        // pd와 분리된다. 회귀가 있으면 여기가 prune/overwrite/journal 대상이 된다.
        let poison_base =
            std::env::temp_dir().join(format!("cys-journal-POISON-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&poison_base);
        let poison_pd = poison_base.join("pack");
        std::fs::create_dir_all(&poison_pd).unwrap();
        std::fs::write(poison_pd.join(PACK_VERSION_FILE), "9.9.9").unwrap();
        std::fs::write(poison_pd.join("README.md"), "POISON-SOUL").unwrap();
        std::fs::write(poison_pd.join("stale.txt"), "POISON-STALE").unwrap();
        let mut pmani = serde_json::Map::new();
        pmani.insert(
            "README.md".to_string(),
            serde_json::json!(content_hash("POISON-SOUL")),
        );
        pmani.insert(
            "stale.txt".to_string(),
            serde_json::json!(content_hash("POISON-STALE")),
        );
        std::fs::write(
            poison_pd.join(INSTALL_MANIFEST),
            serde_json::Value::Object(pmani).to_string(),
        )
        .unwrap();

        // 전역 env를 POISON으로 돌린다 — 트랜잭션 도중 CYS_PACK_DIR 변조 재현(2026-07-15 재앙 조건).
        std::env::set_var(ENV_PACK_DIR, &poison_pd);
        std::env::set_var(ENV_CONFIG_DIR, poison_base.join("cfg"));

        // 주입 dir(pd)로 트랜잭션 실행. stale.txt는 items 부재 → prune 대상.
        let items: Vec<(&str, &str)> = vec![("README.md", "NEW-SOUL"), ("new.txt", "NEW")];
        let res =
            apply_pack_transactional_in(&pd, &items, "2.0.0", &test_free_state("2.0.0"), || Ok(()));

        // 주입 dir 효과 관측(파괴·prune·commit은 여기에만 착지).
        let pd_pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let pd_soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let pd_stale_exists = pd.join("stale.txt").exists();
        // POISON(전역 env 경로) 무손상 관측.
        let poison_pv = std::fs::read_to_string(poison_pd.join(PACK_VERSION_FILE)).unwrap();
        let poison_soul = std::fs::read_to_string(poison_pd.join("README.md")).unwrap();
        let poison_stale_exists = poison_pd.join("stale.txt").exists();
        let poison_journal_exists = pack_journal_dir(&poison_pd).exists();

        restore_env(saved, saved_cfg);
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&poison_base);

        // 주입 dir에 파괴/prune/commit 효과가 착지했다.
        assert!(res.is_ok(), "주입 dir 트랜잭션 실패: {res:?}");
        assert_eq!(pd_pv.trim(), "2.0.0", "주입 dir에 commit marker 미기록");
        assert_eq!(pd_soul, "NEW-SOUL", "주입 dir README.md 미갱신");
        assert!(!pd_stale_exists, "주입 dir stale.txt prune 안됨");
        // POISON(전역 env 경로)은 완전 무손상 — prune/overwrite/journal 0(env 재해석 회귀 시 실패).
        assert_eq!(
            poison_pv.trim(),
            "9.9.9",
            "POISON .pack-version 오염됨 — 트랜잭션이 전역 env pack을 재해석함(회귀)"
        );
        assert_eq!(
            poison_soul, "POISON-SOUL",
            "POISON README.md 덮임 — 트랜잭션이 전역 env pack을 재해석함(회귀)"
        );
        assert!(
            poison_stale_exists,
            "POISON stale.txt가 prune됨 — 전역 env pack mass-prune 회귀(2026-07-15 재앙 클래스)!"
        );
        assert!(
            !poison_journal_exists,
            "POISON 부모에 저널 생성됨 — 트랜잭션이 전역 env pack을 재해석함(회귀)"
        );
    }

    /// ★격리 회귀(issue2 — codex R5, 필수 산출물): recovery/rollback 경로도 **주입된 dir**만
    /// 자가치유 대상으로 삼고 전역 CYS_PACK_DIR(env)이 가리키는 실 pack은 절대 건드리지 않음을
    /// 증명한다. 기존 orphan_journal_recovery_rolls_back는 env=주입 dir이 같은 pd라
    /// recover_pack_journal_in이 pack_dir()을 재해석해도 같은 pd로 수렴하는 false-green이었다 —
    /// 저널 생성·최종 정리가 둘 다 poison으로 재해석되면 주입 pd의 rollback이 깨져도 통과했다.
    /// 여기서는 env를 POISON으로 돌려, 회귀(내부 재해석)가 있으면 주입 dir의 orphan이 롤백되지
    /// 않거나 POISON이 오염돼 아래 assert가 실패하게 만든다.
    #[test]
    fn recover_journal_ignores_env_uses_injected_dir() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();

        // 주입 dir(pd): crash 후 부분적용 — .pack-version 옛 1.0.0(미커밋) + README.md 부분반영
        // + new.txt 신규생성. txn_prestate가 마지막에 env=pd로 설정하므로 아래서 POISON으로 덮는다.
        let (base, pd) =
            txn_prestate("recover-env-inject", &[("README.md", "PARTIAL-NEW")], "1.0.0");
        std::fs::write(pd.join("new.txt"), "ORPHAN-NEW").unwrap();
        // 주입 dir에 orphan 저널 수작업 조립(orphan_journal_recovery_rolls_back와 동일 기법):
        // target 2.0.0, README.md(existed) backup=OLD-SOUL, new.txt(신규) existed=false.
        let jdir = pack_journal_dir(&pd);
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

        // POISON: 전역 env가 가리키는 "실 pack dir" 스탠드인 — 저널 없음(회귀 시 여기서 생성/제거되면 실패).
        let poison_base =
            std::env::temp_dir().join(format!("cys-recover-POISON-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&poison_base);
        let poison_pd = poison_base.join("pack");
        std::fs::create_dir_all(&poison_pd).unwrap();
        std::fs::write(poison_pd.join(PACK_VERSION_FILE), "9.9.9").unwrap();
        std::fs::write(poison_pd.join("README.md"), "POISON-SOUL").unwrap();
        std::env::set_var(ENV_PACK_DIR, &poison_pd);
        std::env::set_var(ENV_CONFIG_DIR, poison_base.join("cfg"));

        // 주입 dir(pd)로 recovery 실행 — env(POISON)이 아니라 pd의 orphan을 롤백해야 한다.
        let recovered = recover_pack_journal_in(&pd);

        // 주입 dir 관측(rollback 착지).
        let pd_soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let pd_new_exists = pd.join("new.txt").exists();
        let pd_pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let pd_journal_exists = pack_journal_dir(&pd).exists();
        // POISON(전역 env 경로) 무손상 관측.
        let poison_pv = std::fs::read_to_string(poison_pd.join(PACK_VERSION_FILE)).unwrap();
        let poison_soul = std::fs::read_to_string(poison_pd.join("README.md")).unwrap();
        let poison_journal_exists = pack_journal_dir(&poison_pd).exists();

        restore_env(saved, saved_cfg);
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&poison_base);

        // 주입 dir에 rollback이 착지했다(pre-state 자가치유).
        assert_eq!(recovered.expect("recover 실패"), true, "주입 dir orphan 미발견");
        assert_eq!(pd_soul, "OLD-SOUL", "주입 dir README.md rollback 안됨");
        assert!(!pd_new_exists, "주입 dir 신규 new.txt 삭제 안됨");
        assert_eq!(pd_pv.trim(), "1.0.0", "주입 dir .pack-version 변경됨(미커밋인데)");
        assert!(!pd_journal_exists, "주입 dir recovery 후 저널 잔존");
        // POISON(전역 env 경로)은 완전 무손상 — rollback/journal 0(env 재해석 회귀 시 실패).
        assert_eq!(
            poison_pv.trim(),
            "9.9.9",
            "POISON .pack-version 오염됨 — recovery가 전역 env pack을 재해석함(회귀)"
        );
        assert_eq!(
            poison_soul, "POISON-SOUL",
            "POISON README.md 덮임 — recovery가 전역 env pack을 재해석함(회귀)"
        );
        assert!(
            !poison_journal_exists,
            "POISON 부모에 저널 생성/제거됨 — recovery가 전역 env pack을 재해석함(회귀)"
        );
    }

    /// ★격리 회귀(issue3 best-effort — codex R5): apply 도중 fault로 rollback을 강제해도 **주입된
    /// dir**만 복원 대상이고 전역 CYS_PACK_DIR(env)이 가리키는 실 pack은 무손상임을 증명한다.
    /// mid_apply_fault_rolls_back_to_prestate의 collide fault 기법 + env-inject의 POISON 분리를 결합한다.
    #[test]
    fn apply_fault_rollback_ignores_env_uses_injected_dir() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();

        let (base, pd) = txn_prestate(
            "fault-env-inject",
            &[("README.md", "OLD-SOUL"), ("stale.txt", "STALE")],
            "1.0.0",
        );
        let pre_fp = fingerprint_dir(&pd);

        // POISON: 전역 env가 가리키는 "실 pack dir" 스탠드인.
        let poison_base =
            std::env::temp_dir().join(format!("cys-fault-POISON-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&poison_base);
        let poison_pd = poison_base.join("pack");
        std::fs::create_dir_all(&poison_pd).unwrap();
        std::fs::write(poison_pd.join(PACK_VERSION_FILE), "9.9.9").unwrap();
        std::fs::write(poison_pd.join("README.md"), "POISON-SOUL").unwrap();
        std::fs::write(poison_pd.join("stale.txt"), "POISON-STALE").unwrap();
        let mut pmani = serde_json::Map::new();
        pmani.insert(
            "README.md".to_string(),
            serde_json::json!(content_hash("POISON-SOUL")),
        );
        pmani.insert(
            "stale.txt".to_string(),
            serde_json::json!(content_hash("POISON-STALE")),
        );
        std::fs::write(
            poison_pd.join(INSTALL_MANIFEST),
            serde_json::Value::Object(pmani).to_string(),
        )
        .unwrap();
        std::env::set_var(ENV_PACK_DIR, &poison_pd);
        std::env::set_var(ENV_CONFIG_DIR, poison_base.join("cfg"));

        // collide fault: README.md 갱신(성공) → collide 파일(성공) → collide/child(부모가 파일이라
        // create_dir_all 실패) = mid-apply fault → rollback 강제.
        let items: Vec<(&str, &str)> =
            vec![("README.md", "NEW"), ("collide", "X"), ("collide/child", "Y")];
        let res =
            apply_pack_transactional_in(&pd, &items, "2.0.0", &test_free_state("2.0.0"), || Ok(()));

        // 주입 dir 관측(rollback 착지).
        let post_fp = fingerprint_dir(&pd);
        let pd_pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let pd_journal_exists = pack_journal_dir(&pd).exists();
        // POISON(전역 env 경로) 무손상 관측.
        let poison_pv = std::fs::read_to_string(poison_pd.join(PACK_VERSION_FILE)).unwrap();
        let poison_soul = std::fs::read_to_string(poison_pd.join("README.md")).unwrap();
        let poison_stale_exists = poison_pd.join("stale.txt").exists();
        let poison_journal_exists = pack_journal_dir(&poison_pd).exists();

        restore_env(saved, saved_cfg);
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&poison_base);

        // 주입 dir: fault → rollback으로 pre-state 복원·미커밋.
        assert!(res.is_err(), "mid-apply fault인데 성공 반환");
        assert_eq!(pd_pv.trim(), "1.0.0", "주입 dir .pack-version 불변이어야(미커밋)");
        assert!(!pd_journal_exists, "주입 dir rollback 후 저널 잔존");
        assert_eq!(pre_fp, post_fp, "주입 dir rollback이 pre-state로 복원 못함(부분적용 잔존)");
        // POISON(전역 env 경로)은 완전 무손상 — prune/overwrite/journal 0(env 재해석 회귀 시 실패).
        assert_eq!(
            poison_pv.trim(),
            "9.9.9",
            "POISON .pack-version 오염됨 — 트랜잭션이 전역 env pack을 재해석함(회귀)"
        );
        assert_eq!(
            poison_soul, "POISON-SOUL",
            "POISON README.md 덮임 — 트랜잭션이 전역 env pack을 재해석함(회귀)"
        );
        assert!(
            poison_stale_exists,
            "POISON stale.txt가 prune됨 — 전역 env pack mass-prune 회귀(2026-07-15 재앙 클래스)!"
        );
        assert!(
            !poison_journal_exists,
            "POISON 부모에 저널 생성됨 — 트랜잭션이 전역 env pack을 재해석함(회귀)"
        );
    }

    /// ★대조(외부 동작 불변): embed/cysd/init-pack 경로(transactional=false)는 .install-manifest.json이
    /// 디렉터리여도 매니페스트 영속을 best-effort로 무시하고 설치를 진행한다 — 파일 반영·.pack-version
    /// 기록이 종전대로 일어난다(fail-closed는 pack-update 트랜잭션 전용).
    #[test]
    fn manifest_write_failure_embed_best_effort_proceeds() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let (base, pd) = txn_prestate("manifest-embed", &[("README.md", "OLD-SOUL")], "1.0.0");
        let mp = pd.join(INSTALL_MANIFEST);
        std::fs::remove_file(&mp).unwrap();
        std::fs::create_dir_all(mp.join("child")).unwrap();

        // new.txt는 신규(preserve-gate 충돌 없음)라 반영된다. README.md는 매니페스트 불가독으로
        // preserve-gate가 안전측 보존(OLD 유지) — 이는 manifest 손상의 정상 부작용이며 embed
        // best-effort 분기와 무관하다. 핵심: 매니페스트 write 실패에도 Err 없이 진행 + 버전 마커 기록.
        let items: Vec<(&str, &str)> = vec![("README.md", "NEW-SOUL"), ("new.txt", "NEW")];
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false);

        let new_exists = pd.join("new.txt").exists();
        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        restore_env(saved, saved_cfg);
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
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let (base, pd) = txn_prestate(
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
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false);

        let pro_file = std::fs::read_to_string(pd.join("pro-only/skill.md")).unwrap_or_default();
        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let pv = std::fs::read_to_string(pd.join(PACK_VERSION_FILE)).unwrap();
        let st_after = read_pack_state(&pd);
        restore_env(saved, saved_cfg);
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
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let (base, pd) = txn_prestate("corruptguard", &[("README.md", "OLD")], "1.0.0");
        std::fs::write(pd.join(PACK_STATE_FILE), b"{not json").unwrap();

        let items: Vec<(&str, &str)> = vec![("README.md", "NEW")];
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false);

        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        restore_env(saved, saved_cfg);
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(res.expect("가드 경로 Ok"), (0, 0));
        assert_eq!(soul, "OLD", "손상 state인데 파일이 변경됨(보존 위반)");
    }

    /// channel=free 정합 불일치 + 음성 pro 증거 없음 → 제한적 자가치유 후 install 진행(v6 §5).
    #[test]
    fn embed_free_mismatch_heals_without_evidence() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        // install-manifest 키는 임베드 트리에 실재하는 rel만(README.md) — pro 파일 증거 없음.
        let (base, pd) = txn_prestate("healok", &[("README.md", "OLD")], "1.0.0");
        write_pack_state(&pd, &test_free_state("0.9.0")).unwrap(); // base 불일치(0.9.0 ≠ 1.0.0)

        let items: Vec<(&str, &str)> = vec![("README.md", "NEW")];
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false);

        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        let st_after = read_pack_state(&pd);
        restore_env(saved, saved_cfg);
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
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let (base, pd) = txn_prestate(
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
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false);

        let pro_file = std::fs::read_to_string(pd.join("pro-only/skill.md")).unwrap_or_default();
        let soul = std::fs::read_to_string(pd.join("README.md")).unwrap();
        restore_env(saved, saved_cfg);
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(res.expect("보존 경로 Ok"), (0, 0), "거짓 free인데 install 진행됨");
        assert_eq!(pro_file, "PRO", "★거짓 free 자가치유가 pro 파일을 prune(R5 재앙 재현)");
        assert_eq!(soul, "OLD", "거짓 free인데 파일 덮임");
    }

    /// v6 음성 증거 ②: accepted 부재여도 pro 전용 파일 실재(임베드 외 설치 기록) → 자가치유 금지.
    #[test]
    fn embed_free_mismatch_with_pro_file_evidence_preserved() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        // install-manifest에 임베드 트리 밖 rel(pro-only/skill.md) = pro 파일 증거.
        let (base, pd) = txn_prestate(
            "proevidence",
            &[("README.md", "OLD"), ("pro-only/skill.md", "PRO")],
            "1.0.0",
        );
        write_pack_state(&pd, &test_free_state("0.9.0")).unwrap();

        let items: Vec<(&str, &str)> = vec![("README.md", "NEW")];
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false);

        let pro_file = std::fs::read_to_string(pd.join("pro-only/skill.md")).unwrap_or_default();
        restore_env(saved, saved_cfg);
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(res.expect("보존 경로 Ok"), (0, 0));
        assert_eq!(pro_file, "PRO", "pro 파일 증거 무시하고 진행됨");
    }

    /// v5 checked 쓰기 순서 fault-injection: `.pack-version` 쓰기 실패(경로가 디렉터리) 시
    /// loud 처리 + state 미생성(불일치 미생성) + install 자체는 Ok(기존 best-effort 외부 동작).
    #[test]
    fn embed_version_write_failure_creates_no_state_mismatch() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let base = std::env::temp_dir().join(format!("cys-vfault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        std::fs::create_dir_all(&pd).unwrap();
        std::env::set_var(ENV_PACK_DIR, &pd);
        std::env::set_var(ENV_CONFIG_DIR, base.join("cfg"));
        // .pack-version 경로를 디렉터리로 만들어 write_atomic(rename) 실패 주입.
        std::fs::create_dir_all(pd.join(PACK_VERSION_FILE).join("child")).unwrap();

        let items: Vec<(&str, &str)> = vec![("README.md", "NEW")];
        let res = install_from_iter(items.iter().copied(), false, "2.0.0", false);

        let state_exists = pd.join(PACK_STATE_FILE).exists();
        let soul_exists = pd.join("README.md").exists();
        restore_env(saved, saved_cfg);
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
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let base = std::env::temp_dir().join(format!("cys-staged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        std::env::set_var(ENV_PACK_DIR, &pd);
        std::env::set_var(ENV_CONFIG_DIR, base.join("claude"));

        let (rel0, _) = PACK_ALL[0];

        let (w1, _k1) = install_staged(false).unwrap();
        assert!(w1 > 0, "신설은 written>0");
        assert!(pd.join(".pack-version").is_file(), ".pack-version 기록");
        assert!(pd.join(rel0).is_file(), "임베드 파일 반영");
        assert!(!pack_prev_dir(&pd).exists(), "첫 설치는 .prev 없음");

        let (w2, _k2) = install_staged(false).unwrap();
        assert_eq!(w2, 0, "멱등 재설치 written=0");
        assert!(pack_prev_dir(&pd).exists(), "재설치는 .prev 1세대 보존");
        assert!(pd.join(rel0).is_file(), "재설치 후 임베드 온전");

        restore_env(saved, saved_cfg);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// user-edit 보존: force=false 재설치가 사용자 편집 파일을 덮지 않는다(init-pack '4 preserved' 정합).
    #[test]
    fn install_staged_preserves_user_edit() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let base = std::env::temp_dir().join(format!("cys-staged-pres-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pd = base.join("pack");
        std::env::set_var(ENV_PACK_DIR, &pd);
        std::env::set_var(ENV_CONFIG_DIR, base.join("claude"));

        install_staged(false).unwrap();
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

        install_staged(false).unwrap();
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

        restore_env(saved, saved_cfg);
        let _ = std::fs::remove_dir_all(&base);
    }
}
